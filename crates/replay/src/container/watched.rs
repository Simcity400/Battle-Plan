//! One read of a block for every consumer of it.
//!
//! [`Replay::read_block_watched`] decodes a block's tables once, for the
//! union of the tables its watches read, and reconstructs each of them once
//! per hour. Each watch is handed exactly what its own read hands it: a watch
//! of changes ([`Watching::Changes`]) the steps [`Replay::read_block_changes`]
//! visits for its fields, and a watch of every hour ([`Watching::Every`]) the
//! frames [`Replay::read_block_alone`] reconstructs for its tables.
//!
//! This holds table by table. A selected read decodes each table's updates on
//! their own, so a table's updates, and every hour and its metrics, are the
//! same whatever else is read. Each table's view is advanced from its own
//! update and its own view of the hour before alone, so its rows,
//! availability and sample hour are those of a read of fewer tables. Which
//! hours share a table's allocation follows from its updates alone: an
//! unchanged table keeps the hour before's allocation, and every other update
//! (a delta included) makes a new one, whoever else holds the view. A table a
//! watch does not read is the one view every unselected table shares, as in
//! its own read.
//!
//! The one check a read makes over every table, that a keyframe holds only
//! updates needing no earlier hour, is made for each watch as its own read
//! makes it: a watch whose own read refuses the block's keyframe is refused
//! with that read's error, and the others go on. A failure of the read
//! itself (the block cannot be read or verified, an allocation fails, the
//! campaign identity differs, cancellation) fails every watch still reading.
//! A watch whose visit fails stops there with its error, and the others go on.

use super::*;
use crate::frame::{
    FrameSource, IntoRecordedFrame, RecordedFrame, advance_public_table,
    advance_public_table_reusing, recorded::Updates,
};
use crate::{
    BranchId,
    frame::{SharedView, Spare, Spares, TableUpdate, keyframe_update, not_selected},
    rows::{compatibility::HISTORY, full_mask},
};
use std::sync::Arc;

use super::changes::{QuietHours, complete_len, touches, view_differs};

/// What one watch of [`Replay::read_block_watched`] reads of a block.
#[derive(Clone, Copy, Debug)]
pub enum Watching<'a> {
    /// The hours at which these fields change, as
    /// [`Replay::read_block_changes`] visits them.
    Changes(&'a [TableFields]),
    /// Every hour of these tables, as [`Replay::read_block_alone`]
    /// reconstructs them.
    Every(&'a [TableId]),
}

/// One step handed to a watch, in hour order.
#[derive(Debug)]
pub enum Watched<'a> {
    /// Before a visited hour replaces these tables, after its preceding
    /// quiet span was handed over. Only watches opting into
    /// [`BlockWatch::releasing`] receive this housekeeping step; no frame
    /// is observed here. Earlier views of these tables can now be released.
    Replacing(&'a [TableId]),
    /// A watch of changes' step, as [`Replay::read_block_changes`] visits it.
    Change(BlockChange<'a>),
    /// An hour of a watch of every hour: the frame
    /// [`Replay::read_block_alone`] reconstructs for its tables, and the
    /// hour's recorded updates. Those of the watch's tables (and Campaign)
    /// are its own read's; the updates of tables it does not read are other
    /// watches' or none, which change facts never read for a table whose
    /// reconstructed view is not complete.
    Hour {
        updates: &'a Arc<RecordedFrame>,
        frame: &'a DecodedFrame,
    },
}

type Step<'s> = dyn FnMut(Watched<'_>) -> io::Result<()> + 's;

/// A watch's visit: on the reading thread, or on any.
enum Visit<'v> {
    Here(Box<Step<'v>>),
    Anywhere(Box<dyn FnMut(Watched<'_>) -> io::Result<()> + Send + 'v>),
}

impl<'v> Visit<'v> {
    fn step(&mut self) -> &mut Step<'v> {
        match self {
            Self::Here(visit) => &mut **visit,
            Self::Anywhere(visit) => &mut **visit,
        }
    }
}

/// One consumer of [`Replay::read_block_watched`]: what it reads, the visit
/// that is handed its steps, and once read its own outcome.
pub struct BlockWatch<'v> {
    watching: Watching<'v>,
    visit: Visit<'v>,
    outcome: Option<io::Result<()>>,
    releasing: bool,
}

