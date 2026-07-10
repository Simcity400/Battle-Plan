param(
    # Smoke/ShortSpeed5/FullGame run the lean `--profile production` preset
    # (RC per-read counters OFF, sidecar logs OFF, compact country frame ON).
    # Debug runs `--profile debug` (full diagnostics: RC counters ON, sidecar
    # logs ON, full per-country provenance, relaxed min-game-hours).
    [ValidateSet("Smoke", "ShortSpeed5", "FullGame", "Debug")]
    [string]$Mode = "FullGame",

    [int]$Hoi4Pid = 0,

    [double]$DurationSeconds = 0,

    [int]$MinGameHours = -1,

    [int]$SampleMs = 1,

    [int]$ReaderConcurrency = 12,

    [double]$EstimatedMegabytesPerSecond = 70,

    [string]$OutDir = "",

    [string]$SessionId = "",

    [switch]$SkipBuild,

    [switch]$SkipDiskSpaceCheck,

    # d53 relation-flag reader path override, passed straight through to
    # recorder_host as --d53-relation-read-mode. Empty = launcher does not pass
    # the flag (recorder_host default = offsets = current production behavior).
    # "shadow" runs both offsets and legacy-scan paths and fail-closes mismatching
    # d53 frames as partial (the live-validation d53 shadow gate).
    [ValidateSet("", "offsets", "scan", "shadow")]
    [string]$D53RelationReadMode = "",

    # When set, forces ALL FIVE fat-domain delta emitters (d41/d44/d48/d51/d53) to
    # the given mode via --dNN-emit-mode. Empty = launcher passes nothing (each
    # defaults to delta). "full" is used by the emit-mode A/B size-attribution run.
    [ValidateSet("", "delta", "full")]
    [string]$EmitModeAll = "",

    # Replay container zstd level override (0 = recorder default, level 13).
    # Validation sessions pass a low level (e.g. 3) so finalize takes seconds.
    [int]$ReplayZstdLevel = 0,

    # The capture-run path defers the expensive host fold by default. Opt in to
    # the legacy inline fold only when the extra turnaround is deliberate.
    [switch]$RunHostFold,

    [double]$MaxTurnaroundMinutes = 15,

    [switch]$AcknowledgeLongRun,

    [switch]$DryRun
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

function Resolve-SingleHoi4Pid {
    $processes = @(Get-Process -Name the game -ErrorAction SilentlyContinue)
    if ($processes.Count -eq 0) {
        throw "hoi4.exe is not running. Start the game before launching production capture."
    }
    if ($processes.Count -gt 1) {
        $ids = ($processes | ForEach-Object { $_.Id }) -join ", "
        throw "multiple hoi4.exe processes are running ($ids). Pass -Pid explicitly."
    }
    return [int]$processes[0].Id
}

function Assert-File {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$Label
    )
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "$Label not found: $Path"
    }
}

