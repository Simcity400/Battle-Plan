[CmdletBinding()]
param(
    [ValidateSet("Check", "Build", "Fast", "Integration", "Test", "Final", "Soak", "Evidence", "Release", "Stats")]
    [string]$Mode = "Final",

    [ValidateSet("All", "Extractor", "Replay")]
    [string]$Workspace = "All",

    [string[]]$Package = @(),

    [string]$FilterExpression = "",

    [switch]$Lib,

    [string[]]$Bin = @(),

    [string[]]$TestTarget = @(),

    [switch]$AllTargets,

    [ValidateSet("Iterative", "Reusable")]
    [string]$CacheMode = "Iterative",

    [switch]$Timings
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..\..")).Path
$targetDir = Join-Path $repoRoot ".cargo-target"
$timingReportDir = Join-Path $repoRoot ".cargo-timings"
$nextestConfig = Join-Path $repoRoot ".config\nextest.toml"
$mutex = [Threading.Mutex]::new($false, "Local\battle_plan_central_rust_validation")
$ownsMutex = $false

function Invoke-Checked {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Program,

        [Parameter(Mandatory = $true)]
        [string[]]$Arguments
    )

    Write-Host ("> {0} {1}" -f $Program, ($Arguments -join " "))
    & $Program @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "$Program exited with code $LASTEXITCODE"
    }
}

function Save-CargoTimingReport {
    param(
        [Parameter(Mandatory = $true)]
        [string]$WorkspaceName,

        [Parameter(Mandatory = $true)]
        [string]$Label
    )

    if (-not $Timings) {
        return
    }
    $source = Join-Path $targetDir "cargo-timings\cargo-timing.html"
    if (-not (Test-Path -LiteralPath $source)) {
        throw "Cargo completed with -Timings but produced no report at $source"
    }
    New-Item -ItemType Directory -Path $timingReportDir -Force | Out-Null
    $stamp = Get-Date -Format "yyyyMMdd-HHmmss-fff"
    $destination = Join-Path $timingReportDir ("{0}-{1}-{2}.html" -f $WorkspaceName.ToLowerInvariant(), $Label, $stamp)
    Copy-Item -LiteralPath $source -Destination $destination
    Write-Host "Timing report: $destination"
}

