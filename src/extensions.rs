//! Chromium's extensions: what they did (`Extension Activity`, Chrome's
//! activity log: table `activitylog_compressed`, its strings and URLs in
//! `string_ids` and `url_ids`, joined here as the `activitylog_uncompressed`
//! view does) and which were installed (`Preferences`, JSON:
//! `extensions.settings`, each extension's name, version, folder,
//! installation time and origin), with the sites given permissions
//! (`profile.content_settings.exceptions`): malicious extensions steal
//! sessions and passwords.

use std::collections::HashMap;

use common::json::{self, Json};
use common::time::Ts;
use sqlite::Database;

use crate::table;
use crate::Error;

/// Something an extension did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionActivity {
    /// The row id (the view's `activity_id`).
    pub rowid: i64,
    /// The extension.
    pub extension_id: String,
    /// When.
    pub time: Option<Ts>,
    /// What kind of action (0 API call, 1 API event, 2 blocked call, 3
    /// content script, 4 DOM access, 5 DOM event, 6 web request, …).
    pub action_type: Option<i64>,
    /// The API (`tabs.executeScript`, `browserAction.onClicked`).
    pub api_name: Option<String>,
    /// Its arguments, as JSON.
    pub args: Option<String>,
    /// The page it acted on.
    pub page_url: Option<String>,
    /// That page's title.
    pub page_title: Option<String>,
    /// A URL among its arguments.
    pub arg_url: Option<String>,
    /// Anything else recorded.
    pub other: Option<String>,
    /// How many times, merged.
    pub count: Option<i64>,
}

/// Every action of an `Extension Activity` database.
pub(crate) fn activity(db: &Database<'_>, problems: &mut Vec<String>) -> Vec<ExtensionActivity> {
    let ids = |table: &str, problems: &mut Vec<String>| -> HashMap<i64, String> {
        table::read(db, table, problems, |row| {
            (
                row.integer("id").unwrap_or(row.rowid),
                row.text("value").unwrap_or_default(),
            )
        })
        .into_iter()
        .collect()
    };
    let strings = ids("string_ids", problems);
    let urls = ids("url_ids", problems);
    table::read(db, "activitylog_compressed", problems, |row| {
        let string = |column: &str| row.integer(column).and_then(|id| strings.get(&id).cloned());
        let url = |column: &str| row.integer(column).and_then(|id| urls.get(&id).cloned());
        ExtensionActivity {
            rowid: row.rowid,
            extension_id: string("extension_id_x").unwrap_or_default(),
            time: row
                .integer("time")
                .filter(|&t| t != 0)
                .map(Ts::from_webkit_micros),
            action_type: row.integer("action_type"),
            api_name: string("api_name_x"),
            args: string("args_x"),
            page_url: url("page_url_x"),
            page_title: string("page_title_x"),
            arg_url: url("arg_url_x"),
            other: string("other_x"),
            count: row.integer("count"),
        }
    })
}

/// An installed extension, from `Preferences`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledExtension {
    /// Its identifier (32 letters).
    pub id: String,
    /// Its name, from its manifest.
    pub name: Option<String>,
    /// Its version, from its manifest.
    pub version: Option<String>,
    /// Its folder (relative to the profile's `Extensions`, or absolute
    /// for the built-in ones).
    pub path: Option<String>,
    /// When it was installed.
    pub installed: Option<Ts>,
    /// Installed from the Chrome Web Store.
    pub from_webstore: Option<bool>,
    /// Installed by default with the browser.
    pub by_default: Option<bool>,
    /// Its state (1: enabled, 0: disabled).
    pub state: Option<i64>,
    /// Where it came from (1: internal, 2: external preferences, 4:
    /// unpacked, …).
    pub location: Option<i64>,
    /// The APIs it was granted.
    pub permissions: Vec<String>,
}

/// A site's permission (`geolocation`, `notifications`, `midi_sysex`, …).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentException {
    /// The permission.
    pub permission: String,
    /// The site.
    pub primary_url: String,
    /// The embedding site (`*` for any).
    pub secondary_url: String,
    /// When the site last used it, or the setting last changed.
    pub last_used: Option<Ts>,
    /// The setting (1 allow, 2 block, …).
    pub setting: Option<i64>,
}

