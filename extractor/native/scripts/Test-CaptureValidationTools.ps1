[CmdletBinding()]
param([string]$OutputDirectory = '')
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot '../../../portable_replay/native/scripts/ProductionArtifact.Attestation.ps1')
. (Join-Path $PSScriptRoot 'CaptureValidation.Tools.ps1')
if (-not $OutputDirectory) {
    $OutputDirectory = Join-Path ([IO.Path]::GetTempPath()) ('capture-tool-units-' + [Guid]::NewGuid().ToString('N'))
}
if (Test-Path -LiteralPath $OutputDirectory) { throw 'Use a new test output directory.' }
$scratch = [IO.Path]::GetFullPath($OutputDirectory)
$null = New-Item -ItemType Directory -Path $scratch
$repo = Join-Path $scratch 'fixture-repo'
$logs = Join-Path $scratch 'evidence'
$null = New-Item -ItemType Directory -Path $logs
$plans = @(Get-CaptureValidationToolPlan $repo)
$cargoPath = Join-Path $scratch 'cargo.exe'
[IO.File]::WriteAllBytes($cargoPath, [byte[]]@(1, 2, 3))
$source = [ordered]@{ schema = 'battle-plan.production_rust_input_digest.v1'; sha256 = 'a' * 64; file_count = 3 }
$builds = @()
foreach ($plan in $plans) {
    $null = [IO.Directory]::CreateDirectory((Split-Path -Parent $plan.Source))
    $null = [IO.Directory]::CreateDirectory((Split-Path -Parent $plan.Executable))
    [IO.File]::WriteAllText($plan.Source, 'fixture only; not compiler evidence')
    if ($plan.Role -ceq 'domain_roster') {
        # Harmless executable fixture for the actual driver's roster call below.
        # No Cargo, Rust target, recorder, collector or game is built/launched.
        Add-Type -OutputAssembly $plan.Executable -OutputType ConsoleApplication -TypeDefinition @'
using System;
using System.IO;
public static class RosterFixture {
    public static int Main(string[] args) {
        if (args.Length != 3 || args[0] != "--native-root" || args[2] != "--json") return 9;
        File.WriteAllText(Path.Combine(args[1], "roster-invoked.txt"), "fixture only");
        Console.WriteLine("[]");
        return 0;
    }
}
'@
    } else { [IO.File]::WriteAllBytes($plan.Executable, [byte[]]@(4, 5, 6)) }
    $stdoutPath = Join-Path $logs ($plan.Role + '.stdout.log')
    $stderrPath = Join-Path $logs ($plan.Role + '.stderr.log')
    $artifact = [ordered]@{ reason = 'compiler-artifact'; target = @{ name = $plan.Target; kind = @('bin'); src_path = $plan.Source }
        profile = @{ test = $false }; executable = $plan.Executable }
    [IO.File]::WriteAllText($stdoutPath, ($artifact | ConvertTo-Json -Depth 5 -Compress) + "`n" + '{"reason":"build-finished","success":true}' + "`n")
    [IO.File]::WriteAllText($stderrPath, 'fixture build diagnostics')
    $stdout = Get-CaptureToolFileReference $stdoutPath; $stdout.path = [IO.Path]::GetFileName($stdoutPath)
    $stderr = Get-CaptureToolFileReference $stderrPath; $stderr.path = [IO.Path]::GetFileName($stderrPath)
    $builds += [ordered]@{ role = $plan.Role; arguments = $plan.Arguments; working_directory = $repo
        exit_code = 0; started = $true; timed_out = $false; problem = $null
        cargo = Get-CaptureToolFileReference $cargoPath
        stdout = $stdout; stderr = $stderr; artifact = Get-CaptureToolFileReference $plan.Executable }
}
$fixtureHash = @{ sha256 = 'c' * 64 }
$fixtureEnv = [ordered]@{}
foreach ($key in $script:ProductionBuildEnvironmentKeys) { $fixtureEnv[$key] = '' }
$environment = New-ProductionBuildEnvironmentRecord -RustcVerbose ("host: x86_64-pc-windows-msvc`ncommit-hash: " + ('d' * 40)) `
    -CargoVersion 'synthetic test fixture' -Linker @{ version_output = 'fixture'; executable = $fixtureHash } `
    -RustcWrapper @{ active = $false } -MsvcToolset @{ version = 'fixture'; compiler = $fixtureHash } `
    -WindowsSdk @{ version = 'fixture'; kernel32_import_library = $fixtureHash; ucrt_import_library = $fixtureHash } `
    -Runner @{ os_description = 'fixture'; process_architecture = 'X64' } -BuildEnv $fixtureEnv
