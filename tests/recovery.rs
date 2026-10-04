//! Deleted history recovered (`tests/fixtures/recovery/`, see its
//! `gen.sh`): a Chromium-shaped history whose visits of one time range were
//! cleared, against the rows its generator deleted (`tests/oracle/recovery/`,
//! from `sootmark-sqlite`); Chromium and Firefox databases deleting in their
//! write-ahead log with `secure_delete` on, as the browsers do, against what
//! their generator did. And no deleted entry where none is on disk.

mod support;

use std::collections::HashMap;
use std::time::Duration;

use browser::{
    read, Area, Confidence, Evidence, History, PageSource, PageState, PageTransition,
    RecoveredVisit, Transition, VisitType,
};
use common::json::{self, Json};
use common::time::Ts;

fn fixture(name: &str) -> Vec<u8> {
    support::fixture(&format!("recovery/{name}"))
}

fn history(database: &str, wal: Option<&str>) -> History {
    let wal = wal.map(fixture).unwrap_or_default();
    let history = read(&fixture(database), &wal).unwrap();
    assert!(
        history.problems.is_empty(),
        "{database}: {:?}",
        history.problems
    );
    history
}

/// The rows an oracle file lists, its values `c0`, `c1`, … as written
/// (`"integer:34"`, `"text:…"`, null).
fn oracle(name: &str) -> Vec<Json> {
    let path = format!(
        "{}/tests/oracle/recovery/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    let text = std::fs::read_to_string(path).unwrap();
    json::parse(&text).unwrap().as_array().unwrap().to_vec()
}

fn oracle_integer(row: &Json, column: &str) -> i64 {
    let value = row.get(column).and_then(Json::as_str).unwrap();
    value.strip_prefix("integer:").unwrap().parse().unwrap()
}

fn oracle_text(row: &Json, column: &str) -> String {
    let value = row.get(column).and_then(Json::as_str).unwrap();
    value.strip_prefix("text:").unwrap().to_owned()
}

#[test]
fn chromium_cleared_time_range() {
    let history = history("chromium.db", None);
    // Pages by id, as the generator wrote them before deleting them.
    let urls: HashMap<i64, Json> = oracle("chromium.db/3-urls.json")
        .into_iter()
        .map(|row| (oracle_integer(&row, "c0"), row))
        .collect();
    assert_eq!(urls.len(), 20);
    let visits = oracle("chromium.db/1-visits.json");
    assert_eq!(visits.len(), 61);

    // Every cleared visit, once, found by its time (each one's own).
    assert_eq!(history.deleted_visits.len(), visits.len());
    let mut by_time: HashMap<Ts, &RecoveredVisit> = HashMap::new();
    for recovered in &history.deleted_visits {
        assert!(by_time
            .insert(recovered.visit.time.unwrap(), recovered)
            .is_none());
    }
    let mut sources = HashMap::new();
    for row in &visits {
        let time = Ts::from_webkit_micros(oracle_integer(row, "c2"));
        let recovered = by_time[&time];
        let visit = &recovered.visit;
        let page = oracle_integer(row, "c1");
        // In freeblocks: the rowid, and so the id, lost.
        assert_eq!(visit.id, 0);
        assert_eq!(recovered.provenance.rowid, None);
        assert_eq!(recovered.provenance.lost, ["id"]);
        assert_eq!(recovered.provenance.page_state, PageState::InUse);
        assert_eq!(recovered.provenance.area, Area::Freeblock);
        assert_eq!(recovered.provenance.evidence, Evidence::Shape);
        assert_eq!(recovered.provenance.confidence, Confidence::Medium);
        assert_eq!(
            visit.transition,
            Transition::Chromium(PageTransition(oracle_integer(row, "c5") as u32))
        );
        assert_eq!(
            visit.from_visit,
            Some(oracle_integer(row, "c3")).filter(|&v| v != 0)
        );
        assert_eq!(
            visit.duration,
            Some(Duration::from_micros(oracle_integer(row, "c7") as u64))
        );
        match recovered.page {
            // Page 54 keeps two visits: it is live.
            PageSource::Live => {
                assert_eq!(page, 54);
                assert!(history.visits.iter().any(|v| v.url == visit.url));
            }
            PageSource::Recovered => {
                assert_eq!(visit.url, oracle_text(&urls[&page], "c1"));
                assert_eq!(visit.title, oracle_text(&urls[&page], "c2"));
            }
            PageSource::NotFound => {
                assert_eq!((visit.url.as_str(), visit.title.as_str()), ("", ""));
            }
        }
        *sources.entry(recovered.page).or_insert(0) += 1;
    }
    // The pages recovered whole (12 of 20) give their visits a URL; the
    // others lost their rowid to a freeblock header, so no visit joins.
    assert_eq!(
        (
            sources[&PageSource::Live],
            sources[&PageSource::Recovered],
            sources[&PageSource::NotFound]
        ),
        (1, 36, 24)
    );

    // Every deleted page, with its counts and last visit.
    assert_eq!(history.deleted_pages.len(), urls.len());
    for recovered in &history.deleted_pages {
        let page = &recovered.page;
        let row = urls
            .values()
            .find(|row| oracle_text(row, "c1") == page.url)
            .unwrap();
        assert_eq!(page.title, oracle_text(row, "c2"));
        assert_eq!(page.visit_count, Some(oracle_integer(row, "c3")));
        assert_eq!(page.typed_count, Some(oracle_integer(row, "c4")));
        assert_eq!(
            page.last_visit,
            Some(Ts::from_webkit_micros(oracle_integer(row, "c5")))
        );
        match recovered.provenance.rowid {
            Some(rowid) => assert_eq!((page.id, rowid), (oracle_integer(row, "c0"), page.id)),
            None => assert_eq!(page.id, 0),
        }
    }
    let whole = history
        .deleted_pages
        .iter()
        .filter(|p| p.provenance.rowid.is_some())
        .count();
    assert_eq!(whole, 12);
}

/// A recovered visit as (id, URL, time in seconds from the fixture's
/// start, where its page came from, where it was found).
fn summary(recovered: &RecoveredVisit, start: Ts) -> (i64, &str, i64, PageSource, PageState) {
    let ticks = |ts: Ts| ts.ticks().unwrap();
    let seconds = (ticks(recovered.visit.time.unwrap()) - ticks(start)) / 10_000_000;
    (
        recovered.visit.id,
        recovered.visit.url.as_str(),
        seconds,
        recovered.page,
        recovered.provenance.page_state,
    )
}

#[test]
fn chromium_deleted_in_the_log() {
    // As checkpointed: nothing deleted yet.
    let before = history("History", None);
    assert_eq!(before.visits.len(), 8);
    assert!(before.deleted_visits.is_empty() && before.deleted_pages.is_empty());
    assert!(before.deleted_downloads.is_empty());

    let history = history("History", Some("History-wal"));
    assert_eq!(history.visits.len(), 3);
    let start = Ts::from_webkit_micros(13_432_723_200_000_000);
    let visits: Vec<_> = history
        .deleted_visits
        .iter()
        .map(|v| summary(v, start))
        .collect();
    let superseded = PageState::Superseded { frame: 0 };
    let forgotten = "https://forgotten.example/account";
    // Visit 2, whose duration the log changed, is live: not deleted.
    assert_eq!(
        visits,
        [
            (8, forgotten, 700, PageSource::Recovered, superseded),
            (7, forgotten, 600, PageSource::Recovered, superseded),
            (
                6,
                "https://forgotten.example/",
                500,
                PageSource::Recovered,
                superseded
            ),
            (
                4,
                "https://example.org/news",
                300,
                PageSource::Live,
                superseded
            ),
            (
                3,
                "https://example.org/news",
                200,
                PageSource::Live,
                superseded
            ),
        ]
    );
    for recovered in &history.deleted_visits {
        let provenance = &recovered.provenance;
        assert_eq!(provenance.table, "visits");
        assert_eq!(provenance.rowid, Some(recovered.visit.id));
        assert_eq!(
            (provenance.area, provenance.evidence, provenance.confidence),
            (Area::Cell, Evidence::CellPointer, Confidence::High)
        );
        assert!(provenance.lost.is_empty() && !provenance.truncated);
    }
    let typed = &history.deleted_visits[2].visit;
    assert_eq!(typed.title, "Forgotten home");
    assert!(typed.typed);
    assert_eq!(typed.duration, Some(Duration::from_secs(10)));

    // The forgotten site's pages, from the file's copy of their page; the
    // news page's older visit count is not a deleted page.
    let pages: Vec<_> = history
        .deleted_pages
        .iter()
        .map(|p| {
            (
                p.page.id,
                p.page.url.as_str(),
                p.page.visit_count,
                p.provenance.page_state,
            )
        })
        .collect();
    assert_eq!(
        pages,
        [
            (5, forgotten, Some(2), PageState::ReplacedInFile),
            (
                4,
                "https://forgotten.example/",
                Some(1),
                PageState::ReplacedInFile
            ),
        ]
    );

    // The download, with its URL chain, from recovered chain rows.
    let [deleted] = history.deleted_downloads.as_slice() else {
        panic!("{:?}", history.deleted_downloads)
    };
    let download = &deleted.download;
    assert_eq!(download.id, 2);
    assert_eq!(
        download.url_chain,
        [
            "https://forgotten.example/get/tool",
            "https://cdn.forgotten.example/tool.exe"
        ]
    );
    assert_eq!(download.url, "https://cdn.forgotten.example/tool.exe");
    assert_eq!(download.target_path, "/srv/downloads/tool.exe");
    assert_eq!(
        download.referrer.as_deref(),
        Some("https://forgotten.example/account")
    );
    assert_eq!(deleted.provenance.page_state, PageState::ReplacedInFile);
    assert_eq!(history.downloads.len(), 1);
}

#[test]
fn firefox_deleted_in_the_log_and_before() {
    // As checkpointed: the two visits deleted before, in freeblocks.
    let before = history("places.sqlite", None);
    let start = Ts::from_unix_micros(1_788_249_600_000_000);
    let visits: Vec<_> = before
        .deleted_visits
        .iter()
        .map(|v| summary(v, start))
        .collect();
    let news = "https://example.org/news";
    assert_eq!(
        visits,
        [
            (0, news, 450, PageSource::Live, PageState::InUse),
            (0, news, 400, PageSource::Live, PageState::InUse),
        ]
    );
    assert!(before.deleted_pages.is_empty());

    let history = history("places.sqlite", Some("places.sqlite-wal"));
    assert_eq!(history.visits.len(), 2);
    let visits: Vec<_> = history
        .deleted_visits
        .iter()
        .map(|v| summary(v, start))
        .collect();
    let forgotten = "https://forgotten.example/";
    let file = PageState::ReplacedInFile;
    assert_eq!(
        visits,
        [
            // Recorded and deleted in one transaction: only in the page as
            // it reads now. The second's rowid is under a freeblock header.
            (9, news, 650, PageSource::Live, PageState::InUse),
            (0, news, 600, PageSource::Live, PageState::InUse),
            (
                2,
                "https://example.com/",
                100,
                PageSource::Live,
                PageState::Superseded { frame: 0 }
            ),
            // In the file's copy of the page: freeblocks of the deletion
            // before the checkpoint, and the forgotten site's cells.
            (0, news, 450, PageSource::Live, file),
            (0, news, 400, PageSource::Live, file),
            (4, forgotten, 300, PageSource::Recovered, file),
            (3, forgotten, 200, PageSource::Recovered, file),
        ]
    );
    let evidence: Vec<_> = history
        .deleted_visits
        .iter()
        .map(|v| (v.provenance.area, v.provenance.evidence))
        .collect();
    assert_eq!(
        evidence,
        [
            (Area::Freeblock, Evidence::Cell),
            (Area::Freeblock, Evidence::Shape),
            (Area::Cell, Evidence::CellPointer),
            (Area::Freeblock, Evidence::Shape),
            (Area::Freeblock, Evidence::Shape),
            (Area::Cell, Evidence::CellPointer),
            (Area::Cell, Evidence::CellPointer),
        ]
    );
    let typed = &history.deleted_visits[6].visit;
    assert_eq!(typed.transition, Transition::Firefox(VisitType::Typed));
    assert_eq!(
        (typed.title.as_str(), typed.frecency),
        ("Forgotten home", Some(900))
    );

    // The forgotten site only: the older frecency and visit counts of the
    // pages kept are not deleted pages.
    let [page] = history.deleted_pages.as_slice() else {
        panic!("{:?}", history.deleted_pages)
    };
    assert_eq!((page.page.id, page.page.url.as_str()), (3, forgotten));
    assert_eq!(page.page.visit_count, Some(2));
    assert!(page.page.typed);
    assert_eq!(
        page.page.last_visit,
        Some(Ts::from_unix_micros(1_788_249_900_000_000))
    );
    assert_eq!(page.provenance.table, "moz_places");
}

/// plaso's files and the synthetic ones were written with their freed
/// space zeroed (freeblocks and the one freelist page all zero; what isn't
/// zero between cell pointers and cells is stale cell pointers): nothing
/// deleted is on disk, and nothing is reported.
#[test]
fn nothing_deleted_where_nothing_is_on_disk() {
    for name in [
        "plaso/History",
        "plaso/History-57.0.2987.133",
        "plaso/History-58.0.3029.96",
        "plaso/History-59.0.3071.86",
        "plaso/History-59_added-fake-column",
        "plaso/places.sqlite",
        "plaso/places118.sqlite.gz",
        "plaso/firefox_25_places.sqlite.gz",
        "plaso/downloads.sqlite",
        "synthetic/History",
        "synthetic/places.sqlite",
        "synthetic/wal/History",
    ] {
        let history = read(&support::fixture(name), &[]).unwrap();
        assert!(history.deleted_visits.is_empty(), "{name}");
        assert!(history.deleted_pages.is_empty(), "{name}");
        assert!(history.deleted_downloads.is_empty(), "{name}");
        assert!(
            !history.problems.iter().any(|p| p.starts_with("recovery:")),
            "{name}: {:?}",
            history.problems
        );
    }
    // Nor from the log: the visit it adds is live.
    let history = read(
        &support::fixture("synthetic/wal/History"),
        &support::fixture("synthetic/wal/History-wal"),
    )
    .unwrap();
    assert!(history.deleted_visits.is_empty() && history.deleted_pages.is_empty());
}

#[test]
fn webcache_is_not_recovered() {
    let history = read(&support::fixture("plaso/WebCacheV01.dat.gz"), &[]).unwrap();
    assert!(!history.visits.is_empty());
    assert!(history.deleted_visits.is_empty() && history.deleted_pages.is_empty());
}
