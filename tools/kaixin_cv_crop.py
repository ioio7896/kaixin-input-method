#!/usr/bin/env python3
"""OpenCV image crop helper for Kaixin Input Method.

The helper prints exactly one JSON object to stdout. It is intentionally small:
Rust owns the screenshot workflow, while this script only detects a prominent
rectangular in-app border and writes the cropped image when a confident match is
found.
"""

from __future__ import annotations

import argparse
import json
import sys
import traceback
from pathlib import Path
from typing import Any

if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8")
if hasattr(sys.stderr, "reconfigure"):
    sys.stderr.reconfigure(encoding="utf-8")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Crop an image to the strongest rectangular border.")
    parser.add_argument("input", type=Path, help="Input image path.")
    parser.add_argument("output", type=Path, help="Output image path written only when cropped.")
    parser.add_argument("--padding", type=int, default=2, help="Pixels to keep around the detected border.")
    parser.add_argument("--min-area", type=float, default=0.04, help="Minimum rectangle area ratio.")
    parser.add_argument("--max-area", type=float, default=0.985, help="Maximum rectangle area ratio.")
    parser.add_argument("--pretty", action="store_true", help="Pretty-print JSON.")
    return parser.parse_args()


def emit(payload: dict[str, Any], pretty: bool) -> int:
    print(json.dumps(payload, ensure_ascii=False, indent=2 if pretty else None))
    return 0 if payload.get("ok") else 1


def read_image(path: Path):
    import cv2
    import numpy as np

    data = np.frombuffer(path.read_bytes(), dtype=np.uint8)
    image = cv2.imdecode(data, cv2.IMREAD_COLOR)
    if image is None:
        raise ValueError(f"OpenCV could not decode image: {path}")
    return image


def write_image(path: Path, image) -> None:
    import cv2

    suffix = path.suffix.lower()
    ext = ".jpg" if suffix in (".jpg", ".jpeg") else ".png"
    ok, encoded = cv2.imencode(ext, image)
    if not ok:
        raise ValueError(f"OpenCV could not encode image: {path}")
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(encoded.tobytes())


def border_density(edges, x: int, y: int, width: int, height: int) -> float:
    import cv2
    import numpy as np

    strip = max(2, min(6, min(width, height) // 80))
    roi = edges[y : y + height, x : x + width]
    if roi.size == 0:
        return 0.0
    parts = [
        roi[:strip, :],
        roi[-strip:, :],
        roi[:, :strip],
        roi[:, -strip:],
    ]
    pixels = sum(part.size for part in parts)
    if pixels == 0:
        return 0.0
    hits = sum(int(cv2.countNonZero(part)) for part in parts)
    return hits / float(pixels)


def find_rect(image, min_area: float, max_area: float):
    import cv2

    height, width = image.shape[:2]
    if width < 48 or height < 48:
        return None

    gray = cv2.cvtColor(image, cv2.COLOR_BGR2GRAY)
    gray = cv2.GaussianBlur(gray, (3, 3), 0)
    edges = cv2.Canny(gray, 48, 144)
    kernel = cv2.getStructuringElement(cv2.MORPH_RECT, (3, 3))
    edges = cv2.dilate(edges, kernel, iterations=1)
    closed = cv2.morphologyEx(edges, cv2.MORPH_CLOSE, kernel, iterations=2)
    contours, _ = cv2.findContours(closed, cv2.RETR_EXTERNAL, cv2.CHAIN_APPROX_SIMPLE)

    image_area = float(width * height)
    best: tuple[float, tuple[int, int, int, int], float] | None = None
    for contour in contours:
        x, y, rect_width, rect_height = cv2.boundingRect(contour)
        if rect_width < 48 or rect_height < 48:
            continue
        area_ratio = (rect_width * rect_height) / image_area
        if area_ratio < min_area or area_ratio > max_area:
            continue
        if x <= 2 and y <= 2 and x + rect_width >= width - 2 and y + rect_height >= height - 2:
            continue

        contour_area = max(float(cv2.contourArea(contour)), 1.0)
        fill_ratio = min(1.0, contour_area / float(rect_width * rect_height))
        perimeter = cv2.arcLength(contour, True)
        approx = cv2.approxPolyDP(contour, 0.025 * perimeter, True)
        corner_bonus = 1.15 if len(approx) == 4 else 1.0
        density = border_density(edges, x, y, rect_width, rect_height)
        if density < 0.025 and fill_ratio < 0.35:
            continue

        margin_penalty = 0.9 if min(x, y, width - (x + rect_width), height - (y + rect_height)) <= 2 else 1.0
        score = area_ratio * (0.55 + density * 3.0) * (0.55 + fill_ratio) * corner_bonus * margin_penalty
        if best is None or score > best[0]:
            best = (score, (x, y, rect_width, rect_height), density)

    return best


def main() -> int:
    args = parse_args()
    try:
        source = args.input.expanduser().resolve()
        output = args.output.expanduser().resolve()
        if not source.is_file():
            return emit({"ok": False, "error": f"image not found: {source}"}, args.pretty)

        image = read_image(source)
        height, width = image.shape[:2]
        found = find_rect(image, args.min_area, args.max_area)
        if found is None:
            return emit(
                {"ok": True, "cropped": False, "reason": "no-confident-border", "width": width, "height": height},
                args.pretty,
            )

        score, (x, y, rect_width, rect_height), density = found
        padding = max(0, args.padding)
        x0 = max(0, x - padding)
        y0 = max(0, y - padding)
        x1 = min(width, x + rect_width + padding)
        y1 = min(height, y + rect_height + padding)
        if (x1 - x0) >= width - 3 and (y1 - y0) >= height - 3:
            return emit(
                {"ok": True, "cropped": False, "reason": "border-is-full-image", "width": width, "height": height},
                args.pretty,
            )

        cropped = image[y0:y1, x0:x1]
        write_image(output, cropped)
        return emit(
            {
                "ok": True,
                "cropped": True,
                "input": str(source),
                "output": str(output),
                "rect": [x0, y0, x1 - x0, y1 - y0],
                "width": width,
                "height": height,
                "confidence": round(float(score), 6),
                "border_density": round(float(density), 6),
            },
            args.pretty,
        )
    except Exception as exc:
        return emit(
            {
                "ok": False,
                "engine": "opencv",
                "error": str(exc),
                "traceback": traceback.format_exc(),
            },
            args.pretty,
        )


if __name__ == "__main__":
    raise SystemExit(main())
