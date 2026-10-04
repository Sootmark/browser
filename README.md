# browser

Browser history for forensics: the pages visited and the files downloaded, from Chromium's `History` (Chrome, Edge, Brave, Opera, Vivaldi) and Firefox's `places.sqlite` and `downloads.sqlite`. Two dependencies, its siblings `sootmark-common` (times) and `sootmark-sqlite` (read without SQLite).

```toml
[dependencies]
sootmark-browser = "0.1"
```

```rust
let database = std::fs::read("History")?;
let wal = std::fs::read("History-wal").unwrap_or_default();
let history = browser::read(&database, &wal)?;
for v in &history.visits {
    println!("{} {} {} {}", v.time.map(|t| t.to_string()).unwrap_or_default(), v.transition, v.url, v.title);
}
for d in &history.downloads {
    println!("{:?} {} -> {} {:?}", d.start, d.url, d.target_path, d.state);
}
for problem in &history.problems {
    eprintln!("{problem}");
}
```

## What you get

- `read(database, wal)`: a `History` with the database's `Kind`, its visits, its downloads and its `problems`. The `-wal` file's committed changes are applied: the latest visits are often only there.
- `Visit`: when, URL, title, how the browser came to the page (`Transition`), the visit it came from, and the page's visit count, typed flag, hidden flag; Chromium adds the time in front and the typed count, Firefox its frecency.
- Chromium transitions as `ui/base/page_transition_types.h` defines them: the core type (`LINK`, `TYPED`, `AUTO_BOOKMARK`, … `KEYWORD_GENERATED`) and the qualifiers (`FORWARD_BACK`, `FROM_ADDRESS_BAR`, `HOME_PAGE`, `FROM_API`, `CHAIN_START`, `CHAIN_END`, `CLIENT_REDIRECT`, `SERVER_REDIRECT`, `BLOCKED`), unknown bits kept; stored signed or unsigned, both read. Firefox visit types as `nsINavHistoryService` defines them (`LINK` … `RELOAD`).
- `Download`: URL and, in Chromium, the whole redirect chain; where the file was saved and, while downloading, written; start and end; bytes received and expected; state (in progress, complete, cancelled, interrupted, paused, blocked, dirty); Chromium's danger type and interrupt reason, referrer, tab URL, MIME type, whether it was opened; Firefox's deleted flag.
- Chromium `History` of every version plaso has test files for (Chrome 8 to 59), and the tables of recent versions (tested with synthetic databases modelled on Chromium's sources): downloads before history version 24 (Chrome 26) kept one URL and Unix seconds, and are read as such.
- Firefox downloads: since Firefox 26, page annotations (`downloads/destinationFileURI`, `downloads/metaData` JSON), joined by attribute name; before, `downloads.sqlite`'s `moz_downloads`.
- `detect(name, data)` and `Kind::of(&database)`: which database a file is, from its tables (`urls` and `visits`; `moz_places` and `moz_historyvisits`; `moz_downloads`), or from its name when only the start of the file is at hand.
- Columns are read by name: one a version lacks reads as `None`, one it added is ignored. Damage is reported, never a panic: a visit whose page row is gone is kept without its URL, metadata that isn't JSON is reported and the rest of the download kept, and the SQLite reader's own findings (damaged pages, a foreign log) are passed on.

## Not yet

- Recovery of deleted visits from free pages (the SQLite reader doesn't yet recover).
- Bookmarks, cookies, form history, autofill, favicons, keyword searches (`keyword_search_terms`), sync sources (`visit_source`), Chromium's `Top Sites` and `Shortcuts`, Safari.
- Danger types and interrupt reasons as names: they are Chromium's numbers, which grow with each version.

## How it's checked

| Check | Result |
|---|---|
| plaso's test files (Apache-2.0, `tests/fixtures/plaso/`, commit `a70dde8`): Chrome 8, 57, 58, 59 and 59 with an added column; Firefox `places.sqlite` (≤ 23), 25 and 118, `downloads.sqlite` (25), against what plaso's own tests expect: counts, URLs, titles, times to the microsecond, sizes, paths | all match: Chrome 8's 69 visits and 2 downloads, one visit and one download in each later Chrome, Firefox 118's 7 downloads |
| Every visit of five of them (Chrome 8 and 59, the three Firefox `places.sqlite`, 194 visits) against the `sqlite3` shell 3.46.1 joining the same tables (`tests/oracle/`, made by `gen.sh`): id, URL, time, raw transition, referring visit, visit count, hidden | all match |
| Databases made by `tests/fixtures/synthetic/gen.sh` (the sqlite3 shell, recent Chromium's tables, synthetic rows): transition qualifiers, a redirect stored with the sign bit set, a three-URL download chain written out of order, an interrupted download, a visit only in the write-ahead log, rows that don't join, Firefox metadata that isn't JSON | as written |
| Property tests: arbitrary bytes, arbitrary pages behind a real header, every fixture and the log damaged and cut anywhere | read or refused, never a panic |

## Licence

MIT or Apache-2.0, at your option. The plaso test files are under the Apache licence 2.0 (`tests/fixtures/plaso/LICENSE`, `NOTICE`); the synthetic ones are made by the script beside them.
