[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$RecorderExe,

    [Parameter(Mandatory = $true)]
    [string]$ReplayExe,

    [Parameter(Mandatory = $true)]
    [string]$CollectorDll,

    [Parameter(Mandatory = $true)]
    [string]$OutputZip,

    [Parameter(Mandatory = $true)]
    [ValidatePattern('^[0-9A-Za-z][0-9A-Za-z._-]{0,63}$')]
    [string]$Version,

    # Deterministic fail-closed injection for the central source self-test. It
    # can only turn a valid build into a failure and exists to prove that the
    # post-creation finally block removes an unpublished temporary archive.
    [switch]$TestOnlyFailAfterTemporaryWrite
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
. (Join-Path $PSScriptRoot 'PortableProduct.PeImports.ps1')
Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem
Add-Type -TypeDefinition @'
using System;
using System.ComponentModel;
using System.IO;
using System.Runtime.InteropServices;
using Microsoft.Win32.SafeHandles;

public static class PortablePackageNative {
    public const uint FILE_ATTRIBUTE_DIRECTORY = 0x10;
    public const uint FILE_ATTRIBUTE_REPARSE_POINT = 0x400;
    private const uint GENERIC_READ = 0x80000000;
    private const uint FILE_SHARE_READ = 0x1;
    private const uint OPEN_EXISTING = 3;
    private const uint FILE_FLAG_OPEN_REPARSE_POINT = 0x00200000;
    private const uint FILE_FLAG_SEQUENTIAL_SCAN = 0x08000000;
    private const uint MOVEFILE_WRITE_THROUGH = 0x00000008;

    [StructLayout(LayoutKind.Sequential)]
    public struct FILETIME { public uint Low; public uint High; }

    [StructLayout(LayoutKind.Sequential)]
    public struct BY_HANDLE_FILE_INFORMATION {
        public uint FileAttributes;
        public FILETIME CreationTime;
        public FILETIME LastAccessTime;
        public FILETIME LastWriteTime;
        public uint VolumeSerialNumber;
        public uint FileSizeHigh;
        public uint FileSizeLow;
        public uint NumberOfLinks;
        public uint FileIndexHigh;
        public uint FileIndexLow;
    }

    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern SafeFileHandle CreateFileW(
        string name, uint access, uint share, IntPtr security, uint creation,
        uint flags, IntPtr template);

    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern bool GetFileInformationByHandle(
        SafeFileHandle handle, out BY_HANDLE_FILE_INFORMATION information);

    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern bool MoveFileExW(string existingName, string newName, uint flags);

    public static void PublishNoReplaceWriteThrough(string temporary, string destination) {
        if (File.Exists(destination) || Directory.Exists(destination)) {
            throw new IOException("durable publication refuses an existing destination: " + destination);
        }
        // Omitting MOVEFILE_REPLACE_EXISTING preserves atomic no-replace
        // publication. WRITE_THROUGH makes the namespace/parent-directory
        // update durable before success is returned.
        if (!MoveFileExW(temporary, destination, MOVEFILE_WRITE_THROUGH)) {
            throw new Win32Exception(Marshal.GetLastWin32Error(),
                "durable no-replace package publication failed: " + destination);
        }
    }

    public static SafeFileHandle OpenReadNoFollow(
        string path, out BY_HANDLE_FILE_INFORMATION information) {
        SafeFileHandle handle = CreateFileW(
            path, GENERIC_READ, FILE_SHARE_READ, IntPtr.Zero, OPEN_EXISTING,
            FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_SEQUENTIAL_SCAN, IntPtr.Zero);
        if (handle.IsInvalid) {
            throw new Win32Exception(Marshal.GetLastWin32Error(),
                "open package artifact without reparse traversal failed: " + path);
        }
        if (!GetFileInformationByHandle(handle, out information)) {
            int error = Marshal.GetLastWin32Error();
            handle.Dispose();
            throw new Win32Exception(error, "inspect opened package artifact failed: " + path);
        }
        if ((information.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT) != 0 ||
            (information.FileAttributes & FILE_ATTRIBUTE_DIRECTORY) != 0) {
            handle.Dispose();
            throw new InvalidDataException("package artifact is a directory or reparse point: " + path);
        }
        return handle;
    }

    public static bool BytesEqual(byte[] left, byte[] right) {
        if (Object.ReferenceEquals(left, right)) return true;
        if (left == null || right == null || left.Length != right.Length) return false;
        int difference = 0;
        for (int i = 0; i < left.Length; ++i) difference |= left[i] ^ right[i];
        return difference == 0;
    }
}
'@

function Resolve-ArtifactPath {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$Label
    )

    $full = [IO.Path]::GetFullPath($Path)
    if ([IO.Path]::GetFileName($full)) {
        return $full
    }
    throw "$Label must name a file: $Path"
}

