"""Tests for tools/check-clippy.py: python tools/check_clippy_test.py"""

import importlib.util
import os
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SPEC = importlib.util.spec_from_file_location("check_clippy", os.path.join(ROOT, "tools", "check-clippy.py"))
cc = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(cc)

OUTPUT = """\
   Compiling replay-columnar v0.1.0 (/home/boat/src/crates/replay-columnar)
warning: call to `std::mem::drop` with a value that does not implement `Drop`. Dropping such a type only extends its contained lifetimes
    --> crates/replay-columnar/src/writer/tests.rs:1341:9
     |
1341 |         drop(hour);
     |         ^^^^^^^^^^
     |
note: argument has type `writer::Hour<'_>`
    --> crates/replay-columnar/src/writer/tests.rs:1341:14
     |
     = help: for further information visit https://rust-lang.github.io/rust-clippy/rust-1.96.0/index.html#drop_non_drop
     = note: `-W clippy::drop-non-drop` implied by `-W clippy::suspicious`

warning: unused import: `std::fmt`
 --> crates/replay-columnar/src/lib.rs:3:5
  |
  = note: `#[warn(unused_imports)]` on by default

\x1b[1m\x1b[33mwarning\x1b[0m: the `dbg!` macro is intended as a debugging tool
  --> crates\\replay-columnar\\src\\lib.rs:9:5
   |
   = help: for further information visit https://rust-lang.github.io/rust-clippy/rust-1.96.0/index.html#dbg_macro

warning: call to `std::mem::drop` with a value that does not implement `Drop`. Dropping such a type only extends its contained lifetimes
    --> crates/replay-columnar/src/writer/tests.rs:1341:9
     = help: for further information visit https://rust-lang.github.io/rust-clippy/rust-1.96.0/index.html#drop_non_drop

warning[E0133]: call to unsafe function `read` is unsafe and requires unsafe block
  --> crates/replay-columnar/src/lib.rs:15:5
   |
   = note: requested on the command line with `-W unsafe-op-in-unsafe-fn`

warning[E0133]: dereference of raw pointer is unsafe and requires unsafe block
  --> crates/replay-columnar/src/lib.rs:71:5

warning[E0133]: call to deprecated safe function `std::env::set_var` is unsafe and requires unsafe block
  --> crates/replay-columnar/src/lib.rs:80:5

error[E0133]: call to unsafe function `read` is unsafe and requires unsafe function or block
  --> crates/replay-columnar/src/lib.rs:90:5

warning: used `unwrap()` on a `Result` value
  --> crates/replay-columnar/tests/helpers.rs:13:5
   = help: for further information visit https://rust-lang.github.io/rust-clippy/rust-1.96.0/index.html#unwrap_used

warning: `replay-columnar` (lib test) generated 6 warnings (5 duplicates)
"""


class Findings(unittest.TestCase):
    def test_clippy_diagnostics_are_read_once_each_at_their_primary_line(self):
        self.assertEqual(cc.findings(OUTPUT.splitlines(), {"unsafe_op_in_unsafe_fn"}), [
            ("crates/replay-columnar/src/writer/tests.rs", 1341, "drop_non_drop", "warning",
             "call to `std::mem::drop` with a value that does not implement `Drop`. Dropping such a type only extends its contained lifetimes"),
            ("crates/replay-columnar/src/lib.rs", 9, "dbg_macro", "warning", "the `dbg!` macro is intended as a debugging tool"),
            ("crates/replay-columnar/src/lib.rs", 15, "unsafe_op_in_unsafe_fn", "warning",
             "call to unsafe function `read` is unsafe and requires unsafe block"),
            ("crates/replay-columnar/src/lib.rs", 71, "unsafe_op_in_unsafe_fn", "warning",
             "dereference of raw pointer is unsafe and requires unsafe block"),
        ])



class Judge(unittest.TestCase):
    def test_only_findings_on_written_lines_fail(self):
        found = [("a.rs", 3, "todo", "warning", "m"), ("a.rs", 9, "todo", "warning", "m"), ("new.rs", 1, "todo", "warning", "m"), ("old.rs", 1, "todo", "warning", "m")]
        new, existing = cc.judge(found, {"a.rs": {3, 4}, "new.rs": None})
        self.assertEqual([(path, line) for path, line, *_ in new], [("a.rs", 3), ("new.rs", 1)])
        self.assertEqual(existing, 2)

    def test_written_lines_come_from_the_diff_and_untracked_files(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            git = lambda *args: subprocess.run(["git", *args], cwd=root, check=True, capture_output=True)
            git("init", "-q", "-b", "main")
            git("config", "diff.noprefix", "true")
            (root / "a.rs").write_text("one\ntwo\nthree\nfour\n", encoding="utf-8")
            (root / "data reader.rs").write_text("one\n", encoding="utf-8")
            git("add", "-A")
            git("-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "-m", "base")
            (root / "a.rs").write_text("one\nTWO\nthree\nfour\nfive\n", encoding="utf-8")
            (root / "data reader.rs").write_text("ONE\n", encoding="utf-8")
            (root / "new.rs").write_text("x\n", encoding="utf-8")
            saved, cc.ROOT = cc.ROOT, root
            try:
                self.assertEqual(cc.written("main", None), {"a.rs": {2, 5}, "data reader.rs": {1}, "new.rs": None})
            finally:
                cc.ROOT = saved


if __name__ == "__main__":
    unittest.main()
