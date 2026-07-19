# XUI05 — tooltip compositor

Status: **REPLAY LAW PROMOTION-READY; EXTERNAL INSTANCE DATA REQUIRED**.

Every downstream compositor law is implemented, product-integrated and twice
verified. The exact A26 producer is now implemented as schedulable domain 72,
and the handoff schema declares its engine-resolved per-instance screen
rectangle and ordered, evaluated tooltip rows. Replay remains fail-closed for
any frame whose authenticated domain-72 instance record is absent; unobtained
data is never coerced to an empty tooltip or reconstructed in the renderer.

## Exact implemented laws

- Hover collision uses the half-open intersection of element rectangle and
  clip rectangle. Frontmost selection is later draw list, then higher z, then
  later item; a frontmost ownerless surface occludes lower tooltip owners.
- `common/defines/00_graphics.lua` supplies show `0.05 s`, hide `0.05 s`, and
  delayed `1.0 s`. Owner changes reset the show clock; hide retains the prior
  tooltip until the exact hide threshold.
- Placement consumes an engine-resolved rectangle and preserves its origin and
  size. Viewport intersection is a scissor only. No cursor offset, edge flip,
  collision response, clamp, resize or rescale is guessed.
- Rows must arrive in strictly increasing unique `source_ordinal` order with
  line kind, source identity, evaluation context and availability. Replay does
  not sort rows, evaluate scripted localisation, evaluate Clausewitz script or
  aggregate dynamic breakdowns. A required unresolved row fails closed.
- The frame is a 54-vertex nine-slice using
  `gfx/interface/tiles/tiles_dialog.dds`, exact 18 px borders, and is emitted as
  the final UI draw family. The source TGA-to-DDS resolver law is reused.

Production sources:

- `replay_interface/native/game_ui/src/tooltip_compositor.rs`
- `replay_interface/native/replay_app/src/tooltip_runtime.rs`

## File and declaration census

The exact installed build has 130 top-level GUI files with 731
`pdx_tooltip`, 42 `pdx_tooltip_delayed`, and 55 `pdx_tooltip_anchor`
declarations. Recursive census has 453 GUI files with 816, 45, and 55
respectively. The verifier pins the executable and effective source hashes.

## Authenticated capture oracles

Two existing RenderDoc captures authenticate the chrome and two exact instance
rectangles at 1920x1080:

| Capture SHA-256 | Screen rectangle `(x,y,w,h)` | Chrome VB |
|---|---:|---|
| `6f88358cd31da7ec45887550dde2c75f14397c114b970b4e29c8c529dde01e4d` | `(1223,671,297,148)` | 1512 bytes, 54 vertices |
| `887a8593398281f5937929b7fdf054ec836128aa30544030df695f4d88717104` | `(1238,876,291,164)` | 1512 bytes, 54 vertices |

`capture_oracles.json` pins both RDCs, chrome VBs, composite quads, constant
buffer ZIPs and exact constant-buffer entries by SHA-256. These rectangles are
validation oracles, not a general placement algorithm.

## External instance-data boundary

Each replayed tooltip instance must carry both exact A26 leaves:

1. `resolved_screen_rectangle`: x, y, width, height, viewport, and exact
   producer-record identity for the hovered instance.
2. `ordered_evaluated_content_rows`: ordinal, kind, source identity,
   evaluation context, availability and final resolved text, including every
   dynamic/scripted breakdown row.

The A26 dossier currently reports `PROMOTION_READY_EXACT_A729_D72`, domain 72
`validated_slice_schedulable`, and no live implementation dependency. The
remaining condition is data presence in the recording being replayed, not an
unimplemented XUI05 rendering law. Missing instance data rejects exactly.

Contract: `extractor/documents/dossiers/A26_RESOLVED_TOOLTIP_INSTANCES.json`.
Politics-specific evaluated leaves remain additionally detailed by A24 and
generic runtime localisation/glyph-run leaves by A25.

## Mechanical verification

- `rustfmt --check`: PASS.
- `cargo test -p game_ui tooltip_compositor --lib`: 4/4 PASS twice.
- `cargo test -p replay_app tooltip_runtime`: 2/2 PASS twice.
- `cargo test -p game_ui`: PASS, including 98/98 library tests, every
  integration test, the acceptance harness, and doc tests.
- `verify_xui05.py`: 24/24 PASS twice, identical canonical verification SHA-256
  `ee99ac2b46179f07a2fde7374b9771e4f3154e1404653e8b4b097d2570064efa`.

Ghidra function address: none. Job ID: none. Ghidra artifact path: none. This
lane used pinned installed content and authenticated RenderDoc data only, as directed.

Evidence artifact path:
`replay_interface/re_evidence/rendering_laws/XUI05/`.

Payload manifest path:
`replay_interface/re_evidence/rendering_laws/XUI05/manifest.json`.

Payload manifest hash:
`20edd961ce3608534691ba3c69607e01ee5b48b65f8270bb936bd3be99384682`.
