[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)] [string] $RecorderExe,
    [Parameter(Mandatory = $true)] [string] $Hoi4Exe,
    [Parameter(Mandatory = $true)] [string] $PdxUserDir,
    [Parameter(Mandatory = $true)] [string] $CollectorDll,
    [Parameter(Mandatory = $true)] [string] $OutDir,
    [string] $ContinueSave,
    [int] $DurationSeconds = 1800,
    [int] $SampleMilliseconds = 250,
    [string[]] $EarlyLaunchArgument = @('--continuelastsave', '--gdpr-compliant'),
    [string] $Python = 'python'
)

$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'Hoi4ClientDriving.OptIn.ps1')
Assert-Hoi4ClientDrivingOptIn -Caller $MyInvocation.MyCommand.Path

foreach ($path in @($RecorderExe, $Hoi4Exe, $PdxUserDir, $CollectorDll)) {
    if (-not (Test-Path -LiteralPath $path)) {
        throw "Required path does not exist: $path"
    }
}
if ($ContinueSave -and -not (Test-Path -LiteralPath $ContinueSave -PathType Leaf)) {
    throw "Continue-save does not exist: $ContinueSave"
}
if ($DurationSeconds -lt 30) {
    throw 'DurationSeconds must be at least 30.'
}
if ($SampleMilliseconds -lt 10) {
    throw 'SampleMilliseconds must be at least 10.'
}
$observedExeSha256 = (Get-FileHash -LiteralPath $Hoi4Exe -Algorithm SHA256).Hash.ToLowerInvariant()
if ($observedExeSha256 -cne '870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad') {
    throw "Hoi4Exe is not exact a729 authority: $observedExeSha256"
}
$existing = @(Get-Process -Name the game -ErrorAction SilentlyContinue)
if ($existing.Count -ne 0) {
    throw "Refusing a second game instance while the game PID(s) $(@($existing.Id) -join ',') exist; the activation receipt must bind the newly launched exact-a729 process unambiguously."
}

$resolvedCollector = (Resolve-Path -LiteralPath $CollectorDll).Path
$resolvedPdxUserDir = (Resolve-Path -LiteralPath $PdxUserDir).Path
$resolvedOut = [System.IO.Path]::GetFullPath($OutDir)
if (Test-Path -LiteralPath $resolvedOut) {
    if (-not (Test-Path -LiteralPath $resolvedOut -PathType Container) -or
        @(Get-ChildItem -LiteralPath $resolvedOut -Force).Count -ne 0) {
        throw "OutDir must be absent or empty so a stale A33 receipt cannot be selected: $resolvedOut"
    }
} else {
    [System.IO.Directory]::CreateDirectory($resolvedOut) | Out-Null
}
$copiedReceipt = Join-Path $resolvedOut 'a33_sunk_ship_activation_probe.jsonl'
$verification = Join-Path $resolvedOut 'a33_sunk_ship_activation_verification.json'
$captureManifest = Join-Path $resolvedOut 'native_session_manifest.json'
$verifier = Join-Path $PSScriptRoot '..\lanes\sunk_ship_history_live\tools\verify_activation_probe.py'
$verifier = [System.IO.Path]::GetFullPath($verifier)
$continueGamePath = Join-Path $resolvedPdxUserDir 'continue_game.json'
$continueGamePriorExists = Test-Path -LiteralPath $continueGamePath -PathType Leaf
$continueGamePriorBytes = if ($continueGamePriorExists) {
    [System.IO.File]::ReadAllBytes($continueGamePath)
} else {
    $null
}
$continueGameOverridden = $false
$continueGameReplacement = $null

