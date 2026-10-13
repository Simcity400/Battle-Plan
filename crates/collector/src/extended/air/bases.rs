use super::*;
use engine::allocation;

impl Capacities {
    /// Attach/cadence floors from native populations. These count-only walks
    /// supply no sampled values; acquisitions retain their original headers.
    pub(crate) fn presize(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
    ) -> Result<(), &'static str> {
        let mut failure = None;
        let mut raise = |index: usize, result: Result<usize, &'static str>| match result {
            Ok(count) => self.rows[index] = self.rows[index].max(count),
            Err(reason) => {
                failure.get_or_insert(reason);
            }
        };
        raise(
            1,
            crate::country_scale::table_demand::rows(cair_base::walk(memory, base, state, &mut [])),
        );
        let aces = cace::walk(memory, base, state, &mut [], &mut []);
        raise(3, crate::country_scale::table_demand::rows(aces.outcome));
        self.ace_names = self.ace_names.max(aces.required_bytes);
        raise(
            4,
            crate::country_scale::table_demand::rows(cloop_history::walk(
                memory,
                base,
                state,
                &mut [],
            )),
        );
        if let Err(reason) = self.presize_missions(memory, base, state) {
            failure.get_or_insert(reason);
        }
        if let Err(reason) = self.sorties.presize(memory, base, state) {
            failure.get_or_insert(reason);
        }
        failure.map_or(Ok(()), Err)
    }
}

fn reserve_to<T>(rows: &mut Vec<T>, len: usize) -> std::io::Result<()> {
    let additional = len.saturating_sub(rows.len());
    allocation::reserve(rows, additional)
}

// ===========================================================================
// Hourly shared bases/registrations/aces split (AirBases,
// AirBaseRegistrations, AirAces).
//
// Acquisition workers copy raw bytes: one authenticated country census,
// the strategic-air manager with its four base vectors, per-base identity
// and host/state/province fields, per-base country containers with pool and
// wing vectors, and per-country ace rosters with ace objects and name bytes.
// Shared spans (country census, manager, vectors, base identities) are taken
// ONCE with the combined mask and serve every dependent replay: Recorded
// spans are address-keyed with last-value repeat, so one census walk feeds
// the bases, registrations and aces decodes with no duplicated copy of the
// walk. The game thread parses only copied identity/field inputs for the fixed
// optional carrier observer; replay projections remain off-thread. The FIFO decode worker
// replays `cair_base::walk`, `cair_base::walk_registrations` and `cace::walk`
// against the recorded spans, so rows, ordinals, skips, truncation counts
// and failure reasons are identical by construction (see `crate::raw` for
// the mirror rules).
//
// The mirror traverses structurally and records every discoverable span,
// never aborting except on pool overflow: validation, precedence and error
// scope are local parses, so the replay derives every outcome from the same
// recorded spans and later extras are harmless. Takes follow walk order and
// the pass stops at the first take failure the typed walk would also stop
// at; container resolve link reads are deduplicated per pass through a
// small seen list (the typed walk caches them in its Resolver), which is
// deterministic across passes and byte-identical within a pass.
// ===========================================================================

/// Shared hourly payload slots in calibration/report order: [bases,
/// registrations, aces]. The registrations rows live in the fog pack; the
/// decode worker fills them from this same log (see
/// `crate::extended::fog::Captured::decode_registrations_payload`).
pub(crate) const AIR_BASES_N: usize = 3;
const BASES_MASK: u32 = 0x01;
pub(super) const AIR_REGISTRATIONS_MASK: u32 = 0x02;
const AIR_ACES_MASK: u32 = 0x04;
/// Country census spans gate every slot: all three typed walks authenticate
/// through the same country reader.
const AIR_BASES_CENSUS_MASK: u32 = 0x07;
/// Manager, vectors and base identities gate bases and registrations: the
/// aces walk never reads them.
const BASE_VECTORS_MASK: u32 = 0x03;

/// Batch chunking for the registration pool/wing pointer blocks. Must match
/// the engine walk's batching exactly: the typed walk issues one
/// `read_slice` per chunk, and the replay serves exact (address, length)
/// spans only.
const REG_MIRROR_BATCH: usize = 64;

/// One hour of raw shared bases payload: the country census, the manager and
/// base vectors, per-base fields, containers with pools and wings, and ace
/// rosters with objects and names. Flat pools recycle capacities and the
/// steady state reuses storage; worker growth follows original native headers
/// before publication, and decode grows from that owned payload.
#[derive(Debug, Default)]
pub(crate) struct AirBasesJob {
    pub log: RawLog<AIR_BASES_N>,
    pub(crate) carrier: carrier::Job,
    pub game_state: usize,
    /// Complete registration-row and wing-handle demand from the original
    /// subtree acquisition. Shared census faults can coexist with these counts;
    /// the worker rejects faulted hints and the owned walk validates all fields.
    pub(crate) registration_demand: Option<[usize; 2]>,
    pub(super) aces_active: bool,
    /// Canonical (pointer, slot) countries parsed from the census takes.
    pub(super) census: Vec<(usize, u32)>,
    /// Link raws already taken this pass with their first successfully
    /// staged slot values. Failed resolutions are not cached.
    pub(super) seen_links: Vec<(u32, u32)>,
    /// Direct-map hints into `seen_links`, reset each pass. Collisions use
    /// the authoritative list, never a second observation of live memory.
    /// Once grown to the live link count every resolvable raw has its own cell.
    pub(super) seen_link_index: Vec<usize>,
    /// Live census demand observed this pass: canonical country slots (the
    /// country count less the sentinel) and country links. Growth sizes
    /// `census` and `seen_links` to these in one step, even when the pass also
    /// recorded a source fault, so a large census cannot keep overflowing.
    pub(super) required_census: usize,
    pub(super) required_links: usize,
    pub(super) capacity_charge: crate::raw_memory::Charge,
    /// How this pass finds each base's occupied container slots: set by the
    /// production copy from [`crate::air_base_changes`], otherwise as the
    /// caller left it. Kept across [`Self::clear`].
    pub(crate) slot_run: crate::air_base_changes::SlotRun,
    /// Bases (by handle) this pass recorded from the full scan after a
    /// compared difference; the replay scans them whole too.
    pub(crate) slot_full: Vec<u64>,
    pub(crate) slot_report: crate::air_base_changes::SlotReport,
    /// How this pass carries its state bases' registration subtrees: set by
    /// the production copy from [`crate::air_base_carry_changes`], otherwise
    /// as the caller left it. Kept across [`Self::clear`].
    pub(crate) carry: CarryRun,
    /// Held subtrees, lent by the sampling thread for the pass.
    pub(crate) registration_store: Option<Box<super::RegistrationStore>>,
    /// What one base's fresh read reached.
    pub(super) reached: Reached,
    /// Each state base whose subtree in the log is a held segment, with the
    /// segment's id ([`super::RegistrationStore`]), in walk order.
    pub(crate) segments: Vec<HeldSegment>,
    /// Where this pass's registration time went, for the status.
    pub(crate) registration_work: crate::air_base_carry_changes::Work,
}

/// A state base whose subtree in the hour's log is held segment `id`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct HeldSegment {
    pub handle: u64,
    pub pointer: usize,
    pub id: u64,
}

/// How one pass carries state bases' registration subtrees.
#[derive(Clone, Copy)]
pub(crate) struct CarryRun {
    pub(crate) changes: &'static dyn crate::air_base_carry_changes::Changes,
    /// Which carryable bases are also read afresh and compared.
    pub(crate) check: crate::sampled_verify::Check,
    /// The hour, which names a mismatch.
    pub(crate) hour: i32,
}

impl Default for CarryRun {
    fn default() -> Self {
        Self {
            changes: &crate::air_base_carry_changes::Observer,
            check: crate::sampled_verify::Check::Nothing,
            hour: 0,
        }
    }
}

impl std::fmt::Debug for CarryRun {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CarryRun")
            .field("check", &self.check)
            .finish_non_exhaustive()
    }
}

impl CarryRun {
    /// The hour being captured, as the production job reads it.
    #[cfg_attr(test, allow(dead_code))]
    pub(crate) fn current(hour: i32) -> Self {
        Self {
            changes: &crate::air_base_carry_changes::Observer,
            check: crate::sampled_verify::current_burned_in(crate::air_base_carry_changes::VERIFY),
            hour,
        }
    }
}

/// What one base's fresh read reached: each nonzero container country with
/// the slot its link named, each container, pool and wing; `lost` when one
/// could not be kept.
#[derive(Debug, Default)]
pub(super) struct Reached {
    links: Vec<(u32, u32)>,
    containers: Vec<usize>,
    pools: Vec<usize>,
    wings: Vec<usize>,
    lost: bool,
}

impl Reached {
    fn clear(&mut self) {
        self.links.clear();
        self.containers.clear();
        self.pools.clear();
        self.wings.clear();
        self.lost = false;
    }

    fn link(&mut self, country: u32, slot: u32) {
        if allocation::advisory(|| allocation::reserve(&mut self.links, 1)).is_ok() {
            self.links.push((country, slot));
        } else {
            self.lost = true;
        }
    }

    fn pool(&mut self, pool: usize) {
        if allocation::advisory(|| allocation::reserve(&mut self.pools, 1)).is_ok() {
            self.pools.push(pool);
        } else {
            self.lost = true;
        }
    }

    fn container(&mut self, container: usize) {
        if allocation::advisory(|| allocation::reserve(&mut self.containers, 1)).is_ok() {
            self.containers.push(container);
        } else {
            self.lost = true;
        }
    }

    fn wing(&mut self, wing: usize) {
        if allocation::advisory(|| allocation::reserve(&mut self.wings, 1)).is_ok() {
            self.wings.push(wing);
        } else {
            self.lost = true;
        }
    }
}

impl AirBasesJob {
    pub fn new() -> Self {
        let mut job = Self {
            carrier: carrier::Job::new(),
            ..Self::default()
        };
        job.presize_countries();
        job
    }

    pub fn clear(&mut self) {
        self.log.clear();
        self.carrier.clear();
        self.game_state = 0;
        self.registration_demand = None;
        self.aces_active = false;
        self.census.clear();
        self.seen_links.clear();
        self.seen_link_index.fill(usize::MAX);
        self.required_census = 0;
        self.required_links = 0;
        self.slot_full.clear();
        self.slot_report = Default::default();
        self.segments.clear();
        self.registration_work = Default::default();
    }

    pub fn grow_for_next(&self) -> bool {
        self.log.grow_for_next()
            || self.required_census > self.census.capacity()
            || self.required_links > self.seen_links.capacity()
            || self.carrier.grow_for_next()
    }

    /// Off-thread only. Census storage follows its own observed live demand.
    pub fn grow(&mut self) {
        engine::allocation::advisory(|| self.grow_hints());
    }

