"""Run a tier of the pipeline in tools/ci.toml and publish its verdict (Python 3.11+).

usage: python tools/ci.py check [--base REF] [--dry-run] [--no-status]
       python tools/ci.py main [--base REF] [--dry-run] [--no-status]
       python tools/ci.py changes --tier TIER --base REF [--github-output FILE]
       python tools/ci.py job --tier TIER --group GROUP --base REF

`changes` and `job` are the GitHub workflow's (.github/workflows/ci.yml) steps. `changes` gives
each job its gate from the files alone, with no toolchain, and writes the matrix of the groups
that have work; `job` runs one group's jobs on a hosted runner, compiling Rust there with Cargo
(each job's `hosted` command) rather than on Boat. On a hosted runner (GITHUB_ACTIONS) nothing is
held, paused or deferred: the workflow's aggregate `check` job is the gate.

`check` runs the check tier against the merge base with REF (default the upstream, `--upstream`,
origin/main unless given). `main` runs the check and main tiers on a commit already on the
upstream against REF (default HEAD~1), for a
poller that tests each new main commit. The working tree counts, uncommitted and untracked
files included. First the `changes` step gives each job its gate; then the jobs it needs run in
parallel, Boat jobs first. Each job declares its inputs, toolchain, command and tier in
tools/ci.toml:
- a job none of whose inputs changed since the base is skipped, never reported as passed;
- a job whose input hash already has a recorded pass is reused, reported with the commit that
  passed it; `files` jobs are reused per file (running `whole` when every file needs it), `crates` jobs per crate (its files and those of
  every workspace crate it depends on), so a Boat job with nothing new costs nothing;
- `crates` jobs compile on Boat through tools/boat.py, for the changed crates and their direct
  dependents. A workspace-wide change, or more crates than the job's `limit`, is left to a later
  tier unless the job takes `--workspace`;
- a `heavy` job runs heavy work here (Boat's test path runs the test programs on this machine):
  it is skipped while the game lock exists, and not run at all while `[local] heavy = "paused"`,
  which the status names; neither counts as a pass or a failure;
- jobs of a later tier are listed as left to that tier.
The verdict is one aggregate, as in T3 Code's `check` job: success only when every job of the
tier passed, was reused, or was skipped because its gate was not needed. A job held for the game,
paused, or deferred to a later tier makes it pending, with the reason; a failure makes it a
failure, and a job that could not run an error. A new run of the same tier on the same branch
supersedes an older one still running, which stops the commands it started and posts nothing.
A job whose definition differs from the base's runs. Boat jobs start first and every job runs in
parallel, below normal priority. Logs, pass records and per-job timings go to the store:
HOI4_CI_STORE, or `captures/ci-results` beside the main checkout. Passes are recorded only if
HEAD and the files are unchanged when the run ends. On a clean tree, the verdict is posted as the
GitHub commit status `ci/<tier>` on HEAD: for `check` when it compared with the merge base with
the upstream, for `main` when HEAD is on the upstream. A commit GitHub doesn't have yet is
reported, not posted.
Exit 0 on success, 1 on a failure, 2 when a job that should have run could not, the files changed
during the run, or GitHub refused the status, 3 when pending, 4 when superseded.
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
import threading
import time
import tomllib

ROOT = Path(__file__).resolve().parents[1]
HOSTED = os.environ.get("GITHUB_ACTIONS") == "true"
PIPELINE = ROOT / "tools" / "ci.toml"
TEMP = Path(os.environ.get("TEMP") or tempfile.gettempdir())
GAME_LOCKS = tuple(TEMP / folder / "hoi4_game.lock" for folder in ("claude", "HOI4Replay"))
GAME_LOCK_STALE = 90 * 60
BOAT_FAILED = 75
COMMAND_LIMIT = 24_000
STATUS_LIMIT = 140
RETAIN_DAYS = 30
KINDS = {"command", "files", "crates"}
ORDER = ("fail", "not run", "cancelled", "held", "paused", "deferred", "pass", "reused", "skipped", "later")
PENDING = ("held", "paused", "deferred")
EXIT = {"success": 0, "failure": 1, "error": 2, "pending": 3}
SUPERSEDED = 4
UNCHANGED = "no input changed since the base"


@dataclass(frozen=True)
class Job:
    name: str
    tier: str
    kind: str
    toolchain: str
    inputs: tuple
    shared: tuple
    command: tuple
    hosted: tuple
    group: str
    whole: tuple
    base: bool
    workspace: bool
    limit: int | None
    heavy: bool
    unset: tuple
    environment: tuple
    widen: tuple
    definition: str


@dataclass(frozen=True)
class Toolchain:
    version: tuple
    files: tuple
    requires: tuple
    tools: tuple
    installed: tuple
    consistent: tuple
    probes: tuple


@dataclass
class Pipeline:
    tiers: dict
    toolchains: dict
    jobs: list
    heavy_local: bool = True
    groups: dict = field(default_factory=dict)


def load_pipeline(text):
    data = tomllib.loads(text)
    tiers = {name: table["budget"] for name, table in data["tier"].items()}
    toolchains = {name: Toolchain(tuple(table.get("version", ())), tuple(table.get("files", ())), tuple(table.get("requires", ())),
                                  tuple(table.get("tools", ())), tuple(table.get("installed", ())), tuple(table.get("consistent", ())),
                                  tuple(table.get("probes", ())))
                  for name, table in data["toolchain"].items()}
    jobs = []
    for table in data["job"]:
        job = Job(
            name=table["name"], tier=table["tier"], kind=table.get("kind", "command"),
            toolchain=table["toolchain"], inputs=tuple(table.get("inputs", ())),
            shared=tuple(table.get("shared", ())), command=tuple(table.get("command", ())), whole=tuple(table.get("whole", ())),
            hosted=tuple(table.get("hosted", ())), group=table.get("group", ""),
            base=table.get("base", False), workspace=table.get("workspace", False), limit=table.get("limit"),
            heavy=table.get("heavy", False), unset=tuple(table.get("unset", ())), environment=tuple(table.get("environment", ())),
            widen=tuple(table.get("widen", ())),
            definition=json.dumps([table, data["toolchain"].get(table["toolchain"]), data.get("group", {}).get(table.get("group"))], sort_keys=True))
        problems = [f"tier {job.tier!r}"] if job.tier not in tiers else []
        problems += [f"kind {job.kind!r}"] if job.kind not in KINDS else []
        problems += [f"toolchain {job.toolchain!r}"] if job.toolchain not in toolchains else []
        problems += [] if job.command else ["command"]
        problems += [f"group {job.group!r}"] if job.group not in data.get("group", {}) else []
        problems += ["name, used twice"] if job.name in {known.name for known in jobs} else []
        if problems:
            raise SystemExit(f"tools/ci.toml: job {job.name}: unknown or missing {', '.join(problems)}")
        jobs.append(job)
    heavy = data.get("local", {}).get("heavy", "run")
    if heavy not in {"run", "paused"}:
        raise SystemExit(f"tools/ci.toml: [local] heavy is run or paused, not {heavy!r}")
    return Pipeline(tiers, toolchains, jobs, heavy == "run", dict(data.get("group", {})))


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


def digest(*parts):
    return hashlib.sha256(json.dumps(parts, sort_keys=True, separators=(",", ":")).encode()).hexdigest()


@dataclass
class Crate:
    name: str
    folder: str
    deps: frozenset


def crates_from_manifests(root):
    """Each workspace member's folder, relative to `root`, and the members it depends on through a
    path dependency of any kind or target, read from the manifests themselves so that planning
    needs no Rust toolchain; None when the root manifest cannot be read."""
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
        sections = [data.get(kind, {}) for kind in ("dependencies", "dev-dependencies", "build-dependencies")]
        for target in data.get("target", {}).values():
            sections += [target.get(kind, {}) for kind in ("dependencies", "dev-dependencies", "build-dependencies")]
        deps = set()
        for section in sections:
            for name, spec in section.items():
                if not isinstance(spec, dict):
                    continue
                found = shared.get(name) if spec.get("workspace") else path / spec["path"] if "path" in spec else None
                if found is not None:
                    deps.add(names.get(key(found)))
        name = names[folder]
        crates[name] = Crate(name, path.relative_to(root).as_posix(), frozenset(deps - {None, name}))
    return crates


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


@dataclass
class Context:
    """What a run decides from: the files as they are (path -> (blob, tracked)), the paths changed
    since the base, the revisions, the toolchain digests and those missing, the workspace crates
    (None when Cargo could not list them), the pass records, whether the game is running and each
    job's definition at the base (None when the base has no readable pipeline), so a job whose
    definition changed runs."""
    tree: dict
    changed: set
    base: str
    head: str
    toolchains: dict
    crates: dict | None
    passed: object
    pipeline: Pipeline
    missing: dict = field(default_factory=dict)
    game: bool = False
    definitions: dict | None = None
    own: dict | None = None
    hosted: bool = False


@dataclass
class Decision:
    job: Job
    state: str
    detail: str = ""
    keys: dict = field(default_factory=dict)
    arguments: list = field(default_factory=list)
    whole: bool = False


def file_entries(tree, paths):
    return [[path, *tree[path]] for path in sorted(paths)]


def toolchain_changed(ctx, job):
    files = matcher(ctx.pipeline.toolchains[job.toolchain].files)
    return any(files(path) for path in ctx.changed)


def reused_detail(records):
    shas = sorted({record["sha"] for record in records})
    return "from " + ", ".join(shas[:3]) + (f" and {len(shas) - 3} more" if len(shas) > 3 else "")


def decide(job, ctx, tier):
    """Whether a job runs, and on what; pure, so the reuse and skip rules can be tested. A job of
    another tier that would run is left to that tier; one that is unchanged or reused says so."""
    decision = decide_inputs(job, ctx)
    if decision.state in ("run", "reused") and ctx.missing.get(job.toolchain):
        decision = Decision(job, "not run", ctx.missing[job.toolchain])
    order = list(ctx.pipeline.tiers)
    if decision.state in ("run", "not run", "deferred"):
        if order.index(job.tier) > order.index(tier):
            return Decision(job, "later", f"{job.tier} tier" + (f": {decision.detail}" if decision.detail else ""))
        if decision.state == "run" and job.heavy and not ctx.hosted and not ctx.pipeline.heavy_local:
            return Decision(job, "paused", "heavy local work paused ([local] heavy in tools/ci.toml)")
        if decision.state == "run" and job.heavy and not ctx.hosted and ctx.game:
            return Decision(job, "held", "the game lock exists; heavy local work waits for the game")
    return decision


def environment_entries(job):
    """The value of each environment variable a job reads, with the content of the file it names
    when it names one, so a pass under one fixture never stands for another."""
    entries = []
    for name in job.environment:
        value = os.environ.get(name)
        content = None
        if value:
            try:
                content = hashlib.sha256(Path(value).read_bytes()).hexdigest()
            except OSError:
                pass
        entries.append([name, value, content])
    return entries


def decide_inputs(job, ctx):
    toolchain = digest(ctx.toolchains[job.toolchain], environment_entries(job)) if job.environment else ctx.toolchains[job.toolchain]
    if job.kind == "crates":
        return decide_crates(job, ctx, toolchain)
    inputs, shared = matcher(job.inputs), matcher(job.shared)
    redefined = ctx.definitions is None or ctx.definitions.get(job.name) != job.definition
    wide = redefined or toolchain_changed(ctx, job) or any(shared(path) for path in ctx.changed)
    if job.kind == "files":
        shared_key = digest(job.definition, toolchain, file_entries(ctx.tree, [p for p in ctx.tree if shared(p)]))
        pool = ctx.tree if wide else [path for path in ctx.changed if path in ctx.tree]
        candidates = sorted(path for path in pool if inputs(path))
        if not candidates:
            return Decision(job, "skipped", UNCHANGED)
        keys = {path: digest(shared_key, path, *ctx.tree[path]) for path in candidates}
        records = {path: ctx.passed(key) for path, key in keys.items()}
        todo = [path for path in candidates if not records[path]]
        if not todo:
            return Decision(job, "reused", f"{len(candidates)} files {reused_detail(records.values())}")
        detail = f"{len(todo)} files" + (f", {len(candidates) - len(todo)} reused" if len(todo) < len(candidates) else "")
        return Decision(job, "run", detail, {path: keys[path] for path in todo}, todo, wide and len(todo) == len(candidates))
    if not (wide or any(inputs(path) for path in ctx.changed)):
        return Decision(job, "skipped", UNCHANGED)
    files = [path for path in ctx.tree if inputs(path) or shared(path)]
    key = digest(job.definition, toolchain, ctx.base if job.base else None, file_entries(ctx.tree, files))
    record = ctx.passed(key)
    if record:
        return Decision(job, "reused", reused_detail([record]))
    return Decision(job, "run", keys={job.name: key})


def decide_crates(job, ctx, toolchain):
    redefined = ctx.definitions is None or ctx.definitions.get(job.name) != job.definition
    if ctx.crates is None:
        rust = redefined or any(path.startswith("crates/") for path in ctx.changed) or toolchain_changed(ctx, job)
        return Decision(job, "not run", "the Cargo manifests could not be read") if rust else Decision(job, "skipped", "no crate changed since the base")
    crates = ctx.crates
    shared = matcher(job.shared)
    shared_files = file_entries(ctx.tree, [path for path in ctx.tree if shared(path)])
    owner = crate_owner(crates)
    widened = matcher(job.widen)
    wide = redefined or toolchain_changed(ctx, job) or any(widened(path) for path in ctx.changed)
    changed = {owner(path) for path in ctx.changed} - {None}
    selected = set(crates) if wide else changed | {crate.name for crate in crates.values() if crate.deps & changed}
    if not selected:
        return Decision(job, "skipped", "no crate changed since the base")
    if wide and not job.workspace and not ctx.hosted:
        return Decision(job, "deferred", "a workspace-wide change; a later tier runs the workspace")
    if job.limit is not None and len(selected) > job.limit and not job.workspace and not ctx.hosted:
        return Decision(job, "deferred", f"{len(selected)} crates, above the job's limit of {job.limit}")
    if ctx.own is None:
        ctx.own = {}
        for path in sorted(ctx.tree):
            name = owner(path)
            if name:
                ctx.own.setdefault(name, []).append([path, *ctx.tree[path]])
    own = ctx.own
    keys = {name: digest(job.definition, toolchain, ctx.base if job.base else None, shared_files, own.get(name, []), [[dep, own.get(dep, [])] for dep in sorted(closure(crates, name))])
            for name in selected}
    records = {name: ctx.passed(key) for name, key in keys.items()}
    todo = sorted(name for name in selected if not records[name])
    scope = f"{len(selected)} crates" + (" (workspace-wide)" if wide else "")
    if not todo:
        return Decision(job, "reused", f"{scope} {reused_detail(records.values())}")
    detail = scope + (f", {len(selected) - len(todo)} reused" if len(todo) < len(selected) else "")
    arguments = ["--workspace"] if wide else [arg for name in todo for arg in ("-p", name)]
    return Decision(job, "run", detail, {name: keys[name] for name in todo}, arguments)


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
    """The aggregate: success only when every job passed, was reused, was skipped because its
    gate was not needed, or belongs to a later tier."""
    states = {outcome.state for outcome in outcomes}
    if "fail" in states:
        return "failure"
    if "not run" in states:
        return "error"
    if states & set(PENDING):
        return "pending"
    return "success"


def describe(outcomes, seconds):
    """A commit status line: the jobs that failed, couldn't run, were held for the game, paused,
    deferred or left to a later tier, then the counts, cut to GitHub's limit."""
    groups = (("fail", "failed"), ("not run", "not run"), ("held", "held for the game"),
              ("paused", "not run (heavy local work paused)"), ("deferred", "deferred"), ("later", "later tier"))
    named = [f"{label}: {', '.join(o.name for o in outcomes if o.state == key)}"
             for key, label in groups if any(o.state == key for o in outcomes)]
    count = lambda key: sum(o.state == key for o in outcomes)
    head = f"{count('pass') + count('fail')} ran, {count('reused')} reused, {count('skipped')} unchanged; {seconds:.0f}s"
    text = "; ".join(named + [head])
    return text if len(text) <= STATUS_LIMIT else text[:STATUS_LIMIT - 1] + "…"