function Invoke-Checked {
    param(
        [Parameter(Mandatory = $true)][string]$Exe,
        [Parameter(Mandatory = $true)][string[]]$Arguments,
        [string]$LogPath = ""
    )
    $exitCode = 0
    if ($LogPath.Length -eq 0) {
        & $Exe @Arguments
        $exitCode = $LASTEXITCODE
    } else {
        $logDir = Split-Path -Parent $LogPath
        New-Item -ItemType Directory -Path $logDir -Force | Out-Null
        $stdoutPath = Join-Path $logDir "collector_record.stdout.tmp.log"
        $stderrPath = Join-Path $logDir "collector_record.stderr.tmp.log"
        Remove-Item -LiteralPath $stdoutPath -Force -ErrorAction SilentlyContinue
        Remove-Item -LiteralPath $stderrPath -Force -ErrorAction SilentlyContinue
        $argumentLine = ($Arguments | ForEach-Object {
            if ($_ -match "[\s`"]") {
                $escapedArg = $_ -replace '"', '\"'
                "`"$escapedArg`""
            } else {
                $_
            }
        }) -join " "
        $process = Start-Process `
            -FilePath $Exe `
            -ArgumentList $argumentLine `
            -Wait `
            -PassThru `
            -NoNewWindow `
            -RedirectStandardOutput $stdoutPath `
            -RedirectStandardError $stderrPath
        $exitCode = $process.ExitCode
        $stdout = ""
        if (Test-Path -LiteralPath $stdoutPath) {
            $stdoutContent = Get-Content -LiteralPath $stdoutPath -Raw
            if ($null -ne $stdoutContent) {
                $stdout = [string]$stdoutContent
            }
        }
        $stderr = ""
        if (Test-Path -LiteralPath $stderrPath) {
            $stderrContent = Get-Content -LiteralPath $stderrPath -Raw
            if ($null -ne $stderrContent) {
                $stderr = [string]$stderrContent
            }
        }
        Set-Content -LiteralPath $LogPath -Value ($stdout + $stderr) -Encoding UTF8
        if ($stdout.Length -gt 0) {
            Write-Host $stdout.TrimEnd()
        }
        if ($stderr.Length -gt 0) {
            Write-Host $stderr.TrimEnd()
        }
        Remove-Item -LiteralPath $stdoutPath -Force -ErrorAction SilentlyContinue
        Remove-Item -LiteralPath $stderrPath -Force -ErrorAction SilentlyContinue
    }
    if ($exitCode -ne 0) {
        Write-Host "ERROR: $Exe exited with code $exitCode"
        exit $exitCode
    }
}

function Invariant-Number {
    param([Parameter(Mandatory = $true)][double]$Value)
    return [string]::Format(
        [System.Globalization.CultureInfo]::InvariantCulture,
        "{0:0.###}",
        $Value
    )
}

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = (Resolve-Path -LiteralPath (Join-Path $scriptDir "..\..\..")).Path
$nativeRoot = Join-Path $repoRoot "extractor\native"
$cargoManifest = Join-Path $nativeRoot "Cargo.toml"
$releaseRoot = Join-Path $nativeRoot "target\release"
$collectorDll = Join-Path $releaseRoot "collector.dll"
$collectorRecord = Join-Path $releaseRoot "collector_record.exe"
$gameDefsOut = Join-Path $repoRoot "extractor\game_defs\out"

$staticTemplatesEquipmentCatalog = Join-Path $gameDefsOut "templates_equipment_catalog\templates_equipment_catalog_live_domain59\domain59_static_catalog.json"
$staticLocalisationJoin = Join-Path $gameDefsOut "localisation_gate\static_localisation_join_r3\localisation_catalog.json"
$staticDecisionsDefinitions = Join-Path $gameDefsOut "decisions_static\decisions_static_definitions\decision_definitions_catalog.json"
$staticIdeaTechLocXref = Join-Path $gameDefsOut "idea_tech_loc_xref\idea_tech_loc_xref_static\idea_tech_loc_xref_catalog.json"
$staticShipDefinitions = Join-Path $gameDefsOut "ship_definitions_static\ship_definitions_static\ship_definitions_catalog.json"
$staticGfxIconManifest = Join-Path $gameDefsOut "gfx_icon_manifest\gfx_icon_asset_manifest_static\gfx_icon_manifest_catalog.json"


if ($DurationSeconds -le 0) {
    switch ($Mode) {
        "Smoke" { $DurationSeconds = 90 }
        "ShortSpeed5" { $DurationSeconds = 300 }
        "FullGame" { $DurationSeconds = 21600 }
        "Debug" { $DurationSeconds = 300 }
    }
}

if ($MinGameHours -lt 0) {
    switch ($Mode) {
        "Smoke" { $MinGameHours = 0 }
        "ShortSpeed5" { $MinGameHours = 48 }
        "FullGame" { $MinGameHours = 48 }
        "Debug" { $MinGameHours = 12 }
    }
}

# Debug mode captures the same production domain set with full diagnostics.
if ($Mode -eq "Debug") {
    $CaptureProfile = "debug"
} else {
    $CaptureProfile = "production"
}

if ($SampleMs -lt 0) {
    throw "-SampleMs must be 0 or greater."
}
if ($ReaderConcurrency -lt 0) {
    throw "-ReaderConcurrency must be 0 or greater."
}
if ($EstimatedMegabytesPerSecond -le 0) {
    throw "-EstimatedMegabytesPerSecond must be greater than zero."
}
if ($DurationSeconds -le 0) {
    throw "-DurationSeconds must be greater than zero."
}
if ($MaxTurnaroundMinutes -le 0) {
    throw "-MaxTurnaroundMinutes must be greater than zero."
}

# Capture duration plus two minutes for launch/pack overhead is the binding
# deferred-fold estimate. Inline folding additionally uses the measured
# 16.6 MB/s host-fold throughput against the estimated raw stream size.
$turnaroundStopwatch = [System.Diagnostics.Stopwatch]::StartNew()
$turnaroundMarginSeconds = 120.0
$foldMegabytesPerSecond = 16.6
$projectedFoldSeconds = 0.0
if ($RunHostFold) {
    $estimatedStreamMegabytes = $DurationSeconds * $EstimatedMegabytesPerSecond
    $projectedFoldSeconds = $estimatedStreamMegabytes / $foldMegabytesPerSecond
}
$projectedTurnaroundSeconds = $DurationSeconds + $turnaroundMarginSeconds + $projectedFoldSeconds
$maxTurnaroundSeconds = $MaxTurnaroundMinutes * 60.0
if (-not $AcknowledgeLongRun -and $projectedTurnaroundSeconds -gt $maxTurnaroundSeconds) {
    $projectedTurnaroundMinutes = Invariant-Number -Value ($projectedTurnaroundSeconds / 60.0)
    $maxTurnaroundDisplay = Invariant-Number -Value $MaxTurnaroundMinutes
    $estimateBasis = if ($RunHostFold) {
        "-DurationSeconds plus a 2-minute margin and estimated host fold at $foldMegabytesPerSecond MB/s"
    } else {
        "-DurationSeconds plus a 2-minute margin"
    }
    throw "Projected capture-run turnaround is $projectedTurnaroundMinutes minutes ($estimateBasis), exceeding -MaxTurnaroundMinutes $maxTurnaroundDisplay. Shorten the capture or pass -AcknowledgeLongRun deliberately."
}

Assert-File -Path $staticTemplatesEquipmentCatalog -Label "templates/equipment static catalog"
Assert-File -Path $staticLocalisationJoin -Label "localisation static join"
Assert-File -Path $staticDecisionsDefinitions -Label "decisions static catalog"
Assert-File -Path $staticIdeaTechLocXref -Label "idea/tech localisation xref"
Assert-File -Path $staticShipDefinitions -Label "ship definitions static catalog"
Assert-File -Path $staticGfxIconManifest -Label "GFX icon manifest"

if ($Hoi4Pid -eq 0) {
    $Hoi4Pid = Resolve-SingleHoi4Pid
}

$stamp = Get-Date -Format "yyyyMMdd_HHmmss"
if ($OutDir.Length -eq 0) {
    $OutDir = Join-Path $nativeRoot "runs\production_speed5_${Mode}_$stamp"
}
if ($SessionId.Length -eq 0) {
    $SessionId = "production_speed5_${Mode}_$stamp"
}

$OutDir = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($OutDir)
New-Item -ItemType Directory -Path $OutDir -Force | Out-Null

if (-not $SkipDiskSpaceCheck) {
    $root = [System.IO.Path]::GetPathRoot($OutDir)
    if ($root.Length -lt 2 -or $root[1] -ne ":") {
        Write-Host "ERROR: disk-space check only supports local drive paths. Pass -SkipDiskSpaceCheck for this OutDir: $OutDir"
        exit 1
    }
    $driveName = $root.Substring(0, 1)
    $drive = Get-PSDrive -Name $driveName -PSProvider FileSystem
    $estimatedBytes = [int64]($DurationSeconds * $EstimatedMegabytesPerSecond * 1MB * 1.25)
    if ($drive.Free -lt $estimatedBytes) {
        $freeGb = [math]::Round($drive.Free / 1GB, 2)
        $needGb = [math]::Round($estimatedBytes / 1GB, 2)
        Write-Host "ERROR: not enough free space on $root for this run estimate: need ${needGb}GB, free ${freeGb}GB. Use -OutDir on a larger drive, lower -DurationSeconds, lower -EstimatedMegabytesPerSecond if you have newer evidence, or pass -SkipDiskSpaceCheck deliberately."
        exit 1
    }
    $freeDisplayGb = [math]::Round($drive.Free / 1GB, 2)
    $estimateDisplayGb = [math]::Round($estimatedBytes / 1GB, 2)
    Write-Host "disk_check=$root free_gb=$freeDisplayGb estimated_required_gb=$estimateDisplayGb"
}

if (-not $SkipBuild) {
    $buildArgs = @(
        "build",
        "--release",
        "--manifest-path",
        $cargoManifest,
        "-p",
        "collector",
        "-p",
        "recorder_host"
    )
    if ($DryRun) {
        Write-Host "DRY RUN build: cargo $($buildArgs -join ' ')"
    } else {
        Invoke-Checked -Exe "cargo" -Arguments $buildArgs
    }
}

Assert-File -Path $collectorDll -Label "release collector DLL"
Assert-File -Path $collectorRecord -Label "release collector recorder"

$durationArg = Invariant-Number -Value $DurationSeconds
$runArgs = @(
    "--pid", "$Hoi4Pid",
    "--dll", $collectorDll,
    "--out-dir", $OutDir,
    "--duration-s", $durationArg,
    "--sample-ms", "$SampleMs",
    "--min-game-hours", "$MinGameHours",
    "--profile", $CaptureProfile,
    "--reader-concurrency", "$ReaderConcurrency",
    "--session-id", $SessionId,
    "--static-templates-equipment-catalog", $staticTemplatesEquipmentCatalog,
    "--static-localisation-join", $staticLocalisationJoin,
    "--static-decisions-definitions", $staticDecisionsDefinitions,
    "--static-idea-tech-loc-xref", $staticIdeaTechLocXref,
    "--static-ship-definitions", $staticShipDefinitions,
    "--static-gfx-icon-manifest", $staticGfxIconManifest
)

if (-not $RunHostFold) {
    $runArgs += "--skip-host-fold"
}

if ($ReplayZstdLevel -gt 0) {
    $runArgs += "--replay-zstd-level"
    $runArgs += "$ReplayZstdLevel"
}

if ($D53RelationReadMode.Length -gt 0) {
    $runArgs += "--d53-relation-read-mode"
    $runArgs += $D53RelationReadMode
}

if ($EmitModeAll.Length -gt 0) {
    foreach ($emitFlag in @("--d41-emit-mode", "--d44-emit-mode", "--d48-emit-mode", "--d51-emit-mode", "--d53-emit-mode")) {
        $runArgs += $emitFlag
        $runArgs += $EmitModeAll
    }
}

$commandLine = "& `"$collectorRecord`" " + ($runArgs | ForEach-Object {
    if ($_ -match "\s") { "`"$_`"" } else { $_ }
}) -join " "

