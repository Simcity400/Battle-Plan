//! The recipe program and the breadth-first engine that runs it.
//!
//! # The program
//!
//! A [`Recipe`] is a flat list of [`RecipeOp`]s plus entry points. Ops come in
//! two kinds and the two alternate:
//!
//!  * **Copy ops** ([`RecipeOp::CopyHeader`], [`RecipeOp::CopyRanges`]) move
//!    bytes. A copy op is the head of a wave: it is submitted, the bytes land,
//!    and only then does anything look at them.
//!  * **Decode ops** ([`RecipeOp::DecodePointer`], [`RecipeOp::DecodeExtent`],
//!    [`RecipeOp::JoinBounded`], [`RecipeOp::WitnessClose`],
//!    [`RecipeOp::Freeze`]) read addresses and lengths out of bytes that are
//!    ALREADY copied, and enqueue the next wave's copy ops. They never parse.
//!
//! [`validate`] enforces the alternation, so a program that tries to decode
//! from bytes it has not copied is rejected before the observation starts.
//!
//! # The waves
//!
//! Roots are at depth 0. Wave `d` is: submit every depth-`d` copy op as one
//! batch, then run every depth-`d` decode op, which fills wave `d + 1`. There
//! are [`FORWARD_WAVES`](super::FORWARD_WAVES) forward waves and one closing
//! witness wave: [`TOTAL_PHASES`](super::TOTAL_PHASES) copy-submission phases,
//! not one per object and not depth × recipe count.
//!
//! A raw dependent chain of that length is single-digit microseconds of DRAM
//! latency. The cost is the SYNCHRONISATION ROUNDS and the BREADTH of each one,
//! which is why everything ready at a depth is batched across every recipe
//! before anything at the next depth is looked at — and why an observation runs
//! only as many waves as its deepest chain, an empty wave costing an `is_empty`
//! test.
//!
//! # Refusals
//!
//! Per-recipe limits produce per-recipe [`Refusal`]s: that recipe's consumer
//! is gapped and every other recipe in the observation is untouched. Custody
//! failure is the one global refusal — it aborts the observation, because an
//! arena that could not be grown cannot be made smaller by dropping a recipe
//! that already copied into it.

use std::collections::{BTreeMap, BTreeSet};
use std::hash::BuildHasherDefault;

use super::arena::Arena;
use super::dedup::OwnershipMap;
use super::policy::{AnchorKind, CopyRequest, GapPolicy, Proof, Provenance};
use super::view::Snapshot;
use super::{
    ConsumerId, CustodyRefusal, FxSet, GapReason, HourStamp, IntervalId, Limits, MalformedReason,
    RecipeId, Refusal, SnapshotMemory, Span, TableId, DEFAULT_CUSTODY_BUDGET_BYTES, FORWARD_WAVES,
    MAX_RECIPE_DEPTH,
};

/// Recipes an observation may hold, so that [`visit_tag`]'s packing is a
/// bound rather than a hope.
///
/// Nothing can approach it: a slice of 2^48 `Recipe`s is over sixteen
/// petabytes. It is checked anyway, once per observation, because the key this
/// replaces cast the recipe index to `u32` — which would have let recipe *n*
/// and recipe *n + 2^32* share a visited entry and prune each other's copy
/// tasks. That was a silent aliasing where this is a stop.
const MAX_RECIPES: usize = 1 << 48;

/// Recipe and op, packed into one word: op in the top sixteen bits, recipe
/// below. `op` is a `u16` by type; `recipe` is bounded by [`MAX_RECIPES`],
/// which [`Observation::new`] asserts.
#[inline]
pub(super) fn visit_tag(recipe: usize, op: u16) -> u64 {
    ((op as u64) << 48) | recipe as u64
}

/// The discovery visited set: which `(recipe, op, address, extent length)`
/// copy tasks have already been expanded.
///
/// # Why it is two tables and not one
///
/// The key has to determine the WHOLE request. For a copy op the spans follow
/// from `(recipe, op, addr)`; for an extent they do not, because two container
/// headers can invoke one `DecodeExtent` against the same array base and
/// decode different counts. The single-table version therefore carried a
/// fourth element on every key — an extent length that was a constant zero on
/// every copy-op task, which is all but all of them.
///
/// That cost 24 bytes per key and four rounds of the hash mixer. Splitting the
/// table by task kind takes the common key to 16 bytes and two rounds, and it
/// is exactly behaviour-preserving rather than approximately so: the two key
/// spaces never overlapped in the single table either. An enqueued extent has
/// `count >= 1` and `stride >= 1` (validation rejects a zero stride, and a
/// zero count returns before enqueuing), so its length is never zero, and a
/// copy op's fourth element always was.
#[derive(Debug, Default)]
pub(super) struct VisitedSet {
    /// `(address, tag)` for copy-op tasks.
    ops: FxSet<(u64, u64)>,
    /// `(address, length, tag)` for decoded extents.
    extents: FxSet<(u64, u64, u64)>,
}

impl VisitedSet {
    /// Sized for `roots` copy-op tasks up front. The set is one insert per
    /// copy task, and a hash table that grows into that from nothing rehashes
    /// every key about twice over; the root count is the one part of the final
    /// size that is known before any memory is touched.
    ///
    /// # The root count is a recipe's claim, not a bound
    ///
    /// It arrives from the recipes and no limit has been applied to it yet:
    /// `max_copy_tasks` is checked per task during discovery, and a recipe may
    /// declare millions of roots and a copy-task limit of one. So this
    /// preallocation is CHARGED to custody and taken through the fallible
    /// reservation, rather than being an infallible `with_capacity` that can
    /// take the machine out — or exceed the ceiling — before the observation
    /// has admitted a single byte.
    pub(super) fn with_root_capacity(
        roots: usize,
        ledger: &super::arena::CustodyLedger,
    ) -> Result<Self, CustodyRefusal> {
        let mut ops: FxSet<(u64, u64)> = FxSet::default();
        if roots > 0 {
            let wanted = super::arena::hash_table_bytes::<(u64, u64)>(roots);
            ledger.charge_tables(wanted)?;
            ops.try_reserve(roots)
                .map_err(|_| CustodyRefusal::AllocationFailed {
                    held: ledger.held(),
                    requested: wanted,
                })?;
            // What it really took, against what was charged for it.
            let took = super::arena::hash_table_bytes::<(u64, u64)>(ops.capacity());
            ledger.charge_tables_overshoot(took.saturating_sub(wanted))?;
        }
        Ok(Self {
            ops,
            extents: FxSet::default(),
        })
    }

    /// True when this task had not been expanded before.
    #[inline]
    pub(super) fn insert_op(&mut self, addr: u64, tag: u64) -> bool {
        self.ops.insert((addr, tag))
    }

    #[inline]
    pub(super) fn insert_extent(&mut self, addr: u64, len: u64, tag: u64) -> bool {
        self.extents.insert((addr, len, tag))
    }

    pub(super) fn len(&self) -> usize {
        self.ops.len() + self.extents.len()
    }

    /// Modelled footprint, for [`Observation::charge_own_tables`].
    pub(super) fn charged_bytes(&self) -> u64 {
        super::arena::hash_table_bytes::<(u64, u64)>(self.ops.capacity())
            + super::arena::hash_table_bytes::<(u64, u64, u64)>(self.extents.capacity())
    }
}

/// The condition under which a decode op dereferences.
///
/// Read at `offset` from the decode site's base, `width` bytes wide, unsigned
/// little-endian — the same shape as the count field beside it. The dereference
/// happens when the value is INSIDE `[min, max]` and the branch ends cleanly
/// when it is outside.
///
/// A guard word the recipe never copied is a recipe defect and refuses with
/// [`Refusal::FieldNotCaptured`], exactly as an uncaptured count or pointer
/// field does. "Could not read the guard" is not "the guard said no".
///
/// # The two shapes it exists for, and both are unions
///
///  * [`ExtentGuard::at_least`] — an MSVC `std::basic_string`'s capacity word.
///    Below 16 the characters are the header's own first sixteen bytes; at 16
///    and above those bytes are a heap pointer.
///  * [`ExtentGuard::exactly`] — an MSVC `_Tree` node's `_Isnil` byte. The
///    single `_Myhead` sentinel every nil link points at is allocated but its
///    value slot is never constructed, so the eight bytes a real node holds its
///    `CCountryLeader*` / `CAdvisor*` in are, on the sentinel, whatever the
///    allocator last left there. Chasing them is a whole-recipe
///    `MemoryUnreadable` on a plausible-looking word and a copy of an unrelated
///    object on an implausible one. Today's walk avoids the sentinel by
///    comparing each child against the head address; a recipe has no equality
///    op, so `_Isnil` is the discriminator, and it is the same byte the walk
///    already refuses a non-sentinel node for setting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExtentGuard {
    pub offset: u32,
    pub width: u8,
    pub min: u64,
    pub max: u64,
}

impl ExtentGuard {
    pub const fn new(offset: u32, width: u8, min: u64, max: u64) -> Self {
        Self {
            offset,
            width,
            min,
            max,
        }
    }

    /// Dereference when the word is at or above `min`.
    pub const fn at_least(offset: u32, width: u8, min: u64) -> Self {
        Self::new(offset, width, min, u64::MAX)
    }

    /// Dereference only when the word is exactly `value`.
    pub const fn exactly(offset: u32, width: u8, value: u64) -> Self {
        Self::new(offset, width, value, value)
    }

    fn admits(&self, value: u64) -> bool {
        value >= self.min && value <= self.max
    }
}

/// A window relative to the address a copy op acts on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RelWindow {
    pub offset: i64,
    pub len: u32,
}

impl RelWindow {
    pub const fn new(offset: i64, len: u32) -> Self {
        Self { offset, len }
    }

    fn resolve(&self, base: u64) -> Option<Span> {
        let start = if self.offset >= 0 {
            base.checked_add(self.offset as u64)?
        } else {
            base.checked_sub(self.offset.unsigned_abs())?
        };
        Some(Span::new(start, u64::from(self.len)))
    }
}

