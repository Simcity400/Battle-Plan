# Ordinary-map raid hover text

Only the three icons produced by ordinary map mode are covered. The unit icon
belongs to raid planning, whose client selection is outside the recording.
Hover uses the map counter popup and hit rectangles in reverse submission order.
A coalescing worker builds text and decodes inline assets for one requested hour,
perspective and content object; the same icon keeps its last text while the next
hour's builds. No simulation or localisation work runs per frame.

## Native evidence

The addresses below refer to the supported executable in `engine::build`.
`P` means the call and branch are present in the indexed executable; live visual
agreement is a separate check. Localisation keys in this table are native call
arguments, not assumptions about a particular content installation.

| Tooltip | Ordered lines and conditions (P) |
|---|---|
| Own active, `141E72380` → `141612820` | Prompt `tooltip_raid_view_details_prompt` in the secondary tooltip string; main string `raid_inst_item_desc(TYPE, TARGET)`, blank line, phase. `140FD5730` wraps a positive-total phase in `raid_inst_progress_desc(PHASE, PROGRESS)`; otherwise only the phase key. Phase 1 always appends preparation diagnostics. Phase 3 appends `tooltip_launch_button_disabled` and execution diagnostics only when diagnostic `140FD6550` returns false. `1414545D0` requires a resolving unit raid back-reference before adding `tooltip_launch_button_right_click(NAME)`, whose NAME is the complete `raid_inst_unit_desc(NAME)` from `1414536D0`. Army resolves before wing. |
| Own or incoming ended, `141613BE0` | `tooltip_raid_result(RAID_DESC, OUTCOME_TYPE)`; details prompt then `tooltip_raid_close_report_prompt`. Description uses the plain target location, not the active custom target description. Phase/outcome key switches use the executable's fallback arm for out-of-range values. |
| Incoming, `141E76D40` → `141617380` | `tooltip_raid_warning_header`; `tooltip_raid_warning_description(COUNTRY, RAID)`; blank line; `tooltip_raid_warning_success_chance_header(VALUE)`; optional blank line, `tooltip_raid_warning_counterplay_header`, then each eligible counterplay line in modifier order. VALUE is one minus the clamped success sum. Ended raids take the common result path. |
| Target/offer, `141E732A0` → `141615C50` → `1416151F0` → `141612FC0` | `TOOLTIP_POTENTIAL_TARGET_ENTRY` or `_NOT_ALLOWED` with TARGET_NAME and CONTROLLER_FLAG. A facility then shows `TOOLTIP_TARGET_FACILITY_PROJECT_UNKNOWN_DESC(CURRENT, REQUIRED)`, `_ONGOING_PROJECT_DESC(PROJECT_NAME)`, or `_NO_ONGOING_PROJECT_DESC`. Unknown facility suppresses the detection line; otherwise a detection-requiring building adds `TOOLTIP_POTENTIAL_TARGET_ENTRY_INTEL(CURRENT)`. More than one type adds `TOOLTIP_POTENTIAL_TARGET_MULTIPLE_RAIDS`. Each type gets `_RAID_HEADER_ALLOWED` or `_RAID_HEADER_NOT_ALLOWED(RAID)`, optional `raid_type_rescue_captured_general_captive_line(NAME)`, and its description in `TOOLTIP_POTENTIAL_TARGET_RAID_DESC(TEXT)` (argument literal at `14292F680`) only outside compact mode. Denied entries append preparation diagnostics; allowed noncompact entries append `TOOLTIP_POTENTIAL_TARGET_LAUNCH_CONDITION` and the failed full launch chain. Positive range adds `RAID_RANGE_FACTOR_TOOLTIP(RANGE_FACTOR)`; once-only types add `tooltip_raid_only_launch_once`. One allowed target with one type gets `RAID_SHIFT_CLICK_QUICK_START`. Compact means anything other than exactly one target and one type. |

`rules.rs` owns separators and outer line conditions. Native
`EnsureTrailingNewlines@1424B6660` adds only missing trailing newlines, never a
leading separator. Arguments stay typed: strings, signed integers, or fixed5;
the content's formatting directive chooses precision, percentage and colour.
`142232AB0`/`1424B89F0` truncate decimal digits. Viewer language and recorded mount
order determine every expansion, country name and asset.

## Derived inputs and diagnostic order

