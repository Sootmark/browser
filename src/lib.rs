//! Browser history for forensics: the pages visited and the files
//! downloaded, read from the browser's own database without SQLite or ESE.
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
//! - Internet Explorer and legacy Edge's `WebCacheV01.dat`, an ESE
//!   database: the visits of its history containers, with the account
//!   each was recorded for; local files opened in Explorer as `file:///`
//!   URLs. Times are FILETIMEs.
//!
//! Columns are read by name: one a version lacks reads as `None`, one it
//! added is ignored. [`read`] takes the database with its write-ahead log,
//! whose committed changes it applies: the latest visits are often only
//! there. Damage is listed in `problems`, never a panic; a visit whose page
//! row is gone is kept, without its URL.
//!
//! Deleted history is recovered from the SQLite databases, apart from the
//! live rows: the records of deleted visits, pages and downloads that
//! `sootmark-sqlite` finds in free space, on the freelist and in the older
//! page versions a write-ahead log keeps, each with where and how it was
//! found ([`Provenance`]). Earlier states of rows still live (a page whose
//! visit count changed, a visit whose duration was written later) are left
//! out.

use common::time::Ts;
use sqlite::Database;

mod autofill;
mod chromium;
mod cookies;
mod extensions;
mod firefox;
mod recovered;
mod safari;
mod table;
mod transition;
mod webcache;

pub use autofill::AutofillEntry;
pub use cookies::{Cookie, CookieStore};
pub use extensions::{
    read_preferences, ContentException, ExtensionActivity, InstalledExtension, Preferences,
};

pub use sqlite::{Area, Confidence, Evidence, PageState};
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
    /// Internet Explorer and legacy Edge's `WebCacheV01.dat`, an ESE
    /// database (table `Containers`): visits only.
    WebCache,
    /// Safari's `History.db` (tables `history_items` and
    /// `history_visits`).
    SafariHistory,
    /// Safari's older `History.plist`.
    SafariHistoryPlist,
    /// Chromium's `Cookies` (table `cookies`) or Firefox's
    /// `cookies.sqlite` (table `moz_cookies`): read with [`read_cookies`].
    Cookies,
    /// Chromium's `Web Data` (table `autofill`): read with
    /// [`read_autofill`].
    Autofill,
    /// Chromium's `Extension Activity` (table `activitylog_compressed`):
    /// read with [`read_extension_activity`].
    ExtensionActivity,
    /// Chromium's `Preferences` (JSON): read with [`read_preferences`].
    Preferences,
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
            ("WebCacheV01.dat", Self::WebCache),
            ("History.db", Self::SafariHistory),
            ("History.plist", Self::SafariHistoryPlist),
            ("Cookies", Self::Cookies),
            ("cookies.sqlite", Self::Cookies),
            ("Web Data", Self::Autofill),
            ("Extension Activity", Self::ExtensionActivity),
            ("Preferences", Self::Preferences),
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
        } else if has("history_items") && has("history_visits") {
            Some(Self::SafariHistory)
        } else if has("cookies") || has("moz_cookies") {
            Some(Self::Cookies)
        } else if has("autofill") {
            Some(Self::Autofill)
        } else if has("activitylog_compressed") {
            Some(Self::ExtensionActivity)
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
/// file. An ESE database is a WebCache when named as one. Anything else is
/// `None`.
#[must_use]
pub fn detect(name: &str, data: &[u8]) -> Option<Kind> {
    if is_ese(data) {
        return (Kind::from_name(name) == Some(Kind::WebCache)).then_some(Kind::WebCache);
    }
    let named = Kind::from_name(name);
    if data.starts_with(b"bplist") || data.trim_ascii_start().starts_with(b"<?xml") {
        return named.filter(|k| *k == Kind::SafariHistoryPlist);
    }
    if data.trim_ascii_start().starts_with(b"{") {
        return named.filter(|k| *k == Kind::Preferences);
    }
    if !data.starts_with(SQLITE_MAGIC) {
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
    /// The visit's row id (`visits.id`, `moz_historyvisits.id`, a WebCache
    /// container's `EntryId`), unique within its table.
    pub id: i64,
    /// The table it's in: `visits`, `moz_historyvisits`, or the WebCache
    /// container's `Container_<ContainerId>`.
    pub table: String,
    /// When.
    pub time: Option<Ts>,
    /// The page; empty when its row is gone.
    pub url: String,
    /// The page's title, as last seen; empty when none.
    pub title: String,
    /// The account the visit was recorded for, when the database says
    /// (WebCache's `Visited: alice@…`).
    pub user: Option<String>,
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

/// A page: a Chromium `urls` row, a Firefox `moz_places` row.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Page {
    /// Its row id (`urls.id`, `moz_places.id`), which visits name; 0 when
    /// a recovered record lost it ([`Provenance::rowid`] is `None`).
    pub id: i64,
    /// Its URL.
    pub url: String,
    /// Its title, as last seen; empty when none.
    pub title: String,
    /// When it was last visited.
    pub last_visit: Option<Ts>,
    /// Visits to it in all.
    pub visit_count: Option<i64>,
    /// Whether its URL was ever typed.
    pub typed: bool,
    /// How many times its URL was typed (Chromium).
    pub typed_count: Option<i64>,
    /// Hidden from the history list and suggestions.
    pub hidden: bool,
    /// Firefox's ranking of it for suggestions.
    pub frecency: Option<i64>,
}

/// Where and how a deleted entry was found: the record `sootmark-sqlite`
/// recovered it from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Provenance {
    /// The table its record fits best: `visits`, `urls`, `downloads`,
    /// `moz_historyvisits`, `moz_places`, `moz_downloads`.
    pub table: String,
    /// Its rowid, when the cell's start survived (never for a record whose
    /// first bytes a freeblock header overwrote).
    pub rowid: Option<i64>,
    /// The page it was found on (for a log frame, the page it is a version
    /// of).
    pub page: u32,
    /// Where its cell starts on that page, from the page's first byte.
    pub offset: usize,
    /// The page as the database reads now (in a table, on the freelist),
    /// or the older version of it a write-ahead log or the file under it
    /// keeps (superseded, never committed, replaced by the log).
    pub page_state: PageState,
    /// Where on the page: a cell, a freeblock, unallocated space, a free
    /// page.
    pub area: Area,
    /// How much of the cell was read as stored.
    pub evidence: Evidence,
    /// How sure the match of the record with its table is.
    pub confidence: Confidence,
    /// Other tables its record fits as well.
    pub also_fits: Vec<String>,
    /// The columns whose values were lost (overwritten, or past the point
    /// where a truncated record stops): the fields read from them are
    /// `None`, empty, zero or false.
    pub lost: Vec<String>,
    /// Whether its payload spilled to overflow pages that are gone: the
    /// values after its local bytes are lost, the one they cut kept as far
    /// as it goes.
    pub truncated: bool,
}

/// Where a recovered visit's page (URL, title, counts) came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PageSource {
    /// The live page row its page id names.
    Live,
    /// A recovered page record with its page id: the page was deleted too.
    Recovered,
    /// Neither, or its page id was lost: URL and title are empty.
    NotFound,
}

