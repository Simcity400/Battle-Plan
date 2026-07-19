# W20 map-furniture retained-draw correction

Status: one retained event's icon descriptors corrected; city-text identity and
geometry remain unproved and product-fail-closed; residuals explicit.

## Authority

- Binary: `hoi4.exe` 1.19.2.0.a729, SHA-256
  `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.
- Retained RDC: `session_2026-07-10/hoi4_frame68489.rdc`, 170,675,566
  bytes, SHA-256
  `c6d7900c865d5c4f895e36a2c1eaf0d7bf48ecd01aa395ada406fb34463932ca`.
- Event exporter: `renderdoc/mine_furniture_draw.py`; it resolves API chunks
  to replay events and samples the target at event-1/event, never end-frame.

## Exact results

The capture action labelled `berlin_vp_icon` by this lane, chunk 23249/event
19603, is a four-vertex triangle strip with a 580x27x1 slot-0 SRV. It uses
min/mag Linear, mip Point; U/V ClampEdge, W Wrap; LOD bias -0.8; MinLOD 0,
MaxLOD 1024. RGB and alpha both use SrcAlpha/InvSrcAlpha/Add. The sampled
29x27 rectangle changes 404 pixels. The fixture does **not** serialize VB/index
bytes, shader bytecode, or the slot-0 texture payload, and it is not a full
action census; therefore it does not prove the event's Berlin/VP identity, an
authored rim, that this was the sole contributing draw, or the absence of a
brightness operation. Fixture:
`renderdoc/fixtures/w20_furniture_draw/berlin_vp_icon.json`, SHA-256
`ed5d813ebd49bf2cca22e118f8a991a8439959c025e9fb4727b8ff734ac20602`.

The adjacent action labelled `berlin_city_text`, chunk 23266/event 19620, is a
42-vertex triangle-list draw. It binds slot-0 resource 343 with descriptor
metadata 512x256x1 and samples a caller-supplied 45x14 screen rectangle; 520
sampled pixels change. The fixture contains **no resource-343 payload**, no
vertex/index bytes, and no shader bytecode. It therefore cannot establish
immutability, byte equality to any DDS, the text `Berlin`, glyph geometry, or
the event's role. It proves only the recorded action/SRV metadata, sampler
(linear/linear/linear, Wrap/Wrap/Wrap, zero bias, MaxLOD 1024), blend (RGB
SrcAlpha/InvSrcAlpha/Add; alpha One/Zero/Maximum), and sampled pixel delta.
Independently, authoritative FILE
`<local-path> of Iron IV/interface/mapicons.gui`
lines 627-679 (60,174 bytes, SHA-256
`4c127c5d46c298dd5e0fd7b61b5f72681915a0bafb834fd1f914d58792ece205`)
declares both victory-point and capital text at `{26,5}` with font `hoi_20bs`.
That FILE law is not a join to resource 343 or this event. Fixture:
`renderdoc/fixtures/w20_furniture_draw/berlin_city_text.json`, SHA-256
`edbc361949d10abebbdc0260cf898f0754bbe52fb26e967e15fb8317ef6dd681`.

## Product change

`furniture_render.rs` now matches the retained icon sampler and blend state.
The prior `hoi_mapfont4` at 0.40 world-space city-name submission remains
disabled because it is not exact-authority-backed; this lane does not claim
that the retained event alone refutes it or proves its replacement.
When the existing FILE cutoff says text is visible, the draw plan records
`CapturedGuiTextRendererUnavailable` and emits no text rather than the wrong
font/path.

## Tests

- `python -B replay_interface/renderdoc/test_mine_furniture_draw.py`: 6 passed.
- `cargo test -p replay_app furniture --no-fail-fast --quiet`: 12 passed.

## Open blockers

1. Export the event's VB/index inputs, shader bytecode/reflection, and slot-0
   texture payload; join them to the authoritative `hoi_20bs` FILE declaration
   before implementing and held-out-gating any screen-space GUI text path.
2. Dynamic capital changes remain absent from the recording contract.
3. Resource-mode icons retain typed `ResourceIconPlacementUnavailable`.
4. Air/naval-base level-to-frame, sub-locator offset, and cutoff equality remain
   unproved. Wave-5 Ghidra semantics and function-name requests are
   **quarantined discovery only**: the orchestrator's serialized identity gate
   found project-program SHA-256
   `16022851eaa571728aa3a0b5fd5f990a56da57e2249a4c1b9cfb30336f59b120`,
   not the required a729 binary SHA-256
   `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.
   No address or semantic claim from that project is production authority.
5. Historical `map_viewer::build_furniture_quads` tooling still contains the
   unproven `hoi_mapfont4`/0.40 world-text implementation. Exact-or-fail policy
   removes its authority and the product path no longer calls it for text, but
   that tooling must be deleted or converted in a separately bounded change.
