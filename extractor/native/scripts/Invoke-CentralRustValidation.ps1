[CmdletBinding()]
param(
    [ValidateSet("Check", "Build", "Fast", "Integration", "Test", "Final", "Soak", "Evidence", "Release", "Lock", "Stats")]
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
$artifactAttestationHelper = Join-Path $PSScriptRoot "ProductionArtifact.Attestation.ps1"
$artifactAttestationPath = Join-Path $targetDir "release\hoi4-production-prebuilt-artifacts.json"
$portablePackageScript = Join-Path $PSScriptRoot "New-PortableProductPackage.ps1"
$portablePackageVerifier = Join-Path $PSScriptRoot "Test-PortableProductPackageReceipt.ps1"
$portableStaticCrtTest = Join-Path $PSScriptRoot "Test-PortableProductStaticCrt.ps1"
$portablePackageSelfTest = Join-Path $PSScriptRoot "Test-PortableProductPackageSelfTest.ps1"
$portablePackageReceiptPath = Join-Path $targetDir "release\hoi4-portable-package-receipt.json"
$expectedRustVersion = "1.96.0"
$expectedNextestVersion = "0.9.140"
. $artifactAttestationHelper
Add-Type -TypeDefinition @'
using System;
using System.ComponentModel;
using System.IO;
using System.Runtime.InteropServices;

public static class PortableReceiptPublishNative {
    private const uint MOVEFILE_WRITE_THROUGH = 0x00000008;

    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern bool MoveFileExW(string existingName, string newName, uint flags);

    public static void PublishNoReplaceWriteThrough(string temporary, string destination) {
        if (File.Exists(destination) || Directory.Exists(destination)) {
            throw new IOException("durable receipt publication refuses an existing destination: " + destination);
        }
        if (!MoveFileExW(temporary, destination, MOVEFILE_WRITE_THROUGH)) {
            throw new Win32Exception(Marshal.GetLastWin32Error(),
                "durable no-replace receipt publication failed: " + destination);
        }
    }
}
'@
$mutex = [Threading.Mutex]::new($false, "Local\battle_plan_central_rust_validation")
$ownsMutex = $false
$releaseInputDigestBefore = $null
$releaseBuildEnvironmentBefore = $null
$releaseArtifactFreshness = $null

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

function Test-CargoManifestHasLibraryTarget {
    param(
        [Parameter(Mandatory = $true)]
        [string]$ManifestPath
    )

    $manifest = Get-Content -LiteralPath $ManifestPath -Raw
    if ($manifest -match '(?m)^[ \t]*\[lib\][ \t]*(?:#.*)?$') {
        return $true
    }

    # Cargo implicitly creates a library target for src/lib.rs unless the
    # package explicitly disables automatic library discovery.
    $packageSection = [regex]::Match(
        $manifest,
        '(?ms)^[ \t]*\[package\][ \t]*(?:#.*)?\r?\n(?<body>.*?)(?=^[ \t]*\[|\z)'
    )
    $autolibDisabled = $packageSection.Success -and
        $packageSection.Groups['body'].Value -match '(?m)^[ \t]*autolib[ \t]*=[ \t]*false[ \t]*(?:#.*)?\r?$'
    $implicitLibrary = Join-Path (Split-Path -Parent $ManifestPath) 'src\lib.rs'
    return ((-not $autolibDisabled) -and (Test-Path -LiteralPath $implicitLibrary -PathType Leaf))
}

function Get-CargoPackageVersion {
    param(
        [Parameter(Mandatory = $true)]
        [string]$ManifestPath
    )

    $manifest = Get-Content -LiteralPath $ManifestPath -Raw
    $packageSection = [regex]::Match(
        $manifest,
        '(?ms)^[ \t]*\[package\][ \t]*(?:#.*)?\r?\n(?<body>.*?)(?=^[ \t]*\[|\z)'
    )
    if (-not $packageSection.Success) {
        throw "Cargo manifest has no [package] section: $ManifestPath"
    }
    $versionMatch = [regex]::Match(
        $packageSection.Groups['body'].Value,
        '(?m)^[ \t]*version[ \t]*=[ \t]*"(?<version>[0-9A-Za-z._-]+)"[ \t]*(?:#.*)?\r?$'
    )
    if (-not $versionMatch.Success) {
        throw "Cargo package version is absent or not portable-package-safe: $ManifestPath"
    }
    return $versionMatch.Groups['version'].Value
}

function Assert-PinnedRustToolchain {
    $rustcVersion = (& rustc --version 2>&1 | Out-String).Trim()
    if ($LASTEXITCODE -ne 0 -or $rustcVersion -notmatch "^rustc $([regex]::Escape($expectedRustVersion))(?:\s|$)") {
        throw "Pinned rustc $expectedRustVersion is required; observed '$rustcVersion'. Install the repository rust-toolchain.toml toolchain."
    }

    $cargoVersion = (& cargo --version 2>&1 | Out-String).Trim()
    if ($LASTEXITCODE -ne 0 -or $cargoVersion -notmatch "^cargo $([regex]::Escape($expectedRustVersion))(?:\s|$)") {
        throw "Pinned Cargo $expectedRustVersion is required; observed '$cargoVersion'."
    }

    $nextestVersion = (& cargo nextest --version 2>&1 | Out-String).Trim()
    if ($LASTEXITCODE -ne 0 -or $nextestVersion -notmatch "^cargo-nextest $([regex]::Escape($expectedNextestVersion))(?:\s|$)") {
        throw "Pinned cargo-nextest $expectedNextestVersion is required; observed '$nextestVersion'."
    }

    Write-Host "Pinned validation toolchain: rustc/cargo $expectedRustVersion; cargo-nextest $expectedNextestVersion"
}

function Enable-StaticMsvcCrtForCentralRelease {
    $verboseVersion = (& rustc -vV 2>&1 | Out-String)
    if ($LASTEXITCODE -ne 0 -or $verboseVersion -notmatch '(?m)^host:\s+x86_64-pc-windows-msvc\s*$') {
        throw "central Windows Release requires rustc host x86_64-pc-windows-msvc"
    }

    # Use Cargo's highest-precedence encoded form so an inherited RUSTFLAGS or
    # checkout-local setting cannot silently return product binaries to /MD.
    $separator = [char]0x1f
    $env:CARGO_ENCODED_RUSTFLAGS = @('-C', 'target-feature=+crt-static') -join $separator
    Remove-Item Env:RUSTFLAGS -ErrorAction SilentlyContinue
    $cfg = @(& rustc -C 'target-feature=+crt-static' --print cfg 2>&1)
    if ($LASTEXITCODE -ne 0 -or $cfg -cnotcontains 'target_env="msvc"' -or
        $cfg -cnotcontains 'target_feature="crt-static"') {
        throw "central Windows Release could not activate the static MSVC CRT target feature"
    }
    return [ordered]@{
        target = 'x86_64-pc-windows-msvc'
        rust_target_feature = '+crt-static'
        c_runtime = 'static'
        admission = 'pe_import_and_delay_import_tables'
        forbidden_import = 'VCRUNTIME*.dll'
    }
}

function Remove-ReleasePromotionState {
    param(
        [Parameter(Mandatory = $true)]
        [string[]]$Paths,

        [Parameter(Mandatory = $true)]
        [string]$Context
    )

    $problems = [System.Collections.Generic.List[string]]::new()
    foreach ($path in @($Paths | Select-Object -Unique)) {
        try {
            if (Test-Path -LiteralPath $path -ErrorAction Stop) {
                Remove-Item -LiteralPath $path -Force -ErrorAction Stop
            }
        }
        catch {
            $problems.Add("$path ($($_.Exception.Message))")
        }
    }
    foreach ($path in @($Paths | Select-Object -Unique)) {
        try {
            if (Test-Path -LiteralPath $path -ErrorAction Stop) {
                $problems.Add("$path (authoritative residue remains)")
            }
        }
        catch {
            $problems.Add("$path (absence could not be verified: $($_.Exception.Message))")
        }
    }
    if ($problems.Count -gt 0) {
        throw "$Context failed to remove and prove absence of release authority: $($problems -join '; ')"
    }
}

try {
    $ownsMutex = $mutex.WaitOne(0)
    if (-not $ownsMutex) {
        throw "Another centralized Rust validation process is already active."
    }

    $extractorReleaseRequested = $Mode -eq "Release" -and $Workspace -in @("All", "Extractor")
    $fullExtractorRelease = $Mode -eq "Release" -and $Workspace -eq "All" -and $Package.Count -eq 0 -and [string]::IsNullOrWhiteSpace($FilterExpression)
    if ($extractorReleaseRequested) {
        # Remove authority before any toolchain or release preflight can fail.
        # A ZIP without both canonical receipts is deliberately non-promotable.
        Remove-ReleasePromotionState `
            -Paths @($artifactAttestationPath, $portablePackageReceiptPath) `
            -Context 'stale release authority cleanup'
        $recorderVersion = Get-CargoPackageVersion -ManifestPath (Join-Path $repoRoot "extractor\native\recorder_host\Cargo.toml")
        $replayVersion = Get-CargoPackageVersion -ManifestPath (Join-Path $repoRoot "replay_interface\native\replay_app\Cargo.toml")
        $candidatePackagePaths = @(
            @($recorderVersion, $replayVersion) |
                Select-Object -Unique |
                ForEach-Object { Join-Path $targetDir ("release\battle-plan-{0}-windows-x86_64.zip" -f $_) }
        )
        Remove-ReleasePromotionState `
            -Paths $candidatePackagePaths `
            -Context 'stale release package cleanup'
        if ($recorderVersion -cne $replayVersion) {
            throw "product executable versions differ: recorder_host=$recorderVersion replay_app=$replayVersion"
        }
        $portablePackagePath = $candidatePackagePaths[0]
    }

    Assert-PinnedRustToolchain

    $releaseRuntimeLinkage = $null
    if ($Mode -eq 'Release') {
        $releaseRuntimeLinkage = Enable-StaticMsvcCrtForCentralRelease
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

    if ($Mode -in @('Final', 'Release')) {
        $staticCrtTestOutput = @(& powershell.exe -NoProfile -ExecutionPolicy Bypass -File $portableStaticCrtTest)
        if ($LASTEXITCODE -ne 0) {
            throw "portable-product static-CRT admission tests exited with code $LASTEXITCODE"
        }
        $staticCrtTestResult = (($staticCrtTestOutput -join [Environment]::NewLine).Trim() | ConvertFrom-Json)
        if ([string]$staticCrtTestResult.schema -cne 'battle-plan.portable_product_static_crt_test.v1' -or
            [string]$staticCrtTestResult.status -cne 'PASS' -or
            [int]$staticCrtTestResult.cases -ne 4) {
            throw "portable-product static-CRT admission tests returned no authority"
        }

        $packageSelfTestOutput = @(& powershell.exe -NoProfile -ExecutionPolicy Bypass -File $portablePackageSelfTest)
        if ($LASTEXITCODE -ne 0) {
            throw "portable-product package builder/verifier self-tests exited with code $LASTEXITCODE"
        }
        $packageSelfTestResult = (($packageSelfTestOutput -join [Environment]::NewLine).Trim() | ConvertFrom-Json)
        if ([string]$packageSelfTestResult.schema -cne 'battle-plan.portable_package_selftest.v1' -or
            [string]$packageSelfTestResult.status -cne 'PASS' -or
            [int]$packageSelfTestResult.cases -ne 11) {
            throw "portable-product package builder/verifier self-tests returned no authority"
        }
    }

    if ($fullExtractorRelease) {
        # Cargo does not delete outputs for targets removed or renamed since a
        # prior build. Prove that every canonical product path starts absent so
        # a successful current Release is the only possible producer.
        $releaseArtifactFreshness = Clear-ProductionArtifactOutputs -RepoRoot $repoRoot
        $releaseBuildEnvironmentBefore = Get-ProductionBuildEnvironment
        $releaseInputDigestBefore = Get-ProductionRustInputDigest -RepoRoot $repoRoot
    }

    $hasSpecificTarget = $Lib -or $Bin.Count -gt 0 -or $TestTarget.Count -gt 0
    if ($Mode -eq "Lock" -and
        ($Package.Count -gt 0 -or -not [string]::IsNullOrWhiteSpace($FilterExpression) -or
         $AllTargets -or $hasSpecificTarget -or $Timings)) {
        throw "Lock regenerates the selected top-level workspace lockfile and does not accept package, filter, target, or timing selectors."
    }
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
            if ($Mode -eq "Lock") {
                Invoke-Checked -Program "cargo" -Arguments @("generate-lockfile")
                continue
            }

            $selection = @("--locked")
            foreach ($name in $Package) {
                $selection += @("--package", $name)
            }
            if ($Package.Count -eq 0) {
                $selection += "--workspace"
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
                        "nextest", "run", "--locked", "--no-fail-fast",
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

    if ($Mode -eq "Lock" -and $Workspace -in @("All", "Extractor")) {
        $standaloneRoots = @(
            (Join-Path $repoRoot "extractor\native\project_progress_live_lane"),
            (Join-Path $repoRoot "extractor\native\lanes\occupation_resistance_compliance_live\replay_verify")
        )
        foreach ($standaloneRoot in $standaloneRoots) {
            Push-Location $standaloneRoot
            try {
                Invoke-Checked -Program "cargo" -Arguments @("generate-lockfile")
            }
            finally {
                Pop-Location
            }
        }
    }

    if ($Package.Count -eq 0 -and $Workspace -in @("All", "Extractor") -and $Mode -in @("Final", "Release")) {
        $standaloneRoots = @(
            (Join-Path $repoRoot "extractor\native\project_progress_live_lane"),
            (Join-Path $repoRoot "extractor\native\lanes\occupation_resistance_compliance_live\replay_verify")
        )
        foreach ($standaloneRoot in $standaloneRoots) {
            $standaloneManifest = Join-Path $standaloneRoot "Cargo.toml"
            Push-Location $standaloneRoot
            try {
                if ($Mode -eq "Final") {
                    Invoke-Checked -Program "cargo" -Arguments @("nextest", "run", "--locked", "--all-targets", "--no-fail-fast")
                    if (Test-CargoManifestHasLibraryTarget -ManifestPath $standaloneManifest) {
                        Invoke-Checked -Program "cargo" -Arguments @("test", "--locked", "--doc", "--no-fail-fast")
                    }
                    else {
                        Write-Host "Skipping standalone doctests: no library target in $standaloneManifest"
                    }
                }
                else {
                    Invoke-Checked -Program "cargo" -Arguments @("build", "--locked", "--release", "--all-targets")
                    Invoke-Checked -Program "cargo" -Arguments @("nextest", "run", "--locked", "--all-targets", "--no-fail-fast", "--cargo-profile", "release")
                }
            }
            finally {
                Pop-Location
            }
        }
    }

    if ($fullExtractorRelease) {
        $releaseBuildEnvironmentAfter = Get-ProductionBuildEnvironment
        if ([string]$releaseBuildEnvironmentBefore.sha256 -cne [string]$releaseBuildEnvironmentAfter.sha256) {
            throw "production build environment changed during centralized Release; no artifact attestation was emitted"
        }
        $releaseInputDigestAfter = Get-ProductionRustInputDigest -RepoRoot $repoRoot
        if ([string]$releaseInputDigestBefore.sha256 -cne [string]$releaseInputDigestAfter.sha256 -or
            [int]$releaseInputDigestBefore.file_count -ne [int]$releaseInputDigestAfter.file_count) {
            throw "production Rust inputs changed during centralized Release; no artifact attestation was emitted"
        }
        try {
            Write-ProductionArtifactAttestation `
                -RepoRoot $repoRoot `
                -ReceiptPath $artifactAttestationPath `
                -RequestedWorkspace $Workspace `
                -SourceDigest $releaseInputDigestAfter `
                -ReleaseRuntimeLinkage $releaseRuntimeLinkage `
                -BuildEnvironment $releaseBuildEnvironmentAfter `
                -ArtifactFreshness $releaseArtifactFreshness

            $captureDriver = Join-Path $repoRoot "extractor\native\scripts\Start-Hoi4ProductionCaptureSafe.ps1"
            $attested = Test-ProductionArtifactAttestation `
                -RepoRoot $repoRoot `
                -ReceiptPath $artifactAttestationPath `
                -ExpectedDriverPath $captureDriver `
                -ExpectedBuildEnvironment $releaseBuildEnvironmentAfter `
                -CurrentSourceDigest $releaseInputDigestAfter

            $packageBuilderHash = (Get-FileHash -LiteralPath $portablePackageScript -Algorithm SHA256).Hash.ToLowerInvariant()
            $packageStdout = @(& powershell.exe -NoProfile -ExecutionPolicy Bypass `
                -File $portablePackageScript `
                -RecorderExe ([string]$attested.artifacts['recorder_host']) `
                -ReplayExe ([string]$attested.artifacts['replay_app']) `
                -CollectorDll ([string]$attested.artifacts['collector_dll']) `
                -OutputZip $portablePackagePath `
                -Version $recorderVersion)
            if ($LASTEXITCODE -ne 0) {
                throw "portable product package builder exited with code $LASTEXITCODE"
            }
            $packageResult = (($packageStdout -join [Environment]::NewLine).Trim() | ConvertFrom-Json)
            $actualPackagePath = [System.IO.Path]::GetFullPath([string]$packageResult.path)
            $expectedPackagePath = [System.IO.Path]::GetFullPath($portablePackagePath)
            if ([string]$packageResult.schema -cne 'hoi4-portable-package.v1' -or
                [string]$packageResult.version -cne $recorderVersion -or
                $actualPackagePath -ine $expectedPackagePath -or
                [string]$packageResult.sha256 -cne (Get-FileHash -LiteralPath $expectedPackagePath -Algorithm SHA256).Hash.ToLowerInvariant() -or
                [string]$packageResult.input_sha256.recorder_host -cne [string]$attested.artifact_sha256['recorder_host'] -or
                [string]$packageResult.input_sha256.replay_app -cne [string]$attested.artifact_sha256['replay_app'] -or
                [string]$packageResult.input_sha256.collector_dll -cne [string]$attested.artifact_sha256['collector_dll'] -or
                $packageResult.static_vcruntime_admission -isnot [bool] -or
                -not $packageResult.static_vcruntime_admission -or
                [int]$packageResult.executable_count -ne 2 -or
                $packageResult.signed -isnot [bool] -or $packageResult.signed -or
                [string]$packageResult.install_kind -cne 'portable_zip') {
                throw "portable product package result failed the centralized Release contract"
            }
            foreach ($role in @('recorder_host', 'replay_app', 'collector_dll')) {
                [void](Assert-PortableProductPeImportAttestation `
                    -Bytes ([System.IO.File]::ReadAllBytes([string]$attested.artifacts[$role])) `
                    -Name $role `
                    -Attestation $packageResult.pe_imports.$role)
                [void](Assert-PortableProductPeImportAttestation `
                    -Bytes ([System.IO.File]::ReadAllBytes([string]$attested.artifacts[$role])) `
                    -Name $role `
                    -Attestation $attested.artifact_pe_imports[$role])
            }
            $releaseInputDigestPackaged = Get-ProductionRustInputDigest -RepoRoot $repoRoot
            if ([string]$releaseInputDigestPackaged.sha256 -cne [string]$releaseInputDigestAfter.sha256 -or
                [int]$releaseInputDigestPackaged.file_count -ne [int]$releaseInputDigestAfter.file_count -or
                $packageBuilderHash -cne (Get-FileHash -LiteralPath $portablePackageScript -Algorithm SHA256).Hash.ToLowerInvariant()) {
                throw "production inputs changed during portable packaging"
            }

            $packageReceipt = [ordered]@{
                schema = 'battle-plan.central_portable_package.v1'
                status = 'PASS_CENTRAL_RELEASE_PORTABLE_PACKAGE'
                successful_exit = $true
                central_validation = [ordered]@{
                    mode = 'Release'
                    requested_workspace = 'All'
                    release_runtime_linkage = $releaseRuntimeLinkage
                }
                artifact_attestation = [ordered]@{
                    path = [string]$attested.receipt_path
                    sha256 = [string]$attested.receipt_sha256
                }
                package_builder = [ordered]@{
                    path = [System.IO.Path]::GetFullPath($portablePackageScript)
                    sha256 = $packageBuilderHash
                }
                package = $packageResult
                source_input_sha256 = [string]$attested.source_input_sha256
                build_environment = $attested.build_environment
            }
            $packageReceiptCandidate = "$portablePackageReceiptPath.$([guid]::NewGuid().ToString('N')).candidate"
            try {
                [System.IO.File]::WriteAllText(
                    $packageReceiptCandidate,
                    ($packageReceipt | ConvertTo-Json -Depth 12),
                    [System.Text.UTF8Encoding]::new($false)
                )
                $packageReceiptStream = [System.IO.File]::Open(
                    $packageReceiptCandidate,
                    [System.IO.FileMode]::Open,
                    [System.IO.FileAccess]::ReadWrite,
                    [System.IO.FileShare]::None
                )
                try { $packageReceiptStream.Flush($true) } finally { $packageReceiptStream.Dispose() }
                if ((Get-FileHash -LiteralPath $expectedPackagePath -Algorithm SHA256).Hash.ToLowerInvariant() -cne [string]$packageResult.sha256 -or
                    (Get-FileHash -LiteralPath $artifactAttestationPath -Algorithm SHA256).Hash.ToLowerInvariant() -cne [string]$attested.receipt_sha256 -or
                    (Get-FileHash -LiteralPath $portablePackageScript -Algorithm SHA256).Hash.ToLowerInvariant() -cne $packageBuilderHash) {
                    throw "portable package promotion inputs changed before receipt publication"
                }
                $candidateReceiptHash = (Get-FileHash -LiteralPath $packageReceiptCandidate -Algorithm SHA256).Hash.ToLowerInvariant()
                $verificationStdout = @(& powershell.exe -NoProfile -ExecutionPolicy Bypass `
                    -File $portablePackageVerifier `
                    -RepoRoot $repoRoot `
                    -ReceiptPath $packageReceiptCandidate `
                    -AllowStagedReceipt)
                if ($LASTEXITCODE -ne 0) {
                    throw "portable package receipt verifier exited with code $LASTEXITCODE"
                }
                $verificationResult = (($verificationStdout -join [Environment]::NewLine).Trim() | ConvertFrom-Json)
                if ([string]$verificationResult.schema -cne 'battle-plan.central_portable_package_verification.v1' -or
                    [string]$verificationResult.status -cne 'PASS_STAGED_CENTRAL_RELEASE_PORTABLE_PACKAGE' -or
                    $verificationResult.canonical_receipt -isnot [bool] -or $verificationResult.canonical_receipt -or
                    $verificationResult.static_vcruntime_admission -isnot [bool] -or
                    -not $verificationResult.static_vcruntime_admission -or
                    [string]$verificationResult.receipt_sha256 -cne $candidateReceiptHash -or
                    [string]$verificationResult.package_sha256 -cne [string]$packageResult.sha256 -or
                    [string]$verificationResult.build_environment_sha256 -cne [string]$releaseBuildEnvironmentAfter.sha256) {
                    throw "portable package receipt verifier returned no promotion authority"
                }
                if ((Get-FileHash -LiteralPath $packageReceiptCandidate -Algorithm SHA256).Hash.ToLowerInvariant() -cne $candidateReceiptHash -or
                    (Get-FileHash -LiteralPath $expectedPackagePath -Algorithm SHA256).Hash.ToLowerInvariant() -cne [string]$packageResult.sha256 -or
                    (Get-FileHash -LiteralPath $artifactAttestationPath -Algorithm SHA256).Hash.ToLowerInvariant() -cne [string]$attested.receipt_sha256 -or
                    (Get-FileHash -LiteralPath $portablePackageScript -Algorithm SHA256).Hash.ToLowerInvariant() -cne $packageBuilderHash) {
                    throw "portable package promotion inputs changed after independent receipt verification"
                }
                Write-Host "Production artifact attestation: $artifactAttestationPath"
                Write-Host "Portable product package: $portablePackagePath"
                Write-Host "Publishing verified portable package receipt: $portablePackageReceiptPath"
                # This atomic, no-replace, write-through same-directory move is
                # deliberately terminal. It durably commits the parent namespace
                # only after the independent verifier accepts the candidate.
                [PortableReceiptPublishNative]::PublishNoReplaceWriteThrough(
                    $packageReceiptCandidate,
                    $portablePackageReceiptPath
                )
            }
            catch {
                $candidateFailure = $_
                try {
                    Remove-ReleasePromotionState `
                        -Paths @($packageReceiptCandidate) `
                        -Context 'staged receipt cleanup'
                }
                catch {
                    throw "portable receipt staging failed ($($candidateFailure.Exception.Message)); candidate cleanup also failed closed ($($_.Exception.Message))"
                }
                throw $candidateFailure
            }
        }
        catch {
            $releaseFailure = $_
            try {
                Remove-ReleasePromotionState `
                    -Paths @($artifactAttestationPath, $portablePackageReceiptPath, $portablePackagePath) `
                    -Context 'failed release cleanup'
            }
            catch {
                throw "centralized Release failed ($($releaseFailure.Exception.Message)); cleanup also failed closed ($($_.Exception.Message))"
            }
            throw $releaseFailure
        }
    }
}
finally {
    if ($ownsMutex) {
        $mutex.ReleaseMutex()
    }
    $mutex.Dispose()
}
