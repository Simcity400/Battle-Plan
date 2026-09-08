# Diagnostic verifier custody is separate from the three shipping artifacts.
Set-StrictMode -Version Latest

function Get-CaptureValidationToolPlan {
    param([Parameter(Mandatory = $true)][string]$RepoRoot)
    $repo = [IO.Path]::GetFullPath($RepoRoot)
    foreach ($spec in @(
        @('fold_check', 'fold_check', 'fold_check', 'extractor/native/fold_check/src/main.rs'),
        @('compact_replay_gate', 'compact_replay_verification_gates', 'compact_replay_gate', 'extractor/native/lanes/compact_replay_verification_gates/src/main.rs'),
        @('container_verify', 'snapshot', 'pack_replay', 'portable_replay/native/snapshot/src/bin/pack_replay.rs'),
        @('domain_roster', 'recorder_host', 'domain_roster', 'extractor/native/recorder_host/src/bin/domain_roster.rs')
    )) {
        [pscustomobject]@{
            Role = $spec[0]; Package = $spec[1]; Target = $spec[2]
            Source = [IO.Path]::GetFullPath((Join-Path $repo $spec[3]))
            Executable = [IO.Path]::GetFullPath((Join-Path $repo ('.cargo-target/release/' + $spec[2] + '.exe')))
            Arguments = @('build', '--locked', '--offline', '--release', '--manifest-path',
                (Join-Path $repo 'extractor/native/Cargo.toml'), '-p', $spec[1], '--bin', $spec[2], '--message-format=json', '--jobs', '2')
        }
    }
}

function Get-CaptureToolFileReference {
    param([Parameter(Mandatory = $true)][string]$Path)
    $file = Get-Item -LiteralPath $Path
    if ($file.PSIsContainer) { throw "Expected a tool evidence file: $Path" }
    return [ordered]@{ path = $file.FullName; bytes = $file.Length
        sha256 = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant() }
}

function Assert-CaptureToolFileReference {
    param($Reference, [string]$Path)
    $actual = Get-CaptureToolFileReference -Path $Path
    if (($Reference.bytes -isnot [int] -and $Reference.bytes -isnot [long]) -or
        $Reference.bytes -ne $actual.bytes -or $Reference.sha256 -cne $actual.sha256) {
        throw "Changed or malformed capture validation tool evidence: $Path"
    }
}

function Assert-CaptureToolCompilerOutput {
    param($Plan, [string]$StdoutPath)
    $targets = @(); $finished = @()
    foreach ($line in [IO.File]::ReadLines($StdoutPath)) {
        if ([string]::IsNullOrWhiteSpace($line)) { continue }
        $message = $line | ConvertFrom-Json
        if ($message.reason -ceq 'build-finished') { $finished += $message }
        if ($message.reason -ceq 'compiler-artifact' -and $message.target.name -ceq $Plan.Target) {
            $targets += $message
        }
    }
    if ($finished.Count -ne 1 -or $finished[0].success -isnot [bool] -or -not $finished[0].success -or
        $targets.Count -ne 1) { throw "No single successful Cargo artifact for $($Plan.Role)" }
    $target = $targets[0]
    if (($target.target.kind -join ',') -cne 'bin' -or $target.profile.test -isnot [bool] -or
        $target.profile.test -or [IO.Path]::GetFullPath([string]$target.executable) -ine $Plan.Executable -or
        [IO.Path]::GetFullPath([string]$target.target.src_path) -ine $Plan.Source) {
        throw "Cargo artifact is not the owning validation target: $($Plan.Role)"
    }
}

function Close-CaptureValidationTools {
    param($Authority)
    if ($null -ne $Authority) { foreach ($lease in $Authority.Leases) { $lease.Dispose() } }
}