function Assert-PeKind {
    param(
        [Parameter(Mandatory = $true)][byte[]]$Bytes,
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][bool]$ExpectDll,
        [Parameter(Mandatory = $true)][bool]$ExpectGui
    )

    $stream = [IO.MemoryStream]::new($Bytes, $false)
    try {
        $reader = [IO.BinaryReader]::new($stream)
        if ($reader.ReadUInt16() -ne 0x5A4D) {
            throw "$Name is not a PE image"
        }
        $stream.Position = 0x3c
        $peOffset = $reader.ReadUInt32()
        if ($peOffset -gt ($stream.Length - 24)) {
            throw "$Name has an invalid PE header offset"
        }
        $stream.Position = $peOffset
        if ($reader.ReadUInt32() -ne 0x00004550) {
            throw "$Name has no PE signature"
        }
        $machine = $reader.ReadUInt16()
        if ($machine -ne 0x8664) {
            throw "$Name is not an AMD64 PE image"
        }
        [void]$reader.ReadUInt16()
        [void]$reader.ReadUInt32()
        [void]$reader.ReadUInt32()
        [void]$reader.ReadUInt32()
        $optionalHeaderSize = $reader.ReadUInt16()
        $characteristics = $reader.ReadUInt16()
        if (($characteristics -band 0x0002) -eq 0) {
            throw "$Name is not marked as an executable image"
        }
        $isDll = ($characteristics -band 0x2000) -ne 0
        if ($isDll -ne $ExpectDll) {
            $wanted = if ($ExpectDll) { 'DLL' } else { 'executable' }
            throw "$Name is not a PE $wanted"
        }
        $optionalHeaderOffset = [uint64]$peOffset + 24
        if ($optionalHeaderSize -lt 2 -or
            $optionalHeaderOffset + $optionalHeaderSize -gt [uint64]$stream.Length) {
            throw "$Name has a truncated PE optional header"
        }
        $stream.Position = [int64]$optionalHeaderOffset
        if ($reader.ReadUInt16() -ne 0x020B) {
            throw "$Name is not a PE32+ image"
        }
        if ($ExpectGui) {
            if ($optionalHeaderSize -lt 70) {
                throw "$Name has no complete PE optional header"
            }
            $subsystemOffset = $optionalHeaderOffset + 68
            $stream.Position = [int64]$subsystemOffset
            if ($reader.ReadUInt16() -ne 2) {
                throw "$Name is not linked as a Windows GUI-subsystem executable"
            }
        }
    }
    finally {
        $stream.Dispose()
    }
}