    fn grow_hints(&mut self) {
        fn grow_to<T>(rows: &mut Vec<T>, required: usize) {
            if required > rows.capacity() {
                let _ = allocation::reserve_exact(rows, required - rows.len());
            }
        }
        // Optional carrier pressure never doubles the predecessor's census.
        if self.log.grow_for_next()
            || self.required_census > self.census.capacity()
            || self.required_links > self.seen_links.capacity()
        {
            self.log.grow();
            grow_to(&mut self.census, self.required_census);
            grow_to(&mut self.seen_links, self.required_links);
        }
        self.carrier.grow();
        let capacity = self
            .required_links
            .max(crate::country_scale::census_rows(0));
        if reserve_to(&mut self.seen_link_index, capacity).is_err() {
            return;
        }
        self.seen_link_index.resize(capacity, usize::MAX);
        self.seen_link_index.fill(usize::MAX);
        self.update_capacity_charge();
    }

    /// Off the game thread: one census row and one link hint per observed
    /// country slot, plus independently observed carrier arena floors, so a
    /// mid-game attachment does not need an earlier callback for storage.
    pub fn presize_countries(&mut self) {
        crate::country_scale::reserve_census(&mut self.census);
        if crate::country_scale::reserve_census(&mut self.seen_links) {
            let capacity = self
                .required_links
                .max(crate::country_scale::census_rows(0));
            if reserve_to(&mut self.seen_link_index, capacity).is_err() {
                return;
            }
            self.seen_link_index.resize(capacity, usize::MAX);
            self.seen_link_index.fill(usize::MAX);
        }
        self.carrier.presize(crate::country_scale::carrier_demand());
        self.update_capacity_charge();
    }

    pub(super) fn update_capacity_charge(&mut self) {
        use crate::raw_memory::{Capacity, vector_bytes};
        self.capacity_charge.set(Capacity {
            auxiliary_vectors: vector_bytes(&self.census)
                + vector_bytes(&self.seen_links)
                + vector_bytes(&self.seen_link_index)
                + self.carrier.bytes(),
            ..Default::default()
        });
    }
}

