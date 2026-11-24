"""Deterministic authored priorities, common-word promotion and safe Base folding.

Only exact phrase/reading pairs are changed. Optional categories and Core
overlays keep their ownership; no substring or lowest-frequency deletion.
"""

from __future__ import annotations

from collections import Counter
from pathlib import Path

COMMON_SOURCE = "data_sources/kaixin/reviewed_common_phrases.tsv"
COMMON_OUTPUT = "lexicon/core/kaixin_reviewed_common.txt"
MODERN_SOURCE = "data_sources/kaixin/modern_life.tsv"
MODERN_OUTPUT = "lexicon/zh/modern_life.txt"
SEASONAL_SOURCE = "data_sources/kaixin/seasonal_hot_terms.tsv"
SEASONAL_OUTPUT = "lexicon/zh/seasonal_hot_terms.txt"
THREE_CHAR_BASE = "lexicon/zh/life_common_3char.txt"
FOLD_TARGETS = {
    "lexicon/zh/life_common_2char_125000.txt",
    THREE_CHAR_BASE,
    "lexicon/zh/life_common_4char.txt",
}
REVIEWED_READING_SOURCES = {
    "lexicon/core/kaixin_polyphone.txt": "data_sources/kaixin/reviewed_core_polyphones.tsv",
    "lexicon/zh/kaixin_recall.txt": "data_sources/kaixin/reviewed_recall_readings.tsv",
}


def data_rows(path: Path):
    for number, line in enumerate(path.read_text(encoding="utf-8-sig").splitlines(), 1):
        if line.strip() and not line.startswith("#"):
            yield number, line.split("\t")


def maintained_rows(root: Path, relative: str, columns: int):
    from lexicon_quality import canonical_entry, phrase_rejection_reason

    result = []
    seen = set()
    for number, fields in data_rows(root / relative):
        if len(fields) != columns or any(not field.strip() for field in fields):
            raise ValueError(f"{relative}:{number}: expected {columns} nonempty fields")
        phrase, reading = canonical_entry(fields[0], fields[1])
        score = int(fields[2])
        if not 1 <= score <= 10000 or phrase_rejection_reason(phrase):
            raise ValueError(f"{relative}:{number}: invalid maintained phrase/score")
        if len(reading.split()) != len(phrase) or (phrase, reading) in seen:
            raise ValueError(f"{relative}:{number}: invalid reading or duplicate pair")
        seen.add((phrase, reading))
        result.append((phrase, reading, score, *fields[3:]))
    return result


def hot_sources(root: Path):
    return maintained_rows(root, MODERN_SOURCE, 5), maintained_rows(root, SEASONAL_SOURCE, 5)


def folded_hot_scores(rows):
    owners = {}
    for phrase, reading, score, owner, _ in rows:
        if owner == "-":
            continue
        if owner not in FOLD_TARGETS:
            raise ValueError(f"Unsafe hot-word fold destination: {owner}")
        owners.setdefault(owner, {})[(phrase, reading)] = score
    return owners


def generated_text(title: str, source: str, rows) -> str:
    return (
        f"# {title}\n# Source: {source}; scripts/lexicon_quality.py --apply\n"
        "# 词语<TAB>全拼<TAB>人工排序分数（1—10000）；不代表实测用户词频。\n"
        + "".join(f"{phrase}\t{reading}\t{score}\n" for phrase, reading, score in rows)
    )


