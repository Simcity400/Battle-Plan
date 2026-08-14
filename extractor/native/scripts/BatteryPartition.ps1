# The central battery's serial/parallel partition.
#
# WHY THIS FILE EXISTS
#
# Invoke-CentralRustValidation.ps1 held ONE machine-wide mutex around its whole
# run, so a lane wanting the battery waited for every other lane's battery to
# finish -- compile included. Lanes routinely burned eight or more blocked
# attempts at 45 s apart, and the thing they were queueing behind was, for most
# of its wall clock, a compile into a build directory that is private to each
# worktree and could never have conflicted with theirs. Measured on this tree:
# the Extractor workspace compiles in 1 m 14 s and runs its entire test set in
# about 141 s, so the compile is not a rounding error in the queue, it is most
# of it.
#
# THE PARTITION IS TWO THINGS, AND ONLY THE FIRST IS A GUESS
#
#   1. OWNERSHIP. A package OWNS a machine-scoped facility if its source
#      mentions one of the markers below. This is a text scan, so it can be
#      wrong -- but it is arranged so the only way it can be wrong is by naming
#      too many owners. Nothing is stripped from the source before matching, not
#      even comments: a stripper that removes a line cannot be proven not to
#      remove a line of code (Rust raw strings can contain lines that begin `/*`
#      or `//`), and removing code is the one error this scan may never make.
#      Measured cost of not stripping: 17 owners instead of 13, and all four
#      extra ones were already serial through the closure below.
#
#   2. REACH. A package is SERIAL if it is an owner or if it (transitively)
#      DEPENDS on one, taken from cargo's own dependency graph.
#
# The dependency graph is what makes reach a fact rather than a guess. An
# earlier version tried to detect reach by looking for `owner::function` in the
# caller's text, and it was wrong in every direction at once: it missed
# `use owner::freeze_process as fp; fp()`, it missed renamed cargo
# dependencies, it missed `pub unsafe extern "system" fn` declarations in the
# owner, and it stopped after one hop, so A->B->C left C parallel. Cargo already
# knows all of this exactly.
#
# WHAT THE MARKERS ARE NOT
#
# They are deliberately NOT "this test measures time". nextest already runs this
# battery ~16 ways parallel inside a single run, so every test in it is already
# required to survive a loaded machine; `Instant::now` is not evidence of
# anything. What a SECOND battery adds is contention for things there is only
# one of on the machine, and that is what the table below names.

Set-StrictMode -Version Latest

# The marker table. Each entry is one machine-scoped facility, with the reason a
# concurrent battery can change a verdict that depends on it. Adding a marker
# moves packages INTO the serial half; removing one needs the argument that the
# facility is not machine-scoped after all.
$script:BatteryContentionMarkers = [ordered]@{
    named_kernel_object = @{
        Pattern = 'CreateMutex[AW]|OpenMutex[AW]|CreateNamedPipe[AW]|CreateFileMapping[AW]|OpenFileMapping[AW]|CreateSemaphore[AW]|OpenEvent[AW]'
        Why     = 'a named kernel object is one object per machine (or per session): two batteries opening the same name get the same object, so one run''s acquisition is the other run''s refusal.'
    }
    machine_namespace_literal = @{
        # Anchored at the start of a string literal, in both Rust spellings.
        # Unanchored, this matched the `Local\` inside every
        # `C:\Users\Example\AppData\Local\...` path in the tree, which is a
        # directory, not a kernel namespace.
        Pattern = '"(?:Global|Local)\\\\|r"(?:Global|Local)\\'
        Why     = 'a literal Global\ or Local\ prefix names an object in a namespace shared by every process on the machine (or session).'
    }
    system_enumeration = @{
        Pattern = 'CreateToolhelp32Snapshot|EnumProcesses|EnumWindows|FindWindow[AW]?\(|Process32(?:First|Next)|Thread32(?:First|Next)'
        Why     = 'a system-wide enumeration sees the other battery''s processes, threads and windows, so both what it finds and how long it takes depend on what else is running.'
    }
    thread_suspension = @{
        Pattern = 'SuspendThread|ResumeThread'
        Why     = 'suspension is driven off an enumeration of everything running, and a suspended-thread window is measured against an OS that is now serving two batteries.'
    }
    registry = @{
        Pattern = 'RegOpenKey|RegQueryValue|RegCreateKey|RegSetValue|HKEY_'
        Why     = 'the registry is one store per machine; a test that writes one is writing where the other battery reads.'
    }
    network_endpoint = @{
        Pattern = 'TcpListener|TcpStream|UdpSocket'
        Why     = 'a port is machine-scoped: the second battery to bind it fails, and which battery is second is a race.'
    }
    process_spawn = @{
        Pattern = 'CreateProcess[AW]|std::process::Command|Command::new'
        Why     = 'a test whose verdict waits on a CHILD PROCESS making progress is measuring the machine''s scheduler, not its own logic, and a second battery is exactly what slows that child down.'
    }
    affinity_priority = @{
        Pattern = 'SetThreadAffinityMask|SetProcessAffinityMask|SetPriorityClass|SetThreadPriority'
        Why     = 'affinity and priority are claims on the machine''s cores, and two batteries making them at once get neither what they asked for nor a repeatable answer.'
    }
}

