//! The subset of Go `text/template` that definitions use:
//! `{{ .Field.Path }}`, `{{ if }} / {{ else if }} / {{ else }} / {{ end }}`,
//! `{{ range }}`, trim markers `{{-` / `-}}`, parentheses, and the
//! functions `eq`, `ne`, `and`, `or`, `not`, `join`, `re_replace`, `len`
//! and `print`.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use regex::Regex;

/// A template value.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Value {
    #[default]
    Null,
    Bool(bool),
    Str(String),
    List(Vec<Value>),
    Map(HashMap<String, Value>),
}

impl Value {
    pub fn truthy(&self) -> bool {
        match self {
            Self::Null => false,
            Self::Bool(b) => *b,
            Self::Str(s) => !s.is_empty(),
            Self::List(l) => !l.is_empty(),
            Self::Map(m) => !m.is_empty(),
        }
    }

    /// The text Go would print.
    pub fn render(&self) -> String {
        match self {
            Self::Null => String::new(),
            Self::Bool(b) => b.to_string(),
            Self::Str(s) => s.clone(),
            Self::List(l) => format!(
                "[{}]",
                l.iter().map(Value::render).collect::<Vec<_>>().join(" ")
            ),
            Self::Map(_) => String::new(),
        }
    }

    fn get(&self, key: &str) -> Value {
        match self {
            Self::Map(m) => m.get(key).cloned().unwrap_or_default(),
            _ => Value::Null,
        }
    }
}

impl From<&str> for Value {
    fn from(s: &str) -> Self {
        Self::Str(s.to_owned())
    }
}

impl From<String> for Value {
    fn from(s: String) -> Self {
        Self::Str(s)
    }
}

impl From<Option<String>> for Value {
    fn from(s: Option<String>) -> Self {
        s.map_or(Self::Null, Self::Str)
    }
}

impl From<bool> for Value {
    fn from(b: bool) -> Self {
        Self::Bool(b)
    }
}

impl<K: Into<String>, V: Into<Value>> FromIterator<(K, V)> for Value {
    fn from_iter<I: IntoIterator<Item = (K, V)>>(iter: I) -> Self {
        Self::Map(
            iter.into_iter()
                .map(|(k, v)| (k.into(), v.into()))
                .collect(),
        )
    }
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("template {template:?}: {message}")]
pub struct TemplateError {
    pub template: String,
    pub message: String,
}

/// Renders `template` with `data` as the root `.`.
pub fn render(template: &str, data: &Value) -> Result<String, TemplateError> {
    if !template.contains("{{") {
        return Ok(template.to_owned());
    }
    let err = |message: String| TemplateError {
        template: template.to_owned(),
        message,
    };
    let tokens = lex(template).map_err(err)?;
    let mut pos = 0;
    let nodes = parse_nodes(&tokens, &mut pos, &[]).map_err(err)?;
    if pos < tokens.len() {
        return Err(err("unexpected {{ end }} or {{ else }}".to_owned()));
    }
    let mut out = String::new();
    exec(&nodes, data, data, &mut out).map_err(err)?;
    Ok(out)
}

// ---- lexing ---------------------------------------------------------------

#[derive(Debug, Clone)]
enum Tok {
    Text(String),
    Action(String),
}

fn lex(src: &str) -> Result<Vec<Tok>, String> {
    let mut toks = Vec::new();
    let mut rest = src;
    let mut trim_next_text = false;
    while let Some(start) = rest.find("{{") {
        let mut text = &rest[..start];
        if trim_next_text {
            text = text.trim_start();
        }
        let after = &rest[start + 2..];
        let trim_left = after.starts_with('-') && after[1..].starts_with(char::is_whitespace);
        if trim_left {
            text = text.trim_end();
        }
        if !text.is_empty() {
            toks.push(Tok::Text(text.to_owned()));
        }
        let body_start = if trim_left { 1 } else { 0 };
        let end = find_action_end(&after[body_start..]).ok_or("unclosed {{")?;
        let mut body = &after[body_start..body_start + end];
        trim_next_text = false;
        if body.ends_with('-') && body[..body.len() - 1].ends_with(char::is_whitespace) {
            body = &body[..body.len() - 1];
            trim_next_text = true;
        }
        toks.push(Tok::Action(body.trim().to_owned()));
        rest = &after[body_start + end + 2..];
    }
    let text = if trim_next_text {
        rest.trim_start()
    } else {
        rest
    };
    if !text.is_empty() {
        toks.push(Tok::Text(text.to_owned()));
    }
    Ok(toks)
}

