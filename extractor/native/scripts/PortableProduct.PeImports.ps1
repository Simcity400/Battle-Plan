Set-StrictMode -Version Latest

$script:PortableProductPeImportsSchema = 'battle-plan.pe_imports.v1'
$script:PortableProductForbiddenRuntimePattern = '(^|[\\/])VCRUNTIME[^\\/]*\.DLL$'

function Read-PeUInt16 {
    param([byte[]]$Bytes, [uint64]$Offset, [string]$Context)
    if ($Offset -gt [uint64]$Bytes.Length -or [uint64]$Bytes.Length - $Offset -lt 2) {
        throw "truncated PE while reading ${Context}"
    }
    return [BitConverter]::ToUInt16($Bytes, [int]$Offset)
}

function Read-PeUInt32 {
    param([byte[]]$Bytes, [uint64]$Offset, [string]$Context)
    if ($Offset -gt [uint64]$Bytes.Length -or [uint64]$Bytes.Length - $Offset -lt 4) {
        throw "truncated PE while reading ${Context}"
    }
    return [BitConverter]::ToUInt32($Bytes, [int]$Offset)
}

function Read-PeUInt64 {
    param([byte[]]$Bytes, [uint64]$Offset, [string]$Context)
    if ($Offset -gt [uint64]$Bytes.Length -or [uint64]$Bytes.Length - $Offset -lt 8) {
        throw "truncated PE while reading ${Context}"
    }
    return [BitConverter]::ToUInt64($Bytes, [int]$Offset)
}

function Convert-PeRvaToFileOffset {
    param(
        [byte[]]$Bytes,
        [uint32]$Rva,
        [uint32]$SizeOfHeaders,
        [object[]]$Sections,
        [string]$Context
    )

    if ($Rva -lt $SizeOfHeaders) {
        if ([uint64]$Rva -ge [uint64]$Bytes.Length) {
            throw "PE ${Context} RVA is outside the file"
        }
        return [uint64]$Rva
    }
    foreach ($section in $Sections) {
        $span = [Math]::Max([uint64]$section.virtual_size, [uint64]$section.raw_size)
        $start = [uint64]$section.virtual_address
        $candidate = [uint64]$Rva
        if ($candidate -ge $start -and $candidate -lt ($start + $span)) {
            $delta = $candidate - $start
            if ($delta -ge [uint64]$section.raw_size) {
                throw "PE ${Context} RVA resolves into zero-filled section data"
            }
            $offset = [uint64]$section.raw_offset + $delta
            if ($offset -ge [uint64]$Bytes.Length) {
                throw "PE ${Context} RVA resolves outside the file"
            }
            return $offset
        }
    }
    throw "PE ${Context} RVA is not mapped by any section"
}

function Read-PeAsciiZ {
    param([byte[]]$Bytes, [uint64]$Offset, [string]$Context)

    $characters = [System.Collections.Generic.List[char]]::new()
    for ($index = $Offset; $index -lt [uint64]$Bytes.Length -and $characters.Count -lt 4096; $index++) {
        $value = $Bytes[[int]$index]
        if ($value -eq 0) {
            if ($characters.Count -eq 0) { throw "PE ${Context} is empty" }
            return -join $characters
        }
        if ($value -lt 0x20 -or $value -gt 0x7e) {
            throw "PE ${Context} is not printable ASCII"
        }
        $characters.Add([char]$value)
    }
    throw "PE ${Context} has no bounded NUL terminator"
}

