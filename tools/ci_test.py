"""Tests for tools/ci.py: python tools/ci_test.py"""

import importlib.util
import os
from pathlib import Path
import re
import unittest

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("ci", ROOT / "tools" / "ci.py")
ci = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(ci)

PIPELINE = """
[group.all]
os = "ubuntu-latest"
[tier.check]
budget = 60
[tier.main]
budget = 900
[toolchain.python]
version = ["python"]
[toolchain.rust]
version = ["rustc"]
files = ["Cargo.lock"]
[[job]]
name = "scan"
group = "all"
tier = "check"
toolchain = "python"
inputs = ["src/**"]
command = ["scan"]
[[job]]
name = "ratchet"
group = "all"
tier = "check"
toolchain = "python"
base = true
inputs = ["src/**"]
command = ["ratchet", "{base}"]
[[job]]
name = "per-file"
group = "all"
tier = "check"
kind = "files"
toolchain = "python"
inputs = ["src/**", "*.md"]
shared = ["tools/scan.py"]
command = ["scan", "{files}"]
[[job]]
name = "slow"
group = "all"
tier = "main"
toolchain = "python"
inputs = ["src/**"]
command = ["slow"]
[[job]]
name = "clippy"
group = "all"
tier = "check"
kind = "crates"
toolchain = "rust"
workspace = true
shared = ["tools/boat.py"]
command = ["boat", "clippy", "{packages}"]
[[job]]
name = "test"
group = "all"
tier = "check"
kind = "crates"
toolchain = "rust"
limit = 3
heavy = true
command = ["boat", "test", "{packages}"]
"""


def crates(**deps):
    return {name: ci.Crate(name, f"crates/{name}", frozenset(uses)) for name, uses in deps.items()}


def context(changed, tree=None, passes=None, workspace=None, base="b1"):
    tree = tree or {"src/a.rs": ("blob-a", True), "src/b.rs": ("blob-b", True), "README.md": ("blob-r", True),
                    "tools/scan.py": ("blob-s", True), "Cargo.lock": ("blob-l", True),
                    "crates/core/src/lib.rs": ("c1", True), "crates/mid/src/lib.rs": ("m1", True),
                    "crates/top/src/lib.rs": ("t1", True), "crates/other/src/lib.rs": ("o1", True)}
    passes = {} if passes is None else passes
    workspace = workspace or crates(core=[], mid=["core"], top=["mid"], other=[])
    pipeline = ci.load_pipeline(PIPELINE)
    definitions = {job.name: job.definition for job in pipeline.jobs}
    return ci.Context(tree, set(changed), base, "head", {"python": "py", "rust": "rs"}, workspace,
                      passes.get, pipeline, definitions=definitions)


def decide(name, ctx, tier="check"):
    job = next(job for job in ctx.pipeline.jobs if job.name == name)
    return ci.decide(job, ctx, tier)


def passing(ctx, *names):
    """The pass store after each named job ran and passed on `ctx`."""
    passes = {}
    for name in names:
        for key in decide(name, ctx).keys.values():
            passes[key] = {"sha": "abc1234567"}
    return passes


class Globs(unittest.TestCase):
    def test_double_star_crosses_folders_and_single_star_does_not(self):
        match = ci.matcher(["src/**", "*.md", "tools/**/*.mjs"])
        self.assertTrue(match("src/deep/x.rs"))
        self.assertTrue(match("README.md"))
        self.assertFalse(match("docs/README.md"))
        self.assertTrue(match("tools/x.mjs"))
        self.assertTrue(match("tools/e2e/x.mjs"))
        self.assertFalse(match("tools/x.mjs.bak"))
        self.assertFalse(ci.matcher([])("anything"))
        self.assertFalse(ci.matcher(["Cargo.toml", "Cargo.lock"])("Cargo.toml.bak"))
        self.assertTrue(ci.matcher(["Cargo.toml", "Cargo.lock"])("Cargo.toml"))


