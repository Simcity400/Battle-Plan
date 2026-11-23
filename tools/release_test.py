import argparse
import base64
import datetime
import hashlib
import http.client
import io
import json
import os
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent))
import release

from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from cryptography.hazmat.primitives.serialization import Encoding, PublicFormat

def minisign_text(comment, box):
    return base64.b64encode(f"untrusted comment: {comment}\n{base64.b64encode(box).decode()}\n".encode()).decode()


def keypair(key_id):
    private = Ed25519PrivateKey.generate()
    public = private.public_key().public_bytes(Encoding.Raw, PublicFormat.Raw)
    return private, minisign_text("minisign public key", b"Ed" + key_id + public)


def signature_file(path, private, key_id, data, trusted):
    signature = private.sign(hashlib.blake2b(data, digest_size=64).digest())
    lines = ["untrusted comment: signature from tauri secret key", base64.b64encode(b"ED" + key_id + signature).decode(),
             f"trusted comment: {trusted}", base64.b64encode(private.sign(signature + trusted.encode())).decode()]
    path.write_text(base64.b64encode(("\n".join(lines) + "\n").encode()).decode(), encoding="utf-8")
    return path


class Versions(unittest.TestCase):
    def test_stable_is_the_next_patch_or_the_version_named_and_others_preview_it(self):
        day = datetime.datetime(2026, 11, 21, 23, 59, tzinfo=datetime.UTC)
        self.assertEqual(release.candidate_version("stable", "1.4.2", day, 900), "1.4.3")
        self.assertEqual(release.candidate_version("stable", "1.4.2", day, 900, "1.5.0"), "1.5.0")
        with self.assertRaisesRegex(release.Refused, "1.4.2 is not newer than 1.4.2"):
            release.candidate_version("stable", "1.4.2", day, 900, "1.4.2")
        self.assertEqual(release.candidate_version("nightly", "1.4.2", day, 900), "1.4.3-nightly.20261121.900")
        self.assertEqual(release.candidate_version("preview", "1.4.2", day, 7), "1.4.3-preview.20261121.7")

    def test_candidates_built_in_mains_order_sort_in_it_and_before_the_stable_they_preview(self):
        first, second = (datetime.datetime(2026, 11, day, tzinfo=datetime.UTC) for day in (21, 22))
        order = ["1.4.2", release.candidate_version("nightly", "1.4.2", first, 900),
                 release.candidate_version("nightly", "1.4.2", first, 901),
                 release.candidate_version("nightly", "1.4.2", second, 902), release.candidate_version("stable", "1.4.2", second, 902)]
        self.assertEqual(sorted(order, key=release.precedence), order)
        with self.assertRaises(release.Refused):
            release.refuse_older(order[1], order[2], "the feed")

    def test_the_lock_stamps_workspace_packages_and_leaves_vendored_and_registry_ones(self):
        lock = ("version = 4\n\n[[package]]\nname = \"replay\"\nversion = \"0.1.0\"\ndependencies = []\n\n"
                "[[package]]\nname = \"eframe\"\nversion = \"0.29.1\"\n\n"
                "[[package]]\nname = \"serde\"\nversion = \"0.1.0\"\nsource = \"registry+https://github.com/rust-lang/crates.io-index\"\n")
        stamped = release.stamp_lock(lock.replace("\n", "\r\n"), {"replay", "serde"}, "0.1.1-nightly.20261121.5")
        self.assertEqual(stamped, lock.replace('"replay"\nversion = "0.1.0"', '"replay"\nversion = "0.1.1-nightly.20261121.5"')
                         .replace("\n", "\r\n"))


