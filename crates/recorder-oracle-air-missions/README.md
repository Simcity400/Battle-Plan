# recorder-oracle-air-missions

Air missions and wing/raid operand comparisons. Owns [air_missions.rs](src/air_missions.rs) and its child tests. The [oracle command](../recorder-oracle/README.md) composes comparisons; this crate calls its specific dependent comparisons directly.

Table completeness, exact joins, differences and named limitations use [shared oracle support](../recorder-oracle-support/README.md). Content-dependent facts resolve through the recorded content stack. Synthetic fixtures are dev-only through the support crate's `test-support` feature.

Shared rules live in [AGENTS.md](../../AGENTS.md); the [recorder guide](../recorder/README.md) defines fresh-save/load and matched-native-observation checks.

The retained trace-count predicate used by regional interception alerts has no saved counterpart. Its matched native alert visibility must agree with the recorded predicate and the existing regional interception aggregate.

Ground Crews is compared exactly with the country's saved strategic-region flag, including omitted false blocks and the sparse recording's omitted false rows. Demand follows the registered wing's pool owner independently of its effective volunteer country or sortie-base eligibility; undemanded rows are refused.

AirRegionalMissions checks regional ordinal contiguity independently of all-active labels. An owning country has all labels known or all unknown; known labels are nonzero and unique, without requiring contiguous values. The text save serializes neither vector order, so their native ordering needs the integration live comparison.

AirTransferInputs compares admitted raw progress and a saved destination state exactly. Incoming ordinals must be complete, unique and contiguous per destination; an uncertified or unreadable required field fails the comparison. The unsaved incoming order and retained total distance need the integration live comparison.

Current executing mission masks and counts are unsaved. The oracle requires a mask for each registered mission and a count exactly for nonzero masks; the integration native comparison checks their values.

AirWings numeric fuel columns retain independent raw cached fuel, current execution/count, potential context and reached training-capacity/override liveness. The oracle checks demanded leaves, current/count joins with registered missions and living-ace fuel-entry availability. These unsaved values require the matched native comparison; no save difference is waived.

Country Air modifier packets must be present for each counter row. Their entries are unsaved runtime accumulator operands; the matched native comparison checks raw presence and signed values independently from effective metadata and feature eligibility.

The execution-demand check requires each nonzero-count mission's raw normalization packet and its independent leaves. AirSortieModifiers requires exactly one certified detection-override availability witness; raw override entries may be zero or signed and are unsaved. The integration native comparison checks the vector and demanded raw normalization values.

The same execution-demand check requires both independent cached aircraft-detection statistics for positive one-bit executions with nonzero aircraft and rejects values outside that demand. The unsaved signed values require a matched native comparison; the Navy patrol hover derives their contribution from the recorded regional registration order and content spotting factors.

The mine oracle compares the complete certified NavalRegionalMines country vectors, including witness/count validity and positive-only saved entries. Empty vector omission differs from a present empty block. Nonpositive raw amounts and exact native vector length need the matched native observation. Air acquisition has no separate targeted mine read.

Country Air combat census availability and living-ace own/type combat demand are checked independently of saved roster facts. Country/ace accumulator raw entries and type predicates are unsaved: the integration run compares their exact signed values, absent keys and direct-positive type gates with a matched native read; no saved difference is waived.

The Air patrol disruption check requires raw disruption exactly under applicable current execution and every independent combat leaf only after nonzero disruption, bypass and positive-count gates. It rejects missing demanded leaves and extra unreachable leaves. These unsaved operands require the integration matched native comparison, including signed words and independent failures.

Held-clock Air patrol disruption requires patrol_night_applied_raw; a missing marker is never an Updated certificate. Updated clocks derive without marker acquisition. The native unsaved comparison must distinguish startup-100000 from an applied factor, including a mod whose completed local clock is0, and check deduplicated region sampling and independent marker faults.

Air mission comparisons receive the verified recorded-content mission spotting factors. Zero/out-of-array factors require all new patrol leaves absent. A nonzero factor requires the reached leaves, so an unreadable native factor cannot masquerade as an admitted zero. Existing Air normalization remains independently required by its original execution/count law.
