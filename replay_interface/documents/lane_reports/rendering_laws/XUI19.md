# XUI19 decisions and missions

Disposition: **PROVEN — PRODUCTION-PROMOTION-READY FAIL-CLOSED CONSUMER; EXACT
UPSTREAM CARRIER REQUIRED**.

The replay-owned panel seam is implemented in
`replay_interface/native/game_ui/src/decision_replay.rs`. It joins the effective
definition catalog, A12's exact target/cooldown laws, shipped card chrome, and
UI13's authenticated runtime text evidence. A panel is admitted only from a
complete ordered producer snapshot; missing or inconsistent semantic state
rejects the whole panel. No renderer-side Clausewitz evaluation or visual fit is
allowed.

## Effective definition and presentation laws

The frozen effective playset (`vanilla`, then `AMSB - DEV`) parses 52 decision
files and 70 category files with zero warnings, omissions, or collisions:

| Fact | Exact count |
|---|---:|
| decisions / categories | 9,526 / 611 |
| plain / targeted / timed card classifications | 7,875 / 346 / 1,305 |
| untargeted / country-target / state-target | 9,180 / 234 / 112 |
| visible / available / allowed clauses | 9,224 / 2,302 / 1,648 |
| activation clauses | 238 |
| political-power / custom costs | 8,799 / 180 |
| re-enable / remove / mission-timeout declarations | 406 / 1,233 / 290 |
| complete / timeout / remove effects | 8,995 / 286 / 1,186 |
| `is_good=yes` / `is_good=no` | 51 / 227 |
| authored priority / fire-once | 926 / 1,056 |

These are definition facts, not evaluated runtime results. State targets are
the explicit numeric A12 branch; a targeted definition without a state-target
declaration uses A12's string/country member. The pinned corpus has no province
target producer.

Shipped `countrydecisionview.gui` has exactly one each of
`decision_item`, `targeted_decision_item`, `timed_decision_item`, and
`on_map_decision_locator_item`. It provides the item background, good/bad
progress sprites, decision icon/default, target flag, timer icon/text, selection
button, and row-name text surfaces. `topbar.gui` provides separate exact counters
for available-to-take and near-timeout decisions. File constants are 30 days for
the timeout alert, 13 days default completed/failed mission retention, and five
rows in the alert tooltip.

## Runtime and capture laws

A12 proves:

- `CCountry+0xFA0 -> CDecisionStatus`;
- re-enable vector `+0x58`, remove vector `+0x70`;
- `CDecisionCooldown+0x08 -> CDecisionDef`, signed remaining days at `+0x10`;
- active targeted-timed vector `+0xE8` and active targeted vector `+0x100`;
- both targeted vectors use the same `0x60`-byte record and vtable RVA
  `0x27D5038`: definition `+0x10`, country/string target `+0x20`, numeric
  target `+0x24`, lifecycle state `+0x28`, ignore flag `+0x2C`, and signed
  days `+0x30`.

UI13 closes the remaining captured card law. `timer_text` received raw signed
integer 180 through formatter tag 5, then exact text bytes
`11 48 31 38 30 20 64 61 79 73 11 21` (`\x11H180 days\x11!`). By contrast,
120 named `cost_and_timer_text` writes were localized row names such as `War
Propaganda`; the widget key is not permission to synthesize numeric cost.

The authenticated final UI13 constructor burst at events 150338..151000 adds a
more discriminating fixture. It contains exactly 22 ordered rows: 20 standard
and two timed. Standard-row names came from caller RVA `0x1D62ECD`; timed-row
names came from `0x1D63448`. A separate sibling text surface received exact
political-power display strings from `0x1D639B7` after formatter tag 9 on 17
standard rows. `Commandeer Civilian Trains`, `Send Guest Workers`, and `Recall
Guest Workers` emitted no cost text in this fixture. Caller RVA `0x1D62B22`
emitted seven headings in exact captured order: Propaganda Efforts, War
Measures, Political Actions, Economic Policy, Prospect for Resources,
Concessions to the Germans, and Foreign Influence.

