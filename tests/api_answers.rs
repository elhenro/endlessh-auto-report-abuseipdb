//! what the worker does with each kind of answer from an api

mod mock;

use endlessh_reporter::webhook::Webhook;
use mock::{abuse, accept, mock_api, run_worker};
use std::time::Duration;

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
