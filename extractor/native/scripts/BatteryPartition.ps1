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
            'contains, or run the full battery.'
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
        [hashtable]$SpliceOwners
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
        [object]$DependencyGraph
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
    $owned = Get-BatteryPackagesForPaths -RepoRoot $RepoRoot -Paths $changed -Inventory $inventory `
        -TargetSourceOwners $DependencyGraph.TargetSourceOwners `
        -SpliceOwners (Get-BatterySourceSpliceOwners -RepoRoot $RepoRoot -Inventory $inventory)

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
    }
}

function Get-BatteryCargoRootPaths {
    <#
    .SYNOPSIS
    Every cargo root the battery drives, absolute, in this file's own order.

    .DESCRIPTION
    Exported so nothing outside this file writes a FOURTH list of the same nine
    directories. The runner already carries three (the target-dir probe, the two
    workspace roots, the standalone roots) and a list that disagrees with this
    one is a root the partition classified and nobody ran, or the reverse.
    #>
    param([Parameter(Mandatory = $true)][string]$RepoRoot)

    return @($script:BatteryCargoRoots | ForEach-Object { Join-Path $RepoRoot $_ })
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
    Write-Host (
        '  NOT A LANDING GATE. This selection cannot see: a test that READS or SHELLS OUT to ' +
        'another package at run time; a build script or proc macro reading a file cargo records ' +
        'no edge for; a file spliced in by a computed literal (concat!/a macro-generated #[path]). ' +
        'It also does not run DOCTESTS, the release detour-identity gate, or the portable package ' +
        'self-test. Run -Mode Final -Workspace All before landing.'
    )
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
