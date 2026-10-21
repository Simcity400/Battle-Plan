# replay-data

`FactionAccounts` keys autonomous accounts by native array index, independently of membership, and preserves balances, gains/baseline, selections/history, projection/shares/rank. `FactionVariables` has an explicit seed occurrence then literal-key rows. `FactionResearch`, `FactionMinimumUpgrades` and `FactionDoctrine` preserve independent sharing/selection operands. `FactionProjection` retains independently refreshed totals/effects. `FactionResources` retains every resource slot; owner zero explicitly denotes the faction-system pool. Held goal entries include their active-section index vector. Writer scheduling and carry conditions are listed in the cadence audit.
Project runtime variable stores share immutable ownership across row clones while retaining their ordinary nested wire encoding.

Intel source rows retain independent source/contribution pool extents and ordered occurrences. Dense zero values are implicit only inside their recorded native extent; keyed zeros and duplicates remain explicit.

The recorded data contract: engine-native rows, typed tables, cadence, exact
values, content identity and their bounded codecs. Frame, spool and container
implementations consume these definitions; this crate depends directly on the
native libraries that own its value types and on `engine-allocation` for checked
growth. It depends on
[replay-identity](../replay-identity/README.md) for the content identity types
its `identity` module writes and reads. [weather-period](../weather-period/README.md) supplies the shared weather-period types and calendar arithmetic, re-exported by `temperature`.

`FactionUnlocks` preserves the two scripted access bytes independently. `FactionRules`, `FactionPrograms` and `FactionManpower` are independent ordered occurrence tables keyed by full faction handle and native ordinal. Together with the complete faction roster, empty tables certify empty lists. Manpower keeps signed values; programme references retain both native words.

`FactionGoalStatus` retains each full faction handle and nullable manifest. `FactionGoalEntries` keys individual native occurrences by owner, list kind and ordinal: runtime goals (including pending states and the held progress cache), nullable completed/cancelled histories, signed extra slots and term dates. Complete owner rows distinguish empty lists from unavailable capture. Definitions and presentation read the recorded content stack.

WarParticipation retains both directional ledgers as separate signed fixed-point columns. WarCapturedProvinces retains first-credit membership independently; WarLendLeaseHistories retains all nonzero native delivery histories for recorded war-roster owners, including targets outside the displayed side; a complete table certifies omitted rows as exact zero for those owners. Playback applies the native same-side display gate. `war_participation` derives the native breakdown and total, with maximum unknown-casualty credit, wrapping arithmetic and Q15 participation shares. Content supplies the lend-lease factors. WarOverviewCountries supplies representative selection and does not represent participation.

| Source | Responsibility |
|---|---|
| [lib.rs](src/lib.rs), [format.rs](src/format.rs) | Shared hour/session/branch types, writer version and format admission. |
| [rows.rs](src/rows.rs), [rows/compatibility.rs](src/rows/compatibility.rs) | Table registration, row dispatch, field masks, native representations and proven wire/meaning history. |
| [extended_rows/](src/extended_rows/), domain row modules | Recorded operands, ordering, observation boundaries, completeness and named refusals. |
| [cadence.rs](src/cadence.rs), [sliced.rs](src/sliced.rs), [cadence-audit.md](cadence-audit.md) | Read scheduling, owner slices, carry rules and supplying hours; the native writers that keep each hourly table hourly. |
| [codec.rs](src/codec.rs), [value.rs](src/value.rs), [fixed_point.rs](src/fixed_point.rs), [display.rs](src/display.rs) | Bounded decoding, exact value encoding and arithmetic, and game display steps. |
| [identity.rs](src/identity.rs) | The recorded form of the content and session identity (`IdentityCodec`); the types and their validation are in [replay-identity](../replay-identity/README.md). |
| [checkpoint/](src/checkpoint/), [rows/checkpoint.rs](src/rows/checkpoint.rs) | Prepared row and value codecs, independent of frame snapshot framing. |
| [country_flags.rs](src/country_flags.rs), [day_night.rs](src/day_night.rs), [naval_trees.rs](src/naval_trees.rs), [temperature.rs](src/temperature.rs) | Shared recorded operands and exact calculations, including the operands that pick a country's flag. |

Output workers can hold `codec::error_details_scope` or use `codec::with_error_details` to retain validation reasons and row/column context. Scopes nest and restore the calling thread's previous setting on return. Outside that scope, installed capture allocation hooks keep static I/O errors allocation-free.

Owner-slice updates validate membership once per distinct owner and merge ordered
owner groups without sorting. Equal owner slices bypass row encoding; complete
table deltas size only changed rows and reserve deletion storage only for actual
deletions. Nested payloads use the enclosing row/change's checked reservation
instead of sizing themselves again, and checked scalar-vector clones copy the
slice in one operation. Display-step rounding uses bounded unsigned arithmetic
with the same sentinel, half-step, nonzero and overflow rules. None of these
paths changes the recorded wire layout or values.

Template layout and settings payloads encode acquisition refusal as a recorded optional value. This remains distinct from historical `NotRecorded`, which current ordinary spool rows cannot emit. Valid settings share their held allocation; a refused settings payload supplies no default priority, restriction or roster position.

