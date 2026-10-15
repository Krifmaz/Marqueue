//! Reads result rows and fields out of HTML and JSON responses.

use std::sync::LazyLock;

use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use regex::Regex;
use scraper::{ElementRef, Html, Selector};
use serde_json::Value as Json;

use crate::filters;
use crate::model::{Field, Rows};
use crate::template::{self, Value};

/// Field values of one row, in definition order.
pub type RowValues = IndexMap<String, String>;

/// Why a row was dropped (kept for validation reports).
#[derive(Debug, Clone, PartialEq)]
pub struct RowError {
    pub row: usize,
    pub message: String,
}

/// Reads every row of an HTML page.
pub fn html_rows(
    page: &str,
    rows: &Rows,
    fields: &IndexMap<String, Field>,
    vars: &Value,
    now: DateTime<Utc>,
) -> Result<(Vec<RowValues>, Vec<RowError>), String> {
    let doc = Html::parse_document(page);
    let root = doc.root_element();
    let matched = select_all(root, root, &rows.selector)?;

    // `after: n` folds the next n rows into each match (multi-line rows).
    let step = rows.after + 1;
    let groups: Vec<Vec<ElementRef>> = if rows.after == 0 {
        matched.into_iter().map(|r| vec![r]).collect()
    } else {
        let mut groups = Vec::new();
        for el in matched {
            let mut g = vec![el];
            g.extend(
                el.next_siblings()
                    .filter_map(ElementRef::wrap)
                    .take(rows.after),
            );
            groups.push(g);
        }
        groups
    };
    let _ = step;

    let mut out = Vec::new();
    let mut errors = Vec::new();
    for (n, group) in groups.iter().enumerate() {
        let mut values = RowValues::new();
        let mut failed = None;
        for (name, field) in fields {
            match html_field(
                root,
                group,
                field,
                rows.remove.as_deref(),
                &values,
                vars,
                now,
            ) {
                Ok(Some(v)) => {
                    values.insert(name.clone(), v);
                }
                Ok(None) => {}
                Err(e) => {
                    failed = Some(format!("field {name}: {e}"));
                    break;
                }
            }
        }
        if failed.is_none() && !values.contains_key("date") {
            if let Some(dh) = &rows.dateheaders {
                if let Some(d) = date_header(root, group[0], dh, vars, now) {
                    values.insert("date".into(), d);
                }
            }
        }
        match failed {
            Some(message) => errors.push(RowError { row: n, message }),
            None => out.push(values),
        }
    }
    Ok((out, errors))
}

/// Reads one field. `Ok(None)` means optional and absent.
fn html_field(
    root: ElementRef,
    row: &[ElementRef],
    field: &Field,
    remove: Option<&str>,
    so_far: &RowValues,
    vars: &Value,
    now: DateTime<Utc>,
) -> Result<Option<String>, String> {
    let mut value = if let Some(text) = &field.text {
        Some(template::render(text, &with_result(vars, so_far)).map_err(|e| e.to_string())?)
    } else if let Some(sel) = &field.selector {
        let sel = template::render(sel, &with_result(vars, so_far)).map_err(|e| e.to_string())?;
        let found = row
            .iter()
            .find_map(|el| select_first(root, *el, &sel).transpose())
            .transpose()?;
        match found {
            Some(el) if !field.case.is_empty() => case_value(root, el, &field.case)?,
            Some(el) => match &field.attribute {
                Some(attr) => el.value().attr(attr).map(str::to_owned),
                None => Some(text_without(el, field.remove.as_deref().or(remove))?),
            },
            None => None,
        }
    } else if !field.case.is_empty() {
        case_value(root, row[0], &field.case)?
    } else {
        None
    };

    if let Some(v) = &value {
        let v = filters::apply(v, &field.filters, now)?;
        value = Some(v);
    }
    match value {
        Some(v) if !v.is_empty() => Ok(Some(v)),
        _ => match &field.default {
            Some(d) => Ok(Some(
                template::render(d, &with_result(vars, so_far)).map_err(|e| e.to_string())?,
            )),
            None if field.optional => Ok(None),
            None if value.is_some() => Ok(Some(String::new())),
            None => Err(format!(
                "selector {:?} found nothing",
                field.selector.as_deref().unwrap_or("")
            )),
        },
    }
}

