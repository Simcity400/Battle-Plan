[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [switch]$AllowReleaseManual,

    [string]$ProductionCaptureReceiptPath = '',

    # Publish the already centrally attested and captured candidate; never rebuild.
    [switch]$PackagePrebuilt,

    [string]$C07GameRoot = '',

    [switch]$Plan
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$global:LASTEXITCODE = 1

function Invoke-NativeChecked {
    param(
        [Parameter(Mandatory = $true)][string]$Program,
        [Parameter(Mandatory = $true)][string[]]$Arguments,
        [Parameter(Mandatory = $true)][string]$Label
    )

    Write-Host ("> {0} {1}" -f $Program, ($Arguments -join ' '))
    $output = @(& $Program @Arguments)
    $exitCode = $LASTEXITCODE
    foreach ($line in $output) { Write-Host ([string]$line) }
    if ($exitCode -ne 0) {
        throw "$Label failed with exit code $exitCode"
    }
    return $output
}

function ConvertFrom-GateJson {
    param(
        [Parameter(Mandatory = $true)][object[]]$Output,
        [Parameter(Mandatory = $true)][string]$Label,
        [switch]$Multiline
    )

    $lines = @($Output | ForEach-Object { [string]$_ } | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
    if ($lines.Count -eq 0) { throw "$Label returned no structured receipt." }
    $json = if ($Multiline) { $lines -join [Environment]::NewLine } else { $lines[-1] }
    try { return ($json | ConvertFrom-Json) }
    catch { throw "$Label returned an invalid structured receipt: $($_.Exception.Message)" }
}

function Test-JsonIntegerAtLeast {
    param(
        [AllowNull()][object]$Value,
        [long]$Minimum
    )

    if ($null -eq $Value -or -not ($Value -is [ValueType])) { return $false }
    $parsed = 0L
    return (
        [long]::TryParse(
            [string]$Value,
            [Globalization.NumberStyles]::Integer,
            [Globalization.CultureInfo]::InvariantCulture,
            [ref]$parsed
        ) -and
        $parsed -ge $Minimum
    )
}

function Get-PrebuiltPackageVersion {
    param([Parameter(Mandatory = $true)][string]$RecorderManifest)

    # The shipping crate declares a literal version. Read it without starting
    # Cargo in the publication phase; inherited/ambiguous versions fail closed.
    $cargo = Get-Content -LiteralPath $RecorderManifest -Raw
    $packages = @([regex]::Matches($cargo, '(?ms)^\[package\][ \t]*\r?\n(?<body>.*?)(?=^\[|\z)'))
    if ($packages.Count -ne 1) { throw 'Expected one recorder package table.' }
    $body = $packages[0].Groups['body'].Value
    $names = @([regex]::Matches($body, '(?m)^name[ \t]*=[ \t]*"(?<value>[^"\r\n]+)"[ \t]*\r?$'))
    $versions = @([regex]::Matches($body, '(?m)^version[ \t]*=[ \t]*"(?<value>[0-9A-Za-z][0-9A-Za-z._-]{0,63})"[ \t]*\r?$'))
    if ($names.Count -ne 1 -or $names[0].Groups['value'].Value -cne 'recorder_host' -or $versions.Count -ne 1) {
        throw 'Prebuilt publication requires the exact literal recorder_host package version.'
    }
    return $versions[0].Groups['value'].Value
}

function Get-PrebuiltPackageInputs {
    param(
        [Parameter(Mandatory = $true)][string]$RepoRoot,
        [Parameter(Mandatory = $true)][string]$CaptureReceiptPath
    )

    $releaseRoot = Join-Path $RepoRoot '.cargo-target\release'
    $attestationPath = Join-Path $releaseRoot 'hoi4-production-prebuilt-artifacts.json'
    # This rechecks the original central release, current source/runner/driver,
    # static CRT, and all canonical artifact byte identities. Never mint a new
    # attestation or replace its recorded build environment during publication.
    $attested = Test-ProductionArtifactAttestation -RepoRoot $RepoRoot -ReceiptPath $attestationPath
    $null = Get-ProductionCapturePromotionEvidence -RepoRoot $RepoRoot `
        -CaptureReceiptPath $CaptureReceiptPath -CurrentArtifactAttestationPath $attestationPath
    $version = Get-PrebuiltPackageVersion -RecorderManifest (Join-Path $RepoRoot 'extractor\native\recorder_host\Cargo.toml')
    $packagePath = Join-Path $releaseRoot ("battle-plan-{0}-windows-x86_64.zip" -f $version)
    $receiptPath = Join-Path $releaseRoot 'hoi4-portable-package-receipt.json'
    foreach ($path in @($packagePath, $receiptPath)) {
        if (Test-Path -LiteralPath $path) { throw "Prebuilt publication refuses an existing package output: $path" }
    }
    return [pscustomobject]@{
        attested = $attested
        version = $version
        package_path = $packagePath
        receipt_path = $receiptPath
    }
}

function Remove-ExactFiles {
    param([Parameter(Mandatory = $true)][string[]]$Paths)

    foreach ($path in @($Paths | Select-Object -Unique)) {
        if (Test-Path -LiteralPath $path) {
            Remove-Item -LiteralPath $path -Force
        }
        if (Test-Path -LiteralPath $path) {
            throw "release output could not be removed: $path"
        }
    }
}

function Enable-StaticMsvcCrt {
    $verboseVersion = (& rustc -vV 2>&1 | Out-String)
    if ($LASTEXITCODE -ne 0 -or $verboseVersion -notmatch '(?m)^host:\s+x86_64-pc-windows-msvc\s*$') {
        throw 'ReleaseManual requires the x86_64-pc-windows-msvc Rust host.'
    }

    $env:CARGO_ENCODED_RUSTFLAGS = @('-C', 'target-feature=+crt-static') -join [char]0x1f
    Remove-Item Env:RUSTFLAGS -ErrorAction SilentlyContinue
    $cfg = @(& rustc -C 'target-feature=+crt-static' --print cfg 2>&1)
    if ($LASTEXITCODE -ne 0 -or $cfg -cnotcontains 'target_env="msvc"' -or
        $cfg -cnotcontains 'target_feature="crt-static"') {
        throw 'ReleaseManual could not activate the static MSVC CRT.'
    }

    return [ordered]@{
        target = 'x86_64-pc-windows-msvc'
        rust_target_feature = '+crt-static'
        c_runtime = 'static'
        admission = 'pe_import_and_delay_import_tables'
        forbidden_import = 'VCRUNTIME*.dll'
    }
}

$releaseOutputs = @()
try {
    if (-not $AllowReleaseManual) {
        throw 'ReleaseManual requires -AllowReleaseManual and explicit owner authorization.'
    }

    $repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..\..'))
    $validation = Join-Path $PSScriptRoot 'Invoke-Validation.ps1'
    $attestationHelper = Join-Path $repoRoot 'portable_replay\native\scripts\ProductionArtifact.Attestation.ps1'
    $packageBuilder = Join-Path $PSScriptRoot 'New-PortableProductPackage.ps1'
    $packageVerifier = Join-Path $PSScriptRoot 'Test-PortableProductPackageReceipt.ps1'
    $passivePolicy = Join-Path $PSScriptRoot 'Test-PassiveCapturePolicy.ps1'
    $headlessUnits = Join-Path $PSScriptRoot 'Test-HeadlessRawCaptureUnits.ps1'
    $releaseUnits = Join-Path $PSScriptRoot 'Test-ReleaseManualUnits.ps1'
    $promotionAbsence = Join-Path $PSScriptRoot 'Test-PromotionGateAbsence.ps1'
    $interactionClosure = Join-Path $PSScriptRoot 'Test-ProductionInteractionClosure.ps1'
    $c07Generator = Join-Path $repoRoot 'replay_interface\re_evidence\rendering_laws\C07\build_c07_portable_tree_library.py'
    $extractorManifest = Join-Path $repoRoot 'extractor\native\Cargo.toml'
    $replayManifest = Join-Path $repoRoot 'replay_interface\native\Cargo.toml'
    $releaseRoot = Join-Path $repoRoot '.cargo-target\release'
    $attestationPath = Join-Path $releaseRoot 'hoi4-production-prebuilt-artifacts.json'
    $packageReceiptPath = Join-Path $releaseRoot 'hoi4-portable-package-receipt.json'

    if ($PackagePrebuilt) {
        if ([string]::IsNullOrWhiteSpace($ProductionCaptureReceiptPath)) {
            throw 'PackagePrebuilt requires a production capture receipt.'
        }
        if (-not [string]::IsNullOrWhiteSpace($C07GameRoot)) {
            throw 'PackagePrebuilt consumes existing central certification; C07GameRoot is a build-phase argument.'
        }
        if ($Plan) {
            Write-Host "PLAN> verify existing canonical attestation $attestationPath and capture $ProductionCaptureReceiptPath"
            Write-Host 'PLAN> publish and verify the package and outer receipt without building or replacing candidate authority'
            $global:LASTEXITCODE = 0
            return
        }
        . $attestationHelper
        . (Join-Path $PSScriptRoot 'ProductionCapture.Promotion.ps1')
        $inputs = Get-PrebuiltPackageInputs -RepoRoot $repoRoot -CaptureReceiptPath $ProductionCaptureReceiptPath
        $attested = $inputs.attested
        $version = [string]$inputs.version
        $packagePath = [string]$inputs.package_path
        $releaseRuntimeLinkage = $attested.release_runtime_linkage
        # Candidate binaries and attestation are never cleanup targets. Claim
        # each package output only after this invocation publishes it.
    }
    else {
        $c07Arguments = @($c07Generator, '--verify')
        if (-not [string]::IsNullOrWhiteSpace($C07GameRoot)) {
            if (-not (Test-Path -LiteralPath $C07GameRoot -PathType Container)) {
                throw "-C07GameRoot is not an existing directory: $C07GameRoot"
            }
            $c07Arguments += @('--game-root', $C07GameRoot)
        }

        if ($Plan) {
            & $validation -Mode Landing -AllValidators -Plan
            if ($LASTEXITCODE -ne 0) { throw "Landing plan failed with exit code $LASTEXITCODE" }
            Write-Host "PLAN> powershell.exe -File $passivePolicy"
            Write-Host "PLAN> powershell.exe -File $headlessUnits"
            Write-Host "PLAN> powershell.exe -File $releaseUnits"
            Write-Host "PLAN> powershell.exe -File $promotionAbsence"
            Write-Host "PLAN> powershell.exe -File $interactionClosure"
            Write-Host ("PLAN> python {0}" -f ($c07Arguments -join ' '))
            Write-Host "PLAN> cargo build --locked --release --manifest-path $replayManifest -p replay_app --bin replay_app"
            Write-Host "PLAN> cargo build --locked --release --manifest-path $extractorManifest -p recorder_host --bin collector_record --no-default-features"
            Write-Host "PLAN> cargo build --locked --release --manifest-path $extractorManifest -p collector --lib --no-default-features"
            if (-not [string]::IsNullOrWhiteSpace($ProductionCaptureReceiptPath)) {
                Write-Host "PLAN> package the three shipping artifacts using $ProductionCaptureReceiptPath"
            }
            $global:LASTEXITCODE = 0
            return
        }

        & $validation -Mode Landing -AllValidators
        if ($LASTEXITCODE -ne 0) { throw "Landing validation failed with exit code $LASTEXITCODE" }

        $gateSpecs = @(
            [pscustomobject]@{ Path = $releaseUnits; Schema = 'hoi4-release-manual-unit-test-v1'; Status = 'pass'; MinCases = 25; Multiline = $false },
            [pscustomobject]@{ Path = $passivePolicy; Schema = 'hoi4-passive-capture-policy-test-v1'; Status = 'pass'; MinCases = 0; Multiline = $false },
            [pscustomobject]@{ Path = $headlessUnits; Schema = 'hoi4-headless-raw-capture-unit-test-v1'; Status = 'pass'; MinCases = 200; Multiline = $false },
            [pscustomobject]@{ Path = $promotionAbsence; Schema = 'battle-plan.promotion_gate_absence_test.v1'; Status = 'PASS'; MinCases = 1; Multiline = $false },
            [pscustomobject]@{ Path = $interactionClosure; Schema = 'battle-plan.production_interaction_closure_test.v1'; Status = 'PASS'; MinCases = 1; Multiline = $true }
        )
        foreach ($gate in $gateSpecs) {
            $label = [IO.Path]::GetFileName([string]$gate.Path)
            $gateOutput = @(Invoke-NativeChecked -Program 'powershell.exe' `
                -Arguments @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', [string]$gate.Path) `
                -Label $label)
            $gateReceipt = ConvertFrom-GateJson -Output $gateOutput -Label $label -Multiline:([bool]$gate.Multiline)
            if ([string]$gateReceipt.Schema -cne [string]$gate.Schema -or
                [string]$gateReceipt.Status -cne [string]$gate.Status -or
                ([long]$gate.MinCases -gt 0 -and
                    -not (Test-JsonIntegerAtLeast -Value $gateReceipt.Cases -Minimum ([long]$gate.MinCases)))) {
                throw "$label returned no complete PASS receipt."
            }
        }
        $c07Output = @(Invoke-NativeChecked -Program 'python' -Arguments $c07Arguments -Label 'C07 game-derived certification')
        $c07Receipt = ConvertFrom-GateJson -Output $c07Output -Label 'C07 game-derived certification' -Multiline
        $expectedC07Outputs = @(
            'c07_tree_complete_buckets.rs',
            'c07_tree_content_hashes.rs',
            'c07_tree_content_library.bin'
        )
        $verifiedC07Outputs = @($c07Receipt.verified_outputs | Sort-Object)
        if ([string]$c07Receipt.schema -cne 'hoi4-c07-complete-engine-derived-tree-content-library-v2' -or
            [string]$c07Receipt.mode -cne 'verify' -or @($c07Receipt.changed_outputs).Count -ne 0 -or
            $verifiedC07Outputs.Count -ne $expectedC07Outputs.Count -or
            @(Compare-Object -ReferenceObject $expectedC07Outputs -DifferenceObject $verifiedC07Outputs -CaseSensitive).Count -ne 0) {
            throw 'C07 game-derived certification returned no complete verify receipt.'
        }

        . $attestationHelper
        [IO.Directory]::CreateDirectory($releaseRoot) | Out-Null
        $releaseRuntimeLinkage = Enable-StaticMsvcCrt

        $metadataText = & cargo metadata --locked --no-deps --format-version 1 --manifest-path $extractorManifest
        if ($LASTEXITCODE -ne 0) { throw "Cargo metadata failed with exit code $LASTEXITCODE" }
        $metadata = $metadataText | ConvertFrom-Json
        $recorderPackage = @($metadata.packages | Where-Object { [string]$_.name -ceq 'recorder_host' })
        if ($recorderPackage.Count -ne 1) { throw 'Cargo metadata did not identify exactly one recorder_host package.' }
        $version = [string]$recorderPackage[0].version
        $packagePath = Join-Path $releaseRoot ("battle-plan-{0}-windows-x86_64.zip" -f $version)

        $releaseOutputs = @($attestationPath, $packageReceiptPath, $packagePath) + @(Get-ProductionArtifactPaths -RepoRoot $repoRoot)
        Remove-ExactFiles -Paths $releaseOutputs
        $artifactFreshness = Clear-ProductionArtifactOutputs -RepoRoot $repoRoot
        $buildEnvironmentBefore = Get-ProductionBuildEnvironment
        $sourceBefore = Get-ProductionRustInputDigest -RepoRoot $repoRoot

        $null = Invoke-NativeChecked -Program 'cargo' -Arguments @(
            'build', '--locked', '--release', '--manifest-path', $replayManifest,
            '-p', 'replay_app', '--bin', 'replay_app'
        ) -Label 'replay_app release build'
        $null = Invoke-NativeChecked -Program 'cargo' -Arguments @(
            'build', '--locked', '--release', '--manifest-path', $extractorManifest,
            '-p', 'recorder_host', '--bin', 'collector_record', '--no-default-features'
        ) -Label 'collector_record release build'
        $null = Invoke-NativeChecked -Program 'cargo' -Arguments @(
            'build', '--locked', '--release', '--manifest-path', $extractorManifest,
            '-p', 'collector', '--lib', '--no-default-features'
        ) -Label 'collector release build'

        $buildEnvironmentAfter = Get-ProductionBuildEnvironment
        $sourceAfter = Get-ProductionRustInputDigest -RepoRoot $repoRoot
        if ([string]$buildEnvironmentBefore.sha256 -cne [string]$buildEnvironmentAfter.sha256) {
            throw 'Production build environment changed during ReleaseManual.'
        }
        if ([string]$sourceBefore.sha256 -cne [string]$sourceAfter.sha256 -or
            [int]$sourceBefore.file_count -ne [int]$sourceAfter.file_count) {
            throw 'Production Rust inputs changed during ReleaseManual.'
        }

        Write-ProductionArtifactAttestation `
            -RepoRoot $repoRoot `
            -ReceiptPath $attestationPath `
            -RequestedWorkspace 'All' `
            -SourceDigest $sourceAfter `
            -ReleaseRuntimeLinkage $releaseRuntimeLinkage `
            -BuildEnvironment $buildEnvironmentAfter `
            -ArtifactFreshness $artifactFreshness
        $driver = Join-Path $PSScriptRoot 'Start-Hoi4ProductionCaptureSafe.ps1'
        $attested = Test-ProductionArtifactAttestation `
            -RepoRoot $repoRoot `
            -ReceiptPath $attestationPath `
            -ExpectedDriverPath $driver `
            -ExpectedBuildEnvironment $buildEnvironmentAfter `
            -CurrentSourceDigest $sourceAfter

        if ([string]::IsNullOrWhiteSpace($ProductionCaptureReceiptPath)) {
            Write-Host "Candidate production artifacts: $attestationPath"
            Write-Host 'No package was emitted because no production capture receipt was supplied.'
            $global:LASTEXITCODE = 0
            return
        }

    }

    if (-not (Test-Path -LiteralPath $ProductionCaptureReceiptPath -PathType Leaf)) {
        throw "Production capture receipt does not exist: $ProductionCaptureReceiptPath"
    }
    $packageBuilderHash = (Get-FileHash -LiteralPath $packageBuilder -Algorithm SHA256).Hash.ToLowerInvariant()
    $packageOutput = @(& powershell.exe -NoProfile -ExecutionPolicy Bypass -File $packageBuilder `
        -RecorderExe ([string]$attested.artifacts.recorder_host) `
        -ReplayExe ([string]$attested.artifacts.replay_app) `
        -CollectorDll ([string]$attested.artifacts.collector_dll) `
        -OutputZip $packagePath `
        -Version $version `
        -ProductionCaptureReceiptPath $ProductionCaptureReceiptPath `
        -ProductionArtifactAttestationPath $attestationPath)
    if ($LASTEXITCODE -ne 0) { throw "Package builder failed with exit code $LASTEXITCODE" }
    if ($PackagePrebuilt) { $releaseOutputs = @($packagePath) }
    $package = (($packageOutput -join [Environment]::NewLine).Trim() | ConvertFrom-Json)
    if ([string]$package.schema -cne 'hoi4-portable-package.v1' -or
        -not $package.admissible_for_production -or
        [string]$package.sha256 -cne (Get-FileHash -LiteralPath $packagePath -Algorithm SHA256).Hash.ToLowerInvariant()) {
        throw 'Package builder returned no production-admissible package authority.'
    }

    $sourceCapability = $package.interaction_closure.source_capability
    $policyInputs = $package.interaction_closure.policy_inputs
    $receipt = [ordered]@{
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
                    package = 'recorder_host'; target = 'collector_record'; default_features = $false
                    enabled_features = @(); artifact_sha256 = [string]$attested.artifact_sha256.recorder_host
                }
                collector_dll = [ordered]@{
                    package = 'collector'; target = 'lib'; crate_types = @('cdylib', 'rlib')
                    default_features = $false; enabled_features = @(); all_non_default_features_disabled = $true
                    forbidden_features = @('test-support'); built_last = $true
                    collector_cargo_sha256 = [string]$policyInputs.collector_cargo_sha256
                    pinned_hook_count = [int]$sourceCapability.exact_pinned_hook_count
                    pinned_hook_contract_source_sha256 = [string]$policyInputs.passive_ready_contract_sha256
                    artifact_sha256 = [string]$attested.artifact_sha256.collector_dll
                }
            }
        }
        artifact_attestation = [ordered]@{ path = [string]$attested.receipt_path; sha256 = [string]$attested.receipt_sha256 }
        package_builder = [ordered]@{ path = [IO.Path]::GetFullPath($packageBuilder); sha256 = $packageBuilderHash }
        package = $package
        source_input_sha256 = [string]$attested.source_input_sha256
        build_environment = $attested.build_environment
    }
    $candidate = "$packageReceiptPath.$([Guid]::NewGuid().ToString('N')).candidate"
    try {
        [IO.File]::WriteAllText($candidate, ($receipt | ConvertTo-Json -Depth 12), [Text.UTF8Encoding]::new($false))
        $null = Invoke-NativeChecked -Program 'powershell.exe' -Arguments @(
            '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $packageVerifier,
            '-RepoRoot', $repoRoot, '-ReceiptPath', $candidate, '-AllowStagedReceipt'
        ) -Label 'staged package receipt verification'
        [IO.File]::Move($candidate, $packageReceiptPath)
        if ($PackagePrebuilt) { $releaseOutputs += $packageReceiptPath }
        $null = Invoke-NativeChecked -Program 'powershell.exe' -Arguments @(
            '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $packageVerifier,
            '-RepoRoot', $repoRoot, '-ReceiptPath', $packageReceiptPath
        ) -Label 'published package receipt verification'
    }
    finally {
        if (Test-Path -LiteralPath $candidate) { Remove-Item -LiteralPath $candidate -Force }
    }

    Write-Host "Production artifact attestation: $attestationPath"
    Write-Host "Portable product package: $packagePath"
    Write-Host "Portable product package receipt: $packageReceiptPath"
    $global:LASTEXITCODE = 0
}
catch {
    $failure = $_
    $global:LASTEXITCODE = 1
    if ($releaseOutputs.Count -gt 0) {
        try { Remove-ExactFiles -Paths $releaseOutputs }
        catch { Write-Error "ReleaseManual failed ($($failure.Exception.Message)); cleanup also failed ($($_.Exception.Message))" }
    }
    Write-Error $failure
}
