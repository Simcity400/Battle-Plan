# M7 — Vertical Slice Verdict (M0→M6 complete)

> Closes the native replay-UI vertical slice (`VERTICAL_SLICE_SPEC.md`). M0→M6 are built, offscreen-PNG-verified, and Codex-reviewed. This is the §M7 deliverable: the **Bevy-vs-raw-wgpu verdict** + the **Phase-1 hardening list** distilled from building the slice.

## Slice status — COMPLETE
The full engine spine is proven end-to-end on REAL data, with no-fabrication enforced throughout (including under caching):
- **M0** store reader — forward-compatible; Absent/Null/Measured trichotomy.
- **M1** render primitives — `.dds` (BGRA8/BCn/RGB555/RXGB + CPU fallback), BMFont glyph quads, 9-slice; offscreen→PNG.
- **M2** `gui_runtime` — parse → scene-graph → full layout (orientation/origo/percent/clip) → draw + hit-test + loc; real topbar, 118 nodes.
- **M3** `bind` — real GER topbar; formatters; `unavailable`→"—" (Codex-proven no snapshot makes an Unavailable field show 0).
- **M4** GPU political map — `R32Uint` id texture + owner LUT shader; pan/zoom; CPU pick 8/8; sea/unowned neutral, never fabricated.
- **M5** state panel + the 3 foundations (overflowing scroll+clip, wrapped markup text, z-order pass-through) + composition.
- **M6** scrubber — windowed **immutable** LRU; domain-scoped views; scrub A→B→A byte-identical; warm fetch **0.019 ms**.

## THE VERDICT: raw `winit` + `wgpu` — CONFIRMED. Do NOT adopt Bevy.
Raw `winit`+`wgpu` carried M1→M6 with zero friction. Everything Codex said to own ourselves — the scene-graph, the layout engine, the scissor/clip stack, hit-testing, draw extraction — is hand-built and works. Bevy would have wrapped a fundamentally **custom Clausewitz renderer** in ECS + render-graph indirection for no benefit, and `bevy_ui` is unusable for this. The one thing Bevy's ECS might have helped — managing many UI entities — was handled cleanly by the `gui_runtime` scene-graph + the M5 gridbox row-factory. **Stay raw-wgpu.** Re-open only if a future view's entity/interaction load genuinely outgrows the scene-graph (not anticipated).

## PHASE-1 HARDENING LIST (carry into the binding sweep + remaining views)
Each was discovered + fixed in-crate while building the slice; listed so they aren't re-learned:
- **DDS reality:** the game UI is ~91% UNCOMPRESSED BGRA8, BCn a minority; RGB555 + RXGB are real; **non-4-pixel-aligned BCn** (e.g. the 37 px flag overlay) MUST route to CPU decode. The transcode fallback is load-bearing, not optional.
- **Flags are uncompressed TGA**, not DDS — needs the TGA decoder.
- **Device limits** must be raised to adapter resolution (the topbar strip is 2346 px > the 2048 downlevel default).
- **Layout:** the orientation/origo 9-anchor + the far-edge percent / negative-size formula (Codex-corrected); pure containers must NOT capture clicks (hit-capability gate — else a full-screen topbar swallows the map); the game **top-aligns** text.
- **Draw-clip:** a unified CPU scissor must crop EVERY primitive (sprite/glyph/9-slice/progressbar) — a per-element clip is easy to miss (the M5 leak).
- **Text:** measure AFTER `$VAR$`/`§`/`£` expansion; honour source spacing (don't synthesize spaces from markup spans).
- **Map:** province-id as `R32Uint` + `textureLoad` (nearest, no sRGB/mip); CPU-side pick (no GPU readback); normalize the `province_to_state` int-value vs string-key typing at load (else silent sea/unowned mis-render).
- **Scrub/cache:** windowed IMMUTABLE LRU — store `Arc<Snapshot>`, hand out clones only (refcount ≥ 2 blocks `&mut`); bind into a FRESH scene-graph, never back into the snapshot; domain-scoped `&`-only views; prefetch must not evict the cursor; the Absent/Null/Measured trichotomy must survive every view accessor.
- **Golden harness:** the offscreen-render→PNG per milestone + the scrub-A→B→A byte-compare ARE the M0.5 golden harness in practice — formalize as the regression baseline for the sweep.

## NEXT (post-slice)
- Extend `game_assets/extract.py` `PRIORITY_GUI` to the tech/doctrine **TREE** views + the remaining ~40 in-scope `.gui` (the binding sweep is already done for the 13 parsed views).
- Implement the **mod-awareness** layer (PLAN §K): resolver parameterized by the active mod-set; record the replay's mod identity at capture; default to the user's mod.
- The per-domain on-disk **store split** (a Python store-format change) only if a long real series needs it — the immutable LRU already makes warm scrubbing ≈ free.
- Validate the cracked-but-unwired **residuals** on a new active frame (the user's parked new-game + save pass).
