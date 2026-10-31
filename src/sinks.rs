//! a sink gets every fresh visitor once per dedupe window and reports it somewhere

use std::{net::IpAddr, time::Instant};
use url::Url;

pub struct Visit {
    pub t: u64,
    pub ip: IpAddr,
    pub port: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// accepted by AbuseIPDB, with the reports left today
    Reported(Option<u64>),
    /// accepted by a generic sink such as the webhook
    Delivered,
    /// the api refused this ip for good, retrying is pointless
    Rejected,
    /// reported recently, by us or by another host on the same key
    Dedup,
    Paused,
    Failed,
    /// no sink configured
    Dry,
}

impl Outcome {
    /// value of the "a" field in the event log
    pub fn action(self) -> &'static str {
        match self {
            Outcome::Reported(_) => "reported",
            Outcome::Delivered => "delivered",
            Outcome::Rejected => "rejected",
            Outcome::Dedup => "dedup",
            Outcome::Paused => "paused",
            Outcome::Failed => "failed",
            Outcome::Dry => "dry",
        }
    }

    pub fn quota(self) -> Option<u64> {
        match self {
            Outcome::Reported(q) => q,
            _ => None,
        }
    }

    /// done with this ip for the dedupe window; failures retry on the next connect
    pub fn settled(self) -> bool {
        !matches!(self, Outcome::Paused | Outcome::Failed)
    }
}

pub trait Sink: Send {
    fn name(&self) -> &'static str;
    fn send(&mut self, visit: &Visit) -> Outcome;
}

/// pause a sink after errors instead of hammering a struggling api
#[derive(Default)]
pub struct Backoff(Option<Instant>);

impl Backoff {
    pub fn set(&mut self, secs: u64) {
        self.0 = Some(Instant::now() + std::time::Duration::from_secs(secs.min(86_400)));
    }

    pub fn remaining(&self) -> Option<u64> {
        let left = self.0?.checked_duration_since(Instant::now())?;
        Some(left.as_secs() + 1)
    }
}

/// https only, except for loopback so tests and local receivers work.
/// asks the parser ureq connects with: prefix checks wave http://localhost:80@evil.example/ through
pub fn check_url(name: &str, url: &str) -> Result<(), String> {
    let allowed = Url::parse(url).is_ok_and(|u| match u.scheme() {
        "https" => true,
        "http" => matches!(u.host_str(), Some("127.0.0.1" | "localhost" | "[::1]")),
        _ => false,
    });
    if allowed {
        Ok(())
    } else {
        Err(format!(
            "{name} must be an https:// url (http:// only for localhost)"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_policy() {
        for ok in [
            "https://api.abuseipdb.com/api/v2",
            "http://127.0.0.1:8080/x",
            "http://localhost/",
            "http://[::1]:9",
        ] {
            assert!(check_url("U", ok).is_ok(), "{ok}");
        }
        for bad in [
            "http://example.com",
            "http://127.0.0.1.evil.com/",
            "ftp://x",
            "http://localhost.evil.com",
            "http://localhost:80@evil.example/",
            "http://127.0.0.1:1@evil.example/api",
            "http://[::1]:1@evil.example/",
            "https://",
            "",
        ] {
            assert!(check_url("U", bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn backoff_expires_and_caps() {
        let mut b = Backoff::default();
        assert_eq!(b.remaining(), None);
        b.set(30);
        assert!(b.remaining().is_some_and(|s| (29..=31).contains(&s)));
        b.set(10_000_000);
        assert!(b.remaining().is_some_and(|s| s <= 86_401));
    }
}
