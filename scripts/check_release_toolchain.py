"""Fail closed if the active compiler differs from the pinned release toolchain."""

import json
from pathlib import Path
import subprocess


def check(repo: Path) -> None:
    expected = json.loads((repo / "release-toolchain.json").read_text(encoding="utf-8"))
    for tool, version in expected.items():
        actual = subprocess.check_output([tool, "--version"], text=True, timeout=15).split()[1]
        if actual != version:
            raise RuntimeError(
                f"Release requires {tool} {version}, found {actual}; "
                f"install/select Rust {expected['rustc']} before building"
            )
    workflow = (repo / ".github/workflows/public-repository.yml").read_text(encoding="utf-8")
    if f"dtolnay/rust-toolchain@{expected['rustc']}" not in workflow:
        raise RuntimeError("CI toolchain differs from release-toolchain.json")


if __name__ == "__main__":
    check(Path(__file__).resolve().parents[1])
