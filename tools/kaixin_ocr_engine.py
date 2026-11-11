#!/usr/bin/env python3
"""Command-line OCR helper for Kaixin Input Method.

The helper keeps stdout machine-readable. In one-shot mode every run prints
exactly one JSON object. In --stdio mode every input line is a JSON request and
every output line is one JSON response. Diagnostic output from RapidOCR is
redirected to stderr so the Rust UI can safely parse stdout.
"""

from __future__ import annotations

import argparse
import base64
import contextlib
import json
import math
import os
import sys
import time
import traceback
from collections import OrderedDict
from pathlib import Path
from typing import Any

from kaixin_ocr_policy import (
    PROFILE_LIMITS, PROFILE_MODELS, PROFILE_SECONDS, WorkBudget, analyze_image,
    image_tiles, intersection, line_bounds, lines_in_region, needs_detector_retry,
    normalize_profile, rect_area, refinement_regions, should_retry_preprocess,
)

if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8")
if hasattr(sys.stderr, "reconfigure"):
    sys.stderr.reconfigure(encoding="utf-8")


REQUIRED_MODEL_FILES = (
    "PP-OCRv6_det_medium.onnx",
    "PP-OCRv6_det_small.onnx",
    "PP-OCRv6_rec_medium.onnx",
    "PP-OCRv6_rec_small.onnx",
    "ppocrv6_dict.txt",
)
EXECUTION_PROVIDERS = ("auto", "cpu", "vulkan", "directml", "cuda")
MAX_ENCODED_IMAGE_BYTES = 64 * 1024 * 1024
MAX_DECODED_IMAGE_BYTES = 48 * 1024 * 1024
MAX_IMAGE_PIXELS = 40_000_000
MAX_IMAGE_SIDE = 16_384


def env_int(name: str, default: int) -> int:
    try:
        return int(os.environ.get(name, default))
    except (TypeError, ValueError):
        return default


def env_bool(name: str, default: bool) -> bool:
    value = os.environ.get(name)
    if value is None:
        return default
    return value.strip().lower() not in {"0", "false", "off", "no", "disabled"}


def env_choice(name: str, default: str, choices: tuple[str, ...]) -> str:
    value = os.environ.get(name, default).strip().lower()
    return value if value in choices else default


def default_intra_op_threads() -> int:
    # Bound inference parallelism so high-core-count PCs stay responsive.
    # KAIXIN_OCR_INTRA_OP_THREADS / --intra-op-threads still override this.
    return max(1, min(4, (os.cpu_count() or 1) - 2))


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Recognize an image with local RapidOCR.")
    parser.add_argument("image", type=Path, nargs="?", help="Image path to recognize.")
    parser.add_argument("--stdio", action="store_true", help="Read JSON lines from stdin.")
    parser.add_argument(
        "--rapidocr-root", type=Path, required=True, help="Path to packaged RapidOCR root."
    )
    parser.add_argument(
        "--model-root",
        type=Path,
        default=None,
        help="Directory for ONNX models. Defaults to RapidOCR's package model dir.",
    )
    parser.add_argument(
        "--lang",
        choices=("auto", "mixed", "zh", "zh_tw", "en"),
        default="mixed",
        help="Recognition language. zh_tw uses the bundled PP-OCRv6 multilingual model.",
    )
    parser.add_argument(
        "--profile",
        choices=tuple(PROFILE_LIMITS),
        default="balanced",
        help="Speed/accuracy profile used for adaptive image scaling.",
    )
    parser.add_argument(
        "--max-side-len",
        type=int,
        default=None,
        help="Override adaptive maximum image side.",
    )
    parser.add_argument(
        "--intra-op-threads",
        type=int,
        default=env_int("KAIXIN_OCR_INTRA_OP_THREADS", default_intra_op_threads()),
        help="CPU inference worker threads.",
    )
    parser.add_argument(
        "--inter-op-threads",
        type=int,
        default=env_int("KAIXIN_OCR_INTER_OP_THREADS", 1),
        help="Legacy compatibility option.",
    )
    parser.add_argument(
        "--rec-batch-num",
        type=int,
        default=env_int("KAIXIN_OCR_REC_BATCH_NUM", 8),
        help="Recognition batch size.",
    )
    parser.add_argument(
        "--cpu-mem-arena",
        action=argparse.BooleanOptionalAction,
        default=env_bool("KAIXIN_OCR_CPU_MEM_ARENA", True),
        help="Legacy compatibility option.",
    )
    parser.add_argument(
        "--provider",
        choices=EXECUTION_PROVIDERS,
        default=env_choice("KAIXIN_OCR_PROVIDER", "cpu", EXECUTION_PROVIDERS),
        help="Compatibility option; OCR always runs on CPU.",
    )
    parser.add_argument("--min-score", type=float, default=0.5, help="Minimum line score.")
    parser.add_argument("--cache-engines", type=int, choices=(1, 2), default=2,
                        help="Maximum resident model configurations (LRU).")
    parser.add_argument("--fast-mode", action=argparse.BooleanOptionalAction,
                        default=env_bool("KAIXIN_OCR_FAST_MODE", False),
                        help="Opt-in recognition width optimization, fast profile only.")
    parser.add_argument(
        "--download-retries",
        type=int,
        default=1,
        help="Retries for engine initialization. Models are never downloaded by this helper.",
    )
    parser.add_argument("--vis", type=Path, default=None, help="Optional visualization image path.")
    parser.add_argument("--pretty", action="store_true", help="Pretty-print JSON.")
    return parser.parse_args()


