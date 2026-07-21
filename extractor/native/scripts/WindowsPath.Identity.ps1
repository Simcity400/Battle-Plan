#Requires -Version 5.1

# Shared Windows pathname identity helpers.  Every inspection opens metadata
# only (desired access zero); FILE_FLAG_OPEN_REPARSE_POINT is used only when a
# caller explicitly needs the leaf's reparse tag.  No payload stream is opened.

if (-not ('Hoi4MpStats.WindowsPathNative' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.ComponentModel;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;
using Microsoft.Win32.SafeHandles;

namespace Hoi4MpStats {
    public sealed class WindowsPathIdentity {
        public string FinalPath { get; set; }
        public UInt32 Attributes { get; set; }
        public UInt32 ReparseTag { get; set; }
        public UInt64 VolumeSerialNumber { get; set; }
        public string FileId128 { get; set; }
        public UInt64 FileIndex { get; set; }
    }

    public static class WindowsPathNative {
        private const UInt32 FILE_SHARE_READ = 0x00000001;
        private const UInt32 FILE_SHARE_WRITE = 0x00000002;
        private const UInt32 FILE_SHARE_DELETE = 0x00000004;
        private const UInt32 OPEN_EXISTING = 3;
        private const UInt32 FILE_FLAG_BACKUP_SEMANTICS = 0x02000000;
        private const UInt32 FILE_FLAG_OPEN_REPARSE_POINT = 0x00200000;
        private const Int32 FileAttributeTagInfo = 9;
        private const Int32 FileIdInfo = 18;
        private const UInt32 MOVEFILE_WRITE_THROUGH = 0x00000008;

        [StructLayout(LayoutKind.Sequential)]
        private struct BY_HANDLE_FILE_INFORMATION {
            public UInt32 FileAttributes;
            public System.Runtime.InteropServices.ComTypes.FILETIME CreationTime;
            public System.Runtime.InteropServices.ComTypes.FILETIME LastAccessTime;
            public System.Runtime.InteropServices.ComTypes.FILETIME LastWriteTime;
            public UInt32 VolumeSerialNumber;
            public UInt32 FileSizeHigh;
            public UInt32 FileSizeLow;
            public UInt32 NumberOfLinks;
            public UInt32 FileIndexHigh;
            public UInt32 FileIndexLow;
        }

        [StructLayout(LayoutKind.Sequential)]
        private struct FILE_ATTRIBUTE_TAG_INFO {
            public UInt32 FileAttributes;
            public UInt32 ReparseTag;
        }

        [StructLayout(LayoutKind.Sequential)]
        private struct FILE_ID_INFO {
            public UInt64 VolumeSerialNumber;
            public UInt64 FileIdLow;
            public UInt64 FileIdHigh;
        }

        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        private static extern SafeFileHandle CreateFileW(
            string name, UInt32 desiredAccess, UInt32 shareMode, IntPtr security,
            UInt32 creationDisposition, UInt32 flags, IntPtr template);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern bool GetFileInformationByHandle(
            SafeFileHandle file, out BY_HANDLE_FILE_INFORMATION info);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern bool GetFileInformationByHandleEx(
            SafeFileHandle file, Int32 infoClass, out FILE_ATTRIBUTE_TAG_INFO info,
            UInt32 size);

        [DllImport(
            "kernel32.dll", EntryPoint = "GetFileInformationByHandleEx",
            SetLastError = true)]
        private static extern bool GetFileIdInformationByHandleEx(
            SafeFileHandle file, Int32 infoClass, out FILE_ID_INFO info,
            UInt32 size);

        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        private static extern UInt32 GetFinalPathNameByHandleW(
            SafeFileHandle file, StringBuilder path, UInt32 pathLength, UInt32 flags);

        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        private static extern bool MoveFileExW(
            string existingName, string replacementName, UInt32 flags);

        public static WindowsPathIdentity Inspect(string path, bool openReparsePoint) {
            UInt32 flags = FILE_FLAG_BACKUP_SEMANTICS;
            if (openReparsePoint) flags |= FILE_FLAG_OPEN_REPARSE_POINT;
            using (SafeFileHandle handle = CreateFileW(
                path, 0, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                IntPtr.Zero, OPEN_EXISTING, flags, IntPtr.Zero)) {
                if (handle.IsInvalid) throw new Win32Exception(
                    Marshal.GetLastWin32Error(), "CreateFileW metadata handle failed");

                BY_HANDLE_FILE_INFORMATION basic;
                if (!GetFileInformationByHandle(handle, out basic)) throw new Win32Exception(
                    Marshal.GetLastWin32Error(), "GetFileInformationByHandle failed");

                FILE_ATTRIBUTE_TAG_INFO tag;
                if (!GetFileInformationByHandleEx(
                    handle, FileAttributeTagInfo, out tag,
                    (UInt32)Marshal.SizeOf(typeof(FILE_ATTRIBUTE_TAG_INFO)))) {
                    throw new Win32Exception(
                        Marshal.GetLastWin32Error(), "FileAttributeTagInfo failed");
                }

                FILE_ID_INFO fileId;
                if (!GetFileIdInformationByHandleEx(
                    handle, FileIdInfo, out fileId,
                    (UInt32)Marshal.SizeOf(typeof(FILE_ID_INFO)))) {
                    throw new Win32Exception(
                        Marshal.GetLastWin32Error(), "FileIdInfo failed");
                }
                byte[] identifier = new byte[16];
                Array.Copy(BitConverter.GetBytes(fileId.FileIdLow), 0, identifier, 0, 8);
                Array.Copy(BitConverter.GetBytes(fileId.FileIdHigh), 0, identifier, 8, 8);
                bool allZero = true;
                bool allOnes = true;
                foreach (byte value in identifier) {
                    allZero &= value == 0;
                    allOnes &= value == 0xff;
                }
                if (allZero || allOnes) {
                    throw new InvalidOperationException(
                        "FileIdInfo returned an unavailable 128-bit identifier sentinel");
                }

                StringBuilder finalPath = new StringBuilder(32768);
                UInt32 count = GetFinalPathNameByHandleW(
                    handle, finalPath, (UInt32)finalPath.Capacity, 0);
                if (count == 0 || count >= finalPath.Capacity) throw new Win32Exception(
                    Marshal.GetLastWin32Error(), "GetFinalPathNameByHandleW failed");

                return new WindowsPathIdentity {
                    FinalPath = finalPath.ToString(),
                    Attributes = tag.FileAttributes,
                    ReparseTag = tag.ReparseTag,
                    VolumeSerialNumber = fileId.VolumeSerialNumber,
                    FileId128 = BitConverter.ToString(identifier)
                        .Replace("-", "").ToLowerInvariant(),
                    FileIndex = ((UInt64)basic.FileIndexHigh << 32) | basic.FileIndexLow
                };
            }
        }

        public static void PublishNewWriteThrough(string source, string destination) {
            if (!MoveFileExW(source, destination, MOVEFILE_WRITE_THROUGH)) {
                throw new Win32Exception(
                    Marshal.GetLastWin32Error(), "MoveFileExW write-through publication failed");
            }
        }
    }
}
'@
}

function ConvertFrom-WindowsExtendedPath {
    param([Parameter(Mandatory = $true)][string]$Path)
    if ($Path.StartsWith('\\?\UNC\', [System.StringComparison]::OrdinalIgnoreCase)) {
        return '\\' + $Path.Substring(8)
    }
    if ($Path.StartsWith('\\?\', [System.StringComparison]::OrdinalIgnoreCase)) {
        return $Path.Substring(4)
    }
    return $Path
}

function Get-WindowsPathIdentity {
    param(
        [Parameter(Mandatory = $true)][string]$LiteralPath,
        [Parameter()][switch]$OpenReparsePoint
    )
    $full = [System.IO.Path]::GetFullPath($LiteralPath)
    $identity = [Hoi4MpStats.WindowsPathNative]::Inspect($full, $OpenReparsePoint.IsPresent)
    [pscustomobject]@{
        final_path = [System.IO.Path]::GetFullPath((ConvertFrom-WindowsExtendedPath $identity.FinalPath))
        attributes = [uint32]$identity.Attributes
        reparse_tag = [uint32]$identity.ReparseTag
        volume_serial_number = ('{0:x16}' -f [uint64]$identity.VolumeSerialNumber)
        file_id_128 = [string]$identity.FileId128
        file_index = ('{0:x16}' -f [uint64]$identity.FileIndex)
    }
}

function Resolve-WindowsPathThroughExistingAncestor {
    param([Parameter(Mandatory = $true)][string]$LiteralPath)
    $full = [System.IO.Path]::GetFullPath($LiteralPath).TrimEnd('\')
    $missing = [System.Collections.Generic.Stack[string]]::new()
    $cursor = $full
    while (-not (Test-Path -LiteralPath $cursor)) {
        $leaf = Split-Path -Leaf $cursor
        if ([string]::IsNullOrWhiteSpace($leaf)) {
            throw "No existing ancestor can anchor path identity: $LiteralPath"
        }
        $missing.Push($leaf)
        $parent = Split-Path -Parent $cursor
        if ([string]::IsNullOrWhiteSpace($parent) -or $parent -ceq $cursor) {
            throw "No existing ancestor can anchor path identity: $LiteralPath"
        }
        $cursor = $parent
    }
    $resolved = (Get-WindowsPathIdentity -LiteralPath $cursor).final_path
    while ($missing.Count -gt 0) {
        $resolved = Join-Path $resolved $missing.Pop()
    }
    [System.IO.Path]::GetFullPath($resolved)
}

function Test-WindowsPathContainedByRoot {
    param(
        [Parameter(Mandatory = $true)][string]$Candidate,
        [Parameter(Mandatory = $true)][string]$Root
    )
    $candidateFull = [System.IO.Path]::GetFullPath($Candidate).TrimEnd('\')
    $rootFull = [System.IO.Path]::GetFullPath($Root).TrimEnd('\')
    $candidateFull.Equals($rootFull, [System.StringComparison]::OrdinalIgnoreCase) -or
        $candidateFull.StartsWith($rootFull + '\', [System.StringComparison]::OrdinalIgnoreCase)
}

function Get-OneDriveAccountRoots {
    $candidates = [System.Collections.Generic.List[object]]::new()
    $accountsKey = 'HKCU:\Software\Microsoft\OneDrive\Accounts'
    if (Test-Path -LiteralPath $accountsKey) {
        foreach ($account in Get-ChildItem -LiteralPath $accountsKey -ErrorAction Stop) {
            $properties = Get-ItemProperty -LiteralPath $account.PSPath -Name UserFolder -ErrorAction SilentlyContinue
            $folder = if ($null -ne $properties) { $properties.UserFolder } else { $null }
            if (-not [string]::IsNullOrWhiteSpace([string]$folder)) {
                $candidates.Add([pscustomobject]@{
                    account_label = "registry:$($account.PSChildName)"
                    path = [string]$folder
                })
            }
        }
    }
    foreach ($name in @('OneDrive', 'OneDriveConsumer', 'OneDriveCommercial')) {
        $value = [Environment]::GetEnvironmentVariable($name)
        if (-not [string]::IsNullOrWhiteSpace($value)) {
            $candidates.Add([pscustomobject]@{ account_label = "environment:$name"; path = $value })
        }
    }

    $seen = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::OrdinalIgnoreCase)
    foreach ($candidate in $candidates) {
        if (-not (Test-Path -LiteralPath $candidate.path -PathType Container)) { continue }
        $identity = Get-WindowsPathIdentity -LiteralPath $candidate.path
        if ($seen.Add($identity.final_path)) {
            [pscustomobject]@{
                account_label = $candidate.account_label
                final_path = $identity.final_path
                volume_serial_number = $identity.volume_serial_number
                file_id_128 = $identity.file_id_128
                file_index = $identity.file_index
            }
        }
    }
}

function Test-CloudFilesReparseTag {
    param([Parameter(Mandatory = $true)][uint32]$Tag)
    $mask = [Convert]::ToUInt32('ffff0fff', 16)
    $cloud = [Convert]::ToUInt32('9000001a', 16)
    (($Tag -band $mask) -eq $cloud)
}

function Resolve-WindowsDurableNewFilePath {
    param([Parameter(Mandatory = $true)][string]$LiteralPath)

    $full = [System.IO.Path]::GetFullPath($LiteralPath)
    $leaf = Split-Path -Leaf $full
    if ([string]::IsNullOrWhiteSpace($leaf) -or
        $leaf.IndexOfAny([System.IO.Path]::GetInvalidFileNameChars()) -ge 0) {
        throw "Durable output path has no valid leaf file name: $LiteralPath"
    }
    $parent = Split-Path -Parent $full
    if ([string]::IsNullOrWhiteSpace($parent)) {
        throw "Durable output path has no parent: $LiteralPath"
    }

    # Resolve before creation so a missing path below a junction is anchored to
    # the junction target. Resolve again after creation and publish exclusively
    # through that final path; later alias replacement cannot redirect writes.
    $expectedFinalParent = Resolve-WindowsPathThroughExistingAncestor -LiteralPath $parent
    [System.IO.Directory]::CreateDirectory($parent) | Out-Null
    $parentIdentity = Get-WindowsPathIdentity -LiteralPath $parent
    if (-not $parentIdentity.final_path.Equals(
        [System.IO.Path]::GetFullPath($expectedFinalParent).TrimEnd('\'),
        [System.StringComparison]::OrdinalIgnoreCase
    )) {
        throw 'Durable output parent identity changed while it was being created.'
    }
    $anchored = Join-Path $parentIdentity.final_path $leaf
    if (Test-Path -LiteralPath $anchored) {
        throw "Durable output already exists and will not be overwritten: $anchored"
    }
    [pscustomobject]@{
        anchored_path = $anchored
        parent_final_path = $parentIdentity.final_path
        parent_volume_serial_number = $parentIdentity.volume_serial_number
        parent_file_id_128 = $parentIdentity.file_id_128
    }
}

function Publish-WindowsDurableNewFileBytes {
    param(
        [Parameter(Mandatory = $true)][string]$LiteralPath,
        [Parameter(Mandatory = $true)][byte[]]$Bytes
    )

    $anchor = Resolve-WindowsDurableNewFilePath -LiteralPath $LiteralPath
    $destination = [string]$anchor.anchored_path
    $parent = [string]$anchor.parent_final_path
    $leaf = Split-Path -Leaf $destination
    $temporary = Join-Path $parent (
        ".{0}.tmp-{1}-{2}" -f $leaf, $PID, [guid]::NewGuid().ToString('N')
    )
    $stream = [System.IO.File]::Open(
        $temporary, [System.IO.FileMode]::CreateNew,
        [System.IO.FileAccess]::ReadWrite, [System.IO.FileShare]::None
    )
    try {
        $stream.Write($Bytes, 0, $Bytes.Length)
        $stream.Flush($true)
        $stream.Position = 0
        $persisted = New-Object byte[] $Bytes.Length
        $offset = 0
        while ($offset -lt $persisted.Length) {
            $count = $stream.Read($persisted, $offset, $persisted.Length - $offset)
            if ($count -eq 0) { throw 'Durable output temp readback ended early.' }
            $offset += $count
        }
        $structuralComparer = [System.Collections.StructuralComparisons]::StructuralEqualityComparer
        if (-not $structuralComparer.Equals([byte[]]$Bytes, [byte[]]$persisted)) {
            throw 'Durable output temp readback differs from supplied bytes.'
        }
    }
    catch {
        $stream.Dispose()
        Remove-Item -LiteralPath $temporary -Force -ErrorAction SilentlyContinue
        throw
    }
    finally {
        $stream.Dispose()
    }

    try {
        [Hoi4MpStats.WindowsPathNative]::PublishNewWriteThrough(
            $temporary, $destination
        )
    }
    catch {
        Remove-Item -LiteralPath $temporary -Force -ErrorAction SilentlyContinue
        throw
    }
    $published = [System.IO.File]::ReadAllBytes($destination)
    $structuralComparer = [System.Collections.StructuralComparisons]::StructuralEqualityComparer
    if (-not $structuralComparer.Equals([byte[]]$Bytes, [byte[]]$published)) {
        throw 'Published durable output differs from verified temp bytes.'
    }
    $publishedIdentity = Get-WindowsPathIdentity -LiteralPath $destination
    if ($publishedIdentity.volume_serial_number -cne $anchor.parent_volume_serial_number) {
        throw 'Published durable output escaped the anchored output volume.'
    }
    [pscustomobject]@{
        anchored_path = $destination
        volume_serial_number = $publishedIdentity.volume_serial_number
        file_id_128 = $publishedIdentity.file_id_128
    }
}
