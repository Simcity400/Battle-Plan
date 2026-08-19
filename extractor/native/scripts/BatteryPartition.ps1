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
    'portable_replay\native\snapshot',
    # Lane LD. This one was MISSED rather than excluded. The crate was added
    # 2026-08-05 (9f930b85) and this inventory was written 2026-08-14
    # (eb5052c0), so it already existed and simply was not enumerated -- the
    # same defect the $portableStandaloneRoots comment in the runner describes
    # ("~190 tests ... had never executed"), one crate short of complete.
    # Until it was listed here nothing anywhere selected `portable_terrain_fixture`:
    # it is a member of no workspace, `--workspace` never reached it, and
    # `cargo check -p portable_terrain_fixture` from the Replay root -- where it
    # is a path dependency of `map_viewer` but not a member -- makes cargo PANIC
    # (exit 101). Its three `#[cfg(test)]` tests had never run, while the
    # fixture they guard is what `map_viewer`'s constant-buffer contract test
    # argues over.
    'portable_replay\native\terrain_fixture',
    # Lane KY. A cargo root of its own for the same reason `test_fs_retry` has
    # to exist at all: it is a `[dev-dependencies]` entry of 13 packages across
    # BOTH workspaces, and only `portable_replay` is a tree both may depend on.
    # Being a path dependency of thirteen packages is exactly what does NOT get
    # it selected -- LD's guard above threw on this crate before this line was
    # added, naming it as a member of no cargo root, which is the check working
    # as designed rather than a second copy of LD's lesson.
    'portable_replay\native\test_fs_retry'
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
    $targetSourceOwners = @{}
    $localIds = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    foreach ($relative in $script:BatteryCargoRoots) {
        $cargoRoot = Join-Path $RepoRoot $relative
        if (-not (Test-Path -LiteralPath (Join-Path $cargoRoot 'Cargo.toml') -PathType Leaf)) {
            throw "battery partition: cargo root has no manifest: $cargoRoot"
        }
        Push-Location $cargoRoot
        try {
            # --all-features widens the resolve: an OPTIONAL dependency that the
            # default feature set leaves out is not an edge without it, and a
            # package reached only through such an edge would be classified as
            # reaching nothing. Widening is the safe direction for both consumers
            # of this graph -- it can only make MORE packages serial and MORE
            # packages affected -- and it costs 0.9 s on this repository.
            $json = & cargo metadata --all-features --format-version 1 --offline
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
            # CARGO'S OWN TARGET TABLE decides which source file is the root of
            # which target, and a manifest may point a target at a file OUTSIDE
            # its own directory (`[[bin]] path = "../../other/src/bin/x.rs"`,
            # `[lib] path`, `package.build`). Nearest-ancestor attribution gets
            # every one of those wrong, and this map -- read from cargo, never
            # inferred -- is what corrects them.
            foreach ($target in @($package.targets)) {
                $sourcePath = [string]$target.src_path
                if ([string]::IsNullOrWhiteSpace($sourcePath)) { continue }
                $canonical = [IO.Path]::GetFullPath($sourcePath)
                if (-not $targetSourceOwners.ContainsKey($canonical)) {
                    $targetSourceOwners[$canonical] =
                        [System.Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
                }
                [void]$targetSourceOwners[$canonical].Add([IO.Path]::GetFullPath($manifestPath))
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
        Edges              = $edges
        IdToName           = $idToName
        IdToManifest       = $idToManifest
        LocalIds           = @($localIds)
        # canonical source path -> the manifests of every target rooted at it.
        TargetSourceOwners = $targetSourceOwners
    }
}

function Get-BatteryDependentIdClosure {
    <#
    .SYNOPSIS
    The seed ids plus every id that (transitively) DEPENDS on one of them.

    .DESCRIPTION
    THE ONE CLOSURE IN THIS FILE. Both questions the partition answers are the
    same walk over cargo's resolve graph with a different seed:

      * which packages are machine-SERIAL?  seed = the facility owners;
      * which packages can a change BREAK?  seed = the packages it touched.

    Keeping them as one function is the authority rule for this file. Two copies
    of a fixed point drift, and the direction they drift in is silent: a closure
    that stops early leaves a machine-contending test outside the mutex, or
    leaves an importer out of an iteration run, and neither errors.

    $Edges maps an id to the ids it DEPENDS ON, so a node joins the closure when
    any of its dependencies is already in it. Iterated to a FIXED POINT, so
    A -> B -> seed puts A in as surely as B; a one-hop rule does not.

    Runs over the WHOLE graph, external nodes included. A path that leaves the
    repository and comes back (`repo/a -> external/bridge -> repo/b`) has both
    of its edges deleted by a repository-pruned graph, and `a` would then be
    classified as reaching nothing.
    #>
    param(
        [Parameter(Mandatory = $true)][hashtable]$Edges,
        [Parameter(Mandatory = $true)][AllowEmptyCollection()][string[]]$SeedIds
    )

    $closure = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    foreach ($id in $SeedIds) { [void]$closure.Add([string]$id) }
    $changed = $true
    while ($changed) {
        $changed = $false
        foreach ($id in @($Edges.Keys)) {
            if ($closure.Contains($id)) { continue }
            foreach ($dependency in $Edges[$id]) {
                if ($closure.Contains($dependency)) {
                    [void]$closure.Add($id)
                    $changed = $true
                    break
                }
            }
        }
    }
    return $closure
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
    $seedIds = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    foreach ($id in $edges.Keys) {
        if ($ownerManifests.Contains([string]$idManifests[$id])) { [void]$seedIds.Add($id) }
    }
    $serialIds = Get-BatteryDependentIdClosure -Edges $edges -SeedIds @($seedIds)

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

# ==========================================================================
# THE AFFECTED-PACKAGE SIDE OF THE SAME CORE
#
# Everything below answers one question -- "which packages can this change
# break?" -- and it is answered with the SAME cargo resolve graph and the SAME
# fixed-point walk the serial half above uses. There is deliberately no second
# graph reader and no second closure: the serial half is "an owner, or anything
# that transitively DEPENDS on one", and the affected set is "a touched package,
# or anything that transitively DEPENDS on one". One walk, two seeds.
#
# Two consumers:
#
#   * `-Package X` on any mode that compiles a narrowed selection. Cargo's
#     `--package X` builds X and X's DEPENDENCIES; X's IMPORTERS are not built
#     at all, so a lane editing a leaf crate had its gate compile four packages
#     while five crates that import it were never compiled. Three lanes did that
#     expansion by hand before it became the runner's job.
#   * `-Mode Iterate`, which derives the seed from a git diff instead of from
#     the caller.
#
# The direction of error is fixed and it is the same as the serial half's: this
# may name TOO MANY packages, never too few, except where a refusal says
# otherwise in so many words.
# ==========================================================================

function Split-BatteryNulSeparatedOutput {
    <#
    .SYNOPSIS
    The records of a `-z` git listing, as strings.

    .DESCRIPTION
    `-z` is used for every git listing here rather than the default, because the
    default QUOTES any path with a special character in it (`core.quotePath` is
    on by default, so that includes every non-ASCII name) and a quoted path
    silently does not match anything on disk. With `-z` git emits the bytes
    verbatim, NUL-terminated, and nothing needs unquoting.

    PowerShell 5.1 splits a native command's stdout on newlines before this sees
    it, so the pieces are rejoined with a newline first. That is lossless for
    everything except a `\r` inside a file name, which this repository does not
    have and which would present as an extra path rather than a missing one.
    #>
    param([Parameter(Mandatory = $true)][AllowNull()][AllowEmptyCollection()][object]$Raw)

    if ($null -eq $Raw) { return @() }
    $joined = (@($Raw) -join "`n")
    if ([string]::IsNullOrEmpty($joined)) { return @() }
    return @($joined -split "`0" | Where-Object { $_ -ne '' })
}

function Get-BatteryChangedPaths {
    <#
    .SYNOPSIS
    Every repository-relative path this working tree changed since $Since.

    .DESCRIPTION
    Two listings, unioned, because either alone is a fail-open:

      * `git diff --name-only <Since>` sees what has been COMMITTED since the
        base and nothing that is still uncommitted;
      * `git status --porcelain` sees the working tree and nothing that was
        committed.

    A lane that has committed half its work and is iterating on the other half
    is the normal case, so both are read.

    `--no-renames` is deliberate. Rename detection reports only the destination,
    and a file moved OUT of a package changes that package too -- it stops
    compiling something. Without detection the same move presents as a delete
    and an add, so both packages are touched.

    THE EXIT CODE IS READ BEFORE ANYTHING ELSE. A pipe overwrites $LASTEXITCODE,
    and "git could not tell us what changed" must never arrive at the caller
    looking like "nothing changed": the first is a refusal, the second would be
    a run that compiles nothing and reports green.
    #>
    param(
        [Parameter(Mandatory = $true)][string]$RepoRoot,
        [Parameter(Mandatory = $true)][string]$Since
    )

    $paths = [System.Collections.Generic.SortedSet[string]]::new([StringComparer]::OrdinalIgnoreCase)

    # RESOLVE THE BASE TO A COMMIT FIRST. `$Since` arrives from a command line;
    # an option-like spelling would otherwise be parsed by git as a FLAG rather
    # than a revision, and `git diff --name-only <flag> --` can succeed while
    # meaning something else entirely. `rev-parse --verify <rev>^{commit}`
    # accepts only something that names one commit, and its output is a hash.
    $resolved = & git -C $RepoRoot rev-parse --verify --quiet ("{0}^{{commit}}" -f $Since)
    $resolveExit = $LASTEXITCODE
    if ($resolveExit -ne 0 -or [string]::IsNullOrWhiteSpace(($resolved | Out-String))) {
        throw (
            "'$Since' does not name a commit in this worktree (git rev-parse exited " +
            "$resolveExit). Pass -Since a commit this lane branched from; refusing " +
            'rather than deriving a selection from a base that does not exist.'
        )
    }
    $Since = ([string](@($resolved) | Select-Object -First 1)).Trim()

    $diffRaw = & git -C $RepoRoot diff --name-only --no-renames -z $Since --
    $diffExit = $LASTEXITCODE
    if ($diffExit -ne 0) {
        throw (
            "git diff against '$Since' exited with code $diffExit. The iteration " +
            'selection has no diff to derive itself from, and an empty selection ' +
            'would run no tests at all. Pass -Since with a commit this worktree ' +
            'contains; the unified validator will otherwise fall back to all shipping tests.'
        )
    }
    foreach ($path in (Split-BatteryNulSeparatedOutput -Raw $diffRaw)) {
        [void]$paths.Add(($path -replace '\\', '/'))
    }

    $statusRaw = & git -C $RepoRoot status --porcelain=v1 --untracked-files=all -z
    $statusExit = $LASTEXITCODE
    if ($statusExit -ne 0) {
        throw (
            "git status exited with code $statusExit. The iteration selection " +
            'cannot see the working tree, and proceeding would test the committed ' +
            'state while claiming to have tested the edits.'
        )
    }
    # Porcelain v1 with -z is a sequence of `XY <space> <path>` records, and a
    # rename or copy record is FOLLOWED by a second record holding the original
    # path. Both paths are changes: the source stopped being compiled where it
    # was and the destination started.
    $tokens = @(Split-BatteryNulSeparatedOutput -Raw $statusRaw)
    $index = 0
    while ($index -lt $tokens.Count) {
        $record = [string]$tokens[$index]
        $index++
        if ($record.Length -lt 4) {
            throw "git status returned a record this scan cannot parse: '$record'"
        }
        [void]$paths.Add(($record.Substring(3) -replace '\\', '/'))
        if ($record[0] -eq 'R' -or $record[0] -eq 'C' -or $record[1] -eq 'R' -or $record[1] -eq 'C') {
            if ($index -ge $tokens.Count) {
                throw "git status ended on a rename record with no original path: '$record'"
            }
            [void]$paths.Add(([string]$tokens[$index] -replace '\\', '/'))
            $index++
        }
    }

    return @($paths)
}

function Get-BatteryPackagesForPaths {
    <#
    .SYNOPSIS
    Which cargo package owns each changed path, and which paths none does.

    .DESCRIPTION
    Nearest-ancestor manifest, LONGEST directory first so a package nested
    inside another wins over its parent. Ownership is recorded by manifest path
    as well as by name, because names are not unique in this repository and the
    closure downstream seeds on identity.

    EVERY file under a package counts, not only `.rs`: a fixture, a `.toml`, a
    file pulled in with `include_str!`, a `build.rs` input, all change what that
    package compiles or asserts.

    THE ONE DIRECTION THIS CAN BE WRONG, NAMED. Lane JD proved that a source
    file's TARGET membership cannot be inferred from its path or its text --
    `#[path]`, `include!`, and macro-generated attributes can splice a file into
    a target that no scan can predict (runs/jd_battery_granularity). The same
    spellings can, in principle, splice a file across a PACKAGE boundary, and
    this map would then attribute the file to the package it sits in rather than
    the package that compiles it.

    That residual is why this function is used by ITERATION ONLY. The owner's
    ruling scopes iteration to "the tests the diff can break" and keeps the full
    battery at landings and nightly, so a miss here costs a lane one late
    signal, not a bad seal. No landing gate calls it. The `-Package` expansion,
    which does run in landing-adjacent modes, is seeded from names the CALLER
    supplied and never from this map.
    #>
    param(
        [Parameter(Mandatory = $true)][string]$RepoRoot,
        [Parameter(Mandatory = $true)][AllowEmptyCollection()][string[]]$Paths,
        [object[]]$Inventory,
        # canonical source path -> manifests, from cargo's own target table.
        [hashtable]$TargetSourceOwners,
        # canonical source path -> manifests, from the splice scan.
        [hashtable]$SpliceOwners,
        # canonical source path -> manifests, from cargo's dep-info. Optional
        # precision layer; when absent the lexical scan above is the floor.
        [hashtable]$DepInfoOwners
    )

    if ($null -eq $Inventory) { $Inventory = @(Get-BatteryPackageInventory -RepoRoot $RepoRoot) }
    $ordered = @($Inventory | Sort-Object -Property @{ Expression = { ([string]$_.Directory).Length } } -Descending)
    $manifestToName = @{}
    foreach ($package in $Inventory) { $manifestToName[[string]$package.Manifest] = [string]$package.Name }

    $packages = [System.Collections.Generic.SortedSet[string]]::new([StringComparer]::Ordinal)
    $manifests = [System.Collections.Generic.SortedSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    $unowned = [System.Collections.Generic.SortedSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    $workspaceWide = [System.Collections.Generic.SortedSet[string]]::new([StringComparer]::OrdinalIgnoreCase)

    foreach ($relative in $Paths) {
        if ([string]::IsNullOrWhiteSpace($relative)) { continue }
        # A workspace-wide input is decided FIRST and short-circuits everything
        # else: `extractor/native/Cargo.toml` sits under a tree and would
        # otherwise be attributed to whichever package happens to enclose it.
        if (Test-BatteryPathIsWorkspaceWideInput -Path $relative) {
            [void]$workspaceWide.Add($relative)
            continue
        }
        $absolute = [IO.Path]::GetFullPath((Join-Path $RepoRoot $relative))
        $claimed = $false

        # 1. CARGO'S TARGET TABLE, which is authority and not inference. A file
        #    a manifest points a target at belongs to that manifest's package
        #    even when it lives under another one.
        if ($null -ne $TargetSourceOwners -and $TargetSourceOwners.ContainsKey($absolute)) {
            foreach ($manifest in $TargetSourceOwners[$absolute]) {
                [void]$manifests.Add([string]$manifest)
                if ($manifestToName.ContainsKey([string]$manifest)) {
                    [void]$packages.Add([string]$manifestToName[[string]$manifest])
                }
                $claimed = $true
            }
        }

        # 2. THE SPLICE SCAN, additive: every package that pulls this file in
        #    with include!/include_str!/include_bytes!/#[path].
        if ($null -ne $SpliceOwners -and $SpliceOwners.ContainsKey($absolute)) {
            foreach ($manifest in $SpliceOwners[$absolute]) {
                [void]$manifests.Add([string]$manifest)
                if ($manifestToName.ContainsKey([string]$manifest)) {
                    [void]$packages.Add([string]$manifestToName[[string]$manifest])
                }
                $claimed = $true
            }
        }

        # 2b. CARGO'S DEP-INFO, additive: every package whose compiler actually
        #     READ this file. This is the layer that sees a file spliced in by a
        #     COMPUTED literal, which no textual scan can resolve -- the residual
        #     step 2 names in its own header.
        if ($null -ne $DepInfoOwners -and $DepInfoOwners.ContainsKey($absolute)) {
            foreach ($manifest in $DepInfoOwners[$absolute]) {
                [void]$manifests.Add([string]$manifest)
                if ($manifestToName.ContainsKey([string]$manifest)) {
                    [void]$packages.Add([string]$manifestToName[[string]$manifest])
                }
                $claimed = $true
            }
        }

        # 3. NEAREST ANCESTOR, longest directory first so a nested package wins.
        foreach ($package in $ordered) {
            if (Test-PathIsInside -Root ([string]$package.Directory) -Path $absolute) {
                [void]$packages.Add([string]$package.Name)
                [void]$manifests.Add([string]$package.Manifest)
                $claimed = $true
                break
            }
        }

        if (-not $claimed) { [void]$unowned.Add($relative) }
    }

    return [ordered]@{
        Packages           = @($packages)
        Manifests          = @($manifests)
        Unowned            = @($unowned)
        WorkspaceWideInput = @($workspaceWide)
    }
}

# The repository-relative paths that are inputs to EVERY cargo package, so a
# change to one of them affects the whole battery and not a computed subset.
# Anchored patterns, matched against forward-slash relative paths.
#
# Lane KV's adversarial review found this hole by name: a change to nothing but
# `.cargo/config.toml` maps to no package, produces an empty affected set, and
# an iteration run would have compiled nothing and reported green.
#
# EVERY ONE IS ANCHORED AT A PATH SEGMENT, NOT AT THE REPOSITORY ROOT. A
# `.cargo/config.toml` NESTED under a cargo root overrides the root one for that
# root alone -- the runner's own target-dir guard says so in as many words -- and
# a nested `rust-toolchain.toml` pins a different compiler for its subtree. A
# root-anchored pattern would call both of those "unowned", and in a MIXED diff
# (one recognised package file plus a nested config) the package's own tests
# would supply a positive census while the rest of that root compiled under
# changed configuration and never ran.
$script:BatteryWorkspaceWideInputPatterns = @(
    '(^|/)\.cargo/',            # target-dir selection, profiles, incremental, linker
    '(^|/)rust-toolchain(\.toml)?$',
    '(^|/)Cargo\.lock$',        # every resolved dependency version
    '(^|/)Cargo\.toml$',        # a manifest edit can move targets, features, members
    '(^|/)\.config/nextest\.toml$'
)

function Test-BatteryPathIsWorkspaceWideInput {
    <#
    .SYNOPSIS
    Is this changed path an input to every cargo package rather than to one?
    #>
    param([Parameter(Mandatory = $true)][string]$Path)

    $normalized = ([string]$Path) -replace '\\', '/'
    foreach ($pattern in $script:BatteryWorkspaceWideInputPatterns) {
        if ($normalized -match $pattern) { return $true }
    }
    return $false
}

function Get-BatterySourceSpliceOwners {
    <#
    .SYNOPSIS
    Which packages pull a file in that does not live in their own directory.

    .DESCRIPTION
    A COMPILE-TIME edge cargo's dependency graph does not have. This repository
    really does this: `map_viewer/tests/xmap00_map_authority.rs` compiles with
    `include_str!("../../replay_app/src/validate.rs")`, and the cargo edge runs
    the OTHER way (replay_app depends on map_viewer), so editing that file
    reaches map_viewer's test through no dependency path at all.

    The scan reads every `.rs` file once - the same walk the marker scan makes -
    and records, for each `include!` / `include_str!` / `include_bytes!` /
    `#[path = ...]` STRING LITERAL, the file it resolves to and the package
    doing the pulling.

    WHY A TEXTUAL SCAN IS ALLOWED HERE WHEN LANE JD PROVED ONE UNSOUND. JD's
    scan DECIDED whether a test could leave the machine-wide mutex, so a missed
    spelling silently removed a serial test's protection. This one only ever
    ADDS packages to an iteration selection. A missed spelling
    (`include!(concat!(..))`, a macro-generated attribute, a literal continued
    across a newline) leaves the selection exactly as wide as it would have been
    without this function; an over-match compiles a package that did not need
    compiling. Neither direction can narrow anything, so the residual is a
    smaller prize rather than a false verdict - and it is named in the run's own
    output, not just here.

    .OUTPUTS
    A hashtable: canonical absolute source path -> HashSet of manifest paths.
    #>
    param(
        [Parameter(Mandatory = $true)][string]$RepoRoot,
        [object[]]$Inventory
    )

    if ($null -eq $Inventory) { $Inventory = @(Get-BatteryPackageInventory -RepoRoot $RepoRoot) }
    # One expression, four spellings, the literal captured in one group. Nothing
    # is stripped from the source first, for the reason the marker scan gives:
    # no regex stripper over Rust can be proven not to delete a line of code.
    #
    # RAW LITERALS COUNT, HASHES INCLUDED. `include_str!(r"../../other/file")`
    # and `#[path = r#"child.rs"#]` are ordinary Rust; a Windows-flavoured path
    # is exactly the case an author reaches for `r"` to write. The hash count
    # is CAPTURED and BACK-REFERENCED (`r(#*)" ... "\1`) rather than merely
    # consumed at the front: a leading-hashes-only pattern matched the opening
    # `r#"` and then left the closing `#` sitting before the `]`, so every
    # hashed `#[path = r#"..."#]` fell out of the scan.
    #
    # Named groups, because four alternatives with anonymous captures made the
    # caller index by position and the indices moved every time the expression
    # was edited. Lazy `.*?` with Singleline for the raw bodies: a raw literal
    # may legally contain a quote, so `[^"]*` is not its content.
    $spliceExpression =
        '(?:include(?:_str|_bytes)?\s*!\s*[\(\[\{]\s*(?:' +
            'r(?<ih>#*)"(?<rawinc>.*?)"\k<ih>' +          # raw literal, hashes matched
            '|"(?<inc>(?:[^"\\]|\\.)*)"' +                # ordinary literal
        ')' +
        '|\#\s*\[\s*path\s*=\s*(?:' +
            'r(?<ph>#*)"(?<rawpath>.*?)"\k<ph>' +
            '|"(?<path>(?:[^"\\]|\\.)*)"' +
        ')\s*\])'

    $owners = @{}
    foreach ($package in $Inventory) {
        $sources = @()
        try {
            $sources = @(Get-ChildItem -LiteralPath $package.Directory -Recurse -File -Filter '*.rs' -ErrorAction Stop |
                    Where-Object {
                        $rel = $_.FullName.Substring(([string]$package.Directory).Length).TrimStart('\', '/')
                        @($rel -split '[\\/]' | Where-Object { $script:BatteryPartitionSkipDirectories -contains $_ }).Count -eq 0
                    })
        }
        catch {
            # ADDITIVE ONLY, so an unreadable directory costs width, not
            # correctness -- but it is never silent.
            Write-Host (
                'splice scan could not list {0} ({1}); iteration selections will not see any file it splices in.' -f `
                    $package.Name, $_.Exception.Message
            )
            continue
        }
        foreach ($file in $sources) {
            $text = $null
            try { $text = Get-Content -LiteralPath $file.FullName -Raw -ErrorAction Stop } catch { $text = $null }
            if ($null -eq $text) { continue }
            foreach ($match in [regex]::Matches($text, $spliceExpression,
                    [Text.RegularExpressions.RegexOptions]::Singleline)) {
                $literal = $null
                $isRaw = $false
                foreach ($group in @('rawinc', 'rawpath')) {
                    if ($match.Groups[$group].Success) { $literal = $match.Groups[$group].Value; $isRaw = $true; break }
                }
                if ($null -eq $literal) {
                    foreach ($group in @('inc', 'path')) {
                        if ($match.Groups[$group].Success) { $literal = $match.Groups[$group].Value; break }
                    }
                }
                if ([string]::IsNullOrWhiteSpace($literal)) { continue }
                # A NON-RAW literal carrying a character escape is not resolved:
                # `\x2e` is a legal spelling of `.` and unescaping it here would
                # be a second Rust lexer. Skipping it costs width only. A RAW
                # literal has no escapes at all, so a backslash in one is just a
                # Windows path separator and must not be skipped -- which is the
                # whole reason an author writes `r"..."` for a path.
                if (-not $isRaw -and $literal -match '\\') { continue }
                $resolved = $null
                try {
                    $resolved = [IO.Path]::GetFullPath((Join-Path $file.DirectoryName $literal))
                }
                catch { continue }
                if (-not (Test-PathIsInside -Root $RepoRoot -Path $resolved)) { continue }
                if (-not $owners.ContainsKey($resolved)) {
                    $owners[$resolved] =
                        [System.Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
                }
                [void]$owners[$resolved].Add([string]$package.Manifest)
            }
        }
    }
    return $owners
}

function Invoke-BatteryCargoCapture {
    <#
    .SYNOPSIS
    Run cargo in a root and return its stdout and exit code, safely.

    .DESCRIPTION
    NOT `& cargo`. PowerShell 5.1 turns a native command's stderr into
    ErrorRecords, and with $ErrorActionPreference = 'Stop' -- which all three
    battery files set -- a cargo run that merely WARNS can terminate the script
    before its exit code is ever read. That turns "cargo succeeded with a
    warning" into a refusal, which is the fail-OPEN-looking failure that is
    actually a false refusal of every mode.

    Start-Process with both streams redirected to files has no such hazard: the
    exit code comes from the process object, and stderr is captured rather than
    interpreted. Shared by every cargo call this file makes for exactly the
    reason the root list is shared -- two spellings of the same invocation drift.
    #>
    param(
        [Parameter(Mandatory = $true)][string]$CargoRoot,
        [Parameter(Mandatory = $true)][string[]]$Arguments
    )

    $stdoutPath = [IO.Path]::GetTempFileName()
    $stderrPath = [IO.Path]::GetTempFileName()
    try {
        $run = Start-Process -FilePath 'cargo' -WorkingDirectory $CargoRoot -NoNewWindow -Wait -PassThru `
            -ArgumentList $Arguments -RedirectStandardOutput $stdoutPath -RedirectStandardError $stderrPath
        # -Encoding UTF8 IS NOT OPTIONAL. Cargo writes UTF-8; PowerShell 5.1's
        # Get-Content defaults to the system ANSI codepage (Windows-1252 on this
        # machine), so a non-ASCII byte in a manifest or artifact path is read
        # as mojibake -- `café` becomes `cafÃ©` -- BEFORE ConvertFrom-Json sees
        # it. The result is a manifest path that matches nothing: a false orphan
        # refusal from the guard, or silently lost ownership in the dep-info
        # layer. Both are wrong answers derived from a correct cargo run.
        $stdout = @()
        if (Test-Path -LiteralPath $stdoutPath) {
            $stdout = @(Get-Content -LiteralPath $stdoutPath -Encoding UTF8)
        }
        $stderr = ''
        if (Test-Path -LiteralPath $stderrPath) {
            $stderr = (Get-Content -LiteralPath $stderrPath -Raw -Encoding UTF8 -ErrorAction SilentlyContinue)
        }
        return [ordered]@{
            ExitCode = [int]$run.ExitCode
            Stdout   = $stdout
            Stderr   = [string]$stderr
        }
    }
    finally {
        Remove-Item -LiteralPath $stdoutPath -Force -ErrorAction SilentlyContinue
        Remove-Item -LiteralPath $stderrPath -Force -ErrorAction SilentlyContinue
    }
}

function Get-BatteryDepInfoSourcePaths {
    <#
    .SYNOPSIS
    The source files one rustc dep-info (`.d`) file names, absolute.

    .DESCRIPTION
    A `.d` is Makefile syntax: `<target>: <prerequisite> <prerequisite> ...`,
    followed by one empty-prerequisite rule per input so a DELETED input does
    not break make. Only the prerequisites are wanted, so the empty rules
    contribute nothing and are skipped by falling out naturally.

    TWO WINDOWS-SPECIFIC HAZARDS, BOTH REAL HERE.

    A drive letter is a colon: `C:\...\x.d: src\lib.rs`. The rule separator is
    therefore the first colon that is NOT the second character of the line, and
    a naive `-split ':'` splits inside the target path instead.

    rustc escapes a space in a path as `\ `, and `\` is also the separator on
    this platform. Splitting on a space preceded by a backslash would break
    `C:\my dir\x.rs` into two nonexistent paths, so the split is on an
    UNESCAPED space and the escape is then removed.

    Paths are relative to the directory rustc ran in, which is the cargo root
    cargo was invoked from -- not the package directory. `-BaseDirectory` is
    that root. An already-absolute prerequisite is left alone, which is what
    [IO.Path]::Combine does with a rooted second argument (Join-Path does not:
    it would produce `C:\root\C:\other`).
    #>
    param(
        [Parameter(Mandatory = $true)][string]$DepInfoPath,
        [Parameter(Mandatory = $true)][string]$BaseDirectory
    )

    $paths = [System.Collections.Generic.List[string]]::new()
    $lines = $null
    # AN UNREADABLE `.d` IS NOT AN EMPTY ONE. Returning @() for both would let a
    # locked or deleted file present as "this target reads no sources", which is
    # indistinguishable from a correct answer. The caller is told.
    # -Encoding UTF8 for the same reason the cargo capture uses it: rustc writes
    # UTF-8 and PowerShell 5.1 would otherwise decode a non-ASCII source path
    # through the ANSI codepage, producing a path that resolves to nothing.
    try { $lines = @(Get-Content -LiteralPath $DepInfoPath -Encoding UTF8 -ErrorAction Stop) }
    catch {
        throw "the dep-info file '$DepInfoPath' could not be read ($($_.Exception.Message)); the files it names cannot be attributed to any package."
    }

    # MAKEFILE LINE CONTINUATIONS, JOINED FIRST. A rule may be wrapped with a
    # trailing backslash, and the continuation line carries no colon -- so the
    # rule-separator scan below would skip it and silently lose every
    # prerequisite after the wrap. The compiler pinned here emits none today
    # (0 of 636 `.d` files in this repository), which is exactly why this must
    # be handled here rather than discovered when it changes.
    $joined = [System.Collections.Generic.List[string]]::new()
    $pending = $null
    foreach ($rawLine in $lines) {
        $current = [string]$rawLine
        if ($null -ne $pending) { $current = $pending + ' ' + $current.TrimStart(); $pending = $null }
        # A trailing backslash continues the line ONLY if it is not itself
        # escaped; `\\` at end of line is a literal backslash.
        if ($current -match '(?<!\\)\\$') { $pending = $current.Substring(0, $current.Length - 1).TrimEnd(); continue }
        $joined.Add($current)
    }
    if ($null -ne $pending) { $joined.Add($pending) }

    foreach ($line in $joined) {
        if ([string]::IsNullOrWhiteSpace($line)) { continue }
        $separator = -1
        for ($index = 0; $index -lt $line.Length; $index++) {
            if ($line[$index] -ne ':') { continue }
            if ($index -eq 1 -and [char]::IsLetter($line[0])) { continue }
            $separator = $index
            break
        }
        if ($separator -lt 0) { continue }
        $prerequisites = $line.Substring($separator + 1)
        foreach ($raw in ($prerequisites -split '(?<!\\) ')) {
            $candidate = ([string]$raw).Trim()
            if ([string]::IsNullOrWhiteSpace($candidate)) { continue }
            $candidate = $candidate -replace '\\ ', ' '
            try { $paths.Add([IO.Path]::GetFullPath([IO.Path]::Combine($BaseDirectory, $candidate))) }
            catch { continue }
        }
    }
    return @($paths)
}

function Select-BatteryFeaturesForRoot {
    <#
    .SYNOPSIS
    The subset of a feature selection that one cargo root can actually resolve.

    .DESCRIPTION
    A FEATURE ONLY HAS TO EXIST IN ITS OWN ROOT. `-Features
    replay_decoder/dq-v3-prototype` is valid in the extractor workspace and
    meaningless in the replay one, which contains no such package -- and cargo
    does not shrug at that, it EXITS 101 with "none of the selected packages
    contains this feature". Passing the caller's whole list to all eleven roots
    therefore turned a legitimate `-Mode Iterate -DepInfo -Features ...` into a
    refusal from the very throw that was added to make failures loud.

    Two spellings, two rules:
      `<package>/<feature>` is kept when that PACKAGE is a member of this root.
      A bare `<feature>` is kept when some member of this root DECLARES it.
    Anything unresolvable here is dropped for this root only, because the root
    that owns it will keep it.

    Dropping is the correct direction: a feature this root cannot resolve gates
    nothing in this root, so no compile-time edge is lost by omitting it.
    #>
    param(
        [Parameter(Mandatory = $true)][AllowEmptyCollection()][string[]]$Features,
        [Parameter(Mandatory = $true)][AllowEmptyCollection()][string[]]$MemberNames,
        [Parameter(Mandatory = $true)][AllowEmptyCollection()][string[]]$DeclaredFeatures
    )

    $members = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    foreach ($name in @($MemberNames)) { [void]$members.Add([string]$name) }
    $declared = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    foreach ($name in @($DeclaredFeatures)) { [void]$declared.Add([string]$name) }

    $kept = [System.Collections.Generic.List[string]]::new()
    foreach ($feature in @($Features)) {
        $spelling = [string]$feature
        if ([string]::IsNullOrWhiteSpace($spelling)) { continue }
        $separator = $spelling.IndexOf('/')
        if ($separator -gt 0) {
            $packageName = $spelling.Substring(0, $separator)
            if ($members.Contains($packageName)) { $kept.Add($spelling) }
            continue
        }
        if ($declared.Contains($spelling)) { $kept.Add($spelling) }
    }
    return @($kept)
}

function Get-BatteryDepInfoPathsForArtifact {
    <#
    .SYNOPSIS
    The dep-info file(s) cargo wrote for one build artifact.

    .DESCRIPTION
    CARGO'S ARTIFACT NAME IS NOT THE DEP-INFO NAME, and three separate
    divergences have to be handled or whole classes of target silently
    contribute nothing:

      1. THE `lib` PREFIX. `libfoo-<hash>.rlib` pairs with `foo-<hash>.d`.

      2. UPLIFTING. A bin's `filenames` name the copy cargo lifted to
         `debug\pack_replay.exe`, while the dep-info stays behind in
         `debug\deps\pack_replay.d`. The `deps` sibling is therefore searched as
         well as the artifact's own directory.

      3. HYPHENS BECOME UNDERSCORES, AND THE HASH COMES BACK. A build script's
         artifact is `build-script-build.exe`; its dep-info is
         `build_script_build-<hash>.d` beside it. So the stem is also tried with
         `-` normalised to `_`, and with a hash suffix.

    RESOLUTION IS EXACT-FIRST, AND AMBIGUITY RESOLVES TO NOTHING. The hashed
    form is only ever needed for an artifact whose own name carries no hash (an
    uplifted bin, a build script); whenever the artifact IS hashed, the exact
    name matches and no guessing happens. So the hashed search runs only if the
    exact one found nothing, and it is accepted ONLY if it matches exactly one
    file.

    That last rule is the difference between a wide answer and a WRONG one. Two
    packages can each produce a `foo` stem, leaving `deps\foo-<hashA>.d` and
    `deps\foo-<hashB>.d`; returning both would attribute one package's sources
    to the other's manifest, which is a false attribution rather than a
    conservative widening. It would also mask a real hole: any same-stem file
    satisfies the caller's "did this target have a `.d`" check, so the
    completeness refusal would stop firing for the target that actually lost
    one. Ambiguity therefore yields NOTHING, and the caller refuses.

    The hash is matched as EXACTLY 16 hex characters, so `foo-*` cannot claim a
    different crate's `foo-bar-<hash>.d` either.
    #>
    param([Parameter(Mandatory = $true)][string]$ArtifactPath)

    $directory = [IO.Path]::GetDirectoryName($ArtifactPath)
    $stem = [IO.Path]::GetFileNameWithoutExtension($ArtifactPath)
    if ([string]::IsNullOrWhiteSpace($directory) -or [string]::IsNullOrWhiteSpace($stem)) { return @() }

    $stems = [System.Collections.Generic.List[string]]::new()
    $stems.Add($stem)
    if ($stem.StartsWith('lib')) { $stems.Add($stem.Substring(3)) }
    foreach ($candidate in @($stems)) {
        $normalised = $candidate -replace '-', '_'
        if ($normalised -cne $candidate) { $stems.Add($normalised) }
    }

    $directories = [System.Collections.Generic.List[string]]::new()
    $directories.Add($directory)
    $depsSibling = Join-Path $directory 'deps'
    if (Test-Path -LiteralPath $depsSibling -PathType Container) { $directories.Add($depsSibling) }

    # PASS 1: the exact name. This is what every hashed artifact hits.
    $found = [System.Collections.Generic.List[string]]::new()
    foreach ($searchDirectory in $directories) {
        foreach ($candidateStem in $stems) {
            $exact = Join-Path $searchDirectory ($candidateStem + '.d')
            if ((Test-Path -LiteralPath $exact -PathType Leaf) -and -not $found.Contains($exact)) {
                $found.Add($exact)
            }
        }
    }
    if ($found.Count -gt 0) { return @($found) }

    # PASS 2: the hashed sibling, for an artifact whose own name has no hash.
    # Enumerated rather than globbed so the 16-hex shape is actually enforced,
    # and accepted only when it is UNAMBIGUOUS.
    $hashedMatches = [System.Collections.Generic.List[string]]::new()
    foreach ($searchDirectory in $directories) {
        foreach ($candidateStem in $stems) {
            $hashed = @()
            try {
                $hashed = @(Get-ChildItem -LiteralPath $searchDirectory -Filter ($candidateStem + '-*.d') `
                        -File -ErrorAction Stop)
            }
            catch { $hashed = @() }
            foreach ($file in $hashed) {
                $fileStem = [IO.Path]::GetFileNameWithoutExtension($file.Name)
                if ($fileStem -notmatch ('^' + [regex]::Escape($candidateStem) + '-[0-9a-f]{16}$')) { continue }
                if (-not $hashedMatches.Contains($file.FullName)) { $hashedMatches.Add($file.FullName) }
            }
        }
    }
    # MORE THAN ONE IS NOT A CHOICE TO MAKE. Two packages producing the same
    # stem would otherwise have one's sources attributed to the other, and any
    # same-stem file would satisfy the caller's completeness check for a target
    # that genuinely lost its dep-info. Returning nothing makes the caller
    # refuse, which is the honest outcome.
    if ($hashedMatches.Count -eq 1) { return @($hashedMatches) }
    return @()
}

function Get-BatteryDepInfoOwners {
    <#
    .SYNOPSIS
    Which package COMPILES each source file, according to the compiler.

    .DESCRIPTION
    THE PRECISION LAYER, AND THE ONLY SOUND ANSWER TO THE COMPUTED LITERAL.

    The lexical splice scan reads `include!`/`include_str!`/`#[path]` STRING
    LITERALS, and names its own residual: a file spliced in by a COMPUTED
    literal -- `include!(concat!(env!("OUT_DIR"), "/x.rs"))`, a macro that
    expands to `#[path = ...]` -- leaves no literal to read. Lane JD proved that
    residual is not closable by adding patterns: it needs macro expansion and a
    real Rust lexer, and a regex that recognises Rust cannot be made sound
    (runs/jd_battery_granularity/WHY_THIS_DID_NOT_LAND.md, 18 findings over 4
    adversarial rounds).

    Cargo and rustc already emit the ground truth. Every compiled unit gets a
    dep-info file listing exactly the source files that went into it, AFTER
    `mod`, `#[path]`, `include!` and macro expansion -- because the compiler did
    the expanding. This function ties each `.d` back to a package by reading the
    `compiler-artifact` events of a compile-only `cargo test --no-run
    --message-format=json` pass, which is JD's design with the provenance its
    own review said it lacked: the event carries `manifest_path`, so the
    attribution is cargo's rather than inferred from a file name.

    ADDITIVE ONLY: it can only ADD packages to an iteration selection, never
    remove one. The lexical scan remains the floor underneath it.

    BUT ADDITIVE IS NOT A LICENCE TO DEGRADE QUIETLY, AND THIS FUNCTION REFUSES
    RATHER THAN SHRINKS. The first version of this treated every failure as a
    printed notice on the reasoning that a missed edge merely costs WIDTH.
    Adversarial review found the hole in that reasoning: the caller asked for
    -DepInfo precisely because the floor is not enough, and in `-Mode Iterate`
    the package this layer would have added is by construction one the run does
    NOT otherwise select -- so the compile failure that hid it is never
    re-encountered, and the run ends GREEN having skipped the consumer the
    changed file actually breaks. "The battery catches it later" is true of
    every mode except the one this layer exists for.

    So a compile-only pass that fails, a `.d` that cannot be read, and an
    artifact with no dep-info at all are all REFUSALS. They mean the layer
    cannot answer the question it was switched on to answer, and a refusal is
    the only outcome that does not quietly hand back the floor while the
    operator believes they got precision.

    WHY IT COMPILES EVERY ROOT AND NOT JUST THE TOUCHED PACKAGES. The question
    is "who splices this changed file in", and the splicer is by definition a
    package the diff did NOT touch. Reading only the touched packages' dep-info
    would answer a different question and find nothing. On a warm target the
    pass is a freshness check that recompiles nothing: 0.36 s for map_viewer,
    measured. On a cold target it is a full compile, which is why this is opt-in
    (`-DepInfo`) rather than the default.

    .OUTPUTS
    A hashtable: canonical absolute source path -> HashSet of manifest paths.
    #>
    param(
        [Parameter(Mandatory = $true)][string]$RepoRoot,
        [string[]]$CargoRoots,
        # THE CALLER'S FEATURE SELECTION, FORWARDED. Without it this pass
        # compiles the DEFAULT feature set while the battery compiles what the
        # caller asked for, and a computed include behind an off-by-default
        # feature is then absent from dep-info exactly when the caller enabled
        # the feature that reaches it -- the one case they would have passed
        # -DepInfo for. Same reason -Features reaches the standalone roots in
        # the runner: a discovery pass on a different configuration answers a
        # different question.
        [string[]]$Features
    )

    if (-not $CargoRoots -or @($CargoRoots).Count -eq 0) {
        $CargoRoots = @(Get-BatteryCargoRootPaths -RepoRoot $RepoRoot)
    }

    $owners = @{}
    foreach ($cargoRoot in $CargoRoots) {
        if (-not (Test-Path -LiteralPath (Join-Path $cargoRoot 'Cargo.toml') -PathType Leaf)) {
            throw (
                "the battery root inventory names '$cargoRoot', which holds no Cargo.toml, so the " +
                'dep-info layer cannot compile it. Every registered root must be a cargo root.'
            )
        }
        $arguments = @('test', '--no-run', '--message-format=json', '--workspace')
        if ($Features -and @($Features).Count -gt 0) {
            # SCOPED TO WHAT THIS ROOT CAN RESOLVE. Cargo exits 101 on a feature
            # naming a package the root does not contain, and the throw below
            # would then refuse a perfectly legitimate invocation.
            $rootMetadata = Invoke-BatteryCargoCapture -CargoRoot $cargoRoot `
                -Arguments @('metadata', '--no-deps', '--format-version', '1', '--offline')
            if ($rootMetadata.ExitCode -ne 0) {
                throw (
                    ("cargo metadata --no-deps failed in {0} (exit {1}) while scoping -Features to " +
                     "that root.{2}cargo stderr:{2}{3}") -f `
                        $cargoRoot, $rootMetadata.ExitCode, [Environment]::NewLine, $rootMetadata.Stderr
                )
            }
            $rootPackages = @((($rootMetadata.Stdout | Out-String) | ConvertFrom-Json).packages)
            $memberNames = @($rootPackages | ForEach-Object { [string]$_.name })
            # Each property is enumerated and its own Name taken, rather than
            # asking a property COLLECTION for `.Name`: a package with no
            # features gives an empty collection, and under Set-StrictMode
            # -Version Latest `.Name` on that is a terminating error.
            $declaredFeatures = [System.Collections.Generic.List[string]]::new()
            foreach ($rootPackage in $rootPackages) {
                if (@($rootPackage.PSObject.Properties.Name) -notcontains 'features') { continue }
                $featureTable = $rootPackage.features
                if ($null -eq $featureTable) { continue }
                foreach ($featureProperty in @($featureTable.PSObject.Properties)) {
                    $declaredFeatures.Add([string]$featureProperty.Name)
                }
            }
            $scopedFeatures = @(Select-BatteryFeaturesForRoot -Features $Features `
                    -MemberNames $memberNames -DeclaredFeatures @($declaredFeatures))
            if ($scopedFeatures.Count -gt 0) {
                $arguments += @('--features', ($scopedFeatures -join ','))
            }
        }
        $compile = Invoke-BatteryCargoCapture -CargoRoot $cargoRoot -Arguments $arguments
        if ($compile.ExitCode -ne 0) {
            throw (
                ('the dep-info compile-only pass in {0} exited {1}. -DepInfo was requested ' +
                 'because the lexical scan is not enough, and the packages this layer would have ' +
                 'added are by construction ones this run does NOT otherwise select -- so ' +
                 'continuing would end green having skipped the very consumers the changed files ' +
                 'reach. Fix the build, or drop -DepInfo to run on the lexical floor alone.' +
                 "{2}cargo stderr:{2}{3}") -f `
                    $cargoRoot, $compile.ExitCode, [Environment]::NewLine, $compile.Stderr
            )
        }
        # Artifacts whose dep-info could not be located, collected across the
        # whole root and reported together rather than one refusal per file.
        $artifactsWithoutDepInfo = [System.Collections.Generic.List[string]]::new()
        foreach ($line in $compile.Stdout) {
            if ([string]::IsNullOrWhiteSpace($line)) { continue }
            $artifactEvent = $null
            try { $artifactEvent = $line | ConvertFrom-Json } catch { continue }
            if ($null -eq $artifactEvent) { continue }
            # PROPERTY PRESENCE IS CHECKED, NOT ASSUMED. Set-StrictMode
            # -Version Latest makes reading an absent property on a
            # ConvertFrom-Json object a terminating error, and cargo's message
            # stream carries several event shapes (build-script-executed,
            # compiler-message, build-finished) that have none of these.
            $eventFields = @($artifactEvent.PSObject.Properties.Name)
            if ($eventFields -notcontains 'reason') { continue }
            if ([string]$artifactEvent.reason -cne 'compiler-artifact') { continue }
            if ($eventFields -notcontains 'manifest_path' -or $eventFields -notcontains 'filenames') { continue }
            $manifest = [string]$artifactEvent.manifest_path
            if ([string]::IsNullOrWhiteSpace($manifest)) { continue }
            $manifest = [IO.Path]::GetFullPath($manifest)
            # ONE `.d` PER ARTIFACT EVENT IS THE COMPLETENESS BAR. An rlib and
            # its rmeta share a single dep-info, so the check is per EVENT
            # rather than per filename; an event that yields none means this
            # target's file set was never read and the layer is incomplete.
            $foundDepInfoForEvent = $false
            foreach ($artifact in @($artifactEvent.filenames)) {
                $artifactPath = [string]$artifact
                if ([string]::IsNullOrWhiteSpace($artifactPath)) { continue }
                foreach ($depInfo in (Get-BatteryDepInfoPathsForArtifact -ArtifactPath $artifactPath)) {
                    $foundDepInfoForEvent = $true
                    foreach ($source in (Get-BatteryDepInfoSourcePaths -DepInfoPath $depInfo -BaseDirectory $cargoRoot)) {
                        if (-not (Test-PathIsInside -Root $RepoRoot -Path $source)) { continue }
                        $relative = $source.Substring($RepoRoot.Length).TrimStart('\', '/')
                        if (@($relative -split '[\\/]' | Where-Object {
                                    $script:BatteryPartitionSkipDirectories -contains $_
                                }).Count -gt 0) { continue }
                        if (-not $owners.ContainsKey($source)) {
                            $owners[$source] =
                                [System.Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
                        }
                        [void]$owners[$source].Add($manifest)
                    }
                }
            }
            # COMPLETENESS IS ASKED ONLY OF THIS REPOSITORY'S OWN PACKAGES.
            # A `cargo test --no-run` over any root also builds the registry
            # dependencies, and their `build-script-build` executables carry no
            # dep-info beside them (74 of them, on the extractor root alone).
            # Their sources are outside the repository, so no changed path can
            # ever resolve to one and a missing `.d` for them costs this
            # derivation nothing. Demanding one made the layer refuse every
            # time it ran -- which is how this bar was found to be mis-scoped.
            if (-not $foundDepInfoForEvent -and (Test-PathIsInside -Root $RepoRoot -Path $manifest)) {
                $targetName = 'unnamed target'
                if ($eventFields -contains 'target') {
                    $targetFields = @($artifactEvent.target.PSObject.Properties.Name)
                    if ($targetFields -contains 'name') { $targetName = [string]$artifactEvent.target.name }
                }
                $artifactsWithoutDepInfo.Add(('{0} ({1})' -f $targetName, $manifest))
            }
        }
        if ($artifactsWithoutDepInfo.Count -gt 0) {
            throw (
                ('the dep-info layer found no `.d` file for {0} compiled target(s) in {1}: {2}. ' +
                 'Cargo reported those targets as built, so their source lists exist somewhere ' +
                 'this derivation did not look -- which means the layer silently covers less than ' +
                 'it claims. Refusing rather than returning a partial map the caller would read ' +
                 'as complete.') -f `
                    $artifactsWithoutDepInfo.Count, $cargoRoot, (($artifactsWithoutDepInfo | Select-Object -First 10) -join '; ')
            )
        }
    }
    return $owners
}

function Get-BatteryDependentClosure {
    <#
    .SYNOPSIS
    The seed packages plus everything that (transitively) depends on them.

    .DESCRIPTION
    THE FIX FOR WHAT `--package X` DOES NOT COMPILE. Cargo's `--package X`
    selects X and builds X's DEPENDENCIES; nothing that IMPORTS X is compiled,
    so a gate run narrowed to a leaf crate never type-checks or tests the crates
    that use it. Measured case: `-Package runtime_text_inputs_live_lane` built
    four packages while five direct importers were never compiled.

    The walk is cargo's own `resolve.nodes` graph and the same fixed point the
    serial half uses: a node is affected if it is a seed, or if ANY of its
    dependencies is affected. Iterated to a fixed point, so A -> B -> seed makes
    A affected as surely as B. All dependency kinds count, dev- and
    build-dependencies included: a dev-dependency is exactly how a crate's TESTS
    reach another crate, and the tests are the point.

    Seeds may be given as NAMES (what `-Package` supplies) or as MANIFEST PATHS
    (what the changed-file map supplies). A name that matches no repository-local
    package in any cargo root is a REFUSAL rather than an empty selection: a
    `-Package` typo must not quietly select nothing and report green.

    -DependencyGraph is injectable so the closure is pinned against hand-built
    graphs, including the A -> B -> C shape a one-hop rule gets wrong and the
    leaves-the-repository-and-returns shape a pruned graph gets wrong.
    #>
    param(
        [Parameter(Mandatory = $true)][string]$RepoRoot,
        [AllowEmptyCollection()][string[]]$SeedPackages = @(),
        [AllowEmptyCollection()][string[]]$SeedManifests = @(),
        [object]$DependencyGraph
    )

    if ($SeedPackages.Count -eq 0 -and $SeedManifests.Count -eq 0) {
        throw (
            'refusing to compute a dependents closure from no seed at all. An ' +
            'empty seed produces an empty selection, and an empty selection is a ' +
            'run that compiles nothing and tests nothing.'
        )
    }
    if ($null -eq $DependencyGraph) {
        $DependencyGraph = Get-BatteryWorkspaceDependencyEdges -RepoRoot $RepoRoot
    }
    $edges = $DependencyGraph.Edges
    $idToName = $DependencyGraph.IdToName
    $idManifests = $DependencyGraph.IdToManifest
    $localIds = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    foreach ($id in $DependencyGraph.LocalIds) { [void]$localIds.Add([string]$id) }

    $seedNames = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    foreach ($name in $SeedPackages) { [void]$seedNames.Add([string]$name) }
    $seedManifestSet = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    foreach ($manifest in $SeedManifests) {
        [void]$seedManifestSet.Add([IO.Path]::GetFullPath([string]$manifest))
    }

    # Seeds are matched only against REPOSITORY-LOCAL ids. `--package` can only
    # ever name a workspace member, and letting a name match an external crate
    # of the same name would seed the closure from a package this repository
    # does not build.
    $affected = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    $matchedNames = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    $matchedManifests = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    foreach ($id in $localIds) {
        $name = [string]$idToName[$id]
        $manifest = [string]$idManifests[$id]
        $isSeed = $false
        if ($seedNames.Contains($name)) { $isSeed = $true; [void]$matchedNames.Add($name) }
        if ($seedManifestSet.Contains($manifest)) { $isSeed = $true; [void]$matchedManifests.Add($manifest) }
        if ($isSeed) { [void]$affected.Add($id) }
    }

    $unmatchedNames = @($SeedPackages | Where-Object { -not $matchedNames.Contains([string]$_) } | Sort-Object -Unique)
    if ($unmatchedNames.Count -gt 0) {
        throw (
            'dependents closure: ' + $unmatchedNames.Count + ' requested package name(s) ' +
            'match no package in any of this repository''s cargo roots: ' +
            ($unmatchedNames -join ', ') + '. Refusing rather than selecting nothing.'
        )
    }
    $unmatchedManifests = @($seedManifestSet |
            Where-Object { -not $matchedManifests.Contains([string]$_) } | Sort-Object -Unique)
    if ($unmatchedManifests.Count -gt 0) {
        throw (
            'dependents closure: ' + $unmatchedManifests.Count + ' seed manifest(s) are ' +
            'not in cargo''s dependency graph: ' + ($unmatchedManifests -join '; ') +
            '. A package no cargo root builds cannot have its dependents computed, ' +
            'and treating it as reaching nothing would hide every importer it has.'
        )
    }

    # THE SAME closure the serial half uses. Not a copy of it.
    $affected = Get-BatteryDependentIdClosure -Edges $edges -SeedIds @($affected)

    $names = [System.Collections.Generic.SortedSet[string]]::new([StringComparer]::Ordinal)
    foreach ($id in $affected) {
        if ($localIds.Contains($id)) { [void]$names.Add([string]$idToName[$id]) }
    }
    # A seed the graph mentions but no `resolve.nodes` entry lists is still
    # affected: it was named, and dropping it would be the one direction this
    # function may never be wrong in.
    foreach ($name in $SeedPackages) { [void]$names.Add([string]$name) }
    return @($names)
}

function Get-BatteryAffectedSelection {
    <#
    .SYNOPSIS
    What `-Mode Iterate` runs: the packages this diff can break.

    .DESCRIPTION
    git diff -> changed paths -> owning packages -> dependents closure. An
    EMPTY change set is a refusal, not a fast green run: an iteration run that
    selects nothing has certified nothing, and the caller asked for a verdict.

    Paths owned by no cargo package (the scripts, the python gates, the
    documents, the evidence trees) are returned separately. They select no cargo
    tests, and the runner's non-cargo gates cover them.
    #>
    param(
        [Parameter(Mandatory = $true)][string]$RepoRoot,
        [Parameter(Mandatory = $true)][string]$Since,
        [object]$DependencyGraph,
        # THE OPTIONAL PRECISION LAYER. Adds cargo's dep-info to the lexical
        # floor, which is the only thing that sees a file spliced in by a
        # computed literal. Additive: it can widen the selection, never narrow
        # it, so a run without it is exactly the run this mode made before.
        [switch]$DepInfo,
        # Injectable so the layer is testable without a compile. Supplying it
        # implies -DepInfo.
        [hashtable]$DepInfoOwners,
        # The caller's feature selection, forwarded to the dep-info compile so
        # it discovers the configuration the battery will actually build.
        [string[]]$Features
    )

    $changed = @(Get-BatteryChangedPaths -RepoRoot $RepoRoot -Since $Since)
    if ($changed.Count -eq 0) {
        throw (
            "no file has changed since '$Since' and the working tree is clean, so " +
            'an iteration run would select no tests at all. That is not a pass. ' +
            'Pass -Since with the commit this lane branched from, or run the full ' +
            'battery.'
        )
    }
    if ($null -eq $DependencyGraph) {
        $DependencyGraph = Get-BatteryWorkspaceDependencyEdges -RepoRoot $RepoRoot
    }
    $inventory = @(Get-BatteryPackageInventory -RepoRoot $RepoRoot)
    # The dep-info layer runs BEFORE the ownership map, because it is one of the
    # map's inputs. An explicitly supplied map implies the switch, so a test can
    # exercise the layer without a compile.
    $depInfoUsed = $false
    if ($null -eq $DepInfoOwners -and $DepInfo) {
        $DepInfoOwners = Get-BatteryDepInfoOwners -RepoRoot $RepoRoot -Features $Features
    }
    if ($null -ne $DepInfoOwners) { $depInfoUsed = $true }
    $owned = Get-BatteryPackagesForPaths -RepoRoot $RepoRoot -Paths $changed -Inventory $inventory `
        -TargetSourceOwners $DependencyGraph.TargetSourceOwners `
        -SpliceOwners (Get-BatterySourceSpliceOwners -RepoRoot $RepoRoot -Inventory $inventory) `
        -DepInfoOwners $DepInfoOwners

    # A WORKSPACE-WIDE INPUT WIDENS TO EVERYTHING. `.cargo/config.toml`, a
    # Cargo.toml, Cargo.lock, the toolchain pin and the nextest config are
    # inputs to every package in the repository, so a diff that touches one has
    # no computable subset: the answer is the whole battery.
    $everything = @($owned.WorkspaceWideInput).Count -gt 0
    if ($everything) {
        $allPackages = [System.Collections.Generic.SortedSet[string]]::new([StringComparer]::Ordinal)
        foreach ($package in $inventory) { [void]$allPackages.Add([string]$package.Name) }
        return [ordered]@{
            ChangedPaths       = $changed
            TouchedPackages    = @($owned.Packages)
            AffectedPackages   = @($allPackages)
            UnownedPaths       = @($owned.Unowned)
            WorkspaceWideInput = @($owned.WorkspaceWideInput)
            SelectsEverything  = $true
            DepInfoUsed        = $depInfoUsed
        }
    }

    $affected = @()
    if (@($owned.Manifests).Count -gt 0) {
        $affected = @(Get-BatteryDependentClosure -RepoRoot $RepoRoot `
                -SeedManifests $owned.Manifests -DependencyGraph $DependencyGraph)
    }
    return [ordered]@{
        ChangedPaths       = $changed
        TouchedPackages    = @($owned.Packages)
        AffectedPackages   = $affected
        UnownedPaths       = @($owned.Unowned)
        WorkspaceWideInput = @()
        SelectsEverything  = $false
        DepInfoUsed        = $depInfoUsed
    }
}

function Get-BatteryCargoRootPaths {
    <#
    .SYNOPSIS
    Every cargo root the battery drives, absolute, in this file's own order.

    .DESCRIPTION
    THE ONLY LIST. Nothing outside this file may restate these directories: a
    list that disagrees with this one is a root the partition classified and
    nobody ran, or the reverse. The runner used to carry three hand-kept copies
    (the target-dir probe, the extractor standalone roots, the portable
    standalone roots); lane LD found `portable_replay\native\terrain_fixture`
    missing from all three at once and derived every one of them from here.
    #>
    param([Parameter(Mandatory = $true)][string]$RepoRoot)

    return @($script:BatteryCargoRoots | ForEach-Object { Join-Path $RepoRoot $_ })
}

function Get-BatteryRegisteredPackageManifests {
    <#
    .SYNOPSIS
    Every package manifest the registered cargo roots have as a MEMBER.

    .DESCRIPTION
    `--no-deps` is what makes this MEMBERSHIP rather than the dependency graph,
    and membership is the whole question: a package that is only a path
    DEPENDENCY of some root is built, never selected, and never tested. It is
    also what `cargo -p` panics on (exit 101, "did not find features for
    (PackageId ...) within activated_features") when asked from a root that
    merely depends on it.

    MANIFEST PATHS, NOT NAMES. Package names are not unique in this repository
    -- the partition says so where it seeds closures on manifest identity -- so
    a name-keyed comparison would let an orphan hide behind a same-named
    package that IS registered.
    #>
    param(
        [Parameter(Mandatory = $true)][string]$RepoRoot,
        [string[]]$CargoRoots
    )

    if (-not $CargoRoots -or @($CargoRoots).Count -eq 0) {
        $CargoRoots = @(Get-BatteryCargoRootPaths -RepoRoot $RepoRoot)
    }
    $members = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    foreach ($cargoRoot in $CargoRoots) {
        if (-not (Test-Path -LiteralPath (Join-Path $cargoRoot 'Cargo.toml') -PathType Leaf)) {
            throw (
                "the battery root inventory names '$cargoRoot', which holds no Cargo.toml. A " +
                'registered root that is not a cargo root cannot be entered, and the packages ' +
                'it was meant to cover would be tested by nothing.'
            )
        }
        # Start-Process rather than `& cargo`, because this runs in EVERY
        # non-Stats mode: under $ErrorActionPreference = 'Stop' a cargo that
        # merely WARNS on stderr can terminate the script before its exit code
        # is read, which would turn a successful metadata call into a refusal of
        # the whole battery.
        $metadataRun = Invoke-BatteryCargoCapture -CargoRoot $cargoRoot `
            -Arguments @('metadata', '--no-deps', '--format-version', '1', '--offline')
        if ($metadataRun.ExitCode -ne 0) {
            throw (
                ("cargo metadata --no-deps failed in $cargoRoot (exit {0}); the battery cannot " +
                 'establish which packages are registered, and assuming none would report every ' +
                 "package in that root as an orphan.{1}cargo stderr:{1}{2}") -f `
                    $metadataRun.ExitCode, [Environment]::NewLine, $metadataRun.Stderr
            )
        }
        $metadata = (($metadataRun.Stdout | Out-String) | ConvertFrom-Json)
        foreach ($package in @($metadata.packages)) {
            $manifestPath = [string]$package.manifest_path
            if ([string]::IsNullOrWhiteSpace($manifestPath)) {
                throw "cargo metadata in $cargoRoot returned a member with no manifest path; membership cannot be established from it."
            }
            [void]$members.Add([IO.Path]::GetFullPath($manifestPath))
        }
    }
    # THE COMMA IS LOAD-BEARING. `return $members` hands a HashSet to the
    # PowerShell pipeline, which ENUMERATES it: the caller gets an Object[] (or,
    # for a single member, the bare string), and `.Contains()` on those is
    # ordinal and case-SENSITIVE -- or, on a string, a SUBSTRING test. Either
    # one silently turns a registered package whose manifest path differs only
    # in casing into an "orphan" and refuses the whole battery. `,` wraps it in
    # a one-element array so the set itself survives the boundary intact.
    return , $members
}

function Assert-BatteryEveryPackageIsRegistered {
    <#
    .SYNOPSIS
    Refuse if any package under the battery trees is a member of no cargo root.

    .DESCRIPTION
    THE ORPHAN GUARD. A package that no registered root has as a member is
    invisible to the entire battery: `--workspace` does not reach it, `-p`
    panics on it, and its `#[cfg(test)]` module has therefore never executed.
    Nothing about that condition is loud on its own -- the battery goes green,
    because green means "everything I selected passed" and the package was never
    selected.

    That is not hypothetical. `portable_terrain_fixture` sat in exactly this
    state from 2026-08-05 to 2026-08-16 with three tests nobody ran, while five
    structurally identical sibling crates were registered; it was found only
    because an iteration selection happened to name it and cargo panicked. This
    assertion is what makes the next one a refusal on the first run instead.

    It is a REFUSAL, not a notice. An earlier version of the runner printed the
    condition and dropped the package, and a notice is precisely how this
    survived for eleven days. The cost of being wrong is one refusal naming the
    crate and the single file that registers it; the cost of dropping is a crate
    whose tests nobody runs and nobody notices.
    #>
    param(
        [Parameter(Mandatory = $true)][string]$RepoRoot,
        [object[]]$Inventory,
        [object]$RegisteredManifests
    )

    if ($null -eq $Inventory) { $Inventory = @(Get-BatteryPackageInventory -RepoRoot $RepoRoot) }
    if ($null -eq $RegisteredManifests) {
        $RegisteredManifests = Get-BatteryRegisteredPackageManifests -RepoRoot $RepoRoot
    }
    if (@($Inventory).Count -eq 0) {
        throw (
            'the battery package inventory is empty, so the orphan guard would pass by ' +
            'having nothing to check. That is the vacuous direction of this assertion and it ' +
            'is refused.'
        )
    }

    $orphans = @($Inventory | Where-Object {
            -not $RegisteredManifests.Contains([IO.Path]::GetFullPath([string]$_.Manifest))
        } | Sort-Object -Property @{ Expression = { [string]$_.Name } })

    if ($orphans.Count -gt 0) {
        $named = @($orphans | ForEach-Object {
                '{0} ({1})' -f [string]$_.Name, [string]$_.Manifest
            }) -join '; '
        throw (
            ('{0} cargo package(s) under the battery trees are a member of NO cargo root: {1}. ' +
             'No battery selects them, `--workspace` does not reach them, and `cargo -p` PANICS ' +
             'on them, so their tests have never run anywhere in this repository. Register each ' +
             'one -- as a member of an existing workspace, or as a cargo root of its own in the ' +
             '$script:BatteryCargoRoots inventory in BatteryPartition.ps1, which is the single ' +
             'list every root is derived from.') -f $orphans.Count, $named
        )
    }
}

function Resolve-BatteryPackageNames {
    <#
    .SYNOPSIS
    Turn caller-supplied `-Package` names into manifest identities, or refuse.

    .DESCRIPTION
    The partition established that a package NAME is not an identity in this
    repository, and seeding a closure from a name would reintroduce exactly the
    ambiguity the manifest path was adopted to remove: two same-named packages
    in different roots expand two different closures, and which one the caller
    meant is not recoverable.

    Resolution is scoped to the cargo roots the run will actually enter, so
    `-Workspace Replay -Package <an extractor crate>` is a REFUSAL here rather
    than an empty selection three hundred lines later. A name matching more than
    one manifest inside that scope is also a refusal: picking either would be a
    guess.
    #>
    param(
        [Parameter(Mandatory = $true)][string]$RepoRoot,
        [Parameter(Mandatory = $true)][AllowEmptyCollection()][string[]]$Names,
        [Parameter(Mandatory = $true)][AllowEmptyCollection()][string[]]$ScopePackages,
        [Parameter(Mandatory = $true)][object]$DependencyGraph,
        # The directories of the cargo roots this run enters. A candidate id
        # whose manifest is outside all of them is not a candidate: without this
        # the search ran over EVERY repository-local id, so a same-named package
        # in a root the caller excluded made an otherwise unambiguous name fail
        # as ambiguous.
        [AllowEmptyCollection()][string[]]$ScopeRoots = @()
    )

    $scope = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    foreach ($name in $ScopePackages) { [void]$scope.Add([string]$name) }

    $idToName = $DependencyGraph.IdToName
    $idManifests = $DependencyGraph.IdToManifest
    $manifests = [System.Collections.Generic.SortedSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    $missing = [System.Collections.Generic.List[string]]::new()
    $ambiguous = [System.Collections.Generic.List[string]]::new()

    foreach ($name in $Names) {
        if (-not $scope.Contains([string]$name)) { $missing.Add([string]$name); continue }
        $matches = [System.Collections.Generic.SortedSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
        foreach ($id in $DependencyGraph.LocalIds) {
            if ([string]$idToName[[string]$id] -cne [string]$name) { continue }
            $manifest = [string]$idManifests[[string]$id]
            if (@($ScopeRoots).Count -gt 0) {
                $inScope = $false
                foreach ($root in $ScopeRoots) {
                    if (Test-PathIsInside -Root ([string]$root) -Path $manifest) { $inScope = $true; break }
                }
                if (-not $inScope) { continue }
            }
            [void]$matches.Add($manifest)
        }
        if ($matches.Count -eq 0) { $missing.Add([string]$name); continue }
        if ($matches.Count -gt 1) {
            $ambiguous.Add(('{0} ({1})' -f $name, ((@($matches)) -join '; ')))
            continue
        }
        foreach ($manifest in $matches) { [void]$manifests.Add([string]$manifest) }
    }

    if ($missing.Count -gt 0) {
        throw (
            '-Package names ' + $missing.Count + ' package(s) that no cargo root this ' +
            'run enters contains: ' + (@($missing) -join ', ') + '. Refusing rather ' +
            'than running a narrowed selection that silently covers none of them; ' +
            'check the name, or widen -Workspace.'
        )
    }
    if ($ambiguous.Count -gt 0) {
        throw (
            '-Package names ' + $ambiguous.Count + ' package(s) that resolve to more than ' +
            'one manifest: ' + (@($ambiguous) -join ' | ') + '. Package names are not ' +
            'unique in this repository and picking one would be a guess.'
        )
    }
    return @($manifests)
}

function Write-BatteryAffectedSelectionSummary {
    <#
    .SYNOPSIS
    Print what the iteration selection is and, just as importantly, is not.
    #>
    param([Parameter(Mandatory = $true)][object]$Selection)

    Write-Host (
        'Iteration selection: {0} changed path(s) -> {1} touched package(s) -> {2} affected package(s) (touched plus their dependents).' -f `
            @($Selection.ChangedPaths).Count, @($Selection.TouchedPackages).Count, @($Selection.AffectedPackages).Count
    )
    if ($Selection.SelectsEverything) {
        Write-Host (
            '  WIDENED TO EVERY PACKAGE: {0} changed path(s) are inputs to every cargo package ({1}).' -f `
                @($Selection.WorkspaceWideInput).Count, (@($Selection.WorkspaceWideInput) -join ', ')
        )
    }
    if (@($Selection.TouchedPackages).Count -gt 0) {
        Write-Host ('  touched:  {0}' -f (@($Selection.TouchedPackages) -join ', '))
    }
    $dependents = @($Selection.AffectedPackages | Where-Object { $Selection.TouchedPackages -notcontains $_ })
    if ($dependents.Count -gt 0) {
        Write-Host ('  dependents added: {0}' -f ($dependents -join ', '))
    }
    if (@($Selection.UnownedPaths).Count -gt 0) {
        # NOT "the gates cover them". The runner's python and PowerShell gates
        # are workspace-conditional, and they are not derived from this diff at
        # all -- they run because the mode runs them. Saying they "cover" an
        # unowned path would be a claim this function cannot make and the
        # earlier wording made it. Iterate requires -Workspace All precisely so
        # that BOTH workspaces' gates do run.
        # The concatenation is PARENTHESISED before -f. Without the parentheses
        # PowerShell binds -f to the LAST fragment only, and the message prints
        # a literal `{0}` -- which is how this was found.
        Write-Host ((
                '  {0} changed path(s) belong to no cargo package (scripts, gates, documents, evidence). They select NO ' +
                'cargo test. Both workspaces'' non-cargo gates run unconditionally in this mode, but a Rust test that ' +
                'reads one of these files at run time is not reachable from any dependency graph.'
            ) -f @($Selection.UnownedPaths).Count)
    }
    # THE LIMITS OF THE SELECTION, PRINTED WITH THE SELECTION. An iteration run
    # is narrower than the battery by design and by the owner's ruling; what it
    # may never be is narrower than the operator believes. These are the classes
    # this derivation cannot see, and they are the reason the full battery still
    # runs at every landing.
    # THE COMPUTED-LITERAL CLAUSE IS CONDITIONAL, because with the dep-info
    # layer on it is no longer true: the compiler resolved the literal and the
    # `.d` names the file. Printing a limit the run does not have would be the
    # same defect as hiding one it does.
    # THE SELECTION IS AN [ordered] DICTIONARY, NOT A PSCustomObject, so its
    # KEYS are what must be probed. `$Selection.PSObject.Properties.Name` lists
    # the dictionary's own .NET members (Count, Keys, Values, IsReadOnly) and
    # never contains a key at all -- so that probe was always false, and this
    # summary printed the computed-literal limitation even on runs that had
    # closed it. Property-style READ ($Selection.DepInfoUsed) works on an
    # ordered dictionary; only the existence test had to change.
    $depInfoUsed = $false
    if (@($Selection.Keys) -contains 'DepInfoUsed') {
        $depInfoUsed = [bool]$Selection.DepInfoUsed
    }
    $limits = @(
        'a test that READS or SHELLS OUT to another package at run time',
        'a build script or proc macro reading a file cargo records no edge for'
    )
    if (-not $depInfoUsed) {
        $limits += 'a file spliced in by a computed literal (concat!/a macro-generated #[path])'
    }
    Write-Host (
        ('  NOT A LANDING GATE. This selection cannot see: {0}. It also does not run DOCTESTS, ' +
         'the release detour-identity gate, packaging, receipt, or live validation. Run ' +
         'Invoke-Validation.ps1 -Mode Landing before landing.') -f ($limits -join '; ')
    )
    if ($depInfoUsed) {
        Write-Host (
            '  DEP-INFO PRECISION LAYER ON: every file the COMPILER read for a target was ' +
            'attributed to that target''s package, so files spliced in by computed literals ' +
            '(concat!, macro-generated #[path]) are covered. The layer is additive -- it widened ' +
            'this selection or left it unchanged, never narrowed it.'
        )
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
