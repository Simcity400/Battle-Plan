# D54 retained offline evidence inventory

Status: **retained corpus narrowed; no remaining fixture retired**.

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

It does **not** prove the requested same-instance lifecycle. The 52/64 wire has
no `order+0x244` instance ID, so pointer/slot reuse cannot be excluded. No
retained record uses the current identity-bearing 60/88 wire. The fixture is
therefore narrowed to one current-wire baseline, redraw, and deletion sequence;
the existence of redraw and tombstone behavior no longer needs rediscovery.

### Type-4 discrimination

There are zero type-4 rows in the decoded stream evidence and zero type-4
orders in all three melted saves. The only substantive save is
`SOV_1946_05_19_09_melted.hoi4` (SHA-256
`7e2d7650fc3eb6d4f1442bad98e2c3fb79fdbe43b2bf32027a74b029233f77ee`):
343 armies, 384 orders, and type counts 1/2/3/5 = 44/181/51/108. A type-4
paradrop must still be created and captured at one authenticated live/save
boundary.

### Membership equality and lifecycle

The SOV save contains 3,514 scheduled-member references, but the retained D54
stream wire predates assigned-division child IDs. Consequently there is no
possible exact set-equality join between retained live rows and save membership,
and no retained assignment/removal lifecycle. This is a schema absence, not a
parser gap. The remaining fixture needs current 60/88 D54 rows and a same-paused
save before/after one membership transition.

### Empty/inactive manager coverage

The SOV save has no zero-member army and 83 armies with zero orders. The other
two melts contain no parsed armies. The decoded D54 records contain zero empty
direct-army-root observations and no current exact inactive rows. Even an empty
legacy direct-root row would not prove a bounded complete manager walk. A
manager/container receipt that enumerates known active, inactive, and
zero-member fixture armies remains required.

## Exact remaining human boundary

1. In one paused session, capture a type-2 order on the current D54 wire at
   baseline, after extending/shortening it, and after deletion. The same army ID
   and `+0x244` instance ID must span baseline/redraw, the ordered path must
   change within one generation, and the old identity must tombstone on delete.
2. Create one type-4 paradrop and capture its raw live geometry plus a melt made
   at that same paused boundary. Promote geometry only on exact live/save
   agreement; otherwise keep type 4 unsupported.
3. At one paused boundary, capture the exact assigned-division ID set for one
   order and the matching save scheduled-member set; then repeat after one
   assignment and one removal.
4. Capture a bounded manager/container walk containing one known active army,
   one known inactive army, and one known zero-member army before/after adding
   and removing a division. Arena scans and cached-address repair remain
   inadmissible.

The 510.6 GB compressed logical corpus is an explicit non-human, machine-load
deferment rather than a reverse-engineering fixture. Its sampled records all use
the legacy precursor schema, so it cannot supply the missing `+0x244`, assigned
IDs, or exact active fields; a future idle-time full sweep may only discover
additional legacy type occurrences or surrogate lifecycle candidates.
