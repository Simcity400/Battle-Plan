[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = (Resolve-Path (Join-Path $scriptDir '..\..\..')).Path
$builderSource = Join-Path $scriptDir 'New-PortableProductPackage.ps1'
$verifierSource = Join-Path $scriptDir 'Test-PortableProductPackageReceipt.ps1'
$attestationSource = Join-Path $scriptDir 'ProductionArtifact.Attestation.ps1'

function Assert-Case {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][bool]$Condition
    )
    if (-not $Condition) { throw "portable package self-test failed: $Name" }
}

function Invoke-ExpectFailure {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][scriptblock]$Body
    )
    try {
        & $Body
    }
    catch {
        return
    }
    throw "portable package self-test expected failure: $Name"
}

function Set-U16 {
    param([byte[]]$Bytes, [int]$Offset, [uint16]$Value)
    [Array]::Copy([BitConverter]::GetBytes($Value), 0, $Bytes, $Offset, 2)
}

function Set-U32 {
    param([byte[]]$Bytes, [int]$Offset, [uint32]$Value)
    [Array]::Copy([BitConverter]::GetBytes($Value), 0, $Bytes, $Offset, 4)
}

function New-MinimalAmd64Pe {
    param(
        [Parameter(Mandatory = $true)][bool]$Dll,
        [Parameter(Mandatory = $true)][bool]$Gui,
        [Parameter(Mandatory = $true)][byte]$Marker
    )
    $bytes = [byte[]]::new(512)
    Set-U16 $bytes 0 0x5a4d
    Set-U32 $bytes 0x3c 0x80
    Set-U32 $bytes 0x80 0x00004550
    Set-U16 $bytes 0x84 0x8664
    Set-U16 $bytes 0x86 1
    Set-U16 $bytes 0x94 0xf0
    $characteristics = if ($Dll) { 0x2002 } else { 0x0002 }
    Set-U16 $bytes 0x96 $characteristics
    Set-U16 $bytes 0x98 0x020b
    Set-U16 $bytes 0xdc $(if ($Gui) { 2 } else { 3 })
    $bytes[$bytes.Length - 1] = $Marker
    return ,$bytes
}

function Write-Bytes {
    param([string]$Path, [byte[]]$Bytes)
    [IO.Directory]::CreateDirectory((Split-Path -Parent $Path)) | Out-Null
    [IO.File]::WriteAllBytes($Path, $Bytes)
}

function Invoke-JsonScript {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string[]]$Arguments
    )
    $stdout = @(& powershell.exe -NoProfile -ExecutionPolicy Bypass -File $Path @Arguments)
    if ($LASTEXITCODE -ne 0) {
        throw "fixture script exited $LASTEXITCODE`: $Path"
    }
    return (($stdout -join [Environment]::NewLine).Trim() | ConvertFrom-Json)
}

