//! Generic snapshot-recipe substrate for the capture floor (stages S4/S5).
//!
//! WHAT THIS IS. The re-priced capture floor
//! (`runs/s0_census/t35/floor_reprice_reply.txt`) rejects both of the two
//! obvious implementations of an hour observation:
//!
//!  * a literal copier over t35's 504,423 contiguous ranges (about
//!    14–32 ms dev / 32–75 ms on the 4c/8GB floor machine), and
//!  * per-domain snapshots, which pay 115.906 MiB for a 58.752 MiB union.
//!
//! What survives is a *recipe* substrate: a small program per reader family
//! that says which bytes to copy and how to get from one copied object to the
//! next, executed breadth-first, with one globally deduplicated set of copied
//! intervals behind it. This module is that substrate. It contains NO reader
//! conversion — the S4/S5 pilots run on top of it and land in later lanes.
//!
//! THE FIVE THINGS IT HAS TO GET RIGHT, and where each one lives:
//!
//! 1. **Hard limits, typed refusals.** Every [`executor::Recipe`] carries a
//!    [`Limits`] block (depth, copy-task count, range count, byte total,
//!    single-range width). Exceeding any of them produces a [`Refusal`]
//!    naming the limit and the value that broke it. There is no path that
//!    trims a result to fit and reports success. See [`executor`].
//!
//! 2. **Breadth-first by discovery depth.** The engine runs [`FORWARD_WAVES`]
//!    forward copy waves (depths `0..=`[`MAX_RECIPE_DEPTH`]) plus one
//!    closing-witness wave, i.e. [`TOTAL_PHASES`] copy-submission phases.
//!    Within a wave every ready dereference is batched; between waves only
//!    address and length decoding happens. A wave with no tasks costs an
//!    `is_empty` test, so an observation runs as many waves as its deepest
//!    chain and no more. See [`executor::Observation`].
//!
//!    The ceiling is [`MAX_RECIPE_DEPTH`] and not t35's measured
//!    [`MEASURED_MAX_DISCOVERY_DEPTH`]; that constant's docs say why the two
//!    are different questions.
//!
//! 3. **Gap coalescing is data, not code.** [`policy::GapPolicy`] is a
//!    per-recipe value: 32 B generically, 64 B inside a proven
//!    object/allocation, wider windows only where a container extent has been
//!    decoded, and never wider than [`MAX_PROVEN_WINDOW_GAP_BYTES`]. Merging
//!    on address proximity alone is not expressible: the merge entry point
//!    takes [`policy::Provenance`], not bare addresses, and two ranges with no
//!    shared provenance get an allowance of zero bytes. Per the gap-curve
//!    verdict (`runs/s0_census/t35_gap_curve/verdict.txt` §4) a window anchor
//!    is a DEREFERENCED POINTER VALUE, never a root read event — see
//!    [`policy::AnchorKind`].
//!
//! 4. **One global interval ownership map.** [`dedup::OwnershipMap`] copies a
//!    shared extent once and carries a dependency list, so a closing witness
//!    that fails gaps exactly the consumers that read those bytes. There is no
//!    global bracket: unaffected consumers keep their hour.
//!
//! 5. **Views, not addresses.** Post-bracket readers get a
//!    [`view::SnapshotView`]: immutable, hour-stamped, own-hour only,
//!    translated through a page/radix index onto the frozen arena. Nothing on
//!    that type returns a live address or a live pointer.
//!
//! CUSTODY. The arena holds at most [`DEFAULT_CUSTODY_BUDGET_BYTES`] (512 MiB,
//! the owner-amended figure from the re-pricing's §4.4). Budget exhaustion and
//! allocation failure are both [`CustodyRefusal`]s that abort the observation;
//! neither is recoverable by copying less.
//!
//! WHAT IS DELIBERATELY ABSENT. This module is not wired into the frame path.
//! It does not know about domains, hours beyond an opaque stamp, brackets, the
//! scheduler, or the wire. It compiles, tests and benches standalone.

pub mod arena;
pub mod dedup;
pub mod executor;
pub mod policy;
pub mod view;
pub mod witness;

#[cfg(any(test, feature = "test-support"))]
pub mod synthetic;

#[cfg(test)]
mod bench;
#[cfg(test)]
mod tests;