function Read-LockedBytes {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$Label
    )

    # Open the pathname itself (not its target) and reject a reparse handle.
    # Share-read only denies write/delete until the authoritative bytes and
    # handle identity/type have been retained.
    $information = [PortablePackageNative+BY_HANDLE_FILE_INFORMATION]::new()
    $handle = [PortablePackageNative]::OpenReadNoFollow($Path, [ref]$information)
    $stream = [IO.FileStream]::new($handle, [IO.FileAccess]::Read)
    try {
        if ($stream.Length -le 0) {
            throw "$Label is empty: $Path"
        }
        if ($stream.Length -gt [int]::MaxValue) {
            throw "$Label exceeds the supported portable-package artifact size"
        }
        $bytes = [byte[]]::new([int]$stream.Length)
        $offset = 0
        while ($offset -lt $bytes.Length) {
            $read = $stream.Read($bytes, $offset, $bytes.Length - $offset)
            if ($read -eq 0) {
                throw "$Label ended before its retained handle length"
            }
            $offset += $read
        }
        if ($stream.ReadByte() -ne -1) {
            throw "$Label grew during its locked read"
        }
        return ,$bytes
    }
    finally {
        $stream.Dispose()
    }
}

function Add-BytesEntry {
    param(
        [Parameter(Mandatory = $true)][IO.Compression.ZipArchive]$Archive,
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][byte[]]$Bytes
    )

    $entry = $Archive.CreateEntry($Name, [IO.Compression.CompressionLevel]::Optimal)
    $entry.LastWriteTime = [DateTimeOffset]::new(1980, 1, 1, 0, 0, 0, [TimeSpan]::Zero)
    $entryStream = $entry.Open()
    try {
        $entryStream.Write($Bytes, 0, $Bytes.Length)
    }
    finally {
        $entryStream.Dispose()
    }
}

function Get-BytesSha256 {
    param([Parameter(Mandatory = $true)][byte[]]$Bytes)

    $sha256 = [Security.Cryptography.SHA256]::Create()
    try {
        return ([BitConverter]::ToString($sha256.ComputeHash($Bytes))).Replace('-', '').ToLowerInvariant()
    }
    finally {
        $sha256.Dispose()
    }
}

function Read-ZipEntryBytes {
    param([Parameter(Mandatory = $true)][IO.Compression.ZipArchiveEntry]$Entry)

    if ($Entry.Length -gt [int]::MaxValue) {
        throw "ZIP entry exceeds supported verification size: $($Entry.FullName)"
    }
    $stream = $Entry.Open()
    try {
        $memory = [IO.MemoryStream]::new([int]$Entry.Length)
        try {
            $stream.CopyTo($memory)
            return ,$memory.ToArray()
        }
        finally {
            $memory.Dispose()
        }
    }
    finally {
        $stream.Dispose()
    }
}

