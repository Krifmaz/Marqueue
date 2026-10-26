//! The parser itself.
//!
//! Order of work:
//! 1. Clean the name: path, extension, leading `[Group]`, CRC, site tags.
//! 2. Take the scene release group from a trailing `-GROUP`.
//! 3. Find where the title ends: the first episode marker, the release year
//!    or a strong quality token, whichever comes first.
//! 4. Read video, audio, language and release info from what follows.
//! 5. Clean the title and split author/title for books.

use std::sync::LazyLock;

use regex::Regex;

use crate::model::*;
use crate::rx::*;

/// What the caller already knows about the release. `Auto` lets the parser
/// decide; the others resolve ambiguous names (e.g. a bare anime episode
/// number like `One.Piece.1089`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Hint {
    #[default]
    Auto,
    Movie,
    Series,
    Anime,
    Book,
}

/// Parses a release or file name with no hint.
///
/// ```
/// let r = playbill::parse("The.Bear.S03E04.2160p.WEB-DL.DDP5.1.HDR.H.265-FLUX");
/// assert_eq!(r.title.as_deref(), Some("The Bear"));
/// assert_eq!(r.seasons, vec![3]);
/// assert_eq!(r.episodes, vec![4]);
/// assert_eq!(r.release_group.as_deref(), Some("FLUX"));
/// ```
pub fn parse(name: &str) -> Release {
    parse_with_hint(name, Hint::Auto)
}

const VIDEO_EXT: &[&str] = &[
    "mkv", "mp4", "avi", "m4v", "ts", "wmv", "mov", "webm", "mpg", "mpeg", "m2ts", "iso",
];
const EBOOK_EXT: &[&str] = &["epub", "mobi", "azw3", "azw", "pdf"];
const AUDIO_EXT: &[&str] = &["m4b", "mp3", "m4a", "flac", "aac", "ogg", "opus"];
const OTHER_EXT: &[&str] = &["nzb", "torrent", "srt", "nfo"];

/// Words after a trailing `-` that are part of a quality tag, not a group.
const NOT_GROUPS: &[&str] = &[
    "dl", "rip", "hd", "ma", "x", "ray", "audio", "cut", "es", "sub", "subs", "dub", "dubbed",
    "lang", "bit", "bits", "dts", "ac3", "aac", "web", "hdr", "dv", "hevc", "avc",
];