impl<'v> BlockWatch<'v> {
    pub fn new(
        watching: Watching<'v>,
        visit: impl FnMut(Watched<'_>) -> io::Result<()> + 'v,
    ) -> Self {
        Self::with(watching, Visit::Here(Box::new(visit)))
    }

    /// A watch whose visits a read with a [`Fanout`] may make on its helpers,
    /// at once with other watches' visits of the same hour.
    pub fn sendable(
        watching: Watching<'v>,
        visit: impl FnMut(Watched<'_>) -> io::Result<()> + Send + 'v,
    ) -> Self {
        Self::with(watching, Visit::Anywhere(Box::new(visit)))
    }

    fn with(watching: Watching<'v>, visit: Visit<'v>) -> Self {
        Self {
            watching,
            visit,
            outcome: None,
            releasing: false,
        }
    }

    /// Request a [`Watched::Replacing`] step before an hour known to visit
    /// this watch is reconstructed. A consumer keeping its last frame only
    /// to build quiet hours can release the views that hour will replace,
    /// letting deltas apply in place. Other retained frames stay intact.
    pub fn releasing(mut self) -> Self {
        self.releasing = true;
        self
    }

    /// Whether it stopped with an outcome of its own: its keyframe refused
    /// or its visit failed.
    pub fn stopped(&self) -> bool {
        self.outcome.is_some()
    }

    /// Its own outcome: the error its own read would have returned for the
    /// block's keyframe, or its visit's error; Ok when neither.
    pub fn finish(self) -> io::Result<()> {
        self.outcome.unwrap_or(Ok(()))
    }
}

/// What a block's read keeps for a reader besides its watches: these tables
/// decoded with the watches' tables, and every hour's recorded updates of
/// all of them handed to `frames` once the block is admitted and before any
/// hour is reconstructed, as [`Replay::read_block_selected`] returns them for
/// those tables (the updates of tables no read selected are unavailable).
/// Tables no watch reads are decoded only: the reader keeping their updates
/// reconstructs its own views.
pub struct Keep<'k> {
    pub tables: &'k [TableId],
    pub frames: &'k mut dyn FnMut(&[Arc<RecordedFrame>]),
}

/// Helper threads on which a block's read hands the watches visited at one
/// hour their steps at once ([`BlockWatch::sendable`]), each in hour order,
/// while the reading thread visits one and waits for the rest before the
/// next hour is reconstructed. Each helper runs as the reading thread does.
pub struct Fanout(rayon::ThreadPool);

impl Fanout {
    /// `helpers` threads, named `name` and a number.
    pub fn new(name: &'static str, helpers: usize) -> io::Result<Self> {
        rayon::ThreadPoolBuilder::new()
            .num_threads(helpers.max(1))
            .thread_name(move |helper| format!("{name} {helper}"))
            .build()
            .map(Self)
            .map_err(io::Error::other)
    }
}

impl Replay {
    /// One read of a block for every one of `watches`: the tables any of
    /// them reads (and Campaign) decoded and reconstructed once, each watch
    /// handed what its own read hands it ([`Watching`]), the watches in order
    /// at each hour. Changed hours, and frames of every hour, share a
    /// table's allocation exactly when the watch's own read's do. Admission
    /// and verification are [`Replay::read_block_changes`]'s, for the union.
    ///
    /// Each watch's own outcome is [`BlockWatch::finish`]'s. An error here is
    /// a failure of the read itself, which fails every watch that had not
    /// stopped; earlier hours may have been visited.
    pub fn read_block_watched(
        &self,
        index: &BlockIndex,
        watches: &mut [BlockWatch<'_>],
        check: impl FnMut() -> io::Result<()>,
    ) -> io::Result<()> {
        self.read_block_watched_keeping(index, watches, None, None, check)
    }

    /// [`Replay::read_block_watched`], keeping the block's frames for another
    /// reader ([`Keep`]), with the watches visited at each hour visited at
    /// once on `fanout`'s helpers.
    pub fn read_block_watched_keeping(
        &self,
        index: &BlockIndex,
        watches: &mut [BlockWatch<'_>],
        keep: Option<Keep<'_>>,
        fanout: Option<&Fanout>,
        mut check: impl FnMut() -> io::Result<()>,
    ) -> io::Result<()> {
        check()?;
        let extra = keep.as_ref().map_or(&[][..], |keep| keep.tables);
        let mut reading = Reading::new(
            &self.campaign_id,
            index.branch,
            self.format,
            watches,
            extra,
            fanout,
        )?;
        let block = self.grouped_block(index)?;
        // Admission as `read_block_selected`: the selected tables
        // decoded, then every section's checksum on a first read.
        let read = super::grouped::read_selected_updates(
            self,
            index,
            &block,
            &reading.tables.tables,
            &mut check,
        )?;
        if !self.is_verified(index) {
            super::grouped::verify_with(&block, index, &mut super::check::Check::new(&mut check))?;
        }
        check()?;
        self.mark_verified(index);
        drop(block);
        check()?;
        let super::grouped::SelectedUpdates {
            hours,
            metrics,
            updates,
            dependent_keyframe,
        } = read;
        let keyframe = Keyframe::Grouped(dependent_keyframe);
        if let Some(keep) = keep {
            // Every hour's frame first, for the reader keeping them.
            let frames =
                engine_allocation::collect_vec(hours.into_iter().zip(metrics).enumerate().map(
                    |(position, (hour, metrics))| {
                        reading
                            .tables
                            .take_frame(
                                index.branch,
                                hour,
                                position == 0,
                                metrics,
                                &updates,
                                position,
                            )
                            .map(Arc::new)
                    },
                ))?
                .into_iter()
                .collect::<io::Result<Vec<_>>>()?;
            drop(updates);
            (keep.frames)(&frames);
            for frame in frames {
                check()?;
                if !reading.hour(frame, keyframe)? {
                    break;
                }
            }
            reading.flush()?;
            return Ok(());
        }
        for (position, (hour, metrics)) in hours.into_iter().zip(metrics).enumerate() {
            check()?;
            let frame = reading.tables.take_frame(
                index.branch,
                hour,
                position == 0,
                metrics,
                &updates,
                position,
            )?;
            if !reading.hour(frame, keyframe)? {
                break;
            }
        }
        reading.flush()?;
        Ok(())
    }
}

impl Replay {
    /// A block's frames for `tables` (and Campaign) as
    /// [`Replay::read_block_selected`] returns them, except that a grouped
    /// block's tables not read are unavailable without a reason, which no
    /// reader reconstructs: for readers that keep a block for others and may
    /// give it up partway. `check` stops table, hour and verification work
    /// cooperatively; an error releases the unfinished read. Admission and
    /// verification are `read_block_selected`'s.
    pub fn read_block_kept(
        &self,
        index: &BlockIndex,
        tables: &[TableId],
        mut check: impl FnMut() -> io::Result<()>,
    ) -> io::Result<Vec<Arc<RecordedFrame>>> {
        self.read_block_kept_inner(index, tables, &mut check, true)
    }

    /// Keep a block, checking willingness only at the original table/hour boundaries.
    pub fn read_block_kept_coarse(
        &self,
        index: &BlockIndex,
        tables: &[TableId],
        mut check: impl FnMut() -> io::Result<()>,
    ) -> io::Result<Vec<Arc<RecordedFrame>>> {
        self.read_block_kept_inner(index, tables, &mut check, false)
    }

    fn read_block_kept_inner(
        &self,
        index: &BlockIndex,
        tables: &[TableId],
        check: &mut dyn FnMut() -> io::Result<()>,
        inner_checks: bool,
    ) -> io::Result<Vec<Arc<RecordedFrame>>> {
        check()?;
        let mut chosen = [false; TABLE_COUNT];
        chosen[TableId::Campaign.index()] = true;
        for table in tables {
            chosen[table.index()] = true;
        }
        let read = BlockTables::new(&self.campaign_id, self.format, &chosen)?;
        let block = self.grouped_block(index)?;
        let selected = if inner_checks {
            super::grouped::read_selected_updates(self, index, &block, &read.tables, check)?
        } else {
            super::grouped::read_selected_updates_coarse(self, index, &block, &read.tables, check)?
        };
        if !self.is_verified(index) {
            if inner_checks {
                super::grouped::verify_with(&block, index, &mut super::check::Check::new(check))?;
            } else {
                super::grouped::verify(&block, index)?;
            }
        }
        if inner_checks {
            check()?;
        }
        self.mark_verified(index);
        drop(block);
        let super::grouped::SelectedUpdates {
            hours,
            metrics,
            updates,
            ..
        } = selected;
        let mut frames = Vec::new();
        engine_allocation::reserve(&mut frames, hours.len())?;
        for (position, (hour, metrics)) in hours.into_iter().zip(metrics).enumerate() {
            check()?;
            frames.push(
                read.take_frame(
                    index.branch,
                    hour,
                    position == 0,
                    metrics,
                    &updates,
                    position,
                )?
                .into_recorded(),
            );
        }
        Ok(frames)
    }
}

/// How a block's read made the keyframe check over tables no watch reads.
#[derive(Clone, Copy)]
enum Keyframe {
    /// A grouped block: whether a table the read decodes although none
    /// selected it holds a first update no keyframe may hold.
    Grouped(bool),
}

/// The decoded union's tables, those read by a watch advanced once per hour
/// exactly as a reader of that table alone advances them, and what each
/// hour's step left for the watches' decisions.
struct BlockTables<'r> {
    campaign_id: &'r [u8],
    format: crate::format::FormatVersion,
    /// The tables read, Campaign included, in table order, and each one's
    /// position among them by table index.
    tables: Vec<TableId>,
    slots: Arc<[Option<usize>; TABLE_COUNT]>,
    views: Vec<Option<SharedView>>,
    last_hour: Option<Hour>,
    /// This hour, by position: whether the update rebuilt the view; the
    /// row count of the complete view a delta applied to; the view of the
    /// hour before that another update replaced.
    rebuilt: Vec<bool>,
    count: Vec<Option<usize>>,
    before: Vec<Option<SharedView>>,
    spares: Spares,
}

impl<'r> BlockTables<'r> {
    fn new(
        campaign_id: &'r [u8],
        format: crate::format::FormatVersion,
        chosen: &[bool; TABLE_COUNT],
    ) -> io::Result<Self> {
        let tables = engine_allocation::collect_vec(
            TableId::ALL
                .into_iter()
                .filter(|table| chosen[table.index()]),
        )?;
        let mut slots = [None; TABLE_COUNT];
        for (position, table) in tables.iter().enumerate() {
            slots[table.index()] = Some(position);
        }
        Ok(Self {
            campaign_id,
            format,
            views: engine_allocation::collect_vec(tables.iter().map(|_| None))?,
            rebuilt: engine_allocation::collect_vec(tables.iter().map(|_| false))?,
            count: engine_allocation::collect_vec(tables.iter().map(|_| None))?,
            before: engine_allocation::collect_vec(tables.iter().map(|_| None))?,
            spares: {
                let mut spares = Spares::default();
                spares.reserve()?;
                spares
            },
            tables,
            slots: Arc::new(slots),
            last_hour: None,
        })
    }

