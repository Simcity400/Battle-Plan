# Battle Plan

Record a Hearts of Iron IV campaign, replay it on an interactive map, and explore its statistics over time. The recorder reads game state without controlling the game; the desktop app provides Campaign map and Overview views. Playback reads `.battleplan` files without live game memory.

This is a Windows project for a hash-verified game executable. Start with the [desktop guide](crates/desktop/README.md) to run the app or the [recorder guide](crates/recorder/README.md) to record a campaign. Shared development, validation and merge rules live in [AGENTS.md](AGENTS.md).

## Components

Each component's README describes its layout, concepts and local rules.

| Component | Responsibility |
|---|---|
| [engine](crates/engine/README.md) | Supported executable, native layouts, guarded memory readers and hook site descriptors. |
| [engine-executable](crates/engine-executable/README.md) | Verified executable identity, file-backed image reads and immutable decoded executable metadata. |
| [engine-native](crates/engine-native/README.md) | Native layouts, guarded memory operations, shared containers, authentication and coupled native readers. |
| [engine-governance-tests](crates/engine-governance-tests/README.md) | Synthetic governance worlds and guarded-call traces shared by research and focus tests. |
| [engine-air-history](crates/engine-air-history/README.md) | Native air history readers and their operands, layouts and synthetic tests. |
| [engine-characters](crates/engine-characters/README.md) | Native characters readers and their operands, layouts and synthetic tests. |
| [engine-decisions](crates/engine-decisions/README.md) | Native decisions readers and their operands, layouts and synthetic tests. |
| [engine-deployment](crates/engine-deployment/README.md) | Native deployment readers and their operands, layouts and synthetic tests. |
| [engine-dlc](crates/engine-dlc/README.md) | Native dlc readers and their operands, layouts and synthetic tests. |
| [engine-equipment](crates/engine-equipment/README.md) | Native equipment readers and their operands, layouts and synthetic tests. |
| [engine-fixed-layout](crates/engine-fixed-layout/README.md) | Native fixed layout readers and their operands, layouts and synthetic tests. |
| [engine-fuel](crates/engine-fuel/README.md) | Native fuel readers and their operands, layouts and synthetic tests. |
| [engine-intelligence](crates/engine-intelligence/README.md) | Native intelligence readers and their operands, layouts and synthetic tests. |
| [engine-map-air](crates/engine-map-air/README.md) | Native map air readers and their operands, layouts and synthetic tests. |
| [engine-naval-history](crates/engine-naval-history/README.md) | Native naval history readers and their operands, layouts and synthetic tests. |
| [engine-peace](crates/engine-peace/README.md) | Native peace readers and their operands, layouts and synthetic tests. |
| [engine-power-balance](crates/engine-power-balance/README.md) | Native power balance readers and their operands, layouts and synthetic tests. |
| [engine-projects](crates/engine-projects/README.md) | Native projects readers and their operands, layouts and synthetic tests. |
| [engine-resources](crates/engine-resources/README.md) | Native resources readers and their operands, layouts and synthetic tests. |
| [engine-scripts](crates/engine-scripts/README.md) | Native scripts readers and their operands, layouts and synthetic tests. |
| [engine-stability](crates/engine-stability/README.md) | Native stability readers and their operands, layouts and synthetic tests. |
| [engine-land-inputs](crates/engine-land-inputs/README.md) | Native land inputs readers and their operands, layouts and synthetic tests. |
| [engine-occupation](crates/engine-occupation/README.md) | Native occupation readers and their operands, layouts and synthetic tests. |
| [engine-production](crates/engine-production/README.md) | Native production readers and their operands, layouts and synthetic tests. |
| [engine-raid-inputs](crates/engine-raid-inputs/README.md) | Native raid inputs readers and their operands, layouts and synthetic tests. |
| [engine-railways](crates/engine-railways/README.md) | Native railways readers and their operands, layouts and synthetic tests. |
| [engine-research](crates/engine-research/README.md) | Native research readers and their operands, layouts and synthetic tests. |
| [engine-visibility](crates/engine-visibility/README.md) | Native visibility readers and their operands, layouts and synthetic tests. |
| [engine-events](crates/engine-events/README.md) | Native events readers and their operands, layouts and synthetic tests. |
| [engine-focus](crates/engine-focus/README.md) | Native focus readers and their operands, layouts and synthetic tests. |
| [engine-mutations](crates/engine-mutations/README.md) | Native mutations readers and their operands, layouts and synthetic tests. |
| [collector](crates/collector/README.md) | Injected DLL, native observation hooks, acquisition and recording pipeline. |
| [collector-advisors](crates/collector-advisors/README.md) | Hired and available advisors, advisor tokens and country character records with their held walks. |
| [collector-air-histories](crates/collector-air-histories/README.md) | Air histories and their history observer. |
| [collector-air-records](crates/collector-air-records/README.md) | Regional air combat holders and their record observer. |
| [collector-air-relations](crates/collector-air-relations/README.md) | Air country relations and their relation observer. |
| [collector-air-sorties](crates/collector-air-sorties/README.md) | Air sortie inputs and the air reference witnesses they read. |
| [collector-army-icons](crates/collector-army-icons/README.md) | Army icon inputs from the division sources. |
| [collector-certificates](crates/collector-certificates/README.md) | Writer certificates, dirty marks, owner-slice selection, sampled verification and held research drift. |
| [collector-combat](crates/collector-combat/README.md) | Owned land-combat details, participants, casualties, modifier operands, reinforcement inputs and the reserves' reinforcement-rate source trees, with their native acquisition and replay conversion. |
| [collector-combat-log](crates/collector-combat-log/README.md) | Kept combat log rows of the loaded session and the native log prune observer. |
| [collector-construction](crates/collector-construction/README.md) | Construction rest, buildings and the held building store; building observer. |
| [collector-country-counters](crates/collector-country-counters/README.md) | Country appearance, country counter inputs and the reserved visibility they share. |
| [collector-division-parts](crates/collector-division-parts/README.md) | Per-division input parts: attack stats with the army stat observer, dig-in factors, land inputs, supply inputs, unit combat references. |
| [collector-economy](crates/collector-economy/README.md) | Owned economy and fuel-demand capture, raw staging/decoding and replay projection. |
| [collector-experience-gains](crates/collector-experience-gains/README.md) | Concurrent experience gain observer and its hourly table pack. |
| [collector-extended-support](crates/collector-extended-support/README.md) | Outcome, sizing, hourly-boundary and wall-time distribution contracts the extended capture families share, and their synthetic bench world. |
| [collector-fuel-awards](crates/collector-fuel-awards/README.md) | Applied country fuel update observer and the CountryFuel daily suffix pack. |
| [collector-governance](crates/collector-governance/README.md) | Politics, parties and party leaders, ideas, research, focus, decisions and buildings, with their held stores and party observers. |
| [collector-hooks](crates/collector-hooks/README.md) | Windows x64 patch publication, thread suspension and synchronization, Tap bridges and publication measurements. |
| [collector-intel](crates/collector-intel/README.md) | Country intel, agencies, operations, operatives, script flags and variables, the daily CountryIntel buffer and the flag and variable observers. |
| [collector-leaders](crates/collector-leaders/README.md) | Leader path and leader portrait tables. |
| [collector-ledger-tables](crates/collector-ledger-tables/README.md) | Hourly packs draining the event feed history and the air loss, building bombing and nuclear strike ledgers. |
| [collector-lexical](crates/collector-lexical/README.md) | The game's interned token dictionary, its owned raw copy and conversion. |
| [collector-lifetime](crates/collector-lifetime/README.md) | Native world-reset observation and occurrence admission shared by between-tick captures. |
| [collector-logistics](crates/collector-logistics/README.md) | Concurrent observation of native logistics balance writes and their available-stock operands. |
| [collector-map-tables](crates/collector-map-tables/README.md) | The ordinary map part's map facilities, naval bases and saved event targets. |
| [collector-memory](crates/collector-memory/README.md) | Terminal witness storage, scoped allocation context, checked raw-reserve diagnostics, native read accounting and optional fault injection. |
| [collector-metadata](crates/collector-metadata/README.md) | Prepared native content rosters, recorded identity verification and executable layout reports for capture. |
| [collector-model-inputs](crates/collector-model-inputs/README.md) | Local sub-unit clones and requirements with their writer observer. |
| [collector-naval-counters](crates/collector-naval-counters/README.md) | Task force counter inputs and supply under naval tree certificates. |
| [collector-naval-details](crates/collector-naval-details/README.md) | Naval combat details (with fixtures and scale tests), naval modifier trees, naval result details, naval combat sources. |
| [collector-naval-movement](crates/collector-naval-movement/README.md) | Physical navy census and reserved launch sources (country naval movement inputs). |
| [collector-new-divisions](crates/collector-new-divisions/README.md) | Hourly equipment of divisions minted since the daily walk. |
| [collector-observers](crates/collector-observers/README.md) | Between-tick native writer observers several capture families share, with the forwarders between their patch owners, including the event feed observer, its native guard and four of the DLL's exports. |
| [collector-output](crates/collector-output/README.md) | Production line and construction output observers with their hook sites, their table packs, rocket output and output continuity. |
| [collector-peace](crates/collector-peace/README.md) | Peace conference observer, its table pack, peace bids and the synthetic peace world fixture. |
| [collector-politics](crates/collector-politics/README.md) | Political event pack and its observer. |
| [collector-primitives](crates/collector-primitives/README.md) | Shared typed capture rows and arenas, observed country sizing, observer publication lanes, address-table hashing, raw-memory capacity accounting and session clock reads. |
| [collector-production](crates/collector-production/README.md) | Production and equipment-variant capture, the authenticated variant registry, owned raw copies and replay projection. |
| [collector-production-daily](crates/collector-production-daily/README.md) | Industrial organisations and country and production line modifiers read in the daily production slot. |
| [collector-projection](crates/collector-projection/README.md) | Checked projection and conversion of owned native rows into replay table outcomes. |
| [collector-projects](crates/collector-projects/README.md) | Special projects and their history, raids, raid target statuses and the raid intel carry. |
| [collector-raids](crates/collector-raids/README.md) | Raid launch and raid state inputs read beside their daily slices for the owners a raid writer marked. |
| [collector-railway-guns](crates/collector-railway-guns/README.md) | Railway gun counter inputs, routes, names and ETA operands, with their name sizing hints. |
| [collector-raw](crates/collector-raw/README.md) | Ordered owned raw-copy spans, typed decoding over those bytes, occurrence-aware lookup and prepared copy plans. |
| [collector-session](crates/collector-session/README.md) | Session world tension tables, dynamic modifiers, province names and naval region access published from held reads. |
| [collector-ship-models](crates/collector-ship-models/README.md) | Ship model operands, their held reads and the ship model writer observer. |
| [collector-slow-tables](crates/collector-slow-tables/README.md) | The Slow job's country factories, tactic unlocks, faction theatres, logistics availability and modifier definitions. |
| [collector-snapshots](crates/collector-snapshots/README.md) | Owned dependency traces and prepared read plans that certify unchanged capture results. |
| [collector-stability](crates/collector-stability/README.md) | Country stability inputs and lines read for the countries a writer marked. |
| [collector-status](crates/collector-status/README.md) | Borrowed status objects, sequences, formatting adapters and checked streaming JSON serialization used by capture diagnostics. |
| [collector-supply-trains](crates/collector-supply-trains/README.md) | Supply nodes and train inventory read in parts beside the daily slice. |
| [collector-test-support](crates/collector-test-support/README.md) | The shared synthetic allocation-denial allocator, pre-main test-stack reservation and per-call benchmark helper for collector libraries. |
| [collector-timing](crates/collector-timing/README.md) | Sampled per-thread hook timing, TSC calibration, original-call accounting and borrowed status output. |
| [collector-visibility](crates/collector-visibility/README.md) | Country visibility with the carried raw accumulator entry of an unmarked country. |
| [collector-war-overview](crates/collector-war-overview/README.md) | War owned relations, relation goals, overview countries and war name rules. |
| [collector-weather](crates/collector-weather/README.md) | Province and region weather records with their window-following carry, region combat modifiers and the weather display. |
| [collector-map-events](crates/collector-map-events/README.md) | The simulation edges behind the game's floating map text: callbacks, per-thread publication lanes and the hourly interval. |
| [recorder](crates/recorder/README.md) | Attach/detach, replay finalization, inspection and save comparison. |
| [recorder-platform](crates/recorder-platform/README.md) | Windows process, memory, module and event operations, and installation/user/capture directory defaults. |
| [recorder-test-support](crates/recorder-test-support/README.md) | Shared synthetic replay, spool, admission-load and allocation-failure fixtures used only through dev-dependencies by recorder and its offline tools. |
| [recorder-metrics](crates/recorder-metrics/README.md) | Exact inclusive recording-interval metrics, recorded failure accounting and native-nanosecond percentiles. |
| [recorder-sizes](crates/recorder-sizes/README.md) | Offline recording size attribution: block sections, columns, compression levels and round-trip hour verification. |
| [recorder-replay](crates/recorder-replay/README.md) | Replay recovery, admission records, finalization, append and inspection. |
| [recorder-derived](crates/recorder-derived/README.md) | Offline derivation checks against recorded clocks, temperature passes and attack/power-balance/naval tree witnesses, using the recording's verified content stack. |
| [recorder-movement-oracle](crates/recorder-movement-oracle/README.md) | Read-only native movement census and comparison against the verified content stack's MovementMap. |
| [recorder-oracle](crates/recorder-oracle/README.md) | Exact-hour text-save comparison, authenticated content joins, recorded continuity self-checks and named unsaved limitations. |
| [recorder-oracle-support](crates/recorder-oracle-support/README.md) | Shared save-comparison inputs, exact token/string/country joins and coverage accounting. |
| [recorder-oracle-air](crates/recorder-oracle-air/README.md) | air save comparisons and recorded consistency checks. |
| [recorder-oracle-air-ledger](crates/recorder-oracle-air-ledger/README.md) | air ledger save comparisons and recorded consistency checks. |
| [recorder-oracle-relations](crates/recorder-oracle-relations/README.md) | relations save comparisons and recorded consistency checks. |
| [recorder-oracle-air-missions](crates/recorder-oracle-air-missions/README.md) | air missions save comparisons and recorded consistency checks. |
| [recorder-oracle-intel](crates/recorder-oracle-intel/README.md) | intel save comparisons and recorded consistency checks. |
| [recorder-oracle-characters](crates/recorder-oracle-characters/README.md) | characters save comparisons and recorded consistency checks. |
| [recorder-oracle-combat-details](crates/recorder-oracle-combat-details/README.md) | combat details save comparisons and recorded consistency checks. |
| [recorder-oracle-core](crates/recorder-oracle-core/README.md) | core save comparisons and recorded consistency checks. |
| [recorder-oracle-naval-history](crates/recorder-oracle-naval-history/README.md) | naval history save comparisons and recorded consistency checks. |
| [recorder-oracle-production](crates/recorder-oracle-production/README.md) | production save comparisons and recorded consistency checks. |
| [recorder-oracle-combat-log](crates/recorder-oracle-combat-log/README.md) | combat log save comparisons and recorded consistency checks. |
| [recorder-oracle-construction](crates/recorder-oracle-construction/README.md) | construction save comparisons and recorded consistency checks. |
| [recorder-oracle-counters](crates/recorder-oracle-counters/README.md) | counters save comparisons and recorded consistency checks. |
| [recorder-oracle-country-rosters](crates/recorder-oracle-country-rosters/README.md) | country rosters save comparisons and recorded consistency checks. |
| [recorder-oracle-fuel-demand](crates/recorder-oracle-fuel-demand/README.md) | fuel demand save comparisons and recorded consistency checks. |
| [recorder-oracle-economy](crates/recorder-oracle-economy/README.md) | economy save comparisons and recorded consistency checks. |
| [recorder-oracle-equipment-destruction](crates/recorder-oracle-equipment-destruction/README.md) | equipment destruction save comparisons and recorded consistency checks. |
| [recorder-oracle-event-feed](crates/recorder-oracle-event-feed/README.md) | event feed save comparisons and recorded consistency checks. |
| [recorder-oracle-faction-theatres](crates/recorder-oracle-faction-theatres/README.md) | faction theatres save comparisons and recorded consistency checks. |
| [recorder-oracle-factories](crates/recorder-oracle-factories/README.md) | factories save comparisons and recorded consistency checks. |
| [recorder-oracle-fog](crates/recorder-oracle-fog/README.md) | fog save comparisons and recorded consistency checks. |
| [recorder-oracle-game-score](crates/recorder-oracle-game-score/README.md) | game score save comparisons and recorded consistency checks. |
| [recorder-oracle-governance](crates/recorder-oracle-governance/README.md) | governance save comparisons and recorded consistency checks. |
| [recorder-oracle-military](crates/recorder-oracle-military/README.md) | military save comparisons and recorded consistency checks. |
| [recorder-oracle-land](crates/recorder-oracle-land/README.md) | land save comparisons and recorded consistency checks. |
| [recorder-oracle-map-inputs](crates/recorder-oracle-map-inputs/README.md) | map inputs save comparisons and recorded consistency checks. |
| [recorder-oracle-modifier-breakdowns](crates/recorder-oracle-modifier-breakdowns/README.md) | modifier breakdowns save comparisons and recorded consistency checks. |
| [recorder-oracle-modifiers](crates/recorder-oracle-modifiers/README.md) | modifiers save comparisons and recorded consistency checks. |
| [recorder-oracle-naval-missions](crates/recorder-oracle-naval-missions/README.md) | naval missions save comparisons and recorded consistency checks. |
| [recorder-oracle-naval-result-sources](crates/recorder-oracle-naval-result-sources/README.md) | naval result sources save comparisons and recorded consistency checks. |
| [recorder-oracle-supply-trains](crates/recorder-oracle-supply-trains/README.md) | supply trains save comparisons and recorded consistency checks. |
| [recorder-oracle-pass-inputs](crates/recorder-oracle-pass-inputs/README.md) | pass inputs save comparisons and recorded consistency checks. |
| [recorder-oracle-projects](crates/recorder-oracle-projects/README.md) | projects save comparisons and recorded consistency checks. |
| [recorder-oracle-peace](crates/recorder-oracle-peace/README.md) | peace save comparisons and recorded consistency checks. |
| [recorder-oracle-plans](crates/recorder-oracle-plans/README.md) | plans save comparisons and recorded consistency checks. |
| [recorder-oracle-politics](crates/recorder-oracle-politics/README.md) | politics save comparisons and recorded consistency checks. |
| [recorder-oracle-power-ledger](crates/recorder-oracle-power-ledger/README.md) | power ledger save comparisons and recorded consistency checks. |
| [recorder-oracle-railway-guns](crates/recorder-oracle-railway-guns/README.md) | railway guns save comparisons and recorded consistency checks. |
| [recorder-oracle-rocket-output](crates/recorder-oracle-rocket-output/README.md) | rocket output save comparisons and recorded consistency checks. |
| [recorder-oracle-saved-event-targets](crates/recorder-oracle-saved-event-targets/README.md) | saved event targets save comparisons and recorded consistency checks. |
| [recorder-oracle-selected-routes](crates/recorder-oracle-selected-routes/README.md) | selected routes save comparisons and recorded consistency checks. |
| [recorder-oracle-session](crates/recorder-oracle-session/README.md) | session save comparisons and recorded consistency checks. |
| [recorder-oracle-stability](crates/recorder-oracle-stability/README.md) | stability save comparisons and recorded consistency checks. |
| [recorder-oracle-unit-combat-references](crates/recorder-oracle-unit-combat-references/README.md) | unit combat references save comparisons and recorded consistency checks. |
| [recorder-oracle-visibility](crates/recorder-oracle-visibility/README.md) | visibility save comparisons and recorded consistency checks. |
| [recorder-oracle-wars](crates/recorder-oracle-wars/README.md) | wars save comparisons and recorded consistency checks. |
| [recorder-oracle-weather](crates/recorder-oracle-weather/README.md) | weather save comparisons and recorded consistency checks. |
| [recorder-app](crates/recorder-app/README.md) | Battle Plan Recorder: the tray program that records campaigns while HOI4 runs. |
| [replay](crates/replay/README.md) | Recording format, tables, branches and sample history. |
| [replay-data](crates/replay-data/README.md) | Recorded rows, cadence, exact values, identity and shared data codecs. |
| [store](crates/store/README.md) | Immutable prepared ranges, manifest publication and storage Clear. |
| [installed-content](crates/installed-content/README.md) | Recorded content stack, game assets, shared text parser and identity verification. |
| [installed-content-core](crates/installed-content-core/README.md) | Verified stack, parser/dispatch and shared catalogue/map/asset foundations. |
| [installed-content-localisation](crates/installed-content-localisation/README.md) | Language merge order, substitutions and controls. |
| [installed-content-country-names](crates/installed-content-country-names/README.md) | Recorded country naming and selection rules. |
| [installed-content-unit-path](crates/installed-content-unit-path/README.md) | Land/transfer paths and selected movement text. |
| [installed-content-naval-path](crates/installed-content-naval-path/README.md) | Naval region/province route searches. |
| [installed-content-raids](crates/installed-content-raids/README.md) | Raid types, naming, targets and tooltips. |
| [installed-content-weather](crates/installed-content-weather/README.md) | Weather entities, thermal intervals and temperature content. |
| [installed-content-states](crates/installed-content-states/README.md) | Authored state history and membership. |
| [installed-content-equipment-graphics](crates/installed-content-equipment-graphics/README.md) | Graphics pools, equipment contexts and map/state joins. |
| [installed-content-settings](crates/installed-content-settings/README.md) | Read-only local game/display settings. |
| [installed-content-regions](crates/installed-content-regions/README.md) | Strategic regions and ordered weather periods. |
| [installed-content-division-names](crates/installed-content-division-names/README.md) | Division and raid name groups. |
| [installed-content-events](crates/installed-content-events/README.md) | Event declarations and ordered options/effects. |
| [installed-content-focus](crates/installed-content-focus/README.md) | Focus file order, parse waves and definitions. |
| [installed-content-scripted-localisation](crates/installed-content-scripted-localisation/README.md) | Scripted text alternatives, triggers and random lists. |
| [installed-content-gfx](crates/installed-content-gfx/README.md) | Graphics descriptors and GUI/sprite registration. |
| [installed-content-abilities](crates/installed-content-abilities/README.md) | Commander ability definitions and adjusters. |
| [installed-content-province-area](crates/installed-content-province-area/README.md) | Province area in native map cells. |
| [installed-content-naval-combat-defines](crates/installed-content-naval-combat-defines/README.md) | Typed naval combat define operands. |
| [installed-content-state-categories](crates/installed-content-state-categories/README.md) | State-category identities and modifiers. |
| [installed-content-wargoals](crates/installed-content-wargoals/README.md) | War-goal definitions and naming. |
| [installed-content-save](crates/installed-content-save/README.md) | Text-save readers and serializer projections for save comparisons. |
| [shader-pipeline](crates/shader-pipeline/README.md) | Native shader compiler, persistent cache and GPU pass binding. |
| [viewer-foundation](crates/viewer-foundation/README.md) | Shared map GUI, text, GPU, camera, visibility, engagement and worker contracts. |
| [viewer-combat](crates/viewer-combat/README.md) | Combat windows, bindings, calculations, retained results and hover text. |
| [viewer-raids](crates/viewer-raids/README.md) | Raid offers, reach, ribbon production, hover text and retained icon history. |
| [viewer-counters](crates/viewer-counters/README.md) | Counter grids, GUI lifecycle, grouping and mutable unit selection. |
| [viewer-counter-products](crates/viewer-counter-products/README.md) | Counter/province hovers, route queries, selected movement and battle-plan products. |
| [viewer-world](crates/viewer-world/README.md) | GPU scene, world labels, passes, figures, buildings, political fill and weather. |
| [viewer](crates/viewer/README.md) | Documents, ordered loading, playback, app/corpus composition and image export. |
| [dashboard-core](crates/dashboard-core/README.md) | Dashboard protocol, transport, recording metadata replies, settings, content findings and block scans. |
| [dashboard-text](crates/dashboard-text/README.md) | Trigger and scripted-text interpreter, effect descriptions, name rules, content catalogue and recording metadata helpers. |
| [dashboard](crates/dashboard/README.md) | Widget data products, data service, host/viewer protocol and frontend types. |
| [desktop](crates/desktop/README.md) | Tauri host, application setup, packaging and native viewer lifecycle. |
| [UI](ui/README.md) | React interface, navigation, layouts, playback and widgets. |

