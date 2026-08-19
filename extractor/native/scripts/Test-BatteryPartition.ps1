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

# ==========================================================================
# GROUP: THE ORPHAN GUARD -- a package that is a member of no cargo root
# ==========================================================================
#
# This reproduces the real defect exactly. `portable_terrain_fixture` sat under
# `portable_replay\native`, was a path DEPENDENCY of a workspace member and a
# member of nothing, and so was selected by no battery for eleven days while its
# three tests never ran. `--workspace` does not reach such a package and
# `cargo -p` PANICS on it (exit 101), so the battery reports green having tested
# it not at all -- there is no loud failure mode to rely on, which is why this
# assertion exists rather than a notice.
$orphanTree = Join-Path ([IO.Path]::GetTempPath()) ("hoi4_partition_orphan_{0}_{1}" -f $PID, [guid]::NewGuid().ToString('N'))
try {
    New-SyntheticCargoWorkspace -Root $orphanTree
    # The orphan sits under the portable tree with its OWN manifest, exactly
    # where the real one sat, and is named in no workspace members list.
    $orphanDir = Join-Path $orphanTree 'portable_replay\native\orphan_crate'
    New-Item -ItemType Directory -Path (Join-Path $orphanDir 'src') -Force | Out-Null
    Set-Content -LiteralPath (Join-Path $orphanDir 'Cargo.toml') -Encoding utf8 -Value (
        "[package]`nname = `"orphan_crate`"`nversion = `"0.1.0`"`nedition = `"2021`"`n")
    Set-Content -LiteralPath (Join-Path $orphanDir 'src\lib.rs') -Encoding utf8 -Value `
        '#[cfg(test)] mod tests { #[test] fn never_ran() { assert!(true); } }'

    $savedRoots = $script:BatteryCargoRoots
    try {
        # kills: dropping the orphan guard, or downgrading it back to the
        # notice-and-drop arm it replaced. With only the workspace registered,
        # `orphan_crate` is a member of nothing and no battery can run its test.
        $script:BatteryCargoRoots = @('extractor\native')
        Assert-Throws 'a package that is a member of NO cargo root is REFUSED, not dropped' `
            { Assert-BatteryEveryPackageIsRegistered -RepoRoot $orphanTree } `
            'member of NO cargo root'
        # kills: a refusal that does not say WHICH package, which is the whole
        # difference between a fixable refusal and a puzzle.
        $orphanMessage = ''
        try { Assert-BatteryEveryPackageIsRegistered -RepoRoot $orphanTree }
        catch { $orphanMessage = [string]$_.Exception.Message }
        Assert-Case 'the refusal NAMES the unregistered package' ($orphanMessage -like '*orphan_crate*')
        Assert-Case 'the refusal names the file that registers it' (
            $orphanMessage -like '*BatteryCargoRoots*')

        # kills: registering the root but leaving the guard keyed to something
        # the registration does not change. Adding the crate as its own cargo
        # root -- the fix lane LD applied to the real one -- must clear it.
        $script:BatteryCargoRoots = @('extractor\native', 'portable_replay\native\orphan_crate')
        $cleared = $true
        try { Assert-BatteryEveryPackageIsRegistered -RepoRoot $orphanTree }
        catch { $cleared = $false }
        Assert-Case 'registering the crate as its own cargo root clears the refusal' $cleared
    }
    finally { $script:BatteryCargoRoots = $savedRoots }
}
finally { Remove-Item -LiteralPath $orphanTree -Recurse -Force -ErrorAction SilentlyContinue }

# kills: comparing registration BY PACKAGE NAME. Names are not unique in this
# repository -- the partition seeds closures on manifest identity for exactly
# that reason -- so an orphan sharing a registered package's name would be
# reported as covered by the package that IS registered, and its tests would go
# on never running.
$sameNameTree = Join-Path ([IO.Path]::GetTempPath()) ("hoi4_partition_samename_{0}_{1}" -f $PID, [guid]::NewGuid().ToString('N'))
try {
    New-SyntheticCargoWorkspace -Root $sameNameTree
    $twinDir = Join-Path $sameNameTree 'portable_replay\native\unrelated_leaf'
    New-Item -ItemType Directory -Path (Join-Path $twinDir 'src') -Force | Out-Null
    # SAME NAME as a real member of the registered workspace, different manifest.
    Set-Content -LiteralPath (Join-Path $twinDir 'Cargo.toml') -Encoding utf8 -Value (
        "[package]`nname = `"unrelated_leaf`"`nversion = `"0.1.0`"`nedition = `"2021`"`n")
    Set-Content -LiteralPath (Join-Path $twinDir 'src\lib.rs') -Encoding utf8 -Value 'pub fn f() {}'
    $savedRoots = $script:BatteryCargoRoots
    try {
        $script:BatteryCargoRoots = @('extractor\native')
        Assert-Throws 'an orphan sharing a REGISTERED package''s name is still an orphan' `
            { Assert-BatteryEveryPackageIsRegistered -RepoRoot $sameNameTree } `
            'member of NO cargo root'
    }
    finally { $script:BatteryCargoRoots = $savedRoots }
}
finally { Remove-Item -LiteralPath $sameNameTree -Recurse -Force -ErrorAction SilentlyContinue }