$good = [ordered]@{ schema = 'hoi4.capture-validation-tools.v1'; authority = 'diagnostic_tools_not_shipping_artifacts'
    source_before = $source; source_after = $source; build_environment = $environment; builds = $builds } | ConvertTo-Json -Depth 12
$receiptPath = Join-Path $logs 'capture-validation-tools.json'
$passed = [Collections.Generic.List[string]]::new()
function Assert-ToolCase {
    param([string]$Name, [scriptblock]$Mutate, [string]$ExpectedFailure = '')
    $receipt = $good | ConvertFrom-Json
    if ($Mutate) { & $Mutate $receipt }
    [IO.File]::WriteAllText($receiptPath, ($receipt | ConvertTo-Json -Depth 12))
    $authority = $null; $failure = ''
    try { $authority = Open-CaptureValidationTools -RepoRoot $repo -ReceiptPath $receiptPath -ExpectedSourceInputSha256 $source.sha256 }
    catch { $failure = $_.Exception.Message }
    finally { Close-CaptureValidationTools $authority }
    if ($ExpectedFailure) {
        if (-not $failure.Contains($ExpectedFailure)) { throw "$Name failed at unexpected boundary: $failure" }
    } elseif ($failure) { throw "$Name failed: $failure" }
    $passed.Add($Name)
}
Assert-ToolCase 'complete separate diagnostic receipt'
Assert-ToolCase 'shipping receipt is not a tools receipt' { param($r) $r.schema = 'battle-plan.central_rust_prebuilt_artifacts.v1' } 'current unchanged production source'
Assert-ToolCase 'stale source rejected' { param($r) $r.source_before.sha256 = 'b' * 64 } 'current unchanged production source'
Assert-ToolCase 'source changed during build rejected' { param($r) $r.source_after.file_count = 4 } 'current unchanged production source'
Assert-ToolCase 'empty source rejected' { param($r) $r.source_before.file_count = 0; $r.source_after.file_count = 0 } 'current unchanged production source'
Assert-ToolCase 'changed build environment rejected' { param($r) $r.build_environment.sha256 = 'b' * 64 } 'build environment record'
Assert-ToolCase 'missing role rejected' { param($r) $r.builds = @($r.builds[0], $r.builds[1]) } 'exactly 4 owning builds'
Assert-ToolCase 'old three-role receipt rejected' { param($r) $r.builds = @($r.builds | Where-Object { $_.role -cne 'domain_roster' }) } 'exactly 4 owning builds'
Assert-ToolCase 'duplicate role rejected' { param($r) $r.builds[2].role = $r.builds[0].role } 'Missing or duplicate'
Assert-ToolCase 'nonzero build rejected' { param($r) $r.builds[0].exit_code = 7 } 'build failed'
Assert-ToolCase 'null exit rejected' { param($r) $r.builds[0].exit_code = $null } 'build failed'
Assert-ToolCase 'string exit rejected' { param($r) $r.builds[0].exit_code = '0' } 'build failed'
Assert-ToolCase 'timeout despite zero exit rejected' { param($r) $r.builds[0].timed_out = $true } 'build failed'
Assert-ToolCase 'not started rejected' { param($r) $r.builds[0].started = $false } 'build failed'
Assert-ToolCase 'wrong target invocation rejected' { param($r) $r.builds[0].arguments[8] = 'replay_app' } 'build failed'
Assert-ToolCase 'different worktree rejected' { param($r) $r.builds[0].working_directory = $scratch } 'build failed'
Assert-ToolCase 'log traversal rejected' { param($r) $r.builds[0].stdout.path = '../outside.log' } 'canonical relative filename'
Assert-ToolCase 'foreign executable rejected' { param($r) $r.builds[0].artifact.path = $cargoPath } 'owning worktree output'
Assert-ToolCase 'artifact changed rejected' { param($r) $r.builds[0].artifact.sha256 = 'b' * 64 } 'Changed or malformed'
Assert-ToolCase 'log changed rejected' { param($r) $r.builds[0].stdout.sha256 = 'b' * 64 } 'Changed or malformed'
Assert-ToolCase 'compiler binary changed rejected' { param($r) $r.builds[0].cargo.sha256 = 'b' * 64 } 'Changed or malformed'

