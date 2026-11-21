"""Tests for tools/ci_results.py: python tools/ci_results_test.py"""

import json
import os
import subprocess
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import ci_results as cr

LISTING = json.dumps({"rust-suites": {
    "replay": {"testcases": {
        "reads": {"filter-match": {"status": "matches"}},
        "writes": {"filter-match": {"status": "matches"}},
        "slow": {"filter-match": {"status": "mismatch", "reason": "ignored"}},
    }},
    "viewer::bin/viewer": {"testcases": {
        "draws": {"filter-match": {"status": "matches"}},
        "times_out": {"filter-match": {"status": "matches"}},
        "never_ran": {"filter-match": {"status": "matches"}},
    }},
}})
JUNIT = """<?xml version="1.0" encoding="UTF-8"?>
<testsuites name="nextest-run" tests="4" failures="1" errors="1">
  <testsuite name="replay" tests="2"><testcase name="reads" classname="replay"/>
    <testcase name="writes" classname="replay"/></testsuite>
  <testsuite name="viewer::bin/viewer" tests="2">
    <testcase name="draws" classname="viewer::bin/viewer"><failure type="test failure">panicked</failure></testcase>
    <testcase name="times_out" classname="viewer::bin/viewer"><error type="test timeout"/></testcase>
  </testsuite>
</testsuites>"""


class Counts(unittest.TestCase):
    def test_a_run_counts_passed_failed_skipped_and_missing_apart(self):
        tests = cr.tests(LISTING, JUNIT)
        self.assertEqual((tests.passed, tests.failed, tests.skipped, tests.missing), (2, 2, 1, 1))
        self.assertEqual(tests.failures, ["viewer::bin/viewer draws", "viewer::bin/viewer times_out"])
        self.assertEqual(tests.describe(), "2 passed, 2 failed, 1 skipped, 1 missing")
        self.assertFalse(tests.complete())

    def test_a_run_whose_every_selected_test_passed_is_complete(self):
        listing = json.dumps({"rust-suites": {"replay": {"testcases": {
            "reads": {"filter-match": {"status": "matches"}},
            "slow": {"filter-match": {"status": "mismatch", "reason": "ignored"}},
        }}}})
        junit = '<testsuites><testsuite name="replay"><testcase name="reads"/></testsuite></testsuites>'
        tests = cr.tests(listing, junit)
        self.assertEqual(tests.describe(), "1 passed, 0 failed, 1 skipped, 0 missing")
        self.assertTrue(tests.complete())

    def test_without_a_report_every_selected_test_is_missing_and_results_are_not_recorded(self):
        tests = cr.tests(LISTING, None)
        self.assertEqual(tests.describe(), "passed not recorded, failed not recorded, 1 skipped, 5 missing")
        self.assertFalse(tests.complete())

    def test_a_binary_left_out_unlisted_leaves_skipped_not_recorded(self):
        listing = json.loads(LISTING)
        listing["rust-suites"]["dashboard"] = {"status": "skipped-default-filter", "testcases": {}}
        tests = cr.tests(json.dumps(listing), JUNIT)
        self.assertEqual(tests.describe(), "2 passed, 2 failed, skipped not recorded, 1 missing")

    def test_without_a_listing_skipped_and_missing_are_not_recorded_never_zero(self):
        tests = cr.tests(None, JUNIT)
        self.assertEqual(tests.describe(), "2 passed, 2 failed, skipped not recorded, missing not recorded")
        self.assertFalse(cr.Tests(passed=3, failed=0).complete())
        self.assertTrue(cr.Tests(passed=3, failed=0, missing=0).complete())
        self.assertEqual(cr.tests("not json", "<not xml").describe(), cr.Tests().describe())
        self.assertEqual(cr.Tests().describe(),
                         "passed not recorded, failed not recorded, skipped not recorded, missing not recorded")


