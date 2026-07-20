[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)] [string] $EffectiveWinnerIndex,
    [Parameter(Mandatory = $true)] [ValidatePattern('^[0-9a-f]{64}$')] [string] $Ui14EngineVfsSha256,
    [Parameter(Mandatory = $true)] [string] $OutDir,
    [string] $Hoi4Exe,
    [string[]] $LaunchArgument = @('--continuelastsave'),
    [int] $StartupTimeoutSeconds = 300,
    [int] $Attempts = 12,
    [int] $RetrySeconds = 15,
    [int] $Workers = 8,
    [string] $Python = 'python'
)

$ErrorActionPreference = 'Stop'
$expectedExeSha = '870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad'
$repoRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..\..'))
$probe = Join-Path $repoRoot 'extractor\native\lanes\scripted_state_flags_variables_live\a38_scripted_gui_runtime_probe.py'
$verifier = Join-Path $repoRoot 'extractor\native\lanes\scripted_state_flags_variables_live\verify_a38_live_probe.py'
$winner = (Resolve-Path -LiteralPath $EffectiveWinnerIndex).Path
$resolvedOut = [System.IO.Path]::GetFullPath($OutDir)
[System.IO.Directory]::CreateDirectory($resolvedOut) | Out-Null

if ($Attempts -lt 1 -or $RetrySeconds -lt 1 -or $StartupTimeoutSeconds -lt 1 -or $Workers -lt 1) {
    throw 'Attempts, RetrySeconds, StartupTimeoutSeconds, and Workers must be positive.'
}

$process = Get-Process -Name the game -ErrorAction SilentlyContinue
if (@($process).Count -gt 1) {
    throw 'A38 acquisition requires exactly one hoi4.exe process.'
}
if (@($process).Count -eq 0) {
    if ([string]::IsNullOrWhiteSpace($Hoi4Exe)) {
        throw 'hoi4.exe is not running; supply -Hoi4Exe for autonomous continue-save launch.'
    }
    $resolvedExe = (Resolve-Path -LiteralPath $Hoi4Exe).Path
    $actualSha = (Get-FileHash -LiteralPath $resolvedExe -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actualSha -ne $expectedExeSha) {
        throw "hoi4.exe SHA-256 mismatch: $actualSha"
    }
    Start-Process -FilePath $resolvedExe -ArgumentList $LaunchArgument -WorkingDirectory (Split-Path -Parent $resolvedExe) -WindowStyle Hidden | Out-Null
    $deadline = [DateTime]::UtcNow.AddSeconds($StartupTimeoutSeconds)
    do {
        Start-Sleep -Seconds 1
        $process = Get-Process -Name the game -ErrorAction SilentlyContinue
    } while (@($process).Count -eq 0 -and [DateTime]::UtcNow -lt $deadline)
}
if (@($process).Count -ne 1) {
    throw 'Autonomous launch did not produce exactly one hoi4.exe before the deadline.'
}

$attemptRows = [System.Collections.Generic.List[object]]::new()
for ($attempt = 1; $attempt -le $Attempts; $attempt++) {
    $stem = 'attempt_{0:d2}' -f $attempt
    $raw = Join-Path $resolvedOut "$stem.raw.json"
    $verification = Join-Path $resolvedOut "$stem.verification.json"
    $probeArgs = @(
        $probe,
        '--out', $raw,
        '--workers', $Workers,
        '--effective-winner-index', $winner,
        '--ui14-engine-vfs-sha256', $Ui14EngineVfsSha256
    )
    & $Python @probeArgs
    $probeExit = $LASTEXITCODE
    $accepted = $false
    if ($probeExit -eq 0 -and (Test-Path -LiteralPath $raw)) {
        & $Python $verifier $raw --out $verification
        $accepted = $LASTEXITCODE -eq 0
    }
    $attemptRows.Add([ordered]@{
        attempt = $attempt
        probe_exit_code = $probeExit
        raw = if (Test-Path -LiteralPath $raw) { $raw } else { $null }
        verification = if (Test-Path -LiteralPath $verification) { $verification } else { $null }
        accepted = $accepted
    })
    if ($accepted) {
        break
    }
    if ($attempt -lt $Attempts) {
        Start-Sleep -Seconds $RetrySeconds
    }
}

$acceptedRows = @($attemptRows | Where-Object accepted)
$bundle = [ordered]@{
    schema = 'extractor.a38_autonomous_acquisition.v1'
    executable_sha256 = $expectedExeSha
    ui14_engine_vfs_sha256 = $Ui14EngineVfsSha256
    effective_winner_index = $winner
    process_id = @($process)[0].Id
    attempts = @($attemptRows)
    accepted_count = $acceptedRows.Count
    status = if ($acceptedRows.Count -eq 1) { 'ACCEPTED_RAW_CAPTURE' } else { 'EXHAUSTED_WITHOUT_ACCEPTED_CAPTURE' }
    production_admission_ready = $false
    production_admission_blockers = @(
        'complete_engine_result_frame_not_compiled',
        'in_process_h4ct_winner_authorization_required'
    )
}
$bundlePath = Join-Path $resolvedOut 'a38_acquisition_bundle.json'
$bundleJson = ($bundle | ConvertTo-Json -Depth 8) + [Environment]::NewLine
[System.IO.File]::WriteAllText($bundlePath, $bundleJson, [System.Text.UTF8Encoding]::new($false))
if ($acceptedRows.Count -ne 1) {
    throw "A38 autonomous acquisition exhausted $Attempts attempts; inspect $bundlePath"
}
Write-Output "A38 raw acquisition accepted: $bundlePath"
