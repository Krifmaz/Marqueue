//! Checks every definition in a folder loads, and that its templates,
//! selectors and filters are ones Scoutline understands.
//!
//! cargo run -p scoutline-defs --example check -- path/to/definitions

use std::collections::BTreeMap;
use std::path::PathBuf;

use scoutline_defs::model::{Field, Filter};
use scoutline_defs::{Definition, extract, filters, template};

fn main() {
    let dir = PathBuf::from(
        std::env::args()
            .nth(1)
            .expect("usage: check <definitions dir>"),
    );
    let mut problems: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut ok = 0;
    let mut total = 0;
    let mut paths: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .collect();
    paths.sort();
    for path in paths {
        if path.extension().and_then(|e| e.to_str()) != Some("yml") {
            continue;
        }
        total += 1;
        let name = path.file_stem().unwrap().to_string_lossy().to_string();
        let text = std::fs::read_to_string(&path).unwrap();
        let def = match Definition::from_yaml(&text) {
            Ok(d) => d,
            Err(e) => {
                problems
                    .entry("load".into())
                    .or_default()
                    .push(format!("{name}: {}", first_line(&e)));
                continue;
            }
        };
        let mut errs = Vec::new();
        check(&def, &mut errs);
        if errs.is_empty() {
            ok += 1;
        }
        for (kind, e) in errs {
            problems
                .entry(kind)
                .or_default()
                .push(format!("{name}: {e}"));
        }
    }
    println!("{ok}/{total} definitions fully understood");
    for (kind, list) in &problems {
        println!("\n== {kind}: {} ==", list.len());
        for p in list.iter().take(12) {
            println!("  {p}");
        }
    }
}

fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or("").chars().take(160).collect()
}

fn vars() -> template::Value {
    use template::Value;
    Value::from_iter([
        ("Keywords", Value::from("test")),
        (
            "Query",
            Value::from_iter([
                ("Keywords", Value::from("test")),
                ("Season", Value::from("1")),
            ]),
        ),
        ("Config", Value::Map(Default::default())),
        ("Categories", Value::List(vec![Value::from("1")])),
        ("Today", Value::from_iter([("Year", Value::from("2026"))])),
        ("True", Value::Bool(true)),
        ("False", Value::Bool(false)),
        ("Result", Value::Map(Default::default())),
    ])
}

fn check(def: &Definition, errs: &mut Vec<(String, String)>) {
    let v = vars();
    let mut tpl = |t: &str| {
        if let Err(e) = template::render(t, &v) {
            errs.push(("template".into(), first_line(&e.message)));
        }
    };
    for p in def.search_paths() {
        tpl(&p.path);
        p.inputs.values().for_each(|i| tpl(i));
    }
    def.search.inputs.values().for_each(|i| tpl(i));
    if let Some(l) = &def.login {
        l.inputs.values().for_each(|i| tpl(i));
    }
    let mut fields: Vec<&Field> = def.search.fields.values().collect();
    if let Some(d) = &def.search.rows.dateheaders {
        fields.push(d);
    }
    for f in &fields {
        if let Some(t) = &f.text {
            tpl(t);
        }
    }

    let json = def
        .search_paths()
        .iter()
        .any(|p| p.response.as_ref().and_then(|r| r.kind.as_deref()) == Some("json"));
    let mut sel = |s: &str| {
        let s = template::render(s, &v).unwrap_or_else(|_| s.to_owned());
        if let Err(e) = extract::check_selector(&s) {
            errs.push(("selector".into(), first_line(&e)));
        }
    };
    if !json {
        sel(&def.search.rows.selector);
        for f in &fields {
            if let Some(s) = &f.selector {
                sel(s);
            }
            for k in f.case.keys().filter(|k| *k != "*") {
                sel(k);
            }
        }
    }
    let mut all_filters: Vec<&Filter> = fields.iter().flat_map(|f| f.filters.iter()).collect();
    all_filters.extend(&def.search.keywordsfilters);
    all_filters.extend(&def.search.rows.filters);
    all_filters.extend(&def.search.preprocessingfilters);
    for f in all_filters {
        if let Err(e) = filters::apply("x", std::slice::from_ref(f), chrono::Utc::now()) {
            if e.contains("unknown filter") || e.contains("bad regex") {
                errs.push(("filter".into(), first_line(&e)));
            }
        }
    }
    for (_, cat) in def.category_map() {
        if scoutline_defs::indexer::category_id(&cat).is_none() {
            errs.push(("category".into(), format!("unknown category {cat:?}")));
        }
    }
}
