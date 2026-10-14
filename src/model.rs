//! Types returned by the parser.

use std::fmt;

/// Defines a fieldless enum with a stable string form, used for display,
/// tests and (with the `serde` feature) serialization.
macro_rules! str_enum {
    ($(#[$meta:meta])* $name:ident { $($(#[$vmeta:meta])* $variant:ident => $text:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        #[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
        pub enum $name {
            $(
                $(#[$vmeta])*
                #[cfg_attr(feature = "serde", serde(rename = $text))]
                $variant
            ),+
        }

        impl $name {
            /// Stable lowercase name, e.g. `"web-dl"`.
            pub fn as_str(self) -> &'static str {
                match self { $(Self::$variant => $text),+ }
            }

            /// Parses the stable name back into the enum.
            pub fn from_name(name: &str) -> Option<Self> {
                match name { $($text => Some(Self::$variant),)+ _ => None }
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }
    };
}

str_enum! {
    /// What kind of media a release looks like.
    MediaKind {
        Unknown => "unknown",
        Movie => "movie",
        Episode => "episode",
        Anime => "anime",
        Ebook => "ebook",
        Audiobook => "audiobook",
    }
}

// The enum comes from a macro, so `#[derive(Default)]` isn't available.
#[allow(clippy::derivable_impls)]
impl Default for MediaKind {
    fn default() -> Self {
        Self::Unknown
    }
}

str_enum! {
    /// Vertical video resolution.
    Resolution {
        R480p => "480p",
        R576p => "576p",
        R720p => "720p",
        R1080p => "1080p",
        R2160p => "2160p",
        R4320p => "4320p",
    }
}

str_enum! {
    /// Where the video came from, roughly worst to best.
    Source {
        Cam => "cam",
        Telesync => "telesync",
        Telecine => "telecine",
        Screener => "screener",
        Dvd => "dvd",
        Sdtv => "sdtv",
        Pdtv => "pdtv",
        Hdtv => "hdtv",
        HdRip => "hdrip",
        /// Plain "WEB": could be WEB-DL or WEBRip.
        Web => "web",
        WebRip => "webrip",
        WebDl => "web-dl",
        BdRip => "bdrip",
        BluRay => "bluray",
        Remux => "remux",
    }
}

str_enum! {
    /// Video codec.
    VideoCodec {
        Mpeg2 => "mpeg2",
        Xvid => "xvid",
        Divx => "divx",
        Vc1 => "vc1",
        H264 => "h264",
        H265 => "h265",
        Vp9 => "vp9",
        Av1 => "av1",
    }
}

str_enum! {
    /// High dynamic range formats.
    Hdr {
        /// Plain "HDR" with no specific format.
        Hdr => "hdr",
        Hdr10 => "hdr10",
        Hdr10Plus => "hdr10+",
        DolbyVision => "dv",
        Hlg => "hlg",
    }
}

str_enum! {
    /// Audio codec, lossy to lossless.
    AudioCodec {
        Mp3 => "mp3",
        Aac => "aac",
        Opus => "opus",
        Ac3 => "ac3",
        Eac3 => "eac3",
        Dts => "dts",
        DtsHd => "dts-hd",
        DtsHdMa => "dts-hd-ma",
        DtsX => "dts-x",
        TrueHd => "truehd",
        Flac => "flac",
        Pcm => "pcm",
    }
}

str_enum! {
    /// Ebook and audiobook file formats.
    BookFormat {
        Epub => "epub",
        Mobi => "mobi",
        Azw3 => "azw3",
        Pdf => "pdf",
        M4b => "m4b",
        M4a => "m4a",
        Mp3 => "mp3",
        Flac => "flac",
    }
}

/// An air date for daily shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Date {
    pub year: u16,
    pub month: u8,
    pub day: u8,
}

impl fmt::Display for Date {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }
}

/// Re-release information. `version` starts at 1 and goes up with each
/// PROPER/REPACK (or an anime `v2`, `v3`); `real` counts REAL tags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Revision {
    pub version: u8,
    pub real: u8,
}

impl Default for Revision {
    fn default() -> Self {
        Self {
            version: 1,
            real: 0,
        }
    }
}

/// Everything the parser could read from one release name.
///
/// Fields that weren't found are `None`, empty or `false`.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Release {
    /// The name exactly as given.
    pub raw: String,
    pub kind: MediaKind,
    /// Clean title, e.g. `"The Bear"`. For books this is the book title.
    pub title: Option<String>,
    pub year: Option<u16>,

    // Episodes
    pub seasons: Vec<u16>,
    pub episodes: Vec<u16>,
    /// Anime absolute episode numbers.
    pub absolute_episodes: Vec<u16>,
    /// A whole season (or several) with no episode numbers.
    pub full_season: bool,
    /// Tagged COMPLETE.
    pub complete: bool,
    /// Season 0.
    pub special: bool,
    pub air_date: Option<Date>,

    // Video
    pub resolution: Option<Resolution>,
    pub source: Option<Source>,
    pub codec: Option<VideoCodec>,
    pub bit_depth: Option<u8>,
    pub hdr: Vec<Hdr>,
    /// Streaming service code as tagged, e.g. `"AMZN"`, `"NF"`.
    pub streaming_service: Option<String>,

    // Audio
    pub audio: Option<AudioCodec>,
    /// e.g. `"5.1"`.
    pub audio_channels: Option<String>,
    pub atmos: bool,

    // Language
    /// Language codes, e.g. `["fr"]`.
    pub languages: Vec<String>,
    pub multi_audio: bool,
    pub dual_audio: bool,

    // Release
    /// Normalized edition, e.g. `"Director's Cut"`.
    pub edition: Option<String>,
    pub revision: Revision,
    pub release_group: Option<String>,
    /// Anime CRC32 checksum, uppercase.
    pub hash: Option<String>,
    /// File extension, if the name had a known one.
    pub container: Option<String>,

    // Books
    pub author: Option<String>,
    pub narrator: Option<String>,
    pub book_format: Option<BookFormat>,
    pub abridged: Option<bool>,
    pub retail: bool,
    pub bitrate_kbps: Option<u16>,
}

impl Release {
    /// True if this is a TV or anime episode or season pack.
    pub fn is_episodic(&self) -> bool {
        matches!(self.kind, MediaKind::Episode | MediaKind::Anime)
            && (!self.seasons.is_empty()
                || !self.episodes.is_empty()
                || !self.absolute_episodes.is_empty()
                || self.air_date.is_some())
    }

    /// True for ebooks and audiobooks.
    pub fn is_book(&self) -> bool {
        matches!(self.kind, MediaKind::Ebook | MediaKind::Audiobook)
    }
}
