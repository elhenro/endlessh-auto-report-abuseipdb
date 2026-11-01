//! one "2026-01-01T00:00:00Z 1.2.3.4" per line, opened per write so moving or emptying it just works

use crate::{dates::timestamp, events::open_append};
use std::{
    fs,
    io::{self, Write},
    net::IpAddr,
    path::{Path, PathBuf},
};

pub struct ReportedFile {
    path: PathBuf,
    max_bytes: u64,
    warned: bool,
}

/// the rotated-out generation lives beside the file, under a fixed suffix one deep
fn backup(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(".old");
    name.into()
}

impl ReportedFile {
    pub fn new(path: PathBuf, max_bytes: u64) -> Self {
        Self {
            path,
            max_bytes,
            warned: false,
        }
    }

    pub fn add(&mut self, t: u64, ip: IpAddr) {
        let line = format!("{} {ip}\n", timestamp(t));
        let written = self
            .rotate_if_due(line.len() as u64)
            .and_then(|()| open_append(&self.path).and_then(|mut f| f.write_all(line.as_bytes())));
        match written {
            Ok(()) => self.warned = false,
            Err(e) if !self.warned => {
                self.warned = true;
                log!("warning: cannot write {}: {e}", self.path.display());
            }
            Err(_) => {}
        }
    }

    /// once the fresh line would pass the cap, the current file becomes `<path>.old`
    /// and the next add starts a new one; sole writer, so the size check is not racy
    fn rotate_if_due(&mut self, incoming: u64) -> io::Result<()> {
        let Some(len) = fs::metadata(&self.path).ok().map(|m| m.len()) else {
            return Ok(()); // nothing on disk yet: the first write cannot overflow
        };
        if len + incoming <= self.max_bytes {
            return Ok(());
        }
        fs::rename(&self.path, backup(&self.path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, os::unix::fs::symlink};

    fn dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("reported-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn appends_in_the_old_format_and_survives_a_trim() {
        let d = dir("format");
        let path = d.join("reportedIps.txt");
        fs::write(&path, "2025-01-01T00:00:00Z 9.9.9.9\n").unwrap();
        let mut f = ReportedFile::new(path.clone(), u64::MAX);
        f.add(1_760_000_000, "8.8.8.8".parse().unwrap());
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "2025-01-01T00:00:00Z 9.9.9.9\n2025-10-09T08:53:20Z 8.8.8.8\n"
        );
        fs::rename(&path, d.join("old")).unwrap();
        f.add(1_760_000_001, "2a00:1450::1".parse().unwrap());
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "2025-10-09T08:53:21Z 2a00:1450::1\n"
        );
        fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn does_not_follow_a_prepared_symlink() {
        let d = dir("symlink");
        let (path, target) = (d.join("reportedIps.txt"), d.join("target"));
        fs::write(&target, "").unwrap();
        symlink(&target, &path).unwrap();
        ReportedFile::new(path, u64::MAX).add(1, "8.8.8.8".parse().unwrap());
        assert_eq!(fs::read_to_string(&target).unwrap(), "");
        fs::remove_dir_all(d).unwrap();
    }

    /// a v4 line is 29 bytes: a 30-byte cap rotates on the second line
    #[test]
    fn past_the_cap_the_old_generation_becomes_dot_old() {
        let d = dir("rotate");
        let path = d.join("reportedIps.txt");
        let mut f = ReportedFile::new(path.clone(), 30);
        f.add(1_760_000_000, "8.8.8.8".parse().unwrap());
        f.add(1_760_000_001, "8.8.8.8".parse().unwrap());
        f.add(1_760_000_002, "8.8.8.8".parse().unwrap());
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "2025-10-09T08:53:22Z 8.8.8.8\n",
            "the fresh generation holds the newest line"
        );
        assert_eq!(
            fs::read_to_string(d.join("reportedIps.txt.old")).unwrap(),
            "2025-10-09T08:53:21Z 8.8.8.8\n",
            "the rotated generation is one behind"
        );

        // a rotation without an add hands the current one over and leaves no file
        f.rotate_if_due(30).unwrap();
        assert!(
            fs::read_to_string(&path).is_err(),
            "gone, nothing written yet"
        );
        assert_eq!(
            fs::read_to_string(d.join("reportedIps.txt.old")).unwrap(),
            "2025-10-09T08:53:22Z 8.8.8.8\n",
            "one generation of history"
        );
        fs::remove_dir_all(d).unwrap();
    }
}
