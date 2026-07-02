# Shared Residual Patch Plan

Lane: `focus_research_decisions_depth_live`
Domain id: `45`

This file contains orchestrator-only shared integration work. The lane does not
keep these edits applied directly.

## Patch A: Compact Completed-Focus Token Closure

Purpose: allow domain 45 to close completed focus tokens exactly in compact
production by preserving the already-validated completed-focus vector in compact
focus rows.

Target: `extractor/native/collector/src/lib.rs`

```diff
@@
-    fn read_country_focus(object_ptr: usize, compact: bool) -> Option<CountryFocusRow> {
+    fn read_country_focus(object_ptr: usize, _compact: bool) -> Option<CountryFocusRow> {
@@
-        let completed = if compact {
-            None
-        } else {
-            completed_count.and_then(|size| read_focus_completed_vector(submgr, size))
-        };
+        let completed = completed_count.and_then(|size| read_focus_completed_vector(submgr, size));
```

Validation already performed transiently, then reverted from the shared file:

```powershell
cargo test --manifest-path extractor\native\Cargo.toml -p collector compact_country_data_payloads_pack_focus_politics_and_decisions
cargo test --manifest-path extractor\native\Cargo.toml -p recorder_host
cargo build --manifest-path extractor\native\Cargo.toml --release -p collector -p recorder_host -p snapshot
extractor\native\target\release\collector_record.exe --pid 45880 --dll extractor\native\target\release\collector.dll --out-dir extractor\native\runs\focus_research_decisions_depth_live_exact_completed_20260701_v1 --duration-s 90 --sample-ms 0 --min-game-hours 12 --scheduled-domain focus_research_decisions_depth_live --stream-format compact --extraction-perf-gate --compact-country-frame
cargo run --manifest-path extractor\native\lanes\focus_research_decisions_depth_live\Cargo.toml --bin gate_depth_stream -- --input extractor\native\runs\focus_research_decisions_depth_live_exact_completed_20260701_v1\collector_stream.bin --out-dir extractor\native\runs\focus_research_decisions_depth_live_exact_completed_gate_20260701_v1
extractor\native\target\release\pack_replay.exe verify extractor\native\runs\focus_research_decisions_depth_live_exact_completed_20260701_v1\native_session.battleplan
```

Proof result:

- Collector targeted test: passed.
- Recorder host tests: `65` tests passed.
- Release build: passed.
- Live proof: `144` records, `exact_completed=1`, `inferred_completed=0`,
  `unresolved_completed=0`, `errors=0`.
- `.battleplan`: verify `OK`.
- Hourly/current-equivalent: yes; `143` game hours advanced over `144` unique
  game-hour frames.
- Skipped hours: `0`.
- Failures: no partial/source/worker/bridge failures.

Risks:

- Payload size increases because compact focus rows include completed-token
  lists. The transient proof observed `completed_focus_tokens=21076`,
  `focus_live` p95 read time `47547us`, frame p95 `58442us`, and still passed
  hourly/current-equivalent cadence with zero skipped hours.
- This must land as a shared collector/recorder integration slot, not as a lane
  direct edit.

## Patch B: Active Targeted-Timed Body Tokens

Purpose: capture the validated body-token portion of
`active_targeted_timed_decision`, the validated country-target tag field for
matched country-target rows, and the validated numeric target id field for
matched `target=<number>` rows, while keeping unmatched transition rows,
numeric target semantic classification, cooldown, days, and state fail-closed
until an oracle proves those fields.

Validated root:

- `CDecisionStatus+0xE8` active targeted timed vector.
- Record vtable RVA `0x27D5038`.
- `record+0x10 -> CDecisionDef`.
- `CDecisionDef+0x120` decision token.
- Probe result: `1620/1620` sampled targeted-timed records resolved tokens
  across `170` decision tokens.
- Full-decision refresh: `3384/3384` targeted-timed records resolved tokens at
  vtable `0x27D5038`.
- Save-oracle target proof: `record+0x20` validates as country target
  `tag_db_id` for `1061` exact save/live matched country-target rows across
  `180` decision tokens. Unmatched country-target transition rows: `33` live
  rows across `5` decision tokens.
- Numeric save-oracle target proof: `record+0x24` validates as numeric target
  id for `2285` exact save/live matched `target=<number>` rows. The best offset
  has `2289` live positive rows versus `2288` save numeric target rows,
  `0.998689` matched-intersection ratio, `4` live-only rows, `3` save-only
  rows, and the runner-up offset has only `7` intersections.

Required integration shape:

1. Add `TARGETED_TIMED_DECISION_VTABLE_RVA = 0x27D5038` in the shared collector.
2. Add `TargetedTimedDecisionRow { decision: String, target_id: Option<u32>,
   target_kind: TargetKind, provenance: TargetProvenance }`.
3. Add `targeted_timed_live: Option<Vec<TargetedTimedDecisionRow>>` to
   `CountryDecisionsRow`.
4. Extend the decisions fingerprint to hash the targeted-timed vector data,
   size, capacity, record pointers, definition pointers, and definition-token
   string fingerprints.
5. Extend the compact `decisions_live` payload with a targeted-timed body-token
   count and rows shaped as `u32 tag_db_id`, `u32 target_id_or_u32_max`, flags
   indicating whether the target is a validated country target with `record+0x20`
   provenance or a validated numeric target with `record+0x24` provenance, plus
   compact string `decision`.
