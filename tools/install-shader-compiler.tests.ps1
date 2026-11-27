Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$installerPath = Join-Path $PSScriptRoot 'install-shader-compiler.ps1'
$tokens = $null
$errors = $null
$ast = [Management.Automation.Language.Parser]::ParseFile($installerPath, [ref]$tokens, [ref]$errors)
if ($errors.Count) { throw $errors[0] }
foreach ($definition in $ast.FindAll({
    param($node)
    $node -is [Management.Automation.Language.FunctionDefinitionAst] -and
    $node.Name -in @('Test-TransientDownloadError', 'Invoke-DownloadWithRetry')
}, $false)) {
    . ([scriptblock]::Create($definition.Extent.Text))
}

$retryCalls = @($ast.EndBlock.FindAll({
    param($node)
    $node -is [Management.Automation.Language.CommandAst] -and $node.GetCommandName() -eq 'Invoke-DownloadWithRetry'
}, $true))
if ($retryCalls.Count -ne 1) { throw "Expected installer script to invoke Invoke-DownloadWithRetry once, found $($retryCalls.Count)." }
$directWebCalls = @($ast.EndBlock.FindAll({
    param($node)
    $node -is [Management.Automation.Language.CommandAst] -and $node.GetCommandName() -eq 'Invoke-WebRequest'
}, $false))
if ($directWebCalls.Count -ne 0) { throw "Installer script must not call Invoke-WebRequest outside Invoke-DownloadWithRetry." }

function New-HttpStatusException([int]$StatusCode, [string]$Message) {
    $ex = [System.Net.WebException]::new($Message, [System.Net.WebExceptionStatus]::ProtocolError)
    $response = [pscustomobject]@{ StatusCode = [System.Net.HttpStatusCode]$StatusCode }
    $ex | Add-Member -NotePropertyName Response -NotePropertyValue $response -Force
    return $ex
}

function Assert-Equal($Actual, $Expected, [string]$Label) {
    if ($Actual -cne $Expected) {
        throw "${Label}: expected '$Expected', got '$Actual'"
    }
}