/// Parses a release or file name, using `hint` to settle ambiguous cases.
pub fn parse_with_hint(name: &str, hint: Hint) -> Release {
    let mut r = Release {
        raw: name.to_string(),
        ..Default::default()
    };

    // ---- 1. Clean ---------------------------------------------------------
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name).trim();
    let mut s = base.to_string();

    if let Some(dot) = s.rfind('.') {
        let ext = s[dot + 1..].to_ascii_lowercase();
        let known = [VIDEO_EXT, EBOOK_EXT, AUDIO_EXT, OTHER_EXT]
            .iter()
            .any(|list| list.contains(&ext.as_str()));
        if known {
            r.container = Some(ext);
            s.truncate(dot);
        }
    }
    let s_spaced = s.replace('_', " ");
    s = s_spaced;

    let mut anime_group = None;
    if let Some(c) = LEADING_GROUP.captures(&s) {
        let g = c[1].trim().to_string();
        let end = c.get(0).unwrap().end();
        if !g.is_empty() && !RES_P.is_match(&g) {
            anime_group = Some(g);
        }
        s = s[end..].to_string();
    }

    if let Some(c) = CRC.captures(&s) {
        r.hash = Some(c[1].to_ascii_uppercase());
        let range = c.get(0).unwrap().range();
        s.replace_range(range, " ");
    }

    let mut site_group = None;
    while let Some(c) = SITE_TAG.captures(&s) {
        let tag = c[1].to_ascii_lowercase();
        if tag.starts_with("yts") || tag == "yify" {
            site_group = Some("YTS".to_string());
        }
        let range = c.get(0).unwrap().range();
        s.replace_range(range, " ");
    }
    let mut s = s.trim().to_string();

    // ---- 2. Scene group ---------------------------------------------------
    let scene_group = take_scene_group(&mut s);
    r.release_group = scene_group.or_else(|| anime_group.clone()).or(site_group);

    // ---- 3. Title boundary ------------------------------------------------
    let book_signal = hint == Hint::Book
        || BOOK_SIGNAL.is_match(&s)
        || r.container
            .as_deref()
            .is_some_and(|e| EBOOK_EXT.contains(&e) || e == "m4b");
    let allow_anime = anime_group.is_some() || hint == Hint::Anime;

    let episode = if book_signal && hint != Hint::Series && hint != Hint::Anime {
        None
    } else {
        find_episode(&s, allow_anime, hint == Hint::Anime)
    };

    let strong = QUALITY_STRONG.find(&s).map(|m| m.start());
    let book_mark = if book_signal {
        BOOK_MARK.find(&s).map(|m| m.start())
    } else {
        None
    };
    let quality_start = min_opt(strong, book_mark);

    // The release year is the last year-like number before quality info, as
    // long as it isn't the very start of the name ("1917.2019.1080p").
    let mut year_start = None;
    if episode.is_none() {
        let limit = quality_start.unwrap_or(s.len());
        for m in YEAR.find_iter(&s) {
            if m.start() >= limit {
                break;
            }
            if has_title_before(&s, m.start()) {
                year_start = Some(m.start());
                r.year = m.as_str().parse().ok();
            }
        }
    }

    let ep_start = episode.as_ref().map(|e| e.start);
    let title_end = [ep_start, year_start, quality_start]
        .into_iter()
        .flatten()
        .min()
        .unwrap_or(s.len());

    // ---- 4. Details -------------------------------------------------------
    let tail_from = episode
        .as_ref()
        .map(|e| e.end.max(title_end))
        .unwrap_or(title_end);
    let tail = &s[tail_from..];
    let after_title = &s[title_end..];

    if let Some(e) = &episode {
        r.seasons = e.seasons.clone();
        r.episodes = e.episodes.clone();
        r.absolute_episodes = e.absolute.clone();
        r.full_season = e.full_season;
        r.air_date = e.air_date;
        if let Some(v) = e.version {
            r.revision.version = r.revision.version.max(v);
        }
    }
    r.special = r.seasons.first() == Some(&0) && r.seasons.len() == 1;

    read_video(&mut r, after_title);
    read_audio(&mut r, tail);
    read_language(&mut r, after_title);
    read_release_info(&mut r, after_title);
    if book_signal {
        read_book(&mut r, &s);
    }

    // ---- 5. Title ---------------------------------------------------------
    let mut title = clean_title(&s[..title_end]);

    // A year at the end of a TV title: "Doctor Who 2005". Movies already
    // have their year, and a number left in their title belongs to it
    // ("Blade Runner 2049").
    if r.year.is_none() {
        if let Some(c) = title.as_deref().and_then(|t| TITLE_YEAR.captures(t)) {
            r.year = c[2].parse().ok();
            title = Some(c[1].trim().to_string());
        }
    }

    // An edition at the end of a movie title: "Aliens Special Edition 1986".
    if let Some(t) = &title {
        if let Some((rest, edition)) = strip_trailing_edition(t) {
            if r.edition.is_none() {
                r.edition = Some(edition);
            }
            title = Some(rest);
        }
    }

    // ---- Kind -------------------------------------------------------------
    let has_episode = episode.is_some();
    r.kind = match hint {
        Hint::Movie => MediaKind::Movie,
        Hint::Series => MediaKind::Episode,
        Hint::Anime => MediaKind::Anime,
        Hint::Book => book_kind(&r),
        Hint::Auto => {
            if book_signal && !has_episode {
                book_kind(&r)
            } else if has_episode && (anime_group.is_some() || !r.absolute_episodes.is_empty()) {
                MediaKind::Anime
            } else if has_episode {
                MediaKind::Episode
            } else if r.year.is_some()
                || r.resolution.is_some()
                || r.source.is_some()
                || r.codec.is_some()
            {
                MediaKind::Movie
            } else {
                MediaKind::Unknown
            }
        }
    };

    // Anime often puts the season in the title: "Spy x Family S2 - 05".
    if r.kind == MediaKind::Anime && r.seasons.is_empty() {
        if let Some(t) = &title {
            if let Some(c) = ANIME_TITLE_SEASON.captures(t) {
                let n = c
                    .get(2)
                    .or(c.get(3))
                    .or(c.get(4))
                    .and_then(|m| m.as_str().parse().ok());
                if let Some(n) = n {
                    r.seasons = vec![n];
                    title = Some(c[1].trim().to_string());
                }
            }
        }
    }

    if r.is_book() {
        if let Some(t) = &title {
            let (author, book_title) = split_author(t);
            r.author = author;
            title = Some(book_title);
        }
    }

    r.title = title.filter(|t| !t.is_empty());
    r
}

