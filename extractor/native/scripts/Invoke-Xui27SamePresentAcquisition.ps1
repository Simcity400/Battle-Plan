[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Hoi4Exe,
    [Parameter(Mandatory = $true)][string]$PdxUserDir,
    [Parameter(Mandatory = $true)][string]$PreparedProductModsetDir,
    [Parameter(Mandatory = $true)][string]$OutDir,
    [string]$PrebuiltArtifactAttestationPath = '',
    [int]$DurationSeconds = 360,
    [int]$SampleMilliseconds = 100,
    [int]$GameLoadTimeoutSeconds = 240,
    [string]$Python = 'python'
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$repo = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..\..'))
$captureHarness = Join-Path $repo 'replay_interface\re_evidence\rendering_laws\capture_harness\capture_harness.py'
$cap6Runner = Join-Path $PSScriptRoot 'Invoke-Cap6FinalD3dEvidence.ps1'
$verifier = Join-Path $repo 'replay_interface\re_evidence\rendering_laws\XUI27\verify_xui27_same_present_acquisition.py'
$resolvedOut = [IO.Path]::GetFullPath($OutDir)
$pythonExe = (Get-Command $Python -CommandType Application -ErrorAction Stop).Source

foreach ($required in @($Hoi4Exe, $PdxUserDir, $PreparedProductModsetDir, $captureHarness, $cap6Runner, $verifier)) {
    if (-not (Test-Path -LiteralPath $required)) { throw "Required XUI27 input is absent: $required" }
}
if (Test-Path -LiteralPath $resolvedOut) {
    throw "XUI27 OutDir must be absent so the acquisition is immutable: $resolvedOut"
}

$staging = "$resolvedOut.xui27-context-$([guid]::NewGuid().ToString('N'))"
if (Test-Path -LiteralPath $staging) { throw "XUI27 context staging collision: $staging" }
[IO.Directory]::CreateDirectory($staging) | Out-Null
$contextMoved = $false
try {
    $preflight = Join-Path $staging 'xui27_capture_harness_preflight.json'
    & $pythonExe -B $captureHarness preflight `
        --packets XUI27 `
        --mode launch `
        --phase synchronized_capture `
        --coordinator direct_api_present `
        --output $preflight
    if ($LASTEXITCODE -ne 0) { throw 'XUI27 capture-harness preflight failed.' }

    & $pythonExe -B $captureHarness snapshot-context `
        --destination $staging `
        --packets XUI27
    if ($LASTEXITCODE -ne 0) { throw 'XUI27 capture-harness context snapshot failed.' }

    $cap6Arguments = @{
        Hoi4Exe = $Hoi4Exe
        PdxUserDir = $PdxUserDir
        PreparedProductModsetDir = $PreparedProductModsetDir
        OutDir = $resolvedOut
        DurationSeconds = $DurationSeconds
        SampleMilliseconds = $SampleMilliseconds
        GameLoadTimeoutSeconds = $GameLoadTimeoutSeconds
        Python = $pythonExe
    }
    if (-not [string]::IsNullOrWhiteSpace($PrebuiltArtifactAttestationPath)) {
        $cap6Arguments.PrebuiltArtifactAttestationPath = $PrebuiltArtifactAttestationPath
    }
    & $cap6Runner @cap6Arguments
    if ($LASTEXITCODE -ne 0) { throw 'CAP6 did not complete the XUI27 selected-Present transaction.' }

    $contextRoot = Join-Path $resolvedOut 'xui27_capture_context'
    if (Test-Path -LiteralPath $contextRoot) { throw 'XUI27 capture context destination already exists.' }
    Move-Item -LiteralPath $staging -Destination $contextRoot
    $contextMoved = $true

    $contextManifest = Join-Path $contextRoot 'evidence_manifest.json'
    & $pythonExe -B $captureHarness package --source $contextRoot --manifest $contextManifest
    if ($LASTEXITCODE -ne 0) { throw 'XUI27 capture-context packaging failed.' }

    $sessionManifest = Join-Path $resolvedOut 'native_session_manifest.json'
    if (-not (Test-Path -LiteralPath $sessionManifest -PathType Leaf)) {
        throw 'XUI27 CAP6 transaction produced no native session manifest.'
    }
    $session = Get-Content -LiteralPath $sessionManifest -Raw | ConvertFrom-Json
    if ([string]::IsNullOrWhiteSpace([string]$session.replay_file)) {
        throw 'XUI27 native session did not publish a replay filename.'
    }
    $recording = Join-Path $resolvedOut ([string]$session.replay_file)
    $worldFrame = Join-Path $resolvedOut 'xui27_joined_worldframe.snapshot.json'
    $admission = Join-Path $resolvedOut 'xui27_same_present_admission.json'
    & $pythonExe -B $verifier `
        --capture-root $resolvedOut `
        --capture-context (Join-Path $contextRoot 'capture_context.json') `
        --capture-context-manifest $contextManifest `
        --cap6-verification (Join-Path $resolvedOut 'cap6_final_d3d_verification.json') `
        --session-manifest $sessionManifest `
        --recording $recording `
        --worldframe-out $worldFrame `
        --out $admission
    if ($LASTEXITCODE -ne 0) {
        throw 'XUI27 same-session Present/WorldFrame admission failed closed.'
    }
    Write-Output "XUI27 authenticated Present/WorldFrame pair admitted: $admission"
} finally {
    # If CAP6 created a failed immutable attempt, keep the prelaunch context
    # beside it. Only a context-only failure before launch is disposable.
    if (-not $contextMoved -and (Test-Path -LiteralPath $staging -PathType Container)) {
        if (Test-Path -LiteralPath $resolvedOut -PathType Container) {
            $failedContext = Join-Path $resolvedOut 'xui27_capture_context_pre_admission'
            if (-not (Test-Path -LiteralPath $failedContext)) {
                Move-Item -LiteralPath $staging -Destination $failedContext
            }
        } else {
            Remove-Item -LiteralPath $staging -Recurse -Force
        }
    }
}
