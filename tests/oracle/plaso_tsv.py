"""Writes plaso's events (psort -a -o json_line) as sorted TSV lines.

One line per event: file (relative to the input directory), data type,
timestamp description, timestamp (microseconds; 0 for "Not a time"), then
the attributes FIELDS lists for its data type, in that order. A list is
written joined with " | ". Backslashes, tabs, carriage returns and line
feeds in text are written as \\\\, \\t, \\r and \\n. An attribute plaso
leaves out is empty. Events of a data type FIELDS doesn't list are left out.

    python3 -I plaso_tsv.py <out.jsonl>
"""

import json
import sys

INPUT_DIRECTORY = "/data/in/"

FIELDS = {
    "edge:resources:load_statistics": (
        "top_level_hostname",
        "resource_hostname",
        "resource_type",
    ),
    "safari:cookie:entry": ("url", "cookie_name", "path", "cookie_value", "flags"),
    "cookie:google:analytics:utma": (
        "url",
        "cookie_name",
        "domain_hash",
        "visitor_identifier",
        "sessions",
    ),
    "cookie:google:analytics:utmb": (
        "url",
        "cookie_name",
        "domain_hash",
        "pages_viewed",
    ),
    "cookie:google:analytics:utmt": ("url", "cookie_name"),
    "cookie:google:analytics:utmz": (
        "url",
        "cookie_name",
        "domain_hash",
        "sessions",
        "sources",
        "utmcsr",
        "utmccn",
        "utmcmd",
        "utmctr",
        "utmcct",
    ),
    "opera:history:entry": ("url", "title", "description", "popularity_index"),
    "opera:history:typed_entry": ("url", "entry_type", "entry_selection"),
    "java:download:idx": ("url", "idx_version", "ip_address"),
    "chrome:cache:entry": ("original_url", "payloads"),
    "firefox:cache:record": (
        "url",
        "version",
        "fetch_count",
        "frequency",
        "request_method",
        "response_code",
        "request_size",
        "info_size",
        "data_size",
        "location",
    ),
}


def text(value):
    """A value as text: a list joined, None empty."""
    if value is None:
        return ""
    if isinstance(value, list):
        return " | ".join(text(item) for item in value)
    return str(value)


def escape(value):
    """Text with backslash, tab, CR and LF escaped."""
    return (
        text(value)
        .replace("\\", "\\\\")
        .replace("\t", "\\t")
        .replace("\r", "\\r")
        .replace("\n", "\\n")
    )


def line(event):
    """The TSV line of one event."""
    file_name = event["display_name"].split(INPUT_DIRECTORY, 1)[1]
    values = [
        file_name,
        event["data_type"],
        event["timestamp_desc"],
        event["timestamp"],
    ]
    values.extend(event.get(field) for field in FIELDS[event["data_type"]])
    return "\t".join(escape(value) for value in values)


def main(path):
    with open(path, encoding="utf-8") as json_lines:
        events = [json.loads(text) for text in json_lines if text.strip()]
    lines = sorted(line(event) for event in events if event["data_type"] in FIELDS)
    for text_line in lines:
        print(text_line)


if __name__ == "__main__":
    main(sys.argv[1])
