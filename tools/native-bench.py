import argparse
import csv
from dataclasses import dataclass, field
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys


METRICS = ("cpu_ms", "rows", "opens", "restores", "bytes", "changed_pct")


@dataclass
class View:
    page: str
    slots: dict
    reasons: set
    totals: dict = field(default_factory=dict)
    samples: int = 0
    changed: int = 0
    pending: bool = False
    replies: dict = field(default_factory=dict)
    answered: set = field(default_factory=set)
    viewport: dict | None = None
    rows_complete: bool = True

    def step(self, row, event):
        if row is None:
            return
        if "Err" in row:
            self.reasons.add(row["Err"])
            self.replies.setdefault(event["hour"], []).append(("failure", row["Err"]))
            return
        sample = row["Ok"]
        self.rows_complete &= sample.get("rows_complete", True)
        if "panel_through" in sample:
            self.replies.setdefault(event["hour"], []).append(("panel-completed-through", sample["panel_through"]))
        for query, through in sample.get("limited", []):
            slot = self.slots.get(str(query), f"live cells {query}")
            self.replies.setdefault(event["hour"], []).append((slot, "limited-through", through))
        self.reasons.update(f"{key}: {reason}" for key, reason in sample["refusals"].items())
        for reply in sample["replies"]:
            self.answered.add(str(reply["id"]))
            slot = self.slots.get(str(reply["id"]), f"live cells {reply['id']}")
            self.replies.setdefault(event["hour"], []).append((slot, reply["hour"], reply.get("kind"), reply["hash"]))
        if event["seed"]:
            return
        for key, value in sample["counts"].items():
            self.totals[key] = self.totals.get(key, 0) + value
        self.pending |= sample["changed"]
        if event["sample"] is not None:
            self.samples += 1
            self.changed += self.pending
            self.pending = False

    def outcomes(self):
        return self.reasons | {f"No reply delivered for {slot}" for key, slot in self.slots.items() if key not in self.answered}

    def notes(self):
        outcome = self.outcomes()
        if not self.rows_complete:
            outcome.add("The followed reader does not count all row visits")
        return " / ".join(sorted(outcome))

    def costs(self, end):
        if not self.samples:
            return dict.fromkeys(METRICS)
        ns_per_cycle = end["service_cpu_ns"] / end["service_cycles"] if end["service_cycles"] else None
        return {
            "cpu_ms": None if ns_per_cycle is None else self.totals.get("cycles", 0) * ns_per_cycle / self.samples / 1e6,
            **{key: self.totals.get(key, 0) / self.samples for key in METRICS[2:-1]},
            "changed_pct": 100 * self.changed / self.samples,
            "rows": self.totals.get("rows", 0) / self.samples if self.rows_complete else None,
        }


@dataclass
class Run:
    spec: dict
    views: dict = field(default_factory=dict)
    indexed: list = field(default_factory=list)
    end: dict | None = None
    samples: list = field(default_factory=list)


def runs(paths):
    found = {}
    for path in paths:
        current = None
        with path.open(encoding="utf-8-sig") as stream:
            for number, line in enumerate(stream, 1):
                event = json.loads(line)
                match event["event"]:
                    case "run":
                        if current is not None and current.end is None:
                            raise ValueError(f"{path}:{number}: previous run is incomplete")
                        current = Run(event)
                        fingerprint = path.with_suffix(".sha256")
                        if fingerprint.is_file():
                            current.spec["recording_sha256"] = fingerprint.read_text(encoding="ascii").strip()
                        key = (event["campaign"], event["quantum"])
                        if key in found:
                            raise ValueError(f"{path}:{number}: duplicate recording and quantum")
                        found[key] = current
                    case "view" if current is not None and current.end is None:
                        key = (event["key"]["kind"], event["key"]["view"])
                        if key in current.views or event["index"] != len(current.indexed):
                            raise ValueError(f"{path}:{number}: duplicate or out-of-order view {key}")
                        view = View(event["key"]["page"], event["slots"], set(event["reasons"]),
                                    viewport=event["key"].get("viewport"))
                        current.views[key] = view
                        current.indexed.append(view)
                    case "step" if current is not None and current.end is None:
                        if len(event["views"]) != len(current.indexed):
                            raise ValueError(f"{path}:{number}: step has an incomplete view list")
                        if event["sample"] is not None:
                            current.samples.append(event["sample"])
                        for view, sample in zip(current.indexed, event["views"], strict=True):
                            view.step(sample, event)
                    case "end" if current is not None and current.end is None:
                        spec = current.spec
                        expected = list(range(spec["first"], spec["last"] + 1, spec["quantum"]))
                        if current.samples != expected or event["samples"] != len(expected) - 1:
                            raise ValueError(f"{path}:{number}: stepped hours do not match the run window")
                        current.end = event
                    case _:
                        raise ValueError(f"{path}:{number}: unexpected {event['event']} event")
    if not found or any(run.end is None for run in found.values()):
        raise ValueError("Input contains an incomplete benchmark run")
    return found


