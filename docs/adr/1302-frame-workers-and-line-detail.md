# 1302. Limit frame workers and line detail to what a frame needs

A frame lane runs one job with the same inputs and code the frame would use in place. That preserves the result when thread creation or job submission fails and the frame must run the job itself. The frame waits for these workers, so they use frame priority. The frame owns the outstanding job and joins it when leaving by success, error or unwind. A worker panic propagates when the frame takes the result, with an existing unwind taking precedence.

When zoomed out, fine border and river strips have multiple triangles per pixel. Simplify their paths by bounded deviation while keeping endpoints and authored anchors, then rebuild the strips so they retain their width and colours. Apply the vertical bound independently because a submerged segment fails depth testing. Select the coarse level only where its projected deviation remains below the screen-pixel tolerance. The map camera's height and vertical field of view bound that projection. A free eye can approach a line independently of its height, so it keeps the fine level. A single index buffer stores fine indices before coarse indices for both selections.

[Frame lanes](../../crates/viewer-scene-base/src/lane.rs) · [Line detail](../../crates/viewer-scene-base/src/line_lod.rs) · [Camera threshold fixtures](../../crates/viewer-scene-base/src/line_lod/tests.rs)
