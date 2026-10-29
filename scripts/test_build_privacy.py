"""Regression checks for private paths in release compiler arguments."""
import os
from pathlib import Path
import sys
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
import build


class ReleaseBuildPrivacyTests(unittest.TestCase):
    def test_encoded_flags_preserve_precedence_and_paths_with_spaces(self):
        repo = ROOT / 'workspace with spaces'
        with patch.dict(os.environ, {
            'CARGO_ENCODED_RUSTFLAGS': '--cfg\x1fexample="two words"',
            'RUSTFLAGS': '--cfg should_not_be_used',
        }):
            flags = build.cargo_build_environment(repo, 'release')['CARGO_ENCODED_RUSTFLAGS'].split('\x1f')
        self.assertEqual(flags[:2], ['--cfg', 'example="two words"'])
        self.assertNotIn('should_not_be_used', flags)
        self.assertIn('--remap-path-prefix=' + str(repo.resolve()) + '=/build/kaixin', flags)

    def test_plain_flags_are_retained_and_debug_builds_keep_source_locations(self):
        with patch.dict(os.environ, {'RUSTFLAGS': '-C debuginfo=1'}):
            os.environ.pop('CARGO_ENCODED_RUSTFLAGS', None)
            flags = build.cargo_build_environment(ROOT, 'release')['CARGO_ENCODED_RUSTFLAGS'].split('\x1f')
            self.assertEqual(flags[:2], ['-C', 'debuginfo=1'])
            self.assertNotIn('CARGO_ENCODED_RUSTFLAGS', build.cargo_build_environment(ROOT, 'debug'))

    def test_changed_compiler_flags_invalidate_the_incremental_build(self):
        with patch.dict(os.environ, {'CARGO_ENCODED_RUSTFLAGS': '--cfg\x1ffirst_build'}):
            first = build._cargo_inputs_fingerprint(ROOT, 'release')
            os.environ['CARGO_ENCODED_RUSTFLAGS'] = '--cfg\x1fsecond_build'
            second = build._cargo_inputs_fingerprint(ROOT, 'release')
        self.assertIsNotNone(first)
        self.assertIsNotNone(second)
        self.assertNotEqual(first[1], second[1])


if __name__ == '__main__':
    unittest.main()
