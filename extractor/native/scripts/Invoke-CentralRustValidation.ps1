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

    [switch]$Timings,

    # A full Release without this path emits only fresh candidate artifacts for
    # a real capture. Supplying the resulting production receipt on a second
    # full Release is the only route to a portable package/production receipt.
    [string]$ProductionCaptureReceiptPath = ""
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
$productionInteractionClosureTest = Join-Path $PSScriptRoot "Test-ProductionInteractionClosure.ps1"
$passiveCapturePolicyTest = Join-Path $PSScriptRoot "Test-PassiveCapturePolicy.ps1"
$readOnlyGameStateWitness = Join-Path $PSScriptRoot "Read-Hoi4GameState.ps1"
$passiveCaptureLifecycle = Join-Path $PSScriptRoot "PassiveCapture.Lifecycle.ps1"
$interactionClosureHelper = Join-Path $PSScriptRoot "PortableProduct.InteractionClosure.ps1"
$productionCapturePromotionHelper = Join-Path $PSScriptRoot "ProductionCapture.Promotion.ps1"
. $interactionClosureHelper
. $productionCapturePromotionHelper
$productionSourceCapabilityPaths = [ordered]@{
    recorder_host_cargo_sha256 = Join-Path $repoRoot 'extractor\native\recorder_host\Cargo.toml'
    recorder_host_main_sha256 = Join-Path $repoRoot 'extractor\native\recorder_host\src\main.rs'
    recorder_host_early_render_launch_sha256 = Join-Path $repoRoot 'extractor\native\recorder_host\src\early_render_launch.rs'
    recorder_host_auto_attach_sha256 = Join-Path $repoRoot 'extractor\native\recorder_host\src\auto_attach.rs'
    extractor_workspace_cargo_sha256 = Join-Path $repoRoot 'extractor\native\Cargo.toml'
    collector_cargo_sha256 = Join-Path $repoRoot 'extractor\native\collector\Cargo.toml'
    character_lane_cargo_sha256 = Join-Path $repoRoot 'extractor\native\lanes\character_presentation_live\Cargo.toml'
    focus_lane_cargo_sha256 = Join-Path $repoRoot 'extractor\native\lanes\focus_research_decisions_depth_live\Cargo.toml'
    collector_lib_sha256 = Join-Path $repoRoot 'extractor\native\collector\src\lib.rs'
    passive_ready_contract_sha256 = Join-Path $repoRoot 'extractor\native\compact_contract\src\passive_ready.rs'
    interaction_closure_helper_sha256 = $interactionClosureHelper
    production_capture_promotion_helper_sha256 = $productionCapturePromotionHelper
}
$portablePackageReceiptPath = Join-Path $targetDir "release\hoi4-portable-package-receipt.json"
$worldFoldCitationVerifier = Join-Path $PSScriptRoot "verify_world_fold_citations.py"
$worldFoldCitationTests = Join-Path $PSScriptRoot "test_verify_world_fold_citations.py"
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

function Get-CurrentInteractionPolicyInputs {
    $inputs = [ordered]@{
        read_only_game_state_witness_sha256 = (Get-FileHash -LiteralPath $readOnlyGameStateWitness -Algorithm SHA256).Hash.ToLowerInvariant()
        passive_capture_policy_test_sha256 = (Get-FileHash -LiteralPath $passiveCapturePolicyTest -Algorithm SHA256).Hash.ToLowerInvariant()
        passive_capture_lifecycle_sha256 = (Get-FileHash -LiteralPath $passiveCaptureLifecycle -Algorithm SHA256).Hash.ToLowerInvariant()
    }
    foreach ($entry in $productionSourceCapabilityPaths.GetEnumerator()) {
        $inputs[$entry.Key] = (Get-FileHash -LiteralPath $entry.Value -Algorithm SHA256).Hash.ToLowerInvariant()
    }
    $sourceTree = Get-ProductionInteractionSourceTreeAuthority -RepoRoot $repoRoot
    $inputs['production_interaction_source_tree_file_count'] = [int]$sourceTree.file_count
    $inputs['production_interaction_source_tree_sha256'] = [string]$sourceTree.sha256
    return $inputs
}

