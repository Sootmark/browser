//! Chromium's disk cache in its block-file format (the profile's `Cache`
//! folder, `Cache_Data` since Chrome 87, and the `GPUCache`, media and
//! application caches; on Linux and Android Chromium uses the "simple"
//! format instead, which isn't read here): every URL the browser cached,
//! when it was first cached and last used, even once the history is
//! cleared.
//!
//! The folder holds `index`, the block files `data_0` to `data_3`, and
//! `f_000001`… for data too big for a block. All integers are
//! little-endian. A cache address (32 bits) names where a record is: bit 31
//! set when in use; bits 28 to 30 the file type (0 a separate file `f_`
//! numbered by bits 0 to 27; 1 the rankings blocks of 36 bytes; 2, 3, 4
//! blocks of 256, 1024 and 4096 bytes); for a block file, bits 24 and 25
//! the number of blocks less one, bits 16 to 23 the file's number (`data_<n>`)
//! and bits 0 to 15 the first block.
//!
//! `index` starts with a 368-byte header: its magic `C3 CA 03 C1`, its
//! version (minor then major, 16 bits each: 2.0, 2.1, 3.0), the number of
//! entries, the bytes stored, the last file made, its id, the address of
//! the statistics, the length of the table (`table_len`), a crash flag, an
//! experiment, when the cache was made (`WebKit` microseconds), then
//! padding and the eviction lists' heads and tails. The table follows:
//! `table_len` addresses, an entry's by the hash of its key, colliding
//! entries chained by their `next` address.
//!
//! A block file starts with an 8192-byte header (magic `C3 CA 04 C1`,
//! version, its number, the next file of its kind, the block size, counts
//! and its allocation bitmap); block `n` is at `8192 + n × block size`. An
//! entry takes one to four 256-byte blocks: its key's hash, the next entry
//! of its chain, its rankings node, its reuse and refetch counts, its state
//! (0 normal, 1 evicted, 2 doomed), when it was made (`WebKit`
//! microseconds), its key's length and the address of a key stored apart,
//! the sizes and addresses of its four data streams (0 the HTTP response
//! headers, 1 the content, 2 metadata), flags (1 parent, 2 child of a
//! sparse entry), a hash of itself, then its key, inline from offset 96
//! through its blocks when it isn't stored apart. Its rankings node (in
//! `data_0`) holds when it was last used and last modified (`WebKit`
//! microseconds). A key is the URL, prefixed since Chrome 77 with
//! `_dk_<site> <site> ` when the cache is partitioned by site (and by
//! `1/0/` for some requests).

use std::collections::HashSet;

use common::bytes::Reader;
use common::time::Ts;

use crate::Error;

const INDEX_MAGIC: u32 = 0xc103_cac3;
const BLOCK_FILE_MAGIC: u32 = 0xc104_cac3;
/// Where the index's table starts.
const INDEX_HEADER: usize = 368;
/// Where a block file's first block starts.
const BLOCK_FILE_HEADER: usize = 8192;
/// The size of an entry's first block, and where its inline key starts.
const ENTRY_BLOCK: usize = 256;
const INLINE_KEY: usize = 96;
/// The most entries a chain is followed for before it is taken for a loop.
const LONGEST_CHAIN: usize = 4096;
/// The longest key read.
const LONGEST_KEY: usize = 1 << 20;

/// A Chromium block-file cache: its header and entries.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChromeCache {
    /// The index's version (`2.1`, `3.0`).
    pub version: String,
    /// When the cache was made.
    pub created: Option<Ts>,
    /// The entries, in index and chain order.
    pub entries: Vec<ChromeCacheEntry>,
    /// Damage met, and files needed but not given.
    pub problems: Vec<String>,
}

/// Where a data stream of an entry is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheStream {
    /// Which: 0 the HTTP response headers, 1 the content, 2 metadata.
    pub index: usize,
    /// Its size, in bytes.
    pub size: u32,
    /// The file it is in (`data_3`, `f_00001a`).
    pub file: String,
    /// Where it starts in that file; `None` for a separate file, whole.
    pub offset: Option<usize>,
}

