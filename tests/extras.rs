//! Cookies, form history, extensions and Safari's history in plaso's test
//! files (Apache-2.0, `tests/fixtures/plaso/`, see its NOTICE): every
//! event plaso's `chrome_17_cookies`, `chrome_66_cookies`,
//! `firefox_2_cookies`, `firefox_10_cookies`, `chrome_autofill`,
//! `chrome_extension_activity`, `chrome_preferences`, `safari_historydb`
//! and `safari_history` read, read the same
//! (`tests/oracle/plaso-extras.tsv`, written from plaso's output).

mod support;

use std::collections::BTreeSet;

use browser::{detect, Kind};
use common::time::Ts;

/// Microseconds, as plaso rounds them.
fn micros(time: Ts) -> String {
    ((time.ticks().unwrap() + 5).div_euclid(10)).to_string()
}

fn fixture(name: &str) -> Vec<u8> {
    support::fixture(&format!("plaso/{name}.gz"))
}

/// Each cookie's times, as plaso's cookie parsers give them.
fn cookie_rows(got: &mut BTreeSet<String>) {
    for name in [
        "cookies.db",
        "Cookies-68.0.3440.106",
        "firefox_10_cookies.sqlite",
        "firefox_2_cookies.sqlite",
    ] {
        let data = fixture(name);
        assert_eq!(detect(name, &data), Some(Kind::Cookies), "{name}");
        let cookies = browser::read_cookies(&data, &[]).unwrap();
        assert_eq!(cookies.problems, Vec::<String>::new(), "{name}");
        for cookie in &cookies.rows {
            // plaso names the host without its leading dot.
            let host = cookie.host.trim_start_matches('.');
            for (desc, time) in [
                ("Creation Time", cookie.created),
                ("Last Access Time", cookie.last_accessed),
                ("Expiration Time", cookie.expires),
            ] {
                if let Some(time) = time {
                    got.insert(format!(
                        "{name}\tcookie\t{host}\t{}\t{}\t{desc}\t{}",
                        cookie.name,
                        cookie.path,
                        micros(time)
                    ));
                }
            }
        }
    }
}

/// Form history and extension activity.
fn chromium_rows(got: &mut BTreeSet<String>) {
    let data = fixture("Web Data");
    assert_eq!(detect("Web Data", &data), Some(Kind::Autofill));
    for entry in &browser::read_autofill(&data, &[]).unwrap().rows {
        for (desc, time) in [
            ("Creation Time", entry.created),
            ("Last Used Time", entry.last_used),
        ] {
            if let Some(time) = time {
                got.insert(format!(
                    "Web Data\tautofill\t{}\t{}\t{desc}\t{}",
                    entry.field,
                    entry.value,
                    micros(time)
                ));
            }
        }
    }
    let data = fixture("Extension Activity");
    assert_eq!(
        detect("Extension Activity", &data),
        Some(Kind::ExtensionActivity)
    );
    for action in &browser::read_extension_activity(&data, &[]).unwrap().rows {
        got.insert(format!(
            "Extension Activity\tactivity\t{}\t{}\t{}\t{}\t{}",
            action.rowid,
            action.extension_id,
            action.api_name.as_deref().unwrap_or_default(),
            action
                .action_type
                .map(|a| a.to_string())
                .unwrap_or_default(),
            micros(action.time.unwrap())
        ));
    }
}

/// Safari's downloads: their start and end, as plaso's
/// `safari_downloads` gives them.
fn safari_download_rows(got: &mut BTreeSet<String>) {
    let data = fixture("Downloads.plist");
    assert_eq!(
        detect("Downloads.plist", &data),
        Some(Kind::SafariDownloads)
    );
    let history = browser::read(&data, &[]).unwrap();
    assert_eq!(history.problems, Vec::<String>::new());
    for download in &history.downloads {
        for (desc, time) in [("Start Time", download.start), ("End Time", download.end)] {
            if let Some(time) = time {
                got.insert(format!(
                    "Downloads.plist\tdownload\t{}\t{}\t{}\t{}\t{desc}\t{}",
                    download.url,
                    download.target_path,
                    download.received_bytes.unwrap_or_default(),
                    download.total_bytes.unwrap_or_default(),
                    micros(time)
                ));
            }
        }
    }
}

/// Safari's visits.
fn safari_rows(got: &mut BTreeSet<String>) {
    for name in ["History.db", "History.plist"] {
        let history = browser::read(&fixture(name), &[]).unwrap();
        assert_eq!(history.problems, Vec::<String>::new(), "{name}");
        for visit in &history.visits {
            got.insert(format!(
                "{name}\tvisit\t{}\t{}\t{}\t{}",
                visit.url,
                visit.title,
                visit.visit_count.map(|c| c.to_string()).unwrap_or_default(),
                micros(visit.time.unwrap())
            ));
        }
    }
}

/// Extensions installed and sites' permissions, from `Preferences`.
fn preference_rows(got: &mut BTreeSet<String>) {
    let preferences = browser::read_preferences(&fixture("Preferences")).unwrap();
    for extension in &preferences.extensions {
        if let Some(installed) = extension.installed {
            got.insert(format!(
                "Preferences\textension\t{}\t{}\t{}\t{}",
                extension.id,
                extension.name.as_deref().unwrap_or_default(),
                extension.path.as_deref().unwrap_or_default(),
                micros(installed)
            ));
        }
    }
    for exception in &preferences.content_exceptions {
        if let Some(used) = exception.last_used {
            got.insert(format!(
                "Preferences\texception\t{}\t{}\t{}\t{}",
                exception.permission,
                exception.primary_url,
                exception.secondary_url,
                micros(used)
            ));
        }
    }
}

#[test]
fn as_plaso_reads_them() {
    let mut got = BTreeSet::new();
    cookie_rows(&mut got);
    chromium_rows(&mut got);
    safari_rows(&mut got);
    safari_download_rows(&mut got);
    preference_rows(&mut got);
    let expected: BTreeSet<&str> = include_str!("oracle/plaso-extras.tsv").lines().collect();
    let got_refs: BTreeSet<&str> = got.iter().map(String::as_str).collect();
    let missing: Vec<&&str> = expected.difference(&got_refs).take(10).collect();
    let extra: Vec<&&str> = got_refs.difference(&expected).take(10).collect();
    assert!(
        missing.is_empty() && extra.is_empty(),
        "missing {missing:#?}\nextra {extra:#?}"
    );
}
