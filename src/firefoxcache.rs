//! Firefox's disk cache: version 1 (Firefox 31 and older, the profile's
//! `Cache` folder) and version 2 (Firefox 32 and later, `cache2/entries`):
//! every URL cached, when it was last fetched and modified and when it
//! expires, how many times it was fetched, and the HTTP request method and
//! response headers, even once the history is cleared. Integers are
//! big-endian in both.
//!
//! Version 1 keeps its records in three block files, `_CACHE_001_`,
//! `_CACHE_002_` and `_CACHE_003_`, of 256, 1024 and 4096-byte blocks
//! after an allocation bitmap, indexed by `_CACHE_MAP_` (not read: the
//! records are found by scanning the blocks, which finds those of evicted
//! entries too, as plaso does). A metadata record starts on a block: the
//! format version (major 1, then minor, 16 bits each), the data's location,
//! the fetch count, when it was last fetched, last modified and expires
//! (Unix seconds, 32 bits each; 0 for none), the data's size, the key's size
//! and the elements' size; then the key (`HTTP:http://…`, NUL-terminated)
//! and the elements, NUL-separated names and values (`request-method`,
//! `response-head`, …). It takes as many blocks as it needs.
//!
//! Version 2 keeps an entry per file, named by the SHA-1 of its key: the
//! content, then a hash of the metadata, a 16-bit hash per 512 KiB chunk
//! of content, then the metadata: its version (1 to 3), the fetch count,
//! when it was last fetched and modified, its frecency, when it expires
//! (`0xFFFFFFFF` for never, in version 1 too), the key's size, from version 2 flags; then the
//! key and the elements as in version 1. The file ends with the metadata's
//! offset (32 bits), which is the content's size. A key is the URL after
//! tags that end with a comma (`a,`, `O^partitionKey=…,`,
//! `~predictor-origin,`), the URL marked by a leading `:`.

use common::bytes::Reader;
use common::time::Ts;

use crate::Error;

/// The longest key a record is believed with, as plaso.
const LONGEST_KEY: u32 = 65_536;
/// The size of a version 1 record's fixed part.
const RECORD_HEADER: usize = 36;
/// The smallest block of a version 1 block file.
const SMALLEST_BLOCK: usize = 256;
/// The size of a version 2 content chunk, each hashed in 16 bits.
const CHUNK: usize = 512 * 1024;
/// An expiry meaning never.
const NEVER: u32 = u32::MAX;
/// A FILETIME meaning never, which [`Ts::from_filetime`] makes a sentinel.
const FILETIME_NEVER: u64 = 0x7FFF_FFFF_FFFF_FFFF;

/// A cached URL.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FirefoxCacheEntry {
    /// Where its metadata starts in its file.
    pub offset: usize,
    /// The cache's version: 1 or 2.
    pub cache_version: u8,
    /// The metadata's format version: `1.11` (major.minor) in version 1,
    /// `1` to `3` in version 2.
    pub format_version: String,
    /// Its key, as stored.
    pub key: String,
    /// The URL: the key without its tags or client id.
    pub url: String,
    /// How many times it was fetched.
    pub fetch_count: u32,
    /// When it was last fetched.
    pub last_fetched: Option<Ts>,
    /// When it was last modified.
    pub last_modified: Option<Ts>,
    /// When it expires ([`Semantic::Sentinel`](common::time::Semantic)
    /// for never).
    pub expires: Option<Ts>,
    /// Its frecency (version 2).
    pub frecency: Option<u32>,
    /// The content's size, in bytes.
    pub data_size: u32,
    /// Where its content is (version 1), as stored.
    pub location: Option<u32>,
    /// The size of its key, in bytes, with its NUL in version 1.
    pub key_size: u32,
    /// The size of its elements, in bytes (version 1).
    pub elements_size: Option<u32>,
    /// Its elements (`request-method`, `response-head`, `security-info`,
    /// …), in order.
    pub elements: Vec<(String, String)>,
}

impl FirefoxCacheEntry {
    /// An element's value by its name.
    #[must_use]
    pub fn element(&self, name: &str) -> Option<&str> {
        self.elements
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }

    /// The HTTP request method (`GET`).
    #[must_use]
    pub fn request_method(&self) -> Option<&str> {
        self.element("request-method")
    }

