//! the tarpit threads over real loopback sockets

use endlessh_reporter::tarpit::{self, Event, Params};
use std::{
    io::Read,
    net::{IpAddr, TcpListener, TcpStream},
    sync::mpsc,
    time::Duration,
};

fn start_tarpit(max_clients: usize) -> (u16, mpsc::Receiver<Event>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (tx, rx) = mpsc::sync_channel(64);
    let params = Params {
        delay: Duration::from_millis(30),
        max_clients,
        max_per_ip: 1000,
        max_line: 32,
    };
    tarpit::spawn(listener, params, tx); // threads run for the rest of the test process
    (port, rx)
}

fn expect_close(rx: &mpsc::Receiver<Event>) -> (IpAddr, u64) {
    loop {
        match rx
            .recv_timeout(Duration::from_secs(5))
            .expect("close event")
        {
            Event::Close { ip, bytes, .. } => return (ip, bytes),
            Event::Accept { .. } => {}
        }
    }
}

#[test]
fn tarpit_dribbles_lines_and_reports_close() {
    let (port, rx) = start_tarpit(10);
    let mut c = TcpStream::connect(("127.0.0.1", port)).unwrap();
    c.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let mut got = Vec::new();
    let mut buf = [0u8; 256];
    while got.len() < 20 {
        let n = c.read(&mut buf).unwrap();
        assert!(n > 0);
        got.extend_from_slice(&buf[..n]);
    }
    assert!(!got.starts_with(b"SSH-") && got.windows(2).any(|w| w == b"\r\n"));
    assert!(
        matches!(rx.recv_timeout(Duration::from_secs(2)).unwrap(), Event::Accept { ip, .. } if ip.is_loopback())
    );
    drop(c);
    let (ip, bytes) = expect_close(&rx);
    assert!(ip.is_loopback() && bytes >= 20);
}

#[test]
fn tarpit_stops_accepting_at_max_clients() {
    let (port, rx) = start_tarpit(1);
    let first = TcpStream::connect(("127.0.0.1", port)).unwrap();
    let _second = TcpStream::connect(("127.0.0.1", port)).unwrap(); // sits in the kernel backlog
    assert!(matches!(
        rx.recv_timeout(Duration::from_secs(2)).unwrap(),
        Event::Accept { .. }
    ));
    assert!(
        rx.recv_timeout(Duration::from_millis(400)).is_err(),
        "second client must wait while the first is held"
    );
    drop(first);
    let (_, _) = expect_close(&rx);
    assert!(
        matches!(
            rx.recv_timeout(Duration::from_secs(5)).unwrap(),
            Event::Accept { .. }
        ),
        "slot freed, second client accepted"
    );
}
