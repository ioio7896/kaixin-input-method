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
    "data_sources/lexicon_fragments/zh-ext/china_prefecture_level_admin_333.txt": (
        "cf0c9a8322b253fa7b5218134238054ca947431bc43a47d3f2a526e17fb8ed67",
        333,
        3,
    ),
    "data_sources/lexicon_fragments/zh-ext/county_admin_short_names_2024.txt": (
        "e370c5f6d9c3d1465852a89c2e43095407491267a6f9afbc8d9288e873a7a3cd",
        2770,
        3,
    ),
    "data_sources/lexicon_fragments/zh-ext/hangzhou_metro_stations_262.txt": (
        "c59fd37d8889097f7dfa46f5a0ea8ed147edafa671083f5eac5500d338d8321b",
        262,
        3,
    ),
    "data_sources/lexicon_fragments/zh-ext/hangzhou_admin.txt": (
        "082e70434a94ed31820657c8a673d44b801f4e5c1a9aa90add5503cf105fe463",
        300,
        3,
    ),
    "data_sources/lexicon_fragments/zh-ext/hangzhou_business.txt": (
        "bbf6c97ba7a70c210302521a422da5f02f70fb4e8ceacc3a653f7c1bd8864e2d",
        85,
        3,
    ),
    "data_sources/lexicon_fragments/zh-ext/hangzhou_food_culture.txt": (
        "52fa56b9e6f6fcb4efe447fa13d2aa319353eb68a4bd6d7882aa35a2bd9691fe",
        75,
        3,
    ),
    "data_sources/lexicon_fragments/zh-ext/hangzhou_landmarks_life.txt": (
        "7b1758d0ccebaf53bcfe6bf3242b8e665edd0c005695f410779e0b0e4fd589c6",
        165,
        3,
    ),
    "data_sources/lexicon_fragments/zh-ext/hangzhou_local_culture.txt": (
        "9244048090e2759942537d9c99fd46cb3fece27d33db709f3deaa60a9ae574f4",
        65,
        3,
    ),
    "data_sources/lexicon_fragments/zh-ext/hangzhou_new_places.txt": (
        "c47fc6428fdc481d8389e5c0ce0657fb46fb782aad19c08fd22e07f8ad2b7e62",
        131,
        3,
    ),
    "data_sources/lexicon_fragments/zh-ext/hangzhou_public_services.txt": (
        "e25892eb12a1e83bc26e0e4e888e74258db28263a68a156edfc63df67a1b5dae",
        165,
        3,
    ),
    "data_sources/lexicon_fragments/zh-ext/hangzhou_transport.txt": (
        "5a6032ad99f69af1ebab90e075b5eac1bdf7c44b9a3a3031528ed0226ca8952a",
        145,
        3,
    ),
    "data_sources/kaixin/common_phrases.tsv": (
        "62dfdd47eaf3f9d18f216079f6ad7ba24f20ceac2d72f198d2761183429d71b3",
        55,
        3,
    ),
    "data_sources/kaixin/polyphone_corrections.tsv": (
        "89d7a621ef5befe38ef8ebcb84b956d663a0d80186ec8f51a97e9568307f2df3",
        145,
        2,
    ),
    "data_sources/kaixin/pronunciation_aliases.tsv": (
        "b813f6c836e1bc1590cb51db2009b24bc6694dbf20f6f3c1ce7871eb16e89fc8",
        27,
        4,
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
            elif relative.endswith("pronunciation_aliases.tsv"):
                try:
                    if int(fields[2]) <= 0:
                        raise ValueError
                except ValueError:
                    fail(f"{relative}: data row {number} has an invalid weight")
                    errors += 1
                if fields[3] not in {"primary", "alternate", "colloquial", "historical"}:
                    fail(f"{relative}: data row {number} has an invalid pronunciation label")
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
