<#
.SYNOPSIS
    Zero-manual-step live-validation runsheet for the Battle Plan extractor.

.DESCRIPTION
    Drives the next live speed-5 deep-war validation session end to end against a
    running hoi4.exe. Three stages:

      Stage 1 - d53 shadow gate: a short capture with the d53 relation reader in
        "shadow" mode (offsets + legacy-scan run side by side). PASS iff
        domain53_relation_shadow_mismatches == 0 (or absent) on every d53 frame.
        Any mismatch is a HARD BLOCKER.

      Stage 2 - paired A/B: a production -Mode ShortSpeed5 capture at current head,
        compared against the recorded 2026-07-09 baseline. Emits a markdown report:
        frame avg/p95/max, game-hours advanced, skipped hours + %, stream bytes +
        bytes/captured-hour, per-domain read_time_us top-10 vs baseline, the five
        delta domains' emitted-vs-observed rows, host_finalize peak RAM + finalize
        seconds, and an explicit VERDICT block.

      Stage 3 (optional, -IncludeEmitModeAB): a short capture with all five delta
        domains forced to emit_mode=full, to attribute the size reduction.

    All captures reuse Start-Hoi4ProductionSpeed5Capture.ps1 (which owns pid
    detection, static-artifact wiring, and the production arg set). Control-field
    injection uses that launcher's -D53RelationReadMode / -EmitModeAll parameters.

    -DryRun validates argument assembly + baseline parsing WITHOUT touching
    hoi4.exe. -SelfTest exercises the pure parsing/verdict functions against the
    real baseline manifest plus synthetic fixtures. Neither attaches to the game.

.NOTES
    PowerShell 5.1 compatible (no &&, no ternary). Zero-data-loss discipline: this
    tooling only reports; it never mutates capture output.
#>
[CmdletBinding()]
param(
    [int]$Hoi4Pid = 0,

    # Recorded 2026-07-09 ShortSpeed5 baseline. Default is repo-root-relative; the
    # orchestrator runs this from the main checkout where the run dir exists.
    [string]$BaselineManifest = "",

    # Root output dir for this session's stage subdirs. Default: runs/live_validation_<stamp>.
    [string]$OutRoot = "",

    [double]$ShadowSeconds = 75,

    # 0 => launcher ShortSpeed5 default (300 s).
    [double]$ShortSpeed5Seconds = 0,

    [double]$ProbeSeconds = 12,

    [double]$EmitModeABSeconds = 75,

    [switch]$IncludeEmitModeAB,

    [switch]$SkipBuild,

    # Skip the game-advancing probe capture (orchestrator already confirmed speed 5).
    [switch]$SkipProbe,

    [switch]$SkipDiskSpaceCheck,

    [double]$FrameAvgTargetMs = 20,

    [double]$Phase2FrameAvgTargetMs = 45,

    [switch]$DryRun,

    [switch]$SelfTest
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$FIVE_DELTA_DOMAINS = @(
    @{ Name = "projects_live"; Id = 41 },
    @{ Name = "supply_logistics_attrition_live"; Id = 44 },
    @{ Name = "construction_buildings_state_damage_live"; Id = 48 },
    @{ Name = "scripted_state_flags_variables_live"; Id = 51 },
    @{ Name = "diplomacy_intel_operatives_live"; Id = 53 }
)
$D53_DOMAIN = "diplomacy_intel_operatives_live"
$SHADOW_MISMATCH_UNIT = "domain53_relation_shadow_mismatches"

# ---------------------------------------------------------------------------
# JSON access helpers (StrictMode-safe: never dereference an absent property).
# ---------------------------------------------------------------------------

function Test-JsonProp {
    param([Parameter(Mandatory = $true)]$Object, [Parameter(Mandatory = $true)][string]$Name)
    if ($null -eq $Object) { return $false }
    $props = $Object.PSObject.Properties
    if ($null -eq $props) { return $false }
    return ($props.Name -contains $Name)
}

function Get-JsonProp {
    param(
        [Parameter(Mandatory = $true)]$Object,
        [Parameter(Mandatory = $true)][string]$Name,
        $Default = $null
    )
    if (Test-JsonProp -Object $Object -Name $Name) {
        return $Object.$Name
    }
    return $Default
}

function Read-ManifestFile {
    param([Parameter(Mandatory = $true)][string]$Path)
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "manifest not found: $Path"
    }
    $raw = Get-Content -LiteralPath $Path -Raw
    if ([string]::IsNullOrWhiteSpace($raw)) {
        throw "manifest is empty: $Path"
    }
    return ($raw | ConvertFrom-Json)
}

# ---------------------------------------------------------------------------
# Pure metric extraction + verdicts (exercised by -SelfTest).
# ---------------------------------------------------------------------------

