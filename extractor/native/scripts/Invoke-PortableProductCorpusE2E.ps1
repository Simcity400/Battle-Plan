[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$PackageDirectory,
    [Parameter(Mandatory = $true)][string]$CorpusManifest,
    [Parameter(Mandatory = $true)][string]$GameInstall,
    [Parameter(Mandatory = $true)][string]$UserData,
    [Parameter(Mandatory = $true)][string]$OutputDirectory
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

# Manifest contract:
# {
#   "schema": "hoi4-portable-product-real-corpus-e2e-v1",
#   "recordings": [{
#     "id": "stable-id",
#     "provenance": "recorder_production_output",
#     "path": "absolute/or/manifest-relative.battleplan",
#     "sha256": "<64 lowercase hex>",
#     "render_hours": "all",
#     "logical_stream_id": 0 // optional, only for a rollover package member
#   }]
# }
# Generated fixtures are intentionally not accepted as a replacement for this
# manifest. Its recording hashes are the corpus selection authority.

function Resolve-ExistingFile([string]$Path, [string]$Label) {
    $full = [IO.Path]::GetFullPath($Path)
    if (-not [IO.File]::Exists($full)) {
        throw "$Label does not exist: $full"
    }
    $item = Get-Item -LiteralPath $full -Force
    if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw "$Label must not be a reparse point: $full"
    }
    $full
}

function Resolve-ExistingDirectory([string]$Path, [string]$Label) {
    $full = [IO.Path]::GetFullPath($Path)
    if (-not [IO.Directory]::Exists($full)) {
        throw "$Label does not exist: $full"
    }
    $full
}

