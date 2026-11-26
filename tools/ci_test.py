"""Tests for tools/ci.py: python tools/ci_test.py"""

import argparse
import contextlib
import importlib.util
import io
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import yaml

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("ci", ROOT / "tools" / "ci.py")
ci = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(ci)

PIPELINE = """
tiers = ["check", "main"]
[pipeline]
implementation = ["tools/ci.py"]
[group.all]
setup = ["python"]
[toolchain.python]
files = []
[toolchain.rust]
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
shared = ["tools/clippy-wrapper.py"]
command = ["cargo", "clippy", "{packages}"]
[[job]]
name = "test"
group = "all"
tier = "check"
kind = "crates"
toolchain = "rust"
command = ["boat", "test", "{packages}"]
"""


def crates(**deps):
    return {name: ci.Crate(name, f"crates/{name}", frozenset(uses), frozenset(uses)) for name, uses in deps.items()}


def context(changed, tree=None, workspace=None):
    tree = tree or {"src/a.rs", "src/b.rs", "README.md", "tools/scan.py", "Cargo.lock", "crates/core/src/lib.rs",
                    "crates/mid/src/lib.rs", "crates/top/src/lib.rs", "crates/other/src/lib.rs"}
    workspace = workspace or crates(core=[], mid=["core"], top=["mid"], other=[])
    pipeline = ci.load_pipeline(PIPELINE)
    definitions = {job.name: job.definition for job in pipeline.jobs}
    return ci.Context(set(tree), set(changed), workspace, pipeline, definitions, pipeline.frame)


def decide(name, ctx, tier="check"):
    job = next(job for job in ctx.pipeline.jobs if job.name == name)
    return ci.decide(job, ctx, tier)


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


class Gates(unittest.TestCase):
    def test_a_job_whose_inputs_did_not_change_is_skipped_not_passed(self):
        self.assertEqual(decide("scan", context(["docs/x.md"])).state, "skipped")
        self.assertEqual(decide("scan", context(["src/a.rs"])).state, "run")

    def test_a_later_tier_runs_the_jobs_of_the_earlier_ones(self):
        self.assertEqual(decide("scan", context(["src/a.rs"]), tier="main").state, "run")

    def test_a_job_of_a_later_tier_that_would_run_is_left_to_it(self):
        self.assertEqual(decide("slow", context(["src/a.rs"])).state, "later")
        self.assertEqual(decide("slow", context(["docs/x"])).state, "skipped")
        self.assertEqual(decide("slow", context(["src/a.rs"]), tier="main").state, "run")

    def test_a_changed_toolchain_file_runs_its_jobs(self):
        self.assertEqual(decide("clippy", context(["Cargo.lock"])).arguments, ["--workspace"])

    def test_a_job_whose_definition_changed_runs(self):
        ctx = context(["docs/x.md"])
        ctx.definitions = dict(ctx.definitions, scan="before")
        self.assertEqual(decide("scan", ctx).state, "run")
        ctx.definitions = None
        self.assertEqual(decide("scan", ctx).state, "run")

    def test_a_pipeline_edit_reaches_only_the_jobs_it_redefines(self):
        listed = PIPELINE.replace('implementation = ["tools/ci.py"]', 'implementation = ["tools/ci.py", "tools/ci.toml"]')
        added = ci.load_pipeline(listed + '[[job]]\nname = "new"\ngroup = "all"\ntier = "check"\ntoolchain = "rust"\nkind = "crates"\ncommand = ["new", "{packages}"]\n')
        ctx = context(["tools/ci.toml"])
        ctx.pipeline = added
        ctx.definitions = {job.name: job.definition for job in ci.load_pipeline(listed).jobs}
        ctx.frame = ci.load_pipeline(listed).frame
        self.assertEqual(ci.decide(next(job for job in added.jobs if job.name == "new"), ctx, "check").arguments, ["--workspace"])
        self.assertEqual(decide("clippy", ctx).state, "skipped", "a job the edit left as it was keeps its own scope")
        self.assertEqual(decide("scan", ctx).state, "skipped")
        ctx.frame = ci.load_pipeline(listed.replace('tiers = ["check", "main"]', 'tiers = ["check", "nightly", "main"]')).frame
        self.assertEqual(decide("clippy", ctx).arguments, ["--workspace"], "an edit outside the jobs reaches every job")
        ctx.frame = None
        self.assertEqual(decide("clippy", ctx).arguments, ["--workspace"], "so does a base whose pipeline cannot be read")
        ctx.pipeline = ci.load_pipeline(PIPELINE)
        ctx.definitions = {job.name: job.definition for job in ctx.pipeline.jobs}
        ctx.frame = ci.load_pipeline(listed).frame
        self.assertEqual(decide("clippy", ctx).arguments, ["--workspace"], "an edit that drops tools/ci.toml from the implementation still reaches every job")
        globbed = ci.load_pipeline(listed.replace('"tools/ci.py", "tools/ci.toml"', '"tools/**"'))
        ctx.pipeline, ctx.frame = globbed, globbed.frame
        ctx.definitions = {job.name: job.definition for job in globbed.jobs}
        self.assertEqual(decide("clippy", ctx).state, "skipped", "an implementation glob covering tools/ci.toml leaves a job-only edit to the jobs it redefines")
        ctx.changed = {"tools/ci.toml", "tools/ci.py"}
        self.assertEqual(decide("clippy", ctx).arguments, ["--workspace"], "and still widens on the rest of the implementation")
        self.assertIsNone(ci.base_pipeline(PIPELINE.replace("[toolchain.python]\nfiles = []\n[toolchain.rust]\nfiles = [\"Cargo.lock\"]\n", "toolchain = []\n")),
                          "a base whose tables have the wrong shape is unreadable, so a change repairing it still plans")

    def test_a_changed_group_redefines_its_jobs(self):
        changed = ci.load_pipeline(PIPELINE.replace('[group.all]\nsetup = ["python"]', '[group.all]\nsetup = ["python", "node"]'))
        job = next(job for job in changed.jobs if job.name == "scan")
        self.assertEqual(ci.decide(job, context(["docs/x.md"]), "check").state, "run")


