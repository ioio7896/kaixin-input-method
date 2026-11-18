"""Deterministic authored priorities, common-word promotion and safe Base folding.

Only exact phrase/reading pairs are changed. Optional categories and Core
overlays keep their ownership; no substring or lowest-frequency deletion.
"""

from __future__ import annotations

from collections import Counter
from pathlib import Path

COMMON_SOURCE = "data_sources/kaixin/reviewed_common_phrases.tsv"
COMMON_OUTPUT = "lexicon/core/kaixin_reviewed_common.txt"
HOT_SOURCE = "data_sources/kaixin/hot_terms_20261027.tsv"
HOT_OUTPUT = "lexicon/zh/hot_terms_20261027.txt"
FOLD_TARGETS = {
    "lexicon/zh/life_common_2char_125000.txt",
    "lexicon/zh/life_common_3char.txt",
    "lexicon/zh/life_common_4char.txt",
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
    hot = maintained_rows(root, HOT_SOURCE, 5)
    output = []
    owners = {}
    folded = []
    for phrase, reading, score, owner, basis in hot:
        if owner == "-":
            output.append((phrase, reading, score))
            continue
        if owner not in FOLD_TARGETS:
            raise ValueError(f"Unsafe hot-word fold destination: {owner}")
        owners.setdefault(owner, {})[(phrase, reading)] = score
        folded.append(
            {"phrase": phrase, "reading": reading, "score": score, "owner": owner, "basis": basis}
        )
    for owner, replacements in owners.items():
        path = root / owner
        result = []
        found = set()
        for line in path.read_text(encoding="utf-8-sig").splitlines():
            if not line.strip() or line.startswith("#"):
                result.append(line)
                continue
            fields = line.split("\t")
            key = (fields[0], fields[1])
            if key in replacements:
                fields[2] = str(replacements[key])
                found.add(key)
            result.append("\t".join(fields))
        for key in sorted(replacements.keys() - found):
            result.append(f"{key[0]}\t{key[1]}\t{replacements[key]}")
        path.write_text("\n".join(result) + "\n", encoding="utf-8", newline="\n")
    (root / HOT_OUTPUT).write_text(
        generated_text(
            f"用户选定热词 {len(hot)} 条；本文件 {len(output)} 条，普通基础库合并 {len(folded)} 条",
            HOT_SOURCE,
            output,
        ),
        encoding="utf-8",
        newline="\n",
    )
    return {
        "common_rows": len(common),
        "hot_source_rows": len(hot),
        "hot_output_rows": len(output),
        "folded": folded,
        "hot_score_distribution": dict(sorted(Counter(row[2] for row in hot).items())),
    }


def check_maintained_sources(root: Path) -> list[str]:
    errors = []
    common = maintained_rows(root, COMMON_SOURCE, 4)
    actual = [tuple(fields[:3]) for _, fields in data_rows(root / COMMON_OUTPUT)]
    if actual != [(p, r, str(s)) for p, r, s, _ in common]:
        errors.append("Reviewed common lexicon differs from maintained source")
    hot = maintained_rows(root, HOT_SOURCE, 5)
    output = [tuple(fields[:3]) for _, fields in data_rows(root / HOT_OUTPUT)]
    expected = [(p, r, str(s)) for p, r, s, owner, _ in hot if owner == "-"]
    if output != expected:
        errors.append("Hot lexicon differs from authored priorities/deduplication ownership")
    owner_rows = {
        owner: {(f[0], f[1]): int(f[2]) for _, f in data_rows(root / owner)}
        for owner in FOLD_TARGETS
    }
    for phrase, reading, score, owner, _ in hot:
        if owner != "-" and (
            owner not in owner_rows or owner_rows[owner].get((phrase, reading)) != score
        ):
            errors.append(f"Missing/stale folded hot priority: {phrase} {reading} in {owner}")
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