function Get-Sha256([string]$Path) {
    (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Invoke-Replay([string]$Exe, [string[]]$Arguments, [string]$Label) {
    $prior = $ErrorActionPreference
    try {
        $ErrorActionPreference = 'Continue'
        $lines = @(& $Exe @Arguments 2>&1)
        $exitCode = $LASTEXITCODE
    }
    finally {
        $ErrorActionPreference = $prior
    }
    $text = ($lines | ForEach-Object { [string]$_ }) -join "`n"
    if ($exitCode -ne 0) {
        throw "$Label failed with exit code $exitCode`n$text"
    }
    [pscustomobject]@{
        exit_code = $exitCode
        output = $text
    }
}

function Assert-PackageChecksums([string]$Root) {
    $checksumPath = Resolve-ExistingFile (Join-Path $Root 'SHA256SUMS.txt') 'Package checksum manifest'
    $expectedNames = @(
        'Battle Plan Recorder.exe',
        'Battle Plan.exe',
        'collector.dll'
    )
    $expected = @{}
    foreach ($line in [IO.File]::ReadAllLines($checksumPath)) {
        if ($line -notmatch '^([0-9a-f]{64})  (.+)$') {
            throw "Malformed package checksum line: $line"
        }
        if ($expected.ContainsKey($Matches[2])) {
            throw "Repeated package checksum entry: $($Matches[2])"
        }
        $expected[$Matches[2]] = $Matches[1]
    }
    if ($expected.Count -ne $expectedNames.Count) {
        throw 'Package checksum manifest must name exactly the recorder, replay app, and collector'
    }
    foreach ($name in $expectedNames) {
        if (-not $expected.ContainsKey($name)) {
            throw "Package checksum manifest is missing $name"
        }
        $path = Resolve-ExistingFile (Join-Path $Root $name) "Package entry $name"
        if ((Get-Sha256 $path) -cne $expected[$name]) {
            throw "Package entry hash mismatch: $name"
        }
    }
}

$package = Resolve-ExistingDirectory $PackageDirectory 'Extracted portable package'
$manifestPath = Resolve-ExistingFile $CorpusManifest 'Corpus manifest'
$game = Resolve-ExistingDirectory $GameInstall 'game installation'
$user = Resolve-ExistingDirectory $UserData 'the game user-data directory'
$output = [IO.Path]::GetFullPath($OutputDirectory)
if ([IO.Directory]::Exists($output) -and
    @(Get-ChildItem -LiteralPath $output -Force).Count -ne 0) {
    throw "Output directory must be absent or empty: $output"
}
[IO.Directory]::CreateDirectory($output) | Out-Null

Assert-PackageChecksums $package
$replayExe = Resolve-ExistingFile (Join-Path $package 'Battle Plan.exe') 'Shipped replay executable'

$manifestBytes = [IO.File]::ReadAllBytes($manifestPath)
$manifest = [Text.Encoding]::UTF8.GetString($manifestBytes) | ConvertFrom-Json
if ([string]$manifest.schema -cne 'hoi4-portable-product-real-corpus-e2e-v1') {
    throw 'Unsupported corpus manifest schema'
}
$recordings = @($manifest.recordings)
if ($recordings.Count -eq 0) {
    throw 'The real-corpus E2E manifest contains no recordings'
}

$manifestRoot = Split-Path -Parent $manifestPath
$decoderId = 'extractor.replay_decoder.production_world_fold.worker.v1'
$roleCoverage = [ordered]@{
    compact_prefold_worldframe = $false
    map_output = $false
    selected_present_ui = $false
}
$typedRoleCoverage = [ordered]@{}
$recordingReceipts = @()

foreach ($recording in $recordings) {
    $id = [string]$recording.id
    if ($id -cnotmatch '^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$') {
        throw "Invalid recording id: $id"
    }
    if ([string]$recording.provenance -cne 'recorder_production_output') {
        throw "$id is not declared as an actual recorder production output"
    }
    if ([string]$recording.render_hours -cne 'all') {
        throw "$id must render every indexed hour; render_hours must be exactly `"all`""
    }
    $declaredPath = [string]$recording.path
    $candidate = if ([IO.Path]::IsPathRooted($declaredPath)) {
        $declaredPath
    }
    else {
        Join-Path $manifestRoot $declaredPath
    }
    $recordingPath = Resolve-ExistingFile $candidate "Recording $id"
    if ([IO.Path]::GetExtension($recordingPath) -cne '.battleplan') {
        throw "$id is not a finalized .battleplan container"
    }
    $recordingSha256 = Get-Sha256 $recordingPath
    if ($recordingSha256 -cne [string]$recording.sha256) {
        throw "$id recording hash mismatch"
    }

    $logicalArgs = @()
    $logicalStreamProperty = $recording.PSObject.Properties['logical_stream_id']
    if ($null -ne $logicalStreamProperty -and $null -ne $logicalStreamProperty.Value) {
        $logicalArgs = @('--logical-stream', ([UInt64]$logicalStreamProperty.Value).ToString())
    }
    $validateArgs = @('--validate-recording', $recordingPath) + $logicalArgs
    $validated = Invoke-Replay $replayExe $validateArgs "$id recorder-to-WorldFrame validation"
    if ($validated.output -notmatch 'opened ".+": ([0-9]+) hour\(s\),') {
        throw "$id validation output did not report an indexed hour count`n$($validated.output)"
    }
    $hourCount = [int]$Matches[1]
    if ($hourCount -le 0) {
        throw "$id contains no replay hours"
    }
    if ($validated.output -notmatch [regex]::Escape($decoderId)) {
        throw "$id did not cross the shipped compact-stream prefold worker"
    }
    if ($validated.output -notmatch '\[compact source: decoder .+, [0-9]+ bytes, sha256 [0-9a-f]{64}\]') {
        throw "$id has no authenticated compact-source receipt"
    }
    $roleCoverage.compact_prefold_worldframe = $true

    $playbackArgs = @(
        '--validate-playback', $recordingPath,
        '--game', $game,
        '--userdata', $user,
        '--non-interactive'
    ) + $logicalArgs
    [void](Invoke-Replay $replayExe $playbackArgs "$id installation/content compatibility")

    $recordingOutput = Join-Path $output $id
    [IO.Directory]::CreateDirectory($recordingOutput) | Out-Null
    $hourReceipts = @()
    for ($ordinal = 0; $ordinal -lt $hourCount; $ordinal++) {
        $bmp = Join-Path $recordingOutput ('hour-{0:D8}.bmp' -f $ordinal)
        $dumpArgs = @(
            '--recording', $recordingPath,
            '--dump-frame', $bmp,
            '--dump-nth-hour', $ordinal.ToString(),
            '--game', $game,
            '--userdata', $user,
            '--non-interactive'
        ) + $logicalArgs
        [void](Invoke-Replay $replayExe $dumpArgs "$id headless render ordinal $ordinal")
        $bmpPath = Resolve-ExistingFile $bmp "$id BMP ordinal $ordinal"
        $pixelReceiptPath = Resolve-ExistingFile "$bmp.pixel-gate-receipt.json" "$id pixel receipt ordinal $ordinal"
        $pixelReceipt = [Text.Encoding]::UTF8.GetString(
            [IO.File]::ReadAllBytes($pixelReceiptPath)
        ) | ConvertFrom-Json
        # G2 was retired (owner decision 2026-07-27): the product emits v4
        # receipts only. FW10's per-receipt G2 disposition latch died with it;
        # the v4 arm is the surviving product contract.
        if ([string]$pixelReceipt.schema -cne 'battle-plan-production-pixel-gate-receipt-v4' -or
            [string]$pixelReceipt.recording_sha256 -cne $recordingSha256) {
            throw "$id ordinal $ordinal pixel receipt is not bound to the recording"
        }
        # The product serializes both flags as booleans and every count as a
        # number, never null: any other JSON type is not a product receipt
        # (PowerShell would otherwise coerce null to 0/false silently).
        if ($pixelReceipt.selected_present_executed -isnot [bool] -or
            $pixelReceipt.map_presented -isnot [bool]) {
            throw "$id ordinal $ordinal selected_present_executed/map_presented are not booleans"
        }
        foreach ($numericField in @(
                'selected_hour',
                'map_operations',
                'selected_present_operations',
                'selected_present_m02_draws',
                'pixel_byte_len')) {
            $numericProperty = $pixelReceipt.PSObject.Properties[$numericField]
            if ($null -eq $numericProperty -or
                $null -eq $numericProperty.Value -or
                $numericProperty.Value -is [bool] -or
                $numericProperty.Value -isnot [ValueType]) {
                throw "$id ordinal $ordinal $numericField is not a number"
            }
            # The product serializes selected_hour as i64 and every count as
            # usize: fractional values are never emitted, and the counts are
            # never negative. Refuse before any [int64] cast can coerce.
            $numericValue = $numericProperty.Value
            $isIntegral = $numericValue -is [int] -or
                $numericValue -is [int64] -or
                $numericValue -is [int16] -or
                ($numericValue -is [decimal] -and
                 [decimal]$numericValue -eq [decimal]::Truncate([decimal]$numericValue))
            if (-not $isIntegral) {
                throw "$id ordinal $ordinal $numericField is not an integer"
            }
            if ($numericField -cne 'selected_hour' -and [int64]$numericValue -lt 0) {
                throw "$id ordinal $ordinal $numericField is negative"
            }
        }
        $selectedPresentExecuted = [bool]$pixelReceipt.selected_present_executed
        $mapPresented = [bool]$pixelReceipt.map_presented
        # G2 retirement: the product never emits either retired field, so a
        # receipt carrying one is not a sequence the product can emit.
        foreach ($retiredField in @('g2_engine_capture', 'g2_exact_pixel_admission')) {
            $retiredProperty = $pixelReceipt.PSObject.Properties[$retiredField]
            if ($null -ne $retiredProperty -and $null -ne $retiredProperty.Value) {
                throw "$id ordinal $ordinal carries retired G2 field $retiredField"
            }
        }
        # The UI pass is unconditional post-retirement: a dump either executes
        # selected-Present or refuses to publish any receipt.
        if (-not $selectedPresentExecuted) {
            throw "$id ordinal $ordinal has no selected-Present execution; the product refuses such a dump"
        }
        # The product refuses to publish any receipt whose dump presented
        # neither authenticated map output nor selected-Present execution.
        if (-not $mapPresented -and -not $selectedPresentExecuted) {
            throw "$id ordinal $ordinal presents neither map output nor selected-Present execution; the product refuses such a dump"
        }
        # The M02 count is incremented at most once per executed
        # selected-Present operation, so it can never exceed the operation
        # frontier the same UI outcome reported.
        if ([int64]$pixelReceipt.selected_present_m02_draws -gt
            [int64]$pixelReceipt.selected_present_operations) {
            throw "$id ordinal $ordinal claims more M02 draws than selected-Present operations; the M02 count is a subset of the executed operation frontier"
        }
        if ([int64]$pixelReceipt.pixel_byte_len -le 0 -or
            ([int64]$pixelReceipt.map_operations + [int64]$pixelReceipt.selected_present_operations) -le 0) {
            throw "$id ordinal $ordinal executed no authenticated product pixels"
        }
        if ($mapPresented) {
            $roleCoverage.map_output = $true
        }
        if ($selectedPresentExecuted) {
            $roleCoverage.selected_present_ui = $true
            $companions = $pixelReceipt.selected_present_typed_companions
            if ($null -eq $companions) {
                throw "$id ordinal $ordinal executed selected-Present without its typed companion receipt"
            }
            foreach ($property in $companions.PSObject.Properties) {
                if ($property.Name -eq 'remaining' -or
                    $property.Name.EndsWith('_unavailable_reason')) {
                    continue
                }
                $role = "selected_present.$($property.Name)"
                if (-not $typedRoleCoverage.Contains($role)) {
                    $typedRoleCoverage[$role] = $false
                }
                $covered = if ($property.Name -eq 'xui15_surface_drawn') {
                    $property.Value -is [bool] -and [bool]$property.Value
                }
                else {
                    $null -ne $property.Value
                }
                if ($covered) {
                    $typedRoleCoverage[$role] = $true
                }
            }
            $remaining = $companions.remaining
            if ($null -eq $remaining) {
                throw "$id ordinal $ordinal selected-Present companion receipt has no remaining typed-consumption object"
            }
            foreach ($property in $remaining.PSObject.Properties) {
                if ($property.Name -eq 'feature_local_unavailable') {
                    continue
                }
                $role = "selected_present.remaining.$($property.Name)"
                if (-not $typedRoleCoverage.Contains($role)) {
                    $typedRoleCoverage[$role] = $false
                }
                if ($null -ne $property.Value) {
                    $typedRoleCoverage[$role] = $true
                }
            }
        }
        $hourReceipts += [ordered]@{
            ordinal = $ordinal
            canonical_hour = [int64]$pixelReceipt.selected_hour
            bmp_sha256 = Get-Sha256 $bmpPath
            pixel_receipt_sha256 = Get-Sha256 $pixelReceiptPath
            map_presented = [bool]$pixelReceipt.map_presented
            selected_present_executed = [bool]$pixelReceipt.selected_present_executed
            map_operations = [int64]$pixelReceipt.map_operations
            selected_present_operations = [int64]$pixelReceipt.selected_present_operations
        }
    }
    $recordingReceipts += [ordered]@{
        id = $id
        path = $recordingPath
        sha256 = $recordingSha256
        hour_count = $hourCount
        validation_output = $validated.output
        hours = $hourReceipts
    }
}

$missingRoles = @($roleCoverage.Keys | Where-Object { -not $roleCoverage[$_] })
if ($missingRoles.Count -ne 0) {
    throw "Real corpus did not exercise every single-machine product role: $($missingRoles -join ', ')"
}
$missingTypedRoles = @($typedRoleCoverage.Keys | Where-Object { -not $typedRoleCoverage[$_] })
if ($typedRoleCoverage.Count -eq 0 -or $missingTypedRoles.Count -ne 0) {
    throw "Real corpus did not exercise every typed selected-Present product role: $($missingTypedRoles -join ', ')"
}

$receipt = [ordered]@{
    schema = 'hoi4-portable-product-real-corpus-e2e-receipt-v1'
    status = 'PASS_REAL_RECORDER_COMPACT_TO_PRODUCT_PIXELS'
    package = [ordered]@{
        replay_exe_sha256 = Get-Sha256 $replayExe
        recorder_exe_sha256 = Get-Sha256 (Join-Path $package 'Battle Plan Recorder.exe')
        collector_dll_sha256 = Get-Sha256 (Join-Path $package 'collector.dll')
    }
    corpus_manifest_sha256 = Get-Sha256 $manifestPath
    role_coverage = $roleCoverage
    typed_role_coverage = $typedRoleCoverage
    recordings = $recordingReceipts
}
$receiptPath = Join-Path $output 'real-corpus-e2e-receipt.json'
$json = ($receipt | ConvertTo-Json -Depth 12) + "`n"
$temporaryReceipt = Join-Path $output ('.real-corpus-e2e-receipt.' + [Guid]::NewGuid().ToString('N') + '.tmp')
$receiptBytes = [Text.UTF8Encoding]::new($false).GetBytes($json)
$receiptStream = [IO.File]::Open(
    $temporaryReceipt,
    [IO.FileMode]::CreateNew,
    [IO.FileAccess]::Write,
    [IO.FileShare]::None
)
try {
    $receiptStream.Write($receiptBytes, 0, $receiptBytes.Length)
    $receiptStream.Flush($true)
}
finally {
    $receiptStream.Dispose()
}
[IO.File]::Move($temporaryReceipt, $receiptPath)
$receipt | ConvertTo-Json -Depth 12