def emit(payload: dict[str, Any], pretty: bool) -> int:
    print(json.dumps(payload, ensure_ascii=False, indent=2 if pretty else None))
    return 0 if payload.get("ok") else 1


def decode_input_text(value: Any) -> str:
    if isinstance(value, bytes):
        return value.decode("utf-8-sig", errors="replace")
    return str(value)


def enum_params(
    model_root: Path,
    args: argparse.Namespace,
    profile: str = "balanced",
    detector: str | None = None,
    limit: int | None = None,
) -> dict[str, Any]:
    from rapidocr import EngineType, LangDet, LangRec, ModelType, OCRVersion

    profile = normalize_profile(profile)
    det, rec = PROFILE_MODELS[profile]
    det = detector or det
    limit = limit or PROFILE_LIMITS[profile][0]
    params = {
        "Global.model_root_dir": str(model_root),
        "Global.log_level": "warning",
        "Global.text_score": 0.0,
        "Global.use_cls": False,
        "Global.min_side_len": 0,
        "Global.max_side_len": limit,
        "EngineConfig.ppocr_cpp.use_vulkan": False,
        "EngineConfig.ppocr_cpp.backend": "cpu",
        "Det.engine_type": EngineType.PPOCR_CPP,
        "Det.ocr_version": OCRVersion.PPOCRV6,
        "Det.lang_type": LangDet.CH,
        "Det.model_type": ModelType.SMALL if det == "small" else ModelType.MEDIUM,
        # `min` enlarges a 360x48 line image to roughly 4800x640. The helper
        # controls scaling through Global.{min,max}_side_len instead, while
        # detector `max` guarantees that a narrow crop is never exploded.
        "Det.limit_type": "max",
        "Det.limit_side_len": limit,
        "Det.thresh": 0.3,
        "Det.box_thresh": 0.5,
        "Det.unclip_ratio": 1.6,
        "Rec.engine_type": EngineType.PPOCR_CPP,
        "Rec.ocr_version": OCRVersion.PPOCRV6,
        # All language choices share these multilingual weights and dictionary.
        "Rec.lang_type": LangRec.CH,
        "Rec.model_type": ModelType.SMALL if rec == "small" else ModelType.MEDIUM,
        "Rec.rec_img_shape": [3, 48, 320],
        "Rec.rec_batch_num": max(1, int(args.rec_batch_num)),
        "Rec.fast_mode": bool(getattr(args, "fast_mode", False) and profile == "fast"),
        "Det.model_path": str(model_root / f"PP-OCRv6_det_{det}.onnx"),
        "Rec.model_path": str(model_root / f"PP-OCRv6_rec_{rec}.onnx"),
        "Rec.rec_keys_path": str(model_root / "ppocrv6_dict.txt"),
    }
    return params


def adaptive_limits(
    profile: str, image_shape: Any, max_side_override: Any = None
) -> tuple[int, float]:
    profile = normalize_profile(profile)
    max_side_len, max_upscale = PROFILE_LIMITS[profile]
    if max_side_override is not None:
        max_side_len = max(320, min(int(max_side_override), 4096))

    height, width = int(image_shape[0]), int(image_shape[1])
    short_side = max(1, min(height, width))
    # Screen text in a crop whose short edge is already >=20 px should retain
    # native pixels. Very tiny inputs may be enlarged, but never beyond the
    # profile's bounded multiplier.
    if short_side >= 20 or max_upscale <= 1.0:
        min_side_len = 0.0
    else:
        min_side_len = min(30.0, short_side * max_upscale)
    return max_side_len, min_side_len