/// Finds the closing `}}`, skipping any inside string literals.
fn find_action_end(s: &str) -> Option<usize> {
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'"' => {
                i += 1;
                while i < b.len() && b[i] != b'"' {
                    if b[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
            }
            b'`' => {
                i += 1;
                while i < b.len() && b[i] != b'`' {
                    i += 1;
                }
            }
            b'}' if b.get(i + 1) == Some(&b'}') => return Some(i),
            _ => {}
        }
        i += 1;
    }
    None
}

// ---- parsing --------------------------------------------------------------

#[derive(Debug, Clone)]
enum Node {
    Text(String),
    Print(Expr),
    If {
        branches: Vec<(Expr, Vec<Node>)>,
        otherwise: Vec<Node>,
    },
    Range {
        over: Expr,
        body: Vec<Node>,
        otherwise: Vec<Node>,
    },
}

#[derive(Debug, Clone)]
enum Expr {
    Field(Vec<String>),
    Dot,
    Lit(Value),
    Call(String, Vec<Expr>),
}

/// Parses nodes until one of `stop` keywords (`else`, `end`) is reached;
/// leaves `pos` on that action.
fn parse_nodes(toks: &[Tok], pos: &mut usize, stop: &[&str]) -> Result<Vec<Node>, String> {
    let mut nodes = Vec::new();
    while *pos < toks.len() {
        match &toks[*pos] {
            Tok::Text(t) => {
                nodes.push(Node::Text(t.clone()));
                *pos += 1;
            }
            Tok::Action(a) => {
                let word = a.split_whitespace().next().unwrap_or("");
                if stop.contains(&word) {
                    return Ok(nodes);
                }
                if a.starts_with("/*") {
                    *pos += 1;
                    continue;
                }
                *pos += 1;
                match word {
                    "if" => nodes.push(parse_if(&a[2..], toks, pos)?),
                    "range" => {
                        let over = parse_expr(strip_range_vars(&a[5..]))?;
                        let body = parse_nodes(toks, pos, &["else", "end"])?;
                        let otherwise = match action(toks, *pos) {
                            Some("else") => {
                                *pos += 1;
                                parse_nodes(toks, pos, &["end"])?
                            }
                            _ => Vec::new(),
                        };
                        expect_end(toks, pos)?;
                        nodes.push(Node::Range {
                            over,
                            body,
                            otherwise,
                        });
                    }
                    "else" | "end" => return Err(format!("unexpected {{{{ {a} }}}}")),
                    _ => nodes.push(Node::Print(parse_expr(a)?)),
                }
            }
        }
    }
    if stop.is_empty() {
        Ok(nodes)
    } else {
        Err("missing {{ end }}".to_owned())
    }
}

fn action(toks: &[Tok], pos: usize) -> Option<&str> {
    match toks.get(pos) {
        Some(Tok::Action(a)) => Some(a.as_str()),
        _ => None,
    }
}

fn expect_end(toks: &[Tok], pos: &mut usize) -> Result<(), String> {
    match action(toks, *pos) {
        Some("end") => {
            *pos += 1;
            Ok(())
        }
        _ => Err("missing {{ end }}".to_owned()),
    }
}

fn parse_if(cond: &str, toks: &[Tok], pos: &mut usize) -> Result<Node, String> {
    let mut branches = vec![(parse_expr(cond)?, parse_nodes(toks, pos, &["else", "end"])?)];
    let mut otherwise = Vec::new();
    while let Some(a) = action(toks, *pos) {
        if a == "end" {
            break;
        }
        // `else` or `else if ...`
        *pos += 1;
        let rest = a[4..].trim();
        if let Some(cond) = rest.strip_prefix("if") {
            branches.push((parse_expr(cond)?, parse_nodes(toks, pos, &["else", "end"])?));
        } else {
            otherwise = parse_nodes(toks, pos, &["end"])?;
            break;
        }
    }
    expect_end(toks, pos)?;
    Ok(Node::If {
        branches,
        otherwise,
    })
}

/// `range $i, $v := .X` -> `.X` (variables become `.`).
fn strip_range_vars(s: &str) -> &str {
    match s.find(":=") {
        Some(i) => &s[i + 2..],
        None => s,
    }
}

#[derive(Debug, Clone, PartialEq)]
enum ETok {
    Word(String),
    Str(String),
    Open,
    Close,
    Pipe,
}

