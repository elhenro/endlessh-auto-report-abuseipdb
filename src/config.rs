//! settings come from the environment; everything the old bash reporter read still works, see legacy.rs

use crate::{
    env::{flag, num, range, secret, var},
    ip::AllowList,
    legacy::{EndlesshArgs, Family},
    sinks::check_url,
};
use std::{path::PathBuf, str::FromStr, time::Duration};

pub struct Config {
    pub port: u16,
    pub family: Option<Family>,
    pub delay: Duration,
    pub max_clients: usize,
    pub max_per_ip: usize,
    pub max_line: usize,
    pub dedupe: Duration,
    pub http_timeout: Duration,
    pub dry_run: bool,
    pub report_all: bool,
    pub categories: String,
    pub allowlist: AllowList,
    pub reported_file: Option<PathBuf>,
    pub events_dir: Option<PathBuf>,
    pub events_retention_days: u32,
    pub events_max_bytes: u64,
    pub public_port: Option<u16>,
    pub abuseipdb_url: String,
    pub api_token: Option<String>,
    pub webhook_url: Option<String>,
    pub webhook_auth: Option<String>,
}

/// endlessh's own upper limit on its delay and client count, which the old reporter passed through
const ENDLESSH_MAX: u32 = i32::MAX as u32;

/// what the bash version sent when `CATEGORIES` was unset: keep it, so old reports and new agree
pub const DEFAULT_CATEGORIES: &str = "18,22";

pub fn check_categories(c: &str) -> Result<(), String> {
    let ok = !c.is_empty()
        && c.split(',')
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
    if ok {
        Ok(())
    } else {
        Err("CATEGORIES must look like 14,22".into())
    }
}

/// a flag in ENDLESSH_ARGS, else the variable, else the default
fn setting<T: FromStr>(name: &str, flag: Option<T>, default: T) -> Result<T, String> {
    flag.map_or_else(|| num(name, default), Ok)
}

/// CURL_TIMEOUT is the old name and took any number, with 0 for no limit. a call blocks all
/// reporting while it runs, so here it stops at the same limit as HTTP_TIMEOUT
fn http_timeout() -> Result<u64, String> {
    if var("HTTP_TIMEOUT").is_some() {
        return range("HTTP_TIMEOUT", num("HTTP_TIMEOUT", 10u64)?, 1, 120);
    }
    Ok(match num("CURL_TIMEOUT", 10u64)? {
        0 => 120,
        n => n.min(120),
    })
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        let dry_run = flag("DRY_RUN")?;
        let categories = var("CATEGORIES").unwrap_or_else(|| DEFAULT_CATEGORIES.into());
        check_categories(&categories)?;
        let abuseipdb_url =
            var("ABUSEIPDB_URL").unwrap_or_else(|| "https://api.abuseipdb.com/api/v2".into());
        check_url("ABUSEIPDB_URL", &abuseipdb_url)?;
        let webhook_url = var("WEBHOOK_URL");
        if let Some(u) = &webhook_url {
            check_url("WEBHOOK_URL", u)?;
        }
        let args = EndlesshArgs::parse(&var("ENDLESSH_ARGS").unwrap_or_default())?;
        // a dry run sends nothing, so a missing key file is no reason to stop
        let api_token = if dry_run {
            None
        } else {
            secret("API_TOKEN", true)?.filter(|t| t != "replace-me-with-abuseipdb-token")
        };
        Ok(Self {
            port: range(
                "TARPIT_PORT",
                setting("TARPIT_PORT", args.port, 2222u16)?,
                1,
                65535,
            )?,
            family: args.family,
            delay: Duration::from_millis(range(
                "TARPIT_DELAY_MS",
                setting("TARPIT_DELAY_MS", args.delay_ms, 10_000u64)?,
                1,
                ENDLESSH_MAX.into(),
            )?),
            max_clients: range(
                "TARPIT_MAX_CLIENTS",
                setting("TARPIT_MAX_CLIENTS", args.max_clients, 4096usize)?,
                1,
                ENDLESSH_MAX as usize,
            )?,
            max_per_ip: range(
                "TARPIT_MAX_PER_IP",
                num("TARPIT_MAX_PER_IP", 32usize)?,
                1,
                1_000_000,
            )?,
            max_line: range(
                "TARPIT_MAX_LINE",
                setting("TARPIT_MAX_LINE", args.max_line, 32usize)?,
                3,
                255,
            )?,
            dedupe: Duration::from_secs(num("DEDUPE_SECONDS", 900u64)?),
            http_timeout: Duration::from_secs(http_timeout()?),
            dry_run,
            report_all: flag("REPORT_ALL_CONNECTIONS")?,
            categories,
            allowlist: AllowList::parse(&var("ALLOWLIST").unwrap_or_default())?,
            reported_file: var("REPORTED_FILE").map(PathBuf::from),
            events_dir: var("EVENTS_DIR").map(PathBuf::from),
            events_retention_days: range(
                "EVENTS_RETENTION_DAYS",
                num("EVENTS_RETENTION_DAYS", 30u32)?,
                1,
                3650,
            )?,
            events_max_bytes: range(
                "EVENTS_MAX_DAILY_MB",
                num("EVENTS_MAX_DAILY_MB", 100u64)?,
                1,
                1_000_000,
            )? * 1024
                * 1024,
            // compose also allows 127.0.0.1:22 here; only used for a warning, so junk is no error
            public_port: var("HOST_PORT").and_then(|v| v.rsplit(':').next()?.parse().ok()),
            abuseipdb_url,
            api_token,
            webhook_auth: if webhook_url.is_some() && !dry_run {
                secret("WEBHOOK_AUTH", false)?
            } else {
                None
            },
            webhook_url,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn categories() {
        for ok in ["14", "14,22", "1,2,3"] {
            assert!(check_categories(ok).is_ok(), "{ok}");
        }
        for bad in ["", ",", "14,", ",14", "14;22", "a", "14, 22"] {
            assert!(check_categories(bad).is_err(), "{bad}");
        }
    }
}
