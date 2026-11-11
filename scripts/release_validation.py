"""Mandatory staged-binary release gate; never guesses versions from PE strings."""
from __future__ import annotations

import json
import hashlib
import os
from pathlib import Path
import shutil
import subprocess
import tempfile


def read_json_command(command: list[str], env: dict[str, str]) -> dict:
    result = subprocess.run(command, capture_output=True, text=True, encoding="utf-8",
                            errors="replace", env=env, timeout=35,
                            creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
    if result.returncode:
        raise RuntimeError(f"Release gate failed ({result.returncode}): {command}\n{result.stderr}")
    try:
        value = json.loads(result.stdout.strip())
    except ValueError as exc:
        raise RuntimeError(f"Missing structured build/probe result: {command}: {result.stdout}") from exc
    if not isinstance(value, dict):
        raise RuntimeError(f"Invalid build/probe result: {command}")
    return value


def validate_metadata(components: dict[str, dict], version: str, protocol: int,
                      commit: str | None = None) -> None:
    expected = {"engine": ("engine", 64), "x64": ("tsf", 64), "x86": ("tsf", 32)}
    if set(components) != set(expected):
        raise RuntimeError("Release gate requires engine, x64 TSF and x86 TSF metadata")
    for name, (kind, bits) in expected.items():
        metadata = components[name]
        fields = {"schema": 1, "component": kind, "bits": bits,
                  "version": version, "protocol": protocol}
        if commit and commit != "unknown":
            fields["git"] = commit
        for field, wanted in fields.items():
            if metadata.get(field) != wanted:
                raise RuntimeError(f"{name} {field} mismatch: {metadata.get(field)!r} != {wanted!r}; "
                                   "rebuild Rust and both TSF architectures together")


def verify_staged_release(repo: Path, stage: Path, runtime: Path, profile: str,
                          commit: str | None = None) -> dict:
    version = (repo / "VERSION").read_text(encoding="utf-8").strip()
    if (stage / "VERSION").read_text(encoding="utf-8").strip() != version:
        raise RuntimeError("Staged VERSION differs from the release source")
    protocol = int((repo / "shared/engine_protocol_version.txt").read_text())
    helpers = {arch: repo / "tsf-tip" / folder / profile.capitalize() / "srf_release_probe.exe"
               for arch, folder in (("x64", "build-package"), ("x86", "build-package-x86"))}
    for helper in helpers.values():
        if not helper.is_file():
            raise RuntimeError(f"Missing release probe: {helper}; run the complete native build")
    engine = stage / "srf_ime_engine.exe"
    with tempfile.TemporaryDirectory(prefix="kaixin-release-probe-") as temp:
        # Production TSF only trusts standard installation layouts. Reproduce
        # one under an isolated LOCALAPPDATA, preserving and hashing staged bytes
        # instead of weakening path trust or touching a real installed instance.
        isolated = Path(temp) / "Programs" / "kaixin"
        isolated.mkdir(parents=True)
        copied: dict[str, str] = {}
        sources = [engine, stage / "VERSION", stage / "current_runtime_payload.txt"]
        sources += [path for path in (stage / "lexicon").rglob("*") if path.is_file()]
        sources += [runtime / arch / "srf_tsf_tip.dll" for arch in ("x64", "x86")]
        for source in sources:
            relative = source.relative_to(stage)
            target = isolated / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source, target)
            expected_hash = hashlib.sha256(source.read_bytes()).hexdigest()
            if hashlib.sha256(target.read_bytes()).hexdigest() != expected_hash:
                raise RuntimeError(f"Release probe copy differs from staged payload: {relative}")
            copied[relative.as_posix()] = expected_hash
        isolated_runtime = isolated / runtime.relative_to(stage)
        env = {k: v for k, v in os.environ.items() if not k.upper().startswith("SRF_")}
        env.update(LOCALAPPDATA=temp, APPDATA=temp,
                   SRF_USER_DICT_PATH=str(Path(temp) / "user.sqlite"), SRF_IME_LOG_LEVEL="error")
        components = {"engine": read_json_command([str(engine), "--build-info"], env)}
        for arch, helper in helpers.items():
            components[arch] = read_json_command(
                [str(helper), "--metadata", str(runtime / arch / "srf_tsf_tip.dll")], env)
        validate_metadata(components, version, protocol, commit)
        with open(Path(temp) / "engine.log", "wb") as log:
            process = subprocess.Popen([str(isolated / engine.name), "--no-warmup"], cwd=isolated, env=env,
                                       stdout=log, stderr=log,
                                       creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
            try:
                probes = {}
                for arch, helper in helpers.items():
                    probes[arch] = read_json_command(
                        [str(helper), "--probe", str(isolated_runtime / arch / "srf_tsf_tip.dll")], env)
                    if probes[arch] != {"handshake": True, "input": "nihao", "matched": True}:
                        raise RuntimeError(f"Invalid {arch} nihao handshake result: {probes[arch]}")
                if process.poll() is not None:
                    raise RuntimeError("Staged engine exited during release validation")
            except Exception:
                log.flush()
                diagnostics = repo / "dist" / f"release-validation-{stage.name}-failure.log"
                chunks = []
                for path in [Path(temp) / "engine.log", *sorted((Path(temp) / "kaixin/logs").glob("*.log"))]:
                    if path.is_file():
                        chunks.append(path.name + "\n" + path.read_text(encoding="utf-8", errors="replace")[-32000:])
                diagnostics.write_text("\n".join(chunks), encoding="utf-8")
                raise
            finally:
                if process.poll() is None:
                    process.terminate()
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=5)
    report = {"schema": 1, "version": version, "components": components, "probes": probes,
              "probe_layout": "isolated LOCALAPPDATA/Programs/kaixin", "payload_sha256": copied}
    (repo / "dist" / f"release-validation-{stage.name}.json").write_text(
        json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    return report
