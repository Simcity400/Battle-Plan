#!/usr/bin/env python3
"""Hold the domain map to being a function of the SOURCE tree, and current.

Two failures put this file here, both of them silent:

  * the generator scanned its own CSV, so consecutive runs were a fixed-point
    iteration over the previous answer rather than a description of the tree
    (lane JL measured an 846 KB self-reference delta; lane JX recorded it as
    JX-R2 and stopped, because "re-run the generator" has no meaning until the
    starting point stops mattering);

  * nothing ever compared the committed CSV to what the tree produces, so it
    drifted 25 domain rows behind the registry -- 63 rows against 88 domains --
    while every lane that read it believed it.

The synthetic-tree tests own the first and the `--check` test owns the second.
"""

from __future__ import annotations

import csv
import importlib.util
import io
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(os.path.abspath(__file__)).parent
GENERATOR = HERE / "map_files_to_domains.py"
REPO = HERE.parents[2]

_spec = importlib.util.spec_from_file_location("map_files_to_domains", GENERATOR)
mfd = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(mfd)


class ExclusionRules(unittest.TestCase):
    """The predicates, read directly. Cheap, and they name the defect."""

    def test_the_generators_own_output_is_not_source(self):
        # kills: deleting the `rel == OUR_OUTPUT` clause.
        self.assertTrue(mfd.is_outside_source_corpus(mfd.OUR_OUTPUT))
        self.assertTrue(mfd.is_outside_source_corpus(mfd.OUR_OUTPUT.replace("/", "\\")))

    def test_append_only_evidence_is_not_source(self):
        # kills: deleting either `runs/` clause. Both spellings occur: the repo
        # has `runs/` at its root and `extractor/native/runs/` under it.
        self.assertTrue(mfd.is_outside_source_corpus("runs/kd_domain_map/RECEIPT.md"))
        self.assertTrue(mfd.is_outside_source_corpus(
            "extractor/native/runs/kd_domain_map/committed_map.csv"))
        self.assertTrue(mfd.is_outside_source_corpus(
            "extractor\\native\\runs\\jl_render_purge\\RECEIPT.md"))

    def test_ordinary_source_is_source(self):
        # kills: an exclusion written wide enough to swallow the corpus -- a
        # rule that excluded everything would pass every test above.
        for rel in ("extractor/native/collector/src/domain_registry.rs",
                    "extractor/coverage_audit/atlas_generated/save_path_catalog.csv",
                    "extractor/documents/technical_evidence.json"):
            self.assertFalse(mfd.is_outside_source_corpus(rel), rel)

    def test_scannable_text_does_not_rewrite_source(self):
        text = ("# d10 - fuel_live\n\n## Condensed knowledge\n\nSee also d11.\n"
                "\n## Files about this domain\n\n- `lanes/d99_elsewhere/x.rs`\n")
        kept = mfd.scannable_text("extractor/documents/domains/d10_fuel_live.md", text)
        self.assertIn("See also d11.", kept)
        self.assertIn("d99_elsewhere", kept)

    def test_a_file_outside_the_dossier_directory_is_read_whole(self):
        # kills: applying the dossier split to every file.
        text = "before\n## Files about this domain\nafter\n"
        self.assertEqual(mfd.scannable_text("extractor/documents/STATUS.md", text),
                         text)


def _run_generator(repo: Path, *extra: str) -> subprocess.CompletedProcess:
    return subprocess.run(
        [sys.executable, str(GENERATOR), "--repo", str(repo), *extra],
        capture_output=True, text=True, encoding="utf-8", errors="replace")


