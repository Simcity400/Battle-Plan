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
    $importPattern =
        'pinned_import!\(\s*"([^"]+)",\s*"([^"]+)",\s*"([^"]+)",\s*"([0-9a-f]{64})"\s*\)'
    foreach ($match in [regex]::Matches(
        $rosterSource,
        $importPattern,
        [Text.RegularExpressions.RegexOptions]::Singleline
    )) {
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
        $entry.Add([byte]1)
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
        Add-PassiveLittleEndianBytes $entry u64 (
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
        $entry.Add([byte]1)
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
    return [pscustomobject]@{
        Count = [uint16]$expectedCount
        Sha256 = Get-PassiveBytesSha256 $preimage.ToArray()
        LogicalNames = $logicalNames
    }
}

function Add-PassiveSha256Bytes {
    param(
        [Parameter(Mandatory = $true)]
        [Collections.Generic.List[byte]]$Destination,
        [Parameter(Mandatory = $true)][string]$Value
    )

    $Destination.AddRange((ConvertFrom-PassiveSha256 $Value))
}

function Add-PassiveHmacSha256 {
    param(
        [Parameter(Mandatory = $true)]
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
    if (-not (Test-Path -LiteralPath $contractPath -PathType Leaf) -or
        -not (Test-Path -LiteralPath $profilePath -PathType Leaf) -or
        -not (Test-Path -LiteralPath $payloadPath -PathType Leaf) -or
        -not (Test-Path -LiteralPath $hostPath -PathType Leaf)) {
        throw 'Passive READY canonical Rust producer sources are unavailable.'
    }
    $contract = Get-Content -LiteralPath $contractPath -Raw
    $profile = Get-Content -LiteralPath $profilePath -Raw
    $payload = Get-Content -LiteralPath $payloadPath -Raw
    $host = Get-Content -LiteralPath $hostPath -Raw
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
        if (-not $contract.Contains($fragment)) {
            throw "Passive READY verifier/producer source parity failed: missing '$fragment'."
        }
    }
    $hookAuthority = Get-PassivePinnedHookAllowlistAuthorityFromSource `
        $contract
    $profileFragment = 'pub const COLLECTOR_PROFILE_ID: &str = "' +
        $CollectorBuildProfileId + '";'
    if (-not $profile.Contains($profileFragment)) {
        throw 'Passive READY verifier build-profile input differs from compact_contract.'
    }
    foreach ($fragment in @(
        'pub const CAPTURE_READY_MAGIC: &[u8; 4] = b"H4RD";',
        'pub const CAPTURE_READY_ACK_MAGIC: &[u8; 4] = b"H4RA";',
        'pub const CAPTURE_READY_VERSION: u16 = 2;',
        'pub const CAPTURE_READY_LEN: usize = 512;',
        'pub const CAPTURE_READY_ACK_LEN: usize = 168;'
    )) {
        if (-not $payload.Contains($fragment)) {
            throw "Passive READY wire verifier/producer parity failed: missing '$fragment'."
        }
    }
    $readyEncoderStart = $payload.IndexOf('impl CaptureReadyV2 {')
    $readyEncoderEnd = $payload.IndexOf(
        'pub struct CaptureReadyAckFlags',
        $readyEncoderStart
    )
    if ($readyEncoderStart -lt 0 -or $readyEncoderEnd -le $readyEncoderStart) {
        throw 'Passive READY producer encoder boundary is unavailable.'
    }
    $readyEncoder = $payload.Substring(
        $readyEncoderStart,
        $readyEncoderEnd - $readyEncoderStart
    )
    $previousOffset = -1
    foreach ($fragment in @(
        'out.extend_from_slice(CAPTURE_READY_MAGIC);',
        'out.extend_from_slice(&CAPTURE_READY_VERSION.to_le_bytes());',
        'out.extend_from_slice(&(CAPTURE_READY_LEN as u16).to_le_bytes());',
        'out.extend_from_slice(&(CAPTURE_READY_LEN as u32).to_le_bytes());',
        'out.extend_from_slice(&self.flags.bits().to_le_bytes());',
        'out.extend_from_slice(&self.pid.to_le_bytes());',
        'out.extend_from_slice(&0u32.to_le_bytes());',
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
            throw "Passive READY wire field order differs at '$fragment'."
        }
        $previousOffset = $offset
    }
    if (-not $host.Contains(
        '"clock_observation_ordinal": ready.clock_observation_ordinal'
    )) {
        throw 'Passive READY receipt producer no longer emits clock_observation_ordinal.'
    }
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
        [string]$CollectorBuildProfileId = 'vanilla_latest_2026_06_30'
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
    Assert-PassiveExactPropertySet `
        -Value $ready `
        -Expected $readyProperties `
        -Label 'CaptureReadyV2 receipt'

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

    $pid = ConvertTo-PassiveReceiptInteger $ready.pid u32 'pid'
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

    if ($ready.schema -isnot [string] -or
        [string]$ready.schema -cne 'hoi4-capture-ready-v2' -or
        $ready.status -isnot [string] -or
        [string]$ready.status -cne 'ready' -or
        $ready.capture_policy -isnot [string] -or
        [string]$ready.capture_policy -cne 'passive-observer-only' -or
        $pid -ne $TargetPid -or
        $creation -ne $TargetCreationFiletimeUtc -or
        $ready.session_id -isnot [string] -or
        [string]$ready.session_id -cne $SessionId -or
        $ready.capture_id -isnot [string] -or
        [string]$ready.capture_id -cne $CaptureId -or
        $duration -ne $RequestedDurationMs -or
        $flags -ne 255 -or
        $domainCount -ne [uint32]$ScheduledDomainNames.Count -or
        $queueCapacity -ne $SchedulerQueueCapacity -or
        $workerCount -ne $SchedulerWorkerCount -or
        $allowedHookCount -ne [uint32]$hookAuthority.Count -or
        $activeHookCount -ne $allowedHookCount -or
        $clockHourRaw -lt 60005999 -or
        $clockHourRaw -gt 63071999 -or
        $clockObservationOrdinal -eq [uint64]::MaxValue -or
        $ready.host_validated -isnot [bool] -or
        -not [bool]$ready.host_validated -or
        $TargetExecutableSha256 -cne
            '870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad' -or
        [string]$ready.target_executable_sha256 -cne $TargetExecutableSha256 -or
        [string]$ready.collector_dll_sha256 -cne $CollectorDllSha256 -or
        [string]$ready.validated_domain_roster_sha256 -cne $expectedRosterSha -or
        [string]$ready.hook_allowlist_sha256 -cne
            [string]$hookAuthority.Sha256 -or
        [string]$ready.clock_witness_sha256 -cne $expectedClockSha -or
        [string]$ready.scheduler_config_sha256 -cne $expectedSchedulerSha -or
        [string]$ready.output_pipe_identity_sha256 -cne
            $expectedOutputPipeIdentitySha -or
        [string]$ready.pipe_token_sha256 -cne $expectedPipeTokenSha -or
        [string]$ready.zero_mutation_policy_sha256 -cne $expectedMutationPolicySha -or
        [string]$ready.session_id_sha256 -cne $expectedSessionSha -or
        [string]$ready.capture_id_sha256 -cne $expectedCaptureSha -or
        [string]$ready.control_sha256 -cne $expectedControlSha -or
        [string]$ready.ready_payload_sha256 -cne $expectedReadyPayloadSha -or
        [string]$ready.ready_ack_sha256 -cne $expectedReadyAckSha -or
        [string]$ready.host_binding_sha256 -cne $expectedHostBindingSha) {
        throw 'CaptureReadyV2 failed passive identity, epoch, configuration, hook, roster, scheduler, zero-mutation, control, or host-binding admission.'
    }

    return [pscustomobject]@{
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
        HookAllowlistSha256 = [string]$hookAuthority.Sha256
        AllowedHookCount = [uint32]$allowedHookCount
        ActiveHookCount = [uint32]$activeHookCount
    }
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
    if ($expectedHookCount -eq 0 -or
        [uint32]$ReadyAuthority.AllowedHookCount -ne $expectedHookCount -or
        [uint32]$ReadyAuthority.ActiveHookCount -ne $expectedHookCount) {
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
        'collector_module_absent_verified',
        'target_process_epoch_revalidated',
        'target_code_temporarily_instrumented', 'pinned_hook_target_code_restored',
        'semantic_game_state_mutated', 'pinned_hook_sites_restored',
        'pinned_hook_site_count', 'pinned_hook_site_baseline_sha256',
        'pinned_hook_site_restored_sha256'
    )
    Assert-PassiveExactPropertySet `
        -Value $teardown `
        -Expected $teardownProperties `
        -Label 'capture teardown receipt'
    $pid = ConvertTo-PassiveReceiptInteger $teardown.pid u32 'teardown.pid'
    $creation = ConvertTo-PassiveReceiptInteger `
        $teardown.process_creation_filetime_utc u64 `
        'teardown.process_creation_filetime_utc'
    $pinnedHookSiteCount = ConvertTo-PassiveReceiptInteger `
        $teardown.pinned_hook_site_count u32 `
        'teardown.pinned_hook_site_count'
    if ($teardown.schema -isnot [string] -or
        [string]$teardown.schema -cne 'hoi4-capture-teardown-v1' -or
        $teardown.status -isnot [string] -or
        [string]$teardown.status -cne 'collector-unloaded' -or
        $teardown.capture_policy -isnot [string] -or
        [string]$teardown.capture_policy -cne 'passive-observer-only' -or
        $pid -ne $TargetPid -or
        $creation -ne $TargetCreationFiletimeUtc -or
        $teardown.session_id -isnot [string] -or
        [string]$teardown.session_id -cne $SessionId -or
        $teardown.capture_id -isnot [string] -or
        [string]$teardown.capture_id -cne $CaptureId -or
        $teardown.data_pipe_eof_observed -isnot [bool] -or
        -not [bool]$teardown.data_pipe_eof_observed -or
        $teardown.hook_lease_released_after_eof -isnot [bool] -or
        -not [bool]$teardown.hook_lease_released_after_eof -or
        $teardown.collector_module_absent_verified -isnot [bool] -or
        -not [bool]$teardown.collector_module_absent_verified -or
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
        CollectorModuleAbsentVerified = $true
        TargetProcessEpochRevalidated = $true
        TargetCodeTemporarilyInstrumented = $true
        PinnedHookTargetCodeRestored = $true
        PinnedHookSitesRestored = $true
        PinnedHookSiteCount = [uint32]$expectedHookCount
        PinnedHookSiteBaselineSha256 = [string]$teardown.pinned_hook_site_baseline_sha256
        PinnedHookSiteRestoredSha256 = [string]$teardown.pinned_hook_site_restored_sha256
    }
}
