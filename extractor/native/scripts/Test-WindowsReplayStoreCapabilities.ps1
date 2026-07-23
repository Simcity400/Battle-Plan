#Requires -Version 5.1

<#
.SYNOPSIS
Fail-closed preflight for the Windows capabilities used by production capture.

.DESCRIPTION
The symlink gate creates and removes one isolated link below the process temp
directory.  It never changes Developer Mode or any other machine setting.  The
OneDrive gate never opens replay payload bytes or changes pin/hydration state.
The WMI and Job-object probes create only process-local, bounded handles.  The
multi-GiB gate inspects the selected filesystem and free-space prerequisites;
it does not claim that the separate streaming/container stress run passed.

READY_TO_RUN means that the observed capability is available, or that the
separate evidence-bearing integration test has non-vacuous prerequisites.
NOT_APPLICABLE records an unavailable attack-surface fixture without claiming
its rejection test ran and leaves aggregate completion blocked. BLOCKED and
PROBE_ERROR are machine-readable, fail-closed outcomes; none is misreported as
a request for product-owner input.
#>

[CmdletBinding()]
param(
    [Parameter()]
    [string]$ReplayStore = $(if (Test-Path Env:BATTLE_PLAN_ONEDRIVE_DEHYDRATED_REPLAY_STORE) { $env:BATTLE_PLAN_ONEDRIVE_DEHYDRATED_REPLAY_STORE } else { $env:HOI4_ONEDRIVE_DEHYDRATED_REPLAY_STORE }),

    [Parameter()]
    [ValidateSet('All', 'DeveloperMode', 'Symlink', 'OneDrive', 'Wmi', 'JobObject', 'MultiGib')]
    [string]$Gate = 'All',

    [Parameter()]
    [string]$StressRoot = $(if (Test-Path Env:BATTLE_PLAN_WINDOWS_MULTI_GIB_STRESS_ROOT) { $env:BATTLE_PLAN_WINDOWS_MULTI_GIB_STRESS_ROOT } else { $env:HOI4_WINDOWS_MULTI_GIB_STRESS_ROOT }),

    [Parameter()]
    [ValidateRange(1, 128)]
    [int]$OneDriveMaxDepth = 32,

    [Parameter()]
    [ValidateRange(1, 1000000)]
    [int]$OneDriveMaxEntries = 250000,

    [Parameter()]
    [ValidateRange(1, 3600)]
    [int]$OneDriveTimeoutSeconds = 30,

    [Parameter()]
    [switch]$Json,

    [Parameter()]
    [string]$OutFile,

    [Parameter()]
    [switch]$RequireReady,

    [Parameter()]
    [switch]$SelfTest
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'WindowsPath.Identity.ps1')

