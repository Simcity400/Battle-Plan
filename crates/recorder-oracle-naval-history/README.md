# recorder-oracle-naval-history

Naval history, bases, railways and theatre save comparisons. Owns [naval_history.rs](src/naval_history.rs) and its child tests. The [oracle command](../recorder-oracle/README.md) composes comparisons; this crate calls its specific dependent comparisons directly.

Table completeness, exact joins, differences and named limitations use [shared oracle support](../recorder-oracle-support/README.md). Content-dependent facts resolve through the recorded content stack. Synthetic fixtures are dev-only through the support crate's `test-support` feature.

Shared rules live in [AGENTS.md](../../AGENTS.md); the [recorder guide](../recorder/README.md) defines fresh-save/load and matched-native-observation checks.

The sunk-ship entry comparison is shared with embedded division career records and keeps the same text/reference omission boundaries.
[naval_management.rs](src/naval_management.rs) compares repair budgets/policies, home-base automation, independent designs, rejoin references, ordered children/composition/queues, ship refit links and refit progress. Runtime composition definition preferences and per-base dockyard assignments have no serialized counterpart and remain named limitations.

Country repair budgets, fleets, task-force policy/composition/children, ship links, base repair
queues and refit progress compare through the authenticated canonical emission census. Flattened
save objects retain their enclosing country's tag. Reserved emissions retain the oracle's named
uncaptured-operands refusal; without admission every saved object remains in the strict comparison.

Refit source rows are sparse: an ordinary ship's empty link has no row, while a saved refit
requires a matching resolved link. The global base census also supplies reserved queues, so
recorded queues exclude provinces serialized only by out-of-scope owners. Canonical/shared
and unknown provinces remain in the strict comparison.

Rejoin references are recorded raw. Their optional save projection uses the same hour's complete
TaskForces census to omit destinations that no longer resolve, including after a repair detachment
has transferred to another fleet and its source reference carries unchanged.

Ship refit links also carry stored identities independently of work-order lifetime. The shared
NavalRefitShipRow resolution rule uses the complete hourly NavalRefitProgress production-line
rows for the serializer's optional link. Daily ProductionLines metadata cannot certify removal;
an unavailable hourly line table refuses the comparison rather than implying absence.

`naval_accidents` compares standing accident and mine report occurrence order, region, stored full equipment/ship identities, sunk predicate, repeated country tags and expiry date. Mine reports serialize victim and mine owner under two country occurrences; neither is discarded. Accumulated raw damage has no save counterpart and requires the matched native-observation check.
