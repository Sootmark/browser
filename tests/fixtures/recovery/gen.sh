#!/bin/sh
# Recreates the databases with deleted history, written by the sqlite3
# shell from synthetic SQL (documentation domains, RFC 2606):
#
# - chromium.db is not made here: it is sootmark-sqlite's recovery fixture
#   (tests/fixtures/recovery/chromium.db in github.com/Sootmark/sqlite,
#   made by its gen.sh with secure_delete off, MIT or Apache-2.0), with the
#   rows its generator deleted (tests/oracle/recovery/chromium.db/, copied
#   from the same repository): a Chromium-shaped History whose visits of
#   one time range were cleared, then the urls left without visits.
# - History, History-wal: a Chromium History in write-ahead log mode, as
#   Chrome keeps it, deleting with secure_delete on, as Chrome does. Its
#   rows are checkpointed into the file; then, in the log only: a site is
#   forgotten (its two pages and their visits), two visits of a page kept
#   are deleted (the page's visit count updated), a kept visit's duration
#   is updated, and a download is deleted with its URL chain. Both files
#   are copied while the connection holds them, before any checkpoint.
# - places.sqlite, places.sqlite-wal: the same for Firefox. Before the
#   checkpoint, with secure_delete off, two visits of a page kept are
#   deleted (their cells left in the page's freeblocks). After it, in the
#   log, with secure_delete on: a site is forgotten (its page and visits),
#   a visit of a page kept is deleted, a kept page's frecency is updated;
#   last, with secure_delete off, two visits are recorded and deleted in
#   one transaction (their cells left only in the page as the log has it
#   now). The file keeps the visits deleted before the checkpoint in its
#   freeblocks, and the forgotten site whole.
#
#   sudo apt-get install sqlite3
#   sh tests/fixtures/recovery/gen.sh
set -eu

