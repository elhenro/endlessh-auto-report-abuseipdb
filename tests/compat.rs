//! what the old bash reporter accepted and did has to keep working: its env vars, flags and files

// each test file uses part of these
#[allow(dead_code)]
mod common;
#[allow(dead_code)]
mod mock;

use common::{as_systemd, command, free_port, serial, visit, Running, BIN};
use endlessh_reporter::{ip::AllowList, reported::ReportedFile, tarpit::Event, worker::Worker};
use mock::{abuse, accept, mock_api};
use std::{
    fs,
    io::Read,
    net::TcpListener,
    os::fd::OwnedFd,
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::Duration,
};

fn with_env(env: &[(&str, &str)]) -> Command {
    let mut c = command(BIN);
    c.env("TARPIT_PORT", free_port().to_string());
    for (k, v) in env {
        c.env(k, v);
    }
    c
}

/// the exit code, if it stops within a second instead of coming up
fn exit_code(env: &[(&str, &str)]) -> Option<i32> {
    let mut child = with_env(env)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let code = (0..100).find_map(|_| {
        thread::sleep(Duration::from_millis(10));
        child.try_wait().unwrap()
    });
    let _ = child.kill();
    let _ = child.wait();
    code.and_then(|s| s.code())
}

#[test]
fn starts_with_every_value_the_old_reporter_took() {
    let _serial = serial();
    for env in [
        ("CURL_TIMEOUT", "0"),
        ("CURL_TIMEOUT", "300"),
        ("TARPIT_DELAY_MS", "2147483647"),
        ("TARPIT_MAX_CLIENTS", "2147483647"),
        ("TARPIT_PORT", "02222"),
        ("DEDUPE_SECONDS", "0"),
        ("CATEGORIES", "18,22"),
        ("ALLOWLIST", "1.2.3.4, 2001:DB8::1 5.6.7.8"),
        ("ALLOWLIST", "010.0.0.1"),
        ("ALLOWLIST", "1:2:3"),
        ("ENDLESSH_ARGS", "-4"),
        ("ENDLESSH_ARGS", "-vv -l 64 -d 500 -m 9"),
        ("ENDLESSH_ARGS", "-f /nonexistent"),
        ("API_TOKEN_FILE", "/nonexistent"),
        ("REPORTED_FILE", ""),
    ] {
        assert_eq!(exit_code(&[env]), None, "{env:?}");
    }
}

#[test]
fn still_refuses_what_the_old_reporter_refused() {
    let _serial = serial();
    for env in [
        ("TARPIT_DELAY_MS", "2147483648"),
        ("TARPIT_DELAY_MS", "0"),
        ("TARPIT_MAX_CLIENTS", "0"),
        ("CURL_TIMEOUT", "soon"),
        ("DRY_RUN", "yes"),
        ("CATEGORIES", "14;22"),
        ("ALLOWLIST", "nope"),
        ("ENDLESSH_ARGS", "-x"),
        ("ENDLESSH_ARGS", "-l 2"),
        ("ENDLESSH_ARGS", "-p 65536"),
    ] {
        assert_eq!(exit_code(&[env]), Some(78), "{env:?}");
    }
    let real = [("DRY_RUN", "0"), ("API_TOKEN_FILE", "/nonexistent")];
    assert_eq!(exit_code(&real), Some(78), "a real run needs its key");
}

#[test]
fn http_timeout_wins_over_the_old_name_and_keeps_its_limits() {
    let _serial = serial();
    assert_eq!(exit_code(&[("HTTP_TIMEOUT", "300")]), Some(78));
    let both = [("HTTP_TIMEOUT", "5"), ("CURL_TIMEOUT", "soon")];
    assert_eq!(exit_code(&both), None);
}

#[test]
fn endlessh_flags_apply_and_win_over_the_variables() {
    let _serial = serial();
    let port = free_port();
    let flag = format!("-p {port} -4 -l 5 -d 20");
    let t = Running::start(&mut with_env(&[("ENDLESSH_ARGS", &flag)]));
    assert!(
        t.startup.contains(&format!("on 0.0.0.0:{port} ")),
        "{}",
        t.startup
    );

    let mut c = visit(port);
    thread::sleep(Duration::from_millis(100));
    let mut got = [0u8; 256];
    let n = c.read(&mut got).unwrap();
    for line in got[..n].split(|b| *b == b'\n').filter(|l| !l.is_empty()) {
        assert!(line.len() <= 5, "{:?}", String::from_utf8_lossy(line));
    }
    drop(t);

    let port = free_port();
    let flag = format!("-p {port} -6");
    let t = Running::start(&mut with_env(&[("ENDLESSH_ARGS", &flag)]));
    assert!(
        t.startup.contains(&format!("on [::]:{port} ")),
        "{}",
        t.startup
    );
    assert!(
        std::net::TcpStream::connect(("127.0.0.1", port)).is_err(),
        "-6 means ipv6 only"
    );
}

#[test]
fn a_signal_ends_it_with_the_old_exit_codes() {
    let _serial = serial();
    for (signal, code) in [("-TERM", 143), ("-INT", 130)] {
        let mut t = Running::start(&mut with_env(&[]));
        let kill = format!("kill {signal} {}", t.child.id());
        let sent = Command::new("sh").args(["-c", &kill]).status().unwrap();
        assert!(sent.success());
        assert_eq!(t.child.wait().unwrap().code(), Some(code), "{signal}");
    }
}

