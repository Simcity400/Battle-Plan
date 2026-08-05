<#
.SYNOPSIS
Prove the C07 gate's two receipt-admission functions reject what they must.

.DESCRIPTION
The C07 tree-library gate has two receipt shapes: the full one, written by a run
against the installed the game, and the reduced committed-artifact one, written when
the pinned installed content are absent. The whole value of the reduced path is that it
CANNOT be mistaken for the full one, and that is a property of these two
functions -- so it is tested here rather than assumed.

The functions are lifted out of Invoke-CentralRustValidation.ps1 by AST rather
than copied, so this test cannot drift away from the code it is testing: if a
function is renamed or removed, the lift fails and this test fails with it.

Run with -SelfTest; it prints a JSON authority object and exits non-zero on any
failure.
#>
[CmdletBinding()]
param(
    [switch]$SelfTest
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$gateScript = Join-Path $PSScriptRoot "Invoke-CentralRustValidation.ps1"
if (-not (Test-Path -LiteralPath $gateScript -PathType Leaf)) {
    throw "the gate script is missing: $gateScript"
}

# Lift only the receipt-admission helpers. Dot-sourcing the whole script would
# run a four-hour validation battery.
$parseErrors = $null
$ast = [System.Management.Automation.Language.Parser]::ParseFile(
    $gateScript, [ref]$null, [ref]$parseErrors)
if ($parseErrors) {
    throw "the gate script does not parse: $($parseErrors[0].Message)"
}
$wanted = @(
    "Get-RequiredReceiptField",
    "Get-RequiredReceiptScalar",
    "Get-RequiredReceiptList",
    "Assert-C07GeneratedTreeLibraryReceipt",
    "Assert-C07CommittedArtifactReceipt"
)
$definitions = $ast.FindAll(
    { param($node) $node -is [System.Management.Automation.Language.FunctionDefinitionAst] },
    $true)
foreach ($name in $wanted) {
    $found = @($definitions | Where-Object { $_.Name -ceq $name })
    if ($found.Count -ne 1) {
        throw "expected exactly one definition of $name in the gate script, found $($found.Count)"
    }
    . ([scriptblock]::Create($found[0].Extent.Text))
}

$pinSource = Join-Path $PSScriptRoot "..\..\..\portable_replay\native\render_stream\src\c07_tree_content_hashes.rs"
if (-not (Test-Path -LiteralPath $pinSource -PathType Leaf)) {
    throw "the C07 pin source is missing: $pinSource"
}
$pinText = Get-Content -LiteralPath $pinSource -Raw
$packedPin = ([regex]::Match(
        $pinText,
        '(?m)^pub const C07_TREE_CONTENT_LIBRARY_PACKED_BYTES_SHA256\s*:\s*&str\s*=\s*"([0-9a-f]{64})"\s*;')).Groups[1].Value
$payloadPin = ([regex]::Match(
        $pinText,
        '(?m)^pub const C07_TREE_CONTENT_LIBRARY_PAYLOAD_SHA256\s*:\s*&str\s*=\s*"([0-9a-f]{64})"\s*;')).Groups[1].Value
if ([string]::IsNullOrWhiteSpace($packedPin) -or [string]::IsNullOrWhiteSpace($payloadPin)) {
    throw "could not read both C07 pins out of $pinSource"
}

function New-CommittedReceipt {
    return [ordered]@{
        schema = "hoi4-c07-committed-artifact-self-consistency-v1"
        mode = "verify-committed"
        game_derived_verification = "NOT RUN: the pinned installed content are absent from this machine"
        checks = @(
            "bucket_record_counts_sum_to_pinned_cell_count",
            "bucket_table_complete_and_ordinal_dense",
            "bucket_table_sorted_by_bucket_id",
            "generated_rust_sources_round_trip",
            "library_payload_sha256_matches_pin",
            "missing_cell_set_pin_is_the_empty_digest",
            "packed_length_equals_bucket_table_total",
            "packed_sha256_matches_pin",
            "per_bucket_instance_sha256_matches_packed_slice"
        )
        unchecked_without_the_game = @("packed_bytes_are_what_the_exact_producer_derives_from_trees_bmp")
        bucket_count = 296
        cell_count = 99056
        packed_bytes = 2377344
        packed_sha256 = $packedPin
        content_library_payload_sha256 = $payloadPin
    }
}

function New-FullReceipt {
    return [ordered]@{
        mode = "verify"
        verified_outputs = @(
            "c07_tree_complete_buckets.rs",
            "c07_tree_content_hashes.rs",
            "c07_tree_content_library.bin"
        )
        changed_outputs = @()
        bucket_count = 296
        cell_count = 99056
        packed_bytes = 2377344
        packed_sha256 = $packedPin
        content_library_payload_sha256 = $payloadPin
    }
}

function Test-Case {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][scriptblock]$Body,
        [Parameter(Mandatory = $true)][bool]$ShouldThrow,
        [string]$ExpectedText = ""
    )
    $threw = $false
    $message = ""
    try { & $Body } catch { $threw = $true; $message = $_.Exception.Message }
    if ($threw -ne $ShouldThrow) {
        throw "case '$Name': expected throw=$ShouldThrow, observed throw=$threw ($message)"
    }
    if ($ShouldThrow -and -not [string]::IsNullOrWhiteSpace($ExpectedText) -and
        $message -notlike "*$ExpectedText*") {
        throw "case '$Name': refusal did not name '$ExpectedText': $message"
    }
    return 1
}

