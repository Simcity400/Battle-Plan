# 0903. Attribute reader work with measurement-only thread scopes

Measurement starts only when a harness enables it. A per-thread stack charges CPU to the current category and family, using the thread name for work before its first scope. CPU excludes time blocked on locks, conditions or channels. Wall time measures the entire attributed span, so the difference exposes off-CPU time. Calibrate the cycle-read cost at enablement and count reads so the harness can measure its own overhead. The short timestamp measures spans without waits. No counter changes which rows are decoded or published.

Shared work keeps the caller's fold, extraction or commit owner. Synchronous helpers may inherit the captured owner, phase and block, and nested typed reader operations append their own attribution. Reconstructed table-hours and decoded sections distinguish first-time work, repeats by the same family, repeats by another family and untracked observations where applicable. The recorder marks the known hour range before counting distinct table-hours. Shared-baseline statistics distinguish copied rows from replaced views that the reader can reuse. They exclude owned changes and snapshots.

Per-frame sampling can read the calling thread's labels without locking and walking every thread's full snapshot. Notes classify outcomes separately from the exclusive time already charged to the label. A scoped replay-data measurement counts operations on the calling thread and restores its previous observation even if the operation panics. The native thread handle is a kernel object reference, so Rust can send it between threads.

[Reader measurement](../../crates/replay-frame/src/cpu.rs) · [Replay-data measurement facade](../../crates/replay-data/src/measurement.rs)
