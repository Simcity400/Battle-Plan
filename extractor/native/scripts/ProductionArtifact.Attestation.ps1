Set-StrictMode -Version Latest
. (Join-Path $PSScriptRoot 'PortableProduct.PeImports.ps1')
. (Join-Path $PSScriptRoot 'Hoi4ClientDriving.OptIn.ps1')

$script:ProductionArtifactReceiptSchema = "battle-plan.central_rust_prebuilt_artifacts.v1"
$script:ProductionArtifactRoles = [ordered]@{
    collector_dll = "collector.dll"
    recorder_host = "collector_record.exe"
    fold_check = "fold_check.exe"
    compact_replay_gate = "compact_replay_gate.exe"
    container_verify = "pack_replay.exe"
    replay_app = "replay_app.exe"
}
$script:ProductionArtifactPackages = @(
    "collector",
    "recorder_host",
    "fold_check",
    "compact_replay_verification_gates",
    "snapshot"
    "replay_app"
)
$script:PortableProductArtifactRoles = @('collector_dll', 'recorder_host', 'replay_app')
$script:ProductionBuildEnvironmentSchema = 'battle-plan.production_build_environment.v1'
$script:ProductionArtifactFreshnessSchema = 'battle-plan.production_artifact_freshness.v1'
$script:ProductionBuildEnvironmentKeys = @(
    'AR', 'AR_x86_64_pc_windows_msvc', 'CC', 'CC_x86_64_pc_windows_msvc',
    'CFLAGS', 'CFLAGS_x86_64_pc_windows_msvc',
    'CARGO_BUILD_TARGET', 'CARGO_ENCODED_RUSTFLAGS', 'CARGO_INCREMENTAL',
    'CARGO_TARGET_DIR', 'CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER',
    'CXX', 'CXXFLAGS', 'INCLUDE', 'LIB', 'LIBPATH', 'PATH_SHA256',
    'RUSTC_WRAPPER', 'RUSTDOCFLAGS', 'RUSTFLAGS',
    'UniversalCRTSdkDir', 'UCRTVersion', 'VCToolsInstallDir', 'VCToolsVersion',
    'WindowsSdkDir', 'WindowsSDKVersion'
)

function Get-ProductionArtifactPaths {
    param([Parameter(Mandatory = $true)][string]$RepoRoot)

    $releaseRoot = Join-Path ([IO.Path]::GetFullPath($RepoRoot)) '.cargo-target\release'
    foreach ($entry in $script:ProductionArtifactRoles.GetEnumerator()) {
        [IO.Path]::GetFullPath((Join-Path $releaseRoot $entry.Value))
    }
}

function Clear-ProductionArtifactOutputs {
    param([Parameter(Mandatory = $true)][string]$RepoRoot)

    $paths = @(Get-ProductionArtifactPaths -RepoRoot $RepoRoot)
    foreach ($path in $paths) {
        if (Test-Path -LiteralPath $path -ErrorAction Stop) {
            Remove-Item -LiteralPath $path -Force -ErrorAction Stop
        }
    }
    $remaining = @($paths | Where-Object { Test-Path -LiteralPath $_ -ErrorAction Stop })
    if ($remaining.Count -ne 0) {
        throw "failed to remove canonical production outputs before Release: $($remaining -join ', ')"
    }
    return [pscustomobject][ordered]@{
        schema = $script:ProductionArtifactFreshnessSchema
        policy = 'canonical_outputs_removed_and_absence_verified_before_build'
        artifact_count = $paths.Count
        artifact_paths = $paths
    }
}

function Get-ProductionBuildEnvironmentRecordHash {
    param([Parameter(Mandatory = $true)]$Record)

    $payload = [ordered]@{
        schema = [string]$Record.schema
        rustc_verbose = [string]$Record.rustc_verbose
        cargo_version = [string]$Record.cargo_version
        linker = $Record.linker
        rustc_wrapper = $Record.rustc_wrapper
        msvc_toolset = $Record.msvc_toolset
        windows_sdk = $Record.windows_sdk
        runner = $Record.runner
        build_env = $Record.build_env
    }
    $bytes = [Text.Encoding]::UTF8.GetBytes(($payload | ConvertTo-Json -Depth 12 -Compress))
    $sha = [Security.Cryptography.SHA256]::Create()
    try { return ([BitConverter]::ToString($sha.ComputeHash($bytes))).Replace('-', '').ToLowerInvariant() }
    finally { $sha.Dispose() }
}