$cases = 0

# The two well-formed receipts are admitted by their own function.
$cases += Test-Case -Name "full receipt is admitted" -ShouldThrow $false -Body {
    Assert-C07GeneratedTreeLibraryReceipt -ReceiptJson ((New-FullReceipt) | ConvertTo-Json -Depth 6) -PinSource $pinSource
}
$cases += Test-Case -Name "committed receipt is admitted" -ShouldThrow $false -Body {
    Assert-C07CommittedArtifactReceipt -ReceiptJson ((New-CommittedReceipt) | ConvertTo-Json -Depth 6) -PinSource $pinSource
}

# THE CENTRAL PROPERTY: neither receipt can be read as the other. A reduced run
# passed off as a full one is exactly the absence-reads-as-pass failure.
$cases += Test-Case -Name "committed receipt is refused by the full gate" -ShouldThrow $true -ExpectedText "not 'verify'" -Body {
    Assert-C07GeneratedTreeLibraryReceipt -ReceiptJson ((New-CommittedReceipt) | ConvertTo-Json -Depth 6) -PinSource $pinSource
}
$cases += Test-Case -Name "full receipt is refused by the committed gate" -ShouldThrow $true -ExpectedText "has no 'schema' field" -Body {
    Assert-C07CommittedArtifactReceipt -ReceiptJson ((New-FullReceipt) | ConvertTo-Json -Depth 6) -PinSource $pinSource
}

# The schema and mode strings are checked by VALUE, not merely for presence. A
# receipt that names a different schema is a receipt this gate does not
# understand, and admitting it would let some future producer's output be read
# under rules written for this one.
$cases += Test-Case -Name "committed receipt with a foreign schema is refused" -ShouldThrow $true -ExpectedText "only accepts hoi4-c07-committed-artifact-self-consistency-v1" -Body {
    $receipt = New-CommittedReceipt
    $receipt.schema = "hoi4-c07-committed-artifact-self-consistency-v2"
    Assert-C07CommittedArtifactReceipt -ReceiptJson ($receipt | ConvertTo-Json -Depth 6) -PinSource $pinSource
}
$cases += Test-Case -Name "committed receipt claiming the full mode is refused" -ShouldThrow $true -ExpectedText "not 'verify-committed'" -Body {
    $receipt = New-CommittedReceipt
    $receipt.mode = "verify"
    Assert-C07CommittedArtifactReceipt -ReceiptJson ($receipt | ConvertTo-Json -Depth 6) -PinSource $pinSource
}
# And the same, the other way: the full gate checks the mode by value, so a
# reduced run cannot be admitted by relabelling its schema.
$cases += Test-Case -Name "committed receipt relabelled as the full schema is still refused by the full gate" -ShouldThrow $true -ExpectedText "not 'verify'" -Body {
    $receipt = New-CommittedReceipt
    $receipt.schema = "hoi4-c07-complete-engine-derived-tree-content-library-v2"
    Assert-C07GeneratedTreeLibraryReceipt -ReceiptJson ($receipt | ConvertTo-Json -Depth 6) -PinSource $pinSource
}

