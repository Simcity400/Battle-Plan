[CmdletBinding()]
param(
    [ValidateSet('Affected', 'Landing', 'ReleaseManual')]
    [string]$Mode = 'Affected',

    [string]$Since = '',

    [switch]$AllowReleaseManual,

    [string]$ProductionCaptureReceiptPath = '',

    [string]$C07GameRoot = '',

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
        'portable_replay/native/snapshot',
        # Standalone root so its own pins (junction_pin, no_console_shellouts)
        # execute at landing; depending crates never run another crate's
        # integration tests.
        'portable_replay/native/test_fs_retry'
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
        # portable_test_fs_retry is seeded explicitly: it is referenced only
        # through dev-dependencies, so the production-forward walk below can
        # never reach it, yet its integration pins (junction_pin,
        # no_console_shellouts) must execute at landing.
        if ([string]$entry.Value.name -in @('collector', 'recorder_host', 'replay_app', 'portable_test_fs_retry')) {
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

function Test-ShippingTestTarget {
    param([Parameter(Mandatory = $true)][string]$Name)

    # Exclusion audit (lane QG follow-up, 2026-08-24). Every name kept out of
    # the landing plan has a recorded reason:
    #   citation_integrity / world_fold_citation_authority - no Rust targets
    #     exist under these names; the gate validates them through
    #     its own python ExtraPlans (verify_citation_integrity.py /
    #     verify_world_fold_citations.py below, scheduled when their inputs
    #     change or -AllValidators is passed), and the names stay listed so
    #     a duplicate Rust reimplementation cannot shadow that authority;
    #   soak / soak_f2 - long-run soak binaries with their own [profile.soak];
    #   fow_hidden_counter_evidence - gate_renderdoc_mined: mined RenderDoc
    #     capture evidence;
    #   frame162662_label_size_gate / frame4700_label_size_gate /
    #     mapname_oracle63_gate - gate_map_bundle(_with_hoi4): map bundle
    #     generated from live captures (oracle63 also needs the install);
    #   c07_generated_library_matches_generator - the generator verifies
    #     against PINNED vanilla installed content and refuses anything else, while
    #     Landing forwards no game root (only ReleaseManual takes
    #     -C07GameRoot), so this is install-backed like the gates above;
    #   xui09_installed_game_evidence / xui22_installed_game_evidence -
    #     gate_hoi4_install;
    #   decision_chrome / event_chrome / flag_precedence / markup_law /
    #     topbar_quads / xui01_definition_admission / xui02_window_manager /
    #     xui17_characters - install-backed game_ui evidence harness binaries,
    #     run manually under [profile.evidence].
    # Everything else runs in the gate: no_silent_skip (the false-green early-
    # return scanner; it was red on master for days precisely because it was
    # not planned), gate_currency, script_status_vocabulary_sweep,
    # extractor_dependency_boundary and replay_dependency_boundary are fast
    # deterministic source/metadata checks; mapname_7b_acceptance and
    # o05_fixed_object_promotion each carry exactly one install-gated test,
    # and those two functions stay excluded from ci BY TEST NAME in
    # .config/nextest.toml (draw_time_cull_matches_the_captured_drawn_set,
    # vanilla_admission_matches_all_frozen_o05_semantics), so planning the
    # targets runs only their committed-fixture deterministic tests.
    return $Name -notin @(
        'citation_integrity', 'world_fold_citation_authority',
        'c07_generated_library_matches_generator',
        'soak', 'soak_f2', 'fow_hidden_counter_evidence',
        'frame162662_label_size_gate', 'frame4700_label_size_gate',
        'mapname_oracle63_gate',
        'xui09_installed_game_evidence',
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

        # The base config's binary() clauses (soak, soak_f2, the game_ui evidence
        # binaries) all name replay_interface test targets; nextest validates every
        # profile's filter against the running workspace and hard-fails on a
        # binary() that matches nothing, so every other workspace gets the
        # derived config with those clauses removed.
        $configName = if ($root.Relative -eq 'replay_interface/native') { 'nextest.toml' } else { 'nextest.no-ui.toml' }
        $arguments = @(
            'nextest', 'run', '--manifest-path', (Join-Path $root.Absolute 'Cargo.toml'),
            '--config-file', (Join-Path $script:RepoRoot ('.config\' + $configName)),
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

    # The landing gate's compile log carried ~8,600 dead-code lint lines per
    # run (test-only helpers that exist precisely because production code does
    # not call them), burying real diagnostics. Silence ONLY that lint for the
    # gate's own compiles; every other warning and every hard error still
    # prints, and ordinary developer builds (which do not set this variable)
    # keep seeing everything. NOTE: changing RUSTFLAGS changes cargo's cache
    # key, so the FIRST gate run after this lands recompiles the test profile
    # from scratch once; subsequent runs reuse the new cache. The variable is
    # set only when absent and removed again in the finally below, so a gate
    # run inside a live PowerShell session cannot leak -Adead_code (or its
    # cache key) into that session's later cargo builds.
    $gateSetRustFlags = $false
    if (-not [Environment]::GetEnvironmentVariable('RUSTFLAGS')) {
        $env:RUSTFLAGS = '-Adead_code'
        $gateSetRustFlags = $true
        Write-Host 'gate: RUSTFLAGS=-Adead_code (dead-code lint silenced for this gate run only)'
    }
    try {
        $cacheDir = Join-Path $script:RepoRoot '.cargo-target'
        New-Item -ItemType Directory -Force -Path $cacheDir | Out-Null
        $cacheTag = Join-Path $cacheDir 'CACHEDIR.TAG'
        if (-not (Test-Path -LiteralPath $cacheTag -PathType Leaf)) {
            Set-Content -LiteralPath $cacheTag -Encoding ascii -Value @(
                'Signature: 8a477f597d28d172789f06886806bc55'
                '# Cache directory tag (https://bford.info/cachedir/spec.html).'
                '# This directory holds the landing gate build cache.'
                '#'
            )
        }

        # ---- Phase A: build every cargo root, strictly serially. ------------
        # All workspaces share ONE target directory: .cargo/config.toml sets
        # [build] target-dir = ".cargo-target", resolved against the repo root,
        # so extractor/native, replay_interface/native and the five
        # portable_replay roots all compile into <repo>/.cargo-target (probe:
        # a check of compact_stream created <repo>/.cargo-target, nothing
        # per-workspace). Concurrent cargo front-ends would serialize on that
        # directory's .cargo-lock anyway, so overlapping BUILDS buys nothing;
        # every root compiles first with a single lock holder at a time.
        foreach ($item in @($plans | Select-Object -First $cargoPlanCount)) {
            # --no-run is build-only and rejects run-phase flags: --no-fail-fast
            # (the only one the plans carry) must be stripped here and stays on
            # the phase-B invocation below.
            $buildArguments = @($item.Arguments | Where-Object { $_ -ne '--no-fail-fast' }) + '--no-run'
            Write-Host ('BUILD> ' + "$([string]$item.Program) $($buildArguments -join ' ')")
            & ([string]$item.Program) @buildArguments
            if ($LASTEXITCODE -ne 0) {
                throw "$($item.Name) build failed with exit code $LASTEXITCODE"
            }
        }

        # ---- Phase B: overlap the test phases. ------------------------------
        # nextest compiles through a short-lived cargo child process and does
        # NOT hold the target-dir lock while tests execute (measured probe:
        # two concurrent `cargo nextest run` invocations sharing one
        # CARGO_TARGET_DIR overlapped; the second finished its own build while
        # the first's 25 s test was still running). Phase-A left fresh
        # fingerprints, so phase-B builds are seconds-long freshness checks.
        # Each lane's output is buffered and printed on completion to keep the
        # gate log readable; unlike the old serial loop, one failing root no
        # longer aborts the remaining roots - all lanes run, then failures are
        # reported together (per-root --no-fail-fast semantics already in
        # place inside each plan).
        $lanes = @()
        foreach ($item in $plans) {
            $laneScript = {
                param($Prog, $ArgArr, $Cwd)
                Set-Location $Cwd
                $output = & $Prog @ArgArr 2>&1
                $exitCode = if ($null -eq $LASTEXITCODE) { 1 } else { $LASTEXITCODE }
                [pscustomobject]@{ ExitCode = $exitCode; Output = @($output) }
            }
            $lanes += @{
                Name = [string]$item.Name
                Job = Start-Job -ScriptBlock $laneScript -ArgumentList @(
                    [string]$item.Program, (, @($item.Arguments)), $script:RepoRoot)
            }
        }
        $failures = @()
        foreach ($lane in $lanes) {
            Wait-Job $lane.Job | Out-Null
            $result = Receive-Job $lane.Job
            Remove-Job $lane.Job
            foreach ($line in @($result.Output)) { Write-Host "[$($lane.Name)] $line" }
            if ([int]$result.ExitCode -ne 0) {
                $failures += "$($lane.Name) validation failed with exit code $($result.ExitCode)"
            }
        }
        if ($failures.Count -gt 0) { throw ($failures -join '; ') }
        return $cargoPlanCount
    } finally {
        if ($gateSetRustFlags) {
            Remove-Item -LiteralPath 'Env:RUSTFLAGS' -ErrorAction SilentlyContinue
        }
    }
}

try {
    $script:RepoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..\..'))
    Set-Location $script:RepoRoot

    if ($Mode -eq 'ReleaseManual') {
        if (-not $AllowReleaseManual) {
            throw 'ReleaseManual is blocked. Re-run with -Mode ReleaseManual -AllowReleaseManual only when the owner explicitly requests release/manual validation.'
        }
        $releaseArguments = @{
            AllowReleaseManual = $true
            Plan = $Plan.IsPresent
        }
        if (-not [string]::IsNullOrWhiteSpace($ProductionCaptureReceiptPath)) {
            $releaseArguments.ProductionCaptureReceiptPath = $ProductionCaptureReceiptPath
        }
        if (-not [string]::IsNullOrWhiteSpace($C07GameRoot)) {
            $releaseArguments.C07GameRoot = $C07GameRoot
        }
        & (Join-Path $PSScriptRoot 'Invoke-ReleaseManual.ps1') @releaseArguments
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
