[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)] [string] $HarnessExe,
    [Parameter(Mandatory = $true)] [string] $Hoi4Exe,
    [Parameter(Mandatory = $true)] [string] $CollectorDll,
    [Parameter(Mandatory = $true)] [string] $OutDir,
    [string] $ProfilePath,
    [string[]] $HarnessArgument = @(),
    [int] $ProcessTimeoutSeconds = 420,
    [int] $HarnessExitTimeoutSeconds = 180,
    [string] $Python = 'python'
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
. (Join-Path $PSScriptRoot 'Hoi4ClientDriving.OptIn.ps1')
Assert-Hoi4ClientDrivingOptIn -Caller $MyInvocation.MyCommand.Path
$expectedExeSha256 = '870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad'

function Write-CreateNewUtf8([string]$Path, [string]$Text) {
    $full = [IO.Path]::GetFullPath($Path)
    if ([IO.File]::Exists($full)) { throw "Refusing to replace immutable A25/A26 artifact: $full" }
    [IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($full)) | Out-Null
    $temporary = [IO.Path]::Combine([IO.Path]::GetDirectoryName($full), ([IO.Path]::GetRandomFileName() + '.tmp'))
    try {
        [IO.File]::WriteAllText($temporary, $Text, [Text.UTF8Encoding]::new($false))
        [IO.File]::Move($temporary, $full)
    } finally {
        if ([IO.File]::Exists($temporary)) { [IO.File]::Delete($temporary) }
    }
}
function Quote-NativeArgument([string]$Value) {
    if ($Value -notmatch '[\s"]') { return $Value }
    return '"' + ($Value -replace '(\\*)"', '$1$1\"' -replace '(\\+)$', '$1$1') + '"'
}

$repo = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..\..'))
if ([string]::IsNullOrWhiteSpace($ProfilePath)) {
    $ProfilePath = Join-Path $repo 'extractor\documents\dossiers\A25_A26_AUTONOMOUS_TOOLTIP_GLYPH_ACTIONS_1920X1080_2560X1440.json'
}
foreach ($path in @($HarnessExe, $Hoi4Exe, $CollectorDll, $ProfilePath)) {
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "Required file does not exist: $path" }
}
$resolvedHarness = (Resolve-Path -LiteralPath $HarnessExe).Path
$resolvedExe = (Resolve-Path -LiteralPath $Hoi4Exe).Path
$resolvedCollectorSource = (Resolve-Path -LiteralPath $CollectorDll).Path
$resolvedProfile = (Resolve-Path -LiteralPath $ProfilePath).Path
$resolvedOut = [IO.Path]::GetFullPath($OutDir)
if ((Get-FileHash -LiteralPath $resolvedExe -Algorithm SHA256).Hash.ToLowerInvariant() -cne $expectedExeSha256) {
    throw 'Hoi4Exe does not match exact a729 authority.'
}
if (@(Get-Process -Name the game -ErrorAction SilentlyContinue).Count -ne 0) {
    throw 'A25/A26 acquisition requires a clean early launch; close the existing hoi4.exe first.'
}
if ($ProcessTimeoutSeconds -lt 60 -or $HarnessExitTimeoutSeconds -lt 30) { throw 'A25/A26 timeout budget is too small.' }
if (Test-Path -LiteralPath $resolvedOut) {
    if (-not (Test-Path -LiteralPath $resolvedOut -PathType Container) -or @(Get-ChildItem -LiteralPath $resolvedOut -Force).Count -ne 0) {
        throw "OutDir must be absent or empty so stale A25/A26 evidence cannot be selected: $resolvedOut"
    }
} else {
    [IO.Directory]::CreateDirectory($resolvedOut) | Out-Null
}

$loadedCollector = Join-Path $resolvedOut 'a25-a26-evidence-collector.dll'
Copy-Item -LiteralPath $resolvedCollectorSource -Destination $loadedCollector
$actionDriver = Join-Path $PSScriptRoot 'Invoke-A25A26AuthenticatedActions.ps1'
$assembler = Join-Path $repo 'extractor\native\lanes\resolved_tooltip_instances_live\capture_a26_same_present.py'
$combinedVerifier = Join-Path $repo 'extractor\native\lanes\resolved_tooltip_instances_live\verify_a25_a26_acquisition.py'
foreach ($path in @($actionDriver, $assembler, $combinedVerifier)) {
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "A25/A26 acquisition component missing: $path" }
}

$actionReceipt = Join-Path $resolvedOut 'action_receipt.json'
$a25Receipt = Join-Path $resolvedOut 'a25_owner_glyph_capture.json'
$a26CaseDir = Join-Path $resolvedOut 'a26_cases'
$a26Matrix = Join-Path $resolvedOut 'a26_same_present_matrix.json'
$admission = Join-Path $resolvedOut 'a25_a26_acquisition_admission.json'
$stdoutPath = Join-Path $resolvedOut 'harness.stdout.log'
$stderrPath = Join-Path $resolvedOut 'harness.stderr.log'
$launchReceipt = Join-Path $resolvedOut 'harness_launch.json'

