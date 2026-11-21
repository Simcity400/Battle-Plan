# Preserve exact award intervals across concurrent writers

## Experience gains

Experience gains sum exact positive awards after native modification and clamping. Concurrent writers lease the table before checking its gate. An exclusive drain closes the gate before checking for leases. This ordering either exposes the lease to the drain or makes the writer refuse without touching the table. A writer that crosses the drain or refuses at its gate withholds the whole interval. The next exclusive reset discards pending sums from that interval before establishing a new census.

Stable owner pointer and link words claim an open-addressed slot once per interval. The claimed-slot list bounds reset work. A power-of-two table mixes strided addresses so closely allocated owners can use distinct probe chains. Preparation sizes for the live census and allows births and replacements to consume spare slots. A larger offered table waits for an exclusive reset. The worker then reclaims the preceding table. A failed publication retains memory until runtime has authenticated the hook teardown and shutdown of capture workers.

`Lanes::record` keeps its writer lease while dereferencing the published table. Replacement takes place with the gate closed and no writer lease left. Final release follows hook quiescence. Both award families' `Hooks::new` constructors require the caller to own the family exclusively and supply the memory source and base for the current process's authenticated executable. Before clearing that ownership or unloading, the caller restores hooks and proves native calls are quiescent.

Each live country gets one row. A matching authenticated owner at consecutive boundaries certifies its interval, including exact zero. Deleted instances contribute no current row and their awards cannot transfer to a reused address or alias. A successful reseed allows a later complete interval. Births and replacements still lack a complete daily window. The tests exercise writer and drain overlap, geometry changes during preparation and census orders that differ from replay key order.

## Fuel awards

Each native fuel receipt records its attempted ordinal before it claims capacity. Producers use unique entry slots and preserve their names until the consumer makes both values owned. A receipt that cannot use the live buffer can spill into independent storage. A lost receipt withholds its interval. Entry and name pressure are counted independently, including losses and a named publisher whose raise is still in flight. Replaced buffers are passed to the boundary through atomic offers and reclaimed by the spool off-thread.

The `Sync` implementation for the fuel `Slot` relies on single-producer slot ownership and the consumer advancing `LOG.consumed` only after copying both the entry and name. In `offer`, only the caller that removes a pointer through the atomic swap can take ownership of that buffer. A displaced offer can be freed directly because it was still pending. The optional diagnostic `TagTimer::new` and `TagTimer::drop` use `_rdtsc` on x86-64, where the intrinsic has no caller preconditions.

The boundary can seal a live buffer after admitted publishers finish, preserving its frontier while new and stale callbacks follow the same successor. A sealed predecessor remains readable until both its entries and names are consumed and no publisher can still reach it. A drain advances only through settled attempts and leaves pressure after its frontier for the next interval. Admission closes before detach checks callback and native instruction-pointer quiescence.

Only later calls of the same fuel update can take a pending root's authenticated location. World, hour and event epochs must all match. Reset, nesting and a lost receipt revoke the root and require a fresh observation. Reset can occur before the air registry exists and may invalidate stored history even when country identity cannot yet resolve. Tap timing diagnostics are independent of publication ordering and count every observed call when enabled.

The hourly handoff is consumed even if a later recording write fails. Each country combines 24 consecutive authenticated updates from the same branch. Repeated roots, resets, refusals, gaps and a reopened campaign interrupt that window. Recipients belong to the root whose authenticated end closes their interval, and maintain full native identity, name and name reason. Day totals retain each receipt identity's earliest and latest hour regardless of sort stability. An overflowing native-width sum refuses the day and its receipts. Complete daily packs capture the Fuel widget's costs, with each own gain converted before summation.

[Experience capture](../../crates/collector-experience-gains/src/experience_gains.rs) · [Fuel receipts](../../crates/collector-fuel-awards/src/fuel_awards.rs) · [Fuel window assembly](../../crates/collector-fuel-awards/src/tables.rs)
