use endlessh_reporter::{
    abuseipdb::AbuseIpdb, config::Config, events::EventLog, harden, listen, log,
    reported::ReportedFile, sinks::Sink, tarpit, tarpit::Params, webhook::Webhook, worker::Worker,
};
use std::{env, io::Write, process::exit, sync::mpsc, thread};

const USAGE: &str = "\
endlessh-reporter - ssh tarpit that reports visitors to AbuseIPDB

configured by environment variables, not by flags: see the readme
(API_TOKEN, WEBHOOK_URL, TARPIT_PORT, DEDUPE_SECONDS, ALLOWLIST, DRY_RUN, ...)

usage: endlessh-reporter [COMMAND]

  (none)         run the tarpit
  status         print the settings this shell resolves, without binding a port
  --version, -V  print the version
  --help, -h     this text
  --healthcheck  exit 0 if the configured port is listening, 1 if not

exit codes: 0 ok, 1 cannot listen or unhealthy, 78 invalid configuration
";

// config errors exit 78 (EX_CONFIG) so the systemd unit does not restart-loop on them
fn fail(code: i32, msg: &str) -> ! {
    // not eprintln!: a panic on a closed stderr would turn exit 78 into a restart loop
    let _ = writeln!(std::io::stderr(), "tarpit: error: {msg}");
    exit(code)
}

fn build_sinks(cfg: &Config) -> Result<Vec<Box<dyn Sink>>, String> {
    let mut sinks: Vec<Box<dyn Sink>> = Vec::new();
    if cfg.dry_run {
        log!("DRY_RUN=1: connections are logged, nothing is sent anywhere");
        return Ok(sinks);
    }
    if let Some(token) = &cfg.api_token {
        let s = AbuseIpdb::new(
            &cfg.abuseipdb_url,
            token.clone(),
            cfg.categories.clone(),
            cfg.http_timeout,
        );
        s.check_token()?;
        sinks.push(Box::new(s));
    }
    if let Some(url) = &cfg.webhook_url {
        sinks.push(Box::new(Webhook::new(
            url.clone(),
            cfg.webhook_auth.clone(),
            cfg.http_timeout,
        )));
    }
    if sinks.is_empty() {
        log!("no sink configured (set API_TOKEN and/or WEBHOOK_URL): running as a plain tarpit");
    }
    Ok(sinks)
}

fn version() -> String {
    format!("endlessh-reporter {}", env!("CARGO_PKG_VERSION"))
}

/// what a start from this shell would do, minus the network and minus binding the port
fn print_status(cfg: &Config) {
    let millis = |d: std::time::Duration| d.as_millis();
    let family = match cfg.family {
        None => "ipv4+ipv6",
        Some(endlessh_reporter::legacy::Family::V4) => "ipv4",
        Some(endlessh_reporter::legacy::Family::V6) => "ipv6",
    };
    let list = |v: &Option<String>| if v.is_some() { "set" } else { "not set" };
    let path = |p: &Option<std::path::PathBuf>| {
        p.as_ref()
            .map_or_else(|| "none".to_string(), |p| p.display().to_string())
    };
    let sinks = if cfg.dry_run {
        "dry run".to_string()
    } else {
        let mut names = Vec::new();
        if cfg.api_token.is_some() {
            names.push("abuseipdb");
        }
        if cfg.webhook_url.is_some() {
            names.push("webhook");
        }
        if names.is_empty() {
            names.push("none (tarpit only)");
        }
        names.join(",")
    };
    log!("{}", version());
    log!("port: {} ({family})", cfg.port);
    log!(
        "limits: delay {} ms, {} clients, {} per ip, line {} bytes",
        millis(cfg.delay),
        cfg.max_clients,
        cfg.max_per_ip,
        cfg.max_line
    );
    log!(
        "dedupe: {} s, http timeout: {} s, report all: {}",
        cfg.dedupe.as_secs(),
        cfg.http_timeout.as_secs(),
        cfg.report_all
    );
    log!("sinks: {sinks}");
    log!("categories: {}", cfg.categories);
    let n = cfg.allowlist.len();
    log!(
        "allowlist: {n} {}",
        if n == 1 { "entry" } else { "entries" }
    );
    log!("api token: {}", list(&cfg.api_token));
    log!("webhook url: {}", list(&cfg.webhook_url));
    log!("reported file: {}", path(&cfg.reported_file));
    log!("events dir: {}", path(&cfg.events_dir));
    if cfg!(target_os = "linux") {
        log!(
            "listening on {}: {}",
            cfg.port,
            if listen::is_listening(cfg.port) {
                "yes"
            } else {
                "no"
            }
        );
    }
}

