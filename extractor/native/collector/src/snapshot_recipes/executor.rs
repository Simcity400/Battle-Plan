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
    RecipeId, Refusal, RefusalRecord, RefusalWitness, SnapshotMemory, Span, TableId,
    DEFAULT_CUSTODY_BUDGET_BYTES, FORWARD_WAVES, MAX_RECIPE_DEPTH,
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

/// One term of a [`CountExpr`]: an unsigned field of already-copied bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CountTerm {
    pub offset: u32,
    pub width: u8,
}

/// An element count derived from a BOUNDED SUM of copied header fields.
///
/// # Why this is still address-and-length decoding
///
/// d81's hash tables size themselves `mask + max_probe + 2` — the engine's own
/// formula, already in production in `d81_hash_table_rows` — because
/// `select_secondary` starts at the masked slot, advances without wrapping, and
/// reads the terminating marker one slot past its last admitted step. A recipe
/// that could only name ONE count field could not copy that table without
/// copying short of it.
///
/// The expression is a sum of at most [`super::MAX_COUNT_EXPR_TERMS`] fields
/// and a constant. There is no multiplication, no field-to-field reference, no
/// branching and no iteration: it computes a LENGTH, which is precisely what
/// guarantee 2 permits between waves. Arithmetic is checked in `u128` and a sum
/// past the op's `max_count` is [`Refusal::ExtentCountExceeded`], never a
/// truncated table.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CountExpr {
    pub terms: Vec<CountTerm>,
    pub constant: u32,
}

/// What a rejected pointer does to the branch that decoded it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RejectAction {
    /// Stop the branch and RECORD the rejection with its witness. The recipe
    /// keeps its other branches and its consumer keeps its hour. This is d81's
    /// shape: it rejects a pointer, reports its own typed `InvalidPointer` with
    /// stage and subject, and walks on.
    StopBranch,
    /// Refuse the recipe with [`Refusal::PointerNotAdmissible`].
    RefuseRecipe,
}

/// The predicate a decoded pointer must satisfy before it is followed.
///
/// d81 admits a pointer on a heap range plus 8-byte alignment, and the
/// selector-owner route additionally admits one exact static address. Without
/// this the substrate follows pointers the reader would have rejected: if the
/// address happens to be readable it copies a wrong object, and if it does not
/// it turns a typed `InvalidPointer` into a generic memory refusal that gaps
/// the whole recipe.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Admissibility {
    /// Inclusive low bound.
    pub low: u64,
    /// EXCLUSIVE high bound.
    pub high: u64,
    /// Required alignment, a power of two. 0 or 1 means none.
    pub align: u32,
    /// Exact addresses admitted regardless of the window. Zero entries are
    /// unused; a null pointer never reaches the predicate, so 0 cannot be a
    /// meaningful singleton.
    pub singletons: [u64; 2],
    pub on_reject: RejectAction,
}

impl Admissibility {
    pub fn admits(&self, addr: u64) -> bool {
        if self.singletons.contains(&addr) && addr != 0 {
            return true;
        }
        if addr < self.low || addr >= self.high {
            return false;
        }
        if self.align > 1 && addr % u64::from(self.align) != 0 {
            return false;
        }
        true
    }

    fn well_formed(&self) -> bool {
        self.high > self.low && (self.align <= 1 || self.align.is_power_of_two())
    }
}

/// Where a branch's carried scalar comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CarrySource {
    /// Read it out of already-copied bytes at the decode site.
    Field { offset: u32, width: u8 },
    /// A literal.
    Constant(u64),
    /// The site's index within the decoded extent it is an element of.
    ///
    /// d81's whole province walk is keyed on this and on nothing else: the rail
    /// table is indexed BY province id, so a slot's id is its position and is
    /// not written anywhere in the slot. A carry that could only read fields
    /// therefore could not express the walk at all. Zero at any site that is
    /// not an extent element, which is what `index` already means there.
    ElementIndex,
}

/// ONE scalar travels with a discovery branch, and it is one on purpose.
///
/// d81 needs a topology province id to select a rail slot, then — after
/// following the rail-info pointer, a wave later — the SAME id to select an
/// active entry and a map-province slot; and in the hub an outer POV id selects
/// a foreign row and later a relation-table entry. Every carrying requirement
/// the census found reuses a single identity, so the context is a single `u64`
/// rather than a register file. That keeps [`CopyTask`] and [`DecodeSite`] —
/// the two types the hottest loops in the substrate copy — eight bytes wider
/// instead of thirty-two.
pub type Carry = Option<u64>;

/// A [`Carry`] as it is STORED on a task and a decode site.
///
/// `Option<u64>` has no niche, so it is sixteen bytes; a `u64` and a flag is
/// nine, and the flag packs into padding both [`CopyTask`] and [`DecodeSite`]
/// already had. That is not a micro-optimisation on this path: those two types
/// are memcpy'd into and out of the wave queue and the site stack once per task
/// and once per decode site, which is the hottest loop the substrate has, and
/// the `Option` spelling measured at +23% on `dispatch: construct + queue push`
/// and +81% on `dispatch: decode_wave` in the substrate's own bench.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct CarrySlot {
    value: u64,
    set: bool,
}

impl CarrySlot {
    #[inline]
    fn get(self) -> Carry {
        if self.set {
            Some(self.value)
        } else {
            None
        }
    }

    #[inline]
    fn from(carry: Carry) -> Self {
        match carry {
            Some(value) => Self { value, set: true },
            None => Self {
                value: 0,
                set: false,
            },
        }
    }
}

/// What a disagreement between back-to-back images does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisagreeAction {
    /// Gap the consumers of the interval. The copier knows the image is torn,
    /// so it never issues it.
    Gap,
    /// Record it and issue the image anyway. For a reader that resolves a
    /// disagreement itself — d81's active-window sampler takes up to three
    /// samples and stops when two agree.
    Record,
}

/// How many back-to-back images one copy takes.
///
/// # This is the only instrument that can see a torn read
///
/// Every `read_*_stable` in d81 is two adjacent guarded reads compared against
/// each other, and `d81_read_stable_bytes` does the same to a whole hash table.
/// The substrate could not express it at all: duplicate copy tasks are pruned,
/// acquisition copies only holes it does not already own, and the closing slot
/// is populated only after every forward wave — nanoseconds is not a gap the
/// opening/closing pair can measure.
///
/// WHAT IT DOES NOT FIX, stated because no API can: the sampling INTERVAL. A
/// pair taken over a 400 KB table is not the same instrument as a pair taken
/// over an 8-byte slot; the window a concurrent writer has to be caught in
/// widens with the range. That is inherent to copy-only brackets and its cost
/// can only be measured from a capture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SampleSpec {
    /// 1..=[`super::MAX_ADJACENT_SAMPLES`].
    pub count: u8,
    pub on_disagree: DisagreeAction,
}

impl Default for SampleSpec {
    fn default() -> Self {
        Self {
            count: 1,
            on_disagree: DisagreeAction::Gap,
        }
    }
}

/// What an unreadable span inside this op's copy costs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tolerance {
    /// The recipe refuses: [`Refusal::MemoryUnreadable`]. The default, and the
    /// only setting under which guarantee 1 is unqualified.
    #[default]
    Required,
    /// RULING A. The unreadable span is recorded as a [`super::Void`] and the
    /// rest of the copy lands. Only a read that touches the void refuses, so a
    /// row whose consumed bytes never reach it publishes.
    ///
    /// It is called SCOPED and not "speculative" because it does two jobs and
    /// only one of them is speculation:
    ///
    ///  * the bytes a copy plan takes because it must take supersets — the
    ///    buckets, nodes, anchors and table tails a selective reader would never
    ///    have touched. No consumer reaches these, so nothing refuses;
    ///  * the bytes ONE ROW consumes out of a shared engine table. The row
    ///    whose slot is in the void refuses, at its own read, and every other
    ///    row of the same table publishes.
    ///
    /// The second is the case ruling A is actually about, and it is why
    /// declaring `Scoped` is not a way of making a failure disappear: it moves
    /// the refusal from the RECIPE to the READ, which is the finest granularity
    /// consumed bytes have. A row that needs the bytes still fails, by name,
    /// with [`super::view::ViewRefusal::Unreadable`].
    Scoped,
}

/// The bytes a reader CONSUMES out of the object an op copies.
///
/// Empty means all of them, which is what every op meant before ruling B.
/// Windows are relative to the op's own address, exactly like
/// [`RecipeOp::CopyRanges`]'s.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CompareMask {
    pub windows: Vec<RelWindow>,
}

/// One stage of a recipe's closing program.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CloseStage {
    /// The stage tag its ops carry.
    pub stage: u16,
    /// Close nothing for this stage where a LATER stage covers the same
    /// interval; that stage's close reports in this one's clothes.
    ///
    /// d81 does exactly this: when the selector slot and the source slot
    /// coincide, the earlier selector close is elided in favour of the later
    /// source-position read, and selector failure attribution is retained.
    pub elide_into: Option<u16>,
}

