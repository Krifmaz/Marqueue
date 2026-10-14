//! The five media types Marqueue tracks.

use playbill::MediaKind;
use serde::{Deserialize, Serialize};

/// A media type a user can request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MediaType {
    Movie,
    Tv,
    Anime,
    Ebook,
    Audiobook,
}

impl MediaType {
    pub const ALL: [MediaType; 5] = [
        MediaType::Movie,
        MediaType::Tv,
        MediaType::Anime,
        MediaType::Ebook,
        MediaType::Audiobook,
    ];

    /// The parser hint to use when searching for this type.
    pub fn hint(self) -> playbill::Hint {
        match self {
            MediaType::Movie => playbill::Hint::Movie,
            MediaType::Tv => playbill::Hint::Series,
            MediaType::Anime => playbill::Hint::Anime,
            MediaType::Ebook | MediaType::Audiobook => playbill::Hint::Book,
        }
    }

    /// True if a parsed release of `kind` can belong to this type.
    pub fn accepts(self, kind: MediaKind) -> bool {
        matches!(
            (self, kind),
            (_, MediaKind::Unknown)
                | (MediaType::Movie, MediaKind::Movie)
                | (MediaType::Tv, MediaKind::Episode)
                | (MediaType::Anime, MediaKind::Anime | MediaKind::Episode)
                | (MediaType::Ebook, MediaKind::Ebook)
                | (MediaType::Audiobook, MediaKind::Audiobook)
        )
    }

    pub fn is_video(self) -> bool {
        matches!(self, MediaType::Movie | MediaType::Tv | MediaType::Anime)
    }
}