/// Deepest pointer-derivation chain t35 measured, at derivation span 0x10000.
pub const MEASURED_MAX_DISCOVERY_DEPTH: u32 = 12;

/// Deepest discovery wave a [`executor::Recipe`] may declare.
///
/// # Why this is not [`MEASURED_MAX_DISCOVERY_DEPTH`]
///
/// It was, and that was a category error rather than a conservative choice.
/// t35's 12 is a measurement of the CHAINS THE CENSUS FOLLOWED — root, vector,
/// object, field, one hop at a time. A recipe that copies a SELF-BALANCING TREE
/// pays its height on top of the chain that reached it, and that height is a
/// property of the container the engine built, not of anything t35 walked.
///
/// The first conversion (d47, characters/leaders/cabinet) is exactly that shape
/// and it does not fit under 12. Its chain is manager (0) -> the manager's
/// pointer vector (1) -> `CCharacter` body (2) -> the map's `_Myhead` sentinel
/// (3) -> the tree root through `_Myhead->_Parent` (4); then the tree's height
/// `h`; then the role object at `5 + h`, and its `CCharacter` back-pointer or
/// its heap string payload at `6 + h`. A red-black tree of `n` nodes has height
/// at most `2*log2(n + 1)`, so:
///
///  * at the widths a live game holds — single-digit entries per character, one
///    country-leader role per ideology — `h <= 6` and the deepest branch lands
///    on 12 EXACTLY, with no margin. One character holding ten roles refuses
///    the whole recipe.
///  * the reader's accepted contract is `_Mysize <= 4096`
///    (`DOMAIN47_SHADOW_WALK_MAX_NODES`), where `h <= 24` and the deepest
///    branch is 30.
///
/// A ceiling that a legal container can cross is not a bound on the recipe, it
/// is a bound on the WORLD, and this substrate refuses rather than truncates
/// (`Refusal::DepthExceeded`) — so leaving it at 12 would convert an unusual
/// but legal game state into a lost domain-hour.
///
/// # What raising it costs
///
/// One `Vec` per declared depth in [`executor::Observation`], and one
/// `is_empty` test per unused wave in the wave loop. Both are per OBSERVATION,
/// not per task: an observation whose deepest chain is 6 runs six waves and
/// skips the rest. Nothing here is a per-byte or per-object cost, which is why
/// the ceiling can be set from the deepest container the readers accept rather
/// than from the deepest one they usually meet.
///
/// It does NOT weaken any limit that bounds work. `max_copy_tasks`,
/// `max_ranges`, `max_bytes` and `max_single_range_bytes` are unchanged, and a
/// recipe still declares its own `max_depth` — this is the ceiling on what it
/// may declare, not a depth anything runs to.
pub const MAX_RECIPE_DEPTH: u32 = 30;

/// Forward breadth-first copy waves: depths 0 through [`MAX_RECIPE_DEPTH`]
/// inclusive. Roots sit at depth zero, so a depth-30 chain needs thirty-one
/// waves, not thirty.
pub const FORWARD_WAVES: u32 = MAX_RECIPE_DEPTH + 1;

/// Copy-submission phases per observation: [`FORWARD_WAVES`] plus the single
/// closing-witness wave that re-copies the opening addresses.
pub const TOTAL_PHASES: u32 = FORWARD_WAVES + 1;

/// Raw custody ceiling for one observation's arena, opening and closing images
/// together. The re-pricing raised this from 256 MiB because a 75–100 MiB
/// image, two A/B witnesses and two overlapping hours can transiently need
/// 300–400 MiB plus metadata.
pub const DEFAULT_CUSTODY_BUDGET_BYTES: u64 = 512 * 1024 * 1024;

/// Generic merge allowance: two ranges reached through the SAME dereferenced
/// pointer may bridge a hole of at most this many unread bytes.
pub const GENERIC_GAP_BYTES: u32 = 32;

/// Merge allowance inside a proven object/allocation.
pub const PROVEN_OBJECT_GAP_BYTES: u32 = 64;

/// Default merge allowance inside a decoded container extent. 128 B is the
/// smallest threshold in the measured global curve that meets both of the
/// re-pricing's rejection ceilings (151,528 ranges, 67.380 MiB).
pub const DEFAULT_PROVEN_EXTENT_GAP_BYTES: u32 = 128;

