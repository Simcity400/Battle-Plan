<#
.SYNOPSIS
Bounded Windows viewer playback through reached public Win32 routes.
.DESCRIPTION
Run is opt-in. The default Prepare mode launches nothing and emits no playback
PASS. Run uses the exact supplied viewer for recording, whole-playback and
content CLI admission, then opens that recording in a responsive desktop window.
It preserves raw stdout/stderr and actual presentation dispositions.

At f3227084 durable presentation/UI output is unavailable. The proposed first
app.rs seam emits hoi4.viewer.desktop.present.v1 and hoi4.viewer.desktop.ui.v1
under REPLAY_DESKTOP_DIAGNOSTICS=1. These schemas are a PROPOSAL, not an API that
exists at this revision. The complete shared seam also needs input.v1 receipts
and the committed UI projection documented by Assert-VdpInteractionSeam below.
Without those fields this runner fails by name before sending input.
Parent/ARROW owns that reservation. No parameter bypasses receipt requirements.

Custody follows Invoke-InstalledProductSmoke.ps1: non-elevated token, exact
artifact hashes, isolated child environment, PID/creation time, responsive
owned window, cooperative close, raw logs and owned-process-only cleanup.
This script does not dot-source smoke (which would launch both products).

Single .battleplan only. Package sets need all-member custody and explicit
logical-stream selection; they are refused here rather than tested as one file.
#>
[CmdletBinding()]
param(
    [ValidateSet('Prepare', 'Run')][string]$Mode = 'Prepare',
    [string]$ViewerExecutable,
    [string]$ExpectedViewerSha256,
    [string]$RecordingPath,
    [string]$ExpectedRecordingSha256,
    [string]$GameRoot,
    [string]$UserDataRoot,
    [string]$ExpectedContentAuthoritySha256,
    [Nullable[long]]$ExpectedHour,
    [Nullable[long]]$SeekHour,
    [string]$FirstCountryTag,
    [string]$SecondCountryTag,
    [string]$TooltipWindowId,
    [string]$TooltipElementId,
    [string]$OutputDirectory,
    [ValidateRange(5, 60)][int]$TimeoutSeconds = 20
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Stop-Vdp {
    param([string]$Code, [string]$Detail)
    throw ($Code + ': ' + $Detail)
}

function Assert-VdpSha256 {
    param($Value, [string]$Name)
    if ($Value -isnot [string] -or $Value -cnotmatch '^[0-9a-f]{64}$') {
        Stop-Vdp 'VDP_IDENTITY_INVALID' "$Name must be a lowercase SHA256."
    }
}

function Get-VdpReference {
    param([string]$Path)
    $item = Get-Item -LiteralPath $Path
    if ($item.PSIsContainer) { Stop-Vdp 'VDP_FILE_REQUIRED' $Path }
    [ordered]@{ path = $item.FullName; bytes = $item.Length
        sha256 = (Get-FileHash -LiteralPath $item.FullName -Algorithm SHA256).Hash.ToLowerInvariant() }
}

function ConvertTo-VdpArgument {
    param([AllowEmptyString()][string]$Value)
    # CommandLineToArgvW quoting, including embedded quotes and trailing slashes.
    '"' + [regex]::Replace([regex]::Replace($Value, '(\\*)"', '$1$1\"'), '(\\+)$', '$1$1') + '"'
}

function Get-VdpCliPlan {
    param([string]$Recording, [string]$Game, [string]$UserData, [string]$AuthorityOutput)
    @(
        [ordered]@{ name = 'validate-recording'; argv = @('--validate-recording', $Recording, '--non-interactive') },
        [ordered]@{ name = 'validate-playback'; argv = @('--validate-playback', $Recording, '--game', $Game, '--userdata', $UserData, '--non-interactive') },
        [ordered]@{ name = 'content-authority'; argv = @('--report-content-authority', $Recording, '--report-out', $AuthorityOutput, '--game', $Game, '--userdata', $UserData, '--non-interactive') },
        [ordered]@{ name = 'desktop'; argv = @('--recording', $Recording, '--game', $Game, '--userdata', $UserData, '--non-interactive') }
    )
}

function Read-VdpJsonLines {
    param([string]$Text, [switch]$Complete)
    # Live stream tails are incomplete until a newline arrives. Debug summaries
    # and similarly named text never become structured presentation evidence.
    $lines = $Text -split "`n"
    $count = $lines.Count - 1
    if ($Complete -and $lines[-1].Length -gt 0) {
        if ($lines[-1] -match 'hoi4\.viewer\.desktop\.') {
            Stop-Vdp 'VDP_RECEIPT_TRUNCATED' 'Final diagnostic line has no newline.'
        }
    }
    for ($index = 0; $index -lt $count; $index++) {
        $line = $lines[$index].TrimEnd("`r")
        if (-not $line.StartsWith('{')) { continue }
        try { $row = $line | ConvertFrom-Json } catch {
            if ($line -match 'hoi4\.viewer\.desktop\.') {
                Stop-Vdp 'VDP_RECEIPT_MALFORMED' "Diagnostic line $($index + 1) is invalid JSON."
            }
            continue
        }
        if ($row.PSObject.Properties.Name -contains 'schema' -and
            [string]$row.schema -cmatch '^hoi4\.viewer\.desktop\.') { $row }
    }
}

function Assert-VdpWholePlayback {
    param([string]$Text)
    $started = @(); $completed = @()
    foreach ($line in ($Text -split "`n")) {
        if (-not $line.StartsWith('{')) { continue }
        try { $row = $line | ConvertFrom-Json } catch { continue }
        if ($row.PSObject.Properties.Name -notcontains 'schema') { continue }
        if ($row.schema -ceq 'replay.validate_playback.stage.v1') { $started += $row }
        if ($row.schema -ceq 'replay.validate_playback.completed.v1') { $completed += $row }
    }
    if ($started.Count -ne 1 -or $completed.Count -ne 1 -or
        $started[0].stage -cne 'all_frame_payloads_and_static_catalog_bindings' -or
        $completed[0].stage -cne $started[0].stage -or $started[0].result -cne 'STARTED' -or
        $completed[0].result -cne 'PASS' -or $completed[0].hours_validated -le 0 -or
        $started[0].expected_hours -ne $completed[0].hours_validated) {
        Stop-Vdp 'VDP_WHOLE_PLAYBACK_RECEIPT_MISSING' 'Require a nonempty completed whole timeline, matching the started hour count.'
    }
}

function Assert-VdpAuthority {
    param($AuthorityReport, [string]$Recording, [string]$ExpectedDigest)
    Assert-VdpSha256 $ExpectedDigest 'expected content authority'
    if ($AuthorityReport.schema -cne 'hoi4.replay.content_authority_report.v1' -or
        [IO.Path]::GetFullPath($AuthorityReport.recording) -ine [IO.Path]::GetFullPath($Recording)) {
        Stop-Vdp 'VDP_CONTENT_RECORDING_MISMATCH' 'Content report must name the exact launched recording.'
    }
    $authority = $AuthorityReport.bundle_content_authority
    foreach ($field in @('authority_sha256', 'manifest_digest_sha256', 'product_modset_sha256', 'effective_vfs_manifest_sha256')) {
        Assert-VdpSha256 $authority.$field $field
    }
    if ($authority.authority_sha256 -cne $ExpectedDigest) {
        Stop-Vdp 'VDP_CONTENT_AUTHORITY_MISMATCH' 'Product-admitted authority differs from the parent-pinned authority.'
    }
    $authority
}

function Get-VdpDesktopEvidence {
    param([object[]]$Rows, [int]$ViewerPid, $Authority, [long]$Hour, $State = $null)
    $pending = $null; $current = $null; $lastGeneration = 0L
    $presented = 0; $rejected = [Collections.Generic.List[object]]::new()
    $frames = [Collections.Generic.List[object]]::new()
    $inputs = [Collections.Generic.List[object]]::new()
    $lastInput = 0L
    if ($null -ne $State) {
        $pending = $State.pending; $current = $State.current; $lastGeneration = $State.last_generation
        $presented = $State.presented_count; $lastInput = $State.last_input
        $rejected = $State.rejected; $frames = $State.frames; $inputs = $State.inputs
    }
    foreach ($row in $Rows) {
        if ($row.schema -cnotin @('hoi4.viewer.desktop.present.v1', 'hoi4.viewer.desktop.ui.v1', 'hoi4.viewer.desktop.input.v1')) {
            Stop-Vdp 'VDP_RECEIPT_SCHEMA_UNSUPPORTED' ([string]$row.schema)
        }
        if ($row.pid -ne $ViewerPid) { Stop-Vdp 'VDP_PROCESS_RECEIPT_MISMATCH' 'Receipt PID differs from the owned child.' }
        if ($row.schema -ceq 'hoi4.viewer.desktop.input.v1') {
            if (($row.input_seq -isnot [int] -and $row.input_seq -isnot [long]) -or $row.input_seq -le $lastInput -or
                $null -eq $current -or $row.generation -ne $current.generation) {
                Stop-Vdp 'VDP_INPUT_GENERATION_MISMATCH' 'Input must identify the current actual presented UI generation in stream order.'
            }
            if ($row.kind -cnotin @('key_down', 'mouse_move')) { Stop-Vdp 'VDP_INPUT_ROUTE_UNSUPPORTED' ([string]$row.kind) }
            $lastInput = [long]$row.input_seq
            $inputs.Add($row)
            continue
        }
        $map = $row.map
        if ($null -eq $map) { Stop-Vdp 'VDP_PRESENT_RECEIPT_MISSING' 'No finalized map receipt.' }
        if ($map.schema -cne 'hoi4.replay-map-composition.xmap22.v7') {
            Stop-Vdp 'VDP_MAP_SCHEMA_UNSUPPORTED' ([string]$map.schema)
        }
        foreach ($pair in @(@('content_manifest_sha256', 'manifest_digest_sha256'),
                            @('product_modset_sha256', 'product_modset_sha256'),
                            @('effective_vfs_manifest_sha256', 'effective_vfs_manifest_sha256'))) {
            if ($map.($pair[0]) -cne $Authority.($pair[1])) {
                Stop-Vdp 'VDP_FRAME_CONTENT_MISMATCH' $pair[0]
            }
        }
        foreach ($field in @('deterministic_sha256', 'replay_frame_identity_sha256', 'viewer_state_sha256')) {
            Assert-VdpSha256 $map.$field $field
        }
        if ($row.schema -ceq 'hoi4.viewer.desktop.present.v1') {
            # Every disposition revokes harness input eligibility. Only the
            # following matching post-disposition UI receipt can restore it.
            $current = $null; $pending = $null
            switch -CaseSensitive ($map.presentation_status) {
                'window_presented' {
                    if ($map.presented_to_product_target -isnot [bool] -or -not $map.presented_to_product_target -or
                        $map.foundation_complete -isnot [bool] -or -not $map.foundation_complete -or
                        $map.execution_complete -isnot [bool] -or -not $map.execution_complete -or
                        $map.final_pixels_measured -isnot [bool] -or -not $map.final_pixels_measured) {
                        Stop-Vdp 'VDP_PRESENT_INCOMPLETE' 'Presented receipt does not confirm completed composition.'
                    }
                    $pending = $map; $presented++
                }
                { $_ -cin @('window_skipped', 'window_occluded') } {
                    if ($map.presented_to_product_target -isnot [bool] -or $map.presented_to_product_target) {
                        Stop-Vdp 'VDP_REJECTED_PRESENT_CLAIM' 'Skipped/occluded receipt claims a presented target.'
                    }
                    $rejected.Add($row)
                }
                default { Stop-Vdp 'VDP_NOT_ACTUAL_WINDOW_PRESENT' ([string]$map.presentation_status) }
            }
        } else {
            if ($null -eq $pending -or $map.presentation_status -cne 'window_presented' -or
                ($pending | ConvertTo-Json -Depth 100 -Compress) -cne ($map | ConvertTo-Json -Depth 100 -Compress)) {
                Stop-Vdp 'VDP_UI_PRESENT_MISMATCH' 'UI must follow the same complete finalized window receipt.'
            }
            if (($row.generation -isnot [long] -and $row.generation -isnot [int]) -or
                $row.generation -le $lastGeneration) {
                Stop-Vdp 'VDP_GENERATION_INVALID' 'Committed UI generation must be a strictly increasing positive integer.'
            }
            if ($frames.Count -eq 0 -and $map.hour_index -ne $Hour) { Stop-Vdp 'VDP_HOUR_MISMATCH' 'Initial presented canonical hour differs from the requested hour.' }
            $lastGeneration = [long]$row.generation
            $current = $row; $pending = $null
            $frames.Add($row)
        }
    }
    if ($frames.Count -gt 10000 -or $inputs.Count -gt 100) { Stop-Vdp 'VDP_RECEIPT_BOUND_EXCEEDED' 'Bounded desktop diagnostic stream exceeded its frame/input limit.' }
    [ordered]@{ current = $current; pending = $pending; last_generation = $lastGeneration; last_input = $lastInput
        presented_count = $presented; rejected = $rejected; frames = $frames; inputs = $inputs }
}

function Assert-VdpInteractionSeam {
    param($Evidence)
    if ($Evidence.presented_count -eq 0) {
        Stop-Vdp 'VDP_ACTUAL_PRESENT_RECEIPT_UNAVAILABLE' 'No durable actual WindowPresented receipt; startup is not playback.'
    }
    if ($null -eq $Evidence.current) {
        Stop-Vdp 'VDP_POSITIVE_GENERATION_UNAVAILABLE' 'No current post-Present UI generation; skipped/occluded generations do not authorize input.'
    }
    # These are a reservation contract, not inferred capabilities at f3227084.
    # Never extract country names/geometry/tooltip proof from snapshot_debug.
    $frame = $Evidence.current
    foreach ($field in @('input_receipt_schema', 'last_input_seq', 'playing', 'transport_index',
                        'selected_country', 'country_choices', 'production_lines', 'controls', 'tooltip', 'window_dpi')) {
        if ($frame.PSObject.Properties.Name -notcontains $field) {
            Stop-Vdp 'VDP_PUBLIC_INPUT_RECEIPT_UNAVAILABLE' "Missing committed UI projection: $field. Parent/ARROW owns the shared seam."
        }
    }
    if ($frame.input_receipt_schema -cne 'hoi4.viewer.desktop.input.v1') { Stop-Vdp 'VDP_PUBLIC_INPUT_RECEIPT_UNAVAILABLE' 'Input receipt capability absent.' }
    if ($frame.last_input_seq -ne $Evidence.last_input) { Stop-Vdp 'VDP_INPUT_SEQUENCE_MISMATCH' 'Presented frame does not correlate to the complete consumed-input stream.' }
    if (($frame.window_dpi -isnot [int] -and $frame.window_dpi -isnot [long]) -or $frame.window_dpi -le 0) { Stop-Vdp 'VDP_WINDOW_DPI_UNAVAILABLE' 'Require the actual viewer HWND DPI with committed geometry.' }
    if ($frame.playing -isnot [bool] -or $frame.playing) { Stop-Vdp 'VDP_TRANSPORT_NOT_PAUSED' 'Require a paused presented transport before scripted actions.' }
    if ($frame.production_lines.open -isnot [bool] -or $frame.production_lines.available -isnot [bool]) { Stop-Vdp 'VDP_PANEL_RECEIPT_INVALID' 'Panel state and availability must be typed booleans.' }
}

function Test-VdpSemanticReceipts {
    param($Receipts)
    if ($null -eq $Receipts -or $Receipts -isnot [array] -or $Receipts.Count -eq 0) { return $false }
    foreach ($receipt in $Receipts) {
        if ($receipt -isnot [string] -or [string]::IsNullOrWhiteSpace($receipt)) { return $false }
    }
    return $true
}

function Get-VdpHoverPoint {
    param($Frame, [string]$WindowId, [string]$ElementId)
    $matches = @($Frame.controls | Where-Object { $_.window_id -ceq $WindowId -and $_.element_id -ceq $ElementId })
    if ($matches.Count -ne 1) { Stop-Vdp 'VDP_CONTROL_UNAVAILABLE' 'Tooltip target must be unique in current committed geometry.' }
    $target = $matches[0]
    if ($target.generation -ne $Frame.generation -or $target.visible -isnot [bool] -or -not $target.visible -or
        $target.owns_pointer -isnot [bool] -or -not $target.owns_pointer -or
        [string]::IsNullOrWhiteSpace($target.runtime_instance_token) -or [string]::IsNullOrWhiteSpace($target.source_file) -or
        [string]::IsNullOrWhiteSpace($target.surface_id) -or $target.coordinate_space -cne 'win32_client_lparam') {
        Stop-Vdp 'VDP_CONTROL_GEOMETRY_UNAVAILABLE' 'Require current visible pointer-owning control with authenticated source and runtime instance.'
    }
    $rect = $target.client_rect; $clip = $target.clip_rect
    foreach ($box in @($rect, $clip)) {
        if (@($box).Count -ne 4) { Stop-Vdp 'VDP_CONTROL_GEOMETRY_INVALID' 'Rectangles must be physical client x/y/width/height.' }
        foreach ($n in $box) {
            if (($n -isnot [int] -and $n -isnot [long]) -or $n -lt -32768 -or $n -gt 32767) { Stop-Vdp 'VDP_CONTROL_GEOMETRY_INVALID' 'Win32 client coordinates must fit signed 16 bits.' }
        }
        if ($box[2] -le 0 -or $box[3] -le 0) { Stop-Vdp 'VDP_CONTROL_GEOMETRY_INVALID' 'Empty control or clip.' }
    }
    $left = [Math]::Max(0, [Math]::Max($rect[0], $clip[0])); $top = [Math]::Max(0, [Math]::Max($rect[1], $clip[1]))
    $right = [Math]::Min($Frame.map.viewport_width, [Math]::Min($rect[0] + $rect[2], $clip[0] + $clip[2]))
    $bottom = [Math]::Min($Frame.map.viewport_height, [Math]::Min($rect[1] + $rect[3], $clip[1] + $clip[3]))
    if ($right -le $left -or $bottom -le $top) { Stop-Vdp 'VDP_CONTROL_GEOMETRY_INVALID' 'Control is fully clipped.' }
    [ordered]@{ x = [int][Math]::Floor(($left + $right - 1) / 2); y = [int][Math]::Floor(($top + $bottom - 1) / 2); target = $target }
}

function Get-VdpSteps {
    param($Initial, [long]$DestinationHour, [string]$FirstTag, [string]$SecondTag)
    if ($DestinationHour -eq $Initial.map.hour_index) { Stop-Vdp 'VDP_SEEK_TARGET_INVALID' 'Seek must reach a different canonical hour.' }
    if ([string]::IsNullOrWhiteSpace($FirstTag) -or [string]::IsNullOrWhiteSpace($SecondTag) -or $FirstTag -ceq $SecondTag -or
        @($Initial.country_choices).Count -lt 2 -or $Initial.country_choices[0] -cne $FirstTag -or $Initial.country_choices[1] -cne $SecondTag) {
        Stop-Vdp 'VDP_COUNTRY_CHOICES_UNAVAILABLE' 'Keys 1/2 must name two distinct countries admitted in the presented frame.'
    }
    $countries = @(
        @{ name = 'country-first'; kind = 'key_down'; vk = 0x31; country = $FirstTag; route = 'country_slot' },
        @{ name = 'country-second'; kind = 'key_down'; vk = 0x32; country = $SecondTag; route = 'country_slot' })
    if ($Initial.selected_country -ceq $FirstTag) { [Array]::Reverse($countries) }
    @(
        @{ name = 'seek-forward'; kind = 'key_down'; vk = 0x27; hour = $DestinationHour; index = $Initial.transport_index + 1; route = 'transport.step_forward' },
        @{ name = 'seek-back'; kind = 'key_down'; vk = 0x25; hour = $Initial.map.hour_index; index = $Initial.transport_index; route = 'transport.step_backward' }
    ) + $countries + @(
        @{ name = 'panel-open'; kind = 'key_down'; vk = 0x50; open = $true; route = 'production_lines.toggle' },
        @{ name = 'tooltip'; kind = 'mouse_move'; route = 'authored_pointer_move' },
        @{ name = 'panel-close'; kind = 'key_down'; vk = 0x50; open = $false; route = 'production_lines.toggle' })
}

function Test-VdpStepResult {
    param($Evidence, $Action)
    if ($Evidence.rejected.Count -gt $Action.rejected_count) { Stop-Vdp 'VDP_INPUT_REJECTED_ROLLBACK' 'Actual skipped/occluded disposition followed the input attempt; it cannot complete this action.' }
    $newInputs = @($Evidence.inputs | Where-Object { $_.input_seq -gt $Action.input_seq_before })
    if ($newInputs.Count -eq 0) { return $false }
    if ($newInputs.Count -ne 1) { Stop-Vdp 'VDP_UNEXPECTED_INPUT' 'More than the one owned public-route input was consumed.' }
    $inputRow = $newInputs[0]; $step = $Action.step
    if ($inputRow.kind -cne $step.kind -or ($step.kind -ceq 'key_down' -and ($inputRow.vk -ne $step.vk -or $inputRow.shift -isnot [bool] -or $inputRow.shift))) {
        Stop-Vdp 'VDP_INPUT_ROUTE_MISMATCH' 'Consumed input differs from the posted public-route event.'
    }
    if ($inputRow.route -cne $step.route -or $inputRow.disposition -cne 'applied') { Stop-Vdp 'VDP_INPUT_NOT_APPLIED' 'Reached input did not apply through the inspected public consumer.' }
    $consumed = @($Evidence.frames | Where-Object { $_.generation -eq $inputRow.generation })
    if ($consumed.Count -ne 1 -or $consumed[0].generation -lt $Action.generation_before) { Stop-Vdp 'VDP_INPUT_GENERATION_MISMATCH' 'Consumed geometry has no current presented receipt.' }
    $before = $consumed[0]
    if ($before.playing -isnot [bool] -or $before.playing) { Stop-Vdp 'VDP_TRANSPORT_NOT_PAUSED' 'Transport was not paused at input consumption.' }
    if ($before.map.hour_index -ne $Action.hour_before -or $before.selected_country -cne $Action.country_before -or
        $before.production_lines.open -ne $Action.panel_before) { Stop-Vdp 'VDP_INPUT_STATE_CHANGED' 'Viewer state changed before the queued action was consumed.' }
    if ($step.kind -ceq 'mouse_move') {
        $point = Get-VdpHoverPoint $before $Action.point.target.window_id $Action.point.target.element_id
        if ($inputRow.x -ne $Action.point.x -or $inputRow.y -ne $Action.point.y -or $point.x -ne $Action.point.x -or $point.y -ne $Action.point.y -or
            $point.target.runtime_instance_token -cne $Action.point.target.runtime_instance_token -or
            $point.target.source_file -cne $Action.point.target.source_file -or
            $point.target.surface_id -cne $Action.point.target.surface_id -or
            ($point.target.child_path -join ',') -cne ($Action.point.target.child_path -join ',') -or
            ($point.target.client_rect -join ',') -cne ($Action.point.target.client_rect -join ',') -or
            ($point.target.clip_rect -join ',') -cne ($Action.point.target.clip_rect -join ',')) {
            Stop-Vdp 'VDP_STALE_CONTROL_GEOMETRY' 'The public pointer route consumed a different current target or rectangle.'
        }
        # The reached consumer's returned target owns the pointer. A rectangle
        # selected by this harness cannot substitute for that hit-test result.
        foreach ($field in @('window_id', 'element_id', 'runtime_instance_token', 'source_file', 'surface_id')) {
            if ($null -eq $inputRow.target -or $inputRow.target.$field -cne $point.target.$field) { Stop-Vdp 'VDP_POINTER_OWNER_MISMATCH' $field }
        }
        foreach ($field in @('client_rect', 'clip_rect', 'child_path')) {
            if (($inputRow.target.$field -join ',') -cne ($point.target.$field -join ',')) { Stop-Vdp 'VDP_POINTER_OWNER_MISMATCH' $field }
        }
    }
    $after = $Evidence.current
    if ($null -eq $after -or $after.generation -le $inputRow.generation -or $after.last_input_seq -lt $inputRow.input_seq) { return $false }
    if ($after.last_input_seq -ne $inputRow.input_seq) { Stop-Vdp 'VDP_UNEXPECTED_INPUT' 'A later unowned input changed the presented outcome.' }
    Assert-VdpInteractionSeam $Evidence
    if ($step.name -like 'seek-*') { return $after.map.hour_index -eq $step.hour -and $after.transport_index -eq $step.index }
    if ($after.map.hour_index -ne $Action.hour_before) { Stop-Vdp 'VDP_HOUR_CHANGED_DURING_UI' 'Non-seek interaction changed the canonical hour.' }
    if ($step.name -like 'country-*') {
        $slot = $step.vk - 0x31
        if (@($before.country_choices).Count -le $slot -or $before.country_choices[$slot] -cne $step.country) { Stop-Vdp 'VDP_COUNTRY_CHOICES_CHANGED' 'Consumed digit addresses a different admitted country.' }
        return $after.selected_country -ceq $step.country -and $after.selected_country -cne $Action.country_before
    }
    if ($after.selected_country -cne $Action.country_before) { Stop-Vdp 'VDP_COUNTRY_CHANGED_DURING_UI' 'Panel/tooltip changed the selected country.' }
    if ($step.name -like 'panel-*') {
        if ($after.production_lines.open -ne $step.open) { return $false }
        if ($step.open -and (-not $after.production_lines.available -or $after.production_lines.country -cne $after.selected_country -or -not (Test-VdpSemanticReceipts $after.production_lines.semantic_receipts))) {
            Stop-Vdp 'VDP_PANEL_NAMED_UNAVAILABLE' 'Panel has no available presented semantic receipt; raw feature gaps are retained.'
        }
        return $true
    }
    if ($step.name -ceq 'tooltip') {
        $tip = $after.tooltip
        if ($null -eq $tip) { return $false }
        $currentPoint = Get-VdpHoverPoint $after $Action.point.target.window_id $Action.point.target.element_id
        foreach ($field in @('window_id', 'element_id', 'runtime_instance_token', 'source_file', 'surface_id')) {
            if ($currentPoint.target.$field -cne $Action.point.target.$field) { Stop-Vdp 'VDP_TOOLTIP_RECEIPT_MISMATCH' "Current tooltip owner changed: $field" }
        }
        foreach ($field in @('child_path', 'client_rect', 'clip_rect')) {
            if (($currentPoint.target.$field -join ',') -cne ($Action.point.target.$field -join ',')) { Stop-Vdp 'VDP_TOOLTIP_RECEIPT_MISMATCH' "Current tooltip owner changed: $field" }
        }
        if ($tip.window_id -cne $Action.point.target.window_id -or $tip.widget_id -cne $Action.point.target.element_id -or
            $tip.runtime_instance_token -cne $Action.point.target.runtime_instance_token -or
            $tip.source_file -cne $currentPoint.target.source_file -or $tip.surface_id -cne $currentPoint.target.surface_id -or
            ($tip.child_path -join ',') -cne ($currentPoint.target.child_path -join ',') -or
            $tip.generation -ne $after.generation -or $tip.presented -isnot [bool] -or -not $tip.presented -or -not (Test-VdpSemanticReceipts $tip.semantic_receipts)) {
            Stop-Vdp 'VDP_TOOLTIP_RECEIPT_MISMATCH' 'Tooltip must be actually drawn for the current target/generation with semantic custody.'
        }
        return $true
    }
    Stop-Vdp 'VDP_STEP_UNSUPPORTED' $step.name
}

function Initialize-VdpWin32 {
    if ('ViewerDesktopValidationNative' -as [type]) { return }
    Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Text;
public static class ViewerDesktopValidationNative {
  [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left, Top, Right, Bottom; }
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassName(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out Rect r);
  [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr h);
  [DllImport("user32.dll")] static extern IntPtr GetWindowDpiAwarenessContext(IntPtr h);
  [DllImport("user32.dll")] static extern IntPtr SetThreadDpiAwarenessContext(IntPtr context);
  public static bool ReadClientForWindow(IntPtr h, out Rect r) {
    r = new Rect();
    IntPtr context = GetWindowDpiAwarenessContext(h);
    if (context == IntPtr.Zero) return false;
    IntPtr previous = SetThreadDpiAwarenessContext(context);
    if (previous == IntPtr.Zero) return false;
    try { return GetClientRect(h, out r); }
    finally { SetThreadDpiAwarenessContext(previous); }
  }
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr h);
  [DllImport("user32.dll")] public static extern short GetAsyncKeyState(int k);
  [DllImport("user32.dll", SetLastError=true)] public static extern bool PostMessage(IntPtr h, uint m, UIntPtr w, IntPtr l);
}
'@
}

