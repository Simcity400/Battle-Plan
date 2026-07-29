Set-StrictMode -Version Latest

$script:ProductionPromotionFailureMask = [uint32]0x13CF
$script:ProductionPromotionUnknownStatusMask = [uint32]0xFFFFC000
$script:ProductionPromotionStatuses = @(
    'existing',
    'production_complete',
    'production_complete_base'
)

function Get-ProductionPromotionProperty {
    param(
        $Object,
        [Parameter(Mandatory = $true)][string]$Name
    )
    if ($null -eq $Object) { return $null }
    $property = $Object.PSObject.Properties[$Name]
    if ($null -eq $property) { return $null }
    return $property.Value
}

function Read-ProductionPromotionJson {
    param([Parameter(Mandatory = $true)][string]$Path)
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "production promotion evidence file is missing: $Path"
    }
    try {
        return Get-Content -LiteralPath $Path -Raw | ConvertFrom-Json
    }
    catch {
        throw "production promotion evidence is not JSON: $Path"
    }
}

function Get-ProductionPromotionSha256 {
    param([Parameter(Mandatory = $true)][string]$Path)
    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Resolve-ContainedProductionPromotionPath {
    param(
        [Parameter(Mandatory = $true)][string]$RunDir,
        [Parameter(Mandatory = $true)][string]$Value,
        [Parameter(Mandatory = $true)][string]$Name
    )
    if ([string]::IsNullOrWhiteSpace($Value)) {
        throw "production promotion evidence path is empty: $Name"
    }
    $candidate = if ([IO.Path]::IsPathRooted($Value)) {
        [IO.Path]::GetFullPath($Value)
    }
    else {
        [IO.Path]::GetFullPath((Join-Path $RunDir $Value))
    }
    $root = [IO.Path]::GetFullPath($RunDir).TrimEnd('\') + '\'
    if (-not $candidate.StartsWith($root, [StringComparison]::OrdinalIgnoreCase)) {
        throw "production promotion evidence path escapes the capture run: $Name"
    }
    if (-not (Test-Path -LiteralPath $candidate -PathType Leaf)) {
        throw "production promotion evidence path is missing: $Name"
    }
    return $candidate
}

function Assert-ProductionPromotionHash {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$Expected,
        [Parameter(Mandatory = $true)][string]$Name
    )
    if ($Expected -cnotmatch '^[0-9a-f]{64}$' -or
        (Get-ProductionPromotionSha256 -Path $Path) -cne $Expected) {
        throw "production promotion evidence hash mismatch: $Name"
    }
}

function Get-ProductionPromotionGateArgument {
    param(
        [Parameter(Mandatory = $true)][object[]]$Arguments,
        [Parameter(Mandatory = $true)][string]$Switch
    )

    $matches = @()
    for ($index = 0; $index -lt $Arguments.Count; $index++) {
        if ([string]$Arguments[$index] -ceq $Switch) {
            $matches += $index
        }
    }
    if ($matches.Count -ne 1 -or
        [int]$matches[0] + 1 -ge $Arguments.Count) {
        throw "independent production gate lacks one exact $Switch argument"
    }
    return [string]$Arguments[[int]$matches[0] + 1]
}

function Get-ProductionCapturePromotionEvidence {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)][string]$RepoRoot,
        [Parameter(Mandatory = $true)][string]$CaptureReceiptPath,
        [Parameter(Mandatory = $true)][string]$CurrentArtifactAttestationPath
    )

    $repo = [IO.Path]::GetFullPath($RepoRoot)
    $receiptPath = [IO.Path]::GetFullPath($CaptureReceiptPath)
    $runDir = Split-Path -Parent $receiptPath
    $receipt = Read-ProductionPromotionJson -Path $receiptPath
    if ([string](Get-ProductionPromotionProperty $receipt 'schema') -cne
            'battle-plan.production_capture_validation_receipt.v1' -or
        [string](Get-ProductionPromotionProperty $receipt 'acceptance_mode') -cne 'production' -or
        (Get-ProductionPromotionProperty $receipt 'admissible_for_production') -isnot [bool] -or
        -not (Get-ProductionPromotionProperty $receipt 'admissible_for_production')) {
        throw 'capture receipt has no production-admissible authority'
    }
    if ([IO.Path]::GetFullPath([string](Get-ProductionPromotionProperty $receipt 'receipt_path')) -ine
        $receiptPath) {
        throw 'capture receipt does not identify its own canonical path'
    }

    $paths = [ordered]@{}
    foreach ($spec in @(
        @('capture_plan', 'capture_plan_path', 'capture_plan_sha256'),
        @('manifest', 'manifest_path', 'manifest_sha256'),
        @('ready', 'capture_ready_receipt_path', 'capture_ready_receipt_sha256'),
        @('teardown', 'capture_teardown_receipt_path', 'capture_teardown_receipt_sha256'),
        @('validation', 'validation_report_path', 'validation_report_sha256'),
        @('sink', 'sink_reconciliation_report_path', 'sink_reconciliation_report_sha256'),
        @('retained_attestation', 'prebuilt_artifact_attestation_path',
            'prebuilt_artifact_attestation_sha256')
    )) {
        $path = Resolve-ContainedProductionPromotionPath `
            -RunDir $runDir `
            -Value ([string](Get-ProductionPromotionProperty $receipt $spec[1])) `
            -Name $spec[0]
        Assert-ProductionPromotionHash `
            -Path $path `
            -Expected ([string](Get-ProductionPromotionProperty $receipt $spec[2])) `
            -Name $spec[0]
        $paths[$spec[0]] = $path
    }
    $speedPath = Resolve-ContainedProductionPromotionPath `
        -RunDir $runDir -Value 'speed5_proof.json' -Name 'speed5_proof'

    $plan = Read-ProductionPromotionJson -Path $paths.capture_plan
    $manifest = Read-ProductionPromotionJson -Path $paths.manifest
    $validation = Read-ProductionPromotionJson -Path $paths.validation
    $sink = Read-ProductionPromotionJson -Path $paths.sink
    $speed = Read-ProductionPromotionJson -Path $speedPath
    $currentAttestationPath = [IO.Path]::GetFullPath($CurrentArtifactAttestationPath)
    $currentAttestation = Read-ProductionPromotionJson -Path $currentAttestationPath
    $retainedAttestation = Read-ProductionPromotionJson -Path $paths.retained_attestation
    $attestationHelper = Join-Path $repo 'extractor\native\scripts\ProductionArtifact.Attestation.ps1'
    $captureDriver = Join-Path $repo 'extractor\native\scripts\Start-Hoi4ProductionCaptureSafe.ps1'
    $lifecycleHelper = Join-Path $repo 'extractor\native\scripts\PassiveCapture.Lifecycle.ps1'
    . $attestationHelper
    . $lifecycleHelper
    $currentArtifactAuthority = Test-ProductionArtifactAttestation `
        -RepoRoot $repo `
        -ReceiptPath $currentAttestationPath `
        -ExpectedDriverPath $captureDriver
    $retainedArtifactAuthority = Test-ProductionArtifactAttestation `
        -RepoRoot $repo `
        -ReceiptPath $paths.retained_attestation `
        -ExpectedDriverPath $captureDriver
    if ([string]$currentArtifactAuthority.source_input_sha256 -cne
            [string]$retainedArtifactAuthority.source_input_sha256 -or
        (($currentArtifactAuthority.artifact_sha256 | ConvertTo-Json -Compress) -cne
            ($retainedArtifactAuthority.artifact_sha256 | ConvertTo-Json -Compress))) {
        throw 'captured prebuilt artifact authority differs from the current Release'
    }

    $captureId = [string](Get-ProductionPromotionProperty $receipt 'capture_id')
    $sessionId = [string](Get-ProductionPromotionProperty $receipt 'session_id')
    if ([string]::IsNullOrWhiteSpace($captureId) -or
        [string]::IsNullOrWhiteSpace($sessionId) -or
        [string](Get-ProductionPromotionProperty $plan 'schema') -cne
            'battle-plan.production_capture_plan.v1' -or
        [string](Get-ProductionPromotionProperty $plan 'capture_id') -cne $captureId -or
        [string](Get-ProductionPromotionProperty $plan 'session_id') -cne $sessionId -or
        [string](Get-ProductionPromotionProperty $manifest 'capture_id') -cne $captureId -or
        [string](Get-ProductionPromotionProperty $manifest 'session_id') -cne $sessionId -or
        [string](Get-ProductionPromotionProperty $plan 'exe_sha256') -cne
            '870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad' -or
        [int](Get-ProductionPromotionProperty $plan 'speed_required_raw') -ne 4 -or
        (Get-ProductionPromotionProperty $plan 'unpaused_required') -isnot [bool] -or
        -not (Get-ProductionPromotionProperty $plan 'unpaused_required') -or
        (Get-ProductionPromotionProperty $plan 'read_only_scheduler_required') -isnot [bool] -or
        -not (Get-ProductionPromotionProperty $plan 'read_only_scheduler_required')) {
        throw 'capture plan has no exact production speed-five identity authority'
    }
    foreach ($closure in @(
        (Get-ProductionPromotionProperty $plan 'interaction_closure'),
        (Get-ProductionPromotionProperty $receipt 'interaction_closure')
    )) {
        if ([string](Get-ProductionPromotionProperty $closure 'policy') -cne
                'passive_injected_observer' -or
            [int](Get-ProductionPromotionProperty $closure 'client_commands_during_capture') -ne 0 -or
            [int](Get-ProductionPromotionProperty $closure 'semantic_engine_state_writes') -ne 0 -or
            [int](Get-ProductionPromotionProperty $closure 'game_process_lifecycle_commands') -ne 0 -or
            [int](Get-ProductionPromotionProperty $closure 'game_ui_commands') -ne 0) {
            throw 'capture interaction closure permits production client driving'
        }
    }

    $speedFirstHour = Get-ProductionPromotionProperty $speed 'first_game_hour_raw'
    $speedLastHour = Get-ProductionPromotionProperty $speed 'last_game_hour_raw'
    $speedCaptureFirstHour =
        Get-ProductionPromotionProperty $speed 'capture_first_game_hour_raw'
    $speedCaptureLastHour =
        Get-ProductionPromotionProperty $speed 'capture_last_game_hour_raw'
    $manifestFirstHour =
        Get-ProductionPromotionProperty $manifest 'first_game_hour_raw'
    $manifestLastHour =
        Get-ProductionPromotionProperty $manifest 'last_game_hour_raw'
    $speedSampleCount =
        Get-ProductionPromotionProperty $speed 'sample_count'
    foreach ($hour in @(
        $speedFirstHour,
        $speedLastHour,
        $speedCaptureFirstHour,
        $speedCaptureLastHour,
        $manifestFirstHour,
        $manifestLastHour
    )) {
        if ($hour -isnot [int] -and $hour -isnot [long]) {
            throw 'capture speed-five proof has a non-integer game-hour boundary'
        }
    }
    if ([string](Get-ProductionPromotionProperty $speed 'schema') -cne
            'battle-plan.speed5_proof.v2' -or
        [string](Get-ProductionPromotionProperty $speed 'session_id') -cne $sessionId -or
        [string](Get-ProductionPromotionProperty $speed 'acceptance_mode') -cne 'production' -or
        [int](Get-ProductionPromotionProperty $speed 'speed_raw') -ne 4 -or
        (Get-ProductionPromotionProperty $speed 'paused') -isnot [bool] -or
        (Get-ProductionPromotionProperty $speed 'paused') -or
        ($speedSampleCount -isnot [int] -and
            $speedSampleCount -isnot [long]) -or
        [int64]$speedSampleCount -lt 3 -or
        [int](Get-ProductionPromotionProperty $speed 'pid') -ne
            [int](Get-ProductionPromotionProperty $plan 'pid') -or
        [int](Get-ProductionPromotionProperty $speed 'pid') -le 0 -or
        [string](Get-ProductionPromotionProperty $speed 'exe_sha256') -cne
            '870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad' -or
        [string](Get-ProductionPromotionProperty $speed 'monitor_sha256') -cnotmatch
            '^[0-9a-f]{64}$' -or
        [string](Get-ProductionPromotionProperty $speed 'collector_control_sha256') -cnotmatch
            '^[0-9a-f]{64}$' -or
        [string](Get-ProductionPromotionProperty $speed 'domain57_game_speed_gate_v2_sha256') -cnotmatch
            '^[0-9a-f]{64}$' -or
        [int64]$speedFirstHour -gt [int64]$speedCaptureFirstHour -or
        [int64]$speedCaptureFirstHour -ne
            [int64]$manifestFirstHour -or
        [int64]$speedCaptureLastHour -ne
            [int64]$manifestLastHour -or
        [int64]$speedCaptureFirstHour -gt [int64]$speedCaptureLastHour -or
        [int64]$speedCaptureLastHour -gt [int64]$speedLastHour -or
        [string](Get-ProductionPromotionProperty $speed 'capture_plan_sha256') -cne
            (Get-ProductionPromotionSha256 -Path $paths.capture_plan)) {
        throw 'capture has no continuous unpaused speed-five proof'
    }

    $registryPath = Join-Path $repo 'extractor\native\domain_registry.json'
    $registry = Read-ProductionPromotionJson -Path $registryPath
    if ([string](Get-ProductionPromotionProperty $registry 'schema') -cne
        'battle-plan.compact_domain_registry.v1') {
        throw 'production domain registry schema is invalid'
    }
    $expectedDomains = @(
        @(Get-ProductionPromotionProperty $registry 'domains') |
            Where-Object {
                $_.schedulable -eq $true -and
                $script:ProductionPromotionStatuses -ccontains [string]$_.status
            } |
            Sort-Object { [int]$_.id }
    )
    if ($expectedDomains.Count -le 0) {
        throw 'production domain registry has no promotable domains'
    }
    $expectedCanonical = @(
        $expectedDomains | ForEach-Object {
            [ordered]@{ id = [int]$_.id; name = [string]$_.name }
        }
    )
    if ((@(Get-ProductionPromotionProperty $plan 'production_profile_domain_roster') |
            ConvertTo-Json -Depth 4 -Compress) -cne
        ($expectedCanonical | ConvertTo-Json -Depth 4 -Compress)) {
        throw 'capture plan omits or reorders current production domains'
    }
    $scheduled = @(Get-ProductionPromotionProperty $plan 'scheduled_domain_roster')
    $scheduledNames = @(Get-ProductionPromotionProperty $plan 'scheduled_domain_names')
    if ($scheduled.Count -ne $expectedDomains.Count -or
        $scheduledNames.Count -ne $expectedDomains.Count) {
        throw 'capture plan scheduled-domain roster is incomplete'
    }
    $authorityPath = Resolve-ContainedProductionPromotionPath `
        -RunDir $runDir `
        -Value ([string](Get-ProductionPromotionProperty $plan 'domain_roster_authority_path')) `
        -Name 'domain_roster_authority'
    Assert-ProductionPromotionHash `
        -Path $authorityPath `
        -Expected ([string](Get-ProductionPromotionProperty $plan 'domain_roster_authority_sha256')) `
        -Name 'domain_roster_authority'
    $authority = @(Read-ProductionPromotionJson -Path $authorityPath)
    if ($authority.Count -ne $expectedDomains.Count) {
        throw 'capture domain-roster authority has a noncanonical row count'
    }
    for ($index = 0; $index -lt $expectedDomains.Count; $index++) {
        $domain = $expectedDomains[$index]
        $scheduledRow = $scheduled[$index]
        $authorityRow = $authority[$index]
        $wireName = [string](Get-ProductionPromotionProperty $scheduledRow 'wire_name')
        if ([int](Get-ProductionPromotionProperty $scheduledRow 'id') -ne [int]$domain.id -or
            [string]$scheduledNames[$index] -cne $wireName -or
            [int](Get-ProductionPromotionProperty $authorityRow 'id') -ne [int]$domain.id -or
            [string](Get-ProductionPromotionProperty $authorityRow 'name') -cne
                [string]$domain.name -or
            [string](Get-ProductionPromotionProperty $authorityRow 'wire_name') -cne
                $wireName -or [string]::IsNullOrWhiteSpace($wireName)) {
            throw "capture domain-roster authority is invalid at index $index"
        }
    }

    $targetPid = Get-ProductionPromotionProperty $plan 'pid'
    $targetCreationFiletimeUtc =
        Get-ProductionPromotionProperty $plan 'process_creation_filetime_utc'
    $requestedDurationMs =
        Get-ProductionPromotionProperty $plan 'requested_duration_ms'
    $schedulerPollMs =
        Get-ProductionPromotionProperty $plan 'read_only_scheduler_poll_ms'
    $schedulerQueueCapacity =
        Get-ProductionPromotionProperty $plan 'read_only_scheduler_queue_cap'
    $schedulerWorkerCount =
        Get-ProductionPromotionProperty $plan 'scheduler_worker_count'
    foreach ($integerInput in @(
        $targetPid,
        $targetCreationFiletimeUtc,
        $requestedDurationMs,
        $schedulerPollMs,
        $schedulerQueueCapacity,
        $schedulerWorkerCount
    )) {
        if ($integerInput -isnot [int] -and
            $integerInput -isnot [long] -and
            $integerInput -isnot [uint32] -and
            $integerInput -isnot [uint64]) {
            throw 'capture plan has a non-integer passive lifecycle input'
        }
    }
    $collectorDllSha256 =
        [string](Get-ProductionPromotionProperty $plan 'staged_dll_sha256')
    $attestedCollectorDllSha256 = [string](
        Get-ProductionPromotionProperty `
            (Get-ProductionPromotionProperty $currentArtifactAuthority 'artifact_sha256') `
            'collector_dll'
    )
    if ([uint64]$targetPid -eq 0 -or
        [uint64]$targetPid -gt [uint32]::MaxValue -or
        [uint64]$targetCreationFiletimeUtc -eq 0 -or
        [uint64]$requestedDurationMs -eq 0 -or
        [uint64]$requestedDurationMs -gt [uint32]::MaxValue -or
        [uint64]$schedulerPollMs -eq 0 -or
        [uint64]$schedulerPollMs -gt [uint32]::MaxValue -or
        [uint64]$schedulerQueueCapacity -gt [uint32]::MaxValue -or
        [uint64]$schedulerWorkerCount -eq 0 -or
        [uint64]$schedulerWorkerCount -gt [uint32]::MaxValue -or
        $collectorDllSha256 -cnotmatch '^[0-9a-f]{64}$' -or
        $collectorDllSha256 -cne $attestedCollectorDllSha256) {
        throw 'capture plan has no exact attested passive lifecycle identity'
    }
    $collectorControlPath = Resolve-ContainedProductionPromotionPath `
        -RunDir $runDir `
        -Value 'collector_control.json' `
        -Name 'collector_control'
    if ([string](Get-ProductionPromotionProperty $speed 'collector_control_sha256') -cne
        (Get-ProductionPromotionSha256 -Path $collectorControlPath)) {
        throw 'speed-five proof does not bind the authenticated collector control'
    }
    $readyAuthority = Test-PassiveCaptureReadyReceipt `
        -RunDir $runDir `
        -TargetPid ([uint32]$targetPid) `
        -TargetCreationFiletimeUtc ([uint64]$targetCreationFiletimeUtc) `
        -SessionId $sessionId `
        -CaptureId $captureId `
        -RequestedDurationMs ([uint32]$requestedDurationMs) `
        -TargetExecutableSha256 `
            '870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad' `
        -CollectorDllSha256 $collectorDllSha256 `
        -ScheduledDomainNames $scheduledNames `
        -SchedulerPollMs ([uint32]$schedulerPollMs) `
        -SchedulerQueueCapacity ([uint32]$schedulerQueueCapacity) `
        -SchedulerWorkerCount ([uint32]$schedulerWorkerCount) `
        -CollectorControlPath $collectorControlPath
    $teardownAuthority = Test-PassiveCaptureTeardownReceipt `
        -RunDir $runDir `
        -TargetPid ([uint32]$targetPid) `
        -TargetCreationFiletimeUtc ([uint64]$targetCreationFiletimeUtc) `
        -SessionId $sessionId `
        -CaptureId $captureId `
        -ReadyAuthority $readyAuthority
    if ([IO.Path]::GetFullPath([string]$readyAuthority.ReceiptPath) -ine
            [IO.Path]::GetFullPath([string]$paths.ready) -or
        [IO.Path]::GetFullPath([string]$teardownAuthority.ReceiptPath) -ine
            [IO.Path]::GetFullPath([string]$paths.teardown)) {
        throw 'capture receipt paths differ from independently authenticated lifecycle receipts'
    }

    $compact = Get-ProductionPromotionProperty $manifest 'compact_stream'
    $frameCount = Get-ProductionPromotionProperty $compact 'frame_end_record_count'
    $gapRows = @(Get-ProductionPromotionProperty $compact 'domain_gap_outcomes')
    if ($frameCount -isnot [int] -and $frameCount -isnot [long]) {
        throw 'capture frame count is not an integer'
    }
    $frameCount = [int64]$frameCount
    if ($frameCount -le 0 -or
        [int64](Get-ProductionPromotionProperty $manifest 'skipped_game_hours') -ne 0 -or
        [int64](Get-ProductionPromotionProperty $manifest 'unique_game_hour_count') -ne
            $frameCount -or
        [int64](Get-ProductionPromotionProperty $compact 'domain_gap_record_count') -ne 0 -or
        $gapRows.Count -ne 0 -or
        ([uint32](Get-ProductionPromotionProperty $compact 'failure_status_flags_or') -band
            $script:ProductionPromotionFailureMask) -ne 0 -or
        ([uint32](Get-ProductionPromotionProperty $compact 'failure_status_flags_or') -band
            $script:ProductionPromotionUnknownStatusMask) -ne 0 -or
        (Get-ProductionPromotionProperty $compact 'failure_status_seen') -eq $true) {
        throw 'capture is not a zero-loss all-hour production stream'
    }
    if ((@(Get-ProductionPromotionProperty $manifest 'scheduled_domains') -join "`0") -cne
        ($scheduledNames -join "`0")) {
        throw 'capture manifest scheduled-domain roster differs from the plan'
    }
    $perDomain = Get-ProductionPromotionProperty $compact 'per_domain'
    $domainCompletenessReceipt = [System.Collections.Generic.List[object]]::new()
    for ($index = 0; $index -lt $expectedDomains.Count; $index++) {
        $wireName = [string]$scheduledNames[$index]
        $row = Get-ProductionPromotionProperty $perDomain $wireName
        $rawStatusFlags = [uint32](Get-ProductionPromotionProperty $row 'status_flags_or')
        $effectiveStatusFlags =
            [uint32](Get-ProductionPromotionProperty $row 'effective_failure_status_flags_or')
        if ($null -eq $row -or
            [int](Get-ProductionPromotionProperty $row 'domain_id') -ne
                [int]$expectedDomains[$index].id -or
            [int64](Get-ProductionPromotionProperty $row 'record_count') -ne $frameCount -or
            [int64](Get-ProductionPromotionProperty $row 'data_record_count') -ne $frameCount -or
            ($rawStatusFlags -band $script:ProductionPromotionFailureMask) -ne 0 -or
            ($effectiveStatusFlags -band $script:ProductionPromotionFailureMask) -ne 0 -or
            ($rawStatusFlags -band $script:ProductionPromotionUnknownStatusMask) -ne 0 -or
            ($effectiveStatusFlags -band $script:ProductionPromotionUnknownStatusMask) -ne 0) {
            throw "production domain $($expectedDomains[$index].id) lacks real-source completeness"
        }
        $domainCompletenessReceipt.Add([ordered]@{
            id = [int]$expectedDomains[$index].id
            name = [string]$expectedDomains[$index].name
            wire_name = $wireName
            record_count = [int64](Get-ProductionPromotionProperty $row 'record_count')
            data_record_count =
                [int64](Get-ProductionPromotionProperty $row 'data_record_count')
            raw_failure_status_flags_or = $rawStatusFlags
            effective_failure_status_flags_or = $effectiveStatusFlags
            real_source_complete =
                ([int64](Get-ProductionPromotionProperty $row 'data_record_count') -eq
                    $frameCount -and
                ($rawStatusFlags -band $script:ProductionPromotionFailureMask) -eq 0 -and
                ($effectiveStatusFlags -band $script:ProductionPromotionFailureMask) -eq 0 -and
                ($rawStatusFlags -band $script:ProductionPromotionUnknownStatusMask) -eq 0 -and
                ($effectiveStatusFlags -band $script:ProductionPromotionUnknownStatusMask) -eq 0)
        })
    }
    if (@($perDomain.PSObject.Properties).Count -ne $expectedDomains.Count) {
        throw 'capture manifest contains a noncanonical production domain census'
    }
    $allDomainsRealSourceComplete =
        ($domainCompletenessReceipt.Count -eq $expectedDomains.Count -and
        @($domainCompletenessReceipt | Where-Object {
            (Get-ProductionPromotionProperty $_ 'real_source_complete') -ne $true
        }).Count -eq 0)
    if (-not $allDomainsRealSourceComplete) {
        throw 'capture has no exact all-domain real-source completeness receipt'
    }

    if ([string](Get-ProductionPromotionProperty $validation 'schema') -cne
            'battle-plan.production_capture_validation.v2' -or
        (Get-ProductionPromotionProperty $validation 'ok') -isnot [bool] -or
        -not (Get-ProductionPromotionProperty $validation 'ok') -or
        @(Get-ProductionPromotionProperty $validation 'problems').Count -ne 0 -or
        [string](Get-ProductionPromotionProperty $validation 'manifest_sha256') -cne
            (Get-ProductionPromotionSha256 -Path $paths.manifest) -or
        [int](Get-ProductionPromotionProperty $validation 'expected_domain_count') -ne
            $expectedDomains.Count -or
        ((@(Get-ProductionPromotionProperty $validation 'expected_domain_roster') |
                ConvertTo-Json -Depth 4 -Compress) -cne
            ($expectedCanonical | ConvertTo-Json -Depth 4 -Compress))) {
        throw 'production validation report has no exact all-domain pass'
    }
    $compactGates = @(
        @(Get-ProductionPromotionProperty $validation 'independent_gates') |
            Where-Object { [string]$_.name -ceq 'compact_gate' }
    )
    if ($compactGates.Count -ne 1 -or
        (Get-ProductionPromotionProperty $compactGates[0] 'passed') -isnot [bool] -or
        -not (Get-ProductionPromotionProperty $compactGates[0] 'passed') -or
        (@(Get-ProductionPromotionProperty $compactGates[0] 'arguments') -cnotcontains
            '--require-zero-loss')) {
        throw 'production validation lacks an independent zero-loss compact gate'
    }
    $speedGates = @(
        @(Get-ProductionPromotionProperty $validation 'independent_gates') |
            Where-Object { [string]$_.name -ceq 'speed5_stream_gate' }
    )
    $speedGateExitCode = if ($speedGates.Count -eq 1) {
        Get-ProductionPromotionProperty $speedGates[0] 'exit_code'
    }
    else {
        $null
    }
    if ($speedGates.Count -ne 1 -or
        (Get-ProductionPromotionProperty $speedGates[0] 'passed') -isnot [bool] -or
        -not (Get-ProductionPromotionProperty $speedGates[0] 'passed') -or
        (Get-ProductionPromotionProperty $speedGates[0] 'started') -isnot [bool] -or
        -not (Get-ProductionPromotionProperty $speedGates[0] 'started') -or
        ($speedGateExitCode -isnot [int] -and
            $speedGateExitCode -isnot [long]) -or
        [int64]$speedGateExitCode -ne 0 -or
        (Get-ProductionPromotionProperty $speedGates[0] 'timed_out') -isnot [bool] -or
        (Get-ProductionPromotionProperty $speedGates[0] 'timed_out') -or
        $null -ne (Get-ProductionPromotionProperty $speedGates[0] 'problem')) {
        throw 'production validation lacks one successful executed speed-five stream gate'
    }
    $speedGateArguments = @(
        Get-ProductionPromotionProperty $speedGates[0] 'arguments'
    )
    $speedVerifierPath = Join-Path `
        $repo 'extractor\native\scripts\verify_speed5_stream.py'
    if ($speedGateArguments.Count -lt 2 -or
        [IO.Path]::GetFullPath([string]$speedGateArguments[0]) -ine
            [IO.Path]::GetFullPath($speedVerifierPath)) {
        throw 'speed-five stream gate did not execute the current canonical verifier'
    }
    $speedGateStreamPath = Resolve-ContainedProductionPromotionPath `
        -RunDir $runDir `
        -Value (Get-ProductionPromotionGateArgument `
            -Arguments $speedGateArguments -Switch '--stream') `
        -Name 'speed5_stream'
    $speedGateMonitorPath = Resolve-ContainedProductionPromotionPath `
        -RunDir $runDir `
        -Value (Get-ProductionPromotionGateArgument `
            -Arguments $speedGateArguments -Switch '--monitor') `
        -Name 'speed5_monitor'
    $speedGateReportPath = Resolve-ContainedProductionPromotionPath `
        -RunDir $runDir `
        -Value (Get-ProductionPromotionGateArgument `
            -Arguments $speedGateArguments -Switch '--out') `
        -Name 'speed5_stream_gate_report'
    $speedGateRootPath = [IO.Path]::GetFullPath(
        (Get-ProductionPromotionGateArgument `
            -Arguments $speedGateArguments -Switch '--root-gate')
    )
    $speedGateTranscriptPath = [IO.Path]::GetFullPath(
        (Get-ProductionPromotionGateArgument `
            -Arguments $speedGateArguments -Switch '--evidence-transcript')
    )
    $speedGateEvidenceVerifierPath = [IO.Path]::GetFullPath(
        (Get-ProductionPromotionGateArgument `
            -Arguments $speedGateArguments -Switch '--evidence-verifier')
    )
    $speedGateNonceLedgerRoot = [IO.Path]::GetFullPath(
        (Get-ProductionPromotionGateArgument `
            -Arguments $speedGateArguments -Switch '--nonce-ledger-root')
    )
    foreach ($path in @($speedGateRootPath, $speedGateTranscriptPath)) {
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
            throw "speed-five stream gate retained authority is missing: $path"
        }
    }
    if (-not (Test-Path -LiteralPath $speedGateEvidenceVerifierPath -PathType Leaf) -or
        -not (Test-Path -LiteralPath $speedGateNonceLedgerRoot -PathType Container) -or
        $speedGateNonceLedgerRoot -ine [IO.Path]::GetFullPath(
            [string](Get-ProductionPromotionProperty $plan 'speed_evidence_nonce_ledger_root')
        )) {
        throw 'speed-five stream gate verifier or nonce-ledger authority is missing'
    }
    $expectedEvidenceVerifierSha = Get-ProductionPromotionGateArgument `
        -Arguments $speedGateArguments -Switch '--expected-verifier-sha256'
    $expectedProbeSha = Get-ProductionPromotionGateArgument `
        -Arguments $speedGateArguments -Switch '--expected-probe-sha256'
    $expectedStaticSha = Get-ProductionPromotionGateArgument `
        -Arguments $speedGateArguments -Switch '--expected-static-sha256'
    if ($expectedEvidenceVerifierSha -cnotmatch '^[0-9a-f]{64}$' -or
        $expectedProbeSha -cnotmatch '^[0-9a-f]{64}$' -or
        $expectedStaticSha -cnotmatch '^[0-9a-f]{64}$' -or
        (Get-ProductionPromotionSha256 -Path $speedGateEvidenceVerifierPath) -cne
            $expectedEvidenceVerifierSha) {
        throw 'speed-five stream gate verifier/probe/static pins are invalid'
    }
    foreach ($binding in @(
        [pscustomobject]@{
            Switch = '--manifest'
            Expected = [IO.Path]::GetFullPath([string]$paths.manifest)
        },
        [pscustomobject]@{
            Switch = '--speed-proof'
            Expected = [IO.Path]::GetFullPath($speedPath)
        },
        [pscustomobject]@{
            Switch = '--capture-plan'
            Expected = [IO.Path]::GetFullPath([string]$paths.capture_plan)
        },
        [pscustomobject]@{
            Switch = '--collector-control'
            Expected = [IO.Path]::GetFullPath($collectorControlPath)
        }
    )) {
        $actual = [IO.Path]::GetFullPath(
            (Get-ProductionPromotionGateArgument `
                -Arguments $speedGateArguments `
                -Switch ([string]$binding.Switch))
        )
        if ($actual -ine [string]$binding.Expected) {
            throw "speed-five stream gate has a mismatched $($binding.Switch) binding"
        }
    }
    if ($speedGateMonitorPath -ine
            [IO.Path]::GetFullPath((Join-Path $runDir 'speed5_monitor.jsonl')) -or
        (Get-ProductionPromotionSha256 -Path $speedGateMonitorPath) -cne
            [string](Get-ProductionPromotionProperty $speed 'monitor_sha256') -or
        $speedGateRootPath -ine [IO.Path]::GetFullPath(
            [string](Get-ProductionPromotionProperty $plan 'domain57_game_speed_gate_v2')
        ) -or
        (Get-ProductionPromotionSha256 -Path $speedGateRootPath) -cne
            [string](Get-ProductionPromotionProperty $speed 'domain57_game_speed_gate_v2_sha256') -or
        (Get-ProductionPromotionSha256 -Path $speedGateRootPath) -cne
            [string](Get-ProductionPromotionProperty $plan 'domain57_game_speed_gate_v2_sha256') -or
        $speedGateTranscriptPath -ine [IO.Path]::GetFullPath(
            [string](Get-ProductionPromotionProperty $plan 'speed_evidence_transcript_path')
        ) -or
        (Get-ProductionPromotionSha256 -Path $speedGateTranscriptPath) -cne
            [string](Get-ProductionPromotionProperty $plan 'speed_evidence_transcript_file_sha256')) {
        throw 'speed-five stream gate evidence paths or hashes differ from the capture'
    }
    $speedGateReport = Read-ProductionPromotionJson -Path $speedGateReportPath
    foreach ($integerField in @(
        (Get-ProductionPromotionProperty $speedGateReport 'completed_frames'),
        (Get-ProductionPromotionProperty $speedGateReport 'first_game_hour_raw'),
        (Get-ProductionPromotionProperty $speedGateReport 'last_game_hour_raw'),
        (Get-ProductionPromotionProperty $speedGateReport 'monitor_samples')
    )) {
        if ($integerField -isnot [int] -and $integerField -isnot [long]) {
            throw 'speed-five stream gate report has a non-integer capture census'
        }
    }
    if ([string](Get-ProductionPromotionProperty $speedGateReport 'schema') -cne
            'battle-plan.speed5_stream_gate.v2' -or
        (Get-ProductionPromotionProperty $speedGateReport 'passed') -isnot [bool] -or
        -not (Get-ProductionPromotionProperty $speedGateReport 'passed') -or
        [int64](Get-ProductionPromotionProperty $speedGateReport 'completed_frames') -ne
            $frameCount -or
        [int64](Get-ProductionPromotionProperty $speedGateReport 'first_game_hour_raw') -ne
            [int64]$manifestFirstHour -or
        [int64](Get-ProductionPromotionProperty $speedGateReport 'last_game_hour_raw') -ne
            [int64]$manifestLastHour -or
        [int64](Get-ProductionPromotionProperty $speedGateReport 'monitor_samples') -ne
            [int64](Get-ProductionPromotionProperty $speed 'sample_count') -or
        [string](Get-ProductionPromotionProperty $speedGateReport 'domain57_game_speed_gate_v2_sha256') -cne
            (Get-ProductionPromotionSha256 -Path $speedGateRootPath) -or
        [string](Get-ProductionPromotionProperty $speedGateReport 'speed_evidence_transcript_file_sha256') -cne
            (Get-ProductionPromotionSha256 -Path $speedGateTranscriptPath) -or
        [string](Get-ProductionPromotionProperty $speedGateReport 'speed_evidence_transcript_sha256') -cne
            [string](Get-ProductionPromotionProperty $plan 'speed_evidence_transcript_sha256') -or
        [string](Get-ProductionPromotionProperty $speedGateReport 'nonce_seal_sha256') -cne
            [string](Get-ProductionPromotionProperty $plan 'speed_evidence_nonce_seal_sha256') -or
        [string](Get-ProductionPromotionProperty $speedGateReport 'collector_control_sha256') -cne
            (Get-ProductionPromotionSha256 -Path $collectorControlPath)) {
        throw 'speed-five stream gate report is absent, failed, or capture-unbound'
    }

    $sinkEvidence = Get-ProductionPromotionProperty $sink 'evidence'
    $sinkManifestSha =
        [string](Get-ProductionPromotionProperty $sinkEvidence 'manifest_sha256')
    $sinkPlanSha =
        [string](Get-ProductionPromotionProperty $sinkEvidence 'production_capture_plan_sha256')
    $sinkSpeedSha =
        [string](Get-ProductionPromotionProperty $sinkEvidence 'speed5_proof_sha256')
    if ([string](Get-ProductionPromotionProperty $sink 'schema') -cne
            'extractor.sink-reconciliation-verification.v2' -or
        [string](Get-ProductionPromotionProperty $sink 'expected_outcome') -cne 'success' -or
        [string](Get-ProductionPromotionProperty $sink 'verdict') -cne 'passed' -or
        @(Get-ProductionPromotionProperty $sink 'problems').Count -ne 0 -or
        [IO.Path]::GetFullPath([string](Get-ProductionPromotionProperty $sink 'run_dir')) -ine
            [IO.Path]::GetFullPath($runDir) -or
        $sinkManifestSha -cne
            (Get-ProductionPromotionSha256 -Path $paths.manifest) -or
        $sinkPlanSha -cne
            (Get-ProductionPromotionSha256 -Path $paths.capture_plan) -or
        $sinkSpeedSha -cne
            (Get-ProductionPromotionSha256 -Path $speedPath)) {
        throw 'independent sink reconciliation is absent or unbound'
    }

    foreach ($attestation in @($currentAttestation, $retainedAttestation)) {
        if ([string](Get-ProductionPromotionProperty $attestation 'schema') -cne
                'battle-plan.central_rust_prebuilt_artifacts.v1' -or
            [string](Get-ProductionPromotionProperty $attestation 'status') -cne
                'PASS_CENTRAL_RELEASE_PREBUILT_ARTIFACTS' -or
            (Get-ProductionPromotionProperty $attestation 'successful_exit') -isnot [bool] -or
            -not (Get-ProductionPromotionProperty $attestation 'successful_exit')) {
            throw 'capture artifact attestation lacks centralized Release authority'
        }
    }
    $currentSource = Get-ProductionPromotionProperty $currentAttestation 'source_input'
    $retainedSource = Get-ProductionPromotionProperty $retainedAttestation 'source_input'
    $currentSourceSha = [string](Get-ProductionPromotionProperty $currentSource 'sha256')
    $retainedSourceSha = [string](Get-ProductionPromotionProperty $retainedSource 'sha256')
    if ($currentSourceSha -cnotmatch '^[0-9a-f]{64}$' -or
        $retainedSourceSha -cne $currentSourceSha -or
        [string](Get-ProductionPromotionProperty $receipt 'prebuilt_source_input_sha256') -cne
            $currentSourceSha -or
        [string](Get-ProductionPromotionProperty $plan 'prebuilt_source_input_sha256') -cne
            $currentSourceSha) {
        throw 'capture was not produced by the exact current production source inputs'
    }
    $currentRows = @(Get-ProductionPromotionProperty $currentAttestation 'artifacts')
    $retainedRows = @(Get-ProductionPromotionProperty $retainedAttestation 'artifacts')
    if ($currentRows.Count -eq 0 -or $retainedRows.Count -ne $currentRows.Count) {
        throw 'capture artifact attestation roster is incomplete'
    }
    $artifactHashes = [ordered]@{}
    foreach ($currentRow in $currentRows) {
        $role = [string](Get-ProductionPromotionProperty $currentRow 'role')
        $matches = @($retainedRows | Where-Object { [string]$_.role -ceq $role })
        $hash = [string](Get-ProductionPromotionProperty $currentRow 'sha256')
        if ([string]::IsNullOrWhiteSpace($role) -or
            $hash -cnotmatch '^[0-9a-f]{64}$' -or
            $matches.Count -ne 1 -or
            [string](Get-ProductionPromotionProperty $matches[0] 'sha256') -cne $hash) {
            throw "capture artifact bytes differ from the current Release: $role"
        }
        $artifactHashes[$role] = $hash
    }

    return [ordered]@{
        schema = 'battle-plan.production_capture_promotion_evidence.v1'
        capture_receipt_path = $receiptPath
        capture_receipt_sha256 = Get-ProductionPromotionSha256 -Path $receiptPath
        capture_id = $captureId
        session_id = $sessionId
        current_artifact_attestation_path = $currentAttestationPath
        current_artifact_attestation_sha256 =
            Get-ProductionPromotionSha256 -Path $currentAttestationPath
        captured_artifact_attestation_sha256 =
            Get-ProductionPromotionSha256 -Path $paths.retained_attestation
        source_input_sha256 = $currentSourceSha
        artifact_sha256 = $artifactHashes
        production_domain_registry_sha256 =
            Get-ProductionPromotionSha256 -Path $registryPath
        production_domain_count = $expectedDomains.Count
        production_domain_roster = $expectedCanonical
        frame_count = $frameCount
        per_domain_real_source_receipt = @($domainCompletenessReceipt)
        all_domains_real_source_complete = $allDomainsRealSourceComplete
        passive_lifecycle_evidence = [ordered]@{
            ready_receipt_sha256 = [string]$readyAuthority.ReceiptSha256
            ready_payload_sha256 = [string]$readyAuthority.ReadyPayloadSha256
            ready_ack_sha256 = [string]$readyAuthority.ReadyAckSha256
            host_binding_sha256 = [string]$readyAuthority.HostBindingSha256
            active_hook_count = [uint32]$readyAuthority.ActiveHookCount
            teardown_receipt_sha256 =
                [string]$teardownAuthority.ReceiptSha256
            data_pipe_eof_observed =
                [bool]$teardownAuthority.DataPipeEofObserved
            hook_lease_released_after_eof =
                [bool]$teardownAuthority.HookLeaseReleasedAfterEof
            collector_module_absent_verified =
                [bool]$teardownAuthority.CollectorModuleAbsentVerified
            target_process_epoch_revalidated =
                [bool]$teardownAuthority.TargetProcessEpochRevalidated
            target_code_temporarily_instrumented =
                [bool]$teardownAuthority.TargetCodeTemporarilyInstrumented
            pinned_hook_target_code_restored =
                [bool]$teardownAuthority.PinnedHookTargetCodeRestored
            pinned_hook_sites_restored =
                [bool]$teardownAuthority.PinnedHookSitesRestored
            pinned_hook_site_count =
                [uint32]$teardownAuthority.PinnedHookSiteCount
        }
        zero_loss_evidence = [ordered]@{
            manifest_sha256 = Get-ProductionPromotionSha256 -Path $paths.manifest
            validation_report_sha256 =
                Get-ProductionPromotionSha256 -Path $paths.validation
            sink_reconciliation_report_sha256 =
                Get-ProductionPromotionSha256 -Path $paths.sink
            frame_count = $frameCount
            unique_game_hour_count =
                [int64](Get-ProductionPromotionProperty $manifest 'unique_game_hour_count')
            skipped_game_hours =
                [int64](Get-ProductionPromotionProperty $manifest 'skipped_game_hours')
            domain_gap_record_count =
                [int64](Get-ProductionPromotionProperty $compact 'domain_gap_record_count')
            independent_compact_gate_require_zero_loss = $true
        }
        zero_loss = (
            [int64](Get-ProductionPromotionProperty $manifest 'skipped_game_hours') -eq 0 -and
            [int64](Get-ProductionPromotionProperty $compact 'domain_gap_record_count') -eq 0 -and
            $gapRows.Count -eq 0
        )
        speed5_evidence = [ordered]@{
            speed5_proof_sha256 = Get-ProductionPromotionSha256 -Path $speedPath
            speed5_stream_gate_report_sha256 =
                Get-ProductionPromotionSha256 -Path $speedGateReportPath
            sample_count = [int64](Get-ProductionPromotionProperty $speed 'sample_count')
            pid = [int](Get-ProductionPromotionProperty $speed 'pid')
            exe_sha256 = [string](Get-ProductionPromotionProperty $speed 'exe_sha256')
            monitor_sha256 =
                [string](Get-ProductionPromotionProperty $speed 'monitor_sha256')
            collector_control_sha256 =
                [string](Get-ProductionPromotionProperty $speed 'collector_control_sha256')
            first_game_hour_raw =
                [int](Get-ProductionPromotionProperty $speed 'first_game_hour_raw')
            last_game_hour_raw =
                [int](Get-ProductionPromotionProperty $speed 'last_game_hour_raw')
            capture_first_game_hour_raw =
                [int](Get-ProductionPromotionProperty $speed 'capture_first_game_hour_raw')
            capture_last_game_hour_raw =
                [int](Get-ProductionPromotionProperty $speed 'capture_last_game_hour_raw')
        }
        speed_raw = [int](Get-ProductionPromotionProperty $speed 'speed_raw')
        unpaused = (-not [bool](Get-ProductionPromotionProperty $speed 'paused'))
    }
}