# kills: an orphan guard that passes by having nothing to check. An empty
# inventory is the vacuous direction -- every package is registered because
# there are no packages -- and it is the shape a broken tree walk produces.
Assert-Throws 'an EMPTY package inventory makes the orphan guard a refusal, not a pass' `
    { Assert-BatteryEveryPackageIsRegistered -RepoRoot 'C:\nonexistent_ld' -Inventory @() `
            -RegisteredManifests ([System.Collections.Generic.HashSet[string]]::new()) } `
    'inventory is empty'

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
# GROUP 5: the affected-package side -- what an iteration run selects
#
# The failure this group exists to kill is the same one as everywhere else in
# this file and it is silent: a selection that comes out too NARROW does not
# error, it reports green having never compiled the code the lane changed.
# ==========================================================================

# kills: `--package X` without the dependents expansion, which is what the
# runner did until this lane. Cargo builds X and X's DEPENDENCIES; the crates
# that IMPORT X are never compiled. Measured on the real tree:
# `-Package runtime_text_inputs_live_lane` compiled 3 repository packages while
# 5 direct importers were not compiled at all.
$tree = New-SyntheticTree -Packages @{
    leaf          = 'pub fn f() -> u32 { 1 }'
    imports_leaf  = 'pub fn g() -> u32 { leaf::f() }'
    two_removes   = 'pub fn h() -> u32 { imports_leaf::g() }'
    unrelated     = 'pub fn k() -> u32 { 2 }'
}
try {
    $edges = New-Edges -Root $tree -Map @{
        partition_test_anchor = @()
        leaf                  = @()
        imports_leaf          = @('leaf')
        two_removes           = @('imports_leaf')
        unrelated             = @()
    }
    $closure = Get-BatteryDependentClosure -RepoRoot $tree -SeedPackages @('leaf') -DependencyGraph $edges
    Assert-Case 'the dependents closure contains the seed itself' ($closure -contains 'leaf')
    Assert-Case 'a DIRECT importer of the seed is selected (the JX defect)' ($closure -contains 'imports_leaf')
    # kills: a one-hop expansion. Cargo's own `--package` is zero-hop; a one-hop
    # fix would still miss the crate two removes away.
    Assert-Case 'an importer TWO removes away is selected (fixed point, not one hop)' (
        $closure -contains 'two_removes')
    # kills: expanding to the whole workspace, which would make the mode
    # pointless and is the other way to be wrong here.
    Assert-Case 'a package that imports nothing in the seed set is NOT selected' (
        $closure -notcontains 'unrelated')

    # kills: seeding the closure from a name that matches nothing and letting
    # the run proceed with an empty selection. nextest exits 0 on a selection
    # that matches nothing, so this refusal is the only thing between a typo and
    # a green run that tested none of what was asked for.
    Assert-Throws 'a seed package name that matches nothing is refused' `
        { Get-BatteryDependentClosure -RepoRoot $tree -SeedPackages @('no_such_crate') -DependencyGraph $edges } `
        'match no package'

    # kills: computing a closure from an empty seed, which returns an empty
    # selection rather than an error.
    Assert-Throws 'a closure from no seed at all is refused' `
        { Get-BatteryDependentClosure -RepoRoot $tree -DependencyGraph $edges } 'no seed at all'
}
finally { Remove-Item -LiteralPath $tree -Recurse -Force -ErrorAction SilentlyContinue }

# kills: leaving the two closures as two copies of one fixed point. Both the
# serial half and the affected set must come out of Get-BatteryDependentIdClosure,
# and this pins the primitive itself against the shape a one-hop rule fails and
# the shape a repository-pruned graph fails.
$primitiveEdges = @{
    a          = [System.Collections.Generic.HashSet[string]]::new([string[]]@('b'), [StringComparer]::Ordinal)
    b          = [System.Collections.Generic.HashSet[string]]::new([string[]]@('external'), [StringComparer]::Ordinal)
    external   = [System.Collections.Generic.HashSet[string]]::new([string[]]@('seed'), [StringComparer]::Ordinal)
    seed       = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    untouched  = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
}
$primitive = Get-BatteryDependentIdClosure -Edges $primitiveEdges -SeedIds @('seed')
Assert-Case 'the closure primitive follows a path that leaves the graph''s local set and returns' (
    $primitive.Contains('a') -and $primitive.Contains('b') -and $primitive.Contains('external'))
Assert-Case 'the closure primitive leaves an unrelated node out' (-not $primitive.Contains('untouched'))
Assert-Case 'the closure primitive keeps its seed' ($primitive.Contains('seed'))

# kills: treating a change to a workspace-wide input as a change to one package
# (or to none). `.cargo/config.toml` selects the target directory, the profiles
# and incremental compilation for EVERY package; a diff touching only it maps to
# no package, and an iteration run would have compiled nothing and reported green.
Assert-Case 'the cargo config is a workspace-wide input' (
    Test-BatteryPathIsWorkspaceWideInput -Path '.cargo/config.toml')
Assert-Case 'a workspace manifest is a workspace-wide input' (
    Test-BatteryPathIsWorkspaceWideInput -Path 'extractor/native/Cargo.toml')
Assert-Case 'a package manifest is a workspace-wide input too (it can move targets and features)' (
    Test-BatteryPathIsWorkspaceWideInput -Path 'extractor/native/collector/Cargo.toml')
Assert-Case 'the lockfile is a workspace-wide input' (
    Test-BatteryPathIsWorkspaceWideInput -Path 'Cargo.lock')
Assert-Case 'the toolchain pin is a workspace-wide input' (
    Test-BatteryPathIsWorkspaceWideInput -Path 'rust-toolchain.toml')
Assert-Case 'the nextest config is a workspace-wide input' (
    Test-BatteryPathIsWorkspaceWideInput -Path 'replay_interface/native/.config/nextest.toml')
# kills: a pattern loose enough to swallow ordinary source, which would widen
# every iteration run to the whole battery and quietly retire the mode.
Assert-Case 'an ordinary source file is not a workspace-wide input' (
    -not (Test-BatteryPathIsWorkspaceWideInput -Path 'extractor/native/collector/src/lib.rs'))
Assert-Case 'a file merely NAMED like a manifest is not one' (
    -not (Test-BatteryPathIsWorkspaceWideInput -Path 'extractor/native/collector/src/Cargo.toml.rs'))
# kills: reading the pattern against a Windows-separated path, which is what
# arrives if a caller ever stops normalising.
Assert-Case 'the workspace-wide test is separator-agnostic' (
    Test-BatteryPathIsWorkspaceWideInput -Path 'extractor\native\Cargo.toml')
# kills: anchoring the config patterns at the REPOSITORY ROOT. A nested
# .cargo/config.toml overrides the root one for its cargo root alone (the
# runner's target-dir guard says so), and a nested rust-toolchain.toml pins a
# different compiler for its subtree. Root-anchored patterns called both of
# those "unowned", and in a MIXED diff the one recognised package supplied a
# positive census while the rest of that root compiled under changed
# configuration and never ran.
Assert-Case 'a NESTED cargo config is a workspace-wide input' (
    Test-BatteryPathIsWorkspaceWideInput -Path 'extractor/native/.cargo/config.toml')
