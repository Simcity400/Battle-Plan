<# Offline owning tests. No product, capture, build, desktop window or input.
Synthetic rows exercise the harness predicate only; they are not product evidence.
Run: powershell.exe -NoProfile -ExecutionPolicy Bypass -File <this> -OutputDirectory <fresh C: scratch>
#>
[CmdletBinding()]
param([Parameter(Mandatory = $true)][string]$OutputDirectory)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$testRoot = [IO.Path]::GetFullPath($OutputDirectory)
if (Test-Path -LiteralPath $testRoot) { throw 'Use new test scratch.' }
$null = New-Item -ItemType Directory -Path $testRoot
$scriptUnderTest = Join-Path $PSScriptRoot 'Invoke-ViewerDesktopPlaybackValidation.ps1'
. $scriptUnderTest
$passes = 0; $failures = [Collections.Generic.List[string]]::new()

function Assert-Test([bool]$Condition, [string]$Message = 'Assertion failed') {
    if (-not $Condition) { throw $Message }
}
function Assert-Refusal([string]$Code, [scriptblock]$Body) {
    try { & $Body | Out-Null } catch {
        if ($_.Exception.Message.StartsWith($Code + ':', [StringComparison]::Ordinal)) { return }
        throw "Expected $Code, got $($_.Exception.Message)"
    }
    throw "Expected refusal $Code; predicate accepted."
}
function Invoke-Test([string]$Name, [scriptblock]$Body) {
    try { & $Body; $script:passes++; Write-Output "ok fixture: $Name" }
    catch { $script:failures.Add("$Name :: $($_.Exception.Message)"); Write-Output "FAIL fixture: $Name :: $($_.Exception.Message)" }
}
function Copy-Fixture($Value) { $Value | ConvertTo-Json -Depth 100 | ConvertFrom-Json }
$authority = Copy-Fixture @{ manifest_digest_sha256 = 'a' * 64; product_modset_sha256 = 'b' * 64; effective_vfs_manifest_sha256 = 'c' * 64 }
function New-Frame([long]$Generation = 1, [long]$Hour = 100, [long]$InputSeq = 0, [string]$Country = '', [bool]$Panel = $false) {
    Copy-Fixture @{
        schema = 'hoi4.viewer.desktop.ui.v1'; pid = 123; generation = $Generation; input_transition_generation = 0
        input_receipt_schema = 'hoi4.viewer.desktop.input.v1'; last_input_seq = $InputSeq
        playing = $false; window_dpi = 144; transport_index = $(if ($Hour -eq 100) { 0 } else { 1 })
        selected_country = $Country; country_choices = @('AAA', 'BBB')
        production_lines = @{ open = $Panel; available = $Panel; country = $Country; semantic_receipts = @('fixture-semantic'); gaps = @() }
        tooltip = $null; semantic_receipts = @('fixture-semantic'); feature_local_gaps = @('fixture named unknown'); gaps_debug = 'fixture-only gap'
        controls = @(@{ generation = $Generation; window_id = 'production'; element_id = 'row'; runtime_instance_token = 'runtime-1'
            source_file = 'interface/production.gui'; surface_id = 'production'; child_path = @(0, 2); coordinate_space = 'win32_client_lparam'
            visible = $true; owns_pointer = $true; client_rect = @(10, 20, 50, 30); clip_rect = @(30, 10, 50, 50) })
        map = @{ schema = 'hoi4.replay-map-composition.xmap22.v7'; presentation_status = 'window_presented'
            hour_index = $Hour; viewport_width = 800; viewport_height = 600
            content_manifest_sha256 = 'a' * 64; product_modset_sha256 = 'b' * 64; effective_vfs_manifest_sha256 = 'c' * 64
            deterministic_sha256 = 'd' * 64; replay_frame_identity_sha256 = 'e' * 64; viewer_state_sha256 = 'f' * 64
            presented_to_product_target = $true; foundation_complete = $true; execution_complete = $true; final_pixels_measured = $true
            element_diagnostic_census = @(@{ gap_id = 'fixture-only'; reason = 'unknown recorded value' }) }
    }
}
function Frame-Rows($Frame) {
    @((Copy-Fixture @{ schema = 'hoi4.viewer.desktop.present.v1'; pid = 123; map = $Frame.map }), $Frame)
}
function New-Evidence($Frame) { Get-VdpDesktopEvidence (Frame-Rows $Frame) 123 $authority 100 }
function New-Action($Before, $Step, $Evidence) {
    [ordered]@{ step = $Step; status = 'queued_unproven'; point = $null
        generation_before = $Before.generation; input_seq_before = $Before.last_input_seq
        hour_before = $Before.map.hour_index; country_before = $Before.selected_country
        panel_before = $Before.production_lines.open; rejected_count = $Evidence.rejected.Count }
}
function New-Input($Action, $Before) {
    $data = @{ schema = 'hoi4.viewer.desktop.input.v1'; pid = 123; input_seq = $Before.last_input_seq + 1
        generation = $Before.generation; kind = $Action.step.kind; route = $Action.step.route; disposition = 'applied' }
    if ($Action.step.kind -ceq 'key_down') { $data.vk = $Action.step.vk; $data.shift = $false }
    else { $data.x = $Action.point.x; $data.y = $Action.point.y; $data.target = $Action.point.target }
    Copy-Fixture $data
}