The [dashboard architecture](docs/dashboard.md) explains how recordings become data products and widgets.

## UI areas

| Area | Responsibility |
|---|---|
| [Shell](ui/src/shell/) | Navigation, Settings, campaign views, native viewer glue and Tauri API wrappers. |
| [Appearance](ui/src/shell/appearance/) | Themes, backdrops, fonts and appearance preferences. |
| [Shared components](ui/src/components/) | Reusable controls, popovers, tooltips, country choices and flags. |
| [Dashboard framework](ui/src/dashboard/) | Layout storage, shared playback clock, campaign sources, data client and product cache. |
| [Charts](ui/src/charts/README.md) | Canvas charts, tables, axes, formatting and viewport windowing. |
| [Widgets](ui/src/widgets/README.md) | Widget registration and individual widget guides, including the map widget. |
| [Generated types](ui/src/generated/) | TypeScript contracts generated from Rust. |

## Diagnostics

| Area | Responsibility |
|---|---|
| [Foreign-unwind fixture](tools/tap-foreign-unwind/README.md) | Out-of-game C++ and SEH checks for the production Tap. |
| [Widget comparison](ui/e2e/README.md) | Fresh Edge captures of widget view-models and screenshots. |
| [Stack frames](tools/stack-frames.py) | Release collector frame sizes and the hourly hook's stack bound check. |
| [Worker CPU profiles](tools/worker_profile.py) | Symbolize diagnostic collector instruction samples and rank self CPU costs by function, thread role and pool job. |

