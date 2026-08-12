#!/usr/bin/env python3
"""Freeze an independent popular three-character ranking benchmark.

Membership and popularity come directly from the ``wordfreq`` API, never from
the candidate lexicon or ``pinyin-ime/data/corpus.txt``.  Every benchmark row is
sampled beyond the generated 20,000-entry lexicon boundary so evaluation cannot
silently reward words copied from the dictionary under test.
"""

from __future__ import annotations

import argparse
from pathlib import Path
import re
from typing import Mapping, Sequence

from build_life_common_3char_20000 import (
    RankedPhrase,
    collect_candidates,
    rank_weight,
)
from pypinyin import Style, lazy_pinyin


DEFAULT_LIMIT = 1_000
DEFAULT_HELDOUT_START = 20_001
DEFAULT_HELDOUT_WINDOW = 10_000
PINYIN_SYLLABLE = re.compile(r"^[a-z]+$")


def load_polyphone_corrections(path: Path) -> dict[str, tuple[str, ...]]:
    corrections: dict[str, tuple[str, ...]] = {}
    with path.open("r", encoding="utf-8-sig") as stream:
        for line_no, raw in enumerate(stream, 1):
            line = raw.strip()
            if not line or line.startswith("#"):
                continue
            fields = line.split("\t")
            if len(fields) != 2:
                raise ValueError(f"{path}:{line_no}: expected phrase and pinyin")
            phrase = fields[0].strip()
            reading = tuple(fields[1].strip().lower().split())
            if not phrase or len(reading) != len(phrase):
                raise ValueError(
                    f"{path}:{line_no}: correction length does not match phrase"
                )
            if any(not PINYIN_SYLLABLE.fullmatch(item) for item in reading):
                raise ValueError(f"{path}:{line_no}: invalid correction pinyin")
            corrections[phrase] = reading
    return corrections


def corrected_reading(
    phrase: str,
    corrections: Mapping[str, Sequence[str]],
) -> tuple[str, ...]:
    """Apply longest matching phrase corrections over pypinyin's base reading."""

    base = tuple(
        item.lower() for item in lazy_pinyin(phrase, style=Style.NORMAL, errors="default")
    )
    ordered = sorted(corrections.items(), key=lambda item: len(item[0]), reverse=True)
    reading: list[str] = []
    offset = 0
    while offset < len(phrase):
        match = next(
            (
                (term, tuple(replacement))
                for term, replacement in ordered
                if phrase.startswith(term, offset)
            ),
            None,
        )
        if match is None:
            reading.append(base[offset])
            offset += 1
        else:
            term, replacement = match
            reading.extend(replacement)
            offset += len(term)
    return tuple(reading)


def evenly_spaced_slice(
    candidates: Sequence[RankedPhrase],
    *,
    start: int,
    window: int,
    count: int,
) -> list[RankedPhrase]:
    """Pick ``count`` deterministic, rank-spread rows from a one-based window."""

    if start < 1:
        raise ValueError("heldout start must be positive")
    if window < 1:
        raise ValueError("heldout window must be positive")
    if not 0 <= count <= window:
        raise ValueError("heldout count must fit inside heldout window")
    first = start - 1
    if first + window > len(candidates):
        raise ValueError("candidate list does not cover the heldout window")
    if count == 0:
        return []
    if count == 1:
        return [candidates[first]]
    indexes = [
        first + index * (window - 1) // (count - 1) for index in range(count)
    ]
    return [candidates[index] for index in indexes]


def build_rows(
    *,
    limit: int,
    heldout_start: int,
    heldout_window: int,
    corrections: Mapping[str, Sequence[str]],
    pool_size: int | None = None,
) -> list[tuple[str, str, tuple[str, ...], int]]:
    if limit < 1:
        raise ValueError("limit must be positive")
    if limit > heldout_window:
        raise ValueError("limit must fit inside heldout window")
    required = heldout_start - 1 + heldout_window
    candidates = collect_candidates(required, pool_size=pool_size)

    selected = evenly_spaced_slice(
        candidates,
        start=heldout_start,
        window=heldout_window,
        count=limit,
    )

    rows: list[tuple[str, str, tuple[str, ...], int]] = []
    for candidate in selected:
        reading = corrected_reading(candidate.phrase, corrections)
        if len(reading) != 3 or any(
            not PINYIN_SYLLABLE.fullmatch(syllable) for syllable in reading
        ):
            raise RuntimeError(
                f"invalid pinyin for {candidate.phrase!r}: {' '.join(reading)}"
            )
        rows.append(
            (
                "wordfreq_rank_heldout",
                candidate.phrase,
                reading,
                rank_weight(candidate.source_rank),
            )
        )
    return rows


def write_rows(
    output: Path,
    rows: Sequence[tuple[str, str, Sequence[str], int]],
    *,
    heldout_start: int,
    heldout_window: int,
) -> None:
    output.parent.mkdir(parents=True, exist_ok=True)
    with output.open("w", encoding="utf-8", newline="\n") as stream:
        stream.write("# category\tphrase\tpinyin\trank_weight\n")
        stream.write(
            "# membership/frequency: wordfreq global rank; "
            "pinyin: pypinyin plus project polyphone corrections\n"
        )
        stream.write(
            f"# heldout: filtered three-character ranks {heldout_start}.."
            f"{heldout_start + heldout_window - 1}, deterministically spaced; "
            "all rows are outside the generated 20,000-entry lexicon\n"
        )
        for category, phrase, reading, weight in rows:
            stream.write(
                f"{category}\t{phrase}\t{' '.join(reading)}\t{weight}\n"
            )


def main() -> int:
    repo = Path(__file__).resolve().parents[1]
    parser = argparse.ArgumentParser()
    parser.add_argument("--limit", type=int, default=DEFAULT_LIMIT)
    parser.add_argument(
        "--heldout-start",
        type=int,
        default=DEFAULT_HELDOUT_START,
        help="one-based filtered three-character rank at which held-out sampling starts",
    )
    parser.add_argument(
        "--heldout-window",
        type=int,
        default=DEFAULT_HELDOUT_WINDOW,
    )
    parser.add_argument(
        "--pool-size",
        type=int,
        help="wordfreq source pool size override",
    )
    parser.add_argument(
        "--corrections",
        type=Path,
        default=repo / "data_sources" / "kaixin" / "polyphone_corrections.tsv",
    )
    parser.add_argument(
        "--output",
        type=Path,
        default=repo / "tests" / "popular_three_char_cases.tsv",
    )
    args = parser.parse_args()
    corrections = load_polyphone_corrections(args.corrections)
    rows = build_rows(
        limit=args.limit,
        heldout_start=args.heldout_start,
        heldout_window=args.heldout_window,
        corrections=corrections,
        pool_size=args.pool_size,
    )
    write_rows(
        args.output,
        rows,
        heldout_start=args.heldout_start,
        heldout_window=args.heldout_window,
    )
    categories: dict[str, int] = {}
    for category, _, _, _ in rows:
        categories[category] = categories.get(category, 0) + 1
    summary = ", ".join(f"{key}={value}" for key, value in sorted(categories.items()))
    print(f"wrote {len(rows)} cases to {args.output} ({summary})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
