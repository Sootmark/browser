//! Deleted history: the records `sootmark-sqlite` recovers, read as rows of
//! their tables and told apart from the live rows.
//!
//! Recovery returns records of deleted rows from the database as it reads
//! now (freeblocks, unallocated space, freelist pages), then from the older
//! page versions a write-ahead log keeps (superseded and uncommitted frames,
//! the file's copies of pages the log replaces), which hold rows deleted
//! since and rows changed since alike. A record whose key a live row has
//! (a visit's page URL and time, a page's URL, a download's start and
//! destination) is an earlier state of that row, and is left out. Of
//! several records with the same key, the strongest is kept, where the
//! first was met. A record that lost its key can't be told apart: it is
//! kept.

use std::cmp::Reverse;
use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::hash::Hash;

use common::time::Ts;
use sqlite::{Confidence, Database, Evidence, RecoveredRecord};

use crate::table::{self, Named};
use crate::{
    Download, Page, PageSource, Provenance, RecoveredDownload, RecoveredPage, RecoveredVisit,
    Transition, Visit,
};

/// A database's recovered records.
pub(crate) struct Recovery {
    /// Those of the database as it reads now, then those of older page
    /// versions.
    records: Vec<RecoveredRecord>,
}

impl Recovery {
    /// Recover the deleted records of `db`; damage met goes to `problems`.
    pub(crate) fn new(db: &Database<'_>, problems: &mut Vec<String>) -> Self {
        let recovered = db.recover();
        problems.extend(recovered.problems.iter().map(|p| format!("recovery: {p}")));
        Self {
            records: recovered
                .records
                .into_iter()
                .chain(recovered.older_versions)
                .collect(),
        }
    }