/// Hard ceiling on any configured merge allowance. The re-pricing permits
/// "128–4,096-byte windows only for a family whose allocation and maximum
/// extent are proven"; nothing may be configured past the top of that band.
pub const MAX_PROVEN_WINDOW_GAP_BYTES: u32 = 4096;

/// Hasher for the substrate's two membership sets: the discovery visited set
/// and the spilled discovery-edge set.
///
/// Both are keyed on engine addresses and small indices, never on anything an
/// outside party chooses, so the collision-attack resistance of the `SipHash`
/// default buys nothing here and its cost is real — the visited set is one
/// insert per copy task. This is the `FxHash` mixer, the same one `rustc` uses
/// for the same reason.
#[derive(Clone, Copy, Debug, Default)]
pub struct FxHasher {
    hash: u64,
}

const FX_MIX: u64 = 0x51_7c_c1_b7_27_22_0a_95;

impl FxHasher {
    #[inline]
    fn add(&mut self, word: u64) {
        self.hash = (self.hash.rotate_left(5) ^ word).wrapping_mul(FX_MIX);
    }
}

impl std::hash::Hasher for FxHasher {
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.add(u64::from(byte));
        }
    }
    #[inline]
    fn write_u8(&mut self, value: u8) {
        self.add(u64::from(value));
    }
    #[inline]
    fn write_u16(&mut self, value: u16) {
        self.add(u64::from(value));
    }
    #[inline]
    fn write_u32(&mut self, value: u32) {
        self.add(u64::from(value));
    }
    #[inline]
    fn write_u64(&mut self, value: u64) {
        self.add(value);
    }
    #[inline]
    fn write_usize(&mut self, value: usize) {
        self.add(value as u64);
    }
    /// # The finalizer is not decoration
    ///
    /// `FxHash`'s mixer is `rotate_left(5) ^ word` then a multiply, and both
    /// of those propagate entropy UPWARD: bit *n* of a product depends only on
    /// bits 0..=*n* of its inputs, so the low bits of the accumulator carry
    /// only the low bits of the keys. `hashbrown` picks its bucket with
    /// `hash as usize & bucket_mask` — exactly those low bits — so a key set
    /// whose low bits are constant lands in a fraction of the table and probes
    /// long chains. This substrate's keys are precisely that shape: engine
    /// addresses are object-aligned, so their bottom three to six bits are
    /// zero on every key, and an extent length is a multiple of its stride.
    ///
    /// One rotate folds the high half — the only part `FxHash` mixes well —
    /// down into the bucket index. It costs a single instruction and it is
    /// worth 39.9 ns per insert on the substrate's own bench
    /// (`visited set: insert, hashed`, 71.6 ns/txn before, 31.7 after).
    ///
    /// Nothing depends on the values this returns. Both sets built on it are
    /// membership-only: [`FxSet`] is a `HashSet` that is inserted into and
    /// never iterated, so changing the hash changes no observable order.
    #[inline]
    fn finish(&self) -> u64 {
        self.hash.rotate_left(20)
    }
}

/// A `HashSet` on [`FxHasher`].
pub type FxSet<T> = std::collections::HashSet<T, std::hash::BuildHasherDefault<FxHasher>>;

/// Identity of a post-bracket reader. One consumer is gapped or not gapped
/// independently of every other: there is no global bracket.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ConsumerId(pub u16);

/// Identity of a recipe within one observation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RecipeId(pub u32);

/// The hour a snapshot belongs to. Opaque here on purpose: this module must
/// not be able to reason about hour arithmetic, only about equality.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HourStamp(pub u64);

/// Identity of one owned, copied interval in the arena.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct IntervalId(pub u32);

/// Identity of a bounded join table registered by a `DecodeExtent` op.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TableId(pub u16);

/// A half-open address range `[start, start + len)`.
///
/// Exclusive ends are computed in `u128` for the same reason
/// [`crate::range_census`] does it: a range's exclusive end can be
/// `u64::MAX + 1`, and saturating that into a `u64` silently shortens the
/// range by a byte, which can let a one-byte-separated pair merge.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Span {
    pub start: u64,
    pub len: u64,
}

impl Span {
    pub const fn new(start: u64, len: u64) -> Self {
        Self { start, len }
    }

