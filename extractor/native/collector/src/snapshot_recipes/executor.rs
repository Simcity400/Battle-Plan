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

const MAX_RECIPES: usize = 1 << 48;

#[inline]
pub(super) fn visit_tag(recipe: usize, op: u16) -> u64 {
    ((op as u64) << 48) | recipe as u64
}

#[derive(Debug, Default)]
pub(super) struct VisitedSet {
    ops: FxSet<(u64, u64)>,
    extents: FxSet<(u64, u64, u64)>,
}

impl VisitedSet {
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
            let took = super::arena::hash_table_bytes::<(u64, u64)>(ops.capacity());
            ledger.charge_tables_overshoot(took.saturating_sub(wanted))?;
        }
        Ok(Self {
            ops,
            extents: FxSet::default(),
        })
    }

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

    pub(super) fn charged_bytes(&self) -> u64 {
        super::arena::hash_table_bytes::<(u64, u64)>(self.ops.capacity())
            + super::arena::hash_table_bytes::<(u64, u64, u64)>(self.extents.capacity())
    }
}

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

    pub const fn at_least(offset: u32, width: u8, min: u64) -> Self {
        Self::new(offset, width, min, u64::MAX)
    }

    pub const fn exactly(offset: u32, width: u8, value: u64) -> Self {
        Self::new(offset, width, value, value)
    }

    fn admits(&self, value: u64) -> bool {
        value >= self.min && value <= self.max
    }
}

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CountTerm {
    pub offset: u32,
    pub width: u8,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CountExpr {
    pub terms: Vec<CountTerm>,
    pub constant: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RejectAction {
    StopBranch,
    RefuseRecipe,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Admissibility {
    pub low: u64,
    pub high: u64,
    pub align: u32,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CarrySource {
    Field { offset: u32, width: u8 },
    Constant(u64),
    ElementIndex,
}

pub type Carry = Option<u64>;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct CarrySlot {
    value: u64,
    holder: u32,
    set: bool,
    holder_set: bool,
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
    fn holder(self) -> Option<IntervalId> {
        if self.set && self.holder_set {
            Some(IntervalId(self.holder))
        } else {
            None
        }
    }

    #[inline]
    fn with_holder(carry: Carry, holder: Option<IntervalId>) -> Self {
        match carry {
            Some(value) => Self {
                value,
                holder: holder.map_or(0, |id| id.0),
                set: true,
                holder_set: holder.is_some(),
            },
            None => Self::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisagreeAction {
    Gap,
    Record,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseAction {
    Gap,
    Record,
}

impl Default for CloseAction {
    fn default() -> Self {
        CloseAction::Gap
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SampleSpec {
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tolerance {
    #[default]
    Required,
    Scoped,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CompareMask {
    pub windows: Vec<RelWindow>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CloseWhen {
    #[default]
    Final,
    AfterWave(u32),
}

impl CloseWhen {
    fn rank(self) -> u64 {
        match self {
            CloseWhen::AfterWave(wave) => u64::from(wave),
            CloseWhen::Final => u64::MAX,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CloseStage {
    pub stage: u16,
    pub elide_into: Option<u16>,
    pub when: CloseWhen,
}

impl CloseStage {
    pub const fn at_end(stage: u16, elide_into: Option<u16>) -> Self {
        Self {
            stage,
            elide_into,
            when: CloseWhen::Final,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClosingProgram {
    pub stages: Vec<CloseStage>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OpMod {
    pub count: Option<CountExpr>,
    pub admit: Option<Admissibility>,
    pub carry: Option<CarrySource>,
    pub key_from_carry: bool,
    pub anchor_on_carry_holder: bool,
    pub instance_from_carry: bool,
    pub samples: SampleSpec,
    pub tolerance: Tolerance,
    pub on_close: CloseAction,
    pub compare: CompareMask,
    pub stage: u16,
}

static UNQUALIFIED: OpMod = OpMod {
    count: None,
    admit: None,
    carry: None,
    key_from_carry: false,
    anchor_on_carry_holder: false,
    instance_from_carry: false,
    samples: SampleSpec {
        count: 1,
        on_disagree: DisagreeAction::Gap,
    },
    tolerance: Tolerance::Required,
    on_close: CloseAction::Gap,
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
            && !self.anchor_on_carry_holder
            && !self.instance_from_carry
            && self.samples == SampleSpec::default()
            && self.tolerance == Tolerance::default()
            && self.on_close == CloseAction::default()
            && self.compare.windows.is_empty()
            && self.stage == 0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Probe {
    FieldEquals {
        offset: u32,
        width: u8,
        value: u64,
    },
    FieldInRange {
        offset: u32,
        width: u8,
        low: u64,
        high: u64,
    },
    SamplesAgree {
        offset: u32,
        len: u32,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecipeOp {
    CopyHeader {
        size: u32,
        next: Vec<u16>,
    },
    CopyRanges {
        windows: Vec<RelWindow>,
        next: Vec<u16>,
    },
    DecodePointer {
        offset: u32,
        guard: Option<ExtentGuard>,
        next: u16,
    },
    DecodeExtent {
        count_offset: u32,
        count_width: u8,
        ptr_offset: u32,
        stride: u32,
        max_count: u32,
        max_bytes: u64,
        trailer_elements: u32,
        guard: Option<ExtentGuard>,
        register: Option<TableId>,
        element: Vec<u16>,
    },
    JoinBounded {
        table: TableId,
        key_offset: u32,
        key_width: u8,
        next: u16,
    },
    SelectByValue {
        probe: Probe,
        then: u16,
        otherwise: Option<u16>,
    },
    WitnessClose {
        next: Vec<u16>,
    },
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Root {
    pub op: u16,
    pub addr: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recipe {
    pub id: RecipeId,
    pub consumer: ConsumerId,
    pub limits: Limits,
    pub policy: GapPolicy,
    pub ops: Vec<RecipeOp>,
    pub roots: Vec<Root>,
    pub mods: Vec<OpMod>,
    pub closing: Option<ClosingProgram>,
}

impl Recipe {
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

    pub fn with_mods(mut self, mods: Vec<OpMod>) -> Self {
        self.mods = mods;
        self
    }

    pub fn with_closing(mut self, closing: ClosingProgram) -> Self {
        self.closing = Some(closing);
        self
    }

    #[inline]
    pub(super) fn op_mod(&self, op: u16) -> &OpMod {
        if self.mods.is_empty() {
            return &UNQUALIFIED;
        }
        &self.mods[op as usize]
    }

    pub(super) fn needs_annotations(&self) -> bool {
        self.closing.is_some() || self.mods.iter().any(|op| !op.is_default())
    }

    pub(super) fn reads_fields(&self) -> bool {
        self.ops.iter().any(|op| {
            matches!(
                op,
                RecipeOp::DecodePointer { .. }
                    | RecipeOp::DecodeExtent { .. }
                    | RecipeOp::JoinBounded { .. }
                    | RecipeOp::SelectByValue { .. }
            )
        }) || self
            .mods
            .iter()
            .any(|modifier| matches!(modifier.carry, Some(CarrySource::Field { .. })))
    }

    /// unconditionally would put one `OwnershipMap::find` on every carry-setting
    pub(super) fn anchors_on_carry_holder(&self) -> bool {
        self.mods
            .iter()
            .any(|modifier| modifier.anchor_on_carry_holder)
    }
}

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

fn validate_mod(recipe: &Recipe, here: u16, op: &RecipeOp) -> Result<(), MalformedReason> {
    if recipe.mods.is_empty() {
        return Ok(());
    }
    let modifier = &recipe.mods[here as usize];
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
    if modifier.anchor_on_carry_holder
        && (!modifier.key_from_carry || !matches!(op, RecipeOp::JoinBounded { .. }))
    {
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
        let inside = match op {
            RecipeOp::CopyHeader { size, .. } => modifier.compare.windows.iter().all(|window| {
                window.offset >= 0
                    && window.len > 0
                    && (window.offset as u64)
                        .checked_add(u64::from(window.len))
                        .map(|end| end <= u64::from(*size))
                        .unwrap_or(false)
            }),
            RecipeOp::CopyRanges { windows, .. } => modifier.compare.windows.iter().all(|mask| {
                mask.len > 0
                    && windows.iter().any(|window| {
                        mask.offset >= window.offset
                            && i128::from(mask.offset) + i128::from(mask.len)
                                <= i128::from(window.offset) + i128::from(window.len)
                    })
            }),
            RecipeOp::DecodeExtent { stride, .. } => modifier.compare.windows.iter().all(|mask| {
                mask.offset >= 0
                    && mask.len > 0
                    && (mask.offset as u64 + u64::from(mask.len)) <= u64::from(*stride)
            }),
            _ => false,
        };
        if !inside {
            return Err(MalformedReason::BadCompareMask { op: here });
        }
    }
    Ok(())
}

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
        if position > 0 {
            let previous = program.stages[position - 1].when;
            if stage.when.rank() < previous.rank() {
                return Err(MalformedReason::BadClosingProgram { stage: stage.stage });
            }
        }
        if let CloseWhen::AfterWave(wave) = stage.when {
            if wave >= FORWARD_WAVES {
                return Err(MalformedReason::BadClosingProgram { stage: stage.stage });
            }
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecipeOutcome {
    Captured,
    Refused(RefusalRecord),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecipeReport {
    pub recipe: RecipeId,
    pub consumer: ConsumerId,
    pub outcome: RecipeOutcome,
    pub copy_tasks: u32,
    pub planned_ranges: u32,
    pub planned_bytes: u64,
    pub frozen_branches: u32,
    pub witness_required: bool,
    pub scoped_voids: u32,
    pub pointers_rejected: u32,
}

#[derive(Debug)]
pub struct SnapshotBuild {
    pub snapshot: Snapshot,
    pub reports: Vec<RecipeReport>,
    pub forward_waves_run: u32,
    pub phases_run: u32,
    pub witnessed_intervals: u32,
    pub closing_copies: u64,
    pub final_wave_copies: u64,
    pub custody_held: u64,
    pub custody_table_bytes: u64,
    pub custody_arena_bytes: u64,
    pub(super) unreadable_closes: Vec<IntervalId>,
    pub closing_order: Vec<IntervalId>,
    pub elisions: Vec<Elision>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Elision {
    pub interval: IntervalId,
    pub elided: u16,
    pub into: u16,
}

impl SnapshotBuild {
    pub fn report_for(&self, recipe: RecipeId) -> Option<&RecipeReport> {
        self.reports.iter().find(|r| r.recipe == recipe)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ObservationConfig {
    pub hour: HourStamp,
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

pub fn observe<M: SnapshotMemory>(
    recipes: &[Recipe],
    memory: &M,
    config: ObservationConfig,
) -> Result<SnapshotBuild, CustodyRefusal> {
    observe_in(
        recipes,
        memory,
        config,
        Arena::with_budget(config.custody_budget),
    )
}

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CopyAttributes {
    samples: SampleSpec,
    tolerance: Tolerance,
    on_close: CloseAction,
    stage: u16,
    op: u16,
}

impl Default for CopyAttributes {
    fn default() -> Self {
        Self {
            samples: SampleSpec::default(),
            tolerance: Tolerance::Required,
            on_close: CloseAction::Gap,
            stage: 0,
            op: 0,
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum MaskSource {
    At(u64),
    Strided(u64, u32, u32),
}

enum ResolvedMask {
    All,
    Spans(Vec<Span>),
    Strided(super::annot::StridedMask),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TaskAnchor {
    Root,
    Pointer,
}

impl TaskAnchor {
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
    Op {
        op: u16,
        addr: u64,
        anchor: TaskAnchor,
        inherited: Proof,
    },
    Extent {
        op: u16,
        span: Span,
        count: u32,
    },
}

#[derive(Clone, Debug)]
pub(super) struct CopyTask {
    recipe: usize,
    depth: u32,
    parent: Option<IntervalId>,
    carry: CarrySlot,
    body: TaskBody,
}

#[derive(Clone, Copy, Debug)]
struct DecodeSite {
    op: u16,
    base: u64,
    region: Proof,
    carry: CarrySlot,
    index: u32,
}

/// interval lookup and the ownership check below are almost always the answer
/// A reach only WIDENS — [`OwnershipMap::add_dependent`] hulls into the node it
#[derive(Clone, Copy, Debug)]
struct DecodeHint {
    interval: u32,
    consumer: ConsumerId,
    reach: Span,
    start: u64,
    opening_offset: u32,
}

const NO_HINT: u32 = u32::MAX;

impl Default for DecodeHint {
    fn default() -> Self {
        Self {
            interval: NO_HINT,
            consumer: ConsumerId(0),
            reach: Span::new(0, 0),
            start: 0,
            opening_offset: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct PrunedKey {
    start: u64,
    len: u64,
    recipe: u32,
    parent: u32,
    request: u32,
}

impl PrunedKey {
    /// Whether two keys ask the ownership map for exactly the same thing.
    #[inline]
    fn same_ask(&self, other: &PrunedKey) -> bool {
        self.start == other.start
            && self.len == other.len
            && self.recipe == other.recipe
            && self.parent == other.parent
    }
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
    /// whether the holder is worth an ownership lookup, and `d81`'s rail walk
    anchors_on_carry_holder: bool,
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
            anchors_on_carry_holder: false,
        }
    }
}

fn stage_entry(recipes: &[Recipe], stage: u16) -> Option<&CloseStage> {
    recipes
        .iter()
        .filter_map(|recipe| recipe.closing.as_ref())
        .find_map(|program| program.stages.iter().find(|entry| entry.stage == stage))
}

fn stage_rank(recipes: &[Recipe], stage: u16) -> Option<usize> {
    recipes
        .iter()
        .filter_map(|recipe| recipe.closing.as_ref())
        .find_map(|program| program.stages.iter().position(|entry| entry.stage == stage))
}

struct Observation<'a, M: SnapshotMemory> {
    recipes: &'a [Recipe],
    memory: &'a M,
    config: ObservationConfig,
    arena: Arena,
    owners: OwnershipMap,
    states: Vec<RecipeState>,
    tables: BTreeMap<(usize, TableId, u64), TableBinding>,
    annot: super::annot::Annotations,
    voids: Vec<super::Void>,
    rejections: Vec<super::PointerRejection>,
    visited: VisitedSet,
    pruned: Vec<CopyTask>,
    pruned_order: Vec<PrunedKey>,
    task_hint: Vec<DecodeHint>,
    waves: Vec<Vec<CopyTask>>,
    forward_waves_run: u32,
    closing_copies: u64,
    by_recipe: Vec<Vec<u32>>,
    requests: Vec<CopyRequest>,
    plan: super::policy::CopyPlan,
    acquisition: super::dedup::AcquisitionPlan,
    owned: Vec<IntervalId>,
    sites: Vec<DecodeSite>,
    own_tables_charged: u64,
    hints_wanted: bool,
    decode_hint: std::cell::Cell<DecodeHint>,
    mid_walk: bool,
    /// ownership links record a `ConsumerId`, and two recipes may legally share
    mid_walk_pending: Vec<(IntervalId, usize)>,
    mid_walk_closed: Vec<(IntervalId, u32)>,
    elisions: Vec<Elision>,
    unreadable_closes: Vec<IntervalId>,
    void_context: Vec<(usize, Span)>,
    mid_walk_done: Vec<bool>,
    stage_rank_memo: Vec<(u32, u16)>,
}

const NO_CLOSING_RANK: u32 = u32::MAX;

impl<'a, M: SnapshotMemory> Observation<'a, M> {
    fn new(
        recipes: &'a [Recipe],
        memory: &'a M,
        config: ObservationConfig,
        arena: Arena,
    ) -> Result<Self, CustodyRefusal> {
        // One ledger for the observation: the ownership map's tables are
        let owners = OwnershipMap::on_ledger(arena.ledger());
        assert!(
            recipes.len() <= MAX_RECIPES,
            "an observation may hold at most {MAX_RECIPES} recipes: past that the discovery \
             visited set's key would alias two of them",
        );
        let visited = VisitedSet::with_root_capacity(
            recipes.iter().map(|recipe| recipe.roots.len()).sum(),
            arena.ledger_ref(),
        )?;
        let own_tables_charged = visited.charged_bytes();
        Ok(Self {
            recipes,
            memory,
            config,
            arena,
            owners,
            states: recipes
                .iter()
                .map(|recipe| {
                    let mut state = RecipeState::new();
                    state.anchors_on_carry_holder = recipe.anchors_on_carry_holder();
                    state
                })
                .collect(),
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
            pruned_order: Vec::new(),
            task_hint: Vec::new(),
            waves: (0..FORWARD_WAVES + 1).map(|_| Vec::new()).collect(),
            forward_waves_run: 0,
            closing_copies: 0,
            by_recipe: Vec::new(),
            requests: Vec::new(),
            plan: super::policy::CopyPlan::new(),
            acquisition: super::dedup::AcquisitionPlan::default(),
            owned: Vec::new(),
            sites: Vec::new(),
            own_tables_charged,
            hints_wanted: recipes.iter().any(Recipe::reads_fields),
            decode_hint: std::cell::Cell::new(DecodeHint::default()),
            mid_walk: recipes.iter().any(|recipe| {
                recipe.closing.as_ref().is_some_and(|program| {
                    program
                        .stages
                        .iter()
                        .any(|stage| stage.when != CloseWhen::Final)
                })
            }),
            mid_walk_pending: Vec::new(),
            mid_walk_closed: Vec::new(),
            elisions: Vec::new(),
            unreadable_closes: Vec::new(),
            void_context: Vec::new(),
            mid_walk_done: Vec::new(),
            stage_rank_memo: Vec::new(),
        })
    }

    /// [`OwnershipMap`] admits its growth before it takes it, because its
    fn charge_own_tables(&mut self, in_flight: u64) -> Result<(), CustodyRefusal> {
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

    fn charge_own_tables_holding(&mut self, tasks: &Vec<CopyTask>) -> Result<(), CustodyRefusal> {
        self.charge_own_tables(super::arena::vec_bytes(tasks))
    }

    fn own_tables_bytes(&self, in_flight: u64) -> u64 {
        const TABLE_BINDING_BYTES: u64 = 96;
        let mut bytes = self.tables.len() as u64 * TABLE_BINDING_BYTES
            + in_flight
            + super::arena::vec_bytes(&self.pruned)
            + super::arena::vec_bytes(&self.pruned_order)
            + super::arena::vec_bytes(&self.task_hint)
            + super::arena::vec_bytes(&self.waves)
            + super::arena::vec_bytes(&self.by_recipe)
            + super::arena::vec_bytes(&self.requests)
            + super::arena::vec_bytes(&self.owned)
            + super::arena::vec_bytes(&self.sites)
            + super::arena::vec_bytes(&self.mid_walk_done)
            + super::arena::vec_bytes(&self.stage_rank_memo)
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

    fn refuse_witnessed(&mut self, recipe: usize, refusal: Refusal, witness: RefusalWitness) {
        if self.states[recipe].refusal.is_none() {
            self.states[recipe].refusal = Some(RefusalRecord::new(refusal, witness));
        }
    }

    fn field_not_captured(&mut self, recipe: usize, span: Span, witness: RefusalWitness) {
        let id = self.recipes[recipe].id;
        let lost_to_own_void =
            self.void_context
                .iter()
                .zip(self.voids.iter())
                .any(|((owner, request), void)| {
                    *owner == recipe && request.contains_span(&span) && void.span.overlaps(&span)
                });
        if lost_to_own_void {
            return;
        }
        self.refuse_witnessed(
            recipe,
            Refusal::FieldNotCaptured { recipe: id, span },
            witness,
        );
    }

    fn refuse(&mut self, recipe: usize, refusal: Refusal) {
        self.refuse_witnessed(recipe, refusal, RefusalWitness::default());
    }

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

    fn reserve_root_queue(&mut self) -> Result<(), CustodyRefusal> {
        let roots: usize = self
            .recipes
            .iter()
            .enumerate()
            .filter(|(index, _)| self.states[*index].refusal.is_none())
            .map(|(_, recipe)| {
                recipe
                    .roots
                    .len()
                    .min(recipe.limits.max_copy_tasks as usize)
            })
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
        for (index, recipe) in self.recipes.iter().enumerate() {
            if let Err(refusal) = validate(recipe) {
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
            self.copy_wave(&tasks, depth)?;
            self.owners.end_wave()?;
            self.decode_wave(&tasks);
            self.charge_own_tables_holding(&tasks)?;
            if self.mid_walk {
                self.mid_walk_close(depth)?;
            }
        }

        debug_assert!(self.waves[FORWARD_WAVES as usize].is_empty());

        self.link_pruned()?;
        self.charge_own_tables(0)?;
        self.enrol_witnesses()?;
        let closing_copies_before_final_wave = self.closing_copies;
        let closing = self.closing_wave()?;
        let (witnessed, unreadable_closes, closing_order, elisions) = closing;
        let final_wave_copies = self
            .closing_copies
            .saturating_sub(closing_copies_before_final_wave);

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
            closing_copies: self.closing_copies,
            final_wave_copies,
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
        let fresh = match &task.body {
            TaskBody::Op { op, addr, .. } => self.visited.insert_op(*addr, visit_tag(recipe, *op)),
            TaskBody::Extent { op, span, .. } => {
                self.visited
                    .insert_extent(span.start, span.len, visit_tag(recipe, *op))
            }
        };
        if !fresh {
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

    /// ownership index once for the whole batch. `end_batch` folds the batch's
    fn copy_wave(&mut self, tasks: &[CopyTask], wave: u32) -> Result<(), CustodyRefusal> {
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
        self.task_hint.clear();
        if self.hints_wanted {
            self.task_hint.resize(tasks.len(), DecodeHint::default());
        }

        let mut result = Ok(());
        for (recipe, indices) in by_recipe.iter().enumerate() {
            if indices.is_empty() {
                continue;
            }
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
            planned.keys_mut().clear();
            if let Err(custody) = Self::reserve_own(
                planned.keys_mut(),
                self.arena.ledger_ref(),
                &mut self.own_tables_charged,
                requests.len(),
            ) {
                result = Err(custody);
                break;
            }
            super::policy::plan_copies_into(&requests, &policy, &mut planned);

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
            let consumer = self.recipes[recipe].consumer;
            let mut cursor = self.owners.sweep();
            let mut custody = Ok(());
            'attach: for position in 0..planned.order().len() {
                let request = requests[planned.order()[position] as usize];
                let parent = tasks[request.task as usize].parent;
                self.owners
                    .sweep_overlapping(&mut cursor, request.span, &mut owned);
                if self.mid_walk
                    && self.mid_walk_reopened(recipe, &tasks[request.task as usize], &owned, wave)
                {
                    continue;
                }
                if self.annot.armed() {
                    self.annotate(recipe, &tasks[request.task as usize], request.span, &owned);
                    self.reconcile_samples(recipe, &tasks[request.task as usize], &owned);
                    if self.mid_walk {
                        custody = self.mid_walk_note(recipe, &tasks[request.task as usize], &owned);
                        if custody.is_err() {
                            break 'attach;
                        }
                    }
                }
                let mut first_owned = true;
                for id in owned.iter() {
                    custody = self.owners.add_dependent(*id, consumer, request.span);
                    if custody.is_err() {
                        break 'attach;
                    }
                    if first_owned && self.hints_wanted {
                        first_owned = false;
                        let slot = &mut self.task_hint[request.task as usize];
                        if slot.interval == NO_HINT {
                            let record = self.owners.record(*id);
                            *slot = DecodeHint {
                                interval: id.0,
                                consumer,
                                reach: self
                                    .owners
                                    .links()
                                    .reach_of(record, consumer)
                                    .unwrap_or(Span::new(0, 0)),
                                start: record.start(),
                                opening_offset: record.opening_offset() as u32,
                            };
                        }
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
        attributes.samples.on_disagree = DisagreeAction::Record;
        attributes.on_close = CloseAction::Record;
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
            if modifier.on_close == CloseAction::Gap {
                attributes.on_close = CloseAction::Gap;
            }
            attributes.op = op;
            attributes.stage = modifier.stage;
        }
        attributes
    }

    fn acquire(
        &mut self,
        cursor: &mut super::dedup::Cursor,
        span: Span,
        plan: &mut super::dedup::AcquisitionPlan,
        recipe: usize,
        attributes: CopyAttributes,
    ) -> Result<Result<(), Span>, CustodyRefusal> {
        self.owners.sweep_plan(cursor, span, plan);
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
            return Ok(None);
        }
        let id = self.owners.insert(hole, offset)?;
        if attributes.samples.count > 1 {
            self.take_adjacent_samples(id, hole, len, attributes)?;
        }
        Ok(Some(()))
    }

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
            let ledger = self.arena.ledger();
            super::arena::charged_reserve(&mut self.voids, &ledger, 1)?;
            super::arena::charged_reserve(&mut self.void_context, &ledger, 1)?;
            self.voids.push(super::Void {
                span: hole,
                recipe: self.recipes[recipe].id,
                witness,
            });
            self.void_context.push((recipe, request));
            self.states[recipe].scoped_voids = self.states[recipe].scoped_voids.saturating_add(1);
            return Ok(());
        }
        let granularity = super::SCOPED_VOID_GRANULARITY_BYTES;
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

    fn take_adjacent_samples(
        &mut self,
        id: IntervalId,
        hole: Span,
        len: usize,
        attributes: CopyAttributes,
    ) -> Result<(), CustodyRefusal> {
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
                break;
            }
            images.success_mask |= 1 << sample;
            images.extra.push(
                u32::try_from(offset)
                    .expect("an arena offset fits a u32 under the custody ceiling"),
            );
        }
        self.annot.set_samples(id, images);
        Ok(())
    }

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
        for &id in owned {
            self.annot
                .entry(id)
                .ask_samples(consumer, spec.count, wants_gap);
        }
    }

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
        let recipes = self.recipes;
        let modifier = recipes[recipe].op_mod(op);
        let stage = modifier.stage;
        let windows = modifier.compare.windows.as_slice();
        let consumer = recipes[recipe].consumer;
        let gap_on_close = modifier.on_close == CloseAction::Gap;

        let resolved = if windows.is_empty() {
            ResolvedMask::All
        } else {
            match mask_source {
                MaskSource::At(base) => {
                    let mut spans: Vec<Span> = Vec::with_capacity(windows.len());
                    for window in windows {
                        if let Some(resolved) = window.resolve(base) {
                            if resolved.overlaps(&span) {
                                spans.push(resolved);
                            }
                        }
                    }
                    if spans.is_empty() {
                        ResolvedMask::All
                    } else {
                        ResolvedMask::Spans(spans)
                    }
                }
                MaskSource::Strided(base, stride, count) => {
                    if stride == 0 || count == 0 {
                        ResolvedMask::All
                    } else {
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
                            ResolvedMask::All
                        } else {
                            ResolvedMask::Strided(super::annot::StridedMask {
                                base,
                                stride,
                                count,
                                windows: element_windows,
                            })
                        }
                    }
                }
            }
        };

        for &id in owned {
            let mut entry = self.annot.entry(id);
            entry.tag_stage(stage);
            entry.ask_close(consumer, gap_on_close);
            match &resolved {
                ResolvedMask::All => entry.consume_all(),
                ResolvedMask::Spans(spans) => entry.consume_spans(spans),
                ResolvedMask::Strided(mask) => entry.consume_strided(mask.clone()),
            }
        }
    }

    fn decode_wave(&mut self, tasks: &[CopyTask]) {
        let recipes = self.recipes;
        let mut stack = std::mem::take(&mut self.sites);
        for (position, task) in tasks.iter().enumerate() {
            if self.refused(task.recipe) {
                continue;
            }
            self.decode_hint.set(
                self.task_hint
                    .get(position)
                    .copied()
                    .unwrap_or_else(DecodeHint::default),
            );
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
                    if element.is_empty() {
                        self.states[task.recipe].frozen_branches =
                            self.states[task.recipe].frozen_branches.saturating_add(1);
                        continue;
                    }
                    let region = Proof::Extent {
                        base: span.start,
                        len: span.len,
                    };
                    let mut overflowed = false;
                    for index in (0..*count).rev() {
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
        let recipes = self.recipes;
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
                    None => {
                        return self.refuse_unrepresentable(recipe, site.base, i64::from(offset))
                    }
                };
                let (value, holder) = match self.read_uint(recipe, addr, 8) {
                    Some(read) => read,
                    None => {
                        let witness = RefusalWitness {
                            read_addr: addr,
                            read_width: 8,
                            ..self.witness_at(recipe, site)
                        };
                        self.field_not_captured(recipe, Span::new(addr, 8), witness);
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
                    carry: carry!(),
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
                    carry: carry!(),
                    body: TaskBody::Op {
                        op: next,
                        addr: site.base,
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
                if !self.guard_admits(recipe, id, site.base, guard) {
                    return;
                }
                let ptr_addr = match site.base.checked_add(u64::from(ptr_offset)) {
                    Some(addr) => addr,
                    None => {
                        return self.refuse_unrepresentable(
                            recipe,
                            site.base,
                            i64::from(ptr_offset),
                        )
                    }
                };
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
                        match self.read_uint(recipe, count_addr, count_width as usize) {
                            Some((value, _)) => value,
                            None => {
                                let witness = RefusalWitness {
                                    read_addr: count_addr,
                                    read_width: count_width,
                                    ..self.witness_at(recipe, site)
                                };
                                self.field_not_captured(
                                    recipe,
                                    Span::new(count_addr, u64::from(count_width)),
                                    witness,
                                );
                                return;
                            }
                        }
                    }
                };
                let (base, base_holder) = match self.read_uint(recipe, ptr_addr, 8) {
                    Some(read) => read,
                    None => {
                        let witness = RefusalWitness {
                            read_addr: ptr_addr,
                            read_width: 8,
                            ..self.witness_at(recipe, site)
                        };
                        self.field_not_captured(recipe, Span::new(ptr_addr, 8), witness);
                        return;
                    }
                };
                if base != 0 && !self.admit_pointer(recipe, site, base, ptr_addr, 8) {
                    return;
                }
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
                let carry = carry!();
                let instance = if self.recipes[recipe].op_mod(site.op).instance_from_carry {
                    match carry.get() {
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
                    carry,
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
                let (table, key_offset, key_width, next) = (*table, *key_offset, *key_width, *next);
                let modifier = self.recipes[recipe].op_mod(site.op);
                let (key_from_carry, anchor_on_carry_holder, instance_from_carry) = (
                    modifier.key_from_carry,
                    modifier.anchor_on_carry_holder,
                    modifier.instance_from_carry,
                );
                let carry = carry!();
                let (key, key_holder) = if key_from_carry {
                    let Some(value) = carry.get() else {
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
                    if anchor_on_carry_holder {
                        match carry.holder() {
                            Some(holder) => (value, holder),
                            None => {
                                let witness = RefusalWitness {
                                    raw: value,
                                    ..self.witness_at(recipe, site)
                                };
                                self.refuse_witnessed(
                                    recipe,
                                    Refusal::CarryHolderUnknown {
                                        recipe: id,
                                        op: site.op,
                                    },
                                    witness,
                                );
                                return;
                            }
                        }
                    } else {
                        match self.owners.find(site.base) {
                            Some(holder) => (value, holder),
                            None => {
                                let witness = RefusalWitness {
                                    raw: value,
                                    read_addr: site.base,
                                    ..self.witness_at(recipe, site)
                                };
                                self.field_not_captured(recipe, Span::new(site.base, 1), witness);
                                return;
                            }
                        }
                    }
                } else {
                    let key_addr = match site.base.checked_add(u64::from(key_offset)) {
                        Some(addr) => addr,
                        None => {
                            return self.refuse_unrepresentable(
                                recipe,
                                site.base,
                                i64::from(key_offset),
                            )
                        }
                    };
                    match self.read_uint(recipe, key_addr, key_width as usize) {
                        Some(read) => read,
                        None => {
                            let witness = RefusalWitness {
                                read_addr: key_addr,
                                read_width: key_width,
                                ..self.witness_at(recipe, site)
                            };
                            self.field_not_captured(
                                recipe,
                                Span::new(key_addr, u64::from(key_width)),
                                witness,
                            );
                            return;
                        }
                    }
                };
                let instance = if instance_from_carry {
                    match carry.get() {
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
                    carry,
                    body: TaskBody::Op {
                        op: next,
                        addr,
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
                        carry: carry!(),
                        index: site.index,
                    });
                }
            }
            RecipeOp::Freeze => {
                self.states[recipe].frozen_branches =
                    self.states[recipe].frozen_branches.saturating_add(1);
            }
            RecipeOp::CopyHeader { .. } | RecipeOp::CopyRanges { .. } => {
                debug_assert!(false, "copy op reached in a decode slot");
            }
        }
    }

    #[inline]
    fn resolve_carry(&mut self, recipe: usize, site: &DecodeSite) -> Result<CarrySlot, ()> {
        let Some(source) = self.recipes[recipe].op_mod(site.op).carry else {
            return Ok(site.carry);
        };
        let wants_holder = self.states[recipe].anchors_on_carry_holder;
        match source {
            CarrySource::Constant(value) => Ok(CarrySlot::with_holder(Some(value), None)),
            CarrySource::ElementIndex => {
                let holder = if wants_holder {
                    self.owners.find(site.base)
                } else {
                    None
                };
                Ok(CarrySlot::with_holder(Some(u64::from(site.index)), holder))
            }
            CarrySource::Field { offset, width } => {
                let addr = match site.base.checked_add(u64::from(offset)) {
                    Some(addr) => addr,
                    None => {
                        self.refuse_unrepresentable(recipe, site.base, i64::from(offset));
                        return Err(());
                    }
                };
                match self.read_uint(recipe, addr, width as usize) {
                    Some((value, holder)) => Ok(CarrySlot::with_holder(
                        Some(value),
                        if wants_holder { Some(holder) } else { None },
                    )),
                    None => {
                        let id = self.recipes[recipe].id;
                        let witness = RefusalWitness {
                            read_addr: addr,
                            read_width: width,
                            ..self.witness_at(recipe, site)
                        };
                        self.field_not_captured(recipe, Span::new(addr, u64::from(width)), witness);
                        Err(())
                    }
                }
            }
        }
    }

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
            let value = match self.read_uint(recipe, addr, term.width as usize) {
                Some((value, _)) => value,
                None => {
                    let witness = RefusalWitness {
                        read_addr: addr,
                        read_width: term.width,
                        ..self.witness_at(recipe, site)
                    };
                    self.field_not_captured(
                        recipe,
                        Span::new(addr, u64::from(term.width)),
                        witness,
                    );
                    return Err(());
                }
            };
            total += u128::from(value);
        }
        Ok(u64::try_from(total).unwrap_or(u64::MAX))
    }

    fn evaluate_probe(&self, recipe: usize, site: &DecodeSite, probe: &Probe) -> Option<bool> {
        match *probe {
            Probe::FieldEquals {
                offset,
                width,
                value,
            } => {
                let addr = site.base.checked_add(u64::from(offset))?;
                let (read, _) = self.read_uint(recipe, addr, width as usize)?;
                Some(read == value)
            }
            Probe::FieldInRange {
                offset,
                width,
                low,
                high,
            } => {
                let addr = site.base.checked_add(u64::from(offset))?;
                let (read, _) = self.read_uint(recipe, addr, width as usize)?;
                Some(read >= low && read <= high)
            }
            Probe::SamplesAgree { offset, len } => {
                let addr = site.base.checked_add(u64::from(offset))?;
                let interval = self
                    .owners
                    .find_hinted(addr, IntervalId(self.decode_hint.get().interval))?;
                let record = *self.owners.record(interval);
                let want = Span::new(addr, u64::from(len));
                if !record.span().contains_span(&want) {
                    return None;
                }
                // THE SAME OWNERSHIP LAW `read_uint` ENFORCES, and it has to be
                if !self
                    .owners
                    .links()
                    .reaches(&record, self.recipes[recipe].consumer, &want)
                {
                    return None;
                }
                let images = self.annot.samples(interval)?;
                let Some(&second) = images.extra.first() else {
                    return Some(false);
                };
                let skew = (addr - record.start()) as usize;
                let width = len as usize;
                let opening = record.opening_offset() + skew;
                let closing = second as usize + skew;
                Some(self.arena.slice(opening, width) == self.arena.slice(closing, width))
            }
        }
    }

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
        match self.read_uint(recipe, addr, guard.width as usize) {
            Some((value, _)) => guard.admits(value),
            None => {
                self.field_not_captured(
                    recipe,
                    Span::new(addr, u64::from(guard.width)),
                    RefusalWitness::default(),
                );
                false
            }
        }
    }

    /// # The ownership check, and why it is not optional
    /// The arena is one buffer and the ownership map is observation-wide, so
    /// [`OwnershipMap::find_hinted`]) and one walk of that interval's dependent
    fn read_uint(&self, recipe: usize, addr: u64, width: usize) -> Option<(u64, IntervalId)> {
        let consumer = self.recipes[recipe].consumer;
        let want = Span::new(addr, width as u64);
        let hint = self.decode_hint.get();
        let resolved = if hint.interval != NO_HINT
            && hint.consumer == consumer
            && hint.reach.contains_span(&want)
        {
            hint
        } else {
            let id = self.owners.find_hinted(addr, IntervalId(hint.interval))?;
            let record = self.owners.record(id);
            if !record.span().contains_span(&want) {
                return None;
            }
            let reach = self.owners.links().reach_of(record, consumer)?;
            if !reach.contains_span(&want) {
                return None;
            }
            let resolved = DecodeHint {
                interval: id.0,
                consumer,
                reach,
                start: record.start(),
                opening_offset: record.opening_offset() as u32,
            };
            self.decode_hint.set(resolved);
            resolved
        };
        let skew = (addr - resolved.start) as usize;
        let bytes = self
            .arena
            .slice(resolved.opening_offset as usize + skew, width);
        let mut raw = [0u8; 8];
        raw[..width].copy_from_slice(bytes);
        Some((u64::from_le_bytes(raw), IntervalId(resolved.interval)))
    }

    /// ownership index which intervals each one covers with a fresh bisect. On
    fn link_pruned(&mut self) -> Result<(), CustodyRefusal> {
        let pruned = std::mem::take(&mut self.pruned);
        let mut requests = std::mem::take(&mut self.requests);
        let mut order = std::mem::take(&mut self.pruned_order);
        let mut owned = std::mem::take(&mut self.owned);
        requests.clear();
        order.clear();
        let result = self.link_pruned_into(&pruned, &mut requests, &mut order, &mut owned);
        self.pruned = pruned;
        self.pruned.clear();
        self.requests = requests;
        self.pruned_order = order;
        self.owned = owned;
        result
    }

    fn link_pruned_into(
        &mut self,
        pruned: &[CopyTask],
        requests: &mut Vec<CopyRequest>,
        order: &mut Vec<PrunedKey>,
        owned: &mut Vec<IntervalId>,
    ) -> Result<(), CustodyRefusal> {
        let live = pruned
            .iter()
            .filter(|task| !self.refused(task.recipe))
            .count();
        Self::reserve_own(
            requests,
            self.arena.ledger_ref(),
            &mut self.own_tables_charged,
            live,
        )?;
        for (index, task) in pruned.iter().enumerate() {
            if self.refused(task.recipe) {
                continue;
            }
            let before = requests.len();
            if let Err((base, offset)) = self.requests_of(task, index as u32, requests) {
                requests.truncate(before);
                self.refuse_unrepresentable(task.recipe, base, offset);
            }
        }
        Self::reserve_own(
            order,
            self.arena.ledger_ref(),
            &mut self.own_tables_charged,
            requests.len(),
        )?;
        for (index, request) in requests.iter().enumerate() {
            let task = &pruned[request.task as usize];
            order.push(PrunedKey {
                start: request.span.start,
                len: request.span.len,
                recipe: task.recipe as u32,
                parent: task.parent.map_or(NO_HINT, |parent| parent.0),
                request: index as u32,
            });
        }
        order.sort_unstable();
        let mut cursor = self.owners.sweep();
        let mut previous: Option<PrunedKey> = None;
        for key in order.iter() {
            if previous.is_some_and(|held| held.same_ask(key)) {
                continue;
            }
            previous = Some(*key);
            let request = requests[key.request as usize];
            let consumer = self.recipes[key.recipe as usize].consumer;
            let parent = (key.parent != NO_HINT).then(|| IntervalId(key.parent));
            self.owners
                .sweep_overlapping(&mut cursor, request.span, owned);
            for id in owned.iter() {
                self.owners.add_dependent(*id, consumer, request.span)?;
                if let Some(parent) = parent {
                    self.owners.add_edge(parent, *id, consumer)?;
                }
            }
        }
        Ok(())
    }

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

    fn effective_stage(&self, id: IntervalId) -> Option<(usize, u16)> {
        let recipes = self.recipes;
        let stages = self.annot.stages_of(id);
        let mut best: Option<(usize, u16)> = None;
        for &stage in stages {
            let effective = match stage_entry(recipes, stage).and_then(|entry| entry.elide_into) {
                Some(target) if stages.contains(&target) => target,
                _ => stage,
            };
            if let Some(rank) = stage_rank(recipes, effective) {
                if best.map_or(true, |(held, _)| rank < held) {
                    best = Some((rank, effective));
                }
            }
        }
        best
    }

    fn fill_stage_ranks(
        &mut self,
        ids: impl Iterator<Item = IntervalId>,
    ) -> Result<(), CustodyRefusal> {
        let wanted = self.owners.len().saturating_sub(self.stage_rank_memo.len());
        Self::reserve_own(
            &mut self.stage_rank_memo,
            self.arena.ledger_ref(),
            &mut self.own_tables_charged,
            wanted,
        )?;
        self.stage_rank_memo
            .resize(self.owners.len(), (NO_CLOSING_RANK, 0));
        for id in ids {
            let found = self
                .effective_stage(id)
                .map(|(rank, stage)| (u32::try_from(rank).unwrap_or(NO_CLOSING_RANK - 1), stage))
                .unwrap_or((NO_CLOSING_RANK, 0));
            if let Some(slot) = self.stage_rank_memo.get_mut(id.0 as usize) {
                *slot = found;
            }
        }
        Ok(())
    }

    #[inline]
    fn filled_rank(&self, id: IntervalId) -> u32 {
        self.stage_rank_memo
            .get(id.0 as usize)
            .map(|(rank, _)| *rank)
            .unwrap_or(NO_CLOSING_RANK)
    }

    #[inline]
    fn filled_stage(&self, id: IntervalId) -> Option<u16> {
        match self.stage_rank_memo.get(id.0 as usize) {
            Some((rank, stage)) if *rank != NO_CLOSING_RANK => Some(*stage),
            _ => None,
        }
    }

    fn close_when(&self, stage: Option<u16>) -> CloseWhen {
        match stage {
            Some(stage) => stage_entry(self.recipes, stage)
                .map(|entry| entry.when)
                .unwrap_or(CloseWhen::Final),
            None => CloseWhen::Final,
        }
    }

    #[inline]
    fn mid_walk_closed_already(&self, id: IntervalId) -> bool {
        self.mid_walk_done
            .get(id.0 as usize)
            .copied()
            .unwrap_or(false)
    }

    /// there could never fire. `acquire`/`take_hole` only create intervals, and
    fn mid_walk_reopened(
        &mut self,
        recipe: usize,
        task: &CopyTask,
        owned: &[IntervalId],
        wave: u32,
    ) -> bool {
        let op = match &task.body {
            TaskBody::Op { op, .. } | TaskBody::Extent { op, .. } => *op,
        };
        let stage = self.recipes[recipe].op_mod(op).stage;
        let mut reopened = false;
        for &id in owned {
            if !self.mid_walk_closed_already(id) {
                continue;
            }
            let closed_after = self
                .mid_walk_closed
                .iter()
                .find(|(closed, _)| *closed == id)
                .map(|(_, at)| *at)
                .unwrap_or(wave);
            let span = self.owners.record(id).span();
            let refusal = Refusal::ClosedIntervalReopened {
                recipe: self.recipes[recipe].id,
                span,
                closed_after_wave: closed_after,
                reopened_in_wave: wave,
            };
            let witness = RefusalWitness {
                stage,
                op,
                subject: span.start,
                raw: span.len,
                ..RefusalWitness::default()
            };
            self.refuse_witnessed(recipe, refusal, witness);
            reopened = true;
        }
        reopened
    }

    fn mid_walk_note(
        &mut self,
        recipe: usize,
        task: &CopyTask,
        owned: &[IntervalId],
    ) -> Result<(), CustodyRefusal> {
        let op = match &task.body {
            TaskBody::Op { op, .. } | TaskBody::Extent { op, .. } => *op,
        };
        if self.recipes[recipe].op_mod(op).stage == 0 {
            return Ok(());
        }
        for &id in owned {
            super::arena::charged_push(
                &mut self.mid_walk_pending,
                self.arena.ledger_ref(),
                (id, recipe),
            )?;
        }
        Ok(())
    }

    fn mid_walk_close(&mut self, depth: u32) -> Result<(), CustodyRefusal> {
        let mut pending = std::mem::take(&mut self.mid_walk_pending);
        if pending.is_empty() {
            self.mid_walk_pending = pending;
            return Ok(());
        }
        if let Err(custody) = self.fill_stage_ranks(pending.iter().map(|(id, _)| *id)) {
            self.mid_walk_pending = pending;
            return Err(custody);
        }
        pending.sort_unstable_by_key(|(id, _)| {
            let rank = self.filled_rank(*id);
            (
                if rank == NO_CLOSING_RANK {
                    None
                } else {
                    Some(rank as usize)
                },
                id.0,
            )
        });
        let wanted = self.owners.len().saturating_sub(self.mid_walk_done.len());
        Self::reserve_own(
            &mut self.mid_walk_done,
            self.arena.ledger_ref(),
            &mut self.own_tables_charged,
            wanted,
        )?;
        self.mid_walk_done.resize(self.owners.len(), false);
        let ledger = self.arena.ledger();
        let mut result = Ok(());
        for position in 0..pending.len() {
            let (id, tagged_by) = pending[position];
            if self.mid_walk_closed_already(id) {
                continue;
            }
            match self.close_when(self.filled_stage(id)) {
                CloseWhen::AfterWave(wave) if wave == depth => {}
                CloseWhen::AfterWave(wave) if wave < depth => {
                    let span = self.owners.record(id).span();
                    let refusal = Refusal::ClosedIntervalReopened {
                        recipe: self.recipes[tagged_by].id,
                        span,
                        closed_after_wave: wave,
                        reopened_in_wave: depth,
                    };
                    let witness = RefusalWitness {
                        subject: span.start,
                        raw: span.len,
                        ..RefusalWitness::default()
                    };
                    self.refuse_witnessed(tagged_by, refusal, witness);
                    continue;
                }
                _ => continue,
            }
            if let Err(custody) = self.record_elisions(id, &ledger) {
                result = Err(custody);
                break;
            }
            if let Err(custody) = self.owners.enrol_witness(id) {
                result = Err(custody);
                break;
            }
            match self.close_one(id) {
                Ok(readable) => {
                    if !readable {
                        if let Err(custody) = super::arena::charged_push(
                            &mut self.unreadable_closes,
                            self.arena.ledger_ref(),
                            id,
                        ) {
                            result = Err(custody);
                            break;
                        }
                    }
                }
                Err(custody) => {
                    result = Err(custody);
                    break;
                }
            }
            self.mid_walk_done[id.0 as usize] = true;
            if let Err(custody) = super::arena::charged_push(
                &mut self.mid_walk_closed,
                self.arena.ledger_ref(),
                (id, depth),
            ) {
                result = Err(custody);
                break;
            }
        }
        pending.clear();
        self.mid_walk_pending = pending;
        result
    }

    fn record_elisions(
        &mut self,
        id: IntervalId,
        ledger: &super::arena::CustodyLedger,
    ) -> Result<(), CustodyRefusal> {
        let recipes = self.recipes;
        let stages = self.annot.stages_of(id);
        for position in 0..stages.len() {
            let stage = self.annot.stages_of(id)[position];
            let Some(target) = stage_entry(recipes, stage).and_then(|entry| entry.elide_into)
            else {
                continue;
            };
            if !self.annot.stages_of(id).contains(&target) {
                continue;
            }
            if self
                .elisions
                .iter()
                .any(|e| e.interval == id && e.elided == stage)
            {
                continue;
            }
            super::arena::charged_push(
                &mut self.elisions,
                ledger,
                Elision {
                    interval: id,
                    elided: stage,
                    into: target,
                },
            )?;
        }
        Ok(())
    }

    fn close_one(&mut self, id: IntervalId) -> Result<bool, CustodyRefusal> {
        let span = self.owners.record(id).span();
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
        self.closing_copies = self.closing_copies.saturating_add(1);
        if memory
            .copy_into(span.start, self.arena.slice_mut(offset, len))
            .is_err()
        {
            return Ok(false);
        }
        self.owners.record_mut(id).set_closing_offset(offset);
        let wanted = self.annot.closing_images_wanted(id);
        for _ in 1..wanted {
            let extra = self.arena.alloc_zeroed(span.len)?;
            self.closing_copies = self.closing_copies.saturating_add(1);
            if memory
                .copy_into(span.start, self.arena.slice_mut(extra, len))
                .is_err()
            {
                break;
            }
            self.annot.push_closing_image(id, extra as u32);
        }
        Ok(true)
    }

    /// history. Two vectors were an unreleased phantom charge for storage the
    fn closing_schedule(&mut self) -> Result<Option<(Vec<IntervalId>, usize)>, CustodyRefusal> {
        if self.recipes.iter().all(|recipe| recipe.closing.is_none()) {
            return Ok(None);
        }
        let ledger = self.arena.ledger();
        let mut order: Vec<IntervalId> = Vec::new();
        super::arena::charged_reserve(&mut order, &ledger, self.owners.witnessed_ids().len())?;
        order.extend(self.mid_walk_closed.iter().map(|(id, _)| *id));
        let first_unclosed = order.len();
        for &id in self.owners.witnessed_ids() {
            if !self.mid_walk_closed_already(id) {
                order.push(id);
            }
        }

        for position in first_unclosed..order.len() {
            self.record_elisions(order[position], &ledger)?;
        }
        self.fill_stage_ranks(order[first_unclosed..].iter().copied())?;
        order[first_unclosed..].sort_unstable_by_key(|id| (self.filled_rank(*id), id.0));
        Ok(Some((order, first_unclosed)))
    }

    fn closing_wave(
        &mut self,
    ) -> Result<(u32, Vec<IntervalId>, Vec<IntervalId>, Vec<Elision>), CustodyRefusal> {
        // list. The clone was there only to release the borrow before
        let schedule = self.closing_schedule()?;
        let (start, total) = match &schedule {
            Some((order, first_unclosed)) => (*first_unclosed, order.len()),
            None => (0, self.owners.witnessed_ids().len()),
        };
        for position in start..total {
            let id = match &schedule {
                Some((order, _)) => order[position],
                None => self.owners.witnessed_ids()[position],
            };
            if !self.close_one(id)? {
                super::arena::charged_push(
                    &mut self.unreadable_closes,
                    self.arena.ledger_ref(),
                    id,
                )?;
            }
        }
        Ok((
            total as u32,
            std::mem::take(&mut self.unreadable_closes),
            schedule.map(|(order, _)| order).unwrap_or_default(),
            std::mem::take(&mut self.elisions),
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

    pub(super) fn groups_checksum(&self) -> u64 {
        let mut sum = 0u64;
        for group in &self.observation.by_recipe {
            for index in group {
                sum = sum.wrapping_mul(31).wrapping_add(u64::from(*index));
            }
        }
        sum
    }

    pub(super) fn value_sizes() -> [(&'static str, usize); 4] {
        [
            ("CopyTask", std::mem::size_of::<CopyTask>()),
            ("TaskBody", std::mem::size_of::<TaskBody>()),
            ("CopyRequest", std::mem::size_of::<CopyRequest>()),
            ("DecodeSite", std::mem::size_of::<DecodeSite>()),
        ]
    }

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

    pub(super) fn take_wave(&mut self, depth: u32) -> Vec<CopyTask> {
        std::mem::take(&mut self.observation.waves[depth as usize])
    }

    pub(super) fn put_wave(&mut self, depth: u32, tasks: Vec<CopyTask>) {
        self.observation.waves[depth as usize] = tasks;
    }

    pub(super) fn reserve_root_queue(&mut self) -> Result<(), CustodyRefusal> {
        self.observation.reserve_root_queue()
    }

    pub(super) fn validate_recipes(&mut self) {
        for (index, recipe) in self.observation.recipes.iter().enumerate() {
            if let Err(refusal) = validate(recipe) {
                self.observation.states[index].refusal = Some(RefusalRecord::bare(refusal));
            }
        }
    }

    pub(super) fn group_pass(&mut self, tasks: &[CopyTask]) -> usize {
        let observation = &mut self.observation;
        for group in observation.by_recipe.iter_mut() {
            group.clear();
        }
        observation
            .by_recipe
            .resize_with(observation.recipes.len(), Vec::new);
        for (index, task) in tasks.iter().enumerate() {
            observation.by_recipe[task.recipe].push(index as u32);
        }
        observation.by_recipe.iter().map(|group| group.len()).sum()
    }

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

    pub(super) fn copy_pass(&mut self, tasks: &[CopyTask]) -> Result<(), CustodyRefusal> {
        self.observation.copy_wave(tasks, 0)?;
        self.observation.owners.end_wave()
    }

    pub(super) fn decode_pass(&mut self, tasks: &[CopyTask]) {
        self.observation.decode_wave(tasks);
    }

    pub(super) fn queued_at(&self, depth: u32) -> usize {
        self.observation.waves[depth as usize].len()
    }

    pub(super) fn frozen_branches(&self) -> u32 {
        self.observation
            .states
            .iter()
            .map(|state| state.frozen_branches)
            .sum()
    }

    pub(super) fn charge_tables(&mut self) -> Result<(), CustodyRefusal> {
        self.observation.charge_own_tables(0)
    }

    pub(super) fn charge_tables_holding(
        &mut self,
        tasks: &Vec<CopyTask>,
    ) -> Result<(), CustodyRefusal> {
        self.observation.charge_own_tables_holding(tasks)
    }

    pub(super) fn own_tables_bytes(&self, in_flight: u64) -> u64 {
        self.observation.own_tables_bytes(in_flight)
    }

    pub(super) fn wave_bytes(tasks: &Vec<CopyTask>) -> u64 {
        super::arena::vec_bytes(tasks)
    }

    pub(super) fn ledger_table_bytes(&self) -> u64 {
        self.observation.arena.ledger_ref().table_bytes()
    }

    pub(super) fn own_tables_charged(&self) -> u64 {
        self.observation.own_tables_charged
    }

    pub(super) fn wave_capacity(&self, depth: u32) -> usize {
        self.observation.waves[depth as usize].capacity()
    }
}