`GetPhaseElapsedHours@140FD53C0` and `GetPhaseProgress@140FD5090` read preparation
hours for phase 2 and travelled distance plus the raid-unit speed for phase 4.
`BASE_DAYS_TO_PREPARE` is used only when the raid type supplies no positive override.
Other phases do not invent a progress fraction.

`140FD0EA0` supplies CURRENT, not the building's detected flag: selected national
intel lane, target-local network strength, controlled-neighbour term, regional
air-score ratio, decryption term and enemy counterintelligence. The lane comes
from the building definition's `detecting_intel_type`. REQUIRED comes from
`NRaids.TARGET_DETECTION_INTEL_TRESHOLD`. `1411DED90`/`1411E3C60` use the strategic
operative roster, unique operative identities, descending contributions and
successive stacking factors. Cryptology uses `1413E3350`'s rounded strength.
Facility selection uses the first country program at the target province and
its active project definition; the definition key is localised in the viewer.
The captive name uses the leader's original +20 input: `140C16FB0` localises it
into +40, which `141612FC0` reads. A character script ID is not its name.

`14157BC30`/`14157C670` compute standard success factors from raw army, wing,
target, air, weather and ace operands. `14157A140` applies each authored mapping
in order, including its separate fixed5 divisions, then clamps the sum.
`14161AC80` considers defender slot 3 and skip slot 6, choosing the smaller
endpoint for counterplay. `141619580` builds each counterplay line: the
modifier's `_source` key takes `VAL` and `MAX`, while
`tooltip_raid_modifier_effect_improvement` (and `_no_source`) take `VALUE`,
`MAX` and `SOURCE`. Resistance's skip uses the resolved target province's
state cores (`141580300`); interception preserves repeated native pool/wing
occurrences (`140C3FDF0`) of the wings in the target region, the only ones it
counts. The regional air scores are rewritten only in a region's phase hour, so
the recorder reads those regions each hour and carries the rest. Air defence sums signed healthy levels using the
building definition's `air_defence` (`1409CB2C0`).

Offer denial (`141615D40`) runs command power (`140A8C5A0`,
`raid_insufficient_command_power(COST)`), essential stockpile requirements,
preparation conditions, then `tooltip_raid_no_valid_unit` with requirement text,
or `tooltip_raid_cannot_reach`. Instance diagnostics (`140FD66E0`/`140FD6550`)
start with `raid_unit_not_yet_at_source(UNIT, SOURCE)`, then essential and
additional pools (`140FD4C00`), then the condition chain. Additional requirements
show their own header only when essential requirements passed. The diagnostic
return value is the final trigger-chain result, even after earlier failures.

`14157A760` lists nuclear equipment first, then equipment in the definition
pointer-map order from `14157E530`. Missing amounts are signed integer differences
in `tooltip_raid_missing_equipment_entry(EQUIPMENT, AMOUNT)`, under
`tooltip_raid_missing_equipment_header` or `tooltip_raid_waiting_for_equipment`.
Equipment uses current-viewer-country localisation (`140BBA630`/`140BBA780`).

The condition chain is visible → category unlock (no text) → once-only used-up
(`tooltip_raid_already_launched`) → show_target → first matching cooldown
(`tooltip_raid_on_cooldown(DAYS)`) → available → optional launchable → optional
launchable_from with source-country ROOT. Addresses: `140A8DDF0`, `140A8C500`,
`140A8C360`, `140A8D3F0`, `1414D7630`, `140A8D6D0`, `140A8D8D0`. Failed script
blocks have `raid_tooltip_trigger_title(TITLE)` before their mode-1 trigger text
(`140A8F290`, `14053FCC0`).

Requirement text (`141618D30`, `141579E40`) lists equipment before battalions.
`1415790B0` uses `raid_unit_requirements_equipment_header(EQUIPMENT, AMOUNT)`,
module entries `raid_unit_requirements_module(NAME)`, country-localised module
names, localised `OR` between alternatives, groups
`raid_unit_requirements_module_group(LIST)`, and
`raid_unit_requirements_module_header`. `141578C80` reads the global subunit definition discriminator (not a country-local clone) and lists active battalions with
nonempty limits using `raid_unit_requirements_battalion(NAME, LIMIT)`.
`141580360` selects the empty/min/max/invalid/exact/min_max limit branch.
`1409FC1E0`/`1409FEBA0` expand equipment groups (`installed_content::equipment::Groups`,
found by token without case) in authored order with terminal deduplication
without case and `", "` separators when the group's key (its description, else
its name) has no localisation; a module is shown by its own name, found by token.
The anonymous requirement wrapper tests `none`, not an empty key:
`14157E990` keeps the default token `0x165` from `1424A9460`, whose native
spelling is `none` (`140082CB0`). A localised wrapper suppresses member
expansion (`1409FC1E0`); without that key the member list expands normally.

