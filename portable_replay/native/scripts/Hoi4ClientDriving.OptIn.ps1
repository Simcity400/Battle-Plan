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
    # This is the complete source-input closure for the unshipped evidence
    # workflows that can transitively launch, stop, or write the game state.
    # Control-Hoi4Game.ps1 is the sharp boundary: it can write game speed and
    # the engine pause byte through WriteProcessMemory. Every controlling entry
    # point must also call Assert-Hoi4ClientDrivingOptIn before it crosses a
    # driving boundary; this roster keeps those inputs out of production
    # packaging even if a caller later omits that runtime check.
    $legacyClientDrivingInputs = @(
        'extractor/native/scripts/Start-Hoi4EarlyLaunchCaptureSafe.ps1',
        'extractor/native/scripts/Invoke-HeadlessRawCapture.ps1',
        'extractor/native/scripts/Invoke-HeadlessRawCaptureUntilClean.ps1',
        'extractor/native/scripts/Measure-InjectionCrashRate.ps1',
        'extractor/native/scripts/Control-Hoi4Game.ps1'
    )
    return $legacyClientDrivingInputs -ccontains $relative
}