/// One instruction of a recipe.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecipeOp {
    /// COPY. Take `size` bytes at the op's address as one object. The object's
    /// width is thereby PROVEN, which is what unlocks the 64 B merge tier for
    /// everything inside it.
    CopyHeader { size: u32, next: Vec<u16> },
    /// COPY. Take a fixed set of windows around the op's address. This is the
    /// per-family window recipe the gap curve priced (d42's 2 KiB, d43's
    /// leaves); nothing about the surrounding allocation is proven, so its
    /// windows merge at the generic tier only.
    CopyRanges {
        windows: Vec<RelWindow>,
        next: Vec<u16>,
    },
    /// DECODE. Read a pointer at `offset` from already-copied bytes and
    /// continue at its VALUE, one wave deeper. A null pointer ends the branch
    /// without refusing: absent is not broken.
    ///
    /// `guard`, when present, is the same conditional [`RecipeOp::DecodeExtent`]
    /// carries and for the same reason: a field that is only a pointer in one
    /// arm of a union. Outside the guard's band the branch ends cleanly, exactly
    /// as a null value ends it.
    DecodePointer {
        offset: u32,
        guard: Option<ExtentGuard>,
        next: u16,
    },
    /// DECODE. Read an element count and a base pointer out of already-copied
    /// bytes and copy the whole element array as ONE transaction — the
    /// "extent batches replace indivisible domain jobs" requirement.
    ///
    /// `max_count`/`max_bytes` are hard: a container that declares more is a
    /// refusal, never a truncated read of the first `max_count` elements.
    DecodeExtent {
        count_offset: u32,
        count_width: u8,
        ptr_offset: u32,
        stride: u32,
        max_count: u32,
        max_bytes: u64,
        /// Elements copied PAST the decoded count. Zero for a plain array.
        ///
        /// This exists for containers whose in-memory image is longer than
        /// their declared element count by a fixed amount, and the case that
        /// forced it is the only one in the tree: an MSVC `std::string` stores
        /// `_Mysize` characters and a NUL, and the reader that decodes it
        /// REFUSES a buffer whose byte at `[size]` is not zero
        /// (`read_msvc_string_with_snapshot_with_policy`). A recipe that copied
        /// `size` bytes would leave that check reading bytes the observation
        /// never took, so the post-bracket decode would refuse every string —
        /// the terminator has to be inside the copy for the copied path to
        /// reach the same verdict as the live one.
        ///
        /// It is added BEFORE the count and byte limits are applied, so the
        /// bound is on what is actually copied rather than on what the
        /// container declared.
        trailer_elements: u32,
        /// When present, the dereference happens ONLY if the guard word is at
        /// or above [`ExtentGuard::min`]. Below it the branch ends without
        /// refusing, exactly as a null pointer does in `DecodePointer`: absent
        /// is not broken.
        ///
        /// This is the language's one conditional, and it is here because
        /// without it a whole class of container cannot be copied at all. An
        /// MSVC `std::basic_string` is a union: at capacity 15 or below the
        /// characters ARE the first sixteen bytes of the header, and at 16 or
        /// above those same bytes are a heap pointer. An unguarded
        /// `DecodeExtent` over that field dereferences the ASCII text of every
        /// short string, which is `Refusal::MemoryUnreadable` on the first one.
        ///
        /// It is a DECODE and not the in-bracket parsing the architecture
        /// forbids, on exactly the same footing as `DecodePointer`'s null test
        /// and `DecodeExtent`'s own count read: one already-copied word is read
        /// to decide what to copy next. Nothing is interpreted, classified or
        /// admitted; the comparison's only output is whether one more copy is
        /// submitted.
        guard: Option<ExtentGuard>,
        /// Register the resulting array so `JoinBounded` can index it.
        register: Option<TableId>,
        /// Decode ops to run against EACH element. They run in the same wave
        /// the array lands in: an element is not one more dereference away.
        ///
        /// EMPTY IS LEGAL and means the array is the END of this branch: the
        /// bytes are captured and nothing is derived from them in-bracket. It
        /// is accounted as one frozen branch, so the "every branch reaches a
        /// freeze" property still holds over `RecipeReport::frozen_branches`.
        ///
        /// The alternative — `element: [Freeze]` — is not equivalent in cost.
        /// The element loop pushes one `DecodeSite` per element per op, so a
        /// byte-strided extent would push and pop one 24-byte site per BYTE of
        /// every string in the observation. At d47's shape that is over half a
        /// million sites per hour to reach a counter.
        element: Vec<u16>,
    },
    /// DECODE. Read a key and index a registered table by it. Bounded by
    /// construction: the index is checked against the table's decoded element
    /// count, and out-of-range is a refusal.
    ///
    /// Hash and scan joins are deliberately absent. Searching copied bytes for
    /// a matching key is parsing, and parsing is forbidden in-bracket.
    JoinBounded {
        table: TableId,
        key_offset: u32,
        key_width: u8,
        next: u16,
    },
    /// DECODE. Declare that this recipe's result is only valid if a closing
    /// witness certifies the bytes it read. Enrols every interval this
    /// recipe's consumer depends on in the closing wave.
    WitnessClose { next: Vec<u16> },
    /// DECODE. End of branch. Every branch must reach one; a program with a
    /// dangling branch is [`MalformedReason::DanglingBranch`].
    Freeze,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpKind {
    Copy,
    Decode,
}

impl RecipeOp {
    pub fn kind(&self) -> OpKind {
        match self {
            RecipeOp::CopyHeader { .. } | RecipeOp::CopyRanges { .. } => OpKind::Copy,
            _ => OpKind::Decode,
        }
    }
}

/// An entry point: an op index and the root address it starts at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Root {
    pub op: u16,
    pub addr: u64,
}

/// One reader family's capture program.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recipe {
    pub id: RecipeId,
    /// The post-bracket reader this recipe captures for.
    pub consumer: ConsumerId,
    pub limits: Limits,
    /// Coalescing policy. Per-recipe DATA — two recipes in one observation may
    /// merge differently because different families have different proofs.
    pub policy: GapPolicy,
    pub ops: Vec<RecipeOp>,
    pub roots: Vec<Root>,
}

impl Recipe {
    /// A recipe with default limits and the default policy.
    pub fn new(id: RecipeId, consumer: ConsumerId, ops: Vec<RecipeOp>, roots: Vec<Root>) -> Self {
        Self {
            id,
            consumer,
            limits: Limits::default(),
            policy: GapPolicy::default(),
            ops,
            roots,
        }
    }
}

/// Structural check. Runs before any memory is touched.
pub fn validate(recipe: &Recipe) -> Result<(), Refusal> {
    let id = recipe.id;
    let bad = |reason: MalformedReason| Refusal::Malformed { recipe: id, reason };

    if recipe.ops.is_empty() {
        return Err(bad(MalformedReason::NoOps));
    }
    if recipe.roots.is_empty() {
        return Err(bad(MalformedReason::NoRoots));
    }
    if recipe.limits.max_depth > MAX_RECIPE_DEPTH {
        return Err(bad(MalformedReason::DepthLimitAboveCeiling {
            limit: recipe.limits.max_depth,
        }));
    }
    for (name, zero) in [
        ("max_copy_tasks", recipe.limits.max_copy_tasks == 0),
        ("max_ranges", recipe.limits.max_ranges == 0),
        ("max_bytes", recipe.limits.max_bytes == 0),
        (
            "max_single_range_bytes",
            recipe.limits.max_single_range_bytes == 0,
        ),
    ] {
        if zero {
            return Err(bad(MalformedReason::ZeroLimit { limit: name }));
        }
    }

    let count = recipe.ops.len();
    let in_range = |op: u16| (op as usize) < count;

    for root in &recipe.roots {
        if !in_range(root.op) {
            return Err(bad(MalformedReason::OpIndexOutOfRange { op: root.op }));
        }
        if recipe.ops[root.op as usize].kind() != OpKind::Copy {
            return Err(bad(MalformedReason::RootIsNotACopyOp { op: root.op }));
        }
    }

    for (index, op) in recipe.ops.iter().enumerate() {
        let here = index as u16;
        let expect = |target: u16, kind: OpKind| -> Result<(), Refusal> {
            if !in_range(target) {
                return Err(bad(MalformedReason::OpIndexOutOfRange { op: target }));
            }
            if recipe.ops[target as usize].kind() != kind {
                return Err(bad(match kind {
                    OpKind::Copy => MalformedReason::ExpectedCopyOp { op: target },
                    OpKind::Decode => MalformedReason::ExpectedDecodeOp { op: target },
                }));
            }
            Ok(())
        };
        match op {
            RecipeOp::CopyHeader { size, next } => {
                if *size == 0 {
                    return Err(bad(MalformedReason::ZeroSizedCopy { op: here }));
                }
                if next.is_empty() {
                    return Err(bad(MalformedReason::DanglingBranch { op: here }));
                }
                for &target in next {
                    expect(target, OpKind::Decode)?;
                }
            }
            RecipeOp::CopyRanges { windows, next } => {
                if windows.is_empty() || windows.iter().any(|w| w.len == 0) {
                    return Err(bad(MalformedReason::ZeroSizedCopy { op: here }));
                }
                if next.is_empty() {
                    return Err(bad(MalformedReason::DanglingBranch { op: here }));
                }
                for &target in next {
                    expect(target, OpKind::Decode)?;
                }
            }
            RecipeOp::DecodePointer { guard, next, .. } => {
                if let Some(guard) = guard {
                    if !matches!(guard.width, 1 | 2 | 4 | 8) {
                        return Err(bad(MalformedReason::BadWidth {
                            op: here,
                            width: guard.width,
                        }));
                    }
                }
                expect(*next, OpKind::Copy)?
            }
            RecipeOp::DecodeExtent {
                count_width,
                stride,
                element,
                max_count,
                max_bytes,
                guard,
                ..
            } => {
                if !matches!(count_width, 1 | 2 | 4 | 8) {
                    return Err(bad(MalformedReason::BadWidth {
                        op: here,
                        width: *count_width,
                    }));
                }
                if let Some(guard) = guard {
                    if !matches!(guard.width, 1 | 2 | 4 | 8) {
                        return Err(bad(MalformedReason::BadWidth {
                            op: here,
                            width: guard.width,
                        }));
                    }
                }
                if *stride == 0 {
                    return Err(bad(MalformedReason::ZeroStride { op: here }));
                }
                if *max_count == 0 {
                    return Err(bad(MalformedReason::ZeroLimit {
                        limit: "DecodeExtent::max_count",
                    }));
                }
                if *max_bytes == 0 {
                    return Err(bad(MalformedReason::ZeroLimit {
                        limit: "DecodeExtent::max_bytes",
                    }));
                }
                // An EMPTY element list is legal here and nowhere else: the
                // array itself is the end of the branch. See the field's own
                // documentation for why, and for what it is accounted as.
                for &target in element {
                    expect(target, OpKind::Decode)?;
                }
            }
            RecipeOp::JoinBounded {
                key_width, next, ..
            } => {
                if !matches!(key_width, 1 | 2 | 4 | 8) {
                    return Err(bad(MalformedReason::BadWidth {
                        op: here,
                        width: *key_width,
                    }));
                }
                expect(*next, OpKind::Copy)?;
            }
            RecipeOp::WitnessClose { next } => {
                if next.is_empty() {
                    return Err(bad(MalformedReason::DanglingBranch { op: here }));
                }
                for &target in next {
                    expect(target, OpKind::Decode)?;
                }
            }
            RecipeOp::Freeze => {}
        }
    }
    Ok(())
}

/// How a recipe ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecipeOutcome {
    /// Every branch reached a `Freeze` within every limit.
    Captured,
    /// A named constraint stopped it. The consumer is gapped with this reason.
    Refused(Refusal),
}

/// Per-recipe accounting for one observation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecipeReport {
    pub recipe: RecipeId,
    pub consumer: ConsumerId,
    pub outcome: RecipeOutcome,
    /// Copy tasks expanded.
    pub copy_tasks: u32,
    /// Physical copy transactions after coalescing, before deduplication.
    pub planned_ranges: u32,
    /// Bytes after coalescing (bridged holes included), before deduplication.
    pub planned_bytes: u64,
    /// Branches that reached `Freeze`.
    pub frozen_branches: u32,
    pub witness_required: bool,
}

