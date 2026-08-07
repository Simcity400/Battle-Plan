<#
.SYNOPSIS
Offline unit gate for Invoke-HeadlessRawCapture.ps1's decision logic.

.DESCRIPTION
The harness is 3.7k lines and, until this file, almost none of it could be
exercised without a live campaign. `Test-RecorderFinalizeProgressBound.ps1`
covers exactly one function; `Test-PassiveCapturePolicy.ps1` covers the
lifecycle mirror and a text census of the harness; `script_order_tests.rs`
covers source ORDER. Everything else -- parameter admission, the CPU-affinity
plan, receipt integer admission, the closed property set, the child-command
quoting, and the collector_record argument plan -- was only ever executed by a
capture, so a defect in any of them cost a game hour to find.

This runs that logic in about a second and never launches anything. The harness
is NEVER executed: each function under test is lifted out of the harness by AST
and re-evaluated on its own, exactly as the finalize-bound test does. Functions
that read the machine (`Get-Hoi4PhysicalCoreTopology`) are replaced by
synthetic doubles, and the harness is separately required to still DEFINE and
CALL the real one, so a rename cannot leave the double standing alone.

WHAT THIS DELIBERATELY DOES NOT COVER
  * The launch block's live half. `-DryRun` still needs hoi4.exe on disk, a
    settings.txt, and the machine-global settings guard; that path is
    Test-HeadlessCaptureSettingsGuard.ps1's. What IS covered here is the half
    that is a pure function of its inputs: the child command's PowerShell
    literal quoting, and the admission of the atomic WMI PID handoff.
  * `Confirm-CollectorTeardownReceipt` and the cleanup arbitration, which close
    over script-scope run state. Their ORDER is already pinned by
    `script_order_tests.rs` (`the_harness_admits_a_teardown_receipt_on_custody_
    alone`, `the_harness_status_verdict_survives_every_route_out`); duplicating
    that here would add a second census to keep in step and no new coverage.

.PARAMETER HarnessPath
The harness to test. Defaults to the sibling Invoke-HeadlessRawCapture.ps1.
Overridable so a reviewer can point the suite at a deliberately mutated COPY
and confirm the cases actually fail -- a test that has never been seen red is
an assertion about nothing.
#>
[CmdletBinding()]
param([string]$HarnessPath = '')

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
if ([string]::IsNullOrWhiteSpace($HarnessPath)) {
    $HarnessPath = Join-Path $scriptDir 'Invoke-HeadlessRawCapture.ps1'
}
if (-not (Test-Path -LiteralPath $HarnessPath -PathType Leaf)) {
    throw "Harness not found: $HarnessPath"
}

# --- extraction --------------------------------------------------------------

$parseErrors = $null
$tokens = $null
$harnessAst = [System.Management.Automation.Language.Parser]::ParseFile(
    $HarnessPath, [ref]$tokens, [ref]$parseErrors)
if ($parseErrors.Count -gt 0) {
    throw "harness does not parse: $(($parseErrors | ForEach-Object { $_.Message }) -join '; ')"
}

# KEYED BY NAME, AND A SECOND DEFINITION IS AN ERROR, NOT A TIE-BREAK.
#
# PowerShell resolves a call to the definition execution reached LAST, so a
# suite that silently kept the first would test one function while the harness
# ran another: adding a second, defective `New-RecorderArgumentPlan` above the
# call site leaves every case here green and ships different behaviour.
# (Codex review 019fdc8a-6a81-7422-af0f-82edb26a4e31, finding 1.)
$harnessFunctions = @{}
$duplicateDefinitions = [System.Collections.Generic.List[string]]::new()
foreach ($node in $harnessAst.FindAll(
        {
            param($n)
            $n -is [System.Management.Automation.Language.FunctionDefinitionAst]
        }, $true)) {
    if ($harnessFunctions.ContainsKey($node.Name)) {
        [void]$duplicateDefinitions.Add(
            "$($node.Name) (lines $($harnessFunctions[$node.Name].Extent.StartLineNumber) and $($node.Extent.StartLineNumber))")
        continue
    }
    $harnessFunctions[$node.Name] = $node
}
if ($duplicateDefinitions.Count -gt 0) {
    throw ('the harness defines these functions more than once: ' +
        ($duplicateDefinitions -join ', ') +
        '. PowerShell calls whichever definition execution reached last, so ' +
        'this suite cannot say which one ships.')
}

# Every function this file drives. Naming them up front means a rename fails
# here with the name, instead of surfacing later as "command not recognised"
# inside one case.
$underTest = @(
    'ConvertTo-NativeArgumentLine',
    'ConvertTo-PowerShellLiteral',
    'ConvertTo-ReceiptUInt32',
    'ConvertTo-ReceiptUInt64',
    'ConvertTo-ReceiptInt32',
    'Get-Utf8Sha256Hex',
    'Assert-ExactJsonPropertySet',
    'Get-SettingsGuardBaseException',
    'Get-SettingsGuardExceptionKind',
    'Get-SettingsGuardKey',
    'Get-Hoi4AffinityBitIndexes',
    'New-Hoi4AffinityMask',
    'ConvertTo-Hoi4AffinityUInt64',
    'Format-Hoi4AffinityMask',
    'Get-Hoi4CpuAffinityPlan',
    'Set-Hoi4ProcessAffinity',
    'New-RecorderArgumentPlan'
)
$missing = @($underTest | Where-Object { -not $harnessFunctions.ContainsKey($_) })
if ($missing.Count -gt 0) {
    throw ("the harness no longer defines: $($missing -join ', '). Either the " +
        'function was renamed -- in which case rename it here too -- or the ' +
        'behaviour it carried moved somewhere this suite does not reach.')
}

# Dot-sourced at SCRIPT scope on purpose. Dot-sourcing inside a helper would
# define these in that helper's scope and every case below would fail to find
# them.
. ([scriptblock]::Create(
    (($underTest | ForEach-Object { $harnessFunctions[$_].Extent.Text }) -join "`n")))

# The harness's own parameter admission, with no body attached. Binding runs
# every Validate* attribute before a script body executes, so this probe
# answers exactly the question a caller asks -- "would the harness accept these
# arguments?" -- without the harness being able to do anything.
$parameterProbe = [scriptblock]::Create(
    $harnessAst.ParamBlock.Extent.Text + "`nreturn `$PSBoundParameters")
$probeParameterNames = @(
    $harnessAst.ParamBlock.Parameters |
        ForEach-Object { [string]$_.Name.VariablePath.UserPath })

# --- assertions --------------------------------------------------------------

$script:cases = 0
$script:failures = [System.Collections.Generic.List[string]]::new()

function Assert-True {
    param([bool]$Condition, [Parameter(Mandatory = $true)][string]$Message)
    $script:cases++
    if (-not $Condition) { [void]$script:failures.Add($Message) }
}

function Assert-Equal {
    param([AllowNull()]$Expected, [AllowNull()]$Actual, [string]$Message)
    $script:cases++
    $ok = if ($null -eq $Expected) { $null -eq $Actual }
          elseif ($null -eq $Actual) { $false }
          else { [string]$Expected -ceq [string]$Actual }
    if (-not $ok) {
        [void]$script:failures.Add("$Message (expected '$Expected', got '$Actual')")
    }
}

function Assert-SequenceEqual {
    param(
        [AllowEmptyCollection()][string[]]$Expected,
        [AllowEmptyCollection()][string[]]$Actual,
        [string]$Message
    )
    $script:cases++
    if ($Expected.Count -ne $Actual.Count) {
        [void]$script:failures.Add(
            "${Message}: length $($Actual.Count), expected $($Expected.Count)`n" +
            "  expected: $($Expected -join ' ')`n  actual:   $($Actual -join ' ')")
        return
    }
    for ($i = 0; $i -lt $Expected.Count; $i++) {
        if ($Expected[$i] -cne $Actual[$i]) {
            [void]$script:failures.Add(
                "${Message}: element $i is '$($Actual[$i])', expected '$($Expected[$i])'")
            return
        }
    }
}

# A refusal is only worth asserting if the reason is the one intended, so the
# message is matched too: `-CpuAffinityCores 1` and an unbuildable mask both
# throw, and a test that accepted either would pass on the wrong bug.
function Assert-Refused {
    param(
        [Parameter(Mandatory = $true)][scriptblock]$Action,
        [Parameter(Mandatory = $true)][string]$MessagePattern,
        [Parameter(Mandatory = $true)][string]$Message
    )
    $script:cases++
    try {
        $null = & $Action
    } catch {
        $text = [string]$_.Exception.Message
        if ($text -notmatch $MessagePattern) {
            [void]$script:failures.Add(
                "${Message}: refused, but for the wrong reason: '$text' does not match /$MessagePattern/")
        }
        return
    }
    [void]$script:failures.Add("${Message}: was accepted, but must be refused.")
}

