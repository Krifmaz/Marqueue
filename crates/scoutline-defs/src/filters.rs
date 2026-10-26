//! The Cardigann filter set, applied in order to a field's raw text.

use chrono::{DateTime, Utc};

use crate::dates;
use crate::model::Filter;
use crate::template::cached_regex;

/// Applies `filters` to `value`. Unknown filters are an error so broken
/// definitions show up in validation instead of returning wrong data.
pub fn apply(value: &str, filters: &[Filter], now: DateTime<Utc>) -> Result<String, String> {
    let mut v = value.to_owned();
    for f in filters {
        v = apply_one(&v, f, now).map_err(|e| format!("filter {}: {e}", f.name))?;
    }
    Ok(v)
}

fn arg(f: &Filter, i: usize) -> Result<&str, String> {
    f.args
        .get(i)
        .ok_or_else(|| format!("missing argument {}", i + 1))
}

fn apply_one(v: &str, f: &Filter, now: DateTime<Utc>) -> Result<String, String> {
    Ok(match f.name.as_str() {
        "querystring" => {
            let key = arg(f, 0)?;
            // Works on full URLs and on bare `a=1&b=2` / `?a=1` strings.
            let query = v.split_once('?').map_or(v, |(_, q)| q);
            url::form_urlencoded::parse(query.as_bytes())
                .find(|(k, _)| k == key)
                .map(|(_, val)| val.into_owned())
                .unwrap_or_default()
        }
        "regexp" => cached_regex(arg(f, 0)?)?.first_group(v).unwrap_or_default(),
        "re_replace" => cached_regex(arg(f, 0)?)?.replace_all(v, f.args.get(1).unwrap_or("")),
        "replace" => v.replace(arg(f, 0)?, f.args.get(1).unwrap_or("")),
        "split" => {
            let sep = arg(f, 0)?;
            let parts: Vec<&str> = v.split(sep).collect();
            let i: i64 = arg(f, 1)?
                .trim()
                .parse()
                .map_err(|_| "index must be a number")?;
            let idx = if i < 0 { parts.len() as i64 + i } else { i };
            usize::try_from(idx)
                .ok()
                .and_then(|i| parts.get(i))
                .map_or_else(String::new, |s| (*s).to_owned())
        }
        "trim" => match f.args.get(0) {
            Some(cut) if !cut.is_empty() => v.trim_matches(|c| cut.contains(c)).to_owned(),
            _ => v.trim().to_owned(),
        },
        "prepend" => format!("{}{v}", arg(f, 0)?),
        "append" => format!("{v}{}", arg(f, 0)?),
        "tolower" => v.to_lowercase(),
        "toupper" => v.to_uppercase(),
        "urldecode" => url::form_urlencoded::parse(format!("x={v}").as_bytes())
            .next()
            .map(|(_, d)| d.into_owned())
            .unwrap_or_default(),
        "urlencode" => url::form_urlencoded::byte_serialize(v.as_bytes()).collect(),
        "htmldecode" => html_decode(v),
        "htmlencode" => v
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;"),
        "dateparse" | "timeparse" => {
            let layout = arg(f, 0)?;
            dates::parse_with_layout(v, layout, now)
                .ok_or_else(|| format!("{v:?} doesn't match layout {layout:?}"))?
                .to_rfc3339()
        }
        "timeago" | "reltime" => dates::parse_ago(v, now)
            .ok_or_else(|| format!("{v:?} isn't a relative time"))?
            .to_rfc3339(),
        "fuzzytime" => dates::parse_fuzzy(v, now)
            .ok_or_else(|| format!("{v:?} isn't a date"))?
            .to_rfc3339(),
        "validfilename" => v
            .chars()
            .map(|c| {
                if "<>:\"/\\|?*".contains(c) || c.is_control() {
                    '_'
                } else {
                    c
                }
            })
            .collect(),
        "diacritics" => {
            use unicode_normalization::UnicodeNormalization;
            v.nfd()
                .filter(|c| !unicode_normalization::char::is_combining_mark(*c))
                .collect()
        }
        "striprussianletters" => v
            .chars()
            .filter(|c| !matches!(c, 'а'..='я' | 'А'..='Я' | 'ё' | 'Ё'))
            .collect::<String>()
            .trim()
            .to_owned(),
        "jsonjoinarray" => {
            let path = arg(f, 0)?;
            let sep = arg(f, 1)?;
            let json: serde_json::Value = serde_json::from_str(v).map_err(|e| e.to_string())?;
            let node = crate::extract::json_path(&json, path)
                .cloned()
                .unwrap_or_default();
            match node {
                serde_json::Value::Array(a) => a
                    .iter()
                    .map(crate::extract::json_text)
                    .collect::<Vec<_>>()
                    .join(sep),
                other => crate::extract::json_text(&other),
            }
        }
        "validate" => {
            // Keep only the words that appear in the allowed list.
            let allowed: Vec<String> = arg(f, 0)?
                .split(',')
                .map(|s| s.trim().to_lowercase())
                .filter(|s| !s.is_empty())
                .collect();
            v.split([',', '/', '|', ' '])
                .map(str::trim)
                .filter(|w| !w.is_empty() && allowed.contains(&w.to_lowercase()))
                .collect::<Vec<_>>()
                .join(", ")
        }
        // Keyword-only and debugging filters leave the value alone.
        "andmatch" | "strdump" | "hexdump" => v.to_owned(),
        other => return Err(format!("unknown filter {other:?}")),
    })
}