The fixture is frozen in `captured_panel_fixture.json` from trace SHA-256
`147248c4be2dca0f35ede4dfe7d746e69ce498213fb24268c8f6263c4ccb4f83`
and capture-manifest SHA-256
`c62b619a8b059bc6370430d2613a3647734e414fd2f513bc8644c876b0580056`.
It proves the observed producer/text/order facts only; it is not a generic
controller ordering algorithm.

The retained human action closes one full effective-cost transaction. Before
activation, `Exhort Heroism` was written by `0x1D62ECD`, formatter tag 9 emitted
`100`, and `0x1D639B7` wrote the exact political-power cost bytes. After the
click, `0x1D63ACB` wrote `\x11H180 days\x11!` for the same row. The topbar
political-power producer `0x188A17A` changed from 227 to 127, exactly matching
the captured cost. The marker-bound after-click screenshot is SHA-256
`25f4db8b4141b60b3fd701cebe86fc47d47ed7f4924712c4e061e94a28cacdd9`;
the foreground before screenshot is
`74a4c0719e5695f863fcb505c390dd0c21ec07e1940b248d0955cfd3a76044aa`.
`click_transition_fixture.json` freezes the byte offsets and raw-line hashes.
This is an exact transaction law, not permission to assume all costs use
political power or that authored base cost always equals charged cost.

## Separate save persistence oracle

The independent later `1944.8.1.1` vanilla autosave is deliberately kept
separate from the `1944.5.1.1` UI13 capture. Its save SHA-256 is
`bbf2172b39d4108b394411dafb478ac4dc37e31371e48c0a2880a6d42e3f5504`;
the Rakaly 0.8.17 melt is
`cff31a2fa58623a0728a7503b4379bae0643958ed88301d8324b77ff3424e556`.
Across 439 countries it contains 5,042 repeated decision-status records:
3,423 active targeted, three active targeted-timed, 91 active timed, 299
re-enable cooldowns, 29 remove cooldowns, and 1,197 random items.

The exact flat record shapes are `(decision,target,ignore,days,state)` for both
targeted families, `(decision,days,state)` for active timed,
`(decision,days)` for both cooldown families, and
`(count,decision,target)` for random items. Persisted semantic states are
`available`, `active`, `completed`, and `re_enable_cooldown`; days are signed,
and targets use both string and numeric encodings. This is a persistence oracle,
not a visible-panel/order/evaluated-trigger/cost/text/alert snapshot.

The shared serializer-token equality fixes the numeric state tables. Timed rows
map `0..4` to `active`, `available`, `completed`, `removed`,
`re_enable_cooldown`; targeted rows map `0..4` to `available`, `active`,
`re_enable_cooldown`, `completed`, `removed`. Unknown values remain unknown and
are rejected by the fail-closed live reader.

The save has `enabled_mods=[]`, while the static XUI19 census intentionally
uses `AMSB - DEV` with `replace_path="common/decisions"`. Consequently the
439-known/357-unknown join against that catalog is expected cross-playset
provenance, not a missing-definition or parser defect. Full rows are retained
in `save_persistence_fixture.json`; the master list records only the useful
aggregate.

## Binary provenance

