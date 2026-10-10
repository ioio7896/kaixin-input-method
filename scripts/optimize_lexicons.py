#!/usr/bin/env python3
"""Regenerate reviewed categories and losslessly consolidate three-char sources."""
from __future__ import annotations

from pathlib import Path
import argparse
import json

ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / "data_sources/lexicon_fragments/zh"
ZH = ROOT / "lexicon/zh"


def read_rows(path: Path) -> list[tuple[str, str, int]]:
    rows = []
    if not path.exists():
        return rows
    for number, line in enumerate(path.read_text(encoding="utf-8-sig").splitlines(), 1):
        if not line.strip() or line.lstrip().startswith(("#", ";")):
            continue
        fields = line.split("\t")
        if len(fields) != 3:
            raise ValueError(f"{path}:{number}: expected phrase, reading, weight")
        phrase, code, weight = fields
        rows.append((phrase, " ".join(code.lower().split()), int(weight)))
    return rows


def consolidate_three_char() -> int:
    SOURCE.mkdir(parents=True, exist_ok=True)
    inputs = (
        (ZH / "lfie-common-3char.txt", SOURCE / "life_common_3char_full.txt"),
        (ZH / "life_common_3char_20000.txt", SOURCE / "life_common_3char_hot.txt"),
    )
    for old, archive in inputs:
        if old.exists() and not archive.exists():
            archive.write_text(old.read_text(encoding="utf-8-sig"), encoding="utf-8")
    if any(not archive.exists() for _, archive in inputs):
        raise FileNotFoundError("Missing three-character source archive")
    # Keep the old Base three-character calibration distribution independently
    # of lexical membership. Removing a duplicate source must not rescale every
    # three-character candidate. This stores scores only, never duplicate words.
    reference_path = ROOT / "shared/lexicon_frequency_reference.json"
    if not reference_path.exists():
        values = [score for _, _, score in read_rows(inputs[1][1])]
        for path in ZH.glob("*.txt"):
            if path.name in {"life_common_3char.txt", "lfie-common-3char.txt", "life_common_3char_20000.txt", "life_hot_3char_curated.txt", "life_hot_4char_curated.txt", "single_char_common_8105.txt"}:
                continue
            values.extend(score for phrase, _, score in read_rows(path) if len(phrase) == 3)
        reference_path.write_text(json.dumps({"3": sorted(values)}, ensure_ascii=False) + "\n", encoding="utf-8")
    merged = {}
    for _, archive in inputs:
        rows = read_rows(archive)
        if any(weight > 10_000 for _, _, weight in rows):
            from normalize_lexicon_freq10k import score_by_rank
            weights = score_by_rank([weight for _, _, weight in rows], 1, 10_000)
            rows = [(phrase, code, weight) for (phrase, code, _), weight in zip(rows, weights)]
        for phrase, code, weight in rows:
            if len(phrase) != 3:
                raise ValueError(f"Expected three characters: {phrase}")
            merged.setdefault((phrase, code), weight)
    text = (
        f"# 开心输入法原生词库：{len(merged)} 条三字词读音记录\n"
        "# Source: data_sources/lexicon_fragments/zh/life_common_3char_full.txt + life_common_3char_hot.txt\n"
        "# 同词同读音只收录一次；重合项沿用完整来源权重，合法多音读法保留。\n"
        "# 格式：词语<TAB>全拼<TAB>万分制排序分数（1-10000）。\n"
    )
    text += "".join(f"{phrase}\t{code}\t{weight}\n" for (phrase, code), weight in merged.items())
    (ZH / "life_common_3char.txt").write_text(text, encoding="utf-8", newline="\n")
    return len(merged)


def retire_migrated_files() -> None:
    for relative in json.loads((ROOT / "shared/retired_lexicon_files.json").read_text(encoding="utf-8")):
        path = (ROOT / "lexicon" / relative).resolve()
        path.relative_to((ROOT / "lexicon").resolve())
        if path.is_file():
            path.unlink()


def main() -> None:
    argparse.ArgumentParser(description=__doc__).parse_args()
    from build_modern_it_lexicons import CATEGORIES, OUT, write
    from build_common_phrase_lexicons import build_communication_fragments
    from merge_zh_ext_lexicons import merge_lexicons
    for tag, (seeds, keywords) in CATEGORIES.items():
        write(OUT / f"{tag}.txt", tag, seeds, keywords, [])
    build_communication_fragments()
    print("three-character union:", consolidate_three_char())
    old_long = ZH / "life_common_phrases_5to8.txt"
    new_long = ZH / "life_common_phrases.txt"
    if old_long.exists() and not new_long.exists():
        new_long.write_text(old_long.read_text(encoding="utf-8-sig"), encoding="utf-8")
    for name, count in merge_lexicons().items():
        print(name, count)
    retire_migrated_files()


if __name__ == "__main__":
    main()
