# 0705. Authenticate full references at a quiescent boundary

Reference registry growth publishes the bucket mask before the slots pointer and has no generation field for a stable retry. Capture resolves references only at a quiescent boundary. Low types have their own roots, while the middle and high ranges share roots. A null root or missing key is an absent result. An invalid header or refused read remains a failure. Entry count is a live occupancy count and has no need for a bucket scan.

The per-pass scoped resolver keeps successes, null roots and failures with no eviction. The smaller province-occupant cache keeps successful header observations within one walk and authenticates each occupant independently. It retains neither unit identities nor bucket entries. A batched stage can acquire first buckets together and then preserve the normal probe order and failure result.

Handles retain the complete type and identity, including dynamically minted types. The stored word order differs from the order hashed by the registry. Probes compare the complete key rather than the auxiliary hash. A cell behind the required probe distance cannot validate a stale key or pointer. The default invalid handle is a fixed value and is independent of the minted-ID counter.

[Reference registry](../../crates/engine-native-parts/src/creference_registry.rs) · [Province unit order](../../crates/engine-native/src/cprovince.rs)
