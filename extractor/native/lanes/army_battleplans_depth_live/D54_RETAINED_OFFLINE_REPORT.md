# D54 retained offline evidence inventory

Status: **retained corpus exhausted; all former reverse-engineering fixtures
retired by exact a729 producer/consumer evidence**.

Machine-readable inventory: `D54_RETAINED_OFFLINE_INVENTORY.json`
Reproducer: `d54_retained_inventory.py`

## Scope and provenance

The inventory is read-only and deterministic. It parses compact `H4CR` record
headers, hashes the concatenated raw D54 records, records source byte lengths
and adjacent manifest hashes, and parses the three retained melted saves with
the repository save oracle. It does not launch the game, attach to a process, use
Ghidra, compile code, run tests, or act as a promotion verifier.

The retained stream corpus contains 113 files:

- 23 raw streams were walked completely: 3,199 D54 data records and 10,956 D54
  rows.
- 90 streams are Zstandard-compressed (4,002,077,509 stored bytes;
  510,579,708,465 manifest-declared decoded bytes). To honor the workstation
  load boundary, the inventory performs a bounded first-D54 schema probe for
  78 streams and reaches clean EOF in the other 12. Those probes decoded only
  236,159,817 logical bytes. A full compressed sweep remains available through
  the explicit `--include-zstd` option but was not run.
- Every decoded D54 data record uses the 52-byte header / 64-byte row family:
  2,051 records have zero auxiliary words and are structurally ambiguous
  between the old direct-root and precursor meanings; 1,226 are mechanically
  the precursor layout. There are zero current 60-byte header / 88-byte row
  records.

The completely walked raw streams contain live rows of types 1/2/3/5 in counts
19/5,669/517/3,263. The compressed first-record probes add
14/14,478/1,290/12,229 observations. No decoded stream observation has type 4.

## What the retained rows establish

### Type-2 redraw and deletion

The complete raw streams contain 59 type-2 ordered-path changes under the
legacy surrogate key `(army_id, generation, slot)` and 768 tombstones that have
a prior live row under that surrogate key. This mechanically proves that the
legacy delta corpus contains path mutation and deletion behavior.

The 52/64 wire cannot by itself prove same-instance lifecycle because it has no
`order+0x244` instance ID. That retained limitation is no longer an RE blocker:
the exact a729 assignment/removal, serializer/loader, and order-identity chains
prove the current identity-bearing law, and the production owner fixture covers
in-place update, generation split, and identity-bearing tombstones. A new live
sequence is optional behavioral corroboration only.

### Type-4 discrimination

There are zero type-4 rows in the decoded stream evidence and zero type-4
orders in all three melted saves. The only substantive save is
`SOV_1946_05_19_09_melted.hoi4` (SHA-256
`7e2d7650fc3eb6d4f1442bad98e2c3fb79fdbe43b2bf32027a74b029233f77ee`):
343 armies, 384 orders, and type counts 1/2/3/5 = 44/181/51/108. A type-4
fixture is therefore absent from retained captures. Exact a729 consumers
`order+0x70` element as source and `u32 order+0xB8` as destination; the
serializer/loader preserve both fields. Production now carries exactly that
pair. No human-created paradrop is required to determine the geometry.

### Membership equality and lifecycle

The SOV save contains 3,514 scheduled-member references, but the retained D54
stream wire predates assigned-division child IDs. Consequently there is no
possible exact set-equality join between retained live rows and save membership,
and no retained assignment/removal lifecycle. This is a retained-schema
absence, not a parser gap. Exact a729 mutation roots and the current pointer-free
carrier now close the production law; a before/after live capture is optional
acceptance evidence, not missing RE.

### Empty/inactive manager coverage

The retained corpus remains a useful negative: it has no manager carrier. That
negative no longer creates a human-input requirement. Exact a729 registered-
reference functions prove the complete type-53 table and iteration boundary,
and production now emits the pointer-free `OGS1` census including zero-order
armies. See
`../army_oob_orders_battleplans_live/ORDERS_MANAGER_SCALARS_RE_20260720.md`.

## Human-input disposition after evidence exhaustion

No human input remains for X1/X8 reverse engineering or production semantics.
The formerly requested redraw/delete, paradrop, and membership transitions are
retained as optional acceptance rows only. They cannot activate or change the
already authenticated production law. See
`X1_X8_TYPE1_TYPE4_SCALAR_RE_20260720.json` and
`X1_X8_PRODUCTION_CLOSEOUT_20260720.json`.

The 510.6 GB compressed logical corpus is an explicit non-human, machine-load
deferment rather than a reverse-engineering fixture. Its sampled records all use
the legacy precursor schema, so it cannot supply the missing `+0x244`, assigned
IDs, or exact active fields; a future idle-time full sweep may only discover
additional legacy type occurrences or surrogate lifecycle candidates and is not
a production-promotion prerequisite.
