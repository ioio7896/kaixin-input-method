"""Shared source, Rust and native verification used by local tools, CI and packaging."""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]


def plan() -> dict:
    return json.loads((ROOT / "shared/verification.json").read_text(encoding="utf-8"))


def tool(name: str) -> str:
    bundled = ROOT / "dist/dev-tools/python"
    candidates = {
        "ruff": bundled / "bin/ruff.exe",
        "clang-format": bundled / "clang_format/data/bin/clang-format.exe",
    }
    candidate = candidates.get(name)
    if candidate and candidate.is_file():
        return str(candidate)
    found = shutil.which(name)
    if found:
        return found
    if name in ("cmake", "ctest"):
        vs = Path(os.environ.get("ProgramFiles", r"C:\Program Files")) / "Microsoft Visual Studio"
        candidates = sorted(
            vs.glob(f"*/*/Common7/IDE/CommonExtensions/Microsoft/CMake/CMake/bin/{name}.exe")
        )
        if candidates:
            return str(candidates[-1])
    raise FileNotFoundError(f"Required tool '{name}' not found; run scripts/setup_dev_tools.ps1")


def run(command: list[str], *, env: dict | None = None) -> None:
    print("+ " + subprocess.list2cmdline(command), flush=True)
    subprocess.run(command, cwd=ROOT, env=env, check=True)


def source_checks() -> None:
    run(["cargo", "fmt", "--all", "--", "--check"])
    run([tool("ruff"), "format", "--check", "."])
    run([tool("ruff"), "check", "."])
    for script in ("generate_shared_contracts.py", "release_version.py"):
        run([sys.executable, str(ROOT / "scripts" / script), "--check"])
    run([sys.executable, "scripts/check_utf8_sources.py"])
    run([sys.executable, "scripts/check_shared_rules.py"])
    run([sys.executable, "scripts/check_architecture.py"])
    run(
        [
            sys.executable,
            "-m",
            "unittest",
            "discover",
            "-s",
            "scripts",
            "-p",
            "test_standardization.py",
        ]
    )
    for script in ("check_clang_format.ps1", "check_powershell.ps1", "check_installer_sources.ps1"):
        run(
            ["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", f"scripts/{script}"]
        )


def rust_checks() -> None:
    # Tests must never modify the developer's installed IME data.
    parent = ROOT / "dist/verification-data"
    parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="rust-", dir=parent) as directory:
        env = dict(os.environ)
        env["LOCALAPPDATA"] = directory
        env["SRF_USER_DICT_PATH"] = str(Path(directory) / "user_dict.kxj")
        for name, features in plan()["rust_profiles"].items():
            print(f"Rust profile: {name}", flush=True)
            common = ["--workspace", "--all-targets", "--locked", *features]
            run(["cargo", "clippy", *common, "--", "-D", "warnings"], env=env)
            run(["cargo", "test", *common], env=env)


def native_checks(build_dir: Path, architecture: str) -> None:
    run(
        [
            tool("cmake"),
            "-S",
            "tsf-tip",
            "-B",
            str(build_dir),
            "-A",
            architecture,
            "-DBUILD_TESTING=ON",
        ]
    )
    run(
        [
            tool("cmake"),
            "--build",
            str(build_dir),
            "--config",
            "Release",
            "--parallel",
            "1",
            "--target",
            plan()["native_target"],
            "--",
            "/nodeReuse:false",
        ]
    )
    run([tool("ctest"), "--test-dir", str(build_dir), "-C", "Release", "--output-on-failure"])


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("scope", choices=("source", "rust", "native", "all"))
    parser.add_argument("--architecture", choices=plan()["native_architectures"])
    parser.add_argument("--build-dir", type=Path)
    args = parser.parse_args()
    if args.build_dir and not args.architecture:
        parser.error("--build-dir requires --architecture so CMake caches cannot mix architectures")
    if args.scope in ("source", "all"):
        source_checks()
    if args.scope in ("rust", "all"):
        rust_checks()
    if args.scope in ("native", "all"):
        for architecture in (
            [args.architecture] if args.architecture else plan()["native_architectures"]
        ):
            native_checks(
                args.build_dir or ROOT / "dist" / f"verify-tsf-{architecture}", architecture
            )


if __name__ == "__main__":
    main()