# Exercise compiler-message shape, with the receipt hashes recomputed so the
# owning Cargo protocol (rather than an incidental digest mismatch) rejects it.
$firstLog = Join-Path $logs $builds[0].stdout.path
$originalLog = [IO.File]::ReadAllText($firstLog)
foreach ($variant in @('missing-finished', 'failed-finished', 'wrong-source', 'test-harness', 'duplicate-target')) {
    $lines = $originalLog.Trim().Split([char]10)
    $message = $lines[0] | ConvertFrom-Json
    $finished = $lines[1]
    switch ($variant) {
        'missing-finished' { $finished = '' }
        'failed-finished' { $finished = '{"reason":"build-finished","success":false}' }
        'wrong-source' { $message.target.src_path = $cargoPath }
        'test-harness' { $message.profile.test = $true }
    }
    $text = ($message | ConvertTo-Json -Depth 5 -Compress) + "`n" + $finished
    if ($variant -eq 'duplicate-target') { $text += "`n" + $lines[0] }
    [IO.File]::WriteAllText($firstLog, $text)
    $expected = if ($variant -in @('wrong-source', 'test-harness')) { 'owning validation target' } else { 'single successful Cargo artifact' }
    Assert-ToolCase "compiler protocol $variant" { param($r)
        $r.builds[0].stdout.sha256 = (Get-CaptureToolFileReference $firstLog).sha256
        $r.builds[0].stdout.bytes = (Get-Item -LiteralPath $firstLog).Length
    } $expected
}
[IO.File]::WriteAllText($firstLog, $originalLog)
[IO.File]::WriteAllText($receiptPath, $good)
$authority = Open-CaptureValidationTools -RepoRoot $repo -ReceiptPath $receiptPath -ExpectedSourceInputSha256 $source.sha256
try {
    foreach ($path in @($plans.Executable) + @($receiptPath, $firstLog)) {
        foreach ($operation in @('write', 'delete')) {
            $refused = $false
            try {
                if ($operation -eq 'write') { [IO.File]::WriteAllText($path, 'mutation') }
                else { [IO.File]::Delete($path) }
            } catch [IO.IOException] { $refused = $true }
            if (-not $refused) { throw "read lease allowed $operation of $path" }
        }
    }
} finally { Close-CaptureValidationTools $authority }
$passed.Add('tool binaries and evidence stay immutable under read leases')
# Failure must close any already-acquired leases so the next build is possible.
Assert-ToolCase 'late roster failure releases all leases' { param($r) $r.builds[-1].artifact.sha256 = 'b' * 64 } 'Changed or malformed'
foreach ($path in @($plans.Executable) + @($receiptPath, $firstLog)) {
    $file = [IO.File]::Open($path, [IO.FileMode]::Open, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
    $file.Dispose()
}
$passed.Add('success and failure release file custody')

[IO.File]::WriteAllText($receiptPath, $good)
$retained = Join-Path $scratch 'retained'
$null = New-Item -ItemType Directory -Path $retained
foreach ($file in @($receiptPath) + @($builds | ForEach-Object { Join-Path $logs $_.stdout.path; Join-Path $logs $_.stderr.path })) {
    Copy-Item -LiteralPath $file -Destination $retained
}
$authority = Open-CaptureValidationTools -RepoRoot $repo -ReceiptPath (Join-Path $retained 'capture-validation-tools.json') -ExpectedSourceInputSha256 $source.sha256
try {
    if ($authority.Receipt.sha256 -cne (Get-CaptureToolFileReference $receiptPath).sha256) { throw 'Retention changed receipt bytes.' }
} finally { Close-CaptureValidationTools $authority }
$passed.Add('retained receipt and relative logs verify unchanged')

# Execute the builder's exact directory scope with a harmless child. Deliberately
# start with PowerShell and OS directories different; no Cargo is invoked.
. (Join-Path $PSScriptRoot 'ProductionCapture.Validation.ps1')
$builderPath = Join-Path $PSScriptRoot 'Build-CaptureValidationTools.ps1'
$tokens = $null; $errors = $null
$builderAst = [Management.Automation.Language.Parser]::ParseFile($builderPath, [ref]$tokens, [ref]$errors)
$scopeFunction = @($builderAst.FindAll({ param($node)
    $node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -ceq 'Invoke-CaptureToolsInRepository'
}, $true))
if ($scopeFunction.Count -ne 1) { throw 'Builder must own one process-directory scope.' }
. ([scriptblock]::Create($scopeFunction[0].Extent.Text))
$originalProcessDirectory = [Environment]::CurrentDirectory
$originalLocation = (Get-Location).Path
try {
    [Environment]::CurrentDirectory = $scratch
    Invoke-CaptureToolsInRepository -RepoPath $repo -Action {
        if ((Get-Location).Path -ine $repo) { throw 'PowerShell location did not enter the owning repository.' }
        $child = Invoke-IndependentCaptureGate -Name 'directory-probe' `
            -FilePath (Get-Command powershell.exe -CommandType Application).Source `
            -Arguments @('-NoProfile', '-Command', '[Environment]::CurrentDirectory') -LogDir $scratch -TimeoutSeconds 10
        if (-not $child.passed -or ([IO.File]::ReadAllText($child.stdout)).Trim() -ine $repo) {
            throw 'Child process did not inherit the claimed repository directory.'
        }
    }
    if ([Environment]::CurrentDirectory -ine $scratch -or (Get-Location).Path -ine $originalLocation) {
        throw 'Builder directory scope did not restore caller state.'
    }
    $failed = $false
    try { Invoke-CaptureToolsInRepository -RepoPath $repo -Action { throw 'fixture-directory-failure' } }
    catch { if ($_.Exception.Message -cne 'fixture-directory-failure') { throw }; $failed = $true }
    if (-not $failed -or [Environment]::CurrentDirectory -ine $scratch -or (Get-Location).Path -ine $originalLocation) {
        throw 'Failed builder directory scope did not restore caller state.'
    }
} finally { [Environment]::CurrentDirectory = $originalProcessDirectory }
$passed.Add('harmless child inherits owning repository and caller directories restore on success or failure')
foreach ($plan in $plans) {
    if (($plan.Arguments[-2..-1] -join ',') -cne '--jobs,2') { throw 'Diagnostic build worker bound is absent.' }
}
$passed.Add('all diagnostic builds explicitly bound Cargo jobs to two')

# Check the owning driver consumes the separate authority before any speed
# transcript nonce can be consumed. No product process is launched by units.
$driver = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'Start-Hoi4ProductionCaptureSafe.ps1') -Raw
foreach ($role in @('fold_check', 'compact_replay_gate', 'container_verify', 'domain_roster')) {
    if ($driver.Contains('$prebuiltArtifacts.artifacts.' + $role) -or -not $driver.Contains('$validationTools.Tools.' + $role)) {
        throw 'Driver mixes shipping and diagnostic artifact roles.'
    }
}
$toolAdmission = $driver.IndexOf('Open-CaptureValidationTools -RepoRoot')
$nonceAdmission = $driver.IndexOf('$speedEvidenceAdmission=Invoke-IndependentCaptureGate')
if ($toolAdmission -lt 0 -or $nonceAdmission -lt 0 -or $toolAdmission -gt $nonceAdmission) { throw 'Tools admitted after speed nonce consumption.' }
$passed.Add('driver uses separate tool authority before speed gate')

