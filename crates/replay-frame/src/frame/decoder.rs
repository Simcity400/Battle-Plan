use super::{
    DecodedFrame, SharedView, TABLE_COUNT, TableUpdate, TableView, validate_applied,
    validate_complete, validate_delta, validate_rows, validate_singleton,
};
use crate::{Hour, codec::invalid, rows::TableId};
use engine_allocation as allocation;
use std::{io, sync::Arc};

#[cfg(test)]
mod admission_tests;
mod resume;

pub struct FrameAdvance {
    branch: crate::BranchId,
    hour: Hour,
    previous_hour: Hour,
    metrics: super::Metrics,
    tables: [Option<SharedView>; TABLE_COUNT],
    pending: [bool; TABLE_COUNT],
}

impl FrameAdvance {
    fn validate(
        previous: Option<&DecodedFrame>,
        frame: &(impl super::FrameSource + ?Sized),
    ) -> io::Result<()> {
        if previous
            .is_some_and(|current| current.branch == frame.branch() && current.hour >= frame.hour())
        {
            return Err(invalid("frame hour did not advance on its branch"));
        }
        if frame.keyframe() && !(0..TABLE_COUNT).all(|index| keyframe_update(frame.update(index))) {
            return Err(invalid("keyframe contains a dependent or partial table"));
        }
        Ok(())
    }
    pub fn begin(
        previous: Option<DecodedFrame>,
        frame: &(impl super::FrameSource + ?Sized),
        selected: &[bool; TABLE_COUNT],
    ) -> io::Result<Self> {
        Self::validate(previous.as_ref(), frame)?;
        let continues = previous
            .as_ref()
            .is_some_and(|current| !frame.keyframe() && current.branch == frame.branch());
        let (previous_hour, tables) = match previous {
            Some(frame) => (frame.hour, frame.tables.map(Some)),
            None => (frame.hour(), std::array::from_fn(|_| None)),
        };
        Self::from_tables(previous_hour, tables, continues, frame, selected)
    }

    fn begin_shared(
        previous: Option<Arc<DecodedFrame>>,
        frame: &(impl super::FrameSource + ?Sized),
        selected: &[bool; TABLE_COUNT],
    ) -> io::Result<Self> {
        Self::validate(previous.as_deref(), frame)?;
        let continues = previous
            .as_ref()
            .is_some_and(|current| !frame.keyframe() && current.branch == frame.branch());
        let (previous_hour, tables) = match previous {
            Some(previous) => {
                let (point, tables) = super::split_shared_tables(previous);
                (point.hour, tables.map(Some))
            }
            None => (frame.hour(), std::array::from_fn(|_| None)),
        };
        Self::from_tables(previous_hour, tables, continues, frame, selected)
    }

    fn from_tables(
        previous_hour: Hour,
        mut tables: [Option<SharedView>; TABLE_COUNT],
        continues: bool,
        frame: &(impl super::FrameSource + ?Sized),
        selected: &[bool; TABLE_COUNT],
    ) -> io::Result<Self> {
        for (index, view) in tables.iter_mut().enumerate() {
            if selected[index] {
                if !continues {
                    *view = None;
                }
            } else if view.is_none() {
                *view = Some(not_selected());
            }
        }
        Ok(Self {
            branch: frame.branch(),
            hour: frame.hour(),
            previous_hour,
            metrics: frame.metrics().clone(),
            tables,
            pending: *selected,
        })
    }

    pub fn previous(&self, table: TableId) -> Option<(Hour, &SharedView)> {
        self.pending[table.index()]
            .then(|| {
                self.tables[table.index()]
                    .as_ref()
                    .map(|view| (self.previous_hour, view))
            })
            .flatten()
    }

    pub fn take_previous(&mut self, table: TableId) -> Option<(Hour, SharedView)> {
        self.tables[table.index()]
            .take()
            .filter(|view| matches!(**view, TableView::Complete(_)))
            .map(|view| (self.previous_hour, view))
    }

    pub fn install(&mut self, table: TableId, view: SharedView) {
        self.tables[table.index()] = Some(view);
        self.pending[table.index()] = false;
    }

