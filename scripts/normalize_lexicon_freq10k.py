#!/usr/bin/env python3
"""Rewrite bundled lexicon frequencies into a bounded 1..10000 score.

The bundled files mix corpus-derived frequencies, percentile-like weights and
curated priority bands.  This script deliberately keeps the lexical rows and
pinyin untouched, but gives each source a documented score band.  Within a
non-flat source, the old values determine the descending order; ties remain
ties.  Flat curated lists receive a fixed band midpoint instead of an
artificial order based on file position.
"""

from __future__ import annotations

import argparse
from pathlib import Path

MAX_SCORE = 10_000


def phrase_len(term: str) -> int:
    return sum(not ch.isspace() for ch in term)


def score_by_rank(values: list[int], low: int, high: int) -> list[int]:
    """Map descending unique values to an inclusive score interval."""
    if not values:
        return []
    low = max(1, min(MAX_SCORE, low))
    high = max(low, min(MAX_SCORE, high))
    unique = sorted(set(max(1, value) for value in values), reverse=True)
    if len(unique) == 1:
        midpoint = (low + high) // 2
        return [midpoint] * len(values)
    positions = {value: index for index, value in enumerate(unique)}
    last = len(unique) - 1
    return [
        high - round(positions[max(1, value)] * (high - low) / last)
        for value in values
    ]


def profile(path: Path, rows: list[tuple[str, list[str], int]]) -> tuple[int, int]:
    """Return the target score band for one source file."""
    rel = path.as_posix().lower()
    name = path.name.lower()

    # Explicit/core sources are intentional overrides.
    if "/core/" in rel:
        if name == "kaixin_explicit.txt":
            return 9_700, 10_000
        return 8_800, 10_000

    # English has an independent candidate universe but uses the same public
    # 1..10000 scale.  Its source is already ordered by frequency.
    if "/en/" in rel:
        return 1, 10_000

    # Curated Base sources: useful, but should not outrank every ordinary word.
    if "/base/" in rel:
        if name == "life_short_hot_2char.txt":
            return 8_200, 8_800
        return 1, 10_000

    # Large is a recall tail and must remain below the normal Base head.
    if "/large/" in rel:
        return 1, 7_000

    # Surnames and newly-added places are intentionally recall-only.  Their
    # low score is also kept low by the runtime's unscaled-source policy.
    if name == "people_names.txt":
        return 200, 500
    if name == "hangzhou_local.txt":
        return 100, 6_500

    # Hand-curated local/admin and modern product sources get a middle band;
    # their runtime percentile calibration still preserves within-file rank.
    if name == "geography_admin.txt":
        return 1_500, 6_500
    if name == "technology.txt":
        return 2_000, 8_800
    if name == "daily_communication.txt":
        return 7_000, 9_300

    # Auxiliary extension dictionaries are normalized independently; the
    # runtime maps their percentiles onto the Base distribution by phrase len.
    return 1, 8_500


def rewrite_file(path: Path) -> tuple[int, int, int]:
    lines = path.read_text(encoding="utf-8").splitlines()
    parsed: list[tuple[int, list[str], int]] = []
    for index, line in enumerate(lines):
        if not line.strip() or line.startswith("#"):
            continue
        fields = line.split("\t")
        if len(fields) < 3:
            continue
        try:
            raw = int(fields[2].strip())
        except ValueError:
            continue
        parsed.append((index, fields, max(1, raw)))

    if not parsed:
        return 0, 0, 0

    low, high = profile(path, [(f[0], f[1], f[2]) for f in parsed])
    old_values = [item[2] for item in parsed]
    new_values = score_by_rank(old_values, low, high)
    for (index, fields, _), score in zip(parsed, new_values):
        fields[2] = str(max(1, min(MAX_SCORE, score)))
        lines[index] = "\t".join(fields)

    # Keep source comments truthful after replacing corpus counts/weights.
    updated: list[str] = []
    for line in lines:
        if line.startswith("#") and (
            "词频为 10^Zipf" in line
            or "词频 = round(" in line
        ):
            line = "# 格式：词语<TAB>全拼<TAB>万分制排序分数；范围 1-10000，按来源与原频次排序归一化。"
        if (
            line.startswith("# 格式：")
            and "万分制" not in line
            and ("权重" in line or "词频" in line)
        ):
            line += "；第三列统一为 1-10000 万分制排序分数。"
        if line.startswith("#") and "统一采用低优先级词频 4000" in line:
            line = line.replace("词频 4000", "万分制分数 300")
        if line.startswith("#") and "低词频 50" in line:
            line = line.replace("低词频 50", "低万分制分数 175")
        updated.append(line)
    path.write_text("\n".join(updated) + "\n", encoding="utf-8", newline="\n")
    return len(parsed), min(new_values), max(new_values)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--root",
        type=Path,
        default=Path(__file__).resolve().parents[1] / "lexicon",
    )
    args = parser.parse_args()
    total = 0
    for path in sorted(args.root.rglob("*.txt")):
        if path.name.lower().startswith("readme"):
            continue
        count, low, high = rewrite_file(path)
        if count:
            total += count
            print(f"{path.relative_to(args.root)}\t{count}\t{low}\t{high}")
    print(f"TOTAL\t{total}")


if __name__ == "__main__":
    main()