BOAT_OUTPUT = """   Compiling replay v0.1.0
Doc tests: not run; they build and run through rustdoc on one machine

running 3 tests
test codec::reads ... ok
test codec::slow ... ignored, takes a minute
test codec::writes ... FAILED

failures:

---- codec::writes stdout ----
thread 'codec::writes' panicked

failures:
    codec::writes

test result: FAILED. 1 passed; 1 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.01s

Test program: replay lib replay: C:/out/replay-1.exe: exit 101

running 2 tests

running 1 test
test decode::tests::child ... FAILED

failures:
    decode::tests::child

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

test decode::tests::spawns_itself_and_sees_the_child_fail ... ok
test decode::tests::reads ... ok

test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.20s

Test program: collector lib collector: C:/out/collector-2.exe: exit 0
Tests: 2 of 2 programs run, 6 tests ran; exit 101; 9.00 s from command start
"""


class Libtest(unittest.TestCase):
    def test_each_program_counts_from_its_own_summary_never_a_child_harness(self):
        tests = cr.libtest(BOAT_OUTPUT)
        self.assertEqual(tests.describe(), "3 passed, 1 failed, 1 skipped, 0 missing")
        self.assertEqual(tests.failures, ["replay lib replay codec::writes"])
        self.assertFalse(tests.complete())
        clean = BOAT_OUTPUT.split("Test program: replay lib replay: C:/out/replay-1.exe: exit 101\n")[1]
        self.assertTrue(cr.libtest(clean.replace("Tests: 2 of 2", "Tests: 1 of 1")).complete())

    def test_a_program_that_ended_before_its_summary_is_named_and_missing_not_recorded(self):
        crashed = ("running 2 tests\ntest a ... ok\nTest program: viewer bin viewer: x: exit 0xc0000005\n"
                   "Test program: map bin map: y: exit 0xc0000135\nTests: 2 of 2 programs run\n")
        tests = cr.libtest(BOAT_OUTPUT.replace("Tests: 2 of 2 programs run", crashed.rstrip("\n")))
        self.assertEqual(tests.describe(), "3 passed, 1 failed, 1 skipped, missing not recorded")
        self.assertEqual(tests.failures[1:], ["viewer bin viewer: ended before its test summary (exit 0xc0000005)",
                                              "map bin map: ended before its test summary (exit 0xc0000135)"])

    def test_a_run_whose_every_program_crashed_records_no_counts_and_names_them(self):
        crashed = "running 2 tests\ntest first ... ok\nTest program: viewer bin viewer: x: exit 0xc0000005\nTests: 1 of 1 programs run\n"
        tests = cr.libtest(crashed)
        self.assertEqual(tests.describe(), cr.Tests().describe())
        self.assertEqual(tests.failures, ["viewer bin viewer: ended before its test summary (exit 0xc0000005)"])

    def test_a_run_cut_short_leaves_missing_not_recorded(self):
        ran = "running 1 test\ntest only ... ok\n\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\nTest program: a lib a: x: exit 0\n"
        self.assertEqual(cr.libtest(ran).describe(), "1 passed, 0 failed, 0 skipped, missing not recorded")
        self.assertIsNone(cr.libtest(ran + "Tests: 1 of 4 programs run, 1 tests ran; exit 0\n").missing)
        self.assertIsNone(cr.libtest(ran + "Tests: 1 of 1 programs run\nrunning 3 tests\n").missing)
        tests = cr.libtest(ran + "Tests: 1 of 1 programs run, 1 tests ran; exit 0\n")
        self.assertEqual(tests.describe(), "1 passed, 0 failed, 0 skipped, 0 missing")
        self.assertTrue(tests.complete())

    def test_output_with_no_program_records_nothing(self):
        self.assertEqual(cr.libtest("BOAT UNAVAILABLE: no VM\n").describe(), cr.Tests().describe())