/// Context of one current worker invocation. Sampling already forces a full
/// refresh on a campaign/load signal, clock discontinuity or refresh request; the
/// worker additionally requires adjacent sequence/hour and the same Shared.
#[derive(Debug, Clone, Copy)]
pub(crate) struct AirBasesMemoContext {
    pub owner: usize,
    pub base: usize,
    pub seq: u64,
    pub hour: i32,
    pub force_full: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct AirBasesMemoKey {
    pub(super) owner: usize,
    pub(super) base: usize,
    pub(super) game_state: usize,
    // Logical slice lengths control admission and therefore observations.
    // Exact equality is deliberately more conservative than "large enough".
    pub(super) capacities: [usize; 5],
    pub(super) carrier_capacities: [usize; 10],
    pub(super) carrier_seed: Result<(), &'static str>,
    pub(super) aces_active: bool,
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct AirBasesMemoReport {
    pub hit: bool,
    pub eligible: bool,
    pub compared: bool,
    pub installed: bool,
    pub compare_ticks: u64,
    pub restore_ticks: u64,
    pub install_ticks: u64,
}

// A separate tracker explicitly includes native memo outputs, which are outside
// raw_memory's generic payload/index scope. No per-frame/pool memo allocation.
static AIR_BASES_MEMO_MEMORY: crate::raw_memory::Tracker = crate::raw_memory::Tracker::new();

pub(crate) fn air_bases_memo_memory() -> impl serde::Serialize {
    let snapshot = AIR_BASES_MEMO_MEMORY.snapshot();
    crate::status::status!({
        "scope": "one worker-owned AirBases memo; actual input and native-output vector capacities, retained across misses",
        "input_capacity_bytes": snapshot.current.payload_vectors,
        "native_output_capacity_bytes": snapshot.current.auxiliary_vectors,
        "current_capacity_bytes": snapshot.current.payload_vectors + snapshot.current.auxiliary_vectors,
        "peak_capacity_bytes": snapshot.peak_total_lower_bound,
        "excluded": ["inline memo metadata", "allocator rounding/metadata and transient reallocation overlap"],
    })
}

/// Exactly one previous complete family result, owned independently of every
/// pooled job/frame. Every capture still acquires current bytes before the game resumes.
/// Equality never serves old observations to a Memory implementation: it only
/// reuses the deterministic native result of an identical complete invocation.
///
/// Full ordered Span equality (not table partitions) retains exact-key cursor,
/// last-repeat and exact-before-bulk semantics even under arbitrary aliases.
/// Rust field equality avoids comparing Span padding; only used bytes matter.
#[derive(Debug)]
pub(crate) struct AirBasesMemo {
    pub(super) key: Option<AirBasesMemoKey>,
    pub(super) previous: Option<(u64, i32)>,
    pub(super) spans: Vec<crate::raw::Span>,
    pub(super) bytes: Vec<u8>,
    pub(super) has_bulk: bool,
    pub(super) bases: Vec<cair_base::Row>,
    pub(super) aces: Vec<cace::Row>,
    pub(super) names: Vec<u8>,
    pub(super) required_names: usize,
    pub(super) registrations: super::fog::RegistrationsMemo,
    pub(super) carrier: carrier::Memo,
    /// State bases' rows walked from held segments, for later hours that
    /// report the same segments; valid independently of the memo's key.
    pub(super) decoded: super::DecodedRegistrations,
    pub(super) capacity_charge: crate::raw_memory::Charge,
}

impl Default for AirBasesMemo {
    fn default() -> Self {
        Self::on_tracker(&AIR_BASES_MEMO_MEMORY)
    }
}

impl AirBasesMemo {
    pub(super) fn on_tracker(tracker: &'static crate::raw_memory::Tracker) -> Self {
        Self {
            key: None,
            previous: None,
            spans: Vec::new(),
            bytes: Vec::new(),
            has_bulk: false,
            bases: Vec::new(),
            aces: Vec::new(),
            names: Vec::new(),
            required_names: 0,
            registrations: super::fog::RegistrationsMemo::default(),
            carrier: carrier::Memo::default(),
            decoded: super::DecodedRegistrations::default(),
            capacity_charge: crate::raw_memory::Charge::empty_on(tracker),
        }
    }

    /// Drop validity, not reusable storage. A missed refresh/typed fallback,
    /// fault or incomplete decode may never carry an older entry forward.
    pub(crate) fn invalidate(&mut self) {
        self.key = None;
        self.previous = None;
    }

    pub(super) fn capacity(&self) -> crate::raw_memory::Capacity {
        use crate::raw_memory::{Capacity, vector_bytes};
        Capacity {
            payload_vectors: vector_bytes(&self.spans)
                + vector_bytes(&self.bytes)
                + self.carrier.bytes().0,
            auxiliary_vectors: vector_bytes(&self.bases)
                + vector_bytes(&self.aces)
                + vector_bytes(&self.names)
                + self.registrations.capacity_bytes()
                + self.carrier.bytes().1
                + self.decoded.capacity_bytes(),
            ..Capacity::default()
        }
    }

    pub(super) fn input_equal(&self, log: &RawLog<AIR_BASES_N>) -> bool {
        // A stored witness contains only successes. Equality of every Span's
        // ok field also rejects a failed current observation, without a second
        // explicit span scan after the decode entry's eligibility preflight.
        let aces_active = self.key.is_none_or(|key| key.aces_active);
        !log.overflowed
            && !log
                .fault
                .iter()
                .enumerate()
                .any(|(slot, fault)| *fault && (slot < 2 || aces_active))
            && self.has_bulk == log.has_bulk
            && self.spans == log.spans
            && self.bytes.as_slice() == log.bytes.as_ref()
    }

    pub(crate) fn decode(
        &mut self,
        air: &mut Captured,
        fog: &mut super::fog::Captured,
        job: &AirBasesJob,
        ctx: AirBasesMemoContext,
        decode_ticks: &mut [u64; AIR_BASES_N],
    ) -> AirBasesMemoReport {
        let mut report = AirBasesMemoReport::default();
        air.active[3] = job.aces_active;
        let compared_started = qpc();
        let adjacent = self.previous.is_some_and(|(seq, hour)| {
            seq.checked_add(1) == Some(ctx.seq) && hour.checked_add(1) == Some(ctx.hour)
        });
        if ctx.force_full || !adjacent {
            self.key = None;
        }
        // Storage must be final before comparing or installing the capacity
        // fingerprint: another pooled frame may still have the old limits.
        fog.prepare_registrations_payload(job);
        fog.prepare_carrier_payload(job, &mut air.carrier);
        let capacity = air
            .bases_memo_context()
            .zip(fog.registrations_memo_context());
        let carrier_capacities =
            carrier_capacities(&air.carrier, &fog.carrier, &fog.carrier_scratch);
        let key = capacity.map(|(air, fog)| AirBasesMemoKey {
            owner: ctx.owner,
            base: ctx.base,
            game_state: job.game_state,
            capacities: [air[0], air[1], air[2], fog[0], fog[1]],
            carrier_capacities,
            carrier_seed: job.carrier.seed,
            aces_active: job.aces_active,
        });
        report.eligible = key.is_some()
            && job.carrier.seed.is_ok()
            && !job.carrier.log.borrow().overflowed
            && !job.log.overflowed
            && !job
                .log
                .fault
                .iter()
                .enumerate()
                .any(|(slot, fault)| *fault && (slot < 2 || job.aces_active))
            && job.log.spans.iter().all(|span| span.ok);
        if report.eligible && self.key == key {
            report.compared = true;
            report.hit =
                self.input_equal(&job.log) && self.carrier.input_equal(&job.carrier.log.borrow());
        }
        report.compare_ticks = elapsed(compared_started);
        decode_ticks[1] += report.compare_ticks;
        if report.hit {
            let restore_before = decode_ticks.iter().sum::<u64>();
            let started = qpc();
            air.bases.rows[..self.bases.len()].copy_from_slice(&self.bases);
            air.bases.outcome = Outcome::Rows {
                len: self.bases.len(),
            };
            decode_ticks[0] += elapsed(started);
            if job.aces_active {
                let started = qpc();
                air.aces.rows[..self.aces.len()].copy_from_slice(&self.aces);
                air.aces.outcome = Outcome::Rows {
                    len: self.aces.len(),
                };
                air.names.first[..self.names.len()].copy_from_slice(&self.names);
                air.names.len = self.names.len();
                air.names.required = self.required_names;
                decode_ticks[2] += elapsed(started);
            } else {
                air.aces.outcome = SKIPPED;
                air.names.reset();
            }
            air.bases_raw_mode = false;
            let started = qpc();
            fog.restore_registrations_memo(&self.registrations);
            self.carrier.restore(&mut air.carrier, &mut fog.carrier);
            decode_ticks[1] += elapsed(started);
            report.restore_ticks = decode_ticks
                .iter()
                .sum::<u64>()
                .saturating_sub(restore_before);
            self.previous = Some((ctx.seq, ctx.hour));
            return report;
        }

        self.key = None;
        // Preserve both original Recorded lifetimes: bases+aces share cursors;
        // registrations starts a separate reader over the same current log.
        air.decode_air_bases_payload(ctx.base, job, decode_ticks);
        let before = self.decoded.counts();
        let reuse = self.decoded.begin(&job.segments);
        fog.decode_registrations_with_carrier_payload(
            ctx.base,
            job,
            &air.bases.rows[..air.bases.outcome.len()],
            &mut air.carrier,
            decode_ticks,
            reuse.then_some(&mut self.decoded as &mut dyn cair_base::RegistrationReuse),
        );
        self.decoded.finish();
        let after = self.decoded.counts();
        crate::air_base_carry_changes::record_decoded(
            after.reused - before.reused,
            after.walked - before.walked,
        );
        // Range validation is memo work too, even when it rejects an install.
        let install_started = qpc();
        if report.eligible && air.bases_memo_complete() && fog.registrations_memo_complete() {
            if reserve_to(&mut self.spans, job.log.spans.len()).is_err()
                || reserve_to(&mut self.bytes, job.log.bytes.len()).is_err()
                || reserve_to(&mut self.bases, air.bases.outcome.len()).is_err()
                || (job.aces_active
                    && (reserve_to(&mut self.aces, air.aces.outcome.len()).is_err()
                        || reserve_to(&mut self.names, air.names.len).is_err()))
            {
                self.invalidate();
                self.capacity_charge.set(self.capacity());
                return report;
            }
            self.spans.clear();
            self.spans.extend_from_slice(&job.log.spans);
            self.bytes.clear();
            self.bytes.extend_from_slice(job.log.bytes.as_ref());
            self.has_bulk = job.log.has_bulk;
            self.bases.clear();
            self.bases
                .extend_from_slice(&air.bases.rows[..air.bases.outcome.len()]);
            self.aces.clear();
            self.names.clear();
            self.required_names = 0;
            if job.aces_active {
                self.aces
                    .extend_from_slice(&air.aces.rows[..air.aces.outcome.len()]);
                self.names
                    .extend_from_slice(&air.names.first[..air.names.len]);
                self.required_names = air.names.required;
            }
            fog.save_registrations_memo(&mut self.registrations);
            if !self.carrier.save(
                &air.carrier,
                &fog.carrier,
                &job.carrier.log.borrow(),
                job.carrier.demand,
            ) {
                self.invalidate();
                self.capacity_charge.set(self.capacity());
                return report;
            }
            self.capacity_charge.set(self.capacity());
            self.key = key;
            self.previous = Some((ctx.seq, ctx.hour));
            report.installed = true;
        } else {
            self.invalidate();
        }
        report.install_ticks = elapsed(install_started);
        decode_ticks[1] += report.install_ticks;
        report
    }
}

impl Captured {
    fn bases_memo_context(&self) -> Option<[usize; 3]> {
        self.bases_raw_mode.then_some([
            self.bases.rows.len(),
            if self.active[3] {
                self.aces.rows.len()
            } else {
                0
            },
            if self.active[3] {
                self.names.first.len()
            } else {
                0
            },
        ])
    }

    fn bases_memo_complete(&self) -> bool {
        matches!(self.bases.outcome, Outcome::Rows { len } if len <= self.bases.rows.len())
            && (!self.active[3]
                || (matches!(self.aces.outcome, Outcome::Rows { .. })
                    && self.names.required >= self.names.len
                    && valid_names(
                        self.aces.outcome,
                        &self.aces.rows,
                        &self.names.first,
                        self.names.len,
                    )))
    }
}

fn carrier_capacities(
    bases: &carrier::Bases,
    registrations: &carrier::Registrations,
    scratch: &carrier::Scratch,
) -> [usize; 10] {
    let bases = bases.capacity();
    let registrations = registrations.capacity();
    let scratch = scratch.capacity();
    [
        bases[0],
        bases[1],
        bases[2],
        registrations[0],
        registrations[1],
        scratch[0],
        scratch[1],
        scratch[2],
        scratch[3],
        scratch[4],
    ]
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct AirBasesCopyReport {
    /// False when staging overflowed: the caller falls back to the typed
    /// walks (identical rows through the existing paths) and enqueues no
    /// payload.
    pub raw_ok: bool,
    pub copy_ticks: [u64; AIR_BASES_N],
    pub raw_bytes: [u64; AIR_BASES_N],
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct AirBasesCopyContext {
    pub base: usize,
    pub game_state: usize,
    pub hour: i32,
    pub force_full: bool,
}

fn bases_pool_u64(log: &RawLog<AIR_BASES_N>, start: usize) -> u64 {
    u64::from_le_bytes(log.bytes[start..start + 8].try_into().unwrap())
}

pub(super) fn bases_pool_u32(log: &RawLog<AIR_BASES_N>, start: usize) -> u32 {
    u32::from_le_bytes(log.bytes[start..start + 4].try_into().unwrap())
}

/// One mirrored span read. `None` addresses (arithmetic overflow) record
/// nothing because the typed walk fails locally without reads.
pub(super) fn bspan(
    log: &mut RawLog<AIR_BASES_N>,
    memory: &impl engine::Memory,
    addr: Option<usize>,
    len: usize,
    mask: u32,
) -> Option<usize> {
    log.take(memory, addr?, len, mask)
}

/// Mirror of one link-table read with local bounds checks. Returns `None`
/// exactly when the typed resolve fails; the replay derives its own reason.
fn bases_link(
    log: &mut RawLog<AIR_BASES_N>,
    memory: &impl engine::Memory,
    links: &Array,
    countries: u32,
    raw: u32,
    mask: u32,
) -> Option<u32> {
    if raw == 0 || raw >= links.count {
        return None;
    }
    let offset = (raw as usize).checked_mul(4)?;
    let start = bspan(log, memory, links.data.checked_add(offset), 4, mask)?;
    let slot = bases_pool_u32(log, start);
    if slot == 0 || slot >= countries {
        return None;
    }
    Some(slot)
}

/// Authenticated country census shared by the bases, registrations and aces
/// walks. Records the exact read set of `ccountry::Reader::new` plus one full
/// `read_at` per slot, taking every discoverable span: validation mismatches
/// never stop the census because each replay derives its own verdict from the
/// same spans (a mismatch fatal to one walk may be unread by another).
/// Returns the parsed link array with its country count for the resolve
/// mirror, or `None` only when the headers themselves are undiscoverable.
/// `required` receives the live [canonical slot, link] demand as soon as both
/// headers parse, so an overflowing census still sizes its growth exactly.
fn copy_air_bases_census(
    log: &mut RawLog<AIR_BASES_N>,
    memory: &impl engine::Memory,
    base: usize,
    game_state: usize,
    census: &mut Vec<(usize, u32)>,
    required: [&mut usize; 2],
    mask: u32,
) -> Option<(Array, u32)> {
    let countries_start = bspan(
        log,
        memory,
        game_state.checked_add(cgame_state::COUNTRIES),
        16,
        mask,
    )?;
    let links_start = bspan(
        log,
        memory,
        game_state.checked_add(cgame_state::COUNTRY_LINKS),
        16,
        mask,
    )?;
    let mut raw = [0u8; 16];
    raw.copy_from_slice(&log.bytes[countries_start..countries_start + 16]);
    let countries = Array::from_header(raw, 8)?;
    raw.copy_from_slice(&log.bytes[links_start..links_start + 16]);
    let links = Array::from_header(raw, 4)?;
    if countries.count < 2 || links.count == 0 {
        return None;
    }
    let [required_census, required_links] = required;
    *required_census = countries.count as usize - 1;
    *required_links = links.count as usize;
    if log.worker_growth_enabled() && reserve_to(census, *required_census).is_err() {
        return None;
    }
    let vtable = base.checked_add(ccountry::VTABLE_RVA)? as u64;
    for slot in 1..countries.count {
        let pointer_addr = (slot as usize)
            .checked_mul(8)
            .and_then(|offset| countries.data.checked_add(offset));
        let Some(pointer_start) = bspan(log, memory, pointer_addr, 8, mask) else {
            continue;
        };
        let pointer = match usize::try_from(bases_pool_u64(log, pointer_start))
            .ok()
            .filter(|&pointer| pointer != 0)
        {
            Some(pointer) => pointer,
            None => continue,
        };
        let vtable_start = bspan(log, memory, Some(pointer), 8, mask);
        let id_start = bspan(log, memory, pointer.checked_add(ccountry::ID), 4, mask);
        let (Some(vtable_start), Some(id_start)) = (vtable_start, id_start) else {
            continue;
        };
        if bases_pool_u64(log, vtable_start) != vtable {
            continue;
        }
        let id = bases_pool_u32(log, id_start);
        let canonical = bases_link(log, memory, &links, countries.count, id, mask);
        let slot_link = bases_link(log, memory, &links, countries.count, slot, mask);
        let (Some(canonical), Some(slot_link)) = (canonical, slot_link) else {
            continue;
        };
        if slot_link != canonical {
            continue;
        }
        let check = bases_link(log, memory, &links, countries.count, canonical, mask);
        if check != Some(canonical) {
            continue;
        }
        let canonical_addr = (canonical as usize)
            .checked_mul(8)
            .and_then(|offset| countries.data.checked_add(offset));
        let canonical_pointer = bspan(log, memory, canonical_addr, 8, mask);
        let Some(canonical_pointer) = canonical_pointer else {
            continue;
        };
        if bases_pool_u64(log, canonical_pointer) != pointer as u64 {
            continue;
        }
        if slot != canonical {
            continue;
        }
        if census.len() == census.capacity() {
            log.overflowed = true;
            return None;
        }
        census.push((pointer, slot));
    }
    Some((links, countries.count))
}

/// Deduplicated resolve-link take returning the first successfully staged
/// slot value for a raw in this pass. Index collisions still consult that
/// authoritative observation; failed resolutions remain uncached.
pub(super) fn resolve_link(
    log: &mut RawLog<AIR_BASES_N>,
    memory: &impl engine::Memory,
    links: &Array,
    seen: &mut Vec<(u32, u32)>,
    index: &mut [usize],
    countries: u32,
    raw: u32,
) -> Option<u32> {
    let bucket = (!index.is_empty()).then(|| raw as usize % index.len());
    if let Some(bucket) = bucket
        && let Some(&(key, slot)) = seen.get(index[bucket])
        && key == raw
    {
        return Some(slot);
    }
    // Each insertion fills its bucket until the next pass clears both stores.
    // An empty bucket proves absence; collisions and invalid hints still need
    // the authoritative list, as does the unindexed path.
    if bucket.is_none_or(|bucket| index[bucket] != usize::MAX)
        && let Some(position) = seen.iter().position(|(key, _)| *key == raw)
    {
        if let Some(bucket) = bucket {
            index[bucket] = position;
        }
        return Some(seen[position].1);
    }
    if raw == 0 || raw >= links.count {
        return None;
    }
    let offset = (raw as usize).checked_mul(4)?;
    let start = bspan(
        log,
        memory,
        links.data.checked_add(offset),
        4,
        AIR_REGISTRATIONS_MASK,
    )?;
    let slot = bases_pool_u32(log, start);
    if slot >= countries {
        return None;
    }
    if seen.len() == seen.capacity()
        && log.worker_growth_enabled()
        && allocation::reserve(seen, 1).is_err()
    {
        return None;
    }
    if seen.len() == seen.capacity() {
        log.overflowed = true;
        return None;
    }
    let position = seen.len();
    seen.push((raw, slot));
    if let Some(bucket) = bucket {
        index[bucket] = position;
    }
    Some(slot)
}

/// Read one native word back out of a recorded pointer-block take (`chunk`
/// words staged at pool offset `start`). The typed walk reads each block
/// with one `read_slice`; the mirror takes the same span and parses the
/// staged bytes locally, never re-reading live memory and never allocating.
fn block_word(log: &RawLog<AIR_BASES_N>, start: usize, index: usize) -> Option<usize> {
    let at = start.checked_add(index.checked_mul(8)?)?;
    let bytes = log.bytes.get(at..at.checked_add(8)?)?;
    usize::try_from(u64::from_le_bytes(bytes.try_into().ok()?)).ok()
}

fn copy_air_base_row(
    log: &mut RawLog<AIR_BASES_N>,
    memory: &impl engine::Memory,
    base: usize,
    pointer: usize,
    state_slot: Option<u32>,
    handle: u64,
) -> Option<cair_base::Row> {
    // The 16-byte identity is taken once per base by the shared vector loop
    // (both walks read it); only the bases-private host/state/province
    // fields are taken here.
    let host = bspan(
        log,
        memory,
        pointer.checked_add(cair_base::HOST_REFERENCE),
        32,
        BASES_MASK,
    )?;
    let state_pointer = usize::try_from(bases_pool_u64(log, host + 8)).ok()?;
    let province_pointer = usize::try_from(bases_pool_u64(log, host + 16)).ok()?;
    let state = if state_pointer == 0 {
        None
    } else {
        let vtable = bspan(log, memory, Some(state_pointer), 8, BASES_MASK)?;
        let expected_state = match base.checked_add(cstate::VTABLE_RVA) {
            Some(expected) => expected as u64,
            None => return None,
        };
        if bases_pool_u64(log, vtable) != expected_state {
            return None;
        }
        let id = match bspan(
            log,
            memory,
            state_pointer.checked_add(cstate::ID),
            4,
            BASES_MASK,
        ) {
            Some(start) => bases_pool_u32(log, start),
            None => return None,
        };
        Some(id)
    };
    if state != state_slot {
        return None;
    }
    let host_ref = bases_pool_u64(log, host);
    if state.is_none() && host_ref == 0 {
        return None;
    }
    let province = if province_pointer != 0 {
        let vtable = bspan(log, memory, Some(province_pointer), 8, BASES_MASK)?;
        let expected_province = match base.checked_add(cprovince::VTABLE_RVA) {
            Some(expected) => expected as u64,
            None => return None,
        };
        if bases_pool_u64(log, vtable) != expected_province {
            return None;
        }
        let owner = bspan(
            log,
            memory,
            province_pointer.checked_add(cprovince::STATE),
            8,
            BASES_MASK,
        )?;
        if state_pointer != 0 && bases_pool_u64(log, owner) != state_pointer as u64 {
            return None;
        }
        let id = bspan(
            log,
            memory,
            province_pointer.checked_add(cprovince::ID),
            4,
            BASES_MASK,
        )?;
        Some(bases_pool_u32(log, id))
    } else {
        None
    };
    Some(cair_base::Row {
        id: handle,
        state,
        province,
        host: host_ref,
        capacity: i32::from_le_bytes(log.bytes[host + 24..host + 28].try_into().unwrap()),
        kind: i32::from_le_bytes(log.bytes[host + 28..host + 32].try_into().unwrap()),
    })
}

/// Mirror of one country's ace roster and objects. Takes every
/// discoverable span; validation mismatches stop only this country's takes
/// (the typed walk fails there). Returns false exactly when the typed walk
/// fails at or before this country.
fn copy_air_aces_country(
    log: &mut RawLog<AIR_BASES_N>,
    memory: &impl engine::Memory,
    base: usize,
    pointer: usize,
    slot: u32,
) -> bool {
    let header = match bspan(
        log,
        memory,
        pointer.checked_add(ccountry::ACES),
        16,
        AIR_ACES_MASK,
    ) {
        Some(start) => start,
        None => return false,
    };
    let mut raw = [0u8; 16];
    raw.copy_from_slice(&log.bytes[header..header + 16]);
    let roster = match Array::from_header(raw, 8) {
        Some(array) => array,
        None => return false,
    };
    for ordinal in 0..roster.count {
        if !copy_air_ace(log, memory, base, pointer, slot, &roster, ordinal) {
            return false;
        }
        if log.overflowed {
            return false;
        }
    }
    true
}

/// Mirror of one ace object with its three name strings. The seven scattered
/// fields are taken as seven exact spans in walk order; heap names take the
/// terminator byte plus the content bytes, inline names take nothing further.
/// Returns false exactly when the typed row read fails for this ace.
fn copy_air_ace(
    log: &mut RawLog<AIR_BASES_N>,
    memory: &impl engine::Memory,
    base: usize,
    country_pointer: usize,
    slot: u32,
    roster: &Array,
    ordinal: u32,
) -> bool {
    let addr = roster.element(ordinal, 8);
    let entry = match bspan(log, memory, addr, 8, AIR_ACES_MASK) {
        Some(start) => start,
        None => return false,
    };
    let pointer = match usize::try_from(bases_pool_u64(log, entry))
        .ok()
        .filter(|&pointer| pointer != 0)
    {
        Some(pointer) => pointer,
        None => return false,
    };
    // Seven scattered fields in walk order: identity, mask, owner/wing,
    // the 96-byte three-string header block, alive, kill type, handled.
    let identity = bspan(log, memory, Some(pointer), 16, AIR_ACES_MASK);
    let mask = bspan(
        log,
        memory,
        pointer.checked_add(cace::AIRCRAFT_TYPE_MASK),
        4,
        AIR_ACES_MASK,
    );
    let owner = bspan(
        log,
        memory,
        pointer.checked_add(cace::OWNER_COUNTRY),
        16,
        AIR_ACES_MASK,
    );
    let strings = bspan(
        log,
        memory,
        pointer.checked_add(cace::NAME),
        96,
        AIR_ACES_MASK,
    );
    let alive = bspan(
        log,
        memory,
        pointer.checked_add(cace::ALIVE),
        1,
        AIR_ACES_MASK,
    );
    let kill_type = bspan(
        log,
        memory,
        pointer.checked_add(cace::KILL_TYPE),
        4,
        AIR_ACES_MASK,
    );
    let handled = bspan(
        log,
        memory,
        pointer.checked_add(cace::HANDLED),
        1,
        AIR_ACES_MASK,
    );
    let (Some(identity), Some(_), Some(owner), Some(strings), Some(alive), Some(_), Some(handled)) =
        (identity, mask, owner, strings, alive, kill_type, handled)
    else {
        return false;
    };
    let expected = match base.checked_add(cace::VTABLE_RVA) {
        Some(expected) => expected as u64,
        None => return false,
    };
    if bases_pool_u64(log, identity) != expected
        || bases_pool_u64(log, identity + 8) == 0
        || bases_pool_u64(log, owner) != country_pointer as u64
        || log.bytes[alive] > 1
        || log.bytes[handled] > 1
    {
        return false;
    }
    if log.bytes[alive] == 0
        && bspan(
            log,
            memory,
            pointer.checked_add(cace::KILLER_ACE_REFERENCE),
            12,
            AIR_ACES_MASK,
        )
        .is_none()
    {
        return false;
    }
    for index in 0..3 {
        let head = strings + index * 32;
        if !copy_ace_name(log, memory, head) {
            return false;
        }
        if log.overflowed {
            return false;
        }
    }
    // Independent appended presentation; failure keeps the established prefix.
    let portrait = bspan(
        log,
        memory,
        pointer.checked_add(cace::PORTRAIT_GENDER),
        8,
        AIR_ACES_MASK,
    );
    if let Some(selected) = bspan(
        log,
        memory,
        pointer.checked_add(cace::TYPE_DEFINITION),
        8,
        AIR_ACES_MASK,
    ) {
        let definition = bases_pool_u64(log, selected) as usize;
        if definition != 0
            && let Some(head) = bspan(
                log,
                memory,
                definition.checked_add(cace::TYPE_NAME),
                32,
                AIR_ACES_MASK,
            )
        {
            copy_ace_name(log, memory, head);
        }
    }
    if let Some(portrait) = portrait {
        let gender = log.bytes[portrait];
        let index = i32::from_le_bytes(log.bytes[portrait + 4..portrait + 8].try_into().unwrap());
        let staged = AcePresentationMemory {
            memory,
            log: std::cell::RefCell::new(log),
        };
        if let Ok(names) = cace::portrait_sources(
            &staged,
            base,
            ccountry::Country {
                pointer: country_pointer,
                canonical_slot: slot,
                id: slot,
            },
            gender,
            index,
        ) {
            for name in names.into_iter().flatten() {
                if let Some((address, len)) = name.heap_payload() {
                    let mut log = staged.log.borrow_mut();
                    bspan(&mut log, memory, Some(address), len, AIR_ACES_MASK);
                }
            }
        }
    }
    !log.overflowed
}

// Stages exactly the shared portrait reader's dependent descriptors. Owned
// decode uses the same reader, so stale pools and failures remain native.
struct AcePresentationMemory<'a, M> {
    memory: &'a M,
    log: std::cell::RefCell<&'a mut RawLog<AIR_BASES_N>>,
}
impl<M: Memory> Memory for AcePresentationMemory<'_, M> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        let mut log = self.log.borrow_mut();
        let Some(start) = bspan(
            &mut log,
            self.memory,
            Some(address),
            destination.len(),
            AIR_ACES_MASK,
        ) else {
            return false;
        };
        destination.copy_from_slice(&log.bytes[start..start + destination.len()]);
        true
    }
}

/// Mirror of one ace name string: header validation is a local parse, heap
/// names take the terminator byte plus the content bytes. Returns false
/// exactly when the typed name read fails.
fn copy_ace_name(log: &mut RawLog<AIR_BASES_N>, memory: &impl engine::Memory, head: usize) -> bool {
    let header = &log.bytes[head..head + 32];
    let len = u64::from_le_bytes(
        header[msvc_string::LENGTH..msvc_string::LENGTH + 8]
            .try_into()
            .unwrap(),
    );
    let capacity = u64::from_le_bytes(
        header[msvc_string::CAPACITY..msvc_string::CAPACITY + 8]
            .try_into()
            .unwrap(),
    );
    if len > capacity || capacity < msvc_string::INLINE_CAPACITY {
        return false;
    }
    let len = match usize::try_from(len) {
        Ok(len) => len,
        Err(_) => return false,
    };
    if capacity == msvc_string::INLINE_CAPACITY {
        if header.get(len) != Some(&0) {
            return false;
        }
        return true;
    }
    let source = match usize::try_from(u64::from_le_bytes(
        header[msvc_string::DATA..msvc_string::DATA + 8]
            .try_into()
            .unwrap(),
    ))
    .ok()
    .filter(|&source| source != 0)
    {
        Some(source) => source,
        None => return false,
    };
    if source.checked_add(len).is_none() {
        return false;
    }
    // Terminator first, then content: the walk's read order.
    if bspan(log, memory, source.checked_add(len), 1, AIR_ACES_MASK).is_none() {
        return false;
    }
    if len == 0 {
        return true;
    }
    if bspan(log, memory, Some(source), len, AIR_ACES_MASK).is_none() {
        return false;
    }
    true
}

/// One listed container the dense plan proved: its slot, its pointer and the
/// log starts of its identity and span, which the slot loop reuses.
#[derive(Clone, Copy, Default)]
struct Planned {
    slot: u32,
    container: usize,
    vtable: usize,
    span: usize,
}

/// Where one slot chunk's words come from: a planned base's listed
/// containers in it (a span of its plan), or the log start and completed
/// element count of the whole chunk's take.
enum ChunkWords {
    Planned(std::ops::Range<usize>),
    Taken { start: usize, copied: usize },
}

/// One pass's slot settings and outcomes for the registration mirror
/// ([`crate::air_base_changes`]).
struct SlotPass<'a> {
    run: crate::air_base_changes::SlotRun,
    full: &'a mut Vec<u64>,
    report: &'a mut crate::air_base_changes::SlotReport,
}

