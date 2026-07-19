# XN16P — PAN2 clear-provenance recovery

Status: **PROVEN — PAN2 RTV/DSV clear gate closed by authenticated replacement capture**

XN16P owns only the `live_PAN2` clear-provenance residual. It does not investigate or modify the synchronized `live_trees_measure` gate or the `live_FINAL_full` rivers pass-close gate. The replacement evidence is frozen in `replay_interface/re_evidence/rendering_laws/XN16P/recovery.json` and mechanically rebuilt by `verify_xn16p.py`.

## Disposition

The original retained `live_PAN2` source remains incomplete: its four bundles contain exactly 72 `last_*_clear` fields, all null, and its event stream contains zero clear event rows. XN16P does not fabricate identities for that capture and does not transport a resource pointer from any other capture into it.

The gate is instead closed by an exact-a729 replacement capture:

`C:\tmp\hoi4_rendering_law_capture\20260717T000008Z-production-complete-frame\gpu\rounds\auto_h2400_plus`

Its pre-existing production verification is `PASS`, binds `hoi4.exe` SHA-256 `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`, validates all four same-round map passes, and verifies all 773 artifacts against evidence-manifest SHA-256 `a6129b173241365d032a9c3edc774e05d0894eb410e29489decee4dfcdb3a769`.

## Exact capture-local chain

The replacement terrain pass contains 14 contiguous captured draws, no blocker, and closes at `finalized_by = pass_close`. Every draw uses the same capture-local OM identities and the same last-clear records:

- RTV view `0x247AA912220`, resource `0x247A68C71A0`, 1920×1080, DXGI format 10, one sample;
- `ClearRenderTargetView` clear sequence **15**, color exactly `(0, 0, 0, 0)`;
- DSV view `0x247AA9132A0`, resource `0x247A68C9AE0`, 1920×1080, DXGI format 45, one sample;
- `ClearDepthStencilView` clear sequence **16**, flags `3` (`DEPTH|STENCIL`), depth `1.0`, stencil `0`.

Those four pointer values are valid only within this replacement round. The verifier confirms none appears in the original PAN2 bundle bytes; they are never used as a cross-capture join key.

The first captured depth-tested draw is draw index `0`: `DrawIndexed(24576, 0, 0)`, pixel shader FNV-1a64 `ce5e5185d46e214b`, vertex shader FNV-1a64 `e5a0acbee3ae07a2`. Depth is enabled, writes are enabled, the comparison enum is `4` (`LESS_EQUAL`), and stencil testing is disabled. The capture therefore proves the exact order:

```text
ClearRenderTargetView(seq 15, 0,0,0,0)
→ ClearDepthStencilView(seq 16, DEPTH|STENCIL, 1.0, 0)
→ first captured depth-tested terrain draw (draw_index 0)
→ remaining terrain draws 1..13
→ terrain pass-close
→ advance to water_pass
```

The replacement round event stream independently fixes the boundaries: round start at line 1, terrain `sequence_complete=true` at line 37, terrain→water advance at line 38, and clean round finish at line 69. The clear sequence numbers are the native hook's capture-local clear-call order; each clear is recorded after the underlying D3D11 call succeeds and stored against both the view and its resource. The bundle then attaches the matching last-clear records to the captured OM targets.

## Exhaustive source recovery

The prior XN16 audit counted only 199 RDCs and reported no mined clear API names. XN16P closes that search-coverage defect:

- 214/214 RDCs were content-hashed, totaling 69,298,844,274 bytes: 39 legacy captures, 88 from `session_2026-07-10`, 72 from `session_2026-07-12`, and 15 newer `C:\tmp` captures;
- all 285 decoded XML, JSONL, gzip-JSONL, and retained Chrome API stores were hashed and searched;
- 202 decoded stores contain `ClearRenderTargetView` (1,269 token occurrences) and 202 contain `ClearDepthStencilView` (1,495 occurrences);
- all 53 retained native `map_frame*_bundle.json` files were hashed and parsed; 14 carry non-null clear records.

The decoded RenderDoc records prove many other capture-local clear calls but cannot authenticate a PAN2 resource identity. They are retained in the exhaustive census and are not used for the result. The replacement native API capture supplies the needed same-capture resource/value/order/draw chain directly.

## Production contract

Replay the terrain pass by creating its own replay-local RTV and DSV from the captured descriptors, then applying the exact clear values and order before draw index 0. Never replay captured pointer values. Reject the frame if either clear record, its order, the OM descriptors, the depth-tested first draw, the pass-close boundary, the executable identity, or the source manifest is missing or mismatched.

The original PAN2 capture remains an admissible source for its previously proven payloads, but not as the provenance source for these clears. XN16P's replacement round is the authoritative clear seam.

## Mechanical closure

`verify_xn16p.py` passed a fresh 214-RDC rebuild and 36/36 checks. It rehashed the complete raw stores and replacement source files, compared the whole recovery document canonically, and checked exact identities, values, event ordering, draw state, pass boundary, manifest, executable, and capture-local pointer policy.

XN16P makes no binary claim and therefore has no Ghidra provenance tuple. No collector or terrain-capture source was edited. No excluded rendering item was investigated. `PROGRAM_STATUS.md` was not edited.
