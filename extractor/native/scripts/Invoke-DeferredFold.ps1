param(
    [Parameter(Mandatory = $true)]
    [string]$RunDir,

    # Zero means recorder_host chooses default_fold_workers(): cores/2,
    # clamped to 1..8. Any positive value is an explicit override.
    [ValidateRange(0, 1024)]
    [int]$Workers = 0,

    [switch]$SkipBuild,

    [switch]$DryRun
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

function Assert-File {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$Label
    )
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "$Label not found: $Path"
    }
}

function Format-CommandArgument {
    param([Parameter(Mandatory = $true)][string]$Value)
    if ($Value -match '[\s"]') {
        return '"' + ($Value -replace '"', '\"') + '"'
    }
    return $Value
}

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = (Resolve-Path -LiteralPath (Join-Path $scriptDir "..\..\..")).Path
$nativeRoot = Join-Path $repoRoot "extractor\native"
$cargoManifest = Join-Path $nativeRoot "Cargo.toml"
$targetRoot = if ([string]::IsNullOrWhiteSpace($env:CARGO_TARGET_DIR)) { Join-Path $repoRoot '.cargo-target' } else { [System.IO.Path]::GetFullPath($env:CARGO_TARGET_DIR) }
$env:CARGO_TARGET_DIR = $targetRoot
$collectorRecord = Join-Path $targetRoot "release\collector_record.exe"
$gameDefsOut = Join-Path $repoRoot "extractor\game_defs\out"

if (-not (Test-Path -LiteralPath $RunDir -PathType Container)) {
    throw "run directory not found: $RunDir"
}
$RunDir = (Resolve-Path -LiteralPath $RunDir).Path

$streamPath = Join-Path $RunDir "collector_stream.bin"
$manifestPath = Join-Path $RunDir "native_session_manifest.json"
Assert-File -Path $streamPath -Label "collector stream"
Assert-File -Path $manifestPath -Label "native session manifest"

try {
    $manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
} catch {
    throw "could not parse native session manifest $($manifestPath): $($_.Exception.Message)"
}

if (-not ($manifest.PSObject.Properties.Name -contains "replay_game_id") -or
    [string]::IsNullOrWhiteSpace([string]$manifest.replay_game_id)) {
    throw "native session manifest has no replay_game_id: $manifestPath"
}
if (-not ($manifest.PSObject.Properties.Name -contains "stream_format") -or
    [string]$manifest.stream_format -ne "compact") {
    throw "deferred fold requires a compact run manifest: $manifestPath"
}
if (-not ($manifest.PSObject.Properties.Name -contains "replay_file") -or
    [string]::IsNullOrWhiteSpace([string]$manifest.replay_file)) {
    throw "native session manifest has no replay_file: $manifestPath"
}
$replayFile = [string]$manifest.replay_file
if ([System.IO.Path]::GetFileName($replayFile) -ne $replayFile) {
    throw "native session manifest replay_file must be a leaf file name: $replayFile"
}
$replayPath = Join-Path $RunDir $replayFile
Assert-File -Path $replayPath -Label "existing replay container"

if (-not ($manifest.PSObject.Properties.Name -contains "host_finalize") -or
    -not ($manifest.host_finalize.PSObject.Properties.Name -contains "fold") -or
    -not ($manifest.host_finalize.fold.PSObject.Properties.Name -contains "skipped") -or
    $manifest.host_finalize.fold.skipped -ne $true) {
    throw "run manifest does not record a pending --skip-host-fold container: $manifestPath"
}
if (-not ($manifest.PSObject.Properties.Name -contains "replay_packaging") -or
    -not ($manifest.replay_packaging.PSObject.Properties.Name -contains "static_artifacts")) {
    throw "run manifest has no replay_packaging metadata required for byte-equivalent repack: $manifestPath"
}