/// What one observation produced, before the post-bracket witness comparison.
#[derive(Debug)]
pub struct SnapshotBuild {
    pub snapshot: Snapshot,
    pub reports: Vec<RecipeReport>,
    /// Forward waves that had at least one copy task.
    pub forward_waves_run: u32,
    /// Copy-submission phases run, including the closing witness wave.
    pub phases_run: u32,
    /// Intervals enrolled in the closing wave.
    pub witnessed_intervals: u32,
    /// Everything the observation held against its ceiling at the end: copied
    /// bytes plus the index and link tables that address them. This is the
    /// number the 512 MiB budget is a budget FOR, and it is on the build
    /// rather than only inside the arena because a caller that wants to know
    /// whether an observation is near the ceiling cannot ask the arena — it
    /// was consumed into the snapshot.
    pub custody_held: u64,
    /// The table half of [`Self::custody_held`]. At the substrate's own bench
    /// shape it is the larger half.
    pub custody_table_bytes: u64,
    /// The arena half: its CAPACITY, which is what custody is charged for and
    /// is at or above [`view::Snapshot::custody_bytes`], the length actually
    /// copied into.
    pub custody_arena_bytes: u64,
    /// Intervals whose closing copy could not be taken. Treated as changed by
    /// the comparison: an uncertifiable witness is a failed one.
    pub(super) unreadable_closes: Vec<IntervalId>,
}

impl SnapshotBuild {
    pub fn report_for(&self, recipe: RecipeId) -> Option<&RecipeReport> {
        self.reports.iter().find(|r| r.recipe == recipe)
    }
}

/// Configuration for one observation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ObservationConfig {
    pub hour: HourStamp,
    /// Private: [`DEFAULT_CUSTODY_BUDGET_BYTES`] is a CEILING, not a default a
    /// caller may raise. A settable field let any caller opt out of the one
    /// number that bounds what an observation can hold.
    custody_budget: u64,
}

impl ObservationConfig {
    pub fn new(hour: HourStamp) -> Self {
        Self {
            hour,
            custody_budget: DEFAULT_CUSTODY_BUDGET_BYTES,
        }
    }

    pub fn custody_budget(&self) -> u64 {
        self.custody_budget
    }

    /// Lower the budget. Raising it past the ceiling is refused.
    pub fn with_custody_budget(mut self, budget: u64) -> Result<Self, CustodyRefusal> {
        if budget > DEFAULT_CUSTODY_BUDGET_BYTES {
            return Err(CustodyRefusal::BudgetExhausted {
                budget: DEFAULT_CUSTODY_BUDGET_BYTES,
                held: 0,
                requested: budget,
            });
        }
        self.custody_budget = budget;
        Ok(self)
    }
}

/// Run one observation: 13 forward waves and the closing witness wave.
pub fn observe<M: SnapshotMemory>(
    recipes: &[Recipe],
    memory: &M,
    config: ObservationConfig,
) -> Result<SnapshotBuild, CustodyRefusal> {
    observe_in(recipes, memory, config, Arena::with_budget(config.custody_budget))
}

/// [`observe`] against a caller-supplied arena — the seam tests use to make
/// allocation failure reachable.
///
/// The ceiling applies here too: an arena budgeted above
/// [`DEFAULT_CUSTODY_BUDGET_BYTES`] is refused rather than honoured, so this
/// entry point cannot be used to get more custody than [`observe`] allows.
pub fn observe_in<M: SnapshotMemory>(
    recipes: &[Recipe],
    memory: &M,
    config: ObservationConfig,
    arena: Arena,
) -> Result<SnapshotBuild, CustodyRefusal> {
    if arena.budget() > DEFAULT_CUSTODY_BUDGET_BYTES {
        return Err(CustodyRefusal::BudgetExhausted {
            budget: DEFAULT_CUSTODY_BUDGET_BYTES,
            held: 0,
            requested: arena.budget(),
        });
    }
    Observation::new(recipes, memory, config, arena)?.run()
}

#[derive(Clone, Copy, Debug)]
struct TableBinding {
    span: Span,
    count: u32,
    stride: u32,
}

/// Which anchor a task's copy requests rest on.
///
/// # Why this is a tag and not an [`AnchorKind`]
///
/// A task used to carry the whole `AnchorKind`, whose two inhabited variants
/// each hold an address — and at every one of the three sites that builds a
/// `TaskBody::Op`, THAT ADDRESS IS THE TASK'S OWN `addr`: a root anchors on
/// `root.addr` and copies at `root.addr`, a dereferenced pointer anchors on the
/// value it read and copies at that value, and a bounded join anchors on the
/// element address it computed and copies there. So the payload was sixteen
/// bytes per task restating a field the task already had, on a value that is
/// memcpy'd into a wave queue and back out of it.
///
/// [`AnchorKind::Mixed`] is deliberately not representable here. It is what
/// COALESCING produces when it merges requests with different anchors — a
/// property of a planned copy, never of a task — and a task that could claim it
/// would be claiming a merge that had not happened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TaskAnchor {
    Root,
    Pointer,
}

impl TaskAnchor {
    /// Rebuild the anchor at the task's address. The inverse of the erasure
    /// above, and the only place it is undone.
    #[inline]
    fn at(self, addr: u64) -> AnchorKind {
        match self {
            TaskAnchor::Root => AnchorKind::Root(addr),
            TaskAnchor::Pointer => AnchorKind::Pointer(addr),
        }
    }
}

#[derive(Clone, Debug)]
enum TaskBody {
    /// A copy op acting at `addr`.
    Op {
        op: u16,
        addr: u64,
        anchor: TaskAnchor,
        /// What is proven about the region the address sits in, inherited from
        /// whatever produced it. `None` for a pointer target: a pointee's
        /// allocation is not proven by the pointer.
        inherited: Proof,
    },
    /// A resolved element array from a `DecodeExtent`.
    Extent { op: u16, span: Span, count: u32 },
}

#[derive(Clone, Debug)]
pub(super) struct CopyTask {
    recipe: usize,
    depth: u32,
    /// The interval whose bytes produced this task's address.
    parent: Option<IntervalId>,
    body: TaskBody,
}

/// A decode op about to run against already-copied bytes.
///
/// A site deliberately does NOT carry the interval its base sits in. The
/// discovery edge a decode op produces is anchored on the interval holding the
/// FIELD it read — `run_site` looks that up as part of reading the field — and
/// an interval for the site's base would be a second, unused lookup per site
/// and, in `DecodeExtent`'s element loop, per element.
#[derive(Clone, Copy, Debug)]
struct DecodeSite {
    op: u16,
    base: u64,
    /// The proven region `base` sits in.
    region: Proof,
}

#[derive(Clone, Debug)]
struct RecipeState {
    copy_tasks: u32,
    planned_ranges: u32,
    planned_bytes: u64,
    frozen_branches: u32,
    witness_required: bool,
    refusal: Option<Refusal>,
}

impl RecipeState {
    fn new() -> Self {
        Self {
            copy_tasks: 0,
            planned_ranges: 0,
            planned_bytes: 0,
            frozen_branches: 0,
            witness_required: false,
            refusal: None,
        }
    }
}

struct Observation<'a, M: SnapshotMemory> {
    recipes: &'a [Recipe],
    memory: &'a M,
    config: ObservationConfig,
    arena: Arena,
    owners: OwnershipMap,
    states: Vec<RecipeState>,
    tables: BTreeMap<(usize, TableId), TableBinding>,
    /// (recipe, op, address, extent length) already expanded.
    visited: VisitedSet,
    /// Tasks pruned as duplicates. Their dependency edges are still real, so
    /// they are replayed for linkage only once every interval exists.
    pruned: Vec<CopyTask>,
    waves: Vec<Vec<CopyTask>>,
    forward_waves_run: u32,
    // Scratch, owned by the observation and reused across waves. A fresh
    // allocation per task or per copy transaction would be a heap allocation
    // inside the bracket, which is the thing being priced.
    /// This wave's task indices, grouped by recipe POSITION. See
    /// [`Observation::copy_wave`].
    by_recipe: Vec<Vec<u32>>,
    requests: Vec<CopyRequest>,
    plan: super::policy::CopyPlan,
    acquisition: super::dedup::AcquisitionPlan,
    owned: Vec<IntervalId>,
    sites: Vec<DecodeSite>,
    /// Bytes of the executor's own tables already charged to the ledger. See
    /// [`Observation::charge_own_tables`].
    own_tables_charged: u64,
}

impl<'a, M: SnapshotMemory> Observation<'a, M> {
    fn new(
        recipes: &'a [Recipe],
        memory: &'a M,
        config: ObservationConfig,
        arena: Arena,
    ) -> Result<Self, CustodyRefusal> {
        // One ledger for the observation: the ownership map's tables are
        // charged against the same 512 MiB the copied bytes are. See
        // `arena::CustodyLedger`.
        let owners = OwnershipMap::on_ledger(arena.ledger());
        assert!(
            recipes.len() <= MAX_RECIPES,
            "an observation may hold at most {MAX_RECIPES} recipes: past that the discovery \
             visited set's key would alias two of them",
        );
        // Fallible, and charged before it is taken: the root count is a
        // recipe's claim and no limit has been applied to it yet. See
        // `VisitedSet::with_root_capacity`.
        let visited = VisitedSet::with_root_capacity(
            recipes.iter().map(|recipe| recipe.roots.len()).sum(),
            arena.ledger_ref(),
        )?;
        // `charge_own_tables` reconciles against a running total, so it has to
        // START from what the visited set has already been charged for, or
        // that preallocation is charged a second time.
        let own_tables_charged = visited.charged_bytes();
        Ok(Self {
            recipes,
            memory,
            config,
            arena,
            owners,
            states: recipes.iter().map(|_| RecipeState::new()).collect(),
            tables: BTreeMap::new(),
            visited,
            pruned: Vec::new(),
            waves: (0..FORWARD_WAVES + 1).map(|_| Vec::new()).collect(),
            forward_waves_run: 0,
            by_recipe: Vec::new(),
            requests: Vec::new(),
            plan: super::policy::CopyPlan::new(),
            acquisition: super::dedup::AcquisitionPlan::default(),
            owned: Vec::new(),
            sites: Vec::new(),
            own_tables_charged,
        })
    }

    /// Charge the tables the EXECUTOR owns to the observation's custody
    /// ledger.
    ///
    /// [`OwnershipMap`] admits its growth before it takes it, because its
    /// growth points are already fallible. The executor's are not: the wave
    /// queues, the pruned-task list and the discovery visited set grow inside
    /// `enqueue`, and the per-wave scratch grows inside the decode dispatch —
    /// once per discovered task each, on the hottest path the substrate has.
    /// So these are RECONCILED rather than admitted: once per wave, the ledger
    /// is brought up to their real capacity, and an observation that has gone
    /// past the ceiling aborts before the next wave copies a byte.
    ///
    /// What that costs, stated rather than hidden: the ledger can lag by one
    /// wave's growth of these tables. It cannot lag by more, and the growth is
    /// bounded by that wave's task count, which `max_copy_tasks` already
    /// bounds per recipe.
    ///
    /// Two structures are deliberately not in the sum. `tables` and
    /// `tables` is a `BTreeMap` whose node allocation is not observable from
    /// outside, and it is bounded by the registered join tables of the recipes
    /// in the observation, which does not scale with the work. `by_recipe`
    /// holds one group per recipe and its groups' capacities are counted
    /// below.
    ///
    /// # `in_flight`, and the hole it closes
    ///
    /// `run` takes a wave's tasks OUT of `self.waves` before running it, so for
    /// the whole of that wave the queue this function measures is empty while
    /// the memory is still held in a local. The baseline it reconciles against
    /// still contains that capacity, so the `bytes <= charged` exit fires, and
    /// growth elsewhere in the wave up to the size of the invisible queue is
    /// charged to nobody. The ledger could therefore sit BELOW what the
    /// observation held — the one direction custody must never take.
    ///
    /// The caller passes the in-flight queue's capacity, so the sum is the
    /// whole footprint whether the tasks are in `self.waves` or in a local.
    /// Callers that hold no wave pass zero.
    fn charge_own_tables(&mut self, in_flight: u64) -> Result<(), CustodyRefusal> {
        let bytes = self.own_tables_bytes(in_flight);
        if bytes <= self.own_tables_charged {
            return Ok(());
        }
        let delta = bytes - self.own_tables_charged;
        self.own_tables_charged = bytes;
        self.arena.ledger_ref().charge_tables_overshoot(delta)
    }