def git(*args, check=True, data=None):
    done = subprocess.run(["git", *args], cwd=ROOT, capture_output=True, input=data)
    if check and done.returncode:
        raise SystemExit(f"git {' '.join(args)} failed: {done.stderr.decode(errors='replace').strip()}")
    return done.stdout.decode("utf-8", errors="replace")


def z(text):
    return [name for name in text.split("\0") if name]


def read_tree():
    """Each file as it is now: path -> (blob id, tracked). Uncommitted content is hashed as Git
    would store it. A sparse checkout is refused: the files it leaves out would be hashed as if
    present."""
    if any(line.startswith("S ") for line in z(git("ls-files", "-t", "-z"))):
        raise SystemExit("ci check needs a full checkout; this worktree is sparse")
    tree = {}
    for line in z(git("ls-files", "-s", "-z")):
        meta, path = line.split("\t", 1)
        tree[path] = (meta.split()[1], True)
    for path in z(git("ls-files", "-d", "-z")):
        tree.pop(path, None)
    modified = [path for path in z(git("ls-files", "-m", "-z")) if path in tree]
    untracked = z(git("ls-files", "-o", "--exclude-standard", "-z"))
    rehash = [path for path in modified + untracked if (ROOT / path).is_file()]
    if rehash:
        blobs = git("hash-object", "--stdin-paths", data="\n".join(rehash).encode()).split()
        for path, blob in zip(rehash, blobs):
            tree[path] = (blob, path not in untracked)
    return tree


