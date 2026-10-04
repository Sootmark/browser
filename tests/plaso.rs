//! plaso's browser test files (Apache-2.0, `tests/fixtures/plaso/`, see
//! its NOTICE), against what plaso's own tests expect of them
//! (`tests/parsers/sqlite_plugins/chrome_history.py`, `firefox_history.py`,
//! `firefox_downloads.py`) and, field by field, against the `sqlite3` shell
//! reading the same files (`tests/oracle/`).

mod support;

use browser::{
    read, CoreTransition, DownloadState, History, Kind, PageTransition, Qualifier, Transition,
    VisitType,
};
use common::time::Ts;

fn fixture(name: &str) -> Vec<u8> {
    support::fixture(&format!("plaso/{name}"))
}

fn history(name: &str) -> History {
    let history = read(&fixture(name), &[]).unwrap();
    assert!(
        history.problems.is_empty(),
        "{name}: {:?}",
        history.problems
    );
    history
}

/// `2011-04-07T12:03:11.000000` from `Ts::to_iso8601`'s seven digits:
/// plaso's form, without its `+00:00`.
fn micros(ts: Option<Ts>) -> String {
    let iso = ts.and_then(|ts| ts.to_iso8601()).unwrap();
    iso[..26].to_owned()
}

#[test]
fn chrome_8_visits_and_downloads() {
    let history = history("History");
    assert_eq!(history.kind, Kind::ChromiumHistory);
    // plaso: 71 events, 69 visits then 2 downloads.
    assert_eq!((history.visits.len(), history.downloads.len()), (69, 2));

    let first = &history.visits[0];
    assert_eq!(micros(first.time), "2011-04-07T12:03:11.000000");
    assert_eq!(first.url, "http://start.ubuntu.com/10.04/Google/");
    assert_eq!(first.title, "Ubuntu Start Page");
    assert_eq!((first.visit_count, first.typed_count), (Some(4), Some(0)));
    assert!(!first.hidden && !first.typed);
    let Transition::Chromium(transition) = first.transition else {
        panic!("{:?}", first.transition)
    };
    // plaso keeps the core type only: 0.
    assert_eq!(transition.core(), CoreTransition::Link);
    assert_eq!(
        transition.qualifiers().collect::<Vec<_>>(),
        [Qualifier::ChainStart, Qualifier::ChainEnd]
    );

    let hidden = &history.visits[20];
    assert_eq!(hidden.url, "http://www.google.ch/blank.html");
    assert!(hidden.hidden);

    let download = &history.downloads[0];
    assert_eq!(download.target_path, "/home/john/Downloads/funcats_scr.exe");
    assert_eq!(
        download.url,
        "http://fatloss4idiotsx.com/download/funcats/funcats_scr.exe"
    );
    // Unix seconds before history version 24.
    assert_eq!(
        download.start.unwrap().to_iso8601().unwrap(),
        "2011-05-23T08:35:30.0000000Z"
    );
    assert_eq!(download.end, None);
    assert_eq!(
        (download.received_bytes, download.total_bytes),
        (Some(1_132_155), Some(1_132_155))
    );
    assert_eq!(download.state, Some(DownloadState::Complete));
}

