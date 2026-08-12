#!/usr/bin/env python3
"""Generate the 8,105-entry common single-Chinese-character lexicon.

Characters follow wordfreq 3.1.1's Chinese frequency order and are restricted
to characters supported by the bundled Rust pinyin table.  The generated file
uses the native two-column format so the engine can index every supported
reading of a polyphonic character instead of pinning it to one pronunciation.
"""

from __future__ import annotations

import argparse
from importlib.metadata import version
from pathlib import Path

from wordfreq import top_n_list


ROOT = Path(__file__).resolve().parents[1]
OUTPUT = ROOT / "lexicon" / "zh" / "single_char_common_8105.txt"
SUPPORTED_CHARS = ROOT / "pinyin-ime" / "data" / "pinyin_supported_chars.txt"
WORDFREQ_VERSION = "3.1.1"
CHARACTER_COUNT = 8_105
SOURCE_WORD_LIMIT = 500_000


def is_han_character(value: str) -> bool:
    if len(value) != 1:
        return False
    codepoint = ord(value)
    return (
        0x3400 <= codepoint <= 0x4DBF
        or 0x4E00 <= codepoint <= 0x9FFF
        or 0xF900 <= codepoint <= 0xFAFF
        or 0x20000 <= codepoint <= 0x2EBEF
        or 0x30000 <= codepoint <= 0x323AF
    )


def load_supported_characters(path: Path) -> set[str]:
    return {
        line.strip()
        for line in path.read_text(encoding="utf-8-sig").splitlines()
        if len(line.strip()) == 1
    }


def ranked_characters(supported: set[str]) -> list[str]:
    selected: list[str] = []
    seen: set[str] = set()
    for value in top_n_list("zh", SOURCE_WORD_LIMIT):
        if value in seen or value not in supported or not is_han_character(value):
            continue
        seen.add(value)
        selected.append(value)
        if len(selected) == CHARACTER_COUNT:
            return selected
    raise RuntimeError(
        f"wordfreq yielded only {len(selected)} supported Han characters; "
        f"need {CHARACTER_COUNT}"
    )


def rank_weight(index: int) -> int:
    """Map frequency rank deterministically onto the native 1..10000 scale."""

    if not 0 <= index < CHARACTER_COUNT:
        raise ValueError(f"rank index outside generated lexicon: {index}")
    return 10_000 - (index * 9_999 // (CHARACTER_COUNT - 1))


def render(characters: list[str]) -> str:
    lines = [
        "# 开心输入法 8105 个常用单字词库",
        "# 频率顺序：wordfreq 3.1.1 中文词频；仅保留拼音引擎支持的汉字。",
        "# 权重：按词频名次线性映射到 10000..1，严格降序。",
        "# 格式：汉字<TAB>权重；拼音由引擎自动生成，以保留多音字全部读音。",
    ]
    lines.extend(f"{character}\t{rank_weight(index)}" for index, character in enumerate(characters))
    return "\n".join(lines) + "\n"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=OUTPUT)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    installed = version("wordfreq")
    if installed != WORDFREQ_VERSION:
        raise RuntimeError(
            f"wordfreq {WORDFREQ_VERSION} is required for deterministic output; "
            f"found {installed}"
        )
    supported = load_supported_characters(SUPPORTED_CHARS)
    characters = ranked_characters(supported)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(render(characters), encoding="utf-8", newline="\n")
    print(
        f"generated {len(characters)} single-character entries: {args.output} "
        f"({characters[0]}..{characters[-1]})"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