try {
    $ownsMutex = $mutex.WaitOne(0)
    if (-not $ownsMutex) {
        throw "Another centralized Rust validation process is already active."
    }

    $env:CARGO_TARGET_DIR = $targetDir
    if (Get-Command sccache -ErrorAction SilentlyContinue) {
        if (-not $env:SCCACHE_CACHE_SIZE) {
            $env:SCCACHE_CACHE_SIZE = "30G"
        }
        if (-not $env:SCCACHE_SERVER_PORT) {
            $env:SCCACHE_SERVER_PORT = "4227"
        }
    }
    if ($CacheMode -eq "Reusable") {
        if (-not (Get-Command sccache -ErrorAction SilentlyContinue)) {
            throw "Reusable cache mode requires sccache on PATH."
        }
        $env:RUSTC_WRAPPER = "sccache"
        $env:CARGO_INCREMENTAL = "0"
    }
    else {
        Remove-Item Env:RUSTC_WRAPPER -ErrorAction SilentlyContinue
        $env:CARGO_INCREMENTAL = "1"
    }
    if (Get-Command lld-link -ErrorAction SilentlyContinue) {
        $env:CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER = "lld-link.exe"
    }

    if ($Mode -eq "Stats") {
        Invoke-Checked -Program "sccache" -Arguments @("--show-stats")
        return
    }

    $hasSpecificTarget = $Lib -or $Bin.Count -gt 0 -or $TestTarget.Count -gt 0
    if ($AllTargets -and $hasSpecificTarget) {
        throw "-AllTargets cannot be combined with -Lib, -Bin, or -TestTarget."
    }
    if ($Mode -in @("Final", "Soak", "Release") -and ($AllTargets -or $hasSpecificTarget)) {
        throw "Final, Soak, and Release own their target sets; target selectors are supported by Check, Build, Fast, Integration, Test, and Evidence."
    }
    if ($Mode -eq "Soak" -and $Workspace -ne "Replay") {
        throw "Soak is a replay-only tier. Use -Workspace Replay."
    }
    if ($Mode -eq "Fast" -and $TestTarget.Count -gt 0) {
        throw "Fast runs unit-test binaries. Use Integration or Test with -TestTarget for integration tests."
    }
    if ($Mode -eq "Integration" -and ($Lib -or $Bin.Count -gt 0)) {
        throw "Integration runs integration-test binaries. Use Fast or Test with -Lib/-Bin for unit tests."
    }
    if ($Mode -in @("Fast", "Integration") -and $AllTargets) {
        throw "Fast and Integration are narrow tiers and do not accept -AllTargets. Use Test or Final for broader validation."
    }
    if ($Mode -eq "Evidence" -and -not ($FilterExpression -or $hasSpecificTarget -or $Package.Count -gt 0)) {
        throw "Evidence tests require an explicit -Package, -Lib, -Bin, -TestTarget, or -FilterExpression selection."
    }

    $targetSelection = @()
    if ($AllTargets) {
        $targetSelection += "--all-targets"
    }
    else {
        if ($Lib) {
            $targetSelection += "--lib"
        }
        foreach ($name in $Bin) {
            $targetSelection += @("--bin", $name)
        }
        foreach ($name in $TestTarget) {
            $targetSelection += @("--test", $name)
        }
    }
    $timingSelection = @()
    if ($Timings) {
        $timingSelection += "--timings"
    }

    $workspaceRoots = switch ($Workspace) {
        "Extractor" { @(Join-Path $repoRoot "extractor\native") }
        "Replay" { @(Join-Path $repoRoot "replay_interface\native") }
        default {
            @(
                (Join-Path $repoRoot "extractor\native"),
                (Join-Path $repoRoot "replay_interface\native")
            )
        }
    }

    foreach ($workspaceRoot in $workspaceRoots) {
        $workspaceName = if ($workspaceRoot -like "*\extractor\native") { "Extractor" } else { "Replay" }
        $nextestConfigSelection = if ($workspaceName -eq "Replay") { @("--config-file", $nextestConfig) } else { @() }
        $nextestCiProfileSelection = if ($workspaceName -eq "Replay") { @("--profile", "ci") } else { @() }
        Push-Location $workspaceRoot
        try {
            $selection = @()
            foreach ($name in $Package) {
                $selection += @("--package", $name)
            }
            if ($selection.Count -eq 0) {
                $selection = @("--workspace")
            }

            switch ($Mode) {
                "Check" {
                    Invoke-Checked -Program "cargo" -Arguments (@("check") + $timingSelection + $targetSelection + $selection)
                    Save-CargoTimingReport -WorkspaceName $workspaceName -Label "check"
                }
                "Build" {
                    Invoke-Checked -Program "cargo" -Arguments (@("build") + $timingSelection + $targetSelection + $selection)
                    Save-CargoTimingReport -WorkspaceName $workspaceName -Label "build"
                }
                "Test" {
                    $arguments = @("nextest", "run") + $nextestConfigSelection +
                        $timingSelection + $targetSelection + $selection
                    if ($FilterExpression) {
                        $arguments += @("-E", $FilterExpression)
                    }
                    Invoke-Checked -Program "cargo" -Arguments $arguments
                    Save-CargoTimingReport -WorkspaceName $workspaceName -Label "test"
                }
                "Fast" {
                    $tierFilter = "kind(lib) | kind(bin)"
                    if ($FilterExpression) {
                        $tierFilter = "($tierFilter) & ($FilterExpression)"
                    }
                    $arguments = @("nextest", "run") + $nextestConfigSelection +
                        @("-E", $tierFilter) + $timingSelection + $targetSelection + $selection
                    Invoke-Checked -Program "cargo" -Arguments $arguments
                    Save-CargoTimingReport -WorkspaceName $workspaceName -Label "fast"
                }
                "Integration" {
                    $tierFilter = "kind(test)"
                    if ($FilterExpression) {
                        $tierFilter = "($tierFilter) & ($FilterExpression)"
                    }
                    $arguments = @("nextest", "run") + $nextestConfigSelection +
                        @("-E", $tierFilter) + $timingSelection + $targetSelection + $selection
                    Invoke-Checked -Program "cargo" -Arguments $arguments
                    Save-CargoTimingReport -WorkspaceName $workspaceName -Label "integration"
                }
                "Soak" {
                    $arguments = @(
                        "nextest", "run", "--no-fail-fast",
                        "--config-file", $nextestConfig, "--profile", "soak",
                        "--package", "replay_app", "--test", "soak", "--test", "soak_f2",
                        "--test-threads", "1"
                    ) + $timingSelection
                    if ($FilterExpression) {
                        $arguments += @("-E", $FilterExpression)
                    }
                    Invoke-Checked -Program "cargo" -Arguments $arguments
                    Save-CargoTimingReport -WorkspaceName $workspaceName -Label "soak"
                }
                "Evidence" {
                    $evidenceProfileSelection = if ($workspaceName -eq "Replay") { @("--profile", "evidence") } else { @() }
                    $arguments = @("nextest", "run", "--no-fail-fast", "--run-ignored", "all") +
                        $nextestConfigSelection + $evidenceProfileSelection +
                        $timingSelection + $targetSelection + $selection
                    if ($FilterExpression) {
                        $arguments += @("-E", $FilterExpression)
                    }
                    Invoke-Checked -Program "cargo" -Arguments $arguments
                    Save-CargoTimingReport -WorkspaceName $workspaceName -Label "evidence"
                }
                "Final" {
                    $arguments = @("nextest", "run", "--all-targets", "--no-fail-fast") +
                        $nextestConfigSelection + $nextestCiProfileSelection +
                        $timingSelection + $selection
                    if ($FilterExpression) {
                        $arguments += @("-E", $FilterExpression)
                    }
                    Invoke-Checked -Program "cargo" -Arguments $arguments
                    Save-CargoTimingReport -WorkspaceName $workspaceName -Label "final-nextest"
                    Invoke-Checked -Program "cargo" -Arguments (@("test", "--doc", "--no-fail-fast") + $timingSelection + $selection)
                    Save-CargoTimingReport -WorkspaceName $workspaceName -Label "final-doctest"
                }
                "Release" {
                    Invoke-Checked -Program "cargo" -Arguments (@("build", "--release", "--all-targets") + $timingSelection + $selection)
                    Save-CargoTimingReport -WorkspaceName $workspaceName -Label "release-build"
                    $arguments = @(
                        "nextest", "run", "--all-targets", "--no-fail-fast",
                        "--cargo-profile", "release"
                    ) + $nextestConfigSelection + $nextestCiProfileSelection +
                        $timingSelection + $selection
                    if ($FilterExpression) {
                        $arguments += @("-E", $FilterExpression)
                    }
                    Invoke-Checked -Program "cargo" -Arguments $arguments
                    Save-CargoTimingReport -WorkspaceName $workspaceName -Label "release-test"
                }
            }
        }
        finally {
            Pop-Location
        }
    }
}
finally {
    if ($ownsMutex) {
        $mutex.ReleaseMutex()
    }
    $mutex.Dispose()
}
