//! Firefox's `places.sqlite` and, up to Firefox 25, `downloads.sqlite`.
//!
//! A visit is a `moz_historyvisits` row; its page (URL, title, counts) is
//! the `moz_places` row its `place_id` names. Since Firefox 26 a download
//! is two annotations of the page it came from (`moz_annos`, named through
//! `moz_anno_attributes`): `downloads/destinationFileURI`, the saved
//! file's `file://` URI, added when the download began, and
//! `downloads/metaData`, a JSON object written when it ended
//! (`{"state":1,"deleted":false,"endTime":1689722333589,"fileSize":66383750}`,
//! the end in milliseconds). Before, `downloads.sqlite` held one
//! `moz_downloads` row per download. Times are `PRTime`, microseconds since
//! 1970. Deleted visits, pages and `moz_downloads` rows are read from
//! recovered records the same way; deleted annotations are not.

use std::collections::HashMap;

use common::json::{self, Json};
use common::time::Ts;
use sqlite::Database;

use crate::recovered::{self, Recovery, Schema};
use crate::table::{self, Named};
use crate::{Download, DownloadState, Entries, Page, Transition, Visit, VisitType};

const DESTINATION: &str = "downloads/destinationFileURI";
const METADATA: &str = "downloads/metaData";

const SCHEMA: Schema = Schema {
    visits: "moz_historyvisits",
    page_id: "place_id",
    transition: "visit_type",
    pages: "moz_places",
    page,
    visit,
};

/// A `places.sqlite` database's visits and downloads, and the visits and
/// pages deleted that recovery finds.
pub(crate) fn places(
    db: &Database<'_>,
    recovery: &Recovery,
    problems: &mut Vec<String>,
) -> Entries {
    let pages: HashMap<i64, Page> = table::read(db, "moz_places", problems, |row| {
        let page = page(row);
        (page.id, page)
    })
    .into_iter()
    .collect();
    let visits = table::read_joined(
        db,
        ("moz_historyvisits", "place_id"),
        ("moz_places", &pages),
        problems,
        visit,
    );
    let (deleted_visits, deleted_pages) =
        recovered::history(recovery, db, &SCHEMA, &pages, &visits);
    Entries {
        downloads: downloads(db, &pages, problems),
        visits,
        deleted_visits,
        deleted_pages,
        deleted_downloads: Vec::new(),
    }
}

/// A `moz_places` row: what a visit or download knows of its page.
fn page(row: &Named<'_>) -> Page {
    Page {
        id: row.integer("id").unwrap_or(row.rowid),
        url: row.text("url").unwrap_or_default(),
        title: row.text("title").unwrap_or_default(),
        last_visit: row.integer("last_visit_date").map(Ts::from_unix_micros),
        visit_count: row.integer("visit_count"),
        typed: row.flag("typed").unwrap_or(false),
        typed_count: None,
        hidden: row.flag("hidden").unwrap_or(false),
        frecency: row.integer("frecency"),
    }
}

fn visit(row: &Named<'_>, page: &Page) -> Visit {
    Visit {
        id: row.integer("id").unwrap_or(row.rowid),
        table: "moz_historyvisits".to_owned(),
        time: row.integer("visit_date").map(Ts::from_unix_micros),
        url: page.url.clone(),
        title: page.title.clone(),
        user: None,
        transition: Transition::Firefox(VisitType::from_raw(
            row.integer("visit_type").unwrap_or(0),
        )),
        from_visit: row.reference("from_visit"),
        duration: None,
        visit_count: page.visit_count,
        typed: page.typed,
        typed_count: None,
        hidden: page.hidden,
        frecency: page.frecency,
    }
}

/// A page's download annotations, as met.
struct Annotated {
    /// The first annotation's row id.
    id: i64,
    place: i64,
    /// The destination URI, and when it was added.
    destination: Option<(String, Option<i64>)>,
    metadata: Option<String>,
}

/// Every download, from the annotations of the pages they came from, in
/// the order their first annotation was written.
fn downloads(
    db: &Database<'_>,
    pages: &HashMap<i64, Page>,
    problems: &mut Vec<String>,
) -> Vec<Download> {
    let names = table::read(db, "moz_anno_attributes", problems, |row| {
        (row.integer("id").unwrap_or(row.rowid), row.text("name"))
    });
    let attribute = |name: &str| {
        names
            .iter()
            .find(|(_, n)| n.as_deref() == Some(name))
            .map(|&(id, _)| id)
    };
    let (Some(destination), metadata) = (attribute(DESTINATION), attribute(METADATA)) else {
        return Vec::new();
    };
    annotations(db, destination, metadata, problems)
        .into_iter()
        .map(|a| annotated_download(a, pages, problems))
        .collect()
}