class Reuse(unittest.TestCase):
    def test_a_job_whose_inputs_did_not_change_is_skipped_not_passed(self):
        decision = decide("scan", context(["docs/x.md"]))
        self.assertEqual(decision.state, "skipped")

    def test_a_recorded_pass_is_reused_and_names_its_commit(self):
        ctx = context(["src/a.rs"])
        self.assertEqual(decide("scan", ctx).state, "run")
        again = decide("scan", context(["src/a.rs"], passes=passing(ctx, "scan")))
        self.assertEqual(again.state, "reused")
        self.assertIn("abc1234567", again.detail)

    def test_any_input_change_invalidates_the_pass(self):
        ctx = context(["src/a.rs"])
        tree = dict(ctx.tree, **{"src/b.rs": ("blob-b2", True)})
        self.assertEqual(decide("scan", context(["src/a.rs"], tree=tree, passes=passing(ctx, "scan"))).state, "run")

    def test_a_pass_recorded_while_a_file_was_untracked_does_not_count_once_it_is_tracked(self):
        untracked = context(["src/new.rs"], tree={"src/new.rs": ("blob-n", False)})
        tracked = context(["src/new.rs"], tree={"src/new.rs": ("blob-n", True)}, passes=passing(untracked, "scan"))
        self.assertEqual(decide("scan", tracked).state, "run")

    def test_a_fixture_named_by_the_environment_keys_the_job(self):
        import tempfile
        pipeline = PIPELINE.replace('command = ["slow"]', 'command = ["slow"]\nenvironment = ["CI_TEST_FIXTURE"]')
        with tempfile.TemporaryDirectory() as folder:
            fixture = Path(folder, "replies.json")
            fixture.write_text("one", encoding="utf-8")
            os.environ["CI_TEST_FIXTURE"] = str(fixture)
            try:
                ctx = context(["src/a.rs"])
                ctx.pipeline = ci.load_pipeline(pipeline)
                ctx.definitions = {job.name: job.definition for job in ctx.pipeline.jobs}
                job = next(job for job in ctx.pipeline.jobs if job.name == "slow")
                first = ci.decide(job, ctx, "main").keys
                fixture.write_text("two", encoding="utf-8")
                self.assertNotEqual(ci.decide(job, ctx, "main").keys, first)
            finally:
                del os.environ["CI_TEST_FIXTURE"]

    def test_a_ratchet_reruns_when_the_base_moves(self):
        ctx = context(["src/a.rs"], base="b1")
        self.assertEqual(decide("ratchet", context(["src/a.rs"], base="b2", passes=passing(ctx, "ratchet"))).state, "run")
        self.assertEqual(decide("ratchet", context(["src/a.rs"], base="b1", passes=passing(ctx, "ratchet"))).state, "reused")

    def test_a_later_tier_runs_the_jobs_of_the_earlier_ones(self):
        self.assertEqual(decide("scan", context(["src/a.rs"]), tier="main").state, "run")

    def test_a_job_of_a_later_tier_that_would_run_is_left_to_it(self):
        self.assertEqual(decide("slow", context(["src/a.rs"])).state, "later")
        self.assertEqual(decide("slow", context(["docs/x"])).state, "skipped")
        self.assertEqual(decide("slow", context(["src/a.rs"]), tier="main").state, "run")

    def test_a_missing_toolchain_is_not_run_rather_than_passed(self):
        ctx = context(["src/a.rs"])
        ctx.missing = {"python": "python is not available"}
        self.assertEqual(decide("scan", ctx).state, "not run")

    def test_a_missing_toolchain_does_not_block_a_job_nothing_selected(self):
        ctx = context(["docs/x.md"])
        ctx.missing = {"python": "python is not available"}
        self.assertEqual(decide("scan", ctx).state, "skipped")

    def test_a_changed_toolchain_definition_runs_its_jobs(self):
        ctx = context(["docs/x.md"])
        changed = ci.load_pipeline(PIPELINE.replace('[toolchain.python]\nversion = ["python"]', '[toolchain.python]\nversion = ["python3"]'))
        job = next(job for job in changed.jobs if job.name == "scan")
        self.assertEqual(ci.decide(job, ctx, "check").state, "run")

    def test_a_job_whose_definition_changed_runs(self):
        ctx = context(["docs/x.md"])
        ctx.definitions = dict(ctx.definitions, scan="before")
        self.assertEqual(decide("scan", ctx).state, "run")
        ctx.definitions = None
        self.assertEqual(decide("scan", ctx).state, "run")