/// A deleted visit, recovered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveredVisit {
    /// The visit as its record and page read; its `id` is 0 when the rowid
    /// was lost, its `transition` [`Transition::NotRecorded`] when the
    /// transition was.
    pub visit: Visit,
    /// Where its page came from.
    pub page: PageSource,
    /// Where and how its record was found.
    pub provenance: Provenance,
}

/// A deleted page, recovered: its visits may be gone too.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveredPage {
    /// The page as its record reads.
    pub page: Page,
    /// Where and how its record was found.
    pub provenance: Provenance,
}

/// A deleted download, recovered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveredDownload {
    /// The download as its record reads, its `id` 0 when the rowid was
    /// lost; a Chromium download's URL chain from its live or recovered
    /// `downloads_url_chains` rows, empty when none is found.
    pub download: Download,
    /// Where and how its record was found.
    pub provenance: Provenance,
}

/// A database's visits and downloads, live and deleted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct History {
    /// Which database it is.
    pub kind: Kind,
    /// In row order: the order the browser recorded them.
    pub visits: Vec<Visit>,
    /// In row order.
    pub downloads: Vec<Download>,
    /// Deleted visits (Chromium `visits`, Firefox `moz_historyvisits`),
    /// recovered: those in the database as it reads now first, then those
    /// in older page versions, each in page and offset order.
    pub deleted_visits: Vec<RecoveredVisit>,
    /// Deleted pages (Chromium `urls`, Firefox `moz_places`) whose URL no
    /// live page has, recovered, in the same order.
    pub deleted_pages: Vec<RecoveredPage>,
    /// Deleted downloads (Chromium `downloads`, Firefox 25 and older
    /// `moz_downloads`), recovered, in the same order.
    pub deleted_downloads: Vec<RecoveredDownload>,
    /// Damage in the database or its log, rows that don't join, and damage
    /// met by recovery (`recovery: …`).
    pub problems: Vec<String>,
}

