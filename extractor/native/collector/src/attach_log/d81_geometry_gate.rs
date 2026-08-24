//! d81's UNCHANGED-GEOMETRY GATE: skip the province walk when the copy it would
//! decode is byte-for-byte the copy it decoded last time (lane PW, 2026-08-23).
//!
//! # What this is, and why it is not a heuristic
//!
//! d81's post-bracket compute is dominated by `d81_walk_provinces`, which on the
//! authoritative arm is a PURE FUNCTION of the hour's own copy plus a handful of
//! scalars: it reads no engine byte (a `SnapshotView` holds no memory handle and
//! cannot), it consults no clock, and it has no interior mutability that
//! survives a call. Two invocations over equal inputs therefore produce equal
//! outputs, and that is a proof rather than an observation.
//!
//! So this gate compares INPUTS, never outputs. When this hour's copy and this
//! hour's scalar inputs are exactly the inputs the previous ACCEPTED walk ran
//! over, the rows that walk produced are the rows this walk would produce, and
//! the cached rows are published instead of recomputing them.
//!
//! **This is not a change signal and does not need one.** Lane GN's R8 sweep
//! established that no universal engine-side change signal exists for d47's
//! tree, and nothing here claims one exists for d81's network. The gate needs no
//! cooperation from the engine at all: it observes the hour exactly as before —
//! the copy is still taken, inside the same bracket, under the same closing
//! sequence — and only the DECODE of that copy is reused. The hour d81 publishes
//! is always the hour it observed.
//!
//! # Why the published rows are still this hour's rows
//!
//! The standing ruling that hour misattribution is unacceptable is about
//! publishing an OBSERVATION taken in one hour under another hour's label.
//! Nothing of the sort happens here:
//!
//!  * this hour's copy is taken inside this hour's bracket, as before;
//!  * the bracket's closing sequence runs in full, as before, and a torn or
//!    label-mismatched bracket never reaches this code at all;
//!  * the gate's verdict is a statement about THIS hour's bytes — "the bytes I
//!    just observed are these bytes" — and the rows it then publishes are the
//!    unique rows those bytes decode to.
//!
//! What is reused is arithmetic, not observation. If the engine's rail network
//! genuinely did not change between two hours, two identical row sets is the
//! CORRECT answer, and computing it twice is the only thing this removes.
//!
//! # What is compared, and why it is a superset
//!
//! Everything the walk can read, with no attempt to narrow it to the fields the
//! walk actually touches. Narrowing would re-derive the walk's read set in a
//! second place, and a gate whose notion of "the inputs" drifts from the walk's
//! is exactly the unsound gate this design refuses to be. Concretely:
//!
//!  * EVERY interval of the snapshot, in the snapshot's own order, framed with
//!    its `(start, len)` so an interval SET that changed shape can never compare
//!    equal to one that did not;
//!  * BOTH images of every interval — opening and closing. The walk reads
//!    through `u*_stable_at`, which requires the two images to agree, so a gate
//!    that compared only the opening could call two hours equal when one of them
//!    would have refused a field as torn;
//!  * the scalar walk inputs that do not live in the copy at all
//!    ([`D81GeometryScalars`]), including the topology's province list, since
//!    the walk drives from it -- both for the rows it visits and, since lane
//!    RY deleted the folded set, for the neighbour-validity test it used to
//!    ask a `BTreeSet` derived from the same list.
//!
//! # The four windows that are NOT compared by their bytes, and why that is
//! still sound
//!
//! Measured, not assumed. The first armed capture hit 0 of 230 frames; a stage
//! histogram put every miss in `IntervalOpening`, and a shape probe
//! (`captures/pw_gate3`, 253 misses) reported the first differing window as
//! exactly 888 bytes with exactly 3.996 windows differing per frame. 888 is
//! `D81_OWNER_CONTAINER_OFF + 8` -- the OWNER hop of the copy plan.
//!
//! The cause is `RecipeOp::CopyHeader`, which copies a PREFIX. To read one
//! 8-byte pointer at `owner + 0x370` the plan copies `[owner, owner + 0x378)`,
//! and the 880 bytes in front of it are live game state. The same is true of
//! the gamestate (1,688 bytes for three pointers), the idler (1,336 for one)
//! and the container (120 for a pointer and a count). Four windows, ~4 KB, of
//! which about 40 bytes are ever decoded.
//!
//! So those four windows are excluded from the byte comparison. What is read
//! THROUGH them divides into two cases, and the stronger one covers most of it:
//!
//!  * the IDLER and OWNER prefixes are read from the copy by NOTHING. The copy
//!    plan walks them so the recipe executor can reach the container and the
//!    map array; no d81 reader decodes a byte of either. `d81_walk_provinces`
//!    is not even handed `game_state`, and it receives the container as the
//!    already-resolved `D81GeometryScalars::container` rather than reading it
//!    back out of the copy.
//!  * the GAMESTATE prefix IS read from the copy, by `d81_copied_roots_agree`
//!    and only there. That is re-pinned.
//!
//! Concretely:
//!
//!  * the six words `d81_copied_roots_agree` decodes -- gamestate to rail
//!    manager to rail table, to active root to active table, to selector map to
//!    selector table -- are checked by CALLING that same function on the copy,
//!    so the gate cannot drift from the walk's own check;
//!  * the container's province array and count reach the walk only as
//!    `D81GeometryScalars::container`, which is compared exactly;
//!  * every extent those pointers lead to is still compared whole, and its
//!    `(start, len)` framing is compared first -- so a table that moved or
//!    resized is a miss no matter what the prefix said.
//!
//! What is given up, stated plainly: a change confined to prefix bytes that NO
//! d81 reader decodes no longer forces a walk. That is the entire intended
//! effect. A change to any byte the walk can reach still does.
//!
//! # The active table, compared through the producer contract instead of by
//! bytes
//!
//! Also measured. With the four prefix windows excluded the gate went to 5 hits
//! of 209 frames and every remaining miss differed in exactly ONE interval, of
//! exactly 429,248 bytes -- `13,414 * 0x20`, the ACTIVE TABLE extent.
//!
//! That table was already known to churn, in this file, before this gate
//! existed. `d81_active_state_from_window` says so outright:
//!
//! > The producer applies zero/nonzero predicates to all three scalar fields.
//! > Preserve that exact semantic value rather than requiring a changing
//! > nonzero counter to be byte-identical.
//!
//! The walk reads seven bytes of each 32-byte entry -- `[+0x12, +0x19)` -- and
//! reduces them to four semantic values, three of them zero/nonzero. The
//! counter underneath moves every hour and no reader ever sees its magnitude.
//!
//! So the gate compares that extent entry by entry, through
//! `d81_active_state_from_window` -- the SAME function the walk decodes with.
//! Calling it rather than restating it is the whole safety argument: the gate
//! cannot disagree with the walk about what an active entry means, because the
//! two ask the same function. Bytes outside `[+0x12, +0x19)` are not compared
//! because no reader decodes them; the extent's `(start, len)` framing is still
//! compared first, so a table that moved or resized is a miss regardless.
//!
//! # When the gate refuses to act, and refuses to remember
//!
//! Fail-closed in both directions. The gate is BYPASSED (full walk) and its
//! memory is CLEARED whenever the previous or the current observation is not a
//! clean, complete copy:
//!
//!  * any plan miss, void, pointer rejection or consumer gap on the snapshot —
//!    a copy with a hole decodes to rows whose refusal depends on WHICH hole,
//!    and the cheapest correct answer is not to reason about it;
//!  * a walk that refused — a refusal is not a row set and is never cached;
//!  * any frame in which the reader did not reach the walk at all.
//!
//! A cleared gate costs the next frame a full walk and nothing else, which is
//! the pre-gate behaviour. That is the only failure mode this design has.

