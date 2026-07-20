# H-A31 live national-focus item owner closeout

Exact a729 static RE now closes the missing live owner and producer route. The
object passed in `RCX` to `0x141371b70`/`0x14136f750` is a
`CNationalFocusView`; its constructor at `0x14136AEC0` installs primary vtable
`0x14299D680`. The view directly owns the final `CNationalFocusTreeItem*`
vector at `+0x2058` (data), `+0x2060` (capacity), `+0x2064` (count), and
`+0x2068` (allocator). The array's `0..count-1` sequence after
`0x14136F750` returns is the engine-filtered and engine-sorted visible order.

Each `0x550`-byte item has exact vtables `0x14299DD40` at `+0x00` and
`0x14299DDE0` at `+0x18`, its `CNationalFocus*` at `+0x20`, its final
presentation code at `+0x540`, and visibility capability bytes at
`+0x54C/+0x54D`. Replay identity is the existing authenticated definition id at
`CNationalFocus+0x18`, never either live pointer.

The dynamic route is distinct from publication:

- `0x14137A780` walks the exact `+0x2058/+0x2064` vector and passes each item
  to `0x141379A50`.
- `0x141379A50` resolves current-country and joint-focus state and calls
  `0x1413778F0`, which commits the final `0..3` code at item `+0x540`.
- `0x141377430(item,state)` owns search-highlight presentation; state `3` is
  highlighted. This is transient and must be hooked, not invented as an item
  byte.
- `0x141BB5DF0` owns display percent. Its detail object carries the definition
  pointer at `+0xF50`; at `0x141BB636E` the exact final integer is moved from
  `ESI` to `EDX` and consumed at `0x141BB6373`. This is the observation
  boundary, not `progress_days / (cost*7)`.
- Historical presentation remains the final byte at definition `+0x5B8`.

The observer must authenticate the view and both item vtables, double-read the
header/pointer array/items, join transient events by item or definition pointer,
replace all pointers with stable definition ids, and commit only a complete
single-country/single-hour epoch. Missing or unstable leaves reject the whole
frame.

No production source was activated in this pass. The fresh discovery export is
job `20260720T011842Z-wave3-h-a31-live-focus-vector-owner-prod-5e55b0d8`,
manifest SHA-256
`bd47df5b3578db6164f643c376fc70339f38b7339044e468c3d56c0fe3b82b69`.
It proves the route. The consolidated exact20 authority is now complete as job
`20260720T020812Z-w3-a31-focus-owner-93fbbf62` on `shard-17`, with 10/10 exact
requested roots and manifest SHA-256
`303351ed65468aa7655b904998819c96ca1c325f906b5a3bf32c4f18decc7969`.
This closes the static receipt only. Source activation remains false until an
authenticated complete pointer-free fixture proves the observer output;
central validation remains required after that activation change.

The subsequent production-wiring pass implemented reserved compact domain 89
(`NFP1`): the fail-closed process-local owner/item producer, stable-id compact
codec, collector seam, recorder fold, typed replay view, and exact XUI10
admission. The static receipt is bound by both manifest SHA-256 and job JSON
SHA-256. This does not change the final activation sentence above: scheduling
and live-fixture acceptance remain false until the authenticated stable plus
transition fixture exists.

The Wave4 acquisition pass then closed the remaining no-caller gap. The
injected collector now has an owner-runnable, evidence-only hook transaction
and a fail-closed receipt writer/regrader; see
`A31_ACQUISITION_OWNER_20260720.md`. This makes the live fixture collectible
without changing either d89 activation latch.
