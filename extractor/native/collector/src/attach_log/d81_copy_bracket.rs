use crate::snapshot_recipes::executor::{
    Admissibility, CountExpr, CountTerm, DisagreeAction, OpMod, Recipe, RecipeOp, RejectAction,
    Root, SampleSpec, Tolerance,
};
use crate::snapshot_recipes::{ConsumerId, Limits, MemoryRefusal, RecipeId, SnapshotMemory, Span};

use super::{
    D81_ACTIVE_ENTRY_STRIDE, D81_ACTIVE_ROOT_TABLE_OFF, D81_CONTAINER_PROVINCE_ARRAY_OFF,
    D81_CONTAINER_PROVINCE_COUNT_OFF, D81_GS_ACTIVE_PROVINCE_ROOT_OFF,
    D81_GS_ANCHOR_SELECTOR_MAP_OFF, D81_GS_RAIL_MANAGER_OFF, D81_IDLER_OWNER_OFF,
    D81_MAP_PROVINCE_RECORD_OFF, D81_OWNER_CONTAINER_OFF, D81_RAIL_MANAGER_TABLE_OFF,
    D81_SELECTOR_ENTRY_STRIDE, D81_SELECTOR_MAP_MASK_OFF, D81_SELECTOR_MAP_MAX_DISTANCE_OFF,
    D81_SELECTOR_MAP_TABLE_OFF, RAIL_HEADER_WINDOW_LEN, RAIL_INFO_PAIRS_PTR_OFF,
};

pub const D81_COPY_BRACKET_AUTHORITATIVE: bool = true;

pub const D81_CONSUMER: ConsumerId = ConsumerId(81);
pub const D81_RECIPE: RecipeId = RecipeId(81);

const D81_MAX_PROVINCES: u32 = super::MAX_MAP_CONTROL_PROVINCE_COUNT;

const D81_MAX_SINGLE_RANGE_BYTES: u64 = super::D81_BATCH_MAX_EXTENT_BYTES as u64;

