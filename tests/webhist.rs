//! Edge's load statistics, Safari's cookies, Google Analytics' cookies,
//! Opera's history and Java's cache index files in plaso's test files
//! (Apache-2.0, `tests/fixtures/plaso/`, see its NOTICE): every event
//! plaso's `sqlite/edge_load_statistics`, `binary_cookies`, cookie plugins
//! (`google_analytics_utma`, `_utmb`, `_utmt`, `_utmz`, through
//! `chrome_17_cookies`, `chrome_66_cookies`, `firefox_2_cookies`,
//! `firefox_10_cookies` and `binary_cookies`), `opera_global`,
//! `opera_typed_history` and `java_idx` make, read the same
//! (`tests/oracle/plaso-webhist.tsv`, written from plaso's output by
//! `tests/oracle/plaso_tsv.py`; see `tests/oracle/README`).
//!
//! Where this crate differs from plaso, the test shows how:
//!
//! - plaso gives the Google Analytics events of Safari's cookies the
//!   cookie's name as their URL (`__utma`), where its other cookie parsers
//!   give the cookie's URL; this crate's cookies keep their host and path.
//! - plaso stores an Opera page's title only when it differs from its URL;
//!   this crate keeps it as written.
//! - plaso reads a `__utmb` time in milliseconds as seconds unless the
//!   count before it is 8 or 9; one of Firefox's (`…5.1383170228399`) has
//!   13 digits after a count of 5, and plaso puts it in the year 45,800.
//!   This crate reads 13 digits as milliseconds (2013-10-30).
//! - plaso's `firefox_10_cookies` plugin reads `firefox_2_cookies.sqlite`
//!   too (the columns it needs are there), so each Google Analytics event
//!   of that file is in its output twice, once per plugin (but for the
//!   `__utmb` below, there once); this crate's cookies are read once.
//! - Edge's `redirect_statistics` (one row here) and the Java files' HTTP
//!   headers beyond `date` aren't read by plaso.

mod compare;
mod support;

use browser::{detect, Cookie, CookieStore, GoogleAnalytics, Kind, TypedEntry};
use common::time::Ts;

fn fixture(name: &str) -> Vec<u8> {
    support::fixture(&format!("plaso/{name}"))
}

/// A time in microseconds, rounded down as plaso's integer divisions do.
fn micros(time: Ts) -> i64 {
    time.ticks().unwrap_or(0).div_euclid(10)
}

fn escape(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('\t', "\\t")
        .replace('\r', "\\r")
        .replace('\n', "\\n")
}

/// A TSV line as `plaso_tsv.py` writes one.
fn line(file: &str, data_type: &str, desc: &str, time: Option<Ts>, fields: &[String]) -> String {
    let mut values = vec![
        file.to_owned(),
        data_type.to_owned(),
        desc.to_owned(),
        time.map_or(0, micros).to_string(),
    ];
    values.extend(fields.iter().map(|field| escape(field)));
    values.join("\t")
}

fn text(value: Option<impl ToString>) -> String {
    value.map(|v| v.to_string()).unwrap_or_default()
}

fn load_statistics(lines: &mut Vec<String>) {
    let data = fixture("load_statistics.db.gz");
    assert_eq!(
        detect("load_statistics.db", &data),
        Some(Kind::LoadStatistics)
    );
    let statistics = browser::read_load_statistics(&data, &[]).unwrap();
    assert_eq!(statistics.problems, Vec::<String>::new());
    for resource in &statistics.resources {
        lines.push(line(
            "load_statistics.db",
            "edge:resources:load_statistics",
            "Last Updated Time",
            resource.last_update,
            &[
                resource.top_level_hostname.clone(),
                resource.resource_hostname.clone(),
                text(resource.resource_type),
            ],
        ));
    }
    // Beyond plaso: the redirect.
    let [redirect] = statistics.redirects.as_slice() else {
        panic!("{:?}", statistics.redirects)
    };
    assert_eq!(
        (
            redirect.source_hostname.as_str(),
            redirect.destination_hostname.as_str(),
            redirect.top_level_document,
        ),
        ("www.bing.com", "www.bing.com", Some(false))
    );
    assert_eq!(
        redirect.last_update.unwrap().to_iso8601().unwrap(),
        "2023-03-13T01:57:44.8960210Z"
    );
}

