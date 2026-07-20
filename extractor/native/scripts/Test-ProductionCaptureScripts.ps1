param()
$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
. (Join-Path $scriptDir "ProductionCapture.Validation.ps1")
$nativeRoot = (Resolve-Path -LiteralPath (Join-Path $scriptDir "..")).Path
$registry = Join-Path $nativeRoot "domain_registry.json"
$repoRoot=(Resolve-Path (Join-Path $nativeRoot '..\..')).Path
$targetRoot = if ([string]::IsNullOrWhiteSpace($env:CARGO_TARGET_DIR)) { Join-Path $repoRoot '.cargo-target' } else { [System.IO.Path]::GetFullPath($env:CARGO_TARGET_DIR) }
$releaseRoot=Join-Path $targetRoot 'release'
$python=(Get-Command python -ErrorAction Stop).Source
$cargo=(Get-Command cargo -ErrorAction Stop).Source
$root = Join-Path ([System.IO.Path]::GetTempPath()) ("hoi4_capture_script_test_" + [guid]::NewGuid().ToString("N"))
$passes = 0; $failures = [System.Collections.Generic.List[string]]::new()
function Assert-Case([string]$Name, [bool]$Condition) { if ($Condition) { $script:passes++ } else { $script:failures.Add($Name) } }

# --- F5/F6 acceptance-tooling self-test orchestration ----------------------
# Resolve the current PowerShell host exe (no hardcoded path) so child self-tests
# run in an isolated process and their exit codes are unambiguous.
$psExe = (Get-Process -Id $PID).Path
if ([string]::IsNullOrEmpty($psExe)) { $psExe = "powershell.exe" }

function Test-ScriptSyntax {
    param([Parameter(Mandatory = $true)][string]$Path)
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        return @("script not found: $Path")
    }
    $tokens = $null
    $errors = $null
    [void][System.Management.Automation.Language.Parser]::ParseFile($Path, [ref]$tokens, [ref]$errors)
    $problems = @()
    foreach ($e in $errors) {
        $problems += ("{0}:{1} {2}" -f (Split-Path -Leaf $Path), $e.Extent.StartLineNumber, $e.Message)
    }
    return $problems
}

function Invoke-ChildSelfTest {
    param([Parameter(Mandatory = $true)][string]$Path)
    # Fresh child process; -SelfTest scripts `exit 0/1` so $LASTEXITCODE is clean.
    # Route the child's stdout straight to the host so this function's return value
    # is ONLY the exit code, not the captured output lines.
    & $psExe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $Path -SelfTest | Out-Host
    return $LASTEXITCODE
}