// ---------------------------------------------------------------------------
// Episodes
// ---------------------------------------------------------------------------

struct EpisodeMatch {
    start: usize,
    end: usize,
    seasons: Vec<u16>,
    episodes: Vec<u16>,
    absolute: Vec<u16>,
    full_season: bool,
    air_date: Option<Date>,
    version: Option<u8>,
}

impl EpisodeMatch {
    fn new(start: usize, end: usize) -> Self {
        Self {
            start,
            end,
            seasons: Vec::new(),
            episodes: Vec::new(),
            absolute: Vec::new(),
            full_season: false,
            air_date: None,
            version: None,
        }
    }
}

fn find_episode(s: &str, allow_anime: bool, loose_anime: bool) -> Option<EpisodeMatch> {
    // S01E01 and friends.
    if let Some(c) = SXE.captures(s) {
        let m = c.get(0).unwrap();
        let mut e = EpisodeMatch::new(m.start(), m.end());
        e.seasons = vec![num(&c[1])];
        let mut eps = vec![num(&c[2])];
        for t in EP_TAIL.captures_iter(&c[3]) {
            push_episode(&mut eps, num(&t[2]), t.get(1).is_some());
        }
        e.episodes = eps;
        return Some(e);
    }

    // 1x05
    if let Some(c) = NXNN.captures(s) {
        let m = c.get(0).unwrap();
        let mut e = EpisodeMatch::new(m.start(), m.end());
        e.seasons = vec![num(&c[1])];
        let mut eps = vec![num(&c[2])];
        if let Some(last) = c.get(3) {
            push_episode(&mut eps, num(last.as_str()), true);
        }
        e.episodes = eps;
        return Some(e);
    }

    // Daily shows.
    if let Some(c) = DAILY.captures(s) {
        let m = c.get(0).unwrap();
        if has_title_before(s, m.start()) {
            let mut e = EpisodeMatch::new(m.start(), m.end());
            e.air_date = Some(Date {
                year: num(&c[1]),
                month: num(&c[2]) as u8,
                day: num(&c[3]) as u8,
            });
            return Some(e);
        }
    }

    // Anime "Title S2 - 05" must win over the "S2" season pack.
    if allow_anime {
        if let Some(e) = anime_match(s, &ANIME_DASH) {
            return Some(e);
        }
    }

    // Season packs.
    for re in [&*SEASON_PACK, &*SEASON_WORD] {
        if let Some(c) = re.captures(s) {
            let m = c.get(0).unwrap();
            if !has_title_before(s, m.start()) {
                continue;
            }
            let mut e = EpisodeMatch::new(m.start(), m.end());
            let first = num(&c[1]);
            e.seasons = match c.get(2).or(c.get(3)).map(|x| num(x.as_str())) {
                Some(last) if last > first && last - first < 100 => (first..=last).collect(),
                _ => vec![first],
            };
            e.full_season = true;
            return Some(e);
        }
    }

    if !allow_anime {
        return None;
    }

    // Anime absolute numbering.
    for re in [&*ANIME_DASH, &*ANIME_BARE] {
        if let Some(e) = anime_match(s, re) {
            return Some(e);
        }
    }
    if loose_anime {
        for c in ANIME_LOOSE.captures_iter(s) {
            let m = c.get(0).unwrap();
            let n = num(&c[1]);
            if has_title_before(s, m.start()) && !(1900..=2099).contains(&n) {
                let mut e = EpisodeMatch::new(m.start(), m.end());
                e.absolute = vec![n];
                e.version = c.get(2).map(|v| num(v.as_str()) as u8);
                return Some(e);
            }
        }
    }
    None
}