$launch = [ordered]@{
    schema = 'hoi4.a25-a26.harness-launch.v1'
    status = 'ARMED'
    created_utc = [DateTime]::UtcNow.ToString('o')
    executable_sha256 = $expectedExeSha256
    harness_file = [IO.Path]::GetFileName($resolvedHarness)
    harness_sha256 = (Get-FileHash -LiteralPath $resolvedHarness -Algorithm SHA256).Hash.ToLowerInvariant()
    collector_file = [IO.Path]::GetFileName($loadedCollector)
    collector_sha256 = (Get-FileHash -LiteralPath $loadedCollector -Algorithm SHA256).Hash.ToLowerInvariant()
    profile_file = [IO.Path]::GetFileName($resolvedProfile)
    profile_sha256 = (Get-FileHash -LiteralPath $resolvedProfile -Algorithm SHA256).Hash.ToLowerInvariant()
    harness_argument_count = $HarnessArgument.Count
    harness_arguments_sha256 = [BitConverter]::ToString(([Security.Cryptography.SHA256]::Create().ComputeHash([Text.Encoding]::UTF8.GetBytes(($HarnessArgument -join "`0"))))).Replace('-','').ToLowerInvariant()
}
Write-CreateNewUtf8 $launchReceipt (($launch | ConvertTo-Json -Depth 8) + "`n")

$start = [Diagnostics.ProcessStartInfo]::new()
$start.FileName = $resolvedHarness
$start.UseShellExecute = $false
$start.CreateNoWindow = $true
$start.RedirectStandardOutput = $true
$start.RedirectStandardError = $true
$start.Arguments = (($HarnessArgument | ForEach-Object { Quote-NativeArgument $_ }) -join ' ')
$start.EnvironmentVariables['BATTLE_PLAN_A25_A26_GAME_EXE'] = $resolvedExe
$start.EnvironmentVariables['BATTLE_PLAN_A25_A26_COLLECTOR_DLL'] = $loadedCollector
$start.EnvironmentVariables['BATTLE_PLAN_A25_A26_OUTPUT_DIR'] = $resolvedOut
$start.EnvironmentVariables['BATTLE_PLAN_A25_A26_ACTION_PROFILE'] = $resolvedProfile
$start.EnvironmentVariables['BATTLE_PLAN_A25_A26_ACTION_SIGNAL_DIR'] = (Join-Path $resolvedOut 'action_signals')
$start.EnvironmentVariables['BATTLE_PLAN_A25_A26_A25_RECEIPT'] = $a25Receipt
$start.EnvironmentVariables['BATTLE_PLAN_A25_A26_A26_CASE_DIR'] = $a26CaseDir
$harness = [Diagnostics.Process]::new(); $harness.StartInfo = $start
if (-not $harness.Start()) { throw 'Custom A25/A26 owner harness did not start.' }
$stdoutTask = $harness.StandardOutput.ReadToEndAsync(); $stderrTask = $harness.StandardError.ReadToEndAsync()

$actionFailure = $null
try {
    & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $actionDriver `
        -Hoi4Exe $resolvedExe `
        -CollectorDll $loadedCollector `
        -ProfilePath $resolvedProfile `
        -OutDir $resolvedOut `
        -ReceiptPath $actionReceipt `
        -ProcessTimeoutSeconds $ProcessTimeoutSeconds
    if ($LASTEXITCODE -ne 0) { throw "Authenticated action driver exited with code $LASTEXITCODE" }
} catch {
    $actionFailure = $_.Exception.Message
}

if ($null -eq $actionFailure) {
    if (-not $harness.WaitForExit($HarnessExitTimeoutSeconds * 1000)) {
        $actionFailure = "Owner harness did not exit within $HarnessExitTimeoutSeconds seconds after the final case."
    } elseif ($harness.ExitCode -ne 0) {
        $actionFailure = "Owner harness exited with code $($harness.ExitCode)."
    }
}
if ($null -ne $actionFailure -and -not $harness.HasExited) {
    try {
        # The helper is scoped to this acquisition.  Do not terminate the game:
        # retaining its paused state and the partial raw directory is more useful
        # for diagnosis than cascading cleanup through a separate process.
        $harness.Kill()
        [void]$harness.WaitForExit(30000)
    } catch {}
}
if ($harness.HasExited) {
    $stdoutTask.Wait(); $stderrTask.Wait()
    Write-CreateNewUtf8 $stdoutPath $stdoutTask.Result
    Write-CreateNewUtf8 $stderrPath $stderrTask.Result
}
if ($null -ne $actionFailure) {
    throw "$actionFailure Evidence and any terminal harness logs were retained at $resolvedOut"
}

$caseOrder = @(
    'hidden_baseline_1920x1080',
    'capture_start_visible_1920x1080',
    'show_1920x1080',
    'hide_1920x1080',
    'destroyed_1920x1080',
    'owner_change_1920x1080',
    'multiline_dynamic_1920x1080',
    'edge_geometry_1920x1080',
    'edge_geometry_2560x1440'
)
$assembleArgs = @('-B', $assembler)
foreach ($caseId in $caseOrder) {
    $casePath = Join-Path $a26CaseDir ($caseId + '.json')
    if (-not (Test-Path -LiteralPath $casePath -PathType Leaf)) { throw "Harness omitted A26 case receipt: $caseId" }
    $assembleArgs += @('--case-receipt', $casePath)
}
$assembleArgs += @('--output', $a26Matrix)
& $Python @assembleArgs
if ($LASTEXITCODE -ne 0) { throw 'A26 matrix assembly rejected the raw case set.' }

& $Python -B $combinedVerifier `
    --out-dir $resolvedOut `
    --profile $resolvedProfile `
    --launch-receipt $launchReceipt `
    --action-receipt $actionReceipt `
    --a25-receipt $a25Receipt `
    --a26-matrix $a26Matrix `
    --collector-dll $loadedCollector `
    --harness-exe $resolvedHarness `
    --output $admission
if ($LASTEXITCODE -ne 0) { throw "A25/A26 live evidence failed independent admission; retained at $resolvedOut" }
Write-Output "A25/A26 tooltip/glyph acquisition accepted: $admission"
