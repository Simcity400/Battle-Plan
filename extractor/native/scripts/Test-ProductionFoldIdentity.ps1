# Offline integration check: execute the owning artifact validator and intercept
# only its external verifier boundary. No verifier binary or game is launched.
[CmdletBinding()]
param()
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
. (Join-Path $PSScriptRoot 'ProductionCapture.Validation.ps1')
$calls = [Collections.Generic.List[object]]::new()
function Invoke-IndependentCaptureGate {
    param($Name,$FilePath,$Arguments,$LogDir,$TimeoutSeconds,$CancellationGraceSeconds)
    $calls.Add([pscustomobject]@{name=$Name; path=$FilePath; arguments=@($Arguments)})
    return [pscustomobject]@{ name=$Name; passed=$true; exit_code=0 }
}
$scratchParent = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\')
$scratch = Join-Path $scratchParent ('production_fold_identity_' + [guid]::NewGuid().ToString('N'))
if (-not $scratch.StartsWith($scratchParent+'\',[StringComparison]::OrdinalIgnoreCase)) { throw 'test scratch escaped parent' }
[void](New-Item -ItemType Directory -Path $scratch)
try {
    $registry = Join-Path $PSScriptRoot '..\domain_registry.json'
    if (-not (Test-Path -LiteralPath $registry -PathType Leaf)) { throw 'owning registry is missing' }
    $invoke = @{RunDir=$scratch; RegistryPath=$registry; SpeedProofPath=(Join-Path $scratch 'missing-speed.json')}
    foreach ($name in @('CompactGateExe','FoldCheckExe','ContainerVerifyExe','PythonExe','StaticBundleScript','StaticBundleDir','GameExePath','ModStatePath','DlcStatePath','LoadOrderPath','SourceInventoryPath','LiveResolutionReportPath','CapturePlanPath','SpeedRootGatePath','CollectorControlPath','SpeedMonitorPath','SpeedStreamVerifier','SpeedEvidenceTranscriptPath','SpeedEvidenceNonceLedgerRoot','SpeedEvidenceVerifier','ExpectedSpeedEvidenceVerifierSha256','ExpectedSpeedEvidenceProbeSha256','ExpectedSpeedStaticDisasmSha256')) {
        $invoke[$name] = Join-Path $scratch ('unavailable-' + $name)
    }
    foreach ($case in @(
        @{label='identity_v1'; json='"identity_v1"'; expected='identity-v1'},
        @{label='legacy'; json='"legacy"'; expected='legacy'},
        @{label='missing'; json=$null; expected=$null},
        @{label='null'; json='null'; expected=$null},
        @{label='array'; json='["identity_v1"]'; expected=$null},
        @{label='unknown'; json='"identity_v2"'; expected=$null}
    )) {
        $manifest = New-TestManifest
        $manifest.hourly_requirement['requested_game_hours'] = 1
        if ($null -ne $case.json) {
            # Preserve JSON arrays as property values; pipeline unwrapping must
            # not manufacture a scalar mode from a one-element array.
            $decoded = ('{"mode":' + $case.json + '}') | ConvertFrom-Json
            $manifest['compact_stream_identity_mode'] = $decoded.mode
        }
        $manifest | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath (Join-Path $scratch 'native_session_manifest.json') -Encoding UTF8
        $calls.Clear()
        $report = Test-ProductionCaptureArtifacts @invoke
        $fold = @($calls | Where-Object { $_.name -ceq 'fold_check' })
        $modeProblem = @($report.problems) -contains 'compact_stream_identity_mode_missing_or_invalid'
        if ($null -eq $case.expected) {
            if ($fold.Count -ne 0 -or -not $modeProblem -or $report.ok) { throw "invalid mode $($case.label) reached a verifier or lacked named refusal" }
        } else {
            if ($fold.Count -ne 1 -or $modeProblem) { throw "explicit mode $($case.label) did not invoke fold exactly once" }
            $expected = @('hash',(Join-Path $scratch 'collector_stream.bin'),'--expected-profile','production','--identity-mode',$case.expected,'--out',(Join-Path $scratch 'independent_validation\fold_check.report.json'))
            if (($fold[0].arguments -join '|') -cne ($expected -join '|')) { throw "wrong actual fold invocation for $($case.label)" }
            if ($report.ok) { throw 'test fixture lacks real capture evidence and must remain failed' }
        }
        if (@($calls | Where-Object { $_.name -ceq 'container_verify' }).Count -ne 1) { throw 'other independent verification was bypassed' }
        Write-Output "PASS actual artifact-validator fold boundary: $($case.label)"
    }
} finally {
    $resolved = [IO.Path]::GetFullPath($scratch)
    if (-not $resolved.StartsWith($scratchParent+'\',[StringComparison]::OrdinalIgnoreCase)) { throw 'refuse test cleanup outside scratch parent' }
    Remove-Item -LiteralPath $resolved -Recurse -Force
}