function Send-VdpPublicInput {
    param($Child, $Record, [string]$Exe, $Frame, $Step, $Point, $Posting = $null)
    if ($null -eq $Posting) { $Posting = @{ input_posted = $false; messages_posted = 0 } }
    Assert-VdpOwnedProcess $Child $Record $Exe
    $handle = $Child.MainWindowHandle; [uint32]$ownerPid = 0
    $null = [ViewerDesktopValidationNative]::GetWindowThreadProcessId($handle, [ref]$ownerPid)
    $class = [Text.StringBuilder]::new(256)
    $null = [ViewerDesktopValidationNative]::GetClassName($handle, $class, $class.Capacity)
    $rect = [ViewerDesktopValidationNative+Rect]::new()
    if ($handle -eq [IntPtr]::Zero -or $ownerPid -ne $Record.pid -or $class.ToString() -cne 'replay_app_window' -or
        -not [ViewerDesktopValidationNative]::IsWindowVisible($handle) -or [ViewerDesktopValidationNative]::IsIconic($handle) -or
        -not [ViewerDesktopValidationNative]::ReadClientForWindow($handle, [ref]$rect) -or
        [ViewerDesktopValidationNative]::GetDpiForWindow($handle) -ne $Frame.window_dpi -or
        $rect.Right - $rect.Left -ne $Frame.map.viewport_width -or $rect.Bottom - $rect.Top -ne $Frame.map.viewport_height) {
        Stop-Vdp 'VDP_WINDOW_GEOMETRY_MISMATCH' 'Require the exact viewer HWND/class/PID and current physical client extent.'
    }
    foreach ($key in @(0x10, 0x11, 0x12)) {
        if ([ViewerDesktopValidationNative]::GetAsyncKeyState($key) -lt 0) { Stop-Vdp 'VDP_MODIFIER_HELD' 'Release modifiers before viewer validation.' }
    }
    # Target only this owned HWND. No global SendInput, focus stealing, the game
    # handles, gameplay commands, window activation, or guessed coordinates.
    if ($Step.kind -ceq 'key_down') {
        if ($Step.vk -notin @(0x27, 0x25, 0x31, 0x32, 0x50)) { Stop-Vdp 'VDP_INPUT_ROUTE_UNSUPPORTED' 'Key not in the inspected viewer route allowlist.' }
        if (-not [ViewerDesktopValidationNative]::PostMessage($handle, 0x100, [UIntPtr][uint32]$Step.vk, [IntPtr]1)) { Stop-Vdp 'VDP_INPUT_POST_FAILED' 'Viewer key-down was not queued.' }
        $Posting.messages_posted++; $Posting.input_posted = $true
        if (-not [ViewerDesktopValidationNative]::PostMessage($handle, 0x101, [UIntPtr][uint32]$Step.vk, [IntPtr](-1073741823))) { Stop-Vdp 'VDP_INPUT_POST_FAILED' 'Viewer key-up was not queued; key-down posting is retained.' }
        $Posting.messages_posted++
    } elseif ($Step.kind -ceq 'mouse_move') {
        $packed = ([long]$Point.y -shl 16) -bor ([long]$Point.x -band 65535)
        if (-not [ViewerDesktopValidationNative]::PostMessage($handle, 0x200, [UIntPtr]::Zero, [IntPtr]$packed)) { Stop-Vdp 'VDP_INPUT_POST_FAILED' 'Viewer pointer message was not queued.' }
        $Posting.messages_posted++; $Posting.input_posted = $true
    } else { Stop-Vdp 'VDP_INPUT_ROUTE_UNSUPPORTED' $Step.kind }
}

