[CmdletBinding()]
param([string]$HarnessPath = '')

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if (-not $HarnessPath) {
    $HarnessPath = Join-Path (Split-Path -Parent $MyInvocation.MyCommand.Path) 'Invoke-Validation.ps1'
}
$tokens = $null
$parseErrors = $null
$ast = [Management.Automation.Language.Parser]::ParseFile($HarnessPath, [ref]$tokens, [ref]$parseErrors)
if ($parseErrors.Count) { throw ($parseErrors.Message -join '; ') }
foreach ($functionName in @('Start-ValidationRoot', 'New-ValidationWorkerSnapshot')) {
    $definitions = @($ast.FindAll({
        param($node)
        $node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq $functionName
    }, $true))
    if ($definitions.Count -ne 1) { throw "Expected one production $functionName function" }
    . ([scriptblock]::Create($definitions[0].Extent.Text))
}

function Assert-True {
    param([bool]$Condition, [string]$Message)
    if (-not $Condition) { throw $Message }
}

function Wait-Condition {
    param([scriptblock]$Condition)
    $deadline = [DateTime]::UtcNow.AddSeconds(20)
    while (-not (& $Condition)) {
        if ([DateTime]::UtcNow -gt $deadline) { throw 'Fake child exceeded its 20-second observation bound' }
        Start-Sleep -Milliseconds 25
    }
}

function Read-LiveText {
    param([string]$Path)
    $stream = [IO.FileStream]::new($Path, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::ReadWrite)
    $reader = [IO.StreamReader]::new($stream)
    try { return $reader.ReadToEnd() } finally { $reader.Dispose() }
}

function Get-LiveHash {
    param([string]$Path)
    $stream = [IO.FileStream]::new($Path, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::ReadWrite)
    $sha = [Security.Cryptography.SHA256]::Create()
    try { return [BitConverter]::ToString($sha.ComputeHash($stream)) } finally { $sha.Dispose(); $stream.Dispose() }
}

function Stop-FakeChild {
    param([string]$StartedPath)
    if (-not (Test-Path -LiteralPath $StartedPath)) { return }
    $started = Get-Content -LiteralPath $StartedPath -Raw | ConvertFrom-Json
    $child = Get-Process -Id $started.ProcessId -ErrorAction SilentlyContinue
    if ($child -and $child.StartTime.ToUniversalTime().ToString('o') -ceq $started.ProcessStartedAt) {
        Stop-Process -Id $started.ProcessId -ErrorAction SilentlyContinue
    }
}