#[test]
fn chrome_27_and_later_as_plaso_expects() {
    // File, visit time, download start and end.
    for (name, visited, start, end) in [
        (
            "History-57.0.2987.133",
            "2018-01-21T14:09:53.885478",
            "2018-01-21T14:09:53.900399",
            "2018-01-21T14:09:54.858738",
        ),
        (
            "History-58.0.3029.96",
            "2018-01-21T14:09:27.315765",
            "2018-01-21T14:09:27.200398",
            "2018-01-21T14:09:28.116062",
        ),
        (
            "History-59.0.3071.86",
            "2018-01-21T14:08:52.037692",
            "2018-01-21T14:08:51.811123",
            "2018-01-21T14:08:52.662377",
        ),
        (
            "History-59_added-fake-column",
            "2018-01-21T14:08:52.037692",
            "2018-01-21T14:08:51.811123",
            "2018-01-21T14:08:52.662377",
        ),
    ] {
        let history = history(name);
        assert_eq!((history.visits.len(), history.downloads.len()), (1, 1));
        let visit = &history.visits[0];
        assert_eq!(micros(visit.time), visited, "{name}");
        assert_eq!(
            visit.url,
            "https://raw.githubusercontent.com/dfirlabs/chrome-specimens/master/generate-specimens.sh"
        );
        assert_eq!(visit.title, "");
        assert_eq!((visit.visit_count, visit.typed_count), (Some(1), Some(0)));
        assert_eq!(
            visit.transition.to_string(),
            "AUTO_TOPLEVEL|CHAIN_START|CHAIN_END"
        );
        assert_eq!(visit.duration, Some(std::time::Duration::ZERO));

        let download = &history.downloads[0];
        let url = "https://raw.githubusercontent.com/log2timeline/l2tbinaries/master/win32/plaso-20171231.1.win32.msi";
        assert_eq!(download.url, url);
        assert_eq!(download.url_chain, [url]);
        assert_eq!(
            download.target_path,
            "/home/ubuntu/Downloads/plaso-20171231.1.win32.msi"
        );
        assert_eq!(micros(download.start), start, "{name}");
        assert_eq!(micros(download.end), end, "{name}");
        assert_eq!(
            (download.received_bytes, download.total_bytes),
            (Some(3_080_192), Some(3_080_192))
        );
        // As the sqlite3 shell reads them (not in plaso's tests).
        assert_eq!(download.state, Some(DownloadState::Complete));
        assert_eq!(download.danger_type, Some(4));
        assert_eq!(download.interrupt_reason, Some(0));
        assert_eq!(download.opened, Some(false));
        assert_eq!(download.referrer.as_deref(), Some(""));
        assert_eq!(download.tab_url.as_deref(), Some(url));
        assert_eq!(
            download.mime_type.as_deref(),
            Some("application/octet-stream")
        );
    }
}

#[test]
fn firefox_places_visits() {
    let history = history("places.sqlite");
    assert_eq!(history.kind, Kind::FirefoxPlaces);
    // plaso's first event; the other 102 are bookmarks.
    assert_eq!(history.visits.len(), 1);
    let visit = &history.visits[0];
    assert_eq!(micros(visit.time), "2011-07-01T11:16:21.371935");
    assert_eq!(visit.url, "http://news.google.com/");
    assert_eq!(visit.title, "Google News");
    assert_eq!(visit.visit_count, Some(1));
    assert_eq!(visit.transition, Transition::Firefox(VisitType::Typed));
    assert!(history.downloads.is_empty());
}

#[test]
fn firefox_25_visits_and_annotated_download() {
    let history = history("firefox_25_places.sqlite.gz");
    assert_eq!(history.visits.len(), 34);
    // plaso's event 10.
    let visit = &history.visits[10];
    assert_eq!(micros(visit.time), "2013-10-30T21:57:11.281942");
    assert_eq!(visit.url, "http://code.google.com/p/plaso");
    assert_eq!(visit.visit_count, Some(1));
    assert_eq!(visit.transition, Transition::Firefox(VisitType::Typed));
    // The redirect that followed it, from it.
    let next = &history.visits[11];
    assert_eq!(
        next.transition,
        Transition::Firefox(VisitType::RedirectPermanent)
    );
    assert_eq!(next.from_visit, Some(visit.id));

    // Not in plaso's tests (its join pairs annotations by adjacent
    // attribute ids, which these aren't); as the sqlite3 shell reads it.
    assert_eq!(history.downloads.len(), 1);
    let download = &history.downloads[0];
    assert_eq!(
        download.target_path,
        "file:///build/Downloads/plaso-static-1.0.2-rc3-win-amd64-vs2010.zip"
    );
    assert_eq!(micros(download.start), "2013-10-30T21:58:24.854681");
    assert_eq!(micros(download.end), "2013-10-30T21:58:28.130000");
    assert_eq!(download.total_bytes, Some(31_850_899));
    assert_eq!(download.state, Some(DownloadState::Complete));
    assert_eq!(download.deleted, None);
}