Row declarations own versions, field counts, keys, representations and
availability. Writers emit the current format; readers admit unchanged contracts
through the compatibility history and refuse changed tables by name. Preserve
recorded absence separately from native empty, zero or absence. Content facts
remain resolved through the recorded content stack. Shared rules are in
[AGENTS.md](../../AGENTS.md); frames are in [replay-frame](../replay-frame/README.md)
and recording I/O is in [replay](../replay/README.md).

The `test-support` feature exposes synthetic row fixtures, historical fixture
encoding and thread-local decode observations for replay's frame/container tests.
These helpers are absent from ordinary builds. Unit tests exercise round trips,
display steps, historical prefixes, delta ownership, malformed input and wide
country lists. [replay-frame](../replay-frame/README.md) owns frame and baseline tests;
[replay](../replay/README.md) owns recovery and complete recording tests. Run
`cargo nextest run -p replay-data -p replay-frame -p replay` for all three.

Terminal failure witnesses live in [witness.rs](src/witness.rs), so capture
helpers can report exact failures without depending on compressed recording I/O.
The row registry owns `rows::TABLE_COUNT`; table accounting needs no frame engine.

`AgencyOperationBuffers` holds one observed shared-client snapshot: completed, running, ready-preparation and unready-preparation references in native order, with each reference's resolved target state. Empty lists are recorded; unavailable acquisition remains a table refusal. UI membership is runtime state, independent of saved operation-roster membership.

ResourceOrigins and ResourceRights carry native list ordinals. ConvoyClients carries independent optional owner/counterparty ordinals for the directional lists; imports and exports use ResourceExchanges ordinals. Ordinary and prepared codecs retain these fields with the rest of each existing row.

TradeRoutes source_state and destination_state retain market-referenced routes' native state endpoints separately from naval ports. Both optional columns belong to the complete format-103 introduction and use ordinary delta and prepared codecs.

Manpower consumers use States locked and total pools, CountryStabilityInputs recruitment/exile numeric columns, CountryCounterInputs raw recruitment entries and sent-expeditionary references, Ships acquired/required crews, AirWings acquired crews and exile attribution, StateResourceInputs recruitment operands and Politics election interval and optional initializer-date cache. Ordinary next election dates derive during playback. Quiet certified values remain ordinary replay rows and sliced values keep their supplying read hours.

`ManpowerReferences` retains an immutable shared ordered reference payload. Ordinary and prepared encodings preserve the count, nulls, duplicates and full handles; row clones share storage without copying a quiet roster.

DivisionTemplateIcons.manpower_exile retains the native template-origin predicate used by ordinary and exile manpower accounting. It is independent of movement exile and optional model availability.

StateResistance is hourly province-hover data, independent of the daily construction and garrison packs. Its hover operands preserve exile host and raw legitimacy, spread source and amount, the cached compliance-gain ratio, ordered scripted target additions and the two local effect accumulators, their eligible keys and the counts of non-flat child/custom-tooltip forms. Modifier sources retain section and source order, optional effective totals and their printed-leaf trees or a named failure. An absent total is unavailable, never a zero; its existing failure field names the reason. Both resistance growth/decay source trees are recorded under source certificates; playback selects the current branch from its exact derived target rather than stored speed. Ordinary column deltas carry unchanged data. Shared displayed stability arithmetic lives in [country_stability.rs](src/country_stability.rs), used by the dashboard and occupation hover.

StateResistance hover/source packets use opaque native codecs. Their catalog admits the complete current layout, retaining the declared suffix indices for schema accounting; it does not promise absent-field decoding or add an older-recording decoder.

CountryWargoals, WargoalTargets and WargoalJustifications hold the complete per-country inventory, the separate unused-goal roster, all ordered target occurrences and stored justification progress, costs and dates. Their typed row API retains native ordinals and full handles. Expiry is metadata; only native roster removal removes a goal. Justification completion is derived with the native stored-end/remaining-cost display rule. Action eligibility is separate from these inventories.
MIO details preserve independent daily balance/level columns, completed traits, ordered planned queues and cursor, policy/cooldown, task slots, variables and flags. Country policy rosters retain runtime cost and cooldown days. Design authorship and frozen bonuses are independent VariantStats columns; organisation history retains optional design dates. Trait coordinates and source definitions resolve through the recorded content stack.

Country policy entries also preserve the cached localised name, country context and selection score. These are stored runtime values; replay does not recalculate the cached name in a later country scope.
VariantDefinitions retains saved ancestry, attribution, role and applied design
state. VariantChanges preserves ordered observed snapshots, compact eligibility
edits, compact source-reference edits, shared author-liveness occurrences and identity tombstones for removal. A complete empty interval means no observed
change; unavailable observation never certifies unchanged state. Snapshot child
ordinals and conditional stat residuals are validated independently of their
enclosing row key.

VariantBonusSnapshots retains exact immutable applied source versions, shared by
ordered references from design states and mutation history. Named sources keep
their country-local handle, name and description; unresolved native handles
remain in the design's original ordered roster.

DesignCountryModifiers retains effective numeric operands for the country/key
scopes demanded by saved designs. A known absent accumulator key is zero;
unavailable country or modifier reads retain a failure scope and no value.

DesignCountryTemplates preserves each demanded country's ordered consumable
design-discount roster: signed uses, unclamped fixed5 factor and ordered repeated
equipment tokens. An observed empty roster is distinct from failed acquisition.