$root = Join-Path ([IO.Path]::GetTempPath()) ("hoi4-portable-package-selftest-{0}" -f [guid]::NewGuid().ToString('N'))
try {
    $fixtureScripts = Join-Path $root 'extractor\native\scripts'
    $releaseRoot = Join-Path $root '.cargo-target\release'
    [IO.Directory]::CreateDirectory($fixtureScripts) | Out-Null
    [IO.Directory]::CreateDirectory((Join-Path $root 'portable_replay\native')) | Out-Null
    [IO.Directory]::CreateDirectory((Join-Path $root 'replay_interface\native')) | Out-Null
    [IO.Directory]::CreateDirectory($releaseRoot) | Out-Null

    foreach ($name in @(
        'Hoi4ClientDriving.OptIn.ps1',
        'PortableProduct.PeImports.ps1',
        'New-PortableProductPackage.ps1',
        'Test-PortableProductPackageReceipt.ps1',
        'ProductionArtifact.Attestation.ps1'
    )) {
        Copy-Item -LiteralPath (Join-Path $scriptDir $name) -Destination (Join-Path $fixtureScripts $name)
    }
    # The attestation binds these exact production paths. Their fixture bodies
    # are inert but content-addressed, so the real helper still validates every
    # path/hash/source-digest relationship.
    [IO.File]::WriteAllText(
        (Join-Path $fixtureScripts 'Invoke-CentralRustValidation.ps1'),
        "throw 'fixture runner must never execute'`n",
        [Text.UTF8Encoding]::new($false)
    )
    [IO.File]::WriteAllText(
        (Join-Path $fixtureScripts 'Start-Hoi4ProductionCaptureSafe.ps1'),
        "throw 'fixture driver must never execute'`n",
        [Text.UTF8Encoding]::new($false)
    )

    $fixtureBuilder = Join-Path $fixtureScripts 'New-PortableProductPackage.ps1'
    $fixtureVerifier = Join-Path $fixtureScripts 'Test-PortableProductPackageReceipt.ps1'
    $fixtureHelper = Join-Path $fixtureScripts 'ProductionArtifact.Attestation.ps1'
    . $fixtureHelper
    $freshness = Clear-ProductionArtifactOutputs -RepoRoot $root

    $recorder = Join-Path $releaseRoot 'collector_record.exe'
    $replay = Join-Path $releaseRoot 'replay_app.exe'
    $collector = Join-Path $releaseRoot 'collector.dll'
    Write-Bytes $recorder (New-MinimalAmd64Pe -Dll $false -Gui $true -Marker 1)
    Write-Bytes $replay (New-MinimalAmd64Pe -Dll $false -Gui $true -Marker 2)
    Write-Bytes $collector (New-MinimalAmd64Pe -Dll $true -Gui $false -Marker 3)
    foreach ($name in @('fold_check.exe', 'compact_replay_gate.exe', 'pack_replay.exe')) {
        Write-Bytes (Join-Path $releaseRoot $name) ([Text.Encoding]::ASCII.GetBytes("fixture-$name"))
    }

    $zip = Join-Path $releaseRoot 'battle-plan-0.1.0-windows-x86_64.zip'
    $builderResult = Invoke-JsonScript -Path $fixtureBuilder -Arguments @(
        '-RecorderExe', $recorder,
        '-ReplayExe', $replay,
        '-CollectorDll', $collector,
        '-OutputZip', $zip,
        '-Version', '0.1.0'
    )
    Assert-Case 'builder emits exact schema' ([string]$builderResult.schema -ceq 'hoi4-portable-package.v1')
    Assert-Case 'builder emits exact package path' ([IO.Path]::GetFullPath([string]$builderResult.path) -ieq [IO.Path]::GetFullPath($zip))
    Assert-Case 'builder emits two GUI executables' ([int]$builderResult.executable_count -eq 2)

    $hexA = 'a' * 64
    $hexB = 'b' * 64
    $hexC = 'c' * 64
    $commitHash = 'd' * 40
    $buildEnvValues = [ordered]@{}
    foreach ($key in $script:ProductionBuildEnvironmentKeys) { $buildEnvValues[$key] = '' }
    $buildEnvValues.CARGO_INCREMENTAL = '0'
    $buildEnvValues.CARGO_TARGET_DIR = Join-Path $root '.cargo-target'
    $buildEnvValues.CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER = 'synthetic-lld-link.exe'
    $buildEnvValues.PATH_SHA256 = $hexC
    $buildEnvironment = New-ProductionBuildEnvironmentRecord `
        -RustcVerbose "rustc 1.96.0`nhost: x86_64-pc-windows-msvc`nrelease: 1.96.0`ncommit-hash: $commitHash" `
        -CargoVersion 'cargo 1.96.0' `
        -Linker ([pscustomobject][ordered]@{
            configured = 'synthetic-lld-link.exe'; version_output = 'LLD 22.1.0 synthetic'
            executable = [pscustomobject]@{ path = 'C:\synthetic\lld-link.exe'; bytes = 1; sha256 = $hexA; file_version = '22.1'; product_version = '22.1' }
        }) `
        -RustcWrapper ([pscustomobject][ordered]@{ active = $false; configured = ''; executable = $null }) `
        -MsvcToolset ([pscustomobject][ordered]@{
            discovery = 'synthetic'; version = '14.50.00000'; root = 'C:\synthetic\MSVC'
            compiler = [pscustomobject]@{ path = 'C:\synthetic\cl.exe'; bytes = 1; sha256 = $hexB; file_version = '19.50'; product_version = '19.50' }
            msvc_linker = [pscustomobject]@{ path = 'C:\synthetic\link.exe'; bytes = 1; sha256 = $hexC; file_version = '14.50'; product_version = '14.50' }
        }) `
        -WindowsSdk ([pscustomobject][ordered]@{
            root = 'C:\synthetic\Windows Kits\10'; version = '10.0.26100.0'
            windows_sdk_dir_env = ''; windows_sdk_version_env = ''; ucrt_version_env = ''
            kernel32_import_library = [pscustomobject]@{ path = 'C:\synthetic\kernel32.lib'; bytes = 1; sha256 = $hexA; file_version = ''; product_version = '' }
            ucrt_import_library = [pscustomobject]@{ path = 'C:\synthetic\ucrt.lib'; bytes = 1; sha256 = $hexB; file_version = ''; product_version = '' }
        }) `
        -Runner ([pscustomobject][ordered]@{
            image_os = 'synthetic'; image_version = '1'; runner_os = 'Windows'; runner_arch = 'X64'
            runner_environment = 'self-test'; os_description = 'synthetic Windows'
            os_architecture = 'X64'; process_architecture = 'X64'
        }) `
        -BuildEnv $buildEnvValues
    $runtimeLinkage = [pscustomobject][ordered]@{
        target = 'x86_64-pc-windows-msvc'; rust_target_feature = '+crt-static'; c_runtime = 'static'
        admission = 'pe_import_and_delay_import_tables'; forbidden_import = 'VCRUNTIME*.dll'
    }
    $sourceDigest = Get-ProductionRustInputDigest -RepoRoot $root
    $attestationPath = Join-Path $releaseRoot 'hoi4-production-prebuilt-artifacts.json'
    Write-ProductionArtifactAttestation `
        -RepoRoot $root `
        -ReceiptPath $attestationPath `
        -RequestedWorkspace All `
        -SourceDigest $sourceDigest `
        -ReleaseRuntimeLinkage $runtimeLinkage `
        -BuildEnvironment $buildEnvironment `
        -ArtifactFreshness $freshness
    $driver = Join-Path $fixtureScripts 'Start-Hoi4ProductionCaptureSafe.ps1'
    $attested = Test-ProductionArtifactAttestation -RepoRoot $root -ReceiptPath $attestationPath -ExpectedDriverPath $driver

    $packageReceiptPath = Join-Path $releaseRoot 'hoi4-portable-package-receipt.json'
    $receipt = [ordered]@{
        schema = 'battle-plan.central_portable_package.v1'
        status = 'PASS_CENTRAL_RELEASE_PORTABLE_PACKAGE'
        successful_exit = $true
        central_validation = [ordered]@{
            mode = 'Release'
            requested_workspace = 'All'
            release_runtime_linkage = $runtimeLinkage
        }
        artifact_attestation = [ordered]@{
            path = [string]$attested.receipt_path
            sha256 = [string]$attested.receipt_sha256
        }
        package_builder = [ordered]@{
            path = [IO.Path]::GetFullPath($fixtureBuilder)
            sha256 = (Get-FileHash -LiteralPath $fixtureBuilder -Algorithm SHA256).Hash.ToLowerInvariant()
        }
        package = $builderResult
        source_input_sha256 = [string]$attested.source_input_sha256
        build_environment = $attested.build_environment
    }
    [IO.File]::WriteAllText(
        $packageReceiptPath,
        ($receipt | ConvertTo-Json -Depth 12),
        [Text.UTF8Encoding]::new($false)
    )

    $verified = Invoke-JsonScript -Path $fixtureVerifier -Arguments @('-RepoRoot', $root, '-ReceiptPath', $packageReceiptPath)
    Assert-Case 'verifier accepts authentic fixture' ([string]$verified.status -ceq 'PASS_CENTRAL_RELEASE_PORTABLE_PACKAGE')
    Assert-Case 'verifier returns builder package hash' ([string]$verified.package_sha256 -ceq [string]$builderResult.sha256)

    $zipOriginal = [IO.File]::ReadAllBytes($zip)
    $zipTampered = [byte[]]$zipOriginal.Clone()
    $zipTampered[$zipTampered.Length - 1] = $zipTampered[$zipTampered.Length - 1] -bxor 0xff
    [IO.File]::WriteAllBytes($zip, $zipTampered)
    Invoke-ExpectFailure 'tampered package bytes are rejected' {
        Invoke-JsonScript -Path $fixtureVerifier -Arguments @('-RepoRoot', $root, '-ReceiptPath', $packageReceiptPath) | Out-Null
    }
    [IO.File]::WriteAllBytes($zip, $zipOriginal)

    $receiptOriginal = [IO.File]::ReadAllText($packageReceiptPath)
    $tamperedReceipt = $receiptOriginal | ConvertFrom-Json
    $tamperedReceipt.status = 'PASS_WITHOUT_AUTHORITY'
    [IO.File]::WriteAllText(
        $packageReceiptPath,
        ($tamperedReceipt | ConvertTo-Json -Depth 12),
        [Text.UTF8Encoding]::new($false)
    )
    Invoke-ExpectFailure 'tampered central receipt is rejected' {
        Invoke-JsonScript -Path $fixtureVerifier -Arguments @('-RepoRoot', $root, '-ReceiptPath', $packageReceiptPath) | Out-Null
    }
    [IO.File]::WriteAllText($packageReceiptPath, $receiptOriginal, [Text.UTF8Encoding]::new($false))

    $wrongKindZip = Join-Path $releaseRoot 'wrong-kind.zip'
    Invoke-ExpectFailure 'DLL cannot masquerade as recorder executable' {
        Invoke-JsonScript -Path $fixtureBuilder -Arguments @(
            '-RecorderExe', $collector,
            '-ReplayExe', $replay,
            '-CollectorDll', $collector,
            '-OutputZip', $wrongKindZip,
            '-Version', '0.1.0'
        ) | Out-Null
    }

    $cleanupZip = Join-Path $releaseRoot 'post-temp-cleanup.zip'
    Invoke-ExpectFailure 'post-temp builder failure is rejected' {
        Invoke-JsonScript -Path $fixtureBuilder -Arguments @(
            '-RecorderExe', $recorder,
            '-ReplayExe', $replay,
            '-CollectorDll', $collector,
            '-OutputZip', $cleanupZip,
            '-Version', '0.1.0',
            '-TestOnlyFailAfterTemporaryWrite'
        ) | Out-Null
    }
    Assert-Case 'post-temp failure publishes no package' (-not (Test-Path -LiteralPath $cleanupZip))
    $cleanupTemporary = @(
        Get-ChildItem -LiteralPath $releaseRoot -Filter '.post-temp-cleanup.zip.*.tmp' -Force
    )
    Assert-Case 'post-temp failure removes unpublished temporary package' ($cleanupTemporary.Count -eq 0)

    [pscustomobject]@{
        schema = 'battle-plan.portable_package_selftest.v1'
        status = 'PASS'
        cases = 11
    } | ConvertTo-Json -Depth 3
}
finally {
    if (Test-Path -LiteralPath $root) {
        Remove-Item -LiteralPath $root -Recurse -Force
    }
}