$staticArtifactMap = @{
    "templates_equipment_catalog" = @{
        Flag = "--static-templates-equipment-catalog"
        Label = "templates/equipment static catalog"
        Path = Join-Path $gameDefsOut "templates_equipment_catalog\templates_equipment_catalog_live_domain59\domain59_static_catalog.json"
    }
    "localisation_join" = @{
        Flag = "--static-localisation-join"
        Label = "localisation static join"
        Path = Join-Path $gameDefsOut "localisation_gate\static_localisation_join_r3\localisation_catalog.json"
    }
    "decisions_static_definitions" = @{
        Flag = "--static-decisions-definitions"
        Label = "decisions static catalog"
        Path = Join-Path $gameDefsOut "decisions_static\decisions_static_definitions\decision_definitions_catalog.json"
    }
    "idea_tech_loc_xref_static" = @{
        Flag = "--static-idea-tech-loc-xref"
        Label = "idea/tech localisation xref"
        Path = Join-Path $gameDefsOut "idea_tech_loc_xref\idea_tech_loc_xref_static\idea_tech_loc_xref_catalog.json"
    }
    "ship_definitions_static" = @{
        Flag = "--static-ship-definitions"
        Label = "ship definitions static catalog"
        Path = Join-Path $gameDefsOut "ship_definitions_static\ship_definitions_static\ship_definitions_catalog.json"
    }
    "gfx_icon_asset_manifest_static" = @{
        Flag = "--static-gfx-icon-manifest"
        Label = "GFX icon manifest"
        Path = Join-Path $gameDefsOut "gfx_icon_manifest\gfx_icon_asset_manifest_static\gfx_icon_manifest_catalog.json"
    }
}

$foldArgs = @("--fold-and-repack-existing-run-dir", $RunDir)
$seenStaticArtifacts = @{}
foreach ($item in @($manifest.replay_packaging.static_artifacts)) {
    if (-not ($item.PSObject.Properties.Name -contains "name") -or
        -not ($item.PSObject.Properties.Name -contains "sha256")) {
        throw "invalid replay_packaging static artifact descriptor in $manifestPath"
    }
    $name = [string]$item.name
    if ($seenStaticArtifacts.ContainsKey($name)) {
        throw "duplicate replay_packaging static artifact name: $name"
    }
    $seenStaticArtifacts[$name] = $true
    if (-not $staticArtifactMap.ContainsKey($name)) {
        throw "unsupported replay_packaging static artifact name: $name"
    }
    $entry = $staticArtifactMap[$name]
    Assert-File -Path $entry.Path -Label $entry.Label
    $foldArgs += [string]$entry.Flag
    $foldArgs += [string]$entry.Path
}
if ($Workers -gt 0) {
    $foldArgs += "--workers"
    $foldArgs += "$Workers"
}

if (-not $SkipBuild) {
    $buildArgs = @(
        "build",
        "--release",
        "--manifest-path", $cargoManifest,
        "-p", "recorder_host",
        "--bin", "collector_record"
    )
    $buildCommand = "cargo " + (($buildArgs | ForEach-Object { Format-CommandArgument -Value $_ }) -join " ")
    if ($DryRun) {
        Write-Host "DRY RUN build: $buildCommand"
    } else {
        & cargo @buildArgs
        if ($LASTEXITCODE -ne 0) {
            throw "cargo build failed with exit code $LASTEXITCODE"
        }
    }
}

if (-not $DryRun -or $SkipBuild) {
    Assert-File -Path $collectorRecord -Label "release collector recorder"
}

$foldCommand = "& " + (Format-CommandArgument -Value $collectorRecord) + " " + (($foldArgs | ForEach-Object { Format-CommandArgument -Value $_ }) -join " ")
$workersDisplay = if ($Workers -gt 0) { "$Workers (override)" } else { "default_fold_workers (cores/2 clamped 1-8)" }
Write-Host "run_dir=$RunDir"
Write-Host "game_id=$($manifest.replay_game_id) workers=$workersDisplay"
Write-Host "replay_path=$replayPath"

if ($DryRun) {
    Write-Host "DRY RUN fold-and-repack: $foldCommand"
    exit 0
}

& $collectorRecord @foldArgs
if ($LASTEXITCODE -ne 0) {
    throw "collector_record fold-and-repack failed with exit code $LASTEXITCODE"
}

Assert-File -Path $replayPath -Label "repacked replay container"
try {
    $completedManifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
} catch {
    throw "could not parse completed native session manifest $($manifestPath): $($_.Exception.Message)"
}
if (-not ($completedManifest.PSObject.Properties.Name -contains "host_finalize") -or
    -not ($completedManifest.host_finalize.PSObject.Properties.Name -contains "fold") -or
    $completedManifest.host_finalize.fold.skipped -eq $true -or
    -not ($completedManifest.host_finalize.fold.PSObject.Properties.Name -contains "hours_emitted")) {
    throw "fold-and-repack completed without final fold telemetry in $manifestPath"
}

Write-Host "deferred_fold_complete=$replayPath"