    /// The hour at `position` of a grouped block's decoded updates, taken
    /// out of them: the union's updates, and an unavailable update without
    /// a reason for every other table, which nothing reconstructs.
    fn take_frame(
        &self,
        branch: BranchId,
        hour: Hour,
        keyframe: bool,
        metrics: Metrics,
        updates: &[Updates],
        position: usize,
    ) -> io::Result<RecordedFrame> {
        let slots = Arc::clone(&self.slots);
        RecordedFrame::from_columns(
            branch,
            hour,
            keyframe,
            metrics,
            slots,
            updates,
            position,
            Arc::new(TableUpdate::Unavailable {
                reason: String::new(),
            }),
        )
    }

    /// Whether a watch reading `own` (positions among the tables) is refused
    /// this keyframe by its own read: one of its tables holds an update no
    /// keyframe may hold, or another table its own read checks does.
    fn refuses_keyframe(&self, own: &[usize], frame: &RecordedFrame, keyframe: Keyframe) -> bool {
        let version = self.format.version();
        TableId::ALL.into_iter().any(|table| {
            let update = &frame.tables[table.index()];
            match self.slots[table.index()] {
                Some(position) if own.contains(&position) => !keyframe_update(update),
                // Its own read does not select this table.
                Some(_) => {
                    let change = &HISTORY[table.index()];
                    change.present(version)
                        && change.table_gap(table, version).is_some()
                        && !keyframe_update(update)
                }
                None => false,
            }
        }) || matches!(keyframe, Keyframe::Grouped(true))
    }