class PerFile(unittest.TestCase):
    def test_only_changed_files_run(self):
        self.assertEqual(decide("per-file", context(["src/a.rs", "src/b.rs", "docs/x.md"])).arguments, ["src/a.rs", "src/b.rs"])

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


    def test_a_widening_file_runs_the_workspace(self):
        pipeline = PIPELINE.replace('command = ["boat", "test", "{packages}"]\n', 'command = ["boat", "test", "{packages}"]\nwiden = [".config/nextest.toml"]\n')
        ctx = context([".config/nextest.toml"])
        ctx.pipeline = ci.load_pipeline(pipeline)
        ctx.definitions = {job.name: job.definition for job in ctx.pipeline.jobs}
        job = next(job for job in ctx.pipeline.jobs if job.name == "test")
        self.assertEqual(ci.decide(job, ctx, "check").arguments, ["--workspace"])

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

    def test_a_job_without_dependents_takes_the_changed_crates_only(self):
        pipeline = ci.load_pipeline(PIPELINE.replace('command = ["cargo", "clippy", "{packages}"]\n', 'command = ["cargo", "clippy", "{packages}"]\ndependents = false\n'))
        ctx = context(["crates/core/src/lib.rs"])
        ctx.pipeline, ctx.definitions = pipeline, {job.name: job.definition for job in pipeline.jobs}
        job = next(job for job in pipeline.jobs if job.name == "clippy")
        self.assertEqual(ci.decide(job, ctx, "check").arguments, ["-p", "core"])
        ctx.transitive = True
        self.assertEqual(ci.decide(job, ctx, "main").arguments, ["-p", "core"])
        ctx.everything = True
        self.assertEqual(ci.decide(job, ctx, "main").arguments, ["--workspace"])

    def test_main_takes_every_crate_above_a_change_and_every_crate_whose_tests_use_one(self):
        workspace = dict(crates(core=[], mid=["core"], top=["mid"], other=[]),
                         probe=ci.Crate("probe", "crates/probe", frozenset({"top"}), frozenset()),
                         outer=ci.Crate("outer", "crates/outer", frozenset({"probe"}), frozenset({"probe"})))
        ctx = context(["crates/core/src/lib.rs"], workspace=workspace)
        self.assertEqual(decide("test", ctx).arguments, ["-p", "core", "-p", "mid"])
        ctx.transitive = True
        decision = decide("test", ctx, tier="main")
        self.assertEqual(decision.arguments, ["-p", "core", "-p", "mid", "-p", "probe", "-p", "top"])
        self.assertEqual(decision.beyond, ["probe", "top"])
        self.assertEqual(decision.detail, "4 crates (2 beyond a pull request's scope)")
        self.assertEqual(decide("test", context(["docs/x.md"], workspace=workspace), tier="main").state, "skipped")


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
                                         '[dependencies]\ntao = { path = "../../vendor/tao" }\n'
                                         '[target.\'cfg(all())\'.dev-dependencies]\nshared-core = { workspace = true, features = ["x"] }\n'
                                         'test-unify = { path = "../test-unify" }\n',
                "crates/test-unify/Cargo.toml": '[package]\nname = "test-unify"\n',
                "crates/gone/Cargo.toml": '[package]\nname = "gone"\n',
                "fuzz/Cargo.toml": '[package]\nname = "fuzz"\n[dependencies]\nt = { package = "top", path = "../crates/top" }\n',
            }
            for name, text in files.items():
                Path(root, name).parent.mkdir(parents=True, exist_ok=True)
                Path(root, name).write_text(text, encoding="utf-8")
            found = ci.crates_from_manifests(root)
            self.assertEqual(sorted(found), ["core", "mid", "test-unify", "top"])
            self.assertEqual(found["mid"].deps, {"core"})
            self.assertEqual(found["mid"].libraries, set())
            self.assertEqual(found["top"].deps, {"mid", "test-unify"})
            self.assertEqual(found["top"].libraries, {"mid"})
            self.assertEqual(found["core"].folder, "crates/core")
            self.assertEqual(ci.manifest_reach(root, "fuzz/Cargo.toml", found), {"top", "mid"})
            self.assertIsNone(ci.manifest_reach(root, "missing/Cargo.toml", found))
        pipeline = ci.load_pipeline(PIPELINE + '[[job]]\nname = "fuzz"\ngroup = "all"\ntier = "check"\ntoolchain = "rust"\n'
                                               'inputs = ["fuzz/**"]\nmanifest = "fuzz/Cargo.toml"\ncommand = ["cargo", "check"]\n')
        ctx = context(["crates/core/src/lib.rs"])
        ctx.pipeline, ctx.definitions = pipeline, {job.name: job.definition for job in pipeline.jobs}
        ctx.reach = {"fuzz": {"mid"}}
        self.assertEqual(decide("fuzz", ctx).state, "skipped")
        ctx.changed = {"crates/mid/src/lib.rs"}
        self.assertEqual(decide("fuzz", ctx).state, "run")
        ctx.reach = {"fuzz": None}
        ctx.changed = {"crates/core/src/lib.rs"}
        self.assertEqual(decide("fuzz", ctx).state, "run")

    def test_a_job_naming_a_package_runs_for_the_crates_its_library_builds_on_only(self):
        pipeline = ci.load_pipeline(PIPELINE + '[[job]]\nname = "shipped"\ngroup = "all"\ntier = "check"\ntoolchain = "rust"\n'
                                               'inputs = ["ui/generated.ts"]\npackage = "mid"\ncommand = ["cargo", "check"]\n')
        job = next(job for job in pipeline.jobs if job.name == "shipped")
        workspace = crates(core=[], mid=["core"], top=["mid"], other=[])
        self.assertEqual(ci.job_reach(ROOT, job, workspace), {"mid", "core"})
        ctx = context(["crates/top/src/lib.rs", "crates/other/src/lib.rs"])
        ctx.pipeline, ctx.definitions = pipeline, {job.name: job.definition for job in pipeline.jobs}
        ctx.reach = {"shipped": ci.job_reach(ROOT, job, workspace)}
        self.assertEqual(decide("shipped", ctx).state, "skipped")
        for path in ("crates/core/src/lib.rs", "crates/mid/src/lib.rs", "ui/generated.ts", "Cargo.lock"):
            ctx.changed = {path}
            self.assertEqual(decide("shipped", ctx).state, "run", path)
        with self.assertRaises(SystemExit):
            ci.job_reach(ROOT, job, crates(core=[]))



