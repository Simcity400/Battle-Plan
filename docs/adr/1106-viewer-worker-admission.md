# 1106. Admit viewer work in dependency order

An hour part can wait for the preceding hour’s corresponding part. A bounded FIFO builder queue ensures the predecessor has already been dequeued when the dependent part starts. Jobs carry the submitting thread’s recording context. Parallel batches may offer work to spare builders, but completion cannot wait for turns still queued behind occupied builders.

An admission failure returns the original job and distinguishes capacity from disconnection. A nonblocking producer registers before trying admission, acknowledges its coalesced wake to sample the generation, then compares the generation after a full queue. A dequeue between the try and the comparison requires another try. A stable generation allows the producer to wait for the next wake. Dropping a listener prevents later lookups from finding it, while a callback already in flight can still run.

Workers release receiver and listener locks before calling subscribers or executing jobs. Dequeue skips the generation update, registry lock and allocation with no listeners. A callback panic re-arms that listener and leaves the accepted job available to run. A job panic leaves the worker available for other jobs. Queue and wake tests synchronize through explicit events, including a competing producer.

[Builder queue](../../crates/viewer-runtime/src/builder_pool.rs) · [Capacity listeners](../../crates/viewer-runtime/src/builder_pool/capacity.rs) · [Scheduling tests](../../crates/viewer-runtime/src/builder_pool/tests.rs)