if ($ContinueSave) {
    $continueIndex = -1
    $gdprIndex = -1
    for ($index = 0; $index -lt $EarlyLaunchArgument.Count; $index++) {
        if ($EarlyLaunchArgument[$index] -ieq '--continuelastsave') {
            if ($continueIndex -ge 0) {
                throw '-EarlyLaunchArgument must contain --continuelastsave exactly once.'
            }
            $continueIndex = $index
        }
        if ($EarlyLaunchArgument[$index] -ieq '--gdpr-compliant') {
            if ($gdprIndex -ge 0) {
                throw '-EarlyLaunchArgument must contain --gdpr-compliant exactly once.'
            }
            $gdprIndex = $index
        }
    }
    if ($continueIndex -lt 0) {
        throw '-ContinueSave requires --continuelastsave in -EarlyLaunchArgument.'
    }
    if ($gdprIndex -lt 0) {
        throw '-ContinueSave requires launcher-effective --gdpr-compliant in -EarlyLaunchArgument.'
    }
    if ($gdprIndex -lt $continueIndex) {
        throw '--gdpr-compliant must follow --continuelastsave to match launcher-effective argument order.'
    }
    $resolvedContinueSave = (Resolve-Path -LiteralPath $ContinueSave).Path
    $saveDirectory = [System.IO.Path]::GetFullPath((Join-Path $resolvedPdxUserDir 'save games'))
    $requestedDirectory = [System.IO.Path]::GetFullPath((Split-Path -Parent $resolvedContinueSave))
    if (-not [string]::Equals(
        $saveDirectory.TrimEnd([char]'\'),
        $requestedDirectory.TrimEnd([char]'\'),
        [System.StringComparison]::OrdinalIgnoreCase
    )) {
        throw "-ContinueSave must be a direct child of $saveDirectory"
    }
    $continueObject = if ($continueGamePriorExists) {
        Get-Content -LiteralPath $continueGamePath -Raw | ConvertFrom-Json
    } else {
        [pscustomobject]@{
            title = [System.IO.Path]::GetFileNameWithoutExtension($resolvedContinueSave)
            desc = ''
            date = [DateTime]::UtcNow.ToString('R')
            filename = ''
            is_remote = $false
        }
    }
    $continueObject.filename = [System.IO.Path]::GetFileName($resolvedContinueSave)
    $continueObject.is_remote = $false
    $continueJson = $continueObject | ConvertTo-Json -Depth 8
    $continueGameReplacement = $continueJson + [Environment]::NewLine
}

$arguments = @(
    '--early-launch-exe', (Resolve-Path -LiteralPath $Hoi4Exe).Path,
    '--pdx-user-dir', $resolvedPdxUserDir,
    '--dll', $resolvedCollector,
    '--out-dir', $resolvedOut,
    '--duration-s', $DurationSeconds,
    '--sample-ms', $SampleMilliseconds,
    '--stream-format', 'compact',
    '--profile', 'debug',
    '--scheduled-domain', 'naval_combat_lifecycle_live',
    '--scheduled-domain', 'naval_combat_members_live',
    '--scheduled-domains-exact',
    '--sidecar-log',
    '--hour-latch'
)
foreach ($value in $EarlyLaunchArgument) {
    $arguments += @('--early-launch-arg', $value)
}

$prior = [System.Environment]::GetEnvironmentVariable(
    'BATTLE_PLAN_ENABLE_A33_SUNK_SHIP_EVIDENCE',
    [System.EnvironmentVariableTarget]::Process
)
try {
    if ($null -ne $continueGameReplacement) {
        # Mark the pointer as overridden before writing so a partial write or
        # encoding failure still restores the exact prior bytes in finally.
        $continueGameOverridden = $true
        [System.IO.File]::WriteAllText(
            $continueGamePath,
            $continueGameReplacement,
            [System.Text.UTF8Encoding]::new($false)
        )
    }
    [System.Environment]::SetEnvironmentVariable(
        'BATTLE_PLAN_ENABLE_A33_SUNK_SHIP_EVIDENCE',
        '1',
        [System.EnvironmentVariableTarget]::Process
    )
    & (Resolve-Path -LiteralPath $RecorderExe).Path @arguments
    if ($LASTEXITCODE -ne 0) {
        throw "collector_record exited with code $LASTEXITCODE"
    }
} finally {
    [System.Environment]::SetEnvironmentVariable(
        'BATTLE_PLAN_ENABLE_A33_SUNK_SHIP_EVIDENCE',
        $prior,
        [System.EnvironmentVariableTarget]::Process
    )
    if ($continueGameOverridden) {
        if ($continueGamePriorExists) {
            [System.IO.File]::WriteAllBytes($continueGamePath, $continueGamePriorBytes)
        } elseif (Test-Path -LiteralPath $continueGamePath) {
            Remove-Item -LiteralPath $continueGamePath -Force
        }
    }
}

if (-not (Test-Path -LiteralPath $captureManifest)) {
    throw "Recorder capture manifest was not produced: $captureManifest"
}
$manifestObject = Get-Content -LiteralPath $captureManifest -Raw | ConvertFrom-Json
$receiptCandidates = [System.Collections.Generic.List[string]]::new()
if ($manifestObject.dll) {
    $receiptCandidates.Add((Join-Path (Split-Path -Parent ([string] $manifestObject.dll)) 'a33_sunk_ship_activation_probe.jsonl'))
}
$receiptCandidates.Add((Join-Path (Split-Path -Parent $resolvedCollector) 'a33_sunk_ship_activation_probe.jsonl'))
Get-ChildItem -LiteralPath $resolvedOut -Recurse -File -Filter 'a33_sunk_ship_activation_probe.jsonl' -ErrorAction SilentlyContinue |
    ForEach-Object { $receiptCandidates.Add($_.FullName) }
$rawReceipt = $receiptCandidates |
    Where-Object { Test-Path -LiteralPath $_ } |
    Sort-Object { (Get-Item -LiteralPath $_).LastWriteTimeUtc } -Descending |
    Select-Object -First 1
if (-not $rawReceipt) {
    throw 'A33 raw activation receipt was not produced beside the loaded collector or under the output directory.'
}
$resolvedRawReceipt = [System.IO.Path]::GetFullPath([string] $rawReceipt)
if (-not [string]::Equals(
    $resolvedRawReceipt,
    [System.IO.Path]::GetFullPath($copiedReceipt),
    [System.StringComparison]::OrdinalIgnoreCase
)) {
    Copy-Item -LiteralPath $resolvedRawReceipt -Destination $copiedReceipt -Force
}
& $Python $verifier $copiedReceipt --capture-manifest $captureManifest --collector-dll $resolvedCollector --out $verification
if ($LASTEXITCODE -ne 0) {
    throw "A33 activation receipt was rejected; inspect $verification"
}

Write-Output "A33 activation candidate accepted: $verification"
