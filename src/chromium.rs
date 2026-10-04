//! Chromium's `History` database, which Chrome, Edge, Brave, Opera and
//! Vivaldi share.
//!
//! A visit is a `visits` row; its page (URL, title, counts) is the `urls`
//! row its `url` column names. A download is a `downloads` row; since
//! history version 24 (Chrome 26) its URLs are `downloads_url_chains` rows,
//! one per redirect, ordered by `chain_index`, and its times are `WebKit`
//! microseconds. Before, the row held one `url` and a `full_path`, and its
//! times were Unix seconds. Deleted visits, pages and downloads are read
//! from recovered records the same way, a deleted download's URL chain from
//! live or recovered chain rows.

use std::collections::HashMap;

use common::time::Ts;
use sqlite::Database;

use crate::recovered::{self, Recovery, Schema};
use crate::table::{self, has_column, Named};
use crate::{
    Download, DownloadState, Entries, Page, PageTransition, RecoveredDownload, Transition, Visit,
};

const CHAINS: &str = "downloads_url_chains";

const SCHEMA: Schema = Schema {
    visits: "visits",
    page_id: "url",
    transition: "transition",
    pages: "urls",
    page,
    visit,
};

/// Every visit with its page, every download with its URL chain, and those
/// deleted that recovery finds.
pub(crate) fn history(
    db: &Database<'_>,
    recovery: &Recovery,
    problems: &mut Vec<String>,
) -> Entries {
    let pages: HashMap<i64, Page> = table::read(db, "urls", problems, |row| {
        let page = page(row);
        (page.id, page)
    })
    .into_iter()
    .collect();
    let visits = table::read_joined(db, ("visits", "url"), ("urls", &pages), problems, visit);
    let (deleted_visits, deleted_pages) =
        recovered::history(recovery, db, &SCHEMA, &pages, &visits);
    let (downloads, deleted_downloads) = downloads(db, recovery, problems);
    Entries {
        visits,
        downloads,
        deleted_visits,
        deleted_pages,
        deleted_downloads,
    }
}

/// A `urls` row: what a visit knows of its page.
fn page(row: &Named<'_>) -> Page {
    let typed_count = row.integer("typed_count");
    Page {
        id: row.integer("id").unwrap_or(row.rowid),
        url: row.text("url").unwrap_or_default(),
        title: row.text("title").unwrap_or_default(),
        last_visit: row.integer("last_visit_time").map(Ts::from_webkit_micros),
        visit_count: row.integer("visit_count"),
        typed: typed_count.is_some_and(|n| n > 0),
        typed_count,
        hidden: row.flag("hidden").unwrap_or(false),
        frecency: None,
    }
}

fn visit(row: &Named<'_>, page: &Page) -> Visit {
    Visit {
        id: row.integer("id").unwrap_or(row.rowid),
        table: "visits".to_owned(),
        time: row.integer("visit_time").map(Ts::from_webkit_micros),
        url: page.url.clone(),
        title: page.title.clone(),
        user: None,
        // Stored as a signed 32-bit value: the low 32 bits are the bits.
        transition: Transition::Chromium(PageTransition(
            row.integer("transition").unwrap_or(0) as u32
        )),
        from_visit: row.reference("from_visit"),
        duration: row
            .integer("visit_duration")
            .and_then(|micros| u64::try_from(micros).ok())
            .map(std::time::Duration::from_micros),
        visit_count: page.visit_count,
        typed: page.typed,
        typed_count: page.typed_count,
        hidden: page.hidden,
        frecency: None,
    }
}

