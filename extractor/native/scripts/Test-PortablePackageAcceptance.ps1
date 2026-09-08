[CmdletBinding()]
param([string]$ScratchRoot = (Join-Path $env:TEMP ('portable-package-tests-' + [guid]::NewGuid().ToString('N'))))
$ErrorActionPreference='Stop'
Set-StrictMode -Version Latest
# Only synthetic files and a harmless compiled .NET CLI fixture are used.
# No product executable, game, Cargo, release build, or network change is invoked.
$testRoot=[IO.Path]::GetFullPath($ScratchRoot)
. (Join-Path $PSScriptRoot 'PortablePackage.Acceptance.ps1')
if (-not $testRoot.StartsWith('C:\',[StringComparison]::OrdinalIgnoreCase) -or (Test-Path -LiteralPath $testRoot) -or
    (Test-WindowsPathContainedByRoot -Candidate $testRoot -Root ([IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../../..'))))) {
    throw 'Tests require a new C: scratch directory outside the repository.'
}
$null=[IO.Directory]::CreateDirectory($testRoot)
$passed=[Collections.Generic.List[string]]::new()
$utf8=[Text.UTF8Encoding]::new($false)
$fixtureExe=Join-Path $testRoot 'SyntheticPackageCli.exe'
$compiler=[CodeDom.Compiler.CompilerParameters]::new()
$compiler.CompilerOptions='/platform:x64'
$compiler.OutputAssembly=$fixtureExe; $compiler.GenerateExecutable=$true; $compiler.GenerateInMemory=$false
$null=$compiler.ReferencedAssemblies.Add('System.dll')
$null=$compiler.ReferencedAssemblies.Add('System.Web.Extensions.dll')
Add-Type -CompilerParameters $compiler -TypeDefinition @'
using System;
using System.IO;
using System.Collections;
using System.Collections.Generic;
using System.Web.Script.Serialization;
public static class SyntheticPackageCli {
    static JavaScriptSerializer json = new JavaScriptSerializer();
    static string Arg(string[] args, string name) { int i=Array.IndexOf(args,name); return i<0 ? null : args[i+1]; }
    public static int Main(string[] args) {
        System.Threading.Thread.Sleep(300);
        var leaked=new List<string>();
        foreach(DictionaryEntry e in Environment.GetEnvironmentVariables()) {
            string k=(string)e.Key;
            if(System.Text.RegularExpressions.Regex.IsMatch(k,"^(HOI4_|CARGO|RUST|SCCACHE|REPLAY_DESKTOP_)",System.Text.RegularExpressions.RegexOptions.IgnoreCase)) leaked.Add(k);
        }
        Console.WriteLine(json.Serialize(new { fixture=true, executable=System.Reflection.Assembly.GetExecutingAssembly().Location,
            path=Environment.GetEnvironmentVariable("PATH"), localappdata=Environment.GetEnvironmentVariable("LOCALAPPDATA"), leaked=leaked, argv=args }));
        if(Array.IndexOf(args,"--configure")>=0) {
            File.WriteAllText(Arg(args,"--config"),json.Serialize(new {schema_version=1,game_install=Arg(args,"--game"),user_data=Arg(args,"--userdata")}));
            return 0;
        }
        string recording=Arg(args,"--validate-recording") ?? Arg(args,"--validate-playback") ?? Arg(args,"--report-content-authority");
        if(recording==null) return 91; // A desktop/control invocation is a test failure.
        var report=json.Deserialize<Dictionary<string,object>>(File.ReadAllText(recording));
        string mode=(string)report["fixture_mode"];
        if(mode=="exit-failure") return 23;
        if(Array.IndexOf(args,"--validate-playback")>=0 && mode!="missing-whole-playback") {
            Console.WriteLine("{\"schema\":\"replay.validate_playback.stage.v1\",\"stage\":\"all_frame_payloads_and_static_catalog_bindings\",\"result\":\"STARTED\",\"expected_hours\":2}");
            Console.WriteLine("{\"schema\":\"replay.validate_playback.completed.v1\",\"stage\":\"all_frame_payloads_and_static_catalog_bindings\",\"result\":\"PASS\",\"hours_validated\":2}");
        }
        if(Array.IndexOf(args,"--report-content-authority")>=0) File.Copy(recording,Arg(args,"--report-out"));
        return 0;
    }
}
'@

function Test-Check { param([bool]$Condition,[string]$Message) if(-not $Condition){throw $Message} }
function Test-Hash { param([string]$Path) (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant() }
function Test-WriteJson { param([string]$Path,$Object) [IO.File]::WriteAllText($Path,($Object | ConvertTo-Json -Depth 20),$utf8) }
function Test-Pass { param([string]$Name) $passed.Add($Name); Write-Host "PASS $Name" }
function Test-Reject {
    param([string]$Name,[scriptblock]$Action,[string]$Expected)
    $failure=''; try { & $Action } catch { $failure=$_.Exception.Message }
    Test-Check ($failure.Contains($Expected)) "$Name expected '$Expected', got '$failure'"
    Test-Pass $Name
}

function New-TestPackage {
    param([string]$Name,[scriptblock]$MutateEntries=$null,[scriptblock]$MutateReceipt=$null,
        [scriptblock]$MutateVerification=$null,[string]$FixtureMode='pass',[long]$ForgedViewerLength=0)
    $dir=Join-Path $testRoot $Name; $null=[IO.Directory]::CreateDirectory($dir)
    $game=Join-Path $dir 'game root'; $user=Join-Path $dir 'user data'
    $null=[IO.Directory]::CreateDirectory($game); $null=[IO.Directory]::CreateDirectory($user)
    [IO.File]::WriteAllText((Join-Path $game 'hoi4.exe'),'inert fixture, never executable')
    [IO.File]::WriteAllText((Join-Path $user 'dlc_load.json'),'{"enabled_mods":[],"disabled_dlcs":[]}')
    $recording=Join-Path $dir 'approved recording.battleplan'
    Test-WriteJson $recording ([ordered]@{schema='hoi4.replay.content_authority_report.v1'; recording=$recording; fixture_mode=$FixtureMode;
        bundle_content_authority=@{authority_sha256=('a'*64);manifest_digest_sha256=('b'*64);product_modset_sha256=('c'*64);effective_vfs_manifest_sha256=('d'*64)}})
    $binaryBytes=[ordered]@{'Battle Plan Recorder.exe'=[byte[]]@(1,2,3);'Battle Plan.exe'=[IO.File]::ReadAllBytes($fixtureExe);'collector.dll'=[byte[]]@(4,5,6)}
    $hashes=[ordered]@{}
    foreach($key in $binaryBytes.Keys) {
        $stream=[IO.MemoryStream]::new([byte[]]$binaryBytes[$key]); try{$hashes[$key]=Get-PpaStreamHash $stream}finally{$stream.Dispose()}
    }
    $entries=[Collections.Generic.List[object]]::new()
    foreach($key in $binaryBytes.Keys){$entries.Add([pscustomobject]@{name=$key;bytes=$binaryBytes[$key]})}
    $docs=Get-PortableProductPackageDocuments '0.0.0-fixture' $hashes
    foreach($key in $docs.Keys){$entries.Add([pscustomobject]@{name=$key;bytes=$docs[$key]})}
    if($MutateEntries){& $MutateEntries $entries}
    $zip=Join-Path $dir 'closed.zip'; $memory=[IO.MemoryStream]::new()
    try {
        $writer=[IO.Compression.ZipArchive]::new($memory,'Create',$true)
        try{foreach($e in $entries){$out=$writer.CreateEntry($e.name).Open();try{$out.Write($e.bytes,0,$e.bytes.Length)}finally{$out.Dispose()}}}finally{$writer.Dispose()}
        $bytes=$memory.ToArray()
    } finally {$memory.Dispose()}
    if($ForgedViewerLength) {
        $offset=[int][BitConverter]::ToUInt32($bytes,$bytes.Length-6)
        foreach($e in $entries) {
            $nameLength=[BitConverter]::ToUInt16($bytes,$offset+28)
            if($utf8.GetString($bytes,$offset+46,$nameLength) -ceq 'Battle Plan.exe') {
                [BitConverter]::GetBytes([uint32]$ForgedViewerLength).CopyTo($bytes,$offset+24)
            }
            $offset+=46+$nameLength+[BitConverter]::ToUInt16($bytes,$offset+30)+[BitConverter]::ToUInt16($bytes,$offset+32)
        }
    }
    [IO.File]::WriteAllBytes($zip,$bytes); $zipHash=Test-Hash $zip
    $receiptPath=Join-Path $dir 'canonical-receipt.json'
    $receipt=[ordered]@{schema='battle-plan.central_portable_package.v1';status='PASS_CENTRAL_RELEASE_PORTABLE_PACKAGE';successful_exit=$true;
        build_environment=@{sha256=('e'*64)}; package=[ordered]@{schema='hoi4-portable-package.v1';version='0.0.0-fixture';sha256=$zipHash;admissible_for_production=$true;
            input_sha256=[ordered]@{recorder_host=$hashes['Battle Plan Recorder.exe'];replay_app=$hashes['Battle Plan.exe'];collector_dll=$hashes['collector.dll']}}}
    if($MutateReceipt){& $MutateReceipt $receipt}
    Test-WriteJson $receiptPath $receipt; $receiptHash=Test-Hash $receiptPath
    $verificationPath=Join-Path $dir 'canonical-verification.json'
    $verification=[ordered]@{schema='battle-plan.central_portable_package_verification.v1';status='PASS_CENTRAL_RELEASE_PORTABLE_PACKAGE';
        canonical_receipt=$true;static_vcruntime_admission=$true;receipt_sha256=$receiptHash;package_sha256=$zipHash;build_environment_sha256=('e'*64)}
    if($MutateVerification){& $MutateVerification $verification}
    Test-WriteJson $verificationPath $verification
    return @{Mode='Prepare';TimeoutSeconds=5;PackageZip=$zip;ExpectedPackageSha256=$zipHash;
        PackageReceiptPath=$receiptPath;ExpectedPackageReceiptSha256=$receiptHash;
        PackageVerificationPath=$verificationPath;ExpectedPackageVerificationSha256=(Test-Hash $verificationPath);
        RecordingPath=$recording;ExpectedRecordingSha256=(Test-Hash $recording);GameRoot=$game;ExpectedGameExeSha256=(Test-Hash (Join-Path $game 'hoi4.exe'));
        UserDataRoot=$user;ExpectedDlcLoadSha256=(Test-Hash (Join-Path $user 'dlc_load.json'));
        ExpectedContentAuthoritySha256=('a'*64);ExpectedEffectiveVfsManifestSha256=('d'*64);OutputDirectory=(Join-Path $dir 'acceptance')}
}
function Test-AcceptanceResult {
    param([string]$Name,$Request,[string]$ExpectedFailure='')
    $result=Invoke-PpaAcceptance $Request
    if($ExpectedFailure) {
        Test-Check ($result.status -ceq 'fail' -and $result.error.Contains($ExpectedFailure) -and -not $result.cli_playback_qualified) "$Name unexpected result: $($result | ConvertTo-Json -Depth 4 -Compress)"
    } else {Test-Check ($result.status -cne 'fail') "$Name failed: $($result.error)"}
    if($null -ne $result.package_after){Test-Check $result.package_after.same_bytes "$Name did not preserve closed ZIP"}
    Test-Pass $Name
    return $result
}

$request=New-TestPackage 'prepare'
$result=Test-AcceptanceResult 'Prepare verifies five entries, launches nothing' $request
Test-Check ($result.status -ceq 'prepared_not_run' -and $result.entries.Count -eq 5 -and $result.cases.Count -eq 0 -and
    $result.extracted.Count -eq 0 -and -not (Test-Path -LiteralPath (Join-Path $request.OutputDirectory 'installed'))) 'Prepare extracted or launched'
Test-Check ($result.plan.Count -eq 4 -and -not $result.desktop_handoff.invoked -and $result.desktop_handoff.parameters.ExpectedViewerSha256 -ceq (Test-Hash $fixtureExe)) 'Plan/handoff identity mismatch'
$disk=Get-Content -LiteralPath (Join-Path $request.OutputDirectory 'PACKAGE_ACCEPTANCE.json') -Raw | ConvertFrom-Json
Test-Check ($disk.package_after.same_bytes -and -not $disk.cli_playback_qualified) 'Prepare receipt promoted execution'

# Exercise the public parameter-binding entry point in a separate interpreter.
$request=New-TestPackage 'public-prepare'
$argv=@('-NoProfile','-ExecutionPolicy','Bypass','-File',(Join-Path $PSScriptRoot 'Invoke-PortablePackageAcceptance.ps1'))
foreach($key in $request.Keys){$argv+=@("-$key",[string]$request[$key])}
$public=& (Join-Path $PSHOME 'powershell.exe') @argv
Test-Check ($LASTEXITCODE -eq 0) "Public Prepare failed: $public"
Test-Check (($public | Out-String | ConvertFrom-Json).status -ceq 'prepared_not_run') 'Public Prepare parameter capture failed'
Test-Pass 'public Prepare wrapper binds explicit inputs without launch'

foreach($key in @('ExpectedPackageSha256','ExpectedPackageReceiptSha256','ExpectedPackageVerificationSha256','ExpectedRecordingSha256','ExpectedGameExeSha256','ExpectedDlcLoadSha256')) {
    $request=New-TestPackage "bad-$key"; $request[$key]='f'*64
    $null=Test-AcceptanceResult "reject $key byte mismatch" $request 'PPA_HASH_MISMATCH'
}
$request=New-TestPackage 'unavailable'; $request.RecordingPath=Join-Path (Split-Path $request.RecordingPath) 'missing.battleplan'
$null=Test-AcceptanceResult 'reject unavailable explicit recording' $request 'PPA_FILE_OPEN'
$request=New-TestPackage 'staged' -MutateVerification {param($v)$v.canonical_receipt=$false}
$null=Test-AcceptanceResult 'reject staged verification' $request 'PPA_PACKAGE_AUTHORITY'
$request=New-TestPackage 'foreign-receipt' -MutateVerification {param($v)$v.receipt_sha256='f'*64}
$null=Test-AcceptanceResult 'reject verification of another receipt' $request 'PPA_PACKAGE_AUTHORITY'
$request=New-TestPackage 'string-boolean' -MutateVerification {param($v)$v.static_vcruntime_admission='true'}
$null=Test-AcceptanceResult 'reject untyped authority boolean' $request 'PPA_PACKAGE_AUTHORITY'
$request=New-TestPackage 'foreign-role' -MutateReceipt {param($r)$r.package.input_sha256.domain_roster='f'*64}
$null=Test-AcceptanceResult 'reject diagnostic role in shipping roster' $request 'PPA_BINARY_ROLES'
$request=New-TestPackage 'binary-tamper' -MutateEntries {param($e)$e[1].bytes[0]=0}
$null=Test-AcceptanceResult 'reject changed binary with otherwise canonical documents' $request 'PPA_BINARY_HASH'
$request=New-TestPackage 'extra-entry' -MutateEntries {param($e)$e.Add([pscustomobject]@{name='extra.dll';bytes=[byte[]]@(1)})}
$null=Test-AcceptanceResult 'reject extra ZIP member' $request 'exactly five entries'
foreach($name in @('../escape.exe','Battle Plan.exe','Battle Plan Recorder.exe','sub/Battle Plan.exe')) {
    $request=New-TestPackage ('alias-'+[guid]::NewGuid().ToString('N')) -MutateEntries {param($e)$e[1].name=$name}
    $null=Test-AcceptanceResult "reject ZIP alias/traversal/duplicate: $name" $request 'missing or duplicated'
}
$request=New-TestPackage 'document-tamper' -MutateEntries {param($e)$e[3].bytes[0]=0}
$null=Test-AcceptanceResult 'reject changed canonical README' $request 'document bytes mismatch'
$request=New-TestPackage 'entry-bound' -ForgedViewerLength (256MB+1)
$null=Test-AcceptanceResult 'reject forged oversized binary before decompression' $request 'PPA_ENTRY_BOUND'
$request=New-TestPackage 'entry-truncated' -ForgedViewerLength 1
$null=Test-AcceptanceResult 'reject binary exceeding advertised length' $request 'PPA_ENTRY_LENGTH'
$request=New-TestPackage 'existing-output'; $null=[IO.Directory]::CreateDirectory($request.OutputDirectory)
$null=Test-AcceptanceResult 'preserve existing output' $request 'PPA_OUTPUT_EXISTS'
$request=New-TestPackage 'source-output'; $request.OutputDirectory=Join-Path $request.GameRoot 'acceptance'
$null=Test-AcceptanceResult 'reject output inside content inputs' $request 'PPA_OUTPUT_LOCATION'
Test-Reject 'reject UNC paths without network resolution' {Assert-PpaLocalPath '\\server\share\input.zip'} 'PPA_LOCAL_PATH'
Test-Reject 'reject alternate data streams' {Assert-PpaLocalPath 'C:\input.zip:stream'} 'PPA_LOCAL_PATH'

$request=New-TestPackage 'leased'
$moveFrom=[IO.Path]::GetFullPath((Split-Path $request.RecordingPath));$moveTo=[IO.Path]::GetFullPath((Join-Path $testRoot 'moved'))
Test-Check ((Test-WindowsPathContainedByRoot -Candidate $moveFrom -Root $testRoot) -and
    (Test-WindowsPathContainedByRoot -Candidate $moveTo -Root $testRoot)) 'Fixture moves escaped owned scratch'
$leases=[Collections.Generic.List[object]]::new()
try {
    $file=Open-PpaFile $request.RecordingPath $request.ExpectedRecordingSha256 $leases
    Test-Reject 'read lease denies input writes' { [IO.File]::Open($request.RecordingPath,'Open','Write','ReadWrite').Dispose() } 'another process'
    Test-Reject 'read lease denies input replacement' { [IO.File]::Move($request.RecordingPath,($request.RecordingPath+'.moved')) } 'another process'
    Test-Reject 'ancestor lease denies directory replacement' { [IO.Directory]::Move($moveFrom,$moveTo) } 'denied'
}finally{foreach($lease in $leases){$lease.Dispose()}}
$writer=[IO.File]::Open($request.RecordingPath,'Open','Write','ReadWrite');$writer.Dispose()
Test-Pass 'all input leases release after scope exit'
$reportPath=Join-Path $moveFrom 'content-authority.json'
Test-WriteJson $reportPath @{schema='leased-synthetic-output';value='validated'}
$leases=[Collections.Generic.List[object]]::new()
try {
    $reportFile=Open-PpaReadLease $reportPath $leases 8MB
    $parsed=Read-PpaJson $reportFile
    Test-Check ($parsed.value -ceq 'validated' -and $reportFile.reference.sha256 -ceq (Test-Hash $reportPath)) 'Report parse/hash custody differs'
    Test-Reject 'validated report lease denies replacement before receipt' { [IO.File]::Move($reportPath,($reportPath+'.moved')) } 'another process'
    Test-Reject 'validated report lease denies writes before receipt' { [IO.File]::WriteAllText($reportPath,'unvalidated') } 'another process'
    Test-Check ($reportFile.reference.sha256 -ceq (Get-PpaStreamHash $reportFile.stream)) 'Report reference changed after validation'
}finally{foreach($lease in $leases){$lease.Dispose()}}
$leases=[Collections.Generic.List[object]]::new()
try{Test-Reject 'bound report before parsing/hashing' {Open-PpaReadLease $reportPath $leases 1} 'PPA_READ_BOUND'}finally{foreach($lease in $leases){$lease.Dispose()}}
# Native junction fixture: no shell deletion/move and no external target.
$junction=Join-Path (Split-Path $request.RecordingPath) 'junction'
$null=New-Item -ItemType Junction -Path $junction -Target $request.GameRoot
$leases=[Collections.Generic.List[object]]::new()
try{Test-Reject 'reject junction ancestor during lease acquisition' {Lock-PpaDirectory $junction $leases} 'PPA_REPARSE_OR_TYPE'}finally{foreach($lease in $leases){$lease.Dispose()}}

foreach($scenario in @('elevated','system','online','untyped')) {
    $sample=@{non_elevated=$true;is_system=$false;network_available=$false}
    switch($scenario){elevated{$sample.non_elevated=$false}system{$sample.is_system=$true}online{$sample.network_available=$true}untyped{$sample.non_elevated='true'}}
    $expected=if($scenario -eq 'online'){'PPA_OFFLINE'}else{'PPA_STANDARD_USER'}
    Test-Reject "reject $scenario environment" {Assert-PpaEnvironment $sample} $expected
}

# Environment substitution exists only in this test script, exclusively for the
# inert fixture. Production Run always measures the actual executing token/NICs.
$realEnvironment=${function:Get-PpaEnvironment}
$script:environmentCalls=0; $script:onlineAt=0
$script:standardToken=$true; $script:systemToken=$false
function Get-PpaEnvironment {
    $script:environmentCalls++
    @{utc=[DateTime]::UtcNow.ToString('o');machine=[Environment]::MachineName;user_sid='SYNTHETIC';non_elevated=$script:standardToken;is_system=$script:systemToken;
        network_available=($script:onlineAt -gt 0 -and $script:environmentCalls -ge $script:onlineAt);fixture=$true}
}
$savedEnv=@{}
foreach($name in @('BATTLE_PLAN_PPA_TEST','CARGO_HOME','RUSTUP_HOME','SCCACHE_PPA_TEST','REPLAY_DESKTOP_DIAGNOSTICS')) {
    $savedEnv[$name]=[Environment]::GetEnvironmentVariable($name);[Environment]::SetEnvironmentVariable($name,'must-not-reach-child')
}
try {
    foreach($scenario in @('online','elevated','system')) {
        $script:environmentCalls=0;$script:onlineAt=0;$script:standardToken=$true;$script:systemToken=$false
        switch($scenario){online{$script:onlineAt=1}elevated{$script:standardToken=$false}system{$script:systemToken=$true}}
        $request=New-TestPackage "run-preflight-$scenario";$request.Mode='Run'
        $expected=if($scenario -eq 'online'){'PPA_OFFLINE'}else{'PPA_STANDARD_USER'}
        $result=Test-AcceptanceResult "Run independently rejects $scenario state before extraction/launch" $request $expected
        Test-Check ($result.cases.Count -eq 0 -and $result.extracted.Count -eq 0) 'Invalid environment reached launch/extraction'
    }
    $script:environmentCalls=0;$script:onlineAt=0;$script:standardToken=$true;$script:systemToken=$false
    $request=New-TestPackage 'run-fixture';$request.Mode='Run'
    $result=Test-AcceptanceResult 'synthetic Run uses actual extracted CLI process seam' $request
    Test-Check ($result.status -ceq 'pass' -and $result.cli_playback_qualified -and $result.cases.Count -eq 4 -and $result.extracted.Count -eq 5) 'Run incomplete'
    Test-Check ($result.environment_samples.Count -eq 9 -and $result.package_after.same_bytes) 'Run lost environment or post-package evidence'
    foreach($case in $result.cases) {
        $raw=Get-Content -LiteralPath $case.stdout.path -First 1 | ConvertFrom-Json
        Test-Check ($raw.fixture -and $raw.executable -ieq (Join-Path $request.OutputDirectory 'installed/Battle Plan.exe') -and
            $raw.path -ieq (Join-Path $env:SystemRoot 'System32') -and $raw.leaked.Count -eq 0 -and
            $raw.localappdata -ieq (Join-Path $request.OutputDirectory 'local-app-data')) 'Child resolved source binary or inherited development environment'
    }
    Test-Check (-not $result.desktop_handoff.invoked -and $result.desktop_handoff.parameters.ViewerExecutable -ieq $result.cases[0].executable) 'Desktop handoff escaped extracted viewer'
    Test-Pass 'all four children use exact extracted viewer and scrubbed environment'
    foreach($pair in @(@('missing-whole-playback','VDP_WHOLE_PLAYBACK_RECEIPT_MISSING'),@('exit-failure','VDP_PRODUCT_EXIT_FAILED'))) {
        $request=New-TestPackage $pair[0] -FixtureMode $pair[0];$request.Mode='Run'
        $result=Test-AcceptanceResult "retain raw failure: $($pair[0])" $request $pair[1]
        Test-Check ($result.package_after.same_bytes -and $result.cases.Count -gt 0 -and $result.cases[-1].stdout.sha256.Length -eq 64) 'Failure lost raw/post-package evidence'
    }
    foreach($key in @('ExpectedContentAuthoritySha256','ExpectedEffectiveVfsManifestSha256')) {
        $request=New-TestPackage "run-bad-$key";$request.Mode='Run';$request[$key]='f'*64
        $expected=if($key -eq 'ExpectedContentAuthoritySha256'){'VDP_CONTENT_AUTHORITY_MISMATCH'}else{'PPA_ROOT_MANIFEST_MISMATCH'}
        $null=Test-AcceptanceResult "reject actual CLI $key mismatch" $request $expected
    }
    $script:environmentCalls=0;$script:onlineAt=3
    $request=New-TestPackage 'online-after-child';$request.Mode='Run'
    $result=Test-AcceptanceResult 'network reappearance after CLI prevents acceptance' $request 'PPA_OFFLINE'
    Test-Check ($result.cases.Count -eq 1 -and $result.package_after.same_bytes) 'Network change did not stop remaining children'
}finally{
    Set-Item function:Get-PpaEnvironment $realEnvironment
    foreach($name in $savedEnv.Keys){[Environment]::SetEnvironmentVariable($name,$savedEnv[$name])}
}
$summary=[ordered]@{schema='hoi4.portable-package-acceptance-tests.v1';status='pass';passed=$passed.Count;cases=@($passed.ToArray());scratch=$testRoot;
    fixture_executable_sha256=(Test-Hash $fixtureExe);scope='Synthetic ZIPs and inert .NET CLI only. No product/game/release execution or actual offline/nonadmin package qualification.'}
Test-WriteJson (Join-Path $testRoot 'TEST_RESULTS.json') $summary
$summary | ConvertTo-Json -Depth 8
