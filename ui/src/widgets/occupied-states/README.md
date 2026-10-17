# Occupied states

**What it shows:** A table of recorded occupied states, legal owners, occupiers, resistance, compliance, garrison requirement/strength and effective occupation laws.

**Product(s) it reads:** [`occupied-states-manifest` and `occupied-states`](../../../../crates/dashboard-occupation/src/products/occupied_states.rs) share one preparation observer. The manifest supplies the branch's replay-block windows, content-derived law facts and warnings; each `occupied-states` reply is one window, so no reply grows with the campaign. Its scan/strength modules own native membership, formatting and law evaluation.

**Settings:** Countries match occupiers by default or original occupied countries; manual/smart-group choices, exact sort (resistance descending initially) and shared table density/bars/ranks; Campaign, Follow or Pin.

**Notable behaviour:**

- Active resistance/garrison state defines occupation, not controller different from owner; annexed non-core states can remain occupied. Original occupied country can differ from today's legal owner.
- Owner/name follows the shown hour. Occupier, occupied endpoint, law and garrison use the supplying occupation read; a later controller cannot inherit yesterday's values. Manual tags select native holders/aliases at that read; unknown identities leave selection partial, not empty.
- Exact fixed-point values drive sort/tooltips; resistance/compliance cell text uses native whole-percent rounding/clamping. Gaps and incomplete catalogues end coverage; a held value cannot restart after a gap without a fresh read. Known empty inventories differ from incomplete ones. Jump uses the supplying read.
- A window lists each state's values where they change, restating every value where a span the state is occupied starts (fixed5 values as shared exact delta runs, texts by index), so it decodes alone and nothing carries across a gap. Its first observation can hold a read from before it.
- The table loads the window holding the shown hour; while following, both neighbours load too, so playback and scrubbing into them never wait. A far jump keeps the last shown hour, marked as updating, until its window arrives. An hour before the first window has no occupied states recorded.
- A window preparation has not reached yet is answered exactly from its block and the blocks just before it, carrying the laws its states hold; the table shows it at once and merges those laws with the manifest's, whose laws and warnings grow as preparation publishes windows.
- Law precedence is state override, occupied-country group, validated occupier default, database fallback. Missing group/trigger operands retain **Not in this recording** or a named refusal, not a guessed default. Content owns names and sprite forms; unsupported static-icon forms are counted.
- Law cells and sorting use localized titles, preserving empty titles. Missing localization says **Occupation law name unavailable**; hover hints retain the law key and refusal.
- Garrison text is required template amount, not strength: native fixed-two-decimal truncation, dash for zero/missing, red **!** for negative. The bar uses independently established native strength/threshold, not the stored ratio; unknown strength hides it. Tooltips retain exact values and the garrison requirement alert; internal missing-input reasons stay in diagnostics.

**Files:** [OccupiedStatesWidget.tsx](OccupiedStatesWidget.tsx) renders the shared table and chooses windows; [occupiedStatesData.ts](occupiedStatesData.ts) validates, decodes windows, selects rows and formats cells, with adjacent tests. `occupiedStates.css`, `definition.ts` and `index.ts` own styling, settings and registration.

Shared framework conventions: [widgets](../README.md).

The preparation observer writes each replay-block reply through the shared window store. Its continuation retains the latest state values and supplying observation; law tokens and refusal identities survive resume. The registered products share the same scan and remain the parity reference. The table exposes "Occupied states values" in parity builds.

When the local look-back cannot prove an unprepared window, the ordinary empty
window carries pending hours. The table renders Still being prepared and keeps
its window request mounted until preparation replaces it.
