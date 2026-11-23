# Keep one division census lineage per callback

The callback acquires an independently bounded census, including its coverage counts, source packet, and five detail tables. A worker may read the full country census or one admitted range. It reserves off-thread storage for the callback to join the ranges. Input admission checks metadata only, so the later core walk can still encounter insufficient private capacity. A fresh fallback is allowed only before anyone borrows the input packet. Once leased, a failed core or detail walk and a mismatched context end that attempt. The original owner remains available to other consumers, and the validated detail-cadence flag stays with the same attempt.

Decode receives owned rows and buffers without rereading game memory. It transfers detail buffers, installs compact core rows and counts, and preserves the callback’s presentation rows and source packet. Core and counts are hourly and report failure even when the detail cadence skips an hour.

The synthetic comparison uses a typed walk with matching pooled capacities. It covers a country alias, null relationships, signed equipment amounts and delivery states. Compare only the outcome-certified rows, including retained truncation prefixes, because owned transfer may retain different headroom. A failed or unadopted batch must still drain every accepted lease.

[Division acquire and decode](../../crates/collector-capture/src/sampling/divisions.rs) · [Synthetic comparison](../../crates/collector-capture/src/sampling/divisions/divisions_test_kit.rs)