    /// Reconcile while `tasks` is held outside `self.waves`.
    ///
    /// The wave loop calls THIS rather than passing a number, so the one thing
    /// that has to be right at that call site — that the held queue is counted
    /// — is carried by the type rather than by whoever edits the argument next.
    fn charge_own_tables_holding(&mut self, tasks: &Vec<CopyTask>) -> Result<(), CustodyRefusal> {
        self.charge_own_tables(super::arena::vec_bytes(tasks))
    }

    /// The footprint [`Self::charge_own_tables`] reconciles against.
    ///
    /// Split out so a test can ask for the same number the charging asks for.
    /// A test that recomputed this sum itself would drift from it the first
    /// time a table was added, and would then agree with the production code
    /// about a footprint neither of them measured.
    fn own_tables_bytes(&self, in_flight: u64) -> u64 {
        let mut bytes = in_flight
            + super::arena::vec_bytes(&self.pruned)
            + super::arena::vec_bytes(&self.waves)
            + super::arena::vec_bytes(&self.by_recipe)
            + super::arena::vec_bytes(&self.requests)
            + super::arena::vec_bytes(&self.owned)
            + super::arena::vec_bytes(&self.sites)
            + self.visited.charged_bytes()
            + self.plan.capacity_bytes()
            + self.acquisition.capacity_bytes();
        for wave in &self.waves {
            bytes += super::arena::vec_bytes(wave);
        }
        for group in &self.by_recipe {
            bytes += super::arena::vec_bytes(group);
        }
        bytes
    }

    fn refuse(&mut self, recipe: usize, refusal: Refusal) {
        if self.states[recipe].refusal.is_none() {
            self.states[recipe].refusal = Some(refusal);
        }
    }

    fn refused(&self, recipe: usize) -> bool {
        self.states[recipe].refusal.is_some()
    }

    /// A field or window address that does not exist. Refusing beats skipping:
    /// a branch that quietly stops because its address did not fit produces a
    /// short result and a `Captured` outcome.
    fn refuse_unrepresentable(&mut self, recipe: usize, base: u64, offset: i64) {
        let id = self.recipes[recipe].id;
        self.refuse(
            recipe,
            Refusal::AddressNotRepresentable {
                recipe: id,
                base,
                offset,
            },
        );
    }

    /// Make room in one of the executor's own tables, charged BEFORE it is
    /// taken and reflected in the reconciliation baseline.
    ///
    /// The executor's tables are normally charged by reconciliation — they grow
    /// inside `enqueue` and the decode dispatch, and `charge_own_tables` brings
    /// the ledger up to their real capacity once per wave. A reservation cannot
    /// use that route on its own: `charged_reserve` charges the growth it takes,
    /// and the next reconciliation would then see the larger capacity and charge
    /// the same bytes a second time. So the baseline is advanced by exactly what
    /// the reservation added, and the two accountings stay disjoint.
    ///
    /// The charge is still taken first and is still fallible, which is the
    /// property that matters: a reservation sized from a recipe's declared root
    /// count is a CLAIM, not a bound, and an infallible `reserve` on it would
    /// let a recipe take the machine out before the observation admitted a byte.
    fn reserve_own<T>(
        table: &mut Vec<T>,
        ledger: &super::arena::CustodyLedger,
        charged: &mut u64,
        additional: usize,
    ) -> Result<(), CustodyRefusal> {
        let before = super::arena::vec_bytes(table);
        super::arena::charged_reserve(table, ledger, additional)?;
        *charged += super::arena::vec_bytes(table) - before;
        Ok(())
    }

    /// Give the depth-zero queue room for every declared root before the first
    /// one is built.
    ///
    /// Depth zero takes one task per root, so its final size is known up front.
    /// Without this the queue doubles its way up from nothing while the roots
    /// are seeded, and at the substrate's own bench shape that growth — the
    /// reallocation memcpys and the first touch of the pages behind them — was
    /// most of what a root's construct-and-push cost. `bench_dispatch_components`
    /// runs a row with this and a row without it.
    ///
    /// # The size is what `enqueue` can ACCEPT, not what the recipe declared
    ///
    /// A root count is a recipe's claim and no limit has been applied to it
    /// yet, so the declared count is not a safe size to reserve: `enqueue`
    /// refuses a recipe once it has expanded `max_copy_tasks` of them, and a
    /// recipe may declare millions of roots against a task limit of a thousand.
    /// Reserving the declared count there would take — and charge — memory the
    /// observation could never fill, and at the ceiling that turns an
    /// observation which used to run into a custody refusal. So each recipe
    /// contributes `min(roots, max_copy_tasks)`, which is exactly the most of
    /// its roots that can ever be queued.
    ///
    /// It is still charged and still fallible. The bound above makes the
    /// reservation honest; it does not make it free, and the sum across recipes
    /// is still large enough to be worth refusing before a byte is copied.
    /// Over-reserving for a recipe that later fails `validate` or refuses
    /// mid-observation is capacity custody has been told about.
    /// # A recipe that failed validation contributes nothing
    ///
    /// `run` validates before it reserves, and this skips every recipe already
    /// carrying a refusal. A malformed recipe enqueues no tasks at all — the
    /// seeding loop passes over it and `enqueue` refuses it a second time — so
    /// counting its declared roots would charge custody for queue slots that
    /// cannot exist. At the ceiling that is not merely wasteful: custody
    /// failure is the one GLOBAL refusal, so a malformed recipe would be able
    /// to abort the well-formed recipes sharing its observation, which is
    /// precisely the blast radius per-recipe refusals exist to prevent.
    fn reserve_root_queue(&mut self) -> Result<(), CustodyRefusal> {
        let roots: usize = self
            .recipes
            .iter()
            .enumerate()
            .filter(|(index, _)| self.states[*index].refusal.is_none())
            .map(|(_, recipe)| recipe.roots.len().min(recipe.limits.max_copy_tasks as usize))
            .sum();
        let ledger = self.arena.ledger_ref();
        Self::reserve_own(
            &mut self.waves[0],
            ledger,
            &mut self.own_tables_charged,
            roots,
        )
    }

    fn run(mut self) -> Result<SnapshotBuild, CustodyRefusal> {
        // Validation FIRST, as its own pass, and only then the reservation.
        //
        // These used to be one loop, with the reservation in front of it, so a
        // recipe that `validate` was about to reject still had its declared
        // roots counted into the queue custody was charged for. `validate` is a
        // pure function of the recipe, so hoisting it changes nothing about
        // which recipes refuse or with what — it only means the reservation
        // knows which of them can enqueue anything.
        for (index, recipe) in self.recipes.iter().enumerate() {
            if let Err(refusal) = validate(recipe) {
                self.states[index].refusal = Some(refusal);
            }
        }
        self.reserve_root_queue()?;

        for (index, recipe) in self.recipes.iter().enumerate() {
            if self.refused(index) {
                continue;
            }
            for root in &recipe.roots {
                let task = CopyTask {
                    recipe: index,
                    depth: 0,
                    parent: None,
                    body: TaskBody::Op {
                        op: root.op,
                        addr: root.addr,
                        anchor: TaskAnchor::Root,
                        inherited: Proof::None,
                    },
                };
                self.enqueue(task);
            }
        }
        self.charge_own_tables(0)?;

        for depth in 0..FORWARD_WAVES {
            let tasks = std::mem::take(&mut self.waves[depth as usize]);
            if tasks.is_empty() {
                continue;
            }
            self.forward_waves_run += 1;
            self.copy_wave(&tasks)?;
            // Fold the wave's new intervals into the index body. Once per
            // wave, not once per transaction: that ratio is the whole point of
            // the sweep.
            self.owners.end_wave()?;
            self.decode_wave(&tasks);
            // The executor's own tables grew by this wave's discoveries.
            // Reconcile them against the ledger before the next wave copies a
            // byte. See `charge_own_tables`.
            //
            // `tasks` is still alive and still held: the take above moved this
            // wave's queue out of `self.waves`, so it has to be handed back in
            // or the reconciliation measures a footprint smaller than the one
            // the observation has.
            self.charge_own_tables_holding(&tasks)?;
        }

        // Anything still queued at depth FORWARD_WAVES was enqueued by the
        // last forward wave. `enqueue` refuses past `max_depth`, and
        // `max_depth` cannot exceed the measured maximum, so this is empty in
        // every accepted program; it is asserted rather than assumed.
        debug_assert!(self.waves[FORWARD_WAVES as usize].is_empty());

        self.link_pruned()?;
        self.charge_own_tables(0)?;
        self.enrol_witnesses()?;
        let (witnessed, unreadable_closes) = self.closing_wave()?;

        let phases_run = self.forward_waves_run + u32::from(witnessed > 0);
        let reports = self.build_reports();
        let gapped = reports
            .iter()
            .filter_map(|report| match report.outcome {
                RecipeOutcome::Refused(refusal) => {
                    Some((report.consumer, GapReason::Refused(refusal)))
                }
                RecipeOutcome::Captured => None,
            })
            .collect::<BTreeMap<ConsumerId, GapReason>>();

        // The ledger outlives the arena: `into_bytes` consumes the arena, and
        // the page index the freeze builds still has to be charged.
        let ledger = self.arena.ledger();
        let (records, links) = self.owners.into_frozen()?;
        let snapshot = Snapshot::freeze(
            self.config.hour,
            self.arena.into_bytes(),
            records,
            links,
            gapped,
            &ledger,
        )?;
        let custody_held = ledger.held();
        let custody_table_bytes = ledger.table_bytes();
        let custody_arena_bytes = ledger.arena_bytes();

        Ok(SnapshotBuild {
            snapshot,
            reports,
            forward_waves_run: self.forward_waves_run,
            phases_run,
            witnessed_intervals: witnessed,
            custody_held,
            custody_table_bytes,
            custody_arena_bytes,
            unreadable_closes,
        })
    }

    fn enqueue(&mut self, task: CopyTask) {
        let recipe = task.recipe;
        if self.refused(recipe) {
            return;
        }
        let limits = self.recipes[recipe].limits;
        if task.depth > limits.max_depth {
            self.refuse(
                recipe,
                Refusal::DepthExceeded {
                    recipe: self.recipes[recipe].id,
                    limit: limits.max_depth,
                    wanted_depth: task.depth,
                },
            );
            return;
        }
        // The key must determine the WHOLE request, not just where it starts.
        // For a copy op the spans follow from `(op, addr)`. For an extent they
        // do not: two container headers can invoke one `DecodeExtent` against
        // the same array base and decode different counts, and keying on the
        // base alone prunes the longer one as a duplicate of the shorter —
        // copying a prefix and reporting `Captured`. See [`VisitedSet`] for
        // why that difference is two tables rather than a wider key.
        let fresh = match &task.body {
            TaskBody::Op { op, addr, .. } => {
                self.visited.insert_op(*addr, visit_tag(recipe, *op))
            }
            TaskBody::Extent { op, span, .. } => {
                self.visited
                    .insert_extent(span.start, span.len, visit_tag(recipe, *op))
            }
        };
        if !fresh {
            // Already expanded, at this depth or a shallower one. Expanding it
            // again would copy nothing (deduplication) but would let a pointer
            // cycle run to the depth limit and refuse a recipe that is
            // perfectly well formed. The dependency edge is still real, so the
            // task is kept for linkage.
            self.pruned.push(task);
            return;
        }
        let state = &mut self.states[recipe];
        if state.copy_tasks == limits.max_copy_tasks {
            let id = self.recipes[recipe].id;
            self.refuse(
                recipe,
                Refusal::CopyTaskLimitExceeded {
                    recipe: id,
                    limit: limits.max_copy_tasks,
                },
            );
            return;
        }
        state.copy_tasks += 1;
        self.waves[task.depth as usize].push(task);
    }

