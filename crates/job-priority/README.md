# job-priority

The scheduling priority of the preparation job a thread works for. A job
prepared ahead of the user (from the recording library) runs each of its threads
one step below the level that thread has in a job the user is waiting for, so the
relative order of a job's threads stays the same.

A thread's priority does not pass to the threads it starts, so every thread a
preparation job can start, in any crate, captures `JobPriority::current()` before
spawning and calls `enter(level)` first: replay decoding and pack helpers, store
stamp and flush helpers, dashboard-core block scans, content-stack probing
(installed-content-core), focus parsing (installed-content-focus), localisation reading
(installed-content-localisation), the dashboard's pipeline and family workers, and focus
content preparation. A helper that works synchronously for its caller (scripted
localisation's large-stack thread) uses `Caller`, which copies the caller's job
and exact priority. Outside a
job, `enter` sets only the thread's own level. The job's flag is shared with its
progress: a job the user joins starts its later threads at their usual levels.

`dashboard-base` re-exports this crate as `prepared::priority`.