`141580CB0` describes a source as `raid_source_desc(WHAT, LOCATION)`.
LOCATION is the state's display name or the ship name. `141581940` selects WHAT
from the resolved air base kind, rocket site, naval base, or building template.
`1409B7290` resolves division/ship name groups. `1409B57C0` chooses Roman `%s`
before decimal `%d`. Literal authored name-group strings and genuine custom
overrides are not client-localised strings. Source air-base kind is captured
from the exact country access-table selection (`1409C4160`/`140C3B7F0`) or
ship +730, including the null case.

## Refusals and shared text

Every refused occurrence is stored as a line name plus reason and counted;
independent lines remain visible. The diagnostic popup exposes these counts. A
raid whose operands were unreadable when recorded has a single `UNREADABLE`
row; its operand and naming lines are refused, and other raids keep theirs.
Irrecoverable cases include generated wing names with no retained original key
or user/generated provenance (`140F51FB0`), programmatic name overrides with lost localisation provenance, random fallback names without the
selected choice (`1409B4CE0`). These are not replaced with names in the
recording client's language.

A state's display name (`1409CC380`) is its custom name (`CState+0x38`, stored
localised by `set_state_name` `140310B50`), else its owner name (`CState+0xE8`,
stored localised by `1409D3770` from `SetOwner` `1409D1710`), else its
template's localised name. The first two are recorded (`States.custom_name`,
`States.owner_name`) without their keys, so
`dashboard::scripted_text::stored_text` draws them only when the viewer's
language is the recording's and refuses them by name otherwise.

Content forms outside the narrow adapters are explicitly refused: unknown
success factors or custom expressions, unsupported naming fields, equipment
groups that contain themselves, native
trigger text forms without a supported adapter, scripted-text brackets the
recorded hour cannot answer or that name a province or unit-leader scope,
unsupported numeric directives, invalid text cycles, and missing inline assets.
No refusal changes the ordinary-map icon eligibility.

Text goes through the shared code: the game's localiser with typed arguments
(`installed_content::localisation`), `crate::combat::text` for `ensure_newlines`
(`EnsureTrailingNewlines@1424B6660`), `country_localised` (`140BBA780`),
`value_limit` (`141580360`), stored state names and the rich-text chunks the
hover draws, which refuse what the viewer does not draw as combat bindings do;
colour codes nest as the font draws them, `§!` returning to the previous colour,
and a withheld line still sets the colour of the lines after it (`142287650`).
Custom trigger tooltips expand scripted text in the raid's event scopes
(`1413931A0` → `SScopeLocalizer` `1402F5F40`) through
`dashboard::scripted_text`. The condition chain's scope and its FROM, a default
scope of the target, are both default-constructed (`140528800`, `140A8D3F0` →
`141580600`), so chains such as `[FROM.ROOT.GetTag]` follow their links
(`141A81250`); inside a scope change only the linked scopes' objects are known,
and a chain or condition that reads their own links is refused. The target
variables stay set, and a unit leader's scope reads as its character
(`140529310`). `trigger.rs` keeps only the condition traversal, native leaf keys
and evaluator joins, until the unified trigger evaluator replaces them. The raid
line order, capture joins, success mappings and named missing-input failures
remain raid-specific. Native modifier values use the existing recorded
modifier-definition feature rules; there is no separate modifier-description
catalogue here.

## Verification

Viewer tests cover the outer tooltip branches, fixed formatting, success
mapping and counterplay, intel arithmetic, native-order equipment maps, naming
bytes and content merges; the engine readers' tests cover memory faults and
capacity demand. The recorder
oracle compares saved references and available raw scalars and reports the
runtime-only remainder explicitly. Live fresh-save agreement, hover appearance
and gameplay cost require the coordinator's matched capture and smoke run.
