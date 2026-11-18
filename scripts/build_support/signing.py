from __future__ import annotations
import argparse
import os
import shutil
from pathlib import Path


def signing_requested(args: argparse.Namespace) -> bool:
    return bool(
        args.sign
        or args.sign_required
        or os.environ.get("KX_SIGN_CERT_SHA1")
        or os.environ.get("KX_SIGN_PFX")
    )


def find_signtool() -> Path | None:
    configured = os.environ.get("KX_SIGNTOOL")
    if configured:
        p = Path(configured)
        if p.is_file():
            return p.resolve()
    w = shutil.which("signtool.exe") or shutil.which("signtool")
    if w:
        return Path(w).resolve()
    roots = [
        Path(os.environ.get("ProgramFiles(x86)", r"C:\Program Files (x86)"))
        / "Windows Kits"
        / "10"
        / "bin",
        Path(os.environ.get("ProgramFiles", r"C:\Program Files")) / "Windows Kits" / "10" / "bin",
    ]
    for root in roots:
        if not root.is_dir():
            continue
        for candidate in sorted(root.glob(r"*\x64\signtool.exe"), reverse=True):
            if candidate.is_file():
                return candidate.resolve()
    return None


def sign_file(path: Path, args: argparse.Namespace, *, context, run, log) -> bool:
    if not signing_requested(args):
        return False
    if context.dry_run:
        log(f"  [dry-run] sign: {path}")
        return True
    if not path.is_file():
        raise FileNotFoundError(f"cannot sign missing file: {path}")

    signtool = find_signtool()
    thumbprint = os.environ.get("KX_SIGN_CERT_SHA1", "").strip()
    pfx = os.environ.get("KX_SIGN_PFX", "").strip()
    pfx_password = os.environ.get("KX_SIGN_PFX_PASSWORD", "")
    timestamp = os.environ.get("KX_SIGN_TIMESTAMP_URL", "http://timestamp.digicert.com").strip()

    missing_reason = None
    if not signtool:
        missing_reason = "signtool.exe not found"
    elif not thumbprint and not pfx:
        missing_reason = "set KX_SIGN_CERT_SHA1 or KX_SIGN_PFX"
    if missing_reason:
        if args.sign_required:
            raise RuntimeError(f"code signing requested but unavailable: {missing_reason}")
        if not context.sign_warning_emitted:
            log(f"  ! code signing skipped: {missing_reason}")
            context.sign_warning_emitted = True
        return False

    cmd = [str(signtool), "sign", "/fd", "SHA256"]
    if timestamp:
        cmd += ["/tr", timestamp, "/td", "SHA256"]
    if thumbprint:
        cmd += ["/sha1", thumbprint]
    else:
        cmd += ["/f", pfx]
        if pfx_password:
            cmd += ["/p", pfx_password]
    cmd.append(str(path))
    # Never print the PFX password in the command transcript or failure text.
    # The raw command is still passed to signtool, while ``run`` uses the
    # redacted representation for stdout, build logs, and BuildCommandError.
    run(cmd, cwd=path.parent, sensitive_values=(pfx_password,))
    return True