def traces(path):
    return sorted(path.glob("*.jsonl")) if path.is_dir() else [path]


def comparison(before, after, output):
    writer = csv.writer(output, lineterminator="\n")
    writer.writerow(["campaign", "quantum", "kind", "view", "page", *(
        f"{key}_{which}" for key in METRICS for which in ("before", "after")
    ), "first_different_hour", "before_reason", "after_reason"])
    if before.keys() != after.keys():
        raise ValueError("Both builds must cover the same recordings and quanta")
    differs = False
    for key in sorted(before):
        left, right = before[key], after[key]
        for field_name in ("trace_version", "recording_bytes", "format", "branch", "country", "first", "last", "recording_sha256"):
            if left.spec.get(field_name) != right.spec.get(field_name):
                raise ValueError(f"{key}: {field_name} differs across runs")
        for widget in sorted(left.views.keys() | right.views.keys()):
            old = left.views.get(widget, View("", {}, {"View absent from the registry"}))
            new = right.views.get(widget, View("", {}, {"View absent from the registry"}))
            old_cost, new_cost = old.costs(left.end), new.costs(right.end)
            first = next((hour for hour in sorted(old.replies.keys() | new.replies.keys())
                          if old.replies.get(hour) != new.replies.get(hour)), None)
            differs |= first is not None or old.outcomes() != new.outcomes() or old.slots != new.slots or old.viewport != new.viewport
            writer.writerow([*key, *widget, new.page or old.page, *(
                value for metric in METRICS for value in (old_cost[metric], new_cost[metric])
            ), first, old.notes(), new.notes()])
    return differs


def summary(fixtures, output_csv, output_markdown):
    columns = ["rank", "kind", "view", "page"]
    labels = list(fixtures)
    for label in labels:
        columns.extend(f"{label}_{metric}" for metric in (*METRICS, "steps", "reason"))
    rows = []
    for key in set().union(*(run.views.keys() for run in fixtures.values())):
        result = dict(zip(("kind", "view"), key, strict=True))
        for label, run in fixtures.items():
            view = run.views.get(key, View("", {}, {"View absent from the registry"}))
            if view.page:
                result["page"] = view.page
            data = view.costs(run.end) | {"steps": view.samples, "reason": view.notes()}
            result.update({f"{label}_{metric}": value for metric, value in data.items()})
        rows.append(result)
    rows.sort(key=lambda row: (-max(row[f"{label}_cpu_ms"] or 0 for label in labels), row["kind"], row["view"]))
    with output_csv.open("w", encoding="utf-8", newline="") as output:
        writer = csv.DictWriter(output, columns)
        writer.writeheader()
        for rank, row in enumerate(rows, 1):
            row["rank"] = rank
            writer.writerow(row)
    with output_markdown.open("w", encoding="utf-8") as output:
        output.write("# Dashboard widget playback cost per 24-hour step\n\n")
        for label, run in fixtures.items():
            output.write(f"- {label}: {run.end['samples']} steps, {len(run.views)} views, "
                         f"{run.end['elapsed_seconds']:.1f} s playback wall time, branch {run.spec['branch']}, "
                         f"hours {run.spec['first']} to {run.spec['last']}, country {run.spec['country']}.\n")
        output.write("\nThe playhead uses one service stepper, one step helper and one fetch worker. "
                     "The paused seed is included in reply comparisons and omitted from playback averages. "
                     "CPU estimates use the service thread's measured CPU time and cycle counts. "
                     "Rows, reader opens and restores are counters attributed to the view's service callbacks. "
                     "A dash denotes an unmeasured cost. Refusals and unsupported routes appear in the last column.\n\n")
        head = ["Rank", "Widget", "View", "Page", *labels, "Refusals or measurement limits"]
        def table(values):
            output.write("| " + " | ".join(str(value).replace("|", r"\|").replace("\n", " ") for value in values) + " |\n")
        table(head)
        table(["---"] * len(head))
        for row in rows:
            cells, reasons = [], []
            for label in labels:
                values = [row[f"{label}_{metric}"] for metric in METRICS]
                if values[0] is None:
                    cells.append("—")
                else:
                    ms, read, opened, restored, size, changed = values
                    seen_rows = f"{read:,.1f}" if read is not None else "—"
                    cells.append(f"{ms:.3f} ms · {seen_rows} rows · {opened:.2f} opens · {restored:.2f} restores · {size:,.0f} B · {changed:.1f}% changed")
                if row[f"{label}_reason"]:
                    reasons.append(f"{label}: {row[f'{label}_reason']}")
            table([row["rank"], row["kind"], row["view"], row["page"], *cells, " / ".join(reasons)])



