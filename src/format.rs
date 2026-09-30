//! Human-readable formatting helpers, in the interface language
//! ([`crate::i18n`]); the `_in` variants take the language explicitly.

use crate::i18n::{Lang, count_in, lang};

const UNITS: [&str; 6] = ["B", "KB", "MB", "GB", "TB", "PB"];
/// As in the Russian Explorer.
const UNITS_RU: [&str; 6] = ["байт", "КБ", "МБ", "ГБ", "ТБ", "ПБ"];

/// `4.6 GB`, `130 MB`, `512 B` (1024-based, one decimal below 100); in
/// Russian `4,6 ГБ`.
pub fn human_size(bytes: u64) -> String {
    human_size_in(lang(), bytes)
}

pub fn human_size_in(lang: Lang, bytes: u64) -> String {
    let units = match lang {
        Lang::En => UNITS,
        Lang::Ru => UNITS_RU,
    };
    if bytes < 1024 {
        return format!("{bytes} {}", units[0]);
    }
    let mut v = bytes as f64;
    let mut unit = 0;
    while v >= 1024.0 && unit < UNITS.len() - 1 {
        v /= 1024.0;
        unit += 1;
    }
    let number = if v >= 100.0 { format!("{v:.0}") } else { format!("{v:.1}") };
    format!("{} {}", decimal(lang, number), units[unit])
}

/// `number` with the decimal separator of `lang` (a comma in Russian).
fn decimal(lang: Lang, number: String) -> String {
    match lang {
        Lang::En => number,
        Lang::Ru => number.replace('.', ","),
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
    percent_in(lang(), part, whole)
}

pub fn percent_in(lang: Lang, part: u64, whole: u64) -> String {
    if part == 0 || whole == 0 {
        return "0%".into();
    }
    let p = part as f64 / whole as f64 * 100.0;
    if p >= 9.95 {
        format!("{p:.0}%")
    } else if p >= 0.05 {
        format!("{}%", decimal(lang, format!("{p:.1}")))
    } else {
        format!("<{}%", decimal(lang, "0.1".into()))
    }
}

/// Unix seconds `t` as a local date, `2024-03-15` (`15.03.2024` in
/// Russian); `—` for 0 (unknown).
pub fn date(t: u32) -> String {
    if t == 0 {
        return "\u{2014}".into();
    }
    date_in(lang(), (t as i64 + local_offset()).div_euclid(86_400))
}

/// Seconds to add to UTC for local time, asked once per run.
pub fn local_offset() -> i64 {
    static OFFSET: std::sync::OnceLock<i64> = std::sync::OnceLock::new();
    *OFFSET.get_or_init(crate::scan::win::utc_offset)
}

/// Unix seconds `t` shifted by `offset` as `2024-03-15 14:02:11`, the form
/// spreadsheets read as a date and time in any language; empty for 0
/// (unknown).
pub fn date_time_at(t: u32, offset: i64) -> String {
    if t == 0 {
        return String::new();
    }
    let local = t as i64 + offset;
    let (y, m, d) = civil_date(local.div_euclid(86_400));
    let secs = local.rem_euclid(86_400);
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}", secs / 3600, secs / 60 % 60, secs % 60)
}

/// The local day `day` (days since 1970-01-01) as a date in `lang`.
fn date_in(lang: Lang, day: i64) -> String {
    let (y, m, d) = civil_date(day);
    match lang {
        Lang::En => format!("{y:04}-{m:02}-{d:02}"),
        Lang::Ru => format!("{d:02}.{m:02}.{y:04}"),
    }
}

/// How long ago something happened, `secs` seconds before now: `today`,
/// `yesterday`, `12 days ago`, `5 months ago`, `3 years ago`.
pub fn ago(secs: u32) -> String {
    ago_in(lang(), secs)
}

