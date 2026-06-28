# collector

In-process Battle Plan collector DLL.

This crate is intentionally separate from the native replay app. It is loaded
inside `hoi4.exe`, while the recorder and replay UI stay external. The current
ABI exposes version/heartbeat functions plus a Windows message-hook procedure
used for the no-op attach proof.

On process attach the DLL starts a short worker that emits a `collector_started`
JSON frame to the sidecar log and, when a recorder is listening, to the named
pipe `\\.\pipe\battle-plan_collector_<pid>`. When loaded inside the supported
`hoi4.exe` build, the frame includes a verified root probe:

- executable code sentinels for the daily-update and long-string offsets;
- live current/start game dates decoded from the game-state root;
- the country database root, bounded tag-table sample, and active
  country-present rows.

Frames include a monotonic producer sequence id. The Python recorder annotates
pipe reads with byte count and CRC32 before folding them into replay provenance
and manifest diagnostics.

For hourly proof runs, the recorder writes a short-lived
`collector_control.json` beside the DLL before attach. The DLL treats that as a
bounded worker request, samples verified roots at the requested interval, emits
JSONL frames over a per-run tokenized named pipe, and exits before the hook is
removed. Bounded worker runs disable sidecar JSONL file writes by default; the
pipe is the authoritative transport. This does not patch HOI4 code. Worker mode
is honored only inside `hoi4.exe`; the controller-side DLL load remains a
one-shot attach heartbeat.

The recorder-side session wrapper reassembles those pipe frames into a validated
`collector_frames.jsonl` stream inside the run directory. The replay snapshot
store still writes one canonical snapshot per game hour, but the raw stream
keeps same-hour samples instead of discarding them. That is the bridge for
sub-hour combat, movement, and event capture.

The root probe is intentionally narrow. It proves attach, transport, build
sentinels, clock decoding, and country identity row emission before any
trampoline hook or broad object traversal is added.

Country-object heap discovery is not part of normal capture. The experimental
whole-heap vtable scan is behind the explicit `country_object_scan` control flag,
defaults off, and is not speed-five safe. Runtime country/economy capture must be
root-anchored or hook-derived, cached before the hot loop, and reduced to bounded
pointer validation plus direct reads during recording.

Current country-scalar bridge:

- the Python recorder may run `live_capture.collector_country_bridge` as a
  preflight step before bounded worker recording;
- the primary preflight path locates the live country array, reads each country
  object's own `tag_db_id`, and avoids the fragile localized-anchor bridge scan;
- same-process cache hits revalidate the cached country-array token, read the
  tag table from the `CountryDataBase` root, and avoid heap scans;
- preflight rows are written only to the temporary `collector_control.json` as a
  compact `tag_db_id:country_object_ptr` string plus a separate
  `tag_db_id:country_record_ptr` string;
- manifests record bridge status and row count, not raw addresses;
- the DLL rejects malformed, duplicate, unaligned, or unexpected-tag rows;
- accepted rows are still revalidated and scalar values are re-read for every
  emitted frame;
- `country_object_scan` remains false for this path.

Political power uses the separate country-record bridge. It is emitted only when
the record pointer still points at the same validated country object.

Runtime rules:

- read-only observation only;
- no save-file dependency;
- unsupported builds must fail closed before domain readers run;
- no heap-wide discovery in the live recording loop;
- hook callbacks must stay tiny and hand work to transport/worker code.

Backpressure rule for future hooks:

- the hook path enqueues bounded descriptors only;
- the worker thread owns serialization and pipe/shared-memory writes;
- diagnostics and optional high-frequency deltas are dropped before scheduled
  snapshots;
- dropped scheduled snapshots become explicit manifest gaps;
- transport stalls fail closed instead of blocking the game thread.
