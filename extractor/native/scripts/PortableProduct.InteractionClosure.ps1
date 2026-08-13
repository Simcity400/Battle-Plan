Set-StrictMode -Version Latest

# Content-addressed derivation cache.
#
# The authorities in this file are whole-tree: changing one production source
# file re-derives every other file's comment-stripped code and capability
# evidence from scratch. That is fine for one Release; it is not fine for the
# package self-test, which mutates one file and re-runs the whole gate several
# hundred times in a row.
#
# Every key below is the SHA256 of the exact text the answer was derived from,
# never a path and never a timestamp. That matters: this file is dot-sourced
# into processes that rewrite production sources between calls, and a
# path-keyed or mtime-keyed cache would hand such a call a stale answer (two
# forbidden symbols of the same length written into the same file inside one
# filesystem timestamp tick is not a hypothetical here - `SuspendThread` and
# `ResumeThread` are both thirteen characters). Keying on content means a hit
# can only return what recomputing would have returned.
#
# Only successful derivations are stored, so an input that throws still throws
# every time it is presented.
#
# The cache is bounded. A caller that rewrites the same file several hundred
# times presents a new content key every time, and the largest production
# source here is over a megabyte, so an unbounded cache would grow without
# limit while the useful entries - the files that did not change - stay a fixed
# size. On overflow the whole cache is dropped rather than evicted piecewise:
# the entries are pure functions of their keys, so dropping them costs one
# re-derivation and can never change an answer.
$script:ProductionInteractionDerivationCache = @{}
$script:ProductionInteractionDerivationCacheWeight = 0
# The budget is a cliff, not a gradient: on overflow the WHOLE cache is dropped,
# so a budget the working set merely brushes against turns every derivation into
# a cold one at unpredictable moments. Measured on this repository the working
# set is about 42 MB of weight, which the previous 64 MB budget was close enough
# to that a few more memoised files would have started dropping everything each
# time. Raised to a size the working set cannot approach; the entries are pure
# functions of their keys, so the only cost of holding them is memory, and this
# process is a validation runner that exits when it is done.
$script:ProductionInteractionDerivationCacheWeightBudget = 1000000000

function Set-ProductionInteractionDerivation {
    param(
        [Parameter(Mandatory = $true)][string]$Key,
        [Parameter(Mandatory = $true)][AllowNull()]$Value,
        [Parameter(Mandatory = $true)][int]$Weight
    )

    if ($script:ProductionInteractionDerivationCacheWeight + $Weight -gt
            $script:ProductionInteractionDerivationCacheWeightBudget) {
        $script:ProductionInteractionDerivationCache = @{}
        $script:ProductionInteractionDerivationCacheWeight = 0
    }
    $script:ProductionInteractionDerivationCache[$Key] = $Value
    $script:ProductionInteractionDerivationCacheWeight += $Weight
}

# Digest of the rules themselves.
#
# Every memo below answers "what does this rule say about these bytes". The
# bytes are in the key; without this the RULES would not be, and editing a
# classification in this file would leave a warm memo answering the old
# question. It is this file's own text, so any edit anywhere in it - a roster,
# a predicate, an entry point - invalidates every memo at once. Deliberately
# coarse: over-invalidating costs one re-derivation, under-invalidating is a
# gate that has stopped asking what it claims to ask.
$script:ProductionInteractionAuthorityWalkId = $null
$script:ProductionInteractionEvaluationDepth = 0

function Enter-ProductionInteractionEvaluation {
    # A source tree authority is a statement about bytes as of ONE walk, and the
    # per-file memo trusts its digests. A walk id alone only proves no NEWER
    # walk has happened, so a caller that kept an authority, edited a file and
    # called back without re-walking would still have been served the old
    # derivation - the memo cannot see an edit it was never asked to look for.
    #
    # So an authority backs at most one evaluation. The outermost gate call
    # retires the walk id when it returns, and any later use of that authority
    # is refused rather than answered. Nesting is counted because the closure
    # assertion calls the findings walk with the same authority.
    $script:ProductionInteractionEvaluationDepth++
}

function Exit-ProductionInteractionEvaluation {
    $script:ProductionInteractionEvaluationDepth--
    if ($script:ProductionInteractionEvaluationDepth -le 0) {
        $script:ProductionInteractionEvaluationDepth = 0
        $script:ProductionInteractionAuthorityWalkId = $null
    }
}

$script:ProductionInteractionRulesDigest = $(
    $rulesPath = if ($PSCommandPath) { $PSCommandPath } else { $MyInvocation.MyCommand.Path }
    if ([string]::IsNullOrEmpty($rulesPath) -or -not (Test-Path -LiteralPath $rulesPath -PathType Leaf)) {
        throw 'production interaction closure cannot digest its own rules'
    }
    $rulesSha = [Security.Cryptography.SHA256]::Create()
    try {
        [BitConverter]::ToString(
            $rulesSha.ComputeHash([IO.File]::ReadAllBytes($rulesPath))
        ).Replace('-', '')
    }
    finally {
        $rulesSha.Dispose()
    }
)

function Get-ProductionInteractionFileDerivation {
    # One file's answer to one rule, keyed on the digest of that file's bytes.
    #
    # The per-file strip and capability-evidence caches key on the TEXT handed
    # to them, which means the caller has already read the file to get it. That
    # is the whole remaining cost of a gate evaluation once everything else is
    # cached: the self-test's source-rejection cases change exactly one of the
    # 362 authority files per iteration, and every rule re-read all 362 to find
    # out which. FileSha256 lets an unchanged file be answered without being
    # opened.
    #
    # The digest MUST be the one the authority walk computed in THIS evaluation
    # - Get-ProductionInteractionSourceTreeAuthority reads every admitted file's
    # bytes and returns file_digests from exactly those bytes, and it runs at
    # the start of every evaluation. A digest from anywhere else is a promise
    # about a file nobody re-read, so an empty one is refused rather than
    # treated as "no digest available".
    param(
        [Parameter(Mandatory = $true)][string]$Kind,
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][AllowEmptyString()][string]$FileSha256,
        [Parameter(Mandatory = $true)][AllowEmptyString()][string]$Discriminator,
        [Parameter(Mandatory = $true)][AllowEmptyString()][string]$AuthorityWalkId,
        [Parameter(Mandatory = $true)][scriptblock]$Compute,
        [int]$Weight = 4096
    )

    if ($FileSha256 -notmatch '^[0-9a-fA-F]{64}$') {
        throw "production interaction file derivation has no authority digest: $Path"
    }
    # A digest is only a statement about the bytes as of the walk that took it.
    # Holding an authority across an edit and passing it back would select the
    # warm derivation for the old bytes, which is the one way this memo could
    # hide a real change - so the derivation refuses any authority but the most
    # recent walk's, and re-verifies the bytes it actually reads on a miss.
    if ($AuthorityWalkId -ne $script:ProductionInteractionAuthorityWalkId) {
        throw (
            'production interaction file derivation was handed a stale source ' +
            "tree authority: $Path"
        )
    }
    $key = 'file-derivation:' + (Get-ProductionInteractionContentKey -Text (
        @(
            $Kind,
            [IO.Path]::GetFullPath($Path),
            $FileSha256,
            $Discriminator,
            $script:ProductionInteractionRulesDigest
        ) -join "`n"
    ))
    if ($script:ProductionInteractionDerivationCache.Contains($key)) {
        return $script:ProductionInteractionDerivationCache[$key]
    }
    # On a miss the compute block opens the file again, after the walk hashed
    # it. Re-digesting what it read closes that window: a tree that changed
    # between the walk and the read is a torn evaluation, and this refuses it
    # rather than filing an answer under a digest that never described it.
    $value = & $Compute
    $verifySha = [Security.Cryptography.SHA256]::Create()
    try {
        $observed = [BitConverter]::ToString(
            $verifySha.ComputeHash([IO.File]::ReadAllBytes($Path))
        ).Replace('-', '')
    }
    finally {
        $verifySha.Dispose()
    }
    if ($observed -ne $FileSha256.ToUpperInvariant() -and
        $observed.ToLowerInvariant() -ne $FileSha256.ToLowerInvariant()) {
        throw (
            'production interaction source changed while the gate was reading ' +
            "it: $Path"
        )
    }
    Set-ProductionInteractionDerivation -Key $key -Value $value -Weight $Weight
    return $value
}

function Get-ProductionInteractionAuthorityFileDigest {
    param(
        [Parameter(Mandatory = $true)]$Authority,
        [Parameter(Mandatory = $true)][string]$Path
    )

    if ($null -eq $Authority -or
        -not ($Authority.PSObject.Properties.Name -contains 'file_digests') -or
        $null -eq $Authority.file_digests -or
        -not $Authority.file_digests.Contains($Path)) {
        throw "production interaction authority carries no digest for: $Path"
    }
    return [string]$Authority.file_digests[$Path]
}

function Get-ProductionInteractionAuthorityWalkId {
    param([Parameter(Mandatory = $true)]$Authority)

    if ($null -eq $Authority -or
        -not ($Authority.PSObject.Properties.Name -contains 'walk_id')) {
        throw 'production interaction authority carries no walk identity'
    }
    return [string]$Authority.walk_id
}

function Get-ProductionInteractionContentKey {
    param([Parameter(Mandatory = $true)][AllowEmptyString()][string]$Text)

    $sha = [Security.Cryptography.SHA256]::Create()
    try {
        return [BitConverter]::ToString(
            $sha.ComputeHash([Text.Encoding]::UTF8.GetBytes($Text))
        ).Replace('-', '')
    }
    finally {
        $sha.Dispose()
    }
}