def changed_since(base):
    return set(z(git("diff", "--name-only", "--no-renames", "-z", base))) | set(z(git("ls-files", "-o", "--exclude-standard", "-z")))


def store_root():
    if os.environ.get("HOI4_CI_STORE"):
        return Path(os.environ["HOI4_CI_STORE"])
    common = Path(git("rev-parse", "--path-format=absolute", "--git-common-dir").strip())
    return common.parent / "captures" / "ci-results"


def write_json(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f"{path.name}.{os.getpid()}.{threading.get_ident()}.tmp")
    temporary.write_text(json.dumps(value, indent=1, sort_keys=True), encoding="utf-8")
    try:
        os.replace(temporary, path)
    except OSError:
        temporary.unlink(missing_ok=True)
        raise


class Store:
    """Pass records, one file per input hash, and one folder per run; shared by every worktree."""

    def __init__(self, root):
        self.root = root
        (root / "passes").mkdir(parents=True, exist_ok=True)
        (root / "runs").mkdir(parents=True, exist_ok=True)

    def _pass(self, key):
        return self.root / "passes" / key[:2] / f"{key}.json"

    def passed(self, key):
        """The record of a pass under `key` younger than RETAIN_DAYS, or None."""
        path = self._pass(key)
        try:
            if time.time() - path.stat().st_mtime > RETAIN_DAYS * 86400:
                return None
            return json.loads(path.read_text(encoding="utf-8"))
        except (FileNotFoundError, ValueError):
            return None

    def record(self, key, entry):
        """Records a pass. Another worktree recording or reading the same key at once makes
        Windows refuse the replacement; the record it holds is the same pass, so that is kept."""
        try:
            write_json(self._pass(key), entry)
        except PermissionError:
            if not self._pass(key).exists():
                raise

    def run_folder(self, head):
        folder = self.root / "runs" / f"{time.strftime('%Y%m%d-%H%M%S')}-{head[:10]}-{os.getpid()}"
        folder.mkdir(parents=True)
        return folder

    def prune(self):
        """Once a day, drops pass records and runs older than RETAIN_DAYS."""
        marker = self.root / "pruned"
        try:
            if time.time() - marker.stat().st_mtime < 86400:
                return
        except FileNotFoundError:
            pass
        marker.touch()
        cutoff = time.time() - RETAIN_DAYS * 86400
        for path in [*self.root.glob("passes/*/*.json"), *self.root.glob("runs/*/*")]:
            try:
                if path.stat().st_mtime < cutoff:
                    path.unlink()
            except OSError:
                pass
        for folder in [*self.root.glob("runs/*"), *self.root.glob("passes/*")]:
            try:
                if folder.stat().st_mtime < cutoff:
                    folder.rmdir()
            except OSError:
                pass


