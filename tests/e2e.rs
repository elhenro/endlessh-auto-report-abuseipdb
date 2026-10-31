//! end to end against local mocks: no internet, no privileged ports

mod mock;

use endlessh_reporter::{tarpit::Event, webhook::Webhook};
use mock::{abuse, accept, mock_api, run_worker};
use std::time::Duration;

#[test]
fn reports_once_filters_and_logs_events() {
    let (base, seen) = mock_api("200 OK", "X-RateLimit-Remaining: 42\r\n", "{}");
    let t = 1_760_000_000;
    let lines = run_worker(
        vec![abuse(&base)],
        "9.9.9.9",
        vec![
            accept(t, "8.8.8.8"),
            accept(t + 1, "8.8.8.8"),  // dedup
            accept(t + 1, "10.0.0.1"), // reserved
            accept(t + 1, "9.9.9.9"),  // allowlisted
            Event::Close {
                t: t + 2,
                ip: "8.8.8.8".parse().unwrap(),
                secs: 3.25,
                bytes: 120,
            },
            Event::Close {
                t: t + 2,
                ip: "10.0.0.1".parse().unwrap(),
                secs: 1.0,
                bytes: 1,
            },
        ],
        "report",
    );
    assert_eq!(
        lines,
        [
            r#"{"t":1760000000,"e":"accept","ip":"8.8.8.8","a":"reported","q":42}"#,
            r#"{"t":1760000001,"e":"accept","ip":"8.8.8.8","a":"dedup"}"#,
            r#"{"t":1760000002,"e":"close","ip":"8.8.8.8","s":3.250,"b":120}"#,
        ]
    );
    let reqs = seen.lock().unwrap();
    assert_eq!(reqs.len(), 1, "only the fresh public ip is reported");
    let r = reqs[0].to_lowercase();
    assert!(r.starts_with("post /report "), "{r}");
    assert!(r.contains("\r\nkey: tok\r\n"), "{r}");
    assert!(
        r.contains("ip=8.8.8.8") && r.contains("categories=14%2c22"),
        "{r}"
    );
    assert!(
        r.contains("no+login+attempted") || r.contains("no%20login%20attempted"),
        "{r}"
    );
}

#[test]
fn webhook_gets_json_and_auth_header() {
    let (base, seen) = mock_api("200 OK", "", "{}");
    let hook = Box::new(Webhook::new(
        format!("{base}/hook"),
        Some("Bearer s3cret".into()),
        Duration::from_secs(5),
    ));
    let lines = run_worker(
        vec![hook],
        "",
        vec![accept(1_760_000_000, "2a00:1450::1")],
        "hook",
    );
    assert_eq!(
        lines,
        [r#"{"t":1760000000,"e":"accept","ip":"2a00:1450::1","a":"delivered"}"#]
    );
    let r = seen.lock().unwrap()[0].clone();
    assert!(r.starts_with("POST /hook "), "{r}");
    assert!(
        r.to_lowercase().contains("authorization: bearer s3cret"),
        "{r}"
    );
    assert!(
        r.ends_with(r#"{"event":"accept","t":1760000000,"ip":"2a00:1450::1","port":40000}"#),
        "{r}"
    );
}

#[test]
fn no_sinks_means_dry() {
    let lines = run_worker(vec![], "", vec![accept(1_760_000_000, "8.8.8.8")], "dry");
    assert_eq!(
        lines,
        [r#"{"t":1760000000,"e":"accept","ip":"8.8.8.8","a":"dry"}"#]
    );
}
