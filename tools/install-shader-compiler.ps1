[CmdletBinding()]
param(
    [string]$ArchivePath,
    [Parameter(Mandatory)][string]$Destination
)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$releaseUri = 'https://github.com/microsoft/DirectXShaderCompiler/releases/download/v1.9.2607/dxc_2026_07_29.zip'
$expectedDigest = 'a1dfb116ba3eeae6a1582291b53a8e7bf65ad760676bd3194685c8f7367cd241'
$downloaded = $false
try {
    if (-not $ArchivePath) {
        $ArchivePath = Join-Path ([IO.Path]::GetTempPath()) ("hoi4-dxc-{0}.zip" -f [guid]::NewGuid())
        $downloaded = $true
        Invoke-WebRequest -UseBasicParsing -Uri $releaseUri -OutFile $ArchivePath
    }
    $digest = (Get-FileHash -LiteralPath $ArchivePath -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($digest -ne $expectedDigest) {
        throw "DXC archive SHA256 mismatch: expected $expectedDigest, got $digest"
    }
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $zip = [IO.Compression.ZipFile]::OpenRead((Resolve-Path -LiteralPath $ArchivePath))
    try {
        New-Item -ItemType Directory -Force -Path $Destination | Out-Null
        foreach ($member in @('bin/x64/dxcompiler.dll', 'bin/x64/dxil.dll')) {
            $entry = $zip.Entries | Where-Object { $_.FullName.Replace('\', '/') -ceq $member } | Select-Object -First 1
            if (-not $entry) { throw "DXC archive is missing $member" }
            $path = Join-Path $Destination ([IO.Path]::GetFileName($member))
            [IO.Compression.ZipFileExtensions]::ExtractToFile($entry, $path, $true)
        }
    } finally {
        $zip.Dispose()
    }
    Write-Output (Join-Path $Destination 'dxcompiler.dll')
} finally {
    if ($downloaded -and (Test-Path -LiteralPath $ArchivePath)) {
        Remove-Item -LiteralPath $ArchivePath
    }
}
