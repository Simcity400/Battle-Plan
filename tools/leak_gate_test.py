"""Tests for tools/leak-gate.py: python tools/leak_gate_test.py"""

import base64
import importlib.util
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import unittest
import zipfile

ROOT = Path(__file__).resolve().parents[1]
GATE = ROOT / "tools" / "leak-gate.py"
SPEC = importlib.util.spec_from_file_location("leak_gate", GATE)
gate = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(gate)



def at(local, domain):
    return local + "@" + domain


TERM = "Planted" + "term"
KEY_LINE = "RWRT" + "y0Iy" + "planted" + "updater" + "keyline"
UPDATER_KEY = base64.b64encode(f"untrusted comment: test key\n{KEY_LINE}\n".encode()).decode()
SAMPLES = {
    "term": "see " + TERM.upper() + " here",
    "user-path": "C:" + "\\" + "Users" + "\\" + "someone" + "\\x",
    "steam-path": "D:/" + "Steam" + "Library" + "/x",
    "email": at("someone", "example.com"),
    "github-token": "gh" + "p_" + "a1" * 18,
    "private-key": "-----BEGIN " + "RSA PRIVATE" + " KEY-----",
    "aws-key": "AK" + "IA" + "ABCDEFGHIJ234567",
    "slack-token": "xo" + "xb-" + "1234567890abc",
    "secret-key": "encrypted " + "secret key",
    "updater-key": KEY_LINE.lower(),
}
NOREPLY = "users.noreply." + "github.com"
NAMED = {"pdb": "app.pdb", "source-map": "app.js.map"}
LINE = re.compile(r"^(.+): ([a-z-]+) x[1-9]\d*$")
FORMS = {
    "utf8": lambda text: text.encode(),
    "utf16": lambda text: text.encode("utf-16-le"),
    "utf16-odd": lambda text: b"\x01" + text.encode("utf-16-le"),
}


def run_gate(test, cwd, *paths):
    env = {**os.environ, "LEAK_GATE_TERMS": f'["{TERM}"]', "TAURI_SIGNING_PRIVATE_KEY": UPDATER_KEY}
    result = subprocess.run([sys.executable, str(GATE), *paths], cwd=cwd, env=env, capture_output=True, text=True)
    test.assertEqual(result.stderr, "")
    *lines, total = result.stdout.splitlines()
    test.assertRegex(total, r"^Leak gate: \d+ files checked, \d+ matches\.$")
    found = {}
    for line in lines:
        test.assertRegex(line, LINE)
        label, kind = LINE.fullmatch(line).groups()
        found.setdefault(label, set()).add(kind)
    return result, found


class Planted(unittest.TestCase):
    def test_each_kind_is_found_in_plain_files_and_archive_members_and_nothing_else_is(self):
        with tempfile.TemporaryDirectory() as scratch:
            scratch = Path(scratch)
            (scratch / "plain").mkdir()
            expected = {}
            inner = scratch / "inner.zip"
            with zipfile.ZipFile(inner, "w", zipfile.ZIP_DEFLATED) as archive:
                archive.writestr("deep/email.txt", SAMPLES["email"])
            expected["outer.zip/inner.zip/deep/email.txt"] = {"email"}
            with zipfile.ZipFile(scratch / "outer.zip", "w", zipfile.ZIP_DEFLATED) as archive:
                for kind, text in SAMPLES.items():
                    for form, encode in FORMS.items():
                        (scratch / "plain" / f"{kind}-{form}.txt").write_bytes(b"\x00 x " + encode(text) + b" x")
                        expected[f"plain/{kind}-{form}.txt"] = {kind}
                        archive.writestr(f"in/{kind}-{form}.txt", b"x " + encode(text) + b"\x00")
                        expected[f"outer.zip/in/{kind}-{form}.txt"] = {kind}
                for kind, name in NAMED.items():
                    (scratch / "plain" / name).write_bytes(b"{}")
                    expected[f"plain/{name}"] = {kind}
                    archive.writestr(f"in/{name}", b"{}")
                    expected[f"outer.zip/in/{name}"] = {kind}
                archive.write(inner, "inner.zip")
                archive.writestr("in/clean.txt", "nothing to see " * 100)
            (scratch / "plain" / "noreply.txt").write_text(at("1+me", NOREPLY) + " " + at("noreply", "github.com"))
            (scratch / "plain" / "LICENSE").write_text("Copyright someone " + SAMPLES["email"])
            inner.unlink()
            result, found = run_gate(self, scratch, "plain", "outer.zip")
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertEqual(found, expected)
        for text in [*SAMPLES.values(), TERM]:
            self.assertNotIn(text.lower(), result.stdout.lower())

    def test_one_kind_does_not_hide_another_in_the_same_bytes(self):
        with tempfile.TemporaryDirectory() as scratch:
            Path(scratch, "LICENSE").write_text(at(SAMPLES["aws-key"], "example.com"))
            Path(scratch, "notes.txt").write_text(SAMPLES["user-path"].replace("someone", TERM))
            _, found = run_gate(self, scratch, "LICENSE", "notes.txt")
        self.assertEqual(found, {"LICENSE": {"aws-key"}, "notes.txt": {"term", "user-path"}})



class Addresses(unittest.TestCase):
    def test_an_address_counts_once_unless_it_is_a_github_noreply_one(self):
        def count(*parts):
            return gate.addresses("".join(parts).encode())
        self.assertEqual(count(at("ab", "example.com"), ", ", at("cd", "example.org")), 2)
        self.assertEqual(count(at("1+me", NOREPLY), " ", at("noreply", "github.com")), 0)
        self.assertEqual(count(at("xnoreply", "github.com")), 1)
        self.assertEqual(count(at("a", "example.com"), " ", at("", "example.com"), " ", at("ab", "example.comx")), 0)
        self.assertEqual(count(at("ab", "cd.com.ef"), at("", "gh.com")), 2)
        self.assertEqual(count(at("x", NOREPLY), at("", "example.org")), 1)

    def test_a_noreply_address_does_not_hide_the_address_after_it(self):
        matcher = gate.Matcher([TERM], [], [("note.txt", "steam-path")])
        data = ("Steam" + "Library" + at("", NOREPLY) + at("", "example.org")).encode()
        self.assertEqual(matcher.scan_bytes("note.txt", data), [("email", 1)])


class Terms(unittest.TestCase):
    def test_overlapping_terms_count_one_match(self):
        for terms in (["foo", "foobar"], ["FOOBAR", "foo"]):
            self.assertEqual(gate.Matcher(terms, [], []).scan_bytes("a.txt", b"x FOOBAR x"), [("term", 1)])

    def test_a_term_inside_a_noreply_address_does_not_make_it_an_address(self):
        matcher = gate.Matcher(["nor"], [], [])
        self.assertEqual(matcher.scan_bytes("a.txt", at("noreply", "github.com").encode()), [("term", 1)])


if __name__ == "__main__":
    unittest.main()