    /// The HTTP response's status line (`HTTP/1.1 200 OK`).
    #[must_use]
    pub fn response_status(&self) -> Option<&str> {
        self.element("response-head")
            .and_then(|head| head.split("\r\n").next())
    }
}

/// The block size of a version 1 block file, from its name.
fn block_size(name: &str) -> usize {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    match base {
        "_CACHE_002_" => 1024,
        "_CACHE_003_" => 4096,
        _ => SMALLEST_BLOCK,
    }
}

/// Every metadata record of a version 1 block file. The blocks follow a
/// bitmap whose size varies (4096 bytes up to Firefox 3; from Firefox 4,
/// 16384, 4096 and 1024 bytes in the three files), so a record is looked
/// for every 256 bytes; `name` (`_CACHE_001_`) gives the block size a
/// record found takes whole blocks of.
pub(crate) fn read_v1(name: &str, data: &[u8]) -> Vec<FirefoxCacheEntry> {
    let block = block_size(name);
    let mut entries = Vec::new();
    let mut at = 0;
    while at < data.len() {
        let found = data.get(at..).and_then(record_v1);
        match found {
            Some((mut entry, size)) => {
                entry.offset = at;
                entries.push(entry);
                at = at.saturating_add(size.div_ceil(block).max(1).saturating_mul(block));
            }
            None => at = at.saturating_add(SMALLEST_BLOCK),
        }
    }
    entries
}

/// The version 1 record at the start of `data`, and its size; `None` when
/// none is there.
fn record_v1(data: &[u8]) -> Option<(FirefoxCacheEntry, usize)> {
    let mut reader = Reader::new(data);
    let major = reader.u16_be().ok()?;
    let minor = reader.u16_be().ok()?;
    let location = reader.u32_be().ok()?;
    let fetch_count = reader.u32_be().ok()?;
    let last_fetched = reader.u32_be().ok()?;
    let last_modified = reader.u32_be().ok()?;
    let expires = reader.u32_be().ok()?;
    let data_size = reader.u32_be().ok()?;
    let key_size = reader.u32_be().ok()?;
    let elements_size = reader.u32_be().ok()?;
    // As plaso tells a record: version 1, a key of a believable size, a
    // fetch count and a time; and here, a key with a client id, ending with
    // its only NUL.
    if major != 1
        || key_size == 0
        || key_size >= LONGEST_KEY
        || last_fetched == 0
        || fetch_count == 0
        || fetch_count > i32::MAX as u32
    {
        return None;
    }
    let key = reader.bytes(key_size as usize).ok()?;
    let (&0, key) = key.split_last()? else {
        return None;
    };
    if key.contains(&0) || !key.contains(&b':') {
        return None;
    }
    let elements = reader.bytes(elements_size as usize).ok()?;
    let key = String::from_utf8_lossy(key).into_owned();
    let entry = FirefoxCacheEntry {
        offset: 0,
        cache_version: 1,
        format_version: format!("{major}.{minor}"),
        url: key
            .split_once(':')
            .map_or(key.as_str(), |(_, url)| url)
            .to_owned(),
        key,
        fetch_count,
        last_fetched: seconds(last_fetched),
        last_modified: seconds(last_modified),
        expires: expiry(expires),
        frecency: None,
        data_size,
        location: Some(location),
        key_size,
        elements_size: Some(elements_size),
        elements: parse_elements(elements),
    };
    Some((
        entry,
        RECORD_HEADER + key_size as usize + elements_size as usize,
    ))
}

