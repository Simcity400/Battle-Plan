"""Receipt rejection and actual lightweight child-process harness checks."""
import importlib.util
import json
import os
from pathlib import Path
import platform
import subprocess
import sys
import tempfile
import unittest
from contextlib import nullcontext

SPEC = importlib.util.spec_from_file_location("coverage", Path(__file__).with_name("Test-ReleaseCoverage.py"))
coverage = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(coverage)


class Receipts(unittest.TestCase):
    def setUp(self):
        self.scratch = tempfile.TemporaryDirectory(prefix="release-coverage-test-")
        self.addCleanup(self.scratch.cleanup)
        self.root = Path(self.scratch.name)
        self.revision = "a" * 40
        self.write("START.json", dict(tip=self.revision, argv=[
            "powershell.exe", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File",
            "C:/repo/extractor/native/scripts/Invoke-Validation.ps1", "-Mode", "ReleaseManual", "-AllowReleaseManual"]))
        snapshot = [dict(path="source.rs", bytes=1, sha256="a" * 64)]
        self.write("SOURCE_BEFORE.json", snapshot)
        self.write("SOURCE_AFTER.json", snapshot)
        (self.root / "stdout.txt").write_text("raw validation output")
        (self.root / "stderr.txt").write_text("")
        self.result = dict(tip=self.revision, exit=0, guard_failure=None,
                           source_files=1, source_unchanged=True, head_unchanged=True, status_porcelain="")
        self.pin_result()

    def write(self, name, value):
        (self.root / name).write_text(json.dumps(value), encoding="utf-8")

    def pin_result(self):
        self.result["files"] = [coverage.reference(self.root / name) for name in
                                ("START.json", "SOURCE_BEFORE.json", "SOURCE_AFTER.json", "stdout.txt", "stderr.txt")]
        self.write("RESULT.json", self.result)

    def status(self, revision=None):
        return coverage.central_run(self.root, revision or self.revision)["status"]

    def test_current_and_historical_are_distinct(self):
        self.assertEqual(self.status(), "current_run_verified")
        self.assertEqual(self.status("b" * 40), "historical_pass")

    def test_missing_result_is_not_pass(self):
        (self.root / "RESULT.json").unlink()
        self.assertEqual(self.status(), "invalid_or_incomplete")

    def test_malformed_root_does_not_abort_other_central_rows(self):
        for value in ([], None, True, 7, "result"):
            with self.subTest(value=value):
                self.write("RESULT.json", value)
                self.assertEqual(self.status(), "invalid_or_incomplete")
        self.pin_result()
        self.assertEqual(self.status(), "current_run_verified")

    def test_nonzero_interrupted_dirty_and_untyped_exit_fail(self):
        for key, value in (("exit", 1), ("exit", False), ("exit", None), ("guard_failure", "timeout"),
                           ("source_unchanged", "true"), ("head_unchanged", False), ("status_porcelain", " M source.rs")):
            with self.subTest(key=key, value=value):
                changed = dict(self.result, **{key: value})
                self.write("RESULT.json", changed)
                self.assertEqual(self.status(), "invalid_or_incomplete")

    def test_changed_raw_log_is_not_pass(self):
        (self.root / "stdout.txt").write_text("changed")
        self.assertEqual(self.status(), "invalid_or_incomplete")

    def test_missing_pinned_log_is_not_pass(self):
        self.result["files"].pop()
        self.write("RESULT.json", self.result)
        self.assertEqual(self.status(), "invalid_or_incomplete")

    def test_plan_filtered_and_direct_build_are_not_release(self):
        original = coverage.read_json(self.root / "START.json")
        for tail in (["-Plan"], ["-Since", "HEAD~1"], ["-Workspace", "Extractor"]):
            with self.subTest(tail=tail):
                self.write("START.json", dict(original, argv=original["argv"] + tail))
                self.pin_result()
                self.assertEqual(self.status(), "invalid_or_incomplete")
        self.write("START.json", dict(original, argv=["cargo", "build", "--release"]))
        self.pin_result()
        self.assertEqual(self.status(), "invalid_or_incomplete")

    def test_snapshot_mismatch_and_empty_fail(self):
        self.write("SOURCE_AFTER.json", [])
        self.pin_result()
        self.assertEqual(self.status(), "invalid_or_incomplete")
        self.write("SOURCE_BEFORE.json", [])
        self.pin_result()
        self.assertEqual(self.status(), "invalid_or_incomplete")

    def smoke(self):
        package = {"package": {"input_sha256": {}}}
        artifacts = []
        for name, role in (("Battle Plan Recorder.exe", "recorder_host"), ("Battle Plan.exe", "replay_app"), ("collector.dll", "collector_dll")):
            (self.root / name).write_text(role)
            artifacts.append(coverage.reference(self.root / name))
            package["package"]["input_sha256"][role] = artifacts[-1]["sha256"]
        self.write("package.json", package)
        cases = []
        for name, code, diagnostic in (("recorder_help", 1, "usage: collector_record"), ("viewer_blockers", 0, "blockers"), ("viewer_missing_profile", 2, "is not configured")):
            (self.root / (name + ".txt")).write_text(diagnostic)
            cases.append(dict(name=name, passed=True, exit_code=code,
                              stdout=coverage.reference(self.root / (name + ".txt")),
                              stderr=coverage.reference(self.root / "stderr.txt")))
        return dict(schema="hoi4.installed-product-smoke.v1", status="pass", mode="cli",
                    machine=platform.node(), non_elevated=True, offline_observed=True,
                    package_receipt=coverage.reference(self.root / "package.json"),
                    environment_samples=[dict(network_available=False)], artifacts=artifacts, cases=cases)

    def smoke_status(self, receipt):
        self.write("SMOKE.json", receipt)
        return coverage.installed_receipt(self.root / "SMOKE.json", self.root / "package.json")["status"]

    def test_smoke_requires_observation_not_defaults(self):
        receipt = self.smoke()
        self.assertEqual(self.smoke_status(receipt), "observed_package_bound")
        for key, value in (("non_elevated", "true"), ("offline_observed", False), ("environment_samples", []),
                           ("machine", "another-machine"), ("cases", []), ("artifacts", [])):
            with self.subTest(key=key):
                self.assertEqual(self.smoke_status(dict(receipt, **{key: value})), "invalid_or_incomplete")

    def test_smoke_malformed_roots_and_machine_types_are_invalid_rows(self):
        receipt = self.smoke()
        for value in ([], None, True, 7, "receipt"):
            with self.subTest(root=value):
                self.assertEqual(self.smoke_status(value), "invalid_or_incomplete")
        for value in ([], {}, None, True, 7):
            with self.subTest(machine=value):
                self.assertEqual(self.smoke_status(dict(receipt, machine=value)), "invalid_or_incomplete")
        self.assertEqual(self.smoke_status(receipt), "observed_package_bound")

    def test_smoke_exit_and_diagnostic_must_support_pass(self):
        receipt = self.smoke()
        receipt["cases"][0]["exit_code"] = 0
        self.assertEqual(self.smoke_status(receipt), "invalid_or_incomplete")
        receipt["cases"][0]["exit_code"] = 1
        path = self.root / "recorder_help.txt"
        path.write_text("unrelated")
        receipt["cases"][0]["stdout"] = coverage.reference(path)
        self.assertEqual(self.smoke_status(receipt), "invalid_or_incomplete")

    def test_desktop_cannot_be_closed_by_cli_receipt(self):
        receipt = self.smoke()
        receipt["mode"] = "desktop"
        self.assertEqual(self.smoke_status(receipt), "invalid_or_incomplete")

    def test_package_replacement_and_installed_mutation_fail(self):
        receipt = self.smoke()
        self.write("package.json", {})
        self.assertEqual(self.smoke_status(receipt), "invalid_or_incomplete")
        receipt = self.smoke()
        (self.root / "collector.dll").write_text("changed")
        self.assertEqual(self.smoke_status(receipt), "invalid_or_incomplete")


