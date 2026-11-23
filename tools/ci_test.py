"""Tests for tools/ci.py: python tools/ci_test.py"""

import argparse
import contextlib
import importlib.util
import io
from pathlib import Path
import re
import sys
import unittest
from xml.sax.saxutils import escape

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
    return ci.Context(set(tree), set(changed), workspace, pipeline, definitions)


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

    def test_an_unchanged_workspace_skips(self):
        self.assertEqual(decide("clippy", context(["docs/x.md"])).state, "skipped")

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

    def test_a_changed_runner_script_selects_no_crates(self):
        self.assertEqual(decide("clippy", context(["tools/boat.py"])).state, "skipped")

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
        ctx.manifests = {"fuzz/Cargo.toml": {"mid"}}
        self.assertEqual(decide("fuzz", ctx).state, "skipped")
        ctx.changed = {"crates/mid/src/lib.rs"}
        self.assertEqual(decide("fuzz", ctx).state, "run")
        ctx.manifests = {"fuzz/Cargo.toml": None}
        ctx.changed = {"crates/core/src/lib.rs"}
        self.assertEqual(decide("fuzz", ctx).state, "run")



class Verdict(unittest.TestCase):
    def outcomes(self, *states):
        return [ci.Outcome(f"job{index}", state) for index, state in enumerate(states)]

    def test_a_not_run_rule_names_its_crates_and_its_panic(self):
        for rule in ('{ crates = ["a"] }', "{ panic = 'x' }", '{ crates = [], panic = \'x\' }'):
            with self.assertRaises(SystemExit, msg=rule):
                ci.load_pipeline(PIPELINE.replace('command = ["boat", "test", "{packages}"]\n',
                                                  'command = ["boat", "test", "{packages}"]\nreport = "r.xml"\nnot-run = { "why" = ' + rule + ' }\n'))

    def test_a_job_that_could_not_run_is_never_a_success(self):
        self.assertEqual(ci.verdict(self.outcomes("pass", "not run", "skipped")), "error")
        self.assertEqual(ci.verdict(self.outcomes("pass", "not run", "fail")), "failure")
        self.assertEqual(ci.verdict(self.outcomes("pass", "skipped", "later", "ci only")), "success")

    def test_a_failure_a_rule_names_is_not_run_and_every_other_failure_still_fails(self):
        fixture = ("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests: Custom { kind: NotFound, "
                   "error: \"BATTLEPLAN_GAME_DIR is unset; executable metadata tests require an explicit supported installation\" }")
        unset = "effect grammar tests require BATTLEPLAN_GAME_DIR with the supported installation"
        metadata = "BATTLEPLAN_GAME_DIR must name the supported installation for executable metadata"
        other_error = unset + ": Custom { kind: InvalidData, error: \"table digest mismatch\" }"

        def panic(thread, message):
            return f"thread '{thread}' (4242) panicked at crates/a/src/lib.rs:1:2:\n{message}\nnote: run with `RUST_BACKTRACE=1`\n"

        def case(name, stderr, kind="test failure with exit code 101"):
            return f'<testcase name="{name}"><failure type="{kind}"/><system-err>{escape(stderr)}</system-err></testcase>'
        report = "<testsuites><testsuite name=\"dashboard-base\"><testcase name=\"passes\"/><testcase name=\"ignored\"><skipped/></testcase>" + "".join([
            case("a::fixture", panic("a::fixture", fixture)),
            case("a::unset", panic("a::unset", unset)),
            case("a::metadata", panic("a::metadata", metadata)),
            case("a::other_error", panic("a::other_error", other_error)),
            case("a::leaks", panic("a::leaks", fixture), kind="test failure with exit code 101 (leaked handles)"),
            case("a::caught_then_returns_err", panic("a::caught_then_returns_err", fixture) + "Error: \"checksum mismatch\"\n"),
            case("a::asserts_on_the_name", panic("a::asserts_on_the_name", 'assertion failed: parsed.contains_key("BATTLEPLAN_GAME_DIR")')),
            case("a::caught_then_asserts", panic("a::caught_then_asserts", fixture) + panic("a::caught_then_asserts", "assertion failed: x")),
            case("a::worker", panic("worker", fixture)),
            case("a::times_out", panic("a::times_out", fixture), kind="test timeout"),
        ]) + "</testsuite><testsuite name=\"viewer-raids\">" + case("a::fixture", panic("a::fixture", fixture)) + "</testsuite></testsuites>"
        rules = list(next(job for job in ci.load_pipeline(ci.PIPELINE.read_text(encoding="utf-8")).jobs
                          if job.name == "rust-test").not_run)
        passed, failures, held = ci.sorted_failures(report, rules)
        self.assertEqual(passed, 1)
        self.assertEqual([test for test, _ in failures], [f"dashboard-base a::{name}" for name in
                                                          ("other_error", "leaks", "caught_then_returns_err",
                                                           "asserts_on_the_name", "caught_then_asserts", "worker", "times_out")]
                         + ["viewer-raids a::fixture"])
        reason = "needs the game install until the dashboard rework (#169)"
        self.assertEqual(held, {reason: {"dashboard-base": 3}})
        self.assertIn("table digest mismatch", failures[0][1])
        self.assertEqual(ci.held_detail(held), f"3 not run here: {reason}")
        lines = ci.failure_report((passed, failures, held))
        self.assertIn(f"not run here: {reason}: 3 tests, in dashboard-base (3)", lines)
        self.assertEqual(sum(line.startswith("--- FAIL ") for line in lines), 8)
        self.assertIsNone(ci.sorted_failures("not a report", rules))
        summary = "        Summary [ 12.000s] {} tests run: 1 passed, 11 failed, 1 skipped\n"
        self.assertTrue(ci.finished_every_test(summary.format(12), (passed, failures, held)))
        self.assertFalse(ci.finished_every_test(summary.format(13), (passed, failures, held)))
        self.assertFalse(ci.finished_every_test("  Cancelling due to interrupt: 1 test still running\n" + summary.format(12),
                                                (passed, failures, held)))
        self.assertFalse(ci.finished_every_test(summary.format("12/13"), (passed, failures, held)))
        self.assertFalse(ci.finished_every_test("error: test run cancelled\n", (passed, failures, held)))


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

    def test_a_split_is_the_same_whatever_order_a_runner_sums_costs_in(self):
        weights = ci.load_weights(crates(a=[], b=[], c=[]), self.table({"a": (0.1, 0.2), "b": (0.2, 0.3), "c": (0.3, 0.1)}))
        self.assertTrue(all(type(cost) is int for pair in weights.values() for cost in pair))
        self.assertEqual(sum(cost for cost, _ in weights.values()), sum(cost for cost, _ in reversed(list(weights.values()))))

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

    def test_a_failure_beyond_a_pull_requests_scope_says_so(self):
        lines = ci.failure_report((3, [("app t::one", "boom"), ("leaf::bin/x t::two", "bang")], {}), {"app"})
        self.assertIn("--- FAIL app t::one (beyond a pull request's scope)", lines)
        self.assertIn("--- FAIL leaf::bin/x t::two", lines)
        self.assertTrue(any(line.startswith("1 of the failures") for line in lines))

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
                ci.warm("fuzz-build", Path(folder, "target"), output)
                return output.read_text(encoding="utf-8").strip()
            try:
                Path(folder, "target").mkdir()
                Path(folder, "target", ci.BUILT_FROM).write_text(head + "\n", encoding="utf-8")
                self.assertEqual(saved(committed), "save=false")
                job = next(job for job in ci.load_pipeline(committed).jobs if job.name == "fuzz-build")
                changed = committed.replace("warm = " + ci.json.dumps(list(job.warm)),
                                            "warm = " + ci.json.dumps([sys.executable, "-c", "pass"]), 1)
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