def game_running():
    """Whether a game lock younger than GAME_LOCK_STALE exists: the agents' or the smoke run's."""
    for lock in GAME_LOCKS:
        try:
            if time.time() - lock.stat().st_mtime < GAME_LOCK_STALE:
                return True
        except FileNotFoundError:
            pass
    return False


if os.name == "nt":
    import ctypes
    from ctypes import wintypes

    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    BELOW_NORMAL = 0x4000

    class _BasicLimits(ctypes.Structure):
        _fields_ = [("PerProcessUserTimeLimit", ctypes.c_int64), ("PerJobUserTimeLimit", ctypes.c_int64),
                    ("LimitFlags", wintypes.DWORD), ("MinimumWorkingSetSize", ctypes.c_size_t),
                    ("MaximumWorkingSetSize", ctypes.c_size_t), ("ActiveProcessLimit", wintypes.DWORD),
                    ("Affinity", ctypes.c_size_t), ("PriorityClass", wintypes.DWORD), ("SchedulingClass", wintypes.DWORD)]

    class _ExtendedLimits(ctypes.Structure):
        _fields_ = [("BasicLimitInformation", _BasicLimits), ("IoInfo", ctypes.c_ulonglong * 6),
                    ("ProcessMemoryLimit", ctypes.c_size_t), ("JobMemoryLimit", ctypes.c_size_t),
                    ("PeakProcessMemoryUsed", ctypes.c_size_t), ("PeakJobMemoryUsed", ctypes.c_size_t)]

    ntdll = ctypes.WinDLL("ntdll")
    for _name, _restype, _argtypes in (
        ("CreateJobObjectW", wintypes.HANDLE, [ctypes.c_void_p, wintypes.LPCWSTR]),
        ("SetInformationJobObject", wintypes.BOOL, [wintypes.HANDLE, ctypes.c_int, ctypes.c_void_p, wintypes.DWORD]),
        ("AssignProcessToJobObject", wintypes.BOOL, [wintypes.HANDLE, wintypes.HANDLE]),
        ("CloseHandle", wintypes.BOOL, [wintypes.HANDLE]),
    ):
        getattr(kernel32, _name).restype = _restype
        getattr(kernel32, _name).argtypes = _argtypes
    ntdll.NtResumeProcess.restype = ctypes.c_long
    ntdll.NtResumeProcess.argtypes = [wintypes.HANDLE]

    def spawn(command, **popen):
        """Starts `command` suspended, puts it in a job object below normal priority that every
        process it starts joins and that ends them all when it closes, then lets it run; returns
        (process, job). A command that cannot be placed in its job is ended and refused."""
        job = kernel32.CreateJobObjectW(None, None)
        if not job:
            raise ctypes.WinError(ctypes.get_last_error())
        limits = _ExtendedLimits()
        limits.BasicLimitInformation.LimitFlags = 0x20 | 0x800 | 0x2000
        limits.BasicLimitInformation.PriorityClass = BELOW_NORMAL
        if not kernel32.SetInformationJobObject(job, 9, ctypes.byref(limits), ctypes.sizeof(limits)):
            error = ctypes.WinError(ctypes.get_last_error())
            kernel32.CloseHandle(job)
            raise error
        process = subprocess.Popen(command, creationflags=BELOW_NORMAL | 0x4, **popen)
        if not kernel32.AssignProcessToJobObject(job, int(process._handle)) or ntdll.NtResumeProcess(int(process._handle)) < 0:
            error = ctypes.WinError(ctypes.get_last_error())
            process.kill()
            process.wait()
            kernel32.CloseHandle(job)
            raise error
        return process, job

    def close(job):
        kernel32.CloseHandle(job)

    def end(process, job):
        """Ends a command and everything it started, by closing its job."""
        kernel32.CloseHandle(job)
else:
    def spawn(command, **popen):
        return subprocess.Popen(command, start_new_session=True, preexec_fn=lambda: os.nice(10), **popen), None

    def close(job):
        pass

    def end(process, job):
        import signal
        try:
            os.killpg(process.pid, signal.SIGTERM)
        except OSError:
            pass


