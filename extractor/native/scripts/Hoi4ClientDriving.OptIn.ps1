Set-StrictMode -Version Latest

$script:Hoi4ClientDrivingOptInEnvironmentVariable = 'BATTLE_PLAN_CLIENT_DRIVING_OPT_IN'
$script:Hoi4ClientDrivingOptInToken = 'I_ACKNOWLEDGE_THIS_TOOL_WILL_CONTROL_HOI4'

function Assert-Hoi4ClientDrivingOptIn {
    param([string]$Caller = $MyInvocation.PSCommandPath)

    $value = [Environment]::GetEnvironmentVariable(
        $script:Hoi4ClientDrivingOptInEnvironmentVariable,
        [EnvironmentVariableTarget]::Process
    )
    if ($value -cne $script:Hoi4ClientDrivingOptInToken) {
        throw (
            "Legacy the game client driving is disabled by default. " +
            "This helper may launch/stop or drive the game client. " +
            "For an isolated, user-authorized evidence run only, set " +
            "$($script:Hoi4ClientDrivingOptInEnvironmentVariable) exactly to " +
            "'$($script:Hoi4ClientDrivingOptInToken)' in the invoking process. " +
            "Caller: $Caller"
        )
    }
}

function Test-Hoi4ClientDrivingInputPath {
    param(
        [Parameter(Mandatory = $true)][string]$RepoRoot,
        [Parameter(Mandatory = $true)][string]$Path
    )

    $repo = [IO.Path]::GetFullPath($RepoRoot).TrimEnd('\')
    $full = [IO.Path]::GetFullPath($Path)
    if (-not $full.StartsWith($repo + '\', [StringComparison]::OrdinalIgnoreCase)) {
        return $false
    }
    $relative = $full.Substring($repo.Length + 1).Replace('\', '/')
    $legacyClientDrivingInputs = @(
        'extractor/native/lanes/design_catalog_live/Invoke-A27AuthenticatedTransactions.ps1',
        'extractor/native/lanes/politics_tooltip_inputs_live/capture_a24_autonomous_scenes.py',
        'extractor/native/lanes/focus_research_decisions_depth_live/tools/capture_a31_fixture_pair.py',
        'extractor/native/scripts/Invoke-A15CounterUiFixture.ps1',
        'extractor/native/scripts/Invoke-A29AuthenticatedClockTransition.ps1',
        'extractor/native/scripts/Invoke-Cid1LifecycleMatrix.ps1',
        'extractor/native/scripts/Invoke-Cid1SdActionDriver.ps1',
        'extractor/native/scripts/Invoke-SinkFailureAcceptance.ps1',
        'extractor/native/scripts/Start-Hoi4EarlyLaunchCaptureSafe.ps1',
        'replay_interface/re_evidence/rendering_laws/CAP6/authenticated_ensure_paused.ps1',
        'replay_interface/re_evidence/rendering_laws/XN14Q/xn14q_autonomous_camera.py',
        'replay_interface/re_evidence/rendering_laws/capture_harness/authenticated_activate_window.ps1',
        'replay_interface/re_evidence/rendering_laws/capture_harness/authenticated_foreground_send_input.ps1',
        'replay_interface/re_evidence/rendering_laws/capture_harness/authenticated_one_hour_pulse.ps1',
        'replay_interface/re_evidence/rendering_laws/capture_harness/authenticated_post_click.ps1',
        'replay_interface/re_evidence/rendering_laws/capture_harness/authenticated_post_key.ps1',
        'replay_interface/re_evidence/rendering_laws/capture_harness/authenticated_strong_key_hover.ps1',
        'replay_interface/re_evidence/rendering_laws/capture_harness/authenticated_unpause.ps1',
        'replay_interface/re_evidence/rendering_laws/capture_harness/scenario_runner.ps1',
        'replay_interface/renderdoc/Invoke-IntelMapAnnotationBridgeAcquisition.ps1',
        'replay_interface/renderdoc/Invoke-RestoreGateH1H2Acquisition.ps1',
        'replay_interface/renderdoc/acquire_restore_gate_h1_h2.py',
        'replay_interface/renderdoc/capture_vanilla.ps1'
    )
    return $legacyClientDrivingInputs -ccontains $relative
}