function ConvertTo-SessionMetrics {
    <#
      Normalize a parsed manifest object into a flat, comparison-friendly shape.
      Returns $null-safe fields; frame/read times converted us -> ms where noted.
    #>
    param([Parameter(Mandatory = $true)]$Manifest, [string]$Label = "")

    $cs = Get-JsonProp -Object $Manifest -Name "compact_stream"
    $frame = $null
    $streamBytes = $null
    $decodable = $null
    if ($null -ne $cs) {
        $frame = Get-JsonProp -Object $cs -Name "frame_time_us"
        $streamBytes = Get-JsonProp -Object $cs -Name "total_bytes"
        $decodable = Get-JsonProp -Object $cs -Name "decodable"
    }

    $frameAvgMs = $null; $frameP95Ms = $null; $frameMaxMs = $null; $frameCount = $null
    if ($null -ne $frame) {
        $avg = Get-JsonProp -Object $frame -Name "avg"
        $p95 = Get-JsonProp -Object $frame -Name "p95"
        $max = Get-JsonProp -Object $frame -Name "max"
        $frameCount = Get-JsonProp -Object $frame -Name "count"
        if ($null -ne $avg) { $frameAvgMs = [double]$avg / 1000.0 }
        if ($null -ne $p95) { $frameP95Ms = [double]$p95 / 1000.0 }
        if ($null -ne $max) { $frameMaxMs = [double]$max / 1000.0 }
    }

    $gameHours = Get-JsonProp -Object $Manifest -Name "game_hours_advanced"
    $skipped = Get-JsonProp -Object $Manifest -Name "skipped_game_hours"
    $capturedHours = Get-JsonProp -Object $Manifest -Name "unique_game_hour_count"

    $skippedPct = $null
    if ($null -ne $skipped -and $null -ne $gameHours -and [double]$gameHours -gt 0) {
        $skippedPct = 100.0 * [double]$skipped / [double]$gameHours
    }

    $bytesPerHour = $null
    if ($null -ne $streamBytes -and $null -ne $capturedHours -and [double]$capturedHours -gt 0) {
        $bytesPerHour = [double]$streamBytes / [double]$capturedHours
    }

    # Per-domain read + row stats.
    $domains = @{}
    if ($null -ne $cs) {
        $perDomain = Get-JsonProp -Object $cs -Name "per_domain"
        if ($null -ne $perDomain) {
            foreach ($prop in $perDomain.PSObject.Properties) {
                $d = $prop.Value
                $rt = Get-JsonProp -Object $d -Name "read_time_us"
                $readAvg = $null; $readP95 = $null; $readMax = $null
                if ($null -ne $rt) {
                    $ra = Get-JsonProp -Object $rt -Name "avg"
                    $rp = Get-JsonProp -Object $rt -Name "p95"
                    $rm = Get-JsonProp -Object $rt -Name "max"
                    if ($null -ne $ra) { $readAvg = [double]$ra }
                    if ($null -ne $rp) { $readP95 = [double]$rp }
                    if ($null -ne $rm) { $readMax = [double]$rm }
                }
                $domains[$prop.Name] = [ordered]@{
                    Name         = $prop.Name
                    DomainId     = Get-JsonProp -Object $d -Name "domain_id"
                    ReadAvgUs    = $readAvg
                    ReadP95Us    = $readP95
                    ReadMaxUs    = $readMax
                    RecordCount  = Get-JsonProp -Object $d -Name "record_count"
                    ChangedRows  = Get-JsonProp -Object $d -Name "changed_rows_total"
                    DataRows     = Get-JsonProp -Object $d -Name "data_rows_total"
                    ObservedRows = Get-JsonProp -Object $d -Name "row_count_total"
                    DataUnits    = Get-JsonProp -Object $d -Name "data_units_total"
                }
            }
        }
    }

    # Shadow mismatch counter (only present in data_units_total when non-zero).
    $shadowMismatches = 0
    if ($domains.ContainsKey($D53_DOMAIN)) {
        $units = $domains[$D53_DOMAIN].DataUnits
        $unitVal = Get-JsonProp -Object $units -Name $SHADOW_MISMATCH_UNIT -Default 0
        if ($null -ne $unitVal) { $shadowMismatches = [double]$unitVal }
    }

    # host_finalize telemetry.
    $hf = Get-JsonProp -Object $Manifest -Name "host_finalize"
    $peakPrivate = $null; $peakWs = $null; $finalizeSeconds = $null
    if ($null -ne $hf) {
        $peakPrivate = Get-JsonProp -Object $hf -Name "peak_private_bytes"
        $peakWs = Get-JsonProp -Object $hf -Name "peak_working_set_bytes"
        $finalizeSeconds = Get-JsonProp -Object $hf -Name "finalize_seconds"
    }

    $hourly = Get-JsonProp -Object $Manifest -Name "hourly_requirement"
    $hourlyMet = $null
    if ($null -ne $hourly) { $hourlyMet = Get-JsonProp -Object $hourly -Name "met" }

    return [ordered]@{
        Label            = $Label
        SessionId        = Get-JsonProp -Object $Manifest -Name "session_id"
        FrameAvgMs       = $frameAvgMs
        FrameP95Ms       = $frameP95Ms
        FrameMaxMs       = $frameMaxMs
        FrameCount       = $frameCount
        GameHours        = $gameHours
        SkippedHours     = $skipped
        CapturedHours    = $capturedHours
        SkippedPct       = $skippedPct
        StreamBytes      = $streamBytes
        BytesPerHour     = $bytesPerHour
        Decodable        = $decodable
        Domains          = $domains
        ShadowMismatches = $shadowMismatches
        D53ReadMode      = Get-JsonProp -Object $Manifest -Name "d53_relation_read_mode"
        PeakPrivateBytes = $peakPrivate
        PeakWorkingSet   = $peakWs
        FinalizeSeconds  = $finalizeSeconds
        HourlyMet        = $hourlyMet
    }
}

function Test-ShadowGate {
    <#
      d53 shadow gate. PASS iff every d53 frame had zero relation shadow mismatches.
      Also confirms the run actually ran in shadow mode (else the counter is
      meaningless and the gate would falsely pass).
    #>
    param([Parameter(Mandatory = $true)]$Metrics)

    $problems = New-Object System.Collections.Generic.List[string]

    if (-not $Metrics.Domains.ContainsKey($D53_DOMAIN)) {
        $problems.Add("d53 domain ($D53_DOMAIN) absent from manifest; no d53 frames captured")
    } else {
        $rc = $Metrics.Domains[$D53_DOMAIN].RecordCount
        if ($null -eq $rc -or [double]$rc -le 0) {
            $problems.Add("d53 domain has zero records; the game may have been paused")
        }
    }

    $readMode = $Metrics.D53ReadMode
    if ($null -eq $readMode) {
        $problems.Add("manifest has no d53_relation_read_mode field (older recorder?); cannot confirm shadow mode ran")
    } elseif ($readMode -ne "shadow") {
        $problems.Add("run used d53_relation_read_mode='$readMode', not 'shadow'; the mismatch counter is meaningless")
    }

    $mismatches = $Metrics.ShadowMismatches
    if ($null -ne $mismatches -and [double]$mismatches -gt 0) {
        $problems.Add("domain53_relation_shadow_mismatches=$mismatches (BLOCKER: offsets and legacy-scan paths diverged)")
    }

    return [ordered]@{
        Pass       = ($problems.Count -eq 0)
        Mismatches = $mismatches
        ReadMode   = $readMode
        Problems   = @($problems)
    }
}

function New-PairedVerdict {
    param(
        [Parameter(Mandatory = $true)]$Head,
        [Parameter(Mandatory = $true)]$Baseline,
        [double]$FrameAvgTargetMs = 20,
        [double]$Phase2FrameAvgTargetMs = 45
    )

    $skippedPass = ($null -ne $Head.SkippedHours -and [double]$Head.SkippedHours -eq 0)

    $frameAvgUnder20 = $false
    $frameAvgUnder45 = $false
    if ($null -ne $Head.FrameAvgMs) {
        $frameAvgUnder20 = ([double]$Head.FrameAvgMs -le $FrameAvgTargetMs)
        $frameAvgUnder45 = ([double]$Head.FrameAvgMs -le $Phase2FrameAvgTargetMs)
    }

    # Size reduction: prefer bytes/captured-hour (fair across differing durations);
    # fall back to total stream bytes.
    $sizeFactorPerHour = $null
    if ($null -ne $Head.BytesPerHour -and [double]$Head.BytesPerHour -gt 0 -and $null -ne $Baseline.BytesPerHour) {
        $sizeFactorPerHour = [double]$Baseline.BytesPerHour / [double]$Head.BytesPerHour
    }
    $sizeFactorTotal = $null
    if ($null -ne $Head.StreamBytes -and [double]$Head.StreamBytes -gt 0 -and $null -ne $Baseline.StreamBytes) {
        $sizeFactorTotal = [double]$Baseline.StreamBytes / [double]$Head.StreamBytes
    }

    $frameAvgDeltaMs = $null
    if ($null -ne $Head.FrameAvgMs -and $null -ne $Baseline.FrameAvgMs) {
        $frameAvgDeltaMs = [double]$Head.FrameAvgMs - [double]$Baseline.FrameAvgMs
    }

    return [ordered]@{
        SkippedZeroPass     = $skippedPass
        FrameAvgUnder20     = $frameAvgUnder20
        FrameAvgUnder45     = $frameAvgUnder45
        FrameAvgTargetMs    = $FrameAvgTargetMs
        Phase2TargetMs      = $Phase2FrameAvgTargetMs
        FrameAvgDeltaMs     = $frameAvgDeltaMs
        SizeFactorPerHour   = $sizeFactorPerHour
        SizeFactorTotal     = $sizeFactorTotal
        HourlyGatePass      = ($skippedPass -and ($null -ne $Head.HourlyMet -and $Head.HourlyMet))
    }
}