class Runner:
    """Runs decided jobs, logging to the run folder. Passes are returned, not recorded, so the run
    records them only if the files it read are still the files it hashed."""

    def __init__(self, folder, values):
        self.folder, self.values = folder, values
        self.lock = threading.Lock()
        self.live = {}
        self.cancelled = False

    def cancel(self):
        """Ends every local command this run started, and starts no more. A Boat command is let
        finish, so it releases its VM; its result is discarded with the run's."""
        with self.lock:
            self.cancelled = True
            ending = {key: entry for key, entry in self.live.items() if not entry[2]}
            for key in ending:
                del self.live[key]
        for process, job, _ in ending.values():
            end(process, job)

    def execute(self, command, log, unset=(), boat=False):
        env = {name: value for name, value in os.environ.items() if name not in unset}
        env["PYTHONUTF8"] = "1"
        with open(log, "ab") as out:
            out.write(f"$ {' '.join(command)}\n".encode())
            out.flush()
            with self.lock:
                if self.cancelled:
                    return 130
                try:
                    process, job = spawn(command, cwd=ROOT, stdout=out, stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL, env=env)
                except OSError as error:
                    out.write(f"could not start: {error}\n".encode())
                    return 127
                self.live[id(process)] = (process, job, boat)
            try:
                return process.wait()
            finally:
                with self.lock:
                    mine = self.live.pop(id(process), None)
                if mine:
                    close(job)

    def run(self, decision):
        """(the outcome, the keys its pass covers)."""
        job, log = decision.job, self.folder / f"{decision.job.name}.log"
        if job.heavy and game_running():
            return Outcome(job.name, "held", "the game lock exists; heavy local work waits for the game"), []
        started = time.monotonic()
        values = dict(self.values, files=decision.arguments, packages=decision.arguments)
        if job.kind == "files" and decision.whole and job.whole:
            code = self.execute(expand(job.whole, values), log, job.unset)
        elif job.kind == "files":
            codes = [self.execute(command, log, job.unset) for command in batches(job.command, values, decision.arguments)]
            code = next((code for code in codes if code), 0)
        else:
            command = job.hosted if HOSTED and job.hosted else job.command
            code = self.execute(expand(command, values), log, job.unset, boat=job.kind == "crates" and not HOSTED)
        seconds = time.monotonic() - started
        detail = decision.detail
        if job.kind == "crates" and log.exists():
            found = re.findall(rb"generated (\d+) warnings?(?: \((\d+) duplicates?\))?", log.read_bytes())
            warnings = sum(int(n) - int(d or 0) for n, d in found)
            detail = ", ".join(filter(None, [detail, f"{warnings} warnings" if warnings else ""]))
        if code == 0:
            return Outcome(job.name, "pass", detail, seconds, str(log)), list(decision.keys.values())
        boat_failed = job.kind == "crates" and log.exists() and b"BOAT UNAVAILABLE" in log.read_bytes()
        if job.kind == "crates" and (code == BOAT_FAILED or boat_failed):
            return Outcome(job.name, "not run", "Boat itself failed (exit 75); run again", seconds, str(log)), []
        if self.cancelled:
            return Outcome(job.name, "cancelled", "a newer run superseded this one", seconds, str(log)), []
        if code == 127:
            return Outcome(job.name, "not run", "the command could not start", seconds, str(log)), []
        return Outcome(job.name, "fail", ", ".join(filter(None, [detail, f"exit {code}"])), seconds, str(log)), []


def probe(tool):
    """Where an optional tool is and which version it says it is, or None when it is absent."""
    found = shutil.which(tool)
    if not found:
        return None
    try:
        said = subprocess.run([found, "--version"], capture_output=True, text=True, errors="replace").stdout
    except OSError:
        said = None
    return [found, said]


def consistency(pipeline, values):
    """Whether each toolchain's `consistent` command accepts its files, run in a scratch folder
    holding copies of only those files (the `installed` list's first two), so the command reads
    nothing installed and changes nothing in the worktree."""
    agrees, read = {}, {}
    for name, toolchain in pipeline.toolchains.items():
        if not toolchain.consistent:
            continue
        with tempfile.TemporaryDirectory() as scratch:
            try:
                for path in consistent_files(toolchain):
                    data = (ROOT / path).read_bytes()
                    read[path] = hashlib.sha256(data).hexdigest()
                    Path(scratch, Path(path).name).write_bytes(data)
                agrees[name] = subprocess.run(expand(toolchain.consistent, values), cwd=scratch, capture_output=True).returncode == 0
            except OSError:
                agrees[name] = False
    return agrees, read


def consistent_files(toolchain):
    """The files a toolchain's `consistent` command reads: the manifest, the lockfile and any
    `.npmrc` among its files."""
    paths = [*toolchain.installed[:2], *(path for path in toolchain.files if Path(path).name.startswith(".npmrc"))]
    return [path for path in paths if (ROOT / path).is_file()]


def unchanged_since(read):
    """Whether every file the consistency check read still holds what it read."""
    for path, hashed in read.items():
        try:
            if hashlib.sha256((ROOT / path).read_bytes()).hexdigest() != hashed:
                return False
        except OSError:
            return False
    return True


def toolchain_digests(pipeline, tree, values, consistent):
    """Each toolchain's digest: its version, its files, and the content of what it requires, so
    an installation that lags its lockfile hashes differently from one that matches."""
    digests, missing = {}, {}
    for name, toolchain in pipeline.toolchains.items():
        version = None
        if toolchain.version:
            try:
                found = subprocess.run(expand(toolchain.version, values), cwd=ROOT, capture_output=True, text=True)
                version = found.stdout if found.returncode == 0 else None
            except OSError:
                pass
            if version is None:
                missing[name] = f"{expand(toolchain.version, values)[0]} is not available"
        installed = {}
        for path in toolchain.requires:
            try:
                installed[path] = hashlib.sha256((ROOT / path).read_bytes()).hexdigest()
            except OSError:
                installed[path] = None
        absent = [path for path, value in installed.items() if value is None]
        if absent and name not in missing:
            missing[name] = f"{', '.join(absent)} is missing" + (" (run npm ci)" if name == "node" else "")
        absent_tools = [tool for tool in toolchain.tools if not shutil.which(tool)]
        if absent_tools and name not in missing:
            missing[name] = f"{', '.join(absent_tools)} is not available"
        if toolchain.installed and name not in missing:
            manifest, lock, present = toolchain.installed
            stale = npm_stale(ROOT / manifest, ROOT / lock, ROOT / present)
            if stale:
                missing[name] = f"the npm installation doesn't match the lockfile ({stale}; run npm ci)"
        if toolchain.consistent and name not in missing and not consistent.get(name, True):
            missing[name] = f"`{' '.join(toolchain.consistent)}` fails: the manifest and lockfile disagree"
        files = matcher(toolchain.files)
        probed = {tool: probe(tool) for tool in toolchain.probes}
        digests[name] = digest(version, installed, probed, file_entries(tree, [path for path in tree if files(path)]))
    return digests, missing


