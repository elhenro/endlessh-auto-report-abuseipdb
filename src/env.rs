//! reading settings and secrets from the environment

use std::{
    env, fs,
    io::ErrorKind,
    path::{Path, PathBuf},
    str::FromStr,
};

pub(crate) fn var(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

pub(crate) fn num<T: FromStr>(name: &str, default: T) -> Result<T, String> {
    match var(name) {
        None => Ok(default),
        Some(v) => v
            .parse()
            .map_err(|_| format!("{name} must be a number, got '{v}'")),
    }
}

pub(crate) fn range<T: PartialOrd + std::fmt::Display>(
    name: &str,
    v: T,
    min: T,
    max: T,
) -> Result<T, String> {
    if v < min || v > max {
        Err(format!("{name} must be {min}..{max}, got {v}"))
    } else {
        Ok(v)
    }
}

pub(crate) fn flag(name: &str) -> Result<bool, String> {
    match var(name).as_deref() {
        None | Some("0") => Ok(false),
        Some("1") => Ok(true),
        Some(_) => Err(format!("{name} must be 0 or 1")),
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
pub(crate) fn secret(name: &str, strip_inner: bool) -> Result<Option<String>, String> {
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

#[cfg(test)]
mod tests {
    use super::*;

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
