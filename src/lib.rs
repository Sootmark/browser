//! Browser history for forensics: the pages visited and the files
//! downloaded, read from the browser's own SQLite database without SQLite.
//!
//! - Chromium's `History`, which Chrome, Edge, Brave, Opera and Vivaldi
//!   share: visits (`visits` joined to `urls`) and downloads (`downloads`
//!   with its redirect chain, `downloads_url_chains`). Times are `WebKit`
//!   microseconds since 1601, except the downloads of databases older than
//!   Chrome 26 (history version 24), which kept Unix seconds.
//! - Firefox's `places.sqlite`: visits (`moz_historyvisits` joined to
//!   `moz_places`) and, since Firefox 26, downloads, kept as page
//!   annotations (`downloads/destinationFileURI`, `downloads/metaData`).
//!   Times are `PRTime`, microseconds since 1970.
//! - Firefox's `downloads.sqlite` (`moz_downloads`), where Firefox 25 and
//!   older kept downloads.
//!
//! Columns are read by name: one a version lacks reads as `None`, one it
//! added is ignored. [`read`] takes the database with its write-ahead log,
//! whose committed changes it applies: the latest visits are often only
//! there. Damage is listed in `problems`, never a panic; a visit whose page
//! row is gone is kept, without its URL.

use common::time::Ts;
use sqlite::Database;

mod chromium;
mod firefox;
mod table;
mod transition;

pub use transition::{CoreTransition, PageTransition, Qualifier, Transition, VisitType};

/// This crate's version, for records of what parsed them.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Which database a file is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A Chromium-family `History` (tables `urls` and `visits`).
    ChromiumHistory,
    /// Firefox's `places.sqlite` (tables `moz_places` and
    /// `moz_historyvisits`).
    FirefoxPlaces,
    /// Firefox's `downloads.sqlite` (table `moz_downloads`), up to
    /// Firefox 25.
    FirefoxDownloads,
}

impl Kind {
    /// The kind a file's name suggests: `History`, `places.sqlite`,
    /// `downloads.sqlite` (case ignored), from a path or a bare name.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
        [
            ("History", Self::ChromiumHistory),
            ("places.sqlite", Self::FirefoxPlaces),
            ("downloads.sqlite", Self::FirefoxDownloads),
        ]
        .into_iter()
        .find(|(known, _)| base.eq_ignore_ascii_case(known))
        .map(|(_, kind)| kind)
    }

    /// The kind an open database is, from the tables it has.
    #[must_use]
    pub fn of(db: &Database<'_>) -> Option<Self> {
        let has = |table: &str| db.table(table).is_some();
        if has("urls") && has("visits") {
            Some(Self::ChromiumHistory)
        } else if has("moz_places") && has("moz_historyvisits") {
            Some(Self::FirefoxPlaces)
        } else if has("moz_downloads") {
            Some(Self::FirefoxDownloads)
        } else {
            None
        }
    }
}

/// Which browser database a file is, from its contents, else its name.
///
/// The tables decide when the file is a SQLite database whose schema
/// reads; the name (as [`Kind::from_name`]) only when it is a SQLite
/// database whose schema doesn't, such as the first pages of a larger
/// file. Anything else is `None`.
#[must_use]
pub fn detect(name: &str, data: &[u8]) -> Option<Kind> {
    if !data.starts_with(b"SQLite format 3\0") {
        return None;
    }
    match Database::open(data) {
        Ok(db) if !db.tables.is_empty() => Kind::of(&db),
        _ => Kind::from_name(name),
    }
}

/// One visit to a page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Visit {
    /// The visit's row id (`visits.id`, `moz_historyvisits.id`).
    pub id: i64,
    /// When.
    pub time: Option<Ts>,
    /// The page; empty when its row is gone.
    pub url: String,
    /// The page's title, as last seen; empty when none.
    pub title: String,
    /// How the browser came to the page.
    pub transition: Transition,
    /// The visit this one came from (the referring page, the redirect's
    /// source): its `id`.
    pub from_visit: Option<i64>,
    /// How long the page was in front (Chromium; zero when not measured).
    pub duration: Option<std::time::Duration>,
    /// Visits to the page in all.
    pub visit_count: Option<i64>,
    /// Whether the page's URL was ever typed.
    pub typed: bool,
    /// How many times the URL was typed (Chromium).
    pub typed_count: Option<i64>,
    /// Hidden from the history list and suggestions (subframes, redirect
    /// sources, errors).
    pub hidden: bool,
    /// Firefox's ranking of the page for suggestions, by frequency and
    /// recency.
    pub frecency: Option<i64>,
}