    /// Advance the watches' tables to `frame`'s hour from their updates,
    /// exactly as `FrameDecoder::advance` advances them, and check the
    /// campaign. Tables decoded only for another reader stay unreconstructed.
    /// With a fanout, the tables an update replaces advance at once on its
    /// helpers: each from its own update and view of the hour before alone.
    fn advance(
        &mut self,
        frame: &RecordedFrame,
        reconstructed: &[usize],
        fanout: Option<&Fanout>,
    ) -> io::Result<()> {
        // Measurement only: each table's reconstruct cycles, when enabled.
        let _cpu = crate::cpu::scope("reconstruct", None);
        let measured = crate::cpu::enabled();
        let hour = frame.hour;
        // As the decoder: only the hour before's complete views are
        // baselines, and a keyframe has none.
        let before_hour = self.last_hour.filter(|_| !frame.keyframe);
        let mut lanes = self.lanes(reconstructed)?;
        let replaced = |lane: &Lane<'_>| {
            !matches!(
                frame.tables[lane.table.index()],
                TableUpdate::Unchanged { .. }
            )
        };
        match fanout {
            Some(fanout) if lanes.iter().filter(|lane| replaced(lane)).count() > 1 => {
                let priority = job_priority::Caller::current();
                let cpu = replay_frame::cpu::Context::capture();
                // Room made before the helpers allocate, so filling it cannot fail meanwhile.
                let mut here = engine_allocation::vec(lanes.len())?;
                fanout.0.in_place_scope(|scope| {
                    for lane in &mut lanes {
                        if replaced(lane) {
                            let (priority, cpu) = (priority.clone(), cpu.clone());
                            scope.spawn(move |_| {
                                let _cpu = cpu.enter("reconstruct");
                                priority.enter();
                                lane.advance(frame, before_hour, measured);
                            });
                        } else {
                            here.push(lane);
                        }
                    }
                    for lane in here {
                        lane.advance(frame, before_hour, measured);
                    }
                });
            }
            _ => {
                for lane in &mut lanes {
                    lane.advance(frame, before_hour, measured);
                    if matches!(lane.outcome, Some(Err(_))) {
                        break;
                    }
                }
            }
        }
        let mut spent: Vec<(usize, u64)> = Vec::new();
        for lane in lanes {
            match lane.outcome {
                Some(Err(error)) => return Err(error),
                Some(Ok(())) if measured => spent.push((lane.table.index(), lane.spent)),
                _ => {}
            }
        }
        if measured {
            crate::cpu::reconstructed(hour, &spent);
        }
        if let Some(position) = self.slots[TableId::Campaign.index()]
            && let Some(view) = &self.views[position]
        {
            validate_campaign_view(view, self.campaign_id)?;
        }
        self.last_hour = Some(hour);
        Ok(())
    }

    /// Each reconstructed table's own state, borrowed apart, in table order.
    fn lanes(&mut self, reconstructed: &[usize]) -> io::Result<Vec<Lane<'_>>> {
        let mut chosen = [false; TABLE_COUNT];
        for &position in reconstructed {
            chosen[self.tables[position].index()] = true;
        }
        let mut spares = self
            .spares
            .slots()
            .iter_mut()
            .enumerate()
            .filter(|(index, _)| chosen[*index])
            .map(|(_, spare)| spare);
        engine_allocation::collect_vec(
            self.views
                .iter_mut()
                .zip(&mut self.count)
                .zip(&mut self.before)
                .zip(&mut self.rebuilt)
                .zip(&self.tables)
                .filter(|(_, table)| chosen[table.index()])
                .map(|((((view, count), before), rebuilt), &table)| Lane {
                    table,
                    view,
                    count,
                    before,
                    rebuilt,
                    spare: spares.next().expect("a spare for every table"),
                    outcome: None,
                    spent: 0,
                }),
        )
    }

