use endlessh_reporter::{
    abuseipdb::AbuseIpdb, config::Config, events::EventLog, harden, listen, log,
    reported::ReportedFile, sinks::Sink, tarpit, tarpit::Params, webhook::Webhook, worker::Worker,
};
use std::{env, io::Write, process::exit, sync::mpsc, thread};

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

fn main() {
    harden::no_inspection();
    harden::exit_on_signals();
    let cfg = Config::from_env().unwrap_or_else(|e| fail(78, &e));
    if env::args().nth(1).as_deref() == Some("--healthcheck") {
        exit(i32::from(!listen::is_listening(cfg.port)));
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
