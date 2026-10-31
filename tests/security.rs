//! security properties against the real binary: no internet, no privileged ports

// each test file uses part of it
#[allow(dead_code)]
mod common;

use common::{as_systemd, command, free_port, serial, visit, Running, BIN};
use std::{
    io::{Read, Write},
    net::TcpListener,
    os::fd::OwnedFd,
    process::Stdio,
    thread,
    time::{Duration, Instant},
};

/// bytes the kernel holds unread on the tarpit's side of the connection from `client`
#[cfg(target_os = "linux")]
fn unread(pid: u32, client: u16) -> Option<usize> {
    let remote = format!(":{client:04X}");
    ["tcp", "tcp6"].iter().find_map(|table| {
        let table = std::fs::read_to_string(format!("/proc/{pid}/net/{table}")).ok()?;
        // sl local_address rem_address st tx_queue:rx_queue ...
        table.lines().find_map(|line| {
            let f: Vec<_> = line.split_whitespace().collect();
            let queue = f[4].split_once(':')?.1;
            f[2].ends_with(&remote)
                .then(|| usize::from_str_radix(queue, 16).unwrap())
        })
    })
}

#[cfg(target_os = "macos")]
fn unread(pid: u32, client: u16) -> Option<usize> {
    let out = std::process::Command::new("/usr/sbin/lsof")
        .args(["-nP", "-a", "-iTCP", "-T", "q", "-p", &pid.to_string()])
        .output()
        .unwrap();
    // ... TCP 127.0.0.1:2222->127.0.0.1:50000 (QR=38 QS=0)
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let line = text.lines().find(|l| l.contains(&format!(":{client} ")))?;
    let queue = line.split("QR=").nth(1)?.split(' ').next()?;
    queue.parse().ok()
}

/// the property the whole design rests on: what a client sends stays in the kernel, unread
#[test]
fn never_reads_what_a_client_sends() {
    let _serial = serial();
    let port = free_port();
    let mut t = Running::start(command(BIN).env("TARPIT_PORT", port.to_string()));
    let mut c = visit(port); // banner seen, so the tarpit holds this connection from here on
    let (pid, client) = (t.child.id(), c.local_addr().unwrap().port());
    let hello = b"SSH-2.0-OpenSSH_9.9\r\n\0\xff\xffINJECT\r\n";
    c.write_all(hello).unwrap();

    // anything reading the socket, even to discard, would empty this queue
    let deadline = Instant::now() + Duration::from_secs(5);
    while unread(pid, client) != Some(hello.len()) {
        assert!(Instant::now() < deadline, "{:?}", unread(pid, client));
        thread::sleep(Duration::from_millis(10));
    }
    thread::sleep(Duration::from_millis(300)); // about 10 banner ticks
    assert_eq!(unread(pid, client), Some(hello.len()), "input consumed");

    // no answer, no crash: only more banner lines
    let mut got = vec![0u8; 4096];
    let n = c.read(&mut got).unwrap();
    let echoed = |needle: &[u8]| got[..n].windows(needle.len()).any(|w| w == needle);
    assert!(n > 0 && !echoed(b"SSH-2.0") && !echoed(b"INJECT"));
    assert!(t.child.try_wait().unwrap().is_none(), "crashed");
}

#[test]
fn every_slot_frees_up_when_clients_leave() {
    let _serial = serial();
    let port = free_port();
    let _t = Running::start(
        command(BIN)
            .env("TARPIT_PORT", port.to_string())
            .env("TARPIT_MAX_CLIENTS", "4"),
    );
    // a leaked slot stalls a later round: its client waits in the backlog until the read times out
    for _ in 0..3 {
        let clients: Vec<_> = (0..4).map(|_| visit(port)).collect();
        drop(clients);
    }
}

#[test]
fn uses_the_socket_systemd_passed() {
    let _serial = serial();
    let socket = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = socket.local_addr().unwrap().port();
    let t = Running::start(&mut as_systemd("1", OwnedFd::from(socket).into()));
    assert!(
        t.startup.contains(&format!("on 127.0.0.1:{port} ")),
        "{}",
        t.startup
    );
    visit(port);
}

#[test]
fn ignores_activation_env_not_meant_for_it() {
    let _serial = serial();
    let others_pid = std::process::id().to_string();
    let mut cases = vec![
        as_systemd("0", Stdio::null()),
        as_systemd("junk", Stdio::null()),
    ];
    for pid in [others_pid.as_str(), "junk", ""] {
        let mut c = command(BIN);
        c.env("LISTEN_PID", pid).env("LISTEN_FDS", "1");
        cases.push(c);
    }
    for mut cmd in cases {
        let port = free_port();
        let t = Running::start(cmd.env("TARPIT_PORT", port.to_string()));
        assert!(t.startup.contains(&format!(":{port} ")), "{}", t.startup);
        visit(port);
    }
}

#[test]
fn refuses_a_passed_fd_that_is_not_a_socket() {
    let _serial = serial();
    let mut child = as_systemd("1", Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    // bounded: the failure this guards against is a process that keeps running with a dead listener
    let exited = (0..500).find_map(|_| {
        thread::sleep(Duration::from_millis(10));
        child.try_wait().unwrap()
    });
    let _ = child.kill();
    let _ = child.wait();
    assert_eq!(exited.and_then(|s| s.code()), Some(1));
}