function Get-ProductionInteractionRelativePath {
    param(
        [Parameter(Mandatory = $true)][string]$Root,
        [Parameter(Mandatory = $true)][string]$Path
    )

    $rootFull = [IO.Path]::GetFullPath($Root).TrimEnd('\') + '\'
    $pathFull = [IO.Path]::GetFullPath($Path)
    if (-not $pathFull.StartsWith(
        $rootFull,
        [StringComparison]::OrdinalIgnoreCase
    )) {
        throw "production interaction source path escapes repository root: $pathFull"
    }
    return $pathFull.Substring($rootFull.Length).Replace('\', '/')
}

function Get-ForbiddenProductionProcessLifecycleSymbols {
    # This is the fixed target-client control roster, not a blanket ban on the
    # recorder operating its own existing GUI. In particular, ShellExecuteW
    # and SendMessageW remain necessary in product_runtime.rs for the
    # recorder's own open-folder action and status control. They remain in the
    # global denylist and are removed only for the recorder role after their
    # exact source callsites are structurally closed below. Replay receives
    # only the exact authenticated self-worker exception structurally closed
    # below; collector receives no exception. Target process creation,
    # suspension, termination, job ownership, message posting, input
    # injection, and hook installation remain denied below.
    return @(
        'CreateProcessA',
        'CreateProcessW',
        'CreateProcessAsUserA',
        'CreateProcessAsUserW',
        'CreateProcessWithLogonW',
        'CreateProcessWithTokenW',
        'WinExec',
        'ShellExecuteA',
        'ShellExecuteW',
        'ShellExecuteExA',
        'ShellExecuteExW',
        'TerminateProcess',
        'TerminateThread',
        'DebugActiveProcess',
        'DebugActiveProcessStop',
        'DebugBreakProcess',
        'GenerateConsoleCtrlEvent',
        'EndTask',
        'OpenThread',
        'SetThreadContext',
        'Wow64SetThreadContext',
        'QueueUserAPC',
        'CreateRemoteThreadEx',
        'VirtualProtectEx',
        'VirtualAllocExNuma',
        'NtCreateThreadEx',
        'RtlCreateUserThread',
        'NtWriteVirtualMemory',
        'NtProtectVirtualMemory',
        'NtAllocateVirtualMemory',
        'NtQueueApcThread',
        'NtQueueApcThreadEx',
        'NtCreateSection',
        'NtMapViewOfSection',
        'ZwCreateThreadEx',
        'ZwWriteVirtualMemory',
        'ZwProtectVirtualMemory',
        'ZwAllocateVirtualMemory',
        'ZwQueueApcThread',
        'ZwCreateSection',
        'ZwMapViewOfSection',
        'NtSuspendProcess',
        'NtResumeProcess',
        'NtTerminateProcess',
        'NtSuspendThread',
        'NtResumeThread',
        'NtTerminateThread',
        # Thread-handle acquisition and the consumers a handle reaches. Both
        # halves were incomplete: OpenThread was listed while NtOpenThread and
        # ZwOpenThread, which do the same thing through ntdll, were not, so the
        # ban on obtaining another thread's handle was never actually in force
        # anywhere in the tree. Three review rounds each found more of this
        # surface - NtSetContextThread, then CancelSynchronousIo and
        # SetThreadInformation, then the Nt/Zw OpenThread pair and the
        # scheduling setters. Each round's findings are named here.
        #
        # Treat this as a census that has grown three times and can grow again,
        # never as a proof that the thread-control surface is closed. Nothing in
        # this lane establishes that, and no comment here should be read as
        # claiming it. What IS established is narrower and is enforced below:
        # four exact files, named by the suspending-code-mutation baseline's own
        # evidence, receive exactly the suspension symbols that evidence records
        # for that file - SuspendThread and ResumeThread today - plus the handle
        # entry point those symbols need, three in total.
        'ZwSuspendThread',
        'ZwResumeThread',
        'ZwTerminateThread',
        'NtSetContextThread',
        'ZwSetContextThread',
        'NtAlertThread',
        'ZwAlertThread',
        'NtAlertResumeThread',
        'ZwAlertResumeThread',
        'NtSetInformationThread',
        'ZwSetInformationThread',
        'NtImpersonateThread',
        'ZwImpersonateThread',
        'QueueUserAPC2',
        'ZwQueueApcThreadEx',
        'NtQueueApcThreadEx2',
        'ZwQueueApcThreadEx2',
        'SetThreadToken',
        'SetThreadInformation',
        'CancelSynchronousIo',
        'NtOpenThread',
        'ZwOpenThread',
        'SetThreadPriority',
        'SetThreadPriorityBoost',
        'SetThreadAffinityMask',
        'SetThreadIdealProcessor',
        'PostMessageA',
        'PostMessageW',
        'PostThreadMessageA',
        'PostThreadMessageW',
        'SuspendThread',
        'Wow64SuspendThread',
        'ResumeThread',
        'CreateJobObjectA',
        'CreateJobObjectW',
        'OpenJobObjectA',
        'OpenJobObjectW',
        'AssignProcessToJobObject',
        'SetInformationJobObject',
        'QueryInformationJobObject',
        'TerminateJobObject',
        'IsProcessInJob'
    )
}

function Get-ForbiddenProductionGameUiSymbols {
    return @(
        'SendInput',
        'keybd_event',
        'mouse_event',
        'SetCursorPos',
        'SetKeyboardState',
        'InitializeTouchInjection',
        'InjectTouchInput',
        'CreateSyntheticPointerDevice',
        'InjectSyntheticPointerInput',
        'PostMessageA',
        'PostMessageW',
        'PostThreadMessageA',
        'PostThreadMessageW',
        'SendMessageA',
        'SendMessageW',
        'SendMessageTimeoutA',
        'SendMessageTimeoutW',
        'SendNotifyMessageA',
        'SendNotifyMessageW',
        'SendMessageCallbackA',
        'SendMessageCallbackW',
        'AttachThreadInput',
        'SetForegroundWindow',
        'BringWindowToTop',
        'SetActiveWindow',
        'SetFocus',
        'ShowWindow',
        'ShowWindowAsync',
        'SetWindowPos',
        'MoveWindow',
        'EnableWindow',
        'SetWindowTextA',
        'SetWindowTextW',
        'SendDlgItemMessageA',
        'SendDlgItemMessageW',
        'CloseWindow',
        'DestroyWindow',
        'ShowOwnedPopups',
        'SetWindowPlacement',
        'AnimateWindow',
        'SwitchToThisWindow',
        'SetParent',
        'BlockInput',
        'ClipCursor',
        'SetWindowsHookExA',
        'SetWindowsHookExW',
        'UnhookWindowsHookEx'
    )
}

function Get-ForbiddenProductionClientCapabilitySymbols {
    return @(
        @(
            @(Get-ForbiddenProductionProcessLifecycleSymbols) +
            @(Get-ForbiddenProductionGameUiSymbols)
        ) | Select-Object -Unique
    )
}

function Get-ForbiddenProductionInteractionMarkers {
    return @(
        'BATTLE_PLAN_CLIENT_DRIVING_EVIDENCE_V1',
        'BATTLE_PLAN_CLIENT_DRIVING_OPT_IN',
        '--early-launch-exe',
        '--early-launch-arg',
        '--early-launch-keep-window',
        'CREATE_SUSPENDED',
        'created_suspended',
        'primary_thread_resumed',
        'early_render_launch_manifest',
        'early_render_capture_receipt',
        '--hour-latch',
        'acquire_hour_latch',
        'release_hour_latch',
        'write_pause_state',
        'saved_pause_state'
    ) + @(Get-ForbiddenProductionClientCapabilitySymbols)
}

function Get-ProductionInteractionMarkerRosterSha256 {
    $roster = @(Get-ForbiddenProductionInteractionMarkers)
    $preimage = $roster -join "`0"
    $sha = [Security.Cryptography.SHA256]::Create()
    try {
        return ([BitConverter]::ToString(
            $sha.ComputeHash([Text.Encoding]::UTF8.GetBytes($preimage))
        )).Replace('-', '').ToLowerInvariant()
    }
    finally {
        $sha.Dispose()
    }
}

function Remove-TomlLineComment {
    param(
        [Parameter(Mandatory = $true)]
        [AllowEmptyString()]
        [string]$Line
    )

    if ($Line.Contains('"""') -or $Line.Contains("'''")) {
        throw 'production Cargo manifest uses unsupported multiline TOML strings'
    }
    $quote = [char]0
    $escaped = $false
    for ($index = 0; $index -lt $Line.Length; $index++) {
        $character = $Line[$index]
        if ($quote -ne [char]0) {
            if ($quote -eq '"' -and $escaped) {
                $escaped = $false
                continue
            }
            if ($quote -eq '"' -and $character -eq '\') {
                $escaped = $true
                continue
            }
            if ($character -eq $quote) {
                $quote = [char]0
            }
            continue
        }
        if ($character -eq '"' -or $character -eq "'") {
            $quote = $character
            continue
        }
        if ($character -eq '#') {
            return $Line.Substring(0, $index)
        }
    }
    if ($quote -ne [char]0) {
        throw 'production Cargo manifest has an unterminated TOML string'
    }
    return $Line
}

function Get-TomlDelimiterBalance {
    param([Parameter(Mandatory = $true)][string]$Text)

    $curly = 0
    $square = 0
    $quote = [char]0
    $escaped = $false
    foreach ($character in $Text.ToCharArray()) {
        if ($quote -ne [char]0) {
            if ($quote -eq '"' -and $escaped) {
                $escaped = $false
                continue
            }
            if ($quote -eq '"' -and $character -eq '\') {
                $escaped = $true
                continue
            }
            if ($character -eq $quote) {
                $quote = [char]0
            }
            continue
        }
        if ($character -eq '"' -or $character -eq "'") {
            $quote = $character
            continue
        }
        switch ($character) {
            '{' { $curly++ }
            '}' { $curly-- }
            '[' { $square++ }
            ']' { $square-- }
        }
        if ($curly -lt 0 -or $square -lt 0) {
            throw 'production Cargo manifest has unbalanced TOML delimiters'
        }
    }
    return [pscustomobject]@{
        Curly = $curly
        Square = $square
        InString = ($quote -ne [char]0)
    }
}

function Split-TomlTopLevel {
    param(
        [Parameter(Mandatory = $true)][string]$Text,
        [Parameter(Mandatory = $true)][char]$Separator
    )

    $parts = [System.Collections.Generic.List[string]]::new()
    $start = 0
    $curly = 0
    $square = 0
    $quote = [char]0
    $escaped = $false
    for ($index = 0; $index -lt $Text.Length; $index++) {
        $character = $Text[$index]
        if ($quote -ne [char]0) {
            if ($quote -eq '"' -and $escaped) {
                $escaped = $false
                continue
            }
            if ($quote -eq '"' -and $character -eq '\') {
                $escaped = $true
                continue
            }
            if ($character -eq $quote) {
                $quote = [char]0
            }
            continue
        }
        if ($character -eq '"' -or $character -eq "'") {
            $quote = $character
            continue
        }
        switch ($character) {
            '{' { $curly++ }
            '}' { $curly-- }
            '[' { $square++ }
            ']' { $square-- }
        }
        if ($character -eq $Separator -and $curly -eq 0 -and $square -eq 0) {
            $parts.Add($Text.Substring($start, $index - $start))
            $start = $index + 1
        }
    }
    $parts.Add($Text.Substring($start))
    return @($parts)
}

function Split-TomlAssignment {
    param([Parameter(Mandatory = $true)][string]$Text)

    $parts = @(Split-TomlTopLevel -Text $Text -Separator '=')
    if ($parts.Count -lt 2) {
        throw "production Cargo manifest statement is not an assignment: $Text"
    }
    return [pscustomobject]@{
        Key = $parts[0].Trim()
        Value = ($parts[1..($parts.Count - 1)] -join '=').Trim()
    }
}

function Split-TomlDottedKey {
    param([Parameter(Mandatory = $true)][string]$Text)

    $parts = [System.Collections.Generic.List[string]]::new()
    $start = 0
    $quote = [char]0
    for ($index = 0; $index -lt $Text.Length; $index++) {
        $character = $Text[$index]
        if ($quote -ne [char]0) {
            if ($character -eq '\') {
                throw 'production Cargo manifest uses an escaped TOML dotted key'
            }
            if ($character -eq $quote) {
                $quote = [char]0
            }
            continue
        }
        if ($character -eq '"' -or $character -eq "'") {
            $quote = $character
            continue
        }
        if ($character -eq '.') {
            $part = $Text.Substring($start, $index - $start).Trim()
            if ([string]::IsNullOrWhiteSpace($part)) {
                throw 'production Cargo manifest has an empty TOML dotted-key segment'
            }
            $parts.Add($part.Trim('"', "'"))
            $start = $index + 1
        }
    }
    if ($quote -ne [char]0) {
        throw 'production Cargo manifest has an unterminated TOML dotted key'
    }
    $last = $Text.Substring($start).Trim()
    if ([string]::IsNullOrWhiteSpace($last)) {
        throw 'production Cargo manifest has an empty TOML dotted-key segment'
    }
    $parts.Add($last.Trim('"', "'"))
    return @($parts)
}

function Get-TomlLogicalAssignments {
    param([Parameter(Mandatory = $true)][string]$Toml)

    $assignments = [System.Collections.Generic.List[object]]::new()
    $section = ''
    $pending = ''
    foreach ($rawLine in ($Toml -split "\r?\n")) {
        $line = (Remove-TomlLineComment -Line $rawLine).Trim()
        if ([string]::IsNullOrWhiteSpace($line)) {
            continue
        }
        if ([string]::IsNullOrWhiteSpace($pending) -and
            $line -match '^\[\[?(.+?)\]\]?$') {
            $section = $Matches[1].Trim()
            if ($section.Contains('\')) {
                throw 'production Cargo manifest uses an escaped TOML table key'
            }
            continue
        }
        $pending = if ([string]::IsNullOrWhiteSpace($pending)) {
            $line
        }
        else {
            "$pending`n$line"
        }
        $balance = Get-TomlDelimiterBalance -Text $pending
        if ($balance.Curly -eq 0 -and $balance.Square -eq 0 -and -not $balance.InString) {
            $assignment = Split-TomlAssignment -Text $pending
            if (([string]$assignment.Key).Contains('\')) {
                throw 'production Cargo manifest uses an escaped TOML assignment key'
            }
            $assignments.Add([pscustomobject]@{
                Section = $section
                Key = $assignment.Key
                Value = $assignment.Value
            })
            $pending = ''
        }
    }
    if (-not [string]::IsNullOrWhiteSpace($pending)) {
        throw 'production Cargo manifest ends with an incomplete TOML assignment'
    }
    return @($assignments)
}

function Get-TomlInlineTableFields {
    param([Parameter(Mandatory = $true)][string]$Value)

    $trimmed = $Value.Trim()
    if (-not ($trimmed.StartsWith('{') -and $trimmed.EndsWith('}'))) {
        return @()
    }
    $body = $trimmed.Substring(1, $trimmed.Length - 2)
    return @(
        Split-TomlTopLevel -Text $body -Separator ',' |
            Where-Object { -not [string]::IsNullOrWhiteSpace($_) } |
            ForEach-Object { Split-TomlAssignment -Text $_ }
    )
}

function Assert-CollectorProductionDependencyFeatureClosure {
    param(
        [Parameter(Mandatory = $true)][string]$CollectorCargo,
        [Parameter(Mandatory = $true)][string]$WorkspaceCargo
    )

    $targetDependencies = @(
        'character_presentation_live_lane',
        'focus_research_decisions_depth_live'
    )
    $forbiddenFeatureFragments = @(
        'a729-fixture-hook',
        'a729-focus-fixture-hook'
    )
    $dependencyRecords = [ordered]@{}
    foreach ($assignment in @(Get-TomlLogicalAssignments -Toml $CollectorCargo)) {
        $section = [string]$assignment.Section
        # The @() has to be outside the if. PowerShell unwraps a single-element
        # result out of an if-expression, so a plain [dependencies] section
        # arrived here as a bare string and the .Count below threw under
        # Set-StrictMode instead of reading 1 - which is every ordinary Cargo
        # manifest, so this closure could not be run at all.
        $sectionParts = @(
            if ([string]::IsNullOrWhiteSpace($section)) {
                @()
            }
            else {
                Split-TomlDottedKey -Text $section
            }
        )
        $keyParts = @(Split-TomlDottedKey -Text ([string]$assignment.Key))
        $dependencyName = $null
        $dependencyField = $null
        $dependencyScope = $null
        $inlineFields = @()
        if ($sectionParts.Count -gt 0 -and
            [string]$sectionParts[$sectionParts.Count - 1] -ceq 'dependencies') {
            $dependencyScope = $sectionParts -join '.'
            if ($keyParts.Count -eq 1) {
                $dependencyName = [string]$keyParts[0]
                $inlineFields = @(Get-TomlInlineTableFields -Value ([string]$assignment.Value))
            }
            elseif ($keyParts.Count -eq 2) {
                $dependencyName = [string]$keyParts[0]
                $dependencyField = [string]$keyParts[1]
            }
            else {
                throw 'production Cargo manifest has an unsupported dependency dotted-key shape'
            }
        }
        elseif ($sectionParts.Count -ge 2 -and
            [string]$sectionParts[$sectionParts.Count - 2] -ceq 'dependencies') {
            if ($keyParts.Count -ne 1) {
                throw 'production Cargo dependency subtable has a non-scalar field key'
            }
            $dependencyName = [string]$sectionParts[$sectionParts.Count - 1]
            $dependencyField = [string]$keyParts[0]
            $dependencyScope =
                @($sectionParts[0..($sectionParts.Count - 2)]) -join '.'
        }
        else {
            continue
        }
        $recordKey = "$dependencyScope`0$dependencyName"
        if (-not $dependencyRecords.Contains($recordKey)) {
            $dependencyRecords[$recordKey] = [ordered]@{
                name = $dependencyName
                package = $null
                features = $null
                workspace = $false
            }
        }
        $record = $dependencyRecords[$recordKey]
        foreach ($field in $inlineFields) {
            $inlineFieldParts = @(Split-TomlDottedKey -Text ([string]$field.Key))
            if ($inlineFieldParts.Count -ne 1) {
                throw 'production Cargo dependency inline table has a dotted field key'
            }
            $inlineFieldName = [string]$inlineFieldParts[0]
            if ($inlineFieldName -ceq 'features') {
                $record.features = [string]$field.Value
            }
            elseif ($inlineFieldName -ceq 'package') {
                $record.package = ([string]$field.Value).Trim().Trim('"', "'")
            }
            elseif ($inlineFieldName -ceq 'workspace') {
                $record.workspace = (([string]$field.Value).Trim() -ceq 'true')
            }
        }
        if ($dependencyField -ceq 'features') {
            $record.features = [string]$assignment.Value
        }
        elseif ($dependencyField -ceq 'package') {
            $record.package = ([string]$assignment.Value).Trim().Trim('"', "'")
        }
        elseif ($dependencyField -ceq 'workspace') {
            $record.workspace = (([string]$assignment.Value).Trim() -ceq 'true')
        }
    }
    $workspaceDependencyRecords = [ordered]@{}
    foreach ($assignment in @(Get-TomlLogicalAssignments -Toml $WorkspaceCargo)) {
        # Same unwrapping hazard as above: the @() belongs outside the if.
        $sectionParts = @(
            if ([string]::IsNullOrWhiteSpace([string]$assignment.Section)) {
                @()
            }
            else {
                Split-TomlDottedKey -Text ([string]$assignment.Section)
            }
        )
        $keyParts = @(Split-TomlDottedKey -Text ([string]$assignment.Key))
        $dependencyName = $null
        $dependencyField = $null
        $inlineFields = @()
        if (($sectionParts -join '.') -ceq 'workspace.dependencies') {
            if ($keyParts.Count -eq 1) {
                $dependencyName = [string]$keyParts[0]
                $inlineFields = @(Get-TomlInlineTableFields -Value ([string]$assignment.Value))
            }
            elseif ($keyParts.Count -eq 2) {
                $dependencyName = [string]$keyParts[0]
                $dependencyField = [string]$keyParts[1]
            }
            else {
                throw 'production workspace Cargo manifest has an unsupported dependency dotted-key shape'
            }
        }
        elseif ($sectionParts.Count -eq 3 -and
            [string]$sectionParts[0] -ceq 'workspace' -and
            [string]$sectionParts[1] -ceq 'dependencies') {
            if ($keyParts.Count -ne 1) {
                throw 'production workspace dependency subtable has a non-scalar field key'
            }
            $dependencyName = [string]$sectionParts[2]
            $dependencyField = [string]$keyParts[0]
        }
        else {
            continue
        }
        if (-not $workspaceDependencyRecords.Contains($dependencyName)) {
            $workspaceDependencyRecords[$dependencyName] = [ordered]@{
                name = $dependencyName
                package = $null
                features = $null
            }
        }
        $workspaceRecord = $workspaceDependencyRecords[$dependencyName]
        foreach ($field in $inlineFields) {
            $fieldParts = @(Split-TomlDottedKey -Text ([string]$field.Key))
            if ($fieldParts.Count -ne 1) {
                throw 'production workspace dependency inline table has a dotted field key'
            }
            if ([string]$fieldParts[0] -ceq 'package') {
                $workspaceRecord.package = ([string]$field.Value).Trim().Trim('"', "'")
            }
            elseif ([string]$fieldParts[0] -ceq 'features') {
                $workspaceRecord.features = [string]$field.Value
            }
        }
        if ($dependencyField -ceq 'package') {
            $workspaceRecord.package = ([string]$assignment.Value).Trim().Trim('"', "'")
        }
        elseif ($dependencyField -ceq 'features') {
            $workspaceRecord.features = [string]$assignment.Value
        }
    }
    foreach ($record in $dependencyRecords.Values) {
        $dependencyName = [string]$record.name
        $packageValue = [string]$record.package
        if ([bool]$record.workspace) {
            if (-not $workspaceDependencyRecords.Contains($dependencyName)) {
                throw "collector production dependency '$dependencyName' has unresolved workspace inheritance"
            }
            $workspaceRecord = $workspaceDependencyRecords[$dependencyName]
            if ([string]::IsNullOrWhiteSpace($packageValue)) {
                $packageValue = [string]$workspaceRecord.package
            }
            if ($null -ne $workspaceRecord.features) {
                $record.features = if ($null -eq $record.features) {
                    [string]$workspaceRecord.features
                }
                else {
                    [string]$record.features + "`n" +
                        [string]$workspaceRecord.features
                }
            }
        }
        if ($packageValue.Contains('\')) {
            throw "collector production dependency '$dependencyName' uses escaped package text"
        }
        $featureValue = $record.features
        if ($null -eq $featureValue) {
            continue
        }
        $featureValue = [string]$featureValue
        if ($featureValue.Contains('\')) {
            throw "collector production dependency '$dependencyName' uses escaped feature text"
        }
        if ($targetDependencies -ccontains $dependencyName -or
            $targetDependencies -ccontains $packageValue) {
            throw "collector production dependency '$dependencyName' declares features"
        }
        foreach ($fragment in $forbiddenFeatureFragments) {
            if ($featureValue.IndexOf($fragment, [StringComparison]::Ordinal) -ge 0) {
                throw "collector production dependency '$dependencyName' enables forbidden feature '$fragment'"
            }
        }
    }
}

function Assert-ProtectedLaneDefaultFeatureClosure {
    param(
        [Parameter(Mandatory = $true)][string]$Cargo,
        [Parameter(Mandatory = $true)][string]$ForbiddenFeature,
        [Parameter(Mandatory = $true)][string]$LaneName
    )

    $features = [ordered]@{}
    foreach ($assignment in @(Get-TomlLogicalAssignments -Toml $Cargo)) {
        if ([string]$assignment.Section -cne 'features') {
            continue
        }
        $keyParts = @(Split-TomlDottedKey -Text ([string]$assignment.Key))
        if ($keyParts.Count -ne 1) {
            throw "$LaneName Cargo features table uses a dotted feature key"
        }
        $features[[string]$keyParts[0]] = ([string]$assignment.Value).Trim()
    }
    if (-not $features.Contains('default') -or
        ([string]$features['default'] -replace '\s', '') -cne '[]' -or
        -not $features.Contains($ForbiddenFeature)) {
        throw "$LaneName does not retain an exact empty default and explicit forbidden hook feature"
    }
}

function Get-RustCodeWithoutCommentsOrStrings {
    param([Parameter(Mandatory = $true)][string]$Text)

    $key = 'rust-code:' + (Get-ProductionInteractionContentKey -Text $Text)
    if ($script:ProductionInteractionDerivationCache.Contains($key)) {
        return [string]$script:ProductionInteractionDerivationCache[$key]
    }
    $stripped = Get-RustCodeWithoutCommentsOrStringsCore -Text $Text
    Set-ProductionInteractionDerivation -Key $key -Value $stripped -Weight $stripped.Length
    return $stripped
}

function Get-RustCodeWithoutCommentsOrStringsCore {
    param([Parameter(Mandatory = $true)][string]$Text)

    $output = [Text.StringBuilder]::new($Text.Length)
    $index = 0
    $blockDepth = 0
    while ($index -lt $Text.Length) {
        if ($blockDepth -gt 0) {
            if ($index + 1 -lt $Text.Length -and
                $Text[$index] -eq '/' -and $Text[$index + 1] -eq '*') {
                $blockDepth++
                [void]$output.Append('  ')
                $index += 2
                continue
            }
            if ($index + 1 -lt $Text.Length -and
                $Text[$index] -eq '*' -and $Text[$index + 1] -eq '/') {
                $blockDepth--
                [void]$output.Append('  ')
                $index += 2
                continue
            }
            [void]$output.Append($(if ($Text[$index] -eq "`n") { "`n" } else { ' ' }))
            $index++
            continue
        }
        if ($index + 1 -lt $Text.Length -and
            $Text[$index] -eq '/' -and $Text[$index + 1] -eq '/') {
            while ($index -lt $Text.Length -and $Text[$index] -ne "`n") {
                [void]$output.Append(' ')
                $index++
            }
            continue
        }
        if ($index + 1 -lt $Text.Length -and
            $Text[$index] -eq '/' -and $Text[$index + 1] -eq '*') {
            $blockDepth = 1
            [void]$output.Append('  ')
            $index += 2
            continue
        }
        # A character literal can hold a double quote ('"'), and taking that for
        # the start of a string desynchronises every literal and comment after
        # it - which blanks real code and leaks comment text into the output. A
        # leading quote that is not a character literal is a lifetime, and
        # copying just the quote through is correct for that.
        if ($Text[$index] -eq "'") {
            $closing = -1
            if ($index + 1 -lt $Text.Length -and $Text[$index + 1] -eq '\') {
                for ($probe = $index + 2; $probe -lt [Math]::Min($index + 12, $Text.Length); $probe++) {
                    if ($Text[$probe] -eq "'") {
                        $closing = $probe
                        break
                    }
                }
            }
            elseif ($index + 2 -lt $Text.Length -and $Text[$index + 2] -eq "'") {
                $closing = $index + 2
            }
            if ($closing -lt 0) {
                [void]$output.Append("'")
                $index++
                continue
            }
            while ($index -le $closing) {
                [void]$output.Append($(if ($Text[$index] -eq "`n") { "`n" } else { ' ' }))
                $index++
            }
            continue
        }
        if ($Text[$index] -eq '"') {
            [void]$output.Append(' ')
            $index++
            $escaped = $false
            while ($index -lt $Text.Length) {
                $character = $Text[$index]
                [void]$output.Append($(if ($character -eq "`n") { "`n" } else { ' ' }))
                $index++
                if ($escaped) {
                    $escaped = $false
                }
                elseif ($character -eq '\') {
                    $escaped = $true
                }
                elseif ($character -eq '"') {
                    break
                }
            }
            continue
        }
        [void]$output.Append($Text[$index])
        $index++
    }
    if ($blockDepth -ne 0) {
        throw 'production Rust source has an unterminated block comment'
    }
    return $output.ToString()
}

function Get-TomlProductionDependencyRecords {
    # Cached on the exact TOML text, the same discipline as the per-file strip:
    # the answer is a pure function of the text handed in, and the key is that
    # text's digest, so a hit can only return what re-parsing would have.
    #
    # This is the shared floor under the manifest walks. Every production unit,
    # every lane root resolution and every dependency-closure step re-parses the
    # same handful of manifests, and the parse - not the read - is what costs:
    # one bare read pass over all 365 authority files is 0.33 s, while the walks
    # that consume these records were 4.6 s and 1.3 s per evaluation.
    param([Parameter(Mandatory = $true)][string]$Toml)

    $recordsKey = 'toml-dependency-records:' +
        (Get-ProductionInteractionContentKey -Text $Toml)
    if ($script:ProductionInteractionDerivationCache.Contains($recordsKey)) {
        return @($script:ProductionInteractionDerivationCache[$recordsKey])
    }
    $records = [ordered]@{}
    foreach ($assignment in @(Get-TomlLogicalAssignments -Toml $Toml)) {
        $sectionParts = @(
            if (-not [string]::IsNullOrWhiteSpace([string]$assignment.Section)) {
                Split-TomlDottedKey -Text ([string]$assignment.Section)
            }
        )
        $keyParts = @(Split-TomlDottedKey -Text ([string]$assignment.Key))
        $logicalParts = @($sectionParts) + @($keyParts)
        $dependenciesIndex = -1
        for ($index = 0; $index -lt $logicalParts.Count; $index++) {
            if ([string]$logicalParts[$index] -ceq 'dependencies') {
                $dependenciesIndex = $index
                break
            }
        }
        if ($dependenciesIndex -lt 0 -or
            $dependenciesIndex -ge ($logicalParts.Count - 1)) {
            continue
        }
        $dependencyParts = @(
            $logicalParts[($dependenciesIndex + 1)..($logicalParts.Count - 1)]
        )
        $dependencyName = [string]$dependencyParts[0]
        $dependencyField = $null
        $inlineFields = @()
        if ($dependencyParts.Count -eq 1) {
            $inlineFields = @(Get-TomlInlineTableFields -Value ([string]$assignment.Value))
        }
        elseif ($dependencyParts.Count -eq 2) {
            $dependencyField = [string]$dependencyParts[1]
        }
        else {
            throw 'production Cargo manifest has an unsupported dependency dotted-key shape'
        }
        $dependencyScope =
            @($logicalParts[0..$dependenciesIndex]) -join '.'
        $recordKey = "$dependencyScope`0$dependencyName"
        if (-not $records.Contains($recordKey)) {
            $records[$recordKey] = [ordered]@{
                name = $dependencyName
                scope = $dependencyScope
                path = $null
                workspace = $false
                # Cargo lets a manifest bind an arbitrary local crate name to a
                # different published package. Carrying the real package name
                # here is what stops a `package = ` rename from hiding which
                # crate a production unit actually pulls in.
                package = $null
            }
        }
        $record = $records[$recordKey]
        foreach ($field in $inlineFields) {
            $fieldParts = @(Split-TomlDottedKey -Text ([string]$field.Key))
            if ($fieldParts.Count -ne 1) {
                throw 'production Cargo dependency inline table has a dotted field key'
            }
            $fieldName = [string]$fieldParts[0]
            if ($fieldName -ceq 'path') {
                $record.path = ([string]$field.Value).Trim().Trim('"', "'")
            }
            elseif ($fieldName -ceq 'workspace') {
                $record.workspace = (([string]$field.Value).Trim() -ceq 'true')
            }
            elseif ($fieldName -ceq 'package') {
                $record.package = ([string]$field.Value).Trim().Trim('"', "'")
            }
        }
        if ($dependencyField -ceq 'path') {
            $record.path = ([string]$assignment.Value).Trim().Trim('"', "'")
        }
        elseif ($dependencyField -ceq 'workspace') {
            $record.workspace = (([string]$assignment.Value).Trim() -ceq 'true')
        }
        elseif ($dependencyField -ceq 'package') {
            $record.package = ([string]$assignment.Value).Trim().Trim('"', "'")
        }
    }
    # Every consumer reads these records and none writes back into them, so one
    # instance shared across hits is indistinguishable from re-parsing.
    $parsed = @($records.Values)
    Set-ProductionInteractionDerivation `
        -Key $recordsKey `
        -Value $parsed `
        -Weight (256 + 256 * $parsed.Count)
    return @($parsed)
}

function Assert-ProductionRepoPathIsPhysical {
    param(
        [Parameter(Mandatory = $true)][string]$RepoRoot,
        [Parameter(Mandatory = $true)][string]$Path
    )

    $repo = [IO.Path]::GetFullPath($RepoRoot).TrimEnd(
        [IO.Path]::DirectorySeparatorChar,
        [IO.Path]::AltDirectorySeparatorChar
    )
    $candidate = [IO.Path]::GetFullPath($Path).TrimEnd(
        [IO.Path]::DirectorySeparatorChar,
        [IO.Path]::AltDirectorySeparatorChar
    )
    if ($candidate -ine $repo -and
        -not $candidate.StartsWith(
            $repo + [IO.Path]::DirectorySeparatorChar,
            [StringComparison]::OrdinalIgnoreCase
        )) {
        throw "production source authority path escapes the repository: $candidate"
    }
    $relative = $candidate.Substring($repo.Length).TrimStart(
        [IO.Path]::DirectorySeparatorChar,
        [IO.Path]::AltDirectorySeparatorChar
    )
    $cursor = $repo
    $segments = @(
        $relative.Split(
            @([IO.Path]::DirectorySeparatorChar, [IO.Path]::AltDirectorySeparatorChar),
            [StringSplitOptions]::RemoveEmptyEntries
        )
    )
    foreach ($segment in @('') + $segments) {
        if (-not [string]::IsNullOrEmpty($segment)) {
            $cursor = Join-Path $cursor $segment
        }
        if (-not (Test-Path -LiteralPath $cursor)) {
            throw "production source authority path is missing: $cursor"
        }
        $item = Get-Item -LiteralPath $cursor -Force
        if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
            throw "production source authority path traverses a reparse point: $cursor"
        }
    }
}

function Assert-ProductionLanePathIsPhysical {
    param(
        [Parameter(Mandatory = $true)][string]$LanesRoot,
        [Parameter(Mandatory = $true)][string]$Path
    )

    $lanes = [IO.Path]::GetFullPath($LanesRoot).TrimEnd(
        [IO.Path]::DirectorySeparatorChar,
        [IO.Path]::AltDirectorySeparatorChar
    )
    $candidate = [IO.Path]::GetFullPath($Path).TrimEnd(
        [IO.Path]::DirectorySeparatorChar,
        [IO.Path]::AltDirectorySeparatorChar
    )
    if (-not $candidate.StartsWith(
            $lanes + [IO.Path]::DirectorySeparatorChar,
            [StringComparison]::OrdinalIgnoreCase
        )) {
        throw "production collector lane dependency escapes the lane root: $candidate"
    }
    $lanesItem = Get-Item -LiteralPath $lanes -Force
    if (($lanesItem.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw "production collector lane root is a reparse point: $lanes"
    }
    $relative = $candidate.Substring($lanes.Length).TrimStart(
        [IO.Path]::DirectorySeparatorChar,
        [IO.Path]::AltDirectorySeparatorChar
    )
    $cursor = $lanes
    foreach ($segment in @($relative.Split(
                @([IO.Path]::DirectorySeparatorChar, [IO.Path]::AltDirectorySeparatorChar),
                [StringSplitOptions]::RemoveEmptyEntries
            ))) {
        $cursor = Join-Path $cursor $segment
        if (-not (Test-Path -LiteralPath $cursor)) {
            throw "production collector lane source path is missing: $cursor"
        }
        $item = Get-Item -LiteralPath $cursor -Force
        if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
            throw "production collector lane source path traverses a reparse point: $cursor"
        }
    }
}

function Get-CollectorProductionLaneRoots {
    param([Parameter(Mandatory = $true)][string]$RepoRoot)

    $repo = [IO.Path]::GetFullPath($RepoRoot)
    $collectorRoot = Join-Path $repo 'extractor\native\collector'
    $collectorCargoPath = Join-Path $collectorRoot 'Cargo.toml'
    $workspaceRoot = Join-Path $repo 'extractor\native'
    $workspaceCargoPath = Join-Path $workspaceRoot 'Cargo.toml'
    $lanesRoot = [IO.Path]::GetFullPath((Join-Path $workspaceRoot 'lanes'))
    if (-not (Test-Path -LiteralPath $collectorCargoPath -PathType Leaf)) {
        throw 'collector Cargo manifest is missing while resolving production lane roots'
    }
    if (-not (Test-Path -LiteralPath $workspaceCargoPath -PathType Leaf) -or
        -not (Test-Path -LiteralPath $lanesRoot -PathType Container)) {
        throw 'extractor workspace or production lane root is missing'
    }
    foreach ($physicalPath in @(
        $collectorRoot,
        $collectorCargoPath,
        $workspaceRoot,
        $workspaceCargoPath,
        $lanesRoot
    )) {
        Assert-ProductionRepoPathIsPhysical -RepoRoot $repo -Path $physicalPath
    }
    $workspaceRecords = @(
        Get-TomlProductionDependencyRecords -Toml (
            Get-Content -LiteralPath $workspaceCargoPath -Raw
        ) | Where-Object { [string]$_.scope -ceq 'workspace.dependencies' }
    )
    $laneRoots = [System.Collections.Generic.List[string]]::new()
    $pendingManifests = [System.Collections.Generic.Queue[string]]::new()
    $pendingManifests.Enqueue($collectorCargoPath)
    $visitedManifests = [System.Collections.Generic.HashSet[string]]::new(
        [StringComparer]::OrdinalIgnoreCase
    )
    while ($pendingManifests.Count -ne 0) {
        $manifestPath = [IO.Path]::GetFullPath($pendingManifests.Dequeue())
        if (-not $visitedManifests.Add($manifestPath)) {
            continue
        }
        if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) {
            throw "production lane Cargo manifest is missing: $manifestPath"
        }
        $manifestItem = Get-Item -LiteralPath $manifestPath -Force
        if (($manifestItem.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
            throw "production lane Cargo manifest is a reparse point: $manifestPath"
        }
        $manifestRoot = Split-Path -Parent $manifestPath
        foreach ($record in @(Get-TomlProductionDependencyRecords -Toml (
                    Get-Content -LiteralPath $manifestPath -Raw
                ))) {
            if ([string]$record.scope -ceq 'workspace.dependencies') {
                continue
            }
            $dependencyPath = [string]$record.path
            $dependencyBase = $manifestRoot
            if ([string]::IsNullOrWhiteSpace($dependencyPath) -and $record.workspace) {
                # Not $matches: that is PowerShell's automatic match-group
                # variable, and any -match/-notmatch evaluated between this
                # assignment and the reads below would silently replace it.
                # Nothing does today, which is exactly why it would not be
                # noticed the day something did.
                $inheritedRecords = @($workspaceRecords | Where-Object {
                    [string]$_.name -ceq [string]$record.name
                })
                if ($inheritedRecords.Count -ne 1 -or
                    [string]::IsNullOrWhiteSpace([string]$inheritedRecords[0].path)) {
                    throw "workspace production dependency lacks one exact path: $($record.name)"
                }
                $dependencyPath = [string]$inheritedRecords[0].path
                $dependencyBase = $workspaceRoot
            }
            if ([string]::IsNullOrWhiteSpace($dependencyPath)) {
                continue
            }
            $resolved = [IO.Path]::GetFullPath((Join-Path $dependencyBase $dependencyPath))
            if (-not $resolved.StartsWith(
                    $lanesRoot + [IO.Path]::DirectorySeparatorChar,
                    [StringComparison]::OrdinalIgnoreCase
                )) {
                continue
            }
            Assert-ProductionLanePathIsPhysical -LanesRoot $lanesRoot -Path $resolved
            if (-not (Test-Path -LiteralPath $resolved -PathType Container)) {
                throw "production collector lane source root is missing: $resolved"
            }
            if (-not $laneRoots.Contains($resolved)) {
                $laneRoots.Add($resolved)
            }
            $pendingManifests.Enqueue((Join-Path $resolved 'Cargo.toml'))
        }
    }
    if ($laneRoots.Count -eq 0) {
        throw 'collector Cargo manifest resolves no production lane roots'
    }
    return @($laneRoots | Sort-Object)
}

function Get-CargoLockPackageDependencyGraph {
    # Cargo's own resolution of the workspace, read as a graph. This is the same
    # TOML lexer the manifests go through, so there is no second parser here -
    # only a second SOURCE, which is the whole point of reading it.
    param([Parameter(Mandatory = $true)][string]$LockPath)

    if (-not (Test-Path -LiteralPath $LockPath -PathType Leaf)) {
        throw "production Cargo.lock is missing: $LockPath"
    }
    # Lexing the whole lock, once per caller. Keyed on the lock's exact text,
    # which is the entire input to the graph.
    $lockText = Get-Content -LiteralPath $LockPath -Raw
    $graphKey = 'cargo-lock-graph:' +
        (Get-ProductionInteractionContentKey -Text $lockText)
    if ($script:ProductionInteractionDerivationCache.Contains($graphKey)) {
        return $script:ProductionInteractionDerivationCache[$graphKey]
    }
    $graph = [System.Collections.Specialized.OrderedDictionary]::new([StringComparer]::Ordinal)
    $current = $null
    foreach ($assignment in @(Get-TomlLogicalAssignments -Toml $lockText)) {
        if (([string]$assignment.Section) -cne 'package') {
            continue
        }
        $key = [string]$assignment.Key
        $value = [string]$assignment.Value
        if ($key -ceq 'name') {
            # Every [[package]] block opens with its name, so a name is what
            # starts a block here. A lock file that stopped doing that would
            # attribute dependencies to the previous package, which is why the
            # coverage assertion below also requires the package it starts from
            # to exist.
            $current = $value.Trim().Trim('"', "'")
            if (-not $graph.Contains($current)) {
                $graph[$current] = [System.Collections.Generic.List[string]]::new()
            }
            continue
        }
        if ($key -cne 'dependencies' -or $null -eq $current) {
            continue
        }
        foreach ($match in [regex]::Matches($value, '"(?<entry>[^"]+)"')) {
            # A lock entry is "name", or "name version", or
            # "name version (source)" when one name resolves to several
            # versions. The name is the first field either way.
            $graph[$current].Add(
                (([string]$match.Groups['entry'].Value).Split(' ')[0])
            )
        }
    }
    Set-ProductionInteractionDerivation `
        -Key $graphKey `
        -Value $graph `
        -Weight (256 + 256 * $graph.Count)
    return $graph
}

function Get-CollectorLockResolvedLaneRoots {
    # What cargo itself says the collector links, restricted to the lane roots.
    #
    # This exists to be a SECOND derivation of the production lane set, computed
    # from an artifact this repository's PowerShell does not write: cargo
    # resolves Cargo.lock from the manifests, and this walk starts at the
    # collector package and follows the lock's own dependency lists. The first
    # derivation - Get-CollectorProductionLaneRoots - walks the manifests with
    # our TOML reader. A dependency shape our reader stops understanding makes
    # the two disagree instead of quietly shrinking the scanned tree, which is
    # the failure this whole gate was one refactor away from.
    #
    # One known asymmetry, stated rather than papered over: Cargo.lock merges a
    # package's normal, build and dev dependencies into one list, while the
    # manifest walk deliberately reads only the normal and target-scoped ones.
    # A lane that became dev-only would therefore show up here as a difference.
    # That is the intended behaviour - a lane leaving what production links is
    # exactly the change this must not absorb silently.
    param([Parameter(Mandatory = $true)][string]$RepoRoot)

    $repo = [IO.Path]::GetFullPath($RepoRoot)
    $workspaceRoot = Join-Path $repo 'extractor\native'
    $lockPath = Join-Path $workspaceRoot 'Cargo.lock'
    $lanesRoot = [IO.Path]::GetFullPath((Join-Path $workspaceRoot 'lanes'))
    if (-not (Test-Path -LiteralPath $lanesRoot -PathType Container)) {
        throw 'extractor production lane root is missing'
    }
    $graph = Get-CargoLockPackageDependencyGraph -LockPath $lockPath
    if (-not $graph.Contains('collector')) {
        throw "production Cargo.lock does not resolve the collector package: $lockPath"
    }
    $reached = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    $pending = [System.Collections.Generic.Queue[string]]::new()
    $pending.Enqueue('collector')
    while ($pending.Count -ne 0) {
        $package = $pending.Dequeue()
        if (-not $reached.Add($package)) {
            continue
        }
        if (-not $graph.Contains($package)) {
            continue
        }
        foreach ($dependency in @($graph[$package])) {
            $pending.Enqueue([string]$dependency)
        }
    }
    $roots = [System.Collections.Generic.List[string]]::new()
    foreach ($laneDirectory in @(
        Get-ChildItem -LiteralPath $lanesRoot -Directory -Force | Sort-Object Name
    )) {
        $manifestPath = Join-Path $laneDirectory.FullName 'Cargo.toml'
        if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) {
            continue
        }
        $packageName = $null
        foreach ($assignment in @(Get-TomlLogicalAssignments -Toml (
                    Get-Content -LiteralPath $manifestPath -Raw
                ))) {
            if (([string]$assignment.Section) -ceq 'package' -and
                ([string]$assignment.Key) -ceq 'name') {
                $packageName = ([string]$assignment.Value).Trim().Trim('"', "'")
                break
            }
        }
        if ([string]::IsNullOrWhiteSpace($packageName)) {
            throw "production lane manifest declares no package name: $manifestPath"
        }
        if ($reached.Contains($packageName)) {
            $roots.Add([IO.Path]::GetFullPath($laneDirectory.FullName).TrimEnd('\'))
        }
    }
    return @($roots | Sort-Object)
}

function Get-CollectorProductionLaneRootCoverageDifferences {
    # The manifest walk and cargo's own resolution have to name the same lanes.
    param([Parameter(Mandatory = $true)][string]$RepoRoot)

    $repo = [IO.Path]::GetFullPath($RepoRoot)
    $derived = @(
        Get-CollectorProductionLaneRoots -RepoRoot $repo |
            ForEach-Object { [IO.Path]::GetFullPath($_).TrimEnd('\') }
    )
    $locked = @(Get-CollectorLockResolvedLaneRoots -RepoRoot $repo)
    $differences = [System.Collections.Generic.List[string]]::new()
    foreach ($root in $locked) {
        if ($derived -cnotcontains $root) {
            $differences.Add(
                'production lane is linked by the resolved collector but is not ' +
                "in the scanned lane roots: $root"
            )
        }
    }
    foreach ($root in $derived) {
        if ($locked -cnotcontains $root) {
            $differences.Add(
                'production lane is in the scanned lane roots but the resolved ' +
                "collector does not link it: $root"
            )
        }
    }
    return @($differences)
}

function Assert-CollectorProductionLaneRootCoverage {
    param([Parameter(Mandatory = $true)][string]$RepoRoot)

    $differences = @(
        Get-CollectorProductionLaneRootCoverageDifferences -RepoRoot $RepoRoot
    )
    if ($differences.Count -ne 0) {
        throw (
            @('production lane root coverage disagrees with the resolved collector:') +
            @($differences | ForEach-Object { "  $_" })
        ) -join [Environment]::NewLine
    }
}

function Get-ProductionInteractionSourceExtensions {
    # The one answer to "which files in a production root are source". A second
    # copy of this list somewhere else is a place where the authority can start
    # reading files that a caller believes it does not.
    return @('.rs', '.c', '.h', '.cc', '.cpp')
}

function Get-ProductionInteractionSourceTreeAuthority {
    param([Parameter(Mandatory = $true)][string]$RepoRoot)

    $repo = [IO.Path]::GetFullPath($RepoRoot)
    # Every rule downstream of here reads only the files this list admits, so a
    # lane that leaves the manifest walk does not fail anything - it makes
    # everything pass over a smaller tree. The cross-check is therefore inside
    # the authority rather than beside one of its callers: the package builder
    # and the receipt verifier read this list too, and neither of them would
    # otherwise notice that production had quietly stopped being scanned.
    Assert-CollectorProductionLaneRootCoverage -RepoRoot $repo
    $roots = @(
        (Join-Path $repo 'extractor\native\recorder_host'),
        (Join-Path $repo 'extractor\native\collector'),
        (Join-Path $repo 'replay_interface\native\replay_app')
    ) + @(Get-CollectorProductionLaneRoots -RepoRoot $repo)
    $sourceExtensions = @(Get-ProductionInteractionSourceExtensions)
    $files = @(
        foreach ($root in $roots) {
            if (-not (Test-Path -LiteralPath $root -PathType Container)) {
                throw "production interaction source root is missing: $root"
            }
            Assert-ProductionRepoPathIsPhysical -RepoRoot $repo -Path $root
            # Enumerated through the framework rather than Get-ChildItem, which
            # materialises a FileInfo for every entry in the tree and dominated
            # this walk. Same entries - files AND directories, hidden ones
            # included, so the reparse-point rule still sees everything - same
            # admission test, same order. This runs once per gate evaluation and
            # the self-test evaluates the gate several hundred times.
            foreach ($entry in ([IO.DirectoryInfo]$root).EnumerateFileSystemInfos(
                '*', [IO.SearchOption]::AllDirectories)) {
                # FileSystemInfo carries the attributes the enumeration already
                # returned, so the reparse-point rule costs no extra syscall.
                if (($entry.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
                    throw "production interaction source tree contains a reparse point: $($entry.FullName)"
                }
                if (($entry.Attributes -band [IO.FileAttributes]::Directory) -ne 0) {
                    continue
                }
                if ($sourceExtensions -contains $entry.Extension) {
                    $entry.FullName
                }
            }
        }
    ) | Sort-Object {
        Get-ProductionInteractionRelativePath -Root $repo -Path $_
    }
    if ($files.Count -eq 0) {
        throw 'production interaction source tree is empty'
    }
    # Same digest Get-FileHash -Algorithm SHA256 produces - SHA256 over the
    # file's bytes - computed without paying the cmdlet's per-file overhead.
    # This walk runs once per gate evaluation and the self-test evaluates the
    # gate several hundred times; measured on the fixture tree it is the
    # difference between about 1.1 s and about 0.05 s per evaluation. The
    # tree digest this feeds is compared byte for byte against the previous
    # implementation's in the lane's differential evidence.
    # The per-file digest is kept, not just folded into the tree digest. It is
    # the SHA256 of the exact bytes this walk read, so every downstream rule
    # that derives something from one file can be keyed on it instead of
    # re-reading the file to find out whether it changed. The self-test's
    # source-rejection cases change exactly one of these files per iteration;
    # without the per-file digests every rule re-reads all 362 to discover that.
    $fileDigests = [System.Collections.Specialized.OrderedDictionary]::new(
        [StringComparer]::OrdinalIgnoreCase
    )
    $rowSha = [Security.Cryptography.SHA256]::Create()
    try {
        $rows = @(
            foreach ($file in $files) {
                $digest = ([BitConverter]::ToString(
                    $rowSha.ComputeHash([IO.File]::ReadAllBytes($file))
                )).Replace('-', '').ToLowerInvariant()
                $fileDigests[[string]$file] = $digest
                [ordered]@{
                    path = Get-ProductionInteractionRelativePath `
                        -Root $repo `
                        -Path $file
                    sha256 = $digest
                }
            }
        )
    }
    finally {
        $rowSha.Dispose()
    }
    $preimage = ($rows | ForEach-Object { "$($_.path)`0$($_.sha256)" }) -join "`n"
    $sha = [Security.Cryptography.SHA256]::Create()
    try {
        $treeSha = ([BitConverter]::ToString(
            $sha.ComputeHash([Text.Encoding]::UTF8.GetBytes($preimage))
        )).Replace('-', '').ToLowerInvariant()
    }
    finally {
        $sha.Dispose()
    }
    # Each walk gets a fresh identity and becomes the only one whose digests the
    # per-file memo will honour. An authority retained across an edit is then
    # refused instead of selecting derivations for bytes that are gone.
    $script:ProductionInteractionAuthorityWalkId = [guid]::NewGuid().ToString('N')
    return [pscustomobject]@{
        files = @([string[]]$files)
        file_count = $files.Count
        sha256 = $treeSha
        file_digests = $fileDigests
        walk_id = $script:ProductionInteractionAuthorityWalkId
    }
}

function Get-ProductionPassiveSourceAdmissionClosure {
    param([Parameter(Mandatory = $true)][string]$RepoRoot)

    $repo = [IO.Path]::GetFullPath($RepoRoot)
    $registryPath = Join-Path $repo 'extractor\native\domain_registry.json'
    $collectorPath = Join-Path $repo 'extractor\native\collector\src\attach_log.rs'
    if (-not (Test-Path -LiteralPath $registryPath -PathType Leaf) -or
        -not (Test-Path -LiteralPath $collectorPath -PathType Leaf)) {
        throw 'passive source-admission authority inputs are missing'
    }
    $registry = Get-Content -LiteralPath $registryPath -Raw | ConvertFrom-Json
    $productionStatuses = @('existing', 'production_complete', 'production_complete_base')
    # `capture_scheduled: false` marks a domain the owner ruled out of capture
    # scope; the key is absent for every domain that is still scheduled. These
    # are not production sources, so asking whether their source is statically
    # impossible is the wrong question -- this closure blocks a package for the
    # shape where a production source cannot exist, and a domain nobody requests
    # is not that shape. Owned by
    # compact_contract::DESCHEDULED_CAPTURE_DOMAIN_IDS.
    #
    # NOT because their hooks are absent. Most descheduled producers do sit
    # behind `!passive_product_capture`, but two do not: d75's edge is installed
    # for d64, which depends on it, and d85's whole unit is installed
    # unconditionally by the production hook registry (an open follow-up, see
    # documents/DECISIONS.md). Reasoning from this list to "these hooks are not
    # installed" is what would mis-scope that work.
    # The key is read through PSObject.Properties because it is ABSENT on every
    # domain that is still scheduled (the key marks the exception, not the rule), and `$_.capture_scheduled` on
    # an object that does not have it is a terminating PropertyNotFoundStrict
    # under the Set-StrictMode -Version Latest that every caller of this
    # function runs with. The package builder, the receipt verifier and the
    # package self-test all reach this line and all three died on it; the
    # interaction-closure gate never calls this function, which is why it went
    # unnoticed. Reproduced on the unmodified helper at aa602182.
    $productionIds = @(
        $registry.domains |
            Where-Object {
                $descheduled = $_.PSObject.Properties['capture_scheduled']
                $_.schedulable -eq $true -and
                -not ($null -ne $descheduled -and $descheduled.Value -eq $false) -and
                $productionStatuses -ccontains [string]$_.status
            } |
            Sort-Object { [int]$_.id } |
            ForEach-Object { [int]$_.id }
    )
    $collectorSource = Get-Content -LiteralPath $collectorPath -Raw
    $collectorCode = Get-RustCodeWithoutCommentsOrStrings -Text $collectorSource
    $passiveD71GatePattern =
        'if\s+passive_product_capture\s*\{\s*' +
        'if\s+compact_explicit_domain_enabled\s*\(\s*&control\s*,\s*' +
        'RAW_DYNAMIC_COLLECTION_TRACE_LIVE_DOMAIN\s*,?\s*\)\s*\{\s*' +
        'return\s+Err\s*\('
    $passiveD71GateCount = [regex]::Matches(
        $collectorCode,
        $passiveD71GatePattern
    ).Count
    if ($passiveD71GateCount -gt 1) {
        throw 'passive D71 source-admission gate is structurally ambiguous'
    }
    $staticallyImpossible = [System.Collections.Generic.List[object]]::new()
    if ($productionIds -contains 71 -and
        $passiveD71GateCount -eq 1) {
        $staticallyImpossible.Add([ordered]@{
            id = 71
            reason = 'passive_product_explicit_domain_gate_rejects_d71'
        })
    }
    return [ordered]@{
        schema = 'hoi4.production-passive-source-admission-closure.v1'
        production_domain_count = $productionIds.Count
        production_domain_ids = $productionIds
        statically_impossible_domains = @($staticallyImpossible)
        no_statically_proven_impossible_production_sources =
            ($staticallyImpossible.Count -eq 0)
    }
}

function Get-ProductionThreadSuspensionEntryPoints {
    # The behaviour this roster stands for is "stop another thread of this
    # process so its execution cannot be observed mid-change". Every name here
    # is an operating-system entry point, so it is fixed by the OS export
    # tables and cannot be renamed by the caller: a detour engine may call
    # itself anything, but it still has to reach one of these to suspend a
    # thread. That is what makes this roster a behaviour test and not a vendor
    # test.
    return @(
        'SuspendThread',
        'Wow64SuspendThread',
        'NtSuspendThread',
        'ZwSuspendThread',
        'NtSuspendProcess',
        'ZwSuspendProcess',
        'ResumeThread',
        'NtResumeThread',
        'ZwResumeThread',
        'NtResumeProcess',
        'ZwResumeProcess'
    )
}

function Get-ProductionCodeMutationEntryPoints {
    # The behaviour this roster stands for is "make live executable bytes
    # writable, rewrite them, and make the processor observe the rewrite".
    # Same argument as above: these are OS entry points, fixed by the export
    # tables.
    return @(
        'VirtualProtect',
        'VirtualProtectEx',
        'VirtualProtectFromApp',
        'NtProtectVirtualMemory',
        'ZwProtectVirtualMemory',
        'FlushInstructionCache',
        'WriteProcessMemory',
        'NtWriteVirtualMemory',
        'ZwWriteVirtualMemory',
        'mprotect'
    )
}

function Get-ProductionSourceLineNumber {
    param(
        [Parameter(Mandatory = $true)][AllowEmptyString()][string]$Text,
        [Parameter(Mandatory = $true)][int]$Index
    )

    if ($Index -le 0) {
        return 1
    }
    return ([regex]::Matches($Text.Substring(0, $Index), "`n").Count + 1)
}

function Get-RustStringLiteralValues {
    param([Parameter(Mandatory = $true)][AllowEmptyString()][string]$Text)

    # Same scan rules as Get-RustCodeWithoutCommentsOrStrings, but it keeps the
    # literals instead of blanking them. Two texts are needed rather than one:
    # identifiers must be read with comments and literals removed, while an
    # export name resolved at run time only ever appears as a literal. Matching
    # the exported name against the raw text instead would fire on a comment
    # that merely mentions the API.
    $literals = [System.Collections.Generic.List[object]]::new()
    $index = 0
    $blockDepth = 0
    while ($index -lt $Text.Length) {
        if ($blockDepth -gt 0) {
            if ($index + 1 -lt $Text.Length -and
                $Text[$index] -eq '/' -and $Text[$index + 1] -eq '*') {
                $blockDepth++
                $index += 2
                continue
            }
            if ($index + 1 -lt $Text.Length -and
                $Text[$index] -eq '*' -and $Text[$index + 1] -eq '/') {
                $blockDepth--
                $index += 2
                continue
            }
            $index++
            continue
        }
        if ($index + 1 -lt $Text.Length -and
            $Text[$index] -eq '/' -and $Text[$index + 1] -eq '/') {
            while ($index -lt $Text.Length -and $Text[$index] -ne "`n") {
                $index++
            }
            continue
        }
        if ($index + 1 -lt $Text.Length -and
            $Text[$index] -eq '/' -and $Text[$index + 1] -eq '*') {
            $blockDepth = 1
            $index += 2
            continue
        }
        # A character literal may contain a double quote ('"'), which would
        # otherwise be taken for the start of a string and desynchronise every
        # literal after it. A leading quote that is not a character literal is a
        # lifetime, and consuming just the quote is correct for that.
        if ($Text[$index] -eq "'") {
            $closing = -1
            if ($index + 1 -lt $Text.Length -and $Text[$index + 1] -eq '\') {
                for ($probe = $index + 2; $probe -lt [Math]::Min($index + 12, $Text.Length); $probe++) {
                    if ($Text[$probe] -eq "'") {
                        $closing = $probe
                        break
                    }
                }
            }
            elseif ($index + 2 -lt $Text.Length -and $Text[$index + 2] -eq "'") {
                $closing = $index + 2
            }
            if ($closing -ge 0) {
                $index = $closing + 1
            }
            else {
                $index++
            }
            continue
        }
        if ($Text[$index] -eq '"') {
            $start = $index
            $index++
            $value = [Text.StringBuilder]::new()
            while ($index -lt $Text.Length) {
                $character = $Text[$index]
                $index++
                if ($character -eq '\') {
                    if ($index -ge $Text.Length) {
                        break
                    }
                    # An export name reached through a run-time lookup is
                    # written as a NUL-terminated literal far more often than
                    # not, so the escapes have to be decoded rather than copied
                    # through: an undecoded \0 leaves a literal '0' on the end
                    # and the name never compares equal.
                    $escape = $Text[$index]
                    $index++
                    switch -CaseSensitive ($escape) {
                        '0' { [void]$value.Append([char]0) }
                        'n' { [void]$value.Append("`n") }
                        'r' { [void]$value.Append("`r") }
                        't' { [void]$value.Append("`t") }
                        '\' { [void]$value.Append('\') }
                        '"' { [void]$value.Append('"') }
                        "'" { [void]$value.Append("'") }
                        'x' {
                            $hex = ''
                            while ($hex.Length -lt 2 -and $index -lt $Text.Length -and
                                $Text[$index] -cmatch '[0-9A-Fa-f]') {
                                $hex += $Text[$index]
                                $index++
                            }
                            if ($hex.Length -gt 0) {
                                [void]$value.Append([char][Convert]::ToInt32($hex, 16))
                            }
                        }
                        'u' {
                            if ($index -lt $Text.Length -and $Text[$index] -eq '{') {
                                $index++
                                $hex = ''
                                while ($index -lt $Text.Length -and $Text[$index] -ne '}') {
                                    $hex += $Text[$index]
                                    $index++
                                }
                                if ($index -lt $Text.Length) {
                                    $index++
                                }
                                if ($hex -cmatch '^[0-9A-Fa-f]+$') {
                                    [void]$value.Append(
                                        [char]::ConvertFromUtf32([Convert]::ToInt32($hex, 16))
                                    )
                                }
                            }
                        }
                        default {
                            # A line continuation swallows the newline and the
                            # indentation that follows it. These files are CRLF,
                            # so the carriage return is part of the newline and
                            # not a character of its own.
                            if ($escape -eq "`n" -or $escape -eq "`r") {
                                if ($escape -eq "`r" -and $index -lt $Text.Length -and
                                    $Text[$index] -eq "`n") {
                                    $index++
                                }
                                while ($index -lt $Text.Length -and
                                    [char]::IsWhiteSpace($Text[$index])) {
                                    $index++
                                }
                            }
                            else {
                                [void]$value.Append($escape)
                            }
                        }
                    }
                    continue
                }
                if ($character -eq '"') {
                    break
                }
                [void]$value.Append($character)
            }
            $literals.Add([ordered]@{
                value = $value.ToString()
                index = $start
            })
            continue
        }
        $index++
    }
    if ($blockDepth -ne 0) {
        throw 'production Rust source has an unterminated block comment'
    }
    return @($literals)
}

function Test-ProductionCapabilityDeclaration {
    param(
        [Parameter(Mandatory = $true)][AllowEmptyString()][string]$Code,
        [Parameter(Mandatory = $true)][int]$NameIndex,
        [Parameter(Mandatory = $true)][int]$AfterNameIndex,
        [Parameter(Mandatory = $true)][bool]$CFamily
    )

    # Declaring the ABI is not the behaviour; calling it is. This is also what
    # keeps the pure binding crates (winapi, windows-sys) from reading as
    # patchers, since every occurrence there is a declaration.
    $prefix = $Code.Substring(0, $NameIndex)
    if ($prefix -cmatch '\bfn\s+$') {
        return $true
    }
    if (-not $CFamily) {
        return $false
    }
    # C and C++ have no `fn`, and no rule over adjacent tokens separates a
    # prototype from a call: `DWORD SuspendThread(HANDLE);` and
    # `TRACE SuspendThread(thread);` are the same text, one a declaration and one
    # a call through a macro, and the parameter list is no better a guide -
    # `SuspendThread(*thread);` is a call. Since the two are not decidable from
    # the text, the tie goes to reporting: only a positively recognised
    # declaration is excused, and everything else is a call. That is the safe
    # direction for a gate, because a missed site is a gate that lies.
    $cursor = $AfterNameIndex
    while ($cursor -lt $Code.Length -and [char]::IsWhiteSpace($Code[$cursor])) {
        $cursor++
    }
    if ($cursor -ge $Code.Length -or $Code[$cursor] -ne '(') {
        return $false
    }
    # A declarator introduces a name; an expression consumes one. Anything
    # ending in an operator is an expression, so an initialiser, a conditional
    # arm, a cast or a qualified call all stay on the reporting side.
    $declarator = $prefix.TrimEnd()
    if ($declarator.Length -gt 0 -and
        $declarator[$declarator.Length - 1] -cnotmatch '[A-Za-z0-9_*&};]') {
        return $false
    }
    # And the name has to sit somewhere a declaration can live: file scope, or a
    # block positively recognised as one that holds declarations. An `else`, a
    # `do`, a `try`, a lambda or a bare nested block is not one of those, so a
    # call inside any of them is still a call.
    return @('file_scope', 'declaration_block') -ccontains (
        Get-CFamilyEnclosingBlockKind -Code $Code -NameIndex $NameIndex
    )
}

function Get-CFamilyEnclosingBlockKind {
    param(
        [Parameter(Mandatory = $true)][AllowEmptyString()][string]$Code,
        [Parameter(Mandatory = $true)][int]$NameIndex
    )

    $depth = 0
    for ($cursor = $NameIndex - 1; $cursor -ge 0; $cursor--) {
        $character = $Code[$cursor]
        if ($character -eq '}') {
            $depth++
            continue
        }
        if ($character -ne '{') {
            continue
        }
        if ($depth -gt 0) {
            $depth--
            continue
        }
        # Recognised positively from the WHOLE header - the text since the
        # previous statement or block boundary - and never from a keyword that
        # merely appears somewhere in it. `extern int patch(void *t) {` opens a
        # function body even though it starts with `extern`, and
        # `int patch(struct Thread *t) {` opens one even though it contains
        # `struct`. An unrecognised header is executable, which is the reporting
        # side.
        $header = $Code.Substring(0, $cursor)
        $boundary = [Math]::Max(
            [Math]::Max($header.LastIndexOf(';'), $header.LastIndexOf('{')),
            $header.LastIndexOf('}')
        )
        $header = $header.Substring($boundary + 1)
        # A class, struct, union, enum or namespace header carries no parameter
        # list, so any parenthesis in it means this is not one of them.
        if ($header -cmatch
            '^\s*(?:typedef\s+)?(?:template\s*<[^{}()]*>\s*)?(?:namespace|class|struct|union|enum)(?:\s+(?:class|struct))?\b[^;{}()]*$') {
            return 'declaration_block'
        }
        # A linkage specification's string is already blanked by the stripper, so
        # `extern "C"` reads as `extern` followed by spaces. Only an attribute or
        # a declspec may stand between it and the brace, and each one is
        # delimited structurally: an unbounded payload would let
        # `extern __declspec(dllexport) int patch(void *t) {` match and hide a
        # whole function body.
        $attribute =
            '(?:__attribute__\s*\(\((?:[^;{}()]|\([^;{}()]*\))*\)\)' +
            '|__declspec\s*\((?:[^;{}()]|\([^;{}()]*\))*\))'
        if ($header -cmatch "^\s*extern\s*(?:$attribute\s*)*$") {
            return 'declaration_block'
        }
        return 'executable_block'
    }
    return 'file_scope'
}

function Get-RustImportItemRanges {
    param([Parameter(Mandatory = $true)][AllowEmptyString()][string]$Code)

    # Naming an export in an import brings it into scope; it does not reach it.
    # The call, the alias call, or a value use does. A `use` item cannot contain
    # a semicolon, so the item is exactly the text from the reserved word to the
    # first one - which is also true of a braced use tree spanning several lines.
    # The word is located on its own rather than through what precedes it, since
    # an attribute can share the line and a visibility can span lines, and `use`
    # is reserved so it cannot be an identifier or a field.
    # An import item is never inside a parenthesis, a bracket, or a macro's token
    # tree, and a macro may be delimited by any of the three - `invoke! { use
    # SuspendThread; }` is a call, not an import. Ordinary module and block braces
    # do hold imports, so braces are only excluded when the brace itself opens a
    # macro token tree, which is what the frame stack distinguishes.
    $shielded = [bool[]]::new($Code.Length + 1)
    $frames = [System.Collections.Generic.List[bool]]::new()
    $shieldDepth = 0
    for ($cursor = 0; $cursor -lt $Code.Length; $cursor++) {
        $character = $Code[$cursor]
        if ($character -eq '(' -or $character -eq '[' -or $character -eq '{') {
            $isShield = ($character -ne '{')
            if (-not $isShield) {
                # Read backwards over the whitespace rather than copying the
                # prefix: this runs for every brace in every scanned file.
                $probe = $cursor - 1
                while ($probe -ge 0 -and [char]::IsWhiteSpace($Code[$probe])) {
                    $probe--
                }
                $isShield = ($probe -ge 0 -and $Code[$probe] -eq '!')
            }
            $frames.Add($isShield)
            if ($isShield) { $shieldDepth++ }
        }
        elseif ($character -eq ')' -or $character -eq ']' -or $character -eq '}') {
            if ($frames.Count -gt 0) {
                if ($frames[$frames.Count - 1]) { $shieldDepth-- }
                $frames.RemoveAt($frames.Count - 1)
            }
        }
        $shielded[$cursor] = ($shieldDepth -gt 0)
    }
    $ranges = [System.Collections.Generic.List[object]]::new()
    foreach ($match in [regex]::Matches(
        $Code,
        '(?<![A-Za-z0-9_])use[ \t\r\n][^;]*;'
    )) {
        if ($shielded[$match.Index]) {
            continue
        }
        $ranges.Add([ordered]@{
            start = $match.Index
            end = $match.Index + $match.Length
        })
    }
    return @($ranges)
}

function Test-ProductionImportContext {
    param(
        [Parameter(Mandatory = $true)]$ImportRanges,
        [Parameter(Mandatory = $true)][int]$NameIndex
    )

    foreach ($range in @($ImportRanges)) {
        if ($NameIndex -ge [int]$range.start -and $NameIndex -lt [int]$range.end) {
            return $true
        }
    }
    return $false
}

function Test-ProductionCallArgumentContext {
    param(
        [Parameter(Mandatory = $true)][AllowEmptyString()][string]$Code,
        [Parameter(Mandatory = $true)][int]$Index,
        [Parameter(Mandatory = $true)][bool]$CFamily
    )

    # A run-time lookup passes the export name to something. A diagnostic
    # constant or a log line holding the same text does not, so the literal only
    # counts when it sits in the argument list of a call - and a macro invocation
    # is not one. Grouping parentheses and the parentheses of a control
    # construct are walked through rather than answered, so `resolve((name))`
    # still counts and `if (name == other)` still does not.
    #
    # A word is only excused as a control construct where the language reserves
    # it, so C `match(name)` and Rust `r#match(name)` are calls.
    $controlKeywords = if ($CFamily) {
        @('if', 'while', 'for', 'switch', 'return', 'sizeof', 'catch', 'else',
            'do', 'alignof', 'typeof', 'decltype', 'static_assert', 'defined',
            '_Alignof', '_Static_assert')
    }
    else {
        @('if', 'while', 'for', 'return', 'match', 'else', 'loop')
    }
    $namedCasts = @('static_cast', 'dynamic_cast', 'const_cast', 'reinterpret_cast')
    $depth = 0
    for ($cursor = $Index - 1; $cursor -ge 0; $cursor--) {
        $character = $Code[$cursor]
        if ($character -eq ')') {
            $depth++
            continue
        }
        if ($character -ne '(') {
            continue
        }
        if ($depth -gt 0) {
            $depth--
            continue
        }
        $prefix = $Code.Substring(0, $cursor).TrimEnd()
        if ($prefix.Length -eq 0) {
            continue
        }
        $last = $prefix[$prefix.Length - 1]
        if ($last -eq '!') {
            return $false
        }
        if ($last -eq '>') {
            # A named cast's template argument list is not a callee.
            if ($prefix -cmatch
                ('(?<![A-Za-z0-9_])(?<word>' + ($namedCasts -join '|') + ')\s*<[^<>]*>$')) {
                continue
            }
            return $true
        }
        if ($last -eq ']') {
            return $true
        }
        if ($last -eq ')') {
            # A completed control condition is not a callee either. A C-style
            # cast is indistinguishable from a call through a parenthesised
            # function pointer without a type table, and stays on the reporting
            # side.
            $withoutCondition = Remove-TrailingParenthesisedGroup -Text $prefix
            if ($null -ne $withoutCondition -and
                $withoutCondition -cmatch
                    ('(?<![A-Za-z0-9_])(?<word>' + (($controlKeywords | ForEach-Object {
                        [regex]::Escape($_)
                    }) -join '|') + ')$')) {
                continue
            }
            return $true
        }
        if ($prefix -cmatch '(?<![A-Za-z0-9_])(?<word>[A-Za-z_][A-Za-z0-9_]*)$') {
            $word = $Matches['word']
            $wordStart = $prefix.Length - $word.Length
            $rawIdentifier = ($wordStart -ge 2 -and
                $prefix.Substring($wordStart - 2, 2) -ceq 'r#')
            if (-not $rawIdentifier -and ($controlKeywords -ccontains $word)) {
                continue
            }
            return $true
        }
    }
    return $false
}

function Remove-TrailingParenthesisedGroup {
    param([Parameter(Mandatory = $true)][AllowEmptyString()][string]$Text)

    if (-not $Text.EndsWith(')')) {
        return $null
    }
    $depth = 0
    for ($cursor = $Text.Length - 1; $cursor -ge 0; $cursor--) {
        $character = $Text[$cursor]
        if ($character -eq ')') {
            $depth++
            continue
        }
        if ($character -eq '(') {
            $depth--
            if ($depth -eq 0) {
                return $Text.Substring(0, $cursor).TrimEnd()
            }
        }
    }
    return $null
}

function Test-ProductionExportResolverCall {
    param(
        [Parameter(Mandatory = $true)][AllowEmptyString()][string]$Code,
        [Parameter(Mandatory = $true)][bool]$CFamily
    )

    foreach ($match in [regex]::Matches(
        $Code,
        '(?<![A-Za-z0-9_])GetProcAddress[AW]?(?![A-Za-z0-9_])'
    )) {
        $afterIndex = $match.Index + $match.Length
        if (Test-ProductionCapabilityDeclaration `
                -Code $Code `
                -NameIndex $match.Index `
                -AfterNameIndex $afterIndex `
                -CFamily $CFamily) {
            continue
        }
        $after = $Code.Substring(
            $afterIndex,
            [Math]::Min(8, $Code.Length - $afterIndex)
        )
        if ($after -cmatch '^\s*\(') {
            return $true
        }
    }
    return $false
}

function Get-ProductionCapabilityEvidence {
    param(
        [Parameter(Mandatory = $true)][AllowEmptyString()][string]$Source,
        [Parameter(Mandatory = $true)][AllowEmptyString()][string]$Code,
        [Parameter(Mandatory = $true)][string[]]$EntryPoints,
        [Parameter(Mandatory = $true)][string]$Path,
        [switch]$CFamily
    )

    # The whole input is in the key: the two texts the answer is read out of,
    # the roster it is read against, the path that is stamped into every site,
    # and the language flag that selects the declaration and call-context rules.
    $key = 'capability-evidence:' + (Get-ProductionInteractionContentKey -Text (
        @(
            (Get-ProductionInteractionContentKey -Text $Source),
            (Get-ProductionInteractionContentKey -Text $Code),
            (@($EntryPoints) -join "`0"),
            $Path,
            [string][bool]$CFamily
        ) -join "`n"
    ))
    if ($script:ProductionInteractionDerivationCache.Contains($key)) {
        # The returned record and its site entries are read-only to every caller
        # in this file: accumulators copy site references, and `resolves_exports`
        # is set on the accumulator, never back onto the evidence. Sharing one
        # instance across hits is therefore indistinguishable from recomputing.
        return $script:ProductionInteractionDerivationCache[$key]
    }
    $evidence = Get-ProductionCapabilityEvidenceCore `
        -Source $Source `
        -Code $Code `
        -EntryPoints $EntryPoints `
        -Path $Path `
        -CFamily:$CFamily
    Set-ProductionInteractionDerivation `
        -Key $key `
        -Value $evidence `
        -Weight (256 + 256 * (@($evidence.sites).Count + @($evidence.literal_capabilities).Count))
    return $evidence
}

function Get-ProductionCapabilityEvidenceCore {
    param(
        [Parameter(Mandatory = $true)][AllowEmptyString()][string]$Source,
        [Parameter(Mandatory = $true)][AllowEmptyString()][string]$Code,
        [Parameter(Mandatory = $true)][string[]]$EntryPoints,
        [Parameter(Mandatory = $true)][string]$Path,
        [switch]$CFamily
    )

    $sites = [System.Collections.Generic.List[object]]::new()
    # `use` is a reserved word in Rust only; in C it could be a macro, and a
    # macro must not be able to shield a value use.
    $importRanges = @(
        if (-not $CFamily) { Get-RustImportItemRanges -Code $Code }
    )

    # An OS export can be reached under any local spelling: `use ... as`, a
    # #[link_name] attribute, or a function pointer bound to the name. All of
    # them have to write the fixed export name somewhere, so every occurrence
    # that is not a declaration and not a bare import is evidence, and any local
    # name it is bound to is followed through to its calls.
    $aliases = [ordered]@{}
    foreach ($aliasMatch in [regex]::Matches(
        $Source,
        '#\[\s*link_name\s*=\s*"(?<cap>[A-Za-z0-9_@.$]+)"\s*\][\s\S]{0,400}?\bfn\s+(?<alias>[A-Za-z_][A-Za-z0-9_]*)'
    )) {
        $capability = $aliasMatch.Groups['cap'].Value
        if ($EntryPoints -ccontains $capability) {
            $aliases[$aliasMatch.Groups['alias'].Value] = $capability
        }
    }

    foreach ($entryPoint in $EntryPoints) {
        $namePattern =
            "(?<![A-Za-z0-9_])$([regex]::Escape($entryPoint))(?![A-Za-z0-9_])"
        foreach ($match in [regex]::Matches($Code, $namePattern)) {
            $afterIndex = $match.Index + $match.Length
            if (Test-ProductionCapabilityDeclaration `
                    -Code $Code `
                    -NameIndex $match.Index `
                    -AfterNameIndex $afterIndex `
                    -CFamily ([bool]$CFamily)) {
                continue
            }
            $after = $Code.Substring(
                $afterIndex,
                [Math]::Min(160, $Code.Length - $afterIndex)
            )
            if ($after -cmatch '^\s+as\s+(?<alias>[A-Za-z_][A-Za-z0-9_]*)') {
                $aliases[$Matches['alias']] = $entryPoint
            }
            $isCall = ($after -cmatch '^\s*\(')
            if (-not $isCall -and (Test-ProductionImportContext `
                        -ImportRanges $importRanges `
                        -NameIndex $match.Index)) {
                continue
            }
            # A name immediately followed by an assignment is being DEFINED, not
            # used: `SuspendThread = 66` is a variant of an enum that happens to
            # spell the entry point. A name that is bound to something -
            # `let suspend = SuspendThread;` - is on the other side of that `=`
            # and is a use of the function pointer, not a definition of a
            # constant, so it stays a `capability_reference`.
            #
            # `==` and `=>` are excluded because neither defines the name, and
            # this distinction only ever REMOVES evidence for crates outside the
            # repository. Inside it, every kind counts, so a definition site is
            # still measured, still reported and still pinned.
            $isDefinition = (-not $isCall -and $after -cmatch '^\s*=(?![=>])')
            $sites.Add([ordered]@{
                path = $Path
                line = Get-ProductionSourceLineNumber -Text $Code -Index $match.Index
                capability = $entryPoint
                token = $entryPoint
                kind = if ($isCall) { 'direct_call' }
                    elseif ($isDefinition) { 'capability_definition' }
                    else { 'capability_reference' }
            })
        }
    }

    foreach ($alias in $aliases.Keys) {
        $aliasName = [string]$alias
        if ($EntryPoints -ccontains $aliasName) {
            continue
        }
        $pattern =
            "(?<![A-Za-z0-9_])(?<!\bfn\s+)$([regex]::Escape($aliasName))\s*\("
        foreach ($match in [regex]::Matches($Code, $pattern)) {
            $sites.Add([ordered]@{
                path = $Path
                line = Get-ProductionSourceLineNumber -Text $Code -Index $match.Index
                capability = [string]$aliases[$alias]
                token = $aliasName
                kind = 'alias_call'
            })
        }
    }

    # The remaining way to reach a fixed export without writing its name as an
    # identifier is to look it up at run time. That lookup needs the exported
    # name as a literal, so the caller pairs an export resolver anywhere in the
    # unit with a literal whose whole content is one of the entry points.
    # Literals come from the lexer, so a comment mentioning the API in quotes
    # cannot produce one, and a shared resolver in a sibling file cannot hide
    # one.
    $literalCapabilities = [System.Collections.Generic.List[object]]::new()
    foreach ($literal in @(Get-RustStringLiteralValues -Text $Source)) {
        $value = ([string]$literal.value).TrimEnd([char]0)
        if (-not ($EntryPoints -ccontains $value)) {
            continue
        }
        # The stripper preserves length, so a literal's index in the raw text is
        # the same index in the stripped code and the enclosing expression can
        # be read there.
        if (-not (Test-ProductionCallArgumentContext `
                    -Code $Code `
                    -Index ([int]$literal.index) `
                    -CFamily ([bool]$CFamily))) {
            continue
        }
        $literalCapabilities.Add([ordered]@{
            path = $Path
            line = Get-ProductionSourceLineNumber `
                -Text $Source `
                -Index ([int]$literal.index)
            capability = $value
            token = $value
            kind = 'dynamic_resolution'
        })
    }

    return [ordered]@{
        sites = @($sites)
        literal_capabilities = @($literalCapabilities)
        resolves_exports = Test-ProductionExportResolverCall `
            -Code $Code `
            -CFamily ([bool]$CFamily)
    }
}

function Add-ProductionCapabilityEvidence {
    param(
        [Parameter(Mandatory = $true)]$Accumulator,
        [Parameter(Mandatory = $true)]$Evidence
    )

    foreach ($site in @($Evidence.sites)) {
        $Accumulator.sites.Add($site)
    }
    foreach ($literal in @($Evidence.literal_capabilities)) {
        $Accumulator.literals.Add($literal)
    }
    if ($Evidence.resolves_exports) {
        $Accumulator.resolves_exports = $true
    }
}

function New-ProductionCapabilityAccumulator {
    return [ordered]@{
        sites = [System.Collections.Generic.List[object]]::new()
        literals = [System.Collections.Generic.List[object]]::new()
        resolves_exports = $false
    }
}

function Resolve-ProductionCapabilitySites {
    param([Parameter(Mandatory = $true)]$Accumulator)

    $sites = [System.Collections.Generic.List[object]]::new()
    foreach ($site in @($Accumulator.sites)) {
        $sites.Add($site)
    }
    if ($Accumulator.resolves_exports) {
        foreach ($literal in @($Accumulator.literals)) {
            $sites.Add($literal)
        }
    }
    return @($sites)
}

function Get-ProductionInteractionScannableSource {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$EarlyRenderEvidencePath
    )

    $source = Get-Content -LiteralPath $Path -Raw
    if ([IO.Path]::GetFullPath($Path) -ine
        [IO.Path]::GetFullPath($EarlyRenderEvidencePath)) {
        return $source
    }
    $actuatorStart = $source.IndexOf(
        '#[cfg(all(windows, feature = "evidence-client-driving"))]',
        [StringComparison]::Ordinal
    )
    $observerStart = $source.IndexOf(
        '#[cfg(all(windows, not(feature = "evidence-client-driving")))]',
        [StringComparison]::Ordinal
    )
    if ($actuatorStart -lt 0 -or $observerStart -le $actuatorStart) {
        throw 'early-render source lacks a bounded evidence-only actuator region'
    }
    # Blank the region rather than delete it, so every reported line number
    # after it still names the line the reader will open.
    $excised = $source.Substring($actuatorStart, $observerStart - $actuatorStart)
    $blanked = [regex]::Replace($excised, '[^\r\n]', ' ')
    return $source.Remove($actuatorStart, $observerStart - $actuatorStart).Insert(
        $actuatorStart,
        $blanked
    )
}

function Get-ProductionInteractionSourceRoots {
    param([Parameter(Mandatory = $true)][string]$RepoRoot)

    $repo = [IO.Path]::GetFullPath($RepoRoot)
    return @(
        (Join-Path $repo 'extractor\native\recorder_host'),
        (Join-Path $repo 'extractor\native\collector'),
        (Join-Path $repo 'replay_interface\native\replay_app')
    ) + @(Get-CollectorProductionLaneRoots -RepoRoot $repo)
}

function Get-ProductionCrateSourceCapabilityScan {
    param(
        [Parameter(Mandatory = $true)][string]$Root,
        [Parameter(Mandatory = $true)][string[]]$SuspensionEntryPoints,
        [Parameter(Mandatory = $true)][string[]]$CodeMutationEntryPoints
    )

    # Cached on ONE pass, after three earlier attempts at a safe key failed.
    #
    # A cache is only sound here if the key describes exactly the bytes the
    # value was derived from, and this function walks a directory. Three keys
    # that do not survive that were tried and removed:
    #   - a path/length/last-write-tick key is defeated by a same-length edit
    #     that restores the timestamp;
    #   - a content key read in a pass BEFORE the scan files a value derived
    #     from state B under a key describing state A;
    #   - re-reading the key in a pass AFTER the scan narrows that to a change
    #     made and reverted inside the scan but does not close it, and it
    #     persists that torn read after the tree is stable again.
    # Every one of those has two passes. This one has one: the loop below reads
    # each admitted file exactly once, keeps the text it read, and digests that
    # same text. The key therefore describes the precise bytes the value is
    # derived from - a change made mid-walk lands in the key as well as in the
    # answer, exactly as it would in an uncached scan - and there is no second
    # read to disagree with the first.
    #
    # What the earlier note got wrong was the cost, and it was measured rather
    # than argued: reading every admitted source under all 22 production
    # dependency roots is 0.75 s and digesting them is 0.04 s, against 2.3 s for
    # one uncached derivation pass over the same corpus with the per-file caches
    # already warm. The derivation, not the read, is the cost - most of it the
    # entry-point prefilter over 40 MB of registry sources, which a hit skips
    # entirely. The classification above re-enters this function about ten times
    # per crate in one evaluation, because the reachability closure is computed
    # once per (crate, lock) pair and each of those walks the shared in-repository
    # crates again with its own visited set.
    #
    # The root path is in the key. Two copies of one crate at different paths
    # cannot share an entry, which costs nothing real (the returned record names
    # its own root) and keeps the value's `root` field honest.
    #
    # The per-file strip and capability-evidence caches below still apply on a
    # miss, and are sound for the same reason: their key is the exact text
    # handed to them, not a re-read of a path.
    $sourceExtensions = @(Get-ProductionInteractionSourceExtensions)
    $excludedTopLevel = @('tests', 'benches', 'examples', 'fuzz', 'target')
    $rootFull = [IO.Path]::GetFullPath($Root).TrimEnd('\')
    $suspension = New-ProductionCapabilityAccumulator
    $mutation = New-ProductionCapabilityAccumulator
    $cFamilyExtensions = @('.c', '.h', '.cc', '.cpp')
    $allEntryPoints = @($SuspensionEntryPoints) + @($CodeMutationEntryPoints) +
        @('GetProcAddress')
    $prefilter = '(?:' + (($allEntryPoints | ForEach-Object {
        [regex]::Escape($_)
    }) -join '|') + ')'
    $admitted = [System.Collections.Generic.List[object]]::new()
    $keyPreimage = [Text.StringBuilder]::new()
    $keySha = [Security.Cryptography.SHA256]::Create()
    try {
        foreach ($file in @(Get-ChildItem -LiteralPath $rootFull -Recurse -Force -File)) {
            if ($sourceExtensions -notcontains $file.Extension) {
                continue
            }
            $relative = $file.FullName.Substring($rootFull.Length + 1).Replace('\', '/')
            $firstSegment = ($relative -split '/')[0]
            if ($relative.Contains('/') -and ($excludedTopLevel -contains $firstSegment)) {
                continue
            }
            $source = Get-Content -LiteralPath $file.FullName -Raw
            if ($null -eq $source) { $source = '' }
            $admitted.Add([pscustomobject]@{
                Relative = $relative
                Source = $source
                CFamily = ($cFamilyExtensions -contains $file.Extension)
            })
            [void]$keyPreimage.Append($relative).Append("`0").Append(
                [BitConverter]::ToString(
                    $keySha.ComputeHash([Text.Encoding]::UTF8.GetBytes($source))
                ).Replace('-', '')
            ).Append("`n")
        }
    }
    finally {
        $keySha.Dispose()
    }
    $scanKey = 'crate-capability-scan:' + (Get-ProductionInteractionContentKey -Text (
        @(
            $rootFull,
            (@($SuspensionEntryPoints) -join "`0"),
            (@($CodeMutationEntryPoints) -join "`0"),
            $keyPreimage.ToString()
        ) -join "`n"
    ))
    if ($script:ProductionInteractionDerivationCache.Contains($scanKey)) {
        # Read-only to every caller: the reachability closure copies site
        # references out of the record and never writes back into it, and the
        # classification stamps its digest onto its own closure record rather
        # than onto this one. Sharing one instance across hits is therefore
        # indistinguishable from recomputing.
        return $script:ProductionInteractionDerivationCache[$scanKey]
    }
    foreach ($entry in $admitted) {
        $source = [string]$entry.Source
        if ([string]::IsNullOrEmpty($source) -or $source -cnotmatch $prefilter) {
            continue
        }
        $relative = [string]$entry.Relative
        $cFamily = [bool]$entry.CFamily
        $code = Get-RustCodeWithoutCommentsOrStrings -Text $source
        Add-ProductionCapabilityEvidence -Accumulator $suspension -Evidence (
            Get-ProductionCapabilityEvidence `
                -Source $source `
                -Code $code `
                -EntryPoints $SuspensionEntryPoints `
                -Path $relative `
                -CFamily:$cFamily
        )
        Add-ProductionCapabilityEvidence -Accumulator $mutation -Evidence (
            Get-ProductionCapabilityEvidence `
                -Source $source `
                -Code $code `
                -EntryPoints $CodeMutationEntryPoints `
                -Path $relative `
                -CFamily:$cFamily
        )
    }
    # An export resolver anywhere in the crate arms the literal evidence for
    # the whole crate, so splitting the lookup across files does not hide it.
    $resolvesExports = ($suspension.resolves_exports -or $mutation.resolves_exports)
    $suspension.resolves_exports = $resolvesExports
    $mutation.resolves_exports = $resolvesExports
    $suspensionSites = @(Resolve-ProductionCapabilitySites -Accumulator $suspension)
    $mutationSites = @(Resolve-ProductionCapabilitySites -Accumulator $mutation)
    $scan = [ordered]@{
        root = $rootFull
        suspension_sites = @($suspensionSites)
        code_mutation_sites = @($mutationSites)
        # The two halves are reported separately because they are separately
        # composable. A crate that only stops threads is still the thread-stop
        # half of a suspending code mutation when its dependent writes the
        # bytes, and reporting only the conjunction is what let exactly that
        # split walk out of this gate: four lanes moved their suspender into a
        # shared crate, kept the VirtualProtect, and both halves stopped being
        # visible at once.
        suspends = ($suspensionSites.Count -gt 0)
        mutates_code = ($mutationSites.Count -gt 0)
        suspends_and_mutates_code =
            ($suspensionSites.Count -gt 0 -and $mutationSites.Count -gt 0)
    }
    Set-ProductionInteractionDerivation `
        -Key $scanKey `
        -Value $scan `
        -Weight (256 + 256 * ($suspensionSites.Count + $mutationSites.Count))
    return $scan
}

function Get-CargoLockPackageVersion {
    param(
        [Parameter(Mandatory = $true)][string]$LockPath,
        [Parameter(Mandatory = $true)][string]$Package
    )

    if (-not (Test-Path -LiteralPath $LockPath -PathType Leaf)) {
        throw "production dependency lock file is missing: $LockPath"
    }
    $lock = Get-Content -LiteralPath $LockPath -Raw
    # One regex over the whole lock per package, and the classification asks for
    # every registry dependency of every unit. Keyed on the lock's exact text
    # and the package name, which is the entire input to the answer.
    $versionKey = 'cargo-lock-package-version:' +
        (Get-ProductionInteractionContentKey -Text (
            @((Get-ProductionInteractionContentKey -Text $lock), $Package) -join "`n"
        ))
    if ($script:ProductionInteractionDerivationCache.Contains($versionKey)) {
        return [string]$script:ProductionInteractionDerivationCache[$versionKey]
    }
    $versions = @([regex]::Matches(
        $lock,
        '(?m)^\[\[package\]\]\r?\nname = "' + [regex]::Escape($Package) +
            '"\r?\nversion = "(?<version>[^"]+)"'
    ) | ForEach-Object { $_.Groups['version'].Value } | Select-Object -Unique)
    if ($versions.Count -ne 1) {
        throw "production dependency '$Package' does not resolve to one exact locked version"
    }
    Set-ProductionInteractionDerivation `
        -Key $versionKey `
        -Value ([string]$versions[0]) `
        -Weight 128
    return [string]$versions[0]
}

function Get-CargoRegistryCrateRoot {
    param(
        [Parameter(Mandatory = $true)][string]$Package,
        [Parameter(Mandatory = $true)][string]$Version
    )

    $cargoHome = if (-not [string]::IsNullOrWhiteSpace($env:CARGO_HOME)) {
        $env:CARGO_HOME
    }
    else {
        Join-Path $env:USERPROFILE '.cargo'
    }
    $sourceRoot = Join-Path $cargoHome 'registry\src'
    if (-not (Test-Path -LiteralPath $sourceRoot -PathType Container)) {
        throw (
            "production dependency behaviour is unprovable: no cargo registry " +
            "source cache at $sourceRoot"
        )
    }
    $candidates = @(
        Get-ChildItem -LiteralPath $sourceRoot -Directory |
            ForEach-Object { Join-Path $_.FullName "$Package-$Version" } |
            Where-Object { Test-Path -LiteralPath $_ -PathType Container }
    )
    if ($candidates.Count -ne 1) {
        throw (
            "production dependency behaviour is unprovable: '$Package' $Version " +
            "has $($candidates.Count) extracted registry sources"
        )
    }
    return [string]$candidates[0]
}

function Get-CrateOwningCargoLockPath {
    param([Parameter(Mandatory = $true)][string]$CrateRoot)

    $probe = Split-Path -Parent ([IO.Path]::GetFullPath($CrateRoot).TrimEnd('\'))
    while (-not [string]::IsNullOrEmpty($probe)) {
        $candidate = Join-Path $probe 'Cargo.lock'
        if (Test-Path -LiteralPath $candidate -PathType Leaf) {
            return $candidate
        }
        $probe = Split-Path -Parent $probe
    }
    return $null
}

function Get-CrateManifestDependencyRoots {
    # The one resolution of "which crate roots does this manifest name", for any
    # crate in the repository - a production unit or a crate a production unit
    # depends on. Both callers below go through this, so a dependency shape the
    # resolver stops understanding stops being followed everywhere at once and
    # cannot go missing from one traversal while another still sees it.
    #
    # dev-dependencies and build-dependencies are not here, because
    # Get-TomlProductionDependencyRecords does not return them: their table
    # names are `dev-dependencies` and `build-dependencies`, not `dev` and
    # `build` sub-tables of `dependencies`. What a production artifact links is
    # the normal and target-scoped graph.
    param(
        [Parameter(Mandatory = $true)][string]$CrateRoot,
        # The lock of the workspace that is BUILDING this crate. A path
        # dependency shared between workspaces - the portable_replay crates are
        # built by both extractor/native and replay_interface/native - has no
        # Cargo.lock above it, and the versions its own registry dependencies
        # resolve to are decided by whichever workspace pulled it in. Walking up
        # from the crate is only correct for a crate that owns its workspace.
        [string]$LockPath
    )

    $crateFull = [IO.Path]::GetFullPath($CrateRoot).TrimEnd('\')
    $manifestPath = Join-Path $crateFull 'Cargo.toml'
    if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) {
        throw "production unit Cargo manifest is missing: $manifestPath"
    }
    $lockPath = if (-not [string]::IsNullOrWhiteSpace($LockPath)) {
        [IO.Path]::GetFullPath($LockPath)
    }
    else {
        Get-CrateOwningCargoLockPath -CrateRoot $crateFull
    }
    if ([string]::IsNullOrWhiteSpace($lockPath)) {
        throw "production unit has no owning Cargo.lock: $crateFull"
    }
    if (-not (Test-Path -LiteralPath $lockPath -PathType Leaf)) {
        throw "production unit Cargo.lock is missing: $lockPath"
    }
    $workspaceRoot = Split-Path -Parent $lockPath
    $workspaceCargoPath = Join-Path $workspaceRoot 'Cargo.toml'
    $workspaceRecords = @(
        if (Test-Path -LiteralPath $workspaceCargoPath -PathType Leaf) {
            Get-TomlProductionDependencyRecords -Toml (
                Get-Content -LiteralPath $workspaceCargoPath -Raw
            ) | Where-Object { [string]$_.scope -ceq 'workspace.dependencies' }
        }
    )
    $dependencies = [System.Collections.Generic.List[object]]::new()
    foreach ($record in @(Get-TomlProductionDependencyRecords -Toml (
                Get-Content -LiteralPath $manifestPath -Raw
            ))) {
        if ([string]$record.scope -ceq 'workspace.dependencies') {
            continue
        }
        $dependencyName = [string]$record.name
        $package = if ([string]::IsNullOrWhiteSpace([string]$record.package)) {
            $dependencyName
        }
        else {
            [string]$record.package
        }
        $dependencyPath = [string]$record.path
        $dependencyBase = $crateFull
        if ([string]::IsNullOrWhiteSpace($dependencyPath) -and $record.workspace) {
            $inherited = @($workspaceRecords | Where-Object {
                [string]$_.name -ceq $dependencyName
            })
            if ($inherited.Count -eq 1) {
                $dependencyPath = [string]$inherited[0].path
                $dependencyBase = $workspaceRoot
                if (-not [string]::IsNullOrWhiteSpace([string]$inherited[0].package)) {
                    $package = [string]$inherited[0].package
                }
            }
        }
        $isPathDependency = (-not [string]::IsNullOrWhiteSpace($dependencyPath))
        $dependencyRoot = if ($isPathDependency) {
            [IO.Path]::GetFullPath((Join-Path $dependencyBase $dependencyPath)).TrimEnd('\')
        }
        else {
            Get-CargoRegistryCrateRoot `
                -Package $package `
                -Version (Get-CargoLockPackageVersion -LockPath $lockPath -Package $package)
        }
        $dependencies.Add([ordered]@{
            name = $dependencyName
            package = $package
            root = $dependencyRoot
            path_dependency = $isPathDependency
        })
    }
    return [ordered]@{
        crate_root = $crateFull
        manifest_path = $manifestPath
        lock_path = $lockPath
        workspace_root = $workspaceRoot
        workspace_cargo_path = $workspaceCargoPath
        dependencies = @($dependencies)
    }
}

function Get-ProductionUnitDependencyClosure {
    # The one traversal that answers "what does a production unit reach through
    # its manifest": for every production unit, the manifest that declares its
    # dependencies, the Cargo.lock that owns it, the workspace manifest that
    # inherited dependencies resolve against, and the resolved root of every
    # dependency it declares - path dependencies inside the repository and
    # registry crates in the cargo source cache alike.
    #
    # It is a closure, not a scan: it says which roots the authority will read,
    # and callers decide what to do with them. Get-ProductionDependencyCode-
    # MutationClassification scans them for capabilities; the package self-test
    # materialises them into its fixture. Both walk the same edges because they
    # call this, so a fixture can no longer be built from a stale hand-written
    # list of the paths someone believed the authority followed.
    param([Parameter(Mandatory = $true)][string]$RepoRoot)

    $repo = [IO.Path]::GetFullPath($RepoRoot)
    $unitRoots = @(Get-ProductionInteractionSourceRoots -RepoRoot $repo)
    $unitRootSet = [System.Collections.Generic.HashSet[string]]::new(
        [StringComparer]::OrdinalIgnoreCase
    )
    foreach ($unitRoot in $unitRoots) {
        [void]$unitRootSet.Add([IO.Path]::GetFullPath($unitRoot).TrimEnd('\'))
    }
    $units = [System.Collections.Generic.List[object]]::new()
    foreach ($unitRoot in $unitRoots) {
        $unitFull = [IO.Path]::GetFullPath($unitRoot).TrimEnd('\')
        $resolved = Get-CrateManifestDependencyRoots -CrateRoot $unitFull
        $dependencies = [System.Collections.Generic.List[object]]::new()
        foreach ($dependency in @($resolved.dependencies)) {
            $dependencies.Add([ordered]@{
                name = [string]$dependency.name
                package = [string]$dependency.package
                root = [string]$dependency.root
                path_dependency = [bool]$dependency.path_dependency
                # A production unit is a scan subject in its own right, so an
                # edge onto one is an edge between units, not onto a dependency.
                production_unit = $unitRootSet.Contains([string]$dependency.root)
            })
        }
        $units.Add([ordered]@{
            unit = Get-ProductionInteractionRelativePath -Root $repo -Path $unitFull
            unit_root = $unitFull
            manifest = Get-ProductionInteractionRelativePath `
                -Root $repo `
                -Path ([string]$resolved.manifest_path)
            manifest_path = [string]$resolved.manifest_path
            lock_path = [string]$resolved.lock_path
            workspace_root = [string]$resolved.workspace_root
            workspace_cargo_path = [string]$resolved.workspace_cargo_path
            dependencies = @($dependencies)
        })
    }
    return @($units)
}

function Get-ProductionUnexercisedCapabilityKinds {
    # The kinds of evidence that do NOT establish that a crate uses the entry
    # point. It is stated as the excluded set rather than the admitted one so a
    # kind added later is treated as evidence until someone decides otherwise -
    # the opposite default silently drops it.
    #
    # `capability_definition` is a name immediately followed by an assignment,
    # which defines it rather than using it: `libc-0.2.189/src/psp.rs:505` is
    # `SuspendThread = 66`, a variant of a PlayStation-Portable enum. A bare
    # `capability_reference` is NOT in this set - `let suspend = SuspendThread;`
    # followed by `suspend(handle)` is an ordinary indirect call, and a rule
    # that discounted it would suppress exactly the composition this closure
    # exists to see.
    return @('capability_definition')
}

function Test-ProductionCapabilityIsExercised {
    param([Parameter(Mandatory = $true)][AllowEmptyCollection()]$Sites)

    $unexercised = @(Get-ProductionUnexercisedCapabilityKinds)
    return (@(@($Sites) | Where-Object {
        $unexercised -cnotcontains [string]$_.kind
    }).Count -gt 0)
}

function Get-ProductionCrateIdentity {
    # A machine-independent name for a crate root, because it goes into a digest
    # that is pinned in a committed baseline. In-repo crates get their
    # repository-relative path; a registry crate gets its extracted directory
    # name, which is package-version - never the absolute cargo-home path, which
    # differs per machine and per user.
    param(
        [Parameter(Mandatory = $true)][string]$RepoRoot,
        [Parameter(Mandatory = $true)][string]$Root
    )

    $repo = [IO.Path]::GetFullPath($RepoRoot).TrimEnd('\')
    $full = [IO.Path]::GetFullPath($Root).TrimEnd('\')
    if ($full.StartsWith($repo + '\', [StringComparison]::OrdinalIgnoreCase)) {
        return (Get-ProductionInteractionRelativePath -Root $repo -Path $full)
    }
    return (Split-Path -Leaf $full)
}

function Get-ProductionDependencyCapabilityClosure {
    # What a dependency edge actually reaches, not what the crate at the far end
    # of it happens to spell itself.
    #
    # This is transitive on purpose. `inline_detour` stops every other thread
    # and asks MinHook to write the bytes; a lane that names only
    # `inline_detour` still causes live code to be rewritten under a freeze, and
    # a rule that read only the crate named in the manifest would call that lane
    # clean. The migration onto the shared freeze is ongoing, so that is not a
    # hypothetical shape - it is the shape the tree is moving towards.
    #
    # The traversal expands crates inside the repository and stops at crates
    # outside it. A registry crate has no Cargo.lock of its own to resolve its
    # dependencies against, so following its manifest would mean guessing at
    # versions; it is scanned for what its own sources do and treated as a leaf.
    #
    # What a reached crate CONTRIBUTES is decided per half, and the evidence
    # each side of the repository boundary needs is deliberately different:
    #
    #   * A crate in this repository contributes a half on any evidence of it,
    #     a bare reference to the entry point included. Its sources are ours to
    #     read and handing an entry point to something else is as reachable as
    #     calling it, so the conservative reading is the right one.
    #   * A crate outside it contributes a half on every kind of evidence
    #     EXCEPT a definition of the name (`Get-ProductionUnexercisedCapability-
    #     Kinds`), and also whenever the crate does both halves itself, which is
    #     the rule that applied to every dependency before this closure existed,
    #     so this can never admit less than it did. The one thing it excludes is
    #     a name that third-party source DEFINES rather than uses:
    #     `libc-0.2.189/src/psp.rs:505` is `SuspendThread = 66`, a variant of a
    #     PlayStation-Portable enum, and without that exclusion every unit that
    #     transitively depends on `libc` and writes a byte anywhere was reported
    #     as patching live code under a freeze. That was measured, not
    #     predicted.
    #
    # The residual is stated rather than hidden: a registry crate that reaches
    # either half only through a SECOND registry crate is not composed here,
    # because registry manifests are not followed at all. Nothing in the tree
    # has that shape today.
    #
    # ScanCache, when the caller supplies one, holds the crate scans already
    # taken during the caller's own evaluation. This closure is computed once
    # per (crate, lock) pair and each computation re-walks the in-repository
    # crates the other pairs already walked with its own visited set, so one
    # classification enters the scan about ten times per crate. The scan itself
    # is content-addressed, so those repeats no longer re-derive anything - but
    # they do re-read and re-digest the crate to find that out, and on the
    # dependency corpus here that read is 0.75 s and its digest 0.4 s, ten times
    # over. The classification hands in one dictionary for the whole evaluation,
    # so each crate is read exactly once per classification. Which crates a
    # closure REACHES is untouched - that is decided by the lock and the
    # manifests, not by this - so the classification's cross-workspace
    # disagreement check keeps exactly the teeth it had.
    param(
        [Parameter(Mandatory = $true)][string]$RepoRoot,
        [Parameter(Mandatory = $true)][string]$Root,
        [Parameter(Mandatory = $true)][string]$LockPath,
        [Parameter(Mandatory = $true)][string[]]$SuspensionEntryPoints,
        [Parameter(Mandatory = $true)][string[]]$CodeMutationEntryPoints,
        $ScanCache = $null
    )

    $repo = [IO.Path]::GetFullPath($RepoRoot).TrimEnd('\')
    $rootFull = [IO.Path]::GetFullPath($Root).TrimEnd('\')
    $visited = [System.Collections.Generic.HashSet[string]]::new(
        [StringComparer]::OrdinalIgnoreCase
    )
    $pending = [System.Collections.Generic.Queue[string]]::new()
    $pending.Enqueue($rootFull)
    $reached = [System.Collections.Generic.List[object]]::new()
    $suspensionSites = [System.Collections.Generic.List[object]]::new()
    $mutationSites = [System.Collections.Generic.List[object]]::new()
    while ($pending.Count -ne 0) {
        $crateRoot = [IO.Path]::GetFullPath($pending.Dequeue()).TrimEnd('\')
        if (-not $visited.Add($crateRoot)) {
            continue
        }
        if (-not (Test-Path -LiteralPath $crateRoot -PathType Container)) {
            throw "production dependency crate root is missing: $crateRoot"
        }
        $crateId = Get-ProductionCrateIdentity -RepoRoot $repo -Root $crateRoot
        $scan = if ($null -ne $ScanCache -and $ScanCache.Contains($crateRoot)) {
            $ScanCache[$crateRoot]
        }
        else {
            $taken = Get-ProductionCrateSourceCapabilityScan `
                -Root $crateRoot `
                -SuspensionEntryPoints $SuspensionEntryPoints `
                -CodeMutationEntryPoints $CodeMutationEntryPoints
            if ($null -ne $ScanCache) {
                $ScanCache[$crateRoot] = $taken
            }
            $taken
        }
        $inRepo = $crateRoot.StartsWith(
            $repo + '\',
            [StringComparison]::OrdinalIgnoreCase
        )
        # Whether a crate contributes a half, and which halves, is decided per
        # half rather than per crate. `$scan.suspends_and_mutates_code` is kept
        # as one of the ways a crate qualifies so that this can never admit
        # LESS than the rule it replaced.
        $bothHalvesHere = [bool]$scan.suspends_and_mutates_code
        $contributesSuspension = if ($inRepo) {
            [bool]$scan.suspends
        }
        else {
            ($bothHalvesHere -or
                (Test-ProductionCapabilityIsExercised -Sites @($scan.suspension_sites)))
        }
        $contributesMutation = if ($inRepo) {
            [bool]$scan.mutates_code
        }
        else {
            ($bothHalvesHere -or
                (Test-ProductionCapabilityIsExercised -Sites @($scan.code_mutation_sites)))
        }
        $reached.Add([ordered]@{
            root = $crateRoot
            crate = $crateId
            in_repository = $inRepo
            suspends = [bool]$scan.suspends
            mutates_code = [bool]$scan.mutates_code
            contributes_suspension = $contributesSuspension
            contributes_code_mutation = $contributesMutation
        })
        if ($contributesSuspension) {
            foreach ($site in @($scan.suspension_sites)) {
                $suspensionSites.Add([ordered]@{
                    crate = $crateId
                    path = [string]$site.path
                    line = $site.line
                    capability = [string]$site.capability
                    kind = [string]$site.kind
                })
            }
        }
        if ($contributesMutation) {
            foreach ($site in @($scan.code_mutation_sites)) {
                $mutationSites.Add([ordered]@{
                    crate = $crateId
                    path = [string]$site.path
                    line = $site.line
                    capability = [string]$site.capability
                    kind = [string]$site.kind
                })
            }
        }
        if (-not $inRepo) {
            continue
        }
        foreach ($dependency in @(
            (Get-CrateManifestDependencyRoots `
                -CrateRoot $crateRoot `
                -LockPath $LockPath).dependencies
        )) {
            $pending.Enqueue([string]$dependency.root)
        }
    }
    return [ordered]@{
        root = $rootFull
        lock_path = [IO.Path]::GetFullPath($LockPath)
        crate = Get-ProductionCrateIdentity -RepoRoot $repo -Root $rootFull
        reached_crates = @($reached)
        suspension_sites = @($suspensionSites)
        code_mutation_sites = @($mutationSites)
        reaches_suspension = ($suspensionSites.Count -gt 0)
        reaches_code_mutation = ($mutationSites.Count -gt 0)
    }
}

function Get-ProductionDependencyCodeMutationClassification {
    param([Parameter(Mandatory = $true)][string]$RepoRoot)

    $repo = [IO.Path]::GetFullPath($RepoRoot)
    $suspensionEntryPoints = @(Get-ProductionThreadSuspensionEntryPoints)
    $codeMutationEntryPoints = @(Get-ProductionCodeMutationEntryPoints)
    $edges = [System.Collections.Generic.List[object]]::new()
    $scans = [ordered]@{}
    # One crate can be reached by two workspaces - the portable_replay crates
    # are path dependencies of both the extractor and the replay workspace - and
    # each resolves that crate's registry dependencies against its own lock. The
    # closure is therefore computed per (crate, lock) and the results are
    # required to agree, rather than letting whichever unit was enumerated first
    # decide what the crate is measured to do.
    $scansByLock = @{}
    $digests = @{}
    # One crate scan per crate for this whole classification, shared by every
    # (crate, lock) closure below. See Get-ProductionDependencyCapabilityClosure.
    $crateScans = @{}
    $unitClosure = @(Get-ProductionUnitDependencyClosure -RepoRoot $repo)

    # Memoised on a one-pass digest of exactly what the classification reads.
    #
    # This is the single most expensive thing a gate evaluation does and it is
    # invariant across the self-test's source-rejection cases: those mutate a
    # production UNIT source, and the classification reads unit MANIFESTS and
    # DEPENDENCY sources. 528 evaluations therefore asked the same question 528
    # times. Measured here: 3.4 s per evaluation, against 1.05 s for the whole
    # authority walk.
    #
    # The key is built from one pass over those inputs, so key and value
    # describe the same snapshot: the resolved unit closure (which manifest,
    # lock and workspace manifest each unit resolved to, and their exact text),
    # every dependency root that closure names with the content digest of every
    # source the crate scan would admit under it, both entry-point rosters, and
    # the rules digest. Anything that can change a classification is in there;
    # a stale answer would need one of those bytes to have changed without its
    # digest changing.
    $classificationKeySha = [Security.Cryptography.SHA256]::Create()
    try {
        $classificationPreimage = [Text.StringBuilder]::new()
        [void]$classificationPreimage.Append($repo).Append("`n")
        [void]$classificationPreimage.Append($suspensionEntryPoints -join "`0").Append("`n")
        [void]$classificationPreimage.Append($codeMutationEntryPoints -join "`0").Append("`n")
        [void]$classificationPreimage.Append($script:ProductionInteractionRulesDigest).Append("`n")
        $manifestInputs = [System.Collections.Generic.List[string]]::new()
        $dependencyRootsForKey = [System.Collections.Generic.List[string]]::new()
        foreach ($unit in $unitClosure) {
            [void]$classificationPreimage.Append('unit ').Append([string]$unit.unit_root).Append("`n")
            foreach ($manifestInput in @(
                [string]$unit.manifest_path,
                [string]$unit.lock_path,
                [string]$unit.workspace_cargo_path
            )) {
                if (-not [string]::IsNullOrEmpty($manifestInput) -and
                    -not $manifestInputs.Contains($manifestInput)) {
                    $manifestInputs.Add($manifestInput)
                }
            }
            foreach ($dependency in @($unit.dependencies)) {
                if ($dependency.production_unit) { continue }
                $dependencyRoot = [string]$dependency.root
                [void]$classificationPreimage.Append('edge ').
                    Append([string]$unit.unit_root).Append("`0").
                    Append([string]$dependency.name).Append("`0").
                    Append([string]$dependency.package).Append("`0").
                    Append($dependencyRoot).Append("`0").
                    Append([string]$unit.lock_path).Append("`n")
                if (-not $dependencyRootsForKey.Contains($dependencyRoot)) {
                    $dependencyRootsForKey.Add($dependencyRoot)
                }
            }
        }
        foreach ($manifestInput in $manifestInputs) {
            [void]$classificationPreimage.Append('manifest ').Append($manifestInput).Append("`0")
            if (Test-Path -LiteralPath $manifestInput -PathType Leaf) {
                [void]$classificationPreimage.Append([BitConverter]::ToString(
                    $classificationKeySha.ComputeHash([IO.File]::ReadAllBytes($manifestInput))
                ).Replace('-', ''))
            }
            else {
                [void]$classificationPreimage.Append('absent')
            }
            [void]$classificationPreimage.Append("`n")
        }
        # The reachability closure expands in-repository crates through their
        # OWN manifests, so a crate no production unit names directly is still
        # read - `recorder_host` names `wire_product`, whose manifest names
        # `wire_format`, and a suspension entry point appearing in
        # wire_format/src/lib.rs changes the classification. Digesting only the
        # directly named roots would have left that change behind a warm memo.
        # The key therefore walks the same edges the classification walks, per
        # (root, lock) pair, and digests every crate it reaches - manifest
        # included, because a manifest edit is what moves the walk itself.
        #
        # This walk reads manifests only, never sources, and every manifest
        # parse below it is memoised on the manifest text, so discovering the
        # reached set costs far less than scanning it.
        $keySourceExtensions = @(Get-ProductionInteractionSourceExtensions)
        $reachedForKey = [System.Collections.Specialized.OrderedDictionary]::new(
            [StringComparer]::OrdinalIgnoreCase
        )
        foreach ($unit in $unitClosure) {
            $unitLockForKey = [string]$unit.lock_path
            foreach ($dependency in @($unit.dependencies)) {
                if ($dependency.production_unit) { continue }
                $pendingForKey = [System.Collections.Generic.Queue[string]]::new()
                $pendingForKey.Enqueue([string]$dependency.root)
                $visitedForKey = [System.Collections.Generic.HashSet[string]]::new(
                    [StringComparer]::OrdinalIgnoreCase
                )
                while ($pendingForKey.Count -ne 0) {
                    $reachedRoot = [IO.Path]::GetFullPath($pendingForKey.Dequeue()).TrimEnd('\')
                    if (-not $visitedForKey.Add($reachedRoot)) { continue }
                    if (-not $reachedForKey.Contains($reachedRoot)) {
                        $reachedForKey[$reachedRoot] = $true
                    }
                    if (-not $reachedRoot.StartsWith(
                            $repo + '\', [StringComparison]::OrdinalIgnoreCase)) {
                        # Registry crates are leaves to the closure: it does not
                        # follow their manifests, so neither does this.
                        continue
                    }
                    if (-not (Test-Path -LiteralPath $reachedRoot -PathType Container)) { continue }
                    foreach ($nested in @(
                        (Get-CrateManifestDependencyRoots `
                            -CrateRoot $reachedRoot `
                            -LockPath $unitLockForKey).dependencies
                    )) {
                        $pendingForKey.Enqueue([string]$nested.root)
                    }
                }
            }
        }
        foreach ($dependencyRoot in @($reachedForKey.Keys)) {
            [void]$classificationPreimage.Append('root ').Append($dependencyRoot).Append("`n")
            if (-not (Test-Path -LiteralPath $dependencyRoot -PathType Container)) {
                [void]$classificationPreimage.Append("absent`n")
                continue
            }
            $reachedManifest = Join-Path $dependencyRoot 'Cargo.toml'
            [void]$classificationPreimage.Append('root-manifest ').Append($reachedManifest).Append("`0")
            if (Test-Path -LiteralPath $reachedManifest -PathType Leaf) {
                [void]$classificationPreimage.Append([BitConverter]::ToString(
                    $classificationKeySha.ComputeHash([IO.File]::ReadAllBytes($reachedManifest))
                ).Replace('-', ''))
            }
            else {
                [void]$classificationPreimage.Append('absent')
            }
            [void]$classificationPreimage.Append("`n")
            foreach ($entry in ([IO.DirectoryInfo]$dependencyRoot).EnumerateFiles(
                '*', [IO.SearchOption]::AllDirectories)) {
                if ($keySourceExtensions -notcontains $entry.Extension) { continue }
                [void]$classificationPreimage.Append($entry.FullName).Append("`0").
                    Append([BitConverter]::ToString(
                        $classificationKeySha.ComputeHash([IO.File]::ReadAllBytes($entry.FullName))
                    ).Replace('-', '')).Append("`n")
            }
        }
        foreach ($unitRootForKey in @($unitClosure | ForEach-Object { [string]$_.unit_root })) {
            [void]$classificationPreimage.Append('unit-root ').Append($unitRootForKey).Append("`n")
        }
        $classificationKey = 'dependency-code-mutation-classification:' +
            (Get-ProductionInteractionContentKey -Text $classificationPreimage.ToString())
    }
    finally {
        $classificationKeySha.Dispose()
    }
    if ($script:ProductionInteractionDerivationCache.Contains($classificationKey)) {
        return $script:ProductionInteractionDerivationCache[$classificationKey]
    }

    foreach ($unit in $unitClosure) {
        $unitLockPath = [string]$unit.lock_path
        foreach ($dependency in @($unit.dependencies)) {
            if ($dependency.production_unit) {
                continue
            }
            $dependencyRoot = [string]$dependency.root
            $scanKey = "$dependencyRoot`0$unitLockPath"
            if (-not $scansByLock.ContainsKey($scanKey)) {
                $scansByLock[$scanKey] = Get-ProductionDependencyCapabilityClosure `
                    -RepoRoot $repo `
                    -Root $dependencyRoot `
                    -LockPath $unitLockPath `
                    -SuspensionEntryPoints $suspensionEntryPoints `
                    -CodeMutationEntryPoints $codeMutationEntryPoints `
                    -ScanCache $crateScans
            }
            $scan = $scansByLock[$scanKey]
            $digest = Get-ProductionCapabilityEvidenceDigest -Scan $scan
            if (-not $scans.Contains($dependencyRoot)) {
                $scans[$dependencyRoot] = $scan
                $digests[$dependencyRoot] = $digest
            }
            elseif ([string]$digests[$dependencyRoot] -cne $digest) {
                throw (
                    'production dependency measures two different capabilities ' +
                    "depending on which workspace resolves it: $dependencyRoot"
                )
            }
            $edges.Add([ordered]@{
                unit = [string]$unit.unit
                unit_root = [string]$unit.unit_root
                manifest = [string]$unit.manifest
                dependency = [string]$dependency.name
                package = [string]$dependency.package
                dependency_root = $dependencyRoot
            })
        }
    }
    # Either half is classified, not only the conjunction. A dependency that
    # only stops threads is still what a dependent's own VirtualProtect runs
    # under, and requiring one crate to spell both halves is what made four
    # lanes disappear from this gate the day their suspender moved one crate
    # away and their code mutation stayed put.
    $classified = [ordered]@{}
    foreach ($key in @($scans.Keys)) {
        if ($scans[$key].reaches_suspension -or $scans[$key].reaches_code_mutation) {
            $classified[$key] = $scans[$key]
        }
    }
    # The digest is over what the crate was measured to do, not over its name or
    # its version, so a patcher swapped for a different crate - or the same crate
    # grown a new capability - moves the digest and has to be accepted again.
    foreach ($key in @($classified.Keys)) {
        $classified[$key]['capability_digest'] =
            Get-ProductionCapabilityEvidenceDigest -Scan $classified[$key]
    }
    $classification = [ordered]@{
        schema = 'hoi4.production-dependency-code-mutation-classification.v1'
        scanned_dependency_count = $scans.Count
        classified_patchers = $classified
        edges = @($edges)
    }
    # Read-only to every consumer: the findings walk reads edges and
    # classified_patchers and never writes back, and the capability digest each
    # classified patcher carries was stamped on above, before this was stored.
    Set-ProductionInteractionDerivation `
        -Key $classificationKey `
        -Value $classification `
        -Weight (4096 + 256 * (@($edges).Count + $classified.Count))
    return $classification
}

function Get-OrdinallySortedKeys {
    param([Parameter(Mandatory = $true)][AllowEmptyCollection()]$Keys)

    $ordered = [string[]]@(@($Keys) | ForEach-Object { [string]$_ })
    if ($ordered.Length -gt 1) {
        [Array]::Sort($ordered, [StringComparer]::Ordinal)
    }
    return $ordered
}

function Get-ProductionCapabilityEvidenceDigest {
    param([Parameter(Mandatory = $true)]$Scan)

    # Same key discipline as the unit baseline: file, capability and kind with a
    # count, never a line number, so the digest survives a reformat of the
    # dependency and moves when its behaviour does. The crate the file belongs
    # to is part of the key because the evidence now spans a closure: two crates
    # in it can both have a src/lib.rs, and without the crate the digest would
    # collapse them and stop moving when one of them changed.
    $counts = [System.Collections.Specialized.OrderedDictionary]::new([StringComparer]::Ordinal)
    foreach ($site in (@($Scan.suspension_sites) + @($Scan.code_mutation_sites))) {
        $key = "$($site.crate)|$($site.path)|$($site.capability)|$($site.kind)"
        if (-not $counts.Contains($key)) {
            $counts[$key] = 0
        }
        $counts[$key] = [int]$counts[$key] + 1
    }
    # Sort-Object is case-insensitive in Windows PowerShell 5.1, so it would
    # order two ordinal-distinct keys arbitrarily and the digest of unchanged
    # evidence could move with enumeration order. The sort has to be ordinal for
    # the same reason the dictionaries are.
    $canonical = (@(Get-OrdinallySortedKeys -Keys $counts.Keys | ForEach-Object {
        "$_ x$($counts[$_])"
    })) -join "`n"
    $sha = [Security.Cryptography.SHA256]::Create()
    try {
        $bytes = $sha.ComputeHash([Text.Encoding]::UTF8.GetBytes($canonical))
    }
    finally {
        $sha.Dispose()
    }
    return (($bytes | ForEach-Object { $_.ToString('x2') }) -join '')
}

function Get-ProductionSuspendingCodeMutationFindings {
    param(
        [Parameter(Mandatory = $true)][string]$RepoRoot,
        [Parameter(Mandatory = $true)]$Authority
    )

    Enter-ProductionInteractionEvaluation
    try {

    $repo = [IO.Path]::GetFullPath($RepoRoot)
    $earlyRenderEvidencePath =
        Join-Path $repo 'extractor\native\recorder_host\src\early_render_launch.rs'
    $suspensionEntryPoints = @(Get-ProductionThreadSuspensionEntryPoints)
    $codeMutationEntryPoints = @(Get-ProductionCodeMutationEntryPoints)
    $unitRoots = @(
        Get-ProductionInteractionSourceRoots -RepoRoot $repo |
            ForEach-Object { [IO.Path]::GetFullPath($_).TrimEnd('\') } |
            Sort-Object { $_.Length } -Descending
    )
    $classification = Get-ProductionDependencyCodeMutationClassification -RepoRoot $repo

    $units = [ordered]@{}
    foreach ($unitRoot in $unitRoots) {
        $units[$unitRoot] = [ordered]@{
            unit = Get-ProductionInteractionRelativePath -Root $repo -Path $unitRoot
            suspension = New-ProductionCapabilityAccumulator
            mutation = New-ProductionCapabilityAccumulator
            suspension_sites = @()
            code_mutation_sites = @()
            delegation_sites = [System.Collections.Generic.List[object]]::new()
        }
    }

    # A crate that names a classified patcher through a re-export never has to
    # write the crate's own path, so the manifest edge - not the callsite text -
    # is what binds a unit to a delegated patcher.
    $delegatedCrates = [ordered]@{}
    foreach ($edge in @($classification.edges)) {
        if (-not $classification.classified_patchers.Contains([string]$edge.dependency_root)) {
            continue
        }
        $unitRoot = [string]$edge.unit_root
        if (-not $delegatedCrates.Contains($unitRoot)) {
            $delegatedCrates[$unitRoot] = [System.Collections.Generic.List[object]]::new()
        }
        $delegatedCrates[$unitRoot].Add($edge)
    }

    foreach ($path in @($Authority.files)) {
        $fullPath = [IO.Path]::GetFullPath($path)
        $owningRoot = @($unitRoots | Where-Object {
            $fullPath.StartsWith($_ + '\', [StringComparison]::OrdinalIgnoreCase)
        } | Select-Object -First 1)
        if ($owningRoot.Count -ne 1) {
            throw "production interaction source file has no owning unit root: $fullPath"
        }
        $unitRoot = [string]$owningRoot[0]
        $relative = Get-ProductionInteractionRelativePath -Root $repo -Path $fullPath
        # One file's contribution to both halves, keyed on the digest the
        # authority walk already took of its bytes. Everything inside is a pure
        # function of those bytes and of the two entry-point rosters, and both
        # rosters are in the key. A file that did not change is not reopened.
        $fileEvidence = Get-ProductionInteractionFileDerivation `
            -Kind 'suspending-code-mutation-file-evidence' `
            -Path $fullPath `
            -FileSha256 (Get-ProductionInteractionAuthorityFileDigest `
                -Authority $Authority -Path $path) `
            -Discriminator (@(
                $relative,
                ($suspensionEntryPoints -join "`0"),
                ($codeMutationEntryPoints -join "`0"),
                [IO.Path]::GetFullPath($earlyRenderEvidencePath)
            ) -join "`n") `
            -AuthorityWalkId (Get-ProductionInteractionAuthorityWalkId -Authority $Authority) `
            -Weight 2048 `
            -Compute {
                $source = Get-ProductionInteractionScannableSource `
                    -Path $fullPath `
                    -EarlyRenderEvidencePath $earlyRenderEvidencePath
                $code = Get-RustCodeWithoutCommentsOrStrings -Text $source
                $cFamily = @('.c', '.h', '.cc', '.cpp') -contains
                    [IO.Path]::GetExtension($fullPath)
                [pscustomobject]@{
                    # The stripped code travels with the evidence because the
                    # delegation scan below needs the same text, and which
                    # crates it looks for comes from the classification rather
                    # than from this file - so that scan stays outside the memo
                    # and is handed the text instead of re-deriving it.
                    Code = $code
                    Suspension = Get-ProductionCapabilityEvidence `
                        -Source $source `
                        -Code $code `
                        -EntryPoints $suspensionEntryPoints `
                        -Path $relative `
                        -CFamily:$cFamily
                    Mutation = Get-ProductionCapabilityEvidence `
                        -Source $source `
                        -Code $code `
                        -EntryPoints $codeMutationEntryPoints `
                        -Path $relative `
                        -CFamily:$cFamily
                }
            }
        Add-ProductionCapabilityEvidence `
            -Accumulator $units[$unitRoot].suspension `
            -Evidence $fileEvidence.Suspension
        Add-ProductionCapabilityEvidence `
            -Accumulator $units[$unitRoot].mutation `
            -Evidence $fileEvidence.Mutation
        if ($delegatedCrates.Contains($unitRoot)) {
            foreach ($edge in @($delegatedCrates[$unitRoot])) {
                $crateRootIdent = ([string]$edge.dependency) -replace '-', '_'
                $pattern =
                    "(?<![A-Za-z0-9_])$([regex]::Escape($crateRootIdent))\s*::"
                foreach ($match in [regex]::Matches($fileEvidence.Code, $pattern)) {
                    $units[$unitRoot].delegation_sites.Add([ordered]@{
                        path = $relative
                        line = Get-ProductionSourceLineNumber `
                            -Text $fileEvidence.Code -Index $match.Index
                        capability = 'delegated_suspending_code_mutation'
                        token = $crateRootIdent
                        kind = 'external_patcher_use'
                    })
                }
            }
        }
    }

    $nonConforming = [System.Collections.Generic.List[object]]::new()
    foreach ($unitRoot in @($units.Keys)) {
        $unit = $units[$unitRoot]
        # An export resolver in any file of the unit arms the literal evidence
        # for the whole unit, so a shared resolver in a sibling file does not
        # hide a run-time lookup.
        $resolvesExports =
            ($unit.suspension.resolves_exports -or $unit.mutation.resolves_exports)
        $unit.suspension.resolves_exports = $resolvesExports
        $unit.mutation.resolves_exports = $resolvesExports
        $unit.suspension_sites = @(
            Resolve-ProductionCapabilitySites -Accumulator $unit.suspension
        )
        $unit.code_mutation_sites = @(
            Resolve-ProductionCapabilitySites -Accumulator $unit.mutation
        )
        $directlySuspendsAndMutates =
            (@($unit.suspension_sites).Count -gt 0 -and
                @($unit.code_mutation_sites).Count -gt 0)
        $delegates = $delegatedCrates.Contains($unitRoot)
        # The two halves are composed across the unit/dependency boundary, not
        # required to sit on one side of it. A unit that writes the bytes itself
        # and takes its freeze from a dependency performs exactly the operation
        # this gate exists to inventory, and reading each side alone reports
        # nothing.
        $reachesSuspension = (@($unit.suspension_sites).Count -gt 0)
        $reachesCodeMutation = (@($unit.code_mutation_sites).Count -gt 0)
        if ($delegates) {
            foreach ($edge in @($delegatedCrates[$unitRoot])) {
                $patcher = $classification.classified_patchers[[string]$edge.dependency_root]
                if ($patcher.reaches_suspension) { $reachesSuspension = $true }
                if ($patcher.reaches_code_mutation) { $reachesCodeMutation = $true }
            }
        }
        if (-not ($reachesSuspension -and $reachesCodeMutation)) {
            continue
        }
        $nonConforming.Add([ordered]@{
            unit = [string]$unit.unit
            unit_root = $unitRoot
            performs_suspending_code_mutation = $directlySuspendsAndMutates
            delegates_suspending_code_mutation = $delegates
            delegated_patchers = @(
                if ($delegates) {
                    $delegatedCrates[$unitRoot] | ForEach-Object {
                        [ordered]@{
                            dependency = [string]$_.dependency
                            package = [string]$_.package
                            manifest = [string]$_.manifest
                            dependency_root = [string]$_.dependency_root
                            reaches_suspension = [bool]$classification.classified_patchers[
                                [string]$_.dependency_root
                            ].reaches_suspension
                            reaches_code_mutation = [bool]$classification.classified_patchers[
                                [string]$_.dependency_root
                            ].reaches_code_mutation
                            capability_digest = [string]$classification.classified_patchers[
                                [string]$_.dependency_root
                            ].capability_digest
                        }
                    }
                }
            )
            # The unit's own sites are reported whenever the unit is reported,
            # not only when the unit spells both halves itself. Gating them on
            # the conjunction is what erased the four lanes' VirtualProtect
            # evidence from the pin at the same moment their suspender moved
            # into a shared crate: the calls were still there, and the accepted
            # inventory stopped naming them.
            suspension_sites = @($unit.suspension_sites)
            code_mutation_sites = @($unit.code_mutation_sites)
            delegation_sites = @($unit.delegation_sites)
        })
    }

    return [ordered]@{
        schema = 'hoi4.production-suspending-code-mutation-findings.v1'
        thread_suspension_entry_points = $suspensionEntryPoints
        code_mutation_entry_points = $codeMutationEntryPoints
        scanned_file_count = @($Authority.files).Count
        scanned_dependency_count = [int]$classification.scanned_dependency_count
        classified_external_patchers = @(
            $classification.classified_patchers.Keys | ForEach-Object {
                [ordered]@{
                    root = [string]$_
                    reaches_suspension =
                        [bool]$classification.classified_patchers[$_].reaches_suspension
                    reaches_code_mutation =
                        [bool]$classification.classified_patchers[$_].reaches_code_mutation
                    reached_crates =
                        @($classification.classified_patchers[$_].reached_crates)
                    suspension_sites =
                        @($classification.classified_patchers[$_].suspension_sites)
                    code_mutation_sites =
                        @($classification.classified_patchers[$_].code_mutation_sites)
                }
            }
        )
        non_conforming_units = @(
            $nonConforming | Sort-Object { [string]$_.unit }
        )
        no_suspending_code_mutation = ($nonConforming.Count -eq 0)
    }

    }
    finally {
        Exit-ProductionInteractionEvaluation
    }
}

function Format-ProductionSuspendingCodeMutationFindings {
    param([Parameter(Mandatory = $true)]$Findings)

    $lines = [System.Collections.Generic.List[string]]::new()
    $lines.Add(
        'production source performs thread-suspending code mutation in ' +
        "$(@($Findings.non_conforming_units).Count) unit(s)."
    )
    $lines.Add(
        'Rule: production code must not rewrite live executable bytes of the ' +
        'target process while stopping that process''s other threads, no ' +
        'matter which primitive performs it. Note that a suspend attempt is ' +
        'not a guarantee: MinHook''s Freeze marks a thread unsuspended when ' +
        'SuspendThread fails and patches anyway (minhook hook.c Freeze), so ' +
        'delegating does not make the mutation safer than doing it here.'
    )
    foreach ($patcher in @($Findings.classified_external_patchers)) {
        $halves = @(
            if ($patcher.reaches_suspension) { 'suspends threads' }
            if ($patcher.reaches_code_mutation) { 'mutates live code' }
        ) -join ' and '
        $lines.Add("  external primitive classified as a suspending code patcher: $($patcher.root) [$halves]")
        foreach ($site in @($patcher.suspension_sites)) {
            $lines.Add("      suspends: $($site.crate)/$($site.path):$($site.line) $($site.capability)")
        }
        foreach ($site in @($patcher.code_mutation_sites)) {
            $lines.Add("      mutates code: $($site.crate)/$($site.path):$($site.line) $($site.capability)")
        }
    }
    foreach ($unit in @($Findings.non_conforming_units)) {
        $lines.Add("  unit $($unit.unit):")
        foreach ($site in @($unit.suspension_sites)) {
            $lines.Add(
                "      suspends threads: $($site.path):$($site.line) " +
                "$($site.capability) [$($site.kind)]"
            )
        }
        foreach ($site in @($unit.code_mutation_sites)) {
            $lines.Add(
                "      mutates live code: $($site.path):$($site.line) " +
                "$($site.capability) [$($site.kind)]"
            )
        }
        if ($unit.delegates_suspending_code_mutation) {
            foreach ($patcher in @($unit.delegated_patchers)) {
                $patcherHalves = @(
                    if ($patcher.reaches_suspension) { 'suspends threads' }
                    if ($patcher.reaches_code_mutation) { 'mutates live code' }
                ) -join ' and '
                $lines.Add(
                    "      delegates to suspending code patcher " +
                    "'$($patcher.dependency)' [$patcherHalves] declared in $($patcher.manifest)"
                )
            }
            foreach ($site in @($unit.delegation_sites)) {
                $lines.Add("      reaches it at: $($site.path):$($site.line)")
            }
        }
    }
    return ($lines -join [Environment]::NewLine)
}

function Get-ProductionSuspendingCodeMutationBaselinePath {
    param([Parameter(Mandatory = $true)][string]$RepoRoot)

    return (Join-Path ([IO.Path]::GetFullPath($RepoRoot)) `
        'extractor\native\scripts\ProductionSuspendingCodeMutation.baseline.v1.json')
}

function Get-ProductionSuspendingCodeMutationBaseline {
    param([Parameter(Mandatory = $true)]$Findings)

    # What the baseline keys on decides what drift it can see. Line numbers move
    # on any edit above a site, so keying on them would turn a reformat into a
    # gate failure and train the reader to re-pin without looking. Statement text
    # would go red on a variable rename for the same reason. The key is therefore
    # the multiset of (file, capability, kind) with a count per unit: a new
    # patcher anywhere, a unit that starts suspending, a site that moves to
    # another file, or a site that disappears all change it, while a reformat or
    # an inserted function above a site does not. What it cannot see is one
    # suspending call being swapped for another of the same capability and kind
    # inside the same file, or a site moving within its own file. Line numbers are
    # still reported, for navigation, and are not part of the key.
    $units = [System.Collections.Generic.List[object]]::new()
    foreach ($unit in @($Findings.non_conforming_units)) {
        $evidence = [System.Collections.Specialized.OrderedDictionary]::new([StringComparer]::Ordinal)
        foreach ($site in (@($unit.suspension_sites) + @($unit.code_mutation_sites))) {
            $key = "$($site.path)|$($site.capability)|$($site.kind)"
            if (-not $evidence.Contains($key)) {
                $evidence[$key] = 0
            }
            $evidence[$key] = [int]$evidence[$key] + 1
        }
        $directEvidence = [System.Collections.Generic.List[object]]::new()
        foreach ($key in (Get-OrdinallySortedKeys -Keys $evidence.Keys)) {
            $parts = ([string]$key) -split '\|'
            $directEvidence.Add([ordered]@{
                file = $parts[0]
                capability = $parts[1]
                kind = $parts[2]
                count = [int]$evidence[$key]
            })
        }
        $delegated = [System.Collections.Generic.List[object]]::new()
        foreach ($patcher in (@($unit.delegated_patchers) |
                Sort-Object { "$($_.dependency)|$($_.package)" })) {
            $delegated.Add([ordered]@{
                # Both spellings are pinned: the classification stays measured,
                # but which crate a unit declares is part of the inventory a
                # human accepted, so re-spelling it through `package =` is a
                # change to that inventory and has to be accepted again.
                dependency = [string]$patcher.dependency
                package = [string]$patcher.package
                # Which half of the operation the edge contributes is part of
                # the accepted inventory: a dependency that stops threads today
                # and starts writing code tomorrow - or the reverse - has
                # changed what the unit reaches through it, and that has to be
                # read by a human again rather than absorbed silently.
                reaches_suspension = [bool]$patcher.reaches_suspension
                reaches_code_mutation = [bool]$patcher.reaches_code_mutation
                capability_digest = [string]$patcher.capability_digest
            })
        }
        $units.Add([ordered]@{
            unit = [string]$unit.unit
            performs_suspending_code_mutation =
                [bool]$unit.performs_suspending_code_mutation
            delegates_suspending_code_mutation =
                [bool]$unit.delegates_suspending_code_mutation
            direct_evidence = @($directEvidence)
            delegated_patchers = @($delegated)
        })
    }
    return [ordered]@{
        schema = 'hoi4.production-suspending-code-mutation-baseline.v1'
        units = @($units | Sort-Object { $_.unit })
    }
}

function Get-ProductionSuspendingCodeMutationBaselineDefects {
    param([Parameter(Mandatory = $true)][AllowNull()]$Pinned)

    # The pin is the accepted inventory, so it is read as evidence and has to be
    # well formed before it is compared. Without this, PowerShell's coercions
    # make a hand-edited pin agree with anything: [bool] on the string "false"
    # is true, [bool] on null is false, [int] on true is 1, and a duplicated key
    # silently keeps whichever entry was written last.
    $defects = [System.Collections.Generic.List[string]]::new()
    if ($null -eq $Pinned) {
        $defects.Add('baseline is null')
        return @($defects)
    }
    if ($Pinned -is [Array]) {
        $defects.Add('baseline is an array, expected an object')
        return @($defects)
    }
    # The schema is compared as a string, so it has to be one: a one-element
    # array holding the right text would otherwise coerce to it and pass.
    $schema = $Pinned.PSObject.Properties['schema']
    if ($null -eq $schema -or -not ($schema.Value -is [string]) -or
        [string]::IsNullOrWhiteSpace([string]$schema.Value)) {
        $defects.Add('baseline schema is missing or is not a string')
    }
    $units = $Pinned.PSObject.Properties['units']
    if ($null -eq $units -or $null -eq $units.Value) {
        $defects.Add('baseline has no units')
        return @($defects)
    }
    if (-not ($units.Value -is [Array])) {
        $defects.Add('baseline units is not an array')
        return @($defects)
    }
    $seenUnits = [System.Collections.Generic.HashSet[string]]::new(
        [StringComparer]::Ordinal
    )
    foreach ($unit in @($units.Value)) {
        if ($null -eq $unit -or $unit -is [Array] -or $unit -is [string]) {
            $defects.Add('baseline unit is not an object')
            continue
        }
        $name = $unit.PSObject.Properties['unit']
        if ($null -eq $name -or -not ($name.Value -is [string]) -or
            [string]::IsNullOrWhiteSpace([string]$name.Value)) {
            $defects.Add('baseline unit has no name')
            continue
        }
        $unitName = [string]$name.Value
        if (-not $seenUnits.Add($unitName)) {
            $defects.Add("baseline lists unit more than once: $unitName")
        }
        foreach ($flag in @(
            'performs_suspending_code_mutation',
            'delegates_suspending_code_mutation'
        )) {
            $property = $unit.PSObject.Properties[$flag]
            if ($null -eq $property -or -not ($property.Value -is [bool])) {
                $defects.Add("$unitName : $flag is not a boolean")
            }
        }
        $seenEvidence = [System.Collections.Generic.HashSet[string]]::new(
            [StringComparer]::Ordinal
        )
        # Each collection is required to BE a collection. A scalar object where
        # an array belongs still enumerates as one element, so it would compare
        # equal while the pin no longer says what it appears to say.
        $evidenceProperty = $unit.PSObject.Properties['direct_evidence']
        if ($null -eq $evidenceProperty -or $null -eq $evidenceProperty.Value -or
            -not ($evidenceProperty.Value -is [Array])) {
            $defects.Add("$unitName : direct_evidence is missing or is not an array")
            continue
        }
        foreach ($entry in @($evidenceProperty.Value)) {
            if ($null -eq $entry -or $entry -is [Array] -or $entry -is [string]) {
                $defects.Add("$unitName : evidence entry is not an object")
                continue
            }
            $fields = @('file', 'capability', 'kind')
            $missing = $false
            foreach ($field in $fields) {
                $property = $entry.PSObject.Properties[$field]
                if ($null -eq $property -or -not ($property.Value -is [string]) -or
                    [string]::IsNullOrWhiteSpace([string]$property.Value)) {
                    $defects.Add("$unitName : evidence entry has no $field")
                    $missing = $true
                }
            }
            $count = $entry.PSObject.Properties['count']
            if ($null -eq $count -or -not ($count.Value -is [int]) -or
                [int]$count.Value -lt 1) {
                $defects.Add("$unitName : evidence entry has a non-positive count")
                $missing = $true
            }
            if ($missing) { continue }
            $key = "$($entry.file)|$($entry.capability)|$($entry.kind)"
            if (-not $seenEvidence.Add($key)) {
                $defects.Add("$unitName : evidence entry appears more than once: $key")
            }
        }
        $seenPatchers = [System.Collections.Generic.HashSet[string]]::new(
            [StringComparer]::Ordinal
        )
        $patcherProperty = $unit.PSObject.Properties['delegated_patchers']
        if ($null -eq $patcherProperty -or $null -eq $patcherProperty.Value -or
            -not ($patcherProperty.Value -is [Array])) {
            $defects.Add("$unitName : delegated_patchers is missing or is not an array")
            continue
        }
        foreach ($entry in @($patcherProperty.Value)) {
            if ($null -eq $entry -or $entry -is [Array] -or $entry -is [string]) {
                $defects.Add("$unitName : delegated patcher is not an object")
                continue
            }
            $missing = $false
            foreach ($field in @('dependency', 'package', 'capability_digest')) {
                $property = $entry.PSObject.Properties[$field]
                if ($null -eq $property -or -not ($property.Value -is [string]) -or
                    [string]::IsNullOrWhiteSpace([string]$property.Value)) {
                    $defects.Add("$unitName : delegated patcher has no $field")
                    $missing = $true
                }
            }
            # These two are compared with [bool], and [bool] on the STRING
            # "false" is $true - so is [bool] on "garbage". A pin that spelled
            # either of them as a string would agree with a measured $true and
            # read, to a human, as the opposite of what the gate was comparing.
            # They have to be real booleans in the file, for the same reason the
            # unit-level flags are.
            foreach ($field in @('reaches_suspension', 'reaches_code_mutation')) {
                $property = $entry.PSObject.Properties[$field]
                if ($null -eq $property -or -not ($property.Value -is [bool])) {
                    $defects.Add(
                        "$unitName : delegated patcher $field is missing or is not a boolean"
                    )
                    $missing = $true
                }
            }
            if ($missing) { continue }
            $key = "$($entry.dependency)|$($entry.package)|$($entry.capability_digest)"
            if (-not $seenPatchers.Add($key)) {
                $defects.Add("$unitName : delegated patcher appears more than once: $key")
            }
        }
    }
    return @($defects)
}

function Read-ProductionSuspendingCodeMutationBaseline {
    param([Parameter(Mandatory = $true)][string]$Path)

    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "accepted suspending code mutation baseline is missing: $Path"
    }
    # Parsing is made terminating here rather than relying on the caller's
    # preference, so a malformed pin cannot leave an empty difference list and
    # read as agreement.
    try {
        $raw = Get-Content -LiteralPath $Path -Raw -ErrorAction Stop
        $pinned = $raw | ConvertFrom-Json -ErrorAction Stop
    }
    catch {
        throw (
            "accepted suspending code mutation baseline is unreadable: $Path - " +
            $_.Exception.Message
        )
    }
    $defects = @(Get-ProductionSuspendingCodeMutationBaselineDefects -Pinned $pinned)
    if ($defects.Count -gt 0) {
        throw (
            @("accepted suspending code mutation baseline is malformed: $Path") +
            @($defects | ForEach-Object { "  $_" })
        ) -join [Environment]::NewLine
    }
    return $pinned
}

function Get-ProductionSuspendingCodeMutationRepinInstruction {
    param([Parameter(Mandatory = $true)][string]$RepoRoot)

    # Named, runnable, and derived from the repo root the caller is actually in,
    # so a worktree lane is not told to repin the main checkout.
    #
    # Why this exists. Lane GR's purge removed two pinned hook sites in
    # land_combat_detail_events_live without regenerating the baseline, and the
    # only thing the gate said was that a case called 'thread-suspending code
    # mutation matches the accepted baseline' had failed. Every -Mode Final and
    # -Mode Release on the tree then stopped at that gate, for everyone, and the
    # route back was to hand-edit a 1,600-line JSON pin. A gate that can be put
    # into a state nobody is told how to leave is a gate that gets deleted, so
    # the drift now names the command that regenerates the pin.
    #
    # This is NOT an invitation to repin without reading. The switch prints
    # every difference it is accepting and refuses to run while any other
    # closure case is failing; accepting the inventory stays a human review
    # decision, exactly as the roster comment in the test file says.
    $script = Join-Path ([IO.Path]::GetFullPath($RepoRoot)) `
        'extractor\native\scripts\Test-ProductionInteractionClosure.ps1'
    # The path is QUOTED. It is interpolated from the caller's repo root, and a
    # checkout under a directory with a space ("C:\HOI4 Project\...") otherwise
    # produces a command PowerShell splits at the space - a named remediation
    # that is not runnable is not a remediation.
    return (
        'To accept the measured inventory, READ the differences above and then run: ' +
        "powershell.exe -NoProfile -ExecutionPolicy Bypass -File `"$script`" " +
        '-AcceptMeasuredSuspendingBaseline'
    )
}

function Write-ProductionSuspendingCodeMutationBaseline {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)]$Measured
    )

    # Written through the same Get-ProductionSuspendingCodeMutationBaseline
    # shape the comparison reads, so the regenerator and the gate can never
    # disagree about the format: there is one producer of this document.
    #
    # Depth 6 is not arbitrary. The document is
    # baseline -> units[] -> direct_evidence[]/delegated_patchers[] -> fields,
    # and ConvertTo-Json silently replaces anything deeper than -Depth with the
    # type name, which would write a pin that reads back as agreement with
    # nothing. Round-tripped below rather than trusted.
    $json = ($Measured | ConvertTo-Json -Depth 6)
    $roundTripped = ($json | ConvertFrom-Json)
    $defects = @(Get-ProductionSuspendingCodeMutationBaselineDefects -Pinned $roundTripped)
    if ($defects.Count -gt 0) {
        throw (
            @("refusing to write a malformed suspending code mutation baseline: $Path") +
            @($defects | ForEach-Object { "  $_" })
        ) -join [Environment]::NewLine
    }
    $residual = @(Compare-ProductionSuspendingCodeMutationBaseline `
        -Measured $Measured -Pinned $roundTripped)
    if ($residual.Count -gt 0) {
        throw (
            @("refusing to write a suspending code mutation baseline that does not " +
              "read back as what was measured: $Path") +
            @($residual | ForEach-Object { "  $_" })
        ) -join [Environment]::NewLine
    }
    # Written beside the target and renamed over it, never truncated in place.
    # Set-Content opens the real pin for writing before it has written a byte,
    # so a cancellation, a crash or an I/O error between truncation and
    # completion destroyed the previous VALID pin and left a partial document
    # behind. The next gate would refuse that file - correctly - but the
    # regenerator would have taken away the only good copy on the way. The
    # rename is the last step and replaces one complete document with another.
    $temporary = "$Path.regenerating"
    if (Test-Path -LiteralPath $temporary) {
        Remove-Item -LiteralPath $temporary -Force
    }
    try {
        Set-Content -LiteralPath $temporary -Value $json -Encoding UTF8
        # [IO.File]::Replace, not Move-Item -Force. Windows PowerShell 5.1's
        # filesystem provider implements the move as FileInfo.MoveTo, which has
        # no overwrite semantics for an existing leaf file: -Force does not make
        # it replace one, it throws, the finally below removes the temporary,
        # and the regenerator prints "accepting" and then changes nothing. That
        # is worse than the bug it was written to fix, and it would never have
        # worked - the switch had not been exercised against an existing pin
        # when it was introduced. Proved both ways now: see
        # runs/ib_harness_fixes/repin_05_* (drift -> repin -> PASS).
        #
        # Replace() is the atomic same-volume swap that was intended: both files
        # are in the same directory by construction, and the destination is
        # required to exist, which it does whenever there is drift to accept.
        #
        # The backup path is REAL, not $null. Windows PowerShell 5.1 converts a
        # $null argument to an empty string when binding a [string] parameter,
        # and File.Replace rejects an empty backup path with "The path is not of
        # a legal form" - measured, not assumed, and it is why this function is
        # now exercised against an existing pin before being believed. The
        # backup is a second safety net while the swap happens and is removed
        # once the new pin is in place.
        if (Test-Path -LiteralPath $Path -PathType Container) {
            # Not a "first creation": a directory standing where the pin belongs
            # is a broken tree, and the Move-Item branch below would cheerfully
            # move the temporary INTO it and report success.
            throw (
                'refusing to write the suspending code mutation baseline: a ' +
                "directory exists where the pin belongs: $Path"
            )
        }
        if (Test-Path -LiteralPath $Path -PathType Leaf) {
            # A UNIQUE backup name, so this never has to delete a pre-existing
            # one to make room. Deleting `<pin>.previous` first would destroy the
            # last valid copy in exactly the case that matters - the active pin
            # already malformed, the backup the only good file - if the Replace
            # then threw.
            $backup = "$Path.previous.$([Guid]::NewGuid().ToString('N'))"
            [IO.File]::Replace($temporary, $Path, $backup)
            # Only after Replace has returned. Until this line the previous pin
            # exists under $backup, so every failure path above still has a
            # complete document on disk.
            Remove-Item -LiteralPath $backup -Force -ErrorAction SilentlyContinue
        }
        else {
            # Genuinely nothing there: nothing to replace, and nothing to lose.
            Move-Item -LiteralPath $temporary -Destination $Path
        }
    }
    finally {
        if (Test-Path -LiteralPath $temporary) {
            Remove-Item -LiteralPath $temporary -Force -ErrorAction SilentlyContinue
        }
    }
}

function Compare-ProductionSuspendingCodeMutationBaseline {
    param(
        [Parameter(Mandatory = $true)]$Measured,
        [Parameter(Mandatory = $true)]$Pinned
    )

    $differences = [System.Collections.Generic.List[string]]::new()
    if (-not ($Pinned.schema -is [string]) -or
        $Pinned.schema -cne [string]$Measured.schema) {
        $differences.Add(
            "baseline schema is '$($Pinned.schema)', measured '$($Measured.schema)'"
        )
        return @($differences)
    }
    $measuredUnits = [System.Collections.Specialized.OrderedDictionary]::new([StringComparer]::Ordinal)
    foreach ($unit in @($Measured.units)) { $measuredUnits[[string]$unit.unit] = $unit }
    $pinnedUnits = [System.Collections.Specialized.OrderedDictionary]::new([StringComparer]::Ordinal)
    foreach ($unit in @($Pinned.units)) { $pinnedUnits[[string]$unit.unit] = $unit }

    foreach ($name in @($measuredUnits.Keys)) {
        if (-not $pinnedUnits.Contains($name)) {
            $differences.Add(
                "unit is not in the accepted baseline: $name - a new suspending " +
                'code patcher has appeared'
            )
        }
    }
    foreach ($name in @($pinnedUnits.Keys)) {
        if (-not $measuredUnits.Contains($name)) {
            # A pin that no longer matches is as much a lie as a missing check,
            # so a site that has gone has to be accepted again too.
            $differences.Add(
                "unit is in the accepted baseline but no longer measures as a " +
                "suspending code patcher: $name - remove it from the baseline"
            )
        }
    }
    foreach ($name in @($measuredUnits.Keys)) {
        if (-not $pinnedUnits.Contains($name)) { continue }
        $measuredUnit = $measuredUnits[$name]
        $pinnedUnit = $pinnedUnits[$name]
        foreach ($flag in @(
            'performs_suspending_code_mutation',
            'delegates_suspending_code_mutation'
        )) {
            if ([bool]$measuredUnit.$flag -ne [bool]$pinnedUnit.$flag) {
                $differences.Add(
                    "$name : $flag is $($measuredUnit.$flag), baseline pinned " +
                    "$($pinnedUnit.$flag)"
                )
            }
        }
        $measuredEvidence = [System.Collections.Specialized.OrderedDictionary]::new([StringComparer]::Ordinal)
        foreach ($entry in @($measuredUnit.direct_evidence)) {
            $measuredEvidence["$($entry.file)|$($entry.capability)|$($entry.kind)"] =
                [int]$entry.count
        }
        $pinnedEvidence = [System.Collections.Specialized.OrderedDictionary]::new([StringComparer]::Ordinal)
        foreach ($entry in @($pinnedUnit.direct_evidence)) {
            $pinnedEvidence["$($entry.file)|$($entry.capability)|$($entry.kind)"] =
                [int]$entry.count
        }
        foreach ($key in @($measuredEvidence.Keys)) {
            if (-not $pinnedEvidence.Contains($key)) {
                $differences.Add("$name : unpinned site $key x$($measuredEvidence[$key])")
            }
            elseif ([int]$pinnedEvidence[$key] -ne [int]$measuredEvidence[$key]) {
                $differences.Add(
                    "$name : site count changed for $key - measured " +
                    "$($measuredEvidence[$key]), baseline pinned $($pinnedEvidence[$key])"
                )
            }
        }
        foreach ($key in @($pinnedEvidence.Keys)) {
            if (-not $measuredEvidence.Contains($key)) {
                $differences.Add("$name : pinned site is gone: $key")
            }
        }
        $measuredPatchers = @(@($measuredUnit.delegated_patchers) | ForEach-Object {
            "$($_.dependency)|$($_.package)|suspends=$([bool]$_.reaches_suspension)|" +
            "mutates=$([bool]$_.reaches_code_mutation)|$($_.capability_digest)"
        })
        $pinnedPatchers = @(@($pinnedUnit.delegated_patchers) | ForEach-Object {
            "$($_.dependency)|$($_.package)|suspends=$([bool]$_.reaches_suspension)|" +
            "mutates=$([bool]$_.reaches_code_mutation)|$($_.capability_digest)"
        })
        foreach ($entry in $measuredPatchers) {
            if ($pinnedPatchers -cnotcontains $entry) {
                $differences.Add("$name : unpinned delegated patcher $entry")
            }
        }
        foreach ($entry in $pinnedPatchers) {
            if ($measuredPatchers -cnotcontains $entry) {
                $differences.Add("$name : pinned delegated patcher is gone: $entry")
            }
        }
    }
    return @($differences)
}

function Get-ProductionThreadFreezeHandleEntryPoints {
    # SuspendThread and ResumeThread take a thread HANDLE, and the accepted
    # files acquire theirs with OpenThread. This is NOT a claim that OpenThread
    # is the only way to get one - NtOpenThread and ZwOpenThread reach the same
    # object through ntdll, which is why they are on the roster. It is only the
    # acquisition step those files actually use, and on its own it can neither
    # stop a thread nor change a byte. It is named here rather than read out of the accepted baseline
    # because that baseline keys on the entry points that actually suspend or
    # mutate code, so OpenThread never appears in its evidence - the WHICH-units
    # question is still answered only by the baseline, and this list only says
    # which symbol the already-accepted freeze needs to reach one.
    return @('OpenThread')
}

function Assert-ProductionThreadFreezeHandleScope {
    param(
        [Parameter(Mandatory = $true)][string]$RelativePath,
        [Parameter(Mandatory = $true)][AllowEmptyString()][string]$Code,
        [Parameter(Mandatory = $true)][string]$Symbol
    )

    # This used to also read the access rights the handle was opened with and
    # reject anything outside THREAD_SUSPEND_RESUME | THREAD_GET_CONTEXT. That
    # check is gone, for two reasons, and its absence is not a gap.
    #
    # It could not be made sound. Deciding which declaration a Rust identifier
    # binds to needs the compiler's own name resolution, and every text
    # approximation of it was evaded in review: a shadowing item with a
    # parenthesised type, a `use ... as`, a `static mut` assigned after its
    # initialiser, a macro that expands to a `const`, and finally a block-local
    # glob import that shadows the constant without the file ever spelling the
    # name. A gate that reports a mask the compiler does not use is worse than
    # no mask check, because it reads as a guarantee.
    #
    # Nor is the roster a substitute for it. Successive versions of this comment
    # claimed a wide handle could reach nothing else, because every other
    # thread-handle consumer was forbidden. That was checked three times and was
    # wrong three times. Those omissions are now named in the roster, but the
    # claim itself was the mistake: whether the thread-control surface is closed
    # is not something this lane establishes, and it is deliberately no longer
    # asserted anywhere.
    #
    # The property this exception actually rests on is narrower, and it is the
    # only one stated: these four exact files - named by the suspending-code
    # mutation baseline's own evidence, not by any list written here - receive
    # exactly the suspension symbols that evidence records, plus the handle
    # those symbols need. No other unit, file or symbol is admitted, and every
    # other file in the tree is unaffected by the exception.
    #
    # What remains is a rule that can be decided from the text and says
    # something real: in an exempt file the handle entry point may only be
    # declared and called directly. A bare reference hands the entry point to
    # something else, and where it then goes is outside anything this reads.
    $cFamily = @('.c', '.h', '.cc', '.cpp') -contains
        [IO.Path]::GetExtension($RelativePath)
    $namePattern = "(?<![A-Za-z0-9_])$([regex]::Escape($Symbol))(?![A-Za-z0-9_])"
    $calls = 0
    foreach ($match in [regex]::Matches($Code, $namePattern)) {
        # Declaring the extern ABI is not exercising it - the same distinction
        # the capability scanner already draws.
        if (Test-ProductionCapabilityDeclaration `
                -Code $Code `
                -NameIndex $match.Index `
                -AfterNameIndex ($match.Index + $match.Length) `
                -CFamily:$cFamily) {
            continue
        }
        $cursor = $match.Index + $match.Length
        while ($cursor -lt $Code.Length -and [char]::IsWhiteSpace($Code[$cursor])) {
            $cursor++
        }
        if ($cursor -ge $Code.Length -or $Code[$cursor] -ne '(') {
            throw (
                "thread-freeze handle symbol '$Symbol' is referenced without a " +
                "direct call in $RelativePath"
            )
        }
        $calls++
    }
    if ($calls -eq 0) {
        # The declaration alone is not the exception's justification. If the
        # exempt file no longer calls it, the symbol has no business being
        # named there and the exemption should have shrunk with the baseline.
        throw "thread-freeze handle symbol '$Symbol' is named but never called in $RelativePath"
    }
}

function Get-ProductionThreadFreezeExemptions {
    param(
        [Parameter(Mandatory = $true)][string]$RepoRoot,
        [Parameter(Mandatory = $true)]$Baseline
    )

    # The exempt set is READ OUT of the accepted suspending-code-mutation
    # baseline and is never written down a second time here. That is the whole
    # design: a hand-copied list of "the lanes that freeze threads" is exactly
    # the drift that let the roster ban and the accepted inventory contradict
    # each other, and it would go stale again the moment a unit is added to or
    # removed from the pin. A unit the pin has not accepted gets nothing.
    $repo = [IO.Path]::GetFullPath($RepoRoot)
    $suspensionEntryPoints = @(Get-ProductionThreadSuspensionEntryPoints)
    $mutationEntryPoints = @(Get-ProductionCodeMutationEntryPoints)
    $handleEntryPoints = @(Get-ProductionThreadFreezeHandleEntryPoints)
    $forbidden = @(Get-ForbiddenProductionClientCapabilitySymbols)

    $exemptions = [System.Collections.Specialized.OrderedDictionary]::new(
        [StringComparer]::OrdinalIgnoreCase
    )
    foreach ($unit in @($Baseline.units)) {
        # Only a unit the pin says PERFORMS the mutation itself. A unit that
        # merely delegates to an external patcher never writes these names, so
        # admitting it would widen the exception past its evidence.
        if (-not [bool]$unit.performs_suspending_code_mutation) {
            continue
        }
        $unitPath = ([string]$unit.unit).Replace('\', '/').TrimEnd('/')
        $unitFiles = [System.Collections.Specialized.OrderedDictionary]::new(
            [StringComparer]::Ordinal
        )
        foreach ($entry in @($unit.direct_evidence)) {
            if (([string]$entry.kind) -cne 'direct_call') {
                continue
            }
            $file = ([string]$entry.file).Replace('\', '/')
            if (-not $unitFiles.Contains($file)) {
                $unitFiles[$file] = [System.Collections.Generic.List[string]]::new()
            }
            $unitFiles[$file].Add([string]$entry.capability)
        }
        foreach ($file in @($unitFiles.Keys)) {
            $capabilities = @($unitFiles[$file])
            # Both halves have to be present in the SAME file: the exception
            # exists because a patcher must stop the threads it is about to
            # rewrite bytes under. A file that only suspends is not that.
            $suspends = @($capabilities | Where-Object {
                $suspensionEntryPoints -ccontains $_
            })
            $mutates = @($capabilities | Where-Object {
                $mutationEntryPoints -ccontains $_
            })
            if ($suspends.Count -eq 0 -or $mutates.Count -eq 0) {
                continue
            }
            if (-not ([string]$file).StartsWith(
                $unitPath + '/',
                [StringComparison]::Ordinal
            )) {
                throw (
                    'accepted suspending code mutation evidence names a file ' +
                    "outside its own unit: $file (unit $unitPath)"
                )
            }
            $symbols = @(
                @(@($suspends) + @($handleEntryPoints)) |
                    Where-Object { $forbidden -ccontains $_ } |
                    Select-Object -Unique
            )
            if ($symbols.Count -eq 0) {
                continue
            }
            $full = [IO.Path]::GetFullPath(
                (Join-Path $repo ($file -replace '/', '\'))
            )
            # Re-deriving the relative path throws if the join escaped the
            # repository, and comparing it back catches a pin that reaches the
            # same file through a traversal segment.
            $roundTrip = Get-ProductionInteractionRelativePath -Root $repo -Path $full
            if ($roundTrip -cne $file) {
                throw (
                    'accepted suspending code mutation evidence names a ' +
                    "non-canonical file path: $file"
                )
            }
            $exemptions[$full] = @($symbols)
        }
    }
    return $exemptions
}

function Get-ProductionInteractionPresentSymbols {
    # Which of these symbols occur in this exact stripped code as whole
    # identifiers, in the order the roster names them. The three whole-tree
    # symbol loops in Assert-ProductionInteractionSourceTreeClosure each ran
    # their own regex per symbol per file - about fifty thousand matches per
    # evaluation over the same 362 files - and the self-test drives that gate
    # once per forbidden symbol per role, so the same file's answer was
    # recomputed several hundred times.
    #
    # The answer is a pure function of the two arguments and the key is both of
    # them: the digest of the exact code text that was matched, and the exact
    # roster it was matched against. The regex the loops built by hand is
    # reproduced here once, escaped, so a caller cannot drift from it - the two
    # rosters that were previously matched unescaped are plain identifiers and
    # escaping is the identity on them.
    param(
        [Parameter(Mandatory = $true)][AllowEmptyString()][string]$Code,
        [Parameter(Mandatory = $true)][AllowEmptyCollection()][string[]]$Symbols
    )

    $key = 'symbol-presence:' + (Get-ProductionInteractionContentKey -Text (
        @(
            (Get-ProductionInteractionContentKey -Text $Code),
            (@($Symbols) -join "`0")
        ) -join "`n"
    ))
    if ($script:ProductionInteractionDerivationCache.Contains($key)) {
        return @($script:ProductionInteractionDerivationCache[$key])
    }
    $present = @(
        foreach ($symbol in @($Symbols)) {
            if ($Code -cmatch
                "(?<![A-Za-z0-9_])$([regex]::Escape($symbol))(?![A-Za-z0-9_])") {
                $symbol
            }
        }
    )
    Set-ProductionInteractionDerivation `
        -Key $key `
        -Value $present `
        -Weight (64 + 64 * $present.Count)
    return @($present)
}

function Get-ProductionInteractionForbiddenSymbolsForPath {
    param(
        [Parameter(Mandatory = $true)][string]$RepoRoot,
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)]$FreezeExemptions
    )

    # The answer depends on the repository root (it decides which of the three
    # exception paths this file is), the file's own identity, and this file's
    # entry in the freeze-exemption map - nothing else. The rest of the map
    # cannot change this path's answer, so it is deliberately not in the key.
    $pathFull = [IO.Path]::GetFullPath($Path)
    $exemptionsForPath = @(
        if ($null -ne $FreezeExemptions -and $FreezeExemptions.Contains($pathFull)) {
            $FreezeExemptions[$pathFull]
        }
    )
    $key = 'path-forbidden:' + (Get-ProductionInteractionContentKey -Text (
        @(
            [IO.Path]::GetFullPath($RepoRoot),
            $pathFull,
            ($exemptionsForPath -join "`0")
        ) -join "`n"
    ))
    if ($script:ProductionInteractionDerivationCache.Contains($key)) {
        return @($script:ProductionInteractionDerivationCache[$key])
    }
    $symbols = @(Get-ProductionInteractionForbiddenSymbolsForPathCore `
        -RepoRoot $RepoRoot `
        -Path $Path `
        -FreezeExemptions $FreezeExemptions)
    Set-ProductionInteractionDerivation `
        -Key $key `
        -Value $symbols `
        -Weight (64 + 64 * $symbols.Count)
    return @($symbols)
}

function Get-ProductionInteractionForbiddenSymbolsForPathCore {
    param(
        [Parameter(Mandatory = $true)][string]$RepoRoot,
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)]$FreezeExemptions
    )

    $repo = [IO.Path]::GetFullPath($RepoRoot)
    $forbidden = @(Get-ForbiddenProductionClientCapabilitySymbols)
    $full = [IO.Path]::GetFullPath($Path)
    $selfUiPath = [IO.Path]::GetFullPath(
        (Join-Path $repo 'extractor\native\recorder_host\src\auto_attach\product_runtime.rs')
    )
    $replayWorkerPath = [IO.Path]::GetFullPath(
        (Join-Path $repo 'replay_interface\native\replay_app\src\recording_open.rs')
    )
    $replayUiPath = [IO.Path]::GetFullPath(
        (Join-Path $repo 'replay_interface\native\replay_app\src\win32.rs')
    )
    $roleExceptions = if ($full -ieq $selfUiPath) {
        @(
            'SendMessageW',
            'ShellExecuteW',
            'EnableWindow',
            'SetWindowTextW',
            'ShowWindow',
            'DestroyWindow'
        )
    }
    elseif ($full -ieq $replayWorkerPath) {
        @(
            'CreateJobObjectW',
            'AssignProcessToJobObject',
            'SetInformationJobObject'
        )
    }
    elseif ($full -ieq $replayUiPath) {
        @('SetWindowTextW', 'DestroyWindow')
    }
    else {
        @()
    }
    $freezeExceptions = @(
        if ($null -ne $FreezeExemptions -and $FreezeExemptions.Contains($full)) {
            $FreezeExemptions[$full]
        }
    )
    $permitted = @(@($roleExceptions) + @($freezeExceptions))
    return @($forbidden | Where-Object { $permitted -cnotcontains $_ })
}

function Assert-ProductionInteractionSourceTreeClosure {
    param(
        [Parameter(Mandatory = $true)][string]$RepoRoot,
        [Parameter(Mandatory = $true)]$Authority
    )

    Enter-ProductionInteractionEvaluation
    try {

    $repo = [IO.Path]::GetFullPath($RepoRoot)
    $recorderMainPath = Join-Path $repo 'extractor\native\recorder_host\src\main.rs'
    $earlyRenderEvidencePath =
        Join-Path $repo 'extractor\native\recorder_host\src\early_render_launch.rs'
    $selfUiPath = Join-Path $repo 'extractor\native\recorder_host\src\auto_attach\product_runtime.rs'
    $replayWorkerPath = Join-Path $repo 'replay_interface\native\replay_app\src\recording_open.rs'
    $replayUiPath = Join-Path $repo 'replay_interface\native\replay_app\src\win32.rs'
    if (-not (Test-Path -LiteralPath $selfUiPath -PathType Leaf)) {
        throw 'recorder self-UI source boundary is missing'
    }
    if (-not (Test-Path -LiteralPath $recorderMainPath -PathType Leaf)) {
        throw 'recorder passive injection source boundary is missing'
    }
    if (-not (Test-Path -LiteralPath $replayWorkerPath -PathType Leaf)) {
        throw 'replay authenticated self-worker source boundary is missing'
    }
    if (-not (Test-Path -LiteralPath $replayUiPath -PathType Leaf)) {
        throw 'replay self-UI source boundary is missing'
    }

    # The rule this gate exists to enforce is behavioural: production code must
    # not rewrite live executable bytes while stopping the target's other
    # threads. It used to be spelled as a ban on one vendor's identifiers,
    # which any equivalent primitive - or a rename - satisfied without changing
    # what the code does. The check below is keyed on the operating-system
    # entry points that thread suspension and code mutation have to reach, and
    # on the measured behaviour of the external primitives the tree delegates
    # to, so an in-tree rename cannot move a site out of it.
    #
    # The tree does perform this today, in 23 units, and those are pinned as an
    # accepted inventory rather than asserted away. A gate that can only ever
    # fail is not a gate: it is pinned so that a NEW patcher, a unit that starts
    # suspending, a site that moves to another file, or a pinned site that has
    # gone all fail it. The inventory is enumerated in the output either way -
    # accepting these sites is not the same as hiding them.
    $suspendingCodeMutation = Get-ProductionSuspendingCodeMutationFindings `
        -RepoRoot $repo `
        -Authority $Authority
    $measuredBaseline = Get-ProductionSuspendingCodeMutationBaseline `
        -Findings $suspendingCodeMutation
    $baselinePath = Get-ProductionSuspendingCodeMutationBaselinePath -RepoRoot $repo
    $pinnedBaseline =
        Read-ProductionSuspendingCodeMutationBaseline -Path $baselinePath
    $differences = @(Compare-ProductionSuspendingCodeMutationBaseline `
        -Measured $measuredBaseline `
        -Pinned $pinnedBaseline)
    if ($differences.Count -gt 0) {
        $report = [System.Collections.Generic.List[string]]::new()
        $report.Add(
            'production thread-suspending code mutation does not match the ' +
            "accepted baseline ($($differences.Count) difference(s)):"
        )
        foreach ($difference in $differences) {
            $report.Add("  $difference")
        }
        $report.Add('')
        $report.Add((Format-ProductionSuspendingCodeMutationFindings `
            -Findings $suspendingCodeMutation))
        throw ($report -join [Environment]::NewLine)
    }

    # The client-capability roster below bans thread suspension outright, which
    # is correct for driving somebody else's game and wrong for the four units
    # the pin above has already accepted: those install inline detours, and
    # freezing the target's other threads is how a read-only observer avoids
    # rewriting an instruction a thread is standing on. Without an exception the
    # two authorities in this file contradict each other and the portable
    # package cannot be built from the repository at all. The exempt set is
    # taken from the pin that was just read - not written down again here - so a
    # unit the pin does not accept is still rejected, and the exception shrinks
    # to nothing on its own when those direct calls move elsewhere.
    $freezeExemptions = Get-ProductionThreadFreezeExemptions `
        -RepoRoot $repo `
        -Baseline $pinnedBaseline
    $handleEntryPoints = @(Get-ProductionThreadFreezeHandleEntryPoints)

    # The three symbol rules below used to open, read and strip every authority
    # file each - three reads of the same 362 files per evaluation, and the
    # package self-test drives this gate once per forbidden symbol per role.
    # They read the file for exactly the same thing (the comment- and
    # string-stripped code, with the early-render evidence region excised), so
    # the read happens once here and the three rules run over what it produced.
    # Their order, and therefore which defect a tree with several is reported
    # for, is unchanged: each rule still visits every file before the next rule
    # starts.
    # Keyed on the digest the authority walk already took of this file's bytes,
    # so a file that did not change is not opened again here. The excision and
    # the strip are both inside the memo because both are pure functions of
    # those bytes.
    $earlyRenderFull = [IO.Path]::GetFullPath($earlyRenderEvidencePath)
    $authorityCode = [System.Collections.Generic.List[object]]::new()
    foreach ($path in @($Authority.files)) {
        $fileDigest = Get-ProductionInteractionAuthorityFileDigest `
            -Authority $Authority `
            -Path $path
        $code = Get-ProductionInteractionFileDerivation `
            -Kind 'authority-stripped-code' `
            -Path $path `
            -FileSha256 $fileDigest `
            -Discriminator $earlyRenderFull `
            -AuthorityWalkId (Get-ProductionInteractionAuthorityWalkId -Authority $Authority) `
            -Weight 8192 `
            -Compute {
                $source = Get-Content -LiteralPath $path -Raw
                if ([IO.Path]::GetFullPath($path) -ieq $earlyRenderFull) {
                    $actuatorStart = $source.IndexOf(
                        '#[cfg(all(windows, feature = "evidence-client-driving"))]',
                        [StringComparison]::Ordinal
                    )
                    $observerStart = $source.IndexOf(
                        '#[cfg(all(windows, not(feature = "evidence-client-driving")))]',
                        [StringComparison]::Ordinal
                    )
                    if ($actuatorStart -lt 0 -or $observerStart -le $actuatorStart) {
                        throw 'early-render source lacks a bounded evidence-only actuator region'
                    }
                    $source = $source.Remove($actuatorStart, $observerStart - $actuatorStart)
                }
                Get-RustCodeWithoutCommentsOrStrings -Text $source
            }
        $authorityCode.Add([pscustomobject]@{
            Path = $path
            Code = $code
        })
    }

    foreach ($entry in $authorityCode) {
        $path = [string]$entry.Path
        $code = [string]$entry.Code
        $pathForbidden = @(Get-ProductionInteractionForbiddenSymbolsForPath `
            -RepoRoot $repo `
            -Path $path `
            -FreezeExemptions $freezeExemptions)
        $presentForbidden = @(Get-ProductionInteractionPresentSymbols `
            -Code $code `
            -Symbols $pathForbidden)
        if ($presentForbidden.Count -gt 0) {
            # First in roster order, which is the symbol the per-symbol loop
            # this replaced would have thrown on.
            throw (
                'production source contains forbidden client capability ' +
                "'$($presentForbidden[0])': $path"
            )
        }
        $fullPath = [IO.Path]::GetFullPath($path)
        if ($freezeExemptions.Contains($fullPath)) {
            $relative = Get-ProductionInteractionRelativePath -Root $repo -Path $fullPath
            foreach ($symbol in @($freezeExemptions[$fullPath])) {
                if ($handleEntryPoints -cnotcontains $symbol) {
                    continue
                }
                Assert-ProductionThreadFreezeHandleScope `
                    -RelativePath $relative `
                    -Code $code `
                    -Symbol $symbol
            }
        }
    }

    $recorderSelfUiExceptions = @(
        'SendMessageW',
        'ShellExecuteW',
        'EnableWindow',
        'SetWindowTextW',
        'ShowWindow',
        'DestroyWindow'
    )
    foreach ($entry in $authorityCode) {
        $path = [string]$entry.Path
        $code = [string]$entry.Code
        # The permitted-path test decides which symbols this file may not carry
        # and reads nothing out of the source, so it is applied first and the
        # code is searched once for what is left. Roster order is preserved, so
        # the symbol reported is the one the per-symbol loop would have reported.
        $selfUiCandidates = @(
            foreach ($symbol in $recorderSelfUiExceptions) {
                $permittedPath = if ($symbol -in @('SetWindowTextW', 'DestroyWindow')) {
                    @(
                        [IO.Path]::GetFullPath($selfUiPath),
                        [IO.Path]::GetFullPath($replayUiPath)
                    )
                }
                else {
                    @([IO.Path]::GetFullPath($selfUiPath))
                }
                if ($permittedPath -notcontains [IO.Path]::GetFullPath($path)) {
                    $symbol
                }
            }
        )
        $escapedSelfUi = @(Get-ProductionInteractionPresentSymbols `
            -Code $code `
            -Symbols $selfUiCandidates)
        if ($escapedSelfUi.Count -gt 0) {
            throw (
                "self-UI exception '$($escapedSelfUi[0])' escaped its exact " +
                "product window source: $path"
            )
        }
    }
    $selfUiSource = Get-Content -LiteralPath $selfUiPath -Raw
    $selfUiCode = Get-RustCodeWithoutCommentsOrStrings -Text $selfUiSource
    if ([regex]::Matches($selfUiCode, '(?<![A-Za-z0-9_])SendMessageW(?![A-Za-z0-9_])').Count -ne 5 -or
        [regex]::Matches($selfUiCode, '(?m)^\s*fn SendMessageW\(').Count -ne 1 -or
        [regex]::Matches($selfUiCode, 'SendMessageW\s*\(\s*status\s*,').Count -ne 4 -or
        [regex]::Matches($selfUiCode, '(?<![A-Za-z0-9_])ShellExecuteW(?![A-Za-z0-9_])').Count -ne 2 -or
        [regex]::Matches($selfUiCode, '(?m)^\s*fn ShellExecuteW\(').Count -ne 1 -or
        [regex]::Matches($selfUiCode, 'let\s+opened\s*=\s*ShellExecuteW\s*\(\s*hwnd\s*,').Count -ne 1 -or
        [regex]::Matches($selfUiCode, '(?<![A-Za-z0-9_])EnableWindow(?![A-Za-z0-9_])').Count -ne 3 -or
        [regex]::Matches($selfUiCode, '(?m)^\s*fn EnableWindow\(').Count -ne 1 -or
        [regex]::Matches($selfUiCode, 'EnableWindow\s*\(\s*button\s*,\s*[01]\s*\)').Count -ne 2 -or
        [regex]::Matches($selfUiCode, '(?<![A-Za-z0-9_])SetWindowTextW(?![A-Za-z0-9_])').Count -ne 4 -or
        [regex]::Matches($selfUiCode, '(?m)^\s*fn SetWindowTextW\(').Count -ne 1 -or
        [regex]::Matches($selfUiCode, 'SetWindowTextW\s*\(\s*button\s*,').Count -ne 1 -or
        [regex]::Matches($selfUiCode, 'SetWindowTextW\s*\(\s*status\s*,').Count -ne 2 -or
        [regex]::Matches($selfUiCode, '(?<![A-Za-z0-9_])ShowWindow(?![A-Za-z0-9_])').Count -ne 3 -or
        [regex]::Matches($selfUiCode, '(?m)^\s*fn ShowWindow\(').Count -ne 1 -or
        [regex]::Matches($selfUiCode, 'ShowWindow\s*\(\s*(?:console|hwnd)\s*,\s*(?:SW_HIDE|SW_SHOW)\s*\)').Count -ne 2 -or
        [regex]::Matches($selfUiCode, '(?<![A-Za-z0-9_])DestroyWindow(?![A-Za-z0-9_])').Count -ne 4 -or
        [regex]::Matches($selfUiCode, '(?m)^\s*fn DestroyWindow\(').Count -ne 1 -or
        [regex]::Matches($selfUiCode, 'DestroyWindow\s*\(\s*hwnd\s*\)').Count -ne 3) {
        throw 'recorder self-UI API exception differs from the exact existing UX callsites'
    }
    $replayUiCode = Get-RustCodeWithoutCommentsOrStrings -Text (
        Get-Content -LiteralPath $replayUiPath -Raw
    )
    if ([regex]::Matches(
        $replayUiCode,
        '(?<![A-Za-z0-9_])SetWindowTextW(?![A-Za-z0-9_])'
    ).Count -ne 2 -or
        [regex]::Matches(
            $replayUiCode,
            'SetWindowTextW\s*\(\s*self\.hwnd\s*,\s*title_wide\.as_ptr\(\)\s*\)'
        ).Count -ne 1 -or
        [regex]::Matches(
            $replayUiCode,
            '(?<![A-Za-z0-9_])DestroyWindow(?![A-Za-z0-9_])'
        ).Count -ne 2 -or
        [regex]::Matches(
            $replayUiCode,
            'DestroyWindow\s*\(\s*self\.hwnd\s*\)'
        ).Count -ne 1) {
        throw 'replay self-UI title exception differs from the exact product window callsite'
    }
    $injectionSymbols = @(
        'VirtualAllocEx',
        'VirtualFreeEx',
        'WriteProcessMemory',
        'CreateRemoteThread',
        'LoadLibraryW',
        'FreeLibrary'
    )
    foreach ($entry in $authorityCode) {
        $path = [string]$entry.Path
        if ([IO.Path]::GetFullPath($path) -ine
            [IO.Path]::GetFullPath($recorderMainPath)) {
            $code = [string]$entry.Code
            $escapedInjection = @(Get-ProductionInteractionPresentSymbols `
                -Code $code `
                -Symbols $injectionSymbols)
            if ($escapedInjection.Count -gt 0) {
                throw (
                    "passive injection capability '$($escapedInjection[0])' " +
                    "escaped recorder main seam: $path"
                )
            }
        }
    }
    $recorderMainCode = Get-RustCodeWithoutCommentsOrStrings -Text (
        Get-Content -LiteralPath $recorderMainPath -Raw
    )
    $injectionCensus = [ordered]@{
        VirtualAllocEx = 2
        VirtualFreeEx = 2
        WriteProcessMemory = 2
        CreateRemoteThread = 3
        LoadLibraryW = 3
        FreeLibrary = 8
    }
    foreach ($entry in $injectionCensus.GetEnumerator()) {
        if ([regex]::Matches(
            $recorderMainCode,
            "(?<![A-Za-z0-9_])$([regex]::Escape([string]$entry.Key))(?![A-Za-z0-9_])"
        ).Count -ne [int]$entry.Value) {
            throw "recorder passive injection capability '$($entry.Key)' differs from its exact source census"
        }
    }
    foreach ($exactCall in @(
        [pscustomobject]@{ Name = 'VirtualAllocEx'; Pattern = 'VirtualAllocEx\s*\(\s*process\.handle\s*,\s*ptr::null_mut\(\)\s*,\s*collector_bytes\.len\(\)\s*,\s*MEM_COMMIT\s*\|\s*MEM_RESERVE\s*,\s*PAGE_READWRITE\s*,?\s*\)'; Count = 1 },
        [pscustomobject]@{ Name = 'WriteProcessMemory'; Pattern = 'WriteProcessMemory\s*\(\s*process\.handle\s*,\s*remote_path\s*,\s*collector_bytes\.as_ptr\(\)\.cast\(\)\s*,\s*collector_bytes\.len\(\)\s*,\s*&mut\s+written\s*,?\s*\)'; Count = 1 },
        [pscustomobject]@{ Name = 'VirtualFreeEx'; Pattern = 'VirtualFreeEx\s*\(\s*process\.handle\s*,\s*remote_path\s*,\s*0\s*,\s*MEM_RELEASE\s*,?\s*\)'; Count = 1 },
        [pscustomobject]@{ Name = 'CreateRemoteThread LoadLibrary'; Pattern = 'CreateRemoteThread\s*\(\s*process\.handle\s*,\s*ptr::null_mut\(\)\s*,\s*0\s*,\s*remote_load_library\s+as\s+\*const\s+c_void\s*,\s*remote_path\s*,\s*0\s*,\s*ptr::null_mut\(\)\s*,?\s*\)'; Count = 1 },
        [pscustomobject]@{ Name = 'CreateRemoteThread FreeLibrary'; Pattern = 'CreateRemoteThread\s*\(\s*remote\.process\.handle\s*,\s*ptr::null_mut\(\)\s*,\s*0\s*,\s*remote\.free_library_address\s+as\s+\*const\s+c_void\s*,\s*remote\.module_base\s+as\s+\*mut\s+c_void\s*,\s*0\s*,\s*ptr::null_mut\(\)\s*,?\s*\)'; Count = 1 }
    )) {
        if ([regex]::Matches(
            $recorderMainCode,
            [string]$exactCall.Pattern
        ).Count -ne [int]$exactCall.Count) {
            throw "recorder passive injection callsite '$($exactCall.Name)' differs from the exact DLL LoadLibrary/FreeLibrary seam"
        }
    }
    $replayWorkerSource = Get-Content -LiteralPath $replayWorkerPath -Raw
    $replayWorkerCode = Get-RustCodeWithoutCommentsOrStrings -Text $replayWorkerSource
    foreach ($symbol in @(
        'CreateJobObjectW',
        'AssignProcessToJobObject',
        'SetInformationJobObject'
    )) {
        if ([regex]::Matches(
            $replayWorkerCode,
            "(?<![A-Za-z0-9_])$symbol(?![A-Za-z0-9_])"
        ).Count -ne 2) {
            throw "replay self-worker exception '$symbol' differs from its exact declaration/call pair"
        }
    }
    foreach ($required in @(
        'AuthenticatedSelfExecutable::open_prefold_worker()',
        'Command::new(worker.path())',
        'resource_job.assign(&child)',
        'worker.verify_unchanged()'
    )) {
        if (-not $replayWorkerSource.Contains($required)) {
            throw "replay self-worker exception lacks authenticated lifecycle marker: $required"
        }
    }
    $replayRoot = [IO.Path]::GetFullPath(
        (Join-Path $repo 'replay_interface\native\replay_app\src')
    )
    $replayCommandCount = 0
    $replayChildKillCount = 0
    foreach ($path in @($Authority.files)) {
        $fullPath = [IO.Path]::GetFullPath($path)
        if (-not $fullPath.StartsWith(
            $replayRoot.TrimEnd('\') + '\',
            [StringComparison]::OrdinalIgnoreCase
        )) {
            continue
        }
        $code = Get-RustCodeWithoutCommentsOrStrings -Text (
            Get-Content -LiteralPath $path -Raw
        )
        $commandCount = [regex]::Matches(
            $code,
            '(?<![A-Za-z0-9_])Command::new\s*\('
        ).Count
        $childKillCount = [regex]::Matches(
            $code,
            '(?<![A-Za-z0-9_])child\.kill\s*\('
        ).Count
        if ($fullPath -ine [IO.Path]::GetFullPath($replayWorkerPath) -and
            ($commandCount -ne 0 -or $childKillCount -ne 0)) {
            throw "replay self-worker process lifecycle escaped recording_open.rs: $path"
        }
        $replayCommandCount += $commandCount
        $replayChildKillCount += $childKillCount
    }
    if ($replayCommandCount -ne 1 -or
        [regex]::Matches(
            $replayWorkerCode,
            '(?<![A-Za-z0-9_])Command::new\s*\(\s*worker\.path\(\)\s*\)'
        ).Count -ne 1 -or
        $replayChildKillCount -ne 4 -or
        [regex]::Matches(
            $replayWorkerCode,
            '(?<![A-Za-z0-9_])child\.kill\s*\(\s*\)'
        ).Count -ne 4) {
        throw 'replay self-worker Command/Child lifecycle differs from the exact authenticated callsite census'
    }

    }
    finally {
        Exit-ProductionInteractionEvaluation
    }
}
