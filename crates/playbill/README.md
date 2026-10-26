# Playbill

Release-name parser for movies, TV, anime, ebooks and audiobooks, written in Rust. Part of [Marqueue](https://github.com/Krifmaz/Marqueue); shared by Marqueue and Scoutline.

It turns a name like this:

```
The.Bear.S03E04.2160p.WEB-DL.DDP5.1.HDR.H.265-FLUX
```

into structured data: title `The Bear`, season 3, episode 4, 2160p, WEB-DL, H.265, HDR, E-AC-3 5.1, group `FLUX`.

## Usage

```rust
use playbill::{parse, parse_with_hint, Hint, MediaKind};

let r = parse("[SubsPlease] Frieren - 28 (1080p) [A1B2C3D4].mkv");
assert_eq!(r.kind, MediaKind::Anime);
assert_eq!(r.title.as_deref(), Some("Frieren"));
assert_eq!(r.absolute_episodes, vec![28]);

// A hint settles names that are ambiguous on their own.
let r = parse_with_hint("One.Piece.1089.1080p.WEB.H264-GROUP", Hint::Anime);
assert_eq!(r.absolute_episodes, vec![1089]);
```

Enable the `serde` feature to serialize `Release`.

Try it on any name:

```
cargo run --example parse -- "Dune.Part.Two.2024.2160p.UHD.BluRay.REMUX.DV.HDR10.TrueHD.Atmos.7.1-FGT"
```

## What it reads

| Area | Fields |
| --- | --- |
| Identity | kind, title, year, author, narrator |
| Episodes | seasons, episodes, absolute episodes, air date, full season, complete, special |
| Video | resolution, source, codec, bit depth, HDR, streaming service |
| Audio | codec, channels, Atmos |
| Language | languages, multi audio, dual audio |
| Release | edition, revision (PROPER/REPACK/v2, REAL), group, CRC hash, container |
| Books | format, abridged, retail, bitrate |

The parser never fails: anything it can't read is left empty.

## Tests

Every case lives in `tests/corpus/*.toml`. Each lists a release name and only the fields it checks:

```toml
[[case]]
name = "Show.Name.S01E01-E03.1080p.WEB-DL.DD5.1.H.264-GROUP"
episodes = [1, 2, 3]
audio = "ac3"
```

When a name parses wrong, add it as a case first, then fix the parser. Run with `cargo test`.