/// Safari's cookies, their creation and expiry.
fn binary_cookies(lines: &mut Vec<String>) -> Vec<Cookie> {
    let data = fixture("Cookies.binarycookies");
    assert_eq!(detect("Cookies", &data), Some(Kind::SafariCookies));
    let cookies = browser::read_binary_cookies(&data).unwrap();
    assert_eq!(cookies.problems, Vec::<String>::new());
    for cookie in &cookies.rows {
        assert_eq!(cookie.store, CookieStore::Safari);
        let flags = u8::from(cookie.secure) | u8::from(cookie.http_only) << 2;
        for (desc, time) in [
            ("Creation Time", cookie.created),
            ("Expiration Time", cookie.expires),
        ] {
            lines.push(line(
                "Cookies.binarycookies",
                "safari:cookie:entry",
                desc,
                time,
                &[
                    cookie.host.clone(),
                    cookie.name.clone(),
                    cookie.path.clone(),
                    cookie.value.clone(),
                    flags.to_string(),
                ],
            ));
        }
    }
    cookies.rows
}

/// The URL plaso gives a cookie's Google Analytics events.
fn plaso_url(cookie: &Cookie) -> String {
    match cookie.store {
        CookieStore::Safari => cookie.name.clone(),
        CookieStore::Chromium | CookieStore::Firefox => format!(
            "{}://{}{}",
            if cookie.secure { "https" } else { "http" },
            cookie.host.trim_start_matches('.'),
            cookie.path
        ),
    }
}

/// The Google Analytics events of one decoded cookie.
fn analytics_lines(file: &str, cookie: &Cookie, lines: &mut Vec<String>) {
    let Some(analytics) = &cookie.analytics else {
        return;
    };
    let url = plaso_url(cookie);
    let variable = |variables: &[(String, String)], key: &str| {
        variables
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
            .unwrap_or_default()
    };
    let (data_type, desc, times, fields) = match analytics {
        GoogleAnalytics::Utma {
            domain_hash,
            visitor_id,
            first_visit,
            previous_visit,
            last_visit,
            sessions,
        } => (
            "utma",
            "Visited Time",
            vec![*first_visit, *previous_visit, *last_visit],
            vec![
                text(domain_hash.as_ref()),
                text(visitor_id.as_ref()),
                text(*sessions),
            ],
        ),
        GoogleAnalytics::Utmb {
            domain_hash,
            pages_viewed,
            last_visit,
        } => (
            "utmb",
            "Last Visited Time",
            vec![*last_visit],
            vec![text(domain_hash.as_ref()), text(*pages_viewed)],
        ),
        GoogleAnalytics::Utmt { last_visit } => {
            ("utmt", "Last Visited Time", vec![*last_visit], Vec::new())
        }
        GoogleAnalytics::Utmz {
            domain_hash,
            last_visit,
            sessions,
            sources,
            variables,
        } => {
            let mut fields = vec![text(domain_hash.as_ref()), text(*sessions), text(*sources)];
            fields.extend(
                ["utmcsr", "utmccn", "utmcmd", "utmctr", "utmcct"]
                    .map(|key| variable(variables, key)),
            );
            ("utmz", "Last Visited Time", vec![*last_visit], fields)
        }
    };
    let mut all = vec![url, cookie.name.clone()];
    all.extend(fields);
    let data_type = format!("cookie:google:analytics:{data_type}");
    let times: Vec<Ts> = times.into_iter().flatten().collect();
    if times.is_empty() {
        lines.push(line(file, &data_type, "Not a time", None, &all));
    }
    for time in times {
        lines.push(line(file, &data_type, desc, Some(time), &all));
    }
}

fn analytics(safari: &[Cookie], lines: &mut Vec<String>) {
    for name in [
        "cookies.db",
        "Cookies-68.0.3440.106",
        "firefox_2_cookies.sqlite",
        "firefox_10_cookies.sqlite",
    ] {
        let cookies = browser::read_cookies(&fixture(&format!("{name}.gz")), &[]).unwrap();
        assert_eq!(cookies.problems, Vec::<String>::new(), "{name}");
        for cookie in &cookies.rows {
            analytics_lines(name, cookie, lines);
        }
    }
    for cookie in safari {
        analytics_lines("Cookies.binarycookies", cookie, lines);
    }
}