    pub fn reached(&self, table: TableId) -> Option<&SharedView> {
        (!self.pending[table.index()])
            .then(|| self.tables[table.index()].as_ref())
            .flatten()
    }

    pub fn finish(self) -> io::Result<DecodedFrame> {
        if self.pending.iter().any(|pending| *pending) {
            return Err(invalid("hour has unapplied tables"));
        }
        Ok(DecodedFrame {
            branch: self.branch,
            hour: self.hour,
            metrics: self.metrics,
            tables: self.tables.map(|view| view.expect("all tables applied")),
        })
    }
}

#[derive(Clone, Default)]
pub struct FrameDecoder {
    current: Option<Arc<DecodedFrame>>,
    selected: Option<[bool; TABLE_COUNT]>,
    change_budget: Option<allocation::credit::MemoryBudget>,
    spares: Spares,
}

impl FrameDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Reconstruct only these tables, retaining the normal wire validation and
    /// baseline rules. Omitted tables are explicitly unavailable, never empty.
    pub fn for_tables(tables: &[TableId]) -> Self {
        let mut selected = [false; TABLE_COUNT];
        for table in tables {
            selected[table.index()] = true;
        }
        Self {
            current: None,
            selected: Some(selected),
            change_budget: None,
            spares: Spares::default(),
        }
    }

    /// Emit source-bound changed keys and field masks for the shared display driver.
    pub fn with_change_facts(mut self, budget: allocation::credit::MemoryBudget) -> Self {
        self.change_budget = Some(budget);
        self
    }

    /// Share the decoder's output without copying a frame or its table directory.
    pub fn shared_current(&self) -> Option<&Arc<DecodedFrame>> {
        self.current.as_ref()
    }

    /// Decode an hour and return a frame callers can retain. It shares every
    /// table with the decoder, so it copies no rows; a table is copied only
    /// when either side writes to it.
    pub fn apply(
        &mut self,
        frame: &(impl super::FrameSource + ?Sized),
    ) -> io::Result<DecodedFrame> {
        self.advance(frame).cloned()
    }

    /// Advance the same decoder without copying its rows into a separate output.
    /// The borrowed hour remains available for validation before the next advance.
    pub fn advance(
        &mut self,
        frame: &(impl super::FrameSource + ?Sized),
    ) -> io::Result<&DecodedFrame> {
        self.advance_inner(frame, None)
    }

    /// Resource refusal or cancellation leaves the baseline available for retry.
    pub fn advance_cancellable(
        &mut self,
        frame: &(impl super::FrameSource + ?Sized),
        cancelled: &dyn Fn() -> bool,
    ) -> io::Result<&DecodedFrame> {
        self.advance_inner(frame, Some(cancelled))
    }

    fn advance_inner(
        &mut self,
        frame: &(impl super::FrameSource + ?Sized),
        cancelled: Option<&dyn Fn() -> bool>,
    ) -> io::Result<&DecodedFrame> {
        if cancelled.is_some_and(|cancelled| cancelled()) {
            return Err(io::Error::from(io::ErrorKind::Interrupted));
        }
        let _cpu = crate::cpu::scope("reconstruct", None);
        let measured = crate::cpu::enabled();
        let selected = self.selected.unwrap_or([true; TABLE_COUNT]);
        FrameAdvance::validate(self.current.as_deref(), frame)?;
        // Reserve change facts before taking baselines so backpressure is retryable.
        let mut changes: [Option<std::sync::Arc<super::ChangeFacts>>; TABLE_COUNT] =
            std::array::from_fn(|_| None);
        let mut stamps: [Option<std::sync::Arc<super::FieldStamp>>; TABLE_COUNT] =
            std::array::from_fn(|_| None);
        if let Some(budget) = &self.change_budget {
            let interrupted = self.current.as_ref().is_some_and(|current| {
                current.branch == frame.branch()
                    && !frame.keyframe()
                    && current.hour.checked_add(1) != Some(frame.hour())
            });
            let replaced = std::array::from_fn(|index| {
                selected[index]
                    && (interrupted
                        || !matches!(frame.update(index), TableUpdate::Unchanged { .. }))
            });
            reclaim_changes(&mut self.current, &replaced);
            let current = self
                .current
                .as_ref()
                .filter(|current| !frame.keyframe() && current.branch == frame.branch());
            let baseline = |index: usize, hour: Hour| {
                current
                    .filter(|current| current.hour == hour)
                    .map(|current| &current.tables[index])
                    .filter(|view| matches!(&***view, TableView::Complete(_)))
            };
            let mut required = 0;
            for index in (0..TABLE_COUNT).filter(|&index| selected[index]) {
                let bytes = match frame.update(index) {
                    TableUpdate::Delta { .. } | TableUpdate::Unchanged { .. } if interrupted => {
                        stamp_bytes()
                    }
                    TableUpdate::Snapshot(_) => stamp_bytes(),
                    TableUpdate::Delta {
                        baseline: hour,
                        changed,
                        deleted,
                    } if baseline(index, *hour).is_some() => {
                        allocation::add(change_bytes(changed.len(), deleted.len())?, stamp_bytes())?
                    }
                    _ => 0,
                };
                required = allocation::add(required, bytes)?;
            }
            let mut credit = change_credit(budget, required, cancelled)?;
            for index in (0..TABLE_COUNT).filter(|&index| selected[index]) {
                match frame.update(index) {
                    TableUpdate::Delta { .. } | TableUpdate::Unchanged { .. } if interrupted => {
                        stamps[index] = Some(field_stamp(
                            None,
                            None,
                            frame.hour(),
                            credit.split_off(stamp_bytes()),
                        )?);
                    }
                    TableUpdate::Snapshot(_) => {
                        stamps[index] = Some(field_stamp(
                            None,
                            None,
                            frame.hour(),
                            credit.split_off(stamp_bytes()),
                        )?)
                    }
                    TableUpdate::Delta {
                        baseline,
                        changed,
                        deleted,
                    } => {
                        if let Some(previous) = current
                            .filter(|current| *baseline == current.hour)
                            .map(|current| &current.tables[index])
                            .filter(|previous| matches!(&***previous, TableView::Complete(_)))
                        {
                            let facts = change_facts(
                                TableId::ALL[index],
                                previous,
                                changed,
                                deleted,
                                credit.split_off(change_bytes(changed.len(), deleted.len())?),
                            )?;
                            stamps[index] = Some(field_stamp(
                                previous.3.as_deref(),
                                Some(&facts),
                                frame.hour(),
                                credit.split_off(stamp_bytes()),
                            )?);
                            changes[index] = Some(std::sync::Arc::new(facts));
                        }
                    }
                    _ => {}
                }
            }
        }
        self.spares.reserve()?;
        let mut advancing = FrameAdvance::begin_shared(self.current.take(), frame, &selected)?;
        let mut spent = Vec::new();
        for table in TableId::ALL {
            let index = table.index();
            if !selected[index] {
                continue;
            }
            let started = if measured { crate::cpu::stamp() } else { 0 };
            let previous = advancing.take_previous(table);
            let mut view = advance_public_table_reusing(
                table,
                frame.update(index),
                frame.shared_update(index),
                previous,
                self.spares.slot(index),
                frame.hour(),
                frame.metrics().table_ns[index] != 0,
            )?;
            if matches!(&*view, TableView::Complete(_)) {
                if let Some(stamp) = stamps[index].take() {
                    view.2 = changes[index].take();
                    view.3 = Some(stamp);
                }
            }
            advancing.install(table, view);
            if measured {
                spent.push((index, crate::cpu::stamp().wrapping_sub(started)));
            }
        }
        if measured {
            crate::cpu::reconstructed(frame.hour(), &spent);
        }
        self.current = Some(Arc::new(advancing.finish()?));
        Ok(self.current.as_deref().unwrap())
    }
}

