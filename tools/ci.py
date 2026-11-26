"""Plan and run the jobs in tools/ci.toml (Python 3.11+).

usage: python tools/ci.py check [--base REF]
       python tools/ci.py changes --tier TIER --base REF [--exact-base] [--everything] [--github-output FILE]
       python tools/ci.py job --tier TIER --group GROUP --jobs NAMES --base REF [--exact-base] [--everything] [--shard AT/COUNT]
       python tools/ci.py times --target DIR
       python tools/ci.py warm --cache NAME --target DIR
       python tools/ci.py weights --timings HTML --junit XML [--out FILE]

GitHub's hosted CI (.github/workflows/ci.yml) is the only test runner and the only gate. Its
`changes` job gives each job its gate from the files alone and writes the matrix of legs; each leg
runs `job` for its group, or for its share of a sharded job. `times` prepares a target folder
restored from a cache, and `warm` fills one for the cache. `weights` writes the measured crate
costs the legs are balanced by, from a cold run's timings and test report. `check` is a quick run
here before a push: the jobs a change touches that compile and test nothing, so nothing goes to
Boat. Exit 0 on success, 1 on a failure, 2 when a job that should have run could not.
"""
import argparse
from concurrent.futures import ThreadPoolExecutor
from dataclasses import dataclass, field
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import time
import tomllib
import xml.etree.ElementTree as ElementTree

ROOT = Path(__file__).resolve().parents[1]
PIPELINE_PATH = "tools/ci.toml"
PIPELINE = ROOT / PIPELINE_PATH
NEXTEST_TESTS_FAILED = 100
SHOWN_LINES = 40
COMMAND_LIMIT = 24_000
KINDS = {"command", "files", "crates"}
ORDER = ("fail", "not run", "pass", "skipped", "later", "ci only")
EXIT = {"success": 0, "failure": 1, "error": 2}
UNCHANGED = "no input changed since the base"
BUILT_FROM = "ci-built-from"
SETTLED = 946684800
UNIFIED = "cfg(all())"
UNIFY = "test-unify"
WEIGHTS = ROOT / "tools" / "ci-weights.json"
LEG_SLACK = 240
GROUP_CACHE = "group-"
SHADER_COMPILER = ("dxcompiler.dll", "dxil.dll")


def nextest_deps_dir(command, root=ROOT, env=None):
    """The folder where `cargo nextest` places compiled test binaries (`<target>/<profile>/deps`)."""
    profile = "debug"
    for index, part in enumerate(command):
        if part == "--cargo-profile" and index + 1 < len(command):
            profile = command[index + 1]
            break
        if part.startswith("--cargo-profile="):
            profile = part.split("=", 1)[1]
            break
    target_env = (os.environ if env is None else env).get("CARGO_TARGET_DIR")
    target = Path(target_env) if target_env else root / ".cargo-target"
    return target / profile / "deps"


def stage_shader_compiler(command, log, root=ROOT, env=None):
    """Stages the pinned DXC DLLs beside `cargo nextest`'s test binaries via tools/install-shader-compiler.ps1."""
    deps = nextest_deps_dir(command, root=root, env=env)
    cache = deps.parent.parent / "shader-compiler"
    deps.mkdir(parents=True, exist_ok=True)
    if not all((cache / name).is_file() for name in SHADER_COMPILER):
        installer = ["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File",
                     str(root / "tools" / "install-shader-compiler.ps1"), "-Destination", str(cache)]
        with open(log, "ab") as out:
            out.write(f"$ {' '.join(installer)}\n".encode())
            out.flush()
            try:
                code = subprocess.run(installer, cwd=root, stdout=out, stderr=subprocess.STDOUT,
                                      stdin=subprocess.DEVNULL).returncode
            except OSError as error:
                out.write(f"could not start: {error}\n".encode())
                return 127
        if code != 0:
            return code
    for name in SHADER_COMPILER:
        shutil.copy2(cache / name, deps / name)
    return 0


@dataclass(frozen=True)
class Job:
    name: str
    tier: str
    kind: str
    toolchain: str
    inputs: tuple
    shared: tuple
    command: tuple
    ci_only: bool
    group: str
    whole: tuple
    base: bool
    widen: tuple
    report: str
    shards: int
    warm: tuple
    dependents: bool
    manifest: str
    package: str
    definition: str


@dataclass
class Pipeline:
    tiers: list
    toolchains: dict
    jobs: list
    groups: dict = field(default_factory=dict)
    implementation: tuple = ()
    frame: str = ""


def load_pipeline(text):
    data = tomllib.loads(text)
    tiers = list(data["tiers"])
    toolchains = {name: tuple(table.get("files", ())) for name, table in data["toolchain"].items()}
    groups = dict(data.get("group", {}))
    jobs = []
    for table in data["job"]:
        job = Job(
            name=table["name"], tier=table["tier"], kind=table.get("kind", "command"),
            toolchain=table["toolchain"], inputs=tuple(table.get("inputs", ())),
            shared=tuple(table.get("shared", ())), command=tuple(table.get("command", ())),
            ci_only=table.get("ci-only", False),
            group=table.get("group", ""), whole=tuple(table.get("whole", ())), base=table.get("base", False),
            widen=tuple(table.get("widen", ())),
            report=table.get("report", ""),
            shards=table.get("shards", 0), warm=tuple(table.get("warm", ())), dependents=table.get("dependents", True),
            manifest=table.get("manifest", ""), package=table.get("package", ""),
            definition=json.dumps([table, data["toolchain"].get(table["toolchain"]), groups.get(table.get("group"))], sort_keys=True))
        problems = [f"tier {job.tier!r}"] if job.tier not in tiers else []
        problems += [f"kind {job.kind!r}"] if job.kind not in KINDS else []
        problems += [f"toolchain {job.toolchain!r}"] if job.toolchain not in toolchains else []
        problems += [] if job.command else ["command"]
        problems += [f"group {job.group!r}"] if job.group not in groups else []
        problems += ["name, used twice"] if job.name in {known.name for known in jobs} else []
        problems += [f"shards {job.shards!r}, a positive count for a crates job"] if job.shards and (
            job.kind != "crates" or not isinstance(job.shards, int) or job.shards < 1) else []
        problems += ["warm, a command for a crates job with shards or a command job"] if job.warm and (
            job.kind == "files" or job.kind == "crates" and not job.shards) else []
        problems += ["manifest or package, one of them and for a command job"] if (job.manifest or job.package) and (
            job.kind != "command" or job.manifest and job.package) else []
        if problems:
            raise SystemExit(f"tools/ci.toml: job {job.name}: unknown or missing {', '.join(problems)}")
        jobs.append(job)
    frame = json.dumps({key: value for key, value in data.items() if key != "job"}, sort_keys=True)
    return Pipeline(tiers, toolchains, jobs, groups, tuple(data.get("pipeline", {}).get("implementation", ())), frame)