/// What `Preferences` says about extensions and site permissions.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Preferences {
    /// The extensions installed.
    pub extensions: Vec<InstalledExtension>,
    /// When the extensions' autoupdater last checked.
    pub autoupdate_last_check: Option<Ts>,
    /// The sites given (or refused) permissions.
    pub content_exceptions: Vec<ContentException>,
}

/// Read a Chromium `Preferences` file.
///
/// # Errors
/// When it isn't JSON.
pub fn read_preferences(data: &[u8]) -> Result<Preferences, Error> {
    let text = String::from_utf8_lossy(data);
    let root =
        json::parse(text.trim_start_matches('\u{feff}')).map_err(|e| Error(e.to_string()))?;
    let extensions = root.get("extensions");
    let webkit_text = |value: Option<&Json>| {
        value
            .and_then(Json::as_str)
            .and_then(|t| t.parse::<i64>().ok())
            .filter(|&t| t != 0)
            .map(Ts::from_webkit_micros)
    };
    let settings = extensions
        .and_then(|e| e.get("settings"))
        .map(members)
        .unwrap_or_default();
    let extensions_installed = settings
        .iter()
        .map(|(id, settings)| {
            let manifest = settings.get("manifest");
            let text = |value: Option<&Json>| value.and_then(Json::as_str).map(str::to_owned);
            InstalledExtension {
                id: id.clone(),
                name: text(manifest.and_then(|m| m.get("name"))),
                version: text(manifest.and_then(|m| m.get("version"))),
                path: text(settings.get("path")),
                installed: webkit_text(settings.get("install_time")),
                from_webstore: settings.get("from_webstore").and_then(boolean),
                by_default: settings.get("was_installed_by_default").and_then(boolean),
                state: settings.get("state").and_then(Json::as_i64),
                location: settings.get("location").and_then(Json::as_i64),
                permissions: settings
                    .get("granted_permissions")
                    .and_then(|p| p.get("api"))
                    .and_then(Json::as_array)
                    .unwrap_or_default()
                    .iter()
                    .filter_map(|p| p.as_str().map(str::to_owned))
                    .collect(),
            }
        })
        .collect();
    let autoupdate_last_check = webkit_text(
        extensions
            .and_then(|e| e.get("autoupdate"))
            .and_then(|a| a.get("last_check")),
    );
    let exceptions = root
        .get("profile")
        .and_then(|p| p.get("content_settings"))
        .and_then(|c| c.get("exceptions"))
        .map(members)
        .unwrap_or_default();
    let mut content_exceptions = Vec::new();
    for (permission, sites) in exceptions {
        for (pattern, setting) in members(sites) {
            let (primary, secondary) = pattern.split_once(',').unwrap_or((pattern, "*"));
            let last_used = setting
                .get("last_used")
                .and_then(seconds)
                .or_else(|| webkit_text(setting.get("last_modified")));
            content_exceptions.push(ContentException {
                permission: permission.clone(),
                primary_url: primary.to_owned(),
                secondary_url: secondary.to_owned(),
                last_used,
                setting: setting.get("setting").and_then(Json::as_i64),
            });
        }
    }
    Ok(Preferences {
        extensions: extensions_installed,
        autoupdate_last_check,
        content_exceptions,
    })
}

/// An object's members; none for anything else.
fn members(value: &Json) -> &[(String, Json)] {
    match value {
        Json::Object(members) => members,
        _ => &[],
    }
}

fn boolean(value: &Json) -> Option<bool> {
    match value {
        Json::Bool(b) => Some(*b),
        _ => None,
    }
}

/// Unix seconds, with a fraction.
fn seconds(value: &Json) -> Option<Ts> {
    let seconds = match value {
        Json::Float(f) => *f,
        Json::Int(n) => *n as f64,
        Json::UInt(n) => *n as f64,
        _ => return None,
    };
    (seconds.is_finite() && seconds > 0.0)
        .then(|| Ts::from_unix_micros((seconds * 1e6).round() as i64))
}
