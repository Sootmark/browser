//! Google Analytics' cookies, from the classic `ga.js` script (2007 to
//! about 2017), which sites set on their own domain and every browser
//! keeps: when a visitor first came, came back and last came, how many
//! sessions and pages, and how they got there (the search engine, the
//! campaign, the words searched). Each is a value of fields separated by
//! dots:
//!
//! - `__utma`, the visitor: domain hash, visitor id, first, previous and
//!   last visit (Unix seconds), sessions (`137167072.1215918423.1383170166.1383170166.1383170166.1`).
//! - `__utmb`, the session: domain hash, pages viewed, a count of 10 less
//!   the requests sent, the session's last time (Unix seconds, or
//!   milliseconds when the count is 8 or 9, or the time has 13 digits)
//!   (`137167072.1.10.1383170166`).
//! - `__utmz`, the campaign: domain hash, last time (Unix seconds),
//!   sessions, sources, then `|`-separated `key=value` variables whose
//!   values are URL-encoded (`utmcsr` the source, `utmccn` the campaign,
//!   `utmcmd` the medium, `utmctr` the words searched, `utmcct` the
//!   referring path); dots in the variables are kept.
//! - `__utmt`, the request throttle.
//!
//! Any of them may instead hold a single number, a time in 100 nanosecond
//! ticks since 1970 (`13113225820000000`), as plaso reads it.

use common::time::{Precision, Ts};

use crate::Cookie;

/// What a Google Analytics cookie says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GoogleAnalytics {
    /// `__utma`: the visitor.
    Utma {
        /// The hash of the site's domain.
        domain_hash: Option<String>,
        /// The visitor's random id.
        visitor_id: Option<String>,
        /// When the visitor first came.
        first_visit: Option<Ts>,
        /// When the visitor came before the last time.
        previous_visit: Option<Ts>,
        /// When the visitor last came (the time of a single-field value).
        last_visit: Option<Ts>,
        /// The visitor's sessions.
        sessions: Option<i64>,
    },
    /// `__utmb`: the session.
    Utmb {
        /// The hash of the site's domain.
        domain_hash: Option<String>,
        /// The pages viewed in the session.
        pages_viewed: Option<i64>,
        /// The session's last time.
        last_visit: Option<Ts>,
    },
    /// `__utmt`: the request throttle.
    Utmt {
        /// Its time.
        last_visit: Option<Ts>,
    },
    /// `__utmz`: how the visitor came.
    Utmz {
        /// The hash of the site's domain.
        domain_hash: Option<String>,
        /// When the campaign was last recorded.
        last_visit: Option<Ts>,
        /// The visitor's sessions.
        sessions: Option<i64>,
        /// The sources the visitor came from.
        sources: Option<i64>,
        /// The variables (`utmcsr`, `utmccn`, `utmcmd`, `utmctr`,
        /// `utmcct`, …), their values URL-decoded, in order.
        variables: Vec<(String, String)>,
    },
}

/// Decode every Google Analytics cookie among `cookies`; a value of an
/// unexpected shape is reported in `problems` and left undecoded.
pub(crate) fn decode_all(cookies: &mut [Cookie], problems: &mut Vec<String>) {
    for cookie in cookies {
        match decode(&cookie.name, &cookie.value) {
            Some(Ok(decoded)) => cookie.analytics = Some(decoded),
            Some(Err(fields)) => problems.push(format!(
                "cookie {} {}: {fields} fields, not a Google Analytics value",
                cookie.rowid, cookie.name
            )),
            None => {}
        }
    }
}

