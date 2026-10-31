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

/// stands in when nothing is configured, so dedupe and the event log work the same
struct Dry;

impl Sink for Dry {
    fn name(&self) -> &'static str {
        "dry"
    }

    fn send(&mut self, _: &Visit) -> Outcome {
        Outcome::Dry
    }
}

/// one sink with its own memory: another sink being paused or down must not make this one repeat or skip visits
struct Tracked {
    sink: Box<dyn Sink>,
    seen: HashMap<IpAddr, Instant>, // by network: rotating within a v6 /64 must not burn quota
    last_prune: Instant,
}

impl Tracked {
    /// None: this network went through here within the dedupe window
    fn visit(&mut self, v: &Visit, dedupe: Duration) -> Option<Outcome> {
        let net = network(v.ip);
        if self.seen.get(&net).is_some_and(|s| s.elapsed() < dedupe) {
            return None;
        }
        let outcome = self.sink.send(v);
        if outcome.settled() {
            self.mark_seen(net, dedupe);
        }
        Some(outcome)
    }

    fn mark_seen(&mut self, net: IpAddr, dedupe: Duration) {
        self.seen.insert(net, Instant::now());
        if self.seen.len() > 5000 && self.last_prune.elapsed() >= Duration::from_secs(60) {
            self.last_prune = Instant::now();
            self.seen.retain(|_, s| s.elapsed() < dedupe);
        }
    }
}

pub struct Worker {
    allow: AllowList,
    dedupe: Duration,
    sinks: Vec<Tracked>,
    events: Option<EventLog>,
}

impl Worker {
    /// the first sink decides the "a" field of the event; the rest still get every visit
    pub fn new(
        allow: AllowList,
        dedupe: Duration,
        mut sinks: Vec<Box<dyn Sink>>,
        events: Option<EventLog>,
    ) -> Self {
        if sinks.is_empty() {
            sinks.push(Box::new(Dry));
        }
        Self {
            allow,
            dedupe,
            sinks: sinks
                .into_iter()
                .map(|sink| Tracked {
                    sink,
                    seen: HashMap::new(),
                    last_prune: Instant::now(),
                })
                .collect(),
            events,
        }
    }

    /// returns once every sender is gone
    pub fn run(mut self, rx: Receiver<Event>) {
        for ev in rx {
            match ev {
                Event::Accept { t, ip, port, ssh } => self.accept(t, ip, port, ssh),
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

    fn accept(&mut self, t: u64, ip: IpAddr, port: u16, ssh: bool) {
        if let Some(why) = self.ignored(ip) {
            log!("connect {ip} -> skipped ({why})");
            return;
        }
        if !ssh {
            log!("connect {ip} -> skipped (not an ssh client)");
            return self.record(t, ip, Outcome::NotSsh);
        }
        let visit = Visit { t, ip, port };
        let dedupe = self.dedupe;
        let outcomes: Vec<_> = self
            .sinks
            .iter_mut()
            .map(|s| s.visit(&visit, dedupe))
            .collect();
        match outcomes[0] {
            None => {
                log!(
                    "connect {ip} -> skipped (reported < {}s ago)",
                    dedupe.as_secs()
                );
                self.record(t, ip, Outcome::Dedup);
            }
            Some(primary) => {
                match primary.quota() {
                    Some(q) => log!("connect {ip} -> {} (quota left: {q})", primary.action()),
                    None => log!("connect {ip} -> {}", primary.action()),
                }
                self.record(t, ip, primary);
            }
        }
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
}