/// `case:` picks the value of the first selector the element matches or
/// contains; `*` matches anything.
fn case_value(
    root: ElementRef,
    el: ElementRef,
    cases: &IndexMap<String, String>,
) -> Result<Option<String>, String> {
    for (sel, v) in cases {
        if sel == "*" {
            return Ok(Some(v.clone()));
        }
        if element_matches(root, el, sel)? || select_first(root, el, sel)?.is_some() {
            return Ok(Some(v.clone()));
        }
    }
    Ok(None)
}

/// Text of the nearest earlier sibling row that is a date header.
fn date_header(
    root: ElementRef,
    row: ElementRef,
    field: &Field,
    vars: &Value,
    now: DateTime<Utc>,
) -> Option<String> {
    let sel = parse_selector(field.selector.as_deref()?).ok()?;
    let header = row
        .prev_siblings()
        .filter_map(ElementRef::wrap)
        .find(|e| sel.matches(e))?;
    let f = Field {
        selector: None,
        text: Some(text_without(header, field.remove.as_deref()).ok()?),
        ..field.clone()
    };
    html_field(root, &[header], &f, None, &RowValues::new(), vars, now)
        .ok()
        .flatten()
}

fn with_result(vars: &Value, result: &RowValues) -> Value {
    let mut v = vars.clone();
    if let Value::Map(m) = &mut v {
        m.insert(
            "Result".into(),
            result
                .iter()
                .map(|(k, v)| (k.clone(), Value::from(v.clone())))
                .collect(),
        );
    }
    v
}

/// Element text with whitespace collapsed, skipping `remove` matches.
fn text_without(el: ElementRef, remove: Option<&str>) -> Result<String, String> {
    let skip = remove.map(parse_selector).transpose()?;
    let mut out = String::new();
    collect_text(el, skip.as_ref(), &mut out);
    Ok(collapse_ws(&out))
}

fn collect_text(el: ElementRef, skip: Option<&Selector>, out: &mut String) {
    for child in el.children() {
        if let Some(t) = child.value().as_text() {
            out.push_str(t);
        } else if let Some(c) = ElementRef::wrap(child) {
            if skip.is_some_and(|s| s.matches(&c)) {
                continue;
            }
            if matches!(c.value().name(), "br" | "p" | "div" | "td" | "li") {
                out.push(' ');
            }
            collect_text(c, skip, out);
        }
    }
}

fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn parse_selector(sel: &str) -> Result<Selector, String> {
    Selector::parse(sel).map_err(|e| format!("bad selector {sel:?}: {e}"))
}

/// Selectors with the jQuery extensions definitions use: `:root` (search
/// from the document), `:contains(text)` anywhere (also inside `:not()`
/// and `:has()`), and `[attr!=value]`. Plain CSS goes straight to
/// `scraper`.
pub fn select_all<'a>(
    root: ElementRef<'a>,
    scope: ElementRef<'a>,
    sel: &str,
) -> Result<Vec<ElementRef<'a>>, String> {
    let sel = rewrite_not_equal(sel.trim());
    let (scope, sel) = match sel.strip_prefix(":root") {
        Some(rest) => (root, rest.trim_start().to_owned()),
        None => (scope, sel),
    };
    if sel.is_empty() {
        return Ok(vec![scope]);
    }
    if !sel.contains(":contains") {
        if let Ok(parsed) = parse_selector(&sel) {
            return Ok(scope.select(&parsed).collect());
        }
    }
    let mut out: Vec<ElementRef> = Vec::new();
    for group in split_top(&sel, ',') {
        for el in eval_group(scope, group.trim())? {
            if !out.iter().any(|o| o.id() == el.id()) {
                out.push(el);
            }
        }
    }
    Ok(out)
}