| Function/address | Finding | Job ID | Artifact path | Manifest SHA-256 |
|---|---|---|---|---|
| `0x140718C50`, `0x14072BE70`, `0x140726DF0` | `CDecisionStatus` size and exact persisted vectors | `20260716T231714Z-d45-cdecisionstatus-persistence-v8-ff03b468` | `C:\Users\Example\ghidra_agent_exports\20260716T231714Z-d45-cdecisionstatus-persistence-v8-ff03b468\evidence\func_14072be70.decompile.c` | `06258b0790148cd6af7770579f925941039475d7239ef52a4cc22e109365f7e6` |
| `0x140160840`, `0x14072BE20`, `0x140726BC0` | cooldown record definition and signed remaining days | `20260716T232041Z-d45-cdecisioncooldown-persistence-v11-cff9bc19` | `C:\Users\Example\ghidra_agent_exports\20260716T232041Z-d45-cdecisioncooldown-persistence-v11-cff9bc19\evidence\func_14072be20.decompile.c` | `92424641d90c8d10d21f6891057c3899bfbb19534344dd40a180fec6ed3f63aa` |
| `0x14072C120`, `0x140727A70`, `0x14072C1F0`, `0x140727D90` | shared targeted/targeted-timed record persistence: target, lifecycle state, ignore flag, and signed days | `20260716T231014Z-d45-decision-persistent-methods-v5-a000cabd` | `C:\Users\Example\ghidra_agent_exports\20260716T231014Z-d45-decision-persistent-methods-v5-a000cabd\evidence\func_14072c120.decompile.c` | `c90836d74d6c1d8076f30adf99524cae8d02f4cd904b2d062950f27fcf21e7ed` |
| `0x14011FEB0`, `0x14072BDE0`, `0x140725A40` | exact target union persistence | `20260716T233024Z-d45-stargeteddecisiontarget-persistence--b215db5d` | `C:\Users\Example\ghidra_agent_exports\20260716T233024Z-d45-stargeteddecisiontarget-persistence--b215db5d\evidence\func_14072bde0.decompile.c` | `0fa9c105b7de8ec3edba061b6187b67a39aace98aa2bde31ba6d13ac4e7f8665` |
| `0x141D619A0`, `0x141D62040`, `0x141D60840`, `0x141D61410` | decision-row widget constructors/binders | `20260716T023917Z-ui13-decision-timer-cost-producers-d1-a9c5eb8b` | `C:\Users\Example\ghidra_agent_exports\20260716T023917Z-ui13-decision-timer-cost-producers-d1-a9c5eb8b\evidence\func_141d619a0.instructions.txt` | `965b1e5219a3cff1417b52c66a8373d8776708fce7de1f2b3ee0e2f28cd43da9` |
| `0x141D5B170`, `0x141D5B9C0` | ordinary decision/event item constructors and callback presenters `0x141D5ED20`, `0x141D5FDA0`, `0x141D5ECD0` | `20260716T023917Z-ui13-decision-timer-cost-producers-d1-a9c5eb8b` | `C:\Users\Example\ghidra_agent_exports\20260716T023917Z-ui13-decision-timer-cost-producers-d1-a9c5eb8b\evidence\func_141d5b170.decompile.c` | `965b1e5219a3cff1417b52c66a8373d8776708fce7de1f2b3ee0e2f28cd43da9` |
| `0x141D5BEB0`, `0x141D5C290` | targeted/timed item constructors and callback presenters `0x141D5F070`, `0x141D60080`, `0x141D5F130` | `20260716T023917Z-ui13-decision-timer-cost-producers-d1-a9c5eb8b` | `C:\Users\Example\ghidra_agent_exports\20260716T023917Z-ui13-decision-timer-cost-producers-d1-a9c5eb8b\evidence\func_141d5beb0.decompile.c` | `965b1e5219a3cff1417b52c66a8373d8776708fce7de1f2b3ee0e2f28cd43da9` |
| `0x141D5F070`, `0x141D60080`, `0x141D5F130` | targeted callbacks resolve `param_1+0xFB8` or `+0xAB0`; `0x141D60080` selects the current country through game-state `+0x520/+0x524` and tests record `+0x2C` before presenting the targeted row | `20260717T140746Z-xui19-a34-callback-roots-v22-v2-21a4fc4d` | `C:\Users\Example\ghidra_agent_exports\20260717T140746Z-xui19-a34-callback-roots-v22-v2-21a4fc4d\evidence\func_141d60080.decompile.c` | `7b17d1065ee5df0c89786cf73f0dce84c3f59f6ca556106028a0ab67dc8efde4` |
| `0x141D62E10` (`0x141D62ECD`), `0x141D633B0` (`0x141D63448`), `0x141D63830` (`0x141D639B7`, `0x141D63ACB`), `0x14188A020` (`0x14188A17A`) | exact captured callsites resolve to standard/targeted row controller, timed row controller, shared cost-or-days presenter, and topbar political-power update function; `0x141D63830` emits `DECISION_COST_COMPACT` or `DECISION_AMOUNT_DAYS` according to controller state | `20260717T140803Z-xui19-a34-captured-controller-producers--28c571bb` | `C:\Users\Example\ghidra_agent_exports\20260717T140803Z-xui19-a34-captured-controller-producers--28c571bb\evidence\func_141d63830.decompile.c` | `cd3c09e544ee9a1fdcf63b735c5b6bce8d95b501de009073723bd8aa118b5508` |
| `0x140082CB0`, `0x14188BA30` | serializer tokens `active_targeted_decision`/`active_targeted_timed_decision` xref the shared persistence function; both decision alert counter keys xref one topbar widget-binding constructor (`0x14188BA30`). The searched generic alert names were absent and `DECISION_TIMEOUT` had no recorded code xref. | `20260717T140819Z-xui19-a34-state-alert-order-anchors-v22--9b59d2cf` | `C:\Users\Example\ghidra_agent_exports\20260717T140819Z-xui19-a34-state-alert-order-anchors-v22--9b59d2cf\string-xrefs.txt` | `cd33527ef2f8f5d73ac87272d13b3c6219927272875ffe379ec7f4e1f4f8f8fe` |
| `0x14188BA30` | exact topbar widget-binding constructor: to-take text/background bind at `+0x98E8/+0x98F0`, timeout text/background at `+0x9938/+0x9940`, and decision glow at `+0x9948`; all five surfaces are initially hidden. This is not the membership/count accumulator. | `20260717T220451Z-xui19-a34-decision-alert-counter-produce-0ab3e85b` | `C:\Users\Example\ghidra_agent_exports\20260717T220451Z-xui19-a34-decision-alert-counter-produce-0ab3e85b\evidence\func_14188ba30.decompile.c` | `e81dd60fdad63039236cf81c6627b3410babbad22f098b7395a47bb946e60623` |
| `0x14188F940` | nearby topbar refresh helper controls pause-state surfaces and does not reference the five decision-alert members; retained to prevent it being misidentified as the alert accumulator. | `20260717T220729Z-xui19-a34-topbar-refresh-and-alert-membe-f1527e95` | `C:\Users\Example\ghidra_agent_exports\20260717T220729Z-xui19-a34-topbar-refresh-and-alert-membe-f1527e95\evidence\func_14188f940.decompile.c` | `ce5ea740a06635a60416f4093a7b5822574fb7203415858330523895e0813120` |
| `0x142232AB0` | exact formatter body used by captured timer output | `20260716T022510Z-ui13-historical-formatter-entry-exact-a7-f4139990` | `C:\Users\Example\ghidra_agent_exports\20260716T022510Z-ui13-historical-formatter-entry-exact-a7-f4139990\evidence\func_142232ab0.instructions.txt` | `2f6af2c08184ee78c8797d5b8a3122bb8653c9e450ae46dddd3ad56de519de4d` |

