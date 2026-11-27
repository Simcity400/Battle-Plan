[CmdletBinding()]
param(
    [string]$ArchivePath,
    [Parameter(Mandatory)][string]$Destination
)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$releaseUri = 'https://github.com/microsoft/DirectXShaderCompiler/releases/download/v1.9.2607/dxc_2026_07_29.zip'
$expectedDigest = 'a1dfb116ba3eeae6a1582291b53a8e7bf65ad760676bd3194685c8f7367cd241'

function Test-TransientDownloadError {
    param([Exception]$Exception)
    $transientStatus = @(
        'Timeout', 'ConnectFailure', 'ReceiveFailure', 'SendFailure',
        'ConnectionClosed', 'KeepAliveFailure', 'PipelineFailure',
        'NameResolutionFailure', 'ProxyNameResolutionFailure',
        'ServerProtocolViolation', 'SecureChannelFailure', 'UnknownError', 'Pending'
    )
    for ($current = $Exception; $null -ne $current; $current = $current.InnerException) {
        $responseProp = $current.PSObject.Properties['Response']
        if ($responseProp -and $null -ne $responseProp.Value) {
            $codeProp = $responseProp.Value.PSObject.Properties['StatusCode']
            if ($codeProp -and $null -ne $codeProp.Value) {
                $code = [int]$codeProp.Value
                return ($code -ge 500 -and $code -le 599)
            }
        }
        $statusProp = $current.PSObject.Properties['StatusCode']
        if ($statusProp -and $null -ne $statusProp.Value) {
            $code = [int]$statusProp.Value
            return ($code -ge 500 -and $code -le 599)
        }
        if ($current -is [System.Net.WebException]) {
            return ($transientStatus -contains $current.Status.ToString())
        }
        if ($current -is [System.Net.Sockets.SocketException] -or
            $current -is [System.TimeoutException] -or
            $current -is [System.Threading.Tasks.TaskCanceledException] -or
            $current -is [System.IO.EndOfStreamException] -or
            $current.GetType().FullName -in @('System.Net.Http.HttpRequestException', 'System.Net.Http.HttpIOException')) {
            return $true
        }
        if ($current.GetType() -eq [System.IO.IOException] -and (
                ($null -ne $current.TargetSite -and $null -ne $current.TargetSite.DeclaringType -and $current.TargetSite.DeclaringType.Namespace -like 'System.Net*') -or
                ($current.Source -like 'System.Net*') -or
                ($current.Message -match 'transport (?:connection|stream)|unexpected EOF|response ended prematurely|connection was closed')
            )) {
            return $true
        }
    }
    return $false
}

function Invoke-DownloadWithRetry {
    param(
        [Parameter(Mandatory)][string]$Uri,
        [Parameter(Mandatory)][string]$OutFile,
        [int]$MaxAttempts = 4,
        [int[]]$BackoffSeconds = @(2, 4, 8),
        [scriptblock]$Download = { param($TargetUri, $TargetPath) Invoke-WebRequest -UseBasicParsing -Uri $TargetUri -OutFile $TargetPath },
        [scriptblock]$Wait = { param($Seconds) Start-Sleep -Seconds $Seconds }
    )
    for ($attempt = 1; $attempt -le $MaxAttempts; $attempt++) {
        try {
            & $Download $Uri $OutFile
            return
        } catch {
            $failure = $_
            if (Test-Path -LiteralPath $OutFile) {
                Remove-Item -LiteralPath $OutFile
            }
            if ($attempt -ge $MaxAttempts -or -not (Test-TransientDownloadError $failure.Exception)) {
                throw $failure
            }
            $delay = $BackoffSeconds[[Math]::Min($attempt - 1, $BackoffSeconds.Length - 1)]
            [Console]::Error.WriteLine("DXC download attempt $attempt/$MaxAttempts failed ($($failure.Exception.Message)); retrying in ${delay}s")
            & $Wait $delay
        }
    }
}

$downloaded = $false
try {
    if (-not $ArchivePath) {
        $ArchivePath = Join-Path ([IO.Path]::GetTempPath()) ("hoi4-dxc-{0}.zip" -f [guid]::NewGuid())
        $downloaded = $true
        Invoke-DownloadWithRetry -Uri $releaseUri -OutFile $ArchivePath
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