class Verdict(unittest.TestCase):
    def outcomes(self, *states):
        return [ci.Outcome(f"job{index}", state) for index, state in enumerate(states)]

    def test_a_job_that_could_not_run_is_never_a_success(self):
        self.assertEqual(ci.verdict(self.outcomes("pass", "not run", "skipped")), "error")
        self.assertEqual(ci.verdict(self.outcomes("pass", "not run", "fail")), "failure")
        self.assertEqual(ci.verdict(self.outcomes("pass", "skipped", "later", "ci only")), "success")

    def test_a_part_a_job_could_not_run_here_is_named_in_its_detail(self):
        log = "$ check\ntree: 3 files\nnot run here: installation comparison (no game installation)\n0 finding(s)\n"
        self.assertEqual(ci.not_run_here(log), ["not run here: installation comparison (no game installation)"])
        self.assertEqual(ci.not_run_here("  not run here: indented output is not a note\n"), [])

    def test_every_junit_failure_is_reported_with_its_output(self):
        report = """<testsuites><testsuite name="dashboard-base">
            <testcase name="ready"/><testcase name="ignored"><skipped/></testcase>
            <testcase name="needs_tables"><failure type="test failure with exit code 101"/>
            <system-err>thread 'needs_tables' panicked at test.rs:1:1:
BATTLEPLAN_GAME_DIR</system-err></testcase></testsuite>
            <testsuite name="consumer::integration"><testcase name="lost"><error message="interrupted"/></testcase>
            </testsuite></testsuites>"""
        lines = ci.failure_report(report, {"consumer"})
        self.assertEqual(lines[0], "1 passed, 2 failed")
        self.assertIn("--- FAIL dashboard-base needs_tables", lines)
        self.assertIn("BATTLEPLAN_GAME_DIR", lines)
        self.assertIn("--- FAIL consumer::integration lost (beyond a pull request's scope)", lines)
        self.assertTrue(any("interrupted" in line for line in lines))
        self.assertIsNone(ci.failure_report("not XML"))


