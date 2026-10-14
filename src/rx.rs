//! Compiled patterns. All run on the working string, where `_` has already
//! been replaced by a space so `\b` treats it as a separator.

use regex::Regex;
use std::sync::LazyLock;

macro_rules! rx {
    ($($name:ident = $pat:expr;)+) => {
        $(pub static $name: LazyLock<Regex> =
            LazyLock::new(|| Regex::new($pat).expect(concat!("bad regex ", stringify!($name))));)+
    };
}

rx! {
    // ---- Cleanup ----------------------------------------------------------
    LEADING_GROUP = r"^\s*\[([^\]]{1,40})\]\s*(?:-\s*)?";
    CRC = r"[\[\(]([0-9A-Fa-f]{8})[\]\)]";
    SITE_TAG = r"(?i)\s*\[\s*(rarbg|eztvx?(?:\.[a-z]{2,3})?|ettv|tgx|rartv|yts(?:\.[a-z]{2,3})?|yify)\s*\]";

    // ---- Episodes ---------------------------------------------------------
    // S01E01, S01E01E02, S01E01-E03, S01E01-03, S01.E01
    SXE = r"(?i)\bS(\d{1,4})[ .]?E(\d{1,4})((?:[ .]?-?[ .]?E\d{1,4}\b|-\d{1,4}\b)*)";
    EP_TAIL = r"(?i)(-)?[ .]?E?(\d{1,4})";
    // 1x05, 1x05-07, 1x05-1x07
    NXNN = r"(?i)\b(\d{1,2})x(\d{2,3})(?:-(?:\d{1,2}x)?(\d{2,3}))?\b";
    // 2024.03.15 / 2024-03-15 / 2024 03 15
    DAILY = r"\b((?:19|20)\d{2})[ .-](0[1-9]|1[0-2])[ .-](0[1-9]|[12]\d|3[01])\b";
    // S01, S01-S03, S01-03
    SEASON_PACK = r"(?i)\bS(\d{1,2})(?:-S?(\d{1,2})|\s?-\s?S(\d{1,2}))?\b";
    // Season 1, Seasons 1-3, Season.2
    SEASON_WORD = r"(?i)\bseasons?[ .]?(\d{1,2})(?:[ .]?(?:-|to|&)[ .]?(\d{1,2}))?\b";
    // Anime: "Title - 05", "Title - 05v2", "Title - 01-12", "Title - 01 ~ 12"
    ANIME_DASH = r"(?i)\s-\s(\d{1,4})(?:v(\d))?(?:\s?[-~]\s?(\d{1,4})(?:v\d)?)?(?:\s|$|\[|\()";
    // Anime with a leading [Group] and no dash: "[Group] Title 05 [1080p]"
    ANIME_BARE = r"(?i)\s(\d{2,4})(?:v(\d))?\s*(?:\[|\(|$)";
    // Anime hint only: "One.Piece.1089.1080p"
    ANIME_LOOSE = r"(?i)\b(\d{2,4})(?:v(\d))?\b";
    // Anime season at the end of the title: "Spy x Family S2", "Season 2", "2nd Season"
    ANIME_TITLE_SEASON = r"(?i)^(.*?)\s+(?:S(\d{1,2})|Season\s?(\d{1,2})|(\d{1,2})(?:st|nd|rd|th)\s+Season)$";

    // ---- Boundaries -------------------------------------------------------
    // Tokens that only appear after a title. Weak words like "web" or
    // "complete" are left out so titles such as "Charlotte's Web" survive.
    QUALITY_STRONG = r"(?i)\b(?:\d{3,4}[pi]|4k|8k|uhd|remux|blu-?ray|bdrip|brrip|bd-?rip|bdmv|web-?dl|webrip|web-?rip|hdtv|pdtv|sdtv|dvd-?rip|dvd[59r]|hdrip|hd-?cam|camrip|telesync|hdts|telecine|dvdscr|screener|[xh]\.?26[45]|hevc|avc|av1|xvid|divx|10-?bits?|hi10p|true-?hd|dts(?:-hd)?|ddp|dd\+|e?ac-?3|proper|repack|rerip|internal|multi|dual[ .-]?audio|hdr10\+?|dovi)\b";
    YEAR = r"\b((?:19|20)\d{2})\b";
    BOOK_MARK = r"(?i)\b(?:epub|mobi|azw3?|pdf|e-?book|audio-?books?|audio book|m4b|unabridged|abridged|retail|(?:narrated|read)\s+by|\d{2,3}\s?kbps)\b";
    BOOK_SIGNAL = r"(?i)\b(?:epub|mobi|azw3?|e-?book|audio-?books?|audio book|m4b|unabridged|abridged|(?:narrated|read)\s+by)\b";
    AUDIOBOOK_SIGNAL = r"(?i)\b(?:audio-?books?|audio book|m4b|unabridged|abridged|(?:narrated|read)\s+by|\d{2,3}\s?kbps)\b";
    // Anything that shows the name has release info (for group detection).
    ANY_MARKER = r"(?i)\b(?:\d{3,4}[pi]|(?:19|20)\d{2}|S\d{1,2}(?:E\d{1,4})?|\d{1,2}x\d{2,3}|web|hdtv|blu-?ray|remux|[xh]\.?26[45]|hevc|xvid|epub|mobi|azw3?|m4b|e-?book|audiobook|retail)\b";

    // ---- Video ------------------------------------------------------------
    RES_P = r"(?i)\b(480|576|720|1080|2160|4320)[pi]\b";
    RES_DIM = r"(?i)\b(?:\d{3,4})x(480|576|720|1080|2160|4320)\b";
    RES_4K = r"(?i)\b(?:4k|uhd)\b";
    RES_8K = r"(?i)\b8k\b";

    SRC_REMUX = r"(?i)\bremux\b";
    SRC_BDRIP = r"(?i)\b(?:bdrip|brrip|bd-?rip|br-?rip)\b";
    SRC_BLURAY = r"(?i)\b(?:blu-?ray|bd(?:25|50|66|100)?|bdmv|uhd[ .-]?bd)\b";
    SRC_WEBDL = r"(?i)\b(?:web-?dl|webdl|web[ .]dl)\b";
    SRC_WEBRIP = r"(?i)\b(?:web-?rip|webrip)\b";
    SRC_WEB = r"(?i)\bweb\b";
    SRC_HDTV = r"(?i)\b(?:hdtv|ahdtv|uhdtv)\b";
    SRC_PDTV = r"(?i)\bpdtv\b";
    SRC_SDTV = r"(?i)\b(?:sdtv|tvrip|dsr(?:ip)?)\b";
    SRC_DVD = r"(?i)\b(?:dvd-?rip|dvd[59r]?|ntsc|pal)\b";
    SRC_HDRIP = r"(?i)\bhd-?rip\b";
    SRC_SCREENER = r"(?i)\b(?:dvdscr|screener|scr)\b";
    SRC_CAM = r"(?i)\b(?:cam|camrip|hd-?cam)\b";
    SRC_TS = r"(?i)\b(?:ts|telesync|hd-?ts|pdvd)\b";
    SRC_TC = r"(?i)\b(?:tc|telecine)\b";

    CODEC_H265 = r"(?i)\b(?:[xh]\.?265|hevc)\b";
    CODEC_H264 = r"(?i)\b(?:[xh]\.?264|avc)\b";
    CODEC_AV1 = r"(?i)\bav1\b";
    CODEC_VP9 = r"(?i)\bvp9\b";
    CODEC_XVID = r"(?i)\bxvid\b";
    CODEC_DIVX = r"(?i)\bdivx\b";
    CODEC_VC1 = r"(?i)\bvc-?1\b";
    CODEC_MPEG2 = r"(?i)\bmpeg-?2\b";

    BIT_DEPTH = r"(?i)\b(8|10|12)[ .-]?bits?\b";
    HI10P = r"(?i)\bhi10p?\b";

    HDR10_PLUS = r"(?i)\bhdr10(?:\+|plus\b)";
    HDR10 = r"(?i)\bhdr10\b";
    HDR_PLAIN = r"(?i)\bhdr\b";
    HLG = r"(?i)\bhlg\b";
    DOLBY_VISION = r"(?i)\b(?:dv|dovi|dolby[ .]?vision)\b";

    SERVICE = r"\b(AMZN|NF|DSNP|HMAX|MAX|ATVP|HULU|PCOK|PMTP|STAN|CRAV|iT|CR|FUNI|HIDIVE|ROKU|SHO|STZ|SKST|BCORE|VRV|ADN|iP|ITVX|RTE|TVNZ|ZEE5|JC)\b";

    // ---- Audio ------------------------------------------------------------
    AUD_TRUEHD = r"(?i)\btrue-?hd\b";
    AUD_DTSX = r"(?i)\bdts[ .:-]?x\b";
    AUD_DTSHDMA = r"(?i)\bdts[ .-]?hd[ .-]?ma\b";
    AUD_DTSHD = r"(?i)\bdts[ .-]?hd\b";
    AUD_DTS = r"(?i)\bdts\b";
    AUD_EAC3 = r"(?i)(?:\bddp|\bdd\+|\be-?ac-?3|\bdolby digital plus)";
    AUD_AC3 = r"(?i)(?:\bac-?3\b|\bdd(?:\d|\b))";
    AUD_AAC = r"(?i)\baac(?:\d|\b)";
    AUD_FLAC = r"(?i)\bflac\b";
    AUD_OPUS = r"(?i)\bopus\b";
    AUD_MP3 = r"(?i)\bmp3\b";
    AUD_PCM = r"(?i)\bl?pcm\b";
    ATMOS = r"(?i)\batmos\b";
    CHANNELS = r"(?:^|[^0-9])([1-9])\.([0-2])(?:[^0-9]|$)";

    // ---- Language ---------------------------------------------------------
    SUBTITLE_TAG = r"(?i)\b[a-z]+(?:[ .-][a-z]+)?[ .-]?(?:subs?|subtitles?|subbed)\b";
    MULTI = r"(?i)\bmulti(?:-?lang|-?audio)?\b";
    DUAL_AUDIO = r"(?i)\bdual[ .-]?audio\b";

    // ---- Release info -----------------------------------------------------
    PROPER = r"(?i)\b(?:proper|repack|rerip)\b";
    REAL = r"\bREAL\b";
    COMPLETE = r"(?i)\bcomplete\b";

    // ---- Books ------------------------------------------------------------
    FMT_EPUB = r"(?i)\bepub\b";
    FMT_MOBI = r"(?i)\bmobi\b";
    FMT_AZW3 = r"(?i)\bazw3?\b";
    FMT_PDF = r"(?i)\bpdf\b";
    FMT_M4B = r"(?i)\bm4b\b";
    FMT_M4A = r"(?i)\bm4a\b";
    FMT_MP3 = r"(?i)\bmp3\b";
    FMT_FLAC = r"(?i)\bflac\b";
    UNABRIDGED = r"(?i)\bunabridged\b";
    ABRIDGED = r"(?i)\babridged\b";
    RETAIL = r"(?i)\bretail\b";
    BITRATE = r"(?i)\b(\d{2,3})\s?kbps\b";
    NARRATOR = r"(?i)\b(?:narrated|read)\s+by\s+(.+?)\s*(?:[\[\(,]|\s-\s|\b(?:19|20)\d{2}\b|\b(?:unabridged|abridged|m4b|mp3|retail|audiobook)\b|\b\d{2,3}\s?kbps\b|$)";
    BY_AUTHOR = r"(?i)^(.+?)\s+by\s+(.+)$";
}