class Keys(unittest.TestCase):
    def test_a_secret_key_passes_and_each_wrong_form_is_named(self):
        secret = minisign_text("rsign encrypted secret key", b"EdSc" + bytes(154))
        self.assertIsNone(release.key_problem(secret, "KEY"))
        wrapped = secret[:40] + "\n" + secret[40:]
        self.assertIn("one line of standard base64", release.key_problem(wrapped, "KEY"))
        self.assertIn("public key", release.key_problem(keypair(bytes(8))[1], "KEY"))
        self.assertIn("no minisign key", release.key_problem(base64.b64encode(b"hello").decode(), "KEY"))

    def test_authenticode_is_all_the_azure_settings_or_none(self):
        everything = {name: "x" for name in release.AZURE}
        with mock.patch.dict(os.environ, everything):
            self.assertTrue(release.authenticode_enabled(required=True))
        with mock.patch.dict(os.environ, {name: "" for name in release.AZURE}):
            self.assertFalse(release.authenticode_enabled(required=False))
            with self.assertRaisesRegex(release.Refused, "asked for"):
                release.authenticode_enabled(required=True)
            os.environ["AZURE_TENANT_ID"] = "x"
            with self.assertRaisesRegex(release.Refused, "AZURE_CLIENT_ID"):
                release.authenticode_enabled(required=False)


