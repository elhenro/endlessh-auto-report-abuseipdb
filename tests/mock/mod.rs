//! local mock api plus a driver that runs the worker over a list of events

use endlessh_reporter::{
    abuseipdb::AbuseIpdb, events::EventLog, ip::AllowList, sinks::Sink, tarpit::Event,
    worker::Worker,
};
use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    path::PathBuf,
    sync::{mpsc, Arc, Mutex},
    thread,
    time::Duration,
};

pub type Requests = Arc<Mutex<Vec<String>>>;

/// answers every request with `status`, `extra` headers and `body`, and records the raw request text
pub fn mock_api(status: &str, extra: &'static str, body: &'static str) -> (String, Requests) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let seen: Requests = Arc::default();
    let (store, status) = (seen.clone(), status.to_string());
    thread::spawn(move || {
        for mut conn in listener.incoming().flatten() {
            let mut buf = Vec::new();
            let mut chunk = [0u8; 1024];
            let body_at = loop {
                let n = conn.read(&mut chunk).unwrap_or(0);
                buf.extend_from_slice(&chunk[..n]);
                if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    break p + 4;
                }
                if n == 0 {
                    break buf.len();
                }
            };
            let head = String::from_utf8_lossy(&buf[..body_at]).to_lowercase();
            let want: usize = head
                .split("content-length: ")
                .nth(1)
                .and_then(|r| r.split("\r\n").next()?.parse().ok())
                .unwrap_or(0);
            while buf.len() < body_at + want {
                let n = conn.read(&mut chunk).unwrap_or(0);
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&chunk[..n]);
            }
            store
                .lock()
                .unwrap()
                .push(String::from_utf8_lossy(&buf).into_owned());
            let _ = write!(
                conn,
                "HTTP/1.1 {status}\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    (base, seen)
}

fn temp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("e2e-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    dir
}

pub fn accept(t: u64, ip: &str) -> Event {
    Event::Accept {
        t,
        ip: ip.parse().unwrap(),
        port: 40000,
    }
}

/// runs the worker to completion over `events` and returns the event log lines
pub fn run_worker(
    sinks: Vec<Box<dyn Sink>>,
    allow: &str,
    events: Vec<Event>,
    tag: &str,
) -> Vec<String> {
    let dir = temp(tag);
    let log = EventLog::open(dir.clone(), 30, u64::MAX).unwrap();
    let worker = Worker::new(
        AllowList::parse(allow).unwrap(),
        Duration::from_secs(900),
        sinks,
        Some(log),
    );
    let (tx, rx) = mpsc::sync_channel(64);
    for e in events {
        tx.send(e).unwrap();
    }
    drop(tx);
    worker.run(rx);
    let mut lines = Vec::new();
    for entry in fs::read_dir(&dir).unwrap() {
        lines.extend(
            fs::read_to_string(entry.unwrap().path())
                .unwrap()
                .lines()
                .map(String::from),
        );
    }
    fs::remove_dir_all(dir).unwrap();
    lines
}

pub fn abuse(base: &str) -> Box<dyn Sink> {
    Box::new(AbuseIpdb::new(
        base,
        "tok".into(),
        "14,22".into(),
        Duration::from_secs(5),
    ))
}