## Recording and dependency checks

Shared recordings are untrusted input. The [replay reader](crates/replay/README.md)
validates container and compressed-block framing and uses checked growth; the
[dashboard](crates/dashboard/README.md) isolates failed requests. Synthetic
[cargo-fuzz targets](fuzz/README.md) exercise opening, partial trailers, blocks,
frames and recording metadata without a game installation.

Run dependency checks after lockfile/dependency changes and before releases:

```powershell
cargo install cargo-deny --locked
cargo deny check advisories
npm.cmd run audit:ui
```

[deny.toml](deny.toml) uses the RustSec database with no ignored advisories.
Vulnerabilities fail the check; review other advisory notices. The npm check audits
the root lockfile's UI runtime and development dependencies against the registry.
Both require network access for current advisory data. Review fixes and run the
affected checks before updating dependencies; audit results do not validate the
recording reader or replace fuzzing.

## License

Battle Plan is free software under the [MIT License](LICENSE).

The image decoders and archive reader in `crates/installed-content` include modified Rust adaptations of the Independent JPEG Group's libjpeg 9a, libpng 1.6.16 and PhysicsFS. This software is based in part on the work of the Independent JPEG Group. The refresh icon is from Lucide. Their notices are in [LICENSES](LICENSES/).

Battle Plan is an unofficial project and is not affiliated with or endorsed by Paradox Interactive. Hearts of Iron IV is a trademark of Paradox Interactive AB. No installed content are included; the app reads them from your own installation.