class PerFile(unittest.TestCase):
    def test_only_changed_files_without_a_pass_run(self):
        ctx = context(["src/a.rs", "src/b.rs", "docs/x.md"])
        self.assertEqual(decide("per-file", ctx).arguments, ["src/a.rs", "src/b.rs"])
        passes = {key: {"sha": "abc"} for path, key in decide("per-file", ctx).keys.items() if path == "src/a.rs"}
        self.assertEqual(decide("per-file", context(["src/a.rs", "src/b.rs"], passes=passes)).arguments, ["src/b.rs"])

    def test_a_changed_shared_input_rescans_every_file(self):
        decision = decide("per-file", context(["tools/scan.py"]))
        self.assertEqual(decision.arguments, ["README.md", "src/a.rs", "src/b.rs"])
        self.assertTrue(decision.whole)
        self.assertFalse(decide("per-file", context(["src/a.rs"])).whole)

    def test_a_deleted_file_is_not_scanned(self):
        self.assertEqual(decide("per-file", context(["src/gone.rs"])).state, "skipped")

    def test_long_file_lists_split_into_command_lines_that_fit(self):
        files = [f"src/{index:05}/{'x' * 90}.rs" for index in range(600)]
        commands = list(ci.batches(["scan", "{files}"], {}, files))
        self.assertGreater(len(commands), 1)
        self.assertTrue(all(sum(len(part) + 1 for part in command) <= ci.COMMAND_LIMIT for command in commands))
        self.assertEqual([file for command in commands for file in command[1:]], files)


