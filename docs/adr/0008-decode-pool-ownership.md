# 0008. Recycle capture payloads off the game thread

The hourly callback takes a ready payload from its family's pool with a try-lock. If the lock is held or the pool is empty, it takes a vacant job and grows what it actually reads. Vacant payloads allocate nothing while a worker owns the ready buffers. Retiring a job queues it for the decode worker, which clears it, grows it from observed demand when necessary, and returns it to the pool. The callback can continue without blocking on worker pool maintenance.

The steady pool size is a retention target. Backlogged frame slots add capacity to that target so a worker can keep up when the backlog drains. Once the backlog clears the worker discards excess payloads. It also keeps sealed source/control plans that decoded successfully, because production decode leaves the captured bytes intact.

The sizing thread presizes idle payloads for map dimension and registry changes. New dimensions must reach idle jobs before the first hourly copy on the new branch. The game thread can still fall back to a vacant job if sizing holds the pool. New modifier hints also update jobs prepared before the campaign loaded. For `keep()`, the loader publishes `SHARED` before it starts the decode worker and clears it after joining that worker. This makes the lookup valid for the worker's lifetime.

[Decode pools](../../crates/collector-capture/src/decode/pools.rs) · [Capture frame pool](../../crates/collector-capture/src/frame_pool.rs)
