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
import os
import sys
import time
import traceback
from pathlib import Path
from typing import Any

if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8")
if hasattr(sys.stderr, "reconfigure"):
    sys.stderr.reconfigure(encoding="utf-8")


REQUIRED_MODEL_FILES = (
    "PP-OCRv6_det_medium.onnx",
    "PP-OCRv6_det_small.onnx",
    "PP-OCRv6_rec_medium.onnx",
)
FAST_DET_MODEL_NAMES = ("PP-OCRv6_det_small.onnx", "PP-OCRv6_det_int8.onnx")
PROFILE_LIMITS = {
    "fast": (1280, 1.0),
    "balanced": (1920, 1.5),
    "accurate": (2560, 2.0),
}
EXECUTION_PROVIDERS = ("auto", "cpu", "directml", "cuda")
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
    # Leave two logical cores available for the UI and screen capture.
    return max(1, (os.cpu_count() or 1) - 2)


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
        choices=("mixed", "zh", "en"),
        default="mixed",
        help="Recognition language. mixed and zh use the Chinese/multilingual model.",
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
        help="ONNXRuntime intra-op worker threads.",
    )
    parser.add_argument(
        "--inter-op-threads",
        type=int,
        default=env_int("KAIXIN_OCR_INTER_OP_THREADS", 1),
        help="ONNXRuntime inter-op worker threads.",
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
        help="Enable the ONNXRuntime CPU memory arena for persistent inference.",
    )
    parser.add_argument(
        "--provider",
        choices=EXECUTION_PROVIDERS,
        default=env_choice("KAIXIN_OCR_PROVIDER", "auto", EXECUTION_PROVIDERS),
        help="ONNX Runtime execution provider; auto selects an installed GPU provider or CPU.",
    )
    parser.add_argument("--min-score", type=float, default=0.5, help="Minimum line score.")
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
    lang: str,
    model_root: Path,
    args: argparse.Namespace,
    fast_det_model: Path | None = None,
) -> dict[str, Any]:
    from rapidocr import EngineType, LangDet, LangRec, ModelType, OCRVersion

    rec_lang = LangRec.EN if lang == "en" else LangRec.CH
    provider = selected_provider(args.provider)
    params = {
        "Global.model_root_dir": str(model_root),
        "Global.log_level": "warning",
        "Global.text_score": 0.0,
        "Global.use_cls": False,
        "Global.min_side_len": 0,
        "Global.max_side_len": PROFILE_LIMITS["balanced"][0],
        "EngineConfig.onnxruntime.intra_op_num_threads": max(1, int(args.intra_op_threads)),
        "EngineConfig.onnxruntime.inter_op_num_threads": max(1, int(args.inter_op_threads)),
        "EngineConfig.onnxruntime.enable_cpu_mem_arena": bool(args.cpu_mem_arena),
        "EngineConfig.onnxruntime.use_cuda": provider == "cuda",
        "EngineConfig.onnxruntime.use_dml": provider == "directml",
        "Det.engine_type": EngineType.ONNXRUNTIME,
        "Det.ocr_version": OCRVersion.PPOCRV6,
        "Det.lang_type": LangDet.CH,
        "Det.model_type": ModelType.MEDIUM,
        # `min` enlarges a 360x48 line image to roughly 4800x640. The helper
        # controls scaling through Global.{min,max}_side_len instead, while
        # detector `max` guarantees that a narrow crop is never exploded.
        "Det.limit_type": "max",
        "Rec.engine_type": EngineType.ONNXRUNTIME,
        "Rec.ocr_version": OCRVersion.PPOCRV6,
        "Rec.lang_type": rec_lang,
        "Rec.model_type": ModelType.MEDIUM,
        "Rec.rec_img_shape": [3, 48, 320],
        "Rec.rec_batch_num": max(1, int(args.rec_batch_num)),
    }
    if fast_det_model is not None:
        params["Det.model_type"] = ModelType.SMALL
        params["Det.model_path"] = str(fast_det_model)
    return params


def selected_provider(requested: str) -> str:
    """Return a provider supported by this packaged ONNX Runtime build."""
    try:
        import onnxruntime as ort

        available = set(ort.get_available_providers())
    except Exception:
        return "cpu"
    aliases = {
        "cuda": "CUDAExecutionProvider",
        "directml": "DmlExecutionProvider",
    }
    if requested == "auto":
        for candidate in ("cuda", "directml"):
            if aliases[candidate] in available:
                return candidate
        return "cpu"
    if requested in aliases and aliases[requested] in available:
        return requested
    return "cpu"


def normalize_profile(value: Any) -> str:
    profile = str(value or "balanced").strip().lower()
    return profile if profile in PROFILE_LIMITS else "balanced"


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


def recognition_quality(lines: list[dict[str, Any]]) -> tuple[int, float, int]:
    """Rank OCR attempts by useful text first, then confidence."""
    chars = sum(len(str(line.get("text", "")).strip()) for line in lines)
    average = (
        sum(float(line.get("score", 0.0)) for line in lines) / len(lines)
        if lines
        else 0.0
    )
    return chars, average, len(lines)


