# 1102. Coalesce combat window reads off the rendering thread

Each combat window part or marker tooltip keeps one lazy read worker. The requesting thread only queues or takes work, or waits with a deadline. At most the newest pending read runs next. An already queued, running, or held key does not run twice. The completion callback wakes the view from the worker. Consume results only when their key still matches the requested view.

Carry the recording's executable tables into the worker with the job. A result that reads any undeclared table cannot be cached and reports the missed table by name. Dispose of discarded jobs and their frames on the worker because they may own the last reference to large row sets. An idle worker retains no completed job's frame, and the owner's drop stops the worker.

[Read worker](../../crates/viewer-runtime/src/reads.rs)