# ---------------------------------------------------------------------------
# Formatting.
# ---------------------------------------------------------------------------

function Format-Bytes {
    # Decimal units (GB = 1e9) to match the plan's stated baseline (28.15 GB /
    # 11.5 MB per captured hour).
    param($Bytes)
    if ($null -eq $Bytes) { return "n/a" }
    $b = [double]$Bytes
    if ($b -ge 1e9) { return ("{0:N2} GB" -f ($b / 1e9)) }
    if ($b -ge 1e6) { return ("{0:N2} MB" -f ($b / 1e6)) }
    if ($b -ge 1e3) { return ("{0:N2} KB" -f ($b / 1e3)) }
    return ("{0} B" -f [int64]$b)
}

function Format-Num {
    param($Value, [int]$Decimals = 1)
    if ($null -eq $Value) { return "n/a" }
    return ([string]::Format([System.Globalization.CultureInfo]::InvariantCulture, "{0:N$Decimals}", [double]$Value))
}

function Format-ArgNumber {
    # Invariant, NO thousands separator - safe to pass as a CLI numeric argument.
    param([Parameter(Mandatory = $true)][double]$Value)
    return ([string]::Format([System.Globalization.CultureInfo]::InvariantCulture, "{0:0.###}", $Value))
}

function Format-Delta {
    param($HeadValue, $BaselineValue, [int]$Decimals = 1, [string]$Suffix = "")
    if ($null -eq $HeadValue -or $null -eq $BaselineValue) { return "n/a" }
    $d = [double]$HeadValue - [double]$BaselineValue
    $sign = ""
    if ($d -ge 0) { $sign = "+" }
    return ("{0}{1}{2}" -f $sign, (Format-Num -Value $d -Decimals $Decimals), $Suffix)
}

function Get-TopReadDomains {
    param([Parameter(Mandatory = $true)]$Metrics, [int]$Top = 10)
    $rows = @()
    foreach ($key in $Metrics.Domains.Keys) {
        $dom = $Metrics.Domains[$key]
        if ($null -ne $dom.ReadAvgUs) {
            $rows += $dom
        }
    }
    return @($rows | Sort-Object -Property @{ Expression = { [double]$_.ReadAvgUs } } -Descending | Select-Object -First $Top)
}

