# The collector's serial tail: what the remaining four consume, and what moving them would cost

Every scheduled game hour the collector builds one frame. Most domains are read
by a bounded worker pool (`spawn_gated`,
`extractor/native/collector/src/attach_log.rs:34090`); a few are read one at a
time on the collector thread *after* `thread::scope` joins. That trailing
stretch is the **serial tail**.

Because the tail is last, it is the part still running when the engine's clock
rolls into the next hour. Those reads observe hour H+1 inside a frame labelled
H, `rejected_bracket_domains` (`attach_log.rs:37682`) classes them
`HourLabelMismatch`, and `HOUR_LABEL_MISMATCH_POLICY::Refuse`
(`attach_log.rs:37506`) turns the frame's DOMAIN_DATA record into a gap rather
than filing data under the wrong hour. The refusal is correct. The position is
what costs hours.

Eight domains were in the tail. **d85, d94, d95 and d97 had no dependency on
worker output at all** and were moved ahead of the fan-out (lane AO; see
`REORDERED_HOUR_SENSITIVE_MAIN_THREAD_READS`, `attach_log.rs:43142`). Four
remain: **d54, d55, d86, d90**. This document says what each of them actually
consumes, whether that is a data or an ordering constraint, what a restructure
would buy, and what it would cost.

## The prize, measured

Two 60-second captures, both with the moved four already in their new position.
That both postdate lane AO is readable from the captures themselves rather than
assumed: t25 reports **zero** `hour_label_mismatch` domain-hours for all four of
d85/d94/d95/d97, which is only reachable once they read ahead of the fan-out;
t26 additionally carries d95's `d95_hooks_installed` / `d95_site_entries_*`
diagnostic units, which lane AU added on top of AO. t25 does not carry those, so
it sits between the two lanes.

| | `runs/live_unlock/t25` | `runs/live_unlock/t26` |
|---|---|---|
| game hours in 60 s | 316 (190 ms/hour) | 651 (92 ms/hour) |
| FRAME_END records | 317 | 644 |
| frame build, `crit_wall_us` p50 | 43.1 ms | 57.0 ms |
| fan-out origin p50 / max | 22.8 / 31.4 ms | 25.7 / 36.8 ms |
| frames whose build saw an hour boundary | 91 of 316 probe frames (28.8%) | 324 of 496 (65.3%) |
| boundary position `at_us` p50 | 67.7 ms | 31.6 ms |
| gap domain-hours, all reasons | 2,471 | 25,771 |
| of which `hour_label_mismatch` | 1,285 (52.0%) | 13,411 (52.0%) |

t26 is the free-run baseline: at 92 ms per game hour against a ~57 ms build,
the boundary lands essentially anywhere in the frame. t25 is the slower,
loaded regime. Both are needed — a loaded machine flatters completeness.

Per-domain `hour_label_mismatch` domain-hours, from
`compact_stream.domain_gap_outcomes` in each capture's
`native_session_manifest.json`. (That flat array is now the run-encoded
`compact_stream.gap_ledger`; expand it with `scripts/gap_ledger.py` for the same
rows. t25 and t26 predate the change and carry the flat array, which nothing
reads any more.)

| domain | t25 | as % of its hours | as % of *boundary* frames | t26 | as % of its hours | as % of boundary frames |
|---|---|---|---|---|---|---|
| d54 | 90 | 28.4% | **99%** | 319 | 49.5% | **76%** |
| d55 | 89 | 28.1% | **97%** | 313 | 48.6% | **74%** |
| d86 | 89 | 28.1% | **97%** | 320 | 49.7% | **76%** |
| d90 | 89 | 28.1% | **97%** | 320 | 49.7% | **76%** |
| tail sum | **357** | | | **1,272** | | |
| pool-domain median | 28 | 8.8% | 31% | 208 | 32.3% | 49% |
| d85/d94/d95/d97 (moved) | 0 | 0.0% | **0%** | 152 | 23.6% | **36%** |

The tail is 27.8% of t25's whole hour-label loss and 14.4% of its total gap
hours; 9.5% and 4.9% respectively in t26.

The sharper statement is the boundary-frame column. **In a frame that saw an
hour boundary at all, the tail four are mislabelled 97–99% of the time in t25
and 74–76% in t26, against 0% / 36% for the four already moved to the front and
31% / 49% for the median pool domain.** They are not unlucky; they are simply
last. Bringing them to the pool median would return 245 domain-hours in t25 and
440 in t26; bringing them to the moved four's position would return 357 and 664.

### The risk, priced with the same instrument

Lane AO could price its move at zero because t22 showed the boundary
distribution and the pre-fan-out window to be **disjoint**: earliest
`lw_next_hour_at_us` 36,390 µs against latest `lw_fanout_origin_us` 31,931 µs.
That argument is not available for a move *into* the pool, and t26 says why:
the boundary's `at_us` p50 is 31.6 ms, which sits inside the worker span
(fan-out origin p50 25.7 ms → `crit_wall` p50 57.0 ms). **The whole in-pool
window carries boundary mass.** A consumer moved into the pool buys a lower
rank, not immunity.