#[test]
fn firefox_118_annotated_downloads() {
    let history = history("places118.sqlite.gz");
    assert_eq!(history.visits.len(), 89);
    // plaso: 7 downloads, the third:
    assert_eq!(history.downloads.len(), 7);
    let download = &history.downloads[2];
    assert_eq!(
        download.url,
        "https://az764295.vo.msecnd.net/stable/74f6148eb9ea00507ec113ec51c489d6ffb4b771/VSCodeUserSetup-x64-1.80.1.exe"
    );
    assert_eq!(
        download.target_path,
        "file:///C:/Users/factdevteam/Downloads/VSCodeUserSetup-x64-1.80.1.exe"
    );
    assert_eq!(micros(download.start), "2023-07-18T23:20:28.452000");
    assert_eq!(micros(download.end), "2023-07-18T23:20:29.974000");
    assert_eq!(download.total_bytes, Some(93_176_792));
    assert_eq!(download.state, Some(DownloadState::Complete));
    assert_eq!(download.deleted, Some(false));
    // The first was cancelled, and has no size.
    let cancelled = &history.downloads[0];
    assert_eq!(cancelled.state, Some(DownloadState::Cancelled));
    assert_eq!(cancelled.total_bytes, None);
}

#[test]
fn firefox_legacy_downloads() {
    let history = history("downloads.sqlite");
    assert_eq!(history.kind, Kind::FirefoxDownloads);
    assert!(history.visits.is_empty());
    let [download] = history.downloads.as_slice() else {
        panic!("{:?}", history.downloads)
    };
    assert_eq!(
        download.url,
        "https://plaso.googlecode.com/files/plaso-static-1.0.1-win32-vs2008.zip"
    );
    assert_eq!(
        download.target_path,
        "file:///D:/plaso-static-1.0.1-win32-vs2008.zip"
    );
    assert_eq!(micros(download.start), "2013-07-18T18:59:59.312000");
    assert_eq!(micros(download.end), "2013-07-18T19:01:18.578000");
    assert_eq!(
        (download.received_bytes, download.total_bytes),
        (Some(15_974_599), Some(15_974_599))
    );
    assert_eq!(download.mime_type.as_deref(), Some("application/zip"));
    assert_eq!(download.state, Some(DownloadState::Complete));
}

#[test]
fn every_visit_matches_sqlite3() {
    for name in [
        "History",
        "History-59.0.3071.86",
        "places.sqlite",
        "firefox_25_places.sqlite.gz",
        "places118.sqlite.gz",
    ] {
        let oracle = std::fs::read_to_string(format!(
            "{}/tests/oracle/{}.visits",
            env!("CARGO_MANIFEST_DIR"),
            name.trim_end_matches(".gz")
        ))
        .unwrap();
        let ours: Vec<String> = history(name)
            .visits
            .iter()
            .map(|v| {
                let raw = match v.transition {
                    Transition::Chromium(PageTransition(bits)) => i64::from(bits),
                    Transition::Firefox(t) => t.raw(),
                };
                let ticks = v.time.and_then(|t| t.ticks()).unwrap_or(0);
                format!(
                    "{}|{}|{}|{raw}|{}|{}|{}",
                    v.id,
                    v.url,
                    ticks / 10,
                    v.from_visit.unwrap_or(0),
                    v.visit_count.unwrap_or(0),
                    u8::from(v.hidden)
                )
            })
            .collect();
        assert_eq!(ours, oracle.lines().collect::<Vec<_>>(), "{name}");
    }
}