function Format-ComparisonReport {
    param(
        [Parameter(Mandatory = $true)]$Head,
        [Parameter(Mandatory = $true)]$Baseline,
        [Parameter(Mandatory = $true)]$Verdict,
        [string]$ShadowSummary = "",
        [string]$EmitModeSummary = ""
    )

    $sb = New-Object System.Text.StringBuilder
    $null = $sb.AppendLine("# Live validation session - paired A/B report")
    $null = $sb.AppendLine("")
    $null = $sb.AppendLine("Generated: $(Get-Date -Format 'yyyy-MM-dd HH:mm:ss')")
    $null = $sb.AppendLine("")
    $null = $sb.AppendLine("- Head session:     $($Head.SessionId)")
    $null = $sb.AppendLine("- Baseline session: $($Baseline.SessionId)")
    $null = $sb.AppendLine("")

    if ($ShadowSummary.Length -gt 0) {
        $null = $sb.AppendLine("## Stage 1 - d53 shadow gate")
        $null = $sb.AppendLine("")
        $null = $sb.AppendLine($ShadowSummary)
        $null = $sb.AppendLine("")
    }

    $null = $sb.AppendLine("## Frame timing (deep-war warm)")
    $null = $sb.AppendLine("")
    $null = $sb.AppendLine("| Metric | Head | Baseline | Delta |")
    $null = $sb.AppendLine("|---|---:|---:|---:|")
    $null = $sb.AppendLine("| frame avg (ms) | $(Format-Num $Head.FrameAvgMs 1) | $(Format-Num $Baseline.FrameAvgMs 1) | $(Format-Delta $Head.FrameAvgMs $Baseline.FrameAvgMs 1 ' ms') |")
    $null = $sb.AppendLine("| frame p95 (ms) | $(Format-Num $Head.FrameP95Ms 1) | $(Format-Num $Baseline.FrameP95Ms 1) | $(Format-Delta $Head.FrameP95Ms $Baseline.FrameP95Ms 1 ' ms') |")
    $null = $sb.AppendLine("| frame max (ms) | $(Format-Num $Head.FrameMaxMs 1) | $(Format-Num $Baseline.FrameMaxMs 1) | $(Format-Delta $Head.FrameMaxMs $Baseline.FrameMaxMs 1 ' ms') |")
    $null = $sb.AppendLine("| frame count | $(Format-Num $Head.FrameCount 0) | $(Format-Num $Baseline.FrameCount 0) | - |")
    $null = $sb.AppendLine("")

    $null = $sb.AppendLine("## Coverage (zero-data-loss)")
    $null = $sb.AppendLine("")
    $null = $sb.AppendLine("| Metric | Head | Baseline |")
    $null = $sb.AppendLine("|---|---:|---:|")
    $null = $sb.AppendLine("| game hours advanced | $(Format-Num $Head.GameHours 0) | $(Format-Num $Baseline.GameHours 0) |")
    $null = $sb.AppendLine("| captured hours | $(Format-Num $Head.CapturedHours 0) | $(Format-Num $Baseline.CapturedHours 0) |")
    $null = $sb.AppendLine("| skipped hours | $(Format-Num $Head.SkippedHours 0) | $(Format-Num $Baseline.SkippedHours 0) |")
    $null = $sb.AppendLine("| skipped % | $(Format-Num $Head.SkippedPct 1) | $(Format-Num $Baseline.SkippedPct 1) |")
    $null = $sb.AppendLine("")

    $null = $sb.AppendLine("## Stream size")
    $null = $sb.AppendLine("")
    $null = $sb.AppendLine("| Metric | Head | Baseline |")
    $null = $sb.AppendLine("|---|---:|---:|")
    $null = $sb.AppendLine("| stream bytes | $(Format-Bytes $Head.StreamBytes) | $(Format-Bytes $Baseline.StreamBytes) |")
    $null = $sb.AppendLine("| bytes / captured hour | $(Format-Bytes $Head.BytesPerHour) | $(Format-Bytes $Baseline.BytesPerHour) |")
    $null = $sb.AppendLine("")

    # Per-domain read table (top 10 by head avg), with baseline deltas.
    $null = $sb.AppendLine("## Per-domain read_time_us (top 10 by head avg)")
    $null = $sb.AppendLine("")
    $null = $sb.AppendLine("| Domain | id | head avg us | base avg us | delta us | head p95 us | base p95 us |")
    $null = $sb.AppendLine("|---|---:|---:|---:|---:|---:|---:|")
    foreach ($dom in (Get-TopReadDomains -Metrics $Head -Top 10)) {
        $base = $null
        if ($Baseline.Domains.ContainsKey($dom.Name)) { $base = $Baseline.Domains[$dom.Name] }
        $baseAvg = $null; $baseP95 = $null
        if ($null -ne $base) { $baseAvg = $base.ReadAvgUs; $baseP95 = $base.ReadP95Us }
        $null = $sb.AppendLine("| $($dom.Name) | $($dom.DomainId) | $(Format-Num $dom.ReadAvgUs 0) | $(Format-Num $baseAvg 0) | $(Format-Delta $dom.ReadAvgUs $baseAvg 0) | $(Format-Num $dom.ReadP95Us 0) | $(Format-Num $baseP95 0) |")
    }
    $null = $sb.AppendLine("")

    # Five delta domains: emitted vs observed rows.
    $null = $sb.AppendLine("## Delta domains - emitted vs observed rows")
    $null = $sb.AppendLine("")
    $null = $sb.AppendLine("| Domain | id | head emitted | head observed | head emit% | base emitted | base observed |")
    $null = $sb.AppendLine("|---|---:|---:|---:|---:|---:|---:|")
    foreach ($spec in $FIVE_DELTA_DOMAINS) {
        $hd = $null; $bd = $null
        if ($Head.Domains.ContainsKey($spec.Name)) { $hd = $Head.Domains[$spec.Name] }
        if ($Baseline.Domains.ContainsKey($spec.Name)) { $bd = $Baseline.Domains[$spec.Name] }
        $hEmit = $null; $hObs = $null; $hPct = $null; $bEmit = $null; $bObs = $null
        if ($null -ne $hd) {
            $hEmit = $hd.ChangedRows; $hObs = $hd.ObservedRows
            if ($null -ne $hEmit -and $null -ne $hObs -and [double]$hObs -gt 0) { $hPct = 100.0 * [double]$hEmit / [double]$hObs }
        }
        if ($null -ne $bd) { $bEmit = $bd.ChangedRows; $bObs = $bd.ObservedRows }
        $null = $sb.AppendLine("| $($spec.Name) | $($spec.Id) | $(Format-Num $hEmit 0) | $(Format-Num $hObs 0) | $(Format-Num $hPct 2) | $(Format-Num $bEmit 0) | $(Format-Num $bObs 0) |")
    }
    $null = $sb.AppendLine("")

    $null = $sb.AppendLine("## host_finalize (G5 bounded memory)")
    $null = $sb.AppendLine("")
    $null = $sb.AppendLine("| Metric | Head | Baseline |")
    $null = $sb.AppendLine("|---|---:|---:|")
    $null = $sb.AppendLine("| peak private bytes | $(Format-Bytes $Head.PeakPrivateBytes) | $(Format-Bytes $Baseline.PeakPrivateBytes) |")
    $null = $sb.AppendLine("| peak working set | $(Format-Bytes $Head.PeakWorkingSet) | $(Format-Bytes $Baseline.PeakWorkingSet) |")
    $null = $sb.AppendLine("| finalize seconds | $(Format-Num $Head.FinalizeSeconds 2) | $(Format-Num $Baseline.FinalizeSeconds 2) |")
    $null = $sb.AppendLine("")

    if ($EmitModeSummary.Length -gt 0) {
        $null = $sb.AppendLine("## Stage 3 - emit-mode A/B (size attribution)")
        $null = $sb.AppendLine("")
        $null = $sb.AppendLine($EmitModeSummary)
        $null = $sb.AppendLine("")
    }

    # VERDICT block.
    $null = $sb.AppendLine("## VERDICT")
    $null = $sb.AppendLine("")
    $skipVerdict = "FAIL"
    if ($Verdict.SkippedZeroPass) { $skipVerdict = "PASS" }
    $avg20Verdict = "FAIL"
    if ($Verdict.FrameAvgUnder20) { $avg20Verdict = "PASS" }
    $avg45Verdict = "FAIL"
    if ($Verdict.FrameAvgUnder45) { $avg45Verdict = "PASS" }
    $hourlyVerdict = "FAIL"
    if ($Verdict.HourlyGatePass) { $hourlyVerdict = "PASS" }
    $null = $sb.AppendLine("- skipped hours == 0: **$skipVerdict** (head skipped=$(Format-Num $Head.SkippedHours 0))")
    $null = $sb.AppendLine("- hourly gate met: **$hourlyVerdict**")
    $null = $sb.AppendLine("- frame avg <= $($Verdict.FrameAvgTargetMs) ms (final target): **$avg20Verdict** (head avg=$(Format-Num $Head.FrameAvgMs 1) ms)")
    $null = $sb.AppendLine("- frame avg <= $($Verdict.Phase2TargetMs) ms (Phase-2 target): **$avg45Verdict**")
    $sizePerHourStr = "n/a"
    if ($null -ne $Verdict.SizeFactorPerHour) { $sizePerHourStr = ("{0}x" -f (Format-Num $Verdict.SizeFactorPerHour 2)) }
    $sizeTotalStr = "n/a"
    if ($null -ne $Verdict.SizeFactorTotal) { $sizeTotalStr = ("{0}x" -f (Format-Num $Verdict.SizeFactorTotal 2)) }
    $null = $sb.AppendLine("- size reduction vs baseline (bytes/captured-hour): **$sizePerHourStr**")
    $null = $sb.AppendLine("- size reduction vs baseline (total stream bytes): **$sizeTotalStr**")
    $null = $sb.AppendLine("- frame avg delta vs baseline: $(Format-Delta $Head.FrameAvgMs $Baseline.FrameAvgMs 1 ' ms')")

    return $sb.ToString()
}