# The trees whose cargo packages the battery compiles and runs.
$script:BatteryPartitionTrees = @(
    'extractor\native',
    'replay_interface\native',
    'portable_replay\native'
)

# Every cargo root the battery drives, workspaces and standalone roots alike.
# The dependency graph is read from all of them because a standalone root's
# packages are not members of either workspace and would otherwise have no edges.
$script:BatteryCargoRoots = @(
    'extractor\native',
    'replay_interface\native',
    'extractor\native\project_progress_live_lane',
    'extractor\native\lanes\occupation_resistance_compliance_live\replay_verify',
    'portable_replay\native\admission',
    'portable_replay\native\compact_stream',
    'portable_replay\native\content_manifest',
    'portable_replay\native\render_stream',
    'portable_replay\native\snapshot'
)

# Directory names never scanned: build output, git metadata, and the append-only
# evidence tree, none of which contain production or test source.
$script:BatteryPartitionSkipDirectories = @('target', '.cargo-target', '.git', 'runs')

function Get-BatteryPackageInventory {
    <#
    .SYNOPSIS
    Every cargo package under the trees the battery builds, with its directory.
    #>
    param([Parameter(Mandatory = $true)][string]$RepoRoot)

    $inventory = [System.Collections.Generic.List[object]]::new()
    foreach ($tree in $script:BatteryPartitionTrees) {
        $base = Join-Path $RepoRoot $tree
        if (-not (Test-Path -LiteralPath $base -PathType Container)) {
            throw "battery partition scan: tree is missing: $base"
        }
        foreach ($manifest in Get-ChildItem -LiteralPath $base -Recurse -File -Filter 'Cargo.toml' -ErrorAction Stop) {
            $relative = $manifest.FullName.Substring($RepoRoot.Length).TrimStart('\', '/')
            $segments = $relative -split '[\\/]'
            if (@($segments | Where-Object { $script:BatteryPartitionSkipDirectories -contains $_ }).Count -gt 0) {
                continue
            }
            $text = Get-Content -LiteralPath $manifest.FullName -Raw
            $name = [regex]::Match($text, '(?m)^[ \t]*name[ \t]*=[ \t]*"(?<name>[0-9A-Za-z._-]+)"[ \t]*(?:#.*)?\r?$')
            if (-not $name.Success) {
                # A virtual manifest (a `[workspace]` with no `[package]`) has no
                # name and owns no tests; anything else without one is a manifest
                # this scan does not understand and must not silently skip.
                if ($text -match '(?m)^[ \t]*\[package\][ \t]*(?:#.*)?$') {
                    throw "battery partition scan: package manifest has no name: $($manifest.FullName)"
                }
                continue
            }
            $inventory.Add([ordered]@{
                Name      = $name.Groups['name'].Value
                Directory = $manifest.DirectoryName
                # The canonical manifest path is the package's IDENTITY. Names
                # are not unique: two repository-local packages can share one,
                # and reconciling on the name lets a scanned marker-free
                # `shared_facility` vouch for an unscanned one that calls
                # SuspendThread.
                Manifest  = [IO.Path]::GetFullPath($manifest.FullName)
            })
        }
    }
    if ($inventory.Count -eq 0) {
        throw (
            'battery partition scan inventoried no cargo packages at all. That is ' +
            'not a pass: it means the scan stopped matching this repository''s ' +
            'layout, and every test would have been classified by an empty table.'
        )
    }
    return $inventory
}

function Get-BatteryFacilityOwners {
    <#
    .SYNOPSIS
    The packages whose own source touches a machine-scoped facility.

    .DESCRIPTION
    A plain text scan over every `.rs` file, with NOTHING stripped -- not
    comments, not doc comments, not block comments. Two earlier strippers were
    both fail-open in ways that were only found by adversarial review: `///.*$`
    applied anywhere on a line erased everything after a string containing
    `file:///x`, and a `/* ... */` regex paired an opener inside one string
    literal with a closer inside a later one and erased every line between. Rust
    raw strings can legally contain lines that begin `/*` or `//`, so no regex
    stripper can be proven safe without lexing Rust properly.

    The cost of that decision is measured, not assumed: stripping nothing names
    17 owners instead of 13, and all four extra packages were already serial
    through the dependency closure. A file that merely DOCUMENTS a facility now
    makes its package an owner, which is over-inclusion -- the only direction
    this scan is allowed to be wrong in.

    An unreadable source file is a REFUSAL, not a package quietly cleared: the
    file this scan could not read is exactly the file that might have carried
    the marker, and its callers depend on the answer.
    #>
    param([Parameter(Mandatory = $true)][string]$RepoRoot)

    $inventory = Get-BatteryPackageInventory -RepoRoot $RepoRoot
    $owners = [System.Collections.Generic.SortedSet[string]]::new([StringComparer]::Ordinal)
    # Manifest paths, because they are the identity a cargo package id can be
    # matched against exactly; names cannot.
    $ownerManifests = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    $scannedManifests = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    $evidence = [ordered]@{}
    foreach ($package in $inventory) {
        $sources = @()
        try {
            $sources = @(Get-ChildItem -LiteralPath $package.Directory -Recurse -File -Filter '*.rs' -ErrorAction Stop |
                    Where-Object {
                        $rel = $_.FullName.Substring($package.Directory.Length).TrimStart('\', '/')
                        @($rel -split '[\\/]' | Where-Object { $script:BatteryPartitionSkipDirectories -contains $_ }).Count -eq 0
                    })
        }
        catch {
            throw (
                "battery partition scan cannot list the sources of package " +
                "'$($package.Name)' ($($package.Directory)): $($_.Exception.Message). " +
                'Refusing rather than classifying a package whose source was not read.'
            )
        }
        $hits = [ordered]@{}
        foreach ($file in $sources) {
            $text = $null
            try {
                $text = Get-Content -LiteralPath $file.FullName -Raw -ErrorAction Stop
            }
            catch {
                throw (
                    "battery partition scan cannot read $($file.FullName): " +
                    "$($_.Exception.Message). Refusing rather than classifying a " +
                    'package whose source was not read.'
                )
            }
            if ($null -eq $text) { continue }
            foreach ($marker in $script:BatteryContentionMarkers.GetEnumerator()) {
                if ($hits.Contains($marker.Key)) { continue }
                if ([regex]::IsMatch($text, $marker.Value.Pattern)) {
                    $hits[$marker.Key] = $file.FullName.Substring($RepoRoot.Length).TrimStart('\', '/').Replace('\', '/')
                }
            }
        }
        [void]$scannedManifests.Add([string]$package.Manifest)
        if ($hits.Count -gt 0) {
            [void]$owners.Add($package.Name)
            [void]$ownerManifests.Add([string]$package.Manifest)
            if ($evidence.Contains($package.Name)) {
                foreach ($hit in $hits.GetEnumerator()) { $evidence[$package.Name][$hit.Key] = $hit.Value }
            }
            else {
                $evidence[$package.Name] = $hits
            }
        }
    }
    if ($owners.Count -eq 0) {
        throw (
            'battery partition scan found NO package that touches a machine-scoped ' +
            'facility. That is not a pass: this repository opens named kernel ' +
            'objects, enumerates system threads and spawns child processes, so an ' +
            'empty owner set means the marker table stopped matching the source ' +
            'and the whole battery would have run without the machine-wide mutex.'
        )
    }
    return [ordered]@{
        Owners           = @($owners)
        OwnerManifests   = @($ownerManifests)
        ScannedManifests = @($scannedManifests)
        Evidence         = $evidence
    }
}

function Test-PathIsInside {
    <#
    .SYNOPSIS
    Is $Path inside $Root, as a canonical path with a separator boundary?

    .DESCRIPTION
    `-like "$Root*"` was wrong twice over: it treats any wildcard
    metacharacter in the repository path as a pattern, and a bare prefix makes
    `C:\repo-copy` count as inside `C:\repo`. Case-insensitive because Windows
    paths are.
    #>
    param(
        [Parameter(Mandatory = $true)][string]$Root,
        [Parameter(Mandatory = $true)][AllowEmptyString()][string]$Path
    )
    if ([string]::IsNullOrWhiteSpace($Path)) { return $false }
    $prefix = [IO.Path]::GetFullPath($Root).TrimEnd('\', '/') + [IO.Path]::DirectorySeparatorChar
    return ([IO.Path]::GetFullPath($Path)).StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)
}

function Get-BatteryWorkspaceDependencyEdges {
    <#
    .SYNOPSIS
    The dependency graph, from cargo itself, over the WHOLE resolve.

    .DESCRIPTION
    Keyed by cargo package ID, not by name, and it keeps EVERY node -- including
    packages outside this repository. Two reasons, both fail-closed:

     * a graph pruned to repository-local nodes loses paths that leave and come
       back. `repo/caller -> C:\shared\bridge -> repo/facility_owner` has both
       of its edges deleted by that pruning, and `caller` is then classified
       parallel while transitively reaching a facility. The closure runs over
       the complete graph and only the RESULT is narrowed to battery packages;
     * package IDs rather than names, because two versions of a name are two
       nodes and collapsing them merges their edges.

    `resolve.nodes` is used rather than `packages[].dependencies`: the former is
    what cargo actually resolved, the latter is the requirement text and has to
    be name-matched back, which is the guessing this whole rewrite removed.

    All dependency kinds count. A dev-dependency is exactly how a crate's TESTS
    reach another crate, and the tests are what is being partitioned.

    .OUTPUTS
    Edges (id -> HashSet of dependency ids), IdToName, LocalIds (ids whose
    manifest is inside the repository).
    #>
    param([Parameter(Mandatory = $true)][string]$RepoRoot)

    $edges = @{}
    $idToName = @{}
    $idToManifest = @{}
    $localIds = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    foreach ($relative in $script:BatteryCargoRoots) {
        $cargoRoot = Join-Path $RepoRoot $relative
        if (-not (Test-Path -LiteralPath (Join-Path $cargoRoot 'Cargo.toml') -PathType Leaf)) {
            throw "battery partition: cargo root has no manifest: $cargoRoot"
        }
        Push-Location $cargoRoot
        try {
            $json = & cargo metadata --format-version 1 --offline
            if ($LASTEXITCODE -ne 0) {
                throw "cargo metadata failed in $cargoRoot; the battery partition has no dependency graph for it."
            }
            $metadata = ($json | Out-String) | ConvertFrom-Json
        }
        finally {
            Pop-Location
        }
        foreach ($package in $metadata.packages) {
            $id = [string]$package.id
            if ([string]::IsNullOrWhiteSpace($id)) {
                throw "cargo metadata in $cargoRoot returned a package with no id; the partition cannot build a graph from it."
            }
            $idToName[$id] = [string]$package.name
            $manifestPath = [string]$package.manifest_path
            if ([string]::IsNullOrWhiteSpace($manifestPath)) {
                throw "cargo metadata in $cargoRoot returned package '$id' with no manifest path; the partition cannot establish its identity."
            }
            $idToManifest[$id] = [IO.Path]::GetFullPath($manifestPath)
            if (Test-PathIsInside -Root $RepoRoot -Path $manifestPath) {
                [void]$localIds.Add($id)
            }
        }
        if ($null -eq $metadata.resolve -or $null -eq $metadata.resolve.nodes) {
            throw "cargo metadata in $cargoRoot returned no resolve graph; the battery partition cannot compute reach."
        }
        foreach ($node in $metadata.resolve.nodes) {
            $id = [string]$node.id
            if (-not $edges.ContainsKey($id)) {
                $edges[$id] = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
            }
            foreach ($dependency in $node.dependencies) {
                [void]$edges[$id].Add([string]$dependency)
            }
        }
    }
    if ($edges.Count -eq 0) {
        throw 'battery partition read an empty dependency graph; every package would have been classified as reaching nothing.'
    }
    return [ordered]@{
        Edges        = $edges
        IdToName     = $idToName
        IdToManifest = $idToManifest
        LocalIds     = @($localIds)
    }
}

function Get-BatteryPartition {
    <#
    .SYNOPSIS
    Split the battery's cargo packages into the serial and parallel halves.

    .DESCRIPTION
    Serial = the transitive closure of the facility owners under the reversed
    dependency graph: an owner, or anything that can reach one. Iterated to a
    FIXED POINT, so A -> B -> owner puts A in the serial half as surely as B.

    .OUTPUTS
    An ordered hashtable: Serial (string[]), Parallel (string[]), Owners
    (string[]), Evidence (package -> marker -> first matching file).
    #>
    param(
        [Parameter(Mandatory = $true)][string]$RepoRoot,
        # Injectable so the closure can be tested against a hand-built graph,
        # including the A->B->C shape a one-hop rule gets wrong and the
        # leaves-and-returns shape a repository-pruned graph gets wrong.
        [object]$DependencyGraph
    )

    $ownership = Get-BatteryFacilityOwners -RepoRoot $RepoRoot
    if ($null -eq $DependencyGraph) {
        $DependencyGraph = Get-BatteryWorkspaceDependencyEdges -RepoRoot $RepoRoot
    }
    $edges = $DependencyGraph.Edges
    $idToName = $DependencyGraph.IdToName
    $localIds = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    foreach ($id in $DependencyGraph.LocalIds) { [void]$localIds.Add($id) }

    # THE UNIVERSES MUST AGREE, AND THEY ARE COMPARED BY MANIFEST PATH.
    #
    # The owner scan walks three source trees; the graph admits every package
    # whose manifest is anywhere inside the repository. A repository-local
    # package the scan never saw -- a vendored path dependency under
    # repo\vendor, a workspace member outside those trees -- would be in the
    # graph, never checked for markers, and cleared as parallel along with
    # everything depending on it.
    #
    # Comparing by NAME is not enough, and the difference is a live fail-open: a
    # scanned, marker-free `shared_facility` under a partition tree would vouch
    # for an UNSCANNED `shared_facility` elsewhere in the repository that calls
    # SuspendThread, and neither it nor its callers would be serial. The
    # canonical manifest path is the identity that cannot be shared.
    $scannedManifests = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    foreach ($manifest in $ownership.ScannedManifests) { [void]$scannedManifests.Add([string]$manifest) }
    $idManifests = $DependencyGraph.IdToManifest
    $unscanned = @($localIds |
            Where-Object { -not $scannedManifests.Contains([string]$idManifests[$_]) } |
            ForEach-Object { '{0} ({1})' -f [string]$idToName[$_], [string]$idManifests[$_] } |
            Sort-Object -Unique)
    if ($unscanned.Count -gt 0) {
        throw (
            'battery partition: ' + $unscanned.Count + ' repository-local package(s) are in ' +
            'cargo''s dependency graph but were never scanned for machine-scoped ' +
            'markers: ' + ($unscanned -join '; ') + '. They would be classified ' +
            'parallel without anything having read them. Add their directory to ' +
            '$script:BatteryPartitionTrees in BatteryPartition.ps1.'
        )
    }

    # Owners are seeded by MANIFEST PATH for the same reason: an id is an owner
    # because THAT package's source matched, not because something else sharing
    # its name did.
    $ownerManifests = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    foreach ($manifest in $ownership.OwnerManifests) { [void]$ownerManifests.Add([string]$manifest) }

    # The closure runs over IDS, across the WHOLE graph, so a path that leaves
    # the repository and comes back is followed rather than severed.
    $serialIds = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    foreach ($id in $edges.Keys) {
        if ($ownerManifests.Contains([string]$idManifests[$id])) { [void]$serialIds.Add($id) }
    }
    $changed = $true
    while ($changed) {
        $changed = $false
        foreach ($id in @($edges.Keys)) {
            if ($serialIds.Contains($id)) { continue }
            foreach ($dependency in $edges[$id]) {
                if ($serialIds.Contains($dependency)) {
                    [void]$serialIds.Add($id)
                    $changed = $true
                    break
                }
            }
        }
    }

    # Only the RESULT is narrowed to the repository's own packages. An external
    # crate that reaches an owner is not a battery package and has no tests here.
    $serial = [System.Collections.Generic.SortedSet[string]]::new([StringComparer]::Ordinal)
    $parallel = [System.Collections.Generic.SortedSet[string]]::new([StringComparer]::Ordinal)
    foreach ($id in $localIds) {
        $name = [string]$idToName[$id]
        if ($serialIds.Contains($id)) { [void]$serial.Add($name) } else { [void]$parallel.Add($name) }
    }
    # An owner that cargo's graph never mentioned (a package in the source trees
    # that no cargo root builds) is still serial: it was read and it matched.
    foreach ($owner in $ownership.Owners) {
        if (-not $serial.Contains($owner)) {
            [void]$serial.Add($owner)
            [void]$parallel.Remove($owner)
        }
    }
    # A name can appear on both sides only if two ids share it and disagree.
    # Serial wins, because serial is the safe side of that disagreement.
    foreach ($name in @($serial)) { [void]$parallel.Remove($name) }

    return [ordered]@{
        Serial   = @($serial)
        Parallel = @($parallel)
        Owners   = @($ownership.Owners)
        Evidence = $ownership.Evidence
    }
}

function Get-BatterySerialFilterset {
    <#
    .SYNOPSIS
    The nextest filterset naming exactly the serial half.

    .DESCRIPTION
    `package(=name)` is the EXACT matcher. The substring form would make
    `package(collector)` also select packages whose names merely contain it,
    which is the difference between a partition and an approximation.

    The parallel half is never spelled out: callers take `not (<this>)`, so the
    two halves are complements by construction and no test can fall out of both.
    #>
    param([Parameter(Mandatory = $true)][AllowEmptyCollection()][string[]]$SerialPackages)

    if ($SerialPackages.Count -eq 0) {
        throw 'refusing to build a serial filterset from an empty package set.'
    }
    foreach ($name in $SerialPackages) {
        if ($name -notmatch '^[0-9A-Za-z._-]+$') {
            throw "battery partition: package name is not filterset-safe: '$name'"
        }
    }
    return (($SerialPackages | ForEach-Object { "package(=$_)" }) -join ' | ')
}

function Get-BatteryRootHalves {
    <#
    .SYNOPSIS
    The two filtersets for ONE cargo root, scoped to the packages it contains.

    .DESCRIPTION
    A filterset naming a package the current cargo root cannot see is a hard
    nextest parse error ("no packages matched this"), so the serial names must be
    intersected with the root's own members before they are spelled.

    Parallel is always `not (Serial)` -- never an enumeration of its own -- so
    the halves are complements and a test cannot be missing from both. When the
    root has no serial package at all, both are $null and the caller runs the
    root once, unfiltered, in the parallel half.
    #>
    param(
        [Parameter(Mandatory = $true)][object]$Partition,
        [Parameter(Mandatory = $true)][AllowEmptyCollection()][string[]]$RootPackages
    )

    $rootSet = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    foreach ($name in $RootPackages) { [void]$rootSet.Add($name) }
    $scoped = @($Partition.Serial | Where-Object { $rootSet.Contains($_) } | Sort-Object)
    if ($scoped.Count -eq 0) {
        return [ordered]@{ SerialPackages = @(); Serial = $null; Parallel = $null }
    }
    $serial = Get-BatterySerialFilterset -SerialPackages $scoped
    return [ordered]@{
        SerialPackages = $scoped
        Serial         = $serial
        Parallel       = "not ($serial)"
    }
}

function Assert-BatteryPartitionCoversPackages {
    <#
    .SYNOPSIS
    Refuse a partition that does not classify a package the battery will run.

    .DESCRIPTION
    THE fail-closed rule. The owner scan walks directories; nextest and the
    dependency graph walk cargo's package graph. If those ever disagree -- a
    package under a tree the scan does not list, a manifest it could not name, a
    cargo root missing from $script:BatteryCargoRoots -- the disagreement must
    not resolve to "then it is parallel". It resolves to a refusal.
    #>
    param(
        [Parameter(Mandatory = $true)][AllowEmptyCollection()][string[]]$ObservedPackages,
        [Parameter(Mandatory = $true)][AllowEmptyCollection()][string[]]$SerialPackages,
        [Parameter(Mandatory = $true)][AllowEmptyCollection()][string[]]$ParallelPackages
    )

    $classified = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    foreach ($name in @($SerialPackages) + @($ParallelPackages)) { [void]$classified.Add($name) }
    $unclassified = @($ObservedPackages | Where-Object { -not $classified.Contains($_) } | Sort-Object -Unique)
    if ($unclassified.Count -gt 0) {
        throw (
            'battery partition does not classify ' + $unclassified.Count + ' package(s) ' +
            'the battery runs tests for: ' + ($unclassified -join ', ') + '. An ' +
            'unclassified package is SERIAL, and this run refuses rather than ' +
            'assume otherwise. Add its cargo root to $script:BatteryCargoRoots in ' +
            'BatteryPartition.ps1.'
        )
    }
}

function Write-BatteryPartitionSummary {
    <#
    .SYNOPSIS
    Print the partition: the owners with their markers, and the closure size.
    #>
    param([Parameter(Mandatory = $true)][object]$Partition)

    Write-Host (
        'Battery partition: {0} machine-serial package(s) ({1} facility owner(s) plus their dependents), {2} parallel-safe' -f `
            @($Partition.Serial).Count, @($Partition.Owners).Count, @($Partition.Parallel).Count
    )
    foreach ($name in $Partition.Owners) {
        $markers = @($Partition.Evidence[$name].Keys) -join ', '
        Write-Host ("  OWNS  {0}  [{1}]" -f $name, $markers)
    }
    Write-Host ("  parallel-safe: {0}" -f (@($Partition.Parallel) -join ', '))
}
