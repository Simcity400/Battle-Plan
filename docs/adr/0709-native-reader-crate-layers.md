# Native reader crate layers

Native readers were built and tested as one crate. A change to a domain reader rebuilt the layouts and readers for other domains through the same dependency.

- `engine-native-memory` owns guarded reads, the Windows exception handler, read plans, buffers, and acquisition counters. `engine-native` forwards the features that control shared memory instrumentation.
- `engine-native-parts` owns independent native layouts and identity readers. It depends on the memory crate.
- `engine-native` owns the mutually dependent layouts. The core keeps the game’s pointer graph in one crate so collectors can follow it through the existing reader types. Splitting the cycle would require typed addresses in place of reader types across the collectors. State and theatre readers share those layout dependencies and live in the core.
- `engine-native-air`, `engine-native-intelligence`, `engine-native-land`, `engine-native-naval`, and `engine-native-politics` own the domain readers. Layouts sit below the derivations that read them. A derived result or walk belongs in its domain crate, and collectors and playback consumers import from the crate that defines the item.

[Memory](../../crates/engine-native-memory/src/lib.rs) · [Parts](../../crates/engine-native-parts/src/lib.rs) · [Layout core](../../crates/engine-native/src/lib.rs)
