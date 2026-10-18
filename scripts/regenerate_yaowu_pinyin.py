#!/usr/bin/env python3
"""Regenerate the explicit pinyin column of the curated medicine lexicon.

The phrase and weight columns are editorial data.  This script deliberately
rewrites only the derived reading column, so a pasted or reordered source list
cannot shift readings onto neighbouring entries.
"""

from __future__ import annotations

import argparse
from pathlib import Path
from polyphone_corrections import corrected_reading, load_corrections


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