# Execute the driver's actual admission, roster binding and invocation statements
# in fixture scope. Extracting these AST nodes does not execute the capture driver.
$tokens = $null; $errors = $null
$driverAst = [Management.Automation.Language.Parser]::ParseInput($driver, [ref]$tokens, [ref]$errors)
if ($errors.Count) { throw 'Driver syntax failed before boundary extraction.' }
function Get-DriverAssignment {
    param([string]$Variable)
    $nodes = @($driverAst.FindAll({ param($node)
        $node -is [Management.Automation.Language.AssignmentStatementAst] -and
        $node.Left -is [Management.Automation.Language.VariableExpressionAst] -and
        $node.Left.VariablePath.UserPath -ceq $Variable
    }, $true))
    if ($nodes.Count -ne 1) { throw "Expected one driver assignment for $Variable; implicit fallback is forbidden." }
    return $nodes[0]
}
$admissionNode = Get-DriverAssignment 'validationTools'
$rosterNode = Get-DriverAssignment 'domainRosterTool'
$invokeNode = Get-DriverAssignment 'domainRosterOutput'
if ($admissionNode.Extent.StartOffset -ge $rosterNode.Extent.StartOffset -or
    $rosterNode.Extent.StartOffset -ge $invokeNode.Extent.StartOffset) { throw 'Roster consumer precedes its admission/binding.' }
