# recorder-oracle-military

Division status/movement, army orders, force rosters, combat membership and the battle-sound list's self-check. Owns [military.rs](src/military.rs) and its child tests. The [oracle command](../recorder-oracle/README.md) composes comparisons; this crate calls its specific dependent comparisons directly.

Table completeness, exact joins, differences and named limitations use [shared oracle support](../recorder-oracle-support/README.md). Content-dependent facts resolve through the recorded content stack. Synthetic fixtures are dev-only through the support crate's `test-support` feature.

Shared rules live in [AGENTS.md](../../AGENTS.md); the [recorder guide](../recorder/README.md) defines fresh-save/load and matched-native-observation checks.

Military comparisons check acquired ship and air-wing crews and required ship crews against their saved manpower words, independently of equipment counts.

Wing exile attribution compares government_in_exile_tag under the signed-positive writer gate; unavailable capture is a difference even when the saved tag is absent. Template manpower exile eligibility compares with the saved origin_type predicate independently of model availability.
Air-wing comparison checks every replacement-setting byte against reinforcement_setting. Missing save keys and unavailable certificate reads produce differences.

Division comparisons include stored experience independently of manpower and exercising status; a refused acquisition is reported explicitly.

Division history comparisons cover the summary, native list ordinals/repetitions, names, dates, countries, province order, medal tokens, medal-count and inheritance booleans, multipliers, orders and embedded sunk-ship operands. Saved flags compare the native boolean emission while the recording retains each source byte. Refused roots are differences, never empty histories.

The division comparison checks pending template conversions, including saved absence and refused acquisition.

The division oracle compares held-officer XP and the retained history-panel counter, including the positive-value omission predicates.

DivisionPanelStats has a separate membership/availability comparison for all 22 cache columns. Those runtime cache numbers are unsaved and require matched native panel observations and sampled rebuild verification; the oracle names that limitation explicitly.
The template comparison checks stored recruitment, restriction and equipment settings, variant exclusions and model choices, plus every occupied grid coordinate. Moving a battalion while keeping composition counts equal is a difference. Grouped composition remains an independent comparison. Grid dimensions and empty cells have no independent saved representation. The oracle compares the writer's signed equipment-niche value and its conditional obsolete-date emission without replacing captured operands.

CountryTemplateState compares each saved country's selected military-staff template and country-wide template lock. The save omits an unresolved selected reference, while the replay retains its complete raw handle. Missing country configuration and deployment wrappers are differences.


CountryUnitBonuses compares each country's entries, in stored order, with its saved `unit_modifiers`: source type, number, the `id` the writer omits for the `none` token and the label it omits when empty; then each target by position and name, its omitted-when-zero combat width, its nonzero stats by name and each battalion grant's category, flags and nonzero stats in stored order. A country without the saved key must have no rows. The save marks no target arm, and the unrecorded requirements, adjusters, priorities and sprite are named limitations. Groups not read at the oracle hour are carried.

Garrison selections compare country defaults, occupied-country groups and state overrides independently, preserving inherited selections and explicit state zero. The next headquarters deployment order follows the native positive-only save emission.

Historical naming groups compare their native empty-key omission. Garrison priority compares independently of the selected template and scope lists. Missing certificates or refused new template operands fail saved-operand comparison.
Conditional exact movement organisation compares the current word exactly with the
saved organisation. Its effective maximum has no saved counterpart; its display
projection is checked against DivisionStatus's maximum-organisation field.

Movement state rows validate controller relation bits separately from the
conditional attack-entry refusal bit. DivisionStatus recognises the local air-sign
refusal flag; undefined flag bits remain differences. Runtime movement entries
without a saved counterpart remain named limitations, not saved-word comparisons.

Training wings compare their certified stop-training flag against the mission save key. Missing demanded observations fail the comparison; attachment coverage is unsaved and needs the matched live check.

Air-grid attachment coverage is an unsaved live-comparison limitation. Saved training-stop and raid-reference resolution remain exact comparisons.

Pool-backed wings compare their certified raid non-resolution against the save's omitted link without duplicating a raw-null packet. Live assignments compare the complete stored handle and liveness; missing demanded certification fails independently of wing type or selected mission. Nonresolving stale raw bits remain an unsaved native-witness limitation.

Template configuration census comparison follows the authenticated canonical country-emission projection. Reserved aliases retain their named uncaptured-operands refusal; absent canonical configuration remains a difference. Without admitted projection, the comparison keeps the full saved census.

An encodable template-settings acquisition refusal still fails saved-settings comparison by name. It is never treated as default settings or skipped as a successful comparison.

Ship naming compares every saved operand separately at the supplying ship hour:
signed kind, nonzero-only order, clear-only ordered flag, stored-length-gated
first-NUL override, nonzero-only programmatic truth, and nullable full naming
variant. Missing wrappers and refused packets are differences, not defaults.
Comparison counts name each operand. Raw non-Boolean bits and bytes after NUL
have no independent save proof. `Ships.name` has an explicit unsaved limitation;
matched native cache evidence at the same full identity/hour, fresh-save/load and
smoke remain integration gates. Rebuilding from operands is not a cache oracle.
