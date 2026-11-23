# 0708. Authenticate province geometry before retaining movement operands

The recorder's load oracle walks province definitions once per load. This walk allocates and reads the whole map, so hourly capture uses its own demand-driven acquisition. The raster's province ID representation bounds the number of census records before allocation. Slot zero is counted and reserved. Each real province must match its census identity and definition ID, and its terrain pointer must have the expected terrain or null-terrain identity. The returned vector keeps one result per real province, indexed by province ID minus one, so an unreadable definition leaves that province missing.

Neighbour insertion keeps one entry for each target, which bounds the list by the census count. The movement oracle retains native entry order. Crossing kind and the flag are independent, and a nonzero flag suppresses the movement-arrow midpoint. Movement distance is signed fixed-point. The crossing point is a signed coordinate pair, copied to the reverse entry with kind, flag and distance. Read the geometry of a candidate edge only after matching the adjacency. An unreadable later entry should let an earlier match stand. Cache each independently demanded definition field as its own result so a failure can affect only readers that reach that field.

Terrain index zero denotes the null terrain. The land and lake bits serve separate decisions. Invasion selection requires both clear. A movement leg between land and sea uses the owning country's transport speed. A port's signed sea-zone ID can reflect a nonpositive authored value. The path classification compares it with the actual sea neighbour.

The native definition ID is unsigned and its neighbour count and target province ID are signed. `TERRAIN_FLAGS` is a byte independent of the terrain object. `PIXEL_SPANS` holds run-length spans. The ID-taking constructor clears the centroid fields. The terrain database index is one-based except for the null object.

[Province definitions](../../crates/engine-native/src/cprovince/definition.rs) · [Adjacency records](../../crates/engine-native-parts/src/province_neighbour.rs)
