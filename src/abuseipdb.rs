//! AbuseIPDB v2 client: https://docs.abuseipdb.com/#report-endpoint

use crate::sinks::{is_success, Backoff, Outcome, Sink, Visit};
use std::{io::Read, time::Duration};
use ureq::{Agent, AgentBuilder, Error, Request, Response};

pub struct AbuseIpdb {
    agent: Agent,
    base: String,
    token: String,
    categories: String,
    backoff: Backoff,
}

impl AbuseIpdb {
    pub fn new(base: &str, token: String, categories: String, timeout: Duration) -> Self {
        // no redirects: the Key header must never follow one to another host
        let agent = AgentBuilder::new()
            .timeout_connect(Duration::from_secs(5))
            .timeout(timeout)
            .redirects(0)
            .build();
        Self {
            agent,
            base: base.trim_end_matches('/').to_string(),
            token,
            categories,
            backoff: Backoff::default(),
        }
    }

    fn authed(&self, req: Request) -> Request {
        req.set("Key", &self.token)
            .set("Accept", "application/json")
    }

    /// a rejected token is fatal at startup: a loud crash beats silent no-op reporting
    pub fn check_token(&self) -> Result<(), String> {
        let req = self
            .agent
            .get(&format!("{}/check", self.base))
            .query("ipAddress", "127.0.0.1");
        match self.authed(req).call() {
            Ok(r) if is_success(r.status()) => log!("AbuseIPDB token ok"),
            Ok(r) => log!(
                "warning: token check returned HTTP {}, continuing",
                r.status()
            ),
            Err(Error::Status(c @ (401 | 403), _)) => {
                return Err(format!("AbuseIPDB rejected the API token (HTTP {c})"))
            }
            Err(Error::Status(c, _)) => log!("warning: token check returned HTTP {c}, continuing"),
            Err(_) => log!("warning: cannot reach AbuseIPDB right now, continuing"),
        }
        Ok(())
    }

    fn pause(&mut self, secs: u64, why: &str) {
        self.backoff.set(secs);
        log!("pausing reports for {}s: {why}", secs.min(86_400));
    }
}

/// start of an error body as one log line: raw newlines or escapes in remote text could forge entries.
/// blanked, not cut at the first newline: pretty-printed json would log as just "{"
fn snippet(resp: Response) -> String {
    let mut body = Vec::new();
    let _ = resp.into_reader().take(200).read_to_end(&mut body);
    body.iter()
        .map(|&b| if b.is_ascii_graphic() { b as char } else { ' ' })
        .collect()
}

/// a 429 means a repeat of an ip reported in the last 15 minutes or the daily quota.
/// a repeat says nothing about quota, and pausing on it would stop all reporting: dedupe here
/// lives in memory, so a restart, a second host on this key or a timed out report all land on it
fn is_repeat(resp: Response) -> bool {
    let quota_left = resp
        .header("x-ratelimit-remaining")
        .and_then(|h| h.trim().parse::<u64>().ok())
        .is_some_and(|n| n > 0);
    quota_left || snippet(resp).to_lowercase().contains("same ip")
}

impl Sink for AbuseIpdb {
    fn name(&self) -> &'static str {
        "abuseipdb"
    }

    fn send(&mut self, v: &Visit) -> Outcome {
        if let Some(left) = self.backoff.remaining() {
            return Outcome::Paused(left);
        }
        let ip = v.ip.to_string();
        // the same comment the bash version sent, so reports read the same on abuseipdb
        let comment = format!(
            "SSH login attempts (endlessh): ACCEPT host={ip} port={}",
            v.port
        );
        let req = self.authed(self.agent.post(&format!("{}/report", self.base)));
        let res = req.send_form(&[
            ("ip", ip.as_str()),
            ("categories", self.categories.as_str()),
            ("comment", comment.as_str()),
        ]);
        match res {
            Ok(resp) if is_success(resp.status()) => Outcome::Reported(
                resp.header("x-ratelimit-remaining")
                    .and_then(|h| h.trim().parse().ok()),
            ),
            Ok(resp) => {
                self.pause(30, "api unavailable");
                Outcome::Failed(resp.status())
            }
            Err(Error::Status(422, resp)) => {
                log!("connect {ip} -> rejected by AbuseIPDB: {}", snippet(resp));
                Outcome::Rejected
            }
            Err(Error::Status(429, resp)) => {
                let secs = resp
                    .header("retry-after")
                    .and_then(|h| h.trim().parse().ok())
                    .unwrap_or(3600);
                if is_repeat(resp) {
                    return Outcome::Dedup;
                }
                self.pause(secs, "daily quota exhausted");
                Outcome::Failed(429)
            }
            Err(Error::Status(c @ (401 | 403), _)) => {
                self.pause(3600, &format!("token rejected (HTTP {c})"));
                Outcome::Failed(c)
            }
            Err(Error::Status(c, _)) => {
                self.pause(30, "api unavailable");
                Outcome::Failed(c)
            }
            Err(_) => {
                self.pause(30, "api unavailable");
                Outcome::Failed(0)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snippet_of(body: &str) -> String {
        snippet(Response::new(422, "Unprocessable Entity", body).unwrap())
    }

    #[test]
    fn snippet_cannot_forge_log_lines() {
        let s = snippet_of("{\n\"e\":1}\r\ntarpit: connect 8.8.8.8 -> reported\x1b[2K\u{2028}");
        assert!(
            s.bytes().all(|b| b.is_ascii_graphic() || b == b' '),
            "{s:?}"
        );
        assert!(s.starts_with("{ \"e\":1}  tarpit: connect"), "{s:?}");
    }

    #[test]
    fn snippet_is_bounded_and_survives_a_cut_multibyte_char() {
        assert_eq!(snippet_of(&"a".repeat(500)).len(), 200);
        let s = snippet_of(&format!("{}é", "a".repeat(199)));
        assert_eq!(s.len(), 200);
        assert!(s.starts_with("aaa"));
    }
}
