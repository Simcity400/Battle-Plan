[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)] [string] $RecorderExe,
    [Parameter(Mandatory = $true)] [string] $Hoi4Exe,
    [Parameter(Mandatory = $true)] [string] $PdxUserDir,
    [Parameter(Mandatory = $true)] [string] $CollectorDll,
    [Parameter(Mandatory = $true)] [string] $OutDir,
    [string] $ContinueSave,
    [int] $DurationSeconds = 300,
    [int] $SampleMilliseconds = 50,
    [string[]] $EarlyLaunchArgument = @('--continuelastsave', '--gdpr-compliant'),
    [string] $Python = 'python'
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$expectedExeSha256 = '870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad'
foreach ($path in @($RecorderExe, $Hoi4Exe, $PdxUserDir, $CollectorDll)) {
    if (-not (Test-Path -LiteralPath $path)) { throw "Required path does not exist: $path" }
}
if ($ContinueSave -and -not (Test-Path -LiteralPath $ContinueSave -PathType Leaf)) {
    throw "Continue-save does not exist: $ContinueSave"
}
if ($DurationSeconds -lt 30 -or $SampleMilliseconds -lt 10) {
    throw 'DurationSeconds must be >=30 and SampleMilliseconds >=10.'
}
if (@(Get-Process -Name the game -ErrorAction SilentlyContinue).Count -ne 0) {
    throw 'C07 acquisition requires a clean early launch; close the existing hoi4.exe first.'
}

$resolvedRecorder = (Resolve-Path -LiteralPath $RecorderExe).Path
$resolvedExe = (Resolve-Path -LiteralPath $Hoi4Exe).Path
$resolvedUserDir = (Resolve-Path -LiteralPath $PdxUserDir).Path
$resolvedCollector = (Resolve-Path -LiteralPath $CollectorDll).Path
$resolvedOut = [System.IO.Path]::GetFullPath($OutDir)
if ((Get-FileHash -LiteralPath $resolvedExe -Algorithm SHA256).Hash.ToLowerInvariant() -cne $expectedExeSha256) {
    throw 'Hoi4Exe does not match exact a729 authority.'
}
if (Test-Path -LiteralPath $resolvedOut) {
    if (-not (Test-Path -LiteralPath $resolvedOut -PathType Container) -or
        @(Get-ChildItem -LiteralPath $resolvedOut -Force).Count -ne 0) {
        throw "OutDir must be absent or empty: $resolvedOut"
    }
} else {
    [System.IO.Directory]::CreateDirectory($resolvedOut) | Out-Null
}