function Format-CompactTable {
    <# One-line-per-metric compact table echoed to the console. #>
    param([Parameter(Mandatory = $true)]$Head, [Parameter(Mandatory = $true)]$Baseline, [Parameter(Mandatory = $true)]$Verdict)
    $lines = @()
    $lines += "  frame_avg_ms   head=$(Format-Num $Head.FrameAvgMs 1)   base=$(Format-Num $Baseline.FrameAvgMs 1)   delta=$(Format-Delta $Head.FrameAvgMs $Baseline.FrameAvgMs 1)"
    $lines += "  frame_p95_ms   head=$(Format-Num $Head.FrameP95Ms 1)   base=$(Format-Num $Baseline.FrameP95Ms 1)"
    $lines += "  frame_max_ms   head=$(Format-Num $Head.FrameMaxMs 1)   base=$(Format-Num $Baseline.FrameMaxMs 1)"
    $lines += "  skipped_hours  head=$(Format-Num $Head.SkippedHours 0)   base=$(Format-Num $Baseline.SkippedHours 0)   head_pct=$(Format-Num $Head.SkippedPct 1)"
    $lines += "  stream_bytes   head=$(Format-Bytes $Head.StreamBytes)   base=$(Format-Bytes $Baseline.StreamBytes)"
    $lines += "  bytes_per_hour head=$(Format-Bytes $Head.BytesPerHour)   base=$(Format-Bytes $Baseline.BytesPerHour)"
    $sizePerHourStr = "n/a"
    if ($null -ne $Verdict.SizeFactorPerHour) { $sizePerHourStr = ("{0}x" -f (Format-Num $Verdict.SizeFactorPerHour 2)) }
    $lines += "  size_reduction $sizePerHourStr (bytes/captured-hour)"
    $skipStr = "FAIL"; if ($Verdict.SkippedZeroPass) { $skipStr = "PASS" }
    $avg20Str = "FAIL"; if ($Verdict.FrameAvgUnder20) { $avg20Str = "PASS" }
    $avg45Str = "FAIL"; if ($Verdict.FrameAvgUnder45) { $avg45Str = "PASS" }
    $lines += "  VERDICT        skipped==0:$skipStr  avg<=20ms:$avg20Str  avg<=45ms:$avg45Str"
    return ($lines -join [Environment]::NewLine)
}

# ---------------------------------------------------------------------------
# Environment + preflight.
# ---------------------------------------------------------------------------

function Resolve-RepoRoot {
    $scriptDir = Split-Path -Parent $PSCommandPath
    return (Resolve-Path -LiteralPath (Join-Path $scriptDir "..\..\..")).Path
}

function Resolve-SingleHoi4Pid {
    $processes = @(Get-Process -Name the game -ErrorAction SilentlyContinue)
    if ($processes.Count -eq 0) {
        throw "hoi4.exe is not running. Start the game (deep-war save, speed 5, UNPAUSED) before the live validation session."
    }
    if ($processes.Count -gt 1) {
        $ids = ($processes | ForEach-Object { $_.Id }) -join ", "
        throw "multiple hoi4.exe processes are running ($ids). Pass -Hoi4Pid explicitly."
    }
    return [int]$processes[0].Id
}

function Get-CollectorPids {
    return @(Get-Process -Name collector_record -ErrorAction SilentlyContinue | ForEach-Object { $_.Id })
}

function Stop-NewCollectors {
    <# Kill only collector_record.exe processes that appeared since $Before (never touch other runs'). #>
    param([AllowNull()][AllowEmptyCollection()][int[]]$Before = @())
    if ($null -eq $Before) { $Before = @() }
    $now = Get-CollectorPids
    foreach ($collectorPid in $now) {
        if ($Before -notcontains $collectorPid) {
            Write-Host "cleanup: stopping orphaned collector_record.exe pid=$collectorPid (started by this session)"
            Stop-Process -Id $collectorPid -Force -ErrorAction SilentlyContinue
        }
    }
}

function Invoke-LauncherCapture {
    <#
      Run one capture via Start-Hoi4ProductionSpeed5Capture.ps1 in a child process
      (isolates its `exit` calls and StrictMode). Returns the manifest path.
      Cleans up any collector_record.exe it spawned on failure.
    #>
    param(
        [Parameter(Mandatory = $true)][string]$LauncherPath,
        [Parameter(Mandatory = $true)][string]$Mode,
        [Parameter(Mandatory = $true)][string]$OutDir,
        [Parameter(Mandatory = $true)][string]$SessionId,
        [Parameter(Mandatory = $true)][int]$Hoi4Pid,
        [double]$DurationSeconds = 0,
        [int]$MinGameHours = -1,
        [string]$D53RelationReadMode = "",
        [string]$EmitModeAll = "",
        [switch]$SkipBuild,
        [switch]$SkipDiskSpaceCheck
    )

    $launcherArgs = @(
        "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", $LauncherPath,
        "-Mode", $Mode,
        "-Hoi4Pid", "$Hoi4Pid",
        "-OutDir", $OutDir,
        "-SessionId", $SessionId
    )
    if ($DurationSeconds -gt 0) { $launcherArgs += @("-DurationSeconds", (Format-ArgNumber -Value $DurationSeconds)) }
    if ($MinGameHours -ge 0) { $launcherArgs += @("-MinGameHours", "$MinGameHours") }
    if ($D53RelationReadMode.Length -gt 0) { $launcherArgs += @("-D53RelationReadMode", $D53RelationReadMode) }
    if ($EmitModeAll.Length -gt 0) { $launcherArgs += @("-EmitModeAll", $EmitModeAll) }
    if ($SkipBuild) { $launcherArgs += "-SkipBuild" }
    if ($SkipDiskSpaceCheck) { $launcherArgs += "-SkipDiskSpaceCheck" }

    $before = Get-CollectorPids
    try {
        & powershell @launcherArgs
        $rc = $LASTEXITCODE
        if ($rc -ne 0) {
            throw "launcher exited with code $rc (mode=$Mode out=$OutDir)"
        }
    } catch {
        Stop-NewCollectors -Before $before
        throw
    }

    $manifest = Join-Path $OutDir "native_session_manifest.json"
    if (-not (Test-Path -LiteralPath $manifest -PathType Leaf)) {
        Stop-NewCollectors -Before $before
        throw "capture produced no manifest at $manifest (mode=$Mode)"
    }
    return $manifest
}

# ---------------------------------------------------------------------------
# Self-test (pure functions vs real baseline + synthetic fixtures).
# ---------------------------------------------------------------------------

function New-SyntheticImprovedManifest {
    <# Deep-clone the baseline and mutate it into a "sub-20ms, zero-skip, lean" run. #>
    param([Parameter(Mandatory = $true)]$Baseline)
    $clone = $Baseline | ConvertTo-Json -Depth 60 | ConvertFrom-Json
    $clone.session_id = "synthetic_improved_head"
    $clone.game_hours_advanced = 6346
    $clone.skipped_game_hours = 0
    $clone.unique_game_hour_count = 6346
    $clone.compact_stream.frame_time_us.avg = 18250.0
    $clone.compact_stream.frame_time_us.p95 = 24100.0
    $clone.compact_stream.frame_time_us.max = 41000.0
    $clone.compact_stream.total_bytes = 2100000000
    $clone.compact_stream.decodable = $true
    # Shrink the five delta domains' emitted rows to prove delta.
    foreach ($name in @("projects_live", "supply_logistics_attrition_live", "construction_buildings_state_damage_live", "scripted_state_flags_variables_live", "diplomacy_intel_operatives_live")) {
        $d = $clone.compact_stream.per_domain.$name
        $d.changed_rows_total = 12345
        $d.data_rows_total = 12345
        $d.read_time_us.avg = [double]$d.read_time_us.avg / 4.0
    }
    $clone.compact_stream.per_domain.diplomacy_intel_operatives_live.data_units_total |
        Add-Member -NotePropertyName "domain53_relation_shadow_mismatches" -NotePropertyValue 0 -Force
    # Attach host_finalize + hourly + read-mode fields the baseline lacked.
    $clone | Add-Member -NotePropertyName "d53_relation_read_mode" -NotePropertyValue "offsets" -Force
    $clone | Add-Member -NotePropertyName "host_finalize" -NotePropertyValue ([pscustomobject]@{
            peak_private_bytes    = 1181116000
            peak_working_set_bytes = 1300000000
            finalize_seconds      = 3.4
        }) -Force
    $clone.hourly_requirement.met = $true
    return $clone
}