/// A cached URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChromeCacheEntry {
    /// Its cache address.
    pub address: u32,
    /// Its key, as stored.
    pub key: String,
    /// The URL: the key without its partitioning prefix.
    pub url: String,
    /// When it was first cached.
    pub created: Option<Ts>,
    /// When it was last used, from its rankings node.
    pub last_used: Option<Ts>,
    /// When it was last modified, from its rankings node.
    pub last_modified: Option<Ts>,
    /// Its state: 0 normal, 1 evicted, 2 doomed.
    pub state: u32,
    /// How many times it was reused.
    pub reuse_count: u32,
    /// How many times it was fetched again.
    pub refetch_count: u32,
    /// Its flags: 1 parent, 2 child of a sparse entry.
    pub flags: u32,
    /// Its data streams that have data.
    pub streams: Vec<CacheStream>,
}

/// A decoded cache address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Address {
    /// A separate file, `f_<number>`.
    Separate(u32),
    /// Blocks of a block file.
    Blocks {
        file: u8,
        block_size: usize,
        first: usize,
        count: usize,
    },
}

impl Address {
    fn decode(raw: u32) -> Option<Self> {
        if raw == 0 {
            return None;
        }
        let block_size = match (raw >> 28) & 7 {
            0 => return Some(Self::Separate(raw & 0x0fff_ffff)),
            1 => 36,
            2 => 256,
            3 => 1024,
            4 => 4096,
            _ => return None,
        };
        Some(Self::Blocks {
            file: ((raw >> 16) & 0xff) as u8,
            block_size,
            first: (raw & 0xffff) as usize,
            count: ((raw >> 24) & 3) as usize + 1,
        })
    }

    fn file(self) -> String {
        match self {
            Self::Separate(number) => format!("f_{number:06x}"),
            Self::Blocks { file, .. } => format!("data_{file}"),
        }
    }
}

/// Whether `data` is a cache's `index` (its magic).
pub(crate) fn is_index(data: &[u8]) -> bool {
    data.get(..4) == Some(&INDEX_MAGIC.to_le_bytes()[..])
}

/// Read a block-file cache from its `index` and its other files, given by
/// name (`data_1`, `f_00001a`) by `file`; a file `file` doesn't give is
/// reported in `problems` where needed.
pub(crate) fn read<'f>(
    index: &[u8],
    file: impl Fn(&str) -> Option<&'f [u8]>,
) -> Result<ChromeCache, Error> {
    let mut reader = Reader::new(index);
    let header = |e: common::bytes::Error| Error(format!("index header: {e}"));
    if reader.u32_le().map_err(header)? != INDEX_MAGIC {
        return Err(Error("not a Chromium cache index".to_owned()));
    }
    let minor = reader.u16_le().map_err(header)?;
    let major = reader.u16_le().map_err(header)?;
    reader.seek(28).map_err(header)?;
    let table_len = reader.u32_le().map_err(header)? as usize;
    reader.seek(40).map_err(header)?;
    let created = webkit(reader.u64_le().map_err(header)?);
    let mut cache = ChromeCache {
        version: format!("{major}.{minor}"),
        created,
        ..ChromeCache::default()
    };
    let table = index.get(INDEX_HEADER..).unwrap_or_default();
    let present = table.len() / 4;
    if table_len > present {
        cache.problems.push(format!(
            "index: table of {table_len} addresses, {present} present"
        ));
    }
    let mut files = Files {
        file,
        missing: Vec::new(),
    };
    let mut seen = HashSet::new();
    for slot in table.chunks_exact(4).take(table_len) {
        let mut address = u32::from_le_bytes([slot[0], slot[1], slot[2], slot[3]]);
        let mut length = 0;
        while address != 0 {
            if !seen.insert(address) || length == LONGEST_CHAIN {
                cache
                    .problems
                    .push(format!("entry {address:#010x}: met twice, chain cut"));
                break;
            }
            length += 1;
            match entry(address, &mut files, &mut cache.problems) {
                Ok((entry, next)) => {
                    cache.entries.push(entry);
                    address = next;
                }
                Err(why) => {
                    cache.problems.push(format!("entry {address:#010x}: {why}"));
                    break;
                }
            }
        }
    }
    cache.problems.extend(
        files
            .missing
            .into_iter()
            .map(|name| format!("{name}: needed, not given")),
    );
    Ok(cache)
}

