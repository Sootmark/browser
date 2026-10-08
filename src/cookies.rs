//! Cookies: Chromium's `Cookies` (table `cookies`; times in `WebKit`
//! microseconds, `secure`/`httponly` renamed `is_secure`/`is_httponly` in
//! Chrome 66) and Firefox's `cookies.sqlite` (table `moz_cookies`;
//! creation and last access in Unix microseconds, expiry in seconds). The
//! value is read as stored: Chromium encrypts it (`encrypted_value`) since
//! Chrome 80, and that isn't decrypted.

use common::time::Ts;
use sqlite::Database;

use crate::table::{self, Named};

/// Which browser a cookie is from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CookieStore {
    /// Chromium (Chrome, Edge, Brave, Opera, Vivaldi).
    Chromium,
    /// Firefox.
    Firefox,
}

/// A cookie.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cookie {
    /// Which browser.
    pub store: CookieStore,
    /// The row id.
    pub rowid: i64,
    /// The host it's for (`.example.com`).
    pub host: String,
    /// Its name.
    pub name: String,
    /// Its value, as stored (empty when Chromium encrypted it).
    pub value: String,
    /// The path it's for.
    pub path: String,
    /// When it was set.
    pub created: Option<Ts>,
    /// When it was last sent.
    pub last_accessed: Option<Ts>,
    /// When it expires; `None` for a session cookie.
    pub expires: Option<Ts>,
    /// Sent over HTTPS only.
    pub secure: bool,
    /// Hidden from scripts.
    pub http_only: bool,
    /// Kept across sessions (Chromium).
    pub persistent: Option<bool>,
}

/// Every cookie of a cookie database.
pub(crate) fn read(db: &Database<'_>, problems: &mut Vec<String>) -> Vec<Cookie> {
    if db.table("cookies").is_some() {
        table::read(db, "cookies", problems, chromium)
    } else {
        table::read(db, "moz_cookies", problems, firefox)
    }
}

fn chromium(row: &Named<'_>) -> Cookie {
    let flag = |new: &str, old: &str| row.flag(new).or_else(|| row.flag(old));
    let webkit = |column: &str| {
        row.integer(column)
            .filter(|&t| t != 0)
            .map(Ts::from_webkit_micros)
    };
    let has_expires = row.flag("has_expires").unwrap_or(true);
    Cookie {
        store: CookieStore::Chromium,
        rowid: row.rowid,
        host: row.text("host_key").unwrap_or_default(),
        name: row.text("name").unwrap_or_default(),
        value: row.text("value").unwrap_or_default(),
        path: row.text("path").unwrap_or_default(),
        created: webkit("creation_utc"),
        last_accessed: webkit("last_access_utc"),
        expires: webkit("expires_utc").filter(|_| has_expires),
        secure: flag("is_secure", "secure").unwrap_or(false),
        http_only: flag("is_httponly", "httponly").unwrap_or(false),
        persistent: flag("is_persistent", "persistent"),
    }
}

fn firefox(row: &Named<'_>) -> Cookie {
    let micros = |column: &str| {
        row.integer(column)
            .filter(|&t| t != 0)
            .map(Ts::from_unix_micros)
    };
    Cookie {
        store: CookieStore::Firefox,
        rowid: row.rowid,
        host: row.text("host").unwrap_or_default(),
        name: row.text("name").unwrap_or_default(),
        value: row.text("value").unwrap_or_default(),
        path: row.text("path").unwrap_or_default(),
        created: micros("creationTime"),
        last_accessed: micros("lastAccessed"),
        expires: row
            .integer("expiry")
            .filter(|&t| t != 0)
            .map(Ts::from_unix_seconds),
        secure: row.flag("isSecure").unwrap_or(false),
        http_only: row.flag("isHttpOnly").unwrap_or(false),
        persistent: None,
    }
}