use std::cell::Cell;
use std::vec::Vec;

use crate::snapshot_recipes::view::{Snapshot, SnapshotView};

use super::d81_vector_store::D81VectorCapture;
use super::{CompactMapControlTopology, D81MapProvinceContainer, D81PhaseAccum, D81WalkRows};

/// The walk inputs that are NOT bytes of the copy.
///
/// Every one of these is an argument `d81_finish_rail_replay_state` hands to
/// `d81_walk_provinces`, or a value one of those arguments is derived from. They
/// are small and comparing them is free, so they are compared exactly rather
/// than digested.
///
/// THE NEIGHBOUR-VALIDITY TEST NEEDS NOTHING FURTHER. It used to be a
/// `BTreeSet<u32>` folded out of the same province list and handed to the walk
/// beside the topology; lane RY deleted the fold and the walk now asks
/// `CompactMapControlTopology::holds_province`, which reads
/// `topology_province_ids` directly. Equal province lists therefore still imply
/// an equal validity test, and there is now only one place for that to be true
/// rather than two to keep in step.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct D81GeometryScalars {
    pub(super) module_base: usize,
    pub(super) game_state: usize,
    pub(super) rail_table: usize,
    pub(super) active_table: usize,
    pub(super) selector_table: usize,
    pub(super) selector_mask: u32,
    pub(super) selector_max_distance: u8,
    pub(super) expected_cprovince_vtable: usize,
    pub(super) container: D81MapProvinceContainer,
    /// `(province_id, province_ptr, state_id)` per topology root, in the
    /// topology's own order, because the walk iterates it in that order and a
    /// reordering is a different walk.
    pub(super) topology_province_ids: Vec<(u32, usize, u32)>,
    pub(super) topology_province_array: usize,
    pub(super) topology_province_count: u32,
}

impl D81GeometryScalars {
    pub(super) fn capture(
        module_base: usize,
        game_state: usize,
        rail_table: usize,
        active_table: usize,
        selector_table: usize,
        selector_mask: u32,
        selector_max_distance: u8,
        expected_cprovince_vtable: usize,
        container: D81MapProvinceContainer,
        topology: &CompactMapControlTopology,
    ) -> Self {
        Self {
            module_base,
            game_state,
            rail_table,
            active_table,
            selector_table,
            selector_mask,
            selector_max_distance,
            expected_cprovince_vtable,
            container,
            topology_province_ids: topology
                .provinces
                .iter()
                .map(|root| (root.province_id, root.province_ptr, root.state_id))
                .collect(),
            topology_province_array: topology.province_array,
            topology_province_count: topology.province_count,
        }
    }
}

/// THE WALK'S COUNTER SIDE-OUTPUT, carried so a gated frame publishes what a
/// walked frame would.
///
/// `d81_walk_provinces` does not only return rows: it bumps four counters that
/// describe the NETWORK rather than the work — how many provinces it visited,
/// how many were railed, and which of the two `+0x11B8` reciprocity witnesses
/// admitted each container binding. All four are published on the wire, and all
/// four are pure functions of the same inputs the rows are.
///
/// A gated frame that left them at zero would silently deflate every one of
/// those instruments in proportion to the gate's hit rate, which is the
/// "an instrument that changes the thing it measures is not an instrument"
/// failure this file exists to avoid. They are therefore replayed on a hit.
///
/// THE TIMERS ARE DELIBERATELY NOT HERE. `walk_us` and its six sub-phases are
/// wall the gated frame genuinely did not spend, and replaying them would make
/// the phase block claim time that was never taken. They stay zero, and
/// `d81_phase_geometry_gate_hit` is what tells a reader why.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct D81WalkCounters {
    pub(super) provinces_visited: u64,
    pub(super) provinces_railed: u64,
    pub(super) admitted_record_is_province: u32,
    pub(super) admitted_key_is_province_id: u32,
}

