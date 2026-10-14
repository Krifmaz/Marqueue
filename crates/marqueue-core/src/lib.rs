//! Marqueue's shared media model, quality profiles and release scoring.
//!
//! Release names are read by [`playbill`]; this crate decides what a parsed
//! release is worth for a given title.

pub mod media;
pub mod profile;

pub use media::MediaType;
pub use profile::{QualityProfile, Rejection, score};