class Signatures(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.installer = self.root / "setup.exe"
        self.installer.write_bytes(b"installer bytes")
        self.key_id = bytes(range(8))
        self.private, self.public = keypair(self.key_id)

    def tearDown(self):
        self.temp.cleanup()

    def test_the_signature_verifies_bound_to_its_version_against_the_apps_key(self):
        signature = signature_file(self.root / "a.sig", self.private, self.key_id, b"installer bytes",
                                   "timestamp:1\tfile:setup.exe\tversion:1.4.3")
        self.assertEqual(release.verify_signature(self.installer, signature, "1.4.3", self.public), "0706050403020100")
        with self.assertRaisesRegex(release.Refused, "bound to version 1.4.3, not 1.4.4"):
            release.verify_signature(self.installer, signature, "1.4.4", self.public)
        with self.assertRaisesRegex(release.Refused, "does not match"):
            release.verify_signature(self.installer, signature, "1.4.3", keypair(bytes(8))[1])
        self.installer.write_bytes(b"installer bytes, signed with Authenticode after")
        with self.assertRaisesRegex(release.Refused, "does not verify"):
            release.verify_signature(self.installer, signature, "1.4.3", self.public)


class ReleaseOrder(unittest.TestCase):
    def order(self, tagged, existing, newest="1.4.2", feed=None):
        with mock.patch.multiple(release, on_main=mock.DEFAULT, read_feed=mock.Mock(return_value=feed),
                                 tag_commit=mock.Mock(return_value=tagged), release_by_tag=mock.Mock(return_value=existing),
                                 newest_release=mock.Mock(return_value=newest)):
            return release.release_order("owner/repo", "stable", "1.4.3", "b" * 40, "v1.4.3", draft=False)

    def test_a_release_whose_feed_failed_is_taken_up_unless_a_newer_one_is_out(self):
        published = {"tag_name": "v1.4.3"}
        self.assertEqual(self.order("b" * 40, published, newest="1.4.3"), (None, published, False))
        with self.assertRaisesRegex(release.Refused, "1.4.3 is not newer than 1.4.4"):
            self.order("b" * 40, published, newest="1.4.4")
        with self.assertRaisesRegex(release.Refused, "1.4.3 is not newer than 1.4.3"):
            self.order(None, None, newest="1.4.3")

    def test_a_feed_that_already_offers_this_release_is_done_and_another_version_of_it_refused(self):
        published = {"tag_name": "v1.4.3"}
        offered = {"commit": "b" * 40, "version": "1.4.3"}
        self.assertEqual(self.order("b" * 40, published, feed=offered), (offered, published, True))
        with self.assertRaisesRegex(release.Refused, "already offers bbbbbbbbbbbb, as 1.4.2"):
            self.order("b" * 40, published, feed={"commit": "b" * 40, "version": "1.4.2"})
        with self.assertRaisesRegex(release.Refused, "no longer names bbbbbbbbbbbb"):
            self.order("c" * 40, published, feed=offered)
        with mock.patch.multiple(release, on_main=mock.DEFAULT, read_feed=mock.Mock(return_value=offered)):
            self.assertEqual(release.release_order("owner/repo", "stable", "1.4.3", "b" * 40, "dry-run", draft=True),
                             (offered, None, False))

    def test_a_tag_on_another_commit_is_refused(self):
        with self.assertRaisesRegex(release.Refused, "already names aaaaaaaaaaaa"):
            self.order("a" * 40, None)

    def test_an_older_version_than_the_feed_is_refused(self):
        with mock.patch.object(release, "run", return_value=mock.Mock(returncode=0)):
            with self.assertRaisesRegex(release.Refused, "not newer than 1.4.4"):
                self.order(None, None, feed={"commit": "a" * 40, "version": "1.4.4"})

    def test_a_feed_emptied_by_a_failed_upload_reads_as_none(self):
        emptied = mock.Mock(returncode=1, stdout="", stderr="no assets to download")
        with mock.patch.object(release, "run", return_value=emptied):
            self.assertIsNone(release.read_feed("owner/repo", "nightly"))
        with mock.patch.object(release, "run", return_value=mock.Mock(returncode=0, stdout="", stderr="")):
            self.assertIsNone(release.read_feed("owner/repo", "nightly"))

    def test_the_newest_release_of_a_channel_is_its_highest_version(self):
        tags = mock.Mock(stdout=" ".join(["v1.4.2", "v1.4.3-nightly.20261121.900", "v1.4.3-nightly.20261122.901",
                                         "nightly", "evidence-x", "v1.4.10"]))
        with mock.patch.object(release, "run", return_value=tags):
            self.assertEqual(release.newest_release("owner/repo", "nightly"), "1.4.3-nightly.20261122.901")
            self.assertEqual(release.newest_release("owner/repo", "stable"), "1.4.10")


def answers(*replies):
    """A stand-in for `release.run` giving each reply's stdout in turn."""
    return mock.Mock(side_effect=[mock.Mock(stdout=reply, returncode=0, stderr="") for reply in replies])


class MainCi(unittest.TestCase):
    def runs(self, *found):
        keys = ("workflow_id", "name", "run_number", "run_attempt", "status", "conclusion")
        listed = [dict(zip(keys, run), path=f".github/workflows/{run[1].lower()}.yml") for run in found]
        return json.dumps({"workflow_runs": listed})

    def test_main_is_green_once_the_latest_run_of_each_workflow_its_push_started_settled_green(self):
        settled = self.runs((1, "CI", 3, 1, "completed", "failure"),
                            (1, "CI", 3, 2, "completed", "success"),
                            (2, "Comments", 8, 1, "completed", "skipped"))
        with mock.patch.object(release, "run", answers(settled)):
            self.assertIsNone(release.ci_state("owner/repo", "b" * 40))
        for listed in (self.runs(), self.runs((2, "Comments", 8, 1, "completed", "success"))):
            with mock.patch.object(release, "run", answers(listed)):
                self.assertEqual(release.ci_state("owner/repo", "b" * 40), "its CI run has not started")
        with mock.patch.object(release, "run", answers(self.runs((1, "CI", 3, 1, "completed", "success"),
                                                                 (2, "Comments", 8, 1, "in_progress", None)))):
            self.assertIn("Comments run is still going", release.ci_state("owner/repo", "b" * 40))
        with mock.patch.object(release, "run", answers(self.runs((1, "CI", 4, 1, "completed", "failure")))):
            self.assertIn("CI run ended failure", release.ci_state("owner/repo", "b" * 40))

    def test_the_newest_kept_candidate_mains_ci_built_is_taken_and_an_unusable_newer_one_passed_by(self):
        def artifact(expired, created, run):
            return {"expired": expired, "created_at": created, "workflow_run": {"id": run}}

        listing = json.dumps({"artifacts": [artifact(False, "2026-11-20", 7), artifact(False, "2026-11-21", 9),
                                            artifact(True, "2026-11-19", 5)]})
        verdicts = {9: "the run that built it ended failure", 7: None}
        with mock.patch.multiple(release, run=answers(listing),
                                 produced=mock.Mock(side_effect=lambda repo, found: verdicts[found["workflow_run"]["id"]])):
            self.assertEqual(release.find_candidate("owner/repo", "candidate-nightly-b"), (7, None))
        expired = json.dumps({"artifacts": [artifact(True, "2026-11-19", 5)]})
        with mock.patch.object(release, "run", answers(expired)):
            self.assertIn("has expired", release.find_candidate("owner/repo", "candidate-nightly-b")[1])
        with mock.patch.object(release, "run", answers(json.dumps({"artifacts": []}))):
            self.assertIn("no candidate", release.find_candidate("owner/repo", "candidate-nightly-b")[1])

    def test_only_mains_candidate_workflow_or_the_release_build_job_produces_a_candidate(self):
        def artifact(branch="main", head=1):
            return {"workflow_run": {"id": 9, "repository_id": 1, "head_repository_id": head, "head_branch": branch}}

        def state(path, event, conclusion="success"):
            return json.dumps({"path": f".github/workflows/{path}", "event": event, "status": "completed", "conclusion": conclusion})

        self.assertIn("not built from this repository's main", release.produced("owner/repo", artifact(branch="feature")))
        self.assertIn("not built from this repository's main", release.produced("owner/repo", artifact(head=2)))
        with mock.patch.object(release, "run", answers(state("release-candidate.yml", "workflow_dispatch"))):
            self.assertIsNone(release.produced("owner/repo", artifact()))
        for event in ("push", "pull_request"):
            with mock.patch.object(release, "run", answers(state("release-candidate.yml", event))):
                self.assertIn("does not build release candidates", release.produced("owner/repo", artifact()))
        with mock.patch.object(release, "run", answers(state("comments.yml", "push"))):
            self.assertIn("does not build release candidates", release.produced("owner/repo", artifact()))
        built = "Build / Windows x64 installer\tsuccess\nPublish\tfailure"
        for event in ("schedule", "workflow_dispatch"):
            with mock.patch.object(release, "run", answers(state("release.yml", event, "failure"), built)):
                self.assertIsNone(release.produced("owner/repo", artifact()))
        skipped = "Resolve the release\tsuccess\nBuild / Windows x64 installer\tskipped"
        with mock.patch.object(release, "run", answers(state("release.yml", "schedule"), skipped)):
            self.assertIn("its build job did not succeed", release.produced("owner/repo", artifact()))


class Resolve(unittest.TestCase):
    def resolve(self, draft=False, sha="", scheduled=True, **changes):
        args = argparse.Namespace(channel="nightly", sha=sha, version="", scheduled=scheduled, draft=draft, prepare=False,
                                  github_output=False)
        settings = dict(repository=mock.Mock(return_value="owner/repo"), git=mock.Mock(return_value=""),
                        on_main=mock.Mock(return_value="b" * 40),
                        read_feed=mock.Mock(return_value={"commit": "a" * 40, "version": "1.4.3-nightly.20261121.900"}),
                        run=mock.Mock(return_value=mock.Mock(returncode=1)),
                        ci_state=mock.Mock(return_value=None), nightly_going=mock.Mock(return_value=None),
                        newest_green=mock.Mock(return_value="b" * 40), whole_run_state=mock.Mock(return_value=None),
                        feed_uploaded=mock.Mock(return_value="2000-01-01T00:00:00Z"),
                        find_candidate=mock.Mock(return_value=(None, "no candidate candidate-nightly-b was built")),
                        outputs=mock.Mock())
        settings.update(changes)
        with mock.patch.multiple(release, **settings):
            release.resolve(args)
            return release.outputs.call_args[0][0]

    def test_a_scheduled_nightly_takes_the_newest_green_commit_since_the_feeds_and_builds_unless_its_candidate_is_kept(self):
        green = mock.Mock(return_value="b" * 40)
        due = self.resolve(newest_green=green)
        self.assertEqual((due["due"], due["commit"], due["artifact"], due["candidate_run"], due["build"], due["publish"]),
                         ("true", "b" * 40, "candidate-nightly-" + "b" * 40, "", "true", "true"))
        green.assert_called_once_with("owner/repo", "a" * 40)
        kept = self.resolve(find_candidate=mock.Mock(return_value=(9, None)))
        self.assertEqual((kept["due"], kept["candidate_run"], kept["build"], kept["publish"]), ("true", "9", "false", "true"))
        self.assertEqual(self.resolve(newest_green=mock.Mock(return_value=None))["due"], "false")
        everything = mock.Mock(return_value="b" * 40)
        self.assertEqual(self.resolve(draft=True, newest_green=everything)["due"], "true")
        everything.assert_called_once_with("owner/repo", None)
        with mock.patch.dict(os.environ, {"PAUSE_NIGHTLY": "true"}):
            self.assertEqual(self.resolve()["due"], "false")
        broken = mock.Mock(return_value="main's last whole-workspace CI run, of cccccccccccc, ended failure")
        self.assertEqual(self.resolve(whole_run_state=broken)["due"], "false")
        self.assertEqual(self.resolve(draft=True, whole_run_state=broken)["due"], "true")

    def test_a_failed_or_running_whole_workspace_run_bars_nightlies_until_one_passes(self):
        def listed(*conclusions):
            return json.dumps({"workflow_runs": [{"conclusion": found, "status": "in_progress" if found is None else "completed",
                                                  "head_sha": "c" * 40} for found in conclusions]})

        with mock.patch.object(release, "run", answers(listed("failure"))):
            self.assertIn("ended failure", release.whole_run_state("owner/repo"))
        with mock.patch.object(release, "run", answers(listed("cancelled"))):
            self.assertIn("ended cancelled", release.whole_run_state("owner/repo"))
        with mock.patch.object(release, "run", answers(listed(None))):
            self.assertIn("still going", release.whole_run_state("owner/repo"))
        with mock.patch.object(release, "run", answers(listed("success"))):
            self.assertIsNone(release.whole_run_state("owner/repo"))
        with mock.patch.object(release, "run", answers(listed())):
            self.assertIsNone(release.whole_run_state("owner/repo"))

    def test_a_nightly_whose_whole_workspace_run_failed_during_its_build_is_not_published(self):
        with tempfile.TemporaryDirectory() as folder:
            Path(folder, "candidate.json").write_text(json.dumps({"commit": "b" * 40, "version": "1.4.4-nightly.20261123.9"}))
            args = argparse.Namespace(channel="nightly", draft=False, public_key="", candidate=folder, commit="b" * 40, version="")
            broken = mock.Mock(return_value="main's last whole-workspace CI run, of bbbbbbbbbbbb, ended failure")
            with mock.patch.multiple(release, repository=mock.Mock(return_value="owner/repo"), git=mock.Mock(return_value=""),
                                     same_workflows=mock.Mock(), whole_run_state=broken, preflight=mock.Mock(side_effect=AssertionError)):
                with self.assertRaisesRegex(release.Refused, "ended failure; the candidate is kept"):
                    release.publish(args)

    def test_a_scheduled_nightly_is_due_once_a_utc_day_and_a_dispatched_one_whenever_asked(self):
        today = f"{datetime.datetime.now(datetime.UTC):%Y-%m-%d}"
        stamped_yesterday = mock.Mock(return_value={"commit": "a" * 40, "version": "1.4.3-nightly.20000101.900",
                                                    "pub_date": "2000-01-01T23:59:00Z"})
        uploaded = mock.Mock(return_value=f"{today}T00:01:00Z")
        self.assertEqual(self.resolve(read_feed=stamped_yesterday, feed_uploaded=uploaded)["due"], "false")
        uploaded.assert_called_once_with("owner/repo", "nightly")
        self.assertEqual(self.resolve(read_feed=stamped_yesterday, feed_uploaded=uploaded, scheduled=False)["due"], "true")
        self.assertEqual(self.resolve(feed_uploaded=mock.Mock(return_value="2000-01-01T00:01:00Z"))["due"], "true")
        self.assertEqual(self.resolve(read_feed=mock.Mock(return_value=None), feed_uploaded=mock.Mock(side_effect=AssertionError))["due"], "true")
        with mock.patch.object(release, "run", answers("\n")):
            with self.assertRaisesRegex(release.Refused, "no upload time"):
                release.feed_uploaded("owner/repo", "nightly")
        self.assertTrue(release.published_today("2026-11-22T00:01:00Z", datetime.datetime(2026, 11, 22, 0, 41)))
        self.assertFalse(release.published_today("2026-11-22T00:01:00Z", datetime.datetime(2026, 11, 23)))

    def test_a_named_nightly_commit_needs_green_ci_and_changes_since_the_feeds(self):
        self.assertEqual(self.resolve(sha="b" * 40)["commit"], "b" * 40)
        with self.assertRaises(release.Refused):
            self.resolve(sha="b" * 40, scheduled=False, ci_state=mock.Mock(return_value="its CI run is still going"))
        released = mock.Mock(return_value=mock.Mock(returncode=0))
        self.assertEqual(self.resolve(sha="b" * 40, run=released)["due"], "false")

    def test_the_newest_green_commit_passes_by_a_tip_still_in_ci_and_one_whose_workflows_differ(self):
        line = mock.Mock(return_value="e\nd\nc\nb")
        states = {"e": "its CI run is still going", "d": "its Comments run is still going", "c": None, "b": None}
        trees = answers("main\nmain\nmain\nolder\nmain\n")
        with mock.patch.multiple(release, git=line, run=trees, ci_state=mock.Mock(side_effect=lambda repo, sha: states[sha])):
            self.assertEqual(release.newest_green("owner/repo", "a"), "b")
        line.assert_called_once_with("rev-list", "--first-parent", "origin/main", "^a")
        self.assertEqual(trees.call_args.kwargs["data"], "origin/main:.github/workflows\ne:.github/workflows\nd:.github/workflows\n"
                         "c:.github/workflows\nb:.github/workflows\n")
        with mock.patch.multiple(release, git=mock.Mock(return_value="e"), run=answers("main\nmain\n"),
                                 ci_state=mock.Mock(return_value="its CI run has not started")):
            self.assertIsNone(release.newest_green("owner/repo", None))

    def test_a_nightly_yields_to_another_nightly_run_going_and_a_dry_run_does_not(self):
        going, feed = mock.Mock(return_value=41), mock.Mock()
        self.assertEqual(self.resolve(nightly_going=going, read_feed=feed)["due"], "false")
        feed.assert_not_called()
        self.assertEqual(self.resolve(draft=True, nightly_going=going)["due"], "true")

    def test_any_other_nightly_release_run_going_counts_and_dry_runs_and_other_channels_do_not(self):
        def found(number, title="Release nightly"):
            return {"id": 1000 + number, "run_number": number, "display_title": title}

        busy = json.dumps({"workflow_runs": [found(44), found(43, "Release nightly (dry run)"),
                                             found(42, "Release stable"), found(40)]})
        alone = json.dumps({"workflow_runs": [found(44), found(43, "Release nightly (dry run)")]})
        with mock.patch.dict(os.environ, {"GITHUB_RUN_ID": "1040"}):
            with mock.patch.object(release, "run", answers(busy)):
                self.assertEqual(release.nightly_going("owner/repo"), 44)
        with mock.patch.dict(os.environ, {"GITHUB_RUN_ID": "1044"}):
            with mock.patch.object(release, "run", answers(busy)):
                self.assertEqual(release.nightly_going("owner/repo"), 40)
            with mock.patch.object(release, "run", answers(alone)):
                self.assertIsNone(release.nightly_going("owner/repo"))

    def test_a_commit_whose_workflows_differ_from_mains_is_refused_before_anything_is_built(self):
        with mock.patch.object(release, "git", return_value=".github/workflows/release.yml"):
            with self.assertRaisesRegex(release.Refused, "workflows differ from main's"):
                release.same_workflows("b" * 40)
        with mock.patch.object(release, "git", return_value=""):
            release.same_workflows("b" * 40)

    def test_only_a_stable_candidate_is_prepared(self):
        for channel in ("nightly", "preview"):
            args = argparse.Namespace(channel=channel, sha="", version="", scheduled=False, draft=False, prepare=True,
                                      github_output=False)
            with mock.patch.multiple(release, repository=mock.DEFAULT, git=mock.DEFAULT, outputs=mock.DEFAULT):
                with self.assertRaisesRegex(release.Refused, "Only a stable candidate is prepared"):
                    release.resolve(args)
                release.repository.assert_not_called()


class Finalize(unittest.TestCase):
    def finalize(self, main, pushes, moved=True):
        calls = []
        heads = iter(range(100))

        def git(*args, **_):
            if args[0] == "show":
                return main
            if args[:2] == ("rev-parse", "origin/main"):
                return f"moved-{next(heads)}" if moved else "base"
            return "base"

        def run(command, **_):
            calls.append(command)
            return mock.Mock(returncode=pushes.pop(0), stderr="GH013: Repository rule violations found for refs/heads/main.")

        with mock.patch.multiple(release, git=mock.Mock(side_effect=git), run=run, stamp=mock.DEFAULT):
            result = release.finalize(argparse.Namespace(version="1.4.4"))
            return result, calls, release.stamp.call_count

    def test_an_older_release_never_sets_mains_workspace_version_back(self):
        for current in ("1.4.4", "1.4.5"):
            result, calls, stamped = self.finalize(f'[workspace.package]\nversion = "{current}"\n', [])
            self.assertEqual((result, calls, stamped), (0, [], 0))

    def test_the_app_pushes_the_bump_to_main_stamping_again_when_main_moved_first(self):
        result, calls, stamped = self.finalize('[workspace.package]\nversion = "1.4.3"\n', [1, 0])
        pushes = [command for command in calls if command[:2] == ["git", "push"]]
        self.assertEqual((result, len(pushes), stamped), (0, 2, 2))
        self.assertEqual(pushes[0][-2:], ["origin", "HEAD:main"])
        with self.assertRaisesRegex(release.Refused, "Main moved during each of 5 attempts"):
            self.finalize('[workspace.package]\nversion = "1.4.3"\n', [1] * 5)

    def test_a_push_main_refused_without_moving_is_named_with_gits_reason(self):
        with self.assertRaisesRegex(release.Refused, "main had not moved: GH013: Repository rule violations"):
            self.finalize('[workspace.package]\nversion = "1.4.3"\n', [1], moved=False)


def model_reply(chosen, finished="stop"):
    text = chosen if isinstance(chosen, str) else json.dumps(chosen)
    return {"choices": [{"index": 0, "finish_reason": finished, "message": {"role": "assistant", "content": text}}]}


class Notes(unittest.TestCase):
    def test_a_summary_keeps_highlights_citing_this_releases_pulls_and_counts_the_other_visible_ones(self):
        chosen = {"highlights": [{"label": "Fog of war.", "detail": "The map hides what a country cannot see.", "pulls": [12, 10]}],
                  "others": [11, 12, 13]}
        self.assertEqual(release.read_highlights(model_reply(chosen), [10, 11, 12, 13, 14]), (
            [{"label": "Fog of war", "detail": "The map hides what a country cannot see.", "pulls": [10, 12]}], 2))
        many = {"highlights": [{"label": f"Feature {n}", "detail": "Works.", "pulls": [n]} for n in range(1, 10)], "others": []}
        highlights, more = release.read_highlights(model_reply(many), range(1, 10))
        self.assertEqual((len(highlights), more), (release.HIGHLIGHTS, 2))

    def test_a_reply_that_is_cut_short_malformed_or_cites_another_releases_pulls_writes_no_summary(self):
        cited = {"highlights": [{"label": "Feature", "detail": "Works.", "pulls": [99]}], "others": []}
        for reply, reason in ((model_reply(cited, "length"), "ended length"), (model_reply("{\"highlights\": ["), "not JSON"),
                              (model_reply(cited), "lacks"), (model_reply({"highlights": [], "others": [99]}), "not this release's"),
                              (model_reply({"highlights": [{"label": "", "detail": "Works.", "pulls": [1]}]}), "lacks"),
                              ([], "not a chat completion"), ({"choices": [None]}, "not a chat completion"),
                              ({"choices": [{"finish_reason": "stop", "message": {"content": json.dumps({"highlights": [], "others": []}),
                                                                                  "refusal": "I can't help with that."}}]}, "refused")):
            with self.assertRaisesRegex(release.Refused, reason):
                release.read_highlights(reply, [1, 2])
        with mock.patch.object(release, "urllib") as network, mock.patch.dict(os.environ, {"GROQ_API_KEY": ""}):
            self.assertIsNone(release.summarize([{"number": 1, "title": "Fix", "body": ""}]))
            network.request.urlopen.assert_not_called()
        nested = mock.MagicMock()
        nested.__enter__.return_value = io.BytesIO(json.dumps(model_reply("[" * 100_000)).encode())
        for failure in (OSError("HTTP Error 500"), http.client.IncompleteRead(b"{"), None):
            failed = mock.Mock(side_effect=failure, return_value=nested)
            with mock.patch.object(release.urllib.request, "urlopen", failed), mock.patch.dict(os.environ, {"GROQ_API_KEY": "key"}):
                self.assertIsNone(release.summarize([{"number": 1, "title": "Fix", "body": ""}]))
        self.assertEqual(json.loads(failed.call_args.args[0].data)["reasoning_effort"], "high")

    def test_the_page_lists_every_pull_beneath_the_summary_or_alone_without_one(self):
        notes = {"version": "1.4.3", "since": "1.4.2", "date": None, "changes": 3, "more": 1,
                 "highlights": [{"label": "Fog of war", "detail": "The map hides what a country cannot see.", "pulls": [10]}],
                 "pulls": [{"number": 10, "title": "Map: fog of war"}, {"number": 11, "title": "CI: faster"}]}
        page = release.render_notes(notes)
        self.assertIn("**v1.4.3** · 3 changes", page)
        self.assertIn("- **Fog of war.** The map hides what a country cannot see.", page)
        self.assertIn("### Plus 1 more fix or improvement.", page)
        self.assertLess(page.index("<summary>"), page.index("- CI: faster (#11)"))
        self.assertIn("- And 1 change made without a pull request.", page)
        plain = release.render_notes({**notes, "highlights": None, "more": None})
        self.assertNotIn("<details>", plain)
        self.assertIn("- Map: fog of war (#10)", plain)
        self.assertEqual(release.render_notes({**notes, "since": None, "changes": 0, "pulls": []}), "The first release on this channel.\n")
        quiet = {**notes, "highlights": [], "more": 0}
        self.assertNotIn("internal", release.render_notes(quiet), "a change without a pull request was never read, so it is not called internal")
        self.assertIn("these changes are internal", release.render_notes({**quiet, "changes": 2}))

    def test_the_feed_carries_this_release_then_the_newest_earlier_ones_it_replaces(self):
        entry = lambda version: {"version": version}
        earlier = [entry(f"1.4.3-nightly.202611{day:02}.{day}") for day in range(1, 13)]
        feed = {"releases": [entry("1.4.3-nightly.20261130.40"), *earlier]}
        carried = release.feed_history(entry("1.4.3-nightly.20261113.13"), feed)
        self.assertEqual([item["version"] for item in carried][:3],
                         ["1.4.3-nightly.20261113.13", "1.4.3-nightly.20261112.12", "1.4.3-nightly.20261111.11"])
        self.assertEqual(len(carried), release.HISTORY)
        self.assertEqual(release.feed_history(entry("1.4.3"), None), [entry("1.4.3")])


class TakenUp(unittest.TestCase):
    def test_a_release_whose_feed_failed_is_taken_up_only_with_the_installer_it_records(self):
        with tempfile.TemporaryDirectory() as temp:
            dist = Path(temp)
            name = "battleplan-1.4.3-x64-setup.exe"
            served = {name: b"signed", f"{name}.sig": b"sig",
                      "SHA256SUMS": f"{hashlib.sha256(b'signed').hexdigest()}  {name}".encode()}

            def download(repo, tag, wanted, into):
                (into / wanted).write_bytes(served[wanted])
                return into / wanted

            chosen = {"installer": {"name": name}}
            recorded = {"body": f"Notes.\nBuilt from commit {'b' * 40}. Signed installer SHA-256: {hashlib.sha256(b'signed').hexdigest()}.\n"}
            with mock.patch.object(release, "download", download):
                with self.assertRaisesRegex(release.Refused, "records no signed installer"):
                    release.taken_up("owner/repo", "v1.4.3", {"body": "Notes."}, chosen, dist, False)
                self.assertEqual(release.taken_up("owner/repo", "v1.4.3", recorded, chosen, dist, False)[0].read_bytes(), b"signed")
                with mock.patch.object(release, "authenticode_valid", return_value=False):
                    with self.assertRaisesRegex(release.Refused, "no valid Authenticode signature"):
                        release.taken_up("owner/repo", "v1.4.3", recorded, chosen, dist, True)
                served[name] = b"swapped"
                with self.assertRaisesRegex(release.Refused, "not the one its release records"):
                    release.taken_up("owner/repo", "v1.4.3", recorded, chosen, dist, False)
                with self.assertRaisesRegex(release.Refused, "has no notes.json"):
                    release.taken_notes("owner/repo", "v1.4.3", {"assets": [{"name": name}]}, dist)
                served["notes.json"] = b'{"version": "1.4.3"}'
                self.assertEqual(release.taken_notes("owner/repo", "v1.4.3", {"assets": [{"name": "notes.json"}]}, dist).read_bytes(),
                                 served["notes.json"])


if __name__ == "__main__":
    unittest.main()