def model_files(model_root: Path) -> list[dict[str, Any]]:
    if not model_root.exists():
        return []
    return [
        {"name": path.name, "bytes": path.stat().st_size}
        for path in sorted(model_root.glob("*.onnx"))
    ]


def missing_required_models(model_root: Path) -> list[Path]:
    return [model_root / name for name in REQUIRED_MODEL_FILES if not (model_root / name).is_file()]


def elapsed_ms(values: Any) -> list[float]:
    return [
        round(float(value) * 1000, 2)
        for value in (values or [])
        if isinstance(value, (int, float))
    ]


def to_line_items(result: Any, min_score: float) -> list[dict[str, Any]]:
    texts = list(result.txts or [])
    scores = [float(v) for v in (result.scores or [])]
    boxes = result.boxes.tolist() if result.boxes is not None else []

    lines: list[dict[str, Any]] = []
    for idx, text in enumerate(texts):
        score = scores[idx] if idx < len(scores) else 0.0
        if score < min_score:
            continue
        lines.append(
            {
                "index": idx,
                "text": text,
                "score": round(score, 6),
                "box": boxes[idx] if idx < len(boxes) else None,
            }
        )
    return lines


def recognition_stats(lines: list[dict[str, Any]]) -> tuple[int, float, int]:
    chars = sum(len(str(line.get("text", "")).strip()) for line in lines)
    average = (
        sum(float(line.get("score", 0.0)) for line in lines) / len(lines)
        if lines
        else 0.0
    )
    return chars, average, len(lines)


def recognition_quality(lines: list[dict[str, Any]]) -> tuple[float, float]:
    """Balance evidence and confidence; extra low-quality characters cannot win alone."""
    if not lines:
        return 0.0, 0.0
    evidence = 0.0
    coverage = 0.0
    seen: set[str] = set()
    for line in lines:
        text = str(line.get("text", "")).strip()
        if not text:
            continue
        score = max(0.0, min(1.0, float(line.get("score", 0.0))))
        useful = sum(char.isalnum() for char in text) / len(text)
        repeats = sum(a == b == c for a, b, c in zip(text, text[1:], text[2:]))
        repeat_penalty = 1.0 - min(0.75, repeats / max(1, len(text) - 2))
        duplicate_weight = 0.5 if text in seen else 1.0
        seen.add(text)
        weight = score ** 3 * (0.5 + useful * 0.5) * repeat_penalty * duplicate_weight
        evidence += math.log1p(len(text)) * weight
        box = line.get("box")
        if box:
            area = abs(sum(box[i][0] * box[(i + 1) % len(box)][1]
                           - box[(i + 1) % len(box)][0] * box[i][1]
                           for i in range(len(box)))) / 2
            coverage += area * weight
    _, average, _ = recognition_stats(lines)
    return average ** 2 * math.log1p(evidence) * (1.0 + 0.03 * math.log1p(coverage)), average


