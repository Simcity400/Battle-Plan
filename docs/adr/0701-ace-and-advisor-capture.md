# 0701. Retain roster rows independently of presentation inputs

## Ace rosters

Walk each canonical country's ace roster in source order. Preserve the raw owner link, including aliases, the complete native reference pairs and the aircraft mask. The roster records the raw kill type, including unknown values, and validates handled and alive independently. Read killer identifiers only for a dead ace, bracketing them with the alive observation. Preserve raw reference sentinels. Nothing resolves a country or takes an assigned-wing reference as a pointer in this row.

Keep exact name, surname and callsign bytes, even when empty or invalid UTF-8. Account for full strings during sizing and omit whole rows on capacity shortage. The table encoder sorts and checks replay keys off the callback. Optional presentation failure does not erase a valid roster row. A selected aircraft definition supplies its literal name, and a readable null definition is distinct from a failed name read.

Portrait selection uses the canonical country slot and the game's recorded gender and signed pool index. A retained country pool takes precedence over a culture pool and never refreshes from it while nonempty. The signed index uses division remainder. Global fallback is consulted only after the culture choice is exhausted, so its failure cannot invalidate a usable culture choice. A nonempty retained country pool must work even when the culture bytes are unavailable.

## Advisor roles

The character's role map supplies advisor occurrences, including idea and slot strings and ordered traits. Appointment comes from the separate appointed list. Validate the role's parent character. Preserve map order, including unsigned hashes and locale-sensitive comparison on a hash collision. Raw object addresses live only for the stopped callback. The role-map reader validates the count before consulting the sentinel, even when a batched caller supplied the sentinel observation. A cache hit and a miss both use those captured controls.

Observe cached numeric modifiers and occupied tooltip keys independently of role definitions. Role rebuilds combine cleared trait-derived entries with independent blocks. Only the character's role-writer certificate permits carry-forward between hours. Sizing uses observed headers and never claims payload without reading it. A slot with its occupied byte clear contributes no tooltip key even if it retains stale text. Append tooltip keys only after the complete role and numeric-cache prefix fits, keeping accepted rows on optional extension failure.

[Ace roster](../../crates/engine-native-air/src/cace.rs) · [Advisor role and cache readers](../../crates/engine-native/src/cadvisor.rs)