function Assert-VdpOwnedProcess {
    param($Child, $Record, [string]$Exe)
    $Child.Refresh()
    if ($Child.Id -ne $Record.pid -or $Child.StartTime.ToUniversalTime().ToString('o') -cne $Record.process_started_utc) {
        Stop-Vdp 'VDP_PROCESS_CUSTODY_LOST' 'PID/creation time no longer matches the launched viewer.'
    }
    if (-not $Child.HasExited -and [IO.Path]::GetFullPath($Child.MainModule.FileName) -ine $Exe) {
        Stop-Vdp 'VDP_PROCESS_IMAGE_MISMATCH' 'Owned process image differs from pinned viewer.'
    }
}

function Read-VdpSharedText {
    param([string]$Path)
    $stream = [IO.File]::Open($Path, 'Open', 'Read', 'ReadWrite')
    try {
        if ($stream.Length -gt 64MB) { Stop-Vdp 'VDP_LOG_BOUND_EXCEEDED' $Path }
        $reader = [IO.StreamReader]::new($stream)
        try { $reader.ReadToEnd() } finally { $reader.Dispose() }
    } finally { $stream.Dispose() }
}

function Read-VdpStreamChunk {
    param([string]$Path, $Cursor, [switch]$Complete)
    $stream = [IO.File]::Open($Path, 'Open', 'Read', 'ReadWrite')
    try {
        $length = $stream.Length
        if ($length -gt 64MB) { Stop-Vdp 'VDP_LOG_BOUND_EXCEEDED' $Path }
        if ($length -lt $Cursor.offset) { Stop-Vdp 'VDP_LOG_TRUNCATED' 'Owned raw log was truncated.' }
        $count = [int]($length - $Cursor.offset)
        $buffer = [byte[]]::new($Cursor.tail.Length + $count)
        [Array]::Copy($Cursor.tail, $buffer, $Cursor.tail.Length)
        $null = $stream.Seek($Cursor.offset, 'Begin')
        $read = 0
        while ($read -lt $count) {
            $n = $stream.Read($buffer, $Cursor.tail.Length + $read, $count - $read)
            if ($n -eq 0) { Stop-Vdp 'VDP_LOG_TRUNCATED' 'Raw log changed while reading.' }
            $read += $n
        }
        $Cursor.offset = $length
        $end = $buffer.Length - 1
        while ($end -ge 0 -and $buffer[$end] -ne 10) { $end-- }
        $Cursor.tail = [byte[]]::new($buffer.Length - $end - 1)
        [Array]::Copy($buffer, $end + 1, $Cursor.tail, 0, $Cursor.tail.Length)
        $utf8 = [Text.UTF8Encoding]::new($false, $true)
        if ($end -ge 0) { Read-VdpJsonLines ($utf8.GetString($buffer, 0, $end + 1)) }
        if ($Complete -and $Cursor.tail.Length -gt 0) { Read-VdpJsonLines ($utf8.GetString($Cursor.tail)) -Complete }
    } finally { $stream.Dispose() }
}