    /// Append the spans one task asks for, with their provenance, to `out`.
    ///
    /// Appends rather than returns: this runs once per task per wave, and a
    /// returned `Vec` is a heap allocation per task.
    ///
    /// `Err((base, offset))` means a window offset carried its base off the
    /// address space. The caller turns that into
    /// [`Refusal::AddressNotRepresentable`] — it is NOT a window to skip.
    fn requests_of(
        &self,
        task: &CopyTask,
        index: u32,
        out: &mut Vec<CopyRequest>,
    ) -> Result<(), (u64, i64)> {
        let mut push = |span: Span, provenance: Provenance| {
            out.push(CopyRequest {
                span,
                provenance,
                task: index,
            })
        };
        match &task.body {
            TaskBody::Op {
                op,
                addr,
                anchor,
                inherited,
            } => match &self.recipes[task.recipe].ops[*op as usize] {
                // `anchor.at(*addr)` is the anchor the task used to carry
                // whole. See [`TaskAnchor`]: the address it rebuilds with is
                // the one the payload always held.
                RecipeOp::CopyHeader { size, .. } => push(
                    Span::new(*addr, u64::from(*size)),
                    Provenance::new(
                        anchor.at(*addr),
                        Proof::Object {
                            base: *addr,
                            len: u64::from(*size),
                        },
                    ),
                ),
                RecipeOp::CopyRanges { windows, .. } => {
                    for window in windows {
                        match window.resolve(*addr) {
                            Some(span) => push(span, Provenance::new(anchor.at(*addr), *inherited)),
                            None => return Err((*addr, window.offset)),
                        }
                    }
                }
                // Validation guarantees a task body only ever names a copy op.
                _ => {}
            },
            TaskBody::Extent { span, .. } => push(
                *span,
                Provenance::new(
                    AnchorKind::Pointer(span.start),
                    Proof::Extent {
                        base: span.start,
                        len: span.len,
                    },
                ),
            ),
        }
        Ok(())
    }

    /// Phase one of a wave: plan every recipe's copies, then submit them.
    ///
    /// Every buffer this uses is owned by the observation and reused across
    /// waves and recipes. That is not a micro-optimisation: a fresh `Vec` per
    /// task or per copy transaction is a heap allocation inside the bracket,
    /// and the bracket is the thing being priced.
    ///
    /// # One batch per recipe, and each batch is a sweep
    ///
    /// A recipe's coalesced copies come out of `plan_copies_into` sorted by
    /// address and pairwise disjoint, and its requests come with a sorted
    /// index alongside. So neither the acquisition nor the dependency attach
    /// looks anything up: each takes a [`super::dedup::Cursor`] and walks the
    /// ownership index once for the whole batch. `end_batch` folds the batch's
    /// new intervals into the wave's index run afterwards, which is what keeps
    /// the next recipe's sweep seeing them.
    ///
    /// Acquiring the whole batch before attaching anything is not an ordering
    /// choice: the attach pass must see every interval the batch created,
    /// including the ones a later copy in the batch produced.
    ///
    /// # The grouping is an index, not a probe
    ///
    /// A recipe is a POSITION in the observation's recipe slice, so grouping a
    /// wave's tasks by recipe is an array index and not a lookup. It used to
    /// be a `BTreeMap::entry` per task — one ordered-map probe on the hottest
    /// path the substrate has, for a key that is already a dense small
    /// integer. Iterating the vector in position order visits the recipes in
    /// exactly the ascending order the map did, so the batch order, the
    /// `end_batch` folds and every refusal are unchanged.
    fn copy_wave(&mut self, tasks: &[CopyTask]) -> Result<(), CustodyRefusal> {
        let mut by_recipe = std::mem::take(&mut self.by_recipe);
        for group in by_recipe.iter_mut() {
            group.clear();
        }
        by_recipe.resize_with(self.recipes.len(), Vec::new);
        for (index, task) in tasks.iter().enumerate() {
            by_recipe[task.recipe].push(index as u32);
        }
        let mut requests = std::mem::take(&mut self.requests);
        let mut planned = std::mem::take(&mut self.plan);
        let mut acquisition = std::mem::take(&mut self.acquisition);
        let mut owned = std::mem::take(&mut self.owned);

        let mut result = Ok(());
        for (recipe, indices) in by_recipe.iter().enumerate() {
            if indices.is_empty() {
                // The map this replaced had no entry for a recipe with no
                // tasks in this wave, so it ran no batch for it either — not
                // even the `end_batch` below.
                continue;
            }
            // Unconditionally, and before anything can `continue`: the batch
            // run must be empty when a batch's sweep starts, or a later
            // batch's lower-addressed hole would be appended behind a higher
            // one and the run would stop being sorted.
            if let Err(custody) = self.owners.end_batch() {
                result = Err(custody);
                break;
            }
            if self.refused(recipe) {
                continue;
            }
            let id = self.recipes[recipe].id;
            let limits = self.recipes[recipe].limits;
            let policy = self.recipes[recipe].policy;

            requests.clear();
            // Every task in the batch produces at least one request, so the
            // batch's task count is a floor on what this buffer is about to
            // hold. Reserved rather than grown into: the buffer starts empty on
            // the first batch of the observation, and doubling up to a hundred
            // thousand requests was 21 of the 26 ns `requests_of` cost per task
            // — the resolution itself is under 5.
            //
            // A floor, not the exact size: a `CopyRanges` task pushes one
            // request per window, so a later push may still grow the buffer.
            // That growth is charged where all of the executor's table growth
            // is, by the per-wave reconciliation.
            //
            // WHAT THIS CHANGES ABOUT REFUSALS, stated rather than left to be
            // discovered. This is a charge point where there was none, so an
            // observation right at the ceiling can now take a global
            // `CustodyRefusal` here instead of reaching a per-recipe refusal
            // — an unrepresentable window, or a range or byte limit — that the
            // resolution below would have produced a moment later.
            //
            // For the range and byte limits that is the module's
            // charge-before-allocate law reaching a buffer that previously grew
            // unadmitted, and nothing more: those limits are checked AFTER the
            // requests are materialised and coalesced, so the buffer was always
            // going to be filled and the only question was whether custody was
            // told before or after.
            //
            // For `AddressNotRepresentable` it is a real widening, and it is
            // not covered by that argument. A task whose FIRST `CopyRanges`
            // window carries its base off the address space breaks out of the
            // resolution loop before it pushes anything, so the old buffer
            // could stay empty and the recipe refused on its own account. This
            // reservation is taken in front of that.
            //
            // AND THE BAND IS NOT NARROW. The reservation is the batch's task
            // count times `size_of::<CopyRequest>()`, which is 64 bytes: at the
            // hundred-thousand-task shape this module's own bench runs, that is
            // 6.1 MiB. An observation with less headroom than that, whose first
            // window is unrepresentable, now takes a GLOBAL custody refusal
            // where it used to gap one consumer.
            //
            // Restoring the old precedence needs a representability preflight
            // over the batch's windows, in the same order, before the
            // reservation — which is a second `resolve` pass over every window
            // on the hottest path in the substrate, i.e. paying throughput for
            // a refusal-precedence guarantee. That trade is the owner's to make
            // and is recorded here rather than taken.
            //
            // The buffer is reused for the whole observation, so the charge is
            // taken on the first batch that needs the room and is a no-op on
            // every batch after it.
            if let Err(custody) = Self::reserve_own(
                &mut requests,
                self.arena.ledger_ref(),
                &mut self.own_tables_charged,
                indices.len(),
            ) {
                result = Err(custody);
                break;
            }
            let mut unrepresentable = None;
            for &index in indices {
                if let Err((base, offset)) =
                    self.requests_of(&tasks[index as usize], index, &mut requests)
                {
                    unrepresentable = Some((base, offset));
                    break;
                }
            }
            if let Some((base, offset)) = unrepresentable {
                self.refuse(
                    recipe,
                    Refusal::AddressNotRepresentable {
                        recipe: id,
                        base,
                        offset,
                    },
                );
                continue;
            }
            // `plan_copies_into` indexes requests with a `u32`. Nothing can
            // reach this — a request needs a task, and tasks are capped by a
            // `u32` limit — but the cast is only sound while it holds, so the
            // bound is checked rather than assumed.
            if requests.len() > u32::MAX as usize {
                self.refuse(
                    recipe,
                    Refusal::RangeLimitExceeded {
                        recipe: id,
                        limit: limits.max_ranges,
                        wanted: u32::MAX,
                    },
                );
                continue;
            }
            super::policy::plan_copies_into(&requests, &policy, &mut planned);

            // Accounting is taken POST-coalescing and PRE-deduplication, on
            // purpose: it is then a property of the recipe alone. Charging
            // after deduplication would make a recipe's own limits depend on
            // which other recipes happened to run first.
            //
            // Accumulation is CHECKED, not saturating. A saturating counter
            // that lands exactly on a limit of `u32::MAX`/`u64::MAX` compares
            // equal to it and the `>` test below does not fire, so overflow
            // would be the one way past a declared cap.
            let state = &mut self.states[recipe];
            let ranges = state
                .planned_ranges
                .checked_add(planned.len() as u32)
                .unwrap_or(u32::MAX);
            let overflowed_ranges = state
                .planned_ranges
                .checked_add(planned.len() as u32)
                .is_none();
            let bytes = state
                .planned_bytes
                .checked_add(planned.total_bytes())
                .unwrap_or(u64::MAX);
            let overflowed_bytes = state
                .planned_bytes
                .checked_add(planned.total_bytes())
                .is_none();
            state.planned_ranges = ranges;
            state.planned_bytes = bytes;

            let widest = planned.widest().unwrap_or(0);
            if widest > limits.max_single_range_bytes {
                self.refuse(
                    recipe,
                    Refusal::RangeTooWide {
                        recipe: id,
                        limit: limits.max_single_range_bytes,
                        wanted: widest,
                    },
                );
                continue;
            }
            if overflowed_ranges || ranges > limits.max_ranges {
                self.refuse(
                    recipe,
                    Refusal::RangeLimitExceeded {
                        recipe: id,
                        limit: limits.max_ranges,
                        wanted: ranges,
                    },
                );
                continue;
            }
            if overflowed_bytes || bytes > limits.max_bytes {
                self.refuse(
                    recipe,
                    Refusal::ByteLimitExceeded {
                        recipe: id,
                        limit: limits.max_bytes,
                        wanted: bytes,
                    },
                );
                continue;
            }

            let mut unreadable = None;
            // The planned copies are sorted and disjoint, so one cursor walks
            // the ownership index for the whole batch.
            let mut cursor = self.owners.sweep();
            for index in 0..planned.copies.len() {
                let span = planned.copies[index].span;
                match self.acquire(&mut cursor, span, &mut acquisition) {
                    Ok(Ok(())) => {}
                    Ok(Err(hole)) => {
                        unreadable = Some(hole);
                        break;
                    }
                    Err(custody) => {
                        result = Err(custody);
                        break;
                    }
                }
            }
            if result.is_err() {
                break;
            }
            if let Some(span) = unreadable {
                self.refuse(recipe, Refusal::MemoryUnreadable { recipe: id, span });
                continue;
            }

            // Ownership and edges are attached per REQUEST, not per planned
            // copy: a merged transaction may serve several tasks with
            // different parents, and each one's edges belong to its own span.
            //
            // The walk is in `planned.order()` — the request indices the
            // coalescer already sorted by address — rather than in request
            // order, so this pass is a second sweep rather than a lookup per
            // request. Every request is still visited exactly once: the order
            // holds every non-empty request, and an empty one attaches to
            // nothing under either order.
            let consumer = self.recipes[recipe].consumer;
            let mut cursor = self.owners.sweep();
            let mut custody = Ok(());
            'attach: for position in 0..planned.order().len() {
                let request = requests[planned.order()[position] as usize];
                let parent = tasks[request.task as usize].parent;
                self.owners
                    .sweep_overlapping(&mut cursor, request.span, &mut owned);
                for id in owned.iter() {
                    custody = self.owners.add_dependent(*id, consumer, request.span);
                    if custody.is_err() {
                        break 'attach;
                    }
                    if let Some(parent) = parent {
                        custody = self.owners.add_edge(parent, *id, consumer);
                        if custody.is_err() {
                            break 'attach;
                        }
                    }
                }
            }
            if let Err(refusal) = custody {
                result = Err(refusal);
                break;
            }
        }
        if result.is_ok() {
            result = self.owners.end_batch();
        }