function New-ProductionBuildEnvironmentRecord {
    param(
        [Parameter(Mandatory = $true)][string]$RustcVerbose,
        [Parameter(Mandatory = $true)][string]$CargoVersion,
        [Parameter(Mandatory = $true)]$Linker,
        [Parameter(Mandatory = $true)]$RustcWrapper,
        [Parameter(Mandatory = $true)]$MsvcToolset,
        [Parameter(Mandatory = $true)]$WindowsSdk,
        [Parameter(Mandatory = $true)]$Runner,
        [Parameter(Mandatory = $true)]$BuildEnv
    )

    $record = [pscustomobject][ordered]@{
        schema = $script:ProductionBuildEnvironmentSchema
        rustc_verbose = $RustcVerbose.Trim()
        cargo_version = $CargoVersion.Trim()
        linker = $Linker
        rustc_wrapper = $RustcWrapper
        msvc_toolset = $MsvcToolset
        windows_sdk = $WindowsSdk
        runner = $Runner
        build_env = $BuildEnv
        sha256 = ''
    }
    $record.sha256 = Get-ProductionBuildEnvironmentRecordHash -Record $record
    return $record
}

function Test-ProductionBuildEnvironmentRecord {
    param([Parameter(Mandatory = $true)]$Record)

    $requiredEnv = $script:ProductionBuildEnvironmentKeys
    $recordedEnv = if ($Record.build_env -is [Collections.IDictionary]) {
        @($Record.build_env.Keys)
    }
    else {
        @($Record.build_env.psobject.Properties.Name)
    }
    if ([string]$Record.schema -cne $script:ProductionBuildEnvironmentSchema -or
        [string]$Record.rustc_verbose -cnotmatch '(?m)^host:\s+x86_64-pc-windows-msvc\s*$' -or
        [string]$Record.rustc_verbose -cnotmatch '(?m)^commit-hash:\s+[0-9a-f]{40}\s*$' -or
        [string]::IsNullOrWhiteSpace([string]$Record.cargo_version) -or
        [string]::IsNullOrWhiteSpace([string]$Record.linker.version_output) -or
        [string]$Record.linker.executable.sha256 -notmatch '^[0-9a-f]{64}$' -or
        $Record.rustc_wrapper.active -isnot [bool] -or
        ($Record.rustc_wrapper.active -and [string]$Record.rustc_wrapper.executable.sha256 -notmatch '^[0-9a-f]{64}$') -or
        [string]::IsNullOrWhiteSpace([string]$Record.msvc_toolset.version) -or
        [string]$Record.msvc_toolset.compiler.sha256 -notmatch '^[0-9a-f]{64}$' -or
        [string]::IsNullOrWhiteSpace([string]$Record.windows_sdk.version) -or
        [string]$Record.windows_sdk.kernel32_import_library.sha256 -notmatch '^[0-9a-f]{64}$' -or
        [string]$Record.windows_sdk.ucrt_import_library.sha256 -notmatch '^[0-9a-f]{64}$' -or
        [string]::IsNullOrWhiteSpace([string]$Record.runner.os_description) -or
        [string]$Record.runner.process_architecture -cne 'X64' -or
        ($recordedEnv -join '|') -cne ($requiredEnv -join '|') -or
        [string]$Record.sha256 -notmatch '^[0-9a-f]{64}$' -or
        [string]$Record.sha256 -cne (Get-ProductionBuildEnvironmentRecordHash -Record $Record)) {
        throw 'production build environment record is incomplete, malformed, or has changed'
    }
}

function Get-ProductionFileIdentity {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$Label
    )

    $full = [IO.Path]::GetFullPath($Path)
    if (-not (Test-Path -LiteralPath $full -PathType Leaf)) { throw "$Label is unavailable: $full" }
    $item = Get-Item -LiteralPath $full
    return [pscustomobject][ordered]@{
        path = $full
        bytes = [int64]$item.Length
        sha256 = (Get-FileHash -LiteralPath $full -Algorithm SHA256).Hash.ToLowerInvariant()
        file_version = [string]$item.VersionInfo.FileVersion
        product_version = [string]$item.VersionInfo.ProductVersion
    }
}