/// jQuery-isms to CSS: `[a!=b]` -> `:not([a=b])`, and unquoted values
/// that aren't identifiers (`[border=1]`) get quotes.
fn rewrite_not_equal(sel: &str) -> String {
    static NE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"\[\s*([\w-]+)\s*!=\s*([^\]]+)\]"#).unwrap());
    static BARE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"\[\s*([\w-]+)\s*([~|^$*]?=)\s*(-?\d[^\]"'\s]*)\s*\]"#).unwrap()
    });
    let s = if sel.contains("!=") {
        NE.replace_all(sel, ":not([$1=$2])").into_owned()
    } else {
        sel.to_owned()
    };
    BARE.replace_all(&s, r#"[$1$2"$3"]"#).into_owned()
}

#[derive(Clone, Copy, PartialEq)]
enum Comb {
    Descendant,
    Child,
    Adjacent,
    Sibling,
}

/// One compound selector split into plain CSS plus text predicates.
struct Compound {
    css: Option<Selector>,
    preds: Vec<Pred>,
}

enum Pred {
    Contains(String),
    NotContains(String),
    /// `:not(x)` / `:has(x)` where `x` itself uses `:contains`.
    Not(String),
    Has(String),
}

/// Splits on `sep` outside quotes, brackets and parentheses.
fn split_top(s: &str, sep: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let (mut depth, mut quote, mut start) = (0i32, None::<char>, 0);
    for (i, c) in s.char_indices() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, '(' | '[') => depth += 1,
            (None, ')' | ']') => depth -= 1,
            (None, c) if c == sep && depth == 0 => {
                parts.push(&s[start..i]);
                start = i + c.len_utf8();
            }
            _ => {}
        }
    }
    parts.push(&s[start..]);
    parts
}

/// `a > b c + d` -> [(Descendant, a), (Child, b), (Descendant, c), (Adjacent, d)].
/// A leading combinator (`> a` inside `:has`) applies to the scope.
fn split_steps(s: &str) -> Vec<(Comb, String)> {
    let mut steps = Vec::new();
    let mut comb = Comb::Descendant;
    let mut cur = String::new();
    let (mut depth, mut quote) = (0i32, None::<char>);
    let mut pending_space = false;
    for c in s.chars() {
        if let Some(q) = quote {
            cur.push(c);
            if c == q {
                quote = None;
            }
            continue;
        }
        match c {
            '"' | '\'' => {
                quote = Some(c);
                cur.push(c);
            }
            '(' | '[' => {
                depth += 1;
                cur.push(c);
            }
            ')' | ']' => {
                depth -= 1;
                cur.push(c);
            }
            '>' | '+' | '~' if depth == 0 => {
                if !cur.trim().is_empty() {
                    steps.push((comb, cur.trim().to_owned()));
                    cur.clear();
                }
                comb = match c {
                    '>' => Comb::Child,
                    '+' => Comb::Adjacent,
                    _ => Comb::Sibling,
                };
                pending_space = false;
            }
            c if c.is_whitespace() && depth == 0 => {
                if !cur.trim().is_empty() {
                    pending_space = true;
                }
            }
            _ => {
                if pending_space {
                    steps.push((comb, cur.trim().to_owned()));
                    cur.clear();
                    comb = Comb::Descendant;
                    pending_space = false;
                }
                cur.push(c);
            }
        }
    }
    if !cur.trim().is_empty() {
        steps.push((comb, cur.trim().to_owned()));
    }
    steps
}

/// The argument of a pseudo-class starting at `s` (just after `(`), and
/// the rest after the closing `)`.
fn pseudo_arg(s: &str) -> Option<(&str, &str)> {
    let (mut depth, mut quote) = (1i32, None::<char>);
    for (i, c) in s.char_indices() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, '(') => depth += 1,
            (None, ')') => {
                depth -= 1;
                if depth == 0 {
                    return Some((&s[..i], &s[i + 1..]));
                }
            }
            _ => {}
        }
    }
    None
}