DesignObserverContext preserves the game fog switch independently of any
observer/design combination. Its absent or failed acquisition never means fog
is enabled or disabled. Reveal joins it with recorded observer intel/access.

DesignOrganisationLiveness records independently current registry lookup verdicts
for raw author handles reached by saved designs. It is separate from frozen
authorship and bonuses, with no observer/design matrix.

AirWings.replacement_setting retains a separately admitted saved byte. WingEquipment remains the complete ordered equipment bank, including zero amounts; no duplicate bank table is recorded.

VariantChanges.scalar holds compact obsolete, auto-upgrade, highlight, ancestor ceiling, role, name, model, icon, names-group and niche edits. Unchanged module, upgrade, mission and source dependencies carry separately; repeated scalar edits retain native occurrence order.

UnitLeaders retains signed residual experience. UnitLeaderTraitExperience records the complete ordered training vector using leader references, native ordinals, lexical identities and signed progress. DivisionStatus retains optional stored experience independently of the manpower-normalized display; native absence and acquisition refusal remain distinct.

`DivisionHistory` has one summary per demanded division, including empty and refused roots, and one row per native career/award ordinal. Entries share immutable ownership across capture snapshots; their ordinary field-mask and prepared codecs preserve the complete values while unaffected entries keep their allocations.

`UnitLeaderTraitExperience` distinguishes ordered training progress from ordered runtime factor overrides in its key. Both preserve repeated definitions and full signed fixed5 values; missing overrides never invent a multiplier or progress entry.

DivisionStatus retains the carried previous-template handle during deployed conversion, separately from experience.

DivisionPanelSources stores distinct demanded country and commander raw source trees, shared variant-terrain values, state/country attrition terms and ordered nonzero held membership. Each source occurrence has its own key; missing certificates have named refusal rows. DivisionStatus.panel_source selects the country's native identity and commander kind/full handle, including independent commander-selection failure. Certified key-range replacements use the ordinary table delta wire format.

DivisionPanelStats stores the 22 deployed active-cache operands separately from DivisionStatus. Its division key and independent optional numeric columns share the existing rebuild certificate; quiet carries add no native reads. Panel/hover and current-speed products join this table, including prepared selection and checkpoint codecs. DivisionStatus keeps independently changing experience, pending template, officer XP, career counter and source selection.
DoctrineCountries preserves every doctrine-array slot and its raw country
reference. DoctrineFolders and DoctrineTracks retain ordered choices,
completions, reward counts, total and banked mastery, unit-source daily mastery
and effective daily gain in separate columns. DoctrineLeaderMastery retains
both reference words and contributions. Cost reductions, mastery grants and
equipment bonuses preserve ordinal membership, optional filters and raw signed
values, including native entries the save serializes as empty blocks. Content
names and reward benefits resolve through the recorded content stack.

Mastery grants retain signed days at an observed expiry-counter anchor.
`DoctrineMasteryGrantRow::days_at` reconstructs the current day word through its
DoctrineCountries counter with native wrapping arithmetic, including refresh
re-anchoring. Expiry alone changes no grant payload.

Doctrine grant filters retain the named track lexer token independently of
track_index. Consumers resolve the token through the recorded dictionary and
content stack; the optional index keeps its signed native value.
ResearchModifiers carries grant counters, ordered speed/cost grant instances, use/claim operands and technology/category restrictions. Projects.runtime carries a pending reward and the exact ordered variable map/seed, while ProjectPrograms carries ordered scientist assignments and delayed timers. ScientistDaily carries independently demanded daily injury values; the supplying hour derives from its daily read. Unknown child capture stays distinct from a complete empty value.


Faction account statistics use scalar occurrences, so changed influence terms and projection source operands carry independently. Phase identifiers, supplying numerators/denominators and sparse initiative anchors reconstruct refresh results without substituting the current hour's operands. Advisor modifiers retain ordered slot headers, literal source names and independent modifier entries. All faction tables share writer-certified hourly availability; that does not imply hourly native owner scans.

FactionAccounts independently carries an optional effective subject-contribution accumulator operand. Projection derives from recorded pre-division resource/equipment products, a factory component and four supplied unit counts. Missing scalar certificates remain distinct from zero.
Manpower consumers use States locked and total pools, CountryStabilityInputs recruitment/exile numeric columns, CountryCounterInputs raw recruitment entries and sent-expeditionary references, Ships acquired/required crews, AirWings acquired crews and exile attribution, StateResourceInputs recruitment operands and Politics election interval and optional initializer-date cache. Ordinary next election dates derive during playback. Quiet certified values remain ordinary replay rows and sliced values keep their supplying read hours.

FactionAccounts independently carries nullable effective subject-contribution and population-growth operands for demanded current members. Manpower prediction is derived from existing controlled-state and recruitment tables at an explicitly dated supplying hour.

ProjectPrograms.facility holds independently available daily scalar sources and specialization contributions; consumers derive the speed product in source order. ScientistDaily.template_visibility dates null/nonempty/empty retained conjunctions with injury; ScientistDaily.template_origin retains nonempty conjunctions' own filename and line, including role replacements. Projects.runtime includes the pending reward template occurrence; it never records a pointer. The daily scientist projection shares the existing slot and adds no acquisition.