$continueGamePath = Join-Path $resolvedUserDir 'continue_game.json'
$priorExists = Test-Path -LiteralPath $continueGamePath -PathType Leaf
$priorBytes = if ($priorExists) { [System.IO.File]::ReadAllBytes($continueGamePath) } else { $null }
$continueOverridden = $false
if ($ContinueSave) {
    if (@($EarlyLaunchArgument | Where-Object { $_ -ieq '--continuelastsave' }).Count -ne 1 -or
        @($EarlyLaunchArgument | Where-Object { $_ -ieq '--gdpr-compliant' }).Count -ne 1) {
        throw '-ContinueSave requires exactly one --continuelastsave and --gdpr-compliant argument.'
    }
    $resolvedSave = (Resolve-Path -LiteralPath $ContinueSave).Path
    $saveRoot = [System.IO.Path]::GetFullPath((Join-Path $resolvedUserDir 'save games'))
    if (-not [System.IO.Path]::GetDirectoryName($resolvedSave).Equals(
        $saveRoot,
        [System.StringComparison]::OrdinalIgnoreCase
    )) {
        throw "-ContinueSave must be a direct child of $saveRoot"
    }
    $continue = if ($priorExists) {
        Get-Content -LiteralPath $continueGamePath -Raw | ConvertFrom-Json
    } else {
        [pscustomobject]@{
            title = [System.IO.Path]::GetFileNameWithoutExtension($resolvedSave)
            desc = ''
            date = [DateTime]::UtcNow.ToString('R')
            filename = ''
            is_remote = $false
        }
    }
    $continue | Add-Member -MemberType NoteProperty -Name filename `
        -Value ([System.IO.Path]::GetFileName($resolvedSave)) -Force
    $continue | Add-Member -MemberType NoteProperty -Name is_remote -Value $false -Force
    $continueOverridden = $true
    [System.IO.File]::WriteAllText(
        $continueGamePath,
        (($continue | ConvertTo-Json -Depth 8) + [Environment]::NewLine),
        [System.Text.UTF8Encoding]::new($false)
    )
}

try {
$manifest = Join-Path $resolvedOut 'native_session_manifest.json'
$observation = Join-Path $resolvedOut 'c07_o05_generic_building_observation.json'
$verification = Join-Path $resolvedOut 'c07_o05_generic_building_verification.json'
$stdout = Join-Path $resolvedOut 'collector_record.stdout.log'
$stderr = Join-Path $resolvedOut 'collector_record.stderr.log'
$verifier = Join-Path $PSScriptRoot 'verify_c07_o05_acquisition.py'
if (-not (Test-Path -LiteralPath $verifier -PathType Leaf)) {
    throw "C07 regrader is missing: $verifier"
}

$arguments = @(
    '--early-launch-exe', $resolvedExe,
    '--pdx-user-dir', $resolvedUserDir,
    '--dll', $resolvedCollector,
    '--out-dir', $resolvedOut,
    '--duration-s', $DurationSeconds,
    '--sample-ms', $SampleMilliseconds,
    '--stream-format', 'compact',
    '--profile', 'debug',
    '--sidecar-log',
    '--hour-latch'
)
foreach ($value in $EarlyLaunchArgument) { $arguments += @('--early-launch-arg', $value) }
$argumentLine = ($arguments | ForEach-Object {
    $text = [string] $_
    if ($text -match '[\s"]') {
        '"' + ($text -replace '"', '\"') + '"'
    } else {
        $text
    }
}) -join ' '

$startInfo = [System.Diagnostics.ProcessStartInfo]::new()
$startInfo.FileName = $resolvedRecorder
$startInfo.Arguments = $argumentLine
$startInfo.UseShellExecute = $false
$startInfo.CreateNoWindow = $true
$startInfo.RedirectStandardOutput = $true
$startInfo.RedirectStandardError = $true
$startInfo.EnvironmentVariables['BATTLE_PLAN_C07_O05_EVIDENCE_OUT_DIR'] = $resolvedOut
$recorder = [System.Diagnostics.Process]::new()
$recorder.StartInfo = $startInfo
try {
    if (-not $recorder.Start()) { throw 'collector_record did not start.' }
    $stdoutTask = $recorder.StandardOutput.ReadToEndAsync()
    $stderrTask = $recorder.StandardError.ReadToEndAsync()
    $recorder.WaitForExit()
    [System.IO.File]::WriteAllText($stdout, $stdoutTask.Result, [System.Text.UTF8Encoding]::new($false))
    [System.IO.File]::WriteAllText($stderr, $stderrTask.Result, [System.Text.UTF8Encoding]::new($false))
    if ($recorder.ExitCode -ne 0) {
        throw "collector_record exited with code $($recorder.ExitCode); inspect $stderr"
    }
} finally {
    $recorder.Dispose()
}

foreach ($path in @($manifest, $observation)) {
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        throw "C07 acquisition output is missing: $path"
    }
}
& $Python $verifier --observation $observation --capture-manifest $manifest --out $verification
if ($LASTEXITCODE -ne 0) {
    throw "C07 O05 observation was rejected; inspect $verification"
}
Write-Output "C07 O05 selected-Present observation accepted: $verification"
} finally {
    if ($continueOverridden) {
        if ($priorExists) {
            [System.IO.File]::WriteAllBytes($continueGamePath, $priorBytes)
        } elseif (Test-Path -LiteralPath $continueGamePath) {
            Remove-Item -LiteralPath $continueGamePath -Force
        }
    }
}
