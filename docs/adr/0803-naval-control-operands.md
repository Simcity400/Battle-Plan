# Cache naval control operands for relevant viewers

The recorder evaluates the game's naval control rule when its input changes so both the viewer and dashboard can answer from the same held operands. Retain the raw share values and apply the recorded region minimum at reply time. A viewer related to any owner can need a row, including faction and subject members, wartime opponents, countries listing a relation, blockaded countries and viewers whose rosters are unknown. One every-viewer row can represent otherwise unaffected viewers. Drop all-zero rows while keeping unavailable results identifiable. An unresolved owner of a nonzero current can withhold the region result for every viewer.

When `NavalSupremacy` changes, recompute the affected regions. A changed or incomplete identity or diplomacy input can affect every current region. Reserved-country inputs are consulted when the reserved slot is involved, with their own status and named refusal. A supremacy read expires after its day even if no subsequent row changes. A resume restores the held viewer index and recomputes the rule from the recorded tables.

[Naval control writer](../../crates/recorder-derive-naval/src/naval_control.rs)
