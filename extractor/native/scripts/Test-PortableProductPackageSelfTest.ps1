[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = (Resolve-Path (Join-Path $scriptDir '..\..\..')).Path
$builderSource = Join-Path $scriptDir 'New-PortableProductPackage.ps1'
$verifierSource = Join-Path $scriptDir 'Test-PortableProductPackageReceipt.ps1'
$attestationSource = Join-Path $scriptDir 'ProductionArtifact.Attestation.ps1'
. (Join-Path $scriptDir 'PortableProduct.InteractionClosure.ps1')
. (Join-Path $scriptDir 'ProductionCapture.Promotion.ps1')
$script:PortablePackageSelfTestCases = 0

function Assert-Case {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][bool]$Condition
    )
    if (-not $Condition) { throw "portable package self-test failed: $Name" }
    $script:PortablePackageSelfTestCases++
}

function Invoke-ExpectFailure {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][scriptblock]$Body
    )
    try {
        & $Body
    }
    catch {
        $script:PortablePackageSelfTestCases++
        return
    }
    throw "portable package self-test expected failure: $Name"
}

function Set-U16 {
    param([byte[]]$Bytes, [int]$Offset, [uint16]$Value)
    [Array]::Copy([BitConverter]::GetBytes($Value), 0, $Bytes, $Offset, 2)
}

function Set-U32 {
    param([byte[]]$Bytes, [int]$Offset, [uint32]$Value)
    [Array]::Copy([BitConverter]::GetBytes($Value), 0, $Bytes, $Offset, 4)
}

function New-MinimalAmd64Pe {
    param(
        [Parameter(Mandatory = $true)][bool]$Dll,
        [Parameter(Mandatory = $true)][bool]$Gui,
        [Parameter(Mandatory = $true)][byte]$Marker,
        [string[]]$EmbeddedText = @()
    )
    $bytes = [byte[]]::new(1024)
    Set-U16 $bytes 0 0x5a4d
    Set-U32 $bytes 0x3c 0x80
    Set-U32 $bytes 0x80 0x00004550
    Set-U16 $bytes 0x84 0x8664
    Set-U16 $bytes 0x86 1
    Set-U16 $bytes 0x94 0xf0
    $characteristics = if ($Dll) { 0x2002 } else { 0x0002 }
    Set-U16 $bytes 0x96 $characteristics
    Set-U16 $bytes 0x98 0x020b
    Set-U16 $bytes 0xdc $(if ($Gui) { 2 } else { 3 })
    $textOffset = 0x200
    foreach ($text in $EmbeddedText) {
        $encoded = [Text.Encoding]::ASCII.GetBytes($text + [char]0)
        if ($textOffset + $encoded.Length -ge $bytes.Length - 1) {
            throw "minimal PE fixture has insufficient marker capacity"
        }
        [Array]::Copy($encoded, 0, $bytes, $textOffset, $encoded.Length)
        $textOffset += $encoded.Length
    }
    $bytes[$bytes.Length - 1] = $Marker
    return ,$bytes
}

function Write-Bytes {
    param([string]$Path, [byte[]]$Bytes)
    [IO.Directory]::CreateDirectory((Split-Path -Parent $Path)) | Out-Null
    [IO.File]::WriteAllBytes($Path, $Bytes)
}

function Get-FixtureRelativePath {
    param(
        [Parameter(Mandatory = $true)][string]$RepoRoot,
        [Parameter(Mandatory = $true)][string]$Path
    )
    $repo = [IO.Path]::GetFullPath($RepoRoot).TrimEnd('\')
    $full = [IO.Path]::GetFullPath($Path)
    if (-not $full.StartsWith($repo + '\', [StringComparison]::OrdinalIgnoreCase)) {
        throw "self-test fixture input is outside the repository: $full"
    }
    return $full.Substring($repo.Length + 1)
}

function Copy-FixtureInput {
    param(
        [Parameter(Mandatory = $true)][string]$Source,
        [Parameter(Mandatory = $true)][string]$Destination
    )
    [IO.Directory]::CreateDirectory((Split-Path -Parent $Destination)) | Out-Null
    Copy-Item -LiteralPath $Source -Destination $Destination -Force
}

function Write-InertFixtureSource {
    param([Parameter(Mandatory = $true)][string]$Destination)
    [IO.Directory]::CreateDirectory((Split-Path -Parent $Destination)) | Out-Null
    [IO.File]::WriteAllText(
        $Destination,
        "// synthetic production source fixture`n",
        [Text.UTF8Encoding]::new($false)
    )
}

function Invoke-JsonScript {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string[]]$Arguments
    )
    $stdout = @(& powershell.exe -NoProfile -ExecutionPolicy Bypass -File $Path @Arguments)
    if ($LASTEXITCODE -ne 0) {
        throw "fixture script exited $LASTEXITCODE`: $Path"
    }
    return (($stdout -join [Environment]::NewLine).Trim() | ConvertFrom-Json)
}

