[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)] [string] $Hoi4Exe,
    [Parameter(Mandatory = $true)] [string] $CollectorDll,
    [Parameter(Mandatory = $true)] [string] $ProfilePath,
    [Parameter(Mandatory = $true)] [string] $ReceiptPath,
    [Parameter(Mandatory = $true)] [uint32] $ExistingPid,
    [string] $SourceInventory = '',
    [string] $Python = 'python',
    [int] $ProcessTimeoutSeconds = 300
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
. (Join-Path $PSScriptRoot 'Hoi4ClientDriving.OptIn.ps1')
Assert-Hoi4ClientDrivingOptIn -Caller $MyInvocation.MyCommand.Path
$expectedExeSha256 = '870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad'
$requiredFamilies = @(
    'air_selection_list',
    'mio_aggregated_bonus_smooth_list',
    'mio_variant_history_smooth_list',
    'mio_production_history_smooth_list'
)

function Write-CreateNewUtf8([string] $Path, [string] $Text) {
    $full = [IO.Path]::GetFullPath($Path)
    [IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($full)) | Out-Null
    $temporary = [IO.Path]::Combine([IO.Path]::GetDirectoryName($full), ([IO.Path]::GetRandomFileName() + '.tmp'))
    try {
        [IO.File]::WriteAllText($temporary, $Text, [Text.UTF8Encoding]::new($false))
        if ([IO.File]::Exists($full)) { throw "Refusing to replace A28 action artifact: $full" }
        [IO.File]::Move($temporary, $full)
    } finally {
        if ([IO.File]::Exists($temporary)) { [IO.File]::Delete($temporary) }
    }
}

$resolvedExe = (Resolve-Path -LiteralPath $Hoi4Exe).Path
$resolvedCollector = (Resolve-Path -LiteralPath $CollectorDll).Path
$resolvedProfile = (Resolve-Path -LiteralPath $ProfilePath).Path
$receiptFull = [IO.Path]::GetFullPath($ReceiptPath)
$artifactDir = [IO.Path]::GetDirectoryName($receiptFull)
if ((Get-FileHash -LiteralPath $resolvedExe -Algorithm SHA256).Hash.ToLowerInvariant() -cne $expectedExeSha256) {
    throw 'A28 action driver executable is not exact a729.'
}
$profileVerifier = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..\documents\dossiers\A28\verify_a28_action_profile.py'))
$validationArguments = @(
    $profileVerifier,
    '--profile', $resolvedProfile,
    '--game-root', ([IO.Path]::GetDirectoryName($resolvedExe))
)
if (-not [string]::IsNullOrWhiteSpace($SourceInventory)) {
    $validationArguments += @('--source-inventory', (Resolve-Path -LiteralPath $SourceInventory).Path)
}
$profileVerificationText = (& $Python @validationArguments | Out-String).Trim()
if ($LASTEXITCODE -ne 0 -or [string]::IsNullOrWhiteSpace($profileVerificationText)) {
    throw 'A28 action profile failed static authentication.'
}
$profileVerification = $profileVerificationText | ConvertFrom-Json
if (-not [bool]$profileVerification.accepted -or
    $profileVerification.navigation_status -cne 'AUTHENTICATED_EXACT_D04_ACTION_LAW') {
    throw 'A28 action profile static verifier did not admit authenticated navigation.'
}
$profileVerificationPath = Join-Path $artifactDir 'a28_action_profile.static_verification.json'
Write-CreateNewUtf8 $profileVerificationPath ($profileVerificationText + "`n")
$profile = Get-Content -LiteralPath $resolvedProfile -Raw | ConvertFrom-Json
if ($profile.schema -cne 'hoi4.a28.air-mio-autonomous-actions.v1' -or -not ($profile.actions -is [array])) {
    throw 'A28 action profile schema/actions are invalid.'
}
if ($profile.authority.executable_sha256 -cne $expectedExeSha256 -or
    $profile.authority.save_sha256 -cne 'c8488b05a3b87c718eeff979976537b79e867634d0b4dccb1eef6c5a45362735' -or
    [int]$profile.authority.initial_game_hour_raw -ne 60830441) {
    throw 'A28 action profile is not bound to the frozen D04 fixture.'
}
if ($profile.navigation_status -cne 'AUTHENTICATED_EXACT_D04_ACTION_LAW') {
    throw 'A28 action profile does not contain an authenticated panel-navigation law.'
}
$markers = @($profile.actions | Where-Object { $_.kind -ceq 'capture_marker' })
$markerFamilies = @($markers | ForEach-Object { [string]$_.family } | Sort-Object -Unique)
if ($markerFamilies.Count -ne 4 -or (Compare-Object $requiredFamilies $markerFamilies)) {
    throw 'A28 action profile must contain all four exact family markers once or more.'
}