fn lex_expr(s: &str) -> Result<Vec<ETok>, String> {
    let mut out = Vec::new();
    let mut chars = s.chars().peekable();
    while let Some(&c) = chars.peek() {
        match c {
            c if c.is_whitespace() => {
                chars.next();
            }
            '(' => {
                chars.next();
                out.push(ETok::Open);
            }
            ')' => {
                chars.next();
                out.push(ETok::Close);
            }
            '|' => {
                chars.next();
                out.push(ETok::Pipe);
            }
            '"' => {
                chars.next();
                let mut v = String::new();
                loop {
                    match chars.next() {
                        Some('\\') => match chars.next() {
                            Some('n') => v.push('\n'),
                            Some('t') => v.push('\t'),
                            Some(o) => v.push(o),
                            None => return Err("unterminated string".into()),
                        },
                        Some('"') => break,
                        Some(o) => v.push(o),
                        None => return Err("unterminated string".into()),
                    }
                }
                out.push(ETok::Str(v));
            }
            '`' => {
                chars.next();
                let mut v = String::new();
                loop {
                    match chars.next() {
                        Some('`') => break,
                        Some(o) => v.push(o),
                        None => return Err("unterminated raw string".into()),
                    }
                }
                out.push(ETok::Str(v));
            }
            _ => {
                let mut w = String::new();
                while let Some(&c) = chars.peek() {
                    if c.is_whitespace() || matches!(c, '(' | ')' | '|' | '"') {
                        break;
                    }
                    w.push(c);
                    chars.next();
                }
                out.push(ETok::Word(w));
            }
        }
    }
    Ok(out)
}

fn parse_expr(s: &str) -> Result<Expr, String> {
    let toks = lex_expr(s)?;
    let mut pos = 0;
    let e = parse_pipeline(&toks, &mut pos)?;
    if pos != toks.len() {
        return Err(format!("unexpected {:?}", toks[pos]));
    }
    Ok(e)
}

/// `cmd | fn args` passes the left side as the last argument.
fn parse_pipeline(toks: &[ETok], pos: &mut usize) -> Result<Expr, String> {
    let mut e = parse_command(toks, pos)?;
    while toks.get(*pos) == Some(&ETok::Pipe) {
        *pos += 1;
        match parse_command(toks, pos)? {
            Expr::Call(name, mut args) => {
                args.push(e);
                e = Expr::Call(name, args);
            }
            Expr::Field(f) if f.len() == 1 && !f[0].is_empty() => {
                e = Expr::Call(f[0].clone(), vec![e])
            }
            other => return Err(format!("cannot pipe into {other:?}")),
        }
    }
    Ok(e)
}

fn parse_command(toks: &[ETok], pos: &mut usize) -> Result<Expr, String> {
    let first = parse_term(toks, pos)?;
    let Some(ETok::Word(name)) = toks
        .get(*pos - 1)
        .filter(|_| matches!(first, Expr::Call(_, ref a) if a.is_empty()))
    else {
        return Ok(first);
    };
    let name = name.clone();
    let mut args = Vec::new();
    while let Some(t) = toks.get(*pos) {
        if matches!(t, ETok::Close | ETok::Pipe) {
            break;
        }
        args.push(parse_term(toks, pos)?);
    }
    Ok(Expr::Call(name, args))
}

fn parse_term(toks: &[ETok], pos: &mut usize) -> Result<Expr, String> {
    let t = toks.get(*pos).ok_or("missing value")?.clone();
    *pos += 1;
    Ok(match t {
        ETok::Str(s) => Expr::Lit(Value::Str(s)),
        ETok::Open => {
            let e = parse_pipeline(toks, pos)?;
            if toks.get(*pos) != Some(&ETok::Close) {
                return Err("missing )".into());
            }
            *pos += 1;
            e
        }
        ETok::Close | ETok::Pipe => return Err("unexpected ) or |".into()),
        ETok::Word(w) => {
            if w == "." {
                Expr::Dot
            } else if let Some(path) = w.strip_prefix('.') {
                Expr::Field(path.split('.').map(str::to_owned).collect())
            } else if w.starts_with('$') {
                // Range variables and `$` are bound to `.`.
                match w.split_once('.') {
                    Some((_, path)) => Expr::Field(path.split('.').map(str::to_owned).collect()),
                    None => Expr::Dot,
                }
            } else if w == "true" || w == "false" {
                Expr::Lit(Value::Bool(w == "true"))
            } else if w == "nil" {
                Expr::Lit(Value::Null)
            } else if w.parse::<f64>().is_ok() {
                Expr::Lit(Value::Str(w))
            } else {
                // A function name; arguments follow in parse_command.
                Expr::Call(w, Vec::new())
            }
        }
    })
}