/// RULING B. The order a recipe closes its witnesses in.
///
/// Stages close in the order they are listed; intervals carrying no listed
/// stage close last, in interval-id order, which is exactly the single generic
/// wave the substrate had before. Elision is per stage and it keeps the elided
/// stage's attribution.
///
/// WHAT THIS DOES NOT DO: run a close mid-walk. d81 closes its rail header
/// inside each province's iteration; reproducing that requires closing copies
/// interleaved with discovery, which is a different executor topology from
/// "every forward wave, then closes". Ordering and elision are expressible;
/// mid-walk scheduling is recorded as a residual rather than pretended.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClosingProgram {
    pub stages: Vec<CloseStage>,
}

/// Opt-in qualifications on a single op.
///
/// # Why a side table and not more fields on [`RecipeOp`]
///
/// Nine of these ten knobs are unused by nine ops out of ten. Putting them in
/// the enum widens every op in every recipe — the enum is matched once per
/// decode site — to carry defaults. A modifier table is either EMPTY, in which
/// case the whole vocabulary costs one branch per site and the executor behaves
/// exactly as it did, or it is one entry per op and the lookup is an array
/// index. `validate` rejects anything in between, because a partial table would
/// silently apply an op's qualifications to its neighbour.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OpMod {
    /// `DecodeExtent` only: derive the count instead of reading one field.
    pub count: Option<CountExpr>,
    /// `DecodePointer`/`DecodeExtent` only: admit the decoded pointer first.
    pub admit: Option<Admissibility>,
    /// Any decode op: set the branch's carried scalar for everything below.
    pub carry: Option<CarrySource>,
    /// `JoinBounded` only: index by the carried scalar rather than a field.
    pub key_from_carry: bool,
    /// `DecodeExtent`/`JoinBounded` only: scope the table binding to the
    /// carried scalar, so one logical table has independent per-instance
    /// bindings.
    pub instance_from_carry: bool,
    /// Copy ops only.
    pub samples: SampleSpec,
    /// Copy ops only.
    pub tolerance: Tolerance,
    /// Copy ops only.
    pub compare: CompareMask,
    /// Any op: the stage tag its intervals and its refusals carry.
    pub stage: u16,
}

/// The answer for every op of every recipe that declares no modifier table.
///
/// A `static` rather than a `OnceLock` or a per-call temporary: `Vec::new` is a
/// const fn, so the whole value is built at compile time and reading it is a
/// static address. See [`Recipe::op_mod`] for why that matters.
static UNQUALIFIED: OpMod = OpMod {
    count: None,
    admit: None,
    carry: None,
    key_from_carry: false,
    instance_from_carry: false,
    samples: SampleSpec {
        count: 1,
        on_disagree: DisagreeAction::Gap,
    },
    tolerance: Tolerance::Required,
    compare: CompareMask {
        windows: Vec::new(),
    },
    stage: 0,
};

impl OpMod {
    fn is_default(&self) -> bool {
        self.count.is_none()
            && self.admit.is_none()
            && self.carry.is_none()
            && !self.key_from_carry
            && !self.instance_from_carry
            && self.samples == SampleSpec::default()
            && self.tolerance == Tolerance::default()
            && self.compare.windows.is_empty()
            && self.stage == 0
    }
}

/// The predicate a [`RecipeOp::SelectByValue`] evaluates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Probe {
    /// An unsigned field of already-copied bytes equals a literal.
    FieldEquals { offset: u32, width: u8, value: u64 },
    /// An unsigned field lies in `[low, high]`, inclusive.
    FieldInRange {
        offset: u32,
        width: u8,
        low: u64,
        high: u64,
    },
    /// The first two back-to-back images of the bytes at this window agree.
    /// Requires the copy that produced them to have declared a sample count
    /// above one; an interval with no second image never agrees, which is the
    /// conservative answer.
    SamplesAgree { offset: u32, len: u32 },
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
    /// DECODE. RULING C (2026-08-09). Evaluate a BOUNDED predicate over
    /// already-copied bytes and choose which copy op runs next.
    ///
    /// # The structural/semantic line, at the op that walks it
    ///
    /// STRUCTURAL discovery — permitted here — decides WHAT TO COPY. Which of
    /// two layouts an object has; whether a wide window's two images agree so
    /// the wide copy stands, or disagree so the three narrow copies are taken
    /// instead; which of an anchor triplet's members the next hop is keyed on.
    /// The op's whole output is an edge in the copy graph.
    ///
    /// SEMANTIC evaluation — still forbidden — computes a RESULT. Filtering
    /// rows, comparing a field against a threshold that decides what the reader
    /// will report, accumulating, or letting a value out of the bracket in any
    /// form other than the copied bytes themselves.
    ///
    /// The bound is what keeps the two apart, and it is in the type rather than
    /// in a comment: the probe reads one field of at most eight bytes, or
    /// compares at most [`super::MAX_PROBE_COMPARE_BYTES`] bytes of two images
    /// that are ALREADY in the arena; there is exactly one taken arm; the arms
    /// are copy ops, so the only thing an arm can do is copy; and nothing the
    /// probe read is retained, passed on, or readable by the consumer. A
    /// program cannot chain these to compute: the op has no output to chain.
    ///
    /// The chosen arm copies at the SITE'S OWN ADDRESS, one wave deeper. The
    /// extra wave is the honest cost of deciding from copied bytes: the bytes
    /// have to have landed before the decision, and the next copy has to be
    /// submitted after it.
    SelectByValue {
        probe: Probe,
        /// Copy op to run when the predicate holds.
        then: u16,
        /// Copy op to run when it does not. `None` ends the branch, which is
        /// a refusal ([`Refusal::ConditionalUnresolved`]) rather than a silent
        /// stop: a conditional that quietly does nothing is a short result
        /// reported as success.
        otherwise: Option<u16>,
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
    /// Per-op qualifications. EMPTY or exactly `ops.len()` long; see [`OpMod`].
    pub mods: Vec<OpMod>,
    /// RULING B. Closure order and elision. `None` is the single generic
    /// closing wave.
    pub closing: Option<ClosingProgram>,
}

impl Recipe {
    /// A recipe with default limits, the default policy, no per-op
    /// qualifications and the generic closing wave.
    pub fn new(id: RecipeId, consumer: ConsumerId, ops: Vec<RecipeOp>, roots: Vec<Root>) -> Self {
        Self {
            id,
            consumer,
            limits: Limits::default(),
            policy: GapPolicy::default(),
            ops,
            roots,
            mods: Vec::new(),
            closing: None,
        }
    }

    /// Attach a full modifier table. Length is checked by [`validate`].
    pub fn with_mods(mut self, mods: Vec<OpMod>) -> Self {
        self.mods = mods;
        self
    }

    pub fn with_closing(mut self, closing: ClosingProgram) -> Self {
        self.closing = Some(closing);
        self
    }

    /// The qualifications of one op.
    ///
    /// A recipe with no modifier table answers [`UNQUALIFIED`], so the caller
    /// pays one `is_empty` test — no allocation, no lock, no atomic. This is on
    /// the hottest path the substrate has (once per decode site), and lane FW's
    /// measurement is why it is spelled this way: the substrate already charges
    /// 618–672 ns per transaction against 7.6 ns of actual copying, so a
    /// vocabulary that added even an atomic load per site would be paying for
    /// expressiveness out of the same budget the conversion is trying to win.
    #[inline]
    pub(super) fn op_mod(&self, op: u16) -> &OpMod {
        if self.mods.is_empty() {
            return &UNQUALIFIED;
        }
        &self.mods[op as usize]
    }