/// Mirror of [`cair_base::dense_plan`]: the same takes, and the same verdict
/// from their bytes, so the replay reaches the decision this pass took. The
/// link read is the deduplicated [`resolve_link`] take of the same entry.
#[allow(clippy::too_many_arguments)]
fn copy_dense_plan(
    log: &mut RawLog<AIR_BASES_N>,
    memory: &impl engine::Memory,
    image: usize,
    pointer: usize,
    containers: &Array,
    links: &Array,
    countries: u32,
    seen: &mut Vec<(u32, u32)>,
    seen_index: &mut [usize],
    plan: &mut [Planned; cair_base::DENSE_BOUND],
) -> Result<usize, cair_base::DenseRefusal> {
    use cair_base::DenseRefusal as Refusal;
    let header = bspan(
        log,
        memory,
        pointer.checked_add(cair_base::REGISTERED_CONTAINERS),
        16,
        AIR_REGISTRATIONS_MASK,
    )
    .ok_or(Refusal::Header)?;
    let mut raw = [0u8; 16];
    raw.copy_from_slice(&log.bytes[header..header + 16]);
    let dense = Array::from_header(raw, 8).ok_or(Refusal::Header)?;
    if dense.count > containers.count {
        return Err(Refusal::LongerThanSlots);
    }
    let len = dense.count as usize;
    if len > cair_base::DENSE_BOUND {
        return Err(Refusal::OverBound);
    }
    let block = if len == 0 {
        0
    } else {
        bspan(
            log,
            memory,
            Some(dense.data),
            len * 8,
            AIR_REGISTRATIONS_MASK,
        )
        .ok_or(Refusal::Header)?
    };
    let expected = image
        .checked_add(ccountry_air_container::VTABLE_RVA)
        .ok_or(Refusal::Container)? as u64;
    for index in 0..len {
        let container = block_word(log, block, index)
            .filter(|&pointer| pointer != 0)
            .ok_or(Refusal::Entry)?;
        if plan[..index]
            .iter()
            .any(|entry| entry.container == container)
        {
            return Err(Refusal::Entry);
        }
        let vtable = bspan(log, memory, Some(container), 8, AIR_REGISTRATIONS_MASK)
            .ok_or(Refusal::Container)?;
        if bases_pool_u64(log, vtable) != expected {
            return Err(Refusal::Container);
        }
        let span = bspan(
            log,
            memory,
            container.checked_add(ccountry_air_container::COUNTRY),
            cair_base::CONTAINER_SPAN,
            AIR_REGISTRATIONS_MASK,
        )
        .ok_or(Refusal::Container)?;
        let country = bases_pool_u32(log, span);
        let slot = if country == 0 {
            0
        } else {
            resolve_link(log, memory, links, seen, seen_index, countries, country)
                .ok_or(Refusal::Country)?
        };
        if log.overflowed {
            return Err(Refusal::Country);
        }
        if slot >= containers.count {
            return Err(Refusal::SlotRange);
        }
        if plan[..index].iter().any(|entry| entry.slot == slot) {
            return Err(Refusal::SharedSlot);
        }
        let word = bspan(
            log,
            memory,
            containers.element(slot, 8),
            8,
            AIR_REGISTRATIONS_MASK,
        )
        .ok_or(Refusal::Stale)?;
        if bases_pool_u64(log, word) != container as u64 {
            return Err(Refusal::Stale);
        }
        plan[index] = Planned {
            slot,
            container,
            vtable,
            span,
        };
    }
    plan[..len].sort_unstable_by_key(|entry| entry.slot);
    Ok(len)
}

