Set-StrictMode -Version Latest

$script:ProductionArtifactReceiptSchema = "battle-plan.central_rust_prebuilt_artifacts.v1"
$script:ProductionArtifactRoles = [ordered]@{
    collector_dll = "collector.dll"
    recorder_host = "collector_record.exe"
    fold_check = "fold_check.exe"
    compact_replay_gate = "compact_replay_gate.exe"
    container_verify = "pack_replay.exe"
}
$script:ProductionArtifactPackages = @(
    "collector",
    "recorder_host",
    "fold_check",
    "compact_replay_verification_gates",
    "snapshot"
)

function Get-ProductionInputFiles {
    param([Parameter(Mandatory = $true)][string]$Root)

    $pending = [System.Collections.Generic.Stack[string]]::new()
    $pending.Push([System.IO.Path]::GetFullPath($Root))
    while ($pending.Count -gt 0) {
        $directory = $pending.Pop()
        foreach ($entry in [System.IO.Directory]::EnumerateFileSystemEntries($directory)) {
            $attributes = [System.IO.File]::GetAttributes($entry)
            if (($attributes -band [System.IO.FileAttributes]::Directory) -ne 0) {
                $name = [System.IO.Path]::GetFileName($entry)
                if ($name -eq "target" -or $name -eq ".cargo-target" -or $name.StartsWith(".tmp", [StringComparison]::OrdinalIgnoreCase)) { continue }
                if (($attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) { continue }
                $pending.Push($entry)
            }
            else {
                $entry
            }
        }
    }
}

function Get-ProductionRustInputDigest {
    param([Parameter(Mandatory = $true)][string]$RepoRoot)

    $repo = [System.IO.Path]::GetFullPath($RepoRoot).TrimEnd('\')
    $roots = @(
        (Join-Path $repo "extractor\native"),
        (Join-Path $repo "portable_replay\native")
    )
    $extensions = @(
        ".rs", ".toml", ".lock", ".c", ".cc", ".cpp", ".h", ".hpp",
        ".rc", ".def", ".bin", ".json", ".py", ".ps1"
    )
    $records = [System.Collections.Generic.List[string]]::new()
    foreach ($root in $roots) {
        if (-not (Test-Path -LiteralPath $root -PathType Container)) {
            throw "production Rust input root is missing: $root"
        }
        foreach ($file in @(Get-ProductionInputFiles -Root $root | Sort-Object)) {
            $full = [System.IO.Path]::GetFullPath($file)
            $extension = [System.IO.Path]::GetExtension($full).ToLowerInvariant()
            if ($extensions -notcontains $extension) { continue }
            $relative = $full.Substring($repo.Length).TrimStart('\').Replace('\', '/')
            $digest = (Get-FileHash -LiteralPath $full -Algorithm SHA256).Hash.ToLowerInvariant()
            $length = [System.IO.FileInfo]::new($full).Length
            $records.Add("$relative`t$length`t$digest")
        }
    }
    foreach ($extra in @(
        (Join-Path $repo ".cargo\config.toml"),
        (Join-Path $repo ".cargo\config"),
        (Join-Path $repo ".config\nextest.toml"),
        (Join-Path $repo "rust-toolchain.toml"),
        (Join-Path $repo "rust-toolchain")
    )) {
        if (-not (Test-Path -LiteralPath $extra -PathType Leaf)) { continue }
        $full = [System.IO.Path]::GetFullPath($extra)
        $relative = $full.Substring($repo.Length).TrimStart('\').Replace('\', '/')
        $digest = (Get-FileHash -LiteralPath $full -Algorithm SHA256).Hash.ToLowerInvariant()
        $length = [System.IO.FileInfo]::new($full).Length
        $records.Add("$relative`t$length`t$digest")
    }
    $records.Sort([System.StringComparer]::Ordinal)
    $payload = [System.Text.Encoding]::UTF8.GetBytes(($records -join "`n") + "`n")
    $hasher = [System.Security.Cryptography.SHA256]::Create()
    try { $digest = ([BitConverter]::ToString($hasher.ComputeHash($payload))).Replace('-', '').ToLowerInvariant() }
    finally { $hasher.Dispose() }
    return [pscustomobject]@{
        schema = "battle-plan.production_rust_input_digest.v1"
        sha256 = $digest
        file_count = $records.Count
        selection = "extractor/native and portable_replay/native Rust/C/Cargo/data/Python/PowerShell inputs plus repository Cargo/toolchain/nextest configuration; Cargo output roots excluded"
    }
}

function Write-ProductionArtifactAttestation {
    param(
        [Parameter(Mandatory = $true)][string]$RepoRoot,
        [Parameter(Mandatory = $true)][string]$ReceiptPath,
        [Parameter(Mandatory = $true)][string]$RequestedWorkspace,
        [Parameter(Mandatory = $true)]$SourceDigest
    )

    if ($RequestedWorkspace -notin @("All", "Extractor")) {
        throw "production artifact attestation requires centralized Release for Extractor or All"
    }
    $repo = [System.IO.Path]::GetFullPath($RepoRoot)
    $releaseRoot = Join-Path $repo ".cargo-target\release"
    $driver = Join-Path $repo "extractor\native\scripts\Start-Hoi4ProductionCaptureSafe.ps1"
    $runner = Join-Path $repo "extractor\native\scripts\Invoke-CentralRustValidation.ps1"
    $artifacts = @()
    foreach ($entry in $script:ProductionArtifactRoles.GetEnumerator()) {
        $path = Join-Path $releaseRoot $entry.Value
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
            throw "centralized Release produced no required production artifact: $path"
        }
        $item = Get-Item -LiteralPath $path
        if ($item.Length -le 0) { throw "centralized Release produced an empty artifact: $path" }
        $artifacts += [ordered]@{
            role = [string]$entry.Key
            package = [string]$script:ProductionArtifactPackages[$artifacts.Count]
            path = [System.IO.Path]::GetFullPath($path)
            bytes = [int64]$item.Length
            sha256 = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
        }
    }
    $validatedWorkspaces = if ($RequestedWorkspace -eq "All") { @("Extractor", "Replay") } else { @("Extractor") }
    $receipt = [ordered]@{
        schema = $script:ProductionArtifactReceiptSchema
        status = "PASS_CENTRAL_RELEASE_PREBUILT_ARTIFACTS"
        successful_exit = $true
        central_validation = [ordered]@{
            mode = "Release"
            requested_workspace = $RequestedWorkspace
            validated_workspaces = $validatedWorkspaces
            workspace_package_selection = @("--workspace")
            production_artifact_packages = @($script:ProductionArtifactPackages)
            runner_path = [System.IO.Path]::GetFullPath($runner)
            runner_sha256 = (Get-FileHash -LiteralPath $runner -Algorithm SHA256).Hash.ToLowerInvariant()
        }
        source_input = $SourceDigest
        capture_driver = [ordered]@{
            path = [System.IO.Path]::GetFullPath($driver)
            sha256 = (Get-FileHash -LiteralPath $driver -Algorithm SHA256).Hash.ToLowerInvariant()
        }
        artifacts = $artifacts
    }
    $parent = Split-Path -Parent $ReceiptPath
    New-Item -ItemType Directory -Path $parent -Force | Out-Null
    $temp = "$ReceiptPath.$([guid]::NewGuid().ToString('N')).tmp"
    [System.IO.File]::WriteAllText(
        $temp,
        ($receipt | ConvertTo-Json -Depth 12),
        [System.Text.UTF8Encoding]::new($false)
    )
    $stream = [System.IO.File]::Open($temp, [System.IO.FileMode]::Open, [System.IO.FileAccess]::ReadWrite, [System.IO.FileShare]::None)
    try { $stream.Flush($true) } finally { $stream.Dispose() }
    Move-Item -LiteralPath $temp -Destination $ReceiptPath -Force
}

function Test-ProductionArtifactAttestation {
    param(
        [Parameter(Mandatory = $true)][string]$RepoRoot,
        [Parameter(Mandatory = $true)][string]$ReceiptPath,
        [Parameter(Mandatory = $true)][string]$ExpectedDriverPath
    )

    if (-not (Test-Path -LiteralPath $ReceiptPath -PathType Leaf)) {
        throw "centralized prebuilt artifact attestation is missing: $ReceiptPath"
    }
    $repo = [System.IO.Path]::GetFullPath($RepoRoot)
    $receipt = Get-Content -LiteralPath $ReceiptPath -Raw | ConvertFrom-Json
    if ([string]$receipt.schema -cne $script:ProductionArtifactReceiptSchema -or
        [string]$receipt.status -cne "PASS_CENTRAL_RELEASE_PREBUILT_ARTIFACTS" -or
        $receipt.successful_exit -isnot [bool] -or -not $receipt.successful_exit) {
        throw "centralized prebuilt artifact attestation has no successful Release authority"
    }
    if ([string]$receipt.central_validation.mode -cne "Release" -or
        [string]$receipt.central_validation.requested_workspace -notin @("All", "Extractor") -or
        (@($receipt.central_validation.validated_workspaces) -notcontains "Extractor") -or
        (@($receipt.central_validation.workspace_package_selection) -join ',') -cne "--workspace" -or
        (@($receipt.central_validation.production_artifact_packages) -join ',') -cne ($script:ProductionArtifactPackages -join ',')) {
        throw "prebuilt artifact attestation was not emitted by a full centralized extractor Release"
    }
    $runner = Join-Path $repo "extractor\native\scripts\Invoke-CentralRustValidation.ps1"
    if ([System.IO.Path]::GetFullPath([string]$receipt.central_validation.runner_path) -ine [System.IO.Path]::GetFullPath($runner) -or
        [string]$receipt.central_validation.runner_sha256 -cne (Get-FileHash -LiteralPath $runner -Algorithm SHA256).Hash.ToLowerInvariant()) {
        throw "centralized runner differs from the prebuilt artifact attestation"
    }
    if ([System.IO.Path]::GetFullPath([string]$receipt.capture_driver.path) -ine [System.IO.Path]::GetFullPath($ExpectedDriverPath) -or
        [string]$receipt.capture_driver.sha256 -cne (Get-FileHash -LiteralPath $ExpectedDriverPath -Algorithm SHA256).Hash.ToLowerInvariant()) {
        throw "production capture driver differs from the prebuilt artifact attestation"
    }
    $currentSource = Get-ProductionRustInputDigest -RepoRoot $repo
    if ([string]$receipt.source_input.schema -cne [string]$currentSource.schema -or
        [int]$receipt.source_input.file_count -ne [int]$currentSource.file_count -or
        [string]$receipt.source_input.sha256 -cne [string]$currentSource.sha256) {
        throw "current source/compiler inputs differ from the centralized prebuilt artifact attestation"
    }
    $expectedReleaseRoot = Join-Path $repo ".cargo-target\release"
    $rows = @($receipt.artifacts)
    if ($rows.Count -ne $script:ProductionArtifactRoles.Count) {
        throw "prebuilt artifact attestation has the wrong artifact count"
    }
    $resolved = [ordered]@{}
    $resolvedHashes = [ordered]@{}
    $index = 0
    foreach ($entry in $script:ProductionArtifactRoles.GetEnumerator()) {
        $matches = @($rows | Where-Object { [string]$_.role -ceq [string]$entry.Key })
        if ($matches.Count -ne 1) { throw "prebuilt artifact role is missing or duplicated: $($entry.Key)" }
        $row = $matches[0]
        $expectedPath = [System.IO.Path]::GetFullPath((Join-Path $expectedReleaseRoot $entry.Value))
        if ([string]$row.package -cne [string]$script:ProductionArtifactPackages[$index] -or
            [System.IO.Path]::GetFullPath([string]$row.path) -ine $expectedPath -or
            -not (Test-Path -LiteralPath $expectedPath -PathType Leaf)) {
            throw "prebuilt artifact identity differs for role $($entry.Key)"
        }
        $item = Get-Item -LiteralPath $expectedPath
        if ([int64]$row.bytes -ne [int64]$item.Length -or [int64]$item.Length -le 0 -or
            [string]$row.sha256 -cne (Get-FileHash -LiteralPath $expectedPath -Algorithm SHA256).Hash.ToLowerInvariant()) {
            throw "prebuilt artifact bytes differ for role $($entry.Key)"
        }
        $resolved[[string]$entry.Key] = $expectedPath
        $resolvedHashes[[string]$entry.Key] = [string]$row.sha256
        $index++
    }
    return [pscustomobject]@{
        receipt_path = [System.IO.Path]::GetFullPath($ReceiptPath)
        receipt_sha256 = (Get-FileHash -LiteralPath $ReceiptPath -Algorithm SHA256).Hash.ToLowerInvariant()
        source_input_sha256 = [string]$currentSource.sha256
        artifacts = $resolved
        artifact_sha256 = $resolvedHashes
    }
}