def apply_maintained_sources(root: Path) -> dict:
    common = maintained_rows(root, COMMON_SOURCE, 4)
    (root / COMMON_OUTPUT).write_text(
        generated_text(
            "常开日常与办公完整词语；保留精确混拼索引",
            COMMON_SOURCE,
            [(p, r, s) for p, r, s, _ in common],
        ),
        encoding="utf-8",
        newline="\n",
    )
    modern, seasonal = hot_sources(root)
    modern_output = []
    seasonal_output = []
    owners = folded_hot_scores(modern + seasonal)
    folded = []
    for source_kind, rows_list, out_bucket in (
        ("modern", modern, modern_output),
        ("seasonal", seasonal, seasonal_output),
    ):
        for phrase, reading, score, owner, basis in rows_list:
            if owner == "-":
                out_bucket.append((phrase, reading, score))
                continue
            folded.append(
                {
                    "source": source_kind,
                    "phrase": phrase,
                    "reading": reading,
                    "score": score,
                    "owner": owner,
                    "basis": basis,
                }
            )
    for owner, replacements in owners.items():
        path = root / owner
        headers = []
        rows = []
        found = set()
        for line in path.read_text(encoding="utf-8-sig").splitlines():
            if not line.strip() or line.startswith("#"):
                headers.append(line)
                continue
            fields = line.split("\t")
            key = (fields[0], fields[1])
            if key in replacements:
                fields[2] = str(replacements[key])
                found.add(key)
            rows.append(fields)
        for key in sorted(replacements.keys() - found):
            rows.append([key[0], key[1], str(replacements[key])])
        if owner in (
            "lexicon/zh/life_common_2char_125000.txt",
            "lexicon/zh/life_common_4char.txt",
        ):
            rows.sort(key=lambda f: (-int(f[2]), f[0], f[1]))
        path.write_text(
            "\n".join(headers + ["\t".join(r) for r in rows]) + "\n",
            encoding="utf-8",
            newline="\n",
        )
    (root / MODERN_OUTPUT).write_text(
        generated_text(
            f"常青现代生活与科技高频词 {len(modern)} 条；本文件 {len(modern_output)} 条，普通基础库合并 {len(folded)} 条",
            MODERN_SOURCE,
            sorted(modern_output, key=lambda r: (-r[2], r[0], r[1])),
        ),
        encoding="utf-8",
        newline="\n",
    )
    (root / SEASONAL_OUTPUT).write_text(
        generated_text(
            f"阶段性网络热词 {len(seasonal_output)} 条（温和权重，按季度复审归档）",
            SEASONAL_SOURCE,
            sorted(seasonal_output, key=lambda r: (-r[2], r[0], r[1])),
        ),
        encoding="utf-8",
        newline="\n",
    )
    all_hot = modern + seasonal
    return {
        "common_rows": len(common),
        "modern_source_rows": len(modern),
        "modern_output_rows": len(modern_output),
        "seasonal_output_rows": len(seasonal_output),
        "folded": folded,
        "hot_score_distribution": dict(sorted(Counter(row[2] for row in all_hot).items())),
    }


def check_maintained_sources(root: Path) -> list[str]:
    errors = []
    common = maintained_rows(root, COMMON_SOURCE, 4)
    actual = [tuple(fields[:3]) for _, fields in data_rows(root / COMMON_OUTPUT)]
    if actual != [(p, r, str(s)) for p, r, s, _ in common]:
        errors.append("Reviewed common lexicon differs from maintained source")
    modern, seasonal = hot_sources(root)
    for label, src_rows, out_rel in (
        ("Modern life", modern, MODERN_OUTPUT),
        ("Seasonal hot", seasonal, SEASONAL_OUTPUT),
    ):
        output = [tuple(fields[:3]) for _, fields in data_rows(root / out_rel)]
        expected = [
            (p, r, str(s))
            for p, r, s in sorted(
                ((p, r, s) for p, r, s, owner, _ in src_rows if owner == "-"),
                key=lambda row: (-row[2], row[0], row[1]),
            )
        ]
        if output != expected:
            errors.append(f"{label} lexicon differs from maintained source")
    owner_rows = {
        owner: {(f[0], f[1]): int(f[2]) for _, f in data_rows(root / owner)}
        for owner in FOLD_TARGETS
    }
    for phrase, reading, score, owner, _ in modern + seasonal:
        if owner != "-" and (
            owner not in owner_rows or owner_rows[owner].get((phrase, reading)) != score
        ):
            errors.append(f"Missing/stale folded hot priority: {phrase} {reading} in {owner}")
    for output, source in REVIEWED_READING_SOURCES.items():
        actual = {(f[0], f[1]): int(f[2]) for _, f in data_rows(root / output)}
        for phrase, reading, score in maintained_rows(root, source, 3):
            if actual.get((phrase, reading)) != score:
                errors.append(f"Missing/stale reviewed reading: {phrase} {reading} in {output}")
    return errors


def write_inventory(root: Path, categories: list[dict]) -> None:
    lines = [
        "# 当前词库清单",
        "",
        "由 `python scripts/lexicon_quality.py --apply` 按当前运行时输出生成；冻结导入报告保留历史数据。",
        "",
        f"扩展分类：{len(categories)} 个；默认开启 {sum(c.get('default_enabled', True) for c in categories)} 个。",
        "",
        "| 文件 | 名称 | 分组 | 默认开启 | 条目数 | 分数范围 |",
        "|---|---|---|---|---:|---|",
    ]
    lines.extend(
        f"| {c['file']} | {c['title']} | {c['group']} | {'是' if c.get('default_enabled', True) else '否'} | {c['rows']} | {c['score_min']}—{c['score_max']} |"
        for c in categories
    )
    lines += [
        "",
        "分数为来源相关的候选排序优先级，不代表实测用户词频；分类开关不影响 core/zh 常开层。",
        "",
    ]
    (root / "docs/lexicon-inventory.md").write_text(
        "\n".join(lines), encoding="utf-8", newline="\n"
    )