/// Every download with its URL chain, and those deleted.
fn downloads(
    db: &Database<'_>,
    recovery: &Recovery,
    problems: &mut Vec<String>,
) -> (Vec<Download>, Vec<RecoveredDownload>) {
    if !has_column(db, "downloads", "target_path") {
        let live = table::read(db, "downloads", problems, legacy_download);
        let deleted = recovered::downloads(recovery, db, "downloads", legacy_download, &live);
        return (live, deleted);
    }
    let chains = url_chains(table::read(db, CHAINS, problems, link));
    let recovered_chains = url_chains(
        recovery
            .read(db, CHAINS, link)
            .into_iter()
            .map(|found| found.entry),
    );
    let live = table::read(db, "downloads", problems, |row| {
        let id = row.integer("id").unwrap_or(row.rowid);
        download(row, id, chains.get(&id).cloned().unwrap_or_default())
    });
    // A deleted download's chain rows are usually deleted with it.
    let deleted_download = |row: &Named<'_>| {
        let id = row.integer("id").unwrap_or(row.rowid);
        let chain = chains.get(&id).or_else(|| recovered_chains.get(&id));
        download(row, id, chain.cloned().unwrap_or_default())
    };
    let deleted = recovered::downloads(recovery, db, "downloads", deleted_download, &live);
    (live, deleted)
}

/// A `downloads_url_chains` row: its download, its place in the chain, its
/// URL.
fn link(row: &Named<'_>) -> (Option<i64>, i64, String) {
    (
        row.integer("id"),
        row.integer("chain_index").unwrap_or(0),
        row.text("url").unwrap_or_default(),
    )
}

/// Each download's URLs, in `chain_index` order, one per index.
fn url_chains(
    links: impl IntoIterator<Item = (Option<i64>, i64, String)>,
) -> HashMap<i64, Vec<String>> {
    let mut chains: HashMap<i64, Vec<(i64, String)>> = HashMap::new();
    for (id, index, url) in links {
        if let Some(id) = id {
            chains.entry(id).or_default().push((index, url));
        }
    }
    chains
        .into_iter()
        .map(|(id, mut chain)| {
            chain.sort_by_key(|&(index, _)| index);
            chain.dedup_by_key(|&mut (index, _)| index);
            (id, chain.into_iter().map(|(_, url)| url).collect())
        })
        .collect()
}

/// A download from history version 24 on.
fn download(row: &Named<'_>, id: i64, url_chain: Vec<String>) -> Download {
    Download {
        id,
        url: url_chain.last().cloned().unwrap_or_default(),
        url_chain,
        target_path: row.text("target_path").unwrap_or_default(),
        current_path: row.text("current_path"),
        start: row.integer("start_time").map(Ts::from_webkit_micros),
        end: row.integer("end_time").map(Ts::from_webkit_micros),
        received_bytes: row.integer("received_bytes"),
        total_bytes: row.integer("total_bytes"),
        state: row.integer("state").map(state),
        danger_type: row.integer("danger_type"),
        interrupt_reason: row.integer("interrupt_reason"),
        referrer: row.text("referrer"),
        tab_url: row.text("tab_url"),
        mime_type: row.text("mime_type"),
        opened: row.flag("opened"),
        deleted: None,
    }
}

/// A download from before history version 24: one URL, Unix seconds.
fn legacy_download(row: &Named<'_>) -> Download {
    let url = row.text("url").unwrap_or_default();
    Download {
        id: row.integer("id").unwrap_or(row.rowid),
        url_chain: vec![url.clone()],
        url,
        target_path: row.text("full_path").unwrap_or_default(),
        current_path: None,
        start: row.integer("start_time").map(Ts::from_unix_seconds),
        end: row.integer("end_time").map(Ts::from_unix_seconds),
        received_bytes: row.integer("received_bytes"),
        total_bytes: row.integer("total_bytes"),
        state: row.integer("state").map(state),
        danger_type: None,
        interrupt_reason: None,
        referrer: None,
        tab_url: None,
        mime_type: None,
        opened: row.flag("opened"),
        deleted: None,
    }
}

/// `history::DownloadState`: 3 was a transient state (bug 140687; before
/// version 24, "removing").
fn state(value: i64) -> DownloadState {
    match value {
        0 => DownloadState::InProgress,
        1 => DownloadState::Complete,
        2 => DownloadState::Cancelled,
        4 => DownloadState::Interrupted,
        other => DownloadState::Other(other),
    }
}
