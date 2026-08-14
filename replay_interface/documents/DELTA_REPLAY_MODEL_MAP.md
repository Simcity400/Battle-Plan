# delta_replay lane — current-model map (written before any change)

Base: lane/delta_replay @ 98f97640 (empty start marker) on lane/resource_wire @ 36f5c3ae.

## 1. How playback resolves a frame today

`WorldTimeline` (replay_core/src/world.rs) wraps a `Box<dyn ReplaySource>` plus a
`hours: Vec<i64>` spine (one entry per recorded snapshot hour, strictly increasing,
validated in `open_with_config`, world.rs:3936).

- `lookup_hour(h)` (:4066) binary-searches the spine and returns a `TimelineLookup`
  { requested_hour, frame_hour = nearest recorded hour AT-OR-BEFORE h, exact_match,
  next_hour }. Before the first hour → `WorldError::BeforeFirst`.
- `state_at(h)` (:4100) = `load_cached(lookup.frame_hour)`.
- `seek(h)` (:4107) = `state_at(h)` verbatim. Doc-comment: *"equivalent to state_at until
  compact delta replay is wired behind TimelineLookup."*
- `play_to(h)` (:4114) = `state_at(h)` verbatim. Doc-comment: the A4 equality hook; a future
  delta walker *"should preserve seek(h) == play_to(h) for all typed views."*
- `load_cached(h)` (:4118): LRU hit → return; else `raw = source.frame(h)` →
  `WorldFrame::from_raw_with_design_catalog(raw, design_catalog)` → insert into LRU → return.

## 2. What a RawFrame / WorldFrame contains

`RawFrame` (source.rs:194): `{ hour_index, date, save_version, schema_version, rest: Map }`.
`rest` carries every top-level domain untouched. **Every recorded hour is a FULL,
self-contained snapshot** — there is no keyframe+delta layering on disk today, and there
is NO cross-frame accumulation anywhere on the replay side (repo-wide grep confirmed:
`from_raw` takes exactly one `RawFrame`; the LRU (`FrameCache`, :4139) holds ≤ 16 decoded
`WorldFrame`s and is a pure cache — eviction never changes correctness).

`WorldFrame::from_raw_with_design_catalog` (:3492) decodes each domain from `frame.rest`
into a typed view. Point-in-time domains (owner_by_state, divisions, countries, politics,
production, …) are naturally whole-state per frame and are already correct under scrub —
they need no delta encoding.

## 3. The cumulative-history domains (the real blocker)

A domain whose truth is a cumulative HISTORY cannot be reconstructed from one frame unless
that frame ships the whole history. FRD is the motivating instance:

- `WorldFrame.frd_direct_transition_history: FrdDirectTransitionHistoryView` (:3095),
  built by `build_frd_direct_transition_history(rest, expected_hour, warnings)` (:19352)
  from `rest["_r2_direct_frd_transitions"]` (KEY, :19357).
- The v3 authority (`FrdDirectTransitionAuthority`, :15034; `schema_version` required == 3,
  :19410) carries `transitions: Vec<FrdDirectTransitionRow>` = the WHOLE history from the
  campaign start up to this frame's hour. Each row is validated: global 1-based `sequence`
  == index+1, `hook_rva` matches `kind`, phase/bits invariants, `event_hour_raw` in
  `[0, expected_hour]` and non-decreasing. `MAX_TRANSITIONS = 1_000_000` (:19358): a history
  longer than the cap makes the WHOLE view `DomainUnavailable` — the long-campaign data loss.
