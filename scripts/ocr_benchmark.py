# SPDX-License-Identifier: Apache-2.0
"""Compare the production OCR worker on reproducible, labelled synthetic images.

Does not capture the user's screen. Results are a regression set, not a claim
about accuracy on real screenshots. Use --manifest to evaluate curated images.
"""
from __future__ import annotations

import argparse
import ctypes
import hashlib
import json
import math
import os
import queue
import statistics
import subprocess
import sys
import threading
import time
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[1]


def distance(a: str, b: str) -> int:
    row = list(range(len(b) + 1))
    for i, x in enumerate(a, 1):
        new = [i]
        for j, y in enumerate(b, 1):
            new.append(min(new[-1] + 1, row[j] + 1, row[j - 1] + (x != y)))
        row = new
    return row[-1]


def normalized(text: str) -> str:
    return "".join(text.split())


def percentile(values: list[float], p: float) -> float:
    return sorted(values)[max(0, math.ceil(len(values) * p) - 1)] if values else 0.0


def memory_bytes(pid: int) -> dict[str, int]:
    if os.name != "nt":
        return {}
    from ctypes import wintypes

    class Counters(ctypes.Structure):
        _fields_ = [("cb", wintypes.DWORD), ("PageFaultCount", wintypes.DWORD)] + [
            (name, ctypes.c_size_t) for name in ("PeakWorkingSetSize", "WorkingSetSize",
                "QuotaPeakPagedPoolUsage", "QuotaPagedPoolUsage", "QuotaPeakNonPagedPoolUsage",
                "QuotaNonPagedPoolUsage", "PagefileUsage", "PeakPagefileUsage")]

    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
    kernel.OpenProcess.restype = wintypes.HANDLE
    kernel.CloseHandle.argtypes = [wintypes.HANDLE]
    # A Windows venv python.exe can be a 3 MiB launcher whose child does the
    # actual inference. Follow this owned process tree before measuring.
    class ProcessEntry(ctypes.Structure):
        _fields_ = [("dwSize", wintypes.DWORD), ("cntUsage", wintypes.DWORD),
                    ("th32ProcessID", wintypes.DWORD), ("th32DefaultHeapID", ctypes.c_size_t),
                    ("th32ModuleID", wintypes.DWORD), ("cntThreads", wintypes.DWORD),
                    ("th32ParentProcessID", wintypes.DWORD), ("pcPriClassBase", wintypes.LONG),
                    ("dwFlags", wintypes.DWORD), ("szExeFile", wintypes.WCHAR * 260)]
    kernel.CreateToolhelp32Snapshot.argtypes = [wintypes.DWORD, wintypes.DWORD]
    kernel.CreateToolhelp32Snapshot.restype = wintypes.HANDLE
    for name in ("Process32FirstW", "Process32NextW"):
        getattr(kernel, name).argtypes = [wintypes.HANDLE, ctypes.POINTER(ProcessEntry)]
    snapshot = kernel.CreateToolhelp32Snapshot(2, 0)
    owned = {pid}
    if snapshot and snapshot != ctypes.c_void_p(-1).value:
        try:
            entry = ProcessEntry()
            entry.dwSize = ctypes.sizeof(entry)
            relationships = []
            more = kernel.Process32FirstW(snapshot, ctypes.byref(entry))
            while more:
                relationships.append((entry.th32ProcessID, entry.th32ParentProcessID))
                more = kernel.Process32NextW(snapshot, ctypes.byref(entry))
            while True:
                expanded = owned | {child for child, parent in relationships if parent in owned}
                if expanded == owned:
                    break
                owned = expanded
        finally:
            kernel.CloseHandle(snapshot)
    psapi = ctypes.WinDLL("psapi", use_last_error=True)
    psapi.GetProcessMemoryInfo.argtypes = [wintypes.HANDLE, ctypes.POINTER(Counters), wintypes.DWORD]
    measured = []
    for process_id in owned:
        handle = kernel.OpenProcess(0x0400 | 0x0010, False, process_id)
        if not handle:
            continue
        try:
            counters = Counters()
            counters.cb = ctypes.sizeof(counters)
            if psapi.GetProcessMemoryInfo(handle, ctypes.byref(counters), counters.cb):
                measured.append({"pid": process_id, "rss": counters.WorkingSetSize,
                                 "peak_rss": counters.PeakWorkingSetSize})
        finally:
            kernel.CloseHandle(handle)
    return max(measured, key=lambda value: value["peak_rss"]) if measured else {}


