//! # Playbill
//!
//! Reads release and file names for movies, TV, anime, ebooks and
//! audiobooks and turns them into structured data.
//!
//! ```
//! use playbill::{parse, MediaKind, Resolution, Source};
//!
//! let r = parse("Dune.Part.Two.2024.2160p.UHD.BluRay.REMUX.DV.HDR10.TrueHD.Atmos.7.1-FGT");
//! assert_eq!(r.kind, MediaKind::Movie);
//! assert_eq!(r.title.as_deref(), Some("Dune Part Two"));
//! assert_eq!(r.year, Some(2024));
//! assert_eq!(r.resolution, Some(Resolution::R2160p));
//! assert_eq!(r.source, Some(Source::Remux));
//! ```
//!
//! The parser never fails: anything it can't read is left empty. Use
//! [`parse_with_hint`] when the caller already knows the media type.

mod model;
mod parse;
mod rx;

pub use model::*;
pub use parse::{Hint, parse, parse_with_hint};