- Consumer: **none, since lane JL (2026-08-15).** This bullet used to name
  `admit_direct_transition_history_view` in `replay_app/src/selected_pov_runtime.rs` as the
  consumer that reads `frame.frd_direct_transition_history()` and expects the FULL history
  (counting every row by kind), and concluded that whatever `seek`/`state_at` returns MUST
  carry the full history because the WorldFrame external contract could not change. That
  consumer no longer exists: the whole `selected_pov_runtime` module was deleted with the
  legacy selected-Present reconstruction path, which owned its only instance
  (`extractor/native/runs/jl_render_purge/`). The view is still **built and stored** by
  `replay_core` (`world.rs::build_frd_direct_transition_history`, field at `:3693`), so the
  producer half is intact. What is gone is the **production presentation consumer** — no
  shipping code reads the view today. The contract itself is NOT unpinned: `replay_core`'s own
  tests still read it and assert full history explicitly, including exact accumulated lengths
  across a seek/play/refold cache boundary
  (`world.rs::frd_delta_seek_equals_play_to_byte_identical_across_cache_boundary`, which asserts
  `view.transitions.len() == (h + 1) * 2` per frame). So `seek`/`state_at` must still carry the
  full history to keep those gates green; what changed is only that the requirement is now held
  by replay_core's contract tests rather than by a downstream UI consumer.

### Producer (recorder_host/src/world_fold.rs)

- `frd_direct_transitions: Vec<DirectEventRow>` (:1752) is a fold-state Vec that
  `.extend()`s every admitted batch (:19284) and is NEVER trimmed. Each hour the ENTIRE
  accumulated Vec is serialized into `_r2_direct_frd_transitions` with `"schema_version": 3`
  (:6269-6289). That is the whole-history-per-frame emission.
- Drain/sequence contiguity is already enforced at fold time (drain_sequence, sequence
  cursor, hour non-regression — :19172-19298). The producer already knows, per hour, exactly
  which rows are NEW that hour (the `parsed.direct_transition_rows` batch appended at :19284).

## 4. Other cumulative-history domains (follow-on, NOT this lane)

Same whole-history-per-frame pattern, each its own sizeable conversion — enumerated so the
accumulator is built general but only FRD rides it here:

- `sunk_ship_history_exact` (SunkShipHistoryExactView)
- `occupation_history` / `occupation_history_complete`
- `fired_events` (FiredEventsView — "cumulative exact fired-event history", world_fold FEV1)
- `delayed_event_lifecycle` (D-… lifecycle history)
- `dynamic_modifier_lifecycle_exact`
- naval combat lifecycle + members (domains 49/50, "cumulative delta")
- `air_loss_stats` (D61) / `air_wing_cumulative` (D-wave3) — cumulative scalars, not row
  histories; would fold as running counters rather than concatenated rows.

## 5. Scope decision

This is NOT the "every domain snapshot needs a delta encoding" stop-condition. Point-in-time
snapshots stay whole-state and self-contained; `seek == play_to` holds for them trivially.
Only cumulative-HISTORY domains need folding, and they are a bounded, enumerable set. I build
the general per-frame-delta accumulator and convert FRD as the first rider; the rest are
follow-on with the pattern documented.

I am explicitly NOT converting point-in-time full snapshots into keyframe+delta chains — that
is a separate PERFORMANCE architecture (decode-cost reduction) and is not the blocker. The
blocker is cumulative-history data loss, which per-frame history-deltas + a fold fix.

## 6. Accumulator design (what this lane builds)

