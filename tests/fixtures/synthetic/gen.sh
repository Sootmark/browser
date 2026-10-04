#!/bin/sh
# Recreates the synthetic databases, written by the sqlite3 shell:
#
# - History: a Chromium History with the tables of recent Chrome (history
#   version 6x; modelled on Chromium's components/history/core/browser
#   sources, not copied from a profile): a typed visit, a link followed by
#   an HTTP redirect (its transition stored negative, as Chrome binds the
#   32-bit value), a reload, a visit whose page row is gone, a download
#   through two redirects (chain rows written out of order) and an
#   interrupted one.
# - wal/History, wal/History-wal: the same, plus a visit committed to the
#   log and not yet checkpointed: both files copied while the connection
#   holds them.
# - places.sqlite: a Firefox page with a download whose metadata is not
#   JSON, and a visit whose page row is gone.
#
# Synthetic data only: documentation domains (RFC 2606) and addresses
# (RFC 5737).
#
#   sudo apt-get install sqlite3
#   sh tests/fixtures/synthetic/gen.sh
set -eu

here=$(cd "$(dirname "$0")" && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
cd "$work"

# 2026-09-01 08:00:00 UTC: 1788249600 Unix seconds, in WebKit microseconds.
t0=13432723200000000

chromium_tables="
CREATE TABLE meta(key LONGVARCHAR NOT NULL UNIQUE PRIMARY KEY, value LONGVARCHAR);
CREATE TABLE urls(id INTEGER PRIMARY KEY AUTOINCREMENT,url LONGVARCHAR,title LONGVARCHAR,visit_count INTEGER DEFAULT 0 NOT NULL,typed_count INTEGER DEFAULT 0 NOT NULL,last_visit_time INTEGER NOT NULL,hidden INTEGER DEFAULT 0 NOT NULL);
CREATE TABLE visits(id INTEGER PRIMARY KEY AUTOINCREMENT,url INTEGER NOT NULL,visit_time INTEGER NOT NULL,from_visit INTEGER,external_referrer_url TEXT,transition INTEGER DEFAULT 0 NOT NULL,segment_id INTEGER,visit_duration INTEGER DEFAULT 0 NOT NULL,incremented_omnibox_typed_score BOOLEAN DEFAULT FALSE NOT NULL,opener_visit INTEGER,originator_cache_guid TEXT,originator_visit_id INTEGER,originator_from_visit INTEGER,originator_opener_visit INTEGER,is_known_to_sync BOOLEAN DEFAULT FALSE NOT NULL,consider_for_ntp_most_visited BOOLEAN DEFAULT FALSE NOT NULL,visited_link_id INTEGER DEFAULT 0 NOT NULL,app_id TEXT);
CREATE TABLE downloads (id INTEGER PRIMARY KEY,guid VARCHAR NOT NULL,current_path LONGVARCHAR NOT NULL,target_path LONGVARCHAR NOT NULL,start_time INTEGER NOT NULL,received_bytes INTEGER NOT NULL,total_bytes INTEGER NOT NULL,state INTEGER NOT NULL,danger_type INTEGER NOT NULL,interrupt_reason INTEGER NOT NULL,hash BLOB NOT NULL,end_time INTEGER NOT NULL,opened INTEGER NOT NULL,last_access_time INTEGER NOT NULL,transient INTEGER NOT NULL,referrer VARCHAR NOT NULL,site_url VARCHAR NOT NULL,embedder_download_data VARCHAR NOT NULL,tab_url VARCHAR NOT NULL,tab_referrer_url VARCHAR NOT NULL,http_method VARCHAR NOT NULL,by_ext_id VARCHAR NOT NULL,by_ext_name VARCHAR NOT NULL,by_web_app_id VARCHAR NOT NULL,etag VARCHAR NOT NULL,last_modified VARCHAR NOT NULL,mime_type VARCHAR(255) NOT NULL,original_mime_type VARCHAR(255) NOT NULL);
CREATE TABLE downloads_url_chains (id INTEGER NOT NULL,chain_index INTEGER NOT NULL,url LONGVARCHAR NOT NULL, PRIMARY KEY (id, chain_index));
INSERT INTO meta VALUES ('version', '69'), ('last_compatible_version', '16');"

# Transitions: 838860801 TYPED|FROM_ADDRESS_BAR|CHAIN_START|CHAIN_END;
# 268435456 LINK|CHAIN_START; -1610612736 LINK|CHAIN_END|SERVER_REDIRECT;
# 805306376 RELOAD|CHAIN_START|CHAIN_END.
chromium_rows="
INSERT INTO urls (id,url,title,visit_count,typed_count,last_visit_time,hidden) VALUES
 (1,'https://example.com/','Example Domain',2,1,$t0+60000000,0),
 (2,'http://example.org/login','',1,0,$t0+10000000,1),
 (3,'https://example.org/login','Sign in',1,0,$t0+10250000,0),
 (4,'https://www.example.net/report.pdf','',1,0,$t0+120000000,0);
INSERT INTO visits (id,url,visit_time,from_visit,transition,visit_duration) VALUES
 (1,1,$t0,0,838860801,5000000),
 (2,2,$t0+10000000,1,268435456,0),
 (3,3,$t0+10250000,2,-1610612736,42000000),
 (4,1,$t0+60000000,0,805306376,1500000),
 (5,99,$t0+70000000,0,805306368,0);
INSERT INTO downloads VALUES
 (1,'6f9619ff-8b86-4011-b42d-00c04fc964ff','/srv/downloads/report.pdf','/srv/downloads/report.pdf',
  $t0+100000000,48213,48213,1,0,0,X'',$t0+101500000,1,$t0+200000000,0,
  'https://example.com/','https://example.net/','','https://example.com/','','GET','','','','\"abc\"','',
  'application/pdf','application/pdf'),
 (2,'0b5a3f1e-2c4d-4e6f-8a9b-1c2d3e4f5a6b','/srv/downloads/setup.exe.crdownload','/srv/downloads/setup.exe',
  $t0+300000000,1024,90000,4,1,20,X'',0,0,0,0,
  '','','','','','GET','','','','','',
  'application/octet-stream','application/octet-stream');
INSERT INTO downloads_url_chains VALUES
 (1,2,'https://www.example.net/report.pdf'),
 (1,0,'http://example.net/r/1'),
 (2,0,'http://192.0.2.10/setup.exe'),
 (1,1,'https://example.net/r/2');"

sqlite3 History "$chromium_tables $chromium_rows"

mkdir wal
sqlite3 wal-src "PRAGMA journal_mode=WAL; $chromium_tables $chromium_rows" >/dev/null
sqlite3 wal-src <<SQL
PRAGMA wal_autocheckpoint=0;
PRAGMA wal_checkpoint(TRUNCATE);
INSERT INTO visits (id,url,visit_time,from_visit,transition) VALUES (6,4,$t0+120000000,0,805306368);
.shell cp wal-src wal/History && cp wal-src-wal wal/History-wal
SQL

# The tables of Firefox 118 that matter here (from plaso's places118.sqlite).
# 1788249600000000: 2026-09-01 08:00:00 UTC in microseconds.
sqlite3 places.sqlite "
CREATE TABLE moz_places (   id INTEGER PRIMARY KEY, url LONGVARCHAR, title LONGVARCHAR, rev_host LONGVARCHAR, visit_count INTEGER DEFAULT 0, hidden INTEGER DEFAULT 0 NOT NULL, typed INTEGER DEFAULT 0 NOT NULL, frecency INTEGER DEFAULT -1 NOT NULL, last_visit_date INTEGER , guid TEXT, foreign_count INTEGER DEFAULT 0 NOT NULL, url_hash INTEGER DEFAULT 0 NOT NULL , description TEXT, preview_image_url TEXT, site_name TEXT, origin_id INTEGER, recalc_frecency INTEGER NOT NULL DEFAULT 0, alt_frecency INTEGER, recalc_alt_frecency INTEGER NOT NULL DEFAULT 0);
CREATE TABLE moz_historyvisits (  id INTEGER PRIMARY KEY, from_visit INTEGER, place_id INTEGER, visit_date INTEGER, visit_type INTEGER, session INTEGER, source INTEGER DEFAULT 0 NOT NULL, triggeringPlaceId INTEGER);
CREATE TABLE moz_anno_attributes (  id INTEGER PRIMARY KEY, name VARCHAR(32) UNIQUE NOT NULL);
CREATE TABLE moz_annos (  id INTEGER PRIMARY KEY, place_id INTEGER NOT NULL, anno_attribute_id INTEGER, content LONGVARCHAR, flags INTEGER DEFAULT 0, expiration INTEGER DEFAULT 0, type INTEGER DEFAULT 0, dateAdded INTEGER DEFAULT 0, lastModified INTEGER DEFAULT 0);
INSERT INTO moz_places (id,url,title,visit_count,typed,frecency,last_visit_date) VALUES
 (1,'https://example.com/file.zip','file.zip',1,1,2000,1788249600000000);
INSERT INTO moz_historyvisits (id,from_visit,place_id,visit_date,visit_type) VALUES
 (1,0,1,1788249600000000,7),
 (2,1,42,1788249660000000,1);
INSERT INTO moz_anno_attributes VALUES (1,'downloads/destinationFileURI'),(2,'downloads/metaData');
INSERT INTO moz_annos (id,place_id,anno_attribute_id,content,dateAdded,lastModified) VALUES
 (1,1,1,'file:///srv/downloads/file.zip',1788249600500000,1788249600500000),
 (2,1,2,'{\"state\":1,\"endTime\":',1788249601000000,1788249601000000);"

cp History places.sqlite "$here/"
rm -rf "$here/wal"
cp -r wal "$here/"