ScientistDaily publishes injury, retained visibility-conjunction state and template origin at daily slot 4, projected from the existing scientist acquisition. Scientists and ScientistSkills keep their hourly roster. Off-slot reads skip the daily table; replay carries its last observation, and the detail panel labels its supplying date. Independent daily read refusals remain unavailable. No native read or census is added.
Combat Log tables retain group and month membership, all three manpower counters, ordered template statistics and battle references, enemy amounts and settled battle sides and arrays. Battle pool indices are native retained-history identities. CombatLogObservers holds ordered active observer operands; aggregate losses reuse LandCombatProgress. The observer key/numeric-column layout is versioned independently; ordinary and prepared row codecs share the same contract.

LandCombatResults also retains supplemental terminal observer operands, once at resolution. Its aggregate terminal losses remain in the existing losses field.
DivisionTemplateIcons exposes full spatial composition as TemplateLayout: each of the three grids keeps its dimensions and every cell in native column-major order. Empty pointers and null-object definitions remain distinct. TemplateSettings keeps priority, restriction and equipment switches, parent and creator/origin inputs, obsolete date, equipment niche, optional division cap and ordered forbidden equipment tokens. These nested values support ordinary snapshots, independent field deltas and prepared row codecs. Native bytes and handles remain exact.

CountryTemplateState holds every canonical country, including an empty template roster: its stored military-staff template handle and country-wide template-lock byte. Stored role choices and cached template manpower, headquarters command-power cost and training time are exposed in TemplateSettings.

Template layouts, settings and garrison selection lists use immutable shared nested values. Their ordinary, delta and prepared encodings are owned native values; cloning an unchanged capture row shares its payload allocation.

CountryTemplateState also retains the raw next headquarters deployment order and country/group/state garrison template selections. Optional scope identifiers preserve state zero independently of group defaults.


OrdersGroupCounterInputs exposes the stored nearest-front province and shared TemplateHqContext: the target template reference, deployed/queued bytes, optional deployment scalars and ordered assembled-variant and required-archetype amounts. Null holders, empty pools and acquisition refusals remain distinct.

TemplateSettings retains current campaign state. The native template/save holders have no paid-edit spending ledger; earlier spending is not inferred from composition or country XP. The transient designer edit cost is not a recorded history, and the cached manpower, command-power and training operands do not constitute the full designer stat sheet.

TemplateSettings retains the selected historical naming-group key and native country-roster position. CountryTemplateState retains the independent garrison reinforcement priority with acquisition refusal separate from every signed value. These facts use the same ordinary, delta and prepared row contracts as the rest of the template configuration. Garrison reinforcement priority uses the optional signed-integer wire kind; schema framing, its derived history contract and prepared rows agree for unreadable, zero, negative and positive values.
Combat Log tables retain group and month membership, all three manpower counters, ordered template statistics and battle references, enemy amounts and settled battle sides and arrays. Battle pool indices are native retained-history identities. CombatLogObservers holds ordered active observer operands; aggregate losses reuse LandCombatProgress. The observer key/numeric-column layout is versioned independently; ordinary and prepared row codecs share the same contract.

TemplateSettings retains the selected historical naming-group key and native country-roster position. CountryTemplateState retains the independent garrison reinforcement priority with acquisition refusal separate from every signed value. These facts use the same ordinary, delta and prepared row contracts as the rest of the template configuration. Garrison reinforcement priority uses the optional signed-integer wire kind; schema framing, its derived history contract and prepared rows agree for unreadable, zero, negative and positive values.
NavalRepairCountries/Fleets/TaskForces, NavalTaskForceComposition, NavalRepairChildren, NavalRepairBases/Queue and NavalRefitShips/Progress retain standing naval policy and native work-order identities. Ordered lists use flat ordinal keys and preserve repeats; refit progress is signed fixed5. Building repair factory limits are separate from naval repair dockyard limits.

NavalRefitShips retains stored work-order links independently of destination lifetime. Its shared
resolve_line rule joins full identities against the complete hourly NavalRefitProgress rows,
including zero progress; daily ProductionLines supplies metadata only. Preserve unavailable
hourly rows separately from a complete empty set when presenting or projecting optional links.
Market rows retain country listings, price maps, subsidy alternatives and independent balances, automation, request/contract manifests and price schedules, contract runtime/variables and current lend-lease terms. MarketEvents preserves schedule revisions, fuel/payment operands, identity lifetimes and changes in native cumulative equipment totals, including final increments before reset/removal. Private holder marks and rare final snapshots are consumed on the worker and never serialized. Fixed-point scales remain explicit; native dynamic exchange references stay complete.

Contract runtime scalars and in-transit equipment have independent codec fields,
so a period-day tick writes no route pool. Agreement pools share immutable owned
storage across held rows, sparse key updates and encoder baselines; equipment
writes copy only the changed pool. Exact key updates encode the same wire delta
as complete tables and move retained rows on membership changes.