def npm_stale(manifest, lock, installed):
    """Why `npm ci` would not reproduce what is installed, or None: the manifest's dependencies
    differ from the lockfile's, or a package is installed at another version or from other
    contents than the lockfile gives it, missing although the lockfile installs it, or installed
    although the lockfile doesn't list it."""
    try:
        declared = json.loads(manifest.read_text(encoding="utf-8"))
        wanted = json.loads(lock.read_text(encoding="utf-8")).get("packages", {})
        present = json.loads(installed.read_text(encoding="utf-8")).get("packages", {})
    except (OSError, ValueError) as error:
        return f"unreadable: {error}"
    for field_name in ("dependencies", "devDependencies", "optionalDependencies", "peerDependencies"):
        if declared.get(field_name, {}) != wanted.get("", {}).get(field_name, {}):
            return f"package.json {field_name} differ from the lockfile's"
    for path, entry in sorted(wanted.items()):
        if not path or entry.get("link"):
            continue
        if path not in present and (entry.get("optional") or entry.get("peer")):
            continue
        have = present.get(path, {})
        if have.get("version") != entry.get("version"):
            return f"{path.removeprefix('node_modules/')} {have.get('version')} is installed, {entry.get('version')} is locked"
        try:
            on_disk = json.loads((installed.parent.parent / path / "package.json").read_text(encoding="utf-8")).get("version")
        except (OSError, ValueError):
            on_disk = None
        if on_disk != entry.get("version"):
            return f"{path.removeprefix('node_modules/')} is not installed at {entry.get('version')} on disk"
        if entry.get("integrity") and have.get("integrity") != entry.get("integrity"):
            return f"{path.removeprefix('node_modules/')} is installed from other contents than the lockfile's"
    for path in sorted(present):
        if path and path not in wanted and not present[path].get("link"):
            return f"{path.removeprefix('node_modules/')} is installed but not locked"
    modules = installed.parent
    try:
        folders = [entry for entry in modules.iterdir() if entry.is_dir() and not entry.name.startswith(".")]
        folders = [inner for entry in folders for inner in (entry.iterdir() if entry.name.startswith("@") else [entry]) if inner.is_dir()]
    except OSError as error:
        return f"node_modules unreadable: {error}"
    for folder in sorted(folders):
        path = "node_modules/" + folder.relative_to(modules).as_posix()
        if path not in wanted and (folder / "package.json").exists():
            return f"{path.removeprefix('node_modules/')} is on disk but not locked"
    return None


def definitions_at(base):
    """Each job's definition in the base's pipeline, or None when it has none that loads."""
    shown = subprocess.run(["git", "show", f"{base}:tools/ci.toml"], cwd=ROOT, capture_output=True)
    if shown.returncode:
        return None
    try:
        return {job.name: job.definition for job in load_pipeline(shown.stdout.decode("utf-8")).jobs}
    except (SystemExit, KeyError, ValueError, TypeError):
        return None


def upstream_repository(upstream):
    """`owner/name` of the GitHub repository the upstream's remote points at, or None."""
    remote = upstream.split("/", 1)[0]
    url = git("remote", "get-url", remote, check=False).strip()
    found = re.search(r"github\.com[:/]([^/]+/[^/]+?)(?:\.git)?/?$", url)
    return found.group(1) if found else None


def post_status(head, state, description, say, context="ci/check", repository=None):
    """Posts `ci/check` through tools/commit_status.py; False when the status was not posted for
    a reason other than GitHub not having the commit yet."""
    sys.path.insert(0, str(ROOT / "tools"))
    import commit_status
    previous = os.environ.get("GH_REPO")
    if repository:
        os.environ["GH_REPO"] = repository
    try:
        posted, message = commit_status.post(state, description, context=context, sha=head)
    finally:
        if repository:
            if previous is None:
                os.environ.pop("GH_REPO", None)
            else:
                os.environ["GH_REPO"] = previous
    if posted:
        say(f"{context}: {message}: {description}")
        return True
    if "No commit found for SHA" in message:
        say(f"{context}: not posted, GitHub doesn't have {head[:10]} yet; push it and run again (passes are reused)")
        return True
    say(f"{context}: NOT POSTED: {message}")
    return False


def report_line(outcome):
    seconds = f"{outcome.seconds:5.1f}s" if outcome.seconds else "      "
    return f"  {outcome.state.upper() if outcome.state == 'fail' else outcome.state:8} {outcome.name:22} {seconds}  {outcome.detail}"


def snapshot(pipeline):
    """HEAD, the file hashes, every file's modification time and the content of what each
    toolchain requires, so a run that saw an edit later undone, or an install made while it ran,
    still differs at its end. Folders are left out: running a job creates caches in them."""
    tree = read_tree()
    times = []
    for path in sorted({path for toolchain in pipeline.toolchains.values() for path in toolchain.requires}):
        try:
            times.append((path, hashlib.sha256((ROOT / path).read_bytes()).hexdigest(), os.stat(ROOT / path).st_mtime_ns))
        except OSError:
            times.append((path, None))
    for path in sorted(tree):
        try:
            times.append((path, os.stat(ROOT / path).st_mtime_ns))
        except OSError:
            times.append((path, None))
    return git("rev-parse", "HEAD").strip(), tree, digest(times)


class Active:
    """The run of a tier on a branch that is current. Starting a run makes it current, before it
    plans, and the record stays after it ends; an older run that sees another one current stops,
    and records and posts nothing."""

    def __init__(self, store, tier, run):
        branch = git("rev-parse", "--abbrev-ref", "HEAD", check=False).strip()
        self.path = store.root / "active" / f"{digest(tier, str(ROOT) if branch in ('', 'HEAD') else branch)[:20]}.json"
        self.run = run
        for attempt in range(50):
            try:
                write_json(self.path, {"run": run, "tier": tier, "branch": branch, "worktree": str(ROOT)})
                break
            except PermissionError:
                if attempt == 49:
                    raise
                time.sleep(0.1)

    def superseded(self):
        """Whether another run is current. A record being replaced at that moment reads as
        unreadable, which is not taken as superseded; the next check sees the new one."""
        try:
            return json.loads(self.path.read_text(encoding="utf-8")).get("run") != self.run
        except (OSError, ValueError):
            return False


def wait_all(futures, say, runner=None, active=None):
    """Each future's result. A first Ctrl+C, which the commands get too, waits for them to stop
    so Boat can release its VM; a second ends them with their job objects. A run that a newer
    one supersedes ends its commands."""
    results, interrupted = [], False
    for future in futures:
        while True:
            try:
                results.append(future.result(timeout=1))
                break
            except TimeoutError:
                if active and runner and not runner.cancelled and active.superseded():
                    say("Superseded by a newer run on this branch: ending the commands this run started")
                    runner.cancel()
                continue
            except KeyboardInterrupt:
                if interrupted:
                    print("Ending the commands it started.", flush=True)
                    os._exit(130)
                interrupted = True
                say("Interrupted: waiting for the commands to stop; Ctrl+C again ends them")
        outcome = results[-1][0]
        say(f"  done {outcome.name}: {outcome.state} in {outcome.seconds:.1f}s")
    if interrupted:
        os._exit(130)
    return results