6. Feed those rows into the domain 45 lane-local schema. The lane schema is
   already proved: depth payload bytes `0..4` carry `decision_target_count`,
   bytes `4..8` remain reserved zero, and target rows are
   `u32 owner_tag_db_id`, `u32 target_id`, `u32 flags`, `u32 source_kind`,
   plus compact string `decision`.
7. Extend recorder-host and lane decoders to count
   `decision_target_rows`, `decision_country_target_rows`, and
   `decision_numeric_target_rows`.
8. Keep badge kind `2` because unmatched transition rows, numeric target
   semantic classification, and cooldown/day/state details remain blocked.

Fields still forbidden in this patch:

- Do not claim numeric target semantic classification as state/province/body id.
- Do not emit cooldown/day/state fields for vtable `0x27D5038`.
- Do not promote `record+0x28`, `+0x30`, `+0x34`, `+0x50`, `+0x5C`, `+0x60`,
  `+0x64`, `+0x70`, `+0x74`, `+0x78`, or `+0x7C` as targets.
- Do not emit `record+0x20` when it does not decode to a valid country tag id,
  or when the validation gate for the current build/mod/save fixture fails.
- Do not emit `record+0x24` when it is zero, fails the current validation gate,
  or belongs to an unmatched live/save transition row.

Proof required after orchestration applies this patch:

```powershell
cargo test --manifest-path extractor\native\Cargo.toml -p collector <new-targeted-timed-token-test>
cargo test --manifest-path extractor\native\Cargo.toml -p recorder_host
cargo test --manifest-path extractor\native\lanes\focus_research_decisions_depth_live\Cargo.toml
extractor\native\target\release\collector_record.exe --pid <pid> --dll extractor\native\target\release\collector.dll --out-dir extractor\native\runs\focus_research_decisions_depth_live_targeted_timed_tokens_<date> --duration-s 60 --sample-ms 0 --min-game-hours 12 --scheduled-domain focus_research_decisions_depth_live --stream-format compact --extraction-perf-gate --compact-country-frame
cargo run --manifest-path extractor\native\lanes\focus_research_decisions_depth_live\Cargo.toml --bin gate_depth_stream -- --input extractor\native\runs\focus_research_decisions_depth_live_targeted_timed_tokens_<date>\collector_stream.bin --out-dir extractor\native\runs\focus_research_decisions_depth_live_targeted_timed_tokens_gate_<date>
extractor\native\target\release\pack_replay.exe verify extractor\native\runs\focus_research_decisions_depth_live_targeted_timed_tokens_<date>\native_session.battleplan
```

Acceptance for Patch B:

- `decision_target_rows > 0`.
- `targeted_timed_country_target_rows > 0` for country-target fixtures, with
  `record+0x20` target tags matching the save oracle for accepted rows.
- `targeted_timed_numeric_target_rows > 0` for numeric-target fixtures, with
  `record+0x24` target ids matching the save oracle for accepted rows.
- Domain 45 country-target rows never exceed the corresponding
  `targeted_timed_count` per country/frame where the base vector is readable.
- Domain 45 total target rows never exceed the corresponding
  `targeted_timed_count` per country/frame where the base vector is readable.
- Badge kind `2` still present for unresolved target/cooldown fields.
- Zero parse errors, zero skipped hours, no partial/source/worker/bridge
  failures.

## Remaining Non-Patch Blockers

- Focus pause: all `24` byte candidates in `CFocusStatus+0x20..+0x37` are
  rejected by the lane-local v2 focus pause candidate gate (`13` direct
  byte-shape rejections plus `11` structural qword-overlap rejections). The
  bounded wider `CFocusStatus+0x00..+0x7F` v2 gate, excluding that prior
  window, rejects `100` additional outside-window byte candidates. The static
  v2 gap gate rejects the final `+0x3C..+0x3F` bytes by proving qword-sized
  CFocusStatus vtable-method access at `+0x38` and no direct access to the gap
  in the scanned methods. The bounded watcher records the paused=true oracle
  contract; a shared patch is not proposed until an alternate root outside
  `CFocusStatus` or oracle-backed field is validated.
- Decision target/cooldown fields: country-target `record+0x20` is validated for
  `1061` exact matched rows and numeric target `record+0x24` is validated for
  `2285` exact matched rows, but same-frame owner/decision/known-target oracle
  is still required for unmatched transition rows, numeric target semantic
  classification, and cooldown/day/state encodings. The v3 full-decision variant
  proof rejects `49` definition-local country-tag slices and leaves `50`
  heuristic slices non-production unless covered by the save-oracle target
  proofs.
- Direct engine hooks: named decision-event string route and named
  focus-completion callback string route are rejected by the v1 static
  string/xref gate. The follow-up static-contract gate rejects all `5`
  remaining text-xref anchors (`on_tech_research_completed`,
  `on_research_complete`, `on_research_complete_limit`,
  `complete_national_focus`, and `uncomplete_national_focus`) as registration
  catalog arguments in function `0x82CB0..0x11896F`, not runtime callbacks.
  A validated non-registration callback/function root and argument contract is
  still required for any hook patch.