fn opera(lines: &mut Vec<String>) {
    let data = fixture("global_history.dat");
    assert_eq!(detect("x", &data), Some(Kind::OperaGlobalHistory));
    let history = browser::read(&data, &[]).unwrap();
    assert_eq!(history.problems, Vec::<String>::new());
    for visit in &history.visits {
        let popularity = visit.frecency.unwrap();
        lines.push(line(
            "global_history.dat",
            "opera:history:entry",
            "Last Visited Time",
            visit.time,
            &[
                visit.url.clone(),
                if visit.title == visit.url {
                    String::new()
                } else {
                    visit.title.clone()
                },
                if popularity < 0 {
                    "First and Only Visit"
                } else {
                    "Last Visit"
                }
                .to_owned(),
                popularity.to_string(),
            ],
        ));
    }
    let data = fixture("typed_history.xml");
    assert_eq!(detect("x", &data), Some(Kind::OperaTypedHistory));
    assert!(browser::read(&data, &[]).is_err());
    let typed = browser::read_opera_typed_history(&data).unwrap();
    assert_eq!(typed.problems, Vec::<String>::new());
    for entry in &typed.rows {
        let (kind, selection) = match &entry.entry {
            TypedEntry::Typed => ("text", "Manually typed."),
            TypedEntry::Selected => ("selected", "Filled from autocomplete."),
            TypedEntry::Other(other) => (other.as_str(), ""),
        };
        lines.push(line(
            "typed_history.xml",
            "opera:history:typed_entry",
            "Last Typed Time",
            entry.time,
            &[entry.url.clone(), kind.to_owned(), selection.to_owned()],
        ));
    }
}

fn java(lines: &mut Vec<String>) {
    for name in ["java.idx", "java_602.idx"] {
        let data = fixture(name);
        assert_eq!(detect(name, &data), Some(Kind::JavaIdx));
        let entry = browser::read_java_idx(&data).unwrap();
        assert_eq!(entry.problems, Vec::<String>::new(), "{name}");
        let fields = [
            entry.url.clone(),
            entry.version.to_string(),
            text(entry.ip_address.as_ref()),
        ];
        for (desc, time) in [
            ("Content Modification Time", entry.modified),
            ("Downloaded Time", entry.downloaded),
            ("Expiration Time", entry.expires),
        ] {
            if time.is_some() {
                lines.push(line(name, "java:download:idx", desc, time, &fields));
            }
        }
    }
}

/// The start of the Google Analytics lines of `firefox_2_cookies.sqlite`,
/// which plaso's output has twice.
const FIREFOX_2_ANALYTICS: &str = "firefox_2_cookies.sqlite\tcookie:google:analytics:";

/// plaso's lines that this crate reads otherwise, and how (see the module
/// documentation).
const DIFFERENCES: [(&str, &str); 1] = [(
    "firefox_2_cookies.sqlite\tcookie:google:analytics:utmb\tLast Visited Time\t\
     1383170228399000000\thttp://theonion.com/\t__utmb\t207318870\t40",
    "firefox_2_cookies.sqlite\tcookie:google:analytics:utmb\tLast Visited Time\t\
     1383170228399000\thttp://theonion.com/\t__utmb\t207318870\t40",
)];

#[test]
fn as_plaso_reads_them() {
    let mut lines = Vec::new();
    load_statistics(&mut lines);
    let safari = binary_cookies(&mut lines);
    analytics(&safari, &mut lines);
    opera(&mut lines);
    java(&mut lines);
    let oracle = include_str!("oracle/plaso-webhist.tsv");
    let mut seen_once = std::collections::BTreeSet::new();
    let expected: Vec<&str> = oracle
        .lines()
        // Every other copy of a doubled line.
        .filter(|line| {
            !line.starts_with(FIREFOX_2_ANALYTICS) || {
                let kept = !seen_once.remove(line);
                if kept {
                    seen_once.insert(*line);
                }
                kept
            }
        })
        .map(|plaso| {
            DIFFERENCES
                .iter()
                .find(|(theirs, _)| *theirs == plaso)
                .map_or(plaso, |(_, ours)| ours)
        })
        .collect();
    compare::assert_same_lines(&lines, &expected);
}

#[test]
fn java_headers_and_status() {
    let entry = browser::read_java_idx(&fixture("java_602.idx")).unwrap();
    assert_eq!(entry.status(), Some("HTTP/1.1 200 OK"));
    assert_eq!(entry.header("Content-Type"), Some("text/plain"));
    assert_eq!(entry.content_size, 14_180);
    assert_eq!(entry.http_headers.len(), 8);
    assert_eq!(
        entry.header("deploy_resource_codebase_ip"),
        Some("10.18.250.10")
    );
    let entry = browser::read_java_idx(&fixture("java.idx")).unwrap();
    assert_eq!(entry.signed, Some(false));
    assert_eq!(entry.http_headers.len(), 7);
}
