<#
.SYNOPSIS
    Project a short production capture's stream size out to a full game
    (MASTER_EXTRACTION_CONTRACT F6 tooling).

.DESCRIPTION
    Reads a capture run's native_session_manifest.json (and, when present, cross-
    checks the on-disk stream file), derives decoded and stored bytes-per-game-hour
    from the hours the capture actually advanced, and extrapolates linearly to a
    configurable full-game hour count. Emits a JSON artifact plus a human summary.

    Linear per-hour extrapolation is the honest model for a per-hour cadence
    stream: every game hour emits one world snapshot's worth of records, so total
    bytes scale with hours advanced. The projection is labeled as an estimate and
    carries its inputs so a reviewer can redo the arithmetic.

    FAIL-CLOSED: a missing manifest, missing byte counts, or a capture that did not
    advance at least one game hour is an error, never a fabricated zero.

    Run with -SelfTest to exercise the arithmetic over a fabricated run directory
    (no live game required). PowerShell 5.1 compatible. No rig hardcoding.

.PARAMETER RunDir
    Capture run directory containing native_session_manifest.json.

.PARAMETER FullGameHours
    Target full-game hour count to project to. Default 105192 = a 1936-01-01 to
    1948-01-01 campaign (12 years x 365.25 d x 24 h). Override for other lengths.

.PARAMETER OutFile
    JSON artifact path. Default <RunDir>\full_game_size_projection.json.

.PARAMETER SelfTest
    Run offline arithmetic self-tests and exit.
