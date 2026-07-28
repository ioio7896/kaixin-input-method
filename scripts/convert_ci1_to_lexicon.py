import re
from pathlib import Path
from typing import Iterable, Optional, Tuple
import unicodedata
import sys
 
ROOT = Path(__file__).resolve().parents[1]
SRC_DIR = ROOT / "ci1"
OUT_DIR = ROOT / "lexicon"
 
RE_PINYIN_LEFT = re.compile(r"^\s*'?[a-z]+(?:'[a-z]+)*\s+.+\S\s*$")
RE_INT = re.compile(r"\d+")
 
 
def try_decode(raw: bytes) -> str:
    # Order matters: try strict decodes, fall back to replacement at the very end.
    for enc in (
        "utf-8-sig",
        "utf-8",
        "gb18030",
        "gbk",
        "big5",
        "utf-16le",
        "utf-16",
    ):
        try:
            return raw.decode(enc)
        except UnicodeDecodeError:
            pass
    return raw.decode("utf-8", errors="replace")
 
 
def sanitize_filename(name: str) -> str:
    # Windows reserved characters: \ / : * ? " < > |
    bad = '<>:"/\\|?*'
    out = "".join("_" if ch in bad else ch for ch in name)
    out = unicodedata.normalize("NFC", out).strip()
    if not out:
        out = "lexicon"
    return out
 
 
def out_name_for(src_path: Path) -> str:
    rel = src_path.relative_to(SRC_DIR)
    # Flatten while preserving uniqueness.
    parts = [sanitize_filename(p) for p in rel.parts]
    return "__".join(parts)
 
 
def parse_entry_line(line: str) -> Optional[Tuple[str, int]]:
    line = line.strip("\ufeff").strip()
    if not line or line.startswith("#"):
        return None
 
    # Tabbed: "词\tfreq" or other tabbed formats; pick first field as phrase, last int as freq.
    if "\t" in line:
        fields = [f.strip() for f in line.split("\t") if f.strip()]
        if not fields:
            return None
        phrase = fields[0]
        if not phrase or (len(fields) == 1 and ":" in phrase):
            return None
        freq = 1
        for f in reversed(fields[1:]):
            m = RE_INT.search(f)
            if m:
                freq = int(m.group(0))
                break
        return phrase, freq
 
    # Space-separated pinyin + phrase: "'ai'ai'fu'mu 哀哀父母"
    if RE_PINYIN_LEFT.match(line) and " " in line:
        _, phrase = line.split(None, 1)
        phrase = phrase.strip()
        if not phrase:
            return None
        return phrase, 1
 
    # Fallback: treat as phrase-only.
    phrase = line.strip()
    if not phrase:
        return None
    return phrase, 1
 
 
def iter_entries(text: str) -> Iterable[Tuple[str, int]]:
    for line in text.splitlines():
        parsed = parse_entry_line(line)
        if parsed:
            yield parsed
 
 
def main() -> int:
    if not SRC_DIR.is_dir():
        print(f"source dir not found: {SRC_DIR}", file=sys.stderr)
        return 2
 
    OUT_DIR.mkdir(parents=True, exist_ok=True)
 
    sources = sorted(SRC_DIR.rglob("*.txt"))
    if not sources:
        print("no .txt sources found under ci1/", file=sys.stderr)
        return 2
 
    for src in sources:
        raw = src.read_bytes()
        text = try_decode(raw)
 
        # Dedup within file, keep max freq.
        best: dict[str, int] = {}
        for phrase, freq in iter_entries(text):
            # Normalize whitespace inside phrase.
            phrase = re.sub(r"\s+", " ", phrase).strip()
            if not phrase:
                continue
            prev = best.get(phrase)
            if prev is None or freq > prev:
                best[phrase] = freq
 
        out_name = out_name_for(src)
        out_path = OUT_DIR / out_name
 
        # Sort by freq desc, then phrase for stability.
        rows = sorted(best.items(), key=lambda kv: (-kv[1], kv[0]))
        out_text = "".join(f"{phrase}\t{freq}\n" for phrase, freq in rows)
        out_path.write_text(out_text, encoding="utf-8", newline="\n")
 
    print(f"wrote {len(sources)} files into {OUT_DIR}")
    return 0
 
 
if __name__ == "__main__":
    raise SystemExit(main())