    /// True when anything in this recipe needs the per-interval annotation
    /// tables. Computed once per observation, not per site.
    pub(super) fn needs_annotations(&self) -> bool {
        self.closing.is_some() || self.mods.iter().any(|op| !op.is_default())
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

    // The modifier table is all or nothing. A short table would leave the tail
    // of the program unqualified; a long one would carry qualifications for ops
    // that do not exist, and neither is distinguishable from a table whose
    // entries have simply been misaligned by an edit.
    if !recipe.mods.is_empty() && recipe.mods.len() != count {
        return Err(bad(MalformedReason::ModsLengthMismatch {
            ops: count as u32,
            mods: recipe.mods.len() as u32,
        }));
    }

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
            RecipeOp::SelectByValue {
                probe,
                then,
                otherwise,
            } => {
                match probe {
                    Probe::FieldEquals { width, .. } | Probe::FieldInRange { width, .. } => {
                        if !matches!(width, 1 | 2 | 4 | 8) {
                            return Err(bad(MalformedReason::BadWidth {
                                op: here,
                                width: *width,
                            }));
                        }
                    }
                    Probe::SamplesAgree { len, .. } => {
                        if *len == 0 || *len > super::MAX_PROBE_COMPARE_BYTES {
                            return Err(bad(MalformedReason::ProbeTooWide {
                                op: here,
                                len: *len,
                            }));
                        }
                    }
                }
                // Both arms are COPY ops. That is the bound in the type: the
                // only thing a conditional can do is choose what to copy.
                expect(*then, OpKind::Copy)?;
                if let Some(target) = otherwise {
                    expect(*target, OpKind::Copy)?;
                }
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
        validate_mod(recipe, here, op).map_err(bad)?;
    }
    validate_closing(recipe).map_err(bad)?;
    Ok(())
}

/// A modifier may only qualify an op that can honour it.
///
/// This is not tidiness. A count expression on a `DecodePointer` or a
/// comparison mask on a `Freeze` would be silently ignored, and a recipe author
/// would then believe a bound was in force that nothing applies — which is the
/// shape of every unbounded-copy defect this vocabulary exists to prevent.
fn validate_mod(recipe: &Recipe, here: u16, op: &RecipeOp) -> Result<(), MalformedReason> {
    if recipe.mods.is_empty() {
        return Ok(());
    }
    let modifier = &recipe.mods[here as usize];
    // A `DecodeExtent` is a decode op that nevertheless CAUSES a copy — the
    // whole element array, as one transaction — so it is the one decode op that
    // may carry a copy's qualifications.
    let copies = op.kind() == OpKind::Copy || matches!(op, RecipeOp::DecodeExtent { .. });
    let is_copy = op.kind() == OpKind::Copy;
    let wrong = MalformedReason::ModOnWrongOp { op: here };

    if modifier.count.is_some() && !matches!(op, RecipeOp::DecodeExtent { .. }) {
        return Err(wrong);
    }
    if modifier.admit.is_some()
        && !matches!(
            op,
            RecipeOp::DecodePointer { .. } | RecipeOp::DecodeExtent { .. }
        )
    {
        return Err(wrong);
    }
    if modifier.carry.is_some() && is_copy {
        return Err(wrong);
    }
    if modifier.key_from_carry && !matches!(op, RecipeOp::JoinBounded { .. }) {
        return Err(wrong);
    }
    if modifier.instance_from_carry
        && !matches!(
            op,
            RecipeOp::DecodeExtent { .. } | RecipeOp::JoinBounded { .. }
        )
    {
        return Err(wrong);
    }
    if !copies
        && (modifier.samples != SampleSpec::default()
            || modifier.tolerance != Tolerance::default()
            || !modifier.compare.windows.is_empty())
    {
        return Err(wrong);
    }

    if let Some(expression) = &modifier.count {
        if expression.terms.len() > super::MAX_COUNT_EXPR_TERMS {
            return Err(MalformedReason::CountExpressionTooManyTerms {
                op: here,
                terms: expression.terms.len() as u32,
            });
        }
        if expression.terms.is_empty() && expression.constant == 0 {
            return Err(MalformedReason::CountExpressionEmpty { op: here });
        }
        for term in &expression.terms {
            if !matches!(term.width, 1 | 2 | 4 | 8) {
                return Err(MalformedReason::BadWidth {
                    op: here,
                    width: term.width,
                });
            }
        }
    }
    if let Some(window) = &modifier.admit {
        if !window.well_formed() {
            return Err(MalformedReason::BadAdmissibilityWindow { op: here });
        }
    }
    if let Some(CarrySource::Field { width, .. }) = modifier.carry {
        if !matches!(width, 1 | 2 | 4 | 8) {
            return Err(MalformedReason::BadWidth { op: here, width });
        }
    }
    if modifier.samples.count == 0 || modifier.samples.count > super::MAX_ADJACENT_SAMPLES {
        return Err(MalformedReason::SampleCountOutOfRange {
            op: here,
            count: modifier.samples.count,
        });
    }
    if !modifier.compare.windows.is_empty() {
        if modifier.compare.windows.len() > super::MAX_COMPARE_WINDOWS {
            return Err(MalformedReason::BadCompareMask { op: here });
        }
        // A mask window must lie inside the object its op copies. One that
        // reached outside would narrow the closing comparison to bytes the
        // interval does not hold — i.e. compare nothing and certify anyway.
        let inside = match op {
            RecipeOp::CopyHeader { size, .. } => modifier.compare.windows.iter().all(|window| {
                window.offset >= 0
                    && window.len > 0
                    && (window.offset as u64)
                        .checked_add(u64::from(window.len))
                        .map(|end| end <= u64::from(*size))
                        .unwrap_or(false)
            }),
            RecipeOp::CopyRanges { windows, .. } => {
                modifier.compare.windows.iter().all(|mask| {
                    mask.len > 0
                        && windows.iter().any(|window| {
                            mask.offset >= window.offset
                                && i128::from(mask.offset) + i128::from(mask.len)
                                    <= i128::from(window.offset) + i128::from(window.len)
                        })
                })
            }
            // An extent's mask is per ELEMENT, so its windows are measured
            // against the stride rather than against the whole array.
            RecipeOp::DecodeExtent { stride, .. } => {
                modifier.compare.windows.iter().all(|mask| {
                    mask.offset >= 0
                        && mask.len > 0
                        && (mask.offset as u64 + u64::from(mask.len)) <= u64::from(*stride)
                })
            }
            _ => false,
        };
        if !inside {
            return Err(MalformedReason::BadCompareMask { op: here });
        }
    }
    Ok(())
}

/// A closing program may only order stages its own ops carry, and it may only
/// elide FORWARDS.
///
/// Backwards elision is rejected rather than normalised: "close this stage
/// where a later one covers it" is a claim about which read happens LAST, and a
/// program that could elide into an earlier stage would be claiming a witness
/// taken before the bytes it certifies.
fn validate_closing(recipe: &Recipe) -> Result<(), MalformedReason> {
    let Some(program) = &recipe.closing else {
        return Ok(());
    };
    if program.stages.len() > super::MAX_CLOSING_STAGES {
        return Err(MalformedReason::BadClosingProgram {
            stage: program.stages.len() as u16,
        });
    }
    for (position, stage) in program.stages.iter().enumerate() {
        if stage.stage == 0 {
            // Stage 0 is "unstaged", which is the implicit last group.
            return Err(MalformedReason::BadClosingProgram { stage: 0 });
        }
        if program.stages[..position]
            .iter()
            .any(|earlier| earlier.stage == stage.stage)
        {
            return Err(MalformedReason::BadClosingProgram { stage: stage.stage });
        }
        if !recipe.mods.iter().any(|op| op.stage == stage.stage) {
            return Err(MalformedReason::BadClosingProgram { stage: stage.stage });
        }
        if let Some(target) = stage.elide_into {
            let later = program.stages[position + 1..]
                .iter()
                .any(|candidate| candidate.stage == target);
            if !later {
                return Err(MalformedReason::BadClosingProgram { stage: stage.stage });
            }
        }
    }
    Ok(())
}

/// How a recipe ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecipeOutcome {
    /// Every branch reached a `Freeze` within every limit.
    Captured,
    /// A named constraint stopped it. The consumer is gapped with this reason,
    /// and the record carries the witness the refusal happened under.
    Refused(RefusalRecord),
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
    /// RULING A. Spans a `Tolerance::Scoped` copy could not read. The
    /// recipe still captured; these bytes are absent and named, and only a read
    /// that touches one refuses. A non-zero count here is a DEGRADED capture
    /// that publishes, which is the whole point of the ruling — so it is on the
    /// report rather than only in the void list.
    pub scoped_voids: u32,
    /// Pointers this recipe decoded, rejected on admissibility, and did not
    /// follow. Branch outcomes, not recipe outcomes.
    pub pointers_rejected: u32,
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
    /// The order the closing wave actually ran in, when a recipe DECLARED one.
    /// RULING B makes closure order a property a recipe states, so it is
    /// reported rather than implied: a program whose stages ran out of order is
    /// a defect nothing else would show.
    ///
    /// EMPTY when no recipe declared a closing program, which means enrolment
    /// order — interval-id order, the single generic wave. It is empty rather
    /// than filled with that default because the default is already knowable
    /// from the witnessed set, and materialising it would be a work-scaled
    /// allocation for every observation that did not ask a question.
    pub closing_order: Vec<IntervalId>,
    /// Closes skipped because a later stage covered the same interval, with the
    /// attribution the elided stage keeps.
    pub elisions: Vec<Elision>,
}

/// One close that did not happen because a later stage covers the same bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Elision {
    pub interval: IntervalId,
    /// The stage whose close was skipped. A failure of the covering close is
    /// reported against this stage as well — that is what "reports in the
    /// elided site's clothes" means.
    pub elided: u16,
    /// The stage that covers it.
    pub into: u16,
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

/// The joined qualifications of one planned copy. See
/// [`Observation::copy_attributes`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CopyAttributes {
    samples: SampleSpec,
    tolerance: Tolerance,
    stage: u16,
    op: u16,
}

impl Default for CopyAttributes {
    fn default() -> Self {
        Self {
            samples: SampleSpec::default(),
            tolerance: Tolerance::Required,
            stage: 0,
            op: 0,
        }
    }
}

/// Where a comparison mask's windows are measured from.
#[derive(Clone, Copy, Debug)]
enum MaskSource {
    /// An op acting at one address: windows are relative to it.
    At(u64),
    /// A decoded extent: windows are relative to EACH element.
    Strided(u64, u32, u32),
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
    /// The branch's carried scalar. See [`Carry`] for why there is one.
    carry: CarrySlot,
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
    /// The branch's carried scalar, propagated from the task that produced the
    /// site and possibly reset by a `carry` modifier at the site itself.
    carry: CarrySlot,
    /// Element index, when the site is one element of a decoded extent. Carried
    /// for the witness: d81's first failure names the index it failed at, and
    /// an extent's elements are otherwise indistinguishable in a refusal.
    index: u32,
}

