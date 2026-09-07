"""Run the owning publication driver with controlled authority/builder fixtures.

No Cargo, game, capture, or native product process is launched. Fixture guards
prove routing and failure propagation; real PE/document admission has its own
owning suites and is not replaced by these fixtures.
"""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

SCRIPTS = Path(__file__).resolve().parent

ATTESTATION = r'''
function Test-ProductionArtifactAttestation {
    param($RepoRoot, $ReceiptPath)
    $root = Join-Path $RepoRoot '.cargo-target/release'
    Add-Content (Join-Path $root 'calls.log') 'attestation-current-source-and-bytes'
    if (Test-Path (Join-Path $root 'reject-attestation')) { throw 'fixture current source or candidate bytes mismatch' }
    $record = Get-Content -LiteralPath $ReceiptPath -Raw | ConvertFrom-Json
    foreach ($role in @('recorder_host', 'replay_app', 'collector_dll')) {
        if ((Get-FileHash -LiteralPath $record.artifacts.$role -Algorithm SHA256).Hash.ToLowerInvariant() -cne $record.artifact_sha256.$role) { throw 'fixture candidate byte mismatch' }
    }
    $record | Add-Member -NotePropertyName receipt_path -NotePropertyValue $ReceiptPath
    $record | Add-Member -NotePropertyName receipt_sha256 -NotePropertyValue ((Get-FileHash -LiteralPath $ReceiptPath).Hash.ToLowerInvariant())
    return $record
}
function Write-ProductionArtifactAttestation { throw 'publication minted a new attestation' }
function Clear-ProductionArtifactOutputs { throw 'publication cleared captured candidates' }
'''
PROMOTION = r'''
function Get-ProductionCapturePromotionEvidence {
    param($RepoRoot, $CaptureReceiptPath, $CurrentArtifactAttestationPath)
    $root = Join-Path $RepoRoot '.cargo-target/release'
    Add-Content (Join-Path $root 'calls.log') 'capture-current-candidate-binding'
    $record = Get-Content -LiteralPath $CaptureReceiptPath -Raw | ConvertFrom-Json
    if ($record.candidate -cne (Get-FileHash -LiteralPath $CurrentArtifactAttestationPath).Hash.ToLowerInvariant()) { throw 'fixture capture candidate identity mismatch' }
    if (Test-Path (Join-Path $root 'reject-capture')) { throw 'fixture capture promotion refused' }
    return $record
}
'''
BUILDER = r'''
param($RecorderExe,$ReplayExe,$CollectorDll,$OutputZip,$Version,$ProductionCaptureReceiptPath,$ProductionArtifactAttestationPath)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $OutputZip
Add-Content (Join-Path $root 'calls.log') 'builder'
if (Test-Path (Join-Path $root 'builder-failure')) { exit 7 }
if (Test-Path (Join-Path $root 'builder-output-race')) {
    [IO.File]::WriteAllText($OutputZip, 'OTHER-INVOCATION-ZIP')
    exit 7
}
if (Test-Path -LiteralPath $OutputZip) { throw 'fixture builder refuses overwrite' }
$attestation = Get-Content -LiteralPath $ProductionArtifactAttestationPath -Raw | ConvertFrom-Json
foreach ($pair in @(@('recorder_host',$RecorderExe),@('replay_app',$ReplayExe),@('collector_dll',$CollectorDll))) {
    if ($pair[1] -cne $attestation.artifacts.($pair[0])) { throw 'substituted candidate path' }
}
[IO.File]::WriteAllText($OutputZip, 'FIXTURE-PACKAGE')
[ordered]@{
 schema='hoi4-portable-package.v1'; admissible_for_production=$true
 version=$Version; path=$OutputZip; sha256=(Get-FileHash -LiteralPath $OutputZip).Hash.ToLowerInvariant()
 input_sha256=$attestation.artifact_sha256
 interaction_closure=@{source_capability=@{exact_pinned_hook_count=1};policy_inputs=@{collector_cargo_sha256=('4'*64);passive_ready_contract_sha256=('5'*64)}}
} | ConvertTo-Json -Depth 8
'''
VERIFIER = r'''
param($RepoRoot,$ReceiptPath,[switch]$AllowStagedReceipt)
$ErrorActionPreference = 'Stop'
$root = Join-Path $RepoRoot '.cargo-target/release'
$phase = if ($AllowStagedReceipt) { 'staged' } else { 'published' }
Add-Content (Join-Path $root 'calls.log') $phase
$receipt = Get-Content -LiteralPath $ReceiptPath -Raw | ConvertFrom-Json
$attestationPath = Join-Path $root 'hoi4-production-prebuilt-artifacts.json'
$attestation = Get-Content -LiteralPath $attestationPath -Raw | ConvertFrom-Json
if ($receipt.schema -cne 'battle-plan.central_portable_package.v1' -or $receipt.status -cne 'PASS_CENTRAL_RELEASE_PORTABLE_PACKAGE' -or -not $receipt.successful_exit) { throw 'outer authority missing' }
if ($receipt.artifact_attestation.path -cne $attestationPath -or $receipt.artifact_attestation.sha256 -cne (Get-FileHash -LiteralPath $attestationPath).Hash.ToLowerInvariant()) { throw 'outer receipt replaced original attestation' }
if ($receipt.source_input_sha256 -cne $attestation.source_input_sha256 -or $receipt.build_environment.sha256 -cne $attestation.build_environment.sha256) { throw 'outer receipt forged source/environment' }
if (($receipt.package.input_sha256 | ConvertTo-Json -Compress) -cne ($attestation.artifact_sha256 | ConvertTo-Json -Compress)) { throw 'outer receipt substituted artifact hashes' }
if (Test-Path (Join-Path $root "$phase-failure")) { throw "fixture $phase verifier failure" }
if ($AllowStagedReceipt -and (Test-Path (Join-Path $root 'receipt-output-race'))) {
    [IO.File]::WriteAllText((Join-Path $root 'hoi4-portable-package-receipt.json'), 'OTHER-INVOCATION-RECEIPT')
}
'PASS owning outer-publication fixture'
'''


