//! Quality profiles and release scoring.
//!
//! A profile lists allowed resolutions and sources in order of preference,
//! a cutoff where upgrades stop, size limits and preferred groups. [`score`]
//! turns a parsed release into one number, or a reason it was rejected.

use playbill::{Release, Resolution, Source};
use serde::{Deserialize, Serialize};

use crate::MediaType;

/// Why a release can't be grabbed under a profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rejection {
    WrongMediaType,
    ResolutionNotAllowed,
    SourceNotAllowed,
    TooSmall,
    TooLarge,
    BlockedGroup,
}

/// A quality profile. Lists are ordered worst to best.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QualityProfile {
    pub name: String,
    pub media_type: MediaType,
    /// Allowed resolutions, worst to best. Empty allows any.
    pub resolutions: Vec<Resolution>,
    /// Allowed sources, worst to best. Empty allows any.
    pub sources: Vec<Source>,
    /// Stop upgrading once a file reaches this resolution.
    pub cutoff: Option<Resolution>,
    /// Size limits per release in MiB.
    pub min_size_mib: Option<u64>,
    pub max_size_mib: Option<u64>,
    pub preferred_groups: Vec<String>,
    pub blocked_groups: Vec<String>,
}

impl QualityProfile {
    fn new(name: &str, media_type: MediaType) -> Self {
        Self {
            name: name.to_string(),
            media_type,
            resolutions: Vec::new(),
            sources: Vec::new(),
            cutoff: None,
            min_size_mib: None,
            max_size_mib: None,
            preferred_groups: Vec::new(),
            blocked_groups: Vec::new(),
        }
    }

    /// Built-in presets: 1080p, 4K, anime and space-saver.
    pub fn presets() -> Vec<QualityProfile> {
        use Resolution::*;
        use Source::*;
        let web_and_disc = vec![Hdtv, Web, WebRip, WebDl, BdRip, BluRay];

        let mut hd = Self::new("HD 1080p", MediaType::Movie);
        hd.resolutions = vec![R720p, R1080p];
        hd.sources = web_and_disc.clone();
        hd.cutoff = Some(R1080p);
        hd.max_size_mib = Some(20_000);

        let mut uhd = Self::new("4K", MediaType::Movie);
        uhd.resolutions = vec![R1080p, R2160p];
        uhd.sources = [web_and_disc.clone(), vec![Remux]].concat();
        uhd.cutoff = Some(R2160p);

        let mut anime = Self::new("Anime", MediaType::Anime);
        anime.resolutions = vec![R720p, R1080p];
        anime.sources = web_and_disc.clone();
        anime.cutoff = Some(R1080p);

        let mut saver = Self::new("Space saver", MediaType::Movie);
        saver.resolutions = vec![R720p, R1080p];
        saver.sources = web_and_disc;
        saver.cutoff = Some(R720p);
        saver.max_size_mib = Some(4_000);

        vec![hd, uhd, anime, saver]
    }

    /// True if a file at `resolution` has met the cutoff, so no more upgrades.
    pub fn cutoff_met(&self, resolution: Option<Resolution>) -> bool {
        match (self.cutoff, resolution) {
            (None, _) => true,
            (Some(cut), Some(r)) => r >= cut,
            (Some(_), None) => false,
        }
    }
}

/// Scores a release for a profile. Higher is better.
///
/// `size_mib` is the release size if the indexer reported one.
pub fn score(
    release: &Release,
    profile: &QualityProfile,
    size_mib: Option<u64>,
) -> Result<i64, Rejection> {
    if !profile.media_type.accepts(release.kind) {
        return Err(Rejection::WrongMediaType);
    }

    let group = release.release_group.as_deref().unwrap_or_default();
    if profile
        .blocked_groups
        .iter()
        .any(|g| g.eq_ignore_ascii_case(group))
    {
        return Err(Rejection::BlockedGroup);
    }

    if let Some(size) = size_mib {
        if profile.min_size_mib.is_some_and(|min| size < min) {
            return Err(Rejection::TooSmall);
        }
        if profile.max_size_mib.is_some_and(|max| size > max) {
            return Err(Rejection::TooLarge);
        }
    }

    let mut total = 0i64;

    if profile.media_type.is_video() {
        total += rank(&profile.resolutions, release.resolution)
            .ok_or(Rejection::ResolutionNotAllowed)?
            * 1000;
        total += rank(&profile.sources, release.source).ok_or(Rejection::SourceNotAllowed)? * 100;
    }

    if profile
        .preferred_groups
        .iter()
        .any(|g| g.eq_ignore_ascii_case(group))
    {
        total += 50;
    }
    // PROPER/REPACK beats the original release.
    total += i64::from(release.revision.version.saturating_sub(1)) * 10;
    total += i64::from(release.revision.real) * 5;

    Ok(total)
}

/// Position in a preference list (1-based), 0 if the list allows anything
/// and the value is unknown, or `None` if the value isn't allowed.
fn rank<T: PartialEq + Copy>(allowed: &[T], value: Option<T>) -> Option<i64> {
    if allowed.is_empty() {
        return Some(0);
    }
    let value = value?;
    allowed
        .iter()
        .position(|&a| a == value)
        .map(|i| i as i64 + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn preset(name: &str) -> QualityProfile {
        QualityProfile::presets()
            .into_iter()
            .find(|p| p.name == name)
            .unwrap()
    }

    #[test]
    fn prefers_higher_resolution_and_better_source() {
        let p = preset("4K");
        let uhd = playbill::parse(
            "Dune.Part.Two.2024.2160p.UHD.BluRay.REMUX.DV.HDR10.TrueHD.Atmos.7.1-FGT",
        );
        let hd = playbill::parse("Dune.Part.Two.2024.1080p.WEB-DL.DDP5.1.H.264-FLUX");
        assert!(score(&uhd, &p, None).unwrap() > score(&hd, &p, None).unwrap());
    }

    #[test]
    fn rejects_disallowed_resolution() {
        let p = preset("HD 1080p");
        let r = playbill::parse("Dune.Part.Two.2024.2160p.WEB-DL.DDP5.1.H.265-FLUX");
        assert_eq!(score(&r, &p, None), Err(Rejection::ResolutionNotAllowed));
    }

    #[test]
    fn rejects_wrong_media_type() {
        let p = preset("HD 1080p");
        let r = playbill::parse("The.Bear.S03E04.1080p.WEB-DL.DDP5.1.H.264-FLUX");
        assert_eq!(score(&r, &p, None), Err(Rejection::WrongMediaType));
    }

    #[test]
    fn size_limits() {
        let p = preset("Space saver");
        let r = playbill::parse("Dune.Part.Two.2024.1080p.WEB-DL.DDP5.1.H.264-FLUX");
        assert_eq!(score(&r, &p, Some(9_000)), Err(Rejection::TooLarge));
        assert!(score(&r, &p, Some(3_000)).is_ok());
    }

    #[test]
    fn repack_beats_original() {
        let p = preset("HD 1080p");
        let orig = playbill::parse("Dune.Part.Two.2024.1080p.WEB-DL.DDP5.1.H.264-FLUX");
        let repack = playbill::parse("Dune.Part.Two.2024.REPACK.1080p.WEB-DL.DDP5.1.H.264-FLUX");
        assert!(score(&repack, &p, None).unwrap() > score(&orig, &p, None).unwrap());
    }

    #[test]
    fn cutoff() {
        let p = preset("HD 1080p");
        assert!(!p.cutoff_met(Some(Resolution::R720p)));
        assert!(p.cutoff_met(Some(Resolution::R1080p)));
    }
}
