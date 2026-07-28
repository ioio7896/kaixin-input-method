#!/usr/bin/env python3
"""Generate the bundled 20,000-word English IME lexicon.

The general words follow wordfreq 3.1.1's English frequency order.  A small
curated digital/technical vocabulary is retained because the IME is commonly
used for source code, documentation, settings, and search.  The generated TSV
is deterministic for a fixed wordfreq version.
"""

from __future__ import annotations

import argparse
from importlib.metadata import version
from pathlib import Path

from wordfreq import top_n_list


ROOT = Path(__file__).resolve().parents[1]
OUTPUT = ROOT / "lexicon" / "en" / "kaixin_common_english.txt"
WORDFREQ_VERSION = "3.1.1"
WORD_COUNT = 20_000
BLOCKED_WORDS = {"fuck", "fucking", "shit"}
CURATED_TERMS = (
    "application",
    "argument",
    "boolean",
    "browser",
    "candidate",
    "clipboard",
    "cloud",
    "code",
    "command",
    "commit",
    "complete",
    "completion",
    "compile",
    "compiler",
    "component",
    "computer",
    "configuration",
    "console",
    "context",
    "controller",
    "correction",
    "database",
    "debug",
    "default",
    "directory",
    "document",
    "email",
    "engine",
    "english",
    "example",
    "file",
    "folder",
    "format",
    "function",
    "github",
    "input",
    "interface",
    "internet",
    "java",
    "javascript",
    "keyboard",
    "layout",
    "linux",
    "macos",
    "microsoft",
    "module",
    "network",
    "node",
    "option",
    "package",
    "password",
    "plugin",
    "prefix",
    "privacy",
    "private",
    "process",
    "program",
    "project",
    "public",
    "python",
    "query",
    "release",
    "request",
    "response",
    "result",
    "return",
    "rust",
    "search",
    "server",
    "settings",
    "software",
    "source",
    "string",
    "struct",
    "system",
    "terminal",
    "token",
    "update",
    "user",
    "vector",
    "version",
    "website",
    "window",
    "windows",
)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=OUTPUT)
    return parser.parse_args()


def is_supported_word(word: str) -> bool:
    return (
        2 <= len(word) <= 24
        and word.isascii()
        and word.isalpha()
        and word == word.lower()
        and word not in BLOCKED_WORDS
    )


def build_words() -> list[str]:
    installed = version("wordfreq")
    if installed != WORDFREQ_VERSION:
        raise RuntimeError(
            f"wordfreq {WORDFREQ_VERSION} is required, found {installed}; "
            "install scripts/requirements-lexicon.txt"
        )

    ranked = []
    seen = set()
    for raw in top_n_list("en", 50_000):
        word = raw.lower()
        if not is_supported_word(word) or word in seen:
            continue
        seen.add(word)
        ranked.append(word)

    curated = list(dict.fromkeys(CURATED_TERMS))
    if len(curated) != len(CURATED_TERMS):
        raise RuntimeError("CURATED_TERMS contains duplicates")
    invalid = [word for word in curated if not is_supported_word(word)]
    if invalid:
        raise RuntimeError(f"invalid curated English terms: {invalid}")

    curated_set = set(curated)
    general_count = WORD_COUNT - len(curated)
    general = [word for word in ranked if word not in curated_set][:general_count]
    if len(general) != general_count:
        raise RuntimeError("wordfreq did not provide enough supported English words")

    selected = set(general) | curated_set
    rank_by_word = {word: rank for rank, word in enumerate(ranked)}
    words = sorted(selected, key=lambda word: (rank_by_word.get(word, 1 << 30), word))
    if len(words) != WORD_COUNT:
        raise RuntimeError(f"expected {WORD_COUNT} unique words, got {len(words)}")
    return words


def main() -> None:
    args = parse_args()
    words = build_words()
    lines = [
        f"# 开心输入法 {WORD_COUNT} 词常用英文词库",
        "# 格式：单词<TAB>输入码<TAB>权重",
        "# Source: wordfreq 3.1.1 (Apache-2.0) + 开心输入法常用技术词筛选",
    ]
    for rank, word in enumerate(words, 1):
        # Keep every weight positive and strictly descending at larger sizes.
        weight = 1_000_001 - rank
        lines.append(f"{word}\t{word}\t{weight}")
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text("\n".join(lines) + "\n", encoding="utf-8", newline="\n")
    print(f"wrote {len(words)} words to {args.output}")


if __name__ == "__main__":
    main()
