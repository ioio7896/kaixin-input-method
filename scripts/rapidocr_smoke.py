#!/usr/bin/env python3
"""Smoke-test the native Rust PP-OCRv6 worker and models used by the OCR window."""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
import time
from pathlib import Path

if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8")
if hasattr(sys.stderr, "reconfigure"):
    sys.stderr.reconfigure(encoding="utf-8")


def repo_root() -> Path:
    return Path(__file__).resolve().parents[1]


def default_rapidocr_root() -> Path:
    return repo_root() / "RapidOCRLite"


def default_ocr_exe() -> Path:
    for candidate in (
        repo_root() / "pinyin-ime" / "target" / "release" / "srf_ime_ocr.exe",
        repo_root() / "pinyin-ime" / "target" / "debug" / "srf_ime_ocr.exe",
        repo_root() / "dist" / "kaixin-package-ocr" / "srf_ime_ocr.exe",
    ):
        if candidate.is_file():
            return candidate
    return repo_root() / "pinyin-ime" / "target" / "release" / "srf_ime_ocr.exe"


def default_image() -> Path:
    return repo_root() / "assets" / "kaixin-input-icon.png"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Run the production native Kaixin PP-OCRv6 worker smoke test."
    )
    parser.add_argument("--ocr-exe", type=Path, default=default_ocr_exe())
    parser.add_argument("--rapidocr-root", type=Path, default=default_rapidocr_root())
    parser.add_argument("--image", type=Path, default=default_image())
    parser.add_argument("--model-root", type=Path, default=None)
    parser.add_argument("--profile", choices=("fast", "balanced", "accurate"), default="balanced")
    parser.add_argument("--lang", choices=("mixed", "zh", "en"), default="mixed")
    parser.add_argument("--vis", type=Path, default=repo_root() / "dist" / "rapidocr_smoke_vis.jpg")
    parser.add_argument("--json", type=Path, default=repo_root() / "dist" / "rapidocr_smoke.json")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    if not args.ocr_exe.is_file():
        raise SystemExit(
            f"Native OCR executable not found: {args.ocr_exe} "
            "(build it with `cargo build --bin srf_ime_ocr`)"
        )
    if not args.image.is_file():
        raise SystemExit(f"Smoke image not found: {args.image}")

    command = [
        str(args.ocr_exe),
        "--ocr-worker",
        str(args.image),
        "--rapidocr-root",
        str(args.rapidocr_root),
        "--profile",
        args.profile,
        "--lang",
        args.lang,
        "--vis",
        str(args.vis),
    ]
    if args.model_root is not None:
        command.extend(("--model-root", str(args.model_root)))

    started = time.perf_counter()
    completed = subprocess.run(command, capture_output=True, text=True, encoding="utf-8")
    elapsed_ms = round((time.perf_counter() - started) * 1000, 2)
    try:
        payload = json.loads(completed.stdout)
    except json.JSONDecodeError as exc:
        raise SystemExit(
            f"OCR worker returned invalid JSON ({exc}); stderr: {completed.stderr.strip()}"
        ) from exc

    report = {
        "ok": completed.returncode == 0 and payload.get("ok") is True,
        "production_worker": str(args.ocr_exe),
        "process_elapsed_ms": elapsed_ms,
        "returncode": completed.returncode,
        "stderr": completed.stderr.strip(),
        "result": payload,
    }
    args.json.parent.mkdir(parents=True, exist_ok=True)
    args.json.write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")
    print(json.dumps(report, ensure_ascii=False, indent=2))
    return 0 if report["ok"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
