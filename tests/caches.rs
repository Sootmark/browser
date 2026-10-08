//! Chromium's and Firefox's disk caches in plaso's test files (Apache-2.0,
//! `tests/fixtures/plaso/`, see its NOTICE): every event plaso's
//! `chrome_cache`, `firefox_cache` and `firefox_cache2` parsers make, read
//! the same (`tests/oracle/plaso-chrome-cache.tsv.gz` and
//! `plaso-firefox-cache.tsv.gz`, written from plaso's output by
//! `tests/oracle/plaso_tsv.py`; see `tests/oracle/README`).
//!
//! The Chromium caches' `index`, `data_0` (rankings), `data_1` (entries)
//! and `data_2` (keys stored apart) are kept, not `data_3` and the `f_`
//! files, which hold the cached content (and, in `chrome_cache_v3`, one
//! key stored apart). `firefox28/E8D65m01` is a copy of
//! `firefox28/_CACHE_003_` (the same SHA-256), kept once.
//!
//! Where this crate differs from plaso, the test shows how:
//!
//! - plaso reads at most the first 160 bytes of a Chromium key, the inline
//!   part of its first block: a longer key (32 of `chrome_cache`'s, 267 of
//!   `chrome_cache_v3`'s) is cut there, and one stored apart (longer than
//!   927 bytes: 2 and 36) is empty. This crate reads the whole key.
//! - plaso's payloads list the data streams in block files only: those in
//!   separate `f_` files are left out (its loop never appends them).
//! - plaso reads Firefox's expiry `0xFFFFFFFF`, never, as 2106-02-07;
//!   this crate as a sentinel.
//! - dfVFS takes the cache2 entry `0FC75AE8…` for another format and plaso
//!   never parses it; this crate reads it (its two events are left out of
//!   the comparison). `C966EB70…`'s end isn't a metadata offset: neither
//!   reads it.

mod compare;
mod support;

use browser::{detect, ChromeCacheEntry, FirefoxCacheEntry, Kind};
use common::time::{Semantic, Ts};

/// plaso's own reading of `0xFFFFFFFF`, in microseconds.
const PLASO_NEVER: i64 = 4_294_967_295_000_000;
/// The longest key Chromium keeps in an entry's blocks; a longer one is
/// stored apart.
const LONGEST_INLINE_KEY: usize = 4 * 256 - 96 - 1;
/// The inline key's room in an entry's first block, all plaso reads.
const FIRST_BLOCK_KEY: usize = 160;

fn micros(time: Ts) -> i64 {
    if time.semantic() == Semantic::Sentinel {
        return PLASO_NEVER;
    }
    time.ticks().unwrap().div_euclid(10)
}

fn escape(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('\t', "\\t")
        .replace('\r', "\\r")
        .replace('\n', "\\n")
}

fn line(file: &str, data_type: &str, desc: &str, time: Ts, fields: &[String]) -> String {
    let mut values = vec![
        file.to_owned(),
        data_type.to_owned(),
        desc.to_owned(),
        micros(time).to_string(),
    ];
    values.extend(fields.iter().map(|field| escape(field)));
    values.join("\t")
}

fn oracle(name: &str) -> String {
    String::from_utf8(support::fixture(&format!("../oracle/{name}"))).unwrap()
}

/// The URL plaso shows for a key: what of it the first block holds, its
/// partitioning prefix removed.
fn plaso_url(entry: &ChromeCacheEntry) -> String {
    let key = &entry.key;
    let shown = if key.len() > LONGEST_INLINE_KEY {
        ""
    } else {
        key.get(..FIRST_BLOCK_KEY.min(key.len())).unwrap()
    };
    if shown.get(..20).unwrap_or(shown).contains("_dk_") {
        shown.trim().rsplit(' ').next().unwrap().to_owned()
    } else {
        shown.to_owned()
    }
}

fn chrome_lines(lines: &mut Vec<String>) {
    for (folder, entries, problems) in [
        ("chrome_cache", 217, &[][..]),
        (
            "chrome_cache_v3",
            862,
            &[
                "entry 0xa00104d8: key in data_3, not given",
                "data_3: needed, not given",
            ][..],
        ),
    ] {
        let files: Vec<(String, Vec<u8>)> = ["data_0", "data_1", "data_2"]
            .iter()
            .map(|name| {
                let data = support::fixture(&format!("plaso/{folder}/{name}.gz"));
                ((*name).to_owned(), data)
            })
            .collect();
        let index = support::fixture(&format!("plaso/{folder}/index.gz"));
        assert_eq!(detect("index", &index), Some(Kind::ChromeCache));
        let cache = browser::read_chrome_cache(&index, |name| {
            files
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, data)| data.as_slice())
        })
        .unwrap();
        assert_eq!(cache.problems, problems, "{folder}");
        assert_eq!(cache.entries.len(), entries, "{folder}");
        for entry in &cache.entries {
            let payloads: Vec<String> = entry
                .streams
                .iter()
                .filter_map(|stream| {
                    Some(format!(
                        "{} (offset: {:#010x})",
                        stream.file, stream.offset?
                    ))
                })
                .collect();
            lines.push(line(
                &format!("{folder}/index"),
                "chrome:cache:entry",
                "Creation Time",
                entry.created.unwrap(),
                &[plaso_url(entry), payloads.join(" | ")],
            ));
        }
    }
}

#[test]
fn chrome_as_plaso_reads_it() {
    let mut lines = Vec::new();
    chrome_lines(&mut lines);
    let oracle = oracle("plaso-chrome-cache.tsv.gz");
    compare::assert_same_lines(&lines, &oracle.lines().collect::<Vec<_>>());
}

