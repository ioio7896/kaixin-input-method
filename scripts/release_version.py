"""One timestamp command updates all app manifests/locks/docs; native/Inno read VERSION."""
from __future__ import annotations

import argparse
from datetime import datetime, timedelta, timezone
from pathlib import Path
import re
import tomllib

ROOT = Path(__file__).resolve().parents[1]
SHANGHAI = timezone(timedelta(hours=8))


def timestamp_version(now: datetime) -> str:
    local = now.astimezone(SHANGHAI)
    # Keep every numeric component valid for SemVer and Windows version resources.
    return f"{local.year}.{int(local.strftime('%m%d'))}.{int(local.strftime('%H%M'))}"


def validate_sources(repo: Path) -> list[str]:
    version = (repo / "VERSION").read_text(encoding="utf-8").strip()
    errors = []
    if not re.fullmatch(r"(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)", version):
        errors.append(f"Invalid numeric SemVer: {version}")
    for relative in ("pinyin-ime/Cargo.toml", "crates/kaixin-core/Cargo.toml"):
        manifest = tomllib.loads((repo / relative).read_text(encoding="utf-8"))
        if manifest["package"]["version"] != version:
            errors.append(f"{relative} does not match VERSION={version}")
    for relative in ("Cargo.lock", "pinyin-ime/Cargo.lock"):
        if not (repo / relative).exists():
            continue
        lock = tomllib.loads((repo / relative).read_text(encoding="utf-8"))
        for package in lock["package"]:
            if package["name"] in ("pinyin-ime", "kaixin-core") and package["version"] != version:
                errors.append(f"{relative}: {package['name']} does not match VERSION={version}")
    if f"当前版本为 **{version}**" not in (repo / "README.md").read_text(encoding="utf-8"):
        errors.append("README.md does not match VERSION")
    return errors


def update_version(repo: Path, version: str) -> None:
    updates = {repo / "VERSION": version + "\n"}
    for relative in ("pinyin-ime/Cargo.toml", "crates/kaixin-core/Cargo.toml"):
        path = repo / relative
        text, count = re.subn(r'(?m)^version = "[^"]+"', f'version = "{version}"',
                              path.read_text(encoding="utf-8"), count=1)
        if count != 1:
            raise RuntimeError(f"Missing package version: {path}")
        updates[path] = text
    for relative in ("Cargo.lock", "pinyin-ime/Cargo.lock"):
        path = repo / relative
        if path.exists():
            updates[path] = re.sub(r'(name = "(?:pinyin-ime|kaixin-core)"\nversion = ")[^"]+',
                                   lambda match: match[1] + version, path.read_text(encoding="utf-8"))
    readme = repo / "README.md"
    updates[readme], count = re.subn(r"当前版本为 \*\*[^*]+\*\*", f"当前版本为 **{version}**",
                                    readme.read_text(encoding="utf-8"))
    if count != 1:
        raise RuntimeError("README version marker missing")
    for path, text in updates.items():
        path.write_text(text, encoding="utf-8", newline="\n")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    if not args.check:
        update_version(ROOT, timestamp_version(datetime.now(SHANGHAI)))
    errors = validate_sources(ROOT)
    if errors:
        raise SystemExit("\n".join(errors))
    print((ROOT / "VERSION").read_text().strip())


if __name__ == "__main__":
    main()
