//! Edge's load statistics (`load_statistics.db`, in a profile): which
//! resources each site's pages loaded from other hosts, and which hosts
//! redirected to which, kept to tell trackers apart. A `load_statistics`
//! row is a resource: the site whose page loaded it
//! (`top_level_hostname`), the resource's host, a hash of its URL (not
//! kept here: raw bytes stored as text), its type (Blink's `ResourceType`:
//! 0 main resource, 1 image, 2 style sheet, 3 script, 4 font, …) and when
//! it was last loaded (`last_update`, `WebKit` microseconds). A
//! `redirect_statistics` row is a redirect from a host to another, for a
//! top-level document or not, with its last time. The sites are there even
//! once their history is cleared.

use common::time::Ts;
use sqlite::Database;

use crate::table::{self, Named};

/// A resource a site's page loaded from another host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceLoad {
    /// The row id.
    pub rowid: i64,
    /// The site whose page loaded it.
    pub top_level_hostname: String,
    /// The resource's host.
    pub resource_hostname: String,
    /// What it was: Blink's `ResourceType` (0 main resource, 1 image, 2
    /// style sheet, 3 script, 4 font, 5 raw, 6 SVG document, 7 XSL style
    /// sheet, 8 link prefetch, 9 text track, 10 audio, 11 video, 12
    /// manifest, 13 speculation rules).
    pub resource_type: Option<i64>,
    /// When it was last loaded.
    pub last_update: Option<Ts>,
}

/// A redirect from a host to another.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostRedirect {
    /// The row id.
    pub rowid: i64,
    /// The host redirected from.
    pub source_hostname: String,
    /// The host redirected to.
    pub destination_hostname: String,
    /// Whether it redirected a top-level document (a page, not a
    /// resource).
    pub top_level_document: Option<bool>,
    /// When it last happened.
    pub last_update: Option<Ts>,
}

/// What a `load_statistics.db` holds.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LoadStatistics {
    /// The resources loaded, in row order.
    pub resources: Vec<ResourceLoad>,
    /// The redirects, in row order.
    pub redirects: Vec<HostRedirect>,
    /// Damage met.
    pub problems: Vec<String>,
}

/// Every resource and redirect of a `load_statistics.db`.
pub(crate) fn read(db: &Database<'_>, problems: &mut Vec<String>) -> LoadStatistics {
    LoadStatistics {
        resources: table::read(db, "load_statistics", problems, resource),
        redirects: table::read(db, "redirect_statistics", problems, redirect),
        problems: Vec::new(),
    }
}

fn webkit(row: &Named<'_>, column: &str) -> Option<Ts> {
    row.integer(column)
        .filter(|&t| t != 0)
        .map(Ts::from_webkit_micros)
}

fn resource(row: &Named<'_>) -> ResourceLoad {
    ResourceLoad {
        rowid: row.rowid,
        top_level_hostname: row.text("top_level_hostname").unwrap_or_default(),
        resource_hostname: row.text("resource_hostname").unwrap_or_default(),
        resource_type: row.integer("resource_type"),
        last_update: webkit(row, "last_update"),
    }
}

fn redirect(row: &Named<'_>) -> HostRedirect {
    HostRedirect {
        rowid: row.rowid,
        source_hostname: row.text("source_hostname").unwrap_or_default(),
        destination_hostname: row.text("destination_hostname").unwrap_or_default(),
        top_level_document: row.flag("is_top_level_document"),
        last_update: webkit(row, "last_update"),
    }
}
