#!/usr/bin/env python3
"""Expand the short high-frequency overlay lexicons deterministically.

Existing hand-curated rows keep their score. Remaining rows are selected from
the frequency-sorted bundled lexicons and mapped into a conservative overlay
band so they improve first-page recall without outranking explicit rules.
"""

from __future__ import annotations

from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
ZH = ROOT / "lexicon" / "zh"
TARGETS = (
    (
        ZH / "life_short_hot_2char.txt",
        (ZH / "life_common_2char_125000.txt",),
        600,
        69,
        2,
        8_200,
        8_800,
        "生活场景二字高频精选（600 条）",
    ),
    (
        ZH / "life_hot_3char_curated.txt",
        (ROOT / "data_sources" / "lexicon_fragments" / "zh-ext" / "chat_common_phrases.txt",),
        400,
        24,
        3,
        8_400,
        9_000,
        "日常口语三字高频精选（400 条）",
    ),
)


def rows(path: Path) -> list[tuple[str, str, int]]:
    result: list[tuple[str, str, int]] = []
    for line_no, line in enumerate(path.read_text(encoding="utf-8-sig").splitlines(), 1):
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        parts = line.split("\t")
        if len(parts) != 3:
            raise ValueError(f"{path}:{line_no}: expected three tab-separated fields")
        try:
            score = int(parts[2])
        except ValueError as exc:
            raise ValueError(f"{path}:{line_no}: invalid score") from exc
        result.append((parts[0].strip(), parts[1].strip(), score))
    return result


def mapped_score(index: int, count: int, low: int, high: int) -> int:
    if count <= 1:
        return high
    return high - round(index * (high - low) / (count - 1))


def build_one(
    output: Path,
    sources: tuple[Path, ...],
    target: int,
    curated_count: int,
    fill_length: int,
    low: int,
    high: int,
    title: str,
) -> None:
    curated = rows(output)[:curated_count]
    selected: list[tuple[str, str, int, bool]] = []
    seen: set[str] = set()
    for phrase, reading, score in curated:
        if phrase not in seen:
            selected.append((phrase, reading, score, True))
            seen.add(phrase)

    candidates: list[tuple[str, str, int]] = []
    for source in sources:
        candidates.extend(rows(source))
    candidates.sort(key=lambda row: (-row[2], row[0], row[1]))
    needed = target - len(selected)
    additions = [
        (p, r, s) for p, r, s in candidates
        if p not in seen and len(p) == fill_length
    ][:needed]
    if len(additions) != needed:
        raise RuntimeError(f"{output}: only found {len(additions)} of {needed} required rows")
    for index, (phrase, reading, _score) in enumerate(additions):
        selected.append((phrase, reading, mapped_score(index, needed, low, high), False))
        seen.add(phrase)

    body = "\n".join(f"{phrase}\t{reading}\t{score}" for phrase, reading, score, _ in selected)
    output.write_text(
        f"# 开心输入法：{title}\n"
        "# 人工精选行保留原权重，其余按完整词库频率补齐；由 scripts/build_short_hot_lexicons.py 生成。\n"
        "# 格式：词语<TAB>全拼<TAB>领域热度\n"
        f"{body}\n",
        encoding="utf-8",
        newline="\n",
    )
    print(f"generated: {output.relative_to(ROOT)} rows={len(selected)} curated={len(curated)}")


def main() -> None:
    for output, sources, target, curated_count, fill_length, low, high, title in TARGETS:
        build_one(output, sources, target, curated_count, fill_length, low, high, title)


if __name__ == "__main__":
    main()