pub fn ago_in(lang: Lang, secs: u32) -> String {
    let days = secs / 86_400;
    let months = (days as f64 / 30.44) as u64;
    let years = (days as f64 / 365.25) as u64;
    let (n, en, ru) = match days {
        0 => return tr_in(lang, "today", "сегодня"),
        1 => return tr_in(lang, "yesterday", "вчера"),
        2..61 => (days as u64, ["day", "days"], ["день", "дня", "дней"]),
        _ if months < 24 => (months, ["month", "months"], ["месяц", "месяца", "месяцев"]),
        _ => (years, ["year", "years"], ["год", "года", "лет"]),
    };
    let amount = count_in(lang, n, en, ru);
    tr_in(lang, &format!("{amount} ago"), &format!("{amount} назад"))
}

fn tr_in(lang: Lang, en: &str, ru: &str) -> String {
    match lang {
        Lang::En => en.into(),
        Lang::Ru => ru.into(),
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
    fn dates_with_time() {
        assert_eq!(date_time_at(0, 0), "");
        // 2024-03-15 14:02:11 UTC.
        assert_eq!(date_time_at(1_710_511_331, 0), "2024-03-15 14:02:11");
        assert_eq!(date_time_at(1_710_511_331, 3 * 3600), "2024-03-15 17:02:11");
        assert_eq!(date_time_at(1_710_511_331, -15 * 3600), "2024-03-14 23:02:11");
    }

    #[test]
    fn percents() {
        assert_eq!(percent_in(Lang::En, 0, 10), "0%");
        assert_eq!(percent_in(Lang::En, 5, 0), "0%");
        assert_eq!(percent_in(Lang::En, 1, 2), "50%");
        assert_eq!(percent_in(Lang::En, 1, 3), "33%");
        assert_eq!(percent_in(Lang::En, 35, 1000), "3.5%");
        assert_eq!(percent_in(Lang::En, 1, 10_000), "<0.1%");
        assert_eq!(percent_in(Lang::En, 999, 10_000), "10%");
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
        assert_eq!(ago_in(Lang::En, 0), "today");
        assert_eq!(ago_in(Lang::En, DAY + 5), "yesterday");
        assert_eq!(ago_in(Lang::En, 60 * DAY), "60 days ago");
        assert_eq!(ago_in(Lang::En, 61 * DAY), "2 months ago");
        assert_eq!(ago_in(Lang::En, 700 * DAY), "22 months ago");
        assert_eq!(ago_in(Lang::En, 731 * DAY), "2 years ago");
        assert_eq!(ago_in(Lang::En, 3653 * DAY), "10 years ago");
    }

    #[test]
    fn russian_forms() {
        assert_eq!(human_size_in(Lang::Ru, 512), "512 байт");
        assert_eq!(human_size_in(Lang::Ru, 4_939_212_390), "4,6 ГБ");
        assert_eq!(human_size_in(Lang::Ru, 136_628_000), "130 МБ");
        assert_eq!(percent_in(Lang::Ru, 35, 1000), "3,5%");
        assert_eq!(percent_in(Lang::Ru, 1, 10_000), "<0,1%");
        assert_eq!(date_in(Lang::Ru, 19_782), "29.02.2024");
        assert_eq!(date_in(Lang::En, 19_782), "2024-02-29");
        const DAY: u32 = 86_400;
        assert_eq!(ago_in(Lang::Ru, 0), "сегодня");
        assert_eq!(ago_in(Lang::Ru, 3 * DAY), "3 дня назад");
        assert_eq!(ago_in(Lang::Ru, 21 * DAY), "21 день назад");
        assert_eq!(ago_in(Lang::Ru, 150 * DAY), "4 месяца назад");
        assert_eq!(ago_in(Lang::Ru, 3653 * DAY), "10 лет назад");
    }

    #[test]
    fn sizes() {
        assert_eq!(human_size_in(Lang::En, 0), "0 B");
        assert_eq!(human_size_in(Lang::En, 1023), "1023 B");
        assert_eq!(human_size_in(Lang::En, 1024), "1.0 KB");
        assert_eq!(human_size_in(Lang::En, 4_939_212_390), "4.6 GB");
        assert_eq!(human_size_in(Lang::En, 136_628_000), "130 MB");
    }

    #[test]
    fn groups() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1000), "1\u{2009}000");
        assert_eq!(thousands(1234567), "1\u{2009}234\u{2009}567");
    }
}
