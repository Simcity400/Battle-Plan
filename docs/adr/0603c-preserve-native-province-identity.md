# Preserve native province identity in the map projection

The native definition table is indexed by record position and the raster maps colors to declared IDs. Refuse records whose ID differs from position before publishing ID-keyed definitions. Otherwise, an out-of-order file can pair a color with a different province's properties. Keep record zero as a sentinel in definitions while excluding it from the color lookup. A later nonzero record takes precedence when a color repeats.

Read table columns with the native delimiters and compare the kind and coastal markers exactly. Numeric fields use signed decimal prefixes narrowed to their destination, including the last-column scan that can continue past a line break. Missing columns have native defaults. The continent index starts at one, with zero representing absence. Report a record missing the blue column and refuse kind values the map reader cannot model. Normalize the decoded raster to rows beginning at the top left irrespective of bitmap storage order. The native uncompressed 24-bit bitmap read derives the raster's location and length rather than trusting the file's advertised image size or pixel offset.

Recorded country colors already include cosmetic identity and HSV changes. The content layer supplies only the autonomy rule for using the overlord's color. The rule uses the native database's folder roster and order. A malformed boolean leaves the previously read value, and an omitted field defaults false.

[Province reader](../../crates/content-reader-map/src/map.rs)