try {
    New-Item -ItemType Directory -Path $root | Out-Null

    # Phase 0 (lane F5/F6): PowerShell syntax gate over every acceptance-tooling
    # script, then the -SelfTest of each script that carries one, in an isolated
    # child process. Folds into the same pass/fail accounting as the heavyweight
    # fixture and cargo gates below. Runs no game.
    $syntaxTargets = @(
        "ProductionCapture.Validation.ps1",
        "Get-FullGameSizeProjection.ps1",
        "Invoke-AcceptanceLadder.ps1",
        "Test-ProductionCaptureScripts.ps1",
        # The ladder reuses the launcher; a syntax break there breaks the ladder.
        "Start-Hoi4ProductionCaptureSafe.ps1",
        "Start-Hoi4ProductionSpeed5Capture.ps1"
    )
    foreach ($name in $syntaxTargets) {
        $problems = @(Test-ScriptSyntax -Path (Join-Path $scriptDir $name))
        Assert-Case ("acceptance-tooling syntax: $name " + ($problems -join '; ')) ($problems.Count -eq 0)
    }
    $selfTestTargets = @(
        "ProductionCapture.Validation.ps1",
        "Get-FullGameSizeProjection.ps1",
        "Invoke-AcceptanceLadder.ps1"
    )
    foreach ($name in $selfTestTargets) {
        $code = Invoke-ChildSelfTest -Path (Join-Path $scriptDir $name)
        Assert-Case ("acceptance-tooling self-test (exit $code): $name") ($code -eq 0)
    }
    $domains = @(Get-RegistryCaptureDomains $registry)
    $perDomain = [ordered]@{}
    foreach ($d in $domains) {
        $perDomain[[string]$d.name] = [ordered]@{ domain_id=[int]$d.id; record_count=10; data_record_count=10; status_flags_or=0; effective_failure_status_flags_or=0 }
    }
    $manifest = [ordered]@{
        schema="battle-plan.native_session.v1"; stream_format="compact"; dll="X:\unique\collector.dll"; session_id="test"; pid=123
        replay_file="native_session.battleplan"; skipped_game_hours=0; unique_game_hour_count=10
        first_game_hour_raw=10;last_game_hour_raw=19
        parse_errors=@(); ordering_errors=@(); hourly_requirement=[ordered]@{met=$true;requested_game_hours=9}
        production_profile=[ordered]@{enabled=$true;profile_domains_not_scheduled=@();missing_required_static_artifacts=@()}
        compact_stream=[ordered]@{
            frame_end_record_count=10;frame_time_us=[ordered]@{count=10;avg=20000;p50=25000;p95=30000;p99=32000;max=99000};decodable=$true;parse_errors=@();ordering_errors=@();failure_status_flags_or=0;failure_status_seen=$false
            domain_coverage=[ordered]@{ok=$true;errors=@()};per_domain=$perDomain
            hour_latch=[ordered]@{requested=$true;enabled=$true;latch_requested_but_inactive=$false;coverage_invariant_ok=$true;hours_unaccounted=0;capture_attempts=10;frames_emitted_to_sink=10;sink_write_success_verified=$true;hours_coalesced=0;capture_failed=0;recorded_failed=0}
        }
        acceptance_telemetry=[ordered]@{
            sink_write_us=[ordered]@{count=10;avg=25;p50=20;p95=60;p99=100;max=120}
            clock_skew=[ordered]@{wall_elapsed_us=300000000;mono_elapsed_us=300000100;drift_us=-100;wall_monotonic_ok=$true}
        }
        host_finalize=[ordered]@{stream_sha256=('d'*64);fold=[ordered]@{skipped=$false;truncated_tail_bytes=0;stop_reason=$null;ends_at_frame_end=$true;malformed_payload_records=0};verify=[ordered]@{stream_sha256_matches_capture=$true;entries_verified=10}}
        replay_packaging=[ordered]@{static_artifacts=@()}
    }
    $static=@{}
    foreach ($name in $script:RequiredStaticArtifactNames) {
        $path=Join-Path $root "$name.json"; Set-Content -LiteralPath $path -Value "{}" -Encoding UTF8; $static[$name]=$path
        $manifest.replay_packaging.static_artifacts += [ordered]@{name=$name;sha256=(Get-FileHash $path -Algorithm SHA256).Hash.ToLowerInvariant()}
    }
    Set-Content -LiteralPath (Join-Path $root "collector_stream.bin") -Value "stream" -Encoding UTF8
    Set-Content -LiteralPath (Join-Path $root "native_session.battleplan") -Value "container" -Encoding UTF8
    $proof=[ordered]@{schema='battle-plan.speed5_proof.v2';domain57_game_speed_gate_v2_sha256=('b'*64);speed_raw=4;sample_count=10;exe_sha256=('a'*64)}
    $proof | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $root "speed5_proof.json") -Encoding UTF8
    function Write-Manifest($Value) { $Value | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath (Join-Path $root "native_session_manifest.json") -Encoding UTF8 }
    [ordered]@{capture_id='capture-test';session_id='test';pid=123;process_start_utc='2026-07-16T00:00:00.0000000Z';process_creation_filetime_utc=134126496000000000} | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $root 'production_capture_plan.json') -Encoding UTF8
    Set-Content -LiteralPath (Join-Path $root 'speed_root_gate.json') -Value '{}' -Encoding UTF8
    Set-Content -LiteralPath (Join-Path $root 'collector_control.json') -Value '{}' -Encoding UTF8
    1..3 | ForEach-Object { '{"speed_raw":4,"paused":false,"game_hour_raw":10}' | Add-Content -LiteralPath (Join-Path $root 'speed5_monitor.jsonl') -Encoding UTF8 }
    function Run-Gate { Test-ProductionCaptureArtifacts -RunDir $root -RegistryPath $registry -SpeedProofPath (Join-Path $root "speed5_proof.json") -StaticArtifactPaths $static -ExpectedStagedDll "X:\unique\collector.dll" -CompactGateExe (Join-Path $releaseRoot 'compact_replay_gate.exe') -FoldCheckExe (Join-Path $releaseRoot 'fold_check.exe') -ContainerVerifyExe (Join-Path $releaseRoot 'pack_replay.exe') -PythonExe $python -StaticBundleScript (Join-Path $repoRoot 'extractor\game_defs\static_bundle.py') -StaticBundleDir $root -GameExePath (Join-Path $root 'templates_equipment_catalog.json') -ModStatePath (Join-Path $root 'localisation_join.json') -DlcStatePath (Join-Path $root 'decisions_static_definitions.json') -LoadOrderPath (Join-Path $root 'idea_tech_loc_xref_static.json') -SourceInventoryPath (Join-Path $root 'templates_equipment_catalog.json') -LiveResolutionReportPath (Join-Path $root 'localisation_join.json') -CapturePlanPath (Join-Path $root 'production_capture_plan.json') -SpeedRootGatePath (Join-Path $root 'speed_root_gate.json') -CollectorControlPath (Join-Path $root 'collector_control.json') -SpeedMonitorPath (Join-Path $root 'speed5_monitor.jsonl') -SpeedStreamVerifier (Join-Path $scriptDir 'verify_speed5_stream.py') -SpeedEvidenceTranscriptPath (Join-Path $root 'speed_root_gate.json') -SpeedEvidenceNonceLedgerRoot $root -SpeedEvidenceVerifier (Join-Path $scriptDir 'verify_speed_root_evidence_v2.py') -ExpectedSpeedEvidenceVerifierSha256 ('a'*64) -ExpectedSpeedEvidenceProbeSha256 ('b'*64) -ExpectedSpeedStaticDisasmSha256 ('c'*64) -ToolTimeoutSeconds 30 }
    Write-Manifest $manifest
    $dummyGate=Run-Gate
    Assert-Case "dummy fixture rejected" (-not $dummyGate.ok)
    foreach($name in @('compact_gate','fold_check','container_verify','static_bundle_verify','speed5_stream_gate')) { Assert-Case "real $name rejects dummy" ($dummyGate.problems -contains "independent_${name}_failed") }
    Assert-Case "all real gates executed" (@($dummyGate.independent_gates).Count -eq 5)
    $notStarted=@($dummyGate.independent_gates | Where-Object started -ne $true | ForEach-Object { "$($_.name):$($_.problem)" })
    Assert-Case ("all real gate processes started " + ($notStarted -join ',')) ($notStarted.Count -eq 0)
    $manifest.compact_stream.failure_status_flags_or=1; Write-Manifest $manifest; Assert-Case "failure mask" (-not (Run-Gate).ok); $manifest.compact_stream.failure_status_flags_or=0
    $manifest.compact_stream.hour_latch.enabled=$false; Write-Manifest $manifest; Assert-Case "latch off" (-not (Run-Gate).ok); $manifest.compact_stream.hour_latch.enabled=$true
    $manifest.compact_stream.per_domain.($domains[0].name).record_count=9; Write-Manifest $manifest; Assert-Case "domain hour gap" (-not (Run-Gate).ok); $manifest.compact_stream.per_domain.($domains[0].name).record_count=10
    $removed=$manifest.compact_stream.per_domain.($domains[0].name); $manifest.compact_stream.per_domain.Remove([string]$domains[0].name); Write-Manifest $manifest; Assert-Case "roster gap" (-not (Run-Gate).ok); $manifest.compact_stream.per_domain[[string]$domains[0].name]=$removed
    $manifest.host_finalize.fold.truncated_tail_bytes=1; Write-Manifest $manifest; Assert-Case "truncation" (-not (Run-Gate).ok); $manifest.host_finalize.fold.truncated_tail_bytes=0
    $manifest.compact_stream.frame_time_us.max=100000; Write-Manifest $manifest; Assert-Case "100ms frame rejected" ((Run-Gate).problems -contains 'frame_time_over_100ms'); $manifest.compact_stream.frame_time_us.max=99999
    $manifest.compact_stream.frame_time_us.count=9; Write-Manifest $manifest; Assert-Case "timing count mismatch" ((Run-Gate).problems -contains 'frame_time_sample_count_mismatch'); $manifest.compact_stream.frame_time_us.count=10
    $proof.speed_raw=3; $proof | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $root "speed5_proof.json"); Write-Manifest $manifest; Assert-Case "not speed5" (-not (Run-Gate).ok)
    & $python (Join-Path $scriptDir 'verify_speed5_stream.py') --self-test
    Assert-Case 'speed stream positive and mutations' ($LASTEXITCODE -eq 0)
    & $python (Join-Path $scriptDir 'verify_speed_root_evidence_v2.py') --self-test
    Assert-Case 'speed root v2 evidence positive and mutations' ($LASTEXITCODE -eq 0)
    & $python (Join-Path $scriptDir 'collect_speed_root_evidence_v2.py') self-test
    Assert-Case 'speed root v2 durable nonce outcomes' ($LASTEXITCODE -eq 0)
    $driverText=Get-Content -LiteralPath (Join-Path $scriptDir 'Start-Hoi4ProductionCaptureSafe.ps1') -Raw
    $verifierSha=(Get-FileHash -LiteralPath (Join-Path $scriptDir 'verify_speed_root_evidence_v2.py') -Algorithm SHA256).Hash.ToLowerInvariant()
    $probeSha=(Get-FileHash -LiteralPath (Join-Path $scriptDir 'collect_speed_root_evidence_v2.py') -Algorithm SHA256).Hash.ToLowerInvariant()
    $schemaSha=(Get-FileHash -LiteralPath (Join-Path $scriptDir 'speed_root_evidence_v2.schema.json') -Algorithm SHA256).Hash.ToLowerInvariant()
    Assert-Case 'canonical speed evidence bytes pinned' ($driverText.Contains("`$expectedSpeedEvidenceVerifierSha = `"$verifierSha`"") -and $driverText.Contains("`$expectedSpeedEvidenceProbeSha = `"$probeSha`"") -and $driverText.Contains("`$expectedSpeedEvidenceSchemaSha = `"$schemaSha`""))
    Assert-Case 'durable marker precedes launch' ($driverText.IndexOf('Write-DurableJson -Path $attemptMarkerPath') -lt $driverText.IndexOf('if (-not $process.Start())'))
    Assert-Case 'bounded child cleanup present' ($driverText.Contains('CloseMainWindow()') -and $driverText.Contains('$process.Kill()') -and $driverText.Contains('WaitForExit'))
    Assert-Case 'unresolved attempt refusal present' $driverText.Contains('unresolved prior injection attempt exists')
    Assert-Case 'resolved attempt retains hash-bound validation receipt' (
        $driverText.Contains('battle-plan.production_capture_validation_receipt.v1') -and
        $driverText.Contains('validation_report_sha256') -and
        $driverText.Contains('manifest_sha256') -and
        $driverText.Contains('capture_plan_sha256') -and
        $driverText.Contains('session_id = $SessionId') -and
        $driverText.Contains('capture_id = $CaptureId') -and
        $driverText.Contains('validation_receipt=$validationReceipt')
    )
    Assert-Case 'acceptance receipt binds extraction replay nonce and observations' (
        $driverText.Contains('snapshot_extraction_report_sha256') -and
        $driverText.Contains('replay_sha256') -and
        $driverText.Contains('acceptance_test_nonce') -and
        $driverText.Contains('acceptance_proof') -and
        $driverText.Contains('acceptance_test_observation')
    )
    Assert-Case 'speed root v2 gate passed exclusively to recorder' ($driverText.Contains('"--domain57-game-speed-gate-v2", $SpeedRootGatePath') -and -not $driverText.Contains('"--speed-root-gate"'))
    Assert-Case 'raw transcript file hash passed separately to recorder' ($driverText.Contains('"--domain57-game-speed-transcript-file-sha256", $SpeedEvidenceTranscriptFileSha256'))
    Assert-Case 'raw evidence transcript retained and identity pinned' ($driverText.Contains('retained_speed_evidence_transcript') -and $driverText.Contains('speed_evidence_transcript_file_id'))
    Assert-Case 'semantic and raw transcript hashes are distinct pins' ($driverText.Contains('SpeedEvidenceTranscriptSha256') -and $driverText.Contains('SpeedEvidenceTranscriptFileSha256') -and $driverText.Contains('speed_evidence_transcript_file_sha256'))
    Assert-Case 'nonce ledger and consumed seal pinned' ($driverText.Contains('speed_evidence_nonce_ledger_root') -and $driverText.Contains('speed_evidence_nonce_seal_sha256'))
    Assert-Case 'collector control hash bound to proof' ($driverText.Contains('collector_control_sha256=$collectorControlSha'))
    Assert-Case 'recorder receives admitted static bundle and live source bindings' ($driverText.Contains('"--static-bundle-manifest", $bundleManifestPath') -and $driverText.Contains('"--static-bundle-source-inventory", $SourceInventoryPath') -and $driverText.Contains('"--static-bundle-live-resolution-report", $LiveResolutionReportPath') -and -not $driverText.Contains('"--static-templates-equipment-catalog"'))
    Assert-Case 'capture plan hashes both live static bindings' ($driverText.Contains('static_bundle_source_inventory_sha256') -and $driverText.Contains('static_bundle_live_resolution_report_sha256'))
    Assert-Case 'static admission cross-binds exact target process identity' ($driverText.Contains('liveResolution.process.pid') -and $driverText.Contains('liveResolution.process.creation_filetime_utc') -and $driverText.Contains('process_creation_filetime_utc'))
    Assert-Case 'pre-injection static gate explicitly admits only deferred bundles' ($driverText.Contains('"--allow-deferred-token-gate"') -and $driverText.Contains('static_bundle_phase = "pre_injection_deferred"'))
    Assert-Case 'post-capture static bundle is finalized and strict verified' ($driverText.Contains('"finalize"') -and $driverText.Contains('"--emitted-tokens",$EmittedTokensPath') -and $driverText.Contains('"--expected-capture-id",$CaptureId') -and $driverText.Contains('"--expected-session-id",$SessionId') -and $driverText.Contains('"--expected-stream-sha256",$captureStreamSha256') -and $driverText.Contains('verify_finalized_static_bundle'))
    Assert-Case 'production schedule is descriptor-derived and excludes D18 override' ($driverText.Contains('production_profile_domain_ids') -and -not $driverText.Contains('"--scheduled-domain", "country_identity"') -and -not $driverText.Contains('"--scheduled-domain","country_identity"'))
    $validationText=Get-Content -LiteralPath (Join-Path $scriptDir 'ProductionCapture.Validation.ps1') -Raw
    Assert-Case 'post-capture gate independently rechecks raw transcript' ($validationText.Contains('"--evidence-transcript",$SpeedEvidenceTranscriptPath') -and $validationText.Contains('"--nonce-ledger-root",$SpeedEvidenceNonceLedgerRoot') -and $validationText.Contains('"--evidence-verifier",$SpeedEvidenceVerifier'))
    Assert-Case 'static validation reproduces current live source join' ($validationText.Contains('"--source-inventory",$SourceInventoryPath') -and $validationText.Contains('"--live-resolution-report",$LiveResolutionReportPath'))
    Assert-Case 'post-capture static validation requires exact finalized provenance' ($validationText.Contains('"--expected-capture-id",$expectedCaptureId') -and $validationText.Contains('"--expected-session-id",$expectedSessionId') -and $validationText.Contains('"--expected-stream-sha256",$expectedStreamSha256'))
    Assert-Case 'post-capture validation rechecks static process identity' ($validationText.Contains('live_resolution_process_identity_mismatch') -and $validationText.Contains('creation_filetime_utc'))
    Assert-Case 'active-integration domains are excluded from production roster' ($script:ProductionDomainStatuses -notcontains 'active_integration' -and @($domains | Where-Object { [string]$_.name -eq 'country_identity' }).Count -eq 0)
    Assert-Case 'compact gate receives manifest-bound minimum duration' ($validationText.Contains('"--min-game-hours",([string]$requestedGameHours)'))
    Assert-Case 'fold check receives production roster authority' ($validationText.Contains('"--expected-profile","production"'))
    Assert-Case 'technology sharing is a required static role' ($script:RequiredStaticArtifactNames -contains 'technology_sharing_static')

    # F1a/F4: keep every extractor Cargo surface that is not covered by a plain
    # main-workspace sweep in maintained automation. Scientists now belongs to
    # the main workspace, but remains named here so this gate cannot silently
    # lose the originally certified F1a surface.
    $packageGates = @(
        [pscustomobject]@{
            name = 'project_progress_live_lane'
            arguments = @('test','--manifest-path',(Join-Path $nativeRoot 'project_progress_live_lane\Cargo.toml'),'--all-targets','--quiet')
        },
        [pscustomobject]@{
            name = 'scientists_live_lane'
            arguments = @('test','--manifest-path',(Join-Path $nativeRoot 'Cargo.toml'),'-p','scientists_live_lane','--all-targets','--quiet')
        },
        [pscustomobject]@{
            name = 'occupation_resistance_compliance_live_replay_verify'
            arguments = @('test','--manifest-path',(Join-Path $nativeRoot 'lanes\occupation_resistance_compliance_live\replay_verify\Cargo.toml'),'--all-targets','--quiet')
        }
    )
    foreach ($gate in $packageGates) {
        & $cargo @($gate.arguments)
        Assert-Case ("F1a package gate: " + $gate.name) ($LASTEXITCODE -eq 0)
    }
}
finally { Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue }
Write-Host "production capture script tests: passed=$passes failed=$($failures.Count)"
if ($failures.Count -ne 0) { $failures | ForEach-Object { Write-Error $_ }; exit 1 }
