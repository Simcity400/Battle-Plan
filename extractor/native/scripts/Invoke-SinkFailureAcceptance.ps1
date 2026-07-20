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
$forbidden = @("--pid", "--out-dir", "--acceptance-test-mode", "--acceptance-test-nonce")
foreach ($flag in $forbidden) {
    if ($RecorderArguments -contains $flag) {
        throw "RecorderArguments must not contain wrapper-owned flag $flag"
    }
}
foreach ($required in @("--early-launch-exe", "--profile", "production", "--production", "--stream-format", "compact")) {
    if ($RecorderArguments -notcontains $required) {
        throw "RecorderArguments must contain $required"
    }
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

$certification = Join-Path $RunDir "sink_reconciliation_verification.json"
& $PythonExe $verifier --run-dir $RunDir --expect $Mode --out $certification
if ($LASTEXITCODE -ne 0) {
    throw "sink-failure receipt verification failed: $certification"
}
Write-Host "Authenticated $Mode acceptance passed: $certification"