/// Whether a base's whole slot vector, read outside the log, holds exactly
/// the planned containers at their slots; `None` when a chunk did not read.
fn whole_scan_agrees(
    memory: &impl engine::Memory,
    containers: &Array,
    plan: &[Planned],
) -> Option<bool> {
    let mut bytes = [0u8; REG_MIRROR_BATCH * 8];
    let mut found = 0;
    let mut first = 0u32;
    while first < containers.count {
        let count = ((containers.count - first) as usize).min(REG_MIRROR_BATCH);
        let address = containers.element(first, 8)?;
        if !engine::read_slice(memory, address, &mut bytes[..count * 8]) {
            return None;
        }
        for (offset, word) in bytes[..count * 8].chunks_exact(8).enumerate() {
            let pointer = u64::from_le_bytes(word.try_into().unwrap());
            if pointer == 0 {
                continue;
            }
            let slot = first + offset as u32;
            match plan.binary_search_by_key(&slot, |entry| entry.slot) {
                Ok(index) if plan[index].container as u64 == pointer => found += 1,
                _ => return Some(false),
            }
        }
        first += count as u32;
    }
    Some(found == plan.len())
}

/// One pass's carry of state bases' registration subtrees
/// ([`super::RegistrationStore`]).
struct CarryPass<'a> {
    run: CarryRun,
    store: Option<&'a mut super::RegistrationStore>,
    reached: &'a mut Reached,
    segments: &'a mut Vec<HeldSegment>,
    work: &'a mut crate::air_base_carry_changes::Work,
}

impl CarryPass<'_> {
    /// The base's subtree in the log is held segment `id`. Advisory: an
    /// unreported base is walked by the decoder.
    fn segment(&mut self, handle: u64, pointer: usize, id: u64) {
        if allocation::advisory(|| allocation::reserve(self.segments, 1)).is_ok() {
            self.segments.push(HeldSegment {
                handle,
                pointer,
                id,
            });
        }
    }
}

/// One base's registration subtree for the log: a state base's held segment
/// when its certificate and slot mode are the held read's and its links
/// still resolve as then, otherwise read afresh and held. A host base, every
/// base without a store, and a base either sampled slice covers (the carry's
/// or the dense slot check's) is read. A read is held only when every pool it
/// listed names the base by `+0x18`, the handle that pool's writers notify.
/// Returns false exactly when the typed walk fails for this base.
#[allow(clippy::too_many_arguments)]
fn registrations(
    log: &mut RawLog<AIR_BASES_N>,
    memory: &impl engine::Memory,
    base: usize,
    handle: u64,
    pointer: usize,
    host: bool,
    links: &Array,
    countries: u32,
    seen: &mut Vec<(u32, u32)>,
    seen_index: &mut [usize],
    demand: &mut Option<[usize; 2]>,
    carrier: &mut Option<cair_base::CarrierCapture<'_, impl Memory>>,
    slots: &mut SlotPass<'_>,
    carry: &mut CarryPass<'_>,
) -> bool {
    use crate::air_base_carry_changes::Work;
    let started = qpc();
    carry.reached.clear();
    let Some(store) = carry.store.as_deref_mut().filter(|_| !host) else {
        let read = copy_air_base_registrations(
            log,
            memory,
            base,
            handle,
            pointer,
            links,
            countries,
            seen,
            seen_index,
            demand,
            carrier,
            slots,
            carry.reached,
        );
        carry.work.note(Work::UNCARRIED, started);
        return read;
    };
    store.visit();
    let planned = slots.run.dense && !slots.full.contains(&handle);
    // Registration precedes the read it certifies.
    let certificate = carry.run.changes.sequence(handle);
    let compare = carry.run.check.covers(handle);
    let fresh = compare || (planned && slots.run.check.covers(handle));
    if !fresh && let Some(held) = store.carryable(handle, pointer, certificate, planned) {
        let (rows, segment) = (held.demand, held.segment);
        let spliced = (held.spans.len(), held.bytes.len());
        let resolved = held.links.iter().all(|&(country, slot)| {
            resolve_link(log, memory, links, seen, seen_index, countries, country) == Some(slot)
                && !log.overflowed
        });
        if resolved && log.splice(held.spans, held.bytes) {
            *demand = demand.and_then(|[read, wings]| {
                Some([read.checked_add(rows[0])?, wings.checked_add(rows[1])?])
            });
            store.carried();
            carry.work.splice(spliced.0, spliced.1);
            carry.segment(handle, pointer, segment);
            carry.work.note(Work::CARRIED, started);
            return true;
        }
        if log.overflowed {
            carry.work.note(Work::CARRIED, started);
            return false;
        }
    }
    let (spans, bytes, before) = (log.spans.len(), log.bytes.len(), *demand);
    if !copy_air_base_registrations(
        log,
        memory,
        base,
        handle,
        pointer,
        links,
        countries,
        seen,
        seen_index,
        demand,
        carrier,
        slots,
        carry.reached,
    ) {
        carry.work.note(Work::FRESH, started);
        return false;
    }
    let pools_name_base = carry.reached.pools.iter().all(|&pool| {
        pool.checked_add(engine::cair_wing_pool::AIR_BASE_REFERENCE)
            .and_then(|at| engine::read::<u64>(memory, at))
            == Some(handle)
    });
    let certificate = certificate.filter(|_| pools_name_base);
    // A difference the slot check found this read records the base from the
    // whole scan.
    let planned = slots.run.dense && !slots.full.contains(&handle);
    let read = before
        .zip(*demand)
        .map(|([rows, wings], [after_rows, after_wings])| [after_rows - rows, after_wings - wings]);
    if let Some(read) = read
        && !carry.reached.lost
    {
        let fresh = super::registration_store::Fresh {
            spans: &log.spans[spans..],
            bytes: &log.bytes[bytes..],
            start: bytes,
            links: &carry.reached.links,
            demand: read,
            link_table: links.data..links.data + links.count as usize * 4,
            containers: &carry.reached.containers,
            wings: &carry.reached.wings,
        };
        if let Some(difference) = store.hold(handle, pointer, certificate, planned, &fresh, compare)
        {
            crate::air_base_carry_changes::note_mismatch(handle, carry.run.hour, difference);
            carry.run.changes.mismatch();
        }
        // Only the segment this read was just held as names its bytes.
        if let Some(segment) = store.segment(handle) {
            carry.segment(handle, pointer, segment);
        }
    }
    carry.work.note(Work::FRESH, started);
    true
}

