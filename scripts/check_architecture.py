"""Reject crate source paths that bypass the shared/core/application dependency boundary."""

from pathlib import Path
import re
import tomllib

ROOT = Path(__file__).resolve().parents[1]


def check() -> None:
    for name in ("kaixin-common", "kaixin-core"):
        directory = (ROOT / "crates" / name).resolve()
        manifest = tomllib.loads((directory / "Cargo.toml").read_text(encoding="utf-8"))
        paths = [manifest.get("lib", {}).get("path", "src/lib.rs")]
        build = manifest["package"].get("build", "build.rs")
        if build is not False and (directory / build).exists():
            paths.append(build)
        for relative in paths:
            target = (directory / relative).resolve()
            if not target.is_relative_to(directory) or not target.is_file():
                raise RuntimeError(f"{name} must own its source: {relative}")
        dependencies = manifest.get("dependencies", {})
        forbidden = {"pinyin-ime"} | ({"kaixin-core"} if name == "kaixin-common" else set())
        if forbidden.intersection(dependencies):
            raise RuntimeError(f"Invalid dependency direction in {name}")
        for source in directory.rglob("*.rs"):
            for relative in re.findall(
                r'#\[path\s*=\s*"([^"]+)"\]', source.read_text(encoding="utf-8")
            ):
                if not (source.parent / relative).resolve().is_relative_to(directory):
                    raise RuntimeError(f"Cross-crate source include in {source.relative_to(ROOT)}")
    print("Crate source boundaries checked")


if __name__ == "__main__":
    check()
