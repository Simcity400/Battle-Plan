[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
. (Join-Path $PSScriptRoot '..\..\..\portable_replay\native\scripts\PortableProduct.PeImports.ps1')

function Set-U16([byte[]]$Bytes, [int]$Offset, [uint16]$Value) {
    [BitConverter]::GetBytes($Value).CopyTo($Bytes, $Offset)
}

function Set-U32([byte[]]$Bytes, [int]$Offset, [uint32]$Value) {
    [BitConverter]::GetBytes($Value).CopyTo($Bytes, $Offset)
}

function Set-U64([byte[]]$Bytes, [int]$Offset, [uint64]$Value) {
    [BitConverter]::GetBytes($Value).CopyTo($Bytes, $Offset)
}

function New-SyntheticPe([string]$RegularImport, [string]$DelayImport) {
    $bytes = [byte[]]::new(2048)
    Set-U16 $bytes 0 0x5a4d
    Set-U32 $bytes 0x3c 0x80
    Set-U32 $bytes 0x80 0x00004550
    Set-U16 $bytes 0x84 0x8664
    Set-U16 $bytes 0x86 1
    Set-U16 $bytes 0x94 240
    Set-U16 $bytes 0x96 0x0022
    Set-U16 $bytes 0x98 0x020b
    Set-U64 $bytes 0xb0 0x0000000140000000
    Set-U32 $bytes 0xd4 0x200
    Set-U32 $bytes 0x104 16
    Set-U32 $bytes 0x110 0x1000
    Set-U32 $bytes 0x114 40
    Set-U32 $bytes 0x170 0x1200
    Set-U32 $bytes 0x174 64
    Set-U32 $bytes 0x190 0x600
    Set-U32 $bytes 0x194 0x1000
    Set-U32 $bytes 0x198 0x600
    Set-U32 $bytes 0x19c 0x200
    Set-U32 $bytes 0x20c 0x1100
    Set-U32 $bytes 0x400 1
    Set-U32 $bytes 0x404 0x1300
    [Text.Encoding]::ASCII.GetBytes($RegularImport + [char]0).CopyTo($bytes, 0x300)
    [Text.Encoding]::ASCII.GetBytes($DelayImport + [char]0).CopyTo($bytes, 0x500)
    return ,$bytes
}

$accepted = Assert-PortableProductStaticCrt -Bytes (New-SyntheticPe 'KERNEL32.dll' 'USER32.dll') -Name 'accepted synthetic PE'
if ((@($accepted.regular) -join ',') -cne 'KERNEL32.dll' -or
    (@($accepted.delay_load) -join ',') -cne 'USER32.dll' -or
    -not $accepted.static_vcruntime_admission) {
    throw 'synthetic allowed-import admission returned the wrong import manifest'
}

foreach ($case in @(
    @{ regular = 'VCRUNTIME140.dll'; delay = 'USER32.dll' },
    @{ regular = 'KERNEL32.dll'; delay = 'VCRUNTIME140_1.dll' }
)) {
    $rejected = $false
    try {
        [void](Assert-PortableProductStaticCrt -Bytes (New-SyntheticPe $case.regular $case.delay) -Name 'forbidden synthetic PE')
    }
    catch {
        if ($_.Exception.Message -notmatch 'imports forbidden portable-runtime DLL') { throw }
        $rejected = $true
    }
    if (-not $rejected) { throw 'synthetic VCRUNTIME import was not rejected' }
}

$malformed = New-SyntheticPe 'KERNEL32.dll' 'USER32.dll'
# Windows PowerShell 5.1 parses this hexadecimal literal as a negative Int32;
# spell the value as a positive Int64 before the UInt32 parameter conversion.
Set-U32 $malformed 0x20c ([uint32]2415919104)
$malformedRejected = $false
try { [void](Get-PortableProductPeImports -Bytes $malformed -Name 'malformed synthetic PE') }
catch { $malformedRejected = $true }
if (-not $malformedRejected) { throw 'malformed import-name RVA was not rejected fail closed' }

[pscustomobject]@{
    schema = 'battle-plan.portable_product_static_crt_test.v1'
    status = 'PASS'
    cases = 4
} | ConvertTo-Json -Compress
