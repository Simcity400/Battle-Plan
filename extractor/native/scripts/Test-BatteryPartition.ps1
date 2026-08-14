# Source tests for the central battery's serial/parallel partition.
#
# The partition decides which tests keep the machine-wide mutex and which run
# without it. Every case is annotated with the production change it kills,
# because the only interesting failure mode here is silent: a partition that
# classifies too much as parallel does not error, it produces a flake somewhere
# else, weeks later, in someone else's lane.
#
# Emits one JSON authority line last, like the other source tests the central
# runner consumes.

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

. (Join-Path $PSScriptRoot 'BatteryPartition.ps1')

$script:Cases = 0
$script:Failures = [System.Collections.Generic.List[string]]::new()

function Assert-Case {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][bool]$Condition
    )
    $script:Cases++
    if (-not $Condition) {
        $script:Failures.Add($Name)
        Write-Host "FAIL: $Name"
    }
}

function Assert-Throws {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][scriptblock]$Action,
        [Parameter(Mandatory = $true)][string]$MessageContains
    )
    $script:Cases++
    $threw = $false
    $message = ''
    # $null = so a non-throwing action's return value does not land on this
    # script's output stream, where the JSON authority line lives.
    try { $null = & $Action } catch { $threw = $true; $message = [string]$_.Exception.Message }
    if (-not $threw) {
        $script:Failures.Add("$Name (did not throw)")
        Write-Host "FAIL: $Name (did not throw)"
        return
    }
    if ($message -notlike "*$MessageContains*") {
        $script:Failures.Add("$Name (wrong message: $message)")
        Write-Host "FAIL: $Name (wrong message: $message)"
    }
}

