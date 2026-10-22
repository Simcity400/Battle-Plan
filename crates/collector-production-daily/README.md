# collector-production-daily

Daily production slot capture: IndustrialOrganisations, CountryProductionModifiers and ProductionLineModifiers, the latter read in up to four consecutive census slot ranges. The host owns dispatch, the range split, loans and table assembly.

MIO runs in an independent worker job submitted before the modifier ranges. Range zero retains the ProductionDaily job; all captures keep slot 8 and the existing late join, ordered modifier assembly and refusal rules. The MIO job takes its existing capture buffers by value, leaving an allocation-free vacant placeholder, and returns them with the daily pack.

Read the [shared rules](../../AGENTS.md) and the [collector guide](../collector/README.md).

Run `cargo test -p collector-production-daily` and `cargo clippy -p collector-production-daily --all-targets -- -D warnings` for its synthetic checks.

The daily MIO census shares identity, history, mutable organisation state and country policies in one authenticated walk. Quiet trait/queue/allowed-policy rosters, variables and flags carry under writer certificates. Every scalar organisation operand, including costs, factors, selected policy and cooldown, is read daily beside funds, level, points, cursor and task referents; direct effect assignments need no scalar certificate. Content owns trait trees and source definitions. Nested memory accounting is cached in the daily worker.

Policy costs and cooldown days are read in every existing daily production-country visit, using the same authenticated policy objects. The same visit reads the authenticated country's effective policy-cost modifier with one keyed accumulator lookup; a failed read leaves it unknown without withholding the roster. Stored localised names, country context and cached selection score carry under the score-refresh callback and existing country-input reload observer; only dirty or sampled caches are copied.