$python = (Get-Command python -CommandType Application | Select-Object -First 1).Source
$directory = Join-Path ([IO.Path]::GetTempPath()) ('validation-root-test-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $directory | Out-Null
$fakeChild = Join-Path $directory 'fake child.py'
@'
import hashlib, json, os, sys, time
mode = sys.argv[1]
print(json.dumps({"mode": mode, "args": sys.argv[2:],
                  "worker": os.environ.get("BATTLE_PLAN_TEST_RECORDER_WORKER"),
                  "sha": os.environ.get("BATTLE_PLAN_TEST_RECORDER_WORKER_SHA256"),
                  "actual_sha": hashlib.sha256(open(os.environ["BATTLE_PLAN_TEST_RECORDER_WORKER"], "rb").read()).hexdigest()}), flush=True)
sys.stdout.write("STDOUT-" + mode)
sys.stdout.flush()
sys.stderr.write("STDERR-" + mode)
sys.stderr.flush()
if mode == "running":
    time.sleep(30)
    sys.exit(99)
sys.exit(17 if mode == "failed" else 0)
'@ | Set-Content -LiteralPath $fakeChild -Encoding UTF8
$probeArguments = @('', 'two words', 'embedded"quote', 'C:\trailing space\', 'slash\\"quote')
$sourceWorker = Join-Path $directory 'cargo-output.exe'
[IO.File]::WriteAllBytes($sourceWorker, [byte[]]@(1, 4, 9, 16, 25))
$workerSha = (Get-FileHash -LiteralPath $sourceWorker -Algorithm SHA256).Hash.ToLowerInvariant()
$workerSnapshot = $null
$roots = @()
$failure = $null
try {
    $workerSnapshot = New-ValidationWorkerSnapshot -SourcePath $sourceWorker `
        -OutputDirectory (Join-Path $directory 'snapshot') -ExpectedSha256 $workerSha
    $workerPath = $workerSnapshot.Path
    Remove-Item -LiteralPath $sourceWorker
    [IO.File]::WriteAllBytes($sourceWorker, [byte[]]@(2, 8, 18, 32, 50))
    Assert-True ((Get-FileHash -LiteralPath $sourceWorker -Algorithm SHA256).Hash.ToLowerInvariant() -cne $workerSha) 'Simulated relink must replace the source image'
    foreach ($operation in @('write', 'delete')) {
        $refused = $false
        try {
            if ($operation -eq 'write') { [IO.File]::WriteAllBytes($workerPath, [byte[]]@(99)) }
            else { [IO.File]::Delete($workerPath) }
        } catch [IO.IOException] { $refused = $true }
        Assert-True $refused "Paired snapshot allowed $operation while roots can read it"
    }
    $mismatchRefused = $false
    try {
        $unexpected = New-ValidationWorkerSnapshot -SourcePath $sourceWorker `
            -OutputDirectory (Join-Path $directory 'mismatched-snapshot') -ExpectedSha256 $workerSha
        $unexpected.Lease.Dispose()
    } catch {
        if ($_.Exception.Message -cne 'Recorder worker changed before snapshot') { throw }
        $mismatchRefused = $true
    }
    Assert-True $mismatchRefused 'Changed build image must not be paired under its old hash'
    Assert-True (-not (Test-Path -LiteralPath (Join-Path $directory 'mismatched-snapshot'))) 'Failed pairing published a snapshot'
    foreach ($mode in @('running', 'complete', 'failed')) {
        $roots += Start-ValidationRoot -Name $mode -Program $python `
            -Arguments (@($fakeChild, $mode) + $probeArguments) -WorkingDirectory $directory `
            -OutputDirectory (Join-Path $directory $mode) -WorkerPath $workerPath -WorkerSha256 $workerSha
    }
    Wait-Condition { (Test-Path -LiteralPath $roots[1].ReceiptPath) -and (Test-Path -LiteralPath $roots[2].ReceiptPath) }
    Wait-Condition {
        (Test-Path -LiteralPath $roots[0].StderrPath) -and
        (Read-LiveText $roots[0].StderrPath).Contains('STDERR-running') -and
        (Read-LiveText $roots[0].StdoutPath).Contains('STDOUT-running')
    }
    Assert-True (-not (Test-Path -LiteralPath $roots[0].ReceiptPath)) 'Running child must not have a terminal receipt'
    $complete = Get-Content -LiteralPath $roots[1].ReceiptPath -Raw | ConvertFrom-Json
    $failed = Get-Content -LiteralPath $roots[2].ReceiptPath -Raw | ConvertFrom-Json
    Assert-True ($complete.ExitCode -eq 0 -and $null -eq $complete.Error) 'Completed later root lost its successful exit'
    Assert-True ($failed.ExitCode -eq 17 -and $null -eq $failed.Error) 'Failed sibling lost its exact native exit'
    foreach ($root in $roots) {
        $stdout = (Read-LiveText $root.StdoutPath)
        $stderr = (Read-LiveText $root.StderrPath)
        Assert-True ($stdout.EndsWith('STDOUT-' + $root.Name)) 'Stdout bytes were buffered or decorated'
        Assert-True ($stderr -ceq ('STDERR-' + $root.Name)) 'Stderr bytes were buffered or merged into stdout'
        $observed = ($stdout -split "`n")[0] | ConvertFrom-Json
        Assert-True ($observed.args.Count -eq $probeArguments.Count) 'Native argument cardinality changed'
        for ($i = 0; $i -lt $probeArguments.Count; $i++) {
            Assert-True ($observed.args[$i] -ceq $probeArguments[$i]) "Native argument $i changed"
        }
        Assert-True ($observed.worker -ceq $workerPath -and $observed.sha -ceq $workerSha) 'Worker pairing changed'
        Assert-True ($observed.actual_sha -ceq $workerSha) 'Child did not read the immutable paired bytes after source replacement'
    }
    Write-Host 'PASS: actual child reads pinned snapshot after Cargo output replacement; writes, deletes and stale pairing are refused'
    Write-Host 'PASS: later completed and failed roots persist receipts while the first root remains active'
    Write-Host 'PASS: exact native arguments, worker pairing and separate partial stdout/stderr'
    $preservedPaths = @($roots | ForEach-Object { $_.StdoutPath; $_.StderrPath }) + @($roots[1].ReceiptPath, $roots[2].ReceiptPath)
    $before = @($preservedPaths | ForEach-Object { (Get-LiveHash $_) })
    Stop-Job $roots[0].Job
    Stop-FakeChild (Join-Path $directory 'running/started.json')
    Assert-True (-not (Test-Path -LiteralPath $roots[0].ReceiptPath)) 'Interrupted job invented an exit receipt'
    for ($i = 0; $i -lt $preservedPaths.Count; $i++) {
        Assert-True ((Get-LiveHash $preservedPaths[$i]) -eq $before[$i]) 'Interrupted sibling changed preserved output'
    }
    Write-Host 'PASS: root interruption preserves completed receipts and partial stream bytes'
    $missing = Start-ValidationRoot -Name 'missing' -Program (Join-Path $directory 'absent.exe') `
        -Arguments @('unused') -WorkingDirectory $directory -OutputDirectory (Join-Path $directory 'missing')
    $roots += $missing
    Wait-Condition { Test-Path -LiteralPath $missing.ReceiptPath }
    $missingReceipt = Get-Content -LiteralPath $missing.ReceiptPath -Raw | ConvertFrom-Json
    Assert-True ($null -eq $missingReceipt.ExitCode -and -not [string]::IsNullOrEmpty($missingReceipt.Error)) 'Launch failure claimed a native exit'
    Write-Host 'PASS: launch failure remains distinct from a native nonzero exit'
} catch {
    $failure = $_
} finally {
    foreach ($root in $roots) {
        Stop-Job $root.Job -ErrorAction SilentlyContinue
        $startedPath = Join-Path (Split-Path -Parent $root.ReceiptPath) 'started.json'
        if ((Test-Path -LiteralPath $startedPath) -and -not (Test-Path -LiteralPath $root.ReceiptPath)) {
            Stop-FakeChild $startedPath
        }
        Remove-Job $root.Job -Force -ErrorAction SilentlyContinue
    }
    if ($workerSnapshot) { $workerSnapshot.Lease.Dispose() }
    Write-Host "VALIDATION_TEST_ARTIFACTS $directory"
}
if ($failure) { throw $failure }