Market contract and country collections also share immutable owned storage.
Their prepared-checkpoint codecs delegate to the underlying values, preserving
the same fields and bounded decoding without copying carried collections.
Exact contract-key updates leave sibling contracts in the encoder baseline.
MarketEvents kind 34 identifies an unavailable delivery amount by country,
agreement/contract, source mode and named reason code. MarketEvents stays a
complete row stream, so independent occurrences and explicit holder refusals
survive encoding keyframes and seeks. A refusal's zero payload never represents a measured delivery amount.
States retains optional cached victory-point indices and controller single-group
predicates for demanded scripted-map consumers. Undemanded operands and guarded
read refusals remain absent. SimulationFeatures retains the native state-array
extent, including holes. MapEvents includes successful forced scripted-map refresh
calls by declaration ordinal; their interval coverage follows the other event families.
`IntelNetworkSummaries` holds independent retained national coverage, controlled/core strength sums and maximum state strength for actual owner/target networks, including an empty network. Its daily samples and ordinary change-only deltas do not derive these caches from per-state rows. The current network census supplies membership when values carry between daily rebuilds.

`PropagandaEfforts` holds logical history by the actual strategic-operative manager slot and stable effort sequence. An owner header distinguishes observed empty history from absence, preserves an independent loaded weekly cache, and marks when subsequent native daily appends make drift exactly derivable from the retained saved channels. Append/removal deltas do not repeat surviving history or daily sums.


Network auxiliary rows retain stable worker-issued occurrence identities and separate changed-word columns. Logical kind/subnetwork/ordinal identities preserve list order and duplicates.

[Intelligence ranges](src/intel_estimates.rs) derive deterministic bounds from a true Q15 quantity, normalized intel, content thresholds/spread, semantic salt and the recorded clock seed/month. Native finite sentinels, positive zero-cap fallback, signed wrapping, integer square-root convergence, Q15 bounds and integer rounding are retained. No local panel state or estimate table is recorded.

The intel game-state tables share TableId declarations across registration, codecs, fixtures, compatibility and prepared selections.

DivisionStatus keeps the exact current and effective maximum organisation only
for a moving division whose ordinary movement takes the low-organisation attack
factor. Two numeric columns carry independently; two bits in the existing movement flags distinguish no factor, a required pair and read refusal. The
existing display-stepped organisation fields continue to serve displays.

The opt-in `movement_witness` collector puts a native first-edge speed sample at
the end of the existing unit combat-reference packet. `Witnessed(None)` writes
zero bytes: ordinary packets keep their exact count/reference bytes, field masks
and table columns. A diagnostic refusal is present and distinct from no sample.

Movement-speed companion tables, status suffixes and template speed keys use the current shared schema declarations. Trade-route endpoints retain their registered table identities.

Movement operand availability reuses existing bytes: DivisionStatus's movement
flags distinguish an unreadable air-supply sign, and MovementStateTerms's branch
byte distinguishes a refused conditional attack entry from its controller relation.
Peaceful movement consumes only common state terms; noncore movement consumes the
air sign only after its bypass/core-access gates. No extra numeric columns are used.
EventFeedHistory carries accepted scripted event/news occurrences, chosen signed option ordinals, decision/mission execution edges and retained pending/fired/delayed registry operations. Tokens and pointer-free scope graphs live in per-pack arenas. The nested occurrence value transposes each country, cause, script span and operation operand into its own exact numeric-difference stream using the ordinary column value codec. Generation is implied by the interval row; sequence uses one anchor plus exact unsigned gaps, and event hours are relative to the row's read hour. The grouped container still treats the outer field as opaque; transposition and numeric prediction happen inside each batch before its ordinary patches and compression. Consumers receive the identical ordered interval row, including empty batches and independent refusals. ScriptEventState reduces baselines, swap-removals and counted delayed visits into current retained state, preserving duplicate instances, fired members, signed countdown wrapping and incomplete coverage. A country's delayed queue is built only by its explicit baseline or empty certificate and carried only while certified; a loss, including one on the current interval, retires the queue and any open baseline, and writer deltas outside them stay occurrences without retained state. One certified queue never holds a compact label twice. Baselines never invent past fires. Missing scope target/combat operands remain independent flags.

Fired-event membership supports replacement-time erasure deltas. Decision effect execution and reached lifecycle/duration stores are distinct occurrence sources; delayed decrements reduce sparse pass/cursor boundaries without observing individual timer visits.

Decision operations retain taken multiplicity/order, selected timer transfers and cleanup, lifecycle/duration results, ignore/target marks and ordered description random counters. Admitted hourly and country-refresh pass anchors derive signed conditional decrements off-thread from definition gates and sparse effect/mutation boundaries. Reset/restore withdraw an owner until its next existing-worker baseline. `ScriptEventState::decisions` supplies only completed baseline/writer/pass certificates.

From format `EVENT_FEED_OWNER_REFUSALS_FORMAT` the capture can refuse one owner without a loss: `DelayedRefused` (the target's delayed queue) and the decision `Refused` operation (the actor's decision state) carry their reason as the token and no identity, instance or scope. Each retires only that owner's certificate and any open baseline, like a loss, until its next baseline; other owners, pending and fired keep theirs and the row's coverage is unchanged. The table layout did not change, so every older batch still decodes.

EventFeedHistory.script_refusals qualifies genuine interval losses independently of transport/flight coverage. Supported decision families carry no unconditional refusal. Missing baselines, open passes, lost writers and specific unsupported trigger operands remain unavailable; independently certified event registries and known occurrence rows remain usable. Ordinary and prepared codecs retain the mask.