class ReviewOfFourteen(unittest.TestCase):
    def test_a_planned_job_the_runner_would_skip_fails_the_leg(self):
        ctx = context(["src/a.rs"])
        decisions = [decide(name, ctx) for name in ("scan", "ratchet")]
        skipped = [decide("scan", context(["docs/x.md"])), decisions[1]]
        self.assertEqual(ci.unplanned_outcomes(["scan", "ratchet"], decisions), [])
        failures = ci.unplanned_outcomes(["scan", "ratchet", "gone"], skipped)
        self.assertEqual([(o.name, o.state) for o in failures], [("scan", "fail"), ("gone", "fail")])
        self.assertEqual(ci.verdict(failures), "failure")

    def test_a_change_to_the_pipeline_implementation_runs_every_job(self):
        ctx = context(["tools/ci.py"])
        for name in ("scan", "ratchet", "per-file", "clippy"):
            self.assertEqual(decide(name, ctx).state, "run", name)
        self.assertEqual(decide("clippy", ctx).arguments, ["--workspace"])

    def test_a_change_to_a_jobs_implementation_script_runs_it(self):
        self.assertEqual(decide("clippy", context(["tools/clippy-wrapper.py"])).arguments, ["--workspace"])
        self.assertEqual(decide("clippy", context(["tools/boat.py"])).state, "skipped")

    def test_an_unreadable_crate_graph_never_skips_a_widened_job(self):
        pipeline = PIPELINE.replace('command = ["boat", "test", "{packages}"]\n', 'command = ["boat", "test", "{packages}"]\nwiden = [".config/nextest.toml"]\n')
        ctx = context([".config/nextest.toml"])
        ctx.pipeline = ci.load_pipeline(pipeline)
        ctx.definitions = {job.name: job.definition for job in ctx.pipeline.jobs}
        ctx.crates = None
        job = next(job for job in ctx.pipeline.jobs if job.name == "test")
        self.assertEqual(ci.decide(job, ctx, "check").state, "not run")

    def test_a_tree_base_ratchets_against_heads_first_parent(self):
        empty_tree = ci.git("hash-object", "-t", "tree", "--stdin", data=b"").strip()
        parent = ci.git("rev-parse", "HEAD^1").strip()
        self.assertEqual(ci.ratchet_base(empty_tree), parent)
        self.assertEqual(ci.ratchet_base(parent), parent)

    def test_a_rolled_back_tip_ratchets_against_the_shared_history(self):
        original = ci.git
        answers = {("cat-file", "-t", "replaced-tip"): "commit", ("merge-base", "HEAD", "replaced-tip"): "head"}
        ci.git = lambda *args, check=True, data=None: answers.get(args, "")
        try:
            self.assertEqual(ci.ratchet_base("replaced-tip"), "head")
        finally:
            ci.git = original
        parent = ci.git("rev-parse", "HEAD^1").strip()
        self.assertEqual(ci.ratchet_base(parent), parent)

    def test_every_group_runs_on_a_fixed_github_hosted_label(self):
        pipeline = ci.load_pipeline((ROOT / "tools" / "ci.toml").read_text(encoding="utf-8"))
        workflow = (ROOT / ".github" / "workflows" / "ci.yml").read_text(encoding="utf-8")
        found = re.search(r"runs-on: \$\{\{ fromJSON\('(\{.*?\})'\)\[matrix\.group\] \}\}", workflow)
        self.assertIsNotNone(found)
        labels = ci.json.loads(found.group(1))
        self.assertEqual(sorted(labels), sorted(pipeline.groups))
        for label in labels.values():
            self.assertRegex(label, r"^(ubuntu|windows)-(latest|\d{2,4}(\.\d+)?)$")
        self.assertNotIn("matrix.os", workflow)

    def test_a_pull_request_compares_with_the_commit_it_was_merged_into(self):
        workflow = yaml.safe_load((ROOT / ".github" / "workflows" / "ci.yml").read_text(encoding="utf-8"))
        tier = next(step["run"] for step in workflow["jobs"]["changes"]["steps"] if step.get("id") == "tier")
        bash = shutil.which("bash")
        self.assertIsNotNone(bash)
        with tempfile.TemporaryDirectory() as folder:
            def git(*args):
                identity = ["-c", "user.name=ci", "-c", "user.email=ci@example.invalid", "-c", "commit.gpgsign=false"]
                return subprocess.run(["git", *identity, *args], cwd=folder, capture_output=True, text=True, check=True).stdout.strip()

            def commit(name):
                Path(folder, name).write_text(name, encoding="utf-8")
                git("add", name)
                git("commit", "--quiet", "--message", name)
                return git("rev-parse", "HEAD")

            def base(event_base):
                output = Path(folder, "output")
                output.write_text("", encoding="utf-8")
                env = dict(ci.os.environ, EVENT="pull_request", PR_BASE=event_base, GITHUB_OUTPUT=output.as_posix())
                subprocess.run([bash, "-e", "-c", tier], cwd=folder, env=env, check=True, capture_output=True)
                return dict(line.split("=", 1) for line in output.read_text(encoding="utf-8").splitlines())["base"]

            git("init", "--quiet", "--initial-branch", "main")
            opened = commit("opened")
            git("checkout", "--quiet", "-b", "change")
            change = commit("change")
            self.assertEqual(base(opened), opened)
            git("checkout", "--quiet", "main")
            merged_into = commit("merged-since")
            git("merge", "--quiet", "--no-ff", "--message", "merge", change)
            self.assertEqual(base(opened), merged_into)