    pub const fn end_excl(&self) -> u128 {
        self.start as u128 + self.len as u128
    }

    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// True when `other` lies wholly inside `self`.
    pub fn contains_span(&self, other: &Span) -> bool {
        u128::from(other.start) >= u128::from(self.start) && other.end_excl() <= self.end_excl()
    }

    pub fn contains_addr(&self, addr: u64) -> bool {
        u128::from(addr) >= u128::from(self.start) && u128::from(addr) < self.end_excl()
    }

    pub fn overlaps(&self, other: &Span) -> bool {
        !self.is_empty()
            && !other.is_empty()
            && u128::from(self.start) < other.end_excl()
            && u128::from(other.start) < self.end_excl()
    }

    /// Unread bytes between `self` and a range starting later. Overlapping or
    /// touching ranges have a gap of zero. `None` when `later` starts before
    /// `self` does, which is a caller error the coalescer never commits
    /// (it sorts first).
    pub fn gap_to(&self, later: &Span) -> Option<u64> {
        if u128::from(later.start) < u128::from(self.start) {
            return None;
        }
        let end = self.end_excl();
        if u128::from(later.start) <= end {
            Some(0)
        } else {
            u64::try_from(u128::from(later.start) - end).ok()
        }
    }

    /// The smallest span covering both. Saturates only for a span covering the
    /// entire address space, which nothing observable can produce.
    pub fn hull(&self, other: &Span) -> Span {
        let start = self.start.min(other.start);
        let end = self.end_excl().max(other.end_excl());
        Span::new(
            start,
            u64::try_from(end - u128::from(start)).unwrap_or(u64::MAX),
        )
    }
}

/// Hard per-recipe limits. Every one of these is checked; none of them has a
/// "clamp and continue" path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Deepest discovery wave this recipe may reach. Must not exceed
    /// [`MAX_RECIPE_DEPTH`].
    pub max_depth: u32,
    /// Copy tasks this recipe may expand, over the whole observation. This is
    /// the discovery-node cap: it bounds fan-out and terminates a pointer
    /// graph whose shape the recipe author mis-modelled.
    pub max_copy_tasks: u32,
    /// Physical copy transactions this recipe may cause, counted AFTER
    /// coalescing and BEFORE global deduplication.
    pub max_ranges: u32,
    /// Bytes this recipe may cause to be copied, counted after coalescing
    /// (so bridged holes count) and before deduplication.
    pub max_bytes: u64,
    /// Widest single copy transaction.
    pub max_single_range_bytes: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_depth: MEASURED_MAX_DISCOVERY_DEPTH,
            max_copy_tasks: 1 << 20,
            max_ranges: 1 << 20,
            max_bytes: 64 * 1024 * 1024,
            max_single_range_bytes: 16 * 1024 * 1024,
        }
    }
}

/// Why a recipe is structurally invalid. Caught before any memory is touched.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MalformedReason {
    NoOps,
    NoRoots,
    OpIndexOutOfRange { op: u16 },
    RootIsNotACopyOp { op: u16 },
    ExpectedCopyOp { op: u16 },
    ExpectedDecodeOp { op: u16 },
    DanglingBranch { op: u16 },
    ZeroSizedCopy { op: u16 },
    ZeroStride { op: u16 },
    BadWidth { op: u16, width: u8 },
    DepthLimitAboveCeiling { limit: u32 },
    ZeroLimit { limit: &'static str },
}