/// Read a version 2 entry file.
pub(crate) fn read_v2(data: &[u8]) -> Result<FirefoxCacheEntry, Error> {
    let fail = |why: &str| Error(format!("not a Firefox cache2 entry: {why}"));
    let tail = data
        .len()
        .checked_sub(4)
        .and_then(|at| data.get(at..))
        .ok_or_else(|| fail("too short"))?;
    let content = u32::from_be_bytes([tail[0], tail[1], tail[2], tail[3]]) as usize;
    let chunks = content.div_ceil(CHUNK);
    let offset = chunks
        .checked_mul(2)
        .and_then(|hashes| content.checked_add(hashes))
        .and_then(|at| at.checked_add(4))
        .filter(|&at| at < data.len())
        .ok_or_else(|| fail("metadata offset past the end"))?;
    let mut reader = Reader::new(data);
    let header = reader
        .seek(offset)
        .and_then(|()| MetadataHeader::parse(&mut reader))
        .map_err(|e| fail(&e.to_string()))?;
    let MetadataHeader {
        version,
        fetch_count,
        last_fetched,
        last_modified,
        frecency,
        expires,
        key_size,
    } = header;
    if !(1..=3).contains(&version)
        || key_size == 0
        || key_size >= LONGEST_KEY
        || last_fetched == 0
        || fetch_count == 0
        || fetch_count > i32::MAX as u32
    {
        return Err(fail("metadata out of range"));
    }
    if version >= 2 {
        let _flags = reader.u32_be().map_err(|e| fail(&e.to_string()))?;
    }
    let key = reader
        .bytes(key_size as usize)
        .map_err(|e| fail(&e.to_string()))?;
    let key = String::from_utf8_lossy(key).into_owned();
    // The key's NUL, the elements, and the offset at the end.
    let elements_end = data.len() - 4;
    let elements = data
        .get(reader.position().saturating_add(1)..elements_end)
        .unwrap_or_default();
    Ok(FirefoxCacheEntry {
        offset,
        cache_version: 2,
        format_version: version.to_string(),
        url: tagged_url(&key).to_owned(),
        key,
        fetch_count,
        last_fetched: seconds(last_fetched),
        last_modified: seconds(last_modified),
        expires: expiry(expires),
        frecency: Some(frecency),
        data_size: content as u32,
        location: None,
        key_size,
        elements_size: None,
        elements: parse_elements(elements),
    })
}

/// A version 2 entry's metadata, up to its key.
struct MetadataHeader {
    version: u32,
    fetch_count: u32,
    last_fetched: u32,
    last_modified: u32,
    frecency: u32,
    expires: u32,
    key_size: u32,
}

impl MetadataHeader {
    fn parse(reader: &mut Reader<'_>) -> common::bytes::Result<Self> {
        Ok(Self {
            version: reader.u32_be()?,
            fetch_count: reader.u32_be()?,
            last_fetched: reader.u32_be()?,
            last_modified: reader.u32_be()?,
            frecency: reader.u32_be()?,
            expires: reader.u32_be()?,
            key_size: reader.u32_be()?,
        })
    }
}

/// The URL of a version 2 key: after the tag that starts with `:`.
fn tagged_url(key: &str) -> &str {
    key.match_indices(':')
        .find(|&(at, _)| at == 0 || key.as_bytes().get(at - 1) == Some(&b','))
        .and_then(|(at, _)| key.get(at + 1..))
        .unwrap_or(key)
}

/// NUL-separated names and values; a trailing name without a value
/// dropped.
fn parse_elements(data: &[u8]) -> Vec<(String, String)> {
    let text = String::from_utf8_lossy(data);
    let mut parts = text.split('\0');
    let mut elements = Vec::new();
    while let (Some(name), Some(value)) = (parts.next(), parts.next()) {
        if name.is_empty() {
            break;
        }
        elements.push((name.to_owned(), value.to_owned()));
    }
    elements
}

/// An expiry: Unix seconds, `0xFFFFFFFF` for never.
fn expiry(value: u32) -> Option<Ts> {
    if value == NEVER {
        Some(Ts::from_filetime(FILETIME_NEVER))
    } else {
        seconds(value)
    }
}

fn seconds(value: u32) -> Option<Ts> {
    (value != 0).then(|| Ts::from_unix_seconds(i64::from(value)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_and_their_urls() {
        assert_eq!(tagged_url("a,:http://x/a:b"), "http://x/a:b");
        assert_eq!(
            tagged_url("O^partitionKey=%28https%2Cx.com%29,a,:https://y/"),
            "https://y/"
        );
        assert_eq!(tagged_url(":https://y/"), "https://y/");
        assert_eq!(tagged_url("no url"), "no url");
    }

    #[test]
    fn elements() {
        assert_eq!(
            parse_elements(b"request-method\0GET\0response-head\0HTTP/1.1 200 OK\r\n\0"),
            [
                ("request-method".to_owned(), "GET".to_owned()),
                ("response-head".to_owned(), "HTTP/1.1 200 OK\r\n".to_owned()),
            ]
        );
    }
}