/// Decodes the HTML entities found in tracker pages.
pub fn html_decode(s: &str) -> String {
    if !s.contains('&') {
        return s.to_owned();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(end) = rest[..rest.len().min(12)].find(';') else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let ent = &rest[1..end];
        let decoded = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" | "#39" => Some('\''),
            "nbsp" => Some(' '),
            e if e.starts_with("#x") || e.starts_with("#X") => u32::from_str_radix(&e[2..], 16)
                .ok()
                .and_then(char::from_u32),
            e if e.starts_with('#') => e[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        };
        match decoded {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::FilterArgs;
    use chrono::TimeZone;

    fn f(name: &str, args: &[&str]) -> Filter {
        Filter {
            name: name.into(),
            args: FilterArgs(args.iter().map(|s| s.to_string()).collect()),
        }
    }

    fn run(v: &str, filters: &[Filter]) -> String {
        apply(
            v,
            filters,
            Utc.with_ymd_and_hms(2026, 10, 14, 12, 0, 0).unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn string_filters() {
        assert_eq!(
            run("browse.php?cat=7&page=2", &[f("querystring", &["cat"])]),
            "7"
        );
        assert_eq!(
            run("Size: 1.5 GB", &[f("regexp", &[r"([\d.]+ \w+)"])]),
            "1.5 GB"
        );
        assert_eq!(
            run("The.Bear.S03", &[f("re_replace", &[r"\.", " "])]),
            "The Bear S03"
        );
        assert_eq!(run("a-b-c", &[f("split", &["-", "1"])]), "b");
        assert_eq!(run("a-b-c", &[f("split", &["-", "-1"])]), "c");
        assert_eq!(run("  x  ", &[f("trim", &[])]), "x");
        assert_eq!(run("[x]", &[f("trim", &["[]"])]), "x");
        assert_eq!(
            run(
                "x",
                &[f("prepend", &["<"]), f("append", &[">"]), f("toupper", &[])]
            ),
            "<X>"
        );
        assert_eq!(run("a%20b+c", &[f("urldecode", &[])]), "a b c");
        assert_eq!(
            run("Tom &amp; Jerry &#39;99&#x21;", &[f("htmldecode", &[])]),
            "Tom & Jerry '99!"
        );
        assert_eq!(run("Amélie", &[f("diacritics", &["replace"])]), "Amelie");
        assert_eq!(
            run(
                "Action, Drama, Nope",
                &[f("validate", &["action, drama, comedy"])]
            ),
            "Action, Drama"
        );
        assert_eq!(
            run(r#"{"g":["a","b"]}"#, &[f("jsonjoinarray", &["$.g", ", "])]),
            "a, b"
        );
    }

    #[test]
    fn lookarounds_use_the_fallback_engine() {
        assert_eq!(
            run("S1E2", &[f("re_replace", &[r"S(\d)(?=E)", "S0$1"])]),
            "S01E2"
        );
        assert_eq!(
            run("a1 2", &[f("re_replace", &[r"(?<=\d)\s+(?=\d)", ""])]),
            "a12"
        );
        assert_eq!(
            run(
                "MULTI FRENCH",
                &[f("regexp", &[r"(?i)\b(MULTI(?!.*ENGLISH))\b"])]
            ),
            "MULTI"
        );
    }

    #[test]
    fn date_filters() {
        assert_eq!(
            run("13/10/2026", &[f("dateparse", &["02/01/2006"])]),
            "2026-10-13T00:00:00+00:00"
        );
        assert_eq!(
            run("2 hours ago", &[f("timeago", &[])]),
            "2026-10-14T10:00:00+00:00"
        );
        assert_eq!(
            run("Yesterday 10:00", &[f("fuzzytime", &[])]),
            "2026-10-13T10:00:00+00:00"
        );
    }

    #[test]
    fn unknown_filter_is_an_error() {
        assert!(apply("x", &[f("frobnicate", &[])], Utc::now()).is_err());
    }
}
