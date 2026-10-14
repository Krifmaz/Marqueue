//! Prints what Playbill reads from each name.
//!
//! ```text
//! cargo run --example parse -- "The.Bear.S03E04.2160p.WEB-DL.DDP5.1.HDR.H.265-FLUX"
//! ```
//!
//! With no arguments, reads one name per line from stdin.

use std::io::{self, BufRead};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let names: Vec<String> = if args.is_empty() {
        io::stdin().lock().lines().map_while(Result::ok).collect()
    } else {
        args
    };

    for name in names.iter().filter(|n| !n.trim().is_empty()) {
        let r = playbill::parse(name);
        println!("{name}");
        let field = |k: &str, v: String| {
            if !v.is_empty() {
                println!("  {k:<12} {v}");
            }
        };
        field("kind", r.kind.to_string());
        field("title", r.title.clone().unwrap_or_default());
        field("year", r.year.map(|y| y.to_string()).unwrap_or_default());
        field("author", r.author.clone().unwrap_or_default());
        field("narrator", r.narrator.clone().unwrap_or_default());
        field("seasons", list(&r.seasons));
        field("episodes", list(&r.episodes));
        field("absolute", list(&r.absolute_episodes));
        field(
            "air date",
            r.air_date.map(|d| d.to_string()).unwrap_or_default(),
        );
        field("full season", flag(r.full_season));
        field("complete", flag(r.complete));
        field("resolution", opt(r.resolution));
        field("source", opt(r.source));
        field("codec", opt(r.codec));
        field(
            "bit depth",
            r.bit_depth.map(|b| b.to_string()).unwrap_or_default(),
        );
        field(
            "hdr",
            r.hdr
                .iter()
                .map(|h| h.to_string())
                .collect::<Vec<_>>()
                .join(", "),
        );
        field("service", r.streaming_service.clone().unwrap_or_default());
        field("audio", opt(r.audio));
        field("channels", r.audio_channels.clone().unwrap_or_default());
        field("atmos", flag(r.atmos));
        field("languages", r.languages.join(", "));
        field("multi", flag(r.multi_audio));
        field("dual audio", flag(r.dual_audio));
        field("edition", r.edition.clone().unwrap_or_default());
        field(
            "revision",
            if r.revision.version > 1 || r.revision.real > 0 {
                format!("v{} real {}", r.revision.version, r.revision.real)
            } else {
                String::new()
            },
        );
        field("group", r.release_group.clone().unwrap_or_default());
        field("hash", r.hash.clone().unwrap_or_default());
        field("format", opt(r.book_format));
        field(
            "abridged",
            r.abridged
                .map(|a| if a { "abridged" } else { "unabridged" }.to_string())
                .unwrap_or_default(),
        );
        field("retail", flag(r.retail));
        field(
            "bitrate",
            r.bitrate_kbps
                .map(|b| format!("{b} kbps"))
                .unwrap_or_default(),
        );
        field("container", r.container.clone().unwrap_or_default());
        println!();
    }
}

fn list(v: &[u16]) -> String {
    v.iter()
        .map(|n| n.to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

fn flag(b: bool) -> String {
    if b { "yes".into() } else { String::new() }
}

fn opt<T: std::fmt::Display>(v: Option<T>) -> String {
    v.map(|x| x.to_string()).unwrap_or_default()
}