impl D81WalkCounters {
    /// Read the counters a walk just produced.
    pub(super) fn capture(
        phases: &D81PhaseAccum,
        admitted_record_is_province: &Cell<u32>,
        admitted_key_is_province_id: &Cell<u32>,
    ) -> Self {
        Self {
            provinces_visited: phases.provinces_visited.get(),
            provinces_railed: phases.provinces_railed.get(),
            admitted_record_is_province: admitted_record_is_province.get(),
            admitted_key_is_province_id: admitted_key_is_province_id.get(),
        }
    }

    /// Put them back on a gated frame.
    ///
    /// ASSIGNED, NOT ADDED. The walk sets these from zero on every frame, so a
    /// gated frame's correct value is the remembered one and not a sum.
    pub(super) fn replay(
        &self,
        phases: &D81PhaseAccum,
        admitted_record_is_province: &Cell<u32>,
        admitted_key_is_province_id: &Cell<u32>,
    ) {
        phases.provinces_visited.set(self.provinces_visited);
        phases.provinces_railed.set(self.provinces_railed);
        admitted_record_is_province.set(self.admitted_record_is_province);
        admitted_key_is_province_id.set(self.admitted_key_is_province_id);
    }
}

/// The remembered observation: the exact bytes the last accepted walk decoded,
/// and the rows it produced from them.
///
/// # Its footprint is the copy's, once
///
/// `image` holds both images of every interval, so it is about twice the
/// snapshot's own byte count — the same order as the copy d81 already takes each
/// frame, held for one extra frame. It is reused in place across frames (the
/// `Vec` keeps its capacity), so the steady state allocates nothing.
pub(super) struct D81GeometryGate {
    /// Framed image of the last accepted observation. Empty when cold.
    image: Vec<u8>,
    /// Present exactly when `image` is populated.
    scalars: Option<D81GeometryScalars>,
    /// The rows `image` decodes to. Present exactly when `image` is populated.
    rows: Option<D81WalkRows>,
    /// The counters that walk bumped. Present exactly when `rows` is.
    counters: D81WalkCounters,
}

impl Default for D81GeometryGate {
    fn default() -> Self {
        Self {
            image: Vec::new(),
            scalars: None,
            rows: None,
            counters: D81WalkCounters::default(),
        }
    }
}

/// The verdict the caller acts on.
pub(super) enum D81GeometryVerdict {
    /// Publish these rows, and replay these counters; the walk is not run.
    Reuse(D81WalkRows, D81WalkCounters),
    /// Run the walk. The gate has already cleared whatever it held, so the
    /// caller only has to hand the result back to [`D81GeometryGate::remember`].
    Walk,
}

impl D81GeometryGate {
    /// Bytes the memo currently holds. Test-facing: a coverage assertion needs
    /// to see that the serialized image grew to cover every sample image.
    pub(super) fn image_len(&self) -> usize {
        self.image.len()
    }

    /// Forget everything. Called on every path that cannot vouch for what the
    /// gate holds; a cleared gate is always correct and only ever costs a walk.
    pub(super) fn clear(&mut self) {
        self.image.clear();
        self.scalars = None;
        self.rows = None;
        self.counters = D81WalkCounters::default();
    }

    /// Is this snapshot a copy the gate may reason about at all?
    ///
    /// Anything less than a complete, defect-free copy is refused: a snapshot
    /// with a void, a rejected pointer, a consumer gap or an unmet sample
    /// decodes to rows whose refusal depends on exactly which byte was missing,
    /// and comparing two such copies is a question this gate does not need to
    /// answer to do its job.
    fn copy_is_clean(
        snapshot: &Snapshot,
        vector: Option<&D81VectorCapture>,
        consumer_gapped: bool,
        plan_misses_total: u32,
    ) -> bool {
        !consumer_gapped
            && plan_misses_total == 0
            && snapshot.voids().is_empty()
            && snapshot.pointer_rejections().is_empty()
            && snapshot.gapped().is_empty()
            && vector.is_none_or(D81VectorCapture::is_defect_free)
    }