if (-not ('Hoi4MpStats.WindowsCapabilityNative' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;
using Microsoft.Win32.SafeHandles;

namespace Hoi4MpStats {
    public static class WindowsCapabilityNative {
        private const UInt32 SYMBOLIC_LINK_FLAG_ALLOW_UNPRIVILEGED_CREATE = 0x2;
        private const Int32 JobObjectBasicAccountingInformation = 1;
        private const UInt32 FILE_SHARE_READ = 0x1;
        private const UInt32 FILE_SHARE_WRITE = 0x2;
        private const UInt32 FILE_SHARE_DELETE = 0x4;
        private const UInt32 OPEN_EXISTING = 3;
        private const UInt32 FILE_FLAG_BACKUP_SEMANTICS = 0x02000000;
        private const UInt32 FILE_FLAG_OPEN_REPARSE_POINT = 0x00200000;

        [StructLayout(LayoutKind.Sequential)]
        private struct IO_COUNTERS {
            public UInt64 ReadOperationCount;
            public UInt64 WriteOperationCount;
            public UInt64 OtherOperationCount;
            public UInt64 ReadTransferCount;
            public UInt64 WriteTransferCount;
            public UInt64 OtherTransferCount;
        }

        [StructLayout(LayoutKind.Sequential)]
        private struct JOBOBJECT_BASIC_ACCOUNTING_INFORMATION {
            public Int64 TotalUserTime;
            public Int64 TotalKernelTime;
            public Int64 ThisPeriodTotalUserTime;
            public Int64 ThisPeriodTotalKernelTime;
            public UInt32 TotalPageFaultCount;
            public UInt32 TotalProcesses;
            public UInt32 ActiveProcesses;
            public UInt32 TotalTerminatedProcesses;
        }

        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        private static extern bool CreateSymbolicLinkW(
            string symlinkFileName, string targetFileName, UInt32 flags);

        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        private static extern SafeFileHandle CreateJobObjectW(IntPtr attributes, string name);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern bool QueryInformationJobObject(
            SafeFileHandle job, Int32 infoClass,
            out JOBOBJECT_BASIC_ACCOUNTING_INFORMATION info,
            UInt32 size, IntPtr returnLength);

        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        private static extern SafeFileHandle CreateFileW(
            string path, UInt32 desiredAccess, UInt32 shareMode, IntPtr security,
            UInt32 creationDisposition, UInt32 flags, IntPtr template);

        public static void CreateFileSymlink(string link, string target) {
            if (!CreateSymbolicLinkW(
                link, target, SYMBOLIC_LINK_FLAG_ALLOW_UNPRIVILEGED_CREATE)) {
                int error = Marshal.GetLastWin32Error();
                // Older supported Windows builds reject the opt-in flag. A
                // privileged token can still prove the same capability with
                // the original API contract.
                if (error == 87 && CreateSymbolicLinkW(link, target, 0)) return;
                throw new Win32Exception(
                    error == 87 ? Marshal.GetLastWin32Error() : error,
                    "CreateSymbolicLinkW failed");
            }
        }

        public static UInt32 ProbeEmptyJobObject() {
            using (SafeFileHandle job = CreateJobObjectW(IntPtr.Zero, null)) {
                if (job.IsInvalid) throw new Win32Exception(
                    Marshal.GetLastWin32Error(), "CreateJobObjectW failed");
                JOBOBJECT_BASIC_ACCOUNTING_INFORMATION info;
                if (!QueryInformationJobObject(
                    job, JobObjectBasicAccountingInformation, out info,
                    (UInt32)Marshal.SizeOf(typeof(JOBOBJECT_BASIC_ACCOUNTING_INFORMATION)),
                    IntPtr.Zero)) {
                    throw new Win32Exception(
                        Marshal.GetLastWin32Error(), "QueryInformationJobObject failed");
                }
                return info.ActiveProcesses;
            }
        }

        public static bool PathEntryExists(string path) {
            using (SafeFileHandle entry = CreateFileW(
                path, 0, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                IntPtr.Zero, OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
                IntPtr.Zero)) {
                if (!entry.IsInvalid) return true;
                int error = Marshal.GetLastWin32Error();
                if (error == 2 || error == 3) return false;
                throw new Win32Exception(error, "reparse-aware residue check failed");
            }
        }
    }
}
'@
}

function New-GateResult {
    param(
        [Parameter(Mandatory)] [string]$Gate,
        [Parameter(Mandatory)] [string]$Status,
        [Parameter(Mandatory)] [string]$ReasonCode,
        [Parameter(Mandatory)] [string]$Detail,
        [Parameter()] [hashtable]$Evidence = @{}
    )

    [pscustomobject]@{
        gate = $Gate
        status = $Status
        reason_code = $ReasonCode
        detail = $Detail
        evidence = [pscustomobject]$Evidence
    }
}

function Get-OverallStatus {
    param([Parameter(Mandatory)] [object[]]$Gates)
    if (@($Gates | Where-Object { $_.status -eq 'PROBE_ERROR' }).Count -gt 0) {
        return 'PROBE_ERROR'
    }
    if (@($Gates | Where-Object { $_.status -ne 'READY_TO_RUN' }).Count -gt 0) {
        return 'BLOCKED'
    }
    'READY_TO_RUN'
}

function Test-SymlinkEvidence {
    param([bool]$Created, [uint32]$ReparseTag)
    $nameSurrogate = [uint32]0x20000000
    $Created -and (($ReparseTag -band $nameSurrogate) -ne 0)
}

function Test-WmiEvidence {
    param([bool]$OwnPidObserved, [bool]$SubscriptionStartedAndStopped)
    $OwnPidObserved -and $SubscriptionStartedAndStopped
}

function Test-JobObjectEvidence {
    param([bool]$Created, [bool]$Queried, [uint32]$ActiveProcesses)
    $Created -and $Queried -and $ActiveProcesses -eq 0
}

function Test-MultiGibPrerequisites {
    param([bool]$Is64BitProcess, [string]$DriveFormat, [int64]$AvailableFreeSpace)
    $minimumFreeBytes = [int64](12GB)
    $supportedLargeFileFormats = @('NTFS', 'ReFS', 'exFAT')
    $Is64BitProcess -and
        $supportedLargeFileFormats -contains $DriveFormat -and
        $AvailableFreeSpace -ge $minimumFreeBytes
}

function Remove-ProbeLeafReparseAware {
    param(
        [Parameter(Mandatory)] [string]$LiteralPath,
        [Parameter(Mandatory)] [string]$ExpectedParentFinalPath
    )
    $problems = [System.Collections.Generic.List[string]]::new()
    try {
        if ([Hoi4MpStats.WindowsCapabilityNative]::PathEntryExists($LiteralPath)) {
            $identity = Get-WindowsPathIdentity -LiteralPath $LiteralPath -OpenReparsePoint
            if (-not (Test-WindowsPathContainedByRoot `
                    -Candidate $identity.final_path -Root $ExpectedParentFinalPath)) {
                $problems.Add('leaf identity escaped the expected probe parent')
            }
            else {
                # File.Delete removes the link itself when the leaf is a file
                # symlink; it does not follow the target.
                [System.IO.File]::Delete($LiteralPath)
            }
        }
    }
    catch {
        $problems.Add(('reparse-aware leaf cleanup failed: ' + $_.Exception.GetType().FullName))
    }
    $residue = $true
    try {
        $residue = [Hoi4MpStats.WindowsCapabilityNative]::PathEntryExists($LiteralPath)
    }
    catch {
        $problems.Add(('probe leaf residue check failed: ' + $_.Exception.GetType().FullName))
    }
    if ($residue) { $problems.Add('probe leaf residue remains') }
    [pscustomobject]@{
        ok = $problems.Count -eq 0
        residue = $residue
        problems = @($problems)
    }
}

function Remove-ProbeDirectoryIdentityAware {
    param(
        [Parameter(Mandatory)] [string]$LiteralPath,
        [Parameter(Mandatory)] [string]$ExpectedFileId128
    )
    $problems = [System.Collections.Generic.List[string]]::new()
    try {
        if ([Hoi4MpStats.WindowsCapabilityNative]::PathEntryExists($LiteralPath)) {
            $identity = Get-WindowsPathIdentity -LiteralPath $LiteralPath -OpenReparsePoint
            if ($identity.file_id_128 -cne $ExpectedFileId128 -or $identity.reparse_tag -ne 0) {
                $problems.Add('probe directory identity changed before cleanup')
            }
            else {
                [System.IO.Directory]::Delete($LiteralPath, $false)
            }
        }
    }
    catch {
        $problems.Add(('identity-aware directory cleanup failed: ' + $_.Exception.GetType().FullName))
    }
    $residue = $true
    try {
        $residue = [Hoi4MpStats.WindowsCapabilityNative]::PathEntryExists($LiteralPath)
    }
    catch {
        $problems.Add(('probe directory residue check failed: ' + $_.Exception.GetType().FullName))
    }
    if ($residue) { $problems.Add('probe directory residue remains') }
    [pscustomobject]@{
        ok = $problems.Count -eq 0
        residue = $residue
        problems = @($problems)
    }
}

function Get-SymlinkCapabilityGate {
    $key = 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\AppModelUnlock'
    $registryValue = $null
    try {
        if (Test-Path -LiteralPath $key) {
            $registryValue = (Get-ItemProperty -LiteralPath $key `
                    -Name AllowDevelopmentWithoutDevLicense `
                    -ErrorAction SilentlyContinue).AllowDevelopmentWithoutDevLicense
        }
    }
    catch {
        # Registry state is context only. The actual CreateSymbolicLinkW result
        # below is the capability authority.
        $registryValue = $null
    }

    $probeRoot = Join-Path ([System.IO.Path]::GetTempPath()) (
        'hoi4-f3-symlink-' + [guid]::NewGuid().ToString('N')
    )
    $target = Join-Path $probeRoot 'target.bin'
    $link = Join-Path $probeRoot 'link.bin'
    $rootIdentity = $null
    $result = try {
        [System.IO.Directory]::CreateDirectory($probeRoot) | Out-Null
        $rootIdentity = Get-WindowsPathIdentity -LiteralPath $probeRoot -OpenReparsePoint
        [System.IO.File]::WriteAllBytes($target, [byte[]]@(0x48, 0x34, 0x43, 0x52))
        [Hoi4MpStats.WindowsCapabilityNative]::CreateFileSymlink($link, $target)
        $identity = Get-WindowsPathIdentity -LiteralPath $link -OpenReparsePoint
        if (-not (Test-SymlinkEvidence -Created $true -ReparseTag $identity.reparse_tag)) {
            New-GateResult `
                -Gate 'file_symlink_creation_and_rejection' `
                -Status 'PROBE_ERROR' `
                -ReasonCode 'SYMLINK_CREATED_WITHOUT_NAME_SURROGATE_TAG' `
                -Detail 'CreateSymbolicLinkW succeeded, but the metadata-only identity probe did not observe a name-surrogate reparse tag.' `
                -Evidence @{
                    create_symbolic_link_succeeded = $true
                    reparse_tag = ('0x{0:x8}' -f [uint32]$identity.reparse_tag)
                    developer_mode_registry_value = $registryValue
                    machine_setting_changed = $false
                }
        }
        else {
            New-GateResult `
            -Gate 'file_symlink_creation_and_rejection' `
            -Status 'READY_TO_RUN' `
            -ReasonCode 'SYMLINK_CREATION_AND_NAME_SURROGATE_METADATA_AVAILABLE' `
            -Detail 'A disposable file symlink was created and recognized through a metadata-only reparse-point handle; the consumer rejection test may run.' `
            -Evidence @{
                create_symbolic_link_succeeded = $true
                reparse_tag = ('0x{0:x8}' -f [uint32]$identity.reparse_tag)
                developer_mode_registry_value = $registryValue
                machine_setting_changed = $false
            }
        }
    }
    catch {
        $nativeCode = if ($_.Exception -is [System.ComponentModel.Win32Exception]) {
            [int]$_.Exception.NativeErrorCode
        }
        elseif ($null -ne $_.Exception.InnerException -and
                $_.Exception.InnerException -is [System.ComponentModel.Win32Exception]) {
            [int]$_.Exception.InnerException.NativeErrorCode
        }
        else { $null }
        New-GateResult `
            -Gate 'file_symlink_creation_and_rejection' `
            -Status $(if ($nativeCode -eq 1314) { 'NOT_APPLICABLE' } else { 'PROBE_ERROR' }) `
            -ReasonCode $(if ($nativeCode -eq 1314) { 'SYMLINK_CREATION_UNAVAILABLE_FOR_CURRENT_TOKEN' } else { 'SYMLINK_CAPABILITY_PROBE_FAILED' }) `
            -Detail $(if ($nativeCode -eq 1314) { 'The current token cannot create a file symlink, so this optional fixture is not applicable; no rejection pass is claimed and no machine setting must be changed.' } else { 'The bounded CreateSymbolicLinkW probe failed indeterminately; no machine setting was changed and capability admission remains closed.' }) `
            -Evidence @{
                create_symbolic_link_succeeded = $false
                win32_error = $nativeCode
                developer_mode_registry_value = $registryValue
                machine_setting_changed = $false
            }
    }

    # Cleanup is independent of the probe verdict: every artifact is attempted
    # even if an earlier deletion fails. Any residue overrides success or N/A.
    $cleanupProblems = [System.Collections.Generic.List[string]]::new()
    if ($null -ne $rootIdentity) {
        foreach ($leaf in @($link, $target)) {
            $cleanup = Remove-ProbeLeafReparseAware `
                -LiteralPath $leaf -ExpectedParentFinalPath $rootIdentity.final_path
            foreach ($problem in @($cleanup.problems)) { $cleanupProblems.Add([string]$problem) }
        }
        $directoryCleanup = Remove-ProbeDirectoryIdentityAware `
            -LiteralPath $probeRoot -ExpectedFileId128 $rootIdentity.file_id_128
        foreach ($problem in @($directoryCleanup.problems)) {
            $cleanupProblems.Add([string]$problem)
        }
    }
    else {
        try {
            if ([Hoi4MpStats.WindowsCapabilityNative]::PathEntryExists($probeRoot)) {
                $cleanupProblems.Add('probe root exists but its creation identity was not captured')
            }
        }
        catch {
            $cleanupProblems.Add('probe root residue is indeterminate because its metadata check failed')
        }
    }
    if ($cleanupProblems.Count -gt 0) {
        return New-GateResult `
            -Gate 'file_symlink_creation_and_rejection' `
            -Status 'PROBE_ERROR' `
            -ReasonCode 'SYMLINK_PROBE_CLEANUP_INCOMPLETE' `
            -Detail 'Independent reparse-aware cleanup failed or left residue; capability admission remains closed.' `
            -Evidence @{
                cleanup_complete = $false
                cleanup_problems = @($cleanupProblems)
                machine_setting_changed = $false
            }
    }
    $result.evidence | Add-Member -NotePropertyName cleanup_complete -NotePropertyValue $true
    $result
}

function Get-WmiLiveSourceGate {
    $watcher = $null
    $subscriptionStarted = $false
    try {
        $query = New-Object System.Management.ObjectQuery(
            "SELECT ProcessId FROM Win32_Process WHERE ProcessId = $PID"
        )
        $searcher = New-Object System.Management.ManagementObjectSearcher($query)
        try {
            $ownPidObserved = @($searcher.Get() | Where-Object {
                [uint32]$_.ProcessId -eq [uint32]$PID
            }).Count -eq 1
        }
        finally {
            $searcher.Dispose()
        }
        $eventQuery = New-Object System.Management.WqlEventQuery(
            'SELECT * FROM Win32_ProcessStartTrace'
        )
        $watcher = New-Object System.Management.ManagementEventWatcher($eventQuery)
        $watcher.Start()
        $subscriptionStarted = $true
        $watcher.Stop()
        if (-not (Test-WmiEvidence $ownPidObserved $subscriptionStarted)) {
            return New-GateResult `
                -Gate 'wmi_live_process_source' `
                -Status 'PROBE_ERROR' `
                -ReasonCode 'WMI_LIVE_SOURCE_INCOMPLETE' `
                -Detail 'WMI did not both identify this process and complete a process-start event subscription lifecycle.' `
                -Evidence @{ own_pid_observed = $ownPidObserved; subscription_start_stop = $subscriptionStarted }
        }
        New-GateResult `
            -Gate 'wmi_live_process_source' `
            -Status 'READY_TO_RUN' `
            -ReasonCode 'WMI_QUERY_AND_EVENT_SUBSCRIPTION_AVAILABLE' `
            -Detail 'WMI identified this process and completed a bounded process-start subscription start/stop lifecycle without launching a process.' `
            -Evidence @{ own_pid_observed = $true; subscription_start_stop = $true; process_launched = $false }
    }
    catch {
        New-GateResult `
            -Gate 'wmi_live_process_source' `
            -Status 'PROBE_ERROR' `
            -ReasonCode 'WMI_LIVE_SOURCE_PROBE_FAILED' `
            -Detail 'The WMI query or process-start event subscription lifecycle failed; WMI auto-attach remains disabled.' `
            -Evidence @{ own_pid_observed = $false; subscription_start_stop = $subscriptionStarted; exception_type = $_.Exception.GetType().FullName }
    }
    finally {
        if ($null -ne $watcher) { $watcher.Dispose() }
    }
}

function Get-JobObjectGate {
    try {
        [uint32]$activeProcesses = [Hoi4MpStats.WindowsCapabilityNative]::ProbeEmptyJobObject()
        if (-not (Test-JobObjectEvidence $true $true $activeProcesses)) {
            return New-GateResult `
                -Gate 'job_object_lifecycle' `
                -Status 'PROBE_ERROR' `
                -ReasonCode 'JOB_OBJECT_EMPTY_ACCOUNTING_INVALID' `
                -Detail 'The disposable Job object did not report the required empty accounting state.' `
                -Evidence @{ created = $true; accounting_queried = $true; active_processes = $activeProcesses }
        }
        New-GateResult `
            -Gate 'job_object_lifecycle' `
            -Status 'READY_TO_RUN' `
            -ReasonCode 'JOB_OBJECT_CREATE_QUERY_CLOSE_AVAILABLE' `
            -Detail 'A disposable empty Job object was created, queried, and closed; no process was assigned or controlled.' `
            -Evidence @{ created = $true; accounting_queried = $true; active_processes = 0; process_assigned = $false }
    }
    catch {
        New-GateResult `
            -Gate 'job_object_lifecycle' `
            -Status 'PROBE_ERROR' `
            -ReasonCode 'JOB_OBJECT_LIFECYCLE_PROBE_FAILED' `
            -Detail 'Create/query/close of a disposable empty Job object failed; Job-backed lifecycle supervision remains disabled.' `
            -Evidence @{ created = $false; accounting_queried = $false; process_assigned = $false; exception_type = $_.Exception.GetType().FullName }
    }
}

function Get-MultiGibStreamingGate {
    param([AllowEmptyString()] [string]$Path)
    $selected = if ([string]::IsNullOrWhiteSpace($Path)) {
        [System.IO.Path]::GetTempPath()
    }
    else { $Path }
    try {
        $selectedFull = [System.IO.Path]::GetFullPath($selected)
        $selectedRoot = [System.IO.Path]::GetPathRoot($selectedFull)
        if (-not $selectedFull.Equals($selectedRoot, [System.StringComparison]::OrdinalIgnoreCase)) {
            $selectedFull = $selectedFull.TrimEnd('\')
        }
    }
    catch {
        return New-GateResult `
            -Gate 'multi_gib_streaming_container_stress' `
            -Status 'PROBE_ERROR' `
            -ReasonCode 'MULTI_GIB_STRESS_PATH_INVALID' `
            -Detail 'The selected stress path is not a valid absolute Windows path; no filesystem mutation occurred.' `
            -Evidence @{ stress_executed = $false; writability_probe_created = $false; exception_type = $_.Exception.GetType().FullName }
    }
    if (-not (Test-Path -LiteralPath $selectedFull -PathType Container)) {
        return New-GateResult `
            -Gate 'multi_gib_streaming_container_stress' `
            -Status 'BLOCKED' `
            -ReasonCode 'MULTI_GIB_STRESS_DIRECTORY_NOT_FOUND' `
            -Detail 'The exact selected stress path must already exist as a directory; no ancestor substitution or directory creation is allowed.' `
            -Evidence @{ selected_path = $selectedFull; stress_executed = $false; writability_probe_created = $false }
    }

    $writeProbe = $null
    $directoryIdentity = $null
    $result = try {
        $directoryIdentity = Get-WindowsPathIdentity -LiteralPath $selectedFull
        $resolved = [string]$directoryIdentity.final_path
        $volumeRoot = [System.IO.Path]::GetPathRoot($resolved)
        $drive = New-Object System.IO.DriveInfo($volumeRoot)
        $is64 = [Environment]::Is64BitProcess
        $format = [string]$drive.DriveFormat
        $driveType = [string]$drive.DriveType
        $free = [int64]$drive.AvailableFreeSpace

        # Prove that this exact, handle-resolved directory is writable. The
        # one-byte leaf is never used as stress evidence and is always cleaned
        # independently below.
        $writeProbe = Join-Path $resolved (
            '.hoi4-f3-write-probe-' + [guid]::NewGuid().ToString('N') + '.tmp'
        )
        $stream = [System.IO.File]::Open(
            $writeProbe, [System.IO.FileMode]::CreateNew,
            [System.IO.FileAccess]::ReadWrite, [System.IO.FileShare]::None
        )
        try {
            $stream.WriteByte(0x00)
            $stream.Flush($true)
        }
        finally { $stream.Dispose() }
        $writeIdentity = Get-WindowsPathIdentity -LiteralPath $writeProbe -OpenReparsePoint
        if (-not (Test-WindowsPathContainedByRoot `
                -Candidate $writeIdentity.final_path -Root $resolved) -or
            $writeIdentity.volume_serial_number -cne $directoryIdentity.volume_serial_number) {
            throw 'Writability probe escaped the selected directory identity.'
        }

        $evidence = @{
            selected_path = $selectedFull
            resolved_selected_path = $resolved
            volume_root = $volumeRoot
            drive_type = $driveType
            drive_format = $format
            supported_large_file_formats = @('NTFS', 'ReFS', 'exFAT')
            is_64_bit_process = $is64
            available_free_bytes = $free
            required_free_bytes = [int64](12GB)
            writable_exact_directory = $true
            writability_probe_created = $true
            stress_executed = $false
            stress_completed = $false
        }
        if ($drive.DriveType -ne [System.IO.DriveType]::Fixed -or
            -not (Test-MultiGibPrerequisites $is64 $format $free)) {
            New-GateResult `
                -Gate 'multi_gib_streaming_container_stress' `
                -Status 'BLOCKED' `
                -ReasonCode 'MULTI_GIB_STRESS_PREREQUISITES_UNAVAILABLE' `
                -Detail 'The exact writable stress directory lacks a fixed NTFS/ReFS/exFAT volume, a 64-bit host, or 12 GiB free; no stress completion is claimed.' `
                -Evidence $evidence
        }
        else {
            New-GateResult `
                -Gate 'multi_gib_streaming_container_stress' `
                -Status 'READY_TO_RUN' `
                -ReasonCode 'MULTI_GIB_STRESS_PREREQUISITES_AVAILABLE' `
                -Detail 'The exact handle-resolved directory is writable on a fixed large-file filesystem with sufficient space; the separate multi-GiB streaming/container stress has not run.' `
                -Evidence $evidence
        }
    }
    catch {
        New-GateResult `
            -Gate 'multi_gib_streaming_container_stress' `
            -Status 'PROBE_ERROR' `
            -ReasonCode 'MULTI_GIB_STRESS_PREFLIGHT_FAILED' `
            -Detail 'The exact selected stress directory could not be identified or write-probed safely; no stress completion is claimed.' `
            -Evidence @{
                selected_path = $selectedFull
                stress_executed = $false
                stress_completed = $false
                exception_type = $_.Exception.GetType().FullName
            }
    }

    if ($null -ne $writeProbe -and $null -ne $directoryIdentity) {
        $cleanup = Remove-ProbeLeafReparseAware `
            -LiteralPath $writeProbe -ExpectedParentFinalPath $directoryIdentity.final_path
        if (-not $cleanup.ok) {
            return New-GateResult `
                -Gate 'multi_gib_streaming_container_stress' `
                -Status 'PROBE_ERROR' `
                -ReasonCode 'MULTI_GIB_WRITABILITY_PROBE_CLEANUP_INCOMPLETE' `
                -Detail 'Reparse-aware cleanup of the exact-directory writability probe failed or left residue.' `
                -Evidence @{
                    selected_path = $selectedFull
                    resolved_selected_path = $directoryIdentity.final_path
                    cleanup_complete = $false
                    cleanup_problems = @($cleanup.problems)
                    stress_executed = $false
                    stress_completed = $false
                }
        }
        $result.evidence | Add-Member -NotePropertyName cleanup_complete -NotePropertyValue $true
    }
    $result
}

function Get-DehydratedStoreGate {
    param(
        [AllowEmptyString()] [string]$Path,
        [Parameter(Mandatory)] [int]$MaxDepth,
        [Parameter(Mandatory)] [int]$MaxEntries,
        [Parameter(Mandatory)] [int]$TimeoutSeconds
    )

    if ([string]::IsNullOrWhiteSpace($Path)) {
        return New-GateResult `
            -Gate 'onedrive_dehydrated_directory_store' `
            -Status 'BLOCKED' `
            -ReasonCode 'ONEDRIVE_STORE_ENV_NOT_SET' `
            -Detail 'Set BATTLE_PLAN_ONEDRIVE_DEHYDRATED_REPLAY_STORE to a genuine dehydrated directory store. The probe will not create or dehydrate one.'
    }

    try {
        $root = Get-Item -LiteralPath $Path -Force -ErrorAction Stop
    }
    catch {
        return New-GateResult `
            -Gate 'onedrive_dehydrated_directory_store' `
            -Status 'BLOCKED' `
            -ReasonCode 'ONEDRIVE_STORE_PATH_UNAVAILABLE' `
            -Detail 'The configured replay-store path is unavailable.' `
            -Evidence @{ configured_path_supplied = $true; exception_type = $_.Exception.GetType().FullName }
    }

    if (-not $root.PSIsContainer) {
        return New-GateResult `
            -Gate 'onedrive_dehydrated_directory_store' `
            -Status 'BLOCKED' `
            -ReasonCode 'ONEDRIVE_STORE_NOT_DIRECTORY' `
            -Detail 'The configured path is not a directory replay store.' `
            -Evidence @{ configured_path_supplied = $true }
    }

    try {
        $rootIdentity = Get-WindowsPathIdentity -LiteralPath $root.FullName
        $oneDriveRoots = @(Get-OneDriveAccountRoots)
        $matchingAccounts = @($oneDriveRoots | Where-Object {
            Test-WindowsPathContainedByRoot -Candidate $rootIdentity.final_path -Root $_.final_path
        })
    }
    catch {
        return New-GateResult `
            -Gate 'onedrive_dehydrated_directory_store' `
            -Status 'PROBE_ERROR' `
            -ReasonCode 'ONEDRIVE_ACCOUNT_IDENTITY_PROBE_FAILED' `
            -Detail 'Could not bind the replay store to a handle-resolved registered OneDrive account root.' `
            -Evidence @{ configured_path_supplied = $true; exception_type = $_.Exception.GetType().FullName }
    }
    if ($matchingAccounts.Count -eq 0) {
        return New-GateResult `
            -Gate 'onedrive_dehydrated_directory_store' `
            -Status 'BLOCKED' `
            -ReasonCode 'ONEDRIVE_ACCOUNT_ROOT_MISMATCH' `
            -Detail 'The handle-resolved replay store is not contained by any environment- or HKCU-registered OneDrive account root.' `
            -Evidence @{
                configured_path_supplied = $true
                registered_account_count = $oneDriveRoots.Count
                replay_store_volume_serial_number = $rootIdentity.volume_serial_number
                replay_store_file_id_128 = $rootIdentity.file_id_128
            }
    }

    # FILE_ATTRIBUTE_OFFLINE, FILE_ATTRIBUTE_RECALL_ON_OPEN, and
    # FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS.  Any of these is metadata evidence
    # that file data is not fully resident.  Merely having REPARSE_POINT is not
    # sufficient: ordinary junctions/symlinks and hydrated cloud files also have
    # that bit.
    [uint32]$offline = 0x00001000
    [uint32]$recallOnOpen = 0x00040000
    [uint32]$recallOnDataAccess = 0x00400000
    [uint32]$dehydratedMask = $offline -bor $recallOnOpen -bor $recallOnDataAccess
    [uint32]$reparsePoint = 0x00000400

    $pending = [System.Collections.Generic.Stack[object]]::new()
    $pending.Push([pscustomobject]@{ path = $root.FullName; depth = 0 })
    $visited = [System.Collections.Generic.HashSet[string]]::new(
        [System.StringComparer]::OrdinalIgnoreCase
    )
    $scannedFiles = 0
    $scannedEntries = 0
    $maxDepthObserved = 0
    $placeholderFiles = 0
    $replaySnapshotPlaceholderFiles = 0
    $offlineFiles = 0
    $recallOnOpenFiles = 0
    $recallOnDataAccessFiles = 0
    $skippedReparseDirectories = 0
    $sample = [System.Collections.Generic.List[string]]::new()
    $snapshotSample = [System.Collections.Generic.List[string]]::new()
    $snapshotCandidates = [System.Collections.Generic.List[string]]::new()
    $snapshotCandidateEvidence = @{}
    $metadataRows = [System.Collections.Generic.List[string]]::new()
    $rootPrefix = $root.FullName.TrimEnd('\', '/') + [System.IO.Path]::DirectorySeparatorChar
    $stopwatch = [System.Diagnostics.Stopwatch]::StartNew()

    try {
        while ($pending.Count -gt 0) {
            if ($stopwatch.Elapsed.TotalSeconds -gt $TimeoutSeconds) {
                throw 'ONEDRIVE_BOUND_EXHAUSTED:TIME'
            }
            $node = $pending.Pop()
            $directory = [string]$node.path
            $depth = [int]$node.depth
            if ($depth -gt $maxDepthObserved) { $maxDepthObserved = $depth }
            if (-not $visited.Add($directory)) {
                continue
            }
            foreach ($entry in Get-ChildItem -LiteralPath $directory -Force -ErrorAction Stop) {
                if ($stopwatch.Elapsed.TotalSeconds -gt $TimeoutSeconds) {
                    throw 'ONEDRIVE_BOUND_EXHAUSTED:TIME'
                }
                $scannedEntries++
                if ($scannedEntries -gt $MaxEntries) {
                    throw 'ONEDRIVE_BOUND_EXHAUSTED:ENTRY_COUNT'
                }
                [uint32]$attributes = [uint32]$entry.Attributes
                if ($entry.PSIsContainer) {
                    # Never follow a descendant reparse directory during a
                    # capability probe. The production loader will classify its
                    # tag through an anchored handle; pathname traversal here
                    # would undermine the non-mutating, bounded preflight.
                    if (($attributes -band $reparsePoint) -ne 0) {
                        $skippedReparseDirectories++
                    }
                    else {
                        $childDepth = $depth + 1
                        if ($childDepth -gt $MaxDepth) {
                            throw 'ONEDRIVE_BOUND_EXHAUSTED:DEPTH'
                        }
                        $pending.Push([pscustomobject]@{
                            path = $entry.FullName
                            depth = $childDepth
                        })
                    }
                    continue
                }

                $scannedFiles++
                $relative = if ($entry.FullName.StartsWith(
                    $rootPrefix,
                    [System.StringComparison]::OrdinalIgnoreCase
                )) {
                    $entry.FullName.Substring($rootPrefix.Length).Replace('\', '/')
                }
                else {
                    $entry.Name
                }
                $metadataRows.Add(("{0}`t{1}`t{2:x8}" -f $relative, [int64]$entry.Length, $attributes))
                if (($attributes -band $offline) -ne 0) { $offlineFiles++ }
                if (($attributes -band $recallOnOpen) -ne 0) { $recallOnOpenFiles++ }
                if (($attributes -band $recallOnDataAccess) -ne 0) { $recallOnDataAccessFiles++ }
                if (($attributes -band $dehydratedMask) -ne 0) {
                    $placeholderFiles++
                    if ($sample.Count -lt 8) {
                        $sample.Add($relative)
                    }
                    # The integration gate loads replay snapshots. An unrelated
                    # cloud placeholder elsewhere under the selected tree must
                    # not make that gate non-vacuous.
                    $relativeParts = @($relative.Split('/'))
                    if (($attributes -band $reparsePoint) -ne 0 -and
                        $relativeParts.Count -eq 2 -and
                        $entry.Name.EndsWith('.snapshot.json', [System.StringComparison]::Ordinal)) {
                        # Bind the exact leaf to a metadata-only handle.  A
                        # generic reparse point is insufficient: require the
                        # Microsoft Cloud Files tag family and re-check final
                        # path containment through the opened handle so a
                        # junction alias cannot manufacture provenance.
                        $candidateIdentity = Get-WindowsPathIdentity `
                            -LiteralPath $entry.FullName -OpenReparsePoint
                        $underStore = Test-WindowsPathContainedByRoot `
                            -Candidate $candidateIdentity.final_path -Root $rootIdentity.final_path
                        $underAccount = @($matchingAccounts | Where-Object {
                            Test-WindowsPathContainedByRoot `
                                -Candidate $candidateIdentity.final_path -Root $_.final_path
                        }).Count -gt 0
                        if ($underStore -and $underAccount -and
                            (Test-CloudFilesReparseTag -Tag $candidateIdentity.reparse_tag)) {
                            $replaySnapshotPlaceholderFiles++
                            $snapshotCandidates.Add($relative)
                            $snapshotCandidateEvidence[$relative] = [pscustomobject]@{
                                relative_path = $relative
                                reparse_tag = ('0x{0:x8}' -f $candidateIdentity.reparse_tag)
                                volume_serial_number = $candidateIdentity.volume_serial_number
                                file_id_128 = $candidateIdentity.file_id_128
                            }
                            $metadataRows.Add(("cloud_identity`t{0}`t{1}`t{2}`t{3}" -f
                                $relative,
                                ('0x{0:x8}' -f $candidateIdentity.reparse_tag),
                                $candidateIdentity.volume_serial_number,
                                $candidateIdentity.file_id_128))
                        }
                    }
                }
            }
        }
    }
    catch {
        $boundExhausted = $_.Exception.Message.StartsWith(
            'ONEDRIVE_BOUND_EXHAUSTED:', [System.StringComparison]::Ordinal
        )
        return New-GateResult `
            -Gate 'onedrive_dehydrated_directory_store' `
            -Status 'PROBE_ERROR' `
            -ReasonCode $(if ($boundExhausted) { 'ONEDRIVE_METADATA_BOUND_EXHAUSTED' } else { 'ONEDRIVE_METADATA_PROBE_FAILED' }) `
            -Detail $(if ($boundExhausted) { 'Metadata-only traversal exhausted its depth, entry-count, or wall-time bound; partial observations are not admissible.' } else { 'Metadata-only traversal failed.' }) `
            -Evidence @{
                scanned_files = $scannedFiles
                scanned_entries = $scannedEntries
                max_depth_observed = $maxDepthObserved
                max_depth_limit = $MaxDepth
                max_entries_limit = $MaxEntries
                timeout_seconds = $TimeoutSeconds
                elapsed_milliseconds = [int64]$stopwatch.ElapsedMilliseconds
                bound_exhausted = $boundExhausted
                exception_type = $_.Exception.GetType().FullName
            }
    }

    if ($stopwatch.Elapsed.TotalSeconds -gt $TimeoutSeconds) {
        return New-GateResult `
            -Gate 'onedrive_dehydrated_directory_store' `
            -Status 'PROBE_ERROR' `
            -ReasonCode 'ONEDRIVE_METADATA_BOUND_EXHAUSTED' `
            -Detail 'Metadata-only traversal exhausted its wall-time bound before canonicalization; partial observations are not admissible.' `
            -Evidence @{
                scanned_files = $scannedFiles; scanned_entries = $scannedEntries
                max_depth_observed = $maxDepthObserved; max_depth_limit = $MaxDepth
                max_entries_limit = $MaxEntries; timeout_seconds = $TimeoutSeconds
                elapsed_milliseconds = [int64]$stopwatch.ElapsedMilliseconds
                bound_exhausted = $true
            }
    }

    $metadataRows.Sort([System.StringComparer]::Ordinal)
    $sample.Sort([System.StringComparer]::Ordinal)
    $snapshotCandidates.Sort([System.StringComparer]::Ordinal)
    $snapshotSample.Clear()
    foreach ($candidate in @($snapshotCandidates | Select-Object -First 8)) {
        $snapshotSample.Add($candidate)
    }
    # Both product layers must observe distinct placeholders from one game.
    # Selecting one candidate from each of two unrelated games would not prove
    # the deployment card's same-store/same-index contract.
    $candidateGroups = @($snapshotCandidates | Group-Object {
        ([string]$_).Split('/')[0]
    } | Sort-Object Name)
    $selectedGroup = @($candidateGroups | Where-Object { $_.Count -ge 2 } | Select-Object -First 1)
    $selectedSnapshots = if ($selectedGroup.Count -eq 1) {
        @($selectedGroup[0].Group | Sort-Object | Select-Object -First 2 | ForEach-Object {
            $snapshotCandidateEvidence[$_]
        })
    }
    else { @() }
    $metadataMaterial = [System.Text.Encoding]::UTF8.GetBytes(
        [string]::Join("`n", $metadataRows)
    )
    $metadataHasher = [System.Security.Cryptography.SHA256]::Create()
    try {
        $metadataDigest = [System.BitConverter]::ToString(
            $metadataHasher.ComputeHash($metadataMaterial)
        ).Replace('-', '').ToLowerInvariant()
    }
    finally {
        $metadataHasher.Dispose()
    }
    if ($stopwatch.Elapsed.TotalSeconds -gt $TimeoutSeconds) {
        return New-GateResult `
            -Gate 'onedrive_dehydrated_directory_store' `
            -Status 'PROBE_ERROR' `
            -ReasonCode 'ONEDRIVE_METADATA_BOUND_EXHAUSTED' `
            -Detail 'Metadata canonicalization exhausted the wall-time bound; partial observations are not admissible.' `
            -Evidence @{
                scanned_files = $scannedFiles; scanned_entries = $scannedEntries
                max_depth_observed = $maxDepthObserved; max_depth_limit = $MaxDepth
                max_entries_limit = $MaxEntries; timeout_seconds = $TimeoutSeconds
                elapsed_milliseconds = [int64]$stopwatch.ElapsedMilliseconds
                bound_exhausted = $true
            }
    }

    $evidence = @{
        scanned_files = $scannedFiles
        scanned_entries = $scannedEntries
        max_depth_observed = $maxDepthObserved
        max_depth_limit = $MaxDepth
        max_entries_limit = $MaxEntries
        timeout_seconds = $TimeoutSeconds
        elapsed_milliseconds = [int64]$stopwatch.ElapsedMilliseconds
        bound_exhausted = $false
        replay_store_metadata_manifest_sha256 = $metadataDigest
        placeholder_files = $placeholderFiles
        replay_snapshot_placeholder_files = $replaySnapshotPlaceholderFiles
        offline_files = $offlineFiles
        recall_on_open_files = $recallOnOpenFiles
        recall_on_data_access_files = $recallOnDataAccessFiles
        skipped_reparse_directories = $skippedReparseDirectories
        placeholder_sample = @($sample)
        replay_snapshot_placeholder_sample = @($snapshotSample)
        selected_replay_snapshots = $selectedSnapshots
        registered_onedrive_account_label = [string]$matchingAccounts[0].account_label
        registered_onedrive_account_volume_serial_number = [string]$matchingAccounts[0].volume_serial_number
        registered_onedrive_account_file_id_128 = [string]$matchingAccounts[0].file_id_128
        replay_store_volume_serial_number = $rootIdentity.volume_serial_number
        replay_store_file_id_128 = $rootIdentity.file_id_128
        probe_opened_payload_bytes = $false
    }

    if ($selectedSnapshots.Count -ne 2) {
        return New-GateResult `
            -Gate 'onedrive_dehydrated_directory_store' `
            -Status 'BLOCKED' `
            -ReasonCode 'ONEDRIVE_REPLAY_SNAPSHOT_PAIR_NOT_DEHYDRATED' `
            -Detail 'No one game has two exact replay *.snapshot.json payloads carrying both REPARSE_POINT and OFFLINE/RECALL metadata; unrelated, cross-game, or fabricated ordinary-file attributes cannot close the replay-store gate.' `
            -Evidence $evidence
    }

    return New-GateResult `
        -Gate 'onedrive_dehydrated_directory_store' `
        -Status 'READY_TO_RUN' `
        -ReasonCode 'ONEDRIVE_CLOUD_PLACEHOLDER_METADATA_PRESENT' `
        -Detail 'Registered OneDrive-root containment and exact Cloud Files placeholder metadata are present. Running the integration test may hydrate files and therefore still requires explicit operator authorization.' `
        -Evidence $evidence
}

function Invoke-CapabilityProbeSelfTest {
    $state = [pscustomobject]@{
        failures = [System.Collections.Generic.List[string]]::new()
        passes = 0
    }
    function Assert-ProbeCase([string]$Name, [bool]$Condition) {
        if ($Condition) { $state.passes++ } else { $state.failures.Add($Name) }
    }

    $ready = New-GateResult -Gate 'a' -Status 'READY_TO_RUN' -ReasonCode 'A' -Detail 'a'
    $notApplicable = New-GateResult -Gate 'n' -Status 'NOT_APPLICABLE' -ReasonCode 'N' -Detail 'n'
    $blocked = New-GateResult -Gate 'b' -Status 'BLOCKED' -ReasonCode 'B' -Detail 'b'
    $error = New-GateResult -Gate 'c' -Status 'PROBE_ERROR' -ReasonCode 'C' -Detail 'c'
    Assert-ProbeCase 'all-ready aggregate' ((Get-OverallStatus @($ready, $ready)) -ceq 'READY_TO_RUN')
    Assert-ProbeCase 'not-applicable gate leaves aggregate incomplete' ((Get-OverallStatus @($ready, $notApplicable)) -ceq 'BLOCKED')
    Assert-ProbeCase 'blocked aggregate' ((Get-OverallStatus @($ready, $blocked)) -ceq 'BLOCKED')
    Assert-ProbeCase 'probe error dominates blocked' ((Get-OverallStatus @($blocked, $error)) -ceq 'PROBE_ERROR')

    $symlinkTag = [uint32]::Parse('a000000c', [System.Globalization.NumberStyles]::HexNumber)
    Assert-ProbeCase 'symlink name-surrogate admitted' (Test-SymlinkEvidence $true $symlinkTag)
    Assert-ProbeCase 'ordinary reparse tag rejected' (-not (Test-SymlinkEvidence $true ([uint32]0x10)))
    Assert-ProbeCase 'uncreated symlink rejected' (-not (Test-SymlinkEvidence $false $symlinkTag))

    Assert-ProbeCase 'complete WMI evidence admitted' (Test-WmiEvidence $true $true)
    Assert-ProbeCase 'query-only WMI evidence rejected' (-not (Test-WmiEvidence $true $false))
    Assert-ProbeCase 'subscription-only WMI evidence rejected' (-not (Test-WmiEvidence $false $true))

    Assert-ProbeCase 'empty queried Job admitted' (Test-JobObjectEvidence $true $true 0)
    Assert-ProbeCase 'unqueried Job rejected' (-not (Test-JobObjectEvidence $true $false 0))
    Assert-ProbeCase 'nonempty Job rejected' (-not (Test-JobObjectEvidence $true $true 1))

    Assert-ProbeCase 'large-file prerequisites admitted' (Test-MultiGibPrerequisites $true 'NTFS' ([int64](12GB)))
    Assert-ProbeCase 'ReFS large-file prerequisites admitted' (Test-MultiGibPrerequisites $true 'ReFS' ([int64](12GB)))
    Assert-ProbeCase 'exFAT large-file prerequisites admitted' (Test-MultiGibPrerequisites $true 'exFAT' ([int64](12GB)))
    Assert-ProbeCase 'FAT32 rejected' (-not (Test-MultiGibPrerequisites $true 'FAT32' ([int64](64GB))))
    Assert-ProbeCase 'unknown filesystem rejected' (-not (Test-MultiGibPrerequisites $true 'FutureFS' ([int64](64GB))))
    Assert-ProbeCase '32-bit host rejected' (-not (Test-MultiGibPrerequisites $false 'NTFS' ([int64](64GB))))
    Assert-ProbeCase 'insufficient free space rejected' (-not (Test-MultiGibPrerequisites $true 'NTFS' ([int64](12GB) - 1)))

    if ($state.failures.Count -gt 0) {
        throw ('Windows capability probe self-test failed: ' + ($state.failures -join '; '))
    }
    [pscustomobject]@{
        schema = 'hoi4_windows_capability_probe_self_test_v1'
        ok = $true
        passed = $state.passes
        failed = 0
    }
}

if ($SelfTest) {
    Invoke-CapabilityProbeSelfTest
    exit 0
}

$gates = switch ($Gate) {
    'DeveloperMode' { @(Get-SymlinkCapabilityGate) }
    'Symlink' { @(Get-SymlinkCapabilityGate) }
    'OneDrive' { @(
        Get-DehydratedStoreGate -Path $ReplayStore `
            -MaxDepth $OneDriveMaxDepth -MaxEntries $OneDriveMaxEntries `
            -TimeoutSeconds $OneDriveTimeoutSeconds
    ) }
    'Wmi' { @(Get-WmiLiveSourceGate) }
    'JobObject' { @(Get-JobObjectGate) }
    'MultiGib' { @(Get-MultiGibStreamingGate -Path $StressRoot) }
    default {
        @(
            Get-SymlinkCapabilityGate
            Get-DehydratedStoreGate -Path $ReplayStore `
                -MaxDepth $OneDriveMaxDepth -MaxEntries $OneDriveMaxEntries `
                -TimeoutSeconds $OneDriveTimeoutSeconds
            Get-WmiLiveSourceGate
            Get-JobObjectGate
            Get-MultiGibStreamingGate -Path $StressRoot
        )
    }
}
$ready = @($gates | Where-Object { $_.status -eq 'READY_TO_RUN' }).Count
$mutations = [System.Collections.Generic.List[string]]::new()
if ($Gate -in @('All', 'DeveloperMode', 'Symlink')) {
    $mutations.Add('temporary_symlink_fixture_create_delete_attempt')
}
if ($Gate -in @('All', 'Wmi')) {
    $mutations.Add('process_local_wmi_subscription_start_stop')
}
if ($Gate -in @('All', 'JobObject')) {
    $mutations.Add('process_local_empty_job_object_create_close')
}
if ($Gate -in @('All', 'MultiGib')) {
    $mutations.Add('temporary_exact_directory_writability_probe_create_delete_attempt')
}
if (-not [string]::IsNullOrWhiteSpace($OutFile)) {
    $mutations.Add('capability_report_file_created')
}
$report = [pscustomobject]@{
    schema = 'hoi4_windows_replay_store_capability_probe_v4'
    machine_settings_changed = $false
    payload_bytes_opened = $false
    mutations = @($mutations)
    selected_gate = $Gate
    probe_script_sha256 = (Get-FileHash -LiteralPath $PSCommandPath -Algorithm SHA256).Hash.ToLowerInvariant()
    path_identity_helper_sha256 = (Get-FileHash -LiteralPath (Join-Path $PSScriptRoot 'WindowsPath.Identity.ps1') -Algorithm SHA256).Hash.ToLowerInvariant()
    overall_status = Get-OverallStatus -Gates $gates
    ready_gate_count = $ready
    total_gate_count = $gates.Count
    gates = $gates
}

if (-not [string]::IsNullOrWhiteSpace($OutFile)) {
    $requestedOutputParent = Split-Path -Parent ([System.IO.Path]::GetFullPath($OutFile))
    $resolvedOutputParentBeforeCreate = Resolve-WindowsPathThroughExistingAncestor `
        -LiteralPath $requestedOutputParent
    if (-not [string]::IsNullOrWhiteSpace($ReplayStore) -and
        (Test-Path -LiteralPath $ReplayStore -PathType Container)) {
        $selectedStoreIdentity = Get-WindowsPathIdentity -LiteralPath $ReplayStore
        if (Test-WindowsPathContainedByRoot `
            -Candidate $resolvedOutputParentBeforeCreate `
            -Root $selectedStoreIdentity.final_path) {
            throw 'Capability evidence output must not modify the replay-store fixture.'
        }
    }
    foreach ($account in @(Get-OneDriveAccountRoots)) {
        if (Test-WindowsPathContainedByRoot `
            -Candidate $resolvedOutputParentBeforeCreate -Root $account.final_path) {
            throw 'Capability evidence output must be outside every registered OneDrive account root.'
        }
    }
    $outputAnchor = Resolve-WindowsDurableNewFilePath -LiteralPath $OutFile
    $utf8 = New-Object System.Text.UTF8Encoding($false)
    $payload = $utf8.GetBytes((($report | ConvertTo-Json -Depth 8) + [Environment]::NewLine))
    Publish-WindowsDurableNewFileBytes `
        -LiteralPath $outputAnchor.anchored_path -Bytes $payload | Out-Null
}

if ($Json) {
    $report | ConvertTo-Json -Depth 8
}
else {
    $report
}

if ($RequireReady -and $report.overall_status -ne 'READY_TO_RUN') {
    exit 2
}
