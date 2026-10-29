"""Read-only structural audit of every three-to-five-character lexicon row.

Risk flags select entries for review; they never prove that a word is invalid.
In particular, a short spelling being part of a longer word must not reject
real terms, names, regional vocabulary, quantities or complete expressions.
"""
from __future__ import annotations

import argparse
from collections import Counter, defaultdict
import hashlib
import json
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[1]
TERMINAL_FUNCTION = re.compile(r"(?:的|地|了|之|而|但|与|及|或|且|从|向|对|由|以|于|在|为|将|均|被|把|让|该|其|若|虽|某)$")
INITIAL_CLASSIFIER = re.compile(r"^[个名份件条张届艘位支种场次].{2,4}$")
NARRATIVE = re.compile(r"(?:笑道|说道|问道|答道|叹道|喝道|骂道|叫道|心想|便道|娇声道|冷声道)$")
DEPENDENT = re.compile(r"^(?:的|地|了|中所|之所|之于)|(?:来讲|来说|而言|起见|由此|之故|所为|之理|不得|无所)$")
COMPARATIVE = re.compile(r"(?:越.{1,2}越|愈.{1,2}愈)$")


def data_rows(path: Path):
    for number, line in enumerate(path.read_text(encoding="utf-8-sig").splitlines(), 1):
        if line.strip() and not line.lstrip().startswith(("#", ";")):
            yield number, line


def risk_flags(phrase: str, completions: set[str]) -> list[str]:
    result = []
    if completions:
        result.append("boundary_of_longer_entry")
    for name, pattern in (
        ("function_word_at_end", TERMINAL_FUNCTION),
        ("classifier_or_surname_at_start", INITIAL_CLASSIFIER),
        ("narrative_boundary", NARRATIVE),
        ("dependent_clause", DEPENDENT),
        ("comparative_frame", COMPARATIVE),
    ):
        if pattern.search(phrase):
            result.append(name)
    return result


def scan(root: Path) -> dict:
    rows = []
    sources = []
    spellings = set()
    total_records = 0
    for path in sorted((root / "lexicon").rglob("*.txt")):
        if "en" in path.relative_to(root / "lexicon").parts:
            continue
        file_rows = list(data_rows(path))
        sources.append({"file": path.relative_to(root).as_posix(), "records": len(file_rows),
                        "sha256": hashlib.sha256(path.read_bytes()).hexdigest()})
        total_records += len(file_rows)
        for number, line in file_rows:
            fields = line.split("\t")
            phrase = fields[0]
            spellings.add(phrase)
            if 3 <= len(phrase) <= 5:
                rows.append({"phrase": phrase, "length": len(phrase),
                             "file": path.relative_to(root).as_posix(), "line": number,
                             "row": line})
    # Frozen imports supply possible complete forms, never semantic approval.
    for path in sorted((root / "data_sources/lexicon_fragments").rglob("*.txt")):
        if "en" not in path.parts:
            spellings.update(line.split("\t")[0] for _, line in data_rows(path))
    short_words = {row["phrase"] for row in rows}
    completions = defaultdict(set)
    for spelling in spellings:
        for removed in (1, 2):
            if 3 <= len(spelling) - removed <= 5:
                for short in (spelling[:-removed], spelling[removed:]):
                    if short in short_words:
                        completions[short].add(spelling)
    findings = {word: {"flags": risk_flags(word, completions[word]),
                       "possible_complete_forms": sorted(completions[word])[:5]}
                for word in sorted(short_words)}
    for row in rows:
        row.update(findings[row["phrase"]])
        row["review_status"] = "needs_review" if row["flags"] else "no_structural_flag"
    return {"records": rows, "sources": sources, "summary": {
        "source_files": len(sources), "all_records_scanned": total_records,
        "short_records_scanned": len(rows), "short_unique_spellings": len(short_words),
        "records_by_length": dict(sorted(Counter(row["length"] for row in rows).items())),
        "unique_by_length": dict(sorted(Counter(map(len, short_words)).items())),
        "flagged_unique_spellings": sum(bool(value["flags"]) for value in findings.values()),
        "note": "Full structural scan, not a semantic certificate. All flags require exact-entry review; absence of a flag does not certify naturalness. No input files are modified.",
    }}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=ROOT)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    result = scan(args.root.resolve())
    args.output.mkdir(parents=True, exist_ok=True)
    (args.output / "summary.json").write_text(json.dumps(result["summary"], ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    (args.output / "source-manifest.json").write_text(json.dumps(result["sources"], ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    with (args.output / "records.jsonl").open("w", encoding="utf-8", newline="\n") as stream:
        for row in result["records"]:
            stream.write(json.dumps(row, ensure_ascii=False) + "\n")
    print(json.dumps(result["summary"], ensure_ascii=False))


if __name__ == "__main__":
    main()
