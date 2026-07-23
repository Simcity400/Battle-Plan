# A28 owner-runnable controller fixtures

> Superseded scheduling note (2026-07-22): domain 71 is production-scheduled
> for the sealed exact-a729 Air-selection and paired MIO-history passive hooks.
> The fixture route described below remains non-promoting; technology canvas
> and heterogeneous MIO aggregate bonus remain omitted and fail closed.

Four controller-bound hook surfaces are implemented. No family is promoted
merely because an installable observer exists, and fixture observations still
need a zero-gap owner capture regraded by the offline fixture gate.

## Equipment-variant standard grid

enumeration/disposition ledger from the same transition. It must then traverse
the finalized grid and enclosing scroll container before either object mutates
again.

The pinned exact20 evidence is:

- job:
  `20260720T133905Z-w5-a28-equipment-grid-owner-geometry-7d8900a4`
- manifest SHA-256:
  `96ad7c23413611dcc1524db1fac91a80c94324fc44f04b0e07430f927d8e76a4`
- producer/callback:
  `0x14177D780` / `0x1417797A0`
- append/finalize/container sync:
  `0x1402DE850` / `0x1422B2F10` / `0x1422AFA30`

The harness record schema is
`hoi4-a28-controller-observation-v1`. Its `authority` object must contain the
exact binary identity, job/manifest above, `phase: "post_return"`, nonzero
transition and QPC identities, the raw producer-record SHA-256, and:

```json
{
  "rvas": {
    "producer": "0x0177D780",
    "callback": "0x017797A0",
    "append": "0x002DE850",
    "finalize": "0x022B2F10",
    "scroll_sync": "0x022AFA30"
  }
}
```

Each `collections[]` entry uses
`kind: "equipment_variant_grid"`, set all six completeness flags to `true`,
and carry a `trace` object matching the DCT1 fields in
`src/lib.rs::CollectionTrace`. Source ordinals must be contiguous; the ordered
IDs must equal included source IDs in source order; every accepted row must be
materialized in the same order; and all identities must be pointer-free.

Run the fail-closed regrader after the owner harness writes the observation:

```powershell
python extractor/native/lanes/dynamic_collection_trace_live/verify_a28_controller_fixture.py `
  C:\path\to\a28-controller-observation.json `
  --receipt C:\path\to\a28-controller-fixture-gate.json
```

## Facility specialization smooth-list

`a729_facility_smooth_list_hook.rs` detours only
`CFacilitiesTabView::SetupDerived @ 0x141C9D840`. Post-return it gates the
concrete standard-list, `specialization_item`, and extended-scrollbar vtables,
walks the exact stored row chain, copies every LexerToken identity, invokes the
resolved row/list geometry methods, and copies fixed min/max/current/target.
Its fresh authority is job
`20260720T141140Z-w5-a28-facility-complete-hook-authority-d14d7d1d`, manifest
`fc5640f163fb89f9c52ae8db73344498f8f004adff5c048d913023807ea5c392`.
The fixture kind is `facility_specialization_smooth_list`; all source rows are
included in stored order and every row template is `specialization_item`.

## Technology specialization badge grid

`a729_technology_badge_grid_hook.rs` brackets controller `0x141BC1A00` and
nested grid-position call `0x1422CA2E0`. It copies the controller `+0x140`
source vector before mutation, gates the standard-grid and composite-row
vtables, and copies post-position row/grid geometry. It does not derive the
collection path from specialization tokens: the same-transition owner fixture
must join exactly one of the 23 authored IDs
`interface/countrytechtreeview.gui#collection-0064` through
`interface/countrytechtreeview.gui#collection-0086` before
`A729TechnologyBadgeGridSource` can be constructed. The two consumed tokens
are one logical composite DCT1 item; source ordinals beyond the two-token limit
are retained with `controller_two_badge_limit`. Missing token two is the exact
`0x4C17` sentinel. Fresh authority is job
`20260720T141305Z-w5-a28-tech-badge-owner-callgraph-comple-b890bceb`, manifest
`f4644574640b69d5ae650ed6a68564cf7efb3b09f5a05b3888b65338f9e77d5e`.
The fixture kind is `technology_specialization_badge_grid` and its one row
template is `special_project_specialization`.

## Five controller-rooted overlap instances

`a729_named_overlap_hook.rs` registers exact overlap pointers from the peace
beneficiary, agency upgrade, divisions-summary item/pool, and market purchase
controllers, then observes `COverlappingElementsBox::Layout @ 0x142372190`
post-return. The registry binds these five authored IDs:

- `interface/peaceconferencewindow.gui#collection-0003`;
- `interface/countryintelligenceagencyview.gui#collection-0008`;
- `interface/countrydiplomacyview.gui#collection-0034`;
- `interface/unitview.gui#collection-0003`;
- `interface/international_market/marketpurchasedraftwindow.gui#collection-0002`.

The hook copies exact selected forward/reverse stored order and resolved child
rectangles. It deliberately does not manufacture pre-filter identities from
child pointers. `A729NamedOverlapCompleteSource::try_join` requires an
owner-captured controller source/disposition ledger and exact slot/template
IDs before the observation can implement `ExactControllerTraceSource`. Fresh
authority is job
`20260720T141317Z-w5-a28-overlap-owner-callgraph-complete-b8a10eba`, manifest
`74d14dced950d49ce7d47e4575bec7b04c84d5739cbc17c09681928b2d72eee4`.

The shared hook sentinels are bound to job
`20260720T215038Z-w5final-a28-tech-overlap-hook-sentinels-ef563094`, artifact
SHA-256 `3eba11027150a60121e8d45e92dbb6e21e4dc85d744c27baaa531a732460e94d`.
The overlap name-to-controller census is job
`20260720T214805Z-w5final-a28-five-overlap-controller-name-f2ef9841`, artifact
SHA-256 `11c73a7d35c701023fb9bfd83a8dc58ba7cd294cc409ed98d659b08bc6c4ed80`.

## Regrading

The same offline regrader accepts one authority family per observation:

```powershell
python extractor/native/lanes/dynamic_collection_trace_live/verify_a28_controller_fixture.py `
  C:\path\to\a28-controller-observation.json `
  --receipt C:\path\to\a28-controller-fixture-gate.json
```

It does not launch the game and does not infer missing fields. A pass is evidence
input only; it does not schedule domain 71.

## Remaining promotion evidence

| Family/controller | Missing exact producer join |
| --- | --- |
| Technology specialization badge grid | Hook complete; zero-gap owner fixture not retained |
| Air-selection standard list | Rejected candidates, portable stable IDs, outer clip/viewport |
| Facility specialization smooth-list | Hook complete; zero-gap owner fixture not retained |
| Three MIO smooth-lists | Rejected accounting, stable IDs, templates, geometry, scroll |
| Five named overlap instances | Hooked owner/layout half; zero-gap controller source/disposition ledgers not retained |

Generic grid/list/overlap layout functions are retained as mechanical widget
law only. They cannot fill a controller-specific DCT1 field by analogy.
