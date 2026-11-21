# Capture operands for raid target evaluation

## Regional air scores

Read the stored country/region pair of superiority weights, land-movement impact and active-mission counters together. The pairs also drive the Air map mode. At hourly-update entry the game has advanced its hour and completed the previous air update. The regional phase pass rewrites only positive region slots with provinces that match the hour's signed remainder from the phase epoch over twice the current combat delay. The same phase rebuilds the aircraft spotting scale. Check that the signed remainder exists.

After a complete read, refresh the phase slots and carry the others in slot order. A gap or a different period requires a new complete read. A moved score array or changed raw country identity invalidates that country's pairs. A country strategic-air reset can clear all its pairs in place outside its phase. The collector requires a matching certification token covering the reset's two site-list clears, and repeats the full read when that token is absent or changes. Runtime slots are merge aids. Only recorded country and region identity survives.

An absent pair means zero only within a complete table proved by that country's last-region witness. The omission rule requires zero weights, zero active-mission counters and an observed zero spotting scale. Keep the witness even when the pair itself is zero. Worker storage grows from the live roster and retained pairs, including shared menu allocations.

Menus follow the pair's read boundary. Keep equal held menus without a refcount change, and share the observed empty menu. Unreadable menus stay missing. Only the observer's entries accumulate contribution maps. Read each nonempty map in order and bound the walk by its declared count, validating each parent link used by the successor walk. An empty map needs neither a node nor a sentinel read. Reuse scratch up to the largest map the worker has read.

[Regional air scores](../../crates/engine-raid-inputs/src/raid_air_scores.rs) · [Air power menus](../../crates/engine-raid-inputs/src/raid_air_scores/menu.rs)

## Facilities, targets and intelligence

The target tooltip selects the first matching province in its controller's program status. Preserve program order and each occurrence, including duplicates. The optional project's definition comes from its current pool entry, even when a missing project leaves a token in the program. Sample hourly because project operations can change the answer between daily histories.

Keep simulation detection from the stored target roster, where invalid targets and detection-required undiscovered buildings are suppressed. The target roster preserves its validity field (save token `0x4af6`) and each country's detection bit, including the undetected result outside the vector. Map buildings use the same bit. Authenticate optional scope arms separately. A zero building type denotes the null arm. An unproven leader captor refuses that row, counts as truncation and does not increase storage demand. Sizing uses headers, and the worker rechecks each original header as it acquires rows.

National counterintelligence uses the strategic-operative mission roster and country modifier operands. Agency active rosters and the agency's differently transformed cache answer different questions. Match the effective getter's lower-bound order, aliases, zero tag and eligibility checks. Preserve the static nationality modifier choice when the sorted roster contains the subject. Copy the country accumulator once at the first key, then search both keys. An oversized, unreadable or otherwise unsuccessful copy uses the selected in-place reads and retains those reads' refusal reasons. A carry supplies only raw key answers under a matching authenticated header. Read the header and derive effective values from current definitions in either case. Size from all mission-roster occurrences before filtering. Report later growth from the walk.

[Program facilities](../../crates/engine-raid-inputs/src/raid_facilities.rs) · [Target detection](../../crates/engine-raid-inputs/src/raid_target_statuses.rs) · [Intelligence operands](../../crates/engine-raid-inputs/src/raid_intel_inputs.rs)

## Distance memo consistency

Land and naval paths keep separate session memos keyed by source and target province. Cache entries include fixed-point distance on success and an explicit failure state. The memo is absent from saves. The game holds its lock across lookups, insertions and rehashes. Capture samples the header and lock before and after copying, accepting only unchanged geometry and count with the lock free at both ends. A concurrent insertion changes count, and rehash changes geometry. An overlapping writer makes this hour unavailable and increments the torn-copy count. A quiet table that fails validation is malformed. The walk has one attempt.

An off-thread hint uses bucket geometry, including the probe tail, to budget output even when the live count grows. The sentinel is separate. Output orders pairs by source and target, retaining exact missing-row demand on capacity shortfall.

[Raid path memo](../../crates/engine-raid-inputs/src/raid_path_memo.rs)
