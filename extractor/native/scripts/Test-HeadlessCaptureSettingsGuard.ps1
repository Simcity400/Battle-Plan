<#
.SYNOPSIS
Exercises the settings.txt guard without launching or stopping HOI4.

.DESCRIPTION
Uses temporary Paradox user directories beneath the settings_guard_lane run
directory. Non-dry harness invocations either refuse at sentinel ownership or
fail at a deliberately absent continue_game.json, before any game launch.
#>
[CmdletBinding()]
param(
    [string]$HoldSentinelPath = '',
    [string]$HolderReadyPath = '',
    [ValidateSet('Write', 'Read')]
    [string]$HoldSentinelAccess = 'Write',
    [ValidateSet('None', 'Read', 'ReadWrite')]
    [string]$HoldSentinelShare = 'ReadWrite'
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$testScriptPath = $MyInvocation.MyCommand.Path
if (-not [string]::IsNullOrWhiteSpace($HoldSentinelPath)) {
    if ([string]::IsNullOrWhiteSpace($HolderReadyPath)) {
        throw '-HolderReadyPath is required with -HoldSentinelPath.'
    }
    $sentinelWaitDeadline = [DateTime]::UtcNow.AddSeconds(30)
    while (-not (Test-Path -LiteralPath $HoldSentinelPath -PathType Leaf)) {
        if ([DateTime]::UtcNow -ge $sentinelWaitDeadline) {
            throw "Timed out waiting for sentinel to hold: $HoldSentinelPath"
        }
        Start-Sleep -Milliseconds 50
    }
    $holderStream = [System.IO.File]::Open(
        $HoldSentinelPath,
        [System.IO.FileMode]::Open,
        ([System.IO.FileAccess]$HoldSentinelAccess),
        ([System.IO.FileShare]$HoldSentinelShare)
    )
    try {
        [System.IO.File]::WriteAllText(
            $HolderReadyPath,
            "holder_pid=$PID",
            [System.Text.UTF8Encoding]::new($false)
        )
        Start-Sleep -Seconds 120
    } finally {
        $holderStream.Dispose()
    }
    exit 0
}

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = (Resolve-Path -LiteralPath (Join-Path $scriptDir '..\..\..')).Path
$nativeRoot = Join-Path $repoRoot 'extractor\native'
$harness = Join-Path $scriptDir 'Invoke-HeadlessRawCapture.ps1'
$laneRoot = Join-Path $nativeRoot 'runs\settings_guard_lane'
$powershellExe = @(Get-Command powershell.exe -CommandType Application -ErrorAction Stop)[0].Source
$localAppData = [Environment]::GetFolderPath([Environment+SpecialFolder]::LocalApplicationData)
if ([string]::IsNullOrWhiteSpace($localAppData)) {
    throw 'Test cannot verify the machine-global sentinel root because LocalApplicationData resolved empty.'
}
$machineGlobalGuardRoot = Join-Path (
    Join-Path $localAppData 'battle-plan'
) 'settings_guard'
$testRoot = Join-Path $laneRoot ("test_settings_guard_" + [Guid]::NewGuid().ToString('N'))
$script:assertionFailures = [System.Collections.Generic.List[string]]::new()
$script:sentinelsToClean = [System.Collections.Generic.List[string]]::new()
$script:temporaryHarnessesToClean = [System.Collections.Generic.List[string]]::new()

function Get-SettingsMetadata {
    param([Parameter(Mandatory = $true)][string]$Path)

    $item = Get-Item -LiteralPath $Path -Force
    return [pscustomobject]@{
        IsReadOnly       = [bool]$item.IsReadOnly
        Attributes       = [string]$item.Attributes
        LastWriteTimeUtc = $item.LastWriteTimeUtc.ToString('o')
        Length           = [long]$item.Length
        Sha256           = (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash
    }
}

function Assert-Observed {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][object]$Observed,
        [Parameter(Mandatory = $true)][object]$Expected,
        [Parameter(Mandatory = $true)][bool]$Pass
    )

    Write-Host "assert=$Name observed=$Observed expected=$Expected pass=$Pass"
    if (-not $Pass) {
        $script:assertionFailures.Add("$Name observed=$Observed expected=$Expected")
    }
}

