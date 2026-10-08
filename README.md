# browser

Browser history for forensics: the pages visited and the files downloaded, from Chromium's `History` (Chrome, Edge, Brave, Opera, Vivaldi), Firefox's `places.sqlite` and `downloads.sqlite`, and Internet Explorer and legacy Edge's `WebCacheV01.dat`, with the deleted visits, pages and downloads the SQLite databases still hold. Three dependencies, its siblings `sootmark-common` (times), `sootmark-sqlite` and `sootmark-ese` (the databases, read without SQLite or ESE).

```toml
[dependencies]
sootmark-browser = "0.6"
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
for r in &history.deleted_visits {
    let p = &r.provenance;
    println!("deleted: {:?} {} ({:?}; page {} {:?} {:?}, {:?})", r.visit.time, r.visit.url, r.page, p.page, p.page_state, p.area, p.confidence);
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
- Internet Explorer and legacy Edge's `WebCacheV01.dat` (an ESE database): the visits of its history containers (`History`, and `MSHist01…` per day or week), with the account each was recorded for (`Visited: alice@…`) and the last visit's time (`AccessedTime`); local files opened in Explorer are there too, as `file:///` URLs. How the page was reached isn't recorded (`Transition::NotRecorded`). Its downloads (`iedownload`) and cache entries aren't read yet.
- Deleted history, apart from the live rows: `deleted_visits` (Chromium `visits`, Firefox `moz_historyvisits`), `deleted_pages` (Chromium `urls`, Firefox `moz_places`: URL, title, last visit, visit and typed counts, hidden, frecency) and `deleted_downloads` (Chromium `downloads` with their URL chains, Firefox 25's `moz_downloads`), from the records `sootmark-sqlite` recovers: freeblocks and unallocated space of table pages and freelist pages (the database as it reads now), and the older page versions a write-ahead log keeps (superseded and uncommitted frames, and the database file's own copies of pages the log replaces). Times are decoded as for live rows.
- Each recovered entry carries its `Provenance`: the table, the rowid when its cell's start survived, the page and offset, the page's state (in use, freelist, or which older version: `Superseded { frame }`, `Uncommitted { frame }`, `Invalid { frame }`, `ReplacedInFile`), the area (cell, freeblock, unallocated, free page), the evidence (cell pointer, whole cell, intact record header, rebuilt from the table's shape), the confidence (high for a whole cell whose values all have their column's usual type and that fits one table; medium for a damaged record, such as one in a freeblock whose rowid a freeblock header overwrote; low otherwise), the columns whose values were lost, and whether its overflow pages are gone (`truncated`). A deleted visit's URL and title come from the live page its page id names, else from a recovered page record with that id, else they are empty, and `page` says which (`Live`, `Recovered`, `NotFound`); a lost id reads 0 and a lost transition `NotRecorded`.
- Earlier states of live rows, which a log's older pages are full of (a page whose visit count or frecency changed, a visit whose duration was written later, a download whose state changed), are not deleted entries: a recovered visit with the page URL and time of a live one, a page with a live page's URL, a download with a live one's start and destination, is left out. Of several copies of the same deleted entry (in a freeblock now and whole in an older page), the strongest is kept; of several versions of a deleted page, the one last visited. A record that lost what identifies it (a visit's time, a page's URL) can't be told apart and is kept.
- Limits: SQLite zeroes freed cells when `secure_delete` is on, and the browsers commonly delete that way (plaso's browser files have their freed space zeroed), so deleted history is found mostly in the write-ahead log's older pages and in the database file under them, until a checkpoint overwrites the file and the log restarts: read the `-wal` with the database. Recovery runs on every `read`; on plaso's files (up to 10 MiB) it takes 0.2 to 2.5 ms in a release build, most of `read`'s time, and grows with the database.
- `detect(name, data)` and `Kind::of(&database)`: which database a file is, from its tables (`urls` and `visits`; `moz_places` and `moz_historyvisits`; `moz_downloads`), or from its name when only the start of the file is at hand; an ESE database named `WebCacheV01.dat` is a WebCache.
- Columns are read by name: one a version lacks reads as `None`, one it added is ignored. Damage is reported, never a panic: a visit whose page row is gone is kept without its URL, metadata that isn't JSON is reported and the rest of the download kept, and the SQLite reader's own findings (damaged pages, a foreign log) are passed on.
- Beyond history: `read_cookies` (Chromium's `Cookies`, both column spellings, and Firefox's `cookies.sqlite`: host, name, value as stored, path, created, last sent, expiry, secure, HTTP-only), `read_autofill` (Chromium's `Web Data`: each value typed in a form field, how often, first and last), `read_extension_activity` (Chromium's `Extension Activity`: each extension's API calls and events with the page they acted on, strings and URLs joined) and `read_preferences` (Chromium's `Preferences`: the extensions installed, with name, version, folder, installation time, origin and granted APIs, and the sites given permissions); and Safari's history (`History.db` and the older `History.plist`) and downloads (`Downloads.plist`) through `read`.

## Not yet

- Deleted Firefox downloads (their annotations, `moz_annos`), and deleted WebCache entries (ESE: not recovered).
- A deleted visit whose page record lost its rowid to a freeblock header isn't joined to it (Chromium's `last_visit_time` could match it to its last visit, a guess not made); its URL is empty.
- WebCache downloads (`iedownload`), cache entries and cookies.
- Bookmarks, cookies, form history, autofill, favicons, keyword searches (`keyword_search_terms`), sync sources (`visit_source`), Chromium's `Top Sites` and `Shortcuts`, Safari.
- Danger types and interrupt reasons as names: they are Chromium's numbers, which grow with each version.