function Assert-Accepted {
    param(
        [Parameter(Mandatory = $true)][scriptblock]$Action,
        [Parameter(Mandatory = $true)][string]$Message
    )
    $script:cases++
    try {
        $null = & $Action
    } catch {
        [void]$script:failures.Add("${Message}: refused with '$($_.Exception.Message)'")
    }
}

# --- 1. parameter admission ---------------------------------------------------
#
# Each range below is a decision the capture depends on, and every one of them
# is currently enforced ONLY by the attribute. `-Speed 4` reaching a graded run
# would produce an hour count nobody could compare with any other trial;
# `-SchedulerQueueCapacity 2` would silently capture under a scheduler shape the
# READY contract does not describe (PRODUCTION_DOMAIN_QUEUE_CAPACITY_V1 = 1).

foreach ($accepted in @(
    @{ Args = @{ Speed = 5 };                        Why = 'the only graded speed' },
    # 1..4 bind. Speed 5 is NOT a binding constraint -- the harness body throws
    # on anything else (`if ($Speed -ne 5) { throw`, censused by
    # Test-PassiveCapturePolicy.ps1). Recorded here so nobody reads the
    # attribute as the enforcement and then widens it.
    @{ Args = @{ Speed = 1 };                        Why = 'binds; refused by the body' },
    @{ Args = @{ Speed = 4 };                        Why = 'binds; refused by the body' },
    @{ Args = @{ DurationSeconds = 10 };             Why = 'shortest admitted window' },
    @{ Args = @{ DurationSeconds = 86400 };          Why = 'longest admitted window' },
    @{ Args = @{ MinGameHours = 0 };                 Why = 'the deliberate vacuous floor' },
    @{ Args = @{ SchedulerQueueCapacity = 1 };       Why = 'the production queue depth' },
    @{ Args = @{ SchedulerPollMilliseconds = 25 };   Why = 'slowest admitted poll' },
    @{ Args = @{ StreamCompression = 'zstd' };       Why = 'default spool codec' },
    @{ Args = @{ StreamCompression = 'none' };       Why = 'raw-bytes debugging codec' },
    @{ Args = @{ CpuAffinityCores = 0 };             Why = 'the affinity off switch' },
    @{ Args = @{ CpuAffinityCores = 1 };             Why = 'bound here, refused by the plan' },
    @{ Args = @{ ReaderConcurrency = 64 };           Why = 'widest reader fan-out' },
    @{ Args = @{ FinalizeCeilingSeconds = 300 };     Why = 'lowest finalize ceiling' },
    @{ Args = @{ SettleSeconds = 0 };                Why = 'deliberate race reproduction' }
)) {
    $splat = $accepted.Args
    Assert-Accepted `
        -Action { & $parameterProbe @splat }.GetNewClosure() `
        -Message "parameter admission rejected $($splat.Keys -join ',')=$($splat.Values -join ',') ($($accepted.Why))"
}

