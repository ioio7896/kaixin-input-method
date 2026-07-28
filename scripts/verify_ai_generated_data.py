#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Verify the frozen AI-assisted project-data artifacts.

This check proves file identity and structural integrity.  It does not claim
that names of real-world entities remain current after the verification date.
"""

from __future__ import annotations

import hashlib
import sqlite3
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
TEXT_FILES = {
    "lexicon/base/china_prefecture_level_admin_333.txt": (
        "7c42ec07dff26f8c678f5a79e3409bd9dc59f0335a1375b0a071cb382244b982",
        333,
        3,
    ),
    "lexicon/base/county_admin_short_names_2024.txt": (
        "3a8b07c00ea23ccce40fc66be29f00bd6f6b3f68ab58641542e9dd44d5fb7ab6",
        2770,
        3,
    ),
    "lexicon/base/hangzhou_metro_stations_262.txt": (
        "d944295047ac351ef1b48a277fcd352e51a27ebfc4da5d747f184d80464cd919",
        262,
        3,
    ),
    "lexicon/ext/hangzhou_admin.txt": (
        "ddc8aebc95b90b616ca94df1e5bdebdcd50756f9ecf9d67aedfdccc187fe0251",
        300,
        3,
    ),
    "lexicon/ext/hangzhou_business.txt": (
        "98a841788c5faf9a37b78a60265faf6583928b3c645af080e6c9be4a768e87d1",
        85,
        3,
    ),
    "lexicon/ext/hangzhou_food_culture.txt": (
        "dc5b1133ec375331242383f0ab784deaaf23fb862b081b5d7543253ad29764ce",
        75,
        3,
    ),
    "lexicon/ext/hangzhou_landmarks_life.txt": (
        "97f6d2ecb7ac93cc27e15fde821adc56b30a4cb87005fd49b90272fd40a7cbf6",
        165,
        3,
    ),
    "lexicon/ext/hangzhou_local_culture.txt": (
        "208ee6393998fa371eccedea131b5fa168906c0fa8b768bd917f7880f808cc6f",
        65,
        3,
    ),
    "lexicon/ext/hangzhou_new_places.txt": (
        "bbfb433ae79d2c9b12a423e5b78108a413224f2e625a125347c311078c05a5d3",
        131,
        3,
    ),
    "lexicon/ext/hangzhou_public_services.txt": (
        "a6309880503dccbafaa5e135a004d6c6e70179b431f06e25e1174b9d18e39208",
        165,
        3,
    ),
    "lexicon/ext/hangzhou_transport.txt": (
        "0609c01c369e1ef9e6fcfa366a273d7e8d7ba8eca0791dffcdd34565d0d6dc39",
        145,
        3,
    ),
    "data_sources/kaixin/common_phrases.tsv": (
        "d4890760141edacfaa443d1ef6c250b75f6d7b9cfb73ae6914e473ead9d07328",
        46,
        3,
    ),
    "data_sources/kaixin/polyphone_corrections.tsv": (
        "b40830a7560d63feeaeeda79c2e5f15cdda35078100d918a543e7c2dff60be6d",
        145,
        2,
    ),
}
SQLITE_FILE = "pinyin-ime/data/s2t_chars.sqlite"
SQLITE_SHA256 = "f09b4aa90b1569a22c49d222e7930bf98406b81a1a12c581a6b67f9e3915cc3a"


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def fail(message: str) -> None:
    print(f"error: {message}", file=sys.stderr)


def main() -> int:
    errors = 0
    for relative, (expected_hash, expected_rows, columns) in TEXT_FILES.items():
        path = ROOT / relative
        if not path.is_file():
            fail(f"missing frozen data file: {relative}")
            errors += 1
            continue
        actual_hash = digest(path)
        if actual_hash != expected_hash:
            fail(f"SHA-256 mismatch for {relative}: {actual_hash}")
            errors += 1
        rows = [
            line
            for line in path.read_text(encoding="utf-8").splitlines()
            if line and not line.startswith("#")
        ]
        if len(rows) != expected_rows:
            fail(f"row-count mismatch for {relative}: {len(rows)}")
            errors += 1
        keys: set[tuple[str, str]] = set()
        for number, row in enumerate(rows, 1):
            fields = row.split("\t")
            if len(fields) != columns:
                fail(f"{relative}: data row {number} has {len(fields)} columns")
                errors += 1
                continue
            if any(not field.strip() for field in fields):
                fail(f"{relative}: data row {number} contains an empty field")
                errors += 1
            key = (fields[0], fields[1])
            if key in keys:
                fail(f"{relative}: duplicate term/pinyin pair at data row {number}")
                errors += 1
            keys.add(key)
            if columns == 3:
                try:
                    if int(fields[2]) <= 0:
                        raise ValueError
                except ValueError:
                    fail(f"{relative}: data row {number} has an invalid weight")
                    errors += 1

    database = ROOT / SQLITE_FILE
    if not database.is_file():
        fail(f"missing frozen data file: {SQLITE_FILE}")
        errors += 1
    else:
        actual_hash = digest(database)
        if actual_hash != SQLITE_SHA256:
            fail(f"SHA-256 mismatch for {SQLITE_FILE}: {actual_hash}")
            errors += 1
        connection = sqlite3.connect(f"file:{database}?mode=ro", uri=True)
        try:
            if connection.execute("PRAGMA integrity_check").fetchone() != ("ok",):
                fail(f"SQLite integrity check failed: {SQLITE_FILE}")
                errors += 1
            summary = connection.execute(
                """
                SELECT COUNT(*), COUNT(DISTINCT simplified), MIN(sort_order),
                       MAX(sort_order), COUNT(DISTINCT sort_order),
                       SUM(simplified = '' OR traditional = '')
                FROM s2t_chars
                """
            ).fetchone()
            if summary != (2714, 2714, 0, 2713, 2714, 0):
                fail(f"unexpected s2t_chars structure/content summary: {summary!r}")
                errors += 1
        finally:
            connection.close()

    if errors:
        fail(f"AI-assisted data verification failed with {errors} error(s)")
        return 1
    print(f"AI-assisted data verification passed: {len(TEXT_FILES) + 1} files")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