here=$(cd "$(dirname "$0")" && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
cd "$work"

# 2026-09-01 08:00:00 UTC in WebKit microseconds (Chromium) and PRTime
# (Firefox).
webkit0=13432723200000000
prtime0=1788249600000000

# --- History, History-wal ----------------------------------------------------
# Transitions: 805306368 LINK|CHAIN_START|CHAIN_END; 838860801
# TYPED|FROM_ADDRESS_BAR|CHAIN_START|CHAIN_END.
sqlite3 History > /dev/null <<SQL
PRAGMA journal_mode = WAL;
PRAGMA wal_autocheckpoint = 0;
CREATE TABLE meta(key LONGVARCHAR NOT NULL UNIQUE PRIMARY KEY, value LONGVARCHAR);
CREATE TABLE urls(id INTEGER PRIMARY KEY AUTOINCREMENT,url LONGVARCHAR,title LONGVARCHAR,visit_count INTEGER DEFAULT 0 NOT NULL,typed_count INTEGER DEFAULT 0 NOT NULL,last_visit_time INTEGER NOT NULL,hidden INTEGER DEFAULT 0 NOT NULL);
CREATE TABLE visits(id INTEGER PRIMARY KEY AUTOINCREMENT,url INTEGER NOT NULL,visit_time INTEGER NOT NULL,from_visit INTEGER,external_referrer_url TEXT,transition INTEGER DEFAULT 0 NOT NULL,segment_id INTEGER,visit_duration INTEGER DEFAULT 0 NOT NULL,incremented_omnibox_typed_score BOOLEAN DEFAULT FALSE NOT NULL,opener_visit INTEGER,originator_cache_guid TEXT,originator_visit_id INTEGER,originator_from_visit INTEGER,originator_opener_visit INTEGER,is_known_to_sync BOOLEAN DEFAULT FALSE NOT NULL,consider_for_ntp_most_visited BOOLEAN DEFAULT FALSE NOT NULL,visited_link_id INTEGER DEFAULT 0 NOT NULL,app_id TEXT);
CREATE TABLE downloads (id INTEGER PRIMARY KEY,guid VARCHAR NOT NULL,current_path LONGVARCHAR NOT NULL,target_path LONGVARCHAR NOT NULL,start_time INTEGER NOT NULL,received_bytes INTEGER NOT NULL,total_bytes INTEGER NOT NULL,state INTEGER NOT NULL,danger_type INTEGER NOT NULL,interrupt_reason INTEGER NOT NULL,hash BLOB NOT NULL,end_time INTEGER NOT NULL,opened INTEGER NOT NULL,last_access_time INTEGER NOT NULL,transient INTEGER NOT NULL,referrer VARCHAR NOT NULL,site_url VARCHAR NOT NULL,embedder_download_data VARCHAR NOT NULL,tab_url VARCHAR NOT NULL,tab_referrer_url VARCHAR NOT NULL,http_method VARCHAR NOT NULL,by_ext_id VARCHAR NOT NULL,by_ext_name VARCHAR NOT NULL,by_web_app_id VARCHAR NOT NULL,etag VARCHAR NOT NULL,last_modified VARCHAR NOT NULL,mime_type VARCHAR(255) NOT NULL,original_mime_type VARCHAR(255) NOT NULL);
CREATE TABLE downloads_url_chains (id INTEGER NOT NULL,chain_index INTEGER NOT NULL,url LONGVARCHAR NOT NULL, PRIMARY KEY (id, chain_index));
INSERT INTO meta VALUES ('version', '69'), ('last_compatible_version', '16');
INSERT INTO urls (id,url,title,visit_count,typed_count,last_visit_time,hidden) VALUES
 (1,'https://example.com/','Example Domain',1,1,$webkit0,0),
 (2,'https://example.org/news','News',3,0,$webkit0+300000000,0),
 (3,'https://example.net/','Example Net',1,0,$webkit0+400000000,0),
 (4,'https://forgotten.example/','Forgotten home',1,1,$webkit0+500000000,0),
 (5,'https://forgotten.example/account','Account settings',2,0,$webkit0+700000000,0);
INSERT INTO visits (id,url,visit_time,from_visit,transition,visit_duration) VALUES
 (1,1,$webkit0,0,838860801,5000000),
 (2,2,$webkit0+100000000,1,805306368,0),
 (3,2,$webkit0+200000000,2,805306368,7000000),
 (4,2,$webkit0+300000000,3,805306368,8000000),
 (5,3,$webkit0+400000000,0,805306368,9000000),
 (6,4,$webkit0+500000000,0,838860801,10000000),
 (7,5,$webkit0+600000000,6,805306368,11000000),
 (8,5,$webkit0+700000000,7,805306368,12000000);
INSERT INTO downloads VALUES
 (1,'6f9619ff-8b86-4011-b42d-00c04fc964ff','/srv/downloads/kept.pdf','/srv/downloads/kept.pdf',
  $webkit0+150000000,1000,1000,1,0,0,X'',$webkit0+151000000,0,0,0,
  'https://example.org/news','','','','','GET','','','','','','application/pdf','application/pdf'),
 (2,'0b5a3f1e-2c4d-4e6f-8a9b-1c2d3e4f5a6b','/srv/downloads/tool.exe','/srv/downloads/tool.exe',
  $webkit0+650000000,90000,90000,1,0,0,X'',$webkit0+652500000,1,0,0,
  'https://forgotten.example/account','','','','','GET','','','','','','application/octet-stream','application/octet-stream');
INSERT INTO downloads_url_chains VALUES
 (1,0,'https://example.org/kept.pdf'),
 (2,0,'https://forgotten.example/get/tool'),
 (2,1,'https://cdn.forgotten.example/tool.exe');
PRAGMA wal_checkpoint(TRUNCATE);
PRAGMA secure_delete = ON;
UPDATE visits SET visit_duration = 6000000 WHERE id = 2;
DELETE FROM visits WHERE url IN (4, 5);
DELETE FROM urls WHERE id IN (4, 5);
DELETE FROM visits WHERE id IN (3, 4);
UPDATE urls SET visit_count = 1, last_visit_time = $webkit0+100000000 WHERE id = 2;
DELETE FROM downloads WHERE id = 2;
DELETE FROM downloads_url_chains WHERE id = 2;
.shell cp History "$here/History" && cp History-wal "$here/History-wal"
SQL

# --- places.sqlite, places.sqlite-wal -----------------------------------------
# Visit types: 1 LINK, 2 TYPED.
sqlite3 places.sqlite > /dev/null <<SQL
PRAGMA journal_mode = WAL;
PRAGMA wal_autocheckpoint = 0;
CREATE TABLE moz_places (   id INTEGER PRIMARY KEY, url LONGVARCHAR, title LONGVARCHAR, rev_host LONGVARCHAR, visit_count INTEGER DEFAULT 0, hidden INTEGER DEFAULT 0 NOT NULL, typed INTEGER DEFAULT 0 NOT NULL, frecency INTEGER DEFAULT -1 NOT NULL, last_visit_date INTEGER , guid TEXT, foreign_count INTEGER DEFAULT 0 NOT NULL, url_hash INTEGER DEFAULT 0 NOT NULL , description TEXT, preview_image_url TEXT, site_name TEXT, origin_id INTEGER, recalc_frecency INTEGER NOT NULL DEFAULT 0, alt_frecency INTEGER, recalc_alt_frecency INTEGER NOT NULL DEFAULT 0);
CREATE TABLE moz_historyvisits (  id INTEGER PRIMARY KEY, from_visit INTEGER, place_id INTEGER, visit_date INTEGER, visit_type INTEGER, session INTEGER, source INTEGER DEFAULT 0 NOT NULL, triggeringPlaceId INTEGER);
INSERT INTO moz_places (id,url,title,rev_host,visit_count,typed,frecency,last_visit_date,guid) VALUES
 (1,'https://example.com/','Example Domain','moc.elpmaxe.',2,1,2000,$prtime0+100000000,'aaaaaaaaaaaa'),
 (2,'https://example.org/news','News','gro.elpmaxe.',3,0,1500,$prtime0+500000000,'bbbbbbbbbbbb'),
 (3,'https://forgotten.example/','Forgotten home','elpmaxe.nettogrof.',2,1,900,$prtime0+300000000,'cccccccccccc');
INSERT INTO moz_historyvisits (id,from_visit,place_id,visit_date,visit_type,session) VALUES
 (1,0,1,$prtime0,2,0),
 (2,1,1,$prtime0+100000000,1,0),
 (3,0,3,$prtime0+200000000,2,0),
 (4,3,3,$prtime0+300000000,1,0),
 (5,0,2,$prtime0+400000000,1,0),
 (6,5,2,$prtime0+450000000,1,0),
 (7,6,2,$prtime0+500000000,1,0);
PRAGMA secure_delete = OFF;
DELETE FROM moz_historyvisits WHERE id IN (5, 6);
UPDATE moz_places SET visit_count = 1 WHERE id = 2;
PRAGMA wal_checkpoint(TRUNCATE);
PRAGMA secure_delete = ON;
DELETE FROM moz_historyvisits WHERE place_id = 3;
DELETE FROM moz_places WHERE id = 3;
DELETE FROM moz_historyvisits WHERE id = 2;
UPDATE moz_places SET visit_count = 1, last_visit_date = $prtime0, frecency = 1000 WHERE id = 1;
UPDATE moz_places SET frecency = 1400 WHERE id = 2;
PRAGMA secure_delete = OFF;
BEGIN;
INSERT INTO moz_historyvisits (id,from_visit,place_id,visit_date,visit_type,session) VALUES
 (8,7,2,$prtime0+600000000,1,0),
 (9,8,2,$prtime0+650000000,1,0);
DELETE FROM moz_historyvisits WHERE id IN (8, 9);
COMMIT;
.shell cp places.sqlite "$here/places.sqlite" && cp places.sqlite-wal "$here/places.sqlite-wal"
SQL