def glob_pattern(pattern):
    """A regex for a path glob: `**` crosses folders, `*` and `?` stay within one."""
    out, at = [], 0
    while at < len(pattern):
        if pattern.startswith("**/", at):
            out.append("(?:.*/)?")
            at += 3
        elif pattern.startswith("**", at):
            out.append(".*")
            at += 2
        else:
            out.append({"*": "[^/]*", "?": "[^/]"}.get(pattern[at], re.escape(pattern[at])))
            at += 1
    return "".join(out)


def matcher(patterns):
    if not patterns:
        return lambda path: False
    compiled = re.compile("(?:" + "|".join(glob_pattern(pattern) for pattern in patterns) + r")\Z")
    return lambda path: compiled.match(path) is not None


@dataclass
class Crate:
    name: str
    folder: str
    deps: frozenset
    libraries: frozenset = frozenset()


def crates_from_manifests(root):
    """Each workspace member's folder, relative to `root`, the members it depends on through a path
    dependency of any kind or target, and those its library itself builds on (normal and build
    dependencies, not dev ones), read from the manifests themselves so that planning needs no Rust
    toolchain; None when the root manifest cannot be read. The dev-dependencies that
    `tools/check-feature-unification.py` generates under `cfg(all())` are not edges, bar `test-unify`
    which they alone reach: the rest only set features on crates the member's tests already build
    through its own dependencies."""
    root = Path(root)
    key = lambda path: os.path.normcase(os.path.normpath(str(path)))
    try:
        workspace = tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))["workspace"]
    except (OSError, ValueError, KeyError):
        return None
    excluded = {key(root / path) for path in workspace.get("exclude", [])}
    shared = {name: root / spec["path"] for name, spec in workspace.get("dependencies", {}).items()
              if isinstance(spec, dict) and "path" in spec}
    members = {}
    for pattern in workspace.get("members", []):
        for manifest in sorted(root.glob(f"{pattern}/Cargo.toml")):
            if key(manifest.parent) in excluded:
                continue
            try:
                data = tomllib.loads(manifest.read_text(encoding="utf-8"))
            except (OSError, ValueError):
                return None
            if "package" in data:
                members[key(manifest.parent)] = (manifest.parent, data)
    names = {folder: data["package"]["name"] for folder, (_, data) in members.items()}
    crates = {}
    for folder, (path, data) in members.items():
        sections = [(kind, data.get(kind, {})) for kind in ("dependencies", "dev-dependencies", "build-dependencies")]
        for platform, target in data.get("target", {}).items():
            for kind in ("dependencies", "dev-dependencies", "build-dependencies"):
                table = target.get(kind, {})
                pinned = (platform, kind) == (UNIFIED, "dev-dependencies")
                sections.append((kind, {name: spec for name, spec in table.items() if name == UNIFY} if pinned else table))
        deps, libraries = set(), set()
        for kind, section in sections:
            for name, spec in section.items():
                if not isinstance(spec, dict):
                    continue
                found = shared.get(name) if spec.get("workspace") else path / spec["path"] if "path" in spec else None
                if found is not None:
                    deps.add(names.get(key(found)))
                    if kind != "dev-dependencies":
                        libraries.add(names.get(key(found)))
        name = names[folder]
        crates[name] = Crate(name, path.relative_to(root).as_posix(), frozenset(deps - {None, name}), frozenset(libraries - {None, name}))
    return crates