    /// Decide, and leave the gate in the right state for what the caller will do
    /// next.
    ///
    /// On `Reuse` the gate keeps what it holds — the inputs are unchanged, so
    /// the memory is still current. On `Walk` the gate is CLEARED before the
    /// caller runs the walk, so a caller that returns early (a refusal, a panic
    /// unwinding through) can never leave stale rows behind a stale image.
    pub(super) fn decide(
        &mut self,
        snapshot: &Snapshot,
        view: &SnapshotView<'_>,
        vector: Option<&D81VectorCapture>,
        scalars: &D81GeometryScalars,
        chain_windows: &[(usize, u64)],
        plan_spans: &[(u64, u64)],
        consumer_gapped: bool,
        plan_misses_total: u32,
        phases: &D81PhaseAccum,
    ) -> D81GeometryVerdict {
        if !Self::copy_is_clean(snapshot, vector, consumer_gapped, plan_misses_total) {
            D81PhaseAccum::bump(&phases.geometry_gate_refused, 1);
            self.clear();
            return D81GeometryVerdict::Walk;
        }
        D81PhaseAccum::bump(&phases.geometry_gate_eligible, 1);
        if self.rows.is_none() {
            D81PhaseAccum::bump(&phases.geometry_gate_cold, 1);
            self.clear();
            return D81GeometryVerdict::Walk;
        }
        if self.scalars.as_ref() != Some(scalars) {
            D81PhaseAccum::bump(&phases.geometry_gate_miss, 1);
            phases.bump_gate_stage(D81GateStage::Scalars);
            self.clear();
            return D81GeometryVerdict::Walk;
        }
        let mut compared = 0u64;
        let mut first_difference = None;
        let mut shape = D81GateMissShape::default();
        let matched = self.image_matches(
            snapshot,
            view,
            vector,
            chain_windows,
            scalars.active_table,
            plan_spans,
            &mut compared,
            &mut first_difference,
            &mut shape,
        );
        D81PhaseAccum::bump(&phases.geometry_gate_compared_bytes, compared);
        if !matched {
            D81PhaseAccum::bump(&phases.geometry_gate_miss, 1);
            if let Some(stage) = first_difference {
                phases.bump_gate_stage(stage);
            }
            phases.record_gate_miss_shape(shape);
            self.clear();
            return D81GeometryVerdict::Walk;
        }
        // THE RE-PIN. The four prefix windows were not compared byte for byte,
        // so the six words the walk decodes THROUGH them are checked here by
        // calling the walk's own check on this frame's copy. Calling
        // `d81_copied_roots_agree` rather than restating it is the point: the
        // gate cannot drift from the check it stands in for, because it IS the
        // check.
        //
        // It reads six 8-byte words out of the copy and touches no engine
        // memory, so it is paid only on a candidate hit and costs nothing a
        // walked frame would not also have paid.
        let source = super::d81_copy_bracket::D81ReadSource::snapshot(*view, vector);
        // AND THE TWO ARRAY HEADERS (Codex r5). `d81_copied_roots_agree` pins
        // the three table roots; the province arrays are reached by a different
        // pair of headers and were not pinned at all. A copy whose source- or
        // map-array header disagrees with the scalars the walk drives from is a
        // copy the walk would have refused, so the gate must refuse it too --
        // and the disagreement is invisible in the compared bytes when the
        // header sits in a prefix window the exclusions skip.
        let arrays_agree = |base: usize, ptr_off: usize, count_off: usize, ptr: usize, count: u32| {
            source
                .u64_at(
                    base.saturating_add(ptr_off),
                    super::d81_copy_bracket::D81Image::Opening,
                )
                .is_some_and(|value| value as usize == ptr)
                && source
                    .u32_at(
                        base.saturating_add(count_off),
                        super::d81_copy_bracket::D81Image::Opening,
                    )
                    .is_some_and(|value| value == count)
        };
        let arrays_pinned = arrays_agree(
            scalars.game_state,
            super::CGAMESTATE_CPROVINCE_ARRAY,
            super::CGAMESTATE_CPROVINCE_COUNT,
            scalars.topology_province_array,
            scalars.topology_province_count,
        ) && arrays_agree(
            scalars.container.container,
            super::D81_CONTAINER_PROVINCE_ARRAY_OFF,
            super::D81_CONTAINER_PROVINCE_COUNT_OFF,
            scalars.container.province_array,
            scalars.container.province_count,
        );
        if !arrays_pinned
            || super::d81_copied_roots_agree(
                &source,
                scalars.game_state,
                scalars.rail_table,
                scalars.active_table,
                scalars.selector_table,
            )
            .is_err()
        {
            D81PhaseAccum::bump(&phases.geometry_gate_miss, 1);
            phases.bump_gate_stage(D81GateStage::ChainRoots);
            self.clear();
            return D81GeometryVerdict::Walk;
        }
        D81PhaseAccum::bump(&phases.geometry_gate_hit, 1);
        // Cloned, not taken: the gate must still hold the image's rows for the
        // NEXT frame, which will compare against the same image.
        let rows = self
            .rows
            .as_ref()
            .expect("rows present: checked immediately above")
            .clone_rows();
        D81GeometryVerdict::Reuse(rows, self.counters)
    }

    /// Remember an accepted walk's inputs and its rows.
    ///
    /// Only ever called with rows the walk actually produced from THIS
    /// snapshot; a refused walk calls [`clear`](Self::clear) instead.
    pub(super) fn remember(
        &mut self,
        snapshot: &Snapshot,
        view: &SnapshotView<'_>,
        vector: Option<&D81VectorCapture>,
        scalars: D81GeometryScalars,
        chain_windows: &[(usize, u64)],
        plan_spans: &[(u64, u64)],
        rows: &D81WalkRows,
        counters: D81WalkCounters,
        consumer_gapped: bool,
        plan_misses_total: u32,
    ) {
        if !Self::copy_is_clean(snapshot, vector, consumer_gapped, plan_misses_total) {
            self.clear();
            return;
        }
        self.image.clear();
        let active_table = scalars.active_table;
        if !self.build_image(snapshot, view, vector, chain_windows, active_table, plan_spans)
        {
            // An interval the view will not hand back cannot be remembered, and
            // a partial image must never be compared against.
            self.clear();
            return;
        }
        self.scalars = Some(scalars);
        self.rows = Some(rows.clone_rows());
        self.counters = counters;
    }

}