/// The download annotations of every page, grouped by page.
fn annotations(
    db: &Database<'_>,
    destination: i64,
    metadata: Option<i64>,
    problems: &mut Vec<String>,
) -> Vec<Annotated> {
    let mut grouped: Vec<Annotated> = Vec::new();
    let mut by_place: HashMap<i64, usize> = HashMap::new();
    let rows = table::read(db, "moz_annos", problems, |row| {
        (
            row.rowid,
            row.integer("place_id"),
            row.integer("anno_attribute_id"),
            row.text("content"),
            row.integer("dateAdded"),
        )
    });
    for (rowid, place, attribute, content, added) in rows {
        let (Some(place), Some(attribute), Some(content)) = (place, attribute, content) else {
            continue;
        };
        let is_destination = attribute == destination;
        if !is_destination && Some(attribute) != metadata {
            continue;
        }
        let at = *by_place.entry(place).or_insert_with(|| {
            grouped.push(Annotated {
                id: rowid,
                place,
                destination: None,
                metadata: None,
            });
            grouped.len() - 1
        });
        if is_destination {
            grouped[at].destination = Some((content, added));
        } else {
            grouped[at].metadata = Some(content);
        }
    }
    grouped
}

fn annotated_download(
    annotated: Annotated,
    pages: &HashMap<i64, Page>,
    problems: &mut Vec<String>,
) -> Download {
    let id = annotated.id;
    let url = if let Some(page) = pages.get(&annotated.place) {
        page.url.clone()
    } else {
        problems.push(format!(
            "moz_annos: download {id}'s page is not in moz_places"
        ));
        String::new()
    };
    let metadata = match annotated.metadata.as_deref().map(json::parse) {
        Some(Ok(metadata)) => metadata,
        Some(Err(e)) => {
            problems.push(format!("moz_annos: download {id}'s metadata: {e}"));
            Json::Null
        }
        None => Json::Null,
    };
    let (target_path, added) = annotated.destination.unwrap_or_default();
    Download {
        id,
        url,
        url_chain: Vec::new(),
        target_path,
        current_path: None,
        start: added.map(Ts::from_unix_micros),
        end: metadata
            .get("endTime")
            .and_then(Json::as_i64)
            .map(Ts::from_unix_millis),
        received_bytes: None,
        total_bytes: metadata.get("fileSize").and_then(Json::as_i64),
        state: metadata.get("state").and_then(Json::as_i64).map(state),
        danger_type: None,
        interrupt_reason: None,
        referrer: None,
        tab_url: None,
        mime_type: None,
        opened: None,
        deleted: match metadata.get("deleted") {
            Some(Json::Bool(deleted)) => Some(*deleted),
            _ => None,
        },
    }
}

/// A `downloads.sqlite` database's downloads (Firefox 25 and older), and
/// those deleted that recovery finds.
pub(crate) fn legacy_downloads(
    db: &Database<'_>,
    recovery: &Recovery,
    problems: &mut Vec<String>,
) -> Entries {
    let downloads = table::read(db, "moz_downloads", problems, legacy_download);
    Entries {
        deleted_downloads: recovered::downloads(
            recovery,
            db,
            "moz_downloads",
            legacy_download,
            &downloads,
        ),
        downloads,
        ..Entries::default()
    }
}

fn legacy_download(row: &Named<'_>) -> Download {
    Download {
        id: row.integer("id").unwrap_or(row.rowid),
        url: row.text("source").unwrap_or_default(),
        url_chain: Vec::new(),
        target_path: row.text("target").unwrap_or_default(),
        current_path: row.text("tempPath"),
        start: row.integer("startTime").map(Ts::from_unix_micros),
        end: row.integer("endTime").map(Ts::from_unix_micros),
        received_bytes: row.integer("currBytes"),
        // -1 when the size was not announced.
        total_bytes: row.integer("maxBytes").filter(|&n| n >= 0),
        state: row.integer("state").map(state),
        danger_type: None,
        interrupt_reason: None,
        referrer: row.text("referrer"),
        tab_url: None,
        mime_type: row.text("mimeType"),
        opened: None,
        deleted: None,
    }
}

/// `nsIDownloadManager`'s states, which the annotations' metadata kept:
/// 5 (queued) and 7 (scanning) are as written.
fn state(value: i64) -> DownloadState {
    match value {
        0 => DownloadState::InProgress,
        1 => DownloadState::Complete,
        2 => DownloadState::Interrupted,
        3 => DownloadState::Cancelled,
        4 => DownloadState::Paused,
        6 | 9 => DownloadState::Blocked,
        8 => DownloadState::Dirty,
        other => DownloadState::Other(other),
    }
}