#>
param(
    [string]$RunDir = "",

    [ValidateRange(1, 10000000)]
    [int]$FullGameHours = 105192,

    [string]$OutFile = "",

    [switch]$SelfTest
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

# Capture our own switches BEFORE dot-sourcing: ProductionCapture.Validation.ps1
# has its own `param([switch]$SelfTest)`, and dot-sourcing runs its param block in
# THIS scope, which would otherwise reset $SelfTest to its (false) default.
$runSelfTest = [bool]$SelfTest

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
. (Join-Path $scriptDir "ProductionCapture.Validation.ps1")

function Get-RequiredNumber {
    <#
        Resolve a numeric manifest field, fail-closed. Throws when the field is
        absent, null, or non-numeric so a projection can never rest on a guess.
    #>
    param(
        [Parameter(Mandatory = $true)]$Manifest,
        [Parameter(Mandatory = $true)][string]$Path
    )
    $resolved = Resolve-JsonPath -Manifest $Manifest -Path $Path
    if (-not $resolved.Found) {
        throw "manifest field '$Path' is missing (cannot project size without it)"
    }
    if ($null -eq $resolved.Value) {
        throw "manifest field '$Path' is null (cannot project size without it)"
    }
    $parsed = 0.0
    if (-not [double]::TryParse(
            [string]$resolved.Value,
            [System.Globalization.NumberStyles]::Float,
            [System.Globalization.CultureInfo]::InvariantCulture,
            [ref]$parsed)) {
        throw "manifest field '$Path' is not numeric ('$($resolved.Value)')"
    }
    return $parsed
}

function New-SizeProjection {
    <#
        Core arithmetic, split out so the self-test can drive it without touching
        disk. Returns the projection artifact object.
    #>
    param(
        [Parameter(Mandatory = $true)][double]$DecodedBytes,
        [Parameter(Mandatory = $true)][double]$StoredBytes,
        [Parameter(Mandatory = $true)][double]$HoursAdvanced,
        [Parameter(Mandatory = $true)][int]$FullGameHours,
        [double]$ObservedStreamFileBytes = -1
    )
    if ($HoursAdvanced -le 0) {
        throw "capture advanced $HoursAdvanced game hours; need >= 1 to project a per-hour rate"
    }
    $decodedPerHour = $DecodedBytes / $HoursAdvanced
    $storedPerHour = $StoredBytes / $HoursAdvanced
    $projectedDecoded = $decodedPerHour * $FullGameHours
    $projectedStored = $storedPerHour * $FullGameHours

    $mb = 1024.0 * 1024.0
    $gb = $mb * 1024.0

    return [ordered]@{
        schema = "full_game_size_projection_v1"
        model  = "linear_per_game_hour"
        note   = "estimate: total stream bytes scale linearly with game hours advanced at the per-hour cadence; redo from the inputs block"
        inputs = [ordered]@{
            decoded_bytes              = [int64]$DecodedBytes
            stored_bytes               = [int64]$StoredBytes
            observed_stream_file_bytes = if ($ObservedStreamFileBytes -ge 0) { [int64]$ObservedStreamFileBytes } else { $null }
            game_hours_advanced        = $HoursAdvanced
            full_game_hours            = $FullGameHours
        }
        rates = [ordered]@{
            decoded_bytes_per_game_hour = [math]::Round($decodedPerHour, 3)
            stored_bytes_per_game_hour  = [math]::Round($storedPerHour, 3)
            decoded_mb_per_game_hour    = [math]::Round($decodedPerHour / $mb, 6)
            stored_mb_per_game_hour     = [math]::Round($storedPerHour / $mb, 6)
        }
        projection = [ordered]@{
            projected_decoded_bytes = [int64]$projectedDecoded
            projected_stored_bytes  = [int64]$projectedStored
            projected_decoded_gb    = [math]::Round($projectedDecoded / $gb, 3)
            projected_stored_gb     = [math]::Round($projectedStored / $gb, 3)
        }
    }
}

function Invoke-SizeProjection {
    param(
        [Parameter(Mandatory = $true)][string]$RunDir,
        [Parameter(Mandatory = $true)][int]$FullGameHours,
        [string]$OutFile = ""
    )
    $manifest = Read-CaptureManifest -RunDir $RunDir

    $decodedBytes = Get-RequiredNumber -Manifest $manifest -Path "stream_storage.decoded_bytes"
    $storedBytes = Get-RequiredNumber -Manifest $manifest -Path "stream_storage.stored_bytes"
    $hoursAdvanced = Get-RequiredNumber -Manifest $manifest -Path "game_hours_advanced"

    # Cross-check against the actual stream file on disk when the manifest names
    # it. FAIL-CLOSED: a named-but-missing stream file, or an on-disk length that
    # disagrees with the manifest's stored_bytes, means the capture is truncated
    # or the manifest is lying about its size — either way the projection must not
    # rest on an unverifiable stored_bytes (e.g. a 1-byte file vs stored_bytes=999).
    # An ABSENT or null raw_stream means there is no on-disk stream to cross-check
    # stored_bytes against, so stored_bytes is unverifiable. That is a fail-closed
    # ERROR (no projection), exactly like a named-but-missing file or a size
    # mismatch: a projection must never rest on an uncorroborated stored_bytes.
    $rawStream = Resolve-JsonPath -Manifest $manifest -Path "raw_stream"
    if (-not $rawStream.Found -or $null -eq $rawStream.Value -or [string]::IsNullOrWhiteSpace([string]$rawStream.Value)) {
        throw "manifest has no raw_stream field (absent or null); cannot cross-check stored_bytes against an on-disk stream, so the projection is refused (fail-closed)"
    }
    # CONTAIN the raw_stream to RunDir (finding 5). raw_stream is a manifest-supplied
    # (user-writable) path; a value like "..\unrelated.bin" or an absolute path could
    # otherwise cross-check stored_bytes against a file OUTSIDE this run whose length
    # happens to match, laundering an unverifiable stored_bytes into a "corroborated"
    # projection. Resolve the combined path and require it to live INSIDE RunDir; use
    # [IO.Path]::Combine so an absolute/rooted raw_stream stays absolute (and thus
    # outside), then normalize away any ".." before the containment check.
    $rawValue = [string]$rawStream.Value
    $runDirFull = [System.IO.Path]::GetFullPath($RunDir)
    $runDirPrefix = $runDirFull.TrimEnd([char]'\', [char]'/') + [System.IO.Path]::DirectorySeparatorChar
    $streamPath = [System.IO.Path]::GetFullPath([System.IO.Path]::Combine($RunDir, $rawValue))
    if (-not $streamPath.StartsWith($runDirPrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "manifest raw_stream '$rawValue' resolves to '$streamPath', which is OUTSIDE the run directory '$runDirFull'; refusing to cross-check stored_bytes against a file outside RunDir (path-escape; fail-closed)"
    }
    if (-not (Test-Path -LiteralPath $streamPath -PathType Leaf)) {
        throw "manifest names raw_stream '$($rawStream.Value)' but that file is missing under '$RunDir'; cannot cross-check stored_bytes (fail-closed)"
    }
    $observedStreamFileBytes = [double]((Get-Item -LiteralPath $streamPath).Length)
    if ([int64]$observedStreamFileBytes -ne [int64]$storedBytes) {
        throw "stream file '$($rawStream.Value)' is $([int64]$observedStreamFileBytes) bytes on disk but the manifest stream_storage.stored_bytes is $([int64]$storedBytes) bytes; truncated/corrupt capture or wrong manifest (fail-closed)"
    }

    $projection = New-SizeProjection -DecodedBytes $decodedBytes -StoredBytes $storedBytes `
        -HoursAdvanced $hoursAdvanced -FullGameHours $FullGameHours `
        -ObservedStreamFileBytes $observedStreamFileBytes

    if ($OutFile.Length -eq 0) {
        $OutFile = Join-Path $RunDir "full_game_size_projection.json"
    }
    $projection | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $OutFile -Encoding UTF8

    Write-Host "full-game size projection ($($projection.model)):"
    Write-Host ("  capture: {0:N0} decoded bytes over {1} game hours" -f $projection.inputs.decoded_bytes, $projection.inputs.game_hours_advanced)
    Write-Host ("  rate:    {0:N3} MB / game hour (decoded), {1:N3} MB / game hour (stored)" -f $projection.rates.decoded_mb_per_game_hour, $projection.rates.stored_mb_per_game_hour)
    Write-Host ("  full game ({0} h): {1:N3} GB decoded, {2:N3} GB stored" -f $FullGameHours, $projection.projection.projected_decoded_gb, $projection.projection.projected_stored_gb)
    Write-Host "  artifact: $OutFile"
    return $projection
}

# ---------------------------------------------------------------------------
# Offline self-test: fabricate a run directory and verify the arithmetic.
# ---------------------------------------------------------------------------

function Invoke-SizeProjectionSelfTest {
    $failures = [System.Collections.Generic.List[string]]::new()
    function Assert-True {
        param([bool]$Condition, [string]$Message)
        if (-not $Condition) { $failures.Add($Message) }
    }

    # 1. Pure-arithmetic path: 1,000,000 decoded bytes over 10 hours = 100000 B/h;
    #    project to 1000 hours = 100,000,000 bytes.
    $p = New-SizeProjection -DecodedBytes 1000000 -StoredBytes 500000 -HoursAdvanced 10 -FullGameHours 1000
    Assert-True ($p.rates.decoded_bytes_per_game_hour -eq 100000) "decoded rate should be 100000, got $($p.rates.decoded_bytes_per_game_hour)"
    Assert-True ($p.projection.projected_decoded_bytes -eq 100000000) "projected decoded should be 100000000, got $($p.projection.projected_decoded_bytes)"
    Assert-True ($p.projection.projected_stored_bytes -eq 50000000) "projected stored should be 50000000, got $($p.projection.projected_stored_bytes)"

    # 2. Fail-closed on zero hours advanced.
    $threw = $false
    try { New-SizeProjection -DecodedBytes 100 -StoredBytes 100 -HoursAdvanced 0 -FullGameHours 1000 | Out-Null }
    catch { $threw = $true }
    Assert-True $threw "zero hours advanced must throw"

    # 3. End-to-end over a fabricated run directory + manifest + stream file.
    $tmp = Join-Path ([System.IO.Path]::GetTempPath()) ("sizeproj_selftest_" + [guid]::NewGuid().ToString("N"))
    New-Item -ItemType Directory -Path $tmp -Force | Out-Null
    try {
        $streamName = "collector_stream.bin"
        [System.IO.File]::WriteAllBytes((Join-Path $tmp $streamName), (New-Object byte[] 2048))
        $manifest = [ordered]@{
            schema              = "native_collector_session_v1"
            raw_stream          = $streamName
            game_hours_advanced = 4
            stream_storage      = [ordered]@{
                compression   = "zstd"
                decoded_bytes = 8000
                stored_bytes  = 2048
            }
        }
        $manifest | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $tmp "native_session_manifest.json") -Encoding UTF8

        $outFile = Join-Path $tmp "projection.json"
        $result = Invoke-SizeProjection -RunDir $tmp -FullGameHours 400 -OutFile $outFile
        Assert-True (Test-Path -LiteralPath $outFile) "projection artifact should be written"
        # 8000 decoded / 4 h = 2000 B/h -> 400 h = 800000 bytes decoded.
        Assert-True ($result.projection.projected_decoded_bytes -eq 800000) "e2e projected decoded should be 800000, got $($result.projection.projected_decoded_bytes)"
        Assert-True ($result.inputs.observed_stream_file_bytes -eq 2048) "observed stream file bytes should be 2048, got $($result.inputs.observed_stream_file_bytes)"

        # 4. Fail-closed when a required byte count is absent.
        $badManifest = [ordered]@{
            schema              = "native_collector_session_v1"
            game_hours_advanced = 4
            stream_storage      = [ordered]@{ compression = "zstd"; stored_bytes = 10 }
        }
        $badDir = Join-Path $tmp "bad"
        New-Item -ItemType Directory -Path $badDir -Force | Out-Null
        $badManifest | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $badDir "native_session_manifest.json") -Encoding UTF8
        $threw2 = $false
        try { Invoke-SizeProjection -RunDir $badDir -FullGameHours 400 -OutFile (Join-Path $badDir "p.json") | Out-Null }
        catch { $threw2 = $true }
        Assert-True $threw2 "missing decoded_bytes must throw fail-closed"

        # 5. Cross-check FAIL-CLOSED: the manifest names a raw_stream that is not on
        #    disk -> cannot verify stored_bytes -> must throw (not project blindly).
        $missDir = Join-Path $tmp "missing_stream"
        New-Item -ItemType Directory -Path $missDir -Force | Out-Null
        $missManifest = [ordered]@{
            schema              = "native_collector_session_v1"
            raw_stream          = "collector_stream.bin"
            game_hours_advanced = 4
            stream_storage      = [ordered]@{ compression = "zstd"; decoded_bytes = 8000; stored_bytes = 2048 }
        }
        $missManifest | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $missDir "native_session_manifest.json") -Encoding UTF8
        $threw3 = $false
        try { Invoke-SizeProjection -RunDir $missDir -FullGameHours 400 -OutFile (Join-Path $missDir "p.json") | Out-Null }
        catch { $threw3 = $true }
        Assert-True $threw3 "a named-but-missing stream file must throw fail-closed"

        # 6. Cross-check FAIL-CLOSED: a 1-byte stream on disk vs stored_bytes=999 in
        #    the manifest is a size mismatch -> must throw, never project from 999.
        $mismatchDir = Join-Path $tmp "mismatch_stream"
        New-Item -ItemType Directory -Path $mismatchDir -Force | Out-Null
        [System.IO.File]::WriteAllBytes((Join-Path $mismatchDir "collector_stream.bin"), (New-Object byte[] 1))
        $mismatchManifest = [ordered]@{
            schema              = "native_collector_session_v1"
            raw_stream          = "collector_stream.bin"
            game_hours_advanced = 4
            stream_storage      = [ordered]@{ compression = "zstd"; decoded_bytes = 8000; stored_bytes = 999 }
        }
        $mismatchManifest | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $mismatchDir "native_session_manifest.json") -Encoding UTF8
        $threw4 = $false
        $mismatchMsg = ""
        try { Invoke-SizeProjection -RunDir $mismatchDir -FullGameHours 400 -OutFile (Join-Path $mismatchDir "p.json") | Out-Null }
        catch { $threw4 = $true; $mismatchMsg = $_.Exception.Message }
        Assert-True $threw4 "a stream-file length that disagrees with stored_bytes must throw fail-closed"
        Assert-True ($mismatchMsg -match "stored_bytes") "the mismatch error must name the stored_bytes disagreement"

        # 7. Cross-check FAIL-CLOSED (finding 5): a manifest with valid byte counts
        #    and hours but NO raw_stream field cannot corroborate stored_bytes, so
        #    the projection is refused and NO artifact is written. Fail-before: the
        #    same manifest WITH a matching raw_stream (test 3) projects fine.
        $noRawDir = Join-Path $tmp "no_raw_stream"
        New-Item -ItemType Directory -Path $noRawDir -Force | Out-Null
        $noRawManifest = [ordered]@{
            schema              = "native_collector_session_v1"
            game_hours_advanced = 4
            stream_storage      = [ordered]@{ compression = "zstd"; decoded_bytes = 8000; stored_bytes = 2048 }
        }
        $noRawManifest | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $noRawDir "native_session_manifest.json") -Encoding UTF8
        $noRawOut = Join-Path $noRawDir "p.json"
        $threw5 = $false
        $noRawMsg = ""
        try { Invoke-SizeProjection -RunDir $noRawDir -FullGameHours 400 -OutFile $noRawOut | Out-Null }
        catch { $threw5 = $true; $noRawMsg = $_.Exception.Message }
        Assert-True $threw5 "an absent raw_stream must throw fail-closed (no projection)"
        Assert-True ($noRawMsg -match "raw_stream") "the error must name the missing raw_stream field"
        Assert-True (-not (Test-Path -LiteralPath $noRawOut -PathType Leaf)) "no projection artifact may be written when raw_stream is absent"

        # 8. Same fail-closed path when raw_stream is present but JSON null.
        $nullRawDir = Join-Path $tmp "null_raw_stream"
        New-Item -ItemType Directory -Path $nullRawDir -Force | Out-Null
        $nullRawManifest = [ordered]@{
            schema              = "native_collector_session_v1"
            raw_stream          = $null
            game_hours_advanced = 4
            stream_storage      = [ordered]@{ compression = "zstd"; decoded_bytes = 8000; stored_bytes = 2048 }
        }
        $nullRawManifest | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $nullRawDir "native_session_manifest.json") -Encoding UTF8
        $threw6 = $false
        try { Invoke-SizeProjection -RunDir $nullRawDir -FullGameHours 400 -OutFile (Join-Path $nullRawDir "p.json") | Out-Null }
        catch { $threw6 = $true }
        Assert-True $threw6 "a null raw_stream must throw fail-closed (no projection)"

        # 9. CONTAINMENT (finding 5): a raw_stream that PATH-ESCAPES RunDir ("..\x")
        #    must be REFUSED even when the escaped file exists AND its length matches
        #    stored_bytes — the cross-check must never rest on a file outside RunDir.
        #    Fail-before: an in-RunDir raw_stream of the same bytes (test 3) projects.
        $escDir = Join-Path $tmp "escape_stream"
        New-Item -ItemType Directory -Path $escDir -Force | Out-Null
        # Plant a matching-length file in the PARENT (outside RunDir) that the escape
        # would point at, so only the containment check (not the length check) rejects.
        [System.IO.File]::WriteAllBytes((Join-Path $tmp "outside_stream.bin"), (New-Object byte[] 2048))
        $escManifest = [ordered]@{
            schema              = "native_collector_session_v1"
            raw_stream          = "..\outside_stream.bin"
            game_hours_advanced = 4
            stream_storage      = [ordered]@{ compression = "zstd"; decoded_bytes = 8000; stored_bytes = 2048 }
        }
        $escManifest | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $escDir "native_session_manifest.json") -Encoding UTF8
        $escOut = Join-Path $escDir "p.json"
        $threw7 = $false
        $escMsg = ""
        try { Invoke-SizeProjection -RunDir $escDir -FullGameHours 400 -OutFile $escOut | Out-Null }
        catch { $threw7 = $true; $escMsg = $_.Exception.Message }
        Assert-True $threw7 "a raw_stream that escapes RunDir ('..\\x') must throw fail-closed even if the outside file's length matches stored_bytes"
        Assert-True ($escMsg -match "OUTSIDE the run directory") "the error must name the path-escape/containment violation"
        Assert-True (-not (Test-Path -LiteralPath $escOut -PathType Leaf)) "no projection artifact may be written for a path-escaping raw_stream"

        # 10. CONTAINMENT: an ABSOLUTE raw_stream pointing outside RunDir is likewise
        #     refused (Combine keeps a rooted path absolute, so it stays outside).
        $absDir = Join-Path $tmp "absolute_stream"
        New-Item -ItemType Directory -Path $absDir -Force | Out-Null
        $absTarget = Join-Path $tmp "outside_stream.bin"   # the 2048-byte file planted above
        $absManifest = [ordered]@{
            schema              = "native_collector_session_v1"
            raw_stream          = $absTarget
            game_hours_advanced = 4
            stream_storage      = [ordered]@{ compression = "zstd"; decoded_bytes = 8000; stored_bytes = 2048 }
        }
        $absManifest | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $absDir "native_session_manifest.json") -Encoding UTF8
        $threw8 = $false
        try { Invoke-SizeProjection -RunDir $absDir -FullGameHours 400 -OutFile (Join-Path $absDir "p.json") | Out-Null }
        catch { $threw8 = $true }
        Assert-True $threw8 "an absolute raw_stream outside RunDir must throw fail-closed"

        # 11. CONTAINMENT fail-before/pass-after: a raw_stream in a SUBDIRECTORY of
        #     RunDir is still inside and must project fine.
        $subDir = Join-Path $tmp "sub_stream"
        New-Item -ItemType Directory -Path (Join-Path $subDir "nested") -Force | Out-Null
        [System.IO.File]::WriteAllBytes((Join-Path $subDir "nested\collector_stream.bin"), (New-Object byte[] 2048))
        $subManifest = [ordered]@{
            schema              = "native_collector_session_v1"
            raw_stream          = "nested\collector_stream.bin"
            game_hours_advanced = 4
            stream_storage      = [ordered]@{ compression = "zstd"; decoded_bytes = 8000; stored_bytes = 2048 }
        }
        $subManifest | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $subDir "native_session_manifest.json") -Encoding UTF8
        $subResult = Invoke-SizeProjection -RunDir $subDir -FullGameHours 400 -OutFile (Join-Path $subDir "p.json")
        Assert-True ($subResult.inputs.observed_stream_file_bytes -eq 2048) "an in-RunDir subdirectory raw_stream must be accepted and cross-checked"
    } finally {
        Remove-Item -LiteralPath $tmp -Recurse -Force -ErrorAction SilentlyContinue
    }

    if ($failures.Count -gt 0) {
        Write-Host "Get-FullGameSizeProjection self-test FAILED:" -ForegroundColor Red
        foreach ($f in $failures) { Write-Host "  - $f" -ForegroundColor Red }
        return $false
    }
    Write-Host "Get-FullGameSizeProjection self-test: all checks passed."
    return $true
}

if ($runSelfTest) {
    if (Invoke-SizeProjectionSelfTest) { exit 0 } else { exit 1 }
}

if ($RunDir.Length -eq 0) {
    throw "-RunDir is required (or pass -SelfTest to run the offline arithmetic checks)."
}
Invoke-SizeProjection -RunDir $RunDir -FullGameHours $FullGameHours -OutFile $OutFile | Out-Null