class Replies:
    """A stand-in for `gh api`: answers each call from `replies` by method and path prefix."""

    def __init__(self, replies):
        self.replies, self.calls = replies, []

    def __call__(self, command, cwd, input, **_):
        method, path = command[3], command[4]
        self.calls.append((method, path, json.loads(input) if input else None))
        for (want_method, prefix), (code, reply) in self.replies.items():
            if method == want_method and path.startswith(prefix):
                return subprocess.CompletedProcess(command, code, json.dumps(reply) if code == 0 else "", "" if code == 0 else reply)
        return subprocess.CompletedProcess(command, 0, "{}", "")


ISSUES = "repos/{owner}/{repo}/issues"


class Publishing(unittest.TestCase):
    def github(self, replies):
        run = Replies(replies)
        return cr.GitHub("checkout", run=run), run

    def test_a_status_is_set_on_the_commit_and_its_description_fits(self):
        github, run = self.github({})
        self.assertIsNone(github.status("abc", "daily-tests/ui", "failure", "x" * 200))
        method, path, body = run.calls[0]
        self.assertEqual((method, path), ("POST", "repos/{owner}/{repo}/statuses/abc"))
        self.assertEqual((body["context"], body["state"]), ("daily-tests/ui", "failure"))
        self.assertEqual(len(body["description"]), cr.DESCRIPTION)

    def test_a_failing_run_opens_the_issue_once_then_comments_on_it(self):
        opened = ("POST", ISSUES, {"title": "Daily tests are failing", "body": "body", "labels": [cr.TRACKING_LABEL]})
        github, run = self.github({("GET", ISSUES): (0, [])})
        self.assertIsNone(github.track(True, "Daily tests are failing", "body"))
        self.assertEqual(run.calls[1:], [("POST", "repos/{owner}/{repo}/labels", {"name": cr.TRACKING_LABEL}), opened])
        github, run = self.github({("GET", ISSUES): (0, []), ("POST", "repos/{owner}/{repo}/labels"): (1, "HTTP 422: already_exists")})
        self.assertIsNone(github.track(True, "Daily tests are failing", "body"))
        self.assertEqual(run.calls[-1], opened)
        github, run = self.github({("GET", ISSUES): (0, [
            {"number": 7, "title": "Daily tests are failing"},
            {"number": 8, "title": "Daily tests are failing", "pull_request": {}},
            {"number": 9, "title": "A maintainer's investigation"},
        ])})
        self.assertIsNone(github.track(True, "Daily tests are failing", "again"))
        self.assertEqual(run.calls[1:], [("POST", f"{ISSUES}/7/comments", {"body": "again"})])

    def test_a_passing_run_closes_its_open_issues_only_and_opens_none(self):
        github, run = self.github({("GET", ISSUES): (0, [
            {"number": 7, "title": "title"}, {"number": 8, "title": "other"}, {"number": 9, "title": "title"},
        ])})
        self.assertIsNone(github.track(False, "title", "passed"))
        self.assertEqual(run.calls[1:], [
            ("POST", f"{ISSUES}/7/comments", {"body": "passed"}),
            ("PATCH", f"{ISSUES}/7", {"state": "closed", "state_reason": "completed"}),
            ("POST", f"{ISSUES}/9/comments", {"body": "passed"}),
            ("PATCH", f"{ISSUES}/9", {"state": "closed", "state_reason": "completed"}),
        ])
        github, run = self.github({("GET", ISSUES): (0, [])})
        self.assertIsNone(github.track(False, "title", "passed"))
        self.assertEqual(len(run.calls), 1)

    def test_a_failed_call_is_named_never_raised(self):
        github, _ = self.github({("POST", "repos/{owner}/{repo}/statuses/"): (1, "HTTP 422: No commit found")})
        self.assertIn("HTTP 422: No commit found", github.status("abc", "daily-tests", "error", "x"))
        github, _ = self.github({("GET", ISSUES): (4, "gh auth login")})
        self.assertIn("gh auth login", github.track(True, "title", "body"))

        def missing(*args, **kwargs):
            raise FileNotFoundError("gh")
        self.assertIn("gh", cr.GitHub("checkout", run=missing).status("abc", "c", "error", "x"))


if __name__ == "__main__":
    unittest.main()
