# SPDX-License-Identifier: Apache-2.0
"""Run all OCR profiles with the Python, helpers and models inside a staged package."""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import subprocess


def verify_ocr_package(repo: Path, package: Path) -> Path:
    package = package.resolve()
    expected = {"fast": "small-det+small-rec", "balanced": "small-det+medium-rec",
                "accurate": "medium-det+medium-rec"}
    requests = []
    for profile in expected:
        requests.extend([
            {"command": "warmup", "profile": profile, "lang": "zh"},
            {"command": "warmup", "profile": profile, "lang": "en"},
            {"image": str((repo / "assets/kaixin-input-icon.png").resolve()),
             "profile": profile, "lang": "auto", "min_score": 0.5},
        ])
    env = dict(os.environ, PYTHONPATH=str(package / ".python-packages"), PYTHONNOUSERSITE="1",
               PYTHONDONTWRITEBYTECODE="1", PYTHONUTF8="1", PYTHONIOENCODING="utf-8",
               PPOCR_BENCH_THREADS="4")
    env.pop("PPOCR_REC_FAST", None)
    command = [str(package / ".python-runtime/python.exe"), "-B",
               str(package / "tools/kaixin_ocr_engine.py"), "--stdio", "--rapidocr-root",
               str(package / "RapidOCRLite"), "--provider", "cpu", "--no-fast-mode"]
    completed = subprocess.run(command, input="".join(json.dumps(r) + "\n" for r in requests)
                               + '{"command":"quit"}\n', capture_output=True, text=True, encoding="utf-8",
                               env=env, cwd=package, timeout=180,
                               creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
    report = package.parent / f"ocr-validation-{package.name}.json"
    payloads = [json.loads(line) for line in completed.stdout.splitlines() if line.strip()]
    report.write_text(json.dumps({"version": (repo / "VERSION").read_text().strip(),
                                 "exit_code": completed.returncode, "responses": payloads,
                                 "stderr": completed.stderr}, ensure_ascii=False, indent=2), encoding="utf-8")
    if completed.returncode or len(payloads) != len(requests):
        raise RuntimeError(f"Staged OCR worker failed or returned an incomplete response; see {report}")
    for index, (profile, model) in enumerate(expected.items()):
        warm, alias, result = payloads[index * 3:index * 3 + 3]
        if not all(p.get("ok") for p in (warm, alias, result)):
            raise RuntimeError(f"Staged OCR {profile} failed; see {report}")
        if not warm.get("warmed_detection") or not warm.get("warmed_recognition") or not alias.get("already_warm"):
            raise RuntimeError(f"Staged OCR warmup/language cache contract failed; see {report}")
        # The square app icon may be detected as one character per line. Its
        # recognized content, rather than incidental line breaks, is the gate.
        recognized = "".join(result.get("text", "").split())
        if (result.get("model_type") != model or "开心输入" not in recognized
                or result.get("auto_language_retry") or result.get("cached_engines", 99) > 2):
            raise RuntimeError(f"Staged OCR {profile} model/result/cache contract failed; see {report}")
    return report


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("package", type=Path)
    args = parser.parse_args()
    print(verify_ocr_package(Path(__file__).resolve().parents[1], args.package))
