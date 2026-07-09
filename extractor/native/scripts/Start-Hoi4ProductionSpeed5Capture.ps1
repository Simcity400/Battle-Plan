param(
    [ValidateSet("Smoke", "ShortSpeed5", "FullGame")]
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

Assert-File -Path $staticTemplatesEquipmentCatalog -Label "templates/equipment static catalog"
Assert-File -Path $staticLocalisationJoin -Label "localisation static join"
Assert-File -Path $staticDecisionsDefinitions -Label "decisions static catalog"
Assert-File -Path $staticIdeaTechLocXref -Label "idea/tech localisation xref"
Assert-File -Path $staticShipDefinitions -Label "ship definitions static catalog"
Assert-File -Path $staticGfxIconManifest -Label "GFX icon manifest"

if ($Hoi4Pid -eq 0) {
    $Hoi4Pid = Resolve-SingleHoi4Pid
}

if ($DurationSeconds -le 0) {
    switch ($Mode) {
        "Smoke" { $DurationSeconds = 90 }
        "ShortSpeed5" { $DurationSeconds = 300 }
        "FullGame" { $DurationSeconds = 21600 }
    }
}

if ($MinGameHours -lt 0) {
    switch ($Mode) {
        "Smoke" { $MinGameHours = 0 }
        "ShortSpeed5" { $MinGameHours = 48 }
        "FullGame" { $MinGameHours = 48 }
    }
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
    "--stream-format", "compact",
    "--extraction-perf-gate",
    "--production",
    "--compact-country-frame",
    "--reader-concurrency", "$ReaderConcurrency",
    "--session-id", $SessionId,
    "--static-templates-equipment-catalog", $staticTemplatesEquipmentCatalog,
    "--static-localisation-join", $staticLocalisationJoin,
    "--static-decisions-definitions", $staticDecisionsDefinitions,
    "--static-idea-tech-loc-xref", $staticIdeaTechLocXref,
    "--static-ship-definitions", $staticShipDefinitions,
    "--static-gfx-icon-manifest", $staticGfxIconManifest
)

$commandLine = "& `"$collectorRecord`" " + ($runArgs | ForEach-Object {
    if ($_ -match "\s") { "`"$_`"" } else { $_ }
}) -join " "

$commandPath = Join-Path $OutDir "production_command.ps1"
Set-Content -LiteralPath $commandPath -Value $commandLine -Encoding UTF8

Write-Host "mode=$Mode pid=$Hoi4Pid duration_s=$durationArg min_game_hours=$MinGameHours sample_ms=$SampleMs reader_concurrency=$ReaderConcurrency"
Write-Host "out_dir=$OutDir"
Write-Host "command_file=$commandPath"

if ($DryRun) {
    Write-Host "DRY RUN capture: $commandLine"
    exit 0
}

$logPath = Join-Path $OutDir "collector_record.console.log"
Invoke-Checked -Exe $collectorRecord -Arguments $runArgs -LogPath $logPath
