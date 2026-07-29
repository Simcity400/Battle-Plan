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
        }
        if ($dependencyField -ceq 'path') {
            $record.path = ([string]$assignment.Value).Trim().Trim('"', "'")
        }
        elseif ($dependencyField -ceq 'workspace') {
            $record.workspace = (([string]$assignment.Value).Trim() -ceq 'true')
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
        foreach ($primitive in @(
            'minhook::',
            'MinHook::',
            'MinHook::create_hook',
            'MinHook::enable_hook',
            'MinHook::disable_hook',
            'MinHook::remove_hook',
            'MH_CreateHook',
            'MH_EnableHook',
            'MH_DisableHook',
            'MH_RemoveHook'
        )) {
            if ($code -cmatch [regex]::Escape($primitive)) {
                throw "production source contains suspending MinHook primitive '$primitive': $path"
            }
        }
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
