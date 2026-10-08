//! Opera's own history, before Opera 15 moved to Chromium (Opera 12 and
//! older, Presto): `global_history.dat` and `typed_history.xml` in the
//! profile.
//!
//! `global_history.dat` is text (UTF-8), four lines per page: its title,
//! its URL, when it was last visited (Unix seconds) and its popularity
//! index, which Opera ranks suggestions by (-1 for a page visited once).
//! `typed_history.xml` lists what was typed in the address bar, newest
//! first: `<typed_history_item content="mbl.is" type="text"
//! last_typed="2013-11-11T22:58:24Z"/>`, its type `text` when typed out,
//! `selected` when picked from the suggestions.

use common::time::Ts;

use crate::{Transition, Visit};

/// How a typed history entry was entered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypedEntry {
    /// Typed out (`text`).
    Typed,
    /// Picked from the suggestions (`selected`).
    Selected,
    /// Any other type, as written; empty when none.
    Other(String),
}

/// Something typed in Opera's address bar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypedUrl {
    /// What was typed: a URL or a host (`mbl.is`).
    pub url: String,
    /// When it was last typed.
    pub time: Option<Ts>,
    /// How.
    pub entry: TypedEntry,
}

/// The table name given to a `global_history.dat`'s visits.
const GLOBAL_HISTORY: &str = "global_history";
/// The longest line a `global_history.dat` is recognized by, as plaso.
const LONGEST_LINE: usize = 512;
const SCHEMES: [&str; 4] = ["file", "http", "https", "ftp"];

/// Whether `data` starts like a `global_history.dat`: a title, a URL of a
/// known scheme, a time and a popularity index, each on its line.
pub(crate) fn is_global_history(data: &[u8]) -> bool {
    let mut lines = data.split(|&b| b == b'\n');
    let mut next = || {
        lines
            .next()
            .filter(|line| line.len() < LONGEST_LINE)
            .and_then(|line| std::str::from_utf8(line).ok())
            .map(str::trim)
            .filter(|line| !line.is_empty())
    };
    let (Some(_title), Some(url), Some(time), Some(popularity)) = (next(), next(), next(), next())
    else {
        return false;
    };
    let scheme = url.split_once(':').map_or("", |(scheme, _)| scheme);
    SCHEMES.contains(&scheme.to_ascii_lowercase().as_str())
        && time.parse::<i64>().is_ok()
        && popularity.parse::<i64>().is_ok()
}

/// Whether `data` is a `typed_history.xml`: its root is `typed_history`.
pub(crate) fn is_typed_history(data: &[u8]) -> bool {
    let text = String::from_utf8_lossy(data.get(..512).unwrap_or(data));
    text.trim_start().starts_with("<?xml")
        && elements(&text)
            .next()
            .is_some_and(|e| e.name == "typed_history")
}

/// Every page of a `global_history.dat`, as a visit at its last visit time;
/// a record whose time or index doesn't read is kept without them and
/// reported.
pub(crate) fn global_history(data: &[u8], problems: &mut Vec<String>) -> Vec<Visit> {
    let text = String::from_utf8_lossy(data);
    let mut lines = text.lines().map(str::trim);
    let mut visits = Vec::new();
    for index in 0i64.. {
        let Some(title) = lines.next().filter(|title| !title.is_empty()) else {
            break;
        };
        let url = lines.next().unwrap_or_default();
        let time = lines.next().unwrap_or_default();
        let popularity = lines.next().unwrap_or_default();
        let time = if let Ok(seconds) = time.parse::<i64>() {
            Some(Ts::from_unix_seconds(seconds))
        } else {
            problems.push(format!("record {index}: time {time:?}"));
            None
        };
        let popularity = popularity.parse::<i64>().ok().or_else(|| {
            problems.push(format!("record {index}: popularity index {popularity:?}"));
            None
        });
        visits.push(Visit {
            id: index,
            table: GLOBAL_HISTORY.to_owned(),
            time,
            url: url.to_owned(),
            title: title.to_owned(),
            user: None,
            transition: Transition::NotRecorded,
            from_visit: None,
            duration: None,
            visit_count: None,
            typed: false,
            typed_count: None,
            hidden: false,
            frecency: popularity,
        });
    }
    visits
}

