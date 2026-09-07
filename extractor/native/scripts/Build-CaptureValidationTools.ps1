[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$OutputDirectory,
    [switch]$Build,
    [ValidateRange(1, 7200)][int]$TimeoutSeconds = 1800
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
function Invoke-CaptureToolsInRepository {
    param([string]$RepoPath, [scriptblock]$Action)
    # ProcessStartInfo inherits the OS directory, not PowerShell's location.
    $previousProcessDirectory = [Environment]::CurrentDirectory
    Push-Location -LiteralPath $RepoPath
    try {
        [Environment]::CurrentDirectory = $RepoPath
        & $Action
    } finally {
        [Environment]::CurrentDirectory = $previousProcessDirectory
        Pop-Location
    }
}
$repo = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../../..'))
. (Join-Path $repo 'portable_replay/native/scripts/ProductionArtifact.Attestation.ps1')
. (Join-Path $PSScriptRoot 'ProductionCapture.Validation.ps1')
. (Join-Path $PSScriptRoot 'CaptureValidation.Tools.ps1')
$plans = @(Get-CaptureValidationToolPlan $repo)
if (-not $Build) {
    # A plan is not a receipt and executes neither Cargo nor the tools.
    $plans | Select-Object Role, Package, Target, Arguments | ConvertTo-Json -Depth 5
    return
}
$out = [IO.Path]::GetFullPath($OutputDirectory)
if ($out -ieq $repo -or $out.StartsWith($repo.TrimEnd('\') + '\', [StringComparison]::OrdinalIgnoreCase)) {
    throw 'Validation build evidence must be outside the repository.'
}
if (Test-Path -LiteralPath $out) { throw 'Use a new validation-tools output directory.' }
$null = New-Item -ItemType Directory -Path $out
$previousTarget = $env:CARGO_TARGET_DIR
$previousWrapper = $env:RUSTC_WRAPPER
$previousCache = $env:SCCACHE_DIR
$env:CARGO_TARGET_DIR = Join-Path $repo '.cargo-target'
$authority = $null
Invoke-CaptureToolsInRepository -RepoPath $repo -Action {
try {
    $env:RUSTC_WRAPPER = (Get-Command sccache -CommandType Application -ErrorAction Stop | Select-Object -First 1).Source
    $env:SCCACHE_DIR = 'C:\sccache_cache'
    $before = Get-ProductionRustInputDigest $repo
    $environmentBefore = Get-ProductionBuildEnvironment
    $builds = @()
    $cargo = (Get-Command cargo -CommandType Application | Select-Object -First 1).Source
    foreach ($plan in $plans) {
        # These exact three diagnostic outputs belong to this worktree. Never
        # clear shipping executables, another worktree, or a shared cache.
        if (Test-Path -LiteralPath $plan.Executable) { Remove-Item -LiteralPath $plan.Executable -Force }
        $result = Invoke-IndependentCaptureGate -Name $plan.Role -FilePath $cargo -Arguments $plan.Arguments `
            -LogDir $out -TimeoutSeconds $TimeoutSeconds -CancellationGraceSeconds 5
        [IO.File]::WriteAllText((Join-Path $out ($plan.Role + '.build-result.json')), ($result | ConvertTo-Json -Depth 6))
        if (-not $result.passed) { throw "Validation tool build failed: $($plan.Role); see $out" }
        if ((Get-CaptureToolFileReference $cargo).sha256 -cne $result.executable_sha256) { throw 'Cargo changed during the validation build.' }
        Assert-CaptureToolCompilerOutput $plan $result.stdout
        $stdout = Get-CaptureToolFileReference $result.stdout
        $stderr = Get-CaptureToolFileReference $result.stderr
        $stdout.path = [IO.Path]::GetFileName($stdout.path)
        $stderr.path = [IO.Path]::GetFileName($stderr.path)
        $builds += [ordered]@{ role = $plan.Role; arguments = $plan.Arguments; working_directory = $repo
            exit_code = $result.exit_code; started = $result.started; timed_out = $result.timed_out; problem = $result.problem
            cargo = Get-CaptureToolFileReference $cargo
            stdout = $stdout; stderr = $stderr; artifact = Get-CaptureToolFileReference $plan.Executable }
    }
    $after = Get-ProductionRustInputDigest $repo
    $environmentAfter = Get-ProductionBuildEnvironment
    if (($before | ConvertTo-Json -Compress) -cne ($after | ConvertTo-Json -Compress)) { throw 'Source changed during validation tool builds.' }
    if ($environmentBefore.sha256 -cne $environmentAfter.sha256) { throw 'Environment changed during validation tool builds.' }
    $receipt = [ordered]@{ schema = 'hoi4.capture-validation-tools.v1'; authority = 'diagnostic_tools_not_shipping_artifacts'
        source_before = $before; source_after = $after; build_environment = $environmentAfter; builds = $builds }
    $pending = Join-Path $out 'capture-validation-tools.pending.json'
    $published = Join-Path $out 'capture-validation-tools.json'
    [IO.File]::WriteAllText($pending, ($receipt | ConvertTo-Json -Depth 12))
    $authority = Open-CaptureValidationTools -RepoRoot $repo -ReceiptPath $pending -ExpectedSourceInputSha256 $after.sha256
    Close-CaptureValidationTools $authority
    $authority = $null
    [IO.File]::Move($pending, $published)
    Write-Output ($receipt | ConvertTo-Json -Depth 12)
} finally {
    Close-CaptureValidationTools $authority
    $env:CARGO_TARGET_DIR = $previousTarget
    $env:RUSTC_WRAPPER = $previousWrapper
    $env:SCCACHE_DIR = $previousCache
}
}