## How it's checked

| Check | Result |
|---|---|
| plaso's test files (Apache-2.0, `tests/fixtures/plaso/`, commit `a70dde8`): Chrome 8, 57, 58, 59 and 59 with an added column; Firefox `places.sqlite` (≤ 23), 25 and 118, `downloads.sqlite` (25), against what plaso's own tests expect: counts, URLs, titles, times to the microsecond, sizes, paths | all match: Chrome 8's 69 visits and 2 downloads, one visit and one download in each later Chrome, Firefox 118's 7 downloads |
| Every visit of five of them (Chrome 8 and 59, the three Firefox `places.sqlite`, 194 visits) against the `sqlite3` shell 3.46.1 joining the same tables (`tests/oracle/`, made by `gen.sh`): id, URL, time, raw transition, referring visit, visit count, hidden | all match |
| Databases made by `tests/fixtures/synthetic/gen.sh` (the sqlite3 shell, recent Chromium's tables, synthetic rows): transition qualifiers, a redirect stored with the sign bit set, a three-URL download chain written out of order, an interrupted download, a visit only in the write-ahead log, rows that don't join, Firefox metadata that isn't JSON | as written |
| plaso's WebCache databases (`WebCacheV01.dat`, `PartitionsEx-WebCacheV01.dat`): the visits of their history containers, against libesedb's reading of the same rows (through `sootmark-ese`'s oracle) | 113 and 69 visits, accounts, URLs, times to the 100 ns |
| Deleted history, `tests/fixtures/recovery/chromium.db`: `sootmark-sqlite`'s Chromium-shaped recovery fixture (its `gen.sh`, the `sqlite3` shell 3.46.1, `secure_delete` off), a time range of visits cleared and then the pages left without visits, against the rows its generator dumped before deleting them (`tests/oracle/recovery/`) | all 61 cleared visits, each once, with its time, transition, referring visit and duration; 20 of 20 deleted pages with URL, title, counts and last visit (12 whole with their rowids, 8 from freeblocks without); 37 visits joined to their URL (1 live page, 36 recovered), 24 left without, their pages' rowids overwritten |
| Databases made by `tests/fixtures/recovery/gen.sh` in write-ahead log mode, deleting with `secure_delete` on after a checkpoint, as the browsers do: a Chromium `History` whose log forgets a site (two pages, three visits), deletes two visits of a kept page, rewrites a visit's duration and deletes a download with its URL chain; a Firefox `places.sqlite` with visits deleted (`secure_delete` off) before the checkpoint, a site forgotten, a visit deleted and frecencies changed in the log, and two visits recorded and deleted in one transaction | Chromium: the 5 deleted visits whole with their rowids (from the superseded frame), their URLs from the 2 recovered pages and the live one, the download with its two-URL chain; the rewritten visit and the kept page's older count not reported. Firefox: the 7 deleted visits (2 from the page as the log has it now, 1 from a superseded frame, 4 from the file's copy) and the forgotten page only. Each file alone: nothing, and the two freeblock visits |
| plaso's files and the synthetic ones above | no deleted entry: their freeblocks and their one freelist page are zeroed, and what isn't zero between their cell pointers and cells is stale cell pointers (checked byte by byte, independently of the reader) |
| Property tests: arbitrary bytes, arbitrary pages behind a real header, every fixture (those with deleted records too) and the logs (with older page versions to recover too) damaged and cut anywhere | read or refused, never a panic |
- Cookies, form history, extensions and Safari: plaso's test files (`tests/fixtures/plaso/`), every one of the 2,144 events its `chrome_17_cookies`, `chrome_66_cookies`, `firefox_2_cookies`, `firefox_10_cookies`, `chrome_autofill`, `chrome_extension_activity`, `chrome_preferences`, `safari_historydb`, `safari_history` and `safari_downloads` parsers read, read the same (`tests/extras.rs`).

## Licence

MIT or Apache-2.0, at your option. The plaso test files are under the Apache licence 2.0 (`tests/fixtures/plaso/LICENSE`, `NOTICE`); the synthetic ones are made by the scripts beside them, `recovery/chromium.db` and its oracle by `sootmark-sqlite`'s (MIT or Apache-2.0).