def benchmark(args):
    args.out = args.out.resolve()
    args.out.mkdir(parents=True, exist_ok=True)
    root = Path(__file__).resolve().parents[1]
    profile = args.out / "profile"
    cache = args.out / "cache"
    env = os.environ | {
        "LOCALAPPDATA": str(profile / "local"),
        "APPDATA": str(profile / "roaming"),
        "BATTLEPLAN_CACHE": str(cache),
    }
    for key in ("BATTLEPLAN_LIBRARY", "BATTLEPLAN_CAPTURES"):
        env.pop(key, None)
    for folder in (profile / "local", profile / "roaming", cache):
        folder.mkdir(parents=True, exist_ok=True)
    flags = ["--all-views", "--steps", str(args.steps)]
    for flag in ("branch", "start"):
        value = getattr(args, flag)
        if value is not None:
            flags.extend((f"--{flag}", str(value)))
    seen = set()
    for fixture in args.recording:
        name, recording = fixture.split("=", 1)
        if not re.fullmatch(r"[\w-]+", name, re.ASCII) or name in seen:
            raise ValueError(f"Invalid or duplicate fixture name: {name}")
        seen.add(name)
        recording = Path(recording).resolve(strict=True)
        with recording.open("rb") as stream:
            fingerprint = hashlib.file_digest(stream, "sha256").hexdigest()
        print(f"Measuring {name} ({fingerprint})", flush=True)
        for quantum in ((args.quantum,) if args.quantum is not None else (24, 3, 1)):
            final = args.out / f"{name}-q{quantum}.jsonl"
            partial = final.with_suffix(".partial")
            if final.exists():
                raise ValueError(f"Use a fresh output path: {final} already exists")
            argv = [str(args.bench.resolve(strict=True)), str(recording), str(args.game_dir.resolve(strict=True)),
                    str(args.user_dir.resolve(strict=True)), str(cache.resolve()), args.country, *flags, "--quantum", str(quantum)]
            with partial.open("wb") as output:
                completed = subprocess.run(argv, cwd=root, env=env, stdout=output,
                                           creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0), check=False)
            if completed.returncode:
                raise ValueError(f"Benchmark exited {completed.returncode} for {name} (partial output {partial})")
            results = runs([partial])
            final.with_suffix(".sha256").write_text(fingerprint + "\n", encoding="ascii")
            partial.replace(final)
            print(f"{final} holds {sum(len(run.samples) - 1 for run in results.values())} playback steps", flush=True)


def main():
    parser = argparse.ArgumentParser(description="Summarize or compare native-bench --all-views traces.")
    commands = parser.add_subparsers(dest="command", required=True)
    compare = commands.add_parser("compare")
    compare.add_argument("before", type=Path, help="JSONL trace or directory of traces")
    compare.add_argument("after", type=Path, help="JSONL trace or directory of traces")
    report = commands.add_parser("summary")
    report.add_argument("--fixture", action="append", required=True, metavar="NAME=JSONL")
    report.add_argument("--csv", type=Path, required=True)
    report.add_argument("--markdown", type=Path, required=True)
    measure = commands.add_parser("measure")
    measure.add_argument("--bench", type=Path, required=True)
    measure.add_argument("--recording", action="append", required=True, metavar="NAME=PATH")
    measure.add_argument("--game-dir", type=Path, required=True)
    measure.add_argument("--user-dir", type=Path, required=True)
    measure.add_argument("--out", type=Path, required=True)
    measure.add_argument("--country", default="auto", help="recorded player by default")
    measure.add_argument("--branch", help="main branch by default")
    measure.add_argument("--start", type=int, help="30%% into the selected branch by default")
    measure.add_argument("--steps", type=int, default=48)
    measure.add_argument("--quantum", type=int, choices=(24, 3, 1), help="all three by default")
    args = parser.parse_args()
    try:
        if args.command == "measure":
            benchmark(args)
            return 0
        if args.command == "compare":
            return int(comparison(runs(traces(args.before)), runs(traces(args.after)), sys.stdout))
        fixtures = {}
        for fixture in args.fixture:
            label, trace = fixture.split("=", 1)
            matches = [run for run in runs([Path(trace)]).values() if run.spec["quantum"] == 24]
            if len(matches) != 1 or label in fixtures:
                raise ValueError(f"{fixture}: expected one 24-hour run and a distinct fixture label")
            fixtures[label] = matches[0]
        summary(fixtures, args.csv, args.markdown)
    except (OSError, ValueError) as error:
        parser.exit(2, f"{error}\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
