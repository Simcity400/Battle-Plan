param(
    [Parameter(Mandatory = $true)][string]$Recording,
    [Parameter(Mandatory = $true)][string]$Evidence,
    [string]$GameDir = $(if (Test-Path Env:BATTLE_PLAN_GAME_DIR) { $env:BATTLE_PLAN_GAME_DIR } else { $env:HOI4_GAME_DIR }),
    [string]$UserDir = $(if (Test-Path Env:BATTLE_PLAN_USER_DIR) { $env:BATTLE_PLAN_USER_DIR } else { $env:HOI4_USER_DIR }),
    [string]$Requests,
    [string]$Prefix,
    [ValidateRange(1, [int]::MaxValue)][int]$Repetitions = 20,
    [string]$Executable,
    [switch]$AllowUnverified,
    [switch]$PrepareOnly
)

$ErrorActionPreference = 'Stop'
$workspace = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$evidencePath = [IO.Path]::GetFullPath($Evidence)
$workspacePrefix = $workspace.TrimEnd('\', '/') + [IO.Path]::DirectorySeparatorChar
if ($evidencePath.Equals($workspace, [StringComparison]::OrdinalIgnoreCase) -or
    $evidencePath.StartsWith($workspacePrefix, [StringComparison]::OrdinalIgnoreCase)) {
    throw 'Evidence and caches must be outside the Git worktree.'
}
if (-not $Executable) {
    $Executable = Join-Path $workspace 'target/release/dashboard-acceptance.exe'
}
$executablePath = (Resolve-Path -LiteralPath $Executable).Path
$recordingPath = (Resolve-Path -LiteralPath $Recording).Path
if ([bool]$GameDir -ne [bool]$UserDir) {
    throw 'Supply both GameDir and UserDir (or BATTLE_PLAN_GAME_DIR and BATTLE_PLAN_USER_DIR), or neither.'
}
$cachePath = Join-Path $evidencePath 'cache'
$localPath = Join-Path $cachePath 'local'
$roamingPath = Join-Path $cachePath 'roaming'
$tempPath = Join-Path $cachePath 'temp'
foreach ($directory in @($evidencePath, $localPath, $roamingPath, $tempPath)) {
    [IO.Directory]::CreateDirectory($directory) | Out-Null
}

# ProcessStartInfo changes the child's environment without altering the user's
# shell. The binary independently verifies these two cache roots before opening.
$start = [Diagnostics.ProcessStartInfo]::new()
$start.FileName = $executablePath
$start.WorkingDirectory = $workspace
$start.UseShellExecute = $false
$start.CreateNoWindow = $true
$start.WindowStyle = [Diagnostics.ProcessWindowStyle]::Hidden
$start.RedirectStandardOutput = $true
$start.RedirectStandardError = $true
$start.EnvironmentVariables['LOCALAPPDATA'] = $localPath
$start.EnvironmentVariables['APPDATA'] = $roamingPath
$start.EnvironmentVariables['TEMP'] = $tempPath
$start.EnvironmentVariables['TMP'] = $tempPath

$arguments = [Collections.Generic.List[string]]::new()
$arguments.Add('--recording')
$arguments.Add($recordingPath)
$arguments.Add('--evidence')
$arguments.Add($evidencePath)
$arguments.Add('--repetitions')
$arguments.Add($Repetitions.ToString([Globalization.CultureInfo]::InvariantCulture))
foreach ($pair in @(
    @('--game-dir', $GameDir), @('--user-dir', $UserDir),
    @('--requests', $Requests), @('--prefix', $Prefix)
)) {
    if ($pair[1]) {
        $arguments.Add($pair[0])
        $arguments.Add((Resolve-Path -LiteralPath $pair[1]).Path)
    }
}
if ($AllowUnverified) { $arguments.Add('--allow-unverified') }
if ($PrepareOnly) { $arguments.Add('--prepare-only') }

# Windows argument quoting works with Windows PowerShell and PowerShell 7;
# ProcessStartInfo.ArgumentList is unavailable in Windows PowerShell.
function ConvertTo-ProcessArgument([string]$Value) {
    '"' + [regex]::Replace([regex]::Replace($Value, '(\\*)"', '$1$1\"'), '(\\+)$', '$1$1') + '"'
}
$start.Arguments = ($arguments | ForEach-Object { ConvertTo-ProcessArgument $_ }) -join ' '
$process = [Diagnostics.Process]::new()
$process.StartInfo = $start
$samplesPath = Join-Path $evidencePath 'process-samples.jsonl'
$samples = [IO.StreamWriter]::new($samplesPath, $false, [Text.UTF8Encoding]::new($false))
$started = [DateTimeOffset]::UtcNow
$peakPrivate = 0L
$peakWorking = 0L
$sampleCount = 0
$startedProcess = $false
try {
    if (-not $process.Start()) { throw 'Could not start acceptance harness.' }
    $startedProcess = $true
    $stdout = $process.StandardOutput.ReadToEndAsync()
    $stderr = $process.StandardError.ReadToEndAsync()
    while (-not $process.WaitForExit(100)) {
        try {
            $process.Refresh()
            $private = $process.PrivateMemorySize64
            $working = $process.WorkingSet64
            $cpuMs = $process.TotalProcessorTime.TotalMilliseconds
        }
        catch {
            if ($process.HasExited) { break }
            throw
        }
        $peakPrivate = [Math]::Max($peakPrivate, $private)
        $peakWorking = [Math]::Max($peakWorking, $working)
        $sampleCount += 1
        $sample = [ordered]@{
            utc = [DateTimeOffset]::UtcNow.ToString('o')
            elapsedMs = ([DateTimeOffset]::UtcNow - $started).TotalMilliseconds
            processId = $process.Id
            privateBytes = $private.ToString([Globalization.CultureInfo]::InvariantCulture)
            workingSetBytes = $working.ToString([Globalization.CultureInfo]::InvariantCulture)
            cpuMs = $cpuMs
        }
        $samples.WriteLine(($sample | ConvertTo-Json -Compress))
    }
    $process.WaitForExit()
    [IO.File]::WriteAllText((Join-Path $evidencePath 'stdout.txt'), $stdout.GetAwaiter().GetResult())
    [IO.File]::WriteAllText((Join-Path $evidencePath 'stderr.txt'), $stderr.GetAwaiter().GetResult())
    $result = [ordered]@{
        executable = $executablePath
        recording = $recordingPath
        startedUtc = $started.ToString('o')
        elapsedMs = ([DateTimeOffset]::UtcNow - $started).TotalMilliseconds
        exitCode = $process.ExitCode
        sampleIntervalMs = 100
        samples = $sampleCount
        peakSampledPrivateBytes = if ($sampleCount) { $peakPrivate.ToString([Globalization.CultureInfo]::InvariantCulture) } else { $null }
        peakSampledWorkingSetBytes = if ($sampleCount) { $peakWorking.ToString([Globalization.CultureInfo]::InvariantCulture) } else { $null }
        scope = 'One process, including production oracle scans; sampled peaks are not an allocation or VRAM bound.'
    }
    [IO.File]::WriteAllText((Join-Path $evidencePath 'process-report.json'), ($result | ConvertTo-Json))
    Get-Content -LiteralPath (Join-Path $evidencePath 'stdout.txt')
    if ($process.ExitCode -ne 0) {
        Get-Content -LiteralPath (Join-Path $evidencePath 'stderr.txt') | Write-Error -ErrorAction Continue
        throw "Acceptance harness exited $($process.ExitCode); inspect $evidencePath."
    }
}
finally {
    $samples.Dispose()
    if ($startedProcess -and -not $process.HasExited) {
        $process.Kill()
        $process.WaitForExit()
    }
    $process.Dispose()
}
