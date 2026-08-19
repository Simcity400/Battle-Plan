//! 4. **One global interval ownership map.** [`dedup::OwnershipMap`] copies a

pub mod annot;
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
#[cfg(test)]
mod vocab_tests;

pub const MEASURED_MAX_DISCOVERY_DEPTH: u32 = 12;

pub const MAX_RECIPE_DEPTH: u32 = 30;

pub const FORWARD_WAVES: u32 = MAX_RECIPE_DEPTH + 1;

pub const TOTAL_PHASES: u32 = FORWARD_WAVES + 1;

pub const DEFAULT_CUSTODY_BUDGET_BYTES: u64 = 512 * 1024 * 1024;

pub const GENERIC_GAP_BYTES: u32 = 32;

pub const PROVEN_OBJECT_GAP_BYTES: u32 = 64;

pub const DEFAULT_PROVEN_EXTENT_GAP_BYTES: u32 = 128;

pub const MAX_PROVEN_WINDOW_GAP_BYTES: u32 = 4096;

pub const MAX_COUNT_EXPR_TERMS: usize = 4;

pub const MAX_ADJACENT_SAMPLES: u8 = 3;

pub const MAX_PROBE_COMPARE_BYTES: u32 = 64;

pub const MAX_COMPARE_WINDOWS: usize = 8;

pub const MAX_CLOSING_STAGES: usize = 32;

pub const SCOPED_VOID_GRANULARITY_BYTES: u64 = 4096;

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
    #[inline]
    fn finish(&self) -> u64 {
        self.hash.rotate_left(20)
    }
}

pub type FxSet<T> = std::collections::HashSet<T, std::hash::BuildHasherDefault<FxHasher>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ConsumerId(pub u16);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RecipeId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HourStamp(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct IntervalId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TableId(pub u16);

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

    pub fn hull(&self, other: &Span) -> Span {
        let start = self.start.min(other.start);
        let end = self.end_excl().max(other.end_excl());
        Span::new(
            start,
            u64::try_from(end - u128::from(start)).unwrap_or(u64::MAX),
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub max_depth: u32,
    pub max_copy_tasks: u32,
    pub max_ranges: u32,
    pub max_bytes: u64,
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
    ModsLengthMismatch { ops: u32, mods: u32 },
    ModOnWrongOp { op: u16 },
    CountExpressionEmpty { op: u16 },
    CountExpressionTooManyTerms { op: u16, terms: u32 },
    SampleCountOutOfRange { op: u16, count: u8 },
    BadCompareMask { op: u16 },
    BadAdmissibilityWindow { op: u16 },
    ProbeTooWide { op: u16, len: u32 },
    BadClosingProgram { stage: u16 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    DepthExceeded {
        recipe: RecipeId,
        limit: u32,
        wanted_depth: u32,
    },
    CopyTaskLimitExceeded {
        recipe: RecipeId,
        limit: u32,
    },
    RangeLimitExceeded {
        recipe: RecipeId,
        limit: u32,
        wanted: u32,
    },
    ByteLimitExceeded {
        recipe: RecipeId,
        limit: u64,
        wanted: u64,
    },
    RangeTooWide {
        recipe: RecipeId,
        limit: u64,
        wanted: u64,
    },
    ExtentCountExceeded {
        recipe: RecipeId,
        limit: u32,
        decoded: u32,
    },
    ExtentBytesExceeded {
        recipe: RecipeId,
        limit: u64,
        decoded: u64,
    },
    JoinOutOfRange {
        recipe: RecipeId,
        table: TableId,
        key: u64,
        count: u32,
    },
    JoinTableMissing {
        recipe: RecipeId,
        table: TableId,
    },
    FieldNotCaptured {
        recipe: RecipeId,
        span: Span,
    },
    MemoryUnreadable {
        recipe: RecipeId,
        span: Span,
    },
    AddressNotRepresentable {
        recipe: RecipeId,
        base: u64,
        offset: i64,
    },
    PointerNotAdmissible {
        recipe: RecipeId,
        addr: u64,
        low: u64,
        high: u64,
    },
    CarryNotSet {
        recipe: RecipeId,
        op: u16,
    },
    CarryHolderUnknown {
        recipe: RecipeId,
        op: u16,
    },
    ConditionalUnresolved {
        recipe: RecipeId,
        op: u16,
    },
    ClosedIntervalReopened {
        recipe: RecipeId,
        span: Span,
        closed_after_wave: u32,
        reopened_in_wave: u32,
    },
    Malformed {
        recipe: RecipeId,
        reason: MalformedReason,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RefusalWitness {
    pub stage: u16,
    pub op: u16,
    pub subject: u64,
    pub related: u64,
    pub index: u32,
    pub raw: u64,
    pub read_addr: u64,
    pub read_width: u8,
    pub attempt: u8,
    pub success_mask: u8,
    pub sample_a: u64,
    pub sample_b: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RefusalRecord {
    pub refusal: Refusal,
    pub witness: RefusalWitness,
}

impl RefusalRecord {
    pub fn new(refusal: Refusal, witness: RefusalWitness) -> Self {
        Self { refusal, witness }
    }

    pub fn bare(refusal: Refusal) -> Self {
        Self {
            refusal,
            witness: RefusalWitness::default(),
        }
    }

    pub fn recipe(&self) -> RecipeId {
        self.refusal.recipe()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PointerRejection {
    pub recipe: RecipeId,
    pub consumer: ConsumerId,
    pub addr: u64,
    pub witness: RefusalWitness,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Void {
    pub span: Span,
    pub recipe: RecipeId,
    pub witness: RefusalWitness,
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
            | Refusal::PointerNotAdmissible { recipe, .. }
            | Refusal::CarryNotSet { recipe, .. }
            | Refusal::CarryHolderUnknown { recipe, .. }
            | Refusal::ConditionalUnresolved { recipe, .. }
            | Refusal::ClosedIntervalReopened { recipe, .. }
            | Refusal::Malformed { recipe, .. } => recipe,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CustodyRefusal {
    BudgetExhausted {
        budget: u64,
        held: u64,
        requested: u64,
    },
    AllocationFailed {
        held: u64,
        requested: u64,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemoryRefusal {
    pub span: Span,
}

pub trait SnapshotMemory {
    fn copy_into(&self, addr: u64, out: &mut [u8]) -> Result<(), MemoryRefusal>;
}

impl<T: SnapshotMemory + ?Sized> SnapshotMemory for &T {
    fn copy_into(&self, addr: u64, out: &mut [u8]) -> Result<(), MemoryRefusal> {
        (**self).copy_into(addr, out)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GapReason {
    Refused(RefusalRecord),
    WitnessChanged {
        interval: IntervalId,
    },
    WitnessDependency {
        interval: IntervalId,
        changed_parent: IntervalId,
    },
    SamplesDisagreed {
        interval: IntervalId,
    },
}