@unittest.skipUnless(os.name == "nt", "Windows process harness")
class ChildHarness(unittest.TestCase):
    def test_real_children_exit_output_environment_and_timeout(self):
        # Load production functions via AST, without executing product launches.
        # Fixture children use the Python interpreter; no native build or game.
        retained = os.environ.get("BATTLE_PLAN_COVERAGE_TEST_SCRATCH", os.environ.get("HOI4_COVERAGE_TEST_SCRATCH"))
        scratch = (nullcontext(tempfile.mkdtemp(prefix="installed-smoke-test-", dir=retained))
                   if retained else tempfile.TemporaryDirectory(prefix="installed-smoke-test-"))
        with scratch as directory:
            root = Path(directory)
            child = root / "child.py"
            child.write_text('import os,sys,time\nprint("CHILD", os.environ["PATH"], flush=True)\nprint("ERR",file=sys.stderr,flush=True)\nassert "BATTLE_PLAN_TEST_POISON" not in os.environ\nif sys.argv[1] == "wait": time.sleep(30)\nsys.exit(int(sys.argv[1]))\n')
            harness = Path(__file__).with_name("Invoke-InstalledProductSmoke.ps1").resolve()
            def quote(value):
                return "'" + str(value).replace("'", "''") + "'"
            script = root / "check.ps1"
            script.write_text(r'''
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$tokens = $null; $errors = $null
$ast = [Management.Automation.Language.Parser]::ParseFile(HARNESS, [ref]$tokens, [ref]$errors)
if ($errors.Count) { throw 'parse failed' }
foreach ($name in @('Get-SmokeReference', 'Get-SmokeEnvironment', 'Invoke-SmokeCase')) {
    $functions = @($ast.FindAll({ param($node) $node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq $name }, $true))
    if ($functions.Count -ne 1) { throw 'owning function missing' }
    . ([scriptblock]::Create($functions[0].Extent.Text))
}
$outputRoot = OUTPUT
$RequireOffline = $false; $DesktopStartup = $false; $TimeoutSeconds = 5
$cases = [Collections.Generic.List[object]]::new()
$samples = [Collections.Generic.List[object]]::new()
$env:BATTLE_PLAN_TEST_POISON = 'must not reach child'
$childPath = CHILD
$argv = '"' + $childPath + '" '
Invoke-SmokeCase -Name success -Exe PYTHON -Arguments ($argv + '0') -ExpectedExit 0 -ExpectedText 'CHILD'
if (-not $cases[0].passed -or $cases[0].exit_code -ne 0) { throw 'success lost' }
foreach ($case in @(@('nonzero', '7', 'CHILD'), @('no-diagnostic', '0', 'ABSENT'), @('timeout', 'wait', 'CHILD'))) {
    $rejected = $false
    try { Invoke-SmokeCase -Name $case[0] -Exe PYTHON -Arguments ($argv + $case[1]) -ExpectedExit 0 -ExpectedText $case[2] } catch { $rejected = $true }
    if (-not $rejected -or $cases[$cases.Count - 1].passed) { throw 'bad child was admitted' }
}
if ($cases[1].exit_code -ne 7) { throw 'native exit lost' }
foreach ($case in $cases) {
    if (-not $case.stdout -or -not $case.stderr) { throw 'raw evidence missing' }
    if (-not (Get-Content -LiteralPath $case.stderr.path -Raw).Contains('ERR')) { throw 'stderr lost' }
}
$outText = Get-Content -LiteralPath $cases[0].stdout.path -Raw
if ($outText.Contains('.cargo') -or $outText.Contains('Python')) { throw 'development PATH leaked' }
Write-Output 'PASS actual children: native exits, raw streams, stripped environment, bounded timeout'
'''.replace("HARNESS", quote(harness)).replace("OUTPUT", quote(root))
                              .replace("$childPath = CHILD", "$childPath = " + quote(child)).replace("PYTHON", quote(sys.executable)), encoding="utf-8")
            result = subprocess.run(["powershell.exe", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", str(script)], capture_output=True, text=True, timeout=30)
            print(result.stdout, end="")
            if retained:
                (root / "check.stdout.txt").write_text(result.stdout, encoding="utf-8")
                (root / "check.stderr.txt").write_text(result.stderr, encoding="utf-8")
                print("RAW_CHILD_FIXTURES", root)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)


if __name__ == "__main__":
    unittest.main(verbosity=2)