function Assert-PackageZipBytes {
    param(
        [Parameter(Mandatory = $true)][byte[]]$ZipBytes,
        [Parameter(Mandatory = $true)]$ExpectedEntries,
        [Parameter(Mandatory = $true)]$ExpectedBinaryHashes,
        [Parameter(Mandatory = $true)]$ExpectedImportAttestations
    )

    $memory = [IO.MemoryStream]::new($ZipBytes, $false)
    try {
        $archive = [IO.Compression.ZipArchive]::new($memory, [IO.Compression.ZipArchiveMode]::Read, $false)
        try {
            $names = @($archive.Entries | ForEach-Object FullName)
            $exeCount = @($names | Where-Object { $_.EndsWith('.exe', [StringComparison]::OrdinalIgnoreCase) }).Count
            if ($exeCount -ne 2 -or $names.Count -ne 5 -or $names -contains '') {
                throw "portable package shape is invalid: entries=$($names.Count), executables=$exeCount"
            }
            foreach ($expected in @('Battle Plan Recorder.exe', 'Battle Plan.exe', 'collector.dll', 'README.txt', 'SHA256SUMS.txt')) {
                $matching = @($archive.Entries | Where-Object FullName -CEQ $expected)
                if ($matching.Count -ne 1) {
                    throw "portable package requires exactly one $expected entry"
                }
                $expectedBytes = [byte[]]$ExpectedEntries[$expected]
                $entry = $matching[0]
                if ($entry.Length -ne $expectedBytes.Length) {
                    throw "portable package entry length mismatch for $expected"
                }
                $actualBytes = Read-ZipEntryBytes -Entry $entry
                if (-not [PortablePackageNative]::BytesEqual($actualBytes, $expectedBytes)) {
                    throw "portable package entry bytes mismatch for $expected"
                }
                $actualHash = Get-BytesSha256 -Bytes $actualBytes
                $expectedHash = Get-BytesSha256 -Bytes $expectedBytes
                if ($actualHash -cne $expectedHash) {
                    throw "portable package entry hash mismatch for $expected"
                }
                if ($ExpectedBinaryHashes.Contains($expected) -and $actualHash -cne $ExpectedBinaryHashes[$expected]) {
                    throw "portable package entry disagrees with SHA256SUMS.txt for $expected"
                }
                if ($ExpectedImportAttestations.Contains($expected)) {
                    [void](Assert-PortableProductPeImportAttestation `
                        -Bytes $actualBytes `
                        -Name $expected `
                        -Attestation $ExpectedImportAttestations[$expected])
                }
            }
        }
        finally {
            $archive.Dispose()
        }
    }
    finally {
        $memory.Dispose()
    }
}

$recorder = Resolve-ArtifactPath -Path $RecorderExe -Label 'Recorder executable'
$replay = Resolve-ArtifactPath -Path $ReplayExe -Label 'Replay executable'
$collector = Resolve-ArtifactPath -Path $CollectorDll -Label 'Collector DLL'
$recorderBytes = Read-LockedBytes -Path $recorder -Label 'Recorder executable'
$replayBytes = Read-LockedBytes -Path $replay -Label 'Replay executable'
$collectorBytes = Read-LockedBytes -Path $collector -Label 'Collector DLL'
Assert-PeKind -Bytes $recorderBytes -Name ([IO.Path]::GetFileName($recorder)) -ExpectDll $false -ExpectGui $true
Assert-PeKind -Bytes $replayBytes -Name ([IO.Path]::GetFileName($replay)) -ExpectDll $false -ExpectGui $true
Assert-PeKind -Bytes $collectorBytes -Name ([IO.Path]::GetFileName($collector)) -ExpectDll $true -ExpectGui $false
$binaryImportAttestations = [ordered]@{
    'Battle Plan Recorder.exe' = Assert-PortableProductStaticCrt -Bytes $recorderBytes -Name 'Battle Plan Recorder.exe'
    'Battle Plan.exe' = Assert-PortableProductStaticCrt -Bytes $replayBytes -Name 'Battle Plan.exe'
    'collector.dll' = Assert-PortableProductStaticCrt -Bytes $collectorBytes -Name 'collector.dll'
}

$outputFull = [IO.Path]::GetFullPath($OutputZip)
$outputParent = Split-Path -Parent $outputFull
if (-not $outputParent) {
    throw 'OutputZip must have a parent directory'
}
[IO.Directory]::CreateDirectory($outputParent) | Out-Null
if (Test-Path -LiteralPath $outputFull) {
    throw "Output ZIP already exists; refusing overwrite: $outputFull"
}

$entries = [ordered]@{
    'Battle Plan Recorder.exe' = $recorderBytes
    'Battle Plan.exe' = $replayBytes
    'collector.dll' = $collectorBytes
}
$binaryHashes = [ordered]@{}
$hashLines = foreach ($name in $entries.Keys) {
    $hash = Get-BytesSha256 -Bytes $entries[$name]
    $binaryHashes[$name] = $hash
    "$hash  $name"
}
$readme = @"
Battle Plan $Version

This is a portable Windows package. It has no installer, service, tray process,
administrator requirement, automatic updater, or network dependency.

Run "Battle Plan Recorder.exe" once and leave its window open while you play.
It watches for supported the game sessions automatically. Use "Stop and
finish recordings" (or close the window) to stop cooperatively and verify any
active recording before exit. The window reports recording start, verified
completion, or failure and shows the exact output path. Use "Open recordings
folder" to reveal the current output directory.

Run "Battle Plan.exe" to open and view a replay. Its picker starts in the
recorder output root when that folder exists. On first use, select the Hearts of
Iron IV installation folder and user-data folder when prompted.

Recorder data is stored under the current user's LocalAppData in
"Battle Plan Recorder" with a protected user/SYSTEM ACL. SHA256SUMS.txt detects
accidental package corruption; it is not a publisher signature.
"@
$utf8 = [Text.UTF8Encoding]::new($false)
$readmeBytes = $utf8.GetBytes($readme.Replace("`r`n", "`n"))
$checksumBytes = $utf8.GetBytes((($hashLines -join "`n") + "`n"))
$packageEntries = [ordered]@{
    'Battle Plan Recorder.exe' = $recorderBytes
    'Battle Plan.exe' = $replayBytes
    'collector.dll' = $collectorBytes
    'README.txt' = $readmeBytes
    'SHA256SUMS.txt' = $checksumBytes
}

$temporary = Join-Path $outputParent ('.' + [IO.Path]::GetFileName($outputFull) + '.' + [Guid]::NewGuid().ToString('N') + '.tmp')
try {
    $file = [IO.File]::Open(
        $temporary,
        [IO.FileMode]::CreateNew,
        [IO.FileAccess]::ReadWrite,
        [IO.FileShare]::None
    )
    try {
        $archive = [IO.Compression.ZipArchive]::new($file, [IO.Compression.ZipArchiveMode]::Create, $true)
        try {
            foreach ($name in $packageEntries.Keys) {
                Add-BytesEntry -Archive $archive -Name $name -Bytes $packageEntries[$name]
            }
        }
        finally {
            $archive.Dispose()
        }
        $file.Flush($true)
    }
    finally {
        $file.Dispose()
    }

    if ($TestOnlyFailAfterTemporaryWrite) {
        throw 'test-only failure after temporary portable package write'
    }

    $temporaryBytes = Read-LockedBytes -Path $temporary -Label 'Completed temporary product ZIP'
    Assert-PackageZipBytes `
        -ZipBytes $temporaryBytes `
        -ExpectedEntries $packageEntries `
        -ExpectedBinaryHashes $binaryHashes `
        -ExpectedImportAttestations $binaryImportAttestations
    [PortablePackageNative]::PublishNoReplaceWriteThrough($temporary, $outputFull)
}
finally {
    if (Test-Path -LiteralPath $temporary) {
        Remove-Item -LiteralPath $temporary -Force
    }
}

$zipBytes = Read-LockedBytes -Path $outputFull -Label 'Completed product ZIP'
Assert-PackageZipBytes `
    -ZipBytes $zipBytes `
    -ExpectedEntries $packageEntries `
    -ExpectedBinaryHashes $binaryHashes `
    -ExpectedImportAttestations $binaryImportAttestations
$zipHash = Get-BytesSha256 -Bytes $zipBytes
[pscustomobject]@{
    schema = 'hoi4-portable-package.v1'
    version = $Version
    path = $outputFull
    sha256 = $zipHash
    input_sha256 = [ordered]@{
        recorder_host = $binaryHashes['Battle Plan Recorder.exe']
        replay_app = $binaryHashes['Battle Plan.exe']
        collector_dll = $binaryHashes['collector.dll']
    }
    pe_imports = [ordered]@{
        recorder_host = $binaryImportAttestations['Battle Plan Recorder.exe']
        replay_app = $binaryImportAttestations['Battle Plan.exe']
        collector_dll = $binaryImportAttestations['collector.dll']
    }
    static_vcruntime_admission = $true
    executable_count = 2
    signed = $false
    install_kind = 'portable_zip'
} | ConvertTo-Json -Depth 8
