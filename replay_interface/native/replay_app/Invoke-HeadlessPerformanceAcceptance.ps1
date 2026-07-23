[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)] [string]$ReplayApp,
    [Parameter(Mandatory = $true)] [string]$ReleaseArtifactReceipt,
    [Parameter(Mandatory = $true)] [string]$Output
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

function Get-StreamSha256 {
    param([Parameter(Mandatory = $true)] [IO.Stream]$Stream)
    $Stream.Position = 0
    $hasher = [Security.Cryptography.SHA256]::Create()
    try {
        return ([BitConverter]::ToString($hasher.ComputeHash($Stream))).Replace('-', '').ToLowerInvariant()
    }
    finally {
        $hasher.Dispose()
        $Stream.Position = 0
    }
}

function Get-BytesSha256 {
    param([Parameter(Mandatory = $true)] [byte[]]$Bytes)
    $hasher = [Security.Cryptography.SHA256]::Create()
    try { return ([BitConverter]::ToString($hasher.ComputeHash($Bytes))).Replace('-', '').ToLowerInvariant() }
    finally { $hasher.Dispose() }
}

function Write-ImmutableJson {
    param(
        [Parameter(Mandatory = $true)] [string]$Path,
        [Parameter(Mandatory = $true)] [object]$Value
    )
    $absolute = [IO.Path]::GetFullPath($Path)
    if ([IO.File]::Exists($absolute)) {
        throw "acceptance receipt already exists; refusing to replace $absolute"
    }
    $parent = [IO.Path]::GetDirectoryName($absolute)
    if ([string]::IsNullOrWhiteSpace($parent)) { throw 'acceptance receipt has no parent directory' }
    [IO.Directory]::CreateDirectory($parent) | Out-Null
    $temporary = "$absolute.incoming.$PID.$([Guid]::NewGuid().ToString('N'))"
    try {
        $bytes = [Text.UTF8Encoding]::new($false).GetBytes((($Value | ConvertTo-Json -Depth 48) + "`n"))
        $stream = [IO.File]::Open($temporary, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
        try {
            $stream.Write($bytes, 0, $bytes.Length)
            $stream.Flush($true)
        }
        finally { $stream.Dispose() }
        [IO.File]::Move($temporary, $absolute)
    }
    finally {
        if ([IO.File]::Exists($temporary)) { [IO.File]::Delete($temporary) }
    }
}

function Assert-HexSha256 {
    param([string]$Value, [string]$Label)
    if ($Value -cnotmatch '^[0-9a-f]{64}$') { throw "$Label is not a lowercase SHA-256" }
}

function Assert-Equal {
    param($Actual, $Expected, [string]$Label)
    if ([string]$Actual -cne [string]$Expected) {
        throw "$Label differs: expected $Expected, observed $Actual"
    }
}

function Assert-SamePath {
    param([string]$Actual, [string]$Expected, [string]$Label)
    $actualFull = [IO.Path]::GetFullPath($Actual)
    $expectedFull = [IO.Path]::GetFullPath($Expected)
    if (-not $actualFull.Equals($expectedFull, [StringComparison]::OrdinalIgnoreCase)) {
        throw "$Label differs: expected $expectedFull, observed $actualFull"
    }
}

function Get-NearestRank {
    param([double[]]$Sorted, [double]$Quantile)
    if ($Sorted.Count -eq 0) { throw 'cannot compute a percentile from zero samples' }
    $rank = [Math]::Ceiling($Quantile * $Sorted.Count)
    return $Sorted[[Math]::Min($Sorted.Count - 1, [Math]::Max(0, [int]$rank - 1))]
}

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..\..\..')).Path
$workloadContractPath = Join-Path $PSScriptRoot 'headless_performance_workload.json'
$attestationHelper = Join-Path $repoRoot 'extractor\native\scripts\ProductionArtifact.Attestation.ps1'
$expectedDriver = Join-Path $repoRoot 'extractor\native\scripts\Start-Hoi4ProductionCaptureSafe.ps1'
. $attestationHelper

$outputPath = [IO.Path]::GetFullPath($Output)
$rawPath = "$outputPath.renderer.json"
$failures = [Collections.Generic.List[string]]::new()
$rendererOutput = [Collections.Generic.List[string]]::new()
$rendererExitCode = $null
$rendererReceipt = $null
$rendererReceiptBytes = $null
$rendererReceiptSha256 = $null
$machine = $null
$replayAppPath = $null
$preHash = $null
$postHash = $null
$attestation = $null
$workloadContract = $null
$workloadContractSha256 = $null
$exeLock = $null

try {
    if ([IO.File]::Exists($outputPath) -or [IO.File]::Exists($rawPath)) {
        throw 'output or retained renderer receipt already exists; refusing replacement'
    }
    $replayAppPath = [IO.Path]::GetFullPath($ReplayApp)
    if (-not [IO.File]::Exists($replayAppPath)) { throw "replay executable is absent: $replayAppPath" }
    if (-not [IO.File]::Exists($workloadContractPath)) { throw 'checked-in workload contract is absent' }
    $workloadContractBytes = [IO.File]::ReadAllBytes($workloadContractPath)
    $workloadContractSha256 = Get-BytesSha256 -Bytes $workloadContractBytes
    $workloadContract = [Text.UTF8Encoding]::new($false, $true).GetString($workloadContractBytes) | ConvertFrom-Json
    Assert-Equal $workloadContract.schema 'battle-plan-headless-performance-stress-workload-v1' 'workload schema'
    if ($workloadContract.certification_enabled -isnot [bool] -or -not $workloadContract.certification_enabled) {
        throw 'checked-in workload is not certification-enabled'
    }
    Assert-Equal $workloadContract.workload_id 'F1_A729_1920X1080_PERFORMANCE_STRESS_V1' 'workload id'
    Assert-Equal $workloadContract.proof_scope 'performance_only_non_semantic_non_promotion' 'workload proof scope'
    Assert-Equal (@($workloadContract.prohibited_claims) -join ',') 'replay_semantic_parity,rendering_law_promotion,reverse_engineering_evidence,pixel_parity' 'prohibited claims'
    Assert-Equal $workloadContract.viewport_width 1920 'contract viewport width'
    Assert-Equal $workloadContract.viewport_height 1080 'contract viewport height'
    Assert-Equal $workloadContract.map_operation_count 1266 'contract map operation count'
    Assert-Equal $workloadContract.selected_present_ui_operation_count 1167 'contract UI operation count'
    Assert-Equal (@($workloadContract.approved_omissions).Count) 0 'contract omission count'
    foreach ($field in @(
        'shader_asset_sha256',
        'synthetic_product_modset_sha256',
        'synthetic_effective_vfs_manifest_sha256',
        'ui_surface_sha256'
    )) {
        Assert-HexSha256 ([string]$workloadContract.$field) "workload $field"
    }
    Assert-Equal $workloadContract.synthetic_product_modset_sha256 'b1836cbf1d952178865b910977dc880296ade11e0df3ebb520c40abb94a3f900' 'synthetic product-modset identity'
    Assert-Equal $workloadContract.synthetic_effective_vfs_manifest_sha256 '217adb01b676c3677fbf605be059298f82d5144f5e2b1cde9d51b5bc186593f0' 'synthetic effective-VFS identity'
    Assert-Equal $workloadContract.ui_surface_algorithm 'rgba32_64_96_16_repeated_1920x1080' 'UI surface algorithm'
    Assert-Equal $workloadContract.ui_surface_sha256 'fe596b4cf34242b20a49f3d5ba95686cd2103097fb314ebc637b0f74f6988a41' 'UI surface identity'

    $mapAuthorityRelative = 'replay_interface/re_evidence/rendering_laws/XN15L/stage_so_census.json'
    $uiAuthorityRelative = 'replay_interface/re_evidence/rendering_laws/M02/production_interleave_closure/contract.json'
    Assert-Equal $workloadContract.map_count_authority.repository_relative_path $mapAuthorityRelative 'map authority path'
    Assert-Equal $workloadContract.map_count_authority.json_pointer '/summary/all/draw_count' 'map authority pointer'
    Assert-Equal $workloadContract.map_count_authority.expected_count 1266 'map authority count'
    Assert-Equal $workloadContract.ui_count_authority.repository_relative_path $uiAuthorityRelative 'UI authority path'
    Assert-Equal $workloadContract.ui_count_authority.json_pointer '/authenticated_m02_draw/visible_ui_pass_draw_count' 'UI authority pointer'
    Assert-Equal $workloadContract.ui_count_authority.expected_count 1167 'UI authority count'
    $mapAuthorityPath = Join-Path $repoRoot $mapAuthorityRelative
    $uiAuthorityPath = Join-Path $repoRoot $uiAuthorityRelative
    Assert-Equal (Get-FileHash -LiteralPath $mapAuthorityPath -Algorithm SHA256).Hash.ToLowerInvariant() $workloadContract.map_count_authority.sha256 'map authority hash'
    Assert-Equal (Get-FileHash -LiteralPath $uiAuthorityPath -Algorithm SHA256).Hash.ToLowerInvariant() $workloadContract.ui_count_authority.sha256 'UI authority hash'
    $mapAuthority = Get-Content -LiteralPath $mapAuthorityPath -Raw -Encoding UTF8 | ConvertFrom-Json
    $uiAuthority = Get-Content -LiteralPath $uiAuthorityPath -Raw -Encoding UTF8 | ConvertFrom-Json
    Assert-Equal $mapAuthority.summary.all.draw_count 1266 'observed map authority count'
    Assert-Equal $uiAuthority.authenticated_m02_draw.visible_ui_pass_draw_count 1167 'observed UI authority count'
    Assert-Equal $workloadContract.shader_asset_path 'performance_assets/f1_stress.hlsl' 'shader asset path'
    $shaderAssetPath = Join-Path $PSScriptRoot $workloadContract.shader_asset_path
    Assert-Equal (Get-FileHash -LiteralPath $shaderAssetPath -Algorithm SHA256).Hash.ToLowerInvariant() $workloadContract.shader_asset_sha256 'shader asset hash'

    $attestation = Test-ProductionArtifactAttestation `
        -RepoRoot $repoRoot `
        -ReceiptPath ([IO.Path]::GetFullPath($ReleaseArtifactReceipt)) `
        -ExpectedDriverPath $expectedDriver
    $attestedReplayPath = [IO.Path]::GetFullPath([string]$attestation.artifacts.replay_app)
    $attestedReplaySha256 = [string]$attestation.artifact_sha256.replay_app
    Assert-HexSha256 $attestedReplaySha256 'attested replay executable hash'
    if (-not $attestedReplayPath.Equals($replayAppPath, [StringComparison]::OrdinalIgnoreCase)) {
        throw 'ReplayApp is not the replay_app artifact in the centralized Release receipt'
    }

    try {
        $processors = @(Get-CimInstance -ClassName Win32_Processor)
        $computer = Get-CimInstance -ClassName Win32_ComputerSystem
        $videoControllers = @(Get-CimInstance -ClassName Win32_VideoController)
        if ($processors.Count -eq 0 -or $null -eq $computer -or $videoControllers.Count -eq 0) {
            throw 'CIM returned incomplete CPU, memory, or GPU inventory'
        }
        $cpuNames = @($processors | ForEach-Object { [string]$_.Name })
        $gpuRows = @($videoControllers | ForEach-Object {
            [ordered]@{ name = [string]$_.Name; driver_version = [string]$_.DriverVersion }
        })
        $physicalMemoryBytes = [uint64]$computer.TotalPhysicalMemory
        $nominalMemoryGiB = [math]::Round($physicalMemoryBytes / 1GB)
        $cpuEligible = @($cpuNames | Where-Object {
            $_ -match '(?i)Intel\(R\).*Core\(TM\).*i5-8400\b' -or $_ -match '(?i)AMD Ryzen 5 2600\b'
        }).Count -gt 0
        $memoryEligible = $nominalMemoryGiB -eq 16
        $machine = [ordered]@{
            cpu_names = $cpuNames
            physical_memory_bytes = $physicalMemoryBytes
            nominal_memory_gib = $nominalMemoryGiB
            video_controllers = $gpuRows
            cpu_contract_met = $cpuEligible
            memory_contract_met = $memoryEligible
        }
        if (-not $cpuEligible) { throw 'reference CPU absent: requires i5-8400 or Ryzen 5 2600' }
        if (-not $memoryEligible) { throw "reference memory absent: requires nominal 16 GiB, observed $nominalMemoryGiB" }
    }
    catch { throw "machine inventory rejected: $($_.Exception.Message)" }

    # Hold a deny-write/delete handle across launch. The two separately
    # calculated hashes prove both sides of execution and the open handle makes
    # a restore-after-substitution race impossible.
    $exeLock = [IO.File]::Open($replayAppPath, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
    $preHash = Get-StreamSha256 -Stream $exeLock
    Assert-Equal $preHash $attestedReplaySha256 'pre-execution executable hash'

    $arguments = [Collections.Generic.List[string]]::new()
    foreach ($item in @(
        '--benchmark-contract', $workloadContractPath,
        '--benchmark-warmup-frames', '120',
        '--benchmark-sample-frames', '600',
        '--benchmark-stress-output', $rawPath,
        '--non-interactive'
    )) { $arguments.Add($item) }

    & $replayAppPath @arguments 2>&1 | ForEach-Object { $rendererOutput.Add([string]$_) }
    $rendererExitCode = $LASTEXITCODE
    $postHash = Get-StreamSha256 -Stream $exeLock
    Assert-Equal $postHash $preHash 'post-execution executable hash'
    if ([IO.File]::Exists($rawPath)) {
        $rendererReceiptBytes = [IO.File]::ReadAllBytes($rawPath)
        $rendererReceiptSha256 = Get-BytesSha256 -Bytes $rendererReceiptBytes
        $rendererReceipt = [Text.UTF8Encoding]::new($false, $true).GetString($rendererReceiptBytes) | ConvertFrom-Json
    }
    if ($rendererExitCode -ne 0) { throw "renderer exited with code $rendererExitCode" }
    if ($null -eq $rendererReceipt) { throw 'renderer emitted no retained raw receipt' }

    Assert-Equal $rendererReceipt.schema 'battle-plan-headless-performance-stress-bench-v1' 'renderer schema'
    Assert-Equal $rendererReceipt.result 'pass' 'renderer result'
    if (@($rendererReceipt.failures).Count -ne 0) { throw 'passing renderer receipt contains failures' }
    Assert-Equal $rendererReceipt.reference_machine_contract 'i5-8400_or_Ryzen_5_2600;16_GiB;GTX_1060_6_GiB_or_RX_580_8_GiB;1920x1080' 'renderer machine contract'
    Assert-Equal $rendererReceipt.thresholds.average_fps_min 60 'average FPS threshold'
    if ([math]::Abs([double]$rendererReceipt.thresholds.average_frame_time_ms_max - (1000.0 / 60.0)) -gt 0.000000001) { throw 'average frame-time threshold differs' }
    Assert-Equal $rendererReceipt.thresholds.p95_frame_time_ms_max 33.3 'p95 threshold'
    Assert-Equal $rendererReceipt.workload.viewport_width 1920 'viewport width'
    Assert-Equal $rendererReceipt.workload.viewport_height 1080 'viewport height'
    Assert-SamePath $rendererReceipt.workload.workload_contract_path $workloadContractPath 'renderer workload-contract path'
    Assert-Equal $rendererReceipt.workload.workload_id $workloadContract.workload_id 'workload id'
    Assert-Equal $rendererReceipt.workload.workload_contract_sha256 $workloadContractSha256 'workload contract hash'
    Assert-Equal $rendererReceipt.workload.proof_scope $workloadContract.proof_scope 'renderer proof scope'
    Assert-Equal (@($rendererReceipt.workload.prohibited_claims) -join ',') (@($workloadContract.prohibited_claims) -join ',') 'renderer prohibited claims'
    Assert-Equal $rendererReceipt.workload.map_count_authority.repository_relative_path $workloadContract.map_count_authority.repository_relative_path 'renderer map-count authority path'
    Assert-Equal $rendererReceipt.workload.map_count_authority.sha256 $workloadContract.map_count_authority.sha256 'renderer map-count authority'
    Assert-Equal $rendererReceipt.workload.map_count_authority.json_pointer $workloadContract.map_count_authority.json_pointer 'renderer map-count authority pointer'
    Assert-Equal $rendererReceipt.workload.map_count_authority.expected_count $workloadContract.map_count_authority.expected_count 'renderer map-count authority count'
    Assert-Equal $rendererReceipt.workload.ui_count_authority.repository_relative_path $workloadContract.ui_count_authority.repository_relative_path 'renderer UI-count authority path'
    Assert-Equal $rendererReceipt.workload.ui_count_authority.sha256 $workloadContract.ui_count_authority.sha256 'renderer UI-count authority'
    Assert-Equal $rendererReceipt.workload.ui_count_authority.json_pointer $workloadContract.ui_count_authority.json_pointer 'renderer UI-count authority pointer'
    Assert-Equal $rendererReceipt.workload.ui_count_authority.expected_count $workloadContract.ui_count_authority.expected_count 'renderer UI-count authority count'
    Assert-SamePath $rendererReceipt.workload.shader_asset_path $shaderAssetPath 'renderer shader-asset path'
    Assert-Equal $rendererReceipt.workload.shader_asset_sha256 $workloadContract.shader_asset_sha256 'renderer shader asset'
    Assert-HexSha256 $rendererReceipt.workload.compiled_vertex_shader_sha256 'compiled vertex shader'
    Assert-HexSha256 $rendererReceipt.workload.compiled_pixel_shader_sha256 'compiled pixel shader'
    Assert-HexSha256 $rendererReceipt.workload.content_manifest_sha256 'synthetic content manifest'
    Assert-HexSha256 $rendererReceipt.workload.prepared_frame_sha256 'prepared stress frame'
    Assert-HexSha256 $rendererReceipt.workload.production_d3d_receipt_sha256 'production D3D receipt'
    Assert-Equal $rendererReceipt.workload.product_modset_sha256 $workloadContract.synthetic_product_modset_sha256 'synthetic product modset'
    Assert-Equal $rendererReceipt.workload.effective_vfs_manifest_sha256 $workloadContract.synthetic_effective_vfs_manifest_sha256 'synthetic effective VFS'
    Assert-Equal $rendererReceipt.workload.ui_surface_algorithm $workloadContract.ui_surface_algorithm 'renderer UI algorithm'
    Assert-Equal $rendererReceipt.workload.ui_surface_sha256 $workloadContract.ui_surface_sha256 'renderer UI surface'
    Assert-Equal $rendererReceipt.workload.warmup_frames 120 'warmup frame count'
    Assert-Equal $rendererReceipt.workload.sample_frames 600 'requested sample frame count'
    Assert-Equal $rendererReceipt.workload.map_operations_per_frame $workloadContract.map_operation_count 'map operation count'
    Assert-Equal $rendererReceipt.workload.selected_present_operations_per_frame $workloadContract.selected_present_ui_operation_count 'selected-Present operation count'
    Assert-Equal $rendererReceipt.workload.feature_local_omissions 0 'stress omission count'
    Assert-Equal $rendererReceipt.workload.gpu_completion_method 'D3D11_QUERY_EVENT_after_each_frame' 'GPU completion method'
    Assert-Equal $rendererReceipt.workload.timer 'std::time::Instant_wall_clock' 'frame timer'
    Assert-Equal $rendererReceipt.metrics.sample_count 600 'sample count'
    Assert-Equal $rendererReceipt.metrics.percentile_method 'nearest_rank_sorted_ceil_q_times_n' 'percentile method'
    $samples = @($rendererReceipt.metrics.frame_times_ms | ForEach-Object { [double]$_ })
    if ($samples.Count -ne 600 -or @($samples | Where-Object { [double]::IsNaN($_) -or [double]::IsInfinity($_) -or $_ -le 0 }).Count -ne 0) {
        throw 'renderer samples are absent, non-finite, non-positive, or not exactly 600'
    }
    $average = ($samples | Measure-Object -Average).Average
    $sorted = [double[]]@($samples | Sort-Object)
    $p50 = Get-NearestRank -Sorted $sorted -Quantile 0.50
    $p95 = Get-NearestRank -Sorted $sorted -Quantile 0.95
    $p99 = Get-NearestRank -Sorted $sorted -Quantile 0.99
    $maximum = $sorted[$sorted.Count - 1]
    $averageFps = 1000.0 / $average
    if ([math]::Abs($average - [double]$rendererReceipt.metrics.average_frame_time_ms) -gt 0.000000001 -or
        [math]::Abs($averageFps - [double]$rendererReceipt.metrics.average_fps) -gt 0.000000001 -or
        [math]::Abs($p50 - [double]$rendererReceipt.metrics.p50_frame_time_ms) -gt 0.000000001 -or
        [math]::Abs($p95 - [double]$rendererReceipt.metrics.p95_frame_time_ms) -gt 0.000000001 -or
        [math]::Abs($p99 - [double]$rendererReceipt.metrics.p99_frame_time_ms) -gt 0.000000001 -or
        [math]::Abs($maximum - [double]$rendererReceipt.metrics.max_frame_time_ms) -gt 0.000000001 -or
        $average -gt (1000.0 / 60.0) -or $p95 -gt 33.3) {
        throw 'renderer sample recomputation or performance thresholds failed'
    }
    $vendor = [uint32]$rendererReceipt.workload.gpu_vendor_id
    $device = [uint32]$rendererReceipt.workload.gpu_device_id
    $dedicated = [uint64]$rendererReceipt.workload.gpu_dedicated_video_memory_bytes
    if ([string]::IsNullOrWhiteSpace([string]$rendererReceipt.workload.gpu_adapter_name)) {
        throw 'selected DXGI adapter name is absent'
    }
    $gpuEligible = ($vendor -eq 0x10DE -and $device -eq 0x1C03 -and $dedicated -eq 6GB) -or
        ($vendor -eq 0x1002 -and $device -eq 0x67DF -and $dedicated -eq 8GB)
    if (-not $gpuEligible) { throw 'selected DXGI adapter vendor/device/dedicated-memory contract failed' }
}
catch {
    $failures.Add($_.Exception.Message)
}
finally {
    if ($null -ne $exeLock) { $exeLock.Dispose() }
}

$result = if ($failures.Count -eq 0) { 'pass' } else { 'fail' }
$releaseReceiptPath = if ([IO.File]::Exists([IO.Path]::GetFullPath($ReleaseArtifactReceipt))) { [IO.Path]::GetFullPath($ReleaseArtifactReceipt) } else { $null }
$receipt = [ordered]@{
    schema = 'battle-plan-weak-pc-performance-acceptance-v3'
    result = $result
    proof_scope = 'performance_only_non_semantic_non_promotion'
    prohibited_claims = @(
        'replay_semantic_parity',
        'rendering_law_promotion',
        'reverse_engineering_evidence',
        'pixel_parity'
    )
    reference_contract = [ordered]@{
        cpu = 'Intel Core i5-8400 or AMD Ryzen 5 2600'
        physical_memory = 'nominal 16 GiB'
        gpu = 'PCI 10DE:1C03 with exactly 6 GiB or PCI 1002:67DF with exactly 8 GiB dedicated memory'
        viewport = '1920x1080'
        average_fps_min = 60.0
        p95_frame_time_ms_max = 33.3
    }
    machine = $machine
    central_release = if ($null -ne $attestation) { [ordered]@{
        receipt_path = $attestation.receipt_path
        receipt_sha256 = $attestation.receipt_sha256
        source_input_sha256 = $attestation.source_input_sha256
        replay_app_sha256 = $attestation.artifact_sha256.replay_app
    }} elseif ($null -ne $releaseReceiptPath) { [ordered]@{
        receipt_path = $releaseReceiptPath
        receipt_sha256 = (Get-FileHash -LiteralPath $releaseReceiptPath -Algorithm SHA256).Hash.ToLowerInvariant()
    }} else { $null }
    replay_app = if ($null -ne $replayAppPath) { [ordered]@{
        path = $replayAppPath
        pre_execution_sha256 = $preHash
        post_execution_sha256 = $postHash
    }} else { $null }
    workload_contract = [ordered]@{
        path = $workloadContractPath
        sha256 = $workloadContractSha256
        workload_id = if ($null -ne $workloadContract) { [string]$workloadContract.workload_id } else { $null }
    }
    renderer_exit_code = $rendererExitCode
    raw_renderer_receipt = if ($null -ne $rendererReceiptBytes) { [ordered]@{
        path = $rawPath
        bytes = $rendererReceiptBytes.Length
        sha256 = $rendererReceiptSha256
    }} else { $null }
    renderer_receipt = $rendererReceipt
    renderer_output = @($rendererOutput)
    failures = @($failures)
}

Write-ImmutableJson -Path $outputPath -Value $receipt
Write-Host "Wrote weak-PC performance acceptance receipt: $outputPath ($result)"
if ($result -ne 'pass') { throw "weak-PC performance acceptance failed; see $outputPath" }
