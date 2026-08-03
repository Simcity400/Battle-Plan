<#
.SYNOPSIS
    Behavioural tests for the bounded world-readiness wait, plus the source
    facts that keep it wired to the scripts that run.

.DESCRIPTION
    The wait takes its observation, liveness probe, clock and sleep as
    scriptblocks, so every case here runs on a virtual clock with no game, no
    process and no elapsed wall time.

    Each case names the production change that makes it red. A case that no
    production change can break is not a test, and is not shipped here.

    Run:  powershell -NoProfile -ExecutionPolicy Bypass -File Test-Hoi4WorldReadyGate.ps1
#>
[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
. (Join-Path $scriptDir 'Hoi4WorldReady.Gate.ps1')

$script:Failures = @()
$script:Passed = 0

function Assert-True {
    param(
        [Parameter(Mandatory = $true)][bool]$Condition,
        [Parameter(Mandatory = $true)][string]$Message
    )
    if ($Condition) {
        $script:Passed++
    } else {
        $script:Failures += $Message
        Write-Host "  FAIL $Message"
    }
}

function Assert-Equal {
    param(
        [AllowNull()][object]$Expected,
        [AllowNull()][object]$Actual,
        [Parameter(Mandatory = $true)][string]$Message
    )
    Assert-True `
        -Condition ([string]$Expected -ceq [string]$Actual) `
        -Message "$Message (expected '$Expected', got '$Actual')"
}

# --- virtual clock -----------------------------------------------------------
# Time advances only when the wait sleeps. That makes every case deterministic
# and makes "waited N s" an assertable quantity rather than a wall-clock race.
$script:Now = [DateTime]::new(2026, 1, 1, 0, 0, 0, [DateTimeKind]::Utc)
$script:Sleeps = 0

function Reset-Clock {
    $script:Now = [DateTime]::new(2026, 1, 1, 0, 0, 0, [DateTimeKind]::Utc)
    $script:Sleeps = 0
}

$virtualNow = { $script:Now }
$virtualSleep = {
    param($ms)
    $script:Sleeps++
    $script:Now = $script:Now.AddMilliseconds($ms)
}

function New-Observation {
    param(
        [Parameter(Mandatory = $true)][int]$PlayerInputGate,
        [int]$Speed = 5,
        [bool]$Paused = $true
    )
    return [pscustomobject]@{
        Pid = 4242
        Speed = $Speed
        Paused = $Paused
        PlayerInputGate = $PlayerInputGate
    }
}

# =============================================================================
Write-Host 'case: a live session is admitted on the first observation'
# RED WHEN: the wait stops returning a set gate at all, or demands extra polls
# before it will accept one -- which would make every healthy capture pay a
# needless delay.
Reset-Clock
$result = Wait-Hoi4WorldReadyState `
    -TargetProcessId 4242 `
    -TimeoutSeconds 600 `
    -Observe { param($r) New-Observation -PlayerInputGate 1 } `
    -TestAlive { $true } `
    -UtcNow $virtualNow -Sleep $virtualSleep
Assert-Equal -Expected 1 -Actual $result.PlayerInputGate `
    -Message 'a set gate is returned as the ready state'
Assert-Equal -Expected 0 -Actual $script:Sleeps `
    -Message 'an already-live session is not made to wait'

# =============================================================================
Write-Host 'case: a loading campaign is WAITED for, not reported as ready'
# THIS IS THE DEFECT. The harness used to return the first observation whose
# singletons were coherent. Mid-load that observation has a zero gate, and the
# capture then died at the evidence-mode unpause.
#
# RED WHEN: the readiness predicate stops testing the gate -- e.g.
# `if ($null -ne $state) { return $state }`, which is exactly the production
# code this replaces. The wait then returns observation #1 with gate 0 and both
# assertions below fail.
Reset-Clock
$script:Polls = 0
$result = Wait-Hoi4WorldReadyState `
    -TargetProcessId 4242 `
    -TimeoutSeconds 600 `
    -Observe {
        param($r)
        $script:Polls++
        if ($script:Polls -le 5) { return (New-Observation -PlayerInputGate 0) }
        return (New-Observation -PlayerInputGate 1)
    } `
    -TestAlive { $true } `
    -UtcNow $virtualNow -Sleep $virtualSleep
Assert-True -Condition ($result.PlayerInputGate -ne 0) `
    -Message 'the state returned from a loading campaign has a set gate'
Assert-Equal -Expected 5 -Actual $script:Sleeps `
    -Message 'the wait polled through the load rather than accepting the first sample'

# =============================================================================
Write-Host 'case: a gate that never arrives is REFUSED, bounded, and says so'
# RED WHEN: the timeout stops throwing (a refusal converted into a proceed), or
# the message stops naming how long it waited, or the wait runs past its budget.
Reset-Clock
$threw = $false
$message = ''
try {
    [void](Wait-Hoi4WorldReadyState `
        -TargetProcessId 4242 `
        -TimeoutSeconds 60 `
        -Observe { param($r) New-Observation -PlayerInputGate 0 } `
        -TestAlive { $true } `
        -UtcNow $virtualNow -Sleep $virtualSleep)
} catch {
    $threw = $true
    $message = $_.Exception.Message
}
Assert-True -Condition $threw `
    -Message 'a gate that never arrives is refused'
Assert-True -Condition ($message -match 'waited \d+ s and it never became non-zero') `
    -Message 'the refusal states how long it waited'
Assert-True -Condition ($message -match 'CGameState\+0xA39') `
    -Message 'the refusal names the engine condition it waited on'
Assert-True -Condition ($message -notmatch 'did not expose stable world singletons') `
    -Message 'the gate refusal is not confused with the singleton refusal'
$elapsed = ($script:Now - [DateTime]::new(2026, 1, 1, 0, 0, 0, [DateTimeKind]::Utc)).TotalSeconds
Assert-True -Condition ($elapsed -le 61) `
    -Message "the wait is bounded by its budget (elapsed ${elapsed}s of a 60s budget)"

# =============================================================================
Write-Host 'case: no observation at all keeps the ORIGINAL diagnosis'
# The two failures point at different things. Collapsing them would make an
# unreadable process look like a slow load.
# RED WHEN: New-Hoi4WorldReadyTimeoutMessage stops branching on whether any
# observation was seen, or drops the last probe error.
Reset-Clock
$message = ''
try {
    [void](Wait-Hoi4WorldReadyState `
        -TargetProcessId 4242 `
        -TimeoutSeconds 30 `
        -Observe { param($r) throw 'CGameState singleton is null, unstable, or misaligned.' } `
        -TestAlive { $true } `
        -UtcNow $virtualNow -Sleep $virtualSleep)
} catch {
    $message = $_.Exception.Message
}
Assert-True -Condition ($message -match 'did not expose stable world singletons within 30 s') `
    -Message 'a process that never observed keeps the singleton diagnosis'
Assert-True -Condition ($message -match 'CGameState singleton is null') `
    -Message 'the last probe error is carried into the refusal'
Assert-True -Condition ($message -notmatch 'never became non-zero') `
    -Message 'the singleton refusal does not claim a gate was observed'

# =============================================================================
Write-Host 'case: an observation without the gate field is NOT readiness'
# An observer that stops reporting the gate must not read as a set gate, and
# must not read as a silent zero either -- both would restore the defect with
# every guard still green.
# It must also fail FAST: a missing field is a wiring fault, and spending the
# whole budget on it then blames the campaign for never entering play.
# RED WHEN: Get-Hoi4PlayerInputGate returns a default instead of throwing,
# Test-Hoi4PlayerInputGateSet stops routing through it, or the gate read moves
# back inside the retry catch.
Reset-Clock
$message = ''
try {
    [void](Wait-Hoi4WorldReadyState `
        -TargetProcessId 4242 `
        -TimeoutSeconds 30 `
        -Observe { param($r) [pscustomobject]@{ Pid = 4242; Speed = 5; Paused = $true } } `
        -TestAlive { $true } `
        -UtcNow $virtualNow -Sleep $virtualSleep)
} catch {
    $message = $_.Exception.Message
}
Assert-True -Condition ($message -match 'no PlayerInputGate field') `
    -Message 'a gate-less observation is rejected by name'
Assert-Equal -Expected 0 -Actual $script:Sleeps `
    -Message 'a wiring fault fails immediately instead of spending the budget'

$predicateThrew = $false
try {
    [void](Test-Hoi4PlayerInputGateSet -State ([pscustomobject]@{ Pid = 1 }))
} catch {
    $predicateThrew = $true
}
Assert-True -Condition $predicateThrew `
    -Message 'the readiness predicate itself refuses a gate-less observation'
Assert-True -Condition (-not (Test-Hoi4PlayerInputGateSet -State $null)) `
    -Message 'a null observation is not readiness'

# =============================================================================
Write-Host 'case: a target that exits during the wait is reported as exiting'
# RED WHEN: the liveness probe is dropped, which would turn a crashed game into
# a 600 s wait ending in the wrong diagnosis.
Reset-Clock
$script:Polls = 0
$message = ''
try {
    [void](Wait-Hoi4WorldReadyState `
        -TargetProcessId 4242 `
        -TimeoutSeconds 600 `
        -Observe { param($r) New-Observation -PlayerInputGate 0 } `
        -TestAlive { $script:Polls++; return ($script:Polls -le 3) } `
        -UtcNow $virtualNow -Sleep $virtualSleep)
} catch {
    $message = $_.Exception.Message
}
Assert-True -Condition ($message -match 'exited before the world became ready') `
    -Message 'a target that goes away is reported as having exited'

# =============================================================================
Write-Host 'case: progress is reported while waiting'
# RED WHEN: the progress callback is dropped, which is what makes a long wait
# indistinguishable from a hang in an unattended capture log.
Reset-Clock
$script:Progress = @()
$script:Polls = 0
[void](Wait-Hoi4WorldReadyState `
    -TargetProcessId 4242 `
    -TimeoutSeconds 600 `
    -Observe {
        param($r)
        $script:Polls++
        if ($script:Polls -le 8) { return (New-Observation -PlayerInputGate 0) }
        return (New-Observation -PlayerInputGate 1)
    } `
    -TestAlive { $true } `
    -ReportProgress { param($p) $script:Progress += , $p } `
    -Sleep $virtualSleep -UtcNow $virtualNow -ProgressIntervalSeconds 1)
Assert-True -Condition ($script:Progress.Count -ge 2) `
    -Message 'the wait reports progress while the gate is zero'
Assert-Equal -Expected 0 -Actual $script:Progress[0].PlayerInputGate `
    -Message 'progress carries the observed gate value'

# =============================================================================
Write-Host 'case: the scripts that run are wired to the wait that is tested'
# A correct library the harness bypasses is not a fix.
# RED WHEN: Wait-Hoi4WorldReady stops delegating, or the harness stops
# dot-sourcing the library.
$harness = Get-Content -LiteralPath (Join-Path $scriptDir 'Invoke-HeadlessRawCapture.ps1') -Raw
Assert-True -Condition ($harness -match "\.\s*\(Join-Path \`$scriptDir 'Hoi4WorldReady\.Gate\.ps1'\)") `
    -Message 'the harness dot-sources the world-ready gate library'
# The word boundary matters: without it the assertion also accepts a call to a
# DIFFERENT function whose name merely starts with this one.
Assert-True -Condition ($harness -match 'return \(Wait-Hoi4WorldReadyState\b') `
    -Message 'the harness world-ready wait delegates to the tested wait'
# And the old shape -- return the first observation whose singletons were
# coherent -- must not come back alongside it.
Assert-True -Condition ($harness -notmatch 'if \(\$null -ne \$state\) \{ return \$state \}') `
    -Message 'the harness does not still accept the first coherent observation as ready'

# The callbacks must CLOSE OVER the values they need. Without .GetNewClosure()
# the observer block resolves $TargetProcessId from the wait's own parameter by
# accident and $ExpectedCreationFiletimeUtc not at all, which under StrictMode
# turns every observation into a caught error and spends the whole budget. The
# replica case below proves the shape works; this proves the harness uses it.
# RED WHEN: .GetNewClosure() is dropped from either callback.
Assert-True `
    -Condition (@([regex]::Matches($harness, '\}\.GetNewClosure\(\)')).Count -ge 2) `
    -Message 'the harness world-ready callbacks capture their values with .GetNewClosure()'
# And the world-ready observation specifically -- matched through its own
# -TimeoutSeconds $RemainingSeconds, so this cannot be satisfied by one of the
# harness's other observation call sites -- must still hand the process epoch
# through, or the per-observation identity assertion silently stops being made.
Assert-True `
    -Condition ($harness -match
        '-TimeoutSeconds \$RemainingSeconds `\s*\r?\n\s*-ExpectedCreationFiletimeUtc \$ExpectedCreationFiletimeUtc') `
    -Message 'the world-ready observation still carries the process creation epoch'

# The observer must actually READ the offset and REPORT it. Asserting the field
# alone leaves the read replaceable with a constant.
$observer = Get-Content -LiteralPath (Join-Path $scriptDir 'Read-Hoi4GameState.ps1') -Raw
Assert-True -Condition ($observer -match '\$gameStatePlayerInputGateOffset = 0xA39L') `
    -Message 'the observer pins the gate offset at 0xA39'
Assert-True -Condition ($observer -match '\$playerInputGate = Read-RemoteByte') `
    -Message 'the observer reads the gate from engine memory'
Assert-True -Condition ($observer -match '\$gameState \+ \$gameStatePlayerInputGateOffset') `
    -Message 'the observer reads it at CGameState plus the pinned offset'
Assert-True -Condition ($observer -match 'PlayerInputGate = \[int\]\$playerInputGate') `
    -Message 'the observer reports the raw observed byte'

# Anti-drift: two scripts read the same engine byte from independently written
# literals. They must not diverge.
# RED WHEN: either literal changes without the other.
$control = Get-Content -LiteralPath (Join-Path $scriptDir 'Control-Hoi4Game.ps1') -Raw
Assert-True -Condition ($control -match '\$gameStatePlayerInputGateOffset = 0xA39L') `
    -Message 'the control helper pins the same gate offset'

# And the refusal this wait exists to STOP TRIPPING must itself be intact.
# Waiting for a condition is not permission to stop refusing when it is absent.
# RED WHEN: the refusal is deleted, or demoted from a throw.
Assert-True `
    -Condition ($control -match 'if \(\$playerInputGate -eq 0\) \{\s+throw \(') `
    -Message 'the control helper still refuses on a zero gate, with a throw'

# =============================================================================
Write-Host 'case: the PRODUCTION closure shape resolves in a clean 5.1 child'
# The cases above drive the wait with self-contained scriptblocks, so none of
# them exercises what the harness actually passes: closures built inside a
# script-scope function that CALL another script-scope function, which in turn
# reads a script-scope variable, handed to a dot-sourced library and invoked
# there. Review raised `.GetNewClosure()` scoping as a suspected production-only
# failure precisely because no test covered it. This runs that exact shape in a
# fresh `powershell.exe -File` child against the real library.
#
# RED WHEN: `.GetNewClosure()` is dropped from the harness callbacks (the
# captured epoch then fails to resolve under StrictMode), when the callback
# signature stops carrying what the observer needs, or if a future PowerShell
# really does stop resolving script-scope commands from a closure module.
$replicaDir = Join-Path ([IO.Path]::GetTempPath()) ("hoi4-wrg-" + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $replicaDir -Force | Out-Null
try {
    $libLiteral = "'" + (Join-Path $scriptDir 'Hoi4WorldReady.Gate.ps1').Replace("'", "''") + "'"
    $replica = @"
Set-StrictMode -Version Latest
`$ErrorActionPreference = 'Stop'
. $libLiteral

# A script-scope variable the observer function must still resolve when it is
# called from inside the closure.
`$readOnlyObserver = 'C:\some\Read-Hoi4GameState.ps1'
`$script:ObserveCalls = 0

function Invoke-BoundedHoi4Observation {
    param(
        [Parameter(Mandatory = `$true)][int]`$TargetProcessId,
        [ValidateRange(1, 120)][int]`$TimeoutSeconds = 30,
        [uint64]`$ExpectedCreationFiletimeUtc = 0
    )
    if ([string]::IsNullOrEmpty(`$readOnlyObserver)) {
        throw 'script-scope observer path did not resolve inside the closure'
    }
    `$script:ObserveCalls++
    `$gate = if (`$script:ObserveCalls -ge 3) { 1 } else { 0 }
    return [pscustomobject]@{
        Pid = `$TargetProcessId
        Epoch = `$ExpectedCreationFiletimeUtc
        PlayerInputGate = `$gate
    }
}

function Wait-Hoi4WorldReady {
    param(
        [Parameter(Mandatory = `$true)][int]`$TargetProcessId,
        [Parameter(Mandatory = `$true)][int]`$TimeoutSeconds,
        [Parameter(Mandatory = `$true)][uint64]`$ExpectedCreationFiletimeUtc
    )
    return (Wait-Hoi4WorldReadyState ``
        -TargetProcessId `$TargetProcessId ``
        -TimeoutSeconds `$TimeoutSeconds ``
        -Observe {
            param(`$RemainingSeconds)
            Invoke-BoundedHoi4Observation ``
                -TargetProcessId `$TargetProcessId ``
                -TimeoutSeconds `$RemainingSeconds ``
                -ExpectedCreationFiletimeUtc `$ExpectedCreationFiletimeUtc
        }.GetNewClosure() ``
        -TestAlive { `$null -ne (Get-Process -Id `$PID -ErrorAction SilentlyContinue) }.GetNewClosure() ``
        -Sleep { param(`$ms) } ``
        -ReportProgress { param(`$p) })
}

`$state = Wait-Hoi4WorldReady -TargetProcessId 4242 -TimeoutSeconds 60 ``
    -ExpectedCreationFiletimeUtc ([uint64]133700000000000000)
if (`$state.PlayerInputGate -ne 1) { throw "gate=`$(`$state.PlayerInputGate)" }
if (`$state.Pid -ne 4242) { throw "closure lost TargetProcessId: `$(`$state.Pid)" }
if (`$state.Epoch -ne 133700000000000000) { throw "closure lost the epoch: `$(`$state.Epoch)" }
if (`$script:ObserveCalls -ne 3) { throw "calls=`$(`$script:ObserveCalls)" }
Write-Host 'REPLICA-OK'
"@
    $replicaPath = Join-Path $replicaDir 'replica.ps1'
    Set-Content -LiteralPath $replicaPath -Value $replica -Encoding UTF8
    $child = & (Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe') `
        -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $replicaPath 2>&1 | Out-String
    Assert-True -Condition ($child -match 'REPLICA-OK') `
        -Message "the production closure shape resolves in a clean child (child said: $($child.Trim()))"
} finally {
    Remove-Item -LiteralPath $replicaDir -Recurse -Force -ErrorAction SilentlyContinue
}

# =============================================================================
Write-Host ''
if ($script:Failures.Count -gt 0) {
    Write-Host "FAILED: $($script:Failures.Count) assertion(s), $($script:Passed) passed"
    throw "Hoi4WorldReady gate tests failed: $($script:Failures -join '; ')"
}
Write-Host "OK: $($script:Passed) assertions passed"