Assert-Case 'a NESTED toolchain pin is a workspace-wide input' (
    Test-BatteryPathIsWorkspaceWideInput -Path 'portable_replay/native/snapshot/rust-toolchain.toml')
Assert-Case 'the extensionless toolchain pin spelling is recognised too' (
    Test-BatteryPathIsWorkspaceWideInput -Path 'rust-toolchain')
# kills: a `.cargo` pattern loose enough to match a directory merely CONTAINING
# those characters, which would widen every diff that touches build output.
Assert-Case 'a directory that merely ends in cargo is not the cargo config dir' (
    -not (Test-BatteryPathIsWorkspaceWideInput -Path 'extractor/native/.cargo-target/debug/x.rs'))

# kills: attributing a spliced file to the package it SITS IN rather than the
# package that COMPILES it. This repository does exactly this today:
# map_viewer/tests/xmap00_map_authority.rs carries
# include_str!("../../replay_app/src/validate.rs"), and the cargo dependency runs
# the other way, so editing that file reaches map_viewer through no edge at all.
$tree = New-SyntheticTree -Packages @{
    provides_the_file = 'pub fn f() -> u32 { 1 }'
    splices_it        = 'pub fn g() {}'
}
try {
    Set-Content -LiteralPath (Join-Path $tree 'extractor\native\provides_the_file\src\pulled.rs') `
        -Encoding utf8 -Value 'pub const X: u32 = 1;'
    Set-Content -LiteralPath (Join-Path $tree 'extractor\native\provides_the_file\src\raw_pulled.rs') `
        -Encoding utf8 -Value 'pub const Y: u32 = 2;'
    # SEPARATE FILES for the two hashed spellings. Pointing both at one file
    # makes a single assertion green while either branch is broken, because the
    # other branch still contributes the key.
    Set-Content -LiteralPath (Join-Path $tree 'extractor\native\provides_the_file\src\hash_path.rs') `
        -Encoding utf8 -Value 'pub const Z: u32 = 3;'
    Set-Content -LiteralPath (Join-Path $tree 'extractor\native\provides_the_file\src\hash_include.rs') `
        -Encoding utf8 -Value 'pub const W: u32 = 4;'
    Set-Content -LiteralPath (Join-Path $tree 'extractor\native\splices_it\src\lib.rs') -Encoding utf8 -Value @(
        'pub fn g() {}',
        'const TEXT: &str = include_str!("../../provides_the_file/src/pulled.rs");',
        '#[path = "../../provides_the_file/src/pulled.rs"]',
        'mod pulled_module;',
        '// A RAW literal is ordinary Rust and is exactly what an author reaches',
        '// for when the path has backslashes in it.',
        'const RAW: &str = include_str!(r"../../provides_the_file/src/raw_pulled.rs");',
        '// And the HASHED raw spelling, whose closing hashes a leading-hash-only',
        '// pattern leaves sitting before the `]`.',
        '#[path = r#"../../provides_the_file/src/hash_path.rs"#]',
        'mod hash_path_module;',
        'const HASHED: &str = include_str!(r##"../../provides_the_file/src/hash_include.rs"##);')
    $splices = Get-BatterySourceSpliceOwners -RepoRoot $tree
    $pulled = [IO.Path]::GetFullPath((Join-Path $tree 'extractor\native\provides_the_file\src\pulled.rs'))
    $rawPulled = [IO.Path]::GetFullPath((Join-Path $tree 'extractor\native\provides_the_file\src\raw_pulled.rs'))
    $hashPath = [IO.Path]::GetFullPath((Join-Path $tree 'extractor\native\provides_the_file\src\hash_path.rs'))
    $hashInclude = [IO.Path]::GetFullPath((Join-Path $tree 'extractor\native\provides_the_file\src\hash_include.rs'))
    # kills: a splice regex that only accepts ordinary quoted literals.
    # `include_str!(r"...")` is legal Rust and was missed.
    Assert-Case 'the splice scan reads a RAW string literal too' ($splices.ContainsKey($rawPulled))
    # kills: consuming the OPENING hashes of a raw literal without matching the
    # closing ones. `#[path = r#"child.rs"#]` then leaves a `#` before the `]`
    # and the whole attribute falls out of the scan, silently.
    Assert-Case 'the splice scan reads a hashed raw #[path] literal' (
        $splices.ContainsKey($hashPath))
    # kills: the include! branch's own hash back-reference. A SEPARATE target
    # file, because pointing both hashed spellings at one file made a single
    # assertion green while either branch was broken -- the other branch still
    # contributed the key.
    Assert-Case 'the splice scan reads a double-hashed raw include_str! literal' (
        $splices.ContainsKey($hashInclude))
    Assert-Case 'the splice scan finds the file a package pulls in from another package' (
        $splices.ContainsKey($pulled))
    Assert-Case 'the splicing package is named as an owner of that file' (
        $splices.ContainsKey($pulled) -and
        @($splices[$pulled]) -contains ([IO.Path]::GetFullPath((Join-Path $tree 'extractor\native\splices_it\Cargo.toml'))))

    # kills: dropping the nearest-ancestor answer once a splice owner exists.
    # The file's OWN package still compiles it, and the map is a union.
    $owned = Get-BatteryPackagesForPaths -RepoRoot $tree `
        -Paths @('extractor/native/provides_the_file/src/pulled.rs') -SpliceOwners $splices
    Assert-Case 'a spliced file maps to BOTH the package holding it and the package pulling it' (
        ($owned.Packages -contains 'provides_the_file') -and ($owned.Packages -contains 'splices_it'))
}
finally { Remove-Item -LiteralPath $tree -Recurse -Force -ErrorAction SilentlyContinue }

# ==========================================================================
# GROUP: THE DEP-INFO PRECISION LAYER
# ==========================================================================
#
# kills: mis-parsing a rustc `.d` file. Both hazards below are Windows-specific
# and both are present in every real dep-info this repository writes: the TARGET
# is an absolute path whose drive letter is a colon (so the rule separator is
# not the first colon on the line), and rustc escapes a space in a path as `\ `
# while `\` is also the separator (so splitting on a bare space breaks a path
# with a space in it into two nonexistent ones).
$depInfoFile = Join-Path ([IO.Path]::GetTempPath()) ("hoi4_depinfo_{0}_{1}.d" -f $PID, [guid]::NewGuid().ToString('N'))
try {
    Set-Content -LiteralPath $depInfoFile -Encoding utf8 -Value @(
        'C:\out\deps\thing-abc123.exe: src\lib.rs src\other.rs',
        'C:\out\deps\thing-abc123.exe: src\my\ dir\spaced.rs',
        '',
        'src\lib.rs:')
    $depSources = @(Get-BatteryDepInfoSourcePaths -DepInfoPath $depInfoFile -BaseDirectory 'C:\base')
    Assert-Case 'the dep-info reader does not split the target path at its DRIVE LETTER colon' (
        ($depSources -notcontains 'C:\base\C') -and
        ($depSources -contains 'C:\base\src\lib.rs'))
    Assert-Case 'the dep-info reader returns every prerequisite on a rule' (
        $depSources -contains 'C:\base\src\other.rs')
    # kills: splitting prerequisites on a bare space. `src\my\ dir\spaced.rs` is
    # ONE path whose space rustc escaped, and a bare split yields two paths that
    # exist nowhere while losing the real one.
    Assert-Case 'the dep-info reader keeps an ESCAPED space as one path' (
        $depSources -contains 'C:\base\src\my dir\spaced.rs')
    # kills: resolving prerequisites against the PACKAGE directory. rustc writes
    # them relative to the directory cargo invoked it from, which is the cargo
    # ROOT, and resolving against anything else produces paths that match no
    # changed file -- a layer that silently finds nothing.
    Assert-Case 'prerequisites resolve against the cargo root, not the package' (
        @($depSources | Where-Object { $_ -like 'C:\base\*' }).Count -eq $depSources.Count)
}
finally { Remove-Item -LiteralPath $depInfoFile -Force -ErrorAction SilentlyContinue }

# kills: reading a `.d` line-by-line without joining MAKEFILE CONTINUATIONS. A
# wrapped rule's continuation line carries no colon, so the rule-separator scan
# skips it and every prerequisite after the wrap is lost silently. The pinned
# compiler emits none today, which is precisely why a change in it must fail
# here rather than quietly narrow what the layer can see.
$continuationFile = Join-Path ([IO.Path]::GetTempPath()) ("hoi4_depinfo_cont_{0}_{1}.d" -f $PID, [guid]::NewGuid().ToString('N'))
try {
    Set-Content -LiteralPath $continuationFile -Encoding utf8 -Value @(
        'C:\out\deps\thing-abc123.exe: src\first.rs \',
        ' src\second.rs \',
        ' src\third.rs')
    $continued = @(Get-BatteryDepInfoSourcePaths -DepInfoPath $continuationFile -BaseDirectory 'C:\base')
    Assert-Case 'a wrapped dep-info rule keeps the prerequisite BEFORE the wrap' (
        $continued -contains 'C:\base\src\first.rs')
    Assert-Case 'a wrapped dep-info rule keeps the prerequisites AFTER the wrap' (
        ($continued -contains 'C:\base\src\second.rs') -and ($continued -contains 'C:\base\src\third.rs'))
}
finally { Remove-Item -LiteralPath $continuationFile -Force -ErrorAction SilentlyContinue }

# kills: treating an UNREADABLE `.d` as an empty one. Both would return no
# sources, and "this target reads nothing" is indistinguishable from a correct
# answer -- so the layer would silently cover less than it reports.
Assert-Throws 'an unreadable dep-info file is a refusal, not an empty source list' `
    { Get-BatteryDepInfoSourcePaths -DepInfoPath 'C:\nonexistent_ld\missing.d' -BaseDirectory 'C:\base' } `
    'could not be read'

# kills: deriving the dep-info name from the artifact name by swapping the
# extension. Cargo diverges from that in three ways, and each one silently
# drops a whole class of target out of the layer. All three were found by the
# completeness check refusing on the real repository, not by reasoning.
$artifactTree = Join-Path ([IO.Path]::GetTempPath()) ("hoi4_artifact_{0}_{1}" -f $PID, [guid]::NewGuid().ToString('N'))
try {
    $depsDir = Join-Path $artifactTree 'debug\deps'
    $buildDir = Join-Path $artifactTree 'debug\build\collector-8054e0867b9bec3c'
    New-Item -ItemType Directory -Path $depsDir -Force | Out-Null
    New-Item -ItemType Directory -Path $buildDir -Force | Out-Null

    # (1) the `lib` prefix: libfoo-<hash>.rlib pairs with foo-<hash>.d
    Set-Content -LiteralPath (Join-Path $depsDir 'foo-0123456789abcdef.d') -Encoding utf8 -Value 'x:'
    $libResolved = @(Get-BatteryDepInfoPathsForArtifact `
            -ArtifactPath (Join-Path $depsDir 'libfoo-0123456789abcdef.rlib'))
    Assert-Case 'an rlib resolves to the dep-info written WITHOUT its lib prefix' (
        @($libResolved | Where-Object { $_ -like '*foo-0123456789abcdef.d' }).Count -eq 1)

    # (2) UPLIFTING: a bin's artifact is debug\pack_replay.exe while its
    # dep-info stays behind in debug\deps\pack_replay.d.
    Set-Content -LiteralPath (Join-Path $depsDir 'pack_replay.d') -Encoding utf8 -Value 'x:'
    $upliftResolved = @(Get-BatteryDepInfoPathsForArtifact `
            -ArtifactPath (Join-Path $artifactTree 'debug\pack_replay.exe'))
    Assert-Case 'an UPLIFTED bin resolves to the dep-info left behind in deps\' (
        @($upliftResolved | Where-Object { $_ -like '*deps\pack_replay.d' }).Count -eq 1)

    # (3) a build script's artifact is build-script-build.exe; its dep-info is
    # build_script_build-<hash>.d beside it -- hyphens become underscores AND a
    # hash appears.
    Set-Content -LiteralPath (Join-Path $buildDir 'build_script_build-8054e0867b9bec3c.d') -Encoding utf8 -Value 'x:'
    $buildResolved = @(Get-BatteryDepInfoPathsForArtifact `
            -ArtifactPath (Join-Path $buildDir 'build-script-build.exe'))
    Assert-Case 'a BUILD SCRIPT resolves through the hyphen-to-underscore and hash rename' (
        @($buildResolved | Where-Object { $_ -like '*build_script_build-8054e0867b9bec3c.d' }).Count -eq 1)

    # kills: matching the hash suffix with a loose `-*` glob, which lets
    # `foo-*.d` claim a DIFFERENT crate's `foo-bar-<hash>.d` and attribute that
    # crate's sources here.
    Set-Content -LiteralPath (Join-Path $depsDir 'foo-bar-fedcba9876543210.d') -Encoding utf8 -Value 'x:'
    $strictResolved = @(Get-BatteryDepInfoPathsForArtifact `
            -ArtifactPath (Join-Path $depsDir 'libfoo-0123456789abcdef.rlib'))
    Assert-Case 'the hash suffix is 16 hex, so a different crate''s dep-info is not claimed' (
        @($strictResolved | Where-Object { $_ -like '*foo-bar-*' }).Count -eq 0)

    # kills: returning EVERY hashed match. Two packages can each produce a
    # `shared_bin` stem. Returning both attributes one package's sources to the
    # other's manifest -- a WRONG attribution, not a conservative widening --
    # and, worse, any same-stem file satisfies the caller's "did this target
    # have a .d" test, so the completeness refusal stops firing for the target
    # that actually lost one. Ambiguity must resolve to NOTHING.
    Set-Content -LiteralPath (Join-Path $depsDir 'shared_bin-1111111111111111.d') -Encoding utf8 -Value 'x:'
    Set-Content -LiteralPath (Join-Path $depsDir 'shared_bin-2222222222222222.d') -Encoding utf8 -Value 'x:'
    $ambiguous = @(Get-BatteryDepInfoPathsForArtifact `
            -ArtifactPath (Join-Path $artifactTree 'debug\shared_bin.exe'))
    Assert-Case 'an AMBIGUOUS same-stem dep-info match resolves to nothing, not to both' (
        $ambiguous.Count -eq 0)
    # ...and one unambiguous hashed match is still resolved, so the rule above
    # is a tie-breaker rather than a blanket refusal of the hashed form.
    Remove-Item -LiteralPath (Join-Path $depsDir 'shared_bin-2222222222222222.d') -Force
    $unambiguous = @(Get-BatteryDepInfoPathsForArtifact `
            -ArtifactPath (Join-Path $artifactTree 'debug\shared_bin.exe'))
    Assert-Case 'a single hashed dep-info match is still resolved' (
        @($unambiguous | Where-Object { $_ -like '*shared_bin-1111111111111111.d' }).Count -eq 1)
    # kills: searching the hashed form even when the exact name matched. The
    # exact file is the right answer and the hashed scan is only a fallback;
    # running both would reintroduce the collision above for hashed artifacts.
    Set-Content -LiteralPath (Join-Path $depsDir 'shared_bin.d') -Encoding utf8 -Value 'x:'
    $exactWins = @(Get-BatteryDepInfoPathsForArtifact `
            -ArtifactPath (Join-Path $artifactTree 'debug\shared_bin.exe'))
    Assert-Case 'an EXACT dep-info match wins outright over the hashed fallback' (
        ($exactWins.Count -eq 1) -and ($exactWins[0] -like '*deps\shared_bin.d'))
}
finally { Remove-Item -LiteralPath $artifactTree -Recurse -Force -ErrorAction SilentlyContinue }

