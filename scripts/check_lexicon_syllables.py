#!/usr/bin/env python3
from __future__ import annotations

import argparse
import re
import sys
import time
from pathlib import Path


PINYIN_TOKEN_RE = re.compile(r"[A-Za-z]+")
SKIP_LEXICON_GROUPS = {"en"}
MIXED_ENTITY_RE = re.compile(r"[A-Za-z0-9]")
CHINESE_CHAR_RE = re.compile(
    r"[\u3400-\u4dbf\u4e00-\u9fff\uf900-\ufaff"
    r"\U00020000-\U0002a6df\U0002a700-\U0002ebef]"
)


def resolve_repo_root() -> Path:
    return Path(__file__).resolve().parents[1]


def load_syllables(path: Path) -> set[str]:
    return {
        line.strip().lower()
        for line in path.read_text(encoding="utf-8-sig").splitlines()
        if line.strip()
    }


def iter_chinese_lexicon_files(lexicon_dir: Path):
    for path in sorted(lexicon_dir.rglob("*.txt"), key=lambda p: p.as_posix().lower()):
        rel_parts = set(path.relative_to(lexicon_dir).parts)
        if rel_parts & SKIP_LEXICON_GROUPS:
            continue
        yield path


def iter_lexicon_lines(path: Path):
    with path.open("r", encoding="utf-8-sig", errors="replace") as f:
        for line_no, line in enumerate(f, 1):
            stripped = line.strip()
            if not stripped or stripped.startswith("#"):
                continue
            yield line_no, line.rstrip("\n")


def _chinese_char_count(phrase: str) -> int:
    """Count Chinese characters in a phrase (excluding whitespace and ASCII)."""
    return len(CHINESE_CHAR_RE.findall(phrase))


def _is_mixed_entity(phrase: str) -> bool:
    """True when phrase contains ASCII letters/digits (e.g. ChatGPT, B站)."""
    return MIXED_ENTITY_RE.search(phrase) is not None


def scan_lexicons(lexicon_dir: Path, syllables: set[str] | None, *,
                  check_counts: bool = False, progress: bool = False):
    """Read/tokenize each row once for both checks; retain existing diagnostics."""
    unknown: dict[str, list[tuple[Path, int, str, str]]] = {}
    mismatches: list[tuple[Path, int, str, str, int, int]] = []
    files = list(iter_chinese_lexicon_files(lexicon_dir))
    started = time.monotonic()
    for index, path in enumerate(files, 1):
        relative = path.relative_to(lexicon_dir).as_posix()
        if progress:
            print(f"  Lexicon [{index}/{len(files)}]: {relative}", flush=True)
        last_progress = time.monotonic()
        for line_no, line in iter_lexicon_lines(path):
            if progress and line_no % 10000 == 0:
                now = time.monotonic()
                if now - last_progress >= 2:
                    print(f"    {relative}: {line_no:,} lines scanned", flush=True)
                    last_progress = now
            fields = [field.strip() for field in line.split("\t") if field.strip()]
            if len(fields) < 2:
                continue
            phrase, code = fields[0], fields[1]
            if _is_mixed_entity(phrase):
                continue
            tokens = PINYIN_TOKEN_RE.findall(code.lower())
            if not tokens:
                continue
            if syllables is not None:
                for token in tokens:
                    if token not in syllables:
                        unknown.setdefault(token, []).append((path, line_no, phrase, code))
            if check_counts:
                char_count = _chinese_char_count(phrase)
                if char_count and len(tokens) != char_count:
                    mismatches.append((path, line_no, phrase, code, char_count, len(tokens)))
    if progress:
        print(f"  Lexicon scan complete: {len(files)} file(s), "
              f"{time.monotonic() - started:.1f}s", flush=True)
    return unknown, mismatches, len(files)


def scan_unknown_syllables(lexicon_dir: Path, syllables: set[str]) -> dict[str, list[tuple[Path, int, str, str]]]:
    return scan_lexicons(lexicon_dir, syllables)[0]


def scan_syllable_count_mismatches(lexicon_dir: Path) -> list[tuple[Path, int, str, str, int, int]]:
    return scan_lexicons(lexicon_dir, None, check_counts=True)[1]


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Check that native Chinese lexicon pinyin codes use known syllables.",
    )
    parser.add_argument("--root", type=Path, default=None, help="repository root")
    parser.add_argument("--progress", action="store_true", help="show file and scan progress")
    parser.add_argument("--max-samples", type=int, default=12, help="samples to print")
    parser.add_argument(
        "--check-syllable-count",
        action="store_true",
        help="also verify pinyin syllable count matches Chinese character count",
    )
    parser.add_argument(
        "--strict-syllable-count",
        action="store_true",
        help="treat pinyin syllable-count mismatches as errors",
    )
    return parser.parse_args()


def main() -> int:
    for stream in (sys.stdout, sys.stderr):
        try:
            stream.reconfigure(encoding="utf-8", errors="replace")
        except (AttributeError, OSError, TypeError, ValueError):
            pass

    args = parse_args()
    root = (args.root or resolve_repo_root()).resolve()
    syllables_path = root / "pinyin-ime" / "data" / "syllables.txt"
    lexicon_dir = root / "lexicon"
    if not syllables_path.is_file():
        print(f"missing syllable table: {syllables_path}")
        return 2
    if not lexicon_dir.is_dir():
        print(f"missing lexicon directory: {lexicon_dir}")
        return 2

    exit_code = 0

    check_counts = args.check_syllable_count or args.strict_syllable_count
    unknown, mismatches, checked = scan_lexicons(
        lexicon_dir, load_syllables(syllables_path),
        check_counts=check_counts, progress=args.progress,
    )
    if unknown:
        total_refs = sum(len(items) for items in unknown.values())
        print(
            f"Lexicon syllable check failed: {len(unknown)} unknown syllable(s), "
            f"{total_refs} reference(s)"
        )
        for token, hits in sorted(unknown.items(), key=lambda kv: (-len(kv[1]), kv[0]))[
            : max(args.max_samples, 0)
        ]:
            path, line_no, phrase, code = hits[0]
            rel = path.relative_to(root).as_posix()
            print(f"  - {token}: {len(hits)} refs; first {rel}:{line_no} {phrase}\t{code}")
        exit_code = 1
    else:
        print(f"Lexicon syllable check passed: {checked} Chinese text file(s) scanned")

    if check_counts:
        if mismatches:
            print(
                f"Lexicon syllable count check: {len(mismatches)} mismatch(es) found"
            )
            for path, line_no, phrase, code, cc, sc in mismatches[
                : max(args.max_samples, 0)
            ]:
                rel = path.relative_to(root).as_posix()
                print(
                    f"  - {rel}:{line_no} '{phrase}' code='{code}' "
                    f"chars={cc} syllables={sc}"
                )
            if args.strict_syllable_count:
                exit_code = 1
            else:
                # Some historical entries intentionally keep more than one
                # reading. Release verification opts into strict mode.
                print("  (syllable count mismatches are advisory; review manually)")
        else:
            print("Lexicon syllable count check passed")

    return exit_code


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except KeyboardInterrupt:
        print("\nLexicon check cancelled.", file=sys.stderr, flush=True)
        raise SystemExit(130)
