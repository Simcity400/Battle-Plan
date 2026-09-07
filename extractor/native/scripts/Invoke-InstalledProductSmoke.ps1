[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$InstalledRoot,
    [Parameter(Mandatory = $true)][string]$PackageReceiptPath,
    [Parameter(Mandatory = $true)][string]$OutputDirectory,
    [switch]$RequireOffline,
    [switch]$DesktopStartup,
    [ValidateRange(5, 60)][int]$TimeoutSeconds = 20
)

# Run only after the owning package verifier passes. This records installed
# process behavior; it does not verify ZIP bytes or qualify capture/playback.
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$outputRoot = [IO.Path]::GetFullPath($OutputDirectory)
$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../../..'))
if ($outputRoot.StartsWith($repoRoot.TrimEnd('\') + '\', [StringComparison]::OrdinalIgnoreCase) -or $outputRoot -eq $repoRoot) {
    throw 'Smoke output must be outside the repository.'
}
if (Test-Path -LiteralPath $outputRoot) { throw 'Use a new output directory to preserve previous receipts.' }
$null = New-Item -ItemType Directory -Path $outputRoot
$cases = [Collections.Generic.List[object]]::new()
$samples = [Collections.Generic.List[object]]::new()
$artifacts = [Collections.Generic.List[object]]::new()
$receipt = [ordered]@{
    schema = 'hoi4.installed-product-smoke.v1'; status = 'fail'
    mode = $(if ($DesktopStartup) { 'desktop' } else { 'cli' })
    machine = [Environment]::MachineName; os = [Environment]::OSVersion.VersionString
    started_utc = [DateTime]::UtcNow.ToString('o'); non_elevated = $false
    offline_observed = $false; package_receipt = $null
    artifacts = @(); environment_samples = @(); cases = @(); error = $null
    limitations = @('Current machine only; not clean-machine qualification.',
        'Network state is sampled; the harness does not change adapters or firewall rules.',
        'Startup is not recording, playback, rendering fidelity or package-admission proof.')
}

function Get-SmokeReference {
    param([string]$Path)
    $item = Get-Item -LiteralPath $Path
    return [ordered]@{ path = $item.FullName; bytes = $item.Length
        sha256 = (Get-FileHash -LiteralPath $item.FullName -Algorithm SHA256).Hash.ToLowerInvariant() }
}

function Get-SmokeEnvironment {
    $network = [Net.NetworkInformation.NetworkInterface]::GetIsNetworkAvailable()
    $sample = [ordered]@{ utc = [DateTime]::UtcNow.ToString('o'); network_available = $network }
    $samples.Add($sample)
    if ($RequireOffline -and $network) { throw 'Offline smoke requires all non-loopback network interfaces unavailable; network state was not changed.' }
    if ($DesktopStartup -and @(Get-Process -Name the game -ErrorAction SilentlyContinue).Count -ne 0) {
        throw 'Desktop startup requires no running the game: recorder discovery could attach. Use the parent capture broker for game scenarios.'
    }
}

function Invoke-SmokeCase {
    param([string]$Name, [string]$Exe, [string]$Arguments, [int]$ExpectedExit,
          [string]$ExpectedText = '', [switch]$Desktop)
    Get-SmokeEnvironment
    $stdout = Join-Path $outputRoot ($Name + '.stdout.txt')
    $stderr = Join-Path $outputRoot ($Name + '.stderr.txt')
    $record = [ordered]@{ name = $Name; executable = $Exe; arguments = $Arguments
        pid = $null; process_started_utc = $null; exit_code = $null
        window_title = $null; window_observed = $false; close_requested = $false
        passed = $false; stdout = $null; stderr = $null; error = $null }
    $child = $null
    try {
        $info = [Diagnostics.ProcessStartInfo]::new()
        $info.FileName = $Exe; $info.Arguments = $Arguments
        $info.WorkingDirectory = $outputRoot
        $info.UseShellExecute = $false
        $info.CreateNoWindow = -not $Desktop
        $info.RedirectStandardOutput = $true; $info.RedirectStandardError = $true
        foreach ($key in @($info.EnvironmentVariables.Keys)) {
            if ([string]$key -match '^(HOI4_|CARGO|RUST|SCCACHE)') { $info.EnvironmentVariables.Remove([string]$key) }
        }
        # No Cargo/Git/Python or repo path in the product child's search path.
        $info.EnvironmentVariables['PATH'] = Join-Path $env:SystemRoot 'System32'
        $info.EnvironmentVariables['LOCALAPPDATA'] = Join-Path $outputRoot 'local-app-data'
        $info.EnvironmentVariables['TEMP'] = $outputRoot
        $info.EnvironmentVariables['TMP'] = $outputRoot
        $child = [Diagnostics.Process]::new(); $child.StartInfo = $info
        if (-not $child.Start()) { throw 'Process did not start.' }
        $record.pid = $child.Id
        $record.process_started_utc = $child.StartTime.ToUniversalTime().ToString('o')
        $stdoutTask = $child.StandardOutput.ReadToEndAsync()
        $stderrTask = $child.StandardError.ReadToEndAsync()
        $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
        while (-not $child.HasExited) {
            Get-SmokeEnvironment
            $child.Refresh()
            if ($Desktop -and -not $record.close_requested -and $child.MainWindowHandle -ne [IntPtr]::Zero -and $child.Responding) {
                $record.window_title = $child.MainWindowTitle
                $record.window_observed = $true
                $record.close_requested = $child.CloseMainWindow()
                if (-not $record.close_requested) { throw 'Window did not accept cooperative close.' }
            }
            if ([DateTime]::UtcNow -gt $deadline) { throw 'Startup/exit timeout; no successful result is inferred from process liveness.' }
            Start-Sleep -Milliseconds 200
        }
        $child.WaitForExit()
        $record.exit_code = $child.ExitCode
        Get-SmokeEnvironment
        $outText = $stdoutTask.GetAwaiter().GetResult()
        $errText = $stderrTask.GetAwaiter().GetResult()
        [IO.File]::WriteAllText($stdout, $outText)
        [IO.File]::WriteAllText($stderr, $errText)
        if ($record.exit_code -ne $ExpectedExit) { throw "Expected exit $ExpectedExit, got $($record.exit_code)." }
        if ($Desktop -and (-not $record.window_observed -or -not $record.close_requested)) { throw 'No responsive desktop window and cooperative close observed.' }
        if ($ExpectedText -and -not ($outText + $errText).Contains($ExpectedText)) { throw "Missing required startup diagnostic: $ExpectedText" }
        $record.passed = $true
    } catch {
        $record.error = $_.Exception.Message
        throw
    } finally {
        if ($child -and $record.pid) {
            if (-not $child.HasExited) {
                # Only the Process object launched above is owned. Never kill a
                # game or process tree. A forced exit cannot pass this case.
                $child.Kill(); $child.WaitForExit()
            }
            if (-not (Test-Path -LiteralPath $stdout)) {
                [IO.File]::WriteAllText($stdout, $stdoutTask.GetAwaiter().GetResult())
                [IO.File]::WriteAllText($stderr, $stderrTask.GetAwaiter().GetResult())
            }
            $child.Dispose()
        }
        if (Test-Path -LiteralPath $stdout) { $record.stdout = Get-SmokeReference $stdout }
        if (Test-Path -LiteralPath $stderr) { $record.stderr = Get-SmokeReference $stderr }
        $cases.Add($record)
    }
}

try {
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    $principal = [Security.Principal.WindowsPrincipal]::new($identity)
    $receipt.non_elevated = -not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
    $receipt['user_sid'] = $identity.User.Value
    if (-not $receipt.non_elevated) { throw 'Run smoke under a non-elevated Windows token; no elevation is requested.' }
    Get-SmokeEnvironment
    $receipt.package_receipt = Get-SmokeReference $PackageReceiptPath
    $package = Get-Content -LiteralPath $PackageReceiptPath -Raw | ConvertFrom-Json
    if ($package.schema -cne 'battle-plan.central_portable_package.v1' -or
        $package.status -cne 'PASS_CENTRAL_RELEASE_PORTABLE_PACKAGE' -or $package.successful_exit -isnot [bool] -or -not $package.successful_exit) {
        throw 'Supply the central package receipt already checked by Test-PortableProductPackageReceipt.ps1.'
    }
    $names = [ordered]@{ recorder_host = 'Battle Plan Recorder.exe'; replay_app = 'Battle Plan.exe'; collector_dll = 'collector.dll' }
    foreach ($role in $names.Keys) {
        $artifact = Get-SmokeReference (Join-Path $InstalledRoot $names[$role])
        if ($artifact.sha256 -cne [string]$package.package.input_sha256.$role) { throw "Installed $role differs from the central package receipt." }
        $artifacts.Add($artifact)
    }
    $recorder = Join-Path ([IO.Path]::GetFullPath($InstalledRoot)) $names.recorder_host
    $viewer = Join-Path ([IO.Path]::GetFullPath($InstalledRoot)) $names.replay_app
    if ($DesktopStartup) {
        Invoke-SmokeCase -Name recorder_desktop -Exe $recorder -Arguments '' -ExpectedExit 0 -Desktop
        Invoke-SmokeCase -Name viewer_desktop -Exe $viewer -Arguments '' -ExpectedExit 0 -Desktop
    } else {
        Invoke-SmokeCase -Name recorder_help -Exe $recorder -Arguments '--help' -ExpectedExit 1 -ExpectedText 'usage: collector_record'
        Invoke-SmokeCase -Name viewer_blockers -Exe $viewer -Arguments '--list-blockers' -ExpectedExit 0
        $profile = Join-Path $outputRoot 'missing-profile.json'
        Invoke-SmokeCase -Name viewer_missing_profile -Exe $viewer -Arguments ('--configure --non-interactive --config "' + $profile + '"') -ExpectedExit 2 -ExpectedText 'is not configured'
    }
    foreach ($artifact in $artifacts) {
        if ((Get-SmokeReference $artifact.path).sha256 -cne $artifact.sha256) { throw 'Installed artifact changed during smoke.' }
    }
    if ((Get-SmokeReference $PackageReceiptPath).sha256 -cne $receipt.package_receipt.sha256) { throw 'Package receipt changed during smoke.' }
    $receipt.status = 'pass'
} catch {
    $receipt.error = $_.Exception.Message
} finally {
    $receipt.artifacts = @($artifacts.ToArray())
    $receipt.environment_samples = @($samples.ToArray())
    $receipt.cases = @($cases.ToArray())
    $receipt.offline_observed = $samples.Count -gt 0 -and @($samples | Where-Object { $_.network_available }).Count -eq 0
    $receipt['finished_utc'] = [DateTime]::UtcNow.ToString('o')
    $path = Join-Path $outputRoot 'SMOKE.json'
    [IO.File]::WriteAllText($path, ($receipt | ConvertTo-Json -Depth 10))
    Write-Output ($receipt | ConvertTo-Json -Depth 10 -Compress)
}
if ($receipt.status -cne 'pass') { exit 1 }
exit 0