# A reduced receipt that stopped performing a check is refused: the checks list
# is the gate's inventory, not decoration.
$cases += Test-Case -Name "committed receipt missing a check is refused" -ShouldThrow $true -ExpectedText "not the nine" -Body {
    $receipt = New-CommittedReceipt
    $receipt.checks = @($receipt.checks | Where-Object { $_ -ne "library_payload_sha256_matches_pin" })
    Assert-C07CommittedArtifactReceipt -ReceiptJson ($receipt | ConvertTo-Json -Depth 6) -PinSource $pinSource
}
$cases += Test-Case -Name "committed receipt with an empty checks list is refused" -ShouldThrow $true -ExpectedText "not the nine" -Body {
    $receipt = New-CommittedReceipt
    $receipt.checks = @()
    Assert-C07CommittedArtifactReceipt -ReceiptJson ($receipt | ConvertTo-Json -Depth 6) -PinSource $pinSource
}

# A reduced receipt that does not declare the game-derived half unrun is refused.
$cases += Test-Case -Name "committed receipt claiming the game half ran is refused" -ShouldThrow $true -ExpectedText "does not declare" -Body {
    $receipt = New-CommittedReceipt
    $receipt.game_derived_verification = "verified against the installed game"
    Assert-C07CommittedArtifactReceipt -ReceiptJson ($receipt | ConvertTo-Json -Depth 6) -PinSource $pinSource
}

# Both gates still bind their digests to the literals the crate compiles.
$cases += Test-Case -Name "committed receipt with a drifted packed digest is refused" -ShouldThrow $true -ExpectedText "Never edit a pin literal" -Body {
    $receipt = New-CommittedReceipt
    $receipt.packed_sha256 = ("0" * 64)
    Assert-C07CommittedArtifactReceipt -ReceiptJson ($receipt | ConvertTo-Json -Depth 6) -PinSource $pinSource
}
$cases += Test-Case -Name "committed receipt with a drifted payload digest is refused" -ShouldThrow $true -ExpectedText "Never edit a pin literal" -Body {
    $receipt = New-CommittedReceipt
    $receipt.content_library_payload_sha256 = ("1" * 64)
    Assert-C07CommittedArtifactReceipt -ReceiptJson ($receipt | ConvertTo-Json -Depth 6) -PinSource $pinSource
}
$cases += Test-Case -Name "committed receipt with a moved bucket count is refused" -ShouldThrow $true -ExpectedText "bucket_count" -Body {
    $receipt = New-CommittedReceipt
    $receipt.bucket_count = 295
    Assert-C07CommittedArtifactReceipt -ReceiptJson ($receipt | ConvertTo-Json -Depth 6) -PinSource $pinSource
}

# An empty stdout is not a pass. This is the shape a generator that died before
# printing would leave behind.
$cases += Test-Case -Name "an empty committed receipt is refused" -ShouldThrow $true -ExpectedText "printed no committed-artifact receipt" -Body {
    Assert-C07CommittedArtifactReceipt -ReceiptJson "" -PinSource $pinSource
}

$result = [ordered]@{
    schema = "battle-plan.c07_gate_receipt_admission.v1"
    status = "PASS"
    cases = $cases
}
Write-Output ($result | ConvertTo-Json -Compress)
exit 0
