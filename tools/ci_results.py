"""The results of a test run, counted and published (Python 3.11+).

`tests(listing, junit)` counts a nextest run from the output of `cargo nextest list
--message-format json` and the JUnit report of `cargo nextest run`: each listed test the filters
select is passed, failed or missing (selected, with no result), and each they leave out is
skipped. A count neither file can give is None, which `Tests.describe` says is not recorded.
`libtest(output)` counts the libtest programs `tools/boat.py test` runs from its output.

`GitHub(checkout)` publishes through `gh api` in a checkout of the repository, so the calls are
the repository's free REST API and never Actions: `status` sets a commit status, and `track`
keeps one open issue, labelled TRACKING_LABEL and named by its title, while runs fail: it opens it, comments on it
for each failing run, and closes it at the first passing one. Each returns None, or what failed.
"""
import json
import os
import re
import subprocess
import xml.etree.ElementTree as ElementTree

TRACKING_LABEL = "daily-tests"
DESCRIPTION = 140
BODY = 60_000
GH_TIMEOUT = 60


class Tests:
    """Test counts, each None when not recorded, and the tests that failed."""

    def __init__(self, passed=None, failed=None, skipped=None, missing=None, failures=()):
        self.passed, self.failed, self.skipped, self.missing = passed, failed, skipped, missing
        self.failures = list(failures)

    def describe(self):
        counts = (("passed", self.passed), ("failed", self.failed), ("skipped", self.skipped), ("missing", self.missing))
        return ", ".join(f"{name} not recorded" if count is None else f"{count} {name}" for name, count in counts)

    def complete(self):
        """Whether every selected test passed and none is missing; the skipped count, of tests
        not selected, does not decide it."""
        return self.failed == 0 and self.missing == 0 and self.passed is not None


def listed(listing):
    """(selected, skipped) test keys of a nextest JSON listing, skipped None when a binary was left
    out without listing its tests; or None when it is not a listing."""
    try:
        suites = json.loads(listing)["rust-suites"]
        selected, skipped = set(), set()
        for binary, suite in suites.items():
            if suite.get("status", "listed") != "listed":
                skipped = None
            for name, case in suite.get("testcases", {}).items():
                if case.get("filter-match", {}).get("status") == "matches":
                    selected.add((binary, name))
                elif skipped is not None:
                    skipped.add((binary, name))
        return selected, skipped
    except (ValueError, TypeError, KeyError, AttributeError):
        return None


def reported(junit):
    """{test key: passed} from a JUnit report, or None when it is not one."""
    try:
        root = ElementTree.fromstring(junit)
    except ElementTree.ParseError:
        return None
    results = {}
    for suite in root.iter("testsuite"):
        for case in suite.iter("testcase"):
            key = (suite.get("name") or case.get("classname"), case.get("name"))
            if case.find("skipped") is None:
                results[key] = case.find("failure") is None and case.find("error") is None
    return results


def tests(listing, junit):
    """The `Tests` of a run from its listing and JUnit texts, either None when absent."""
    selection = listed(listing) if listing is not None else None
    results = reported(junit) if junit is not None else None
    skipped = len(selection[1]) if selection and selection[1] is not None else None
    if results is None:
        return Tests(skipped=skipped, missing=len(selection[0]) if selection else None)
    failures = sorted(f"{binary} {name}" for (binary, name), passed in results.items() if not passed)
    passed = sum(results.values())
    if selection is None:
        return Tests(passed, len(failures), None, None, failures)
    return Tests(passed, len(failures), skipped, len(selection[0] - results.keys()), failures)


LIBTEST_RUNNING = re.compile(r"^running (\d+) tests?$")
LIBTEST_SUMMARY = re.compile(r"^test result: \w+\. (\d+) passed; (\d+) failed; (\d+) ignored; (\d+) measured")
LIBTEST_FAILED = re.compile(r"^    (\S.*)$")
BOAT_PROGRAM = re.compile(r"^Test program: (\S+) (\S+) (\S+): .*: exit (\S+)")
BOAT_SUMMARY = re.compile(r"^Tests: (\d+) of (\d+) programs run")


def program_counts(lines):
    """(passed, failed, ignored, failing names) of one program's output, from its own harness: the
    first test count it announces and the last summary, which must account for that count. Tests
    a program starts as child processes print their own harness output in between. None when
    the program ended before its summary."""
    announced = next((int(found[1]) for line in lines if (found := LIBTEST_RUNNING.match(line))), None)
    summaries = [index for index, line in enumerate(lines) if LIBTEST_SUMMARY.match(line)]
    if announced is None or not summaries:
        return None
    passed, failed, ignored, measured = map(int, LIBTEST_SUMMARY.match(lines[summaries[-1]]).groups())
    if passed + failed + ignored + measured != announced:
        return None
    headers = [index for index in range(summaries[-1]) if lines[index] == "failures:"]
    names = []
    if failed and headers:
        for line in lines[headers[-1] + 1:summaries[-1]]:
            if found := LIBTEST_FAILED.match(line):
                names.append(found[1])
            elif names:
                break
    return passed, failed, ignored, names


