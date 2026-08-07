param(
    [Parameter(Mandatory = $true)]
    [string]$RunDir,

    # Accepted for compatibility and forwarded verbatim. The repack now always
    # builds the certified H4IX/H4CP artifacts, and that build is a single
    # streaming pass, so the value does not change the work done.
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
# Two admissible shapes, mirroring collector_record's own gate:
#
#   deferred_fold        host_finalize.fold.skipped=true — the capture finalized
#                        with --skip-host-fold and published a container whose
#                        fold this run replaces.
#   unfinalized_capture  no host_finalize at all — the capture died (or its pack
#                        failed) before finalize, so there is no container and
#                        replay_file may be null. THIS is the case recovery
#                        exists for; refusing it made the "raw survives, refold
#                        offline" promise false for exactly the runs that need
#                        it (capture t32, 2026-08-07).
#
# The recorder re-checks all of this; these throws only fail fast with a better
# message. They must never be STRICTER than the recorder, or the wrapper becomes
# a second, drifting gate in front of the real one.
# Type checks are exact, not coercing: `-ne $true` would accept the STRING
# "true", and a nonblank `decoded_sha256` is not the same thing as a SHA-256.
# The recorder requires a JSON boolean, a numeric byte count and a lowercase
# 64-hex digest, and a wrapper that admits more than the gate it fronts is how a
# dry run passes and the real call refuses.
$hasHostFinalize = $manifest.PSObject.Properties.Name -contains "host_finalize"
if ($hasHostFinalize) {
    if (-not ($manifest.host_finalize.PSObject.Properties.Name -contains "fold") -or
        -not ($manifest.host_finalize.fold.PSObject.Properties.Name -contains "skipped") -or
        -not ($manifest.host_finalize.fold.skipped -is [bool]) -or
        -not $manifest.host_finalize.fold.skipped) {
        throw "run manifest already records a completed host finalize with a real fold; repacking it would re-seal an already-sealed recording: $manifestPath"
    }
    $admission = "deferred_fold"
} else {
    if (-not ($manifest.PSObject.Properties.Name -contains "stream_storage")) {
        throw "run manifest has neither host_finalize nor a stream_storage capture receipt; nothing can bind a container to these bytes: $manifestPath"
    }
    # Exactly the recorder's rule: an unsigned integer (serde `as_u64`), so a
    # negative or fractional value is refused here too rather than passing a
    # loose "is a number" check and being refused by the real gate.
    $decodedBytes = $manifest.stream_storage.decoded_bytes
    if ($null -eq $decodedBytes -or
        -not ($decodedBytes -is [int] -or $decodedBytes -is [long] -or $decodedBytes -is [decimal] -or $decodedBytes -is [double]) -or
        [double]$decodedBytes -lt 0 -or
        [double]$decodedBytes -ne [math]::Floor([double]$decodedBytes)) {
        throw "run manifest stream_storage.decoded_bytes is not a non-negative integer: $manifestPath"
    }
    # Case-INSENSITIVE, matching the recorder, which lowercases the digest
    # before validating it. A case-sensitive check here would refuse a manifest
    # the recorder accepts.
    if ([string]$manifest.stream_storage.decoded_sha256 -notmatch '^[0-9a-fA-F]{64}$') {
        throw "run manifest stream_storage.decoded_sha256 is not a SHA-256: $manifestPath"
    }
    # The recorder additionally refuses when a finalizer for this run is still
    # alive, and when a sealed container for this session already exists. Both
    # need process/container inspection this wrapper cannot do, so they are
    # left to the gate rather than approximated here.
    $admission = "unfinalized_capture"
}

$replayFile = if ($manifest.PSObject.Properties.Name -contains "replay_file") { [string]$manifest.replay_file } else { "" }
if (-not [string]::IsNullOrWhiteSpace($replayFile)) {
    if ([System.IO.Path]::GetFileName($replayFile) -ne $replayFile) {
        throw "native session manifest replay_file must be a leaf file name: $replayFile"
    }
} elseif ($admission -eq "deferred_fold") {
    throw "native session manifest has no replay_file: $manifestPath"
} else {
    # The packer nulled replay_file. The intended name is recovered the same way
    # the recorder recovers it: from replay_artifact.recovery.
    if (-not ($manifest.PSObject.Properties.Name -contains "replay_artifact") -or
        -not ($manifest.replay_artifact.PSObject.Properties.Name -contains "recovery")) {
        throw "unfinalized capture manifest has neither replay_file nor replay_artifact.recovery, so the container cannot be named: $manifestPath"
    }
    $replayFile = if ($manifest.replay_artifact.recovery -eq $true) { "native_session.failed.battleplan" } else { "native_session.battleplan" }
}
$replayPath = Join-Path $RunDir $replayFile
if ($admission -eq "deferred_fold") {
    Assert-File -Path $replayPath -Label "existing replay container"
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
# The repack produces binary hourly persistence (H4IX + H4CP), the same
# composition an ordinary compact capture seals inline, so the completion
# receipt is `fold.producer=replay_decoder` with an hour count — never the
# legacy per-hour JSON `hours_emitted`.
if (-not ($completedManifest.PSObject.Properties.Name -contains "host_finalize") -or
    -not ($completedManifest.host_finalize.PSObject.Properties.Name -contains "fold") -or
    $completedManifest.host_finalize.fold.skipped -eq $true -or
    [string]$completedManifest.host_finalize.fold.producer -ne "replay_decoder" -or
    -not ($completedManifest.host_finalize.fold.PSObject.Properties.Name -contains "hour_count")) {
    throw "fold-and-repack completed without certified replay artifact telemetry in $manifestPath"
}
if ([string]::IsNullOrWhiteSpace([string]$completedManifest.replay_file)) {
    throw "fold-and-repack sealed a container but the manifest still denies it (replay_file is null) in $manifestPath"
}

Write-Host "deferred_fold_complete=$replayPath"
