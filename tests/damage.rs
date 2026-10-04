//! Arbitrary bytes, and real databases damaged and cut anywhere, read or
//! are refused: never a panic.

mod support;

use proptest::prelude::*;

/// Every kind, from the oldest version to the newest.
const DATABASES: [&str; 8] = [
    "plaso/History",
    "plaso/History-59.0.3071.86",
    "plaso/places.sqlite",
    "plaso/places118.sqlite.gz",
    "plaso/firefox_25_places.sqlite.gz",
    "plaso/downloads.sqlite",
    "synthetic/History",
    "synthetic/places.sqlite",
];

fn read_everything(data: &[u8], wal: &[u8]) {
    let _ = browser::read(data, wal);
    let _ = browser::detect("History", data);
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

    #[test]
    fn damaged_logs_never_panic(
        flips in proptest::collection::vec((any::<usize>(), any::<u8>()), 1..20),
        cut in any::<usize>(),
    ) {
        let database = support::fixture("synthetic/wal/History");
        let mut wal = support::fixture("synthetic/wal/History-wal");
        for &(at, byte) in &flips {
            let at = at % wal.len();
            wal[at] = byte;
        }
        wal.truncate(cut % (wal.len() + 1));
        read_everything(&database, &wal);
    }
}
