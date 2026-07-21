[CmdletBinding()]
param(
    [Parameter(Mandatory=$true)][string]$PinsFile,
    [string]$StateFile = "",
    [string]$RunRoot = "",
    [switch]$ConfirmPausedVisible
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$ladder = Join-Path $scriptDir "Invoke-AcceptanceLadder.ps1"

if (-not (Test-Path -LiteralPath $PinsFile -PathType Leaf)) {
    throw "F6 pins file is missing: $PinsFile"
}
if (-not (Test-Path -LiteralPath $ladder -PathType Leaf)) {
    throw "F6 acceptance ladder is missing: $ladder"
}

# Keep this wrapper deliberately thin. `ConfirmPausedVisible` is retained as a
# compatibility/ergonomics switch, but is never required or accepted as
# evidence. The exact D57 projection and replay-derived proof determine pause.
# The ladder allocates the fresh run,
# enforces the synthetic-fault predecessor, creates the random acceptance nonce,
# starts the Safe launcher, and independently re-grades both replay evidence and
# receipt transport. The acknowledgement above is operator ergonomics only.
$arguments = @(
    "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass",
    "-File", $ladder,
    "-Rung", "paused-short-proof",
    "-Live",
    "-PinsFile", (Resolve-Path -LiteralPath $PinsFile).Path
)
if (-not [string]::IsNullOrWhiteSpace($StateFile)) {
    $arguments += @("-StateFile", $StateFile)
}
if (-not [string]::IsNullOrWhiteSpace($RunRoot)) {
    $arguments += @("-RunRoot", $RunRoot)
}

& powershell.exe @arguments
if ($LASTEXITCODE -ne 0) {
    throw "F6 paused-short-proof ladder failed with exit code $LASTEXITCODE"
}
