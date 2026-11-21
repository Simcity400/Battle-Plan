# 0904. Encode row values with explicit bounds and provenance

The recording row codec reads explicit little-endian fields and canonical unsigned LEB128. The numeric delta path needs up to 65 bits. Bound collection allocation by the bytes available in the input. Diagnostic excerpts can shorten displayed text while retaining the original value. Acquisition uses static diagnostics to avoid allocating. An output-worker scope may attach formatted row and column context. The scope is thread-local, may nest and must end on the thread that entered it.

Prepared row codecs keep raw decoded values under their own format. Fixed prepared keys, decoded numbers, field absence and absolute typed patches retain their meaning when checkpoint pages are restored. Reuse the strict recording field codec only when it has no display conversion or field gaps. A zero-width retired value has a single prepared representation. A previously decoded value edited by its owner survives another prepare cycle without a second display rounding.

Fuel demand packs in the old format carry their lists and hour inline. Newer split packs read their lists from complete `FuelDemandParts` for the same hour. Keep inline lists when the pack still carries them. Withhold a pack by reason when its parts are missing or inconsistent. Occurrence keys keep repeated entities distinct, and list order is carried separately so an insertion need not change the existing identities. A pack with more occurrences than the key can express stays whole. Size old packs using the original bytes and field positions.

[Row codec](../../crates/replay-rows/src/codec.rs) · [Prepared values](../../crates/replay-rows/src/checkpoint/values.rs) · [Fuel-demand join](../../crates/replay-frame/src/fuel_demand.rs)
