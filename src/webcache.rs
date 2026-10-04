//! Internet Explorer and legacy Edge's `WebCacheV01.dat`, an ESE database
//! read with `sootmark-ese`. Its `Containers` table lists the caches; the
//! history ones (`History`, and `MSHist01…` for each day or week) keep one
//! row per page in `Container_<ContainerId>`, its `Url` written
//! `Visited: alice@https://…` (or `:2014051220140513: alice@…` in the
//! period containers). Local files opened in Explorer are there too, as
//! `file:///C:/…` URLs.
//!
//! How a page was reached isn't recorded; the time is `AccessedTime`, the
//! last visit (a FILETIME, UTC).

use common::time::Ts;
use ese::{Database, Row, Table, Value};

use crate::{Transition, Visit};

const CONTAINERS: &str = "Containers";

/// The visits of every history container, container by container.
pub(crate) fn visits(db: &Database<'_>, problems: &mut Vec<String>) -> Vec<Visit> {
    let mut visits = Vec::new();
    for id in history_containers(db, problems) {
        let name = format!("Container_{id}");
        let (Some(table), Ok(mut rows)) = (db.table(&name), db.rows(&name)) else {
            problems.push(format!("{CONTAINERS}: no table {name}"));
            continue;
        };
        visits.extend(rows.by_ref().filter_map(|row| visit(table, &row)));
        problems.extend(rows.problems().iter().map(|p| format!("{name}: {p}")));
    }
    visits
}

/// Whether the file is a WebCache database: an ESE one with `Containers`.
pub(crate) fn is_webcache(db: &Database<'_>) -> bool {
    db.table(CONTAINERS).is_some()
}

/// The ids of the history containers.
fn history_containers(db: &Database<'_>, problems: &mut Vec<String>) -> Vec<i64> {
    let (Some(table), Ok(mut rows)) = (db.table(CONTAINERS), db.rows(CONTAINERS)) else {
        return Vec::new();
    };
    let ids = rows
        .by_ref()
        .filter(|row| {
            row.get(table, "Name")
                .and_then(Value::as_text)
                .is_some_and(|name| name == "History" || name.starts_with("MSHist"))
        })
        .filter_map(|row| row.get(table, "ContainerId").and_then(Value::as_i64))
        .collect();
    problems.extend(rows.problems().iter().map(|p| format!("{CONTAINERS}: {p}")));
    ids
}

fn visit(table: &Table, row: &Row) -> Option<Visit> {
    let get = |name: &str| row.get(table, name);
    let integer = |name: &str| get(name).and_then(Value::as_i64);
    let (user, url) = split_url(get("Url").and_then(Value::as_text)?);
    let time = integer("AccessedTime")
        .and_then(|ticks| u64::try_from(ticks).ok())
        .map(Ts::from_filetime);
    Some(Visit {
        id: integer("EntryId").unwrap_or_default(),
        table: table.name.clone(),
        time,
        url: url.to_owned(),
        title: String::new(),
        user: user.map(str::to_owned),
        transition: Transition::NotRecorded,
        from_visit: None,
        duration: None,
        visit_count: integer("AccessCount"),
        typed: false,
        typed_count: None,
        hidden: false,
        frecency: None,
    })
}

/// `Visited: alice@https://x` as the account and the URL; a URL without
/// the prefix as it is.
fn split_url(raw: &str) -> (Option<&str>, &str) {
    let Some((head, url)) = raw.split_once('@') else {
        return (None, raw);
    };
    // The prefix is `Visited: ` or `:<period>: `, never part of a URL.
    match head.rsplit_once(": ") {
        Some((prefix, user)) if !prefix.contains('/') && !user.contains('/') => (Some(user), url),
        _ => (None, raw),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_with_their_accounts() {
        assert_eq!(
            split_url("Visited: test@https://www.bing.com/search?q=a@b"),
            (Some("test"), "https://www.bing.com/search?q=a@b")
        );
        assert_eq!(
            split_url(":2014051220140513: test@file:///C:/Users/test/x.txt"),
            (Some("test"), "file:///C:/Users/test/x.txt")
        );
        assert_eq!(
            split_url("https://user@example.com/"),
            (None, "https://user@example.com/")
        );
        assert_eq!(split_url("about:blank"), (None, "about:blank"));
        // Windows account names may hold spaces.
        assert_eq!(
            split_url("Visited: John Doe@https://www.bing.com/"),
            (Some("John Doe"), "https://www.bing.com/")
        );
    }
}