The direct instrument for rank is `lane_wall.hour_boundary_position
.brackets_closed_before` — of the frame's ~60 clock brackets, how many had
already closed when the boundary landed:

| | t25 | t26 |
|---|---|---|
| p50 | 54 | 5 |
| mean | 40.6 | 20.1 |
| p95 / max | 56 / 59 | 56 / 59 |

The tail four close at rank ≈ 57–60, and the measured 97–99% / 74–76% mismatch
rates are exactly what that distribution predicts. The moved four close at rank
≈ 1–4 and measure 0% / 36%.

**A chained consumer's mismatch rate is bounded below by its own producer's
rate** — it cannot close before the worker it waits on — **and above by the tail
rate it has today.** Those bounds are measurable per domain and are given below.

## d54 — `army_battleplans_depth_live`

* Read site: the `if army_battleplans_depth_live_enabled` block at
  `attach_log.rs:45204`, calling `read_compact_army_battleplans_depth_live`
  (`attach_log.rs:70463`) under `lane_time_main`.
* Consumes: `army_order_read` (**d27**, `COMPACT_DOMAIN_ARMY_ORDERS`,
  `attach_log.rs:17722`) and `oob_command_read` (**d28**, `:17723`). It reads
  `army_order_read.status_flags`, `.observed_army_rows`, `.observed_order_rows`
  and its observation list, and the d28 rows.
* Own engine read: yes — `read_exact_domain54_orders_groups(state)`
  (`attach_log.rs:70577`).
* **Verdict: hard DATA dependency**, on two producers. Not ordering.
* Read cost: p50 671 µs (t25) / 718 µs (t26).

**Restructure.** d54 becomes a dependent worker that owns the d27 and d28
handles, joins them inside the pool, runs, and returns all three reads to the
collector thread. Its bracket then closes at `max(d27_end, d28_end) + 0.7 ms`.

Bound on the result: d28's own mismatch rate is 10 of 91 boundary frames (11%)
in t25 and 205 of 421 (49%) in t26, against d54's present 99% / 76%. Expected
recovery **60–80 domain-hours in t25 and 70–114 in t26, per 60-second capture**.

## d55 — `land_combat_detail_events_live`

* Read site: `attach_log.rs:45148`, `read_compact_land_combat_detail_events_live`
  (`attach_log.rs:67505`), under `lane_time_main`.
* Consumes: `land_combat_read` (**d21**, `COMPACT_DOMAIN_LAND_COMBATS`,
  `attach_log.rs:17717`) — its `.rows`, including each sample's
  `attacker_divisions`, `body_addr`, `attacker_side_addr` and
  `defender_side_addr`, plus `.status_flags`.
* Own engine read: yes — `domain55_passive_carrier_rows(GetModuleHandleW(null),
  &land_combat_read.rows)` at `attach_log.rs:67625` walks the combat sides
  **through the addresses d21 censused**.
* **Verdict: hard DATA dependency.** The dependency is stronger than d54's:
  d55's own engine read is *addressed* by d21's output, so it cannot even begin
  before d21 finishes.
* Read cost: p50 1,049 µs (t25) / 1,505 µs (t26).

**Restructure.** Same shape: a dependent worker that owns d21's handle. Bracket
closes at `d21_end + 1.5 ms`. d21's own rate is 28 of 91 boundary frames (31%)
in t25 and 211 of 421 (50%) in t26 against d55's 97% / 74%. Expected recovery
**~61 domain-hours in t25 and ~102 in t26.**

## d86 — `sunk_ship_history_live`

* Bracket: `attach_log.rs:45126`, opened jointly with d90's over the whole
  `publish_naval_member_epochs_after_terminal_drain` call (`:45138`, body at
  `:63380`).
* Its work inside that call is one statement:
  `sunk_ship_history_live_lane::a729_hook::peek_complete_snapshot()`
  (`attach_log.rs:63482`), which stores into `read.sunk_ship_complete_snapshot`.
* The payload is *encoded* far later, at `attach_log.rs:45244`, from that stored
  snapshot. No engine read happens on either path.
* Depends on d49 for exactly two things, and neither is data:
  1. a `&mut CompactNavalCombatLiveRead` to store the snapshot into
     (`attach_log.rs:63389`) — plumbing;
  2. the peek must precede `publish_member_epoch_snapshot(bindings,
     game_hour_raw)` (`attach_log.rs:63516`) — the "old-drain then new-snapshot
     ordering" of the comment at `attach_log.rs:45114`. It is the *republish*
     that needs d49's bindings, not the peek.
* **Verdict: ORDERING only.** d86 reads nothing d49 produced.
* Read cost: p50 5 µs (t25) / 6 µs (t26).

