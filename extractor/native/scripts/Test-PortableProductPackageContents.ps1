[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
Add-Type -AssemblyName System.IO.Compression
. (Join-Path $PSScriptRoot 'PortableProduct.PackageContents.ps1')

$hashes = [ordered]@{
    'Battle Plan Recorder.exe' = '1' * 64
    'Battle Plan.exe' = '2' * 64
    'collector.dll' = '3' * 64
}
$version = '1.2.3-test'
$documents = Get-PortableProductPackageDocuments -Version $version -BinaryHashes $hashes
$utf8 = [Text.UTF8Encoding]::new($false)
$passed = [System.Collections.Generic.List[string]]::new()

function Invoke-PackageContentsCase {
    param([string]$Name, [scriptblock]$Mutate, [string]$ExpectedFailure = '', [switch]$ForgeReadmeLength)

    $entries = [System.Collections.Generic.List[object]]::new()
    foreach ($binary in $hashes.Keys) {
        # Only the document/roster boundary is under test; the owning verifier
        # separately binds real binary bytes to PE and artifact attestations.
        $entries.Add([pscustomobject]@{ Name = $binary; Bytes = [byte[]]@(1, 2, 3) })
    }
    foreach ($document in $documents.Keys) {
        $entries.Add([pscustomobject]@{ Name = $document; Bytes = [byte[]]$documents[$document].Clone() })
    }
    if ($null -ne $Mutate) { & $Mutate $entries }
    $memory = [IO.MemoryStream]::new()
    try {
        $writer = [IO.Compression.ZipArchive]::new($memory, [IO.Compression.ZipArchiveMode]::Create, $true)
        try {
            foreach ($item in $entries) {
                $entry = $writer.CreateEntry($item.Name)
                $stream = $entry.Open()
                try { $stream.Write($item.Bytes, 0, $item.Bytes.Length) }
                finally { $stream.Dispose() }
            }
        }
        finally { $writer.Dispose() }
        if ($ForgeReadmeLength) {
            $zipBytes = $memory.ToArray()
            $end = $zipBytes.Length - 22
            if ([BitConverter]::ToUInt32($zipBytes, $end) -ne 0x06054b50) { throw 'fixture has no canonical ZIP end record' }
            $offset = [int][BitConverter]::ToUInt32($zipBytes, $end + 16)
            $patched = $false
            for ($index = 0; $index -lt $entries.Count; $index++) {
                if ([BitConverter]::ToUInt32($zipBytes, $offset) -ne 0x02014b50) { throw 'fixture central directory is malformed' }
                $nameLength = [BitConverter]::ToUInt16($zipBytes, $offset + 28)
                $entryName = $utf8.GetString($zipBytes, $offset + 46, $nameLength)
                if ($entryName -ceq 'README.txt') {
                    [BitConverter]::GetBytes([uint32]$documents['README.txt'].Length).CopyTo($zipBytes, $offset + 24)
                    $patched = $true
                }
                $offset += 46 + $nameLength + [BitConverter]::ToUInt16($zipBytes, $offset + 30) + [BitConverter]::ToUInt16($zipBytes, $offset + 32)
            }
            if (-not $patched) { throw 'fixture did not patch the README length' }
            $memory.Position = 0
            $memory.Write($zipBytes, 0, $zipBytes.Length)
        }
        $memory.Position = 0
        $archive = [IO.Compression.ZipArchive]::new($memory, [IO.Compression.ZipArchiveMode]::Read, $true)
        $failure = ''
        try {
            try { Assert-PortableProductPackageContents -Archive $archive -Version $version -BinaryHashes $hashes }
            catch { $failure = $_.Exception.Message }
        }
        finally { $archive.Dispose() }
        if ($ExpectedFailure) {
            if (-not $failure.Contains($ExpectedFailure)) {
                throw "$Name did not reject at the expected boundary '$ExpectedFailure': $failure"
            }
        }
        elseif ($failure) { throw "$Name failed: $failure" }
        $passed.Add($Name)
    }
    finally { $memory.Dispose() }
}

Invoke-PackageContentsCase 'canonical five-entry package'
Invoke-PackageContentsCase 'archive order is immaterial' { param($e) $e.Reverse() }
Invoke-PackageContentsCase 'development corpus is excluded' {
    param($e) $e.Add([pscustomobject]@{ Name = 'corpus/data.bin'; Bytes = [byte[]]@(0) })
} 'exactly five entries'
Invoke-PackageContentsCase 'runtime dependency is excluded' {
    param($e) $e.Add([pscustomobject]@{ Name = 'VCRUNTIME140.dll'; Bytes = [byte[]]@(0) })
} 'exactly five entries'
Invoke-PackageContentsCase 'missing instructions' { param($e) $e.RemoveAt(3) } 'exactly five entries'
Invoke-PackageContentsCase 'duplicate entry replacing checksum' { param($e) $e[4].Name = $e[3].Name } 'missing or duplicated'
Invoke-PackageContentsCase 'case alias' { param($e) $e[3].Name = 'readme.txt' } 'missing or duplicated'
Invoke-PackageContentsCase 'nested entry' { param($e) $e[3].Name = 'docs/README.txt' } 'missing or duplicated'
Invoke-PackageContentsCase 'traversal entry' { param($e) $e[3].Name = '../README.txt' } 'missing or duplicated'
Invoke-PackageContentsCase 'directory entry' { param($e) $e[3].Name = 'README.txt/' } 'missing or duplicated'
Invoke-PackageContentsCase 'empty instructions' { param($e) $e[3].Bytes = [byte[]]@() } 'document length mismatch: README.txt'
Invoke-PackageContentsCase 'same-length changed instructions' { param($e) $e[3].Bytes[0] = [byte][char]'X' } 'document bytes mismatch: README.txt'
Invoke-PackageContentsCase 'wrong version instructions' {
    param($e) $e[3].Bytes = (Get-PortableProductPackageDocuments -Version '9.2.3-test' -BinaryHashes $hashes)['README.txt']
} 'document bytes mismatch: README.txt'
Invoke-PackageContentsCase 'wrong binary checksum' { param($e) $e[4].Bytes[0] = [byte][char]'9' } 'document bytes mismatch: SHA256SUMS.txt'
Invoke-PackageContentsCase 'empty checksum file' { param($e) $e[4].Bytes = [byte[]]@() } 'document length mismatch: SHA256SUMS.txt'
Invoke-PackageContentsCase 'missing instructions checksum' {
    param($e) $lines = $utf8.GetString($e[4].Bytes).Split([char]10); $e[4].Bytes = $utf8.GetBytes(($lines[0..2] -join "`n") + "`n")
} 'document length mismatch: SHA256SUMS.txt'
Invoke-PackageContentsCase 'checksum duplicate record' {
    param($e) $e[4].Bytes = $utf8.GetBytes($utf8.GetString($e[4].Bytes) + ('1' * 64) + "  README.txt`n")
} 'document length mismatch: SHA256SUMS.txt'
Invoke-PackageContentsCase 'checksum wrong filename' {
    param($e) $e[4].Bytes = $utf8.GetBytes($utf8.GetString($e[4].Bytes).Replace('README.txt', 'readme.txt'))
} 'document bytes mismatch: SHA256SUMS.txt'
Invoke-PackageContentsCase 'checksum BOM' { param($e) $e[4].Bytes = [byte[]]@(239, 187, 191) + $e[4].Bytes } 'document length mismatch: SHA256SUMS.txt'
Invoke-PackageContentsCase 'checksum CRLF' {
    param($e) $e[4].Bytes = $utf8.GetBytes($utf8.GetString($e[4].Bytes).Replace("`n", "`r`n"))
} 'document length mismatch: SHA256SUMS.txt'
Invoke-PackageContentsCase 'changed instructions with recomputed checksum' {
    param($e)
    $e[3].Bytes[0] = [byte][char]'X'
    $sha = [Security.Cryptography.SHA256]::Create()
    try { $hash = [BitConverter]::ToString($sha.ComputeHash($e[3].Bytes)).Replace('-', '').ToLowerInvariant() }
    finally { $sha.Dispose() }
    $text = $utf8.GetString($e[4].Bytes) -replace '[0-9a-f]{64}  README.txt', "$hash  README.txt"
    $e[4].Bytes = $utf8.GetBytes($text)
} 'document bytes mismatch: README.txt'

Invoke-PackageContentsCase 'forged length conceals truncated instructions' {
    param($e) $e[3].Bytes = $e[3].Bytes[0..($e[3].Bytes.Length - 2)]
} 'document is truncated: README.txt' -ForgeReadmeLength
Invoke-PackageContentsCase 'forged length conceals expanded instructions' {
    param($e) $e[3].Bytes = $e[3].Bytes + [byte[]]@(0)
} 'document exceeds canonical length: README.txt' -ForgeReadmeLength

# Independently check the manifest grammar and all four payload identities;
# these assertions do not use the production document verifier as their oracle.
$lines = $utf8.GetString($documents['SHA256SUMS.txt']).Split([char]10)
if ($lines.Count -ne 5 -or $lines[4] -cne '') { throw 'checksum manifest must have four LF-terminated records' }
$expectedNames = @('Battle Plan Recorder.exe', 'Battle Plan.exe', 'collector.dll', 'README.txt')
$sha = [Security.Cryptography.SHA256]::Create()
try { $readmeHash = [BitConverter]::ToString($sha.ComputeHash($documents['README.txt'])).Replace('-', '').ToLowerInvariant() }
finally { $sha.Dispose() }
for ($index = 0; $index -lt 4; $index++) {
    $expectedHash = if ($index -eq 3) { $readmeHash } else { $hashes[$expectedNames[$index]] }
    if ($lines[$index] -cne "$expectedHash  $($expectedNames[$index])") { throw 'checksum payload binding differs' }
}
$passed.Add('all four payloads have exact SHA256SUMS records')

[pscustomobject]@{
    schema = 'battle-plan.portable_package_contents_test.v1'
    status = 'PASS'
    cases = $passed.Count
    passed_cases = @($passed)
} | ConvertTo-Json -Depth 3
