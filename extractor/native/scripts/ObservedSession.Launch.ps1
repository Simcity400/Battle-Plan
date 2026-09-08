# Isolated adaptation of Invoke-HeadlessRawCapture.ps1 at
# 4c571a5e7764df6a37f32c99913e767c79bb2eac: ConvertTo-NativeArgumentLine,
# Invoke-BoundedNativeCapture, Invoke-BoundedWmiLaunch, and its caller handoff.
# WMI service creation avoids inheriting the automation caller's job. Retain
# raw results before identity checks; omit the legacy control/cleanup paths.
# Importing this file only defines functions. Never import the legacy harness.

function ConvertTo-ObservedArgumentLine {
    param([AllowEmptyCollection()][AllowEmptyString()][string[]]$Arguments)
    return (($Arguments | ForEach-Object {
        $value = [string]$_
        if ($value.IndexOf([char]0) -ge 0) { throw 'NUL is not a Windows command-line argument.' }
        if ($value.Length -eq 0 -or $value -match '[\s"]') {
            '"' + ($value -replace '(\\*)"', '$1$1\"' -replace '(\\+)$', '$1$1') + '"'
        } else { $value }
    }) -join ' ')
}

function Write-ObservedJson {
    param([string]$Path, [object]$Value)
    $temporary = $Path + '.' + [Guid]::NewGuid().ToString('N') + '.tmp'
    [IO.File]::WriteAllText($temporary, ($Value | ConvertTo-Json -Depth 12), [Text.UTF8Encoding]::new($false))
    if ([IO.File]::Exists($Path)) { [IO.File]::Replace($temporary, $Path, [NullString]::Value) }
    else { [IO.File]::Move($temporary, $Path) }
}

function Get-ObservedFileIdentity {
    param([string]$Path)
    $item = Get-Item -LiteralPath $Path -ErrorAction Stop
    if ($item.PSIsContainer) { throw "Not a file: $Path" }
    return [ordered]@{
        path = $item.FullName
        bytes = [long]$item.Length
        sha256 = (Get-FileHash -LiteralPath $item.FullName -Algorithm SHA256 -ErrorAction Stop).Hash.ToLowerInvariant()
    }
}

function Assert-ObservedAbsolutePath {
    param([string]$Path)
    # IsPathRooted also accepts C:relative and \current-drive-relative on .NET
    # Framework. Neither identifies an explicit absolute filesystem location.
    if ($Path -notmatch '^(?:[a-zA-Z]:[\\/]|\\\\[^\\/]+[\\/][^\\/]+[\\/])') {
        throw 'Path must be absolute, including its drive or UNC server/share.'
    }
}

function Get-ObservedProcessIdentity {
    param([int]$TargetProcessId)
    $process = Get-Process -Id $TargetProcessId -ErrorAction Stop
    try {
        # Hold the handle while observing; do not reopen by a potentially reused PID.
        $null = $process.Handle
        if ($process.HasExited) { throw 'Process exited before custody observation.' }
        $result = [ordered]@{
            pid = [int]$process.Id
            creation_filetime = $process.StartTime.ToUniversalTime().ToFileTimeUtc().ToString([Globalization.CultureInfo]::InvariantCulture)
            exe_path = $process.MainModule.FileName
            session_id = [int]$process.SessionId
        }
        if ($process.HasExited) { throw 'Process exited during custody observation.' }
        return $result
    } finally { $process.Dispose() }
}

function Assert-ObservedUInt {
    param([object]$Value, [uint64]$Maximum, [switch]$Nonzero)
    if (-not ($Value -is [int] -or $Value -is [long] -or $Value -is [uint32] -or $Value -is [uint64])) {
        throw 'Expected an integer, not a coercible JSON value.'
    }
    if ($Value -lt 0 -or $Value -gt $Maximum -or ($Nonzero -and $Value -eq 0)) {
        throw 'Integer outside custody bounds.'
    }
}

