//! what the worker does with each kind of answer from an api

mod mock;

use endlessh_reporter::{
    sinks::{Outcome, Sink, Visit},
    webhook::Webhook,
};
use mock::{abuse, accept, mock_api, run_worker};
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};

const T: u64 = 1_760_000_000;

// body per https://docs.abuseipdb.com/#report-endpoint
const REPEAT: &str = r#"{"errors":[{"detail":"You can only report the same IP address (`8.8.8.8`) once in 15 minutes.","status":429,"source":{"parameter":"ip"}}]}"#;

fn actions(lines: &[String]) -> Vec<&str> {
    lines
        .iter()
        .map(|l| {
            l.split("\"a\":\"")
                .nth(1)
                .unwrap()
                .split('"')
                .next()
                .unwrap()
        })
        .collect()
}

#[test]
fn quota_exhaustion_pauses_instead_of_hammering() {
    let (base, seen) = mock_api("429 Too Many Requests", "Retry-After: 100\r\n", "{}");
    let events = vec![
        accept(T, "8.8.8.8"),
        accept(T, "1.1.1.1"),
        accept(T, "9.9.9.9"),
    ];
    let lines = run_worker(vec![abuse(&base)], "", events, "quota");
    assert_eq!(actions(&lines), ["failed", "paused", "paused"]);
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[test]
fn a_repeat_report_settles_that_ip_and_pauses_nothing() {
    let (base, seen) = mock_api("429 Too Many Requests", "", REPEAT);
    let events = vec![
        accept(T, "8.8.8.8"),
        accept(T, "8.8.8.8"),
        accept(T, "1.1.1.1"),
    ];
    let lines = run_worker(vec![abuse(&base)], "", events, "repeat");
    assert_eq!(actions(&lines), ["dedup", "dedup", "dedup"]);
    assert_eq!(
        seen.lock().unwrap().len(),
        2,
        "8.8.8.8 is not retried, 1.1.1.1 still goes out"
    );
}

#[test]
fn a_429_with_quota_left_is_no_quota_problem_whatever_the_body_says() {
    let (base, seen) = mock_api(
        "429 Too Many Requests",
        "X-RateLimit-Remaining: 500\r\n",
        "{}",
    );
    let events = vec![accept(T, "8.8.8.8"), accept(T, "1.1.1.1")];
    let lines = run_worker(vec![abuse(&base)], "", events, "quota-left");
    assert_eq!(actions(&lines), ["dedup", "dedup"]);
    assert_eq!(seen.lock().unwrap().len(), 2);
}

#[test]
fn a_redirect_is_no_report() {
    let (base, seen) = mock_api("302 Found", "Location: http://127.0.0.1:1/\r\n", "{}");
    let events = vec![accept(T, "8.8.8.8"), accept(T, "1.1.1.1")];
    let lines = run_worker(vec![abuse(&base)], "", events, "redirect");
    assert_eq!(actions(&lines), ["failed", "paused"]);
    assert_eq!(seen.lock().unwrap().len(), 1, "not followed, not retried");
}

#[test]
fn a_redirect_is_no_delivery() {
    let (base, seen) = mock_api("302 Found", "Location: http://127.0.0.1:1/\r\n", "{}");
    let hook = Box::new(Webhook::new(base, None, Duration::from_secs(5)));
    let events = vec![accept(T, "8.8.8.8"), accept(T, "1.1.1.1")];
    let lines = run_worker(vec![hook], "", events, "hook-redirect");
    assert_eq!(actions(&lines), ["failed", "paused"]);
    assert_eq!(seen.lock().unwrap().len(), 1, "not followed, not retried");
}

#[test]
fn an_interim_answer_is_no_report() {
    let (base, _) = mock_api("100 Continue", "", "{}");
    let events = vec![accept(T, "8.8.8.8"), accept(T, "1.1.1.1")];
    let lines = run_worker(vec![abuse(&base)], "", events, "interim");
    assert_eq!(actions(&lines), ["failed", "paused"]);
    let (base, _) = mock_api("100 Continue", "", "{}");
    let hook = Box::new(Webhook::new(base, None, Duration::from_secs(5)));
    let events = vec![accept(T, "8.8.8.8")];
    let lines = run_worker(vec![hook], "", events, "hook-interim");
    assert_eq!(actions(&lines), ["failed"]);
}

fn hook(base: String) -> Box<Webhook> {
    Box::new(Webhook::new(base, None, Duration::from_secs(5)))
}

#[test]
fn a_paused_api_does_not_make_the_webhook_repeat_itself() {
    let (api, api_seen) = mock_api("429 Too Many Requests", "Retry-After: 100\r\n", "{}");
    let (url, hook_seen) = mock_api("200 OK", "", "{}");
    let events = vec![
        accept(T, "8.8.8.8"),
        accept(T, "8.8.8.8"),
        accept(T, "8.8.8.8"),
        accept(T, "1.1.1.1"),
    ];
    let lines = run_worker(vec![abuse(&api), hook(url)], "", events, "both-paused");
    assert_eq!(actions(&lines), ["failed", "paused", "paused", "paused"]);
    assert_eq!(api_seen.lock().unwrap().len(), 1);
    assert_eq!(
        hook_seen.lock().unwrap().len(),
        2,
        "each fresh visitor once"
    );
}

/// fails the first time, works after that; counts what reaches it
struct Flaky(Arc<AtomicUsize>);

impl Sink for Flaky {
    fn name(&self) -> &'static str {
        "flaky"
    }

    fn send(&mut self, _: &Visit) -> Outcome {
        match self.0.fetch_add(1, Ordering::SeqCst) {
            0 => Outcome::Failed,
            _ => Outcome::Delivered,
        }
    }
}

#[test]
fn a_sink_that_failed_is_retried_while_the_settled_one_is_left_alone() {
    let (api, api_seen) = mock_api("200 OK", "", "{}");
    let calls = Arc::new(AtomicUsize::new(0));
    let events = vec![
        accept(T, "8.8.8.8"),
        accept(T, "8.8.8.8"),
        accept(T, "8.8.8.8"),
    ];
    let sinks: Vec<Box<dyn Sink>> = vec![abuse(&api), Box::new(Flaky(calls.clone()))];
    let lines = run_worker(sinks, "", events, "both-failed");
    assert_eq!(actions(&lines), ["reported", "dedup", "dedup"]);
    assert_eq!(api_seen.lock().unwrap().len(), 1, "settled after one");
    assert_eq!(calls.load(Ordering::SeqCst), 2, "failed, then delivered");
}

#[test]
fn rotating_within_a_v6_64_burns_one_report() {
    let (base, seen) = mock_api("200 OK", "X-RateLimit-Remaining: 42\r\n", "{}");
    let events = vec![
        accept(T, "2a00:1450::1"),
        accept(T, "2a00:1450::2"),
        accept(T, "2a00:1450::ffff:1"),
        accept(T, "2a00:1450:0:1::1"), // next /64
    ];
    let lines = run_worker(vec![abuse(&base)], "", events, "v6");
    assert_eq!(actions(&lines), ["reported", "dedup", "dedup", "reported"]);
    assert_eq!(seen.lock().unwrap().len(), 2);
}
