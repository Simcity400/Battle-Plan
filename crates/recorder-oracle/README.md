# recorder-oracle

The relation comparison groups include FactionAccounts, FactionVariables, FactionResearch, FactionMinimumUpgrades, FactionDoctrine, FactionProjection FactionResources and FactionAdvisorModifiers independently. A missing or failed group remains a named oracle failure; unsaved held caches have separate screen checks.

Composes exact-hour text-save comparisons in their native comparison order. [oracle.rs](src/oracle.rs) selects the recording branch/hour, authenticates identities/content, prepares shared operands and calls each comparison family. It keeps whole-oracle integration tests and fresh-load checks.

Demanded patrol mission save fields and retained radar/Air caches bind through the recording's lexical tokens and authenticated physical-country projection before comparisons run. Missing or ambiguous demanded token bindings fail preparation.

[recorder-oracle-support](../recorder-oracle-support/README.md) owns exact joins, country-emission projection, Report coverage/verdicts and feature-gated shared synthetic fixtures. Differences and incomplete comparisons fail the command; saved inputs and unsaved limits remain distinct. Only verified recorded content stacks compare, with mods resolved through installed-content.

## Comparison families

| Crate | Responsibility |
|---|---|
| [recorder-oracle-air](../recorder-oracle-air/README.md) | Regional air-combat records and air-force save comparisons. |
| [recorder-oracle-air-ledger](../recorder-oracle-air-ledger/README.md) | Country air-loss and bombing comparisons, including continuity limits. |
| [recorder-oracle-air-missions](../recorder-oracle-air-missions/README.md) | Air missions and wing/raid operand comparisons. |
| [recorder-oracle-characters](../recorder-oracle-characters/README.md) | Character rosters, leader roles, stored names and portrait/codename inputs. |
| [recorder-oracle-combat-details](../recorder-oracle-combat-details/README.md) | Retained combat detail rows, commander operands and nested value trees. |
| [recorder-oracle-combat-log](../recorder-oracle-combat-log/README.md) | Combat-log records and their saved source joins. |
| [recorder-oracle-construction](../recorder-oracle-construction/README.md) | Construction quantities, progress, output, target levels and occupation inputs. |
| [recorder-oracle-core](../recorder-oracle-core/README.md) | Country, province and state values, flags, player identity and boundary counters. |
| [recorder-oracle-counters](../recorder-oracle-counters/README.md) | Unit and headquarters counter operands and leader-picture inputs. |
| [recorder-oracle-country-rosters](../recorder-oracle-country-rosters/README.md) | Ordered country roster comparisons. |
| [recorder-oracle-economy](../recorder-oracle-economy/README.md) | Country economy, fuel, stockpile, resource and trade comparisons. |
| [recorder-oracle-equipment-destruction](../recorder-oracle-equipment-destruction/README.md) | Recorded equipment-destruction comparisons and continuity coverage. |
| [recorder-oracle-event-feed](../recorder-oracle-event-feed/README.md) | Recorded event-feed consistency and explicit partial/unavailable interval limits. |
| [recorder-oracle-faction-theatres](../recorder-oracle-faction-theatres/README.md) | Faction and theatre membership comparisons. |
| [recorder-oracle-factories](../recorder-oracle-factories/README.md) | Factory totals using their captured ownership/control inputs. |
| [recorder-oracle-fog](../recorder-oracle-fog/README.md) | Fog operands and visibility consistency checks. |
| [recorder-oracle-fuel-demand](../recorder-oracle-fuel-demand/README.md) | Country fuel-demand and consumption comparisons. |
| [recorder-oracle-game-score](../recorder-oracle-game-score/README.md) | Recorded game-score inputs and multiplier comparisons. |
| [recorder-oracle-governance](../recorder-oracle-governance/README.md) | Country governance, focus, research and power-balance comparisons. |
| [recorder-oracle-intel](../recorder-oracle-intel/README.md) | Intelligence agencies, operatives and operation save comparisons. |
| [recorder-oracle-land](../recorder-oracle-land/README.md) | Division land-manager records and land-history save comparisons. |
| [recorder-oracle-map-inputs](../recorder-oracle-map-inputs/README.md) | Country and map presentation operand comparisons. |
| [recorder-oracle-military](../recorder-oracle-military/README.md) | Division status/movement, army orders, force rosters, combat membership and the battle-sound list's self-check. |
| [recorder-oracle-modifier-breakdowns](../recorder-oracle-modifier-breakdowns/README.md) | Power-balance modifier description-tree comparisons. |
| [recorder-oracle-modifiers](../recorder-oracle-modifiers/README.md) | Stored country modifier comparisons. |
| [recorder-oracle-naval-history](../recorder-oracle-naval-history/README.md) | Naval history, bases, railways and theatre save comparisons. |
| [recorder-oracle-naval-missions](../recorder-oracle-naval-missions/README.md) | Naval mission operands and presence-window comparisons. |
| [recorder-oracle-naval-result-sources](../recorder-oracle-naval-result-sources/README.md) | Naval-result source checks and event-loss coverage limits. |
| [recorder-oracle-pass-inputs](../recorder-oracle-pass-inputs/README.md) | Naval, country and orders-group counter operands and leader-picture inputs. |
| [recorder-oracle-peace](../recorder-oracle-peace/README.md) | Peace-conference save comparisons and recorded finalizer completeness. |
| [recorder-oracle-plans](../recorder-oracle-plans/README.md) | Battle-plan/order identities and execution operands. |
| [recorder-oracle-politics](../recorder-oracle-politics/README.md) | Country politics, character and naming comparisons. |
| [recorder-oracle-power-ledger](../recorder-oracle-power-ledger/README.md) | Recorded country-power ledger comparisons. |
| [recorder-oracle-production](../recorder-oracle-production/README.md) | Production lines, variants, industrial organisations and output accounting. |
| [recorder-oracle-projects](../recorder-oracle-projects/README.md) | Special-project and raid identities, equipment, state and path comparisons. |
| [recorder-oracle-railway-guns](../recorder-oracle-railway-guns/README.md) | Railway-gun save and movement consistency checks. |
| [recorder-oracle-relations](../recorder-oracle-relations/README.md) | Country relations, opinions, faction membership and directed visibility joins. |
| [recorder-oracle-rocket-output](../recorder-oracle-rocket-output/README.md) | Recorded rocket-output continuity and delivery accounting. |
| [recorder-oracle-saved-event-targets](../recorder-oracle-saved-event-targets/README.md) | Saved event-target identity comparisons. |
| [recorder-oracle-selected-routes](../recorder-oracle-selected-routes/README.md) | Selected-route labels, representatives, nodes and contact invariants. |
| [recorder-oracle-session](../recorder-oracle-session/README.md) | Save-boundary and session identity comparisons. |
| [recorder-oracle-stability](../recorder-oracle-stability/README.md) | Country stability operand comparisons. |
| [recorder-oracle-supply-trains](../recorder-oracle-supply-trains/README.md) | Supply-train comparisons with verified map membership inputs. |
| [recorder-oracle-unit-combat-references](../recorder-oracle-unit-combat-references/README.md) | Unit-to-combat identity and membership comparisons. |
| [recorder-oracle-visibility](../recorder-oracle-visibility/README.md) | Country access, visibility and presentation operand comparisons. |
| [recorder-oracle-wars](../recorder-oracle-wars/README.md) | War participants and directed relation comparisons. |
| [recorder-oracle-weather](../recorder-oracle-weather/README.md) | Recorded weather and temperature save comparisons. |

Shared rules live in [AGENTS.md](../../AGENTS.md). The [recorder guide](../recorder/README.md#fresh-save-oracle) defines the required fresh-save/load, native-observation and smoke workflow.

The war-goal inventory group compares CountryWargoals, WargoalTargets and WargoalJustifications through recorder-oracle-wars.
The naval management family compares the saved standing repair/refit/reinforcement controls and ordered native lists independently, including ship-to-work-order links and signed fixed5 refit progress.
The event-feed comparison includes retained pending, fired-once and country delayed-event state from the projected save, in addition to occurrence accounting.

Air mission comparisons receive the mission spotting-factor vector from the same verified recorded content defines as other content-dependent checks, preserving mod array length and zero/nonzero values without another captured fact.

Validation-refused and failed-hook-family tables remain incomplete comparisons, including tables whose normal event publication is deferred. Zero value differences cannot pass a run with these refusals; the report retains their full reasons.
