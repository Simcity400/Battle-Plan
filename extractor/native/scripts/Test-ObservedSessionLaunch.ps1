# Offline tests. All WMI calls and game observations below are replaced in this
# test scope; only harmless PowerShell helper children are actually launched.
[CmdletBinding()]
param([Parameter(Mandatory = $true)][string]$OutputDirectory)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
. (Join-Path $PSScriptRoot 'ObservedSession.Launch.ps1')
$realFileIdentity = ${function:Get-ObservedFileIdentity}
$realProcessIdentity = ${function:Get-ObservedProcessIdentity}
$testRoot = (New-Item -ItemType Directory -Path $OutputDirectory -ErrorAction Stop).FullName
[IO.File]::WriteAllText((Join-Path $testRoot 'OFFLINE_TEST_ONLY.txt'), 'Fixture receipts are NOT production launch, capture, or E2E proof. Only PowerShell timeout helpers are real children.')
$script:case = ''
$script:calls = 0
$script:observations = 0
$script:identity = $null
$script:request = $null
$script:passed = 0
$script:failed = 0
$script:results = @()

function Assert-Test {
    param([bool]$Condition, [string]$Message)
    if (-not $Condition) { throw "ASSERT: $Message" }
}
function Test-Case {
    param([string]$Name, [scriptblock]$Body)
    try {
        & $Body
        $script:passed++
        $script:results += [ordered]@{ name = $Name; status = 'passed' }
        Write-Output "PASS $Name"
    } catch {
        $script:failed++
        $script:results += [ordered]@{ name = $Name; status = 'failed'; error = $_.ToString() }
        Write-Output "FAIL $Name : $_"
    }
}

# A hard sentinel replaces CIM even if another test mock fails.
function Invoke-CimMethod {
    param($ClassName, $MethodName, $OperationTimeoutSec, $Arguments, $ErrorAction)
    $script:calls++
    Assert-Test ($ClassName -ceq 'Win32_Process' -and $MethodName -ceq 'Create') 'wrong WMI API'
    Assert-Test ($Arguments.CommandLine -ceq $script:request.command_line) 'command differs from request'
    Assert-Test ($Arguments.CurrentDirectory -ceq (Split-Path -Parent $script:fixtureExe)) 'working directory differs'
    Assert-Test ($OperationTimeoutSec -eq 1) 'WMI timeout not forwarded'
    if ($script:case -eq 'cim-throws') { throw 'offline simulated CIM transport failure' }
    $result = [pscustomobject]@{ ReturnValue = [uint32]0; ProcessId = [uint32]4242 }
    switch ($script:case) {
        'wmi-rejection' { $result.ReturnValue = [uint32]2 }
        'zero-pid' { $result.ProcessId = [uint32]0 }
        'string-pid' { $result.ProcessId = '4242' }
        'null-return' { $result.ReturnValue = $null }
    }
    return $result
}
function Get-ObservedProcessIdentity {
    param([int]$TargetProcessId)
    $script:observations++
    if ($script:case -eq 'lookup-fails' -or ($script:case -eq 'exited-after-handoff' -and $script:observations -ge 2)) {
        throw 'offline process no longer exists'
    }
    $copy = [ordered]@{}
    foreach ($key in $script:identity.Keys) { $copy[$key] = $script:identity[$key] }
    if ($script:case -eq 'wrong-image') { $copy.exe_path = 'C:\unowned\hoi4.exe' }
    if ($script:case -eq 'wrong-pid') { $copy.pid++ }
    if ($script:case -eq 'old-process') { $copy.creation_filetime = '1' }
    if ($script:case -eq 'future-process') { $copy.creation_filetime = [long]::MaxValue.ToString() }
    if ($script:case -eq 'numeric-epoch') { $copy.creation_filetime = [long]$copy.creation_filetime }
    if ($script:case -eq 'reused-pid' -and $script:observations -ge 2) { $copy.creation_filetime = ([long]$copy.creation_filetime + 1).ToString() }
    if ($script:case -eq 'reused-after-hash' -and $script:observations -eq 3) { $copy.creation_filetime = ([long]$copy.creation_filetime + 1).ToString() }
    if ($script:case -eq 'session-change' -and $script:observations -ge 2) { $copy.session_id++ }
    return $copy
}
function Get-ObservedFileIdentity {
    param([string]$Path)
    $identity = & $realFileIdentity $Path
    if ($script:calls -gt 0) {
        if (($script:case -eq 'hash-change' -and $Path -ceq $script:fixtureExe) -or
            ($script:case -eq 'source-change' -and $Path.EndsWith('ObservedSession.Launch.ps1'))) { $identity.sha256 = 'f' * 64 }
        if ($script:case -eq 'hash-read-fails' -and $Path -ceq $script:fixtureExe) { throw 'offline hash read failure' }
    }
    return $identity
}
function Invoke-ObservedLaunchHelper {
    param([string]$RequestPath, [string]$HelperPath, [int]$TimeoutSeconds)
    $script:request = Get-Content -LiteralPath $RequestPath -Raw | ConvertFrom-Json
    $script:identity = [ordered]@{ pid = 4242; creation_filetime = $script:request.launch_started_filetime
        exe_path = $script:request.exe_before.path; session_id = 3 }
    if ($script:case -eq 'lease-held') {
        $writeOpened = $false
        try {
            $writeLease = [IO.File]::Open($script:fixtureExe, [IO.FileMode]::Open, [IO.FileAccess]::Write, [IO.FileShare]::ReadWrite)
            $writeOpened = $true
            $writeLease.Dispose()
        } catch [IO.IOException] { }
        Assert-Test (-not $writeOpened) 'executable writable during handoff'
    }
    $result = [ordered]@{ pid = 123; creation_filetime = '1'; exe_path = 'OFFLINE FIXTURE'
        exit_code = 0; timed_out = $false; terminated = $false; residual = $false; elapsed_ms = 0; error = $null }
    if ($script:case -eq 'helper-start-fails') { throw 'offline helper start failure' }
    if ($script:case -eq 'timeout-no-result') { $result.timed_out = $true; return $result }
    try { Invoke-ObservedWmiWorker $RequestPath }
    catch { $result.exit_code = 1; $result.error = $_.Exception.Message }
    $dir = Split-Path -Parent $RequestPath
    switch ($script:case) {
        'missing-result' { Remove-Item -LiteralPath (Join-Path $dir 'WMI_RESULT.json') }
        'missing-handoff' { Remove-Item -LiteralPath (Join-Path $dir 'PROCESS_HANDOFF.json') }
        'malformed-result' { [IO.File]::WriteAllText((Join-Path $dir 'WMI_RESULT.json'), '{bad') }
        'handoff-pid-mismatch' {
            $script:identity.pid = 999
            Write-ObservedJson (Join-Path $dir 'PROCESS_HANDOFF.json') $script:identity
        }
        'timeout-after-created' { $result.timed_out = $true; $result.terminated = $true }
        'nonzero-after-created' { $result.exit_code = 17 }
        'unknown-helper-exit' { $result.exit_code = $null }
        'helper-residual' { $result.residual = $true }
    }
    return $result
}