function Get-PortableProductPeImports {
    param(
        [Parameter(Mandatory = $true)][byte[]]$Bytes,
        [Parameter(Mandatory = $true)][string]$Name
    )

    if ($Bytes.Length -lt 64 -or (Read-PeUInt16 $Bytes 0 'DOS signature') -ne 0x5a4d) {
        throw "$Name is not a complete PE image"
    }
    $peOffset = [uint64](Read-PeUInt32 $Bytes 0x3c 'PE header offset')
    if ((Read-PeUInt32 $Bytes $peOffset 'PE signature') -ne 0x00004550) {
        throw "$Name has no PE signature"
    }
    $coffOffset = $peOffset + 4
    $sectionCount = [uint32](Read-PeUInt16 $Bytes ($coffOffset + 2) 'COFF section count')
    $optionalSize = [uint32](Read-PeUInt16 $Bytes ($coffOffset + 16) 'COFF optional-header size')
    if ($sectionCount -eq 0 -or $sectionCount -gt 96) {
        throw "$Name has an invalid PE section count"
    }
    $optionalOffset = $coffOffset + 20
    if ($optionalSize -lt 2 -or $optionalOffset + $optionalSize -gt [uint64]$Bytes.Length) {
        throw "$Name has a truncated PE optional header"
    }
    $magic = Read-PeUInt16 $Bytes $optionalOffset 'optional-header magic'
    if ($magic -eq 0x20b) {
        if ($optionalSize -lt 112) { throw "$Name has a truncated PE32+ optional header" }
        $imageBase = Read-PeUInt64 $Bytes ($optionalOffset + 24) 'PE32+ image base'
        $directoryCountOffset = $optionalOffset + 108
        $directoryOffset = $optionalOffset + 112
    }
    elseif ($magic -eq 0x10b) {
        if ($optionalSize -lt 96) { throw "$Name has a truncated PE32 optional header" }
        $imageBase = [uint64](Read-PeUInt32 $Bytes ($optionalOffset + 28) 'PE32 image base')
        $directoryCountOffset = $optionalOffset + 92
        $directoryOffset = $optionalOffset + 96
    }
    else {
        throw "$Name has an unsupported PE optional-header magic"
    }
    $sizeOfHeaders = Read-PeUInt32 $Bytes ($optionalOffset + 60) 'size of headers'
    $directoryCount = Read-PeUInt32 $Bytes $directoryCountOffset 'data-directory count'
    $availableDirectoryBytes = [uint64]$optionalSize - ($directoryOffset - $optionalOffset)
    if ([uint64]$directoryCount * 8 -gt $availableDirectoryBytes) {
        throw "$Name declares data directories beyond its optional header"
    }
    $sectionTable = $optionalOffset + $optionalSize
    if ($sectionTable + ([uint64]$sectionCount * 40) -gt [uint64]$Bytes.Length) {
        throw "$Name has a truncated PE section table"
    }
    $sections = @()
    for ($sectionIndex = 0; $sectionIndex -lt $sectionCount; $sectionIndex++) {
        $offset = $sectionTable + ([uint64]$sectionIndex * 40)
        $sections += [pscustomobject]@{
            virtual_size = Read-PeUInt32 $Bytes ($offset + 8) 'section virtual size'
            virtual_address = Read-PeUInt32 $Bytes ($offset + 12) 'section virtual address'
            raw_size = Read-PeUInt32 $Bytes ($offset + 16) 'section raw size'
            raw_offset = Read-PeUInt32 $Bytes ($offset + 20) 'section raw offset'
        }
    }

    function Get-Directory([uint32]$Index, [string]$Context) {
        if ($directoryCount -le $Index) {
            return [pscustomobject]@{ rva = [uint32]0; size = [uint32]0 }
        }
        $offset = $directoryOffset + ([uint64]$Index * 8)
        $rva = Read-PeUInt32 $Bytes $offset "$Context RVA"
        $size = Read-PeUInt32 $Bytes ($offset + 4) "$Context size"
        if (($rva -eq 0) -xor ($size -eq 0)) {
            throw "$Name has an inconsistent $Context directory"
        }
        return [pscustomobject]@{ rva = $rva; size = $size }
    }

    function Read-ImportDirectory($Directory, [uint32]$DescriptorSize, [uint32]$NameFieldOffset, [bool]$DelayLoad) {
        if ($Directory.rva -eq 0) { return @() }
        if ($Directory.size -lt $DescriptorSize) { throw "$Name has a truncated PE import directory" }
        $result = [System.Collections.Generic.List[string]]::new()
        $terminated = $false
        for ($relative = [uint64]0; $relative + $DescriptorSize -le [uint64]$Directory.size; $relative += $DescriptorSize) {
            if ([uint64]$Directory.rva + $relative -gt [uint32]::MaxValue) {
                throw "$Name has an overflowing PE import-directory RVA"
            }
            $descriptor = Convert-PeRvaToFileOffset `
                $Bytes ([uint32]([uint64]$Directory.rva + $relative)) $sizeOfHeaders $sections 'import descriptor'
            if ($descriptor + $DescriptorSize -gt [uint64]$Bytes.Length) {
                throw "$Name has a truncated PE import descriptor"
            }
            $allZero = $true
            for ($byteIndex = [uint64]0; $byteIndex -lt $DescriptorSize; $byteIndex++) {
                if ($Bytes[[int]($descriptor + $byteIndex)] -ne 0) { $allZero = $false; break }
            }
            if ($allZero) { $terminated = $true; break }
            $nameValue = [uint64](Read-PeUInt32 $Bytes ($descriptor + $NameFieldOffset) 'import name')
            if ($nameValue -eq 0) { throw "$Name has an import descriptor without a DLL name" }
            if ($DelayLoad) {
                $attributes = Read-PeUInt32 $Bytes $descriptor 'delay-import attributes'
                if (($attributes -band 0xfffffffe) -ne 0) {
                    throw "$Name has unsupported delay-import attributes"
                }
                if (($attributes -band 1) -eq 0) {
                    if ($nameValue -lt $imageBase -or $nameValue - $imageBase -gt [uint32]::MaxValue) {
                        throw "$Name has an invalid VA-based delay-import name"
                    }
                    $nameValue -= $imageBase
                }
            }
            if ($nameValue -gt [uint32]::MaxValue) { throw "$Name has an out-of-range import-name RVA" }
            $nameOffset = Convert-PeRvaToFileOffset $Bytes ([uint32]$nameValue) $sizeOfHeaders $sections 'import DLL name'
            $result.Add((Read-PeAsciiZ $Bytes $nameOffset 'import DLL name'))
        }
        if (-not $terminated) { throw "$Name has no bounded terminator in its PE import directory" }
        return @($result)
    }

    $regular = @(Read-ImportDirectory (Get-Directory 1 'import') 20 12 $false)
    $delay = @(Read-ImportDirectory (Get-Directory 13 'delay import') 32 4 $true)
    $all = @($regular + $delay | Sort-Object -Unique)
    $forbidden = @($all | Where-Object { $_ -match $script:PortableProductForbiddenRuntimePattern })
    return [pscustomobject]@{
        schema = $script:PortableProductPeImportsSchema
        regular = $regular
        delay_load = $delay
        all = $all
        forbidden_pattern = 'VCRUNTIME*.dll'
        vcruntime_imports = $forbidden
        static_vcruntime_admission = ($forbidden.Count -eq 0)
    }
}

function Assert-PortableProductStaticCrt {
    param(
        [Parameter(Mandatory = $true)][byte[]]$Bytes,
        [Parameter(Mandatory = $true)][string]$Name
    )

    $imports = Get-PortableProductPeImports -Bytes $Bytes -Name $Name
    if (-not $imports.static_vcruntime_admission) {
        throw "$Name imports forbidden portable-runtime DLL(s): $($imports.vcruntime_imports -join ', ')"
    }
    return $imports
}

function Assert-PortableProductPeImportAttestation {
    param(
        [Parameter(Mandatory = $true)][byte[]]$Bytes,
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)]$Attestation
    )

    $current = Assert-PortableProductStaticCrt -Bytes $Bytes -Name $Name
    if ([string]$Attestation.schema -cne $script:PortableProductPeImportsSchema -or
        [string]$Attestation.forbidden_pattern -cne 'VCRUNTIME*.dll' -or
        $Attestation.static_vcruntime_admission -isnot [bool] -or
        -not $Attestation.static_vcruntime_admission -or
        @($Attestation.vcruntime_imports).Count -ne 0) {
        throw "$Name has no valid static-CRT PE-import attestation"
    }
    foreach ($field in @('regular', 'delay_load', 'all')) {
        $expected = @($current.$field)
        $observed = @($Attestation.$field)
        if ($expected.Count -ne $observed.Count) {
            throw "$Name PE-import attestation count differs for $field"
        }
        for ($index = 0; $index -lt $expected.Count; $index++) {
            if ([string]$expected[$index] -cne [string]$observed[$index]) {
                throw "$Name PE-import attestation differs for $field"
            }
        }
    }
    return $current
}