/// Every entry of a `typed_history.xml`, in file order; one whose time
/// doesn't read is kept without it and reported.
pub(crate) fn typed_history(data: &[u8], problems: &mut Vec<String>) -> Vec<TypedUrl> {
    let text = String::from_utf8_lossy(data);
    elements(&text)
        .filter(|element| element.name == "typed_history_item")
        .enumerate()
        .map(|(index, item)| {
            let last_typed = item.attribute("last_typed");
            let time = last_typed.as_deref().and_then(Ts::parse_iso8601_utc);
            if time.is_none() {
                problems.push(format!("typed_history_item {index}: time {last_typed:?}"));
            }
            TypedUrl {
                url: item.attribute("content").unwrap_or_default(),
                time,
                entry: match item.attribute("type").unwrap_or_default().as_str() {
                    "text" => TypedEntry::Typed,
                    "selected" => TypedEntry::Selected,
                    other => TypedEntry::Other(other.to_owned()),
                },
            }
        })
        .collect()
}

/// An XML start tag: its name and attributes, as written.
struct Element<'t> {
    name: &'t str,
    attributes: &'t str,
}

impl Element<'_> {
    /// An attribute's value, its entities decoded.
    fn attribute(&self, name: &str) -> Option<String> {
        let mut rest = self.attributes;
        loop {
            let (key, after) = rest.split_once('=')?;
            let after = after.trim_start();
            let quote = after.chars().next().filter(|q| matches!(q, '"' | '\''))?;
            let (value, tail) = after[1..].split_once(quote)?;
            if key.trim() == name {
                return Some(unescape(value));
            }
            rest = tail;
        }
    }
}

/// The start tags of an XML document, in order; declarations, comments
/// and end tags skipped.
fn elements(text: &str) -> impl Iterator<Item = Element<'_>> {
    text.split('<').skip(1).filter_map(|tag| {
        let tag = tag.split_once('>').map_or(tag, |(tag, _)| tag);
        if tag.starts_with(['?', '!', '/']) {
            return None;
        }
        let tag = tag.strip_suffix('/').unwrap_or(tag);
        let (name, attributes) = tag
            .split_once(|c: char| c.is_ascii_whitespace())
            .unwrap_or((tag, ""));
        Some(Element { name, attributes })
    })
}

/// XML's five entities and character references decoded; anything else
/// kept as written.
fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let decoded = rest.split_once(';').and_then(|(entity, _)| {
            let character = match &entity[1..] {
                "amp" => '&',
                "lt" => '<',
                "gt" => '>',
                "quot" => '"',
                "apos" => '\'',
                reference => {
                    let code = match reference.strip_prefix("#x") {
                        Some(hex) => u32::from_str_radix(hex, 16).ok(),
                        None => reference.strip_prefix('#')?.parse().ok(),
                    };
                    char::from_u32(code?)?
                }
            };
            Some((character, entity.len() + 1))
        });
        if let Some((character, length)) = decoded {
            out.push(character);
            rest = &rest[length..];
        } else {
            out.push('&');
            rest = &rest[1..];
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attributes_and_entities() {
        let xml = "<?xml version=\"1.0\"?>\n<typed_history>\n<typed_history_item \
                   content='a&amp;b&#x41;&#66;&bogus;' type=\"text\"\n last_typed=\"x\"/>";
        let items: Vec<_> = elements(xml).collect();
        assert_eq!(items.len(), 2);
        assert_eq!(
            items[1].attribute("content").as_deref(),
            Some("a&bAB&bogus;")
        );
        assert_eq!(items[1].attribute("type").as_deref(), Some("text"));
        assert_eq!(items[1].attribute("missing"), None);
        assert!(is_typed_history(xml.as_bytes()));
    }
}
