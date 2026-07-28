#!/usr/bin/env python3
"""Validate cross-language rules shared by Rust, C++, installers, and diagnostics."""

from __future__ import annotations

import argparse
import os
import re
import subprocess
import sys
from pathlib import Path


APP_PATH_NAME = "kaixin"
CONFIG_FILE_NAME = "kaixin.ini"
LOG_DIR_NAME = "logs"
ENGINE_PIPE_BASE = r"\\.\pipe\KaixinInput_Engine_V5"
ENGINE_MUTEX_BASE = r"Local\KaixinInput_Engine_Mutex_V5"
FNV64_OFFSET = 1469598103934665603
FNV64_PRIME = 1099511628211
MASK64 = (1 << 64) - 1


def strip_windows_verbatim_prefix(text: str) -> str:
    if text.startswith(r"\\?\UNC\\"):
        return r"\\" + text[len(r"\\?\UNC\\") :]
    if text.startswith(r"\\?\UNC\\"):
        return r"\\" + text[len(r"\\?\UNC\\") :]
    if text.startswith(r"\\?\UNC"):
        rest = text[len(r"\\?\UNC") :].lstrip("\\")
        return r"\\" + rest
    if text.startswith(r"\\?\\"):
        return text[len(r"\\?\\") :]
    if text.startswith(r"\\?"):
        rest = text[len(r"\\?") :]
        return rest.lstrip("\\")
    return text


def stable_path_hash(text: str) -> int:
    value = text.lower()
    h = FNV64_OFFSET
    for unit in value.encode("utf-16le"):
        # Kept for clarity; the loop below consumes UTF-16 code units.
        _ = unit
    data = value.encode("utf-16le")
    for i in range(0, len(data), 2):
        unit = data[i] | (data[i + 1] << 8)
        h ^= unit
        h = (h * FNV64_PRIME) & MASK64
    return h


def suffix_for_install_root(path: str) -> str:
    return f"{stable_path_hash(strip_windows_verbatim_prefix(path)):016x}"


def pipe_name_for_suffix(suffix: str) -> str:
    return f"{ENGINE_PIPE_BASE}_{suffix}" if suffix else ENGINE_PIPE_BASE


def mutex_name_for_suffix(suffix: str) -> str:
    return f"{ENGINE_MUTEX_BASE}_{suffix}" if suffix else ENGINE_MUTEX_BASE


def check_spec_samples() -> list[str]:
    errors: list[str] = []
    plain = r"C:\Program Files (x86)\kaixin"
    verbatim = r"\\?\C:\Program Files (x86)\kaixin"
    expected = "70489ba2d8c271ce"
    if strip_windows_verbatim_prefix(verbatim) != plain:
        errors.append("verbatim path prefix was not stripped correctly")
    for sample in [plain, verbatim]:
        actual = suffix_for_install_root(sample)
        if actual != expected:
            errors.append(f"suffix mismatch for {sample}: got {actual}, expected {expected}")
    if pipe_name_for_suffix(expected) != rf"{ENGINE_PIPE_BASE}_{expected}":
        errors.append("pipe name derivation mismatch")
    if mutex_name_for_suffix(expected) != rf"{ENGINE_MUTEX_BASE}_{expected}":
        errors.append("mutex name derivation mismatch")
    if APP_PATH_NAME != "kaixin" or CONFIG_FILE_NAME != "kaixin.ini" or LOG_DIR_NAME != "logs":
        errors.append("shared app path constants changed unexpectedly")
    return errors


def powershell_json(script: str) -> str:
    cmd = [
        "powershell",
        "-NoProfile",
        "-ExecutionPolicy",
        "Bypass",
        "-Command",
        script,
    ]
    return subprocess.check_output(cmd, text=True, encoding="utf-8", errors="replace")


def running_engine_pipe_names() -> list[str]:
    script = r"""
$items = Get-CimInstance Win32_Process -Filter "name='srf_ime_engine.exe'" |
  Select-Object ProcessId,CommandLine
$items | ConvertTo-Json -Compress
"""
    try:
        raw = powershell_json(script).strip()
    except Exception:
        return []
    if not raw:
        return []
    import json

    data = json.loads(raw)
    if isinstance(data, dict):
        data = [data]
    pipes: list[str] = []
    for item in data:
        cmd = item.get("CommandLine") or ""
        match = re.search(r"--pipe-name(?:=|\s+)\"?([^\"\s]+)", cmd)
        if match:
            pipes.append(match.group(1))
    return pipes


def check_runtime(install_root: str | None) -> list[str]:
    errors: list[str] = []
    if not install_root:
        install_root = os.environ.get("KAIXIN_INSTALL_ROOT") or r"C:\Program Files (x86)\kaixin"
    expected_suffix = suffix_for_install_root(install_root)
    expected_pipe = pipe_name_for_suffix(expected_suffix)
    pipes = running_engine_pipe_names()
    if not pipes:
        print("shared-rules runtime: no running srf_ime_engine.exe found; skipped")
        return errors
    mismatches = [pipe for pipe in pipes if pipe != expected_pipe]
    if mismatches:
        errors.append(
            "running engine pipe mismatch: expected "
            f"{expected_pipe}, found {', '.join(mismatches)}"
        )
    else:
        print(f"shared-rules runtime: {len(pipes)} engine process(es), pipe={expected_pipe}")
    return errors


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--runtime", action="store_true", help="also check running engine process pipes")
    parser.add_argument("--install-root", help="install root used for --runtime checks")
    args = parser.parse_args()

    errors = check_spec_samples()
    if args.runtime:
        errors.extend(check_runtime(args.install_root))
    if errors:
        for error in errors:
            print(f"ERROR: {error}", file=sys.stderr)
        return 1
    print("shared rules check passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
