#!/usr/bin/env python3
from __future__ import annotations

import argparse
import sqlite3
from pathlib import Path


def read_rows(path: Path) -> list[tuple[str, str, str, str]]:
    rows: list[tuple[str, str, str, str]] = []
    seen: set[str] = set()
    for line_no, raw in enumerate(path.read_text(encoding="utf-8-sig").splitlines(), 1):
        if not raw.strip() or raw.startswith("#"):
            continue
        fields = raw.split("\t")
        if len(fields) != 4 or any(not field for field in fields):
            raise ValueError(f"{path}:{line_no}: expected four non-empty TSV fields")
        case_id, category, input_text, expected = fields
        if case_id in seen:
            raise ValueError(f"{path}:{line_no}: duplicate case_id {case_id!r}")
        seen.add(case_id)
        rows.append((case_id, category, input_text, expected))
    return rows


def main() -> int:
    repo = Path(__file__).resolve().parents[1]
    parser = argparse.ArgumentParser(description="Build input smoke cases from audited TSV")
    parser.add_argument("--source", type=Path, default=repo / "tests" / "input_cases.tsv")
    parser.add_argument("--output", type=Path, default=repo / "tests" / "input_cases.sqlite")
    args = parser.parse_args()

    rows = read_rows(args.source)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with sqlite3.connect(args.output) as connection:
        connection.executescript(
            """
            DROP TABLE IF EXISTS cases;
            DROP TABLE IF EXISTS case_metadata;
            CREATE TABLE cases (
                sort_order INTEGER NOT NULL,
                case_id TEXT PRIMARY KEY,
                category TEXT NOT NULL,
                input TEXT NOT NULL,
                expected TEXT NOT NULL
            );
            CREATE TABLE case_metadata (key TEXT PRIMARY KEY, value TEXT NOT NULL);
            """
        )
        connection.executemany(
            "INSERT INTO cases(sort_order, case_id, category, input, expected) VALUES (?, ?, ?, ?, ?)",
            ((index, *row) for index, row in enumerate(rows)),
        )
        connection.executemany(
            "INSERT INTO case_metadata(key, value) VALUES (?, ?)",
            [
                ("source", args.source.name),
                ("generator", "scripts/build_input_cases_sqlite.py"),
                ("schema", "cases(sort_order, case_id, category, input, expected)"),
            ],
        )
    print(f"wrote {len(rows)} audited input cases to {args.output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
