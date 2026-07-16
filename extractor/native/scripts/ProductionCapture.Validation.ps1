Set-StrictMode -Version Latest

$script:CanonicalCompactFailureMask = [uint32]0x13CF
$script:RequiredStaticArtifactNames = @(
    "templates_equipment_catalog",
    "localisation_join",
    "decisions_static_definitions",
    "idea_tech_loc_xref_static",
    "ship_definitions_static",
    "gfx_icon_asset_manifest_static",
    "technology_sharing_static"
)
$script:ProductionDomainStatuses = @(
    "existing",
    "production_complete",
    "production_complete_base",
    "validated_slice",
    "validated_partial_slice",
    "validated_slice_open_gaps"
)

function Get-ObjectProperty {
    param($Object, [Parameter(Mandatory = $true)][string]$Name)
    if ($null -eq $Object) { return $null }
    $property = $Object.PSObject.Properties[$Name]
    if ($null -eq $property) { return $null }
    return $property.Value
}

function Get-RegistryCaptureDomains {
    param([Parameter(Mandatory = $true)][string]$RegistryPath)
    if (-not (Test-Path -LiteralPath $RegistryPath -PathType Leaf)) {
        throw "domain registry not found: $RegistryPath"
    }
    $registry = Get-Content -LiteralPath $RegistryPath -Raw | ConvertFrom-Json
    if ([string](Get-ObjectProperty $registry "schema") -ne "battle-plan.compact_domain_registry.v1") {
        throw "unsupported domain registry schema in $RegistryPath"
    }
    $domains = @(
        $registry.domains |
            Where-Object {
                $_.schedulable -eq $true -and
                $script:ProductionDomainStatuses -contains ([string]$_.status)
            } |
            Sort-Object { [int]$_.id }
    )
    if ($domains.Count -eq 0) { throw "domain registry contains no production-profile domains" }
    $duplicateIds = @($domains | Group-Object id | Where-Object Count -ne 1)
    if ($duplicateIds.Count -ne 0) { throw "domain registry has duplicate schedulable ids" }
    return $domains
}

function Add-CaptureProblem {
    param([System.Collections.Generic.List[string]]$Problems, [string]$Problem)
    if (-not $Problems.Contains($Problem)) { $Problems.Add($Problem) }
}

function Test-HexSha256 {
    param($Value)
    return $null -ne $Value -and ([string]$Value) -match '^[0-9a-fA-F]{64}$'
}

function ConvertTo-NativeArgumentLine {
    param([Parameter(Mandatory = $true)][object[]]$Arguments)
    return (($Arguments | ForEach-Object {
        $value = [string]$_
        if ($value -match '[\s"]') { '"' + ($value -replace '(\\*)"', '$1$1\"' -replace '(\\+)$', '$1$1') + '"' }
        else { $value }
    }) -join ' ')
}

