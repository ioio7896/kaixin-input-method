#!/usr/bin/env python3
"""Build an independent four-character popularity benchmark from corpus counts.

The candidate lexicon is used only to recover a phrase's established reading when
available. Popularity and membership come from pinyin-ime/data/corpus.txt; unseen
phrases fall back to the single-character pronunciation table so missing lexicon
coverage remains measurable.
"""

from __future__ import annotations

import argparse
from collections import Counter
from pathlib import Path


def is_four_cjk(text: str) -> bool:
    return len(text) == 4 and all("\u3400" <= char <= "\u9fff" for char in text)


def iter_dictionary_rows(path: Path):
    with path.open("r", encoding="utf-8-sig", errors="ignore") as stream:
        for raw in stream:
            line = raw.strip()
            if not line or line.startswith("#"):
                continue
            fields = line.split("\t")
            if len(fields) < 2:
                continue
            phrase = fields[0].strip()
            reading = fields[1].strip().lower().split()
            if phrase and reading:
                yield phrase, reading


def load_readings(repo: Path) -> tuple[dict[str, list[str]], dict[str, str]]:
    phrase_readings: dict[str, list[str]] = {}
    char_readings: dict[str, str] = {}
    for path in (repo / "lexicon").rglob("*.txt"):
        for phrase, reading in iter_dictionary_rows(path):
            if len(phrase) == 1 and len(reading) == 1:
                char_readings.setdefault(phrase, reading[0])
            elif is_four_cjk(phrase) and len(reading) == 4:
                phrase_readings.setdefault(phrase, reading)
    return phrase_readings, char_readings


def main() -> int:
    repo = Path(__file__).resolve().parents[1]
    parser = argparse.ArgumentParser()
    parser.add_argument("--limit", type=int, default=1000)
    parser.add_argument(
        "--output",
        type=Path,
        default=repo / "tests" / "popular_four_char_cases.tsv",
    )
    args = parser.parse_args()

    counts: Counter[str] = Counter()
    corpus = repo / "pinyin-ime" / "data" / "corpus.txt"
    with corpus.open("r", encoding="utf-8", errors="ignore") as stream:
        for raw in stream:
            phrase = raw.strip()
            if is_four_cjk(phrase):
                counts[phrase] += 1

    phrase_readings, char_readings = load_readings(repo)
    rows: list[tuple[str, list[str], int, str]] = []
    for phrase, count in counts.most_common():
        reading = phrase_readings.get(phrase)
        source = "corpus_phrase_reading"
        if reading is None:
            reading = [char_readings.get(char, "") for char in phrase]
            source = "corpus_char_fallback"
        if len(reading) != 4 or any(not syllable for syllable in reading):
            continue
        rows.append((phrase, reading, count, source))
        if len(rows) >= max(args.limit, 1):
            break

    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.output.open("w", encoding="utf-8", newline="\n") as stream:
        stream.write("# category\tphrase\tpinyin\tcorpus_count\n")
        for phrase, reading, count, source in rows:
            stream.write(f"{source}\t{phrase}\t{' '.join(reading)}\t{count}\n")
    print(f"wrote {len(rows)} cases to {args.output}")
    return 0 if len(rows) >= args.limit else 1


if __name__ == "__main__":
    raise SystemExit(main())
