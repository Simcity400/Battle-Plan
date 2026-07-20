# X9 specialization token-namespace promotion — 2026-07-20

Portable X9 rows additionally require the terminal D41 `PPA1` seal and its
payload/sequence/hour/count/status/D18 cross-joins. Replay, Python validation,
and UI reject inner-only or mutated authority; raw serializer handles remain
restricted to compact delta state and never enter portable JSON.

## Outcome

X9 no longer needs a human-created four-value save/live permutation. Fresh exact-a729 RE proves
that the `CBreakthroughProgress` tree key is the engine serializer token id, which is the same
namespace exposed by the collector's authenticated runtime token-record dictionary.

The production route stays fail-closed. D41 still uses its existing 16-byte breakthrough row. Its
flags now carry a closed exact-name enum only after a complete heap census has found one
unambiguous token record whose text is exactly one of:

1. `specialization_nuclear`
2. `specialization_naval`
3. `specialization_air`
4. `specialization_land`

Unknown, modded, not-yet-scanned, or ambiguous handles remain numeric and fold with
`specialization: null`. No name is inferred from handle order, value correlation, save order, or
the retained two-handle streams.

## Exact engine join

- `0x1419F14F0`, the `CBreakthroughProgress` writer, walks the ordered tree and calls
  `0x1424ADFE0(serializer, node+0x1C, node+0x20)`. The first value is the token key and the second
  is `points_raw`.
- `0x1419F10C0`, the corresponding reader, takes the parser token id at `parser+0x30`, inserts it
  as the ordered-tree key at `node+0x1C`, and writes the parsed value at `node+0x20`.

That bidirectional serializer law closes the namespace join which the prerequisite re-audit had
correctly left open. The absence of a static `specialization_air` executable literal remains true
but is no longer a blocker: authored game-file tokens are materialized in the runtime token
registry, and semantic admission requires the exact registry text.

## Carrier and consumers

The row remains:

`tag_db_id:u32, specialization_handle:u32, points_raw:u32, flags:u32`

D41 header extension 5 is the explicit semantic-version boundary. Extensions 0 through 4 require
the specialization bits to remain zero, so previously reserved values cannot be reinterpreted as
exact authority.

- bit 0: semantic enum present;
- bits 1–3: 1 nuclear, 2 naval, 3 air, 4 land;
- bit 31: deletion.

The recorder validates flag combinations and deletion purity, folds the exact name and authority,
and preserves raw handles. Replay core rejects raw-only, unknown, partial, duplicate, unresolved
country-owner, or forged semantic rows from its typed exact view. The game UI consumes only that
typed view.
The replay app exposes country-scoped and global breakthrough admission through the same adapter;
raw D41 JSON does not cross that product seam.

Machine-readable evidence and receipt hashes are in
`X9_SPECIALIZATION_TOKEN_NAMESPACE_PROMOTION_20260720.json`.

## Validation boundary

No Cargo command, build, test, verifier, compiler, or formatter was run in this subagent lane. The
root agent must run centralized validation.
