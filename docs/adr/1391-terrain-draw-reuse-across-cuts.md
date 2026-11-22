# 1391. Retain terrain draw bindings by wrap view across cuts

Keep a terrain block list independently for each wrap view. Compare the drawn block sequence at the same wrap offset. A block sampling a coast rim must also use the same slot of the same GPU stream, still owned by the same height allocation. Equal bytes in a replacement rim do not prove the preserved ownership and slot contract. Sampled or static coast heights require no rim match.

Reuse the exact previous block-list allocation when that test succeeds. Retained draw preparation can compare its per-view keys by allocation identity and keep bindings even if another part of the cover changed. Every new keyed view still needs its own compatible attachments, sampled textures and draw state. The regression test changes a block and replaces a rim with a byte-equal allocation separately, so both invalidation paths are exercised.

[Retained per-view cuts and render bindings](../../crates/viewer-terrain/src/terrain_shader.rs)
