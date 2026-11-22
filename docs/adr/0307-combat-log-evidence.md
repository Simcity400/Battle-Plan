# Combat log evidence belongs to its recorded hour

The dashboard reads the combat log's retained memberships, monthly counts, template statistics and battles as state at the requested hour. It reads finalized combats and battle modifier insertions through their declared logs. A finalized row with an unrecorded cell cannot supply a complete finalized combat. Each part of the reply carries its own refusal when the relevant input is incomplete.

Battle slots can be reused. Consider only modifier insertions at or after the battle record's own hour, taking the latest insertion for each `(battle, side, ordinal)` and ignoring battles the reply never references. The fixture with two battles in slot 4 exercises this boundary.

Keep the observer operands in stored order. Resolve participant names under the identity and membership rules in effect at the hour. A raw reference the recording cannot resolve is reported by id. An unknown participant or an empty side cannot prove that a side should be filtered out. Gather distinct design handles before reading their labels at the hour, and use the template views to name templates. Format fixed5 quantities exactly as decimals with trailing zeros removed.

[Combat log product](../../crates/dashboard-land/src/products/combat_log.rs) · [Combat log tests](../../crates/dashboard-land/src/products/combat_log/tests.rs)