fn anime_match(s: &str, re: &Regex) -> Option<EpisodeMatch> {
    for c in re.captures_iter(s) {
        let m = c.get(0).unwrap();
        let n = num(&c[1]);
        if !has_title_before(s, m.start()) || (1900..=2099).contains(&n) && c[1].len() == 4 {
            continue;
        }
        let mut e = EpisodeMatch::new(m.start(), m.end());
        let mut eps = vec![n];
        if let Some(last) = c.get(3) {
            push_episode(&mut eps, num(last.as_str()), true);
        }
        e.absolute = eps;
        e.version = c.get(2).map(|v| num(v.as_str()) as u8);
        return Some(e);
    }
    None
}

/// Adds an episode number; `range` fills in everything since the last one.
fn push_episode(eps: &mut Vec<u16>, n: u16, range: bool) {
    let last = *eps.last().unwrap_or(&0);
    if range && n > last && n - last <= 200 {
        eps.extend(last + 1..=n);
    } else if !eps.contains(&n) {
        eps.push(n);
    }
}

// ---------------------------------------------------------------------------
// Details
// ---------------------------------------------------------------------------

fn read_video(r: &mut Release, t: &str) {
    r.resolution = if let Some(c) = RES_P.captures(t).or_else(|| RES_DIM.captures(t)) {
        match &c[1] {
            "480" => Some(Resolution::R480p),
            "576" => Some(Resolution::R576p),
            "720" => Some(Resolution::R720p),
            "1080" => Some(Resolution::R1080p),
            "2160" => Some(Resolution::R2160p),
            "4320" => Some(Resolution::R4320p),
            _ => None,
        }
    } else if RES_8K.is_match(t) {
        Some(Resolution::R4320p)
    } else if RES_4K.is_match(t) {
        Some(Resolution::R2160p)
    } else {
        None
    };

    let sources: [(&Regex, Source); 15] = [
        (&SRC_REMUX, Source::Remux),
        (&SRC_BDRIP, Source::BdRip),
        (&SRC_WEBDL, Source::WebDl),
        (&SRC_WEBRIP, Source::WebRip),
        (&SRC_BLURAY, Source::BluRay),
        (&SRC_HDTV, Source::Hdtv),
        (&SRC_PDTV, Source::Pdtv),
        (&SRC_SDTV, Source::Sdtv),
        (&SRC_HDRIP, Source::HdRip),
        (&SRC_SCREENER, Source::Screener),
        (&SRC_DVD, Source::Dvd),
        (&SRC_WEB, Source::Web),
        (&SRC_CAM, Source::Cam),
        (&SRC_TS, Source::Telesync),
        (&SRC_TC, Source::Telecine),
    ];
    r.source = first_match(t, &sources);

    let codecs: [(&Regex, VideoCodec); 8] = [
        (&CODEC_H265, VideoCodec::H265),
        (&CODEC_H264, VideoCodec::H264),
        (&CODEC_AV1, VideoCodec::Av1),
        (&CODEC_VP9, VideoCodec::Vp9),
        (&CODEC_XVID, VideoCodec::Xvid),
        (&CODEC_DIVX, VideoCodec::Divx),
        (&CODEC_VC1, VideoCodec::Vc1),
        (&CODEC_MPEG2, VideoCodec::Mpeg2),
    ];
    r.codec = first_match(t, &codecs);

    r.bit_depth = BIT_DEPTH
        .captures(t)
        .and_then(|c| c[1].parse().ok())
        .or_else(|| HI10P.is_match(t).then_some(10));

    let mut hdr = Vec::new();
    if DOLBY_VISION.is_match(t) {
        hdr.push(Hdr::DolbyVision);
    }
    if HDR10_PLUS.is_match(t) {
        hdr.push(Hdr::Hdr10Plus);
    }
    if HDR10.find_iter(t).any(|m| {
        let rest = &t[m.end()..];
        !rest.starts_with('+') && !rest.to_ascii_lowercase().starts_with("plus")
    }) {
        hdr.push(Hdr::Hdr10);
    }
    if HLG.is_match(t) {
        hdr.push(Hdr::Hlg);
    }
    if HDR_PLAIN.is_match(t) && !hdr.iter().any(|h| matches!(h, Hdr::Hdr10 | Hdr::Hdr10Plus)) {
        hdr.push(Hdr::Hdr);
    }
    r.hdr = hdr;

    r.streaming_service = SERVICE.captures(t).map(|c| c[1].to_string());
}

