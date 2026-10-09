# Occupied states

**What it shows:** A table of recorded occupied states, legal owners, occupiers, resistance, compliance, garrison requirement/strength and effective occupation laws.

**Product(s) it reads:** [`occupied-states`](../../../../crates/dashboard/src/products/occupied_states.rs) supplies branch histories and content-derived law facts. Its scan/strength modules own native membership, formatting and law evaluation.

**Settings:** Countries match occupiers by default or original occupied countries; manual/smart-group choices, exact sort (resistance descending initially) and shared table density/bars/ranks; Campaign, Follow or Pin.

**Notable behaviour:**

- Active resistance/garrison state defines occupation, not controller different from owner; annexed non-core states can remain occupied. Original occupied country can differ from today's legal owner.
- Owner/name follows the shown hour. Occupier, occupied endpoint, law and garrison use the supplying occupation read; a later controller cannot inherit yesterday's values. Manual tags select native holders/aliases at that read; unknown identities leave selection partial, not empty.
- Exact fixed-point values drive sort/tooltips; resistance/compliance cell text uses native whole-percent rounding/clamping. Gaps and incomplete catalogues end coverage; a held value cannot restart after a gap without a fresh read. Known empty inventories differ from incomplete ones. Jump uses the supplying read.
- Law precedence is state override, occupied-country group, validated occupier default, database fallback. Missing group/trigger operands retain **Not in this recording** or a named refusal, not a guessed default. Content owns names and sprite forms; unsupported static-icon forms are counted.
- Garrison text is required template amount, not strength: native fixed-two-decimal truncation, dash for zero/missing, red **!** for negative. The bar uses independently established native strength/threshold, not the stored ratio; unknown strength hides it. Exact raws and missing reasons remain in tooltips.

**Files:** [OccupiedStatesWidget.tsx](OccupiedStatesWidget.tsx) renders the shared table; [occupiedStatesData.ts](occupiedStatesData.ts) validates, selects rows and formats cells, with adjacent tests. `occupiedStates.css`, `definition.ts` and `index.ts` own styling, settings and registration.

Shared framework conventions: [widgets](../README.md).
