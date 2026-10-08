//! Arbitrary bytes, and real databases damaged and cut anywhere, read or
//! are refused: never a panic.

mod support;

use proptest::prelude::*;

/// Every kind, from the oldest version to the newest, and databases with
/// deleted records to recover.
const DATABASES: [&str; 22] = [
    "plaso/History",
    "plaso/History-59.0.3071.86",
    "plaso/places.sqlite",
    "plaso/places118.sqlite.gz",
    "plaso/firefox_25_places.sqlite.gz",
    "plaso/WebCacheV01.dat.gz",
    "plaso/downloads.sqlite",
    "synthetic/History",
    "synthetic/places.sqlite",
    "recovery/chromium.db",
    "recovery/places.sqlite",
    "plaso/Cookies.binarycookies",
    "plaso/load_statistics.db.gz",
    "plaso/cookies.db.gz",
    "plaso/global_history.dat",
    "plaso/typed_history.xml",
    "plaso/java.idx",
    "plaso/java_602.idx",
    "plaso/chrome_cache/index.gz",
    "plaso/firefox_cache/firefox3/_CACHE_001_.gz",
    "plaso/firefox_cache/cache2/1F4B3A4FC81FB19C530758231FA54313BE8F6FA2",
    "plaso/firefox_cache/cache2/0EDDF8C091E2FED62E44BEDDDC1723F5BF38FE4F",
];

/// Databases with their logs: a visit only in the log; deletions only in
/// the log, whose older page versions are recovered from.
const LOGGED: [(&str, &str); 3] = [
    ("synthetic/wal/History", "synthetic/wal/History-wal"),
    ("recovery/History", "recovery/History-wal"),
    ("recovery/places.sqlite", "recovery/places.sqlite-wal"),
];

fn read_everything(data: &[u8], wal: &[u8]) {
    let _ = browser::read(data, wal);
    let _ = browser::read_cookies(data, wal);
    let _ = browser::read_autofill(data, wal);
    let _ = browser::read_extension_activity(data, wal);
    let _ = browser::read_preferences(data);
    let _ = browser::read_binary_cookies(data);
    let _ = browser::read_load_statistics(data, wal);
    let _ = browser::read_opera_typed_history(data);
    let _ = browser::read_java_idx(data);
    let _ = browser::read_chrome_cache(data, |_| Some(data));
    let _ = browser::read_firefox_cache1("_CACHE_002_", data);
    let _ = browser::read_firefox_cache2(data);
    for name in [
        "History",
        "_CACHE_001_",
        "0EDDF8C091E2FED62E44BEDDDC1723F5BF38FE4F",
    ] {
        let _ = browser::detect(name, data);
    }
}

/// A Chromium cache's files, as stored.
fn chrome_cache() -> Vec<(String, Vec<u8>)> {
    ["index", "data_0", "data_1", "data_2"]
        .iter()
        .map(|name| {
            let data = support::fixture(&format!("plaso/chrome_cache/{name}.gz"));
            ((*name).to_owned(), data)
        })
        .collect()
}

proptest! {
    #[test]
    fn arbitrary_bytes_never_panic(data in proptest::collection::vec(any::<u8>(), 0..4096)) {
        read_everything(&data, &[]);
    }

    /// A real header with arbitrary pages behind it.
    #[test]
    fn arbitrary_pages_never_panic(tail in proptest::collection::vec(any::<u8>(), 0..8192)) {
        let mut data = support::fixture("synthetic/History")[..100].to_vec();
        data.extend(tail);
        read_everything(&data, &[]);
    }

    #[test]
    fn damaged_databases_never_panic(
        which in 0..DATABASES.len(),
        flips in proptest::collection::vec((any::<usize>(), any::<u8>()), 1..40),
        cut in any::<usize>(),
    ) {
        let mut data = support::fixture(DATABASES[which]);
        for &(at, byte) in &flips {
            let at = at % data.len();
            data[at] = byte;
        }
        data.truncate(1 + cut % data.len());
        read_everything(&data, &[]);
    }

    /// The first bytes of a file of each binary kind, arbitrary bytes
    /// behind them.
    #[test]
    fn arbitrary_bodies_never_panic(
        which in 0usize..4,
        tail in proptest::collection::vec(any::<u8>(), 0..4096),
    ) {
        let (fixture, keep) = [
            ("plaso/Cookies.binarycookies", 8),
            ("plaso/java.idx", 6),
            ("plaso/java_602.idx", 6),
            ("plaso/firefox_cache/firefox3/_CACHE_001_.gz", 4096),
        ][which];
        let mut data = support::fixture(fixture)[..keep].to_vec();
        data.extend(tail);
        read_everything(&data, &[]);
    }

    /// A real index's header, an arbitrary table: addresses of any type,
    /// file and block, chains that loop.
    #[test]
    fn arbitrary_chrome_cache_tables_never_panic(
        table in proptest::collection::vec(any::<u32>(), 0..512),
    ) {
        let files = chrome_cache();
        let mut index = files[0].1[..368].to_vec();
        index.extend(table.iter().flat_map(|a| a.to_le_bytes()));
        let _ = browser::read_chrome_cache(&index, |name| {
            files.iter().find(|(n, _)| n == name).map(|(_, d)| d.as_slice())
        });
    }

    /// The index and the block files damaged and cut.
    #[test]
    fn damaged_chrome_caches_never_panic(
        which in 0usize..4,
        flips in proptest::collection::vec((any::<usize>(), any::<u8>()), 1..200),
        cut in any::<usize>(),
    ) {
        let mut files = chrome_cache();
        let data = &mut files[which].1;
        for &(at, byte) in &flips {
            let at = at % data.len();
            data[at] = byte;
        }
        data.truncate(1 + cut % data.len());
        let (index, others) = files.split_first().unwrap();
        let _ = browser::read_chrome_cache(&index.1, |name| {
            others.iter().find(|(n, _)| n == name).map(|(_, d)| d.as_slice())
        });
    }

    #[test]
    fn damaged_logs_never_panic(
        which in 0..LOGGED.len(),
        flips in proptest::collection::vec((any::<usize>(), any::<u8>()), 1..20),
        cut in any::<usize>(),
    ) {
        let (database, wal) = LOGGED[which];
        let database = support::fixture(database);
        let mut wal = support::fixture(wal);
        for &(at, byte) in &flips {
            let at = at % wal.len();
            wal[at] = byte;
        }
        wal.truncate(cut % (wal.len() + 1));
        read_everything(&database, &wal);
    }
}