fn read_audio(r: &mut Release, t: &str) {
    let codecs: [(&Regex, AudioCodec); 12] = [
        (&AUD_TRUEHD, AudioCodec::TrueHd),
        (&AUD_DTSX, AudioCodec::DtsX),
        (&AUD_DTSHDMA, AudioCodec::DtsHdMa),
        (&AUD_DTSHD, AudioCodec::DtsHd),
        (&AUD_DTS, AudioCodec::Dts),
        (&AUD_EAC3, AudioCodec::Eac3),
        (&AUD_AC3, AudioCodec::Ac3),
        (&AUD_FLAC, AudioCodec::Flac),
        (&AUD_PCM, AudioCodec::Pcm),
        (&AUD_OPUS, AudioCodec::Opus),
        (&AUD_AAC, AudioCodec::Aac),
        (&AUD_MP3, AudioCodec::Mp3),
    ];
    r.audio = first_match(t, &codecs);
    r.atmos = ATMOS.is_match(t);
    r.audio_channels = CHANNELS.captures(t).map(|c| format!("{}.{}", &c[1], &c[2]));
}

/// Language words and the code each maps to.
static LANGUAGES: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
    [
        (r"english|eng", "en"),
        (r"french|truefrench|vff|vfq|vf2|vfi", "fr"),
        (r"german|deutsch|ger", "de"),
        (r"spanish|español|espanol|castellano|esp|spa", "es"),
        (r"latino|latam", "es-419"),
        (r"italian|ita", "it"),
        (r"japanese|jpn|jap", "ja"),
        (r"korean|kor", "ko"),
        (r"chinese|mandarin|cantonese|chs|cht", "zh"),
        (r"russian|rus", "ru"),
        (r"portuguese|por", "pt"),
        (r"brazilian|pt-br", "pt-BR"),
        (r"dutch|flemish", "nl"),
        (r"polish", "pl"),
        (r"hindi", "hi"),
        (r"swedish", "sv"),
        (r"norwegian", "no"),
        (r"danish", "da"),
        (r"finnish", "fi"),
        (r"turkish", "tr"),
        (r"arabic", "ar"),
        (r"hebrew", "he"),
        (r"thai", "th"),
        (r"vietnamese", "vi"),
        (r"czech", "cs"),
        (r"hungarian", "hu"),
        (r"greek", "el"),
        (r"ukrainian", "uk"),
    ]
    .into_iter()
    .map(|(words, code)| (Regex::new(&format!(r"(?i)\b(?:{words})\b")).unwrap(), code))
    .collect()
});

fn read_language(r: &mut Release, t: &str) {
    // "Eng-Subs" names a subtitle track, not the audio language.
    let t = &*SUBTITLE_TAG.replace_all(t, " ");
    for (re, code) in LANGUAGES.iter() {
        if re.is_match(t) && !r.languages.iter().any(|l| l == code) {
            r.languages.push(code.to_string());
        }
    }
    r.multi_audio = MULTI.is_match(t);
    r.dual_audio = DUAL_AUDIO.is_match(t);
}

