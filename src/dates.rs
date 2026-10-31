//! days since 1970-01-01 <-> civil date (Howard Hinnant's algorithms)

pub fn date_string(days: i64) -> String {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    format!("{:04}-{m:02}-{d:02}", yoe + era * 400 + i64::from(m <= 2))
}

pub fn parse_date(s: &str) -> Option<i64> {
    let mut parts = s.splitn(3, '-').map(|p| p.parse::<i64>().ok());
    let (y, m, d) = (parts.next()??, parts.next()??, parts.next()??);
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let doy = (153 * ((m + 9) % 12) + 2) / 5 + d - 1;
    Some(era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_roundtrip() {
        assert_eq!(date_string(0), "1970-01-01");
        assert_eq!(date_string(1_760_000_000 / 86_400), "2025-10-09");
        assert_eq!(date_string(parse_date("2024-02-29").unwrap()), "2024-02-29");
        for d in (-1000..30_000).step_by(37) {
            assert_eq!(parse_date(&date_string(d)), Some(d));
        }
        assert_eq!(parse_date("nope"), None);
        assert_eq!(parse_date("2025-13-01"), None);
    }
}