pub(super) fn reclaim_changes(
    current: &mut Option<Arc<DecodedFrame>>,
    replaced: &[bool; TABLE_COUNT],
) {
    let Some(current) = current.as_mut().and_then(Arc::get_mut) else {
        return;
    };
    for (index, view) in current.tables.iter_mut().enumerate() {
        if replaced[index]
            && view
                .2
                .as_ref()
                .is_some_and(|facts| Arc::strong_count(facts) == 1)
        {
            view.2 = None;
        }
    }
}

#[derive(Debug)]
struct ChangeFactAdmissionRefused {
    budget: allocation::credit::MemoryBudget,
    bytes: usize,
    release: allocation::credit::ReleaseToken,
}

impl std::fmt::Display for ChangeFactAdmissionRefused {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .write_str("display change-fact working set remains pinned; common paging is required")
    }
}
impl std::error::Error for ChangeFactAdmissionRefused {}

impl FrameDecoder {
    /// A credit refusal occurs before baseline consumption, so callers may release pins and retry.
    pub fn change_fact_admission_refused(error: &io::Error) -> bool {
        error
            .get_ref()
            .is_some_and(|error| error.is::<ChangeFactAdmissionRefused>())
    }
}

impl FrameDecoder {
    /// The owner and whole next reservation needed by a pre-baseline refusal.
    pub fn change_fact_pressure(
        error: &io::Error,
    ) -> Option<(
        &allocation::credit::MemoryBudget,
        usize,
        allocation::credit::ReleaseToken,
    )> {
        let refused = error
            .get_ref()?
            .downcast_ref::<ChangeFactAdmissionRefused>()?;
        Some((&refused.budget, refused.bytes, refused.release))
    }
}

