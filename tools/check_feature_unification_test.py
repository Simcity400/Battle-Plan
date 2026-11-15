"""Tests for tools/check-feature-unification.py: python tools/check_feature_unification_test.py"""

import importlib.util
import os
import tempfile
import tomllib
import unittest
from pathlib import Path

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SPEC = importlib.util.spec_from_file_location('check_feature_unification', os.path.join(ROOT, 'tools', 'check-feature-unification.py'))
fu = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(fu)


class UnifyManifest(unittest.TestCase):
    def test_a_version_in_both_tables_has_one_name_when_only_one_table_holds_a_second_version(self):
        target = {('windows-sys', '0.61.2', 'target'): ['Win32'], ('windows-sys', '0.59.0', 'target'): ['Win32']}
        host = {('windows-sys', '0.61.2', 'host'): ['Win32']}
        kept = fu.UNIFY_DIR
        with tempfile.TemporaryDirectory() as folder:
            fu.UNIFY_DIR = Path(folder)
            try:
                fu.write_unify(target, host)
            finally:
                fu.UNIFY_DIR = kept
            manifest = tomllib.loads((Path(folder) / 'Cargo.toml').read_text(encoding='utf-8'))
        named = lambda table: {key for key, spec in manifest[table].items() if spec['version'] == '0.61.2'}
        self.assertEqual(named('dependencies'), named('build-dependencies'))
        self.assertEqual(len(manifest['dependencies']), 2)


if __name__ == '__main__':
    unittest.main()
