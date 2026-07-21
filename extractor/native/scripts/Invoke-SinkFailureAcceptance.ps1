param(
    [Parameter(Mandatory = $true)]
    [ValidateSet("sink-open-fault", "sink-write-fault")]
    [string]$Mode,
    [Parameter(Mandatory = $true)][string]$RecorderPath,
    [Parameter(Mandatory = $true)][string]$RunDir,
    [string]$PythonExe = "python.exe",
    [Parameter(ValueFromRemainingArguments = $true)]
    [string[]]$RecorderArguments
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$verifier = Join-Path $scriptDir "verify_sink_reconciliation.py"

if (-not (Test-Path -LiteralPath $RecorderPath -PathType Leaf)) {
    throw "collector_record executable not found: $RecorderPath"
}
if (-not (Test-Path -LiteralPath $verifier -PathType Leaf)) {
    throw "sink reconciliation verifier not found: $verifier"
}
if (Test-Path -LiteralPath $RunDir) {
    throw "refusing to reuse a sink-failure run directory: $RunDir"
}
$RecorderPath = (Resolve-Path -LiteralPath $RecorderPath).Path
$recorderFile = Split-Path -Leaf $RecorderPath
$recorderSha256 = (Get-FileHash -LiteralPath $RecorderPath -Algorithm SHA256).Hash.ToLowerInvariant()

function Get-RecorderOptionValues {
    param([string[]]$Arguments, [string]$Name)
    $values = [System.Collections.Generic.List[string]]::new()
    for ($i = 0; $i -lt @($Arguments).Count; $i++) {
        $argument = [string]$Arguments[$i]
        if ($argument -ceq $Name) {
            if (($i + 1) -ge $Arguments.Count) {
                throw "RecorderArguments option $Name has no value"
            }
            $values.Add([string]$Arguments[$i + 1])
            $i++
        } elseif ($argument.StartsWith("$Name=", [StringComparison]::Ordinal)) {
            $values.Add($argument.Substring($Name.Length + 1))
        }
    }
    # Let PowerShell enumerate the collected values. The caller wraps the result
    # in @(...), so zero/one/many occurrences remain distinguishable. Returning
    # a unary-comma nested array would make duplicate options appear as one item.
    return $values.ToArray()
}

function Require-OneRecorderOption {
    param([string[]]$Arguments, [string]$Name, [string]$Expected = "")
    $values = @(Get-RecorderOptionValues -Arguments $Arguments -Name $Name)
    if ($values.Count -ne 1 -or [string]::IsNullOrWhiteSpace($values[0])) {
        throw "RecorderArguments must contain exactly one nonblank $Name value"
    }
    if (-not [string]::IsNullOrEmpty($Expected) -and $values[0] -cne $Expected) {
        throw "RecorderArguments $Name must be '$Expected', got '$($values[0])'"
    }
    return $values[0]
}

$forbidden = @("--pid", "--out-dir", "--acceptance-test-mode", "--acceptance-test-nonce")
foreach ($flag in $forbidden) {
    if (@($RecorderArguments | Where-Object { [string]$_ -ceq $flag -or ([string]$_).StartsWith("$flag=", [StringComparison]::Ordinal) }).Count -ne 0) {
        throw "RecorderArguments must not contain wrapper-owned flag $flag"
    }
}
Require-OneRecorderOption -Arguments $RecorderArguments -Name "--early-launch-exe" | Out-Null
Require-OneRecorderOption -Arguments $RecorderArguments -Name "--profile" -Expected "production" | Out-Null
Require-OneRecorderOption -Arguments $RecorderArguments -Name "--stream-format" -Expected "compact" | Out-Null
Require-OneRecorderOption -Arguments $RecorderArguments -Name "--capture-id" | Out-Null
$productionFlags = @($RecorderArguments | Where-Object { [string]$_ -ceq "--production" })
if ($productionFlags.Count -ne 1 -or @($RecorderArguments | Where-Object { ([string]$_).StartsWith("--production=", [StringComparison]::Ordinal) }).Count -ne 0) {
    throw "RecorderArguments must contain exactly one bare --production flag"
}

$nonceBytes = [byte[]]::new(32)
$rng = [Security.Cryptography.RandomNumberGenerator]::Create()
try { $rng.GetBytes($nonceBytes) } finally { $rng.Dispose() }
$nonce = -join ($nonceBytes | ForEach-Object { $_.ToString("x2") })
$arguments = @($RecorderArguments) + @(
    "--out-dir", $RunDir,
    "--acceptance-test-mode", $Mode,
    "--acceptance-test-nonce", $nonce
)

& $RecorderPath @arguments
$recorderExit = $LASTEXITCODE
if ($recorderExit -eq 0) {
    throw "sink-failure acceptance arm unexpectedly completed successfully"
}
$receipt = Join-Path $RunDir "collector_sink_failure.json"
$plan = Join-Path $RunDir "sink_failure_acceptance_plan.json"
if (-not (Test-Path -LiteralPath $receipt -PathType Leaf) -or
    -not (Test-Path -LiteralPath $plan -PathType Leaf)) {
    throw "recorder failed without the authenticated sink-failure receipt and pre-launch plan"
}
$planObject = Get-Content -LiteralPath $plan -Raw | ConvertFrom-Json
if ([string]$planObject.recorder_executable_file -cne $recorderFile -or
    [string]$planObject.recorder_executable_sha256 -cne $recorderSha256) {
    throw "sink-failure plan is not bound to the recorder executable selected by this wrapper"
}

$certification = Join-Path $RunDir "sink_reconciliation_verification.json"
& $PythonExe $verifier --run-dir $RunDir --expect $Mode --out $certification
if ($LASTEXITCODE -ne 0) {
    throw "sink-failure receipt verification failed: $certification"
}
Write-Host "Authenticated $Mode acceptance passed: $certification"
