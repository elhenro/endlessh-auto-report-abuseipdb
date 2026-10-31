//! settings come from the environment; names match the old bash reporter where they still apply

use crate::{ip::AllowList, sinks::check_url};
use std::{
    env, fs,
    io::ErrorKind,
    path::{Path, PathBuf},
    str::FromStr,
    time::Duration,
};

pub struct Config {
    pub port: u16,
    pub delay: Duration,
    pub max_clients: usize,
    pub max_per_ip: usize,
    pub max_line: usize,
    pub dedupe: Duration,
    pub http_timeout: Duration,
    pub dry_run: bool,
    pub categories: String,
    pub allowlist: AllowList,
    pub events_dir: Option<PathBuf>,
    pub events_retention_days: u32,
    pub events_max_bytes: u64,
    pub public_port: Option<u16>,
    pub abuseipdb_url: String,
    pub api_token: Option<String>,
    pub webhook_url: Option<String>,
    pub webhook_auth: Option<String>,
}

fn var(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn num<T: FromStr>(name: &str, default: T) -> Result<T, String> {
    match var(name) {
        None => Ok(default),
        Some(v) => v
            .parse()
            .map_err(|_| format!("{name} must be a number, got '{v}'")),
    }
}

fn range<T: PartialOrd + std::fmt::Display>(name: &str, v: T, min: T, max: T) -> Result<T, String> {
    if v < min || v > max {
        Err(format!("{name} must be {min}..{max}, got {v}"))
    } else {
        Ok(v)
    }
}

/// api keys lose all whitespace (stray newlines); header values like "Bearer abc" only lose the ends
fn clean(s: &str, strip_inner: bool) -> Option<String> {
    let s = if strip_inner {
        s.split_whitespace().collect()
    } else {
        s.trim().to_string()
    };
    Some(s).filter(|s: &String| !s.is_empty())
}

/// NAME from the env, else the file in NAME_FILE, else systemd's $CREDENTIALS_DIRECTORY/<name>
fn secret(name: &str, strip_inner: bool) -> Result<Option<String>, String> {
    if let Some(v) = var(name) {
        return Ok(clean(&v, strip_inner));
    }
    let explicit = var(&format!("{name}_FILE")).map(PathBuf::from);
    let path = match (&explicit, var("CREDENTIALS_DIRECTORY")) {
        (Some(p), _) => p.clone(),
        (None, Some(dir)) => Path::new(&dir).join(name.to_lowercase()),
        (None, None) => return Ok(None),
    };
    match fs::read_to_string(&path) {
        Ok(s) => Ok(clean(&s, strip_inner)),
        // a credentials dir without this credential just means the sink is off
        Err(e) if explicit.is_none() && e.kind() == ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("cannot read {}: {e}", path.display())),
    }
}

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

impl Config {
    pub fn from_env() -> Result<Self, String> {
        let dry_run = match var("DRY_RUN").as_deref() {
            None | Some("0") => false,
            Some("1") => true,
            Some(_) => return Err("DRY_RUN must be 0 or 1".into()),
        };
        let categories = var("CATEGORIES").unwrap_or_else(|| "14,22".into());
        check_categories(&categories)?;
        let abuseipdb_url =
            var("ABUSEIPDB_URL").unwrap_or_else(|| "https://api.abuseipdb.com/api/v2".into());
        check_url("ABUSEIPDB_URL", &abuseipdb_url)?;
        let webhook_url = var("WEBHOOK_URL");
        if let Some(u) = &webhook_url {
            check_url("WEBHOOK_URL", u)?;
        }
        let api_token =
            secret("API_TOKEN", true)?.filter(|t| t != "replace-me-with-abuseipdb-token");
        Ok(Self {
            port: range("TARPIT_PORT", num("TARPIT_PORT", 2222u16)?, 1, 65535)?,
            delay: Duration::from_millis(range(
                "TARPIT_DELAY_MS",
                num("TARPIT_DELAY_MS", 10_000u64)?,
                1,
                3_600_000,
            )?),
            max_clients: range(
                "TARPIT_MAX_CLIENTS",
                num("TARPIT_MAX_CLIENTS", 4096usize)?,
                1,
                1_000_000,
            )?,
            max_per_ip: range(
                "TARPIT_MAX_PER_IP",
                num("TARPIT_MAX_PER_IP", 32usize)?,
                1,
                1_000_000,
            )?,
            max_line: range("TARPIT_MAX_LINE", num("TARPIT_MAX_LINE", 32usize)?, 3, 255)?,
            dedupe: Duration::from_secs(num("DEDUPE_SECONDS", 900u64)?),
            http_timeout: Duration::from_secs(range(
                "HTTP_TIMEOUT",
                num("HTTP_TIMEOUT", 10u64)?,
                1,
                120,
            )?),
            dry_run,
            categories,
            allowlist: AllowList::parse(&var("ALLOWLIST").unwrap_or_default())?,
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
            webhook_auth: if webhook_url.is_some() {
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

    #[test]
    fn range_check_and_clean() {
        assert!(range("X", 5, 1, 10).is_ok());
        assert!(range("X", 0, 1, 10).is_err());
        assert_eq!(clean(" ab\n c ", true).as_deref(), Some("abc"));
        assert_eq!(
            clean(" Bearer  abc\n", false).as_deref(),
            Some("Bearer  abc")
        );
        assert_eq!(clean("  \n", true), None);
    }
}