The earlier depth-two export of `0x14188BA30` failed while traversing an
external symbol and is retained as job
`20260717T142137Z-xui19-a34-decision-alert-counter-produce-eb670641`; it is not
cited as successful evidence. The reassigned depth-one export above succeeds
and corrects the earlier interpretation: the shared string-xref root is the
widget-binding constructor, not the later alert-membership/count accumulator.

## Production admission

The typed seam admits only `producer_complete=true` snapshots containing:

- final contiguous presentation order and exact decision/category identity;
- evaluated availability and semantic pending/running/completed/failed/removed
  lifecycle;
- A12-typed country/state target;
- effective political-power/custom cost and exact producer-formatted text;
- live timer plus re-enable/remove cooldown remaining days;
- exact runtime name/timer text;
- per-row membership and exact totals for both decision alert counters.

It rejects incomplete producers, unknown definitions, category or target-scope
mismatches, duplicate/unordered rows, active rows not running, cooldown rows
without positive days, numeric timer/text asymmetry, and alert mismatches.
Measured-empty is admitted only as a complete empty row list with both counters
zero.

The current A12 replay handoff is exact for status counts, active timed rows,
both targeted vectors, delta events, and both cooldown vectors. Scheduled JSON
now reads `CDecisionStatus+0x100` as `targeted_live` and `+0xE8` as
`targeted_timed_live`; every row preserves exact decision identity, typed
target, lifecycle state, ignore flag, and signed days. Re-enable and remove
cooldown rows preserve exact decision identity plus signed remaining days in
scheduled JSON and as D45 snapshot events (kinds 6 and 7) through the recorder
projection. D45 schema v4 now carries both targeted families through the compact
stream and replay projection using source kind 2 for `+0x100` and source kind 1
for `+0xE8`; each target row carries the typed target, exact lifecycle state,
ignore flag, signed days, and decision token. Legacy v2/v3 payloads remain
decoder-supported and cannot claim the new fields. The handoff still does not carry the complete ordered visible
panel, evaluated trigger results, generic arbitrary per-row effective/custom
costs, post-result retention state, generic resolved text/order, or alert
membership. This is an
active extractor deep-RE dependency, not a rendering partial. Its exact separate contract is
`extractor/documents/dossiers/A34_XUI19_DECISION_PANEL_INPUTS.json` and the
concise master row is in `extractor/documents/RE_REMAINING.md`.