# Reuse the authenticated exact-window/SendInput implementation.  Markers are
# bounded dwell operations; they never fabricate a semantic observation.
$childActions = [Collections.Generic.List[object]]::new()
foreach ($action in $profile.actions) {
    if ([string]$action.kind -ceq 'capture_marker') {
        $milliseconds = [int]$action.dwell_milliseconds
        if ($milliseconds -lt 100 -or $milliseconds -gt 60000) { throw 'A28 capture marker dwell is outside 100..60000 ms.' }
        $childActions.Add([ordered]@{label=[string]$action.label;kind='wait';milliseconds=$milliseconds})
    } else {
        $childActions.Add($action)
    }
}
$childProfile = [ordered]@{
    schema = 'hoi4.a36.authenticated-matrix-actions.v1'
    profile_id = "a28-child-$($profile.profile_id)"
    expected_client_width = [int]$profile.expected_client_width
    expected_client_height = [int]$profile.expected_client_height
    actions = $childActions
}
$childProfilePath = Join-Path $artifactDir 'a28_authenticated_actions.child.json'
$childReceiptPath = Join-Path $artifactDir 'a28_authenticated_actions.child.receipt.json'
Write-CreateNewUtf8 $childProfilePath (($childProfile | ConvertTo-Json -Depth 20) + "`n")

$driver = Join-Path $PSScriptRoot 'Invoke-A36AuthenticatedMatrixActions.ps1'
& powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $driver `
    -Hoi4Exe $resolvedExe -CollectorDll $resolvedCollector -ProfilePath $childProfilePath `
    -ReceiptPath $childReceiptPath -ExistingPid $ExistingPid -ProcessTimeoutSeconds $ProcessTimeoutSeconds
if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $childReceiptPath -PathType Leaf)) {
    throw 'Authenticated A28 machine action execution failed.'
}
$childReceipt = Get-Content -LiteralPath $childReceiptPath -Raw | ConvertFrom-Json
if ($childReceipt.schema -cne 'hoi4.a36.authenticated-matrix-action-receipt.v1' -or
    $childReceipt.executable_sha256 -cne $expectedExeSha256 -or
    [uint32]$childReceipt.process_id -ne $ExistingPid) {
    throw 'Authenticated child action receipt does not bind the requested exact process.'
}
$receipt = [ordered]@{
    schema = 'hoi4.a28.air-mio-action-receipt.v1'
    status = 'PASS'
    executable_sha256 = $expectedExeSha256
    collector_sha256 = (Get-FileHash -LiteralPath $resolvedCollector -Algorithm SHA256).Hash.ToLowerInvariant()
    process_id = $ExistingPid
    profile_file = [IO.Path]::GetFileName($resolvedProfile)
    profile_sha256 = (Get-FileHash -LiteralPath $resolvedProfile -Algorithm SHA256).Hash.ToLowerInvariant()
    child_profile_sha256 = (Get-FileHash -LiteralPath $childProfilePath -Algorithm SHA256).Hash.ToLowerInvariant()
    child_receipt_sha256 = (Get-FileHash -LiteralPath $childReceiptPath -Algorithm SHA256).Hash.ToLowerInvariant()
    navigation_status = [string]$profile.navigation_status
    machine_navigation_authenticated = $true
    profile_static_verification_file = [IO.Path]::GetFileName($profileVerificationPath)
    profile_static_verification_sha256 = (Get-FileHash -LiteralPath $profileVerificationPath -Algorithm SHA256).Hash.ToLowerInvariant()
    navigation_authority = $profile.navigation_authority
    panel_transactions = @(
        [ordered]@{panel='air_selection_list';open_actions=@('open_air_overview','open_first_d04_airbase_quick_selection')},
        [ordered]@{panel='mio_aggregated_bonus_smooth_list';open_actions=@('open_country_politics','open_first_d04_materiel_mio_roster','open_first_d04_mio_details')},
        [ordered]@{panel='mio_variant_history_smooth_list';open_actions=@('open_mio_history_tab')},
        [ordered]@{panel='mio_production_history_smooth_list';open_actions=@('open_mio_history_tab')}
    )
    family_markers = @($markers | ForEach-Object { [ordered]@{family=[string]$_.family;label=[string]$_.label} })
    semantic_authority = 'four raw controller observations plus mechanical aggregate regrader only'
    promotion_authorized = $false
}
Write-CreateNewUtf8 $receiptFull (($receipt | ConvertTo-Json -Depth 20) + "`n")
