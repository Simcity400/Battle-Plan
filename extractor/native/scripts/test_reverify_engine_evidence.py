import hashlib
import unittest

from reverify_engine_evidence import matches_lf_export


def pin(data):
    return len(data), hashlib.sha256(data).hexdigest()


class LfExportTests(unittest.TestCase):
    def setUp(self):
        self.raw = b"// build pin\nvoid f()\r\n{\r\n  return;\r\n}\r\n"
        self.lf = self.raw.replace(b"\r\n", b"\n")

    def test_exact_crlf_conversion_with_two_independent_pins(self):
        self.assertTrue(matches_lf_export(self.lf, self.raw, pin(self.raw), pin(self.lf)))

    def test_changed_non_newline_bytes_fail_even_with_a_restamped_checkout_pin(self):
        for changed in [self.lf.replace(b"return", b"other"),
                        self.lf.replace(b"  return", b" return"),
                        self.lf.replace(b"void f", b"void g"), self.lf + b"\n"]:
            self.assertFalse(matches_lf_export(changed, self.raw, pin(self.raw), pin(changed)))

    def test_wrong_raw_hash_size_or_absent_pin_fail(self):
        for raw_pin in [None, (len(self.raw), "0" * 64), (len(self.raw) + 1, pin(self.raw)[1])]:
            self.assertFalse(matches_lf_export(self.lf, self.raw, raw_pin, pin(self.lf)))

    def test_wrong_checkout_hash_size_or_absent_pin_fail(self):
        for committed_pin in [None, (len(self.lf), "0" * 64), (len(self.lf) + 1, pin(self.lf)[1])]:
            self.assertFalse(matches_lf_export(self.lf, self.raw, pin(self.raw), committed_pin))

    def test_changed_canonical_body_cannot_be_laundered_by_line_endings(self):
        changed = self.raw.replace(b"return", b"other")
        lf = changed.replace(b"\r\n", b"\n")
        self.assertFalse(matches_lf_export(lf, changed, pin(self.raw), pin(lf)))

    def test_lone_cr_is_not_normalized(self):
        raw = b"a\rb\r\n"
        for committed in [b"a\nb\n", b"a\rb\n"]:
            self.assertFalse(matches_lf_export(committed, raw, pin(raw), pin(committed)))


if __name__ == "__main__":
    unittest.main()
