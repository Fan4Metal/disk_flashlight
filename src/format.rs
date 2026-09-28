//! Human-readable formatting helpers.

const UNITS: [&str; 6] = ["B", "KB", "MB", "GB", "TB", "PB"];

/// `4.6 GB`, `130.3 MB`, `512 B` (1024-based, one decimal above bytes).
pub fn human_size(bytes: u64) -> String {
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut v = bytes as f64;
    let mut unit = 0;
    while v >= 1024.0 && unit < UNITS.len() - 1 {
        v /= 1024.0;
        unit += 1;
    }
    if v >= 100.0 {
        format!("{v:.0} {}", UNITS[unit])
    } else {
        format!("{v:.1} {}", UNITS[unit])
    }
}

/// `1 234 567` with thin-space thousands separators.
pub fn thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push('\u{2009}');
        }
        out.push(ch);
    }
    out
}

/// `part` as a share of `whole`: `42%` from 10% up, `3.5%` below, `<0.1%`
/// for a sliver, `0%` for nothing.
pub fn percent(part: u64, whole: u64) -> String {
    if part == 0 || whole == 0 {
        return "0%".into();
    }
    let p = part as f64 / whole as f64 * 100.0;
    if p >= 9.95 {
        format!("{p:.0}%")
    } else if p >= 0.05 {
        format!("{p:.1}%")
    } else {
        "<0.1%".into()
    }
}

/// Unix seconds `t` as a local date, `2024-03-15`; `—` for 0 (unknown).
pub fn date(t: u32) -> String {
    if t == 0 {
        return "\u{2014}".into();
    }
    static OFFSET: std::sync::OnceLock<i64> = std::sync::OnceLock::new();
    let offset = *OFFSET.get_or_init(crate::scan::win::utc_offset);
    let (y, m, d) = civil_date((t as i64 + offset).div_euclid(86_400));
    format!("{y:04}-{m:02}-{d:02}")
}

/// How long ago something happened, `secs` seconds before now: `today`,
/// `yesterday`, `12 days ago`, `5 months ago`, `3 years ago`.
pub fn ago(secs: u32) -> String {
    let days = secs / 86_400;
    let months = (days as f64 / 30.44) as u32;
    match days {
        0 => "today".into(),
        1 => "yesterday".into(),
        2..61 => format!("{days} days ago"),
        _ if months < 24 => format!("{months} months ago"),
        _ => format!("{} years ago", (days as f64 / 365.25) as u32),
    }
}

/// Year, month and day of the day `days` after 1970-01-01, in the
/// proleptic Gregorian calendar (Howard Hinnant's `civil_from_days`).
fn civil_date(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percents() {
        assert_eq!(percent(0, 10), "0%");
        assert_eq!(percent(5, 0), "0%");
        assert_eq!(percent(1, 2), "50%");
        assert_eq!(percent(1, 3), "33%");
        assert_eq!(percent(35, 1000), "3.5%");
        assert_eq!(percent(1, 10_000), "<0.1%");
        assert_eq!(percent(999, 10_000), "10%");
    }

    #[test]
    fn dates() {
        assert_eq!(civil_date(0), (1970, 1, 1));
        assert_eq!(civil_date(19_723), (2024, 1, 1));
        assert_eq!(civil_date(19_782), (2024, 2, 29));
        assert_eq!(civil_date(-1), (1969, 12, 31));
        // The last day u32 Unix seconds reach.
        assert_eq!(civil_date(u32::MAX as i64 / 86_400), (2106, 2, 7));
        assert_eq!(date(0), "\u{2014}");
    }

    #[test]
    fn ages() {
        const DAY: u32 = 86_400;
        assert_eq!(ago(0), "today");
        assert_eq!(ago(DAY + 5), "yesterday");
        assert_eq!(ago(60 * DAY), "60 days ago");
        assert_eq!(ago(61 * DAY), "2 months ago");
        assert_eq!(ago(700 * DAY), "22 months ago");
        assert_eq!(ago(731 * DAY), "2 years ago");
        assert_eq!(ago(3653 * DAY), "10 years ago");
    }

    #[test]
    fn sizes() {
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(1023), "1023 B");
        assert_eq!(human_size(1024), "1.0 KB");
        assert_eq!(human_size(4_939_212_390), "4.6 GB");
        assert_eq!(human_size(136_628_000), "130 MB");
    }

    #[test]
    fn groups() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1000), "1\u{2009}000");
        assert_eq!(thousands(1234567), "1\u{2009}234\u{2009}567");
    }
}
