#!/bin/sh
# Recreates the *.visits files: every visit of plaso's test files as the
# sqlite3 shell reads them, in visit row order, one per line:
#
#   id|url|time (Unix microseconds)|transition|from_visit|visit_count|hidden
#
# Chromium times are WebKit microseconds (since 1601), shifted to 1970 here;
# the transition is its 32 bits, unsigned.
#
#   sudo apt-get install sqlite3
#   sh tests/oracle/gen.sh
set -eu

here=$(cd "$(dirname "$0")" && pwd)
fixtures="$here/../fixtures/plaso"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

chromium="SELECT v.id, IFNULL(u.url, ''), v.visit_time - 11644473600000000,
  v.transition & 4294967295, IFNULL(v.from_visit, 0), IFNULL(u.visit_count, 0),
  IFNULL(u.hidden, 0)
  FROM visits v LEFT JOIN urls u ON u.id = v.url ORDER BY v.id;"
firefox="SELECT v.id, IFNULL(p.url, ''), IFNULL(v.visit_date, 0), v.visit_type,
  IFNULL(v.from_visit, 0), IFNULL(p.visit_count, 0), IFNULL(p.hidden, 0)
  FROM moz_historyvisits v LEFT JOIN moz_places p ON p.id = v.place_id
  ORDER BY v.id;"

for name in History History-59.0.3071.86; do
    sqlite3 -readonly "$fixtures/$name" "$chromium" > "$here/$name.visits"
done
for name in places.sqlite firefox_25_places.sqlite places118.sqlite; do
    file="$fixtures/$name"
    if [ -f "$file.gz" ]; then
        gzip -dc "$file.gz" > "$work/$name"
        file="$work/$name"
    fi
    sqlite3 -readonly "$file" "$firefox" > "$here/$name.visits"
done