fn unquote(s: &str) -> String {
    let s = s.trim();
    s.strip_prefix('"')
        .and_then(|x| x.strip_suffix('"'))
        .or_else(|| s.strip_prefix('\'').and_then(|x| x.strip_suffix('\'')))
        .unwrap_or(s)
        .to_owned()
}

fn parse_compound(s: &str) -> Result<Compound, String> {
    let mut css = String::new();
    let mut preds = Vec::new();
    let mut rest = s;
    while !rest.is_empty() {
        let lower = rest;
        let take = |prefix: &str| lower.strip_prefix(prefix).and_then(pseudo_arg);
        if let Some((arg, after)) = take(":contains(") {
            preds.push(Pred::Contains(unquote(arg)));
            rest = after;
        } else if let Some((arg, after)) = take(":not(").filter(|(a, _)| needs_custom(a)) {
            match arg.trim().strip_prefix(":contains(").and_then(pseudo_arg) {
                Some((inner, tail)) if tail.trim().is_empty() => {
                    preds.push(Pred::NotContains(unquote(inner)))
                }
                _ => preds.push(Pred::Not(arg.to_owned())),
            }
            rest = after;
        } else if let Some((arg, after)) = take(":has(").filter(|(a, _)| needs_custom(a)) {
            preds.push(Pred::Has(arg.to_owned()));
            rest = after;
        } else {
            // Copy one unit: a quoted string, a bracket/paren group, or a char.
            let c = rest.chars().next().unwrap();
            let len = match c {
                '[' => rest.find(']').map_or(rest.len(), |i| i + 1),
                '(' => pseudo_arg(&rest[1..]).map_or(rest.len(), |(a, _)| a.len() + 2),
                _ => c.len_utf8(),
            };
            css.push_str(&rest[..len]);
            rest = &rest[len..];
        }
    }
    let css = css.trim();
    Ok(Compound {
        css: if css.is_empty() || css == "*" {
            None
        } else {
            Some(parse_selector(css)?)
        },
        preds,
    })
}

/// True when `arg` of `:not()`/`:has()` needs Scoutline's evaluator:
/// `:contains`, or a nested `:has` that `scraper` rejects.
fn needs_custom(arg: &str) -> bool {
    arg.contains(":contains") || arg.contains(":has(")
}

fn compound_matches(root: ElementRef, el: ElementRef, c: &Compound) -> Result<bool, String> {
    if c.css.as_ref().is_some_and(|s| !s.matches(&el)) {
        return Ok(false);
    }
    for p in &c.preds {
        let ok = match p {
            Pred::Contains(t) => el.text().collect::<String>().contains(t.as_str()),
            Pred::NotContains(t) => !el.text().collect::<String>().contains(t.as_str()),
            Pred::Not(inner) => !split_top(inner, ',')
                .into_iter()
                .map(|g| compound_matches(root, el, &parse_compound(g.trim())?))
                .collect::<Result<Vec<_>, String>>()?
                .into_iter()
                .any(|m| m),
            Pred::Has(inner) => !select_all(root, el, inner)?.is_empty(),
        };
        if !ok {
            return Ok(false);
        }
    }
    Ok(true)
}

fn eval_group<'a>(scope: ElementRef<'a>, group: &str) -> Result<Vec<ElementRef<'a>>, String> {
    let root = scope;
    let mut current = vec![scope];
    for (comb, text) in split_steps(group) {
        let compound = parse_compound(&text)?;
        let mut next: Vec<ElementRef> = Vec::new();
        for el in &current {
            let candidates: Vec<ElementRef> = match comb {
                Comb::Descendant => el
                    .descendants()
                    .skip(1)
                    .filter_map(ElementRef::wrap)
                    .collect(),
                Comb::Child => el.children().filter_map(ElementRef::wrap).collect(),
                Comb::Adjacent => el
                    .next_siblings()
                    .filter_map(ElementRef::wrap)
                    .take(1)
                    .collect(),
                Comb::Sibling => el.next_siblings().filter_map(ElementRef::wrap).collect(),
            };
            for c in candidates {
                if !next.iter().any(|n| n.id() == c.id()) && compound_matches(root, c, &compound)? {
                    next.push(c);
                }
            }
        }
        current = next;
    }
    Ok(current)
}