fn main() {
    harden::no_inspection();
    harden::exit_on_signals();
    let arg = env::args().nth(1);
    match arg.as_deref() {
        Some("--version" | "-V" | "version") => {
            println!("{}", version());
            return;
        }
        Some("--help" | "-h" | "help") => {
            print!("{USAGE}");
            return;
        }
        _ => {}
    }
    let cfg = Config::from_env().unwrap_or_else(|e| fail(78, &e));
    if arg.as_deref() == Some("--healthcheck") {
        exit(i32::from(!listen::is_listening(cfg.port)));
    }
    if arg.as_deref() == Some("status") {
        print_status(&cfg);
        return;
    }
    if let Some(other) = arg.as_deref() {
        log!("warning: ignoring unknown argument '{other}', see --help");
    }
    let sinks = build_sinks(&cfg).unwrap_or_else(|e| fail(78, &e));
    let names: Vec<_> = sinks.iter().map(|s| s.name()).collect();
    let events = cfg.events_dir.clone().and_then(|dir| {
        match EventLog::open(dir.clone(), cfg.events_retention_days, cfg.events_max_bytes) {
            Ok(l) => {
                log!(
                    "writing events to {} (keeping {} days)",
                    dir.display(),
                    cfg.events_retention_days
                );
                Some(l)
            }
            Err(e) => {
                log!("warning: {e}, events disabled");
                None
            }
        }
    });

    let listener = listen::listen(cfg.port, cfg.family)
        .unwrap_or_else(|e| fail(1, &format!("cannot listen on port {}: {e}", cfg.port)));
    // full address, so a v4-only fallback shows as 0.0.0.0 instead of [::]
    let addr = listener.local_addr().ok();
    let bound = addr.map_or_else(|| cfg.port.to_string(), |a| a.to_string());
    let bound_port = addr.map(|a| a.port());
    if !names.is_empty() && cfg.public_port.or(bound_port).is_some_and(|p| p != 22) {
        log!(
            "warning: reporting visitors on a port other than 22. browsers refuse to connect to 22 \
             but not here, and a web page using dns rebinding can make them open with SSH- and get \
             their users reported. set DRY_RUN=1 until the tarpit is on 22"
        );
    }
    if env::var_os("TARPIT_PORT").is_some() && bound_port.is_some_and(|p| p != cfg.port) {
        log!(
            "warning: TARPIT_PORT={} is not in effect, the socket passed in is on {bound}: re-run the installer to move it",
            cfg.port
        );
    }
    if cfg.report_all && !names.is_empty() {
        log!(
            "warning: REPORT_ALL_CONNECTIONS=1: anything that makes a machine connect here, such as a web page \
             or a url fetcher, gets whoever runs it reported"
        );
    }
    log!(
        "starting tarpit on {bound} (sinks: [{}], dedupe={}s)",
        names.join(","),
        cfg.dedupe.as_secs()
    );

    let (tx, rx) = mpsc::sync_channel(4096);
    let worker = Worker::new(cfg.allowlist, cfg.dedupe, sinks, events)
        .report_all(cfg.report_all)
        .reported_to(cfg.reported_file.map(ReportedFile::new));
    thread::Builder::new()
        .name("worker".into())
        .spawn(move || worker.run(rx))
        .expect("spawn worker thread");
    let params = Params {
        delay: cfg.delay,
        max_clients: cfg.max_clients,
        max_per_ip: cfg.max_per_ip,
        max_line: cfg.max_line,
    };
    let _ = tarpit::spawn(listener, params, tx).join();
    fail(1, "tarpit stopped unexpectedly");
}