def make_fixtures(directory: Path) -> list[dict[str, Any]]:
    from PIL import Image, ImageDraw, ImageFont

    directory.mkdir(parents=True, exist_ok=True)
    font_path = Path(os.environ.get("WINDIR", "C:/Windows")) / "Fonts/msyh.ttc"
    if not font_path.exists():
        raise RuntimeError("Synthetic Chinese fixtures need Microsoft YaHei; use --manifest on other hosts.")
    cases = []

    def save(name: str, size: tuple[int, int], lines: list[tuple[int, int, str]],
             font_size: int, background: Any = "white", foreground: Any = "black") -> None:
        picture = Image.new("RGB", size, background)
        draw = ImageDraw.Draw(picture)
        font = ImageFont.truetype(str(font_path), font_size)
        boxes = []
        for x, y, text in lines:
            draw.text((x, y), text, font=font, fill=foreground)
            boxes.append(list(draw.textbbox((x, y), text, font=font)))
        filename = directory / f"{name}.png"
        picture.save(filename)
        cases.append({"id": name, "image": str(filename.resolve()), "text": "\n".join(t for _, _, t in lines),
                      "lines": [{"text": line[2], "box": box} for line, box in zip(lines, boxes)]})

    save("normal", (1000, 220), [(24, 20, "开心输入法，截图识别文字。"),
         (24, 65, "Hello Windows 2026"), (24, 110, "订单编号 ABC-12345，金额 128.50 元")], 26)
    save("single", (160, 100), [(45, 22, "好")], 38)
    save("dark", (1400, 320), [(24, 15, "深色界面也能识别中文和英文"),
         (24, 70, "Build completed: 2026-11-11"), (24, 120, "文件路径 D:/Documents/report.txt")], 18, (28, 28, 32), (220, 220, 224))
    save("low-contrast", (1100, 260), [(24, 25, "浅色文字测试：请检查识别结果"),
         (24, 65, "Invoice 2026 Total 128.50")], 18, (220, 220, 220), (178, 178, 178))
    save("tiny-crop", (520, 64), [(8, 8, "小字识别测试 ABC 123456")], 13)
    dense = [(30 + col * 1280, 30 + row * 380,
              f"第{row * 3 + col + 1:02d}行 截图小字测试 ABC 12345")
             for row in range(5) for col in range(3)]
    save("4k-small", (3840, 2160), dense, 14)
    save("4k-normal", (3840, 2160), [(48, 48, "四千像素截图也要保持完整文字"),
         (1250, 450, "Windows screenshot OCR 2026"), (2600, 1200, "识别结果与原图位置一致")], 30)
    save("seams", (3840, 2160), [(1180, 990, "跨越分块接缝的文字不能重复或遗漏 ABC 12345"),
         (2480, 1080, "第二处接缝保留完整句子 Hello World")], 22)
    save("long", (800, 5000), [(20, 80 + i * 600, f"第{i + 1}段 长截图需要按顺序识别 ABC 2026")
                               for i in range(8)], 22)
    save("blank", (800, 500), [], 24)
    (directory / "manifest.json").write_text(json.dumps(cases, ensure_ascii=False, indent=2), encoding="utf-8")
    return cases


class Worker:
    def __init__(self, python: Path, helper: Path, root: Path, log: Path,
                 threads: int, batch: int, fast_mode: bool | None) -> None:
        command = [str(python), "-B", str(helper), "--stdio", "--rapidocr-root", str(root),
                   "--provider", "cpu", "--intra-op-threads", str(threads), "--rec-batch-num", str(batch)]
        if fast_mode is not None:
            command.append("--fast-mode" if fast_mode else "--no-fast-mode")
        self.log = log.open("w", encoding="utf-8")
        env = dict(os.environ, PYTHONUTF8="1", PYTHONDONTWRITEBYTECODE="1", PPOCR_BENCH_THREADS=str(threads))
        env.pop("PPOCR_REC_FAST", None)
        self.process = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
            stderr=self.log, text=True, encoding="utf-8", cwd=ROOT, env=env,
            creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
        self.responses: queue.Queue[str] = queue.Queue()
        threading.Thread(target=self._read, daemon=True).start()

    def _read(self) -> None:
        assert self.process.stdout is not None
        for line in self.process.stdout:
            self.responses.put(line)
        self.responses.put("")

    def request(self, request: dict[str, Any]) -> tuple[dict[str, Any], float]:
        start = time.perf_counter()
        assert self.process.stdin is not None
        self.process.stdin.write(json.dumps(request, ensure_ascii=False) + "\n")
        self.process.stdin.flush()
        try:
            line = self.responses.get(timeout=140)
        except queue.Empty as exc:
            raise TimeoutError("OCR worker exceeded 140 seconds") from exc
        if not line:
            raise RuntimeError(f"OCR worker exited: {self.process.poll()}")
        payload = json.loads(line)
        if not payload.get("ok"):
            raise RuntimeError(payload.get("error", "OCR failed"))
        return payload, (time.perf_counter() - start) * 1000

    def close(self) -> None:
        if self.process.poll() is None:
            try:
                assert self.process.stdin is not None
                self.process.stdin.write('{"command":"quit"}\n')
                self.process.stdin.flush()
                self.process.wait(timeout=5)
            except (OSError, subprocess.TimeoutExpired):
                self.process.kill()
                self.process.wait(timeout=5)
        for stream in (self.process.stdin, self.process.stdout):
            if stream:
                stream.close()
        self.log.close()