/// Mirror of one base's `registration` subtree: containers, pools and wings
/// with the engine's 64-row batch chunking. Stops at the first typed abort
/// point, mirroring the walk's fail-fast; null containers are measured
/// absence. A base whose dense list proves its occupied slots takes none of
/// its slot vector beyond the plan's slot words, as the replay's walk then
/// lays its chunks out from the plan ([`copy_dense_plan`]). Returns false
/// exactly when the typed walk fails for this base.
#[allow(clippy::too_many_arguments)]
fn copy_air_base_registrations(
    log: &mut RawLog<AIR_BASES_N>,
    memory: &impl engine::Memory,
    base: usize,
    base_handle: u64,
    pointer: usize,
    links: &Array,
    countries: u32,
    seen: &mut Vec<(u32, u32)>,
    seen_index: &mut [usize],
    demand: &mut Option<[usize; 2]>,
    carrier: &mut Option<cair_base::CarrierCapture<'_, impl Memory>>,
    slots: &mut SlotPass<'_>,
    reached: &mut Reached,
) -> bool {
    let header = match bspan(
        log,
        memory,
        pointer.checked_add(cair_base::COUNTRY_CONTAINERS),
        16,
        AIR_REGISTRATIONS_MASK,
    ) {
        Some(start) => start,
        None => return false,
    };
    let mut raw = [0u8; 16];
    raw.copy_from_slice(&log.bytes[header..header + 16]);
    let containers = match Array::from_header(raw, 8) {
        Some(array) => array,
        None => return false,
    };
    if let Some(carrier) = carrier.as_mut() {
        carrier.indexed_header(containers.count);
    }
    let expected_pool = match base.checked_add(cair_wing_pool::VTABLE_RVA) {
        Some(expected) => expected as u64,
        None => return false,
    };
    let expected_primary = match base.checked_add(engine::cair_wing::VTABLE_RVA) {
        Some(expected) => expected as u64,
        None => return false,
    };
    let expected_secondary = match base.checked_add(engine::cair_wing::SECONDARY_VTABLE_RVA) {
        Some(expected) => expected as u64,
        None => return false,
    };
    let expected_container = match base.checked_add(ccountry_air_container::VTABLE_RVA) {
        Some(expected) => expected as u64,
        None => return false,
    };
    let mut plan = [Planned::default(); cair_base::DENSE_BOUND];
    let planned = if slots.run.dense && !slots.full.contains(&base_handle) {
        match copy_dense_plan(
            log,
            memory,
            base,
            pointer,
            &containers,
            links,
            countries,
            seen,
            seen_index,
            &mut plan,
        ) {
            Ok(len) => Some(len),
            Err(refusal) => {
                slots.report.refusals[refusal as usize] += 1;
                None
            }
        }
    } else {
        None
    };
    if log.overflowed {
        return false;
    }
    // The hour's slice also scans a fast-path base whole, outside the log; a
    // difference records the base from the full scan, as the replay will.
    let planned = match planned {
        Some(len) if slots.run.check.covers(base_handle) => {
            slots.report.verified += 1;
            match whole_scan_agrees(memory, &containers, &plan[..len]) {
                Some(true) => Some(len),
                Some(false) => {
                    slots.report.mismatches += 1;
                    if allocation::advisory(|| allocation::reserve(slots.full, 1)).is_err() {
                        return false;
                    }
                    slots.full.push(base_handle);
                    None
                }
                None => {
                    slots.report.unread += 1;
                    Some(len)
                }
            }
        }
        other => other,
    };
    match planned {
        Some(_) => slots.report.dense += 1,
        None => slots.report.full += 1,
    }
    let mut next_planned = 0;
    for chunk_first in (0..containers.count).step_by(REG_MIRROR_BATCH) {
        let count = ((containers.count - chunk_first) as usize).min(REG_MIRROR_BATCH);
        // A planned base's chunk holds exactly its listed containers; any
        // other base's is taken whole.
        let container_chunk = match planned {
            Some(len) => {
                let end = chunk_first + count as u32;
                let first = next_planned;
                while plan[..len]
                    .get(next_planned)
                    .is_some_and(|entry| entry.slot < end)
                {
                    next_planned += 1;
                }
                if first == next_planned {
                    // A chunk naming no listed container is null throughout.
                    continue;
                }
                ChunkWords::Planned(first..next_planned)
            }
            None => {
                let Some(address) = containers.element(chunk_first, 8) else {
                    return false;
                };
                // Retain the completed pointer prefix even when a later
                // element faults: earlier containers still need validation
                // in source order.
                match log.take_array_chunk(memory, address, 8, count, AIR_REGISTRATIONS_MASK) {
                    Some((start, copied)) => ChunkWords::Taken { start, copied },
                    None => return false,
                }
            }
        };
        for offset in 0..count {
            let slot = chunk_first + offset as u32;
            // A listed container's identity and span were taken by the plan.
            let (container, listed) = match &container_chunk {
                ChunkWords::Planned(listed) => {
                    match plan[listed.clone()].iter().find(|entry| entry.slot == slot) {
                        Some(entry) => (entry.container, Some(*entry)),
                        None => continue,
                    }
                }
                ChunkWords::Taken { start, copied } => {
                    if offset >= *copied {
                        // The first failed pointer is already recorded
                        // exactly. Earlier complete slots retain their native
                        // validation/row order.
                        return false;
                    }
                    match usize::try_from(bases_pool_u64(log, start + offset * 8))
                        .ok()
                        .filter(|&pointer| pointer != 0)
                    {
                        Some(container) => (container, None),
                        None => continue,
                    }
                }
            };
            let vtable = match listed {
                Some(entry) => entry.vtable,
                None => match bspan(log, memory, Some(container), 8, AIR_REGISTRATIONS_MASK) {
                    Some(start) => start,
                    None => return false,
                },
            };
            if bases_pool_u64(log, vtable) != expected_container {
                return false;
            }
            let span = match listed {
                Some(entry) => entry.span,
                None => match bspan(
                    log,
                    memory,
                    container.checked_add(ccountry_air_container::COUNTRY),
                    cair_base::CONTAINER_SPAN,
                    AIR_REGISTRATIONS_MASK,
                ) {
                    Some(start) => start,
                    None => return false,
                },
            };
            let country = bases_pool_u32(log, span);
            reached.container(container);
            // The fuel scalars the walk passes the observer from this same span.
            let mut words = [0u8; cair_base::CONTAINER_SPAN];
            words.copy_from_slice(&log.bytes[span..span + cair_base::CONTAINER_SPAN]);
            let (_, fuel) = cair_base::container_span_words(&words);
            // Slot/country agreement mirrors the walk's resolver path exactly:
            // the reserved slot accepts only zero or unlinked ids (a readable
            // zero entry is absence; unreadable entries stay faults), any other
            // slot requires its resolved canonical slot. Link reads are
            // deduplicated per pass like the walk's Resolver cache.
            if slot == 0 {
                if country != 0 {
                    match resolve_link(log, memory, links, seen, seen_index, countries, country) {
                        Some(0) => {}
                        _ => return false,
                    }
                    if log.overflowed {
                        return false;
                    }
                }
            } else {
                match resolve_link(log, memory, links, seen, seen_index, countries, country) {
                    Some(canonical) if canonical == slot => {}
                    _ => return false,
                }
                if log.overflowed {
                    return false;
                }
            }
            if country != 0 {
                reached.link(country, slot);
            }
            if let Some(carrier) = carrier.as_mut() {
                carrier.indexed_container(country, slot, container);
                carrier.begin_registration(fuel);
            }
            let pools_header = match bspan(
                log,
                memory,
                container.checked_add(ccountry_air_container::POOLS),
                16,
                AIR_REGISTRATIONS_MASK,
            ) {
                Some(start) => start,
                None => return false,
            };
            raw.copy_from_slice(&log.bytes[pools_header..pools_header + 16]);
            let pools = match Array::from_header(raw, 8) {
                Some(array) => array,
                None => return false,
            };
            if let Some(carrier) = carrier.as_mut() {
                carrier.registration_header(pools.count);
            }
            let mut registration_wings = Some(0usize);
            *demand = demand.and_then(|[rows, wings]| Some([rows.checked_add(1)?, wings]));
            let mut pool_first = 0u32;
            while pool_first < pools.count {
                let chunk = ((pools.count - pool_first) as usize).min(REG_MIRROR_BATCH);
                let first_address = pools.element(pool_first, 8);
                let block = match bspan(
                    log,
                    memory,
                    first_address,
                    chunk * 8,
                    AIR_REGISTRATIONS_MASK,
                ) {
                    Some(start) => start,
                    None => return false,
                };
                for index in 0..chunk {
                    let pointer = match block_word(log, block, index) {
                        Some(pointer) => pointer,
                        None => return false,
                    };
                    // A null pool pointer inside a counted vector refuses the
                    // table, exactly like the typed walk.
                    if pointer == 0 {
                        return false;
                    }
                    // `copy_records` POOL_FIELDS: vtable word plus wing-vector
                    // header as two exact spans, in field order.
                    let pool_vtable =
                        match bspan(log, memory, Some(pointer), 8, AIR_REGISTRATIONS_MASK) {
                            Some(start) => start,
                            None => return false,
                        };
                    let wings = match bspan(
                        log,
                        memory,
                        pointer.checked_add(engine::cair_wing_pool::WINGS),
                        16,
                        AIR_REGISTRATIONS_MASK,
                    ) {
                        Some(start) => start,
                        None => return false,
                    };
                    if bases_pool_u64(log, pool_vtable) != expected_pool {
                        return false;
                    }
                    reached.pool(pointer);
                    let mut header = [0u8; 16];
                    header.copy_from_slice(&log.bytes[wings..wings + 16]);
                    let members = match Array::from_header(header, 8) {
                        Some(array) => array,
                        None => return false,
                    };
                    registration_wings = registration_wings
                        .and_then(|count| count.checked_add(members.count as usize));
                    if let Some(carrier) = carrier.as_mut() {
                        carrier.begin_pool(pointer, members);
                    }
                    *demand = demand.and_then(|[rows, wings]| {
                        Some([rows, wings.checked_add(members.count as usize)?])
                    });
                    let mut wing_first = 0u32;
                    while wing_first < members.count {
                        let wing_chunk =
                            ((members.count - wing_first) as usize).min(REG_MIRROR_BATCH);
                        let wing_address = members.element(wing_first, 8);
                        let wing_block = match bspan(
                            log,
                            memory,
                            wing_address,
                            wing_chunk * 8,
                            AIR_REGISTRATIONS_MASK,
                        ) {
                            Some(start) => start,
                            None => return false,
                        };
                        let mut wing_pointers = [0usize; REG_MIRROR_BATCH];
                        for (wing_index, destination) in
                            wing_pointers[..wing_chunk].iter_mut().enumerate()
                        {
                            let wing = match block_word(log, wing_block, wing_index) {
                                Some(wing) => wing,
                                None => return false,
                            };
                            if wing == 0 {
                                return false;
                            }
                            *destination = wing;
                        }
                        let mut fields = [(0usize, 0usize); REG_MIRROR_BATCH];
                        let mut complete_wings = 0;
                        for (wing_index, &wing) in wing_pointers[..wing_chunk].iter().enumerate() {
                            // `copy_records` WING_FIELDS: identity32, then the
                            // exact +38 backlink. The complete chunk's copied
                            // prefix precedes any row's semantic validation.
                            let identity =
                                match bspan(log, memory, Some(wing), 32, AIR_REGISTRATIONS_MASK) {
                                    Some(start) => start,
                                    None => break,
                                };
                            reached.wing(wing);
                            let backlink = match bspan(
                                log,
                                memory,
                                wing.checked_add(engine::cair_wing::POOL),
                                8,
                                AIR_REGISTRATIONS_MASK,
                            ) {
                                Some(start) => start,
                                None => break,
                            };
                            fields[wing_index] = (identity, backlink);
                            complete_wings += 1;
                        }
                        if log.overflowed {
                            return false;
                        }
                        // copy_records validates its completely copied prefix
                        // before reporting a later field failure. Feed that same
                        // authenticated prefix; the incomplete pool is still
                        // discarded by the original registration failure.
                        for (wing_index, &(identity, backlink)) in
                            fields[..complete_wings].iter().enumerate()
                        {
                            if bases_pool_u64(log, identity) != expected_primary
                                || bases_pool_u64(log, identity + 16) != expected_secondary
                            {
                                return false;
                            }
                            if bases_pool_u64(log, backlink) != pointer as u64
                                || bases_pool_u64(log, identity + 24) == 0
                            {
                                return false;
                            }
                            if let Some(carrier) = carrier.as_mut() {
                                carrier.wing(wing_pointers[wing_index]);
                            }
                        }
                        if complete_wings != wing_chunk {
                            return false;
                        }
                        wing_first += wing_chunk as u32;
                    }
                    if let Some(carrier) = carrier.as_mut() {
                        carrier.finish_pool();
                    }
                }
                if log.overflowed {
                    return false;
                }
                pool_first += chunk as u32;
            }
            if let Some(carrier) = carrier.as_mut() {
                carrier.finish_registration(registration_wings);
            }
            if log.overflowed {
                return false;
            }
        }
    }
    true
}

