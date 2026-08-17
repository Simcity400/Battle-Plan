use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::adapter::{
    build_lane_frame_from_adapter_input_ref, build_lane_frame_reusing_walk, AdapterBuildStats,
    AdapterInput, AdapterInputRef, AdvisorRoot, CachedWalk, CharacterPortraitInput,
    CharacterRoot, CountryLeaderRoleRoot, CountryPoliticsRoot, TraitDefinitionRoot,
    UnitLeaderRoot, CACHEABLE_WALK_FAMILIES,
};
use crate::domain47_characters_leaders_cabinet_live as compact;
use crate::extractor::{
    encode_frame_payload, encode_frame_payload_over, LaneFrame, LaneMemory, ResolvedTypeRoots,
    DOMAIN_ID, DOMAIN_NAME,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeRootCache {
    pub roots: Option<ResolvedTypeRoots>,
    pub roots_validated: bool,
    pub skipped_root_rows: u32,
    /// Which row families the prewarm that adopted these roots actually
    /// OBSERVED.
    ///
    /// The character census and the unit leaders it reaches through
    /// `CCharacter+0xA8` come from an exact engine-owned manager roster; the
    /// country-leader-role, advisor and trait-definition roots come from the
    /// shared heap sweep, which needs seconds to complete its first pass. Those
    /// two sources fail independently, and this field is what lets a prewarm
    /// adopt the half it did read instead of discarding both.
    ///
    /// A clear bit means the frame walk must not emit that family AND the fold
    /// must not treat its absence as a deletion — see
    /// [`compact::FAMILY_CHARACTER`].
    ///
    /// Not `Default`-derived: zero would mean "observed nothing" and would turn
    /// every `..Default::default()` construction into a silent total gap.
    pub known_families: u32,
    pub characters: Vec<CharacterRoot>,
    pub unit_leaders: Vec<UnitLeaderRoot>,
    pub unit_leader_trait_tokens: Vec<TraitDefinitionRoot>,
    pub country_leader_roles: Vec<CountryLeaderRoleRoot>,
    pub advisors: Vec<AdvisorRoot>,
    pub portraits: Vec<CharacterPortraitInput>,
}

impl Default for NativeRootCache {
    fn default() -> Self {
        Self {
            roots: None,
            roots_validated: false,
            skipped_root_rows: 0,
            known_families: compact::FAMILY_ALL,
            characters: Vec::new(),
            unit_leaders: Vec::new(),
            unit_leader_trait_tokens: Vec::new(),
            country_leader_roles: Vec::new(),
            advisors: Vec::new(),
            portraits: Vec::new(),
        }
    }
}

impl NativeRootCache {
    pub fn stable_root_count(&self) -> usize {
        self.characters.len()
            + self.unit_leaders.len()
            + self.unit_leader_trait_tokens.len()
            + self.country_leader_roles.len()
            + self.advisors.len()
            + self.portraits.len()
    }

    pub fn has_stable_roots(&self) -> bool {
        self.roots_validated && self.roots.is_some() && self.stable_root_count() != 0
    }

}

/// Is this frame's rowless outcome already decided by the inputs
/// [`read_native_scheduled_frame`] was handed, before it walks anything?
///
/// The walk's own product can only be discarded, never published, whenever this
/// holds: it is two of the three disjuncts of the post-walk gate below, and
/// both of them are pure functions of the cache and the frame input. A torn
/// root read or a clock-refused prewarm (`skipped_root_rows`) and a skipped
/// country row (`skipped_input_rows`) are ROW-level loss inside families the
/// frame did observe, which the family-presence mask cannot describe — so the
/// gate replaces the built frame with a rowless `SOURCE_UNAVAILABLE |
/// PARTIAL_DATA` one whatever the walk found. The third disjunct,
/// `AdapterBuildStats::has_rejections`, is the walk's own finding and is NOT
/// knowable here; frames that only fail it still walk.
///
/// HOW OFTEN THE OUTCOME IS PRE-DECIDED. In `runs/live_unlock/t49_pricing`,
/// `lane_wall.hour_boundary_position.brackets_closed_before` has p50 0 over the
/// 325 frames that witnessed an hour boundary, and the host's p50 is
/// `sorted[ceil(n/2) - 1]`, so at least 163 of that capture's 334 extracted
/// frames witnessed the boundary before ANY bracket had closed. d47's prewarm
/// bracket is the first bracket in the frame — the prewarm runs before the
/// main-thread reads and before the fan-out, and the clock ordinal totally
/// orders every authenticated read — so those frames witnessed it at or before
/// the prewarm's own two observations. That verdict is `Straddled` or
/// `WrongHour` (`Unreadable` also produces a zero here, but t49 records no d47
/// `clock_unreadable` gap at all), and all three poison the roots
/// (`domain47_apply_prewarm_clock_verdict`) before the fan-out opens. At least
/// 163 of 334 frames therefore had d47's rowless outcome DECIDED before any
/// worker started.
///
/// WHAT THAT COUNT IS NOT. It is not a count of walks removed. This arm skips a
/// walk only where the cache also holds stable roots, and a cold blocked
/// adoption installs a ROOTLESS cache carrying `skipped_root_rows`
/// (`domain47_apply_blocked_adoption`): such a frame is pre-decided too, and it
/// already took the rootless arm below without walking anything. How many of
/// the 163 held stable roots is not on the wire — that diagnostic rides d47's
/// PERF record, which a stripped hour does not have. Closing it needs an
/// unstripped per-frame counter of "pre-decided AND stable roots".
///
/// NOR DOES ANYTHING HERE PRICE A REMOVED WALK. `read_time_us` is a DOMAIN_PERF
/// distribution and a stripped d47 hour has no PERF record, so t49's p50
/// 43.2 ms describes its 39 SURVIVING reads and no stripped one; the prewarm
/// p50 (42.4 ms over all 334 extracted frames) is a different population and
/// must not be added to it. The saving is settled by a live A/B on frame wall,
/// not from these manifests. What is established without measurement is the
/// direction: on a pre-decided frame the walk's product cannot be published,
/// whatever it costs.
///
/// WHAT IS LOST BY NOT WALKING. `AdapterBuildStats`, which describes rows this
/// frame does not publish. NOT the parse errors: those are the self-decode of
/// the frame's own emitted payload, and a rowless payload has none on this path
/// already. Each cause is still named without them, one diagnostic per
/// disjunct: poisoned roots by `domain47_prewarm_skipped_root_rows` and
/// `domain47_prewarm_clock_verdict`, a skipped country row by
/// `domain47_country_bridge_unstable_rows`, which the collector increments on
/// the same rejection that increments `skipped_input_rows` (and which is the
/// only one of the three that says anything on that path — a frame whose
/// country bridge rejected a row can carry a clean `InFrameHour` prewarm).
/// All three ride this hour's provisional PERF record and survive exactly when
/// that record does — a bracket-rejected hour has its PERF and DATA records
/// dropped in reconciliation and keeps only its DOMAIN_GAP reason, which is the
/// same before this change and after it.
/// The emitted payload, its status word and its family mask are unchanged —
/// `skipped_root_rows_emit_rowless_partial_read` exercises this exact path with
/// engine memory present and pins the wire outcome.
fn rowless_outcome_is_already_decided(cache: &NativeRootCache, frame_input: &NativeFrameInput) -> bool {
    cache.skipped_root_rows != 0 || frame_input.skipped_input_rows != 0
}

/// The rowless fail-closed frame the three gates in [`read_native_scheduled_frame`]
/// all publish, built in one place so a pre-walk refusal and a post-walk refusal
/// cannot spell themselves differently.
fn rowless_partial_frame(elapsed_wall_ms: u64, status_flags: u32) -> LaneFrame {
    let mut frame = LaneFrame::new(elapsed_wall_ms);
    frame.status_flags =
        status_flags | compact::STATUS_SOURCE_UNAVAILABLE | compact::STATUS_PARTIAL_DATA;
    frame
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NativeFrameInput {
    pub elapsed_wall_ms: u64,
    pub status_flags: u32,
    pub skipped_input_rows: u32,
    pub countries: Vec<CountryPoliticsRoot>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NativeScheduledRead {
    pub domain_id: u16,
    pub domain_name: &'static str,
    pub payload: Vec<u8>,
    pub read_time_us: u64,
    pub row_count: u32,
    pub data_units_total: u32,
    pub status_flags: u32,
    pub schema_flags: u32,
    pub adapter_stats: AdapterBuildStats,
    pub parse_errors: Vec<String>,
}

/// What d47's clock bracket has to hold, and nothing else (lane HC, task #119).
///
/// # Why this type exists
///
/// The copy-only-bracket rule says a clock bracket may hold engine OBSERVATIONS
/// and nothing else: work that only touches bytes this process already owns
/// belongs after the bracket closes. [`read_native_scheduled_frame`] ran three
/// phases back to back inside the ONE bracket `spawn_gated` opened around the
/// whole worker, and only the FIRST of them names an engine address:
///
///  1. [`build_lane_frame_from_adapter_input_ref`] — the only phase that reads
///     the game's memory.
///  2. [`encode_frame_payload`] — serialises the frame phase 1 just built.
///  3. [`compact::decode_payload`] — re-parses those same bytes to derive the
///     record's row and unit counts, its status/schema flags and its parse
///     errors.
///
/// Phases 2 and 3 cannot observe a torn read, cannot straddle an hour and
/// cannot be misattributed, because they never touch the engine. Every
/// microsecond they cost was a microsecond the bracket was held open for no
/// observation. Lane DB named that quantity in 2026-08-06 and its instrument
/// `d47_cost_read_bracket_observation_vs_compute` measured it at **71.5-74.3 %
/// of the bracket** on the committed fixture — two runs on one tip, both
/// published in `runs/hc_d47_copy_only/` (`instruments_currenttip.txt` and
/// `instruments_relabelled.txt`), per invocation. That band is an UPPER bound
/// on the live share for the three reasons the instrument's own doc comment
/// gives, and it is a debug-profile figure.
///
/// This type is what phase 1 hands across the bracket close: an OWNED frame and
/// the stats that describe it. It holds no engine address, no `LaneMemory` and
/// no borrow of the root cache, which is what lets the worker drop its clock
/// guard before [`finish_native_scheduled_read`] runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeScheduledObservation {
    /// What phase 1 built, or the fail-closed rowless frame one of the three
    /// gates decided on. Either way it is complete: nothing after the bracket
    /// may change which rows this frame carries.
    frame: LaneFrame,
    stats: AdapterBuildStats,
    /// What the OBSERVE half spent, already elapsed.
    ///
    /// A DURATION AND NOT THE OPENING `Instant`, and the difference is the
    /// whole point (Codex 019ff886, finding 2). Carrying the opening instant
    /// and sampling it at the end of the compute would silently widen
    /// `read_time_us`: in production the two halves are no longer adjacent —
    /// `spawn_gated_split` drops the `ReaderClockBracketGuard` between them,
    /// and that drop takes the closing clock read, renders the bracket verdict,
    /// records the bracket, and under the `range-tracer` feature flushes the
    /// trace. None of that was inside the old timer.
    ///
    /// A `Duration`, NOT a microsecond count, so the two halves are added at
    /// full resolution and truncated once (Codex 019ff897, finding 4).
    /// Truncating each half first can lose up to a microsecond per half.
    ///
    /// WHAT `read_time_us` NOW IS, stated exactly: the sum of two timed phase
    /// bodies. It is NOT the former single continuous span, and the residue is
    /// the call/return/destructure seam between them, which no longer belongs
    /// to either half. On the split path that is the honest quantity — the
    /// alternative sweeps in the bracket close — and on the composed path the
    /// seam is a function call.
    observe: Duration,
    /// Lane MT: the adopting frame's walk this frame reused for the families
    /// `cached.families` names (their vectors in `frame` are empty). Shared,
    /// immutable, decided before the bracket closed - the compute half only
    /// reads it. `None` on a frame that walked every family itself.
    cached: Option<Arc<CachedWalk>>,
}

impl NativeScheduledObservation {
    /// Lane MT: turn THIS (adopting) observation's walk output into the
    /// [`CachedWalk`] later reused frames take, for `families` (a subset of
    /// [`CACHEABLE_WALK_FAMILIES`] the collector proved witnessed), and make
    /// this observation the first to encode over it. Returns `None` - and
    /// changes nothing - when the walk is not reusable: it rejected or
    /// degraded a row, it built no rows (rowless outcome), it itself reused a
    /// cache, or `families` is empty.
    ///
    /// NOTHING IS CLONED (Codex 01a00f20 finding 2): the cached families'
    /// rows are MOVED out of `frame` into the cache (the frame's vectors for
    /// them are left empty, exactly the shape a reused frame has, and the
    /// encoder reads them back from the cache), the portrait rows are
    /// partitioned by key (cached keys' admissions move into the cache in
    /// their input order; the rest stay as this frame's extras), and the two
    /// indexes are sorted vectors. What remains is one sort of ~25k keys.
    pub fn adopt_cached_walk(&mut self, families: u32) -> Option<Arc<CachedWalk>> {
        let families = families & CACHEABLE_WALK_FAMILIES;
        // Unit leaders and their traits are one loop: both bits or neither.
        let leader_pair = compact::FAMILY_UNIT_LEADER | compact::FAMILY_UNIT_LEADER_TRAIT;
        let families = if families & leader_pair == leader_pair {
            families
        } else {
            families & !leader_pair
        };
        if families == 0
            || self.cached.is_some()
            || self.stats.has_rejections()
            || self.stats.identity_degraded_rows != 0
            || (self.frame.characters.is_empty()
                && self.frame.advisor_roster.is_empty()
                && self.frame.country_leader_roles.is_empty()
                && self.frame.unit_leaders.is_empty())
        {
            return None;
        }
        let mut cached = CachedWalk {
            families,
            stats: self.stats.clone(),
            ..CachedWalk::default()
        };
        let mut known_keys: Vec<(u32, u32)> = Vec::new();
        if families & compact::FAMILY_CHARACTER != 0 {
            cached.characters = std::mem::take(&mut self.frame.characters);
            cached.character_identity = cached
                .characters
                .iter()
                .enumerate()
                .map(|(index, row)| ((row.country_tag_db_id, row.char_id), index as u32))
                .collect();
            cached.character_identity.sort_unstable_by_key(|(key, _)| *key);
            known_keys.extend(cached.character_identity.iter().map(|(key, _)| *key));
        }
        if families & compact::FAMILY_UNIT_LEADER != 0 {
            cached.unit_leaders = std::mem::take(&mut self.frame.unit_leaders);
            cached.unit_leader_traits = std::mem::take(&mut self.frame.unit_leader_traits);
            known_keys.extend(cached.unit_leaders.iter().map(|r| (r.country_tag_db_id, r.char_id)));
        }
        if families & compact::FAMILY_COUNTRY_LEADER_ROLE != 0 {
            cached.country_leader_roles = std::mem::take(&mut self.frame.country_leader_roles);
            known_keys.extend(cached.country_leader_roles.iter().map(|r| (r.country_tag_db_id, r.char_id)));
        }
        if families & compact::FAMILY_ADVISOR_ROSTER != 0 {
            cached.advisor_roster = std::mem::take(&mut self.frame.advisor_roster);
            known_keys.extend(cached.advisor_roster.iter().map(|r| (r.country_tag_db_id, r.char_id)));
        }
        known_keys.sort_unstable();
        known_keys.dedup();
        cached.known_keys = known_keys;
        // Portraits. The cache holds the cached keys' admissions in input
        // order. When EVERY admitted portrait has a cached key (the common
        // case) they MOVE into the cache and this frame encodes them from
        // there; otherwise the frame keeps its whole family (input order on
        // the wire, Codex 01a00f20 round 2) and the cache takes a clone of the
        // cached-key subset for the reused frames after it.
        let cached_key = |p: &crate::extractor::CharacterPortraitRecord| {
            cached.knows(&(p.country_tag_db_id, p.char_id))
        };
        if self.frame.character_portraits.iter().all(cached_key) {
            cached.character_portraits = std::mem::take(&mut self.frame.character_portraits);
        } else {
            cached.character_portraits = self
                .frame
                .character_portraits
                .iter()
                .filter(|p| cached_key(p))
                .cloned()
                .collect();
        }
        let cached = Arc::new(cached);
        self.cached = Some(cached.clone());
        Some(cached)
    }

    /// Which families this observation took from a cached walk (0 = none).
    pub fn reused_walk_families(&self) -> u32 {
        self.cached.as_ref().map_or(0, |c| c.families)
    }
}

/// PHASE 1 — the only half that reads the engine.
///
/// Split out of [`read_native_scheduled_frame`] by lane HC so the worker can
/// close its clock bracket here. Its body is that function's first half,
/// unchanged: the same three gates, the same walk, the same family-mask and
/// rejection decisions, in the same order.
pub fn observe_native_scheduled_frame<M: LaneMemory>(
    mem: &M,
    cache: &NativeRootCache,
    frame_input: NativeFrameInput,
) -> NativeScheduledObservation {
    observe_native_scheduled_frame_reusing(mem, cache, frame_input, None)
}

/// [`observe_native_scheduled_frame`] for a frame that REPLAYED the discovery
/// witness and may take `cached`'s families from the adopting frame's walk
/// (lane MT). `None` is the plain call.
pub fn observe_native_scheduled_frame_reusing<M: LaneMemory>(
    mem: &M,
    cache: &NativeRootCache,
    frame_input: NativeFrameInput,
    cached: Option<Arc<CachedWalk>>,
) -> NativeScheduledObservation {
    let started = Instant::now();
    let (frame, stats, cached) =
        observe_native_scheduled_parts(mem, cache, frame_input, cached);
    NativeScheduledObservation {
        frame,
        stats,
        observe: started.elapsed(),
        cached,
    }
}

/// A whole span as the `u64` microseconds the wire carries. Converted ONCE, at
/// the end, over the summed halves — see
/// [`NativeScheduledObservation::observe`].
fn duration_us(span: Duration) -> u64 {
    span.as_micros().min(u64::MAX as u128) as u64
}

/// PHASE 2 + 3 — engine-free, and therefore run AFTER the bracket has closed.
///
/// Takes the owned observation and does exactly what the tail of
/// [`read_native_scheduled_frame`] did: encode the frame, self-decode the
/// payload for the gate's counts and flags, and assemble the read. There is no
/// path from here to the game's memory: the argument holds none.
pub fn finish_native_scheduled_read(
    observation: NativeScheduledObservation,
) -> NativeScheduledRead {
    let NativeScheduledObservation {
        frame,
        stats,
        observe,
        cached,
    } = observation;
    let compute_started = Instant::now();
    let payload = encode_frame_payload_over(&frame, cached.as_deref());
    let gate = compact::decode_payload(0, &payload);
    // THE SUM OF TWO MEASURED HALVES, not one span across the bracket close.
    // See [`NativeScheduledObservation::observe`] for what a single span would
    // have swept up that the unsplit function never charged here, and for what
    // this quantity is and is not. Summed as `Duration`s and converted once, so
    // neither half is truncated before the addition; saturating, so a
    // pathological clock cannot wrap the published figure.
    let read_time_us = duration_us(observe.saturating_add(compute_started.elapsed()));
    NativeScheduledRead {
        domain_id: DOMAIN_ID,
        domain_name: DOMAIN_NAME,
        payload,
        read_time_us,
        row_count: gate.row_count_total.min(u32::MAX as u64) as u32,
        data_units_total: gate
            .data_units_total
            .values()
            .copied()
            .sum::<u64>()
            .min(u32::MAX as u64) as u32,
        status_flags: gate.status_flags_or | frame.status_flags,
        schema_flags: gate.schema_flags_or,
        adapter_stats: stats,
        parse_errors: gate.parse_errors,
    }
}

/// The two halves, run back to back on one thread.
///
/// KEPT, and it is not dead code: the fixture and benchmark entry points, the
/// lane's own tests and every caller that is not inside a clock bracket use
/// this. Production's d47 worker does NOT — it calls the two halves either side
/// of its bracket close (see `spawn_gated_split` at the d47 call site in the
/// collector). Composing them here rather than duplicating the body is what
/// stops the bracketed and unbracketed paths from drifting.
pub fn read_native_scheduled_frame<M: LaneMemory>(
    mem: &M,
    cache: &NativeRootCache,
    frame_input: NativeFrameInput,
) -> NativeScheduledRead {
    finish_native_scheduled_read(observe_native_scheduled_frame(mem, cache, frame_input))
}

fn observe_native_scheduled_parts<M: LaneMemory>(
    mem: &M,
    cache: &NativeRootCache,
    frame_input: NativeFrameInput,
    cached: Option<Arc<CachedWalk>>,
) -> (LaneFrame, AdapterBuildStats, Option<Arc<CachedWalk>>) {
    if cache.has_stable_roots()
        && rowless_outcome_is_already_decided(cache, &frame_input)
    {
        // NOT WALKING IS THE POINT. The gate below would replace whatever this
        // walk built with exactly this frame, and both of the conditions that
        // decide it are already true — see
        // [`rowless_outcome_is_already_decided`]. Building the frame first
        // would read the whole character world out of the engine to produce a
        // payload this function is already committed to discarding.
        (
            rowless_partial_frame(frame_input.elapsed_wall_ms, frame_input.status_flags),
            AdapterBuildStats::default(),
            None,
        )
    } else if cache.has_stable_roots() {
        // BORROW the cached roots instead of deep-cloning them into an owned
        // `AdapterInput` (d47 critical-path work, 2026-08-03). The frame walk
        // only reads them, and the clone was thousands of per-hour `String`
        // allocations that were dropped again at the end of the same frame.
        // `adapter_input_from_cache` still exists and still produces the same
        // owned value for the fixture/benchmark entry points; both forms feed
        // the identical builder over the identical data.
        let adapter_input = adapter_input_ref_from_cache(cache, &frame_input);
        let roots = &cache.roots.expect("checked by has_stable_roots");
        // Lane MT: a cached walk serving NO family is no cache at all.
        let cached = cached.filter(|c| c.families != 0);
        let mut adapter_result = match cached.as_deref() {
            Some(cached_walk) => {
                build_lane_frame_reusing_walk(mem, roots, &adapter_input, cached_walk)
            }
            None => build_lane_frame_from_adapter_input_ref(mem, roots, &adapter_input),
        };
        // The frame can only have observed what the roots that fed it did. The
        // adapter builds every family it is given input for and cannot tell an
        // empty family from an unobserved one — that distinction lives in the
        // cache, which knows which prewarm sources succeeded, so it is applied
        // here rather than inferred from row counts.
        //
        // The active-ruler family is the exception, and it is decided HERE
        // because it is the only family whose source is not the root cache: it
        // is walked fresh from this frame's country politics roots. A country
        // whose ruler chain could not be read produces no row, and the fold
        // full-replaces `commanders.leader` for every family it is told was
        // observed — so declaring the family observed while one country's read
        // failed would DELETE that country's leader and call it current. The
        // frame says it did not observe the family instead, and the fold keeps
        // the last leader it was told about.
        //
        // A country that has no leader at all is not a failure and does not
        // reach this: `active_ruler_absent_roots` counts those separately.
        adapter_result.frame.known_families = if adapter_result.stats.active_ruler_unresolved_roots
            == 0
        {
            cache.known_families
        } else {
            // The rows go with the bit. A family declared unobserved carries no
            // rows — the decoder refuses a payload that says otherwise, and a
            // refused payload costs the WHOLE d47 hour, census included. So the
            // successfully-read rulers are dropped here rather than emitted
            // under a declaration that disowns them.
            //
            // Losing them is the honest price of a whole-family vocabulary: the
            // mask can say "this family was not observed" and cannot say "these
            // three countries of it were". Retaining the previous hour's
            // leaders for every country beats publishing a set that silently
            // omits the ones that failed.
            adapter_result.frame.active_rulers.clear();
            cache.known_families & !compact::FAMILY_ACTIVE_RULER
        };
        if adapter_result.stats.has_rejections() {
            // An adapter rejection is ROW-level loss inside families this frame
            // did observe. The family-presence mask cannot describe it — it is
            // a whole-family vocabulary — so this frame must stay OUTSIDE the
            // expected-partial tolerance, and `SOURCE_UNAVAILABLE` is what
            // keeps it there.
            //
            // Before per-family admission this branch set `PARTIAL_DATA` alone,
            // which was harmless only because no tolerance covered it. It does
            // now, and a tolerance written for a declared family gap must never
            // absorb an undeclared row loss.
            //
            // The two SIBLING conditions this gate used to test alongside the
            // rejection — a torn root (`cache.skipped_root_rows`) and a skipped
            // country row (`adapter_input.skipped_input_rows`) — are the same
            // row-level loss and publish the same frame, but neither needs the
            // walk to be answered, so both are now decided before it in
            // [`rowless_outcome_is_already_decided`]. Reaching this arm means
            // the walk itself found the loss.
            let mut frame = LaneFrame::new(adapter_input.elapsed_wall_ms);
            frame.status_flags = adapter_input.status_flags
                | compact::STATUS_SOURCE_UNAVAILABLE
                | compact::STATUS_PARTIAL_DATA;
            // The rowless frame carries NO cached families either: a
            // rejection replaces every row, cached ones included.
            (frame, adapter_result.stats, None)
        } else {
            (adapter_result.frame, adapter_result.stats, cached)
        }
    } else {
        (
            rowless_partial_frame(frame_input.elapsed_wall_ms, frame_input.status_flags),
            AdapterBuildStats::default(),
            None,
        )
    }
}

pub fn empty_native_scheduled_read(elapsed_wall_ms: u64, status_flags: u32) -> NativeScheduledRead {
    let mut frame = LaneFrame::new(elapsed_wall_ms);
    frame.status_flags = status_flags;
    let payload = encode_frame_payload(&frame);
    let gate = compact::decode_payload(0, &payload);
    NativeScheduledRead {
        domain_id: DOMAIN_ID,
        domain_name: DOMAIN_NAME,
        payload,
        read_time_us: 0,
        row_count: 0,
        data_units_total: 0,
        status_flags: gate.status_flags_or | frame.status_flags,
        schema_flags: gate.schema_flags_or,
        adapter_stats: AdapterBuildStats::default(),
        parse_errors: gate.parse_errors,
    }
}

pub fn adapter_input_from_cache(
    cache: &NativeRootCache,
    frame_input: NativeFrameInput,
) -> AdapterInput {
    AdapterInput {
        elapsed_wall_ms: frame_input.elapsed_wall_ms,
        status_flags: frame_input.status_flags,
        skipped_input_rows: frame_input.skipped_input_rows,
        characters: cache.characters.clone(),
        unit_leaders: cache.unit_leaders.clone(),
        unit_leader_trait_tokens: cache.unit_leader_trait_tokens.clone(),
        country_leader_roles: cache.country_leader_roles.clone(),
        countries: frame_input.countries,
        advisors: cache.advisors.clone(),
        portraits: cache.portraits.clone(),
    }
}

/// Allocation-free twin of [`adapter_input_from_cache`]: the same field-for-field
/// projection, borrowing the cache's root vectors and the frame's country vector
/// instead of cloning them. `adapter_input_ref_from_cache(c, &f)` is
/// `adapter_input_from_cache(c, f).as_ref()` by construction —
/// `adapter_input_ref_matches_owned_projection` pins that equality.
pub fn adapter_input_ref_from_cache<'a>(
    cache: &'a NativeRootCache,
    frame_input: &'a NativeFrameInput,
) -> AdapterInputRef<'a> {
    AdapterInputRef {
        elapsed_wall_ms: frame_input.elapsed_wall_ms,
        status_flags: frame_input.status_flags,
        skipped_input_rows: frame_input.skipped_input_rows,
        characters: &cache.characters,
        unit_leaders: &cache.unit_leaders,
        unit_leader_trait_tokens: &cache.unit_leader_trait_tokens,
        country_leader_roles: &cache.country_leader_roles,
        countries: &frame_input.countries,
        advisors: &cache.advisors,
        portraits: &cache.portraits,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::adapter::{
        CARMY_LEADER_ATTACK_OFF, CARMY_LEADER_DEFENSE_OFF, CARMY_LEADER_LOGISTICS_OFF,
        CARMY_LEADER_PLANNING_OFF, CUNIT_LEADER_EXPERIENCE_RAW_OFF,
    };
    use crate::extractor::{
        CADVISOR_CHARACTER_PTR_OFF, CCHARACTER_COUNTRY_TAG_DB_ID_OFF, CCHARACTER_ID_OFF,
        CCHARACTER_TYPE_ID, CCHARACTER_TYPE_OFF, CCHARACTER_UNIT_LEADER_PTR_OFF,
        CCOUNTRY_LEADER_CHARACTER_PTR_OFF, CCOUNTRY_POLITICAL_STATUS_OFF,
        CPOLITICAL_PARTY_LEADER_VEC_CAP_OFF, CPOLITICAL_PARTY_LEADER_VEC_OFF,
        CPOLITICAL_PARTY_LEADER_VEC_SIZE_OFF, CPOLITICAL_STATUS_RULING_PARTY_OFF,
        CUNIT_LEADER_CHARACTER_ID_COPY_OFF, CUNIT_LEADER_ID_OFF, CUNIT_LEADER_SKILL_SOURCE_PTR_OFF,
        CUNIT_LEADER_SKILL_VALUE_OFF, CUNIT_LEADER_TRAIT_CAPACITY_OFF, CUNIT_LEADER_TRAIT_DATA_OFF,
        CUNIT_LEADER_TRAIT_SIZE_OFF, CUNIT_LEADER_TYPE_ID, CUNIT_LEADER_TYPE_OFF,
    };

    const CHAR_VT: u64 = 0x1000_1000;
    const ARMY_VT: u64 = 0x1000_2000;
    const NAVY_VT: u64 = 0x1000_3000;
    const COUNTRY_LEADER_VT: u64 = 0x1000_4000;
    const ADVISOR_VT: u64 = 0x1000_5000;
    const TRAIT_VT: u64 = 0x1000_6000;

    #[derive(Clone, Default)]
    struct FakeMemory {
        bytes: BTreeMap<u64, u8>,
    }

    impl FakeMemory {
        fn write_u8(&mut self, addr: u64, value: u8) {
            self.bytes.insert(addr, value);
        }

        fn write_u32(&mut self, addr: u64, value: u32) {
            self.write_bytes(addr, &value.to_le_bytes());
        }

        fn write_i32(&mut self, addr: u64, value: i32) {
            self.write_bytes(addr, &value.to_le_bytes());
        }

        fn write_u64(&mut self, addr: u64, value: u64) {
            self.write_bytes(addr, &value.to_le_bytes());
        }

        fn write_bytes(&mut self, addr: u64, bytes: &[u8]) {
            for (idx, byte) in bytes.iter().enumerate() {
                self.bytes.insert(addr + idx as u64, *byte);
            }
        }
    }

    impl LaneMemory for FakeMemory {
        fn read_u8(&self, addr: u64) -> Option<u8> {
            self.bytes.get(&addr).copied()
        }
    }

    #[test]
    fn scheduled_frame_uses_cached_roots_and_frame_politics() {
        let mut mem = FakeMemory::default();
        let character = 0x1000;
        let leader = 0x2000;
        let skill_source = 0x2100;
        let trait_data = 0x2200;
        let trait_def = 0x2300;
        let country_leader = 0x3000;
        let country = 0x4000;
        let status = 0x4100;
        let party = 0x4200;
        let leader_vec = 0x4300;
        let advisor = 0x5000;

        write_character(&mut mem, character, 1458, 2);
        mem.write_u64(leader, ARMY_VT);
        mem.write_u32(leader + CUNIT_LEADER_TYPE_OFF, CUNIT_LEADER_TYPE_ID);
        mem.write_u32(leader + CUNIT_LEADER_ID_OFF, 500);
        mem.write_u32(leader + CUNIT_LEADER_CHARACTER_ID_COPY_OFF, 1458);
        mem.write_i32(leader + CUNIT_LEADER_EXPERIENCE_RAW_OFF, 42_000);
        mem.write_u64(leader + CUNIT_LEADER_SKILL_SOURCE_PTR_OFF, skill_source);
        mem.write_u32(skill_source + CUNIT_LEADER_SKILL_VALUE_OFF, 4);
        mem.write_u8(leader + CARMY_LEADER_ATTACK_OFF, 6);
        mem.write_u8(leader + CARMY_LEADER_DEFENSE_OFF, 7);
        mem.write_u8(leader + CARMY_LEADER_PLANNING_OFF, 5);
        mem.write_u8(leader + CARMY_LEADER_LOGISTICS_OFF, 8);
        mem.write_u64(leader + CUNIT_LEADER_TRAIT_DATA_OFF, trait_data);
        mem.write_u32(leader + CUNIT_LEADER_TRAIT_CAPACITY_OFF, 1);
        mem.write_u32(leader + CUNIT_LEADER_TRAIT_SIZE_OFF, 1);
        mem.write_u64(trait_data, trait_def);
        mem.write_u64(trait_def, TRAIT_VT);

        mem.write_u64(country_leader, COUNTRY_LEADER_VT);
        mem.write_u64(
            country_leader + CCOUNTRY_LEADER_CHARACTER_PTR_OFF,
            character,
        );
        mem.write_u64(country + CCOUNTRY_POLITICAL_STATUS_OFF, status);
        mem.write_u64(status + CPOLITICAL_STATUS_RULING_PARTY_OFF, party);
        mem.write_u64(party + CPOLITICAL_PARTY_LEADER_VEC_OFF, leader_vec);
        mem.write_u32(party + CPOLITICAL_PARTY_LEADER_VEC_SIZE_OFF, 1);
        mem.write_u32(party + CPOLITICAL_PARTY_LEADER_VEC_CAP_OFF, 1);
        mem.write_u64(leader_vec, country_leader);

        mem.write_u64(advisor, ADVISOR_VT);
        mem.write_u64(advisor + CADVISOR_CHARACTER_PTR_OFF, character);

        let cache = NativeRootCache {
            roots: Some(roots()),
            roots_validated: true,
            skipped_root_rows: 0,
            known_families: compact::FAMILY_ALL,
            characters: vec![CharacterRoot {
                character_addr: character,
                char_id: 1458,
                name: Some("Maxwell Aitken".to_string()),
                token: Some("ENG_maxwell_aitken".to_string()),
            }],
            unit_leaders: vec![UnitLeaderRoot {
                leader_addr: leader,
                country_tag_db_id: 2,
                leader_kind: compact::UNIT_LEADER_ARMY,
                name: Some("Maxwell Aitken".to_string()),
                token: Some("ENG_maxwell_aitken".to_string()),
            }],
            unit_leader_trait_tokens: vec![TraitDefinitionRoot {
                trait_addr: trait_def,
                token: "old_guard".to_string(),
            }],
            country_leader_roles: vec![CountryLeaderRoleRoot {
                country_leader_addr: country_leader,
                char_id: 1458,
                name: Some("Maxwell Aitken".to_string()),
                token: Some("ENG_maxwell_aitken".to_string()),
            }],
            advisors: vec![AdvisorRoot {
                advisor_addr: advisor,
                idea_token: "ENG_maxwell_aitken".to_string(),
                slot: Some("political_advisor".to_string()),
                ledger: None,
            }],
            portraits: vec![CharacterPortraitInput {
                country_tag_db_id: 2,
                char_id: 1458,
                role: "civilian".to_string(),
                size: "large".to_string(),
                gfx: "GFX_portrait_ENG_maxwell_aitken".to_string(),
                gfx_encoding: compact::PortraitIdentityEncoding::Utf8,
            }],
        };
        let read = read_native_scheduled_frame(
            &mem,
            &cache,
            NativeFrameInput {
                elapsed_wall_ms: 88,
                countries: vec![CountryPoliticsRoot {
                    country_addr: country,
                    country_tag_db_id: 2,
                    ruling_party: Some("democratic".to_string()),
                    ruler_name: Some("Maxwell Aitken".to_string()),
                    ruler_token: Some("ENG_maxwell_aitken".to_string()),
                    active_idea_tokens: vec!["ENG_maxwell_aitken".to_string()],
                }],
                ..NativeFrameInput::default()
            },
        );
        assert_eq!(read.domain_id, 47);
        assert_eq!(read.status_flags, 0);
        assert!(read.parse_errors.is_empty(), "{:?}", read.parse_errors);
        assert_eq!(read.row_count, 8);
        assert_eq!(read.adapter_stats.appointed_advisor_rows, 1);
        assert_eq!(read.adapter_stats.portrait_rows, 1);
        let decoded = compact::decode_payload_rows(0, &read.payload).unwrap();
        assert!(decoded.rows.iter().any(|row| matches!(
            row,
            compact::DecodedDomain47Row::AppointedAdvisor {
                country_tag_db_id: 2,
                char_id: 1458,
                idea_token,
                ..
            } if idea_token == "ENG_maxwell_aitken"
        )));
    }

    #[test]
    fn skipped_root_rows_emit_rowless_partial_read() {
        let mut mem = FakeMemory::default();
        let character = 0x1000;
        write_character(&mut mem, character, 1458, 2);

        let cache = NativeRootCache {
            roots: Some(roots()),
            roots_validated: true,
            skipped_root_rows: 1,
            characters: vec![CharacterRoot {
                character_addr: character,
                char_id: 1458,
                name: Some("Maxwell Aitken".to_string()),
                token: Some("ENG_maxwell_aitken".to_string()),
            }],
            ..NativeRootCache::default()
        };

        let read = read_native_scheduled_frame(&mem, &cache, NativeFrameInput::default());
        assert_ne!(read.status_flags & compact::STATUS_PARTIAL_DATA, 0);
        assert_eq!(read.row_count, 0);
        assert_eq!(read.data_units_total, 0);
        let decoded = compact::decode_payload_rows(0, &read.payload).unwrap();
        assert_ne!(decoded.status_flags & compact::STATUS_PARTIAL_DATA, 0);
        // Row-level loss is not a family gap. It must keep the second failure
        // bit that holds it outside d47's expected-partial tolerance, and it
        // must declare no mask — there is nothing it can honestly name.
        assert_ne!(read.status_flags & compact::STATUS_SOURCE_UNAVAILABLE, 0);
        assert_eq!(decoded.family_presence_mask, compact::FAMILY_ALL);
        assert!(decoded.rows.is_empty());
    }

    /// Every engine read the frame walk performs, counted. `LaneMemory::read_u8`
    /// takes `&self` and every other method of the trait funnels through it in
    /// this fixture (`read_array` loops over it, `read_object_window` defaults
    /// to `None`), so this counts the walk's whole appetite for the engine.
    struct CountingMemory {
        inner: FakeMemory,
        reads: std::cell::Cell<u64>,
    }

    impl CountingMemory {
        fn new(inner: FakeMemory) -> Self {
            Self {
                inner,
                reads: std::cell::Cell::new(0),
            }
        }
    }

    impl LaneMemory for CountingMemory {
        fn read_u8(&self, addr: u64) -> Option<u8> {
            self.reads.set(self.reads.get().saturating_add(1));
            self.inner.read_u8(addr)
        }
    }

    /// A rowless outcome that is already decided by the cache and the frame
    /// input costs NO engine reads — and publishes exactly the frame the
    /// post-walk gate published.
    ///
    /// This is the whole of lane GT's fix. In `runs/live_unlock/t49_pricing`
    /// `lane_wall.hour_boundary_position.brackets_closed_before` p50 is 0 over
    /// 325 boundary frames, so at least 163 of that capture's 334 extracted
    /// frames poisoned these roots before the fan-out even opened — every one
    /// of them committed to a rowless d47 hour before any worker started. How
    /// many of those still held stable roots (and so actually walked), and what
    /// a walk costs on a frame whose records are stripped, are both absent from
    /// that capture's wire — see `rowless_outcome_is_already_decided`.
    ///
    /// kills: delete the `rowless_outcome_is_already_decided` arm from
    /// `read_native_scheduled_frame` (restoring the post-walk-only gate). The
    /// wire assertions all still pass — that is the point, the wire is
    /// unchanged — and the read counters go nonzero on both refusal cases.
    #[test]
    fn a_predecided_rowless_frame_reads_no_engine_memory() {
        let cache_with = |skipped_root_rows: u32, character: u64| NativeRootCache {
            roots: Some(roots()),
            roots_validated: true,
            skipped_root_rows,
            characters: vec![CharacterRoot {
                character_addr: character,
                char_id: 1458,
                name: Some("Maxwell Aitken".to_string()),
                token: Some("ENG_maxwell_aitken".to_string()),
            }],
            ..NativeRootCache::default()
        };
        let character = 0x1000;
        let memory = || {
            let mut mem = FakeMemory::default();
            write_character(&mut mem, character, 1458, 2);
            CountingMemory::new(mem)
        };
        // The frame both refusals publish, built by the one constructor every
        // caller of this module already uses for a rowless d47 hour.
        let expected = empty_native_scheduled_read(
            0,
            compact::STATUS_SOURCE_UNAVAILABLE | compact::STATUS_PARTIAL_DATA,
        );

        for (case, cache, frame_input) in [
            (
                "torn or clock-refused roots",
                cache_with(1, character),
                NativeFrameInput::default(),
            ),
            (
                "a skipped country row",
                cache_with(0, character),
                NativeFrameInput {
                    skipped_input_rows: 1,
                    ..NativeFrameInput::default()
                },
            ),
        ] {
            let mem = memory();
            let read = read_native_scheduled_frame(&mem, &cache, frame_input);
            assert_eq!(
                mem.reads.get(),
                0,
                "{case}: the walk ran for a frame whose rowless outcome was already decided"
            );
            // ...and the hour is published exactly as it was before, so the
            // saving costs the wire nothing.
            assert_eq!(read.payload, expected.payload, "{case}");
            assert_eq!(read.status_flags, expected.status_flags, "{case}");
            assert_eq!(read.row_count, 0, "{case}");
            assert_eq!(read.data_units_total, 0, "{case}");
            assert_eq!(read.schema_flags, expected.schema_flags, "{case}");
            assert_ne!(read.status_flags & compact::STATUS_PARTIAL_DATA, 0, "{case}");
            assert_ne!(
                read.status_flags & compact::STATUS_SOURCE_UNAVAILABLE,
                0,
                "{case}"
            );
        }

        // THE CONTROL. The same roots and the same memory, with neither
        // refusal set, still walk — a gate that swallowed the ordinary frame
        // would pass every assertion above.
        let mem = memory();
        let read = read_native_scheduled_frame(&mem, &cache_with(0, character), NativeFrameInput::default());
        assert_ne!(
            mem.reads.get(),
            0,
            "the ordinary frame must still read the engine"
        );
        assert_ne!(read.row_count, 0, "the ordinary frame must still publish");
    }

    /// A recycled CHARACTER must cost its own text and nothing else — checked
    /// at the SCHEDULED layer, which is where the cost is decided.
    ///
    /// The adapter-level tests cannot see this: `read_native_scheduled_frame`
    /// throws away the adapter's whole frame and substitutes a rowless one
    /// whenever `has_rejections()` holds, so an adapter that "degrades" a row
    /// and a wrapper that discards every row are indistinguishable one layer
    /// down. Codex 019fd6ea, finding 1: the first version of this change
    /// routed the degradation through `has_rejections`, and one recycled
    /// character out of thousands emptied the entire domain-47 frame.
    ///
    /// MUTATION: add `identity_degraded_rows` to `AdapterBuildStats::
    /// has_rejections` and this fails with `row_count == 0`.
    #[test]
    fn a_recycled_character_costs_its_own_text_and_not_the_frame() {
        let mut mem = FakeMemory::default();
        let character = 0x1000;
        // The census read char 1458 here; a different character occupies the
        // storage now.
        write_character(&mut mem, character, 9999, 2);

        let cache = NativeRootCache {
            roots: Some(roots()),
            roots_validated: true,
            skipped_root_rows: 0,
            characters: vec![CharacterRoot {
                character_addr: character,
                char_id: 1458,
                name: Some("Maxwell Aitken".to_string()),
                token: Some("ENG_maxwell_aitken".to_string()),
            }],
            ..NativeRootCache::default()
        };

        let read = read_native_scheduled_frame(&mem, &cache, NativeFrameInput::default());
        // The frame survives, and it carries the character that is actually
        // there.
        assert_ne!(read.row_count, 0, "the frame was discarded, not degraded");
        let decoded = compact::decode_payload_rows(0, &read.payload).unwrap();
        assert!(decoded.rows.iter().any(|row| matches!(
            row,
            compact::DecodedDomain47Row::Character {
                char_id: 9999,
                country_tag_db_id: 2,
                name: None,
                token: None,
            }
        )));
        // The census's text reached nothing.
        assert!(!read.payload.windows(14).any(|w| w == b"Maxwell Aitken"));
        // Declared, not silent.
        assert_ne!(read.status_flags & compact::STATUS_PARTIAL_DATA, 0);
    }

    /// A cache built from the character manager census while the heap sweep was
    /// still cold publishes its characters, and says on the wire which families
    /// it never looked at.
    ///
    /// This is the frame the whole change exists to produce. Before it, the
    /// identical cache state published a rowless
    /// `SOURCE_UNAVAILABLE|PARTIAL_DATA` record for every hour of a capture's
    /// first ~12 seconds.
    ///
    /// MUTATION: drop the `adapter_result.frame.known_families = cache.known_families`
    /// assignment in `read_native_scheduled_frame` and the payload claims to
    /// have observed the advisor and role families it never read.
    #[test]
    fn a_census_only_cache_publishes_its_characters_and_declares_the_rest_unknown() {
        let mut mem = FakeMemory::default();
        let character = 0x1000;
        write_character(&mut mem, character, 1458, 2);

        let cache = NativeRootCache {
            roots: Some(roots()),
            roots_validated: true,
            skipped_root_rows: 0,
            known_families: compact::FAMILY_CENSUS_ONLY,
            characters: vec![CharacterRoot {
                character_addr: character,
                char_id: 1458,
                name: Some("Maxwell Aitken".to_string()),
                token: Some("ENG_maxwell_aitken".to_string()),
            }],
            // The sweep-derived families are EMPTY because they were never
            // read, which is exactly the state the mask exists to describe.
            ..NativeRootCache::default()
        };

        // A real country politics root, so the bridge is available and the
        // status word is decided by the family gap alone. An empty `countries`
        // vector would set `BRIDGE_UNAVAILABLE` and the "only PARTIAL"
        // assertion below would be passing for the wrong reason.
        let country = 0x4000;
        let status = 0x4100;
        let party = 0x4200;
        let leader_vec = 0x4300;
        let country_leader = 0x4400;
        mem.write_u64(country_leader, COUNTRY_LEADER_VT);
        mem.write_u64(
            country_leader + CCOUNTRY_LEADER_CHARACTER_PTR_OFF,
            character,
        );
        mem.write_u64(country + CCOUNTRY_POLITICAL_STATUS_OFF, status);
        mem.write_u64(status + CPOLITICAL_STATUS_RULING_PARTY_OFF, party);
        mem.write_u64(party + CPOLITICAL_PARTY_LEADER_VEC_OFF, leader_vec);
        mem.write_u32(party + CPOLITICAL_PARTY_LEADER_VEC_SIZE_OFF, 1);
        mem.write_u32(party + CPOLITICAL_PARTY_LEADER_VEC_CAP_OFF, 1);
        mem.write_u64(leader_vec, country_leader);
        let read = read_native_scheduled_frame(
            &mem,
            &cache,
            NativeFrameInput {
                countries: vec![CountryPoliticsRoot {
                    country_addr: country,
                    country_tag_db_id: 2,
                    ruling_party: Some("democratic".to_string()),
                    ruler_name: Some("Maxwell Aitken".to_string()),
                    ruler_token: Some("ENG_maxwell_aitken".to_string()),
                    active_idea_tokens: vec![],
                }],
                ..NativeFrameInput::default()
            },
        );
        assert!(read.parse_errors.is_empty(), "{:?}", read.parse_errors);
        assert_ne!(read.row_count, 0, "the character census must be published");

        // PARTIAL and ONLY partial, against the SHARED failure mask rather than
        // a list of bits written here. Any other failure-class bit would take
        // this record out of d47's expected-partial tolerance and the fold
        // would discard the census this whole change exists to deliver — and a
        // hand-written list would not notice a bit added to the vocabulary
        // later.
        assert_eq!(
            read.status_flags & compact_contract::COMPACT_STATUS_FAILURE_MASK,
            compact::STATUS_PARTIAL_DATA
        );

        let decoded = compact::decode_payload_rows(0, &read.payload).unwrap();
        assert_eq!(decoded.family_presence_mask, compact::FAMILY_CENSUS_ONLY);
        assert_eq!(
            compact::absent_family_names(decoded.family_presence_mask),
            vec![
                "unit_leader_traits",
                "country_leader_roles",
                "advisor_roster",
                "appointed_advisors"
            ]
        );
        assert!(decoded.rows.iter().any(|row| matches!(
            row,
            compact::DecodedDomain47Row::Character { char_id: 1458, .. }
        )));
        assert!(
            !decoded.rows.iter().any(|row| matches!(
                row,
                compact::DecodedDomain47Row::AdvisorRoster { .. }
                    | compact::DecodedDomain47Row::CountryLeaderRole { .. }
                    | compact::DecodedDomain47Row::UnitLeaderTrait { .. }
            )),
            "a family declared unobserved must carry no rows"
        );
    }

    /// One country's ruler resolves and another's does not: the active-ruler
    /// family is declared unobserved, its rows go with the declaration, and the
    /// rest of the payload still publishes.
    ///
    /// This is the shape that made the first version of the fix worse than the
    /// bug. Clearing `FAMILY_ACTIVE_RULER` while leaving the successful rows in
    /// the frame produced a payload the canonical decoder refuses — rows for a
    /// declared-unobserved family — and the collector replaces a refused
    /// payload with a rowless `SOURCE_UNAVAILABLE|PARTIAL_DATA` record, losing
    /// the entire d47 hour including the character census.
    ///
    /// MUTATION: delete the `active_rulers.clear()` in
    /// `read_native_scheduled_frame` and this test fails on `parse_errors`.
    #[test]
    fn a_partly_unresolved_ruler_walk_drops_the_family_and_still_publishes() {
        let mut mem = FakeMemory::default();
        let character = 0x1000;
        write_character(&mut mem, character, 1458, 2);

        // A country whose whole chain reads.
        let good_country = 0x4000;
        let status = 0x4100;
        let party = 0x4200;
        let leader_vec = 0x4300;
        let country_leader = 0x4400;
        mem.write_u64(country_leader, COUNTRY_LEADER_VT);
        mem.write_u64(
            country_leader + CCOUNTRY_LEADER_CHARACTER_PTR_OFF,
            character,
        );
        mem.write_u64(good_country + CCOUNTRY_POLITICAL_STATUS_OFF, status);
        mem.write_u64(status + CPOLITICAL_STATUS_RULING_PARTY_OFF, party);
        mem.write_u64(party + CPOLITICAL_PARTY_LEADER_VEC_OFF, leader_vec);
        mem.write_u32(party + CPOLITICAL_PARTY_LEADER_VEC_SIZE_OFF, 1);
        mem.write_u32(party + CPOLITICAL_PARTY_LEADER_VEC_CAP_OFF, 1);
        mem.write_u64(leader_vec, country_leader);

        // And one whose political-status pointer is unreadable.
        let unreadable_country = 0x5000;

        let cache = NativeRootCache {
            roots: Some(roots()),
            roots_validated: true,
            known_families: compact::FAMILY_ALL,
            characters: vec![CharacterRoot {
                character_addr: character,
                char_id: 1458,
                name: Some("Maxwell Aitken".to_string()),
                token: Some("ENG_maxwell_aitken".to_string()),
            }],
            ..NativeRootCache::default()
        };
        let politics = |addr: u64, tag: u32| CountryPoliticsRoot {
            country_addr: addr,
            country_tag_db_id: tag,
            ruling_party: Some("democratic".to_string()),
            ruler_name: Some("Maxwell Aitken".to_string()),
            ruler_token: Some("ENG_maxwell_aitken".to_string()),
            active_idea_tokens: vec![],
        };
        let read = read_native_scheduled_frame(
            &mem,
            &cache,
            NativeFrameInput {
                countries: vec![politics(good_country, 2), politics(unreadable_country, 3)],
                ..NativeFrameInput::default()
            },
        );

        // The payload is VALID. Before the rows were dropped alongside the bit,
        // this was a decoder refusal and the whole hour was lost.
        assert!(read.parse_errors.is_empty(), "{:?}", read.parse_errors);
        assert_eq!(read.adapter_stats.active_ruler_unresolved_roots, 1);

        let decoded = compact::decode_payload_rows(0, &read.payload).unwrap();
        assert_eq!(
            decoded.family_presence_mask & compact::FAMILY_ACTIVE_RULER,
            0,
            "a family with an unreadable member is not observed"
        );
        assert!(
            !decoded
                .rows
                .iter()
                .any(|row| matches!(row, compact::DecodedDomain47Row::ActiveRuler { .. })),
            "the rows must go with the declaration"
        );
        // The census still publishes: that is the whole point.
        assert!(decoded.rows.iter().any(|row| matches!(
            row,
            compact::DecodedDomain47Row::Character { char_id: 1458, .. }
        )));
        assert_eq!(
            read.status_flags & compact_contract::COMPACT_STATUS_FAILURE_MASK,
            compact::STATUS_PARTIAL_DATA
        );
    }

    /// Fixture with EVERY root family populated, so an equality assertion over
    /// the adapter input cannot pass vacuously.
    fn borrowed_input_fixture() -> (FakeMemory, NativeRootCache, NativeFrameInput) {
        let mut mem = FakeMemory::default();
        let character = 0x1000;
        let leader = 0x2000;
        let skill_source = 0x2100;
        let trait_data = 0x2200;
        let trait_def = 0x2300;
        let country_leader = 0x3000;
        let country = 0x4000;
        let advisor = 0x5000;

        write_character(&mut mem, character, 1458, 2);
        mem.write_u64(leader, ARMY_VT);
        mem.write_u32(leader + CUNIT_LEADER_TYPE_OFF, CUNIT_LEADER_TYPE_ID);
        mem.write_u32(leader + CUNIT_LEADER_ID_OFF, 500);
        mem.write_u32(leader + CUNIT_LEADER_CHARACTER_ID_COPY_OFF, 1458);
        mem.write_i32(leader + CUNIT_LEADER_EXPERIENCE_RAW_OFF, 42_000);
        mem.write_u64(leader + CUNIT_LEADER_SKILL_SOURCE_PTR_OFF, skill_source);
        mem.write_u32(skill_source + CUNIT_LEADER_SKILL_VALUE_OFF, 4);
        mem.write_u8(leader + CARMY_LEADER_ATTACK_OFF, 6);
        mem.write_u8(leader + CARMY_LEADER_DEFENSE_OFF, 7);
        mem.write_u8(leader + CARMY_LEADER_PLANNING_OFF, 5);
        mem.write_u8(leader + CARMY_LEADER_LOGISTICS_OFF, 8);
        mem.write_u64(leader + CUNIT_LEADER_TRAIT_DATA_OFF, trait_data);
        mem.write_u32(leader + CUNIT_LEADER_TRAIT_CAPACITY_OFF, 1);
        mem.write_u32(leader + CUNIT_LEADER_TRAIT_SIZE_OFF, 1);
        mem.write_u64(trait_data, trait_def);
        mem.write_u64(trait_def, TRAIT_VT);
        mem.write_u64(country_leader, COUNTRY_LEADER_VT);
        mem.write_u64(country_leader + CCOUNTRY_LEADER_CHARACTER_PTR_OFF, character);
        mem.write_u64(advisor, ADVISOR_VT);
        mem.write_u64(advisor + CADVISOR_CHARACTER_PTR_OFF, character);

        let cache = NativeRootCache {
            roots: Some(roots()),
            roots_validated: true,
            skipped_root_rows: 0,
            known_families: compact::FAMILY_ALL,
            characters: vec![CharacterRoot {
                character_addr: character,
                char_id: 1458,
                name: Some("Maxwell Aitken".to_string()),
                token: Some("ENG_maxwell_aitken".to_string()),
            }],
            unit_leaders: vec![UnitLeaderRoot {
                leader_addr: leader,
                country_tag_db_id: 2,
                leader_kind: compact::UNIT_LEADER_ARMY,
                name: Some("Maxwell Aitken".to_string()),
                token: Some("ENG_maxwell_aitken".to_string()),
            }],
            unit_leader_trait_tokens: vec![TraitDefinitionRoot {
                trait_addr: trait_def,
                token: "old_guard".to_string(),
            }],
            country_leader_roles: vec![CountryLeaderRoleRoot {
                country_leader_addr: country_leader,
                char_id: 1458,
                name: Some("Maxwell Aitken".to_string()),
                token: Some("ENG_maxwell_aitken".to_string()),
            }],
            advisors: vec![AdvisorRoot {
                advisor_addr: advisor,
                idea_token: "ENG_maxwell_aitken".to_string(),
                slot: Some("political_advisor".to_string()),
                ledger: None,
            }],
            portraits: vec![CharacterPortraitInput {
                country_tag_db_id: 2,
                char_id: 1458,
                role: "civilian".to_string(),
                size: "large".to_string(),
                gfx: "GFX_portrait_ENG_maxwell_aitken".to_string(),
                gfx_encoding: compact::PortraitIdentityEncoding::Utf8,
            }],
        };
        let frame_input = NativeFrameInput {
            elapsed_wall_ms: 88,
            status_flags: 0,
            skipped_input_rows: 3,
            countries: vec![CountryPoliticsRoot {
                country_addr: country,
                country_tag_db_id: 2,
                ruling_party: Some("democratic".to_string()),
                ruler_name: Some("Maxwell Aitken".to_string()),
                ruler_token: Some("ENG_maxwell_aitken".to_string()),
                active_idea_tokens: vec!["ENG_maxwell_aitken".to_string()],
            }],
        };
        (mem, cache, frame_input)
    }

    /// The borrowed projection must be the owned projection, field for field.
    ///
    /// MUTATION: point any field of `adapter_input_ref_from_cache` at a
    /// different cache vector (e.g. `advisors: &cache.portraits`… or simply
    /// swap `characters` and `country_leader_roles`), or drop
    /// `skipped_input_rows` — this equality fails.
    #[test]
    fn adapter_input_ref_matches_owned_projection() {
        let (_mem, cache, frame_input) = borrowed_input_fixture();
        let owned = adapter_input_from_cache(&cache, frame_input.clone());
        let borrowed = adapter_input_ref_from_cache(&cache, &frame_input);
        assert_eq!(borrowed, owned.as_ref());
        // Non-vacuity: every root family carries at least one row.
        assert!(!borrowed.characters.is_empty());
        assert!(!borrowed.unit_leaders.is_empty());
        assert!(!borrowed.unit_leader_trait_tokens.is_empty());
        assert!(!borrowed.country_leader_roles.is_empty());
        assert!(!borrowed.countries.is_empty());
        assert!(!borrowed.advisors.is_empty());
        assert!(!borrowed.portraits.is_empty());
        assert_eq!(borrowed.skipped_input_rows, 3);
    }

    /// End-to-end: the scheduled read (which now BORROWS the root cache) must
    /// produce the byte-identical payload, row count and adapter stats that the
    /// owned clone-then-build path produces.
    ///
    /// SCOPE, stated honestly: `build_lane_frame_from_adapter_input` DELEGATES
    /// to the `_ref` builder, so this test cannot detect a change inside the
    /// shared builder body — that is what the pre-existing adapter tests cover.
    /// What it does pin is everything the borrowed path does DIFFERENTLY: the
    /// `adapter_input_ref_from_cache` projection and the PARTIAL/clean wrapping
    /// in `read_native_scheduled_frame` that consumes it.
    ///
    /// MUTATION: make `adapter_input_ref_from_cache` hand the builder an empty
    /// `countries` slice (or any wrong/missing root family), or drop the
    /// `skipped_input_rows` disjunct from `rowless_outcome_is_already_decided`
    /// (the pre-walk gate the scheduled reader's `skipped_input_rows` arm moved
    /// into) — the payload diverges and this fails.
    ///
    /// ADAPTER STATS ARE COMPARED ONLY WHERE THERE IS A WALK. A frame whose
    /// rowless outcome is decided before the walk does not run one, so it has
    /// no stats to agree with the owned build's; it reports the default, and
    /// the owned build here is this test's own instrument rather than anything
    /// production performs on that path. The payload, the status word and the
    /// row count — everything that reaches the wire — are compared on both
    /// arms.
    /// Lane MT: a frame that takes the cacheable families from the adopting
    /// frame's walk publishes the SAME payload as a frame that walks them,
    /// row for row and byte for byte, over the fixture memory (which does not
    /// change between the two, exactly as a validated witness proves for the
    /// engine); its stats agree on every counted field; and the reuse is
    /// visible in `reused_walk_families`.
    ///
    /// kills: a cached family the reused walk ALSO walked (double rows), a
    /// stats field not copied from the cache, portraits admitted twice or not
    /// at all, appointed advisors computed over an empty roster, the ruler
    /// backfill missing the cached identity index.
    #[test]
    fn reused_walk_matches_full_walk() {
        let (mem, cache, frame_input) = borrowed_input_fixture();
        let mut input = frame_input;
        input.skipped_input_rows = 0;
        let input_again = input.clone();
        let plain = observe_native_scheduled_frame(&mem, &cache, input.clone());
        assert_eq!(plain.reused_walk_families(), 0);
        let plain_read = finish_native_scheduled_read(plain.clone());
        // The adopting frame's walk, cached for every cacheable family (the
        // collector decides the mask from the witness; here it is total). The
        // adopting observation itself now encodes over the cache ...
        let mut full = plain.clone();
        let cached = full
            .adopt_cached_walk(CACHEABLE_WALK_FAMILIES)
            .expect("the fixture walk is clean, so it caches");
        assert_eq!(cached.families, CACHEABLE_WALK_FAMILIES);
        assert!(!cached.characters.is_empty(), "fixture has characters");
        assert!(full.frame.characters.is_empty(), "moved, not cloned");
        assert_eq!(full.reused_walk_families(), CACHEABLE_WALK_FAMILIES);
        // ... and publishes the plain walk's payload byte for byte.
        assert_eq!(finish_native_scheduled_read(full.clone()).payload, plain_read.payload);
        let reused = observe_native_scheduled_frame_reusing(&mem, &cache, input, Some(cached.clone()));
        assert_eq!(reused.reused_walk_families(), CACHEABLE_WALK_FAMILIES);
        // The reused frame's own vectors for the cached families are empty ...
        assert!(reused.frame.characters.is_empty());
        assert!(reused.frame.advisor_roster.is_empty());
        assert!(reused.frame.country_leader_roles.is_empty());
        assert!(reused.frame.unit_leaders.is_empty());
        // ... and the payload is the full walk's, byte for byte.
        let full_read = finish_native_scheduled_read(full.clone());
        let reused_read = finish_native_scheduled_read(reused.clone());
        assert_eq!(reused_read.payload, full_read.payload);
        assert_eq!(reused_read.row_count, full_read.row_count);
        assert_eq!(reused_read.status_flags, full_read.status_flags);
        assert!(reused_read.parse_errors.is_empty());
        let without_timings = |mut stats: AdapterBuildStats| {
            stats.walk_characters_us = 0;
            stats.walk_unit_leaders_us = 0;
            stats.walk_country_leader_roles_us = 0;
            stats.walk_active_rulers_us = 0;
            stats.walk_advisors_us = 0;
            stats.walk_appointed_advisors_us = 0;
            stats.walk_portraits_us = 0;
            stats
        };
        assert_eq!(without_timings(reused_read.adapter_stats), without_timings(full_read.adapter_stats));
        // A cached walk that serves nothing is no cache: the frame walks.
        let none = observe_native_scheduled_frame_reusing(
            &mem,
            &cache,
            input_again.clone(),
            Some(Arc::new(CachedWalk::default())),
        );
        assert_eq!(none.reused_walk_families(), 0);
        // A PARTIAL mask (advisor roster only): live families' keys then admit
        // portraits the cache does not know, and the wire order must still be
        // the plain frame's (Codex 01a00f20 round 2).
        let mut partial = plain.clone();
        let cached_partial = partial
            .adopt_cached_walk(compact::FAMILY_ADVISOR_ROSTER)
            .expect("advisor roster caches");
        assert_eq!(cached_partial.families, compact::FAMILY_ADVISOR_ROSTER);
        assert_eq!(finish_native_scheduled_read(partial.clone()).payload, plain_read.payload);
        let reused_partial = observe_native_scheduled_frame_reusing(
            &mem,
            &cache,
            input_again.clone(),
            Some(cached_partial),
        );
        assert_eq!(finish_native_scheduled_read(reused_partial).payload, plain_read.payload);
        // THE LIVE-ONLY FALLBACK (Codex 01a00f20 rounds 3-4), on a CONSISTENT
        // two-character world: a second character (7,7) censused and appointed
        // through a second advisor, and no advisor for (2,1458). Portrait inputs in order [(2,1458), (7,7)]. With the advisor
        // roster cached, the cache knows (7,7) only; (2,1458)'s portrait is
        // admitted by the LIVE character family, so both the adopting frame
        // and the reused frame must build the whole family in input order.
        //
        // kills: dropping the whole-family fallback leaves the reused frame
        // with the one live-only extra and the encoder prepending the cached
        // row: order [(7,7), (2,1458)], payload != plain.
        let mut mem2 = mem.clone();
        write_character(&mut mem2, 0x6000, 7, 7);
        mem2.write_u64(0x7000, ADVISOR_VT);
        mem2.write_u64(0x7000 + CADVISOR_CHARACTER_PTR_OFF, 0x6000);
        let mut cache_two = cache.clone();
        // (7,7) is a censused character too (Codex round 5: an advisor without
        // a character root would be an orphan the fold rejects); the cache
        // mask below is advisor-only, so (2,1458) is still admitted by the
        // LIVE character family alone.
        cache_two.characters.push(CharacterRoot {
            character_addr: 0x6000,
            char_id: 7,
            name: Some("Other".to_string()),
            token: Some("OTHER_other".to_string()),
        });
        cache_two.advisors = vec![AdvisorRoot {
            advisor_addr: 0x7000,
            idea_token: "OTHER_theorist".to_string(),
            slot: Some("theorist".to_string()),
            ledger: None,
        }];
        cache_two.portraits.push(CharacterPortraitInput {
            country_tag_db_id: 7,
            char_id: 7,
            role: "civilian".to_string(),
            size: "large".to_string(),
            gfx: "GFX_portrait_other".to_string(),
            gfx_encoding: compact::PortraitIdentityEncoding::Utf8,
        });
        let plain_two = observe_native_scheduled_frame(&mem2, &cache_two, input_again.clone());
        let plain_two_read = finish_native_scheduled_read(plain_two.clone());
        assert_eq!(plain_two.frame.character_portraits.len(), 2, "both admitted");
        let mut adopting_two = plain_two.clone();
        let cached_two = adopting_two
            .adopt_cached_walk(compact::FAMILY_ADVISOR_ROSTER)
            .expect("advisor roster caches");
        assert_eq!(cached_two.known_keys, vec![(7, 7)]);
        assert_eq!(cached_two.character_portraits.len(), 1, "the cached-key subset");
        assert_eq!(adopting_two.frame.character_portraits.len(), 2, "kept whole: a live-only admission");
        assert_eq!(finish_native_scheduled_read(adopting_two).payload, plain_two_read.payload);
        let fallback = observe_native_scheduled_frame_reusing(
            &mem2,
            &cache_two,
            input_again.clone(),
            Some(cached_two),
        );
        let keys: Vec<(u32, u32)> = fallback
            .frame
            .character_portraits
            .iter()
            .map(|p| (p.country_tag_db_id, p.char_id))
            .collect();
        assert_eq!(keys, vec![(2, 1458), (7, 7)], "whole family, input order");
        let fallback_read = finish_native_scheduled_read(fallback);
        assert_eq!(fallback_read.payload, plain_two_read.payload);
        assert_eq!(fallback_read.adapter_stats.portrait_rows, 2);
        // A cache built from a reused observation is refused (no chaining).
        assert!(reused.clone().adopt_cached_walk(CACHEABLE_WALK_FAMILIES).is_none());
        // Only the leader pair together: a mask naming one half caches neither.
        let half = plain
            .clone()
            .adopt_cached_walk(compact::FAMILY_CHARACTER | compact::FAMILY_UNIT_LEADER)
            .expect("characters alone cache");
        assert_eq!(half.families, compact::FAMILY_CHARACTER);
        assert!(half.unit_leaders.is_empty());
    }

    #[test]
    fn borrowed_scheduled_read_matches_owned_build() {
        let (mem, cache, frame_input) = borrowed_input_fixture();
        // The fixture deliberately carries skipped_input_rows, which makes the
        // scheduled reader emit the rowless-PARTIAL frame; compare the CLEAN
        // build too so the row-bearing path is covered.
        let mut clean_input = frame_input.clone();
        clean_input.skipped_input_rows = 0;

        for input in [frame_input, clean_input] {
            let read = read_native_scheduled_frame(&mem, &cache, input.clone());
            let owned = adapter_input_from_cache(&cache, input);
            let owned_result = crate::adapter::build_lane_frame_from_adapter_input(
                &mem,
                &cache.roots.expect("fixture has roots"),
                &owned,
            );
            let decided_before_the_walk =
                cache.skipped_root_rows != 0 || owned.skipped_input_rows != 0;
            let expected_frame = if decided_before_the_walk || owned_result.stats.has_rejections() {
                let mut frame = LaneFrame::new(owned.elapsed_wall_ms);
                // Row-level loss inside an observed family is not a family gap,
                // so it must stay outside d47's expected-partial tolerance;
                // `SOURCE_UNAVAILABLE` is what holds it there.
                frame.status_flags = owned.status_flags
                    | compact::STATUS_SOURCE_UNAVAILABLE
                    | compact::STATUS_PARTIAL_DATA;
                frame
            } else {
                let mut frame = owned_result.frame.clone();
                // The fixture's country has no readable political-status chain,
                // so its ruler is unresolved and the active-ruler family cannot
                // be declared observed. Mirrored here rather than hard-coded so
                // the rule, not this fixture's shape, is what is being pinned.
                frame.known_families = if owned_result.stats.active_ruler_unresolved_roots == 0 {
                    cache.known_families
                } else {
                    cache.known_families & !compact::FAMILY_ACTIVE_RULER
                };
                frame
            };
            assert_eq!(
                read.payload,
                encode_frame_payload(&expected_frame),
                "payload must be byte-identical to the owned build"
            );
            // The walk's per-family spans (lane MT) are timings and differ
            // between two runs; every counted field must still agree.
            let without_timings = |mut stats: AdapterBuildStats| {
                stats.walk_characters_us = 0;
                stats.walk_unit_leaders_us = 0;
                stats.walk_country_leader_roles_us = 0;
                stats.walk_active_rulers_us = 0;
                stats.walk_advisors_us = 0;
                stats.walk_appointed_advisors_us = 0;
                stats.walk_portraits_us = 0;
                stats
            };
            assert_eq!(
                without_timings(read.adapter_stats),
                if decided_before_the_walk {
                    AdapterBuildStats::default()
                } else {
                    without_timings(owned_result.stats)
                }
            );
            assert_eq!(read.status_flags, {
                let gate = compact::decode_payload(0, &read.payload);
                gate.status_flags_or | expected_frame.status_flags
            });
        }
    }

    /// Portrait roots are roots. Before the portrait feed existed they were
    /// the one root list the shared collector was forbidden to populate, and a
    /// cache holding nothing else was still "no stable roots"; now a prewarm
    /// that read portraits has read something, and `has_stable_roots` has to
    /// agree or the adoption gate discards the census that produced them.
    #[test]
    fn portrait_roots_count_toward_the_adoptable_root_total() {
        let mut cache = NativeRootCache {
            roots: Some(roots()),
            roots_validated: true,
            ..NativeRootCache::default()
        };
        assert_eq!(cache.stable_root_count(), 0);
        assert!(!cache.has_stable_roots());
        cache.portraits.push(CharacterPortraitInput {
            country_tag_db_id: 2,
            char_id: 1458,
            role: "civilian".to_string(),
            size: "large".to_string(),
            gfx: "GFX_portrait_ENG_maxwell_aitken".to_string(),
            gfx_encoding: compact::PortraitIdentityEncoding::Utf8,
        });
        assert_eq!(cache.stable_root_count(), 1);
        assert!(cache.has_stable_roots());
    }

    fn roots() -> ResolvedTypeRoots {
        ResolvedTypeRoots {
            ccharacter_vtable: CHAR_VT,
            carmy_leader_vtable: ARMY_VT,
            cnavy_leader_vtable: NAVY_VT,
            ccountry_leader_vtable: COUNTRY_LEADER_VT,
            cadvisor_vtable: ADVISOR_VT,
            cunit_leader_trait_vtable: TRAIT_VT,
        }
    }

    fn write_character(mem: &mut FakeMemory, addr: u64, char_id: u32, tag_db_id: u32) {
        mem.write_u64(addr, CHAR_VT);
        mem.write_u32(addr + CCHARACTER_TYPE_OFF, CCHARACTER_TYPE_ID);
        mem.write_u32(addr + CCHARACTER_ID_OFF, char_id);
        mem.write_u32(addr + CCHARACTER_COUNTRY_TAG_DB_ID_OFF, tag_db_id);
        mem.write_u64(addr + CCHARACTER_UNIT_LEADER_PTR_OFF, 0x2000);
    }
}