mod op {
    pub const GAMESTATE: u16 = 0;
    pub const RAIL_MANAGER_PTR: u16 = 1;
    pub const RAIL_MANAGER: u16 = 2;
    pub const ACTIVE_ROOT_PTR: u16 = 3;
    pub const ACTIVE_ROOT: u16 = 4;
    pub const SELECTOR_MAP_PTR: u16 = 5;
    pub const SELECTOR_MAP: u16 = 6;
    pub const SELECTOR_TABLE: u16 = 7;
    pub const RAIL_TABLE: u16 = 8;
    pub const ACTIVE_TABLE: u16 = 9;
    pub const ACTIVE_ENTRY: u16 = 10;
    pub const SOURCE_ARRAY: u16 = 11;
    pub const MAP_ARRAY: u16 = 12;
    pub const IDLER_SLOT: u16 = 13;
    pub const IDLER_PTR: u16 = 14;
    pub const IDLER: u16 = 15;
    pub const OWNER_PTR: u16 = 16;
    pub const OWNER: u16 = 17;
    pub const CONTAINER_PTR: u16 = 18;
    pub const CONTAINER: u16 = 19;
    pub const IDLER_SLOT_B: u16 = 20;
    pub const FREEZE: u16 = 21;
    pub const COUNT: usize = 22;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Domain81PlanRoots {
    pub game_state: u64,
    pub idler_slot: u64,
    pub idler_slot_b: u64,
    pub container: u64,
    pub province_slots: u32,
}

fn heap_window(on_reject: RejectAction) -> Admissibility {
    Admissibility {
        low: super::HEAP_LO as u64,
        high: super::HEAP_HI as u64,
        align: 8,
        singletons: [0, 0],
        on_reject,
    }
}

pub fn domain81_copy_plan(roots: &Domain81PlanRoots) -> Recipe {
    let mut ops = vec![RecipeOp::Freeze; op::COUNT];

    ops[op::GAMESTATE as usize] = RecipeOp::CopyHeader {
        size: (D81_GS_ANCHOR_SELECTOR_MAP_OFF + 8) as u32,
        next: vec![
            op::RAIL_MANAGER_PTR,
            op::ACTIVE_ROOT_PTR,
            op::SELECTOR_MAP_PTR,
            op::SOURCE_ARRAY,
        ],
    };
    ops[op::RAIL_MANAGER_PTR as usize] = RecipeOp::DecodePointer {
        offset: D81_GS_RAIL_MANAGER_OFF as u32,
        guard: None,
        next: op::RAIL_MANAGER,
    };
    ops[op::RAIL_MANAGER as usize] = RecipeOp::CopyHeader {
        size: (D81_RAIL_MANAGER_TABLE_OFF + 8) as u32,
        next: vec![op::RAIL_TABLE],
    };
    ops[op::ACTIVE_ROOT_PTR as usize] = RecipeOp::DecodePointer {
        offset: D81_GS_ACTIVE_PROVINCE_ROOT_OFF as u32,
        guard: None,
        next: op::ACTIVE_ROOT,
    };
    ops[op::ACTIVE_ROOT as usize] = RecipeOp::CopyHeader {
        size: (D81_ACTIVE_ROOT_TABLE_OFF + 8) as u32,
        next: vec![op::ACTIVE_TABLE],
    };

    ops[op::SELECTOR_MAP_PTR as usize] = RecipeOp::DecodePointer {
        offset: D81_GS_ANCHOR_SELECTOR_MAP_OFF as u32,
        guard: None,
        next: op::SELECTOR_MAP,
    };
    ops[op::SELECTOR_MAP as usize] = RecipeOp::CopyHeader {
        size: (D81_SELECTOR_MAP_MAX_DISTANCE_OFF + 1) as u32,
        next: vec![op::SELECTOR_TABLE],
    };
    ops[op::SELECTOR_TABLE as usize] = RecipeOp::DecodeExtent {
        count_offset: D81_SELECTOR_MAP_MASK_OFF as u32,
        count_width: 4,
        ptr_offset: D81_SELECTOR_MAP_TABLE_OFF as u32,
        stride: D81_SELECTOR_ENTRY_STRIDE as u32,
        max_count: D81_MAX_SELECTOR_SLOTS,
        max_bytes: D81_MAX_SINGLE_RANGE_BYTES,
        trailer_elements: 0,
        guard: None,
        register: None,
        element: vec![op::FREEZE],
    };

    ops[op::RAIL_TABLE as usize] = RecipeOp::DecodeExtent {
        count_offset: D81_RAIL_MANAGER_TABLE_OFF as u32,
        count_width: 4,
        ptr_offset: D81_RAIL_MANAGER_TABLE_OFF as u32,
        stride: 8,
        max_count: D81_MAX_PROVINCES,
        max_bytes: D81_MAX_SINGLE_RANGE_BYTES,
        trailer_elements: 0,
        guard: None,
        register: None,
        element: vec![op::FREEZE],
    };
    ops[op::ACTIVE_TABLE as usize] = RecipeOp::DecodeExtent {
        count_offset: D81_ACTIVE_ROOT_TABLE_OFF as u32,
        count_width: 4,
        ptr_offset: D81_ACTIVE_ROOT_TABLE_OFF as u32,
        stride: D81_ACTIVE_ENTRY_STRIDE as u32,
        max_count: D81_MAX_PROVINCES,
        max_bytes: D81_MAX_SINGLE_RANGE_BYTES,
        trailer_elements: 0,
        guard: None,
        register: None,
        element: vec![op::ACTIVE_ENTRY],
    };
    ops[op::ACTIVE_ENTRY as usize] = RecipeOp::Freeze;
    ops[op::SOURCE_ARRAY as usize] = RecipeOp::DecodeExtent {
        count_offset: super::CGAMESTATE_CPROVINCE_COUNT as u32,
        count_width: 4,
        ptr_offset: super::CGAMESTATE_CPROVINCE_ARRAY as u32,
        stride: 8,
        max_count: D81_MAX_PROVINCES,
        max_bytes: D81_MAX_SINGLE_RANGE_BYTES,
        trailer_elements: 0,
        guard: None,
        register: None,
        element: vec![op::FREEZE],
    };
    ops[op::MAP_ARRAY as usize] = RecipeOp::DecodeExtent {
        count_offset: D81_CONTAINER_PROVINCE_COUNT_OFF as u32,
        count_width: 4,
        ptr_offset: D81_CONTAINER_PROVINCE_ARRAY_OFF as u32,
        stride: 8,
        max_count: D81_MAX_PROVINCES,
        max_bytes: D81_MAX_SINGLE_RANGE_BYTES,
        trailer_elements: 0,
        guard: None,
        register: None,
        element: vec![op::FREEZE],
    };

    ops[op::IDLER_SLOT as usize] = RecipeOp::CopyHeader {
        size: 8,
        next: vec![op::IDLER_PTR],
    };
    ops[op::IDLER_PTR as usize] = RecipeOp::DecodePointer {
        offset: 0,
        guard: None,
        next: op::IDLER,
    };
    ops[op::IDLER as usize] = RecipeOp::CopyHeader {
        size: (D81_IDLER_OWNER_OFF + 8) as u32,
        next: vec![op::OWNER_PTR],
    };
    ops[op::OWNER_PTR as usize] = RecipeOp::DecodePointer {
        offset: D81_IDLER_OWNER_OFF as u32,
        guard: None,
        next: op::OWNER,
    };
    ops[op::OWNER as usize] = RecipeOp::CopyHeader {
        size: (D81_OWNER_CONTAINER_OFF + 8) as u32,
        next: vec![op::CONTAINER_PTR],
    };
    ops[op::CONTAINER_PTR as usize] = RecipeOp::DecodePointer {
        offset: D81_OWNER_CONTAINER_OFF as u32,
        guard: None,
        next: op::CONTAINER,
    };
    ops[op::CONTAINER as usize] = RecipeOp::CopyHeader {
        size: (D81_CONTAINER_PROVINCE_COUNT_OFF + 4) as u32,
        next: vec![op::MAP_ARRAY],
    };

    ops[op::IDLER_SLOT_B as usize] = RecipeOp::CopyHeader {
        size: 8,
        next: vec![op::FREEZE],
    };

    let mut mods = vec![OpMod::default(); op::COUNT];

    mods[op::SELECTOR_TABLE as usize].count = Some(CountExpr {
        terms: vec![
            CountTerm {
                offset: D81_SELECTOR_MAP_MASK_OFF as u32,
                width: 4,
            },
            CountTerm {
                offset: D81_SELECTOR_MAP_MAX_DISTANCE_OFF as u32,
                width: 1,
            },
        ],
        constant: 2,
    });

    for scoped in [
        op::RAIL_TABLE,
        op::ACTIVE_TABLE,
        op::SOURCE_ARRAY,
        op::MAP_ARRAY,
        op::SELECTOR_TABLE,
    ] {
        mods[scoped as usize].tolerance = Tolerance::Scoped;
    }

    for fatal in [
        op::RAIL_MANAGER_PTR,
        op::ACTIVE_ROOT_PTR,
        op::SELECTOR_MAP_PTR,
        op::IDLER_PTR,
        op::OWNER_PTR,
        op::CONTAINER_PTR,
    ] {
        mods[fatal as usize].admit = Some(heap_window(RejectAction::RefuseRecipe));
    }

    for op in [
        op::GAMESTATE,
        op::RAIL_MANAGER,
        op::ACTIVE_ROOT,
        op::SELECTOR_MAP,
        op::SELECTOR_TABLE,
        op::RAIL_TABLE,
        op::ACTIVE_TABLE,
        op::SOURCE_ARRAY,
        op::MAP_ARRAY,
        op::IDLER_SLOT,
        op::IDLER_SLOT_B,
        op::IDLER,
        op::OWNER,
        op::CONTAINER,
    ] {
        mods[op as usize].samples = SampleSpec {
            count: D81_CLOSING_IMAGES_HELD as u8,
            on_disagree: DisagreeAction::Record,
        };
    }
    mods[op::ACTIVE_TABLE as usize].samples = SampleSpec {
        count: super::D81_ACTIVE_COHERENCE_READS as u8,
        on_disagree: DisagreeAction::Record,
    };

    let enumerated = CountExpr {
        terms: Vec::new(),
        constant: roots.province_slots.max(1),
    };
    mods[op::RAIL_TABLE as usize].count = Some(enumerated.clone());
    mods[op::ACTIVE_TABLE as usize].count = Some(enumerated);

    let mut recipe = Recipe::new(
        D81_RECIPE,
        D81_CONSUMER,
        ops,
        vec![
            Root {
                op: op::GAMESTATE,
                addr: roots.game_state,
            },
            Root {
                op: op::IDLER_SLOT,
                addr: roots.idler_slot,
            },
            Root {
                op: op::IDLER_SLOT_B,
                addr: roots.idler_slot_b,
            },
            Root {
                op: op::CONTAINER,
                addr: roots.container,
            },
        ],
    );
    recipe.limits = Limits {
        max_single_range_bytes: D81_MAX_SINGLE_RANGE_BYTES,
        ..Limits::default()
    };
    recipe.with_mods(mods)
}

const D81_MAX_SELECTOR_SLOTS: u32 = 1 << 20;

#[derive(Default)]
pub struct Domain81SnapshotMemory;

pub struct CountingSnapshotMemory<M = Domain81SnapshotMemory> {
    inner: M,
    calls: std::cell::Cell<u64>,
}

impl<M: Default> Default for CountingSnapshotMemory<M> {
    fn default() -> Self {
        Self::new(M::default())
    }
}

impl<M> CountingSnapshotMemory<M> {
    pub fn new(inner: M) -> Self {
        Self {
            inner,
            calls: std::cell::Cell::new(0),
        }
    }

