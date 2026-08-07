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
//! batch, then run every depth-`d` decode op, which fills wave `d + 1`. t35
//! measured a maximum discovery depth of 12, so there are
//! [`FORWARD_WAVES`](super::FORWARD_WAVES) = 13 forward waves and one closing
//! witness wave: [`TOTAL_PHASES`](super::TOTAL_PHASES) = 14 copy-submission
//! phases, not 24 and not 12 × 68.
//!
//! A raw 12-hop dependent chain is 1–2 µs of DRAM latency. The cost is the 13
//! synchronisation rounds and the BREADTH of each one, which is why everything
//! ready at a depth is batched across every recipe before anything at the next
//! depth is looked at.
//!
//! # Refusals
//!
//! Per-recipe limits produce per-recipe [`Refusal`]s: that recipe's consumer
//! is gapped and every other recipe in the observation is untouched. Custody
//! failure is the one global refusal — it aborts the observation, because an
//! arena that could not be grown cannot be made smaller by dropping a recipe
//! that already copied into it.

use std::collections::{BTreeMap, BTreeSet};

use super::arena::Arena;
use super::dedup::OwnershipMap;
use super::policy::{AnchorKind, CopyRequest, GapPolicy, Proof, Provenance};
use super::view::Snapshot;
use super::{
    ConsumerId, CustodyRefusal, GapReason, HourStamp, IntervalId, Limits, MalformedReason,
    RecipeId, Refusal, SnapshotMemory, Span, TableId, DEFAULT_CUSTODY_BUDGET_BYTES, FORWARD_WAVES,
    MEASURED_MAX_DISCOVERY_DEPTH,
};

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
    DecodePointer { offset: u32, next: u16 },
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
        /// Register the resulting array so `JoinBounded` can index it.
        register: Option<TableId>,
        /// Decode ops to run against EACH element. They run in the same wave
        /// the array lands in: an element is not one more dereference away.
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
    if recipe.limits.max_depth > MEASURED_MAX_DISCOVERY_DEPTH {
        return Err(bad(MalformedReason::DepthLimitAboveMeasuredMaximum {
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
            RecipeOp::DecodePointer { next, .. } => expect(*next, OpKind::Copy)?,
            RecipeOp::DecodeExtent {
                count_width,
                stride,
                element,
                max_count,
                max_bytes,
                ..
            } => {
                if !matches!(count_width, 1 | 2 | 4 | 8) {
                    return Err(bad(MalformedReason::BadWidth {
                        op: here,
                        width: *count_width,
                    }));
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
                if element.is_empty() {
                    return Err(bad(MalformedReason::DanglingBranch { op: here }));
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
    Observation::new(recipes, memory, config, arena).run()
}

#[derive(Clone, Copy, Debug)]
struct TableBinding {
    span: Span,
    count: u32,
    stride: u32,
}

#[derive(Clone, Debug)]
enum TaskBody {
    /// A copy op acting at `addr`.
    Op {
        op: u16,
        addr: u64,
        anchor: AnchorKind,
        /// What is proven about the region the address sits in, inherited from
        /// whatever produced it. `None` for a pointer target: a pointee's
        /// allocation is not proven by the pointer.
        inherited: Proof,
    },
    /// A resolved element array from a `DecodeExtent`.
    Extent { op: u16, span: Span, count: u32 },
}

#[derive(Clone, Debug)]
struct CopyTask {
    recipe: usize,
    depth: u32,
    /// The interval whose bytes produced this task's address.
    parent: Option<IntervalId>,
    body: TaskBody,
}

/// A decode op about to run against already-copied bytes.
#[derive(Clone, Debug)]
struct DecodeSite {
    op: u16,
    base: u64,
    /// The proven region `base` sits in.
    region: Proof,
    parent: Option<IntervalId>,
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
    visited: BTreeSet<(usize, u16, u64, u64)>,
    /// Tasks pruned as duplicates. Their dependency edges are still real, so
    /// they are replayed for linkage only once every interval exists.
    pruned: Vec<CopyTask>,
    waves: Vec<Vec<CopyTask>>,
    forward_waves_run: u32,
    // Scratch, owned by the observation and reused across waves. A fresh
    // allocation per task or per copy transaction would be a heap allocation
    // inside the bracket, which is the thing being priced.
    by_recipe: BTreeMap<usize, Vec<u32>>,
    requests: Vec<CopyRequest>,
    plan: super::policy::CopyPlan,
    acquisition: super::dedup::AcquisitionPlan,
    owned: Vec<IntervalId>,
    sites: Vec<DecodeSite>,
}

impl<'a, M: SnapshotMemory> Observation<'a, M> {
    fn new(
        recipes: &'a [Recipe],
        memory: &'a M,
        config: ObservationConfig,
        arena: Arena,
    ) -> Self {
        Self {
            recipes,
            memory,
            config,
            arena,
            owners: OwnershipMap::new(),
            states: recipes.iter().map(|_| RecipeState::new()).collect(),
            tables: BTreeMap::new(),
            visited: BTreeSet::new(),
            pruned: Vec::new(),
            waves: (0..FORWARD_WAVES + 1).map(|_| Vec::new()).collect(),
            forward_waves_run: 0,
            by_recipe: BTreeMap::new(),
            requests: Vec::new(),
            plan: super::policy::CopyPlan::new(),
            acquisition: super::dedup::AcquisitionPlan::default(),
            owned: Vec::new(),
            sites: Vec::new(),
        }
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

    fn run(mut self) -> Result<SnapshotBuild, CustodyRefusal> {
        for (index, recipe) in self.recipes.iter().enumerate() {
            if let Err(refusal) = validate(recipe) {
                self.states[index].refusal = Some(refusal);
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
                        anchor: AnchorKind::Root(root.addr),
                        inherited: Proof::None,
                    },
                };
                self.enqueue(task);
            }
        }

        for depth in 0..FORWARD_WAVES {
            let tasks = std::mem::take(&mut self.waves[depth as usize]);
            if tasks.is_empty() {
                continue;
            }
            self.forward_waves_run += 1;
            self.copy_wave(&tasks)?;
            self.decode_wave(&tasks);
        }

        // Anything still queued at depth FORWARD_WAVES was enqueued by the
        // last forward wave. `enqueue` refuses past `max_depth`, and
        // `max_depth` cannot exceed the measured maximum, so this is empty in
        // every accepted program; it is asserted rather than assumed.
        debug_assert!(self.waves[FORWARD_WAVES as usize].is_empty());

        self.link_pruned();
        self.enrol_witnesses();
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

        let snapshot = Snapshot::freeze(
            self.config.hour,
            self.arena.into_bytes(),
            self.owners.into_records(),
            gapped,
        );

        Ok(SnapshotBuild {
            snapshot,
            reports,
            forward_waves_run: self.forward_waves_run,
            phases_run,
            witnessed_intervals: witnessed,
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
        // For a copy op the spans follow from `(op, addr)`, so the fourth
        // element is unused. For an extent they do not: two container headers
        // can invoke one `DecodeExtent` against the same array base and decode
        // different counts, and keying on the base alone prunes the longer one
        // as a duplicate of the shorter — copying a prefix and reporting
        // `Captured`.
        let key = match &task.body {
            TaskBody::Op { op, addr, .. } => (recipe, *op, *addr, 0u64),
            TaskBody::Extent { op, span, .. } => (recipe, *op, span.start, span.len),
        };
        if !self.visited.insert(key) {
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
                RecipeOp::CopyHeader { size, .. } => push(
                    Span::new(*addr, u64::from(*size)),
                    Provenance::new(
                        *anchor,
                        Proof::Object {
                            base: *addr,
                            len: u64::from(*size),
                        },
                    ),
                ),
                RecipeOp::CopyRanges { windows, .. } => {
                    for window in windows {
                        match window.resolve(*addr) {
                            Some(span) => push(span, Provenance::new(*anchor, *inherited)),
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
    fn copy_wave(&mut self, tasks: &[CopyTask]) -> Result<(), CustodyRefusal> {
        let mut by_recipe = std::mem::take(&mut self.by_recipe);
        by_recipe.clear();
        for (index, task) in tasks.iter().enumerate() {
            by_recipe.entry(task.recipe).or_default().push(index as u32);
        }
        let mut requests = std::mem::take(&mut self.requests);
        let mut planned = std::mem::take(&mut self.plan);
        let mut acquisition = std::mem::take(&mut self.acquisition);
        let mut owned = std::mem::take(&mut self.owned);

        let mut result = Ok(());
        for (recipe, indices) in by_recipe.iter() {
            let recipe = *recipe;
            if self.refused(recipe) {
                continue;
            }
            let id = self.recipes[recipe].id;
            let limits = self.recipes[recipe].limits;
            let policy = self.recipes[recipe].policy;

            requests.clear();
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
            for index in 0..planned.copies.len() {
                let span = planned.copies[index].span;
                match self.acquire(span, &mut acquisition) {
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
            let consumer = self.recipes[recipe].consumer;
            for request in requests.iter() {
                let parent = tasks[request.task as usize].parent;
                self.owners.overlapping_into(request.span, &mut owned);
                for id in owned.iter() {
                    self.owners.add_dependent(*id, consumer, request.span);
                    if let Some(parent) = parent {
                        self.owners.add_edge(parent, *id, consumer);
                    }
                }
            }
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
        span: Span,
        plan: &mut super::dedup::AcquisitionPlan,
    ) -> Result<Result<(), Span>, CustodyRefusal> {
        self.owners.plan_into(span, plan);
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
            self.owners.insert(hole, offset);
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
                    let parent = self.owners.find(*addr);
                    for op in next.iter().rev() {
                        stack.push(DecodeSite {
                            op: *op,
                            base: *addr,
                            region,
                            parent,
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
                                parent: self.owners.find(base),
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
        match &recipes[recipe].ops[site.op as usize] {
            RecipeOp::DecodePointer { offset, next } => {
                let (offset, next) = (*offset, *next);
                let addr = match site.base.checked_add(u64::from(offset)) {
                    Some(addr) => addr,
                    None => return self.refuse_unrepresentable(recipe, site.base, i64::from(offset)),
                };
                let value = match self.read_uint(addr, 8) {
                    Some(value) => value,
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
                    parent: self.owners.find(addr),
                    body: TaskBody::Op {
                        op: next,
                        addr: value,
                        anchor: AnchorKind::Pointer(value),
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
                    Some(value) => value,
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
                let base = match self.read_uint(ptr_addr, 8) {
                    Some(value) => value,
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
                if raw_count > u64::from(max_count) {
                    self.refuse(
                        recipe,
                        Refusal::ExtentCountExceeded {
                            recipe: id,
                            limit: max_count,
                            // Saturating only for a count so absurd it is
                            // already far past any limit worth reporting
                            // exactly.
                            decoded: u32::try_from(raw_count).unwrap_or(u32::MAX),
                        },
                    );
                    return;
                }
                let count = raw_count as u32;
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
                if let Some(table) = register {
                    self.tables
                        .insert((recipe, table), TableBinding { span, count, stride });
                }
                let task = CopyTask {
                    recipe,
                    depth: depth + 1,
                    parent: self.owners.find(ptr_addr),
                    body: TaskBody::Extent {
                        op: site.op,
                        span,
                        count,
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
                let key = match self.read_uint(key_addr, key_width as usize) {
                    Some(value) => value,
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
                    parent: self.owners.find(key_addr),
                    body: TaskBody::Op {
                        op: next,
                        addr,
                        // An address produced by bounded index arithmetic into
                        // a PROVEN extent is a discovered object address, not a
                        // static one, so it is a legitimate window anchor. The
                        // extent proof travels with it.
                        anchor: AnchorKind::Pointer(addr),
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
                        parent: site.parent,
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

    /// Read an unsigned little-endian integer out of the arena.
    fn read_uint(&self, addr: u64, width: usize) -> Option<u64> {
        let bytes = self.owners.read(&self.arena, addr, width)?;
        let mut raw = [0u8; 8];
        raw[..width].copy_from_slice(bytes);
        Some(u64::from_le_bytes(raw))
    }

    /// Attach the dependency edges of tasks that deduplication pruned.
    fn link_pruned(&mut self) {
        let pruned = std::mem::take(&mut self.pruned);
        let mut requests = std::mem::take(&mut self.requests);
        let mut owned = std::mem::take(&mut self.owned);
        for task in pruned {
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
            for index in 0..requests.len() {
                let span = requests[index].span;
                self.owners.overlapping_into(span, &mut owned);
                for id in owned.iter() {
                    self.owners.add_dependent(*id, consumer, span);
                    if let Some(parent) = task.parent {
                        self.owners.add_edge(parent, *id, consumer);
                    }
                }
            }
        }
        self.requests = requests;
        self.owned = owned;
    }

    /// An interval is witnessed when any consumer depending on it asked for a
    /// witness. Sharing therefore widens the closing wave, never narrows it.
    fn enrol_witnesses(&mut self) {
        let wanted: BTreeSet<ConsumerId> = self
            .recipes
            .iter()
            .zip(self.states.iter())
            .filter(|(_, state)| state.witness_required && state.refusal.is_none())
            .map(|(recipe, _)| recipe.consumer)
            .collect();
        if wanted.is_empty() {
            return;
        }
        let ids: Vec<IntervalId> = self
            .owners
            .records()
            .iter()
            .filter(|record| record.consumers().any(|c| wanted.contains(&c)))
            .map(|record| record.id)
            .collect();
        for id in ids {
            self.owners.enrol_witness(id);
        }
    }

    /// The fourteenth phase: re-copy the opening addresses.
    ///
    /// This does NOT re-run discovery. A closing witness re-copies what the
    /// opening wave already found; a parent that changed invalidates its
    /// dependent subtree afterwards, which is cheaper than a reverse chase and
    /// strictly more conservative.
    fn closing_wave(&mut self) -> Result<(u32, Vec<IntervalId>), CustodyRefusal> {
        let ids: Vec<IntervalId> = self
            .owners
            .records()
            .iter()
            .filter(|record| record.witnessed)
            .map(|record| record.id)
            .collect();
        let mut unreadable = Vec::new();
        for id in &ids {
            let span = self.owners.record(*id).span;
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
                unreadable.push(*id);
                continue;
            }
            self.owners.record_mut(*id).closing_offset = Some(offset);
        }
        Ok((ids.len() as u32, unreadable))
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
