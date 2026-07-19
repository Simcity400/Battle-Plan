# XN17A — production display-variant admission and composition

Status: **PROVEN — PROMOTION READY / FAIL CLOSED**.

XN17A closes the production admission law for non-UI display variants. It does
not duplicate upstream shader, geometry, map-mode, particle, unit, or weather
equations. A frame is admitted exactly when display geometry is in the
fixed-base domain, selected map-mode/toggle/VFS inputs are complete, and every
requested producer has a frozen promotion contract. An unavailable producer
rejects the requested feature/frame; it never becomes a guessed colour,
substitute effect, synthesized population, or replay-host default.

This gate is complete, not partial. XN12C, XN13, XN14, XN15, and XN16 are
explicit active upstream feature gates. Their state cannot silently weaken a
frame, and their eventual frozen manifests can be inserted without changing
the XN17A decision rule.

## Supported display domain

- Map geometry is fixed at **5632×2048**. C04 proves the parameterized wrap
  consumer but admits only the authenticated fixed-base regime;
  `MapBundle::validate_product_dimensions` enforces the same boundary.
- A target extent is admitted when width and height are nonzero `u32` values,
  allocation succeeds, and `aspect=f32(width)/f32(height)` is finite and
  positive. C01 proves all 64 VP and all 64 inverse bytes for 2,433 applicable
  records across retained **1920×1080** and **2752×1152** regimes. Other
  positive extents are law-admitted, not mislabeled capture pixel oracles.
- Viewport size is construction state. A size change requires reconstruction
  of target, projection, dependent half-resolution post-FX resources, and pass
  graph. Live mutation with old camera/root/plane state rejects.
- Camera X/Z and height must be finite. X uses the 5632 wrap law. Z uses C03's
  projected-ground-footprint repair. Height lies in
  `[max(configured_min,active_map_min),configured_max]` under the C02/C03
  current/target and clamp order. Matrix fixtures use shipped 50/3000 bounds;
  `active_map_min` remains runtime data.

The factorized matrix covers eight resolution/aspect cases, five seam/edge/
center positions, three heights, 15 admitted built-in mode identities, every
**64/64** combination of the six display fields, and three admitted VFS shapes:
**345,600** domain cases. A domain case reaches presentation only after all
requested producer gates pass. XN11 dynamic modes are evaluated per effective
database entry and are not artificially capped.

## Map modes and toggles

Selection records engine registry identity/name; toolbar index never replaces
engine internal ID. XN08 closes operatives, supply, resistance, compliance and
raids. XN09 closes strategic air/navy, infrastructure and factions residuals.
XN11 closes the generic mod-extensible registry/evaluator. Vanilla resource
mode's exact contribution is no province/state fill. Diplomacy fill RGB is a
user scope removal, so diplomacy full-frame fill is outside the admitted exact
variant set rather than approximated.

XN10 supplies `player_counters_only`, `counter_color_mode`,
`show_allied_plans`, `show_radar`, `show_day_night`, and `show_fog_of_war`.
Record all six at replay start and every transition. Composition order is:
semantic population filters; terrain/city lighting; FoW; distance fog;
day/night; optional radar; M03 arrows in recorded `engine_draw_ordinal`; then
CounterQuads. GUI registration order is not render order.

## DLC/VFS and controlled absence

Every replay pins enabled DLC/mod order, `replace_path`, the effective ordered
VFS manifest, and normalized archive digests. UI15 admits loose layers and
attested staged archives, and rejects unstaged, missing, malformed, empty,
changed, or staged-drifted archives. Old expansion payloads in
`integrated_dlc/*` are runtime layers before enabled `dlc/*`; XN12A's corrected
18,238-row census proves why omitting that phase is invalid.

An actual XN12A registry miss has exact null/no-spawn behavior. A
producer-proven optional population may be empty. Missing required state,
selected map-mode input, asset, pass predecessor, palette, or trigger record
rejects the feature/frame. Aliases and fallback visuals are forbidden.

## Current producer gates

| Contribution | Admission |
|---|---|
| Geometry/aspect/camera | C01-C04 frozen |
| Selectable/scripted map modes | XN08-XN11 frozen; user-removed diplomacy fill excluded |
| Display toggles | XN10 frozen |
| Particle registry/lifecycle | corrected XN12A frozen |
| Particle simulation/render | XN12B frozen |
| Retained particle draw/asset correlation | XN12C active; required when that correlation is claimed |
| Dynamic unit presentation | XN13 active; contribution rejects |
| Weather/atmosphere | XN14 active; contribution rejects |
| Semantic effect-trigger stream | XN15 active; rejects until its final verifier and XN13/XN14/XN15A2 dependencies freeze |
| Final base-map synchronized composition | XN16 is `HUMAN_CAPTURE_REQUIRED`; its three external gates reject final composition |