class PurityOnASyntheticTree(unittest.TestCase):
    """The end-to-end property: the answer does not depend on the last answer.

    A predicate test can be satisfied by a predicate nothing calls. This builds
    a five-file repository whose only d11 evidence OUTSIDE the registry is
    planted inside three generated artifacts -- the map, a dossier's derived
    section, and a lane evidence file -- runs the generator twice, and requires
    that none of the three reaches a row and that the second run reproduces the
    first byte for byte.
    """

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="kd_domain_map_")
        self.repo = Path(self.tmp.name)
        self.addCleanup(self.tmp.cleanup)

    def _write(self, rel: str, text: str) -> None:
        path = self.repo / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")

    def _build_tree(self) -> None:
        subprocess.run(["git", "init", "-q", str(self.repo)], check=True,
                       capture_output=True)
        self._write("extractor/native/domain_registry.json", json.dumps({"domains": [
            {"id": 10, "name": "fuel_live"},
            {"id": 11, "name": "production_live"},
        ]}))
        # The one honest source file. It names d10 and nothing else.
        self._write("extractor/native/collector/src/only_source.rs",
                    "// d10 is read here.\n")
        # Generated artifact 1: a previous map, poisoned with a d11 row.
        self._write(mfd.OUR_OUTPUT,
                    "domain_id,domain_name,file_count,files\r\n"
                    "11,production_live,1,extractor/native/lanes/d11_ghost/lib.rs\r\n")
        # Generated artifact 2: a dossier whose derived section names d11, and
        # whose hand-written half names only d10.
        self._write("extractor/documents/domains/d10_fuel_live.md",
                    "# d10\n\n## Condensed knowledge\n\nfuel.\n\n"
                    "## Files about this domain\n\n- `lanes/d11_ghost/lib.rs`\n")
        # Generated artifact 3: append-only lane evidence quoting the map.
        self._write("extractor/native/runs/some_lane/RECEIPT.md",
                    "The d11 row moved: extractor/native/lanes/d11_ghost/lib.rs\n")
        subprocess.run(["git", "-C", str(self.repo), "add", "-A"], check=True,
                       capture_output=True)

    def test_planted_evidence_in_generated_artifacts_never_reaches_the_map(self):
        self._build_tree()
        first = _run_generator(self.repo)
        self.assertEqual(first.returncode, 0, first.stderr)
        out = (self.repo / mfd.OUR_OUTPUT).read_text(encoding="utf-8")
        rows = {r[0]: r[3] for r in mfd.parsed_rows(out)[1:]}

        # kills: any exclusion regressing. `d11_ghost` and the three artifacts
        # that name it exist ONLY as generated copies of a previous map, so any
        # of them appearing in any row means one was read back. d11 itself
        # still has a row -- the registry names every domain, and the registry
        # is source -- so the assertion is on the row's CONTENT, not on d11
        # being absent, which would pass for the wrong reason.
        self.assertEqual(rows["11"], "extractor/native/domain_registry.json")
        self.assertNotIn("d11_ghost", out)
        self.assertNotIn("runs/some_lane", out)
        self.assertNotIn(mfd.OUR_OUTPUT, out)
        # The dossier still reaches its OWN domain, through its filename.
        self.assertIn("extractor/documents/domains/d10_fuel_live.md", rows["10"])
        self.assertIn("only_source.rs", rows["10"])

        # kills: the fixed-point iteration itself -- run 1 overwrote the
        # poisoned CSV, so a generator that reads its own output would produce
        # a different run 2 from a different starting state.
        second = _run_generator(self.repo)
        self.assertEqual(second.returncode, 0, second.stderr)
        self.assertEqual((self.repo / mfd.OUR_OUTPUT).read_text(encoding="utf-8"),
                         out)

    def test_check_mode_writes_nothing_and_reports_drift(self):
        self._build_tree()
        poisoned = (self.repo / mfd.OUR_OUTPUT).read_bytes()

        drifted = _run_generator(self.repo, "--check")
        # kills: a --check that cannot go red, and a --check that writes.
        self.assertEqual(drifted.returncode, 1, drifted.stdout)
        self.assertIn("DRIFT", drifted.stderr)
        self.assertEqual((self.repo / mfd.OUR_OUTPUT).read_bytes(), poisoned)

        self.assertEqual(_run_generator(self.repo).returncode, 0)
        current = _run_generator(self.repo, "--check")
        self.assertEqual(current.returncode, 0, current.stderr)