/// The cache's files, by name, and those asked for but not given.
struct Files<F> {
    file: F,
    missing: Vec<String>,
}

impl<'f, F: Fn(&str) -> Option<&'f [u8]>> Files<F> {
    /// The bytes an address names: its blocks, or a separate file whole.
    fn bytes(&mut self, address: Address) -> Result<&'f [u8], String> {
        let name = address.file();
        let Some(data) = (self.file)(&name) else {
            if !self.missing.contains(&name) {
                self.missing.push(name.clone());
            }
            return Err(format!("in {name}, not given"));
        };
        let Address::Blocks {
            block_size,
            first,
            count,
            ..
        } = address
        else {
            return Ok(data);
        };
        let mut reader = Reader::new(data);
        if reader.u32_le().ok() != Some(BLOCK_FILE_MAGIC) {
            return Err(format!("{name} is not a block file"));
        }
        let start = first
            .checked_mul(block_size)
            .and_then(|at| at.checked_add(BLOCK_FILE_HEADER));
        let end = count
            .checked_mul(block_size)
            .and_then(|size| start?.checked_add(size));
        start
            .zip(end)
            .and_then(|(start, end)| data.get(start..end))
            .ok_or_else(|| format!("past the end of {name}"))
    }
}

/// An entry's fixed fields, up to its inline key.
struct EntryStore {
    next: u32,
    rankings: u32,
    reuse_count: u32,
    refetch_count: u32,
    state: u32,
    created: u64,
    /// The key's length, and the address of a key stored apart.
    key: (u32, u32),
    stream_sizes: [u32; 4],
    stream_addresses: [u32; 4],
    flags: u32,
}

impl EntryStore {
    fn parse(blocks: &[u8]) -> common::bytes::Result<Self> {
        let mut reader = Reader::new(blocks);
        let _hash = reader.u32_le()?;
        let next = reader.u32_le()?;
        let rankings = reader.u32_le()?;
        let reuse_count = reader.u32_le()?;
        let refetch_count = reader.u32_le()?;
        let state = reader.u32_le()?;
        let created = reader.u64_le()?;
        let key = (reader.u32_le()?, reader.u32_le()?);
        let mut stream_sizes = [0u32; 4];
        for size in &mut stream_sizes {
            *size = reader.u32_le()?;
        }
        let mut stream_addresses = [0u32; 4];
        for address in &mut stream_addresses {
            *address = reader.u32_le()?;
        }
        Ok(Self {
            next,
            rankings,
            reuse_count,
            refetch_count,
            state,
            created,
            key,
            stream_sizes,
            stream_addresses,
            flags: reader.u32_le()?,
        })
    }
}