Invoke-Test 'empty/startup/debug summaries never prove presentation' {
    $rows = @(Read-VdpJsonLines "replay_app: XMAP22 shared window composition WindowPresented: receipt_sha256=foo`n")
    Assert-Test ($rows.Count -eq 0)
    Assert-Refusal 'VDP_ACTUAL_PRESENT_RECEIPT_UNAVAILABLE' { Assert-VdpInteractionSeam (Get-VdpDesktopEvidence $rows 123 $authority 100) }
}
Invoke-Test 'actual present alone has no UI generation' {
    $rows = Frame-Rows (New-Frame)
    Assert-Refusal 'VDP_POSITIVE_GENERATION_UNAVAILABLE' { Assert-VdpInteractionSeam (Get-VdpDesktopEvidence @($rows[0]) 123 $authority 100) }
}
Invoke-Test 'minimal proposed seam still refuses missing public-input projection' {
    $frame = New-Frame; $frame.PSObject.Properties.Remove('controls')
    Assert-Refusal 'VDP_PUBLIC_INPUT_RECEIPT_UNAVAILABLE' { Assert-VdpInteractionSeam (New-Evidence $frame) }
}
Invoke-Test 'process/content/hour/full-map mismatches rejected' {
    $rows = Frame-Rows (New-Frame); $rows[1].pid = 124
    Assert-Refusal 'VDP_PROCESS_RECEIPT_MISMATCH' { Get-VdpDesktopEvidence $rows 123 $authority 100 }
    $rows = Frame-Rows (New-Frame); $rows[0].map.content_manifest_sha256 = 'b' * 64
    Assert-Refusal 'VDP_FRAME_CONTENT_MISMATCH' { Get-VdpDesktopEvidence $rows 123 $authority 100 }
    Assert-Refusal 'VDP_HOUR_MISMATCH' { New-Evidence (New-Frame 1 101) }
    $rows = Frame-Rows (New-Frame); $rows[1].map.viewer_state_sha256 = '0' * 64
    Assert-Refusal 'VDP_UI_PRESENT_MISMATCH' { Get-VdpDesktopEvidence $rows 123 $authority 100 }
}
Invoke-Test 'zero negative fractional and replayed generations rejected' {
    foreach ($generation in @(0, -1, 1.5)) {
        $frame = New-Frame; $frame.generation = $generation
        Assert-Refusal 'VDP_GENERATION_INVALID' { New-Evidence $frame }
    }
    $rows = @(Frame-Rows (New-Frame)) + @(Frame-Rows (New-Frame))
    Assert-Refusal 'VDP_GENERATION_INVALID' { Get-VdpDesktopEvidence $rows 123 $authority 100 }
}
Invoke-Test 'offscreen pending and malformed positive booleans rejected' {
    foreach ($status in @('offscreen_readback_measured', 'window_pending_present')) {
        $rows = Frame-Rows (New-Frame); $rows[0].map.presentation_status = $status
        Assert-Refusal 'VDP_NOT_ACTUAL_WINDOW_PRESENT' { Get-VdpDesktopEvidence @($rows[0]) 123 $authority 100 }
    }
    $rows = Frame-Rows (New-Frame); $rows[0].map.presented_to_product_target = 'true'
    Assert-Refusal 'VDP_PRESENT_INCOMPLETE' { Get-VdpDesktopEvidence @($rows[0]) 123 $authority 100 }
}
Invoke-Test 'occlusion and skip retain rejects and revoke current eligibility' {
    foreach ($status in @('window_occluded', 'window_skipped')) {
        $first = New-Frame; $rows = Frame-Rows (New-Frame 2)
        $rows[0].map.presentation_status = $status; $rows[0].map.presented_to_product_target = $false
        $ev = Get-VdpDesktopEvidence (@(Frame-Rows $first) + @($rows[0])) 123 $authority 100
        Assert-Test ($ev.rejected.Count -eq 1 -and $null -eq $ev.current)
        Assert-Refusal 'VDP_POSITIVE_GENERATION_UNAVAILABLE' { Assert-VdpInteractionSeam $ev }
        Assert-Refusal 'VDP_UI_PRESENT_MISMATCH' { Get-VdpDesktopEvidence @($rows[1]) 123 $authority 100 $ev }
    }
}
Invoke-Test 'explicit later presentation restores eligibility; gaps remain unchanged' {
    $rows = Frame-Rows (New-Frame 2); $rows[0].map.presentation_status = 'window_occluded'; $rows[0].map.presented_to_product_target = $false
    $ev = Get-VdpDesktopEvidence (@(Frame-Rows (New-Frame)) + @($rows[0]) + @(Frame-Rows (New-Frame 3))) 123 $authority 100
    Assert-VdpInteractionSeam $ev
    Assert-Test ($ev.current.generation -eq 3 -and $ev.rejected.Count -eq 1)
    Assert-Test ($ev.current.feature_local_gaps[0] -ceq 'fixture named unknown')
    Assert-Test ($ev.current.map.element_diagnostic_census[0].reason -ceq 'unknown recorded value')
}
Invoke-Test 'seven-step sparse-hour sequence needs input and later actual present' {
    $frame = New-Frame; $ev = New-Evidence $frame
    $steps = @(Get-VdpSteps $frame 160 'AAA' 'BBB')
    Assert-Test ($steps.Count -eq 7)
    foreach ($step in $steps) {
        $action = New-Action $frame $step $ev
        if ($step.kind -ceq 'mouse_move') { $action.point = Get-VdpHoverPoint $frame 'production' 'row' }
        Assert-Test (-not (Test-VdpStepResult $ev $action)) 'Liveness without consumed input passed.'
        $inputRow = New-Input $action $frame
        $ev = Get-VdpDesktopEvidence @($inputRow) 123 $authority 100 $ev
        Assert-Test (-not (Test-VdpStepResult $ev $action)) 'Consumed input without a later Present passed.'
        $after = New-Frame ($frame.generation + 1) $frame.map.hour_index $inputRow.input_seq $frame.selected_country $frame.production_lines.open
        if ($step.name -like 'seek-*') { $after.map.hour_index = $step.hour; $after.transport_index = $step.index }
        if ($step.name -like 'country-*') { $after.selected_country = $step.country; $after.production_lines.country = $step.country }
        if ($step.name -like 'panel-*') { $after.production_lines.open = $step.open; $after.production_lines.available = $step.open }
        if ($step.name -ceq 'tooltip') {
            $after.tooltip = Copy-Fixture @{ window_id = 'production'; widget_id = 'row'; runtime_instance_token = 'runtime-1'
                source_file = 'interface/production.gui'; surface_id = 'production'; child_path = @(0, 2)
                generation = $after.generation; presented = $true; semantic_receipts = @('fixture tooltip composition') }
        }
        $ev = Get-VdpDesktopEvidence (Frame-Rows $after) 123 $authority 100 $ev
        Assert-Test (Test-VdpStepResult $ev $action) $step.name
        $frame = $after
    }
    Assert-Test ($ev.inputs.Count -eq 7 -and $ev.current.generation -eq 8)
}
Invoke-Test 'country routes avoid initial no-op and reject missing slots' {
    $frame = New-Frame 1 100 0 'AAA'; $steps = @(Get-VdpSteps $frame 160 'AAA' 'BBB')
    Assert-Test ($steps[2].vk -eq 0x32 -and $steps[3].vk -eq 0x31)
    Assert-Refusal 'VDP_COUNTRY_CHOICES_UNAVAILABLE' { Get-VdpSteps $frame 160 'ZZZ' 'BBB' }
    Assert-Refusal 'VDP_SEEK_TARGET_INVALID' { Get-VdpSteps $frame 100 'AAA' 'BBB' }
}
Invoke-Test 'pointer uses clipped current rect and rejects stale hidden duplicate targets' {
    $frame = New-Frame; $point = Get-VdpHoverPoint $frame 'production' 'row'
    Assert-Test ($point.x -eq 44 -and $point.y -eq 34)
    $frame.controls[0].generation = 0
    Assert-Refusal 'VDP_CONTROL_GEOMETRY_UNAVAILABLE' { Get-VdpHoverPoint $frame 'production' 'row' }
    $frame = New-Frame; $frame.controls[0].owns_pointer = $false
    Assert-Refusal 'VDP_CONTROL_GEOMETRY_UNAVAILABLE' { Get-VdpHoverPoint $frame 'production' 'row' }
    $frame = New-Frame; $frame.controls[0].clip_rect = @(1000, 1000, 10, 10)
    Assert-Refusal 'VDP_CONTROL_GEOMETRY_INVALID' { Get-VdpHoverPoint $frame 'production' 'row' }
    $frame = New-Frame; $frame.controls = @($frame.controls[0], $frame.controls[0])
    Assert-Refusal 'VDP_CONTROL_UNAVAILABLE' { Get-VdpHoverPoint $frame 'production' 'row' }
}
Invoke-Test 'input generation mismatch rejected before claimed result' {
    $frame = New-Frame; $ev = New-Evidence $frame
    $step = @(Get-VdpSteps $frame 160 'AAA' 'BBB')[0]
    $action = New-Action $frame $step $ev; $inputRow = New-Input $action $frame; $inputRow.generation = 2
    Assert-Refusal 'VDP_INPUT_GENERATION_MISMATCH' { Get-VdpDesktopEvidence @($inputRow) 123 $authority 100 $ev }
}
Invoke-Test 'rejected input rollback cannot pass through a later positive frame' {
    $frame = New-Frame; $ev = New-Evidence $frame
    $action = New-Action $frame (@(Get-VdpSteps $frame 160 'AAA' 'BBB')[0]) $ev
    $inputRow = New-Input $action $frame; $ev = Get-VdpDesktopEvidence @($inputRow) 123 $authority 100 $ev
    $rows = Frame-Rows (New-Frame 2); $rows[0].map.presentation_status = 'window_skipped'; $rows[0].map.presented_to_product_target = $false
    $ev = Get-VdpDesktopEvidence (@($rows[0]) + @(Frame-Rows (New-Frame 3 160 1))) 123 $authority 100 $ev
    Assert-Refusal 'VDP_INPUT_REJECTED_ROLLBACK' { Test-VdpStepResult $ev $action }
}
Invoke-Test 'wrong route and unapplied dispatch cannot pass' {
    foreach ($mutation in @('route', 'disposition')) {
        $frame = New-Frame; $ev = New-Evidence $frame
        $action = New-Action $frame (@(Get-VdpSteps $frame 160 'AAA' 'BBB')[0]) $ev
        $inputRow = New-Input $action $frame; $inputRow.$mutation = 'blocked'
        $ev = Get-VdpDesktopEvidence @($inputRow) 123 $authority 100 $ev
        Assert-Refusal 'VDP_INPUT_NOT_APPLIED' { Test-VdpStepResult $ev $action }
    }
}
Invoke-Test 'panel unavailable stays named even when toggle succeeds' {
    $frame = New-Frame 1 100 0 'AAA'; $ev = New-Evidence $frame
    $action = New-Action $frame (@(Get-VdpSteps $frame 160 'AAA' 'BBB')[4]) $ev
    $inputRow = New-Input $action $frame; $ev = Get-VdpDesktopEvidence @($inputRow) 123 $authority 100 $ev
    $after = New-Frame 2 100 1 'AAA' $true; $after.production_lines.available = $false
    $ev = Get-VdpDesktopEvidence (Frame-Rows $after) 123 $authority 100 $ev
    Assert-Refusal 'VDP_PANEL_NAMED_UNAVAILABLE' { Test-VdpStepResult $ev $action }
}
Invoke-Test 'tooltip requires actual consumer target and visible semantic result' {
    $frame = New-Frame 1 100 0 'AAA' $true; $ev = New-Evidence $frame
    $action = New-Action $frame (@(Get-VdpSteps $frame 160 'AAA' 'BBB')[5]) $ev
    $action.point = Get-VdpHoverPoint $frame 'production' 'row'
    $inputRow = New-Input $action $frame; $inputRow.target.element_id = 'another'
    $ev = Get-VdpDesktopEvidence @($inputRow) 123 $authority 100 $ev
    Assert-Refusal 'VDP_POINTER_OWNER_MISMATCH' { Test-VdpStepResult $ev $action }
    $ev = New-Evidence $frame; $inputRow = New-Input $action $frame
    $ev = Get-VdpDesktopEvidence @($inputRow) 123 $authority 100 $ev
    $after = New-Frame 2 100 1 'AAA' $true
    $after.tooltip = Copy-Fixture @{ window_id = 'production'; widget_id = 'row'; runtime_instance_token = 'runtime-1'
        source_file = 'interface/production.gui'; surface_id = 'production'; child_path = @(0, 2)
        generation = 2; presented = $false; semantic_receipts = @() }
    $ev = Get-VdpDesktopEvidence (Frame-Rows $after) 123 $authority 100 $ev
    Assert-Refusal 'VDP_TOOLTIP_RECEIPT_MISMATCH' { Test-VdpStepResult $ev $action }
}
Invoke-Test 'null blank and non-array semantic receipts never qualify panels or tooltips' {
    Assert-Test (-not (Test-VdpSemanticReceipts @()))
    foreach ($bad in @($null, '', 'scalar', @($null), @(''), @(' '), @(42))) {
        Assert-Test (-not (Test-VdpSemanticReceipts $bad)) 'Malformed semantic receipts accepted.'
    }
    Assert-Test (Test-VdpSemanticReceipts @('fixture actual semantic reference'))
    $frame = New-Frame 1 100 0 'AAA'; $ev = New-Evidence $frame
    $action = New-Action $frame (@(Get-VdpSteps $frame 160 'AAA' 'BBB')[4]) $ev
    $inputRow = New-Input $action $frame; $ev = Get-VdpDesktopEvidence @($inputRow) 123 $authority 100 $ev
    $after = New-Frame 2 100 1 'AAA' $true; $after.production_lines.semantic_receipts = $null
    $ev = Get-VdpDesktopEvidence (Frame-Rows $after) 123 $authority 100 $ev
    Assert-Refusal 'VDP_PANEL_NAMED_UNAVAILABLE' { Test-VdpStepResult $ev $action }
    $frame = New-Frame 1 100 0 'AAA' $true; $ev = New-Evidence $frame
    $action = New-Action $frame (@(Get-VdpSteps $frame 160 'AAA' 'BBB')[5]) $ev
    $action.point = Get-VdpHoverPoint $frame 'production' 'row'
    $inputRow = New-Input $action $frame; $ev = Get-VdpDesktopEvidence @($inputRow) 123 $authority 100 $ev
    $after = New-Frame 2 100 1 'AAA' $true
    $after.tooltip = Copy-Fixture @{ window_id = 'production'; widget_id = 'row'; runtime_instance_token = 'runtime-1'
        source_file = 'interface/production.gui'; surface_id = 'production'; child_path = @(0, 2)
        generation = 2; presented = $true; semantic_receipts = $null }
    $ev = Get-VdpDesktopEvidence (Frame-Rows $after) 123 $authority 100 $ev
    Assert-Refusal 'VDP_TOOLTIP_RECEIPT_MISMATCH' { Test-VdpStepResult $ev $action }
}
Invoke-Test 'tooltip identity includes changed child source surface and current geometry' {
    foreach ($field in @('child_path', 'source_file', 'surface_id', 'client_rect')) {
        $frame = New-Frame 1 100 0 'AAA' $true; $ev = New-Evidence $frame
        $action = New-Action $frame (@(Get-VdpSteps $frame 160 'AAA' 'BBB')[5]) $ev
        $action.point = Get-VdpHoverPoint $frame 'production' 'row'
        $inputRow = New-Input $action $frame; $ev = Get-VdpDesktopEvidence @($inputRow) 123 $authority 100 $ev
        $after = New-Frame 2 100 1 'AAA' $true
        $after.tooltip = Copy-Fixture @{ window_id = 'production'; widget_id = 'row'; runtime_instance_token = 'runtime-1'
            source_file = 'interface/production.gui'; surface_id = 'production'; child_path = @(0, 2)
            generation = 2; presented = $true; semantic_receipts = @('fixture tooltip') }
        switch ($field) {
            'child_path' { $after.controls[0].child_path = @(0, 9) }
            'source_file' { $after.tooltip.source_file = 'other.gui' }
            'surface_id' { $after.tooltip.surface_id = 'other' }
            'client_rect' { $after.controls[0].client_rect = @(11, 20, 50, 30) }
        }
        $ev = Get-VdpDesktopEvidence (Frame-Rows $after) 123 $authority 100 $ev
        Assert-Refusal 'VDP_TOOLTIP_RECEIPT_MISMATCH' { Test-VdpStepResult $ev $action }
    }
}
Invoke-Test 'queued versus consumed child-path change is stale even at identical coordinates' {
    $frame = New-Frame 1 100 0 'AAA' $true; $ev = New-Evidence $frame
    $action = New-Action $frame (@(Get-VdpSteps $frame 160 'AAA' 'BBB')[5]) $ev
    $action.point = Get-VdpHoverPoint $frame 'production' 'row'
    $consumed = New-Frame 2 100 0 'AAA' $true; $consumed.controls[0].child_path = @(1, 9)
    $ev = Get-VdpDesktopEvidence (Frame-Rows $consumed) 123 $authority 100 $ev
    $inputRow = New-Input $action $consumed; $inputRow.target = $consumed.controls[0]
    $ev = Get-VdpDesktopEvidence @($inputRow) 123 $authority 100 $ev
    Assert-Refusal 'VDP_STALE_CONTROL_GEOMETRY' { Test-VdpStepResult $ev $action }
}
Invoke-Test 'newer stable consumed target qualifies; invisible final target does not' {
    $frame = New-Frame 1 100 0 'AAA' $true; $ev = New-Evidence $frame
    $action = New-Action $frame (@(Get-VdpSteps $frame 160 'AAA' 'BBB')[5]) $ev
    $action.point = Get-VdpHoverPoint $frame 'production' 'row'
    $consumed = New-Frame 2 100 0 'AAA' $true
    $ev = Get-VdpDesktopEvidence (Frame-Rows $consumed) 123 $authority 100 $ev
    $inputRow = New-Input $action $consumed
    $ev = Get-VdpDesktopEvidence @($inputRow) 123 $authority 100 $ev
    $after = New-Frame 3 100 1 'AAA' $true
    $after.tooltip = Copy-Fixture @{ window_id = 'production'; widget_id = 'row'; runtime_instance_token = 'runtime-1'
        source_file = 'interface/production.gui'; surface_id = 'production'; child_path = @(0, 2)
        generation = 3; presented = $true; semantic_receipts = @('fixture tooltip') }
    $ev = Get-VdpDesktopEvidence (Frame-Rows $after) 123 $authority 100 $ev
    Assert-Test (Test-VdpStepResult $ev $action)
    $ev.current.controls[0].visible = $false
    Assert-Refusal 'VDP_CONTROL_GEOMETRY_UNAVAILABLE' { Test-VdpStepResult $ev $action }
}
Invoke-Test 'pipe drain timeout is named and bounded without blocking fixture tasks' {
    $never = [pscustomobject]@{ RequestedTimeout = -1 }
    $never | Add-Member ScriptMethod Wait { param($Millis) $this.RequestedTimeout = $Millis; return $false }
    Assert-Refusal 'VDP_PIPE_DRAIN_TIMEOUT' { Wait-VdpPipeDrain @($never) }
    Assert-Test ($never.RequestedTimeout -ge 0 -and $never.RequestedTimeout -le 5000)
    Wait-VdpPipeDrain @([Threading.Tasks.Task]::FromResult(0), [Threading.Tasks.Task]::FromResult(0))
    $failed = [pscustomobject]@{}
    $failed | Add-Member ScriptMethod Wait { param($Millis) throw 'fixture pipe fault' }
    Assert-Refusal 'VDP_PIPE_DRAIN_FAILED' { Wait-VdpPipeDrain @($failed) }
}
Invoke-Test 'stream parser ignores partial tail and rejects malformed/truncated diagnostics' {
    Assert-Test (@(Read-VdpJsonLines '{"schema":"hoi4.viewer.desktop.ui.v1"').Count -eq 0)
    Assert-Refusal 'VDP_RECEIPT_TRUNCATED' { Read-VdpJsonLines '{"schema":"hoi4.viewer.desktop.ui.v1"' -Complete }
    Assert-Refusal 'VDP_RECEIPT_MALFORMED' { Read-VdpJsonLines "{`"schema`":`"hoi4.viewer.desktop.ui.v1`",bad}`n" }
    $path = Join-Path $testRoot 'incremental.txt'; $bytes = [Text.Encoding]::UTF8.GetBytes("{`"schema`":`"hoi4.viewer.desktop.ui.v1`",`"note`":`"é`"}`n")
    $cut = [Array]::IndexOf($bytes, [byte]0xc3) + 1
    [IO.File]::WriteAllBytes($path, $bytes[0..($cut - 1)])
    $cursor = @{ offset = 0L; tail = [byte[]]::new(0) }
    Assert-Test (@(Read-VdpStreamChunk $path $cursor).Count -eq 0)
    [IO.File]::WriteAllBytes($path, $bytes)
    $rows = @(Read-VdpStreamChunk $path $cursor -Complete)
    Assert-Test ($rows.Count -eq 1 -and $rows[0].note -ceq 'é')
}
Invoke-Test 'whole timeline requires exact nonzero started/completed count' {
    $start = '{"schema":"replay.validate_playback.stage.v1","stage":"all_frame_payloads_and_static_catalog_bindings","result":"STARTED","expected_hours":2}'
    $end = '{"schema":"replay.validate_playback.completed.v1","stage":"all_frame_payloads_and_static_catalog_bindings","result":"PASS","hours_validated":2}'
    Assert-VdpWholePlayback ($start + "`n" + $end)
    Assert-Refusal 'VDP_WHOLE_PLAYBACK_RECEIPT_MISSING' { Assert-VdpWholePlayback $start }
    Assert-Refusal 'VDP_WHOLE_PLAYBACK_RECEIPT_MISSING' { Assert-VdpWholePlayback ($start + "`n" + $end.Replace(':2}', ':1}')) }
}
Invoke-Test 'argument quoting preserves whitespace quotes and trailing backslashes' {
    Assert-Test ((ConvertTo-VdpArgument 'C:\space here\') -ceq '"C:\space here\\"')
    Assert-Test ((ConvertTo-VdpArgument 'a"b') -ceq '"a\"b"')
    Assert-Test ((ConvertTo-VdpArgument '') -ceq '""')
}
Invoke-Test 'owned Win32 route fixture checks PID class client DPI modifiers and key allowlist' {
    # This in-memory stand-in has NO P/Invoke. Never initialize the real native
    # helper in offline tests; all calls below hit this explicitly synthetic type.
    Add-Type -TypeDefinition @'
using System;
using System.Text;
using System.Collections.Generic;
public static class ViewerDesktopValidationNative {
  public struct Rect { public int Left, Top, Right, Bottom; }
  public static uint Owner = 123, Dpi = 144;
  public static string ClassName = "replay_app_window";
  public static bool Visible = true, Iconic = false, PostOk = true;
  public static short Modifier = 0;
  public static int FailAt = -1;
  public static List<uint> Messages = new List<uint>();
  public static uint GetWindowThreadProcessId(IntPtr h, out uint pid) { pid = Owner; return 1; }
  public static int GetClassName(IntPtr h, StringBuilder s, int n) { s.Append(ClassName); return ClassName.Length; }
  public static bool ReadClientForWindow(IntPtr h, out Rect r) { r = new Rect { Right = 800, Bottom = 600 }; return true; }
  public static uint GetDpiForWindow(IntPtr h) { return Dpi; }
  public static bool IsWindowVisible(IntPtr h) { return Visible; }
  public static bool IsIconic(IntPtr h) { return Iconic; }
  public static short GetAsyncKeyState(int k) { return Modifier; }
  public static bool PostMessage(IntPtr h, uint m, UIntPtr w, IntPtr l) { Messages.Add(m); return PostOk && Messages.Count != FailAt; }
}
'@
    $exe = 'C:\offline-fixture.exe'; $start = [DateTime]::UtcNow
    $child = [pscustomobject]@{ Id = 123; StartTime = $start; HasExited = $false; MainModule = @{ FileName = $exe }; MainWindowHandle = [IntPtr]1234 }
    $child | Add-Member ScriptMethod Refresh {}
    $record = @{ pid = 123; process_started_utc = $start.ToUniversalTime().ToString('o') }
    $frame = New-Frame; $step = @(Get-VdpSteps $frame 160 'AAA' 'BBB')[0]
    Send-VdpPublicInput $child $record $exe $frame $step $null
    Assert-Test ([ViewerDesktopValidationNative]::Messages.Count -eq 2)
    [ViewerDesktopValidationNative]::Messages.Clear()
    [ViewerDesktopValidationNative]::Owner = 456
    Assert-Refusal 'VDP_WINDOW_GEOMETRY_MISMATCH' { Send-VdpPublicInput $child $record $exe $frame $step $null }
    [ViewerDesktopValidationNative]::Owner = 123; [ViewerDesktopValidationNative]::Dpi = 96
    Assert-Refusal 'VDP_WINDOW_GEOMETRY_MISMATCH' { Send-VdpPublicInput $child $record $exe $frame $step $null }
    [ViewerDesktopValidationNative]::Dpi = 144; [ViewerDesktopValidationNative]::ClassName = 'the game'
    Assert-Refusal 'VDP_WINDOW_GEOMETRY_MISMATCH' { Send-VdpPublicInput $child $record $exe $frame $step $null }
    [ViewerDesktopValidationNative]::ClassName = 'replay_app_window'; [ViewerDesktopValidationNative]::Modifier = -1
    Assert-Refusal 'VDP_MODIFIER_HELD' { Send-VdpPublicInput $child $record $exe $frame $step $null }
    [ViewerDesktopValidationNative]::Modifier = 0
    $step.vk = 0x20
    Assert-Refusal 'VDP_INPUT_ROUTE_UNSUPPORTED' { Send-VdpPublicInput $child $record $exe $frame $step $null }
    Assert-Test ([ViewerDesktopValidationNative]::Messages.Count -eq 0) 'Refused custody sent input.'
    $step.vk = 0x27; $posting = @{ input_posted = $false; messages_posted = 0 }
    [ViewerDesktopValidationNative]::FailAt = 1
    Assert-Refusal 'VDP_INPUT_POST_FAILED' { Send-VdpPublicInput $child $record $exe $frame $step $null $posting }
    Assert-Test (-not $posting.input_posted -and $posting.messages_posted -eq 0)
    [ViewerDesktopValidationNative]::Messages.Clear(); [ViewerDesktopValidationNative]::FailAt = 2
    Assert-Refusal 'VDP_INPUT_POST_FAILED' { Send-VdpPublicInput $child $record $exe $frame $step $null $posting }
    Assert-Test ($posting.input_posted -and $posting.messages_posted -eq 1) 'Partial key posting custody lost.'
    [ViewerDesktopValidationNative]::FailAt = -1
    $child.Id = 999
    Assert-Refusal 'VDP_PROCESS_CUSTODY_LOST' { Send-VdpPublicInput $child $record $exe $frame $step $null }
}
Invoke-Test 'Prepare pins files emits no product PASS and never invokes process driver' {
    $exe = Join-Path $testRoot 'fixture viewer.exe'; $replay = Join-Path $testRoot 'fixture.battleplan'
    [IO.File]::WriteAllText($exe, 'NOT AN EXECUTABLE; offline preparation only')
    [IO.File]::WriteAllText($replay, 'NOT A RECORDING; offline preparation only')
    $planRoot = Join-Path $testRoot 'prepare'
    $exeHash = (Get-VdpReference $exe).sha256; $replayHash = (Get-VdpReference $replay).sha256
    & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $scriptUnderTest -Mode Prepare -ViewerExecutable $exe -ExpectedViewerSha256 $exeHash -RecordingPath $replay -ExpectedRecordingSha256 $replayHash -GameRoot $testRoot -UserDataRoot $testRoot -ExpectedContentAuthoritySha256 ('a' * 64) -ExpectedHour 100 -SeekHour 160 -FirstCountryTag AAA -SecondCountryTag BBB -TooltipWindowId production -TooltipElementId row -OutputDirectory $planRoot | Out-Null
    Assert-Test ($LASTEXITCODE -eq 0) 'Prepare invocation failed.'
    $receipt = Get-Content -LiteralPath (Join-Path $planRoot 'DESKTOP_PLAYBACK.json') -Raw | ConvertFrom-Json
    Assert-Test ($receipt.status -ceq 'prepared_not_run' -and -not $receipt.playback_qualified -and $receipt.inputs_sent -eq 0 -and $receipt.cases.Count -eq 0)
    Assert-Test ($receipt.plan.Count -eq 4 -and $receipt.plan[3].argv[0] -ceq '--recording')
}

$summary = [ordered]@{ schema = 'hoi4.viewer-desktop-offline-tests.v1'; fixture_only = $true; product_launches = 0
    source = Get-VdpReference $scriptUnderTest; passed = $passes; failed = $failures.Count; failures = @($failures.ToArray()) }
[IO.File]::WriteAllText((Join-Path $testRoot 'OFFLINE_TESTS.json'), ($summary | ConvertTo-Json -Depth 10))
Write-Output ($summary | ConvertTo-Json -Depth 10 -Compress)
if ($failures.Count -gt 0 -or $passes -eq 0) { exit 1 }
exit 0
