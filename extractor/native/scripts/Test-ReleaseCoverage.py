"""Read-only release evidence inventory. Exit 1 means completion proof is missing.

No builds, captures, package admission, or product launches are performed here.
Raw central runner receipts use the existing START/RESULT/SOURCE dialect.
Historical passing checks remain useful evidence but cannot close current rows.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import platform


def read_json(path):
    return json.loads(Path(path).read_text(encoding="utf-8-sig"))


def reference(path):
    path = Path(path).resolve()
    data = path.read_bytes()
    return {"path": str(path), "bytes": len(data), "sha256": hashlib.sha256(data).hexdigest()}


def checked_reference(ref):
    actual = reference(ref["path"])
    if type(ref["bytes"]) is not int or any(actual[k] != ref[k] for k in ("bytes", "sha256")):
        raise ValueError(f"changed evidence: {ref['path']}")
    return actual


def central_run(directory, revision):
    directory = Path(directory).resolve()
    row = {"path": str(directory), "status": "missing", "issues": []}
    try:
        result = read_json(directory / "RESULT.json")
        if not isinstance(result, dict):
            raise ValueError("RESULT must be a JSON object")
        row["revision"] = result.get("tip")
        row["current_revision"] = result.get("tip") == revision
        row["receipt"] = reference(directory / "RESULT.json")
        refs = result["files"]
        if not isinstance(refs, list) or not refs:
            raise ValueError("no raw evidence references")
        verified = [checked_reference(ref) for ref in refs]
        paths = {Path(ref["path"]).resolve() for ref in verified}
        required = {directory / name for name in (
            "START.json", "SOURCE_BEFORE.json", "SOURCE_AFTER.json", "stdout.txt", "stderr.txt")}
        if not required <= paths:
            raise ValueError("missing pinned START/source snapshots/raw stdout/stderr")
        start = read_json(directory / "START.json")
        if start["tip"] != result["tip"]:
            raise ValueError("START and RESULT revisions differ")
        args = start["argv"]
        # Accept only the committed central entrypoint's unfiltered release invocation.
        if not isinstance(args, list) or len(args) < 9 or args[:5] != [
            "powershell.exe", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File"
        ] or Path(args[5]).name != "Invoke-Validation.ps1" or args[6:9] != [
            "-Mode", "ReleaseManual", "-AllowReleaseManual"
        ]:
            raise ValueError("not the unfiltered central ReleaseManual invocation")
        tail = args[9:]
        if len(tail) % 2 or any(tail[i] not in ("-C07GameRoot", "-ProductionCaptureReceiptPath")
                               or not tail[i + 1] for i in range(0, len(tail), 2)):
            raise ValueError("filtered, planned, or unknown central arguments")
        if (type(result["exit"]) is not int or result["exit"] != 0
                or result.get("guard_failure") is not None
                or result.get("source_unchanged") is not True
                or result.get("head_unchanged") is not True
                or result.get("status_porcelain") != ""):
            raise ValueError("central run failed, was interrupted, or changed source")
        before = read_json(directory / "SOURCE_BEFORE.json")
        after = read_json(directory / "SOURCE_AFTER.json")
        if not isinstance(before, list) or not before or before != after:
            raise ValueError("source snapshots absent, empty, or different")
        if type(result["source_files"]) is not int or result["source_files"] != len(before):
            raise ValueError("source snapshot count differs")
        row["status"] = "current_run_verified" if row["current_revision"] else "historical_pass"
        row["command"] = args
        row["raw_references"] = verified
        row["limitation"] = "Run provenance only; artifact/package identity needs the owning verifiers."
    except (OSError, ValueError, KeyError, TypeError) as error:
        row["status"] = "invalid_or_incomplete"
        row["issues"].append(str(error))
    return row


# These rows map contract areas to their owning executable checks. Presence of
# source is deliberately separate from runtime acceptance; no fabricated PASS.
CONTRACT_ROWS = [
    ("passive_recording", "Recording and coverage", "vanilla-capture",
     "extractor/native/scripts/Test-PassiveCapturePolicy.ps1",
     "powershell.exe -NoProfile -File extractor/native/scripts/Test-PassiveCapturePolicy.ps1",
     "Current vanilla capture: complete scheduled observations or exact typed gaps, tick/hour attribution, restored hooks and unloaded collector."),
    ("automatic_lifecycle", "Time and identity", "vanilla-capture",
     "extractor/native/recorder_host/src/auto_attach.rs",
     None,
     "Parent-brokered current-artifact app-first/game-first, simultaneous games, relaunch, identity change and rollback/branch receipts."),
    ("container_open", "Format and content", "portable-adversarial",
     "replay_interface/native/replay_app/src/main.rs",
     '& "$InstalledRoot/Battle Plan.exe" --validate-recording "$Recording"',
     "Owning container adversarial checks and an actual current-artifact .battleplan open receipt."),
    ("content_admission", "Format and content", "content-admission",
     "replay_interface/native/replay_app/src/content_authority_report.rs",
     '& "$InstalledRoot/Battle Plan.exe" --report-content-authority "$Recording" --report-out "$Scratch/content.json" --config "$Profile" --non-interactive',
     "Current vanilla effective VFS/DLC/profile identity, relocation and precise refusal evidence from the content owner."),
    ("map_ui_reconstruction", "Reconstruction and truth", "existing UI/model lanes",
     "replay_interface/native/replay_app/src/main.rs",
     '& "$InstalledRoot/Battle Plan.exe" --recording "$Recording" --config "$Profile" --non-interactive --dump-frame "$Scratch/frame.bmp" --dump-nth-hour 0',
     "Current recording open/render and faithful scene/UI evidence, including unavailable values and presentation/seek continuity; an image alone does not prove fidelity."),
    ("recording_reach", "Recording and coverage", "vanilla-capture",
     "replay_interface/native/replay_app/src/recording_reach.rs",
     '& "$InstalledRoot/Battle Plan.exe" --report-recording-reach "$Recording" --report-out "$Scratch/reach.json" --report-hours all',
     "Actual current-recording coverage report; exact gaps cannot be inferred from a successful process exit."),
    ("package_bytes", "Release", "release-package",
     "extractor/native/scripts/Test-PortableProductPackageReceipt.ps1",
     'powershell.exe -NoProfile -File extractor/native/scripts/Test-PortableProductPackageReceipt.ps1 -RepoRoot "$RepoRoot" -ReceiptPath "$PackageReceipt"',
     "Current central artifact attestation, capture binding and owning closed-ZIP byte verification."),
    ("installed_desktop", "Release", "release-coverage",
     "extractor/native/scripts/Invoke-InstalledProductSmoke.ps1",
     'powershell.exe -NoProfile -File extractor/native/scripts/Invoke-InstalledProductSmoke.ps1 -InstalledRoot "$InstalledRoot" -PackageReceiptPath "$PackageReceipt" -OutputDirectory "$Scratch/desktop" -DesktopStartup -RequireOffline',
     "Observe both actual desktop windows and cooperative exit under a non-elevated token offline on this machine. This does not prove playback or capture."),
    ("installed_cli", "Release", "release-coverage",
     "extractor/native/scripts/Invoke-InstalledProductSmoke.ps1",
     'powershell.exe -NoProfile -File extractor/native/scripts/Invoke-InstalledProductSmoke.ps1 -InstalledRoot "$InstalledRoot" -PackageReceiptPath "$PackageReceipt" -OutputDirectory "$Scratch/cli" -RequireOffline',
     "Installed recorder/viewer launch and missing-profile refusal with development environment removed, non-elevated token and sampled offline state."),
    ("independent_review", "Release", "orchestrator",
     "AGENTS.md", None,
     "Final-revision independent lane approvals and integration review with raw validation references."),
]


def installed_receipt(path, package_receipt):
    row = {"path": str(path), "status": "invalid_or_incomplete"}
    try:
        receipt = read_json(path)
        if not isinstance(receipt, dict):
            raise ValueError("smoke receipt must be a JSON object")
        if receipt["schema"] != "hoi4.installed-product-smoke.v1" or receipt["status"] != "pass":
            raise ValueError("smoke did not pass")
        if not isinstance(receipt["machine"], str):
            raise ValueError("smoke machine must be a string")
        if receipt["machine"].casefold() != platform.node().casefold():
            raise ValueError("smoke was not recorded on this machine")
        if receipt["non_elevated"] is not True or receipt["offline_observed"] is not True:
            raise ValueError("nonadministrator/offline observation missing")
        if not package_receipt or checked_reference(receipt["package_receipt"]) != reference(package_receipt):
            raise ValueError("smoke is not bound to the supplied package receipt")
        samples = receipt["environment_samples"]
        if not samples or any(s["network_available"] is not False for s in samples):
            raise ValueError("offline samples missing or contradicted")
        cases = receipt["cases"]
        expected = {"recorder_desktop", "viewer_desktop"} if receipt["mode"] == "desktop" else {
            "recorder_help", "viewer_blockers", "viewer_missing_profile"}
        if receipt["mode"] not in ("desktop", "cli") or len(cases) != len(expected) or {c["name"] for c in cases} != expected:
            raise ValueError("missing or duplicate smoke cases")
        for case in cases:
            if case["passed"] is not True:
                raise ValueError("smoke case failed")
            checked_reference(case["stdout"])
            checked_reference(case["stderr"])
            expected_exit = {"recorder_help": 1, "viewer_missing_profile": 2}.get(case["name"], 0)
            if type(case["exit_code"]) is not int or case["exit_code"] != expected_exit:
                raise ValueError("smoke exit contradicts its claimed PASS")
            if receipt["mode"] == "desktop" and (case["window_observed"] is not True or case["close_requested"] is not True):
                raise ValueError("desktop startup/close observation absent")
            diagnostic = {"recorder_help": "usage: collector_record", "viewer_missing_profile": "is not configured"}.get(case["name"])
            if diagnostic and diagnostic not in (Path(case["stdout"]["path"]).read_text(encoding="utf-8-sig") + Path(case["stderr"]["path"]).read_text(encoding="utf-8-sig")):
                raise ValueError("required startup diagnostic absent")
        expected_artifacts = {"Battle Plan Recorder.exe": "recorder_host", "Battle Plan.exe": "replay_app", "collector.dll": "collector_dll"}
        if len(receipt["artifacts"]) != 3 or {Path(a["path"]).name for a in receipt["artifacts"]} != set(expected_artifacts):
            raise ValueError("incomplete installed artifact inventory")
        package = read_json(package_receipt)
        for artifact in receipt["artifacts"]:
            checked_reference(artifact)
            if artifact["sha256"] != package["package"]["input_sha256"][expected_artifacts[Path(artifact["path"]).name]]:
                raise ValueError("installed artifact does not match package binding")
        row.update(status="observed_package_bound", mode=receipt["mode"], receipt=reference(path),
                   limitation="Package currency/admission remains owned by release-package; sampled offline state is not a clean-machine claim.")
    except (OSError, ValueError, KeyError, TypeError) as error:
        row["issue"] = str(error)
    return row


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo-root", type=Path, default=Path(__file__).resolve().parents[3])
    parser.add_argument("--central-run", type=Path, action="append", default=[])
    parser.add_argument("--installed-smoke", type=Path, action="append", default=[])
    parser.add_argument("--package-receipt", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    root = args.repo_root.resolve()
    def git(*argv):
        return subprocess.check_output(["git", *argv], cwd=root, text=True).strip()
    revision, dirty = git("rev-parse", "HEAD"), git("status", "--porcelain=v1")
    rows = []
    for key, section, owner, source, command, missing in CONTRACT_ROWS:
        path = root / source
        rows.append(dict(id=key, requirement=f"PROJECT.md#{section}", owner=owner,
                         source=reference(path) if path.is_file() else {"path": source, "missing": True},
                         status="missing_runtime_proof", command=command, needed=missing))
    central = [central_run(path, revision) for path in args.central_run]
    rows.insert(0, dict(id="central_release", requirement="PROJECT.md#Release", owner="orchestrator",
                       status="current_run_verified" if not dirty and any(r["status"] == "current_run_verified" for r in central) else "missing_current_proof",
                       command='powershell.exe -NoProfile -ExecutionPolicy Bypass -File extractor/native/scripts/Invoke-Validation.ps1 -Mode ReleaseManual -AllowReleaseManual -C07GameRoot "$GameRoot"',
                       needed="Unfiltered central release of both native workspaces after final integration; no direct/filtered/plan-only substitute.", evidence=central))
    smoke = [installed_receipt(path, args.package_receipt) for path in args.installed_smoke]
    for row in rows:
        if row["id"] in ("installed_cli", "installed_desktop"):
            mode = row["id"].removeprefix("installed_")
            row["evidence"] = [r for r in smoke if r.get("mode") == mode or r["status"] == "invalid_or_incomplete"]
            if any(r.get("mode") == mode and r["status"] == "observed_package_bound" for r in smoke):
                row["status"] = "observed_package_bound_currency_pending"
    output = dict(schema="hoi4.release-coverage.v1", revision=revision, source_status=dirty,
                  scope={"campaign": "vanilla", "machine": "current machine only", "performance": "excluded",
                         "clean_machine_claim": False, "modded_or_separate_vm_required": False},
                  complete=False, rows=rows,
                  limitation="Inventory is not release approval. Owning capture/content/container/render/package and review receipts must be verified by integration.")
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(output, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"output": str(args.output.resolve()), "complete": False,
                      "rows": {r["id"]: r["status"] for r in rows}}))
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