function Get-CurrentProductionCaptureEvidence {
    if ([string]::IsNullOrWhiteSpace($ProductionCaptureReceiptPath)) {
        throw 'production capture evidence was not supplied'
    }
    return Get-ProductionCapturePromotionEvidence `
        -RepoRoot $repoRoot `
        -CaptureReceiptPath $ProductionCaptureReceiptPath `
        -CurrentArtifactAttestationPath $artifactAttestationPath
}

function Get-CurrentPinnedHookSourceAuthority {
    $path = [string]$productionSourceCapabilityPaths.passive_ready_contract_sha256
    $source = Get-Content -LiteralPath $path -Raw
    $rosterDeclaration =
        'pub const PINNED_PRODUCTION_HOOK_SPECS_V1: &[PinnedProductionHookSpecV1] = &['
    $countDeclaration =
        'pub const PINNED_PRODUCTION_HOOK_COUNT_V1: u16 = PINNED_PRODUCTION_HOOK_SPECS_V1.len() as u16;'
    $rosterStart = $source.IndexOf($rosterDeclaration, [StringComparison]::Ordinal)
    $rosterBodyStart = $rosterStart + $rosterDeclaration.Length
    $rosterEnd = if ($rosterStart -ge 0) {
        $source.IndexOf('];', $rosterBodyStart, [StringComparison]::Ordinal)
    }
    else { -1 }
    if ($rosterStart -lt 0 -or -not $source.Contains($countDeclaration) -or
        $rosterEnd -le $rosterBodyStart) {
        throw 'passive-ready source lacks a bounded slice roster and derived count declaration'
    }
    $rosterBody = $source.Substring($rosterBodyStart, $rosterEnd - $rosterBodyStart)
    $count = [regex]::Matches(
        $rosterBody,
        '(?m)^\s*pinned_(?:import|rva)!\('
    ).Count
    if ($count -le 0 -or $count -gt [uint16]::MaxValue) {
        throw 'passive-ready source has an invalid mechanically counted pinned hook roster'
    }
    return [ordered]@{
        count = [int][uint16]$count
        source_sha256 = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
    }
}

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

    if ($Workspace -in @("All", "Extractor") -and
        $Mode -in @("Check", "Build", "Fast", "Integration", "Test", "Final", "Release")) {
        # These source checks run before Cargo and, for Release, before stale
        # promotion authority is removed. A changed shipping fold must first
        # force an explicit review of every wire-format transcription it backs.
        Invoke-Checked -Program "python" -Arguments @($worldFoldCitationTests)
        Invoke-Checked -Program "python" -Arguments @(
            $worldFoldCitationVerifier,
            "--repo-root",
            $repoRoot
        )
    }

    $extractorReleaseRequested = $Mode -eq "Release" -and $Workspace -in @("All", "Extractor")
    $fullExtractorRelease = $Mode -eq "Release" -and $Workspace -eq "All" -and $Package.Count -eq 0 -and [string]::IsNullOrWhiteSpace($FilterExpression)
    if (-not [string]::IsNullOrWhiteSpace($ProductionCaptureReceiptPath) -and
        -not $fullExtractorRelease) {
        throw 'production capture evidence is accepted only by an unfiltered All-workspace Release'
    }
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
        $interactionClosureOutput = @(& powershell.exe -NoProfile -ExecutionPolicy Bypass -File $productionInteractionClosureTest)
        if ($LASTEXITCODE -ne 0) {
            throw "production interaction-closure source tests exited with code $LASTEXITCODE"
        }
        $interactionClosureResult = (($interactionClosureOutput -join [Environment]::NewLine).Trim() | ConvertFrom-Json)
        if ([string]$interactionClosureResult.schema -cne 'battle-plan.production_interaction_closure_test.v1' -or
            [string]$interactionClosureResult.status -cne 'PASS' -or
            [int]$interactionClosureResult.cases -ne 30 -or
            [string]$interactionClosureResult.policy -cne 'passive_injected_observer') {
            throw "production interaction-closure source tests returned no authority"
        }

        $passivePolicyOutput = @(& powershell.exe -NoProfile -ExecutionPolicy Bypass -File $passiveCapturePolicyTest)
        if ($LASTEXITCODE -ne 0) {
            throw "passive capture-policy source tests exited with code $LASTEXITCODE"
        }
        $passivePolicyResult = (($passivePolicyOutput -join [Environment]::NewLine).Trim() | ConvertFrom-Json)
        if ([string]$passivePolicyResult.Schema -cne 'hoi4-passive-capture-policy-test-v1' -or
            [string]$passivePolicyResult.Status -cne 'pass') {
            throw "passive capture-policy source tests returned no authority"
        }

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
            # 61 + 5. The five are the three pinned manifest-graph cases, the
            # case that the derived fixture reaches exactly the delegating unit
            # closure the committed baseline accepts, and the case that a
            # drifted accepted-inventory pin is rejected.
            [int]$packageSelfTestResult.cases -ne 66) {
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
    $featureSelection = @()
    $evidenceClientDrivingRequested = $Bin -ccontains 'collector_record_evidence'
    if ($evidenceClientDrivingRequested) {
        if ($Mode -cne 'Evidence' -or $Workspace -cne 'Extractor' -or
            $Package.Count -ne 1 -or $Package[0] -cne 'recorder_host') {
            throw "collector_record_evidence is isolated to explicit Evidence -Workspace Extractor -Package recorder_host"
        }
        $featureSelection = @('--features', 'recorder_host/evidence-client-driving')
    }
    elseif ($Mode -cne 'Evidence' -and @($Bin | Where-Object { $_ -like '*evidence*' }).Count -gt 0) {
        throw "evidence binaries are accepted only by the explicit Evidence tier"
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
                        $timingSelection + $featureSelection + $targetSelection + $selection
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
                    if ($workspaceName -ceq 'Extractor') {
                        # Rebuild both shipped injection components after every
                        # all-target Release build/test. The canonical recorder
                        # has an explicit empty feature set; the collector cdylib
                        # is rebuilt last with both default and unshipped
                        # evidence features disabled. Target-cache ordering can
                        # therefore never select an all-target/evidence variant
                        # as a package input.
                        Invoke-Checked -Program "cargo" -Arguments @(
                            "build", "--locked", "--release",
                            "--package", "recorder_host",
                            "--bin", "collector_record",
                            "--no-default-features"
                        )
                        Invoke-Checked -Program "cargo" -Arguments @(
                            "build", "--locked", "--release",
                            "--package", "collector",
                            "--lib",
                            "--no-default-features"
                        )
                    }
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

            if ([string]::IsNullOrWhiteSpace($ProductionCaptureReceiptPath)) {
                Write-Host "Candidate production artifacts: $artifactAttestationPath"
                Write-Host "No portable package was emitted: rerun this exact full Release with -ProductionCaptureReceiptPath after a production capture passes."
                return
            }

            $productionCaptureEvidence = Get-CurrentProductionCaptureEvidence

            $packageBuilderHash = (Get-FileHash -LiteralPath $portablePackageScript -Algorithm SHA256).Hash.ToLowerInvariant()
            $packageStdout = @(& powershell.exe -NoProfile -ExecutionPolicy Bypass `
                -File $portablePackageScript `
                -RecorderExe ([string]$attested.artifacts['recorder_host']) `
                -ReplayExe ([string]$attested.artifacts['replay_app']) `
                -CollectorDll ([string]$attested.artifacts['collector_dll']) `
                -OutputZip $portablePackagePath `
                -Version $recorderVersion `
                -ProductionCaptureReceiptPath $ProductionCaptureReceiptPath `
                -ProductionArtifactAttestationPath $artifactAttestationPath)
            if ($LASTEXITCODE -ne 0) {
                throw "portable product package builder exited with code $LASTEXITCODE"
            }
            $packageResult = (($packageStdout -join [Environment]::NewLine).Trim() | ConvertFrom-Json)
            $currentInteractionPolicyInputs = Get-CurrentInteractionPolicyInputs
            $currentPinnedHookAuthority = Get-CurrentPinnedHookSourceAuthority
            $actualPackagePath = [System.IO.Path]::GetFullPath([string]$packageResult.path)
            $expectedPackagePath = [System.IO.Path]::GetFullPath($portablePackagePath)
            if ([string]$packageResult.schema -cne 'hoi4-portable-package.v1' -or
                [string]$packageResult.version -cne $recorderVersion -or
                $actualPackagePath -ine $expectedPackagePath -or
                [string]$packageResult.sha256 -cne (Get-FileHash -LiteralPath $expectedPackagePath -Algorithm SHA256).Hash.ToLowerInvariant() -or
                [string]$packageResult.input_sha256.recorder_host -cne [string]$attested.artifact_sha256['recorder_host'] -or
                [string]$packageResult.input_sha256.replay_app -cne [string]$attested.artifact_sha256['replay_app'] -or
                [string]$packageResult.input_sha256.collector_dll -cne [string]$attested.artifact_sha256['collector_dll'] -or
                $packageResult.admissible_for_production -isnot [bool] -or
                -not $packageResult.admissible_for_production -or
                (($packageResult.production_capture_evidence | ConvertTo-Json -Depth 12 -Compress) -cne
                    ($productionCaptureEvidence | ConvertTo-Json -Depth 12 -Compress)) -or
                [string]$packageResult.interaction_closure.policy -cne 'passive_injected_observer' -or
                $packageResult.interaction_closure.zip_entry_bytes_reverified -isnot [bool] -or
                -not $packageResult.interaction_closure.zip_entry_bytes_reverified -or
                ($packageResult.interaction_closure.policy_inputs | ConvertTo-Json -Compress) -cne
                    ($currentInteractionPolicyInputs | ConvertTo-Json -Compress) -or
                [string]$packageResult.interaction_closure.source_capability.schema -cne
                    'hoi4.production-source-capability-closure.v1' -or
                -not $packageResult.interaction_closure.source_capability.canonical_observer_feature_isolated -or
                -not $packageResult.interaction_closure.source_capability.target_lifecycle_actuators_evidence_only -or
                -not $packageResult.interaction_closure.source_capability.production_auto_attach_target_control_absent -or
                -not $packageResult.interaction_closure.source_capability.collector_fixture_hooks_default_off -or
                -not $packageResult.interaction_closure.source_capability.collector_cdylib_explicit_empty_features -or
                -not $packageResult.interaction_closure.source_capability.structural_dependency_feature_parse -or
                -not $packageResult.interaction_closure.source_capability.production_source_tree_closed -or
                -not $packageResult.interaction_closure.source_capability.canonical_hook_name_families_closed -or
                [int]$packageResult.interaction_closure.source_capability.exact_pinned_hook_count -ne
                    [int]$currentPinnedHookAuthority.count -or
                [string]$packageResult.interaction_closure.source_capability.pinned_hook_contract_source_sha256 -cne
                    [string]$currentPinnedHookAuthority.source_sha256 -or
                [string]$packageResult.interaction_closure.source_capability.exact_target_executable_sha256 -cne
                    '870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad' -or
                [string]$packageResult.interaction_closure.recorder_host.schema -cne 'hoi4.interaction-closure.v1' -or
                [string]$packageResult.interaction_closure.replay_app.schema -cne 'hoi4.interaction-closure.v1' -or
                [string]$packageResult.interaction_closure.collector_dll.schema -cne 'hoi4.interaction-closure.v1' -or
                [int]$packageResult.interaction_closure.recorder_host.exact_pinned_hook_count -ne
                    [int]$currentPinnedHookAuthority.count -or
                [int]$packageResult.interaction_closure.collector_dll.exact_pinned_hook_count -ne
                    [int]$currentPinnedHookAuthority.count -or
                [int]$packageResult.interaction_closure.replay_app.exact_pinned_hook_count -ne
                    [int]$currentPinnedHookAuthority.count -or
                [string]$packageResult.interaction_closure.recorder_host.exact_pinned_hook_contract -cne
                    'a729.passive_ready.pinned_production_hook_specs.v1' -or
                [string]$packageResult.interaction_closure.collector_dll.exact_pinned_hook_contract -cne
                    'a729.passive_ready.pinned_production_hook_specs.v1' -or
                [string]$packageResult.interaction_closure.recorder_host.pinned_hook_contract_source_sha256 -cne
                    [string]$currentPinnedHookAuthority.source_sha256 -or
                [string]$packageResult.interaction_closure.collector_dll.pinned_hook_contract_source_sha256 -cne
                    [string]$currentPinnedHookAuthority.source_sha256 -or
                (@($packageResult.interaction_closure.recorder_host.permitted_injection_capabilities) -join '|') -cne
                    'OpenProcess|VirtualAllocEx|VirtualFreeEx|WriteProcessMemory|CreateRemoteThread|LoadLibraryW|FreeLibrary' -or
                (@($packageResult.interaction_closure.recorder_host.permitted_self_ui_capabilities) -join '|') -cne
                    'SendMessageW|ShellExecuteW|EnableWindow|SetWindowTextW|ShowWindow|DestroyWindow' -or
                (@($packageResult.interaction_closure.replay_app.permitted_self_ui_capabilities) -join '|') -cne
                    'SetWindowTextW|DestroyWindow' -or
                @($packageResult.interaction_closure.collector_dll.permitted_self_ui_capabilities).Count -ne 0 -or
                (@($packageResult.interaction_closure.replay_app.permitted_self_worker_capabilities) -join '|') -cne
                    'CreateProcessW|TerminateProcess|CreateJobObjectW|AssignProcessToJobObject|SetInformationJobObject' -or
                @($packageResult.interaction_closure.recorder_host.permitted_self_worker_capabilities).Count -ne 0 -or
                @($packageResult.interaction_closure.collector_dll.permitted_self_worker_capabilities).Count -ne 0 -or
                @($packageResult.interaction_closure.replay_app.permitted_injection_capabilities).Count -ne 0 -or
                @($packageResult.interaction_closure.collector_dll.permitted_injection_capabilities).Count -ne 0 -or
                -not $packageResult.interaction_closure.recorder_host.forbidden_symbols_absent -or
                -not $packageResult.interaction_closure.replay_app.forbidden_symbols_absent -or
                -not $packageResult.interaction_closure.collector_dll.forbidden_symbols_absent -or
                $packageResult.interaction_closure.recorder_host.semantic_engine_state_writes -or
                $packageResult.interaction_closure.replay_app.semantic_engine_state_writes -or
                $packageResult.interaction_closure.collector_dll.semantic_engine_state_writes -or
                # Target-process lifecycle control and game-UI control are not
                # separate receipt verdicts to read here. Both rosters are
                # subsets of the interaction-marker roster the package enforcers
                # throw on, and a permitted role capability is filtered out of
                # both, so a build that produced a receipt at all has already
                # rejected every symbol either verdict could report.
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
                    production_feature_closure = [ordered]@{
                        schema = 'battle-plan.production_feature_closure.v1'
                        rebuild_order = @('recorder_host', 'collector_dll')
                        recorder_host = [ordered]@{
                            package = 'recorder_host'
                            target = 'collector_record'
                            default_features = $false
                            enabled_features = @()
                            artifact_sha256 = [string]$attested.artifact_sha256['recorder_host']
                        }
                        collector_dll = [ordered]@{
                            package = 'collector'
                            target = 'lib'
                            crate_types = @('cdylib', 'rlib')
                            default_features = $false
                            enabled_features = @()
                            all_non_default_features_disabled = $true
                            forbidden_features = @(
                                'test-support',
                                'unshipped-a25-runtime-text-evidence',
                                'unshipped-a39-character-presentation-evidence',
                                'unshipped-a31-focus-presentation-evidence'
                            )
                            built_last = $true
                            collector_cargo_sha256 =
                                [string]$packageResult.interaction_closure.policy_inputs.collector_cargo_sha256
                            pinned_hook_count = [int]$currentPinnedHookAuthority.count
                            pinned_hook_contract_source_sha256 =
                                [string]$currentPinnedHookAuthority.source_sha256
                            artifact_sha256 = [string]$attested.artifact_sha256['collector_dll']
                        }
                    }
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
                $preVerificationSourceDigest = Get-ProductionRustInputDigest -RepoRoot $repoRoot
                if ([string]$preVerificationSourceDigest.sha256 -cne [string]$releaseInputDigestAfter.sha256 -or
                    [int]$preVerificationSourceDigest.file_count -ne [int]$releaseInputDigestAfter.file_count -or
                    ((Get-CurrentInteractionPolicyInputs) | ConvertTo-Json -Compress) -cne
                        ($packageResult.interaction_closure.policy_inputs | ConvertTo-Json -Compress) -or
                    (Get-FileHash -LiteralPath $expectedPackagePath -Algorithm SHA256).Hash.ToLowerInvariant() -cne [string]$packageResult.sha256 -or
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
                    [string]$verificationResult.interaction_closure.policy -cne 'passive_injected_observer' -or
                    $verificationResult.interaction_closure.zip_entry_bytes_reverified -isnot [bool] -or
                    -not $verificationResult.interaction_closure.zip_entry_bytes_reverified -or
                    ($verificationResult.interaction_closure.policy_inputs | ConvertTo-Json -Compress) -cne
                        ((Get-CurrentInteractionPolicyInputs) | ConvertTo-Json -Compress) -or
                    ($verificationResult.interaction_closure.source_capability | ConvertTo-Json -Compress) -cne
                        ($packageResult.interaction_closure.source_capability | ConvertTo-Json -Compress) -or
                    -not $verificationResult.interaction_closure.recorder_host.forbidden_symbols_absent -or
                    -not $verificationResult.interaction_closure.replay_app.forbidden_symbols_absent -or
                    -not $verificationResult.interaction_closure.collector_dll.forbidden_symbols_absent -or
                    [string]$verificationResult.receipt_sha256 -cne $candidateReceiptHash -or
                    [string]$verificationResult.package_sha256 -cne [string]$packageResult.sha256 -or
                    (($verificationResult.production_capture_evidence | ConvertTo-Json -Depth 12 -Compress) -cne
                        ($productionCaptureEvidence | ConvertTo-Json -Depth 12 -Compress)) -or
                    [string]$verificationResult.build_environment_sha256 -cne [string]$releaseBuildEnvironmentAfter.sha256) {
                    throw "portable package receipt verifier returned no promotion authority"
                }
                $postVerificationSourceDigest = Get-ProductionRustInputDigest -RepoRoot $repoRoot
                $postVerificationCaptureEvidence = Get-CurrentProductionCaptureEvidence
                if ([string]$postVerificationSourceDigest.sha256 -cne [string]$releaseInputDigestAfter.sha256 -or
                    [int]$postVerificationSourceDigest.file_count -ne [int]$releaseInputDigestAfter.file_count -or
                    ((Get-CurrentInteractionPolicyInputs) | ConvertTo-Json -Compress) -cne
                        ($packageResult.interaction_closure.policy_inputs | ConvertTo-Json -Compress) -or
                    (Get-FileHash -LiteralPath $packageReceiptCandidate -Algorithm SHA256).Hash.ToLowerInvariant() -cne $candidateReceiptHash -or
                    (Get-FileHash -LiteralPath $expectedPackagePath -Algorithm SHA256).Hash.ToLowerInvariant() -cne [string]$packageResult.sha256 -or
                    (Get-FileHash -LiteralPath $artifactAttestationPath -Algorithm SHA256).Hash.ToLowerInvariant() -cne [string]$attested.receipt_sha256 -or
                    (Get-FileHash -LiteralPath $portablePackageScript -Algorithm SHA256).Hash.ToLowerInvariant() -cne $packageBuilderHash -or
                    (($postVerificationCaptureEvidence | ConvertTo-Json -Depth 12 -Compress) -cne
                        ($productionCaptureEvidence | ConvertTo-Json -Depth 12 -Compress))) {
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

                # Publication is not success authority by itself. Re-open the
                # canonical receipt, package, and attestation through the
                # independent verifier, then repeat every mutable source/hash
                # check. Any mismatch flows to the outer fail-closed cleanup,
                # which removes all three published promotion artifacts.
                $publishedReceiptHash =
                    (Get-FileHash -LiteralPath $portablePackageReceiptPath -Algorithm SHA256).Hash.ToLowerInvariant()
                if ($publishedReceiptHash -cne $candidateReceiptHash) {
                    throw 'published portable package receipt differs from the verified candidate'
                }
                $canonicalVerificationStdout = @(& powershell.exe -NoProfile -ExecutionPolicy Bypass `
                    -File $portablePackageVerifier `
                    -RepoRoot $repoRoot `
                    -ReceiptPath $portablePackageReceiptPath)
                if ($LASTEXITCODE -ne 0) {
                    throw "canonical portable package receipt verifier exited with code $LASTEXITCODE"
                }
                $canonicalVerification =
                    (($canonicalVerificationStdout -join [Environment]::NewLine).Trim() | ConvertFrom-Json)
                if ([string]$canonicalVerification.schema -cne
                        'battle-plan.central_portable_package_verification.v1' -or
                    [string]$canonicalVerification.status -cne
                        'PASS_CENTRAL_RELEASE_PORTABLE_PACKAGE' -or
                    $canonicalVerification.canonical_receipt -isnot [bool] -or
                    -not $canonicalVerification.canonical_receipt -or
                    [string]$canonicalVerification.receipt_sha256 -cne $publishedReceiptHash -or
                    [string]$canonicalVerification.package_sha256 -cne [string]$packageResult.sha256 -or
                    (($canonicalVerification.production_capture_evidence | ConvertTo-Json -Depth 12 -Compress) -cne
                        ($productionCaptureEvidence | ConvertTo-Json -Depth 12 -Compress)) -or
                    [string]$canonicalVerification.build_environment_sha256 -cne
                        [string]$releaseBuildEnvironmentAfter.sha256 -or
                    [string]$canonicalVerification.interaction_closure.policy -cne
                        'passive_injected_observer' -or
                    -not $canonicalVerification.interaction_closure.zip_entry_bytes_reverified -or
                    ($canonicalVerification.interaction_closure.source_capability | ConvertTo-Json -Compress) -cne
                        ($packageResult.interaction_closure.source_capability | ConvertTo-Json -Compress)) {
                    throw 'published portable package receipt has no canonical verification authority'
                }
                $finalPublishedSourceDigest = Get-ProductionRustInputDigest -RepoRoot $repoRoot
                $finalPublishedCaptureEvidence = Get-CurrentProductionCaptureEvidence
                if ([string]$finalPublishedSourceDigest.sha256 -cne
                        [string]$releaseInputDigestAfter.sha256 -or
                    [int]$finalPublishedSourceDigest.file_count -ne
                        [int]$releaseInputDigestAfter.file_count -or
                    ((Get-CurrentInteractionPolicyInputs) | ConvertTo-Json -Compress) -cne
                        ($packageResult.interaction_closure.policy_inputs | ConvertTo-Json -Compress) -or
                    (Get-FileHash -LiteralPath $portablePackageReceiptPath -Algorithm SHA256).Hash.ToLowerInvariant() -cne
                        $publishedReceiptHash -or
                    (Get-FileHash -LiteralPath $expectedPackagePath -Algorithm SHA256).Hash.ToLowerInvariant() -cne
                        [string]$packageResult.sha256 -or
                    (Get-FileHash -LiteralPath $artifactAttestationPath -Algorithm SHA256).Hash.ToLowerInvariant() -cne
                        [string]$attested.receipt_sha256 -or
                    (Get-FileHash -LiteralPath $portablePackageScript -Algorithm SHA256).Hash.ToLowerInvariant() -cne
                        $packageBuilderHash -or
                    (($finalPublishedCaptureEvidence | ConvertTo-Json -Depth 12 -Compress) -cne
                        ($productionCaptureEvidence | ConvertTo-Json -Depth 12 -Compress))) {
                    throw 'published portable package promotion inputs changed during canonical re-verification'
                }
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
