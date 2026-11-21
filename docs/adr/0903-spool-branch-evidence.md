# 0903. Distinguish observed clocks, queued writes and durable history

Start a branch on a load, native world invalidation or clock discontinuity. A load signal wins even if known gaps cover the skipped range. A consecutive observed hour can still be a load. A failed load-signal read leaves the last known signal in force, with the failure recorded separately. Widen before comparing adjacent clocks so wrap at the signed boundary is a discontinuity. Known gaps describe exact missing ranges. The first command clock can precede the first frame at the same hour.

Record `loaded_hour` only from the first callback after a witnessed load. Losing that callback leaves the loaded clock unknown. `first_hour` identifies the first actually observed clock, and a recovered tail can seed the last observation without pretending the missing callbacks ran. A process creates one session identity. Resuming attaches to the same identity and still checks later observations for discontinuity. Recovery seeds parentage only from fully decoded and retained hours, preserving the exact gaps.

Observe once per callback and reuse the observation when storage retries. Perform fallible validation before mutating the tracker. Copy the returned gap notes into the stream. Admission can count an already queued hour as parentage evidence because cancelling that write also cancels every later queued branch start that relied on it. Acknowledgement adds a successfully written hour to the tracker's recorded history. Observing a clock alone gives no parentage evidence. A lost hour never fills a gap just because both endpoints were observed.

The replay frame crate owns the envelope shared by spool records and replay blocks. Filesystem I/O stays in the replay crate. The header keeps its magic, format, payload length and CRC in little-endian order. The decoder shares the same allocation bound for both kinds of record.

[Branch tracker](../../crates/replay-frame/src/branches.rs) · [Spool record envelope](../../crates/replay-frame/src/envelope.rs)