def should_retry_preprocess(lines: list[dict[str, Any]]) -> bool:
    chars, average, _ = recognition_quality(lines)
    return chars <= 1 or average < 0.62


def should_retry_english(lines: list[dict[str, Any]]) -> bool:
    text = "".join(str(line.get("text", "")) for line in lines)
    if not text:
        return True
    cjk = sum("\u4e00" <= char <= "\u9fff" for char in text)
    latin = sum(char.isascii() and char.isalpha() for char in text)
    _, average, _ = recognition_quality(lines)
    return (latin > cjk * 2 and latin >= 3) or average < 0.62


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
        self.provider = selected_provider(args.provider)
        self.fast_det_model = next(
            (
                self.model_root / name
                for name in FAST_DET_MODEL_NAMES
                if (self.model_root / name).is_file()
            ),
            None,
        )
        # mixed and zh use the same PP-OCRv6 multilingual recognizer. Cache by
        # effective recognizer and detector model rather than UI profile/size.
        self._engines: dict[tuple[str, str], Any] = {}
        self._detectors: dict[str, Any] = {}

    def _rapidocr_class(self) -> Any:
        if self._rapidocr_cls is None:
            with contextlib.redirect_stdout(sys.stderr):
                from rapidocr import RapidOCR

            self._rapidocr_cls = RapidOCR
        return self._rapidocr_cls

    def _effective_lang(self, lang: str) -> str:
        return "en" if lang == "en" else "zh"

    def _detector_variant(self, profile: str) -> str:
        return "fast" if normalize_profile(profile) == "fast" and self.fast_det_model else "medium"

    def _load_engine(self, lang: str, profile: str) -> tuple[Any, float, bool, str]:
        variant = self._detector_variant(profile)
        key = (self._effective_lang(lang), variant)
        if key in self._engines:
            return self._engines[key], 0.0, True, variant

        rapidocr_cls = self._rapidocr_class()
        params = enum_params(
            self._effective_lang(lang),
            self.model_root,
            self.args,
            self.fast_det_model if variant == "fast" else None,
        )
        with contextlib.redirect_stdout(sys.stderr):
            init_start = time.perf_counter()
            engine = rapidocr_cls(params=params)
            init_seconds = time.perf_counter() - init_start
        # RapidOCR builds detection and recognition together. Replace a newly
        # built duplicate detector when only the recognition language differs,
        # so English/Chinese engines retain one detector session at runtime.
        if variant in self._detectors:
            engine.text_det = self._detectors[variant]
        else:
            from rapidocr.ch_ppocr_det.utils import DetPreProcess

            # RapidOCR 3.9.2 initializes models lazily, so `text_det` is None
            # until its internal loader runs. Materialize it here before
            # applying the project-specific preprocessor and sharing it with
            # the other recognition-language engine.
            detector = engine._load_det_model()
            # RapidOCR's built-in max mode silently replaces configured limits
            # with 960/1500/2000. Keep its preprocessing implementation but
            # make the three Kaixin profiles use their explicit long-edge cap.
            detector.get_preprocess = lambda _max_wh, det=detector: DetPreProcess(
                det.limit_side_len, "max", det.mean, det.std
            )
            self._detectors[variant] = engine.text_det
        self._engines[key] = engine
        return engine, init_seconds, False, variant

    def _clear_engine(self, lang: str, profile: str) -> None:
        variant = self._detector_variant(profile)
        self._engines.pop((self._effective_lang(lang), variant), None)
        self._detectors.pop(variant, None)

    def warmup(self, lang: str = "mixed", profile: str = "balanced") -> dict[str, Any]:
        import numpy as np

        profile = normalize_profile(profile)
        engine, init_seconds, cached, variant = self._load_engine(lang, profile)
        # Exercise the first graph execution too. A blank, screen-line-shaped
        # buffer keeps this cheap and removes first-inference latency later.
        engine.max_side_len = PROFILE_LIMITS[profile][0]
        engine.min_side_len = 0
        engine.text_det.limit_side_len = PROFILE_LIMITS[profile][0]
        warm_start = time.perf_counter()
        with contextlib.redirect_stdout(sys.stderr):
            engine(np.full((48, 320, 3), 255, dtype=np.uint8))
        return {
            "ok": True,
            "command": "warmup",
            "cached": cached,
            "profile": profile,
            "detector_variant": variant,
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
        if lang not in {"auto", "mixed", "zh", "en"}:
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
        max_attempts = max(1, int(self.args.download_retries))
        load_start = time.perf_counter()
        print("KAIXIN_OCR_STAGE model-load", file=sys.stderr, flush=True)
        loader, loader_init, loader_cached, loader_variant = self._load_engine(
            "en" if lang == "en" else "zh", profile
        )
        with contextlib.redirect_stdout(sys.stderr):
            image_array = loader.load_img(image_bytes if image_bytes is not None else image)
        validate_image_shape(image_array)
        print("KAIXIN_OCR_STAGE image-loaded", file=sys.stderr, flush=True)
        load_seconds = time.perf_counter() - load_start
        applied_max_side, applied_min_side = adaptive_limits(profile, image_array.shape, max_side_len)

        def run_attempt(effective_lang: str, source_image: Any) -> tuple[Any, list[dict[str, Any]], float, float, bool, str]:
            last_error: Exception | None = None
            for attempt in range(1, max_attempts + 1):
                try:
                    print(
                        f"KAIXIN_OCR_STAGE inference lang={effective_lang} attempt={attempt}",
                        file=sys.stderr,
                        flush=True,
                    )
                    engine, init_seconds, cached, detector_variant = self._load_engine(effective_lang, profile)
                    engine.max_side_len = applied_max_side
                    engine.min_side_len = applied_min_side
                    engine.text_det.limit_side_len = applied_max_side
                    with contextlib.redirect_stdout(sys.stderr):
                        infer_start = time.perf_counter()
                        result = engine(source_image)
                        infer_seconds = time.perf_counter() - infer_start
                    print("KAIXIN_OCR_STAGE inference-complete", file=sys.stderr, flush=True)
                    return result, to_line_items(result, float(min_score)), init_seconds, infer_seconds, cached, detector_variant
                except Exception as exc:
                    last_error = exc
                    self._clear_engine(effective_lang, profile)
                    if attempt < max_attempts:
                        print(f"RapidOCR init/inference failed, retrying ({attempt}/{max_attempts})...", file=sys.stderr)
                        time.sleep(1.0)
            raise last_error or RuntimeError("RapidOCR inference failed")

        effective_lang = "zh" if lang in {"auto", "mixed", "zh"} else "en"
        result, lines, init_seconds, infer_seconds, cached, detector_variant = run_attempt(effective_lang, image_array)
        if init_seconds == 0.0:
            init_seconds, cached, detector_variant = loader_init, loader_cached, loader_variant
        language_retry = False
        if lang == "auto" and should_retry_english(lines):
            english_result, english_lines, english_init, english_infer, english_cached, english_variant = run_attempt("en", image_array)
            if recognition_quality(english_lines) > recognition_quality(lines):
                result, lines = english_result, english_lines
                effective_lang, init_seconds, infer_seconds = "en", english_init, english_infer
                cached, detector_variant = english_cached, english_variant
            language_retry = True

        preprocess_retry = False
        if should_retry_preprocess(lines):
            import cv2

            gray = cv2.cvtColor(image_array, cv2.COLOR_BGR2GRAY)
            enhanced = cv2.createCLAHE(clipLimit=2.0, tileGridSize=(8, 8)).apply(gray)
            if min(enhanced.shape[:2]) >= 31:
                prepared = cv2.adaptiveThreshold(
                    enhanced, 255, cv2.ADAPTIVE_THRESH_GAUSSIAN_C, cv2.THRESH_BINARY, 31, 9
                )
            else:
                _, prepared = cv2.threshold(enhanced, 0, 255, cv2.THRESH_BINARY + cv2.THRESH_OTSU)
            prepared = cv2.cvtColor(prepared, cv2.COLOR_GRAY2BGR)
            retry_result, retry_lines, retry_init, retry_infer, retry_cached, retry_variant = run_attempt(effective_lang, prepared)
            preprocess_retry = True
            if recognition_quality(retry_lines) > recognition_quality(lines):
                result, lines = retry_result, retry_lines
                init_seconds, infer_seconds = retry_init, retry_infer
                cached, detector_variant = retry_cached, retry_variant

        vis_status = None
        if vis_value:
            vis_path = Path(str(vis_value)).expanduser().resolve()
            vis_path.parent.mkdir(parents=True, exist_ok=True)
            with contextlib.redirect_stdout(sys.stderr):
                result.vis(str(vis_path))
            vis_status = str(vis_path)

        text = "\n".join(line["text"] for line in lines)
        return {
            "ok": True,
            "engine": "rapidocr",
            "backend": f"onnxruntime/{self.provider}",
            "profile": profile,
            "ocr_version": "PP-OCRv6",
            "model_type": "small-det+medium-rec" if detector_variant == "fast" else "medium",
            "detector_variant": detector_variant,
            "max_side_len": applied_max_side,
            "min_side_len": applied_min_side,
            "language": effective_lang if lang == "auto" else lang,
            "auto_language_retry": language_retry,
            "auto_preprocess_retry": preprocess_retry,
            "image": str(image),
            "rapidocr_root": str(self.rapidocr_root),
            "model_root": str(self.model_root),
            "models": model_files(self.model_root),
            "cached": cached,
            "init_ms": round(init_seconds * 1000, 2),
            "load_ms": round(load_seconds * 1000, 2),
            "infer_ms": round(infer_seconds * 1000, 2),
            "step_ms": elapsed_ms(result.elapse_list),
            "text": text,
            "lines": lines,
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