/// What a SQLite database holds, live and deleted.
#[derive(Default)]
struct Entries {
    visits: Vec<Visit>,
    downloads: Vec<Download>,
    deleted_visits: Vec<RecoveredVisit>,
    deleted_pages: Vec<RecoveredPage>,
    deleted_downloads: Vec<RecoveredDownload>,
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

/// Read a browser's history database: a SQLite one with its `-wal` file's
/// committed changes (`wal` may be empty), or a WebCache (`wal` unused).
///
/// # Errors
/// When it isn't a SQLite or ESE database, or has none of the tables of a
/// [`Kind`].
pub fn read(database: &[u8], wal: &[u8]) -> Result<History, Error> {
    if is_ese(database) {
        return read_webcache(database);
    }
    if database.starts_with(b"bplist") || database.trim_ascii_start().starts_with(b"<?xml") {
        let mut problems = Vec::new();
        let visits = safari::history_plist(database, &mut problems);
        return Ok(History::of_visits(
            Kind::SafariHistoryPlist,
            visits,
            problems,
        ));
    }
    let db = Database::open_with_wal(database, wal).map_err(|e| Error(e.to_string()))?;
    let kind = Kind::of(&db).ok_or_else(|| Error("not a browser history database".to_owned()))?;
    let mut problems = db.problems.clone();
    let recovery = recovered::Recovery::new(&db, &mut problems);
    let entries = match kind {
        Kind::ChromiumHistory => chromium::history(&db, &recovery, &mut problems),
        Kind::FirefoxPlaces => firefox::places(&db, &recovery, &mut problems),
        Kind::FirefoxDownloads => firefox::legacy_downloads(&db, &recovery, &mut problems),
        Kind::SafariHistory => Entries {
            visits: safari::history_db(&db, &mut problems),
            ..Entries::default()
        },
        // `Kind::of` names SQLite databases only; a WebCache reads as one.
        Kind::WebCache => return read_webcache(database),
        Kind::SafariHistoryPlist
        | Kind::Cookies
        | Kind::Autofill
        | Kind::ExtensionActivity
        | Kind::Preferences => {
            return Err(Error(format!("a {kind:?} database, not history")));
        }
    };
    Ok(History {
        kind,
        visits: entries.visits,
        downloads: entries.downloads,
        deleted_visits: entries.deleted_visits,
        deleted_pages: entries.deleted_pages,
        deleted_downloads: entries.deleted_downloads,
        problems,
    })
}

impl History {
    fn of_visits(kind: Kind, visits: Vec<Visit>, problems: Vec<String>) -> Self {
        Self {
            kind,
            visits,
            downloads: Vec::new(),
            deleted_visits: Vec::new(),
            deleted_pages: Vec::new(),
            deleted_downloads: Vec::new(),
            problems,
        }
    }
}

/// A database's rows of one kind, and its problems.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rows<T> {
    /// The rows, in row order.
    pub rows: Vec<T>,
    /// Damage met.
    pub problems: Vec<String>,
}

/// Open a SQLite database with its log, read with `read`.
fn rows<T>(
    database: &[u8],
    wal: &[u8],
    read: impl FnOnce(&Database<'_>, &mut Vec<String>) -> Vec<T>,
) -> Result<Rows<T>, Error> {
    let db = Database::open_with_wal(database, wal).map_err(|e| Error(e.to_string()))?;
    let mut problems = db.problems.clone();
    let rows = read(&db, &mut problems);
    Ok(Rows { rows, problems })
}

/// Read a cookie database: Chromium's `Cookies` or Firefox's
/// `cookies.sqlite`, with its `-wal` file's committed changes.
///
/// # Errors
/// When it isn't a SQLite database.
pub fn read_cookies(database: &[u8], wal: &[u8]) -> Result<Rows<Cookie>, Error> {
    rows(database, wal, cookies::read)
}

/// Read Chromium's form history (`Web Data`).
///
/// # Errors
/// When it isn't a SQLite database.
pub fn read_autofill(database: &[u8], wal: &[u8]) -> Result<Rows<AutofillEntry>, Error> {
    rows(database, wal, autofill::read)
}

/// Read Chromium's extension activity log (`Extension Activity`).
///
/// # Errors
/// When it isn't a SQLite database.
pub fn read_extension_activity(
    database: &[u8],
    wal: &[u8],
) -> Result<Rows<ExtensionActivity>, Error> {
    rows(database, wal, extensions::activity)
}

const SQLITE_MAGIC: &[u8] = b"SQLite format 3\0";

/// Whether `data` starts like an ESE database (its signature at offset 4).
fn is_ese(data: &[u8]) -> bool {
    data.get(4..8) == Some(&[0xef, 0xcd, 0xab, 0x89][..])
}

fn read_webcache(data: &[u8]) -> Result<History, Error> {
    let db = ese::Database::open(data).map_err(|e| Error(e.to_string()))?;
    if !webcache::is_webcache(&db) {
        return Err(Error("an ESE database, but not a WebCache".to_owned()));
    }
    let mut problems = db.problems.clone();
    let visits = webcache::visits(&db, &mut problems);
    Ok(History::of_visits(Kind::WebCache, visits, problems))
}