function New-SyntheticShadowManifest {
    param([Parameter(Mandatory = $true)]$Baseline, [double]$Mismatches = 0, [string]$ReadMode = "shadow")
    $clone = $Baseline | ConvertTo-Json -Depth 60 | ConvertFrom-Json
    $clone.session_id = "synthetic_shadow"
    $clone | Add-Member -NotePropertyName "d53_relation_read_mode" -NotePropertyValue $ReadMode -Force
    $units = $clone.compact_stream.per_domain.diplomacy_intel_operatives_live.data_units_total
    if ($Mismatches -gt 0) {
        $units | Add-Member -NotePropertyName "domain53_relation_shadow_mismatches" -NotePropertyValue $Mismatches -Force
    } else {
        if (Test-JsonProp -Object $units -Name "domain53_relation_shadow_mismatches") {
            $units.PSObject.Properties.Remove("domain53_relation_shadow_mismatches")
        }
    }
    return $clone
}

function Invoke-SelfTest {
    param([Parameter(Mandatory = $true)][string]$BaselinePath)
    $failures = New-Object System.Collections.Generic.List[string]
    function Assert-True { param([bool]$Cond, [string]$Msg) if (-not $Cond) { $script:selfTestFailures.Add($Msg) } }

    $script:selfTestFailures = $failures

    Write-Host "== SELF TEST =="
    Write-Host "baseline: $BaselinePath"
    $baseline = Read-ManifestFile -Path $BaselinePath
    $baseMetrics = ConvertTo-SessionMetrics -Manifest $baseline -Label "baseline"

    # 1. Baseline metric extraction sanity (against the known 2026-07-09 numbers).
    Assert-True ([math]::Abs([double]$baseMetrics.FrameAvgMs - 69.9) -lt 0.2) "baseline frame avg should be ~69.9 ms, got $($baseMetrics.FrameAvgMs)"
    Assert-True ([math]::Abs([double]$baseMetrics.FrameP95Ms - 82.4) -lt 0.2) "baseline frame p95 should be ~82.4 ms, got $($baseMetrics.FrameP95Ms)"
    Assert-True ([math]::Abs([double]$baseMetrics.FrameMaxMs - 130.1) -lt 0.2) "baseline frame max should be ~130.1 ms, got $($baseMetrics.FrameMaxMs)"
    Assert-True ([int64]$baseMetrics.SkippedHours -eq 3897) "baseline skipped hours should be 3897, got $($baseMetrics.SkippedHours)"
    Assert-True ([int64]$baseMetrics.GameHours -eq 6346) "baseline game hours should be 6346, got $($baseMetrics.GameHours)"
    Assert-True ([int64]$baseMetrics.StreamBytes -eq 28150453026) "baseline stream bytes should be 28150453026, got $($baseMetrics.StreamBytes)"
    Assert-True ($baseMetrics.Domains.ContainsKey($D53_DOMAIN)) "baseline should contain d53 domain"
    Assert-True ($baseMetrics.Domains.Count -eq 55) "baseline should have 55 domains, got $($baseMetrics.Domains.Count)"

    # 2. Shadow gate: baseline was NOT a shadow run -> gate must FAIL (misconfigured), not falsely pass.
    $baseShadow = Test-ShadowGate -Metrics $baseMetrics
    Assert-True (-not $baseShadow.Pass) "baseline (non-shadow run) must not pass the shadow gate"

    # 3. Shadow gate: synthetic clean shadow run -> PASS.
    $shadowOk = ConvertTo-SessionMetrics -Manifest (New-SyntheticShadowManifest -Baseline $baseline -Mismatches 0 -ReadMode "shadow") -Label "shadow_ok"
    $okVerdict = Test-ShadowGate -Metrics $shadowOk
    Assert-True ($okVerdict.Pass) "clean shadow run should PASS the gate; problems=$($okVerdict.Problems -join '; ')"

    # 4. Shadow gate: synthetic mismatching shadow run -> FAIL (blocker).
    $shadowBad = ConvertTo-SessionMetrics -Manifest (New-SyntheticShadowManifest -Baseline $baseline -Mismatches 42 -ReadMode "shadow") -Label "shadow_bad"
    $badVerdict = Test-ShadowGate -Metrics $shadowBad
    Assert-True (-not $badVerdict.Pass) "shadow run with 42 mismatches must FAIL the gate"
    Assert-True ([double]$shadowBad.ShadowMismatches -eq 42) "mismatch count should parse to 42, got $($shadowBad.ShadowMismatches)"

    # 5. Paired verdict on synthetic improved head vs baseline.
    $headMetrics = ConvertTo-SessionMetrics -Manifest (New-SyntheticImprovedManifest -Baseline $baseline) -Label "synthetic_head"
    $verdict = New-PairedVerdict -Head $headMetrics -Baseline $baseMetrics -FrameAvgTargetMs $FrameAvgTargetMs -Phase2FrameAvgTargetMs $Phase2FrameAvgTargetMs
    Assert-True ($verdict.SkippedZeroPass) "improved head has 0 skipped -> SkippedZeroPass should be true"
    Assert-True ($verdict.FrameAvgUnder20) "improved head avg 18.25 ms -> should meet the 20 ms target"
    Assert-True ($verdict.FrameAvgUnder45) "improved head avg 18.25 ms -> should meet the 45 ms target"
    Assert-True ($null -ne $verdict.SizeFactorPerHour -and [double]$verdict.SizeFactorPerHour -gt 1.0) "improved head should show a size reduction > 1x"
    Assert-True ($verdict.HourlyGatePass) "improved head with met hourly + 0 skip -> hourly gate pass"

    # 6. Report renders without error and contains the VERDICT block.
    $report = Format-ComparisonReport -Head $headMetrics -Baseline $baseMetrics -Verdict $verdict -ShadowSummary "PASS (0 mismatches)"
    Assert-True ($report -match "## VERDICT") "report should contain a VERDICT block"
    Assert-True ($report -match "Delta domains") "report should contain the delta-domain table"

    Write-Host ""
    Write-Host "-- rendered synthetic report (excerpt) --"
    $verdictIdx = $report.IndexOf("## VERDICT")
    if ($verdictIdx -ge 0) { Write-Host $report.Substring($verdictIdx) }

    Write-Host ""
    if ($failures.Count -eq 0) {
        Write-Host "SELF TEST: PASS (all assertions green)"
        return 0
    }
    Write-Host "SELF TEST: FAIL"
    foreach ($f in $failures) { Write-Host "  - $f" }
    return 1
}