/// WHICH PART OF THE OBSERVATION FIRST DIFFERED (lane PW).
///
/// The first live capture with the gate armed hit 0 of 230 frames, while the
/// PUBLISHED geometry rows are 93-98% identical on three sealed spools. That gap
/// is the gate's own residual made visible: the gate keys on the COPY, which is
/// a strict superset of what the rows encode, so any volatile byte inside a
/// copied extent that the walk never decodes costs a hit without costing
/// correctness. "0 hits" does not say WHICH byte, and the fix must not be
/// guessed at; these stages name it.
///
/// Ordinals are stable and each is published as its own wire counter, so a
/// capture reports the distribution rather than one sample.
/// WHERE, inside the copy, a miss first differed.
///
/// The first armed capture (`captures/pw_gate2`, 406 frames) put all 405 misses
/// in `IntervalOpening`: the interval COUNT, the per-interval `(start, len)`
/// framing, the vector capture and the scalars all matched exactly, hour after
/// hour, and only the copied BYTES differed. So the copy's shape is stable --
/// no allocation moved, no table was resized -- and something inside a copied
/// window is simply volatile.
///
/// That narrows the question to "which window", and these three sums answer it
/// without a new wire unit kind. They are summed over frames like every other
/// d81 unit, so dividing by `geometry_gate_miss` gives means; and because a
/// copy plan's windows have distinctive fixed lengths, a mean `len` that comes
/// out an exact integer identifies the table on its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct D81GateMissShape {
    /// How many intervals differed in this observation, not just the first.
    /// One differing interval means a single volatile field; thousands means a
    /// whole table churns.
    pub(super) differing_intervals: u64,
    /// The first differing interval's length.
    pub(super) first_len: u64,
    /// The offset of the first differing byte INSIDE THE SERIALIZED RUN it was
    /// found in - not inside the source interval (Codex r2 #2).
    ///
    /// For an ordinary record the two coincide. For the ACTIVE TABLE they do
    /// not: that extent is serialized as five reduced bytes per 32-byte entry,
    /// so entry `p`\'s `active_a` differs at run offset `5*p` where its source
    /// offset is `32*p + 0x12 + 2`. A framing mismatch likewise reports an
    /// offset inside the framing run. The unit is named
    /// `..._miss_first_run_offset` on the wire to say so.
    pub(super) first_offset: u64,
    /// When the FIRST difference landed in [`D81GateStage::Vector`], which BIN
    /// of [`super::d81_vector_store::D81_VECTOR_STORE_NAMES`] it was in -- a
    /// store index in `0..D81_VECTOR_STORE_COUNT`, or the trailing `framing`
    /// bin for the capture's own framing.
    ///
    /// [`super::d81_vector_store::D81_VECTOR_STORE_UNATTRIBUTED`] when the miss
    /// was NOT in that stage, which is the value the default carries and the
    /// one the accumulator declines to bin. Every miss therefore appears in
    /// `geometry_gate_miss_stage` and only a vector miss appears here, so the
    /// two histograms agree by construction rather than by arithmetic.
    ///
    /// Lane TD. `rc_vanilla4` put 125 of 159 misses in `Vector`, and that stage
    /// covers all nine families plus the two closing stores, so the capture
    /// could not say whether the churn was the rail pairs (whose only in-place
    /// mutator is `CProvinceRailwayInfo::SetRailway`, claim
    /// `claim_19f6d2c17f8bc5f17460`) or the active window (`SProvinceSupplyData`,
    /// claim `claim_d48c7f58b39be669efec`). That is the difference between a
    /// partial rail predicate being worth pricing and candidate (a) being dead;
    /// see `runs/td_rail_change_signal/DOSSIER.md` sections 6 and 7.
    pub(super) first_vector_store: usize,
}

