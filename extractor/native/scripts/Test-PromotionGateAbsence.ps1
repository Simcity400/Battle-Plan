# Does the promotion gate refuse a capture because a field is ABSENT?
#
# PowerShell coerces $null to zero, so `[uint32](Get-...Property $row 'x')` on a
# manifest that never published `x` yields 0 -- and every promotion clause that
# demands a zero therefore PASSED on missing data. The per-domain clause
# required `effective_failure_status_flags_or`, which the manifest did not
# publish at all, so the gate approved every domain of every capture for want of
# evidence.
#
# This proves the coercion is real (so the test is not asserting a hypothetical)
# and that `Get-ProductionPromotionRequired*` refuses each way a field can fail
# to be a usable integer.
#
# Run:
#   powershell.exe -NoProfile -ExecutionPolicy Bypass -File Test-PromotionGateAbsence.ps1
# The -ExecutionPolicy Bypass affects this child process only; this host's
# machine policy is Restricted and stays that way.

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

. (Join-Path $PSScriptRoot 'ProductionCapture.Promotion.ps1')

$script:Failures = [System.Collections.Generic.List[string]]::new()
$script:Cases = 0

function Assert-True {
    param([Parameter(Mandatory = $true)][bool]$Condition,
          [Parameter(Mandatory = $true)][string]$Because)
    $script:Cases++
    if (-not $Condition) { $script:Failures.Add($Because) }
}

function Assert-Throws {
    param([Parameter(Mandatory = $true)][scriptblock]$Action,
          [Parameter(Mandatory = $true)][string]$Because)
    $script:Cases++
    try {
        & $Action | Out-Null
        $script:Failures.Add("did not throw: $Because")
    }
    catch {
        # expected
    }
}

# ---------------------------------------------------------------------------
# 1. The trap is real. If this ever stops holding, the helpers below are
#    unnecessary -- but it holds, and it is why they exist.
# ---------------------------------------------------------------------------
$empty = [pscustomobject]@{ present = 3 }
Assert-True -Condition ([uint32](Get-ProductionPromotionProperty $empty 'absent') -eq 0) `
    -Because 'PRECONDITION: [uint32] of an absent property is expected to be 0'
Assert-True -Condition ([int64](Get-ProductionPromotionProperty $empty 'absent') -eq 0) `
    -Because 'PRECONDITION: [int64] of an absent property is expected to be 0'
Assert-True `
    -Condition (([uint32](Get-ProductionPromotionProperty $empty 'absent') -band 0x13CF) -eq 0) `
    -Because 'PRECONDITION: an absent status word masks clean, which is the defect'

# ---------------------------------------------------------------------------
# 2. The required accessors refuse absence, null, and non-integers.
#    MUTATION: replace a `Get-ProductionPromotionRequiredUInt32` call in
#    ProductionCapture.Promotion.ps1 with `[uint32](Get-ProductionPromotionProperty ...)`
#    and section 4 below fails.
# ---------------------------------------------------------------------------
Assert-Throws -Action { Get-ProductionPromotionRequiredUInt32 $empty 'absent' 'row' } `
    -Because 'an absent property must be a hard failure, not 0'
Assert-Throws -Action { Get-ProductionPromotionRequiredInteger $empty 'absent' 'row' } `
    -Because 'an absent property must be a hard failure, not 0'
Assert-Throws -Action { Get-ProductionPromotionRequiredInteger $null 'present' 'row' } `
    -Because 'an absent OBJECT must be a hard failure'

$nulled = [pscustomobject]@{ flags = $null }
Assert-Throws -Action { Get-ProductionPromotionRequiredUInt32 $nulled 'flags' 'row' } `
    -Because 'an explicit null must be a hard failure, not 0'