function Assert-ObservedProcess {
    param([object]$Identity, [int]$ExpectedPid, [string]$ExpectedPath, [string]$EarliestCreation, [string]$LatestCreation)
    Assert-ObservedUInt $Identity.pid ([int]::MaxValue) -Nonzero
    Assert-ObservedUInt $Identity.session_id ([int]::MaxValue)
    if ($Identity.pid -ne $ExpectedPid -or -not ($Identity.exe_path -is [string]) -or
        -not [StringComparer]::OrdinalIgnoreCase.Equals($Identity.exe_path, $ExpectedPath)) {
        throw 'Process PID/image differs from launch authority.'
    }
    [long]$creation = 0
    if (-not ($Identity.creation_filetime -is [string]) -or
        $Identity.creation_filetime -cnotmatch '^[1-9][0-9]{0,18}$' -or
        -not [long]::TryParse($Identity.creation_filetime, [ref]$creation) -or
        $creation -lt [long]$EarliestCreation -or $creation -gt [long]$LatestCreation) {
        throw 'Process creation FILETIME is not canonical or is outside this launch interval.'
    }
}

function Invoke-ObservedWmiWorker {
    param([string]$RequestPath)
    $request = Get-Content -LiteralPath $RequestPath -Raw | ConvertFrom-Json
    $directory = Split-Path -Parent $RequestPath
    $created = Invoke-CimMethod -ClassName Win32_Process -MethodName Create `
        -OperationTimeoutSec $request.timeout_seconds -Arguments @{
            CommandLine = $request.command_line
            CurrentDirectory = $request.working_directory
        } -ErrorAction Stop
    # Persist the API result even when subsequent process lookup fails.
    Write-ObservedJson (Join-Path $directory 'WMI_RESULT.json') ([ordered]@{
        ReturnValue = $created.ReturnValue; ProcessId = $created.ProcessId
    })
    Assert-ObservedUInt $created.ReturnValue ([uint32]::MaxValue)
    if ($created.ReturnValue -ne 0) { throw "Win32_Process.Create returned $($created.ReturnValue)." }
    Assert-ObservedUInt $created.ProcessId ([int]::MaxValue) -Nonzero
    $identity = Get-ObservedProcessIdentity ([int]$created.ProcessId)
    Assert-ObservedProcess $identity ([int]$created.ProcessId) $request.exe_before.path `
        $request.launch_started_filetime ([DateTime]::UtcNow.ToFileTimeUtc().ToString())
    Write-ObservedJson (Join-Path $directory 'PROCESS_HANDOFF.json') $identity
}

function Invoke-ObservedBoundedHelper {
    param([string]$Command, [string]$OutputDirectory, [int]$TimeoutSeconds)
    $encoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($Command))
    $process = [Diagnostics.Process]::new()
    $process.StartInfo.FileName = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
    $process.StartInfo.Arguments = '-NoProfile -NonInteractive -ExecutionPolicy Bypass -EncodedCommand ' + $encoded
    $process.StartInfo.UseShellExecute = $false
    $process.StartInfo.CreateNoWindow = $true
    $process.StartInfo.RedirectStandardOutput = $true
    $process.StartInfo.RedirectStandardError = $true
    $started = $false
    $watch = [Diagnostics.Stopwatch]::StartNew()
    $result = [ordered]@{ pid = $null; creation_filetime = $null; exe_path = $process.StartInfo.FileName
        exit_code = $null; timed_out = $false; terminated = $false; residual = $false; elapsed_ms = 0; error = $null }
    try {
        $started = $process.Start()
        if (-not $started) { throw 'PowerShell helper did not start.' }
        # Process.Start retains the actual child handle, including its exit code.
        $result.pid = $process.Id
        $result.creation_filetime = $process.StartTime.ToUniversalTime().ToFileTimeUtc().ToString()
        $stdout = $process.StandardOutput.ReadToEndAsync()
        $stderr = $process.StandardError.ReadToEndAsync()
        if (-not $process.WaitForExit($TimeoutSeconds * 1000)) {
            $result.timed_out = $true
            $process.Kill() # Only the owned helper handle; never the WMI-created game.
            $result.terminated = $true
            if (-not $process.WaitForExit(2000)) { throw 'Owned helper survived its termination bound.' }
        }
        $result.exit_code = $process.ExitCode
        if (-not $stdout.Wait(1000) -or -not $stderr.Wait(1000)) { throw 'Helper stream flush exceeded its bound.' }
        [IO.File]::WriteAllText((Join-Path $OutputDirectory 'helper.stdout.txt'), $stdout.Result)
        [IO.File]::WriteAllText((Join-Path $OutputDirectory 'helper.stderr.txt'), $stderr.Result)
    } catch { $result.error = $_.Exception.Message }
    finally {
        if ($started -and -not $process.HasExited) {
            try {
                $process.Kill()
                $result.terminated = $true
                $result.residual = -not $process.WaitForExit(2000)
            } catch { $result.residual = $true; $result.error = $_.Exception.Message }
        }
        $result.elapsed_ms = [long]$watch.ElapsedMilliseconds
        $process.Dispose()
    }
    return $result
}