#[derive(Clone, Debug)]
struct RecipeState {
    copy_tasks: u32,
    planned_ranges: u32,
    planned_bytes: u64,
    frozen_branches: u32,
    witness_required: bool,
    scoped_voids: u32,
    pointers_rejected: u32,
    refusal: Option<RefusalRecord>,
}

impl RecipeState {
    fn new() -> Self {
        Self {
            copy_tasks: 0,
            planned_ranges: 0,
            planned_bytes: 0,
            frozen_branches: 0,
            witness_required: false,
            scoped_voids: 0,
            pointers_rejected: 0,
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
    /// Registered join tables, keyed by `(recipe, table, INSTANCE)`. The
    /// instance is the branch's carried scalar where the registering op asked
    /// for it and zero otherwise, so repeated per-country or per-province
    /// instances of one logical table stay independently addressable instead of
    /// overwriting each other.
    tables: BTreeMap<(usize, TableId, u64), TableBinding>,
    /// Per-interval consumption, samples and stage tags. Untouched, and never
    /// allocated into, by an observation whose recipes declare no
    /// qualifications.
    annot: super::annot::Annotations,
    /// RULING A. Spans a speculative copy could not read.
    voids: Vec<super::Void>,
    /// Pointers decoded, rejected on admissibility, and not followed.
    rejections: Vec<super::PointerRejection>,
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
            annot: {
                let mut annot = super::annot::Annotations::new();
                if recipes.iter().any(Recipe::needs_annotations) {
                    annot.arm();
                }
                annot
            },
            voids: Vec::new(),
            rejections: Vec::new(),
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
    /// `by_recipe` holds one group per recipe and its groups' capacities are
    /// counted below.
    ///
    /// `tables` USED to be excluded on the grounds that it is bounded by the
    /// registered join tables of the recipes in the observation and so does not
    /// scale with the work. That stopped being true when table bindings became
    /// instance-scoped: the key is `(recipe, table, instance)` and the instance
    /// is a decoded scalar, so a recipe that registers one table per province
    /// has one entry per province. It is modelled below rather than measured,
    /// for the reason [`super::annot::Annotations::capacity_bytes`] gives: a
    /// `BTreeMap`'s allocation is not observable from outside it.
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
        // The annotation tables reconcile on the SAME schedule and for the same
        // reason: they grow one insert deep inside the dependency attach, once
        // per (request, interval), so they cannot admit their own growth. Doing
        // it here rather than once at the end is what keeps them inside the
        // ceiling — a recipe that masks a hundred thousand intervals would
        // otherwise hold work-scaled memory the ledger had never seen until the
        // observation was already over.
        if self.annot.armed() {
            let ledger = self.arena.ledger();
            self.annot.reconcile(&ledger)?;
        }
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
        // One modelled node per binding: the key is three words and the value
        // four, plus a `BTreeMap`'s own per-entry overhead. Deliberately
        // generous — an under-model here is custody the ledger never sees.
        const TABLE_BINDING_BYTES: u64 = 96;
        let mut bytes = self.tables.len() as u64 * TABLE_BINDING_BYTES
            + in_flight
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

    /// Refuse with whatever witness the site could give. The FIRST refusal
    /// wins, as it always has: a recipe's outcome is the constraint that
    /// stopped it, not the last one it noticed.
    fn refuse_witnessed(&mut self, recipe: usize, refusal: Refusal, witness: RefusalWitness) {
        if self.states[recipe].refusal.is_none() {
            self.states[recipe].refusal = Some(RefusalRecord::new(refusal, witness));
        }
    }

    /// Refuse from a place that knows only the reason. Kept distinct from
    /// [`Self::refuse_witnessed`] so an empty witness is a deliberate statement
    /// at the call site rather than a default someone forgot to fill.
    fn refuse(&mut self, recipe: usize, refusal: Refusal) {
        self.refuse_witnessed(recipe, refusal, RefusalWitness::default());
    }

    /// The witness a decode site can always produce.
    fn witness_at(&self, recipe: usize, site: &DecodeSite) -> RefusalWitness {
        RefusalWitness {
            stage: self.recipes[recipe].op_mod(site.op).stage,
            op: site.op,
            subject: site.base,
            related: site.region.region().map(|span| span.start).unwrap_or(0),
            index: site.index,
            ..RefusalWitness::default()
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
                // Structural, and found before any memory is touched, so there
                // is nothing to witness beyond the reason itself.
                self.states[index].refusal = Some(RefusalRecord::bare(refusal));
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
                    carry: CarrySlot::default(),
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
        let closing = self.closing_wave()?;
        let (witnessed, unreadable_closes, closing_order, elisions) = closing;

        let phases_run = self.forward_waves_run + u32::from(witnessed > 0);
        let reports = self.build_reports();
        let gapped = reports
            .iter()
            .filter_map(|report| match report.outcome {
                RecipeOutcome::Refused(record) => {
                    Some((report.consumer, GapReason::Refused(record)))
                }
                RecipeOutcome::Captured => None,
            })
            .collect::<BTreeMap<ConsumerId, GapReason>>();

        // The ledger outlives the arena: `into_bytes` consumes the arena, and
        // the page index the freeze builds still has to be charged.
        let ledger = self.arena.ledger();
        self.annot.reconcile(&ledger)?;
        let (records, links) = self.owners.into_frozen()?;
        let snapshot = Snapshot::freeze(
            self.config.hour,
            self.arena.into_bytes(),
            records,
            links,
            gapped,
            self.annot,
            self.voids,
            self.rejections,
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
            closing_order,
            elisions,
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
                let attributes = self.copy_attributes(recipe, &planned, index, tasks);
                match self.acquire(&mut cursor, span, &mut acquisition, recipe, attributes) {
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
                // The annotations hang off the intervals a request covers, so
                // this is the one pass that already knows both. Skipped whole
                // when no recipe in the observation declares any: the branch is
                // one predictable test per request.
                if self.annot.armed() {
                    self.annotate(recipe, &tasks[request.task as usize], request.span, &owned);
                    self.reconcile_samples(recipe, &tasks[request.task as usize], &owned);
                }
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

    /// What the ops behind one planned copy asked of it.
    ///
    /// A coalesced transaction can serve requests from several ops, so the
    /// attributes are JOINED, and every join is towards the stricter side:
    /// `Required` beats `Scoped` (one op that needs the bytes makes the
    /// whole transaction's failure fatal), `Gap` beats `Record`, and the sample
    /// count is the maximum anyone asked for. A merge can therefore make a copy
    /// stricter than one of its contributors asked, never looser.
    #[inline]
    fn copy_attributes(
        &self,
        recipe: usize,
        planned: &super::policy::CopyPlan,
        index: usize,
        tasks: &[CopyTask],
    ) -> CopyAttributes {
        let mut attributes = CopyAttributes::default();
        if self.recipes[recipe].mods.is_empty() {
            return attributes;
        }
        attributes.tolerance = Tolerance::Scoped;
        // A JOIN NEEDS THE RIGHT IDENTITY, and this one is `Record`. The join
        // below only ever moves the policy towards `Gap`, so starting from
        // `SampleSpec::default()` — which is `Gap` — made every copy's policy
        // `Gap` whatever its ops declared, and a recipe that elected to resolve
        // a disagreement itself was gapped by one anyway.
        attributes.samples.on_disagree = DisagreeAction::Record;
        let copy = planned.copies[index];
        for &task in planned.sources_of(&copy) {
            let op = match tasks[task as usize].body {
                TaskBody::Op { op, .. } | TaskBody::Extent { op, .. } => op,
            };
            let modifier = self.recipes[recipe].op_mod(op);
            attributes.samples.count = attributes.samples.count.max(modifier.samples.count);
            if modifier.samples.on_disagree == DisagreeAction::Gap {
                attributes.samples.on_disagree = DisagreeAction::Gap;
            }
            if modifier.tolerance == Tolerance::Required {
                attributes.tolerance = Tolerance::Required;
            }
            attributes.op = op;
            attributes.stage = modifier.stage;
        }
        attributes
    }

    /// Copy `span`, skipping whatever is already owned.
    ///
    /// `Ok(Err(hole))` is a memory refusal at that sub-span; `Err(_)` is a
    /// custody refusal, which is not attributable to one recipe.
    ///
    /// # Two amendments live here
    ///
    /// RULING A: a hole a `Tolerance::Scoped` copy cannot read is recorded
    /// as a [`super::Void`] and the loop moves on to the next hole, so the rest
    /// of an over-read copy still lands. Nothing about the readable bytes is
    /// changed by the void; what changes is that the recipe is not refused for
    /// bytes no consumer may ever reach.
    ///
    /// The A/B PAIR: a hole whose op asked for more than one sample is copied
    /// again immediately, into its own arena slot, before the next hole is even
    /// planned. That adjacency is the whole instrument — a second image taken a
    /// wave later measures a different thing — so the extra copies are here and
    /// not anywhere that could be reordered away from them.
    fn acquire(
        &mut self,
        cursor: &mut super::dedup::Cursor,
        span: Span,
        plan: &mut super::dedup::AcquisitionPlan,
        recipe: usize,
        attributes: CopyAttributes,
    ) -> Result<Result<(), Span>, CustodyRefusal> {
        self.owners.sweep_plan(cursor, span, plan);
        // `plan` is a buffer the OBSERVATION owns and the caller has already
        // taken out of `self`, so iterating it while `self` is mutated is not a
        // conflicting borrow — and it must stay that way. Copying the holes out
        // to satisfy a borrow that is not there costs a heap allocation PER COPY
        // TRANSACTION, which measured as 17% on the substrate's own 100k-range
        // bench: the whole vocabulary's per-transaction budget, spent on nothing.
        // Nothing below re-enters this function, so nothing rewrites `plan`.
        for hole in plan.holes.iter().copied() {
            match self.take_hole(hole, recipe, attributes)? {
                Some(()) => {}
                None => {
                    if attributes.tolerance == Tolerance::Required {
                        return Ok(Err(hole));
                    }
                    self.narrow_and_void(hole, span, recipe, attributes)?;
                }
            }
        }
        Ok(Ok(()))
    }

    /// One bulk copy: allocate, copy, own it, and take its adjacent samples.
    /// `None` means the engine refused the span.
    #[inline]
    fn take_hole(
        &mut self,
        hole: Span,
        recipe: usize,
        attributes: CopyAttributes,
    ) -> Result<Option<()>, CustodyRefusal> {
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
            // The arena bytes stay allocated and stay accounted. Custody is
            // what the observation HELD, not what it ended up using; pretending
            // a failed copy costs nothing would let a recipe that refuses on
            // every span run past the budget.
            return Ok(None);
        }
        let id = self.owners.insert(hole, offset)?;
        if attributes.samples.count > 1 {
            self.take_adjacent_samples(id, hole, len, attributes)?;
        }
        Ok(Some(()))
    }

    /// RULING A, and the part of it no flag alone can deliver: find out WHICH
    /// bytes of a refused scoped copy are unreadable.
    ///
    /// [`super::SnapshotMemory`] is all-or-nothing by design — a partial copy is
    /// not expressible, which is the copy-only architecture in the type system —
    /// so a 400 KB table with one bad page comes back as one refusal covering
    /// the whole table. Voiding all of it would gap every row of the table, and
    /// "refuses only rows whose consumed bytes intersect it" would be a
    /// statement about a set that is always everything.
    ///
    /// So a refused scoped span is BISECTED and retried. Each half that copies
    /// is owned normally; each half that refuses is bisected again, down to
    /// [`super::SCOPED_VOID_GRANULARITY_BYTES`], at which point it becomes a
    /// void. The split point is page-aligned where it can be, so the descent
    /// converges on the unreadable page rather than on an arbitrary midpoint.
    ///
    /// WHAT IT COSTS, stated rather than hidden. Zero on the readable path: a
    /// copy that succeeds is never bisected, and the overwhelming majority of
    /// copies succeed. On the failure path it is `O(k · log(n / granularity))`
    /// extra transactions for `k` unreadable regions — about fourteen for one
    /// bad page in a 64 MiB extent — paid once, on a frame that was going to be
    /// degraded either way.
    fn narrow_and_void(
        &mut self,
        hole: Span,
        request: Span,
        recipe: usize,
        attributes: CopyAttributes,
    ) -> Result<(), CustodyRefusal> {
        if hole.len <= super::SCOPED_VOID_GRANULARITY_BYTES {
            let witness = RefusalWitness {
                stage: attributes.stage,
                op: attributes.op,
                subject: hole.start,
                related: request.start,
                raw: hole.len,
                read_addr: hole.start,
                attempt: 1,
                ..RefusalWitness::default()
            };
            super::arena::charged_push(
                &mut self.voids,
                self.arena.ledger_ref(),
                super::Void {
                    span: hole,
                    recipe: self.recipes[recipe].id,
                    witness,
                },
            )?;
            self.states[recipe].scoped_voids = self.states[recipe].scoped_voids.saturating_add(1);
            return Ok(());
        }
        let granularity = super::SCOPED_VOID_GRANULARITY_BYTES;
        // CHECKED, and in `u128`. A span whose exclusive end is past the top of
        // the address space is representable as a `Span` — `end_excl` is a
        // `u128` for exactly that reason — but its midpoint is not a `u64` sum.
        // Wrapping it splits `[2^64 - 0x1000, +0x2000)` into a tail page and a
        // page at address ZERO, and the second half then copies and OWNS
        // unrelated bytes at the bottom of the address space. There is no
        // partial-result path here: an unrepresentable request refuses.
        let end = hole.end_excl();
        let midpoint = u128::from(hole.start) + u128::from(hole.len) / 2;
        let aligned = midpoint - (midpoint % u128::from(granularity));
        let split = if aligned > u128::from(hole.start) && aligned < end {
            aligned
        } else {
            midpoint
        };
        let (Ok(split), Ok(_end)) = (u64::try_from(split), u64::try_from(end)) else {
            self.refuse_unrepresentable(recipe, hole.start, hole.len as i64);
            return Ok(());
        };
        for part in [
            Span::new(hole.start, split - hole.start),
            Span::new(split, (end - u128::from(split)) as u64),
        ] {
            if part.is_empty() {
                continue;
            }
            if self.take_hole(part, recipe, attributes)?.is_none() {
                self.narrow_and_void(part, request, recipe, attributes)?;
            }
        }
        Ok(())
    }

    /// Copy `hole` again, back to back, once per extra sample.
    fn take_adjacent_samples(
        &mut self,
        id: IntervalId,
        hole: Span,
        len: usize,
        attributes: CopyAttributes,
    ) -> Result<(), CustodyRefusal> {
        // NO ASK IS RECORDED HERE, and that is the point. `attributes` is a
        // JOIN over every op whose request this coalesced copy serves — the
        // maximum count and the strictest policy — which is exactly right for
        // deciding HOW MANY IMAGES TO TAKE and exactly wrong as a request.
        // Recording it as one would invent a request nobody made: a recipe
        // whose two ops ask for `(2, Gap)` and `(3, Record)` would be credited
        // with `(3, Gap)` and gapped for a third image only the non-gapping op
        // ever wanted.
        //
        // The asks are recorded per REQUEST, from each op's own declared spec,
        // by `reconcile_samples` in the dependency attach — which runs for the
        // request that created this interval too.
        let mut images = super::annot::SampleImages {
            extra: Vec::new(),
            success_mask: 1,
            asks: Vec::new(),
        };
        for sample in 1..attributes.samples.count {
            let offset = self.arena.alloc_zeroed(hole.len)?;
            let memory = self.memory;
            if memory
                .copy_into(hole.start, self.arena.slice_mut(offset, len))
                .is_err()
            {
                // A sample that could not be taken is a FAILED sample, not an
                // absent one: it certifies nothing, and the comparison must not
                // read it as agreement.
                // The images this interval HOLDS is `1 + extra.len()`, so a
                // sample that could not be taken simply is not there — and
                // every consumer that asked for more than were taken is short,
                // which is what `SampleImages::unmet` reads. There is no
                // interval-wide "incomplete" flag any more: shortness is a
                // question about one consumer's request, not about the
                // interval.
                break;
            }
            images.success_mask |= 1 << sample;
            images.extra.push(
                u32::try_from(offset).expect("an arena offset fits a u32 under the custody ceiling"),
            );
        }
        self.annot.set_samples(id, images);
        Ok(())
    }

    /// Make sure an interval a request reaches actually HAS the images that
    /// request asked for — and say so when it cannot.
    ///
    /// # The hole this closes
    ///
    /// Adjacent samples are taken while a HOLE is being filled. Deduplication
    /// means a recipe can reach an interval another recipe already owns, in
    /// which case there is no hole and no second copy is taken. If that second
    /// recipe asked for an A/B pair, it would get one image, no
    /// [`super::annot::SampleImages`] record at all, and therefore no
    /// disagreement — a consumer that asked for torn detection being told the
    /// bytes are clean because nobody looked.
    ///
    /// The moment for an adjacent sample has passed by the time this runs — a
    /// pair taken now would be a pair across the wave, which is a different
    /// instrument — so the honest outcome is not to take one. It is to record
    /// the sampling as INCOMPLETE, which the closing comparison treats as a
    /// failed pair and which gaps the consumer that asked. Fail closed.
    ///
    /// It also upgrades `gap_on_disagree`: if the first contributor was content
    /// to resolve a disagreement itself and a later one is not, the stricter
    /// side wins, exactly as [`Self::copy_attributes`] joins them.
    fn reconcile_samples(&mut self, recipe: usize, task: &CopyTask, owned: &[IntervalId]) {
        let op = match &task.body {
            TaskBody::Op { op, .. } | TaskBody::Extent { op, .. } => *op,
        };
        let spec = self.recipes[recipe].op_mod(op).samples;
        if spec.count <= 1 {
            return;
        }
        let wants_gap = spec.on_disagree == DisagreeAction::Gap;
        let consumer = self.recipes[recipe].consumer;
        // The request is RECORDED, and nothing about the interval's own state
        // is touched. Whether it was met is then a question about this
        // consumer's request against the images that exist, asked at closure —
        // which is the only place both facts are known, and the only spelling
        // under which one recipe's stricter requirement cannot leak onto its
        // neighbour's hour.
        for &id in owned {
            let mut images = match self.annot.samples(id) {
                Some(existing) => existing.clone(),
                // No record means no ADJACENT sample was ever taken — but the
                // opening image exists, or the interval would not. Its bit is
                // set, so `held()` is one and not zero.
                None => super::annot::SampleImages {
                    extra: Vec::new(),
                    success_mask: 1,
                    asks: Vec::new(),
                },
            };
            images.ask(consumer, spec.count, wants_gap);
            self.annot.set_samples(id, images);
        }
    }

    /// Record one request's consumption and stage against the intervals it
    /// covers.
    fn annotate(&mut self, recipe: usize, task: &CopyTask, span: Span, owned: &[IntervalId]) {
        let (op, mask_source) = match &task.body {
            TaskBody::Op { op, addr, .. } => (*op, MaskSource::At(*addr)),
            TaskBody::Extent { op, span, count } => {
                let stride = match &self.recipes[recipe].ops[*op as usize] {
                    RecipeOp::DecodeExtent { stride, .. } => *stride,
                    _ => 0,
                };
                (*op, MaskSource::Strided(span.start, stride, *count))
            }
        };
        let modifier = self.recipes[recipe].op_mod(op);
        let stage = modifier.stage;
        let windows = modifier.compare.windows.clone();
        for &id in owned {
            if stage != 0 {
                self.annot.tag_stage(id, stage);
            }
            if windows.is_empty() {
                // No mask means "this contributor reads all of it", and that is
                // final: see the module header on monotonicity.
                self.annot.consume_all(id);
                continue;
            }
            match mask_source {
                MaskSource::At(base) => {
                    let mut spans: Vec<Span> = Vec::with_capacity(windows.len());
                    for window in &windows {
                        if let Some(resolved) = window.resolve(base) {
                            if resolved.overlaps(&span) {
                                spans.push(resolved);
                            }
                        }
                    }
                    if spans.is_empty() {
                        // A mask that resolved to nothing INSIDE this request is
                        // not a narrower statement, it is no statement at all —
                        // and recording it would leave the interval with an
                        // empty consumed set, which the closing comparison would
                        // read as "nothing to compare, therefore unchanged".
                        // That is a witness certifying an object it never
                        // looked at. Fall back to the whole request.
                        self.annot.consume_all(id);
                    } else {
                        self.annot.consume_spans(id, &spans);
                    }
                }
                MaskSource::Strided(base, stride, count) => {
                    if stride == 0 || count == 0 {
                        self.annot.consume_all(id);
                        continue;
                    }
                    let element_windows: Vec<(u32, u32)> = windows
                        .iter()
                        .filter(|window| {
                            window.offset >= 0
                                && (window.offset as u64 + u64::from(window.len))
                                    <= u64::from(stride)
                        })
                        .map(|window| (window.offset as u32, window.len))
                        .collect();
                    if element_windows.len() != windows.len() {
                        // A window that does not fit inside one element is not
                        // a per-element mask. Rather than guess what it meant,
                        // the interval goes back to fully consumed — the safe
                        // direction.
                        self.annot.consume_all(id);
                        continue;
                    }
                    self.annot.consume_strided(
                        id,
                        super::annot::StridedMask {
                            base,
                            stride,
                            count,
                            windows: element_windows,
                        },
                    );
                }
            }
        }
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
                            carry: task.carry,
                            index: 0,
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
                            stack.push(DecodeSite {
                                op: *op,
                                base,
                                region,
                                carry: task.carry,
                                index,
                            });
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
        // The carried scalar is resolved LAZILY, inside each arm that enqueues
        // work, and not once at the top. `Freeze` and the copy arms need no
        // carry at all and they are most of the sites an observation runs: the
        // eager version cost 83% on `dispatch: decode_wave (Freeze only)` in
        // the substrate's own bench, for a value that arm discards.
        macro_rules! carry {
            () => {
                match self.resolve_carry(recipe, site) {
                    Ok(carry) => carry,
                    Err(()) => return,
                }
            };
        }
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
                        let witness = RefusalWitness {
                            read_addr: addr,
                            read_width: 8,
                            ..self.witness_at(recipe, site)
                        };
                        self.refuse_witnessed(
                            recipe,
                            Refusal::FieldNotCaptured {
                                recipe: id,
                                span: Span::new(addr, 8),
                            },
                            witness,
                        );
                        return;
                    }
                };
                if value == 0 {
                    return;
                }
                if !self.admit_pointer(recipe, site, value, addr, 8) {
                    return;
                }
                let task = CopyTask {
                    recipe,
                    depth: depth + 1,
                    parent: Some(holder),
                    carry: CarrySlot::from(carry!()),
                    body: TaskBody::Op {
                        op: next,
                        addr: value,
                        anchor: TaskAnchor::Pointer,
                        inherited: Proof::None,
                    },
                };
                self.enqueue(task);
            }
            RecipeOp::SelectByValue {
                probe,
                then,
                otherwise,
            } => {
                let (probe, then, otherwise) = (*probe, *then, *otherwise);
                let taken = match self.evaluate_probe(recipe, site, &probe) {
                    Some(true) => Some(then),
                    Some(false) => otherwise,
                    None => {
                        let witness = self.witness_at(recipe, site);
                        self.refuse_witnessed(
                            recipe,
                            Refusal::ConditionalUnresolved {
                                recipe: id,
                                op: site.op,
                            },
                            witness,
                        );
                        return;
                    }
                };
                let Some(next) = taken else {
                    let witness = self.witness_at(recipe, site);
                    self.refuse_witnessed(
                        recipe,
                        Refusal::ConditionalUnresolved {
                            recipe: id,
                            op: site.op,
                        },
                        witness,
                    );
                    return;
                };
                let parent = self.owners.find(site.base);
                let task = CopyTask {
                    recipe,
                    depth: depth + 1,
                    parent,
                    carry: CarrySlot::from(carry!()),
                    body: TaskBody::Op {
                        op: next,
                        addr: site.base,
                        // The selected copy stands where the probed object
                        // stands, so it inherits that object's proof and its
                        // anchor. It is not a new dereference and must not be
                        // priced as one.
                        anchor: TaskAnchor::Pointer,
                        inherited: site.region,
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
                let ptr_addr = match site.base.checked_add(u64::from(ptr_offset)) {
                    Some(addr) => addr,
                    None => {
                        return self
                            .refuse_unrepresentable(recipe, site.base, i64::from(ptr_offset))
                    }
                };
                // M1. A derived count reads several header fields and adds a
                // constant; the ordinary form reads one. Both are length
                // decoding out of bytes the recipe has already copied.
                let raw_count = match self.recipes[recipe].op_mod(site.op).count.clone() {
                    Some(expression) => match self.evaluate_count(recipe, site, &expression) {
                        Ok(count) => count,
                        Err(()) => return,
                    },
                    None => {
                        let count_addr = match site.base.checked_add(u64::from(count_offset)) {
                            Some(addr) => addr,
                            None => {
                                return self.refuse_unrepresentable(
                                    recipe,
                                    site.base,
                                    i64::from(count_offset),
                                )
                            }
                        };
                        match self.read_uint(count_addr, count_width as usize) {
                            Some((value, _)) => value,
                            None => {
                                let witness = RefusalWitness {
                                    read_addr: count_addr,
                                    read_width: count_width,
                                    ..self.witness_at(recipe, site)
                                };
                                self.refuse_witnessed(
                                    recipe,
                                    Refusal::FieldNotCaptured {
                                        recipe: id,
                                        span: Span::new(count_addr, u64::from(count_width)),
                                    },
                                    witness,
                                );
                                return;
                            }
                        }
                    }
                };
                let (base, base_holder) = match self.read_uint(ptr_addr, 8) {
                    Some(read) => read,
                    None => {
                        let witness = RefusalWitness {
                            read_addr: ptr_addr,
                            read_width: 8,
                            ..self.witness_at(recipe, site)
                        };
                        self.refuse_witnessed(
                            recipe,
                            Refusal::FieldNotCaptured {
                                recipe: id,
                                span: Span::new(ptr_addr, 8),
                            },
                            witness,
                        );
                        return;
                    }
                };
                if base != 0 && !self.admit_pointer(recipe, site, base, ptr_addr, 8) {
                    return;
                }
                // The TRAILER is part of the request, so it is part of what the
                // limits bound. Adding it after the check would let a recipe
                // copy `max_count + trailer` elements out of a limit that says
                // `max_count`.
                let wanted_count = raw_count.saturating_add(u64::from(trailer_elements));
                if wanted_count > u64::from(max_count) {
                    let witness = RefusalWitness {
                        raw: wanted_count,
                        related: base,
                        ..self.witness_at(recipe, site)
                    };
                    self.refuse_witnessed(
                        recipe,
                        Refusal::ExtentCountExceeded {
                            recipe: id,
                            limit: max_count,
                            // Saturating only for a count so absurd it is
                            // already far past any limit worth reporting
                            // exactly.
                            decoded: u32::try_from(wanted_count).unwrap_or(u32::MAX),
                        },
                        witness,
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
                // M2. A table registered under the branch's carried scalar is a
                // per-instance binding: the same logical table decoded once per
                // country or per province stays independently addressable
                // instead of the second registration overwriting the first.
                let carry = carry!();
                let instance = if self.recipes[recipe].op_mod(site.op).instance_from_carry {
                    match carry {
                        Some(value) => value,
                        None => {
                            let witness = self.witness_at(recipe, site);
                            self.refuse_witnessed(
                                recipe,
                                Refusal::CarryNotSet {
                                    recipe: id,
                                    op: site.op,
                                },
                                witness,
                            );
                            return;
                        }
                    }
                } else {
                    0
                };
                if base == 0 || count == 0 {
                    if let Some(table) = register {
                        self.tables.insert(
                            (recipe, table, instance),
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
                        (recipe, table, instance),
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
                    carry: CarrySlot::from(carry),
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
                let modifier = self.recipes[recipe].op_mod(site.op);
                let (key_from_carry, instance_from_carry) =
                    (modifier.key_from_carry, modifier.instance_from_carry);
                // M2. The key may be the scalar the branch is carrying rather
                // than a field of the object under the site. A province id
                // decoded at the topology can therefore select into a table two
                // waves later, which is the case `JoinBounded` could not
                // express at all.
                let carry = carry!();
                let (key, key_holder) = if key_from_carry {
                    let Some(value) = carry else {
                        let witness = self.witness_at(recipe, site);
                        self.refuse_witnessed(
                            recipe,
                            Refusal::CarryNotSet {
                                recipe: id,
                                op: site.op,
                            },
                            witness,
                        );
                        return;
                    };
                    // The carried scalar was decoded from bytes some interval
                    // holds, but not necessarily one under this site, so the
                    // dependency edge is anchored on the site's own interval.
                    match self.owners.find(site.base) {
                        Some(holder) => (value, holder),
                        None => {
                            let witness = RefusalWitness {
                                raw: value,
                                read_addr: site.base,
                                ..self.witness_at(recipe, site)
                            };
                            self.refuse_witnessed(
                                recipe,
                                Refusal::FieldNotCaptured {
                                    recipe: id,
                                    span: Span::new(site.base, 1),
                                },
                                witness,
                            );
                            return;
                        }
                    }
                } else {
                    let key_addr = match site.base.checked_add(u64::from(key_offset)) {
                        Some(addr) => addr,
                        None => {
                            return self
                                .refuse_unrepresentable(recipe, site.base, i64::from(key_offset))
                        }
                    };
                    match self.read_uint(key_addr, key_width as usize) {
                        Some(read) => read,
                        None => {
                            let witness = RefusalWitness {
                                read_addr: key_addr,
                                read_width: key_width,
                                ..self.witness_at(recipe, site)
                            };
                            self.refuse_witnessed(
                                recipe,
                                Refusal::FieldNotCaptured {
                                    recipe: id,
                                    span: Span::new(key_addr, u64::from(key_width)),
                                },
                                witness,
                            );
                            return;
                        }
                    }
                };
                let instance = if instance_from_carry {
                    match carry {
                        Some(value) => value,
                        None => {
                            let witness = self.witness_at(recipe, site);
                            self.refuse_witnessed(
                                recipe,
                                Refusal::CarryNotSet {
                                    recipe: id,
                                    op: site.op,
                                },
                                witness,
                            );
                            return;
                        }
                    }
                } else {
                    0
                };
                let binding = match self.tables.get(&(recipe, table, instance)) {
                    Some(binding) => *binding,
                    None => {
                        let witness = RefusalWitness {
                            raw: instance,
                            ..self.witness_at(recipe, site)
                        };
                        self.refuse_witnessed(
                            recipe,
                            Refusal::JoinTableMissing { recipe: id, table },
                            witness,
                        );
                        return;
                    }
                };
                if key >= u64::from(binding.count) {
                    let witness = RefusalWitness {
                        raw: key,
                        related: binding.span.start,
                        ..self.witness_at(recipe, site)
                    };
                    self.refuse_witnessed(
                        recipe,
                        Refusal::JoinOutOfRange {
                            recipe: id,
                            table,
                            key,
                            count: binding.count,
                        },
                        witness,
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
                    carry: CarrySlot::from(carry),
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
                        carry: CarrySlot::from(carry!()),
                        index: site.index,
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

    /// The carried scalar in force at a site.
    ///
    /// `Err(())` means the recipe has been refused and the caller must stop;
    /// `Ok(None)` means nothing on this branch has set one, which is legal
    /// until something asks for it.
    #[inline]
    fn resolve_carry(&mut self, recipe: usize, site: &DecodeSite) -> Result<Carry, ()> {
        let Some(source) = self.recipes[recipe].op_mod(site.op).carry else {
            return Ok(site.carry.get());
        };
        match source {
            CarrySource::Constant(value) => Ok(Some(value)),
            CarrySource::ElementIndex => Ok(Some(u64::from(site.index))),
            CarrySource::Field { offset, width } => {
                let addr = match site.base.checked_add(u64::from(offset)) {
                    Some(addr) => addr,
                    None => {
                        self.refuse_unrepresentable(recipe, site.base, i64::from(offset));
                        return Err(());
                    }
                };
                match self.read_uint(addr, width as usize) {
                    Some((value, _)) => Ok(Some(value)),
                    None => {
                        let id = self.recipes[recipe].id;
                        let witness = RefusalWitness {
                            read_addr: addr,
                            read_width: width,
                            ..self.witness_at(recipe, site)
                        };
                        self.refuse_witnessed(
                            recipe,
                            Refusal::FieldNotCaptured {
                                recipe: id,
                                span: Span::new(addr, u64::from(width)),
                            },
                            witness,
                        );
                        Err(())
                    }
                }
            }
        }
    }

    /// M3. Check a decoded pointer against its op's admissibility window.
    ///
    /// Returns false when the branch must stop. A rejection that is recorded
    /// rather than fatal is NOT a refusal: the recipe keeps its other branches
    /// and its consumer keeps its hour, and the rejection travels to the
    /// consumer with its witness so the reader can raise its own typed failure.
    fn admit_pointer(
        &mut self,
        recipe: usize,
        site: &DecodeSite,
        value: u64,
        read_addr: u64,
        read_width: u8,
    ) -> bool {
        let Some(window) = self.recipes[recipe].op_mod(site.op).admit else {
            return true;
        };
        if window.admits(value) {
            return true;
        }
        let id = self.recipes[recipe].id;
        let witness = RefusalWitness {
            raw: value,
            read_addr,
            read_width,
            ..self.witness_at(recipe, site)
        };
        match window.on_reject {
            RejectAction::RefuseRecipe => {
                self.refuse_witnessed(
                    recipe,
                    Refusal::PointerNotAdmissible {
                        recipe: id,
                        addr: value,
                        low: window.low,
                        high: window.high,
                    },
                    witness,
                );
            }
            RejectAction::StopBranch => {
                self.states[recipe].pointers_rejected =
                    self.states[recipe].pointers_rejected.saturating_add(1);
                let consumer = self.recipes[recipe].consumer;
                // Unbounded growth is not possible: one rejection per decoded
                // pointer, and copy tasks are already capped per recipe. It is
                // still charged, because it is work-scaled and it survives into
                // the snapshot.
                if super::arena::charged_push(
                    &mut self.rejections,
                    self.arena.ledger_ref(),
                    super::PointerRejection {
                        recipe: id,
                        consumer,
                        addr: value,
                        witness,
                    },
                )
                .is_err()
                {
                    // Custody is the one global refusal and it is not
                    // attributable to this recipe, so it cannot be reported
                    // here. Refusing the recipe is the conservative
                    // alternative to dropping the record silently.
                    self.refuse_witnessed(
                        recipe,
                        Refusal::PointerNotAdmissible {
                            recipe: id,
                            addr: value,
                            low: window.low,
                            high: window.high,
                        },
                        witness,
                    );
                }
            }
        }
        false
    }

    /// M1. Sum a count expression's terms out of already-copied bytes.
    fn evaluate_count(
        &mut self,
        recipe: usize,
        site: &DecodeSite,
        expression: &CountExpr,
    ) -> Result<u64, ()> {
        let id = self.recipes[recipe].id;
        let mut total = u128::from(expression.constant);
        for term in &expression.terms {
            let addr = match site.base.checked_add(u64::from(term.offset)) {
                Some(addr) => addr,
                None => {
                    self.refuse_unrepresentable(recipe, site.base, i64::from(term.offset));
                    return Err(());
                }
            };
            let value = match self.read_uint(addr, term.width as usize) {
                Some((value, _)) => value,
                None => {
                    let witness = RefusalWitness {
                        read_addr: addr,
                        read_width: term.width,
                        ..self.witness_at(recipe, site)
                    };
                    self.refuse_witnessed(
                        recipe,
                        Refusal::FieldNotCaptured {
                            recipe: id,
                            span: Span::new(addr, u64::from(term.width)),
                        },
                        witness,
                    );
                    return Err(());
                }
            };
            total += u128::from(value);
        }
        // Summed in `u128` so a header full of `u64::MAX` cannot wrap into a
        // small, admissible count. Anything past `u64` is reported at the cap,
        // where the op's own `max_count` check refuses it.
        Ok(u64::try_from(total).unwrap_or(u64::MAX))
    }

    /// RULING C. Evaluate a bounded probe over already-copied bytes.
    ///
    /// `None` means the probe could not be answered — its bytes are not in the
    /// arena — which is a refusal at the caller rather than a silent `false`.
    fn evaluate_probe(&self, recipe: usize, site: &DecodeSite, probe: &Probe) -> Option<bool> {
        match *probe {
            Probe::FieldEquals {
                offset,
                width,
                value,
            } => {
                let addr = site.base.checked_add(u64::from(offset))?;
                let (read, _) = self.read_uint(addr, width as usize)?;
                Some(read == value)
            }
            Probe::FieldInRange {
                offset,
                width,
                low,
                high,
            } => {
                let addr = site.base.checked_add(u64::from(offset))?;
                let (read, _) = self.read_uint(addr, width as usize)?;
                Some(read >= low && read <= high)
            }
            Probe::SamplesAgree { offset, len } => {
                let addr = site.base.checked_add(u64::from(offset))?;
                let interval = self.owners.find(addr)?;
                let record = *self.owners.record(interval);
                let want = Span::new(addr, u64::from(len));
                if !record.span().contains_span(&want) {
                    return None;
                }
                let images = self.annot.samples(interval)?;
                let Some(&second) = images.extra.first() else {
                    // No second image was taken, so nothing agrees. Reported as
                    // "false" rather than "unanswerable": a probe on samples a
                    // recipe never asked for is a recipe defect, but the
                    // conservative branch — the fallback shape — is the right
                    // answer for an image that cannot be certified.
                    return Some(false);
                };
                let skew = (addr - record.start()) as usize;
                let width = len as usize;
                let opening = record.opening_offset() + skew;
                let closing = second as usize + skew;
                Some(
                    self.arena.slice(opening, width) == self.arena.slice(closing, width),
                )
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
    /// RULING B. The order the closing wave runs in, and the closes it skips.
    ///
    /// `None` — no recipe declared a closing program — is enrolment order,
    /// which is interval-id order: exactly the single generic wave, and no work
    /// is done to produce it.
    ///
    /// With a program, an interval's position is the position of the EARLIEST
    /// of its stages that the program lists; an interval carrying no listed
    /// stage sorts last. Ordering by the earliest stage rather than the latest
    /// is what makes elision meaningful: a stage that elides into a later one
    /// hands its interval to that later position, and everything else keeps the
    /// place its own stage declares.
    fn closing_schedule(
        &mut self,
    ) -> Result<(Option<Vec<IntervalId>>, Vec<Elision>), CustodyRefusal> {
        let mut elisions: Vec<Elision> = Vec::new();
        if self.recipes.iter().all(|recipe| recipe.closing.is_none()) {
            // NO program, no permutation, and no copy of the witness list. The
            // list is one entry per witnessed interval and it is built at the
            // moment custody is fullest; duplicating it to hand back an order
            // identical to the one already in hand would be work-scaled memory
            // for nothing.
            return Ok((None, elisions));
        }
        // The programs are ITERATED, not collected. One `&ClosingProgram` per
        // recipe is work-scaled — an observation may hold any number of recipes
        // — so a `Vec` of them is memory the ledger would have to be told about,
        // at the fullest moment of the observation, to answer a question two
        // closures can answer by walking the recipes they already have.
        //
        // One program per observation is not a restriction worth adding: each
        // interval's stages come from the recipes that reached it, and two
        // recipes' programs order disjoint stage sets. So the rank of a stage is
        // looked up across every program, and the first match wins.
        let recipes = self.recipes;
        let rank_of = |stage: u16| -> Option<usize> {
            recipes
                .iter()
                .filter_map(|recipe| recipe.closing.as_ref())
                .find_map(|program| program.stages.iter().position(|s| s.stage == stage))
        };
        let elide_target = |stage: u16| -> Option<u16> {
            recipes
                .iter()
                .filter_map(|recipe| recipe.closing.as_ref())
                .find_map(|program| {
                    program
                        .stages
                        .iter()
                        .find(|s| s.stage == stage)
                        .map(|entry| entry.elide_into)
                })
                .flatten()
        };
        // CHARGED BEFORE TAKEN, and through the fallible path. `to_vec` would
        // be an infallible allocation of one entry per witnessed interval at the
        // fullest moment of the observation: at the ceiling that aborts the
        // process where it should return a `CustodyRefusal`.
        let ledger = self.arena.ledger();
        let mut order: Vec<IntervalId> = Vec::new();
        super::arena::charged_reserve(&mut order, &ledger, self.owners.witnessed_ids().len())?;
        order.extend_from_slice(self.owners.witnessed_ids());

        // The elisions are recorded in ONE pass, and the order is then decided
        // by sorting `order` in place. The `keyed` vector this replaces was a
        // second work-scaled allocation whose charge was never released when it
        // was dropped, so an observation carried a phantom charge for scratch it
        // no longer held. Recomputing a rank per comparison is a walk over one
        // interval's stage list — single digits — against an allocation the
        // ceiling has to be told about.
        let annot = &self.annot;
        let effective_rank = |id: IntervalId| -> usize {
            let stages = annot.stages_of(id);
            let mut best = usize::MAX;
            for &stage in stages {
                let effective = match elide_target(stage) {
                    Some(target) if stages.contains(&target) => target,
                    _ => stage,
                };
                if let Some(rank) = rank_of(effective) {
                    best = best.min(rank);
                }
            }
            best
        };
        for &id in &order {
            let stages = annot.stages_of(id);
            for &stage in stages {
                // An elided stage does not get to claim its own position: its
                // close happens where the covering stage's does, and the fact
                // that it did is what carries the attribution.
                if let Some(target) = elide_target(stage) {
                    if stages.contains(&target)
                        && !elisions
                            .iter()
                            .any(|e| e.interval == id && e.elided == stage)
                    {
                        // Charged where it grows: bounded by the witnessed count
                        // times the stage count, which is not small enough to
                        // leave unadmitted at the ceiling.
                        super::arena::charged_push(
                            &mut elisions,
                            &ledger,
                            Elision {
                                interval: id,
                                elided: stage,
                                into: target,
                            },
                        )?;
                    }
                }
            }
        }
        // UNSTABLE, with the id in the key. `sort_by_key` is stable and Rust's
        // stable sort allocates auxiliary storage proportional to the slice —
        // work-scaled scratch the ledger was never told about, taken at the
        // fullest moment of the observation. `sort_unstable_by_key` sorts in
        // place, and putting the interval id in the key makes the order TOTAL,
        // so equal ranks still come out in enrolment order. The tie-break is
        // now a property of the key rather than of the algorithm, which is the
        // stronger statement of the two.
        order.sort_unstable_by_key(|id| (effective_rank(*id), id.0));
        Ok((Some(order), elisions))
    }

    /// The closing phase: re-copy the opening addresses.
    ///
    /// This does NOT re-run discovery. A closing witness re-copies what the
    /// opening wave already found; a parent that changed invalidates its
    /// dependent subtree afterwards, which is cheaper than a reverse chase and
    /// strictly more conservative.
    fn closing_wave(
        &mut self,
    ) -> Result<(u32, Vec<IntervalId>, Vec<IntervalId>, Vec<Elision>), CustodyRefusal> {
        // Enrolment order, which `enrol_witnesses` produces by walking the
        // records in id order: the same order a scan of every record produced,
        // without the scan. A closing program permutes it; see
        // `closing_schedule`.
        //
        // Walked BY POSITION rather than through a clone of the enrolment
        // list. The clone was there only to release the borrow before
        // `record_mut`, and it was a second `Vec<IntervalId>` as long as the
        // witness list — work-scaled memory the ledger never saw, taken at the
        // exact moment custody is fullest.
        let (schedule, elisions) = self.closing_schedule()?;
        let witnessed = match &schedule {
            Some(order) => order.len(),
            None => self.owners.witnessed_ids().len(),
        };
        let mut unreadable = Vec::new();
        for position in 0..witnessed {
            let id = match &schedule {
                Some(order) => order[position],
                None => self.owners.witnessed_ids()[position],
            };
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
        // EMPTY means enrolment order — see `SnapshotBuild::closing_order`. A
        // caller that wants to check a declared order is a caller that declared
        // one, and materialising the default order for everybody else would be
        // a work-scaled allocation at the moment custody is fullest.
        Ok((
            witnessed as u32,
            unreadable,
            schedule.unwrap_or_default(),
            elisions,
        ))
    }

    fn build_reports(&self) -> Vec<RecipeReport> {
        self.recipes
            .iter()
            .zip(self.states.iter())
            .map(|(recipe, state)| RecipeReport {
                recipe: recipe.id,
                consumer: recipe.consumer,
                outcome: match state.refusal {
                    Some(record) => RecipeOutcome::Refused(record),
                    None => RecipeOutcome::Captured,
                },
                copy_tasks: state.copy_tasks,
                planned_ranges: state.planned_ranges,
                planned_bytes: state.planned_bytes,
                frozen_branches: state.frozen_branches,
                witness_required: state.witness_required,
                scoped_voids: state.scoped_voids,
                pointers_rejected: state.pointers_rejected,
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
                    carry: CarrySlot::default(),
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
                    carry: CarrySlot::default(),
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
                self.observation.states[index].refusal = Some(RefusalRecord::bare(refusal));
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