    /// Whether the table at `position` differs this hour in availability, a
    /// truncated table's missing count, an unavailable table's reason, row
    /// keys or `fields`, as `read_block_changes` decides it.
    fn differs(&self, position: usize, fields: Option<u64>, update: &TableUpdate) -> bool {
        let table = self.tables[position];
        let view = self.views[position].as_ref().expect("an advanced table");
        if let TableUpdate::Delta {
            changed: rows,
            deleted,
            ..
        } = update
        {
            return match (&**view, self.count[position]) {
                (TableView::Complete(after), Some(count)) => {
                    !deleted.is_empty()
                        || after.len() != count
                        || rows.iter().any(|row| touches(table, fields, row))
                }
                _ => true,
            };
        }
        self.before[position]
            .as_ref()
            .is_none_or(|before| view_differs(table, fields, before, view))
    }

    /// The views of the hour before, which no decision needs any more.
    fn release_before(&mut self) {
        for before in &mut self.before {
            *before = None;
        }
    }
}

/// One table's state while an hour advances it ([`BlockTables::advance`]).
struct Lane<'t> {
    table: TableId,
    view: &'t mut Option<SharedView>,
    count: &'t mut Option<usize>,
    before: &'t mut Option<SharedView>,
    rebuilt: &'t mut bool,
    spare: &'t mut Option<Spare>,
    /// None until it advanced, or when an earlier table failed first.
    outcome: Option<io::Result<()>>,
    /// Its reconstruct cycles, when measured.
    spent: u64,
}

impl Lane<'_> {
    fn advance(&mut self, frame: &RecordedFrame, before_hour: Option<Hour>, measured: bool) {
        let started = if measured { crate::cpu::stamp() } else { 0 };
        self.outcome = Some(self.advanced(frame, before_hour));
        if measured {
            self.spent = crate::cpu::stamp().wrapping_sub(started);
        }
    }

    fn advanced(&mut self, frame: &RecordedFrame, before_hour: Option<Hour>) -> io::Result<()> {
        let table = self.table;
        let update = &frame.tables[table.index()];
        let read = frame.metrics.table_ns[table.index()] != 0;
        let hour = frame.hour;
        let before = self.view.take();
        let view = if let TableUpdate::Delta { .. } = update {
            // The view moves into the delta, which applies in place when
            // no visited frame still holds it. A delta always yields a
            // new view, as it does for a reader holding the hour before.
            *self.count = before.as_deref().and_then(complete_len);
            *self.before = None;
            *self.rebuilt = true;
            advance_public_table_reusing(
                table,
                update,
                frame.shared_update(table.index()),
                baseline(before_hour, before),
                self.spare,
                hour,
                read,
            )?
        } else {
            // A spare serves only the delta right after the one that kept it.
            self.spare.take();
            let view = advance_public_table(
                table,
                update,
                baseline(before_hour, before.clone()),
                hour,
                read,
            )?;
            // Only an unchanged table keeps the hour before's view; it is
            // still held here, so a new view cannot reuse its address.
            *self.rebuilt = before
                .as_ref()
                .is_none_or(|before| !SharedView::ptr_eq(before, &view));
            *self.count = None;
            *self.before = before;
            view
        };
        *self.view = Some(view);
        Ok(())
    }
}

/// The hour before's view as the decoder's baseline: only a complete one.
fn baseline(hour: Option<Hour>, view: Option<SharedView>) -> Option<(Hour, SharedView)> {
    let hour = hour?;
    view.filter(|view| matches!(**view, TableView::Complete(_)))
        .map(|view| (hour, view))
}

/// What a watch of changes follows: each reconstructed table's watched
/// fields (None for a Campaign it does not name) and whether its read hours
/// are visited, the quiet hours not yet visited and this hour's lists.
struct Changes {
    fields: Vec<(Option<u64>, bool)>,
    quiet: QuietHours,
    changed: Vec<TableId>,
    rebuilt: Vec<TableId>,
    /// Whether this hour is visited rather than kept quiet.
    visits: bool,
}

/// One watch while the block is read.
struct Consumer {
    /// Its reconstructed tables (Campaign too), by position among the
    /// union's, in table order.
    own: Vec<usize>,
    /// None for a watch of every hour.
    changes: Option<Changes>,
    /// The frame it was last handed, kept to hand the next with only its
    /// changed tables replaced; a table an hour will replace is released
    /// before the hour is reconstructed, so no delta copies for it.
    frame: Option<DecodedFrame>,
    replaced: Vec<TableId>,
}

/// A block's read for its watches.
struct Reading<'w, 'v, 'r> {
    tables: BlockTables<'r>,
    /// Positions of the tables any watch reads (Campaign too). Tables kept
    /// only as recorded updates for another reader need no views here.
    reconstructed: Vec<usize>,
    branch: BranchId,
    /// The tables whose updates the hour handed to watches of every hour
    /// keeps: theirs, those kept for another reader and Campaign. Every
    /// other table's update is released once the hour is reconstructed,
    /// unless the hour is kept for another reader whole.
    kept: [bool; TABLE_COUNT],
    consumers: Vec<Consumer>,
    watches: &'w mut [BlockWatch<'v>],
    fanout: Option<&'w Fanout>,
}

