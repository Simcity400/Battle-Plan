[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$RepoRoot,
    [Parameter(Mandatory = $true)][string]$ReceiptPath,
    [switch]$AllowStagedReceipt
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$repo = [IO.Path]::GetFullPath($RepoRoot)
$receiptFull = [IO.Path]::GetFullPath($ReceiptPath)
$attestationHelper = Join-Path $repo 'extractor\native\scripts\ProductionArtifact.Attestation.ps1'
$builder = Join-Path $repo 'extractor\native\scripts\New-PortableProductPackage.ps1'
$driver = Join-Path $repo 'extractor\native\scripts\Start-Hoi4ProductionCaptureSafe.ps1'
$expectedReceipt = Join-Path $repo '.cargo-target\release\hoi4-portable-package-receipt.json'
$expectedAttestation = Join-Path $repo '.cargo-target\release\hoi4-production-prebuilt-artifacts.json'
$expectedReceiptFull = [IO.Path]::GetFullPath($expectedReceipt)
if ($AllowStagedReceipt) {
    $expectedParent = Split-Path -Parent $expectedReceiptFull
    $candidateName = [IO.Path]::GetFileName($receiptFull)
    if ((Split-Path -Parent $receiptFull) -ine $expectedParent -or
        $candidateName -cnotmatch '^hoi4-portable-package-receipt\.json\.[0-9a-f]{32}\.candidate$') {
        throw 'staged portable package receipt is outside the canonical release directory or has a noncanonical candidate name'
    }
}
elseif ($receiptFull -ine $expectedReceiptFull) {
    throw 'portable package receipt is outside the canonical release location'
}
. $attestationHelper

Add-Type -AssemblyName System.IO.Compression
Add-Type -TypeDefinition @'
using System;
using System.ComponentModel;
using System.IO;
using System.Runtime.InteropServices;
using Microsoft.Win32.SafeHandles;
public static class PortablePackageReceiptNative {
    private const uint GENERIC_READ = 0x80000000;
    private const uint FILE_SHARE_READ = 0x1;
    private const uint OPEN_EXISTING = 3;
    private const uint FILE_FLAG_OPEN_REPARSE_POINT = 0x00200000;
    private const uint FILE_ATTRIBUTE_DIRECTORY = 0x10;
    private const uint FILE_ATTRIBUTE_REPARSE_POINT = 0x400;
    [StructLayout(LayoutKind.Sequential)] public struct FILETIME { public uint Low; public uint High; }
    [StructLayout(LayoutKind.Sequential)] public struct INFO {
        public uint Attributes; public FILETIME Creation; public FILETIME Access; public FILETIME Write;
        public uint Volume; public uint SizeHigh; public uint SizeLow; public uint Links; public uint IndexHigh; public uint IndexLow;
    }
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
    private static extern SafeFileHandle CreateFileW(string name, uint access, uint share, IntPtr security, uint creation, uint flags, IntPtr template);
    [DllImport("kernel32.dll", SetLastError=true)] private static extern bool GetFileInformationByHandle(SafeFileHandle handle, out INFO info);
    public static SafeFileHandle Open(string path) {
        var handle = CreateFileW(path, GENERIC_READ, FILE_SHARE_READ, IntPtr.Zero, OPEN_EXISTING, FILE_FLAG_OPEN_REPARSE_POINT, IntPtr.Zero);
        if (handle.IsInvalid) throw new Win32Exception(Marshal.GetLastWin32Error(), "open portable package authority failed: " + path);
        INFO info;
        if (!GetFileInformationByHandle(handle, out info)) { int e=Marshal.GetLastWin32Error(); handle.Dispose(); throw new Win32Exception(e); }
        if ((info.Attributes & (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT)) != 0) { handle.Dispose(); throw new InvalidDataException("portable package authority is a directory or reparse point: " + path); }
        return handle;
    }
}
'@

function Read-LockedBytes([string]$Path) {
    $handle = [PortablePackageReceiptNative]::Open($Path)
    $stream = [IO.FileStream]::new($handle, [IO.FileAccess]::Read)
    try {
        if ($stream.Length -le 0 -or $stream.Length -gt [int]::MaxValue) { throw "invalid authority file length: $Path" }
        $bytes = [byte[]]::new([int]$stream.Length)
        $offset = 0
        while ($offset -lt $bytes.Length) {
            $read = $stream.Read($bytes, $offset, $bytes.Length - $offset)
            if ($read -eq 0) { throw "authority file truncated during locked read: $Path" }
            $offset += $read
        }
        return ,$bytes
    }
    finally { $stream.Dispose() }
}

function Get-ByteHash([byte[]]$Bytes) {
    $sha = [Security.Cryptography.SHA256]::Create()
    try { ([BitConverter]::ToString($sha.ComputeHash($Bytes))).Replace('-', '').ToLowerInvariant() }
    finally { $sha.Dispose() }
}

$receiptBytes = Read-LockedBytes $receiptFull
$receipt = [Text.Encoding]::UTF8.GetString($receiptBytes) | ConvertFrom-Json
if ([string]$receipt.schema -cne 'battle-plan.central_portable_package.v1' -or
    [string]$receipt.status -cne 'PASS_CENTRAL_RELEASE_PORTABLE_PACKAGE' -or
    $receipt.successful_exit -isnot [bool] -or -not $receipt.successful_exit -or
    [string]$receipt.central_validation.mode -cne 'Release' -or
    [string]$receipt.central_validation.requested_workspace -cne 'All') {
    throw 'portable package receipt has no full centralized Release authority'
}
if ([string]$receipt.central_validation.release_runtime_linkage.target -cne 'x86_64-pc-windows-msvc' -or
    [string]$receipt.central_validation.release_runtime_linkage.rust_target_feature -cne '+crt-static' -or
    [string]$receipt.central_validation.release_runtime_linkage.c_runtime -cne 'static' -or
    [string]$receipt.central_validation.release_runtime_linkage.admission -cne 'pe_import_and_delay_import_tables' -or
    [string]$receipt.central_validation.release_runtime_linkage.forbidden_import -cne 'VCRUNTIME*.dll') {
    throw 'portable package receipt has no static MSVC CRT release contract'
}
if ([IO.Path]::GetFullPath([string]$receipt.package_builder.path) -ine [IO.Path]::GetFullPath($builder) -or
    [string]$receipt.package_builder.sha256 -cne (Get-FileHash -LiteralPath $builder -Algorithm SHA256).Hash.ToLowerInvariant()) {
    throw 'portable package builder differs from its receipt'
}
if ([IO.Path]::GetFullPath([string]$receipt.artifact_attestation.path) -ine [IO.Path]::GetFullPath($expectedAttestation)) {
    throw 'portable package receipt does not bind the canonical production artifact attestation'
}
$attested = Test-ProductionArtifactAttestation -RepoRoot $repo -ReceiptPath ([string]$receipt.artifact_attestation.path) -ExpectedDriverPath $driver
Test-ProductionBuildEnvironmentRecord -Record $receipt.build_environment
if ([string]$receipt.artifact_attestation.sha256 -cne [string]$attested.receipt_sha256 -or
    [string]$receipt.source_input_sha256 -cne [string]$attested.source_input_sha256 -or
    [string]$receipt.build_environment.sha256 -cne [string]$attested.build_environment.sha256) {
    throw 'portable package receipt is stale against its production artifact attestation'
}
$version = [string]$receipt.package.version
if ($version -notmatch '^[0-9A-Za-z][0-9A-Za-z._-]{0,63}$') { throw 'portable package version is invalid' }
$expectedZip = Join-Path $repo ('.cargo-target\release\battle-plan-{0}-windows-x86_64.zip' -f $version)
if ([string]$receipt.package.schema -cne 'hoi4-portable-package.v1' -or
    [IO.Path]::GetFullPath([string]$receipt.package.path) -ine [IO.Path]::GetFullPath($expectedZip) -or
    [int]$receipt.package.executable_count -ne 2 -or
    $receipt.package.static_vcruntime_admission -isnot [bool] -or
    -not $receipt.package.static_vcruntime_admission -or
    $receipt.package.signed -isnot [bool] -or $receipt.package.signed -or
    [string]$receipt.package.install_kind -cne 'portable_zip') {
    throw 'portable package identity differs from its central receipt'
}
$zipBytes = Read-LockedBytes $expectedZip
if ((Get-ByteHash $zipBytes) -cne [string]$receipt.package.sha256) { throw 'portable package bytes differ from its receipt' }
$memory = [IO.MemoryStream]::new($zipBytes, $false)
try {
    $archive = [IO.Compression.ZipArchive]::new($memory, [IO.Compression.ZipArchiveMode]::Read, $false)
    try {
        $expectedNames = @('Battle Plan Recorder.exe', 'Battle Plan.exe', 'collector.dll', 'README.txt', 'SHA256SUMS.txt')
        if ($archive.Entries.Count -ne 5) { throw 'portable package does not contain exactly five entries' }
        foreach ($name in $expectedNames) {
            if (@($archive.Entries | Where-Object FullName -CEQ $name).Count -ne 1) { throw "portable package entry is missing or duplicated: $name" }
        }
        $roles = [ordered]@{
            'Battle Plan Recorder.exe' = 'recorder_host'
            'Battle Plan.exe' = 'replay_app'
            'collector.dll' = 'collector_dll'
        }
        foreach ($pair in $roles.GetEnumerator()) {
            $entry = @($archive.Entries | Where-Object FullName -CEQ $pair.Key)[0]
            $entryStream = $entry.Open()
            $entryMemory = $null
            try { $entryMemory = [IO.MemoryStream]::new(); $entryStream.CopyTo($entryMemory); $entryBytes = $entryMemory.ToArray() }
            finally { if ($null -ne $entryMemory) { $entryMemory.Dispose() }; $entryStream.Dispose() }
            $hash = Get-ByteHash $entryBytes
            if ($hash -cne [string]$attested.artifact_sha256[$pair.Value] -or
                $hash -cne [string]$receipt.package.input_sha256.($pair.Value)) {
                throw "portable package entry is not bound to its attested artifact: $($pair.Key)"
            }
            [void](Assert-PortableProductPeImportAttestation `
                -Bytes $entryBytes `
                -Name ([string]$pair.Key) `
                -Attestation $receipt.package.pe_imports.($pair.Value))
            [void](Assert-PortableProductPeImportAttestation `
                -Bytes $entryBytes `
                -Name ([string]$pair.Key) `
                -Attestation $attested.artifact_pe_imports[$pair.Value])
        }
    }
    finally { $archive.Dispose() }
}
finally { $memory.Dispose() }

[pscustomobject]@{
    schema = 'battle-plan.central_portable_package_verification.v1'
    status = if ($AllowStagedReceipt) { 'PASS_STAGED_CENTRAL_RELEASE_PORTABLE_PACKAGE' } else { 'PASS_CENTRAL_RELEASE_PORTABLE_PACKAGE' }
    canonical_receipt = -not [bool]$AllowStagedReceipt
    receipt_sha256 = Get-ByteHash $receiptBytes
    package_sha256 = [string]$receipt.package.sha256
    build_environment_sha256 = [string]$receipt.build_environment.sha256
    static_vcruntime_admission = $true
} | ConvertTo-Json -Depth 3