function New-SyntheticTree {
    <#
    .SYNOPSIS
    A throwaway repository root holding exactly the packages a case needs.

    .DESCRIPTION
    Every tree gets an ANCHOR package carrying one marker unless -NoAnchor is
    passed. Without it, any tree built to test "this is NOT a marker" would trip
    the empty-owner refusal instead of testing what it meant to, and the case
    would be passing for the wrong reason.
    #>
    param(
        [Parameter(Mandatory = $true)][hashtable]$Packages,
        [switch]$NoAnchor
    )

    $root = Join-Path ([IO.Path]::GetTempPath()) ("hoi4_partition_test_{0}_{1}" -f $PID, [guid]::NewGuid().ToString('N'))
    foreach ($tree in @('extractor\native', 'replay_interface\native', 'portable_replay\native')) {
        New-Item -ItemType Directory -Path (Join-Path $root $tree) -Force | Out-Null
    }
    if (-not $NoAnchor) {
        $Packages = $Packages.Clone()
        $Packages['partition_test_anchor'] = 'pub fn f() { unsafe { CreateMutexW(a, b, c) }; }'
    }
    foreach ($entry in $Packages.GetEnumerator()) {
        $dir = Join-Path $root ('extractor\native\' + $entry.Key)
        New-Item -ItemType Directory -Path (Join-Path $dir 'src') -Force | Out-Null
        Set-Content -LiteralPath (Join-Path $dir 'Cargo.toml') -Encoding utf8 -Value @"
[package]
name = "$($entry.Key)"
version = "0.1.0"
"@
        Set-Content -LiteralPath (Join-Path $dir 'src\lib.rs') -Encoding utf8 -Value $entry.Value
    }
    return $root
}

function New-Edges {
    <#
    .SYNOPSIS
    A hand-built dependency graph, so the closure is tested without cargo.

    .DESCRIPTION
    Package ids are the names here, which is all the closure needs; the
    id-vs-name distinction is exercised against REAL cargo metadata by the
    synthetic-workspace case below, not here.
    #>
    param(
        [Parameter(Mandatory = $true)][hashtable]$Map,
        [Parameter(Mandatory = $true)][string]$Root,
        [string[]]$External = @()
    )
    $edges = @{}
    $idToName = @{}
    $idToManifest = @{}
    $local = [System.Collections.Generic.List[string]]::new()
    foreach ($entry in $Map.GetEnumerator()) {
        $set = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
        foreach ($d in @($entry.Value)) { [void]$set.Add($d) }
        $edges[$entry.Key] = $set
        $idToName[$entry.Key] = $entry.Key
        # Manifest paths mirror where New-SyntheticTree writes each package, so
        # the identity reconciliation sees the same paths the scan did.
        $idToManifest[$entry.Key] = if ($External -contains $entry.Key) {
            Join-Path ([IO.Path]::GetTempPath()) ("outside_the_repo_{0}\Cargo.toml" -f $entry.Key)
        }
        else {
            Join-Path $Root ("extractor\native\{0}\Cargo.toml" -f $entry.Key)
        }
        if ($External -notcontains $entry.Key) { $local.Add($entry.Key) }
    }
    return [ordered]@{ Edges = $edges; IdToName = $idToName; IdToManifest = $idToManifest; LocalIds = @($local) }
}

function New-SyntheticCargoWorkspace {
    <#
    .SYNOPSIS
    A real cargo workspace on disk, so the metadata reader is exercised.

    .DESCRIPTION
    Path dependencies only, so `cargo metadata --offline` needs no registry.
    Carries the shapes a name-and-text reader gets wrong: a RENAMED dependency
    (`renamed = { path = "..", package = "facility_owner" }`), a DEV-dependency
    (which is how a crate's tests reach another crate), and -- when -BridgeRoot
    is given -- a package OUTSIDE the repository root that the workspace depends
    on and which depends back in on the facility owner. That last one is the
    shape a metadata reader that prunes non-local nodes severs.
    #>
    param(
        [Parameter(Mandatory = $true)][string]$Root,
        [string]$BridgeRoot
    )

    $members = @('facility_owner', 'renames_it', 'dev_depends_on_it', 'unrelated_leaf')
    if ($BridgeRoot) { $members += 'reaches_through_the_bridge' }
    New-Item -ItemType Directory -Path (Join-Path $Root 'extractor\native') -Force | Out-Null
    New-Item -ItemType Directory -Path (Join-Path $Root 'replay_interface\native') -Force | Out-Null
    New-Item -ItemType Directory -Path (Join-Path $Root 'portable_replay\native') -Force | Out-Null
    Set-Content -LiteralPath (Join-Path $Root 'extractor\native\Cargo.toml') -Encoding utf8 -Value (
        "[workspace]`nresolver = `"2`"`nmembers = [" +
        (($members | ForEach-Object { "`"$_`"" }) -join ', ') + "]`n")
    $bodies = @{
        facility_owner    = 'pub fn freeze_process(h: u32) -> u32 { unsafe { SuspendThread(h) } }'
        renames_it        = 'pub fn f() {}'
        dev_depends_on_it = 'pub fn f() {}'
        unrelated_leaf    = 'pub fn f() -> u32 { 1 }'
    }
    $manifests = @{
        facility_owner    = ''
        renames_it        = "[dependencies]`nrenamed = { path = `"../facility_owner`", package = `"facility_owner`" }`n"
        dev_depends_on_it = "[dev-dependencies]`nfacility_owner = { path = `"../facility_owner`" }`n"
        unrelated_leaf    = ''
    }
    if ($BridgeRoot) {
        # The bridge lives OUTSIDE $Root and depends back into it, so the only
        # path from `reaches_through_the_bridge` to the facility owner leaves
        # the repository and returns.
        $bridgeDir = Join-Path $BridgeRoot 'outside_bridge'
        New-Item -ItemType Directory -Path (Join-Path $bridgeDir 'src') -Force | Out-Null
        $ownerManifestDir = (Join-Path $Root 'extractor\native\facility_owner') -replace '\\', '/'
        Set-Content -LiteralPath (Join-Path $bridgeDir 'Cargo.toml') -Encoding utf8 -Value (
            "[package]`nname = `"outside_bridge`"`nversion = `"0.1.0`"`nedition = `"2021`"`n`n" +
            "[dependencies]`nfacility_owner = { path = `"$ownerManifestDir`" }`n")
        Set-Content -LiteralPath (Join-Path $bridgeDir 'src\lib.rs') -Encoding utf8 -Value 'pub fn f() {}'
        $bridgePath = $bridgeDir -replace '\\', '/'
        $bodies['reaches_through_the_bridge'] = 'pub fn f() {}'
        $manifests['reaches_through_the_bridge'] =
            "[dependencies]`noutside_bridge = { path = `"$bridgePath`" }`n"
    }
    foreach ($member in $members) {
        $dir = Join-Path $Root "extractor\native\$member"
        New-Item -ItemType Directory -Path (Join-Path $dir 'src') -Force | Out-Null
        Set-Content -LiteralPath (Join-Path $dir 'Cargo.toml') -Encoding utf8 -Value (
            "[package]`nname = `"$member`"`nversion = `"0.1.0`"`nedition = `"2021`"`n`n" + $manifests[$member])
        Set-Content -LiteralPath (Join-Path $dir 'src\lib.rs') -Encoding utf8 -Value $bodies[$member]
    }
}

# ==========================================================================
# GROUP 1: ownership -- one case per marker
# ==========================================================================

# kills: dropping ANY ONE marker from $script:BatteryContentionMarkers. A single
# CreateMutexW case used to stand for all eight, which meant deleting
# `affinity_priority` left every case green while replay_decoder's
# SetThreadPriority use moved into the parallel half.
$markerCases = @(
    @{ Marker = 'named_kernel_object'; Package = 'marker_named_object'
        Source = 'pub fn f() { unsafe { CreateMutexW(std::ptr::null_mut(), 0, n) }; }'
        Site = 'extractor/native/recorder_host/src/auto_attach/singleton.rs' },
    @{ Marker = 'machine_namespace_literal'; Package = 'marker_namespace'
        Source = 'pub const N: &str = "Global\\Hoi4Replay.ObserverLease";'
        Site = 'extractor/native/recorder_host/src/main.rs' },
    @{ Marker = 'system_enumeration'; Package = 'marker_enumeration'
        Source = 'pub fn f() { unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) }; }'
        Site = 'extractor/native/inline_detour/src/freeze.rs' },
    @{ Marker = 'thread_suspension'; Package = 'marker_suspension'
        Source = 'pub fn f(h: Handle) { unsafe { SuspendThread(h) }; }'
        Site = 'extractor/native/inline_detour/src/freeze.rs' },
    @{ Marker = 'registry'; Package = 'marker_registry'
        Source = 'pub fn f() { unsafe { RegOpenKeyExW(HKEY_LOCAL_MACHINE, k, 0, a, o) }; }'
        Site = 'extractor/native/recorder_host/src' },
    @{ Marker = 'network_endpoint'; Package = 'marker_socket'
        Source = 'pub fn f() -> std::io::Result<()> { let _l = TcpListener::bind("127.0.0.1:0")?; Ok(()) }'
        Site = 'a port is machine-scoped wherever it is bound' },
    @{ Marker = 'process_spawn'; Package = 'marker_spawn'
        Source = 'pub fn f() { let _ = Command::new("python").status(); }'
        Site = 'extractor/native/wire_format/src/bin/bench_gaps.rs' },
    @{ Marker = 'affinity_priority'; Package = 'marker_priority'
        Source = 'pub fn f(h: Handle) { unsafe { SetThreadPriority(h, 1) }; }'
        Site = 'extractor/native/replay_decoder/src/lib.rs' }
)
foreach ($case in $markerCases) {
    $tree = New-SyntheticTree -NoAnchor -Packages @{
        $case.Package = $case.Source
        # A companion that must NOT be dragged in, so a marker whose pattern has
        # widened into something that matches everything fails here too.
        pure_arithmetic_companion = 'pub fn g(a: u32, b: u32) -> u32 { a.wrapping_add(b) }'
    }
    try {
        $ownership = Get-BatteryFacilityOwners -RepoRoot $tree
        Assert-Case ("marker {0} makes its package a facility owner (real site: {1})" -f $case.Marker, $case.Site) (
            ($ownership.Owners -contains $case.Package) -and
            (@($ownership.Evidence[$case.Package].Keys) -contains $case.Marker))
        Assert-Case ("marker {0} does not claim an unrelated package" -f $case.Marker) (
            $ownership.Owners -notcontains 'pure_arithmetic_companion')
    }
    finally { Remove-Item -LiteralPath $tree -Recurse -Force -ErrorAction SilentlyContinue }
}

# kills: reintroducing ANY comment stripper. Two were tried and both were
# fail-open: `///.*$` anywhere on a line erased everything after a string
# containing `file:///x`, and an unanchored `/* ... */` paired an opener inside
# one string literal with a closer inside a later one. Rust raw strings can
# legally begin a physical line with `/*` or `//`, so no regex stripper is safe.
# These three sources must ALL make their package an owner.
$strippableShapes = @{
    marker_after_double_slash_string = 'pub fn f() { let u = "https://x"; unsafe { CreateMutexW(p, 0, n) }; } // trailing'
    marker_after_triple_slash_string = 'pub fn f() { let u = "file:///x"; unsafe { CreateMutexW(p, 0, n) }; }'
    marker_inside_raw_string_block   = @'
pub const SNIPPET: &str = r#"
/* a line that begins a block comment, inside a raw string
"#;
pub fn f() { unsafe { CreateMutexW(p, 0, n) }; }
pub const CLOSER: &str = r#"
*/
"#;
'@
    # The marker is on the SAME physical line as the raw string's interior `//`
    # and its closing delimiter. A whole-line `(?m)^[ \t]*//.*$` stripper erases
    # that line and with it the call. Putting the marker on a later line, as an
    # earlier version of this case did, made it vacuous: the stripper would have
    # left it visible and the case would have passed against the very rule it
    # exists to forbid.
    marker_after_raw_string_slashes  = @'
pub const SNIPPET: &str = r#"
// interior comment line "#; pub fn f() { unsafe { CreateMutexW(p, 0, n) }; }
'@
}
foreach ($shape in $strippableShapes.GetEnumerator()) {
    $tree = New-SyntheticTree -NoAnchor -Packages @{ $shape.Key = $shape.Value }
    try {
        $ownership = Get-BatteryFacilityOwners -RepoRoot $tree
        Assert-Case ("a marker survives the shape '{0}' (nothing is stripped)" -f $shape.Key) (
            $ownership.Owners -contains $shape.Key)
    }
    finally { Remove-Item -LiteralPath $tree -Recurse -Force -ErrorAction SilentlyContinue }
}

# kills: the unanchored `(Global|Local)\\` pattern. It matched the `Local\` of
# every C:\Users\Example\AppData\Local\... path in the tree.
$tree = New-SyntheticTree -Packages @{
    local_appdata_path = 'pub const P: &str = "C:\\Users\\Example\\AppData\\Local\\hoi4_captures\\s1";'
}
try {
    $ownership = Get-BatteryFacilityOwners -RepoRoot $tree
    Assert-Case 'an AppData\Local path is not a kernel namespace' (
        $ownership.Owners -notcontains 'local_appdata_path')
}
finally { Remove-Item -LiteralPath $tree -Recurse -Force -ErrorAction SilentlyContinue }

# ==========================================================================
# GROUP 2: reach -- the dependency closure
# ==========================================================================

# kills: THE cross-crate fail-open, and the ONE-HOP version of its fix.
# A lane crate whose own source names no Windows API still suspends every thread
# in its process when it calls inline_detour::freeze_process(); and a package two
# removes away reaches it just as surely. A one-hop rule left `two_removes`
# parallel. Also kills every text-based reach rule: `aliased_call` reaches the
# owner through `use owner::f as g`, which no `owner::f` regex can see, and the
# dependency graph does not care how the call is spelled.
$tree = New-SyntheticTree -NoAnchor -Packages @{
    facility_owner = 'pub fn freeze_process(h: u32) -> u32 { unsafe { SuspendThread(h) } }'
    one_hop        = 'pub fn f() { let _ = facility_owner::freeze_process(0); }'
    two_removes    = 'pub fn g() { one_hop::f(); }'
    aliased_call   = "use facility_owner::freeze_process as fp;`npub fn h() { let _ = fp(0); }"
    unrelated_leaf = 'pub fn k(a: u32) -> u32 { a + 1 }'
}
try {
    $edges = New-Edges -Root $tree -Map @{
        facility_owner = @()
        one_hop        = @('facility_owner')
        two_removes    = @('one_hop')
        aliased_call   = @('facility_owner')
        unrelated_leaf = @()
    }
    $partition = Get-BatteryPartition -RepoRoot $tree -DependencyGraph $edges
    Assert-Case 'a direct dependent of a facility owner is serial' ($partition.Serial -contains 'one_hop')
    Assert-Case 'a dependent TWO removes from a facility owner is serial (fixed point, not one hop)' (
        $partition.Serial -contains 'two_removes')
    Assert-Case 'a renamed import reaches the owner because the DEPENDENCY does, not the spelling' (
        $partition.Serial -contains 'aliased_call')
    Assert-Case 'a package that depends on no owner stays parallel' (
        $partition.Parallel -contains 'unrelated_leaf')
    Assert-Case 'the owner itself is serial' ($partition.Serial -contains 'facility_owner')
    Assert-Case 'the owner list is the raw-marker set, not the closure' (
        (@($partition.Owners).Count -eq 1) -and ($partition.Owners -contains 'facility_owner'))
}
finally { Remove-Item -LiteralPath $tree -Recurse -Force -ErrorAction SilentlyContinue }

# kills: excluding process_spawn from propagation. A public wrapper around
# `Command::new(..).status()` performs exactly the child-process operation that
# made its owner serial, so its callers are serial too.
$tree = New-SyntheticTree -NoAnchor -Packages @{
    spawn_owner  = 'pub fn run_python() { let _ = Command::new("python").status(); }'
    spawn_caller = 'pub fn f() { spawn_owner::run_python(); }'
    spawn_unrelated = 'pub fn g() -> u32 { 3 }'
}
try {
    $edges = New-Edges -Root $tree -Map @{ spawn_owner = @(); spawn_caller = @('spawn_owner'); spawn_unrelated = @() }
    $partition = Get-BatteryPartition -RepoRoot $tree -DependencyGraph $edges
    Assert-Case 'a caller of a child-process wrapper is serial' ($partition.Serial -contains 'spawn_caller')
    Assert-Case 'a package unrelated to the spawn owner stays parallel' (
        $partition.Parallel -contains 'spawn_unrelated')
}
finally { Remove-Item -LiteralPath $tree -Recurse -Force -ErrorAction SilentlyContinue }

# kills: a closure that stops iterating on the first pass. Ordering matters: if
# the fixed-point loop visits `c` before `b`, a single pass leaves `c` parallel.
$tree = New-SyntheticTree -NoAnchor -Packages @{
    zzz_owner = 'pub fn f(h: u32) -> u32 { unsafe { SuspendThread(h) } }'
    aaa_far   = 'pub fn f() {}'
    mmm_mid   = 'pub fn f() {}'
}
try {
    $edges = New-Edges -Root $tree -Map @{ zzz_owner = @(); mmm_mid = @('zzz_owner'); aaa_far = @('mmm_mid') }
    $partition = Get-BatteryPartition -RepoRoot $tree -DependencyGraph $edges
    Assert-Case 'the closure is order-independent (a far package visited first is still reached)' (
        ($partition.Serial -contains 'aaa_far') -and ($partition.Serial -contains 'mmm_mid'))
}
finally { Remove-Item -LiteralPath $tree -Recurse -Force -ErrorAction SilentlyContinue }

# kills: pruning the graph to repository-local nodes BEFORE computing reach. A
# path that leaves the repository and comes back --
# `caller -> external_bridge -> facility_owner` -- loses both of its edges to
# that pruning, and `caller` is then classified parallel while transitively
# reaching a facility. The closure runs over the whole graph and narrows only
# its result.
$tree = New-SyntheticTree -NoAnchor -Packages @{
    facility_owner = 'pub fn f(h: u32) -> u32 { unsafe { SuspendThread(h) } }'
    caller         = 'pub fn f() {}'
    unrelated_leaf = 'pub fn f() -> u32 { 1 }'
}
try {
    $graph = New-Edges -Root $tree -External @('external_bridge') -Map @{
        facility_owner = @()
        external_bridge = @('facility_owner')
        caller         = @('external_bridge')
        unrelated_leaf = @()
    }
    $partition = Get-BatteryPartition -RepoRoot $tree -DependencyGraph $graph
    Assert-Case 'reach through a node OUTSIDE the repository is still reach' (
        $partition.Serial -contains 'caller')
    Assert-Case 'a package outside the repository is not reported as a battery package' (
        ($partition.Serial -notcontains 'external_bridge') -and
        ($partition.Parallel -notcontains 'external_bridge'))
    Assert-Case 'a package with no path to an owner is still parallel' (
        $partition.Parallel -contains 'unrelated_leaf')
}
finally { Remove-Item -LiteralPath $tree -Recurse -Force -ErrorAction SilentlyContinue }

# kills: every way Get-BatteryWorkspaceDependencyEdges itself can break. Every
# other closure case injects a hand-built graph, so manifest containment, the
# resolve-graph read, dependency KINDS and renamed dependencies could all rot
# while the suite stayed green. This one drives REAL `cargo metadata` over a
# real workspace on disk.
#
#  * `renames_it` depends on facility_owner under the name `renamed`. A reader
#    that matched dependency NAMES against package names would miss it entirely.
#  * `dev_depends_on_it` reaches the owner only through a dev-dependency, which
#    is exactly how a crate's TESTS reach another crate -- and the tests are
#    what is being partitioned.
#
# It also carries the external bridge END TO END, through the real metadata
# reader: `reaches_through_the_bridge -> outside_bridge (outside the repository
# root) -> facility_owner`. The injected-graph case elsewhere proves the CLOSURE
# follows such a path; only this one proves the READER does not sever it, so
# reintroducing local-node pruning inside Get-BatteryWorkspaceDependencyEdges
# fails here rather than nowhere.
$cargoTree = Join-Path ([IO.Path]::GetTempPath()) ("hoi4_partition_cargo_{0}_{1}" -f $PID, [guid]::NewGuid().ToString('N'))
$bridgeTree = Join-Path ([IO.Path]::GetTempPath()) ("hoi4_partition_bridge_{0}_{1}" -f $PID, [guid]::NewGuid().ToString('N'))
try {
    New-Item -ItemType Directory -Path $bridgeTree -Force | Out-Null
    New-SyntheticCargoWorkspace -Root $cargoTree -BridgeRoot $bridgeTree
    $savedRoots = $script:BatteryCargoRoots
    try {
        $script:BatteryCargoRoots = @('extractor\native')
        $graph = Get-BatteryWorkspaceDependencyEdges -RepoRoot $cargoTree
        Assert-Case 'cargo metadata yields a resolve graph with every workspace member' (
            @($graph.LocalIds).Count -eq 5)
        $bridgeIds = @($graph.IdToName.Keys | Where-Object { [string]$graph.IdToName[$_] -ceq 'outside_bridge' })
        Assert-Case 'a package outside the repository is KEPT in the graph the closure walks' (
            $bridgeIds.Count -eq 1 -and $graph.Edges.ContainsKey($bridgeIds[0]))
        Assert-Case 'a package outside the repository is NOT a battery-local package' (
            @($graph.LocalIds) -notcontains $bridgeIds[0])
        $partition = Get-BatteryPartition -RepoRoot $cargoTree -DependencyGraph $graph
        Assert-Case 'a RENAMED cargo dependency on a facility owner is serial' (
            $partition.Serial -contains 'renames_it')
        Assert-Case 'a DEV-dependency on a facility owner is serial' (
            $partition.Serial -contains 'dev_depends_on_it')
        Assert-Case 'reach through an OUT-OF-REPOSITORY bridge survives the real metadata reader' (
            $partition.Serial -contains 'reaches_through_the_bridge')
        Assert-Case 'the out-of-repository bridge is not reported as a battery package' (
            ($partition.Serial -notcontains 'outside_bridge') -and
            ($partition.Parallel -notcontains 'outside_bridge'))
        Assert-Case 'a real workspace member depending on nothing stays parallel' (
            $partition.Parallel -contains 'unrelated_leaf')
    }
    finally { $script:BatteryCargoRoots = $savedRoots }
}
finally {
    Remove-Item -LiteralPath $cargoTree -Recurse -Force -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath $bridgeTree -Recurse -Force -ErrorAction SilentlyContinue
}

# kills: reconciling the graph against the ownership scan BY NAME. Two
# repository-local packages can share a name: a scanned, marker-free
# `shared_facility` under a partition tree, and an unscanned one elsewhere in
# the repository that calls SuspendThread. Under a name comparison the scanned
# copy vouches for the unscanned one, the unscanned one never becomes an owner,
# and both it and its caller run parallel. The canonical MANIFEST PATH is the
# identity that cannot be shared.
$tree = New-SyntheticTree -Packages @{ shared_facility = 'pub fn f() -> u32 { 1 }' }
try {
    $graph = New-Edges -Root $tree -Map @{ shared_facility = @(); partition_test_anchor = @() }
    # A second id, same NAME, different manifest -- the unscanned copy.
    $graph.IdToName['unscanned_twin'] = 'shared_facility'
    $graph.IdToManifest['unscanned_twin'] = Join-Path $tree 'vendor\shared_facility\Cargo.toml'
    $graph.Edges['unscanned_twin'] = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    $graph.LocalIds = @($graph.LocalIds) + 'unscanned_twin'
    Assert-Throws 'a repository-local package sharing a scanned package''s NAME is still refused' `
        { Get-BatteryPartition -RepoRoot $tree -DependencyGraph $graph } 'never scanned for machine-scoped'
}
finally { Remove-Item -LiteralPath $tree -Recurse -Force -ErrorAction SilentlyContinue }

# kills: admitting a repository-local package into the graph that the owner scan
# never read. A vendored path dependency under repo\vendor, or a workspace
# member outside the three scanned trees, would be classified parallel with
# nothing having looked at it for markers -- and would drag its dependents
# along.
$tree = New-SyntheticTree -Packages @{ scanned = 'pub fn f() -> u32 { 1 }' }
try {
    $graph = New-Edges -Root $tree -Map @{ scanned = @(); partition_test_anchor = @(); never_scanned_vendored = @() }
    Assert-Throws 'a repository-local package the scan never read is refused' `
        { Get-BatteryPartition -RepoRoot $tree -DependencyGraph $graph } 'never scanned for machine-scoped'
}
finally { Remove-Item -LiteralPath $tree -Recurse -Force -ErrorAction SilentlyContinue }

# kills: `-like "$RepoRoot*"` as the containment test. A sibling directory whose
# name merely starts with the repository's counts as inside it.
Assert-Case 'a sibling path that shares a name prefix is not inside the repository' (
    -not (Test-PathIsInside -Root 'C:\repo' -Path 'C:\repo-copy\src\lib.rs'))
Assert-Case 'a path inside the repository is inside it, case-insensitively' (
    Test-PathIsInside -Root 'C:\repo' -Path 'c:\REPO\src\lib.rs')

# ==========================================================================
# GROUP 3: the fail-closed rules
# ==========================================================================

# kills: letting a marker table that has stopped matching the source report a
# clean parallel-only battery. An all-parallel partition is indistinguishable
# from a broken scan.
$tree = New-SyntheticTree -NoAnchor -Packages @{ all_pure = 'pub fn f() -> u32 { 1 }' }
try {
    Assert-Throws 'a scan that finds no facility owner at all is refused' `
        { Get-BatteryFacilityOwners -RepoRoot $tree } 'found NO package that touches a machine-scoped'
}
finally { Remove-Item -LiteralPath $tree -Recurse -Force -ErrorAction SilentlyContinue }

# kills: continuing past a source file the scan could not read. The file that
# could not be read is exactly the file that might have carried the marker, and
# under the closure its dependents' classification rides on the answer. The
# failure is injected by holding the file open with FileShare.None, which is a
# real read failure rather than a simulated one -- and is also what a concurrent
# editor or a virus scanner does.
$tree = New-SyntheticTree -Packages @{ readable = 'pub fn f() -> u32 { 1 }' }
$locked = $null
try {
    $lockedPath = Join-Path $tree 'extractor\native\readable\src\locked.rs'
    Set-Content -LiteralPath $lockedPath -Encoding utf8 -Value 'pub fn g() -> u32 { 2 }'
    $locked = [IO.File]::Open($lockedPath, [IO.FileMode]::Open, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
    Assert-Throws 'a source file that cannot be read is a refusal, not a cleared package' `
        { Get-BatteryFacilityOwners -RepoRoot $tree } 'Refusing rather than classifying'
}
finally {
    if ($null -ne $locked) { $locked.Dispose() }
    Remove-Item -LiteralPath $tree -Recurse -Force -ErrorAction SilentlyContinue
}

# kills: THE fail-open reconciliation defect. Removing it, or writing it as
# "anything unclassified is parallel", is the change that puts an unclassified
# test in the half with no mutex.
Assert-Throws 'a package the partition never classified is refused, not assumed parallel' {
    Assert-BatteryPartitionCoversPackages `
        -ObservedPackages @('known_serial', 'known_parallel', 'a_package_the_graph_never_saw') `
        -SerialPackages @('known_serial') `
        -ParallelPackages @('known_parallel')
} 'a_package_the_graph_never_saw'

# kills: a manifest this scan cannot name being skipped in silence. A [package]
# without a parseable name is a package whose tests would run unclassified.
$tree = New-SyntheticTree -Packages @{ named = 'pub fn f() -> u32 { 1 }' }
try {
    $anon = Join-Path $tree 'extractor\native\anonymous'
    New-Item -ItemType Directory -Path (Join-Path $anon 'src') -Force | Out-Null
    Set-Content -LiteralPath (Join-Path $anon 'Cargo.toml') -Encoding utf8 -Value "[package]`nversion = `"0.1.0`"`n"
    Assert-Throws 'a [package] manifest with no parseable name is refused' `
        { Get-BatteryFacilityOwners -RepoRoot $tree } 'has no name'
}
finally { Remove-Item -LiteralPath $tree -Recurse -Force -ErrorAction SilentlyContinue }

# kills: treating a virtual [workspace] manifest as a nameless package, which
# would fire the refusal above on every workspace root -- the mirror defect.
$tree = New-SyntheticTree -Packages @{ named = 'pub fn f() -> u32 { 1 }' }
try {
    Set-Content -LiteralPath (Join-Path $tree 'extractor\native\Cargo.toml') -Encoding utf8 `
        -Value "[workspace]`nmembers = [`"named`"]`n"
    $ownership = Get-BatteryFacilityOwners -RepoRoot $tree
    Assert-Case 'a virtual workspace manifest is not a package' (
        $ownership.Owners -contains 'partition_test_anchor')
}
finally { Remove-Item -LiteralPath $tree -Recurse -Force -ErrorAction SilentlyContinue }

# kills: an empty package inventory passing quietly -- what a scan pointed at
# the wrong repository root looks like.
$emptyRoot = Join-Path ([IO.Path]::GetTempPath()) ("hoi4_partition_empty_{0}_{1}" -f $PID, [guid]::NewGuid().ToString('N'))
foreach ($tree in @('extractor\native', 'replay_interface\native', 'portable_replay\native')) {
    New-Item -ItemType Directory -Path (Join-Path $emptyRoot $tree) -Force | Out-Null
}
try {
    Assert-Throws 'an empty package inventory is refused' `
        { Get-BatteryPackageInventory -RepoRoot $emptyRoot } 'inventoried no cargo packages'
}
finally { Remove-Item -LiteralPath $emptyRoot -Recurse -Force -ErrorAction SilentlyContinue }

# ==========================================================================
# GROUP 4: the filtersets the runner passes to nextest
# ==========================================================================

# kills: spelling the serial half with nextest's SUBSTRING matcher instead of
# the exact one. The substring form silently widens or narrows the half
# depending on what other packages are named.
$filter = Get-BatterySerialFilterset -SerialPackages @('recorder_host', 'wire_format')
Assert-Case 'the serial filterset uses the exact package matcher' (
    $filter -ceq 'package(=recorder_host) | package(=wire_format)')

# kills: interpolating a package name into a filterset without checking it.
Assert-Throws 'a filterset-unsafe package name is refused' `
    { Get-BatterySerialFilterset -SerialPackages @('bad name)|all(') } 'not filterset-safe'

Assert-Throws 'an empty serial set cannot produce a filterset' `
    { Get-BatterySerialFilterset -SerialPackages @() } 'empty package set'

# kills: enumerating the parallel half instead of complementing the serial one.
# Two independently-spelled halves can both miss a test; `not (serial)` cannot.
$halves = Get-BatteryRootHalves `
    -Partition ([ordered]@{ Serial = @('recorder_host', 'absent_here'); Parallel = @('collector'); Owners = @(); Evidence = @{} }) `
    -RootPackages @('recorder_host', 'collector')
Assert-Case 'the parallel half is the complement of the serial half, not a list' (
    $halves.Parallel -ceq 'not (package(=recorder_host))')
# kills: passing the whole serial set to a cargo root that does not contain some
# of it. nextest rejects a filterset naming a package it cannot see, so an
# unscoped filterset turns every standalone root into a hard failure.
Assert-Case 'the serial filterset is scoped to the packages the root contains' (
    $halves.SerialPackages.Count -eq 1 -and $halves.SerialPackages[0] -ceq 'recorder_host')

# kills: emitting a filterset for a root with nothing serial in it. `not ()` is
# not parseable and an empty `-E` selects nothing, silently skipping a root.
$none = Get-BatteryRootHalves `
    -Partition ([ordered]@{ Serial = @('recorder_host'); Parallel = @('collector'); Owners = @(); Evidence = @{} }) `
    -RootPackages @('collector')
Assert-Case 'a root with no serial package gets no filterset at all' (
    $null -eq $none.Serial -and $null -eq $none.Parallel)

# ==========================================================================
# verdict
# ==========================================================================
$status = if ($script:Failures.Count -eq 0) { 'PASS' } else { 'FAIL' }
$result = [ordered]@{
    schema   = 'battle-plan.battery_partition_test.v1'
    status   = $status
    cases    = $script:Cases
    failures = @($script:Failures)
}
Write-Output ($result | ConvertTo-Json -Depth 4 -Compress)
if ($status -cne 'PASS') { exit 1 }