def merge_tile_lines(lines: list[dict[str, Any]]) -> list[dict[str, Any]]:
    """Suppress overlapping detections, never equal text at unrelated locations."""
    def bounds(line: dict[str, Any]) -> tuple[float, float, float, float]:
        box = line["box"]
        return min(p[0] for p in box), min(p[1] for p in box), max(p[0] for p in box), max(p[1] for p in box)

    kept: list[dict[str, Any]] = []
    for line in sorted(lines, key=lambda item: (not item.get("tile_edge", False), item["score"]), reverse=True):
        x0, y0, x1, y1 = bounds(line)
        duplicate = False
        replace = []
        for previous in kept:
            a0, b0, a1, b1 = bounds(previous)
            intersection = max(0, min(x1, a1) - max(x0, a0)) * max(0, min(y1, b1) - max(y0, b0))
            smaller = min((x1 - x0) * (y1 - y0), (a1 - a0) * (b1 - b0))
            if smaller > 0 and intersection / smaller > 0.65:
                text = "".join(str(line["text"]).split())
                previous_text = "".join(str(previous["text"]).split())
                # A clipped prefix often has an even higher confidence than
                # the complete line from its neighbour. Preserve completeness.
                if (previous_text and previous_text != text and previous_text in text
                        and line["score"] >= 0.8 and (x1-x0)*(y1-y0) >= (a1-a0)*(b1-b0)):
                    replace.append(previous)
                    continue
                duplicate = True
                break
        if not duplicate:
            replaced = {id(item) for item in replace}
            kept = [item for item in kept if id(item) not in replaced]
            kept.append(line)
    kept.sort(key=lambda item: (bounds(item)[1], bounds(item)[0]))
    rows: list[list[dict[str, Any]]] = []
    for line in kept:
        _, top, _, bottom = bounds(line)
        if rows:
            _, row_top, _, row_bottom = bounds(rows[-1][0])
            same_row = abs((top + bottom) - (row_top + row_bottom)) <= min(bottom - top, row_bottom - row_top)
        else:
            same_row = False
        if same_row:
            rows[-1].append(line)
        else:
            rows.append([line])
    kept = []
    for row in rows:
        joined = []
        for line in sorted(row, key=lambda item: bounds(item)[0]):
            if joined and joined[-1].get("clip_right") and line.get("clip_left"):
                previous = joined[-1]
                a0, b0, a1, b1 = bounds(previous)
                x0, y0, x1, y1 = bounds(line)
                overlap = max(0, min(a1, x1) - max(a0, x0))
                left, right = str(previous["text"]).rstrip(), str(line["text"]).lstrip()
                common = next((n for n in range(min(len(left), len(right)), 1, -1)
                               if left[-n:] == right[:n]), 0)
                if overlap and common:
                    previous = dict(previous, text=left + right[common:],
                                    score=min(previous["score"], line["score"]),
                                    box=[[a0, min(b0, y0)], [max(a1, x1), min(b0, y0)],
                                         [max(a1, x1), max(b1, y1)], [a0, max(b1, y1)]],
                                    clip_right=line.get("clip_right", False))
                    joined[-1] = previous
                    continue
            joined.append(line)
        kept.extend(joined)
    for index, line in enumerate(kept):
        line["index"] = index
        line.pop("tile_edge", None)
    return kept


def error_payload(exc: BaseException) -> dict[str, Any]:
    return {
        "ok": False,
        "engine": "rapidocr",
        "error": str(exc),
        "traceback": traceback.format_exc(),
    }


def decode_memory_image(encoded: Any) -> bytes:
    value = str(encoded)
    if len(value) > MAX_ENCODED_IMAGE_BYTES:
        raise ValueError(
            f"encoded image is too large: {len(value)} bytes "
            f"(limit {MAX_ENCODED_IMAGE_BYTES})"
        )
    image_bytes = base64.b64decode(value, validate=True)
    if len(image_bytes) > MAX_DECODED_IMAGE_BYTES:
        raise ValueError(
            f"decoded image is too large: {len(image_bytes)} bytes "
            f"(limit {MAX_DECODED_IMAGE_BYTES})"
        )
    return image_bytes


def validate_image_shape(image_array: Any) -> None:
    if image_array is None or getattr(image_array, "ndim", 0) < 2:
        raise ValueError("image decoder returned an invalid image")
    height, width = (int(value) for value in image_array.shape[:2])
    if height <= 0 or width <= 0:
        raise ValueError(f"image has invalid dimensions: {width}x{height}")
    if max(width, height) > MAX_IMAGE_SIDE or width * height > MAX_IMAGE_PIXELS:
        raise ValueError(
            f"image dimensions exceed safety limit: {width}x{height} "
            f"(max side {MAX_IMAGE_SIDE}, max pixels {MAX_IMAGE_PIXELS})"
        )