$commandPath = Join-Path $OutDir "production_command.ps1"
Set-Content -LiteralPath $commandPath -Value $commandLine -Encoding UTF8

$deferredFoldScript = Join-Path $scriptDir "Invoke-DeferredFold.ps1"
$deferredFoldCommand = "& `"$deferredFoldScript`" -RunDir `"$OutDir`""

$d53RelationDisplay = if ($ReplayZstdLevel -gt 0) {
    $runArgs += "--replay-zstd-level"
    $runArgs += "$ReplayZstdLevel"
}

if ($D53RelationReadMode.Length -gt 0) { $D53RelationReadMode } else { "(default)" }
$emitModeDisplay = if ($EmitModeAll.Length -gt 0) { $EmitModeAll } else { "(default)" }
$hostFoldDisplay = if ($RunHostFold) { "inline" } else { "deferred" }
$maxTurnaroundDisplay = Invariant-Number -Value $MaxTurnaroundMinutes
$projectedTurnaroundDisplay = Invariant-Number -Value ($projectedTurnaroundSeconds / 60.0)
$projectedFoldDisplay = Invariant-Number -Value ($projectedFoldSeconds / 60.0)
Write-Host "mode=$Mode profile=$CaptureProfile pid=$Hoi4Pid duration_s=$durationArg min_game_hours=$MinGameHours sample_ms=$SampleMs reader_concurrency=$ReaderConcurrency d53_relation_read_mode=$d53RelationDisplay emit_mode_all=$emitModeDisplay host_fold=$hostFoldDisplay max_turnaround_min=$maxTurnaroundDisplay projected_turnaround_min=$projectedTurnaroundDisplay projected_fold_min=$projectedFoldDisplay"
Write-Host "out_dir=$OutDir"
Write-Host "command_file=$commandPath"

if ($DryRun) {
    Write-Host "DRY RUN capture: $commandLine"
    Write-Host "deferred_fold_command=$deferredFoldCommand"
    exit 0
}

$logPath = Join-Path $OutDir "collector_record.console.log"
Invoke-Checked -Exe $collectorRecord -Arguments $runArgs -LogPath $logPath
Write-Host "deferred_fold_command=$deferredFoldCommand"
$turnaroundStopwatch.Stop()
$actualTurnaroundMinutes = $turnaroundStopwatch.Elapsed.TotalMinutes
$actualTurnaroundDisplay = Invariant-Number -Value $actualTurnaroundMinutes
Write-Host "actual_turnaround_min=$actualTurnaroundDisplay max_turnaround_min=$maxTurnaroundDisplay"
if ($actualTurnaroundMinutes -gt $MaxTurnaroundMinutes) {
    $warningMessage = "TURNAROUND LIMIT EXCEEDED: actual capture-run turnaround was $actualTurnaroundDisplay minutes, above -MaxTurnaroundMinutes $maxTurnaroundDisplay."
    Write-Warning $warningMessage
    Write-Host "WARNING: $warningMessage" -ForegroundColor Yellow
}
