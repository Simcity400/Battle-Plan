[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Path,
    [Parameter(Mandatory)][string]$Scratch
)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$names = 'AZURE_TENANT_ID', 'AZURE_CLIENT_ID', 'AZURE_CLIENT_SECRET', 'AZURE_SIGNING_ENDPOINT', 'AZURE_SIGNING_ACCOUNT', 'AZURE_SIGNING_PROFILE'
$missing = @($names | Where-Object { -not [Environment]::GetEnvironmentVariable($_) })
if ($missing.Count -gt 0) { throw "Authenticode signing needs $($missing -join ', ')." }
$kits = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\bin'
$signtool = Get-ChildItem -LiteralPath $kits -Directory | Where-Object Name -Match '^10\.' |
    Sort-Object { [version]$_.Name } -Descending |
    ForEach-Object { Join-Path $_.FullName 'x64\signtool.exe' } | Where-Object { Test-Path -LiteralPath $_ } |
    Select-Object -First 1
if (-not $signtool) { throw "signtool.exe was not found under $kits." }

New-Item -ItemType Directory -Force -Path $Scratch | Out-Null
$package = Join-Path $Scratch 'artifact-signing.zip'
Invoke-WebRequest -UseBasicParsing -OutFile $package -Uri 'https://api.nuget.org/v3-flatcontainer/microsoft.artifactsigning.client/1.0.151/microsoft.artifactsigning.client.1.0.151.nupkg'
$digest = (Get-FileHash -LiteralPath $package -Algorithm SHA256).Hash.ToLowerInvariant()
if ($digest -ne '35375a4bc4e1102da729fc3d8d9925f774c3baae6b8b7ff6398d948ca9ab0d8a') { throw "The Artifact Signing client's SHA256 is $digest, not the pinned one." }
$client = Join-Path $Scratch 'artifact-signing'
Expand-Archive -LiteralPath $package -DestinationPath $client -Force
$dlib = Join-Path $client 'bin\x64\Azure.CodeSigning.Dlib.dll'
if (-not (Test-Path -LiteralPath $dlib)) { throw 'The Artifact Signing client has no bin\x64\Azure.CodeSigning.Dlib.dll.' }
$metadata = Join-Path $Scratch 'artifact-signing.json'
[IO.File]::WriteAllText($metadata, (@{ Endpoint = $env:AZURE_SIGNING_ENDPOINT; CodeSigningAccountName = $env:AZURE_SIGNING_ACCOUNT; CertificateProfileName = $env:AZURE_SIGNING_PROFILE } | ConvertTo-Json))

& $signtool sign /fd SHA256 /tr 'http://timestamp.acs.microsoft.com' /td SHA256 /dlib $dlib /dmdf $metadata $Path
if ($LASTEXITCODE -ne 0) { throw "signtool could not sign $(Split-Path -Leaf $Path) (exit $LASTEXITCODE)." }
$signature = Get-AuthenticodeSignature -LiteralPath $Path
if ($signature.Status -ne 'Valid') { throw "The installer's Authenticode signature is $($signature.Status): $($signature.StatusMessage)" }
Write-Output "Authenticode: $(Split-Path -Leaf $Path) is signed, and the signature is valid."
