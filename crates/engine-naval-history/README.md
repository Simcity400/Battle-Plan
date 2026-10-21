# engine-naval-history

Native naval history readers and their operands, layouts and synthetic tests.

The public capture API is available through [engine](../engine/README.md). Shared rules live in [AGENTS.md](../../AGENTS.md); field contracts, arithmetic, native gates and failure order stay beside each reader.

The module list is in [lib.rs](src/lib.rs). Production dependencies are [engine-executable](../engine-executable/README.md), [engine-native](../engine-native/README.md). Tests keep their synthetic fixtures and existing ignored scale gates.

Embedded sunk-ship records use the same decoder as the global retained list. They retain their own lifetime and are not subjected to the global list's prune/date admission. Growing text storage follows the record's acquired string demand without restarting its reads.

`cnaval_accident_report` copies country-owned standing accident and mine reports in native occurrence order, keeping raw full model/ship references, signed damage, sunk byte, victim/mine-owner references and expiry hour after live ships disappear. Its caller supplies the existing authenticated navy and complete first/change writer certificate.

Naval modifier trees accept a sorted demanded key set without widening combat capture. Root-gated consumers read no child membership or labels when every demanded effective root is zero.

Transport speed acquisition exposes owned final operands for scalar and batched
queries. Native gates, conditional reduction arithmetic, shared-memo admission
and first refusals stay in acquisition; final modifier scaling/clamping and speed
multiplication can run after resume without a Memory capability. Synchronous
consumers evaluate the same owned operands immediately.
