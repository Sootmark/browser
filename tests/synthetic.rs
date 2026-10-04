//! Databases made by `tests/fixtures/synthetic/gen.sh` with the sqlite3
//! shell: recent Chromium's tables with transition qualifiers, redirects,
//! a download's URL chain and an interrupted download; a visit only in the
//! write-ahead log; rows that don't join; Firefox download metadata that
//! isn't JSON. And which database a file is.

mod support;

use std::time::Duration;

use browser::{detect, read, DownloadState, Kind};
use common::time::Ts;

fn fixture(name: &str) -> Vec<u8> {
    support::fixture(&format!("synthetic/{name}"))
}

/// `Ts::to_iso8601`, to the microsecond.
fn micros(ts: Option<Ts>) -> Option<String> {
    ts.and_then(|ts| ts.to_iso8601())
        .map(|iso| iso[..26].to_owned())
}

#[test]
fn chromium_visits_with_qualifiers_and_redirects() {
    let history = read(&fixture("History"), &[]).unwrap();
    assert_eq!(history.kind, Kind::ChromiumHistory);
    let visits: Vec<_> = history
        .visits
        .iter()
        .map(|v| {
            (
                v.id,
                v.url.as_str(),
                v.transition.to_string(),
                v.from_visit,
                micros(v.time),
            )
        })
        .collect();
    let at = |t: &str| Some(format!("2026-09-01T08:{t}"));
    assert_eq!(
        visits,
        [
            (
                1,
                "https://example.com/",
                "TYPED|FROM_ADDRESS_BAR|CHAIN_START|CHAIN_END".to_owned(),
                None,
                at("00:00.000000")
            ),
            (
                2,
                "http://example.org/login",
                "LINK|CHAIN_START".to_owned(),
                Some(1),
                at("00:10.000000")
            ),
            (
                3,
                "https://example.org/login",
                "LINK|CHAIN_END|SERVER_REDIRECT".to_owned(),
                Some(2),
                at("00:10.250000")
            ),
            (
                4,
                "https://example.com/",
                "RELOAD|CHAIN_START|CHAIN_END".to_owned(),
                None,
                at("01:00.000000")
            ),
            // Its page row is gone: kept, without a URL.
            (
                5,
                "",
                "LINK|CHAIN_START|CHAIN_END".to_owned(),
                None,
                at("01:10.000000")
            ),
        ]
    );
    assert_eq!(history.problems, ["visits: row 5's url is not in urls"]);
    let first = &history.visits[0];
    assert_eq!(first.title, "Example Domain");
    assert!(first.typed);
    assert_eq!((first.visit_count, first.typed_count), (Some(2), Some(1)));
    assert_eq!(first.duration, Some(Duration::from_secs(5)));
    assert!(history.visits[1].hidden);
    assert_eq!(history.visits[2].duration, Some(Duration::from_secs(42)));
}

#[test]
fn chromium_downloads_with_url_chains() {
    let history = read(&fixture("History"), &[]).unwrap();
    let [report, setup] = history.downloads.as_slice() else {
        panic!("{:?}", history.downloads)
    };
    // Chain rows written out of order, read by chain_index.
    assert_eq!(
        report.url_chain,
        [
            "http://example.net/r/1",
            "https://example.net/r/2",
            "https://www.example.net/report.pdf"
        ]
    );
    assert_eq!(report.url, "https://www.example.net/report.pdf");
    assert_eq!(report.target_path, "/srv/downloads/report.pdf");
    assert_eq!(
        micros(report.start),
        Some("2026-09-01T08:01:40.000000".to_owned())
    );
    assert_eq!(
        micros(report.end),
        Some("2026-09-01T08:01:41.500000".to_owned())
    );
    assert_eq!(report.state, Some(DownloadState::Complete));
    assert_eq!(report.opened, Some(true));
    assert_eq!(report.referrer.as_deref(), Some("https://example.com/"));
    assert_eq!(report.mime_type.as_deref(), Some("application/pdf"));

    assert_eq!(setup.url, "http://192.0.2.10/setup.exe");
    assert_eq!(
        setup.current_path.as_deref(),
        Some("/srv/downloads/setup.exe.crdownload")
    );
    assert_eq!(setup.state, Some(DownloadState::Interrupted));
    assert_eq!(
        (setup.danger_type, setup.interrupt_reason),
        (Some(1), Some(20))
    );
    assert_eq!(
        (setup.received_bytes, setup.total_bytes),
        (Some(1024), Some(90_000))
    );
    // Not ended: zero, no time.
    assert_eq!(micros(setup.end), None);
}

#[test]
fn visits_only_in_the_log() {
    let database = fixture("wal/History");
    let urls = |wal: &[u8]| -> Vec<String> {
        read(&database, wal)
            .unwrap()
            .visits
            .into_iter()
            .map(|v| v.url)
            .collect()
    };
    assert_eq!(urls(&[]).len(), 5);
    let with_log = urls(&fixture("wal/History-wal"));
    assert_eq!(with_log.len(), 6);
    assert_eq!(with_log[5], "https://www.example.net/report.pdf");
}

#[test]
fn firefox_metadata_that_is_not_json() {
    let history = read(&fixture("places.sqlite"), &[]).unwrap();
    assert_eq!(history.kind, Kind::FirefoxPlaces);
    let [download] = history.downloads.as_slice() else {
        panic!("{:?}", history.downloads)
    };
    // The destination is kept; what the metadata held is not.
    assert_eq!(download.url, "https://example.com/file.zip");
    assert_eq!(download.target_path, "file:///srv/downloads/file.zip");
    assert_eq!(
        micros(download.start),
        Some("2026-09-01T08:00:00.500000".to_owned())
    );
    assert_eq!((download.state, download.end), (None, None));
    assert_eq!(history.visits.len(), 2);
    assert_eq!(history.visits[0].frecency, Some(2000));
    assert!(history.visits[0].typed);
    assert_eq!(history.visits[1].url, "");
    assert_eq!(history.problems.len(), 2, "{:?}", history.problems);
    assert!(history.problems[0].contains("row 2's place_id is not in moz_places"));
    assert!(history.problems[1].starts_with("moz_annos: download 1's metadata: "));
}

#[test]
fn detect_by_tables_then_name() {
    let history = fixture("History");
    let places = fixture("places.sqlite");
    // The tables decide, whatever the name.
    assert_eq!(detect("History", &history), Some(Kind::ChromiumHistory));
    assert_eq!(detect("copy.bin", &history), Some(Kind::ChromiumHistory));
    assert_eq!(detect("History", &places), Some(Kind::FirefoxPlaces));
    // The first page alone: its schema doesn't read, the name tells.
    let head = &history[..512];
    assert_eq!(detect("Default/History", head), Some(Kind::ChromiumHistory));
    assert_eq!(
        detect("C:\\Profile\\places.sqlite", head),
        Some(Kind::FirefoxPlaces)
    );
    assert_eq!(detect("Cookies", head), None);
    // Not SQLite at all.
    assert_eq!(detect("History", b"not a database"), None);
    assert_eq!(
        Kind::from_name("downloads.sqlite"),
        Some(Kind::FirefoxDownloads)
    );
    assert_eq!(Kind::from_name("History-journal"), None);
}

#[test]
fn other_databases_are_refused() {
    assert!(read(b"not a database", &[]).is_err());
    let err = read(&fixture("wal/History-wal"), &[]).unwrap_err();
    assert!(!err.0.is_empty());
}
