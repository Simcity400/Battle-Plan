import contextlib
import hashlib
import io
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace

sys.path.insert(0, str(Path(__file__).resolve().parent))
import suite


def _write_shell_folder(folder, revision, *, record_name=suite.SHELL_RECORD, exe_bytes=b"fake-shell-exe", cargo_arguments=None, sha256=None, recorded_bytes=None):
    folder = Path(folder)
    folder.mkdir(parents=True, exist_ok=True)
    exe = folder / suite.APP_EXE
    exe.write_bytes(exe_bytes)
    args = suite.shell_cargo_arguments(shell=True) if cargo_arguments is None else cargo_arguments
    digest = hashlib.sha256(exe_bytes).hexdigest() if sha256 is None else sha256
    size = len(exe_bytes) if recorded_bytes is None else recorded_bytes
    record = {
        "source": "github-actions" if record_name == suite.SHELL_RECORD else "boat",
        "revision": revision,
        "cargo_arguments": args,
        "artifacts": [{
            "path": suite.APP_EXE,
            "sha256": digest,
            "bytes": size,
            "cargo_arguments": args,
        }],
    }
    (folder / record_name).write_text(json.dumps(record), encoding="utf-8")
    return exe


class CoresAndProfileTests(unittest.TestCase):
    def test_parse_cores_count_prefers_even_logical_cpus(self):
        parsed = suite.parse_cores("4", system_mask=0xFFFF)
        self.assertEqual(parsed["count"], 4)
        self.assertEqual(parsed["mask"], 0x55)
        self.assertEqual(parsed["cpus"], [0, 2, 4, 6])

    def test_parse_cores_fills_odd_after_even(self):
        parsed = suite.parse_cores("10", system_mask=0xFFFF)
        self.assertEqual(parsed["count"], 10)
        self.assertEqual(parsed["cpus"], [0, 2, 4, 6, 8, 10, 12, 14, 1, 3])
        self.assertEqual(parsed["mask"].bit_count(), 10)

    def test_parse_cores_hex_mask(self):
        parsed = suite.parse_cores("0x55", system_mask=0xFFFF)
        self.assertEqual(parsed["mask"], 0x55)
        self.assertEqual(parsed["count"], 4)
        self.assertEqual(parsed["cpus"], [0, 2, 4, 6])

    def test_parse_cores_rejects_out_of_range(self):
        with self.assertRaises(ValueError):
            suite.parse_cores("0", system_mask=0xFFFF)
        with self.assertRaises(ValueError):
            suite.parse_cores("17", system_mask=0xFFFF)
        with self.assertRaises(ValueError):
            suite.parse_cores("0x10000", system_mask=0xFFFF)




    def test_run_build_ui_guards_ownership_links_and_failed_builds(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            unmarked = root / "user-data"
            unmarked.mkdir()
            precious = unmarked / "campaign.battleplan"
            precious.write_text("keep", encoding="utf-8")
            with self.assertRaisesRegex(RuntimeError, "battleplan-ui-bundle"):
                suite.run_build_ui(unmarked, runner=lambda *a, **k: SimpleNamespace(returncode=0))
            self.assertEqual(precious.read_text(encoding="utf-8"), "keep")

            if os.name == "nt":
                junction = root / "link-to-user-data"
                subprocess.run(["cmd", "/c", "mklink", "/J", str(junction), str(unmarked)], check=True, capture_output=True)
                try:
                    with self.assertRaisesRegex(RuntimeError, "symlink or junction"):
                        suite.validate_ui_output(junction)
                    with self.assertRaisesRegex(RuntimeError, "symlink or junction"):
                        suite.validate_ui_output(junction / "nested")
                finally:
                    subprocess.run(["cmd", "/c", "rmdir", str(junction)], check=False, capture_output=True)

            empty_dir = root / "empty-out"
            empty_dir.mkdir()
            self.assertEqual(suite.run_build_ui(empty_dir, runner=lambda *a, **k: SimpleNamespace(returncode=1)), 1)
            self.assertFalse((empty_dir / suite.UI_BUNDLE_MARKER).exists(), "failed first build must not mark an empty folder")

            def fake_ok(cmd, cwd=None, env=None):
                out_dir = Path(cmd[cmd.index("--outDir") + 1])
                (out_dir / "index.html").write_text("v1", encoding="utf-8")
                return SimpleNamespace(returncode=0)

            self.assertEqual(suite.run_build_ui(empty_dir, runner=fake_ok), 0)
            self.assertTrue((empty_dir / suite.UI_BUNDLE_MARKER).is_file())
            self.assertEqual((empty_dir / "index.html").read_text(encoding="utf-8"), "v1")

            self.assertEqual(suite.run_build_ui(empty_dir, runner=lambda *a, **k: SimpleNamespace(returncode=1)), 1)
            self.assertTrue((empty_dir / suite.UI_BUNDLE_MARKER).is_file(), "failed rebuild must keep marker and ownership")
            self.assertEqual((empty_dir / "index.html").read_text(encoding="utf-8"), "v1")

            def fake_v2(cmd, cwd=None, env=None):
                out_dir = Path(cmd[cmd.index("--outDir") + 1])
                (out_dir / "index.html").write_text("v2", encoding="utf-8")
                return SimpleNamespace(returncode=0)

            self.assertEqual(suite.run_build_ui(empty_dir, runner=fake_v2), 0)
            self.assertTrue((empty_dir / suite.UI_BUNDLE_MARKER).is_file())
            self.assertEqual((empty_dir / "index.html").read_text(encoding="utf-8"), "v2")

            raced_dir = root / "raced-empty"
            raced_dir.mkdir()
            def fake_race_runner(cmd, cwd=None, env=None):
                out_dir = Path(cmd[cmd.index("--outDir") + 1])
                (out_dir / "index.html").write_text("built", encoding="utf-8")
                (raced_dir / "suddenly-appeared.txt").write_text("precious", encoding="utf-8")
                return SimpleNamespace(returncode=0)

            with self.assertRaisesRegex(RuntimeError, "battleplan-ui-bundle"):
                suite.run_build_ui(raced_dir, runner=fake_race_runner)
            self.assertEqual((raced_dir / "suddenly-appeared.txt").read_text(encoding="utf-8"), "precious")

            raced_post_validate = root / "raced-post-validate"
            raced_post_validate.mkdir()
            orig_validate = suite.validate_ui_output
            calls = [0]
            def inject_after_second_validate(path):
                res = orig_validate(path)
                calls[0] += 1
                if calls[0] == 2:
                    (res / "late-file.txt").write_text("must-survive", encoding="utf-8")
                return res
            suite.validate_ui_output = inject_after_second_validate
            try:
                with self.assertRaisesRegex(RuntimeError, "late-file.txt"):
                    suite.run_build_ui(raced_post_validate, runner=fake_ok)
            finally:
                suite.validate_ui_output = orig_validate
            backups = list(root.glob(f".{raced_post_validate.name}.old-*"))
            self.assertEqual(len(backups), 1)
            self.assertEqual((backups[0] / "late-file.txt").read_text(encoding="utf-8"), "must-survive")

            interrupted = root / "interrupted-target"
            leftover_old = root / f".{interrupted.name}.old-123"
            leftover_old.mkdir()
            self.assertEqual(suite.find_ui_leftovers(interrupted), [leftover_old])


class HostedShellTests(unittest.TestCase):
    SHA = "a" * 40
    OTHER_SHA = "b" * 40

    def _result(self, returncode=0, stdout="", stderr=""):
        return SimpleNamespace(returncode=returncode, stdout=stdout, stderr=stderr)

    def test_parse_github_repo_handles_https_and_ssh(self):
        self.assertEqual(suite.parse_github_repo("https://github.com/Org/Repo.git"), "Org/Repo")
        self.assertEqual(suite.parse_github_repo("https://github.com/Org/Repo"), "Org/Repo")
        self.assertEqual(suite.parse_github_repo("git@github.com:Org/Repo.git"), "Org/Repo")
        self.assertEqual(suite.parse_github_repo("ssh://git@github.com/Org/Repo.git"), "Org/Repo")
        self.assertIsNone(suite.parse_github_repo("https://example.invalid/Org/Repo.git"))

    def test_shell_build_accepts_hosted_and_boat_records_and_rejects_mismatches(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            hosted = root / "hosted"
            _write_shell_folder(hosted, self.SHA, record_name=suite.SHELL_RECORD)
            info = suite.shell_build(hosted, expected_revision=self.SHA, verify_hash=True)
            self.assertEqual(info["revision"], self.SHA)
            self.assertIn(suite.SHELL_FEATURE, ",".join(info["cargoArguments"]))

            boat = root / "boat"
            _write_shell_folder(boat, self.SHA, record_name=suite.BOAT_RECORD)
            self.assertEqual(suite.shell_build(boat, expected_revision=self.SHA, verify_hash=True)["revision"], self.SHA)

            with self.assertRaisesRegex(RuntimeError, "not " + self.OTHER_SHA):
                suite.shell_build(hosted, expected_revision=self.OTHER_SHA, verify_hash=True)

            bad_hash = root / "bad-hash"
            _write_shell_folder(bad_hash, self.SHA, sha256="0" * 64)
            with self.assertRaisesRegex(RuntimeError, "SHA-256"):
                suite.shell_build(bad_hash, expected_revision=self.SHA, verify_hash=True)

            bad_size = root / "bad-size"
            _write_shell_folder(bad_size, self.SHA, recorded_bytes=999)
            with self.assertRaisesRegex(RuntimeError, "bytes, expected 999"):
                suite.shell_build(bad_size, expected_revision=self.SHA, verify_hash=True)

            no_feature = root / "no-feature"
            _write_shell_folder(no_feature, self.SHA, cargo_arguments=suite.shell_cargo_arguments(shell=False))
            with self.assertRaisesRegex(RuntimeError, "not a measuring shell"):
                suite.shell_build(no_feature, expected_revision=self.SHA, verify_hash=True)

            with self.assertRaisesRegex(RuntimeError, "holds no shell build record"):
                suite.shell_build(boat, expected_revision=self.SHA, verify_hash=True, require_hosted=True)

            both = root / "both-records"
            _write_shell_folder(both, self.OTHER_SHA, record_name=suite.SHELL_RECORD, exe_bytes=b"old-hosted-exe")
            _write_shell_folder(both, self.SHA, record_name=suite.BOAT_RECORD, exe_bytes=b"new-boat-exe")
            self.assertEqual(suite.shell_build(both)["revision"], self.SHA)
            (both / suite.APP_EXE).write_bytes(b"unknown-exe")
            with self.assertRaisesRegex(RuntimeError, "neither matches"):
                suite.shell_build(both)

    def test_shell_destination_uses_shared_shell_for_main_and_requires_out_for_branch(self):
        with tempfile.TemporaryDirectory() as tmp:
            gate = Path(tmp) / "gate"

            def on_main_runner(cmd, **kwargs):
                return self._result(0)

            dest = suite.shell_destination(self.SHA, gate_dir=gate, env={}, runner=on_main_runner)
            self.assertEqual(dest, gate / "shared-shell" / self.SHA[:10])

            dest_env = suite.shell_destination(self.SHA, env={suite.GATE_DIR_ENV: str(gate)}, runner=on_main_runner)
            self.assertEqual(dest_env, gate / "shared-shell" / self.SHA[:10])

            def off_main_runner(cmd, **kwargs):
                if "merge-base" in cmd:
                    return self._result(1)
                return self._result(0)

            with self.assertRaisesRegex(RuntimeError, "not on origin/main"):
                suite.shell_destination(self.SHA, gate_dir=gate, env={}, runner=off_main_runner)

            explicit = Path(tmp) / "branch-shell"
            self.assertEqual(
                suite.shell_destination(self.SHA, out=explicit, env={suite.GATE_DIR_ENV: str(gate)}, runner=off_main_runner),
                explicit,
            )

    def test_fetch_shell_reuses_valid_shell_without_calling_gh(self):
        with tempfile.TemporaryDirectory() as tmp:
            gate = Path(tmp) / "gate"
            target = gate / "shared-shell" / self.SHA[:10]
            _write_shell_folder(target, self.SHA)
            calls = []

            def runner(cmd, **kwargs):
                calls.append(cmd)
                if cmd[0] == "gh":
                    raise AssertionError("gh must not be called when a verified shell is already present")
                if "rev-parse" in cmd:
                    return self._result(0, stdout=self.SHA + "\n")
                return self._result(0)

            stdout = io.StringIO()
            with contextlib.redirect_stdout(stdout):
                code = suite.run_fetch_shell(commit="origin/main", gate_dir=gate, env={}, runner=runner)
            self.assertEqual(code, 0)
            self.assertIn(str(target), stdout.getvalue())
            self.assertFalse(any(cmd[0] == "gh" for cmd in calls))

    def test_fetch_shell_downloads_verifies_and_ignores_expired_failed_and_foreign_artifacts(self):
        with tempfile.TemporaryDirectory() as tmp:
            target = Path(tmp) / "out-shell"
            _write_shell_folder(target, self.OTHER_SHA)
            artifact_name = suite.shell_artifact_name(self.SHA)
            downloaded = []

            def runner(cmd, **kwargs):
                if cmd[0] == "git" and "rev-parse" in cmd:
                    return self._result(0, stdout=self.SHA + "\n")
                if cmd[0] == "git" and "remote" in cmd:
                    return self._result(0, stdout="https://github.com/Owner/Repo.git\n")
                if cmd[:2] == ["gh", "api"] and "actions/artifacts?" in cmd[2]:
                    payload = {"artifacts": [
                        {"created_at": "2026-11-27T05:00:00Z", "expired": True, "workflow_run": {"id": 10, "repository_id": 1, "head_repository_id": 1}},
                        {"created_at": "2026-11-27T04:00:00Z", "expired": False, "workflow_run": {"id": 11, "repository_id": 1, "head_repository_id": 2}},
                        {"created_at": "2026-11-27T03:00:00Z", "expired": False, "workflow_run": {"id": 12, "repository_id": 1, "head_repository_id": 1}},
                        {"created_at": "2026-11-27T02:00:00Z", "expired": False, "workflow_run": {"id": 13, "repository_id": 1, "head_repository_id": 1}},
                    ]}
                    return self._result(0, stdout=json.dumps(payload))
                if cmd[:2] == ["gh", "api"] and cmd[2].endswith("/actions/runs/12"):
                    return self._result(0, stdout=json.dumps({"path": f".github/workflows/{suite.SHELL_WORKFLOW}", "status": "completed", "conclusion": "failure"}))
                if cmd[:2] == ["gh", "api"] and cmd[2].endswith("/actions/runs/13"):
                    return self._result(0, stdout=json.dumps({"path": f".github/workflows/{suite.SHELL_WORKFLOW}", "status": "completed", "conclusion": "success"}))
                if cmd[:3] == ["gh", "run", "download"]:
                    downloaded.append(cmd)
                    self.assertEqual(cmd[3], "13")
                    self.assertEqual(cmd[cmd.index("--name") + 1], artifact_name)
                    dest = Path(cmd[cmd.index("--dir") + 1])
                    _write_shell_folder(dest, self.SHA, exe_bytes=b"new-hosted-shell")
                    return self._result(0)
                raise AssertionError(f"unexpected command: {cmd}")

            stdout = io.StringIO()
            with contextlib.redirect_stdout(stdout):
                code = suite.run_fetch_shell(commit=self.SHA, out=target, env={}, runner=runner)
            self.assertEqual(code, 0)
            self.assertEqual(len(downloaded), 1)
            self.assertEqual(suite.shell_build(target, expected_revision=self.SHA, verify_hash=True)["revision"], self.SHA)
            self.assertEqual((target / suite.APP_EXE).read_bytes(), b"new-hosted-shell")

    def test_fetch_shell_refuses_to_overwrite_unmarked_boat_or_mixed_folder(self):
        with tempfile.TemporaryDirectory() as tmp:
            target = Path(tmp) / "user-stuff"
            target.mkdir()
            (target / "keep.txt").write_text("keep", encoding="utf-8")

            def runner(cmd, **kwargs):
                if cmd[0] == "git" and "rev-parse" in cmd:
                    return self._result(0, stdout=self.SHA + "\n")
                raise AssertionError(f"should fail before calling {cmd}")

            with self.assertRaisesRegex(RuntimeError, "dedicated hosted measuring shell"):
                suite.run_fetch_shell(commit=self.SHA, out=target, env={}, runner=runner)
            self.assertEqual((target / "keep.txt").read_text(encoding="utf-8"), "keep")

            boat_dir = Path(tmp) / "boat-output"
            _write_shell_folder(boat_dir, self.SHA, record_name=suite.BOAT_RECORD)
            (boat_dir / "recorder.exe").write_bytes(b"keep-recorder")
            with self.assertRaisesRegex(RuntimeError, "dedicated hosted measuring shell"):
                suite.run_fetch_shell(commit=self.SHA, out=boat_dir, env={}, runner=runner)
            self.assertEqual((boat_dir / "recorder.exe").read_bytes(), b"keep-recorder")

            mixed_dir = Path(tmp) / "mixed-shell"
            _write_shell_folder(mixed_dir, self.OTHER_SHA, record_name=suite.SHELL_RECORD)
            _write_shell_folder(mixed_dir, self.OTHER_SHA, record_name=suite.BOAT_RECORD)
            (mixed_dir / "recorder.exe").write_bytes(b"keep-mixed-recorder")
            with self.assertRaisesRegex(RuntimeError, "dedicated hosted measuring shell"):
                suite.run_fetch_shell(commit=self.SHA, out=mixed_dir, env={}, runner=runner)
            self.assertEqual((mixed_dir / "recorder.exe").read_bytes(), b"keep-mixed-recorder")

    def test_fetch_shell_reports_active_run_by_title_only_and_ignores_branch_head_sha(self):
        with tempfile.TemporaryDirectory() as tmp:
            target = Path(tmp) / "out"
            dispatched = []

            def runner(cmd, **kwargs):
                if cmd[0] == "git" and "rev-parse" in cmd:
                    return self._result(0, stdout=self.SHA + "\n")
                if cmd[0] == "git" and "remote" in cmd:
                    return self._result(0, stdout="https://github.com/Owner/Repo.git\n")
                if cmd[:2] == ["gh", "api"] and "actions/artifacts?" in cmd[2]:
                    return self._result(0, stdout=json.dumps({"artifacts": []}))
                if cmd[:2] == ["gh", "api"] and f"workflows/{suite.SHELL_WORKFLOW}/runs" in cmd[2]:
                    return self._result(0, stdout=json.dumps({"workflow_runs": [
                        {"id": 99, "status": "in_progress", "head_sha": self.OTHER_SHA, "display_title": f"{suite.SHELL_RUN_NAME_PREFIX}{self.SHA}", "html_url": "https://github.com/Owner/Repo/actions/runs/99"},
                    ]}))
                if cmd[:3] == ["gh", "workflow", "run"]:
                    dispatched.append(cmd)
                    return self._result(0)
                raise AssertionError(f"unexpected command: {cmd}")

            stderr = io.StringIO()
            with contextlib.redirect_stderr(stderr):
                code = suite.run_fetch_shell(commit=self.SHA, out=target, dispatch=True, env={}, runner=runner)
            self.assertEqual(code, 1)
            self.assertEqual(dispatched, [])
            self.assertIn("already in_progress", stderr.getvalue())
            self.assertIn("actions/runs/99", stderr.getvalue())

            def branch_run_on_main_runner(cmd, **kwargs):
                if cmd[:2] == ["gh", "api"] and f"workflows/{suite.SHELL_WORKFLOW}/runs" in cmd[2]:
                    return self._result(0, stdout=json.dumps({"workflow_runs": [
                        {"id": 100, "status": "in_progress", "head_sha": self.SHA, "display_title": f"{suite.SHELL_RUN_NAME_PREFIX}{self.OTHER_SHA}", "html_url": "https://github.com/Owner/Repo/actions/runs/100"},
                    ]}))
                if cmd[:2] == ["gh", "api"] and cmd[2].endswith(f"/commits/{self.SHA}"):
                    return self._result(0, stdout=json.dumps({"sha": self.SHA}))
                return runner(cmd, **kwargs)

            stderr = io.StringIO()
            with contextlib.redirect_stderr(stderr):
                code = suite.run_fetch_shell(commit=self.SHA, out=target, dispatch=True, env={}, runner=branch_run_on_main_runner)
            self.assertEqual(code, 1)
            self.assertEqual(len(dispatched), 1)

    def test_fetch_shell_missing_hint_never_suggests_boat_into_shared_shell_or_mismatched_head(self):
        with tempfile.TemporaryDirectory() as tmp:
            gate = Path(tmp) / "gate"

            def make_runner(head_sha):
                def runner(cmd, **kwargs):
                    if cmd[0] == "git" and "rev-parse" in cmd:
                        if "HEAD^{commit}" in cmd:
                            return self._result(0, stdout=head_sha + "\n")
                        return self._result(0, stdout=self.SHA + "\n")
                    if cmd[0] == "git" and "merge-base" in cmd:
                        return self._result(0)
                    if cmd[0] == "git" and "remote" in cmd:
                        return self._result(0, stdout="https://github.com/Owner/Repo.git\n")
                    if cmd[:2] == ["gh", "api"] and "actions/artifacts?" in cmd[2]:
                        return self._result(0, stdout=json.dumps({"artifacts": []}))
                    if cmd[:2] == ["gh", "api"] and f"workflows/{suite.SHELL_WORKFLOW}/runs" in cmd[2]:
                        return self._result(0, stdout=json.dumps({"workflow_runs": []}))
                    raise AssertionError(f"unexpected command: {cmd}")
                return runner

            stderr = io.StringIO()
            with contextlib.redirect_stderr(stderr):
                code = suite.run_fetch_shell(commit="origin/main", gate_dir=gate, env={}, runner=make_runner(self.OTHER_SHA))
            self.assertEqual(code, 1)
            self.assertNotIn("build --shell", stderr.getvalue())

            stderr = io.StringIO()
            with contextlib.redirect_stderr(stderr):
                code = suite.run_fetch_shell(commit="origin/main", gate_dir=gate, env={}, runner=make_runner(self.SHA))
            self.assertEqual(code, 1)
            self.assertIn("build --shell --out <separate-dir>", stderr.getvalue())
            self.assertNotIn("shared-shell", stderr.getvalue())

    def test_fetch_shell_dispatches_pushed_commit_and_refuses_unpushed_commit(self):
        with tempfile.TemporaryDirectory() as tmp:
            target = Path(tmp) / "out"
            dispatched = []

            def pushed_runner(cmd, **kwargs):
                if cmd[0] == "git" and "rev-parse" in cmd:
                    return self._result(0, stdout=self.SHA + "\n")
                if cmd[0] == "git" and "remote" in cmd:
                    return self._result(0, stdout="https://github.com/Owner/Repo.git\n")
                if cmd[:2] == ["gh", "api"] and "actions/artifacts?" in cmd[2]:
                    return self._result(0, stdout=json.dumps({"artifacts": []}))
                if cmd[:2] == ["gh", "api"] and f"workflows/{suite.SHELL_WORKFLOW}/runs" in cmd[2]:
                    return self._result(0, stdout=json.dumps({"workflow_runs": []}))
                if cmd[:2] == ["gh", "api"] and cmd[2].endswith(f"/commits/{self.SHA}"):
                    return self._result(0, stdout=json.dumps({"sha": self.SHA}))
                if cmd[:3] == ["gh", "workflow", "run"]:
                    dispatched.append(cmd)
                    return self._result(0)
                raise AssertionError(f"unexpected command: {cmd}")

            stderr = io.StringIO()
            with contextlib.redirect_stderr(stderr):
                code = suite.run_fetch_shell(commit=self.SHA, out=target, dispatch=True, env={}, runner=pushed_runner)
            self.assertEqual(code, 1)
            self.assertEqual(dispatched, [["gh", "workflow", "run", suite.SHELL_WORKFLOW, "--repo", "Owner/Repo", "-f", f"ref={self.SHA}"]])
            self.assertIn(f"Dispatched {suite.SHELL_WORKFLOW}", stderr.getvalue())
            self.assertIn(f"--commit {self.SHA}", stderr.getvalue())

            def unpushed_runner(cmd, **kwargs):
                if cmd[:2] == ["gh", "api"] and cmd[2].endswith(f"/commits/{self.SHA}"):
                    return self._result(1, stderr="HTTP 422: No commit found")
                return pushed_runner(cmd, **kwargs)

            with self.assertRaisesRegex(RuntimeError, "push your branch before dispatching"):
                suite.run_fetch_shell(commit=self.SHA, out=target, dispatch=True, env={}, runner=unpushed_runner)

    def test_build_shell_prints_hosted_note_and_shares_cargo_arguments(self):
        with tempfile.TemporaryDirectory() as tmp:
            args = suite.parse(["build", "--shell", "--out", tmp])
            self.assertTrue(args.shell)

            seen = []
            stderr = io.StringIO()
            with contextlib.redirect_stderr(stderr):
                code = suite.run_build_boat(tmp, shell=True, runner=lambda cmd, cwd=None: seen.append(cmd) or self._result(0))
            self.assertEqual(code, 0)
            self.assertEqual(seen[0][-5:], suite.shell_cargo_arguments(shell=True)[1:])
            self.assertIn("suite.py shell", stderr.getvalue())

            stderr_app = io.StringIO()
            with contextlib.redirect_stderr(stderr_app):
                code = suite.run_build_boat(tmp, shell=False, runner=lambda cmd, cwd=None: self._result(0))
            self.assertEqual(code, 0)
            self.assertEqual(stderr_app.getvalue(), "")

    def test_build_hosted_shell_requires_github_actions_and_stages_record_and_prunes(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source_root = root / "source-checkout"
            source_root.mkdir()
            out_dir = root / "dist"
            target_dir = root / "target"
            (target_dir / "ci" / ".fingerprint").mkdir(parents=True)
            (target_dir / "release" / "deps").mkdir(parents=True)
            (target_dir / "release" / ".fingerprint").mkdir(parents=True)
            used, unused = "0123456789abcdef", "fedcba9876543210"
            (target_dir / "release" / "deps" / f"libused-{used}.rlib").write_bytes(b"used")
            (target_dir / "release" / "deps" / f"libold-{unused}.rlib").write_bytes(b"old")

            with self.assertRaisesRegex(RuntimeError, "GITHUB_ACTIONS=true"):
                suite.run_build_hosted_shell(out_dir, target=target_dir, source_root=source_root, env={})

            seen_cargo = []
            seen_cwd = []

            def runner(cmd, **kwargs):
                if cmd[0] == "git" and "rev-parse" in cmd:
                    self.assertEqual(Path(cmd[2]).resolve(), source_root.resolve())
                    return self._result(0, stdout=self.SHA + "\n")
                if cmd[0] == "cargo":
                    seen_cargo.append(cmd)
                    seen_cwd.append(Path(kwargs["cwd"]).resolve())
                    self.assertTrue((source_root / "ui" / "dist" / "index.html").is_file())
                    exe = target_dir / "release" / suite.APP_EXE
                    exe.write_bytes(b"compiled-shell-binary")
                    lines = [
                        json.dumps({"reason": "compiler-artifact", "fresh": False, "filenames": [str(target_dir / "release" / "deps" / f"libused-{used}.rlib")]}),
                        json.dumps({"reason": "compiler-artifact", "fresh": False, "executable": str(exe), "filenames": [str(exe)]}),
                    ]
                    return self._result(0, stdout="\n".join(lines) + "\n")
                raise AssertionError(f"unexpected command: {cmd}")

            code = suite.run_build_hosted_shell(
                out_dir,
                target=target_dir,
                source_root=source_root,
                env={"GITHUB_ACTIONS": "true", "GITHUB_RUN_ID": "4242"},
                runner=runner,
            )
            self.assertEqual(code, 0)
            self.assertEqual(seen_cargo[0], ["cargo", *suite.shell_cargo_arguments(shell=True), "--message-format=json-render-diagnostics"])
            self.assertEqual(seen_cwd[0], source_root.resolve())
            self.assertFalse((source_root / "ui" / "dist").exists())
            info = suite.shell_build(out_dir, expected_revision=self.SHA, verify_hash=True, require_hosted=True)
            self.assertEqual(info["revision"], self.SHA)
            self.assertFalse((target_dir / "ci").exists())
            self.assertFalse((target_dir / "release" / suite.APP_EXE).exists())
            self.assertTrue((target_dir / "release" / "deps" / f"libused-{used}.rlib").is_file())
            self.assertFalse((target_dir / "release" / "deps" / f"libold-{unused}.rlib").exists())
            self.assertEqual((target_dir / "ci-built-from").read_text(encoding="utf-8").strip(), self.SHA)


if __name__ == "__main__":
    unittest.main()
