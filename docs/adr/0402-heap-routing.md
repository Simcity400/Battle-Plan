# 0402. Route large heap blocks directly to Windows pages

Layout alone selects the backend so a block always returns through the allocator that supplied it. Normal requests use mimalloc. Large requests within the system's reservation-alignment limit reserve address space through Windows and commit only the pages they need. This avoids mimalloc's larger size classes and slice rounding. The reservation can grow in place. Shrinking decommits the trailing pages, and growth proceeds only within an uncommitted region of the same reservation.

Large freed blocks hold their own list headers while waiting for reuse. Each size class covers an octave, and a higher class can serve any request from a lower one. A same-class request chooses the smallest fitting block. Higher classes supply the oldest block from the first available class. The summary bits let allocation skip empty classes without locking, and a rotating sweep lets an untouched class release expired blocks. Contention falls back to fresh pages or immediate release. Unlinked chains are released after the class lock has been dropped.

The short retention window balances reuse against committed memory. Immediate release and recommit added about a tenth to the CPU time of a 13-year preparation benchmark, while a longer window saved little more and kept gigabytes committed. Small-block pages commit on demand. The application sets allocator options before worker threads run and has workers collect between tasks, since a worker can free pages and remain idle long enough to miss allocation-driven purging. Dropping the preparation guard also collects.

Reallocation keeps a small block only when its usable size can serve the new request without excessive slack. The thread that first calls into mimalloc may be reallocating a block allocated on another thread. The bundled v3 allocator's realloc path can dereference an uninitialized thread heap, so the wrapper checks usable size or allocates, copies and frees. The regression test keeps the process global allocator as the system allocator to preserve that first-call condition. Large-block pooling tests inject the time so expiry assertions need no wait.

Heap diagnostics read the pinned mimalloc option and statistics layout. A statistics layout the allocator rejects yields an unavailable result instead of a memory count, and the snapshot uses stack storage so it can be taken inside the allocator. The large-block counters distinguish overlapping calls from callers that actually found a held class lock.

## Unsafe operation invariants

`Heap` routes deallocation with the same layout rule as allocation. During `Heap::realloc`, the allocator contract supplies the valid requested size and the original alignment. `return_freed_memory_promptly` sets purge options before concurrent access. The `retention` queries require no allocation pointer. `committed` passes a buffer whose first words identify its size and version, which mimalloc checks before writing statistics.

`Kept::push` writes a header only to a freed reservation's committed, aligned base with room for that header. `Kept::{expire, take, take_oldest}` dereference only bases whose headers were written by `push`. The owning class lock protects unlinking. `release_chain` reads headers before releasing their bases after unlinking. `release` receives the base of a reservation created by `alloc_at`.

`alloc_at` can decommit only the trailing committed pages of a reused reservation. A fresh allocation first reserves the address range and then commits its front pages, which start zeroed. `resize` decommits only committed pages in the block's own reservation. Its zeroed `MEMORY_BASIC_INFORMATION` is valid for `VirtualQuery`, and growth commits the uncommitted tail only after verifying that Windows reports the same allocation base and sufficient space.

In `large_blocks_are_their_own_pages_reused_by_size_and_given_back`, the sequential cases free each reservation once with the same injected time so it remains eligible for reuse. The threaded cases read initialized bytes only after successful allocation, write only their own live block, and free it once after verification. The live-address set asserts that separate threads never receive the same live base.

[Heap routing](../../crates/engine-heap/src/lib.rs) · [Large blocks](../../crates/engine-heap/src/pages.rs) · [First-realloc regression](../../crates/engine-heap/tests/first_realloc.rs)