impl Default for D81GateMissShape {
    fn default() -> Self {
        Self {
            differing_intervals: 0,
            first_len: 0,
            first_offset: 0,
            // NOT `usize::default()`, which is 0 and is a real store index
            // (`RailHeader`). A derived `Default` would report every non-vector
            // miss as a rail-header miss.
            first_vector_store: super::d81_vector_store::D81_VECTOR_STORE_UNATTRIBUTED,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum D81GateStage {
    /// The scalars, compared before the image is walked at all.
    Scalars = 0,
    /// The number of snapshot intervals.
    IntervalCount = 1,
    /// An interval's `(start, len, has_closing)` framing -- the copy's SHAPE
    /// changed, not its contents. A heap address that moved lands here.
    IntervalFraming = 2,
    /// An interval's opening image bytes.
    IntervalOpening = 3,
    /// An interval's closing image bytes.
    IntervalClosing = 4,
    /// The vector capture's presence flag or its content.
    Vector = 5,
    /// Everything fed matched, but the remembered image was longer.
    TrailingLength = 6,
    /// Every compared byte matched, but the six words the walk decodes THROUGH
    /// the uncompared prefix windows did not agree with the scalars. See the
    /// re-pin in `decide`.
    ChainRoots = 7,
}

impl D81GateStage {
    pub(super) const COUNT: usize = 8;

    pub(super) fn ordinal(self) -> usize {
        self as usize
    }
}

/// Hand every byte of the observation's CONTENT to `sink`, in a fixed order.
///
/// One function, used to BUILD the remembered image and to COMPARE against it,
/// because two of them would be two orderings to keep in step and a gate whose
/// build and compare disagree is a gate that fires on inputs that differ.
///
/// The framing per snapshot interval is `(start, len, has_closing, opening[,
/// closing])`, in the snapshot's own order, with the interval COUNT first so a
/// prefix of a longer image cannot compare equal to a shorter one. The vector
/// capture's own framing is [`D81VectorCapture::feed_content`]'s.
///
/// Returns false when the view refused an interval; the caller must then treat
/// the observation as unrememberable rather than compare a partial image.
fn feed_observation(
    snapshot: &Snapshot,
    view: &SnapshotView<'_>,
    vector: Option<&D81VectorCapture>,
    chain_windows: &[(usize, u64)],
    active_table: usize,
    plan_spans: &[(u64, u64)],
    fed_bytes: &mut u64,
    sink: &mut dyn FnMut(D81GateStage, usize, &[u8]),
) -> bool {
    let intervals = snapshot.intervals();
    feed_run(
        sink,
        fed_bytes,
        D81GateStage::IntervalCount,
        usize::MAX,
        &(intervals.len() as u64).to_le_bytes(),
    );
    for (ordinal, interval) in intervals.iter().enumerate() {
        let start = interval.start();
        let len = interval.len();
        let span_end = start.saturating_add(len);

        // WHETHER THIS INTERVAL IS EXCLUSIVELY WHAT IT LOOKS LIKE (Codex r1 #2).
        //
        // The substrate deduplicates overlapping requests, so an interval whose
        // `(start, len)` matches a chain prefix is not thereby PROVEN to hold
        // only that prefix -- another recipe operation can be served from the
        // same record. Both exclusions below are therefore gated on the
        // interval being disjoint from every other interval in the snapshot: a
        // record nothing else overlaps can only hold what its own operation put
        // there.
        //
        // FAIL-CLOSED, and cheaply: an overlapping interval is simply compared
        // by bytes like any other, which costs a miss (a walk) and never a
        // wrong row. The snapshot holds ~14 intervals, so the scan is free.
        // A chain prefix's own span is exactly the record: the plan copies the
        // prefix and the record is that copy.
        let is_chain_window = chain_windows
            .iter()
            .any(|(base, prefix_len)| *base as u64 == start && *prefix_len == len)
            && record_holds_one_role(plan_spans, start, len, (start, span_end));
        // The active table's own span STARTS at the record but may be longer
        // than it, if the ownership map split the extent; the record is then a
        // piece of that one extent, which is still one role.
        let is_active_table = start == active_table as u64
            && plan_spans
                .iter()
                .copied()
                .find(|(span_start, _)| *span_start == start)
                .map(|own| own.1 >= span_end && record_holds_one_role(plan_spans, start, len, own))
                .unwrap_or(false);

        // EVERY IMAGE, NOT JUST TWO (Codex r1 #1).
        //
        // The recipe takes ADJACENT SAMPLES -- two per ordinary interval, three
        // across the active table -- and the walk reads through `u*_stable_at`,
        // which compares an image against its PAIRED sample. Serializing only
        // the opening and the closing left those samples out of the key, so two
        // observations whose opening and closing agreed could differ in a
        // sample: the gate would hit while a full walk saw the disagreement and
        // REFUSED. `DisagreeAction::Record` means such a disagreement need not
        // gap the consumer, so `copy_is_clean` did not close it either.
        let opening_images = view.sample_count(start);
        let closing_images = if interval.closing_offset().is_some() {
            snapshot
                .annotations()
                .closing_images(interval.id())
                .map(|images| images.held())
                .unwrap_or(1)
        } else {
            0
        };

        // Framing first, and always -- even for a skipped window. "Which
        // records exist, where, how long, and how many images each holds" is
        // the copy's SHAPE, and the shape is never excused.
        feed_run(
            sink,
            fed_bytes,
            D81GateStage::IntervalFraming,
            ordinal,
            &start.to_le_bytes(),
        );
        feed_run(
            sink,
            fed_bytes,
            D81GateStage::IntervalFraming,
            ordinal,
            &len.to_le_bytes(),
        );
        feed_run(
            sink,
            fed_bytes,
            D81GateStage::IntervalFraming,
            ordinal,
            &[opening_images as u8, closing_images as u8],
        );
        if is_chain_window {
            continue;
        }

        for image in 0..opening_images {
            let Ok(bytes) = view.sample_bytes(start, len as usize, image) else {
                return false;
            };
            if is_active_table {
                feed_active_entries(D81GateStage::IntervalOpening, ordinal, bytes, sink, fed_bytes);
            } else {
                feed_run(sink, fed_bytes, D81GateStage::IntervalOpening, ordinal, bytes);
            }
        }
        for image in 0..closing_images {
            let Ok(bytes) = view.closing_sample_bytes(start, len as usize, image) else {
                return false;
            };
            if is_active_table {
                feed_active_entries(D81GateStage::IntervalClosing, ordinal, bytes, sink, fed_bytes);
            } else {
                feed_run(sink, fed_bytes, D81GateStage::IntervalClosing, ordinal, bytes);
            }
        }
    }
    // THE VECTOR CAPTURE IS PART OF THE COPY, and leaving it out would be one
    // way this gate could be unsound. Lane HS moved d81's scattered
    // per-province families off the recipe's per-province branch and into flat
    // slabs carried beside the snapshot; `D81ReadSource::snapshot` is handed
    // both, and the walk reads from whichever holds the span.
    feed_run(
        sink,
        fed_bytes,
        D81GateStage::Vector,
        usize::MAX,
        &[u8::from(vector.is_some())],
    );
    if let Some(vector) = vector {
        let mut fed = 0u64;
        // THE ORDINAL CARRIES THE VECTOR STORE, not an interval index (lane
        // TD). Every other stage's ordinal indexes `snapshot.intervals()`; this
        // one indexes `D81VectorCapture::stores()`, because the vector capture
        // has no intervals and the bin it needed was "which family". The one
        // place that reads the ordinal without knowing the stage --
        // `interval_len_at` -- is guarded on this stage in `image_matches`.
        vector.feed_content(&mut |store, bytes| {
            fed += bytes.len() as u64;
            sink(D81GateStage::Vector, store, bytes);
        });
        *fed_bytes += fed;
    }
    true
}

/// Does this record hold ONLY the role it looks like?
///
/// THE GUARD ON BOTH EXCLUSIONS, at its fourth form. The three earlier ones are
/// recorded because each was wrong in a way that looked right, and the tests
/// below carry all three geometries so none can come back:
///
///  * "does this record overlap another RECORD" is a TAUTOLOGY. `sweep_plan`
///    subtracts existing ownership and creates records only for uncovered
///    holes, so the final records never overlap (Codex r2).
///  * "does another role's BASE begin inside this record" misses a CONTAINING
///    extent: `sweep_plan` preserves the earlier record and fills only the
///    holes around it, so an extent decoded later can contain an intact record
///    while beginning below it (Codex r3).
///  * "no OTHER span overlaps, where other means not equal to mine" cannot see
///    an EXACT DUPLICATE. Two roles whose spans coincide are indistinguishable
///    by bounds, and both compare equal to the record's own span, so both were
///    excused (Codex r4).
///
/// What holds is a COUNT. The record is accepted only if exactly ONE span in
/// the plan's pre-dedup list overlaps it, and that span is the record's own.
/// Two roles over the same bytes are then two overlaps and the record is
/// refused, whether their bounds coincide or not — which is the property the
/// bounds comparison could not express.
///
/// The list is the recipe's own account of what it asked for, carried out of
/// `D81PlanObservation` rather than re-derived, so it cannot drift from the
/// copy. It must therefore contain each span exactly once: the caller unions in
/// only the hops `bulk_spans` omits, never the ones it already holds, or an
/// ordinary record would count two overlaps and no exclusion would ever fire.
///
/// Fail-closed and cheap: a record that fails this is compared by bytes like
/// any other, which costs a miss (a walk) and never a wrong row. A record no
/// span accounts for is refused, and so is every record when the list is empty.
pub(super) fn record_holds_one_role(
    plan_spans: &[(u64, u64)],
    start: u64,
    len: u64,
    own: (u64, u64),
) -> bool {
    let end = start.saturating_add(len);
    let mut overlaps = 0usize;
    let mut saw_own = false;
    for &(span_start, span_end) in plan_spans {
        // A ZERO-LENGTH SPAN STILL NAMES A ROLE (Codex r5). Its count came out
        // zero from a header the copy read, but the operation is real and the
        // walk can still read through the record those bytes live in --
        // snapshot ownership is consumer-level, not per-operation, so a
        // zero-sized request is served from whatever record covers its base.
        // Compared by the ordinary half-open rule such a span vanishes at a
        // record's START (`start >= span_end` holds when both equal the base),
        // which is exactly where a second role would hide. Its base is
        // therefore treated as touching the record whenever it lies at or
        // inside it -- deliberately conservative at the far edge, because
        // over-counting only costs a walk.
        let touches = if span_start == span_end {
            span_start >= start && span_start <= end
        } else {
            span_start < end && start < span_end
        };
        if !touches {
            continue;
        }
        overlaps += 1;
        if overlaps > 1 {
            return false;
        }
        saw_own = (span_start, span_end) == own;
    }
    overlaps == 1 && saw_own
}

/// Hand one run of bytes to the sink and charge it to the compared-bytes total.
///
/// The total counts SERIALIZED bytes -- framing, reduced active state and
/// vector content included -- because that is what the comparison actually
/// walks (Codex r1 #3). It is not the sum of raw interval lengths.
#[inline]
fn feed_run(
    sink: &mut dyn FnMut(D81GateStage, usize, &[u8]),
    fed_bytes: &mut u64,
    stage: D81GateStage,
    ordinal: usize,
    bytes: &[u8],
) {
    *fed_bytes += bytes.len() as u64;
    sink(stage, ordinal, bytes);
}

/// Feed one active-table image as the SEMANTIC state of each entry.
///
/// The walk decodes `[entry + 0x12, entry + 0x19)` and reduces it through
/// `d81_active_state_from_window`; everything else in the 32-byte entry is
/// bytes no d81 reader reads. Feeding the reduction is therefore feeding
/// exactly the walk's input, and it is fed through the walk's own function so
/// the two cannot drift.
///
/// ONE SINK CALL PER IMAGE, not per entry (Codex r1 #3): the miss-shape
/// counters are scoped to an interval, and a reduction that called the sink
/// 13,414 times would have reported one differing interval as thousands and
/// published a `first_len` of 5.
///
/// A trailing partial entry cannot occur -- the extent is a whole number of
/// strides -- but is appended verbatim if it somehow does: unexplained bytes
/// are compared, never ignored.
fn feed_active_entries(
    stage: D81GateStage,
    ordinal: usize,
    bytes: &[u8],
    sink: &mut dyn FnMut(D81GateStage, usize, &[u8]),
    fed_bytes: &mut u64,
) {
    let stride = super::D81_ACTIVE_ENTRY_STRIDE;
    let offset = super::D81_ACTIVE_WINDOW_OFFSET;
    let window_len = super::D81_ACTIVE_WINDOW_LEN;
    let mut reduced = Vec::with_capacity(bytes.len() / stride.max(1) * 5 + 8);
    let mut at = 0usize;
    while at + stride <= bytes.len() {
        let entry = &bytes[at..at + stride];
        let mut window = [0u8; super::D81_ACTIVE_WINDOW_LEN];
        window.copy_from_slice(&entry[offset..offset + window_len]);
        let state = super::d81_active_state_from_window(window);
        reduced.extend_from_slice(&[
            state.active_a,
            state.active_b,
            state.active_flags,
            (state.active_u16 & 0xFF) as u8,
            (state.active_u16 >> 8) as u8,
        ]);
        at += stride;
    }
    if at < bytes.len() {
        reduced.extend_from_slice(&bytes[at..]);
    }
    *fed_bytes += reduced.len() as u64;
    sink(stage, ordinal, &reduced);
}

impl D81GeometryGate {
    /// Serialise this observation into `self.image`.
    fn build_image(
        &mut self,
        snapshot: &Snapshot,
        view: &SnapshotView<'_>,
        vector: Option<&D81VectorCapture>,
        chain_windows: &[(usize, u64)],
        active_table: usize,
        plan_spans: &[(u64, u64)],
    ) -> bool {
        let mut image = std::mem::take(&mut self.image);
        let mut fed = 0u64;
        let complete = feed_observation(
            snapshot,
            view,
            vector,
            chain_windows,
            active_table,
            plan_spans,
            &mut fed,
            &mut |_stage, _ordinal, bytes| image.extend_from_slice(bytes),
        );
        self.image = image;
        complete
    }

    /// Compare this observation against the remembered image WITHOUT
    /// materialising it.
    ///
    /// The comparison is exact bytes, never a digest: a digest would need a
    /// collision argument, and on the one path where this gate can be wrong --
    /// republishing the previous rows for an hour whose network moved --
    /// "almost certainly not" is not the standard. An exact compare has no such
    /// path.
    ///
    /// `feed_observation` has no early exit, so a mismatch keeps walking the
    /// rest of the observation rather than stopping at the first differing
    /// byte. That costs a miss the remainder of one linear pass and buys the
    /// single ordering the build shares; misses are the minority case
    /// (`runs/pw_d81_split/MEASUREMENT.md`), so the trade is priced.
    fn image_matches(
        &self,
        snapshot: &Snapshot,
        view: &SnapshotView<'_>,
        vector: Option<&D81VectorCapture>,
        chain_windows: &[(usize, u64)],
        active_table: usize,
        plan_spans: &[(u64, u64)],
        compared: &mut u64,
        first_difference: &mut Option<D81GateStage>,
        shape: &mut D81GateMissShape,
    ) -> bool {
        let image = &self.image;
        let mut cursor = 0usize;
        let mut equal = true;
        // INTERVAL SCOPE, not sink-call scope (Codex r1 #3). One interval that
        // differs counts once however many runs it was fed in, and the recorded
        // length is the INTERVAL's, not the run's.
        let mut last_counted: Option<usize> = None;
        let complete = feed_observation(
            snapshot,
            view,
            vector,
            chain_windows,
            active_table,
            plan_spans,
            compared,
            &mut |stage, ordinal, bytes| {
                let at = cursor;
                if take_eq(image, &mut cursor, bytes) {
                    return;
                }
                let offset = first_difference_offset(image, at, bytes);
                // Resynchronise so later runs are still compared.
                cursor = at.saturating_add(bytes.len());
                let is_interval_content = matches!(
                    stage,
                    D81GateStage::IntervalOpening | D81GateStage::IntervalClosing
                );
                if is_interval_content && last_counted != Some(ordinal) {
                    last_counted = Some(ordinal);
                    shape.differing_intervals += 1;
                }
                if equal {
                    equal = false;
                    *first_difference = Some(stage);
                    // THE VECTOR STAGE'S ORDINAL IS NOT AN INTERVAL INDEX (lane
                    // TD). Every other stage either carries a real interval
                    // ordinal or `usize::MAX`, which `interval_len_at` returns
                    // `None` for; the vector stage now carries a store index in
                    // 0..11, which would have named an unrelated interval's
                    // length. Guarding on the stage rather than on the range is
                    // what keeps that true if either numbering grows.
                    let interval_len = if matches!(stage, D81GateStage::Vector) {
                        shape.first_vector_store =
                            super::d81_vector_store::d81_vector_store_bin(ordinal);
                        None
                    } else {
                        interval_len_at(snapshot, ordinal)
                    };
                    shape.first_len = interval_len.unwrap_or(bytes.len() as u64);
                    shape.first_offset = offset as u64;
                }
            },
        );
        if complete && equal && cursor != image.len() {
            *first_difference = Some(D81GateStage::TrailingLength);
        }
        // A remembered image with bytes left over describes a DIFFERENT
        // observation that happens to share a prefix, which is a miss.
        complete && equal && cursor == image.len()
    }
}

/// The source interval's own length, for the miss-shape report.
///
/// `usize::MAX` is the ordinal the non-interval stages carry; those have no
/// interval and fall back to the fed run's length at the call site.
fn interval_len_at(snapshot: &Snapshot, ordinal: usize) -> Option<u64> {
    snapshot.intervals().get(ordinal).map(|record| record.len())
}


/// Offset of the first byte of `want` that differs from `image[at..]`.
///
/// Returns 0 when the image is too short to hold `want` at all -- that is a
/// framing difference rather than a content one, and the stage already says so.
fn first_difference_offset(image: &[u8], at: usize, want: &[u8]) -> usize {
    for (index, byte) in want.iter().enumerate() {
        match image.get(at + index) {
            Some(seen) if seen == byte => {}
            _ => return index,
        }
    }
    0
}

#[inline]
fn take_eq(image: &[u8], cursor: &mut usize, want: &[u8]) -> bool {
    let end = match cursor.checked_add(want.len()) {
        Some(end) if end <= image.len() => end,
        _ => return false,
    };
    if &image[*cursor..end] != want {
        return false;
    }
    *cursor = end;
    true
}

impl D81WalkRows {
    /// A deep copy of the rows.
    ///
    /// `D81WalkRows` is deliberately not `Clone`: the walk's product is moved
    /// into the payload exactly once on every path that existed before this
    /// gate, and deriving `Clone` would make an accidental second copy of a
    /// multi-megabyte row set invisible at the call site. This one is spelled
    /// out so every reuse is a line someone chose to write.
    pub(super) fn clone_rows(&self) -> D81WalkRows {
        D81WalkRows {
            provinces: self.provinces.clone(),
            observed_neighbors: self.observed_neighbors,
        }
    }
}
