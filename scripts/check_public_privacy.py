#!/usr/bin/env python3
"""Check public Git contents for private runtime files, secrets and home paths."""
from __future__ import annotations
import re
import subprocess
from pathlib import Path, PurePosixPath
ROOT = Path(__file__).resolve().parents[1]
SECRET_PATTERNS = (
    re.compile(rb"-----BEGIN [A-Z ]*PRIVATE KEY-----"),
    re.compile(rb"AKIA[0-9A-Z]{16}"),
    re.compile(rb"gh[pousr]_[A-Za-z0-9_]{20,}"),
    re.compile(rb"github_pat_[A-Za-z0-9_]{40,}"),
    re.compile(rb"sk-[A-Za-z0-9_-]{20,}"),
)
# Match both plain Windows paths and JSON-escaped paths. Report filenames only.
HOME_PATH = re.compile(rb"(?i)(?:[a-z]:[\\/]+Users[\\/]+(?!Public(?:[\\/]|$)|Default(?:[\\/]|$))[^\\/\s\"<>]+|/(?:Users|home)/[^/\s\"<>]+)")
WORKSPACE_PATH = re.compile(rb"(?i)G:[\\/]+code1[\\/]+")
PRIVATE_NAMES = ("user_dict.sqlite", "clipboard_store.sqlite", "ocr_history.sqlite", "engine_capability.")
PRIVATE_PARTS = {".git", "dist", "target", "__pycache__", ".venv-rapidocr", ".agents", ".codex", ".aws", ".ssh", ".gnupg", ".lexicon_cache"}
PRIVATE_CONFIGS = {"config.private.ini", ".git-credentials", ".netrc", "_netrc", ".pypirc", "id_rsa", "id_dsa", "id_ecdsa", "id_ed25519"}
def violations(relative: str, data: bytes) -> list[str]:
    path = PurePosixPath(relative.replace("\\", "/"))
    parts = tuple(part.lower() for part in path.parts)
    name = path.name.lower()
    reasons = []
    if any(part in PRIVATE_PARTS or part.startswith((".venv", "kaixin-diagnostics-")) for part in parts): reasons.append("private/generated directory")
    if path.name.lower().startswith(PRIVATE_NAMES): reasons.append("private runtime data")
    if name in PRIVATE_CONFIGS or name.endswith(".local.ini") or (".cargo" in parts and name in {"credentials", "credentials.toml"}): reasons.append("private configuration/credential file")
    if path.suffix.lower() in {".log", ".pfx", ".p12", ".pem", ".key", ".dmp", ".pdb"} or path.name.startswith('.env'): reasons.append("private/generated file")
    if any(pattern.search(data) for pattern in SECRET_PATTERNS): reasons.append("possible credential")
    if HOME_PATH.search(data) or WORKSPACE_PATH.search(data): reasons.append("personal absolute path")
    return reasons

def main() -> int:
    result = subprocess.run(["git", "ls-files", "--cached", "--others", "--exclude-standard", "-z"], cwd=ROOT, capture_output=True, check=True)
    errors = []
    paths = [p.decode('utf-8') for p in result.stdout.split(b'\0') if p]
    for name in paths:
        path = ROOT/name
        if path.is_file():
            reasons = violations(name, path.read_bytes())
            if reasons: errors.append((name, reasons))
    for name, reasons in errors: print(name+": "+", ".join(reasons))
    print(f"Public privacy check: {len(paths)} files, {len(errors)} violation(s)")
    return 1 if errors else 0
if __name__ == '__main__': raise SystemExit(main())