function Invoke-IndependentCaptureGate {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][string]$FilePath,
        [Parameter(Mandatory = $true)][object[]]$Arguments,
        [Parameter(Mandatory = $true)][string]$LogDir,
        [int]$TimeoutSeconds = 900,
        [int]$CancellationGraceSeconds = 5
    )
    New-Item -ItemType Directory -Path $LogDir -Force | Out-Null
    $stdout = Join-Path $LogDir "$Name.stdout.log"
    $stderr = Join-Path $LogDir "$Name.stderr.log"
    if (-not (Test-Path -LiteralPath $FilePath -PathType Leaf)) {
        return [pscustomobject]@{ name=$Name; passed=$false; started=$false; exit_code=$null; timed_out=$false; problem="tool_missing"; stdout=$stdout; stderr=$stderr }
    }
    $process = $null
    $timedOut = $false
    try {
        $start=[System.Diagnostics.ProcessStartInfo]::new()
        $start.FileName=$FilePath; $start.Arguments=ConvertTo-NativeArgumentLine $Arguments
        $start.UseShellExecute=$false; $start.CreateNoWindow=$true
        $start.RedirectStandardOutput=$true; $start.RedirectStandardError=$true
        $process=[System.Diagnostics.Process]::new(); $process.StartInfo=$start
        if (-not $process.Start()) { throw "process start returned false" }
        $stdoutTask=$process.StandardOutput.ReadToEndAsync(); $stderrTask=$process.StandardError.ReadToEndAsync()
        if (-not $process.WaitForExit([Math]::Max(1, $TimeoutSeconds) * 1000)) {
            $timedOut = $true
            try { [void]$process.CloseMainWindow() } catch {}
            if (-not $process.WaitForExit([Math]::Max(1, $CancellationGraceSeconds) * 1000)) {
                try { $process.Kill() } catch {}
                [void]$process.WaitForExit(5000)
            }
        }
        if ($process.HasExited) {
            [System.IO.File]::WriteAllText($stdout,[string]$stdoutTask.Result,[System.Text.UTF8Encoding]::new($false))
            [System.IO.File]::WriteAllText($stderr,[string]$stderrTask.Result,[System.Text.UTF8Encoding]::new($false))
        }
        $process.Refresh()
        $exitCode = if ($process.HasExited) { $process.ExitCode } else { $null }
        return [pscustomobject]@{ name=$Name; passed=(-not $timedOut -and $exitCode -eq 0); started=$true; exit_code=$exitCode; timed_out=$timedOut; problem=$(if ($timedOut) { "timeout" } elseif ($exitCode -ne 0) { "nonzero_exit" } else { $null }); stdout=$stdout; stderr=$stderr }
    }
    catch {
        return [pscustomobject]@{ name=$Name; passed=$false; started=$false; exit_code=$null; timed_out=$timedOut; problem=$_.Exception.Message; stdout=$stdout; stderr=$stderr }
    }
    finally { if ($null -ne $process) { $process.Dispose() } }
}