class Sharding(unittest.TestCase):
    WORKSPACE = crates(hub=[], mid=["hub"], app=["mid", "hub"], tool=["hub"], leaf=[], other=["leaf"])
    COSTS = {"hub": (600, 900), "mid": (300, 600), "app": (300, 1200), "tool": (100, 900), "leaf": (500, 300), "other": (50, 100)}

    @staticmethod
    def table(costs):
        return ci.json.dumps({"crates": {name: {"build": build, "test": test} for name, (build, test) in costs.items()}})

    def sharded(self, changed, tier="check", shards=2, everything=False):
        pipeline = PIPELINE.replace('command = ["boat", "test", "{packages}"]\n', f'command = ["boat", "test", "{{packages}}"]\nshards = {shards}\n')
        tree = {f"crates/{name}/src/lib.rs" for name in self.WORKSPACE}
        ctx = context(changed, tree=tree, workspace=self.WORKSPACE)
        ctx.pipeline = ci.load_pipeline(pipeline)
        ctx.definitions = {job.name: job.definition for job in ctx.pipeline.jobs}
        ctx.everything, ctx.transitive = everything, tier != "check"
        return ctx, [ci.decide(job, ctx, tier) for job in ctx.pipeline.jobs], self.table(self.COSTS)

    def test_every_crate_runs_once_and_a_dependent_joins_the_runner_building_its_dependencies(self):
        everything = frozenset(self.WORKSPACE)
        split, _ = ci.shard_crates(self.WORKSPACE, sorted(self.WORKSPACE), self.COSTS, everything, 2)
        self.assertEqual(sorted(name for shard in split for name in shard), sorted(self.WORKSPACE))
        self.assertEqual(split, ci.shard_crates(self.WORKSPACE, list(reversed(sorted(self.WORKSPACE))), self.COSTS, everything, 2)[0])
        clusters = crates(hub=[], app=["hub"], leaf=[], other=["leaf"])
        costs = {"hub": (500, 0), "app": (0, 100), "leaf": (500, 0), "other": (0, 100)}
        split, load = ci.shard_crates(clusters, sorted(clusters), costs, frozenset(clusters), 2)
        self.assertEqual((sorted(split), load), ([["app", "hub"], ["leaf", "other"]], 600))

    def test_a_library_the_restored_cache_holds_costs_nothing(self):
        _, load = ci.shard_crates(self.WORKSPACE, ["app"], self.COSTS, frozenset(self.WORKSPACE), 1)
        self.assertEqual(load, 1200 + 300 + 300 + 600)
        _, load = ci.shard_crates(self.WORKSPACE, ["app"], self.COSTS, frozenset({"app"}), 1)
        self.assertEqual(load, 1200 + 300)
        _, decisions, _ = self.sharded(["crates/mid/src/lib.rs"])
        self.assertEqual(next(d for d in decisions if d.job.name == "test").dirty, {"mid", "app"})
        _, decisions, _ = self.sharded(["crates/mid/src/lib.rs", "Cargo.lock"])
        self.assertEqual(next(d for d in decisions if d.job.name == "test").dirty, set(self.WORKSPACE))
        tested_only = dict(self.WORKSPACE, app=ci.Crate("app", "crates/app", frozenset({"mid", "hub"}), frozenset({"hub"})))
        self.assertEqual(ci.dependents_closure(tested_only, {"mid"}), {"mid"})

    def test_a_runner_is_added_only_while_it_saves_more_than_the_slack(self):
        pair = crates(a=[], b=[])
        small = {"a": (0, ci.LEG_SLACK / 2), "b": (0, ci.LEG_SLACK / 2)}
        self.assertEqual(ci.split_crates(pair, ["a", "b"], small, frozenset(pair), 4), [["a", "b"]])
        large = {"a": (0, 4 * ci.LEG_SLACK), "b": (0, 4 * ci.LEG_SLACK)}
        self.assertEqual(ci.split_crates(pair, ["a", "b"], large, frozenset(pair), 4), [["a"], ["b"]])
        self.assertEqual(ci.split_crates(pair, ["a"], large, frozenset(pair), 4), [["a"]])

    def test_a_crate_the_table_lacks_takes_the_median_and_no_table_weighs_all_alike(self):
        weights = ci.load_weights(crates(a=[], b=[], c=[], new=[]), self.table({"a": (1, 10), "b": (2, 20), "c": (3, 30), "gone": (9, 90)}))
        self.assertEqual(weights, {"a": (1, 10), "b": (2, 20), "c": (3, 30), "new": (2, 20)})
        self.assertEqual(ci.load_weights(crates(a=[]), ""), {"a": (1, 1)})
        for broken in ("not json", '{"crates": {"a": {"build": 1}}}', '{"crates": {"a": {"build": "x", "test": 1}}}',
                       '{"crates": {"a": {"build": -1, "test": 1}}}', '{"crates": []}'):
            with self.assertRaises(SystemExit, msg=broken):
                ci.load_weights({}, broken)


    def test_a_cold_runs_timings_and_test_report_give_each_crate_its_build_and_test(self):
        units = [{"name": "app", "target": "", "duration": 10}, {"name": "app", "target": " build-script (run)", "duration": 1},
                 {"name": "app", "target": ' app "lib" (test)', "duration": 20}, {"name": "app", "target": ' x "bin" (test)', "duration": 5},
                 {"name": "app", "target": ' e "example"', "duration": 2}, {"name": "serde", "target": "", "duration": 99}]
        timings = f"<script>const UNIT_DATA = {ci.json.dumps(units)};\nconst CONCURRENCY_DATA = [];</script>"
        report = ('<testsuites><testsuite name="app"><testcase name="a" time="1.5"/></testsuite>'
                  '<testsuite name="app::bin/x"><testcase name="b" time="0.5"/></testsuite></testsuites>')
        self.assertEqual(ci.measured_costs(timings, report, crates(app=[])), {"app": [11.0, 29.0]})

    def test_changes_and_each_runner_split_a_job_alike(self):
        ctx, decisions, table = self.sharded(["crates/hub/src/lib.rs"])
        legs = ci.matrix_legs(ctx.pipeline, decisions, ctx.crates, table)
        tests = [leg for leg in legs if leg["jobs"] == "test"]
        self.assertEqual([leg["shard"].rsplit("/", 1)[0] for leg in tests], ["1/2", "2/2"])
        self.assertEqual([leg["name"] for leg in tests], ["test 1/2", "test 2/2"])
        self.assertTrue(all(leg["needs_python"] for leg in legs))
        self.assertIn("clippy", next(leg for leg in legs if leg["shard"] == "")["jobs"].split())
        self.assertEqual({leg["name"]: leg["cache"] for leg in legs}, {"all": "group-all", "test 1/2": "test", "test 2/2": "test"})
        taken = []
        for leg in tests:
            cut, refused = ci.take_shard([d for d in decisions if d.job.name == "test"], leg["shard"], ctx.crates, table)
            self.assertEqual(refused, [])
            taken += cut[0].arguments[1::2]
        self.assertEqual(sorted(taken), ["app", "hub", "mid", "tool"])

    def test_a_runner_that_would_split_otherwise_fails_its_leg(self):
        ctx, decisions, table = self.sharded(["crates/hub/src/lib.rs"])
        mine = [d for d in decisions if d.job.name == "test"]
        legs = [leg for leg in ci.matrix_legs(ctx.pipeline, decisions, ctx.crates, table) if leg["jobs"] == "test"]
        digest = legs[0]["shard"].rsplit("/", 1)[1]
        for shard in (f"3/3/{digest}", f"1/1/{digest}", "1/2/000000000000", "1/2", "x"):
            _, refused = ci.take_shard(mine, shard, ctx.crates, table)
            self.assertEqual([o.state for o in refused], ["fail"], shard)
        moved = self.table(dict(self.COSTS, tool=(100, 5000)))
        _, refused = ci.take_shard(mine, legs[0]["shard"], ctx.crates, moved)
        self.assertEqual([o.state for o in refused], ["fail"])

    def test_everything_tests_the_workspace_and_names_what_lies_beyond_a_pull_requests_scope(self):
        _, decisions, _ = self.sharded(["crates/leaf/src/lib.rs"], tier="main", everything=True)
        decision = next(d for d in decisions if d.job.name == "test")
        self.assertEqual(decision.arguments, ["--workspace"])
        self.assertEqual(decision.beyond, ["app", "hub", "mid", "tool"])
        self.assertIn("workspace-wide; 4 beyond a pull request's scope", decision.detail)
        _, decisions, _ = self.sharded(["crates/leaf/src/lib.rs"], tier="main")
        self.assertEqual(next(d for d in decisions if d.job.name == "test").arguments, ["-p", "leaf", "-p", "other"])
        _, decisions, _ = self.sharded(["docs/x.md"], tier="main", everything=True)
        decision = next(d for d in decisions if d.job.name == "test")
        self.assertEqual((decision.state, decision.arguments), ("run", ["--workspace"]))
        self.assertEqual(len(decision.beyond), len(self.WORKSPACE))
        self.assertEqual(next(d for d in decisions if d.job.name == "scan").state, "run")


    def test_only_a_crates_job_shards_and_neither_an_unsharded_crates_job_nor_a_files_job_warms(self):
        with self.assertRaises(SystemExit):
            ci.load_pipeline(PIPELINE.replace('command = ["scan"]\n', 'command = ["scan"]\nshards = 2\n'))
        with self.assertRaises(SystemExit):
            ci.load_pipeline(PIPELINE.replace('kind = "crates"\ntoolchain = "rust"\ncommand = ["boat"', 'kind = "crates"\ntoolchain = "rust"\nwarm = ["cargo"]\ncommand = ["boat"'))
        with self.assertRaises(SystemExit):
            ci.load_pipeline(PIPELINE.replace('command = ["scan", "{files}"]\n', 'command = ["scan", "{files}"]\nwarm = ["cargo"]\n'))
        warmed = ci.load_pipeline(PIPELINE.replace('command = ["scan"]\n', 'command = ["scan"]\nwarm = ["cargo"]\n'))
        self.assertEqual(next(job for job in warmed.jobs if job.name == "scan").warm, ("cargo",))