        self.by_recipe = by_recipe;
        self.requests = requests;
        self.plan = planned;
        self.acquisition = acquisition;
        self.owned = owned;
        result
    }

    /// Copy `span`, skipping whatever is already owned.
    ///
    /// `Ok(Err(hole))` is a memory refusal at that sub-span; `Err(_)` is a
    /// custody refusal, which is not attributable to one recipe.
    fn acquire(
        &mut self,
        cursor: &mut super::dedup::Cursor,
        span: Span,
        plan: &mut super::dedup::AcquisitionPlan,
    ) -> Result<Result<(), Span>, CustodyRefusal> {
        self.owners.sweep_plan(cursor, span, plan);
        for hole in plan.holes.iter().copied() {
            let len = match usize::try_from(hole.len) {
                Ok(len) => len,
                Err(_) => {
                    return Err(CustodyRefusal::AllocationFailed {
                        held: self.arena.held(),
                        requested: hole.len,
                    })
                }
            };
            let offset = self.arena.alloc_zeroed(hole.len)?;
            let memory = self.memory;
            if memory
                .copy_into(hole.start, self.arena.slice_mut(offset, len))
                .is_err()
            {
                // The arena bytes stay allocated and stay accounted. Custody
                // is what the observation HELD, not what it ended up using;
                // pretending a failed copy costs nothing would let a recipe
                // that refuses on every span run past the budget.
                return Ok(Err(hole));
            }
            self.owners.insert(hole, offset)?;
        }
        Ok(Ok(()))
    }

    /// Phase two of a wave: run every decode op against bytes that have landed.
    fn decode_wave(&mut self, tasks: &[CopyTask]) {
        // `recipes` is a `&'a [Recipe]` copied out of `self`, so the ops can be
        // BORROWED while `self` is mutated. Cloning a `RecipeOp` to escape that
        // borrow would allocate its `next` vector once per decode site.
        let recipes = self.recipes;
        let mut stack = std::mem::take(&mut self.sites);
        for task in tasks {
            if self.refused(task.recipe) {
                continue;
            }
            stack.clear();
            match &task.body {
                TaskBody::Op { op, addr, .. } => {
                    let (region, next) = match &recipes[task.recipe].ops[*op as usize] {
                        RecipeOp::CopyHeader { size, next } => (
                            Proof::Object {
                                base: *addr,
                                len: u64::from(*size),
                            },
                            next.as_slice(),
                        ),
                        RecipeOp::CopyRanges { next, .. } => (Proof::None, next.as_slice()),
                        _ => (Proof::None, [].as_slice()),
                    };
                    for op in next.iter().rev() {
                        stack.push(DecodeSite {
                            op: *op,
                            base: *addr,
                            region,
                        });
                    }
                }
                TaskBody::Extent { op, span, count } => {
                    let (stride, element) = match &recipes[task.recipe].ops[*op as usize] {
                        RecipeOp::DecodeExtent {
                            stride, element, ..
                        } => (*stride, element.as_slice()),
                        _ => continue,
                    };
                    // A TERMINAL extent: the bytes are captured and nothing is
                    // derived from them in-bracket. Accounted as one frozen
                    // branch so `frozen_branches` still counts every branch
                    // that ended cleanly, and skipped WITHOUT walking the
                    // elements — the whole point of allowing the empty list is
                    // not to push a site per byte of every string.
                    if element.is_empty() {
                        self.states[task.recipe].frozen_branches =
                            self.states[task.recipe].frozen_branches.saturating_add(1);
                        continue;
                    }
                    let region = Proof::Extent {
                        base: span.start,
                        len: span.len,
                    };
                    // Elements run in the wave the array landed in: indexing
                    // an array the observation already holds is not another
                    // dereference.
                    let mut overflowed = false;
                    for index in (0..*count).rev() {
                        // The array was copied, so it cannot wrap the address
                        // space; the checked form is here so a memory
                        // implementation that accepted a wrapping span cannot
                        // turn into a wrapped element address.
                        let base = match u64::from(index)
                            .checked_mul(u64::from(stride))
                            .and_then(|skew| span.start.checked_add(skew))
                        {
                            Some(base) => base,
                            None => {
                                overflowed = true;
                                break;
                            }
                        };
                        for op in element.iter().rev() {
                            stack.push(DecodeSite { op: *op, base, region });
                        }
                    }
                    if overflowed {
                        self.refuse_unrepresentable(task.recipe, span.start, 0);
                        continue;
                    }
                }
            }
            self.run_sites(task.recipe, task.depth, &mut stack);
        }
        self.sites = stack;
    }

    fn run_sites(&mut self, recipe: usize, depth: u32, stack: &mut Vec<DecodeSite>) {
        // Decode ops can chain to other decode ops (`WitnessClose`), so the
        // walk is bounded rather than assumed acyclic.
        let budget = self.recipes[recipe]
            .limits
            .max_copy_tasks
            .saturating_mul(2)
            .max(1024);
        let mut steps: u32 = 0;
        while let Some(site) = stack.pop() {
            if self.refused(recipe) {
                return;
            }
            steps += 1;
            if steps > budget {
                let id = self.recipes[recipe].id;
                let limit = self.recipes[recipe].limits.max_copy_tasks;
                self.refuse(recipe, Refusal::CopyTaskLimitExceeded { recipe: id, limit });
                return;
            }
            self.run_site(recipe, depth, &site, stack);
        }
    }

    fn run_site(
        &mut self,
        recipe: usize,
        depth: u32,
        site: &DecodeSite,
        stack: &mut Vec<DecodeSite>,
    ) {
        let id = self.recipes[recipe].id;
        // Borrowed, not cloned: see `decode_wave`.
        let recipes = self.recipes;
        match &recipes[recipe].ops[site.op as usize] {
            RecipeOp::DecodePointer {
                offset,
                guard,
                next,
            } => {
                let (offset, guard, next) = (*offset, *guard, *next);
                if !self.guard_admits(recipe, id, site.base, guard) {
                    return;
                }
                let addr = match site.base.checked_add(u64::from(offset)) {
                    Some(addr) => addr,
                    None => return self.refuse_unrepresentable(recipe, site.base, i64::from(offset)),
                };
                let (value, holder) = match self.read_uint(addr, 8) {
                    Some(read) => read,
                    None => {
                        self.refuse(
                            recipe,
                            Refusal::FieldNotCaptured {
                                recipe: id,
                                span: Span::new(addr, 8),
                            },
                        );
                        return;
                    }
                };
                if value == 0 {
                    return;
                }
                let task = CopyTask {
                    recipe,
                    depth: depth + 1,
                    parent: Some(holder),
                    body: TaskBody::Op {
                        op: next,
                        addr: value,
                        anchor: TaskAnchor::Pointer,
                        inherited: Proof::None,
                    },
                };
                self.enqueue(task);
            }
            RecipeOp::DecodeExtent {
                count_offset,
                count_width,
                ptr_offset,
                stride,
                max_count,
                max_bytes,
                trailer_elements,
                guard,
                register,
                ..
            } => {
                let (count_offset, count_width, ptr_offset, stride, max_count, max_bytes, register) = (
                    *count_offset,
                    *count_width,
                    *ptr_offset,
                    *stride,
                    *max_count,
                    *max_bytes,
                    *register,
                );
                let (trailer_elements, guard) = (*trailer_elements, *guard);
                // THE GUARD FIRST, and before any other field is even
                // addressed. Outside its band this op takes no extent at all,
                // so reading the count and the pointer would be reading fields
                // that, in the guarded-out shape, are not a count and a pointer
                // — for the case this exists for they are the characters of a
                // short string.
                if !self.guard_admits(recipe, id, site.base, guard) {
                    return;
                }
                let count_addr = match site.base.checked_add(u64::from(count_offset)) {
                    Some(addr) => addr,
                    None => {
                        return self
                            .refuse_unrepresentable(recipe, site.base, i64::from(count_offset))
                    }
                };
                let ptr_addr = match site.base.checked_add(u64::from(ptr_offset)) {
                    Some(addr) => addr,
                    None => {
                        return self
                            .refuse_unrepresentable(recipe, site.base, i64::from(ptr_offset))
                    }
                };
                let raw_count = match self.read_uint(count_addr, count_width as usize) {
                    Some((value, _)) => value,
                    None => {
                        self.refuse(
                            recipe,
                            Refusal::FieldNotCaptured {
                                recipe: id,
                                span: Span::new(count_addr, u64::from(count_width)),
                            },
                        );
                        return;
                    }
                };
                let (base, base_holder) = match self.read_uint(ptr_addr, 8) {
                    Some(read) => read,
                    None => {
                        self.refuse(
                            recipe,
                            Refusal::FieldNotCaptured {
                                recipe: id,
                                span: Span::new(ptr_addr, 8),
                            },
                        );
                        return;
                    }
                };
                // The TRAILER is part of the request, so it is part of what the
                // limits bound. Adding it after the check would let a recipe
                // copy `max_count + trailer` elements out of a limit that says
                // `max_count`.
                let wanted_count = raw_count.saturating_add(u64::from(trailer_elements));
                if wanted_count > u64::from(max_count) {
                    self.refuse(
                        recipe,
                        Refusal::ExtentCountExceeded {
                            recipe: id,
                            limit: max_count,
                            // Saturating only for a count so absurd it is
                            // already far past any limit worth reporting
                            // exactly.
                            decoded: u32::try_from(wanted_count).unwrap_or(u32::MAX),
                        },
                    );
                    return;
                }
                let count = wanted_count as u32;
                let bytes = u64::from(count) * u64::from(stride);
                if bytes > max_bytes {
                    self.refuse(
                        recipe,
                        Refusal::ExtentBytesExceeded {
                            recipe: id,
                            limit: max_bytes,
                            decoded: bytes,
                        },
                    );
                    return;
                }
                if base == 0 || count == 0 {
                    if let Some(table) = register {
                        self.tables.insert(
                            (recipe, table),
                            TableBinding {
                                span: Span::new(base, 0),
                                count: 0,
                                stride,
                            },
                        );
                    }
                    return;
                }
                let span = Span::new(base, bytes);
                // THE TRAILER IS COPIED AND IT IS NOT AN ELEMENT. It widens
                // the span — which is the whole point, the terminator has to be
                // inside the copy — and it is excluded from both the registered
                // element count and the per-element decode loop. A join that
                // could index it, or an element op that could run against it,
                // would be reaching past the container's own declared end.
                let elements = (count - trailer_elements) as u32;
                if let Some(table) = register {
                    self.tables.insert(
                        (recipe, table),
                        TableBinding {
                            span,
                            count: elements,
                            stride,
                        },
                    );
                }
                let task = CopyTask {
                    recipe,
                    depth: depth + 1,
                    parent: Some(base_holder),
                    body: TaskBody::Extent {
                        op: site.op,
                        span,
                        count: elements,
                    },
                };
                self.enqueue(task);
            }
            RecipeOp::JoinBounded {
                table,
                key_offset,
                key_width,
                next,
            } => {
                let (table, key_offset, key_width, next) =
                    (*table, *key_offset, *key_width, *next);
                let key_addr = match site.base.checked_add(u64::from(key_offset)) {
                    Some(addr) => addr,
                    None => {
                        return self
                            .refuse_unrepresentable(recipe, site.base, i64::from(key_offset))
                    }
                };
                let (key, key_holder) = match self.read_uint(key_addr, key_width as usize) {
                    Some(read) => read,
                    None => {
                        self.refuse(
                            recipe,
                            Refusal::FieldNotCaptured {
                                recipe: id,
                                span: Span::new(key_addr, u64::from(key_width)),
                            },
                        );
                        return;
                    }
                };
                let binding = match self.tables.get(&(recipe, table)) {
                    Some(binding) => *binding,
                    None => {
                        self.refuse(recipe, Refusal::JoinTableMissing { recipe: id, table });
                        return;
                    }
                };
                if key >= u64::from(binding.count) {
                    self.refuse(
                        recipe,
                        Refusal::JoinOutOfRange {
                            recipe: id,
                            table,
                            key,
                            count: binding.count,
                        },
                    );
                    return;
                }
                // `key < binding.count` and `count * stride` was already
                // bounded by the extent's byte cap, so this cannot overflow;
                // the checked form keeps that true if either bound is ever
                // relaxed.
                let addr = match key
                    .checked_mul(u64::from(binding.stride))
                    .and_then(|skew| binding.span.start.checked_add(skew))
                {
                    Some(addr) => addr,
                    None => {
                        self.refuse(
                            recipe,
                            Refusal::JoinOutOfRange {
                                recipe: id,
                                table,
                                key,
                                count: binding.count,
                            },
                        );
                        return;
                    }
                };
                let task = CopyTask {
                    recipe,
                    depth: depth + 1,
                    parent: Some(key_holder),
                    body: TaskBody::Op {
                        op: next,
                        addr,
                        // An address produced by bounded index arithmetic into
                        // a PROVEN extent is a discovered object address, not a
                        // static one, so it is a legitimate window anchor. The
                        // extent proof travels with it.
                        anchor: TaskAnchor::Pointer,
                        inherited: Proof::Extent {
                            base: binding.span.start,
                            len: binding.span.len,
                        },
                    },
                };
                self.enqueue(task);
            }
            RecipeOp::WitnessClose { next } => {
                self.states[recipe].witness_required = true;
                for op in next.iter().rev() {
                    stack.push(DecodeSite {
                        op: *op,
                        base: site.base,
                        region: site.region,
                    });
                }
            }
            RecipeOp::Freeze => {
                self.states[recipe].frozen_branches =
                    self.states[recipe].frozen_branches.saturating_add(1);
            }
            RecipeOp::CopyHeader { .. } | RecipeOp::CopyRanges { .. } => {
                // Unreachable: validation rejects a copy op in a decode slot.
                debug_assert!(false, "copy op reached in a decode slot");
            }
        }
    }

    /// Evaluate a decode op's guard. `false` means the branch ends here — either
    /// because the guard said so (cleanly) or because the recipe has been
    /// refused for not capturing the guard word.
    ///
    /// One spelling for both guarded ops. Two would be how the string guard and
    /// the tree-sentinel guard drift into different refusal behaviour, and the
    /// refusal behaviour is the load-bearing half: an uncaptured guard word must
    /// never read as "the guard said no", because that silently drops every
    /// payload behind it while reporting `Captured`.
    fn guard_admits(
        &mut self,
        recipe: usize,
        id: RecipeId,
        base: u64,
        guard: Option<ExtentGuard>,
    ) -> bool {
        let Some(guard) = guard else {
            return true;
        };
        let addr = match base.checked_add(u64::from(guard.offset)) {
            Some(addr) => addr,
            None => {
                self.refuse_unrepresentable(recipe, base, i64::from(guard.offset));
                return false;
            }
        };
        match self.read_uint(addr, guard.width as usize) {
            Some((value, _)) => guard.admits(value),
            None => {
                self.refuse(
                    recipe,
                    Refusal::FieldNotCaptured {
                        recipe: id,
                        span: Span::new(addr, u64::from(guard.width)),
                    },
                );
                false
            }
        }
    }

    /// Read an unsigned little-endian integer out of the arena, with the
    /// interval that held it.
    ///
    /// The holder comes back with the value because every caller needs both
    /// and looking it up twice is one ordered lookup per decode site: the
    /// interval a field was read out of IS the discovery edge's parent.
    fn read_uint(&self, addr: u64, width: usize) -> Option<(u64, IntervalId)> {
        let (bytes, holder) = self.owners.read_at(&self.arena, addr, width)?;
        let mut raw = [0u8; 8];
        raw[..width].copy_from_slice(bytes);
        Some((u64::from_le_bytes(raw), holder))
    }

    /// Attach the dependency edges of tasks that deduplication pruned.
    fn link_pruned(&mut self) -> Result<(), CustodyRefusal> {
        let pruned = std::mem::take(&mut self.pruned);
        let mut requests = std::mem::take(&mut self.requests);
        let mut owned = std::mem::take(&mut self.owned);
        let mut custody = Ok(());
        for task in pruned {
            if custody.is_err() {
                break;
            }
            if self.refused(task.recipe) {
                continue;
            }
            let consumer = self.recipes[task.recipe].consumer;
            requests.clear();
            // A pruned duplicate has the same op and address as the task that
            // was expanded, so an unrepresentable address here would already
            // have refused the recipe. Handled rather than ignored so that
            // stays true if either ever stops holding.
            if let Err((base, offset)) = self.requests_of(&task, 0, &mut requests) {
                self.refuse_unrepresentable(task.recipe, base, offset);
                continue;
            }
            'attach: for index in 0..requests.len() {
                let span = requests[index].span;
                self.owners.overlapping_into(span, &mut owned);
                for id in owned.iter() {
                    custody = self.owners.add_dependent(*id, consumer, span);
                    if custody.is_err() {
                        break 'attach;
                    }
                    if let Some(parent) = task.parent {
                        custody = self.owners.add_edge(parent, *id, consumer);
                        if custody.is_err() {
                            break 'attach;
                        }
                    }
                }
            }
        }
        self.requests = requests;
        self.owned = owned;
        custody
    }

    /// An interval is witnessed when any consumer depending on it asked for a
    /// witness. Sharing therefore widens the closing wave, never narrows it.
    fn enrol_witnesses(&mut self) -> Result<(), CustodyRefusal> {
        let wanted: BTreeSet<ConsumerId> = self
            .recipes
            .iter()
            .zip(self.states.iter())
            .filter(|(_, state)| state.witness_required && state.refusal.is_none())
            .map(|(recipe, _)| recipe.consumer)
            .collect();
        if wanted.is_empty() {
            return Ok(());
        }
        // BY POSITION, not through a collected id list. The list existed only
        // to end the borrow of `links` before `enrol_witness` takes `&mut`,
        // and it was one `Vec<IntervalId>` the size of the whole record table
        // — work-scaled memory the ledger never saw. Re-borrowing per record
        // ends the borrow at the same place and allocates nothing. A record is
        // 32 bytes and `Copy`.
        for position in 0..self.owners.len() {
            let record = self.owners.records()[position];
            let enrol = self
                .owners
                .links()
                .consumers(&record)
                .any(|consumer| wanted.contains(&consumer));
            if enrol {
                self.owners.enrol_witness(record.id())?;
            }
        }
        Ok(())
    }

    /// The fourteenth phase: re-copy the opening addresses.
    ///
    /// This does NOT re-run discovery. A closing witness re-copies what the
    /// opening wave already found; a parent that changed invalidates its
    /// dependent subtree afterwards, which is cheaper than a reverse chase and
    /// strictly more conservative.
    fn closing_wave(&mut self) -> Result<(u32, Vec<IntervalId>), CustodyRefusal> {
        // Enrolment order, which `enrol_witnesses` produces by walking the
        // records in id order: the same order a scan of every record produced,
        // without the scan.
        //
        // Walked BY POSITION rather than through a clone of the enrolment
        // list. The clone was there only to release the borrow before
        // `record_mut`, and it was a second `Vec<IntervalId>` as long as the
        // witness list — work-scaled memory the ledger never saw, taken at the
        // exact moment custody is fullest.
        let witnessed = self.owners.witnessed_ids().len();
        let mut unreadable = Vec::new();
        for position in 0..witnessed {
            let id = self.owners.witnessed_ids()[position];
            let id = &id;
            let span = self.owners.record(*id).span();
            let len = match usize::try_from(span.len) {
                Ok(len) => len,
                Err(_) => {
                    return Err(CustodyRefusal::AllocationFailed {
                        held: self.arena.held(),
                        requested: span.len,
                    })
                }
            };
            let offset = self.arena.alloc_zeroed(span.len)?;
            let memory = self.memory;
            if memory
                .copy_into(span.start, self.arena.slice_mut(offset, len))
                .is_err()
            {
                // Charged: this list is bounded by the witnessed interval
                // count, it grows at the moment custody is fullest, and it
                // survives into `SnapshotBuild`. A region that became
                // unreadable mid-bracket can put every witnessed interval on
                // it.
                super::arena::charged_push(&mut unreadable, self.arena.ledger_ref(), *id)?;
                continue;
            }
            self.owners.record_mut(*id).set_closing_offset(offset);
        }
        Ok((witnessed as u32, unreadable))
    }

    fn build_reports(&self) -> Vec<RecipeReport> {
        self.recipes
            .iter()
            .zip(self.states.iter())
            .map(|(recipe, state)| RecipeReport {
                recipe: recipe.id,
                consumer: recipe.consumer,
                outcome: match state.refusal {
                    Some(refusal) => RecipeOutcome::Refused(refusal),
                    None => RecipeOutcome::Captured,
                },
                copy_tasks: state.copy_tasks,
                planned_ranges: state.planned_ranges,
                planned_bytes: state.planned_bytes,
                frozen_branches: state.frozen_branches,
                witness_required: state.witness_required,
            })
            .collect()
    }
}