def manifest_reach(root, manifest, crates):
    """The workspace crates a manifest outside the workspace builds: its path dependencies of any
    kind, and every library they build on, at any depth; None when it cannot be read."""
    root, key = Path(root), lambda path: os.path.normcase(os.path.normpath(str(path)))
    try:
        data = tomllib.loads((root / manifest).read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return None
    folders = {key(root / crate.folder): crate.name for crate in crates.values()}
    kinds = ("dependencies", "dev-dependencies", "build-dependencies")
    sections = [data.get(kind, {}) for kind in kinds] + [target.get(kind, {}) for target in data.get("target", {}).values() for kind in kinds]
    here = (root / manifest).parent
    return library_reach(crates, {folders.get(key(here / spec["path"])) for section in sections for spec in section.values()
                                  if isinstance(spec, dict) and "path" in spec} - {None})


def library_reach(crates, names):
    """`names` and every workspace crate their libraries build on, at any depth."""
    reach, pending = set(names), list(names)
    while pending:
        for dep in crates[pending.pop()].libraries - reach:
            reach.add(dep)
            pending.append(dep)
    return reach


def test_reach(crates, names):
    dependencies = {dep for name in names for dep in crates[name].deps}
    return set(names) | library_reach(crates, dependencies)


def job_reach(root, job, crates):
    """The workspace crates a job's `manifest` or `package` builds; None when the manifest cannot
    be read. A package the workspace lacks is refused by name."""
    if job.manifest:
        return manifest_reach(root, job.manifest, crates)
    if job.package not in crates:
        raise SystemExit(f"tools/ci.toml: job {job.name}: package {job.package!r} is not a workspace crate")
    return library_reach(crates, {job.package})


def crate_owner(crates):
    """path -> the crate whose folder holds it, or None; the deepest folder wins."""
    folders = {crate.folder: crate.name for crate in crates.values()}

    def owner(path):
        parts = path.split("/")
        for end in range(len(parts) - 1, 0, -1):
            name = folders.get("/".join(parts[:end]))
            if name:
                return name
        return None
    return owner


def closure(crates, name):
    seen, pending = set(), [name]
    while pending:
        for dep in crates[pending.pop()].deps:
            if dep not in seen and dep != name:
                seen.add(dep)
                pending.append(dep)
    return seen


def dependents_closure(crates, names):
    """`names` and every crate whose library builds on one of them, directly or through others:
    the libraries a change makes a restored cache rebuild."""
    users = {}
    for crate in crates.values():
        for dep in crate.libraries:
            users.setdefault(dep, set()).add(crate.name)
    seen, pending = set(names), list(names)
    while pending:
        for user in users.get(pending.pop(), ()):
            if user not in seen:
                seen.add(user)
                pending.append(user)
    return seen


def load_weights(crates, text):
    """Each crate's measured cost, (build, test): the core-seconds its library build takes, and
    its test builds and test runs, from the committed table `ci.py weights` writes, in whole
    seconds so that a sum is exact in any order and every runner splits alike. A crate the table
    lacks takes the median of those it has. A table that is not one, or an entry without both
    costs as non-negative numbers, is refused, so a broken table fails the pull request that
    commits it rather than main."""
    try:
        table = json.loads(text).get("crates", {}) if text else {}
        costs = {name: (round(cost["build"]), round(cost["test"])) for name, cost in table.items()}
    except (ValueError, TypeError, KeyError, AttributeError) as error:
        raise SystemExit(f"tools/ci-weights.json is not a table of crate costs: {error!r}")
    wrong = sorted(name for name, (build, test) in costs.items() if build < 0 or test < 0)
    if wrong:
        raise SystemExit(f"tools/ci-weights.json: negative costs for {', '.join(wrong)}")
    known = {name: cost for name, cost in costs.items() if name in crates}
    builds, tests = sorted(cost[0] for cost in known.values()), sorted(cost[1] for cost in known.values())
    median = (builds[len(builds) // 2], tests[len(tests) // 2]) if known else (1, 1)
    return {name: known.get(name, median) for name in crates}


def shard_crates(crates, names, weights, dirty, count):
    """`names` split into `count` sorted lists, and the busiest list's cost: each crate's test
    builds and runs, and the library build of each crate it or its workspace dependencies need
    that is `dirty` (changed, or above a changed crate, so not fresh in the restored cache), once
    per runner. Greedy, the costliest crate first, each to the runner it adds least to the
    busiest of, so a crate joins the runner already building its dependencies. Pure and ordered,
    so `changes` and each runner agree without being told."""
    needs = {name: ({name} | closure(crates, name)) & dirty for name in names}
    own = {name: weights[name][1] + sum(weights[dep][0] for dep in needs[name]) for name in names}
    loads, built, shards = [0.0] * count, [set() for _ in range(count)], [[] for _ in range(count)]
    for name in sorted(names, key=lambda name: (-own[name], name)):
        added = [weights[name][1] + sum(weights[dep][0] for dep in needs[name] - built[at]) for at in range(count)]
        at = min(range(count), key=lambda at: (loads[at] + added[at], at))
        loads[at] += added[at]
        built[at] |= needs[name]
        shards[at].append(name)
    return [sorted(shard) for shard in shards], max(loads)


def split_crates(crates, names, weights, dirty, most):
    """The fewest runners, of at most `most`, whose busiest is within LEG_SLACK of the least busy
    any count achieves, split by shard_crates: a runner that saves less than that is not worth
    its setup and its place in the queue."""
    splits = [shard_crates(crates, names, weights, dirty, count) for count in range(1, max(1, min(most, len(names))) + 1)]
    best = min(load for _, load in splits)
    return next(split for split, load in splits if load <= best + LEG_SLACK)


@dataclass
class Context:
    """What a plan decides from: the files as they are, the paths changed since the base, the
    workspace crates (None when the manifests could not be read), the pipeline, each job's
    definition at the base (None when the base has no readable pipeline, so every job runs),
    whether the run takes every job across everything it covers, whatever changed (main's nightly,
    or a push to main with no green main commit to compare with), whether a crates job takes
    every crate above a changed one rather than its direct dependents (a push to main), and the
    workspace crates each job naming a `manifest` or a `package` builds, by job (None when they
    could not be read). `frame` is the base pipeline's settings outside its jobs (None when
    unreadable)."""
    tree: set
    changed: set
    crates: dict | None
    pipeline: Pipeline
    definitions: dict | None = None
    frame: str | None = None
    everything: bool = False
    transitive: bool = False
    reach: dict = field(default_factory=dict)


@dataclass
class Decision:
    job: Job
    state: str
    detail: str = ""
    arguments: list = field(default_factory=list)
    whole: bool = False
    crates: list = field(default_factory=list)
    beyond: list = field(default_factory=list)
    dirty: frozenset = frozenset()


def widened(ctx, job):
    """Whether a change reaches every input of the job: its definition differs from the base's, or
    a file every run of it depends on changed (its toolchain's, its shared scripts, the files it
    names in `widen`, or the pipeline's own implementation). An edit to tools/ci.toml that leaves
    everything outside its jobs as it was reaches only the jobs whose definition it changed."""
    if ctx.definitions is None or ctx.definitions.get(job.name) != job.definition:
        return True
    if PIPELINE_PATH in ctx.changed and ctx.frame != ctx.pipeline.frame:
        return True
    implementation = matcher(ctx.pipeline.implementation)
    own = matcher(ctx.pipeline.toolchains[job.toolchain] + job.shared + job.widen)
    return any(own(path) or path != PIPELINE_PATH and implementation(path) for path in ctx.changed)


def decide(job, ctx, tier):
    """Whether a job runs, and on what; pure, so the gates can be tested. A job of a later tier
    that would run is left to that tier."""
    decision = decide_inputs(job, ctx)
    if decision.state in ("run", "not run") and ctx.pipeline.tiers.index(job.tier) > ctx.pipeline.tiers.index(tier):
        return Decision(job, "later", f"{job.tier} tier" + (f": {decision.detail}" if decision.detail else ""))
    return decision


def decide_inputs(job, ctx):
    if job.kind == "crates":
        return decide_crates(job, ctx)
    patterns = job.inputs
    if job.manifest or job.package:
        reach = ctx.reach.get(job.name) if ctx.crates else None
        patterns += ("crates/**",) if reach is None else tuple(f"{ctx.crates[name].folder}/**" for name in sorted(reach))
    inputs = matcher(patterns)
    wide = ctx.everything or widened(ctx, job)
    if job.kind == "files":
        pool = ctx.tree if wide else ctx.changed & ctx.tree
        candidates = sorted(path for path in pool if inputs(path))
        if not candidates:
            return Decision(job, "skipped", UNCHANGED)
        return Decision(job, "run", f"{len(candidates)} files", candidates, wide)
    if not (wide or any(inputs(path) for path in ctx.changed)):
        return Decision(job, "skipped", UNCHANGED)
    return Decision(job, "run")


def decide_crates(job, ctx):
    """The changed crates and, unless the job says `dependents = false` (Clippy: a dependent's
    lints cannot change when its own source did not, and the tests compile it), their direct
    dependents, or on a transitive run every crate whose library builds on a changed one and
    every crate whose tests use one of those; the workspace when the change is wide, or on a run
    of everything; and the crates it takes that a pull request's scope leaves out. The crates a
    restored cache must rebuild are dirty: every crate when a toolchain file changed, or the
    changed crates and those above them."""
    wide = widened(ctx, job)
    if ctx.crates is None:
        rust = wide or ctx.everything or any(path.startswith("crates/") for path in ctx.changed)
        return Decision(job, "not run", "the Cargo manifests could not be read") if rust else Decision(job, "skipped", "no crate changed since the base")
    crates = ctx.crates
    owner = crate_owner(crates)
    changed = {owner(path) for path in ctx.changed} - {None}
    dependents = {crate.name for crate in crates.values() if crate.deps & changed} if job.dependents else set()
    toolchain = matcher(ctx.pipeline.toolchains[job.toolchain])
    built = dependents_closure(crates, changed)
    dirty = frozenset(crates) if any(toolchain(path) for path in ctx.changed) else frozenset(built)
    reach = built | {crate.name for crate in crates.values() if crate.deps & built} if ctx.transitive and job.dependents else changed | dependents
    selected = set(crates) if wide else reach
    if not selected and not ctx.everything:
        return Decision(job, "skipped", "no crate changed since the base")
    if ctx.everything:
        selected = set(crates)
    beyond = [] if wide else sorted(selected - changed - dependents)
    wide = wide or ctx.everything
    notes = (["workspace-wide"] if wide else []) + ([f"{len(beyond)} beyond a pull request's scope"] if beyond else [])
    detail = f"{len(selected)} crates" + (f" ({'; '.join(notes)})" if notes else "")
    names = sorted(selected)
    arguments = ["--workspace"] if wide else [arg for name in names for arg in ("-p", name)]
    return Decision(job, "run", detail, arguments, crates=names, beyond=beyond, dirty=dirty)


def expand(command, values, batch=None):
    """The command with each `{name}` replaced; `{files}` and `{packages}` spread into arguments."""
    out = []
    for part in command:
        if part in ("{files}", "{packages}"):
            out.extend(batch if batch is not None else values[part[1:-1]])
        else:
            out.append(part.format(**{k: v for k, v in values.items() if isinstance(v, str)}))
    return out


def batches(command, values, items):
    """The command once per run of items that fits on a Windows command line."""
    fixed = sum(len(part) + 1 for part in expand(command, values, []))
    chunk, size = [], fixed
    for item in items:
        if chunk and size + len(item) + 1 > COMMAND_LIMIT:
            yield expand(command, values, chunk)
            chunk, size = [], fixed
        chunk.append(item)
        size += len(item) + 1
    if chunk:
        yield expand(command, values, chunk)


@dataclass
class Outcome:
    name: str
    state: str
    detail: str = ""
    seconds: float = 0.0
    log: str | None = None


def verdict(outcomes):
    """Success only when every job passed, was skipped because its gate was not needed, or was
    left to a later tier or to CI."""
    states = {outcome.state for outcome in outcomes}
    if "fail" in states:
        return "failure"
    if "not run" in states:
        return "error"
    return "success"


def git(*args, check=True, data=None):
    done = subprocess.run(["git", *args], cwd=ROOT, capture_output=True, input=data)
    if check and done.returncode:
        raise SystemExit(f"git {' '.join(args)} failed: {done.stderr.decode(errors='replace').strip()}")
    return done.stdout.decode("utf-8", errors="replace")


def z(text):
    return [name for name in text.split("\0") if name]


def read_tree():
    """The files as they are now, tracked and untracked, without the deleted ones."""
    tree = set(z(git("ls-files", "-z"))) - set(z(git("ls-files", "-d", "-z")))
    return tree | set(z(git("ls-files", "-o", "--exclude-standard", "-z")))


def changed_since(base):
    return set(z(git("diff", "--name-only", "--no-renames", "-z", base))) | set(z(git("ls-files", "-o", "--exclude-standard", "-z")))


def failure_report(report, beyond=()):
    """The pass and failure counts and the start of each failed test's output from nextest's JUnit report."""
    try:
        root = ElementTree.fromstring(report)
    except ElementTree.ParseError:
        return None
    passed, failures = 0, []
    for suite in root.iter("testsuite"):
        for case in suite.iter("testcase"):
            if case.find("skipped") is not None:
                continue
            failed = [part for part in (case.find("failure"), case.find("error")) if part is not None]
            if not failed:
                passed += 1
                continue
            output = "\n".join(filter(None, [
                *(f"{part.get('type', '')}: {part.get('message', '')}\n{part.text or ''}" for part in failed),
                *((part.text or "") for part in (case.find("system-out"), case.find("system-err")) if part is not None)]))
            binary = suite.get("name") or case.get("classname") or ""
            failures.append((f"{binary} {case.get('name')}", output))
    lines = [f"{passed} passed, {len(failures)} failed"]
    outside = [test for test, _ in failures if test.split(" ", 1)[0].split("::", 1)[0] in beyond]
    if outside:
        lines.append(f"{len(outside)} of the failures are in crates beyond a pull request's scope (the changed crates and their"
                     " direct dependents): only this run tests them")
    for test, output in failures:
        shown = output.splitlines()
        lines += [f"--- FAIL {test}" + (" (beyond a pull request's scope)" if test in outside else ""), *shown[:SHOWN_LINES]]
        lines += [f"    ({len(shown) - SHOWN_LINES} more lines)"] if len(shown) > SHOWN_LINES else []
    return lines


COMPILING = re.compile(r"^\s+(?:Compiling|Checking) (\S+) v", re.M)


def compiled(log):
    """How many units a Cargo command built, naming the first few, so a leg shows how much its
    restored cache saved."""
    try:
        names = COMPILING.findall(Path(log).read_text(encoding="utf-8", errors="replace"))
    except OSError:
        return ""
    shown = ", ".join(dict.fromkeys(names[:8]))
    return f"compiled {len(names)} units" + (f" ({shown}{', ...' if len(names) > 8 else ''})" if names else "")



def not_run_here(log_text):
    """The parts a job printed it could not run on this machine, as `not run here: <part> (<why>)`."""
    return [line.strip() for line in log_text.splitlines() if line.startswith("not run here: ")]


class Runner:
    """Runs decided jobs, each logging to its own file in the run's folder."""

    def __init__(self, folder, values, crates, local=False):
        self.folder, self.values, self.local = folder, values, local
        self.crates = crates

    def execute(self, command, log):
        env = dict(os.environ)
        env["PYTHONUTF8"] = "1"
        with open(log, "ab") as out:
            out.write(f"$ {' '.join(command)}\n".encode())
            out.flush()
            try:
                return subprocess.run(command, cwd=ROOT, stdout=out, stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL, env=env).returncode
            except OSError as error:
                out.write(f"could not start: {error}\n".encode())
                return 127

    def run(self, decision):
        job, log = decision.job, self.folder / f"{decision.job.name}.log"
        started = time.monotonic()
        values = dict(self.values, files=decision.arguments, packages=decision.arguments)
        command = job.command
        report = ROOT / job.report if job.report and not self.local else None
        if job.kind == "files" and decision.whole and job.whole:
            code = self.execute(expand(job.whole, values), log)
        elif job.kind == "files":
            codes = [self.execute(part, log) for part in batches(command, values, decision.arguments)]
            code = next((code for code in codes if code), 0)
        else:
            if report:
                report.unlink(missing_ok=True)
            if (not self.local and sys.platform == "win32" and job.name == "rust-test"
                    and "shader-pipeline" in test_reach(self.crates, decision.crates)):
                stage_code = stage_shader_compiler(command, log)
                if stage_code != 0:
                    return Outcome(job.name, "fail", f"staging pinned shader compiler failed, exit {stage_code}", time.monotonic() - started, str(log))
            code = self.execute(expand(command, values), log)
            if report and report.is_file() and code in (0, NEXTEST_TESTS_FAILED):
                lines = failure_report(report.read_text(encoding="utf-8", errors="replace"), set(decision.beyond))
                if lines is not None:
                    log.with_suffix(".failures.txt").write_text("\n".join(lines) + "\n", encoding="utf-8")
        seconds = time.monotonic() - started
        notes = not_run_here(log.read_text(encoding="utf-8", errors="replace"))
        detail = ", ".join(filter(None, [decision.detail, compiled(log) if job.kind == "crates" else "", *notes]))
        if code == 0:
            return Outcome(job.name, "pass", detail, seconds, str(log))
        if code == 127:
            return Outcome(job.name, "not run", "the command could not start", seconds, str(log))
        return Outcome(job.name, "fail", ", ".join(filter(None, [detail, f"exit {code}"])), seconds, str(log))


def run_all(runner, decisions):
    """Runs decisions in parallel, crates jobs one after another (they share a target folder)."""
    serial = [d for d in decisions if d.job.kind == "crates"]
    parallel = [d for d in decisions if d.job.kind != "crates"]
    with ThreadPoolExecutor(max_workers=len(parallel) + 1) as pool:
        futures = [pool.submit(runner.run, decision) for decision in parallel]
        futures.append(pool.submit(lambda: [runner.run(decision) for decision in serial]))
        results = []
        for future in futures:
            found = future.result()
            results += found if isinstance(found, list) else [found]
    return results


def report(outcomes, title):
    """Prints each failed job's log tail and test failures, then one line per job; the verdict."""
    ordered = sorted(outcomes, key=lambda outcome: (ORDER.index(outcome.state), outcome.name))
    for outcome in ordered:
        if outcome.state in ("fail", "not run") and outcome.log:
            lines = Path(outcome.log).read_text(encoding="utf-8", errors="replace").splitlines()
            print(f"\n== {outcome.name} ({outcome.log})\n" + "\n".join(lines[-200:]), flush=True)
    for outcome in ordered:
        failures = Path(outcome.log).with_suffix(".failures.txt") if outcome.log else None
        if failures and failures.is_file():
            print(f"\n== {outcome.name}: tests\n" + failures.read_text(encoding="utf-8", errors="replace"), flush=True)
    print("")
    for outcome in ordered:
        seconds = f"{outcome.seconds:6.1f}s" if outcome.seconds else "       "
        print(f"  {outcome.state.upper() if outcome.state == 'fail' else outcome.state:8} {outcome.name:22} {seconds}  {outcome.detail}", flush=True)
    state = verdict(ordered)
    print(f"{title}: {state}")
    return EXIT[state]


def pipeline_at(base):
    """The base's pipeline, or None when it has none that loads."""
    shown = subprocess.run(["git", "show", f"{base}:{PIPELINE_PATH}"], cwd=ROOT, capture_output=True)
    if shown.returncode:
        return None
    try:
        return base_pipeline(shown.stdout.decode("utf-8"))
    except UnicodeDecodeError:
        return None


def base_pipeline(text):
    """A base's pipeline from its text, or None when it does not load as one, whatever is wrong
    with it, so a change repairing a broken pipeline plans against it as unreadable."""
    try:
        return load_pipeline(text)
    except (SystemExit, KeyError, ValueError, TypeError, AttributeError, IndexError):
        return None


def definitions_at(base):
    """Each job's definition in the base's pipeline, or None when it has none that loads."""
    pipeline = pipeline_at(base)
    return None if pipeline is None else {job.name: job.definition for job in pipeline.jobs}


def ratchet_base(base):
    """The commit a ratchet compares with: its merge base with HEAD when the base is a commit, so
    a rollback or force-push ratchets only what HEAD adds to history they share (restored code
    already passed there); for a tree (a new main against the empty tree), HEAD's first parent;
    None when there is none."""
    if git("cat-file", "-t", base, check=False).strip() == "commit":
        shared = git("merge-base", "HEAD", base, check=False).strip()
        if shared:
            return shared
    return git("rev-parse", "--verify", "--quiet", "HEAD^1", check=False).strip() or None


def plan(tier, base_ref, exact=False, everything=False):
    """The decisions for a tier against the merge base with `base_ref`, or against `base_ref`
    itself when `exact` (main compares with its last green commit, which a force-push may have
    dropped), from the files alone; a tier after the first takes every crate above a changed one,
    and `everything` takes every job across everything it covers."""
    pipeline = load_pipeline(PIPELINE.read_text(encoding="utf-8"))
    if tier not in pipeline.tiers:
        sys.exit(f"tools/ci.toml has no tier {tier!r}")
    if exact:
        base = git("rev-parse", "--verify", "--quiet", f"{base_ref}^{{tree}}", check=False).strip() and base_ref
    else:
        base = git("merge-base", "HEAD", base_ref, check=False).strip()
    if not base:
        sys.exit(f"No base at {base_ref}: fetch it, or pass --base.")
    values = {"python": sys.executable, "node": shutil.which("node") or "node",
              "npm": shutil.which("npm.cmd") or shutil.which("npm") or "npm", "base": ratchet_base(base) or base}
    reaching = [job for job in pipeline.jobs if job.manifest or job.package]
    crates = crates_from_manifests(ROOT) if reaching or any(job.kind == "crates" for job in pipeline.jobs) else {}
    reach = {job.name: job_reach(ROOT, job, crates) for job in reaching if crates}
    at_base = pipeline_at(base)
    ctx = Context(read_tree(), changed_since(base), crates, pipeline,
                  None if at_base is None else {job.name: job.definition for job in at_base.jobs},
                  None if at_base is None else at_base.frame, everything=everything,
                  transitive=tier != pipeline.tiers[0], reach=reach)
    decisions = [decide(job, ctx, tier) for job in pipeline.jobs]
    if not ratchet_base(base):
        decisions = [Decision(d.job, "not run", f"no commit to ratchet against: {base} is not one and HEAD has no parent")
                     if d.job.base and d.state == "run" else d for d in decisions]
    return ctx, values, decisions, base


def unplanned_outcomes(planned, decisions):
    """An outcome failing each job `changes` planned that this run would not run, so a job the
    gate selected can never pass by being skipped on the runner."""
    named = {decision.job.name: decision for decision in decisions}
    failures = []
    for name in planned:
        decision = named.get(name)
        if decision is None:
            failures.append(Outcome(name, "fail", "planned by changes, but tools/ci.toml has no such job"))
        elif decision.state != "run":
            failures.append(Outcome(name, "fail", f"planned by changes, but this runner would not run it ({decision.state}: {decision.detail})"))
    return failures


def committed_weights():
    """The committed table of measured crate costs, as HEAD has it on every runner."""
    return git("show", "HEAD:tools/ci-weights.json", check=False)


def split_digest(split):
    return hashlib.sha256(json.dumps(split).encode()).hexdigest()[:12]


def shards(decision, crates, table):
    """The crate lists a sharded job's runners take, None for an empty one, or [None] for a job
    that is not sharded; `table` is the text of the measured crate costs."""
    if not decision.job.shards or not decision.crates:
        return [None]
    weights = load_weights(crates, table)
    return [shard or None for shard in split_crates(crates, decision.crates, weights, decision.dirty, decision.job.shards)]


def leg_cache(job):
    """The name of the target folder cache the job's leg restores: a sharded job's own, or its
    group's, which the group's unsharded jobs share as they share its leg."""
    return job.name if job.shards else GROUP_CACHE + job.group


def caches(pipeline):
    """The caches the cache workflow fills: each one a job with a `warm` command restores."""
    return sorted({leg_cache(job) for job in pipeline.jobs if job.warm})


def matrix_legs(pipeline, decisions, crates, table):
    """The workflow's matrix: one leg per group for its unsharded jobs, and one per runner of each
    sharded job, numbered within the job's shards."""
    needs = lambda group: {f"needs_{need}": True for need in pipeline.groups[group].get("setup", [])}
    groups, legs = {}, []
    for decision in decisions:
        if decision.state != "run":
            continue
        split = shards(decision, crates, table)
        if split == [None]:
            groups.setdefault(decision.job.group, []).append(decision.job.name)
            continue
        for at, shard in enumerate(split, 1):
            if shard:
                legs.append(dict(group=decision.job.group, name=f"{decision.job.name} {at}/{len(split)}", jobs=decision.job.name,
                                 shard=f"{at}/{len(split)}/{split_digest(split)}", cache=leg_cache(decision.job), **needs(decision.job.group)))
    return [dict(group=group, name=group, jobs=" ".join(names), shard="", cache=GROUP_CACHE + group, **needs(group))
            for group, names in groups.items()] + legs


def changes(args):
    """The workflow's `changes` job: each job's gate, and the matrix of legs."""
    ctx, _, decisions, base = plan(args.tier, args.base, exact=args.exact_base, everything=args.everything)
    for decision in decisions:
        print(f"  {decision.state:8} {decision.job.name:22} {decision.job.group:8} {decision.detail}")
    sharded = any(d.state == "run" and d.job.shards for d in decisions)
    load_weights({}, committed_weights())
    matrix = matrix_legs(ctx.pipeline, decisions, ctx.crates, committed_weights() if sharded else "")
    for leg in matrix:
        print(f"leg {leg['name']}: {leg['jobs']}")
    print(f"legs: {len(matrix) or 'none'}")
    unplanned = [decision.job.name for decision in decisions if decision.state == "not run"]
    if unplanned:
        print(f"could not plan: {', '.join(unplanned)}")
        return EXIT["error"]
    if args.github_output:
        with open(args.github_output, "a", encoding="utf-8") as out:
            out.write(f"matrix={json.dumps(matrix, separators=(',', ':'))}\n")
            out.write(f"any={'true' if matrix else 'false'}\n")
            out.write(f"base={base}\n")
    return 0


def take_shard(decisions, shard, crates, table):
    """The decisions with the one planned sharded job cut to runner `at` of `count`'s crates, and
    the outcomes failing the leg when this runner would split the job otherwise than `changes`,
    whose split the shard names by digest."""
    try:
        at, count, planned = shard.split("/")
        at, count = int(at), int(count)
    except ValueError:
        return decisions, [Outcome(shard, "fail", "a shard is AT/COUNT/DIGEST")]
    sharded = [d for d in decisions if d.state == "run" and d.job.shards]
    if len(sharded) != 1:
        return decisions, [Outcome(shard, "fail", f"shard {shard} names {len(sharded)} sharded jobs to run, not one")]
    decision = sharded[0]
    split = shards(decision, crates, table)
    if len(split) != count or not 1 <= at <= count or not split[at - 1] or split_digest(split) != planned:
        return decisions, [Outcome(decision.job.name, "fail", f"planned as shard {shard}, but this runner splits the job "
                                   f"otherwise ({len(split)}/{split_digest(split)})")]
    mine = split[at - 1]
    cut = Decision(decision.job, "run", f"shard {shard}: {len(mine)} of {decision.detail}",
                   [arg for name in mine for arg in ("-p", name)], crates=mine, beyond=decision.beyond, dirty=decision.dirty)
    return [cut if d is decision else d for d in decisions], []


def job_group(args):
    """The workflow's matrix leg: runs the jobs of one group that `changes` gated in, or one
    runner's share of a sharded job."""
    ctx, values, decisions, _ = plan(args.tier, args.base, exact=args.exact_base, everything=args.everything)
    if args.group not in ctx.pipeline.groups:
        sys.exit(f"tools/ci.toml has no group {args.group!r}")
    planned = args.jobs.split()
    wrong = [d.job.name for d in decisions if d.job.name in planned and d.job.group != args.group]
    if wrong:
        sys.exit(f"planned jobs outside group {args.group}: {', '.join(wrong)}")
    decisions = [d for d in decisions if d.job.name in planned]
    outcomes = unplanned_outcomes(planned, decisions)
    if args.shard:
        decisions, refused = take_shard(decisions, args.shard, ctx.crates, committed_weights())
        outcomes += refused
    runner = Runner(Path(tempfile.mkdtemp(prefix="ci-")), values, ctx.crates)
    outcomes += run_all(runner, [d for d in decisions if d.state == "run"])
    return report(outcomes, f"group {args.group}" + (f" shard {args.shard}" if args.shard else ""))


def check(args):
    """The quick run here before a push: the check tier's jobs the change touches, except those
    that compile or test, which CI runs."""
    ctx, values, decisions, base = plan("check", args.base)
    print(f"ci check: HEAD {git('rev-parse', 'HEAD').strip()[:10]} against {base[:10]}, {len(changed_since(base))} changed files", flush=True)
    outcomes = []
    running = []
    for decision in decisions:
        if decision.state == "run" and (decision.job.ci_only or decision.job.kind == "crates"):
            outcomes.append(Outcome(decision.job.name, "ci only", decision.detail))
        elif decision.state == "run":
            running.append(decision)
        else:
            outcomes.append(Outcome(decision.job.name, decision.state, decision.detail))
    folder = Path(tempfile.mkdtemp(prefix="ci-check-"))
    print(f"logs: {folder}", flush=True)
    outcomes += run_all(Runner(folder, values, ctx.crates, local=True), running)
    return report(outcomes, "ci check (GitHub CI is the gate)")


def newer_paths(changes, tracked):
    """The paths a restored target folder must see as newer than itself, from `changes` (status,
    path) since the commit it was built from: each changed path and every folder above it, so a
    build script watching a folder sees a file added to or removed from it; and, for a file added
    or removed, every file in its top two folders (its crate's), since a new file can change how
    the crate compiles (a second candidate for a module) without being in any unit's dep-info."""
    newer, structural = set(), set()
    for status, path in changes:
        parts = path.split("/")
        newer.update("/".join(parts[:end]) for end in range(1, len(parts) + 1))
        if status in ("A", "D") and len(parts) > 2:
            structural.add("/".join(parts[:2]) + "/")
    newer.update(path for path in tracked if path.startswith(tuple(structural)))
    return newer


def built_from(target):
    """The commit a target folder names as the one it was built from, when this clone has it."""
    try:
        named = (Path(target) / BUILT_FROM).read_text(encoding="utf-8").strip()
    except OSError:
        return None
    return named if named and git("cat-file", "-t", named, check=False).strip() == "commit" else None


def set_times(target):
    """Sets every tracked file's and folder's time so Cargo's freshness check, which compares
    times, sees in a target folder restored from a cache exactly the files that differ from the
    commit it was built from: those are now, the rest a fixed old time older than any build.
    Without that commit every file is now, so every workspace crate rebuilds; third-party crates
    are keyed by version, not time, and stay."""
    now = time.time()
    source = built_from(target)
    tracked = z(git("ls-files", "-z"))
    newer = None
    if source:
        fields = z(git("diff", "--name-status", "--no-renames", "-z", source, "HEAD"))
        newer = newer_paths(zip(fields[0::2], fields[1::2]), tracked)
    folders = {"/".join(path.split("/")[:end]) for path in tracked for end in range(1, path.count("/") + 1)}
    for path in [*tracked, *sorted(folders, key=lambda folder: -folder.count("/"))]:
        stamp = now if newer is None or path in newer else SETTLED
        try:
            os.utime(ROOT / path, (stamp, stamp))
        except OSError:
            pass
    print(f"file times: {len(newer) if source else 'every'} paths newer than the target folder"
          + (f" (built from {source[:10]})" if source else " (it names no commit this clone has)"), flush=True)
    return 0


UNIT = re.compile(r"-([0-9a-f]{16})(?=[./\\]|$)")


def live_units(messages):
    """The unit hashes a Cargo build used, built or fresh, from its JSON messages: every artifact's
    file names and every build script's output folder."""
    live = set()
    for line in messages.splitlines():
        try:
            message = json.loads(line)
        except ValueError:
            continue
        paths = message.get("filenames") or []
        paths += [message["out_dir"]] if message.get("reason") == "build-script-executed" else []
        for path in paths:
            live.update(UNIT.findall(path.replace("\\", "/")))
    return live


def prune(profile, live):
    """Removes from a profile folder (such as `.cargo-target/debug`) every unit the last build did not
    use, so a unit that comes back later (a feature or profile setting changed and changed back)
    is built again rather than found fresh against files it never saw; then the executables,
    which no runner reuses and which would fill the cache."""
    removed = 0
    for folder in (".fingerprint", "build", "deps"):
        for entry in (Path(profile) / folder).glob("*"):
            found = UNIT.search(entry.name)
            if found and found.group(1) not in live:
                shutil.rmtree(entry) if entry.is_dir() else entry.unlink()
                removed += 1
    for pattern in ("*.exe", "*.pdb", "deps/*.exe", "deps/*.pdb"):
        for entry in Path(profile).glob(pattern):
            entry.unlink()
    return removed


def rust_unchanged(source, pipeline, jobs):
    """Whether nothing a Rust build reads changed between `source` and HEAD: no crate, vendored
    crate, Rust toolchain file or input of the jobs."""
    rust = matcher(("crates/**", *pipeline.toolchains["rust"], *(path for job in jobs for path in job.inputs)))
    return not any(rust(path) for path in z(git("diff", "--name-only", "--no-renames", "-z", source, "HEAD")))


def warm(name, target, github_output=None):
    """Fills a target folder for the cache `name`: runs the `warm` command of each job that
    restores it, one after another, which together build every unit its legs may reuse, and, when
    all succeed, prunes the units none of them used and names HEAD as the commit the folder was
    built from. A failed build saves nothing: its folder mixes units rebuilt at HEAD with the
    commit it named before. A folder that neither a Rust input nor a definition of those jobs
    changed since is not built or saved again."""
    pipeline = load_pipeline(PIPELINE.read_text(encoding="utf-8"))
    jobs = [job for job in pipeline.jobs if job.warm and leg_cache(job) == name]
    if not jobs:
        sys.exit(f"tools/ci.toml has no job with a warm command that restores the cache {name!r}")
    source = built_from(target)
    built = definitions_at(source) or {} if source else {}
    save = not (source and rust_unchanged(source, pipeline, jobs) and all(built.get(job.name) == job.definition for job in jobs))
    if save:
        started = time.monotonic()
        live, code = set(), 0
        for job in jobs:
            with tempfile.TemporaryFile() as messages:
                code = subprocess.run([*job.warm, "--message-format=json-render-diagnostics"], cwd=ROOT, stdout=messages,
                                      stdin=subprocess.DEVNULL).returncode
                messages.seek(0)
                live |= live_units(messages.read().decode("utf-8", errors="replace"))
            if code:
                break
        print(f"warmed {name} in {time.monotonic() - started:.0f}s, exit {code}, {len(live)} units", flush=True)
        if code == 0:
            profiles = [folder for folder in Path(target).iterdir() if (folder / ".fingerprint").is_dir()]
            print(f"pruned {sum(prune(folder, live) for folder in profiles)} units the build did not use", flush=True)
            (Path(target) / BUILT_FROM).write_text(git("rev-parse", "HEAD").strip() + "\n", encoding="utf-8")
        else:
            save = False
            print("the build failed: nothing is saved", flush=True)
    else:
        print(f"no Rust input or job definition changed since {source[:10]}: nothing to build or save", flush=True)
    if github_output:
        with open(github_output, "a", encoding="utf-8") as out:
            out.write(f"save={'true' if save else 'false'}\n")
    return 0


LIBRARY_UNITS = ("", " build-script", " build-script (run)")


def measured_costs(timings, report, crates):
    """name -> [build, test] core-seconds of each workspace crate in a cold `cargo test --no-run
    --timings` report (its library and build script units are the build; its test, bin and
    example units the test) and a nextest JUnit report of running those tests (each test's time
    adds to its crate's test)."""
    found = re.search(r"\bconst UNIT_DATA = ", timings)
    if found is None:
        raise SystemExit("the timings report has no UNIT_DATA")
    units, _ = json.JSONDecoder().raw_decode(timings, found.end())
    costs = {}
    for unit in units:
        if unit["name"] in crates:
            costs.setdefault(unit["name"], [0.0, 0.0])[0 if unit["target"] in LIBRARY_UNITS else 1] += float(unit["duration"])
    for suite in ElementTree.fromstring(report).iter("testsuite"):
        name = (suite.get("name") or "").split("::", 1)[0]
        if name in costs:
            costs[name][1] += sum(float(case.get("time") or 0) for case in suite.iter("testcase"))
    return costs


def write_weights(timings, report, out):
    """Writes the measured crate costs of a cold run into the committed table, keeping the
    entries of crates the run did not build and dropping crates the workspace no longer has."""
    crates = crates_from_manifests(ROOT)
    measured = measured_costs(Path(timings).read_text(encoding="utf-8"), Path(report).read_text(encoding="utf-8"), crates)
    try:
        table = json.loads(Path(out).read_text(encoding="utf-8")).get("crates", {})
    except (OSError, ValueError):
        table = {}
    table = {name: cost for name, cost in table.items() if name in crates}
    table.update({name: {"build": round(build, 1), "test": round(test, 1)} for name, (build, test) in measured.items()})
    rows = ",\n".join(f"    {json.dumps(name)}: {json.dumps(table[name], sort_keys=True)}" for name in sorted(table))
    Path(out).write_text('{\n  "crates": {\n' + rows + "\n  }\n}\n", encoding="utf-8")
    print(f"{len(measured)} crates measured, {len(table)} in {out}; {len(crates) - len(table)} take the median")
    return 0


def main(argv=None):
    for stream in (sys.stdout, sys.stderr):
        if hasattr(stream, "reconfigure"):
            stream.reconfigure(errors="replace")
    parser = argparse.ArgumentParser(description="Plan and run the jobs in tools/ci.toml.")
    commands = parser.add_subparsers(dest="command", required=True)
    checking = commands.add_parser("check", help="a quick run here before a push")
    checking.add_argument("--base", default="origin/main", metavar="REF", help="compare with the merge base with REF (default origin/main)")
    gating = commands.add_parser("changes", help="the workflow's gates and matrix")
    gating.add_argument("--tier", default="check")
    gating.add_argument("--base", required=True, metavar="REF")
    gating.add_argument("--github-output", metavar="FILE")
    gating.add_argument("--exact-base", action="store_true", help="compare with the base itself, not the merge base")
    gating.add_argument("--everything", action="store_true", help="run every job across everything it covers, whatever changed")
    grouped = commands.add_parser("job", help="run one leg's jobs, as the workflow does")
    grouped.add_argument("--tier", default="check")
    grouped.add_argument("--group", required=True)
    grouped.add_argument("--jobs", required=True, help="the job names `changes` planned for this leg, space-separated")
    grouped.add_argument("--base", required=True, metavar="REF")
    grouped.add_argument("--exact-base", action="store_true", help="compare with the base itself, not the merge base")
    grouped.add_argument("--everything", action="store_true", help="run every job across everything it covers, whatever changed")
    grouped.add_argument("--shard", default="", metavar="AT/COUNT", help="run this runner's share of the one sharded job planned")
    timing = commands.add_parser("times", help="set file times for a target folder restored from a cache")
    timing.add_argument("--target", required=True)
    warming = commands.add_parser("warm", help="build what a target folder cache keeps, and name HEAD in it")
    warming.add_argument("--cache", required=True, help="a sharded job's name, or group-NAME for a group's unsharded jobs")
    warming.add_argument("--target", required=True)
    warming.add_argument("--github-output", metavar="FILE")
    weighing = commands.add_parser("weights", help="write the measured crate costs the legs are balanced by")
    weighing.add_argument("--timings", required=True, help="cargo-timing.html of a cold `cargo test --no-run --timings`")
    weighing.add_argument("--junit", required=True, help="nextest's JUnit report of running those tests")
    weighing.add_argument("--out", default=str(WEIGHTS))
    args = parser.parse_args(argv)
    if args.command == "check":
        return check(args)
    if args.command == "changes":
        return changes(args)
    if args.command == "job":
        return job_group(args)
    if args.command == "times":
        return set_times(args.target)
    if args.command == "weights":
        return write_weights(args.timings, args.junit, args.out)
    return warm(args.cache, args.target, args.github_output)


if __name__ == "__main__":
    sys.exit(main())