    /// The records that fit `table` best, each converted as a row of it.
    pub(crate) fn read<T>(
        &self,
        db: &Database<'_>,
        table: &str,
        mut convert: impl FnMut(&Named<'_>) -> T,
    ) -> Vec<Found<T>> {
        let Some(columns) = table::columns(db, table) else {
            return Vec::new();
        };
        self.records
            .iter()
            .filter(|record| record.table.as_deref() == Some(table))
            .map(|record| Found {
                entry: convert(&Named::recovered(&columns, record)),
                provenance: provenance(table, &columns, record),
            })
            .collect()
    }
}

/// An entry read from a recovered record.
pub(crate) struct Found<T> {
    pub(crate) entry: T,
    pub(crate) provenance: Provenance,
}

fn provenance(table: &str, columns: &[String], record: &RecoveredRecord) -> Provenance {
    Provenance {
        table: table.to_owned(),
        rowid: record.rowid,
        page: record.page,
        offset: record.offset,
        page_state: record.page_state,
        area: record.area,
        evidence: record.evidence,
        confidence: record.confidence,
        also_fits: record.also_fits.clone(),
        lost: columns
            .iter()
            .zip(&record.values)
            .filter(|(_, value)| value.is_none())
            .map(|(column, _)| column.clone())
            .collect(),
        truncated: record.truncated,
    }
}

/// How much of a record survived: its confidence, then its evidence, then
/// the fewer values lost.
fn strength(provenance: &Provenance) -> (Confidence, u8, Reverse<usize>) {
    let evidence = match provenance.evidence {
        Evidence::CellPointer => 3,
        Evidence::Cell => 2,
        Evidence::Record => 1,
        Evidence::Shape => 0,
    };
    (
        provenance.confidence,
        evidence,
        Reverse(provenance.lost.len()),
    )
}

/// The entries whose key no live entry has, each key once: of those with
/// the same key, the one ranked highest, where the first was met. Entries
/// without a key are kept.
fn deleted<T, K: Hash + Eq, R: Ord>(
    found: Vec<Found<T>>,
    key: impl Fn(&T) -> Option<K>,
    live: &HashSet<K>,
    rank: impl Fn(&Found<T>) -> R,
) -> Vec<Found<T>> {
    let mut kept: Vec<Found<T>> = Vec::new();
    let mut at: HashMap<K, usize> = HashMap::new();
    for found in found {
        let Some(key) = key(&found.entry) else {
            kept.push(found);
            continue;
        };
        if live.contains(&key) {
            continue;
        }
        match at.entry(key) {
            Entry::Occupied(slot) => {
                let i = *slot.get();
                if rank(&found) > rank(&kept[i]) {
                    kept[i] = found;
                }
            }
            Entry::Vacant(slot) => {
                slot.insert(kept.len());
                kept.push(found);
            }
        }
    }
    kept
}

/// A browser's visits and pages: their tables, the columns recovery needs
/// by name, and how their rows read.
pub(crate) struct Schema {
    pub(crate) visits: &'static str,
    /// The visits column naming the page.
    pub(crate) page_id: &'static str,
    pub(crate) transition: &'static str,
    pub(crate) pages: &'static str,
    pub(crate) page: fn(&Named<'_>) -> Page,
    pub(crate) visit: fn(&Named<'_>, &Page) -> Visit,
}

/// The deleted visits and pages, joined with the live pages and the
/// recovered ones.
pub(crate) fn history(
    recovery: &Recovery,
    db: &Database<'_>,
    schema: &Schema,
    live_pages: &HashMap<i64, Page>,
    live_visits: &[Visit],
) -> (Vec<RecoveredVisit>, Vec<RecoveredPage>) {
    let pages = recovery.read(db, schema.pages, schema.page);
    let visits = deleted_visits(recovery, db, schema, live_pages, &pages, live_visits);
    (visits, deleted_pages(pages, live_pages))
}

/// A visit's key: its page's URL and its time.
fn visit_key(visit: &Visit) -> Option<(String, Ts)> {
    Some((visit.url.clone(), visit.time?))
}

/// A page's key: its URL.
fn page_key(page: &Page) -> Option<String> {
    Some(page.url.clone()).filter(|url| !url.is_empty())
}

fn deleted_visits(
    recovery: &Recovery,
    db: &Database<'_>,
    schema: &Schema,
    live_pages: &HashMap<i64, Page>,
    recovered_pages: &[Found<Page>],
    live_visits: &[Visit],
) -> Vec<RecoveredVisit> {
    let recovered_pages = by_id(recovered_pages);
    let missing = Page::default();
    let found = recovery.read(db, schema.visits, |row| {
        let id = row.integer(schema.page_id);
        let (page, source) = match id.map(|id| (live_pages.get(&id), recovered_pages.get(&id))) {
            Some((Some(page), _)) => (page, PageSource::Live),
            Some((None, Some(page))) => (*page, PageSource::Recovered),
            _ => (&missing, PageSource::NotFound),
        };
        ((schema.visit)(row, page), source)
    });
    let live: HashSet<_> = live_visits.iter().filter_map(visit_key).collect();
    deleted(
        found,
        |(visit, _)| visit_key(visit),
        &live,
        |found| strength(&found.provenance),
    )
    .into_iter()
    .map(|Found { entry, provenance }| {
        let (mut visit, page) = entry;
        if provenance.lost.iter().any(|c| c == schema.transition) {
            visit.transition = Transition::NotRecorded;
        }
        RecoveredVisit {
            visit,
            page,
            provenance,
        }
    })
    .collect()
}

/// The recovered pages whose rowid is known, by it: of several versions of
/// a page, the one last visited.
fn by_id(pages: &[Found<Page>]) -> HashMap<i64, &Page> {
    let mut by_id: HashMap<i64, &Page> = HashMap::new();
    for found in pages {
        let Some(id) = found.provenance.rowid else {
            continue;
        };
        let page = by_id.entry(id).or_insert(&found.entry);
        if last_visited(&found.entry) > last_visited(page) {
            *page = &found.entry;
        }
    }
    by_id
}

/// When a page was last visited, as ticks to compare.
fn last_visited(page: &Page) -> Option<i64> {
    page.last_visit.and_then(|ts| ts.ticks())
}

/// The recovered pages whose URL no live page has: of several versions of
/// a page, the one last visited.
fn deleted_pages(pages: Vec<Found<Page>>, live_pages: &HashMap<i64, Page>) -> Vec<RecoveredPage> {
    let live: HashSet<_> = live_pages.values().filter_map(page_key).collect();
    deleted(pages, page_key, &live, |found| {
        (last_visited(&found.entry), strength(&found.provenance))
    })
    .into_iter()
    .map(|Found { entry, provenance }| RecoveredPage {
        page: entry,
        provenance,
    })
    .collect()
}

/// A download's key: when it began and where it was saved.
fn download_key(download: &Download) -> Option<(Ts, String)> {
    Some((download.start?, download.target_path.clone()))
}

/// The deleted downloads of `table`, its records converted as its rows.
pub(crate) fn downloads(
    recovery: &Recovery,
    db: &Database<'_>,
    table: &str,
    convert: impl FnMut(&Named<'_>) -> Download,
    live_downloads: &[Download],
) -> Vec<RecoveredDownload> {
    let live: HashSet<_> = live_downloads.iter().filter_map(download_key).collect();
    deleted(
        recovery.read(db, table, convert),
        download_key,
        &live,
        |found| strength(&found.provenance),
    )
    .into_iter()
    .map(|Found { entry, provenance }| RecoveredDownload {
        download: entry,
        provenance,
    })
    .collect()
}