function Invoke-Fixture {
    param([string]$Name, [switch]$ExpectSuccess, [string]$ExpectedOutcome = 'created', [string]$ErrorContains = '')
    $script:case = $Name
    $script:calls = 0
    $script:observations = 0
    $caseRoot = (New-Item -ItemType Directory -Path (Join-Path $testRoot $Name)).FullName
    $script:fixtureExe = Join-Path $caseRoot 'fixture file.exe'
    [IO.File]::WriteAllText($script:fixtureExe, 'OFFLINE fixture; not a PE executable')
    $hash = (& $realFileIdentity $script:fixtureExe).sha256
    $argsForGame = @('', 'space value', 'embedded"quote', 'C:\trailing space\', '$(no-execution); & ` | %PATH%', "apostrophe's")
    $parameters = @{ GameExe = $script:fixtureExe; GameArguments = $argsForGame; ExpectedExeSha256 = $hash
        OutputDirectory = Join-Path $caseRoot 'custody'; TimeoutSeconds = 1 }
    switch ($Name) {
        'empty-argv' { $parameters.GameArguments = @() }
        'bad-hash' { $parameters.ExpectedExeSha256 = '0' * 64 }
        'malformed-hash' { $parameters.ExpectedExeSha256 = 'unknown' }
        'relative-exe' { $parameters.GameExe = 'hoi4.exe' }
        'missing-exe' { $parameters.GameExe = Join-Path $caseRoot 'absent.exe' }
        'nul-argv' { $parameters.GameArguments = @("before$([char]0)after") }
        'oversize-argv' { $parameters.GameArguments = @('x' * 33000) }
    }
    $failure = $null
    try { $null = Invoke-ObservedSession @parameters }
    catch { $failure = $_ }
    $receipt = Get-Content -LiteralPath (Join-Path $parameters.OutputDirectory 'RECEIPT.json') -Raw | ConvertFrom-Json
    Assert-Test ($receipt.schema -ceq 'hoi4.observed_session_launch.v1') 'schema'
    if ($ExpectSuccess) {
        Assert-Test ($null -eq $failure) "unexpected failure: $failure"
        Assert-Test ($receipt.status -ceq 'launched' -and $receipt.custody_status -ceq 'verified') 'missing verified handoff'
        Assert-Test ($receipt.process.pid -eq 4242 -and $receipt.process.creation_filetime -is [string]) 'observer field contract'
        Assert-Test ($receipt.process.exe_path -ceq $script:fixtureExe -and $receipt.process.exe_sha256 -ceq $hash) 'image authority'
        Assert-Test ($receipt.process.session_id -eq 3 -and $script:observations -eq 3) 'independent observation chain'
        Assert-Test ($receipt.exe_before.sha256 -ceq $receipt.exe_after.sha256) 'before/after bytes'
        Assert-Test ($receipt.argv.Count -eq $parameters.GameArguments.Count) 'argv count changed'
        for ($i = 0; $i -lt $receipt.argv.Count; $i++) { Assert-Test ($receipt.argv[$i] -ceq $parameters.GameArguments[$i]) 'argv changed' }
    } else {
        Assert-Test ($null -ne $failure) 'expected failure'
        Assert-Test ($receipt.status -in @('failed', 'timed_out')) 'failure status'
        Assert-Test ($receipt.custody_status -ceq 'unverified' -and $null -eq $receipt.process) 'failed attempt acquired custody'
        if ($ErrorContains) { Assert-Test ($receipt.error.Contains($ErrorContains)) "missing error: $ErrorContains (got $($receipt.error))" }
    }
    Assert-Test ($receipt.launch_outcome -ceq $ExpectedOutcome) "outcome $($receipt.launch_outcome), expected $ExpectedOutcome"
    if ($ExpectedOutcome -eq 'not_attempted') { Assert-Test ($script:calls -eq 0 -and -not $receipt.possible_surviving_game) 'preflight launched' }
    if ($ExpectedOutcome -eq 'created' -or $ExpectedOutcome -eq 'unknown') { Assert-Test $receipt.possible_surviving_game 'uncertain residual concealed' }
    if ($Name.StartsWith('timeout')) { Assert-Test ($receipt.status -ceq 'timed_out') 'typed timeout' }
    if ($Name -eq 'lookup-fails') {
        Assert-Test (Test-Path -LiteralPath (Join-Path $parameters.OutputDirectory 'WMI_RESULT.json')) 'lost returned WMI facts'
        Assert-Test (-not (Test-Path -LiteralPath (Join-Path $parameters.OutputDirectory 'PROCESS_HANDOFF.json'))) 'invented handoff'
    }
}

Test-Case 'exact argv escaping' {
    $argv = @('plain', '', 'has space', 'a"b', 'C:\has space\', 'C:\plain\', '$();&|`', "quote's")
    $expected = 'plain "" "has space" "a\"b" "C:\has space\\" C:\plain\ $();&|` quote''s'
    Assert-Test ((ConvertTo-ObservedArgumentLine $argv) -ceq $expected) 'Windows escaping changed'
}
foreach ($name in @('success', 'empty-argv', 'lease-held')) { Test-Case $name { Invoke-Fixture $name -ExpectSuccess } }
foreach ($name in @('bad-hash', 'malformed-hash', 'relative-exe', 'missing-exe', 'nul-argv', 'oversize-argv')) {
    Test-Case $name { Invoke-Fixture $name -ExpectedOutcome 'not_attempted' }
}
Test-Case 'wmi-rejection' { Invoke-Fixture 'wmi-rejection' -ExpectedOutcome 'rejected' }
foreach ($name in @('cim-throws', 'null-return', 'helper-start-fails', 'timeout-no-result', 'missing-result', 'malformed-result')) {
    Test-Case $name { Invoke-Fixture $name -ExpectedOutcome 'unknown' }
}
foreach ($name in @('zero-pid', 'string-pid', 'lookup-fails', 'exited-after-handoff', 'wrong-image', 'wrong-pid',
    'old-process', 'future-process', 'numeric-epoch', 'reused-pid', 'reused-after-hash', 'session-change', 'hash-change',
    'source-change', 'hash-read-fails', 'missing-handoff', 'handoff-pid-mismatch', 'timeout-after-created',
    'nonzero-after-created', 'unknown-helper-exit', 'helper-residual')) {
    Test-Case $name { Invoke-Fixture $name }
}
Test-Case 'existing output is preserved' {
    $dir = Join-Path $testRoot 'success/custody'
    $before = [IO.File]::ReadAllText((Join-Path $dir 'RECEIPT.json'))
    $script:calls = 0
    $refused = $false
    try { $null = Invoke-ObservedSession -GameExe $script:fixtureExe -GameArguments @() -ExpectedExeSha256 ('0' * 64) -OutputDirectory $dir }
    catch { $refused = $true }
    Assert-Test ($refused -and $script:calls -eq 0) 'output reuse was allowed'
    Assert-Test ([IO.File]::ReadAllText((Join-Path $dir 'RECEIPT.json')) -ceq $before) 'previous evidence overwritten'
}
Test-Case 'absolute path requires drive or UNC share' {
    foreach ($path in @('C:relative.exe', '\current-drive.exe', 'hoi4.exe', '//server')) {
        $refused = $false
        try { Assert-ObservedAbsolutePath $path } catch { $refused = $true }
        Assert-Test $refused "accepted ambiguous path $path"
    }
    Assert-ObservedAbsolutePath 'F:/explicit/hoi4.exe'
    Assert-ObservedAbsolutePath '\\server\share\hoi4.exe'
}
Test-Case 'standalone entrypoint retains preflight failure' {
    $dir = Join-Path $testRoot 'entrypoint-refusal'
    $refused = $false
    try {
        & (Join-Path $PSScriptRoot 'Start-Hoi4ObservedSession.ps1') -GameExe (Join-Path $testRoot 'does-not-exist.exe') `
            -GameArguments @() -ExpectedExeSha256 ('0' * 64) -OutputDirectory $dir
    } catch { $refused = $true }
    Assert-Test $refused 'entrypoint did not throw'
    $receipt = Get-Content -LiteralPath (Join-Path $dir 'RECEIPT.json') -Raw | ConvertFrom-Json
    Assert-Test ($receipt.status -ceq 'failed' -and $receipt.launch_outcome -ceq 'not_attempted') 'entrypoint refusal receipt'
}
Test-Case 'real current-process identity' {
    $observed = & $realProcessIdentity $PID
    Assert-Test ($observed.pid -eq $PID -and $observed.creation_filetime -cmatch '^[1-9][0-9]+$') 'native identity read'
    Assert-Test ([IO.File]::Exists($observed.exe_path)) 'native image missing'
}
Test-Case 'real helper success' {
    $dir = (New-Item -ItemType Directory -Path (Join-Path $testRoot 'real-helper-success')).FullName
    $result = Invoke-ObservedBoundedHelper '[Console]::Out.WriteLine("offline success"); exit 0' $dir 5
    Write-ObservedJson (Join-Path $dir 'HELPER.json') $result
    Assert-Test ($result.exit_code -eq 0 -and -not $result.timed_out -and $null -eq $result.error) 'helper success lost'
    Assert-Test ((Get-Content -LiteralPath (Join-Path $dir 'helper.stdout.txt') -Raw).Contains('offline success')) 'stdout lost'
}
Test-Case 'real helper nonzero' {
    $dir = (New-Item -ItemType Directory -Path (Join-Path $testRoot 'real-helper-nonzero')).FullName
    $result = Invoke-ObservedBoundedHelper '[Console]::Error.WriteLine("offline failure"); exit 23' $dir 5
    Write-ObservedJson (Join-Path $dir 'HELPER.json') $result
    Assert-Test ($result.exit_code -eq 23 -and -not $result.timed_out) 'nonzero exit became success'
    Assert-Test ((Get-Content -LiteralPath (Join-Path $dir 'helper.stderr.txt') -Raw).Contains('offline failure')) 'stderr lost'
}
Test-Case 'real helper timeout' {
    $dir = (New-Item -ItemType Directory -Path (Join-Path $testRoot 'real-helper-timeout')).FullName
    $result = Invoke-ObservedBoundedHelper '[Console]::Out.WriteLine("before timeout"); Start-Sleep -Seconds 20' $dir 1
    Write-ObservedJson (Join-Path $dir 'HELPER.json') $result
    Assert-Test ($result.timed_out -and $result.terminated -and -not $result.residual) 'timeout/helper cleanup failed'
    Assert-Test ($result.elapsed_ms -lt 8000) 'timeout not bounded'
    Assert-Test ((Get-Content -LiteralPath (Join-Path $dir 'helper.stdout.txt') -Raw).Contains('before timeout')) 'timeout stdout lost'
}

Write-ObservedJson (Join-Path $testRoot 'TEST_RESULTS.json') ([ordered]@{
    schema = 'hoi4.observed_session_offline_tests.v1'; production_proof = $false
    passed = $script:passed; failed = $script:failed; tests = $script:results
})
Write-Output "Tests: $script:passed passed, $script:failed failed. Offline only; no game launched."
if ($script:failed -ne 0 -or $script:passed -eq 0) { exit 1 }