if ($rosterNode.Right.Extent.Text -cne '[string]$validationTools.Tools.domain_roster') { throw 'Roster path does not come solely from admitted tools.' }
$rosterPlan = @($plans | Where-Object { $_.Role -ceq 'domain_roster' })[0]
if ($plans.Count -ne 4 -or $rosterPlan.Package -cne 'recorder_host' -or $rosterPlan.Target -cne 'domain_roster' -or
    $rosterPlan.Source -ine (Join-Path $repo 'extractor/native/recorder_host/src/bin/domain_roster.rs')) {
    throw 'Roster build is not the existing recorder_host diagnostic target.'
}
$rosterBytes = [IO.File]::ReadAllBytes($rosterPlan.Executable)
foreach ($variant in @('admitted', 'missing-role', 'foreign-role', 'foreign-output', 'unavailable', 'tampered')) {
    $receipt = $good | ConvertFrom-Json
    switch ($variant) {
        'missing-role' { $receipt.builds = @($receipt.builds | Where-Object { $_.role -cne 'domain_roster' }) }
        'foreign-role' { $receipt.builds[-1].role = 'recorder_host' }
        'foreign-output' { $receipt.builds[-1].artifact.path = $cargoPath }
        'unavailable' { [IO.File]::Delete($rosterPlan.Executable) }
        'tampered' { [IO.File]::WriteAllBytes($rosterPlan.Executable, [byte[]]@(4, 5, 6)) }
    }
    [IO.File]::WriteAllText($receiptPath, ($receipt | ConvertTo-Json -Depth 12))
    $repoRoot = $repo; $nativeRoot = $repo; $ValidationToolsReceiptPath = $receiptPath
    $prebuiltArtifacts = [pscustomobject]@{ source_input_sha256 = $source.sha256 }
    $marker = Join-Path $nativeRoot 'roster-invoked.txt'
    if (Test-Path -LiteralPath $marker) { [IO.File]::Delete($marker) }
    $validationTools = $null; $failure = ''
    try {
        . ([scriptblock]::Create($admissionNode.Extent.Text))
        . ([scriptblock]::Create($rosterNode.Extent.Text))
        if ($validationTools.Leases.Count -lt $plans.Count) { throw 'No active admitted tool leases.' }
        . ([scriptblock]::Create($invokeNode.Extent.Text))
        if ($LASTEXITCODE -ne 0 -or ($domainRosterOutput -join '') -cne '[]') { throw 'Driver did not execute the admitted roster fixture.' }
    } catch { $failure = $_.Exception.Message }
    finally { Close-CaptureValidationTools $validationTools }
    if ($variant -ceq 'admitted') {
        if ($failure -or -not (Test-Path -LiteralPath $marker)) { throw "Admitted roster did not reach consumer: $failure" }
    } elseif (-not $failure -or (Test-Path -LiteralPath $marker)) { throw "Rejected roster reached consumer: $variant" }
    [IO.File]::WriteAllBytes($rosterPlan.Executable, $rosterBytes)
    $passed.Add("driver roster boundary $variant")
}

