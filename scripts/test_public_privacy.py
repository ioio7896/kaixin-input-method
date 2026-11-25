import unittest
from check_public_privacy import violations


class PublicPrivacyTests(unittest.TestCase):
    def test_local_backups_and_development_transcripts_are_excluded(self):
        for name in (
            "kaixin-source-2026.1124.1736.zip",
            "2026_11_23-16_29_23",
            "2026_11_19-17_33_47.md",
        ):
            self.assertIn("local source backup or development transcript", violations(name, b""))

    def test_runtime_databases_and_logs_are_excluded(self):
        for name in (
            "user_dict.sqlite-wal",
            "clipboard_store.sqlite",
            "user_dict.kxj",
            "clipboard_store.kxj.tmp",
            "clipboard_manager_window.sqlite-journal",
            "ocr_history.sqlite",
            "runtime_events.sqlite-wal",
            "cedict_cache.sqlite-journal",
            "engine_capability.dat",
            "engine.log",
        ):
            self.assertTrue(violations(name, b""), name)

    def test_static_data_is_allowed(self):
        self.assertEqual(violations("pinyin-ime/data/s2t_chars.sqlite", b"SQLite format 3"), [])
        self.assertEqual(violations("tests/input_cases.sqlite", b"SQLite format 3"), [])

    def test_private_configuration_and_credential_files_are_excluded(self):
        for name in (
            "kaixin.ini",
            "config.private.ini",
            "settings.LOCAL.INI",
            ".cargo/credentials.toml",
            ".cargo/credentials",
            ".git-credentials",
            ".netrc",
            "id_ed25519",
            ".ssh/config",
            ".gnupg/secring.gpg",
            ".lexicon_cache/index",
            ".venv/pyvenv.cfg",
            "kaixin-diagnostics-example/config.ini",
        ):
            for relative in (name, name.replace("/", chr(92))):
                self.assertTrue(violations(relative, b""), relative)
        self.assertEqual(violations(".cargo/config.toml", b"[build]"), [])

    def test_home_paths_include_json_escaping(self):
        home = "C:" + chr(92) + "Users" + chr(92) + "PrivateAccount" + chr(92) + "file.txt"
        for value in (
            home,
            home.replace(chr(92), chr(92) * 2),
            "/" + "home" + "/PrivateAccount/file.txt",
        ):
            self.assertIn("personal absolute path", violations("docs/example.md", value.encode()))

    def test_public_windows_paths_remain_usable(self):
        value = "C:" + chr(92) + "Users" + chr(92) + "Public" + chr(92) + "file.txt"
        self.assertEqual(violations("docs/example.md", value.encode()), [])

    def test_credentials_are_detected_without_printing_values(self):
        values = [
            ("ghp_" + "A" * 32).encode(),
            ("github_pat_" + "A" * 48).encode(),
            ("AKIA" + "0" * 16).encode(),
        ]
        for value in values:
            self.assertIn("possible credential", violations("settings.txt", value))

    def test_normal_example_contact_and_attribution_are_allowed(self):
        self.assertEqual(violations("LICENSE", b"Copyright upstream; example@example.com"), [])


if __name__ == "__main__":
    unittest.main()
