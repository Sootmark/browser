//! Java's deployment cache index files (`.idx`, one per file in
//! `Sun\Java\Deployment\cache\6.0\<n>\` or `~/.java/deployment/cache/`):
//! what the Java browser plugin and Java Web Start downloaded (applets,
//! JARs, JNLP files), from where and when, kept after the browser's own
//! history is cleared. Exploit kits delivered their payloads this way.
//!
//! Every integer is big-endian; every string is its length (16 bits) then
//! its bytes. The file starts with a busy and an incomplete flag (a byte
//! each) and the format version (32 bits: 602 to 605, for cache 6.02 to
//! 6.05). Section 1 follows: in 6.02 and up to 6.04 two unknown bytes,
//! then a shortcut-image flag, the content's size (32 bits), when it was
//! last modified on the server and when it expires (Java milliseconds since
//! 1970, 64 bits; 0 for none); from 6.03 also when it was last validated, a
//! known-to-be-signed flag and the sizes of sections 2, 3 and 4. Section 2
//! follows section 1 in 6.02 and starts at offset 128 from 6.03: the
//! resource's version, its URL, a namespace, from 6.03 the server's IP
//! address, then the HTTP response headers (their number, 32 bits, then
//! name and value strings; the status line's name is `<null>`). Sections 3
//! and 4, the JAR's manifest and its signers, aren't read. When it was
//! downloaded is the `date` header's time (RFC 1123).

use common::bytes::Reader;
use common::time::{days_from_civil, Precision, Ts, TICKS_PER_SECOND};

use crate::Error;

/// Where section 2 starts from version 6.03.
const SECTION_2: usize = 128;
/// The status line's header name.
const STATUS: &str = "<null>";

/// What a Java cache index file says of the file it indexes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct JavaCacheEntry {
    /// The format version: 602 to 605.
    pub version: u32,
    /// Being written when the file was copied.
    pub busy: bool,
    /// The download didn't finish.
    pub incomplete: bool,
    /// Where it was downloaded from.
    pub url: String,
    /// The content's size, in bytes.
    pub content_size: u32,
    /// When it was last modified on the server.
    pub modified: Option<Ts>,
    /// When it expires.
    pub expires: Option<Ts>,
    /// When it was last validated against the server (6.03 and later).
    pub validated: Option<Ts>,
    /// When it was downloaded: the `date` header's time.
    pub downloaded: Option<Ts>,
    /// Known to be signed (6.03 and later).
    pub signed: Option<bool>,
    /// The resource's version, when it has one.
    pub resource_version: String,
    /// The server's IP address (6.03 and later).
    pub ip_address: Option<String>,
    /// The HTTP response headers, in order (the status line's name is
    /// `<null>`).
    pub http_headers: Vec<(String, String)>,
    /// Damage met.
    pub problems: Vec<String>,
}

impl JavaCacheEntry {
    /// The HTTP status line (`HTTP/1.1 200 OK`).
    #[must_use]
    pub fn status(&self) -> Option<&str> {
        self.header(STATUS)
    }

