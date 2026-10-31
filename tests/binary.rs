//! operational behaviour of the real binary: no internet, no privileged ports

// each test file uses part of these
#[allow(dead_code)]
mod common;
#[allow(dead_code)]
mod mock;

use common::{command, free_port, serial, visit, Running, BIN};
use mock::mock_api;
use std::{thread, time::Duration};

#[test]
fn survives_a_closed_stdout() {
    let _serial = serial();
    let port = free_port();
    let mut t = Running::start(command(BIN).env("TARPIT_PORT", port.to_string()));
    t.close_stdout();
    // every visit logs a line, and the first one written to the closed pipe used to abort the process
    for _ in 0..3 {
        visit(port);
        thread::sleep(Duration::from_millis(50));
    }
    assert!(t.child.try_wait().unwrap().is_none(), "died");
}

#[test]
fn warns_when_reporting_on_a_port_browsers_can_reach() {
    let _serial = serial();
    let warned = |dry: &str, host_port: Option<&str>| {
        let port = free_port();
        let mut c = command(BIN);
        c.env("TARPIT_PORT", port.to_string())
            .env("DRY_RUN", dry)
            .env("WEBHOOK_URL", "http://127.0.0.1:1/");
        if let Some(p) = host_port {
            c.env("HOST_PORT", p);
        }
        Running::start(&mut c)
            .startup
            .contains("warning: reporting")
    };
    assert!(warned("0", None), "bound port is not 22");
    assert!(warned("0", Some("2222")));
    assert!(!warned("0", Some("22")), "published on 22");
    assert!(!warned("0", Some("127.0.0.1:22")));
    assert!(!warned("1", None), "dry run reports nothing");
}

#[test]
fn a_token_check_that_gets_redirected_is_not_ok() {
    let _serial = serial();
    let (base, _) = mock_api("302 Found", "Location: http://127.0.0.1:1/\r\n", "{}");
    let port = free_port();
    let t = Running::start(
        command(BIN)
            .env("TARPIT_PORT", port.to_string())
            .env("DRY_RUN", "0")
            .env("API_TOKEN", "tok")
            .env("ABUSEIPDB_URL", base),
    );
    assert!(t.startup.contains("returned HTTP 302"), "{}", t.startup);
    assert!(!t.startup.contains("token ok"), "{}", t.startup);
}

#[cfg(target_os = "linux")]
#[test]
fn healthcheck_tells_listening_from_not() {
    let _serial = serial();
    let port = free_port();
    let _t = Running::start(command(BIN).env("TARPIT_PORT", port.to_string()));
    let check = |p: u16| {
        command(BIN)
            .env("TARPIT_PORT", p.to_string())
            .arg("--healthcheck")
            .status()
            .unwrap()
            .code()
    };
    assert_eq!(check(port), Some(0));
    assert_eq!(check(free_port()), Some(1));
}
