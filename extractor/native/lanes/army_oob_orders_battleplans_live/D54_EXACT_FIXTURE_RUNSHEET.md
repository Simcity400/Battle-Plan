# D54 exact fixture runsheet

Authority: the game `1.19.2.0.a729`, SHA-256
`870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.

Carrier contract: `D54_EXACT_FIXTURE_CARRIER_CONTRACT.json`
Observer: `d54_exact_fixture_observer.py`

This runsheet describes a future authenticated harness session. It does not
authorize a launch, attach, build, test, Ghidra job, or production activation.
D54 static and exact-10 authority are already complete. The S4 offline
publication/repack gate and S8 same-boundary save-oracle observation are not
executable RE lanes. None has a missing Ghidra seed; only the current-60/88-wire
observations below remain for this carrier.

## Capture invariants

- Use one process and paused boundaries. Give every boundary a unique ID and a
  shared token used by its live and melted-save observations.
- Capture current domain 54 only: header length 60, row length 88. Preserve the
  raw 32-byte H4CR header, payload SHA-256, stream sequence, game hour, record
  offset, row index, flags, and raw 88-byte row.
- For each live order, preserve the raw order pointer and exact bytes at
  `+0x30`, `+0x38`, `+0x70`, `+0x78`, `+0x7C`, `+0xB8`, `+0x210`, `+0x21C`,
  `+0x244`, and `+0x248`. Preserve path and assigned-division arrays in
  engine order.
- Never replace a pointer, raw byte window, or ordered array with a sorted or
  deduplicated convenience view. Set comparison is secondary and must leave the
  source arrays intact. Preserve save members as ordered typed `{id,type}`
  references as well as the derived ordered integer ID array.
- Input `activation_requested` is `false`. The output carrier keeps every
  activation false even when all observation receipts are complete.

## Boundary sequence

| Fixture | Ordered phases | Required mechanical receipt |
|---|---|---|
| Type-2 redraw/delete | `baseline`, `redraw`, `delete` | Baseline/redraw retain raw pointer, army ID, `+0x244` instance ID, and generation; ordered `+0x70` path with live count `+0x7C` changes; `+0x78` capacity is retained diagnostically; delete row is the same-identity tombstone. |
| Membership lifecycle | `baseline`, `assign`, `remove` | Same raw pointer and order identity; live ordered `+0x210/+0x21C` IDs equal same-boundary save scheduled-member IDs as duplicate-free sets; transitions add then remove exactly one member. |
| Empty/inactive manager | `baseline`, `add_member`, `remove_member` | Stable container; producer/consumer-rooted chain; no arena scan or cached repair; raw descriptor; exact contiguous bounds; ordered entries containing active, inactive, and zero-member fixture IDs; same-boundary save membership; add/remove restores zero-member baseline. |

The membership baseline needs at least two divisions. The manager fixture uses
three distinct army IDs. Each fixture army entry retains its raw army window,
enumeration index, pointer, member IDs, fixture role, and the observation that
establishes the active/inactive/zero-member classification.

Type-4 is no longer a fixture discriminator. Exact a729 consumers
and `+0xB8` as target; production carries exactly that ordered pair. Type-1
uses ordered `+0x70` data with live count `+0x7C`. The remaining rows are optional runtime
acceptance for lifecycle behavior, not missing reverse engineering.

## Deferred use

When an operator supplies the completed observation JSON, the observer's
`build` command can normalize it into the carrier. An incomplete or malformed
observation produces no carrier. A complete carrier is still evidence only:
promotion requires human review and a separate production source change.
