<#
.SYNOPSIS
Headless, uncompressed, no-debug-scaffolding recorder test loop.

Launches the game into its last save, unpauses it and sets the game speed by
authenticated memory write (no window activation, no synthetic input, no
client interaction), attaches the production recorder with the raw
uncompressed H4CR stream, and leaves every artifact on disk in a form that
can be read directly.

Deliberately NOT included: zstd stream compression, the debug capture
profile, sidecar logs, per-read RC counters. The point of this loop is a
plain stream plus a plain manifest.
#>
[CmdletBinding()]
param(
    [string]$Hoi4Exe = 'X:\SteamLibrary\steamapps\common\Hearts of Iron IV\hoi4.exe',

    [string]$PdxUserDir = 'C:\Users\Example\OneDrive\Documents\Paradox Interactive\Hearts of Iron IV',


    [string]$OutDir = '',

    [ValidateRange(10, 86400)]
    [double]$DurationSeconds = 120,

    [ValidateRange(1, 5)]
    [int]$Speed = 5,

    [ValidateRange(0, 100000)]
    [int]$MinGameHours = 0,

    [ValidateRange(1, 10000)]
    [int]$SampleMilliseconds = 1,

    [ValidateRange(1, 64)]
    [int]$ReaderConcurrency = 12,

    # Seconds to wait for the loaded campaign to expose stable world singletons.
    [ValidateRange(30, 1800)]
    [int]$WorldReadyTimeoutSeconds = 600,

    # Startup + finalize headroom allowed on top of -DurationSeconds before the
    # recorder is treated as wedged rather than slow.
    [ValidateRange(30, 3600)]
    [int]$RecorderOverheadSeconds = 300,

    # Seconds of continuously-advancing clock required before the recorder is
    # allowed to inject.
    #
    # MITIGATION, not a fix. Injecting while the engine is still finishing its
    # parallel post-load work has crashed the game (access violation calling
    # address 0x0 on a tbb.dll worker thread); injecting into a settled process
    # has been reliable. Hook installation freezes threads per hook via MinHook,
    # and a thread created during that window can miss the freeze. Set to 0 to
    # deliberately reproduce the race.
    [ValidateRange(0, 900)]
    [int]$SettleSeconds = 90,


    # Reuse an already-running campaign instead of launching one.
    #
    # Valid only for a game process that has never been recorded. The recorder
    # takes its injection lease by assigning the target to a job object, and job
    # membership cannot be revoked for the life of a process, so a second
    # capture against the same PID always fails with "already belongs to a job".
    # A repeatable loop must launch a fresh game per capture (the default).
    [switch]$UseRunningGame,

    # Close a running hoi4.exe before launching a fresh one.
    [switch]$StopExistingGame,

    # Leave the game running after the capture instead of closing it.
    [switch]$KeepGameAfterCapture,

    # Override the derived production roster with an explicit domain list.
    # Used to measure how the injection-crash rate scales with the number of
    # installed hooks; not a production configuration.
    [string[]]$ScheduledDomainOverride = @(),

    [switch]$SkipHostFold,

    # Drop --static-gfx-icon-manifest from the capture.
    #
    # Escape hatch for a KNOWN outstanding blocker, not a default: the recorder
    # requires meta.unused_missing_texture_allowlist_entries == 0, and the
    # checked-in allowlist currently leaves 72 rows unused, so finalize rejects
    # the artifact after the whole raw stream has already been written. Use this
    # to exercise runtime domain extraction while that is being fixed; a capture
    # produced this way is missing that static artifact and is not promotable.
    # See extractor/native/runs/gfx_allowlist_staleness_triage.json.
    [switch]$SkipGfxIconManifest,

    [switch]$DryRun
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

function Get-SettingsGuardBaseException {
    param([Parameter(Mandatory = $true)][System.Exception]$Exception)

    return $Exception.GetBaseException()
}

function Get-SettingsGuardExceptionKind {
    param([Parameter(Mandatory = $true)][System.Exception]$Exception)

    $baseException = Get-SettingsGuardBaseException -Exception $Exception
    if ($baseException -is [System.IO.FileNotFoundException]) {
        return 'not_found'
    }
    if ($baseException -is [System.IO.DirectoryNotFoundException]) {
        return 'not_found'
    }
    if ($baseException -is [System.UnauthorizedAccessException]) {
        return 'access_denied'
    }
    if ($baseException -is [System.IO.IOException]) {
        $win32Code = [int]$baseException.HResult -band 0xFFFF
        if ($win32Code -eq 32 -or $win32Code -eq 33) {
            return 'sharing_violation'
        }
    }
    return 'other'
}

if ($null -eq ('Hoi4MpStats.SettingsGuardNativeMethods' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Text;
using Microsoft.Win32.SafeHandles;

namespace Hoi4MpStats {
    public static class SettingsGuardNativeMethods {
        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        public static extern uint GetFinalPathNameByHandle(
            SafeFileHandle file,
            StringBuilder path,
            uint pathLength,
            uint flags);
    }
}
'@
}

function Get-SettingsGuardCanonicalPath {
    param([Parameter(Mandatory = $true)][string]$Path)

    $stream = $null
    try {
        $share = [System.IO.FileShare](
            [int][System.IO.FileShare]::ReadWrite -bor
            [int][System.IO.FileShare]::Delete
        )
        $stream = [System.IO.File]::Open(
            $Path,
            [System.IO.FileMode]::Open,
            [System.IO.FileAccess]::Read,
            $share
        )

        # A volume-GUID final path gives junctions, symbolic links and SUBST
        # spellings one stable identity. Fall back only for providers that do
        # not expose a volume GUID (for example some network redirectors).
        $lastWin32Error = 0
        foreach ($flags in @(1, 2, 0)) {
            $capacity = 1024
            while ($capacity -le 32768) {
                $buffer = [System.Text.StringBuilder]::new($capacity)
                $length = [Hoi4MpStats.SettingsGuardNativeMethods]::GetFinalPathNameByHandle(
                    $stream.SafeFileHandle,
                    $buffer,
                    [uint32]$buffer.Capacity,
                    [uint32]$flags
                )
                if ($length -eq 0) {
                    $lastWin32Error = [Runtime.InteropServices.Marshal]::GetLastWin32Error()
                    break
                }
                if ($length -lt [uint32]$buffer.Capacity) {
                    return $buffer.ToString()
                }
                $capacity = [int]$length + 1
            }
        }
        throw [ComponentModel.Win32Exception]::new(
            $lastWin32Error,
            "GetFinalPathNameByHandle failed for '$Path'"
        )
    } finally {
        if ($null -ne $stream) {
            $stream.Dispose()
        }
    }
}

function Get-SettingsGuardKey {
    param([Parameter(Mandatory = $true)][string]$Identity)

    $hash = [System.Security.Cryptography.SHA256]::Create()
    try {
        $keyBytes = [System.Text.Encoding]::UTF8.GetBytes(
            $Identity.ToUpperInvariant()
        )
        return ([System.BitConverter]::ToString(
            $hash.ComputeHash($keyBytes)
        )).Replace('-', '').ToLowerInvariant()
    } finally {
        $hash.Dispose()
    }
}

function Write-SettingsGuardNonThrowingWarning {
    param([Parameter(Mandatory = $true)][string]$Message)

    try {
        Write-Warning $Message -WarningAction Continue
    } catch {
        # A restore/cleanup diagnostic must never replace the capture error or
        # exit code already unwinding through the outer finally.
        try {
            [Console]::Error.WriteLine("WARNING: $Message")
        } catch {
        }
    }
}

$expectedExeSha256 = '870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad'
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = (Resolve-Path -LiteralPath (Join-Path $scriptDir '..\..\..')).Path
$releaseRoot = Join-Path $repoRoot '.cargo-target\release'
$nativeRoot = Join-Path $repoRoot 'extractor\native'
$collectorDll = Join-Path $releaseRoot 'collector.dll'
$collectorRecord = Join-Path $releaseRoot 'collector_record.exe'
$headlessControl = Join-Path $scriptDir 'Control-Hoi4Game.ps1'
$streamReport = Join-Path $scriptDir 'report_raw_capture.py'
$gameDefsOut = Join-Path $repoRoot 'extractor\game_defs\out'
# Get-Command can return several python.exe entries (real install plus the
# WindowsApps stub); take the first rather than joining them into one string.
$pythonExe = @(Get-Command python.exe -CommandType Application -ErrorAction Stop)[0].Source

$staticCatalogs = [ordered]@{
    '--static-templates-equipment-catalog' = Join-Path $gameDefsOut 'templates_equipment_catalog\templates_equipment_catalog_live_domain59\domain59_static_catalog.json'
    '--static-localisation-join'           = Join-Path $gameDefsOut 'localisation_gate\static_localisation_join_r3\localisation_catalog.json'
    '--static-decisions-definitions'       = Join-Path $gameDefsOut 'decisions_static\decisions_static_definitions\decision_definitions_catalog.json'
    '--static-idea-tech-loc-xref'          = Join-Path $gameDefsOut 'idea_tech_loc_xref\idea_tech_loc_xref_static\idea_tech_loc_xref_catalog.json'
    '--static-ship-definitions'            = Join-Path $gameDefsOut 'ship_definitions_static\ship_definitions_static\ship_definitions_catalog.json'
    '--static-gfx-icon-manifest'           = Join-Path $gameDefsOut 'gfx_icon_manifest\gfx_icon_asset_manifest_static\gfx_icon_manifest_catalog.json'
}

foreach ($path in @($Hoi4Exe, $PdxUserDir, $collectorDll, $collectorRecord, $headlessControl)) {
    if (-not (Test-Path -LiteralPath $path)) { throw "Required path is missing: $path" }
}
if ($SkipGfxIconManifest) {
    $staticCatalogs.Remove('--static-gfx-icon-manifest')
    Write-Warning 'Capturing WITHOUT the GFX icon manifest static artifact (-SkipGfxIconManifest). This capture is not promotable.'
}
foreach ($entry in $staticCatalogs.GetEnumerator()) {
    if (-not (Test-Path -LiteralPath $entry.Value -PathType Leaf)) {
        throw "Static catalog for $($entry.Key) is missing: $($entry.Value)"
    }
}

# `--profile production` is early-launch-only ("production/profile capture
# requires authenticated --early-launch-exe so render creation cannot be
# missed") and additionally demands a process-bound static bundle. A late
# attach therefore selects the production roster explicitly, exactly as the
# shipped product worker does. Passing no schedule at all would silently fall
# back to the eight-domain compatibility set, so the roster is mandatory.
$rosterTool = Join-Path $releaseRoot 'domain_roster.exe'
if (-not (Test-Path -LiteralPath $rosterTool -PathType Leaf)) {
    throw "Production roster tool is missing: $rosterTool (cargo build --release -p recorder_host)"
}
if ($ScheduledDomainOverride.Count -gt 0) {
    # `powershell -File` binds a comma-joined argument as ONE literal string —
    # it never gets PowerShell's array splitting. The 2026-07-26 crash trials
    # passed "session_header_live,fuel_live" that way; the recorder rejected it
    # as a single unknown domain and exited before injecting, so the whole
    # minimal arm silently measured nothing. Split here so both invocation
    # styles mean the same thing.
    $scheduledDomains = @($ScheduledDomainOverride | ForEach-Object { $_ -split ',' } | Where-Object { $_ })
    Write-Warning "Using an explicit $($scheduledDomains.Count)-domain roster override; this is a diagnostic configuration, not production coverage."
} else {
    $scheduledDomains = @(& $rosterTool --native-root $nativeRoot)
    if ($LASTEXITCODE -ne 0) { throw 'Production domain roster derivation failed.' }
    if ($scheduledDomains.Count -eq 0) { throw 'Derived production domain roster is empty.' }
}

$Hoi4Exe = (Resolve-Path -LiteralPath $Hoi4Exe).Path
$PdxUserDir = (Resolve-Path -LiteralPath $PdxUserDir).Path
$observedExeSha256 = (Get-FileHash -LiteralPath $Hoi4Exe -Algorithm SHA256).Hash.ToLowerInvariant()
if ($observedExeSha256 -cne $expectedExeSha256) {
    throw "hoi4.exe is not the pinned exact-a729 build: $observedExeSha256"
}

$stamp = Get-Date -Format 'yyyyMMdd_HHmmss'
if ([string]::IsNullOrWhiteSpace($OutDir)) {
    $OutDir = Join-Path $repoRoot "extractor\native\runs\headless_raw_$stamp"
}
$OutDir = [System.IO.Path]::GetFullPath($OutDir)
if (Test-Path -LiteralPath $OutDir) {
    if (@(Get-ChildItem -LiteralPath $OutDir -Force).Count -ne 0) {
        throw "OutDir must be absent or empty: $OutDir"
    }
} else {
    New-Item -ItemType Directory -Path $OutDir -Force | Out-Null
}

$settingsPath = Join-Path $PdxUserDir 'settings.txt'
$settingsPath = [System.IO.Path]::GetFullPath($settingsPath)
if (-not (Test-Path -LiteralPath $settingsPath -PathType Leaf)) {
    throw "Required settings.txt is missing: '$settingsPath' (derived from PdxUserDir '$PdxUserDir'). The capture harness requires this launch-settings evidence to resolve the running process modset."
}
$settingsGuardCanonicalPath = Get-SettingsGuardCanonicalPath -Path $settingsPath
$settingsGuardKey = Get-SettingsGuardKey -Identity $settingsGuardCanonicalPath
$settingsGuardLegacyKey = Get-SettingsGuardKey -Identity $settingsPath
$settingsGuardLocalAppData = [Environment]::GetFolderPath(
    [Environment+SpecialFolder]::LocalApplicationData
)
if ([string]::IsNullOrWhiteSpace($settingsGuardLocalAppData)) {
    throw "Settings guard cannot create checkout-independent kill-safe recovery evidence because the machine-global LocalApplicationData folder resolved empty for '$settingsPath'."
}
$settingsGuardRoot = Join-Path (
    Join-Path $settingsGuardLocalAppData 'battle-plan'
) 'settings_guard'
$settingsGuardCanonicalSentinel = Join-Path $settingsGuardRoot "$settingsGuardKey.json"
$settingsGuardLegacySentinel = Join-Path $settingsGuardRoot "$settingsGuardLegacyKey.json"
$settingsGuardSentinel = $settingsGuardCanonicalSentinel
$settingsGuardPriorReadOnly = $false
$settingsGuardRestoreRequired = $false
$settingsGuardOwnerStream = $null

try {
    if ($DryRun) {
        if ($UseRunningGame) {
            Write-Host "phase=settings_guard action=skip reason=use_running_game dry_run=True path=$settingsPath sentinel=$settingsGuardSentinel"
            Write-Warning 'The settings guard cannot protect -UseRunningGame because the boot-time settings.txt rewrite has already happened; modset resolution may be unresolved for this attach.'
        } else {
            Write-Host "phase=settings_guard action=skip reason=dry_run would_apply=True path=$settingsPath sentinel=$settingsGuardSentinel"
        }
    } else {
        $canonicalSentinelExists = Test-Path `
            -LiteralPath $settingsGuardCanonicalSentinel `
            -PathType Leaf
        $legacySentinelIsDistinct = -not [System.StringComparer]::OrdinalIgnoreCase.Equals(
            $settingsGuardLegacySentinel,
            $settingsGuardCanonicalSentinel
        )
        $legacySentinelExists = (
            $legacySentinelIsDistinct -and
            (Test-Path -LiteralPath $settingsGuardLegacySentinel -PathType Leaf)
        )
        if ($canonicalSentinelExists -and $legacySentinelExists) {
            throw "Settings guard found both canonical and legacy sentinels for '$settingsPath': '$settingsGuardCanonicalSentinel' and '$settingsGuardLegacySentinel'. Refusing to choose between concurrent recovery records; verify all holders and restore the attribute manually."
        }
        if ($legacySentinelExists) {
            $settingsGuardSentinel = $settingsGuardLegacySentinel
            Write-Warning "Settings guard found a legacy DOS-path sentinel at '$settingsGuardSentinel'; it will be recovered before this run publishes the canonical guard at '$settingsGuardCanonicalSentinel'."
        }
        if (Test-Path -LiteralPath $settingsGuardSentinel -PathType Leaf) {
            $settingsGuardReadStream = $null
            $settingsGuardReader = $null
            $settingsGuardSentinelVanishedDuringRead = $false
            try {
                $settingsGuardReadStream = [System.IO.File]::Open(
                    $settingsGuardSentinel,
                    [System.IO.FileMode]::Open,
                    [System.IO.FileAccess]::Read,
                    [System.IO.FileShare]::ReadWrite
                )
                $settingsGuardReader = [System.IO.StreamReader]::new(
                    $settingsGuardReadStream,
                    [System.Text.UTF8Encoding]::new($false),
                    $true,
                    1024,
                    $true
                )
                $settingsGuardRecordJson = $settingsGuardReader.ReadToEnd()
                $settingsGuardRecord = $settingsGuardRecordJson | ConvertFrom-Json
            } catch {
                $sentinelReadException = Get-SettingsGuardBaseException -Exception $_.Exception
                $sentinelReadExceptionKind = Get-SettingsGuardExceptionKind -Exception $_.Exception
                if ($sentinelReadExceptionKind -eq 'not_found') {
                    $settingsGuardSentinelVanishedDuringRead = $true
                } elseif ($sentinelReadExceptionKind -eq 'sharing_violation') {
                    throw "Settings guard sentinel '$settingsGuardSentinel' has an incompatible live holder before its recovery record can be read; this may be the capture writer or an exclusive reader. Refusing to touch '$settingsPath' or the sentinel. Wait for all holders to close, then retry. $($sentinelReadException.Message)"
                } else {
                    throw "Settings guard sentinel is unreadable: $settingsGuardSentinel. Inspect it and manually restore the read-only state of '$settingsPath' before removing it. $($_.Exception.Message)"
                }
            } finally {
                if ($null -ne $settingsGuardReader) {
                    $settingsGuardReader.Dispose()
                }
                if ($null -ne $settingsGuardReadStream) {
                    $settingsGuardReadStream.Dispose()
                }
            }
            if ($settingsGuardSentinelVanishedDuringRead) {
                Write-Warning "Settings guard sentinel '$settingsGuardSentinel' vanished before its recovery record could be read; another run already recovered it. Continuing without touching '$settingsPath'."
                Write-Host "phase=settings_guard action=skip reason=sentinel_vanished path=$settingsPath sentinel=$settingsGuardSentinel"
            } else {
                if ($null -eq $settingsGuardRecord) {
                    throw "Settings guard sentinel is invalid: $settingsGuardSentinel. It must be a schema-version 2 JSON object. Inspect it and manually restore '$settingsPath' before removing it."
                }
                $schemaProperty = $settingsGuardRecord.PSObject.Properties['schema_version']
                $recordedPathProperty = $settingsGuardRecord.PSObject.Properties['settings_path']
                $recordedPriorProperty = $settingsGuardRecord.PSObject.Properties['prior_read_only']
                $recordedOwnerPidProperty = $settingsGuardRecord.PSObject.Properties['owner_pid']
                $recordedOwnerStartProperty = $settingsGuardRecord.PSObject.Properties['owner_start_utc']
                if ($null -eq $schemaProperty -or $schemaProperty.Value -isnot [int] -or
                    [int]$schemaProperty.Value -ne 2 -or
                    $null -eq $recordedPathProperty -or
                    $null -eq $recordedPriorProperty -or
                    $recordedPriorProperty.Value -isnot [bool] -or
                    $null -eq $recordedOwnerPidProperty -or
                    $recordedOwnerPidProperty.Value -isnot [int] -or
                    [int]$recordedOwnerPidProperty.Value -le 0 -or
                    $null -eq $recordedOwnerStartProperty -or
                    $recordedOwnerStartProperty.Value -isnot [string] -or
                    [string]::IsNullOrWhiteSpace([string]$recordedOwnerStartProperty.Value)) {
                    throw "Settings guard sentinel is invalid or from an unsupported schema: $settingsGuardSentinel. It must contain schema_version=2, settings_path, a Boolean prior_read_only, a positive integer owner_pid, and owner_start_utc. Inspect it and manually restore '$settingsPath' before removing it."
                }
                $recordedSettingsPathRaw = [string]$recordedPathProperty.Value
                if ([string]::IsNullOrWhiteSpace($recordedSettingsPathRaw)) {
                    throw "Settings guard sentinel contains an empty settings_path: $settingsGuardSentinel. Inspect it and manually restore '$settingsPath' before removing it."
                }
                if ([System.StringComparer]::OrdinalIgnoreCase.Equals(
                    $recordedSettingsPathRaw,
                    $settingsGuardCanonicalPath
                ) -or [System.StringComparer]::OrdinalIgnoreCase.Equals(
                    $recordedSettingsPathRaw,
                    $settingsPath
                )) {
                    $recordedSettingsPath = $recordedSettingsPathRaw
                } else {
                    try {
                        $recordedSettingsPath = [System.IO.Path]::GetFullPath(
                            $recordedSettingsPathRaw
                        )
                    } catch {
                        throw "Settings guard sentinel contains an invalid settings_path: $settingsGuardSentinel. Inspect it and manually restore '$settingsPath' before removing it. $($_.Exception.Message)"
                    }
                }
                if ([System.StringComparer]::OrdinalIgnoreCase.Equals(
                    $recordedSettingsPath,
                    $settingsGuardCanonicalPath
                ) -or [System.StringComparer]::OrdinalIgnoreCase.Equals(
                    $recordedSettingsPath,
                    $settingsPath
                )) {
                    # The second arm accepts schema-2 sentinels written before
                    # canonical final paths were introduced.
                    $recordedSettingsCanonicalPath = $settingsGuardCanonicalPath
                } elseif (Test-Path -LiteralPath $recordedSettingsPath -PathType Leaf) {
                    try {
                        $recordedSettingsCanonicalPath = Get-SettingsGuardCanonicalPath -Path $recordedSettingsPath
                    } catch {
                        throw "Settings guard sentinel settings_path could not be canonicalized: '$recordedSettingsPath' ($settingsGuardSentinel). Refusing to touch '$settingsPath'. $($_.Exception.Message)"
                    }
                } else {
                    $recordedSettingsCanonicalPath = $recordedSettingsPath
                }
                if (-not [System.StringComparer]::OrdinalIgnoreCase.Equals(
                    $recordedSettingsCanonicalPath,
                    $settingsGuardCanonicalPath
                )) {
                    throw "Settings guard sentinel path mismatch: '$recordedSettingsPath' does not identify '$settingsPath' (canonical '$settingsGuardCanonicalPath'; $settingsGuardSentinel). Refusing to overwrite recovery evidence."
                }
                try {
                    $recordedOwnerStart = [DateTimeOffset]::ParseExact(
                        [string]$recordedOwnerStartProperty.Value,
                        'o',
                        [Globalization.CultureInfo]::InvariantCulture,
                        [Globalization.DateTimeStyles]::RoundtripKind
                    ).ToUniversalTime()
                } catch {
                    throw "Settings guard sentinel contains an invalid owner_start_utc: $settingsGuardSentinel. Inspect it and manually restore '$settingsPath' before removing it. $($_.Exception.Message)"
                }

                $recordedPriorReadOnly = [bool]$recordedPriorProperty.Value
                $recordedOwnerPid = [int]$recordedOwnerPidProperty.Value
                $settingsGuardRecoveryClaim = $null
                $settingsGuardRecoveryClaimStatus = ''
                $settingsGuardSentinelVanished = $false
                $settingsGuardClaimAttempts = 3
                for ($claimAttempt = 1; $claimAttempt -le $settingsGuardClaimAttempts; $claimAttempt++) {
                    try {
                        $settingsGuardRecoveryClaim = [System.IO.File]::Open(
                            $settingsGuardSentinel,
                            [System.IO.FileMode]::Open,
                            [System.IO.FileAccess]::ReadWrite,
                            [System.IO.FileShare]::None
                        )
                        $settingsGuardRecoveryClaimStatus = 'exclusive'
                        break
                    } catch {
                        $claimException = Get-SettingsGuardBaseException -Exception $_.Exception
                        $claimExceptionKind = Get-SettingsGuardExceptionKind -Exception $_.Exception
                        if ($claimExceptionKind -eq 'not_found') {
                            $settingsGuardSentinelVanished = $true
                            break
                        }
                        if ($claimExceptionKind -eq 'access_denied') {
                            throw "Settings guard cannot take a recovery claim on sentinel '$settingsGuardSentinel' because access was denied. Refusing to touch '$settingsPath' or the sentinel; this is not evidence that a live capture run holds the guard. $($claimException.Message)"
                        }
                        if ($claimExceptionKind -ne 'sharing_violation') {
                            $claimHResult = '0x{0:X8}' -f ([int]$claimException.HResult)
                            throw "Settings guard recovery claim failed unexpectedly for sentinel '$settingsGuardSentinel' ($($claimException.GetType().FullName), HResult=$claimHResult). Refusing to touch '$settingsPath' or the sentinel; this is not evidence that a live capture run holds the guard. $($claimException.Message)"
                        }

                        # An exclusive claim also conflicts with an ordinary reader. A
                        # read-only probe whose share excludes writers distinguishes that
                        # benign case from the guard owner's live write handle.
                        $settingsGuardReaderOnlyProbe = $null
                        try {
                            $settingsGuardReaderOnlyProbe = [System.IO.File]::Open(
                                $settingsGuardSentinel,
                                [System.IO.FileMode]::Open,
                                [System.IO.FileAccess]::Read,
                                [System.IO.FileShare]::Read
                            )
                            $settingsGuardRecoveryClaimStatus = 'readers_only'
                            break
                        } catch {
                            $readerProbeException = Get-SettingsGuardBaseException -Exception $_.Exception
                            $readerProbeExceptionKind = Get-SettingsGuardExceptionKind -Exception $_.Exception
                            if ($readerProbeExceptionKind -eq 'not_found') {
                                $settingsGuardSentinelVanished = $true
                                break
                            }
                            if ($readerProbeExceptionKind -eq 'access_denied') {
                                throw "Settings guard cannot inspect the holders of sentinel '$settingsGuardSentinel' because read access was denied. Refusing to touch '$settingsPath' or the sentinel; this is not evidence that a live capture run holds the guard. $($readerProbeException.Message)"
                            }
                            if ($readerProbeExceptionKind -ne 'sharing_violation') {
                                $readerProbeHResult = '0x{0:X8}' -f ([int]$readerProbeException.HResult)
                                throw "Settings guard holder inspection failed unexpectedly for sentinel '$settingsGuardSentinel' ($($readerProbeException.GetType().FullName), HResult=$readerProbeHResult). Refusing to touch '$settingsPath' or the sentinel; this is not evidence that a live capture run holds the guard. $($readerProbeException.Message)"
                            }
                        } finally {
                            if ($null -ne $settingsGuardReaderOnlyProbe) {
                                $settingsGuardReaderOnlyProbe.Dispose()
                            }
                        }

                        if ($claimAttempt -lt $settingsGuardClaimAttempts) {
                            Start-Sleep -Milliseconds 200
                        }
                    }
                }

                if ($settingsGuardSentinelVanished) {
                    Write-Warning "Settings guard sentinel '$settingsGuardSentinel' vanished before its recovery claim could be taken; another run already recovered it. Continuing without touching '$settingsPath'."
                    Write-Host "phase=settings_guard action=skip reason=sentinel_vanished path=$settingsPath sentinel=$settingsGuardSentinel"
                } elseif ([string]::IsNullOrWhiteSpace($settingsGuardRecoveryClaimStatus)) {
                    throw "Settings guard sentinel '$settingsGuardSentinel' still has an incompatible live holder after $settingsGuardClaimAttempts recovery-claim attempts; this may be the capture writer or an exclusive reader (recorded writer PID $recordedOwnerPid at $($recordedOwnerStart.ToString('o'))). Refusing to touch '$settingsPath' or the sentinel. Wait for all holders to close, or remove the sentinel deliberately only after verifying no run holds the guard."
                } else {
                    try {
                        Write-Warning "Interrupted settings guard detected at '$settingsGuardSentinel' (no live holder; written by PID $recordedOwnerPid at $($recordedOwnerStart.ToString('o'))); restoring IsReadOnly=$recordedPriorReadOnly on '$settingsPath' before this run."
                        $recoveryItem = Get-Item -LiteralPath $settingsPath -Force
                        $recoveryItem.IsReadOnly = $recordedPriorReadOnly
                        $recoveryItem.Refresh()
                        if ([bool]$recoveryItem.IsReadOnly -ne $recordedPriorReadOnly) {
                            throw "Interrupted settings guard recovery did not restore IsReadOnly=$recordedPriorReadOnly on '$settingsPath'. Sentinel retained at '$settingsGuardSentinel'."
                        }
                    } finally {
                        if ($null -ne $settingsGuardRecoveryClaim) {
                            try {
                                $settingsGuardRecoveryClaim.Dispose()
                            } finally {
                                $settingsGuardRecoveryClaim = $null
                            }
                        }
                    }
                    $settingsGuardRecoverySentinelRemoved = $false
                    try {
                        Remove-Item -LiteralPath $settingsGuardSentinel -Force
                        $settingsGuardRecoverySentinelRemoved = $true
                    } catch {
                        if (-not (Test-Path -LiteralPath $settingsGuardSentinel -PathType Leaf)) {
                            $settingsGuardRecoverySentinelRemoved = $true
                        } else {
                            Write-Warning "Settings guard restored IsReadOnly=$recordedPriorReadOnly on '$settingsPath', but could not remove recovered sentinel '$settingsGuardSentinel'. The recovery is complete and the sentinel was retained for the next run. Error: $($_.Exception.Message)"
                        }
                    }
                    Write-Host "phase=settings_guard action=recover owner_status=no_live_holder claim_status=$settingsGuardRecoveryClaimStatus writer_pid=$recordedOwnerPid writer_start_utc=$($recordedOwnerStart.ToString('o')) restored_read_only=$recordedPriorReadOnly sentinel_removed=$settingsGuardRecoverySentinelRemoved path=$settingsPath sentinel=$settingsGuardSentinel"
                    if (-not $settingsGuardRecoverySentinelRemoved) {
                        throw "Settings guard recovery restored IsReadOnly=$recordedPriorReadOnly on '$settingsPath', but retained sentinel '$settingsGuardSentinel' prevents a new guard from being applied. Retry after the process reading the sentinel releases it."
                    }
                }
            }
        }

        $settingsGuardSentinel = $settingsGuardCanonicalSentinel
        if ($UseRunningGame) {
            Write-Host "phase=settings_guard action=skip reason=use_running_game path=$settingsPath sentinel=$settingsGuardSentinel"
            Write-Warning 'The settings guard cannot protect -UseRunningGame because the boot-time settings.txt rewrite has already happened; modset resolution may be unresolved for this attach.'
        } else {
            $settingsItem = Get-Item -LiteralPath $settingsPath -Force
            $settingsGuardPriorReadOnly = [bool]$settingsItem.IsReadOnly
            $settingsGuardOwnerProcess = Get-Process -Id $PID -ErrorAction Stop
            try {
                $settingsGuardOwnerStartUtc = ([DateTimeOffset]$settingsGuardOwnerProcess.StartTime).ToUniversalTime().ToString('o')
            } catch {
                throw "Settings guard cannot read its own process start time for PID $PID, so it cannot create kill-safe ownership evidence for '$settingsPath'. $($_.Exception.Message)"
            }
            New-Item -ItemType Directory -Path $settingsGuardRoot -Force | Out-Null
            $settingsGuardRecord = [ordered]@{
                schema_version  = 2
                settings_path   = $settingsGuardCanonicalPath
                prior_read_only = $settingsGuardPriorReadOnly
                created_utc     = [DateTime]::UtcNow.ToString('o')
                owner_pid       = $PID
                owner_start_utc = $settingsGuardOwnerStartUtc
            }
            $settingsGuardJson = $settingsGuardRecord | ConvertTo-Json -Compress
            $settingsGuardEncoding = [System.Text.UTF8Encoding]::new($false)
            $settingsGuardBytes = $settingsGuardEncoding.GetBytes($settingsGuardJson)
            $settingsGuardOwnerStream = [System.IO.File]::Open(
                $settingsGuardSentinel,
                [System.IO.FileMode]::CreateNew,
                [System.IO.FileAccess]::Write,
                [System.IO.FileShare]::ReadWrite
            )
            try {
                $settingsGuardOwnerStream.Write(
                    $settingsGuardBytes,
                    0,
                    $settingsGuardBytes.Length
                )
                $settingsGuardOwnerStream.Flush($true)
            } catch {
                $settingsGuardWriteFailure = $_
                try {
                    $settingsGuardOwnerStream.Dispose()
                } catch {
                    # Preserve the original write/flush failure below.
                } finally {
                    $settingsGuardOwnerStream = $null
                }
                # A hard kill between CreateNew and Flush can still strand an
                # incomplete sentinel because no finally can run. In-process I/O
                # failures are cleaned here before settings.txt is made read-only.
                Remove-Item -LiteralPath $settingsGuardSentinel -Force -ErrorAction SilentlyContinue
                throw $settingsGuardWriteFailure
            }
            $settingsGuardRestoreRequired = $true

            $settingsItem.IsReadOnly = $true
            $settingsItem.Refresh()
            if (-not [bool]$settingsItem.IsReadOnly) {
                throw "Settings guard could not set IsReadOnly=True on '$settingsPath'. Sentinel retained at '$settingsGuardSentinel'."
            }
            Write-Host "phase=settings_guard action=apply prior_read_only=$settingsGuardPriorReadOnly path=$settingsPath sentinel=$settingsGuardSentinel"
        }
    }

function ConvertTo-NativeArgumentLine {
    param([Parameter(Mandatory = $true)][object[]]$Arguments)
    return (($Arguments | ForEach-Object {
        $value = [string]$_
        if ($value -match '[\s"]') { '"' + ($value -replace '(\\*)"', '$1$1\"' -replace '(\\+)$', '$1$1') + '"' }
        else { $value }
    }) -join ' ')
}

function Get-Hoi4Processes {
    # PowerShell unwraps single-element arrays on return, which breaks `.Count`
    # under StrictMode; callers wrap this in @() to restore array semantics.
    return @(Get-Process -Name the game -ErrorAction SilentlyContinue)
}

function Wait-Hoi4WorldReady {
    param(
        [Parameter(Mandatory = $true)][int]$TargetProcessId,
        [Parameter(Mandatory = $true)][int]$TimeoutSeconds
    )
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    $lastError = 'not attempted'
    while ([DateTime]::UtcNow -lt $deadline) {
        $probe = Get-Process -Id $TargetProcessId -ErrorAction SilentlyContinue
        if ($null -eq $probe) { throw "the game PID $TargetProcessId exited before the world became ready." }
        try {
            $state = & $headlessControl -TargetPid $TargetProcessId
            if ($null -ne $state) { return $state }
        } catch {
            # CGameState / CInGameIdler do not exist until the save is loaded.
            $lastError = $_.Exception.Message
        }
        Start-Sleep -Milliseconds 750
    }
    throw "the game PID $TargetProcessId did not expose stable world singletons within $TimeoutSeconds s. Last probe error: $lastError"
}

# --- Phase 1: obtain a live campaign process -------------------------------
$launchedProcess = $null
$gamePid = 0

if ($UseRunningGame) {
    $running = @(Get-Hoi4Processes)
    if ($running.Count -ne 1) { throw "-UseRunningGame needs exactly one hoi4.exe; found $($running.Count)." }
    $gamePid = [int]$running[0].Id
    Write-Host "phase=reuse pid=$gamePid"
} else {
    $running = @(Get-Hoi4Processes)
    if ($running.Count -ne 0) {
        if (-not $StopExistingGame) {
            throw "hoi4.exe PID(s) $(@($running.Id) -join ',') are running. Pass -StopExistingGame to close them, or -UseRunningGame to attach to the existing campaign."
        }
        foreach ($proc in $running) {
            Write-Host "phase=stop_existing pid=$($proc.Id)"
            if ($DryRun) { continue }
            try { $proc.CloseMainWindow() | Out-Null } catch { }
            if (-not $proc.WaitForExit(15000)) { $proc.Kill(); [void]$proc.WaitForExit(15000) }
        }
    }

    $continueGame = Join-Path $PdxUserDir 'continue_game.json'
    if (-not (Test-Path -LiteralPath $continueGame -PathType Leaf)) {
        throw "Launcher continue selection is missing: $continueGame"
    }
    $continueTitle = (Get-Content -LiteralPath $continueGame -Raw | ConvertFrom-Json).title
    Write-Host "phase=launch continue_title=$continueTitle"

    # Launch through WMI rather than Start-Process. The recorder refuses a
    # target that already belongs to a job object (it takes its own injection
    # lease as a job), and a process started directly inherits the caller's job
    # when this script runs under an automation host that uses one. Win32_Process
    # creates the game from the WMI service, outside any such job.
    $launchCommandLine = '"' + $Hoi4Exe + '" --continuelastsave --gdpr-compliant "-userdir=' + $PdxUserDir + '"'
    if ($DryRun) {
        Write-Host "DRY RUN launch: $launchCommandLine"
    } else {
        $created = Invoke-CimMethod -ClassName Win32_Process -MethodName Create -Arguments @{
            CommandLine      = $launchCommandLine
            CurrentDirectory = (Split-Path -Parent $Hoi4Exe)
        }
        if ([int]$created.ReturnValue -ne 0) {
            throw "Win32_Process.Create failed for hoi4.exe with return value $($created.ReturnValue)."
        }
        $gamePid = [int]$created.ProcessId
        $launchedProcess = Get-Process -Id $gamePid -ErrorAction Stop
        Write-Host "phase=launched pid=$gamePid"
    }
}

if ($DryRun) {
    Write-Host "DRY RUN: stopping before world-ready wait. out_dir=$OutDir"
    return
}

# --- Phase 2: headless world-ready + unpause + speed ------------------------
$readyState = Wait-Hoi4WorldReady -TargetProcessId $gamePid -TimeoutSeconds $WorldReadyTimeoutSeconds
Write-Host "phase=world_ready pid=$gamePid initial_paused=$($readyState.Paused) initial_speed=$($readyState.Speed)"

# The world singletons become readable before the save has finished loading,
# and the engine sets its own paused flag when the load completes. A single
# unpause write can therefore be silently reverted, which previously produced
# a capture whose clock never advanced. Re-apply until the hour actually moves.
$control = $null
$controlDeadline = [DateTime]::UtcNow.AddSeconds(120)
while ([DateTime]::UtcNow -lt $controlDeadline) {
    $attempt = & $headlessControl -TargetPid $gamePid -Speed $Speed -Play
    if ([int]$attempt.Speed -eq $Speed -and -not [bool]$attempt.Paused -and [bool]$attempt.Advancing) {
        $control = $attempt
        break
    }
    Write-Host "phase=control_retry speed=$($attempt.Speed) paused=$($attempt.Paused) advancing=$($attempt.Advancing)"
    Start-Sleep -Milliseconds 1000
}
if ($null -eq $control) {
    throw "Headless control could not hold speed $Speed with an advancing clock within 120 s."
}
Write-Host "phase=controlled speed=$($control.Speed) paused=$($control.Paused) hour_delta=$($control.HourDelta) advancing=$($control.Advancing)"
$controlPath = Join-Path $OutDir 'headless_control.json'
($control | ConvertTo-Json -Depth 8) | Set-Content -LiteralPath $controlPath -Encoding UTF8

# Require the clock to keep advancing for the whole settle window. A single
# advancing sample only proves the save finished loading, not that the engine's
# parallel post-load work has drained -- and it is that work that the injection
# race kills the game in.
if ($SettleSeconds -gt 0) {
    Write-Host "phase=settle seconds=$SettleSeconds"
    $settleDeadline = [DateTime]::UtcNow.AddSeconds($SettleSeconds)
    while ([DateTime]::UtcNow -lt $settleDeadline) {
        Start-Sleep -Seconds 10
        $sample = & $headlessControl -TargetPid $gamePid -Speed $Speed -Play
        if (-not $sample.Advancing -or [int]$sample.Speed -ne $Speed) {
            Write-Host "phase=settle_restart speed=$($sample.Speed) advancing=$($sample.Advancing)"
            $settleDeadline = [DateTime]::UtcNow.AddSeconds($SettleSeconds)
        }
    }
    Write-Host 'phase=settled'
}

# --- Phase 3: raw uncompressed capture --------------------------------------
$sessionId = "headless_raw_$stamp"
$captureId = "headless-raw-capture-$stamp"
$durationArg = [string]::Format([Globalization.CultureInfo]::InvariantCulture, '{0:0.###}', $DurationSeconds)
$recorderArgs = @(
    '--pid', "$gamePid",
    '--dll', $collectorDll,
    '--out-dir', $OutDir,
    '--duration-s', $durationArg,
    '--sample-ms', "$SampleMilliseconds",
    '--min-game-hours', "$MinGameHours",
    '--stream-format', 'compact',
    '--stream-compression', 'none',
    '--compact-country-frame',
    '--hour-latch',
    '--reader-concurrency', "$ReaderConcurrency",
    '--session-id', $sessionId,
    '--capture-id', $captureId
)
foreach ($domain in $scheduledDomains) {
    $recorderArgs += '--scheduled-domain'
    $recorderArgs += $domain
}
foreach ($entry in $staticCatalogs.GetEnumerator()) {
    $recorderArgs += $entry.Key
    $recorderArgs += $entry.Value
}
if ($SkipHostFold) { $recorderArgs += '--skip-host-fold' }

($recorderArgs -join "`n") | Set-Content -LiteralPath (Join-Path $OutDir 'recorder_args.txt') -Encoding UTF8

$stdoutPath = Join-Path $OutDir 'collector_record.stdout.log'
$stderrPath = Join-Path $OutDir 'collector_record.stderr.log'
Write-Host "phase=capture duration_s=$durationArg compression=none scheduled_domains=$($scheduledDomains.Count) out_dir=$OutDir"

# Redirect straight to files rather than in-memory pipes: an in-memory read
# hides the recorder's progress until it exits, which makes a startup hang
# indistinguishable from a slow capture.
$recorderProcess = Start-Process -FilePath $collectorRecord `
    -ArgumentList (ConvertTo-NativeArgumentLine $recorderArgs) `
    -NoNewWindow -PassThru `
    -RedirectStandardOutput $stdoutPath `
    -RedirectStandardError $stderrPath

# Bound the wait: capture duration plus a startup/finalize margin. A recorder
# that outlives this budget is wedged, not slow, and hanging forever here is
# what previously left orphaned recorders holding machine-wide capture state.
#
# Also watch the game itself. Collector injection can kill the game, and the
# recorder does not always notice promptly -- it sat idle for the whole budget,
# turning a dead attempt into an 8-minute stall. There is nothing left to
# capture once the target is gone, so end the attempt immediately.
$recorderBudgetSeconds = $DurationSeconds + $RecorderOverheadSeconds
$recorderDeadline = [DateTime]::UtcNow.AddSeconds($recorderBudgetSeconds)
$gameDied = $false
while (-not $recorderProcess.HasExited) {
    if ([DateTime]::UtcNow -ge $recorderDeadline) {
        Write-Warning "collector_record exceeded its $recorderBudgetSeconds s budget (capture $DurationSeconds s + $RecorderOverheadSeconds s overhead); terminating so it cannot wedge later captures."
        try { $recorderProcess.Kill() } catch { }
        [void]$recorderProcess.WaitForExit(30000)
        Write-Host 'phase=capture_timeout'
        break
    }
    if (-not (Get-Process -Id $gamePid -ErrorAction SilentlyContinue)) {
        $gameDied = $true
        Write-Host "phase=game_died pid=$gamePid"
        Write-Warning "the game PID $gamePid exited during capture (collector injection can crash the game); ending this attempt."
        # Give the recorder a short grace period to finalize the valid prefix.
        if (-not $recorderProcess.WaitForExit(30000)) {
            try { $recorderProcess.Kill() } catch { }
            [void]$recorderProcess.WaitForExit(30000)
        }
        break
    }
    Start-Sleep -Seconds 2
}
if (-not $recorderProcess.HasExited) { [void]$recorderProcess.WaitForExit(30000) }
$recorderExit = $recorderProcess.ExitCode
Write-Host "phase=capture_done exit=$recorderExit"

# --- Phase 4: leave the machine in a known state ----------------------------
if (-not $KeepGameAfterCapture -and $null -ne $launchedProcess) {
    $launchedProcess.Refresh()
    if (-not $launchedProcess.HasExited) {
        Write-Host "phase=stop_game pid=$gamePid"
        try { $launchedProcess.CloseMainWindow() | Out-Null } catch { }
        if (-not $launchedProcess.WaitForExit(20000)) { $launchedProcess.Kill(); [void]$launchedProcess.WaitForExit(20000) }
    }
}

# --- Phase 5: read the outputs directly -------------------------------------
Write-Host ''
Get-ChildItem -LiteralPath $OutDir -Force | Sort-Object Length -Descending |
    Select-Object Name, Length | Format-Table -AutoSize | Out-String | Write-Host

# An attempt whose game died mid-capture legitimately has no manifest. Reading
# the outputs is reporting, not a gate: it must never turn a failed attempt into
# a terminating error for whatever is driving this script.
$manifestPath = Join-Path $OutDir 'native_session_manifest.json'
if ((Test-Path -LiteralPath $streamReport -PathType Leaf) -and
    (Test-Path -LiteralPath $manifestPath -PathType Leaf)) {
    try {
        $previous = $ErrorActionPreference
        $ErrorActionPreference = 'Continue'
        & $pythonExe $streamReport --run-dir $OutDir 2>&1 | Write-Host
    } catch {
        Write-Host "report failed: $($_.Exception.Message)"
    } finally {
        $ErrorActionPreference = $previous
    }
} elseif ($gameDied) {
    Write-Host 'no manifest: the game died during capture, so there is nothing to finalize'
} else {
    Write-Host "no manifest under $OutDir"
}

Write-Host "out_dir=$OutDir"
Write-Host "recorder_exit=$recorderExit"
Write-Host "game_died=$gameDied"
if ($null -eq $recorderExit) { exit 1 }
exit [int]$recorderExit
} finally {
    $settingsGuardRestoreSucceeded = $false
    if ($settingsGuardRestoreRequired) {
        try {
            $settingsRestoreItem = Get-Item -LiteralPath $settingsPath -Force
            $settingsRestoreItem.IsReadOnly = $settingsGuardPriorReadOnly
            $settingsRestoreItem.Refresh()
            if ([bool]$settingsRestoreItem.IsReadOnly -ne $settingsGuardPriorReadOnly) {
                throw "IsReadOnly remained $($settingsRestoreItem.IsReadOnly)"
            }
            $settingsGuardRestoreSucceeded = $true
        } catch {
            Write-SettingsGuardNonThrowingWarning -Message "SETTINGS GUARD RESTORE FAILED for '$settingsPath'. Manually set IsReadOnly=$settingsGuardPriorReadOnly, then remove sentinel '$settingsGuardSentinel'. The sentinel was retained for recovery. Error: $($_.Exception.Message)"
        }
    }

    $settingsGuardOwnerStreamClosed = $true
    if ($null -ne $settingsGuardOwnerStream) {
        try {
            $settingsGuardOwnerStream.Dispose()
            $settingsGuardOwnerStream = $null
        } catch {
            $settingsGuardOwnerStreamClosed = $false
            Write-SettingsGuardNonThrowingWarning -Message "Settings guard restored=$settingsGuardRestoreSucceeded for '$settingsPath', but could not close its live-owner handle for sentinel '$settingsGuardSentinel'. The sentinel was retained. Error: $($_.Exception.Message)"
        }
    }

    if ($settingsGuardRestoreRequired -and
        $settingsGuardRestoreSucceeded -and
        $settingsGuardOwnerStreamClosed) {
        Write-Host "phase=settings_guard action=release prior_read_only=$settingsGuardPriorReadOnly restored_read_only=$settingsGuardPriorReadOnly path=$settingsPath sentinel=$settingsGuardSentinel"
        try {
            Remove-Item -LiteralPath $settingsGuardSentinel -Force
        } catch {
            Write-SettingsGuardNonThrowingWarning -Message "Settings guard restored IsReadOnly=$settingsGuardPriorReadOnly on '$settingsPath', but could not remove sentinel '$settingsGuardSentinel'. Remove it manually after verifying the attribute. Error: $($_.Exception.Message)"
        }
    }
}