/// Edition phrases and their normalized names.
static EDITIONS: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
    [
        (r"director'?s[ .]cut", "Director's Cut"),
        (r"extended(?:[ .](?:cut|edition|version))?", "Extended"),
        (r"theatrical(?:[ .](?:cut|edition|version))?", "Theatrical"),
        (r"unrated", "Unrated"),
        (r"uncut", "Uncut"),
        (r"imax(?:[ .]edition)?", "IMAX"),
        (r"(?:the[ .])?final[ .]cut", "Final Cut"),
        (r"special[ .]edition", "Special Edition"),
        (r"ultimate[ .](?:edition|cut)", "Ultimate Edition"),
        (
            r"\d{2,3}(?:th|st|nd|rd)[ .]anniversary(?:[ .]edition)?",
            "Anniversary Edition",
        ),
        (r"anniversary[ .]edition", "Anniversary Edition"),
        (r"collector'?s[ .]edition", "Collector's Edition"),
        (r"criterion(?:[ .]collection)?", "Criterion"),
        (r"open[ .]matte", "Open Matte"),
        (r"remastered", "Remastered"),
    ]
    .into_iter()
    .map(|(words, name)| (Regex::new(&format!(r"(?i)\b{words}\b")).unwrap(), name))
    .collect()
});

static TRAILING_EDITIONS: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
    EDITIONS
        .iter()
        .map(|(re, name)| {
            (
                Regex::new(&format!(
                    r"(?i)^(.*\S)\s+{}$",
                    re.as_str().trim_start_matches("(?i)")
                ))
                .unwrap(),
                *name,
            )
        })
        .collect()
});

static TITLE_YEAR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(.*\S)\s+\(?((?:19|20)\d{2})\)?$").unwrap());

fn strip_trailing_edition(title: &str) -> Option<(String, String)> {
    TRAILING_EDITIONS.iter().find_map(|(re, name)| {
        re.captures(title)
            .map(|c| (c[1].trim().to_string(), name.to_string()))
    })
}

fn read_release_info(r: &mut Release, t: &str) {
    r.edition = EDITIONS
        .iter()
        .find(|(re, _)| re.is_match(t))
        .map(|(_, name)| name.to_string());
    let propers = PROPER.find_iter(t).count() as u8;
    r.revision.version = r.revision.version.max(1 + propers);
    r.revision.real = REAL.find_iter(t).count() as u8;
    r.complete = COMPLETE.is_match(t);
}

fn read_book(r: &mut Release, s: &str) {
    let formats: [(&Regex, BookFormat); 8] = [
        (&FMT_EPUB, BookFormat::Epub),
        (&FMT_AZW3, BookFormat::Azw3),
        (&FMT_MOBI, BookFormat::Mobi),
        (&FMT_PDF, BookFormat::Pdf),
        (&FMT_M4B, BookFormat::M4b),
        (&FMT_M4A, BookFormat::M4a),
        (&FMT_FLAC, BookFormat::Flac),
        (&FMT_MP3, BookFormat::Mp3),
    ];
    r.book_format = first_match(s, &formats).or_else(|| {
        r.container.as_deref().and_then(|e| match e {
            "azw" => Some(BookFormat::Azw3),
            other => BookFormat::from_name(other),
        })
    });
    r.abridged = if UNABRIDGED.is_match(s) {
        Some(false)
    } else if ABRIDGED.is_match(s) {
        Some(true)
    } else {
        None
    };
    r.retail = RETAIL.is_match(s);
    r.bitrate_kbps = BITRATE.captures(s).and_then(|c| c[1].parse().ok());

    let spaced = if s.contains(' ') {
        s.to_string()
    } else {
        s.replace('.', " ")
    };
    r.narrator = NARRATOR
        .captures(&spaced)
        .map(|c| c[1].trim().to_string())
        .filter(|n| !n.is_empty());

    // Book audio fields come from the format, not video-style tags.
    r.audio = None;
    r.audio_channels = None;
}

fn book_kind(r: &Release) -> MediaKind {
    let audio_format = matches!(
        r.book_format,
        Some(BookFormat::M4b | BookFormat::M4a | BookFormat::Mp3 | BookFormat::Flac)
    );
    if audio_format
        || r.narrator.is_some()
        || r.abridged.is_some()
        || AUDIOBOOK_SIGNAL.is_match(&r.raw)
    {
        MediaKind::Audiobook
    } else {
        MediaKind::Ebook
    }
}