// ---- evaluation -----------------------------------------------------------

fn exec(nodes: &[Node], root: &Value, dot: &Value, out: &mut String) -> Result<(), String> {
    for n in nodes {
        match n {
            Node::Text(t) => out.push_str(t),
            Node::Print(e) => out.push_str(&eval(e, root, dot)?.render()),
            Node::If {
                branches,
                otherwise,
            } => {
                match branches
                    .iter()
                    .find_map(|(c, body)| match eval(c, root, dot) {
                        Ok(v) if v.truthy() => Some(Ok(body)),
                        Ok(_) => None,
                        Err(e) => Some(Err(e)),
                    }) {
                    Some(body) => exec(body?, root, dot, out)?,
                    None => exec(otherwise, root, dot, out)?,
                }
            }
            Node::Range {
                over,
                body,
                otherwise,
            } => {
                let items = match eval(over, root, dot)? {
                    Value::List(l) => l,
                    Value::Map(m) => {
                        let mut keys: Vec<_> = m.into_iter().collect();
                        keys.sort_by(|a, b| a.0.cmp(&b.0));
                        keys.into_iter().map(|(_, v)| v).collect()
                    }
                    Value::Null => Vec::new(),
                    other => vec![other],
                };
                if items.is_empty() {
                    exec(otherwise, root, dot, out)?;
                }
                for item in &items {
                    exec(body, root, item, out)?;
                }
            }
        }
    }
    Ok(())
}

fn eval(e: &Expr, root: &Value, dot: &Value) -> Result<Value, String> {
    Ok(match e {
        Expr::Dot => dot.clone(),
        Expr::Lit(v) => v.clone(),
        Expr::Field(path) => {
            // Inside range, `.Foo` still reads the item first, then the root.
            let mut v = dot.get(&path[0]);
            if v == Value::Null {
                v = root.get(&path[0]);
            }
            for p in &path[1..] {
                v = v.get(p);
            }
            v
        }
        Expr::Call(name, args) => {
            let a = |i: usize| -> Result<Value, String> {
                args.get(i)
                    .map(|e| eval(e, root, dot))
                    .unwrap_or(Ok(Value::Null))
            };
            match name.as_str() {
                "eq" => {
                    let first = a(0)?;
                    let mut any = false;
                    for i in 1..args.len().max(2) {
                        if equal(&first, &a(i)?) {
                            any = true;
                        }
                    }
                    Value::Bool(any)
                }
                "ne" => Value::Bool(!equal(&a(0)?, &a(1)?)),
                "not" => Value::Bool(!a(0)?.truthy()),
                // Go returns the deciding argument, not a bool.
                "and" => {
                    let mut last = Value::Bool(true);
                    for i in 0..args.len() {
                        last = a(i)?;
                        if !last.truthy() {
                            break;
                        }
                    }
                    last
                }
                "or" => {
                    let mut last = Value::Bool(false);
                    for i in 0..args.len() {
                        last = a(i)?;
                        if last.truthy() {
                            break;
                        }
                    }
                    last
                }
                "join" => {
                    let sep = a(1)?.render();
                    match a(0)? {
                        Value::List(l) => {
                            Value::Str(l.iter().map(Value::render).collect::<Vec<_>>().join(&sep))
                        }
                        other => Value::Str(other.render()),
                    }
                }
                "re_replace" => {
                    let re = cached_regex(&a(1)?.render())?;
                    Value::Str(re.replace_all(&a(0)?.render(), &a(2)?.render()))
                }
                "len" => Value::Str(
                    match a(0)? {
                        Value::List(l) => l.len(),
                        Value::Str(s) => s.chars().count(),
                        Value::Map(m) => m.len(),
                        _ => 0,
                    }
                    .to_string(),
                ),
                "print" => Value::Str(
                    (0..args.len())
                        .map(|i| a(i).map(|v| v.render()))
                        .collect::<Result<Vec<_>, _>>()?
                        .concat(),
                ),
                other => return Err(format!("unknown function {other}")),
            }
        }
    })
}

fn equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Bool(x), Value::Str(y)) | (Value::Str(y), Value::Bool(x)) => x.to_string() == *y,
        (Value::Null, Value::Str(s)) | (Value::Str(s), Value::Null) => s.is_empty(),
        _ => a == b,
    }
}

