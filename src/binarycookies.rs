//! Safari's cookies (`Cookies.binarycookies`, in `~/Library/Cookies` or an
//! app's container), a format of Apple's own whose integers are
//! big-endian in the file's frame and little-endian inside its pages.
//!
//! The file starts with `cook`, the number of pages (big-endian 32 bits)
//! and each page's size (the same), then the pages one after the other,
//! then a checksum (big-endian: for each page, the sum of every fourth of
//! its bytes) and the footer `07 17 20 05`; recent versions add a binary
//! property list after it. A page starts with `00 00 01 00`, then the
//! number of cookies and each cookie's offset in the page (little-endian
//! 32 bits, as everything in a page), then four zero bytes. A cookie is a
//! record: its size, a version, its flags (1 secure, 4 HTTP-only), a
//! has-port flag, the offsets from the record's start of its domain, name,
//! path and value (NUL-terminated strings), the offset of a comment (0 for
//! none) and of a port list, then when it expires and when it was made
//! (Cocoa seconds, 64-bit floats).

use common::bytes::Reader;
use common::time::Ts;

use crate::{Cookie, CookieStore};

/// The file's signature.
pub(crate) const SIGNATURE: &[u8] = b"cook";
/// A page's signature.
const PAGE_SIGNATURE: [u8; 4] = [0, 0, 1, 0];
/// The footer after the checksum.
const FOOTER: [u8; 4] = [0x07, 0x17, 0x20, 0x05];
/// The size of a cookie record's fixed part, up to its strings.
const RECORD_HEADER: usize = 56;
/// Cookie flags.
const SECURE: u32 = 1;
const HTTP_ONLY: u32 = 4;

/// Every cookie of a `Cookies.binarycookies` file, page by page; damage to
/// `problems`.
pub(crate) fn read(data: &[u8], problems: &mut Vec<String>) -> Vec<Cookie> {
    let mut reader = Reader::new(data);
    let page_sizes = match page_sizes(&mut reader) {
        Ok(sizes) => sizes,
        Err(why) => {
            problems.push(why);
            return Vec::new();
        }
    };
    let mut cookies = Vec::new();
    let mut sum = 0u32;
    for (number, size) in page_sizes.into_iter().enumerate() {
        let at = reader.offset();
        let Ok(page) = reader.bytes(size) else {
            problems.push(format!("page {number} at {at}: cut short"));
            return cookies;
        };
        sum = sum.wrapping_add(checksum(page));
        page_cookies(page, at, number, &mut cookies, problems);
    }
    check_footer(&mut reader, sum, problems);
    cookies
}

/// The header's page sizes.
fn page_sizes(reader: &mut Reader<'_>) -> Result<Vec<usize>, String> {
    let header = |e: common::bytes::Error| format!("header: {e}");
    if reader.bytes(SIGNATURE.len()).map_err(header)? != SIGNATURE {
        return Err("no cook signature".to_owned());
    }
    let pages = reader.u32_be().map_err(header)?;
    let count = common::bytes::checked_count(
        u64::from(pages),
        4,
        u64::from(u32::MAX),
        reader.remaining(),
        reader.offset(),
    )
    .map_err(header)?;
    (0..count)
        .map(|_| reader.u32_be().map(|size| size as usize).map_err(header))
        .collect()
}

/// A page's share of the checksum: the sum of every fourth byte.
fn checksum(page: &[u8]) -> u32 {
    page.iter().step_by(4).map(|&b| u32::from(b)).sum()
}

/// The checksum and footer after the pages, when there.
fn check_footer(reader: &mut Reader<'_>, sum: u32, problems: &mut Vec<String>) {
    let (Ok(stored), Ok(footer)) = (reader.u32_be(), reader.array::<4>()) else {
        problems.push("no checksum and footer after the pages".to_owned());
        return;
    };
    if footer != FOOTER {
        problems.push(format!("footer {footer:02x?}, not 07 17 20 05"));
    } else if stored != sum {
        problems.push(format!("checksum {stored:#x}, the pages sum to {sum:#x}"));
    }
}