    pub fn calls(&self) -> u64 {
        self.calls.get()
    }
}

impl<M: SnapshotMemory> SnapshotMemory for CountingSnapshotMemory<M> {
    fn copy_into(&self, addr: u64, out: &mut [u8]) -> Result<(), MemoryRefusal> {
        self.calls.set(self.calls.get().saturating_add(1));
        self.inner.copy_into(addr, out)
    }
}

impl SnapshotMemory for Domain81SnapshotMemory {
    fn copy_into(&self, addr: u64, out: &mut [u8]) -> Result<(), MemoryRefusal> {
        let span = Span::new(addr, out.len() as u64);
        let Ok(start) = usize::try_from(addr) else {
            return Err(MemoryRefusal { span });
        };
        if super::read_into(start, out) {
            Ok(())
        } else {
            Err(MemoryRefusal { span })
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum D81Image {
    Opening,
    Sample(usize),
    Closing,
    ClosingSample(usize),
}

pub const D81_CLOSING_IMAGES_HELD: usize = 2;

pub const D81_MISS_UNREADABLE: u32 = 1;
pub const D81_MISS_NOT_CAPTURED: u32 = 2;
pub const D81_MISS_NOT_OWNED: u32 = 3;
pub const D81_MISS_NOT_CONTIGUOUS: u32 = 4;
pub const D81_MISS_NO_CLOSING_IMAGE: u32 = 5;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct D81ViewMisses {
    pub unreadable: u32,
    pub not_captured: u32,
    pub not_owned: u32,
    pub not_contiguous: u32,
    pub no_closing_image: u32,
    pub samples_absent: u32,
    pub first_class: u32,
    pub first_addr: u64,
    pub first_len: u32,
    pub last_class: u32,
    pub last_addr: u64,
}

impl D81ViewMisses {
    pub fn plan_defects(&self) -> u32 {
        self.not_captured
            .saturating_add(self.not_owned)
            .saturating_add(self.not_contiguous)
            .saturating_add(self.no_closing_image)
    }

    pub fn total(&self) -> u32 {
        self.unreadable.saturating_add(self.plan_defects())
    }
}

pub enum D81ReadSource<'a> {
    Live,
    Snapshot(D81SnapshotSource<'a>),
    #[cfg(test)]
    Staged(&'a dyn crate::snapshot_recipes::SnapshotMemory),
}

pub struct D81SnapshotSource<'a> {
    view: crate::snapshot_recipes::view::SnapshotView<'a>,
    vector: Option<&'a super::d81_vector_store::D81VectorCapture>,
    misses: std::cell::Cell<D81ViewMisses>,
}

impl<'a> D81ReadSource<'a> {
    pub fn snapshot(
        view: crate::snapshot_recipes::view::SnapshotView<'a>,
        vector: Option<&'a super::d81_vector_store::D81VectorCapture>,
    ) -> Self {
        D81ReadSource::Snapshot(D81SnapshotSource {
            view,
            vector,
            misses: std::cell::Cell::new(D81ViewMisses::default()),
        })
    }

    pub fn is_snapshot(&self) -> bool {
        matches!(self, D81ReadSource::Snapshot(_))
    }

    pub fn misses(&self) -> D81ViewMisses {
        match self {
            D81ReadSource::Snapshot(source) => source.misses.get(),
            _ => D81ViewMisses::default(),
        }
    }

    pub fn read_into(&self, addr: usize, out: &mut [u8], image: D81Image) -> bool {
        match self {
            D81ReadSource::Live => super::read_into(addr, out),
            D81ReadSource::Snapshot(source) => source.read_into(addr, out, image),
            #[cfg(test)]
            D81ReadSource::Staged(memory) => memory.copy_into(addr as u64, out).is_ok(),
        }
    }

    pub fn scalar<const N: usize>(&self, addr: usize, image: D81Image) -> Option<[u8; N]> {
        let mut out = [0u8; N];
        self.read_into(addr, &mut out, image).then_some(out)
    }

    pub fn u64_at(&self, addr: usize, image: D81Image) -> Option<u64> {
        self.scalar::<8>(addr, image).map(u64::from_le_bytes)
    }

    pub fn u32_at(&self, addr: usize, image: D81Image) -> Option<u32> {
        self.scalar::<4>(addr, image).map(u32::from_le_bytes)
    }

    pub fn u8_at(&self, addr: usize, image: D81Image) -> Option<u8> {
        self.scalar::<1>(addr, image).map(|byte| byte[0])
    }

    pub fn paired_image(image: D81Image) -> D81Image {
        match image {
            D81Image::Opening => D81Image::Sample(1),
            D81Image::Closing => D81Image::ClosingSample(1),
            other => other,
        }
    }

    pub fn u64_stable_at(&self, addr: usize, image: D81Image) -> Option<u64> {
        let first = self.u64_at(addr, image)?;
        let second = self.u64_at(addr, Self::paired_image(image))?;
        (first == second).then_some(first)
    }

    pub fn u32_stable_at(&self, addr: usize, image: D81Image) -> Option<u32> {
        let first = self.u32_at(addr, image)?;
        let second = self.u32_at(addr, Self::paired_image(image))?;
        (first == second).then_some(first)
    }

    pub fn u8_stable_at(&self, addr: usize, image: D81Image) -> Option<u8> {
        let first = self.u8_at(addr, image)?;
        let second = self.u8_at(addr, Self::paired_image(image))?;
        (first == second).then_some(first)
    }

    pub fn u64_stable(&self, addr: usize) -> Option<u64> {
        self.u64_stable_at(addr, D81Image::Opening)
    }

    pub fn u32_stable(&self, addr: usize) -> Option<u32> {
        self.u32_stable_at(addr, D81Image::Opening)
    }

    pub fn u8_stable(&self, addr: usize) -> Option<u8> {
        self.u8_stable_at(addr, D81Image::Opening)
    }
}

impl D81SnapshotSource<'_> {
    fn note(&self, class: u32, span: crate::snapshot_recipes::Span) {
        let mut misses = self.misses.get();
        match class {
            D81_MISS_UNREADABLE => misses.unreadable = misses.unreadable.saturating_add(1),
            D81_MISS_NOT_CAPTURED => misses.not_captured = misses.not_captured.saturating_add(1),
            D81_MISS_NOT_OWNED => misses.not_owned = misses.not_owned.saturating_add(1),
            D81_MISS_NOT_CONTIGUOUS => {
                misses.not_contiguous = misses.not_contiguous.saturating_add(1)
            }
            _ => misses.no_closing_image = misses.no_closing_image.saturating_add(1),
        }
        if misses.first_class == 0 {
            misses.first_class = class;
            misses.first_addr = span.start;
            misses.first_len = span.len.min(u64::from(u32::MAX)) as u32;
        }
        misses.last_class = class;
        misses.last_addr = span.start;
        self.misses.set(misses);
    }

    fn note_absent_sample(&self) {
        let mut misses = self.misses.get();
        misses.samples_absent = misses.samples_absent.saturating_add(1);
        self.misses.set(misses);
    }

    fn class_of(refusal: crate::snapshot_recipes::view::ViewRefusal) -> u32 {
        use crate::snapshot_recipes::view::ViewRefusal as R;
        match refusal {
            R::Unreadable { .. } => D81_MISS_UNREADABLE,
            R::NotOwned { .. } => D81_MISS_NOT_OWNED,
            R::NotContiguous { .. } => D81_MISS_NOT_CONTIGUOUS,
            R::NotCaptured { .. } | R::ForeignHour { .. } | R::ConsumerGapped(_) => {
                D81_MISS_NOT_CAPTURED
            }
        }
    }

    fn read_into(&self, addr: usize, out: &mut [u8], image: D81Image) -> bool {
        let span = crate::snapshot_recipes::Span::new(addr as u64, out.len() as u64);
        if let Some(vector) = self.vector {
            use super::d81_vector_store::{VectorAnswer, VectorImage};
            let asked = match image {
                D81Image::Opening => VectorImage::Opening,
                D81Image::Sample(index) => VectorImage::Sample(index),
                D81Image::Closing => VectorImage::Closing,
                D81Image::ClosingSample(index) => VectorImage::ClosingSample(index),
            };
            match vector.read(addr as u64, out.len() as u32, asked) {
                VectorAnswer::Served(bytes) => {
                    out.copy_from_slice(bytes);
                    return true;
                }
                VectorAnswer::NotCovered => {}
                VectorAnswer::Unreadable => {
                    self.note(D81_MISS_UNREADABLE, span);
                    return false;
                }
                VectorAnswer::SampleAbsent => {
                    match image {
                        D81Image::Closing | D81Image::ClosingSample(_) => {
                            self.note(D81_MISS_UNREADABLE, span);
                        }
                        _ => self.note_absent_sample(),
                    }
                    return false;
                }
                VectorAnswer::Aliased => {
                    self.note(D81_MISS_NOT_CAPTURED, span);
                    return false;
                }
                VectorAnswer::NeverObserved => {
                    let class = match image {
                        D81Image::Closing | D81Image::ClosingSample(_) => D81_MISS_NO_CLOSING_IMAGE,
                        _ => D81_MISS_NOT_CAPTURED,
                    };
                    self.note(class, span);
                    return false;
                }
            }
        }
        match image {
            D81Image::Opening => match self.view.read_into(addr as u64, out) {
                Ok(()) => true,
                Err(refusal) => {
                    self.note(Self::class_of(refusal), span);
                    false
                }
            },
            D81Image::Sample(index) => {
                match self.view.sample_bytes(addr as u64, out.len(), index) {
                    Ok(bytes) => {
                        out.copy_from_slice(bytes);
                        true
                    }
                    Err(crate::snapshot_recipes::view::ViewRefusal::NotCaptured { .. }) => {
                        self.note_absent_sample();
                        false
                    }
                    Err(refusal) => {
                        self.note(Self::class_of(refusal), span);
                        false
                    }
                }
            }
            D81Image::Closing | D81Image::ClosingSample(_) => {
                let index = match image {
                    D81Image::ClosingSample(index) => index,
                    _ => 0,
                };
                match self
                    .view
                    .closing_sample_bytes(addr as u64, out.len(), index)
                {
                    Ok(bytes) => {
                        out.copy_from_slice(bytes);
                        true
                    }
                    Err(crate::snapshot_recipes::view::ViewRefusal::NotCaptured { .. }) => {
                        self.note(D81_MISS_NO_CLOSING_IMAGE, span);
                        false
                    }
                    Err(refusal) => {
                        self.note(Self::class_of(refusal), span);
                        false
                    }
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct D81PlanCounters {
    pub ran: bool,
    pub copies_issued: u64,
    pub intervals_closed: u32,
    pub scoped_voids: u32,
    pub pointers_rejected: u32,
    pub custody_refused: bool,
    pub consumer_gapped: bool,
    pub refused: bool,
    pub refusal_ordinal: u32,
    pub gap_ordinal: u32,
    pub custody_held_bytes: u64,
    pub plan_us: u64,
    pub misses: D81ViewMisses,
}

pub fn d81_plan_refusal_ordinal(refusal: &crate::snapshot_recipes::Refusal) -> u32 {
    use crate::snapshot_recipes::Refusal as R;
    match refusal {
        R::DepthExceeded { .. } => 1,
        R::CopyTaskLimitExceeded { .. } => 2,
        R::RangeLimitExceeded { .. } => 3,
        R::ByteLimitExceeded { .. } => 4,
        R::RangeTooWide { .. } => 5,
        R::ExtentCountExceeded { .. } => 6,
        R::ExtentBytesExceeded { .. } => 7,
        R::JoinOutOfRange { .. } => 8,
        R::JoinTableMissing { .. } => 9,
        R::FieldNotCaptured { .. } => 10,
        R::MemoryUnreadable { .. } => 11,
        R::AddressNotRepresentable { .. } => 12,
        R::PointerNotAdmissible { .. } => 13,
        R::CarryNotSet { .. } => 14,
        R::ConditionalUnresolved { .. } => 15,
        R::ClosedIntervalReopened { .. } => 16,
        R::Malformed { .. } => 17,
        R::CarryHolderUnknown { .. } => 18,
    }
}

pub fn d81_plan_refusal_unit_name(ordinal: u32) -> &'static str {
    match ordinal {
        1 => "d81_plan_refusal_01_depth_exceeded",
        2 => "d81_plan_refusal_02_copy_task_limit",
        3 => "d81_plan_refusal_03_range_limit",
        4 => "d81_plan_refusal_04_byte_limit",
        5 => "d81_plan_refusal_05_range_too_wide",
        6 => "d81_plan_refusal_06_extent_count",
        7 => "d81_plan_refusal_07_extent_bytes",
        8 => "d81_plan_refusal_08_join_out_of_range",
        9 => "d81_plan_refusal_09_join_table_missing",
        10 => "d81_plan_refusal_10_field_not_captured",
        11 => "d81_plan_refusal_11_memory_unreadable",
        12 => "d81_plan_refusal_12_address_not_representable",
        13 => "d81_plan_refusal_13_pointer_not_admissible",
        14 => "d81_plan_refusal_14_carry_not_set",
        15 => "d81_plan_refusal_15_conditional_unresolved",
        16 => "d81_plan_refusal_16_closed_interval_reopened",
        17 => "d81_plan_refusal_17_malformed",
        18 => "d81_plan_refusal_18_carry_holder_unknown",
        _ => "d81_plan_refusal_00_unknown",
    }
}

pub fn d81_plan_gap_ordinal(refusal: &crate::snapshot_recipes::view::ViewRefusal) -> u32 {
    use crate::snapshot_recipes::view::ViewRefusal as V;
    use crate::snapshot_recipes::GapReason as G;
    match refusal {
        V::ForeignHour { .. } => 1,
        V::ConsumerGapped(G::Refused(_)) => 2,
        V::ConsumerGapped(G::WitnessChanged { .. }) => 3,
        V::ConsumerGapped(G::WitnessDependency { .. }) => 4,
        V::ConsumerGapped(G::SamplesDisagreed { .. }) => 5,
        V::NotCaptured { .. } => 6,
        V::NotContiguous { .. } => 7,
        V::NotOwned { .. } => 8,
        V::Unreadable { .. } => 9,
    }
}

pub fn d81_plan_gap_unit_name(ordinal: u32) -> &'static str {
    match ordinal {
        1 => "d81_plan_gap_01_foreign_hour",
        2 => "d81_plan_gap_02_recipe_refused",
        3 => "d81_plan_gap_03_witness_changed",
        4 => "d81_plan_gap_04_witness_dependency",
        5 => "d81_plan_gap_05_samples_disagreed",
        6 => "d81_plan_gap_06_not_captured",
        7 => "d81_plan_gap_07_not_contiguous",
        8 => "d81_plan_gap_08_not_owned",
        9 => "d81_plan_gap_09_unreadable",
        _ => "d81_plan_gap_00_unknown",
    }
}

pub const D81_COPY_BRACKET_SHADOW_ENV: &str = "BATTLE_PLAN_D81_COPY_BRACKET_SHADOW";

pub const D81_SHADOW_FIELD_UNITS: [&str; 7] = [
    "d81_shadow_divergence_00_none",
    "d81_shadow_divergence_01_row_present",
    "d81_shadow_divergence_02_province_id",
    "d81_shadow_divergence_03_active_terminal",
    "d81_shadow_divergence_04_anchor_variant",
    "d81_shadow_divergence_05_anchor_bits",
    "d81_shadow_divergence_06_neighbors",
];

pub fn d81_shadow_divergence_unit_name(ordinal: u32) -> &'static str {
    D81_SHADOW_FIELD_UNITS
        .get(ordinal as usize)
        .copied()
        .unwrap_or("d81_shadow_divergence_99_unknown")
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct D81EqualityCounters {
    pub armed: bool,
    pub ran: bool,
    pub rows_live: u32,
    pub rows_authoritative: u32,
    pub rows_compared: u32,
    pub rows_equal: u32,
    pub rows_divergent: u32,
    pub first_divergence_province: u32,
    pub first_divergence_field: u32,
    pub failures_agree: bool,
    pub live_failure: u32,
    pub authoritative_failure: u32,
    pub live_neighbors: u32,
    pub authoritative_neighbors: u32,
    pub live_walk_us: u64,
}

pub struct D81WalkVerdict<'a, F> {
    pub provinces: &'a [compact_contract::rail_replay::RailProvinceState],
    pub observed_neighbors: usize,
    pub failure: Option<F>,
    pub failure_word: u32,
}

pub fn d81_equality_compare<F: PartialEq>(
    live: &D81WalkVerdict<'_, F>,
    authoritative: &D81WalkVerdict<'_, F>,
    counters: &mut D81EqualityCounters,
) {
    counters.ran = true;
    counters.rows_live = live.provinces.len().min(u32::MAX as usize) as u32;
    counters.rows_authoritative = authoritative.provinces.len().min(u32::MAX as usize) as u32;
    counters.live_neighbors = live.observed_neighbors.min(u32::MAX as usize) as u32;
    counters.authoritative_neighbors =
        authoritative.observed_neighbors.min(u32::MAX as usize) as u32;
    counters.live_failure = live.failure_word;
    counters.authoritative_failure = authoritative.failure_word;
    counters.failures_agree = live.failure == authoritative.failure;

    let rows = live.provinces.len().max(authoritative.provinces.len());
    for index in 0..rows {
        counters.rows_compared = counters.rows_compared.saturating_add(1);
        let (Some(left), Some(right)) = (
            live.provinces.get(index),
            authoritative.provinces.get(index),
        ) else {
            counters.rows_divergent = counters.rows_divergent.saturating_add(1);
            if counters.first_divergence_field == 0 {
                counters.first_divergence_field = 1;
                counters.first_divergence_province = live
                    .provinces
                    .get(index)
                    .or_else(|| authoritative.provinces.get(index))
                    .map(|row| row.province_id)
                    .unwrap_or(0);
            }
            continue;
        };
        let diverged = if left.province_id != right.province_id {
            2
        } else if left.active_terminal != right.active_terminal {
            3
        } else if left.anchor_variant != right.anchor_variant {
            4
        } else if left.anchor_bits != right.anchor_bits {
            5
        } else if left.neighbors != right.neighbors {
            6
        } else {
            0
        };
        if diverged == 0 {
            counters.rows_equal = counters.rows_equal.saturating_add(1);
        } else {
            counters.rows_divergent = counters.rows_divergent.saturating_add(1);
            if counters.first_divergence_field == 0 {
                counters.first_divergence_province = left.province_id;
                counters.first_divergence_field = diverged;
            }
        }
    }
}