function Assert-MachineGlobalSentinelPath {
    param(
        [Parameter(Mandatory = $true)][string]$CaseName,
        [Parameter(Mandatory = $true)][string]$SentinelPath
    )

    $sentinelFullPath = [System.IO.Path]::GetFullPath($SentinelPath)
    $repoPrefix = [System.IO.Path]::GetFullPath($repoRoot).TrimEnd('\') + '\'
    $globalPrefix = [System.IO.Path]::GetFullPath($machineGlobalGuardRoot).TrimEnd('\') + '\'
    $underRepo = $sentinelFullPath.StartsWith(
        $repoPrefix,
        [System.StringComparison]::OrdinalIgnoreCase
    )
    $underGlobalRoot = $sentinelFullPath.StartsWith(
        $globalPrefix,
        [System.StringComparison]::OrdinalIgnoreCase
    )
    Assert-Observed -Name "$CaseName.sentinel_outside_repo" -Observed $underRepo -Expected $false -Pass (-not $underRepo)
    Assert-Observed -Name "$CaseName.sentinel_under_machine_global_root" -Observed $underGlobalRoot -Expected $true -Pass $underGlobalRoot
}

function Get-LegacySentinelPath {
    param([Parameter(Mandatory = $true)][string]$SettingsPath)

    $hash = [System.Security.Cryptography.SHA256]::Create()
    try {
        $identity = [System.IO.Path]::GetFullPath($SettingsPath).ToUpperInvariant()
        $bytes = [System.Text.Encoding]::UTF8.GetBytes($identity)
        $key = ([System.BitConverter]::ToString(
            $hash.ComputeHash($bytes)
        )).Replace('-', '').ToLowerInvariant()
        return Join-Path $machineGlobalGuardRoot "$key.json"
    } finally {
        $hash.Dispose()
    }
}

function Invoke-HarnessPrelaunch {
    param(
        [Parameter(Mandatory = $true)][string]$CaseName,
        [Parameter(Mandatory = $true)][string]$PdxUserDir,
        [Parameter(Mandatory = $true)][string]$OutDir,
        [switch]$DryRun,
        [switch]$UseRunningGame,
        [string]$HarnessPath = $harness,
        [switch]$UseProductionRoster,
        [string]$ChildWarningAction = ''
    )

    $liveCaptureProcesses = @(Get-Process -Name 'the game', 'collector_record' -ErrorAction SilentlyContinue)
    if ($liveCaptureProcesses.Count -ne 0) {
        $description = @($liveCaptureProcesses | ForEach-Object { "$($_.ProcessName):$($_.Id)" }) -join ','
        throw "SAFETY ABORT before case '$CaseName': capture process(es) are running: $description"
    }
    Write-Host "safety_check=passed case=$CaseName live_capture_processes=0"

    $caseRoot = Split-Path -Parent $PdxUserDir
    $logKey = $CaseName -replace '[^A-Za-z0-9_.-]', '_'
    $stdoutPath = Join-Path $caseRoot "$logKey.stdout.log"
    $stderrPath = Join-Path $caseRoot "$logKey.stderr.log"
    $childArguments = @(
        '-NoProfile',
        '-ExecutionPolicy', 'Bypass',
        '-File', ('"' + $HarnessPath + '"'),
        '-PdxUserDir', ('"' + $PdxUserDir + '"'),
        '-OutDir', ('"' + $OutDir + '"'),
        '-DurationSeconds', '30'
    )
    if (-not $UseProductionRoster) {
        $childArguments += @(
            '-ScheduledDomainOverride',
            'session_header_live'
        )
    }
    if ($DryRun) {
        $childArguments += '-DryRun'
    }
    if ($UseRunningGame) {
        $childArguments += '-UseRunningGame'
    }
    if (-not [string]::IsNullOrWhiteSpace($ChildWarningAction)) {
        $childArguments += @('-WarningAction', $ChildWarningAction)
    }
    $child = Start-Process -FilePath $powershellExe `
        -ArgumentList $childArguments `
        -NoNewWindow -Wait -PassThru `
        -RedirectStandardOutput $stdoutPath `
        -RedirectStandardError $stderrPath
    $stdout = if (Test-Path -LiteralPath $stdoutPath) {
        @(Get-Content -LiteralPath $stdoutPath)
    } else {
        @()
    }
    $stderr = if (Test-Path -LiteralPath $stderrPath) {
        @(Get-Content -LiteralPath $stderrPath)
    } else {
        @()
    }
    $output = @($stdout) + @($stderr)
    $exitCode = [int]$child.ExitCode
    Write-Host "case=$CaseName child_output_begin"
    $stdout | ForEach-Object { Write-Host ([string]$_) }
    if ($stderr.Count -ne 0) {
        $stderrMessageLines = [System.Collections.Generic.List[string]]::new()
        foreach ($line in $stderr) {
            if ([string]$line -match '^At\s*$') {
                break
            }
            $stderrMessageLines.Add([string]$line)
        }
        $stderrMessage = (($stderrMessageLines -join ' ') -replace '\s+', ' ').Trim()
        Write-Host "child_error=$stderrMessage"
    }
    Write-Host "case=$CaseName child_output_end exit_code=$exitCode"
    return [pscustomobject]@{
        ExitCode = $exitCode
        Text     = ($output | ForEach-Object { [string]$_ }) -join "`n"
    }
}

function Get-EmittedSentinelPath {
    param(
        [Parameter(Mandatory = $true)][string]$Text,
        [Parameter(Mandatory = $true)][string]$Action
    )

    $pattern = '(?m)^phase=settings_guard action=' +
        [regex]::Escape($Action) +
        '[^\r\n]* sentinel=(?<path>[^\r\n]+)$'
    $match = [regex]::Match($Text, $pattern)
    if (-not $match.Success) {
        throw "No phase=settings_guard action=$Action line with sentinel=<path> was emitted."
    }
    $path = $match.Groups['path'].Value.Trim()
    Write-Host "parsed_sentinel action=$Action path=$path"
    return $path
}

function Get-ProbedSentinelPath {
    param(
        [Parameter(Mandatory = $true)][string]$CaseName,
        [Parameter(Mandatory = $true)][string]$PdxUserDir,
        [Parameter(Mandatory = $true)][string]$CaseRoot
    )

    $settingsPath = Join-Path $PdxUserDir 'settings.txt'
    $before = Get-SettingsMetadata -Path $settingsPath
    $probeResult = Invoke-HarnessPrelaunch `
        -CaseName "$CaseName.sentinel_probe" `
        -PdxUserDir $PdxUserDir `
        -OutDir (Join-Path $CaseRoot 'probe_out') `
        -DryRun
    $after = Get-SettingsMetadata -Path $settingsPath
    Assert-Observed `
        -Name "$CaseName.probe_expected_failure" `
        -Observed ($probeResult.Text -match 'Launcher continue selection is missing') `
        -Expected $true `
        -Pass ($probeResult.ExitCode -ne 0 -and
            $probeResult.Text -match 'Launcher continue selection is missing')
    $sentinelPath = Get-EmittedSentinelPath -Text $probeResult.Text -Action 'skip'
    $script:sentinelsToClean.Add($sentinelPath)
    Assert-MachineGlobalSentinelPath -CaseName "$CaseName.probe" -SentinelPath $sentinelPath
    Assert-Observed `
        -Name "$CaseName.probe_sentinel_absent" `
        -Observed (Test-Path -LiteralPath $sentinelPath) `
        -Expected $false `
        -Pass (-not (Test-Path -LiteralPath $sentinelPath))
    Assert-Observed -Name "$CaseName.probe_attributes" -Observed $after.Attributes -Expected $before.Attributes -Pass ($after.Attributes -ceq $before.Attributes)
    Assert-Observed -Name "$CaseName.probe_last_write_utc" -Observed $after.LastWriteTimeUtc -Expected $before.LastWriteTimeUtc -Pass ($after.LastWriteTimeUtc -ceq $before.LastWriteTimeUtc)
    Assert-Observed -Name "$CaseName.probe_length" -Observed $after.Length -Expected $before.Length -Pass ($after.Length -eq $before.Length)
    Assert-Observed -Name "$CaseName.probe_sha256" -Observed $after.Sha256 -Expected $before.Sha256 -Pass ($after.Sha256 -ceq $before.Sha256)
    return $sentinelPath
}

function Write-TestSentinel {
    param(
        [Parameter(Mandatory = $true)][string]$SentinelPath,
        [Parameter(Mandatory = $true)][string]$SettingsPath,
        [Parameter(Mandatory = $true)][bool]$PriorReadOnly,
        [Parameter(Mandatory = $true)][int]$OwnerPid,
        [Parameter(Mandatory = $true)][string]$OwnerStartUtc
    )

    New-Item -ItemType Directory -Path (Split-Path -Parent $SentinelPath) -Force | Out-Null
    $record = [ordered]@{
        schema_version  = 2
        settings_path   = [System.IO.Path]::GetFullPath($SettingsPath)
        prior_read_only = $PriorReadOnly
        created_utc     = [DateTime]::UtcNow.AddMinutes(-5).ToString('o')
        owner_pid       = $OwnerPid
        owner_start_utc = $OwnerStartUtc
    }
    [System.IO.File]::WriteAllText(
        $SentinelPath,
        ($record | ConvertTo-Json -Compress),
        [System.Text.UTF8Encoding]::new($false)
    )
}

function Get-ProcessStartUtc {
    param([Parameter(Mandatory = $true)][System.Diagnostics.Process]$Process)

    $Process.Refresh()
    return ([DateTimeOffset]$Process.StartTime).ToUniversalTime().ToString('o')
}

function Start-TestOwner {
    return Start-Process -FilePath $powershellExe `
        -ArgumentList @('-NoProfile', '-Command', 'Start-Sleep -Seconds 120') `
        -NoNewWindow -PassThru
}

function Start-TestSentinelHolder {
    param(
        [Parameter(Mandatory = $true)][string]$SentinelPath,
        [Parameter(Mandatory = $true)][string]$ReadyPath,
        [ValidateSet('Write', 'Read')]
        [string]$Access = 'Write',
        [ValidateSet('None', 'Read', 'ReadWrite')]
        [string]$Share = 'ReadWrite'
    )

    return Start-Process -FilePath $powershellExe `
        -ArgumentList @(
            '-NoProfile',
            '-ExecutionPolicy', 'Bypass',
            '-File', ('"' + $testScriptPath + '"'),
            '-HoldSentinelPath', ('"' + $SentinelPath + '"'),
            '-HolderReadyPath', ('"' + $ReadyPath + '"'),
            '-HoldSentinelAccess', $Access,
            '-HoldSentinelShare', $Share
        ) `
        -NoNewWindow -PassThru
}

function Wait-TestSentinelHolder {
    param(
        [Parameter(Mandatory = $true)][System.Diagnostics.Process]$Process,
        [Parameter(Mandatory = $true)][string]$ReadyPath
    )

    $deadline = [DateTime]::UtcNow.AddSeconds(30)
    while (-not (Test-Path -LiteralPath $ReadyPath -PathType Leaf)) {
        $Process.Refresh()
        if ($Process.HasExited) {
            throw "Sentinel holder PID $($Process.Id) exited before acquiring the handle (exit=$($Process.ExitCode))."
        }
        if ([DateTime]::UtcNow -ge $deadline) {
            throw "Timed out waiting for sentinel holder PID $($Process.Id) to acquire the handle."
        }
        Start-Sleep -Milliseconds 50
    }
}

function Stop-TestOwner {
    param([AllowNull()][System.Diagnostics.Process]$Process)

    if ($null -ne $Process) {
        $Process.Refresh()
        if (-not $Process.HasExited) {
            $Process.Kill()
            [void]$Process.WaitForExit(10000)
        }
    }
}

function New-CleanupFailureHarness {
    $copyPath = Join-Path $scriptDir (
        "Invoke-HeadlessRawCapture.settings-guard-test-" +
        [Guid]::NewGuid().ToString('N') +
        '.ps1'
    )
    $source = [System.IO.File]::ReadAllText($harness)
    $needle = '            Write-Host "phase=settings_guard action=apply prior_read_only=$settingsGuardPriorReadOnly path=$settingsPath sentinel=$settingsGuardSentinel"'
    $injection = @'
            $settingsGuardCleanupTestBlocker = [System.IO.File]::Open(
                $settingsGuardSentinel,
                [System.IO.FileMode]::Open,
                [System.IO.FileAccess]::Read,
                [System.IO.FileShare]::ReadWrite
            )
            throw 'settings-guard-original-error-probe'
'@
    $firstNeedle = $source.IndexOf($needle, [System.StringComparison]::Ordinal)
    $lastNeedle = $source.LastIndexOf($needle, [System.StringComparison]::Ordinal)
    if ($firstNeedle -lt 0 -or $firstNeedle -ne $lastNeedle) {
        throw 'Cannot create cleanup-failure harness: the apply-phase insertion point is missing or ambiguous.'
    }
    $instrumented = $source.Replace(
        $needle,
        $needle + [Environment]::NewLine + $injection.TrimEnd()
    )
    [System.IO.File]::WriteAllText(
        $copyPath,
        $instrumented,
        [System.Text.UTF8Encoding]::new($false)
    )
    $script:temporaryHarnessesToClean.Add($copyPath)
    return $copyPath
}

function Invoke-GateBCase {
    param(
        [Parameter(Mandatory = $true)][string]$CaseName,
        [Parameter(Mandatory = $true)][bool]$PriorReadOnly
    )

    $caseRoot = Join-Path $testRoot $CaseName
    $pdxDir = Join-Path $caseRoot 'pdx'
    $outDir = Join-Path $caseRoot 'out'
    New-Item -ItemType Directory -Path $pdxDir -Force | Out-Null
    $settingsPath = Join-Path $pdxDir 'settings.txt'
    Set-Content -LiteralPath $settingsPath -Value "settings-guard-$CaseName" -Encoding UTF8
    (Get-Item -LiteralPath $settingsPath -Force).IsReadOnly = $PriorReadOnly
    $before = Get-SettingsMetadata -Path $settingsPath

    Write-Host "gate=GATE-B case=$CaseName prior_read_only=$PriorReadOnly settings_path=$settingsPath"
    $attributeEvents = @()
    if (-not $PriorReadOnly) {
        $attributeEventQueue = [System.Collections.Queue]::Synchronized(
            (New-Object System.Collections.Queue)
        )
        $watcher = New-Object System.IO.FileSystemWatcher
        $watcher.Path = $pdxDir
        $watcher.Filter = [System.IO.Path]::GetFileName($settingsPath)
        $watcher.IncludeSubdirectories = $false
        $watcher.NotifyFilter = [System.IO.NotifyFilters]::Attributes
        $sourceIdentifier = "settings_guard_attribute_" + [Guid]::NewGuid().ToString('N')
        $eventSubscription = Register-ObjectEvent `
            -InputObject $watcher `
            -EventName Changed `
            -SourceIdentifier $sourceIdentifier `
            -MessageData $attributeEventQueue `
            -Action {
                $event.MessageData.Enqueue([pscustomobject]@{
                    change_type   = [string]$event.SourceEventArgs.ChangeType
                    name          = [string]$event.SourceEventArgs.Name
                    full_path     = [string]$event.SourceEventArgs.FullPath
                    observed_utc  = [DateTime]::UtcNow.ToString('o')
                })
            }
        try {
            $watcher.EnableRaisingEvents = $true
            $result = Invoke-HarnessPrelaunch -CaseName $CaseName -PdxUserDir $pdxDir -OutDir $outDir
            $eventDeadline = [DateTime]::UtcNow.AddSeconds(5)
            while ($attributeEventQueue.Count -eq 0 -and [DateTime]::UtcNow -lt $eventDeadline) {
                Start-Sleep -Milliseconds 50
            }
            if ($attributeEventQueue.Count -ne 0) {
                # Allow the release notification to join the apply notification.
                Start-Sleep -Milliseconds 250
            }
            $attributeEvents = @($attributeEventQueue.ToArray())
        } finally {
            $watcher.EnableRaisingEvents = $false
            Unregister-Event -SourceIdentifier $sourceIdentifier -ErrorAction SilentlyContinue
            Remove-Job -Id $eventSubscription.Id -Force -ErrorAction SilentlyContinue
            $watcher.Dispose()
        }
        $attributeEventsJson = $attributeEvents | ConvertTo-Json -Compress -Depth 4
        Write-Host "observed_attribute_events case=$CaseName count=$($attributeEvents.Count) events=$attributeEventsJson"
    } else {
        # An already-read-only file has no apply-side attribute transition, so
        # a watcher assertion here would be a flake rather than a guard witness.
        $result = Invoke-HarnessPrelaunch -CaseName $CaseName -PdxUserDir $pdxDir -OutDir $outDir
    }
    $after = Get-SettingsMetadata -Path $settingsPath
    $applySentinel = Get-EmittedSentinelPath -Text $result.Text -Action 'apply'
    $releaseSentinel = Get-EmittedSentinelPath -Text $result.Text -Action 'release'
    $script:sentinelsToClean.Add($applySentinel)
    Assert-MachineGlobalSentinelPath -CaseName $CaseName -SentinelPath $applySentinel

    Assert-Observed -Name "$CaseName.exit_nonzero" -Observed $result.ExitCode -Expected 'nonzero' -Pass ($result.ExitCode -ne 0)
    Assert-Observed -Name "$CaseName.expected_failure" -Observed ($result.Text -match 'Launcher continue selection is missing') -Expected $true -Pass ($result.Text -match 'Launcher continue selection is missing')
    Assert-Observed -Name "$CaseName.apply_prior" -Observed ($result.Text -match "phase=settings_guard action=apply prior_read_only=$PriorReadOnly") -Expected $true -Pass ($result.Text -match "phase=settings_guard action=apply prior_read_only=$PriorReadOnly")
    if (-not $PriorReadOnly) {
        Assert-Observed -Name "$CaseName.attribute_change_witness" -Observed $attributeEvents.Count -Expected 'at least 1' -Pass ($attributeEvents.Count -ge 1)
    }
    Assert-Observed -Name "$CaseName.phase_sentinel_match" -Observed $releaseSentinel -Expected $applySentinel -Pass ([System.StringComparer]::OrdinalIgnoreCase.Equals($releaseSentinel, $applySentinel))
    Assert-Observed -Name "$CaseName.restored_read_only" -Observed $after.IsReadOnly -Expected $PriorReadOnly -Pass ($after.IsReadOnly -eq $PriorReadOnly)
    Assert-Observed -Name "$CaseName.attributes" -Observed $after.Attributes -Expected $before.Attributes -Pass ($after.Attributes -ceq $before.Attributes)
    Assert-Observed -Name "$CaseName.last_write_utc" -Observed $after.LastWriteTimeUtc -Expected $before.LastWriteTimeUtc -Pass ($after.LastWriteTimeUtc -ceq $before.LastWriteTimeUtc)
    Assert-Observed -Name "$CaseName.length" -Observed $after.Length -Expected $before.Length -Pass ($after.Length -eq $before.Length)
    Assert-Observed -Name "$CaseName.sha256" -Observed $after.Sha256 -Expected $before.Sha256 -Pass ($after.Sha256 -ceq $before.Sha256)
    Assert-Observed -Name "$CaseName.sentinel_absent" -Observed (Test-Path -LiteralPath $applySentinel) -Expected $false -Pass (-not (Test-Path -LiteralPath $applySentinel))
}

function Invoke-GateC {
    $caseName = 'sentinel_recovery'
    $caseRoot = Join-Path $testRoot $caseName
    $pdxDir = Join-Path $caseRoot 'pdx'
    $outDir = Join-Path $caseRoot 'out'
    New-Item -ItemType Directory -Path $pdxDir -Force | Out-Null
    $settingsPath = Join-Path $pdxDir 'settings.txt'
    Set-Content -LiteralPath $settingsPath -Value 'settings-guard-sentinel-recovery' -Encoding UTF8
    $sentinelPath = Get-ProbedSentinelPath -CaseName $caseName -PdxUserDir $pdxDir -CaseRoot $caseRoot
    $settingsItem = Get-Item -LiteralPath $settingsPath -Force
    $settingsItem.IsReadOnly = $true
    $before = Get-SettingsMetadata -Path $settingsPath
    Write-TestSentinel `
        -SentinelPath $sentinelPath `
        -SettingsPath $settingsItem.FullName `
        -PriorReadOnly $false `
        -OwnerPid 2147483647 `
        -OwnerStartUtc ([DateTimeOffset]::UtcNow.AddMinutes(-5).ToString('o'))

    Write-Host "gate=GATE-C case=$caseName observed_before_read_only=$($before.IsReadOnly) recorded_prior_read_only=False settings_path=$settingsPath sentinel_path=$sentinelPath"
    $result = Invoke-HarnessPrelaunch -CaseName $caseName -PdxUserDir $pdxDir -OutDir $outDir
    $after = Get-SettingsMetadata -Path $settingsPath
    $recoverSentinel = Get-EmittedSentinelPath -Text $result.Text -Action 'recover'
    $applySentinel = Get-EmittedSentinelPath -Text $result.Text -Action 'apply'
    $releaseSentinel = Get-EmittedSentinelPath -Text $result.Text -Action 'release'

    Assert-Observed -Name "$caseName.exit_nonzero" -Observed $result.ExitCode -Expected 'nonzero' -Pass ($result.ExitCode -ne 0)
    Assert-Observed -Name "$caseName.expected_failure" -Observed ($result.Text -match 'Launcher continue selection is missing') -Expected $true -Pass ($result.Text -match 'Launcher continue selection is missing')
    Assert-Observed -Name "$caseName.warning" -Observed ($result.Text -match 'Interrupted settings guard detected') -Expected $true -Pass ($result.Text -match 'Interrupted settings guard detected')
    Assert-Observed -Name "$caseName.owner_status" -Observed ($result.Text -match 'action=recover owner_status=no_live_holder') -Expected $true -Pass ($result.Text -match 'action=recover owner_status=no_live_holder')
    Assert-Observed -Name "$caseName.apply_used_recorded_prior" -Observed ($result.Text -match 'phase=settings_guard action=apply prior_read_only=False') -Expected $true -Pass ($result.Text -match 'phase=settings_guard action=apply prior_read_only=False')
    Assert-Observed -Name "$caseName.recover_sentinel" -Observed $recoverSentinel -Expected $sentinelPath -Pass ([System.StringComparer]::OrdinalIgnoreCase.Equals($recoverSentinel, $sentinelPath))
    Assert-Observed -Name "$caseName.apply_sentinel" -Observed $applySentinel -Expected $sentinelPath -Pass ([System.StringComparer]::OrdinalIgnoreCase.Equals($applySentinel, $sentinelPath))
    Assert-Observed -Name "$caseName.release_sentinel" -Observed $releaseSentinel -Expected $sentinelPath -Pass ([System.StringComparer]::OrdinalIgnoreCase.Equals($releaseSentinel, $sentinelPath))
    Assert-Observed -Name "$caseName.restored_recorded_prior" -Observed $after.IsReadOnly -Expected $false -Pass (-not $after.IsReadOnly)
    Assert-Observed -Name "$caseName.last_write_utc" -Observed $after.LastWriteTimeUtc -Expected $before.LastWriteTimeUtc -Pass ($after.LastWriteTimeUtc -ceq $before.LastWriteTimeUtc)
    Assert-Observed -Name "$caseName.length" -Observed $after.Length -Expected $before.Length -Pass ($after.Length -eq $before.Length)
    Assert-Observed -Name "$caseName.sha256" -Observed $after.Sha256 -Expected $before.Sha256 -Pass ($after.Sha256 -ceq $before.Sha256)
    Assert-Observed -Name "$caseName.sentinel_absent" -Observed (Test-Path -LiteralPath $sentinelPath) -Expected $false -Pass (-not (Test-Path -LiteralPath $sentinelPath))
}

function Invoke-SameHostStaleCase {
    $caseName = 'same_host_stale'
    $caseRoot = Join-Path $testRoot $caseName
    $pdxDir = Join-Path $caseRoot 'pdx'
    $outDir = Join-Path $caseRoot 'out'
    New-Item -ItemType Directory -Path $pdxDir -Force | Out-Null
    $settingsPath = Join-Path $pdxDir 'settings.txt'
    Set-Content -LiteralPath $settingsPath -Value 'settings-guard-same-host-stale' -Encoding UTF8
    $sentinelPath = Get-ProbedSentinelPath -CaseName $caseName -PdxUserDir $pdxDir -CaseRoot $caseRoot
    $testHost = Get-Process -Id $PID -ErrorAction Stop
    $ownerStartUtc = Get-ProcessStartUtc -Process $testHost
    (Get-Item -LiteralPath $settingsPath -Force).IsReadOnly = $true
    $before = Get-SettingsMetadata -Path $settingsPath
    Write-TestSentinel `
        -SentinelPath $sentinelPath `
        -SettingsPath $settingsPath `
        -PriorReadOnly $false `
        -OwnerPid $PID `
        -OwnerStartUtc $ownerStartUtc

    Write-Host "gate=SAME-HOST-STALE owner_pid=$PID owner_start_utc=$ownerStartUtc live_handle=False settings_path=$settingsPath sentinel_path=$sentinelPath"
    $result = Invoke-HarnessPrelaunch -CaseName $caseName -PdxUserDir $pdxDir -OutDir $outDir
    $after = Get-SettingsMetadata -Path $settingsPath
    $recoverSentinel = Get-EmittedSentinelPath -Text $result.Text -Action 'recover'

    Assert-Observed -Name "$caseName.exit_nonzero" -Observed $result.ExitCode -Expected 'nonzero' -Pass ($result.ExitCode -ne 0)
    Assert-Observed -Name "$caseName.recovered" -Observed ($result.Text -match 'action=recover owner_status=no_live_holder') -Expected $true -Pass ($result.Text -match 'action=recover owner_status=no_live_holder')
    Assert-Observed -Name "$caseName.no_incompatible_holder_refusal" -Observed ($result.Text -match 'incompatible live holder') -Expected $false -Pass ($result.Text -notmatch 'incompatible live holder')
    Assert-Observed -Name "$caseName.apply" -Observed ($result.Text -match 'action=apply prior_read_only=False') -Expected $true -Pass ($result.Text -match 'action=apply prior_read_only=False')
    Assert-Observed -Name "$caseName.recover_sentinel" -Observed $recoverSentinel -Expected $sentinelPath -Pass ([System.StringComparer]::OrdinalIgnoreCase.Equals($recoverSentinel, $sentinelPath))
    Assert-Observed -Name "$caseName.restored_recorded_prior" -Observed $after.IsReadOnly -Expected $false -Pass (-not $after.IsReadOnly)
    Assert-Observed -Name "$caseName.last_write_utc" -Observed $after.LastWriteTimeUtc -Expected $before.LastWriteTimeUtc -Pass ($after.LastWriteTimeUtc -ceq $before.LastWriteTimeUtc)
    Assert-Observed -Name "$caseName.length" -Observed $after.Length -Expected $before.Length -Pass ($after.Length -eq $before.Length)
    Assert-Observed -Name "$caseName.sha256" -Observed $after.Sha256 -Expected $before.Sha256 -Pass ($after.Sha256 -ceq $before.Sha256)
    Assert-Observed -Name "$caseName.sentinel_absent" -Observed (Test-Path -LiteralPath $sentinelPath) -Expected $false -Pass (-not (Test-Path -LiteralPath $sentinelPath))
}

function Invoke-LiveOwnerCase {
    $caseName = 'live_owner'
    $caseRoot = Join-Path $testRoot $caseName
    $pdxDir = Join-Path $caseRoot 'pdx'
    $outDir = Join-Path $caseRoot 'out'
    New-Item -ItemType Directory -Path $pdxDir -Force | Out-Null
    $settingsPath = Join-Path $pdxDir 'settings.txt'
    Set-Content -LiteralPath $settingsPath -Value 'settings-guard-live-owner' -Encoding UTF8
    $sentinelPath = Get-ProbedSentinelPath -CaseName $caseName -PdxUserDir $pdxDir -CaseRoot $caseRoot
    $holderReadyPath = Join-Path $caseRoot 'holder.ready'
    $ownerProcess = $null
    try {
        $ownerProcess = Start-TestSentinelHolder `
            -SentinelPath $sentinelPath `
            -ReadyPath $holderReadyPath
        $ownerStartUtc = Get-ProcessStartUtc -Process $ownerProcess
        (Get-Item -LiteralPath $settingsPath -Force).IsReadOnly = $true
        $before = Get-SettingsMetadata -Path $settingsPath
        Write-TestSentinel `
            -SentinelPath $sentinelPath `
            -SettingsPath $settingsPath `
            -PriorReadOnly $false `
            -OwnerPid $ownerProcess.Id `
            -OwnerStartUtc $ownerStartUtc
        Wait-TestSentinelHolder -Process $ownerProcess -ReadyPath $holderReadyPath

        Write-Host "gate=LIVE-OWNER owner_pid=$($ownerProcess.Id) owner_start_utc=$ownerStartUtc live_handle=True settings_path=$settingsPath sentinel_path=$sentinelPath"
        $result = Invoke-HarnessPrelaunch -CaseName $caseName -PdxUserDir $pdxDir -OutDir $outDir
        $after = Get-SettingsMetadata -Path $settingsPath
        $compactedOutput = $result.Text -replace '\s', ''
        $compactedSentinel = $sentinelPath -replace '\s', ''
        $normalizedOutput = ($result.Text -replace '\s+', ' ').Trim()

        Assert-Observed -Name "$caseName.exit_nonzero" -Observed $result.ExitCode -Expected 'nonzero' -Pass ($result.ExitCode -ne 0)
        Assert-Observed -Name "$caseName.incompatible_holder_refusal" -Observed ($normalizedOutput -match 'incompatible live holder') -Expected $true -Pass ($normalizedOutput -match 'incompatible live holder')
        Assert-Observed -Name "$caseName.holder_kinds_named" -Observed ($normalizedOutput -match 'capture writer or an exclusive reader') -Expected $true -Pass ($normalizedOutput -match 'capture writer or an exclusive reader')
        Assert-Observed -Name "$caseName.retry_confirmation" -Observed ($result.Text -match 'after 3 recovery-claim attempts') -Expected $true -Pass ($result.Text -match 'after 3 recovery-claim attempts')
        Assert-Observed -Name "$caseName.writer_pid_named" -Observed ($result.Text -match "recorded writer PID $($ownerProcess.Id)") -Expected $true -Pass ($result.Text -match "recorded writer PID $($ownerProcess.Id)")
        Assert-Observed -Name "$caseName.sentinel_named" -Observed ($compactedOutput -match [regex]::Escape($compactedSentinel)) -Expected $true -Pass ($compactedOutput -match [regex]::Escape($compactedSentinel))
        Assert-Observed -Name "$caseName.no_recover" -Observed ($result.Text -match 'action=recover') -Expected $false -Pass ($result.Text -notmatch 'action=recover')
        Assert-Observed -Name "$caseName.no_apply" -Observed ($result.Text -match 'action=apply') -Expected $false -Pass ($result.Text -notmatch 'action=apply')
        Assert-Observed -Name "$caseName.still_read_only" -Observed $after.IsReadOnly -Expected $true -Pass $after.IsReadOnly
        Assert-Observed -Name "$caseName.attributes" -Observed $after.Attributes -Expected $before.Attributes -Pass ($after.Attributes -ceq $before.Attributes)
        Assert-Observed -Name "$caseName.last_write_utc" -Observed $after.LastWriteTimeUtc -Expected $before.LastWriteTimeUtc -Pass ($after.LastWriteTimeUtc -ceq $before.LastWriteTimeUtc)
        Assert-Observed -Name "$caseName.sha256" -Observed $after.Sha256 -Expected $before.Sha256 -Pass ($after.Sha256 -ceq $before.Sha256)
        Assert-Observed -Name "$caseName.sentinel_present" -Observed (Test-Path -LiteralPath $sentinelPath) -Expected $true -Pass (Test-Path -LiteralPath $sentinelPath)
    } finally {
        Stop-TestOwner -Process $ownerProcess
    }
}

function Invoke-BenignReaderCase {
    $caseName = 'benign_reader'
    $caseRoot = Join-Path $testRoot $caseName
    $pdxDir = Join-Path $caseRoot 'pdx'
    $outDir = Join-Path $caseRoot 'out'
    New-Item -ItemType Directory -Path $pdxDir -Force | Out-Null
    $settingsPath = Join-Path $pdxDir 'settings.txt'
    Set-Content -LiteralPath $settingsPath -Value 'settings-guard-benign-reader' -Encoding UTF8
    $sentinelPath = Get-ProbedSentinelPath -CaseName $caseName -PdxUserDir $pdxDir -CaseRoot $caseRoot
    $readerReadyPath = Join-Path $caseRoot 'reader.ready'
    $readerProcess = $null
    try {
        $readerProcess = Start-TestSentinelHolder `
            -SentinelPath $sentinelPath `
            -ReadyPath $readerReadyPath `
            -Access 'Read'
        $readerStartUtc = Get-ProcessStartUtc -Process $readerProcess
        (Get-Item -LiteralPath $settingsPath -Force).IsReadOnly = $true
        $before = Get-SettingsMetadata -Path $settingsPath
        Write-TestSentinel `
            -SentinelPath $sentinelPath `
            -SettingsPath $settingsPath `
            -PriorReadOnly $false `
            -OwnerPid $readerProcess.Id `
            -OwnerStartUtc $readerStartUtc
        Wait-TestSentinelHolder -Process $readerProcess -ReadyPath $readerReadyPath

        Write-Host "gate=BENIGN-READER reader_pid=$($readerProcess.Id) reader_start_utc=$readerStartUtc access=Read share=ReadWrite settings_path=$settingsPath sentinel_path=$sentinelPath"
        $result = Invoke-HarnessPrelaunch -CaseName $caseName -PdxUserDir $pdxDir -OutDir $outDir
        $after = Get-SettingsMetadata -Path $settingsPath
        $readerProcess.Refresh()
        $recoverSentinel = Get-EmittedSentinelPath -Text $result.Text -Action 'recover'

        $retainedSentinelStop = $result.Text -match '(?s)Settings guard recovery restored IsReadOnly=False.+retained sentinel.+prevents a new guard'
        Assert-Observed -Name "$caseName.exit_nonzero" -Observed $result.ExitCode -Expected 'nonzero' -Pass ($result.ExitCode -ne 0)
        Assert-Observed -Name "$caseName.retained_sentinel_stop_diagnostic" -Observed $retainedSentinelStop -Expected $true -Pass $retainedSentinelStop
        Assert-Observed -Name "$caseName.recovered_as_readers_only" -Observed ($result.Text -match 'action=recover owner_status=no_live_holder claim_status=readers_only') -Expected $true -Pass ($result.Text -match 'action=recover owner_status=no_live_holder claim_status=readers_only')
        Assert-Observed -Name "$caseName.recovery_recorded_retained_sentinel" -Observed ($result.Text -match 'action=recover[^\r\n]+sentinel_removed=False') -Expected $true -Pass ($result.Text -match 'action=recover[^\r\n]+sentinel_removed=False')
        Assert-Observed -Name "$caseName.no_incompatible_holder_refusal" -Observed ($result.Text -match 'incompatible live holder') -Expected $false -Pass ($result.Text -notmatch 'incompatible live holder')
        Assert-Observed -Name "$caseName.no_apply" -Observed ($result.Text -match 'action=apply') -Expected $false -Pass ($result.Text -notmatch 'action=apply')
        Assert-Observed -Name "$caseName.recover_sentinel" -Observed $recoverSentinel -Expected $sentinelPath -Pass ([System.StringComparer]::OrdinalIgnoreCase.Equals($recoverSentinel, $sentinelPath))
        Assert-Observed -Name "$caseName.reader_still_holding_after_recovery" -Observed (-not $readerProcess.HasExited) -Expected $true -Pass (-not $readerProcess.HasExited)
        Assert-Observed -Name "$caseName.restored_recorded_prior" -Observed $after.IsReadOnly -Expected $false -Pass (-not $after.IsReadOnly)
        Assert-Observed -Name "$caseName.last_write_utc" -Observed $after.LastWriteTimeUtc -Expected $before.LastWriteTimeUtc -Pass ($after.LastWriteTimeUtc -ceq $before.LastWriteTimeUtc)
        Assert-Observed -Name "$caseName.length" -Observed $after.Length -Expected $before.Length -Pass ($after.Length -eq $before.Length)
        Assert-Observed -Name "$caseName.sha256" -Observed $after.Sha256 -Expected $before.Sha256 -Pass ($after.Sha256 -ceq $before.Sha256)
        Assert-Observed -Name "$caseName.sentinel_retained" -Observed (Test-Path -LiteralPath $sentinelPath) -Expected $true -Pass (Test-Path -LiteralPath $sentinelPath)
    } finally {
        Stop-TestOwner -Process $readerProcess
    }
}

function Invoke-DeadOwnerCase {
    $caseName = 'dead_owner'
    $caseRoot = Join-Path $testRoot $caseName
    $pdxDir = Join-Path $caseRoot 'pdx'
    $outDir = Join-Path $caseRoot 'out'
    New-Item -ItemType Directory -Path $pdxDir -Force | Out-Null
    $settingsPath = Join-Path $pdxDir 'settings.txt'
    Set-Content -LiteralPath $settingsPath -Value 'settings-guard-dead-owner' -Encoding UTF8
    $sentinelPath = Get-ProbedSentinelPath -CaseName $caseName -PdxUserDir $pdxDir -CaseRoot $caseRoot
    $ownerProcess = Start-TestOwner
    $ownerPid = $ownerProcess.Id
    $ownerStartUtc = Get-ProcessStartUtc -Process $ownerProcess
    Stop-TestOwner -Process $ownerProcess
    $ownerExists = $null -ne (Get-Process -Id $ownerPid -ErrorAction SilentlyContinue)
    Assert-Observed -Name "$caseName.owner_absent_before_run" -Observed $ownerExists -Expected $false -Pass (-not $ownerExists)

    (Get-Item -LiteralPath $settingsPath -Force).IsReadOnly = $true
    Write-TestSentinel `
        -SentinelPath $sentinelPath `
        -SettingsPath $settingsPath `
        -PriorReadOnly $false `
        -OwnerPid $ownerPid `
        -OwnerStartUtc $ownerStartUtc
    Write-Host "gate=DEAD-OWNER owner_pid=$ownerPid owner_start_utc=$ownerStartUtc settings_path=$settingsPath sentinel_path=$sentinelPath"
    $result = Invoke-HarnessPrelaunch -CaseName $caseName -PdxUserDir $pdxDir -OutDir $outDir
    $after = Get-SettingsMetadata -Path $settingsPath
    $recoverSentinel = Get-EmittedSentinelPath -Text $result.Text -Action 'recover'

    Assert-Observed -Name "$caseName.exit_nonzero" -Observed $result.ExitCode -Expected 'nonzero' -Pass ($result.ExitCode -ne 0)
    Assert-Observed -Name "$caseName.no_live_holder_recovery" -Observed ($result.Text -match 'action=recover owner_status=no_live_holder') -Expected $true -Pass ($result.Text -match 'action=recover owner_status=no_live_holder')
    Assert-Observed -Name "$caseName.apply" -Observed ($result.Text -match 'action=apply prior_read_only=False') -Expected $true -Pass ($result.Text -match 'action=apply prior_read_only=False')
    Assert-Observed -Name "$caseName.recover_sentinel" -Observed $recoverSentinel -Expected $sentinelPath -Pass ([System.StringComparer]::OrdinalIgnoreCase.Equals($recoverSentinel, $sentinelPath))
    Assert-Observed -Name "$caseName.restored_recorded_prior" -Observed $after.IsReadOnly -Expected $false -Pass (-not $after.IsReadOnly)
    Assert-Observed -Name "$caseName.sentinel_absent" -Observed (Test-Path -LiteralPath $sentinelPath) -Expected $false -Pass (-not (Test-Path -LiteralPath $sentinelPath))
}

function Invoke-RecycledOwnerCase {
    $caseName = 'recycled_pid'
    $caseRoot = Join-Path $testRoot $caseName
    $pdxDir = Join-Path $caseRoot 'pdx'
    $outDir = Join-Path $caseRoot 'out'
    New-Item -ItemType Directory -Path $pdxDir -Force | Out-Null
    $settingsPath = Join-Path $pdxDir 'settings.txt'
    Set-Content -LiteralPath $settingsPath -Value 'settings-guard-recycled-pid' -Encoding UTF8
    $sentinelPath = Get-ProbedSentinelPath -CaseName $caseName -PdxUserDir $pdxDir -CaseRoot $caseRoot
    $ownerProcess = $null
    try {
        $ownerProcess = Start-TestOwner
        $liveStartUtc = [DateTimeOffset]::Parse(
            (Get-ProcessStartUtc -Process $ownerProcess)
        )
        $recordedStartUtc = $liveStartUtc.AddMinutes(-5).ToUniversalTime().ToString('o')
        (Get-Item -LiteralPath $settingsPath -Force).IsReadOnly = $true
        Write-TestSentinel `
            -SentinelPath $sentinelPath `
            -SettingsPath $settingsPath `
            -PriorReadOnly $false `
            -OwnerPid $ownerProcess.Id `
            -OwnerStartUtc $recordedStartUtc

        Write-Host "gate=RECYCLED-PID live_pid=$($ownerProcess.Id) live_start_utc=$($liveStartUtc.ToString('o')) recorded_start_utc=$recordedStartUtc settings_path=$settingsPath sentinel_path=$sentinelPath"
        $result = Invoke-HarnessPrelaunch -CaseName $caseName -PdxUserDir $pdxDir -OutDir $outDir
        $after = Get-SettingsMetadata -Path $settingsPath
        $recoverSentinel = Get-EmittedSentinelPath -Text $result.Text -Action 'recover'

        Assert-Observed -Name "$caseName.exit_nonzero" -Observed $result.ExitCode -Expected 'nonzero' -Pass ($result.ExitCode -ne 0)
        Assert-Observed -Name "$caseName.no_live_holder_recovery" -Observed ($result.Text -match 'action=recover owner_status=no_live_holder') -Expected $true -Pass ($result.Text -match 'action=recover owner_status=no_live_holder')
        Assert-Observed -Name "$caseName.apply" -Observed ($result.Text -match 'action=apply prior_read_only=False') -Expected $true -Pass ($result.Text -match 'action=apply prior_read_only=False')
        Assert-Observed -Name "$caseName.recover_sentinel" -Observed $recoverSentinel -Expected $sentinelPath -Pass ([System.StringComparer]::OrdinalIgnoreCase.Equals($recoverSentinel, $sentinelPath))
        Assert-Observed -Name "$caseName.restored_recorded_prior" -Observed $after.IsReadOnly -Expected $false -Pass (-not $after.IsReadOnly)
        Assert-Observed -Name "$caseName.sentinel_absent" -Observed (Test-Path -LiteralPath $sentinelPath) -Expected $false -Pass (-not (Test-Path -LiteralPath $sentinelPath))
    } finally {
        Stop-TestOwner -Process $ownerProcess
    }
}

function Invoke-UseRunningGameCase {
    $caseName = 'use_running_game'
    $caseRoot = Join-Path $testRoot $caseName
    $pdxDir = Join-Path $caseRoot 'pdx'
    $outDir = Join-Path $caseRoot 'out'
    New-Item -ItemType Directory -Path $pdxDir -Force | Out-Null
    $settingsPath = Join-Path $pdxDir 'settings.txt'
    Set-Content -LiteralPath $settingsPath -Value 'settings-guard-use-running-game' -Encoding UTF8
    $sentinelPath = Get-ProbedSentinelPath -CaseName $caseName -PdxUserDir $pdxDir -CaseRoot $caseRoot
    $testHost = Get-Process -Id $PID -ErrorAction Stop
    $ownerStartUtc = Get-ProcessStartUtc -Process $testHost
    (Get-Item -LiteralPath $settingsPath -Force).IsReadOnly = $true
    $before = Get-SettingsMetadata -Path $settingsPath
    Write-TestSentinel `
        -SentinelPath $sentinelPath `
        -SettingsPath $settingsPath `
        -PriorReadOnly $false `
        -OwnerPid $PID `
        -OwnerStartUtc $ownerStartUtc

    Write-Host "gate=USE-RUNNING-GAME recoverable_sentinel=True settings_path=$settingsPath sentinel_path=$sentinelPath"
    $result = Invoke-HarnessPrelaunch `
        -CaseName $caseName `
        -PdxUserDir $pdxDir `
        -OutDir $outDir `
        -UseRunningGame
    $after = Get-SettingsMetadata -Path $settingsPath
    $recoverSentinel = Get-EmittedSentinelPath -Text $result.Text -Action 'recover'
    $skipSentinel = Get-EmittedSentinelPath -Text $result.Text -Action 'skip'

    Assert-Observed -Name "$caseName.exit_nonzero" -Observed $result.ExitCode -Expected 'nonzero' -Pass ($result.ExitCode -ne 0)
    Assert-Observed -Name "$caseName.expected_failure" -Observed ($result.Text -match '-UseRunningGame needs exactly one hoi4.exe') -Expected $true -Pass ($result.Text -match '-UseRunningGame needs exactly one hoi4.exe')
    Assert-Observed -Name "$caseName.recovery_ran" -Observed ($result.Text -match 'action=recover owner_status=no_live_holder') -Expected $true -Pass ($result.Text -match 'action=recover owner_status=no_live_holder')
    Assert-Observed -Name "$caseName.skip_phase" -Observed ($result.Text -match 'action=skip reason=use_running_game') -Expected $true -Pass ($result.Text -match 'action=skip reason=use_running_game')
    Assert-Observed -Name "$caseName.no_apply" -Observed ($result.Text -match 'action=apply') -Expected $false -Pass ($result.Text -notmatch 'action=apply')
    Assert-Observed -Name "$caseName.recover_sentinel" -Observed $recoverSentinel -Expected $sentinelPath -Pass ([System.StringComparer]::OrdinalIgnoreCase.Equals($recoverSentinel, $sentinelPath))
    Assert-Observed -Name "$caseName.skip_sentinel" -Observed $skipSentinel -Expected $sentinelPath -Pass ([System.StringComparer]::OrdinalIgnoreCase.Equals($skipSentinel, $sentinelPath))
    Assert-Observed -Name "$caseName.restored_recorded_prior" -Observed $after.IsReadOnly -Expected $false -Pass (-not $after.IsReadOnly)
    Assert-Observed -Name "$caseName.last_write_utc" -Observed $after.LastWriteTimeUtc -Expected $before.LastWriteTimeUtc -Pass ($after.LastWriteTimeUtc -ceq $before.LastWriteTimeUtc)
    Assert-Observed -Name "$caseName.length" -Observed $after.Length -Expected $before.Length -Pass ($after.Length -eq $before.Length)
    Assert-Observed -Name "$caseName.sha256" -Observed $after.Sha256 -Expected $before.Sha256 -Pass ($after.Sha256 -ceq $before.Sha256)
    Assert-Observed -Name "$caseName.sentinel_absent" -Observed (Test-Path -LiteralPath $sentinelPath) -Expected $false -Pass (-not (Test-Path -LiteralPath $sentinelPath))
}

function Invoke-AliasEquivalenceCase {
    $caseName = 'alias_equivalence'
    $caseRoot = Join-Path $testRoot $caseName
    $realPdxDir = Join-Path $caseRoot 'pdx_real'
    $aliasPdxDir = Join-Path $caseRoot 'pdx_alias'
    New-Item -ItemType Directory -Path $realPdxDir -Force | Out-Null
    New-Item -ItemType Junction -Path $aliasPdxDir -Target $realPdxDir | Out-Null
    $settingsPath = Join-Path $realPdxDir 'settings.txt'
    Set-Content -LiteralPath $settingsPath -Value 'settings-guard-alias-equivalence' -Encoding UTF8

    $realSentinel = Get-ProbedSentinelPath `
        -CaseName "$caseName.real" `
        -PdxUserDir $realPdxDir `
        -CaseRoot (Join-Path $caseRoot 'real_probe')
    $aliasSentinel = Get-ProbedSentinelPath `
        -CaseName "$caseName.alias" `
        -PdxUserDir $aliasPdxDir `
        -CaseRoot (Join-Path $caseRoot 'alias_probe')
    $sameSentinel = [System.StringComparer]::OrdinalIgnoreCase.Equals(
        $realSentinel,
        $aliasSentinel
    )
    Assert-Observed -Name "$caseName.same_sentinel" -Observed $aliasSentinel -Expected $realSentinel -Pass $sameSentinel

    $holderReadyPath = Join-Path $caseRoot 'holder.ready'
    $holderProcess = $null
    try {
        $holderProcess = Start-TestSentinelHolder `
            -SentinelPath $realSentinel `
            -ReadyPath $holderReadyPath
        $ownerStartUtc = Get-ProcessStartUtc -Process $holderProcess
        (Get-Item -LiteralPath $settingsPath -Force).IsReadOnly = $true
        $before = Get-SettingsMetadata -Path $settingsPath
        Write-TestSentinel `
            -SentinelPath $realSentinel `
            -SettingsPath $settingsPath `
            -PriorReadOnly $false `
            -OwnerPid $holderProcess.Id `
            -OwnerStartUtc $ownerStartUtc
        Wait-TestSentinelHolder -Process $holderProcess -ReadyPath $holderReadyPath

        Write-Host "gate=ALIAS-EQUIVALENCE real_pdx=$realPdxDir alias_pdx=$aliasPdxDir sentinel_path=$realSentinel"
        $result = Invoke-HarnessPrelaunch `
            -CaseName $caseName `
            -PdxUserDir $aliasPdxDir `
            -OutDir (Join-Path $caseRoot 'out')
        $after = Get-SettingsMetadata -Path $settingsPath
        $normalizedOutput = ($result.Text -replace '\s+', ' ').Trim()

        Assert-Observed -Name "$caseName.exit_nonzero" -Observed $result.ExitCode -Expected 'nonzero' -Pass ($result.ExitCode -ne 0)
        Assert-Observed -Name "$caseName.alias_found_live_holder" -Observed ($normalizedOutput -match 'incompatible live holder') -Expected $true -Pass ($normalizedOutput -match 'incompatible live holder')
        Assert-Observed -Name "$caseName.no_apply" -Observed ($result.Text -match 'action=apply') -Expected $false -Pass ($result.Text -notmatch 'action=apply')
        Assert-Observed -Name "$caseName.no_recover" -Observed ($result.Text -match 'action=recover') -Expected $false -Pass ($result.Text -notmatch 'action=recover')
        Assert-Observed -Name "$caseName.still_read_only" -Observed $after.IsReadOnly -Expected $true -Pass $after.IsReadOnly
        Assert-Observed -Name "$caseName.attributes" -Observed $after.Attributes -Expected $before.Attributes -Pass ($after.Attributes -ceq $before.Attributes)
        Assert-Observed -Name "$caseName.sha256" -Observed $after.Sha256 -Expected $before.Sha256 -Pass ($after.Sha256 -ceq $before.Sha256)
        Assert-Observed -Name "$caseName.sentinel_retained" -Observed (Test-Path -LiteralPath $realSentinel) -Expected $true -Pass (Test-Path -LiteralPath $realSentinel)
    } finally {
        Stop-TestOwner -Process $holderProcess
        if (Test-Path -LiteralPath $settingsPath -PathType Leaf) {
            (Get-Item -LiteralPath $settingsPath -Force).IsReadOnly = $false
        }
    }
}

function Invoke-LegacySentinelMigrationCase {
    $caseName = 'legacy_sentinel_migration'
    $caseRoot = Join-Path $testRoot $caseName
    $pdxDir = Join-Path $caseRoot 'pdx'
    New-Item -ItemType Directory -Path $pdxDir -Force | Out-Null
    $settingsPath = Join-Path $pdxDir 'settings.txt'
    Set-Content -LiteralPath $settingsPath -Value 'settings-guard-legacy-migration' -Encoding UTF8
    $canonicalSentinel = Get-ProbedSentinelPath `
        -CaseName $caseName `
        -PdxUserDir $pdxDir `
        -CaseRoot $caseRoot
    $legacySentinel = Get-LegacySentinelPath -SettingsPath $settingsPath
    $script:sentinelsToClean.Add($legacySentinel)
    $sentinelsDiffer = -not [System.StringComparer]::OrdinalIgnoreCase.Equals(
        $legacySentinel,
        $canonicalSentinel
    )
    Assert-Observed -Name "$caseName.distinct_legacy_key" -Observed $sentinelsDiffer -Expected $true -Pass $sentinelsDiffer

    $original = Get-SettingsMetadata -Path $settingsPath
    (Get-Item -LiteralPath $settingsPath -Force).IsReadOnly = $true
    $before = Get-SettingsMetadata -Path $settingsPath
    Write-TestSentinel `
        -SentinelPath $legacySentinel `
        -SettingsPath $settingsPath `
        -PriorReadOnly $false `
        -OwnerPid 2147483647 `
        -OwnerStartUtc ([DateTimeOffset]::UtcNow.AddMinutes(-5).ToString('o'))

    Write-Host "gate=LEGACY-SENTINEL-MIGRATION legacy_sentinel=$legacySentinel canonical_sentinel=$canonicalSentinel"
    $result = Invoke-HarnessPrelaunch `
        -CaseName $caseName `
        -PdxUserDir $pdxDir `
        -OutDir (Join-Path $caseRoot 'out')
    $after = Get-SettingsMetadata -Path $settingsPath
    $recoverSentinel = Get-EmittedSentinelPath -Text $result.Text -Action 'recover'
    $applySentinel = Get-EmittedSentinelPath -Text $result.Text -Action 'apply'
    $releaseSentinel = Get-EmittedSentinelPath -Text $result.Text -Action 'release'
    $normalizedOutput = ($result.Text -replace '\s+', ' ').Trim()

    Assert-Observed -Name "$caseName.exit_nonzero" -Observed $result.ExitCode -Expected 'nonzero' -Pass ($result.ExitCode -ne 0)
    Assert-Observed -Name "$caseName.migration_warning" -Observed ($normalizedOutput -match 'legacy DOS-path sentinel') -Expected $true -Pass ($normalizedOutput -match 'legacy DOS-path sentinel')
    Assert-Observed -Name "$caseName.recovered_legacy" -Observed $recoverSentinel -Expected $legacySentinel -Pass ([System.StringComparer]::OrdinalIgnoreCase.Equals($recoverSentinel, $legacySentinel))
    Assert-Observed -Name "$caseName.applied_canonical" -Observed $applySentinel -Expected $canonicalSentinel -Pass ([System.StringComparer]::OrdinalIgnoreCase.Equals($applySentinel, $canonicalSentinel))
    Assert-Observed -Name "$caseName.released_canonical" -Observed $releaseSentinel -Expected $canonicalSentinel -Pass ([System.StringComparer]::OrdinalIgnoreCase.Equals($releaseSentinel, $canonicalSentinel))
    Assert-Observed -Name "$caseName.restored_recorded_prior" -Observed $after.IsReadOnly -Expected $false -Pass (-not $after.IsReadOnly)
    Assert-Observed -Name "$caseName.attributes" -Observed $after.Attributes -Expected $original.Attributes -Pass ($after.Attributes -ceq $original.Attributes)
    Assert-Observed -Name "$caseName.last_write_utc" -Observed $after.LastWriteTimeUtc -Expected $before.LastWriteTimeUtc -Pass ($after.LastWriteTimeUtc -ceq $before.LastWriteTimeUtc)
    Assert-Observed -Name "$caseName.sha256" -Observed $after.Sha256 -Expected $before.Sha256 -Pass ($after.Sha256 -ceq $before.Sha256)
    Assert-Observed -Name "$caseName.legacy_absent" -Observed (Test-Path -LiteralPath $legacySentinel) -Expected $false -Pass (-not (Test-Path -LiteralPath $legacySentinel))
    Assert-Observed -Name "$caseName.canonical_absent" -Observed (Test-Path -LiteralPath $canonicalSentinel) -Expected $false -Pass (-not (Test-Path -LiteralPath $canonicalSentinel))
}

function Invoke-ExclusiveReaderCase {
    $caseName = 'exclusive_reader'
    $caseRoot = Join-Path $testRoot $caseName
    $pdxDir = Join-Path $caseRoot 'pdx'
    New-Item -ItemType Directory -Path $pdxDir -Force | Out-Null
    $settingsPath = Join-Path $pdxDir 'settings.txt'
    Set-Content -LiteralPath $settingsPath -Value 'settings-guard-exclusive-reader' -Encoding UTF8
    $sentinelPath = Get-ProbedSentinelPath `
        -CaseName $caseName `
        -PdxUserDir $pdxDir `
        -CaseRoot $caseRoot
    $readerReadyPath = Join-Path $caseRoot 'reader.ready'
    $readerProcess = $null
    try {
        $readerProcess = Start-TestSentinelHolder `
            -SentinelPath $sentinelPath `
            -ReadyPath $readerReadyPath `
            -Access 'Read' `
            -Share 'None'
        $readerStartUtc = Get-ProcessStartUtc -Process $readerProcess
        (Get-Item -LiteralPath $settingsPath -Force).IsReadOnly = $true
        $before = Get-SettingsMetadata -Path $settingsPath
        Write-TestSentinel `
            -SentinelPath $sentinelPath `
            -SettingsPath $settingsPath `
            -PriorReadOnly $false `
            -OwnerPid $readerProcess.Id `
            -OwnerStartUtc $readerStartUtc
        Wait-TestSentinelHolder -Process $readerProcess -ReadyPath $readerReadyPath

        Write-Host "gate=EXCLUSIVE-READER reader_pid=$($readerProcess.Id) access=Read share=None settings_path=$settingsPath sentinel_path=$sentinelPath"
        $result = Invoke-HarnessPrelaunch `
            -CaseName $caseName `
            -PdxUserDir $pdxDir `
            -OutDir (Join-Path $caseRoot 'out')
        $after = Get-SettingsMetadata -Path $settingsPath
        $normalizedOutput = ($result.Text -replace '\s+', ' ').Trim()

        Assert-Observed -Name "$caseName.exit_nonzero" -Observed $result.ExitCode -Expected 'nonzero' -Pass ($result.ExitCode -ne 0)
        Assert-Observed -Name "$caseName.incompatible_holder" -Observed ($normalizedOutput -match 'incompatible live holder') -Expected $true -Pass ($normalizedOutput -match 'incompatible live holder')
        Assert-Observed -Name "$caseName.reader_named" -Observed ($normalizedOutput -match 'capture writer or an exclusive reader') -Expected $true -Pass ($normalizedOutput -match 'capture writer or an exclusive reader')
        Assert-Observed -Name "$caseName.no_false_live_capture_claim" -Observed ($normalizedOutput -match 'held by a live capture run') -Expected $false -Pass ($normalizedOutput -notmatch 'held by a live capture run')
        Assert-Observed -Name "$caseName.no_unreadable_misclassification" -Observed ($normalizedOutput -match 'sentinel is unreadable') -Expected $false -Pass ($normalizedOutput -notmatch 'sentinel is unreadable')
        Assert-Observed -Name "$caseName.no_apply" -Observed ($result.Text -match 'action=apply') -Expected $false -Pass ($result.Text -notmatch 'action=apply')
        Assert-Observed -Name "$caseName.no_recover" -Observed ($result.Text -match 'action=recover') -Expected $false -Pass ($result.Text -notmatch 'action=recover')
        Assert-Observed -Name "$caseName.still_read_only" -Observed $after.IsReadOnly -Expected $true -Pass $after.IsReadOnly
        Assert-Observed -Name "$caseName.attributes" -Observed $after.Attributes -Expected $before.Attributes -Pass ($after.Attributes -ceq $before.Attributes)
        Assert-Observed -Name "$caseName.sha256" -Observed $after.Sha256 -Expected $before.Sha256 -Pass ($after.Sha256 -ceq $before.Sha256)
        Assert-Observed -Name "$caseName.sentinel_retained" -Observed (Test-Path -LiteralPath $sentinelPath) -Expected $true -Pass (Test-Path -LiteralPath $sentinelPath)
    } finally {
        Stop-TestOwner -Process $readerProcess
        if (Test-Path -LiteralPath $settingsPath -PathType Leaf) {
            (Get-Item -LiteralPath $settingsPath -Force).IsReadOnly = $false
        }
    }
}

function Invoke-CleanupFailurePreservesOriginalCase {
    $caseName = 'cleanup_failure_preserves_original'
    $caseRoot = Join-Path $testRoot $caseName
    $pdxDir = Join-Path $caseRoot 'pdx'
    New-Item -ItemType Directory -Path $pdxDir -Force | Out-Null
    $settingsPath = Join-Path $pdxDir 'settings.txt'
    Set-Content -LiteralPath $settingsPath -Value 'settings-guard-cleanup-failure' -Encoding UTF8
    $before = Get-SettingsMetadata -Path $settingsPath
    $expectedSentinel = Get-ProbedSentinelPath `
        -CaseName $caseName `
        -PdxUserDir $pdxDir `
        -CaseRoot (Join-Path $caseRoot 'probe')
    $instrumentedHarness = New-CleanupFailureHarness

    Write-Host "gate=CLEANUP-FAILURE warning_action=Stop settings_path=$settingsPath harness=$instrumentedHarness"
    $result = Invoke-HarnessPrelaunch `
        -CaseName $caseName `
        -PdxUserDir $pdxDir `
        -OutDir (Join-Path $caseRoot 'out') `
        -HarnessPath $instrumentedHarness `
        -UseProductionRoster `
        -ChildWarningAction 'Stop'
    $after = Get-SettingsMetadata -Path $settingsPath
    $applySentinel = Get-EmittedSentinelPath -Text $result.Text -Action 'apply'
    $releaseSentinel = Get-EmittedSentinelPath -Text $result.Text -Action 'release'
    $script:sentinelsToClean.Add($applySentinel)
    $normalizedOutput = ($result.Text -replace '\s+', ' ').Trim()

    Assert-Observed -Name "$caseName.exit_one" -Observed $result.ExitCode -Expected 1 -Pass ($result.ExitCode -eq 1)
    Assert-Observed -Name "$caseName.original_error_preserved" -Observed ($normalizedOutput -match 'settings-guard-original-error-probe') -Expected $true -Pass ($normalizedOutput -match 'settings-guard-original-error-probe')
    Assert-Observed -Name "$caseName.cleanup_warning_emitted" -Observed ($normalizedOutput -match 'could not remove sentinel') -Expected $true -Pass ($normalizedOutput -match 'could not remove sentinel')
    Assert-Observed -Name "$caseName.warning_preference_did_not_replace_error" -Observed ($normalizedOutput -match 'WarningPreference') -Expected $false -Pass ($normalizedOutput -notmatch 'WarningPreference')
    Assert-Observed -Name "$caseName.expected_sentinel" -Observed $applySentinel -Expected $expectedSentinel -Pass ([System.StringComparer]::OrdinalIgnoreCase.Equals($applySentinel, $expectedSentinel))
    Assert-Observed -Name "$caseName.phase_sentinel_match" -Observed $releaseSentinel -Expected $applySentinel -Pass ([System.StringComparer]::OrdinalIgnoreCase.Equals($releaseSentinel, $applySentinel))
    Assert-Observed -Name "$caseName.restored_read_only" -Observed $after.IsReadOnly -Expected $false -Pass (-not $after.IsReadOnly)
    Assert-Observed -Name "$caseName.attributes" -Observed $after.Attributes -Expected $before.Attributes -Pass ($after.Attributes -ceq $before.Attributes)
    Assert-Observed -Name "$caseName.last_write_utc" -Observed $after.LastWriteTimeUtc -Expected $before.LastWriteTimeUtc -Pass ($after.LastWriteTimeUtc -ceq $before.LastWriteTimeUtc)
    Assert-Observed -Name "$caseName.length" -Observed $after.Length -Expected $before.Length -Pass ($after.Length -eq $before.Length)
    Assert-Observed -Name "$caseName.sha256" -Observed $after.Sha256 -Expected $before.Sha256 -Pass ($after.Sha256 -ceq $before.Sha256)
    Assert-Observed -Name "$caseName.sentinel_retained" -Observed (Test-Path -LiteralPath $applySentinel) -Expected $true -Pass (Test-Path -LiteralPath $applySentinel)
}

function Invoke-MalformedSentinelCase {
    param(
        [Parameter(Mandatory = $true)]
        [ValidateSet(
            'unparseable',
            'empty',
            'missing_fields',
            'legacy_schema',
            'non_boolean_prior',
            'missing_owner_start',
            'path_mismatch'
        )]
        [string]$Variant
    )

    $caseName = "malformed_$Variant"
    $caseRoot = Join-Path $testRoot $caseName
    $pdxDir = Join-Path $caseRoot 'pdx'
    $outDir = Join-Path $caseRoot 'out'
    New-Item -ItemType Directory -Path $pdxDir -Force | Out-Null
    $settingsPath = Join-Path $pdxDir 'settings.txt'
    Set-Content -LiteralPath $settingsPath -Value "settings-guard-$caseName" -Encoding UTF8
    $sentinelPath = Get-ProbedSentinelPath `
        -CaseName $caseName `
        -PdxUserDir $pdxDir `
        -CaseRoot $caseRoot
    $ownerStartUtc = [DateTimeOffset]::UtcNow.AddMinutes(-5).ToString('o')
    $validRecord = [ordered]@{
        schema_version  = 2
        settings_path   = [System.IO.Path]::GetFullPath($settingsPath)
        prior_read_only = $false
        created_utc     = [DateTime]::UtcNow.AddMinutes(-5).ToString('o')
        owner_pid       = 2147483647
        owner_start_utc = $ownerStartUtc
    }
    $expectedDiagnostic = switch ($Variant) {
        'unparseable'         { 'sentinel is unreadable' }
        'empty'               { 'sentinel is invalid' }
        'missing_fields'      { 'invalid or from an unsupported schema' }
        'legacy_schema'       { 'invalid or from an unsupported schema' }
        'non_boolean_prior'   { 'invalid or from an unsupported schema' }
        'missing_owner_start' { 'invalid or from an unsupported schema' }
        'path_mismatch'       { 'sentinel path mismatch' }
    }
    switch ($Variant) {
        'unparseable' {
            $sentinelContent = '{'
        }
        'empty' {
            $sentinelContent = ''
        }
        'missing_fields' {
            $sentinelContent = ([ordered]@{
                schema_version = 2
                settings_path  = [System.IO.Path]::GetFullPath($settingsPath)
            } | ConvertTo-Json -Compress)
        }
        'legacy_schema' {
            $validRecord.schema_version = 1
            $sentinelContent = $validRecord | ConvertTo-Json -Compress
        }
        'non_boolean_prior' {
            $validRecord.prior_read_only = 'false'
            $sentinelContent = $validRecord | ConvertTo-Json -Compress
        }
        'missing_owner_start' {
            $validRecord.Remove('owner_start_utc')
            $sentinelContent = $validRecord | ConvertTo-Json -Compress
        }
        'path_mismatch' {
            $validRecord.settings_path = Join-Path $pdxDir 'different_settings.txt'
            $sentinelContent = $validRecord | ConvertTo-Json -Compress
        }
    }

    New-Item -ItemType Directory -Path (Split-Path -Parent $sentinelPath) -Force | Out-Null
    [System.IO.File]::WriteAllText(
        $sentinelPath,
        $sentinelContent,
        [System.Text.UTF8Encoding]::new($false)
    )
    $before = Get-SettingsMetadata -Path $settingsPath

    Write-Host "gate=MALFORMED-SENTINEL variant=$Variant settings_path=$settingsPath sentinel_path=$sentinelPath"
    $result = Invoke-HarnessPrelaunch -CaseName $caseName -PdxUserDir $pdxDir -OutDir $outDir
    $after = Get-SettingsMetadata -Path $settingsPath
    $compactedOutput = $result.Text -replace '\s', ''
    $compactedSentinel = $sentinelPath -replace '\s', ''
    $normalizedOutput = ($result.Text -replace '\s+', ' ').Trim()
    $diagnosticMatch = [regex]::Match(
        $normalizedOutput,
        [regex]::Escape($expectedDiagnostic),
        [System.Text.RegularExpressions.RegexOptions]::IgnoreCase
    )
    $observedDiagnostic = if ($diagnosticMatch.Success) {
        $diagnosticMatch.Value
    } else {
        '<not found>'
    }
    Write-Host "observed_malformed_diagnostic variant=$Variant observed='$observedDiagnostic' expected='$expectedDiagnostic' matched=$($diagnosticMatch.Success)"

    Assert-Observed -Name "$caseName.exit_nonzero" -Observed $result.ExitCode -Expected 'nonzero' -Pass ($result.ExitCode -ne 0)
    Assert-Observed -Name "$caseName.variant_diagnostic" -Observed $observedDiagnostic -Expected $expectedDiagnostic -Pass $diagnosticMatch.Success
    Assert-Observed -Name "$caseName.no_apply" -Observed ($result.Text -match 'action=apply') -Expected $false -Pass ($result.Text -notmatch 'action=apply')
    Assert-Observed -Name "$caseName.sentinel_named" -Observed ($compactedOutput -match [regex]::Escape($compactedSentinel)) -Expected $true -Pass ($compactedOutput -match [regex]::Escape($compactedSentinel))
    Assert-Observed -Name "$caseName.not_read_only" -Observed $after.IsReadOnly -Expected $false -Pass (-not $after.IsReadOnly)
    Assert-Observed -Name "$caseName.attributes" -Observed $after.Attributes -Expected $before.Attributes -Pass ($after.Attributes -ceq $before.Attributes)
    Assert-Observed -Name "$caseName.last_write_utc" -Observed $after.LastWriteTimeUtc -Expected $before.LastWriteTimeUtc -Pass ($after.LastWriteTimeUtc -ceq $before.LastWriteTimeUtc)
    Assert-Observed -Name "$caseName.length" -Observed $after.Length -Expected $before.Length -Pass ($after.Length -eq $before.Length)
    Assert-Observed -Name "$caseName.sha256" -Observed $after.Sha256 -Expected $before.Sha256 -Pass ($after.Sha256 -ceq $before.Sha256)
    Assert-Observed -Name "$caseName.sentinel_retained" -Observed (Test-Path -LiteralPath $sentinelPath) -Expected $true -Pass (Test-Path -LiteralPath $sentinelPath)
}

$failureMessage = $null
try {
    $liveCaptureProcesses = @(Get-Process -Name 'the game', 'collector_record' -ErrorAction SilentlyContinue)
    if ($liveCaptureProcesses.Count -ne 0) {
        $description = @($liveCaptureProcesses | ForEach-Object { "$($_.ProcessName):$($_.Id)" }) -join ','
        throw "SAFETY ABORT: capture process(es) are running: $description"
    }

    New-Item -ItemType Directory -Path $testRoot -Force | Out-Null
    Write-Host "safety_check=passed live_capture_processes=0 test_root=$testRoot"
    Invoke-GateBCase -CaseName 'prior_false' -PriorReadOnly $false
    Invoke-GateBCase -CaseName 'prior_true' -PriorReadOnly $true
    Invoke-GateC
    Invoke-SameHostStaleCase
    Invoke-LiveOwnerCase
    Invoke-BenignReaderCase
    Invoke-DeadOwnerCase
    Invoke-RecycledOwnerCase
    Invoke-UseRunningGameCase
    Invoke-AliasEquivalenceCase
    Invoke-LegacySentinelMigrationCase
    Invoke-ExclusiveReaderCase
    Invoke-CleanupFailurePreservesOriginalCase
    foreach ($variant in @(
        'unparseable',
        'empty',
        'missing_fields',
        'legacy_schema',
        'non_boolean_prior',
        'missing_owner_start',
        'path_mismatch'
    )) {
        Invoke-MalformedSentinelCase -Variant $variant
    }

    if ($script:assertionFailures.Count -ne 0) {
        $failureMessage = "FAILED assertion_count=$($script:assertionFailures.Count): $($script:assertionFailures -join '; ')"
    }
} catch {
    $failureMessage = "FAILED unhandled_error=$($_.Exception.Message)"
} finally {
    foreach ($sentinelPath in $script:sentinelsToClean) {
        if (Test-Path -LiteralPath $sentinelPath) {
            Remove-Item -LiteralPath $sentinelPath -Force -ErrorAction SilentlyContinue
        }
    }
    foreach ($temporaryHarness in $script:temporaryHarnessesToClean) {
        if (Test-Path -LiteralPath $temporaryHarness -PathType Leaf) {
            Remove-Item -LiteralPath $temporaryHarness -Force -ErrorAction SilentlyContinue
        }
    }
    if (Test-Path -LiteralPath $testRoot) {
        $resolvedTestRoot = (Resolve-Path -LiteralPath $testRoot).Path
        $resolvedLaneRoot = (Resolve-Path -LiteralPath $laneRoot).Path
        if ($resolvedTestRoot.StartsWith(
            $resolvedLaneRoot.TrimEnd('\') + '\',
            [System.StringComparison]::OrdinalIgnoreCase
        )) {
            Remove-Item -LiteralPath $resolvedTestRoot -Recurse -Force
        } else {
            $failureMessage = "FAILED cleanup_refused unexpected_test_root=$resolvedTestRoot"
        }
    }
}

if ($null -ne $failureMessage) {
    Write-Host $failureMessage
    exit 1
}

Write-Host 'result=PASS gate_b_cases=2 gate_c_cases=1 owner_cases=4 benign_reader_cases=1 exclusive_reader_cases=1 alias_cases=1 legacy_migration_cases=1 cleanup_failure_cases=1 use_running_game_cases=1 malformed_sentinel_cases=7 assertion_failures=0'
exit 0
