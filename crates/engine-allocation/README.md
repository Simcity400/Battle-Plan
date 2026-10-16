# engine-allocation

Checked heap growth shared by native capture and the replay writer: fallible reserve, collection and copy helpers, the capture hooks that stop or fail growth, and the advisory scope for sizing work.

It has no workspace dependencies, so the content and recording layers can use it without the native readers. [engine-native](../engine-native/README.md) re-exports it as `allocation`, and the public API stays available through [engine](../engine/README.md). Shared rules live in [AGENTS.md](../../AGENTS.md).

`alloc-fault-injection` adds the before-growth hook tests use to deny chosen growths. `test-support` exposes `DeniedAllocator`, the global allocator engine-native's tests install; `with_allocations_denied` needs both `test-support` and `alloc-fault-injection`.