def boat_arguments(command):
    """The arguments a command passes to tools/boat.py, or None when it does not run Boat."""
    for at, part in enumerate(command):
        if part.replace("\\", "/").endswith("tools/boat.py"):
            return command[at + 1:]
    return None


def wake_boat(decisions, values):
    """Starts resuming the VM each Boat job will take, before any local work, through
    tools/boat.py's `start_waking`; a wake that cannot start leaves the run to choose its VM."""
    sys.path.insert(0, str(ROOT / "tools"))
    try:
        import boat
    except Exception:
        return
    for decision in decisions:
        if decision.state != "run" or decision.job.kind != "crates":
            continue
        arguments = boat_arguments(expand(decision.job.command, dict(values, packages=decision.arguments)))
        if arguments:
            try:
                boat.start_waking(arguments, cwd=ROOT)
            except OSError:
                pass


def run_tier(args, tier):
    started = time.monotonic()
    say = lambda text: print(text, flush=True)
    active = None if args.dry_run else Active(Store(store_root()), tier, f"{time.strftime('%Y%m%d-%H%M%S')}-{os.getpid()}-{time.monotonic_ns()}")
    definition = PIPELINE.read_text(encoding="utf-8")
    pipeline = load_pipeline(definition)
    base = git("merge-base", "HEAD", args.base, check=False).strip()
    if not base:
        sys.exit(f"No merge base with {args.base}: fetch it, or pass --base.")
    values = {"python": sys.executable, "node": shutil.which("node") or "node",
              "npm": shutil.which("npm.cmd") or shutil.which("npm") or "npm", "base": base}
    consistent, read = consistency(pipeline, values)
    before = snapshot(pipeline)
    head, tree = before[:2]
    if PIPELINE.read_text(encoding="utf-8") != definition or not unchanged_since(read):
        sys.exit("tools/ci.toml or the npm manifests changed while ci started; run it again.")
    changed = changed_since(base)
    clean = not git("status", "--porcelain", "--untracked-files=normal").strip()
    store = Store(store_root())
    toolchains, missing = toolchain_digests(pipeline, tree, values, consistent)
    order = list(pipeline.tiers)
    needs_crates = any(job.kind == "crates" and order.index(job.tier) <= order.index(tier) for job in pipeline.jobs)
    ctx = Context(tree, changed, base, head, toolchains, crates_from_manifests(ROOT) if needs_crates else {}, store.passed,
                  pipeline, missing, game_running(), definitions_at(base))
    decisions = [decide(job, ctx, tier) for job in pipeline.jobs]
    if not args.dry_run:
        wake_boat(decisions, values)
    say(f"ci {tier}: HEAD {head[:10]}{'' if clean else ' with uncommitted changes'}, base {base[:10]}, {len(changed)} changed files")
    say("changes:")
    for decision in decisions:
        say(f"  {decision.state:8} {decision.job.name:22} {decision.detail}")
    if args.dry_run:
        return 0
    folder = store.run_folder(head)
    runner = Runner(folder, values)
    running = sorted((d for d in decisions if d.state == "run"), key=lambda d: d.job.kind != "crates")
    outcomes = [Outcome(d.job.name, d.state, d.detail) for d in decisions if d.state != "run"]
    pool = ThreadPoolExecutor(max_workers=max(1, len(running)))
    results = wait_all([pool.submit(runner.run, decision) for decision in running], say, runner, active)
    pool.shutdown()
    if runner.cancelled or active.superseded():
        say(f"ci/{tier}: superseded by a newer run on this branch; nothing recorded or posted")
        return SUPERSEDED
    outcomes += [outcome for outcome, _ in results]
    steady = snapshot(pipeline) == before
    if steady:
        entry = {"sha": head[:10] + ("" if clean else "+changes"), "at": time.strftime("%Y-%m-%dT%H:%M:%S")}
        for outcome, keys in results:
            for key in keys:
                store.record(key, dict(entry, job=outcome.name, seconds=round(outcome.seconds, 1)))
    seconds = time.monotonic() - started
    ordered = sorted(outcomes, key=lambda outcome: (ORDER.index(outcome.state), outcome.name))
    for outcome in ordered:
        if outcome.state in ("fail", "not run") and outcome.log:
            lines = Path(outcome.log).read_text(encoding="utf-8", errors="replace").splitlines()
            say(f"\n== {outcome.name} ({outcome.log})\n" + "\n".join(lines[-60:]))
    say("")
    for outcome in ordered:
        say(report_line(outcome))
    budget = pipeline.tiers[tier]
    over = seconds > budget
    say(f"{tier} tier: {seconds:.1f}s, {'OVER' if over else 'within'} its {budget}s budget")
    state = verdict(ordered) if steady else "error"
    description = describe(ordered, seconds)
    write_json(folder / "run.json", {
        "tier": tier, "head": head, "base": base, "clean": clean, "steady": steady, "state": state,
        "seconds": round(seconds, 1), "budget": budget, "over_budget": over, "description": description,
        "jobs": [{"name": o.name, "state": o.state, "detail": o.detail, "seconds": round(o.seconds, 1)} for o in ordered]})
    code = EXIT[state]
    context = f"ci/{tier}"
    if tier == "check":
        official = base == git("merge-base", "HEAD", args.upstream, check=False).strip()
        why = f"it compared against {args.base}, not the merge base with {args.upstream}"
    else:
        parent = git("rev-parse", "--verify", "--quiet", "HEAD^1", check=False).strip()
        official = bool(parent) and base == parent and \
            subprocess.run(["git", "merge-base", "--is-ancestor", "HEAD", args.upstream], cwd=ROOT).returncode == 0
        why = f"HEAD is not on {args.upstream}, or the base is not HEAD's first parent"
    if not steady:
        say(f"{context}: not posted and nothing recorded: files or HEAD changed while it ran; run it again")
    elif args.no_status:
        say(f"{context}: not posted (--no-status)")
    elif not clean:
        say(f"{context}: not posted, the worktree has uncommitted changes, so no commit is what ran")
    elif not official:
        say(f"{context}: not posted, {why}")
    elif active.superseded():
        say(f"{context}: superseded by a newer run on this branch; not posted")
        return SUPERSEDED
    elif not post_status(head, state, description, say, context, upstream_repository(args.upstream)):
        code = EXIT["error"] if code in (EXIT["success"], EXIT["pending"]) else code
    store.prune()
    return code