/// Every way a recipe can fail to produce a complete result. A refusal always
/// names the recipe and the constraint; it never stands for a partial result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// Discovery wanted to enqueue work past the recipe's depth limit.
    DepthExceeded {
        recipe: RecipeId,
        limit: u32,
        wanted_depth: u32,
    },
    /// Discovery wanted to expand more copy tasks than the recipe allows.
    CopyTaskLimitExceeded { recipe: RecipeId, limit: u32 },
    /// Post-coalescing copy transactions exceeded the recipe's cap.
    RangeLimitExceeded {
        recipe: RecipeId,
        limit: u32,
        wanted: u32,
    },
    /// Post-coalescing bytes exceeded the recipe's cap.
    ByteLimitExceeded {
        recipe: RecipeId,
        limit: u64,
        wanted: u64,
    },
    /// One copy transaction was wider than the recipe allows.
    RangeTooWide {
        recipe: RecipeId,
        limit: u64,
        wanted: u64,
    },
    /// A decoded container declared more elements than the op permits.
    ExtentCountExceeded {
        recipe: RecipeId,
        limit: u32,
        decoded: u32,
    },
    /// A decoded container declared more bytes than the op permits.
    ExtentBytesExceeded {
        recipe: RecipeId,
        limit: u64,
        decoded: u64,
    },
    /// A bounded join key fell outside the registered table.
    JoinOutOfRange {
        recipe: RecipeId,
        table: TableId,
        key: u64,
        count: u32,
    },
    /// A bounded join referred to a table no `DecodeExtent` had registered.
    JoinTableMissing { recipe: RecipeId, table: TableId },
    /// A decode op tried to read a field the recipe never copied. This is a
    /// recipe defect, not an engine one, and it is fatal to the recipe: a
    /// silently-zero field is exactly truncation-as-success.
    FieldNotCaptured { recipe: RecipeId, span: Span },
    /// The engine could not copy a span the recipe asked for.
    MemoryUnreadable { recipe: RecipeId, span: Span },
    /// An address the recipe asked for does not exist: a window offset or a
    /// field offset carried its base off either end of the address space.
    ///
    /// This is a refusal rather than a skipped window on purpose. Dropping the
    /// unrepresentable part of a request and copying the rest is a partial
    /// result, and a partial result reported as success is the one outcome the
    /// limit discipline exists to prevent.
    AddressNotRepresentable {
        recipe: RecipeId,
        base: u64,
        offset: i64,
    },
    /// Structural defect, found before execution.
    Malformed {
        recipe: RecipeId,
        reason: MalformedReason,
    },
}

impl Refusal {
    pub fn recipe(&self) -> RecipeId {
        match *self {
            Refusal::DepthExceeded { recipe, .. }
            | Refusal::CopyTaskLimitExceeded { recipe, .. }
            | Refusal::RangeLimitExceeded { recipe, .. }
            | Refusal::ByteLimitExceeded { recipe, .. }
            | Refusal::RangeTooWide { recipe, .. }
            | Refusal::ExtentCountExceeded { recipe, .. }
            | Refusal::ExtentBytesExceeded { recipe, .. }
            | Refusal::JoinOutOfRange { recipe, .. }
            | Refusal::JoinTableMissing { recipe, .. }
            | Refusal::FieldNotCaptured { recipe, .. }
            | Refusal::MemoryUnreadable { recipe, .. }
            | Refusal::AddressNotRepresentable { recipe, .. }
            | Refusal::Malformed { recipe, .. } => recipe,
        }
    }
}

/// Custody failure. Not attributable to one recipe and not recoverable by
/// copying less, so it fails the whole observation closed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CustodyRefusal {
    /// The arena would have exceeded its byte budget.
    BudgetExhausted {
        budget: u64,
        held: u64,
        requested: u64,
    },
    /// The allocator refused to grow the arena.
    AllocationFailed { held: u64, requested: u64 },
}

/// The engine could not read `span` out of the observed process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemoryRefusal {
    pub span: Span,
}

/// The one way this module touches observed memory.
///
/// Bulk, whole-range, all-or-nothing: a partial copy is not expressible. That
/// is the copy-only-brackets architecture in the type system — the substrate
/// can move bytes and it cannot compute over them in flight.
pub trait SnapshotMemory {
    /// Copy `out.len()` bytes starting at `addr` into `out`, or refuse.
    ///
    /// Refusal must leave no claim on `out`: the caller discards it.
    fn copy_into(&self, addr: u64, out: &mut [u8]) -> Result<(), MemoryRefusal>;
}

impl<T: SnapshotMemory + ?Sized> SnapshotMemory for &T {
    fn copy_into(&self, addr: u64, out: &mut [u8]) -> Result<(), MemoryRefusal> {
        (**self).copy_into(addr, out)
    }
}

/// Why a consumer's hour is gapped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GapReason {
    /// Its recipe refused during the observation.
    Refused(Refusal),
    /// Bytes it read changed between the opening and closing witness copies.
    WitnessChanged { interval: IntervalId },
    /// A parent of bytes it read changed, so the pointer chain that reached
    /// them may be stale.
    WitnessDependency {
        interval: IntervalId,
        changed_parent: IntervalId,
    },
}