class Crates(unittest.TestCase):
    def test_a_changed_crate_selects_itself_and_its_direct_dependents_only(self):
        decision = decide("clippy", context(["crates/core/src/lib.rs"]))
        self.assertEqual(decision.state, "run")
        self.assertEqual(decision.arguments, ["-p", "core", "-p", "mid"])

    def test_an_unchanged_workspace_skips(self):
        self.assertEqual(decide("clippy", context(["docs/x.md"])).state, "skipped")

    def test_a_widening_file_runs_the_workspace(self):
        pipeline = PIPELINE.replace('limit = 3\nheavy = true\n', 'limit = 3\nheavy = true\nwiden = [".config/nextest.toml"]\n')
        ctx = context([".config/nextest.toml"])
        ctx.pipeline = ci.load_pipeline(pipeline)
        ctx.definitions = {job.name: job.definition for job in ctx.pipeline.jobs}
        ctx.hosted = True
        job = next(job for job in ctx.pipeline.jobs if job.name == "test")
        self.assertEqual(ci.decide(job, ctx, "check").state, "run")

    def test_a_changed_group_redefines_its_jobs(self):
        changed = ci.load_pipeline(PIPELINE.replace('[group.all]\nos = "ubuntu-latest"', '[group.all]\nos = "windows-latest"'))
        ctx = context(["docs/x.md"])
        job = next(job for job in changed.jobs if job.name == "scan")
        self.assertEqual(ci.decide(job, ctx, "check").state, "run")

    def test_the_upstream_names_its_github_repository(self):
        self.assertIsNone(ci.upstream_repository("no-such-remote-for-this-test/main"))

    def test_a_changed_crate_job_definition_checks_the_workspace(self):
        ctx = context(["docs/x.md"])
        ctx.definitions = dict(ctx.definitions, clippy="before")
        self.assertEqual(decide("clippy", ctx).arguments, ["--workspace"])

    def test_unlisted_crates_skip_and_a_crate_change_cannot_run(self):
        ctx = context(["docs/x.md"])
        ctx.crates = None
        self.assertEqual(decide("clippy", ctx).state, "skipped")
        ctx.changed = {"crates/core/src/lib.rs"}
        self.assertEqual(decide("clippy", ctx).state, "not run")

    def test_a_changed_runner_invalidates_crate_passes_without_selecting_crates(self):
        ctx = context(["crates/core/src/lib.rs"])
        passes = passing(ctx, "clippy")
        tree = dict(ctx.tree, **{"tools/boat.py": ("boat2", True)})
        self.assertEqual(decide("clippy", context(["crates/core/src/lib.rs"], tree=tree, passes=passes)).state, "run")
        self.assertEqual(decide("clippy", context(["tools/boat.py"], tree=tree)).state, "skipped")

    def test_a_crate_key_follows_its_transitive_dependencies(self):
        ctx = context(["crates/top/src/lib.rs"])
        passes = passing(ctx, "clippy")
        tree = dict(ctx.tree, **{"crates/core/src/lib.rs": ("c2", True)})
        self.assertEqual(decide("clippy", context(["crates/top/src/lib.rs"], tree=tree, passes=passes)).state, "run")
        tree = dict(ctx.tree, **{"crates/other/src/lib.rs": ("o2", True)})
        self.assertEqual(decide("clippy", context(["crates/top/src/lib.rs"], tree=tree, passes=passes)).state, "reused")

    def test_a_partial_pass_runs_only_the_rest(self):
        ctx = context(["crates/core/src/lib.rs"])
        passes = {key: {"sha": "abc"} for name, key in decide("clippy", ctx).keys.items() if name == "core"}
        decision = decide("clippy", context(["crates/core/src/lib.rs"], passes=passes))
        self.assertEqual(decision.arguments, ["-p", "mid"])
        self.assertIn("1 reused", decision.detail)

    def test_more_crates_than_a_jobs_limit_are_left_to_a_later_tier(self):
        workspace = crates(core=[], a=["core"], b=["core"], c=["core"])
        tree = {f"crates/{name}/src/lib.rs": (name, True) for name in workspace}
        ctx = context(["crates/core/src/lib.rs"], tree=tree, workspace=workspace)
        self.assertEqual(decide("test", ctx).state, "deferred")
        self.assertEqual(decide("clippy", ctx).arguments, ["-p", "a", "-p", "b", "-p", "c", "-p", "core"])

    def test_a_workspace_wide_change_checks_the_workspace_and_leaves_tests_to_a_later_tier(self):
        ctx = context(["Cargo.lock"])
        self.assertEqual(decide("clippy", ctx).arguments, ["--workspace"])
        self.assertEqual(decide("test", ctx).state, "deferred")

    def test_heavy_local_work_is_skipped_while_the_game_runs(self):
        ctx = context(["crates/core/src/lib.rs"])
        ctx.game = True
        self.assertEqual(decide("test", ctx).state, "held")
        self.assertEqual(decide("clippy", ctx).state, "run")

    def test_a_broken_toolchain_overrides_a_recorded_pass(self):
        ctx = context(["src/a.rs"])
        ctx.passed = passing(ctx, "scan").get
        ctx.missing = {"python": "stale"}
        self.assertEqual(decide("scan", ctx).state, "not run")

    def test_paused_heavy_local_work_is_reported_as_not_run_and_never_passes(self):
        ctx = context(["crates/core/src/lib.rs"])
        ctx.pipeline.heavy_local = False
        decision = decide("test", ctx)
        self.assertEqual(decision.state, "paused")
        outcome = ci.Outcome("test", decision.state, decision.detail)
        self.assertEqual(ci.verdict([outcome]), "pending")
        self.assertIn("not run (heavy local work paused): test", ci.describe([outcome], 1))
        self.assertEqual(decide("test", context(["docs/x.md"], workspace=None)).state, "skipped")

    def test_a_nested_crate_folder_owns_its_files(self):
        workspace = {"outer": ci.Crate("outer", "crates/outer", frozenset()),
                     "inner": ci.Crate("inner", "crates/outer/inner", frozenset())}
        owner = ci.crate_owner(workspace)
        self.assertEqual(owner("crates/outer/inner/src/lib.rs"), "inner")
        self.assertEqual(owner("crates/outer/src/lib.rs"), "outer")
        self.assertIsNone(owner("docs/x.md"))

    def test_manifests_map_path_dependencies_to_workspace_crates(self):
        import tempfile
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            files = {
                "Cargo.toml": '[workspace]\nmembers = ["crates/*"]\nexclude = ["crates/gone"]\n'
                              '[workspace.dependencies]\nshared-core = { path = "crates/core" }\nserde = "1"\n',
                "crates/core/Cargo.toml": '[package]\nname = "core"\n[dependencies]\nserde = { workspace = true }\n',
                "crates/mid/Cargo.toml": '[package]\nname = "mid"\n[dev-dependencies]\nshared-core = { workspace = true }\n',
                "crates/top/Cargo.toml": '[package]\nname = "top"\n[target."cfg(windows)".dependencies]\nm = { package = "mid", path = "../mid" }\n'
                                         '[dependencies]\ntao = { path = "../../vendor/tao" }\n',
                "crates/gone/Cargo.toml": '[package]\nname = "gone"\n',
            }
            for name, text in files.items():
                Path(root, name).parent.mkdir(parents=True, exist_ok=True)
                Path(root, name).write_text(text, encoding="utf-8")
            found = ci.crates_from_manifests(root)
            self.assertEqual(sorted(found), ["core", "mid", "top"])
            self.assertEqual(found["mid"].deps, {"core"})
            self.assertEqual(found["top"].deps, {"mid"})
            self.assertEqual(found["core"].folder, "crates/core")