#[test]
fn chrome_keys_times_and_streams() {
    let files: Vec<(String, Vec<u8>)> = ["data_0", "data_1", "data_2"]
        .iter()
        .map(|name| {
            let data = support::fixture(&format!("plaso/chrome_cache/{name}.gz"));
            ((*name).to_owned(), data)
        })
        .collect();
    let index = support::fixture("plaso/chrome_cache/index.gz");
    let cache = browser::read_chrome_cache(&index, |name| {
        files
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, data)| data.as_slice())
    })
    .unwrap();
    assert_eq!(cache.version, "2.1");
    assert_eq!(
        cache.created.unwrap().to_iso8601().unwrap(),
        "2014-04-30T16:44:29.7561230Z"
    );
    let first = &cache.entries[0];
    assert_eq!(
        first.url,
        "https://s.ytimg.com/yts/imgbin/player-common-vfliLfqPT.webp"
    );
    assert_eq!(
        first.last_used.unwrap().to_iso8601().unwrap(),
        "2014-04-30T16:44:36.2579570Z"
    );
    assert_eq!(first.streams.len(), 2);
    assert_eq!(
        (first.streams[1].file.as_str(), first.streams[1].size),
        ("f_000010", 25_960)
    );
    // A key stored apart, in data_2, read whole.
    let long = cache
        .entries
        .iter()
        .filter(|e| e.key.len() > LONGEST_INLINE_KEY);
    assert_eq!(long.count(), 2);
    // Without the files, every entry is reported, nothing read.
    let bare = browser::read_chrome_cache(&index, |_| None).unwrap();
    assert!(bare.entries.is_empty());
    assert_eq!(bare.problems.last().unwrap(), "data_1: needed, not given");
}

/// The cache2 entry plaso never parses.
const UNPARSED: &str = "cache2/0FC75AE83A9F7CAEFD982F3A793D9CDE5166ECD2";

fn firefox_line(file: &str, entry: &FirefoxCacheEntry, lines: &mut Vec<String>) {
    let v1 = entry.cache_version == 1;
    let fields = [
        entry.key.clone(),
        if v1 {
            entry.format_version.clone()
        } else {
            "2".to_owned()
        },
        entry.fetch_count.to_string(),
        entry.frecency.map(|f| f.to_string()).unwrap_or_default(),
        entry.request_method().unwrap_or_default().to_owned(),
        entry.response_status().unwrap_or_default().to_owned(),
        entry.key_size.to_string(),
        entry
            .elements_size
            .map(|s| s.to_string())
            .unwrap_or_default(),
        if v1 {
            entry.data_size.to_string()
        } else {
            String::new()
        },
        entry.location.map(|l| l.to_string()).unwrap_or_default(),
    ];
    for (desc, time) in [
        ("Last Visited Time", entry.last_fetched),
        ("Content Modification Time", entry.last_modified),
        ("Expiration Time", entry.expires),
    ] {
        if let Some(time) = time {
            lines.push(line(file, "firefox:cache:record", desc, time, &fields));
        }
    }
}

#[test]
fn firefox_as_plaso_reads_it() {
    let mut lines = Vec::new();
    for (file, fixture) in [
        ("firefox3/_CACHE_001_", "firefox3/_CACHE_001_.gz"),
        ("firefox3/_CACHE_002_", "firefox3/_CACHE_002_.gz"),
        ("firefox3/_CACHE_003_", "firefox3/_CACHE_003_.gz"),
        ("firefox28/_CACHE_001_", "firefox28/_CACHE_001_.gz"),
        ("firefox28/_CACHE_002_", "firefox28/_CACHE_002_.gz"),
        ("firefox28/_CACHE_003_", "firefox28/_CACHE_003_.gz"),
        ("firefox28/E8D65m01", "firefox28/_CACHE_003_.gz"),
    ] {
        let data = support::fixture(&format!("plaso/firefox_cache/{fixture}"));
        let entries = browser::read_firefox_cache1(file, &data).unwrap();
        assert_eq!(entries.problems, Vec::<String>::new());
        for entry in &entries.rows {
            firefox_line(file, entry, &mut lines);
        }
    }
    let mut read = 0;
    for name in [
        "0EDDF8C091E2FED62E44BEDDDC1723F5BF38FE4F",
        "0FC75AE83A9F7CAEFD982F3A793D9CDE5166ECD2",
        "1F4B3A4FC81FB19C530758231FA54313BE8F6FA2",
        "9E599395B8E39ED759C56FC9CD6BBD80FBB426DC",
        "C966EB70794E44E7E3E8A260106D0C72439AF65B",
    ] {
        let file = format!("cache2/{name}");
        // Stored as is: an entry's content may be gzip.
        let data = std::fs::read(format!(
            "{}/tests/fixtures/plaso/firefox_cache/{file}",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();
        let Ok(entry) = browser::read_firefox_cache2(&data) else {
            assert_eq!(detect(name, &data), None);
            continue;
        };
        assert_eq!(detect(name, &data), Some(Kind::FirefoxCache2));
        read += 1;
        if file == UNPARSED {
            assert_eq!(
                entry.url,
                "https://github.com/log2timeline/plaso/issues/counts"
            );
            continue;
        }
        firefox_line(&file, &entry, &mut lines);
    }
    assert_eq!(read, 4);
    let oracle = oracle("plaso-firefox-cache.tsv.gz");
    compare::assert_same_lines(&lines, &oracle.lines().collect::<Vec<_>>());
}