Wire: `_r2_direct_frd_transitions` gains a v4 form. v4 = same authority identity fields as v3
PLUS `delta_base_sequence` (global sequence of the first row in THIS frame's delta) and
`cumulative_count` (total transitions through this frame). `transitions` carries ONLY the
rows whose `event_hour_raw` falls in this frame's window. `schema_version` selects the
reading (policy from the resource_wire lane): v3 → whole-history path unchanged; v4 → delta.

A v4 frame may also carry `direct_history_reset: true` (absent means false), which says its rows
begin a NEW history rather than continuing the accumulated one — the consumer end of
`DELTA_WIRE_DESIGN.md` §7.3.1's D45-local reset badge. The recorder's fold sets it when the producer
badged a D45 record after its direct history died, and the accumulator drops its rows, its running
count and its non-decreasing-hour thread before validating the frame. It is required rather than
cosmetic: after a reset `delta_base_sequence` returns to 1 while the accumulator holds N rows, and
the contiguity rule below reads an unannounced regression as gap/overlap/reorder.

A reset MUST come with `direct_history_base_sequence`: the raw `row.sequence` its restarted row 1
holds. `delta_base_sequence`/`cumulative_count` count the emitted VECTOR while `row.sequence` is the
producer's own counter, which does not restart with the history, so the accumulator keeps a
`sequence_offset` anchored on the declared value and checks rows at `running + 1 + offset`. The
anchor cannot be read off the first row: an empty restart batch is legal and carries none, and the
frame after it opens at the batch's cursor. A reset with no declared anchor is a typed refusal.

The OPENING frame of a recording may carry the same `direct_history_base_sequence` with
`direct_history_reset` absent/false, and the accumulator anchors on it identically. A recorder's
fold takes its first admitted batch as the stream baseline whatever raw sequence that batch begins
at — a hook epoch's sequence space is process-wide while a fold is logical-stream scoped — so a
capture that attached with history already drained opens at raw N while `delta_base_sequence` still
opens at 1. Without the declaration replay read the opening as raw 1 and refused every row, losing
the domain for the recording. Three refusals bound the admission: an anchor of 0 (the space is
1-based), a bare declaration on any frame after the opening one (only a reset badge re-anchors a
history mid-stream), and an UNDECLARED opening whose rows do not begin at raw 1 — that last one is
the pre-existing strictness and it is unchanged.

Replay: `WorldTimeline` folds v4 deltas from the first frame up to `frame_hour`, validating
contiguity (delta_base_sequence == running_count+1; rows contiguous; hours non-decreasing
across frames; cumulative_count == running_count+len; version match) and reconstructs the
full `FrdDirectTransitionHistoryView`. Any gap/out-of-order/version-mismatch/over-bound is a
TYPED `WorldError::FrdDelta` — never a silent wrong state. The fold reads frames from
`source.frame()` directly, so it is correct across LRU eviction boundaries.

`seek(h)` uses the accumulator (anchor via TimelineLookup, fold forward). `play_to(h)` is an
INDEPENDENT linear reference (walk the spine first→frame_hour, append each frame's delta).
Both materialize the full history into the returned frame; the gate asserts they are
byte-identical, and a fault injected into the accumulator diverges them (goes red).

Bounds: per-frame delta ≤ MAX_TRANSITIONS; total accumulated ≤ a documented ceiling
(`MAX_ACCUMULATED_TRANSITIONS`) — larger than the per-frame/old whole-history cap so a
>old-cap history round-trips under delta where it was lost under cumulative. Test config can
lower caps to prove the semantics deterministically without materializing 1M rows.

## 7. Design decisions & Codex round-1 responses

- **play_to is fail-closed (Codex round-1 blocking #1, fixed).** play_to is a
  SEPARATE fold from the seek accumulator (so mutation A diverges them), but it
  re-validates the v4 authority, base/cumulative cursors, per-row contract, and
  cross-frame hour non-regression via the shared pure helpers. Calling play_to
  directly on a corrupt recording is a typed `WorldError::FrdDelta`, never a
  silent Present-but-wrong history. Test: `frd_delta_play_to_is_also_fail_closed_on_a_gap`.

- **Mixed v3/v4 timeline is refused (Codex rounds 1–4, CLOSED).** A recording is
  HOMOGENEOUS by producer construction — a v4 recording never emits a v3 frame
  (pre-baseline frames carry NO FRD key, not v3; once baseline is ready every
  frame is v4). A "v4-earlier + v3-anchor" timeline is only reachable by a
  corrupt/hand-built store, and the source API admits legacy directory and
  prefolded inputs (not only hash-authenticated containers), so trusting the
  anchor's schema in isolation is not enough. `WorldTimeline::ensure_frd_homogeneous`
  (backed by a `OnceLock`) runs a one-time, cached O(recorded-hours) scan that
  classifies every non-null FRD payload as v3 or v4 and returns a typed
  `WorldError::FrdDelta` if both appear. It is LAZY (never at open) and only runs
  when FRD is actually consumed, on both the v3-anchor path (`load_cached`) and
  the v4 `play_to` branch. A frame that will not decode leaves the verdict
  unestablishable and is refused fail-closed (no incomplete `Ok` is cached).
  Tested in both orderings (v3→v4 and v4→v3).

- **Bounds.** Per-frame delta cap 1,000,000 rows; accumulated cap 64,000,000
  rows; all sequence arithmetic is `checked_add`. The accumulator checks
  projected growth BEFORE materializing rows.
