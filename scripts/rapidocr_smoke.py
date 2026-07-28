#!/usr/bin/env python3
"""Smoke-test the local RapidOCR source tree with PP-OCRv5 mobile models."""

from __future__ import annotations

import argparse
import json
import sys
import time
from pathlib import Path


def repo_root() -> Path:
    return Path(__file__).resolve().parents[1]


def default_rapidocr_root() -> Path:
    return repo_root() / "RapidOCR-3.9.0"


def default_image() -> Path:
    return (
        default_rapidocr_root()
        / "python"
        / "tests"
        / "test_files"
        / "ch_en_num.jpg"
    )


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Run a local RapidOCR PP-OCRv5 mobile smoke test."
    )
    parser.add_argument(
        "--rapidocr-root",
        type=Path,
        default=default_rapidocr_root(),
        help="Path to RapidOCR source root.",
    )
    parser.add_argument(
        "--image",
        type=Path,
        default=default_image(),
        help="Image to recognize.",
    )
    parser.add_argument(
        "--model-root",
        type=Path,
        default=None,
        help="Directory for downloaded ONNX models. Defaults to RapidOCR's package model dir.",
    )
    parser.add_argument(
        "--vis",
        type=Path,
        default=repo_root() / "dist" / "rapidocr_smoke_vis.jpg",
        help="Visualization output path.",
    )
    parser.add_argument(
        "--json",
        type=Path,
        default=repo_root() / "dist" / "rapidocr_smoke.json",
        help="JSON report output path.",
    )
    return parser.parse_args()


def model_files(model_root: Path) -> list[dict[str, object]]:
    if not model_root.exists():
        return []
    return [
        {
            "name": path.name,
            "bytes": path.stat().st_size,
        }
        for path in sorted(model_root.glob("*.onnx"))
    ]


def main() -> int:
    args = parse_args()
    rapidocr_python = args.rapidocr_root / "python"
    if not rapidocr_python.is_dir():
        raise SystemExit(f"RapidOCR python directory not found: {rapidocr_python}")
    if not args.image.is_file():
        raise SystemExit(f"Smoke image not found: {args.image}")

    sys.path.insert(0, str(rapidocr_python))

    from rapidocr import EngineType, LangCls, LangDet, LangRec, ModelType, OCRVersion, RapidOCR

    model_root = (
        args.model_root
        if args.model_root is not None
        else rapidocr_python / "rapidocr" / "models"
    )
    model_root.mkdir(parents=True, exist_ok=True)

    params = {
        "Global.model_root_dir": str(model_root),
        "Global.log_level": "warning",
        "Det.engine_type": EngineType.ONNXRUNTIME,
        "Det.ocr_version": OCRVersion.PPOCRV5,
        "Det.lang_type": LangDet.CH,
        "Det.model_type": ModelType.MOBILE,
        "Cls.engine_type": EngineType.ONNXRUNTIME,
        "Cls.ocr_version": OCRVersion.PPOCRV5,
        "Cls.lang_type": LangCls.CH,
        "Cls.model_type": ModelType.MOBILE,
        "Rec.engine_type": EngineType.ONNXRUNTIME,
        "Rec.ocr_version": OCRVersion.PPOCRV5,
        "Rec.lang_type": LangRec.CH,
        "Rec.model_type": ModelType.MOBILE,
    }

    init_start = time.perf_counter()
    engine = RapidOCR(params=params)
    init_seconds = time.perf_counter() - init_start

    infer_start = time.perf_counter()
    result = engine(args.image)
    infer_seconds = time.perf_counter() - infer_start

    args.vis.parent.mkdir(parents=True, exist_ok=True)
    try:
        result.vis(str(args.vis))
        vis_path = str(args.vis)
    except Exception as exc:  # visualization is useful, but OCR success is the point.
        vis_path = f"failed: {exc}"

    report = {
        "rapidocr_root": str(args.rapidocr_root),
        "image": str(args.image),
        "model_root": str(model_root),
        "models": model_files(model_root),
        "ocr_version": "PP-OCRv5",
        "model_type": "mobile",
        "engine": "onnxruntime",
        "init_seconds": round(init_seconds, 3),
        "infer_seconds": round(infer_seconds, 3),
        "step_seconds": [round(v, 3) for v in (result.elapse_list or [])],
        "texts": list(result.txts or []),
        "scores": [round(float(v), 4) for v in (result.scores or [])],
        "boxes": result.boxes.tolist() if result.boxes is not None else [],
        "vis": vis_path,
    }

    args.json.parent.mkdir(parents=True, exist_ok=True)
    args.json.write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")
    print(json.dumps(report, ensure_ascii=False, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