class Installed(unittest.TestCase):
    def write(self, folder, name, packages, **fields):
        path = Path(folder, name)
        path.write_text(ci.json.dumps(dict(fields, packages=packages)), encoding="utf-8")
        return path

    def install(self, folder, packages):
        for path, version in packages.items():
            Path(folder, path).mkdir(parents=True, exist_ok=True)
            Path(folder, path, "package.json").write_text(ci.json.dumps({"version": version}), encoding="utf-8")

    def test_an_installation_that_lags_its_lockfile_is_named(self):
        import shutil
        import tempfile
        with tempfile.TemporaryDirectory() as root:
            folder = Path(root, "node_modules")
            folder.mkdir()
            self.install(root, {"node_modules/typescript": "7.0.2"})
            manifest = self.write(folder, "package.json", {})
            lock = self.write(folder, "lock.json", {"": {}, "node_modules/typescript": {"version": "7.0.2"}})
            stale = lambda lock, installed: ci.npm_stale(manifest, lock, installed)
            same = self.write(folder, "same.json", {"node_modules/typescript": {"version": "7.0.2"}})
            old = self.write(folder, "old.json", {"node_modules/typescript": {"version": "6.0.0"}})
            self.assertIsNone(stale(lock, same))
            self.assertIn("typescript 6.0.0 is installed, 7.0.2 is locked", stale(lock, old))
            extra = self.write(folder, "extra.json", {"node_modules/typescript": {"version": "7.0.2"},
                                                      "node_modules/@msgpack/msgpack": {"version": "3.1.2"}})
            self.assertIn("@msgpack/msgpack is installed but not locked", stale(lock, extra))
            optional = self.write(folder, "optional.json", {"": {}, "node_modules/typescript": {"version": "7.0.2"},
                                                            "node_modules/@ts/win": {"version": "7.0.2", "optional": True}})
            wrong = self.write(folder, "wrong.json", {"node_modules/typescript": {"version": "7.0.2"},
                                                      "node_modules/@ts/win": {"version": "7.0.1"}})
            self.assertIsNone(stale(optional, same))
            self.install(root, {"node_modules/@ts/win": "7.0.2"})
            self.assertIn("@ts/win 7.0.1 is installed", stale(optional, wrong))
            forked = self.write(folder, "forked.json", {"": {}, "node_modules/typescript": {"version": "7.0.2", "integrity": "sha512-fork"}})
            self.assertIn("other contents", stale(forked, same))
            bumped = self.write(folder, "bumped.json", {}, devDependencies={"typescript": "7.0.3"})
            self.assertIn("devDependencies differ", ci.npm_stale(bumped, lock, same))
            shutil.rmtree(Path(root, "node_modules", "@ts"))
            self.install(root, {"node_modules/tinyglobby": "0.2.17"})
            self.assertIn("tinyglobby is on disk but not locked", stale(lock, same))
            shutil.rmtree(Path(root, "node_modules", "tinyglobby"))
            self.install(root, {"node_modules/typescript": "6.9.9"})
            self.assertIn("typescript is not installed at 7.0.2 on disk", stale(lock, same))