function Invoke-ObservedLaunchHelper {
    param([string]$RequestPath, [string]$HelperPath, [int]$TimeoutSeconds)
    $helperLiteral = "'" + $HelperPath.Replace("'", "''") + "'"
    $requestLiteral = "'" + $RequestPath.Replace("'", "''") + "'"
    $command = "`$ErrorActionPreference = 'Stop'; Set-StrictMode -Version Latest; . $helperLiteral; Invoke-ObservedWmiWorker -RequestPath $requestLiteral"
    return Invoke-ObservedBoundedHelper $command (Split-Path -Parent $RequestPath) ($TimeoutSeconds + 5)
}

function Invoke-ObservedSession {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)][string]$GameExe,
        [Parameter(Mandatory = $true)][AllowEmptyCollection()][AllowEmptyString()][string[]]$GameArguments,
        [Parameter(Mandatory = $true)][string]$ExpectedExeSha256,
        [Parameter(Mandatory = $true)][string]$OutputDirectory,
        [ValidateRange(1, 120)][int]$TimeoutSeconds = 30
    )
    $ErrorActionPreference = 'Stop'
    Set-StrictMode -Version Latest
    Assert-ObservedAbsolutePath $OutputDirectory
    $output = [IO.Path]::GetFullPath($OutputDirectory)
    # New-Item without Force refuses preexisting custody. No earlier receipt is overwritten.
    $null = New-Item -ItemType Directory -Path $output -ErrorAction Stop
    $receiptPath = Join-Path $output 'RECEIPT.json'
    $receipt = [ordered]@{
        schema = 'hoi4.observed_session_launch.v1'; status = 'preparing'; custody_status = 'unverified'
        started_at_utc = [DateTime]::UtcNow.ToString('o'); finished_at_utc = $null
        game_exe_requested = $GameExe; argv = @($GameArguments); expected_exe_sha256 = $ExpectedExeSha256
        exe_before = $null; exe_after = $null; source_before = $null; source_after = $null
        launch_started_filetime = $null; launch_outcome = 'not_attempted'; possible_surviving_game = $false
        helper = $null; wmi_result = $null; handoff = $null; observation_before_hash = $null
        observation_after_hash = $null; process = $null; error = $null
        claim = 'Launch custody only; no game readiness, campaign, capture, or replay proof.'
    }
    $lease = $null
    try {
        Write-ObservedJson $receiptPath $receipt
        if ($ExpectedExeSha256 -cnotmatch '^[a-fA-F0-9]{64}$') { throw 'ExpectedExeSha256 must contain 64 hexadecimal digits.' }
        Assert-ObservedAbsolutePath $GameExe
        $exe = (Get-Item -LiteralPath $GameExe -ErrorAction Stop).FullName
        if ([IO.Path]::GetExtension($exe) -ine '.exe') { throw 'GameExe must select an executable file.' }
        # Deny replacement/writes for the entire hash/launch/handoff interval.
        $lease = [IO.File]::Open($exe, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
        $receipt.exe_before = Get-ObservedFileIdentity $exe
        if ($receipt.exe_before.sha256 -ine $ExpectedExeSha256) { throw 'Executable SHA256 differs from explicit authority.' }
        $helperPath = Join-Path $PSScriptRoot 'ObservedSession.Launch.ps1'
        $receipt.source_before = @(
            Get-ObservedFileIdentity (Join-Path $PSScriptRoot 'Start-Hoi4ObservedSession.ps1')
            Get-ObservedFileIdentity $helperPath
        )
        $commandLine = ConvertTo-ObservedArgumentLine (@($exe) + @($GameArguments))
        if ($commandLine.Length -gt 32766) { throw 'Windows command line exceeds its length bound.' }
        $receipt.launch_started_filetime = [DateTime]::UtcNow.ToFileTimeUtc().ToString()
        $request = [ordered]@{
            exe_before = $receipt.exe_before; argv = @($GameArguments); command_line = $commandLine
            working_directory = Split-Path -Parent $exe; timeout_seconds = $TimeoutSeconds
            launch_started_filetime = $receipt.launch_started_filetime
        }
        $requestPath = Join-Path $output 'REQUEST.json'
        Write-ObservedJson $requestPath $request
        $receipt.status = 'launching'
        $receipt.launch_outcome = 'unknown'
        $receipt.possible_surviving_game = $true
        Write-ObservedJson $receiptPath $receipt
        $receipt.helper = Invoke-ObservedLaunchHelper $requestPath $helperPath $TimeoutSeconds
        $wmiPath = Join-Path $output 'WMI_RESULT.json'
        if (Test-Path -LiteralPath $wmiPath -PathType Leaf) {
            $receipt.wmi_result = Get-Content -LiteralPath $wmiPath -Raw | ConvertFrom-Json
            Assert-ObservedUInt $receipt.wmi_result.ReturnValue ([uint32]::MaxValue)
            if ($receipt.wmi_result.ReturnValue -eq 0) { $receipt.launch_outcome = 'created' }
            else { $receipt.launch_outcome = 'rejected'; $receipt.possible_surviving_game = $false }
        }
        $handoffPath = Join-Path $output 'PROCESS_HANDOFF.json'
        if (Test-Path -LiteralPath $handoffPath -PathType Leaf) {
            $receipt.handoff = Get-Content -LiteralPath $handoffPath -Raw | ConvertFrom-Json
        }
        if ($receipt.helper.timed_out) { throw [TimeoutException]::new('WMI helper deadline exceeded; game creation may have completed and is not cancelled.') }
        if ($receipt.helper.residual -or $null -ne $receipt.helper.error -or
            $null -eq $receipt.helper.exit_code -or $receipt.helper.exit_code -ne 0) { throw 'WMI helper failed; inspect retained helper evidence.' }
        if ($null -eq $receipt.wmi_result -or $receipt.launch_outcome -ne 'created') { throw 'No successful WMI result.' }
        Assert-ObservedUInt $receipt.wmi_result.ProcessId ([int]::MaxValue) -Nonzero
        $createdPid = [int]$receipt.wmi_result.ProcessId
        $latest = [DateTime]::UtcNow.ToFileTimeUtc().ToString()
        Assert-ObservedProcess $receipt.handoff $createdPid $exe $receipt.launch_started_filetime $latest
        $receipt.observation_before_hash = Get-ObservedProcessIdentity $createdPid
        $receipt.exe_after = Get-ObservedFileIdentity $exe
        $receipt.source_after = @(
            Get-ObservedFileIdentity (Join-Path $PSScriptRoot 'Start-Hoi4ObservedSession.ps1')
            Get-ObservedFileIdentity $helperPath
        )
        $receipt.observation_after_hash = Get-ObservedProcessIdentity $createdPid
        foreach ($observed in @($receipt.observation_before_hash, $receipt.observation_after_hash)) {
            Assert-ObservedProcess $observed $createdPid $exe $receipt.launch_started_filetime $latest
            if ($observed.creation_filetime -cne $receipt.handoff.creation_filetime -or
                $observed.session_id -ne $receipt.handoff.session_id) { throw 'Process incarnation changed across the custody handoff.' }
        }
        foreach ($pair in @(@($receipt.exe_before, $receipt.exe_after),
            @($receipt.source_before[0], $receipt.source_after[0]), @($receipt.source_before[1], $receipt.source_after[1]))) {
            if ($pair[0].sha256 -cne $pair[1].sha256 -or $pair[0].bytes -ne $pair[1].bytes -or
                -not [StringComparer]::OrdinalIgnoreCase.Equals($pair[0].path, $pair[1].path)) { throw 'File identity changed across launch.' }
        }
        $receipt.process = [ordered]@{
            pid = $createdPid; creation_filetime = $receipt.handoff.creation_filetime
            exe_path = $exe; exe_sha256 = $receipt.exe_after.sha256; session_id = $receipt.handoff.session_id
        }
        $receipt.custody_status = 'verified'
        $receipt.status = 'launched'
    } catch {
        $receipt.status = if ($_.Exception -is [TimeoutException]) { 'timed_out' } else { 'failed' }
        $receipt.error = $_.Exception.Message
        throw
    } finally {
        if ($null -ne $lease) { $lease.Dispose() }
        $receipt.finished_at_utc = [DateTime]::UtcNow.ToString('o')
        Write-ObservedJson $receiptPath $receipt
    }
    return [pscustomobject]$receipt
}
