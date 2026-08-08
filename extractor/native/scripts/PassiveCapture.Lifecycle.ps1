<#
.SYNOPSIS
    Strict authority checks for passive collector READY and teardown receipts.

.DESCRIPTION
    Dot-source this file from capture launchers.  It performs no process
    control and opens no handle to HOI4.  The caller supplies the independently
    established process/configuration pins; this verifier admits only the exact
    closed READY v2 and teardown v1 schemas emitted by collector_record.
#>

Set-StrictMode -Version Latest

# --- named refusals -----------------------------------------------------------
#
# WHY A STRUCTURED THROW AND NOT A SENTENCE.
#
# Everything in this file that compares the verifier's mirror against the
# collector's own producer source used to fail with a bare `throw '<sentence>'`.
# A sentence is enough for a human reading a transcript and useless to the
# harness, which has to decide what to do with a live recorder and then say why
# in a receipt somebody can grep. Run t36 is the whole argument: the mirror
# refused the READY receipt with "Passive READY wire field order differs at
# ...", the harness had no idea what class of failure that was, and the capture
# ended with the game pinned, the settings guard applied and no machine-readable
# statement of what had gone wrong anywhere on disk.
#
# So the mirror-versus-producer checks throw an ErrorRecord whose TargetObject
# carries the class, the field, and the two sides of the comparison. The
# Exception.Message is unchanged in kind -- every existing caller that prints
# `$_.Exception.Message` still prints a sentence -- and MEASURED on Windows
# PowerShell 5.1: `throw $errorRecord` preserves both TargetObject and
# FullyQualifiedErrorId, including through an intermediate `catch { throw }`.
$script:PassiveReadyMismatchSchema = 'hoi4-passive-ready-contract-mismatch-v1'

function New-PassiveReadyContractMismatch {
    param(
        [Parameter(Mandatory = $true)][string]$Class,
        [Parameter(Mandatory = $true)][string]$Field,
        [Parameter(Mandatory = $true)][AllowEmptyString()][string]$Expected,
        [Parameter(Mandatory = $true)][AllowEmptyString()][string]$Observed,
        [Parameter(Mandatory = $true)][string]$Message
    )

    $detail = [pscustomobject]@{
        schema   = $script:PassiveReadyMismatchSchema
        class    = $Class
        field    = $Field
        expected = $Expected
        observed = $Observed
        message  = $Message
    }
    return [Management.Automation.ErrorRecord]::new(
        [InvalidOperationException]::new($Message),
        "PassiveReadyContractMismatch.$Class",
        [Management.Automation.ErrorCategory]::InvalidData,
        $detail
    )
}

