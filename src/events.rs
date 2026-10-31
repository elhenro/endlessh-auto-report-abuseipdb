//! append-only jsonl event log for dashboards; one file per utc day: events-YYYY-MM-DD.jsonl
//!   {"t":1760000000,"e":"accept","ip":"1.2.3.4","a":"reported","q":2995}
//!   {"t":1760000812,"e":"close","ip":"1.2.3.4","s":812.345,"b":4096}
//! a = reported|delivered|dedup|dry|paused|failed|rejected, q = api quota left, s = seconds held, b = bytes sent
//! values are numbers, fixed words or formatted ips, so no json escaping is needed

use crate::dates::{date_string, parse_date};
use std::{
    fs::{self, DirBuilder, File, OpenOptions, Permissions},
    io::Write,
    net::IpAddr,
    os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt},
    path::PathBuf,
};

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
        let opened = OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o644)
            .open(&path)
            .and_then(|f| {
                f.set_permissions(Permissions::from_mode(0o644))?;
                Ok(f)
            });
        self.file = opened
            .map_err(|e| log!("warning: cannot open {}: {e}", path.display()))
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

    fn prune(&self, today: i64) {
        let Ok(entries) = fs::read_dir(&self.dir) else {
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let file_day = name
                .to_str()
                .and_then(|n| n.strip_prefix("events-")?.strip_suffix(".jsonl"))
                .and_then(parse_date);
            if file_day.is_some_and(|d| d < today - self.keep_days) {
                let _ = fs::remove_file(entry.path());
            }
        }
    }
}
