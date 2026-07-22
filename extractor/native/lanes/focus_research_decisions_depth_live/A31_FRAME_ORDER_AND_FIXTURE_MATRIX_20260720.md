# A31 frame-order and fixture-matrix closeout

Wave 5 found and closed one remaining acquisition defect without enabling
compact domain 89. The first owner-runnable implementation materialized its
candidate when `0x14137A780` returned. Exact-a729 frame callers instead run the
item-state update first and the detail/transient update second:

- `0x14137A180` calls `0x14137A780` at `0x14137A2B8`, then calls
  `0x14137A600` at `0x14137A2C0`.
- `0x1413762F0` calls `0x14137A780` at `0x14137633B`, then calls
  `0x14137A600` at `0x141376344`.
- `0x14137A600` walks the national detail vector and invokes the exact
  display-percent producer `0x141BB5DF0`, then walks the joint vector and
  invokes `0x141BB5A70`.

Materializing at the earlier return could not authenticate same-cycle display
percentage and joint-country leaves. The fixture hook graph now includes the
sentinel-gated `0x14137A600` entry. The `0x14137A780` return records a pending
dynamic refresh for one exact view; only a following `0x14137A600` return for
that same view consumes the pending marker and materializes the pointer-free
candidate. An independent detail refresh cannot reuse an earlier dynamic
update. The completion QPC in the receipt is now after both producer families.
The receipt also carries the arm QPC and requires a common-frequency
`arm <= completion <= selected Present` bracket.
All hook-family counters reset at arm, so each receipt proves invocation for
its own transaction rather than inheriting counts from an earlier fixture.
Sealing now rejects before any evidence file is written unless all six
function-entry hook families plus both inline boundary witnesses were observed,
and the vector-publish count agrees with the candidate's same-epoch Boolean.

The ordinary transition route `0x1413762F0` does not call the vector publisher.
The receipt therefore carries an explicit same-epoch-publish witness. The
stable member of the pair must have that witness; a later transition may omit
it only because the collector still authenticates the exact view and both item
vtables and proves identical opening/closing vector headers, pointer order, and
definition joins. A missing publisher is not treated as a missing vector, and
a nonzero publisher count must agree exactly with the Boolean witness.

Fresh exact-request-membership a729 authority is job
`20260720T103726Z-w5-a31-exact-frame-order-boundary-a49db01b`, manifest
SHA-256 `c3c2a38805153497ad3298893a1b101582aa0651604a742e78214108b7820745`,
and job-receipt SHA-256
`58ec9118271f47d517ed1b29cb3c900dccc4e0f60e553bbb9bbd47e3de4d082b`.
The two caller decompiles hash to
`ad32507288d515bf7735666d4d01cc635a2993e1b574d1481b4f097a2f60e38b`
and `7bc1ff401c76518f2860c547abfed7d41b31aa79184a174afc7c7734f350a084`.
The `0x14137A600` instructions hash to
`983cdac02fa4ed5422fa9dff55dce9bfdd8e377bd642f271b3799f1351261293`.
Its decompile hash is
`458ae468aa356b2f0212261b93346de4e16f2d9fcc8793215e3d2f93389415ae`
and proves the exact one-argument `void(longlong view)` ABI used by the detour.
Its independently dumped 16-byte entry sentinel is bound by job
`20260720T102614Z-w5-a31-detail-refresh-entry-sentinel-7f8e06ea`, artifact
SHA-256 `31966be8db16e8aaf55465e901b6d0e8b704f8ecf0af0b83371fc540bc69723b`.

The portable NFP1 authority fields use the superset 13/13 exact-root job
`20260720T104531Z-w5-a31-consolidated-owner-frame-order-pr-79b1ba63`, manifest
SHA-256 `b319dd1b721ed6a13e47088b7def034111f9882631080bffbf049fbb79265a2f`
and job-receipt SHA-256
`b1c9d8c0678956f17bea2e7f9e06e7ff4baf31254c11b3c983a9a5ca9ae2d04b`.

