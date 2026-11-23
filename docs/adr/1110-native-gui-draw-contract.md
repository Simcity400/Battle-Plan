# 1110. Preserve native GUI draw records through shader submission

The layout path preserves each draw's local positions, UVs, attribute encoding and world matrix until GPU submission. Installed shaders can inspect raw position, secondary color or constant bytes without declaring the same field names. The shared projection uses a GUI viewport obtained by truncating the device extent divided by the resolved scale. The viewport clamp is already in the scale and applies once. Text may instead name an explicit font viewport, and a bake uses its target. The projection is column-major with scalar floating-point ordering and uses signed translation and depth appropriate to the target.

Topology depends on the native record kind. Plain sprites, progress bars, shield overlays and text sprites use the bottom-left, top-left, bottom-right, top-right strip. Plain tiles use the bottom-left, top-left, top-right, bottom-right strip. Inline icons begin at top-left and walk the top and then bottom edge. Glyphs, cornered tiles and nine-slice backgrounds use six records per quad as lists. Inline icons are emitted before the grouped glyphs of their frame, allowing an overlapping glyph to draw last. Bake backgrounds use the default blend state and write all channels. Glyphs keep the selected text technique's blend state.

The payload records allocation size and initialized ranges separately. Constant buffers from the pool may expose slot bytes beyond the upload that retain prior contents. Unpooled bindings expose their upload extent, with device zero for reads past it. A shader read of unwritten bytes within the exposed binding withholds that draw by name. Remaining textures retain the previous bind state when a draw does not set their slot. Shader compilation and refusal occur at sorted submission so one affected draw does not discard its neighbors.

The submitter tracks retained program and binding state through the actual sorted order. An undeclared technique's null record keeps the prior program and applies its own blend and binding changes. An unavailable shader can still update the retained program record, so a later missing technique can inherit and refuse it. A failure to draw for unavailable texture input can occur after its program has been bound. Vertex-only text constants can leave the earlier pixel constants in place, including any unknown pooled tail. Unavailable state remains named. The binding arena withholds only the draw whose uniform-slot count exceeds the device's dynamic-binding limit. A failure while assembling a batch leaves the caller's previous batch intact.

| Native `b0` record | Uploaded bytes | Visible bytes | Written bytes |
| --- | ---: | ---: | ---: |
| Sprite, bound texture, plain tile or text sprite | 112 | 256 | 104 |
| Cornered tile or shield overlay | 112 | 112 | 104 |
| Progress bar | 112 | 112 | 100 |
| Masked flag | 96 | 96 | 96 |
| Text and inline quad | 80 | 80 | 80 |
| Circular progress | 64 | 64 | 64 |

Sprite constants place colors and UV offsets after the matrix, followed by time and animation time. Shield overlay clocks are zero. The masked flag writes a zero tail after its flag coordinates. Inline quads preserve the text binding, and bake backgrounds initialize the whole block. Text, masked flags and circular progress bind `b0` to the vertex stage. The sprite, progress and tile paths bind both stages. Animation `b1` uses two 80-byte records in a pooled allocation. Each record starts with 48 initialized zero bytes, with later ranges marked only for the writes that its type performs. Without sampler slots the selected animation technique draws as an ordinary sprite with its mask, and binds neither animation records nor their textures. Slot width is one beyond the highest relevant common or pixel slot. An undeclared technique has no sampler slots.

Installed-effect tests compare actual submission with separately populated native records. Their projection, topology, constants and texture layout come from independent test inputs so reusing a production calculation cannot hide a regression.

[Draw submission](../../crates/viewer-native-gui/src/native_shader/draw.rs) · [Direct installed-effect records](../../crates/viewer-native-gui/src/native_shader/tests/models.rs)