#[test]
fn says_so_when_the_socket_unit_decides_the_port() {
    let _serial = serial();
    let socket = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = socket.local_addr().unwrap().port();
    let warned = |tarpit_port: u16| {
        let socket = OwnedFd::from(socket.try_clone().unwrap());
        let mut c = as_systemd("1", socket.into());
        c.env("TARPIT_PORT", tarpit_port.to_string());
        Running::start(&mut c).startup.contains("is not in effect")
    };
    assert!(warned(port.wrapping_add(1)));
    assert!(!warned(port));
}

fn temp(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("compat-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// which port the installer believes sshd wants, with systemctl/sshd stubbed on PATH
fn sshd_wants(tag: &str, systemctl: &str, sshd: Option<&str>, ports: &[u16]) -> String {
    let dir = temp(tag);
    let bin = dir.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let exe = |name: &str, body: &str| {
        let path = bin.join(name);
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(&path, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    };
    exe("systemctl", systemctl);
    if let Some(body) = sshd {
        exe("sshd", body);
    }
    let script = format!(
        "say() {{ :; }}; die() {{ exit 1; }}; MODE=systemd; . ./install-ports.sh
         for p in {}; do sshd_wants $p && echo \"$p yes\" || echo \"$p no\"; done",
        ports
            .iter()
            .map(u16::to_string)
            .collect::<Vec<_>>()
            .join(" ")
    );
    let out = Command::new("bash")
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .env(
            "PATH",
            format!("{}:{}", bin.display(), std::env::var("PATH").unwrap()),
        )
        .args(["-c", &script])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

/// an install that moved sshd to make room for the tarpit: the socket unit owns the port, and
/// sshd_config's `Port` (or its absence, meaning 22) is not what sshd listens on
#[test]
fn the_installer_reads_sshd_port_from_the_socket_unit() {
    let got = sshd_wants(
        "sshd-wants-socket",
        r#"case $1 in
             is-enabled) [ "$3" = ssh.socket ] ;;
             show) printf '%s\n' "0.0.0.0:2247 (Stream)" "[::]:2247 (Stream)" ;;
             *) exit 1 ;;
           esac"#,
        Some("exit 1"), // sshd -T would claim 22, and must not be asked
        &[22, 2247],
    );
    assert_eq!(got, "22 no\n2247 yes\n");
}

/// without a socket unit the config decides: sshd -T still names the port
#[test]
fn the_installer_reads_sshd_port_from_the_config_without_a_socket_unit() {
    let got = sshd_wants(
        "sshd-wants-config",
        r#"case $1 in
             is-enabled) [ "$3" = ssh ] ;;
             *) exit 1 ;;
           esac"#,
        Some(r#"[ "$1" = -T ] && echo "port 2345""#),
        &[22, 2345],
    );
    assert_eq!(got, "22 no\n2345 yes\n");
}

/// runs a worker over `events` against an api answering `status`; returns (api calls, audit file)
fn run(status: &str, report_all: bool, events: Vec<Event>, tag: &str) -> (usize, String) {
    let (base, seen) = mock_api(status, "", "{}");
    let dir = temp(tag);
    let file = dir.join("reportedIps.txt");
    let worker = Worker::new(
        AllowList::default(),
        Duration::from_secs(900),
        vec![abuse(&base)],
        None,
    )
    .report_all(report_all)
    .reported_to(Some(ReportedFile::new(file.clone(), u64::MAX)));
    let (tx, rx) = mpsc::sync_channel(64);
    for e in events {
        tx.send(e).unwrap();
    }
    drop(tx);
    worker.run(rx);
    let audit = fs::read_to_string(&file).unwrap_or_default();
    fs::remove_dir_all(dir).unwrap();
    let calls = seen.lock().unwrap().len();
    (calls, audit)
}

fn silent(ip: &str) -> Event {
    Event::Accept {
        t: 1,
        ip: ip.parse().unwrap(),
        port: 1,
        ssh: false,
    }
}

#[test]
fn the_audit_file_holds_what_the_api_took_and_nothing_else() {
    let events = || {
        vec![
            accept(1, "8.8.8.8"),
            accept(1, "8.8.8.8"),
            accept(1, "1.1.1.1"),
        ]
    };
    let (calls, audit) = run("200 OK", false, events(), "ok");
    assert_eq!(calls, 2, "the repeat is deduped");
    let lines: Vec<_> = audit.lines().collect();
    assert_eq!(lines.len(), 2, "{audit}");
    assert!(
        lines[0].ends_with("Z 8.8.8.8") && lines[1].ends_with("Z 1.1.1.1"),
        "{audit}"
    );
    assert_eq!(
        lines[0].len(),
        "2026-01-01T00:00:00Z 8.8.8.8".len(),
        "{audit}"
    );

    let (_, audit) = run("500 Oops", false, events(), "failed");
    assert_eq!(audit, "", "a failed report was not reported");
    let (_, audit) = run("422 No", false, events(), "rejected");
    assert_eq!(audit, "", "neither was a rejected one");
}

#[test]
fn every_connection_gets_reported_only_when_asked_for() {
    let (calls, audit) = run("200 OK", false, vec![silent("8.8.8.8")], "off");
    assert_eq!((calls, audit.as_str()), (0, ""));
    let (calls, audit) = run("200 OK", true, vec![silent("8.8.8.8")], "on");
    assert_eq!(calls, 1);
    assert!(audit.ends_with("Z 8.8.8.8\n"), "{audit}");
}
