//! Date handling for definitions: Go-style layouts (`dateparse`), relative
//! times (`timeago`, "3 hours ago"), and a lenient parser for the final
//! `date` field.

use chrono::{DateTime, Datelike, NaiveDate, NaiveDateTime, NaiveTime, TimeDelta, TimeZone, Utc};
use std::sync::LazyLock;

use regex::Regex;

/// Converts a Go reference-time layout (`2006-01-02 15:04:05`) to a
/// chrono format string.
pub fn go_layout_to_chrono(layout: &str) -> String {
    // Longest tokens first so `2006` wins over `2`, `January` over `Jan`.
    const TOKENS: &[(&str, &str)] = &[
        ("January", "%B"),
        ("Monday", "%A"),
        ("Z07:00", "%:z"),
        ("-07:00", "%:z"),
        ("-0700", "%z"),
        ("2006", "%Y"),
        (".000000", "%.6f"),
        (".000", "%.3f"),
        (".999999", "%.f"),
        (".999", "%.f"),
        ("Jan", "%b"),
        ("Mon", "%a"),
        ("MST", "%Z"),
        ("_2", "%e"),
        ("01", "%m"),
        ("02", "%d"),
        ("03", "%I"),
        ("04", "%M"),
        ("05", "%S"),
        ("06", "%y"),
        ("15", "%H"),
        ("PM", "%p"),
        ("pm", "%P"),
        ("1", "%m"),
        ("2", "%d"),
        ("3", "%I"),
        ("4", "%M"),
        ("5", "%S"),
    ];
    let mut out = String::new();
    let mut rest = layout;
    'outer: while !rest.is_empty() {
        for (go, ch) in TOKENS {
            if let Some(r) = rest.strip_prefix(go) {
                out.push_str(ch);
                rest = r;
                continue 'outer;
            }
        }
        let c = rest.chars().next().unwrap();
        if c == '%' {
            out.push_str("%%");
        } else {
            out.push(c);
        }
        rest = &rest[c.len_utf8()..];
    }
    out
}

/// Parses `value` with a Go layout. Missing date parts default to today,
/// missing time to midnight; times without a zone are taken as UTC.
pub fn parse_with_layout(value: &str, layout: &str, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let fmt = go_layout_to_chrono(layout);
    let value = value.trim();
    if let Ok(dt) = DateTime::parse_from_str(value, &fmt) {
        return Some(dt.with_timezone(&Utc));
    }
    if let Ok(dt) = NaiveDateTime::parse_from_str(value, &fmt) {
        return Some(Utc.from_utc_datetime(&dt));
    }
    if let Ok(d) = NaiveDate::parse_from_str(value, &fmt) {
        return Some(Utc.from_utc_datetime(&d.and_time(NaiveTime::MIN)));
    }
    // Layouts without a year, e.g. "Jan 02 15:04": add this year.
    if !fmt.contains("%Y") && !fmt.contains("%y") {
        let with_year = format!("{value} {}", now.year());
        let fmt = format!("{fmt} %Y");
        if let Ok(dt) = NaiveDateTime::parse_from_str(&with_year, &fmt) {
            return Some(Utc.from_utc_datetime(&dt));
        }
    }
    if let Ok(t) = NaiveTime::parse_from_str(value, &fmt) {
        return Some(Utc.from_utc_datetime(&now.date_naive().and_time(t)));
    }
    None
}

/// "3 days 4 hours ago", "1 week", "5 mins", "2h": that long before `now`.
pub fn parse_ago(value: &str, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    static PART: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)(\d+(?:[.,]\d+)?)\s*([a-zа-я]+)").unwrap());
    let v = value.trim().to_lowercase();
    if v == "now" || v == "just now" {
        return Some(now);
    }
    let mut total = TimeDelta::zero();
    let mut any = false;
    for c in PART.captures_iter(&v) {
        let n: f64 = c[1].replace(',', ".").parse().ok()?;
        let unit = &c[2];
        let secs = if unit.starts_with("mo") {
            30.0 * 86400.0
        } else if unit.starts_with("mi") || unit == "m" {
            60.0
        } else if unit.starts_with('s') {
            1.0
        } else if unit.starts_with('h') {
            3600.0
        } else if unit.starts_with('d') {
            86400.0
        } else if unit.starts_with('w') {
            7.0 * 86400.0
        } else if unit.starts_with('y') {
            365.0 * 86400.0
        } else {
            continue;
        };
        total += TimeDelta::milliseconds((n * secs * 1000.0) as i64);
        any = true;
    }
    any.then(|| now - total)
}

/// "Today, 14:20", "Yesterday 09:00", "today": relative day names.
pub fn parse_fuzzy(value: &str, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    static DAY: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r"(?i)^\s*(today|yesterday|tomorrow)[\s,at]*(\d{1,2}:\d{2}(?::\d{2})?\s*(?:[ap]m)?)?",
        )
        .unwrap()
    });
    if let Some(c) = DAY.captures(value) {
        let offset = match c[1].to_lowercase().as_str() {
            "yesterday" => -1,
            "tomorrow" => 1,
            _ => 0,
        };
        let day = now.date_naive() + TimeDelta::days(offset);
        let time = c
            .get(2)
            .and_then(|t| {
                let t = t.as_str().trim().to_uppercase();
                ["%H:%M:%S", "%H:%M", "%I:%M %p", "%I:%M%p", "%I:%M:%S %p"]
                    .iter()
                    .find_map(|f| NaiveTime::parse_from_str(&t, f).ok())
            })
            .unwrap_or(NaiveTime::MIN);
        return Some(Utc.from_utc_datetime(&day.and_time(time)));
    }
    parse_ago(value, now)
        .filter(|_| value.to_lowercase().contains("ago") || value.trim() == "now")
        .or_else(|| parse_lenient(value, now))
}