The final decision rebuild operation carries its optional certified fixed5 political balance; other decision operations add no balance bytes. Retained script reducer continuation serializes owner maps as ordered pairs and preserves baseline/loss/pass state through preparation resume.

RaidAirScores holds the regional friendly and enemy active-mission counters alongside the superiority weights and movement impact. Zero-weight entries with a nonzero counter remain present, because they select the Air map mode's colour. Its independently optional raw aircraft spotting scale supplies naval patrol normalization; scale-only and unknown-scale entries remain present. These unsaved values follow the same phase-carried read as the weights, and witnessed omitted pairs have an observed zero scale.

RadarCoverage carries an optional writer-certified radius for coverage symbols; host centres derive from content. A radius-only delta carries the unchanged province membership, and absent certification is distinct from a zero radius.

AirMissionInputs supplies the regional interception alert's retained trace-count predicate as an independently optional 0/1 field; it changes independently of the current sortie operands.

AirGroundCrews holds the writer-certified enabled flag for pool-owner/region pairs demanded by registered Air missions. A complete table omits observed false flags; an explicit absent field is an independent acquisition refusal. It uses the existing hourly mission worker and carries quiet flags without native reads.

AirWings attached_orders_group covers the Air base grid as well as tactical targeting. A null already-gathered pointer proves zero; a nonnull pointer joins the same pass's authenticated OrderRoots. An unavailable identity remains absent rather than becoming a detached wing. This adds no native read or wire column.

ShipModels.has_criticals retains the existing held critical-array nonempty predicate for naval repair-base eligibility. Absent observation stays independent of current hull HP.

AirWings.map_flags packs independent known/value bits for training-stop and attached-group coverage. Unreached or uncertified fields remain unknown, distinct from observed false.

ShipModels.air_capacity_kind holds the independently refit-certified ordinary/special carrier subtype. Land bases and mega carriers derive their subtypes from existing rows. An absent non-mega operand refuses capacity independently.

AirWings.map_flags also packs independently known raid-assignment liveness in bits4/5. It is distinct from the idle-transport raw-reference observation and adds no column.

AirRegionalMissions.active_order is an optional stable increasing label for the owning country's all-active mission vector. Regional ordinal and global paint order remain independent; missing order certification does not invalidate regional registrations. Gaps have no meaning.

AirTransferInputs holds each demanded relocating wing's destination, independently certified incoming ordinal and admitted raw progress/total distance. Known nonmembers are omitted; an explicit unknown ordinal preserves failed witnessing. Incoming order and progress have separate native writers and availability.

AirMissionInputs current_type and effective_aircraft retain the simulation execution words independently of selected missions, the saved active flag and retained combat high-water counts. A zero current mask leaves the count absent; missing demanded fields refuse regional statistics.

CountryCounterInputs.air_modifiers retains ten optional raw country Air modifier entries in the native purpose-key order. The containing optional packet distinguishes a failed complete accumulator read from known absent entries; it shares the counter row's writer-certified carry and daily verification. Playback applies recorded modifier metadata and feature eligibility.

AirMissionInputs.normalization retains executing nonzero-count wings' selected night statistic and independently acquired raw night/weather entries. Requested-key feature gates can prove a present entry without reading its disabled raw value. AceRangeInputs.normalization projects living aces' raw entries from the existing complete range accumulator read. WeatherRegions.air_detection projects its raw entry from the existing rotating-window accumulator read.

AirSortieModifiers uses its additional detection-override scope and singleton availability scope to distinguish certified empty state from an unavailable vector. These are actual simulation-console state; no local mode, panel or evaluated hover is captured.

WeatherProvinces.daylight belongs to slot zero. Its completed-update marker derives the seasonal direction and feathering from the frame date and recorded content; its held packet preserves suspended/startup direction and native feather words. Independent missing held operands refuse the Air fuel forecast. The province vectors derive from the recorded map geometry and graphics defines through day_night::ProvinceClock::forecast.

NavalAccidentReports retains the separate ordered accident and mine-report families under country/kind/ordinal keys. Stored region, full equipment/ship references, damage, raw sunk byte, victim and mine-owner references and expiry hour stay distinct from current live ship state. Complete empty tables prove no reports; unavailable certification remains a refusal.

VolunteerHosts.air_capacity_modifier independently retains the directed effective Air volunteer-capacity modifier. RaidCountryInputs.volunteer_stockpile_planes retains demanded senders' uncapped signed whole-plane stockpile total before raid caps; unavailable certification stays distinct from zero.

AirWings uses independent numeric fuel columns for raw cached fuel, current execution/count, potential fuel context and reached training capacity/override liveness from the existing worker census, including idle and unregistered wings. AirMissionInputs reuses the same execution observations. AceRangeInputs.fuel_modifier projects living aces' raw entry from the already copied complete range accumulator; outer absence differs from a known absent key. Changing execution counts delta independently of quiet fuel statistics/context/training values. No new table, hook or acquisition job is used.

ShipModels.naval_power retains independently optional current selected-design dominance operands: definition weight, range, speed, classification-selected carrier size and the heavy attack statistic. It shares the existing hourly worker census and certified carry, without a new table.

