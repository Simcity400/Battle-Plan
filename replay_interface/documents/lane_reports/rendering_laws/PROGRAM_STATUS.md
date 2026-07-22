# Rendering-laws RE program status

This is the orchestrator-owned live status surface for
`RENDERING_LAWS_RE_EXECUTION_PLAN.md`. Investigator lanes must not edit it.
Canonical renderer and fidelity documents remain unchanged until a packet has
passed its evidence, verifier, composition, and final-review gates.

## Frozen authority

- Workspace issue tree: current workspace HEAD recorded in
  `replay_interface/re_evidence/rendering_laws/AUTHORITY.json`.
- Replay authority: `C:\Users\Example\battle-plan_main` at
  `6aab56e455b39bdb9f17a38078f59b5a6a8d4755` (read-only for this wave).
  NOTE (2026-07-19): that checkout was a per-wave read-only worktree and is no
  longer on disk; the commit itself is preserved in the pre-consolidation
  bundle (see `documents/GIT_CONSOLIDATION_2026-07-19.md`).
- Program: `hoi4.exe 1.19.2.0.a729`.
- SHA-256: `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.
- Image base: `0x140000000`.
- Broker: read-only, no-analysis, serialized `hoi4_current/hoi4.exe` only.
- Plan SHA-256: `af1f48a8ceed22c017def9323816addfa3199132384ca365d301d6d3e939e0c0`.

## State vocabulary

- `ACTIVE`: owner is investigating and may enqueue bounded broker jobs.
- `QUEUED`: packet is issued but intentionally waiting for capacity or a
  producer contract.
- `EVIDENCE`: report exists and is undergoing mechanical reproduction.
- `CAPTURE`: static and retained-corpus work is exhausted; a discriminating
  capture/live-session card remains.
- `HUMAN_CAPTURE_REQUIRED`: deprecated historical label. Current H-* identifiers
  are stable external-requirement card IDs; the canonical register classifies
  their execution as autonomous unless `user_action_required` explicitly names one.
- `PROVEN`, `PARTIAL`, `REFUTED`, `BLOCKED`: terminal investigator disposition.
- `PROMOTED`: exact law passed composition and the final review wave.

`PROVEN` is not equivalent to `PROMOTED`.

## Current post-automation census (2026-07-22)

- Frozen rendering-law RE register: 51 rows = 51 `PROVEN`; the 50 active reports
  are exact/fail-closed packets. This closes packet-local RE, not the additive
  extractor-input activation gates below.
- Canonical promotion-dependency authority is
  `extractor/documents/PRODUCTION_PROMOTION_HUMAN_INPUT_REGISTER_20260719.json`:
  55 autonomous implementation-ready external-state acquisition cards, zero
  runtime-admitted cards, zero user-action cards, zero unresolved
  RE/engineering cards, zero stale/closed canonical cards, and 12 separate
  product decisions. `H-AIR` is retired outside the canonical set. Packet-local
  Runtime-input gates: zero. Packet-local Implementation gates: zero; the 55
  acquisition cards are receipt-pending activation work, not unresolved packet RE.
- Event/compose external capture closure is `PROVEN`: the synchronized clean
  terrain -> water -> rivers -> trees round includes pass-close payloads and
  RTV/DSV clear provenance, passes all three structural validators, and passes
  the production D3D11 renderer with final target FNV-1a64
  `3b3e6af22757b767`. The 775-artifact outer package manifest is SHA-256
  `09c07387a50df6a8756319c4951f03c4a0b1125cd2e0a1f64c579a1e3e8efceb`.
- Historical human-contract authority:
  `final_integration/human_contracts/remaining_contract_registry_20260717.json`;
  its mechanical verifier left M02 and M03 at the time of that freeze and
  hash-binds their post-capture automatic intake audits. Both contracts are now
  closed: M02 via its validated capture and M03 via the pinned a729 command
  paths plus retained labelled semantic and CPU-to-GPU fixtures. The 07-17
  registry is retained as historical evidence, not current authority. O04
  closed through the
  labelled-Paris 81-object / 243-draw synchronized package; UI13 closed through
  an independently reproduced 341-artifact final producer capture gate.
- First-order integration authority: `final_integration/event_compose/mechanical_result.json`
  SHA-256 `30847a64eccf6b4fdeb0f219becafe491779d89b0d235950d54e96850786200b`;
  `final_integration/event_compose/manifest.json` SHA-256
  `0761192ef700849c15e2b73480fb36a9b913ca4a43f0ea2492073bfe60edf388`.

## Non-UI packet register

| Packet | Owner | State | Primary dependency / next gate |
|---|---|---|---|
| B01 | `cont_b01` | PROVEN | Exact type/window/ramp/writer law passes 1,038/1,038 ordered runs with zero open leaves |
| B02 | `cont_ui04` | PROVEN | Exact two-pass producer and f32 order pass 8,578/8,578 with zero open leaves |
| B03 | `cont_b01` | PROVEN | Full-pool contour construction passes 40,736 exact matches with 452 gains and zero losses; land 27,024/27,024, engine-oriented WaterWater 8,627/8,627, closed loops 928/928 |
| B04 | `cont_ui06` | PROVEN | Promotion gate passes; live worker replay additionally matches 971,204/971,204 outputs across 12,130,762 recorded comparisons with a synchronized RDC |
| B05 | `b05_area_thickness` | PROVEN | Exact semantic contract passes 1,081 states, 13,413 provinces, 92 countries, and scalar gate |
| B06 | `cont_ui05` | PROVEN | Exact pack/upload/cadence and 88-frame carrier census pass with zero open leaves |
| L01 | `l01_moon_z` | PROVEN | Exact height-minus-sun-Y producer passes 126 frames / 1,641 writes with zero open leaves |
| L02 | `l02_shadow_fit` | PROVEN | All producer/consumer laws and 1,715 compositions pass with zero leaves or capture cards |
| L03 | `cont_ui05` | PROVEN | Exact 629-object projection gate has zero residuals; supplemental synchronized capture records 391,608 CPU decisions and its RDC boundary |
| L04 | `l04_exposure` | PROVEN | Exact shader/order law and externalized capture contract pass 198 retained draws |
| L06 | `l06_northern_ice` | PROVEN | Recorded-state contract passes 206/206 over 10,196 records with exact SnowMud producer join |
| C00 | `cont_c00` | PROVEN | CAMERA-ROOT published; 12/12 jobs and 596/596 artifacts verify with zero open leaves |
| C01 | `cont_c00` | PROVEN | 2,820/2,820 records adjudicated; exact applicable laws pass with zero open leaves |
| C02 | `cont_ui07` | PROVEN | Exact temporal law closes 12/12 leaves; deep verifier passes with one live menu-speed consumer |
| C03 | `cont_o05` | PROVEN | Exact wheel/clamp/state/order law passes 740 artifacts with zero promotion gaps |
| C04 | `cont_ui07` | PROVEN | Fixed 5632x2048 base-map atomic state and 14 synchronized terrain GPU joins are exact; non-5632 maps and unauthenticated in-session viewport changes are typed fail-closed expansion regimes |
| T00 | `cont_ui06` | PROVEN | One authenticated consecutive-owner transition, its atomic CPU state, and 14 same-update terrain GPU joins close the transition law for promotion |
| T01 | `t01_terrain_transition` | PROVEN | Exact consecutive-owner composition passes 536/536 masks over 22,528 records; two promotion runs are byte-identical and no capture blocks promotion |
| R01 | `r01_river_vb` | PROVEN | Installed topology regenerates all 580 walks and 5,784,192/5,784,192 VB bytes exactly |
| R02 | `r02_river_runtime` | PROVEN | Promotion gate passes 33/33 with exact helper, ordering, widths, loaders, and fail-closed profile |
| R03 | `r03_strait_mesh` | PROVEN | SAFE, BLANK, and UNKNOWN changed-map launches all classify with valid sequential-quad topology; blank and arbitrary unknown unnamed types are admitted |
| O01 | `resume_ui01` | PROVEN | Exact PRNG seed/key/state/order and conversion pass 3,985/3,985 |
| O02 | `o02_tree_slope` | PROVEN | Exact producer/sampler law; 3,985/3,985 bit-equal slope pairs |
| O03 | removed | PROVEN | Closed by mining: `clamp((350-cameraY)/100,0,1)`, ULP-exact over 1,080 draws |
| O04 | `cont_o06` | PROVEN | Exact baseline/changed streams contain 5,407/5,437 transforms; the labelled-Paris package adds 81 unique CPU objects joined to 243 exact FILE-scaled WorldMatrix draws and the root gate reproduces |
| O05 | `cont_o05` | PROVEN | Promotion verifier passes 50 objects, 150 writers/pixels, 281 semantic objects with zero gaps |
| O06 | `cont_o06` | PROVEN | `rivers.bmp` generator/resource lifecycle passes 2,883,584/2,883,584 bytes |
| N01 | `n01_label_bbox` | PROVEN | Impassable-inclusive bbox is semantic-only; no renderer change |
| N02 | `n02_map_labels` | PROVEN | Exact construction/emission law and three production-port mismatches pass over 91 VBs / 939 glyphs |
| N03 | `n03_label_components` | PROVEN | Exact effective topology passes 241/241 rows, 768/768 cases, and 277 drawable components |
| N04 | `n04_label_alpha` | PROVEN | Exact dual-alpha integrator and selector-submit law pass 6,174 draws with zero unresolved leaves |
| M01 | `m01_3d_unit_models` | PROVEN | Exact 413-object three-pass identity and game-file anchor seams pass twice; A7/A14 are frozen typed live inputs with fail-closed production handling |
| M02 | `m02_combat_indicators` | PROVEN | Closed 2026-07-18 (row corrected 2026-07-19): capture CAPTURE_VALIDATED, all five labelled A-E states proven, portable carrier `m02_combat.rs`, twice-stable 140/140 receipts (`M02.md`; `re_evidence/rendering_laws/M02/production_completion/`) |
| M03 | `cont_ui03` | PROVEN | Pinned a729 command paths plus retained labelled lifecycle, definition, selection, edit-mode, culling, comparator, and CPU-to-GPU fixtures close the exact replay carrier with no operator input remaining |
| M04 | `m04_rails` | PROVEN | Exact anchor/carrier/alpha/update law passes 12,449/12,449 retained rail draws |

## Shared services

| Service | Owner | State | Deliverable |
|---|---|---|---|
| Retained-corpus inventory | `corpus_inventory` | PROVEN | 35/35 packets; 160 RDCs, 87,449 artifacts, 211 issue-bearing components routed |
| Existing-evidence registry | `evidence_registry` | PROVEN | 35/35 packets and 45 durable-job snapshot records reconciled |
| Serialized broker queue | orchestrator | ACTIVE | Exact-a729 20-shard identity is user-authorized and repeatedly verified; active extractor producer jobs remain deduplicated through `ghidra_list_jobs` and every frozen report must retain address/job/artifact/manifest provenance |

Current topology clarification (2026-07-20): the live broker has 20 configured
and ready shards. Historical exact-ten receipts remain frozen evidence records,
but they do not require a ten-shard reset for current work and do not make a
20-shard job non-authoritative. Pool size is operational capacity; job authority
comes from exact identity, mode, job/request/manifest hashes, artifact hashes,
and the retained instruction/dataflow evidence. See
`replay_interface/tools/ghidra/CURRENT_AUTHORITY.md`.

## UI packet register

Packet reports and packet-local findings ledgers hold detailed addresses,
artifacts, hashes, refutations, and incidental discoveries. This master surface
records only state and the single promotion gate.

| Packet | State | Primary dependency / next gate |
|---|---|---|
| UI01 | PROVEN | Exact oscillator/PRNG/event/replay-input law passes the pinned promotion verifier |
| UI02 | PROVEN | Exact event/state/frame/callback/sound law and retained oracle pass the promotion gate |
| UI03 | PROVEN | Exact logical visibility, primary render-list predicate, and descendant traversal law pass the promotion verifier twice with zero open leaves |
| UI04 | PROVEN | Exact QPC binary32 clock, cap, dispatch order, and policy pass all 12 promotion checks |
| UI05 | PROVEN | Object-to-Channel2 pack/upload chain and exhaustive 88-frame census pass with zero open leaves |
| UI06 | PROVEN | Extended and legacy fixed-thumb laws pass the exact retained pixel join |
| UI07 | PROVEN | Exact input/repeat/drag/arbitration law passes all eight verifier checks |
| UI08 | PROVEN | Two parsers agree on 157/157 records across 453 files; no authored scrollbar repeat timer |
| UI09 | PROVEN | Exact focus and authored tech/doctrine laws plus fail-closed admission/rejection for all six unavailable regimes pass the production promotion gate |
| UI10 | PROVEN | 10,888 focus nodes, 438 tech edges, 155 doctrine edges and typed A22/A23 subsets reconcile exactly |
| UI11 | PROVEN | Exact frame/strip/font/glyph/pixel laws pass the pinned promotion verifier |
| UI12 | PROVEN | Exact locator/frame/fallback/cutoff law passes shipped-file and boundary verifier cases |
| UI13 | PROVEN | Final capture records 1,000,000 formatter calls, 842,965 setter calls and all 23 fields; both decision and populated war-loss states pass the deterministic packet gate twice |
| UI14 | PROVEN | Two reversed exact-a729 launches both parse `a_ui14_referee` before `z_ui14_referee` and open z byte `5a`; the prior later-enabled-mod-wins product assumption is mechanically refuted |
| UI15 | PROVEN | Archive staging/collision, staged-root handoff, DLC expansion, and namespace-digest enforcement are implemented fail-closed and pass the production promotion gate |

## Full 1:1 completion scope

The frozen 51-row census above covers the issued rendering-law packets; it is
not a claim that every graphics and UI surface in the executable has been
admitted to a packet. The registers below are the additive completion scope for
a literal 1:1 replay. Every row blocks final 1:1 promotion until it is `PROVEN`
or `PROMOTED`. They do not retroactively change the frozen packet census.

For each `QUEUED` row, the first gate is an exact-a729 evidence audit against
installed content, existing reports, retained RenderDoc captures, and current replay
code. Only the surviving engine-only questions become bounded Ghidra or capture
packets. No visually tuned substitute is an allowed disposition. The extractor
ledger remains independently compiled in `extractor/documents/RE_REMAINING.md`;
its rows are repeated here only as production dependencies of the 1:1 replay.

The following are already closed and must not be reopened: tree camera-height
fade, border-chain direction/texture V, diplomacy fill RGB, and the fact that
the vanilla resource mode submits no province/state fill.

The additive register contains 17 non-UI work packages and 27 UI work packages.
CORRECTION (2026-07-19): the two exact XN16 payloads (`trees_measure`
water+rivers, `FINAL_full` rivers pass-close RTV) no longer block anything —
XN16 closed through the exact-a729 replacement synchronized four-pass package
(production refreeze 12/12 twice); the two shots are retained only as optional
historical scene-parity cards (`XN16.md`).

### Non-UI missing-scope register

| Packet | State | Primary dependency / next gate |
|---|---|---|
| XN01 focus-pause producer | PROVEN | Extractor A2 pins `CCountry+0x1380 -> CNationalFocusProgress+0xB0`; guarded collector, D45, recorder, and replay-fold gates pass |
| XN02 decision-runtime producer | PROVEN | Extractor A12 pins the target union and re-enable/remove cooldown vectors; definition scope joins and fail-closed production gates pass |
| XN03 air-combat lifecycle producer | PROVEN | Extractor A4 is promotion-ready: exact country/region/date/mission history is retained; persistent wing pairing and reinforcement-source enum are proven absent and fail closed |
| XN04 air-loss attribution producer | PROVEN | Extractor A21 is promotion-ready: typed damage causes and the dedicated sunk-convoy history pass the collector, compact-contract, and recorder folds |
| XN05 session-message producer | PROVEN | Extractor A15 is promotion-ready: the pollable d57 player/chat roots and typed chat/system/hotjoin/tag-switch dispositions pass the 27-test lane gate and 8 focused collector tests |
| XN06 building-token registry | PROVEN | Extractor B8/X6 is promotion-ready: the shared 55-entry registry passes 7,456 rows with 43 observed, 12 independently absent, and zero ambiguity or mismatch |
| XN07 extractor integration | PROVEN | A1 hub motorization, A14 repair cause/location, and A22 24-channel air-loss integration pass the final collector/recorder/world-fold verifier; the released collector hash is unchanged |
| XN08 unimplemented selectable map modes | PROVEN | Exact intelligence, supply, resistance, compliance, and raids laws are frozen in `XN08.md`; contract, evidence ledger, hashes, and residual-marker checks pass |
| XN09 residual selectable map-mode laws | PROVEN | Naval three-resource dominance, air 40/60 fill thresholds and neutral/auxiliary branches, infrastructure normalization, and faction color transitions are frozen; the verifier passes and the old naval alpha-floor premise is refuted |
| XN10 map display toggles | PROVEN | Runtime state, ordering, filtering, and composition are frozen in `XN10.md`; the promotion contract passes 67/67 mechanical checks |
| XN11 scripted map-mode layer | PROVEN | Generic scripted-mode registry, evaluation, ordering, runtime inputs, and compositor law are frozen in `XN11.md`; the promotion contract passes 24/24 mechanical checks |
| XN12 particle/effect renderer | PROVEN | XN12A lifecycle/VFS, XN12B timeline/packet submission, and XN12C 198-frame draw/asset correlation are integrated; the final gate passes 54/54 with zero residuals |
| XN13 dynamic unit presentation | PROVEN | XN13 main, producer associations, and ground presentation are integrated with zero unresolved rows; the final hash-sealed gate passes identically twice |
| XN14 weather/atmosphere renderer | PROVEN | XN14A/B, the standalone interface, and XN15-trigger integration are frozen; the final hash-sealed gate passes 71/71 twice with zero residuals |
| XN15 effect-trigger recording | PROVEN | The fail-closed recording contract for XN12-XN14 identity, ordering, lifecycle, palettes, weather membership, and cold seek is frozen and passes 74/74 twice with zero RE residuals |
| XN16 event/compose capture closure | PROVEN | Production package rehashes 775/775 outer and 773/773 inner artifacts; live replay passes 19/19 twice and the production refreeze passes 12/12 twice with final FNV-1a64 `3b3e6af22757b767` |
| XN17 non-UI display-variant parity | PROVEN | H/S/C/D/M/F and XN17F/P production receipts are frozen; the shared supersession gate passes 88/88 twice with historical target-specific rejections retained as non-admission evidence |

### UI missing-scope register

UI01-UI15 prove reusable laws and the two explicit live gates above; they do not
prove population and final composition of all 133 panel files or all 2,009
`containerWindowType` definitions. The in-game replay surface is 105 panel
families, of which the prior coverage census identified 102 as requiring work
beyond static layout. The 25 frontend/meta/dev and three DLC-music families are
included below because literal full-UI parity does not permit the previous
descope.

| Packet | State | Primary dependency / next gate |
|---|---|---|
| XUI01 complete panel census | PROVEN | Effective VFS admits all 454 GUI files and 9,155 windows with zero definition rejection; 190 definition-only runtime dependencies across 77 files are assigned to exact downstream XUI owners; gate passes 44/44 twice |
| XUI02 runtime window manager | PROVEN | Open/close, visibility, modal stack, z-order, focus transfer/restoration, tooltip ownership and input traversal pass 69/69 twice over 330 activation sources, 775 resolved windows and 8,380 explicit dependencies |
| XUI03 dynamic collections | DATA_BLOCKED | Typed consumer covers all 795 constructs and passes focused 4/4 plus twice-stable 35/35. A28 static RE is exhausted and four owner-hook families are implemented fail-closed; activation waits on authentic owner captures/ledgers, while unsupported air/MIO graphs remain unavailable rather than inferred |
| XUI04 widget visual states | PROVEN | Production resolver covers all 49 construct types, 27,135 elements, 18 widget families, and 216 state cells; focused 9/9 and twice-stable 47/47 promotion gates pass with zero residuals |
| XUI05 tooltip compositor | DATA_BLOCKED | All non-data compositor laws pass twice-stable 24/24 gates, and A26's exact final-blob/geometry owner, TTI2 carrier, fold, HUD consumer, assembler, verifier, and centralized validation are complete. Activation waits only on the autonomous H-A26 selected-Present matrix and its refreeze |
| XUI06 text and localization renderer | DATA_BLOCKED | Production renderer/admission passes focused 5/5 plus twice-stable 35/35; A25's exact post-layout producer/consumer RE, RTI1 schema 3, fold and selected-Present assembly are complete and centrally validated. Activation waits on the autonomous mixed text/icon tooltip receipt |
| XUI07 runtime UI graphics | PROVEN | Production law resolves 3,863 assignments across 162 graphics files and all 14 aliases; focused 15/15 and twice-stable promotion gates pass with zero open leaves |
| XUI08 designers and peace conference | DATA_BLOCKED | Replay-owned admission/rendering passes focused 14/14 and verifier 16/16 twice; A40 RE and schema engineering are closed. Activation waits on autonomous six-designer SDI1 observations plus one loss-free peace lifecycle, with terminal allocation identity explicitly unavailable rather than fabricated |
| XUI09 research state | DATA_BLOCKED | Exact A29 ahead-of-time producer/carrier/admission is integrated fail-closed, the replay-owned renderer leaves are closed, and centralized validation passed. Production remains unavailable only until H-A29 authenticates a same-hour branch-1/2 producer receipt; base-factor or save synthesis is forbidden |
| XUI10 national-focus state | DATA_BLOCKED | Replay-owned focus ordering, progress/pause, topology, icons, admission and centralized validation are closed, while the reserved d89 presentation companion remains unavailable until H-A31 authenticates one stable and one discriminating exact-a729 selected-Present frame |
| XUI11 politics detail | DATA_BLOCKED | Production law passes 95/95 twice; A24's exact producer graph, reversible owner, PTI1 carrier, fold, strict replay projection and UI binding are implemented and centrally validated. D73 remains fail-closed only until autonomous H-A24 authenticates the six-family selected-Present matrix |
| XUI12 production and stockpile | PROVEN | Replay-owned D1-D7 seams and completeness laws pass their gates; extractor X4-P1..P6 plus the exact save/rehost deployment-queue carrier, lifecycle and replay path are promotion-ready and centrally validated |
| XUI13 templates and variants | DATA_BLOCKED | The retained single-hour catalog and d78 selected-designer context are consumed, temporal A27 v3 reconstruction is wired fail-closed, and centralized validation passed. H-A27's contiguous three-hour real-mutation receipt remains before historical catalog state is admissible; designer-surface activation/effective-VFS GUI-root closure is a separate recording gate |
| XUI14 intelligence and operatives | DATA_BLOCKED | Replay-owned seven-surface laws and Wave 6 extractor producer/visibility/identity paths are implemented and centrally validated. The only remaining gate is the mechanically graded selected-Present build/injection matrix; no semantic RE or human interpretation remains |
| XUI15 trade state | DATA_BLOCKED | MTRF v2, MTRR, MTRH, and MTRD v2 close the canonical resource-trade path and reject legacy mixed v1/v2 rows. PLIC v2 production-licence manager rows, D53 partner/lifecycle agreement, fold, typed replay, UI cards and centralized validation are wired; the exact current-hour producer agreement remains before the expanded surface is promotion-authoritative |
| XUI16 occupation and garrisons | DATA_BLOCKED | A35's exact garrison/equipment/history/global-alert law, d52 fold, replay binding, production-renderer callsite and centralized validation are wired fail-closed. One autonomous active-detail fixture remains, alongside shared A25 text and A28 collection ordering |
| XUI17 characters, leaders, traits, and aces | DATA_BLOCKED | Replay-owned A39 admission is wired, the early-launch owner joins hashed CPR1/RTI1 evidence to exact selected context/hour, and centralized validation passed. H-A39's autonomous mixed trait, blocked-hover, portrait, and assigned/unassigned-ace fixture remains; A25 text and A28 visible order stay shared upstream dependencies |
| XUI18 power balance | DATA_BLOCKED | Replay-owned laws and A37's exact controller/panel lifecycle, selected-Present materializer, PBP2 fold/replay/UI path and autonomous fixture owner are implemented and centrally validated. Activation remains fail-closed pending the same-transition/open-panel runtime receipt plus shared A25 text; no semantic RE remains |
| XUI19 decisions and missions | DATA_BLOCKED | Replay-owned admission/presentation passes 113/113 twice with a byte-identical 26-artifact manifest; A34's corrected controller graph, DPN1 v2 carrier, selected-Present join, fold/replay/UI and evidence transaction are complete and centrally validated. Activation waits on clean catalog packaging plus the autonomous nonempty/measured-empty matrix |
| XUI20 combat detail and losses | DATA_BLOCKED | Replay-owned land/naval/air loss and region, combat-log-family, D60 and W1A/W1B admission/rendering pass 48/48 twice; malformed data rejects. Static RE and carriers are closed; activation remains split across autonomous H-A33-SUNK and H-X2 loss/width/panel observations |
| XUI21 fired events and popups | DATA_BLOCKED | Replay-owned lifecycle/popup laws pass 41/41 twice, and the common-fire/pending/visible/resolution producer, carrier and completeness telemetry are implemented fail-closed. H-POPUP is the remaining autonomous five-class/four-resolution coverage matrix |
| XUI22 generic scripted-GUI state | DATA_BLOCKED | Replay-owned generic evaluation passes 25/25 twice with a 24-artifact manifest; A38's exact owner/updater/text/sprite hooks, d84 carrier, fold, typed replay and evaluator are implemented and centrally validated. Activation waits only on autonomous representative live materialization |
| XUI23 active abilities | DATA_BLOCKED | Replay-owned ability rendering and A36's exact selected-context producer, active-record/caster trust boundary, ABA2/ABL1 transport and UI admission are implemented and centrally validated. d88 remains fail-closed only until the live transition/multi-leader/measured-empty matrix passes |
| XUI24 mod-added UI discovery | PROVEN | Effective loose/archive/DLC/mod registration, immediate-parent replacement, namespace/DLC ordinals, dynamic discovery and installed scripted-localisation grammar pass 37/37 twice with zero residuals |
| XUI25 display/language/input variants | DATA_BLOCKED | Exact layout, pixel-oracle, ordered glyph-set, resize/clipping, typed launch carrier, input-family observer, manifest-pinned v2 materializer, pre-d77 admission bind and centralized validation are implemented. One autonomous same-launch pointer-movement receipt and the XUI25 authority refreeze remain; keyboard/controller traversal is explicitly unsupported. |
| XUI26 frontend/meta/dev and DLC music | PROVEN | All 28 families and 232 GUI definitions are admitted; 17 replay-owned routes and 9 explicit external-service inputs pass the 338-check gate twice with byte-identical manifest `1391c95e2623c3847242c74216d2c9b9cf391cce1335515ca635a1635a2be38e` |
| XUI27 final UI composition | PROVEN | Replay-owned terminal freeze is independently green with 372 current sources, 26 recursive packet authorities, two byte-identical canonical pass receipts, and zero current replay failures. Its 182 historical drifts and six extractor-boundary rows are explicit non-certifying inventory; this row does not claim those external inputs complete or certify the final extractor-created end-to-end replay |

The legacy seam dedup found two replay-owned integration defects outside the
frozen XUI packet gates: production NAME1b still uses an unconditional all-DLC
predicate instead of authenticated `session.active_dlcs`, and the counter binding
does not yet join several already-recorded supply/dig-in/training/ability/order
inputs. They are current X-DLCLIST-NAME1B and X-COUNTER-BIND seams in
`documents/SEAM_CONTRACTS.md`; the missing exact organization maximum/ratio is
the separate upstream A15-orgmax seam. XUI27's frozen receipt does not close them.

## Review policy

No adversarial review runs during discovery, corpus mining, Ghidra export,
capture preparation, law synthesis, or initial integration. The sole review
wave starts only after all packet dispositions, evidence hashes, focused
verifiers, and cross-packet composition results are frozen.

## Evidence and capture stores (machine-local index, audited 2026-07-19)

**All machine-local evidence now lives under ONE root: `C:\hoi4_evidence\`**
(physically consolidated 2026-07-19; a junction was left at every old path, so
historical references below and in older docs still resolve). The root carries
its own `README.md` manifest describing every subdirectory. Never committed —
size and/or copyrighted game content.

| `C:\hoi4_evidence\` subdir | Role | Old path (junction still valid) |
| --- | --- | --- |
| `renderdoc\` (`mined/` = 175 entries incl. the 38-frame parity-oracle ladder, `session_2026-07-10/12`, `mined_w11c/w13/w20/w23`) | RenderDoc mined-frame corpus — backbone of CORPUS_INVENTORY | `%LOCALAPPDATA%\hoi4_renderdoc` |
| `rendering_law_capture\` (~54.7 GB) | Live RenderDoc capture-session store feeding L03/L05/M02/M03/O04/UI13 | `C:\tmp\hoi4_rendering_law_capture` |
| `capture_corpus_preserved\` (~7 GB: `live_PAN2`, `live_PAN_chunks`, `live_trees_measure`, `live_FINAL_full`, `gate18_final`, `session_WAVE1_CAP4_rounds`) | Preserved live-capture sessions from the retired `hoi4_captures` store | `C:\Users\Example\hoi4_capture_corpus_preserved` |
| `border_pool_preserved\` (+`_backup`) | Border-pool ground truth `session_BORDERZOOM_3` | `C:\Users\Example\hoi4_border_pool_preserved*` |
| `ghidra_agent_exports\` (~1,966 entries) | Ghidra function/DAT evidence manifests cited by packet `ALL_FINDINGS.md` files | `C:\Users\Example\ghidra_agent_exports` |
| `ghidra_proj\`, `ghidra_current\`, `ghidra_re_copy\`, `ghidra_workers_v2\`, `ghidra_identity_870ebf\` | Ghidra projects / serialized program copies (870ebf identity) | `C:\Users\Example\ghidra_hoi4_*`, `C:\tmp\ghidra-hoi4-870ebf-identity` |
| `rederive\` | Per-law re-derivation working dirs | `C:\Users\Example\hoi4_rederive` |
| `lanes\` | Off-repo orchestration state: lane logs, verdicts, wave boards, `FIX_LIST.md` handoff, Codex thread ids (`threads/*.tid`) | `C:\Users\Example\hoi4_lanes` |
| `worktree_uncommitted_backup\` | 2026-07-08 uncommitted-work patch set (7 of 8 subsumed; see GIT_CONSOLIDATION doc) | `C:\Users\Example\hoi4_worktree_uncommitted_backup` |
| `git_history_backup\hoi4-preconsolidation-20260719.bundle` | Full 625-ref pre-consolidation git history (bundle-verified; second copy in `C:\tmp` is expendable) | `C:\Users\Example\hoi4_git_history_backup` |
| `re_scratch_june\` | June 2026 RE scratch (Codex disassembly runs, minidump analysis) | `C:\Users\Example\Downloads\hoi4_2026*` |

**RECOVERED 2026-07-19:** the original `restore_gate_1`
(grading/bloom/political-v2 validation capture) survived under the local Claude
data backup and is now copied byte-exactly to
`<local-path>`. The 243-file tree
and portable admission contract are recorded in
`re_evidence/rendering_laws/RESTORE_GATE_1_RECOVERY_20260719/`; recapturing the
original frame is no longer a human requirement.