impl<'w, 'v, 'r> Reading<'w, 'v, 'r> {
    fn new(
        campaign_id: &'r [u8],
        branch: BranchId,
        format: crate::format::FormatVersion,
        watches: &'w mut [BlockWatch<'v>],
        extra: &[TableId],
        fanout: Option<&'w Fanout>,
    ) -> io::Result<Self> {
        let mut chosen = [false; TABLE_COUNT];
        chosen[TableId::Campaign.index()] = true;
        for watch in watches.iter() {
            match watch.watching {
                Watching::Changes(fields) => {
                    for entry in fields {
                        chosen[entry.table.index()] = true;
                    }
                }
                Watching::Every(tables) => {
                    for table in tables {
                        chosen[table.index()] = true;
                    }
                }
            }
        }
        let reconstructed = chosen;
        for table in extra {
            chosen[table.index()] = true;
        }
        let tables = BlockTables::new(campaign_id, format, &chosen)?;
        let reconstructed = engine_allocation::collect_vec(
            tables
                .tables
                .iter()
                .enumerate()
                .filter_map(|(position, table)| reconstructed[table.index()].then_some(position)),
        )?;
        let mut kept = [false; TABLE_COUNT];
        kept[TableId::Campaign.index()] = true;
        for table in extra {
            kept[table.index()] = true;
        }
        for watch in watches.iter() {
            if let Watching::Every(selected) = watch.watching {
                for table in selected {
                    kept[table.index()] = true;
                }
            }
        }
        let mut consumers = Vec::new();
        engine_allocation::reserve(&mut consumers, watches.len())?;
        for watch in watches.iter() {
            let own_tables: Vec<TableId> = match watch.watching {
                Watching::Changes(fields) => {
                    engine_allocation::collect_vec(TableId::ALL.into_iter().filter(|&table| {
                        table == TableId::Campaign
                            || fields.iter().any(|entry| entry.table == table)
                    }))?
                }
                Watching::Every(selected) => engine_allocation::collect_vec(
                    TableId::ALL
                        .into_iter()
                        .filter(|&table| table == TableId::Campaign || selected.contains(&table)),
                )?,
            };
            let own: Vec<usize> = engine_allocation::collect_vec(
                own_tables
                    .iter()
                    .map(|table| tables.slots[table.index()].expect("a chosen table")),
            )?;
            let changes = match watch.watching {
                Watching::Changes(fields) => Some(Changes {
                    // Duplicate entries of a table watch the union of their
                    // fields.
                    fields: engine_allocation::collect_vec(own_tables.iter().map(|&table| {
                        let named = fields
                            .iter()
                            .filter(|entry| entry.table == table)
                            .fold(None, |union, entry| Some(union.unwrap_or(0) | entry.fields));
                        let read = fields
                            .iter()
                            .any(|entry| entry.table == table && entry.read);
                        (named, read)
                    }))?,
                    quiet: QuietHours::for_tables(own_tables),
                    changed: Vec::new(),
                    rebuilt: Vec::new(),
                    visits: false,
                }),
                Watching::Every(_) => None,
            };
            let replaced = engine_allocation::vec(if watch.releasing { own.len() } else { 0 })?;
            consumers.push(Consumer {
                own,
                changes,
                frame: None,
                replaced,
            });
        }
        Ok(Self {
            tables,
            reconstructed,
            branch,
            kept,
            consumers,
            watches,
            fanout,
        })
    }

    fn live(&self) -> bool {
        self.watches.iter().any(|watch| !watch.stopped())
    }

    fn release_visiting(&mut self, frame: &RecordedFrame) -> io::Result<()> {
        let tables = &self.tables;
        each_watch(
            self.fanout,
            &mut self.consumers,
            &mut *self.watches,
            |consumer, watch| watch.releasing && visits_before(tables, consumer, frame),
            |consumer, visit| {
                if let Some(changes) = &mut consumer.changes {
                    changes.quiet.flush(&mut *visit)?;
                }
                consumer.replaced.clear();
                for &position in &consumer.own {
                    let table = tables.tables[position];
                    if !matches!(frame.tables[table.index()], TableUpdate::Unchanged { .. }) {
                        consumer.replaced.push(table);
                    }
                }
                visit(Watched::Replacing(&consumer.replaced))
            },
        )
    }

    /// Reconstruct one hour and hand it to every watch still reading;
    /// whether any still reads.
    fn hour(&mut self, frame: impl IntoRecordedFrame, keyframe: Keyframe) -> io::Result<bool> {
        let mut frame = frame.into_recorded();
        if frame.keyframe {
            for (consumer, watch) in self.consumers.iter().zip(self.watches.iter_mut()) {
                if !watch.stopped()
                    && self
                        .tables
                        .refuses_keyframe(&consumer.own, &frame, keyframe)
                {
                    watch.outcome = Some(Err(invalid_static(
                        "keyframe contains a dependent or partial table",
                    )));
                }
            }
            if !self.live() {
                return Ok(false);
            }
        } else {
            self.release_visiting(&frame)?;
            // A table this hour replaces is released from the frames kept,
            // so its delta applies in place unless a visitor holds it.
            for consumer in &mut self.consumers {
                if let Some(kept) = &mut consumer.frame {
                    for &position in &consumer.own {
                        let table = self.tables.tables[position];
                        if !matches!(frame.tables[table.index()], TableUpdate::Unchanged { .. }) {
                            kept.tables[table.index()] = not_selected();
                        }
                    }
                }
            }
        }
        self.tables.advance(&frame, &self.reconstructed, self.fanout)?;
        let metrics = &frame.metrics;
        // Every watch keeping this hour quiet shares one copy of its metrics.
        let mut quiet_metrics: Option<Arc<Metrics>> = None;
        for (consumer, watch) in self.consumers.iter_mut().zip(self.watches.iter()) {
            if watch.stopped() {
                continue;
            }
            let Some(changes) = &mut consumer.changes else {
                continue;
            };
            changes.changed.clear();
            changes.rebuilt.clear();
            for (&position, &(fields, visit_read)) in consumer.own.iter().zip(&changes.fields) {
                let table = self.tables.tables[position];
                let read = metrics.table_ns[table.index()] != 0;
                let differs = self
                    .tables
                    .differs(position, fields, &frame.tables[table.index()]);
                if fields.is_some() && (frame.keyframe || differs || (visit_read && read)) {
                    engine_allocation::reserve(&mut changes.changed, 1)?;
                    changes.changed.push(table);
                }
                if frame.keyframe || self.tables.rebuilt[position] {
                    engine_allocation::reserve(&mut changes.rebuilt, 1)?;
                    changes.rebuilt.push(table);
                }
            }
            changes.visits = frame.keyframe || !changes.changed.is_empty();
            if !changes.visits {
                let views = &self.tables.views;
                let rebuilt = &self.tables.rebuilt;
                changes.quiet.push(
                    frame.hour,
                    Arc::clone(quiet_metrics.get_or_insert_with(|| Arc::new(metrics.clone()))),
                    consumer.own.iter().map(|&position| {
                        (
                            views[position].as_ref().and_then(SharedView::sampled_at),
                            rebuilt[position],
                        )
                    }),
                )?;
            }
        }
        self.tables.release_before();
        if let Some(owned) = Arc::get_mut(&mut frame) {
            owned.retain_updates(&self.kept)?;
        }
        let (tables, branch, frame) = (&self.tables, self.branch, &frame);
        each_watch(
            self.fanout,
            &mut self.consumers,
            &mut *self.watches,
            |consumer, _| {
                consumer
                    .changes
                    .as_ref()
                    .is_none_or(|changes| changes.visits)
            },
            |consumer, visit| {
                if let Some(changes) = &mut consumer.changes {
                    changes.quiet.flush(&mut *visit)?;
                }
                let decoded = project(&mut consumer.frame, tables, &consumer.own, branch, frame);
                match &consumer.changes {
                    Some(changes) => visit(Watched::Change(BlockChange::Changed {
                        frame: decoded,
                        tables: &changes.changed,
                        rebuilt: &changes.rebuilt,
                    })),
                    None => visit(Watched::Hour {
                        updates: frame,
                        frame: decoded,
                    }),
                }
            },
        )?;
        Ok(self.live())
    }

    /// Hand every watch of changes still reading its quiet hours not yet
    /// visited.
    fn flush(&mut self) -> io::Result<()> {
        each_watch(
            self.fanout,
            &mut self.consumers,
            &mut *self.watches,
            |consumer, _| consumer.changes.is_some(),
            |consumer, visit| match &mut consumer.changes {
                Some(changes) => changes.quiet.flush(visit),
                None => Ok(()),
            },
        )
    }
}