/// Best effort for whatever ends up in the `date` field.
pub fn parse_lenient(value: &str, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let v = value.trim();
    if v.is_empty() {
        return None;
    }
    // Unix seconds or milliseconds.
    if v.chars().all(|c| c.is_ascii_digit()) {
        let n: i64 = v.parse().ok()?;
        return match v.len() {
            9..=10 => Utc.timestamp_opt(n, 0).single(),
            12..=13 => Utc.timestamp_millis_opt(n).single(),
            _ => None,
        };
    }
    if let Ok(d) = DateTime::parse_from_rfc3339(v) {
        return Some(d.with_timezone(&Utc));
    }
    if let Ok(d) = DateTime::parse_from_rfc2822(v) {
        return Some(d.with_timezone(&Utc));
    }
    const NAIVE: &[&str] = &[
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%d %H:%M",
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%d %H:%M:%S%.f",
        "%d.%m.%Y %H:%M:%S",
        "%d.%m.%Y %H:%M",
        "%d/%m/%Y %H:%M:%S",
        "%d/%m/%Y %H:%M",
        "%m/%d/%Y %I:%M:%S %p",
        "%m/%d/%Y %I:%M %p",
        "%b %d %Y %H:%M",
        "%d %b %Y %H:%M",
        "%d %b %Y, %H:%M",
        "%B %d, %Y %H:%M",
    ];
    for f in NAIVE {
        if let Ok(d) = NaiveDateTime::parse_from_str(v, f) {
            return Some(Utc.from_utc_datetime(&d));
        }
    }
    const DATES: &[&str] = &[
        "%Y-%m-%d",
        "%d.%m.%Y",
        "%d/%m/%Y",
        "%d %b %Y",
        "%b %d %Y",
        "%B %d, %Y",
        "%d %B %Y",
    ];
    for f in DATES {
        if let Ok(d) = NaiveDate::parse_from_str(v, f) {
            return Some(Utc.from_utc_datetime(&d.and_time(NaiveTime::MIN)));
        }
    }
    if v.to_lowercase().contains("ago") {
        return parse_ago(v, now);
    }
    if v.to_lowercase().starts_with("today") || v.to_lowercase().starts_with("yesterday") {
        return parse_fuzzy(v, now);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 14, 12, 0, 0).unwrap()
    }

    #[test]
    fn go_layouts() {
        assert_eq!(
            go_layout_to_chrono("2006-01-02 15:04:05"),
            "%Y-%m-%d %H:%M:%S"
        );
        assert_eq!(
            go_layout_to_chrono("Jan 2, 2006 3:04 PM"),
            "%b %d, %Y %I:%M %p"
        );
        let d = parse_with_layout("13 Oct 2026 08:30", "02 Jan 2006 15:04", now()).unwrap();
        assert_eq!(d.to_rfc3339(), "2026-10-13T08:30:00+00:00");
        let d = parse_with_layout("2026-10-13", "2006-01-02", now()).unwrap();
        assert_eq!(d.to_rfc3339(), "2026-10-13T00:00:00+00:00");
        let d = parse_with_layout("Oct 13 08:30", "Jan 02 15:04", now()).unwrap();
        assert_eq!(d.year(), 2026);
        let d =
            parse_with_layout("2026-10-13 08:30 +0200", "2006-01-02 15:04 -0700", now()).unwrap();
        assert_eq!(d.to_rfc3339(), "2026-10-13T06:30:00+00:00");
    }

    #[test]
    fn relative() {
        assert_eq!(
            parse_ago("3 hours ago", now()),
            Some(now() - TimeDelta::hours(3))
        );
        assert_eq!(
            parse_ago("1 day 2 hours", now()),
            Some(now() - TimeDelta::hours(26))
        );
        assert_eq!(
            parse_ago("2 weeks ago", now()),
            Some(now() - TimeDelta::days(14))
        );
        assert_eq!(
            parse_ago("5 mins", now()),
            Some(now() - TimeDelta::minutes(5))
        );
        assert_eq!(
            parse_ago("1 month", now()),
            Some(now() - TimeDelta::days(30))
        );
        assert_eq!(parse_ago("nonsense", now()), None);
        let y = parse_fuzzy("Yesterday, 09:15", now()).unwrap();
        assert_eq!(y.to_rfc3339(), "2026-10-13T09:15:00+00:00");
        let t = parse_fuzzy("Today 2:05 pm", now()).unwrap();
        assert_eq!(t.to_rfc3339(), "2026-10-14T14:05:00+00:00");
    }

    #[test]
    fn lenient() {
        assert_eq!(
            parse_lenient("1760342400", now()).unwrap().to_rfc3339(),
            "2025-10-13T08:00:00+00:00"
        );
        assert_eq!(
            parse_lenient("2026-10-13 08:00:00", now()).unwrap().day(),
            13
        );
        assert_eq!(
            parse_lenient("Tue, 13 Oct 2026 08:00:00 +0000", now())
                .unwrap()
                .day(),
            13
        );
        assert_eq!(
            parse_lenient("13.10.2026 08:00", now()).unwrap().month(),
            10
        );
        assert!(parse_lenient("2 hours ago", now()).is_some());
        assert!(parse_lenient("whenever", now()).is_none());
    }
}
