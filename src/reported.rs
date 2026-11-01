//! one "2026-01-01T00:00:00Z 1.2.3.4" per line, opened per write so moving or emptying it just works

use crate::{dates::timestamp, events::open_append};
use std::{io::Write, net::IpAddr, path::PathBuf};

pub struct ReportedFile {
    path: PathBuf,
    warned: bool,
}

impl ReportedFile {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            warned: false,
        }
    }

    pub fn add(&mut self, t: u64, ip: IpAddr) {
        let line = format!("{} {ip}\n", timestamp(t));
        let written = open_append(&self.path).and_then(|mut f| f.write_all(line.as_bytes()));
        match written {
            Ok(()) => self.warned = false,
            Err(e) if !self.warned => {
                self.warned = true;
                log!("warning: cannot write {}: {e}", self.path.display());
            }
            Err(_) => {}
        }
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
        let mut f = ReportedFile::new(path.clone());
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
        ReportedFile::new(path).add(1, "8.8.8.8".parse().unwrap());
        assert_eq!(fs::read_to_string(&target).unwrap(), "");
        fs::remove_dir_all(d).unwrap();
    }
}