function Get-ProductionBuildEnvironment {
    $rustcVerbose = (& rustc -vV 2>&1 | Out-String).Trim()
    if ($LASTEXITCODE -ne 0 -or [string]::IsNullOrWhiteSpace($rustcVerbose)) {
        throw 'rustc -vV failed while recording the production build environment'
    }
    $cargoVersion = (& cargo --version 2>&1 | Out-String).Trim()
    if ($LASTEXITCODE -ne 0 -or [string]::IsNullOrWhiteSpace($cargoVersion)) {
        throw 'cargo --version failed while recording the production build environment'
    }

    $toolsetRoot = $env:VCToolsInstallDir
    $toolsetVersion = $env:VCToolsVersion
    $toolsetSource = 'central_release_explicit_environment'
    if ([string]::IsNullOrWhiteSpace($toolsetRoot)) {
        $programFilesX86 = [Environment]::GetFolderPath([Environment+SpecialFolder]::ProgramFilesX86)
        $vswhere = Join-Path $programFilesX86 'Microsoft Visual Studio\Installer\vswhere.exe'
        if (-not (Test-Path -LiteralPath $vswhere -PathType Leaf)) {
            throw 'Visual Studio toolset identity is unavailable: vswhere.exe was not found'
        }
        $installation = (& $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath 2>&1 | Out-String).Trim()
        if ($LASTEXITCODE -ne 0 -or [string]::IsNullOrWhiteSpace($installation)) {
            throw 'Visual Studio C++ toolset discovery failed'
        }
        $versionFile = Join-Path $installation 'VC\Auxiliary\Build\Microsoft.VCToolsVersion.default.txt'
        if (-not (Test-Path -LiteralPath $versionFile -PathType Leaf)) {
            throw 'Visual Studio default C++ toolset version file is unavailable'
        }
        $toolsetVersion = (Get-Content -LiteralPath $versionFile -Raw).Trim()
        $toolsetRoot = Join-Path $installation ("VC\Tools\MSVC\{0}" -f $toolsetVersion)
    }
    $toolsetRoot = [IO.Path]::GetFullPath($toolsetRoot)
    $env:VCToolsInstallDir = $toolsetRoot
    $env:VCToolsVersion = [string]$toolsetVersion
    $clPath = Join-Path $toolsetRoot 'bin\Hostx64\x64\cl.exe'
    $msvcLinkPath = Join-Path $toolsetRoot 'bin\Hostx64\x64\link.exe'
    $clIdentity = Get-ProductionFileIdentity -Path $clPath -Label 'MSVC compiler'
    $msvcLinkIdentity = Get-ProductionFileIdentity -Path $msvcLinkPath -Label 'MSVC linker'

    $configuredLinker = [Environment]::GetEnvironmentVariable('CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER')
    if ([string]::IsNullOrWhiteSpace($configuredLinker)) {
        $configuredLinker = $msvcLinkPath
        $env:CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER = $configuredLinker
    }
    $linkerCommand = Get-Command $configuredLinker -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($null -eq $linkerCommand) { throw "configured production linker is unavailable: $configuredLinker" }
    $linkerIdentity = Get-ProductionFileIdentity -Path $linkerCommand.Source -Label 'production linker'
    $linkerVersion = if ([IO.Path]::GetFileName($linkerIdentity.path) -ieq 'lld-link.exe') {
        (& $linkerIdentity.path --version 2>&1 | Out-String).Trim()
    }
    else { [string]$linkerIdentity.file_version }
    if ([string]::IsNullOrWhiteSpace($linkerVersion)) { throw 'production linker version is unavailable' }

    $wrapperName = [Environment]::GetEnvironmentVariable('RUSTC_WRAPPER')
    $wrapperIdentity = if ([string]::IsNullOrWhiteSpace($wrapperName)) {
        [pscustomobject][ordered]@{ active = $false; configured = ''; executable = $null }
    }
    else {
        $wrapperCommand = Get-Command $wrapperName -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
        if ($null -eq $wrapperCommand) { throw "configured rustc wrapper is unavailable: $wrapperName" }
        [pscustomobject][ordered]@{
            active = $true
            configured = $wrapperName
            executable = Get-ProductionFileIdentity -Path $wrapperCommand.Source -Label 'rustc wrapper'
        }
    }

    $sdkRoot = $env:WindowsSdkDir
    if ([string]::IsNullOrWhiteSpace($sdkRoot)) {
        $sdkRoot = [string](Get-ItemProperty -LiteralPath 'HKLM:\SOFTWARE\Microsoft\Windows Kits\Installed Roots' -Name KitsRoot10 -ErrorAction Stop).KitsRoot10
    }
    $sdkRoot = [IO.Path]::GetFullPath($sdkRoot)
    $sdkVersion = ([string]$env:WindowsSDKVersion).Trim().TrimEnd('\')
    if ([string]::IsNullOrWhiteSpace($sdkVersion)) {
        $sdkVersion = [string]@(Get-ChildItem -LiteralPath (Join-Path $sdkRoot 'Include') -Directory |
            Where-Object Name -Match '^\d+\.\d+\.\d+\.\d+$' |
            Sort-Object { [version]$_.Name } -Descending |
            Select-Object -First 1 -ExpandProperty Name)
    }
    if ([string]::IsNullOrWhiteSpace($sdkVersion)) { throw 'Windows SDK version is unavailable' }
    $env:WindowsSdkDir = $sdkRoot
    $env:WindowsSDKVersion = "$sdkVersion\"
    $env:UniversalCRTSdkDir = $sdkRoot
    $env:UCRTVersion = $sdkVersion
    $kernel32 = Get-ProductionFileIdentity -Path (Join-Path $sdkRoot ("Lib\{0}\um\x64\kernel32.lib" -f $sdkVersion)) -Label 'Windows SDK kernel32 import library'
    $ucrt = Get-ProductionFileIdentity -Path (Join-Path $sdkRoot ("Lib\{0}\ucrt\x64\ucrt.lib" -f $sdkVersion)) -Label 'Windows SDK UCRT import library'

    $buildEnv = [ordered]@{}
    foreach ($name in @(
        'AR', 'AR_x86_64_pc_windows_msvc', 'CC', 'CC_x86_64_pc_windows_msvc',
        'CFLAGS', 'CFLAGS_x86_64_pc_windows_msvc',
        'CARGO_BUILD_TARGET', 'CARGO_ENCODED_RUSTFLAGS', 'CARGO_INCREMENTAL',
        'CARGO_TARGET_DIR', 'CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER',
        'CXX', 'CXXFLAGS', 'INCLUDE', 'LIB', 'LIBPATH', 'RUSTC_WRAPPER',
        'RUSTDOCFLAGS', 'RUSTFLAGS',
        'UniversalCRTSdkDir', 'UCRTVersion', 'VCToolsInstallDir', 'VCToolsVersion',
        'WindowsSdkDir', 'WindowsSDKVersion'
    )) {
        $buildEnv[$name] = [string][Environment]::GetEnvironmentVariable($name)
    }
    $pathBytes = [Text.Encoding]::UTF8.GetBytes([string]$env:PATH)
    $pathHasher = [Security.Cryptography.SHA256]::Create()
    try { $buildEnv['PATH_SHA256'] = ([BitConverter]::ToString($pathHasher.ComputeHash($pathBytes))).Replace('-', '').ToLowerInvariant() }
    finally { $pathHasher.Dispose() }
    $orderedBuildEnv = [ordered]@{}
    foreach ($name in $script:ProductionBuildEnvironmentKeys) { $orderedBuildEnv[$name] = $buildEnv[$name] }
    return New-ProductionBuildEnvironmentRecord `
        -RustcVerbose $rustcVerbose `
        -CargoVersion $cargoVersion `
        -Linker ([pscustomobject][ordered]@{ configured = $configuredLinker; version_output = $linkerVersion; executable = $linkerIdentity }) `
        -RustcWrapper $wrapperIdentity `
        -MsvcToolset ([pscustomobject][ordered]@{
            discovery = $toolsetSource; version = [string]$toolsetVersion; root = $toolsetRoot
            compiler = $clIdentity; msvc_linker = $msvcLinkIdentity
        }) `
        -WindowsSdk ([pscustomobject][ordered]@{
            root = $sdkRoot; version = $sdkVersion
            windows_sdk_dir_env = [string]$env:WindowsSdkDir
            windows_sdk_version_env = [string]$env:WindowsSDKVersion
            ucrt_version_env = [string]$env:UCRTVersion
            kernel32_import_library = $kernel32; ucrt_import_library = $ucrt
        }) `
        -Runner ([pscustomobject][ordered]@{
            image_os = [string]$env:ImageOS; image_version = [string]$env:ImageVersion
            runner_os = [string]$env:RUNNER_OS; runner_arch = [string]$env:RUNNER_ARCH
            runner_environment = [string]$env:RUNNER_ENVIRONMENT
            os_description = [Runtime.InteropServices.RuntimeInformation]::OSDescription
            os_architecture = [Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()
            process_architecture = [Runtime.InteropServices.RuntimeInformation]::ProcessArchitecture.ToString()
        }) `
        -BuildEnv $orderedBuildEnv
}

function Get-ProductionInputFiles {
    param([Parameter(Mandatory = $true)][string]$Root)

    $pending = [System.Collections.Generic.Stack[string]]::new()
    $pending.Push([System.IO.Path]::GetFullPath($Root))
    while ($pending.Count -gt 0) {
        $directory = $pending.Pop()
        foreach ($entry in [System.IO.Directory]::EnumerateFileSystemEntries($directory)) {
            $attributes = [System.IO.File]::GetAttributes($entry)
            if (($attributes -band [System.IO.FileAttributes]::Directory) -ne 0) {
                $name = [System.IO.Path]::GetFileName($entry)
                if ($name -eq "target" -or $name -eq ".cargo-target" -or $name.StartsWith(".tmp", [StringComparison]::OrdinalIgnoreCase)) { continue }
                if (($attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) { continue }
                $pending.Push($entry)
            }
            else {
                $entry
            }
        }
    }
}

function Get-ProductionRustInputDigest {
    param([Parameter(Mandatory = $true)][string]$RepoRoot)

    $repo = [System.IO.Path]::GetFullPath($RepoRoot).TrimEnd('\')
    $roots = @(
        (Join-Path $repo "extractor\native"),
        (Join-Path $repo "portable_replay\native"),
        (Join-Path $repo "replay_interface\native")
    )
    $extensions = @(
        ".rs", ".toml", ".lock", ".c", ".cc", ".cpp", ".h", ".hpp",
        ".rc", ".def", ".bin", ".hlsl", ".json", ".py", ".ps1"
    )
    $records = [System.Collections.Generic.List[string]]::new()
    foreach ($root in $roots) {
        if (-not (Test-Path -LiteralPath $root -PathType Container)) {
            throw "production Rust input root is missing: $root"
        }
        foreach ($file in @(Get-ProductionInputFiles -Root $root | Sort-Object)) {
            $full = [System.IO.Path]::GetFullPath($file)
            if (Test-Hoi4ClientDrivingInputPath -RepoRoot $repo -Path $full) { continue }
            $extension = [System.IO.Path]::GetExtension($full).ToLowerInvariant()
            if ($extensions -notcontains $extension) { continue }
            $relative = $full.Substring($repo.Length).TrimStart('\').Replace('\', '/')
            $digest = (Get-FileHash -LiteralPath $full -Algorithm SHA256).Hash.ToLowerInvariant()
            $length = [System.IO.FileInfo]::new($full).Length
            $records.Add("$relative`t$length`t$digest")
        }
    }
    foreach ($extra in @(
        (Join-Path $repo ".gitattributes"),
        (Join-Path $repo ".cargo\config.toml"),
        (Join-Path $repo ".cargo\config"),
        (Join-Path $repo ".config\nextest.toml"),
        (Join-Path $repo "rust-toolchain.toml"),
        (Join-Path $repo "rust-toolchain"),
        # Passive capture admission code executes outside the Rust input roots
        # but remains part of the production trust boundary. Legacy client
        # drivers and evidence-only launch/action helpers are deliberately not
        # production inputs; Test-Hoi4ClientDrivingInputPath excludes them.
        (Join-Path $repo "extractor\game_defs\static_bundle.py"),
        (Join-Path $repo "extractor\game_defs\live_binding_resolver.py"),
        (Join-Path $repo "extractor\game_defs\build_runtime_static_bundle.py"),
        (Join-Path $repo "extractor\game_defs\prepare_product_modset_capture.py"),
        (Join-Path $repo "extractor\game_defs\product_modset_admission.py"),
        (Join-Path $repo "extractor\game_defs\supported_product_modsets.json"),
        (Join-Path $repo "extractor\game_defs\materialize_ui14_engine_vfs.py"),
        (Join-Path $repo "extractor\game_defs\verify_product_modset_capture_ready.py"),
        (Join-Path $repo "replay_interface\re_evidence\rendering_laws\XN15L\verify_xn15l_live.py"),
        (Join-Path $repo "replay_interface\re_evidence\rendering_laws\XN15L\verify_xn15l_composed.py"),
        (Join-Path $repo "replay_interface\re_evidence\rendering_laws\XN15L\stage_so_census.json"),
        (Join-Path $repo "replay_interface\re_evidence\rendering_laws\M02\production_interleave_closure\contract.json"),
        (Join-Path $repo "replay_interface\re_evidence\rendering_laws\capture_harness\authenticated_wait_for_paused_save.ps1"),
        (Join-Path $repo "replay_interface\re_evidence\rendering_laws\CAP6\verify_live_final_d3d_receipt.py"),
        # The combined B4/B5/B7 production owner executes these read-only
        # memory observers and the independent B4 regrader outside the native
        # source root. A centralized Release receipt must therefore become
        # stale whenever any of their exact decoding/admission logic changes.
        (Join-Path $repo "extractor\re_evidence\B4_B5_B7_OFFLINE_ORACLES\capture_b4_sunk_convoy_fixture.py"),
        (Join-Path $repo "extractor\re_evidence\B4_B5_B7_OFFLINE_ORACLES\verify_b4_sunk_convoy_fixture.py"),
        (Join-Path $repo "extractor\re_evidence\B4_B5_B7_OFFLINE_ORACLES\capture_and_verify_h_b5.py"),
        (Join-Path $repo "extractor\re_evidence\B4_B5_B7_OFFLINE_ORACLES\capture_and_verify_h_b7.py")
    )) {
        if (-not (Test-Path -LiteralPath $extra -PathType Leaf)) { continue }
        $full = [System.IO.Path]::GetFullPath($extra)
        if (Test-Hoi4ClientDrivingInputPath -RepoRoot $repo -Path $full) { continue }
        $relative = $full.Substring($repo.Length).TrimStart('\').Replace('\', '/')
        $digest = (Get-FileHash -LiteralPath $full -Algorithm SHA256).Hash.ToLowerInvariant()
        $length = [System.IO.FileInfo]::new($full).Length
        $records.Add("$relative`t$length`t$digest")
    }
    $records.Sort([System.StringComparer]::Ordinal)
    $payload = [System.Text.Encoding]::UTF8.GetBytes(($records -join "`n") + "`n")
    $hasher = [System.Security.Cryptography.SHA256]::Create()
    try { $digest = ([BitConverter]::ToString($hasher.ComputeHash($payload))).Replace('-', '').ToLowerInvariant() }
    finally { $hasher.Dispose() }
    return [pscustomobject]@{
        schema = "battle-plan.production_rust_input_digest.v1"
        sha256 = $digest
        file_count = $records.Count
        selection = "production runtime, passive observer, verifier, repository checkout/Cargo/toolchain/nextest, and product-modset inputs; Cargo output roots and all legacy client-driving/evidence-only acquisition helpers excluded by Hoi4ClientDriving.OptIn.ps1"
    }
}

function Write-ProductionArtifactAttestation {
    param(
        [Parameter(Mandatory = $true)][string]$RepoRoot,
        [Parameter(Mandatory = $true)][string]$ReceiptPath,
        [Parameter(Mandatory = $true)][string]$RequestedWorkspace,
        [Parameter(Mandatory = $true)]$SourceDigest,
        [Parameter(Mandatory = $true)]$ReleaseRuntimeLinkage,
        [Parameter(Mandatory = $true)]$BuildEnvironment,
        [Parameter(Mandatory = $true)]$ArtifactFreshness
    )

    if ($RequestedWorkspace -ne "All") {
        throw "production artifact attestation requires centralized Release for All workspaces"
    }
    Test-ProductionBuildEnvironmentRecord -Record $BuildEnvironment
    $repo = [System.IO.Path]::GetFullPath($RepoRoot)
    $releaseRoot = Join-Path $repo ".cargo-target\release"
    $expectedPaths = @(Get-ProductionArtifactPaths -RepoRoot $repo)
    if ([string]$ArtifactFreshness.schema -cne $script:ProductionArtifactFreshnessSchema -or
        [string]$ArtifactFreshness.policy -cne 'canonical_outputs_removed_and_absence_verified_before_build' -or
        [int]$ArtifactFreshness.artifact_count -ne $expectedPaths.Count -or
        (@($ArtifactFreshness.artifact_paths | ForEach-Object { [IO.Path]::GetFullPath([string]$_) }) -join '|') -ine ($expectedPaths -join '|')) {
        throw 'production artifact freshness record is missing or does not cover every canonical output'
    }
    $driver = Join-Path $repo "extractor\native\scripts\Start-Hoi4ProductionCaptureSafe.ps1"
    $runner = Join-Path $repo "extractor\native\scripts\Invoke-CentralRustValidation.ps1"
    $artifacts = @()
    foreach ($entry in $script:ProductionArtifactRoles.GetEnumerator()) {
        $path = Join-Path $releaseRoot $entry.Value
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
            throw "centralized Release produced no required production artifact: $path"
        }
        $item = Get-Item -LiteralPath $path
        if ($item.Length -le 0) { throw "centralized Release produced an empty artifact: $path" }
        $artifact = [ordered]@{
            role = [string]$entry.Key
            package = [string]$script:ProductionArtifactPackages[$artifacts.Count]
            path = [System.IO.Path]::GetFullPath($path)
            bytes = [int64]$item.Length
            sha256 = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
        }
        if ([string]$entry.Key -in $script:PortableProductArtifactRoles) {
            $artifact.pe_imports = Assert-PortableProductStaticCrt `
                -Bytes ([System.IO.File]::ReadAllBytes($path)) `
                -Name ([string]$entry.Value)
        }
        $artifacts += $artifact
    }
    $validatedWorkspaces = @("Extractor", "Replay")
    $receipt = [ordered]@{
        schema = $script:ProductionArtifactReceiptSchema
        status = "PASS_CENTRAL_RELEASE_PREBUILT_ARTIFACTS"
        successful_exit = $true
        central_validation = [ordered]@{
            mode = "Release"
            requested_workspace = $RequestedWorkspace
            validated_workspaces = $validatedWorkspaces
            workspace_package_selection = @("--workspace")
            production_artifact_packages = @($script:ProductionArtifactPackages)
            release_runtime_linkage = $ReleaseRuntimeLinkage
            runner_path = [System.IO.Path]::GetFullPath($runner)
            runner_sha256 = (Get-FileHash -LiteralPath $runner -Algorithm SHA256).Hash.ToLowerInvariant()
        }
        source_input = $SourceDigest
        build_environment = $BuildEnvironment
        artifact_freshness = $ArtifactFreshness
        capture_driver = [ordered]@{
            path = [System.IO.Path]::GetFullPath($driver)
            sha256 = (Get-FileHash -LiteralPath $driver -Algorithm SHA256).Hash.ToLowerInvariant()
        }
        artifacts = $artifacts
    }
    $parent = Split-Path -Parent $ReceiptPath
    New-Item -ItemType Directory -Path $parent -Force | Out-Null
    $temp = "$ReceiptPath.$([guid]::NewGuid().ToString('N')).tmp"
    [System.IO.File]::WriteAllText(
        $temp,
        ($receipt | ConvertTo-Json -Depth 12),
        [System.Text.UTF8Encoding]::new($false)
    )
    $stream = [System.IO.File]::Open($temp, [System.IO.FileMode]::Open, [System.IO.FileAccess]::ReadWrite, [System.IO.FileShare]::None)
    try { $stream.Flush($true) } finally { $stream.Dispose() }
    Move-Item -LiteralPath $temp -Destination $ReceiptPath -Force
}

function Test-ProductionArtifactAttestation {
    param(
        [Parameter(Mandatory = $true)][string]$RepoRoot,
        [Parameter(Mandatory = $true)][string]$ReceiptPath,
        [Parameter(Mandatory = $true)][string]$ExpectedDriverPath,
        $ExpectedBuildEnvironment = $null
    )

    if (-not (Test-Path -LiteralPath $ReceiptPath -PathType Leaf)) {
        throw "centralized prebuilt artifact attestation is missing: $ReceiptPath"
    }
    $repo = [System.IO.Path]::GetFullPath($RepoRoot)
    $receipt = Get-Content -LiteralPath $ReceiptPath -Raw | ConvertFrom-Json
    if ([string]$receipt.schema -cne $script:ProductionArtifactReceiptSchema -or
        [string]$receipt.status -cne "PASS_CENTRAL_RELEASE_PREBUILT_ARTIFACTS" -or
        $receipt.successful_exit -isnot [bool] -or -not $receipt.successful_exit) {
        throw "centralized prebuilt artifact attestation has no successful Release authority"
    }
    if ([string]$receipt.central_validation.mode -cne "Release" -or
        [string]$receipt.central_validation.requested_workspace -cne "All" -or
        (@($receipt.central_validation.validated_workspaces) -notcontains "Extractor") -or
        (@($receipt.central_validation.validated_workspaces) -notcontains "Replay") -or
        (@($receipt.central_validation.workspace_package_selection) -join ',') -cne "--workspace" -or
        (@($receipt.central_validation.production_artifact_packages) -join ',') -cne ($script:ProductionArtifactPackages -join ',')) {
        throw "prebuilt artifact attestation was not emitted by a full centralized All-workspace Release"
    }
    if ([string]$receipt.central_validation.release_runtime_linkage.target -cne 'x86_64-pc-windows-msvc' -or
        [string]$receipt.central_validation.release_runtime_linkage.rust_target_feature -cne '+crt-static' -or
        [string]$receipt.central_validation.release_runtime_linkage.c_runtime -cne 'static' -or
        [string]$receipt.central_validation.release_runtime_linkage.admission -cne 'pe_import_and_delay_import_tables' -or
        [string]$receipt.central_validation.release_runtime_linkage.forbidden_import -cne 'VCRUNTIME*.dll') {
        throw "prebuilt artifact attestation has no static MSVC CRT release contract"
    }
    Test-ProductionBuildEnvironmentRecord -Record $receipt.build_environment
    if ($null -ne $ExpectedBuildEnvironment) {
        Test-ProductionBuildEnvironmentRecord -Record $ExpectedBuildEnvironment
        if ([string]$receipt.build_environment.sha256 -cne [string]$ExpectedBuildEnvironment.sha256) {
            throw 'live production build environment differs from the prebuilt artifact attestation'
        }
    }
    $expectedPaths = @(Get-ProductionArtifactPaths -RepoRoot $repo)
    if ([string]$receipt.artifact_freshness.schema -cne $script:ProductionArtifactFreshnessSchema -or
        [string]$receipt.artifact_freshness.policy -cne 'canonical_outputs_removed_and_absence_verified_before_build' -or
        [int]$receipt.artifact_freshness.artifact_count -ne $expectedPaths.Count -or
        (@($receipt.artifact_freshness.artifact_paths | ForEach-Object { [IO.Path]::GetFullPath([string]$_) }) -join '|') -ine ($expectedPaths -join '|')) {
        throw 'prebuilt artifact attestation has no complete Release output-freshness authority'
    }
    $runner = Join-Path $repo "extractor\native\scripts\Invoke-CentralRustValidation.ps1"
    if ([System.IO.Path]::GetFullPath([string]$receipt.central_validation.runner_path) -ine [System.IO.Path]::GetFullPath($runner) -or
        [string]$receipt.central_validation.runner_sha256 -cne (Get-FileHash -LiteralPath $runner -Algorithm SHA256).Hash.ToLowerInvariant()) {
        throw "centralized runner differs from the prebuilt artifact attestation"
    }
    if ([System.IO.Path]::GetFullPath([string]$receipt.capture_driver.path) -ine [System.IO.Path]::GetFullPath($ExpectedDriverPath) -or
        [string]$receipt.capture_driver.sha256 -cne (Get-FileHash -LiteralPath $ExpectedDriverPath -Algorithm SHA256).Hash.ToLowerInvariant()) {
        throw "production capture driver differs from the prebuilt artifact attestation"
    }
    $currentSource = Get-ProductionRustInputDigest -RepoRoot $repo
    if ([string]$receipt.source_input.schema -cne [string]$currentSource.schema -or
        [int]$receipt.source_input.file_count -ne [int]$currentSource.file_count -or
        [string]$receipt.source_input.sha256 -cne [string]$currentSource.sha256) {
        throw "current source/compiler inputs differ from the centralized prebuilt artifact attestation"
    }
    $expectedReleaseRoot = Join-Path $repo ".cargo-target\release"
    $rows = @($receipt.artifacts)
    if ($rows.Count -ne $script:ProductionArtifactRoles.Count) {
        throw "prebuilt artifact attestation has the wrong artifact count"
    }
    $resolved = [ordered]@{}
    $resolvedHashes = [ordered]@{}
    $resolvedImports = [ordered]@{}
    $index = 0
    foreach ($entry in $script:ProductionArtifactRoles.GetEnumerator()) {
        $matches = @($rows | Where-Object { [string]$_.role -ceq [string]$entry.Key })
        if ($matches.Count -ne 1) { throw "prebuilt artifact role is missing or duplicated: $($entry.Key)" }
        $row = $matches[0]
        $expectedPath = [System.IO.Path]::GetFullPath((Join-Path $expectedReleaseRoot $entry.Value))
        if ([string]$row.package -cne [string]$script:ProductionArtifactPackages[$index] -or
            [System.IO.Path]::GetFullPath([string]$row.path) -ine $expectedPath -or
            -not (Test-Path -LiteralPath $expectedPath -PathType Leaf)) {
            throw "prebuilt artifact identity differs for role $($entry.Key)"
        }
        $item = Get-Item -LiteralPath $expectedPath
        if ([int64]$row.bytes -ne [int64]$item.Length -or [int64]$item.Length -le 0 -or
            [string]$row.sha256 -cne (Get-FileHash -LiteralPath $expectedPath -Algorithm SHA256).Hash.ToLowerInvariant()) {
            throw "prebuilt artifact bytes differ for role $($entry.Key)"
        }
        if ([string]$entry.Key -in $script:PortableProductArtifactRoles) {
            $resolvedImports[[string]$entry.Key] = Assert-PortableProductPeImportAttestation `
                -Bytes ([System.IO.File]::ReadAllBytes($expectedPath)) `
                -Name ([string]$entry.Value) `
                -Attestation $row.pe_imports
        }
        $resolved[[string]$entry.Key] = $expectedPath
        $resolvedHashes[[string]$entry.Key] = [string]$row.sha256
        $index++
    }
    return [pscustomobject]@{
        receipt_path = [System.IO.Path]::GetFullPath($ReceiptPath)
        receipt_sha256 = (Get-FileHash -LiteralPath $ReceiptPath -Algorithm SHA256).Hash.ToLowerInvariant()
        source_input_sha256 = [string]$currentSource.sha256
        build_environment = $receipt.build_environment
        artifacts = $resolved
        artifact_sha256 = $resolvedHashes
        artifact_pe_imports = $resolvedImports
        release_runtime_linkage = $receipt.central_validation.release_runtime_linkage
    }
}