function Test-ProductionCaptureArtifacts {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)][string]$RunDir,
        [Parameter(Mandatory = $true)][string]$RegistryPath,
        [Parameter(Mandatory = $true)][string]$SpeedProofPath,
        [hashtable]$StaticArtifactPaths = @{},
        [string]$ExpectedStagedDll = "",
        [string]$CompactGateExe = "",
        [string]$FoldCheckExe = "",
        [string]$ContainerVerifyExe = "",
        [string]$PythonExe = "",
        [string]$StaticBundleScript = "",
        [string]$StaticBundleDir = "",
        [string]$GameExePath = "",
        [string]$ModStatePath = "",
        [string]$DlcStatePath = "",
        [string]$LoadOrderPath = "",
        [string]$SourceInventoryPath = "",
        [string]$LiveResolutionReportPath = "",
        [string]$CapturePlanPath = "",
        [string]$SpeedRootGatePath = "",
        [string]$CollectorControlPath = "",
        [string]$SpeedMonitorPath = "",
        [string]$SpeedStreamVerifier = "",
        [string]$SpeedEvidenceTranscriptPath = "",
        [string]$SpeedEvidenceNonceLedgerRoot = "",
        [string]$SpeedEvidenceVerifier = "",
        [string]$ExpectedSpeedEvidenceVerifierSha256 = "",
        [string]$ExpectedSpeedEvidenceProbeSha256 = "",
        [string]$ExpectedSpeedStaticDisasmSha256 = "",
        [int]$ToolTimeoutSeconds = 900,
        [int]$CancellationGraceSeconds = 5
    )

    $problems = [System.Collections.Generic.List[string]]::new()
    $manifestPath = Join-Path $RunDir "native_session_manifest.json"
    if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) {
        Add-CaptureProblem $problems "manifest_missing"
        return [pscustomobject]@{ ok = $false; problems = @($problems); manifest_path = $manifestPath }
    }
    try { $manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json }
    catch {
        Add-CaptureProblem $problems "manifest_not_json"
        return [pscustomobject]@{ ok = $false; problems = @($problems); manifest_path = $manifestPath }
    }

    $expectedDomains = @(Get-RegistryCaptureDomains -RegistryPath $RegistryPath)
    $expectedIds = @($expectedDomains | ForEach-Object { [int]$_.id })
    $expectedDataIds = @($expectedDomains | Where-Object { $_.requires_data_record -eq $true } | ForEach-Object { [int]$_.id })

    if ([string](Get-ObjectProperty $manifest "stream_format") -ne "compact") { Add-CaptureProblem $problems "stream_format_not_compact" }
    if ((Get-ObjectProperty (Get-ObjectProperty $manifest "production_profile") "enabled") -ne $true) { Add-CaptureProblem $problems "production_profile_not_enabled" }
    if (@(Get-ObjectProperty (Get-ObjectProperty $manifest "production_profile") "profile_domains_not_scheduled").Count -ne 0) { Add-CaptureProblem $problems "profile_domains_not_scheduled" }
    if (@(Get-ObjectProperty (Get-ObjectProperty $manifest "production_profile") "missing_required_static_artifacts").Count -ne 0) { Add-CaptureProblem $problems "required_static_artifacts_missing_from_profile" }

    $compact = Get-ObjectProperty $manifest "compact_stream"
    if ($null -eq $compact) { Add-CaptureProblem $problems "compact_stream_summary_missing" }
    $frameCount = [int64](Get-ObjectProperty $compact "frame_end_record_count")
    if ($frameCount -le 0) { Add-CaptureProblem $problems "no_frame_end_records" }
    $frameTiming = Get-ObjectProperty $compact "frame_time_us"
    if ($null -eq $frameTiming) { Add-CaptureProblem $problems "frame_time_us_missing" }
    else {
        if ([int64](Get-ObjectProperty $frameTiming "count") -ne $frameCount) { Add-CaptureProblem $problems "frame_time_sample_count_mismatch" }
        if ($null -eq (Get-ObjectProperty $frameTiming "max") -or [double](Get-ObjectProperty $frameTiming "max") -ge 100000.0) { Add-CaptureProblem $problems "frame_time_over_100ms" }
    }
    if ((Get-ObjectProperty $compact "decodable") -ne $true) { Add-CaptureProblem $problems "compact_stream_not_decodable" }
    if (@(Get-ObjectProperty $manifest "parse_errors").Count -ne 0 -or @(Get-ObjectProperty $compact "parse_errors").Count -ne 0) { Add-CaptureProblem $problems "parse_errors_present" }
    if (@(Get-ObjectProperty $manifest "ordering_errors").Count -ne 0 -or @(Get-ObjectProperty $compact "ordering_errors").Count -ne 0) { Add-CaptureProblem $problems "ordering_errors_present" }
    if ([int64](Get-ObjectProperty $manifest "skipped_game_hours") -ne 0) { Add-CaptureProblem $problems "skipped_game_hours" }
    if ([int64](Get-ObjectProperty $manifest "unique_game_hour_count") -ne $frameCount) { Add-CaptureProblem $problems "frame_hour_count_mismatch" }
    $hourly = Get-ObjectProperty $manifest "hourly_requirement"
    if ($null -eq $hourly -or (Get-ObjectProperty $hourly "met") -ne $true) { Add-CaptureProblem $problems "recorder_hourly_requirement_not_met" }
    $requestedGameHours = [int64](Get-ObjectProperty $hourly "requested_game_hours")
    if ($requestedGameHours -le 0) { Add-CaptureProblem $problems "requested_game_hours_not_positive" }

    $failureFlags = [uint32](Get-ObjectProperty $compact "failure_status_flags_or")
    if (($failureFlags -band $script:CanonicalCompactFailureMask) -ne 0) { Add-CaptureProblem $problems ("canonical_failure_status_seen_0x{0:X}" -f ($failureFlags -band $script:CanonicalCompactFailureMask)) }
    if ((Get-ObjectProperty $compact "failure_status_seen") -eq $true) { Add-CaptureProblem $problems "recorder_failure_status_seen" }
    $coverage = Get-ObjectProperty $compact "domain_coverage"
    if ($null -eq $coverage -or (Get-ObjectProperty $coverage "ok") -ne $true) { Add-CaptureProblem $problems "domain_coverage_not_ok" }
    if (@(Get-ObjectProperty $coverage "errors").Count -ne 0) { Add-CaptureProblem $problems "domain_coverage_errors" }

    $seenById = @{}
    $perDomain = Get-ObjectProperty $compact "per_domain"
    if ($null -ne $perDomain) {
        foreach ($property in $perDomain.PSObject.Properties) {
            $row = $property.Value
            $id = [int](Get-ObjectProperty $row "domain_id")
            if ($seenById.ContainsKey($id)) { Add-CaptureProblem $problems "duplicate_domain_id_$id" }
            else { $seenById[$id] = $row }
        }
    }
    $actualIds = @($seenById.Keys | Sort-Object)
    if (($actualIds -join ',') -ne (($expectedIds | Sort-Object) -join ',')) { Add-CaptureProblem $problems "registry_roster_mismatch" }
    foreach ($id in $expectedIds) {
        if (-not $seenById.ContainsKey($id)) { Add-CaptureProblem $problems "domain_${id}_missing"; continue }
        $row = $seenById[$id]
        if ([int64](Get-ObjectProperty $row "record_count") -ne $frameCount) { Add-CaptureProblem $problems "domain_${id}_record_count_mismatch" }
        if ($expectedDataIds -contains $id -and [int64](Get-ObjectProperty $row "data_record_count") -ne $frameCount) { Add-CaptureProblem $problems "domain_${id}_data_record_count_mismatch" }
        $rawFlags = [uint32](Get-ObjectProperty $row "status_flags_or")
        $effectiveFlags = Get-ObjectProperty $row "effective_failure_status_flags_or"
        if ($null -eq $effectiveFlags) {
            # Fail-closed: an absent/unreadable per-domain effective-failure field
            # means we cannot evaluate whether this domain masked a real failure,
            # so it is a validation FAILURE, not a silent pass.
            Add-CaptureProblem $problems "domain_${id}_effective_failure_status_flags_or_missing"
        }
        elseif (($rawFlags -band $script:CanonicalCompactFailureMask) -ne 0 -and ([uint32]$effectiveFlags -band $script:CanonicalCompactFailureMask) -ne 0) {
            Add-CaptureProblem $problems "domain_${id}_effective_failure_status"
        }
    }

    $latch = Get-ObjectProperty $compact "hour_latch"
    if ($null -eq $latch) { Add-CaptureProblem $problems "hour_latch_summary_missing" }
    else {
        if ((Get-ObjectProperty $latch "requested") -ne $true -or (Get-ObjectProperty $latch "enabled") -ne $true) { Add-CaptureProblem $problems "hour_latch_not_enabled" }
        if ((Get-ObjectProperty $latch "latch_requested_but_inactive") -eq $true) { Add-CaptureProblem $problems "hour_latch_inactive" }
        if ((Get-ObjectProperty $latch "coverage_invariant_ok") -ne $true) { Add-CaptureProblem $problems "hour_latch_coverage_failed" }
        if ([int64](Get-ObjectProperty $latch "hours_unaccounted") -ne 0) { Add-CaptureProblem $problems "hour_latch_unaccounted_hours" }
        if ([int64](Get-ObjectProperty $latch "capture_attempts") -ne $frameCount) { Add-CaptureProblem $problems "hour_latch_attempt_count_mismatch" }
        if ([int64](Get-ObjectProperty $latch "frames_emitted_to_sink") -ne $frameCount) { Add-CaptureProblem $problems "hour_latch_sink_count_mismatch" }
        if ((Get-ObjectProperty $latch "sink_write_success_verified") -ne $true) { Add-CaptureProblem $problems "sink_write_success_unverified" }
        if ([int64](Get-ObjectProperty $latch "hours_coalesced") -ne 0) { Add-CaptureProblem $problems "coalesced_game_hours" }
        if ([int64](Get-ObjectProperty $latch "capture_failed") -ne 0 -or [int64](Get-ObjectProperty $latch "recorded_failed") -ne 0) { Add-CaptureProblem $problems "hour_latch_failed_captures" }
    }

    $finalize = Get-ObjectProperty $manifest "host_finalize"
    $fold = Get-ObjectProperty $finalize "fold"
    if ($null -eq $fold -or (Get-ObjectProperty $fold "skipped") -eq $true) { Add-CaptureProblem $problems "host_fold_missing_or_skipped" }
    else {
        if ([int64](Get-ObjectProperty $fold "truncated_tail_bytes") -ne 0) { Add-CaptureProblem $problems "fold_truncated_tail" }
        if ($null -ne (Get-ObjectProperty $fold "stop_reason")) { Add-CaptureProblem $problems "fold_stop_reason_present" }
        if ((Get-ObjectProperty $fold "ends_at_frame_end") -ne $true) { Add-CaptureProblem $problems "fold_not_closed_at_frame_end" }
        if ([int64](Get-ObjectProperty $fold "malformed_payload_records") -ne 0) { Add-CaptureProblem $problems "fold_malformed_payload_records" }
    }
    $verify = Get-ObjectProperty $finalize "verify"
    if ($null -eq $verify) { Add-CaptureProblem $problems "container_verify_missing" }
    else {
        if ((Get-ObjectProperty $verify "stream_sha256_matches_capture") -ne $true) { Add-CaptureProblem $problems "container_stream_sha_mismatch" }
        if ([int64](Get-ObjectProperty $verify "entries_verified") -le 0) { Add-CaptureProblem $problems "container_no_entries_verified" }
    }

    $replayFile = [string](Get-ObjectProperty $manifest "replay_file")
    if ([string]::IsNullOrWhiteSpace($replayFile) -or [System.IO.Path]::GetFileName($replayFile) -ne $replayFile) { Add-CaptureProblem $problems "invalid_replay_file_name" }
    elseif (-not (Test-Path -LiteralPath (Join-Path $RunDir $replayFile) -PathType Leaf)) { Add-CaptureProblem $problems "replay_container_missing" }
    $rawPath = Join-Path $RunDir "collector_stream.bin"
    if (-not (Test-Path -LiteralPath $rawPath -PathType Leaf)) { Add-CaptureProblem $problems "raw_stream_missing" }

    $packaging = Get-ObjectProperty $manifest "replay_packaging"
    $packaged = @{}
    foreach ($item in @(Get-ObjectProperty $packaging "static_artifacts")) {
        $name = [string](Get-ObjectProperty $item "name")
        $sha = [string](Get-ObjectProperty $item "sha256")
        if ($packaged.ContainsKey($name)) { Add-CaptureProblem $problems "duplicate_static_artifact_$name" }
        $packaged[$name] = $sha
    }
    if ((@($packaged.Keys | Sort-Object) -join ',') -ne (($script:RequiredStaticArtifactNames | Sort-Object) -join ',')) { Add-CaptureProblem $problems "static_artifact_roster_mismatch" }
    foreach ($name in $script:RequiredStaticArtifactNames) {
        if (-not $packaged.ContainsKey($name) -or -not (Test-HexSha256 $packaged[$name])) { Add-CaptureProblem $problems "static_artifact_${name}_sha_invalid"; continue }
        if ($StaticArtifactPaths.ContainsKey($name)) {
            $path = [string]$StaticArtifactPaths[$name]
            if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { Add-CaptureProblem $problems "static_artifact_${name}_source_missing" }
            else {
                $actual = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
                if ($actual -ne ([string]$packaged[$name]).ToLowerInvariant()) { Add-CaptureProblem $problems "static_artifact_${name}_source_sha_mismatch" }
            }
        }
    }

    if (-not (Test-Path -LiteralPath $SpeedProofPath -PathType Leaf)) { Add-CaptureProblem $problems "speed5_proof_missing" }
    else {
        try { $speedProof = Get-Content -LiteralPath $SpeedProofPath -Raw | ConvertFrom-Json }
        catch { $speedProof = $null; Add-CaptureProblem $problems "speed5_proof_not_json" }
        if ($null -ne $speedProof) {
            if ([string](Get-ObjectProperty $speedProof "schema") -ne "battle-plan.speed5_proof.v2" -or [int](Get-ObjectProperty $speedProof "speed_raw") -ne 4) { Add-CaptureProblem $problems "speed5_not_verified" }
            if (-not (Test-HexSha256 (Get-ObjectProperty $speedProof "domain57_game_speed_gate_v2_sha256"))) { Add-CaptureProblem $problems "speed_root_semantics_not_verified" }
            if ([int64](Get-ObjectProperty $speedProof "sample_count") -lt 3) { Add-CaptureProblem $problems "speed5_insufficient_samples" }
            if (-not (Test-HexSha256 (Get-ObjectProperty $speedProof "exe_sha256"))) { Add-CaptureProblem $problems "speed5_exe_sha_invalid" }
            if (-not (Test-HexSha256 (Get-ObjectProperty $speedProof "collector_control_sha256"))) { Add-CaptureProblem $problems "collector_control_sha_invalid" }
        }
    }

    if (-not [string]::IsNullOrWhiteSpace($ExpectedStagedDll)) {
        if ([string](Get-ObjectProperty $manifest "dll") -ne $ExpectedStagedDll) { Add-CaptureProblem $problems "manifest_dll_is_not_unique_staged_copy" }
    }

    $capturePlan = $null
    if (-not [string]::IsNullOrWhiteSpace($CapturePlanPath) -and (Test-Path -LiteralPath $CapturePlanPath -PathType Leaf)) {
        try { $capturePlan = Get-Content -LiteralPath $CapturePlanPath -Raw | ConvertFrom-Json }
        catch { $capturePlan = $null; Add-CaptureProblem $problems "capture_plan_not_json" }
        if ($null -ne $capturePlan) {
            $planSourceInventory=[string](Get-ObjectProperty $capturePlan "static_bundle_source_inventory")
            if (-not (Test-Path -LiteralPath $SourceInventoryPath -PathType Leaf)) { Add-CaptureProblem $problems "source_inventory_missing" }
            elseif ([string]::IsNullOrWhiteSpace($planSourceInventory) -or [System.IO.Path]::GetFullPath($planSourceInventory) -cne [System.IO.Path]::GetFullPath($SourceInventoryPath) -or [string](Get-ObjectProperty $capturePlan "static_bundle_source_inventory_sha256") -cne (Get-FileHash -LiteralPath $SourceInventoryPath -Algorithm SHA256).Hash.ToLowerInvariant()) { Add-CaptureProblem $problems "source_inventory_plan_binding_mismatch" }
            $planLiveResolution=[string](Get-ObjectProperty $capturePlan "static_bundle_live_resolution_report")
            if (-not (Test-Path -LiteralPath $LiveResolutionReportPath -PathType Leaf)) { Add-CaptureProblem $problems "live_resolution_report_missing" }
            elseif ([string]::IsNullOrWhiteSpace($planLiveResolution) -or [System.IO.Path]::GetFullPath($planLiveResolution) -cne [System.IO.Path]::GetFullPath($LiveResolutionReportPath) -or [string](Get-ObjectProperty $capturePlan "static_bundle_live_resolution_report_sha256") -cne (Get-FileHash -LiteralPath $LiveResolutionReportPath -Algorithm SHA256).Hash.ToLowerInvariant()) { Add-CaptureProblem $problems "live_resolution_report_plan_binding_mismatch" }
            else {
                try { $liveResolution = Get-Content -LiteralPath $LiveResolutionReportPath -Raw | ConvertFrom-Json }
                catch { $liveResolution = $null; Add-CaptureProblem $problems "live_resolution_report_not_json" }
                if ($null -ne $liveResolution) {
                    $reportedProcess = Get-ObjectProperty $liveResolution "process"
                    $reportedPid = [int](Get-ObjectProperty $reportedProcess "pid")
                    $reportedCreation = [int64](Get-ObjectProperty $reportedProcess "creation_filetime_utc")
                    $plannedPid = [int](Get-ObjectProperty $capturePlan "pid")
                    $plannedCreation = [int64](Get-ObjectProperty $capturePlan "process_creation_filetime_utc")
                    if (
                        $reportedPid -le 0 -or $reportedCreation -le 0 -or
                        $plannedPid -le 0 -or $plannedCreation -le 0 -or
                        $reportedPid -ne $plannedPid -or $reportedCreation -ne $plannedCreation
                    ) { Add-CaptureProblem $problems "live_resolution_process_identity_mismatch" }
                }
            }
        }
    }
    $expectedCaptureId = [string](Get-ObjectProperty $capturePlan "capture_id")
    $expectedSessionId = [string](Get-ObjectProperty $manifest "session_id")
    $expectedStreamSha256 = [string](Get-ObjectProperty $finalize "stream_sha256")
    if ([string]::IsNullOrWhiteSpace($expectedCaptureId)) { Add-CaptureProblem $problems "capture_id_missing_from_plan" }
    if ([string]::IsNullOrWhiteSpace($expectedSessionId)) { Add-CaptureProblem $problems "session_id_missing_from_manifest" }
    if (-not (Test-HexSha256 $expectedStreamSha256)) { Add-CaptureProblem $problems "capture_stream_sha256_invalid" }
    if (
        $null -ne $capturePlan -and
        [string](Get-ObjectProperty $capturePlan "session_id") -cne $expectedSessionId
    ) { Add-CaptureProblem $problems "capture_plan_session_id_mismatch" }

    # These gates are deliberately independent of recorder-authored claims. A
    # caller cannot obtain a production PASS without executing every verifier.
    $toolResults = [System.Collections.Generic.List[object]]::new()
    $toolLogDir = Join-Path $RunDir "independent_validation"
    $requiredConfig = @($CompactGateExe,$FoldCheckExe,$ContainerVerifyExe,$PythonExe,$StaticBundleScript,$StaticBundleDir,$GameExePath,$ModStatePath,$DlcStatePath,$LoadOrderPath,$SourceInventoryPath,$LiveResolutionReportPath,$CapturePlanPath,$SpeedRootGatePath,$CollectorControlPath,$SpeedMonitorPath,$SpeedStreamVerifier,$SpeedEvidenceTranscriptPath,$SpeedEvidenceNonceLedgerRoot,$SpeedEvidenceVerifier,$ExpectedSpeedEvidenceVerifierSha256,$ExpectedSpeedEvidenceProbeSha256,$ExpectedSpeedStaticDisasmSha256)
    if (@($requiredConfig | Where-Object { [string]::IsNullOrWhiteSpace([string]$_) }).Count -ne 0) {
        Add-CaptureProblem $problems "independent_gate_configuration_missing"
    }
    else {
        $compactReport = Join-Path $toolLogDir "compact_gate.report.json"
        $toolResults.Add((Invoke-IndependentCaptureGate -Name "compact_gate" -FilePath $CompactGateExe -Arguments @("--run-dir",$RunDir,"--min-game-hours",([string]$requestedGameHours),"--max-stream-bytes","2199023255552","--max-record-bytes","268435456","--max-bytes-per-frame","2147483648","--write-report",$compactReport) -LogDir $toolLogDir -TimeoutSeconds $ToolTimeoutSeconds -CancellationGraceSeconds $CancellationGraceSeconds))
        $foldReport = Join-Path $toolLogDir "fold_check.report.json"
        $toolResults.Add((Invoke-IndependentCaptureGate -Name "fold_check" -FilePath $FoldCheckExe -Arguments @("hash",$rawPath,"--expected-profile","production","--out",$foldReport) -LogDir $toolLogDir -TimeoutSeconds $ToolTimeoutSeconds -CancellationGraceSeconds $CancellationGraceSeconds))
        $replayPath = if ([string]::IsNullOrWhiteSpace($replayFile)) { Join-Path $RunDir "native_session.battleplan" } else { Join-Path $RunDir $replayFile }
        $toolResults.Add((Invoke-IndependentCaptureGate -Name "container_verify" -FilePath $ContainerVerifyExe -Arguments @("verify",$replayPath) -LogDir $toolLogDir -TimeoutSeconds $ToolTimeoutSeconds -CancellationGraceSeconds $CancellationGraceSeconds))
        $toolResults.Add((Invoke-IndependentCaptureGate -Name "static_bundle_verify" -FilePath $PythonExe -Arguments @($StaticBundleScript,"verify","--bundle",$StaticBundleDir,"--exe",$GameExePath,"--mod-state",$ModStatePath,"--dlc-state",$DlcStatePath,"--load-order",$LoadOrderPath,"--source-inventory",$SourceInventoryPath,"--live-resolution-report",$LiveResolutionReportPath,"--expected-capture-id",$expectedCaptureId,"--expected-session-id",$expectedSessionId,"--expected-stream-sha256",$expectedStreamSha256) -LogDir $toolLogDir -TimeoutSeconds $ToolTimeoutSeconds -CancellationGraceSeconds $CancellationGraceSeconds))
        $speedReport = Join-Path $toolLogDir "speed5_stream_gate.report.json"
        $toolResults.Add((Invoke-IndependentCaptureGate -Name "speed5_stream_gate" -FilePath $PythonExe -Arguments @($SpeedStreamVerifier,"--stream",$rawPath,"--manifest",$manifestPath,"--speed-proof",$SpeedProofPath,"--monitor",$SpeedMonitorPath,"--capture-plan",$CapturePlanPath,"--root-gate",$SpeedRootGatePath,"--collector-control",$CollectorControlPath,"--evidence-transcript",$SpeedEvidenceTranscriptPath,"--nonce-ledger-root",$SpeedEvidenceNonceLedgerRoot,"--evidence-verifier",$SpeedEvidenceVerifier,"--expected-verifier-sha256",$ExpectedSpeedEvidenceVerifierSha256,"--expected-probe-sha256",$ExpectedSpeedEvidenceProbeSha256,"--expected-static-sha256",$ExpectedSpeedStaticDisasmSha256,"--out",$speedReport) -LogDir $toolLogDir -TimeoutSeconds $ToolTimeoutSeconds -CancellationGraceSeconds $CancellationGraceSeconds))
        foreach ($toolResult in $toolResults) { if (-not $toolResult.passed) { Add-CaptureProblem $problems ("independent_{0}_failed" -f $toolResult.name) } }
    }

    return [pscustomobject]@{
        schema = "battle-plan.production_capture_validation.v2"
        ok = ($problems.Count -eq 0)
        canonical_failure_mask = ("0x{0:X}" -f $script:CanonicalCompactFailureMask)
        expected_domain_count = $expectedIds.Count
        frame_count = $frameCount
        manifest_path = $manifestPath
        problems = @($problems)
        independent_gates = @($toolResults)
    }
}