/// A compiled pattern: the fast `regex` engine when it can, `fancy-regex`
/// for the lookarounds and backreferences .NET-style definitions use.
#[derive(Clone)]
pub enum Rx {
    Fast(Regex),
    Fancy(fancy_regex::Regex),
}

impl Rx {
    /// First capture group, or the whole match without groups.
    pub fn first_group(&self, text: &str) -> Option<String> {
        match self {
            Self::Fast(r) => r.captures(text).map(|c| {
                c.get(1)
                    .or_else(|| c.get(0))
                    .map_or("", |m| m.as_str())
                    .to_owned()
            }),
            Self::Fancy(r) => r.captures(text).ok().flatten().map(|c| {
                c.get(1)
                    .or_else(|| c.get(0))
                    .map_or("", |m| m.as_str())
                    .to_owned()
            }),
        }
    }

    /// `replacement` uses Go/.NET `$1` / `${name}` syntax.
    pub fn replace_all(&self, text: &str, replacement: &str) -> String {
        let rep = go_replacement(replacement);
        match self {
            Self::Fast(r) => r.replace_all(text, rep.as_str()).into_owned(),
            Self::Fancy(r) => r.replace_all(text, rep.as_str()).into_owned(),
        }
    }
}

/// Regex cache: definitions reuse the same few patterns for every row.
pub fn cached_regex(pattern: &str) -> Result<Rx, String> {
    static CACHE: LazyLock<Mutex<HashMap<String, Rx>>> = LazyLock::new(Default::default);
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(re) = cache.get(pattern) {
        return Ok(re.clone());
    }
    let translated = translate_regex(pattern);
    let re = match Regex::new(&translated) {
        Ok(r) => Rx::Fast(r),
        Err(fast_err) => fancy_regex::Regex::new(&translated)
            .map(Rx::Fancy)
            .map_err(|_| format!("bad regex {pattern:?}: {fast_err}"))?,
    };
    if cache.len() < 4096 {
        cache.insert(pattern.to_owned(), re.clone());
    }
    Ok(re)
}

/// .NET regex syntax the Rust engines spell differently: named groups
/// `(?<name>`, `[` and a `-` after a class shorthand inside character
/// classes, and `\p{IsBlock}` names.
fn translate_regex(p: &str) -> String {
    let mut out = String::with_capacity(p.len() + 8);
    let mut chars = p.chars().peekable();
    let mut in_class = false;
    let mut class_start = false;
    let mut prev_was_shorthand = false;
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                let Some(n) = chars.next() else {
                    out.push('\\');
                    break;
                };
                if (n == 'p' || n == 'P') && chars.peek() == Some(&'{') {
                    let mut name = String::new();
                    chars.next();
                    for x in chars.by_ref() {
                        if x == '}' {
                            break;
                        }
                        name.push(x);
                    }
                    let name = match name.strip_prefix("Is").unwrap_or(&name) {
                        "CJKUnifiedIdeographs" | "CJKUnifiedIdeographsExtensionA" => {
                            "Han".to_owned()
                        }
                        "BasicLatin" => "ASCII".to_owned(),
                        other => other.to_owned(),
                    };
                    out.push_str(&format!("\\{n}{{{name}}}"));
                    prev_was_shorthand = true;
                } else {
                    out.push('\\');
                    out.push(n);
                    prev_was_shorthand = in_class && "dDsSwW".contains(n);
                }
                class_start = false;
                continue;
            }
            '[' if in_class => {
                // .NET reads a bare `[` inside a class as a literal.
                if chars.peek() == Some(&':') {
                    out.push('[');
                } else {
                    out.push_str("\\[");
                }
            }
            '[' => {
                in_class = true;
                class_start = true;
                out.push('[');
                if chars.peek() == Some(&'^') {
                    out.push(chars.next().unwrap());
                }
                // A leading `]` is a literal in .NET.
                if chars.peek() == Some(&']') {
                    chars.next();
                    out.push_str("\\]");
                }
                prev_was_shorthand = false;
                continue;
            }
            ']' if in_class && !class_start => {
                in_class = false;
                out.push(']');
            }
            '-' if in_class && prev_was_shorthand => out.push_str("\\-"),
            '(' if !in_class && chars.peek() == Some(&'?') => {
                out.push('(');
                out.push(chars.next().unwrap());
                // `(?<name>` -> `(?P<name>`, but not lookbehind `(?<=` / `(?<!`.
                if chars.peek() == Some(&'<') {
                    let mut look = chars.clone();
                    look.next();
                    if !matches!(look.peek(), Some('=') | Some('!')) {
                        out.push('P');
                    }
                }
            }
            _ => out.push(c),
        }
        class_start = false;
        prev_was_shorthand = false;
    }
    out
}

