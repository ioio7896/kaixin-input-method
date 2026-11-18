"""Record reproducible speed + candidate quality against the staged lexicon."""

from __future__ import annotations

import argparse
import ctypes
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import platform
import secrets
import sqlite3
import statistics
import subprocess
import tempfile
import time

if __package__:
    from .build_input_cases_sqlite import read_rows
else:
    from build_input_cases_sqlite import read_rows


def percentiles(values: list[float]) -> dict:
    if not values:
        raise ValueError("No cold-start samples")
    ordered = sorted(values)

    def percentile(p: float) -> float:
        return ordered[min(int((len(ordered) - 1) * p + 0.5), len(ordered) - 1)]

    return {
        "samples": len(values),
        "average_us": statistics.mean(values),
        "p50_us": percentile(0.5),
        "p95_us": percentile(0.95),
        "p99_us": percentile(0.99),
        "max_us": max(values),
    }


def prepare_profile(profile: Path) -> None:
    """Create the DPAPI capability normally provisioned by the TSF client.

    A fresh benchmark profile has no TSF host. Do not read/copy the installed
    user's token, and keep the production authentication/DPAPI cost in lookup.
    """

    class Blob(ctypes.Structure):
        _fields_ = [("size", ctypes.c_uint32), ("data", ctypes.c_void_p)]

    crypt = ctypes.WinDLL("crypt32", use_last_error=True)
    protect = crypt.CryptProtectData
    protect.argtypes = [
        ctypes.POINTER(Blob),
        ctypes.c_void_p,
        ctypes.c_void_p,
        ctypes.c_void_p,
        ctypes.c_void_p,
        ctypes.c_uint32,
        ctypes.POINTER(Blob),
    ]
    protect.restype = ctypes.c_int
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    free = kernel.LocalFree
    free.argtypes, free.restype = [ctypes.c_void_p], ctypes.c_void_p
    token = secrets.token_hex(32).encode("ascii")
    plain = ctypes.create_string_buffer(token)
    incoming = Blob(len(token), ctypes.cast(plain, ctypes.c_void_p))
    encrypted = Blob()
    if not protect(ctypes.byref(incoming), None, None, None, None, 1, ctypes.byref(encrypted)):
        raise ctypes.WinError(ctypes.get_last_error())
    try:
        data = profile / "kaixin"
        data.mkdir()
        (data / "engine_capability.dat").write_bytes(
            b"KXIPC-DPAPI-1\n" + ctypes.string_at(encrypted.data, encrypted.size)
        )
    finally:
        free(encrypted.data)