`final_integration/event_compose` has useful passing narrower joins, but does
not supply XN16's missing synchronized trees/water/rivers frame, FINAL rivers
pass-close RTV, or PAN2 clear provenance. The stale `PROGRAM_STATUS` PROVEN row
is not consumed. These are upstream typed gates, not XN17A residuals.

## Ghidra provenance

XN17A makes no new binary claim and required no new Ghidra job. The machine
ledger `ghidra_provenance.json` freezes each consumed function address, job ID,
artifact path, artifact hash where published, manifest path, and manifest hash.

| Address | Job ID | Artifact path | Manifest SHA-256 |
|---|---|---|---|
| `0x14124bf10` | `20260716T040552Z-c00-camera-class-pre-rebuild-neighborhoo-a3312ec3` | `C:\Users\Example\ghidra_agent_exports\20260716T040552Z-c00-camera-class-pre-rebuild-neighborhoo-a3312ec3\evidence\func_14124bf10.decompile.c` | `f60c8c6e12a6afcfaa9295cec0541ff4b66a85192a4decaf311ecff4a03ed633` |
| `0x1424e8ca0` | `20260716T004707Z-c01-exact-matrix-core-v1-70d9a3d8` | `C:\Users\Example\ghidra_agent_exports\20260716T004707Z-c01-exact-matrix-core-v1-70d9a3d8\evidence\func_1424e8ca0.decompile.c` | `46b22c5f59a0ef708efd539fb4a47b12afe51ba0b3276c3e359217563aac8511` |
| `0x14124e290` | `20260716T090609Z-c03-wheel-clamp-update-promotion-v1-1f4090a4` | `C:\Users\Example\ghidra_agent_exports\20260716T090609Z-c03-wheel-clamp-update-promotion-v1-1f4090a4\evidence\func_14124e290.decompile.c` | `e194ce0d243e5bf94d3c0371035b7b4a7f5f8a39e9919599557ff35dbdc9290b` |
| `0x14124c280` | `20260716T084427Z-c03-camera-scalar-writer-clamp-v1-384d67eb` | `C:\Users\Example\ghidra_agent_exports\20260716T084427Z-c03-camera-scalar-writer-clamp-v1-384d67eb\evidence\func_14124c280.decompile.c` | `1f357289fc4e4860233fa3c14fbd7d20ede31717bbd69b53980dcd2edb803a93` |
| `0x14124cdd0` | `20260716T090609Z-c03-wheel-clamp-update-promotion-v1-1f4090a4` | `C:\Users\Example\ghidra_agent_exports\20260716T090609Z-c03-wheel-clamp-update-promotion-v1-1f4090a4\evidence\func_14124cdd0.decompile.c` | `e194ce0d243e5bf94d3c0371035b7b4a7f5f8a39e9919599557ff35dbdc9290b` |
| `0x14124c8c0` | `20260716T092319Z-c04-current-wrap-updater-and-view-constr-5d046294` | `C:\Users\Example\ghidra_agent_exports\20260716T092319Z-c04-current-wrap-updater-and-view-constr-5d046294\evidence\func_14124c8c0.decompile.c` | `8379801854757045986f695acb91bca67bb03f8b3bf7c5f28fbae26813eb974a` |
| `0x140dedfc0` | `20260717T000245Z-xn08-mapmode-registry-102f3ba3` | `C:\Users\Example\ghidra_agent_exports\20260717T000245Z-xn08-mapmode-registry-102f3ba3\evidence\func_140dedfc0.decompile.c` | `418ff9caa5584d42625a137d4fd56bf69d99335d44b799e9f6da46058edfa903` |
| `0x141563c80` | `20260716T235833Z-xn10-toggle-registration-and-radar-color-069992c0` | `C:\Users\Example\ghidra_agent_exports\20260716T235833Z-xn10-toggle-registration-and-radar-color-069992c0\evidence\func_141563c80.decompile.c` | `1ed289663731268cb6f1354afd1a598863864be1fd3e3e45c40f7a02feec3d9b` |

## Mechanical closeout

`admission_contract.json` is normative. `admission_matrix.json` stores the
factorized domain without bloating the master list, and
`dependency_snapshot.json` freezes consumed report/result hashes.
`verify_xn17a.py` checks authority, dependency hashes/statuses, product source
gates, exact f32 aspect rows, 64 toggle states, factor count, scope removals,
active-producer rejects, and every Ghidra tuple. Expected disposition is
`PROMOTION_READY_FAIL_CLOSED`; the frozen result passes **118/118** checks and
has SHA-256
`a16ba4c79056b554766e28db9bb4d56de841cd64abb13448d7d424c7cd4869a9`.
No `PROGRAM_STATUS` or production source file was modified.
