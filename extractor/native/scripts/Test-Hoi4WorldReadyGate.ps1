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
# ...and scopes that to the observations. Polling never sampled the engine
# BETWEEN observations, so an unscoped "it never became non-zero" asserts a
# continuous history the harness cannot know -- the owner returning to the main
# menu mid-wait produces exactly the entered-then-cleared history it would deny.
# RED WHEN: the scoping is dropped from the refusal wording.
Assert-True -Condition ($message -match 'never became non-zero in any observation taken') `
    -Message 'the refusal claims only what polling establishes'
# The refusal must not enumerate the histories that fit the evidence, because
# that set is not enumerable: the byte can be set and cleared any number of
# times while unsampled, including entirely before the first observation or
# entirely after the last. Two review rounds were spent adding a missing case to
# a list that could never be complete. What it must state is the only sound
# conclusion -- that the current value is unknown.
# RED WHEN: the message goes back to claiming a complete set of histories, or
# drops the unknown-current-value conclusion.
Assert-True -Condition ($message -match 'may have transitioned any number of times while unsampled') `
    -Message 'the refusal says the unsampled history is not enumerable'
Assert-True -Condition ($message -match 'its current value is unknown') `
    -Message 'the refusal states the sound conclusion about the current value'
# And it must not make a present-tense claim about the engine, which an
# unsampled set would falsify.
# RED WHEN: the refusal reverts to asserting what the toggle would do now.
Assert-True -Condition ($message -match 'is not a claim about the engine now') `
    -Message 'the refusal is attributed to the state last observed, not to the engine now'
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
Write-Host 'case: the PRODUCTION functions resolve and describe in clean 5.1 children'
# The cases above drive the wait with self-contained scriptblocks, so none of
# them exercises what the harness actually passes: closures built inside a
# script-scope function that CALL other script-scope functions, handed to a
# dot-sourced library and invoked there. `.GetNewClosure()` scoping was raised in
# review as a suspected production-only failure precisely because no test covered
# it, and t27 once spent its whole 600 s budget on exactly that.
#
# THE PRODUCTION FUNCTIONS THEMSELVES RUN HERE, lifted out of the harness by AST
# rather than hand-copied. A replica would keep passing while production lost its
# `-DescribeStall` wiring, which makes the RED WHEN below a fiction.
#
# AND IN BOTH INVOCATION MODES. `-File` makes a script's functions global, so a
# by-name call from inside a closure resolves there and the capture-by-value the
# harness does would look unnecessary. `-Command "& script"` keeps the script's
# own scope, which is the mode that broke t27 and the only one that can prove
# the captured variables are doing work.
#
# RED WHEN: `.GetNewClosure()` is dropped, when the harness stops passing
# -DescribeStall, when the describer's helpers are called by name instead of
# through captured values (the `-Command` child then fails), or when the
# selection, the log path or the banner verdict stops reaching the message.
$replicaDir = Join-Path ([IO.Path]::GetTempPath()) ("hoi4-wrg-" + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $replicaDir -Force | Out-Null
try {
    $harnessPath = Join-Path $scriptDir 'Invoke-HeadlessRawCapture.ps1'
    $replicaParseErrors = $null
    $replicaTokens = $null
    $replicaAst = [System.Management.Automation.Language.Parser]::ParseFile(
        $harnessPath, [ref]$replicaTokens, [ref]$replicaParseErrors)
    if ($replicaParseErrors.Count -gt 0) {
        throw "harness does not parse: $($replicaParseErrors[0].Message)"
    }
    $wanted = @(
        'Wait-Hoi4WorldReady',
        'Read-Hoi4SharedText',
        'Select-Hoi4CurrentSessionLog',
        'Get-Hoi4TextDigest',
        'Test-Hoi4SessionLaunchBanner',
        'New-Hoi4StallDescription'
    )
    $lifted = @{}
    foreach ($node in $replicaAst.FindAll(
            {
                param($n)
                $n -is [System.Management.Automation.Language.FunctionDefinitionAst]
            }, $true)) {
        if ($wanted -contains $node.Name -and -not $lifted.ContainsKey($node.Name)) {
            $lifted[$node.Name] = $node.Extent.Text
        }
    }
    $missingFns = @($wanted | Where-Object { -not $lifted.ContainsKey($_) })
    if ($missingFns.Count -gt 0) {
        throw ("the harness no longer defines: $($missingFns -join ', '). Either they " +
            'were renamed, or the stall-description path moved somewhere this case ' +
            'does not reach.')
    }
    $productionText = ($wanted | ForEach-Object { $lifted[$_] }) -join "`n`n"

    # A real log file, so Read-Hoi4SharedText's share mode and disposal run for
    # real. It carries no launch banner: the shape t44 left behind.
    $replicaLog = Join-Path $replicaDir 'game.log'
    Set-Content -LiteralPath $replicaLog -Encoding UTF8 -NoNewline -Value (
        "[08:25:35][no_game_date][defines.cpp:270]: 4469 defines loaded`n" +
        "[08:25:46][1936.01.01.12][history.cpp:165]: Executing History to 1936.1.1.12")

    $libLiteral = "'" + (Join-Path $scriptDir 'Hoi4WorldReady.Gate.ps1').Replace("'", "''") + "'"
    $logLiteral = "'" + $replicaLog.Replace("'", "''") + "'"
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

# --- lifted verbatim from Invoke-HeadlessRawCapture.ps1 ----------------------
$productionText
# --- end lifted ---------------------------------------------------------------

# The child's OWN pid: production's TestAlive really calls Get-Process, so a
# made-up id makes the wait exit with "the target exited" before anything here
# is exercised.
`$selfPid = `$PID

# FIRST: a healthy campaign. The describer must never run.
`$state = Wait-Hoi4WorldReady -TargetProcessId `$selfPid -TimeoutSeconds 60 ``
    -ExpectedCreationFiletimeUtc ([uint64]133700000000000000)
if (`$state.PlayerInputGate -ne 1) { throw "gate=`$(`$state.PlayerInputGate)" }
if (`$state.Pid -ne `$selfPid) { throw "closure lost TargetProcessId: `$(`$state.Pid)" }
if (`$state.Epoch -ne 133700000000000000) { throw "closure lost the epoch: `$(`$state.Epoch)" }
if (`$script:ObserveCalls -ne 3) { throw "calls=`$(`$script:ObserveCalls)" }

# SECOND: the stall. Singletons never come up and the real log on disk shows no
# session launch, so the timeout must carry the selection and the banner verdict.
# BOUNDED to 3 s: this case is about the closure resolving, not about the budget.
`$script:ObserveCalls = 0
function Invoke-BoundedHoi4Observation {
    param(
        [Parameter(Mandatory = `$true)][int]`$TargetProcessId,
        [ValidateRange(1, 120)][int]`$TimeoutSeconds = 30,
        [uint64]`$ExpectedCreationFiletimeUtc = 0
    )
    `$script:ObserveCalls++
    throw 'CInGameIdler singleton pair is null, incoherent, or misaligned.'
}
`$startedAt = [DateTime]::UtcNow
`$stalled = `$null
try {
    Wait-Hoi4WorldReady -TargetProcessId `$selfPid -TimeoutSeconds 3 ``
        -ExpectedCreationFiletimeUtc ([uint64]133700000000000000) ``
        -ContinueSelection ([pscustomobject]@{
            Title = 'Imperial Japan 1941.9.20.19'
            Filename = 'JAP_1941_09_20_17.hoi4'
        }) ``
        -GameLogPath $logLiteral ``
        -GameLogPriorLength 0 ``
        -GameLogPriorDigest ''
} catch {
    `$stalled = `$_.Exception.Message
}
`$elapsed = ([DateTime]::UtcNow - `$startedAt).TotalSeconds
if (`$elapsed -gt 40) { throw "the bounded wait ran `$elapsed s" }
if (`$stalled -notlike '*did not expose stable world singletons*') {
    throw "the real refusal was replaced: `$stalled"
}
foreach (`$fragment in @('Imperial Japan 1941.9.20.19', 'JAP_1941_09_20_17.hoi4',
        'NO campaign session')) {
    if (`$stalled -notlike "*`$fragment*") {
        throw "the stall description lost '`$fragment': `$stalled"
    }
}
Write-Host 'REPLICA-OK'
"@
    $replicaPath = Join-Path $replicaDir 'replica.ps1'
    Set-Content -LiteralPath $replicaPath -Value $replica -Encoding UTF8
    $ps = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'

    $child = & $ps -NoProfile -NonInteractive -ExecutionPolicy Bypass `
        -File $replicaPath 2>&1 | Out-String
    Assert-True -Condition ($child -match 'REPLICA-OK') `
        -Message "the production functions resolve and describe under -File (child said: $($child.Trim()))"

    # The mode that actually motivates the capture-by-value: run via -Command so
    # the script keeps its own scope and its functions are NOT global.
    $commandChild = & $ps -NoProfile -NonInteractive -ExecutionPolicy Bypass `
        -Command "& '$($replicaPath.Replace("'", "''"))'" 2>&1 | Out-String
    Assert-True -Condition ($commandChild -match 'REPLICA-OK') `
        -Message ("the production functions resolve and describe under -Command, " +
            "where script-scope functions are not global (child said: $($commandChild.Trim()))")
} finally {
    Remove-Item -LiteralPath $replicaDir -Recurse -Force -ErrorAction SilentlyContinue
}

# =============================================================================
Write-Host 'case: the stall description reaches the timeout that had no cause'
# THIS IS THE DEFECT (ledger #165). A game whose continue selection the engine
# declined sits at the frontend: no singletons, gate never set, and the wait
# spends its budget and then reports "did not expose stable world singletons",
# which reads as an offsets or process fault. t44 paid that twice and the real
# cause was only ever visible in the game's own log.
#
# RED WHEN: the describer is not consulted, its text is dropped from the
# message, or it is consulted on the wrong arm. Any of those puts the message
# back to the one that sent t44's diagnosis to the wrong place.
Reset-Clock
$script:DescribeCalls = 0
$timeoutMessage = $null
try {
    Wait-Hoi4WorldReadyState `
        -TargetProcessId 4242 `
        -TimeoutSeconds 300 `
        -Observe { param($r) throw 'CInGameIdler singleton pair is null.' } `
        -TestAlive { $true } `
        -DescribeStall {
            param($waited)
            $script:DescribeCalls++
            return "LAUNCHED ON 'Vichy France 1944' AFTER $waited s"
        } `
        -UtcNow $virtualNow -Sleep $virtualSleep
} catch {
    $timeoutMessage = $_.Exception.Message
}
Assert-Equal -Expected 1 -Actual $script:DescribeCalls `
    -Message 'the describer is consulted exactly once, when the budget runs out'
Assert-True -Condition ($timeoutMessage -like '*did not expose stable world singletons*') `
    -Message 'the original diagnosis is kept'
Assert-True -Condition ($timeoutMessage -like "*LAUNCHED ON 'Vichy France 1944' AFTER 300 s*") `
    -Message "the describer's text reaches the message, with the seconds waited"

# =============================================================================
Write-Host 'case: the describer can NEVER shorten or fail a wait'
# The previous two designs could, and both were wrong: refusing on a missing
# banner aborts a slow load, and refusing on "missing banner AND quiet log"
# aborts a silent deserialize. Refusing a healthy capture costs a game hour; a
# misleading timeout costs a reader ten minutes.
#
# RED WHEN: the describer is moved back into the poll loop, or its return value
# is ever allowed to become a throw. Either change makes the wait end early
# here, and both assertions fail.
Reset-Clock
$script:DescribeCalls = 0
$script:Polls = 0
$result = Wait-Hoi4WorldReadyState `
    -TargetProcessId 4242 `
    -TimeoutSeconds 600 `
    -Observe {
        param($r)
        $script:Polls++
        # Unreadable for well over any plausible grace window, then ready --
        # the healthy-but-slow campaign the earlier designs refused.
        if ($script:Polls -le 400) { throw 'CInGameIdler singleton pair is null.' }
        return (New-Observation -PlayerInputGate 1)
    } `
    -TestAlive { $true } `
    -DescribeStall {
        param($waited)
        $script:DescribeCalls++
        return 'this must never be reached'
    } `
    -UtcNow $virtualNow -Sleep $virtualSleep
Assert-Equal -Expected 1 -Actual $result.PlayerInputGate `
    -Message 'a campaign that is slow past any grace window still reaches readiness'
Assert-Equal -Expected 0 -Actual $script:DescribeCalls `
    -Message 'a wait that succeeds never consults the describer at all'

# =============================================================================
Write-Host 'case: a describer that THROWS cannot replace the real refusal'
# It is decoration on a refusal that is already happening, so its own failure
# must not become the reported cause.
#
# RED WHEN: the try/catch around the describer is removed (the describer's
# exception then replaces the timeout), or the failure is swallowed silently so
# nothing says the description was lost.
Reset-Clock
$timeoutMessage = $null
try {
    Wait-Hoi4WorldReadyState `
        -TargetProcessId 4242 `
        -TimeoutSeconds 120 `
        -Observe { param($r) throw 'CInGameIdler singleton pair is null.' } `
        -TestAlive { $true } `
        -DescribeStall { param($waited) throw 'the describer is broken' } `
        -UtcNow $virtualNow -Sleep $virtualSleep
} catch {
    $timeoutMessage = $_.Exception.Message
}
Assert-True -Condition ($timeoutMessage -like '*did not expose stable world singletons*') `
    -Message 'a throwing describer leaves the real refusal intact'
Assert-True -Condition ($timeoutMessage -like '*stall description failed: the describer is broken*') `
    -Message 'the describer failure is reported rather than swallowed'

# =============================================================================
Write-Host 'case: a wait that DID observe a session is not described'
# The description is about a launch that never announced itself. Once any
# observation succeeded the campaign demonstrably exists, and the other timeout
# arm already explains itself.
#
# RED WHEN: the `-not $sawObservation` guard on the describer is dropped, which
# would append launch speculation to a refusal that is really about the gate.
Reset-Clock
$script:DescribeCalls = 0
$timeoutMessage = $null
try {
    Wait-Hoi4WorldReadyState `
        -TargetProcessId 4242 `
        -TimeoutSeconds 120 `
        -Observe { param($r) New-Observation -PlayerInputGate 0 } `
        -TestAlive { $true } `
        -DescribeStall { param($waited) $script:DescribeCalls++; return 'must not appear' } `
        -UtcNow $virtualNow -Sleep $virtualSleep
} catch {
    $timeoutMessage = $_.Exception.Message
}
Assert-Equal -Expected 0 -Actual $script:DescribeCalls `
    -Message 'the describer is not consulted when observations succeeded'
Assert-True -Condition ($timeoutMessage -like '*in-game-session latch*') `
    -Message 'that arm keeps its own explanation'

# =============================================================================
Write-Host ''
if ($script:Failures.Count -gt 0) {
    Write-Host "FAILED: $($script:Failures.Count) assertion(s), $($script:Passed) passed"
    throw "Hoi4WorldReady gate tests failed: $($script:Failures -join '; ')"
}
Write-Host "OK: $($script:Passed) assertions passed"