A final shard-18 re-audit used job
`20260720T134711Z-wave5-a31-owner-item-frame-order-selecte-7a69968f`
and resolved all 13 requested roots. Its manifest is
`b319dd1b721ed6a13e47088b7def034111f9882631080bffbf049fbb79265a2f`.
It confirms that `0x14136F750` receives the view as its first argument and
publishes the item vector at view offsets `+0x2058/+0x2064`, and that
`0x141379A50` has the currently hooked
`(item,country_context,char,float,context)` ABI. It also closes two misleading
interpretations: `0x141BB5DF0` returns no display percentage (the final integer
is in `ESI`, copied to `EDX` at callsite `0x141BB636E`), and `0x141BB5A70` is a
continuous-focus detail update rather than a joint-view country resolver.

NFP1 schema v2 now carries the paired-refresh completion frontier and the
canonical portable selected-Present receipt digest. The recorder fold
preserves those fields, and replay admits d89 only when the digest recomputes
against the selected Present in the production render frame. The raw injected
hook selection digest remains separately named in the acquisition receipt.
This removes the earlier schema-1/schema-2 dead route and prevents a
same-hour-but-foreign Present from being accepted.

The hook owner now installs both internal observations itself. A byte-gated
near relay at `0x141BB636E` captures the final ESI percentage and replays the
displaced `mov edx,esi; mov rcx,rax` before the original virtual call. A second
relay replaces only the direct call at `0x141379E5F`; it joins the exact
positive country DB-id to the item held by the enclosing `0x141379A50`
detour. The producer entry records explicit hidden first, so absence and a
missed positive callback cannot collapse into the same state.

Fresh inline-boundary authority is job
`20260720T213713Z-w5final-a31-exact-display-joint-instrume-56d3fc12`, manifest
SHA-256 `76cb5caf7dc418b5fbe79dac037440ccb3cecd34140ee9019ad33c96950e0273`
and job JSON SHA-256
`5c0dcd6032b2aa747ed02bec309c9c41d0697e445b46b86b91f9fb4132f5059a`.
The receipt regrader requires nonzero display-callsite and joint-boundary
witness counts in addition to the original six hook families. The joint count
is incremented only by the exact positive `0x141379E5F` callsite, never by the
producer's hidden default, so both fixture members must expose at least one
positive joint-country item.

`Invoke-A31NationalFocusFixturePair.ps1` and
`tools/capture_a31_fixture_pair.py` form the unattended live path. The wrapper
owns a clean exact-save launch and paused-readiness proof. The Python driver
discovers the sole exact process and SWE country object, invokes the injected
collector through its one-argument JSON command export, executes the frozen
1920x1080 focus actions, captures both receipts, and runs both receipt regrades
plus the pair regrade. It closes the focus panel while the two mid-function
sites are installed and restored. Neither path alters either activation latch.

`tools/verify_a31_fixture_matrix.py` closes the other static acceptance seam.
It independently regrades two committed NFP1 receipts, requires the same
country and a later transition hour, rejects a reused selected-Present receipt,
and requires at least one common stable focus ID whose final state, exact
display percent, presentation flags, or joint-country leaf changed. A changed
visible list alone is not a qualifying transition.

No retained `.nfp1` record or A31 acquisition receipt exists in the available
capture/evidence roots. Therefore `PRODUCTION_SCHEDULING_ENABLED`,
`LIVE_FIXTURE_ACCEPTED`, `PRODUCTION_ACTIVATION_AUTHORIZED`, and
`COMPACT_DOMAIN_89_SCHEDULED` remain false. Acquisition no longer requires a
person: run the wrapper after central validation. It binds the frozen save,
attested binaries, static bundle, action profile, address discovery, paired
receipts, and independent matrix regrade into one immutable admission.