/// One structural copy pass: shared census, manager with base vectors,
/// per-base fields and registrations, then ace rosters. Phases are
/// independent: a validation or take failure stops only the takes belonging
/// to walks that would stop there (tracked with alive flags), so every
/// replay derives its own verdict from the recorded spans. The aces phase
/// runs last on the shared census takes. The job is cleared on entry so
/// pooled refreshes never accumulate spans across hours.
#[cfg(test)]
pub(super) fn copy_air_bases_pass(
    job: &mut AirBasesJob,
    memory: &impl engine::Memory,
    base: usize,
    game_state: usize,
    aces_active: bool,
    ticks: &mut [u64; AIR_BASES_N],
) {
    copy_air_bases_pass_with_context(job, memory, base, game_state, aces_active, ticks);
}

#[cfg(test)]
fn copy_air_bases_pass_with_context(
    job: &mut AirBasesJob,
    memory: &impl engine::Memory,
    base: usize,
    game_state: usize,
    aces_active: bool,
    ticks: &mut [u64; AIR_BASES_N],
) {
    copy_air_bases_pass_mode::<false>(job, memory, base, game_state, aces_active, ticks);
}

fn copy_air_bases_pass_with_context_growing(
    job: &mut AirBasesJob,
    memory: &impl engine::Memory,
    base: usize,
    game_state: usize,
    aces_active: bool,
    ticks: &mut [u64; AIR_BASES_N],
) {
    copy_air_bases_pass_mode::<true>(job, memory, base, game_state, aces_active, ticks);
}

fn copy_air_bases_pass_mode<const GROWING: bool>(
    job: &mut AirBasesJob,
    memory: &impl engine::Memory,
    base: usize,
    game_state: usize,
    aces_active: bool,
    ticks: &mut [u64; AIR_BASES_N],
) {
    let started = qpc();
    job.clear();
    if GROWING {
        job.log.enable_worker_growth();
    }
    job.game_state = game_state;
    job.aces_active = aces_active;
    let census_mask = if aces_active {
        AIR_BASES_CENSUS_MASK
    } else {
        BASE_VECTORS_MASK
    };
    let census = read_accounting::with_table(TableId::AirBases, || {
        copy_air_bases_census(
            &mut job.log,
            memory,
            base,
            game_state,
            &mut job.census,
            [&mut job.required_census, &mut job.required_links],
            census_mask,
        )
    });
    ticks[0] += elapsed(started);
    if job.log.overflowed {
        return;
    }
    let (links, countries) = match census {
        Some(census) => census,
        None => {
            // Census headers undiscoverable: the manager/base phases are
            // independent and continue (their replays fail at the same
            // headers); resolve and roster takes are skipped (their replays
            // fail at the headers too).
            copy_air_bases_vectors(job, memory, base, None, ticks);
            return;
        }
    };
    copy_air_bases_vectors(job, memory, base, Some((&links, countries)), ticks);
    if job.log.overflowed || !aces_active {
        return;
    }
    // Aces phase: independent of the base phases, served by the shared
    // census takes plus roster/object/name takes below. Index iteration
    // avoids cloning the census on the game thread.
    let _reads = read_accounting::table(TableId::AirAces);
    let aces_started = qpc();
    let mut ace_index = 0;
    while ace_index < job.census.len() {
        let (pointer, slot) = job.census[ace_index];
        ace_index += 1;
        if !copy_air_aces_country(&mut job.log, memory, base, pointer, slot) {
            break;
        }
        if job.log.overflowed {
            break;
        }
    }
    ticks[2] += elapsed(aces_started);
}

