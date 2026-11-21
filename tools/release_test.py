import argparse
import base64
import datetime
import hashlib
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
        return json.dumps({"workflow_runs": [dict(zip(keys, run)) for run in found]})

    def test_main_is_green_once_the_latest_run_of_each_workflow_its_push_started_settled_green(self):
        settled = self.runs((1, "Release candidate", 3, 1, "completed", "failure"),
                            (1, "Release candidate", 3, 2, "completed", "success"),
                            (2, "Comments", 8, 1, "completed", "skipped"))
        with mock.patch.object(release, "run", answers(settled)):
            self.assertIsNone(release.ci_state("owner/repo", "b" * 40))
        with mock.patch.object(release, "run", answers(self.runs())):
            self.assertIsNone(release.ci_state("owner/repo", "b" * 40))
        with mock.patch.object(release, "run", answers(self.runs((2, "Comments", 8, 1, "in_progress", None)))):
            self.assertIn("Comments run is still going", release.ci_state("owner/repo", "b" * 40))
        with mock.patch.object(release, "run", answers(self.runs((1, "Release candidate", 4, 1, "completed", "failure")))):
            self.assertIn("Release candidate run ended failure", release.ci_state("owner/repo", "b" * 40))

    def test_the_newest_kept_candidate_mains_ci_built_is_taken_and_an_unusable_newer_one_passed_by(self):
        def artifact(expired, created, run):
            return {"expired": expired, "created_at": created, "workflow_run": {"id": run}}

        listing = json.dumps({"artifacts": [artifact(False, "2026-11-20", 7), artifact(False, "2026-11-21", 9),
                                            artifact(True, "2026-11-19", 5)]})
        verdicts = {9: "the run that built it ended failure", 7: None}
        with mock.patch.multiple(release, run=answers(listing),
                                 produced=mock.Mock(side_effect=lambda repo, found, channel: verdicts[found["workflow_run"]["id"]])):
            self.assertEqual(release.find_candidate("owner/repo", "candidate-nightly-b", "nightly"), (7, None))
        expired = json.dumps({"artifacts": [artifact(True, "2026-11-19", 5)]})
        with mock.patch.object(release, "run", answers(expired)):
            self.assertIn("has expired", release.find_candidate("owner/repo", "candidate-nightly-b", "nightly")[1])
        with mock.patch.object(release, "run", answers(json.dumps({"artifacts": []}))):
            self.assertIn("no candidate", release.find_candidate("owner/repo", "candidate-nightly-b", "nightly")[1])

    def test_only_mains_candidate_workflow_or_the_release_build_job_produces_a_candidate(self):
        def artifact(branch="main", head=1):
            return {"workflow_run": {"id": 9, "repository_id": 1, "head_repository_id": head, "head_branch": branch}}

        def state(path, event, conclusion="success"):
            return json.dumps({"path": f".github/workflows/{path}", "event": event, "status": "completed", "conclusion": conclusion})

        self.assertIn("not built from this repository's main", release.produced("owner/repo", artifact(branch="feature"), "nightly"))
        self.assertIn("not built from this repository's main", release.produced("owner/repo", artifact(head=2), "nightly"))
        with mock.patch.object(release, "run", answers(state("release-candidate.yml", "push"))):
            self.assertIsNone(release.produced("owner/repo", artifact(), "nightly"))
        with mock.patch.object(release, "run", answers(state("release-candidate.yml", "pull_request"))):
            self.assertIn("does not build release candidates", release.produced("owner/repo", artifact(), "nightly"))
        with mock.patch.object(release, "run", answers(state("comments.yml", "push"))):
            self.assertIn("does not build release candidates", release.produced("owner/repo", artifact(), "nightly"))
        built = "Build / Windows x64 installer\tsuccess\nPublish\tfailure"
        with mock.patch.object(release, "run", answers(state("release.yml", "workflow_dispatch", "failure"), built)):
            self.assertIsNone(release.produced("owner/repo", artifact(), "stable"))
        with mock.patch.object(release, "run", answers(state("release.yml", "workflow_dispatch"))):
            self.assertIn("does not build release candidates", release.produced("owner/repo", artifact(), "nightly"))


class Resolve(unittest.TestCase):
    def resolve(self, draft=False, **changes):
        args = argparse.Namespace(channel="nightly", sha="", version="", scheduled=True, draft=draft, prepare=False,
                                  github_output=False)
        settings = dict(repository=mock.Mock(return_value="owner/repo"), git=mock.Mock(return_value=""),
                        on_main=mock.Mock(return_value="b" * 40),
                        read_feed=mock.Mock(return_value={"commit": "a" * 40, "version": "1.4.3-nightly.20261121.900"}),
                        run=mock.Mock(return_value=mock.Mock(returncode=1)),
                        ci_state=mock.Mock(return_value=None),
                        find_candidate=mock.Mock(return_value=(None, "the run that builds it is still going")),
                        outputs=mock.Mock())
        settings.update(changes)
        with mock.patch.multiple(release, **settings):
            release.resolve(args)
            return release.outputs.call_args[0][0]

    def test_a_scheduled_nightly_is_due_only_when_main_moved_its_ci_is_green_and_its_candidate_is_kept(self):
        built = mock.Mock(return_value=(9, None))
        self.assertEqual(self.resolve()["due"], "false")
        self.assertEqual(self.resolve(ci_state=mock.Mock(return_value="its Comments run is still going"),
                                      find_candidate=built)["due"], "false")
        unchanged = mock.Mock(return_value=mock.Mock(returncode=0))
        self.assertEqual(self.resolve(run=unchanged, find_candidate=built)["due"], "false")
        self.assertEqual(self.resolve(draft=True, run=unchanged, find_candidate=built)["due"], "true")
        with mock.patch.dict(os.environ, {"PAUSE_NIGHTLY": "true"}):
            self.assertEqual(self.resolve(find_candidate=built)["due"], "false")
        due = self.resolve(find_candidate=built)
        self.assertEqual((due["due"], due["commit"], due["artifact"], due["candidate_run"], due["build"]),
                         ("true", "b" * 40, "candidate-nightly-" + "b" * 40, "9", "false"))


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


if __name__ == "__main__":
    unittest.main()