def libtest(output):
    """The `Tests` of libtest programs run by `boat.py test`, from its output: each program's own
    summary gives its passed, failed and ignored (skipped) tests. Missing is 0 only when Boat says
    every program ran and each ended with its summary, else None: some tests are unknown."""
    passed = failed = skipped = 0
    failures, every, complete, summarised, lines = [], False, True, False, []
    for line in output.splitlines():
        line = line.rstrip()
        if found := BOAT_SUMMARY.match(line):
            every = found[1] == found[2]
        elif found := BOAT_PROGRAM.match(line):
            program = f"{found[1]} {found[2]} {found[3]}"
            counts = program_counts(lines)
            if counts is None:
                complete = False
                failures.append(f"{program}: ended before its test summary (exit {found[4]})")
            else:
                summarised = True
                passed, failed, skipped = passed + counts[0], failed + counts[1], skipped + counts[2]
                failures += [f"{program} {name}" for name in counts[3]]
            lines = []
        else:
            lines.append(line)
    if any(LIBTEST_RUNNING.match(line) for line in lines):
        complete = False
    if not summarised:
        return Tests(failures=failures)
    return Tests(passed, failed, skipped, 0 if every and complete else None, failures)


class GitHub:
    """Commit statuses and the tracking issue of the repository `checkout` is a clone of."""

    def __init__(self, checkout, gh="gh", run=subprocess.run):
        self.checkout, self.gh, self.run = checkout, gh, run

    def api(self, method, path, body=None):
        """The decoded reply of one REST call; raises RuntimeError naming the call when it fails."""
        command = [self.gh, "api", "--method", method, path, *(["--input", "-"] if body is not None else [])]
        flags = {"creationflags": subprocess.CREATE_NO_WINDOW} if os.name == "nt" else {}
        try:
            done = self.run(command, cwd=self.checkout, input=None if body is None else json.dumps(body),
                            capture_output=True, text=True, encoding="utf-8", timeout=GH_TIMEOUT, **flags)
        except (OSError, subprocess.TimeoutExpired) as error:
            raise RuntimeError(f"{method} {path}: {error}") from None
        if done.returncode != 0:
            reason = (done.stderr or done.stdout or "").strip().splitlines()
            raise RuntimeError(f"{method} {path}: exit {done.returncode}: {reason[0] if reason else 'no output'}")
        return json.loads(done.stdout) if done.stdout.strip() else None

    def status(self, commit, context, state, description):
        """Sets `context` on `commit` to `state`: pending, success, failure or error."""
        if len(description) > DESCRIPTION:
            description = description[:DESCRIPTION - 1] + "…"
        try:
            self.api("POST", f"repos/{{owner}}/{{repo}}/statuses/{commit}",
                     {"state": state, "context": context, "description": description})
        except RuntimeError as error:
            return f"status {context}: {error}"
        return None

    def track(self, failing, title, body):
        """Opens or comments on the tracking issue when `failing`, else comments on and closes it."""
        if len(body) > BODY:
            body = body[:BODY] + "\n\n(cut short)"
        try:
            listed_issues = self.api("GET", f"repos/{{owner}}/{{repo}}/issues?state=open&labels={TRACKING_LABEL}&per_page=100")
            open_issues = [issue["number"] for issue in listed_issues or []
                           if "pull_request" not in issue and issue.get("title") == title]
            if failing and not open_issues:
                try:
                    self.api("POST", "repos/{owner}/{repo}/labels", {"name": TRACKING_LABEL})
                except RuntimeError as error:
                    if "already_exists" not in str(error) and "422" not in str(error):
                        raise
                self.api("POST", "repos/{owner}/{repo}/issues", {"title": title, "body": body, "labels": [TRACKING_LABEL]})
            for number in open_issues[:1] if failing else open_issues:
                self.api("POST", f"repos/{{owner}}/{{repo}}/issues/{number}/comments", {"body": body})
                if not failing:
                    self.api("PATCH", f"repos/{{owner}}/{{repo}}/issues/{number}", {"state": "closed", "state_reason": "completed"})
        except (RuntimeError, ValueError, TypeError, KeyError) as error:
            return f"tracking issue: {error}"
        return None