    /// A header's value by its name, case ignored.
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.http_headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// Whether `data` starts like a Java cache index file: two flag bytes and a
/// version from 602 to 605.
#[must_use]
pub fn is_java_idx(data: &[u8]) -> bool {
    matches!(data.get(..6), Some([0 | 1, 0 | 1, 0, 0, 2, 0x5a..=0x5d]))
}

/// Read a Java deployment cache index file (`.idx`).
///
/// # Errors
/// When it doesn't start with a supported version, or its URL can't be
/// read.
pub fn read_java_idx(data: &[u8]) -> Result<JavaCacheEntry, Error> {
    if !is_java_idx(data) {
        return Err(Error(
            "not a Java cache index file (6.02 to 6.05)".to_owned(),
        ));
    }
    let mut reader = Reader::new(data);
    let fail = |e: common::bytes::Error| Error(format!("Java cache index: {e}"));
    let busy = reader.u8().map_err(fail)? != 0;
    let incomplete = reader.u8().map_err(fail)? != 0;
    let version = reader.u32_be().map_err(fail)?;
    let mut entry = JavaCacheEntry {
        version,
        busy,
        incomplete,
        ..JavaCacheEntry::default()
    };
    section_1(&mut reader, &mut entry).map_err(fail)?;
    if version >= 603 {
        reader.seek(SECTION_2).map_err(fail)?;
    }
    entry.resource_version = string(&mut reader).map_err(fail)?;
    entry.url = string(&mut reader).map_err(fail)?;
    let _namespace = string(&mut reader).map_err(fail)?;
    if version >= 603 {
        entry.ip_address = Some(string(&mut reader).map_err(fail)?);
    }
    headers(&mut reader, &mut entry);
    if let Some(date) = entry.header("date").map(str::to_owned) {
        entry.downloaded = rfc1123(&date);
        if entry.downloaded.is_none() {
            entry.problems.push(format!("date header {date:?}"));
        }
    }
    Ok(entry)
}

/// Section 1: the content's size and times.
fn section_1(reader: &mut Reader<'_>, entry: &mut JavaCacheEntry) -> common::bytes::Result<()> {
    if entry.version < 605 {
        let _unknown = reader.u16_be()?;
    }
    let _shortcut_image = reader.u8()?;
    entry.content_size = reader.u32_be()?;
    entry.modified = java_time(reader.u64_be()?);
    entry.expires = java_time(reader.u64_be()?);
    if entry.version >= 603 {
        entry.validated = java_time(reader.u64_be()?);
        entry.signed = Some(reader.u8()? != 0);
    }
    Ok(())
}

/// The HTTP headers, as many as announced and present.
fn headers(reader: &mut Reader<'_>, entry: &mut JavaCacheEntry) {
    let count = match reader.u32_be() {
        Ok(count) => count,
        Err(e) => {
            entry.problems.push(format!("HTTP headers: {e}"));
            return;
        }
    };
    for index in 0..count {
        match string(reader).and_then(|name| Ok((name, string(reader)?))) {
            Ok(header) => entry.http_headers.push(header),
            Err(e) => {
                entry
                    .problems
                    .push(format!("HTTP header {index} of {count}: {e}"));
                return;
            }
        }
    }
}

/// A string: its length (16 bits), then its bytes.
fn string(reader: &mut Reader<'_>) -> common::bytes::Result<String> {
    let length = reader.u16_be()?;
    let bytes = reader.bytes(usize::from(length))?;
    Ok(String::from_utf8_lossy(bytes).into_owned())
}

/// Java milliseconds since 1970; 0 for none.
fn java_time(millis: u64) -> Option<Ts> {
    i64::try_from(millis)
        .ok()
        .filter(|&millis| millis != 0)
        .map(Ts::from_unix_millis)
}

/// An RFC 1123 date in GMT (`Wed, 05 May 2010 03:52:31 GMT`).
fn rfc1123(text: &str) -> Option<Ts> {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let (_weekday, rest) = text.split_once(", ")?;
    let mut fields = rest.split_ascii_whitespace();
    let day: u32 = fields.next()?.parse().ok()?;
    let month = fields.next()?;
    let month = MONTHS.iter().position(|&m| m == month)? + 1;
    let year: i64 = fields.next()?.parse().ok()?;
    let mut clock = fields.next()?.split(':').map(str::parse::<i64>);
    let (Some(Ok(hour)), Some(Ok(minute)), Some(Ok(second)), None) =
        (clock.next(), clock.next(), clock.next(), clock.next())
    else {
        return None;
    };
    if fields.next() != Some("GMT")
        || fields.next().is_some()
        || !(1..=31).contains(&day)
        || !(0..24).contains(&hour)
        || !(0..60).contains(&minute)
        || !(0..61).contains(&second)
        || !(1..=9999).contains(&year)
    {
        return None;
    }
    let days = days_from_civil(year, month as u32, day);
    let seconds = days * 86_400 + hour * 3_600 + minute * 60 + second;
    Some(Ts::from_ticks(
        seconds * TICKS_PER_SECOND,
        Precision::Second,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc1123_dates() {
        assert_eq!(
            rfc1123("Sun, 13 Jan 2013 16:22:01 GMT").and_then(|t| t.to_iso8601()),
            Some("2013-01-13T16:22:01.0000000Z".to_owned())
        );
        assert_eq!(rfc1123("Sun, 13 Foo 2013 16:22:01 GMT"), None);
        assert_eq!(rfc1123("Sun, 13 Jan 2013 16:22 GMT"), None);
        assert_eq!(rfc1123("Sun, 13 Jan 2013 16:22:01 PST"), None);
    }
}
