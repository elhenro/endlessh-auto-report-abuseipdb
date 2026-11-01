#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    V4,
    V6,
}

/// ENDLESSH_ARGS values win over the TARPIT_* settings
#[derive(Debug, Default, PartialEq, Eq)]
pub struct EndlesshArgs {
    pub port: Option<u16>,
    pub delay_ms: Option<u64>,
    pub max_clients: Option<usize>,
    pub max_line: Option<usize>,
    pub family: Option<Family>,
}

fn number<T: std::str::FromStr>(flag: char, v: &str) -> Result<Option<T>, String> {
    v.parse()
        .map(Some)
        .map_err(|_| format!("ENDLESSH_ARGS: -{flag} takes a number, got '{v}'"))
}

impl EndlesshArgs {
    /// clusters like -46, values as -l64 or -l 64, last one wins
    pub fn parse(s: &str) -> Result<Self, String> {
        let mut out = Self::default();
        let mut words = s.split_whitespace();
        while let Some(word) = words.next() {
            let flags = word
                .strip_prefix('-')
                .filter(|f| !f.is_empty())
                .ok_or_else(|| format!("ENDLESSH_ARGS: unexpected '{word}'"))?;
            for (i, flag) in flags.char_indices() {
                match flag {
                    '4' => out.family = Some(Family::V4),
                    '6' => out.family = Some(Family::V6),
                    'v' | 's' => {} // logging is always on and always goes to stdout
                    'd' | 'f' | 'l' | 'm' | 'p' => {
                        let rest = &flags[i + 1..];
                        let value = Some(rest)
                            .filter(|r| !r.is_empty())
                            .or_else(|| words.next())
                            .ok_or_else(|| format!("ENDLESSH_ARGS: -{flag} needs a value"))?;
                        match flag {
                            'd' => out.delay_ms = number(flag, value)?,
                            'l' => out.max_line = number(flag, value)?,
                            'm' => out.max_clients = number(flag, value)?,
                            'p' => out.port = number(flag, value)?,
                            _ => log!(
                                "warning: ENDLESSH_ARGS: -f is not supported, {value} is not read: use the TARPIT_* settings"
                            ),
                        }
                        break;
                    }
                    other => return Err(format!("ENDLESSH_ARGS: unknown flag -{other}")),
                }
            }
        }
        Ok(out)
    }
}

/// entries are text: 010.0.0.1 cannot match a visitor shown as 10.0.0.1, so it is skipped like any miss
pub(crate) fn was_valid_allowlist_entry(entry: &str) -> bool {
    let e = entry.to_ascii_lowercase();
    let octets: Vec<_> = e.split('.').collect();
    let dotted = octets.len() == 4
        && octets.iter().all(|o| {
            (1..=3).contains(&o.len())
                && o.bytes().all(|b| b.is_ascii_digit())
                && o.parse::<u16>().is_ok_and(|n| n <= 255)
        });
    let colons = (2..=45).contains(&e.len())
        && e.bytes()
            .all(|b| b.is_ascii_hexdigit() || b == b':' || b == b'.')
        && e.matches(':').count() >= 2;
    dotted || colons
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn takes_the_flags() {
        let a = EndlesshArgs::parse("-4 -v -vv -d 500 -l64 -m 9 -p 2200 -s").unwrap();
        assert_eq!(a.family, Some(Family::V4));
        assert_eq!(a.delay_ms, Some(500));
        assert_eq!(a.max_line, Some(64));
        assert_eq!(a.max_clients, Some(9));
        assert_eq!(a.port, Some(2200));
        assert_eq!(EndlesshArgs::parse("").unwrap(), EndlesshArgs::default());
        assert_eq!(
            EndlesshArgs::parse("-46").unwrap().family,
            Some(Family::V6),
            "last one wins"
        );
        assert_eq!(EndlesshArgs::parse("-vl 7").unwrap().max_line, Some(7));
        assert_eq!(
            EndlesshArgs::parse("-f /etc/x -d 1").unwrap().delay_ms,
            Some(1),
            "-f is skipped with its value"
        );
    }

    #[test]
    fn refuses_bad_flags() {
        for bad in ["-x", "foo", "-", "-d", "-p x", "-l 3 7", "-h", "-V"] {
            assert!(EndlesshArgs::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn old_allowlist_shapes() {
        for old in ["010.0.0.1", "1:2:3", ":::", "ABC::"] {
            assert!(was_valid_allowlist_entry(old), "{old}");
        }
        for no in [
            "nope",
            "1.2.3",
            "1.2.3.256",
            "1000.1.1.1",
            "a:b",
            "1.2.3.4/x",
            "fe80::1%eth0",
        ] {
            assert!(!was_valid_allowlist_entry(no), "{no}");
        }
    }
}