function Wait-VdpPipeDrain {
    param([object[]]$Tasks)
    $drainTimer = [Diagnostics.Stopwatch]::StartNew()
    foreach ($task in $Tasks) {
        if ($null -eq $task) { continue }
        $remaining = [Math]::Max(0, 5000 - [int]$drainTimer.ElapsedMilliseconds)
        try { $done = $task.Wait($remaining) }
        catch { Stop-Vdp 'VDP_PIPE_DRAIN_FAILED' $_.Exception.Message }
        if (-not $done) { Stop-Vdp 'VDP_PIPE_DRAIN_TIMEOUT' 'Raw pipe drain exceeded its five-second total bound; logs may be incomplete.' }
    }
}

function Invoke-VdpProcess {
    param($Case, [string]$Exe, [string]$Root, [int]$Timeout, $Authority, [long]$Hour,
        [Collections.Generic.List[object]]$Cases)
    $desktop = $Case.name -ceq 'desktop'
    $record = [ordered]@{ name = $Case.name; executable = $Exe; argv = $Case.argv
        pid = $null; process_started_utc = $null; exit_code = $null
        window_observed = $false; window_handle = $null; window_title = $null
        close_requested = $false; forced_exit = $false; error = $null
        stdout = $null; stderr = $null; desktop_evidence = $null; actions = @(); playback_qualified = $false
        pipe_drain_complete = $false; cleanup_errors = @() }
    $Cases.Add($record)
    $child = $null; $outStream = $null; $errStream = $null; $outTask = $null; $errTask = $null
    $stdout = Join-Path $Root ($Case.name + '.stdout.txt')
    $stderr = Join-Path $Root ($Case.name + '.stderr.txt')
    $cursor = @{ offset = 0L; tail = [byte[]]::new(0) }; $evidence = $null
    $actions = [Collections.Generic.List[object]]::new(); $steps = $null; $activeAction = $null
    $stepIndex = 0; $complete = $false
    $drainAttempted = $false
    try {
        if (@(Get-Process -Name the game -ErrorAction SilentlyContinue).Count -gt 0) {
            Stop-Vdp 'VDP_HOI4_RUNNING' 'This bounded viewer-only lane requires no running HOI4.'
        }
        $info = [Diagnostics.ProcessStartInfo]::new()
        $info.FileName = $Exe
        $info.Arguments = ($Case.argv | ForEach-Object { ConvertTo-VdpArgument $_ }) -join ' '
        $info.WorkingDirectory = $Root; $info.UseShellExecute = $false
        $info.CreateNoWindow = -not $desktop
        $info.RedirectStandardOutput = $true; $info.RedirectStandardError = $true
        foreach ($key in @($info.EnvironmentVariables.Keys)) {
            if ([string]$key -match '^(HOI4_|CARGO|RUST|SCCACHE|REPLAY_DESKTOP_)') { $info.EnvironmentVariables.Remove([string]$key) }
        }
        $info.EnvironmentVariables['PATH'] = Join-Path $env:SystemRoot 'System32'
        $info.EnvironmentVariables['LOCALAPPDATA'] = Join-Path $Root 'local-app-data'
        $info.EnvironmentVariables['TEMP'] = $Root; $info.EnvironmentVariables['TMP'] = $Root
        if ($desktop) { $info.EnvironmentVariables['REPLAY_DESKTOP_DIAGNOSTICS'] = '1' }
        $outStream = [IO.FileStream]::new($stdout, 'CreateNew', 'Write', 'Read', 1, $true)
        $errStream = [IO.FileStream]::new($stderr, 'CreateNew', 'Write', 'Read', 1, $true)
        $child = [Diagnostics.Process]::new(); $child.StartInfo = $info
        if (-not $child.Start()) { Stop-Vdp 'VDP_PROCESS_START_FAILED' $Case.name }
        $record.pid = $child.Id
        $record.process_started_utc = $child.StartTime.ToUniversalTime().ToString('o')
        $outTask = $child.StandardOutput.BaseStream.CopyToAsync($outStream)
        $errTask = $child.StandardError.BaseStream.CopyToAsync($errStream)
        $timer = [Diagnostics.Stopwatch]::StartNew()
        $deadline = [double]$Timeout
        while (-not $child.HasExited) {
            Assert-VdpOwnedProcess $child $record $Exe
            if (@(Get-Process -Name the game -ErrorAction SilentlyContinue).Count -gt 0) { Stop-Vdp 'VDP_HOI4_RUNNING' 'the game appeared during viewer validation.' }
            if ($outStream.Length -gt 64MB -or $errStream.Length -gt 64MB) { Stop-Vdp 'VDP_LOG_BOUND_EXCEEDED' $Case.name }
            if ($desktop -and $child.MainWindowHandle -ne [IntPtr]::Zero -and $child.Responding) {
                $record.window_observed = $true; $record.window_title = $child.MainWindowTitle
                $record.window_handle = $child.MainWindowHandle.ToInt64()
                $newRows = @(Read-VdpStreamChunk $stderr $cursor)
                $evidence = Get-VdpDesktopEvidence $newRows $record.pid $Authority $Hour $evidence
                if ($null -ne $evidence.current) {
                    if ($null -eq $steps) {
                        Assert-VdpInteractionSeam $evidence
                        if ($evidence.inputs.Count -ne 0) { Stop-Vdp 'VDP_UNEXPECTED_INPUT' 'Unowned input was consumed before the harness started.' }
                        if ($evidence.current.production_lines.open) { Stop-Vdp 'VDP_PANEL_INITIAL_STATE' 'Production lines must initially be closed.' }
                        $steps = @(Get-VdpSteps $evidence.current $SeekHour $FirstCountryTag $SecondCountryTag)
                        Initialize-VdpWin32
                    }
                    if ($null -ne $activeAction -and (Test-VdpStepResult $evidence $activeAction)) {
                        $activeAction.status = 'observed_presented'; $activeAction['result_generation'] = $evidence.current.generation
                        $activeAction['result_hour'] = $evidence.current.map.hour_index
                        $activeAction['result_receipt_sha256'] = $evidence.current.map.deterministic_sha256
                        $activeAction['result_ui'] = $evidence.current
                        $stepIndex++; $activeAction = $null
                        $complete = $stepIndex -eq $steps.Count
                    }
                    if (-not $complete -and $null -eq $activeAction) {
                        Assert-VdpInteractionSeam $evidence
                        $frame = $evidence.current; $step = $steps[$stepIndex]; $point = $null
                        if ($step.kind -ceq 'mouse_move') { $point = Get-VdpHoverPoint $frame $TooltipWindowId $TooltipElementId }
                        $activeAction = [ordered]@{ step = $step; status = 'posting_unproven'; point = $point
                            input_posted = $false; messages_posted = 0
                            generation_before = $frame.generation; input_seq_before = $frame.last_input_seq
                            hour_before = $frame.map.hour_index; country_before = $frame.selected_country
                            panel_before = $frame.production_lines.open; rejected_count = $evidence.rejected.Count }
                        $actions.Add($activeAction)
                        Send-VdpPublicInput $child $record $Exe $frame $step $point $activeAction
                        $activeAction.status = 'queued_unproven'
                        $deadline = $timer.Elapsed.TotalSeconds + $Timeout
                    }
                }
                if ($null -ne $activeAction -and $evidence.rejected.Count -gt $activeAction.rejected_count) {
                    Stop-Vdp 'VDP_INPUT_REJECTED_ROLLBACK' 'Actual skipped/occluded frame after queued input; raw rejection retained.'
                }
            }
            if ($complete -or $timer.Elapsed.TotalSeconds -ge $deadline) {
                if (-not $desktop) { Stop-Vdp 'VDP_CLI_TIMEOUT' $Case.name }
                if (-not $record.window_observed) { Stop-Vdp 'VDP_WINDOW_UNAVAILABLE' 'No responsive owned viewer window.' }
                if (-not $complete) {
                    if ($null -eq $steps) { Assert-VdpInteractionSeam $evidence }
                    Stop-Vdp 'VDP_INTERACTION_TIMEOUT' "No actual presented result for $($activeAction.step.name)."
                }
                $record.close_requested = $child.CloseMainWindow()
                if (-not $record.close_requested) { Stop-Vdp 'VDP_COOPERATIVE_CLOSE_FAILED' 'Viewer did not accept close.' }
                if (-not $child.WaitForExit(5000)) { Stop-Vdp 'VDP_CLOSE_TIMEOUT' 'Viewer did not close within five seconds.' }
                break
            }
            Start-Sleep -Milliseconds 100
        }
        if (-not $child.HasExited) { Stop-Vdp 'VDP_PROCESS_EXIT_UNOBSERVED' 'No bounded process exit was observed.' }
        $record.exit_code = $child.ExitCode
        $drainAttempted = $true
        Wait-VdpPipeDrain @($outTask, $errTask)
        $record.pipe_drain_complete = $true
        $outStream.Flush(); $errStream.Flush()
        if ($record.exit_code -ne 0) { Stop-Vdp 'VDP_PRODUCT_EXIT_FAILED' "$($Case.name) exit $($record.exit_code); retain raw diagnostics." }
        if ($desktop) {
            if (-not $record.window_observed -or -not $record.close_requested) { Stop-Vdp 'VDP_DESKTOP_EXIT_WITHOUT_PLAYBACK' 'Viewer exited without bounded responsive-window custody and cooperative close.' }
            $rows = @(Read-VdpStreamChunk $stderr $cursor -Complete)
            $evidence = Get-VdpDesktopEvidence $rows $record.pid $Authority $Hour $evidence
            if (-not $complete -or $actions.Count -ne 7) { Stop-Vdp 'VDP_INTERACTIONS_INCOMPLETE' 'Startup or partial actions cannot qualify playback.' }
            # Recheck the final action after draining stdout/stderr so a buffered
            # reject or foreign input cannot be hidden behind graceful close.
            if (-not (Test-VdpStepResult $evidence $actions[$actions.Count - 1])) { Stop-Vdp 'VDP_FINAL_GENERATION_UNAVAILABLE' 'Final presented result was not retained through graceful exit.' }
            $record.playback_qualified = $true
        } elseif ($Case.name -ceq 'validate-playback') {
            Assert-VdpWholePlayback (Read-VdpSharedText $stdout)
        }
    } catch {
        $record.error = $_.Exception.Message
        throw
    } finally {
        $record.actions = @($actions.ToArray())
        if ($null -ne $evidence) {
            $record.desktop_evidence = [ordered]@{ current = $evidence.current; presented_count = $evidence.presented_count
                rejected = @($evidence.rejected.ToArray()); input_count = $evidence.inputs.Count }
        }
        if ($null -ne $child -and $null -ne $record.pid) {
            if (-not $child.HasExited) {
                Assert-VdpOwnedProcess $child $record $Exe
                if ($desktop -and $child.MainWindowHandle -ne [IntPtr]::Zero) {
                    $record.close_requested = $child.CloseMainWindow()
                    $null = $child.WaitForExit(5000)
                }
                if (-not $child.HasExited) {
                    Assert-VdpOwnedProcess $child $record $Exe
                    $record.forced_exit = $true; $child.Kill()
                    if (-not $child.WaitForExit(5000)) { Stop-Vdp 'VDP_OWNED_EXIT_TIMEOUT' 'Owned viewer did not exit after termination.' }
                }
            }
            $record.exit_code = $child.ExitCode
        }
        if (-not $drainAttempted) {
            try { Wait-VdpPipeDrain @($outTask, $errTask); $record.pipe_drain_complete = $true }
            catch { $record.cleanup_errors += $_.Exception.Message; $record.playback_qualified = $false }
        }
        if ($null -ne $outStream) { $outStream.Dispose() }
        if ($null -ne $errStream) { $errStream.Dispose() }
        if ($null -ne $child) { $child.Dispose() }
        if (Test-Path -LiteralPath $stdout) { $record.stdout = Get-VdpReference $stdout }
        if (Test-Path -LiteralPath $stderr) { $record.stderr = Get-VdpReference $stderr }
    }
}

