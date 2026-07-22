[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Hoi4Exe,
    [Parameter(Mandatory = $true)][string]$PdxUserDir,
    [Parameter(Mandatory = $true)][string]$ContinueSave,
    [Parameter(Mandatory = $true)][string]$PreparedProductModsetDir,
    [Parameter(Mandatory = $true)][string]$OutDir,
    [Parameter(Mandatory = $true)][string]$RelocationDir,
    [string]$ExpectedContinueSaveSha256 = 'c8488b05a3b87c718eeff979976537b79e867634d0b4dccb1eef6c5a45362735',
    [string]$PrebuiltArtifactAttestationPath = '',
    [string]$ReplayAppExe = '',
    [int]$DurationSeconds = 600,
    [int]$MinGameHours = 1,
    [int]$SampleMilliseconds = 25,
    [string]$Python = 'python',
    [string]$CrashDumpTool = 'procdump.exe',
    [switch]$OperatorApproved
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$expectedExeSha256 = '870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad'
$repo = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..\..'))
$earlyDriver = Join-Path $PSScriptRoot 'Start-Hoi4EarlyLaunchCaptureSafe.ps1'
$productionDriver = Join-Path $PSScriptRoot 'Start-Hoi4ProductionCaptureSafe.ps1'
$attestationHelpers = Join-Path $PSScriptRoot 'ProductionArtifact.Attestation.ps1'
$composedVerifier = Join-Path $repo 'replay_interface\re_evidence\rendering_laws\XN15L\verify_xn15l_composed.py'

function Get-Sha256([string]$Path) {
    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}
function Get-Sha256Bytes([byte[]]$Bytes) {
    $hasher = [Security.Cryptography.SHA256]::Create()
    try { return ([BitConverter]::ToString($hasher.ComputeHash($Bytes))).Replace('-', '').ToLowerInvariant() }
    finally { $hasher.Dispose() }
}
function ConvertTo-NativeArgumentLine([object[]]$Arguments) {
    return (($Arguments | ForEach-Object {
        $value = [string]$_
        if ($value -match '[\s"]') {
            '"' + ($value -replace '(\\*)"', '$1$1\"' -replace '(\\+)$', '$1$1') + '"'
        } else { $value }
    }) -join ' ')
}
function Write-AtomicReplaceBytes([string]$Path, [byte[]]$Bytes) {
    $full = [IO.Path]::GetFullPath($Path)
    $parent = [IO.Path]::GetDirectoryName($full)
    [IO.Directory]::CreateDirectory($parent) | Out-Null
    $temporary = [IO.Path]::Combine($parent, ([IO.Path]::GetRandomFileName() + '.tmp'))
    try {
        [IO.File]::WriteAllBytes($temporary, $Bytes)
        $stream = [IO.File]::Open($temporary, [IO.FileMode]::Open, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
        try { $stream.Flush($true) } finally { $stream.Dispose() }
        if ([IO.File]::Exists($full)) { [IO.File]::Replace($temporary, $full, $null, $true) }
        else { [IO.File]::Move($temporary, $full) }
    } finally { if ([IO.File]::Exists($temporary)) { [IO.File]::Delete($temporary) } }
}
function Write-AtomicNewText([string]$Path, [string]$Text) {
    $full = [IO.Path]::GetFullPath($Path)
    if ([IO.File]::Exists($full)) { throw "Refusing to replace immutable XN15L artifact: $full" }
    $parent = [IO.Path]::GetDirectoryName($full)
    [IO.Directory]::CreateDirectory($parent) | Out-Null
    $temporary = [IO.Path]::Combine($parent, ([IO.Path]::GetRandomFileName() + '.tmp'))
    try {
        [IO.File]::WriteAllText($temporary, $Text, [Text.UTF8Encoding]::new($false))
        $stream = [IO.File]::Open($temporary, [IO.FileMode]::Open, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
        try { $stream.Flush($true) } finally { $stream.Dispose() }
        [IO.File]::Move($temporary, $full)
    } finally { if ([IO.File]::Exists($temporary)) { [IO.File]::Delete($temporary) } }
}
function Write-AtomicNewJson([string]$Path, $Value) {
    Write-AtomicNewText -Path $Path -Text (($Value | ConvertTo-Json -Depth 30) + "`n")
}
function Copy-AtomicNewFile([string]$Source, [string]$Destination) {
    $full = [IO.Path]::GetFullPath($Destination)
    if ([IO.File]::Exists($full)) { throw "Refusing to replace immutable XN15L artifact: $full" }
    $parent = [IO.Path]::GetDirectoryName($full)
    [IO.Directory]::CreateDirectory($parent) | Out-Null
    $temporary = [IO.Path]::Combine($parent, ([IO.Path]::GetRandomFileName() + '.tmp'))
    try {
        [IO.File]::Copy($Source, $temporary, $false)
        $stream = [IO.File]::Open($temporary, [IO.FileMode]::Open, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
        try { $stream.Flush($true) } finally { $stream.Dispose() }
        [IO.File]::Move($temporary, $full)
    } finally { if ([IO.File]::Exists($temporary)) { [IO.File]::Delete($temporary) } }
}
function New-ArtifactRow([string]$Path) {
    $item = Get-Item -LiteralPath $Path
    return [ordered]@{
        file = $item.Name
        bytes = [int64]$item.Length
        sha256 = Get-Sha256 $item.FullName
    }
}
function Invoke-ImmutableGate([string]$Gate, [string]$Program, [object[]]$Arguments, [string]$Root) {
    $stdoutPath = Join-Path $Root ($Gate + '.stdout.log')
    $stderrPath = Join-Path $Root ($Gate + '.stderr.log')
    $start = [Diagnostics.ProcessStartInfo]::new()
    $start.FileName = $Program
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    if ($null -ne $start.PSObject.Properties['ArgumentList']) {
        foreach ($argument in $Arguments) { [void]$start.ArgumentList.Add([string]$argument) }
    } else { $start.Arguments = ConvertTo-NativeArgumentLine $Arguments }
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $start
    try {
        if (-not $process.Start()) { throw "$Gate process start returned false" }
        $stdoutTask = $process.StandardOutput.ReadToEndAsync()
        $stderrTask = $process.StandardError.ReadToEndAsync()
        $process.WaitForExit()
        $exitCode = $process.ExitCode
        Write-AtomicNewText $stdoutPath ([string]$stdoutTask.Result)
        Write-AtomicNewText $stderrPath ([string]$stderrTask.Result)
    } finally { $process.Dispose() }
    $row = [ordered]@{
        gate = $Gate
        executable_sha256 = Get-Sha256 $Program
        exit_code = $exitCode
        stdout = New-ArtifactRow $stdoutPath
        stderr = New-ArtifactRow $stderrPath
    }
    if ($exitCode -ne 0) { throw "$Gate rejected the XN15L transaction; immutable logs are in $Root" }
    return $row
}
function Stop-OwnedExactHoi4([string]$Executable, [DateTime]$NotBeforeUtc, [Nullable[int]]$ExpectedProcessId) {
    $candidates = @(Get-Process -Name the game -ErrorAction SilentlyContinue)
    if ($null -ne $ExpectedProcessId) {
        $candidates = @($candidates | Where-Object { $_.Id -eq $ExpectedProcessId.Value })
    }
    foreach ($candidate in $candidates) {
        try {
            $path = [IO.Path]::GetFullPath([string]$candidate.MainModule.FileName)
            $started = $candidate.StartTime.ToUniversalTime()
            if ($path.Equals($Executable, [StringComparison]::OrdinalIgnoreCase) -and
                $started -ge $NotBeforeUtc -and (Get-Sha256 $path) -ceq $expectedExeSha256) {
                Stop-Process -Id $candidate.Id -Force -ErrorAction Stop
                [void]$candidate.WaitForExit(30000)
            }
        } catch {
            if (-not $candidate.HasExited) { throw }
        } finally { $candidate.Dispose() }
    }
}

if (-not $OperatorApproved) { throw 'XN15L early-launch acquisition requires explicit -OperatorApproved authority.' }
if ($DurationSeconds -lt 300 -or $DurationSeconds -gt 3600) { throw 'DurationSeconds must be 300..3600.' }
if ($MinGameHours -lt 1 -or $MinGameHours -gt 48) { throw 'MinGameHours must be 1..48.' }
if ($SampleMilliseconds -lt 1 -or $SampleMilliseconds -gt 1000) { throw 'SampleMilliseconds must be 1..1000.' }
if ($ExpectedContinueSaveSha256 -cnotmatch '^[0-9a-f]{64}$') { throw 'ExpectedContinueSaveSha256 is noncanonical.' }
if (@(Get-Process -Name the game -ErrorAction SilentlyContinue).Count -ne 0) { throw 'XN15L requires a clean launch with no existing hoi4.exe.' }
foreach ($required in @($earlyDriver, $productionDriver, $attestationHelpers, $composedVerifier)) {
    if (-not (Test-Path -LiteralPath $required -PathType Leaf)) { throw "Missing XN15L component: $required" }
}
$exe = (Resolve-Path -LiteralPath $Hoi4Exe).Path
$user = (Resolve-Path -LiteralPath $PdxUserDir).Path
$save = (Resolve-Path -LiteralPath $ContinueSave).Path
$prepared = (Resolve-Path -LiteralPath $PreparedProductModsetDir).Path
$pythonExe = (Get-Command $Python -CommandType Application -ErrorAction Stop).Source
$out = [IO.Path]::GetFullPath($OutDir)
$relocation = [IO.Path]::GetFullPath($RelocationDir)
if ((Get-Sha256 $exe) -cne $expectedExeSha256) { throw 'Hoi4Exe is not exact a729.' }
if ((Get-Sha256 $save) -cne $ExpectedContinueSaveSha256) { throw 'ContinueSave differs from the explicitly frozen save identity.' }
$saveSha256 = $ExpectedContinueSaveSha256
$saveBytes = [int64](Get-Item -LiteralPath $save).Length
$saveRoot = [IO.Path]::GetFullPath((Join-Path $user 'save games')).TrimEnd([char]'\')
if (-not [string]::Equals((Split-Path -Parent $save).TrimEnd([char]'\'), $saveRoot, [StringComparison]::OrdinalIgnoreCase)) {
    throw "ContinueSave must be a direct child of $saveRoot"
}
if ($out -ieq $relocation -or $out.StartsWith($relocation + '\', [StringComparison]::OrdinalIgnoreCase) -or
    $relocation.StartsWith($out + '\', [StringComparison]::OrdinalIgnoreCase)) {
    throw 'OutDir and RelocationDir must be independent roots.'
}
foreach ($path in @($out, $relocation)) {
    if (Test-Path -LiteralPath $path) { throw "XN15L output roots must be absent and immutable: $path" }
}

. $attestationHelpers
if ([string]::IsNullOrWhiteSpace($PrebuiltArtifactAttestationPath)) {
    $PrebuiltArtifactAttestationPath = Join-Path $repo '.cargo-target\release\hoi4-production-prebuilt-artifacts.json'
}
$prebuilt = Test-ProductionArtifactAttestation -RepoRoot $repo -ReceiptPath $PrebuiltArtifactAttestationPath -ExpectedDriverPath $productionDriver
$prebuiltJson = Get-Content -LiteralPath $PrebuiltArtifactAttestationPath -Raw | ConvertFrom-Json
if ([string]$prebuiltJson.central_validation.requested_workspace -cne 'All' -or
    (@($prebuiltJson.central_validation.validated_workspaces) -notcontains 'Replay')) {
    throw 'XN15L composition requires a centralized Release that validated both Extractor and Replay.'
}
if ([string]::IsNullOrWhiteSpace($ReplayAppExe)) {
    $ReplayAppExe = Join-Path $repo '.cargo-target\release\replay_app.exe'
}
$replayApp = (Resolve-Path -LiteralPath $ReplayAppExe).Path
$canonicalReplayApp = [IO.Path]::GetFullPath((Join-Path $repo '.cargo-target\release\replay_app.exe'))
if (-not $replayApp.Equals($canonicalReplayApp, [StringComparison]::OrdinalIgnoreCase)) {
    throw 'ReplayAppExe must be the centralized release artifact under .cargo-target/release.'
}
$attestationItem = Get-Item -LiteralPath $PrebuiltArtifactAttestationPath
$replayAppItem = Get-Item -LiteralPath $replayApp
$replayNativeRoot = [IO.Path]::GetFullPath((Join-Path $repo 'replay_interface\native')).TrimEnd([char]'\')
$replayInputs = @(Get-ChildItem -LiteralPath $replayNativeRoot -Recurse -File |
    Where-Object {
        $relative = $_.FullName.Substring($replayNativeRoot.Length).TrimStart([char]'\')
        $relative -notmatch '(^|\\)(target|\.cargo-target|\.tmp[^\\]*)(\\|$)' -and
        $_.Extension.ToLowerInvariant() -in @('.rs', '.toml', '.lock', '.c', '.cc', '.cpp', '.h', '.hpp', '.json', '.py', '.ps1')
    })
$newestReplayInputUtc = ($replayInputs | Measure-Object -Property LastWriteTimeUtc -Maximum).Maximum
if ($null -eq $newestReplayInputUtc -or $replayAppItem.LastWriteTimeUtc -lt $newestReplayInputUtc -or
    $attestationItem.LastWriteTimeUtc -lt $replayAppItem.LastWriteTimeUtc) {
    throw 'Replay source/artifact/central-All attestation order is stale; rerun centralized Release -Workspace All.'
}

$continuePath = Join-Path $user 'continue_game.json'
$priorExists = Test-Path -LiteralPath $continuePath -PathType Leaf
$priorBytes = if ($priorExists) { [IO.File]::ReadAllBytes($continuePath) } else { $null }
$priorSha256 = if ($priorExists) { Get-Sha256Bytes $priorBytes } else { $null }
$continue = if ($priorExists) { Get-Content -LiteralPath $continuePath -Raw | ConvertFrom-Json } else {
    [pscustomobject]@{ title = [IO.Path]::GetFileNameWithoutExtension($save); desc = ''; date = [DateTime]::UtcNow.ToString('R'); filename = ''; is_remote = $false }
}
if ($null -eq $continue.PSObject.Properties['filename']) { $continue | Add-Member -NotePropertyName filename -NotePropertyValue '' }
if ($null -eq $continue.PSObject.Properties['is_remote']) { $continue | Add-Member -NotePropertyName is_remote -NotePropertyValue $false }
$continue.filename = [IO.Path]::GetFileName($save)
$continue.is_remote = $false
$replacementBytes = [Text.UTF8Encoding]::new($false).GetBytes((($continue | ConvertTo-Json -Depth 10) + "`n"))
$replacementSha256 = Get-Sha256Bytes $replacementBytes
$launchNotBefore = [DateTime]::UtcNow
$overridden = $false
$captureError = $null
$ownedProcessId = $null
try {
    $overridden = $true
    Write-AtomicReplaceBytes $continuePath $replacementBytes
    if ((Get-Sha256 $continuePath) -cne $replacementSha256) { throw 'continue_game.json override changed before launch.' }
    & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $earlyDriver `
        -Hoi4Exe $exe -PdxUserDir $user -PreparedProductModsetDir $prepared -OutDir $out `
        -Purpose XN15L -DurationSeconds $DurationSeconds -MinGameHours $MinGameHours `
        -SampleMilliseconds $SampleMilliseconds -PythonExe $pythonExe `
        -PrebuiltArtifactAttestationPath $PrebuiltArtifactAttestationPath `
        -CrashDumpTool $CrashDumpTool -OperatorApproved
    if ($LASTEXITCODE -ne 0) { throw "XN15L early-launch driver exited with code $LASTEXITCODE." }
    $resolvedPath = Join-Path $out 'early_launch_resolved_attempt.json'
    if (-not (Test-Path -LiteralPath $resolvedPath -PathType Leaf)) { throw 'XN15L early launch emitted no resolved attempt.' }
    $resolved = Get-Content -LiteralPath $resolvedPath -Raw | ConvertFrom-Json
    $ownedProcessId = [int]$resolved.process_id
} catch { $captureError = $_ } finally {
    try { Stop-OwnedExactHoi4 -Executable $exe -NotBeforeUtc $launchNotBefore -ExpectedProcessId $ownedProcessId }
    finally {
        if ($overridden) {
            $continueChanged = -not (Test-Path -LiteralPath $continuePath -PathType Leaf) -or
                (Get-Sha256 $continuePath) -cne $replacementSha256
            if ($priorExists) {
                Write-AtomicReplaceBytes $continuePath $priorBytes
                if ((Get-Sha256 $continuePath) -cne $priorSha256) { throw 'continue_game.json prior bytes were not restored.' }
            } elseif (Test-Path -LiteralPath $continuePath) { Remove-Item -LiteralPath $continuePath -Force }
            $overridden = $false
            if ($continueChanged) { throw 'continue_game.json changed during the owned XN15L launch; prior bytes were restored.' }
        }
    }
}
if ($null -ne $captureError) { throw $captureError }
if ((Get-Sha256 $save) -cne $saveSha256 -or (Get-Item -LiteralPath $save).Length -ne $saveBytes) {
    throw 'ContinueSave bytes changed during the owned XN15L transaction.'
}

$manifestPath = Join-Path $out 'native_session_manifest.json'
$liveVerificationPath = Join-Path $out 'xn15l_live_verification.json'
foreach ($required in @($manifestPath, $liveVerificationPath, (Join-Path $out 'collector_stream.bin'))) {
    if (-not (Test-Path -LiteralPath $required -PathType Leaf)) { throw "XN15L capture artifact is absent: $required" }
}
$manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
$live = Get-Content -LiteralPath $liveVerificationPath -Raw | ConvertFrom-Json
if ([string]$live.verdict -cne 'passed' -or @($live.problems).Count -ne 0) { throw 'Base XN15L live verification did not pass.' }
$replayName = [string]$manifest.replay_file
if ([IO.Path]::GetFileName($replayName) -cne $replayName) { throw 'Capture manifest replay filename is nonportable.' }
$sourceReplay = Join-Path $out $replayName
if (-not (Test-Path -LiteralPath $sourceReplay -PathType Leaf)) { throw 'Captured portable replay is absent.' }
$rawHours = @($live.evidence.compact_stream.game_hours)
if ($rawHours.Count -eq 0) { throw 'Live verification retained no selected d77 hour.' }
$selectedGameHourRaw = [int]$rawHours[$rawHours.Count - 1]
$selectedUpdateIndex = $selectedGameHourRaw - 43799999
if ($selectedUpdateIndex -lt 0) { throw 'Selected d77 hour cannot be converted to a compact update index.' }

[IO.Directory]::CreateDirectory($relocation) | Out-Null
$relocatedReplay = Join-Path $relocation $replayName
Copy-AtomicNewFile -Source $sourceReplay -Destination $relocatedReplay
$sourceReplaySha256 = Get-Sha256 $sourceReplay
if ((Get-Sha256 $relocatedReplay) -cne $sourceReplaySha256 -or
    (Get-Item -LiteralPath $relocatedReplay).Length -ne (Get-Item -LiteralPath $sourceReplay).Length) {
    throw 'Relocated replay bytes differ from the captured replay.'
}
$configPath = Join-Path $relocation 'playback-profile.json'
$pixelBmp = Join-Path $relocation 'xn15l-native-frame.bmp'
$pixelReceipt = $pixelBmp + '.pixel-gate-receipt.json'
$gameRoot = Split-Path -Parent $exe
$gates = [Collections.Generic.List[object]]::new()
$gates.Add((Invoke-ImmutableGate -Gate 'recording_validate' -Program $replayApp -Root $relocation -Arguments @(
    '--validate-recording', $relocatedReplay
)))
$gates.Add((Invoke-ImmutableGate -Gate 'playback_vfs_validate' -Program $replayApp -Root $relocation -Arguments @(
    '--validate-playback', $relocatedReplay, '--game', $gameRoot, '--userdata', $user,
    '--config', $configPath, '--non-interactive'
)))
$gates.Add((Invoke-ImmutableGate -Gate 'native_composition_pixel' -Program $replayApp -Root $relocation -Arguments @(
    '--recording', $relocatedReplay, '--game', $gameRoot, '--userdata', $user,
    '--config', $configPath, '--non-interactive', '--dump-frame', $pixelBmp,
    '--dump-hour', [string]$selectedUpdateIndex
)))
foreach ($required in @($pixelBmp, $pixelReceipt)) {
    if (-not (Test-Path -LiteralPath $required -PathType Leaf)) { throw "Native composition gate emitted no artifact: $required" }
}

$ledgerPath = Join-Path $relocation 'xn15l_composed_execution.json'
Write-AtomicNewJson $ledgerPath ([ordered]@{
    schema = 'hoi4.xn15l.composed-execution.v1'
    status = 'GATES_COMPLETED'
    executable_sha256 = $expectedExeSha256
    prebuilt_artifact_attestation_sha256 = Get-Sha256 $PrebuiltArtifactAttestationPath
    replay_app = [ordered]@{
        file = [IO.Path]::GetFileName($replayApp)
        bytes = [int64](Get-Item -LiteralPath $replayApp).Length
        sha256 = Get-Sha256 $replayApp
    }
    runner_sha256 = Get-Sha256 $MyInvocation.MyCommand.Path
    composed_verifier_sha256 = Get-Sha256 $composedVerifier
    continue_save_sha256 = $saveSha256
    continue_save_bytes = $saveBytes
    continue_selection_sha256 = $replacementSha256
    prepared_product_modset_sha256 = [string]$manifest.portable_content_manifest.product_modset_sha256
    capture_manifest_sha256 = Get-Sha256 $manifestPath
    live_verification_sha256 = Get-Sha256 $liveVerificationPath
    source_replay_file = $replayName
    source_replay_sha256 = $sourceReplaySha256
    relocated_replay_file = $replayName
    relocated_replay_sha256 = Get-Sha256 $relocatedReplay
    selected_game_hour_raw = $selectedGameHourRaw
    selected_update_index = $selectedUpdateIndex
    pixel_bmp = [IO.Path]::GetFileName($pixelBmp)
    pixel_receipt = [IO.Path]::GetFileName($pixelReceipt)
    gates = @($gates)
})

$admissionPath = Join-Path $relocation 'xn15l_composed_admission.json'
& $pythonExe -B $composedVerifier --capture-dir $out --relocation-dir $relocation --out $admissionPath
if ($LASTEXITCODE -ne 0) { throw "XN15L composed admission rejected the transaction; inspect $admissionPath" }
Write-Output "XN15L capture-to-native-pixel transaction admitted: $admissionPath"
