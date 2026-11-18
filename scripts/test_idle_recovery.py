"""Exercise real five-minute pipe expiry with isolated x64 and x86 TSF bridges."""

from __future__ import annotations

import argparse
import os
from pathlib import Path
import secrets
import shutil
import subprocess
import tempfile


ROOT = Path(__file__).resolve().parents[1]


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--stage", type=Path, default=ROOT / "dist" / "kaixin-package-ime")
    parser.add_argument("--arch", choices=("x64", "x86", "both"), default="both")
    args = parser.parse_args()
    stage = args.stage.resolve(strict=True)
    runtime = Path((stage / "current_runtime_payload.txt").read_text(encoding="utf-8").strip())
    arches = ("x64", "x86") if args.arch == "both" else (args.arch,)
    folders = {"x64": "build-package", "x86": "build-package-x86"}
    helpers = {
        arch: ROOT / "tsf-tip" / folders[arch] / "Release" / "srf_idle_recovery_probe.exe"
        for arch in arches
    }
    for helper in helpers.values():
        if not helper.is_file():
            raise RuntimeError(f"Missing idle recovery probe: {helper}")

    with tempfile.TemporaryDirectory(prefix="kaixin-idle-test-") as temp:
        temporary = Path(temp)
        # A synthetic legacy-format token lets both isolated clients authenticate
        # without accessing the user's DPAPI profile or racing token creation.
        data = temporary / "kaixin"
        data.mkdir()
        (data / "engine_capability.dat").write_text(secrets.token_hex(32), encoding="ascii")
        isolated = temporary / "Programs" / "kaixin"
        isolated.mkdir(parents=True)
        for name in ("srf_ime_engine.exe", "VERSION", "current_runtime_payload.txt"):
            shutil.copy2(stage / name, isolated / name)
        shutil.copytree(stage / "lexicon", isolated / "lexicon")
        isolated_helpers = {}
        for arch, helper in helpers.items():
            destination = isolated / runtime / arch / helper.name
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(helper, destination)
            isolated_helpers[arch] = destination
        env = {
            key: value for key, value in os.environ.items() if not key.upper().startswith("SRF_")
        }
        env.update(
            LOCALAPPDATA=temp,
            APPDATA=temp,
            SRF_USER_DICT_PATH=str(temporary / "user.sqlite"),
            SRF_IME_LOG_LEVEL="error",
        )
        flags = getattr(subprocess, "CREATE_NO_WINDOW", 0)
        engine = subprocess.Popen(
            [str(isolated / "srf_ime_engine.exe"), "--no-warmup"],
            cwd=isolated,
            env=env,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            creationflags=flags,
        )
        probes = {}
        try:
            for arch, helper in isolated_helpers.items():
                probes[arch] = subprocess.Popen(
                    [str(helper)],
                    cwd=isolated,
                    env=env,
                    stdout=subprocess.PIPE,
                    stderr=subprocess.PIPE,
                    creationflags=flags,
                )
            print("Waiting for the real 5-minute pipe expiry", flush=True)
            for arch, probe in probes.items():
                stdout, stderr = probe.communicate(timeout=335)
                if probe.returncode:
                    raise RuntimeError(
                        f"{arch} idle probe failed ({probe.returncode}): "
                        f"{stderr.decode(errors='replace')}"
                    )
                print(f"{arch}: {stdout.decode(errors='replace').strip()}", flush=True)
            if engine.poll() is not None:
                raise RuntimeError("The isolated engine exited during the idle test")
        finally:
            for probe in probes.values():
                if probe.poll() is None:
                    probe.kill()
                    probe.wait(timeout=10)
            if engine.poll() is None:
                engine.terminate()
                engine.wait(timeout=10)


if __name__ == "__main__":
    main()
