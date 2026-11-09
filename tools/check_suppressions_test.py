"""Tests for tools/check-suppressions.py: python tools/check_suppressions_test.py"""

import importlib.util
import os
import unittest

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SPEC = importlib.util.spec_from_file_location("check_suppressions", os.path.join(ROOT, "tools", "check-suppressions.py"))
cs = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(cs)


class Unreasoned(unittest.TestCase):
    def test_a_suppression_with_a_reason_is_fine(self):
        self.assertEqual(cs.unreasoned('#[allow(dead_code, reason = "used by the hook")]\nfn f() {}'), [])
        self.assertEqual(cs.unreasoned('#![expect(clippy::all, reason = "generated")]'), [])

    def test_a_suppression_without_one_is_reported_at_its_line(self):
        self.assertEqual(cs.unreasoned("fn a() {}\n#[allow(dead_code)]\nfn f() {}"), [2])
        self.assertEqual(cs.unreasoned("#![expect(unused)]"), [1])

    def test_a_reason_on_a_later_line_of_the_same_attribute_counts(self):
        text = '#[allow(\n    clippy::too_many_arguments,\n    reason = "one per column"\n)]\nfn f() {}'
        self.assertEqual(cs.unreasoned(text), [])

    def test_parentheses_inside_a_reason_do_not_end_the_attribute_early(self):
        self.assertEqual(cs.unreasoned('#[allow(dead_code, reason = "see f() (below)")]'), [])

    def test_a_conditional_suppression_needs_a_reason_too(self):
        self.assertEqual(cs.unreasoned("#[cfg_attr(not(test), allow(dead_code))]\nfn f() {}"), [1])
        self.assertEqual(cs.unreasoned('#[cfg_attr(not(test), allow(dead_code, reason = "x"))]'), [])
        self.assertEqual(cs.unreasoned("#[cfg_attr(test, derive(Debug))]\nstruct S;"), [])

    def test_each_suppression_in_a_conditional_one_needs_its_own_reason(self):
        text = '#[cfg_attr(test, allow(dead_code, reason = "x"), allow(unused_variables))]'
        self.assertEqual(cs.unreasoned(text), [1])
        self.assertEqual(cs.unreasoned("#[cfg_attr(test, allow(a), expect(b))]"), [1, 1])
        self.assertEqual(cs.unreasoned("#[cfg_attr(a, cfg_attr(b, allow(c)))]"), [1])
        self.assertEqual(cs.unreasoned('#[cfg_attr(a, cfg_attr(b, allow(c, reason = "x")))]'), [])

    def test_raw_identifiers_name_the_same_attributes(self):
        self.assertEqual(cs.unreasoned("#[cfg_attr(all(), r#allow(dead_code))]"), [1])
        self.assertEqual(cs.unreasoned("#[r#expect(dead_code)]"), [1])
        self.assertEqual(cs.unreasoned("#[cfg_attr(a, r#cfg_attr(b, r#expect(c)))]"), [1])

    def test_raw_c_and_byte_strings_are_masked_like_raw_strings(self):
        self.assertEqual(cs.unreasoned('const Q: &CStr = cr#"""#;\n#[allow(dead_code)]\nfn f() {}'), [2])
        self.assertEqual(cs.unreasoned('const Q: &[u8] = br"#[allow(x)]";\n#[allow(dead_code)]\nfn f() {}'), [2])

    def test_comments_and_literals_are_not_attributes(self):
        self.assertEqual(cs.unreasoned("// #[allow(dead_code)]\nfn f() {}"), [])
        self.assertEqual(cs.unreasoned("/* #[allow(dead_code)] */ fn f() {}"), [])
        self.assertEqual(cs.unreasoned('const S: &str = r#"#[allow(dead_code)]"#;'), [])
        self.assertEqual(cs.unreasoned('const S: &str = "#[allow(dead_code)]";'), [])
        self.assertEqual(cs.unreasoned("/// #[allow(dead_code)]\nfn f() {}"), [])

    def test_the_word_reason_in_a_comment_or_string_is_not_a_reason(self):
        self.assertEqual(cs.unreasoned('#[allow(dead_code /* reason = "later" */)]'), [1])
        self.assertEqual(cs.unreasoned('#[allow(dead_code)] // reason = "x"'), [1])
        self.assertEqual(cs.unreasoned('#[allow(dead_code)]\nconst S: &str = "reason = x";'), [1])

    def test_a_quote_in_a_char_literal_does_not_hide_the_next_attribute(self):
        self.assertEqual(cs.unreasoned("const Q: char = '\"';\n#[allow(dead_code)]\nfn f() {}"), [2])
        self.assertEqual(cs.unreasoned("fn f<'a>(x: &'a str) {}\n#[allow(dead_code)]\nfn g() {}"), [2])

    def test_other_attributes_are_ignored(self):
        self.assertEqual(cs.unreasoned("#[derive(Debug)]\n#[cfg(test)]\nfn f() {}"), [])


class Files(unittest.TestCase):
    def test_a_name_status_entry_is_read_at_its_old_name_when_renamed_or_copied_and_at_its_own_otherwise(self):
        output = "M\0a.rs\0A\0b.rs\0R090\0old.rs\0new.rs\0C100\0src.rs\0copy.rs\0"
        self.assertEqual(
            cs.parse_name_status(output),
            [("a.rs", "a.rs"), ("b.rs", "b.rs"), ("new.rs", "old.rs"), ("copy.rs", "src.rs")],
        )
        self.assertEqual(cs.parse_name_status(""), [])

    def test_only_rust_files_under_the_checked_trees_are_candidates(self):
        names = ["crates/a/src/lib.rs", "tools/x.py", "ui/a.rs", "vendor/b/c.rs", "crates/a/Cargo.toml"]
        self.assertEqual(cs.rust_files(names), ["crates/a/src/lib.rs", "vendor/b/c.rs"])


if __name__ == "__main__":
    unittest.main()