# Run the actual hard-pin check against checkout bytes and mutated scratch copies.
# Do not normalize at verification time: even newline-only tampering must fail.
$scriptDir = $PSScriptRoot
foreach ($variable in @('speedEvidenceVerifier', 'speedEvidenceProbe', 'speedEvidenceSchema',
    'expectedSpeedEvidenceVerifierSha', 'expectedSpeedEvidenceProbeSha', 'expectedSpeedEvidenceSchemaSha')) {
    . ([scriptblock]::Create((Get-DriverAssignment $variable).Extent.Text))
}
$pinChecks = @($driverAst.FindAll({ param($node)
    $node -is [Management.Automation.Language.ForEachStatementAst] -and
    $node.Variable.VariablePath.UserPath -ceq 'toolPin'
}, $true))
if ($pinChecks.Count -ne 1) { throw 'Expected the actual driver hard-pin check.' }
$pinCheck = [scriptblock]::Create($pinChecks[0].Extent.Text)
. $pinCheck
$passed.Add('actual driver speed pins match exact checkout bytes including unchanged probe')
foreach ($variable in @('speedEvidenceVerifier', 'speedEvidenceSchema')) {
    $originalPath = Get-Variable -Name $variable -ValueOnly
    $originalBytes = [IO.File]::ReadAllBytes($originalPath)
    $copy = Join-Path $scratch ([IO.Path]::GetFileName($originalPath))
    foreach ($variant in @('content', 'crlf', 'missing')) {
        [IO.File]::WriteAllBytes($copy, $originalBytes)
        if ($variant -ceq 'content') { [IO.File]::AppendAllText($copy, ' ') }
        if ($variant -ceq 'crlf') {
            $lf = [Text.Encoding]::UTF8.GetString($originalBytes).Replace("`r`n", "`n")
            [IO.File]::WriteAllText($copy, $lf.Replace("`n", "`r`n"), [Text.UTF8Encoding]::new($false))
        }
        if ($variant -ceq 'missing') { [IO.File]::Delete($copy) }
        Set-Variable -Name $variable -Value $copy
        $failure = ''
        try { . $pinCheck } catch { $failure = $_.Exception.Message }
        finally { Set-Variable -Name $variable -Value $originalPath }
        if (-not $failure.Contains('canonical speed evidence tooling hash mismatch')) { throw "Speed pin $variant did not fail at its owning check: $failure" }
        $passed.Add("actual speed pin rejects $variable $variant")
    }
}
foreach ($name in @('Start-Hoi4ProductionCaptureSafe.ps1', 'CaptureValidation.Tools.ps1', 'Build-CaptureValidationTools.ps1', 'Test-CaptureValidationTools.ps1')) {
    $tokens = $null; $errors = $null
    $null = [Management.Automation.Language.Parser]::ParseFile((Join-Path $PSScriptRoot $name), [ref]$tokens, [ref]$errors)
    if ($errors.Count) { throw "Script parse failure: $name" }
}
$passed.Add('all four scripts parse')
[ordered]@{ schema = 'hoi4.capture-validation-tools-tests.v1'; status = 'PASS'; cases = $passed.Count
    scratch = $scratch; fixture_authority = 'synthetic protocol tests only; no Cargo build or capture'; passed = @($passed.ToArray()) } | ConvertTo-Json -Depth 5