/// The entry at `address`, and the next of its chain; a key that can't be
/// read is reported in `problems` and left empty.
fn entry<'f, F: Fn(&str) -> Option<&'f [u8]>>(
    raw: u32,
    files: &mut Files<F>,
    problems: &mut Vec<String>,
) -> Result<(ChromeCacheEntry, u32), String> {
    let address = Address::decode(raw).ok_or("not a block address")?;
    if !matches!(address, Address::Blocks { block_size, .. } if block_size == ENTRY_BLOCK) {
        return Err("not in a 256-byte block file".to_owned());
    }
    let blocks = files.bytes(address)?;
    let store = EntryStore::parse(blocks).map_err(|e| e.to_string())?;
    let key = entry_key(blocks, store.key, files).unwrap_or_else(|why| {
        problems.push(format!("entry {raw:#010x}: {why}"));
        String::new()
    });
    let (last_used, last_modified) = rankings_times(store.rankings, files);
    let streams = (0..4)
        .zip(store.stream_sizes.into_iter().zip(store.stream_addresses))
        .filter_map(|(index, (size, raw))| {
            let address = Address::decode(raw)?;
            let offset = match address {
                Address::Separate(_) => None,
                Address::Blocks {
                    block_size, first, ..
                } => Some(BLOCK_FILE_HEADER.saturating_add(first.saturating_mul(block_size))),
            };
            Some(CacheStream {
                index,
                size,
                file: address.file(),
                offset,
            })
        })
        .collect();
    Ok((
        ChromeCacheEntry {
            address: raw,
            url: url(&key).to_owned(),
            key,
            created: webkit(store.created),
            last_used,
            last_modified,
            state: store.state,
            reuse_count: store.reuse_count,
            refetch_count: store.refetch_count,
            flags: store.flags,
            streams,
        },
        store.next,
    ))
}

/// An entry's key: inline through its blocks, or stored apart.
fn entry_key<'f, F: Fn(&str) -> Option<&'f [u8]>>(
    blocks: &[u8],
    (length, long_key): (u32, u32),
    files: &mut Files<F>,
) -> Result<String, String> {
    let length = length as usize;
    if length > LONGEST_KEY {
        return Err(format!("key of {length} bytes"));
    }
    let stored = match Address::decode(long_key) {
        Some(address) => files.bytes(address).map_err(|why| format!("key {why}"))?,
        None => blocks.get(INLINE_KEY..).unwrap_or_default(),
    };
    let key = stored
        .get(..length)
        .ok_or_else(|| format!("key of {length} bytes, {} stored", stored.len()))?;
    Ok(String::from_utf8_lossy(key).into_owned())
}

/// When an entry was last used and modified, from its rankings node; none
/// when the node can't be read.
fn rankings_times<'f, F: Fn(&str) -> Option<&'f [u8]>>(
    raw: u32,
    files: &mut Files<F>,
) -> (Option<Ts>, Option<Ts>) {
    let node = Address::decode(raw).and_then(|address| files.bytes(address).ok());
    let mut reader = Reader::new(node.unwrap_or_default());
    match (reader.u64_le(), reader.u64_le()) {
        (Ok(used), Ok(modified)) => (webkit(used), webkit(modified)),
        _ => (None, None),
    }
}

/// The URL of a key: what follows the partitioning prefix
/// (`_dk_<site> <site> `), as plaso takes it.
fn url(key: &str) -> &str {
    let head = key.get(..20).unwrap_or(key);
    if head.contains("_dk_") {
        key.trim().rsplit(' ').next().unwrap_or(key)
    } else {
        key
    }
}

fn webkit(micros: u64) -> Option<Ts> {
    i64::try_from(micros)
        .ok()
        .filter(|&micros| micros != 0)
        .map(Ts::from_webkit_micros)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses() {
        assert_eq!(
            Address::decode(0xc103_005a),
            Some(Address::Blocks {
                file: 3,
                block_size: 4096,
                first: 0x5a,
                count: 2,
            })
        );
        assert_eq!(
            Address::decode(0x8000_0010).map(Address::file).as_deref(),
            Some("f_000010")
        );
        assert_eq!(Address::decode(0), None);
        assert_eq!(Address::decode(0xd000_0000), None);
    }

    #[test]
    fn partitioned_keys() {
        assert_eq!(
            url("1/0/_dk_https://a.com https://a.com https://b.com/x.png"),
            "https://b.com/x.png"
        );
        assert_eq!(url("https://b.com/a b"), "https://b.com/a b");
    }
}