function Invoke-VdpMain {
    $repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../../..'))
    if ([string]::IsNullOrWhiteSpace($OutputDirectory)) { Stop-Vdp 'VDP_OUTPUT_REQUIRED' 'Supply a new directory on C: outside the repo.' }
    $root = [IO.Path]::GetFullPath($OutputDirectory)
    if (-not $root.StartsWith('C:\', [StringComparison]::OrdinalIgnoreCase) -or
        $root -ieq $repoRoot -or $root.StartsWith($repoRoot.TrimEnd('\') + '\', [StringComparison]::OrdinalIgnoreCase)) {
        Stop-Vdp 'VDP_OUTPUT_LOCATION_INVALID' 'Use C: scratch outside the repository.'
    }
    # Prevent junctions from redirecting scratch writes back into a repository.
    for ($ancestor = [IO.DirectoryInfo]::new($root); $null -ne $ancestor; $ancestor = $ancestor.Parent) {
        if ($ancestor.Exists -and ($ancestor.Attributes -band [IO.FileAttributes]::ReparsePoint)) { Stop-Vdp 'VDP_OUTPUT_REPARSE_POINT' $ancestor.FullName }
    }
    if (Test-Path -LiteralPath $root) { Stop-Vdp 'VDP_OUTPUT_EXISTS' 'Use a fresh directory; prior evidence is retained.' }
    $null = New-Item -ItemType Directory -Path $root
    $cases = [Collections.Generic.List[object]]::new()
    $locks = [Collections.Generic.List[object]]::new()
    $receipt = [ordered]@{ schema = 'hoi4.viewer-desktop-playback-validation.v1'; status = 'fail'
        mode = $Mode; started_utc = [DateTime]::UtcNow.ToString('o'); machine = [Environment]::MachineName
        non_elevated = $false; viewer = $null; recording = $null; content_authority = $null
        expected_hour = $ExpectedHour; plan = @(); cases = @(); error = $null
        playback_qualified = $false; inputs_sent = 0
        missing_interactions = @('seek', 'country', 'panel', 'tooltip')
        limitations = @('At f3227084 the required diagnostic/input seam is unavailable; Run fails by name.',
            'Single recording only; no package-set, surviving-line, performance, fidelity or clean-machine qualification.') }
    try {
        foreach ($value in @($ViewerExecutable, $RecordingPath, $GameRoot, $UserDataRoot)) {
            if ([string]::IsNullOrWhiteSpace($value)) { Stop-Vdp 'VDP_INPUT_REQUIRED' 'Explicit viewer, recording, game and user-data roots are required.' }
        }
        if ($null -eq $ExpectedHour) { Stop-Vdp 'VDP_HOUR_REQUIRED' 'Supply the exact initial canonical hour.' }
        if ($null -eq $SeekHour) { Stop-Vdp 'VDP_SEEK_HOUR_REQUIRED' 'Supply the next recorded canonical hour for the public Right-key route.' }
        foreach ($value in @($FirstCountryTag, $SecondCountryTag, $TooltipWindowId, $TooltipElementId)) {
            if ([string]::IsNullOrWhiteSpace($value)) { Stop-Vdp 'VDP_INTERACTION_TARGET_REQUIRED' 'Supply two admitted country tags and the tooltip window/element identifiers.' }
        }
        Assert-VdpSha256 $ExpectedViewerSha256 'viewer'; Assert-VdpSha256 $ExpectedRecordingSha256 'recording'
        Assert-VdpSha256 $ExpectedContentAuthoritySha256 'content authority'
        $exe = [IO.Path]::GetFullPath($ViewerExecutable); $recording = [IO.Path]::GetFullPath($RecordingPath)
        if ([IO.Path]::GetExtension($recording) -ine '.battleplan') { Stop-Vdp 'VDP_RECORDING_SET_UNSUPPORTED' 'All-member package-set custody is required before supporting rollover.' }
        if ([IO.Path]::GetExtension($exe) -ine '.exe' -or [IO.Path]::GetFileNameWithoutExtension($exe) -ieq 'the game') { Stop-Vdp 'VDP_VIEWER_IMAGE_REQUIRED' 'Supply the parent-pinned replay viewer executable.' }
        foreach ($path in @($exe, $recording)) { $locks.Add([IO.File]::Open($path, 'Open', 'Read', 'Read')) }
        $receipt.viewer = Get-VdpReference $exe; $receipt.recording = Get-VdpReference $recording
        if ($receipt.viewer.sha256 -cne $ExpectedViewerSha256) { Stop-Vdp 'VDP_VIEWER_HASH_MISMATCH' $exe }
        if ($receipt.recording.sha256 -cne $ExpectedRecordingSha256) { Stop-Vdp 'VDP_RECORDING_HASH_MISMATCH' $recording }
        foreach ($path in @($GameRoot, $UserDataRoot)) {
            if (-not (Test-Path -LiteralPath $path -PathType Container)) { Stop-Vdp 'VDP_CONTENT_ROOT_MISSING' $path }
        }
        $authorityPath = Join-Path $root 'content-authority.json'
        $receipt.plan = @(Get-VdpCliPlan $recording ([IO.Path]::GetFullPath($GameRoot)) ([IO.Path]::GetFullPath($UserDataRoot)) $authorityPath)
        if ($Mode -ceq 'Prepare') {
            $receipt.status = 'prepared_not_run'
        } else {
            $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
            try {
                $principal = [Security.Principal.WindowsPrincipal]::new($identity)
                $receipt.non_elevated = -not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
                $receipt['user_sid'] = $identity.User.Value
            } finally { $identity.Dispose() }
            if (-not $receipt.non_elevated) { Stop-Vdp 'VDP_ELEVATED_TOKEN' 'Run under a non-elevated Windows token.' }
            $authority = $null
            foreach ($case in $receipt.plan) {
                Invoke-VdpProcess $case $exe $root $TimeoutSeconds $authority $ExpectedHour $cases
                if ($case.name -ceq 'content-authority') {
                    $authority = Assert-VdpAuthority (Get-Content -LiteralPath $authorityPath -Raw | ConvertFrom-Json) $recording $ExpectedContentAuthoritySha256
                    $receipt.content_authority = Get-VdpReference $authorityPath
                }
            }
            $desktopCase = @($cases | Where-Object { $_.name -ceq 'desktop' })
            if ($desktopCase.Count -ne 1 -or -not $desktopCase[0].playback_qualified -or $desktopCase[0].forced_exit) { Stop-Vdp 'VDP_INTERACTIONS_INCOMPLETE' 'No qualified desktop sequence.' }
            $receipt.playback_qualified = $true; $receipt.status = 'pass'; $receipt.missing_interactions = @()
        }
    } catch {
        $receipt.error = $_.Exception.Message
    } finally {
        foreach ($handle in $locks) { $handle.Dispose() }
        $receipt.inputs_sent = @($cases | ForEach-Object { $_.actions } | Where-Object { $null -ne $_ -and $_.input_posted }).Count
        $receipt.cases = @($cases.ToArray()); $receipt['finished_utc'] = [DateTime]::UtcNow.ToString('o')
        [IO.File]::WriteAllText((Join-Path $root 'DESKTOP_PLAYBACK.json'), ($receipt | ConvertTo-Json -Depth 100))
        Write-Output ($receipt | ConvertTo-Json -Depth 100 -Compress)
    }
    if ($receipt.status -ceq 'fail') { return 1 }
    return 0
}

# Offline tests load only the functions. Dot-sourcing cannot launch a product.
if ($MyInvocation.InvocationName -ne '.') {
    $result = @(Invoke-VdpMain)
    $result | Select-Object -SkipLast 1 | Write-Output
    exit [int]$result[-1]
}