/// "Author - Title" or "Title by Author".
fn split_author(t: &str) -> (Option<String>, String) {
    if let Some((author, title)) = t.split_once(" - ") {
        let (author, title) = (author.trim(), title.trim());
        if !author.is_empty() && !title.is_empty() {
            return (Some(author.to_string()), title.to_string());
        }
    }
    if let Some(c) = BY_AUTHOR.captures(t) {
        return (Some(c[2].trim().to_string()), c[1].trim().to_string());
    }
    (None, t.to_string())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Takes a trailing `-GROUP` off the name.
fn take_scene_group(s: &mut String) -> Option<String> {
    let trimmed = s.trim_end();
    let idx = trimmed.rfind('-')?;
    let cand = &trimmed[idx + 1..];
    let before = trimmed[..idx].chars().last()?;
    let lower = cand.to_ascii_lowercase();
    let ok = !cand.is_empty()
        && cand.len() <= 32
        && before != ' '
        && !cand.contains([' ', '.', '[', ']', '(', ')'])
        && cand.chars().any(|c| c.is_ascii_alphabetic())
        && !NOT_GROUPS.contains(&lower.as_str())
        && !RES_P.is_match(cand)
        && !SXE.is_match(cand)
        && !EPISODE_ONLY.is_match(cand)
        && ANY_MARKER.is_match(&trimmed[..idx]);
    if !ok {
        return None;
    }
    let group = cand.to_string();
    s.truncate(idx);
    Some(group)
}

static EPISODE_ONLY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^e\d{1,4}$").unwrap());

/// True when there is some title text before `pos`.
fn has_title_before(s: &str, pos: usize) -> bool {
    s[..pos].chars().any(|c| c.is_alphanumeric())
}

/// Turns the raw title slice into a readable title.
fn clean_title(raw: &str) -> Option<String> {
    let t = raw.trim_matches(|c: char| c.is_whitespace() || "-._[({~".contains(c));
    if t.is_empty() {
        return None;
    }

    let words: Vec<String> = if t.contains(' ') {
        t.split(' ').map(str::to_string).collect()
    } else {
        // Dotted names: rebuild acronyms like S.W.A.T. and J.R.R.
        let parts: Vec<&str> = t.split('.').collect();
        let mut out: Vec<String> = Vec::new();
        let mut acronym = String::new();
        let mut run = 0;
        for p in &parts {
            if p.chars().count() == 1 && p.chars().all(char::is_alphabetic) {
                acronym.push_str(p);
                acronym.push('.');
                run += 1;
            } else {
                flush_acronym(&mut out, &mut acronym, &mut run);
                if !p.is_empty() {
                    out.push(p.to_string());
                }
            }
        }
        flush_acronym(&mut out, &mut acronym, &mut run);
        out
    };

    let joined = words
        .iter()
        .filter(|w| !w.is_empty())
        .cloned()
        .collect::<Vec<_>>()
        .join(" ");
    let cleaned = joined
        .trim_matches(|c: char| c.is_whitespace() || "-([".contains(c))
        .to_string();
    (!cleaned.is_empty()).then_some(cleaned)
}

fn flush_acronym(out: &mut Vec<String>, acronym: &mut String, run: &mut usize) {
    match *run {
        0 => {}
        // A single letter is a word ("A Quiet Place"), not an acronym.
        1 => out.push(acronym.trim_end_matches('.').to_string()),
        _ => out.push(acronym.clone()),
    }
    acronym.clear();
    *run = 0;
}

fn first_match<T: Copy>(t: &str, table: &[(&Regex, T)]) -> Option<T> {
    table.iter().find(|(re, _)| re.is_match(t)).map(|(_, v)| *v)
}

fn num(s: &str) -> u16 {
    s.parse().unwrap_or(0)
}

fn min_opt(a: Option<usize>, b: Option<usize>) -> Option<usize> {
    match (a, b) {
        (Some(x), Some(y)) => Some(x.min(y)),
        (x, None) => x,
        (None, y) => y,
    }
}
