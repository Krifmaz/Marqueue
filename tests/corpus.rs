//! Runs every case in `tests/corpus/*.toml`.
//!
//! Each case gives a release name and only the fields it wants checked;
//! anything left out isn't compared. An empty string means "expect none".
//! To add a case, append a `[[case]]` block to the right file.

use std::fs;
use std::path::Path;

use playbill::Release;
use serde::Deserialize;

#[derive(Deserialize)]
struct CorpusFile {
    case: Vec<Case>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct Case {
    name: String,
    hint: Option<String>,

    kind: Option<String>,
    title: Option<String>,
    year: Option<u16>,
    author: Option<String>,
    narrator: Option<String>,

    seasons: Option<Vec<u16>>,
    episodes: Option<Vec<u16>>,
    absolute: Option<Vec<u16>>,
    air_date: Option<String>,
    full_season: Option<bool>,
    complete: Option<bool>,
    special: Option<bool>,

    resolution: Option<String>,
    source: Option<String>,
    codec: Option<String>,
    bit_depth: Option<u8>,
    hdr: Option<Vec<String>>,
    service: Option<String>,

    audio: Option<String>,
    channels: Option<String>,
    atmos: Option<bool>,

    languages: Option<Vec<String>>,
    multi: Option<bool>,
    dual_audio: Option<bool>,

    edition: Option<String>,
    version: Option<u8>,
    real: Option<u8>,
    group: Option<String>,
    hash: Option<String>,
    container: Option<String>,

    format: Option<String>,
    abridged: Option<bool>,
    retail: Option<bool>,
    bitrate: Option<u16>,
}

/// Compares one optional expectation and records a readable failure.
fn check<T: std::fmt::Debug + PartialEq>(
    errs: &mut Vec<String>,
    field: &str,
    want: Option<T>,
    got: T,
) {
    if let Some(want) = want {
        if want != got {
            errs.push(format!("    {field}: want {want:?}, got {got:?}"));
        }
    }
}

/// Strings: "" in the corpus means the field should be empty.
fn text(v: Option<impl ToString>) -> String {
    v.map(|x| x.to_string()).unwrap_or_default()
}

fn compare(c: &Case, r: &Release) -> Vec<String> {
    let mut e = Vec::new();
    check(&mut e, "kind", c.kind.clone(), r.kind.to_string());
    check(&mut e, "title", c.title.clone(), text(r.title.as_ref()));
    check(&mut e, "year", c.year, r.year.unwrap_or(0));
    check(&mut e, "author", c.author.clone(), text(r.author.as_ref()));
    check(
        &mut e,
        "narrator",
        c.narrator.clone(),
        text(r.narrator.as_ref()),
    );

    check(&mut e, "seasons", c.seasons.clone(), r.seasons.clone());
    check(&mut e, "episodes", c.episodes.clone(), r.episodes.clone());
    check(
        &mut e,
        "absolute",
        c.absolute.clone(),
        r.absolute_episodes.clone(),
    );
    check(&mut e, "air_date", c.air_date.clone(), text(r.air_date));
    check(&mut e, "full_season", c.full_season, r.full_season);
    check(&mut e, "complete", c.complete, r.complete);
    check(&mut e, "special", c.special, r.special);

    check(
        &mut e,
        "resolution",
        c.resolution.clone(),
        text(r.resolution),
    );
    check(&mut e, "source", c.source.clone(), text(r.source));
    check(&mut e, "codec", c.codec.clone(), text(r.codec));
    check(&mut e, "bit_depth", c.bit_depth, r.bit_depth.unwrap_or(0));
    check(
        &mut e,
        "hdr",
        c.hdr.clone(),
        r.hdr.iter().map(|h| h.to_string()).collect::<Vec<_>>(),
    );
    check(
        &mut e,
        "service",
        c.service.clone(),
        text(r.streaming_service.as_ref()),
    );

    check(&mut e, "audio", c.audio.clone(), text(r.audio));
    check(
        &mut e,
        "channels",
        c.channels.clone(),
        text(r.audio_channels.as_ref()),
    );
    check(&mut e, "atmos", c.atmos, r.atmos);

    check(
        &mut e,
        "languages",
        c.languages.clone(),
        r.languages.clone(),
    );
    check(&mut e, "multi", c.multi, r.multi_audio);
    check(&mut e, "dual_audio", c.dual_audio, r.dual_audio);

    check(
        &mut e,
        "edition",
        c.edition.clone(),
        text(r.edition.as_ref()),
    );
    check(&mut e, "version", c.version, r.revision.version);
    check(&mut e, "real", c.real, r.revision.real);
    check(
        &mut e,
        "group",
        c.group.clone(),
        text(r.release_group.as_ref()),
    );
    check(&mut e, "hash", c.hash.clone(), text(r.hash.as_ref()));
    check(
        &mut e,
        "container",
        c.container.clone(),
        text(r.container.as_ref()),
    );

    check(&mut e, "format", c.format.clone(), text(r.book_format));
    check(&mut e, "abridged", c.abridged.map(Some), r.abridged);
    check(&mut e, "retail", c.retail, r.retail);
    check(&mut e, "bitrate", c.bitrate, r.bitrate_kbps.unwrap_or(0));
    e
}

fn hint(name: Option<&str>) -> playbill::Hint {
    use playbill::Hint;
    match name {
        None | Some("auto") => Hint::Auto,
        Some("movie") => Hint::Movie,
        Some("series") => Hint::Series,
        Some("anime") => Hint::Anime,
        Some("book") => Hint::Book,
        Some(other) => panic!("unknown hint {other:?}"),
    }
}

#[test]
fn corpus() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus");
    let mut files: Vec<_> = fs::read_dir(&dir)
        .expect("tests/corpus exists")
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "toml"))
        .collect();
    files.sort();

    let (mut total, mut failures) = (0, Vec::new());
    for path in &files {
        let raw = fs::read_to_string(path).unwrap();
        let file: CorpusFile =
            toml::from_str(&raw).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        for c in &file.case {
            total += 1;
            let r = playbill::parse_with_hint(&c.name, hint(c.hint.as_deref()));
            let errs = compare(c, &r);
            if !errs.is_empty() {
                let file_name = path.file_name().unwrap().to_string_lossy();
                failures.push(format!("[{file_name}] {}\n{}", c.name, errs.join("\n")));
            }
        }
    }

    assert!(total > 0, "corpus is empty");
    if !failures.is_empty() {
        panic!(
            "{} of {total} corpus cases failed:\n\n{}",
            failures.len(),
            failures.join("\n\n")
        );
    }
    println!("{total} corpus cases passed");
}
