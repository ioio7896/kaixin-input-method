"""Regression test for failures before current-user installer initialization."""

import os
from pathlib import Path
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]


class InstallerDiagnosticsTests(unittest.TestCase):
    @unittest.skipUnless(os.name == "nt", "Windows PowerShell is required")
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
                ["powershell.exe", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File",
                 str(ROOT / "tsf-tip/installer/install_current_user.ps1"),
                 "-InstallationRoot", str(root / "missing")],
                env=env, capture_output=True,
            )
            self.assertNotEqual(result.returncode, 0)
            content = log.read_text(encoding="utf-8-sig")
            current = content.rsplit("event=install_session_start", 1)[-1]
            self.assertIn("FAIL: Could not resolve runtime payload", current)
            self.assertNotIn("event=install_health_check status=ok", current)


if __name__ == "__main__":
    unittest.main()
