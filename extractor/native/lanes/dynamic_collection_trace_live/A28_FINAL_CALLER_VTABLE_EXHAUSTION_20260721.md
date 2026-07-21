# A28 final list, smooth-list, overlap, and controller exhaustion

Status: `STATIC_RE_EXHAUSTED_HOOKS_IMPLEMENTED_D71_UNSCHEDULED`.

Authority is `hoi4.exe 1.19.2.0.a729`, image base `0x140000000`, SHA-256
`870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.
The final consolidated export is job
`20260720T220101Z-w5final-a28-hook-and-residual-controller-16a41b38`, manifest
SHA-256 `5d878f9c1818bae247f5fc3c6c8c42766dba27841afbd63de69ea957d07505b4`.
It resolves all 26 requested roots exactly and exports 67 functions with
P-code. Hook entry bytes are independently retained by job
`20260720T215038Z-w5final-a28-tech-overlap-hook-sentinels-ef563094`, artifact
SHA-256 `3eba11027150a60121e8d45e92dbb6e21e4dc85d744c27baaa531a732460e94d`.

## Runtime family result

The facility `smoothListBoxType` is not a distinct unproven runtime class. Its
concrete `CFacilitiesTabView::SetupDerived @ 0x141C9D840` owner field `+0xB58`
is a standard-list object with primary/secondary vtables
`0x142B2CB48/0x142B2CC78`, a concrete `specialization_item` row with vtables
`0x142A49938/0x142A499A8`, and an extended scrollbar with vtables
`0x142B32FF0`, `0x142B33270`, and `0x142B33300`. The stored child chain is
head `+0x90`, count `+0xA0`, child `node+0`, next `node+0x10`; the scrollbar
pointer is `+0x190`. This exact join is implemented in
`src/a729_facility_smooth_list_hook.rs`.

The technology specialization badge controller is not inferred from generic
grid behavior. `0x141BC1A00` consumes the source vector returned by
`0x140B90430`, constructs the two-icon composite through `0x141BBB9F0`, inserts
through `0x1402DE580`, and positions through `0x1422CA2E0`. The concrete grid
vtables are `0x142B2F458` at object start and `0x142B2F6E0` at `+0x1F8`; the
composite-row vtable is `0x142A36B60`. The bracketing controller/position hook
is implemented in `src/a729_technology_badge_grid_hook.rs`; its source type
requires a same-transition owner-fixture join to one of authored collection
IDs `0064..0086` and never manufactures a collection path from badge tokens.

The five named overlap instances are rooted by controller rather than by a
global layout scan. The exact roots/fields are:

| Authored instance | Owner boundary | Runtime field |
| --- | --- | --- |
| peace beneficiary flags | `0x141E34B30` | returned item `+0x5D0` |
| agency upgrade repetitions | `0x141A00A50` | returned item `+0x1068` |
| diplomacy divisions summary | `0x141CE5430` / pool `0x141CE6B40` | item `+0x58`, authored extent 202x27 |
| unit-view divisions summary | `0x141CE5430` / pool `0x141CE6B40` | item `+0x58`, authored extent 192x27 |
| market CIC and subsidy | `0x142034060` | owner `+0x1340` |

The registry is consumed only at post-return overlap layout `0x142372190`;
clip propagation is `0x142371F90`. Name xrefs for
`beneficiary_negotiator_flags`, `repetitions_icons`, `divisions_summary`, and
`cic_and_subsidy` are frozen by job
`20260720T214805Z-w5final-a28-five-overlap-controller-name-f2ef9841`, artifact
SHA-256 `11c73a7d35c701023fb9bfd83a8dc58ba7cd294cc409ed98d659b08bc6c4ed80`.
The exact registry/layout hook and the mandatory controller-ledger join are
implemented in `src/a729_named_overlap_hook.rs`.

## Exhausted residual callers

The air-selection roots `0x141F4A980`, `0x141CDB770`, `0x141951BA0`,
`0x141951A10`, and `0x1415A29D0` were re-exported in the consolidated job.
They expose accepted materialized options and refresh paths, but the retained
static graph does not carry one same-transition ledger containing every
rejected candidate, its portable stable identity, exact reason, outer resolved
clip/viewport, and scroll state. A generic standard-list traversal cannot
supply those missing controller facts.

The complete MIO neighborhood was also re-exported:

- `0x141F05960` clears list owner `+0x3860`, calls `0x141F06630` and
  `0x141F05CD0`, appends the constructed header/row vectors, and finalizes by
  the list virtual `+0xC8`;
- `0x141F06630` constructs the aggregated-bonus rows at `+0x2090/+0x209C`;
- `0x141F05CD0` constructs the dynamic-equipment/history rows at
  `+0x20A8/+0x20B4` and optional singleton `+0x20C0`;
- `0x141F06360` grows the reusable row pool to the source count, binds each
  row in source order, clears the destination list, appends every row, and
  finalizes;
- `0x140DA1420` and `0x140DA1680` are the upstream valid-entry/handle-vector
  enumerators.

This exhausts the retained MIO callers and proves accepted construction order,
but does not prove a zero-gap pre-filter ledger for all three authored
smooth-list instances. The same row can be constructed from localized tooltip
content or opaque game-object references, so neither child text, pointer
identity, nor accepted order is promoted to `stable_item_id`. MIO therefore
remains an honest fixture-data gap, not an unimplemented generic list law.

## Promotion disposition

`A28_D71_SCHEDULED` and `A28_LIVE_PROMOTION_BOUND` remain `false`. Facility and
technology hooks still require authentic zero-gap owner captures. The overlap
hook additionally requires its same-transition controller source/disposition
ledger. Air and MIO require the missing controller ledgers listed above. The
offline regrader now understands equipment, facility, technology-badge, and
named-overlap authority sets, but a verifier-capable schema is not evidence by
itself.

No build, test, verifier, compiler, formatter, or generator was run while
performing this static RE and implementation pass.