FleetCounterInputs carries a certified mission-region multiset as an immutable shared vector. Quiet hourly rows share their payload without per-fleet allocation; the nested codec writes only explicit region identities and preserves duplicate occurrences.

NavalMissionTaskForces retains active kind-three convoy-detection operands independently of requested mission convoy counts. raiding_spotting and raiding_region are independently delta-encoded nullable numeric columns; raiding_targets keeps only the stable pair of stored targets' native liveness. Activation derives from TaskForceCounterInputs.is_in_regions. Inactive missions omit unreached fields; an active mission with an absent field refuses that field.

CountryCounterInputs.mines_effect_reduction retains the independently optional raw mine-penalty reduction entry. A failed complete accumulator census remains distinct from a known absent key; existing country writer certificates and daily verification carry it.

AirBaseRegistrations has independent optional numeric fuel_demand_raw and fuel_allocated_raw columns for state containers. Its existing optional pool packet carries first/change subtype identities and boundaries, including empty and repeated pools; state pool fuel results remain absent because playback derives them. Numeric updates do not rewrite pool metadata.

NavalDominanceTaskForceRanges holds current range operands only for otherwise omitted siblings of registered strike fleets. It uses the same row contract as NavalMissionTaskForces and shares its changed-owner country/commander sources. Both tables share immutable headquarters lists across unchanged observations. A refusal is independent of ordinary mission operands; playback joins it only for a missing demanded sibling.

AirMissionInputs.detection_stat_13 and detection_stat_14 retain the independent raw cached aircraft-detection words selected by positive one-bit executing missions with nonzero aircraft. They share the existing normalization stat selector and delta as separate numeric columns, so quiet statistics do not repeat with aircraft-count changes. Navy patrol spotting applies the recorded content factors and existing regional registration order.

WeatherRegions.map_modifiers_known certifies the independently encoded naval_detection, air_attack and air_defence raw entries. A false gate refuses these map terms; a true gate with None proves an absent key. The existing weather accumulator read/carry supplies them without additional acquisition, and quiet sibling columns do not repeat on a numeric change.

`NavalMissionCountries.escort_tree` independently stores the certified escort source preorder for demanded regional ledgers. Missing/refused and proven empty remain distinct. Source leaves use the recorded language; display-language recovery shares the content-backed label index.

CountryCounterInputs adds separate nullable Air combat numeric columns and a complete-census-known scalar, reusing its recorded home-defence and power-projection entries. AceRangeInputs adds separate living-ace own speed/attack/defence raw columns and type raw speed/positive predicates with explicit known flags. Unknown copies and known absent keys remain distinct; quiet columns do not rewrite a combined payload.


NavalPatrolMissionInputs retains independent numeric detection/progress/target columns for distinct regional-ledger members. Existing mission and sibling-range rows supply source context first; omitted-source headquarters membership shares an immutable allocation. The ordered regional ledger preserves repeated displayed progress entries. Country and commander mission modifiers include spotting, visibility and speed entries projected from unchanged complete accumulator reads.

NavalMissionTaskForces also retains independent numeric first-live-target country/count/invasion operands, first convoy-design identity and four raw stat leaves, and demanded source cached speed/signed terrain class. Known no design is variant zero; failed acquisition stays absent. Current fuel usage reuses fuel_requested. Scalar changes do not rewrite stable target or design columns. The directed spotting relation's raw value has its own known predicate in NavalPatrolMissionInputs.

AirMissionInputs retains patrol_disruption_raw and the reached cached attack, defence, default speed and raw wing attack/defence terms as six independent numeric optional columns. Zero disruption and native bypass gates omit the combat subtree; failures never become zero. Quiet cached values do not repeat when disruption changes. The existing asynchronous registered-mission visit supplies them without a new table, scan, hook or allocation.

AirMissionInputs.patrol_night_applied_raw is the seventh independent disruption-related numeric leaf. Ordinary Updated province clocks derive the applied factor from their existing certificate and content. Held clocks require the raw sampled marker: startup-100000 means empty;0..100000 is the factor used by the last completed night accumulator fold. Localhour0 is not a startup certificate under arbitrary mod longitude offsets.

New patrol detection/disruption leaves require a nonzero installed spotting factor. Zero/out-of-array factors leave those leaves absent; failed native factor acquisition leaves demanded nonzero-factor values unknown. Recorded content provides the factor, so no duplicate factor fact is serialized. The oracle resolves the same verified content vector before validating field demand.

The writer uses format 116 and dense shared table registrations. DivisionStatus has 45 fields, with its deployed cache in DivisionPanelStats (22 fields). States uses manpower_locked at 10, best_vp_index at 11 and single_controller at 12. DivisionTemplateIcons uses manpower_exile at 16, base_width at 17, layout at 18, settings at 19 and speed_keys at 20. AirWings uses acquired_manpower at 19, exile_country at 20 and replacement_setting at 21. ResearchModifiers grant kinds occupy 25 through 30 and sharing roster is 31.

Main's format history keeps its numbering, including Ships naming at 107 and
event-feed owner refusals at 109 and production-purpose/control columns at 110.
Recorder changes follow it in their original order; this is history only,
without old alias decoders or translation:

| Format | Recorder change |
|---|---|
| 111 | Exact combat-member scalar and modifier-identity copies. |
| 112 | Exact division-province and production-efficiency copies. |
| 113 | Container-private movement/maxima aliases and exact nested sums. |
| 114 | Combat, province and production alias markers become container-private. |
| 115 | Conditional supply ratio, supply grace and organisation display aliases. |
| 116 | Owner-approved daily commander, combat-observer and division display samples. |

Independently recorded DivisionStatus and CountryCounterInputs width columns retain the existing modifier payload layouts. DivisionStatus's daily display sampling is a separate format-116 meaning change. Historical absence of progression/source suffixes is distinct from a current read refusal; current capture always supplies Value, including Value(None). Retired inputs keep their named framing and do not refuse either host table.

Air and Navy suffixes on weather, ace, country, wing execution, volunteer, fleet and mission source rows retain unrecorded fields separately from recorded false certificates and nullable raw entries. Current projections always publish a value or their existing independent refusal; older prefixes keep named field gaps. The wire kinds and current numeric delta columns are unchanged.

StateResistance.map retains the map marker's demanded cached target, ordered active resistance and compliance definition names, and the actual occupied-country pointer identity demanded by core badge hovers. Native state occurrences precede core occurrences without deduplication. Known non-occupied states carry Value(None); missing evidence remains distinct from empty active lists. Display sorting, percentages, colors and icons resolve through the recorded content stack.

CombatLogObservers uses stable labels for full combat/side, collection kind,
semantic parent and native occurrence index. Conversion reuses only labels from
that encoder's preceding complete baseline, including continuous keyframes;
branches, gaps and refusals restart labels from the complete current sample.
Equal entries remain separate occurrences. Independent full identities and signed numeric columns
avoid identity/amount mixing. `logical_order` and `fact` reconstruct the native
ordered view for dashboard and save comparisons without losing a stored word.

Combat-member scalar copies, modifier division identities, division provinces and
production efficiencies use container-private alias columns. Native/public row
schemas, acquisition, spools and prepared snapshots contain only ordinary values.
Literal exceptions and incomplete independent sources preserve availability.

Divisions.province aliases only unique full-handle membership in a complete
ProvinceUnitOrder.mixed census. Repeated, absent and unequal memberships keep
literals; native occupant order and independent availability are unchanged.

ProductionLines.efficiency_raw aliases only a complete, identity/count-matched
ProductionLineModifiers factory prefix. It uses the native wrapping sum and
signed shifted average, clamped by active and maximum factory counts. Missing
prefixes and unequal values keep literals.

Movement aliases use a container-private marks column, not a public row field.
Path totals alias only exact path lengths; base speed aliases only a present
matching panel statistic. Optional absence, partial counts and literal exceptions
stay independent. The stored count placeholder preserves strategic-action
admission; public readers restore the full count before publication.

DivisionStatus maximum HP and organisation use the same private column. Each
aliases only its complete panel source after the native minimum and declared
`MAXIMUM_STEP` produce the identical recorded value. Missing sources and unequal
values keep literals; the native row fields and their masks are unchanged.

CombatLogObservers state damage, NavalCombatDetails side damage totals and
complete CountryFuel award totals also use private container marks. Exact
wrapping sums retain all attribution, history and recipient children; literal
exceptions and incomplete days preserve independent operands and availability.
Native row schemas, spools and prepared snapshots contain only resolved values.

Conditional display copies also use container-private rule bits. DivisionStatus's
supply ratio and remaining grace reuse their recorded exact operands and declared
display methods; grace joins the complete recorded movement-defines packet.
Divisions' organisation reuses complete same-frame DivisionStatus exact
organisation through `ORGANISATION_STEP`. Optional absence and incomplete sources
leave literal payloads; unequal present values use literal escapes. No public row
field, extra exact capture, availability or sample-origin change is introduced.

`Ships.name` preserves the hourly retained display-cache bytes, including empty,
non-UTF-8 and embedded NUL bytes. `Ships.naming` independently retains signed kind
and order, both raw flag bytes, the complete override byte vector (its encoded
length is the stored length), and nullable full naming-equipment identity. Either
optional acquisition may refuse without dropping core ship values. Format 107
adds columns 9 and 10; ordinary deltas, prepared checkpoints, field sizes and
column inspection share those declarations. The cache is unsaved; saved naming
operands do not certify its freshness.

Military-aid consumers share `military_aid` for alias-held volunteer-list validation, full-handle live resolution, logical-host attribution, signed-positive expeditionary source ownership and repeated-reference signed-wrapping manpower sums. These helpers consume recorded rows only; they introduce no air-volunteer classifier or content defaults.

Owner decision 2026-10-21 (format 116): UnitLeaderTraitExperience is daily slot 12. UnitLeaders.experience, CombatLogObservers.a/b and DivisionStatus.supply/experience/officer_experience use the same daily slot; their rows and all other columns remain hourly. New occurrences read their first values immediately, and attach/load/full refresh/recovery refreshes every value. Off-slot columns retain the last daily sample, not a same-hour observation; hourly identity changes cannot inherit a different referent's sample. Consumers use these held values without requiring hourly freshness. Division supply ratio, exact ratio, grace and exercise eligibility remain hourly; the latter independently demands current XP. Training retains its normal table supplying hour. No other cadence changes.