foreach ($refused in @(
    @{ Args = @{ Speed = 6 };                      Name = 'Speed' },
    @{ Args = @{ Speed = 0 };                      Name = 'Speed' },
    @{ Args = @{ DurationSeconds = 9 };            Name = 'DurationSeconds' },
    @{ Args = @{ DurationSeconds = 86401 };        Name = 'DurationSeconds' },
    @{ Args = @{ MinGameHours = -1 };              Name = 'MinGameHours' },
    @{ Args = @{ SampleMilliseconds = 0 };         Name = 'SampleMilliseconds' },
    @{ Args = @{ SchedulerQueueCapacity = 0 };     Name = 'SchedulerQueueCapacity' },
    @{ Args = @{ SchedulerQueueCapacity = 2 };     Name = 'SchedulerQueueCapacity' },
    @{ Args = @{ SchedulerPollMilliseconds = 0 };  Name = 'SchedulerPollMilliseconds' },
    @{ Args = @{ SchedulerPollMilliseconds = 26 }; Name = 'SchedulerPollMilliseconds' },
    @{ Args = @{ ReaderConcurrency = 0 };          Name = 'ReaderConcurrency' },
    @{ Args = @{ ReaderConcurrency = 65 };         Name = 'ReaderConcurrency' },
    @{ Args = @{ StreamCompression = 'gzip' };     Name = 'StreamCompression' },
    @{ Args = @{ CpuAffinityCores = 1025 };        Name = 'CpuAffinityCores' },
    @{ Args = @{ WorldReadyTimeoutSeconds = 29 };  Name = 'WorldReadyTimeoutSeconds' },
    @{ Args = @{ RecorderOverheadSeconds = 29 };   Name = 'RecorderOverheadSeconds' },
    @{ Args = @{ FinalizeNoProgressSeconds = 14 }; Name = 'FinalizeNoProgressSeconds' },
    @{ Args = @{ FinalizeCeilingSeconds = 299 };   Name = 'FinalizeCeilingSeconds' },
    @{ Args = @{ SettleSeconds = 901 };            Name = 'SettleSeconds' },
    @{ Args = @{ InjectionReadyTimeoutSeconds = 4 }; Name = 'InjectionReadyTimeoutSeconds' },
    @{ Args = @{ WitnessIntervalSeconds = 1 };     Name = 'WitnessIntervalSeconds' },
    @{ Args = @{ StallEscalationSamples = 1 };     Name = 'StallEscalationSamples' },
    @{ Args = @{ StallEscalationSeconds = 4 };     Name = 'StallEscalationSeconds' }
)) {
    $splat = $refused.Args
    $name = $refused.Name
    # The parameter must EXIST before its refusal means anything. A deleted or
    # renamed parameter makes the probe throw "A parameter cannot be found that
    # matches parameter name 'Speed'" -- a message that contains the name and
    # would otherwise be counted as a working range check on a parameter that is
    # gone.
    Assert-True `
        -Condition ($probeParameterNames -ccontains $name) `
        -Message "the harness no longer declares -$name; its refusal below would be a missing-parameter error, not a range check"
    Assert-Refused `
        -Action { & $parameterProbe @splat }.GetNewClosure() `
        -MessagePattern ([regex]::Escape($name)) `
        -Message "parameter admission accepted $name=$($splat[$name])"
}

# --- 2. receipt integer admission --------------------------------------------
#
# These decide whether a JSON number from a receipt is allowed to become a PID,
# a FILETIME or an hour. "Canonical" is the whole point: a float, a numeric
# STRING, or a value that does not round-trip its own text must be refused
# rather than coerced, because every one of those coercions has a plausible
# wrong answer (58600.0 -> 58600 hides a producer emitting a float PID).
#
# MEASURED LIMIT, so a green run here is not read as more than it is. Two
# clauses in the u32/u64 admission cannot be reached and no case below claims
# to: swapping `NumberStyles::None` for `AllowLeadingSign`, and dropping the
# `$text -cne $parsed.ToString(...)` round-trip, both leave this suite passing.
# The type gate above them admits only typed integers, and no typed integer
# stringifies with a sign the range check would then accept, or with text that
# does not round-trip. They are defence against a shape the gate already
# excludes. The i32 path is different -- its `AllowLeadingSign` IS load-bearing
# and is covered.

foreach ($case in @(
    @{ Value = [int32]7;                 Expected = 7 },
    @{ Value = [int64]7;                 Expected = 7 },
    @{ Value = [byte]7;                  Expected = 7 },
    @{ Value = [decimal]7;               Expected = 7 },
    @{ Value = [int64]0;                 Expected = 0 },
    @{ Value = [int64]4294967295;        Expected = 4294967295 }
)) {
    Assert-Equal `
        -Expected $case.Expected `
        -Actual (ConvertTo-ReceiptUInt32 -Value $case.Value -Field 'probe') `
        -Message "ConvertTo-ReceiptUInt32 mis-admitted $($case.Value.GetType().Name) $($case.Value)"
}
foreach ($case in @(
    @{ Value = '7';            Why = 'a numeric string is not a JSON integer' },
    @{ Value = [double]7.0;    Why = 'a JSON float is not an integer even at .0' },
    @{ Value = [double]7.5;    Why = 'a fractional value' },
    @{ Value = [decimal]7.5;   Why = 'a fractional decimal' },
    @{ Value = [int32]-1;      Why = 'negative into u32' },
    @{ Value = [int64]4294967296; Why = 'one past u32' },
    @{ Value = $null;          Why = 'absent' },
    @{ Value = $true;          Why = 'a boolean' }
)) {
    Assert-Refused `
        -Action { ConvertTo-ReceiptUInt32 -Value $case.Value -Field 'probe' }.GetNewClosure() `
        -MessagePattern 'probe' `
        -Message "ConvertTo-ReceiptUInt32 admitted $($case.Why)"
}

# THE TYPE, not just the value. Assert-Equal compares as text, so a converter
# that returned the canonical STRING instead of the parsed number would satisfy
# every value case above and every fixture replay below -- and then a caller
# doing `[uint32]$pid -eq 0` or arithmetic on the result would be comparing a
# string. The declared return type is the contract.
# (Codex review 019fdc8a-6a81-7422-af0f-82edb26a4e31, finding 5.)
Assert-True `
    -Condition ((ConvertTo-ReceiptUInt32 -Value ([int64]7) -Field 'probe') -is [uint32]) `
    -Message 'ConvertTo-ReceiptUInt32 does not return a [uint32]'
Assert-True `
    -Condition ((ConvertTo-ReceiptUInt64 -Value ([int64]7) -Field 'probe') -is [uint64]) `
    -Message 'ConvertTo-ReceiptUInt64 does not return a [uint64]'
Assert-True `
    -Condition ((ConvertTo-ReceiptInt32 -Value ([int64]7) -Field 'probe') -is [int32]) `
    -Message 'ConvertTo-ReceiptInt32 does not return an [int32]'

Assert-Equal `
    -Expected ([uint64]::MaxValue) `
    -Actual (ConvertTo-ReceiptUInt64 -Value ([uint64]::MaxValue) -Field 'probe') `
    -Message 'ConvertTo-ReceiptUInt64 lost the top of the u64 range'
Assert-Equal `
    -Expected 134305757425608784 `
    -Actual (ConvertTo-ReceiptUInt64 -Value ([int64]134305757425608784) -Field 'probe') `
    -Message 'ConvertTo-ReceiptUInt64 mis-admitted a real process-creation FILETIME'
Assert-Refused `
    -Action { ConvertTo-ReceiptUInt64 -Value ([int64]-1) -Field 'probe' } `
    -MessagePattern 'probe' `
    -Message 'ConvertTo-ReceiptUInt64 admitted a negative FILETIME'

Assert-Equal -Expected -5 `
    -Actual (ConvertTo-ReceiptInt32 -Value ([int32]-5) -Field 'probe') `
    -Message 'ConvertTo-ReceiptInt32 refused a signed value it must accept'
Assert-Equal -Expected 60832272 `
    -Actual (ConvertTo-ReceiptInt32 -Value ([int64]60832272) -Field 'probe') `
    -Message 'ConvertTo-ReceiptInt32 mis-admitted a real clock_hour_raw'
Assert-Refused `
    -Action { ConvertTo-ReceiptInt32 -Value ([int64]2147483648) -Field 'probe' } `
    -MessagePattern 'probe' `
    -Message 'ConvertTo-ReceiptInt32 admitted one past i32'

# --- 3. the closed property set ----------------------------------------------

$handoff = [pscustomobject]@{
    pid                           = 58600
    process_creation_filetime_utc = 134305757425608784
    executable_path               = 'X:\SteamLibrary\steamapps\common\Hearts of Iron IV\hoi4.exe'
}
$handoffProperties = @('pid', 'process_creation_filetime_utc', 'executable_path')
Assert-Accepted `
    -Action {
        Assert-ExactJsonPropertySet -Value $handoff -Expected $handoffProperties -Label 'probe'
    } `
    -Message 'the WMI atomic handoff shape is no longer admitted by its own property set'
Assert-Refused `
    -Action {
        Assert-ExactJsonPropertySet `
            -Value $handoff `
            -Expected ($handoffProperties + 'extra') `
            -Label 'probe'
    } `
    -MessagePattern 'probe has 3 properties; expected exactly 4' `
    -Message 'a handoff missing a required property was admitted'
$handoffWithExtra = $handoff | Select-Object *, @{ Name = 'smuggled'; Expression = { 1 } }
Assert-Refused `
    -Action {
        Assert-ExactJsonPropertySet `
            -Value $handoffWithExtra -Expected $handoffProperties -Label 'probe'
    } `
    -MessagePattern 'probe has 4 properties; expected exactly 3' `
    -Message 'a handoff carrying an unexpected property was admitted'
# ORDINAL, not case-insensitive. A producer emitting `Pid` is a different
# producer, and the whole point of a closed set is that it says so.
$handoffMiscased = [pscustomobject]@{
    Pid                           = 58600
    process_creation_filetime_utc = 1
    executable_path               = 'x'
}
Assert-Refused `
    -Action {
        Assert-ExactJsonPropertySet `
            -Value $handoffMiscased -Expected $handoffProperties -Label 'probe'
    } `
    -MessagePattern 'property set differs at index' `
    -Message 'the closed property set compares names case-insensitively'

# --- 4. digests ---------------------------------------------------------------

Assert-Equal `
    -Expected 'ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad' `
    -Actual (Get-Utf8Sha256Hex -Value 'abc') `
    -Message 'Get-Utf8Sha256Hex disagrees with the SHA-256 "abc" vector'
# The encoder must emit no BOM: the roster digest computed here is compared
# against one the recorder computes over the same bytes, and three leading
# bytes would make every comparison fail with no clue why. The expected
# values are the published SHA-256 vectors for the raw bytes (61 62 63 and
# C3 A9), computed independently of PowerShell — the dead in-harness byte
# hasher that used to compute them was deleted by owner ruling 2026-08-08.
Assert-Equal `
    -Expected 'ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad' `
    -Actual (Get-Utf8Sha256Hex -Value 'abc') `
    -Message 'Get-Utf8Sha256Hex is emitting a byte-order mark'
# U+00E9 is one char, two UTF-8 bytes and two UTF-16 bytes with different
# values, so this separates UTF-8 from the .NET default encoding.
Assert-Equal `
    -Expected '4a99557e4033c3539de2eb65472017cad5f9557f7a0625a09f1c3f6e2ba69c4c' `
    -Actual (Get-Utf8Sha256Hex -Value ([char]0x00E9)) `
    -Message 'Get-Utf8Sha256Hex is not encoding non-ASCII as UTF-8'

# --- 5. child-command and native-command quoting ------------------------------
#
# `ConvertTo-PowerShellLiteral` builds the -EncodedCommand the WMI launch helper
# runs. Asserting the STRING it returns would only restate the implementation,
# so each case is evaluated as PowerShell and required to give the input back:
# that is the property the launch path actually needs, and it is the one a
# switch to double quotes would break.
foreach ($value in @(
    'plain',
    "it's",
    "''double''",
    'X:\SteamLibrary\steamapps\common\Hearts of Iron IV\hoi4.exe',
    'a "quoted" word',
    '$(Get-Process)',
    '$env:PATH',
    "back`tick`u{60}here",
    "line1`nline2",
    'trailing\'
)) {
    $literal = ConvertTo-PowerShellLiteral -Value $value
    $roundTripped = & ([scriptblock]::Create($literal))
    Assert-Equal -Expected $value -Actual $roundTripped `
        -Message "ConvertTo-PowerShellLiteral does not round-trip $([regex]::Escape($value))"
}
# An empty value is refused at binding rather than quoted, and that is the
# right answer for this function: every caller passes a command line, a
# directory or a receipt path, and an empty one of those is a bug upstream --
# emitting `''` into the child command would carry it into the launch instead.
Assert-Refused `
    -Action { ConvertTo-PowerShellLiteral -Value '' } `
    -MessagePattern 'empty string' `
    -Message 'ConvertTo-PowerShellLiteral quoted an empty value instead of refusing it'

# The MSVCRT command-line rules, which decide what collector_record actually
# receives. Each row is a shape the recorder argument plan can produce.
foreach ($case in @(
    @{ In = @('plain');                        Out = 'plain' },
    @{ In = @('a', 'b');                       Out = 'a b' },
    @{ In = @('has space');                    Out = '"has space"' },
    @{ In = @('C:\dir with space\');           Out = '"C:\dir with space\\"' },
    @{ In = @('no-space-trailing\');           Out = 'no-space-trailing\' },
    @{ In = @('say "hi"');                     Out = '"say \"hi\""' },
    @{ In = @('a\"b c');                       Out = '"a\\\"b c"' },
    # An EMPTY argument must survive as an empty argument. Dropping it shifts
    # every later flag one position left, so `--capture-id "" --session-id X`
    # would reach the collector as `--capture-id --session-id X` -- a capture
    # running under a session id it never chose.
    @{ In = @('');                             Out = '""' },
    @{ In = @('--capture-id', '', '--session-id', 'x');
       Out = '--capture-id "" --session-id x' }
)) {
    Assert-Equal `
        -Expected $case.Out `
        -Actual (ConvertTo-NativeArgumentLine -Arguments $case.In) `
        -Message "ConvertTo-NativeArgumentLine mis-quoted @($($case.In -join '|'))"
}

# --- 6. settings-guard classification ----------------------------------------
#
# The guard makes a user's settings.txt read-only and must put it back. Which
# recovery it chooses turns entirely on this classification, and getting it
# wrong is how a run either clobbers another run's sentinel or refuses to
# release its own.
$sharingViolation = [System.IO.IOException]::new('sharing', -2147024864)  # win32 32
$lockViolation = [System.IO.IOException]::new('lock', -2147024863)        # win32 33
foreach ($case in @(
    @{ Ex = [System.IO.FileNotFoundException]::new('f');      Kind = 'not_found' },
    @{ Ex = [System.IO.DirectoryNotFoundException]::new('d'); Kind = 'not_found' },
    @{ Ex = [System.UnauthorizedAccessException]::new('u');   Kind = 'access_denied' },
    @{ Ex = $sharingViolation;                                Kind = 'sharing_violation' },
    @{ Ex = $lockViolation;                                   Kind = 'sharing_violation' },
    @{ Ex = [System.IO.IOException]::new('other', -2147024891); Kind = 'other' },
    @{ Ex = [System.InvalidOperationException]::new('i');     Kind = 'other' }
)) {
    Assert-Equal `
        -Expected $case.Kind `
        -Actual (Get-SettingsGuardExceptionKind -Exception $case.Ex) `
        -Message "Get-SettingsGuardExceptionKind mis-classified $($case.Ex.GetType().Name)"
}
# Wrapped exceptions are the normal case: every one of these arrives inside a
# MethodInvocationException from a .NET call, and a classifier reading only the
# outer type would answer 'other' for all of them.
$wrapped = [System.Exception]::new(
    'outer', [System.Exception]::new('middle', $sharingViolation))
Assert-Equal -Expected 'sharing_violation' `
    -Actual (Get-SettingsGuardExceptionKind -Exception $wrapped) `
    -Message 'Get-SettingsGuardExceptionKind does not look through wrapping exceptions'
Assert-Equal -Expected $sharingViolation.Message `
    -Actual (Get-SettingsGuardBaseException -Exception $wrapped).Message `
    -Message 'Get-SettingsGuardBaseException did not reach the innermost exception'

# The sentinel key is derived from an UPPERCASED identity, so the same file
# reached through two spellings claims the same sentinel. If it did not, two
# concurrent runs could each believe they held the guard on the same file.
Assert-Equal `
    -Expected 'b5d4045c3f466fa91fe2cc6abe79232a1a57cdf104f7a26e716e0a1e2789df78' `
    -Actual (Get-SettingsGuardKey -Identity 'ABC') `
    -Message 'Get-SettingsGuardKey is not SHA-256 over the uppercased identity'
Assert-Equal `
    -Expected (Get-SettingsGuardKey -Identity 'ABC') `
    -Actual (Get-SettingsGuardKey -Identity 'abc') `
    -Message 'Get-SettingsGuardKey is case-sensitive; one file would take two sentinels'
Assert-True `
    -Condition ((Get-SettingsGuardKey -Identity 'a') -cne (Get-SettingsGuardKey -Identity 'b')) `
    -Message 'Get-SettingsGuardKey collides across distinct identities'

# --- 7. affinity mask arithmetic ---------------------------------------------
#
# PowerShell has no unsigned shift that survives bit 63, which is why these go
# through bytes. Bit 63 is not hypothetical: it is logical processor 63 on a
# 64-thread machine, and a mask built with `1 -shl 63` there is NEGATIVE.
Assert-Equal -Expected 0 -Actual (New-Hoi4AffinityMask -BitIndexes @()) `
    -Message 'an empty bit list did not produce an empty mask'
Assert-Equal -Expected 1 -Actual (New-Hoi4AffinityMask -BitIndexes @(0)) `
    -Message 'bit 0'
Assert-Equal -Expected ([uint64]9223372036854775808) `
    -Actual (New-Hoi4AffinityMask -BitIndexes @(63)) `
    -Message 'bit 63 did not survive as an unsigned mask'
Assert-Equal -Expected ([uint64]::MaxValue) `
    -Actual (New-Hoi4AffinityMask -BitIndexes (0..63)) `
    -Message 'a full 64-processor mask'
foreach ($bad in @(64, -1, 100)) {
    Assert-Refused `
        -Action { New-Hoi4AffinityMask -BitIndexes @($bad) }.GetNewClosure() `
        -MessagePattern 'outside the 64-bit affinity mask' `
        -Message "New-Hoi4AffinityMask accepted logical processor $bad"
}
Assert-SequenceEqual `
    -Expected @('0', '63') `
    -Actual @(Get-Hoi4AffinityBitIndexes -Mask ([uint64]9223372036854775809) |
        ForEach-Object { [string]$_ }) `
    -Message 'Get-Hoi4AffinityBitIndexes lost an end of the mask'
foreach ($bits in @(@(), @(0), @(63), @(0, 1, 8, 31, 32, 63), @(2, 4, 6))) {
    $mask = New-Hoi4AffinityMask -BitIndexes $bits
    Assert-SequenceEqual `
        -Expected @($bits | ForEach-Object { [string]$_ }) `
        -Actual @(Get-Hoi4AffinityBitIndexes -Mask $mask | ForEach-Object { [string]$_ }) `
        -Message "mask round trip lost bits @($($bits -join ','))"
}
# A read-back mask arrives as an IntPtr, and one with bit 63 set is a NEGATIVE
# Int64 that a direct [uint64] cast throws on.
Assert-Equal -Expected ([uint64]::MaxValue) `
    -Actual (ConvertTo-Hoi4AffinityUInt64 -Value ([IntPtr]::new(-1))) `
    -Message 'ConvertTo-Hoi4AffinityUInt64 mishandled an all-ones read-back'
Assert-Equal -Expected 255 `
    -Actual (ConvertTo-Hoi4AffinityUInt64 -Value ([IntPtr]::new(255))) `
    -Message 'ConvertTo-Hoi4AffinityUInt64 mishandled an ordinary read-back'
Assert-Equal -Expected '0x000000000000000f' `
    -Actual (Format-Hoi4AffinityMask -Mask ([uint64]15)) `
    -Message 'Format-Hoi4AffinityMask is not a lowercase 16-digit hex mask'

# --- 8. the CPU affinity plan -------------------------------------------------
#
# The plan is the 4c floor emulation. Its one job is to select n PHYSICAL cores
# and produce the mask of their logical processors -- never an index range.
# Every case below is driven over a synthetic topology, because the real
# topology is this machine's and a test that agreed with it would only ever
# describe this machine.

# The double stands in for the OS reader. The harness must still define and
# CALL the real one, or the double would be covering an absence.
Assert-True -Condition ($harnessFunctions.ContainsKey('Get-Hoi4PhysicalCoreTopology')) `
    -Message 'the harness no longer defines Get-Hoi4PhysicalCoreTopology; the affinity plan double covers nothing'
$planCalls = @(
    $harnessFunctions['Get-Hoi4CpuAffinityPlan'].Body.FindAll(
        {
            param($n)
            $n -is [System.Management.Automation.Language.CommandAst] -and
            $n.GetCommandName() -eq 'Get-Hoi4PhysicalCoreTopology'
        }, $true))
Assert-True -Condition ($planCalls.Count -ge 1) `
    -Message 'Get-Hoi4CpuAffinityPlan no longer reads the OS topology; the plan under test is not the plan that ships'

# THE REAL READER IS RUN ONCE, BEFORE THE DOUBLE REPLACES IT.
#
# A double that supplies EfficiencyClass, Group, Mask, LogicalProcessors and
# PrimaryLogicalProcessor itself agrees with the synthetic model however wrong
# the real parser is: the PROCESSOR_RELATIONSHIP offsets are hand-decoded
# (efficiency class at +9, GroupCount at +30, GROUP_AFFINITY mask at +32, group
# at +40), and a wrong offset there produces a plausible-looking record that
# masks the wrong processors. So the real reader runs against this machine's
# actual topology first, and is checked for the invariants that hold on ANY
# machine rather than for this one's numbers -- a test that agreed with this
# 16-processor box would only ever describe this box.
# (Codex review 019fdc8a-6a81-7422-af0f-82edb26a4e31, finding 3.)
#
# MEASURED LIMIT, and it is the HARDWARE's, not this suite's. Two of the four
# hand-decoded offsets cannot be falsified on a single-group, single-class
# machine, and both mutations were run:
#   * reading EfficiencyClass as an Int32 at +9 instead of a byte gives the same
#     0 when every core is one class and the reserved run behind it is zero;
#   * reading the processor group at +36 instead of +40 lands inside the
#     GROUP_AFFINITY mask, whose high bytes are also 0 on a <=32-processor
#     single-group box.
# The mask offset (+32) and the primary-logical-processor rule ARE falsified
# here, by the overlap and lowest-bit checks. Closing the other two needs a
# hybrid or multi-group machine to run on -- a hardware requirement, not a
# missing assertion. The SELECTION rules those fields drive are covered against
# synthetic hybrid and multi-group topologies below.
$topologyTypeBlock = @(
    $harnessAst.FindAll(
        {
            param($n)
            $n -is [System.Management.Automation.Language.IfStatementAst] -and
            $n.Extent.Text -match 'CpuTopologyNativeMethods' -and
            $n.Extent.Text -match 'Add-Type'
        }, $true))
Assert-Equal -Expected 1 -Actual $topologyTypeBlock.Count `
    -Message 'the harness no longer declares the GetLogicalProcessorInformationEx interop; the real topology reader cannot be exercised'
if ($topologyTypeBlock.Count -eq 1) {
    . ([scriptblock]::Create(
        $topologyTypeBlock[0].Extent.Text + "`n" +
        $harnessFunctions['Get-Hoi4PhysicalCoreTopology'].Extent.Text))

    $realTopology = @(Get-Hoi4PhysicalCoreTopology)
    Assert-True -Condition ($realTopology.Count -ge 1) `
        -Message 'the real topology reader returned no physical cores on a machine that has some'
    # Every logical processor on the machine, exactly once. This is the check a
    # wrong mask offset fails: a garbage KAFFINITY either overlaps another
    # core's bits or fails to sum to the processor count.
    $seenLogical = [System.Collections.Generic.HashSet[int]]::new()
    $overlapping = $false
    $malformed = [System.Collections.Generic.List[string]]::new()
    foreach ($core in $realTopology) {
        if ([uint64]$core.Mask -eq 0) {
            [void]$malformed.Add("core $($core.CoreIndex) has an empty mask")
        }
        $derived = @(Get-Hoi4AffinityBitIndexes -Mask ([uint64]$core.Mask))
        if (@($core.LogicalProcessors).Count -ne $derived.Count) {
            [void]$malformed.Add("core $($core.CoreIndex) logical processors disagree with its own mask")
        }
        if ([int]$core.PrimaryLogicalProcessor -ne [int]$derived[0]) {
            [void]$malformed.Add("core $($core.CoreIndex) primary logical processor is not the lowest bit of its mask")
        }
        # EfficiencyClass is a byte read at a hand-decoded offset; a wrong
        # offset here reads part of the reserved run or the group count.
        if ([int]$core.EfficiencyClass -lt 0 -or [int]$core.EfficiencyClass -gt 255) {
            [void]$malformed.Add("core $($core.CoreIndex) efficiency class $($core.EfficiencyClass) is not a byte")
        }
        if ([int]$core.Group -lt 0 -or [int]$core.Group -gt 1023) {
            [void]$malformed.Add("core $($core.CoreIndex) processor group $($core.Group) is outside the Windows range")
        }
        foreach ($logical in $derived) {
            if (-not $seenLogical.Add([int]$logical)) { $overlapping = $true }
        }
    }
    Assert-Equal -Expected 0 -Actual $malformed.Count `
        -Message "the real topology reader produced malformed core records: $($malformed -join '; ')"
    Assert-True -Condition (-not $overlapping) `
        -Message 'the real topology reader gave one logical processor to two physical cores; the GROUP_AFFINITY mask offset is wrong'
    Assert-Equal `
        -Expected ([int][Environment]::ProcessorCount) `
        -Actual $seenLogical.Count `
        -Message 'the real topology reader did not account for every logical processor the OS reports'
    Assert-True `
        -Condition (@($realTopology | ForEach-Object { [int]$_.CoreIndex } | Sort-Object -Unique).Count -eq $realTopology.Count) `
        -Message 'the real topology reader repeated a core index'
}

# From here the double replaces it: the plan's SELECTION rules need machines
# this one is not. A later definition wins at call time, so this must come
# after the real reader has been exercised.
$script:syntheticTopology = @()
function Get-Hoi4PhysicalCoreTopology { return $script:syntheticTopology }

function New-SyntheticCore {
    param(
        [Parameter(Mandatory = $true)][int]$CoreIndex,
        [Parameter(Mandatory = $true)][int]$EfficiencyClass,
        [Parameter(Mandatory = $true)][int]$Group,
        [Parameter(Mandatory = $true)][int[]]$LogicalProcessors
    )
    # Built through the harness's own mask arithmetic rather than restated, so
    # a topology fixture cannot disagree with the code that reads it.
    $mask = New-Hoi4AffinityMask -BitIndexes $LogicalProcessors
    return [pscustomobject]@{
        CoreIndex               = $CoreIndex
        EfficiencyClass         = $EfficiencyClass
        Group                   = $Group
        Mask                    = $mask
        LogicalProcessors       = @(Get-Hoi4AffinityBitIndexes -Mask $mask)
        PrimaryLogicalProcessor = @(Get-Hoi4AffinityBitIndexes -Mask $mask)[0]
    }
}

# Sibling numbering is a CONVENTION, not a contract. This layout -- core i owns
# logical i and i+8 -- is the one an index-range mask gets wrong: masking
# logical 0..7 there selects EIGHT half-cores across all eight cores instead of
# four whole ones, which is a different machine that still looks right in Task
# Manager. Every homogeneous case below uses it for that reason.
$script:syntheticTopology = @(
    0..7 | ForEach-Object {
        New-SyntheticCore -CoreIndex $_ -EfficiencyClass 0 -Group 0 `
            -LogicalProcessors @($_, ($_ + 8))
    }
)
$plan = Get-Hoi4CpuAffinityPlan -CoreCount 4 -OneThreadPerCore $false
Assert-Equal -Expected '0x0000000000000f0f' -Actual $plan.MaskHex `
    -Message 'the 4c/8t plan selected an index range instead of four whole cores'
Assert-SequenceEqual -Expected @('0', '1', '2', '3') `
    -Actual @($plan.SelectedCoreIndexes | ForEach-Object { [string]$_ }) `
    -Message '4c/8t selected core indexes'
Assert-SequenceEqual -Expected @('0', '1', '2', '3', '8', '9', '10', '11') `
    -Actual @($plan.LogicalProcessors | ForEach-Object { [string]$_ }) `
    -Message '4c/8t selected logical processors'
Assert-Equal -Expected 8 -Actual $plan.MachinePhysicalCores `
    -Message 'the plan miscounted the machine physical cores'
Assert-Equal -Expected 16 -Actual $plan.MachineLogicalProcessors `
    -Message 'the plan miscounted the machine logical processors'
Assert-Equal -Expected 0 -Actual $plan.Group -Message '4c/8t processor group'

$plan = Get-Hoi4CpuAffinityPlan -CoreCount 4 -OneThreadPerCore $true
Assert-Equal -Expected '0x000000000000000f' -Actual $plan.MaskHex `
    -Message 'the 4c/4t plan did not take exactly one sibling per core'
Assert-SequenceEqual -Expected @('0', '1', '2', '3') `
    -Actual @($plan.LogicalProcessors | ForEach-Object { [string]$_ }) `
    -Message '4c/4t selected logical processors'

# Hybrid, majority-E. Four cores drawn from both classes is not a machine
# anyone ships and the number could not be compared with anything, so the
# selection must stay inside one class -- the one with the most cores.
$script:syntheticTopology = @(
    (0..3 | ForEach-Object {
        New-SyntheticCore -CoreIndex $_ -EfficiencyClass 1 -Group 0 `
            -LogicalProcessors @(($_ * 2), (($_ * 2) + 1))
    }) +
    (0..7 | ForEach-Object {
        New-SyntheticCore -CoreIndex ($_ + 4) -EfficiencyClass 0 -Group 0 `
            -LogicalProcessors @($_ + 8)
    })
)
$plan = Get-Hoi4CpuAffinityPlan -CoreCount 4 -OneThreadPerCore $false
Assert-Equal -Expected 0 -Actual $plan.EfficiencyClass `
    -Message 'the hybrid plan did not prefer the class with the most cores'
Assert-SequenceEqual -Expected @('4', '5', '6', '7') `
    -Actual @($plan.SelectedCoreIndexes | ForEach-Object { [string]$_ }) `
    -Message 'the hybrid plan crossed a class boundary'

# Hybrid, equal counts. The tie goes to the more performant class, because a
# higher EfficiencyClass is the faster one.
$script:syntheticTopology = @(
    (0..3 | ForEach-Object {
        New-SyntheticCore -CoreIndex $_ -EfficiencyClass 1 -Group 0 `
            -LogicalProcessors @(($_ * 2), (($_ * 2) + 1))
    }) +
    (0..3 | ForEach-Object {
        New-SyntheticCore -CoreIndex ($_ + 4) -EfficiencyClass 0 -Group 0 `
            -LogicalProcessors @($_ + 8)
    })
)
$plan = Get-Hoi4CpuAffinityPlan -CoreCount 4 -OneThreadPerCore $false
Assert-Equal -Expected 1 -Actual $plan.EfficiencyClass `
    -Message 'a class-count tie did not go to the more performant class'
Assert-Equal -Expected '0x00000000000000ff' -Actual $plan.MaskHex `
    -Message 'the tie-break plan mask'

Assert-Refused `
    -Action { Get-Hoi4CpuAffinityPlan -CoreCount 5 -OneThreadPerCore $false } `
    -MessagePattern 'exceeds the largest homogeneous core group' `
    -Message 'the plan spanned two efficiency classes to reach the requested count'
Assert-Refused `
    -Action { Get-Hoi4CpuAffinityPlan -CoreCount 1 -OneThreadPerCore $false } `
    -MessagePattern 'needs at least 2 physical cores' `
    -Message 'a single-core plan was accepted; the switch exists to measure competition for cores'
Assert-Refused `
    -Action { Get-Hoi4CpuAffinityPlan -CoreCount 0 -OneThreadPerCore $false } `
    -MessagePattern 'needs at least 2 physical cores' `
    -Message 'a zero-core plan was accepted'

# -PriorMaskOut is declared nullable and the harness calls it with a literal
# $null in two places (the recorder pin and the game-mask restore). A [ref]
# type on that parameter makes those calls throw AT BINDING -- t38 lost its
# graded window to exactly that, mid-capture, with the recorder already
# launched. The dead PID keeps every case on this side of a real process: the
# function must get PAST binding and its own [ref] admission, and then fail on
# the process lookup, never on the parameter.
$script:deadPid = 0x7ffffffc
while ($null -ne (Get-Process -Id $script:deadPid -ErrorAction SilentlyContinue)) {
    $script:deadPid -= 4
}
Assert-Refused `
    -Action { Set-Hoi4ProcessAffinity -ProcessId $script:deadPid -Mask 0xff `
        -Label 'unit' -ExpectedCreationFiletimeUtc $null -PriorMaskOut $null } `
    -MessagePattern 'Cannot find a process' `
    -Message 'a literal $null PriorMaskOut did not survive parameter binding (the t38 graded-window kill)'
$script:priorCell = $null
Assert-Refused `
    -Action { Set-Hoi4ProcessAffinity -ProcessId $script:deadPid -Mask 0xff `
        -Label 'unit' -ExpectedCreationFiletimeUtc $null `
        -PriorMaskOut ([ref]$script:priorCell) } `
    -MessagePattern 'Cannot find a process' `
    -Message 'a real [ref] cell no longer binds to PriorMaskOut'
Assert-Refused `
    -Action { Set-Hoi4ProcessAffinity -ProcessId $script:deadPid -Mask 0xff `
        -Label 'unit' -ExpectedCreationFiletimeUtc $null -PriorMaskOut 42 } `
    -MessagePattern 'must be a \[ref\] cell' `
    -Message 'a non-ref PriorMaskOut was accepted; the prior mask would be written into a copy nobody reads'

# Group-blind API, fail closed. Process.ProcessorAffinity carries no processor
# group and targets the process's PRIMARY group, so a plan whose cores live in
# group 1 would put group 1's bit NUMBERS onto group 0's processors -- and the
# numeric read-back would confirm it.
$script:syntheticTopology = @(
    0..7 | ForEach-Object {
        New-SyntheticCore -CoreIndex $_ -EfficiencyClass 0 -Group 1 `
            -LogicalProcessors @($_, ($_ + 8))
    }
)
Assert-Refused `
    -Action { Get-Hoi4CpuAffinityPlan -CoreCount 4 -OneThreadPerCore $false } `
    -MessagePattern 'carries no group and silently targets' `
    -Message 'a non-zero processor group was masked as if it were group 0'
$script:syntheticTopology = @(
    (0..3 | ForEach-Object {
        New-SyntheticCore -CoreIndex $_ -EfficiencyClass 0 -Group 0 `
            -LogicalProcessors @($_, ($_ + 4))
    }) +
    (0..3 | ForEach-Object {
        New-SyntheticCore -CoreIndex ($_ + 4) -EfficiencyClass 0 -Group 1 `
            -LogicalProcessors @($_, ($_ + 4))
    })
)
Assert-Refused `
    -Action { Get-Hoi4CpuAffinityPlan -CoreCount 4 -OneThreadPerCore $false } `
    -MessagePattern 'carries no group and silently targets' `
    -Message 'a multi-group machine was masked with a single group-blind mask'

# --- 9. the collector_record argument plan ------------------------------------
#
# The whole capture configuration, asserted argument for argument. A flag lost
# here is not a crash: it is a capture that runs, seals, and answers a different
# question than the one asked.
$planCatalogs = [ordered]@{
    '--static-templates-equipment-catalog' = 'C:\defs\templates.json'
    '--static-localisation-join'           = 'C:\defs\loc.json'
}
$planArgs = @{
    GamePid                   = 58600
    CollectorDll              = 'C:\rel\collector.dll'
    OutputDirectory           = 'C:\runs\t99'
    DurationArgument          = '60'
    SampleMilliseconds        = 1
    MinGameHours              = 1
    StreamCompression         = 'zstd'
    SchedulerPollMilliseconds = 2
    SchedulerQueueCapacity    = 1
    ReaderConcurrency         = 12
    SessionId                 = 'c660d85d-d96d-4dcc-bda6-4e1910f7d866'
    CaptureId                 = 'headless-raw-capture-20260807_212901'
    ScheduledDomains          = @('session_header_live', 'fuel_live')
    StaticCatalogs            = $planCatalogs
}
$expectedPlan = @(
    '--pid', '58600',
    '--dll', 'C:\rel\collector.dll',
    '--out-dir', 'C:\runs\t99',
    '--duration-s', '60',
    '--sample-ms', '1',
    '--min-game-hours', '1',
    '--stream-format', 'compact',
    '--stream-compression', 'zstd',
    '--compact-country-frame',
    '--read-only-scheduler',
    '--read-only-scheduler-poll-ms', '2',
    '--read-only-scheduler-queue-cap', '1',
    '--reader-concurrency', '12',
    '--session-id', 'c660d85d-d96d-4dcc-bda6-4e1910f7d866',
    '--capture-id', 'headless-raw-capture-20260807_212901',
    '--scheduled-domain', 'session_header_live',
    '--scheduled-domain', 'fuel_live',
    '--static-templates-equipment-catalog', 'C:\defs\templates.json',
    '--static-localisation-join', 'C:\defs\loc.json'
)
Assert-SequenceEqual `
    -Expected $expectedPlan `
    -Actual @(New-RecorderArgumentPlan @planArgs) `
    -Message 'the collector_record argument plan changed'

Assert-SequenceEqual `
    -Expected ($expectedPlan + @('--skip-host-fold', '--keep-spool-after-seal')) `
    -Actual @(New-RecorderArgumentPlan @planArgs -SkipHostFold $true -KeepSpoolAfterSeal $true) `
    -Message 'the optional plan flags are missing or out of order'
Assert-SequenceEqual `
    -Expected ($expectedPlan + @('--keep-spool-after-seal')) `
    -Actual @(New-RecorderArgumentPlan @planArgs -KeepSpoolAfterSeal $true) `
    -Message '-KeepSpoolAfterSeal alone'
# The spool codec is a value, not a switch: a plan that quietly always sent
# 'zstd' would satisfy every case above that did not look.
$uncompressedArgs = $planArgs.Clone()
$uncompressedArgs['StreamCompression'] = 'none'
Assert-SequenceEqual `
    -Expected ($expectedPlan -replace '^zstd$', 'none') `
    -Actual @(New-RecorderArgumentPlan @uncompressedArgs) `
    -Message 'the spool codec does not reach the collector'
$badCodecArgs = $planArgs.Clone()
$badCodecArgs['StreamCompression'] = 'gzip'
Assert-Refused `
    -Action { New-RecorderArgumentPlan @badCodecArgs } `
    -MessagePattern 'StreamCompression' `
    -Message 'the plan accepted a spool codec the collector does not implement'

# An empty roster does not fail at the collector -- it selects the eight-domain
# compatibility set, and the run seals looking successful.
$emptyRosterArgs = $planArgs.Clone()
$emptyRosterArgs['ScheduledDomains'] = @()
Assert-Refused `
    -Action { New-RecorderArgumentPlan @emptyRosterArgs } `
    -MessagePattern 'nonempty scheduled-domain roster' `
    -Message 'the plan accepted an empty domain roster'

# Catalog ORDER is load-bearing: these are flag/value PAIRS in an argument
# vector, and a plain hashtable would order them by hash bucket.
$reorderedCatalogs = [ordered]@{
    '--static-localisation-join'           = 'C:\defs\loc.json'
    '--static-templates-equipment-catalog' = 'C:\defs\templates.json'
}
$reorderedArgs = $planArgs.Clone()
$reorderedArgs['StaticCatalogs'] = $reorderedCatalogs
$reorderedPlan = @(New-RecorderArgumentPlan @reorderedArgs)
Assert-Equal -Expected '--static-localisation-join' `
    -Actual $reorderedPlan[$reorderedPlan.Count - 4] `
    -Message 'the static-catalog flags did not keep their declared order'

# The plan is inside the graded window, so it may carry nothing that drives the
# client and nothing the passive scheduler retired.
foreach ($forbidden in @('--hour-latch', '--profile', '--early-launch-exe')) {
    Assert-True `
        -Condition (-not (@(New-RecorderArgumentPlan @planArgs) -ccontains $forbidden)) `
        -Message "the collector_record argument plan carries '$forbidden'"
}

# The plan's empty-roster refusal is the BACKSTOP, and it fires inside the
# graded window. The roster override is refused earlier, in the harness body,
# before the settings guard is applied and before anything is launched -- and
# that check is not in any function this file can extract, so it is pinned
# structurally instead. Without it, `-ScheduledDomainOverride ','` splits to
# nothing and reaches a collector that answers an empty schedule with its
# eight-domain compatibility set.
# The CONDITION is what is pinned, not the throw. A census that only found the
# message would go green on `if ($false) { throw ... }` -- the same
# census-cannot-see-reachability miss Test-PassiveCapturePolicy.ps1 documents
# for function definitions, and the reason that file switched to the parser.
$overrideGuards = @(
    $harnessAst.FindAll(
        {
            param($n)
            $n -is [System.Management.Automation.Language.IfStatementAst] -and
            $n.Extent.Text -match 'ScheduledDomainOverride was supplied but names no domain'
        }, $true) |
    Where-Object {
        $_.Clauses[0].Item1.Extent.Text -match
            '\$scheduledDomains\.Count\s+-eq\s+0'
    })
Assert-Equal -Expected 1 -Actual $overrideGuards.Count `
    -Message 'the harness no longer refuses an explicit roster override that names no domain (the refusal must be reached by an actual emptiness test, not merely present)'
$gradedWindowOpens = $harnessAst.Extent.Text.IndexOf(
    '$gradedWindowStarted = $true', [StringComparison]::Ordinal)
Assert-True -Condition ($gradedWindowOpens -ge 0) `
    -Message 'could not locate the graded window opening in the harness'
if ($overrideGuards.Count -eq 1 -and $gradedWindowOpens -ge 0) {
    Assert-True `
        -Condition ($overrideGuards[0].Extent.StartOffset -lt $gradedWindowOpens) `
        -Message 'the roster-override refusal moved into or past the graded window; a run that cannot work would still open one'
}

# Defined before it is called, for the same reason the finalize bound is: a
# function only exists once execution reaches its definition.
$planDefinitionLine = $harnessFunctions['New-RecorderArgumentPlan'].Extent.StartLineNumber
$planCallSites = @(
    $harnessAst.FindAll(
        {
            param($n)
            $n -is [System.Management.Automation.Language.CommandAst] -and
            $n.GetCommandName() -eq 'New-RecorderArgumentPlan'
        }, $true))
Assert-Equal -Expected 1 -Actual $planCallSites.Count `
    -Message 'the recorder argument plan is built somewhere other than its one call site'
if ($planCallSites.Count -eq 1) {
    Assert-True `
        -Condition ($planDefinitionLine -lt $planCallSites[0].Extent.StartLineNumber) `
        -Message 'New-RecorderArgumentPlan is defined at or after its call site'

    # THE CALL SITE'S OWN BINDINGS, not just the function's behaviour.
    #
    # Every case above drives the plan with its own correct splat, so all of
    # them stay green if the HARNESS passes the wrong variable:
    # `-ReaderConcurrency $SchedulerQueueCapacity` keeps the single call site,
    # satisfies every plan assertion, and sends `--reader-concurrency 1` on a
    # capture that asked for 12. The parameter-to-variable map is therefore
    # pinned here. (Codex review 019fdc8a-6a81-7422-af0f-82edb26a4e31,
    # finding 2.)
    $expectedBindings = [ordered]@{
        GamePid                   = 'gamePid'
        CollectorDll              = 'collectorDll'
        OutputDirectory           = 'OutDir'
        DurationArgument          = 'durationArg'
        SampleMilliseconds        = 'SampleMilliseconds'
        MinGameHours              = 'MinGameHours'
        StreamCompression         = 'StreamCompression'
        SchedulerPollMilliseconds = 'SchedulerPollMilliseconds'
        SchedulerQueueCapacity    = 'SchedulerQueueCapacity'
        ReaderConcurrency         = 'ReaderConcurrency'
        SessionId                 = 'sessionId'
        CaptureId                 = 'captureId'
        ScheduledDomains          = 'scheduledDomains'
        StaticCatalogs            = 'staticCatalogs'
        SkipHostFold              = 'SkipHostFold'
        KeepSpoolAfterSeal        = 'KeepSpoolAfterSeal'
    }
    $elements = @($planCallSites[0].CommandElements)
    $observedBindings = [ordered]@{}
    for ($i = 1; $i -lt $elements.Count; $i++) {
        if ($elements[$i] -isnot
            [System.Management.Automation.Language.CommandParameterAst]) {
            continue
        }
        $parameterName = [string]$elements[$i].ParameterName
        $argument = $elements[$i].Argument
        if ($null -eq $argument -and ($i + 1) -lt $elements.Count) {
            $argument = $elements[$i + 1]
        }
        if ($null -eq $argument) {
            $observedBindings[$parameterName] = '<no argument>'
            continue
        }
        # One variable per argument, whatever wraps it: `$OutDir`,
        # `([bool]$SkipHostFold)`. Two variables in one argument means the call
        # site is computing something, which this map cannot describe and must
        # not silently accept.
        $variables = @(
            [regex]::Matches([string]$argument.Extent.Text, '\$([A-Za-z_][A-Za-z0-9_]*)') |
                ForEach-Object { $_.Groups[1].Value })
        $observedBindings[$parameterName] = if ($variables.Count -eq 1) {
            $variables[0]
        } else {
            "<$($variables.Count) variables: $($argument.Extent.Text)>"
        }
    }
    # The SET, not the order: reordering named parameters is a no-op in
    # PowerShell, and failing a gate over it would be friction with no safety
    # behind it. What must not change is which parameters are passed and what
    # each one is bound to.
    Assert-SequenceEqual `
        -Expected @($expectedBindings.Keys | Sort-Object) `
        -Actual @($observedBindings.Keys | Sort-Object) `
        -Message 'the recorder argument plan call site does not pass exactly the expected parameter set'
    foreach ($parameterName in $expectedBindings.Keys) {
        $expectedVariable = [string]$expectedBindings[$parameterName]
        $observedVariable = if ($observedBindings.Contains($parameterName)) {
            [string]$observedBindings[$parameterName]
        } else {
            '<absent>'
        }
        Assert-Equal `
            -Expected $expectedVariable `
            -Actual $observedVariable `
            -Message "the harness binds -$parameterName to the wrong value; a capture would run under a setting nobody asked for"
    }
}

# --- 10. receipt-fixture replay ----------------------------------------------
#
# Lane EQ's dialect fixture holds real CAPTURE_READY_V2 receipts from t35 and
# t36. `Test-PassiveCapturePolicy.ps1` replays them through the LIFECYCLE
# mirror; this replays them through the HARNESS's own admission helpers, which
# are a separate implementation and were never run against a real receipt.
$fixturePath = Join-Path $scriptDir 'PassiveCaptureReady.ReceiptDialects.fixture.v1.json'
if (-not (Test-Path -LiteralPath $fixturePath -PathType Leaf)) {
    throw "receipt-dialect fixture is missing: $fixturePath"
}
# PRECONDITIONS, not cases. Nothing a harness edit can do makes the fixture the
# wrong shape, so counting these as harness assertions inflates the case count
# with checks that cannot fail for the reason this file exists.
# (Codex review 019fdc8a-6a81-7422-af0f-82edb26a4e31, finding 4.)
$fixture = Get-Content -LiteralPath $fixturePath -Raw | ConvertFrom-Json
if ([string]$fixture.schema -cne
    'hoi4-passive-capture-ready-receipt-dialects-fixture-v1') {
    throw "the receipt-dialect fixture changed schema: $($fixture.schema)"
}
$fixtureCases = @($fixture.cases)
if ($fixtureCases.Count -lt 3) {
    throw "the receipt-dialect fixture covers $($fixtureCases.Count) dialects, not three."
}

$u32Fields = @(
    'pid', 'requested_duration_ms', 'readiness_flags', 'validated_domain_count',
    'scheduler_queue_capacity', 'scheduler_worker_count', 'allowed_hook_count',
    'active_hook_count')
$u64Fields = @('process_creation_filetime_utc', 'clock_observation_ordinal')
$i32Fields = @('clock_hour_raw')

foreach ($case in $fixtureCases) {
    $name = [string]$case.name
    $receipt = $case.receipt
    foreach ($field in $u32Fields) {
        Assert-Accepted `
            -Action {
                ConvertTo-ReceiptUInt32 -Value $receipt.$field -Field $field
            }.GetNewClosure() `
            -Message "harness u32 admission refused real receipt '$name' field '$field'"
    }
    foreach ($field in $u64Fields) {
        Assert-Accepted `
            -Action {
                ConvertTo-ReceiptUInt64 -Value $receipt.$field -Field $field
            }.GetNewClosure() `
            -Message "harness u64 admission refused real receipt '$name' field '$field'"
    }
    foreach ($field in $i32Fields) {
        Assert-Accepted `
            -Action {
                ConvertTo-ReceiptInt32 -Value $receipt.$field -Field $field
            }.GetNewClosure() `
            -Message "harness i32 admission refused real receipt '$name' field '$field'"
    }

    # WHAT THIS DOES AND DOES NOT PROVE. `$ownProperties` comes from the receipt
    # being checked, so the accepting case cannot catch a receipt carrying a
    # field the harness has no name for -- an unknown property joins BOTH sides.
    # It is kept for the narrower thing it does prove: the helper admits a real
    # 36-property object without a spurious failure, whatever order the
    # properties arrive in. Naming the expected set here instead would mean
    # keeping a third copy of the READY vocabulary in step with the contract and
    # the lifecycle mirror -- the drift this suite exists to avoid -- and
    # Test-PassiveCapturePolicy.ps1 already asserts the closed READY set against
    # its own explicit list. What IS proved unaided is the pair of refusals
    # below and the shuffled/duplicated sets, none of which the receipt supplies.
    # (Codex review 019fdc8a-6a81-7422-af0f-82edb26a4e31, finding 4.)
    $ownProperties = @($receipt.PSObject.Properties.Name)
    Assert-Accepted `
        -Action {
            Assert-ExactJsonPropertySet -Value $receipt -Expected $ownProperties -Label $name
        }.GetNewClosure() `
        -Message "real receipt '$name' is not admitted by its own property set"
    # Order-insensitivity is the helper's own claim (it sorts both sides), and a
    # shuffled expectation is the only way to test it.
    $shuffled = @($ownProperties | Sort-Object -Descending)
    Assert-Accepted `
        -Action {
            Assert-ExactJsonPropertySet -Value $receipt -Expected $shuffled -Label $name
        }.GetNewClosure() `
        -Message "the closed property set depends on the order the expectation is written in ('$name')"
    # Same COUNT, different content: the count check alone would pass this.
    $substituted = @(@($ownProperties | Select-Object -Skip 1) + 'unheard_of_field')
    Assert-Refused `
        -Action {
            Assert-ExactJsonPropertySet -Value $receipt -Expected $substituted -Label $name
        }.GetNewClosure() `
        -MessagePattern 'property set differs at index' `
        -Message "real receipt '$name' was admitted against a same-size property set with a substituted name"
    $shortSet = @($ownProperties | Select-Object -Skip 1)
    Assert-Refused `
        -Action {
            Assert-ExactJsonPropertySet -Value $receipt -Expected $shortSet -Label $name
        }.GetNewClosure() `
        -MessagePattern ([regex]::Escape($name)) `
        -Message "real receipt '$name' was admitted against a property set one short"
    Assert-Refused `
        -Action {
            Assert-ExactJsonPropertySet `
                -Value $receipt `
                -Expected ($ownProperties + 'unheard_of_field') `
                -Label $name
        }.GetNewClosure() `
        -MessagePattern ([regex]::Escape($name)) `
        -Message "real receipt '$name' was admitted against a property set one long"

    # A field re-typed on the wire is the failure this admission exists for, so
    # the same real receipt is re-offered with one field spelled as a string.
    $mutated = $receipt | ConvertTo-Json -Depth 8 -Compress | ConvertFrom-Json
    $mutated.pid = [string]$receipt.pid
    Assert-Refused `
        -Action { ConvertTo-ReceiptUInt32 -Value $mutated.pid -Field 'pid' }.GetNewClosure() `
        -MessagePattern 'pid' `
        -Message "a stringified pid was admitted from receipt '$name'"
}

# A fixture PRECONDITION, and only that. Rebuilding these digests is the
# LIFECYCLE mirror's job and Test-PassiveCapturePolicy.ps1 does it; asserting
# here that three digest fields merely exist proved nothing about the harness
# and was counted as if it did.
# (Codex review 019fdc8a-6a81-7422-af0f-82edb26a4e31, finding 4.)
$pinnedDigests = @(
    $fixtureCases |
        Where-Object { $null -ne $_.expected_ready_payload_sha256 } |
        ForEach-Object { [string]$_.expected_ready_payload_sha256 })
if ($pinnedDigests.Count -lt 3) {
    throw "the receipt-dialect fixture pins $($pinnedDigests.Count) rebuilt payload digests, not three."
}

# --- verdict ------------------------------------------------------------------

if ($script:failures.Count -gt 0) {
    foreach ($failure in $script:failures) {
        [Console]::Error.WriteLine("FAIL: $failure")
    }
    [Console]::Error.WriteLine(
        "$($script:failures.Count) of $($script:cases) harness unit cases failed.")
    exit 1
}

# JSON, not an object: every consumer runs this in a child `powershell.exe
# -File` and pipes stdout through ConvertFrom-Json. A bare [pscustomobject]
# reaching a child's stdout is rendered by the default FORMATTER, which
# ConvertFrom-Json rejects with `Invalid JSON primitive` -- the exact trap
# Test-PassiveCapturePolicy.ps1 documents.
([pscustomobject]@{
    Schema  = 'hoi4-headless-raw-capture-unit-test-v1'
    Status  = 'pass'
    Cases   = $script:cases
    Harness = $HarnessPath
} | ConvertTo-Json -Depth 4 -Compress)
