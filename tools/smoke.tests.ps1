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
        'Invoke-WithFocusRetry', 'Get-UiRegionBox', 'Invoke-ObservedUiStep', 'Invoke-BoundedUiSequence',
        'Invoke-ExitThenVerify', 'Read-SharedUtf8Text', 'Start-SmokeTool', 'Wait-SmokeTool'
    )
}, $false)) { . ([scriptblock]::Create($definition.Extent.Text)) }
try { $ScratchRoot = Resolve-LaunchUserDirectory $ScratchRoot }
catch { throw "ScratchRoot prerequisite failed: supply an existing physical, hyphen-free directory without junctions, symbolic links or subst drives. $($_.Exception.Message)" }

if (-not ('Hoi4ProbeInput' -as [type])) {
    Add-Type 'public static class Hoi4ProbeInput { public static System.IntPtr GetForegroundWindow() { return System.IntPtr.Zero; } }'
}
function Get-Process { param([string]$Name, $ErrorAction) }
$fixtureState = @{ Arguments=$null; SelectionPath=''; SelectionAtLaunch=$null }
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
    $failure = $null
    try { $null = & $Driver -Action Launch -GameExe $exe -UserDirectory $userPath -ContinueLastSave -ExpectedSave $saveName -Foreground }
    catch { $failure = $_.Exception.Message }
    if (-not $failure -or $failure -notlike "*$expectedError*") { throw "$name expected '$expectedError', got '$failure'." }
    if ($expectedError -eq 'SYNTHETIC_PROCESS_BOUNDARY') {
        Assert-Bytes $fixtureState.SelectionAtLaunch ($utf8.GetBytes($expected.Replace('fixture.hoi4', ($saveName -replace [char]0xE9, '\u00e9'))))
        if ($fixtureState.Arguments -notcontains '--continuelastsave' -or $fixtureState.Arguments -notcontains ('"-userdir=' + $userPath + '"')) { throw 'Launch lost the continuation flag or quoted user directory.' }
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
$cleanup = $smoke.Find({param($node) $node -is [Management.Automation.Language.TryStatementAst] -and $node.Finally -and $node.Finally.Extent.Text -match '\$ownedGameStopped='}, $true).Finally
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
    $focusResult = Invoke-WithFocusRetry -MaxAttempts 4 -Pause { $script:focusPauses++ } -Action {
        $script:focusCalls++
        if ($script:focusCalls -lt 3) { throw 'Windows refused to focus the game; input was not sent.' }
        'focused'
    }
    if ($focusResult -cne 'focused' -or $focusCalls -ne 3 -or $focusPauses -ne 2) { throw "Focus retry recovered unexpectedly: result=$focusResult calls=$focusCalls pauses=$focusPauses" }
    $lostCalls = 0
    $lostResult = Invoke-WithFocusRetry -MaxAttempts 3 -Pause {} -Action {
        $script:lostCalls++
        if ($script:lostCalls -eq 1) { throw 'Game focus was lost; input was not sent.' }
        'recovered'
    }
    if ($lostResult -cne 'recovered' -or $lostCalls -ne 2) { throw "Focus-lost retry failed: result=$lostResult calls=$lostCalls" }
    $exhaustedCalls = 0; $exhaustedError = $null
    try {
        $null = Invoke-WithFocusRetry -MaxAttempts 3 -Pause {} -Action {
            $script:exhaustedCalls++
            throw 'Windows refused to focus the game; input was not sent.'
        }
    } catch { $exhaustedError = $_.Exception.Message }
    if ($exhaustedCalls -ne 3 -or $exhaustedError -notlike '*Windows refused to focus the game*') { throw "Bounded focus retry did not stop at limit: calls=$exhaustedCalls error=$exhaustedError" }
    $nonFocusCalls = 0; $nonFocusError = $null
    try {
        $null = Invoke-WithFocusRetry -MaxAttempts 4 -Pause {} -Action {
            $script:nonFocusCalls++
            throw 'The game window no longer belongs to the selected process.'
        }
    } catch { $nonFocusError = $_.Exception.Message }
    if ($nonFocusCalls -ne 1 -or $nonFocusError -notlike '*no longer belongs*') { throw "Non-focus error was retried: calls=$nonFocusCalls error=$nonFocusError" }
    $script:Passed++; Write-Host 'PASS focus-borrow retries recover transiently and stay bounded'

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
    $okRun = Start-SmokeTool -Exe $python -Arguments @('-c', "import sys; print('ok stdout'); print('ok stderr', file=sys.stderr)") -Name 'ok-tool' -WorkingDirectory $toolDir -OutputDir $toolDir
    $okOut = Wait-SmokeTool -Run $okRun -Seconds 15 -Report $toolReport -WriteReport {} -Utf8 $utf8
    if ($okOut.Trim() -cne 'ok stdout' -or (Read-SharedUtf8Text $okRun.StdoutPath).Trim() -cne 'ok stdout' -or
        (Read-SharedUtf8Text $okRun.StderrPath).Trim() -cne 'ok stderr' -or
        (Read-SharedUtf8Text $okRun.LogPath) -notmatch 'ok stdout' -or (Read-SharedUtf8Text $okRun.LogPath) -notmatch 'ok stderr' -or
        $toolReport.checks.Count -ne 1 -or $toolReport.checks[0].exit -ne 0) {
        throw 'Completed tool did not persist stdout, stderr and combined log files.'
    }

    $failRun = Start-SmokeTool -Exe $python -Arguments @('-c', "import sys; print('bad stdout'); print('bad stderr', file=sys.stderr); sys.exit(9)") -Name 'fail-tool' -WorkingDirectory $toolDir -OutputDir $toolDir
    $failMsg = $null
    try { $null = Wait-SmokeTool -Run $failRun -Seconds 15 -Report $toolReport -WriteReport {} -Utf8 $utf8 } catch { $failMsg = $_.Exception.Message }
    if ($failMsg -notlike '*fail-tool failed with exit 9*' -or (Read-SharedUtf8Text $failRun.StderrPath).Trim() -cne 'bad stderr') {
        throw "Failed tool error or stderr log mismatch: $failMsg"
    }

    $readyFile = Join-Path $toolDir 'ready'
    $stopFile = Join-Path $toolDir 'stop'
    $watcher = [IO.FileSystemWatcher]::new($toolDir, 'ready')
    $liveCode = "import os, sys, time; print('live stdout', flush=True); print('live stderr', file=sys.stderr, flush=True); open(sys.argv[1], 'w').close(); deadline = time.monotonic() + 15; " +
        "exec('while not os.path.exists(sys.argv[2]) and time.monotonic() < deadline: time.sleep(0.02)')"
    $liveRun = Start-SmokeTool -Exe $python -Arguments @('-c', $liveCode, $readyFile, $stopFile) -Name 'live-tool' -WorkingDirectory $toolDir -OutputDir $toolDir
    try {
        if (-not (Test-Path -LiteralPath $readyFile)) {
            $changed = $watcher.WaitForChanged([IO.WatcherChangeTypes]::Created, 10000)
            if ($changed.TimedOut -and -not (Test-Path -LiteralPath $readyFile)) { throw 'Synthetic live tool did not signal readiness.' }
        }
        $liveStdout = Read-SharedUtf8Text $liveRun.StdoutPath
        $liveStderr = Read-SharedUtf8Text $liveRun.StderrPath
        if ($liveStdout.Trim() -cne 'live stdout' -or $liveStderr.Trim() -cne 'live stderr') {
            throw "Tool streams were not persisted while running: out=[$liveStdout] err=[$liveStderr]"
        }
        $timeoutMsg = $null
        try { $null = Wait-SmokeTool -Run $liveRun -TimeoutMs 1 -Report $toolReport -WriteReport {} -Utf8 $utf8 } catch { $timeoutMsg = $_.Exception.Message }
        if ($timeoutMsg -notlike '*Timed out: live-tool*' -or (Read-SharedUtf8Text $liveRun.LogPath) -notmatch 'live stdout' -or (Read-SharedUtf8Text $liveRun.LogPath) -notmatch 'live stderr') {
            throw "Timed-out tool did not retain streamed output: $timeoutMsg"
        }
        [IO.File]::WriteAllText($stopFile, 'stop')
        $null = $liveRun.Process.WaitForExit(10000)
    } finally {
        $watcher.Dispose()
        if (-not $liveRun.Process.HasExited) {
            try { $liveRun.Process.Kill(); $null = $liveRun.Process.WaitForExit(5000) } catch {}
        }
    }
    $script:Passed++; Write-Host 'PASS tool stdout and stderr stream to files on disk while running and across timeout'
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