pub(super) fn pinned_change_refusal(
    budget: &allocation::credit::MemoryBudget,
    bytes: usize,
    release: allocation::credit::ReleaseToken,
) -> io::Error {
    io::Error::new(
        io::ErrorKind::OutOfMemory,
        ChangeFactAdmissionRefused {
            budget: budget.clone(),
            bytes,
            release,
        },
    )
}

/// `bytes` of `budget` for change facts and field stamps; a refusal is the pressure
/// [`FrameDecoder::change_fact_admission_refused`] names, retryable once pins are released.
pub fn change_credit(
    budget: &allocation::credit::MemoryBudget,
    bytes: usize,
    cancelled: Option<&dyn Fn() -> bool>,
) -> io::Result<allocation::credit::Credit> {
    if cancelled.is_some_and(|cancelled| cancelled()) {
        return Err(io::Error::from(io::ErrorKind::Interrupted));
    }
    // Any synchronous caller or visitor may hold credit; never wait on its stack.
    let release = budget.release_token();
    budget.admit_item(bytes).map_err(|error| {
        if error.kind() == io::ErrorKind::WouldBlock {
            pinned_change_refusal(budget, bytes, release)
        } else {
            error
        }
    })
}

/// Whether a keyframe may hold `update`: one that needs no earlier hour.
/// [`FrameDecoder::advance`] refuses a keyframe holding any other, whichever
/// tables it reconstructs.
pub fn keyframe_update(update: &TableUpdate) -> bool {
    matches!(
        update,
        TableUpdate::Snapshot(_)
            | TableUpdate::Unavailable { .. }
            | TableUpdate::NotRecorded { .. }
            | TableUpdate::Incompatible { .. }
    )
}

/// The views deltas replaced while another holder still shared them, one
/// per table, kept until that table's next update.
#[derive(Clone, Default)]
pub struct Spares(Vec<Option<Spare>>);

impl Spares {
    /// Room for every table's spare, made once.
    pub fn reserve(&mut self) -> io::Result<()> {
        if self.0.is_empty() {
            self.0 = allocation::vec(TABLE_COUNT)?;
            self.0.resize_with(TABLE_COUNT, || None);
        }
        Ok(())
    }

    /// `index`'s spare; [`Self::reserve`] made room for it.
    pub fn slot(&mut self, index: usize) -> &mut Option<Spare> {
        &mut self.0[index]
    }

    /// Every table's spare by table index, each to borrow on its own.
    pub fn slots(&mut self) -> &mut [Option<Spare>] {
        &mut self.0
    }
}

/// A view a delta replaced while another holder still shared it, that delta,
/// and the view the delta made. Once nothing else holds the replaced view,
/// the next delta catches it up in place instead of copying the shared view
/// again, so a holder keeping each hour's view until the next costs one copy,
/// not one each hour.
#[derive(Clone, Debug)]
pub struct Spare {
    view: SharedView,
    delta: Arc<TableUpdate>,
    made: std::sync::Weak<TableView>,
}

