//! Standard Newznab categories. Indexers may add their own ids above
//! 100000; those are mapped back to a standard parent by the caller.

/// A standard Newznab category.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Category {
    pub id: u32,
    pub name: &'static str,
}

macro_rules! categories {
    ($($konst:ident = $id:literal, $name:literal;)+) => {
        $(pub const $konst: Category = Category { id: $id, name: $name };)+
        /// Every standard category, parents first in each group.
        pub const ALL: &[Category] = &[$($konst),+];
    };
}

categories! {
    CONSOLE = 1000, "Console";
    MOVIES = 2000, "Movies";
    MOVIES_FOREIGN = 2010, "Movies/Foreign";
    MOVIES_OTHER = 2020, "Movies/Other";
    MOVIES_SD = 2030, "Movies/SD";
    MOVIES_HD = 2040, "Movies/HD";
    MOVIES_UHD = 2045, "Movies/UHD";
    MOVIES_BLURAY = 2050, "Movies/BluRay";
    MOVIES_3D = 2060, "Movies/3D";
    MOVIES_DVD = 2070, "Movies/DVD";
    MOVIES_WEBDL = 2080, "Movies/WEB-DL";
    AUDIO = 3000, "Audio";
    AUDIO_MP3 = 3010, "Audio/MP3";
    AUDIO_VIDEO = 3020, "Audio/Video";
    AUDIO_AUDIOBOOK = 3030, "Audio/Audiobook";
    AUDIO_LOSSLESS = 3040, "Audio/Lossless";
    AUDIO_OTHER = 3050, "Audio/Other";
    PC = 4000, "PC";
    TV = 5000, "TV";
    TV_WEBDL = 5010, "TV/WEB-DL";
    TV_FOREIGN = 5020, "TV/Foreign";
    TV_SD = 5030, "TV/SD";
    TV_HD = 5040, "TV/HD";
    TV_UHD = 5045, "TV/UHD";
    TV_OTHER = 5050, "TV/Other";
    TV_SPORT = 5060, "TV/Sport";
    TV_ANIME = 5070, "TV/Anime";
    TV_DOCUMENTARY = 5080, "TV/Documentary";
    XXX = 6000, "XXX";
    BOOKS = 7000, "Books";
    BOOKS_MAGS = 7010, "Books/Mags";
    BOOKS_EBOOK = 7020, "Books/EBook";
    BOOKS_COMICS = 7030, "Books/Comics";
    BOOKS_TECHNICAL = 7040, "Books/Technical";
    BOOKS_OTHER = 7050, "Books/Other";
    BOOKS_FOREIGN = 7060, "Books/Foreign";
    OTHER = 8000, "Other";
}

/// Looks up a standard category by id.
pub fn by_id(id: u32) -> Option<Category> {
    ALL.iter().copied().find(|c| c.id == id)
}

/// The top-level group a standard id belongs to, e.g. 5040 -> 5000.
pub fn parent_id(id: u32) -> Option<u32> {
    (1000..100_000).contains(&id).then(|| id / 1000 * 1000)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookups() {
        assert_eq!(by_id(5070).unwrap().name, "TV/Anime");
        assert_eq!(parent_id(5070), Some(5000));
        assert_eq!(parent_id(100_123), None);
    }
}