$root = Join-Path ([IO.Path]::GetTempPath()) ('hoi4-dxc-test-' + [guid]::NewGuid().ToString('N'))
$null = New-Item -ItemType Directory -Path $root
try {
    $out = Join-Path $root 'attempt.zip'

    $attempts = 0
    $delays = [Collections.Generic.List[int]]::new()
    Invoke-DownloadWithRetry -Uri 'https://example.invalid/dxc.zip' -OutFile $out -Download {
        param($u, $o)
        $script:attempts++
        [IO.File]::WriteAllText($o, "partial-$script:attempts")
        if ($script:attempts -lt 3) {
            throw (New-HttpStatusException 504 'The remote server returned an error: (504) Gateway Timeout.')
        }
        [IO.File]::WriteAllText($o, 'complete')
    } -Wait {
        param($s)
        if (Test-Path -LiteralPath $out) { throw 'Partial file was not removed before waiting for retry.' }
        $delays.Add($s)
    }
    Assert-Equal $attempts 3 '504 recovery attempts'
    Assert-Equal ($delays -join ',') '2,4' '504 recovery backoff'
    Assert-Equal ([IO.File]::ReadAllText($out)) 'complete' '504 recovery content'
    Remove-Item -LiteralPath $out

    foreach ($case in @(
        @{ Name = '500 internal server error'; Make = { New-HttpStatusException 500 'Internal Server Error' } },
        @{ Name = '502 bad gateway'; Make = { New-HttpStatusException 502 'Bad Gateway' } },
        @{ Name = '503 service unavailable'; Make = { New-HttpStatusException 503 'Service Unavailable' } },
        @{ Name = 'timeout status'; Make = { [System.Net.WebException]::new('The operation has timed out', [System.Net.WebExceptionStatus]::Timeout) } },
        @{ Name = 'connection closed'; Make = { [System.Net.WebException]::new('The underlying connection was closed', [System.Net.WebExceptionStatus]::ConnectionClosed) } },
        @{ Name = 'receive failure'; Make = { [System.Net.WebException]::new('The underlying connection was closed unexpectedly', [System.Net.WebExceptionStatus]::ReceiveFailure) } },
        @{ Name = 'socket reset inside IOException'; Make = { [System.IO.IOException]::new('Transport read failed', [System.Net.Sockets.SocketException]::new(10054)) } },
        @{ Name = 'premature EOF IOException without inner'; Make = { [System.IO.IOException]::new('Received an unexpected EOF or 0 bytes from the transport stream.') } },
        @{ Name = 'premature response end IOException with System.Net source'; Make = { $e = [System.IO.IOException]::new('Stream aborted'); $e.Source = 'System.Net.Http'; $e } },
        @{ Name = 'EndOfStreamException'; Make = { [System.IO.EndOfStreamException]::new('Unexpected end of stream') } },
        @{ Name = 'TimeoutException'; Make = { [System.TimeoutException]::new('Timed out') } },
        @{ Name = 'TaskCanceledException'; Make = { [System.Threading.Tasks.TaskCanceledException]::new('Request timed out') } }
    )) {
        $script:attempts = 0
        $delays.Clear()
        $caught = $null
        try {
            Invoke-DownloadWithRetry -Uri 'https://example.invalid/dxc.zip' -OutFile $out -Download {
                param($u, $o)
                $script:attempts++
                [IO.File]::WriteAllText($o, 'partial')
                throw (& $case.Make)
            } -Wait {
                param($s)
                $delays.Add($s)
            }
        } catch {
            $caught = $_.Exception
        }
        if ($null -eq $caught) { throw "$($case.Name): expected failure after exhausting retries" }
        Assert-Equal $script:attempts 4 "$($case.Name) attempt count"
        Assert-Equal ($delays -join ',') '2,4,8' "$($case.Name) growing pauses"
        if (Test-Path -LiteralPath $out) { throw "$($case.Name): partial file remained after failure" }
    }

    foreach ($case in @(
        @{ Name = '404 not found'; Expected = '404'; Make = { New-HttpStatusException 404 'The remote server returned an error: (404) Not Found.' } },
        @{ Name = '403 forbidden'; Expected = '403'; Make = { New-HttpStatusException 403 'The remote server returned an error: (403) Forbidden.' } },
        @{ Name = '429 rate limited'; Expected = '429'; Make = { New-HttpStatusException 429 'Too Many Requests' } },
        @{ Name = 'trust failure'; Expected = 'TLS trust'; Make = { [System.Net.WebException]::new('TLS trust failed', [System.Net.WebExceptionStatus]::TrustFailure) } },
        @{ Name = 'local disk error'; Expected = 'disk full'; Make = { [System.IO.IOException]::new('disk full') } },
        @{ Name = 'local file lock error'; Expected = 'used by another process'; Make = { [System.IO.IOException]::new('The process cannot access the file because it is being used by another process.') } }
    )) {
        $script:attempts = 0
        $delays.Clear()
        $caught = $null
        try {
            Invoke-DownloadWithRetry -Uri 'https://example.invalid/dxc.zip' -OutFile $out -Download {
                param($u, $o)
                $script:attempts++
                throw (& $case.Make)
            } -Wait {
                param($s)
                $delays.Add($s)
            }
        } catch {
            $caught = $_.Exception
        }
        if ($null -eq $caught -or $caught.Message -notlike "*$($case.Expected)*") {
            throw "$($case.Name): expected message containing '$($case.Expected)', got '$($caught.Message)'"
        }
        Assert-Equal $script:attempts 1 "$($case.Name) must fail immediately"
        Assert-Equal $delays.Count 0 "$($case.Name) must not sleep"
    }

    $dest = Join-Path $root 'dest'
    $state = @{ WebCalls = 0; LastTempPath = $null; Mode = 'wrong-bytes'; FakeZip = $null }
    function Invoke-WebRequest {
        param([switch]$UseBasicParsing, [string]$Uri, [string]$OutFile)
        $state.WebCalls++
        $state.LastTempPath = $OutFile
        if ($state.Mode -eq '404') {
            throw (New-HttpStatusException 404 'The remote server returned an error: (404) Not Found.')
        }
        if ($state.Mode -eq 'good-zip') {
            Copy-Item -LiteralPath $state.FakeZip -Destination $OutFile
            return
        }
        [IO.File]::WriteAllText($OutFile, 'wrong archive bytes')
    }

    $hashErr = $null
    try {
        $null = & $installerPath -Destination $dest
    } catch {
        $hashErr = $_.Exception.Message
    }
    if (-not $hashErr -or $hashErr -notlike '*DXC archive SHA256 mismatch:*') {
        throw "Expected immediate SHA256 mismatch error, got '$hashErr'"
    }
    Assert-Equal $state.WebCalls 1 'SHA256 mismatch must not retry download'
    if ($state.LastTempPath -and (Test-Path -LiteralPath $state.LastTempPath)) {
        throw 'Downloaded temp archive was not removed after SHA256 mismatch.'
    }

    $state.WebCalls = 0
    $state.LastTempPath = $null
    $state.Mode = '404'
    $notFoundErr = $null
    try {
        $null = & $installerPath -Destination $dest
    } catch {
        $notFoundErr = $_.Exception.Message
    }
    if (-not $notFoundErr -or $notFoundErr -notlike '*404*') {
        throw "Expected immediate 404 error from script, got '$notFoundErr'"
    }
    Assert-Equal $state.WebCalls 1 'Script 404 must fail on first attempt'
    if ($state.LastTempPath -and (Test-Path -LiteralPath $state.LastTempPath)) {
        throw 'Downloaded temp archive remained after 404.'
    }

    Add-Type -AssemblyName System.IO.Compression
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $fakeZip = Join-Path $root 'good.zip'
    $zip = [IO.Compression.ZipFile]::Open($fakeZip, [IO.Compression.ZipArchiveMode]::Create)
    try {
        foreach ($entryName in @('bin/x64/dxcompiler.dll', 'bin/x64/dxil.dll')) {
            $entry = $zip.CreateEntry($entryName)
            $writer = [IO.StreamWriter]::new($entry.Open())
            try { $writer.Write("stub:$entryName") } finally { $writer.Dispose() }
        }
    } finally {
        $zip.Dispose()
    }
    $state.WebCalls = 0
    $state.LastTempPath = $null
    $state.Mode = 'good-zip'
    $state.FakeZip = $fakeZip
    function Get-FileHash {
        param([string]$LiteralPath, [string]$Algorithm)
        return [pscustomobject]@{ Hash = 'A1DFB116BA3EEAE6A1582291B53A8E7BF65AD760676BD3194685C8F7367CD241' }
    }
    $installed = & $installerPath -Destination $dest
    Assert-Equal $state.WebCalls 1 'Successful download calls Invoke-WebRequest once'
    Assert-Equal $installed (Join-Path $dest 'dxcompiler.dll') 'Script outputs dxcompiler.dll path'
    Assert-Equal ([IO.File]::ReadAllText((Join-Path $dest 'dxcompiler.dll'))) 'stub:bin/x64/dxcompiler.dll' 'dxcompiler.dll extracted'
    Assert-Equal ([IO.File]::ReadAllText((Join-Path $dest 'dxil.dll'))) 'stub:bin/x64/dxil.dll' 'dxil.dll extracted'
    if ($state.LastTempPath -and (Test-Path -LiteralPath $state.LastTempPath)) {
        throw 'Downloaded temp archive was not removed after successful install.'
    }
} finally {
    if (Test-Path -LiteralPath $root) {
        Remove-Item -LiteralPath $root -Recurse
    }
}
Write-Host 'PASS install-shader-compiler tests'