class RapidOcrService:
    def __init__(self, args: argparse.Namespace) -> None:
        self.args = args
        os.environ.setdefault("PPOCR_BENCH_THREADS", str(max(1, int(args.intra_op_threads))))
        self.rapidocr_root = args.rapidocr_root.expanduser().resolve()
        self.rapidocr_python = self.rapidocr_root / "python"
        if not (self.rapidocr_python / "rapidocr").is_dir():
            raise RuntimeError(f"RapidOCR python package not found: {self.rapidocr_python}")
        sys.path.insert(0, str(self.rapidocr_python))

        self.model_root = (
            args.model_root.expanduser().resolve()
            if args.model_root
            else self.rapidocr_python / "rapidocr" / "models"
        )
        missing_models = missing_required_models(self.model_root)
        if missing_models:
            raise RuntimeError(
                "required RapidOCR model files are missing: "
                + ", ".join(str(path) for path in missing_models)
            )

        self._rapidocr_cls = None
        # Keys describe actual models and native creation parameters, not UI
        # language labels. Explicitly close evicted native handles before load.
        self._engines: OrderedDict[tuple[Any, ...], Any] = OrderedDict()
        self._warmed: set[tuple[Any, ...]] = set()
        self.cache_limit = max(1, min(2, int(getattr(args, "cache_engines", 2))))

    def _rapidocr_class(self) -> Any:
        if self._rapidocr_cls is None:
            with contextlib.redirect_stdout(sys.stderr):
                from rapidocr import RapidOCR

            self._rapidocr_cls = RapidOCR
        return self._rapidocr_cls

    def _engine_params(self, profile: str, detector: str | None = None,
                       limit: int | None = None) -> tuple[dict[str, Any], tuple[Any, ...]]:
        params = enum_params(self.model_root, self.args, profile, detector, limit)
        return params, tuple(sorted((key, str(value)) for key, value in params.items()))

    def _load_engine(self, profile: str, detector: str | None = None,
                     limit: int | None = None) -> tuple[Any, float, bool, tuple[Any, ...]]:
        params, key = self._engine_params(profile, detector, limit)
        if key in self._engines:
            self._engines.move_to_end(key)
            return self._engines[key], 0.0, True, key
        while len(self._engines) >= self.cache_limit:
            self._clear_engine(next(iter(self._engines)))
        rapidocr_cls = self._rapidocr_class()
        with contextlib.redirect_stdout(sys.stderr):
            init_start = time.perf_counter()
            engine = rapidocr_cls(params=params)
            engine._ensure_engine()
        self._engines[key] = engine
        return engine, time.perf_counter() - init_start, False, key

    def _clear_engine(self, key: tuple[Any, ...]) -> None:
        engine = self._engines.pop(key, None)
        self._warmed.discard(key)
        native = getattr(engine, "_engine", None)
        if native is not None:
            native.close()
            engine._engine = None

    def close(self) -> None:
        for key in list(self._engines):
            self._clear_engine(key)

    def warmup(self, lang: str = "mixed", profile: str = "balanced") -> dict[str, Any]:
        import cv2
        import numpy as np

        profile = normalize_profile(profile)
        engine, init_seconds, cached, key = self._load_engine(profile)
        engine.max_side_len = PROFILE_LIMITS[profile][0]
        engine.min_side_len = 0
        warm_start = time.perf_counter()
        already_warm = key in self._warmed
        if not already_warm:
            sample = np.full((48, 320, 3), 255, dtype=np.uint8)
            cv2.putText(sample, "Kaixin OCR 2026", (8, 33), cv2.FONT_HERSHEY_SIMPLEX,
                        0.8, (0, 0, 0), 2, cv2.LINE_AA)
            try:
                with contextlib.redirect_stdout(sys.stderr):
                    engine(sample, use_det=True, use_cls=False, use_rec=True)
                    # Explicit rec-only execution also works if detection found
                    # no box, and warms the recognizer's first graph execution.
                    engine(sample, use_det=False, use_cls=False, use_rec=True)
                self._warmed.add(key)
            except Exception:
                self._clear_engine(key)
                raise
        return {
            "ok": True,
            "command": "warmup",
            "cached": cached,
            "profile": profile,
            "detector_variant": PROFILE_MODELS[profile][0],
            "recognizer_variant": PROFILE_MODELS[profile][1],
            "already_warm": already_warm,
            "warmed_detection": True,
            "warmed_recognition": True,
            "cached_engines": len(self._engines),
            "init_ms": round(init_seconds * 1000, 2),
            "warmup_ms": round((time.perf_counter() - warm_start) * 1000, 2),
        }

    def crop_border(self, image_value: Any, output_value: Any) -> dict[str, Any]:
        """Run the OpenCV border detector in the already persistent worker."""
        from kaixin_cv_crop import find_rect, read_image, safe_border_crop_rect, write_image

        image = Path(str(image_value)).expanduser().resolve()
        output = Path(str(output_value)).expanduser().resolve()
        if not image.is_file():
            raise FileNotFoundError(f"image not found: {image}")
        source = read_image(image)
        height, width = source.shape[:2]
        found = find_rect(source, 0.04, 0.985)
        if found is None:
            return {"ok": True, "command": "crop", "cropped": False, "width": width, "height": height}
        score, (x, y, rect_width, rect_height), density = found
        crop_rect = safe_border_crop_rect(x, y, rect_width, rect_height, width, height)
        if crop_rect is None:
            return {
                "ok": True,
                "command": "crop",
                "cropped": False,
                "reason": "unsafe-or-full-image-border",
                "width": width,
                "height": height,
            }
        x0, y0, x1, y1 = crop_rect
        write_image(output, source[y0:y1, x0:x1])
        return {
            "ok": True,
            "command": "crop",
            "cropped": True,
            "output": str(output),
            "rect": [x0, y0, x1 - x0, y1 - y0],
            "width": width,
            "height": height,
            "confidence": round(float(score), 6),
            "border_density": round(float(density), 6),
        }

    def recognize(
        self,
        image_value: Any,
        lang: str,
        min_score: float,
        vis_value: Any = None,
        profile: str = "balanced",
        max_side_len: Any = None,
    ) -> dict[str, Any]:
        request_start = time.perf_counter()
        if lang not in {"auto", "mixed", "zh", "zh_tw", "en"}:
            raise ValueError(f"unsupported language: {lang!r}")

        image_bytes: bytes | None = None
        if isinstance(image_value, dict):
            encoded = image_value.get("image_bytes_base64")
            if encoded:
                image_bytes = decode_memory_image(encoded)
            image_value = image_value.get("image")
        image = Path(str(image_value)).expanduser().resolve()
        if image_bytes is None and not image.is_file():
            raise FileNotFoundError(f"image not found: {image}")

        profile = normalize_profile(profile)
        max_attempts = max(1, min(2, int(self.args.download_retries) + 1))
        load_start = time.perf_counter()
        from rapidocr.utils.load_image import LoadImage
        with contextlib.redirect_stdout(sys.stderr):
            image_array = LoadImage()(image_bytes if image_bytes is not None else image)
        validate_image_shape(image_array)
        print("KAIXIN_OCR_STAGE image-loaded", file=sys.stderr, flush=True)
        load_seconds = time.perf_counter() - load_start
        applied_max_side, applied_min_side = adaptive_limits(profile, image_array.shape, max_side_len)
        tiles = image_tiles(image_array.shape, applied_max_side)
        budget = WorkBudget(PROFILE_SECONDS[profile])
        analysis_start = time.perf_counter()
        analysis = analyze_image(image_array)
        preprocess_seconds = time.perf_counter() - analysis_start
        attempts: list[dict[str, Any]] = []
        total_init = 0.0
        total_infer = 0.0
        all_cached = True
        primary_det, primary_rec = PROFILE_MODELS[profile]
        step_seconds: list[float] = []

        def run_region(region: tuple[int, int, int, int], kind: str = "primary",
                       scale: float = 1.0, detector: str | None = None,
                       enhance: bool = False) -> list[dict[str, Any]] | None:
            nonlocal total_init, total_infer, preprocess_seconds, all_cached
            import cv2

            optional = kind != "primary"
            x0, y0, x1, y1 = region
            scale = min(scale, applied_max_side / max(x1 - x0, y1 - y0))
            target_width = max(1, round((x1 - x0) * scale))
            target_height = max(1, round((y1 - y0) * scale))
            pixels = target_width * target_height
            last_error: Exception | None = None
            for attempt in range(1, max_attempts + 1):
                if not budget.reserve(pixels, kind, optional):
                    return None
                attempt_start = time.perf_counter()
                infer_start = None
                attempt_infer = 0.0
                succeeded = False
                attempt_init = 0.0
                key = None
                try:
                    print(
                        f"KAIXIN_OCR_STAGE inference kind={kind} attempt={attempt}",
                        file=sys.stderr,
                        flush=True,
                    )
                    engine, init_seconds, cached, key = self._load_engine(profile, detector, applied_max_side)
                    total_init += init_seconds
                    all_cached = all_cached and cached
                    attempt_init = init_seconds
                    engine.max_side_len = applied_max_side
                    # Explicit scaling below also keeps tiny crops within the
                    # same pixel budget as ordinary images.
                    engine.min_side_len = 0
                    prepare_start = time.perf_counter()
                    source = image_array[y0:y1, x0:x1]
                    if source.shape[:2] != (target_height, target_width):
                        source = cv2.resize(source, (target_width, target_height), interpolation=cv2.INTER_CUBIC)
                    if enhance:
                        lab = cv2.cvtColor(source, cv2.COLOR_BGR2LAB)
                        lab[:, :, 0] = cv2.createCLAHE(clipLimit=2.0, tileGridSize=(8, 8)).apply(lab[:, :, 0])
                        source = cv2.cvtColor(lab, cv2.COLOR_LAB2BGR)
                    preprocess_seconds += time.perf_counter() - prepare_start
                    with contextlib.redirect_stdout(sys.stderr):
                        infer_start = time.perf_counter()
                        result = engine(source, use_det=True, use_cls=False, use_rec=True)
                        attempt_infer = time.perf_counter() - infer_start
                        succeeded = True
                    for index, value in enumerate(result.elapse_list or []):
                        while len(step_seconds) <= index:
                            step_seconds.append(0.0)
                        step_seconds[index] += float(value or 0.0)
                    lines = to_line_items(result, 0.0)
                    mapped = []
                    sx, sy = target_width / (x1 - x0), target_height / (y1 - y0)
                    for line in lines:
                        if line["box"] is None:
                            continue
                        box = line["box"]
                        line["clip_left"] = x0 > 0 and min(p[0] for p in box) < 4 * sx
                        line["clip_right"] = x1 < image_array.shape[1] and max(p[0] for p in box) > target_width - 4 * sx
                        line["tile_edge"] = (
                            (x0 > 0 and min(p[0] for p in box) < 4 * sx)
                            or (y0 > 0 and min(p[1] for p in box) < 4 * sy)
                            or (x1 < image_array.shape[1] and max(p[0] for p in box) > target_width - 4 * sx)
                            or (y1 < image_array.shape[0] and max(p[1] for p in box) > target_height - 4 * sy))
                        line["box"] = [[min(x1, max(x0, p[0] / sx + x0)),
                                        min(y1, max(y0, p[1] / sy + y0))] for p in box]
                        mapped.append(line)
                    return mapped
                except Exception as exc:
                    last_error = exc
                    if key is not None:
                        self._clear_engine(key)
                    if attempt < max_attempts:
                        print(f"RapidOCR init/inference failed, retrying ({attempt}/{max_attempts})...", file=sys.stderr)
                finally:
                    if infer_start is not None and not succeeded:
                        attempt_infer = time.perf_counter() - infer_start
                    total_infer += attempt_infer
                    attempts.append({
                        "kind": kind, "language": "multilingual", "retry": attempt, "ok": succeeded,
                        "detector": detector or primary_det, "recognizer": primary_rec,
                        "rect": list(region), "scale": round(scale, 3), "pixels": pixels,
                        "init_ms": round(attempt_init * 1000, 2),
                        "infer_ms": round(attempt_infer * 1000, 2),
                        "total_ms": round((time.perf_counter() - attempt_start) * 1000, 2),
                    })
            if optional:
                budget.skipped.append(f"{kind}:inference-error")
                print(f"Optional OCR pass failed: {last_error}", file=sys.stderr, flush=True)
                return None
            raise last_error or RuntimeError("RapidOCR inference failed")

        tile_lines = []
        tiny_scale = max(1.0, applied_min_side / max(1, min(image_array.shape[:2])))
        for tile in tiles:
            tile_lines.extend(run_region(tile, scale=tiny_scale) or [])
        lines = merge_tile_lines(tile_lines)

        def accept_region(region: tuple[int, int, int, int], candidate: list[dict[str, Any]] | None) -> None:
            nonlocal lines
            if not candidate:
                return
            previous = lines_in_region(lines, region)
            if recognition_quality(candidate) <= recognition_quality(previous):
                return
            # Do not remove a full line when an optional crop covers only part
            # of it. Seam deduplication prefers interior, confident results.
            remove = {id(line) for line in previous if (bounds := line_bounds(line)) is not None
                      and intersection(bounds, region) >= rect_area(bounds) * 0.95}
            lines = merge_tile_lines([line for line in lines if id(line) not in remove] + candidate)

        upscale = PROFILE_LIMITS[profile][1]
        if upscale > 1:
            for region in refinement_regions(lines, analysis):
                if max(region[2] - region[0], region[3] - region[1]) * upscale > applied_max_side:
                    # Retain the primary result rather than shrinking a long
                    # line immediately after upscaling it.
                    continue
                accept_region(region, run_region(region, "small-text", scale=upscale))

        if should_retry_preprocess(lines, analysis):
            for region in analysis.regions[:8]:
                previous = lines_in_region(lines, region)
                if previous and recognition_stats(previous)[1] >= 0.82:
                    continue
                accept_region(region, run_region(region, "contrast", enhance=True))

        if profile == "balanced" and needs_detector_retry(lines, analysis):
            for region in analysis.regions:
                if not lines_in_region(lines, region):
                    accept_region(region, run_region(region, "detector-fallback", detector="medium"))
                if budget.extra_calls >= 8:
                    break

        vis_status = None
        if vis_value:
            import numpy as np
            from rapidocr.output import RapidOCROutput

            result = RapidOCROutput(
                img=image_array,
                boxes=np.asarray([line["box"] for line in lines], dtype=np.float32).reshape(-1, 4, 2),
                txts=tuple(line["text"] for line in lines),
                scores=tuple(line["score"] for line in lines), elapse_list=step_seconds, word_results=(None,))
            vis_path = Path(str(vis_value)).expanduser().resolve()
            vis_path.parent.mkdir(parents=True, exist_ok=True)
            with contextlib.redirect_stdout(sys.stderr):
                result.vis(str(vis_path))
            vis_status = str(vis_path)

        low_confidence_lines = [line for line in lines if line["score"] < float(min_score)]
        for line in lines:
            line.pop("clip_left", None)
            line.pop("clip_right", None)
        lines = [line for line in lines if line["score"] >= float(min_score)]
        text = "\n".join(line["text"] for line in lines)
        return {
            "ok": True,
            "engine": "rapidocr",
            "backend": "rapidocrlite/cpu",
            "profile": profile,
            "ocr_version": "PP-OCRv6",
            "model_type": f"{primary_det}-det+{primary_rec}-rec",
            "detector_variant": primary_det,
            "recognizer_variant": primary_rec,
            "fast_mode": bool(getattr(self.args, "fast_mode", False) and profile == "fast"),
            "max_side_len": applied_max_side,
            "min_side_len": applied_min_side,
            "language": "mixed" if lang == "auto" else lang,
            "model_language": "multilingual",
            "auto_language_retry": False,
            "auto_preprocess_retry": any(item["kind"] == "contrast" for item in attempts),
            "detector_fallback": any(item["kind"] == "detector-fallback" for item in attempts),
            "image": str(image),
            "rapidocr_root": str(self.rapidocr_root),
            "model_root": str(self.model_root),
            "models": model_files(self.model_root),
            "cached": all_cached,
            "cached_engines": len(self._engines),
            "init_ms": round(total_init * 1000, 2),
            "load_ms": round(load_seconds * 1000, 2),
            "infer_ms": round(total_infer * 1000, 2),
            "preprocess_ms": round(preprocess_seconds * 1000, 2),
            "total_ms": round((time.perf_counter() - request_start) * 1000, 2),
            "attempts": attempts,
            "tile_count": len(tiles),
            "analysis": analysis.summary(),
            "work_pixels": budget.pixels,
            "optional_calls": budget.extra_calls,
            "skipped_passes": budget.skipped,
            "step_ms": elapsed_ms(step_seconds),
            "text": text,
            "lines": lines,
            "low_confidence_lines": low_confidence_lines,
            "vis": vis_status,
        }

    def recognize_request(self, request: dict[str, Any]) -> dict[str, Any]:
        return self.recognize(
            {
                "image": request.get("image"),
                "image_bytes_base64": request.get("image_bytes_base64"),
            },
            str(request.get("lang", self.args.lang)),
            float(request.get("min_score", self.args.min_score)),
            request.get("vis"),
            str(request.get("profile", self.args.profile)),
            request.get("max_side_len", request.get("limit_side_len", self.args.max_side_len)),
        )