/// Per-phase access to a live [`Observation`], so the executor's dispatch
/// block can be PRICED rather than inferred from a residual.
///
/// The substrate's flat 100k-transaction figure is about twice the sum of the
/// named component rows in [`super::bench`], and until this existed the
/// difference had no owner: task materialisation, the wave queue, request
/// resolution and the decode dispatch are all inside `run`, none of them is
/// reachable from outside the module, and none of them had a row. This probe
/// runs the REAL methods — `enqueue`, `requests_of`, `copy_wave`, `decode_wave`
/// — one phase per call, so the caller can put a timer around exactly one of
/// them.
///
/// Test-only. It exposes the observation's internal phase order, which nothing
/// in production may drive out of order.
#[cfg(test)]
pub(super) struct DispatchProbe<'a, M: SnapshotMemory> {
    observation: Observation<'a, M>,
}

#[cfg(test)]
impl<'a, M: SnapshotMemory> DispatchProbe<'a, M> {
    pub(super) fn new(
        recipes: &'a [Recipe],
        memory: &'a M,
        config: ObservationConfig,
    ) -> Result<Self, CustodyRefusal> {
        let arena = Arena::with_budget(config.custody_budget());
        Ok(Self {
            observation: Observation::new(recipes, memory, config, arena)?,
        })
    }