impl Spare {
    /// `current`'s rows, owned, when `current` is the view the spare's delta
    /// made and nothing else holds the spare's view.
    fn take_rows(self, current: &SharedView) -> io::Result<Option<crate::rows::TableRows>> {
        let Self { view, delta, made } = self;
        if !std::ptr::eq(made.as_ptr(), Arc::as_ptr(&current.0)) {
            return Ok(None);
        }
        let TableUpdate::Delta {
            changed, deleted, ..
        } = &*delta
        else {
            return Ok(None);
        };
        let Ok(TableView::Complete(rows)) = view.try_unwrap() else {
            return Ok(None);
        };
        match rows.apply_delta_validated(changed, deleted) {
            Ok(rows) => Ok(Some(rows)),
            Err(error) if allocation::is_allocation_error(&error) => Err(error),
            Err(_) => Ok(None),
        }
    }
}

/// Apply a public table view, separately attributed from private alias source applications.
pub fn advance_public_table(
    table: TableId,
    update: &TableUpdate,
    previous: Option<(Hour, SharedView)>,
    hour: Hour,
    read: bool,
) -> io::Result<SharedView> {
    let _cpu = crate::cpu::reader("reader_public_apply", table);
    advance_table(table, update, previous, hour, read)
}

/// [`advance_public_table`] keeping the view a delta replaces while another
/// holder still shares it in `spare`, with the delta (`shared` when the
/// source shares it, else a copy), for the table's next update to reuse.
pub fn advance_public_table_reusing(
    table: TableId,
    update: &TableUpdate,
    shared: Option<&Arc<TableUpdate>>,
    previous: Option<(Hour, SharedView)>,
    spare: &mut Option<Spare>,
    hour: Hour,
    read: bool,
) -> io::Result<SharedView> {
    let _cpu = crate::cpu::reader("reader_public_apply", table);
    advance_table_with(
        table,
        update,
        previous,
        Some(Reuse { shared, spare }),
        hour,
        read,
    )
}

/// What a reader keeps to reuse replaced views ([`Spare`]).
struct Reuse<'a> {
    shared: Option<&'a Arc<TableUpdate>>,
    spare: &'a mut Option<Spare>,
}

/// One table's view at `hour`, from its update and the hour before's complete
/// view with that hour (`previous`), as [`FrameDecoder::advance`] reconstructs
/// it: a failed update becomes a named unavailable view, and a complete table
/// `read` this hour (its own nonzero timing) is sampled at it. Only an
/// allocation failure is an error.
pub fn advance_table(
    table: TableId,
    update: &TableUpdate,
    previous: Option<(Hour, SharedView)>,
    hour: Hour,
    read: bool,
) -> io::Result<SharedView> {
    advance_table_with(table, update, previous, None, hour, read)
}

fn advance_table_with(
    table: TableId,
    update: &TableUpdate,
    previous: Option<(Hour, SharedView)>,
    reuse: Option<Reuse<'_>>,
    hour: Hour,
    read: bool,
) -> io::Result<SharedView> {
    let _table = replay_data::measurement::table(table);
    match apply_table(table, update, previous, reuse, hour) {
        Ok(mut view) => {
            // The collector exports zero timing for skipped tables
            // (sampling::exported_table_ticks). Equality may encode a
            // successful acquisition as Unchanged; completeness and this
            // table's own timing, not the wire update, witness it.
            if matches!(*view, TableView::Complete(_)) && read {
                view.1 = Some(hour);
            }
            Ok(view)
        }
        Err(error) if allocation::is_allocation_error(&error) => Err(error),
        Err(error) => Ok(TableView::Unavailable {
            reason: error.to_string(),
        }
        .into()),
    }
}

/// Why a table not selected for reconstruction holds no rows: what tells its view apart
/// ([`TableView::is_not_selected`]).
pub const NOT_SELECTED: &str = "Table not selected for reconstruction";