**d86 should not be moved. It should be stamped.** Its bracket is five
microseconds of queue peek wrapped around zero engine reads: it witnesses when
the collector *collected* the events, not when the engine *observed* the
sinkings — the hook did that, at an instant no bracket in the frame sees. This
is precisely the d94/d95 situation, and the owner-approved answer there was a
**producer-hour stamp** (`compact_contract::pov_visibility::FLAG_PRODUCER_HOUR`,
`compact_contract::spotted_units::FLAG_LAND_FRAME_HOUR`), not a reorder. An A33
event that carried the engine hour its hook fired in would make d86's bracket
position irrelevant to attribution, which no amount of moving can achieve.

If d86 is moved anyway, one question has to be answered first and it belongs to
the lane that owns the A33 read path: **does an A33 queue entry carry the member
epoch it was produced under, or is epoch attribution derived from the drain
sitting before the republish?** Draining at the front of frame N would take
events produced after frame N-1's tail drain but *before* frame N-1's republish,
together with events produced after it — two epochs in one batch. If entries
carry their epoch the move is safe; if not, the republish must move with the
drain, and the republish genuinely is pinned behind d49.

## d90 — `convoy_transport_attack_live`

* Bracket: `attach_log.rs:45132`, over the same call.
* Two distinct pieces of work inside it:
  1. `reserve_convoy_transport_before_resolver_republish()`
     (`attach_log.rs:63402`) — a queue reservation, ordering-only, like d86's;
  2. from `attach_log.rs:63543`, the route publication: it takes d49's
     `read.convoy_clients` census (`CFEXMember+0xD0`) and calls
     `naval4950::read_live_a729_client_route(client)`
     (`attach_log.rs:63563`) **per client** — live engine reads through
     addresses d49 censused.
* **Verdict: mixed. The queue drain is ordering; the route publication is a
  hard DATA dependency on d49, of the same addressed kind as d55's on d21.**
* Read cost: p50 11.5 ms (t25) / 1.5 ms (t26) — by far the most expensive
  member of the tail, and the reason d86's joint bracket is as wide as it is.

**Restructure: not worth pursuing as a move.** Splitting d90 would mean
publishing the queue part and the route part under separate brackets, which the
CTA2 carrier does not express — it emits one batch per hour. Chaining the whole
of d90 onto d49's handle is possible in principle, but d49's own read is
11.7 ms (t25) and its mismatch rate is comparable to the pool's, so the gain is
the same shape as d54/d55's and the plumbing is considerably worse (the
terminal-drain call also performs d86's peek and the shared epoch republish).
The honest ranking is d55, then d54, then d90.

## Summary of dependency kinds

| domain | consumes | kind | own engine read | read p50 (t25/t26) |
|---|---|---|---|---|
| d54 | d27 + d28 reads | **data** | yes | 0.67 / 0.72 ms |
| d55 | d21 rows **and addresses** | **data** | yes, through d21's addresses | 1.05 / 1.51 ms |
| d86 | nothing; must precede the epoch republish | **ordering** | **no** | 0.005 / 0.006 ms |
| d90 | d49 client census (route half) | **data** + ordering | yes | 11.5 / 1.5 ms |

## What may be implemented, and what may not

Nothing here is implementable behaviour-preservingly offline, and the reason is
uniform: every one of these restructures changes **which instant an engine read
observes**, which is a claim only a live capture can settle.

Three narrower observations, in decreasing confidence:

1. **d86's producer-hour stamp is the right fix and needs no reorder.** It is
   the same change d94 and d95 have already taken, on a domain whose payload
   likewise comes from a retained producer rather than from a read. Owner
   ruling on d94/d95 was explicitly "stamp, do not revert the move"; the same
   logic makes "stamp, do not move" correct here. Evidence needed: none beyond
   the A33 hook being able to read `CGameState+0x468` at hook entry, which is
   the same authenticated path d94 and d95 now use.

2. **d54 and d55 are the two real reorder candidates**, and both need the same
   two pieces of evidence before they may land:
   * a dependent-worker construction that preserves `joined_or_panic_outcome`
     semantics for d21/d27/d28 — their panic substitutes and their
     `domain_outcomes` entries are main-thread state today
     (`attach_log.rs:44487`, `:44495`, `:44719`);
   * a free-run capture at t26's speed showing (a) the predicted rank, and
     (b) that adding two more concurrent engine readers to a pool already bounded
     at `reader_concurrency = 12` does not lengthen the worker span enough to
     give back what the move buys, and produces no new `Torn`.

3. **Narrowing d86's bracket to its own peek is not worth doing on its own.**
   It would cut the bracket from ~1.5–11.5 ms (d90's route reads) to ~6 µs, but
   the bracket would still *close* at the same point in the frame, and the
   measurements above show position, not width, is what decides a mismatch.

## What does not change under any of these

`HOUR_LABEL_MISMATCH_POLICY` stays `Refuse`; the producer queue capacity stays
1; every read keeps its own `ReaderClockBracketGuard`; the emission order at the
tail is untouched. A restructure that needed any of those relaxed would be
trading an honest gap for a silent relabel, which is the thing the tail's cost
is worth paying to avoid.