def run_stdio(args: argparse.Namespace) -> int:
    try:
        service = RapidOcrService(args)
    except Exception as exc:
        print(json.dumps(error_payload(exc), ensure_ascii=False), flush=True)
        return 1

    stdin = getattr(sys.stdin, "buffer", sys.stdin)
    for raw_line in stdin:
        line = decode_input_text(raw_line).strip()
        if not line:
            continue
        try:
            request = json.loads(line)
            if request.get("command") == "quit":
                service.close()
                return 0
            if request.get("command") == "warmup":
                response = service.warmup(
                    str(request.get("lang", args.lang)),
                    str(request.get("profile", args.profile)),
                )
            elif request.get("command") == "crop":
                response = service.crop_border(request.get("image"), request.get("output"))
            else:
                response = service.recognize_request(request)
        except Exception as exc:
            response = error_payload(exc)
        print(json.dumps(response, ensure_ascii=False), flush=True)
    service.close()
    return 0


def main() -> int:
    args = parse_args()
    if args.stdio:
        return run_stdio(args)
    if args.image is None:
        return emit({"ok": False, "error": "image path is required"}, args.pretty)

    try:
        service = RapidOcrService(args)
        payload = service.recognize(
            args.image,
            args.lang,
            args.min_score,
            args.vis,
            args.profile,
            args.max_side_len,
        )
        return emit(payload, args.pretty)
    except Exception as exc:
        return emit(error_payload(exc), args.pretty)


if __name__ == "__main__":
    raise SystemExit(main())