/// What became of a download.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadState {
    /// Under way, or the browser stopped before it ended.
    InProgress,
    /// Finished.
    Complete,
    /// Cancelled by the user.
    Cancelled,
    /// Stopped by an error (Firefox: failed).
    Interrupted,
    /// Paused (Firefox).
    Paused,
    /// Blocked by parental controls or policy (Firefox).
    Blocked,
    /// Blocked as malware, or as a potentially unwanted program (Firefox).
    Dirty,
    /// Any other value, as written.
    Other(i64),
}

/// One download.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Download {
    /// Its row id: `downloads.id` (Chromium), the first of its
    /// annotations' (Firefox), `moz_downloads.id` (Firefox up to 25).
    pub id: i64,
    /// Where it was fetched from, after redirects.
    pub url: String,
    /// Every URL on the way, from the first requested to [`url`](Self::url)
    /// (Chromium).
    pub url_chain: Vec<String>,
    /// Where the file was saved: a path (Chromium), a `file://` URI
    /// (Firefox).
    pub target_path: String,
    /// Where the file was written while downloading (Chromium; Firefox up
    /// to 25).
    pub current_path: Option<String>,
    /// When it began.
    pub start: Option<Ts>,
    /// When it ended.
    pub end: Option<Ts>,
    /// Bytes received (Chromium; Firefox up to 25).
    pub received_bytes: Option<i64>,
    /// The file's size: as announced (Chromium; Firefox up to 25), or of
    /// the finished file (Firefox).
    pub total_bytes: Option<i64>,
    /// What became of it.
    pub state: Option<DownloadState>,
    /// Chromium's verdict on the file (`download::DownloadDangerType`:
    /// 0 not dangerous, 1 dangerous file type, 2 dangerous URL, 3
    /// dangerous content, 4 maybe dangerous, 5 uncommon, 6 kept by the
    /// user, 7 dangerous host, 8 potentially unwanted, …).
    pub danger_type: Option<i64>,
    /// Why it stopped, when interrupted (Chromium's
    /// `download::DownloadInterruptReason`, 0 for none).
    pub interrupt_reason: Option<i64>,
    /// The referring page.
    pub referrer: Option<String>,
    /// The page in the tab that started it (Chromium).
    pub tab_url: Option<String>,
    /// Its MIME type, as served (Chromium; Firefox up to 25).
    pub mime_type: Option<String>,
    /// Whether it was opened from the browser (Chromium).
    pub opened: Option<bool>,
    /// Whether the user deleted the file from the browser (Firefox).
    pub deleted: Option<bool>,
}

/// A database's visits and downloads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct History {
    /// Which database it is.
    pub kind: Kind,
    /// In row order: the order the browser recorded them.
    pub visits: Vec<Visit>,
    /// In row order.
    pub downloads: Vec<Download>,
    /// Damage in the database or its log, and rows that don't join.
    pub problems: Vec<String>,
}

/// Why a file isn't browser history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(pub String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

/// Read a browser's history database, with its `-wal` file's committed
/// changes (`wal` may be empty).
///
/// # Errors
/// When it isn't a SQLite database, or has none of the tables of a
/// [`Kind`].
pub fn read(database: &[u8], wal: &[u8]) -> Result<History, Error> {
    let db = Database::open_with_wal(database, wal).map_err(|e| Error(e.to_string()))?;
    let kind = Kind::of(&db).ok_or_else(|| Error("not a browser history database".to_owned()))?;
    let mut problems = db.problems.clone();
    let (visits, downloads) = match kind {
        Kind::ChromiumHistory => (
            chromium::visits(&db, &mut problems),
            chromium::downloads(&db, &mut problems),
        ),
        Kind::FirefoxPlaces => firefox::places(&db, &mut problems),
        Kind::FirefoxDownloads => (Vec::new(), firefox::legacy_downloads(&db, &mut problems)),
    };
    Ok(History {
        kind,
        visits,
        downloads,
        problems,
    })
}