/// Manager, vectors and per-base takes with per-walk alive flags. Shared
/// spans (manager, headers, element pointers, identities) stop both walks on
/// failure; private spans stop only their own walk while the other continues,
/// mirroring each typed walk's fail-fast scope.
#[allow(clippy::too_many_lines, clippy::too_many_arguments)]
fn copy_air_bases_vectors(
    job: &mut AirBasesJob,
    memory: &impl engine::Memory,
    base: usize,
    census: Option<(&Array, u32)>,
    ticks: &mut [u64; AIR_BASES_N],
) {
    let _reads = read_accounting::table(TableId::AirBases);
    let vectors_started = qpc();
    let AirBasesJob {
        log,
        carrier: staged,
        game_state,
        seen_links,
        seen_link_index,
        registration_demand: job_registration_demand,
        slot_run,
        slot_full,
        slot_report,
        carry: carry_run,
        registration_store,
        reached,
        segments,
        registration_work,
        ..
    } = job;
    let mut slots = SlotPass {
        run: *slot_run,
        full: slot_full,
        report: slot_report,
    };
    let mut carry = CarryPass {
        run: *carry_run,
        store: registration_store.as_deref_mut(),
        reached,
        segments,
        work: registration_work,
    };
    // A pass that stops before its end holds nothing.
    let mut complete = false;
    if let Some(store) = carry.store.as_deref_mut() {
        store.begin();
    }
    let carrier::Job {
        log: carrier_log,
        index,
        required_index,
        bases: carrier_bases,
        registrations: carrier_registrations,
        scratch,
        demand: staged_demand,
        seed,
    } = staged;
    let growing = log.worker_growth_enabled();
    if growing {
        carrier_log.get_mut().enable_worker_growth();
    }
    let index = std::cell::RefCell::new(if growing {
        super::carrier_memory::ExactIndex::new_growing(index)
    } else {
        super::carrier_memory::ExactIndex::new(index)
    });
    *seed = super::carrier_memory::seed_census(carrier_log.get_mut(), log, &mut index.borrow_mut());
    if let Err(reason) = *seed {
        carrier::note_seed_refusal(reason);
    }
    *required_index = index.borrow().required_slots();
    let (lengths, demand) = {
        let optional = super::carrier_memory::CaptureMemory::new(carrier_log, memory, &index);
        let countries = seed.and_then(|()| ccountry::Reader::new(&optional, base, *game_state));
        let mut refusal = countries.as_ref().err().copied();
        let mut observer = countries.as_ref().ok().and_then(|countries| {
            let captured = if growing {
                cair_base::CarrierCapture::new_growing(
                    &optional,
                    base,
                    *game_state,
                    countries,
                    carrier::buffers_growing(carrier_bases, carrier_registrations, scratch),
                )
            } else {
                Ok(cair_base::CarrierCapture::new(
                    &optional,
                    base,
                    *game_state,
                    countries,
                    carrier::buffers(carrier_bases, carrier_registrations, scratch),
                ))
            };
            match captured {
                Ok(capture) => Some(capture),
                Err(reason) => {
                    refusal = Some(reason);
                    None
                }
            }
        });
        (|| {
            let manager = match bspan(
                &mut *log,
                memory,
                (*game_state).checked_add(cgame_state::STRATEGIC_AIR_MANAGER),
                8,
                BASE_VECTORS_MASK,
            ) {
                Some(start) => match usize::try_from(bases_pool_u64(&*log, start))
                    .ok()
                    .filter(|&pointer| pointer != 0)
                {
                    Some(manager) => manager,
                    None => {
                        ticks[0] += elapsed(vectors_started);
                        return;
                    }
                },
                None => {
                    ticks[0] += elapsed(vectors_started);
                    return;
                }
            };
            let vtable = bspan(&mut *log, memory, Some(manager), 8, BASE_VECTORS_MASK);
            let expected_manager = base.checked_add(cstrategic_air_manager::VTABLE_RVA);
            let manager_ok = match (vtable, expected_manager) {
                (Some(start), Some(expected)) => bases_pool_u64(&*log, start) == expected as u64,
                _ => false,
            };
            // Four base vectors in walk order, with the walk's indexed flags.
            let mut headers = [(0usize, 0u32, true); 4];
            let mut headers_ok = manager_ok;
            for (slot, (offset, indexed)) in [
                (cstrategic_air_manager::STATE_AIR_BASES, true),
                (cstrategic_air_manager::STATE_ROCKET_BASES, true),
                (cstrategic_air_manager::STATE_GUN_BASES, true),
                (cstrategic_air_manager::STATELESS_AIR_BASES, false),
            ]
            .into_iter()
            .enumerate()
            {
                let header = bspan(
                    &mut *log,
                    memory,
                    manager.checked_add(offset),
                    16,
                    BASE_VECTORS_MASK,
                );
                match header {
                    Some(start) => {
                        let mut raw = [0u8; 16];
                        raw.copy_from_slice(&log.bytes[start..start + 16]);
                        match Array::from_header(raw, 8) {
                            Some(array) => {
                                headers[slot] = (start, array.count, indexed);
                                let _ = array;
                            }
                            None => headers_ok = false,
                        }
                    }
                    None => headers_ok = false,
                }
                if log.overflowed {
                    ticks[0] += elapsed(vectors_started);
                    return;
                }
            }
            let mut bases_alive = headers_ok;
            let mut reg_alive = headers_ok && census.is_some();
            let mut registration_demand = Some([0usize; 2]);
            let mut bases_ticks = 0u64;
            let mut reg_ticks = 0u64;
            for (header_start, count, indexed) in headers {
                if !bases_alive && !reg_alive {
                    break;
                }
                // Re-parse the recorded header (already validated above when
                // headers_ok; re-parse identically when headers failed so the
                // failure scope stays per-walk below).
                let mut raw = [0u8; 16];
                raw.copy_from_slice(&log.bytes[header_start..header_start + 16]);
                let Some(array) = Array::from_header(raw, 8) else {
                    // Both typed walks fail on this header; later vectors are unread
                    // by either replay.
                    break;
                };
                let _ = count;
                for index in 0..array.count {
                    if !bases_alive && !reg_alive {
                        break;
                    }
                    let addr = (index as usize)
                        .checked_mul(8)
                        .and_then(|offset| array.data.checked_add(offset));
                    let element = bspan(&mut *log, memory, addr, 8, BASE_VECTORS_MASK);
                    let Some(element) = element else {
                        bases_alive = false;
                        reg_alive = false;
                        break;
                    };
                    let pointer = match usize::try_from(bases_pool_u64(&*log, element))
                        .ok()
                        .filter(|&pointer| pointer != 0)
                    {
                        Some(pointer) => pointer,
                        None => {
                            if indexed {
                                continue;
                            }
                            bases_alive = false;
                            reg_alive = false;
                            break;
                        }
                    };
                    if bases_alive {
                        let begin = qpc();
                        // Shared identity take (both walks read it); a failure or a
                        // failed identity stops both walks, mirroring both typed
                        // walks. Bases-private fields follow on the bases path only.
                        let identity =
                            bspan(&mut *log, memory, Some(pointer), 16, BASE_VECTORS_MASK);
                        let expected = base.checked_add(cair_base::VTABLE_RVA);
                        let (identity_ok, handle) = match (identity, expected) {
                            (Some(start), Some(expected)) => (
                                bases_pool_u64(&*log, start) == expected as u64
                                    && bases_pool_u64(&*log, start + 8) != 0,
                                bases_pool_u64(&*log, start + 8),
                            ),
                            _ => (false, 0),
                        };
                        if !identity_ok {
                            bases_alive = false;
                            reg_alive = false;
                            bases_ticks += elapsed(begin);
                            if log.overflowed {
                                break;
                            }
                            // Both typed walks fail at this base; later bases are
                            // unread by either replay.
                            break;
                        }
                        if let Some(carrier) = observer.as_mut() {
                            carrier.begin_base(pointer, handle, !indexed);
                        }
                        optional.set_mask(1);
                        match copy_air_base_row(
                            &mut *log,
                            memory,
                            base,
                            pointer,
                            indexed.then_some(index),
                            handle,
                        ) {
                            Some(source) => {
                                if let Some(carrier) = observer.as_mut() {
                                    carrier.authenticated_base(source);
                                }
                            }
                            None => bases_alive = false,
                        }
                        bases_ticks += elapsed(begin);
                        if reg_alive {
                            let _reads = read_accounting::table(TableId::AirBaseRegistrations);
                            let begin = qpc();
                            let (links, countries) = census.expect("reg takes need census links");
                            optional.set_mask(2);
                            if !registrations(
                                &mut *log,
                                memory,
                                base,
                                handle,
                                pointer,
                                !indexed,
                                links,
                                countries,
                                &mut *seen_links,
                                &mut *seen_link_index,
                                &mut registration_demand,
                                &mut observer,
                                &mut slots,
                                &mut carry,
                            ) {
                                reg_alive = false;
                            }
                            reg_ticks += elapsed(begin);
                        }
                    } else if reg_alive {
                        // The bases walk already stopped: the registrations walk
                        // still needs the shared identity take plus its own takes.
                        let _reads = read_accounting::table(TableId::AirBaseRegistrations);
                        let begin = qpc();
                        let identity =
                            bspan(&mut *log, memory, Some(pointer), 16, BASE_VECTORS_MASK);
                        let expected = base.checked_add(cair_base::VTABLE_RVA);
                        let (identity_ok, handle) = match (identity, expected) {
                            (Some(start), Some(expected)) => (
                                bases_pool_u64(&*log, start) == expected as u64
                                    && bases_pool_u64(&*log, start + 8) != 0,
                                bases_pool_u64(&*log, start + 8),
                            ),
                            _ => (false, 0),
                        };
                        if !identity_ok {
                            reg_alive = false;
                            reg_ticks += elapsed(begin);
                            break;
                        }
                        if let Some(carrier) = observer.as_mut() {
                            carrier.begin_base(pointer, handle, !indexed);
                        }
                        optional.set_mask(2);
                        let (links, countries) = census.expect("reg takes need census links");
                        if !registrations(
                            &mut *log,
                            memory,
                            base,
                            handle,
                            pointer,
                            !indexed,
                            links,
                            countries,
                            &mut *seen_links,
                            &mut *seen_link_index,
                            &mut registration_demand,
                            &mut observer,
                            &mut slots,
                            &mut carry,
                        ) {
                            reg_alive = false;
                        }
                        reg_ticks += elapsed(begin);
                    }
                    optional.set_mask(1);
                    let dense_started = qpc();
                    if let Some(carrier) = observer.as_mut() {
                        carrier.finish_base(reg_alive);
                    }
                    bases_ticks += elapsed(dense_started);
                    if log.overflowed {
                        break;
                    }
                }
                if log.overflowed {
                    break;
                }
            }
            if reg_alive && !log.overflowed {
                *job_registration_demand = registration_demand;
            }
            complete = reg_alive && !log.overflowed;
            ticks[0] += elapsed(vectors_started)
                .saturating_sub(bases_ticks)
                .saturating_sub(reg_ticks);
            ticks[0] += bases_ticks;
            ticks[1] += reg_ticks;
        })();
        if let Some(store) = carry.store.as_deref_mut() {
            store.finish(complete);
        }
        if let Some(carrier) = observer.as_mut() {
            carrier.finish_walk();
        }
        observer.as_ref().map_or(
            (
                cair_base::CarrierLengths::default(),
                cair_base::CarrierDemand {
                    failure: refusal,
                    ..Default::default()
                },
            ),
            |carrier| (carrier.lengths(), carrier.demand()),
        )
    };
    *required_index = (*required_index).max(index.borrow().required_slots());
    carrier_bases.install(lengths, demand);
    carrier_registrations.install(lengths, demand);
    *staged_demand = demand;
}

/// Acquire the shared bases, registrations and aces into one owned payload.
pub(crate) fn copy_air_bases_refresh_growing(
    memory: &impl Memory,
    ctx: AirBasesCopyContext,
    job: &mut AirBasesJob,
) -> AirBasesCopyReport {
    let mut ticks = [0u64; AIR_BASES_N];
    // Tests keep the slot settings they chose; production reads the hour's.
    #[cfg(not(test))]
    {
        job.slot_run = crate::air_base_changes::SlotRun::current();
        job.carry = CarryRun::current(ctx.hour);
    }
    copy_air_bases_pass_with_context_growing(
        job,
        memory,
        ctx.base,
        ctx.game_state,
        replay::cadence::should_walk(TableId::AirAces, ctx.hour, ctx.force_full),
        &mut ticks,
    );
    #[cfg(not(test))]
    {
        crate::air_base_changes::record(&job.slot_report);
        crate::air_base_carry_changes::record_work(&job.registration_work);
        if let Some(store) = job.registration_store.as_deref() {
            crate::air_base_carry_changes::record(
                &store.counts(),
                (!job.log.overflowed).then(|| store.bases()),
            );
        }
    }
    AirBasesCopyReport {
        raw_ok: !job.log.overflowed,
        copy_ticks: ticks,
        raw_bytes: {
            let extra = job.carrier.log.borrow().raw_bytes;
            [
                job.log.raw_bytes[0].saturating_add(extra[0]),
                job.log.raw_bytes[1].saturating_add(extra[1]),
                job.log.raw_bytes[2],
            ]
        },
    }
}

#[cfg(test)]
pub(crate) fn copy_air_bases_refresh(
    memory: &impl Memory,
    ctx: AirBasesCopyContext,
    job: &mut AirBasesJob,
) -> AirBasesCopyReport {
    let mut ticks = [0u64; AIR_BASES_N];
    copy_air_bases_pass_with_context(
        job,
        memory,
        ctx.base,
        ctx.game_state,
        replay::cadence::should_walk(TableId::AirAces, ctx.hour, ctx.force_full),
        &mut ticks,
    );
    AirBasesCopyReport {
        raw_ok: !job.log.overflowed,
        copy_ticks: ticks,
        raw_bytes: {
            let extra = job.carrier.log.borrow().raw_bytes;
            [
                job.log.raw_bytes[0].saturating_add(extra[0]),
                job.log.raw_bytes[1].saturating_add(extra[1]),
                job.log.raw_bytes[2],
            ]
        },
    }
}

impl Captured {
    /// Install an accepted worker failure after the ordinary capture was gated.
    /// Registrations are failed separately by their fog owner; no live retry runs.
    pub(crate) fn fail_air_bases_capture(&mut self, reason: &'static str) {
        self.bases.outcome = Outcome::Unavailable { reason };
        self.carrier.reset();
        self.aces.outcome = failed(self.active[3], reason);
        self.names.reset();
        self.bases_raw_mode = false;
    }

    /// Decode-worker interpretation of a raw shared bases payload: replays
    /// the bases and aces walks against the recorded spans into the pooled
    /// buffers (the fog registrations half runs from the same log through
    /// `super::fog::Captured::decode_registrations_payload`). Installs
    /// identical rows, truncation counts and failure reasons.
    pub(crate) fn decode_air_bases_payload(
        &mut self,
        base: usize,
        job: &AirBasesJob,
        decode_ticks: &mut [u64; AIR_BASES_N],
    ) {
        self.active[3] = job.aces_active;
        let recorded = crate::raw::Recorded::build(&job.log);
        let started = qpc();
        let walked = if job.log.worker_growth_enabled() {
            cair_base::walk_growing(&recorded, base, job.game_state, &mut self.bases.rows)
        } else {
            cair_base::walk(&recorded, base, job.game_state, &mut self.bases.rows)
        };
        self.bases.outcome = bounded(walked, self.bases.rows.len());
        decode_ticks[0] += elapsed(started);
        if !job.aces_active {
            self.aces.outcome = SKIPPED;
            self.names.reset();
            self.bases_raw_mode = false;
            return;
        }
        let started = qpc();
        let walked = if job.log.worker_growth_enabled() {
            cace::walk_growing(
                &recorded,
                base,
                job.game_state,
                &mut self.aces.rows,
                &mut self.names.first,
            )
        } else {
            cace::walk(
                &recorded,
                base,
                job.game_state,
                &mut self.aces.rows,
                &mut self.names.first,
            )
        };
        self.aces.outcome = names_outcome(walked, &self.aces.rows, &self.names.first);
        self.names.len = walked.bytes_len;
        self.names.required = walked.required_bytes;
        decode_ticks[2] += elapsed(started);
        self.bases_raw_mode = false;
    }
}
