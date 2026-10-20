# job-priority

The scheduling priority of the preparation job a thread works for. A job
prepared ahead of the user (from the recording library) runs each of its threads
one step below the level that thread has in a job the user is waiting for, so the
relative order of a job's threads stays the same.

A thread's priority does not pass to the threads it starts, so every thread a
preparation job can start, in any crate, captures `JobPriority::current()` before
spawning and calls `enter(level)` first. Covered spawn sites are:

- `replay::container::parallel_map`: scoped frame and table decode workers
  (`Normal`); the stream producer and its grouped-table decode setup use
  `Caller` to match the observer they synchronously supply.
- `replay::container::pack`: scoped block pack workers and the pack writer's
  table/section helpers (`Normal`).
- `store`: scoped content-stamp and flush helpers (`Normal`).
- `dashboard-core::blocks::scan_blocks_by`: scoped block read-ahead workers
  (`Normal`), including production-lines block decoding and newest-window scans.
- `dashboard::prepared::prepass`: hourly pre-pass workers and the shared pool's
  block-task workers (`BelowNormal`, or the coordinator's `Normal` level while
  supplying the walk), including per-member watches and shared-reader deposits.
- `dashboard::prepared::pipeline::independent::shared`: the fixed shared-reader
  threads (`BelowNormal`).
- `dashboard::prepared::pipeline::independent`: family workers and every
  `Beside` job (`Normal`), including production scans and newest battle windows.
- `dashboard::prepared::pipeline::lanes`: family observation workers
  (`BelowNormal`); `pipeline::flush`: scoped family flush helpers (`Normal`).
- `dashboard-forces::products::parts_race_prepared::block`: scoped air and navy
  fold projections (`Normal`).
- `dashboard-politics-ledger`: scoped spending presentation helpers (`Normal`).
- Content-stack probing (`installed-content-core`), focus parsing (`installed-content-focus`),
  localisation reading (`installed-content-localisation`) and the dashboard's focus
  content preparation: scoped helpers (`Normal`).

- `installed-content-core::parallel::map`: scoped content-file read and parse workers
  (stacked `.gui` and `.gfx` reads), at their caller's job and priority (`Caller`).
- `dashboard-game-window::products::game_window::warm_in_background`: the read-ahead
  of a newly opened source's first game window, run as a job ahead of the user
  (`JobPriority::run` with its flag set), so it and its helpers run one step lower.

A helper that works synchronously for its caller (scripted localisation's
large-stack thread, content-file reads) uses `Caller`, which copies the caller's job and exact
priority. Outside a job, `enter` sets only the thread's own level. The job's flag
is shared with its progress: a job the user joins starts its later threads at
their usual levels.

The preparation queue's persistent runner threads start before a job is chosen;
each queued preparation installs its own flag and enters the coordinator's
`Normal` level with `JobPriority::run`, restoring its job and priority afterward.
The runner does not enter Windows background mode. `enter` also resets reused
threads to the requested level, including normal after below normal. Service
loader lanes and the response pump serve queries and stay at normal priority,
without a preparation job. A block scan serving a query likewise starts its
read-ahead workers at normal priority; production scans started by preparation
carry that preparation's flag through the same block scanner.

`dashboard-base` re-exports this crate as `prepared::priority`.