class CheckComparesTheWholeFile(unittest.TestCase):
    """`--check` compares parsed rows rather than bytes, because core.autocrlf
    makes a checkout's line endings machine-dependent. These are the shapes a
    row comparison can be blind to if it is written as a dict keyed on domain
    id -- which is how the first version of it WAS written, and Codex's round-1
    review reproduced a green `--check` against a renamed header through it.
    """

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="kd_domain_map_check_")
        self.repo = Path(self.tmp.name)
        self.addCleanup(self.tmp.cleanup)
        subprocess.run(["git", "init", "-q", str(self.repo)], check=True,
                       capture_output=True)
        (self.repo / "extractor/native").mkdir(parents=True)
        (self.repo / "extractor/native/domain_registry.json").write_text(
            json.dumps({"domains": [{"id": 10, "name": "fuel_live"},
                                    {"id": 11, "name": "production_live"}]}),
            encoding="utf-8")
        src = self.repo / "extractor/native/collector/src"
        src.mkdir(parents=True)
        (src / "a.rs").write_text("// d10\n", encoding="utf-8")
        (src / "b.rs").write_text("// d11\n", encoding="utf-8")
        subprocess.run(["git", "-C", str(self.repo), "add", "-A"], check=True,
                       capture_output=True)
        self.assertEqual(_run_generator(self.repo).returncode, 0)
        # read_bytes, not read_text: universal newlines would translate the
        # CRLF csv.writer emits into LF and these tests edit whole lines.
        self.current = (self.repo / mfd.OUR_OUTPUT).read_bytes().decode("utf-8")
        self.assertEqual(_run_generator(self.repo, "--check").returncode, 0)

    def _check_after(self, text: str) -> subprocess.CompletedProcess:
        (self.repo / mfd.OUR_OUTPUT).write_text(text, encoding="utf-8",
                                                newline="")
        return _run_generator(self.repo, "--check")

    def test_a_renamed_header_is_drift(self):
        # kills: comparing only the data rows. The header is part of the
        # committed machine-readable contract.
        bad = self.current.replace("domain_id,domain_name,file_count,files",
                                   "wrong,name,count,paths", 1)
        self.assertNotEqual(bad, self.current)
        done = self._check_after(bad)
        self.assertEqual(done.returncode, 1, done.stdout)
        self.assertIn("header is", done.stderr)

    def test_a_row_with_the_wrong_column_count_is_drift(self):
        # kills: a comparison that joins the columns back together. The
        # malformed row is DERIVED from the generated one -- name and count
        # merged into a single quoted field, count and file list otherwise
        # untouched -- so the only thing that changed is the column structure.
        # A first version of this case hard-coded the row and so changed its
        # content too, which meant the old lossy comparator still caught it and
        # the case proved nothing (round-2 SHOULD-FIX 2).
        rows = mfd.parsed_rows(self.current)
        row = next(r for r in rows[1:] if r[0] == "10")
        merged = [row[0], f"{row[1]},{row[2]}", row[3]]
        # The control: under the comparator this replaced, the two ARE equal.
        self.assertEqual(",".join(row[1:]), ",".join(merged[1:]))
        self.assertNotEqual(row, merged)

        buf = io.StringIO(newline="")
        writer = csv.writer(buf)
        for candidate in rows:
            writer.writerow(merged if candidate is row else candidate)
        done = self._check_after(buf.getvalue())
        self.assertEqual(done.returncode, 1, done.stdout)
        self.assertIn("could not have written", done.stderr)

    def test_a_non_numeric_domain_id_is_reported_not_raised(self):
        # kills: sorting the diagnostic's ids with key=int over whatever a
        # hand-edited file happens to hold. The gate was already red here; it
        # printed a ValueError traceback on top of the red, which is not a
        # diagnostic (round-2 SHOULD-FIX 1).
        done = self._check_after(self.current + 'not_a_domain,x,1,x\r\n')
        self.assertEqual(done.returncode, 1, done.stdout)
        self.assertIn("could not have written", done.stderr)
        self.assertNotIn("Traceback", done.stderr)

    def test_an_id_that_is_a_digit_but_not_an_int_is_reported_not_raised(self):
        # kills: `row[0].isdigit()` as the guard. It answers a DIFFERENT
        # question from the one the sort asks -- "²".isdigit() is True and
        # int("²") raises -- so a superscript id passed the guard and then
        # reached the very key=int sort the guard exists to protect. Also
        # covers the spellings int() accepts but this generator never writes.
        for spelling in ("²", "010", "+1", " 1", "9" * 5000):
            with self.subTest(spelling=spelling):
                done = self._check_after(self.current + f'{spelling},x,1,x\r\n')
                self.assertEqual(done.returncode, 1, done.stdout)
                self.assertNotIn("Traceback", done.stderr)
                self.assertIn("could not have written", done.stderr)

    def test_an_empty_map_is_reported_without_a_negative_count(self):
        # kills: `len(have) - 1` as the data-row count, which called an empty
        # file "-1 rows".
        done = self._check_after("")
        self.assertEqual(done.returncode, 1, done.stdout)
        self.assertNotIn("-1", done.stderr)
        self.assertNotIn("Traceback", done.stderr)

    def test_a_duplicated_domain_row_is_drift(self):
        # kills: keying the comparison on domain id, which collapses two rows
        # for d10 into one and calls the file current.
        lines = self.current.rstrip("\r\n").split("\r\n")
        done = self._check_after("\r\n".join(lines + [lines[1]]) + "\r\n")
        self.assertEqual(done.returncode, 1, done.stdout)
        self.assertIn("distinct domain id", done.stderr)

    def test_reordered_rows_are_drift(self):
        # kills: an unordered comparison. Rows are written in ascending domain
        # id and a reader is entitled to that.
        lines = self.current.rstrip("\r\n").split("\r\n")
        done = self._check_after("\r\n".join([lines[0], lines[2], lines[1]])
                                 + "\r\n")
        self.assertEqual(done.returncode, 1, done.stdout)
        self.assertIn("different order", done.stderr)

    def test_line_endings_alone_are_not_drift(self):
        # kills: a byte comparison, which would fail on every checkout whose
        # core.autocrlf setting differs from the machine that regenerated.
        self.assertEqual(
            self._check_after(self.current.replace("\r\n", "\n")).returncode, 0)