    /// Fold every task's fields into a checksum.
    ///
    /// The construct row's sink is otherwise observed only through its length,
    /// and a length is something the optimiser can keep while dropping the
    /// stores that filled the elements — which would price an empty loop. This
    /// reads the parts of every task that construction wrote, so none of them
    /// is dead.
    pub(super) fn checksum(tasks: &[CopyTask]) -> u64 {
        let mut sum = 0u64;
        for task in tasks {
            sum = sum
                .wrapping_mul(31)
                .wrapping_add(task.recipe as u64)
                .wrapping_add(u64::from(task.depth))
                .wrapping_add(u64::from(task.parent.map_or(0, |id| id.0)));
            sum = sum.wrapping_add(match &task.body {
                TaskBody::Op {
                    op,
                    addr,
                    anchor,
                    inherited,
                } => {
                    // EVERY field construction wrote, `inherited` included. A
                    // checksum that skipped it would let the optimiser drop the
                    // twenty-four bytes it stores and the row would price a
                    // narrower task than the executor builds.
                    let proof = match inherited {
                        Proof::None => 1,
                        Proof::Object { base, len } => base ^ len ^ 2,
                        Proof::Extent { base, len } => base ^ len ^ 3,
                    };
                    addr ^ u64::from(*op) ^ u64::from(*anchor == TaskAnchor::Root) ^ proof
                }
                TaskBody::Extent { op, span, count } => {
                    span.start ^ span.len ^ u64::from(*op) ^ u64::from(*count)
                }
            });
        }
        sum
    }

    /// Fold every request's fields into a checksum, for the same reason.
    ///
    /// `requests_of` writes a span, an anchor and a proof per request, and a
    /// row that only counted them would let the provenance construction — the
    /// part the anchor erasure changed — be optimised away.
    pub(super) fn requests_checksum(&self) -> u64 {
        let mut sum = 0u64;
        for request in &self.observation.requests {
            let anchor = match request.provenance.anchor {
                AnchorKind::Root(addr) => addr ^ 1,
                AnchorKind::Pointer(addr) => addr ^ 2,
                AnchorKind::Mixed => 3,
            };
            let proof = match request.provenance.proof {
                Proof::None => 1,
                Proof::Object { base, len } => base ^ len ^ 2,
                Proof::Extent { base, len } => base ^ len ^ 3,
            };
            sum = sum
                .wrapping_mul(31)
                .wrapping_add(request.span.start ^ request.span.len)
                .wrapping_add(anchor)
                .wrapping_add(proof)
                .wrapping_add(u64::from(request.task));
        }
        sum
    }

    /// Fold the grouping pass's output, so the indices it wrote are read.
    pub(super) fn groups_checksum(&self) -> u64 {
        let mut sum = 0u64;
        for group in &self.observation.by_recipe {
            for index in group {
                sum = sum.wrapping_mul(31).wrapping_add(u64::from(*index));
            }
        }
        sum
    }

    /// Sizes of the values the dispatch block moves, in bytes. A queue push is
    /// a memcpy of `CopyTask`, so its width is a priced quantity and not a
    /// detail.
    pub(super) fn value_sizes() -> [(&'static str, usize); 4] {
        [
            ("CopyTask", std::mem::size_of::<CopyTask>()),
            ("TaskBody", std::mem::size_of::<TaskBody>()),
            ("CopyRequest", std::mem::size_of::<CopyRequest>()),
            ("DecodeSite", std::mem::size_of::<DecodeSite>()),
        ]
    }

    /// Every recipe's roots, constructed and enqueued: the whole
    /// materialisation path (`CopyTask` literal, depth check, visited insert,
    /// task-limit check, wave push). Returns the tasks queued at depth zero.
    pub(super) fn seed_roots(&mut self) -> usize {
        for (index, recipe) in self.observation.recipes.iter().enumerate() {
            for root in &recipe.roots {
                let task = CopyTask {
                    recipe: index,
                    depth: 0,
                    parent: None,
                    body: TaskBody::Op {
                        op: root.op,
                        addr: root.addr,
                        anchor: TaskAnchor::Root,
                        inherited: Proof::None,
                    },
                };
                self.observation.enqueue(task);
            }
        }
        self.observation.waves[0].len()
    }

    /// The `CopyTask` values alone, into a caller-owned sink: construction and
    /// the queue push, with the visited insert and the limit checks left out.
    /// The sink is the same shape as a wave queue (`Vec<CopyTask>`, reused), so
    /// the push side is the real one.
    pub(super) fn construct_only(recipes: &[Recipe], sink: &mut Vec<CopyTask>) {
        sink.clear();
        for (index, recipe) in recipes.iter().enumerate() {
            for root in &recipe.roots {
                sink.push(CopyTask {
                    recipe: index,
                    depth: 0,
                    parent: None,
                    body: TaskBody::Op {
                        op: root.op,
                        addr: root.addr,
                        anchor: TaskAnchor::Root,
                        inherited: Proof::None,
                    },
                });
            }
        }
    }

    /// Take a wave's tasks out of the queue, as `run` does.
    pub(super) fn take_wave(&mut self, depth: u32) -> Vec<CopyTask> {
        std::mem::take(&mut self.observation.waves[depth as usize])
    }

    pub(super) fn put_wave(&mut self, depth: u32, tasks: Vec<CopyTask>) {
        self.observation.waves[depth as usize] = tasks;
    }

    /// The production root-queue reservation, so a row can price the seeding
    /// path WITH it against a row that prices the same path without it. A probe
    /// that skipped this would be pricing a shape production does not run.
    pub(super) fn reserve_root_queue(&mut self) -> Result<(), CustodyRefusal> {
        self.observation.reserve_root_queue()
    }

    /// `run`'s validation pass, which the reservation depends on having run:
    /// it is what tells `reserve_root_queue` which recipes can enqueue.
    pub(super) fn validate_recipes(&mut self) {
        for (index, recipe) in self.observation.recipes.iter().enumerate() {
            if let Err(refusal) = validate(recipe) {
                self.observation.states[index].refusal = Some(refusal);
            }
        }
    }

    /// `copy_wave`'s grouping pass on its own: clear the groups, size them to
    /// the recipe count, and index every task into its recipe's group.
    pub(super) fn group_pass(&mut self, tasks: &[CopyTask]) -> usize {
        let observation = &mut self.observation;
        for group in observation.by_recipe.iter_mut() {
            group.clear();
        }
        observation.by_recipe.resize_with(observation.recipes.len(), Vec::new);
        for (index, task) in tasks.iter().enumerate() {
            observation.by_recipe[task.recipe].push(index as u32);
        }
        observation.by_recipe.iter().map(|group| group.len()).sum()
    }

    /// `requests_of` over a whole wave, appending into the observation's own
    /// reused request buffer exactly as `copy_wave` does.
    ///
    /// `reserve` selects which of the two shapes is priced: `true` is what
    /// `copy_wave` does — the batch's task count reserved before the first
    /// request is pushed — and `false` is the buffer growing from wherever it
    /// is, which is what the same loop cost before that reservation existed.
    pub(super) fn requests_pass(&mut self, tasks: &[CopyTask], reserve: bool) -> usize {
        let mut requests = std::mem::take(&mut self.observation.requests);
        requests.clear();
        if reserve {
            Observation::<M>::reserve_own(
                &mut requests,
                self.observation.arena.ledger_ref(),
                &mut self.observation.own_tables_charged,
                tasks.len(),
            )
            .expect("custody budget");
        }
        for (index, task) in tasks.iter().enumerate() {
            self.observation
                .requests_of(task, index as u32, &mut requests)
                .expect("bench tasks have representable addresses");
        }
        let produced = requests.len();
        self.observation.requests = requests;
        produced
    }

    /// The real `copy_wave`, so a probe can get bytes on the ground before the
    /// decode row runs against them.
    pub(super) fn copy_pass(&mut self, tasks: &[CopyTask]) -> Result<(), CustodyRefusal> {
        self.observation.copy_wave(tasks)?;
        self.observation.owners.end_wave()
    }

    /// The real `decode_wave`: the per-task body match, the `DecodeSite` push
    /// per branch, and `run_sites`/`run_site` draining them.
    pub(super) fn decode_pass(&mut self, tasks: &[CopyTask]) {
        self.observation.decode_wave(tasks);
    }

    /// Tasks the decode pass enqueued at `depth`, so a probe can assert it did
    /// the work rather than trusting the clock.
    pub(super) fn queued_at(&self, depth: u32) -> usize {
        self.observation.waves[depth as usize].len()
    }

    pub(super) fn frozen_branches(&self) -> u32 {
        self.observation.states.iter().map(|state| state.frozen_branches).sum()
    }

    pub(super) fn charge_tables(&mut self) -> Result<(), CustodyRefusal> {
        self.observation.charge_own_tables(0)
    }

    /// Reconcile with a wave's queue held outside `self.waves`.
    ///
    /// Routed through the SAME method `run`'s wave loop calls, not through
    /// `charge_own_tables` with a number this probe made up. A probe that
    /// passed its own `in_flight` would be testing arithmetic rather than the
    /// path production takes, and would keep passing if the wave loop stopped
    /// handing its queue over.
    pub(super) fn charge_tables_holding(
        &mut self,
        tasks: &Vec<CopyTask>,
    ) -> Result<(), CustodyRefusal> {
        self.observation.charge_own_tables_holding(tasks)
    }

    /// The footprint the reconciliation measures, asked for through the
    /// production accessor rather than recomputed.
    pub(super) fn own_tables_bytes(&self, in_flight: u64) -> u64 {
        self.observation.own_tables_bytes(in_flight)
    }

    /// Bytes a held wave queue occupies, measured the way custody measures a
    /// table.
    pub(super) fn wave_bytes(tasks: &Vec<CopyTask>) -> u64 {
        super::arena::vec_bytes(tasks)
    }

    /// Table bytes the custody ledger says the observation holds.
    pub(super) fn ledger_table_bytes(&self) -> u64 {
        self.observation.arena.ledger_ref().table_bytes()
    }

    /// The executor's own reconciled footprint: what `charge_own_tables` has
    /// brought the ledger up to, plus anything a reservation advanced it by.
    /// Against [`Self::ledger_table_bytes`] this is the double-charge check.
    pub(super) fn own_tables_charged(&self) -> u64 {
        self.observation.own_tables_charged
    }

    pub(super) fn wave_capacity(&self, depth: u32) -> usize {
        self.observation.waves[depth as usize].capacity()
    }
}
