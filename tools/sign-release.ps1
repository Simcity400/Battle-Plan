[CmdletBinding()]
param([Parameter(Mandatory)][string]$Path)
$ErrorActionPreference = 'Stop'

if ((Get-AuthenticodeSignature -LiteralPath $Path).Status -eq 'Valid') {
    Write-Output "Already signed: $Path"
    exit 0
}
foreach ($name in 'BATTLE_PLAN_SIGN_DLIB', 'BATTLE_PLAN_SIGN_METADATA') {
    if (-not [Environment]::GetEnvironmentVariable($name)) { throw "$name is not set; the release workflow prepares it." }
}
$kits = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\bin'
$signtool = Get-ChildItem -LiteralPath $kits -Directory | Where-Object Name -Match '^10\.' |
    Sort-Object { [version]$_.Name } -Descending |
    ForEach-Object { Join-Path $_.FullName 'x64\signtool.exe' } | Where-Object { Test-Path -LiteralPath $_ } |
    Select-Object -First 1
if (-not $signtool) { throw "signtool.exe was not found under $kits." }
& $signtool sign /v /fd SHA256 /tr 'http://timestamp.acs.microsoft.com' /td SHA256 /dlib $(if (Test-Path Env:BATTLE_PLAN_SIGN_DLIB) { $env:BATTLE_PLAN_SIGN_DLIB } else { $env:HOI4_SIGN_DLIB }) /dmdf $(if (Test-Path Env:BATTLE_PLAN_SIGN_METADATA) { $env:BATTLE_PLAN_SIGN_METADATA } else { $env:HOI4_SIGN_METADATA }) $Path
if ($LASTEXITCODE -ne 0) { throw "signtool could not sign $Path (exit $LASTEXITCODE)." }
