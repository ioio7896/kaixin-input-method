import unittest
from check_public_privacy import violations

class PublicPrivacyTests(unittest.TestCase):
    def test_runtime_databases_and_logs_are_excluded(self):
        for name in ('user_dict.sqlite-wal', 'clipboard_store.sqlite', 'ocr_history.sqlite', 'engine_capability.dat', 'engine.log'):
            self.assertTrue(violations(name, b''), name)

    def test_static_data_is_allowed(self):
        self.assertEqual(violations('pinyin-ime/data/s2t_chars.sqlite', b'SQLite format 3'), [])
        self.assertEqual(violations('tests/input_cases.sqlite', b'SQLite format 3'), [])

    def test_home_paths_include_json_escaping(self):
        home = 'C:' + chr(92) + 'Users' + chr(92) + 'PrivateAccount' + chr(92) + 'file.txt'
        for value in (home, home.replace(chr(92), chr(92)*2), '/'+ 'home' +'/PrivateAccount/file.txt'):
            self.assertIn('personal absolute path', violations('docs/example.md', value.encode()))

    def test_public_windows_paths_remain_usable(self):
        value = 'C:' + chr(92) + 'Users' + chr(92) + 'Public' + chr(92) + 'file.txt'
        self.assertEqual(violations('docs/example.md', value.encode()), [])

    def test_credentials_are_detected_without_printing_values(self):
        values = [('ghp_' + 'A'*32).encode(), ('github_pat_' + 'A'*48).encode(), ('AKIA' + '0'*16).encode()]
        for value in values:
            self.assertIn('possible credential', violations('settings.txt', value))

    def test_normal_example_contact_and_attribution_are_allowed(self):
        self.assertEqual(violations('LICENSE', b'Copyright upstream; example@example.com'), [])

if __name__ == '__main__': unittest.main()
