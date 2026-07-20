# XUI10 — national-focus runtime state and presentation admission

Status: **D89_PRODUCTION_ROUTE_IMPLEMENTED; LIVE_ACTIVATION_FALSE**. Every
A31 field recoverable from the current live roots, Ghidra, installed content, and
retained focus captures now has an exact producer or final engine field, and
the replay-owned presentation boundary has a complete deterministic gate. The
exact20-bound owner/item producer, reserved d89 NFP1 codec, recorder fold,
typed replay view, and exact XUI10 admission are now wired; scheduling remains
false pending the authenticated live fixture and root central validation.

## Authority and scope

The result is bound to `hoi4.exe` 1.19.2.0.a729, image base `0x140000000`, and
SHA-256 `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.
This lane joined the frozen UI10 game-file topology, UI09 retained focus canvas,
the extractor focus readers, and the production replay layer. After Ghidra was
authorized, `ghidra_status` confirmed the exact program, base, executable hash,
and authorized 20-shard pool before address work. Existing jobs were inspected
before every new producer batch. `PROGRAM_STATUS` was not updated.

## Exact closed laws

UI10's vanilla-only parser proves 67 focus trees and 10,888 unique definitions:
10,273 local, 335 shared, and 280 joint nodes. The graph contains 27,155 typed
edges with no unresolved imports, unresolved endpoints, or collisions. This is
the static catalog and effective-topology authority.

The three retained 2752x1152 focus frames, 38044, 38304, and 38562, each expose
the same 8,376-draw canvas under scissor `[10,138,2727,1128]`. Its exact layer
counts are 6,985 link tiles, 1,159 node/label draws, 184 joint overlays, and 48
auxiliary overlays. The draw sequence is invariant under the two authenticated
affine comparisons. UI09 remains the authority for producer-resolved positions,
recorded transform, clip, draw order, and runtime text.

The capture join also closes the observed glow state exactly. The canvas has
one and only one `423x115` three-frame resource draw: chunk `120573`, event
`115163`, resource `466852`, drawn as the first `141x115` atlas frame. Its
captured 194,580-byte GPU payload hashes to
`69f4da764cede73f0f0f44defd3c896ec8aac9a25f7b275b2acf44dd77c8a933`,
which byte-matches `gfx/interface/ongoing_focus_goal.dds` after its 128-byte
DDS header. It differs from `search_current_focus.dds`, whose payload hashes
to `705a8b5760dd11252d2d0880bdab071c09ecce0eecc6b23a3e99869a0c056848`.
All 8,376 canvas payloads retain identical order and bytes across frames 38044,
38304, and 38562. Therefore ongoing glow is present and highlight glow absent
in all three retained focus frames. This is an exact observed-state result; it
does not manufacture the general runtime predicates.

`nationalfocusview.gui` authors a `165x128` focus item, `(96,130)` spacing,
`hoi_16mbs` titles, and `GFX_goal_unknown` as the missing-icon fallback. An
authored nonempty icon is retained exactly; an absent icon uses that exact
fallback; an authored empty icon rejects.

The FILE layer also closes every semantic-state background. The effective
`common/national_focus/00_titlebar_styles.txt` contains exactly 22 style rows:
one default and 21 joint-focus families. Every row supplies exact unavailable,
available, current, and completed sprite tokens; all 88 tokens exist in
`interface/nationalfocusview.gfx`. All 280 joint focuses carry a `text_icon`
style reference, all 10,888 nodes resolve to one style, and there are zero
incomplete or unresolved style rows. The focus-definition parser now preserves
the node kind, `text_icon`, style identity, and all four resolved sprite tokens.

The same parser closes the optional authored overlay exactly. Of 10,888 nodes,
93 carry one of 25 nonempty overlay sprite tokens and the other 10,795 are
authored-empty. Every token resolves in the frozen `interface/goals.gfx` or
`interface/right_vs_left_campaign.gfx` registry. The production presentation
retains that exact optional token and rejects an authored empty string.

The focus transport now preserves pause in the compact D12 record. Bit 6 means
pause is present and bit 7 is its Boolean value; bit 7 without bit 6 rejects.
Bits 2 and 3 now separately authenticate successful reads of the national and
continuous current pointers. An empty compact token plus its read bit therefore
produces exact measured `[]`; absence of the bit remains unavailable. Nonempty
tokens without read bits reject. Production compact collection now reads and
carries the complete ordered focus vector instead of deliberately omitting it;
the recorder rejects list-presence, token-country, empty-token, and count/list
inconsistency. Legacy nullable current tokens remain compatibility leaves.

## Production boundary

`game_ui::national_focus::admit_national_focus_presentation` preserves the
legacy D12/D45 state contract. The production exact entrypoint,
`game_ui::national_focus::admit_exact_national_focus_presentation`, additionally
requires the hour-bound d89 typed view and accepts only a
measured focus-tree identity, measured exact current/continuous sets, exact
completed sequence and count, engine-evaluated available set, final visible
sequence, pause, and conditionally measured progress days. It mechanically
enforces:

- current and continuous sets each contain zero or one unique ID and are
  mutually exclusive;
- completed count equals sequence length, and completed nodes are disjoint from
  current and available nodes;
- visible/current IDs belong to the selected effective topology, while
  available IDs belong to the exact visible set;
- every displayed or completed ID has a catalog definition;
- producer visible order equals the admitted UI09 focus-canvas node sequence;
- every visible title has a runtime localized-text witness;
- progress days are finite and nonnegative and exist only when a focus is
  active; and
- every standard node uses the exact default state sprite, while every joint
  node requires its exact FILE-resolved style family; and
- every node retains its exact optional FILE-authored overlay token; and
- the underlying UI09 focus canvas independently admits.

The resulting exact sequence assigns semantic `Completed`, `Current`, `Available`, or
`Unavailable` state plus the exact FILE-resolved background and optional
overlay sprites. It does not invent availability, progress fraction, marker
visibility, draw position, or ordering. D89 supplies the exact display percent,
continuous/highlight/historical visibility, and measured joint-view country;
missing or malformed d89 rejects rather than using legacy inference.

## Exact upstream producer contract

`extractor/documents/dossiers/A31_NATIONAL_FOCUS_PRESENTATION_INPUTS.json` and
`runtime_producer_laws.json` freeze the promotion-ready runtime reads:

- final visible order is the pointer sequence at view `+0x2058`, count
  `+0x2064`, after `0x14136f750`; each item identifies its focus through
  item `+0x20`;
- `0x14137a780` updates every visible item through `0x141379a50`; the final
  state code is item `+0x540`: `0` unavailable/other, `1` completed, `2`
  selectable, and `3` active/current (including visible joint progress);
- the exact available set is therefore the visible items whose final state is
  `2`; the standard `continuous_glow` is shown exactly for state `3`;
- `highlight_glow` is not the active-focus marker. `0x141377430` shows it only
  for search highlight state `3`; reset is `0`, ordinary matches are `2`, and
  the selected search-result index is stored at view `+0x2278`;
- the `historical` widget is shown exactly when the final engine byte at focus
  definition `+0x5b8` is nonzero. The extractor can read this byte directly;
  active AI strategy-plan identity is not required;
- `viewing_flag` and `viewing_flag_border` are shown only when the joint-focus
  helper resolves a positive country identity; exact zero hides both; and
- `0x141bb5df0` produces the final integer progress percentage. It emits `0`
  for not started, `100` for completed, and performs the in-progress fixed-point
  integer divisions before the final normalization. The extractor must capture
  that final integer or every same-tick operand.

Static trigger presence, the live non-completed candidate vector, base focus
cost times seven, authored order, coordinate sorting, and strategy-plan filename
inference remain forbidden substitutes.

## Incidental findings retained

- Pause was already read by the live focus collector but was dropped from the
  compact D12 replay payload; the transport fix closes that loss.
- The raw pointer reads already distinguished no current focus from read
  failure. D12 was discarding that distinction; bits 2 and 3 now preserve it.
- Scheduled compact focus collection requested `compact=true`, which omitted
  the completed vector even though D12 already supported it. It now requests
  the exact vector and the recorder verifies count equality.
- The historical focus-submanager `+0x78` candidate vector is merely
  non-completed. Availability is instead the final item state code `2`.
- The successful rebuild trace at `0x141371b70` reads the focus-tree root from
  game `+0x1370`, walks the root `+0x98` vector using count `+0xa4` and stride
  `0x38`, calls `0x1413637c0` per row, then calls tree builder `0x14136f750`
  and zoom/pan updater `0x141377e00`. The earlier visible/filter label for
  `0x141377e00` is refuted.
- `0x1413637c0` is a `0x68`-byte row-record allocator/initializer, not a style
  selection predicate.
- `0x14136f750` filters focuses through `0x1402d5c00`, sorts the retained
  records through `0x141366ca0`/`0x1413677f0`, creates
  `CNationalFocusTreeItem` objects through `0x14136a5c0`, and publishes their
  final order at view `+0x2058`/`+0x2064`.
- The successful node-maintenance trace at `0x14137a600` keeps separate
  national (`+0x20a0`, count `+0x20ac`) and joint (`+0x20b8`, count `+0x20c4`)
  vectors, with distinct liveness and update callees. `0x141bb4710` and
  `0x141bb46f0` are exact GUI-liveness reads of bit 3 at GUI object `+0xa5`.
- Completed history can retain IDs outside the currently selected tree. The
  consumer retains such history when the catalog contains the definition and
  does not incorrectly require current-tree membership.
- Static files, not capture inference, close all 22 state-background families.
  Ghidra now supplies the general availability, visible-order, progress, and
  marker producers that the captures alone could not supply.
- Static files also close all 93 authored overlay uses and their 25 exact sprite
  tokens; this field is no longer an A31 runtime producer leaf.
- Retained GPU bytes close the observed continuous/highlight state across all
  three focus captures: continuous is drawn and highlight is not. The general
  predicates are now independently proven by `0x1413778f0` and `0x141377430`.
- `common/ai_strategy_plans/GER_historical_strategy_plan.txt` (SHA-256
  `71dcda22e97db90d9901df7865b33d888b849e3e5f079dafc4ca7de3a4250f24`)
  supplies the exact ordered 75-focus `German_historical` list. Its
  `allowed`, `enable`, and `abort` blocks prove that active-plan selection is
  runtime-evaluated. The list is useful FILE data, but it cannot substitute
  for the final engine marker byte at focus `+0x5b8`.
- The 75 `German_historical` plan entries are not the same dataset as the 81
  German focus nodes carrying the authored historical search filter. A new
  complete FILE registry preserves the distinction: 128 effective files,
  350 plans, 12,868 ordered references, 6,126 unique known focus IDs, zero
  unresolved references, and zero plan-ID collisions.
- All 10,888 nodes have authored base cost, while shipped decisions explicitly
  reduce focus time by 35 or 70 days. `cost*7` is therefore not an effective
  display-progress law.
- `0x141baefc0` toggles three glow-related widgets together for another focus
  presentation object. `0x141baf450` and `0x141bad5c0` refer to unrelated
  `focus_progress`-named widgets; none is the national-focus tree percentage
  producer, which is `0x141bb5df0`.

## Mechanical verification

The replay-only verifier now freezes the d89 exact-admission route and requires
9 XUI10 admission tests plus the 11 UI09 canvas/replay-adapter admission tests. The
actual `replay_core` typed-view gate also covers d89 hour binding, ordering, and
fail-closed rejection. This wave has not run those gates locally: the root
agent owns the single centralized Rust validation run after all parallel edits
have landed.

The verifier independently parses all 81 frozen game-definition sources and
reproduces the 10,888-node kind census, all 22 complete background families,
all 280 joint style joins, and the exact 93-node/25-token overlay census. It
rehashes the executable, frozen UI10/UI09 artifacts, shipped GUI/GFX inputs,
replay production items, test items, runtime laws, and complete Ghidra
provenance. Shared Rust files are pinned by the exact `FocusView` and
`rt_research_focus` item bodies, so unrelated concurrent lanes cannot perturb
the XUI10 gate while any XUI10-relevant change still fails closed. The final
two verifier runs are byte-identical. No extractor file is read and no
extractor command is executed by this replay promotion gate.

## Ghidra provenance

Every relied-on artifact is also machine-readable in
`runtime_producer_laws.json`. Paths below are absolute and every row records the
function address, job ID, artifact, artifact hash, and manifest hash.

| Function | Job ID | Artifact path | Artifact SHA-256 | Manifest hash |
|---|---|---|---|---|
| `0x141371b70` | `20260716T022035Z-ui04-view-delta-consumers-and-neighbours-04c34999` | `C:\Users\Example\ghidra_agent_exports\20260716T022035Z-ui04-view-delta-consumers-and-neighbours-04c34999\evidence\func_141371b70.decompile.c` | `daa73a9544572e330cacd68af3c9180af1d00e91dddc85d1243a57e439aa0f5e` | `34dc1550021dcb9f3de2894fff397998a14299c1c8415d9e23fb7a8f1b6803fb` |
| `0x14137a600` | `20260716T022035Z-ui04-view-delta-consumers-and-neighbours-04c34999` | `C:\Users\Example\ghidra_agent_exports\20260716T022035Z-ui04-view-delta-consumers-and-neighbours-04c34999\evidence\func_14137a600.instructions.txt` | `983cdac02fa4ed5422fa9dff55dce9bfdd8e377bd642f271b3799f1351261293` | `34dc1550021dcb9f3de2894fff397998a14299c1c8415d9e23fb7a8f1b6803fb` |
| `0x14136f750` | `20260717T140546Z-xui10-a31-focus-tree-build-producer-v10--f2667cc5` | `C:\Users\Example\ghidra_agent_exports\20260717T140546Z-xui10-a31-focus-tree-build-producer-v10--f2667cc5\evidence\func_14136f750.decompile.c` | `4616a08b70a2e8c9e1b340aab31171d612f937baf9e8ccfb716f90381c812503` | `4c1dbb28aa55b7d2c70f7b2841ad8d1eb4b3fb2ec7c0eeedae78bac58b3bf9ae` |
| `0x1402d5c00` | `20260717T140546Z-xui10-a31-focus-tree-build-producer-v10--f2667cc5` | `C:\Users\Example\ghidra_agent_exports\20260717T140546Z-xui10-a31-focus-tree-build-producer-v10--f2667cc5\evidence\func_1402d5c00.decompile.c` | `0dffcb3ffce5a4b7076a419197e8a4c13a74ae8158af6bcd824d2b000ba54ca1` | `4c1dbb28aa55b7d2c70f7b2841ad8d1eb4b3fb2ec7c0eeedae78bac58b3bf9ae` |
| `0x14136a5c0` | `20260717T140546Z-xui10-a31-focus-tree-build-producer-v10--f2667cc5` | `C:\Users\Example\ghidra_agent_exports\20260717T140546Z-xui10-a31-focus-tree-build-producer-v10--f2667cc5\evidence\func_14136a5c0.decompile.c` | `9edb0001d9470433def5444378afab4d75a5a8374dc6e9cff1e0258e86175903` | `4c1dbb28aa55b7d2c70f7b2841ad8d1eb4b3fb2ec7c0eeedae78bac58b3bf9ae` |
| `0x141366ca0` | `20260717T140546Z-xui10-a31-focus-tree-build-producer-v10--f2667cc5` | `C:\Users\Example\ghidra_agent_exports\20260717T140546Z-xui10-a31-focus-tree-build-producer-v10--f2667cc5\evidence\func_141366ca0.decompile.c` | `d89e5253a309854c393cbeb8efe27fa8011f1aba6dffe226143260d1b26f06de` | `4c1dbb28aa55b7d2c70f7b2841ad8d1eb4b3fb2ec7c0eeedae78bac58b3bf9ae` |
| `0x1413677f0` | `20260717T140546Z-xui10-a31-focus-tree-build-producer-v10--f2667cc5` | `C:\Users\Example\ghidra_agent_exports\20260717T140546Z-xui10-a31-focus-tree-build-producer-v10--f2667cc5\evidence\func_1413677f0.decompile.c` | `a0d8ed16a2e7db523ab78ed8fab63181f34b5f1d2c0c71548821f362559b4fbf` | `4c1dbb28aa55b7d2c70f7b2841ad8d1eb4b3fb2ec7c0eeedae78bac58b3bf9ae` |
| `0x141377e00` | `20260717T140609Z-xui10-a31-visible-order-filter-producer--3dd06c07` | `C:\Users\Example\ghidra_agent_exports\20260717T140609Z-xui10-a31-visible-order-filter-producer--3dd06c07\evidence\func_141377e00.decompile.c` | `8f9cdd2997da5909b9cb4a4f7a9015516d94d59613657b101c153a5f75797be1` | `d83bbe9e8b8b163b95e81353a81ed0ddb14c9654154b831adae93f6db52e9f0a` |
| `0x1413637c0` | `20260717T140631Z-xui10-a31-tree-style-row-producer-v10-v2-cbcddc8d` | `C:\Users\Example\ghidra_agent_exports\20260717T140631Z-xui10-a31-tree-style-row-producer-v10-v2-cbcddc8d\evidence\func_1413637c0.decompile.c` | `042cb19f6175a10fb01b220bb30a84f26151cb0c828abd5a2d706c7eb1d1311c` | `8f1e7060c21b24a6e4cb2d973eeb43a47151d761b090774b46450bb0c15b152a` |
| `0x141377430` | `20260717T141607Z-xui10-a31-focus-widget-state-producers-v-8cd49044` | `C:\Users\Example\ghidra_agent_exports\20260717T141607Z-xui10-a31-focus-widget-state-producers-v-8cd49044\evidence\func_141377430.decompile.c` | `91ee4d8c9450d2f878e92a3be9b2e81c8fbe45e07551309f98f4754ec71b8604` | `fdcea426c9fb4f09f1552cce50941b263f8c17f7b959e5a13349fdb6530a8d18` |
| `0x1413778f0` | `20260717T141607Z-xui10-a31-focus-widget-state-producers-v-8cd49044` | `C:\Users\Example\ghidra_agent_exports\20260717T141607Z-xui10-a31-focus-widget-state-producers-v-8cd49044\evidence\func_1413778f0.decompile.c` | `024ac349f5877ffc874681187b516d4f35fd13ad59046ff76fb01b4fe958d306` | `fdcea426c9fb4f09f1552cce50941b263f8c17f7b959e5a13349fdb6530a8d18` |
| `0x141379a50` | `20260717T141607Z-xui10-a31-focus-widget-state-producers-v-8cd49044` | `C:\Users\Example\ghidra_agent_exports\20260717T141607Z-xui10-a31-focus-widget-state-producers-v-8cd49044\evidence\func_141379a50.decompile.c` | `e879062a0b8bc1bfa8b26e13e5c5a64d880f0df0cca3dba0bd387ef1030f28ae` | `fdcea426c9fb4f09f1552cce50941b263f8c17f7b959e5a13349fdb6530a8d18` |
| `0x14137a780` | `20260717T141607Z-xui10-a31-focus-widget-state-producers-v-8cd49044` | `C:\Users\Example\ghidra_agent_exports\20260717T141607Z-xui10-a31-focus-widget-state-producers-v-8cd49044\evidence\func_14137a780.decompile.c` | `23eedf3959f15506098f16044933d64eb103c8a165da83d49c70c2d16198bb22` | `fdcea426c9fb4f09f1552cce50941b263f8c17f7b959e5a13349fdb6530a8d18` |
| `0x14136dc20` | `20260717T141607Z-xui10-a31-focus-widget-state-producers-v-8cd49044` | `C:\Users\Example\ghidra_agent_exports\20260717T141607Z-xui10-a31-focus-widget-state-producers-v-8cd49044\evidence\func_14136dc20.decompile.c` | `8d91b8778b4b333922e9a80cbff6f0dbb5a4e6c947acec579299caf352141f5a` | `fdcea426c9fb4f09f1552cce50941b263f8c17f7b959e5a13349fdb6530a8d18` |
| `0x141bb5df0` | `20260717T140613Z-xui10-a31-national-node-update-producer--bc33e96f` | `C:\Users\Example\ghidra_agent_exports\20260717T140613Z-xui10-a31-national-node-update-producer--bc33e96f\evidence\func_141bb5df0.decompile.c` | `dad133942c62e1d4daeca845ee6802b245e001ce23c233da1a61814609fdf707` | `9cccf4260940425b45352fd5e0def288b7d9be6391b64dce4b9629c25969e14e` |
| `0x141bb5a70` | `20260717T140618Z-xui10-a31-joint-node-update-producer-v10-90d73a26` | `C:\Users\Example\ghidra_agent_exports\20260717T140618Z-xui10-a31-joint-node-update-producer-v10-90d73a26\evidence\func_141bb5a70.decompile.c` | `7134bd9ae2543e184317dc75cb77de5f60996fa3cab55488f4306f0c9c8491a1` | `f652049e59a535a9aa177d5053a3fec7d9e7e1f367fdb7acff1e06bba4a97280` |
| `0x141bb4710` | `20260717T140623Z-xui10-a31-national-node-liveness-v10-v2-c0e52aff` | `C:\Users\Example\ghidra_agent_exports\20260717T140623Z-xui10-a31-national-node-liveness-v10-v2-c0e52aff\evidence\func_141bb4710.decompile.c` | `7add2b0dcfa42917365fade41975c262b321e62c4aed45b0ef06eb949c613586` | `c8eda576ff809e7276b9e4c14ce6cfe6efa6db85a686530a0cd2690d9a9ec84b` |
| `0x141bb46f0` | `20260717T140627Z-xui10-a31-joint-node-liveness-v10-v2-69a8ba83` | `C:\Users\Example\ghidra_agent_exports\20260717T140627Z-xui10-a31-joint-node-liveness-v10-v2-69a8ba83\evidence\func_141bb46f0.decompile.c` | `db9e1b00ce713657ca8866e2784ae20f95a23a9efadb0499d40a6b6f9f3d7e2e` | `a16e21cadf233daf1e01a1149719daa6e886db1ae2942e140b1b205b1811a2fe` |

The widget-name xrefs are job
`20260717T141230Z-xui10-a31-focus-widget-binding-names-v1-ac86e60c`, artifact
`C:\Users\Example\ghidra_agent_exports\20260717T141230Z-xui10-a31-focus-widget-binding-names-v1-ac86e60c\string-xrefs.txt`, artifact SHA-256
`38821cc63ebd66700a3660cfb09330788301f7a177797665c88e93547405126d`,
identity-manifest hash
`cd33527ef2f8f5d73ac87272d13b3c6219927272875ffe379ec7f4e1f4f8f8fe`.
There are no remaining exact producer addresses in A31.

## Evidence

- `replay_interface/re_evidence/rendering_laws/XUI10/production_contract.json`
- `replay_interface/re_evidence/rendering_laws/XUI10/capture_marker_audit.json`
- `replay_interface/re_evidence/rendering_laws/XUI10/historical_plan_registry.json`
- `replay_interface/re_evidence/rendering_laws/XUI10/build_historical_plan_registry.py`
- `replay_interface/re_evidence/rendering_laws/XUI10/runtime_producer_laws.json`
- `replay_interface/re_evidence/rendering_laws/XUI10/verification.json`
- `replay_interface/re_evidence/rendering_laws/XUI10/evidence_manifest_replay.json`
  (the manifest recursively binds this report, so its own hash is intentionally
  not embedded here and is recorded by the parent XUI27 freeze)
- `replay_interface/re_evidence/rendering_laws/XUI10/verify_xui10.py`
