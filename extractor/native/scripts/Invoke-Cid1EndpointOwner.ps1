[CmdletBinding()]
param(
    [ValidateSet('Observe','Finalize')][string]$Mode,
    [Parameter(Mandatory=$true)][string]$EndpointRoot,
    [Parameter(Mandatory=$true)][ValidatePattern('^[A-Za-z0-9_]{2,64}$')][string]$Role,
    [uint32]$ProcessId,
    [string]$SavePath,
    [string]$StaticDisasmPath,
    [string]$RakalyPath,
    [string]$RolloverSetDescriptorPath,
    [int]$LogicalStreamId = -1,
    [string]$Python = 'python.exe'
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$expectedExeSha256 = '870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad'
$root = [IO.Path]::GetFullPath($EndpointRoot)
$pendingPath = Join-Path $root 'endpoint_pending.json'
$descriptorPath = Join-Path $root 'endpoint_descriptor.json'

function Get-Sha256([string]$Path) { (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant() }
function New-Nonce { $b=[byte[]]::new(32);[Security.Cryptography.RandomNumberGenerator]::Fill($b);[Convert]::ToHexString($b).ToLowerInvariant() }
function Write-CreateNewJson([string]$Path,$Value) {
    $full=[IO.Path]::GetFullPath($Path);$parent=[IO.Path]::GetDirectoryName($full);[IO.Directory]::CreateDirectory($parent)|Out-Null
    if([IO.File]::Exists($full)){throw "Refusing to replace immutable CID1 endpoint artifact: $full"}
    $temporary=[IO.Path]::Combine($parent,([IO.Path]::GetRandomFileName()+'.tmp'))
    try { [IO.File]::WriteAllText($temporary,(($Value|ConvertTo-Json -Depth 20)+"`n"),[Text.UTF8Encoding]::new($false));$stream=[IO.File]::Open($temporary,[IO.FileMode]::Open,[IO.FileAccess]::ReadWrite,[IO.FileShare]::None);try{$stream.Flush($true)}finally{$stream.Dispose()};[IO.File]::Move($temporary,$full) }
    finally { if([IO.File]::Exists($temporary)){[IO.File]::Delete($temporary)} }
}

$pythonCommand=Get-Command $Python -ErrorAction Stop
$pythonPath=[string]$pythonCommand.Source
$collector=(Resolve-Path -LiteralPath (Join-Path $PSScriptRoot 'collect_campaign_identity_evidence.py')).Path
$verifier=(Resolve-Path -LiteralPath (Join-Path $PSScriptRoot 'verify_campaign_identity_evidence.py')).Path
$boundary=(Resolve-Path -LiteralPath (Join-Path $PSScriptRoot 'materialize_cid1_stream_boundary.py')).Path
$oracleTool=(Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..\..\save_oracle\parse_campaign_identity.py')).Path

if($Mode -eq 'Observe') {
    foreach($required in @($SavePath,$StaticDisasmPath)){if([string]::IsNullOrWhiteSpace($required)-or-not(Test-Path -LiteralPath $required -PathType Leaf)){throw "Observe requires existing input: $required"}}
    if(Test-Path -LiteralPath $root){throw "Refusing to reuse CID1 endpoint root: $root"}
    [IO.Directory]::CreateDirectory($root)|Out-Null
    $process=Get-Process -Id $ProcessId -ErrorAction Stop
    $exe=[string]$process.MainModule.FileName
    if((Get-Sha256 $exe)-cne $expectedExeSha256){throw 'Endpoint process is not exact a729.'}
    $creation=$process.StartTime.ToFileTimeUtc()
    $machineGuid=[string](Get-ItemPropertyValue -LiteralPath 'HKLM:\SOFTWARE\Microsoft\Cryptography' -Name MachineGuid -ErrorAction Stop)
    $machineBytes=[Text.Encoding]::UTF8.GetBytes(('hoi4-cid1-machine-v1'+[char]0+$machineGuid.ToLowerInvariant()))
    $machineId=[Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($machineBytes)).ToLowerInvariant()
    $nonce=New-Nonce;$session=('cid1_'+$Role+'_'+$nonce.Substring(0,16));$ledger=Join-Path $root 'nonce_ledger';$transcript=Join-Path $root 'campaign_identity_transcript.json'
    & $pythonPath $collector init --pid $ProcessId --session-id $session --evidence-nonce $nonce --nonce-ledger-root $ledger --static-disasm (Resolve-Path -LiteralPath $StaticDisasmPath).Path --out $transcript --action-token AUTHORIZE_READ_ONLY_CAMPAIGN_IDENTITY_EVIDENCE_INIT
    if($LASTEXITCODE-ne 0){throw 'CID1 endpoint transcript init failed.'}
    $actionToken='OBSERVE_CAMPAIGN_IDENTITY_'+$Role+'_AUTOMATIC_SAME_HOUR_V1'
    & $pythonPath $collector observe --transcript $transcript --label $Role --barrier-mode automatic_same_hour --action-token $actionToken
    if($LASTEXITCODE-ne 0){throw 'CID1 endpoint automatic same-hour observation failed.'}
    & $pythonPath $collector seal --transcript $transcript --action-token SEAL_CAMPAIGN_IDENTITY_TRANSCRIPT_WITHOUT_VERDICT
    if($LASTEXITCODE-ne 0){throw 'CID1 endpoint transcript seal failed.'}
    $transcriptJson=Get-Content -LiteralPath $transcript -Raw|ConvertFrom-Json

    $oracleRoot=Join-Path $root 'save_oracle';$oracleArgs=@($oracleTool,(Resolve-Path -LiteralPath $SavePath).Path,'--out-dir',$oracleRoot)
    if(-not[string]::IsNullOrWhiteSpace($RakalyPath)){$oracleArgs+=@('--rakaly',(Resolve-Path -LiteralPath $RakalyPath).Path)}
    & $pythonPath @oracleArgs
    if($LASTEXITCODE-ne 0){throw 'CID1 endpoint save oracle failed.'}
    $oracle=Join-Path $oracleRoot 'campaign_identity_oracle.json';$oracleJson=Get-Content -LiteralPath $oracle -Raw|ConvertFrom-Json
    if($oracleJson.identity.pair_valid-ne $true){throw 'CID1 endpoint save oracle has no valid identity pair.'}
    $gate=Join-Path $root 'campaign_identity_gate.json'
    & $pythonPath $verifier --transcript $transcript --nonce-ledger-root $ledger --expected-transcript-sha256 ([string]$transcriptJson.transcript_sha256) --expected-session-id $session --expected-evidence-nonce $nonce --expected-exe-sha256 $expectedExeSha256 --expected-probe-tool-sha256 (Get-Sha256 $collector) --expected-static-disasm-sha256 (Get-Sha256 $StaticDisasmPath) --expected-seed ([int]$oracleJson.identity.game_unique_seed) --expected-id ([string]$oracleJson.identity.game_unique_id) --out $gate
    if($LASTEXITCODE-ne 0){throw 'CID1 endpoint independent transcript gate failed.'}
    Write-CreateNewJson $pendingPath ([ordered]@{schema='battle-plan.campaign_identity_endpoint_pending.v1';role=$Role;machine_id=$machineId;process_id=$ProcessId;process_creation_filetime_utc=$creation;executable_sha256=$expectedExeSha256;save_path=(Resolve-Path -LiteralPath $SavePath).Path;save_sha256=Get-Sha256 $SavePath;transcript_path=$transcript;transcript_file_sha256=Get-Sha256 $transcript;transcript_sha256=[string]$transcriptJson.transcript_sha256;gate_path=$gate;gate_sha256=Get-Sha256 $gate;oracle_path=$oracle;oracle_sha256=Get-Sha256 $oracle})
    Write-Output $pendingPath
    return
}

if(-not(Test-Path -LiteralPath $pendingPath -PathType Leaf)){throw 'Finalize requires the immutable endpoint_pending.json.'}
if([string]::IsNullOrWhiteSpace($RolloverSetDescriptorPath)-or-not(Test-Path -LiteralPath $RolloverSetDescriptorPath -PathType Leaf)){throw 'Finalize requires the durable logical-stream-set descriptor.'}
if($LogicalStreamId-lt 0){throw 'Finalize requires a non-negative logical stream id.'}
$pending=Get-Content -LiteralPath $pendingPath -Raw|ConvertFrom-Json
if($pending.schema-cne 'battle-plan.campaign_identity_endpoint_pending.v1'-or$pending.role-cne $Role-or$pending.executable_sha256-cne $expectedExeSha256){throw 'Endpoint pending authority mismatch.'}
foreach($binding in @(@($pending.transcript_path,$pending.transcript_file_sha256),@($pending.gate_path,$pending.gate_sha256),@($pending.oracle_path,$pending.oracle_sha256))){if(-not(Test-Path -LiteralPath $binding[0] -PathType Leaf)-or(Get-Sha256 $binding[0])-cne [string]$binding[1]){throw 'Endpoint pending artifact drifted before finalization.'}}
$streamBoundary=Join-Path $root 'stream_boundary.json'
& $pythonPath $boundary --rollover-set (Resolve-Path -LiteralPath $RolloverSetDescriptorPath).Path --logical-stream-id $LogicalStreamId --transcript-sha256 ([string]$pending.transcript_sha256) --out $streamBoundary
if($LASTEXITCODE-ne 0){throw 'CID1 endpoint raw stream-boundary materialization failed.'}
Write-CreateNewJson $descriptorPath ([ordered]@{role=$Role;machine_id=[string]$pending.machine_id;transcript_path=[string]$pending.transcript_path;transcript_file_sha256=[string]$pending.transcript_file_sha256;transcript_sha256=[string]$pending.transcript_sha256;gate_path=[string]$pending.gate_path;gate_sha256=[string]$pending.gate_sha256;oracle_path=[string]$pending.oracle_path;oracle_sha256=[string]$pending.oracle_sha256;stream_boundary_path=$streamBoundary;stream_boundary_sha256=Get-Sha256 $streamBoundary})
Write-Output $descriptorPath