$stringy = [pscustomobject]@{ flags = '0' }
Assert-Throws -Action { Get-ProductionPromotionRequiredUInt32 $stringy 'flags' 'row' } `
    -Because 'a string that looks numeric is not an integer'

$booly = [pscustomobject]@{ flags = $false }
Assert-Throws -Action { Get-ProductionPromotionRequiredUInt32 $booly 'flags' 'row' } `
    -Because '$false must not be read as 0'

$fractional = [pscustomobject]@{ flags = 1.5 }
Assert-Throws -Action { Get-ProductionPromotionRequiredInteger $fractional 'flags' 'row' } `
    -Because 'a fractional number is not an integer count'

# Windows PowerShell's ConvertFrom-Json returns System.Decimal for a JSON
# number carrying more precision than a double. Testing integrality by casting
# to double first ERASES the fraction -- [Math]::Floor([double]$x) -eq [double]$x
# is true for this value -- so it would have been accepted as the integer 10 and
# could then satisfy a count or domain-id comparison.
$highPrecision = ConvertFrom-Json '{"x":10.000000000000000000000000001}'
Assert-True -Condition ($highPrecision.x -is [decimal]) `
    -Because 'PRECONDITION: a high-precision JSON number arrives as System.Decimal'
Assert-True -Condition ([Math]::Floor([double]$highPrecision.x) -eq [double]$highPrecision.x) `
    -Because 'PRECONDITION: casting to double first erases the fraction, which is the trap'
Assert-Throws -Action { Get-ProductionPromotionRequiredInteger $highPrecision 'x' 'row' } `
    -Because 'a decimal that is only integral AFTER precision loss is not an integer'

$nan = [pscustomobject]@{ flags = [double]::NaN }
Assert-Throws -Action { Get-ProductionPromotionRequiredInteger $nan 'flags' 'row' } `
    -Because 'NaN is not an integer count'
$inf = [pscustomobject]@{ flags = [double]::PositiveInfinity }
Assert-Throws -Action { Get-ProductionPromotionRequiredInteger $inf 'flags' 'row' } `
    -Because 'infinity is not an integer count'

$negative = [pscustomobject]@{ flags = -1 }
Assert-Throws -Action { Get-ProductionPromotionRequiredUInt32 $negative 'flags' 'row' } `
    -Because 'a negative value is not a uint32 status word'

$tooBig = [pscustomobject]@{ flags = 4294967296 }
Assert-Throws -Action { Get-ProductionPromotionRequiredUInt32 $tooBig 'flags' 'row' } `
    -Because 'a value above uint32 is not a status word'

# ---------------------------------------------------------------------------
# 3. Present values still read correctly, in every shape ConvertFrom-Json emits.
# ---------------------------------------------------------------------------
$present = ConvertFrom-Json '{"zero":0,"flags":4919,"big":4294967295,"whole":8.0}'
Assert-True -Condition ((Get-ProductionPromotionRequiredUInt32 $present 'zero' 'row') -eq 0) `
    -Because 'a PUBLISHED zero is a real measurement and must be accepted'
Assert-True -Condition ((Get-ProductionPromotionRequiredUInt32 $present 'flags' 'row') -eq 4919) `
    -Because 'a published status word must round-trip'
Assert-True `
    -Condition ((Get-ProductionPromotionRequiredUInt32 $present 'big' 'row') -eq [uint32]::MaxValue) `
    -Because 'the top of the uint32 range must be accepted'
Assert-True -Condition ((Get-ProductionPromotionRequiredInteger $present 'whole' 'row') -eq 8) `
    -Because 'a whole-valued JSON double is an integer count'
# An ordered dictionary, which is how the in-process attestation hands values over.
$dict = [ordered]@{ flags = 7 }
Assert-True -Condition ((Get-ProductionPromotionRequiredUInt32 $dict 'flags' 'row') -eq 7) `
    -Because 'dictionary-backed evidence must read the same as JSON-backed evidence'