class Local(unittest.TestCase):
    def test_every_job_that_compiles_or_tests_is_left_to_ci(self):
        pipeline = ci.load_pipeline((ROOT / "tools" / "ci.toml").read_text(encoding="utf-8"))
        sent = []
        originals = ci.plan, ci.run_all, ci.report
        ci.plan = lambda tier, base: (pipeline, {}, [ci.Decision(job, "run") for job in pipeline.jobs], "HEAD")
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

    def test_the_cache_workflow_warms_every_sharded_job_and_restores_what_ci_restores(self):
        import yaml
        pipeline = ci.load_pipeline((ROOT / "tools" / "ci.toml").read_text(encoding="utf-8"))
        cache = yaml.safe_load((ROOT / ".github" / "workflows" / "rust-cache.yml").read_text(encoding="utf-8"))
        ci_workflow = yaml.safe_load((ROOT / ".github" / "workflows" / "ci.yml").read_text(encoding="utf-8"))
        warm = cache["jobs"]["warm"]
        self.assertEqual(sorted(warm["strategy"]["matrix"]["job"]), sorted(job.name for job in pipeline.jobs if job.warm))
        self.assertLessEqual({job.name for job in pipeline.jobs if job.shards}, {job.name for job in pipeline.jobs if job.warm})
        paths = [step["with"]["path"] for step in warm["steps"] if str(step.get("uses", "")).startswith("actions/cache/")]
        paths += [step["with"]["path"] for step in ci_workflow["jobs"]["run"]["steps"] if str(step.get("uses", "")).startswith("actions/cache/")]
        self.assertEqual(len(paths), 3)
        self.assertEqual(len(set(paths)), 1)

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


if __name__ == "__main__":
    unittest.main()