/// Checks a selector parses, without a page (definition validation).
pub fn check_selector(sel: &str) -> Result<(), String> {
    let sel = rewrite_not_equal(sel.trim());
    let sel = sel.strip_prefix(":root").unwrap_or(&sel).trim();
    if sel.is_empty() {
        return Ok(());
    }
    if !sel.contains(":contains") && parse_selector(sel).is_ok() {
        return Ok(());
    }
    for g in split_top(sel, ',') {
        for (_, step) in split_steps(g.trim()) {
            for p in parse_compound(&step)?.preds {
                match p {
                    Pred::Not(inner) => split_top(&inner, ',')
                        .into_iter()
                        .try_for_each(|i| check_selector(i.trim()))?,
                    Pred::Has(inner) => check_selector(&inner)?,
                    _ => {}
                }
            }
        }
    }
    Ok(())
}

/// True if `el` matches `sel` (used by `case:`).
pub fn element_matches(root: ElementRef, el: ElementRef, sel: &str) -> Result<bool, String> {
    let sel = rewrite_not_equal(sel.trim());
    if !sel.contains(":contains") {
        if let Ok(parsed) = parse_selector(&sel) {
            return Ok(parsed.matches(&el));
        }
    }
    for g in split_top(&sel, ',') {
        let steps = split_steps(g.trim());
        if steps.len() == 1 && compound_matches(root, el, &parse_compound(&steps[0].1)?)? {
            return Ok(true);
        }
    }
    Ok(false)
}

pub fn select_first<'a>(
    root: ElementRef<'a>,
    scope: ElementRef<'a>,
    sel: &str,
) -> Result<Option<ElementRef<'a>>, String> {
    Ok(select_all(root, scope, sel)?.into_iter().next())
}

// ---- JSON -------------------------------------------------------------------

/// Reads every row of a JSON response.
pub fn json_rows(
    body: &str,
    rows: &Rows,
    fields: &IndexMap<String, Field>,
    vars: &Value,
    now: DateTime<Utc>,
) -> Result<(Vec<RowValues>, Vec<RowError>), String> {
    let doc: Json = serde_json::from_str(body).map_err(|e| format!("response isn't JSON: {e}"))?;
    let Some(node) = json_path(&doc, &rows.selector) else {
        return Ok((Vec::new(), Vec::new()));
    };
    let top: Vec<&Json> = match node {
        Json::Array(a) => a.iter().collect(),
        Json::Object(o) if rows.multiple => o.values().collect(),
        other => vec![other],
    };
    // (row, parent) pairs; `rows.attribute` expands a nested array.
    let mut pairs: Vec<(&Json, Option<&Json>)> = Vec::new();
    for parent in top {
        match &rows.attribute {
            Some(attr) => match json_path(parent, attr) {
                Some(Json::Array(children)) => {
                    pairs.extend(children.iter().map(|c| (c, Some(parent))))
                }
                Some(child) => pairs.push((child, Some(parent))),
                None if rows.missing_attribute_is_empty => {}
                None => return Err(format!("rows attribute {attr:?} missing")),
            },
            None => pairs.push((parent, None)),
        }
    }

    let mut out = Vec::new();
    let mut errors = Vec::new();
    for (n, (row, parent)) in pairs.into_iter().enumerate() {
        let mut values = RowValues::new();
        let mut failed = None;
        for (name, field) in fields {
            match json_field(row, parent, field, &values, vars, now) {
                Ok(Some(v)) => {
                    values.insert(name.clone(), v);
                }
                Ok(None) => {}
                Err(e) => {
                    failed = Some(format!("field {name}: {e}"));
                    break;
                }
            }
        }
        match failed {
            Some(message) => errors.push(RowError { row: n, message }),
            None => out.push(values),
        }
    }
    Ok((out, errors))
}

