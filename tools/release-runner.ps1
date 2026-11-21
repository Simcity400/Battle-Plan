[CmdletBinding(DefaultParameterSetName = 'Start')]
param(
    [Parameter(ParameterSetName = 'Install', Mandatory)][switch]$Install,
    [Parameter(ParameterSetName = 'Install', Mandatory)][string]$TokenFile,
    [string]$Root = (Join-Path $env:LOCALAPPDATA 'BattlePlan release runner'),
    [string]$Repository = $env:GITHUB_REPOSITORY,
    [string]$PrepLock = $(if (Test-Path Env:BATTLE_PLAN_PREP_LOCK) { $env:BATTLE_PLAN_PREP_LOCK } else { $env:HOI4_PREP_LOCK }),
    [string]$GameLock = ((Join-Path $env:TEMP 'claude\hoi4_game.lock') -replace '\\', '/'),
    [int]$TargetCapGB = 40,
    [string]$SmokeRecording = '',
    [string]$GameDir = $(if (Test-Path Env:BATTLE_PLAN_GAME_DIR) { $env:BATTLE_PLAN_GAME_DIR } else { $env:HOI4_GAME_DIR }),
    [string]$UserDir = (Join-Path ([Environment]::GetFolderPath('MyDocuments')) 'Paradox Interactive\Hearts of Iron IV')
)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$runner = Join-Path $Root 'runner'
$task = 'BattlePlan release runner'

function Enter-Background {
    (Get-Process -Id $PID).PriorityClass = 'BelowNormal'
    Add-Type -Namespace the game -Name Priority -MemberDefinition @'
[System.Runtime.InteropServices.DllImport("ntdll.dll")]
public static extern int NtSetInformationProcess(System.IntPtr process, int informationClass, ref int information, int length);
'@
    $low = 1
    $status = [Hoi4.Priority]::NtSetInformationProcess([System.Diagnostics.Process]::GetCurrentProcess().Handle, 33, [ref]$low, 4)
    if ($status -ne 0) { throw ('Low I/O priority was refused: NTSTATUS 0x{0:X8}' -f $status) }
}

if ($Install -and -not $Repository) { throw 'Pass -Repository owner/repo or set GITHUB_REPOSITORY.' }

if (-not $Install) {
    Enter-Background
    $run = Join-Path $runner 'run.cmd'
    if (-not (Test-Path -LiteralPath $run)) { throw "$run is missing; install the runner first." }
    $git = Split-Path (Get-Command git.exe -ErrorAction Stop).Source
    while ($git -and -not (Test-Path -LiteralPath (Join-Path $git 'bin\bash.exe'))) { $git = Split-Path $git }
    if (-not $git) { throw "Git for Windows' bin\bash.exe was not found above git.exe." }
    $env:PATH = "$(Join-Path $git 'bin');$env:PATH"
    while ($true) {
        $process = Start-Process -FilePath $run -WorkingDirectory $runner -NoNewWindow -PassThru -Wait
        Start-Sleep -Seconds 30
    }
}

$version = '2.338.2'
$digest = 'a1f17aa987fc8158bc7521d284912267a0d01b7c2c4684b33c86783b5e8221cc'
$folders = @{ target = 'target'; cargo = 'cargo'; rustup = 'rustup'; npm = 'npm-cache'; temp = 'temp' }
$at = @{}
foreach ($key in $folders.Keys) { $at[$key] = Join-Path $Root $folders[$key] }
New-Item -ItemType Directory -Force -Path (@($runner, (Join-Path $Root 'work')) + $at.Values) | Out-Null
if (-not (Test-Path -LiteralPath (Join-Path $runner 'config.cmd'))) {
    $archive = Join-Path $Root "actions-runner-win-x64-$version.zip"
    Invoke-WebRequest -UseBasicParsing -OutFile $archive -Uri "https://github.com/actions/runner/releases/download/v$version/actions-runner-win-x64-$version.zip"
    $found = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($found -ne $digest) { throw "The runner archive's SHA256 is $found, not $digest." }
    Expand-Archive -LiteralPath $archive -DestinationPath $runner -Force
    Remove-Item -LiteralPath $archive
}
$environment = @(
    "CARGO_TARGET_DIR=$($at.target)", "CARGO_HOME=$($at.cargo)", "RUSTUP_HOME=$($at.rustup)",
    "npm_config_cache=$($at.npm)", "TEMP=$($at.temp)", "TMP=$($at.temp)",
    "BATTLE_PLAN_PREP_LOCK=$PrepLock", "BATTLE_PLAN_GAME_LOCK=$GameLock", "BATTLE_PLAN_TARGET_CAP_GB=$TargetCapGB",
    "BATTLE_PLAN_RELEASE_SMOKE_RECORDING=$SmokeRecording", "BATTLE_PLAN_GAME_DIR=$GameDir", "BATTLE_PLAN_USER_DIR=$UserDir")
[IO.File]::WriteAllText((Join-Path $runner '.env'), ($environment -join "`n") + "`n")
$token = (Get-Content -LiteralPath $TokenFile -Raw).Trim()
if (-not $token) { throw "$TokenFile holds no token." }
Push-Location $runner
try {
    & .\config.cmd --unattended --replace --url "https://github.com/$Repository" --token $token --name hoi4-release --labels release --work (Join-Path $Root 'work')
    if ($LASTEXITCODE -ne 0) { throw "The runner could not register (exit $LASTEXITCODE)." }
} finally {
    Pop-Location
    $token = $null
}
$launcher = Join-Path $Root 'release-runner.ps1'
Copy-Item -LiteralPath $PSCommandPath -Destination $launcher -Force
$action = New-ScheduledTaskAction -Execute 'powershell.exe' -Argument "-NoProfile -NonInteractive -WindowStyle Hidden -ExecutionPolicy Bypass -File `"$launcher`" -Root `"$Root`""
$trigger = New-ScheduledTaskTrigger -AtLogOn -User "$env:USERDOMAIN\$env:USERNAME"
$principal = New-ScheduledTaskPrincipal -UserId "$env:USERDOMAIN\$env:USERNAME" -LogonType Interactive -RunLevel Limited
$settings = New-ScheduledTaskSettingsSet -Priority 7 -ExecutionTimeLimit ([TimeSpan]::Zero) -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -StartWhenAvailable -MultipleInstances IgnoreNew
Register-ScheduledTask -TaskName $task -Action $action -Trigger $trigger -Principal $principal -Settings $settings -Force | Out-Null
Start-ScheduledTask -TaskName $task
Write-Output "Registered hoi4-release for $Repository and started '$task'."