/// One view shared by every unselected table of every hour a thread builds. Each thread holds
/// its own, so threads building frames at once never contend for one count; the view is told
/// apart by what it holds, never by its address.
pub fn not_selected() -> SharedView {
    #[cfg(any(test, feature = "test-support"))]
    UNSELECTED_SHARES.with(|count| count.set(count.get() + 1));
    thread_local! {
        static VIEW: SharedView = TableView::Unavailable {
            reason: NOT_SELECTED.into(),
        }
        .into();
    }
    VIEW.with(SharedView::clone)
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    /// Times this thread shared the unselected view anew.
    static UNSELECTED_SHARES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// `read`'s result and the times it shared the unselected view anew on this thread.
#[cfg(any(test, feature = "test-support"))]
pub fn count_unselected<T>(read: impl FnOnce() -> T) -> (T, usize) {
    let before = UNSELECTED_SHARES.with(std::cell::Cell::get);
    let result = read();
    (
        result,
        UNSELECTED_SHARES.with(std::cell::Cell::get) - before,
    )
}

/// An unchanged table is the previous hour's allocation, shared rather than
/// copied; a delta takes the baseline by value, which copies it only when an
/// earlier hour's frame still holds it.
fn apply_table(
    table: TableId,
    update: &TableUpdate,
    previous: Option<(Hour, SharedView)>,
    reuse: Option<Reuse<'_>>,
    hour: Hour,
) -> io::Result<SharedView> {
    // A spare serves only the update right after the delta that kept it.
    let (spare, reuse) = match reuse {
        Some(Reuse { shared, spare }) => (spare.take(), Some((shared, spare))),
        None => (None, None),
    };
    match update {
        TableUpdate::Snapshot(values) => {
            validate_complete(table, values, hour)?;
            let rows = values.try_clone_validated()?;
            replay_data::measurement::count(replay_data::measurement::Counter::SnapshotCopies, 1);
            replay_data::measurement::count(
                replay_data::measurement::Counter::SnapshotRows,
                values.len(),
            );
            Ok(TableView::Complete(rows).into())
        }
        TableUpdate::Truncated { rows, missing } => {
            validate_rows(table, rows)?;
            if *missing == 0 {
                return Err(invalid("truncated table has no missing rows"));
            }
            let rows = rows.try_clone_validated()?;
            replay_data::measurement::count(replay_data::measurement::Counter::SnapshotCopies, 1);
            replay_data::measurement::count(
                replay_data::measurement::Counter::SnapshotRows,
                rows.len(),
            );
            Ok(TableView::Truncated {
                rows,
                missing: *missing,
            }
            .into())
        }
        TableUpdate::Unavailable { reason } | TableUpdate::Incompatible { reason, .. } => {
            if reason.is_empty() {
                return Err(invalid("unavailable table needs a reason"));
            }
            Ok(TableView::Unavailable {
                reason: allocation::string(reason)?,
            }
            .into())
        }
        TableUpdate::NotRecorded { mode } => Ok(TableView::NotRecorded { mode: *mode }.into()),
        TableUpdate::Delta { baseline, .. } | TableUpdate::Unchanged { baseline } => {
            let Some((_, previous_view)) =
                previous.filter(|(previous_hour, _)| previous_hour == baseline && *baseline < hour)
            else {
                return Err(invalid(format!(
                    "missing table baseline at hour {baseline}"
                )));
            };
            if let TableUpdate::Delta {
                changed, deleted, ..
            } = update
            {
                validate_delta(table, changed, deleted)?;
                // The baseline passed full row validation, so a delta, in
                // place or on a copy, checks only the keys it can have changed.
                let mut replaced = None;
                let rows = match previous_view.try_unwrap() {
                    Ok(TableView::Complete(previous_rows)) => {
                        previous_rows.apply_delta_validated(changed, deleted)?
                    }
                    Err(shared) => {
                        let TableView::Complete(previous_rows) = &*shared else {
                            unreachable!("only complete tables become baselines")
                        };
                        let started = if crate::cpu::enabled() {
                            crate::cpu::stamp()
                        } else {
                            0
                        };
                        let caught = match spare {
                            Some(spare) => spare.take_rows(&shared)?,
                            None => None,
                        };
                        let rows = match caught {
                            Some(rows) => {
                                crate::cpu::reused(table, rows.len(), started);
                                rows.apply_delta_validated(changed, deleted)?
                            }
                            None => {
                                let rows =
                                    previous_rows.apply_delta_shared_validated(changed, deleted)?;
                                crate::cpu::copied(table, previous_rows.len(), started);
                                rows
                            }
                        };
                        replaced = Some(shared);
                        rows
                    }
                    Ok(_) => unreachable!("only complete tables become baselines"),
                };
                validate_applied(table, &rows, hour)?;
                let view: SharedView = TableView::Complete(rows).into();
                if let (Some(mut replaced), Some((shared, slot))) = (replaced, reuse) {
                    let delta = match shared {
                        Some(shared) => Some(Arc::clone(shared)),
                        None => super::recorded::copy_update(update).ok().map(Arc::new),
                    };
                    // Only its rows are reused; its change facts' credit goes back now.
                    replaced.2 = None;
                    replaced.3 = None;
                    *slot = delta.map(|delta| Spare {
                        view: replaced,
                        delta,
                        made: Arc::downgrade(&view.0),
                    });
                }
                Ok(view)
            } else {
                // This private baseline already passed full row validation. Only
                // the singleton's relation to the new enclosing hour can have changed.
                let TableView::Complete(previous_rows) = &*previous_view else {
                    unreachable!("only complete tables become baselines");
                };
                validate_singleton(table, previous_rows, hour)?;
                Ok(previous_view)
            }
        }
    }
}

fn change_facts(
    table: TableId,
    previous: &SharedView,
    changed: &[crate::rows::DeltaRow],
    deleted: &[u128],
    credit: allocation::credit::Credit,
) -> io::Result<super::ChangeFacts> {
    use super::{ChangeFacts, RowChange};
    let TableView::Complete(rows) = &**previous else {
        unreachable!("complete delta baseline")
    };
    let mut facts = ChangeFacts {
        previous: previous.identity(),
        changed: allocation::vec(changed.len())?,
        deleted: allocation::copy_vec(deleted)?,
        fields: 0,
        membership: !deleted.is_empty(),
        _credit: credit,
    };
    for row in changed {
        let fields = change_fields(table, row);
        let inserted = !rows.contains_key(row.key);
        facts.fields |= fields;
        facts.membership |= inserted;
        facts.changed.push(RowChange {
            key: row.key,
            fields,
            inserted,
        });
    }
    Ok(facts)
}

pub(super) fn change_fields(table: TableId, row: &crate::rows::DeltaRow) -> u64 {
    // Historical schemas keep their column positions; omitted suffixes become NotRecorded.
    let missing = if row.recorded_fields >= u64::BITS as usize {
        0
    } else {
        u64::MAX << row.recorded_fields
    };
    row.mask | (table.full_mask() & missing)
}

pub(super) fn field_stamp(
    previous: Option<&super::FieldStamp>,
    changes: Option<&super::ChangeFacts>,
    hour: Hour,
    credit: allocation::credit::Credit,
) -> io::Result<std::sync::Arc<super::FieldStamp>> {
    use super::FieldStamp;
    use std::sync::Arc;
    let mut stamp = FieldStamp {
        epoch: previous.map_or_else(|| Arc::new(()), |old| Arc::clone(&old.epoch)),
        fields: previous.map_or([i64::MIN; 64], |old| old.fields),
        membership: previous.map_or(i64::MIN, |old| old.membership),
        _credit: credit,
    };
    if let Some(changes) = changes {
        if changes.membership {
            stamp.membership = i64::from(hour);
        }
        let mut fields = changes.fields;
        while fields != 0 {
            stamp.fields[fields.trailing_zeros() as usize] = i64::from(hour);
            fields &= fields - 1;
        }
    }
    Ok(Arc::new(stamp))
}

pub(super) const fn stamp_bytes() -> usize {
    size_of::<super::FieldStamp>() + 4 * size_of::<usize>()
}
pub(super) fn change_bytes(changed: usize, deleted: usize) -> io::Result<usize> {
    allocation::add(
        size_of::<super::ChangeFacts>() + 2 * size_of::<usize>(),
        allocation::add(
            allocation::mul(changed, size_of::<super::RowChange>())?,
            allocation::mul(deleted, size_of::<u128>())?,
        )?,
    )
}
