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
    $node.Name -in @('Get-SubstDriveNames', 'Resolve-LaunchUserDirectory', 'Write-LaunchSelection', 'Copy-ShaderCompiler')
}, $false)) { . ([scriptblock]::Create($definition.Extent.Text)) }
try { $ScratchRoot = Resolve-LaunchUserDirectory $ScratchRoot }
catch { throw "ScratchRoot prerequisite failed: supply an existing physical, hyphen-free directory without junctions, symbolic links or subst drives. $($_.Exception.Message)" }

# Exercise real Launch, stopping at the process boundary without desktop APIs.
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

# Run Smoke's production snapshot, write and finally blocks with stopped synthetic
# processes; omit unrelated live capture, binary receipt and oracle operations.
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
    # Setup that fails after the lock is taken, before anything launches, releases it.
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
    # The staged viewer finds the compiler beside itself, or the override names it.
    $built = Join-Path $root 'built'; $staged = Join-Path $root 'staged'
    $null = New-Item -ItemType Directory -Path (Join-Path $built 'shader-tools\dxc') -Force
    $null = New-Item -ItemType Directory -Path $staged
    foreach ($name in @('dxc.exe', 'dxcompiler.dll', 'dxil.dll')) { [IO.File]::WriteAllText((Join-Path $built "shader-tools\dxc\$name"), $name) }
    $refused = $null
    try { $null = Copy-ShaderCompiler $staged $root '' } catch { $refused = $_.Exception.Message }
    if ($refused -notlike '*needs the pinned shader compiler*') { throw "A build without the compiler was not refused: $refused" }
    $refused = $null
    try { $null = Copy-ShaderCompiler $built $staged (Join-Path $root 'absent.exe') } catch { $refused = $_.Exception.Message }
    if ($refused -ne 'BATTLE_PLAN_SHADER_COMPILER names no file.' -or (Test-Path -LiteralPath (Join-Path $staged 'shader-tools'))) { throw "A missing override was not refused: $refused" }
    $override = Join-Path $built 'shader-tools\dxc\dxc.exe'
    if ((Copy-ShaderCompiler $built $staged $override) -cne $override -or (Test-Path -LiteralPath (Join-Path $staged 'shader-tools'))) { throw 'An override was not used as it is.' }
    $compiler = Copy-ShaderCompiler $built $staged ''
    if ($compiler -cne (Join-Path $staged 'shader-tools\dxc\dxc.exe')) { throw "Staged compiler path: $compiler" }
    foreach ($name in @('dxc.exe', 'dxcompiler.dll', 'dxil.dll')) {
        if ([IO.File]::ReadAllText((Join-Path $staged "shader-tools\dxc\$name")) -cne $name) { throw "The staged compiler lacks $name." }
    }
    $script:Passed++; Write-Host 'PASS the staged viewer takes the shader compiler along'
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