class OversizeSkipsAreNeverSilent(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="kd_domain_map_big_")
        self.repo = Path(self.tmp.name)
        self.addCleanup(self.tmp.cleanup)
        subprocess.run(["git", "init", "-q", str(self.repo)], check=True,
                       capture_output=True)
        (self.repo / "extractor/native").mkdir(parents=True)
        (self.repo / "extractor/native/domain_registry.json").write_text(
            json.dumps({"domains": [{"id": 10, "name": "fuel_live"}]}),
            encoding="utf-8")
        src = self.repo / "extractor/native/collector/src"
        src.mkdir(parents=True)
        (src / "small.rs").write_text("// d10\n", encoding="utf-8")
        (src / "huge.rs").write_text("// d10\n" + "x" * (mfd.MAX_BYTES + 1),
                                     encoding="utf-8")
        subprocess.run(["git", "-C", str(self.repo), "add", "-A"], check=True,
                       capture_output=True)

    def test_both_modes_name_the_file_they_did_not_read(self):
        # kills: dropping report_oversize from either mode. A file over the
        # guard is not evidence that it names no domain, it is a file the run
        # did not read -- and the gate is the invocation most likely to be the
        # only output anyone sees, which is where `attach_log.rs` disappeared
        # from 55 rows without a word.
        write = _run_generator(self.repo)
        self.assertEqual(write.returncode, 0, write.stderr)
        self.assertIn("huge.rs", write.stdout)
        self.assertIn("NOT READ", write.stdout)
        self.assertNotIn("huge.rs", (self.repo / mfd.OUR_OUTPUT)
                         .read_text(encoding="utf-8"))

        green = _run_generator(self.repo, "--check")
        self.assertEqual(green.returncode, 0, green.stderr)
        self.assertIn("huge.rs", green.stderr)

        # kills: moving report_oversize into the green branch, which would leave
        # every other case passing while the RED check -- the one a lane
        # actually reads, because it is the one that stopped their battery --
        # went silent again (round-2 NIT 1).
        (self.repo / mfd.OUR_OUTPUT).write_text(
            "domain_id,domain_name,file_count,files\r\n", encoding="utf-8",
            newline="")
        red = _run_generator(self.repo, "--check")
        self.assertEqual(red.returncode, 1, red.stdout)
        self.assertIn("huge.rs", red.stderr)


class CommittedMapIsCurrent(unittest.TestCase):
    def test_regenerating_this_repo_would_change_nothing(self):
        # kills: the drift this lane closed, coming back. Any tracked source
        # file gaining or losing a dNN token without the map being regenerated
        # fails here, naming the rows that moved.
        done = _run_generator(REPO, "--check")
        self.assertEqual(done.returncode, 0,
                         (done.stdout or "") + (done.stderr or ""))


if __name__ == "__main__":
    unittest.main(verbosity=2)
