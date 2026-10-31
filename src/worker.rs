//! single worker thread: filter and dedupe visitors, hand fresh ones to the sinks, record events.
//! slow or dead apis only ever block this thread, never the tarpit.

use crate::{
    events::EventLog,
    ip::{is_reserved, network, AllowList},
    sinks::{Outcome, Sink, Visit},
    tarpit::Event,
};
use std::{
    collections::HashMap,
    net::IpAddr,
    sync::mpsc::Receiver,
    time::{Duration, Instant},
};

pub struct Worker {
    allow: AllowList,
    dedupe: Duration,
    sinks: Vec<Box<dyn Sink>>,
    events: Option<EventLog>,
    seen: HashMap<IpAddr, Instant>, // by network: rotating within a v6 /64 must not burn quota
    last_prune: Instant,
}

impl Worker {
    /// the first sink decides the "a" field of the event; the rest still get every visit
    pub fn new(
        allow: AllowList,
        dedupe: Duration,
        sinks: Vec<Box<dyn Sink>>,
        events: Option<EventLog>,
    ) -> Self {
        Self {
            allow,
            dedupe,
            sinks,
            events,
            seen: HashMap::new(),
            last_prune: Instant::now(),
        }
    }

    /// returns once every sender is gone
    pub fn run(mut self, rx: Receiver<Event>) {
        for ev in rx {
            match ev {
                Event::Accept { t, ip, port } => self.accept(t, ip, port),
                Event::Close { t, ip, secs, bytes } => self.close(t, ip, secs, bytes),
            }
        }
    }

    fn ignored(&self, ip: IpAddr) -> Option<&'static str> {
        if is_reserved(ip) {
            Some("private/reserved address")
        } else if self.allow.contains(ip) {
            Some("allowlisted")
        } else {
            None
        }
    }

    fn accept(&mut self, t: u64, ip: IpAddr, port: u16) {
        if let Some(why) = self.ignored(ip) {
            log!("connect {ip} -> skipped ({why})");
            return;
        }
        if self
            .seen
            .get(&network(ip))
            .is_some_and(|s| s.elapsed() < self.dedupe)
        {
            log!(
                "connect {ip} -> skipped (reported < {}s ago)",
                self.dedupe.as_secs()
            );
            return self.record(t, ip, Outcome::Dedup);
        }
        let visit = Visit { t, ip, port };
        let outcomes: Vec<Outcome> = self.sinks.iter_mut().map(|s| s.send(&visit)).collect();
        let primary = outcomes.first().copied().unwrap_or(Outcome::Dry);
        match primary.quota() {
            Some(q) => log!("connect {ip} -> {} (quota left: {q})", primary.action()),
            None => log!("connect {ip} -> {}", primary.action()),
        }
        if primary.settled() {
            self.mark_seen(ip);
        }
        self.record(t, ip, primary);
    }

    fn record(&mut self, t: u64, ip: IpAddr, outcome: Outcome) {
        if let Some(log) = self.events.as_mut() {
            log.accept(t, ip, outcome.action(), outcome.quota());
        }
    }

    fn close(&mut self, t: u64, ip: IpAddr, secs: f64, bytes: u64) {
        if self.ignored(ip).is_some() {
            return;
        }
        if let Some(log) = self.events.as_mut() {
            log.close(t, ip, secs, bytes);
        }
    }

    fn mark_seen(&mut self, ip: IpAddr) {
        self.seen.insert(network(ip), Instant::now());
        if self.seen.len() > 5000 && self.last_prune.elapsed() >= Duration::from_secs(60) {
            self.last_prune = Instant::now();
            let keep = self.dedupe;
            self.seen.retain(|_, s| s.elapsed() < keep);
        }
    }
}