Assert-Throws -Action { Get-ProductionPromotionRequiredUInt32 $dict 'absent' 'row' } `
    -Because 'a dictionary missing the key must fail the same way'

# ---------------------------------------------------------------------------
# 3a. Wrongly-TYPED evidence is a second way to pass a gate without evidence.
#     PowerShell coerces on -eq and on casts, so a manifest can satisfy a
#     boolean requirement with a number or a string, and a "must be 0" count
#     requirement with the string "0" or with $false.
# ---------------------------------------------------------------------------
Assert-True -Condition (1 -eq $true) `
    -Because 'PRECONDITION: the NUMBER 1 satisfies -eq $true'
Assert-True -Condition ('true' -eq $true) `
    -Because 'PRECONDITION: the STRING "true" satisfies -eq $true'
Assert-True -Condition ([int64]'0' -eq 0) `
    -Because 'PRECONDITION: the STRING "0" casts to the integer 0'
Assert-True -Condition ([int64]$false -eq 0) `
    -Because 'PRECONDITION: $false casts to the integer 0'
# The accessors refuse all four shapes.
$typed = ConvertFrom-Json '{"met":1,"metstr":"true","zerostr":"0"}'
Assert-Throws -Action { Get-ProductionPromotionRequiredInteger $typed 'metstr' 'row' } `
    -Because 'a string must not satisfy an integer requirement'
Assert-Throws -Action { Get-ProductionPromotionRequiredInteger $typed 'zerostr' 'row' } `
    -Because 'the string "0" must not satisfy a must-be-zero requirement'

# ---------------------------------------------------------------------------
# 3c. Identities. `[string]123` is "123" and `[string]$null` is "", so casting
#     an identity before comparing it accepts the wrong type and turns absence
#     into the empty string. capture_id and session_id are what bind a receipt
#     to the plan and the manifest.
# ---------------------------------------------------------------------------
Assert-True -Condition ([string]123 -ceq '123') `
    -Because 'PRECONDITION: a number casts to the string a plan would publish'
Assert-True -Condition ([string]$null -ceq '') `
    -Because 'PRECONDITION: an absent identity casts to the empty string'
$ids = ConvertFrom-Json '{"num":123,"good":"c-1","blank":"   "}'
Assert-Throws -Action { Get-ProductionPromotionRequiredString $ids 'num' 'receipt' } `
    -Because 'a numeric identity must not authenticate a string one'
Assert-Throws -Action { Get-ProductionPromotionRequiredString $ids 'blank' 'receipt' } `
    -Because 'a whitespace identity is not an identity'
Assert-Throws -Action { Get-ProductionPromotionRequiredString $ids 'absent' 'receipt' } `
    -Because 'an absent identity must be a hard failure'
Assert-True -Condition ((Get-ProductionPromotionRequiredString $ids 'good' 'receipt') -ceq 'c-1') `
    -Because 'a published string identity must round-trip'

$gateTextProbe = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'ProductionCapture.Promotion.ps1') -Raw

# ---------------------------------------------------------------------------
# 3b. The array form -- which, measured, is NOT the same defect.
#
#     `@($null).Count` is 1, not 0. So a clause of the shape
#     `@(Get-...Property $x 'absent').Count -ne 0 { throw }` already FAILS on a
#     missing list: absence arrives as a one-element array holding $null. The
#     integer coercion and the array coercion behave OPPOSITELY, and that is
#     worth pinning, because "PowerShell turns absence into a passing value" is
#     true of one and false of the other.
#
#     What IS true of arrays is an unrolling trap in the other direction: a
#     function that returns `@()` returns NOTHING, and `$x = if (...) { @() }`
#     assigns $null -- so a naive accessor turns a PUBLISHED empty list back
#     into absence. Get-ProductionPromotionRequiredArray keeps the two apart.
# ---------------------------------------------------------------------------
Assert-True -Condition (@(Get-ProductionPromotionProperty $empty 'absent').Count -eq 1) `
    -Because 'PRECONDITION: @() of an absent property is a 1-element array holding $null, so the array clauses already fail closed'