class RestoredTarget(unittest.TestCase):
    TRACKED = ["crates/a/src/lib.rs", "crates/a/src/x.rs", "crates/b/src/lib.rs", "Cargo.lock"]

    def test_a_changed_path_and_every_folder_above_it_are_newer(self):
        self.assertEqual(ci.newer_paths([("M", "crates/a/src/x.rs"), ("M", "Cargo.lock")], self.TRACKED),
                         {"crates", "crates/a", "crates/a/src", "crates/a/src/x.rs", "Cargo.lock"})

    def test_a_file_added_or_removed_makes_its_whole_crate_newer(self):
        for status in ("A", "D"):
            newer = ci.newer_paths([(status, "crates/a/src/native.rs")], self.TRACKED)
            self.assertLessEqual({"crates/a/src/lib.rs", "crates/a/src/x.rs", "crates/a/src/native.rs"}, newer, status)
            self.assertNotIn("crates/b/src/lib.rs", newer, status)

    def test_pruning_keeps_the_units_the_build_used_and_no_executables(self):
        import tempfile
        used, unused = "0123456789abcdef", "fedcba9876543210"
        messages = "\n".join([
            ci.json.dumps({"reason": "compiler-artifact", "filenames": [f"D:\\t\\debug\\deps\\liba-{used}.rlib"]}),
            ci.json.dumps({"reason": "build-script-executed", "out_dir": "D:\\t\\debug\\build\\a-1111222233334444\\out"}),
            "not json"])
        live = ci.live_units(messages)
        self.assertEqual(live, {used, "1111222233334444"})
        with tempfile.TemporaryDirectory() as folder:
            profile = Path(folder)
            for path in (f"deps/liba-{used}.rlib", f"deps/liba-{unused}.rlib", f"deps/a-{used}.exe", f".fingerprint/a-{unused}/lib-a",
                         f".fingerprint/a-{used}/lib-a", "build/a-1111222233334444/out/x", "a.exe", "deps/kept.txt"):
                Path(profile, path).parent.mkdir(parents=True, exist_ok=True)
                Path(profile, path).write_text("x", encoding="utf-8")
            ci.prune(profile, live)
            left = sorted(path.relative_to(profile).as_posix() for path in profile.rglob("*") if path.is_file())
        self.assertEqual(left, sorted(["build/a-1111222233334444/out/x", f".fingerprint/a-{used}/lib-a", "deps/kept.txt", f"deps/liba-{used}.rlib"]))

    def test_a_changed_job_definition_warms_again_though_no_rust_input_changed(self):
        import tempfile
        original = ci.PIPELINE
        head = ci.git("rev-parse", "HEAD").strip()
        committed = ci.git("show", "HEAD:tools/ci.toml")
        with tempfile.TemporaryDirectory() as folder:
            def saved(text):
                pipeline, output = Path(folder, "ci.toml"), Path(folder, "output")
                pipeline.write_text(text, encoding="utf-8")
                output.write_text("", encoding="utf-8")
                ci.PIPELINE = pipeline
                ci.warm("group-rust", Path(folder, "target"), output)
                return output.read_text(encoding="utf-8").strip()
            try:
                Path(folder, "target").mkdir()
                Path(folder, "target", ci.BUILT_FROM).write_text(head + "\n", encoding="utf-8")
                self.assertEqual(saved(committed), "save=false")
                changed = re.sub(r"^warm = \[.*\]$", lambda _: "warm = " + ci.json.dumps([sys.executable, "-c", "pass"]), committed, flags=re.M)
                self.assertNotEqual(changed, committed)
                self.assertEqual(saved(changed), "save=true")
            finally:
                ci.PIPELINE = original

    def test_only_a_build_that_succeeded_names_the_commit(self):
        import tempfile
        original = ci.PIPELINE
        with tempfile.TemporaryDirectory() as folder:
            def run(code):
                pipeline = Path(folder, "ci.toml")
                pipeline.write_text(PIPELINE.replace('command = ["boat", "test", "{packages}"]\n',
                                                     'command = ["boat", "test", "{packages}"]\nshards = 2\nwarm = '
                                                     + ci.json.dumps([sys.executable, "-c", f"import sys; sys.exit({code})"]) + "\n"), encoding="utf-8")
                ci.PIPELINE = pipeline
                output = Path(folder, f"output-{code}")
                ci.warm("test", Path(folder, "target"), github_output=output)
                return Path(folder, "target", ci.BUILT_FROM), output.read_text(encoding="utf-8")
            try:
                Path(folder, "target", "debug").mkdir(parents=True)
                named, output = run(1)
                self.assertFalse(named.exists())
                self.assertEqual(output, "save=false\n")
                named, output = run(0)
                self.assertEqual(named.read_text(encoding="utf-8").strip(), ci.git("rev-parse", "HEAD").strip())
                self.assertEqual(output, "save=true\n")
            finally:
                ci.PIPELINE = original

    def test_a_groups_cache_holds_every_build_its_jobs_warm_and_none_when_one_fails(self):
        stale = "fedcba9876543210"
        original = ci.PIPELINE
        with tempfile.TemporaryDirectory() as folder:
            def build(unit, code=0):
                message = ci.json.dumps({"reason": "compiler-artifact", "filenames": [f"t/debug/deps/lib{unit}-{unit * 16}.rlib"]})
                return ci.json.dumps([sys.executable, "-c", f"import sys; print({message!r}); sys.exit({code})"])

            def run(second):
                pipeline = Path(folder, "ci.toml")
                pipeline.write_text(PIPELINE.replace('command = ["scan"]\n', f'command = ["scan"]\nwarm = {build("a")}\n')
                                    .replace('command = ["slow"]\n', f'command = ["slow"]\nwarm = {second}\n'), encoding="utf-8")
                ci.PIPELINE = pipeline
                output = Path(folder, "output")
                output.write_text("", encoding="utf-8")
                ci.warm("group-all", Path(folder, "target"), github_output=output)
                return output.read_text(encoding="utf-8")

            try:
                deps = Path(folder, "target", "debug", "deps")
                deps.mkdir(parents=True)
                Path(folder, "target", "debug", ".fingerprint").mkdir()
                for name in (f"liba-{'a' * 16}.rlib", f"libb-{'b' * 16}.rlib", f"libc-{stale}.rlib"):
                    Path(deps, name).write_text("x", encoding="utf-8")
                self.assertEqual(run(build("b", code=1)), "save=false\n")
                self.assertEqual(len(list(deps.iterdir())), 3)
                self.assertEqual(run(build("b")), "save=true\n")
                self.assertEqual(sorted(path.name for path in deps.iterdir()), [f"liba-{'a' * 16}.rlib", f"libb-{'b' * 16}.rlib"])
            finally:
                ci.PIPELINE = original