## Incidental findings retained

- The live effective playset contains 9,526 decisions, one more than the prior
  generated `decisions.json`; production must rebuild the static bundle rather
  than silently reuse the stale file.
- `cost_and_timer_text` is a misleading authored name in the captured state; it
  carried row names, not numeric costs. Numeric political-power text was emitted
  to a separate sibling surface by `0x1D639B7` on 17 captured rows.
- Three standard rows in the authenticated constructor fixture emitted no cost
  text; absence must remain absence rather than being filled from the widget
  name or neighboring rows.
- One exact activation charged 100 political power (227 to 127) and replaced
  `Exhort Heroism` cost text with an exact 180-day active timer; the timer and
  cost use distinct producer RVAs.
- The separate vanilla save proves six repeated persistence families and four
  semantic state values across 5,042 records. Its static-catalog mismatch is a
  playset mismatch and must not be promoted as a parser defect.
- `CDecisionStatus+0x100`, not the older transient `+0x88` alias, is the
  persisted active-targeted vector. The shared serializer/deserializer and the
  save record shapes independently agree on target, state, ignore, and days.
- Completion/failure retention (default 13 days) is distinct from the authored
  decision/mission timeout and must remain a separate lifecycle input.
- D45 cooldown snapshot events are baseline-bearing state rows despite sharing
  the decision-event envelope; kinds 6 and 7 must not be interpreted as
  transitions.
- Authored `priority` exists on 926 effective definitions, but it is not the
  final materialized UI order and is never used as a fallback.
- The same depth-one export distinguishes ordinary decision, event, targeted,
  and timed item classes and their callback presenter roots. Those findings are
  retained even though only the targeted and timed callbacks are direct A34
  closure targets.
- The two persisted targeted families have different numeric state-name tables.
  The shared reader formerly labelled both with the targeted table; it now
  selects the targeted table for `+0x100` and the timed table for `+0xE8`, while
  preserving the raw numeric state in both carriers.
- `0x141D63830` is a single state-sensitive presenter: controller state zero
  emits `DECISION_COST_COMPACT`, state one emits `DECISION_AMOUNT_DAYS`, and
  other states hide the surface. This confirms the captured cost/timer
  transition is one producer law rather than two unrelated widgets.
- `0x141D643A0` repeats that cost/days law for the targeted subcontext at record
  `+0x18`; it is not an independent formatter rule.
- `0x141D65890` maps record `+0x28 == 1` to `TIMED_DECISION_COMPLETED`, `== 3`
  to `TIMED_DECISION_FAILED`, and all other values to the live
  `DECISION_AMOUNT_DAYS` path.
- `0x14188BA30` binds and initially hides the two decision-counter texts, their
  two backgrounds, and the decision glow at exact topbar member offsets. The
  negative result is important: it does not calculate alert membership or
  counts, so the extractor must still carry those facts explicitly.

Mechanical evidence is frozen under
`replay_interface/re_evidence/rendering_laws/XUI19/`.
