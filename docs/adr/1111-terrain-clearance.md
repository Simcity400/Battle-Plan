# Evaluate sight against the continuous drawn terrain

Free-camera icon visibility tests the line from the eye to the anchor against the drawn ground or water. The anchor uses the closest world-wrap copy and a band near it excludes its own ground. A pyramid of block maxima skips regions that the line clears by the caller's band. Level one covers the sample grid surrounding each two-by-two block of cells. The raster has an extra horizontal cell across the wrap seam and one fewer vertical cell than sample rows. Walks clamp to edge rows beyond the map and handle a short last block at the seam. An incomplete raster produces no occlusion evidence.

Near the surface, test the actual triangle breaks, including a cell's authored diagonal. Clearance is linear within a triangle, or the minimum of two linear clearances where water covers it. Checking the boundaries detects a vertex crest that uniform sampling could miss. Cap the reported clearance to the requested band and stop once the line is fully occluded. Advance by at least one representable float when an edge nudge or minimum step would round away. Subtracting a small world distance at large coordinates cannot be assumed to cross an edge.

For combat widths, read province terrain from the recorded content and resolve categories in load order using ASCII case-insensitive names. Convert `combat_width` and `combat_support_width` to fixed5. A failed conversion is reported and leaves the earlier value in place.

[Terrain sight](../../crates/viewer-gpu/src/terrain_sight.rs) · [Seam and precision tests](../../crates/viewer-gpu/src/terrain_sight/tests.rs) · [Terrain widths](../../crates/viewer-gpu/src/terrain_content.rs)