fn json_field(
    row: &Json,
    parent: Option<&Json>,
    field: &Field,
    so_far: &RowValues,
    vars: &Value,
    now: DateTime<Utc>,
) -> Result<Option<String>, String> {
    let ctx = with_result(vars, so_far);
    let mut value = if let Some(text) = &field.text {
        Some(template::render(text, &ctx).map_err(|e| e.to_string())?)
    } else if let Some(sel) = &field.selector {
        let sel = template::render(sel, &ctx).map_err(|e| e.to_string())?;
        let found = match sel.strip_prefix("..") {
            Some(up) => parent.and_then(|p| json_path(p, up)),
            None => json_path(row, &sel),
        };
        found.filter(|v| !v.is_null()).map(|v| {
            let text = json_text(v);
            // `case` on JSON compares the value itself.
            field
                .case
                .iter()
                .find(|(k, _)| *k == "*" || k.eq_ignore_ascii_case(&text))
                .map_or(text, |(_, v)| v.clone())
        })
    } else {
        None
    };
    if let Some(v) = &value {
        value = Some(filters::apply(v, &field.filters, now)?);
    }
    match value {
        Some(v) if !v.is_empty() => Ok(Some(v)),
        _ => match &field.default {
            Some(d) => Ok(Some(template::render(d, &ctx).map_err(|e| e.to_string())?)),
            None if field.optional => Ok(None),
            None if value.is_some() => Ok(Some(String::new())),
            None => Err(format!(
                "{:?} not found",
                field.selector.as_deref().unwrap_or("")
            )),
        },
    }
}

/// Simple JSON paths: `$.data.items`, `data.items[0].name`, `items`.
pub fn json_path<'a>(v: &'a Json, path: &str) -> Option<&'a Json> {
    let path = path.trim().trim_start_matches('$').trim_start_matches('.');
    let mut cur = v;
    if path.is_empty() {
        return Some(cur);
    }
    for part in path.split('.') {
        let (key, indexes) = match part.find('[') {
            Some(i) => (&part[..i], &part[i..]),
            None => (part, ""),
        };
        if !key.is_empty() {
            cur = cur.get(key)?;
        }
        for idx in indexes.split('[').filter(|s| !s.is_empty()) {
            let i: usize = idx.trim_end_matches(']').parse().ok()?;
            cur = cur.get(i)?;
        }
    }
    Some(cur)
}

/// A JSON value as text, the way .NET prints it (so `case` keys written
/// for Prowlarr, like `True`, still match).
pub fn json_text(v: &Json) -> String {
    match v {
        Json::Null => String::new(),
        Json::Bool(true) => "True".into(),
        Json::Bool(false) => "False".into(),
        Json::String(s) => s.clone(),
        Json::Number(n) => n.to_string(),
        other => other.to_string(),
    }
}

// ---- values -----------------------------------------------------------------

/// "1.5 GB", "1,234.5 MiB", "1.234,5 MB", "700M", "123456": bytes.
pub fn parse_size(s: &str) -> Option<u64> {
    static SIZE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)([\d.,]+)\s*([kmgtp]?)(i?)(b|bytes?|o)?\b").unwrap());
    let s = s.trim();
    let c = SIZE.captures(s)?;
    let num = normalize_number(&c[1])?;
    let unit = c[2].to_ascii_lowercase();
    let power = match unit.as_str() {
        "" => 0,
        "k" => 1,
        "m" => 2,
        "g" => 3,
        "t" => 4,
        "p" => 5,
        _ => return None,
    };
    // Trackers mean 1024 even when they write "GB".
    Some((num * 1024f64.powi(power)).round() as u64)
}

