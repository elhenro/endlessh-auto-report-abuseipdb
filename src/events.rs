//! append-only jsonl event log for dashboards; one file per utc day: events-YYYY-MM-DD.jsonl
//!   {"t":1760000000,"e":"accept","ip":"1.2.3.4","a":"reported","q":2995}
//!   {"t":1760000812,"e":"close","ip":"1.2.3.4","s":812.345,"b":4096}
//! a = reported|delivered|dedup|dry|paused|failed|rejected|notssh, q = api quota left, s = seconds held, b = bytes sent
//! values are numbers, fixed words or formatted ips, so no json escaping is needed

use crate::dates::{date_string, parse_date};
use std::{
    fs::{self, DirBuilder, File, OpenOptions, Permissions},
    io::{self, Write},
    net::IpAddr,
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};

/// whoever else can write to the directory may have prepared this name: a symlink or a hard link
/// would have the appends land on another file, a fifo would hang the worker
pub(crate) fn open_append(path: &Path) -> io::Result<File> {
    let f = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o644)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    let m = f.metadata()?;
    if !m.is_file() || m.nlink() != 1 {
        return Err(io::Error::other("not a plain file"));
    }
    Ok(f)
}

/// today's file, chmodded for dashboards: all the more reason it has to be our own
fn open_day_file(dir: &Path, path: &Path) -> io::Result<File> {
    let f = open_append(path)?;
    if f.metadata()?.uid() != fs::metadata(dir)?.uid() {
        return Err(io::Error::other("not owned by the directory's owner"));
    }
    f.set_permissions(Permissions::from_mode(0o644))?;
    Ok(f)
}

pub struct EventLog {
    dir: PathBuf,
    keep_days: i64,
    max_bytes: u64, // per day: one host looping connects must not fill the disk
    open_day: i64,
    file: Option<File>,
    written: u64,
    capped: bool,
    warned: bool,
}

impl EventLog {
    pub fn open(dir: PathBuf, keep_days: u32, max_bytes: u64) -> Result<Self, String> {
        let err = |e: std::io::Error| format!("cannot create {}: {e}", dir.display());
        let existed = dir.is_dir();
        DirBuilder::new()
            .recursive(true)
            .mode(0o755)
            .create(&dir)
            .map_err(err)?;
        if !existed {
            // 755/644 whatever the umask: a dashboard running as another uid reads these (attacker ips are public anyway)
            fs::set_permissions(&dir, Permissions::from_mode(0o755)).map_err(err)?;
        } else if fs::metadata(&dir).map_err(err)?.permissions().mode() & 0o022 != 0 {
            // an existing directory keeps the mode its owner chose, unless others could swap files in it
            return Err(format!("{} is writable by other users", dir.display()));
        }
        Ok(Self {
            dir,
            keep_days: keep_days.into(),
            max_bytes,
            open_day: i64::MIN,
            file: None,
            written: 0,
            capped: false,
            warned: false,
        })
    }

    pub fn accept(&mut self, t: u64, ip: IpAddr, action: &str, quota: Option<u64>) {
        let q = quota.map(|q| format!(",\"q\":{q}")).unwrap_or_default();
        self.write(
            t,
            &format!("\"e\":\"accept\",\"ip\":\"{ip}\",\"a\":\"{action}\"{q}"),
        );
    }

    pub fn close(&mut self, t: u64, ip: IpAddr, secs: f64, bytes: u64) {
        self.write(
            t,
            &format!("\"e\":\"close\",\"ip\":\"{ip}\",\"s\":{secs:.3},\"b\":{bytes}"),
        );
    }

    fn write(&mut self, t: u64, members: &str) {
        let day = (t / 86_400) as i64;
        if day != self.open_day {
            self.rotate(day);
        }
        let Some(f) = self.file.as_mut() else { return };
        let line = format!("{{\"t\":{t},{members}}}\n");
        if self.written + line.len() as u64 > self.max_bytes {
            if !self.capped {
                self.capped = true;
                log!(
                    "warning: event log hit EVENTS_MAX_DAILY_MB, dropping events until the next utc day"
                );
            }
            return;
        }
        self.written += line.len() as u64;
        // one write call per line keeps lines whole for a reader tailing the file
        if f.write_all(line.as_bytes()).is_err() && !self.warned {
            self.warned = true;
            log!("warning: cannot write events to {}", self.dir.display());
        }
    }

    fn rotate(&mut self, day: i64) {
        self.open_day = day;
        let path = self.dir.join(format!("events-{}.jsonl", date_string(day)));
        self.file = open_day_file(&self.dir, &path)
            .map_err(|e| {
                log!(
                    "warning: no events today, cannot open {}: {e}",
                    path.display()
                )
            })
            .ok();
        // what is already there counts, or every restart would hand out a fresh budget
        self.written = self
            .file
            .as_ref()
            .and_then(|f| f.metadata().ok())
            .map_or(0, |m| m.len());
        self.capped = false;
        self.prune(day);
    }

    /// old days go first, then whole days from the old end while the directory is over the budget.
    /// the budget is daily cap times retention, which is what the readme promises
    fn prune(&self, today: i64) {
        let Ok(entries) = fs::read_dir(&self.dir) else {
            return;
        };
        let mut days: Vec<(i64, PathBuf, u64)> = Vec::new();
        for entry in entries.flatten() {
            let name = entry.file_name();
            let file_day = name
                .to_str()
                .and_then(|n| n.strip_prefix("events-")?.strip_suffix(".jsonl"))
                .and_then(parse_date);
            let Some(day) = file_day else { continue };
            if day < today - self.keep_days {
                let _ = fs::remove_file(entry.path());
                continue;
            }
            let len = entry.metadata().map_or(0, |m| m.len());
            days.push((day, entry.path(), len));
        }
        let budget = self.max_bytes.saturating_mul(self.keep_days.max(0) as u64);
        let mut total: u64 = days.iter().map(|d| d.2).sum();
        days.sort_by_key(|d| d.0);
        for (day, path, len) in days {
            // never today's: it is open, and it is what the tarpit is writing
            if total <= budget || day >= today {
                break;
            }
            if fs::remove_file(&path).is_ok() {
                total = total.saturating_sub(len);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("events-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn drops_whole_days_from_the_old_end_past_the_budget() {
        let d = dir("budget");
        let today = 20_700; // 2026-09-03, any day works
        for day in [today - 2, today - 1, today] {
            let path = d.join(format!("events-{}.jsonl", date_string(day)));
            fs::write(&path, vec![b'x'; 1000]).unwrap();
        }
        // two days of room: 3 files of 1000 bytes do not fit
        let log = EventLog::open(d.clone(), 2, 1_200).unwrap();
        log.prune(today);
        let left: Vec<i64> = fs::read_dir(&d)
            .unwrap()
            .flatten()
            .filter_map(|e| {
                let n = e.file_name();
                let n = n
                    .to_str()?
                    .strip_prefix("events-")?
                    .strip_suffix(".jsonl")?;
                parse_date(n)
            })
            .collect();
        assert_eq!(left.len(), 2, "only the budget, from the old end");
        assert!(!left.contains(&(today - 2)), "the oldest day went first");
        assert!(left.contains(&today), "today is never pruned");
        let _ = fs::remove_dir_all(&d);
    }
}
