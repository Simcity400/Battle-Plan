# XUI02 — runtime window manager

Status: **PROVEN REPLAY-OWNED MANAGER LAW; OMITTED FROM CURRENT PRODUCT ROUTE**

Authority: `hoi4.exe 1.19.2.0.a729`, image base `0x140000000`, SHA-256 `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.

The older v2 activation-admission contract is retained below for provenance.

## Promotion result (v3)

XUI02 now owns replay view state instead of waiting for captured private-client UI
state. `ReplayOwnedWindowManager` registers all **9,155 / 9,155** admitted XUI01
window identities and starts them dormant. Every identity participates in the same
deterministic open/close, stack, modal, focus, tooltip, and traversal laws; there is no
special-case subset and no extractor dependency for these view-state transitions.

The executable laws are:

- `open` is equivalent to visible, composed, and present exactly once in the stack;
- open panels receive unique contiguous `u32` ordinals in back-to-front order;
- `bring_to_front` removes and appends exactly one open panel, then renumbers the stack;
- the topmost modal exclusively owns input; its composed owner cannot close while the
  modal is open, and background focus/stack/tooltip commands reject;
- opening a panel records prior focus; closing the input owner restores that target only
  if its panel remains open and the widget remains in an explicit traversal list,
  otherwise the new input owner's first explicit target is selected;
- keyboard and controller keep separate caller-supplied, unique nonblank orders and
  traverse cyclically in both directions;
- only the current input panel may own a resolved-tooltip record, and only for a
  widget in that panel's explicit tooltip-target set; open, close, focus, traversal,
  or z-order transfer clears stale tooltip ownership.

These are explicitly replay-product laws. They do not claim that remote players' private
the game windows were captured. UI03 remains authority for the engine visibility/draw gate,
UI07 remains authority for pointer arbitration, and the tooltip compositor remains
authority for tooltip timing, placement, and draw order.

The complete law remains executable in `game_ui::window_manager_law`, but normal
`replay_app` now constructs only `ProductUiRuntime::from_authenticated_hud_roles`.
It does not construct `ReplayOwnedWindowManager`, so XUI02 panels are omitted from
the current product until an admitted panel/window-manager route exists. The focused
manager-law suite passes **17/17**. The full `game_ui` library gate passes with zero
failures, and `cargo check -p replay_app` passes.

### New Ghidra pass and retained negative findings

The string pass tested whether stripped binary names could supply a stronger private-
engine modal/focus/stack/traversal law. They could not. The one concrete `TopMost` xref
was exported and refuted: `0x140D5D7D0` is `taskforce.cpp` parent/child attachment
logic using `pTopMostFleet`, not GUI stacking. The manager policy is therefore labeled
replay-owned rather than misrepresented as recovered engine state.

| Function address(es) | Job ID | Artifact path | Manifest SHA-256 | Result |
|---|---|---|---|---|
| none | `20260717T231328Z-xui02-runtime-manager-modal-anchors-v1-e23b1a1a` | `C:\Users\Example\ghidra_agent_exports\20260717T231328Z-xui02-runtime-manager-modal-anchors-v1-e23b1a1a\string-xrefs.txt` | `cd33527ef2f8f5d73ac87272d13b3c6219927272875ffe379ec7f4e1f4f8f8fe` | all five modal-name anchors absent |
| `0x140D5D7D0` | `20260717T231334Z-xui02-runtime-manager-zstack-anchors-v1-31b9e667` | `C:\Users\Example\ghidra_agent_exports\20260717T231334Z-xui02-runtime-manager-zstack-anchors-v1-31b9e667\string-xrefs.txt` | `cd33527ef2f8f5d73ac87272d13b3c6219927272875ffe379ec7f4e1f4f8f8fe` | only `TopMost` xref; other z-stack names absent |
| none | `20260717T231339Z-xui02-runtime-manager-focus-anchors-v1-8c242d5b` | `C:\Users\Example\ghidra_agent_exports\20260717T231339Z-xui02-runtime-manager-focus-anchors-v1-8c242d5b\string-xrefs.txt` | `cd33527ef2f8f5d73ac87272d13b3c6219927272875ffe379ec7f4e1f4f8f8fe` | `SetFocus`/`GetFocus` have no code xrefs; remaining anchors absent |
| none | `20260717T231343Z-xui02-runtime-manager-tooltip-owner-anch-bcda8bbc` | `C:\Users\Example\ghidra_agent_exports\20260717T231343Z-xui02-runtime-manager-tooltip-owner-anch-bcda8bbc\string-xrefs.txt` | `cd33527ef2f8f5d73ac87272d13b3c6219927272875ffe379ec7f4e1f4f8f8fe` | no tooltip-owner symbols; three unreferenced `SetTooltip` occurrences retained |
| `0x140D96CB0`, `0x140DF0A40`, `0x141A83A10`, `0x141A84750`, `0x1420CE1E0`, `0x141A83820`, `0x141EBFD40`, `0x141FFD290` | `20260717T231349Z-xui02-runtime-manager-input-traversal-an-46267615` | `C:\Users\Example\ghidra_agent_exports\20260717T231349Z-xui02-runtime-manager-input-traversal-an-46267615\string-xrefs.txt` | `cd33527ef2f8f5d73ac87272d13b3c6219927272875ffe379ec7f4e1f4f8f8fe` | broad incidental `Controller` xrefs; no GUI traversal graph promoted |
| `0x140D5D7D0` | `20260717T231433Z-xui02-topmost-consumer-v1-366bd0c0` | `C:\Users\Example\ghidra_agent_exports\20260717T231433Z-xui02-topmost-consumer-v1-366bd0c0\evidence\func_140d5d7d0.decompile.c` | `a78f6bf9b272a69acc4f8a6fcb61b37746e27dc1714c7dbddb9dcc6e5d6e3e0a` | refuted GUI candidate; exact task-force family |

Full terms, artifact hashes, incidental addresses, and negative results are frozen in
`replay_interface/re_evidence/rendering_laws/XUI02/ghidra_runtime_manager_ledger.json`.

No new capture or human input is required. No `PROGRAM_STATUS.md` edit or adversarial
review was performed.

### Production effective-VFS re-audit (2026-07-18)

The release integration no longer compares production playback to the frozen
454-file/9,155-window vanilla census. `register_effective_dormant_window_manager`
accepts the exact `interface/*.gui` winners exposed by the replay-admitted central
VFS, requires every file and window to parse without rejection, cross-checks each
file's admitted-window count, rejects duplicate window identity, and starts the
resulting namespace dormant. This registrar remains frozen evidence, but current
`ProductUiRuntime` does not construct the replay-owned manager from that catalog.

The frozen registrar and activation inventory remain evidence/verifier APIs; they
are not production selection inputs. The focused changed-namespace test
`admitted_effective_namespace_registers_without_frozen_vanilla_counts` proves that
an authenticated effective namespace with a different window count is admitted
without weakening any manager invariant. This proves the internal manager law; it
does not create a product panel route. XUI02 is product-omitted for lack of an
admitted panel/window-manager route, not because of mod/DLC namespace drift.

### V3 mechanical freeze

- promotion verifier: **71/71** on two consecutive byte-identical passes;
- verification SHA-256: `a1933529547ace523cff8a31a795c1b68df8b3b5860f79a94e7fc45634ddf348`;
- promotion contract SHA-256: `9db0801f37ba374b2d133556c8a6d19320ab457245a86c1bb39f16d12ad967c8`;
- activation inventory SHA-256: `8bd3b1f1bb99503acef0d0310bcb1766ba6dfdb35d0020d115056e6a261ad82c`;
- production manager SHA-256: `9d7af30505e216bf510458d8f6563552819935da989555278864d6fcd2ce0ce2`;
- focused tests SHA-256: `762b2bd3485a8aadd1b71e9f7b44035c36cd5cd9c20a61becbf61783d61b835d`;
- Ghidra runtime-manager ledger SHA-256: `b66d039ecf9e8d47d2479508f3dd4e202fbb74f15ead5352a80fde2006468285`.

## Historical external activation admission (v2, retained for provenance)

XUI02 now has both halves of the production contract. The dormant registry still covers all **9,155 / 9,155** exact XUI01 window definitions across 454 active GUI files, with zero rejected definitions. The new activation gate admits complete authenticated controller frames, expands each supplied source through exact same-file window ancestry, and preserves a typed producer dependency for every unsupplied row.

The exact activation inventory contains **318 source windows**. A complete frame for those sources resolves **669 exact window rows**: 318 direct sources and 351 inherited descendants. The other **8,486** rows are not silently coerced closed: 289 name their downstream XUI owner packet, 165 are static/helper roots with no runtime producer, and 8,032 name the absence of a current runtime producer.

| Activation class | Direct rows | Effective rows after ancestry |
|---|---:|---:|
| exact production controller window | 151 | 441 |
| typed file-controller root | 145 | 203 |
| explicit state-seam root | 5 | 8 |
| scripted-GUI predicate root | 17 | 17 |
| downstream-owner root | 102 | 289 |
| static/helper root with no runtime producer | 129 | 165 |
| root with no current runtime producer | 1,373 | 8,032 |
| inherited child | 7,233 | — |

The scripted-GUI join exhausts the effective 40-definition catalog: 17 `window_name` values match the frozen XUI01 playset and become exact XUI22/A38 activation sources; the remaining 23 names are recorded as absent from that playset rather than invented or discarded.

## Exact admission rules

Every runtime state must carry a producer packet ID, a 64-character lowercase SHA-256 manifest, and a nonblank producer-record identity. Unknown or duplicate sources reject. Closed windows must be invisible, uncomposed, ownerless, ordinal-free, and traversal-free. Visible windows must be composed, and composition is equivalent to having a stack ordinal. A modal owner must be another supplied, open, composed source in the same frame and cannot self-own.

Tooltip ownership is admitted only with an A26 resolved-tooltip-instance record. Focus ownership, restoration, keyboard traversal, or controller traversal is admitted only with an A32 variant-envelope record; traversal values must be unique and nonblank. The 17 scripted-GUI sources retain A38 as their explicit engine-evaluation dependency. These are data dependencies, not renderer guesses.

Inherited descendants receive only source open/visible/composed state and authenticated producer provenance. Modal ownership, stack ordinal, focus/restoration, tooltip ownership, and both traversal graphs remain on the direct source row; duplicating them onto nested windows would fabricate ownership.

## Production integration

`game_ui::window_manager_law` exports:

- `register_dormant_window_manager`, the exact 9,155-row fail-closed baseline;
- `resolvable_activation_sources`, the frozen 318-source catalog;
- `apply_runtime_activation_frame`, the authenticated frame validator and 9,155-row resolver/dependency partition.

The gate rejects namespace drift, definition rejection, duplicate definition identity, dependency-classification drift, unknown dormant state, unowned dormant activation, activation-inventory drift, duplicate runtime state, unresolved activation sources, invalid state relations, and invalid modal ownership. `replay_app` continues to register the exact dormant baseline at startup; downstream controllers can now submit complete frames without weakening that baseline.

The focused test target passes **9/9**, including direct A38 blocked-path coverage. The verifier also passes the full `game_ui` library gate and `cargo check -p replay_app`. It reproduces all five generated outputs byte-for-byte across two builder executions and passes **66/66** mechanical checks.

## Retained and binary evidence

The authored corpus pass still reproduces 454 files, 27,135 elements, 9,155 windows, zero parse failures, 1,037 visibility/animation assignments, 1,156 tooltip assignments, 486 `shortcut` assignments, and 39 `priority` assignments. It finds no authored focus-owner/restoration graph and no modal/z-order/topmost/stack/layer owner graph. Static source order, `priority`, and pointer collision order are therefore not promoted as visual stack order.

No new Ghidra operation was performed for this offline integration closure. Inherited exact binary laws remain pinned:

| Function address(es) | Job ID | Artifact path(s) | Manifest SHA-256 |
|---|---|---|---|
| `0x1422a4570`, `0x1422a44a0`, `0x1423158b0`, `0x1423128e0` | `20260716T032617Z-ui03-visibility-write-callers-v1-e7bef57f` | `evidence/func_1422a4570.decompile.c`, `func_1422a44a0.decompile.c`, `func_1423158b0.decompile.c`, `func_1423128e0.decompile.c` | `c9d79ce0db729d53b8954426b7c6496f793b74004e71b7ec4271b000d907652d` |
| `0x1422284b0`, `0x1422998c0`, `0x142220300`, `0x142229cf0`, `0x142299200` | `20260716T043945Z-ui07-guiobject-event-slot10-lower-neighb-5f5329bd` | matching `evidence/func_<address>.decompile.c` files | `0b1f8e644087b46b5df69252b2160d73ab56b500905a7923fc6a097ab0722d` |
| `0x142247750`, `0x142247fe0`, `0x142226cc0`, `0x1422284b0`, `0x142341000` | `20260716T044603Z-ui07-exact-gui-event-dispatch-root-v1-22f22462` | matching named decompiles under `evidence/` | `aed9b9cd838d694cb709f6384529357a1ac1440779e6244d22f90232d148585d` |
| `0x142226bd0` | `20260716T045001Z-ui07-collision-list-finalize-order-v1-8d10ad7e` | `evidence/func_142226bd0.decompile.c` | `6de3ee98d397b7601d7d7f08683eeceacf9ed8a859137259486a1e1859e0d2e4` |
| N/A — offline XUI02 integration | N/A | `replay_interface/re_evidence/rendering_laws/XUI02/artifact_manifest.json` | `19e74302e824dec40b1ed4fd747453a748ff87263797c78545aee450ef325642` |

## Mechanical freeze

- activation inventory: `5e9ae16b50a3d4e016f22aa78545476cd733da8f0be84afa2ceb1acb612ae611`;
- promotion contract: `82d722c5b2e90667fae63c3f6935e2b48ad52b90d655c5ff20bcbc1c294235fd`;
- verification: `1eb77a8525a8e1b37243757faef2345d69d3ee1d525592b1bd54bd242cddb90b`;
- production gate: `b308e581799afdb494443e6294cbc2732f85bc80c2f8ebbcdf76f006c81d2293`;
- focused tests: `f7a30817457d02fb66ad065feae6cd5c67459ffb9df2f0780d5abf9446cee297`.

No `PROGRAM_STATUS.md` edit, capture, or adversarial review was performed.
