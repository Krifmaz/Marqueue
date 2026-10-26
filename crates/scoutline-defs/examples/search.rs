//! Runs one search through a definition, for trying definitions by hand.
//!
//! cargo run -p scoutline-defs --example search -- <definitions dir> <id> <query> [key=value ...]

use std::collections::BTreeMap;
use std::path::PathBuf;

use scoutline_core::settings::{IndexerSettings, SettingValue};
use scoutline_core::{Engine, SearchQuery};

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let dir = PathBuf::from(args.next().expect("definitions dir"));
    let id = args.next().expect("definition id");
    let query = args.next().unwrap_or_default();
    let settings: BTreeMap<String, SettingValue> = args
        .filter_map(|kv| {
            kv.split_once('=')
                .map(|(k, v)| (k.to_owned(), SettingValue::Text(v.to_owned())))
        })
        .collect();

    let s = IndexerSettings {
        id: id.clone(),
        definition: Some(id),
        settings,
        ..Default::default()
    };
    let indexers = scoutline_defs::build_indexers(&[s], &dir, &Default::default())
        .unwrap_or_else(|e| panic!("{e}"));
    let engine = Engine::builder().indexers(indexers).build().unwrap();
    let out = engine.search(&SearchQuery::general(query)).await;
    for (id, e) in &out.errors {
        eprintln!("error from {id}: {e}");
    }
    println!("{} results", out.results.len());
    for r in out.results.iter().take(8) {
        let o = r.best_origin();
        println!(
            "{:>6} seeds {:>9} MiB  {:<70.70}  parsed: {:?} {:?} {:?}",
            o.seeders.unwrap_or(0),
            r.size.unwrap_or(0) / 1_048_576,
            r.title,
            r.release.title.as_deref().unwrap_or(""),
            r.release
                .resolution
                .map(|x| x.to_string())
                .unwrap_or_default(),
            r.categories,
        );
    }
}