class Waking(unittest.TestCase):
    def test_boat_arguments_are_what_follows_the_boat_script(self):
        command = ["py", "tools/check-clippy.py", "--against", "b", "--", "py", "tools/boat.py", "clippy", "--all-targets", "-p", "a"]
        self.assertEqual(ci.boat_arguments(command), ["clippy", "--all-targets", "-p", "a"])
        self.assertIsNone(ci.boat_arguments(["cargo", "clippy"]))


class Superseding(unittest.TestCase):
    def test_a_newer_run_on_the_branch_supersedes_an_older_one(self):
        import tempfile
        with tempfile.TemporaryDirectory() as folder:
            store = ci.Store(Path(folder))
            older = ci.Active(store, "check", "run-1")
            self.assertFalse(older.superseded())
            newer = ci.Active(store, "check", "run-2")
            self.assertTrue(older.superseded())
            self.assertFalse(newer.superseded())
            self.assertTrue(older.superseded())
            self.assertFalse(ci.Active(store, "main", "run-3").superseded())
            self.assertFalse(newer.superseded())

    def test_cancelling_ends_local_commands_and_lets_boat_finish(self):
        runner = ci.Runner(Path("."), {})
        ended = []
        original = ci.end
        ci.end = lambda process, job: ended.append(process)
        try:
            runner.live = {1: ("local", None, False), 2: ("boat", None, True)}
            runner.cancel()
        finally:
            ci.end = original
        self.assertEqual(ended, ["local"])
        self.assertEqual(list(runner.live), [2])
        self.assertTrue(runner.cancelled)


class Verdict(unittest.TestCase):
    def outcomes(self, *states):
        return [ci.Outcome(f"job{index}", state) for index, state in enumerate(states)]

    def test_a_job_that_could_not_run_is_never_a_success(self):
        self.assertEqual(ci.verdict(self.outcomes("pass", "not run", "reused")), "error")
        self.assertEqual(ci.verdict(self.outcomes("pass", "not run", "fail")), "failure")
        self.assertEqual(ci.verdict(self.outcomes("pass", "reused", "skipped", "later")), "success")
        for state in ("held", "paused", "deferred"):
            self.assertEqual(ci.verdict(self.outcomes("pass", state)), "pending", state)

    def test_the_status_names_failed_unrun_and_deferred_jobs_within_githubs_limit(self):
        outcomes = [ci.Outcome("rust-clippy", "fail"), ci.Outcome("ui-tests", "not run"), ci.Outcome("browser", "later"),
                    ci.Outcome("comments", "pass"), ci.Outcome("citations", "reused"), ci.Outcome("suppressions", "skipped", ci.UNCHANGED),
                    ci.Outcome("rust-test", "held", "the game lock exists")]
        text = ci.describe(outcomes, 41.2)
        self.assertEqual(text, "failed: rust-clippy; not run: ui-tests; held for the game: rust-test; later tier: browser; 2 ran, 1 reused, 1 unchanged; 41s")
        long = [ci.Outcome("x" * 40 + str(index), "fail") for index in range(10)]
        self.assertLessEqual(len(ci.describe(long, 1)), ci.STATUS_LIMIT)


class Pipeline(unittest.TestCase):
    def test_the_pipeline_loads_and_runs_every_repo_check(self):
        pipeline = ci.load_pipeline((ROOT / "tools" / "ci.toml").read_text(encoding="utf-8"))
        commands = " ".join(part for job in pipeline.jobs for part in job.command)
        scripts = sorted(path.name for path in (ROOT / "tools").glob("check-*.py"))
        self.assertTrue(scripts)
        self.assertEqual([script for script in scripts if f"tools/{script}" not in commands], [])

    def test_an_unknown_tier_is_refused(self):
        with self.assertRaises(SystemExit):
            ci.load_pipeline(re.sub(r'(name = "scan"\ngroup = "all"\ntier = )"check"', r'\1"weekly"', PIPELINE))


if __name__ == "__main__":
    unittest.main()