/// A cookie's value, decoded when its name is a Google Analytics one; the
/// number of fields when the value doesn't fit.
fn decode(name: &str, value: &str) -> Option<Result<GoogleAnalytics, usize>> {
    let fields: Vec<&str> = value.split('.').collect();
    let decoded = match (name, fields.as_slice()) {
        ("__utma", [single]) => GoogleAnalytics::Utma {
            domain_hash: None,
            visitor_id: None,
            first_visit: None,
            previous_visit: None,
            last_visit: ticks(single),
            sessions: None,
        },
        ("__utma", [hash, visitor, first, previous, last, sessions]) => GoogleAnalytics::Utma {
            domain_hash: Some((*hash).to_owned()),
            visitor_id: Some((*visitor).to_owned()),
            first_visit: seconds(first),
            previous_visit: seconds(previous),
            last_visit: seconds(last),
            sessions: integer(sessions),
        },
        ("__utmb", [single]) => GoogleAnalytics::Utmb {
            domain_hash: None,
            pages_viewed: None,
            last_visit: ticks(single),
        },
        ("__utmb", [hash, pages, count, last]) => GoogleAnalytics::Utmb {
            domain_hash: Some((*hash).to_owned()),
            pages_viewed: integer(pages),
            last_visit: if matches!(*count, "8" | "9") || last.len() >= MILLISECOND_DIGITS {
                integer(last).map(Ts::from_unix_millis)
            } else {
                seconds(last)
            },
        },
        ("__utmt", [single]) => GoogleAnalytics::Utmt {
            last_visit: ticks(single),
        },
        ("__utmz", [single]) => GoogleAnalytics::Utmz {
            domain_hash: None,
            last_visit: ticks(single),
            sessions: None,
            sources: None,
            variables: Vec::new(),
        },
        ("__utmz", [hash, last, sessions, sources, _, ..]) => GoogleAnalytics::Utmz {
            domain_hash: Some((*hash).to_owned()),
            last_visit: seconds(last),
            sessions: integer(sessions),
            sources: integer(sources),
            // The variables are what follows the fourth dot, dots and all.
            variables: value
                .splitn(5, '.')
                .nth(4)
                .unwrap_or_default()
                .split('|')
                .map(|variable| {
                    let (key, value) = variable.split_once('=').unwrap_or((variable, ""));
                    (key.to_owned(), percent_decode(value))
                })
                .collect(),
        },
        ("__utma" | "__utmb" | "__utmt" | "__utmz", fields) => return Some(Err(fields.len())),
        _ => return None,
    };
    Some(Ok(decoded))
}

/// The digits from which a `__utmb` time is in milliseconds whatever its
/// count says: in seconds, 10^12 is past the year 30,000.
const MILLISECOND_DIGITS: usize = 13;

/// A decimal integer, as Python's `int` reads one: optional sign, digits.
fn integer(text: &str) -> Option<i64> {
    let digits = text.strip_prefix(['+', '-']).unwrap_or(text);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

fn seconds(text: &str) -> Option<Ts> {
    integer(text).map(Ts::from_unix_seconds)
}

/// 100 nanosecond ticks since 1970.
fn ticks(text: &str) -> Option<Ts> {
    integer(text).map(|ticks| Ts::from_ticks(ticks, Precision::Tick))
}

/// `%XX` escapes decoded, as UTF-8 (invalid sequences replaced); a `%` not
/// followed by two hex digits kept as it is.
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while let Some(&byte) = bytes.get(at) {
        let escaped = (byte == b'%')
            .then(|| bytes.get(at + 1..at + 3))
            .flatten()
            .and_then(|hex| std::str::from_utf8(hex).ok())
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        if let Some(value) = escaped {
            decoded.push(value);
            at += 3;
        } else {
            decoded.push(byte);
            at += 1;
        }
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn campaign_variables_keep_their_dots() {
        let Some(Ok(GoogleAnalytics::Utmz {
            domain_hash,
            sessions,
            variables,
            ..
        })) = decode(
            "__utmz",
            "207318870.1383170190.1.1.utmcsr=google.is|utmctr=enders%20game|utmcmd",
        )
        else {
            panic!("not decoded");
        };
        assert_eq!(domain_hash.as_deref(), Some("207318870"));
        assert_eq!(sessions, Some(1));
        assert_eq!(
            variables,
            [
                ("utmcsr".to_owned(), "google.is".to_owned()),
                ("utmctr".to_owned(), "enders game".to_owned()),
                ("utmcmd".to_owned(), String::new()),
            ]
        );
    }

    #[test]
    fn shapes_that_do_not_fit() {
        assert_eq!(decode("__utma", "1.2.3"), Some(Err(3)));
        assert_eq!(decode("__utmt", "1.2"), Some(Err(2)));
        assert_eq!(decode("SID", "1.2.3"), None);
        assert_eq!(percent_decode("100%25%2"), "100%%2");
    }
}
