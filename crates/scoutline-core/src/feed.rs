//! Reads Newznab and Torznab RSS responses.

use chrono::{DateTime, Utc};
use roxmltree::{Document, Node, ParsingOptions};

use crate::error::IndexerError;

/// One `<item>` from a feed, before playbill parsing.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FeedItem {
    pub title: String,
    pub guid: Option<String>,
    pub link: Option<String>,
    pub comments: Option<String>,
    pub published: Option<DateTime<Utc>>,
    pub size: Option<u64>,
    pub categories: Vec<u32>,
    pub enclosure_url: Option<String>,
    /// Every `newznab:attr` / `torznab:attr`, in order.
    pub attrs: Vec<(String, String)>,
}

impl FeedItem {
    /// First value of an extended attribute, e.g. `"seeders"`.
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    pub fn attr_num<T: std::str::FromStr>(&self, name: &str) -> Option<T> {
        self.attr(name)?.trim().parse().ok()
    }
}

/// Parses a feed. A Newznab `<error>` response becomes [`IndexerError::Api`].
pub fn parse_feed(xml: &str) -> Result<Vec<FeedItem>, IndexerError> {
    let opts = ParsingOptions {
        allow_dtd: true,
        ..ParsingOptions::default()
    };
    let doc = Document::parse_with_options(xml.trim_start_matches('\u{feff}'), opts)
        .map_err(|e| IndexerError::Parse(e.to_string()))?;
    let root = doc.root_element();

    if root.tag_name().name() == "error" {
        return Err(IndexerError::Api {
            code: root
                .attribute("code")
                .and_then(|c| c.parse().ok())
                .unwrap_or(0),
            description: root.attribute("description").unwrap_or("").to_owned(),
        });
    }
    if root.tag_name().name() != "rss" {
        return Err(IndexerError::Parse(format!(
            "expected <rss>, got <{}>",
            root.tag_name().name()
        )));
    }

    Ok(root
        .descendants()
        .filter(|n| n.has_tag_name("item"))
        .filter_map(read_item)
        .collect())
}

fn read_item(item: Node) -> Option<FeedItem> {
    let mut out = FeedItem::default();
    let mut text_categories = Vec::new();
    let mut enclosure_len = None;
    let mut size_elem = None;

    for child in item.children().filter(Node::is_element) {
        let text = || {
            child
                .text()
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .map(str::to_owned)
        };
        // `attr` lives in the newznab or torznab namespace; accept either.
        match child.tag_name().name() {
            "title" => out.title = text().unwrap_or_default(),
            "guid" => out.guid = text(),
            "link" => out.link = text(),
            "comments" => out.comments = text(),
            "pubDate" => {
                out.published = text()
                    .and_then(|d| DateTime::parse_from_rfc2822(&d).ok())
                    .map(|d| d.with_timezone(&Utc));
            }
            "size" => size_elem = text().and_then(|s| s.parse().ok()),
            "category" => text_categories.extend(text().and_then(|c| c.parse::<u32>().ok())),
            "enclosure" => {
                out.enclosure_url = child.attribute("url").map(str::to_owned);
                enclosure_len = child
                    .attribute("length")
                    .and_then(|l| l.parse::<u64>().ok());
            }
            "attr" => {
                if let (Some(k), Some(v)) = (child.attribute("name"), child.attribute("value")) {
                    out.attrs.push((k.to_owned(), v.to_owned()));
                }
            }
            _ => {}
        }
    }

    if out.title.is_empty() {
        return None;
    }
    out.categories = out
        .attrs
        .iter()
        .filter(|(k, _)| k == "category")
        .filter_map(|(_, v)| v.trim().parse().ok())
        .collect();
    if out.categories.is_empty() {
        out.categories = text_categories;
    }
    out.size = out
        .attr_num::<u64>("size")
        .or(size_elem)
        .or(enclosure_len)
        .filter(|&s| s > 0);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TORZNAB: &str = include_str!("../tests/fixtures/torznab.xml");
    const NEWZNAB: &str = include_str!("../tests/fixtures/newznab.xml");

    #[test]
    fn reads_torznab() {
        let items = parse_feed(TORZNAB).unwrap();
        assert_eq!(items.len(), 2);
        let a = &items[0];
        assert_eq!(
            a.title,
            "The.Bear.S03E04.2160p.WEB-DL.DDP5.1.HDR.H.265-FLUX"
        );
        assert_eq!(a.size, Some(6_442_450_944));
        assert_eq!(a.categories, vec![5000, 5045]);
        assert_eq!(a.attr("seeders"), Some("120"));
        assert_eq!(a.attr_num::<u32>("peers"), Some(135));
        assert_eq!(
            a.published.unwrap().to_rfc3339(),
            "2026-10-13T08:00:00+00:00"
        );
    }

    #[test]
    fn reads_newznab() {
        let items = parse_feed(NEWZNAB).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0].enclosure_url.as_deref(),
            Some("https://nzb.example/getnzb/abc.nzb&i=1&r=KEY")
        );
        assert_eq!(items[0].size, Some(5_368_709_120));
        assert_eq!(items[0].categories, vec![5040]);
    }

    #[test]
    fn api_error() {
        let err = parse_feed(
            r#"<?xml version="1.0"?><error code="100" description="Incorrect user credentials"/>"#,
        )
        .unwrap_err();
        assert!(matches!(err, IndexerError::Api { code: 100, .. }));
    }

    #[test]
    fn garbage_is_parse_error() {
        assert!(matches!(
            parse_feed("<html>nope"),
            Err(IndexerError::Parse(_))
        ));
        assert!(matches!(
            parse_feed("<html></html>"),
            Err(IndexerError::Parse(_))
        ));
    }
}