@unittest.skipUnless(os.name == 'nt', 'Windows PowerShell publication')
class Publication(unittest.TestCase):
    def setUp(self):
        self.scratch = tempfile.TemporaryDirectory(prefix='prebuilt-publication-')
        self.addCleanup(self.scratch.cleanup)
        self.root = Path(self.scratch.name)
        self.scripts = self.root / 'extractor/native/scripts'
        self.scripts.mkdir(parents=True)
        self.release = self.root / '.cargo-target/release'
        self.release.mkdir(parents=True)
        self.driver = self.scripts / 'Invoke-ReleaseManual.ps1'
        shutil.copyfile(SCRIPTS / self.driver.name, self.driver)
        self.write(self.scripts / 'Invoke-Validation.ps1', "throw 'publication invoked central build gate'\n")
        helper = self.root / 'portable_replay/native/scripts/ProductionArtifact.Attestation.ps1'
        self.write(helper, ATTESTATION)
        self.write(self.scripts / 'ProductionCapture.Promotion.ps1', PROMOTION)
        self.write(self.scripts / 'New-PortableProductPackage.ps1', BUILDER)
        self.write(self.scripts / 'Test-PortableProductPackageReceipt.ps1', VERIFIER)
        self.manifest = self.root / 'extractor/native/recorder_host/Cargo.toml'
        self.write(self.manifest, '[package]\nname = "recorder_host"\nversion = "0.1.0"\n[features]\ndefault = []\n')
        names = {'recorder_host':'collector_record.exe','replay_app':'replay_app.exe','collector_dll':'collector.dll'}
        artifacts = {}
        hashes = {}
        for role, name in names.items():
            path = self.release / name
            self.write(path, 'CAPTURED-' + role)
            artifacts[role] = str(path)
            hashes[role] = self.hash(path)
        self.attestation = self.release / 'hoi4-production-prebuilt-artifacts.json'
        self.write(self.attestation, json.dumps(dict(artifacts=artifacts, artifact_sha256=hashes,
                   source_input_sha256='a'*64, build_environment={'sha256':'b'*64}, release_runtime_linkage={'c_runtime':'static'})))
        self.capture = self.root / 'capture_validation_receipt.json'
        self.write(self.capture, json.dumps({'candidate':self.hash(self.attestation)}))
        self.candidates = {Path(p):self.hash(Path(p)) for p in artifacts.values()}
        self.candidates[self.attestation] = self.hash(self.attestation)
        self.zip = self.release / 'battle-plan-0.1.0-windows-x86_64.zip'
        self.outer = self.release / 'hoi4-portable-package-receipt.json'

    def write(self, path, text):
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding='utf-8')

    def hash(self, path):
        return hashlib.sha256(path.read_bytes()).hexdigest()

    def run_driver(self, *extra, capture=True):
        argv = ['powershell.exe','-NoProfile','-ExecutionPolicy','Bypass','-File',str(self.driver),'-AllowReleaseManual','-PackagePrebuilt']
        if capture:
            argv += ['-ProductionCaptureReceiptPath',str(self.capture)]
        result = subprocess.run(argv + list(extra), capture_output=True, text=True, timeout=30)
        for path, digest in self.candidates.items():
            self.assertTrue(path.exists(), str(path))
            self.assertEqual(self.hash(path), digest, str(path))
        self.assertFalse(list(self.release.glob('*.candidate')), result.stdout + result.stderr)
        return result

    def assert_refused(self, marker=None, expected=None, **kwargs):
        if marker:
            self.write(self.release / marker, 'reject')
        result = self.run_driver(**kwargs)
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        if expected:
            self.assertIn(expected, result.stdout + result.stderr)
        self.assertFalse(self.zip.exists())
        self.assertFalse(self.outer.exists())

    def test_publish_outer_from_exact_captured_candidates(self):
        result = self.run_driver()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        receipt = json.loads(self.outer.read_text())
        self.assertEqual(receipt['package']['version'], '0.1.0')
        self.assertEqual(receipt['artifact_attestation']['sha256'], self.hash(self.attestation))
        self.assertEqual(receipt['source_input_sha256'], 'a'*64)
        self.assertEqual((self.release/'calls.log').read_text().splitlines(),
                         ['attestation-current-source-and-bytes','capture-current-candidate-binding','builder','staged','published'])

    def test_missing_capture_argument(self):
        self.assert_refused(capture=False, expected='requires a production capture receipt')

    def test_missing_capture_file(self):
        self.capture.unlink()
        self.assert_refused()

    def test_stale_source_or_attestation_refusal(self):
        self.assert_refused('reject-attestation', 'current source or candidate bytes mismatch')
        self.assertEqual((self.release/'calls.log').read_text().splitlines(), ['attestation-current-source-and-bytes'])

    def test_changed_candidate_is_refused_without_replacement(self):
        path = self.release / 'collector.dll'
        self.write(path, 'CHANGED-CANDIDATE')
        self.candidates[path] = self.hash(path)
        self.assert_refused(expected='candidate byte mismatch')

    def test_central_dispatch_forwards_prebuilt_and_rejects_filtering(self):
        dispatcher = self.scripts / 'Invoke-Validation.ps1'
        shutil.copyfile(SCRIPTS / dispatcher.name, dispatcher)
        self.write(self.driver, "param([switch]$AllowReleaseManual,[switch]$PackagePrebuilt,[switch]$Plan,$ProductionCaptureReceiptPath)\n"
                   "if (-not $AllowReleaseManual -or -not $PackagePrebuilt -or -not $Plan -or -not $ProductionCaptureReceiptPath) { throw 'dispatch lost publication arguments' }\n"
                   "$global:LASTEXITCODE=0; 'PASS publication dispatch'\n")
        argv = ['powershell.exe','-NoProfile','-ExecutionPolicy','Bypass','-File',str(dispatcher),
                '-Mode','ReleaseManual','-AllowReleaseManual','-PackagePrebuilt','-Plan',
                '-ProductionCaptureReceiptPath',str(self.capture)]
        result = subprocess.run(argv,capture_output=True,text=True,timeout=15)
        self.assertEqual(result.returncode,0,result.stdout+result.stderr)
        self.assertIn('PASS publication dispatch',result.stdout)
        for extra in (['-Since','HEAD'], ['-AllValidators']):
            result = subprocess.run(argv+extra,capture_output=True,text=True,timeout=15)
            self.assertNotEqual(result.returncode,0)
            self.assertIn('not publication arguments',result.stdout+result.stderr)

    def test_capture_identity_mismatch(self):
        self.write(self.capture, json.dumps({'candidate':'0'*64}))
        self.assert_refused(expected='capture candidate identity mismatch')

    def test_capture_promotion_failure(self):
        self.assert_refused('reject-capture', 'capture promotion refused')

    def test_existing_outputs_are_preserved(self):
        for path in (self.zip, self.outer):
            with self.subTest(path=path.name):
                self.write(path, 'PREVIOUS-VALID-OUTPUT')
                result = self.run_driver()
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(path.read_text(), 'PREVIOUS-VALID-OUTPUT')
                path.unlink()

    def test_unknown_or_inherited_manifest_version_rejected(self):
        for text in ('[package]\nname="other"\nversion="0.1.0"\n',
                     '[package]\nname="recorder_host"\nversion.workspace=true\n'):
            with self.subTest(text=text):
                self.write(self.manifest, text)
                self.assert_refused(expected='exact literal recorder_host package version')

    def test_builder_failure_preserves_candidate_authority(self):
        self.assert_refused('builder-failure', 'Package builder failed with exit code 7')

    def test_staged_verifier_failure_removes_only_new_package(self):
        self.assert_refused('staged-failure', 'staged package receipt verification failed')

    def test_final_verifier_failure_removes_only_new_publication(self):
        self.assert_refused('published-failure', 'published package receipt verification failed')

    def test_output_races_do_not_delete_other_invocation_files(self):
        for marker, path, content in (('builder-output-race', self.zip, 'OTHER-INVOCATION-ZIP'),
                                      ('receipt-output-race', self.outer, 'OTHER-INVOCATION-RECEIPT')):
            with self.subTest(marker=marker):
                self.write(self.release/marker, 'race')
                result = self.run_driver()
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(path.read_text(), content)
                path.unlink()
                (self.release/marker).unlink()

    def test_plan_does_not_read_or_mutate_candidate_authority(self):
        self.capture.unlink()
        result = self.run_driver('-Plan')
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn('without building or replacing candidate authority', result.stdout)
        self.assertFalse((self.release/'calls.log').exists())
        self.assertFalse(self.outer.exists())

    def test_build_phase_argument_rejected(self):
        result = self.run_driver('-C07GameRoot', str(self.root))
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('build-phase argument', result.stdout + result.stderr)


if __name__ == '__main__':
    result = unittest.TextTestRunner(verbosity=2).run(unittest.defaultTestLoader.loadTestsFromTestCase(Publication))
    passed = result.wasSuccessful() and result.testsRun >= 16 and not result.skipped
    print(json.dumps({'schema':'hoi4-prebuilt-package-publication-tests.v1','status':'pass' if passed else 'fail','cases':result.testsRun}))
    raise SystemExit(0 if passed else 1)