/// Hand every watch still reading that `chosen` picks its `step`: in order
/// here, or with a fanout at once, each [sendable] watch on a helper and
/// every other watch here (one sendable watch too when no other is). The
/// helpers finish before this returns. A step's error stops its watch with
/// that outcome.
///
/// [sendable]: BlockWatch::sendable
fn each_watch(
    fanout: Option<&Fanout>,
    consumers: &mut [Consumer],
    watches: &mut [BlockWatch<'_>],
    chosen: impl Fn(&Consumer, &BlockWatch<'_>) -> bool,
    step: impl Fn(&mut Consumer, &mut Step<'_>) -> io::Result<()> + Sync,
) -> io::Result<()> {
    let count = consumers.len();
    let picked = consumers
        .iter_mut()
        .zip(watches.iter_mut())
        .filter(|(consumer, watch)| !watch.stopped() && chosen(&**consumer, &**watch));
    let Some(fanout) = fanout else {
        for (consumer, watch) in picked {
            visit_one(&step, consumer, watch.visit.step(), &mut watch.outcome);
        }
        return Ok(());
    };
    let mut here = engine_allocation::vec(count)?;
    let mut anywhere = engine_allocation::vec(count)?;
    for (consumer, watch) in picked {
        let BlockWatch { visit, outcome, .. } = watch;
        match visit {
            Visit::Here(visit) => here.push((consumer, &mut **visit, outcome)),
            Visit::Anywhere(visit) => anywhere.push((consumer, visit, outcome)),
        }
    }
    let stays = if here.is_empty() {
        anywhere.pop()
    } else {
        None
    };
    if anywhere.is_empty() {
        for (consumer, visit, outcome) in here {
            visit_one(&step, consumer, visit, outcome);
        }
        if let Some((consumer, visit, outcome)) = stays {
            visit_one(&step, consumer, &mut **visit, outcome);
        }
        return Ok(());
    }
    let step = &step;
    let priority = job_priority::Caller::current();
    let cpu = replay_frame::cpu::Context::capture();
    let progress = crate::progress::Progress::current();
    fanout.0.in_place_scope(|scope| {
        for (consumer, visit, outcome) in anywhere {
            let (priority, cpu, progress) = (priority.clone(), cpu.clone(), progress.clone());
            scope.spawn(move |_| {
                let _progress = progress.map(crate::progress::Progress::install);
                let _cpu = cpu.enter("watch visit");
                priority.enter();
                visit_one(step, consumer, &mut **visit, outcome);
            });
        }
        for (consumer, visit, outcome) in here {
            visit_one(step, consumer, visit, outcome);
        }
        if let Some((consumer, visit, outcome)) = stays {
            visit_one(step, consumer, &mut **visit, outcome);
        }
    });
    Ok(())
}

/// Hand one watch its step; an error stops it with that outcome.
fn visit_one(
    step: &impl Fn(&mut Consumer, &mut Step<'_>) -> io::Result<()>,
    consumer: &mut Consumer,
    visit: &mut Step<'_>,
    outcome: &mut Option<io::Result<()>>,
) {
    if let Err(error) = step(consumer, visit) {
        *outcome = Some(Err(error));
    }
}

/// A visit witnessed without advancing rows: every-hour watches, read
/// hours, and a delta from a complete baseline deleting keys or naming
/// watched fields. A failed delta makes that complete view unavailable,
/// which also visits. Other cases keep their views until the normal
/// decision, including unwatched deltas in a quiet span.
fn visits_before(tables: &BlockTables<'_>, consumer: &Consumer, frame: &RecordedFrame) -> bool {
    let Some(changes) = &consumer.changes else {
        return true;
    };
    consumer
        .own
        .iter()
        .zip(&changes.fields)
        .any(|(&position, &(fields, read))| {
            let table = tables.tables[position];
            fields.is_some()
                && ((read && frame.metrics.table_ns[table.index()] != 0)
                    || match &frame.tables[table.index()] {
                        TableUpdate::Delta {
                            changed, deleted, ..
                        } => match tables.views[position].as_deref() {
                            Some(TableView::Complete(rows)) => {
                                !deleted.is_empty()
                                    || changed.iter().any(|row| {
                                        touches(table, fields, row)
                                            || (row.mask == full_mask(row.recorded_fields)
                                                && !has_key(rows, row.key))
                                    })
                            }
                            _ => false,
                        },
                        _ => false,
                    })
        })
}

fn has_key(rows: &TableRows, key: u128) -> bool {
    let mut low = 0;
    let mut high = rows.len();
    while low < high {
        let middle = low + (high - low) / 2;
        match rows
            .get(middle)
            .expect("a row within the table")
            .key()
            .cmp(&key)
        {
            std::cmp::Ordering::Less => low = middle + 1,
            std::cmp::Ordering::Equal => return true,
            std::cmp::Ordering::Greater => high = middle,
        }
    }
    false
}

/// The frame a watch reading `own` is handed at `frame`'s hour: its tables'
/// views (Campaign too) and the view every unselected table shares for the
/// others, as its own decoder reconstructs them. The frame it was last handed
/// is reused, with each table whose view or sample hour differs replaced.
fn project<'f>(
    kept: &'f mut Option<DecodedFrame>,
    tables: &BlockTables<'_>,
    own: &[usize],
    branch: BranchId,
    frame: &RecordedFrame,
) -> &'f DecodedFrame {
    let view = |position: usize| tables.views[position].clone().unwrap_or_else(not_selected);
    match kept {
        Some(kept) => {
            for &position in own {
                let index = tables.tables[position].index();
                let current = tables.views[position].as_ref();
                let same = current.is_some_and(|current| {
                    SharedView::ptr_eq(&kept.tables[index], current)
                        && kept.tables[index].sampled_at() == current.sampled_at()
                });
                if !same {
                    kept.tables[index] = view(position);
                }
            }
            kept.branch = branch;
            kept.hour = frame.hour;
            kept.metrics = frame.metrics.clone();
        }
        None => {
            *kept = Some(DecodedFrame {
                branch,
                hour: frame.hour,
                tables: std::array::from_fn(|index| match tables.slots[index] {
                    Some(position) if own.contains(&position) => view(position),
                    _ => not_selected(),
                }),
                metrics: frame.metrics.clone(),
            });
        }
    }
    kept.as_ref().expect("a projected frame")
}

// Each table section decoded on this thread, counted in tests.
#[cfg(test)]
thread_local! {
    static DECODED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(super) fn note_decoded() {
    DECODED.with(|count| count.set(count.get() + 1));
}

#[cfg(test)]
pub(super) fn decoded() -> usize {
    DECODED.with(std::cell::Cell::get)
}

#[cfg(test)]
#[path = "watched_tests.rs"]
mod tests;
