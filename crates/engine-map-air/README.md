# engine-map-air

Native map air readers and their operands, layouts and synthetic tests.

The public capture API is available through [engine](../engine/README.md). Shared rules live in [AGENTS.md](../../AGENTS.md); field contracts, arithmetic, native gates and failure order stay beside each reader.

The module list is in [lib.rs](src/lib.rs). Production dependencies are [engine-native](../engine-native/README.md). Tests keep their synthetic fixtures and existing ignored scale gates.

The radar census offers its authenticated group and host pointers to a caller-supplied demanded symbol reader. Ordinary coverage walks read no symbol fields. New or writer-changed groups read only their cached radius. Host centres derive from the recorded state definitions and province pixel bounds, without capture.

The registered-mission operand packet includes the retained base-to-region trace's count threshold for interception alerts. It shares the existing 64-byte gathered row, and its optional read fails independently of sortie operands and CAS diversion.

The country-region lookup exposes its authenticated regional object to shared consumers. Ground Crews reads only its enabled byte and fails independently of the object's sortie-efficiency accumulator.

The registered-mission gather reuses the wing worker's current simulation execution and demanded effective aircraft when available, including independent read failures. The ordinary reader copies them itself, with retained effective aircraft demanded only for nonzero current masks in a stack-backed subset of the same batch. Neither path invalidates the remaining operands on an execution-leaf failure.

Only nonzero current executions with nonzero effective aircraft read wing normalization: the current mission's cached night statistic and two requested raw modifier entries. Feature-disabled entries preserve presence without demanding their raw word. The registered-mission worker supplies the authenticated embedded wing; no new census, hook or synchronous work is added.

Positive one-bit executions with nonzero aircraft also project cached surface/submarine aircraft-detection words from that same selected stat record for Navy patrol spotting. The selector, including its failure, is shared with night normalization. One two-word copy preserves independent leaf failures; no additional selector read or wing traversal is used.

The detection console vector reader is caller-owned first/change acquisition: its original header determines growth, bounded batches preserve raw signed values, and duplicate country keys refuse the vector. Periodic callers are not used.

Regional Navy spotting borrows the same selected record for raw cached attack and defence only when nonzero mission disruption reaches its combat reduction. It reads raw disruption first, skips bypass/current-count branches, then reads default cached speed and two raw wing combat keys through a shared accumulator header and bounded independent probes. No complete modifier scan, new census, hook, synchronous hourly read or per-wing allocation is added.

Nonzero-disruption contributors share the sortie worker's authenticated weather-manager observation, including a failure. Clear update gates acquire no held night marker. Otherwise the existing reusable mission-order scratch deduplicates demanded regions, and each reads only its original ordered sampled province's last-applied factor. The region vector, clock header/hour and marker are never read for an ordinary certified update or an unrelated wing.

All newly added Navy Air spotting leaves also require a nonzero loaded mission spotting factor. One stack cache shares the original dynamic array header and each distinct current-kind observation across the entire registered-mission pass. Zero/out-of-array factors acquire none of the new detection, disruption, combat or held-marker fields. A failed factor observation stays unknown and is not retried; the existing Air night/weather normalization is independent. The cache has the mission mask's inherent32 positions and never caps the installed array length.