/// Handles both `1,234.5` and `1.234,5`.
pub fn normalize_number(s: &str) -> Option<f64> {
    let s = s.trim();
    let (dots, commas) = (s.matches('.').count(), s.matches(',').count());
    let cleaned = match (dots, commas) {
        (_, 0) if dots <= 1 => s.to_owned(),
        (_, 0) => s.replace('.', ""),
        (0, 1) if s.len() - s.find(',')? - 1 != 3 => s.replace(',', "."),
        (0, _) => s.replace(',', ""),
        _ if s.rfind(',')? > s.rfind('.')? => s.replace('.', "").replace(',', "."),
        _ => s.replace(',', ""),
    };
    cleaned.parse().ok()
}

/// "1,234" or "1.2k" seeders/grabs.
pub fn parse_count(s: &str) -> Option<u32> {
    let t = s.trim().to_lowercase();
    if let Some(k) = t.strip_suffix('k') {
        return Some((normalize_number(k)? * 1000.0) as u32);
    }
    let digits: String = t
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == '.' || *c == ',')
        .collect();
    normalize_number(&digits).map(|n| n as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes() {
        assert_eq!(parse_size("1.5 GB"), Some(1_610_612_736));
        assert_eq!(parse_size("1,5 GiB"), Some(1_610_612_736));
        assert_eq!(parse_size("1,234.5 MB"), Some(1_294_467_072));
        assert_eq!(parse_size("1.234,5 MB"), Some(1_294_467_072));
        assert_eq!(parse_size("700M"), Some(734_003_200));
        assert_eq!(parse_size("123456"), Some(123_456));
        assert_eq!(parse_size("2 TB"), Some(2_199_023_255_552));
        assert_eq!(parse_size("n/a"), None);
    }

    #[test]
    fn counts() {
        assert_eq!(parse_count("1,234"), Some(1234));
        assert_eq!(parse_count("1.2k"), Some(1200));
        assert_eq!(parse_count(" 17 "), Some(17));
        assert_eq!(parse_count("-"), None);
    }

    #[test]
    fn json_paths() {
        let v: Json = serde_json::from_str(r#"{"data":{"items":[{"n":"a"},{"n":"b"}]}}"#).unwrap();
        assert_eq!(json_path(&v, "$.data.items[1].n"), Some(&Json::from("b")));
        assert_eq!(
            json_path(&v, "data.items")
                .unwrap()
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert_eq!(json_path(&v, "nope"), None);
    }

    #[test]
    fn contains_selector() {
        let doc = Html::parse_document(
            "<table><tr><td>Size</td><td>1 GB</td></tr><tr><td>Seeders</td><td>5</td></tr></table>",
        );
        let root = doc.root_element();
        let found = select_all(root, root, r#"td:contains("Seeders") + td"#).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].text().collect::<String>(), "5");
        let rows = select_all(root, root, "tr:contains(Size)").unwrap();
        assert_eq!(rows.len(), 1);
        let rows = select_all(root, root, r#"tr:not(:contains("Size"))"#).unwrap();
        assert_eq!(rows.len(), 1);
        let rows = select_all(
            root,
            root,
            r#"table tr:has(td:contains("Seeders")) > td:nth-child(2)"#,
        )
        .unwrap();
        assert_eq!(rows[0].text().collect::<String>(), "5");
        let cells = select_all(root, root, r#"td[class!="x"]:contains("1 GB")"#).unwrap();
        assert_eq!(cells.len(), 1);
        let either =
            select_all(root, root, r#"td:contains("Size"), td:contains("Seeders")"#).unwrap();
        assert_eq!(either.len(), 2);
        let nested = select_all(
            root,
            root,
            "table:has(tr:has(td)) tr:not(:has(td:nth-child(3)))",
        )
        .unwrap();
        assert_eq!(nested.len(), 2);
        let doc2 = Html::parse_document(r#"<table border="1"><tr><td>x</td></tr></table>"#);
        let r2 = doc2.root_element();
        assert_eq!(select_all(r2, r2, "table[border=1] td").unwrap().len(), 1);
    }
}
