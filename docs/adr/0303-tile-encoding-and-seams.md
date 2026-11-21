# 0303. Serve the same chart from tile reads and whole views

Tiles are addressed by series, measure, line, bin width and position on the absolute grid. The unaffected tiles can be reused as a view advances. A tile wholly before the last recorded hour is immutable. Each tile reads its preceding bin or hour to decide whether its first line continues from the previous tile. Requests group keys as the dashboard does and cut each key's membership spans to the tile's read range. A requested key absent from an explicit membership list draws nowhere. The assembled view cuts summary tiles to complete bins. It joins hourly runs across valid seams and cuts them to the exact view hours.

One base64 field carries the binary arrays. The tile first writes a flag for each bin and key. For each present summary bin, it encodes the first, last, least and greatest values, using one value for a level bin. A lossless fixed-point value fits in a signed 32-bit word. Otherwise the tile uses floating point. Five hour offsets encode the bounds and the recorded hours of the last and extreme values in the smallest available byte width. A one-hour tile uses run starts and seam flags instead of hour offsets. Optional marks carry each bin's drops and count the drops before that tile.

[Tile encoding](../../crates/dashboard-query/src/tile.rs) · [View assembly](../../crates/dashboard-query/src/tile/view.rs)
