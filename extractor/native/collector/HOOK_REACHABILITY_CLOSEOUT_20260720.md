# Hook reachability closeout (2026-07-20)

This closes three static production-reachability gaps without changing any
lane-owned activation decision.

## Shared runtime-text teardown

- The collector now tears down A39 once after the last compact frame and every
  terminal producer drain. A39 closes callback admission, disables its four
  detours, waits for in-flight callbacks, and removes its hook set.
- Only after A39 succeeds does the collector call A25's sole runtime-text hook
  uninstall once. The A25 hook owner now also counts all three callback kinds
  and waits for quiescence after disable and before trampoline removal.
- A failed disable, quiescence wait, or removal is unload-unsafe. The attach
  worker retains its collector module reference and refuses a second attach
  attempt instead of unloading code which a detour may still reach.

## A33/d86 scheduling reachability

- A33 evidence automation remains a raw JSONL sidecar and cannot enter H4CR.
- Production d86 enablement requires the lane-owned
  `PRODUCTION_SCHEDULING_ENABLED` latch plus an explicit compact
  `sunk_ship_history_live` request.
- Once that production condition is true, every scheduled hour appends one d86
  perf record and one SHP1 domain-data record immediately after the old A33
  queue drain and the new d49/d50 member-epoch publication. Source, clock, or
  activation failure emits typed source-unavailable SHP1 data; it is never
  interpreted as measured-empty history.
- The current scheduling and live-fixture latches remain false.

## A25 owner-glyph evidence separation

`install_dormant_evidence_automation` now checks the lane-owned false
`OWNER_GLYPH_CAPTURE_ACTIVATION` evidence latch before creating state. Linking
the helper or calling it from a new site
cannot activate the harness probe, and evidence mode remains mechanically
distinct from production d76 promotion.

Static regression tests cover teardown cardinality/order, fail-closed A39
quiescence, the d86 scheduling latch, and the A25 evidence latch. Central Rust
validation remains root-owned and was not run for this edit.