function Open-CaptureValidationTools {
    param(
        [Parameter(Mandatory = $true)][string]$RepoRoot,
        [Parameter(Mandatory = $true)][string]$ReceiptPath,
        # Obtained from Test-ProductionArtifactAttestation, never from the tools receipt.
        [Parameter(Mandatory = $true)][ValidatePattern('^[0-9a-f]{64}$')][string]$ExpectedSourceInputSha256
    )
    $leases = [Collections.Generic.List[object]]::new()
    try {
        $leases.Add([IO.File]::Open($ReceiptPath, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read))
        $receiptFile = Get-CaptureToolFileReference $ReceiptPath
        $receipt = Get-Content -LiteralPath $ReceiptPath -Raw | ConvertFrom-Json
        if ($receipt.schema -cne 'hoi4.capture-validation-tools.v1' -or
            $receipt.authority -cne 'diagnostic_tools_not_shipping_artifacts' -or
            $receipt.source_before.schema -cne 'battle-plan.production_rust_input_digest.v1' -or
            $receipt.source_before.sha256 -cne $ExpectedSourceInputSha256 -or
            ($receipt.source_before | ConvertTo-Json -Compress) -cne ($receipt.source_after | ConvertTo-Json -Compress) -or
            ($receipt.source_before.file_count -isnot [int] -and $receipt.source_before.file_count -isnot [long]) -or
            $receipt.source_before.file_count -lt 1) { throw 'Validation tools do not bind the current unchanged production source.' }
        Test-ProductionBuildEnvironmentRecord -Record $receipt.build_environment
        $plans = @(Get-CaptureValidationToolPlan $RepoRoot)
        if (@($receipt.builds).Count -ne $plans.Count) { throw "Validation tools require exactly $($plans.Count) owning builds." }
        $evidence = [Collections.Generic.List[string]]::new()
        $evidence.Add($receiptFile.path)
        $tools = [ordered]@{}
        foreach ($plan in $plans) {
            $builds = @($receipt.builds | Where-Object { $_.role -ceq $plan.Role })
            if ($builds.Count -ne 1) { throw "Missing or duplicate validation role: $($plan.Role)" }
            $build = $builds[0]
            if (($build.exit_code -isnot [int] -and $build.exit_code -isnot [long]) -or $build.exit_code -ne 0 -or
                $build.timed_out -isnot [bool] -or $build.timed_out -or $null -ne $build.problem -or
                $build.started -isnot [bool] -or -not $build.started -or
                ($build.arguments -join [char]0) -cne ($plan.Arguments -join [char]0) -or
                [IO.Path]::GetFullPath([string]$build.working_directory) -ine [IO.Path]::GetFullPath($RepoRoot)) {
                throw "Validation build failed or used different inputs: $($plan.Role)"
            }
            if ([IO.Path]::GetFileName([string]$build.cargo.path) -ine 'cargo.exe') { throw 'Validation tools were not built by the recorded Cargo executable.' }
            Assert-CaptureToolFileReference $build.cargo $build.cargo.path
            foreach ($stream in @('stdout', 'stderr')) {
                $ref = $build.$stream
                $expectedName = $plan.Role + '.' + $stream + '.log'
                if ($ref.path -cne $expectedName) { throw 'Validation log must use its canonical relative filename.' }
                $path = Join-Path (Split-Path -Parent $receiptFile.path) $expectedName
                $leases.Add([IO.File]::Open($path, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read))
                Assert-CaptureToolFileReference $ref $path
                $evidence.Add($path)
            }
            Assert-CaptureToolCompilerOutput $plan (Join-Path (Split-Path -Parent $receiptFile.path) $build.stdout.path)
            if ([IO.Path]::GetFullPath([string]$build.artifact.path) -ine $plan.Executable) {
                throw "Validation executable is not the owning worktree output: $($plan.Role)"
            }
            # Hold read leases through capture and validation. A build cannot
            # replace the admitted executable between hashing and execution.
            $lease = [IO.File]::Open($plan.Executable, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
            $leases.Add($lease)
            Assert-CaptureToolFileReference $build.artifact $plan.Executable
            $tools[$plan.Role] = $plan.Executable
        }
        return [pscustomobject]@{ Tools = $tools; Receipt = $receiptFile
            EvidenceFiles = @($evidence.ToArray()); Leases = @($leases.ToArray()) }
    } catch {
        foreach ($lease in $leases) { $lease.Dispose() }
        throw
    }
}
