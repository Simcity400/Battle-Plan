<#
.SYNOPSIS
Offline launch/selection regressions; no game, recorder or input injector is run.
#>
[CmdletBinding()]
param(
    [string]$Driver = '',
    [string]$ScratchRoot = ([IO.Path]::GetTempPath())
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if (-not $Driver) { $Driver = Join-Path $PSScriptRoot 'smoke.ps1' }
$testScript = $PSCommandPath
$tokens = $null; $errors = $null
$driverAst = [Management.Automation.Language.Parser]::ParseFile($Driver, [ref]$tokens, [ref]$errors)
if ($errors.Count) { throw $errors[0] }
foreach ($definition in $driverAst.FindAll({param($node)
    $node -is [Management.Automation.Language.FunctionDefinitionAst] -and
    $node.Name -in @(
        'Get-SubstDriveNames', 'Resolve-LaunchUserDirectory', 'Write-LaunchSelection', 'Copy-ShaderCompiler',
        'Invoke-WithFocusRetry', 'Invoke-SequentialFocusInput', 'Get-UiRegionBox',
        'Test-UiPatchMatch', 'Resolve-UiDialogProbe', 'Resolve-SmokeUiProbes', 'Test-UiDialogState',
        'Reset-UiToCleanMap', 'Invoke-ObservedUiStep', 'Invoke-BoundedUiSequence',
        'Invoke-UiSaveFlow', 'Invoke-UiExitFlow', 'Invoke-ExitThenVerify',
        'Read-SharedUtf8Text', 'Start-SmokeTool', 'Wait-SmokeTool',
        'Get-ActiveSmokeTools', 'Invoke-FailureGamePause', 'Invoke-FailureRecorderCleanup'
    )
}, $false)) { . ([scriptblock]::Create($definition.Extent.Text)) }
try { $ScratchRoot = Resolve-LaunchUserDirectory $ScratchRoot }
catch { throw "ScratchRoot prerequisite failed: supply an existing physical, hyphen-free directory without junctions, symbolic links or subst drives. $($_.Exception.Message)" }

if (-not ('Hoi4ProbeInput' -as [type])) {
    Add-Type 'public static class Hoi4ProbeInput { public static System.IntPtr GetForegroundWindow() { return System.IntPtr.Zero; } }'
}
$fixtureState = @{ Arguments=$null; SelectionPath=''; SelectionAtLaunch=$null; QueriedProcesses=[Collections.Generic.List[string]]::new() }
function Get-Process {
    [CmdletBinding()]
    param([string[]]$Name)
    if ($Name) { foreach ($entry in $Name) { $fixtureState.QueriedProcesses.Add([string]$entry) } }
}
function Start-Process {
    param($FilePath, $WorkingDirectory, $ArgumentList, [switch]$PassThru)
    $fixtureState.Arguments = $ArgumentList
    $fixtureState.SelectionAtLaunch = [IO.File]::ReadAllBytes($fixtureState.SelectionPath)
    throw 'SYNTHETIC_PROCESS_BOUNDARY'
}

$root = Join-Path $ScratchRoot ('smokepreflight' + [Guid]::NewGuid().ToString('N'))
$null = New-Item -ItemType Directory -Path $root
$fixtureProfile = Join-Path $root 'physical profile'
$null = New-Item -ItemType Directory -Path (Join-Path $fixtureProfile 'save games') -Force
$exe = Join-Path $root 'hoi4.exe'
[IO.File]::WriteAllBytes($exe, [byte[]]@())
$savePath = Join-Path $fixtureProfile 'save games\fixture.hoi4'
[IO.File]::WriteAllBytes($savePath, [byte[]]@())
(Get-Item -LiteralPath $savePath).LastWriteTime = [DateTime]::new(2026, 10, 3, 4, 5, 6)
$selectionPath = Join-Path $fixtureProfile 'continue_game.json'
$fixtureState.SelectionPath = $selectionPath
$utf8 = [Text.UTF8Encoding]::new($false, $true)
$expected = '{"title":"fixture.hoi4","desc":"","date":"Sat Oct  3 04:05:06 2026\n","filename":"fixture.hoi4","is_remote":false}'
$script:Passed = 0
$substDrive = $null

function Assert-Bytes([byte[]]$actual, [byte[]]$expectedBytes) {
    if ([Convert]::ToBase64String($actual) -cne [Convert]::ToBase64String($expectedBytes)) { throw 'Selection bytes differ from the expected BOM-free ASCII JSON.' }
}
function Assert-Launch([string]$name, [string]$expectedError = 'SYNTHETIC_PROCESS_BOUNDARY', [string]$userPath = $fixtureProfile, [string]$saveName = 'fixture.hoi4') {
    $fixtureState.Arguments = $null
    $fixtureState.QueriedProcesses.Clear()
    $failure = $null
    try { $null = & $Driver -Action Launch -GameExe $exe -UserDirectory $userPath -ContinueLastSave -ExpectedSave $saveName -Foreground }
    catch { $failure = $_.Exception.Message }
    if (-not $failure -or $failure -notlike "*$expectedError*") { throw "$name expected '$expectedError', got '$failure'." }
    if ($expectedError -eq 'SYNTHETIC_PROCESS_BOUNDARY') {
        Assert-Bytes $fixtureState.SelectionAtLaunch ($utf8.GetBytes($expected.Replace('fixture.hoi4', ($saveName -replace [char]0xE9, '\u00e9'))))
        if ($fixtureState.Arguments -notcontains '--continuelastsave' -or $fixtureState.Arguments -notcontains ('"-userdir=' + $userPath + '"')) { throw 'Launch lost the continuation flag or quoted user directory.' }
        if ($fixtureState.QueriedProcesses -notcontains 'hoi4' -or $fixtureState.QueriedProcesses -contains 'the') { throw "Launch queried wrong process names: $($fixtureState.QueriedProcesses -join ',')" }
    } elseif ($null -ne $fixtureState.Arguments) { throw "$name reached Start-Process." }
    $script:Passed++; Write-Host "PASS $name"
}
function Assert-ScratchRefusal([string]$path) {
    $previous = $ErrorActionPreference; $ErrorActionPreference = 'Continue'
    try { $message = & powershell -NoProfile -ExecutionPolicy Bypass -File $testScript -Driver $Driver -ScratchRoot $path 2>&1 | Out-String; $result = $LASTEXITCODE }
    finally { $ErrorActionPreference = $previous }
    if ($result -eq 0 -or $message -notlike '*ScratchRoot prerequisite failed:*') { throw "Expected named ScratchRoot prerequisite failure, got: $message" }
    $script:Passed++; Write-Host 'PASS named scratch-root prerequisite'
}

$smoke = $driverAst.Find({param($node) $node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq 'Invoke-FullSmoke'}, $false)
$snapshot = $smoke.Body.EndBlock.Statements | Where-Object { $_.Extent.Text -match '^\$selection = ' }
$write = $smoke.Find({param($node) $node -is [Management.Automation.Language.PipelineAst] -and $node.Extent.Text -match '^Write-LaunchSelection '}, $true)
$writtenInit = $smoke.Body.EndBlock.Statements | Where-Object { $_.Extent.Text -eq '$selectionWritten = $false' }
$writtenMark = $smoke.Find({param($node) $node -is [Management.Automation.Language.AssignmentStatementAst] -and $node.Extent.Text -eq '$selectionWritten = $true'}, $true)
$smokeTry = $smoke.Find({param($node) $node -is [Management.Automation.Language.TryStatementAst] -and $node.Finally -and $node.Finally.Extent.Text -match '\$ownedGameStopped='}, $true)
$smokeCatch = $smokeTry.CatchClauses[0].Body
$cleanup = $smokeTry.Finally
function Assert-SmokeRestore([byte[]]$before, [string]$mode = 'normal') {
    $userPath = $fixtureProfile; $ExpectedSave = 'fixture.hoi4'
    . ([scriptblock]::Create($snapshot.Extent.Text))
    . ([scriptblock]::Create($writtenInit.Extent.Text))
    if ($mode -eq 'before_write') { [IO.File]::WriteAllText($selectionPath, 'external selection') }
    else {
        & ([scriptblock]::Create($write.Extent.Text))
        . ([scriptblock]::Create($writtenMark.Extent.Text))
        Assert-Bytes ([IO.File]::ReadAllBytes($selectionPath)) ($utf8.GetBytes($expected))
    }
    $settingsPath = Join-Path $fixtureProfile 'settings.txt'; $settingsBytes = [byte[]]@(1, 2, 3)
    $lock = Join-Path $root 'gamelock'; $ownerToken = 'synthetic'
    $null = New-Item -ItemType Directory -Path $lock
    [IO.File]::WriteAllText((Join-Path $lock 'owner'), $ownerToken)
    $reader = $null; $game = $null; $recorderRun = $null; $report = @{}; $failure = $null
    if ($mode -eq 'before_write') { $failure = [InvalidOperationException]::new('synthetic preparation failure') }
    $started = [Diagnostics.Stopwatch]::StartNew()
    function Write-Report {}
    $heldFile = $null
    if ($mode -eq 'already_missing') { Remove-Item -LiteralPath $selectionPath }
    if ($mode -eq 'delete_failure') { $heldFile = [IO.File]::Open($selectionPath, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read) }
    try { . ([scriptblock]::Create($cleanup.Extent.Text.Trim().Substring(1, $cleanup.Extent.Text.Trim().Length - 2))) }
    finally { if ($heldFile) { $heldFile.Dispose() } }
    if ($mode -eq 'delete_failure') {
        if ($report.cleanup_complete -or $report.passed -or -not $report.cleanup_error -or -not $failure -or -not (Test-Path -LiteralPath $selectionPath) -or -not (Test-Path -LiteralPath (Join-Path $lock 'owner'))) { throw 'Removal failure was not reported or the game lock was removed.' }
        Remove-Item -LiteralPath $selectionPath
        Remove-Item -LiteralPath (Join-Path $lock 'owner')
        Remove-Item -LiteralPath $lock
        $script:Passed++; Write-Host 'PASS selection removal failure reports incomplete cleanup and retains lock'
        return
    }
    if (-not $report.cleanup_complete -or (Test-Path -LiteralPath $lock)) { throw 'Smoke cleanup failed.' }
    if ($mode -eq 'before_write') {
        if ([IO.File]::ReadAllText($selectionPath) -cne 'external selection') { throw 'Preparation failure deleted an unowned selection.' }
        Remove-Item -LiteralPath $selectionPath
    } elseif ($null -eq $before) {
        if (Test-Path -LiteralPath $selectionPath) { throw 'Smoke did not restore the missing selection.' }
    } else { Assert-Bytes ([IO.File]::ReadAllBytes($selectionPath)) $before }
    Assert-Bytes ([IO.File]::ReadAllBytes($settingsPath)) $settingsBytes
    $script:Passed++; Write-Host "PASS Smoke selection cleanup ($mode)"
}

try {
    Assert-Launch 'missing selection is created with exact bytes'
    $bad = [ordered]@{
        'empty selection' = [byte[]]@()
        'UTF-8 BOM' = [byte[]](@(0xEF, 0xBB, 0xBF) + $utf8.GetBytes($expected))
        'UTF-16' = [byte[]](@(0xFF, 0xFE) + [Text.Encoding]::Unicode.GetBytes($expected))
        'single quotes' = $utf8.GetBytes("{'title':'x','desc':'','date':'x','filename':'fixture.hoi4','is_remote':false}")
        'unquoted names' = $utf8.GetBytes('{title:"x",desc:"",date:"x",filename:"fixture.hoi4",is_remote:false}')
        'duplicate field' = $utf8.GetBytes('{"title":null,"title":"x","desc":"","date":"x","filename":"fixture.hoi4","is_remote":false}')
        'missing date' = $utf8.GetBytes('{"title":"x","desc":"","filename":"fixture.hoi4","is_remote":false}')
    }
    foreach ($case in $bad.GetEnumerator()) {
        [IO.File]::WriteAllBytes($selectionPath, $case.Value)
        Assert-Launch ($case.Key + ' replaced before launch')
        [IO.File]::WriteAllBytes($selectionPath, $case.Value)
        Assert-SmokeRestore $case.Value
    }
    Remove-Item -LiteralPath $selectionPath
    Assert-SmokeRestore $null
    Assert-SmokeRestore $null 'before_write'
    Assert-SmokeRestore $null 'delete_failure'
    Assert-SmokeRestore $null 'already_missing'
    $setup = $smoke.Find({param($node) $node -is [Management.Automation.Language.TryStatementAst] -and $node.CatchClauses.Count -and $node.Body.Extent.Text -match "WriteAllText\(\(Join-Path \`$lock 'owner'\)"}, $true)
    if (-not $setup) { throw 'Smoke has no guarded setup after taking the game lock.' }
    $lock = Join-Path $root 'setuplock'; $ownerToken = 'synthetic'; $sourceSave = Join-Path $root 'no such save.hoi4'
    $null = New-Item -ItemType Directory -Path $lock
    $output = Join-Path $root 'setup output'
    $failed = $false
    try { & ([scriptblock]::Create($setup.Extent.Text)) } catch { $failed = $true }
    if (-not $failed -or (Test-Path -LiteralPath $lock)) { throw 'A failed setup kept the game lock.' }
    $script:Passed++; Write-Host 'PASS failed setup releases the game lock'
    $unicodeName = 'caf' + [char]0xE9 + '.hoi4'
    $unicodeSave = Join-Path $fixtureProfile ('save games\' + $unicodeName)
    [IO.File]::WriteAllBytes($unicodeSave, [byte[]]@())
    (Get-Item -LiteralPath $unicodeSave).LastWriteTime = [DateTime]::new(2026, 10, 3, 4, 5, 6)
    $previousCulture = [Threading.Thread]::CurrentThread.CurrentCulture
    try {
        [Threading.Thread]::CurrentThread.CurrentCulture = [Globalization.CultureInfo]::GetCultureInfo('fr-FR')
        Assert-Launch 'ASCII-safe non-ASCII filename and invariant save timestamp' 'SYNTHETIC_PROCESS_BOUNDARY' $fixtureProfile $unicodeName
    } finally { [Threading.Thread]::CurrentThread.CurrentCulture = $previousCulture }
    Assert-Launch 'ExpectedSave is required' 'ContinueLastSave requires ExpectedSave.' $fixtureProfile ''
    Assert-Launch 'missing save' 'ExpectedSave is missing: absent.hoi4' $fixtureProfile 'absent.hoi4'
    Assert-Launch 'save path refused' 'ExpectedSave must be a local .hoi4 save filename.' $fixtureProfile '..\fixture.hoi4'
    $alias = Join-Path $root 'alias'; $null = New-Item -ItemType Junction -Path $alias -Target $fixtureProfile
    Assert-Launch 'junction profile' 'UserDirectory traverses a junction or symbolic link' $alias
    $parentAlias = Join-Path $root 'parentalias'; $null = New-Item -ItemType Junction -Path $parentAlias -Target $root
    Assert-Launch 'junction ancestor' 'UserDirectory traverses a junction or symbolic link' (Join-Path $parentAlias 'physical profile')
    $hyphenated = Join-Path $root 'bad-profile'; $null = New-Item -ItemType Directory -Path $hyphenated
    Assert-Launch 'hyphenated profile' 'entire path contains no hyphen' $hyphenated
    Assert-ScratchRefusal $hyphenated
    Assert-ScratchRefusal $alias
    $used = @([IO.Directory]::GetLogicalDrives() | ForEach-Object { $_.TrimEnd('\') })
    $available = @([char[]]'ZYXWVUTSRQPONMLKJIHGFED' | Where-Object { "${_}:" -notin $used })
    if (-not $available.Count) { throw 'Subst test prerequisite: no unused drive letter.' }
    $substDrive = [string]$available[0] + ':'
    & (Join-Path $env:SystemRoot 'System32\subst.exe') $substDrive $root
    if ($LASTEXITCODE -ne 0) { $substDrive = $null; throw 'Subst test prerequisite: could not create the synthetic drive.' }
    Assert-Launch 'subst profile names the drive' "UserDirectory is on subst drive ${substDrive}." ($substDrive + '\physical profile')
    Assert-ScratchRefusal ($substDrive + '\')
    $built = Join-Path $root 'built'; $staged = Join-Path $root 'staged'
    $null = New-Item -ItemType Directory -Path $built -Force
    $null = New-Item -ItemType Directory -Path $staged
    [IO.File]::WriteAllText((Join-Path $built 'dxcompiler.dll'), 'dxcompiler.dll')
    $refused = $null
    try { $null = Copy-ShaderCompiler $built $staged } catch { $refused = $_.Exception.Message }
    if ($refused -notlike '*needs the pinned shader compiler*' -or (Test-Path -LiteralPath (Join-Path $staged 'dxcompiler.dll'))) { throw "A build without dxil.dll was not refused: $refused" }
    [IO.File]::WriteAllText((Join-Path $built 'dxil.dll'), 'dxil.dll')
    $compiler = Copy-ShaderCompiler $built $staged
    if ($compiler -cne (Join-Path $staged 'dxcompiler.dll')) { throw "Staged compiler path: $compiler" }
    foreach ($name in @('dxcompiler.dll', 'dxil.dll')) {
        if ([IO.File]::ReadAllText((Join-Path $staged $name)) -cne $name) { throw "The staged compiler lacks $name." }
    }
    if (Test-Path -LiteralPath (Join-Path $staged 'shader-tools')) { throw 'The staged viewer took a shader-tools folder.' }
    $script:Passed++; Write-Host 'PASS the staged viewer takes the shader compiler along'
    . (Join-Path (Split-Path -Parent $Driver) 'game-clock.ps1')
    $withheld = '{"paused":"command_inputs::IDLER_RVA withheld: unresolved"}' | ConvertFrom-Json
    $bare = ConvertTo-GameClock ('{"game_state":4096,"hour":1936,"play_time":77,"seed":-5,"speed":3,"tick_busy":0,"play_session_started_at":1778888888}' | ConvertFrom-Json) $withheld
    if ($null -ne $bare.paused -or $bare.presentation_valid -or $null -ne $bare.eye -or $null -ne $bare.viewport -or $bare.hour -ne 1936 -or $bare.seed -ne -5) { throw 'A clock without its presentation was not left absent.' }
    $named = @(Get-ClockWithheld ([pscustomobject]@{ Withheld = $withheld }))
    if ($named.Count -ne 1 -or $named[0] -cne 'paused: command_inputs::IDLER_RVA withheld: unresolved') { throw 'A withheld clock field was not named.' }
    $full = ConvertTo-GameClock ('{"game_state":4096,"hour":1936,"play_time":77,"seed":-5,"speed":3,"tick_busy":0,"play_session_started_at":1778888888,"paused":false,"eye":[1500.5,320.0,900.25],"look_at":[1500.5,0.0,1010.75],"viewport":[0,0,2560,1440]}' | ConvertFrom-Json) $null
    if ($full.paused -ne 0 -or $null -eq $full.paused -or -not $full.presentation_valid -or $full.eye[2] -ne 900.25 -or $full.viewport[3] -ne 1440) { throw 'A full clock line was not read.' }
    if ($full.Same($bare) -or -not $full.Same($full) -or $null -ne (ConvertTo-GameClock $null $null)) { throw 'Clock agreement or an unreadable clock was misread.' }
    $script:Passed++; Write-Host 'PASS clock lines keep withheld fields absent and named'

    $focusCalls = 0; $focusPauses = 0
    $focusResult = Invoke-WithFocusRetry -MaxAttempts 4 -OnRetry { $script:focusPauses++ } -Operation {
        $script:focusCalls++
        if ($script:focusCalls -lt 3) { throw 'Windows refused to focus the game; input was not sent.' }
        'focused'
    }
    if ($focusResult -cne 'focused' -or $focusCalls -ne 3 -or $focusPauses -ne 2) { throw "Focus retry recovered unexpectedly: result=$focusResult calls=$focusCalls pauses=$focusPauses" }
    $lostCalls = 0
    $lostResult = Invoke-WithFocusRetry -MaxAttempts 3 -OnRetry {} -Operation {
        $script:lostCalls++
        if ($script:lostCalls -eq 1) { throw 'Game focus was lost; input was not sent.' }
        'recovered'
    }
    if ($lostResult -cne 'recovered' -or $lostCalls -ne 2) { throw "Focus-lost retry failed: result=$lostResult calls=$lostCalls" }
    $partialCalls = 0; $partialError = $null
    try {
        $null = Invoke-WithFocusRetry -MaxAttempts 4 -OnRetry {} -Operation {
            $script:partialCalls++
            throw 'Game focus was lost after delivering partial input (1 of 4).'
        }
    } catch { $partialError = $_.Exception.Message }
    if ($partialCalls -ne 1 -or $partialError -notlike '*partial input*') { throw "Partial-input focus loss was retried: calls=$partialCalls error=$partialError" }
    $exhaustedCalls = 0; $exhaustedError = $null
    try {
        $null = Invoke-WithFocusRetry -MaxAttempts 3 -OnRetry {} -Operation {
            $script:exhaustedCalls++
            throw 'Windows refused to focus the game; input was not sent.'
        }
    } catch { $exhaustedError = $_.Exception.Message }
    if ($exhaustedCalls -ne 3 -or $exhaustedError -notlike '*Windows refused to focus the game*') { throw "Bounded focus retry did not stop at limit: calls=$exhaustedCalls error=$exhaustedError" }
    $nonFocusCalls = 0; $nonFocusError = $null
    try {
        $null = Invoke-WithFocusRetry -MaxAttempts 4 -OnRetry {} -Operation {
            $script:nonFocusCalls++
            throw 'The game window no longer belongs to the selected process.'
        }
    } catch { $nonFocusError = $_.Exception.Message }
    if ($nonFocusCalls -ne 1 -or $nonFocusError -notlike '*no longer belongs*') { throw "Non-focus error was retried: calls=$nonFocusCalls error=$nonFocusError" }

    $driveAst = $smoke.Find({param($node) $node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq 'Drive'}, $true)
    if (-not $driveAst) { throw 'Drive function not found in Invoke-FullSmoke.' }
    . ([scriptblock]::Create($driveAst.Extent.Text))
    $stubDriverLog = Join-Path $root 'stub-driver.jsonl'
    $stubDriverPath = Join-Path $root 'stub-driver.ps1'
    $stubDriverBody = @'
param(
    [string]$CaptureRoot,
    [string]$InputShimDir,
    [string]$Action,
    [int]$GameId,
    [string]$GameExe,
    [string]$UserDirectory,
    [switch]$Foreground,
    [switch]$KeepForeground,
    [string]$Out,
    [string]$Text
)
$logPath = Join-Path (Split-Path -Parent $PSCommandPath) 'stub-driver.jsonl'
$attempt = if (Test-Path -LiteralPath $logPath) { @([IO.File]::ReadAllLines($logPath)).Count + 1 } else { 1 }
[IO.File]::AppendAllText($logPath, (([ordered]@{ attempt = $attempt; action = $Action; out = $Out; text = $Text } | ConvertTo-Json -Compress) + "`n"))
if ($Action -ceq 'Screenshot') {
    if ($attempt -eq 1) { throw 'Windows refused to focus the game; input was not sent.' }
    return [pscustomobject]@{ Action = $Action; Path = $Out; Width = 1920; Height = 1080 }
}
if ($Action -ceq 'Text' -and $Text -ceq 'pre-event-loss') {
    if ($attempt -eq 3) { throw 'Game focus was lost; input was not sent.' }
    return [pscustomobject]@{ Action = $Action; Text = $Text }
}
if ($Action -ceq 'Text' -and $Text -ceq 'mid-event-loss') {
    throw 'Game focus was lost after delivering partial input (1 of 3).'
}
throw "Unexpected action received by driver: $Action"
'@
    [IO.File]::WriteAllText($stubDriverPath, $stubDriverBody, $utf8)
    $savedDriver = $Driver
    try {
        $driver = $stubDriverPath
        $CaptureRoot = $root; $InputShimDir = ''; $game = [pscustomobject]@{ Id = 4321 }; $gamePath = $exe; $userPath = $fixtureProfile; $Foreground = $false
        $driveOut = Join-Path $root 'drive-shot.png'
        $driveReply = Drive 'Screenshot' @{ Out = $driveOut } $false {}
        $textPreLossReply = Drive 'Text' @{ Text = 'pre-event-loss' } $true {}
        $midLossErr = $null
        try { $null = Drive 'Text' @{ Text = 'mid-event-loss' } $true {} } catch { $midLossErr = $_.Exception.Message }
    } finally {
        $Driver = $savedDriver
    }
    $driveEntries = @([IO.File]::ReadAllLines($stubDriverLog) | ForEach-Object { $_ | ConvertFrom-Json })
    if ($driveReply.Action -cne 'Screenshot' -or $driveEntries.Count -ne 5 -or
        $driveEntries[0].action -cne 'Screenshot' -or $driveEntries[1].action -cne 'Screenshot' -or
        $textPreLossReply.Text -cne 'pre-event-loss' -or $driveEntries[2].action -cne 'Text' -or $driveEntries[3].action -cne 'Text' -or
        $driveEntries[4].text -cne 'mid-event-loss' -or $midLossErr -notlike '*partial input*') {
        throw "Drive did not preserve Action or pre/mid-input focus semantics: $($driveEntries | ConvertTo-Json -Compress) err=$midLossErr"
    }
    $script:Passed++; Write-Host 'PASS focus-borrow retries preserve Drive Action, retry pre-input loss, and refuse partial replay'

    $delivered = [Collections.Generic.List[string]]::new()
    $reacquired = 0
    $failedOnce = $false
    Invoke-SequentialFocusInput -Items @('n1', 'n2', 'n3', 'n4') -MaxAttemptsPerItem 3 -Pause {} -Reacquire { $script:reacquired++ } -SendItem {
        param($item)
        if ($item -eq 'n3' -and -not $script:failedOnce) {
            $script:failedOnce = $true
            throw 'Game focus was lost; input was not sent.'
        }
        $delivered.Add([string]$item)
    }
    if (($delivered -join ',') -cne 'n1,n2,n3,n4' -or $reacquired -ne 1) {
        throw "Per-item focus retry duplicated or dropped input: delivered=$($delivered -join ',') reacquired=$reacquired"
    }
    $dupDelivered = [Collections.Generic.List[string]]::new()
    $outerAttempts = 0; $partialErr = $null
    try {
        $null = Invoke-WithFocusRetry -MaxAttempts 3 -OnRetry {} -Operation {
            $script:outerAttempts++
            Invoke-SequentialFocusInput -Items @('a', 'b', 'c') -MaxAttemptsPerItem 2 -Pause {} -Reacquire {} -SendItem {
                param($item)
                if ($item -eq 'b') { throw 'Game focus was lost; input was not sent.' }
                $dupDelivered.Add([string]$item)
            }
        }
    } catch { $partialErr = $_.Exception.Message }
    if ($outerAttempts -ne 1 -or ($dupDelivered -join ',') -cne 'a' -or $partialErr -notlike '*partial input*') {
        throw "Partial sequential input was replayed by outer focus retry: attempts=$outerAttempts delivered=$($dupDelivered -join ',') err=$partialErr"
    }
    $script:Passed++; Write-Host 'PASS sequential wheel and text focus retries never duplicate delivered input'

    Add-Type -AssemblyName System.Drawing
    $imgDir = Join-Path $root 'uiimg'
    $null = New-Item -ItemType Directory -Path $imgDir
    function New-SolidPng([string]$path, [int]$w, [int]$h, [Drawing.Color]$bg, [scriptblock]$paint = {}) {
        $bmp = [Drawing.Bitmap]::new($w, $h)
        try {
            $g = [Drawing.Graphics]::FromImage($bmp)
            try {
                $g.Clear($bg)
                & $paint $bmp $g
            } finally { $g.Dispose() }
            $bmp.Save($path, [Drawing.Imaging.ImageFormat]::Png)
        } finally { $bmp.Dispose() }
    }
    $cleanMapImg = Join-Path $imgDir 'clean-map.png'
    $noisyMapImg = Join-Path $imgDir 'noisy-map.png'
    $menuOpenImg = Join-Path $imgDir 'menu-open.png'
    $confirmOpenImg = Join-Path $imgDir 'confirm-open.png'
    $menuPatchImg = Join-Path $imgDir 'menu-patch.png'
    $saveDialogPatchImg = Join-Path $imgDir 'save-dialog-patch.png'
    $confirmPatchImg = Join-Path $imgDir 'confirm-patch.png'
    New-SolidPng $cleanMapImg 320 200 ([Drawing.Color]::FromArgb(40, 70, 110))
    New-SolidPng $noisyMapImg 320 200 ([Drawing.Color]::FromArgb(40, 70, 110)) {
        param($bmp, $g)
        for ($py = 50; $py -lt 62; $py++) {
            for ($px = 60; $px -lt 75; $px++) {
                $bmp.SetPixel($px, $py, [Drawing.Color]::FromArgb(240, 220, 40))
            }
        }
    }
    New-SolidPng $menuOpenImg 320 200 ([Drawing.Color]::FromArgb(40, 70, 110)) {
        param($bmp, $g)
        $brush = [Drawing.SolidBrush]::new([Drawing.Color]::FromArgb(200, 190, 165))
        try { $g.FillRectangle($brush, 40, 30, 220, 140) } finally { $brush.Dispose() }
    }
    New-SolidPng $confirmOpenImg 320 200 ([Drawing.Color]::FromArgb(40, 70, 110)) {
        param($bmp, $g)
        $brush = [Drawing.SolidBrush]::new([Drawing.Color]::FromArgb(25, 30, 45))
        try { $g.FillRectangle($brush, 60, 40, 200, 120) } finally { $brush.Dispose() }
    }
    New-SolidPng $menuPatchImg 80 40 ([Drawing.Color]::FromArgb(200, 190, 165))
    New-SolidPng $saveDialogPatchImg 80 40 ([Drawing.Color]::FromArgb(145, 160, 180))
    New-SolidPng $confirmPatchImg 80 40 ([Drawing.Color]::FromArgb(25, 30, 45))

    $coordOnlyUi = [pscustomobject]@{ width = 320; height = 200; save_menu = @(150, 100); save_name = @(150, 100); exit_confirm = @(150, 100) }
    $coordOnlyRefused = $null
    try { $null = Resolve-SmokeUiProbes -Ui $coordOnlyUi } catch { $coordOnlyRefused = $_.Exception.Message }
    if (-not $coordOnlyRefused -or $coordOnlyRefused -notlike '*calibrated pause-menu reference image*') {
        throw "Coordinate-only UiConfig was not refused by name: $coordOnlyRefused"
    }

    $patchedUi = [pscustomobject]@{
        width = 320; height = 200
        menu = @(10, 10); speed_up = @(20, 10); speed_down = @(30, 10); pause = @(40, 10)
        save_menu = @(150, 100); save_name = @(150, 100); save_confirm = @(160, 120); save_back = @(120, 120)
        menu_close = @(150, 150); exit_game = @(150, 130); exit_confirm = @(150, 100)
        map_center = @(160, 100); capture_cursor = @(200, 20)
        menu_patch = $menuPatchImg; menu_patch_origin = @(110, 80)
        save_dialog_patch = $saveDialogPatchImg; save_dialog_patch_origin = @(110, 80)
        exit_confirm_patch = $confirmPatchImg; exit_confirm_patch_origin = @(110, 80)
    }
    $resolvedProbes = Resolve-SmokeUiProbes -Ui $patchedUi
    if (Test-UiDialogState -ShotPath $noisyMapImg -Probe $resolvedProbes.Menu) {
        throw 'Calibrated menu patch falsely matched closed noisy map.'
    }
    if (-not (Test-UiDialogState -ShotPath $menuOpenImg -Probe $resolvedProbes.Menu)) {
        throw 'Calibrated menu patch did not match open menu.'
    }

    $badPatchUiPath = Join-Path $root 'bad-patch-ui.json'
    $badPatchUi = [ordered]@{
        width = 320; height = 200; expected_hour = 1; expected_seed = 1
        menu = @(10, 10); speed_up = @(20, 10); speed_down = @(30, 10); pause = @(40, 10)
        save_menu = @(150, 100); save_name = @(150, 100); save_confirm = @(160, 120); save_back = @(120, 120)
        menu_close = @(150, 150); exit_game = @(150, 130); exit_confirm = @(150, 100)
        map_center = @(160, 100); capture_cursor = @(200, 20)
        menu_patch = (Join-Path $root 'missing-menu.png'); menu_patch_origin = @(110, 80)
        save_dialog_patch = $saveDialogPatchImg; save_dialog_patch_origin = @(110, 80)
        exit_confirm_patch = $confirmPatchImg; exit_confirm_patch_origin = @(110, 80)
    }
    [IO.File]::WriteAllText($badPatchUiPath, ($badPatchUi | ConvertTo-Json -Depth 4), $utf8)
    [IO.File]::WriteAllText((Join-Path $fixtureProfile 'settings.txt'), "save_as_binary=no`n", $utf8)
    $badPatchLock = Join-Path $root 'badpatchlock'
    $badPatchReceipt = Join-Path $root 'bad-patch-receipt.json'
    [IO.File]::WriteAllText($badPatchReceipt, '{}', $utf8)
    $badPatchErr = $null
    $fixtureState.QueriedProcesses.Clear()
    try {
        $null = & $Driver -Action Smoke -GameExe $exe -UserDirectory $fixtureProfile -ExpectedSave 'fixture.hoi4' `
            -UiConfig $badPatchUiPath -BuildReceipt $badPatchReceipt -CaptureRoot $root -Out (Join-Path $root 'badpatchout') -GameLock $badPatchLock
    } catch { $badPatchErr = $_.Exception.Message }
    if (-not $badPatchErr -or $badPatchErr -notlike '*calibrated pause-menu reference image*' -or (Test-Path -LiteralPath $badPatchLock)) {
        throw "Invalid UI patch stranded shared game lock or failed unexpectedly: err=$badPatchErr lockExists=$(Test-Path -LiteralPath $badPatchLock)"
    }
    if ($fixtureState.QueriedProcesses -notcontains 'hoi4' -or $fixtureState.QueriedProcesses -notcontains 'recorder' -or $fixtureState.QueriedProcesses -contains 'the') {
        throw "Smoke preflight queried wrong process names: $($fixtureState.QueriedProcesses -join ',')"
    }

    $nonImagePatch = Join-Path $imgDir 'not-an-image.txt'
    [IO.File]::WriteAllText($nonImagePatch, 'not a png image', $utf8)
    $nonImageUiPath = Join-Path $root 'non-image-ui.json'
    $nonImageUi = [ordered]@{
        width = 320; height = 200; expected_hour = 1; expected_seed = 1
        menu = @(10, 10); speed_up = @(20, 10); speed_down = @(30, 10); pause = @(40, 10)
        save_menu = @(150, 100); save_name = @(150, 100); save_confirm = @(160, 120); save_back = @(120, 120)
        menu_close = @(150, 150); exit_game = @(150, 130); exit_confirm = @(150, 100)
        map_center = @(160, 100); capture_cursor = @(200, 20)
        menu_patch = $nonImagePatch; menu_patch_origin = @(110, 80)
        save_dialog_patch = $saveDialogPatchImg; save_dialog_patch_origin = @(110, 80)
        exit_confirm_patch = $confirmPatchImg; exit_confirm_patch_origin = @(110, 80)
    }
    [IO.File]::WriteAllText($nonImageUiPath, ($nonImageUi | ConvertTo-Json -Depth 4), $utf8)
    $nonImageLock = Join-Path $root 'nonimagelock'
    $nonImageErr = $null
    try {
        $null = & $Driver -Action Smoke -GameExe $exe -UserDirectory $fixtureProfile -ExpectedSave 'fixture.hoi4' `
            -UiConfig $nonImageUiPath -BuildReceipt $badPatchReceipt -CaptureRoot $root -Out (Join-Path $root 'nonimageout') -GameLock $nonImageLock
    } catch { $nonImageErr = $_.Exception.Message }
    if (-not $nonImageErr -or $nonImageErr -notlike '*readable calibrated pause-menu reference image*' -or (Test-Path -LiteralPath $nonImageLock)) {
        throw "Non-image patch was not refused before lock: err=$nonImageErr lockExists=$(Test-Path -LiteralPath $nonImageLock)"
    }

    $oobUiPath = Join-Path $root 'oob-patch-ui.json'
    $oobUi = [ordered]@{
        width = 320; height = 200; expected_hour = 1; expected_seed = 1
        menu = @(10, 10); speed_up = @(20, 10); speed_down = @(30, 10); pause = @(40, 10)
        save_menu = @(150, 100); save_name = @(150, 100); save_confirm = @(160, 120); save_back = @(120, 120)
        menu_close = @(150, 150); exit_game = @(150, 130); exit_confirm = @(150, 100)
        map_center = @(160, 100); capture_cursor = @(200, 20)
        menu_patch = $menuPatchImg; menu_patch_origin = @(250, 80)
        save_dialog_patch = $saveDialogPatchImg; save_dialog_patch_origin = @(110, 80)
        exit_confirm_patch = $confirmPatchImg; exit_confirm_patch_origin = @(110, 80)
    }
    [IO.File]::WriteAllText($oobUiPath, ($oobUi | ConvertTo-Json -Depth 4), $utf8)
    $oobLock = Join-Path $root 'ooblock'
    $oobErr = $null
    try {
        $null = & $Driver -Action Smoke -GameExe $exe -UserDirectory $fixtureProfile -ExpectedSave 'fixture.hoi4' `
            -UiConfig $oobUiPath -BuildReceipt $badPatchReceipt -CaptureRoot $root -Out (Join-Path $root 'oobout') -GameLock $oobLock
    } catch { $oobErr = $_.Exception.Message }
    if (-not $oobErr -or $oobErr -notlike '*calibrated window bounds*' -or (Test-Path -LiteralPath $oobLock)) {
        throw "Out-of-bounds patch was not refused before lock: err=$oobErr lockExists=$(Test-Path -LiteralPath $oobLock)"
    }

    $verticalOobUi = [pscustomobject]@{
        width = 320; height = 200
        menu_patch = $menuPatchImg; menu_patch_origin = @(110, 170)
        save_dialog_patch = $saveDialogPatchImg; save_dialog_patch_origin = @(110, 80)
        exit_confirm_patch = $confirmPatchImg; exit_confirm_patch_origin = @(110, 80)
    }
    $verticalOobErr = $null
    try { $null = Resolve-SmokeUiProbes -Ui $verticalOobUi } catch { $verticalOobErr = $_.Exception.Message }
    if (-not $verticalOobErr -or $verticalOobErr -notlike '*calibrated window bounds*') {
        throw "Vertically out-of-bounds patch was not refused: err=$verticalOobErr"
    }

    $exactFitUi = [pscustomobject]@{
        width = 320; height = 200
        menu_patch = $menuPatchImg; menu_patch_origin = @(240, 160)
        save_dialog_patch = $saveDialogPatchImg; save_dialog_patch_origin = @(110, 80)
        exit_confirm_patch = $confirmPatchImg; exit_confirm_patch_origin = @(110, 80)
    }
    $exactFitProbes = Resolve-SmokeUiProbes -Ui $exactFitUi
    if (($exactFitProbes.Menu.Origin -join ',') -cne '240,160') {
        throw "Exact-fit patch origin mismatch: $($exactFitProbes.Menu.Origin -join ',')"
    }

    $missingOriginUiPath = Join-Path $root 'missing-origin-ui.json'
    $missingOriginUi = [ordered]@{
        width = 2560; height = 1440; expected_hour = 1; expected_seed = 1
        menu = @(10, 10); speed_up = @(20, 10); speed_down = @(30, 10); pause = @(40, 10)
        save_menu = @(150, 100); save_name = @(150, 100); save_confirm = @(160, 120); save_back = @(120, 120)
        menu_close = @(150, 150); exit_game = @(150, 130); exit_confirm = @(150, 100)
        map_center = @(160, 100); capture_cursor = @(200, 20)
        menu_patch = $menuPatchImg
        save_dialog_patch = $saveDialogPatchImg; save_dialog_patch_origin = @(110, 80)
        exit_confirm_patch = $confirmPatchImg; exit_confirm_patch_origin = @(110, 80)
    }
    [IO.File]::WriteAllText($missingOriginUiPath, ($missingOriginUi | ConvertTo-Json -Depth 4), $utf8)
    $missingOriginLock = Join-Path $root 'missingoriginlock'
    $missingOriginErr = $null
    try {
        $null = & $Driver -Action Smoke -GameExe $exe -UserDirectory $fixtureProfile -ExpectedSave 'fixture.hoi4' `
            -UiConfig $missingOriginUiPath -BuildReceipt $badPatchReceipt -CaptureRoot $root -Out (Join-Path $root 'missingoriginout') -GameLock $missingOriginLock
    } catch { $missingOriginErr = $_.Exception.Message }
    if (-not $missingOriginErr -or $missingOriginErr -notlike '*calibrated pause-menu reference image and origin*' -or (Test-Path -LiteralPath $missingOriginLock)) {
        throw "JSON menu_patch without menu_patch_origin inherited default MenuPatchOrigin or acquired lock: err=$missingOriginErr lockExists=$(Test-Path -LiteralPath $missingOriginLock)"
    }
    $getProcessCalls = @($driverAst.FindAll({
        param($node)
        $node -is [Management.Automation.Language.CommandAst] -and $node.GetCommandName() -ieq 'Get-Process'
    }, $true))
    if ($getProcessCalls.Count -lt 3) { throw "Expected Get-Process calls in smoke.ps1, found $($getProcessCalls.Count)." }
    foreach ($call in $getProcessCalls) {
        $hasBoundTarget = $false
        for ($i = 1; $i -lt $call.CommandElements.Count; $i++) {
            $el = $call.CommandElements[$i]
            if ($el -is [Management.Automation.Language.CommandParameterAst]) {
                if ($el.ParameterName -in @('Name', 'Id')) { $hasBoundTarget = $true }
                $i++
            } else {
                throw "Get-Process call has unexpected positional argument '$($el.Extent.Text)': $($call.Extent.Text)"
            }
        }
        if (-not $hasBoundTarget) { throw "Get-Process call lacks -Name or -Id: $($call.Extent.Text)" }
    }
    $script:Passed++; Write-Host 'PASS Get-Process calls bind named targets without stray positional arguments'
    $script:Passed++; Write-Host 'PASS missing, unreadable, out-of-bounds, and origin-less UI patches are refused before acquiring the shared game lock'

    $noiseClicks = [Collections.Generic.List[string]]::new()
    $noiseEscapes = 0
    $noiseExitErr = $null
    try {
        $null = Invoke-UiExitFlow -MaxAttempts 2 -MaxPolls 2 -Pause {} `
            -HasExited { $false } -GetExitCode { 0 } -WaitForExit { param($ms) $false } `
            -PauseGame {} -Click { param($name) $noiseClicks.Add([string]$name) } `
            -SendEscape { $script:noiseEscapes++ } `
            -IsMenuOpen { Test-UiDialogState -ShotPath $noisyMapImg -Probe $resolvedProbes.Menu } `
            -IsConfirmOpen { Test-UiDialogState -ShotPath $noisyMapImg -Probe $resolvedProbes.ExitConfirm }
    } catch { $noiseExitErr = $_.Exception.Message }
    if ($noiseClicks -contains 'exit_game' -or $noiseClicks -contains 'exit_confirm' -or $noiseEscapes -ne 0 -or $noiseExitErr -notlike '*did not exit through its confirmation dialog*') {
        throw "Closed menu with unrelated pixel noise sent follow-up click or blind Escape: clicks=$($noiseClicks -join ',') escapes=$noiseEscapes err=$noiseExitErr"
    }

    $alreadyOpenShot = @{ Path = $menuOpenImg }
    $alreadyOpenClicks = [Collections.Generic.List[string]]::new()
    $alreadyOpenEscapes = 0
    $alreadyOpenErr = $null
    try {
        $null = Invoke-UiExitFlow -MaxAttempts 2 -MaxPolls 2 -Pause {} `
            -HasExited { $false } -GetExitCode { 0 } -WaitForExit { param($ms) $false } `
            -PauseGame {} -Click {
                param($name)
                $alreadyOpenClicks.Add([string]$name)
                if ($name -eq 'menu') { $alreadyOpenShot.Path = if ($alreadyOpenShot.Path -eq $menuOpenImg) { $cleanMapImg } else { $menuOpenImg } }
            } -SendEscape { $script:alreadyOpenEscapes++ } `
            -IsMenuOpen { Test-UiDialogState -ShotPath $alreadyOpenShot.Path -Probe $resolvedProbes.Menu } `
            -IsConfirmOpen { Test-UiDialogState -ShotPath $alreadyOpenShot.Path -Probe $resolvedProbes.ExitConfirm }
    } catch { $alreadyOpenErr = $_.Exception.Message }
    if ($alreadyOpenEscapes -lt 1 -or $alreadyOpenClicks.Count -ne 0 -or $alreadyOpenErr -notlike '*starting UI state was not the clean map*') {
        throw "Already-open pause menu was mistaken for clean map or sent confirmation clicks: escapes=$alreadyOpenEscapes clicks=$($alreadyOpenClicks -join ',') err=$alreadyOpenErr"
    }

    $alreadyOpenRecoversShot = @{ Path = $menuOpenImg }
    $alreadyOpenRecoversClicks = [Collections.Generic.List[string]]::new()
    $alreadyOpenRecoversEscapes = 0
    $alreadyOpenRecovers = Invoke-UiExitFlow -MaxAttempts 2 -MaxPolls 2 -Pause {} `
        -HasExited { $false } -GetExitCode { 0 } -WaitForExit { param($ms) $true } `
        -PauseGame {} -Click {
            param($name)
            $alreadyOpenRecoversClicks.Add([string]$name)
            if ($name -eq 'menu') { $alreadyOpenRecoversShot.Path = if ($alreadyOpenRecoversShot.Path -eq $menuOpenImg) { $cleanMapImg } else { $menuOpenImg } }
            elseif ($name -eq 'exit_game') { $alreadyOpenRecoversShot.Path = $confirmOpenImg }
        } -SendEscape {
            $script:alreadyOpenRecoversEscapes++
            $alreadyOpenRecoversShot.Path = $cleanMapImg
        } -IsMenuOpen { Test-UiDialogState -ShotPath $alreadyOpenRecoversShot.Path -Probe $resolvedProbes.Menu } `
        -IsConfirmOpen { Test-UiDialogState -ShotPath $alreadyOpenRecoversShot.Path -Probe $resolvedProbes.ExitConfirm }
    if (-not $alreadyOpenRecovers.Done -or $alreadyOpenRecoversEscapes -ne 1 -or ($alreadyOpenRecoversClicks -join ',') -cne 'menu,exit_game,exit_confirm') {
        throw "Already-open pause menu recovery failed: escapes=$alreadyOpenRecoversEscapes clicks=$($alreadyOpenRecoversClicks -join ',')"
    }

    $noiseSaveClicks = [Collections.Generic.List[string]]::new()
    $noiseSaveEscapes = 0
    $noiseSaveErr = $null
    try {
        $null = Invoke-UiSaveFlow -MaxAttempts 2 -MaxPolls 2 -Pause {} `
            -IsSaveComplete { $false } -CheckClock {} -PauseGame {} `
            -Click { param($name) $noiseSaveClicks.Add([string]$name) } `
            -TypeSaveName {} -SendEscape { $script:noiseSaveEscapes++ } `
            -IsMenuOpen { Test-UiDialogState -ShotPath $noisyMapImg -Probe $resolvedProbes.Menu } `
            -IsSaveDialogOpen { Test-UiDialogState -ShotPath $noisyMapImg -Probe $resolvedProbes.SaveDialog } `
            -WaitForSave { [pscustomobject]@{ Done = $false; Reason = 'not saved' } }
    } catch { $noiseSaveErr = $_.Exception.Message }
    if ($noiseSaveClicks -contains 'save_menu' -or $noiseSaveClicks -contains 'save_confirm' -or $noiseSaveEscapes -ne 0 -or $noiseSaveErr -notlike '*did not produce a complete text save*') {
        throw "Closed menu with unrelated pixel noise sent save follow-up click or blind Escape: clicks=$($noiseSaveClicks -join ',') escapes=$noiseSaveEscapes err=$noiseSaveErr"
    }

    $openDepth = 1
    $resetEscapes = 0
    $cleanedState = Reset-UiToCleanMap -MaxEscapes 3 -Pause {} -IsAnyDialogOpen { $script:openDepth -gt 0 } -SendEscape { $script:resetEscapes++; $script:openDepth-- }
    if (-not $cleanedState -or $resetEscapes -ne 1) {
        throw "Reset-UiToCleanMap sent wrong Escape count: cleaned=$cleanedState escapes=$resetEscapes"
    }
    $script:Passed++; Write-Host 'PASS calibrated dialog probes reject noise, identify already-open menus, and reset cleanly'

    $preConfirmClicks = [Collections.Generic.List[string]]::new()
    $preConfirmExited = $false
    $preConfirmMenuOpen = $false
    $preConfirmReport = [ordered]@{}
    $preConfirmVerified = $false
    $preConfirmErr = $null
    try {
        Invoke-ExitThenVerify -Report $preConfirmReport -ExitGame {
            $null = Invoke-UiExitFlow -MaxAttempts 3 -MaxPolls 2 -Pause {} `
                -HasExited { $script:preConfirmExited } -GetExitCode { 0 } -WaitForExit { param($ms) $true } `
                -PauseGame {} -Click {
                    param($name)
                    $preConfirmClicks.Add([string]$name)
                    if ($name -eq 'menu') { $script:preConfirmMenuOpen = $true; $script:preConfirmExited = $true }
                } -SendEscape {} -IsMenuOpen { $script:preConfirmMenuOpen } -IsConfirmOpen { $false }
        } -Verify {
            $script:preConfirmVerified = $true
            $preConfirmReport.oracle_differences = 0
        }
    } catch { $preConfirmErr = $_.Exception.Message }
    if ($preConfirmReport.normal_game_exit -ne $false -or -not $preConfirmVerified -or $preConfirmReport.oracle_differences -ne 0 -or
        ($preConfirmClicks -contains 'exit_confirm') -or $preConfirmErr -notlike '*before exit confirmation*') {
        throw "Pre-confirmation exit falsely succeeded or skipped verify: exit=$($preConfirmReport.normal_game_exit) verified=$preConfirmVerified clicks=$($preConfirmClicks -join ',') err=$preConfirmErr"
    }

    $crashReport = [ordered]@{}
    $crashVerified = $false
    $crashErr = $null
    $crashAttempts = 0
    $crashUiState = @{ Menu = $false; Confirm = $false }
    try {
        Invoke-ExitThenVerify -Report $crashReport -ExitGame {
            $null = Invoke-UiExitFlow -MaxAttempts 3 -MaxPolls 2 -Pause {} `
                -HasExited { $false } -GetExitCode { -1073741819 } -WaitForExit { param($ms) $true } `
                -PauseGame {} -Click {
                    param($name)
                    if ($name -eq 'menu') { $crashUiState.Menu = $true }
                    elseif ($name -eq 'exit_game') { $crashUiState.Confirm = $true }
                    elseif ($name -eq 'exit_confirm') { $script:crashAttempts++ }
                } -SendEscape { $crashUiState.Menu = $false; $crashUiState.Confirm = $false } `
                -IsMenuOpen { $crashUiState.Menu } -IsConfirmOpen { $crashUiState.Confirm }
        } -Verify {
            $script:crashVerified = $true
            $crashReport.oracle_differences = 0
        }
    } catch { $crashErr = $_.Exception.Message }
    if ($crashReport.normal_game_exit -ne $false -or -not $crashVerified -or $crashAttempts -ne 1 -or $crashErr -notlike '*-1073741819*') {
        throw "Non-zero exit code after confirmation was accepted or skipped verify: exit=$($crashReport.normal_game_exit) verified=$crashVerified attempts=$crashAttempts err=$crashErr"
    }

    $cleanExitReport = [ordered]@{}
    $cleanExitResult = $null
    $cleanUiState = @{ Menu = $false; Confirm = $false }
    Invoke-ExitThenVerify -Report $cleanExitReport -ExitGame {
        $script:cleanExitResult = Invoke-UiExitFlow -MaxAttempts 3 -MaxPolls 2 -Pause {} `
            -HasExited { $false } -GetExitCode { 0 } -WaitForExit { param($ms) $true } `
            -PauseGame {} -Click {
                param($name)
                if ($name -eq 'menu') { $cleanUiState.Menu = $true }
                elseif ($name -eq 'exit_game') { $cleanUiState.Confirm = $true }
            } -SendEscape { $cleanUiState.Menu = $false; $cleanUiState.Confirm = $false } `
            -IsMenuOpen { $cleanUiState.Menu } -IsConfirmOpen { $cleanUiState.Confirm }
    } -Verify { $cleanExitReport.oracle_differences = 0 }
    if ($cleanExitReport.normal_game_exit -ne $true -or -not $cleanExitResult.ConfirmationSent -or $cleanExitResult.ExitCode -ne 0) {
        throw 'Confirmed zero-code exit did not report clean normal_game_exit.'
    }

    $lateCleanState = @{ Menu = $false; Confirm = $false; Exited = $false; Waits = 0; Escapes = 0; Clicks = [Collections.Generic.List[string]]::new() }
    $lateCleanReport = [ordered]@{}
    $lateCleanResult = $null
    Invoke-ExitThenVerify -Report $lateCleanReport -ExitGame {
        $script:lateCleanResult = Invoke-UiExitFlow -MaxAttempts 3 -MaxPolls 2 -Pause {} `
            -HasExited { $lateCleanState.Exited } -GetExitCode { 0 } -WaitForExit {
                param($ms)
                $lateCleanState.Waits++
                $lateCleanState.Exited = $true
                $false
            } -PauseGame {} -Click {
                param($name)
                $lateCleanState.Clicks.Add([string]$name)
                if ($name -eq 'menu') { $lateCleanState.Menu = $true }
                elseif ($name -eq 'exit_game') { $lateCleanState.Confirm = $true }
            } -SendEscape { $lateCleanState.Escapes++ } `
            -IsMenuOpen { $lateCleanState.Menu } -IsConfirmOpen { $lateCleanState.Confirm }
    } -Verify { $lateCleanReport.oracle_differences = 0 }
    if ($lateCleanReport.normal_game_exit -ne $true -or -not $lateCleanResult.ConfirmationSent -or $lateCleanResult.ExitCode -ne 0 -or
        $lateCleanState.Escapes -ne 0 -or ($lateCleanState.Clicks -join ',') -cne 'menu,exit_game,exit_confirm') {
        throw "Clean exit right after first wait failed or sent extra input: exit=$($lateCleanReport.normal_game_exit) escapes=$($lateCleanState.Escapes) clicks=$($lateCleanState.Clicks -join ',')"
    }

    $lateCrashState = @{ Menu = $false; Confirm = $false; Exited = $false }
    $lateCrashErr = $null
    try {
        $null = Invoke-UiExitFlow -MaxAttempts 3 -MaxPolls 2 -Pause {} `
            -HasExited { $lateCrashState.Exited } -GetExitCode { 7 } -WaitForExit {
                param($ms)
                $lateCrashState.Exited = $true
                $false
            } -PauseGame {} -Click {
                param($name)
                if ($name -eq 'menu') { $lateCrashState.Menu = $true }
                elseif ($name -eq 'exit_game') { $lateCrashState.Confirm = $true }
            } -SendEscape {} -IsMenuOpen { $lateCrashState.Menu } -IsConfirmOpen { $lateCrashState.Confirm }
    } catch { $lateCrashErr = $_.Exception.Message }
    if ($lateCrashErr -notlike '*non-zero exit code 7 after exit confirmation*') {
        throw "Non-zero exit after wait 1 was misclassified: $lateCrashErr"
    }

    $slowExitState = @{
        Menu = $false; Confirm = $false; Exited = $false; Confirmed = $false; Waits = 0
        PostConfirmClicks = 0; PostConfirmEscapes = 0; PostConfirmPauses = 0; PostConfirmMenuProbes = 0; PostConfirmConfirmProbes = 0
    }
    $slowExitReport = [ordered]@{}
    $slowExitResult = $null
    Invoke-ExitThenVerify -Report $slowExitReport -ExitGame {
        $script:slowExitResult = Invoke-UiExitFlow -MaxAttempts 3 -MaxPolls 2 -Pause {} `
            -HasExited { $slowExitState.Exited } -GetExitCode { 0 } -WaitForExit {
                param($ms)
                $slowExitState.Waits++
                if ($slowExitState.Waits -eq 1) { return $false }
                $slowExitState.Exited = $true
                $true
            } -PauseGame {
                if ($slowExitState.Confirmed) { $slowExitState.PostConfirmPauses++ }
            } -Click {
                param($name)
                if ($slowExitState.Confirmed) { $slowExitState.PostConfirmClicks++ }
                if ($name -eq 'menu') { $slowExitState.Menu = $true }
                elseif ($name -eq 'exit_game') { $slowExitState.Confirm = $true }
                elseif ($name -eq 'exit_confirm') { $slowExitState.Confirmed = $true }
            } -SendEscape {
                if ($slowExitState.Confirmed) { $slowExitState.PostConfirmEscapes++ }
            } -IsMenuOpen {
                if ($slowExitState.Confirmed) { $slowExitState.PostConfirmMenuProbes++ }
                $slowExitState.Menu
            } -IsConfirmOpen {
                if ($slowExitState.Confirmed) { $slowExitState.PostConfirmConfirmProbes++ }
                $slowExitState.Confirm
            }
    } -Verify { $slowExitReport.oracle_differences = 0 }
    if ($slowExitReport.normal_game_exit -ne $true -or -not $slowExitResult.ConfirmationSent -or $slowExitResult.ExitCode -ne 0 -or
        $slowExitState.Waits -ne 2 -or $slowExitState.PostConfirmClicks -ne 0 -or $slowExitState.PostConfirmEscapes -ne 0 -or
        $slowExitState.PostConfirmPauses -ne 0 -or $slowExitState.PostConfirmMenuProbes -ne 0 -or $slowExitState.PostConfirmConfirmProbes -ne 0) {
        throw "Slow shutdown after exit_confirm re-entered UI/clock/Escape or failed: exit=$($slowExitReport.normal_game_exit) waits=$($slowExitState.Waits) postClicks=$($slowExitState.PostConfirmClicks) postEscapes=$($slowExitState.PostConfirmEscapes) postPauses=$($slowExitState.PostConfirmPauses) postMenu=$($slowExitState.PostConfirmMenuProbes) postConfirm=$($slowExitState.PostConfirmConfirmProbes)"
    }

    $hungAfterConfirmState = @{
        Menu = $false; Confirm = $false; Confirmed = $false; Waits = 0
        PostConfirmCalls = 0
    }
    $hungAfterConfirmErr = $null
    $exitConfirmationState = @{ Sent = $false }
    $outerCatchCalls = @{ Escape = 0; Pause = 0 }
    $outerGame = [pscustomobject]@{ HasExited = $false }
    $outerReader = [pscustomobject]@{}
    $outerCatchReport = [ordered]@{}
    $outerToolTracker = [Collections.Generic.List[object]]::new()
    try {
        $null = Invoke-UiExitFlow -MaxAttempts 3 -MaxPolls 2 -Pause {} `
            -HasExited { $false } -GetExitCode { 0 } -WaitForExit { param($ms) $hungAfterConfirmState.Waits++; $false } `
            -PauseGame { if ($hungAfterConfirmState.Confirmed) { $hungAfterConfirmState.PostConfirmCalls++ } } `
            -Click {
                param($name)
                if ($hungAfterConfirmState.Confirmed) { $hungAfterConfirmState.PostConfirmCalls++ }
                if ($name -eq 'menu') { $hungAfterConfirmState.Menu = $true }
                elseif ($name -eq 'exit_game') { $hungAfterConfirmState.Confirm = $true }
                elseif ($name -eq 'exit_confirm') { $hungAfterConfirmState.Confirmed = $true }
            } -SendEscape { if ($hungAfterConfirmState.Confirmed) { $hungAfterConfirmState.PostConfirmCalls++ } } `
            -IsMenuOpen { if ($hungAfterConfirmState.Confirmed) { $hungAfterConfirmState.PostConfirmCalls++ }; $hungAfterConfirmState.Menu } `
            -IsConfirmOpen { if ($hungAfterConfirmState.Confirmed) { $hungAfterConfirmState.PostConfirmCalls++ }; $hungAfterConfirmState.Confirm } `
            -OnConfirmationSent { $exitConfirmationState.Sent = $true }
    } catch {
        $hungAfterConfirmErr = $_.Exception.Message
        $game = $outerGame; $reader = $outerReader; $recorderRun = $null; $toolTracker = $outerToolTracker; $report = $outerCatchReport; $failure = $null
        function Stage([string]$name) { $report.phase = $name }
        function Drive([string]$act, [hashtable]$params) { if ($act -eq 'Key' -and $params.Key -eq 'Escape') { $outerCatchCalls.Escape++ } }
        function Pause-Game([bool]$pause) { $outerCatchCalls.Pause++ }
        . ([scriptblock]::Create($smokeCatch.Extent.Text.Trim().Substring(1, $smokeCatch.Extent.Text.Trim().Length - 2)))
    }
    if ($hungAfterConfirmState.PostConfirmCalls -ne 0 -or $hungAfterConfirmState.Waits -lt 2 -or $hungAfterConfirmErr -notlike '*did not exit after exit_confirm*' -or
        -not $exitConfirmationState.Sent -or $outerCatchCalls.Escape -ne 0 -or $outerCatchCalls.Pause -ne 0) {
        throw "Hung shutdown after exit_confirm re-entered UI/clock/Escape or outer catch sent Escape/Pause: postCalls=$($hungAfterConfirmState.PostConfirmCalls) waits=$($hungAfterConfirmState.Waits) confirmed=$($exitConfirmationState.Sent) outerEscape=$($outerCatchCalls.Escape) outerPause=$($outerCatchCalls.Pause) err=$hungAfterConfirmErr"
    }

    $preConfirmCatchCalls = @{ Escape = 0; Pause = 0 }
    $exitConfirmationState = @{ Sent = $false }
    $preConfirmCatchReport = [ordered]@{}
    try {
        throw 'Synthetic failure before exit confirmation.'
    } catch {
        $game = $outerGame; $reader = $outerReader; $recorderRun = $null; $toolTracker = $outerToolTracker; $report = $preConfirmCatchReport; $failure = $null
        function Stage([string]$name) { $report.phase = $name }
        function Drive([string]$act, [hashtable]$params) { if ($act -eq 'Key' -and $params.Key -eq 'Escape') { $preConfirmCatchCalls.Escape++ } }
        function Pause-Game([bool]$pause) { $preConfirmCatchCalls.Pause++ }
        . ([scriptblock]::Create($smokeCatch.Extent.Text.Trim().Substring(1, $smokeCatch.Extent.Text.Trim().Length - 2)))
    }
    if ($preConfirmCatchCalls.Escape -ne 1 -or $preConfirmCatchCalls.Pause -ne 1) {
        throw "Outer catch before exit confirmation did not pause game: escape=$($preConfirmCatchCalls.Escape) pause=$($preConfirmCatchCalls.Pause)"
    }
    $stopOwnedGameAst = $smoke.Find({ param($node) $node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq 'Stop-OwnedGame' }, $true)
    if (-not $stopOwnedGameAst -or $stopOwnedGameAst.Extent.Text -notmatch '-OnConfirmationSent\s*\{\s*\$exitConfirmationState\.Sent\s*=\s*\$true\s*\}') {
        throw 'Stop-OwnedGame does not propagate -OnConfirmationSent to $exitConfirmationState.Sent.'
    }
    $script:Passed++; Write-Host 'PASS exit requires confirmation and exit code 0, waits cleanly without re-entering UI or outer pause after confirmation, and verifies offline on failure'

    $box = Get-UiRegionBox @(10, 20) 1920 1080 120 60
    if (($box -join ',') -cne '0,0,130,80') { throw "Clamped top-left UI box mismatch: $($box -join ',')" }
    $edgeBox = Get-UiRegionBox @(1910, 1070) 1920 1080 120 60
    if (($edgeBox -join ',') -cne '1790,1010,1920,1080') { throw "Clamped bottom-right UI box mismatch: $($edgeBox -join ',')" }

    $stepPolls = 0; $stepActed = 0
    $observed = Invoke-ObservedUiStep -MaxPolls 4 -Pause {} -Act { $script:stepActed++ } -Observe {
        $script:stepPolls++
        $script:stepPolls -ge 3
    }
    if (-not $observed -or $stepActed -ne 1 -or $stepPolls -ne 3) { throw "Observed step failed: observed=$observed acted=$stepActed polls=$stepPolls" }
    $missedPolls = 0
    $missed = Invoke-ObservedUiStep -MaxPolls 3 -Pause {} -Act {} -Observe { $script:missedPolls++; $false }
    if ($missed -or $missedPolls -ne 3) { throw "Missed observed step should return false after bound: missed=$missed polls=$missedPolls" }

    $seqAttempts = 0; $seqResets = 0
    $seqResult = Invoke-BoundedUiSequence -Name 'exit' -MaxAttempts 3 -FailureMessage 'The game did not exit through its confirmation dialog.' -Reset { $script:seqResets++ } -Attempt {
        param($attempt)
        $script:seqAttempts++
        if ($attempt -lt 2) { return [pscustomobject]@{ Done=$false; Reason='exit_game did not open confirmation' } }
        return [pscustomobject]@{ Done=$true; Reason=$null }
    }
    if (-not $seqResult.Done -or $seqResult.Attempts -ne 2 -or $seqAttempts -ne 2 -or $seqResets -ne 1 -or $seqResult.Failures.Count -ne 1) {
        throw "Observed sequence retry did not recover on second attempt."
    }
    $exhaustedSeqError = $null; $exhaustedSeqAttempts = 0
    try {
        $null = Invoke-BoundedUiSequence -Name 'save' -MaxAttempts 3 -FailureMessage 'The UI did not produce a complete text save.' -Reset {} -Attempt {
            $script:exhaustedSeqAttempts++
            return [pscustomobject]@{ Done=$false; Reason='save_menu did not open save dialog' }
        }
    } catch { $exhaustedSeqError = $_.Exception.Message }
    if ($exhaustedSeqAttempts -ne 3 -or $exhaustedSeqError -notlike '*The UI did not produce a complete text save.*' -or $exhaustedSeqError -notlike '*save_menu did not open save dialog*') {
        throw "Exhausted sequence error mismatch: attempts=$exhaustedSeqAttempts error=$exhaustedSeqError"
    }
    $invariantAttempts = 0; $invariantError = $null
    try {
        $null = Invoke-BoundedUiSequence -Name 'save' -MaxAttempts 3 -FailureMessage 'The UI did not produce a complete text save.' -Reset {} -Attempt {
            $script:invariantAttempts++
            throw 'Game advanced during the save.'
        }
    } catch { $invariantError = $_.Exception.Message }
    if ($invariantAttempts -ne 1 -or $invariantError -notlike '*Game advanced during the save.*') {
        throw "Invariant error was retried: attempts=$invariantAttempts error=$invariantError"
    }
    $script:Passed++; Write-Host 'PASS observed UI steps and sequences retry boundedly and stop on invariants'

    $exitOkReport = [ordered]@{}
    $exitOkVerified = $false
    Invoke-ExitThenVerify -Report $exitOkReport -ExitGame {} -Verify { $script:exitOkVerified = $true; $exitOkReport.oracle_differences = 0 }
    if (-not $exitOkVerified -or -not $exitOkReport.normal_game_exit -or $exitOkReport.oracle_differences -ne 0) {
        throw 'Clean exit path did not run verification or record normal_game_exit.'
    }
    $exitFailReport = [ordered]@{}
    $exitFailVerified = $false
    $exitFailThrown = $null
    try {
        Invoke-ExitThenVerify -Report $exitFailReport -ExitGame { throw 'The game did not exit through its confirmation dialog.' } -Verify {
            $script:exitFailVerified = $true
            $exitFailReport.oracle_differences = 0
            $exitFailReport.window_headless_identical = $true
        }
    } catch { $exitFailThrown = $_.Exception.Message }
    if (-not $exitFailVerified -or $exitFailReport.normal_game_exit -ne $false -or $exitFailReport.oracle_differences -ne 0 -or
        -not $exitFailReport.window_headless_identical -or $exitFailReport.exit_error -cne 'The game did not exit through its confirmation dialog.' -or
        $exitFailThrown -cne 'The game did not exit through its confirmation dialog.') {
        throw "Failed exit hid verdicts or did not rethrow honestly: verified=$exitFailVerified thrown=$exitFailThrown"
    }
    $bothFailReport = [ordered]@{}
    $bothFailThrown = $null
    try {
        Invoke-ExitThenVerify -Report $bothFailReport -ExitGame { throw 'Windows refused to focus the game; input was not sent.' } -Verify {
            throw 'Fresh-save oracle did not establish complete matching coverage.'
        }
    } catch { $bothFailThrown = $_.Exception.Message }
    if ($bothFailReport.exit_error -cne 'Windows refused to focus the game; input was not sent.' -or
        $bothFailThrown -cne 'Fresh-save oracle did not establish complete matching coverage.') {
        throw "Dual exit+verify failure lost exit_error or verify exception: exit_error=$($bothFailReport.exit_error) thrown=$bothFailThrown"
    }
    $script:Passed++; Write-Host 'PASS oracle and render verdicts still run when game exit fails'

    $toolDir = Join-Path $root 'toollogs'
    $null = New-Item -ItemType Directory -Path $toolDir
    $python = (Get-Command python -ErrorAction Stop).Source
    $toolReport = [ordered]@{ checks = @() }
    $toolTracker = [Collections.Generic.List[object]]::new()
    $okRun = Start-SmokeTool -Exe $python -Arguments @('-c', "import sys; print('ok stdout'); print('ok stderr', file=sys.stderr)") -Name 'ok-tool' -WorkingDirectory $toolDir -OutputDir $toolDir -Tracker $toolTracker
    $okOut = Wait-SmokeTool -Run $okRun -Seconds 15 -Report $toolReport -WriteReport {} -Utf8 $utf8
    if ($okOut.Trim() -cne 'ok stdout' -or (Read-SharedUtf8Text $okRun.StdoutPath).Trim() -cne 'ok stdout' -or
        (Read-SharedUtf8Text $okRun.StderrPath).Trim() -cne 'ok stderr' -or
        (Read-SharedUtf8Text $okRun.LogPath) -notmatch 'ok stdout' -or (Read-SharedUtf8Text $okRun.LogPath) -notmatch 'ok stderr' -or
        $toolReport.checks.Count -ne 1 -or $toolReport.checks[0].exit -ne 0) {
        throw 'Completed tool did not persist stdout, stderr and combined log files.'
    }

    $failRun = Start-SmokeTool -Exe $python -Arguments @('-c', "import sys; print('bad stdout'); print('bad stderr', file=sys.stderr); sys.exit(9)") -Name 'fail-tool' -WorkingDirectory $toolDir -OutputDir $toolDir -Tracker $toolTracker
    $failMsg = $null
    try { $null = Wait-SmokeTool -Run $failRun -Seconds 15 -Report $toolReport -WriteReport {} -Utf8 $utf8 } catch { $failMsg = $_.Exception.Message }
    if ($failMsg -notlike '*fail-tool failed with exit 9*' -or (Read-SharedUtf8Text $failRun.StderrPath).Trim() -cne 'bad stderr') {
        throw "Failed tool error or stderr log mismatch: $failMsg"
    }

    $readyEventName = 'Local\BattlePlanSmokeReady_' + [Guid]::NewGuid().ToString('N')
    $stopEventName = 'Local\BattlePlanSmokeStop_' + [Guid]::NewGuid().ToString('N')
    $readyEvent = [Threading.EventWaitHandle]::new($false, [Threading.EventResetMode]::ManualReset, $readyEventName)
    $stopEvent = [Threading.EventWaitHandle]::new($false, [Threading.EventResetMode]::ManualReset, $stopEventName)
    $liveCode = "import ctypes, sys; k = ctypes.windll.kernel32; print('live stdout', flush=True); print('live stderr', file=sys.stderr, flush=True); r = k.OpenEventW(0x0002, False, sys.argv[1]); k.SetEvent(r); k.CloseHandle(r); s = k.OpenEventW(0x00100000, False, sys.argv[2]); k.WaitForSingleObject(s, 0xFFFFFFFF); k.CloseHandle(s)"
    $liveRun = Start-SmokeTool -Exe $python -Arguments @('-c', $liveCode, $readyEventName, $stopEventName) -Name 'detach' -WorkingDirectory $toolDir -OutputDir $toolDir -Tracker $toolTracker
    try {
        if (-not $readyEvent.WaitOne(10000)) { throw 'Synthetic live tool did not signal readiness.' }
        $liveStdout = Read-SharedUtf8Text $liveRun.StdoutPath
        $liveStderr = Read-SharedUtf8Text $liveRun.StderrPath
        if ($liveStdout.Trim() -cne 'live stdout' -or $liveStderr.Trim() -cne 'live stderr') {
            throw "Tool streams were not persisted while running: out=[$liveStdout] err=[$liveStderr]"
        }
        $timeoutMsg = $null
        try { $null = Wait-SmokeTool -Run $liveRun -TimeoutMs 1 -Report $toolReport -WriteReport {} -Utf8 $utf8 } catch { $timeoutMsg = $_.Exception.Message }
        if ($timeoutMsg -notlike '*Timed out: detach*' -or (Read-SharedUtf8Text $liveRun.LogPath) -notmatch 'live stdout' -or (Read-SharedUtf8Text $liveRun.LogPath) -notmatch 'live stderr') {
            throw "Timed-out tool did not retain streamed output: $timeoutMsg"
        }

        $active = @(Get-ActiveSmokeTools $toolTracker)
        if ($active.Count -ne 1 -or $active[0].Process.Id -ne $liveRun.Process.Id) {
            throw "Tracked active tool list mismatch: count=$($active.Count)"
        }

        $secondDetachStarted = $false
        $detachSkipReport = [ordered]@{}
        $fakeRecorder = [pscustomobject]@{ Process = $liveRun.Process }
        Invoke-FailureRecorderCleanup -RecorderRun $fakeRecorder -ToolTracker $toolTracker -Report $detachSkipReport -RunDetach {
            $script:secondDetachStarted = $true
        } -WaitRecorder {}
        if ($secondDetachStarted -or $detachSkipReport.recorder_cleanup_error -notlike "*$($liveRun.Process.Id)*") {
            throw "Second detach was launched while first detach PID $($liveRun.Process.Id) was still running."
        }

        $retainedLock = Join-Path $root 'retainedtoollock'
        $null = New-Item -ItemType Directory -Path $retainedLock
        [IO.File]::WriteAllText((Join-Path $retainedLock 'owner'), 'tool-owner')
        $lock = $retainedLock; $ownerToken = 'tool-owner'
        $reader = $null; $game = $null; $recorderRun = $null; $report = [ordered]@{}; $failure = $null
        $selection = $null; $selectionWritten = $false; $settingsPath = Join-Path $fixtureProfile 'settings.txt'; $settingsBytes = [byte[]]@(1, 2, 3)
        $started = [Diagnostics.Stopwatch]::StartNew()
        function Write-Report {}
        . ([scriptblock]::Create($cleanup.Extent.Text.Trim().Substring(1, $cleanup.Extent.Text.Trim().Length - 2)))
        if ($report.cleanup_complete -ne $false -or -not (Test-Path -LiteralPath $retainedLock) -or
            @($report.retained_tool_pids | Where-Object { $_.pid -eq $liveRun.Process.Id }).Count -ne 1) {
            throw 'Cleanup released game lock or marked complete while a timed-out tool PID was still alive.'
        }

        $null = $stopEvent.Set()
        if (-not $liveRun.Process.WaitForExit(10000)) { throw 'Synthetic live tool did not exit after stop event.' }
        $retryDetachStarted = $false
        $detachRanReport = [ordered]@{}
        Invoke-FailureRecorderCleanup -RecorderRun ([pscustomobject]@{ Process = [pscustomobject]@{ HasExited = $false } }) -ToolTracker $toolTracker -Report $detachRanReport -RunDetach {
            $script:retryDetachStarted = $true
        } -WaitRecorder {}
        if (-not $retryDetachStarted) { throw 'Failure detach did not run when no prior detach was alive.' }
        Remove-Item -LiteralPath (Join-Path $retainedLock 'owner') -Force
        Remove-Item -LiteralPath $retainedLock -Force
    } finally {
        $null = $stopEvent.Set()
        $readyEvent.Dispose()
        $stopEvent.Dispose()
        if (-not $liveRun.Process.HasExited) {
            try { $liveRun.Process.Kill(); $null = $liveRun.Process.WaitForExit(5000) } catch {}
        }
    }
    $script:Passed++; Write-Host 'PASS tool streams persist deterministically, track active PIDs, guard double detach, and retain lock'
    Write-Host "$script:Passed offline checks passed."
} finally {
    if ($substDrive) { & (Join-Path $env:SystemRoot 'System32\subst.exe') $substDrive /D }
    foreach ($name in @('alias', 'parentalias')) {
        $link = Join-Path $root $name
        if (Test-Path -LiteralPath $link) { [IO.Directory]::Delete($link) }
    }
    $resolvedRoot = [IO.Path]::GetFullPath($root)
    if (-not $resolvedRoot.StartsWith($ScratchRoot.TrimEnd('\') + '\', [StringComparison]::OrdinalIgnoreCase)) { throw 'Refusing test cleanup outside ScratchRoot.' }
    Remove-Item -LiteralPath $resolvedRoot -Recurse -Force
}
