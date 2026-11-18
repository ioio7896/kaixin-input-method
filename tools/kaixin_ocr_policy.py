# SPDX-License-Identifier: Apache-2.0
"""Bounded image planning for the screenshot OCR worker.

No model is loaded here. Geometry/budget rules also run in source-only CI.
Image analysis uses a downsampled copy; all returned rectangles use source pixels.
"""

from __future__ import annotations

import math
import time
from dataclasses import dataclass, field
from typing import Any

PROFILE_LIMITS = {"fast": (1280, 1.0), "balanced": (1920, 1.5), "accurate": (2560, 2.0)}
PROFILE_MODELS = {
    "fast": ("small", "small"),
    "balanced": ("small", "medium"),
    "accurate": ("medium", "medium"),
}
PROFILE_SECONDS = {"fast": 30, "balanced": 60, "accurate": 90}
MAX_TILES = 32
MAX_WORK_PIXELS = 64_000_000
MAX_REFINEMENT_PIXELS = 8_000_000
MAX_REFINEMENTS = 8
Rect = tuple[int, int, int, int]


def normalize_profile(value: Any) -> str:
    profile = str(value or "balanced").strip().lower()
    return profile if profile in PROFILE_LIMITS else "balanced"


def image_tiles(shape: Any, limit: int) -> list[Rect]:
    """Cover the *whole* image, including ordinary 16:9 4K captures.

    Partition into equal cores plus shared margins, avoiding almost-full-image
    duplicate tiles just above a boundary. Never truncate an over-budget image.
    """
    height, width = map(int, shape[:2])
    limit = max(320, min(int(limit), 1920))
    overlap = min(192, max(128, limit // 10))

    def segments(length: int) -> list[tuple[int, int]]:
        if length <= limit:
            return [(0, length)]
        count = math.ceil(length / (limit - overlap))
        return [
            (
                max(0, round(length * i / count) - overlap // 2),
                min(length, round(length * (i + 1) / count) + overlap // 2),
            )
            for i in range(count)
        ]

    tiles = [(x0, y0, x1, y1) for y0, y1 in segments(height) for x0, x1 in segments(width)]
    if len(tiles) > MAX_TILES or sum(rect_area(r) for r in tiles) > MAX_WORK_PIXELS:
        raise ValueError("截图分块超出处理预算，请缩小选区或分次识别。")
    return tiles


def rect_area(rect: Rect) -> int:
    x0, y0, x1, y1 = rect
    return max(0, x1 - x0) * max(0, y1 - y0)


def line_bounds(line: dict[str, Any]) -> Rect | None:
    box = line.get("box")
    if not box:
        return None
    return (
        math.floor(min(p[0] for p in box)),
        math.floor(min(p[1] for p in box)),
        math.ceil(max(p[0] for p in box)),
        math.ceil(max(p[1] for p in box)),
    )


def intersection(a: Rect, b: Rect) -> int:
    return max(0, min(a[2], b[2]) - max(a[0], b[0])) * max(0, min(a[3], b[3]) - max(a[1], b[1]))


@dataclass
class ImageAnalysis:
    components: int = 0
    small_components: int = 0
    contrast: float = 0.0
    regions: list[Rect] = field(default_factory=list)
    small_regions: list[Rect] = field(default_factory=list)

    @property
    def low_contrast(self) -> bool:
        return self.components >= 2 and 3.0 < self.contrast < 65.0

    def summary(self) -> dict[str, Any]:
        return {
            "components": self.components,
            "small_components": self.small_components,
            "contrast": round(self.contrast, 2),
            "low_contrast": self.low_contrast,
            "regions": len(self.regions),
            "small_regions": len(self.small_regions),
        }


def analyze_image(image: Any) -> ImageAnalysis:
    """Find text-like component groups, without treating confidence as truth.

    The gradient detects both dark-on-light and light-on-dark text. Component
    shape/density filters exclude large panels and thin rules. This is a hint
    for optional passes, never a reason to drop primary OCR results.
    """
    import cv2
    import numpy as np

    height, width = image.shape[:2]
    scale = min(1.0, 2048.0 / max(width, height))
    gray = cv2.cvtColor(image, cv2.COLOR_BGR2GRAY)
    if scale < 1:
        gray = cv2.resize(
            gray,
            (max(1, round(width * scale)), max(1, round(height * scale))),
            interpolation=cv2.INTER_AREA,
        )
    gradient = cv2.morphologyEx(gray, cv2.MORPH_GRADIENT, np.ones((3, 3), np.uint8))
    if int(gradient.max()) < 4:
        return ImageAnalysis()
    _, mask = cv2.threshold(gradient, 0, 255, cv2.THRESH_BINARY + cv2.THRESH_OTSU)
    _, labels, stats, _ = cv2.connectedComponentsWithStats(mask, 8)
    glyphs, small = [], []
    text_mask = np.zeros_like(mask)
    small_mask = np.zeros_like(mask)
    strengths = []
    for x, y, w, h, area in stats[1:]:
        # The gradient adds roughly two pixels to character bounds.
        source_h = max(1.0, (int(h) - 2) / scale)
        if not (
            3 <= h
            and 5 <= source_h <= 64
            and 2 <= w <= h * 5
            and area >= 5
            and 0.12 <= area / (w * h) <= 1.0
        ):
            continue
        glyphs.append((x, y, w, h))
        text_mask[y : y + h, x : x + w] = 255
        strengths.append(float(gradient[y : y + h, x : x + w].max()))
        if source_h <= 19:
            small.append((x, y, w, h))
            small_mask[y : y + h, x : x + w] = 255
    del labels

    def groups(source: Any) -> list[Rect]:
        joined = cv2.dilate(source, np.ones((3, max(9, round(22 * scale))), np.uint8))
        _, _, regions, _ = cv2.connectedComponentsWithStats(joined, 8)
        output = []
        for x, y, w, h, area in regions[1:]:
            if w / scale < 24 or h / scale > 192 or area < 18:
                continue
            output.append(
                (
                    max(0, math.floor(x / scale) - 8),
                    max(0, math.floor(y / scale) - 6),
                    min(width, math.ceil((x + w) / scale) + 8),
                    min(height, math.ceil((y + h) / scale) + 6),
                )
            )
        return sorted(output, key=lambda r: (r[1], r[0]))

    return ImageAnalysis(
        len(glyphs),
        len(small),
        float(np.median(strengths)) if strengths else 0.0,
        groups(text_mask),
        groups(small_mask) if len(small) >= 3 else [],
    )


def lines_in_region(lines: list[dict[str, Any]], region: Rect) -> list[dict[str, Any]]:
    return [
        line
        for line in lines
        if (bounds := line_bounds(line)) is not None
        and intersection(bounds, region) >= min(rect_area(bounds), rect_area(region)) * 0.5
    ]


def needs_detector_retry(lines: list[dict[str, Any]], analysis: ImageAnalysis) -> bool:
    if analysis.components < 3 or not analysis.regions:
        return False
    missed = sum(not lines_in_region(lines, region) for region in analysis.regions)
    return missed >= max(1, math.ceil(len(analysis.regions) * 0.4))


def should_retry_preprocess(lines: list[dict[str, Any]], analysis: ImageAnalysis) -> bool:
    # A correctly recognized single character is a successful result.
    if not analysis.low_contrast:
        return False
    return not lines or sum(float(line.get("score", 0)) for line in lines) / len(lines) < 0.82


def refinement_regions(lines: list[dict[str, Any]], analysis: ImageAnalysis) -> list[Rect]:
    candidates = []
    for region in analysis.small_regions:
        covered = lines_in_region(lines, region)
        confidence = (
            sum(float(line.get("score", 0)) for line in covered) / len(covered) if covered else 0.0
        )
        if confidence < 0.94:
            candidates.append((confidence, region))
    # Missing/uncertain regions first, stable reading order on ties.
    return [
        region
        for _, region in sorted(candidates, key=lambda value: (value[0], value[1][1], value[1][0]))
    ][:MAX_REFINEMENTS]


class WorkBudget:
    """Bound work between native calls; the Rust parent enforces a hard timeout."""

    def __init__(self, seconds: float, clock: Any = time.perf_counter) -> None:
        self.clock = clock
        self.deadline = clock() + seconds
        self.pixels = 0
        self.extra_pixels = 0
        self.extra_calls = 0
        self.skipped: list[str] = []

    def reserve(self, pixels: int, kind: str, optional: bool = False) -> bool:
        reason = None
        if self.clock() >= self.deadline:
            reason = "time"
        elif self.pixels + pixels > MAX_WORK_PIXELS:
            reason = "pixels"
        elif optional and (
            self.extra_calls >= MAX_REFINEMENTS
            or self.extra_pixels + pixels > MAX_REFINEMENT_PIXELS
        ):
            reason = "refinement"
        if reason:
            if not optional:
                raise TimeoutError("OCR 处理超出时间或像素预算，请缩小选区后重试。")
            self.skipped.append(f"{kind}:{reason}")
            return False
        self.pixels += pixels
        if optional:
            self.extra_pixels += pixels
            self.extra_calls += 1
        return True
