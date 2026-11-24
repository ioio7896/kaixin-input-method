"""Installer regressions using the native Rust srf_ime_setup binary."""

import hashlib
import os
from pathlib import Path
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]


def ensure_setup_exe() -> Path:
    for profile in ("release", "debug"):
        candidate = ROOT / "pinyin-ime" / "target" / profile / "srf_ime_setup.exe"
        if candidate.is_file():
            return candidate
    subprocess.run(
        [
            "cargo",
            "build",
            "--manifest-path",
            str(ROOT / "pinyin-ime" / "Cargo.toml"),
            "--locked",
            "--no-default-features",
            "--bin",
            "srf_ime_setup",
        ],
        cwd=ROOT,
        check=True,
    )
    return ROOT / "pinyin-ime" / "target" / "debug" / "srf_ime_setup.exe"


class InstallerDiagnosticsTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.setup_exe = ensure_setup_exe()

    def run_ps(self, script: str, root: Path):
        env = {k: v for k, v in os.environ.items() if k.upper() != "PSMODULEPATH"}
        subprocess.run(
            [
                "powershell.exe",
                "-NoProfile",
                "-ExecutionPolicy",
                "Bypass",
                "-Command",
                script + "\nexit 0",
            ],
            cwd=ROOT,
            env={**env, "KX_TEST_DIR": str(root), "KX_SETUP_EXE": str(self.setup_exe)},
            check=True,
        )

    @unittest.skipUnless(os.name == "nt", "NTFS ACLs are required")
    def test_install_directory_acl_upgrade(self):
        with tempfile.TemporaryDirectory(prefix="kaixin-acl-") as directory:
            root = Path(directory) / "kaixin"
            root.mkdir()
            (root / "child.dll").touch()
            self.run_ps(
                r"""
                $ErrorActionPreference = 'Stop'
                $root = $env:KX_TEST_DIR
                $exe = $env:KX_SETUP_EXE
                $original = Get-Acl -LiteralPath $root
                & $exe ensure-acl --root $root --machine
                if ($LASTEXITCODE -ne 0) { throw 'ensure-acl machine failed' }
                if ((Get-Acl -LiteralPath $root).Sddl -ne $original.Sddl) { throw 'fresh machine ACL changed' }
                $original.SetAccessRuleProtection($true, $true)
                [IO.DirectoryInfo]::new($root).SetAccessControl($original)
                & $exe ensure-acl --root $root --machine
                if ($LASTEXITCODE -ne 0) { throw 'ensure-acl machine upgrade failed' }
                if ((Get-Acl -LiteralPath $root).AreAccessRulesProtected) { throw 'legacy inheritance still disabled' }
                & $exe ensure-acl --root $root
                if ($LASTEXITCODE -ne 0) { throw 'ensure-acl user failed' }
                $acl = Get-Acl -LiteralPath (Join-Path $root 'child.dll')
                $rx = [Security.AccessControl.FileSystemRights]::ReadAndExecute
                foreach ($sid in 'S-1-15-2-1', 'S-1-15-2-2') {
                    $rules = $acl.GetAccessRules($false, $true, [Security.Principal.SecurityIdentifier])
                    if (-not ($rules | Where-Object {
                        $_.IdentityReference.Value -eq $sid -and $_.AccessControlType -eq 'Allow' -and
                        ($_.FileSystemRights -band $rx) -eq $rx -and -not ($_.FileSystemRights -band 2)
                    })) { throw "missing inherited read/execute grant for $sid" }
                }
                $before = (Get-Acl -LiteralPath $root).Sddl
                & $exe ensure-acl --root $root
                if ($LASTEXITCODE -ne 0) { throw 'ensure-acl user repeat failed' }
                if ((Get-Acl -LiteralPath $root).Sddl -ne $before) { throw 'non-idempotent user ACL' }
                """,
                root,
            )

    @unittest.skipUnless(os.name == "nt", "Windows setup helper is required")
    def test_fast_manifest_requires_each_core_file_and_repair_checks_optional_files(self):
        with tempfile.TemporaryDirectory(prefix="kaixin-integrity-") as directory:
            root = Path(directory)
            core = root / "tip.dll"
            core.write_bytes(b"core")
            digest = hashlib.sha256(core.read_bytes()).hexdigest()
            (root / "package_manifest.sha256").write_text(
                f"{digest} *tip.dll\n{digest} *ocr.dat\n", encoding="utf-8"
            )

            ok = subprocess.run(
                [
                    str(self.setup_exe),
                    "verify-manifest",
                    "--root",
                    str(root),
                    "--core-file",
                    "tip.dll",
                ],
                capture_output=True,
                text=True,
            )
            self.assertEqual(ok.returncode, 0, ok.stderr)

            full_fail = subprocess.run(
                [str(self.setup_exe), "verify-manifest", "--root", str(root)],
                capture_output=True,
                text=True,
            )
            self.assertNotEqual(full_fail.returncode, 0)
            self.assertIn("Missing required file", full_fail.stderr)

            missing_core = subprocess.run(
                [
                    str(self.setup_exe),
                    "verify-manifest",
                    "--root",
                    str(root),
                    "--core-file",
                    "x86.dll",
                ],
                capture_output=True,
                text=True,
            )
            self.assertNotEqual(missing_core.returncode, 0)
            self.assertIn("Missing core files", missing_core.stderr)

            core.write_bytes(b"changed")
            tampered = subprocess.run(
                [
                    str(self.setup_exe),
                    "verify-manifest",
                    "--root",
                    str(root),
                    "--core-file",
                    "tip.dll",
                ],
                capture_output=True,
                text=True,
            )
            self.assertNotEqual(tampered.returncode, 0)
            self.assertIn("Package integrity check failed", tampered.stderr)

    @unittest.skipUnless(os.name == "nt", "Windows setup helper is required")
    def test_missing_runtime_is_logged_in_current_session(self):
        with tempfile.TemporaryDirectory(prefix="kaixin-installer-test-") as directory:
            root = Path(directory)
            state = root / "kaixin"
            state.mkdir()
            log = state / "install_user.log"
            log.write_text(
                "event=install_session_start\nevent=install_health_check status=ok\n",
                encoding="utf-8",
            )
            env = os.environ.copy()
            env["LOCALAPPDATA"] = str(root)
            result = subprocess.run(
                [
                    str(self.setup_exe),
                    "install-user",
                    "--root",
                    str(root / "missing"),
                ],
                env=env,
                capture_output=True,
                text=True,
            )
            self.assertNotEqual(result.returncode, 0)
            content = log.read_text(encoding="utf-8-sig")
            current = content.rsplit("event=install_session_start", 1)[-1]
            self.assertIn("FAIL: Could not resolve runtime payload", current)
            self.assertNotIn("event=install_health_check status=ok", current)


if __name__ == "__main__":
    unittest.main()