# ---------------------------------------------------------------------------
# Main.
# ---------------------------------------------------------------------------

$repoRoot = Resolve-RepoRoot
$nativeRoot = Join-Path $repoRoot "extractor\native"
$launcherPath = Join-Path $nativeRoot "scripts\Start-Hoi4ProductionSpeed5Capture.ps1"
$cargoManifest = Join-Path $nativeRoot "Cargo.toml"

if ($BaselineManifest.Length -eq 0) {
    $BaselineManifest = Join-Path $nativeRoot "runs\production_speed5_ShortSpeed5_20260709_124210\native_session_manifest.json"
}

if ($SelfTest) {
    $rc = Invoke-SelfTest -BaselinePath $BaselineManifest
    exit $rc
}

# Resolve the session output root.
$stamp = Get-Date -Format "yyyyMMdd_HHmmss"
if ($OutRoot.Length -eq 0) {
    $OutRoot = Join-Path $nativeRoot "runs\live_validation_$stamp"
}
$OutRoot = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($OutRoot)

Write-Host "=== the game live validation session ==="
Write-Host "repo_root=$repoRoot"
Write-Host "launcher=$launcherPath"
Write-Host "baseline=$BaselineManifest"
Write-Host "out_root=$OutRoot"

if (-not (Test-Path -LiteralPath $launcherPath -PathType Leaf)) {
    throw "launcher not found: $launcherPath"
}

# Parse baseline up front (fail fast if the baseline is missing/corrupt).
$baselineManifestObj = Read-ManifestFile -Path $BaselineManifest
$baselineMetrics = ConvertTo-SessionMetrics -Manifest $baselineManifestObj -Label "baseline"
Write-Host "baseline parsed: frame_avg=$(Format-Num $baselineMetrics.FrameAvgMs 1)ms skipped=$(Format-Num $baselineMetrics.SkippedHours 0) stream=$(Format-Bytes $baselineMetrics.StreamBytes)"

if ($DryRun) {
    Write-Host ""
    Write-Host "=== DRY RUN (no hoi4.exe attach) ==="
    $pidForDisplay = $Hoi4Pid
    if ($pidForDisplay -eq 0) { $pidForDisplay = "<auto-detect>" }

    $shadowOut = Join-Path $OutRoot "stage1_shadow"
    $abOut = Join-Path $OutRoot "stage2_shortspeed5"
    $emitOut = Join-Path $OutRoot "stage3_emitmode_full"

    Write-Host ""
    Write-Host "Stage 0 (preflight) would:"
    Write-Host "  - require exactly one hoi4.exe (pid=$pidForDisplay)"
    Write-Host "  - build: cargo build --release --manifest-path `"$cargoManifest`" -p collector -p recorder_host"
    if (-not $SkipProbe) {
        Write-Host "  - probe capture (~$(Format-Num $ProbeSeconds 0)s Smoke) and require game_hours_advanced > 0"
    }
    Write-Host ""
    Write-Host "Stage 1 (shadow gate) launcher invocation:"
    Write-Host "  powershell -NoProfile -File `"$launcherPath`" -Mode Smoke -Hoi4Pid $pidForDisplay -OutDir `"$shadowOut`" -SessionId live_validation_${stamp}_shadow -DurationSeconds $(Format-Num $ShadowSeconds 0) -MinGameHours 0 -D53RelationReadMode shadow -SkipBuild"
    Write-Host ""
    Write-Host "Stage 2 (paired A/B) launcher invocation:"
    if ($ShortSpeed5Seconds -gt 0) {
        Write-Host "  powershell -NoProfile -File `"$launcherPath`" -Mode ShortSpeed5 -Hoi4Pid $pidForDisplay -OutDir `"$abOut`" -SessionId live_validation_${stamp}_ab -DurationSeconds $(Format-ArgNumber $ShortSpeed5Seconds) -SkipBuild"
    } else {
        Write-Host "  powershell -NoProfile -File `"$launcherPath`" -Mode ShortSpeed5 -Hoi4Pid $pidForDisplay -OutDir `"$abOut`" -SessionId live_validation_${stamp}_ab -SkipBuild   # (-DurationSeconds omitted => launcher default 300s)"
    }
    if ($IncludeEmitModeAB) {
        Write-Host ""
        Write-Host "Stage 3 (emit-mode A/B) launcher invocation:"
        Write-Host "  powershell -NoProfile -File `"$launcherPath`" -Mode Smoke -Hoi4Pid $pidForDisplay -OutDir `"$emitOut`" -SessionId live_validation_${stamp}_emitfull -DurationSeconds $(Format-Num $EmitModeABSeconds 0) -MinGameHours 0 -EmitModeAll full -SkipBuild"
    }
    Write-Host ""
    Write-Host "=== comparison skeleton (head := baseline placeholder in dry run) ==="
    $skeletonVerdict = New-PairedVerdict -Head $baselineMetrics -Baseline $baselineMetrics -FrameAvgTargetMs $FrameAvgTargetMs -Phase2FrameAvgTargetMs $Phase2FrameAvgTargetMs
    Write-Host (Format-CompactTable -Head $baselineMetrics -Baseline $baselineMetrics -Verdict $skeletonVerdict)
    Write-Host ""
    Write-Host "DRY RUN complete: argument assembly + baseline parsing OK. No capture performed."
    exit 0
}

# --- Real session (attaches to hoi4.exe) ---
New-Item -ItemType Directory -Path $OutRoot -Force | Out-Null

if ($Hoi4Pid -eq 0) {
    $Hoi4Pid = Resolve-SingleHoi4Pid
}
Write-Host "hoi4_pid=$Hoi4Pid"

# Disk-space guard (delegate the detailed estimate to the launcher per-stage; do a
# coarse floor here so we abort before building if the drive is nearly full).
if (-not $SkipDiskSpaceCheck) {
    $root = [System.IO.Path]::GetPathRoot($OutRoot)
    if ($root.Length -ge 2 -and $root[1] -eq ":") {
        $drive = Get-PSDrive -Name $root.Substring(0, 1) -PSProvider FileSystem
        $freeGb = [math]::Round($drive.Free / 1GB, 2)
        Write-Host "disk_check root=$root free_gb=$freeGb"
        if ($drive.Free -lt (5GB)) {
            throw "less than 5 GB free on $root; a speed-5 capture needs tens of GB. Free space or pass -SkipDiskSpaceCheck."
        }
    }
}

# Build once; all stage launcher calls then use -SkipBuild.
if (-not $SkipBuild) {
    Write-Host "building collector + recorder_host (release)..."
    & cargo build --release --manifest-path $cargoManifest -p collector -p recorder_host
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed (rc=$LASTEXITCODE)" }
}

