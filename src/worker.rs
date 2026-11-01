//! single worker thread: filter and dedupe visitors, hand fresh ones to the sinks, record events.
//! slow or dead apis only ever block this thread, never the tarpit.

use crate::{
    events::EventLog,
    ip::{is_reserved, network, AllowList},
    reported::ReportedFile,
    sinks::{Outcome, Sink, Visit},
    tarpit::{now_unix, Event},
    throttle::Throttled,
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

/// a v6 flood can leave a key per /64 in the dedupe window: past this, the map is thinned at once
/// instead of waiting for the 60s prune, so the memory limit holds. dropping a key only means a
/// repeat gets sent again, and the api answers a repeat without spending quota
const SEEN_CAP: usize = 250_000;

fn prune_seen(seen: &mut HashMap<IpAddr, Instant>, dedupe: Duration, cap: usize) {
    seen.retain(|_, s| s.elapsed() < dedupe);
    if seen.len() > cap {
        // every key is still inside the window: give up dedupe rather than the memory limit
        log!("warning: {cap} networks are inside the dedupe window, forgetting them early");
        seen.clear();
    }
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
        let due = self.seen.len() > 5000 && self.last_prune.elapsed() >= Duration::from_secs(60);
        if due || self.seen.len() > SEEN_CAP {
            self.last_prune = Instant::now();
            prune_seen(&mut self.seen, dedupe, SEEN_CAP);
        }
    }
}

pub struct Worker {
    allow: AllowList,
    dedupe: Duration,
    sinks: Vec<Tracked>,
    events: Option<EventLog>,
    report_all: bool,
    reported: Option<ReportedFile>,
    skipped: Throttled,
    not_ssh: Throttled,
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
            report_all: false,
            reported: None,
            skipped: Throttled::default(),
            not_ssh: Throttled::default(),
        }
    }

    /// also report clients that never said `SSH-`, as the old reporter did. see REPORT_ALL_CONNECTIONS in the readme
    pub fn report_all(mut self, on: bool) -> Self {
        self.report_all = on;
        self
    }

    /// keep an audit file of what the api accepted
    pub fn reported_to(mut self, file: Option<ReportedFile>) -> Self {
        self.reported = file;
        self
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
            // attacker-paced: one line a minute says the same, and a flood cannot evict the reports below
            self.skipped.hit(|n| match n {
                1 => log!("connect {ip} -> skipped ({why})"),
                n => log!("{n} connections skipped, last {ip} ({why})"),
            });
            return;
        }
        if !ssh && !self.report_all {
            self.not_ssh.hit(|n| match n {
                1 => log!("connect {ip} -> skipped (not an ssh client)"),
                n => log!("{n} connections skipped (not an ssh client), last {ip}"),
            });
            return self.record(t, ip, Outcome::NotSsh);
        }
        let visit = Visit { t, ip, port };
        let dedupe = self.dedupe;
        let outcomes: Vec<_> = self
            .sinks
            .iter_mut()
            .map(|s| s.visit(&visit, dedupe))
            .collect();
        if outcomes
            .iter()
            .flatten()
            .any(|o| matches!(o, Outcome::Reported(_)))
        {
            if let Some(file) = self.reported.as_mut() {
                file.add(now_unix(), ip);
            }
        }
        match outcomes[0] {
            None => {
                let dedupe = format!("reported < {}s ago", dedupe.as_secs());
                self.skipped.hit(|n| match n {
                    1 => log!("connect {ip} -> skipped ({dedupe})"),
                    n => log!("{n} connections skipped, last {ip} ({dedupe})"),
                });
                self.record(t, ip, Outcome::Dedup);
            }
            Some(primary) => {
                // a rejection was logged with the api's reason already
                if primary != Outcome::Rejected {
                    log!("connect {ip} -> {}", primary.describe());
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

#[cfg(test)]
mod tests {
    use super::*;

    fn net(i: u32) -> IpAddr {
        IpAddr::V4(std::net::Ipv4Addr::from(i))
    }

    #[test]
    fn a_full_window_is_thinned_rather_than_grown() {
        let dedupe = Duration::from_secs(900);
        let mut seen: HashMap<IpAddr, Instant> =
            (0..10).map(|i| (net(i), Instant::now())).collect();
        prune_seen(&mut seen, dedupe, 4);
        assert!(
            seen.is_empty(),
            "all keys fresh: dedupe gives way, not the memory limit"
        );

        // under the cap this is only the usual age prune
        let mut seen: HashMap<IpAddr, Instant> = HashMap::new();
        seen.insert(net(1), Instant::now() - Duration::from_secs(901));
        seen.insert(net(2), Instant::now());
        prune_seen(&mut seen, dedupe, 4);
        assert_eq!(seen.len(), 1, "the expired key went, the fresh one stayed");
    }

    #[test]
    fn a_flood_drops_keys_at_the_cap_without_waiting_for_the_minute() {
        let dedupe = Duration::from_secs(900);
        let mut t = Tracked {
            sink: Box::new(Dry),
            seen: HashMap::new(),
            last_prune: Instant::now(),
        };
        for i in 0..SEEN_CAP + 1 {
            t.mark_seen(net(i as u32), dedupe);
        }
        assert!(t.seen.len() <= SEEN_CAP + 1, "capped, not 250k and growing");
    }
}