# kills: passing the caller's whole -Features list to every cargo root. A
# feature only has to exist in ITS OWN root: `replay_decoder/dq-v3-prototype` is
# valid in the extractor workspace and makes cargo EXIT 101 in the replay one
# ("none of the selected packages contains this feature"). The dep-info layer's
# own compile-failure throw then refused a legitimate invocation.
$scoped = @(Select-BatteryFeaturesForRoot `
        -Features @('replay_decoder/dq-v3-prototype', 'map_viewer/d3d11', 'a-bare-feature', 'absent-bare') `
        -MemberNames @('map_viewer', 'replay_core') `
        -DeclaredFeatures @('a-bare-feature'))
Assert-Case 'a package-qualified feature is KEPT for the root that has the package' (
    $scoped -contains 'map_viewer/d3d11')
Assert-Case 'a package-qualified feature is DROPPED for a root without that package' (
    $scoped -notcontains 'replay_decoder/dq-v3-prototype')
Assert-Case 'a bare feature some member DECLARES is kept' ($scoped -contains 'a-bare-feature')
Assert-Case 'a bare feature no member declares is dropped' ($scoped -notcontains 'absent-bare')

# kills: reading cargo's UTF-8 output through PowerShell 5.1's ANSI default.
# Get-Content without -Encoding UTF8 decodes via the system codepage
# (Windows-1252 here), so a non-ASCII manifest path arrives as mojibake --
# `cafe\u0301` style bytes become `Ã©` -- BEFORE ConvertFrom-Json. The result is
# a manifest path matching nothing: a false orphan refusal, or lost ownership.
$encodingFile = Join-Path ([IO.Path]::GetTempPath()) ("hoi4_enc_{0}_{1}.json" -f $PID, [guid]::NewGuid().ToString('N'))
try {
    $nonAscii = 'C:\repo\caf' + [char]0xE9 + '\Cargo.toml'
    [IO.File]::WriteAllText($encodingFile, $nonAscii, [System.Text.UTF8Encoding]::new($false))
    $viaUtf8 = (Get-Content -LiteralPath $encodingFile -Raw -Encoding UTF8).Trim()
    Assert-Case 'UTF-8 cargo output round-trips only when the encoding is stated' (
        $viaUtf8 -ceq $nonAscii)
    Assert-Case 'the ANSI default really does mangle it (so the case is not vacuous)' (
        (Get-Content -LiteralPath $encodingFile -Raw).Trim() -cne $nonAscii)
}
finally { Remove-Item -LiteralPath $encodingFile -Force -ErrorAction SilentlyContinue }

# kills: reverting the summary's key probe to $Selection.PSObject.Properties.Name.
# The two cases above test the PowerShell semantics; this one drives the actual
# production function and reads what it PRINTS, so the fix cannot be undone in
# BatteryPartition.ps1 while the suite stays green.
$summarySelection = [ordered]@{
    ChangedPaths       = @('extractor/native/collector/src/lib.rs')
    TouchedPackages    = @('collector')
    AffectedPackages   = @('collector')
    UnownedPaths       = @()
    WorkspaceWideInput = @()
    SelectsEverything  = $false
    DepInfoUsed        = $true
}
# NOT Out-String: it wraps at the console width, which splits the "cannot see:"
# sentence across lines and makes a single-line match fail for a reason that has
# nothing to do with the code under test. Each record's own text is taken instead.
$summaryOn = ((@(Write-BatteryAffectedSelectionSummary -Selection $summarySelection 6>&1) |
        ForEach-Object { [string]$_ }) -join "`n")
Assert-Case 'the summary ANNOUNCES the dep-info layer when it ran' (
    $summaryOn -like '*DEP-INFO PRECISION LAYER ON*')
# Scoped to the LIMITS sentence ("cannot see: ..."), because the announcement
# the layer prints when it DID run also contains the words "computed literals" --
# a bare -notlike would fail on the very message that proves the fix works.
Assert-Case 'the summary stops LISTING computed literals among what it cannot see' (
    $summaryOn -notmatch 'cannot see:[^\r\n]*computed literal')
$summarySelection.DepInfoUsed = $false
$summaryOff = ((@(Write-BatteryAffectedSelectionSummary -Selection $summarySelection 6>&1) |
        ForEach-Object { [string]$_ }) -join "`n")
Assert-Case 'the summary still names the computed-literal limit when the layer did NOT run' (
    $summaryOff -match 'cannot see:[^\r\n]*computed literal')
Assert-Case 'the summary does not announce a layer that did not run' (
    $summaryOff -notlike '*DEP-INFO PRECISION LAYER ON*')

# kills: shipping the lexical splice scan as if it were complete. THIS IS THE
# CASE THE LEXICAL SCAN CANNOT SEE. `include!(concat!(..))` puts no path literal
# in the source at all, so the regex has nothing to match -- lane JD proved that
# residual needs macro expansion and a real Rust lexer, not more patterns. The compiler resolved
# it, so cargo's dep-info names the file and the layer attributes it correctly.
#
# The assertions are PAIRED on purpose: the same tree is asked both ways, so
# this fails if the lexical scan silently starts matching (making the case
# vacuous) as well as if the dep-info layer stops working.
$concatTree = Join-Path ([IO.Path]::GetTempPath()) ("hoi4_concat_{0}_{1}" -f $PID, [guid]::NewGuid().ToString('N'))
try {
    New-Item -ItemType Directory -Path (Join-Path $concatTree 'replay_interface\native') -Force | Out-Null
    New-Item -ItemType Directory -Path (Join-Path $concatTree 'portable_replay\native') -Force | Out-Null
    $nativeRoot = Join-Path $concatTree 'extractor\native'
    New-Item -ItemType Directory -Path $nativeRoot -Force | Out-Null
    Set-Content -LiteralPath (Join-Path $nativeRoot 'Cargo.toml') -Encoding utf8 -Value (
        "[workspace]`nresolver = `"2`"`nmembers = [`"holds_it`", `"pulls_it`"]`n")
    # The file lives in one package...
    New-Item -ItemType Directory -Path (Join-Path $nativeRoot 'holds_it\src') -Force | Out-Null
    Set-Content -LiteralPath (Join-Path $nativeRoot 'holds_it\Cargo.toml') -Encoding utf8 -Value (
        "[package]`nname = `"holds_it`"`nversion = `"0.1.0`"`nedition = `"2021`"`n")
    Set-Content -LiteralPath (Join-Path $nativeRoot 'holds_it\src\lib.rs') -Encoding utf8 -Value 'pub fn f() {}'
    Set-Content -LiteralPath (Join-Path $nativeRoot 'holds_it\src\spliced.rs') -Encoding utf8 -Value `
        'pub fn spliced_value() -> u32 { 7 }'
    # ...and ANOTHER package compiles it, through a path no literal spells out.
    New-Item -ItemType Directory -Path (Join-Path $nativeRoot 'pulls_it\src') -Force | Out-Null
    Set-Content -LiteralPath (Join-Path $nativeRoot 'pulls_it\Cargo.toml') -Encoding utf8 -Value (
        "[package]`nname = `"pulls_it`"`nversion = `"0.1.0`"`nedition = `"2021`"`n")
    Set-Content -LiteralPath (Join-Path $nativeRoot 'pulls_it\src\lib.rs') -Encoding utf8 -Value @(
        'include!(concat!("../../holds_it/src/", "spliced.rs"));',
        '#[cfg(test)]',
        'mod tests {',
        '    #[test]',
        '    fn the_spliced_file_is_compiled_into_this_package() {',
        '        assert_eq!(super::spliced_value(), 7);',
        '    }',
        '}')

    $splicedPath = 'extractor/native/holds_it/src/spliced.rs'
    $pullsManifest = [IO.Path]::GetFullPath((Join-Path $nativeRoot 'pulls_it\Cargo.toml'))

    # THE LEXICAL FLOOR, ASKED FIRST. It attributes the file to the package it
    # SITS IN and to nothing else, because there is no literal to read.
    $concatSplices = Get-BatterySourceSpliceOwners -RepoRoot $concatTree
    $lexical = Get-BatteryPackagesForPaths -RepoRoot $concatTree -Paths @($splicedPath) `
        -SpliceOwners $concatSplices
    Assert-Case 'the LEXICAL scan misses a file spliced in by a computed literal' (
        ($lexical.Packages -contains 'holds_it') -and ($lexical.Packages -notcontains 'pulls_it'))

    # THE PRECISION LAYER. One compile-only pass, and the compiler's own answer.
    $concatRoots = @($nativeRoot)
    $depOwners = Get-BatteryDepInfoOwners -RepoRoot $concatTree -CargoRoots $concatRoots
    $splicedAbsolute = [IO.Path]::GetFullPath((Join-Path $concatTree ($splicedPath -replace '/', '\')))
    Assert-Case 'DEP-INFO finds the file spliced in by a computed literal' (
        $depOwners.ContainsKey($splicedAbsolute))
    Assert-Case 'dep-info attributes it to the package whose COMPILER read it' (
        $depOwners.ContainsKey($splicedAbsolute) -and
        (@($depOwners[$splicedAbsolute]) -contains $pullsManifest))

    # kills: making the precision layer a REPLACEMENT rather than a union. The
    # file's own package still compiles it, and losing that would narrow the
    # selection -- the one direction this layer is not allowed to move it.
    $withDepInfo = Get-BatteryPackagesForPaths -RepoRoot $concatTree -Paths @($splicedPath) `
        -SpliceOwners $concatSplices -DepInfoOwners $depOwners
    Assert-Case 'the dep-info layer is ADDITIVE: both packages are named' (
        ($withDepInfo.Packages -contains 'holds_it') -and ($withDepInfo.Packages -contains 'pulls_it'))

    # kills: continuing past a failed compile-only pass. The first version of
    # this printed a notice and carried on, reasoning that a missed edge only
    # costs WIDTH. In -Mode Iterate that reasoning is false: the package this
    # layer would have added is by construction one the run does NOT otherwise
    # select, so nothing downstream re-encounters the failure and the run ends
    # green having skipped the consumer the changed file actually breaks.
    Set-Content -LiteralPath (Join-Path $nativeRoot 'holds_it\src\lib.rs') -Encoding utf8 -Value `
        'pub fn f() { this is not rust }'
    Assert-Throws 'a FAILED dep-info compile is a refusal, not a silent fall back to the floor' `
        { Get-BatteryDepInfoOwners -RepoRoot $concatTree -CargoRoots $concatRoots } `
        'exited'
}
finally { Remove-Item -LiteralPath $concatTree -Recurse -Force -ErrorAction SilentlyContinue }

# kills: `return $set` instead of `return ,$set`. PowerShell ENUMERATES a
# HashSet across a function boundary: the caller gets an Object[] whose
# .Contains() is ordinal and case-SENSITIVE, or -- for a single member -- the
# bare string, whose .Contains() is a SUBSTRING test. Either one turns a
# registered package whose manifest path differs only in casing into an
# "orphan" and refuses every mode of the battery. This asserts the type
# survives, because the symptom is a false refusal nowhere near the cause.
$caseTree = Join-Path ([IO.Path]::GetTempPath()) ("hoi4_partition_case_{0}_{1}" -f $PID, [guid]::NewGuid().ToString('N'))
try {
    New-SyntheticCargoWorkspace -Root $caseTree
    $savedRoots = $script:BatteryCargoRoots
    try {
        $script:BatteryCargoRoots = @('extractor\native')
        $registered = Get-BatteryRegisteredPackageManifests -RepoRoot $caseTree
        Assert-Case 'the registered-manifest set survives the function boundary as a HashSet' (
            $registered -is [System.Collections.Generic.HashSet[string]])
        $anyManifest = @($registered)[0]
        Assert-Case 'the registered-manifest set compares manifest paths CASE-INSENSITIVELY' (
            $registered.Contains(([string]$anyManifest).ToLowerInvariant()) -and
            $registered.Contains(([string]$anyManifest).ToUpperInvariant()))
    }
    finally { $script:BatteryCargoRoots = $savedRoots }
}
finally { Remove-Item -LiteralPath $caseTree -Recurse -Force -ErrorAction SilentlyContinue }

# kills: probing an [ordered] dictionary with $Selection.PSObject.Properties.Name,
# which lists the dictionary's .NET members (Count, Keys, Values) and NEVER a
# key -- so the probe was always false and the summary printed the
# computed-literal limitation even on the runs that had closed it.
$orderedProbe = [ordered]@{ DepInfoUsed = $true; TouchedPackages = @() }
Assert-Case 'a selection key is found through Keys, which is how the summary must probe it' (
    @($orderedProbe.Keys) -contains 'DepInfoUsed')
Assert-Case 'a selection key is NOT found through PSObject.Properties (the defect this replaced)' (
    $orderedProbe.PSObject.Properties.Name -notcontains 'DepInfoUsed')

# ==========================================================================
# GROUP: THE RUNNER ACTUALLY WIRES THESE IN
# ==========================================================================
#
# kills: a helper that is correct, tested, and CALLED BY NOBODY. Adversarial
# review demonstrated both holes by deletion: removing the runner's orphan-guard
# call left all six orphan cases green, and removing the -DepInfo forwarding
# left all eight dep-info cases green. These assertions are source-level on
# purpose -- driving the whole runner from here would take the machine-wide
# battery mutex, which is what this suite runs INSIDE.
$runnerSource = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'Invoke-CentralRustValidation.ps1') -Raw
Assert-Case 'the runner CALLS the orphan guard' (
    $runnerSource -match 'Assert-BatteryEveryPackageIsRegistered\s+-RepoRoot')
Assert-Case 'the runner forwards -DepInfo into the affected selection' (
    $runnerSource -match 'Get-BatteryAffectedSelection[\s\S]{0,400}?-DepInfo:\$DepInfo')
Assert-Case 'the runner forwards -Features into the affected selection too' (
    $runnerSource -match 'Get-BatteryAffectedSelection[\s\S]{0,400}?-Features\s+\$Features')
Assert-Case 'the runner declares the -DepInfo switch' (
    $runnerSource -match '\[switch\]\$DepInfo')
# kills: accepting -DepInfo in a mode that has no diff-derived selection to
# refine, where it would be a silent no-op the caller reads as precision.
Assert-Case 'the runner refuses -DepInfo outside -Mode Iterate' (
    $runnerSource -match "-DepInfo applies to -Mode Iterate only")
# kills: reintroducing a hand-kept root list in the runner. Three copies is how
# one crate came to be missing from all of them.
Assert-Case 'the runner derives its standalone roots from the ONE inventory' (
    $runnerSource -match '\$allCargoRoots\s*=\s*@\(Get-BatteryCargoRootPaths')
Assert-Case 'the runner no longer hand-lists the portable roots' (
    $runnerSource -notmatch 'Join-Path \$repoRoot "portable_replay\\native\\admission"')

# kills: mapping a changed path to nothing and calling that "no packages
# affected". A path under no package must be REPORTED as unowned, so the runner
# can say what its cargo selection does not cover.
$tree = New-SyntheticTree -Packages @{ only_package = 'pub fn f() {}' }
try {
    $owned = Get-BatteryPackagesForPaths -RepoRoot $tree -Paths @('documents/NOTES.md')
    Assert-Case 'a path under no cargo package is reported unowned, not silently dropped' (
        (@($owned.Unowned).Count -eq 1) -and (@($owned.Packages).Count -eq 0))
    # kills: nearest-ancestor picking the PARENT of a nested package.
    $owned = Get-BatteryPackagesForPaths -RepoRoot $tree -Paths @('extractor/native/only_package/src/lib.rs')
    Assert-Case 'a source file maps to the package whose directory contains it' (
        $owned.Packages -contains 'only_package')
}
finally { Remove-Item -LiteralPath $tree -Recurse -Force -ErrorAction SilentlyContinue }

# kills: resolving a caller's -Package name outside the roots the run enters,
# which lets `-Workspace Replay -Package <an extractor crate>` skip every root
# and return green.
$tree = New-SyntheticTree -Packages @{ in_scope = 'pub fn f() {}'; out_of_scope = 'pub fn g() {}' }
try {
    $edges = New-Edges -Root $tree -Map @{
        partition_test_anchor = @()
        in_scope              = @()
        out_of_scope          = @()
    }
    $manifests = Resolve-BatteryPackageNames -RepoRoot $tree -Names @('in_scope') `
        -ScopePackages @('in_scope') -DependencyGraph $edges
    # kills: returning the WRONG manifest while still returning exactly one.
    # Counting alone passes for any single answer, which is the weak form of
    # this assertion.
    Assert-Case 'a name inside the run''s scope resolves to its OWN manifest' (
        (@($manifests).Count -eq 1) -and
        ([IO.Path]::GetFullPath([string](@($manifests)[0])) -ieq
            [IO.Path]::GetFullPath((Join-Path $tree 'extractor\native\in_scope\Cargo.toml'))))
    Assert-Throws 'a -Package name outside the roots this run enters is refused' `
        { Resolve-BatteryPackageNames -RepoRoot $tree -Names @('out_of_scope') `
                -ScopePackages @('in_scope') -DependencyGraph $edges } 'no cargo root this run enters'
    # kills: searching EVERY repository-local id for a name match regardless of
    # which roots the run enters. A same-named package in an excluded root then
    # makes an unambiguous caller fail as "ambiguous". -ScopeRoots is what
    # narrows the candidates to the roots actually being driven.
    $twinRoot = Join-Path $tree 'extractor\native\twin_elsewhere'
    New-Item -ItemType Directory -Path (Join-Path $twinRoot 'src') -Force | Out-Null
    $twinEdges = $edges
    $twinEdges.IdToName['in_scope_twin'] = 'in_scope'
    $twinEdges.IdToManifest['in_scope_twin'] = (Join-Path $twinRoot 'Cargo.toml')
    $twinEdges.LocalIds = @($twinEdges.LocalIds) + @('in_scope_twin')
    Assert-Throws 'two manifests sharing a name are ambiguous when both are in scope' `
        { Resolve-BatteryPackageNames -RepoRoot $tree -Names @('in_scope') `
                -ScopePackages @('in_scope') -DependencyGraph $twinEdges } 'more than'
    $scoped = Resolve-BatteryPackageNames -RepoRoot $tree -Names @('in_scope') `
        -ScopePackages @('in_scope') -DependencyGraph $twinEdges `
        -ScopeRoots @((Join-Path $tree 'extractor\native\in_scope'))
    Assert-Case 'the same name is unambiguous once the candidates are scoped to the entered roots' (
        (@($scoped).Count -eq 1) -and
        ([IO.Path]::GetFullPath([string](@($scoped)[0])) -ieq
            [IO.Path]::GetFullPath((Join-Path $tree 'extractor\native\in_scope\Cargo.toml'))))
}
finally { Remove-Item -LiteralPath $tree -Recurse -Force -ErrorAction SilentlyContinue }

# kills: exporting a fourth hand-kept list of cargo roots in the runner. The
# roots the iteration mode drives must be the roots the partition classifies.
$rootPaths = Get-BatteryCargoRootPaths -RepoRoot 'C:\somewhere'
Assert-Case 'the exported cargo-root list is the partition''s own' (
    @($rootPaths).Count -eq @($script:BatteryCargoRoots).Count -and
    $rootPaths[0] -ceq (Join-Path 'C:\somewhere' $script:BatteryCargoRoots[0]))
# kills: the case above being VACUOUS for the thing it is actually about. It
# pins the helper's projection, and a runner that stopped calling the helper --
# or grew a fourth hand-kept root list beside it -- left it green. The runner is
# read as text because it cannot be dot-sourced (loading it runs a battery).
$runnerText = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'Invoke-CentralRustValidation.ps1') -Raw
# TWO call sites, counted, and the count is the assertion. One occurrence was
# vacuous for the thing this case is about: the runner asks the helper twice --
# once for the root-membership census that decides which selected packages are
# selectable at all, once for the loop that DRIVES the standalone roots -- and
# an occurrence check stayed green when the driving loop was replaced by a
# hand-kept list, because the census still matched.
$rootHelperCalls = @([regex]::Matches($runnerText, 'Get-BatteryCargoRootPaths\s+-RepoRoot')).Count
Assert-Case 'the runner drives BOTH the membership census and the iteration roots through the exported helper' (
    $rootHelperCalls -ge 2)

# kills: parsing a `-z` git listing with a newline splitter. The records are
# NUL-terminated and PowerShell has already split the stream on newlines.
$records = Split-BatteryNulSeparatedOutput -Raw @("a/b.rs`0c/d.rs`0")
Assert-Case 'a NUL-separated listing splits into its records' (
    @($records).Count -eq 2 -and $records[0] -ceq 'a/b.rs' -and $records[1] -ceq 'c/d.rs')
Assert-Case 'an empty listing is no records, not one empty one' (
    @(Split-BatteryNulSeparatedOutput -Raw $null).Count -eq 0)

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
