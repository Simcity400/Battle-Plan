<#
.SYNOPSIS
Launch one explicitly selected executable through WMI and retain process custody.
.DESCRIPTION
Passive setup only. Supply every game argument explicitly (including an empty
array). No defaults select a save, userdata directory, speed, or game state.
Call with PowerShell's & operator and array splatting to preserve argv; Windows
PowerShell -File cannot faithfully pass arbitrary string arrays.

OutputDirectory must not exist. RECEIPT.json schema
hoi4.observed_session_launch.v1 accepts custody only when status=launched and
custody_status=verified. process contains pid (integer), creation_filetime
(decimal Windows FILETIME string), exe_path, exe_sha256, session_id (integer).
Consumers must reobserve this incarnation; this is not campaign/recording proof.
Failures throw (nonzero when invoked by powershell.exe) and retain raw evidence.
A timeout can leave an unknown WMI-created process. No game is ever terminated.
#>
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
. (Join-Path $PSScriptRoot 'ObservedSession.Launch.ps1')
Invoke-ObservedSession @PSBoundParameters