# Game-advancing probe.
if (-not $SkipProbe) {
    Write-Host ""
    Write-Host "=== Preflight: game-advancing probe (~$(Format-Num $ProbeSeconds 0)s) ==="
    $probeOut = Join-Path $OutRoot "stage0_probe"
    $probeManifestPath = Invoke-LauncherCapture -LauncherPath $launcherPath -Mode Smoke `
        -OutDir $probeOut -SessionId "live_validation_${stamp}_probe" -Hoi4Pid $Hoi4Pid `
        -DurationSeconds $ProbeSeconds -MinGameHours 0 -SkipBuild -SkipDiskSpaceCheck:$SkipDiskSpaceCheck
    $probeMetrics = ConvertTo-SessionMetrics -Manifest (Read-ManifestFile -Path $probeManifestPath) -Label "probe"
    if ($null -eq $probeMetrics.GameHours -or [double]$probeMetrics.GameHours -le 0) {
        throw "PROBE FAILED: game_hours_advanced=$($probeMetrics.GameHours). The game appears PAUSED. UNPAUSE and set SPEED 5, then re-run."
    }
    Write-Host "probe OK: game_hours_advanced=$($probeMetrics.GameHours) (game is advancing)"
}

# === Stage 1: d53 shadow gate ===
Write-Host ""
Write-Host "=== Stage 1: d53 shadow gate (~$(Format-Num $ShadowSeconds 0)s) ==="
$shadowOut = Join-Path $OutRoot "stage1_shadow"
$shadowManifestPath = Invoke-LauncherCapture -LauncherPath $launcherPath -Mode Smoke `
    -OutDir $shadowOut -SessionId "live_validation_${stamp}_shadow" -Hoi4Pid $Hoi4Pid `
    -DurationSeconds $ShadowSeconds -MinGameHours 0 -D53RelationReadMode "shadow" -SkipBuild -SkipDiskSpaceCheck:$SkipDiskSpaceCheck
$shadowMetrics = ConvertTo-SessionMetrics -Manifest (Read-ManifestFile -Path $shadowManifestPath) -Label "shadow"
$shadowVerdict = Test-ShadowGate -Metrics $shadowMetrics

$shadowSummaryLines = @()
if ($shadowVerdict.Pass) {
    $shadowSummaryLines += "PASS - domain53_relation_shadow_mismatches == 0 on every d53 frame (read_mode=$($shadowVerdict.ReadMode))."
} else {
    $shadowSummaryLines += "FAIL - d53 shadow gate BLOCKER:"
    foreach ($p in $shadowVerdict.Problems) { $shadowSummaryLines += "  - $p" }
}
$shadowSummary = ($shadowSummaryLines -join [Environment]::NewLine)
Write-Host $shadowSummary

if (-not $shadowVerdict.Pass) {
    throw "STAGE 1 BLOCKER: d53 shadow gate failed. The offsets relation reader diverged from the legacy scan (or the run was misconfigured). Do NOT proceed to production capture; investigate before the paired A/B."
}

# === Stage 2: paired A/B ===
Write-Host ""
Write-Host "=== Stage 2: paired A/B (ShortSpeed5 vs 2026-07-09 baseline) ==="
$abOut = Join-Path $OutRoot "stage2_shortspeed5"
$abManifestPath = Invoke-LauncherCapture -LauncherPath $launcherPath -Mode ShortSpeed5 `
    -OutDir $abOut -SessionId "live_validation_${stamp}_ab" -Hoi4Pid $Hoi4Pid `
    -DurationSeconds $ShortSpeed5Seconds -SkipBuild -SkipDiskSpaceCheck:$SkipDiskSpaceCheck
$headMetrics = ConvertTo-SessionMetrics -Manifest (Read-ManifestFile -Path $abManifestPath) -Label "head"
$pairedVerdict = New-PairedVerdict -Head $headMetrics -Baseline $baselineMetrics -FrameAvgTargetMs $FrameAvgTargetMs -Phase2FrameAvgTargetMs $Phase2FrameAvgTargetMs

# === Stage 3: emit-mode A/B (optional) ===
$emitSummary = ""
if ($IncludeEmitModeAB) {
    Write-Host ""
    Write-Host "=== Stage 3: emit-mode A/B (all five delta domains emit_mode=full) ==="
    $emitOut = Join-Path $OutRoot "stage3_emitmode_full"
    $emitManifestPath = Invoke-LauncherCapture -LauncherPath $launcherPath -Mode Smoke `
        -OutDir $emitOut -SessionId "live_validation_${stamp}_emitfull" -Hoi4Pid $Hoi4Pid `
        -DurationSeconds $EmitModeABSeconds -MinGameHours 0 -EmitModeAll "full" -SkipBuild -SkipDiskSpaceCheck:$SkipDiskSpaceCheck
    $emitMetrics = ConvertTo-SessionMetrics -Manifest (Read-ManifestFile -Path $emitManifestPath) -Label "emit_full"
    $emitFactor = "n/a"
    if ($null -ne $emitMetrics.BytesPerHour -and [double]$emitMetrics.BytesPerHour -gt 0 -and $null -ne $headMetrics.BytesPerHour) {
        $emitFactor = ("{0}x" -f (Format-Num ([double]$emitMetrics.BytesPerHour / [double]$headMetrics.BytesPerHour) 2))
    }
    $emitLines = @()
    $emitLines += "Delta (head, all delta) bytes/captured-hour: $(Format-Bytes $headMetrics.BytesPerHour)"
    $emitLines += "Full (emit_mode=full) bytes/captured-hour:   $(Format-Bytes $emitMetrics.BytesPerHour)"
    $emitLines += "Delta size advantage (full / delta): $emitFactor"
    $emitSummary = ($emitLines -join [Environment]::NewLine)
    Write-Host $emitSummary
}

# === Report ===
$report = Format-ComparisonReport -Head $headMetrics -Baseline $baselineMetrics -Verdict $pairedVerdict -ShadowSummary $shadowSummary -EmitModeSummary $emitSummary
$reportPath = Join-Path $OutRoot "live_validation_report.md"
Set-Content -LiteralPath $reportPath -Value $report -Encoding UTF8

Write-Host ""
Write-Host "=== paired A/B (compact) ==="
Write-Host (Format-CompactTable -Head $headMetrics -Baseline $baselineMetrics -Verdict $pairedVerdict)
Write-Host ""
Write-Host "report written: $reportPath"

# Overall exit code: shadow already gated (threw on fail); fail the session if
# the paired run lost data (skipped hours) - that violates zero-data-loss.
if (-not $pairedVerdict.SkippedZeroPass) {
    Write-Host "SESSION VERDICT: paired A/B has skipped hours (data loss). Reported for triage; exiting non-zero."
    exit 2
}
Write-Host "SESSION VERDICT: shadow gate PASS, paired A/B zero-skip. See report for perf/size verdict."
exit 0
