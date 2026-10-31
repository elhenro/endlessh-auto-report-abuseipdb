//! generic sink: POST one json object per fresh visitor to any url
//!   {"event":"accept","t":1760000000,"ip":"1.2.3.4","port":51234}

use crate::sinks::{Backoff, Outcome, Sink, Visit};
use std::time::Duration;
use ureq::{Agent, AgentBuilder, Error};

pub struct Webhook {
    agent: Agent,
    url: String,
    auth: Option<String>, // full Authorization header value, e.g. "Bearer abc"
    backoff: Backoff,
}

impl Webhook {
    pub fn new(url: String, auth: Option<String>, timeout: Duration) -> Self {
        let agent = AgentBuilder::new()
            .timeout_connect(Duration::from_secs(5))
            .timeout(timeout)
            .redirects(0)
            .build();
        Self {
            agent,
            url,
            auth,
            backoff: Backoff::default(),
        }
    }

    fn fail(&mut self, why: &str) -> Outcome {
        log!("webhook {why}, pausing 30s");
        self.backoff.set(30);
        Outcome::Failed
    }
}

pub fn body(v: &Visit) -> String {
    // every value is a number or a formatted ip, so no json escaping is needed
    format!(
        "{{\"event\":\"accept\",\"t\":{},\"ip\":\"{}\",\"port\":{}}}",
        v.t, v.ip, v.port
    )
}

impl Sink for Webhook {
    fn name(&self) -> &'static str {
        "webhook"
    }

    fn send(&mut self, v: &Visit) -> Outcome {
        if self.backoff.remaining().is_some() {
            return Outcome::Paused;
        }
        let mut req = self
            .agent
            .post(&self.url)
            .set("Content-Type", "application/json");
        if let Some(a) = &self.auth {
            req = req.set("Authorization", a);
        }
        // log status codes only: the url itself may carry a secret
        match req.send_string(&body(v)) {
            // redirects are off, so a 3xx arrives here too: nothing was delivered
            Ok(r) if r.status() < 300 => Outcome::Delivered,
            Ok(r) => self.fail(&format!("answered HTTP {}", r.status())),
            Err(Error::Status(c, _)) => self.fail(&format!("answered HTTP {c}")),
            Err(e) => self.fail(&format!("unreachable ({})", e.kind())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_is_valid_shape() {
        let v = Visit {
            t: 5,
            ip: "2001:db8::1".parse().unwrap(),
            port: 4242,
        };
        assert_eq!(
            body(&v),
            r#"{"event":"accept","t":5,"ip":"2001:db8::1","port":4242}"#
        );
    }
}