Assert-Throws -Action { Get-ProductionPromotionRequiredArray $empty 'absent' 'report' } `
    -Because 'an absent list must be a hard failure, not an empty list'
Assert-Throws -Action { Get-ProductionPromotionRequiredArray $nulled 'flags' 'report' } `
    -Because 'a null list must be a hard failure'
$lists = ConvertFrom-Json '{"empty":[],"one":["a"],"scalar":7}'
Assert-True -Condition ((Get-ProductionPromotionRequiredArray $lists 'empty' 'r').Count -eq 0) `
    -Because 'a PUBLISHED empty list is real evidence and must be accepted'
Assert-True -Condition ((Get-ProductionPromotionRequiredArray $lists 'one' 'r').Count -eq 1) `
    -Because 'a published list must round-trip'
Assert-True -Condition ((Get-ProductionPromotionRequiredArray $lists 'scalar' 'r').Count -eq 1) `
    -Because 'a published scalar wraps to a 1-element list, as @() does'

foreach ($list in @('domain_gap_outcomes', 'problems')) {
    $shape = "@(Get-ProductionPromotionProperty `$validation '$list')"
    Assert-True -Condition (-not $gateTextProbe.Contains($shape)) `
        -Because "the gate still reads $list through @(), so a missing list passes"
}

# ---------------------------------------------------------------------------
# 4. The gate SOURCE no longer coerces the fields whose absence it must catch.
#    This is what fails if someone reintroduces the shape.
# ---------------------------------------------------------------------------
$gateText = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'ProductionCapture.Promotion.ps1') -Raw
foreach ($field in @(
    'effective_failure_status_flags_or',
    'status_flags_or',
    'skipped_game_hours',
    'domain_gap_record_count',
    'failure_status_flags_or',
    'client_commands_during_capture',
    'game_ui_commands'
)) {
    foreach ($cast in @('uint32', 'int', 'int64', 'long', 'uint64')) {
        $shape = "[$cast](Get-ProductionPromotionProperty `$row '$field')"
        Assert-True -Condition (-not $gateText.Contains($shape)) `
            -Because "the gate still coerces $field via [$cast]; absence would read as 0"
    }
}
# The literal defect: a cast applied directly to a property read of any object.
$pattern = "\[(uint32|int|int64|long|uint64)\]\(Get-ProductionPromotionProperty [^)]*'(" +
    "effective_failure_status_flags_or|skipped_game_hours|domain_gap_record_count|" +
    "failure_status_flags_or|client_commands_during_capture|semantic_engine_state_writes|" +
    "game_process_lifecycle_commands|game_ui_commands)'\)"
$offenders = [regex]::Matches($gateText, $pattern)
Assert-True -Condition ($offenders.Count -eq 0) `
    -Because ("$($offenders.Count) gate clause(s) still read a zero-required field " +
        'through a cast, so a missing field reads as a clean 0')

# The gate must actually demand the field that was never published: the name
# has to appear as an argument to a Required* accessor somewhere in the file.
$requiredEffective = [regex]::Matches(
    $gateText,
    "Get-ProductionPromotionRequired[A-Za-z0-9]+[\s\S]{0,80}?'effective_failure_status_flags_or'")
Assert-True -Condition ($requiredEffective.Count -ge 1) `
    -Because 'the per-domain clause must REQUIRE effective_failure_status_flags_or'

# ---------------------------------------------------------------------------
# Emits the same JSON receipt shape the other source self-tests use, so
# Invoke-CentralRustValidation.ps1 can pin the case count and refuse a run in
# which these cases silently stopped executing.
$receipt = [ordered]@{
    schema = 'battle-plan.promotion_gate_absence_test.v1'
    status = if ($script:Failures.Count -eq 0) { 'PASS' } else { 'FAIL' }
    cases = $script:Cases
    failures = @($script:Failures)
}
$receipt | ConvertTo-Json -Depth 4 -Compress
if ($script:Failures.Count -ne 0) { exit 1 }
exit 0
