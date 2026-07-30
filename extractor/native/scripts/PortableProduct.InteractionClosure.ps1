Set-StrictMode -Version Latest

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
        $sectionParts = if ([string]::IsNullOrWhiteSpace($section)) {
            @()
        }
        else {
            @(Split-TomlDottedKey -Text $section)
        }
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
        $sectionParts = if ([string]::IsNullOrWhiteSpace([string]$assignment.Section)) {
            @()
        }
        else {
            @(Split-TomlDottedKey -Text ([string]$assignment.Section)
            )
        }
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
    param([Parameter(Mandatory = $true)][string]$Toml)

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
    return @($records.Values)
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
                $matches = @($workspaceRecords | Where-Object {
                    [string]$_.name -ceq [string]$record.name
                })
                if ($matches.Count -ne 1 -or
                    [string]::IsNullOrWhiteSpace([string]$matches[0].path)) {
                    throw "workspace production dependency lacks one exact path: $($record.name)"
                }
                $dependencyPath = [string]$matches[0].path
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

function Get-ProductionInteractionSourceTreeAuthority {
    param([Parameter(Mandatory = $true)][string]$RepoRoot)

    $repo = [IO.Path]::GetFullPath($RepoRoot)
    $roots = @(
        (Join-Path $repo 'extractor\native\recorder_host'),
        (Join-Path $repo 'extractor\native\collector'),
        (Join-Path $repo 'replay_interface\native\replay_app')
    ) + @(Get-CollectorProductionLaneRoots -RepoRoot $repo)
    $sourceExtensions = @('.rs', '.c', '.h', '.cc', '.cpp')
    $files = @(
        foreach ($root in $roots) {
            if (-not (Test-Path -LiteralPath $root -PathType Container)) {
                throw "production interaction source root is missing: $root"
            }
            Assert-ProductionRepoPathIsPhysical -RepoRoot $repo -Path $root
            $entries = @(Get-ChildItem -LiteralPath $root -Recurse -Force)
            foreach ($entry in $entries) {
                if (($entry.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
                    throw "production interaction source tree contains a reparse point: $($entry.FullName)"
                }
            }
            $entries |
                Where-Object { -not $_.PSIsContainer } |
                Where-Object { $sourceExtensions -contains $_.Extension }
        }
    ) | Sort-Object {
        Get-ProductionInteractionRelativePath -Root $repo -Path $_.FullName
    }
    if ($files.Count -eq 0) {
        throw 'production interaction source tree is empty'
    }
    $rows = @(
        foreach ($file in $files) {
            [ordered]@{
                path = Get-ProductionInteractionRelativePath `
                    -Root $repo `
                    -Path $file.FullName
                sha256 = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
            }
        }
    )
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
    return [pscustomobject]@{
        files = @($files.FullName)
        file_count = $files.Count
        sha256 = $treeSha
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
    $productionIds = @(
        $registry.domains |
            Where-Object {
                $_.schedulable -eq $true -and
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
            $sites.Add([ordered]@{
                path = $Path
                line = Get-ProductionSourceLineNumber -Text $Code -Index $match.Index
                capability = $entryPoint
                token = $entryPoint
                kind = if ($isCall) { 'direct_call' } else { 'capability_reference' }
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

    $sourceExtensions = @('.rs', '.c', '.h', '.cc', '.cpp')
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
        if ([string]::IsNullOrEmpty($source) -or $source -cnotmatch $prefilter) {
            continue
        }
        $cFamily = ($cFamilyExtensions -contains $file.Extension)
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
    return [ordered]@{
        root = $rootFull
        suspension_sites = @($suspensionSites)
        code_mutation_sites = @($mutationSites)
        suspends_and_mutates_code =
            ($suspensionSites.Count -gt 0 -and $mutationSites.Count -gt 0)
    }
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
    $versions = @([regex]::Matches(
        $lock,
        '(?m)^\[\[package\]\]\r?\nname = "' + [regex]::Escape($Package) +
            '"\r?\nversion = "(?<version>[^"]+)"'
    ) | ForEach-Object { $_.Groups['version'].Value } | Select-Object -Unique)
    if ($versions.Count -ne 1) {
        throw "production dependency '$Package' does not resolve to one exact locked version"
    }
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

function Get-ProductionDependencyCodeMutationClassification {
    param([Parameter(Mandatory = $true)][string]$RepoRoot)

    $repo = [IO.Path]::GetFullPath($RepoRoot)
    $suspensionEntryPoints = @(Get-ProductionThreadSuspensionEntryPoints)
    $codeMutationEntryPoints = @(Get-ProductionCodeMutationEntryPoints)
    $unitRoots = @(Get-ProductionInteractionSourceRoots -RepoRoot $repo)
    $unitRootSet = [System.Collections.Generic.HashSet[string]]::new(
        [StringComparer]::OrdinalIgnoreCase
    )
    foreach ($unitRoot in $unitRoots) {
        [void]$unitRootSet.Add([IO.Path]::GetFullPath($unitRoot).TrimEnd('\'))
    }
    $edges = [System.Collections.Generic.List[object]]::new()
    $scans = [ordered]@{}
    foreach ($unitRoot in $unitRoots) {
        $unitFull = [IO.Path]::GetFullPath($unitRoot).TrimEnd('\')
        $manifestPath = Join-Path $unitFull 'Cargo.toml'
        if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) {
            throw "production unit Cargo manifest is missing: $manifestPath"
        }
        $lockPath = $null
        $probe = Split-Path -Parent $unitFull
        while (-not [string]::IsNullOrEmpty($probe)) {
            $candidate = Join-Path $probe 'Cargo.lock'
            if (Test-Path -LiteralPath $candidate -PathType Leaf) {
                $lockPath = $candidate
                break
            }
            $probe = Split-Path -Parent $probe
        }
        if ($null -eq $lockPath) {
            throw "production unit has no owning Cargo.lock: $unitFull"
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
            $dependencyBase = $unitFull
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
            $dependencyRoot = if (-not [string]::IsNullOrWhiteSpace($dependencyPath)) {
                [IO.Path]::GetFullPath((Join-Path $dependencyBase $dependencyPath)).TrimEnd('\')
            }
            else {
                Get-CargoRegistryCrateRoot `
                    -Package $package `
                    -Version (Get-CargoLockPackageVersion -LockPath $lockPath -Package $package)
            }
            if ($unitRootSet.Contains($dependencyRoot)) {
                # A production unit is scanned in its own right below.
                continue
            }
            if (-not $scans.Contains($dependencyRoot)) {
                $scans[$dependencyRoot] = Get-ProductionCrateSourceCapabilityScan `
                    -Root $dependencyRoot `
                    -SuspensionEntryPoints $suspensionEntryPoints `
                    -CodeMutationEntryPoints $codeMutationEntryPoints
            }
            $edges.Add([ordered]@{
                unit = Get-ProductionInteractionRelativePath -Root $repo -Path $unitFull
                unit_root = $unitFull
                manifest = Get-ProductionInteractionRelativePath -Root $repo -Path $manifestPath
                dependency = $dependencyName
                package = $package
                dependency_root = $dependencyRoot
            })
        }
    }
    $classified = [ordered]@{}
    foreach ($key in @($scans.Keys)) {
        if ($scans[$key].suspends_and_mutates_code) {
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
    return [ordered]@{
        schema = 'hoi4.production-dependency-code-mutation-classification.v1'
        scanned_dependency_count = $scans.Count
        classified_patchers = $classified
        edges = @($edges)
    }
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
    # dependency and moves when its behaviour does.
    $counts = [System.Collections.Specialized.OrderedDictionary]::new([StringComparer]::Ordinal)
    foreach ($site in (@($Scan.suspension_sites) + @($Scan.code_mutation_sites))) {
        $key = "$($site.path)|$($site.capability)|$($site.kind)"
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
        $source = Get-ProductionInteractionScannableSource `
            -Path $fullPath `
            -EarlyRenderEvidencePath $earlyRenderEvidencePath
        $code = Get-RustCodeWithoutCommentsOrStrings -Text $source
        $cFamily = @('.c', '.h', '.cc', '.cpp') -contains
            [IO.Path]::GetExtension($fullPath)
        Add-ProductionCapabilityEvidence `
            -Accumulator $units[$unitRoot].suspension `
            -Evidence (Get-ProductionCapabilityEvidence `
                -Source $source `
                -Code $code `
                -EntryPoints $suspensionEntryPoints `
                -Path $relative `
                -CFamily:$cFamily)
        Add-ProductionCapabilityEvidence `
            -Accumulator $units[$unitRoot].mutation `
            -Evidence (Get-ProductionCapabilityEvidence `
                -Source $source `
                -Code $code `
                -EntryPoints $codeMutationEntryPoints `
                -Path $relative `
                -CFamily:$cFamily)
        if ($delegatedCrates.Contains($unitRoot)) {
            foreach ($edge in @($delegatedCrates[$unitRoot])) {
                $crateRootIdent = ([string]$edge.dependency) -replace '-', '_'
                $pattern =
                    "(?<![A-Za-z0-9_])$([regex]::Escape($crateRootIdent))\s*::"
                foreach ($match in [regex]::Matches($code, $pattern)) {
                    $units[$unitRoot].delegation_sites.Add([ordered]@{
                        path = $relative
                        line = Get-ProductionSourceLineNumber -Text $code -Index $match.Index
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
        if (-not $directlySuspendsAndMutates -and -not $delegates) {
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
                            capability_digest = [string]$classification.classified_patchers[
                                [string]$_.dependency_root
                            ].capability_digest
                        }
                    }
                }
            )
            suspension_sites = @(
                if ($directlySuspendsAndMutates) { $unit.suspension_sites }
            )
            code_mutation_sites = @(
                if ($directlySuspendsAndMutates) { $unit.code_mutation_sites }
            )
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
        $lines.Add("  external primitive classified as a suspending code patcher: $($patcher.root)")
        foreach ($site in @($patcher.suspension_sites)) {
            $lines.Add("      suspends: $($site.path):$($site.line) $($site.capability)")
        }
        foreach ($site in @($patcher.code_mutation_sites)) {
            $lines.Add("      mutates code: $($site.path):$($site.line) $($site.capability)")
        }
    }
    foreach ($unit in @($Findings.non_conforming_units)) {
        $lines.Add("  unit $($unit.unit):")
        if ($unit.performs_suspending_code_mutation) {
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
        }
        if ($unit.delegates_suspending_code_mutation) {
            foreach ($patcher in @($unit.delegated_patchers)) {
                $lines.Add(
                    "      delegates to suspending code patcher " +
                    "'$($patcher.dependency)' declared in $($patcher.manifest)"
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
            "$($_.dependency)|$($_.package)|$($_.capability_digest)"
        })
        $pinnedPatchers = @(@($pinnedUnit.delegated_patchers) | ForEach-Object {
            "$($_.dependency)|$($_.package)|$($_.capability_digest)"
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

function Assert-ProductionInteractionSourceTreeClosure {
    param(
        [Parameter(Mandatory = $true)][string]$RepoRoot,
        [Parameter(Mandatory = $true)]$Authority
    )

    $repo = [IO.Path]::GetFullPath($RepoRoot)
    $forbidden = @(Get-ForbiddenProductionClientCapabilitySymbols)
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

    foreach ($path in @($Authority.files)) {
        $source = Get-Content -LiteralPath $path -Raw
        if ([IO.Path]::GetFullPath($path) -ieq
            [IO.Path]::GetFullPath($earlyRenderEvidencePath)) {
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
        $code = Get-RustCodeWithoutCommentsOrStrings -Text $source
        $pathForbidden = if ([IO.Path]::GetFullPath($path) -ieq
            [IO.Path]::GetFullPath($selfUiPath)) {
            @($forbidden | Where-Object {
                $_ -cnotin @(
                    'SendMessageW',
                    'ShellExecuteW',
                    'EnableWindow',
                    'SetWindowTextW',
                    'ShowWindow',
                    'DestroyWindow'
                )
            })
        }
        elseif ([IO.Path]::GetFullPath($path) -ieq
            [IO.Path]::GetFullPath($replayWorkerPath)) {
            @($forbidden | Where-Object {
                $_ -cnotin @(
                    'CreateJobObjectW',
                    'AssignProcessToJobObject',
                    'SetInformationJobObject'
                )
            })
        }
        elseif ([IO.Path]::GetFullPath($path) -ieq
            [IO.Path]::GetFullPath($replayUiPath)) {
            @($forbidden | Where-Object {
                $_ -cnotin @('SetWindowTextW', 'DestroyWindow')
            })
        }
        else {
            @($forbidden)
        }
        foreach ($symbol in $pathForbidden) {
            if ($code -cmatch "(?<![A-Za-z0-9_])$([regex]::Escape($symbol))(?![A-Za-z0-9_])") {
                throw "production source contains forbidden client capability '$symbol': $path"
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
    foreach ($path in @($Authority.files)) {
        $source = Get-Content -LiteralPath $path -Raw
        if ([IO.Path]::GetFullPath($path) -ieq
            [IO.Path]::GetFullPath($earlyRenderEvidencePath)) {
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
        $code = Get-RustCodeWithoutCommentsOrStrings -Text $source
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
            if ($permittedPath -notcontains [IO.Path]::GetFullPath($path) -and
                $code -cmatch "(?<![A-Za-z0-9_])$symbol(?![A-Za-z0-9_])") {
                throw "self-UI exception '$symbol' escaped its exact product window source: $path"
            }
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
    foreach ($path in @($Authority.files)) {
        $source = Get-Content -LiteralPath $path -Raw
        if ([IO.Path]::GetFullPath($path) -ieq
            [IO.Path]::GetFullPath($earlyRenderEvidencePath)) {
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
        if ([IO.Path]::GetFullPath($path) -ine
            [IO.Path]::GetFullPath($recorderMainPath)) {
            $code = Get-RustCodeWithoutCommentsOrStrings -Text $source
            foreach ($symbol in $injectionSymbols) {
                if ($code -cmatch
                    "(?<![A-Za-z0-9_])$([regex]::Escape($symbol))(?![A-Za-z0-9_])") {
                    throw "passive injection capability '$symbol' escaped recorder main seam: $path"
                }
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