def plan(tier, base_ref, gates, exact=False):
    """The decisions for a tier against the merge base with `base_ref`, or against `base_ref`
    itself when `exact` (a push compares with the tip it replaced, which a force-push may have
    dropped). With `gates`, from the files alone: no toolchain is asked, no pass is reused,
    nothing is held or deferred."""
    pipeline = load_pipeline(PIPELINE.read_text(encoding="utf-8"))
    if exact:
        base = git("rev-parse", "--verify", "--quiet", f"{base_ref}^{{tree}}", check=False).strip() and base_ref
    else:
        base = git("merge-base", "HEAD", base_ref, check=False).strip()
    if not base:
        sys.exit(f"No base at {base_ref}: fetch it, or pass --base.")
    values = {"python": sys.executable, "node": shutil.which("node") or "node",
              "npm": shutil.which("npm.cmd") or shutil.which("npm") or "npm", "base": base}
    tree, changed, head = read_tree(), changed_since(base), git("rev-parse", "HEAD").strip()
    if gates:
        toolchains, missing, passed = {name: name for name in pipeline.toolchains}, {}, lambda key: None
    else:
        toolchains, missing = toolchain_digests(pipeline, tree, values, consistency(pipeline, values)[0])
        passed = Store(store_root()).passed
    order = list(pipeline.tiers)
    crates = crates_from_manifests(ROOT) if any(job.kind == "crates" and order.index(job.tier) <= order.index(tier) for job in pipeline.jobs) else {}
    ctx = Context(tree, changed, base, head, toolchains, crates, passed, pipeline, missing, False, definitions_at(base),
                  hosted=gates or HOSTED)
    return pipeline, values, [decide(job, ctx, tier) for job in pipeline.jobs]


def changes(args):
    """The workflow's `changes` job: each job's gate, and the matrix of the groups with work."""
    pipeline, values, decisions = plan(args.tier, args.base, gates=True, exact=args.exact_base)
    groups = {}
    for decision in decisions:
        print(f"  {decision.state:8} {decision.job.name:22} {decision.job.group:8} {decision.detail}")
        if decision.state == "run":
            groups.setdefault(decision.job.group, []).append(decision.job.name)
    matrix = [dict(pipeline.groups[group], group=group, jobs=" ".join(names),
                   **{f"needs_{need}": True for need in pipeline.groups[group].get("setup", [])})
              for group, names in groups.items()]
    for entry in matrix:
        entry.pop("setup", None)
    print(f"groups with work: {', '.join(groups) or 'none'}")
    unplanned = [decision.job.name for decision in decisions if decision.state == "not run"]
    if unplanned:
        print(f"could not plan: {', '.join(unplanned)}")
        return EXIT["error"]
    if args.github_output:
        with open(args.github_output, "a", encoding="utf-8") as out:
            out.write(f"matrix={json.dumps(matrix, separators=(',', ':'))}\n")
            out.write(f"any={'true' if matrix else 'false'}\n")
            out.write(f"base={values['base']}\n")
    return 0


def job_group(args):
    """The workflow's matrix job: runs one group's jobs that `changes` gated in, Rust jobs one
    after another (they share a target folder) and the rest in parallel."""
    pipeline, values, decisions = plan(args.tier, args.base, gates=False, exact=args.exact_base)
    mine = [decision for decision in decisions if decision.job.group == args.group]
    if not mine:
        sys.exit(f"tools/ci.toml has no group {args.group!r}")
    store = Store(store_root())
    folder = store.run_folder(git("rev-parse", "HEAD").strip())
    runner = Runner(folder, values)
    outcomes = [Outcome(d.job.name, d.state, d.detail) for d in mine if d.state != "run"]
    serial = [d for d in mine if d.state == "run" and d.job.kind == "crates"]
    parallel = [d for d in mine if d.state == "run" and d.job.kind != "crates"]
    pool = ThreadPoolExecutor(max_workers=max(1, len(parallel) + 1))
    futures = [pool.submit(runner.run, decision) for decision in parallel]
    futures.append(pool.submit(lambda: [runner.run(decision) for decision in serial]))
    results = []
    for future in futures:
        found = future.result()
        results += found if isinstance(found, list) else [found]
    pool.shutdown()
    outcomes += [outcome for outcome, _ in results]
    ordered = sorted(outcomes, key=lambda outcome: (ORDER.index(outcome.state), outcome.name))
    for outcome in ordered:
        if outcome.state in ("fail", "not run") and outcome.log:
            lines = Path(outcome.log).read_text(encoding="utf-8", errors="replace").splitlines()
            print(f"\n== {outcome.name}\n" + "\n".join(lines[-200:]), flush=True)
    print("")
    for outcome in ordered:
        print(report_line(outcome), flush=True)
    state = verdict(ordered)
    print(f"group {args.group}: {state}")
    return EXIT[state]


def main(argv=None):
    for stream in (sys.stdout, sys.stderr):
        if hasattr(stream, "reconfigure"):
            stream.reconfigure(errors="replace")
    parser = argparse.ArgumentParser(description="Run a tier of tools/ci.toml.")
    commands = parser.add_subparsers(dest="command", required=True)
    for tier, help, base in (("check", "the per-change tier", None), ("main", "the tier for each new main commit", "HEAD~1")):
        command = commands.add_parser(tier, help=help)
        command.add_argument("--upstream", default="origin/main", metavar="REF", help="the main branch to compare with and post for (default origin/main)")
        command.add_argument("--base", default=base, metavar="REF", help="default: the upstream for check, HEAD~1 for main")
        command.add_argument("--dry-run", action="store_true", help="print what would run, reuse or skip")
        command.add_argument("--no-status", action="store_true", help="don't post the commit status")
    gating = commands.add_parser("changes", help="the workflow's gates and job matrix")
    gating.add_argument("--tier", default="check")
    gating.add_argument("--base", required=True, metavar="REF")
    gating.add_argument("--github-output", metavar="FILE")
    gating.add_argument("--exact-base", action="store_true", help="compare with the base itself, not the merge base")
    grouped = commands.add_parser("job", help="run one group's jobs, as the workflow does")
    grouped.add_argument("--tier", default="check")
    grouped.add_argument("--group", required=True)
    grouped.add_argument("--base", required=True, metavar="REF")
    grouped.add_argument("--exact-base", action="store_true", help="compare with the base itself, not the merge base")
    args = parser.parse_args(argv)
    if args.command == "changes":
        return changes(args)
    if args.command == "job":
        return job_group(args)
    args.base = args.base or args.upstream
    return run_tier(args, args.command)


if __name__ == "__main__":
    sys.exit(main())
