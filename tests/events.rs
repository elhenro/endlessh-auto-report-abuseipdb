//! the event log on a real filesystem

use endlessh_reporter::events::EventLog;
use std::{
    fs::{self, Permissions},
    net::IpAddr,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
};

#[test]
fn writes_lines_rotates_and_prunes() {
    let dir = std::env::temp_dir().join(format!("events-test-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir(&dir).unwrap();
    fs::set_permissions(&dir, Permissions::from_mode(0o755)).unwrap();
    let mut log = EventLog::open(dir.clone(), 2, u64::MAX).unwrap();
    let ip: IpAddr = "8.8.8.8".parse().unwrap();
    log.accept(86_400 * 10, ip, "reported", Some(7));
    log.close(86_400 * 10 + 5, ip, 12.3456, 99);
    log.accept(86_400 * 11, ip, "dedup", None);
    log.accept(86_400 * 14, ip, "dry", None); // days 10 and 11 are now older than 2 days
    let mut names: Vec<_> = fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    assert_eq!(names, ["events-1970-01-15.jsonl"]);
    assert_eq!(
        fs::read_to_string(dir.join(&names[0])).unwrap(),
        "{\"t\":1209600,\"e\":\"accept\",\"ip\":\"8.8.8.8\",\"a\":\"dry\"}\n"
    );
    let mut again = EventLog::open(dir.clone(), 30, u64::MAX).unwrap();
    again.close(86_400 * 14 + 1, ip, 1.5, 3);
    let text = fs::read_to_string(dir.join(&names[0])).unwrap();
    assert!(
        text.ends_with("{\"t\":1209601,\"e\":\"close\",\"ip\":\"8.8.8.8\",\"s\":1.500,\"b\":3}\n")
    );
    assert_eq!(
        fs::metadata(dir.join(&names[0]))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o644
    );
    fs::remove_dir_all(&dir).unwrap();
}

fn test_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("events-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    dir
}

fn mode(dir: &PathBuf) -> u32 {
    fs::metadata(dir).unwrap().permissions().mode() & 0o777
}

#[test]
fn directory_modes() {
    let dir = test_dir("modes");
    EventLog::open(dir.clone(), 1, 1).unwrap();
    assert_eq!(
        mode(&dir),
        0o755,
        "new directory is readable for a dashboard"
    );
    fs::set_permissions(&dir, Permissions::from_mode(0o700)).unwrap();
    EventLog::open(dir.clone(), 1, 1).unwrap();
    assert_eq!(mode(&dir), 0o700, "a private directory must stay private");
    for writable in [0o777, 0o775, 0o757] {
        fs::set_permissions(&dir, Permissions::from_mode(writable)).unwrap();
        assert!(EventLog::open(dir.clone(), 1, 1).is_err(), "{writable:o}");
        assert_eq!(mode(&dir), writable, "refusing must not touch it");
    }
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn stops_at_the_daily_cap_across_restarts_and_resumes_next_day() {
    let dir = test_dir("cap");
    let ip: IpAddr = "8.8.8.8".parse().unwrap();
    let (day, cap) = (86_400 * 10, 120);
    let today = dir.join("events-1970-01-11.jsonl");
    let mut log = EventLog::open(dir.clone(), 30, cap).unwrap();
    for _ in 0..50 {
        log.accept(day, ip, "dry", None);
    }
    let full = fs::metadata(&today).unwrap().len();
    assert!(full > 0 && full <= cap, "{full}");
    assert!(
        fs::read_to_string(&today).unwrap().ends_with("}\n"),
        "whole lines only"
    );

    let mut restarted = EventLog::open(dir.clone(), 30, cap).unwrap();
    restarted.accept(day, ip, "dry", None);
    assert_eq!(fs::metadata(&today).unwrap().len(), full);

    restarted.accept(day + 86_400, ip, "dry", None);
    assert!(dir.join("events-1970-01-12.jsonl").exists());
    fs::remove_dir_all(&dir).unwrap();
}
