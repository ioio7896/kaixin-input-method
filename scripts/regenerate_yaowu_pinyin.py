#!/usr/bin/env python3
"""Regenerate the explicit pinyin column of the curated medicine lexicon.

The phrase and weight columns are editorial data.  This script deliberately
rewrites only the derived reading column, so a pasted or reordered source list
cannot shift readings onto neighbouring entries.
"""

from __future__ import annotations

import argparse
from pathlib import Path
import re

from pypinyin import Style, lazy_pinyin


PINYIN_SYLLABLE = re.compile(r"^[a-z]+$")


def load_corrections(path: Path) -> dict[str, tuple[str, ...]]:
    corrections: dict[str, tuple[str, ...]] = {}
    for line_no, raw in enumerate(path.read_text(encoding="utf-8-sig").splitlines(), 1):
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        fields = [field.strip() for field in line.split("\t")]
        if len(fields) not in (2, 3):
            raise ValueError(f"{path}:{line_no}: expected phrase, pinyin[, scope]")
        phrase, code = fields[:2]
        reading = tuple(code.lower().split())
        if len(reading) != len(phrase) or any(
            not PINYIN_SYLLABLE.fullmatch(item) for item in reading
        ):
            raise ValueError(f"{path}:{line_no}: invalid correction")
        corrections[phrase] = reading
    return corrections


def corrected_reading(phrase: str, corrections: dict[str, tuple[str, ...]]) -> tuple[str, ...]:
    base = tuple(lazy_pinyin(phrase, style=Style.NORMAL, errors="default"))
    ordered = sorted(corrections.items(), key=lambda item: len(item[0]), reverse=True)
    reading: list[str] = []
    offset = 0
    while offset < len(phrase):
        match = next(
            ((term, value) for term, value in ordered if phrase.startswith(term, offset)),
            None,
        )
        if match is None:
            reading.append(base[offset].lower())
            offset += 1
        else:
            term, value = match
            reading.extend(value)
            offset += len(term)
    return tuple(reading)


def main() -> int:
    repo = Path(__file__).resolve().parents[1]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--input",
        type=Path,
        default=repo / "data_sources/lexicon_fragments/zh-ext/yaowu.txt",
    )
    parser.add_argument(
        "--corrections",
        type=Path,
        default=repo / "data_sources/kaixin/polyphone_corrections.tsv",
    )
    args = parser.parse_args()

    corrections = load_corrections(args.corrections)
    rendered: list[str] = []
    changed = 0
    seen_phrases: set[str] = set()
    duplicates = 0
    for line_no, raw in enumerate(args.input.read_text(encoding="utf-8-sig").splitlines(), 1):
        if not raw or raw.lstrip().startswith("#"):
            rendered.append(raw)
            continue
        fields = raw.split("\t")
        if len(fields) != 3:
            raise ValueError(f"{args.input}:{line_no}: expected phrase, pinyin, weight")
        phrase, old_code, weight = (field.strip() for field in fields)
        if phrase in seen_phrases:
            duplicates += 1
            continue
        seen_phrases.add(phrase)
        code = " ".join(corrected_reading(phrase, corrections))
        changed += code != old_code
        rendered.append(f"{phrase}\t{code}\t{weight}")
    args.input.write_text("\n".join(rendered) + "\n", encoding="utf-8", newline="\n")
    print(
        f"rewrote {args.input}: {changed} reading(s) updated, "
        f"{duplicates} duplicate row(s) removed"
    )
    from merge_zh_ext_lexicons import merge_lexicons

    merge_lexicons()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