$root = Join-Path ([IO.Path]::GetTempPath()) ("hoi4-portable-package-selftest-{0}" -f [guid]::NewGuid().ToString('N'))
try {
    $fixtureScripts = Join-Path $root 'extractor\native\scripts'
    $releaseRoot = Join-Path $root '.cargo-target\release'
    [IO.Directory]::CreateDirectory($fixtureScripts) | Out-Null
    [IO.Directory]::CreateDirectory((Join-Path $root 'portable_replay\native')) | Out-Null
    [IO.Directory]::CreateDirectory((Join-Path $root 'replay_interface\native')) | Out-Null
    [IO.Directory]::CreateDirectory($releaseRoot) | Out-Null

    foreach ($name in @(
        'Hoi4ClientDriving.OptIn.ps1',
        'PortableProduct.PeImports.ps1',
        'PortableProduct.InteractionClosure.ps1',
        'ProductionCapture.Promotion.ps1',
        'New-PortableProductPackage.ps1',
        'Test-PortableProductPackageReceipt.ps1',
        'ProductionArtifact.Attestation.ps1',
        'Read-Hoi4GameState.ps1',
        'Test-PassiveCapturePolicy.ps1',
        'PassiveCapture.Lifecycle.ps1'
    )) {
        Copy-Item -LiteralPath (Join-Path $scriptDir $name) -Destination (Join-Path $fixtureScripts $name)
    }
    # Deriving the fixture from the authority's own walk means this file no
    # longer double-checks that walk by restating it. The pin below is the
    # compensation: a hand-written manifest graph with a hand-written expected
    # answer, so a traversal that stops following lane-to-lane edges, resolves a
    # workspace-inherited path against the wrong base, or starts reporting a
    # production unit as one of its own dependencies still fails by name.
    $pinnedGraphRoot = Join-Path $root '.pinned_manifest_graph'
    $pinnedGraphManifests = [ordered]@{
        'extractor\native\Cargo.toml' = @'
[workspace]
members = ["collector", "recorder_host", "lanes/lane_alpha", "lanes/lane_beta"]

[workspace.dependencies]
pinned_workspace_dep = { path = "pinned/workspace_dep" }
'@
        'extractor\native\Cargo.lock' = @'
version = 3
'@
        'extractor\native\recorder_host\Cargo.toml' = @'
[package]
name = "recorder_host"
'@
        'extractor\native\collector\Cargo.toml' = @'
[package]
name = "collector"

[dependencies]
lane_alpha = { path = "../lanes/lane_alpha" }
shared_alpha = { path = "../shared_alpha" }
'@
        'extractor\native\lanes\lane_alpha\Cargo.toml' = @'
[package]
name = "lane_alpha"

[dependencies]
lane_beta = { path = "../lane_beta" }
pinned_workspace_dep = { workspace = true }
'@
        'extractor\native\lanes\lane_beta\Cargo.toml' = @'
[package]
name = "lane_beta"
'@
        'extractor\native\shared_alpha\Cargo.toml' = @'
[package]
name = "shared_alpha"
'@
        'extractor\native\pinned\workspace_dep\Cargo.toml' = @'
[package]
name = "pinned_workspace_dep"
'@
        'replay_interface\native\Cargo.toml' = @'
[workspace]
members = ["replay_app"]
'@
        'replay_interface\native\Cargo.lock' = @'
version = 3
'@
        'replay_interface\native\replay_app\Cargo.toml' = @'
[package]
name = "replay_app"

[dependencies]
replay_shared = { path = "../replay_shared" }
'@
        'replay_interface\native\replay_shared\Cargo.toml' = @'
[package]
name = "replay_shared"
'@
    }
    foreach ($pinnedManifest in $pinnedGraphManifests.GetEnumerator()) {
        $pinnedManifestPath = Join-Path $pinnedGraphRoot $pinnedManifest.Key
        [IO.Directory]::CreateDirectory((Split-Path -Parent $pinnedManifestPath)) | Out-Null
        [IO.File]::WriteAllText(
            $pinnedManifestPath,
            ([string]$pinnedManifest.Value + "`n"),
            [Text.UTF8Encoding]::new($false)
        )
    }
    $pinnedGraphClosure = @(
        Get-ProductionUnitDependencyClosure -RepoRoot $pinnedGraphRoot
    )
    $pinnedGraphUnits = @(
        $pinnedGraphClosure | ForEach-Object { [string]$_.unit } | Sort-Object
    )
    Assert-Case 'pinned manifest graph resolves the exact production unit closure' (
        ($pinnedGraphUnits -join '|') -ceq (@(
            'extractor/native/collector',
            'extractor/native/lanes/lane_alpha',
            'extractor/native/lanes/lane_beta',
            'extractor/native/recorder_host',
            'replay_interface/native/replay_app'
        ) -join '|')
    )
    $pinnedGraphDependencyRoots = @(
        $pinnedGraphClosure |
            ForEach-Object { @($_.dependencies) } |
            Where-Object { -not $_.production_unit } |
            ForEach-Object {
                Get-FixtureRelativePath `
                    -RepoRoot $pinnedGraphRoot `
                    -Path ([string]$_.root)
            } |
            Sort-Object -Unique
    )
    Assert-Case 'pinned manifest graph resolves the exact dependency roots' (
        ($pinnedGraphDependencyRoots -join '|') -ceq (@(
            'extractor\native\pinned\workspace_dep',
            'extractor\native\shared_alpha',
            'replay_interface\native\replay_shared'
        ) -join '|')
    )
    $pinnedGraphWorkspaceInputs = @(
        $pinnedGraphClosure | ForEach-Object {
            '{0}=>{1}+{2}' -f
                [string]$_.unit,
                (Get-FixtureRelativePath -RepoRoot $pinnedGraphRoot -Path ([string]$_.lock_path)),
                (Get-FixtureRelativePath -RepoRoot $pinnedGraphRoot -Path ([string]$_.workspace_cargo_path))
        } | Sort-Object
    )
    Assert-Case 'pinned manifest graph binds every unit to its owning lock and workspace manifest' (
        ($pinnedGraphWorkspaceInputs -join '|') -ceq (@(
            'extractor/native/collector=>extractor\native\Cargo.lock+extractor\native\Cargo.toml',
            'extractor/native/lanes/lane_alpha=>extractor\native\Cargo.lock+extractor\native\Cargo.toml',
            'extractor/native/lanes/lane_beta=>extractor\native\Cargo.lock+extractor\native\Cargo.toml',
            'extractor/native/recorder_host=>extractor\native\Cargo.lock+extractor\native\Cargo.toml',
            'replay_interface/native/replay_app=>replay_interface\native\Cargo.lock+replay_interface\native\Cargo.toml'
        ) -join '|')
    )

    # This fixture used to name every file it created. The production authority
    # does not work from a list: it walks each production unit's Cargo manifest
    # and reads whatever those path edges reach. The two drifted, and the walk
    # then demanded a crate the list had never heard of
    # (extractor\native\compact_stream, pulled in by verbatim-copied lane
    # manifests), which aborted this self-test before its first case ran.
    #
    # The SHAPE of the fixture is now derived from the authority's own walk -
    # Get-ProductionUnitDependencyClosure, the function the classification
    # itself calls - so there is one traversal and no transcription of it. The
    # cost is that this fixture is no longer an independent check of that walk;
    # a pinned case below feeds the same function a hand-written manifest graph
    # with a hand-written expected answer, so a traversal regression still has a
    # named failing case.
    #
    # CONTENT stays the fixture's own decision. The files listed here are the
    # ones the authority makes content assertions about, so they are mirrored
    # byte for byte; every other source file the walk reaches is created at its
    # real relative path with an inert body. The fixture is a product tree that
    # has to pass, not a copy of the repository - copying the repository's own
    # lane sources would import their forbidden client-capability symbols and
    # this self-test would only ever restate that the repository is what it is.
    $verbatimFixtureSources = @(
        'extractor\native\recorder_host\src\main.rs',
        'extractor\native\recorder_host\src\early_render_launch.rs',
        'extractor\native\recorder_host\src\auto_attach.rs',
        'extractor\native\recorder_host\src\auto_attach\product_runtime.rs',
        'extractor\native\collector\src\lib.rs',
        'extractor\native\compact_contract\src\passive_ready.rs',
        'extractor\native\compact_contract\src\build_profile.rs',
        'extractor\native\compact_contract\src\control_transport\payloads.rs',
        'replay_interface\native\replay_app\src\main.rs',
        'replay_interface\native\replay_app\src\recording_open.rs',
        'replay_interface\native\replay_app\src\win32.rs'
    )
    $repoRootFull = [IO.Path]::GetFullPath($repoRoot).TrimEnd('\')
    $productionClosureUnits = @(
        Get-ProductionUnitDependencyClosure -RepoRoot $repoRoot
    )
    $productionLaneRoots = @(Get-CollectorProductionLaneRoots -RepoRoot $repoRoot)
    # Every production unit must have an owning Cargo.lock and the workspace
    # manifest beside it: the capability closure walks up from the unit to the
    # lock to find its workspace root and to resolve inherited dependency
    # paths and locked versions. Those come out of the walk, not out of a list.
    $fixtureManifestInputs = [System.Collections.Generic.List[string]]::new()
    $fixtureSourceRoots = [System.Collections.Generic.List[string]]::new()
    foreach ($closureUnit in $productionClosureUnits) {
        foreach ($manifestInput in @(
            [string]$closureUnit.manifest_path,
            [string]$closureUnit.lock_path,
            [string]$closureUnit.workspace_cargo_path
        )) {
            if ((Test-Path -LiteralPath $manifestInput -PathType Leaf) -and
                -not $fixtureManifestInputs.Contains($manifestInput)) {
                $fixtureManifestInputs.Add($manifestInput)
            }
        }
        $closureUnitRoot = [string]$closureUnit.unit_root
        if (-not $fixtureSourceRoots.Contains($closureUnitRoot)) {
            $fixtureSourceRoots.Add($closureUnitRoot)
        }
        foreach ($dependency in @($closureUnit.dependencies)) {
            # A dependency that is itself a production unit is already a root
            # here, and a registry crate is read out of the real cargo source
            # cache by both the authority and this fixture, so neither is
            # materialised.
            if ($dependency.production_unit -or -not $dependency.path_dependency) {
                continue
            }
            $dependencyRoot = [string]$dependency.root
            if (-not $dependencyRoot.StartsWith(
                    $repoRootFull + '\',
                    [StringComparison]::OrdinalIgnoreCase
                )) {
                continue
            }
            if (-not $fixtureSourceRoots.Contains($dependencyRoot)) {
                $fixtureSourceRoots.Add($dependencyRoot)
            }
            $dependencyManifest = Join-Path $dependencyRoot 'Cargo.toml'
            if ((Test-Path -LiteralPath $dependencyManifest -PathType Leaf) -and
                -not $fixtureManifestInputs.Contains($dependencyManifest)) {
                $fixtureManifestInputs.Add($dependencyManifest)
            }
        }
    }
    if ($fixtureSourceRoots.Count -eq 0 -or $fixtureManifestInputs.Count -eq 0) {
        throw 'self-test fixture derived no production roots from the source authority'
    }
    foreach ($manifestInput in $fixtureManifestInputs) {
        Copy-FixtureInput `
            -Source $manifestInput `
            -Destination (Join-Path $root (
                Get-FixtureRelativePath -RepoRoot $repoRoot -Path $manifestInput
            ))
    }
    $fixtureSourceExtensions = @(Get-ProductionInteractionSourceExtensions)
    $fixtureMaterialisedSources = [System.Collections.Generic.List[string]]::new()
    foreach ($fixtureSourceRoot in $fixtureSourceRoots) {
        foreach ($sourceFile in @(
            Get-ChildItem -LiteralPath $fixtureSourceRoot -Recurse -Force -File
        )) {
            if ($fixtureSourceExtensions -notcontains $sourceFile.Extension) {
                continue
            }
            $relativeSource =
                Get-FixtureRelativePath -RepoRoot $repoRoot -Path $sourceFile.FullName
            $destination = Join-Path $root $relativeSource
            if ($verbatimFixtureSources -ccontains $relativeSource) {
                Copy-FixtureInput -Source $sourceFile.FullName -Destination $destination
            }
            else {
                Write-InertFixtureSource -Destination $destination
            }
            $fixtureMaterialisedSources.Add($relativeSource)
        }
    }
    # A cited file the walk never reaches is the failure that started this: it
    # used to surface as Copy-Item throwing on a path that does not exist, with
    # nothing to say which authority was supposed to read it.
    $unreachedVerbatimSources = @(
        $verbatimFixtureSources |
            Where-Object { $fixtureMaterialisedSources -cnotcontains $_ }
    )
    if ($unreachedVerbatimSources.Count -ne 0) {
        throw (
            'self-test fixture cites a source the production traversal never ' +
            "reaches: $($unreachedVerbatimSources -join ', ')"
        )
    }

    # The rejection loop far below mutates one source per role, and those four
    # are named by hand rather than derived. A name that is not in the fixture
    # used to surface as ReadAllText on a missing path three hours into the run,
    # behind every case above it - which is exactly how the citation of a lib.rs
    # in replay_app, a crate whose root is main.rs and which has never had a
    # lib.rs in any commit, survived. They are resolved and checked here, beside
    # the fixture that has to contain them.
    $sourceRejectionRoles = @(
        [pscustomobject]@{
            Name = 'recorder'
            Path = Join-Path $root 'extractor\native\recorder_host\src\main.rs'
        },
        [pscustomobject]@{
            Name = 'collector'
            Path = Join-Path $root 'extractor\native\collector\src\lib.rs'
        },
        [pscustomobject]@{
            Name = 'replay'
            # main.rs carries no self-UI exception, so the whole roster still
            # has to be rejected in it.
            Path = Join-Path $root 'replay_interface\native\replay_app\src\main.rs'
        },
        [pscustomobject]@{
            Name = 'production-lane'
            Path = Join-Path $root 'extractor\native\lanes\active_command_abilities_live\src\lib.rs'
        }
    )
    $missingSourceRejectionRoles = @(
        $sourceRejectionRoles |
            Where-Object {
                -not (Test-Path -LiteralPath ([string]$_.Path) -PathType Leaf)
            } |
            ForEach-Object { "$($_.Name)=$([string]$_.Path)" }
    )
    if ($missingSourceRejectionRoles.Count -ne 0) {
        throw (
            'self-test source-rejection role names a fixture source that does ' +
            "not exist: $($missingSourceRejectionRoles -join ', ')"
        )
    }
    $fixtureProductionLaneRoots = @(
        Get-CollectorProductionLaneRoots -RepoRoot $root
    )
    $expectedProductionLaneNames = @(
        $productionLaneRoots | ForEach-Object { Split-Path -Leaf $_ } | Sort-Object
    )
    $fixtureProductionLaneNames = @(
        $fixtureProductionLaneRoots |
            ForEach-Object { Split-Path -Leaf $_ } |
            Sort-Object
    )
    if (($expectedProductionLaneNames -join '|') -cne
            ($fixtureProductionLaneNames -join '|') -or
        $fixtureProductionLaneNames -cnotcontains
            'production_render_catalog_capture_contract') {
        throw 'self-test fixture does not reproduce the transitive production lane closure'
    }
    $fixtureCollectorCargoPath =
        Join-Path $root 'extractor\native\collector\Cargo.toml'
    $fixtureCollectorCargoOriginal =
        [IO.File]::ReadAllText($fixtureCollectorCargoPath)
    $activeLaneInline =
        'active_command_abilities_live_lane = { path = "../lanes/active_command_abilities_live" }'
    $activeLaneDotted =
        'active_command_abilities_live_lane.path = "../lanes/active_command_abilities_live"'
    $fixtureCollectorCargoDotted =
        $fixtureCollectorCargoOriginal.Replace($activeLaneInline, $activeLaneDotted)
    if ($fixtureCollectorCargoDotted -ceq $fixtureCollectorCargoOriginal) {
        throw 'self-test fixture could not create dotted production dependency syntax'
    }
    [IO.File]::WriteAllText(
        $fixtureCollectorCargoPath,
        $fixtureCollectorCargoDotted,
        [Text.UTF8Encoding]::new($false)
    )
    try {
        $dependencyAliasInline = @(
            Get-TomlProductionDependencyRecords -Toml @'
[dependencies]
dependencies = { package = "active_command_abilities_live_lane", path = "../lanes/active_command_abilities_live" }
'@
        )
        $dependencyAliasDotted = @(
            Get-TomlProductionDependencyRecords -Toml @'
[dependencies]
dependencies.path = "../lanes/active_command_abilities_live"
'@
        )
        Assert-Case 'dotted dependency path remains in production lane closure' (
            @(
                Get-CollectorProductionLaneRoots -RepoRoot $root |
                    ForEach-Object { Split-Path -Leaf $_ }
            ) -ccontains 'active_command_abilities_live' -and
            $dependencyAliasInline.Count -eq 1 -and
            [string]$dependencyAliasInline[0].name -ceq 'dependencies' -and
            [string]$dependencyAliasInline[0].path -ceq
                '../lanes/active_command_abilities_live' -and
            $dependencyAliasDotted.Count -eq 1 -and
            [string]$dependencyAliasDotted[0].name -ceq 'dependencies' -and
            [string]$dependencyAliasDotted[0].path -ceq
                '../lanes/active_command_abilities_live'
        )
    }
    finally {
        [IO.File]::WriteAllText(
            $fixtureCollectorCargoPath,
            $fixtureCollectorCargoOriginal,
            [Text.UTF8Encoding]::new($false)
        )
    }
    $fixtureWorkspaceCargoPath = Join-Path $root 'extractor\native\Cargo.toml'
    $fixtureWorkspaceCargoOriginal =
        [IO.File]::ReadAllText($fixtureWorkspaceCargoPath)
    $fixtureCollectorCargoWorkspace = $fixtureCollectorCargoOriginal.Replace(
        $activeLaneInline,
        'active_command_abilities_live_lane = { workspace = true }'
    )
    if ($fixtureCollectorCargoWorkspace -ceq $fixtureCollectorCargoOriginal) {
        throw 'self-test fixture could not create workspace production dependency syntax'
    }
    [IO.File]::WriteAllText(
        $fixtureWorkspaceCargoPath,
        ($fixtureWorkspaceCargoOriginal +
            "`n[workspace.dependencies]`n" +
            'active_command_abilities_live_lane = { path = "lanes/active_command_abilities_live" }' +
            "`n"),
        [Text.UTF8Encoding]::new($false)
    )
    [IO.File]::WriteAllText(
        $fixtureCollectorCargoPath,
        $fixtureCollectorCargoWorkspace,
        [Text.UTF8Encoding]::new($false)
    )
    try {
        Assert-Case 'workspace dependency path remains in production lane closure' (
            @(
                Get-CollectorProductionLaneRoots -RepoRoot $root |
                    ForEach-Object { Split-Path -Leaf $_ }
            ) -ccontains 'active_command_abilities_live'
        )
    }
    finally {
        [IO.File]::WriteAllText(
            $fixtureCollectorCargoPath,
            $fixtureCollectorCargoOriginal,
            [Text.UTF8Encoding]::new($false)
        )
        [IO.File]::WriteAllText(
            $fixtureWorkspaceCargoPath,
            $fixtureWorkspaceCargoOriginal,
            [Text.UTF8Encoding]::new($false)
        )
    }
    [IO.File]::WriteAllText(
        (Join-Path $root 'extractor\native\collector\src\attach_log.rs'),
        "// synthetic contract fixture: every production domain has an admitted passive source`n",
        [Text.UTF8Encoding]::new($false)
    )
    Copy-Item `
        -LiteralPath (Join-Path $repoRoot 'extractor\native\domain_registry.json') `
        -Destination (Join-Path $root 'extractor\native\domain_registry.json')
    $fixturePassiveReadyPath =
        Join-Path $root 'extractor\native\compact_contract\src\passive_ready.rs'
    $fixturePassiveReady = Get-Content -LiteralPath $fixturePassiveReadyPath -Raw
    $fixtureRosterDeclaration =
        'pub const PINNED_PRODUCTION_HOOK_SPECS_V1: &[PinnedProductionHookSpecV1] = &['
    $fixtureCountDeclaration =
        'pub const PINNED_PRODUCTION_HOOK_COUNT_V1: u16 = PINNED_PRODUCTION_HOOK_SPECS_V1.len() as u16;'
    $fixtureRosterStart =
        $fixturePassiveReady.IndexOf($fixtureRosterDeclaration, [StringComparison]::Ordinal)
    $fixtureRosterBodyStart = $fixtureRosterStart + $fixtureRosterDeclaration.Length
    $fixtureRosterEnd = if ($fixtureRosterStart -ge 0) {
        $fixturePassiveReady.IndexOf('];', $fixtureRosterBodyStart, [StringComparison]::Ordinal)
    }
    else { -1 }
    if ($fixtureRosterStart -lt 0 -or
        -not $fixturePassiveReady.Contains($fixtureCountDeclaration) -or
        $fixtureRosterEnd -le $fixtureRosterBodyStart) {
        throw 'self-test fixture lacks a bounded slice roster and derived hook count'
    }
    $fixtureRosterBody = $fixturePassiveReady.Substring(
        $fixtureRosterBodyStart,
        $fixtureRosterEnd - $fixtureRosterBodyStart
    )
    $expectedPinnedHookCount = [regex]::Matches(
        $fixtureRosterBody,
        '(?m)^\s*pinned_(?:import|rva)!\('
    ).Count
    if ($expectedPinnedHookCount -le 0 -or $expectedPinnedHookCount -gt [uint16]::MaxValue) {
        throw 'self-test fixture has an invalid mechanically counted hook roster'
    }
    $expectedPinnedHookSourceSha256 =
        (Get-FileHash -LiteralPath $fixturePassiveReadyPath -Algorithm SHA256).Hash.ToLowerInvariant()
    # The attestation binds these exact production paths. Their fixture bodies
    # are inert but content-addressed, so the real helper still validates every
    # path/hash/source-digest relationship.
    [IO.File]::WriteAllText(
        (Join-Path $fixtureScripts 'Invoke-CentralRustValidation.ps1'),
        "throw 'fixture runner must never execute'`n",
        [Text.UTF8Encoding]::new($false)
    )
    [IO.File]::WriteAllText(
        (Join-Path $fixtureScripts 'Start-Hoi4ProductionCaptureSafe.ps1'),
        "throw 'fixture driver must never execute'`n",
        [Text.UTF8Encoding]::new($false)
    )
    Copy-Item `
        -LiteralPath (Join-Path $scriptDir 'verify_speed5_stream.py') `
        -Destination (Join-Path $fixtureScripts 'verify_speed5_stream.py')

    # The accepted suspending-code-mutation inventory is pinned per tree: the
    # repository's own pin names the exact files and call counts of the lanes
    # that patch live code, and this fixture deliberately does not carry those
    # sources. So the fixture measures its own inventory with the authority's
    # own functions and writes it as the fixture's pin. On its own that would be
    # a gate that cannot fail, so it is bracketed: the case below pins that the
    # fixture reaches exactly the units the committed repository pin says
    # delegate, with the digest that pin accepted, and a later case drives the
    # builder against a fixture pin that has drifted and requires a rejection.
    $fixtureSuspendingBaselinePath =
        Get-ProductionSuspendingCodeMutationBaselinePath -RepoRoot $root
    $fixtureSuspendingBaseline = Get-ProductionSuspendingCodeMutationBaseline `
        -Findings (Get-ProductionSuspendingCodeMutationFindings `
            -RepoRoot $root `
            -Authority (Get-ProductionInteractionSourceTreeAuthority -RepoRoot $root))
    [IO.File]::WriteAllText(
        $fixtureSuspendingBaselinePath,
        ($fixtureSuspendingBaseline | ConvertTo-Json -Depth 8),
        [Text.UTF8Encoding]::new($false)
    )
    $repoSuspendingBaseline = Read-ProductionSuspendingCodeMutationBaseline `
        -Path (Get-ProductionSuspendingCodeMutationBaselinePath -RepoRoot $repoRoot)
    $repoDelegatingUnits = @(
        $repoSuspendingBaseline.units |
            Where-Object { [bool]$_.delegates_suspending_code_mutation } |
            ForEach-Object { [string]$_.unit } |
            Sort-Object
    )
    $repoAcceptedPatcherDigests = @(
        $repoSuspendingBaseline.units |
            ForEach-Object { @($_.delegated_patchers) } |
            Where-Object { [string]$_.package -ceq 'minhook' } |
            ForEach-Object { [string]$_.capability_digest } |
            Select-Object -Unique
    )
    $fixtureBaselineUnits = @($fixtureSuspendingBaseline.units)
    $fixtureBaselineUnitNames = @(
        $fixtureBaselineUnits | ForEach-Object { [string]$_.unit } | Sort-Object
    )
    Assert-Case 'fixture reaches the exact accepted delegating unit closure' (
        $repoDelegatingUnits.Count -gt 0 -and
        $repoAcceptedPatcherDigests.Count -eq 1 -and
        ($fixtureBaselineUnitNames -join '|') -ceq ($repoDelegatingUnits -join '|') -and
        @($fixtureBaselineUnits | Where-Object {
            [bool]$_.performs_suspending_code_mutation -or
            -not [bool]$_.delegates_suspending_code_mutation -or
            @($_.direct_evidence).Count -ne 0 -or
            @($_.delegated_patchers).Count -ne 1 -or
            [string]@($_.delegated_patchers)[0].package -cne 'minhook' -or
            [string]@($_.delegated_patchers)[0].capability_digest -cne
                $repoAcceptedPatcherDigests[0]
        }).Count -eq 0
    )

    $fixtureBuilder = Join-Path $fixtureScripts 'New-PortableProductPackage.ps1'
    $fixtureVerifier = Join-Path $fixtureScripts 'Test-PortableProductPackageReceipt.ps1'
    $fixtureHelper = Join-Path $fixtureScripts 'ProductionArtifact.Attestation.ps1'
    . $fixtureHelper
    . (Join-Path $fixtureScripts 'PassiveCapture.Lifecycle.ps1')
    $freshness = Clear-ProductionArtifactOutputs -RepoRoot $root

    $recorder = Join-Path $releaseRoot 'collector_record.exe'
    $replay = Join-Path $releaseRoot 'replay_app.exe'
    $collector = Join-Path $releaseRoot 'collector.dll'
    Write-Bytes $recorder (New-MinimalAmd64Pe -Dll $false -Gui $true -Marker 1 -EmbeddedText @(
        'BATTLE_PLAN_OBSERVER_ONLY_RECORDER_V1',
        'BATTLE_PLAN_NO_SEMANTIC_ENGINE_STATE_WRITES_V1'
    ))
    Write-Bytes $replay (New-MinimalAmd64Pe -Dll $false -Gui $true -Marker 2)
    Write-Bytes $collector (New-MinimalAmd64Pe -Dll $true -Gui $false -Marker 3 -EmbeddedText @(
        'BATTLE_PLAN_NO_SEMANTIC_ENGINE_STATE_WRITES_V1'
    ))
    foreach ($name in @('fold_check.exe', 'compact_replay_gate.exe', 'pack_replay.exe')) {
        Write-Bytes (Join-Path $releaseRoot $name) ([Text.Encoding]::ASCII.GetBytes("fixture-$name"))
    }

    $zip = Join-Path $releaseRoot 'battle-plan-0.1.0-windows-x86_64.zip'
    $builderResult = Invoke-JsonScript -Path $fixtureBuilder -Arguments @(
        '-RecorderExe', $recorder,
        '-ReplayExe', $replay,
        '-CollectorDll', $collector,
        '-OutputZip', $zip,
        '-Version', '0.1.0'
    )
    Assert-Case 'builder emits exact schema' ([string]$builderResult.schema -ceq 'hoi4-portable-package.v1')
    Assert-Case 'builder emits exact package path' ([IO.Path]::GetFullPath([string]$builderResult.path) -ieq [IO.Path]::GetFullPath($zip))
    Assert-Case 'builder emits two GUI executables' ([int]$builderResult.executable_count -eq 2)
    Assert-Case 'builder without real capture evidence is explicitly non-production' (
        $builderResult.admissible_for_production -is [bool] -and
        -not $builderResult.admissible_for_production -and
        $null -eq $builderResult.production_capture_evidence
    )
    Assert-Case 'builder reverified passive interaction closure' (
        [string]$builderResult.interaction_closure.policy -ceq 'passive_injected_observer' -and
        $builderResult.interaction_closure.zip_entry_bytes_reverified -is [bool] -and
        $builderResult.interaction_closure.zip_entry_bytes_reverified -and
        [string]$builderResult.interaction_closure.policy_inputs.read_only_game_state_witness_sha256 -cmatch '^[0-9a-f]{64}$' -and
        [string]$builderResult.interaction_closure.policy_inputs.passive_capture_policy_test_sha256 -cmatch '^[0-9a-f]{64}$' -and
        [string]$builderResult.interaction_closure.policy_inputs.passive_capture_lifecycle_sha256 -cmatch '^[0-9a-f]{64}$' -and
        [string]$builderResult.interaction_closure.policy_inputs.passive_ready_contract_sha256 -cmatch '^[0-9a-f]{64}$' -and
        [string]$builderResult.interaction_closure.source_capability.schema -ceq
            'hoi4.production-source-capability-closure.v1' -and
        $builderResult.interaction_closure.source_capability.canonical_observer_feature_isolated -and
        $builderResult.interaction_closure.source_capability.target_lifecycle_actuators_evidence_only -and
        $builderResult.interaction_closure.source_capability.collector_fixture_hooks_default_off -and
        $builderResult.interaction_closure.source_capability.structural_dependency_feature_parse -and
        $builderResult.interaction_closure.source_capability.production_source_tree_closed -and
        $builderResult.interaction_closure.source_capability.canonical_hook_name_families_closed -and
        [int]$builderResult.interaction_closure.source_capability.exact_pinned_hook_count -eq
            $expectedPinnedHookCount -and
        [string]$builderResult.interaction_closure.source_capability.pinned_hook_contract_source_sha256 -ceq
            $expectedPinnedHookSourceSha256 -and
        [string]$builderResult.interaction_closure.recorder_host.schema -ceq 'hoi4.interaction-closure.v1' -and
        [string]$builderResult.interaction_closure.replay_app.schema -ceq 'hoi4.interaction-closure.v1' -and
        [string]$builderResult.interaction_closure.collector_dll.schema -ceq 'hoi4.interaction-closure.v1' -and
        [int]$builderResult.interaction_closure.recorder_host.exact_pinned_hook_count -eq
            $expectedPinnedHookCount -and
        [int]$builderResult.interaction_closure.collector_dll.exact_pinned_hook_count -eq
            $expectedPinnedHookCount -and
        (@($builderResult.interaction_closure.recorder_host.permitted_self_ui_capabilities) -join '|') -ceq
            'SendMessageW|ShellExecuteW|EnableWindow|SetWindowTextW|ShowWindow|DestroyWindow' -and
        (@($builderResult.interaction_closure.replay_app.permitted_self_ui_capabilities) -join '|') -ceq
            'SetWindowTextW|DestroyWindow' -and
        @($builderResult.interaction_closure.collector_dll.permitted_self_ui_capabilities).Count -eq 0 -and
        (@($builderResult.interaction_closure.replay_app.permitted_self_worker_capabilities) -join '|') -ceq
            'CreateProcessW|TerminateProcess|CreateJobObjectW|AssignProcessToJobObject|SetInformationJobObject' -and
        @($builderResult.interaction_closure.recorder_host.permitted_self_worker_capabilities).Count -eq 0 -and
        @($builderResult.interaction_closure.collector_dll.permitted_self_worker_capabilities).Count -eq 0 -and
        $builderResult.interaction_closure.replay_app.forbidden_symbols_absent -and
        (@($builderResult.interaction_closure.recorder_host.permitted_injection_capabilities) -join '|') -ceq
            'OpenProcess|VirtualAllocEx|VirtualFreeEx|WriteProcessMemory|CreateRemoteThread|LoadLibraryW|FreeLibrary'
    )

    # The fixture's accepted inventory is measured, so on its own it can only
    # agree with itself. This drives the drift gate the other way: one accepted
    # unit is taken out of the pin while the tree still reaches it, which is
    # exactly the shape of "a unit started suspending and nobody accepted it".
    $fixtureBaselineOriginal = [IO.File]::ReadAllText($fixtureSuspendingBaselinePath)
    $driftedBaseline = $fixtureBaselineOriginal | ConvertFrom-Json
    $driftedBaseline.units = @(
        @($driftedBaseline.units) | Select-Object -Skip 1
    )
    if (@($driftedBaseline.units).Count -ne ($fixtureBaselineUnits.Count - 1)) {
        throw 'self-test could not drift the fixture suspending-code-mutation pin'
    }
    [IO.File]::WriteAllText(
        $fixtureSuspendingBaselinePath,
        ($driftedBaseline | ConvertTo-Json -Depth 8),
        [Text.UTF8Encoding]::new($false)
    )
    try {
        Invoke-ExpectFailure 'accepted suspending-code-mutation drift is rejected' {
            Invoke-JsonScript -Path $fixtureBuilder -Arguments @(
                '-RecorderExe', $recorder,
                '-ReplayExe', $replay,
                '-CollectorDll', $collector,
                '-OutputZip', (Join-Path $releaseRoot 'drifted-suspending-baseline.zip'),
                '-Version', '0.1.0'
            ) | Out-Null
        }
    }
    finally {
        [IO.File]::WriteAllText(
            $fixtureSuspendingBaselinePath,
            $fixtureBaselineOriginal,
            [Text.UTF8Encoding]::new($false)
        )
    }

    $evidenceRecorder = Join-Path $releaseRoot 'collector_record_evidence_marker.exe'
    $evidenceRecorderBytes = [byte[]][IO.File]::ReadAllBytes($recorder).Clone()
    $evidenceMarker = [Text.Encoding]::ASCII.GetBytes('BATTLE_PLAN_CLIENT_DRIVING_EVIDENCE_V1')
    [Array]::Copy($evidenceMarker, 0, $evidenceRecorderBytes, 0x300, $evidenceMarker.Length)
    Write-Bytes $evidenceRecorder $evidenceRecorderBytes
    Invoke-ExpectFailure 'evidence recorder marker is rejected' {
        Invoke-JsonScript -Path $fixtureBuilder -Arguments @(
            '-RecorderExe', $evidenceRecorder,
            '-ReplayExe', $replay,
            '-CollectorDll', $collector,
            '-OutputZip', (Join-Path $releaseRoot 'forbidden-evidence-recorder.zip'),
            '-Version', '0.1.0'
        ) | Out-Null
    }

    $suspendedLaunchRecorder = Join-Path $releaseRoot 'collector_record_suspended_launch_marker.exe'
    $suspendedLaunchBytes = [byte[]][IO.File]::ReadAllBytes($recorder).Clone()
    $suspendedLaunchMarker = [Text.Encoding]::ASCII.GetBytes('CREATE_SUSPENDED')
    [Array]::Copy(
        $suspendedLaunchMarker,
        0,
        $suspendedLaunchBytes,
        0x340,
        $suspendedLaunchMarker.Length
    )
    Write-Bytes $suspendedLaunchRecorder $suspendedLaunchBytes
    Invoke-ExpectFailure 'target suspended-launch marker is rejected' {
        Invoke-JsonScript -Path $fixtureBuilder -Arguments @(
            '-RecorderExe', $suspendedLaunchRecorder,
            '-ReplayExe', $replay,
            '-CollectorDll', $collector,
            '-OutputZip', (Join-Path $releaseRoot 'forbidden-suspended-launch-recorder.zip'),
            '-Version', '0.1.0'
        ) | Out-Null
    }

    $targetLifecycleRecorder = Join-Path $releaseRoot 'collector_record_target_lifecycle_marker.exe'
    $targetLifecycleBytes = [byte[]][IO.File]::ReadAllBytes($recorder).Clone()
    $targetLifecycleMarker = [Text.Encoding]::ASCII.GetBytes('early_render_capture_receipt')
    [Array]::Copy($targetLifecycleMarker, 0, $targetLifecycleBytes, 0x380, $targetLifecycleMarker.Length)
    Write-Bytes $targetLifecycleRecorder $targetLifecycleBytes
    Invoke-ExpectFailure 'target early-render lifecycle marker is rejected' {
        Invoke-JsonScript -Path $fixtureBuilder -Arguments @(
            '-RecorderExe', $targetLifecycleRecorder,
            '-ReplayExe', $replay,
            '-CollectorDll', $collector,
            '-OutputZip', (Join-Path $releaseRoot 'forbidden-target-lifecycle-recorder.zip'),
            '-Version', '0.1.0'
        ) | Out-Null
    }

    $evidenceCollector = Join-Path $releaseRoot 'collector_evidence_marker.dll'
    $evidenceCollectorBytes = [byte[]][IO.File]::ReadAllBytes($collector).Clone()
    [Array]::Copy($evidenceMarker, 0, $evidenceCollectorBytes, 0x300, $evidenceMarker.Length)
    Write-Bytes $evidenceCollector $evidenceCollectorBytes
    Invoke-ExpectFailure 'evidence collector marker is rejected' {
        Invoke-JsonScript -Path $fixtureBuilder -Arguments @(
            '-RecorderExe', $recorder,
            '-ReplayExe', $replay,
            '-CollectorDll', $evidenceCollector,
            '-OutputZip', (Join-Path $releaseRoot 'forbidden-evidence-collector.zip'),
            '-Version', '0.1.0'
        ) | Out-Null
    }

    # Repinned deliberately, three times. The roster gained the thread-handle
    # acquisition and consumption surface it was missing: the Zw
    # suspend/resume/terminate aliases, Nt/Zw SetContextThread, the alert and
    # alert-resume pairs, Nt/Zw SetInformationThread and ImpersonateThread, the
    # QueueUserAPC2 and QueueApcThreadEx2 forms, SetThreadToken,
    # SetThreadInformation, CancelSynchronousIo, the Nt/Zw OpenThread pair, and
    # the scheduling setters. NtOpenThread in particular meant the ban on
    # obtaining another thread's handle had never been in force at all, since
    # only the Win32 spelling was listed.
    #
    # Each of three review rounds found more of this surface, so expect this
    # census to move again. It is a census, not a proof that the thread-control
    # surface is closed. Two files pin it and both have to move together.
    $fixedForbiddenMarkerCount = 147
    $fixedForbiddenMarkerRosterSha256 =
        '3068b0722bf931718c471cbeb998998b05ff9cca2ae72a4ecb2ce8a9a8bcf617'
    $forbiddenMarkerRoster = @(Get-ForbiddenProductionInteractionMarkers)
    Assert-Case 'forbidden interaction marker roster is the fixed reviewed census' (
        $forbiddenMarkerRoster.Count -eq $fixedForbiddenMarkerCount -and
        (Get-ProductionInteractionMarkerRosterSha256) -ceq
            $fixedForbiddenMarkerRosterSha256
    )
    foreach ($role in @('recorder', 'replay', 'collector')) {
        $allRejected = $true
        $roleForbiddenMarkerRoster = if ($role -ceq 'recorder') {
            @($forbiddenMarkerRoster | Where-Object {
                $_ -cnotin @(
                    'SendMessageW',
                    'ShellExecuteW',
                    'EnableWindow',
                    'SetWindowTextW',
                    'ShowWindow',
                    'DestroyWindow'
                )
            })
        }
        elseif ($role -ceq 'replay') {
            @($forbiddenMarkerRoster | Where-Object {
                $_ -cnotin @(
                    'SetWindowTextW',
                    'DestroyWindow',
                    'CreateProcessW',
                    'TerminateProcess',
                    'CreateJobObjectW',
                    'AssignProcessToJobObject',
                    'SetInformationJobObject'
                )
            })
        }
        else {
            @($forbiddenMarkerRoster)
        }
        foreach ($symbol in $roleForbiddenMarkerRoster) {
            $baseBytes = switch ($role) {
                'recorder' { [IO.File]::ReadAllBytes($recorder) }
                'replay' { [IO.File]::ReadAllBytes($replay) }
                'collector' { [IO.File]::ReadAllBytes($collector) }
                default { throw "unexpected binary interaction-closure role: $role" }
            }
            $mutatedBytes = [byte[]]$baseBytes.Clone()
            $marker = [Text.Encoding]::ASCII.GetBytes($symbol)
            [Array]::Copy($marker, 0, $mutatedBytes, 0x300, $marker.Length)
            $mutatedPath = Join-Path $releaseRoot ("forbidden-{0}-{1}.bin" -f $role, $symbol)
            Write-Bytes $mutatedPath $mutatedBytes
            $rejected = $false
            try {
                Invoke-JsonScript -Path $fixtureBuilder -Arguments @(
                    '-RecorderExe', $(if ($role -ceq 'recorder') { $mutatedPath } else { $recorder }),
                    '-ReplayExe', $(if ($role -ceq 'replay') { $mutatedPath } else { $replay }),
                    '-CollectorDll', $(if ($role -ceq 'collector') { $mutatedPath } else { $collector }),
                    '-OutputZip', (Join-Path $releaseRoot ("forbidden-{0}-{1}.zip" -f $role, $symbol)),
                    '-Version', '0.1.0'
                ) | Out-Null
            }
            catch {
                $rejected = $true
            }
            if (-not $rejected) {
                $allRejected = $false
                break
            }
        }
        Assert-Case "$role binary rejects the complete lifecycle/UI API roster" $allRejected
    }

    $fixtureCollectorCargo =
        Join-Path $root 'extractor\native\collector\Cargo.toml'
    $collectorCargoOriginal = [IO.File]::ReadAllText($fixtureCollectorCargo)
    $cargoMutations = [ordered]@{
        'multiline character dependency features are rejected' =
            $collectorCargoOriginal.Replace(
                'character_presentation_live_lane = { path = "../lanes/character_presentation_live" }',
                @'
character_presentation_live_lane = {
    path = "../lanes/character_presentation_live",
    features = [
        "a729-fixture-hook",
    ],
}
'@
            )
        'multiline focus dependency table features are rejected' =
            $collectorCargoOriginal.Replace(
                'focus_research_decisions_depth_live = { path = "../lanes/focus_research_decisions_depth_live" }',
                @'
[dependencies.focus_research_decisions_depth_live]
path = "../lanes/focus_research_decisions_depth_live"
features = [
    "a729-focus-fixture-hook",
]
[dependencies]
'@
            )
        'multiline aliased fixture feature is rejected' =
            $collectorCargoOriginal.Replace(
                "[target.'cfg(windows)'.dependencies]",
                @'
hidden_fixture_alias = {
    package = "runtime_text_inputs_live_lane",
    path = "../lanes/runtime_text_inputs_live",
    features = [
        "a729-fixture-hook",
    ],
}
[target.'cfg(windows)'.dependencies]
'@
            )
        'escaped aliased fixture feature is rejected' =
            $collectorCargoOriginal.Replace(
                "[target.'cfg(windows)'.dependencies]",
                @'
escaped_fixture_alias = {
    package = "runtime_text_inputs_live_lane",
    path = "../lanes/runtime_text_inputs_live",
    features = [
        "a729\u002dfixture\u002dhook",
    ],
}
[target.'cfg(windows)'.dependencies]
'@
            )
        'escaped inline package alias is rejected' =
            $collectorCargoOriginal.Replace(
                "[target.'cfg(windows)'.dependencies]",
                @'
escaped_package_alias = {
    package = "character\u005fpresentation\u005flive\u005flane",
    path = "../lanes/character_presentation_live",
    features = [
        "ordinary-looking-feature",
    ],
}
[target.'cfg(windows)'.dependencies]
'@
            )
        'aliased character dependency subtable features are rejected' =
            $collectorCargoOriginal.Replace(
                "[target.'cfg(windows)'.dependencies]",
                @'
[dependencies.character_alias]
package = "character_presentation_live_lane"
path = "../lanes/character_presentation_live"
features = [
    "ordinary-looking-feature",
]
[target.'cfg(windows)'.dependencies]
'@
            )
        'dotted aliased focus dependency features are rejected' =
            $collectorCargoOriginal.Replace(
                "[target.'cfg(windows)'.dependencies]",
                @'
focus_alias.package = "focus_research_decisions_depth_live"
focus_alias.path = "../lanes/focus_research_decisions_depth_live"
focus_alias.features = [
    "ordinary-looking-feature",
]
[target.'cfg(windows)'.dependencies]
'@
            )
        'mixed dotted and subtable aliases are one dependency record' =
            $collectorCargoOriginal.Replace(
                "[target.'cfg(windows)'.dependencies]",
                @'
mixed_alias.package = "character_presentation_live_lane"
mixed_alias.path = "../lanes/character_presentation_live"
[dependencies.mixed_alias]
features = [
    "ordinary-looking-feature",
]
[target.'cfg(windows)'.dependencies]
'@
            )
        'features preceding package retain alias state' =
            $collectorCargoOriginal.Replace(
                "[target.'cfg(windows)'.dependencies]",
                @'
[dependencies.stateful_alias]
features = [
    "ordinary-looking-feature",
]
package = "character_presentation_live_lane"
path = "../lanes/character_presentation_live"
[target.'cfg(windows)'.dependencies]
'@
            )
        'features preceding escaped subtable package are rejected' =
            $collectorCargoOriginal.Replace(
                "[target.'cfg(windows)'.dependencies]",
                @'
[dependencies.escaped_stateful_alias]
features = [
    "ordinary-looking-feature",
]
package = "focus\u005fresearch\u005fdecisions\u005fdepth\u005flive"
path = "../lanes/focus_research_decisions_depth_live"
[target.'cfg(windows)'.dependencies]
'@
            )
        'quoted dotted dependency alias is rejected' =
            $collectorCargoOriginal.Replace(
                "[target.'cfg(windows)'.dependencies]",
                @'
[dependencies]
"quoted.alias".package = "focus_research_decisions_depth_live"
"quoted.alias".path = "../lanes/focus_research_decisions_depth_live"
"quoted.alias".features = [
    "ordinary-looking-feature",
]
[target.'cfg(windows)'.dependencies]
'@
            )
        'target dependency subtable retains quoted dotted alias state' =
            $collectorCargoOriginal.Replace(
                "[target.'cfg(windows)'.dependencies]",
                @'
[target.'cfg(windows)'.dependencies."target.alias"]
features = [
    "ordinary-looking-feature",
]
package = "character_presentation_live_lane"
path = "../lanes/character_presentation_live"
[target.'cfg(windows)'.dependencies]
'@
            )
    }
    foreach ($mutation in $cargoMutations.GetEnumerator()) {
        if ([string]$mutation.Value -ceq $collectorCargoOriginal) {
            throw "portable package self-test could not apply Cargo mutation: $($mutation.Key)"
        }
        [IO.File]::WriteAllText(
            $fixtureCollectorCargo,
            [string]$mutation.Value,
            [Text.UTF8Encoding]::new($false)
        )
        try {
            Invoke-ExpectFailure ([string]$mutation.Key) {
                Invoke-JsonScript -Path $fixtureBuilder -Arguments @(
                    '-RecorderExe', $recorder,
                    '-ReplayExe', $replay,
                    '-CollectorDll', $collector,
                    '-OutputZip', (Join-Path $releaseRoot ("forbidden-cargo-{0}.zip" -f
                        ([guid]::NewGuid().ToString('N')))),
                    '-Version', '0.1.0'
                ) | Out-Null
            }
        }
        finally {
            [IO.File]::WriteAllText(
                $fixtureCollectorCargo,
                $collectorCargoOriginal,
                [Text.UTF8Encoding]::new($false)
            )
        }
    }

    $fixtureWorkspaceCargo = Join-Path $root 'extractor\native\Cargo.toml'
    $workspaceCargoOriginal = [IO.File]::ReadAllText($fixtureWorkspaceCargo)
    $workspaceInheritedCargo = $collectorCargoOriginal.Replace(
        "[target.'cfg(windows)'.dependencies]",
        @'
workspace_character_alias = {
    workspace = true,
    features = [
        "ordinary-looking-feature",
    ],
}
[target.'cfg(windows)'.dependencies]
'@
    )
    $workspaceCargoMutation = $workspaceCargoOriginal + @'

[workspace.dependencies]
workspace_character_alias = {
    package = "character_presentation_live_lane",
    path = "lanes/character_presentation_live",
}
'@
    [IO.File]::WriteAllText(
        $fixtureCollectorCargo,
        $workspaceInheritedCargo,
        [Text.UTF8Encoding]::new($false)
    )
    [IO.File]::WriteAllText(
        $fixtureWorkspaceCargo,
        $workspaceCargoMutation,
        [Text.UTF8Encoding]::new($false)
    )
    try {
        Invoke-ExpectFailure 'workspace-inherited protected alias features are rejected' {
            Invoke-JsonScript -Path $fixtureBuilder -Arguments @(
                '-RecorderExe', $recorder,
                '-ReplayExe', $replay,
                '-CollectorDll', $collector,
                '-OutputZip', (Join-Path $releaseRoot 'forbidden-workspace-inherited-feature.zip'),
                '-Version', '0.1.0'
            ) | Out-Null
        }
    }
    finally {
        [IO.File]::WriteAllText(
            $fixtureCollectorCargo,
            $collectorCargoOriginal,
            [Text.UTF8Encoding]::new($false)
        )
        [IO.File]::WriteAllText(
            $fixtureWorkspaceCargo,
            $workspaceCargoOriginal,
            [Text.UTF8Encoding]::new($false)
        )
    }

    $workspaceAdditiveCollectorCargo = $collectorCargoOriginal.Replace(
        "[target.'cfg(windows)'.dependencies]",
        @'
workspace_additive_alias = {
    workspace = true,
    features = [
        "ordinary-looking-feature",
    ],
}
[target.'cfg(windows)'.dependencies]
'@
    )
    $workspaceAdditiveRootCargo = $workspaceCargoOriginal + @'

[workspace.dependencies]
workspace_additive_alias = {
    package = "runtime_text_inputs_live_lane",
    path = "lanes/runtime_text_inputs_live",
    features = [
        "a729-fixture-hook",
    ],
}
'@
    [IO.File]::WriteAllText(
        $fixtureCollectorCargo,
        $workspaceAdditiveCollectorCargo,
        [Text.UTF8Encoding]::new($false)
    )
    [IO.File]::WriteAllText(
        $fixtureWorkspaceCargo,
        $workspaceAdditiveRootCargo,
        [Text.UTF8Encoding]::new($false)
    )
    try {
        Invoke-ExpectFailure 'workspace and local dependency features are scanned additively' {
            Invoke-JsonScript -Path $fixtureBuilder -Arguments @(
                '-RecorderExe', $recorder,
                '-ReplayExe', $replay,
                '-CollectorDll', $collector,
                '-OutputZip', (Join-Path $releaseRoot 'forbidden-workspace-additive-feature.zip'),
                '-Version', '0.1.0'
            ) | Out-Null
        }
    }
    finally {
        [IO.File]::WriteAllText(
            $fixtureCollectorCargo,
            $collectorCargoOriginal,
            [Text.UTF8Encoding]::new($false)
        )
        [IO.File]::WriteAllText(
            $fixtureWorkspaceCargo,
            $workspaceCargoOriginal,
            [Text.UTF8Encoding]::new($false)
        )
    }

    $fixtureCharacterCargo =
        Join-Path $root 'extractor\native\lanes\character_presentation_live\Cargo.toml'
    $characterCargoOriginal = [IO.File]::ReadAllText($fixtureCharacterCargo)
    $characterCargoDefaultMutation = $characterCargoOriginal.Replace(
        'default = []',
        'default = ["a729-fixture-hook"]'
    )
    if ($characterCargoDefaultMutation -ceq $characterCargoOriginal) {
        throw 'portable package self-test could not mutate protected lane defaults'
    }
    [IO.File]::WriteAllText(
        $fixtureCharacterCargo,
        $characterCargoDefaultMutation,
        [Text.UTF8Encoding]::new($false)
    )
    try {
        Invoke-ExpectFailure 'protected lane forbidden hook cannot enter default features' {
            Invoke-JsonScript -Path $fixtureBuilder -Arguments @(
                '-RecorderExe', $recorder,
                '-ReplayExe', $replay,
                '-CollectorDll', $collector,
                '-OutputZip', (Join-Path $releaseRoot 'forbidden-lane-default-feature.zip'),
                '-Version', '0.1.0'
            ) | Out-Null
        }
    }
    finally {
        [IO.File]::WriteAllText(
            $fixtureCharacterCargo,
            $characterCargoOriginal,
            [Text.UTF8Encoding]::new($false)
        )
    }

    foreach ($sourceRole in $sourceRejectionRoles) {
        $sourceOriginal = [IO.File]::ReadAllText([string]$sourceRole.Path)
        $allSourceCapabilitiesRejected = $true
        foreach ($symbol in @(Get-ForbiddenProductionClientCapabilitySymbols)) {
            [IO.File]::WriteAllText(
                [string]$sourceRole.Path,
                ($sourceOriginal + "`nunsafe extern `"system`" { fn $symbol(); }`n"),
                [Text.UTF8Encoding]::new($false)
            )
            $rejected = $false
            try {
                Invoke-JsonScript -Path $fixtureBuilder -Arguments @(
                    '-RecorderExe', $recorder,
                    '-ReplayExe', $replay,
                    '-CollectorDll', $collector,
                    '-OutputZip', (Join-Path $releaseRoot ("forbidden-source-{0}-{1}.zip" -f
                        $sourceRole.Name, $symbol)),
                    '-Version', '0.1.0'
                ) | Out-Null
            }
            catch {
                $rejected = $true
            }
            finally {
                [IO.File]::WriteAllText(
                    [string]$sourceRole.Path,
                    $sourceOriginal,
                    [Text.UTF8Encoding]::new($false)
                )
            }
            if (-not $rejected) {
                $allSourceCapabilitiesRejected = $false
                break
            }
        }
        Assert-Case "$($sourceRole.Name) source rejects the complete lifecycle/UI API roster" (
            $allSourceCapabilitiesRejected
        )
    }
    $allProductionLaneRootsScanned = $true
    foreach ($fixtureLaneRoot in $fixtureProductionLaneRoots) {
        $fixtureLaneLib = Join-Path $fixtureLaneRoot 'src\lib.rs'
        $fixtureLaneOriginal = [IO.File]::ReadAllText($fixtureLaneLib)
        [IO.File]::WriteAllText(
            $fixtureLaneLib,
            ($fixtureLaneOriginal +
                "`nunsafe extern `"system`" { fn SuspendThread(); }`n"),
            [Text.UTF8Encoding]::new($false)
        )
        $rejected = $false
        try {
            Invoke-JsonScript -Path $fixtureBuilder -Arguments @(
                '-RecorderExe', $recorder,
                '-ReplayExe', $replay,
                '-CollectorDll', $collector,
                '-OutputZip', (Join-Path $releaseRoot (
                    "forbidden-lane-root-{0}.zip" -f (Split-Path -Leaf $fixtureLaneRoot)
                )),
                '-Version', '0.1.0'
            ) | Out-Null
        }
        catch {
            $rejected = $true
        }
        finally {
            [IO.File]::WriteAllText(
                $fixtureLaneLib,
                $fixtureLaneOriginal,
                [Text.UTF8Encoding]::new($false)
            )
        }
        if (-not $rejected) {
            $allProductionLaneRootsScanned = $false
            break
        }
    }
    Assert-Case 'every direct and transitive production lane root is source-scanned' (
        $allProductionLaneRootsScanned
    )
    $fixtureShadowEarlyRender = Join-Path (
        $fixtureProductionLaneRoots |
            Where-Object { (Split-Path -Leaf $_) -ceq 'active_command_abilities_live' } |
            Select-Object -First 1
    ) 'src\early_render_launch.RS'
    [IO.File]::WriteAllText(
        $fixtureShadowEarlyRender,
        @'
#[cfg(all(windows, feature = "evidence-client-driving"))]
unsafe extern "system" { fn SuspendThread(); }
#[cfg(all(windows, not(feature = "evidence-client-driving")))]
pub fn observer_only() {}
'@,
        [Text.UTF8Encoding]::new($false)
    )
    try {
        Invoke-ExpectFailure 'evidence interval exclusion is exact recorder path only' {
            Invoke-JsonScript -Path $fixtureBuilder -Arguments @(
                '-RecorderExe', $recorder,
                '-ReplayExe', $replay,
                '-CollectorDll', $collector,
                '-OutputZip', (Join-Path $releaseRoot 'forbidden-shadow-early-render.zip'),
                '-Version', '0.1.0'
            ) | Out-Null
        }
    }
    finally {
        Remove-Item -LiteralPath $fixtureShadowEarlyRender -Force
    }
    # This used to append `use minhook::MinHook; ... MinHook::enable_hook(..)`
    # to a lane that already declares minhook and require a rejection. It could
    # never get one, and it had never been run: the authority binds a unit to a
    # delegated patcher by the MANIFEST EDGE, not by the callsite text - by
    # design, because a re-export lets a crate use a patcher without ever
    # writing its name - and that lane's edge was already in the accepted
    # inventory, so the mutation changed nothing the gate keys on.
    #
    # The mutation now has the shape the authority actually enforces: a lane
    # with no minhook edge takes one on, which is a new delegated patcher and
    # has to be accepted again. air_loss_stats_live is in the production
    # closure and declares no minhook today; its owning lock resolves minhook
    # 0.9.0, so the classification reaches the same registry source as the
    # lanes that do.
    #
    # Limit worth stating: a lane that reaches minhook only through another
    # in-repo crate's re-export still has no minhook edge of its own, and a
    # pure re-exporter has no suspend/mutate site of its own to be classified
    # on. That path is covered neither here nor by the authority.
    $fixtureProductionLane =
        Join-Path $root 'extractor\native\lanes\air_loss_stats_live\Cargo.toml'
    $fixtureProductionLaneOriginal = [IO.File]::ReadAllText($fixtureProductionLane)
    if ($fixtureProductionLaneOriginal.Contains('minhook')) {
        throw 'self-test lane already declares the suspending primitive it plants'
    }
    [IO.File]::WriteAllText(
        $fixtureProductionLane,
        ($fixtureProductionLaneOriginal +
            "`n[target.'cfg(windows)'.dependencies]`nminhook = `"0.9.0`"`n"),
        [Text.UTF8Encoding]::new($false)
    )
    try {
        Invoke-ExpectFailure 'production-linked lane cannot take on the suspending MinHook primitive' {
            Invoke-JsonScript -Path $fixtureBuilder -Arguments @(
                '-RecorderExe', $recorder,
                '-ReplayExe', $replay,
                '-CollectorDll', $collector,
                '-OutputZip', (Join-Path $releaseRoot 'forbidden-lane-minhook.zip'),
                '-Version', '0.1.0'
            ) | Out-Null
        }
    }
    finally {
        [IO.File]::WriteAllText(
            $fixtureProductionLane,
            $fixtureProductionLaneOriginal,
            [Text.UTF8Encoding]::new($false)
        )
    }

    $hexA = 'a' * 64
    $hexB = 'b' * 64
    $hexC = 'c' * 64
    $commitHash = 'd' * 40
    $buildEnvValues = [ordered]@{}
    foreach ($key in $script:ProductionBuildEnvironmentKeys) { $buildEnvValues[$key] = '' }
    $buildEnvValues.CARGO_INCREMENTAL = '0'
    $buildEnvValues.CARGO_TARGET_DIR = Join-Path $root '.cargo-target'
    $buildEnvValues.CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER = 'synthetic-lld-link.exe'
    $buildEnvValues.PATH_SHA256 = $hexC
    $buildEnvironment = New-ProductionBuildEnvironmentRecord `
        -RustcVerbose "rustc 1.96.0`nhost: x86_64-pc-windows-msvc`nrelease: 1.96.0`ncommit-hash: $commitHash" `
        -CargoVersion 'cargo 1.96.0' `
        -Linker ([pscustomobject][ordered]@{
            configured = 'synthetic-lld-link.exe'; version_output = 'LLD 22.1.0 synthetic'
            executable = [pscustomobject]@{ path = 'C:\synthetic\lld-link.exe'; bytes = 1; sha256 = $hexA; file_version = '22.1'; product_version = '22.1' }
        }) `
        -RustcWrapper ([pscustomobject][ordered]@{ active = $false; configured = ''; executable = $null }) `
        -MsvcToolset ([pscustomobject][ordered]@{
            discovery = 'synthetic'; version = '14.50.00000'; root = 'C:\synthetic\MSVC'
            compiler = [pscustomobject]@{ path = 'C:\synthetic\cl.exe'; bytes = 1; sha256 = $hexB; file_version = '19.50'; product_version = '19.50' }
            msvc_linker = [pscustomobject]@{ path = 'C:\synthetic\link.exe'; bytes = 1; sha256 = $hexC; file_version = '14.50'; product_version = '14.50' }
        }) `
        -WindowsSdk ([pscustomobject][ordered]@{
            root = 'C:\synthetic\Windows Kits\10'; version = '10.0.26100.0'
            windows_sdk_dir_env = ''; windows_sdk_version_env = ''; ucrt_version_env = ''
            kernel32_import_library = [pscustomobject]@{ path = 'C:\synthetic\kernel32.lib'; bytes = 1; sha256 = $hexA; file_version = ''; product_version = '' }
            ucrt_import_library = [pscustomobject]@{ path = 'C:\synthetic\ucrt.lib'; bytes = 1; sha256 = $hexB; file_version = ''; product_version = '' }
        }) `
        -Runner ([pscustomobject][ordered]@{
            image_os = 'synthetic'; image_version = '1'; runner_os = 'Windows'; runner_arch = 'X64'
            runner_environment = 'self-test'; os_description = 'synthetic Windows'
            os_architecture = 'X64'; process_architecture = 'X64'
        }) `
        -BuildEnv $buildEnvValues
    $runtimeLinkage = [pscustomobject][ordered]@{
        target = 'x86_64-pc-windows-msvc'; rust_target_feature = '+crt-static'; c_runtime = 'static'
        admission = 'pe_import_and_delay_import_tables'; forbidden_import = 'VCRUNTIME*.dll'
    }
    $sourceDigest = Get-ProductionRustInputDigest -RepoRoot $root
    $attestationPath = Join-Path $releaseRoot 'hoi4-production-prebuilt-artifacts.json'
    Write-ProductionArtifactAttestation `
        -RepoRoot $root `
        -ReceiptPath $attestationPath `
        -RequestedWorkspace All `
        -SourceDigest $sourceDigest `
        -ReleaseRuntimeLinkage $runtimeLinkage `
        -BuildEnvironment $buildEnvironment `
        -ArtifactFreshness $freshness
    $driver = Join-Path $fixtureScripts 'Start-Hoi4ProductionCaptureSafe.ps1'
    $attested = Test-ProductionArtifactAttestation -RepoRoot $root -ReceiptPath $attestationPath -ExpectedDriverPath $driver

    # Build a small but exact synthetic receipt graph. This does not stand in
    # for live evidence; it exercises the package/promotion binding itself.
    $captureRun = Join-Path $root 'synthetic-production-capture'
    [IO.Directory]::CreateDirectory($captureRun) | Out-Null
    $retainedAttestationPath = Join-Path $captureRun 'prebuilt_artifact_attestation.json'
    Copy-Item -LiteralPath $attestationPath -Destination $retainedAttestationPath
    $registry = Get-Content -LiteralPath (Join-Path $root 'extractor\native\domain_registry.json') -Raw |
        ConvertFrom-Json
    $promotionStatuses = @('existing', 'production_complete', 'production_complete_base')
    $domains = @(
        $registry.domains |
            Where-Object { $_.schedulable -eq $true -and $promotionStatuses -ccontains [string]$_.status } |
            Sort-Object { [int]$_.id }
    )
    $canonicalRoster = @(
        $domains | ForEach-Object { [ordered]@{ id = [int]$_.id; name = [string]$_.name } }
    )
    $scheduledRoster = @(
        $domains | ForEach-Object { [ordered]@{ id = [int]$_.id; wire_name = [string]$_.name } }
    )
    $scheduledNames = @($domains | ForEach-Object { [string]$_.name })
    $authority = @(
        $domains | ForEach-Object {
            [ordered]@{ id = [int]$_.id; name = [string]$_.name; wire_name = [string]$_.name }
        }
    )
    $perDomain = [ordered]@{}
    foreach ($domain in $domains) {
        $perDomain[[string]$domain.name] = [ordered]@{
            domain_id = [int]$domain.id
            record_count = 2
            data_record_count = 2
            status_flags_or = 0
            effective_failure_status_flags_or = 0
        }
    }
    $captureId = 'synthetic-capture'
    $sessionId = '11111111-1111-4111-8111-111111111111'
    $targetPid = 123
    $targetCreationFiletimeUtc = [uint64]132537600000000000
    $requestedDurationMs = 1000
    $schedulerPollMs = 10
    $schedulerQueueCapacity = 1
    $schedulerWorkerCount = 1
    $targetExecutableSha256 =
        '870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad'
    $collectorDllSha256 =
        (Get-FileHash -LiteralPath $collector -Algorithm SHA256).Hash.ToLowerInvariant()
    $speedRootGatePath = Join-Path $captureRun 'speed_root_gate.json'
    [IO.File]::WriteAllText(
        $speedRootGatePath,
        '{"synthetic":"root-gate-retained-by-self-test"}',
        [Text.UTF8Encoding]::new($false)
    )
    $speedEvidenceTranscriptPath =
        Join-Path $captureRun 'speed_evidence_transcript.json'
    [IO.File]::WriteAllText(
        $speedEvidenceTranscriptPath,
        '{"synthetic":"raw-speed-evidence-transcript"}',
        [Text.UTF8Encoding]::new($false)
    )
    $speedEvidenceNonceLedgerRoot = Join-Path $captureRun 'speed_nonce_ledger'
    [IO.Directory]::CreateDirectory($speedEvidenceNonceLedgerRoot) | Out-Null
    $authorityPath = Join-Path $captureRun 'production_domain_roster_authority.json'
    [IO.File]::WriteAllText($authorityPath, ($authority | ConvertTo-Json -Depth 6),
        [Text.UTF8Encoding]::new($false))
    $planPath = Join-Path $captureRun 'production_capture_plan.json'
    $plan = [ordered]@{
        schema = 'battle-plan.production_capture_plan.v1'
        capture_id = $captureId
        session_id = $sessionId
        pid = $targetPid
        process_creation_filetime_utc = $targetCreationFiletimeUtc
        requested_duration_ms = $requestedDurationMs
        read_only_scheduler_poll_ms = $schedulerPollMs
        read_only_scheduler_queue_cap = $schedulerQueueCapacity
        scheduler_worker_count = $schedulerWorkerCount
        exe_sha256 = $targetExecutableSha256
        staged_dll_sha256 = $collectorDllSha256
        speed_required_raw = 4
        unpaused_required = $true
        read_only_scheduler_required = $true
        domain57_game_speed_gate_v2 = $speedRootGatePath
        domain57_game_speed_gate_v2_sha256 =
            (Get-FileHash -LiteralPath $speedRootGatePath -Algorithm SHA256).Hash.ToLowerInvariant()
        speed_evidence_transcript_path = $speedEvidenceTranscriptPath
        speed_evidence_transcript_sha256 = $hexA
        speed_evidence_transcript_file_sha256 =
            (Get-FileHash -LiteralPath $speedEvidenceTranscriptPath -Algorithm SHA256).Hash.ToLowerInvariant()
        speed_evidence_nonce_ledger_root = $speedEvidenceNonceLedgerRoot
        speed_evidence_nonce_seal_sha256 = $hexC
        prebuilt_source_input_sha256 = [string]$attested.source_input_sha256
        production_profile_domain_roster = $canonicalRoster
        scheduled_domain_roster = $scheduledRoster
        scheduled_domain_names = $scheduledNames
        domain_roster_authority_path = $authorityPath
        domain_roster_authority_sha256 =
            (Get-FileHash -LiteralPath $authorityPath -Algorithm SHA256).Hash.ToLowerInvariant()
        interaction_closure = [ordered]@{
            policy = 'passive_injected_observer'
            client_commands_during_capture = 0
            semantic_engine_state_writes = 0
            game_process_lifecycle_commands = 0
            game_ui_commands = 0
        }
    }
    [IO.File]::WriteAllText($planPath, ($plan | ConvertTo-Json -Depth 10),
        [Text.UTF8Encoding]::new($false))
    $manifestPath = Join-Path $captureRun 'native_session_manifest.json'
    $manifest = [ordered]@{
        capture_id = $captureId
        session_id = $sessionId
        pid = $targetPid
        skipped_game_hours = 0
        unique_game_hour_count = 2
        first_game_hour_raw = 100
        last_game_hour_raw = 101
        scheduled_domains = $scheduledNames
        compact_stream = [ordered]@{
            frame_end_record_count = 2
            domain_gap_record_count = 0
            domain_gap_outcomes = @()
            failure_status_flags_or = 0
            failure_status_seen = $false
            per_domain = $perDomain
        }
    }
    [IO.File]::WriteAllText($manifestPath, ($manifest | ConvertTo-Json -Depth 10),
        [Text.UTF8Encoding]::new($false))
    $collectorControlPath = Join-Path $captureRun 'collector_control.json'
    $pipeToken = 'synthetic-passive-pipe-token'
    $pipeName = "\\.\pipe\battle-plan_control_$targetPid`_$pipeToken"
    $collectorControl = [ordered]@{
        pipe_token = $pipeToken
        capture_ready_pipe_name_v1 = $pipeName
    }
    [IO.File]::WriteAllText(
        $collectorControlPath,
        ($collectorControl | ConvertTo-Json -Compress),
        [Text.UTF8Encoding]::new($false)
    )
    $hookAuthority =
        Assert-PassiveReadySourceContractParity 'vanilla_latest_2026_06_30'
    $clockHourRaw = 60006000
    $clockObservationOrdinal = [uint64]1
    $ready = [ordered]@{
        schema = 'hoi4-capture-ready-v2'
        status = 'ready'
        capture_policy = 'passive-observer-only'
        pid = $targetPid
        process_creation_filetime_utc = $targetCreationFiletimeUtc
        session_id = $sessionId
        capture_id = $captureId
        requested_duration_ms = $requestedDurationMs
        readiness_flags = 255
        validated_domain_count = $scheduledNames.Count
        scheduler_queue_capacity = $schedulerQueueCapacity
        scheduler_worker_count = $schedulerWorkerCount
        allowed_hook_count = [int]$hookAuthority.Count
        active_hook_count = [int]$hookAuthority.Count
        clock_hour_raw = $clockHourRaw
        clock_observation_ordinal = $clockObservationOrdinal
        target_executable_sha256 = $targetExecutableSha256
        collector_dll_sha256 = $collectorDllSha256
        hook_allowlist_sha256 = [string]$hookAuthority.Sha256
        hook_outcomes_sha256 = $hexA
        validated_domain_roster_sha256 =
            Get-PassiveUtf8Sha256 ($scheduledNames -join "`n")
        clock_witness_sha256 = Get-PassiveClockWitnessSha256 `
            -ClockHourRaw $clockHourRaw `
            -ObservationOrdinal $clockObservationOrdinal `
            -TargetExecutableSha256 $targetExecutableSha256 `
            -CollectorBuildProfileId 'vanilla_latest_2026_06_30'
        scheduler_config_sha256 = Get-PassiveSchedulerConfigSha256 `
            -PollMilliseconds $schedulerPollMs `
            -DomainQueueCapacity $schedulerQueueCapacity `
            -WorkerCount $schedulerWorkerCount
        output_pipe_identity_sha256 = Get-PassiveOutputPipeIdentitySha256 `
            -CanonicalPipeName $pipeName `
            -TargetPid $targetPid `
            -TargetCreationFiletimeUtc $targetCreationFiletimeUtc `
            -SessionId $sessionId
        zero_mutation_policy_sha256 = Get-PassiveZeroMutationPolicySha256
        session_id_sha256 = Get-PassiveUtf8Sha256 $sessionId
        capture_id_sha256 = Get-PassiveUtf8Sha256 $captureId
        control_sha256 =
            (Get-FileHash -LiteralPath $collectorControlPath -Algorithm SHA256).Hash.ToLowerInvariant()
        pipe_token_sha256 = Get-PassiveUtf8Sha256 $pipeToken
        ready_payload_sha256 = $hexA
        host_binding_sha256 = $hexA
        host_validated = $true
        public_receipt_core_sha256 = $hexB
        ready_ack_sha256 = $hexA
    }
    $ready.ready_payload_sha256 = Get-PassiveCaptureReadyPayloadSha256 `
        -Ready ([pscustomobject]$ready) `
        -PipeToken $pipeToken
    $hostBindingBytes = [Collections.Generic.List[byte]]::new()
    $hostBindingBytes.AddRange(
        [Text.UTF8Encoding]::new($false).GetBytes(
            "battle-plan.capture_ready.host_binding.v2`0"
        )
    )
    $hostBindingBytes.AddRange(
        (ConvertFrom-PassiveSha256 $ready.ready_payload_sha256)
    )
    Add-PassiveLittleEndianBytes $hostBindingBytes u32 $targetPid
    Add-PassiveLittleEndianBytes `
        $hostBindingBytes u64 $targetCreationFiletimeUtc
    Add-PassiveLittleEndianBytes $hostBindingBytes u32 $requestedDurationMs
    foreach ($digest in @(
        $targetExecutableSha256,
        $collectorDllSha256,
        $ready.session_id_sha256,
        $ready.capture_id_sha256,
        $ready.pipe_token_sha256
    )) {
        $hostBindingBytes.AddRange((ConvertFrom-PassiveSha256 $digest))
    }
    $ready.host_binding_sha256 =
        Get-PassiveBytesSha256 $hostBindingBytes.ToArray()
    $ready.ready_ack_sha256 = Get-PassiveCaptureReadyAckSha256 `
        -Ready ([pscustomobject]$ready) `
        -PipeToken $pipeToken
    $readyPath = Join-Path $captureRun 'CAPTURE_READY_V2.json'
    [IO.File]::WriteAllText(
        $readyPath,
        ($ready | ConvertTo-Json -Depth 6),
        [Text.UTF8Encoding]::new($false)
    )
    $teardown = [ordered]@{
        schema = 'hoi4-capture-teardown-v2'
        status = 'collector-unloaded'
        capture_policy = 'passive-observer-only'
        pid = $targetPid
        process_creation_filetime_utc = $targetCreationFiletimeUtc
        session_id = $sessionId
        capture_id = $captureId
        session_id_sha256 = $ready.session_id_sha256
        capture_id_sha256 = $ready.capture_id_sha256
        pipe_token_sha256 = $ready.pipe_token_sha256
        collector_dll_sha256 = $collectorDllSha256
        capture_ready_payload_sha256 = $ready.ready_payload_sha256
        capture_ready_receipt_core_sha256 = $ready.public_receipt_core_sha256
        capture_ready_ack_sha256 = $ready.ready_ack_sha256
        capture_ready_receipt_sha256 =
            (Get-FileHash -LiteralPath $readyPath -Algorithm SHA256).Hash.ToLowerInvariant()
        host_binding_sha256 = $ready.host_binding_sha256
        data_pipe_eof_observed = $true
        hook_lease_released_after_eof = $true
        collector_module_disposition = 'unloaded'
        collector_module_retention_policy = 'inline-detour.module.process-lifetime.v1'
        target_process_epoch_revalidated = $true
        target_code_temporarily_instrumented = $true
        pinned_hook_target_code_restored = $true
        semantic_game_state_mutated = $false
        pinned_hook_sites_restored = $true
        pinned_hook_site_count = [int]$hookAuthority.Count
        pinned_hook_site_baseline_sha256 = $hexC
        pinned_hook_site_restored_sha256 = $hexC
    }
    $teardownPath = Join-Path $captureRun 'CAPTURE_TEARDOWN_V1.json'
    [IO.File]::WriteAllText(
        $teardownPath,
        ($teardown | ConvertTo-Json -Depth 6),
        [Text.UTF8Encoding]::new($false)
    )
    $speedMonitorPath = Join-Path $captureRun 'speed5_monitor.jsonl'
    [IO.File]::WriteAllText(
        $speedMonitorPath,
        (@(
            '{"game_hour_raw":100,"speed_raw":4,"paused":false}',
            '{"game_hour_raw":100,"speed_raw":4,"paused":false}',
            '{"game_hour_raw":101,"speed_raw":4,"paused":false}'
        ) -join "`n") + "`n",
        [Text.UTF8Encoding]::new($false)
    )
    $speedPath = Join-Path $captureRun 'speed5_proof.json'
    $speed = [ordered]@{
        schema = 'battle-plan.speed5_proof.v2'
        session_id = $sessionId
        acceptance_mode = 'production'
        pid = $targetPid
        exe_sha256 = $targetExecutableSha256
        monitor_sha256 =
            (Get-FileHash -LiteralPath $speedMonitorPath -Algorithm SHA256).Hash.ToLowerInvariant()
        collector_control_sha256 =
            (Get-FileHash -LiteralPath $collectorControlPath -Algorithm SHA256).Hash.ToLowerInvariant()
        domain57_game_speed_gate_v2_sha256 =
            (Get-FileHash -LiteralPath $speedRootGatePath -Algorithm SHA256).Hash.ToLowerInvariant()
        speed_raw = 4
        paused = $false
        sample_count = 3
        first_game_hour_raw = 100
        last_game_hour_raw = 101
        capture_first_game_hour_raw = 100
        capture_last_game_hour_raw = 101
        capture_plan_sha256 =
            (Get-FileHash -LiteralPath $planPath -Algorithm SHA256).Hash.ToLowerInvariant()
    }
    [IO.File]::WriteAllText($speedPath, ($speed | ConvertTo-Json -Depth 6),
        [Text.UTF8Encoding]::new($false))
    $streamPath = Join-Path $captureRun 'collector_stream.bin'
    [IO.File]::WriteAllBytes(
        $streamPath,
        [Text.Encoding]::ASCII.GetBytes('synthetic-stream-bound-by-speed-gate-report')
    )
    $speedGateReportPath =
        Join-Path $captureRun 'independent_validation\speed5_stream_gate.report.json'
    [IO.Directory]::CreateDirectory(
        (Split-Path -Parent $speedGateReportPath)
    ) | Out-Null
    $speedGateReport = [ordered]@{
        schema = 'battle-plan.speed5_stream_gate.v2'
        passed = $true
        completed_frames = 2
        first_game_hour_raw = 100
        last_game_hour_raw = 101
        monitor_samples = 3
        domain57_game_speed_gate_v2_sha256 =
            (Get-FileHash -LiteralPath $speedRootGatePath -Algorithm SHA256).Hash.ToLowerInvariant()
        speed_evidence_transcript_sha256 = $hexA
        speed_evidence_transcript_file_sha256 =
            (Get-FileHash -LiteralPath $speedEvidenceTranscriptPath -Algorithm SHA256).Hash.ToLowerInvariant()
        nonce_seal_sha256 = $hexC
        collector_control_sha256 =
            (Get-FileHash -LiteralPath $collectorControlPath -Algorithm SHA256).Hash.ToLowerInvariant()
    }
    [IO.File]::WriteAllText(
        $speedGateReportPath,
        ($speedGateReport | ConvertTo-Json -Depth 6),
        [Text.UTF8Encoding]::new($false)
    )
    $speedStreamVerifierPath =
        Join-Path $fixtureScripts 'verify_speed5_stream.py'
    $speedEvidenceVerifierPath = $speedStreamVerifierPath
    $speedEvidenceVerifierSha =
        (Get-FileHash -LiteralPath $speedEvidenceVerifierPath -Algorithm SHA256).Hash.ToLowerInvariant()
    $speedGateArguments = @(
        $speedStreamVerifierPath,
        '--stream', $streamPath,
        '--manifest', $manifestPath,
        '--speed-proof', $speedPath,
        '--monitor', $speedMonitorPath,
        '--capture-plan', $planPath,
        '--root-gate', $speedRootGatePath,
        '--collector-control', $collectorControlPath,
        '--evidence-transcript', $speedEvidenceTranscriptPath,
        '--nonce-ledger-root', $speedEvidenceNonceLedgerRoot,
        '--evidence-verifier', $speedEvidenceVerifierPath,
        '--expected-verifier-sha256', $speedEvidenceVerifierSha,
        '--expected-probe-sha256', $hexB,
        '--expected-static-sha256', $hexC,
        '--out', $speedGateReportPath
    )
    $validationPath = Join-Path $captureRun 'production_validation_report.json'
    $validation = [ordered]@{
        schema = 'battle-plan.production_capture_validation.v2'
        ok = $true
        problems = @()
        manifest_sha256 =
            (Get-FileHash -LiteralPath $manifestPath -Algorithm SHA256).Hash.ToLowerInvariant()
        expected_domain_count = $domains.Count
        expected_domain_roster = $canonicalRoster
        independent_gates = @(
            [ordered]@{
                name = 'compact_gate'
                passed = $true
                arguments = @('--run-dir', $captureRun, '--require-zero-loss')
            },
            [ordered]@{
                name = 'speed5_stream_gate'
                passed = $true
                started = $true
                exit_code = 0
                timed_out = $false
                problem = $null
                executable_sha256 = $hexA
                arguments = $speedGateArguments
            }
        )
    }
    [IO.File]::WriteAllText($validationPath, ($validation | ConvertTo-Json -Depth 10),
        [Text.UTF8Encoding]::new($false))
    $sinkPath = Join-Path $captureRun 'sink_reconciliation_verification.json'
    $sink = [ordered]@{
        schema = 'extractor.sink-reconciliation-verification.v2'
        run_dir = $captureRun
        expected_outcome = 'success'
        verdict = 'passed'
        problems = @()
        evidence = [ordered]@{
            manifest_sha256 =
                (Get-FileHash -LiteralPath $manifestPath -Algorithm SHA256).Hash.ToLowerInvariant()
            production_capture_plan_sha256 =
                (Get-FileHash -LiteralPath $planPath -Algorithm SHA256).Hash.ToLowerInvariant()
            speed5_proof_sha256 =
                (Get-FileHash -LiteralPath $speedPath -Algorithm SHA256).Hash.ToLowerInvariant()
        }
    }
    [IO.File]::WriteAllText($sinkPath, ($sink | ConvertTo-Json -Depth 8),
        [Text.UTF8Encoding]::new($false))
    $captureReceiptPath = Join-Path $captureRun 'capture_validation_receipt.json'
    $captureReceipt = [ordered]@{
        schema = 'battle-plan.production_capture_validation_receipt.v1'
        capture_id = $captureId
        session_id = $sessionId
        acceptance_mode = 'production'
        admissible_for_production = $true
        receipt_path = $captureReceiptPath
        capture_plan_path = $planPath
        capture_plan_sha256 =
            (Get-FileHash -LiteralPath $planPath -Algorithm SHA256).Hash.ToLowerInvariant()
        manifest_path = $manifestPath
        manifest_sha256 =
            (Get-FileHash -LiteralPath $manifestPath -Algorithm SHA256).Hash.ToLowerInvariant()
        capture_ready_receipt_path = $readyPath
        capture_ready_receipt_sha256 =
            (Get-FileHash -LiteralPath $readyPath -Algorithm SHA256).Hash.ToLowerInvariant()
        capture_teardown_receipt_path = $teardownPath
        capture_teardown_receipt_sha256 =
            (Get-FileHash -LiteralPath $teardownPath -Algorithm SHA256).Hash.ToLowerInvariant()
        validation_report_path = $validationPath
        validation_report_sha256 =
            (Get-FileHash -LiteralPath $validationPath -Algorithm SHA256).Hash.ToLowerInvariant()
        sink_reconciliation_report_path = $sinkPath
        sink_reconciliation_report_sha256 =
            (Get-FileHash -LiteralPath $sinkPath -Algorithm SHA256).Hash.ToLowerInvariant()
        prebuilt_artifact_attestation_path = $retainedAttestationPath
        prebuilt_artifact_attestation_sha256 =
            (Get-FileHash -LiteralPath $retainedAttestationPath -Algorithm SHA256).Hash.ToLowerInvariant()
        prebuilt_source_input_sha256 = [string]$attested.source_input_sha256
        interaction_closure = [ordered]@{
            policy = 'passive_injected_observer'
            client_commands_during_capture = 0
            semantic_engine_state_writes = 0
            game_process_lifecycle_commands = 0
            game_ui_commands = 0
        }
    }
    [IO.File]::WriteAllText($captureReceiptPath, ($captureReceipt | ConvertTo-Json -Depth 10),
        [Text.UTF8Encoding]::new($false))

    Remove-Item -LiteralPath $zip -Force
    $builderResult = Invoke-JsonScript -Path $fixtureBuilder -Arguments @(
        '-RecorderExe', $recorder,
        '-ReplayExe', $replay,
        '-CollectorDll', $collector,
        '-OutputZip', $zip,
        '-Version', '0.1.0',
        '-ProductionCaptureReceiptPath', $captureReceiptPath,
        '-ProductionArtifactAttestationPath', $attestationPath
    )
    Assert-Case 'builder promotes only exact current real-capture evidence' (
        $builderResult.admissible_for_production -is [bool] -and
        $builderResult.admissible_for_production -and
        [string]$builderResult.production_capture_evidence.schema -ceq
            'battle-plan.production_capture_promotion_evidence.v1' -and
        $builderResult.production_capture_evidence.all_domains_real_source_complete -and
        $builderResult.production_capture_evidence.zero_loss -and
        [int]$builderResult.production_capture_evidence.speed_raw -eq 4 -and
        $builderResult.production_capture_evidence.unpaused
    )
    $fixtureAttachLog = Join-Path $root 'extractor\native\collector\src\attach_log.rs'
    $fixtureAttachLogOriginal = [IO.File]::ReadAllText($fixtureAttachLog)
    [IO.File]::WriteAllText(
        $fixtureAttachLog,
        ($fixtureAttachLogOriginal +
            @'

fn synthetic_structural_d71_passive_gate() {
    if passive_product_capture {
        if compact_explicit_domain_enabled(
            &control,
            RAW_DYNAMIC_COLLECTION_TRACE_LIVE_DOMAIN,
        ) {
            return Err("synthetic passive D71 rejection".into());
        }
    }
}
'@),
        [Text.UTF8Encoding]::new($false)
    )
    try {
        $blockedD71Closure =
            Get-ProductionPassiveSourceAdmissionClosure -RepoRoot $root
        Assert-Case 'statically impossible D71 passive source is exact and non-admissible' (
            -not $blockedD71Closure.no_statically_proven_impossible_production_sources -and
            @($blockedD71Closure.statically_impossible_domains).Count -eq 1 -and
            [int]$blockedD71Closure.statically_impossible_domains[0].id -eq 71
        )
    }
    finally {
        [IO.File]::WriteAllText(
            $fixtureAttachLog,
            $fixtureAttachLogOriginal,
            [Text.UTF8Encoding]::new($false)
        )
    }
    $manifestOriginal = [IO.File]::ReadAllText($manifestPath)
    $validationOriginal = [IO.File]::ReadAllText($validationPath)
    $sinkOriginal = [IO.File]::ReadAllText($sinkPath)
    $speedOriginal = [IO.File]::ReadAllText($speedPath)
    $readyOriginal = [IO.File]::ReadAllText($readyPath)
    $teardownOriginal = [IO.File]::ReadAllText($teardownPath)
    $captureReceiptOriginal = [IO.File]::ReadAllText($captureReceiptPath)
    try {
        $mutatedManifest = $manifestOriginal | ConvertFrom-Json
        $d71Domain = @($domains | Where-Object { [int]$_.id -eq 71 })
        if ($d71Domain.Count -ne 1) {
            throw 'production promotion self-test requires exact D71 registry membership'
        }
        $d71WireName = [string]$d71Domain[0].name
        $mutatedManifest.compact_stream.per_domain.$d71WireName.status_flags_or = 0x100
        [IO.File]::WriteAllText($manifestPath, ($mutatedManifest | ConvertTo-Json -Depth 10),
            [Text.UTF8Encoding]::new($false))
        $mutatedManifestSha =
            (Get-FileHash -LiteralPath $manifestPath -Algorithm SHA256).Hash.ToLowerInvariant()
        $mutatedValidation = $validationOriginal | ConvertFrom-Json
        $mutatedValidation.manifest_sha256 = $mutatedManifestSha
        [IO.File]::WriteAllText($validationPath,
            ($mutatedValidation | ConvertTo-Json -Depth 10),
            [Text.UTF8Encoding]::new($false))
        $mutatedSink = $sinkOriginal | ConvertFrom-Json
        $mutatedSink.evidence.manifest_sha256 = $mutatedManifestSha
        [IO.File]::WriteAllText($sinkPath, ($mutatedSink | ConvertTo-Json -Depth 8),
            [Text.UTF8Encoding]::new($false))
        $mutatedReceipt = $captureReceiptOriginal | ConvertFrom-Json
        $mutatedReceipt.manifest_sha256 = $mutatedManifestSha
        $mutatedReceipt.validation_report_sha256 =
            (Get-FileHash -LiteralPath $validationPath -Algorithm SHA256).Hash.ToLowerInvariant()
        $mutatedReceipt.sink_reconciliation_report_sha256 =
            (Get-FileHash -LiteralPath $sinkPath -Algorithm SHA256).Hash.ToLowerInvariant()
        [IO.File]::WriteAllText($captureReceiptPath,
            ($mutatedReceipt | ConvertTo-Json -Depth 10),
            [Text.UTF8Encoding]::new($false))
        Invoke-ExpectFailure 'D71 source-unavailable status cannot be masked into promotion' {
            Get-ProductionCapturePromotionEvidence `
                -RepoRoot $root `
                -CaptureReceiptPath $captureReceiptPath `
                -CurrentArtifactAttestationPath $attestationPath | Out-Null
        }
    }
    finally {
        [IO.File]::WriteAllText($manifestPath, $manifestOriginal,
            [Text.UTF8Encoding]::new($false))
        [IO.File]::WriteAllText($validationPath, $validationOriginal,
            [Text.UTF8Encoding]::new($false))
        [IO.File]::WriteAllText($sinkPath, $sinkOriginal,
            [Text.UTF8Encoding]::new($false))
        [IO.File]::WriteAllText($captureReceiptPath, $captureReceiptOriginal,
            [Text.UTF8Encoding]::new($false))
    }

    $invokeManifestPromotionFailure = {
        param(
            [Parameter(Mandatory = $true)][string]$Name,
            [Parameter(Mandatory = $true)][scriptblock]$Mutate
        )
        try {
            $mutatedManifest = $manifestOriginal | ConvertFrom-Json
            & $Mutate $mutatedManifest
            [IO.File]::WriteAllText(
                $manifestPath,
                ($mutatedManifest | ConvertTo-Json -Depth 10),
                [Text.UTF8Encoding]::new($false)
            )
            $mutatedManifestSha =
                (Get-FileHash -LiteralPath $manifestPath -Algorithm SHA256).Hash.ToLowerInvariant()
            $mutatedValidation = $validationOriginal | ConvertFrom-Json
            $mutatedValidation.manifest_sha256 = $mutatedManifestSha
            [IO.File]::WriteAllText(
                $validationPath,
                ($mutatedValidation | ConvertTo-Json -Depth 10),
                [Text.UTF8Encoding]::new($false)
            )
            $mutatedSink = $sinkOriginal | ConvertFrom-Json
            $mutatedSink.evidence.manifest_sha256 = $mutatedManifestSha
            [IO.File]::WriteAllText(
                $sinkPath,
                ($mutatedSink | ConvertTo-Json -Depth 8),
                [Text.UTF8Encoding]::new($false)
            )
            $mutatedReceipt = $captureReceiptOriginal | ConvertFrom-Json
            $mutatedReceipt.manifest_sha256 = $mutatedManifestSha
            $mutatedReceipt.validation_report_sha256 =
                (Get-FileHash -LiteralPath $validationPath -Algorithm SHA256).Hash.ToLowerInvariant()
            $mutatedReceipt.sink_reconciliation_report_sha256 =
                (Get-FileHash -LiteralPath $sinkPath -Algorithm SHA256).Hash.ToLowerInvariant()
            [IO.File]::WriteAllText(
                $captureReceiptPath,
                ($mutatedReceipt | ConvertTo-Json -Depth 10),
                [Text.UTF8Encoding]::new($false)
            )
            Invoke-ExpectFailure $Name {
                Get-ProductionCapturePromotionEvidence `
                    -RepoRoot $root `
                    -CaptureReceiptPath $captureReceiptPath `
                    -CurrentArtifactAttestationPath $attestationPath | Out-Null
            }
        }
        finally {
            [IO.File]::WriteAllText(
                $manifestPath,
                $manifestOriginal,
                [Text.UTF8Encoding]::new($false)
            )
            [IO.File]::WriteAllText(
                $validationPath,
                $validationOriginal,
                [Text.UTF8Encoding]::new($false)
            )
            [IO.File]::WriteAllText(
                $sinkPath,
                $sinkOriginal,
                [Text.UTF8Encoding]::new($false)
            )
            [IO.File]::WriteAllText(
                $captureReceiptPath,
                $captureReceiptOriginal,
                [Text.UTF8Encoding]::new($false)
            )
        }
    }
    & $invokeManifestPromotionFailure `
        'unknown compact status bit cannot be promoted' `
        {
            param($Value)
            $firstProperty = @($Value.compact_stream.per_domain.PSObject.Properties)[0]
            $firstProperty.Value.status_flags_or = 0x4000
        }
    & $invokeManifestPromotionFailure `
        'missing manifest speed boundary cannot be promoted' `
        {
            param($Value)
            $Value.first_game_hour_raw = $null
        }

    try {
        $mutatedReady = $readyOriginal | ConvertFrom-Json
        $mutatedReady.host_validated = $false
        [IO.File]::WriteAllText(
            $readyPath,
            ($mutatedReady | ConvertTo-Json -Depth 8),
            [Text.UTF8Encoding]::new($false)
        )
        $mutatedReceipt = $captureReceiptOriginal | ConvertFrom-Json
        $mutatedReceipt.capture_ready_receipt_sha256 =
            (Get-FileHash -LiteralPath $readyPath -Algorithm SHA256).Hash.ToLowerInvariant()
        [IO.File]::WriteAllText(
            $captureReceiptPath,
            ($mutatedReceipt | ConvertTo-Json -Depth 10),
            [Text.UTF8Encoding]::new($false)
        )
        Invoke-ExpectFailure 'unauthenticated passive READY cannot be promoted' {
            Get-ProductionCapturePromotionEvidence `
                -RepoRoot $root `
                -CaptureReceiptPath $captureReceiptPath `
                -CurrentArtifactAttestationPath $attestationPath | Out-Null
        }
    }
    finally {
        [IO.File]::WriteAllText($readyPath, $readyOriginal,
            [Text.UTF8Encoding]::new($false))
        [IO.File]::WriteAllText($captureReceiptPath, $captureReceiptOriginal,
            [Text.UTF8Encoding]::new($false))
    }

    try {
        $mutatedTeardown = $teardownOriginal | ConvertFrom-Json
        $mutatedTeardown.semantic_game_state_mutated = $true
        [IO.File]::WriteAllText(
            $teardownPath,
            ($mutatedTeardown | ConvertTo-Json -Depth 8),
            [Text.UTF8Encoding]::new($false)
        )
        $mutatedReceipt = $captureReceiptOriginal | ConvertFrom-Json
        $mutatedReceipt.capture_teardown_receipt_sha256 =
            (Get-FileHash -LiteralPath $teardownPath -Algorithm SHA256).Hash.ToLowerInvariant()
        [IO.File]::WriteAllText(
            $captureReceiptPath,
            ($mutatedReceipt | ConvertTo-Json -Depth 10),
            [Text.UTF8Encoding]::new($false)
        )
        Invoke-ExpectFailure 'semantic game-state mutation in teardown cannot be promoted' {
            Get-ProductionCapturePromotionEvidence `
                -RepoRoot $root `
                -CaptureReceiptPath $captureReceiptPath `
                -CurrentArtifactAttestationPath $attestationPath | Out-Null
        }
    }
    finally {
        [IO.File]::WriteAllText($teardownPath, $teardownOriginal,
            [Text.UTF8Encoding]::new($false))
        [IO.File]::WriteAllText($captureReceiptPath, $captureReceiptOriginal,
            [Text.UTF8Encoding]::new($false))
    }

    try {
        $mutatedValidation = $validationOriginal | ConvertFrom-Json
        $mutatedValidation.independent_gates = @(
            $mutatedValidation.independent_gates |
                Where-Object { [string]$_.name -cne 'speed5_stream_gate' }
        )
        [IO.File]::WriteAllText(
            $validationPath,
            ($mutatedValidation | ConvertTo-Json -Depth 10),
            [Text.UTF8Encoding]::new($false)
        )
        $mutatedReceipt = $captureReceiptOriginal | ConvertFrom-Json
        $mutatedReceipt.validation_report_sha256 =
            (Get-FileHash -LiteralPath $validationPath -Algorithm SHA256).Hash.ToLowerInvariant()
        [IO.File]::WriteAllText(
            $captureReceiptPath,
            ($mutatedReceipt | ConvertTo-Json -Depth 10),
            [Text.UTF8Encoding]::new($false)
        )
        Invoke-ExpectFailure 'missing independent speed-five stream gate cannot be promoted' {
            Get-ProductionCapturePromotionEvidence `
                -RepoRoot $root `
                -CaptureReceiptPath $captureReceiptPath `
                -CurrentArtifactAttestationPath $attestationPath | Out-Null
        }
    }
    finally {
        [IO.File]::WriteAllText($validationPath, $validationOriginal,
            [Text.UTF8Encoding]::new($false))
        [IO.File]::WriteAllText($captureReceiptPath, $captureReceiptOriginal,
            [Text.UTF8Encoding]::new($false))
    }

    try {
        $mutatedSpeed = $speedOriginal | ConvertFrom-Json
        $mutatedSpeed.capture_first_game_hour_raw = 99
        [IO.File]::WriteAllText(
            $speedPath,
            ($mutatedSpeed | ConvertTo-Json -Depth 8),
            [Text.UTF8Encoding]::new($false)
        )
        $mutatedSink = $sinkOriginal | ConvertFrom-Json
        $mutatedSink.evidence.speed5_proof_sha256 =
            (Get-FileHash -LiteralPath $speedPath -Algorithm SHA256).Hash.ToLowerInvariant()
        [IO.File]::WriteAllText(
            $sinkPath,
            ($mutatedSink | ConvertTo-Json -Depth 8),
            [Text.UTF8Encoding]::new($false)
        )
        $mutatedReceipt = $captureReceiptOriginal | ConvertFrom-Json
        $mutatedReceipt.sink_reconciliation_report_sha256 =
            (Get-FileHash -LiteralPath $sinkPath -Algorithm SHA256).Hash.ToLowerInvariant()
        [IO.File]::WriteAllText(
            $captureReceiptPath,
            ($mutatedReceipt | ConvertTo-Json -Depth 10),
            [Text.UTF8Encoding]::new($false)
        )
        Invoke-ExpectFailure 'speed-five proof from a different capture interval is rejected' {
            Get-ProductionCapturePromotionEvidence `
                -RepoRoot $root `
                -CaptureReceiptPath $captureReceiptPath `
                -CurrentArtifactAttestationPath $attestationPath | Out-Null
        }
    }
    finally {
        [IO.File]::WriteAllText($speedPath, $speedOriginal,
            [Text.UTF8Encoding]::new($false))
        [IO.File]::WriteAllText($sinkPath, $sinkOriginal,
            [Text.UTF8Encoding]::new($false))
        [IO.File]::WriteAllText($captureReceiptPath, $captureReceiptOriginal,
            [Text.UTF8Encoding]::new($false))
    }

    $packageReceiptPath = Join-Path $releaseRoot 'hoi4-portable-package-receipt.json'
    $receipt = [ordered]@{
        schema = 'battle-plan.central_portable_package.v1'
        status = 'PASS_CENTRAL_RELEASE_PORTABLE_PACKAGE'
        successful_exit = $true
        central_validation = [ordered]@{
            mode = 'Release'
            requested_workspace = 'All'
            release_runtime_linkage = $runtimeLinkage
            production_feature_closure = [ordered]@{
                schema = 'battle-plan.production_feature_closure.v1'
                rebuild_order = @('recorder_host', 'collector_dll')
                recorder_host = [ordered]@{
                    package = 'recorder_host'
                    target = 'collector_record'
                    default_features = $false
                    enabled_features = @()
                    artifact_sha256 = [string]$attested.artifact_sha256['recorder_host']
                }
                collector_dll = [ordered]@{
                    package = 'collector'
                    target = 'lib'
                    crate_types = @('cdylib', 'rlib')
                    default_features = $false
                    enabled_features = @()
                    # This belongs to collector_dll, which is where
                    # Invoke-CentralRustValidation emits it and where the
                    # receipt verifier reads it. The fixture had it on
                    # recorder_host, where nothing produces it and nothing
                    # reads it, so the verifier hit a missing property under
                    # Set-StrictMode instead of checking the closure.
                    all_non_default_features_disabled = $true
                    forbidden_features = @(
                        'test-support',
                        'unshipped-a25-runtime-text-evidence',
                        'unshipped-a39-character-presentation-evidence',
                        'unshipped-a31-focus-presentation-evidence'
                    )
                    built_last = $true
                    collector_cargo_sha256 =
                        [string]$builderResult.interaction_closure.policy_inputs.collector_cargo_sha256
                    pinned_hook_count =
                        [int]$builderResult.interaction_closure.source_capability.exact_pinned_hook_count
                    pinned_hook_contract_source_sha256 =
                        [string]$builderResult.interaction_closure.policy_inputs.passive_ready_contract_sha256
                    artifact_sha256 = [string]$attested.artifact_sha256['collector_dll']
                }
            }
        }
        artifact_attestation = [ordered]@{
            path = [string]$attested.receipt_path
            sha256 = [string]$attested.receipt_sha256
        }
        package_builder = [ordered]@{
            path = [IO.Path]::GetFullPath($fixtureBuilder)
            sha256 = (Get-FileHash -LiteralPath $fixtureBuilder -Algorithm SHA256).Hash.ToLowerInvariant()
        }
        package = $builderResult
        source_input_sha256 = [string]$attested.source_input_sha256
        build_environment = $attested.build_environment
    }
    [IO.File]::WriteAllText(
        $packageReceiptPath,
        ($receipt | ConvertTo-Json -Depth 12),
        [Text.UTF8Encoding]::new($false)
    )

    $verified = Invoke-JsonScript -Path $fixtureVerifier -Arguments @('-RepoRoot', $root, '-ReceiptPath', $packageReceiptPath)
    Assert-Case 'verifier accepts authentic fixture' ([string]$verified.status -ceq 'PASS_CENTRAL_RELEASE_PORTABLE_PACKAGE')
    Assert-Case 'verifier returns builder package hash' ([string]$verified.package_sha256 -ceq [string]$builderResult.sha256)

    $zipOriginal = [IO.File]::ReadAllBytes($zip)
    $zipTampered = [byte[]]$zipOriginal.Clone()
    $zipTampered[$zipTampered.Length - 1] = $zipTampered[$zipTampered.Length - 1] -bxor 0xff
    [IO.File]::WriteAllBytes($zip, $zipTampered)
    Invoke-ExpectFailure 'tampered package bytes are rejected' {
        Invoke-JsonScript -Path $fixtureVerifier -Arguments @('-RepoRoot', $root, '-ReceiptPath', $packageReceiptPath) | Out-Null
    }
    [IO.File]::WriteAllBytes($zip, $zipOriginal)

    $receiptOriginal = [IO.File]::ReadAllText($packageReceiptPath)
    $tamperedReceipt = $receiptOriginal | ConvertFrom-Json
    $tamperedReceipt.status = 'PASS_WITHOUT_AUTHORITY'
    [IO.File]::WriteAllText(
        $packageReceiptPath,
        ($tamperedReceipt | ConvertTo-Json -Depth 12),
        [Text.UTF8Encoding]::new($false)
    )
    Invoke-ExpectFailure 'tampered central receipt is rejected' {
        Invoke-JsonScript -Path $fixtureVerifier -Arguments @('-RepoRoot', $root, '-ReceiptPath', $packageReceiptPath) | Out-Null
    }
    [IO.File]::WriteAllText($packageReceiptPath, $receiptOriginal, [Text.UTF8Encoding]::new($false))

    $tamperedSourceCapability = $receiptOriginal | ConvertFrom-Json
    $tamperedSourceCapability.package.interaction_closure.source_capability.collector_fixture_hooks_default_off = $false
    [IO.File]::WriteAllText(
        $packageReceiptPath,
        ($tamperedSourceCapability | ConvertTo-Json -Depth 12),
        [Text.UTF8Encoding]::new($false)
    )
    Invoke-ExpectFailure 'tampered production source-capability closure is rejected' {
        Invoke-JsonScript -Path $fixtureVerifier -Arguments @('-RepoRoot', $root, '-ReceiptPath', $packageReceiptPath) | Out-Null
    }
    [IO.File]::WriteAllText($packageReceiptPath, $receiptOriginal, [Text.UTF8Encoding]::new($false))

    $tamperedFeatureClosure = $receiptOriginal | ConvertFrom-Json
    $tamperedFeatureClosure.central_validation.production_feature_closure.collector_dll.built_last = $false
    [IO.File]::WriteAllText(
        $packageReceiptPath,
        ($tamperedFeatureClosure | ConvertTo-Json -Depth 12),
        [Text.UTF8Encoding]::new($false)
    )
    Invoke-ExpectFailure 'tampered production feature closure is rejected' {
        Invoke-JsonScript -Path $fixtureVerifier -Arguments @('-RepoRoot', $root, '-ReceiptPath', $packageReceiptPath) | Out-Null
    }
    [IO.File]::WriteAllText($packageReceiptPath, $receiptOriginal, [Text.UTF8Encoding]::new($false))

    $tamperedClosure = $receiptOriginal | ConvertFrom-Json
    $tamperedClosure.package.interaction_closure.recorder_host.forbidden_symbols_absent = $false
    [IO.File]::WriteAllText(
        $packageReceiptPath,
        ($tamperedClosure | ConvertTo-Json -Depth 12),
        [Text.UTF8Encoding]::new($false)
    )
    Invoke-ExpectFailure 'tampered interaction-closure verdict is rejected' {
        Invoke-JsonScript -Path $fixtureVerifier -Arguments @('-RepoRoot', $root, '-ReceiptPath', $packageReceiptPath) | Out-Null
    }
    [IO.File]::WriteAllText($packageReceiptPath, $receiptOriginal, [Text.UTF8Encoding]::new($false))

    $tamperedMarkers = $receiptOriginal | ConvertFrom-Json
    $tamperedMarkers.package.interaction_closure.collector_dll.required_markers = @()
    [IO.File]::WriteAllText(
        $packageReceiptPath,
        ($tamperedMarkers | ConvertTo-Json -Depth 12),
        [Text.UTF8Encoding]::new($false)
    )
    Invoke-ExpectFailure 'tampered interaction-closure marker roster is rejected' {
        Invoke-JsonScript -Path $fixtureVerifier -Arguments @('-RepoRoot', $root, '-ReceiptPath', $packageReceiptPath) | Out-Null
    }
    [IO.File]::WriteAllText($packageReceiptPath, $receiptOriginal, [Text.UTF8Encoding]::new($false))

    $tamperedPolicyInput = $receiptOriginal | ConvertFrom-Json
    $tamperedPolicyInput.package.interaction_closure.policy_inputs.read_only_game_state_witness_sha256 = ('f' * 64) -join ''
    [IO.File]::WriteAllText(
        $packageReceiptPath,
        ($tamperedPolicyInput | ConvertTo-Json -Depth 12),
        [Text.UTF8Encoding]::new($false)
    )
    Invoke-ExpectFailure 'tampered passive policy-input digest is rejected' {
        Invoke-JsonScript -Path $fixtureVerifier -Arguments @('-RepoRoot', $root, '-ReceiptPath', $packageReceiptPath) | Out-Null
    }
    [IO.File]::WriteAllText($packageReceiptPath, $receiptOriginal, [Text.UTF8Encoding]::new($false))

    $wrongKindZip = Join-Path $releaseRoot 'wrong-kind.zip'
    Invoke-ExpectFailure 'DLL cannot masquerade as recorder executable' {
        Invoke-JsonScript -Path $fixtureBuilder -Arguments @(
            '-RecorderExe', $collector,
            '-ReplayExe', $replay,
            '-CollectorDll', $collector,
            '-OutputZip', $wrongKindZip,
            '-Version', '0.1.0'
        ) | Out-Null
    }

    $cleanupZip = Join-Path $releaseRoot 'post-temp-cleanup.zip'
    Invoke-ExpectFailure 'post-temp builder failure is rejected' {
        Invoke-JsonScript -Path $fixtureBuilder -Arguments @(
            '-RecorderExe', $recorder,
            '-ReplayExe', $replay,
            '-CollectorDll', $collector,
            '-OutputZip', $cleanupZip,
            '-Version', '0.1.0',
            '-TestOnlyFailAfterTemporaryWrite'
        ) | Out-Null
    }
    Assert-Case 'post-temp failure publishes no package' (-not (Test-Path -LiteralPath $cleanupZip))
    $cleanupTemporary = @(
        Get-ChildItem -LiteralPath $releaseRoot -Filter '.post-temp-cleanup.zip.*.tmp' -Force
    )
    Assert-Case 'post-temp failure removes unpublished temporary package' ($cleanupTemporary.Count -eq 0)

    $publishedCleanupZip = Join-Path $releaseRoot 'post-publish-cleanup.zip'
    Invoke-ExpectFailure 'post-publication builder failure is rejected' {
        Invoke-JsonScript -Path $fixtureBuilder -Arguments @(
            '-RecorderExe', $recorder,
            '-ReplayExe', $replay,
            '-CollectorDll', $collector,
            '-OutputZip', $publishedCleanupZip,
            '-Version', '0.1.0',
            '-TestOnlyFailAfterPublishedWrite'
        ) | Out-Null
    }
    Assert-Case 'post-publication failure removes published package' (
        -not (Test-Path -LiteralPath $publishedCleanupZip)
    )

    [pscustomobject]@{
        schema = 'battle-plan.portable_package_selftest.v1'
        status = 'PASS'
        cases = $script:PortablePackageSelfTestCases
    } | ConvertTo-Json -Depth 3
}
finally {
    if (Test-Path -LiteralPath $root) {
        Remove-Item -LiteralPath $root -Recurse -Force
    }
}