/// The cookies of a page that starts at `base` in the file.
fn page_cookies(
    page: &[u8],
    base: usize,
    number: usize,
    cookies: &mut Vec<Cookie>,
    problems: &mut Vec<String>,
) {
    let mut reader = Reader::new(page);
    let damaged = |problems: &mut Vec<String>, why: String| {
        problems.push(format!("page {number} at {base}: {why}"));
    };
    match reader.array::<4>() {
        Ok(PAGE_SIGNATURE) => {}
        Ok(other) => return damaged(problems, format!("signature {other:02x?}")),
        Err(e) => return damaged(problems, e.to_string()),
    }
    let offsets = reader
        .u32_le()
        .map_err(|e| e.to_string())
        .and_then(|count| {
            let count =
                common::bytes::checked_count(u64::from(count), 4, u64::MAX, reader.remaining(), 4)
                    .map_err(|e| e.to_string())?;
            (0..count)
                .map(|_| reader.u32_le().map_err(|e| e.to_string()))
                .collect::<Result<Vec<u32>, String>>()
        });
    let offsets = match offsets {
        Ok(offsets) => offsets,
        Err(why) => return damaged(problems, why),
    };
    for offset in offsets {
        match cookie(page, offset as usize, base) {
            Ok(cookie) => cookies.push(cookie),
            Err(why) => damaged(problems, format!("cookie at {offset}: {why}")),
        }
    }
}

/// The cookie whose record starts at `offset` in a page that starts at
/// `base` in the file.
fn cookie(page: &[u8], offset: usize, base: usize) -> Result<Cookie, String> {
    let record = page.get(offset..).ok_or("past the page's end")?;
    let RecordHeader {
        size,
        flags,
        offsets: [domain, name, path, value],
        expires,
        created,
    } = RecordHeader::parse(record).map_err(|e| e.to_string())?;
    if size < RECORD_HEADER {
        return Err(format!("size {size}, shorter than its header"));
    }
    let record = record.get(..size).ok_or("cut short")?;
    let text = |at: u32| string(record, at as usize);
    let cocoa = |seconds: f64| (seconds != 0.0).then(|| Ts::from_cocoa_seconds(seconds));
    Ok(Cookie {
        store: CookieStore::Safari,
        rowid: i64::try_from(base.saturating_add(offset)).unwrap_or(i64::MAX),
        host: text(domain)?,
        name: text(name)?,
        value: text(value)?,
        path: text(path)?,
        created: cocoa(created),
        last_accessed: None,
        expires: cocoa(expires),
        secure: flags & SECURE != 0,
        http_only: flags & HTTP_ONLY != 0,
        persistent: None,
        analytics: None,
    })
}

/// A cookie record's fixed part.
struct RecordHeader {
    size: usize,
    flags: u32,
    /// The offsets of the domain, name, path and value.
    offsets: [u32; 4],
    expires: f64,
    created: f64,
}

impl RecordHeader {
    fn parse(record: &[u8]) -> common::bytes::Result<Self> {
        let mut reader = Reader::new(record);
        let size = reader.u32_le()? as usize;
        let _version = reader.u32_le()?;
        let flags = reader.u32_le()?;
        let _has_port = reader.u32_le()?;
        let mut offsets = [0u32; 4];
        for offset in &mut offsets {
            *offset = reader.u32_le()?;
        }
        let _comment = reader.u32_le()?;
        let _ports = reader.u32_le()?;
        Ok(Self {
            size,
            flags,
            offsets,
            expires: reader.f64_le()?,
            created: reader.f64_le()?,
        })
    }
}

/// The NUL-terminated string at `at` in a record; empty for offset 0, which
/// means none.
fn string(record: &[u8], at: usize) -> Result<String, String> {
    if at == 0 {
        return Ok(String::new());
    }
    let bytes = record
        .get(at..)
        .ok_or_else(|| format!("string at {at}, past the record's end"))?;
    let end = bytes
        .iter()
        .position(|&b| b == 0)
        .ok_or_else(|| format!("string at {at} not terminated"))?;
    Ok(String::from_utf8_lossy(&bytes[..end]).into_owned())
}
