# recorder-oracle-naval-missions

Naval mission operands and presence-window comparisons. Owns [naval_missions.rs](src/naval_missions.rs) and its child tests. The [oracle command](../recorder-oracle/README.md) composes comparisons; this crate calls its specific dependent comparisons directly.

Table completeness, exact joins, differences and named limitations use [shared oracle support](../recorder-oracle-support/README.md). Content-dependent facts resolve through the recorded content stack. Synthetic fixtures are dev-only through the support crate's `test-support` feature.

Shared rules live in [AGENTS.md](../../AGENTS.md); the [recorder guide](../recorder/README.md) defines fresh-save/load and matched-native-observation checks.

Retained convoy-raiding grid packets compare the saved active truth, signed nonzero spotting scalar and retained region, with save keys resolved through the recorded lexical dictionary and authenticated country projection. Inactive packets do not demand retained fields. Target reference liveness has no save representation and requires matched native observation.

Escort source checks validate the recorded preorder and compare its effective root to the independent raw country entry under recorded modifier metadata and simulation features. Source leaves and cached display labels are unsaved: the integration native/viewer comparison verifies those explicitly.

Patrol inputs compare every distinct actual regional-ledger member's full stored target and demanded signed progress/chance against lexical-bound save words. Stale references still compare in full. Target liveness compares against the complete current hourly task-force census, independently of saved target presence; ledger membership retains its independent native-observation limitation. A null target without a selected live convoy does not demand progress/chance. Convoy selection follows the trade-before-transport priority, so an unknown trade reference cannot demand a later transport payload. Live convoy radar/Air caches compare the positive-only serializer projection; nonpositive exact values remain a named limitation. Missing demanded acquisition fails comparison coverage. Current detection/visibility statistics, formation class, contextual known flags, modifier accumulators and character-source joins have no save representation and are named separately for the matched live check.

The convoy-target oracle joins target-country aliases to the authenticated saved convoy pool and compares its first signed-nonzero entry's full variant identity in stored order. It does not classifier-filter or discard negative quantities. Live target resolution and runtime target/stat cache operands require a matched native observation; every such limitation is named separately from the saved identity comparison.

Demanded strike-fleet sibling ranges have their own comparison. Complete recorded mission registrations identify strike fleets; their otherwise omitted siblings are checked against both the hourly roster and saved fleet/mission membership. Fuel, requested fuel, underway, positive convoy counts, headquarters count and saved leader use the same comparison as ordinary mission rows. Unsaved range, modifier and headquarters-character operands remain explicitly named for matched native observation, alongside the unsaved registration witness.