def evaluate(cases: list[dict[str, Any]], worker: Worker, profile: str, repeats: int) -> dict[str, Any]:
    warmup, warmup_ms = worker.request({"command": "warmup", "profile": profile, "lang": "auto"})
    results, times = [], []
    error_sum = char_sum = hit_sum = line_sum = 0
    for case in cases:
        best = None
        for repeat in range(repeats):
            result, elapsed = worker.request({"image": case["image"], "profile": profile, "lang": "auto", "min_score": 0.5})
            times.append(elapsed)
            results.append({"case": case["id"], "repeat": repeat, "wall_ms": round(elapsed, 2),
                            "memory": memory_bytes(worker.process.pid), "result": result})
            best = result
        assert best is not None
        expected, actual = normalized(case["text"]), normalized(best["text"])
        error_sum += distance(expected, actual)
        char_sum += len(expected)
        # Exact labelled-line content coverage, not detection recall (which
        # would require polygon matching and a separate detector-only metric).
        for line in case["lines"]:
            hit_sum += normalized(line["text"]) in actual
            line_sum += 1
        print(f"{profile} {case['id']}: {results[-1]['wall_ms']} ms", flush=True)
    return {"warmup": warmup, "cold_worker_to_warm_ms": round(warmup_ms, 2),
            "cer": error_sum / max(1, char_sum), "exact_line_coverage": hit_sum / max(1, line_sum),
            "characters": char_sum, "labelled_lines": line_sum, "runs": len(times),
            "p50_ms": statistics.median(times), "p95_ms": percentile(times, .95),
            "peak_rss_bytes": max((x["memory"].get("peak_rss", 0) for x in results), default=0), "results": results}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=ROOT / "dist/ocr-optimization/evaluation")
    parser.add_argument("--manifest", type=Path)
    parser.add_argument("--baseline", type=Path)
    parser.add_argument("--helper", type=Path, default=ROOT / "tools/kaixin_ocr_engine.py")
    parser.add_argument("--rapidocr-root", type=Path, default=ROOT / "RapidOCRLite")
    parser.add_argument("--python", type=Path, default=Path(sys.executable))
    parser.add_argument("--repeats", type=int, default=2)
    parser.add_argument("--profiles", default="fast,balanced,accurate")
    parser.add_argument("--tune", action="store_true", help="Also compare 2/4 threads, 4/8 batch and fast mode.")
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    cases = json.loads(args.manifest.read_text(encoding="utf-8")) if args.manifest else make_fixtures(args.output / "images")
    if args.manifest:
        for case in cases:
            path = Path(case["image"])
            case["image"] = str(path if path.is_absolute() else args.manifest.parent / path)
    configs = []
    if args.baseline:
        configs.append(("baseline-balanced", args.baseline, "balanced", 4, 8, None))
    configs += [(f"current-{p}", args.helper, p, 4, 8, False) for p in args.profiles.split(",")]
    if args.tune:
        configs += [(f"balanced-t{t}-b{b}", args.helper, "balanced", t, b, False)
                    for t, b in ((2, 4), (2, 8), (4, 4))]
        configs.append(("fast-width-optimization", args.helper, "fast", 4, 8, True))
    report = {"scope": "Synthetic regression fixtures; not real-world accuracy or a representative latency distribution.",
              "version": (ROOT / "VERSION").read_text().strip(), "python": sys.version,
              "fixtures": cases, "configs": {}}
    for name, helper, profile, threads, batch, fast_mode in configs:
        source_hashes = {helper.name: hashlib.sha256(helper.read_bytes()).hexdigest()}
        policy = helper.with_name("kaixin_ocr_policy.py")
        if policy.is_file():
            source_hashes[policy.name] = hashlib.sha256(policy.read_bytes()).hexdigest()
        worker = Worker(args.python, helper.resolve(), args.rapidocr_root.resolve(), args.output / f"{name}.log", threads, batch, fast_mode)
        try:
            result = evaluate(cases, worker, profile, max(1, args.repeats))
            result.update({"source_sha256": source_hashes,
                           "profile": profile, "threads": threads, "batch": batch, "fast_mode": fast_mode})
            report["configs"][name] = result
        finally:
            worker.close()
        (args.output / "report.json").write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")
    rows = ["# OCR 对照评测", "", "合成回归样本；不代表真实截图准确率。P95 为此样本集耗时分位数。",
            "行覆盖率指整行文本精确命中，不等同于检测召回率。", "",
            "| 配置 | CER | 整行覆盖率 | P50 ms | P95 ms | 峰值内存 MiB |", "| --- | ---: | ---: | ---: | ---: | ---: |"]
    for name, value in report["configs"].items():
        rows.append(f"| {name} | {value['cer']:.2%} | {value['exact_line_coverage']:.2%} | {value['p50_ms']:.1f} | {value['p95_ms']:.1f} | {value['peak_rss_bytes'] / 1048576:.1f} |")
    (args.output / "report.md").write_text("\n".join(rows) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