def generate_report(repo: Path, binaries: Path, lexicon: Path, output: Path) -> dict:
    output.mkdir(parents=True, exist_ok=True)
    source = repo / "tests/category_ranking_cases.tsv"
    cases = read_rows(source)
    database = output / "cases.sqlite"
    with sqlite3.connect(database) as connection:
        connection.execute("DROP TABLE IF EXISTS cases")
        connection.execute(
            "CREATE TABLE cases(sort_order INTEGER, case_id TEXT PRIMARY KEY, category TEXT, input TEXT, expected TEXT)"
        )
        connection.executemany(
            "INSERT INTO cases VALUES (?, ?, ?, ?, ?)",
            ((index, *case) for index, case in enumerate(cases)),
        )
    timings = {}
    memory = {}
    cold_samples: dict[str, list[float]] = {}
    process_wall = []
    with tempfile.TemporaryDirectory(prefix="kaixin-benchmark-") as temporary:
        base_env = {k: v for k, v in os.environ.items() if not k.upper().startswith("SRF_")}

        def run(name: str, arguments: list[str]) -> float:
            profile = Path(temporary) / name
            profile.mkdir()
            prepare_profile(profile)
            env = dict(
                base_env,
                LOCALAPPDATA=str(profile),
                APPDATA=str(profile),
                SRF_USER_DICT_PATH=str(profile / "user.sqlite"),
                SRF_IME_LOG_LEVEL="error",
            )
            start = time.perf_counter()
            with (output / f"{name}.log").open("w", encoding="utf-8") as log:
                # The production service accepts the explicit development
                # lexicon under cwd; point it at the actual staged package.
                result = subprocess.run(
                    arguments,
                    cwd=lexicon.parent,
                    env=env,
                    stdout=log,
                    stderr=subprocess.STDOUT,
                    timeout=180,
                    creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0),
                )
            if result.returncode:
                raise RuntimeError(f"Benchmark {name} failed; see {output / (name + '.log')}")
            return (time.perf_counter() - start) * 1_000_000

        perf = str(binaries / "input_perf.exe")
        common = ["--lexicon", str(lexicon), "--engine-profile", "runtime-hot-cold"]
        for trial in range(5):
            name = f"cold-{trial + 1}"
            metrics = output / f"{name}.jsonl"
            process_wall.append(
                run(name, [perf, *common, "--mode", "cold-start", "--json", str(metrics)])
            )
            for metric in map(json.loads, metrics.read_text(encoding="utf-8").splitlines()):
                if metric["metric"] == "memory":
                    memory[name] = metric
                else:
                    cold_samples.setdefault(metric["metric"], []).append(metric["average_us"])
        timings.update({name: percentiles(values) for name, values in cold_samples.items()})
        timings["cold_process_wall"] = percentiles(process_wall)
        for name, arguments in (
            ("warm", ["--mode", "plain", "--warmup", "2"]),
            ("incremental", ["--mode", "plain", "--incremental", "--warmup", "0"]),
            ("pipe", ["--mode", "e2e-pipe", "--threads", "1", "--warmup", "2"]),
        ):
            metrics = output / f"{name}.jsonl"
            run(
                name,
                [
                    perf,
                    *common,
                    "--cases",
                    str(database),
                    "--workload-size",
                    "300",
                    "--iterations",
                    "3",
                    "--slow",
                    "10",
                    *arguments,
                    "--json",
                    str(metrics),
                ],
            )
            for metric in map(json.loads, metrics.read_text(encoding="utf-8").splitlines()):
                if metric["metric"] == "memory":
                    memory[name] = metric
                else:
                    if metric["samples"] != 900:
                        raise RuntimeError(
                            f"{name} measured {metric['samples']} samples instead of the fixed 900; rebuild input_perf"
                        )
                    timings[name + "." + metric["metric"]] = metric
        quality_path = output / "quality.json"
        run(
            "quality",
            [
                str(binaries / "input_eval.exe"),
                *common,
                "--cases",
                str(database),
                "--json",
                str(quality_path),
            ],
        )
    quality = json.loads(quality_path.read_text(encoding="utf-8"))
    report = {
        "schema_version": 1,
        "version": (repo / "VERSION").read_text().strip(),
        "created_utc": datetime.now(timezone.utc).isoformat(),
        "platform": platform.platform(),
        "cpu": platform.processor(),
        "toolchain": json.loads((repo / "release-toolchain.json").read_text()),
        "profile": "runtime-hot-cold",
        "cases_sha256": hashlib.sha256(source.read_bytes()).hexdigest(),
        "lexicon_sha256": {
            path.name: hashlib.sha256(path.read_bytes()).hexdigest()
            for path in sorted(lexicon.glob("*"))
            if path.is_file()
        },
        "measurement_scope": "Fresh-process cold start (OS disk cache retained); first candidates through real IPC; no TSF candidate-window paint measurement",
        "timings": timings,
        "memory": memory,
        "quality": quality,
    }
    (output / "report.json").write_text(
        json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
    )
    lines = [
        f"# 输入评测 {report['version']}",
        "",
        "运行配置：安装包暂存词库、隔离用户数据、runtime-hot-cold。冷启动为 5 次新进程，保留操作系统磁盘缓存；首批候选为 IPC 返回时间，不包含 TSF 候选窗绘制。",
        "",
        "| 指标 | 样本 | P50 (ms) | P95 (ms) | P99 (ms) |",
        "|---|---:|---:|---:|---:|",
    ]
    for name, metric in timings.items():
        lines.append(
            f"| {name} | {metric['samples']} | {metric['p50_us'] / 1000:.3f} | {metric['p95_us'] / 1000:.3f} | {metric['p99_us'] / 1000:.3f} |"
        )
    lines += ["", "| 分组 | 样例 | Top1 | Top3 |", "|---|---:|---:|---:|"]
    for name, stats in quality["categories"].items():
        lines.append(
            f"| {name} | {stats['total']} | {stats['top1_hits'] / stats['total']:.1%} | {stats['top3_hits'] / stats['total']:.1%} |"
        )
    lines += [
        "",
        f"总体：Top1 {quality['top1_pct']:.1f}%，Top3 {quality['top3_pct']:.1f}%。",
        "",
        "| 场景 | 工作集 (MiB) | 峰值工作集 (MiB) |",
        "|---|---:|---:|",
    ]
    for name, metric in memory.items():
        lines.append(
            f"| {name} | {metric['working_set_bytes'] / 1048576:.1f} | {metric['peak_working_set_bytes'] / 1048576:.1f} |"
        )
    lines += [
        "",
        "未命中样例详见 quality.json；各场景日志保留最慢输入。该小型人工回归集不代表所有用户输入分布。",
    ]
    (output / "report.md").write_text("\n".join(lines) + "\n", encoding="utf-8")
    return report


def main() -> None:
    repo = Path(__file__).resolve().parents[1]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--lexicon", required=True, type=Path)
    parser.add_argument("--binaries", type=Path, default=repo / "pinyin-ime/target/release")
    parser.add_argument("--output", type=Path, default=repo / "dist/input-benchmark")
    args = parser.parse_args()
    generate_report(repo, args.binaries.resolve(), args.lexicon.resolve(), args.output.resolve())
    print(args.output / "report.md")


if __name__ == "__main__":
    main()