/// Go/.NET use `$1` and `${name}`; the `regex` crate needs `${1}` when a
/// group number is followed by a letter or digit.
pub(crate) fn go_replacement(r: &str) -> String {
    static GROUP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\$(\d+)").unwrap());
    GROUP.replace_all(r, "$${${1}}").into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data() -> Value {
        Value::from_iter([
            ("Keywords", Value::from("The Bear S03")),
            (
                "Config",
                Value::from_iter([
                    ("sort", Value::from("seeders")),
                    ("freeleech", Value::Bool(false)),
                ]),
            ),
            (
                "Query",
                Value::from_iter([
                    ("Season", Value::from("3")),
                    ("Ep", Value::Null),
                    ("IMDBID", Value::from("tt5321")),
                ]),
            ),
            ("Categories", Value::List(vec!["1".into(), "7".into()])),
            ("True", Value::Bool(true)),
            ("False", Value::Bool(false)),
        ])
    }

    fn r(t: &str) -> String {
        render(t, &data()).unwrap()
    }

    #[test]
    fn fields_and_text() {
        assert_eq!(
            r("q={{ .Keywords }}&s={{ .Config.sort }}"),
            "q=The Bear S03&s=seeders"
        );
        assert_eq!(r("{{ .Query.Missing }}x"), "x");
        assert_eq!(r("plain"), "plain");
    }

    #[test]
    fn conditionals() {
        assert_eq!(
            r("{{ if .Query.Ep }}ep{{ else if .Query.Season }}season{{ else }}none{{ end }}"),
            "season"
        );
        assert_eq!(
            r(r#"{{ if eq .Config.sort "seeders" "leechers" }}yes{{ else }}no{{ end }}"#),
            "yes"
        );
        assert_eq!(
            r("{{ if .Config.freeleech }}fl{{ else }}all{{ end }}"),
            "all"
        );
        assert_eq!(
            r("{{ if and .Query.Season (not .Query.Ep) }}pack{{ end }}"),
            "pack"
        );
        assert_eq!(r("{{ or .Query.Ep .Query.Season }}"), "3");
        assert_eq!(r(r#"{{ if ne .Config.sort "added" }}x{{ end }}"#), "x");
    }

    #[test]
    fn range_and_join() {
        assert_eq!(
            r("{{ range .Categories }}c[]={{ . }}&{{ end }}"),
            "c[]=1&c[]=7&"
        );
        assert_eq!(r(r#"{{ join .Categories "," }}"#), "1,7");
        assert_eq!(r("{{ range .Nothing }}x{{ else }}empty{{ end }}"), "empty");
        assert_eq!(
            r("{{ range $i, $c := .Categories }}{{ $c }};{{ end }}"),
            "1;7;"
        );
    }

    #[test]
    fn re_replace_and_trim_markers() {
        assert_eq!(
            r(r#"{{ re_replace .Keywords "S(\\d+)" "Season $1x" }}"#),
            "The Bear Season 03x"
        );
        assert_eq!(r("a  {{- .Config.sort -}}  b"), "aseedersb");
        assert_eq!(r(r#"{{ .Config.sort | print "by " }}"#), "by seeders");
    }

    #[test]
    fn dotnet_regex_syntax() {
        for (pat, input, want) in [
            (
                r##"[\[!"#$%&'()*+,\-.\/:;<=>?@[\]^_`{|}~]"##,
                "a[b]c!",
                "a b c ",
            ),
            (r"(?i)T[\s-_]?(\d{1,2})\b", "t-3", "S3"),
            (r"([\p{IsCJKUnifiedIdeographs}\W]+)", "ab漢字cd", "ab cd"),
            (r"(?<y>\d{4})", "in 2026", "in y"),
            (r"(?<=a)b", "ab cb", "a  cb"),
        ] {
            let re = cached_regex(pat).unwrap_or_else(|e| panic!("{e}"));
            let repl = if pat.starts_with("(?i)T") {
                "S$1"
            } else if pat.contains("?<y>") {
                "y"
            } else {
                " "
            };
            assert_eq!(re.replace_all(input, repl), want, "{pat}");
        }
    }

    #[test]
    fn errors() {
        assert!(render("{{ if .X }}", &data()).is_err());
        assert!(render("{{ end }}", &data()).is_err());
        assert!(render("{{ nope .X }}", &data()).is_err());
    }
}
