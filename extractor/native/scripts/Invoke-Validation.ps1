[CmdletBinding()]
param(
    [ValidateSet('Affected', 'Landing', 'ReleaseManual')]
    [string]$Mode = 'Affected',

    [string]$Since = '',

    [switch]$AllowReleaseManual,

    [switch]$Plan,

    [switch]$AllValidators
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$global:LASTEXITCODE = 1

function Invoke-Captured {
    param(
        [Parameter(Mandatory = $true)][string]$Program,
        [Parameter(Mandatory = $true)][string[]]$Arguments,
        [string]$WorkingDirectory = $script:RepoRoot,
        [switch]$AllowFailure
    )

    $oldPreference = $ErrorActionPreference
    $stderrPath = [IO.Path]::GetTempFileName()
    try {
        $ErrorActionPreference = 'Continue'
        Push-Location $WorkingDirectory
        $output = @(& $Program @Arguments 2> $stderrPath)
        $exitCode = $LASTEXITCODE
        $stderr = [IO.File]::ReadAllText($stderrPath)
        $result = [pscustomobject]@{
            ExitCode = $exitCode
            Stdout   = (($output | ForEach-Object { [string]$_ }) -join "`n")
            Stderr   = $stderr
        }
    }
    finally {
        Pop-Location
        Remove-Item -LiteralPath $stderrPath -Force -ErrorAction SilentlyContinue
        $ErrorActionPreference = $oldPreference
    }

    if ($result.ExitCode -ne 0 -and -not $AllowFailure) {
        throw ("{0} {1} failed (exit {2})`n{3}`n{4}" -f $Program, ($Arguments -join ' '), $result.ExitCode, $result.Stderr, $result.Stdout)
    }
    return $result
}

function Convert-ToRepoPath {
    param([Parameter(Mandatory = $true)][string]$Path)

    $normalized = $Path.Replace('\', '/')
    if ($normalized.StartsWith('./', [StringComparison]::Ordinal)) {
        return $normalized.Substring(2)
    }
    return $normalized
}

function Get-ChangedPaths {
    param([string]$RequestedBase)

    $forceAll = $false
    $base = $RequestedBase
    if ([string]::IsNullOrWhiteSpace($base)) {
        foreach ($candidate in @('origin/master', 'origin/main', 'master', 'main')) {
            $probe = Invoke-Captured -Program 'git' -Arguments @('rev-parse', '--verify', '--quiet', $candidate) -AllowFailure
            if ($probe.ExitCode -eq 0) {
                $mergeBase = Invoke-Captured -Program 'git' -Arguments @('merge-base', 'HEAD', $candidate) -AllowFailure
                if ($mergeBase.ExitCode -eq 0 -and -not [string]::IsNullOrWhiteSpace($mergeBase.Stdout)) {
                    $base = $mergeBase.Stdout.Trim()
                    break
                }
            }
        }
        if ([string]::IsNullOrWhiteSpace($base)) {
            $base = 'HEAD'
            $forceAll = $true
        }
    }
    else {
        [void](Invoke-Captured -Program 'git' -Arguments @('rev-parse', '--verify', "$base^{commit}"))
    }

    $paths = [Collections.Generic.SortedSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    $committed = Invoke-Captured -Program 'git' -Arguments @('diff', '--name-only', "$base...HEAD")
    $working = Invoke-Captured -Program 'git' -Arguments @('diff', '--name-only', 'HEAD')
    $untracked = Invoke-Captured -Program 'git' -Arguments @('ls-files', '--others', '--exclude-standard')
    foreach ($output in @($committed.Stdout, $working.Stdout, $untracked.Stdout)) {
        foreach ($line in ($output -split "`r?`n")) {
            if (-not [string]::IsNullOrWhiteSpace($line)) {
                [void]$paths.Add((Convert-ToRepoPath $line))
            }
        }
    }
    return [pscustomobject]@{
        Base     = $base
        Paths    = @($paths)
        ForceAll = $forceAll
    }
}

function Test-HandledNonCargoPath {
    param([Parameter(Mandatory = $true)][string]$Path)

    return $Path -match '^(?:\.circleci/|\.github/|\.codex/|AGENTS\.md$|PROJECT\.md$|extractor/documents/|replay_interface/documents/|extractor/native/scripts/|extractor/game_defs/|extractor/save_oracle/|replay_interface/tools/|.*(?:^|/)runs/|.*(?:^|/)re_evidence/)'
}

function Test-CitationInput {
    param([Parameter(Mandatory = $true)][string]$Path)

    if ($Path -match '(?:^|/)(?:target|\.cargo-target|\.git|\.claude|node_modules|\.venv|\.pnpm-store|\.reference|\.t3-worktrees|captures|runs|re_evidence)(?:/|$)') {
        return $false
    }
    return $Path -notmatch '(?i)\.(?:png|jpe?g|gif|bmp|ico|tga|dds|webp|svgz|zip|gz|xz|zst|7z|rar|tar|cab|msi|exe|dll|pdb|lib|obj|so|dylib|a|o|bin|dat|pak|ttf|otf|woff2?|wav|mp3|ogg|mp4|avi|mkv|pyc|pyd|class|jar|rlib|rmeta|db|sqlite|parquet)$'
}

function Get-PathTextAtRevision {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$Base
    )

    $current = Join-Path $script:RepoRoot ($Path.Replace('/', '\'))
    $parts = @()
    if (Test-Path -LiteralPath $current -PathType Leaf) {
        try { $parts += [IO.File]::ReadAllText($current) } catch { }
    }
    $old = Invoke-Captured -Program 'git' -Arguments @('show', "${Base}:$Path") -AllowFailure
    if ($old.ExitCode -eq 0) { $parts += $old.Stdout }
    return ($parts -join "`n")
}

function Test-WorldFoldInput {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$Base
    )

    if ($Path -match '^extractor/native/scripts/(?:verify|restamp)_world_fold_citations\.py$' -or
        $Path -match '^extractor/native/scripts/world_fold_.*\.(?:json|txt)$' -or
        $Path -eq 'extractor/native/recorder_host/src/world_fold.rs') {
        return $true
    }
    $text = Get-PathTextAtRevision -Path $Path -Base $Base
    return $text -match '@(?:world-fold-authority|lane-payload-authority|leaf-source-authority)|world_fold\.rs(?:::|:)'
}

function Get-CargoTopology {
    $rootPaths = @(
        'extractor/native',
        'replay_interface/native',
        'portable_replay/native/admission',
        'portable_replay/native/compact_stream',
        'portable_replay/native/content_manifest',
        'portable_replay/native/render_stream',
        'portable_replay/native/snapshot'
    )

    $roots = @()
    $packagesById = @{}
    $reverse = @{}
    $forward = @{}
    $forwardProduction = @{}
    foreach ($relativeRoot in $rootPaths) {
        $absoluteRoot = Join-Path $script:RepoRoot ($relativeRoot.Replace('/', '\'))
        $metadataText = & cargo metadata --manifest-path (Join-Path $absoluteRoot 'Cargo.toml') `
            --no-deps --format-version 1
        if ($LASTEXITCODE -ne 0) {
            throw "cargo metadata failed for $relativeRoot with exit code $LASTEXITCODE"
        }
        $metadata = $metadataText | ConvertFrom-Json
        $members = [Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
        foreach ($member in @($metadata.workspace_members)) { [void]$members.Add([string]$member) }
        $memberPackages = @($metadata.packages | Where-Object { $members.Contains([string]$_.id) })
        $roots += [pscustomobject]@{
            Relative = $relativeRoot; Absolute = $absoluteRoot; Packages = $memberPackages
        }
        foreach ($package in $memberPackages) {
            $id = [string]$package.id
            $packagesById[$id] = $package
            $reverse[$id] = [Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
            $forward[$id] = [Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
            $forwardProduction[$id] = [Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
        }
    }

    $manifestToId = @{}
    foreach ($entry in $packagesById.GetEnumerator()) {
        $manifestToId[[IO.Path]::GetFullPath([string]$entry.Value.manifest_path)] = [string]$entry.Key
    }
    foreach ($entry in $packagesById.GetEnumerator()) {
        $dependentId = [string]$entry.Key
        foreach ($dependency in @($entry.Value.dependencies | Where-Object {
            $null -ne $_.PSObject.Properties['path'] -and $null -ne $_.path
        })) {
            $dependencyManifest = [IO.Path]::GetFullPath((Join-Path ([string]$dependency.path) 'Cargo.toml'))
            if (-not $manifestToId.ContainsKey($dependencyManifest)) { continue }
            $dependencyId = [string]$manifestToId[$dependencyManifest]
            [void]$reverse[$dependencyId].Add($dependentId)
            [void]$forward[$dependentId].Add($dependencyId)
            if ([string]$dependency.kind -cne 'dev') { [void]$forwardProduction[$dependentId].Add($dependencyId) }
        }
    }
    return [pscustomobject]@{
        Roots = $roots; PackagesById = $packagesById; Reverse = $reverse
        Forward = $forward; ForwardProduction = $forwardProduction
    }
}

function Get-PackageSourceRoots {
    param([Parameter(Mandatory = $true)]$Package)

    $roots = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    [void]$roots.Add([IO.Path]::GetFullPath((Split-Path -Parent ([string]$Package.manifest_path))))
    foreach ($target in @($Package.targets)) {
        $source = [IO.Path]::GetFullPath([string]$target.src_path)
        $directory = Split-Path -Parent $source
        if ((Split-Path -Leaf $directory) -eq 'bin') { $directory = Split-Path -Parent $directory }
        if ((Split-Path -Leaf $directory) -eq 'src') { [void]$roots.Add($directory) }
    }
    return @($roots)
}

function Get-AffectedPackageIds {
    param(
        [Parameter(Mandatory = $true)]$Topology,
        [Parameter(Mandatory = $true)][string[]]$ChangedPaths
    )

    $selected = [Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    $uncertain = $false
    foreach ($relative in $ChangedPaths) {
        if ($relative -in @(
            'extractor/native/scripts/Invoke-Validation.ps1',
            '.circleci/config.yml'
        )) {
            return [pscustomobject]@{ PackageIds = @(); ForceAll = $true }
        }
        if ($relative -match '(^|/)(?:Cargo\.toml|Cargo\.lock|rust-toolchain(?:\.toml)?|nextest\.toml)$' -or
            $relative -match '^(?:\.cargo/|\.config/)') {
            return [pscustomobject]@{ PackageIds = @(); ForceAll = $true }
        }
        $absolute = [IO.Path]::GetFullPath((Join-Path $script:RepoRoot ($relative.Replace('/', '\'))))
        $owners = @()
        foreach ($entry in $Topology.PackagesById.GetEnumerator()) {
            foreach ($sourceRoot in (Get-PackageSourceRoots -Package $entry.Value)) {
                if ($absolute -ieq $sourceRoot -or $absolute.StartsWith($sourceRoot + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
                    $owners += [string]$entry.Key
                    break
                }
            }
        }
        if ($owners.Count -eq 0) {
            if (-not (Test-HandledNonCargoPath $relative)) { $uncertain = $true }
            continue
        }
        foreach ($owner in $owners) { [void]$selected.Add($owner) }
    }
    if ($uncertain) { return [pscustomobject]@{ PackageIds = @(); ForceAll = $true } }

    $queue = [Collections.Generic.Queue[string]]::new()
    foreach ($id in $selected) { $queue.Enqueue($id) }
    while ($queue.Count -gt 0) {
        $id = $queue.Dequeue()
        if (-not $Topology.Reverse.ContainsKey($id)) { continue }
        foreach ($dependent in $Topology.Reverse[$id]) {
            if ($selected.Add([string]$dependent)) { $queue.Enqueue([string]$dependent) }
        }
    }
    return [pscustomobject]@{ PackageIds = @($selected); ForceAll = $false }
}

function Get-ShippingPackageIds {
    param([Parameter(Mandatory = $true)]$Topology)

    $selected = [Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    $queue = [Collections.Generic.Queue[string]]::new()
    foreach ($entry in $Topology.PackagesById.GetEnumerator()) {
        if ([string]$entry.Value.name -in @('collector', 'recorder_host', 'replay_app')) {
            if ($selected.Add([string]$entry.Key)) { $queue.Enqueue([string]$entry.Key) }
        }
    }
    while ($queue.Count -gt 0) {
        $id = $queue.Dequeue()
        if (-not $Topology.ForwardProduction.ContainsKey($id)) { continue }
        foreach ($dependency in $Topology.ForwardProduction[$id]) {
            if ($selected.Add([string]$dependency)) { $queue.Enqueue([string]$dependency) }
        }
    }
    return @($selected | Where-Object {
        [string]$Topology.PackagesById[$_].name -notin @('production_hook_receipt', 'replay_test_gates')
    })
}

function Test-ProductionInteractionInput {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)]$Topology,
        [Parameter(Mandatory = $true)][string[]]$ProductionPackageIds
    )

    if ($Path -in @(
        'extractor/native/scripts/Test-ProductionInteractionClosure.ps1',
        'extractor/native/scripts/PortableProduct.InteractionClosure.ps1',
        'extractor/native/scripts/Start-Hoi4ProductionCaptureSafe.ps1',
        'extractor/native/scripts/Read-Hoi4GameState.ps1',
        'extractor/native/scripts/New-PortableProductPackage.ps1',
        'extractor/native/scripts/Test-PortableProductPackageReceipt.ps1',
        'extractor/native/scripts/ProductionCapture.Promotion.ps1',
        'extractor/native/scripts/Invoke-CentralRustValidation.ps1',
        'extractor/native/scripts/ProductionCapture.Validation.ps1',
        'extractor/native/scripts/verify_sink_reconciliation.py',
        'extractor/native/scripts/PassiveCapture.Lifecycle.ps1',
        'portable_replay/native/scripts/ProductionArtifact.Attestation.ps1',
        'extractor/native/Cargo.toml',
        'extractor/native/Cargo.lock',
        'replay_interface/native/Cargo.toml',
        'replay_interface/native/Cargo.lock'
    ) -or $Path -match '^extractor/native/scripts/production_.*(?:interaction|suspend).*(?:json|txt)$') {
        return $true
    }

    $absolute = [IO.Path]::GetFullPath((Join-Path $script:RepoRoot ($Path.Replace('/', '\'))))
    foreach ($id in $ProductionPackageIds) {
        if (-not $Topology.PackagesById.ContainsKey($id)) { continue }
        foreach ($sourceRoot in (Get-PackageSourceRoots -Package $Topology.PackagesById[$id])) {
            if ($absolute -ieq $sourceRoot -or $absolute.StartsWith($sourceRoot + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
                return $true
            }
        }
    }
    return $false
}

function Test-ShippingTestTarget {
    param([Parameter(Mandatory = $true)][string]$Name)

    return $Name -notin @(
        'citation_integrity', 'world_fold_citation_authority',
        'c07_generated_library_matches_generator',
        'script_status_vocabulary_sweep', 'no_silent_skip', 'gate_currency',
        'extractor_dependency_boundary', 'replay_dependency_boundary',
        'soak', 'soak_f2', 'fow_hidden_counter_evidence',
        'frame162662_label_size_gate', 'frame4700_label_size_gate',
        'mapname_7b_acceptance', 'mapname_oracle63_gate',
        'o05_fixed_object_promotion', 'xui09_installed_game_evidence',
        'xui22_installed_game_evidence', 'decision_chrome', 'event_chrome',
        'flag_precedence', 'markup_law', 'topbar_quads',
        'xui01_definition_admission', 'xui02_window_manager', 'xui17_characters'
    )
}

function Invoke-ShippingTests {
    param(
        [Parameter(Mandatory = $true)]$Topology,
        [Parameter(Mandatory = $true)][string[]]$ShippingIds,
        [string[]]$SelectedIds,
        [object[]]$ExtraPlans = @(),
        [switch]$All,
        [switch]$PlanOnly
    )

    $shippingBins = @('collector_record', 'replay_app')
    $plans = @()
    foreach ($root in $Topology.Roots) {
        $packages = @($root.Packages | Where-Object {
            $ShippingIds -contains [string]$_.id -and
            ($All -or $SelectedIds -contains [string]$_.id)
        })
        if ($packages.Count -eq 0) { continue }

        $hasLibrary = $false
        $tests = [Collections.Generic.SortedSet[string]]::new([StringComparer]::Ordinal)
        $bins = [Collections.Generic.SortedSet[string]]::new([StringComparer]::Ordinal)
        foreach ($package in $packages) {
            foreach ($target in @($package.targets)) {
                $kinds = @($target.kind)
                if (@($kinds | Where-Object { $_ -in @('lib', 'rlib', 'cdylib', 'staticlib', 'proc-macro') }).Count -gt 0) {
                    $hasLibrary = $true
                }
                if ($kinds -contains 'test' -and (Test-ShippingTestTarget ([string]$target.name))) {
                    [void]$tests.Add([string]$target.name)
                }
                if ($kinds -contains 'bin' -and $shippingBins -contains [string]$target.name) {
                    [void]$bins.Add([string]$target.name)
                }
            }
        }
        if (-not $hasLibrary -and $tests.Count -eq 0 -and $bins.Count -eq 0) { continue }

        $arguments = @(
            'nextest', 'run', '--manifest-path', (Join-Path $root.Absolute 'Cargo.toml'),
            '--config-file', (Join-Path $script:RepoRoot '.config\nextest.toml'),
            '--profile', 'ci', '--no-fail-fast'
        )
        if (Test-Path -LiteralPath (Join-Path $root.Absolute 'Cargo.lock') -PathType Leaf) { $arguments += '--locked' }
        foreach ($package in $packages) { $arguments += @('-p', [string]$package.name) }
        if ($hasLibrary) { $arguments += '--lib' }
        foreach ($test in $tests) { $arguments += @('--test', $test) }
        foreach ($bin in $bins) { $arguments += @('--bin', $bin) }
        $plans += [pscustomobject]@{
            Name = $root.Relative; Program = 'cargo'; Arguments = $arguments
        }
    }
    $cargoPlanCount = $plans.Count
    $plans += @($ExtraPlans)
    $prefix = if ($PlanOnly) { 'PLAN> ' } else { '> ' }
    foreach ($item in $plans) {
        Write-Host ($prefix + "$($item.Program) $($item.Arguments -join ' ')")
    }
    if ($PlanOnly -or $plans.Count -eq 0) { return $cargoPlanCount }

    foreach ($item in $plans) {
        $program = [string]$item.Program
        $arguments = @($item.Arguments)
        & $program @arguments
        if ($LASTEXITCODE -ne 0) {
            throw "$($item.Name) validation failed with exit code $LASTEXITCODE"
        }
    }
    return $cargoPlanCount
}

try {
    $script:RepoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..\..'))
    Set-Location $script:RepoRoot

    if ($Mode -eq 'ReleaseManual') {
        if (-not $AllowReleaseManual) {
            throw 'ReleaseManual is blocked. Re-run with -Mode ReleaseManual -AllowReleaseManual only when the owner explicitly requests release/manual validation.'
        }
        if ($Plan) {
            Write-Host 'PLAN> Invoke-CentralRustValidation.ps1 -Mode Release -Workspace All'
            $global:LASTEXITCODE = 0
            return
        }
        & (Join-Path $PSScriptRoot 'Invoke-CentralRustValidation.ps1') -Mode Release -Workspace All
        if ($LASTEXITCODE -ne 0) {
            throw "release/manual validation failed with exit code $LASTEXITCODE"
        }
        $global:LASTEXITCODE = 0
        return
    }

    $changes = Get-ChangedPaths -RequestedBase $Since
    $topology = Get-CargoTopology
    $shippingPackageIds = @(Get-ShippingPackageIds -Topology $topology)
    $runAll = $Mode -eq 'Landing' -or $changes.ForceAll
    $selection = $null
    if (-not $runAll) {
        $selection = Get-AffectedPackageIds -Topology $topology -ChangedPaths $changes.Paths
        $runAll = $selection.ForceAll
    }

    $python = if (Get-Command python -ErrorAction SilentlyContinue) { 'python' } elseif (Get-Command py -ErrorAction SilentlyContinue) { 'py' } else { '' }
    $citationChanged = $AllValidators -or @($changes.Paths | Where-Object { Test-CitationInput $_ }).Count -gt 0
    $worldFoldChanged = $AllValidators -or @($changes.Paths | Where-Object { Test-WorldFoldInput -Path $_ -Base $changes.Base }).Count -gt 0
    $productModsetChanged = $Mode -eq 'Landing' -or @($changes.Paths | Where-Object {
        $_ -like 'extractor/game_defs/*'
    }).Count -gt 0
    $peaceOracleChanged = $Mode -eq 'Landing' -or @($changes.Paths | Where-Object {
        $_ -like 'extractor/save_oracle/*'
    }).Count -gt 0
    $productionPackageIds = $shippingPackageIds
    $interactionChanged = $AllValidators -or @($changes.Paths | Where-Object {
        Test-ProductionInteractionInput -Path $_ -Topology $topology -ProductionPackageIds $productionPackageIds
    }).Count -gt 0
    if (($citationChanged -or $worldFoldChanged -or $productModsetChanged -or $peaceOracleChanged) -and
        [string]::IsNullOrWhiteSpace($python)) {
        throw 'Python is required for the product or citation tests selected by this change.'
    }

    $extraPlans = @()
    if ($citationChanged) {
        $extraPlans += [pscustomobject]@{
            Name = 'citation integrity'; Program = $python
            Arguments = @((Join-Path $PSScriptRoot 'verify_citation_integrity.py'), '--repo-root', $script:RepoRoot)
        }
    }
    if ($worldFoldChanged) {
        $extraPlans += [pscustomobject]@{
            Name = 'world-fold citations'; Program = $python
            Arguments = @((Join-Path $PSScriptRoot 'verify_world_fold_citations.py'), '--repo-root', $script:RepoRoot)
        }
    }
    if ($productModsetChanged) {
        $extraPlans += [pscustomobject]@{
            Name = 'product modset admission'; Program = $python
            Arguments = @((Join-Path $script:RepoRoot 'extractor\game_defs\tests\test_product_modset_admission.py'))
        }
    }
    if ($peaceOracleChanged) {
        $extraPlans += [pscustomobject]@{
            Name = 'peace save oracle'; Program = $python
            Arguments = @((Join-Path $script:RepoRoot 'extractor\save_oracle\test_parse_intel_peace.py'))
        }
    }
    if ($interactionChanged) {
        $extraPlans += [pscustomobject]@{
            Name = 'production interaction closure'; Program = 'powershell.exe'
            Arguments = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', (Join-Path $PSScriptRoot 'Test-ProductionInteractionClosure.ps1'))
        }
    }

    if ($runAll) {
        Write-Host "$Mode validation: all shipping-product tests (base $($changes.Base))."
        $testRuns = Invoke-ShippingTests -Topology $topology -ShippingIds $shippingPackageIds `
            -ExtraPlans $extraPlans -All -PlanOnly:$Plan
    }
    else {
        Write-Host ("Affected validation: {0} changed path(s), {1} affected package(s), base {2}." -f $changes.Paths.Count, $selection.PackageIds.Count, $changes.Base)
        $testRuns = Invoke-ShippingTests -Topology $topology -ShippingIds $shippingPackageIds `
            -SelectedIds $selection.PackageIds -ExtraPlans $extraPlans -PlanOnly:$Plan
    }
    if ($testRuns -eq 0 -and $extraPlans.Count -eq 0) {
        Write-Host 'No shipping product or validator input changed.'
    }
    $global:LASTEXITCODE = 0
}
catch {
    $global:LASTEXITCODE = 1
    throw
}
