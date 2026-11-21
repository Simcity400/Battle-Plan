"""Refuse GitHub workflow files GitHub could not run (Python 3.11+, PyYAML).

usage: python tools/check-workflows.py [--head REV]

Each `.github/workflows/*.yml` or `*.yaml` at REV (default: the working tree) must parse as YAML
with no repeated key, and have the shape GitHub reads: a mapping with `on` and `jobs`; each job
a mapping that either runs on `runs-on` with a non-empty list of `steps`, or calls a reusable
workflow with `uses` and has no `steps`; each step a mapping with exactly one of `run` or `uses`,
a non-empty string. Only `true` and `false` are booleans, as in the YAML 1.2 GitHub reads, so `on` is a
key. When `actionlint` is on PATH it also checks the files with the revision's
`.github/actionlint.yaml`; it is skipped, and said so, when it is not. Exits 1 on any failure.
"""
import argparse
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

try:
    import yaml
except ModuleNotFoundError:
    sys.exit("PyYAML is missing: python -m pip install pyyaml")

ROOT = Path(__file__).resolve().parents[1]
FOLDER = ".github/workflows"
CONFIGS = (".github/actionlint.yaml", ".github/actionlint.yml")
BOOL = "tag:yaml.org,2002:bool"


class WorkflowLoader(yaml.SafeLoader):
    """A safe loader with YAML 1.2 booleans that refuses a key repeated in one mapping, which
    PyYAML would overwrite."""

    def construct_mapping(self, node, deep=False):
        seen = set()
        for key, _ in node.value:
            name = self.construct_object(key, deep=deep)
            if name in seen:
                raise yaml.constructor.ConstructorError(None, None, f"repeated key {name!r}", key.start_mark)
            seen.add(name)
        return super().construct_mapping(node, deep)


WorkflowLoader.yaml_implicit_resolvers = {
    first: [(tag, pattern) for tag, pattern in resolvers if tag != BOOL]
    for first, resolvers in yaml.SafeLoader.yaml_implicit_resolvers.items()
}
WorkflowLoader.add_implicit_resolver(BOOL, re.compile(r"^(?:true|True|TRUE|false|False|FALSE)$"), list("tTfF"))


def shape(document):
    """What GitHub would refuse in a parsed workflow, one line each."""
    if not isinstance(document, dict):
        return ["the file is not a mapping"]
    problems = []
    if "on" not in document:
        problems.append("no `on` trigger")
    jobs = document.get("jobs")
    if not isinstance(jobs, dict) or not jobs:
        return [*problems, "`jobs` is not a non-empty mapping"]
    for name, job in jobs.items():
        if not isinstance(job, dict):
            problems.append(f"job {name}: not a mapping")
            continue
        if "uses" in job:
            if not named(job["uses"]):
                problems.append(f"job {name}: `uses` names no workflow")
            if "steps" in job or "runs-on" in job:
                problems.append(f"job {name}: a reusable-workflow call takes neither `steps` nor `runs-on`")
            continue
        if "runs-on" not in job:
            problems.append(f"job {name}: neither `runs-on` nor `uses`")
        steps = job.get("steps")
        if not isinstance(steps, list) or not steps:
            problems.append(f"job {name}: `steps` is not a non-empty list")
            continue
        for index, step in enumerate(steps, 1):
            if not isinstance(step, dict):
                problems.append(f"job {name} step {index}: not a mapping")
            elif ("run" in step) == ("uses" in step):
                problems.append(f"job {name} step {index}: needs exactly one of `run` or `uses`")
            elif "run" in step and not named(step["run"]):
                problems.append(f"job {name} step {index}: `run` is not a non-empty string")
            elif "uses" in step and not named(step["uses"]):
                problems.append(f"job {name} step {index}: `uses` names no action")
    return problems


def named(value):
    return isinstance(value, str) and bool(value.strip())


def problems(name, text):
    try:
        document = yaml.load(text, Loader=WorkflowLoader)
    except yaml.YAMLError as error:
        return [f"{name}: does not parse: {' '.join(str(error).split())}"]
    return [f"{name}: {problem}" for problem in shape(document)]


def files(head):
    """({workflow path: text}, {actionlint config path: text}) at `head`, or in the working tree."""
    if head is None:
        workflows = {f"{FOLDER}/{path.name}": path.read_text(encoding="utf-8") for path in sorted((ROOT / FOLDER).glob("*")) if path.suffix in (".yml", ".yaml")}
        return workflows, {name: (ROOT / name).read_text(encoding="utf-8") for name in CONFIGS if (ROOT / name).is_file()}
    listing = subprocess.run(["git", "ls-tree", "-z", "--name-only", head, "--", f"{FOLDER}/", *CONFIGS], cwd=ROOT, capture_output=True, text=True, encoding="utf-8")
    if listing.returncode:
        sys.exit(f"Cannot list {head}: {listing.stderr.strip()}")
    shown = lambda name: subprocess.run(["git", "show", f"{head}:{name}"], cwd=ROOT, capture_output=True, check=True).stdout.decode("utf-8")
    names = [name for name in listing.stdout.split("\0") if name]
    workflows = {name: shown(name) for name in names if name.startswith(f"{FOLDER}/") and name.endswith((".yml", ".yaml"))}
    return workflows, {name: shown(name) for name in names if name in CONFIGS}


def actionlint(workflows, configs):
    """actionlint's exit code on the files, or None when it is not installed."""
    program = shutil.which("actionlint")
    if program is None:
        return None
    with tempfile.TemporaryDirectory() as scratch:
        Path(scratch, ".git").mkdir()
        for name, text in {**workflows, **configs}.items():
            path = Path(scratch, name)
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(text, encoding="utf-8")
        config = ["-config-file", min(configs)] if configs else []
        return subprocess.run([program, "-no-color", *config, *workflows], cwd=scratch).returncode


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n", 1)[0])
    parser.add_argument("--head", metavar="REV", help="check a revision instead of the working tree")
    args = parser.parse_args()
    workflows, configs = files(args.head)
    found = [problem for name, text in workflows.items() for problem in problems(name, text)]
    for problem in found:
        print(problem)
    linted = actionlint(workflows, configs) if workflows else 0
    if linted is None:
        print("actionlint is not installed: skipped")
    print(f"{len(workflows)} workflow file(s); {len(found)} problem(s)")
    return 1 if found or linted else 0


if __name__ == "__main__":
    sys.exit(main())