class Local(unittest.TestCase):
    def test_every_job_that_compiles_or_tests_is_left_to_ci(self):
        ctx = context([])
        ctx.pipeline = ci.load_pipeline((ROOT / "tools" / "ci.toml").read_text(encoding="utf-8"))
        sent = []
        originals = ci.plan, ci.run_all, ci.report
        ci.plan = lambda tier, base: (ctx, {}, [ci.Decision(job, "run") for job in ctx.pipeline.jobs], "HEAD")
        ci.run_all = lambda runner, decisions: sent.extend(decisions) or []
        ci.report = lambda outcomes, label: 0
        try:
            with contextlib.redirect_stdout(io.StringIO()):
                ci.check(argparse.Namespace(base="HEAD"))
        finally:
            ci.plan, ci.run_all, ci.report = originals
        runs_here = [decision.job for decision in sent]
        self.assertTrue(runs_here)
        self.assertFalse([job.name for job in runs_here if job.kind == "crates" or re.search(r"(^|-)tests?$", job.name)], [job.name for job in runs_here])
        compiles = [job.name for job in runs_here if {"cargo", "tools/boat.py"} & set(job.command)]
        self.assertEqual(compiles, [])


class Pipeline(unittest.TestCase):
    def test_the_pipeline_loads_and_runs_every_repo_check(self):
        pipeline = ci.load_pipeline((ROOT / "tools" / "ci.toml").read_text(encoding="utf-8"))
        commands = " ".join(part for job in pipeline.jobs for part in job.command)
        scripts = sorted(path.name for path in (ROOT / "tools").glob("check-*.py"))
        self.assertTrue(scripts)
        self.assertEqual([script for script in scripts if f"tools/{script}" not in commands], [])

    def test_the_cache_workflow_warms_every_rust_legs_cache_and_restores_what_ci_restores(self):
        pipeline = ci.load_pipeline((ROOT / "tools" / "ci.toml").read_text(encoding="utf-8"))
        cache = yaml.safe_load((ROOT / ".github" / "workflows" / "rust-cache.yml").read_text(encoding="utf-8"))
        ci_workflow = yaml.safe_load((ROOT / ".github" / "workflows" / "ci.yml").read_text(encoding="utf-8"))
        warm = cache["jobs"]["warm"]
        self.assertEqual(sorted(warm["strategy"]["matrix"]["cache"]), ci.caches(pipeline))
        builds = [job for job in pipeline.jobs if "rust" in pipeline.groups[job.group].get("setup", [])]
        self.assertEqual([job.name for job in builds if not job.warm], [])
        steps = [step["with"] for step in [*warm["steps"], *ci_workflow["jobs"]["run"]["steps"]] if str(step.get("uses", "")).startswith("actions/cache/")]
        self.assertEqual(len(steps), 3)
        self.assertEqual(len({step["path"] for step in steps}), 1)
        self.assertEqual(len({step["key"] for step in steps}), 1)
        self.assertIn("matrix.cache", steps[0]["key"])
        self.assertEqual(len({step["restore-keys"] for step in steps if "restore-keys" in step}), 1)

    def test_an_unknown_tier_is_refused(self):
        with self.assertRaises(SystemExit):
            ci.load_pipeline(re.sub(r'(name = "scan"\ngroup = "all"\ntier = )"check"', r'\1"weekly"', PIPELINE))

    def test_nextest_deps_dir_resolves_profile_and_target_dir(self):
        root = Path("repo")
        self.assertEqual(
            ci.nextest_deps_dir(["cargo", "nextest", "run", "--cargo-profile", "ci"], root=root, env={}),
            root / ".cargo-target" / "ci" / "deps",
        )
        self.assertEqual(
            ci.nextest_deps_dir(["cargo", "nextest", "run", "--cargo-profile=release"], root=root, env={"CARGO_TARGET_DIR": "custom"}),
            Path("custom") / "release" / "deps",
        )
        self.assertEqual(
            ci.nextest_deps_dir(["cargo", "nextest", "run"], root=root, env={}),
            root / ".cargo-target" / "debug" / "deps",
        )

    def test_stage_shader_compiler_copies_cached_dlls_beside_nextest_binaries(self):
        import tempfile
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            cache = root / ".cargo-target" / "shader-compiler"
            cache.mkdir(parents=True)
            for name in ci.SHADER_COMPILER:
                (cache / name).write_bytes(name.encode("ascii"))
            log = root / "stage.log"
            code = ci.stage_shader_compiler(["cargo", "nextest", "run", "--cargo-profile", "ci"], log, root=root, env={})
            self.assertEqual(code, 0)
            deps = root / ".cargo-target" / "ci" / "deps"
            for name in ci.SHADER_COMPILER:
                self.assertEqual((deps / name).read_bytes(), name.encode("ascii"))

    def test_windows_stages_dxc_for_the_shards_test_dependencies(self):
        workspace = crates(**{"shader-pipeline": [], "renderer": ["shader-pipeline"], "consumer": ["fixture"], "other": []})
        workspace["fixture"] = ci.Crate("fixture", "crates/fixture", frozenset({"renderer"}), frozenset())
        pipeline = ci.load_pipeline(PIPELINE.replace('name = "test"', 'name = "rust-test"'))
        job = next(job for job in pipeline.jobs if job.name == "rust-test")
        with tempfile.TemporaryDirectory() as folder, patch.object(ci.sys, "platform", "win32"):
            runner = ci.Runner(Path(folder), {}, workspace)
            with patch.object(ci, "stage_shader_compiler", return_value=0) as stage, patch.object(runner, "execute", return_value=0):
                for shard, expected in [(["shader-pipeline"], True), (["other", "fixture"], True), (["consumer"], False)]:
                    with self.subTest(shard=shard):
                        stage.reset_mock()
                        self.assertEqual(runner.run(ci.Decision(job, "run", crates=shard)).state, "pass")
                        self.assertEqual(stage.called, expected)


if __name__ == "__main__":
    unittest.main()