# The fragment checks are all the same shape -- "this exact text must still be
# in the producer source" -- and there are two dozen of them. A helper keeps
# each one a single line so that adding a pinned fragment cannot accidentally
# reintroduce an unnamed `throw`.
function Assert-PassiveProducerFragment {
    param(
        [Parameter(Mandatory = $true)][AllowEmptyString()][string]$Source,
        [Parameter(Mandatory = $true)][string]$Fragment,
        [Parameter(Mandatory = $true)][string]$Class,
        [Parameter(Mandatory = $true)][string]$Message
    )

    if ($Source.Contains($Fragment)) { return }
    throw (New-PassiveReadyContractMismatch `
        -Class $Class `
        -Field $Fragment `
        -Expected 'present in the canonical producer source' `
        -Observed 'absent' `
        -Message $Message)
}

# Reads the structured detail back off ANY error, and answers $null when there
# is none.
#
# The schema test is load-bearing rather than a null check: PowerShell sets
# TargetObject to the thrown value for a plain `throw '<string>'` too, so an
# unstructured refusal arrives here carrying its own message as a TargetObject.
# Without the schema test that string would be read as a mismatch record and
# every property access on it would be $null under StrictMode.
function Get-PassiveReadyContractMismatch {
    param([Parameter(Mandatory = $true)][AllowNull()]$ErrorRecord)

    if ($null -eq $ErrorRecord) { return $null }
    $target = $null
    try { $target = $ErrorRecord.TargetObject } catch { return $null }
    if ($null -eq $target -or $target -is [string]) { return $null }
    $schema = $null
    try { $schema = [string]$target.schema } catch { return $null }
    if ($schema -cne $script:PassiveReadyMismatchSchema) { return $null }
    return $target
}

# Turns the accumulated-admission refusal back into per-clause records.
#
# `Add-PassiveAdmissionFailure` formats every clause as either `<name>` or
# `<name> (expected='<x>' actual='<y>')`, and `Test-PassiveCaptureReadyReceipt`
# joins them with '; '. That format is the only machine-readable statement of
# WHICH field disagreed, and it was being thrown away into a sentence. Parsed
# here, next to the code that writes it, so the two cannot drift apart
# unnoticed -- the unit suite round-trips a formatted clause list through both.
function ConvertFrom-PassiveAdmissionFailureClause {
    param([Parameter(Mandatory = $true)][AllowEmptyString()][string]$Clause)

    $match = [regex]::Match(
        $Clause,
        "^(?<field>.*?) \(expected='(?<expected>.*)' actual='(?<actual>.*)'\)$",
        [Text.RegularExpressions.RegexOptions]::Singleline
    )
    if ($match.Success) {
        return [pscustomobject]@{
            field    = $match.Groups['field'].Value
            expected = $match.Groups['expected'].Value
            observed = $match.Groups['actual'].Value
        }
    }
    return [pscustomobject]@{
        field    = $Clause
        expected = ''
        observed = ''
    }
}

$script:PassiveReadyAdmissionSchema = 'hoi4-passive-ready-admission-refusal-v1'

# The clause-accumulator refusal, carried as data rather than only as prose.
#
# The two lists are already the answer to "which field disagreed, and does the
# disagreement cost custody" -- they were being joined into one sentence and the
# structure discarded at the throw. Both are attached here so the harness's
# refusal receipt can name them per clause and per side without re-parsing the
# composed message.
function New-PassiveReadyAdmissionRefusal {
    param(
        [Parameter(Mandatory = $true)][string]$Message,
        [Parameter(Mandatory = $true)]
        [AllowEmptyCollection()]
        [Collections.Generic.List[string]]$CustodyFailures,
        [Parameter(Mandatory = $true)]
        [AllowEmptyCollection()]
        [Collections.Generic.List[string]]$GradingFailures
    )

    $detail = [pscustomobject]@{
        schema           = $script:PassiveReadyAdmissionSchema
        class            = 'receipt_admission'
        custody_failures = [string[]]$CustodyFailures.ToArray()
        grading_failures = [string[]]$GradingFailures.ToArray()
        message          = $Message
    }
    return [Management.Automation.ErrorRecord]::new(
        [InvalidOperationException]::new($Message),
        'PassiveReadyContractMismatch.receipt_admission',
        [Management.Automation.ErrorCategory]::InvalidData,
        $detail
    )
}

# The single classifier: ANY error out of READY validation becomes a named
# refusal record, and there is no route through it that produces an unnamed one.
#
# Three inputs are possible and all three are covered. A mirror-versus-producer
# mismatch arrives structured and keeps its class, field and both sides. A
# clause-accumulator refusal arrives structured and is expanded per clause, with
# the custody/grading side kept -- that side is what decides whether the
# recorder's later teardown record can still be believed. Anything else -- an
# unreadable receipt, a torn control file, a defect in this verifier -- is
# classed `unclassified` and still names the exception type and message, because
# "the harness could not say what went wrong" has to be a statement on disk
# rather than an absence.
function New-PassiveReadyRefusalRecord {
    param(
        [Parameter(Mandatory = $true)][AllowNull()]$ErrorRecord,
        [Parameter(Mandatory = $true)][bool]$CustodyEstablished,
        [ValidateNotNullOrEmpty()][string]$Stage = 'ready_validation'
    )

    $message = ''
    $exceptionType = ''
    if ($null -ne $ErrorRecord) {
        try { $message = [string]$ErrorRecord.Exception.Message } catch { }
        try { $exceptionType = [string]$ErrorRecord.Exception.GetType().FullName } catch { }
    }
    $message = ($message -replace '\s+', ' ').Trim()

    $class = 'unclassified'
    $clauses = [Collections.Generic.List[object]]::new()

    $target = $null
    if ($null -ne $ErrorRecord) {
        try { $target = $ErrorRecord.TargetObject } catch { $target = $null }
    }
    $targetSchema = ''
    if ($null -ne $target -and $target -isnot [string]) {
        try { $targetSchema = [string]$target.schema } catch { $targetSchema = '' }
    }

    if ($targetSchema -ceq $script:PassiveReadyMismatchSchema) {
        $class = [string]$target.class
        $clauses.Add([pscustomobject]@{
            side     = 'custody'
            field    = [string]$target.field
            expected = [string]$target.expected
            observed = [string]$target.observed
        })
    } elseif ($targetSchema -ceq $script:PassiveReadyAdmissionSchema) {
        $class = [string]$target.class
        foreach ($side in @('custody', 'grading')) {
            $list = if ($side -ceq 'custody') {
                @($target.custody_failures)
            } else {
                @($target.grading_failures)
            }
            foreach ($clause in $list) {
                if ($null -eq $clause) { continue }
                $parsed = ConvertFrom-PassiveAdmissionFailureClause -Clause ([string]$clause)
                $clauses.Add([pscustomobject]@{
                    side     = $side
                    field    = $parsed.field
                    expected = $parsed.expected
                    observed = $parsed.observed
                })
            }
        }
    } else {
        $clauses.Add([pscustomobject]@{
            side     = 'custody'
            field    = $(if ([string]::IsNullOrWhiteSpace($exceptionType)) {
                'ready_validation'
            } else {
                $exceptionType
            })
            expected = 'READY validation completes and publishes an authority'
            observed = $(if ([string]::IsNullOrWhiteSpace($message)) {
                'the verifier failed without a message'
            } else {
                $message
            })
        })
    }

    # NOT 'hoi4-capture-refusal-v1': that is the ENVELOPE the harness writes,
    # and this record is one element of its `capture_refusals` array. Two
    # different shapes under one schema name is a reader's problem, not a
    # writer's, and it costs nothing to keep them apart.
    return [pscustomobject]@{
        schema              = 'hoi4-capture-refusal-record-v1'
        stage               = $Stage
        class               = $class
        custody_established = $CustodyEstablished
        message             = $(if ([string]::IsNullOrWhiteSpace($message)) {
            'READY validation failed without a message'
        } else {
            $message
        })
        clauses             = @($clauses.ToArray())
    }
}

function ConvertTo-PassiveReceiptInteger {
    param(
        [AllowNull()][object]$Value,
        [Parameter(Mandatory = $true)]
        [ValidateSet('u32', 'u64', 'i32')]
        [string]$Kind,
        [Parameter(Mandatory = $true)][string]$Field
    )

    if (-not (
        $Value -is [byte] -or $Value -is [uint16] -or
        $Value -is [uint32] -or $Value -is [uint64] -or
        $Value -is [int16] -or $Value -is [int32] -or
        $Value -is [int64] -or $Value -is [decimal]
    )) {
        throw "Passive lifecycle field '$Field' is not a JSON integer."
    }
    $text = [Convert]::ToString(
        $Value,
        [Globalization.CultureInfo]::InvariantCulture
    )
    switch ($Kind) {
        'u32' {
            [uint32]$parsed = 0
            $ok = [uint32]::TryParse(
                $text,
                [Globalization.NumberStyles]::None,
                [Globalization.CultureInfo]::InvariantCulture,
                [ref]$parsed
            )
        }
        'u64' {
            [uint64]$parsed = 0
            $ok = [uint64]::TryParse(
                $text,
                [Globalization.NumberStyles]::None,
                [Globalization.CultureInfo]::InvariantCulture,
                [ref]$parsed
            )
        }
        'i32' {
            [int32]$parsed = 0
            $ok = [int32]::TryParse(
                $text,
                [Globalization.NumberStyles]::AllowLeadingSign,
                [Globalization.CultureInfo]::InvariantCulture,
                [ref]$parsed
            )
        }
    }
    if (-not $ok -or
        $text -cne $parsed.ToString(
            [Globalization.CultureInfo]::InvariantCulture
        )) {
        throw "Passive lifecycle field '$Field' is not a canonical bounded $Kind."
    }
    return $parsed
}

function Get-PassiveUtf8Sha256 {
    param([Parameter(Mandatory = $true)][string]$Value)

    $hash = [Security.Cryptography.SHA256]::Create()
    try {
        $bytes = [Text.UTF8Encoding]::new($false).GetBytes($Value)
        return ([BitConverter]::ToString(
            $hash.ComputeHash($bytes)
        )).Replace('-', '').ToLowerInvariant()
    } finally {
        $hash.Dispose()
    }
}

function Get-PassiveBytesSha256 {
    param([Parameter(Mandatory = $true)][byte[]]$Bytes)

    $hash = [Security.Cryptography.SHA256]::Create()
    try {
        return ([BitConverter]::ToString(
            $hash.ComputeHash($Bytes)
        )).Replace('-', '').ToLowerInvariant()
    } finally {
        $hash.Dispose()
    }
}

function Add-PassiveLittleEndianBytes {
    param(
        [Parameter(Mandatory = $true)]
        # A mandatory collection parameter also rejects an EMPTY collection in
        # Windows PowerShell, and every caller passes a freshly-created empty
        # List[byte] on its first append. Without this the whole passive READY
        # digest path threw on its first call and could never run.
        [AllowEmptyCollection()]
        [Collections.Generic.List[byte]]$Destination,
        [Parameter(Mandatory = $true)]
        [ValidateSet('u16', 'u32', 'u64', 'i32')]
        [string]$Kind,
        [Parameter(Mandatory = $true)][object]$Value
    )

    [byte[]]$bytes = switch ($Kind) {
        'u16' { [BitConverter]::GetBytes([uint16]$Value) }
        'u32' { [BitConverter]::GetBytes([uint32]$Value) }
        'u64' { [BitConverter]::GetBytes([uint64]$Value) }
        'i32' { [BitConverter]::GetBytes([int32]$Value) }
    }
    if (-not [BitConverter]::IsLittleEndian) {
        [Array]::Reverse($bytes)
    }
    $Destination.AddRange($bytes)
}

function New-PassiveLabelledPreimage {
    param([Parameter(Mandatory = $true)][string]$Label)

    $labelBytes = [Text.UTF8Encoding]::new($false).GetBytes($Label)
    if ($labelBytes.Length -gt [uint16]::MaxValue) {
        throw "Passive READY digest label '$Label' exceeds u16 length."
    }
    $bytes = [Collections.Generic.List[byte]]::new()
    Add-PassiveLittleEndianBytes $bytes u16 ([uint16]$labelBytes.Length)
    $bytes.AddRange($labelBytes)
    Add-PassiveLittleEndianBytes $bytes u16 ([uint16]1)
    return ,$bytes
}

function ConvertFrom-PassiveCanonicalUuid {
    param([Parameter(Mandatory = $true)][string]$Value)

    if ($Value -cnotmatch
        '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$' -or
        $Value -ceq '00000000-0000-0000-0000-000000000000') {
        throw 'Passive READY session id is not a nonzero canonical lowercase UUID.'
    }
    $hex = $Value.Replace('-', '')
    $bytes = [byte[]]::new(16)
    for ($index = 0; $index -lt $bytes.Length; $index++) {
        $bytes[$index] = [Convert]::ToByte($hex.Substring($index * 2, 2), 16)
    }
    return ,$bytes
}

function Get-PassiveSchedulerConfigSha256 {
    param(
        [Parameter(Mandatory = $true)][uint32]$PollMilliseconds,
        [Parameter(Mandatory = $true)][uint32]$DomainQueueCapacity,
        [Parameter(Mandatory = $true)][uint32]$WorkerCount
    )

    if ($PollMilliseconds -eq 0 -or
        $PollMilliseconds -gt [uint32]::MaxValue / 1000 -or
        $DomainQueueCapacity -ne 1 -or
        $WorkerCount -eq 0 -or
        $WorkerCount -gt [uint16]::MaxValue) {
        throw 'Passive READY scheduler inputs do not fit the production scheduler contract.'
    }
    $bytes = New-PassiveLabelledPreimage `
        'battle-plan.capture_ready.scheduler_config.v1'
    Add-PassiveLittleEndianBytes `
        $bytes u32 ([uint32]($PollMilliseconds * 1000))
    Add-PassiveLittleEndianBytes $bytes u32 $DomainQueueCapacity
    Add-PassiveLittleEndianBytes $bytes u32 ([uint32]2)
    Add-PassiveLittleEndianBytes $bytes u16 ([uint16]$WorkerCount)
    Add-PassiveLittleEndianBytes $bytes u16 ([uint16]0)
    Add-PassiveLittleEndianBytes $bytes u32 ([uint32]0)
    Add-PassiveLittleEndianBytes $bytes u64 ([uint64](256 * 1024 * 1024))
    return Get-PassiveBytesSha256 $bytes.ToArray()
}

function Get-PassiveClockWitnessSha256 {
    param(
        [Parameter(Mandatory = $true)][int32]$ClockHourRaw,
        [Parameter(Mandatory = $true)][uint64]$ObservationOrdinal,
        [Parameter(Mandatory = $true)][string]$TargetExecutableSha256,
        [Parameter(Mandatory = $true)][string]$CollectorBuildProfileId
    )

    if ($ClockHourRaw -lt 60005999 -or $ClockHourRaw -gt 63071999) {
        throw "Passive READY clock hour $ClockHourRaw is outside years 1850..2200."
    }
    if ($ObservationOrdinal -eq [uint64]::MaxValue) {
        throw 'Passive READY clock observation ordinal is exhausted.'
    }
    $buildProfileSha256 = Get-PassiveUtf8Sha256 $CollectorBuildProfileId
    $bytes = New-PassiveLabelledPreimage `
        'battle-plan.capture_ready.read_only_clock_witness.v1'
    Add-PassiveLittleEndianBytes $bytes i32 $ClockHourRaw
    Add-PassiveLittleEndianBytes $bytes i32 $ClockHourRaw
    Add-PassiveLittleEndianBytes $bytes u64 $ObservationOrdinal
    $bytes.AddRange((ConvertFrom-PassiveSha256 $TargetExecutableSha256))
    $bytes.AddRange((ConvertFrom-PassiveSha256 $buildProfileSha256))
    return Get-PassiveBytesSha256 $bytes.ToArray()
}

function Get-PassiveOutputPipeIdentitySha256 {
    param(
        [Parameter(Mandatory = $true)][string]$CanonicalPipeName,
        [Parameter(Mandatory = $true)][uint32]$TargetPid,
        [Parameter(Mandatory = $true)][uint64]$TargetCreationFiletimeUtc,
        [Parameter(Mandatory = $true)][string]$SessionId
    )

    $pipeBytes = [Text.UTF8Encoding]::new($false).GetBytes($CanonicalPipeName)
    if ($pipeBytes.Length -gt 256 -or
        -not $CanonicalPipeName.StartsWith('\\.\pipe\') -or
        $CanonicalPipeName.Substring(9).Length -eq 0 -or
        $CanonicalPipeName.Substring(9).Contains('\') -or
        $CanonicalPipeName.Substring(9).Contains('/') -or
        @($pipeBytes | Where-Object { $_ -lt 0x21 -or $_ -gt 0x7e }).Count -ne 0 -or
        $TargetPid -eq 0 -or
        $TargetCreationFiletimeUtc -eq 0) {
        throw 'Passive READY output-pipe identity is not canonical.'
    }
    $sessionBytes = ConvertFrom-PassiveCanonicalUuid $SessionId
    $bytes = New-PassiveLabelledPreimage `
        'battle-plan.capture_ready.output_pipe_identity.v1'
    Add-PassiveLittleEndianBytes $bytes u16 ([uint16]$pipeBytes.Length)
    $bytes.AddRange($pipeBytes)
    Add-PassiveLittleEndianBytes $bytes u32 $TargetPid
    Add-PassiveLittleEndianBytes $bytes u64 $TargetCreationFiletimeUtc
    $bytes.AddRange($sessionBytes)
    return Get-PassiveBytesSha256 $bytes.ToArray()
}

function Get-PassiveZeroMutationPolicySha256 {
    $bytes = New-PassiveLabelledPreimage `
        'battle-plan.capture_ready.zero_mutation_policy.v1'
    Add-PassiveLittleEndianBytes $bytes u16 ([uint16]1)
    return Get-PassiveBytesSha256 $bytes.ToArray()
}

function Get-PassiveNamedValueSha256 {
    param(
        [Parameter(Mandatory = $true)][string]$Label,
        [Parameter(Mandatory = $true)][string]$Value
    )

    $valueBytes = [Text.UTF8Encoding]::new($false).GetBytes($Value)
    if ($valueBytes.Length -gt [uint16]::MaxValue) {
        throw "Passive READY named value '$Value' exceeds u16 length."
    }
    $bytes = New-PassiveLabelledPreimage $Label
    Add-PassiveLittleEndianBytes $bytes u16 ([uint16]$valueBytes.Length)
    $bytes.AddRange($valueBytes)
    return Get-PassiveBytesSha256 $bytes.ToArray()
}

function Get-PassivePinnedMacroMandatoryByte {
    param(
        [Parameter(Mandatory = $true)][string]$ContractSource,
        [Parameter(Mandatory = $true)][string]$MacroName
    )

    $macroStart = $ContractSource.IndexOf("macro_rules! $MacroName")
    if ($macroStart -lt 0) {
        throw "Pinned hook macro '$MacroName' is not declared by the canonical roster source."
    }
    # Bound the search to THIS macro's body, ending at its own closing brace --
    # the first `}` in column 0, since every brace inside a macro body is
    # indented. Bounding at "the next macro_rules!" instead leaves the last
    # macro in the file scanning to EOF, where the struct definitions, doc
    # comments and unit tests declare `mandatory:` too; that read four literals
    # where the macro declares one.
    $bodyMatch = [regex]::Match(
        $ContractSource.Substring($macroStart),
        '^macro_rules!\s+\S+\s*\{.*?^\}',
        (
            [Text.RegularExpressions.RegexOptions]::Singleline -bor
            [Text.RegularExpressions.RegexOptions]::Multiline
        )
    )
    if (-not $bodyMatch.Success) {
        throw "Pinned hook macro '$MacroName' has no closing brace in the canonical roster source."
    }
    $body = $bodyMatch.Value
    # NOT $matches: that is a read/write PowerShell automatic variable which any
    # later -match in this scope silently overwrites.
    $mandatoryMatches = [regex]::Matches($body, 'mandatory:\s*(true|false)\s*,')
    if ($mandatoryMatches.Count -ne 1) {
        throw "Pinned hook macro '$MacroName' declares $($mandatoryMatches.Count) mandatory literals; expected exactly 1."
    }
    if ($mandatoryMatches[0].Groups[1].Value -ceq 'true') { return [byte]1 }
    return [byte]0
}

function Get-PassivePinnedHookAllowlistAuthorityFromSource {
    param([Parameter(Mandatory = $true)][string]$ContractSource)

    if (-not $ContractSource.Contains(
        'pub const PINNED_PRODUCTION_HOOK_COUNT_V1: u16 = PINNED_PRODUCTION_HOOK_SPECS_V1.len() as u16;'
    )) {
        throw 'Pinned production hook-count authority is not derived from the canonical roster.'
    }
    $start = $ContractSource.IndexOf(
        'pub const PINNED_PRODUCTION_HOOK_SPECS_V1'
    )
    $end = $ContractSource.IndexOf('];', $start)
    if ($start -lt 0 -or $end -le $start) {
        throw 'Pinned production hook source roster boundary is unavailable.'
    }
    $rosterSource = $ContractSource.Substring($start, $end - $start)
    $entries = [Collections.Generic.List[object]]::new()
    $primitiveIds = @{}
    foreach ($match in [regex]::Matches(
        $ContractSource,
        'const ([A-Z0-9_]+): &str = "([^"]+)";'
    )) {
        $primitiveIds[$match.Groups[1].Value] = $match.Groups[2].Value
    }
    # Import-slot primitive and width, PARSED rather than assumed. This encoder
    # is a deliberately independent re-derivation of the Rust roster digest, and
    # independence is only worth something while it tracks the contract. It did
    # not: when the two bootstrap entries were corrected from the retired inline
    # primitive (minhook-0.9.0.normal-jmp-rel5, 5 bytes) to the one the provider
    # actually installs (pe64.import-address-slot.atomic-cas.v1, 8 bytes), this
    # file and its Python twin (verify_sink_reconciliation.py) were left behind
    # and went on silently producing the pre-correction digest
    # 1eed3cf79a86d69cb79f3d55ca95e5d8817cb3846100e96fcb103e0c21d2a3a6.
    $importPrimitiveId = [string]$primitiveIds['PE64_IMPORT_ADDRESS_SLOT_ATOMIC_CAS']
    if ([string]::IsNullOrEmpty($importPrimitiveId)) {
        throw 'Pinned import-slot primitive id is not declared by the canonical roster source.'
    }
    $importWidthMatch = [regex]::Match(
        $ContractSource,
        'pub const PE64_IMPORT_ADDRESS_SLOT_ATOMIC_WIDTH: u16 = (\d+);'
    )
    if (-not $importWidthMatch.Success) {
        throw 'Pinned import-slot patch width is not declared by the canonical roster source.'
    }
    $importPatchLen = [uint16]$importWidthMatch.Groups[1].Value
    # The mandatory bit is PARSED from each macro, never assumed, for the same
    # reason the import primitive above is. It is part of the Rust allowlist
    # preimage (`out.push(u8::from(entry.mandatory))`), so it moves the digest.
    # This encoder previously hardcoded 1 for BOTH macros; when the gameplay
    # producer detours became non-mandatory it kept emitting the superseded
    # digest b100b582f946788d45d66a4ee9192ac506a5498148582815476642c2010ce2dd
    # and rejected every otherwise-valid receipt the recorder published.
    $importMandatory = Get-PassivePinnedMacroMandatoryByte `
        -ContractSource $ContractSource -MacroName 'pinned_import'
    $rvaMandatory = Get-PassivePinnedMacroMandatoryByte `
        -ContractSource $ContractSource -MacroName 'pinned_rva'
    $importPattern =
        'pinned_import!\(\s*"([^"]+)",\s*"([^"]+)",\s*"([^"]+)",\s*"([0-9a-f]{64})"\s*\)'
    # How many IMPORT SLOTS the canonical roster declares. Derived here, from
    # the same roster text the digest is built from, because the teardown
    # receipt now carries a verified-import-slot count and a count is only a
    # check if something independent knows what it should be. Two is the
    # current answer; nothing below assumes that.
    $importSlotCount = 0
    foreach ($match in [regex]::Matches(
        $rosterSource,
        $importPattern,
        [Text.RegularExpressions.RegexOptions]::Singleline
    )) {
        $importSlotCount++
        $identitySha = Get-PassiveNamedValueSha256 `
            'battle-plan.capture_ready.hook_identity.v1' `
            $match.Groups[1].Value
        $moduleSha = Get-PassiveNamedValueSha256 `
            'battle-plan.capture_ready.hook_target_name.v1' `
            $match.Groups[2].Value
        $symbolSha = Get-PassiveNamedValueSha256 `
            'battle-plan.capture_ready.hook_target_name.v1' `
            $match.Groups[3].Value
        $primitiveSha = Get-PassiveNamedValueSha256 `
            'battle-plan.capture_ready.hook_primitive_id.v1' `
            $importPrimitiveId
        $entry = [Collections.Generic.List[byte]]::new()
        Add-PassiveSha256Bytes $entry $identitySha
        Add-PassiveLittleEndianBytes $entry u16 ([uint16]1)
        Add-PassiveLittleEndianBytes $entry u16 $importPatchLen
        Add-PassiveSha256Bytes $entry $moduleSha
        Add-PassiveSha256Bytes $entry $symbolSha
        Add-PassiveSha256Bytes $entry $primitiveSha
        Add-PassiveSha256Bytes $entry $match.Groups[4].Value
        $entry.Add($importMandatory)
        $entry.AddRange([byte[]]::new(7))
        $entries.Add([pscustomobject]@{
            IdentitySha256 = $identitySha
            Bytes = $entry.ToArray()
        })
    }
    $rvaPattern =
        'pinned_rva!\(\s*"([^"]+)",\s*(0x[0-9A-Fa-f]+|\d+),\s*(\d+),\s*([A-Z0-9_]+|"[^"]+"),\s*"([0-9a-f]{64})"\s*\)'
    foreach ($match in [regex]::Matches(
        $rosterSource,
        $rvaPattern,
        [Text.RegularExpressions.RegexOptions]::Singleline
    )) {
        $primitiveSymbol = $match.Groups[4].Value
        if ($primitiveSymbol.StartsWith('"') -and
            $primitiveSymbol.EndsWith('"')) {
            $primitiveId = $primitiveSymbol.Substring(
                1,
                $primitiveSymbol.Length - 2
            )
        } elseif ($primitiveIds.ContainsKey($primitiveSymbol)) {
            $primitiveId = [string]$primitiveIds[$primitiveSymbol]
        } else {
            throw "Unknown pinned-hook primitive '$primitiveSymbol'."
        }
        $identitySha = Get-PassiveNamedValueSha256 `
            'battle-plan.capture_ready.hook_identity.v1' `
            $match.Groups[1].Value
        $moduleSha = Get-PassiveNamedValueSha256 `
            'battle-plan.capture_ready.hook_target_name.v1' `
            'hoi4.exe'
        $primitiveSha = Get-PassiveNamedValueSha256 `
            'battle-plan.capture_ready.hook_primitive_id.v1' `
            $primitiveId
        $entry = [Collections.Generic.List[byte]]::new()
        Add-PassiveSha256Bytes $entry $identitySha
        Add-PassiveLittleEndianBytes $entry u16 ([uint16]2)
        Add-PassiveLittleEndianBytes `
            $entry u16 ([uint16]::Parse($match.Groups[3].Value))
        Add-PassiveSha256Bytes $entry $moduleSha
        # $( ) not ( ): in argument position a bare parenthesis begins an
        # expression, so `if` was parsed as a command name and every call threw
        # "The term 'if' is not recognized". A subexpression admits statements.
        Add-PassiveLittleEndianBytes $entry u64 $(
            if ($match.Groups[2].Value.StartsWith('0x')) {
                [Convert]::ToUInt64($match.Groups[2].Value.Substring(2), 16)
            } else {
                [uint64]::Parse(
                    $match.Groups[2].Value,
                    [Globalization.CultureInfo]::InvariantCulture
                )
            }
        )
        Add-PassiveSha256Bytes $entry $primitiveSha
        $entry.AddRange([byte[]]::new(24))
        Add-PassiveSha256Bytes $entry $match.Groups[5].Value
        $entry.Add($rvaMandatory)
        $entry.AddRange([byte[]]::new(7))
        $entries.Add([pscustomobject]@{
            IdentitySha256 = $identitySha
            Bytes = $entry.ToArray()
        })
    }
    if ($entries.Count -eq 0 -or $entries.Count -gt [uint16]::MaxValue) {
        throw "Pinned production hook source roster has invalid count $($entries.Count)."
    }
    $expectedCount = [uint16]$entries.Count
    $ordered = @($entries | Sort-Object IdentitySha256)
    if (@($ordered.IdentitySha256 | Select-Object -Unique).Count -ne
        $expectedCount) {
        throw 'Pinned production hook source roster identities are not unique.'
    }
    $logicalNames = @(
        [regex]::Matches(
            $rosterSource,
            'pinned_(?:import|rva)!\(\s*"([^"]+)"',
            [Text.RegularExpressions.RegexOptions]::Singleline
        ) | ForEach-Object { $_.Groups[1].Value }
    )
    if ($logicalNames.Count -ne $expectedCount -or
        @($logicalNames | Select-Object -Unique).Count -ne $expectedCount) {
        throw 'Pinned production hook source logical-name roster is incomplete or duplicated.'
    }
    for ($index = 1; $index -lt $logicalNames.Count; $index++) {
        if ([StringComparer]::Ordinal.Compare(
            $logicalNames[$index - 1],
            $logicalNames[$index]
        ) -ge 0) {
            throw 'Pinned production hook source logical-name roster is not strictly sorted.'
        }
    }
    $forbiddenNames = @(
        $logicalNames | Where-Object {
            $_ -cmatch '(?:^|\.)a(?:25|28|30)(?:\.|$)' -or
            $_ -cmatch '(?:^|\.)render(?:\.|$)' -or
            $_ -cmatch '(?:^|\.)evidence(?:\.|$)' -or
            ($_ -cnotin @(
                'bootstrap.d3d11_create_device',
                'bootstrap.d3d_compile'
            ) -and
                $_ -cnotmatch '^presentation\.d82\.[a-z0-9_.-]+$' -and
                $_ -cnotmatch '^gameplay\.d[0-9]+\.[a-z0-9_.-]+$')
        }
    )
    if ($forbiddenNames.Count -ne 0) {
        throw "Pinned production roster contains evidence/render hooks: $($forbiddenNames -join ', ')."
    }
    $preimage = New-PassiveLabelledPreimage `
        'battle-plan.capture_ready.hook_allowlist.v1'
    Add-PassiveLittleEndianBytes $preimage u16 ([uint16]$ordered.Count)
    foreach ($entry in $ordered) {
        $preimage.AddRange([byte[]]$entry.Bytes)
    }
    if ($importSlotCount -eq 0) {
        throw 'Pinned production hook source roster declares no import-slot specs.'
    }
    return [pscustomobject]@{
        Count = [uint16]$expectedCount
        ImportSlotCount = [uint16]$importSlotCount
        Sha256 = Get-PassiveBytesSha256 $preimage.ToArray()
        LogicalNames = $logicalNames
    }
}

# The collector's compile-time feature vocabulary, PARSED from the Rust
# contract rather than restated here.
#
# The word at CaptureReadyV2 offset 20 was reserved-must-be-zero until lane EH
# gave it to `CollectorFeatureBits`. This file did not follow, and every part of
# it that touches offset 20 was then wrong at once: the encoder mirror below
# still demanded the literal `&0u32.to_le_bytes();`, the payload rebuild still
# wrote a zero there, and the receipt's two new properties failed the closed
# property set. Run t36's READY receipt was therefore refused before a single
# admission clause ran -- which took its CUSTODY authority with it, so the
# recorder's teardown receipt (57 of 57 pinned sites restored to baseline) could
# not be authenticated, the graded window never closed, the settings guard
# stayed applied and the game was left running.
#
# Restating the bit assignments as PowerShell literals would have set the same
# trap for the next feature bit. They are read out of the contract instead, so a
# bit added in Rust is known here without an edit, and a bit this file cannot
# parse is a loud failure rather than a silently narrower mask.
function Get-PassiveCollectorFeatureAuthorityFromSource {
    param([Parameter(Mandatory = $true)][string]$PayloadSource)

    $start = $PayloadSource.IndexOf('impl CollectorFeatureBits {')
    $end = $PayloadSource.IndexOf('pub struct CaptureReadyV2 {', [Math]::Max($start, 0))
    if ($start -lt 0 -or $end -le $start) {
        throw (New-PassiveReadyContractMismatch `
            -Class 'collector_feature_dialect' `
            -Field 'impl CollectorFeatureBits' `
            -Expected 'the feature impl opens before the CaptureReadyV2 struct' `
            -Observed "impl_at=$start struct_at=$end" `
            -Message 'Collector feature-bit authority boundary is unavailable.')
    }
    $implSource = $PayloadSource.Substring($start, $end - $start)

    $bits = [ordered]@{}
    foreach ($match in [regex]::Matches(
        $implSource,
        'pub const ([A-Z0-9_]+): u32 = 1 << (\d+);'
    )) {
        $shift = [int]$match.Groups[2].Value
        if ($shift -lt 0 -or $shift -gt 31) {
            throw (New-PassiveReadyContractMismatch `
                -Class 'collector_feature_dialect' `
                -Field $match.Groups[1].Value `
                -Expected 'a shift in 0..31' `
                -Observed "1 << $shift" `
                -Message "Collector feature bit '$($match.Groups[1].Value)' is not a u32 bit.")
        }
        $bits[$match.Groups[1].Value] = [uint32](1 -shl $shift)
    }
    if ($bits.Count -eq 0) {
        throw (New-PassiveReadyContractMismatch `
            -Class 'collector_feature_dialect' `
            -Field 'CollectorFeatureBits bit constants' `
            -Expected 'at least one `pub const NAME: u32 = 1 << n;`' `
            -Observed 'none' `
            -Message 'Collector feature-bit authority declares no bits.')
    }
    if (-not $bits.Contains('DECLARED') -or [uint32]$bits['DECLARED'] -ne [uint32]1) {
        throw (New-PassiveReadyContractMismatch `
            -Class 'collector_feature_dialect' `
            -Field 'DECLARED' `
            -Expected '1' `
            -Observed $(
                if ($bits.Contains('DECLARED')) {
                    [string][uint32]$bits['DECLARED']
                } else {
                    'not declared'
                }
            ) `
            -Message 'Collector feature-bit authority does not put DECLARED in bit 0.')
    }

    function Get-PassiveCollectorFeatureMask {
        param(
            [Parameter(Mandatory = $true)][string]$ImplSource,
            [Parameter(Mandatory = $true)][object]$Bits,
            [Parameter(Mandatory = $true)][string]$MaskName
        )

        $maskMatch = [regex]::Match(
            $ImplSource,
            ('pub const ' + $MaskName + ': u32 = ([^;]+);'),
            [Text.RegularExpressions.RegexOptions]::Singleline
        )
        if (-not $maskMatch.Success) {
            throw (New-PassiveReadyContractMismatch `
                -Class 'collector_feature_dialect' `
                -Field $MaskName `
                -Expected "pub const ${MaskName}: u32 = ...;" `
                -Observed 'not declared' `
                -Message "Collector feature-bit authority does not declare $MaskName.")
        }
        $mask = [uint32]0
        $expression = $maskMatch.Groups[1].Value
        $members = [regex]::Matches($expression, 'Self::([A-Z0-9_]+)')
        if ($members.Count -eq 0) {
            throw (New-PassiveReadyContractMismatch `
                -Class 'collector_feature_dialect' `
                -Field $MaskName `
                -Expected 'at least one Self::NAME member' `
                -Observed $expression.Trim() `
                -Message "Collector feature-bit authority declares an empty $MaskName.")
        }
        foreach ($member in $members) {
            $name = $member.Groups[1].Value
            if (-not $Bits.Contains($name)) {
                throw (New-PassiveReadyContractMismatch `
                    -Class 'collector_feature_dialect' `
                    -Field "$MaskName member $name" `
                    -Expected 'a bit constant this verifier already parsed' `
                    -Observed "known bits: $(@($Bits.Keys) -join ',')" `
                    -Message "Collector feature mask $MaskName names unknown bit '$name'.")
            }
            $mask = [uint32]($mask -bor [uint32]$Bits[$name])
        }
        # THE WHOLE EXPRESSION, OR NOTHING. Scanning for `Self::NAME` and
        # ignoring what is left silently narrows the mask the moment a term is
        # written any other way: spelling one member as the literal `1 << 5`
        # instead of `Self::UNSHIPPED_A31_FOCUS_PRESENTATION_EVIDENCE` is
        # identical Rust, and left this file believing KNOWN_MASK was 31 while
        # the collector used 63 -- so a receipt declaring that feature would
        # have been refused as unknown. Everything except the members and the
        # `|` that joins them has to be whitespace.
        $residue = ([regex]::Replace($expression, 'Self::[A-Z0-9_]+', '') `
            -replace '\|', '').Trim()
        if ($residue -ne '') {
            throw (New-PassiveReadyContractMismatch `
                -Class 'collector_feature_dialect' `
                -Field $MaskName `
                -Expected 'only Self::NAME members joined by |' `
                -Observed $residue `
                -Message (
                    "Collector feature mask $MaskName has terms this verifier " +
                    "cannot read: '$residue'."
                ))
        }
        return $mask
    }

    $knownMask = Get-PassiveCollectorFeatureMask `
        -ImplSource $implSource -Bits $bits -MaskName 'KNOWN_MASK'
    $diagnosticMask = Get-PassiveCollectorFeatureMask `
        -ImplSource $implSource -Bits $bits -MaskName 'DIAGNOSTIC_MASK'

    $namesStart = $implSource.IndexOf('pub const FEATURE_NAMES:')
    $namesEnd = $implSource.IndexOf('];', [Math]::Max($namesStart, 0))
    if ($namesStart -lt 0 -or $namesEnd -le $namesStart) {
        throw (New-PassiveReadyContractMismatch `
            -Class 'collector_feature_dialect' `
            -Field 'FEATURE_NAMES' `
            -Expected 'pub const FEATURE_NAMES: [...] = [ .. ];' `
            -Observed "declaration_at=$namesStart terminator_at=$namesEnd" `
            -Message 'Collector feature-bit authority does not declare FEATURE_NAMES.')
    }
    $namesSource = $implSource.Substring($namesStart, $namesEnd - $namesStart)
    $featureNames = [Collections.Generic.List[object]]::new()
    $namedMask = [uint32]0
    foreach ($match in [regex]::Matches(
        $namesSource,
        'Self::([A-Z0-9_]+)\s*,\s*"([^"]+)"',
        [Text.RegularExpressions.RegexOptions]::Singleline
    )) {
        $name = $match.Groups[1].Value
        if (-not $bits.Contains($name)) {
            throw (New-PassiveReadyContractMismatch `
                -Class 'collector_feature_dialect' `
                -Field "FEATURE_NAMES entry $name" `
                -Expected 'a bit constant this verifier already parsed' `
                -Observed "known bits: $(@($bits.Keys) -join ',')" `
                -Message "Collector FEATURE_NAMES names unknown bit '$name'.")
        }
        $bit = [uint32]$bits[$name]
        if (($namedMask -band $bit) -ne 0) {
            throw (New-PassiveReadyContractMismatch `
                -Class 'collector_feature_dialect' `
                -Field "FEATURE_NAMES entry $name" `
                -Expected 'each bit named exactly once' `
                -Observed 'named a second time' `
                -Message "Collector FEATURE_NAMES names bit '$name' twice.")
        }
        $namedMask = [uint32]($namedMask -bor $bit)
        $featureNames.Add([pscustomobject]@{
            Bit = $bit
            CargoName = $match.Groups[2].Value
        })
    }
    if ($featureNames.Count -eq 0) {
        throw (New-PassiveReadyContractMismatch `
            -Class 'collector_feature_dialect' `
            -Field 'FEATURE_NAMES' `
            -Expected 'at least one (Self::NAME, "cargo-name") entry' `
            -Observed 'none' `
            -Message 'Collector FEATURE_NAMES is empty.')
    }
    # EVERY ENTRY, OR NONE. The same failure as the mask residue above, one
    # level down: an entry whose bit is written as a literal rather than
    # `Self::NAME` matches nothing and is dropped without a word, leaving a
    # compiled feature with no name to report. The cargo-name string literals
    # are counted independently of how the bit is spelled, so a dropped entry
    # shows up as a mismatch.
    $quotedNames = [regex]::Matches($namesSource, '"[^"]*"').Count
    if ($quotedNames -ne $featureNames.Count) {
        throw (New-PassiveReadyContractMismatch `
            -Class 'collector_feature_dialect' `
            -Field 'FEATURE_NAMES' `
            -Expected "$quotedNames entries bound to a readable bit" `
            -Observed "$($featureNames.Count) bound" `
            -Message (
                "Collector FEATURE_NAMES declares $quotedNames cargo names but " +
                "$($featureNames.Count) of them are bound to a bit this verifier " +
                'can read.'
            ))
    }
    # Every bit the contract DECLARES has to be inside KNOWN_MASK. Without
    # this, a bit constant left out of the mask expression is simply invisible
    # here while the collector can still set it, and this file would refuse the
    # resulting receipt as carrying an unknown bit.
    foreach ($entry in $bits.GetEnumerator()) {
        if (([uint32]$entry.Value -band $knownMask) -eq 0) {
            throw (New-PassiveReadyContractMismatch `
                -Class 'collector_feature_dialect' `
                -Field $entry.Key `
                -Expected "inside KNOWN_MASK 0x$($knownMask.ToString('x8'))" `
                -Observed "0x$(([uint32]$entry.Value).ToString('x8'))" `
                -Message (
                    "Collector feature bit '$($entry.Key)' is declared but is not " +
                    'in KNOWN_MASK.'
                ))
        }
    }
    # DECLARED is not a cargo feature and must not be spelled as one; every
    # other known bit must be, or a set bit would have no name to report.
    if (($namedMask -band [uint32]$bits['DECLARED']) -ne 0) {
        throw (New-PassiveReadyContractMismatch `
            -Class 'collector_feature_dialect' `
            -Field 'FEATURE_NAMES/DECLARED' `
            -Expected 'DECLARED absent from FEATURE_NAMES' `
            -Observed 'DECLARED spelled as a cargo feature' `
            -Message 'Collector FEATURE_NAMES spells DECLARED as a cargo feature.')
    }
    if ([uint32]($namedMask -bor [uint32]$bits['DECLARED']) -ne $knownMask) {
        throw (New-PassiveReadyContractMismatch `
            -Class 'collector_feature_dialect' `
            -Field 'FEATURE_NAMES coverage' `
            -Expected "0x$($knownMask.ToString('x8'))" `
            -Observed "0x$(([uint32]($namedMask -bor [uint32]$bits['DECLARED'])).ToString('x8'))" `
            -Message 'Collector FEATURE_NAMES does not cover every known feature bit.')
    }
    if (($diagnosticMask -band [uint32]$bits['DECLARED']) -ne 0) {
        throw (New-PassiveReadyContractMismatch `
            -Class 'collector_feature_dialect' `
            -Field 'DIAGNOSTIC_MASK' `
            -Expected 'DECLARED excluded' `
            -Observed "0x$($diagnosticMask.ToString('x8'))" `
            -Message 'Collector DIAGNOSTIC_MASK includes DECLARED.')
    }
    if (($diagnosticMask -band (-bnot $knownMask)) -ne 0) {
        throw (New-PassiveReadyContractMismatch `
            -Class 'collector_feature_dialect' `
            -Field 'DIAGNOSTIC_MASK' `
            -Expected "a subset of KNOWN_MASK 0x$($knownMask.ToString('x8'))" `
            -Observed "0x$($diagnosticMask.ToString('x8'))" `
            -Message 'Collector DIAGNOSTIC_MASK is not a subset of KNOWN_MASK.')
    }
    $ordered = @(
        $featureNames | Sort-Object -Property { [uint32]$_.Bit }
    )
    return [pscustomobject]@{
        DeclaredBit = [uint32]$bits['DECLARED']
        KnownMask = [uint32]$knownMask
        DiagnosticMask = [uint32]$diagnosticMask
        FeatureNames = $ordered
    }
}

function Get-PassiveControlTransportPayloadSource {
    $nativeRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
    $payloadPath = Join-Path `
        $nativeRoot 'compact_contract\src\control_transport\payloads.rs'
    if (-not (Test-Path -LiteralPath $payloadPath -PathType Leaf)) {
        throw (New-PassiveReadyContractMismatch `
            -Class 'producer_sources' `
            -Field 'control_transport/payloads.rs' `
            -Expected 'readable at the canonical path' `
            -Observed "missing: $payloadPath" `
            -Message 'Passive READY canonical Rust producer sources are unavailable.')
    }
    return (Get-Content -LiteralPath $payloadPath -Raw)
}

function Get-PassiveCollectorFeatureAuthority {
    return (Get-PassiveCollectorFeatureAuthorityFromSource `
        (Get-PassiveControlTransportPayloadSource))
}

# The producer's feature word as it rides the wire at CaptureReadyV2 offset 20.
#
# ABSENT IS ZERO, AND ZERO IS A REAL VALUE. A receipt written before the build
# declaration existed carries no `collector_feature_bits` property; the payload
# it authenticated has a zero there, which is exactly what
# `CollectorFeatureBits::undeclared()` encodes. Reading the missing property as
# zero is therefore not a default, it is the observation, and it is what keeps
# every pre-declaration receipt reproducible byte for byte.
function Get-PassiveCaptureReadyFeatureBits {
    param([Parameter(Mandatory = $true)][object]$Ready)

    if ($null -eq $Ready.PSObject.Properties['collector_feature_bits']) {
        return [uint32]0
    }
    return (ConvertTo-PassiveReceiptInteger `
        $Ready.collector_feature_bits u32 'collector_feature_bits')
}

function Add-PassiveSha256Bytes {
    param(
        [Parameter(Mandatory = $true)]
        # A mandatory collection parameter also rejects an EMPTY collection in
        # Windows PowerShell, and every caller passes a freshly-created empty
        # List[byte] on its first append. Without this the whole passive READY
        # digest path threw on its first call and could never run.
        [AllowEmptyCollection()]
        [Collections.Generic.List[byte]]$Destination,
        [Parameter(Mandatory = $true)][string]$Value
    )

    $Destination.AddRange((ConvertFrom-PassiveSha256 $Value))
}

function Add-PassiveHmacSha256 {
    param(
        [Parameter(Mandatory = $true)]
        # A mandatory collection parameter also rejects an EMPTY collection in
        # Windows PowerShell, and every caller passes a freshly-created empty
        # List[byte] on its first append. Without this the whole passive READY
        # digest path threw on its first call and could never run.
        [AllowEmptyCollection()]
        [Collections.Generic.List[byte]]$Destination,
        [Parameter(Mandatory = $true)][string]$Token
    )

    $key = [Text.UTF8Encoding]::new($false).GetBytes($Token)
    $hmac = [Security.Cryptography.HMACSHA256]::new($key)
    try {
        $Destination.AddRange($hmac.ComputeHash($Destination.ToArray()))
    } finally {
        $hmac.Dispose()
    }
}

function Get-PassiveCaptureReadyPayloadSha256 {
    param(
        [Parameter(Mandatory = $true)][object]$Ready,
        [Parameter(Mandatory = $true)][string]$PipeToken
    )

    $bytes = [Collections.Generic.List[byte]]::new()
    $bytes.AddRange([Text.Encoding]::ASCII.GetBytes('H4RD'))
    Add-PassiveLittleEndianBytes $bytes u16 ([uint16]2)
    Add-PassiveLittleEndianBytes $bytes u16 ([uint16]512)
    Add-PassiveLittleEndianBytes $bytes u32 ([uint32]512)
    Add-PassiveLittleEndianBytes $bytes u32 (
        ConvertTo-PassiveReceiptInteger $Ready.readiness_flags u32 `
            'readiness_flags'
    )
    Add-PassiveLittleEndianBytes $bytes u32 (
        ConvertTo-PassiveReceiptInteger $Ready.pid u32 'pid'
    )
    # Offset 20: the collector's own feature word since lane EH, zero on every
    # receipt written before it. NOT a reserved zero -- see
    # Get-PassiveCaptureReadyFeatureBits.
    Add-PassiveLittleEndianBytes $bytes u32 (
        Get-PassiveCaptureReadyFeatureBits -Ready $Ready
    )
    Add-PassiveLittleEndianBytes $bytes u64 (
        ConvertTo-PassiveReceiptInteger `
            $Ready.process_creation_filetime_utc u64 `
            'process_creation_filetime_utc'
    )
    Add-PassiveLittleEndianBytes $bytes u32 (
        ConvertTo-PassiveReceiptInteger `
            $Ready.requested_duration_ms u32 `
            'requested_duration_ms'
    )
    Add-PassiveLittleEndianBytes $bytes u32 (
        ConvertTo-PassiveReceiptInteger `
            $Ready.validated_domain_count u32 `
            'validated_domain_count'
    )
    Add-PassiveLittleEndianBytes $bytes u32 (
        ConvertTo-PassiveReceiptInteger `
            $Ready.scheduler_queue_capacity u32 `
            'scheduler_queue_capacity'
    )
    Add-PassiveLittleEndianBytes $bytes u16 ([uint16](
        ConvertTo-PassiveReceiptInteger `
            $Ready.scheduler_worker_count u32 `
            'scheduler_worker_count'
    ))
    Add-PassiveLittleEndianBytes $bytes u16 ([uint16](
        ConvertTo-PassiveReceiptInteger `
            $Ready.allowed_hook_count u32 `
            'allowed_hook_count'
    ))
    Add-PassiveLittleEndianBytes $bytes u16 ([uint16](
        ConvertTo-PassiveReceiptInteger `
            $Ready.active_hook_count u32 `
            'active_hook_count'
    ))
    Add-PassiveLittleEndianBytes $bytes u16 ([uint16]0)
    Add-PassiveLittleEndianBytes $bytes i32 (
        ConvertTo-PassiveReceiptInteger `
            $Ready.clock_hour_raw i32 `
            'clock_hour_raw'
    )
    Add-PassiveLittleEndianBytes $bytes u64 (
        ConvertTo-PassiveReceiptInteger `
            $Ready.clock_observation_ordinal u64 `
            'clock_observation_ordinal'
    )
    foreach ($field in @(
        'target_executable_sha256',
        'collector_dll_sha256',
        'hook_allowlist_sha256',
        'hook_outcomes_sha256',
        'validated_domain_roster_sha256',
        'clock_witness_sha256',
        'scheduler_config_sha256',
        'output_pipe_identity_sha256',
        'pipe_token_sha256',
        'zero_mutation_policy_sha256',
        'session_id_sha256',
        'capture_id_sha256',
        'control_sha256'
    )) {
        Add-PassiveSha256Bytes $bytes ([string]$Ready.$field)
    }
    if ($bytes.Count -ne 480) {
        throw "Passive READY authenticated body is $($bytes.Count) bytes, not 480."
    }
    Add-PassiveHmacSha256 $bytes $PipeToken
    if ($bytes.Count -ne 512) {
        throw "Passive READY authenticated payload is $($bytes.Count) bytes, not 512."
    }
    return Get-PassiveBytesSha256 $bytes.ToArray()
}

function Get-PassiveCaptureReadyAckSha256 {
    param(
        [Parameter(Mandatory = $true)][object]$Ready,
        [Parameter(Mandatory = $true)][string]$PipeToken
    )

    $bytes = [Collections.Generic.List[byte]]::new()
    $bytes.AddRange([Text.Encoding]::ASCII.GetBytes('H4RA'))
    Add-PassiveLittleEndianBytes $bytes u16 ([uint16]2)
    Add-PassiveLittleEndianBytes $bytes u16 ([uint16]168)
    Add-PassiveLittleEndianBytes $bytes u32 ([uint32]168)
    Add-PassiveLittleEndianBytes $bytes u32 ([uint32]1)
    Add-PassiveLittleEndianBytes $bytes u32 (
        ConvertTo-PassiveReceiptInteger $Ready.pid u32 'pid'
    )
    Add-PassiveLittleEndianBytes $bytes u32 ([uint32]0)
    Add-PassiveLittleEndianBytes $bytes u64 (
        ConvertTo-PassiveReceiptInteger `
            $Ready.process_creation_filetime_utc u64 `
            'process_creation_filetime_utc'
    )
    Add-PassiveLittleEndianBytes $bytes u32 (
        ConvertTo-PassiveReceiptInteger `
            $Ready.requested_duration_ms u32 `
            'requested_duration_ms'
    )
    Add-PassiveLittleEndianBytes $bytes u32 (
        ConvertTo-PassiveReceiptInteger `
            $Ready.validated_domain_count u32 `
            'validated_domain_count'
    )
    Add-PassiveSha256Bytes $bytes ([string]$Ready.ready_payload_sha256)
    Add-PassiveSha256Bytes `
        $bytes ([string]$Ready.public_receipt_core_sha256)
    Add-PassiveSha256Bytes $bytes ([string]$Ready.session_id_sha256)
    if ($bytes.Count -ne 136) {
        throw "Passive READY ACK authenticated body is $($bytes.Count) bytes, not 136."
    }
    Add-PassiveHmacSha256 $bytes $PipeToken
    if ($bytes.Count -ne 168) {
        throw "Passive READY ACK is $($bytes.Count) bytes, not 168."
    }
    return Get-PassiveBytesSha256 $bytes.ToArray()
}

function Assert-PassiveReadySourceContractParity {
    param([Parameter(Mandatory = $true)][string]$CollectorBuildProfileId)

    $nativeRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
    $contractPath = Join-Path $nativeRoot 'compact_contract\src\passive_ready.rs'
    $profilePath = Join-Path $nativeRoot 'compact_contract\src\build_profile.rs'
    $payloadPath = Join-Path `
        $nativeRoot 'compact_contract\src\control_transport\payloads.rs'
    $hostPath = Join-Path $nativeRoot 'recorder_host\src\main.rs'
    $missingSources = @(
        @($contractPath, $profilePath, $payloadPath, $hostPath) |
            Where-Object { -not (Test-Path -LiteralPath $_ -PathType Leaf) }
    )
    if ($missingSources.Count -gt 0) {
        throw (New-PassiveReadyContractMismatch `
            -Class 'producer_sources' `
            -Field 'compact_contract/recorder_host sources' `
            -Expected 'all four canonical producer sources readable' `
            -Observed "missing: $($missingSources -join '; ')" `
            -Message 'Passive READY canonical Rust producer sources are unavailable.')
    }
    $contract = Get-Content -LiteralPath $contractPath -Raw
    $profile = Get-Content -LiteralPath $profilePath -Raw
    $payload = Get-Content -LiteralPath $payloadPath -Raw
    # NOT $host: that is a read-only PowerShell automatic variable, so assigning
    # it threw SessionStateUnauthorizedAccessException and every capture died
    # here -- after the recorder had already injected -- leaving the graded
    # window open, the settings guard applied, and no manifest on disk.
    $hostSource = Get-Content -LiteralPath $hostPath -Raw
    $requiredContractFragments = @(
        'pub const PASSIVE_READY_DIGEST_VERSION: u16 = 1;',
        'pub const PRODUCTION_DOMAIN_QUEUE_CAPACITY_V1: u32 = 1;',
        'pub const PRODUCTION_OUTPUT_QUEUE_CAPACITY_V1: u32 = 2;',
        'pub const PRODUCTION_DOMAIN_TIMEOUT_US_V1: u32 = 0;',
        'pub const PRODUCTION_PROPOSAL_POOL_CAPACITY_BYTES_V1: u64 = 256 * 1024 * 1024;',
        'b"battle-plan.capture_ready.scheduler_config.v1"',
        'b"battle-plan.capture_ready.read_only_clock_witness.v1"',
        'b"battle-plan.capture_ready.output_pipe_identity.v1"',
        'b"battle-plan.capture_ready.zero_mutation_policy.v1"'
    )
    foreach ($fragment in $requiredContractFragments) {
        Assert-PassiveProducerFragment `
            -Source $contract `
            -Fragment $fragment `
            -Class 'contract_fragment' `
            -Message "Passive READY verifier/producer source parity failed: missing '$fragment'."
    }
    # The roster parse is the same DIALECT question one level down -- can this
    # verifier still re-derive the allowlist digest from the contract's own
    # roster -- so a failure in it has to arrive at the harness classified, not
    # as an anonymous sentence. Its own checks name their subject in prose and
    # are left as they are; this gives the whole family a class and a field
    # without restating twenty individual comparisons. An already-structured
    # mismatch passes straight through so the more specific class wins.
    try {
        $hookAuthority = Get-PassivePinnedHookAllowlistAuthorityFromSource `
            $contract
    } catch {
        if ($null -ne (Get-PassiveReadyContractMismatch -ErrorRecord $_)) { throw }
        $rosterReason = ''
        try { $rosterReason = ([string]$_.Exception.Message -replace '\s+', ' ').Trim() } catch { }
        if ([string]::IsNullOrWhiteSpace($rosterReason)) {
            $rosterReason = 'the roster parse failed without a message'
        }
        throw (New-PassiveReadyContractMismatch `
            -Class 'hook_roster_dialect' `
            -Field 'PINNED_PRODUCTION_HOOK_SPECS_V1' `
            -Expected 'a roster this verifier can re-derive the allowlist digest from' `
            -Observed $rosterReason `
            -Message $rosterReason)
    }
    $profileFragment = 'pub const COLLECTOR_PROFILE_ID: &str = "' +
        $CollectorBuildProfileId + '";'
    if (-not $profile.Contains($profileFragment)) {
        $declaredProfile = [regex]::Match(
            $profile,
            'pub const COLLECTOR_PROFILE_ID: &str = "([^"]*)";'
        )
        throw (New-PassiveReadyContractMismatch `
            -Class 'build_profile' `
            -Field 'COLLECTOR_PROFILE_ID' `
            -Expected $CollectorBuildProfileId `
            -Observed $(
                if ($declaredProfile.Success) {
                    $declaredProfile.Groups[1].Value
                } else {
                    'not declared by compact_contract'
                }
            ) `
            -Message 'Passive READY verifier build-profile input differs from compact_contract.')
    }
    foreach ($fragment in @(
        'pub const CAPTURE_READY_MAGIC: &[u8; 4] = b"H4RD";',
        'pub const CAPTURE_READY_ACK_MAGIC: &[u8; 4] = b"H4RA";',
        'pub const CAPTURE_READY_VERSION: u16 = 2;',
        'pub const CAPTURE_READY_LEN: usize = 512;',
        'pub const CAPTURE_READY_ACK_LEN: usize = 168;',
        # The DECODE side of offset 20, pinned so the feature word cannot move
        # or quietly go back to reserved-must-be-zero without this file hearing
        # about it. The ACK's own offset-20 reserved check is pinned beside it:
        # the two payloads share a prefix layout and only READY gave that word
        # away.
        'collector_features: CollectorFeatureBits::from_bits(read_u32(bytes, 20))?,',
        'if read_u16(bytes, 50) != 0 {',
        'if read_u32(bytes, 20) != 0 {'
    )) {
        Assert-PassiveProducerFragment `
            -Source $payload `
            -Fragment $fragment `
            -Class 'wire_fragment' `
            -Message "Passive READY wire verifier/producer parity failed: missing '$fragment'."
    }
    # Parsed here so a contract this file cannot read is a source-parity
    # failure, reported before a capture opens a graded window, rather than a
    # surprise in the middle of receipt admission.
    [void](Get-PassiveCollectorFeatureAuthorityFromSource $payload)
    $readyEncoderStart = $payload.IndexOf('impl CaptureReadyV2 {')
    $readyEncoderEnd = $payload.IndexOf(
        'pub struct CaptureReadyAckFlags',
        $readyEncoderStart
    )
    if ($readyEncoderStart -lt 0 -or $readyEncoderEnd -le $readyEncoderStart) {
        throw (New-PassiveReadyContractMismatch `
            -Class 'encoder_boundary' `
            -Field 'impl CaptureReadyV2 { .. pub struct CaptureReadyAckFlags' `
            -Expected 'the encoder impl opens before the ACK-flags struct' `
            -Observed "impl_at=$readyEncoderStart ack_flags_at=$readyEncoderEnd" `
            -Message 'Passive READY producer encoder boundary is unavailable.')
    }
    $readyEncoder = $payload.Substring(
        $readyEncoderStart,
        $readyEncoderEnd - $readyEncoderStart
    )
    $previousOffset = -1
    # The fragment admitted immediately before this one, so a refusal can say
    # what the producer was expected to append AFTER. t36's refusal named only
    # the fragment that was missing (`&0u32` at offset 20), which reads as "this
    # line is gone" when the truth was "the contract moved and this mirror did
    # not" -- two different repairs.
    $previousFragment = '(start of the CaptureReadyV2 encoder)'
    foreach ($fragment in @(
        'out.extend_from_slice(CAPTURE_READY_MAGIC);',
        'out.extend_from_slice(&CAPTURE_READY_VERSION.to_le_bytes());',
        'out.extend_from_slice(&(CAPTURE_READY_LEN as u16).to_le_bytes());',
        'out.extend_from_slice(&(CAPTURE_READY_LEN as u32).to_le_bytes());',
        'out.extend_from_slice(&self.flags.bits().to_le_bytes());',
        'out.extend_from_slice(&self.pid.to_le_bytes());',
        # Offset 20. Reserved-must-be-zero until lane EH, the collector's
        # feature word since. The literal that used to stand here was the
        # `&0u32` spelling, and leaving it in place after the contract moved is
        # what refused run t36's READY receipt -- and with it the teardown
        # receipt that receipt's custody authorises.
        'out.extend_from_slice(&self.collector_features.bits().to_le_bytes());',
        'out.extend_from_slice(&self.process_creation_filetime_utc.to_le_bytes());',
        'out.extend_from_slice(&self.requested_duration_ms.to_le_bytes());',
        'out.extend_from_slice(&self.validated_domain_count.to_le_bytes());',
        'out.extend_from_slice(&self.scheduler_queue_capacity.to_le_bytes());',
        'out.extend_from_slice(&self.scheduler_worker_count.to_le_bytes());',
        'out.extend_from_slice(&self.allowed_hook_count.to_le_bytes());',
        'out.extend_from_slice(&self.active_hook_count.to_le_bytes());',
        'out.extend_from_slice(&0u16.to_le_bytes());',
        'out.extend_from_slice(&self.clock_hour_raw.to_le_bytes());',
        'out.extend_from_slice(&self.clock_observation_ordinal.to_le_bytes());',
        'out.extend_from_slice(&self.target_executable_sha256);',
        'out.extend_from_slice(&self.collector_dll_sha256);',
        'out.extend_from_slice(&self.hook_allowlist_sha256);',
        'out.extend_from_slice(&self.hook_outcomes_sha256);',
        'out.extend_from_slice(&self.validated_domain_roster_sha256);',
        'out.extend_from_slice(&self.clock_witness_sha256);',
        'out.extend_from_slice(&self.scheduler_config_sha256);',
        'out.extend_from_slice(&self.output_pipe_identity_sha256);',
        'out.extend_from_slice(&self.pipe_token_sha256);',
        'out.extend_from_slice(&self.zero_mutation_policy_sha256);',
        'out.extend_from_slice(&self.session_id_sha256);',
        'out.extend_from_slice(&self.capture_id_sha256);',
        'out.extend_from_slice(&self.control_sha256);'
    )) {
        $offset = $readyEncoder.IndexOf($fragment, $previousOffset + 1)
        if ($offset -le $previousOffset) {
            throw (New-PassiveReadyContractMismatch `
                -Class 'wire_field_order' `
                -Field $fragment `
                -Expected "appended after '$previousFragment'" `
                -Observed $(
                    if ($readyEncoder.IndexOf($fragment) -lt 0) {
                        'the encoder no longer appends this field at all'
                    } else {
                        'the encoder appends it earlier than the mirror expects'
                    }
                ) `
                -Message "Passive READY wire field order differs at '$fragment'.")
        }
        $previousOffset = $offset
        $previousFragment = $fragment
    }
    Assert-PassiveProducerFragment `
        -Source $hostSource `
        -Fragment '"clock_observation_ordinal": ready.clock_observation_ordinal' `
        -Class 'host_receipt_field' `
        -Message 'Passive READY receipt producer no longer emits clock_observation_ordinal.'
    return $hookAuthority
}

function ConvertFrom-PassiveSha256 {
    param(
        [Parameter(Mandatory = $true)]
        [ValidatePattern('^[0-9a-f]{64}$')]
        [string]$Value
    )

    $bytes = [byte[]]::new(32)
    for ($index = 0; $index -lt 32; $index++) {
        $bytes[$index] = [Convert]::ToByte(
            $Value.Substring($index * 2, 2),
            16
        )
    }
    return ,$bytes
}

# Which CaptureReadyV2 receipt dialect this file is looking at.
#
# There are exactly two, and the closed property set stays closed in both. A
# receipt written before lane EH carries neither declaration property; one
# written after carries both, always -- `collector_build_json` is called with
# `Some(..)` unconditionally, so an UNDECLARING collector still writes
# `collector_feature_bits: 0` and a `collector_build` block saying so. Anything
# in between is a producer this file does not know, and admitting it would mean
# guessing which half of a build declaration to believe.
function Get-PassiveCaptureReadyDeclaredProperties {
    param([Parameter(Mandatory = $true)][object]$Ready)

    $pair = @('collector_feature_bits', 'collector_build')
    $present = @(
        $pair | Where-Object { $null -ne $Ready.PSObject.Properties[$_] }
    )
    if ($present.Count -eq 0) {
        return @()
    }
    if ($present.Count -ne $pair.Count) {
        throw (
            'CaptureReadyV2 receipt carries a partial collector build ' +
            "declaration ($($present -join ', ')); the properties " +
            "'$($pair -join "', '")' are written together or not at all."
        )
    }
    return $pair
}

# What the receipt's `collector_build` block has to say about its own feature
# word, checked rather than admitted.
#
# ADMITTING A PROPERTY IS NOT CHECKING IT -- the same lesson the teardown
# receipt's import-slot count learned. This block is the AUTHORITY the manifest
# copies, and the whole point of lane EH was that a diagnostic build cannot pass
# for a production one. A `diagnostic_build: false` that nothing derives from
# the bits is exactly the silent claim it was meant to replace.
#
# Returns the failing clauses, so the caller decides what a failure means.
#
# TOTAL BY CONSTRUCTION -- it reports, it does not throw. The clauses this feeds
# are the CUSTODY accumulator, and a function that throws instead of
# accumulating takes custody down with it: that is precisely how t36 lost the
# teardown receipt it had already earned. A `collector_feature_bits` that is a
# string, or a `collector_build.feature_bits` that will not convert, is a
# refusal with a name, not an escape hatch out of the accumulator.
function Get-PassiveCollectorBuildDeclarationFailures {
    param(
        [Parameter(Mandatory = $true)][object]$Ready,
        [Parameter(Mandatory = $true)][object]$FeatureAuthority
    )

    $failures = [Collections.Generic.List[string]]::new()
    try {
        return (Get-PassiveCollectorBuildDeclarationFailuresCore `
            -Ready $Ready -FeatureAuthority $FeatureAuthority)
    } catch {
        $failures.Add(
            'collector build declaration could not be read: ' +
            (([string]$_.Exception.Message) -replace '\s+', ' ').Trim()
        )
        return $failures
    }
}

function Get-PassiveCollectorBuildDeclarationFailuresCore {
    param(
        [Parameter(Mandatory = $true)][object]$Ready,
        [Parameter(Mandatory = $true)][object]$FeatureAuthority
    )

    $failures = [Collections.Generic.List[string]]::new()
    $bits = Get-PassiveCaptureReadyFeatureBits -Ready $Ready

    # Mirrors CollectorFeatureBits::from_bits: zero is the undeclared value, any
    # other value must set DECLARED and must set nothing outside KNOWN_MASK. A
    # word this file would refuse is a word the Rust decoder refuses too, so the
    # receipt could not have been produced by a contract-conforming collector.
    $declaredBit = [uint32]$FeatureAuthority.DeclaredBit
    $knownMask = [uint32]$FeatureAuthority.KnownMask
    $unknown = [uint32]($bits -band (-bnot $knownMask))
    if ($bits -ne 0 -and $unknown -ne 0) {
        $failures.Add(
            "collector_feature_bits sets bits outside the known mask (unknown=$unknown known=$knownMask)"
        )
    }
    if ($bits -ne 0 -and ($bits -band $declaredBit) -eq 0) {
        $failures.Add(
            "collector_feature_bits is nonzero without the DECLARED bit (bits=$bits)"
        )
    }
    if ($null -eq $Ready.PSObject.Properties['collector_build']) {
        return $failures
    }

    $build = $Ready.collector_build
    if ($null -eq $build -or $build -isnot [psobject]) {
        $failures.Add('collector_build is not an object')
        return $failures
    }
    try {
        Assert-PassiveExactPropertySet `
            -Value $build `
            -Expected @(
                'declared', 'feature_bits', 'features',
                'diagnostic_build', 'source'
            ) `
            -Label 'CaptureReadyV2 collector_build'
    } catch {
        $failures.Add([string]$_.Exception.Message)
        return $failures
    }

    if ($build.source -isnot [string] -or
        [string]$build.source -cne 'capture_ready_v2.collector_features') {
        $failures.Add(
            "collector_build.source is not the authenticated payload (actual='$($build.source)')"
        )
    }
    $buildBits = ConvertTo-PassiveReceiptInteger `
        $build.feature_bits u32 'collector_build.feature_bits'
    if ($buildBits -ne $bits) {
        $failures.Add(
            "collector_build.feature_bits disagrees with collector_feature_bits (expected='$bits' actual='$buildBits')"
        )
    }
    $expectedDeclared = (($bits -band $declaredBit) -ne 0)
    if ($build.declared -isnot [bool] -or
        [bool]$build.declared -ne $expectedDeclared) {
        $failures.Add(
            "collector_build.declared is not derived from the DECLARED bit (expected='$expectedDeclared' actual='$($build.declared)')"
        )
    }
    # `null` is ABSENT and is not `false`; flattening the two is the t35 defect
    # lane EH was fixing, so it is checked in both directions.
    if (-not $expectedDeclared) {
        if ($null -ne $build.diagnostic_build) {
            $failures.Add(
                "collector_build.diagnostic_build is not null on an undeclared build (actual='$($build.diagnostic_build)')"
            )
        }
        if ($null -ne $build.features) {
            $failures.Add(
                "collector_build.features is not null on an undeclared build (actual='$($build.features)')"
            )
        }
        return $failures
    }
    $expectedDiagnostic = (
        ($bits -band [uint32]$FeatureAuthority.DiagnosticMask) -ne 0
    )
    if ($build.diagnostic_build -isnot [bool] -or
        [bool]$build.diagnostic_build -ne $expectedDiagnostic) {
        $failures.Add(
            "collector_build.diagnostic_build is not derived from DIAGNOSTIC_MASK (expected='$expectedDiagnostic' actual='$($build.diagnostic_build)')"
        )
    }
    $expectedFeatures = @(
        $FeatureAuthority.FeatureNames |
            Where-Object { ($bits -band [uint32]$_.Bit) -ne 0 } |
            ForEach-Object { [string]$_.CargoName }
    )
    $observedFeatures = @($build.features)
    if ($null -eq $build.features) {
        $failures.Add(
            'collector_build.features is null on a declared build'
        )
    # A JSON ARRAY, NOT SOMETHING ARRAY-SHAPED. `collector_build_json` maps the
    # names through `Value::from(Vec<Value>)` and so always emits an array, but
    # `@($build.features)` wraps a bare string into a one-element array just as
    # happily -- so a receipt saying `"features": "test-support"` compared equal
    # to the real `["test-support"]` and reported nothing. ConvertFrom-Json
    # leaves a JSON array as Object[], which is what is required here.
    } elseif ($build.features -isnot [array]) {
        $failures.Add(
            "collector_build.features is not a JSON array (type='$($build.features.GetType().FullName)' actual='$($build.features)')"
        )
    } elseif ($observedFeatures.Count -ne $expectedFeatures.Count) {
        $failures.Add(
            "collector_build.features has $($observedFeatures.Count) entries (expected='$($expectedFeatures -join ',')' actual='$($observedFeatures -join ',')')"
        )
    } else {
        for ($index = 0; $index -lt $expectedFeatures.Count; $index++) {
            $observed = $observedFeatures[$index]
            # A FLAT ARRAY OF STRINGS. Requiring the outer value to be an array
            # is not enough: `[string]` on a one-element `Object[]` yields the
            # contained string, so `[["test-support"],["range-tracer"]]` -- an
            # array of arrays, which `Value::from(Vec<Value>)` over cargo names
            # can never produce -- compared equal element for element and
            # reported nothing.
            if ($observed -isnot [string]) {
                $failures.Add(
                    "collector_build.features[$index] is not a string (type='$(if ($null -eq $observed) { 'null' } else { $observed.GetType().FullName })')"
                )
                break
            }
            if ([string]$observed -cne [string]$expectedFeatures[$index]) {
                $failures.Add(
                    "collector_build.features differs at index $index (expected='$($expectedFeatures -join ',')' actual='$($observedFeatures -join ',')')"
                )
                break
            }
        }
    }
    return $failures
}

function Assert-PassiveExactPropertySet {
    param(
        [Parameter(Mandatory = $true)][object]$Value,
        [Parameter(Mandatory = $true)][string[]]$Expected,
        [Parameter(Mandatory = $true)][string]$Label
    )

    $actual = @($Value.PSObject.Properties.Name | Sort-Object)
    $wanted = @($Expected | Sort-Object)
    if ($actual.Count -ne $wanted.Count) {
        throw "$Label has $($actual.Count) properties; expected exactly $($wanted.Count)."
    }
    for ($index = 0; $index -lt $wanted.Count; $index++) {
        if ([string]$actual[$index] -cne [string]$wanted[$index]) {
            throw "$Label property set differs at index $index ('$($actual[$index])' versus '$($wanted[$index])')."
        }
    }
}

function Read-PassiveLifecycleJson {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$Label
    )

    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "$Label is missing: $Path"
    }
    $length = (Get-Item -LiteralPath $Path).Length
    if ($length -le 0 -or $length -gt 16384) {
        throw "$Label has invalid length $length."
    }
    try {
        return Get-Content -LiteralPath $Path -Raw | ConvertFrom-Json
    } catch {
        throw "$Label is not valid JSON: $($_.Exception.Message)"
    }
}

function Test-PassiveCaptureReadyReceipt {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)][string]$RunDir,
        [Parameter(Mandatory = $true)][uint32]$TargetPid,
        [Parameter(Mandatory = $true)][uint64]$TargetCreationFiletimeUtc,
        [Parameter(Mandatory = $true)][string]$SessionId,
        [Parameter(Mandatory = $true)][string]$CaptureId,
        [Parameter(Mandatory = $true)][uint32]$RequestedDurationMs,
        [Parameter(Mandatory = $true)]
        [ValidatePattern('^[0-9a-f]{64}$')]
        [string]$TargetExecutableSha256,
        [Parameter(Mandatory = $true)]
        [ValidatePattern('^[0-9a-f]{64}$')]
        [string]$CollectorDllSha256,
        [Parameter(Mandatory = $true)][string[]]$ScheduledDomainNames,
        [Parameter(Mandatory = $true)][uint32]$SchedulerPollMs,
        [Parameter(Mandatory = $true)][uint32]$SchedulerQueueCapacity,
        [Parameter(Mandatory = $true)][uint32]$SchedulerWorkerCount,
        [Parameter(Mandatory = $true)][string]$CollectorControlPath,
        [ValidateNotNullOrEmpty()]
        [string]$CollectorBuildProfileId = 'vanilla_latest_2026_06_30',
        # Receives the CUSTODY authority even when the receipt is refused.
        #
        # Two different questions are answered from one receipt, and conflating
        # them stranded a capture. "Is this capture gradeable?" fails on a
        # partial hook roster, a wrong domain count, a stale clock. "Does this
        # receipt bind a named collector to THIS process, so its later teardown
        # record can be believed?" fails only on identity: schema, pid, epoch,
        # session, capture, the digest chain, the canonical allowlist.
        #
        # Run t20 failed the first (active_hook_count=51 of 55) and passed the
        # second. The function threw, the caller's READY authority stayed $null,
        # and the recorder's teardown receipt -- which proved all 55 pinned
        # sites restored to their baseline digest -- was then refused for want
        # of an authority that had in fact been established. The graded window
        # never closed, so the settings guard stayed applied and the game was left
        # running for an operator to sort out by hand.
        #
        # Writing custody out through a [ref] rather than returning it keeps the
        # refusal a THROW: no caller can accidentally treat a refused receipt as
        # an admitted one, because the only value this function ever returns is
        # an admitted authority.
        #
        # No `= $null` default. Windows PowerShell 5.1 runs the argument
        # transformation over a parameter's DEFAULT as well as over a supplied
        # value, and [ref] refuses $null: with the default present, every call
        # that omits this parameter died on "Cannot process argument
        # transformation on parameter 'CustodyAuthorityOut'. Reference type is
        # expected in argument." That is exactly what the package builder does
        # through Get-ProductionCapturePromotionEvidence, so the whole
        # production-capture promotion path could not run at all. Omitting the
        # default leaves the variable $null without a transformation, which is
        # what the guard below already expects, and a caller that does supply
        # one is still held to [ref].
        [ref]$CustodyAuthorityOut
    )

    if ($ScheduledDomainNames.Count -eq 0) {
        throw 'Passive READY authority requires a nonempty scheduled-domain roster.'
    }
    $hookAuthority = Assert-PassiveReadySourceContractParity `
        $CollectorBuildProfileId
    $readyPath = Join-Path $RunDir 'CAPTURE_READY_V2.json'
    $ready = Read-PassiveLifecycleJson `
        -Path $readyPath `
        -Label 'CaptureReadyV2 receipt'
    $readyProperties = @(
        'schema', 'status', 'capture_policy', 'pid',
        'process_creation_filetime_utc', 'session_id', 'capture_id',
        'requested_duration_ms', 'readiness_flags', 'validated_domain_count',
        'scheduler_queue_capacity', 'scheduler_worker_count',
        'allowed_hook_count', 'active_hook_count', 'clock_hour_raw',
        'clock_observation_ordinal', 'target_executable_sha256',
        'collector_dll_sha256',
        'hook_allowlist_sha256', 'hook_outcomes_sha256',
        'validated_domain_roster_sha256', 'clock_witness_sha256',
        'scheduler_config_sha256', 'output_pipe_identity_sha256',
        'zero_mutation_policy_sha256', 'session_id_sha256',
        'capture_id_sha256', 'control_sha256', 'pipe_token_sha256',
        'ready_payload_sha256', 'host_binding_sha256', 'host_validated',
        'public_receipt_core_sha256', 'ready_ack_sha256'
    )
    # Two dialects, one closed set each. See
    # Get-PassiveCaptureReadyDeclaredProperties.
    $readyProperties += (Get-PassiveCaptureReadyDeclaredProperties -Ready $ready)
    Assert-PassiveExactPropertySet `
        -Value $ready `
        -Expected $readyProperties `
        -Label 'CaptureReadyV2 receipt'
    $featureAuthority = Get-PassiveCollectorFeatureAuthority

    $digestFields = @(
        'target_executable_sha256', 'collector_dll_sha256',
        'hook_allowlist_sha256', 'hook_outcomes_sha256',
        'validated_domain_roster_sha256', 'clock_witness_sha256',
        'scheduler_config_sha256', 'output_pipe_identity_sha256',
        'zero_mutation_policy_sha256', 'session_id_sha256',
        'capture_id_sha256', 'control_sha256', 'pipe_token_sha256',
        'ready_payload_sha256', 'host_binding_sha256',
        'public_receipt_core_sha256', 'ready_ack_sha256'
    )
    foreach ($field in $digestFields) {
        $value = $ready.PSObject.Properties[$field].Value
        if ($value -isnot [string] -or
            [string]$value -cnotmatch '^[0-9a-f]{64}$' -or
            [string]$value -cmatch '^0{64}$') {
            throw "CaptureReadyV2 field '$field' is not a nonzero canonical SHA-256."
        }
    }

    # NOT $pid: read-only PowerShell automatic variable, same failure as $host.
    $receiptPid = ConvertTo-PassiveReceiptInteger $ready.pid u32 'pid'
    $creation = ConvertTo-PassiveReceiptInteger `
        $ready.process_creation_filetime_utc u64 `
        'process_creation_filetime_utc'
    $duration = ConvertTo-PassiveReceiptInteger `
        $ready.requested_duration_ms u32 `
        'requested_duration_ms'
    $flags = ConvertTo-PassiveReceiptInteger `
        $ready.readiness_flags u32 `
        'readiness_flags'
    $domainCount = ConvertTo-PassiveReceiptInteger `
        $ready.validated_domain_count u32 `
        'validated_domain_count'
    $queueCapacity = ConvertTo-PassiveReceiptInteger `
        $ready.scheduler_queue_capacity u32 `
        'scheduler_queue_capacity'
    $workerCount = ConvertTo-PassiveReceiptInteger `
        $ready.scheduler_worker_count u32 `
        'scheduler_worker_count'
    $allowedHookCount = ConvertTo-PassiveReceiptInteger `
        $ready.allowed_hook_count u32 `
        'allowed_hook_count'
    $activeHookCount = ConvertTo-PassiveReceiptInteger `
        $ready.active_hook_count u32 `
        'active_hook_count'
    $clockHourRaw = ConvertTo-PassiveReceiptInteger `
        $ready.clock_hour_raw i32 `
        'clock_hour_raw'
    $clockObservationOrdinal = ConvertTo-PassiveReceiptInteger `
        $ready.clock_observation_ordinal u64 `
        'clock_observation_ordinal'

    $expectedSessionSha = Get-PassiveUtf8Sha256 $SessionId
    $expectedCaptureSha = Get-PassiveUtf8Sha256 $CaptureId
    $expectedRosterSha = Get-PassiveUtf8Sha256 (
        $ScheduledDomainNames -join "`n"
    )
    $expectedClockSha = Get-PassiveClockWitnessSha256 `
        -ClockHourRaw $clockHourRaw `
        -ObservationOrdinal $clockObservationOrdinal `
        -TargetExecutableSha256 $TargetExecutableSha256 `
        -CollectorBuildProfileId $CollectorBuildProfileId
    $expectedSchedulerSha = Get-PassiveSchedulerConfigSha256 `
        -PollMilliseconds $SchedulerPollMs `
        -DomainQueueCapacity $SchedulerQueueCapacity `
        -WorkerCount $SchedulerWorkerCount
    $expectedMutationPolicySha = Get-PassiveZeroMutationPolicySha256
    if (-not (Test-Path -LiteralPath $CollectorControlPath -PathType Leaf)) {
        throw "CaptureReadyV2 collector control authority is missing: $CollectorControlPath"
    }
    try {
        $collectorControlBytes = [IO.File]::ReadAllBytes($CollectorControlPath)
        $collectorControlText = [Text.UTF8Encoding]::new(
            $false,
            $true
        ).GetString($collectorControlBytes)
        $collectorControl = $collectorControlText | ConvertFrom-Json
    } catch {
        throw "CaptureReadyV2 collector control authority is invalid JSON: $($_.Exception.Message)"
    }
    if ($collectorControl.pipe_token -isnot [string] -or
        [string]::IsNullOrEmpty([string]$collectorControl.pipe_token) -or
        $collectorControl.capture_ready_pipe_name_v1 -isnot [string]) {
        throw 'CaptureReadyV2 collector control lacks its pipe-token/name authority.'
    }
    $pipeToken = [string]$collectorControl.pipe_token
    $expectedPipeName = "\\.\pipe\battle-plan_control_$TargetPid`_$pipeToken"
    if ([string]$collectorControl.capture_ready_pipe_name_v1 -cne
        $expectedPipeName) {
        throw 'CaptureReadyV2 collector control pipe name differs from canonical host derivation.'
    }
    $expectedPipeTokenSha = Get-PassiveUtf8Sha256 $pipeToken
    $expectedOutputPipeIdentitySha = Get-PassiveOutputPipeIdentitySha256 `
        -CanonicalPipeName $expectedPipeName `
        -TargetPid $TargetPid `
        -TargetCreationFiletimeUtc $TargetCreationFiletimeUtc `
        -SessionId $SessionId
    $expectedControlSha = Get-PassiveBytesSha256 $collectorControlBytes
    $expectedReadyPayloadSha = Get-PassiveCaptureReadyPayloadSha256 `
        -Ready $ready `
        -PipeToken $pipeToken
    $expectedReadyAckSha = Get-PassiveCaptureReadyAckSha256 `
        -Ready $ready `
        -PipeToken $pipeToken

    $hostBindingBytes = [Collections.Generic.List[byte]]::new()
    $utf8 = [Text.UTF8Encoding]::new($false)
    $hostBindingBytes.AddRange($utf8.GetBytes(
        "battle-plan.capture_ready.host_binding.v2`0"
    ))
    $hostBindingBytes.AddRange(
        (ConvertFrom-PassiveSha256 $ready.ready_payload_sha256)
    )
    Add-PassiveLittleEndianBytes $hostBindingBytes u32 $TargetPid
    Add-PassiveLittleEndianBytes `
        $hostBindingBytes u64 $TargetCreationFiletimeUtc
    Add-PassiveLittleEndianBytes `
        $hostBindingBytes u32 $RequestedDurationMs
    foreach ($digest in @(
        $TargetExecutableSha256,
        $CollectorDllSha256,
        $expectedSessionSha,
        $expectedCaptureSha,
        [string]$ready.pipe_token_sha256
    )) {
        $hostBindingBytes.AddRange((ConvertFrom-PassiveSha256 $digest))
    }
    $expectedHostBindingSha = Get-PassiveBytesSha256 `
        $hostBindingBytes.ToArray()

    # One clause per admitted property, each naming itself. A single fused
    # `-or` chain admits and rejects identically, so a rejection said only that
    # one of forty-one things was wrong -- which is the same as saying nothing,
    # and cost a whole graded window per diagnostic bisection. The admission
    # decision is unchanged: any failing clause still rejects the receipt.
    #
    # The accumulator is a List passed explicitly to the recorder. A nested
    # function assigning to `$script:`-scoped state would write a DIFFERENT
    # variable from the local this function later tests, so the guard would
    # collect nothing and admit every receipt -- a guard that cannot fail.
    #
    # TWO accumulators, one per question (see -CustodyAuthorityOut above).
    # $custodyFailures holds the clauses that decide whether this receipt names
    # THIS collector in THIS process at all; $gradingFailures holds the clauses
    # that decide whether the capture it opens is worth grading. Any failure in
    # either still refuses the receipt -- the admission decision is unchanged.
    # The split only decides whether custody survives the refusal.
    $custodyFailures = [Collections.Generic.List[string]]::new()
    $gradingFailures = [Collections.Generic.List[string]]::new()
    function Add-PassiveAdmissionFailure {
        param(
            [Parameter(Mandatory = $true)]
            # A mandatory collection parameter also rejects an EMPTY collection
            # in Windows PowerShell, exactly as documented on
            # Add-PassiveSha256Bytes above -- and the accumulator is empty on
            # the very first clause, which is the common case of a receipt that
            # is about to be admitted.
            [AllowEmptyCollection()]
            [Collections.Generic.List[string]]$Failures,
            [Parameter(Mandatory = $true)][string]$Clause,
            [Parameter(Mandatory = $true)][bool]$Failed,
            $Expected = $null,
            $Actual = $null
        )
        if (-not $Failed) { return }
        if ($null -eq $Expected -and $null -eq $Actual) {
            $Failures.Add($Clause)
        } else {
            $Failures.Add("$Clause (expected='$Expected' actual='$Actual')")
        }
    }

    Add-PassiveAdmissionFailure $custodyFailures 'schema' `
        ($ready.schema -isnot [string] -or
            [string]$ready.schema -cne 'hoi4-capture-ready-v2') `
        'hoi4-capture-ready-v2' $ready.schema
    Add-PassiveAdmissionFailure $custodyFailures 'status' `
        ($ready.status -isnot [string] -or [string]$ready.status -cne 'ready') `
        'ready' $ready.status
    Add-PassiveAdmissionFailure $custodyFailures 'capture_policy' `
        ($ready.capture_policy -isnot [string] -or
            [string]$ready.capture_policy -cne 'passive-observer-only') `
        'passive-observer-only' $ready.capture_policy
    Add-PassiveAdmissionFailure $custodyFailures 'pid' `
        ($receiptPid -ne $TargetPid) $TargetPid $receiptPid
    Add-PassiveAdmissionFailure $custodyFailures 'process_creation_filetime_utc' `
        ($creation -ne $TargetCreationFiletimeUtc) `
        $TargetCreationFiletimeUtc $creation
    Add-PassiveAdmissionFailure $custodyFailures 'session_id' `
        ($ready.session_id -isnot [string] -or
            [string]$ready.session_id -cne $SessionId) `
        $SessionId $ready.session_id
    Add-PassiveAdmissionFailure $custodyFailures 'capture_id' `
        ($ready.capture_id -isnot [string] -or
            [string]$ready.capture_id -cne $CaptureId) `
        $CaptureId $ready.capture_id
    # Custody, not grading: the requested duration is one of the fields hashed
    # into host_binding_sha256 below, so a receipt that disagrees with the
    # harness here cannot bind to this host at all.
    Add-PassiveAdmissionFailure $custodyFailures 'requested_duration_ms' `
        ($duration -ne $RequestedDurationMs) $RequestedDurationMs $duration
    Add-PassiveAdmissionFailure $gradingFailures 'readiness_flags' `
        ($flags -ne 255) 255 $flags
    Add-PassiveAdmissionFailure $gradingFailures 'validated_domain_count' `
        ($domainCount -ne [uint32]$ScheduledDomainNames.Count) `
        ([uint32]$ScheduledDomainNames.Count) $domainCount
    Add-PassiveAdmissionFailure $gradingFailures 'scheduler_queue_capacity' `
        ($queueCapacity -ne $SchedulerQueueCapacity) `
        $SchedulerQueueCapacity $queueCapacity
    Add-PassiveAdmissionFailure $gradingFailures 'scheduler_worker_count' `
        ($workerCount -ne $SchedulerWorkerCount) `
        $SchedulerWorkerCount $workerCount
    # Custody: this is the recorder saying which roster it was WILLING to
    # install, and the teardown record has to restore exactly that many pinned
    # sites. A recorder working from a different allowlist is a different
    # collector and its teardown record is not about the same sites.
    Add-PassiveAdmissionFailure $custodyFailures 'allowed_hook_count' `
        ($allowedHookCount -ne [uint32]$hookAuthority.Count) `
        ([uint32]$hookAuthority.Count) $allowedHookCount
    # Grading, and this is the clause that ended t20 at 51 of 55. A partial
    # roster means the capture cannot be graded -- domains behind the four
    # missing hooks are silent, not empty. It says nothing about custody: the
    # collector still pinned, and must still restore, all $allowedHookCount
    # sites, which is what the teardown record is checked against.
    Add-PassiveAdmissionFailure $gradingFailures 'active_hook_count' `
        ($activeHookCount -ne $allowedHookCount) $allowedHookCount $activeHookCount
    # More live hooks than the allowlist permits is the one direction that IS a
    # custody fact: the collector instrumented something no roster declared, so
    # nothing bounds what its teardown has to put back.
    Add-PassiveAdmissionFailure $custodyFailures 'active_hook_count exceeds the allowlist' `
        ($activeHookCount -gt $allowedHookCount) "<= $allowedHookCount" $activeHookCount
    Add-PassiveAdmissionFailure $gradingFailures 'clock_hour_raw' `
        ($clockHourRaw -lt 60005999 -or $clockHourRaw -gt 63071999) `
        '60005999..63071999' $clockHourRaw
    Add-PassiveAdmissionFailure $gradingFailures 'clock_observation_ordinal' `
        ($clockObservationOrdinal -eq [uint64]::MaxValue) `
        'not u64::MAX' $clockObservationOrdinal
    Add-PassiveAdmissionFailure $custodyFailures 'host_validated' `
        ($ready.host_validated -isnot [bool] -or
            -not [bool]$ready.host_validated) `
        $true $ready.host_validated
    # CUSTODY, not grading. This word is inside the authenticated payload and
    # therefore inside `ready_payload_sha256`, `host_binding_sha256` and the
    # ACK -- the whole chain that says this receipt belongs to this collector in
    # this process. A build declaration this file cannot reconcile means the
    # bytes it rebuilt are not the bytes the collector signed, and every digest
    # clause below would be answering about a different payload.
    foreach ($clause in (Get-PassiveCollectorBuildDeclarationFailures `
            -Ready $ready -FeatureAuthority $featureAuthority)) {
        Add-PassiveAdmissionFailure $custodyFailures $clause $true
    }
    Add-PassiveAdmissionFailure $custodyFailures 'target_executable_sha256 is the pinned a729 build' `
        ($TargetExecutableSha256 -cne
            '870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad') `
        '870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad' `
        $TargetExecutableSha256

    $digestClauses = [ordered]@{
        'target_executable_sha256'       = $TargetExecutableSha256
        'collector_dll_sha256'           = $CollectorDllSha256
        'validated_domain_roster_sha256' = $expectedRosterSha
        'hook_allowlist_sha256'          = [string]$hookAuthority.Sha256
        'clock_witness_sha256'           = $expectedClockSha
        'scheduler_config_sha256'        = $expectedSchedulerSha
        'output_pipe_identity_sha256'    = $expectedOutputPipeIdentitySha
        'pipe_token_sha256'              = $expectedPipeTokenSha
        'zero_mutation_policy_sha256'    = $expectedMutationPolicySha
        'session_id_sha256'              = $expectedSessionSha
        'capture_id_sha256'              = $expectedCaptureSha
        'control_sha256'                 = $expectedControlSha
        'ready_payload_sha256'           = $expectedReadyPayloadSha
        'ready_ack_sha256'               = $expectedReadyAckSha
        'host_binding_sha256'            = $expectedHostBindingSha
    }
    # Every digest is custody: the chain is what ties this receipt to this
    # process, this collector build, this pipe and this host.
    foreach ($clause in $digestClauses.GetEnumerator()) {
        $observed = [string]$ready.PSObject.Properties[$clause.Key].Value
        Add-PassiveAdmissionFailure $custodyFailures $clause.Key `
            ($observed -cne [string]$clause.Value) `
            ([string]$clause.Value) $observed
    }

    $admissionFailures = [Collections.Generic.List[string]]::new()
    $admissionFailures.AddRange($custodyFailures)
    $admissionFailures.AddRange($gradingFailures)
    $captureAdmitted = ($admissionFailures.Count -eq 0)
    $authority = [pscustomobject]@{
        Schema = 'hoi4-passive-lifecycle-ready-authority-v1'
        ReceiptPath = [IO.Path]::GetFullPath($readyPath)
        ReceiptSha256 = (
            Get-FileHash -LiteralPath $readyPath -Algorithm SHA256
        ).Hash.ToLowerInvariant()
        ReadyPayloadSha256 = [string]$ready.ready_payload_sha256
        PublicReceiptCoreSha256 = [string]$ready.public_receipt_core_sha256
        ReadyAckSha256 = [string]$ready.ready_ack_sha256
        SessionIdSha256 = [string]$ready.session_id_sha256
        CaptureIdSha256 = [string]$ready.capture_id_sha256
        PipeTokenSha256 = [string]$ready.pipe_token_sha256
        HostBindingSha256 = [string]$ready.host_binding_sha256
        CollectorDllSha256 = [string]$ready.collector_dll_sha256
        TargetExecutableSha256 = [string]$ready.target_executable_sha256
        ExpectedHookCount = [uint32]$hookAuthority.Count
        ExpectedImportSlotCount = [uint32]$hookAuthority.ImportSlotCount
        HookAllowlistSha256 = [string]$hookAuthority.Sha256
        AllowedHookCount = [uint32]$allowedHookCount
        ActiveHookCount = [uint32]$activeHookCount
        # $true only on the value this function RETURNS. A custody authority
        # handed out through -CustodyAuthorityOut always carries $false, so a
        # consumer that reads it can never mistake a refused capture for an
        # admitted one.
        CaptureAdmitted = $captureAdmitted
        AdmissionFailures = [string[]]$admissionFailures.ToArray()
    }

    # Publish custody BEFORE the refusal. The whole point is that the caller
    # still knows which collector is in which process when the capture itself
    # is refused; publishing after the throw would publish nothing.
    if ($custodyFailures.Count -eq 0 -and $null -ne $CustodyAuthorityOut) {
        $CustodyAuthorityOut.Value = $authority
    }

    if ($admissionFailures.Count -ne 0) {
        $custodyNote = if ($custodyFailures.Count -eq 0) {
            'READY custody is established, so the collector teardown receipt remains admissible'
        } else {
            'READY custody is NOT established'
        }
        # Structured, so the harness's refusal receipt can name each clause and
        # the side it fell on without re-parsing this sentence. The sentence
        # itself is unchanged: it is what every existing caller prints.
        $refusalMessage = (
            "CaptureReadyV2 failed passive admission on " +
            "$($admissionFailures.Count) clause(s): " +
            ($admissionFailures -join '; ') +
            ". $custodyNote."
        )
        throw (New-PassiveReadyAdmissionRefusal `
            -Message $refusalMessage `
            -CustodyFailures $custodyFailures `
            -GradingFailures $gradingFailures)
    }

    return $authority
}

function Test-PassiveCaptureTeardownReceipt {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)][string]$RunDir,
        [Parameter(Mandatory = $true)][uint32]$TargetPid,
        [Parameter(Mandatory = $true)][uint64]$TargetCreationFiletimeUtc,
        [Parameter(Mandatory = $true)][string]$SessionId,
        [Parameter(Mandatory = $true)][string]$CaptureId,
        [Parameter(Mandatory = $true)][object]$ReadyAuthority
    )

    if ([string]$ReadyAuthority.Schema -cne
        'hoi4-passive-lifecycle-ready-authority-v1') {
        throw 'Teardown verification requires admitted READY authority.'
    }
    $expectedHookCount = [uint32]$ReadyAuthority.ExpectedHookCount
    # The roster this teardown has to account for is the ALLOWED one, and it has
    # to be the canonical roster. How many of those hooks actually took is not a
    # custody fact and is deliberately not required to be all of them: the
    # collector pins every allowed site whether or not its detour installed, so
    # `pinned_hook_site_count` below is still checked against the full roster
    # and `pinned_hook_site_restored_sha256` still has to equal the baseline
    # over all of them. Requiring ActiveHookCount == expected here refused the
    # t20 teardown record -- which proved all 55 sites restored -- purely
    # because 51 of 55 detours installed, and that left a real hook-custody
    # question unanswerable on a run that had in fact answered it.
    #
    # The other direction stays fatal: more live hooks than the allowlist
    # permits means something outside the roster was instrumented, and nothing
    # then bounds what the teardown had to put back.
    if ($expectedHookCount -eq 0 -or
        [uint32]$ReadyAuthority.AllowedHookCount -ne $expectedHookCount -or
        [uint32]$ReadyAuthority.ActiveHookCount -gt $expectedHookCount) {
        throw 'Teardown verification requires canonical hook-roster READY authority.'
    }
    $teardownPath = Join-Path $RunDir 'CAPTURE_TEARDOWN_V1.json'
    $teardown = Read-PassiveLifecycleJson `
        -Path $teardownPath `
        -Label 'capture teardown receipt'
    $teardownProperties = @(
        'schema', 'status', 'capture_policy', 'pid',
        'process_creation_filetime_utc', 'session_id', 'capture_id',
        'session_id_sha256', 'capture_id_sha256', 'pipe_token_sha256',
        'collector_dll_sha256', 'capture_ready_payload_sha256',
        'capture_ready_receipt_core_sha256', 'capture_ready_ack_sha256',
        'capture_ready_receipt_sha256', 'host_binding_sha256',
        'data_pipe_eof_observed', 'hook_lease_released_after_eof',
        'collector_module_disposition',
        'collector_module_retention_policy',
        'target_process_epoch_revalidated',
        'target_code_temporarily_instrumented', 'pinned_hook_target_code_restored',
        'semantic_game_state_mutated', 'pinned_hook_sites_restored',
        'pinned_hook_site_count', 'pinned_hook_site_baseline_sha256',
        'pinned_hook_site_restored_sha256',
        # Lane AS 2026-08-05: the restoration proof now verifies IAT slots
        # for ImportExport specs (previously vacuous -- the snapshot took the
        # exported function's prologue while the primitive writes only an IAT
        # store). The count of verified import slots rides the receipt. t24
        # was refused 29-vs-28 by this exact-property pin before this line.
        'pinned_hook_import_slot_count'
    )
    Assert-PassiveExactPropertySet `
        -Value $teardown `
        -Expected $teardownProperties `
        -Label 'capture teardown receipt'
    # NOT $pid: read-only PowerShell automatic variable, same failure as $host.
    $receiptPid = ConvertTo-PassiveReceiptInteger $teardown.pid u32 'teardown.pid'
    $creation = ConvertTo-PassiveReceiptInteger `
        $teardown.process_creation_filetime_utc u64 `
        'teardown.process_creation_filetime_utc'
    $pinnedHookSiteCount = ConvertTo-PassiveReceiptInteger `
        $teardown.pinned_hook_site_count u32 `
        'teardown.pinned_hook_site_count'
    # ADMITTING A PROPERTY IS NOT CHECKING IT. `pinned_hook_import_slot_count`
    # was added to $teardownProperties so the exact-property pin would stop
    # refusing the receipt, and then nothing read it: a receipt claiming ZERO
    # verified import slots -- i.e. that the IAT verification lane AS added
    # never ran -- passed teardown verification and returned custody authority.
    # It is the count of the ONLY thing the ImportExport primitive mutates, so
    # it is checked against the canonical roster like every other count here.
    $pinnedHookImportSlotCount = ConvertTo-PassiveReceiptInteger `
        $teardown.pinned_hook_import_slot_count u32 `
        'teardown.pinned_hook_import_slot_count'
    $expectedImportSlotCount = [uint32]$ReadyAuthority.ExpectedImportSlotCount
    if ($expectedImportSlotCount -eq 0) {
        throw 'Teardown verification requires a canonical import-slot count.'
    }
    if ($teardown.schema -isnot [string] -or
        [string]$teardown.schema -cne 'hoi4-capture-teardown-v2' -or
        $teardown.status -isnot [string] -or
        ([string]$teardown.status -cne 'collector-retained-pinned' -and
         [string]$teardown.status -cne 'collector-unloaded') -or
        $teardown.capture_policy -isnot [string] -or
        [string]$teardown.capture_policy -cne 'passive-observer-only' -or
        $receiptPid -ne $TargetPid -or
        $creation -ne $TargetCreationFiletimeUtc -or
        $teardown.session_id -isnot [string] -or
        [string]$teardown.session_id -cne $SessionId -or
        $teardown.capture_id -isnot [string] -or
        [string]$teardown.capture_id -cne $CaptureId -or
        $teardown.data_pipe_eof_observed -isnot [bool] -or
        -not [bool]$teardown.data_pipe_eof_observed -or
        $teardown.hook_lease_released_after_eof -isnot [bool] -or
        -not [bool]$teardown.hook_lease_released_after_eof -or
        $teardown.collector_module_disposition -isnot [string] -or
        ([string]$teardown.collector_module_disposition -cne 'retained-pinned' -and
         [string]$teardown.collector_module_disposition -cne 'unloaded') -or
        ([string]$teardown.status -ceq 'collector-retained-pinned' -and
         [string]$teardown.collector_module_disposition -cne 'retained-pinned') -or
        ([string]$teardown.status -ceq 'collector-unloaded' -and
         [string]$teardown.collector_module_disposition -cne 'unloaded') -or
        $teardown.collector_module_retention_policy -isnot [string] -or
        [string]$teardown.collector_module_retention_policy -cne
            'inline-detour.module.process-lifetime.v1' -or
        $teardown.target_process_epoch_revalidated -isnot [bool] -or
        -not [bool]$teardown.target_process_epoch_revalidated -or
        $teardown.target_code_temporarily_instrumented -isnot [bool] -or
        -not [bool]$teardown.target_code_temporarily_instrumented -or
        $teardown.pinned_hook_target_code_restored -isnot [bool] -or
        -not [bool]$teardown.pinned_hook_target_code_restored -or
        # The producer's legacy false value is not positive evidence and is
        # deliberately not returned in TeardownAuthority. A true value remains
        # a one-way safety rejection.
        $teardown.semantic_game_state_mutated -isnot [bool] -or
        [bool]$teardown.semantic_game_state_mutated -or
        $teardown.pinned_hook_sites_restored -isnot [bool] -or
        -not [bool]$teardown.pinned_hook_sites_restored -or
        $pinnedHookSiteCount -ne $expectedHookCount -or
        $pinnedHookImportSlotCount -ne $expectedImportSlotCount -or
        $teardown.pinned_hook_site_baseline_sha256 -isnot [string] -or
        [string]$teardown.pinned_hook_site_baseline_sha256 -cnotmatch '^[0-9a-f]{64}$' -or
        $teardown.pinned_hook_site_restored_sha256 -isnot [string] -or
        [string]$teardown.pinned_hook_site_restored_sha256 -cne
            [string]$teardown.pinned_hook_site_baseline_sha256 -or
        [string]$teardown.capture_ready_receipt_sha256 -cne
            [string]$ReadyAuthority.ReceiptSha256 -or
        [string]$teardown.capture_ready_payload_sha256 -cne
            [string]$ReadyAuthority.ReadyPayloadSha256 -or
        [string]$teardown.capture_ready_receipt_core_sha256 -cne
            [string]$ReadyAuthority.PublicReceiptCoreSha256 -or
        [string]$teardown.capture_ready_ack_sha256 -cne
            [string]$ReadyAuthority.ReadyAckSha256 -or
        [string]$teardown.session_id_sha256 -cne
            [string]$ReadyAuthority.SessionIdSha256 -or
        [string]$teardown.capture_id_sha256 -cne
            [string]$ReadyAuthority.CaptureIdSha256 -or
        [string]$teardown.pipe_token_sha256 -cne
            [string]$ReadyAuthority.PipeTokenSha256 -or
        [string]$teardown.collector_dll_sha256 -cne
            [string]$ReadyAuthority.CollectorDllSha256 -or
        [string]$teardown.host_binding_sha256 -cne
            [string]$ReadyAuthority.HostBindingSha256) {
        throw 'Teardown receipt failed schema, target epoch, identity, EOF, hook release, DLL unload, READY, or host-binding admission.'
    }

    return [pscustomobject]@{
        Schema = 'hoi4-passive-lifecycle-teardown-authority-v1'
        ReceiptPath = [IO.Path]::GetFullPath($teardownPath)
        ReceiptSha256 = (
            Get-FileHash -LiteralPath $teardownPath -Algorithm SHA256
        ).Hash.ToLowerInvariant()
        DataPipeEofObserved = $true
        HookLeaseReleasedAfterEof = $true
        CollectorModuleDisposition    = [string]$teardown.collector_module_disposition
        TargetProcessEpochRevalidated = $true
        TargetCodeTemporarilyInstrumented = $true
        PinnedHookTargetCodeRestored = $true
        PinnedHookSitesRestored = $true
        PinnedHookSiteCount = [uint32]$expectedHookCount
        PinnedHookSiteBaselineSha256 = [string]$teardown.pinned_hook_site_baseline_sha256
        PinnedHookSiteRestoredSha256 = [string]$teardown.pinned_hook_site_restored_sha256
    }
}
