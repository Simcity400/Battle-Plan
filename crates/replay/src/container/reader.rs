use super::aliases::private::{
    self,
    streaming::{Resolution, SourceFacts, Sources},
};
use super::grouped::{anchors::TableAnchors, chunks, cursor::TableCursor};
use super::*;
use crate::frame::{
    FrameAdvance, RecordedFrame, SharedView, TableUpdate, advance_public_table, not_selected,
};
use crate::rows::compatibility::HISTORY;
use engine_allocation as allocation;
use std::sync::Arc;

#[cfg(test)]
pub(crate) mod ownership;
pub(super) mod source;
#[cfg(all(test, windows))]
mod source_tests;
mod workers;

#[derive(Debug)]
pub(super) struct FrameFailure {
    hour: Hour,
    error: io::Error,
}

impl std::fmt::Display for FrameFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "invalid frame baseline at hour {}: {}",
            self.hour,
            crate::codec::diagnostic_text(&self.error.to_string())
        )
    }
}

impl std::error::Error for FrameFailure {}

fn begin_frame(
    previous: Option<DecodedFrame>,
    recorded: &RecordedFrame,
    selected: &[bool; TABLE_COUNT],
) -> io::Result<FrameAdvance> {
    FrameAdvance::begin(previous, recorded, selected).map_err(|error| {
        if recording_damage(&error) {
            io::Error::new(
                error.kind(),
                FrameFailure {
                    hour: recorded.hour,
                    error,
                },
            )
        } else {
            error
        }
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReaderSelection {
    tables: Vec<TableId>,
    output: [bool; TABLE_COUNT],
    slots: Arc<[Option<usize>; TABLE_COUNT]>,
}

impl ReaderSelection {
    pub fn new(tables: &[TableId]) -> io::Result<Self> {
        let mut output = [false; TABLE_COUNT];
        output[TableId::Campaign.index()] = true;
        for table in tables {
            output[table.index()] = true;
        }
        let tables = allocation::collect_vec(
            TableId::ALL
                .into_iter()
                .filter(|table| output[table.index()]),
        )?;
        let mut slots = [None; TABLE_COUNT];
        for (slot, table) in tables.iter().enumerate() {
            slots[table.index()] = Some(slot);
        }
        Ok(Self {
            tables,
            output,
            slots: Arc::new(slots),
        })
    }
    pub fn tables(&self) -> &[TableId] {
        &self.tables
    }
    fn decoded(&self, version: u16) -> [bool; TABLE_COUNT] {
        let mut selected = self.output;
        loop {
            let before = selected;
            for table in TableId::ALL {
                if selected[table.index()] && private::stored(table, version) {
                    for source in private::dependencies(table) {
                        selected[source.index()] = true;
                    }
                }
                let change = &HISTORY[table.index()];
                if change.present(version) && change.table_gap(table, version).is_some() {
                    selected[table.index()] = true;
                }
            }
            if before == selected {
                return selected;
            }
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ReaderExecution {
    pub workers: usize,
    pub read_ahead: bool,
}

impl Default for ReaderExecution {
    fn default() -> Self {
        Self {
            workers: std::thread::available_parallelism().map_or(1, usize::from),
            read_ahead: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockReceipt {
    position: usize,
    ordinal: usize,
    index: BlockIndex,
    source_digest: blake3::Hash,
}

impl BlockReceipt {
    pub fn block(&self) -> &BlockIndex {
        &self.index
    }
    pub fn ordinal(&self) -> usize {
        self.ordinal
    }
}

pub enum ReadStep<'a> {
    Hour {
        block: &'a BlockIndex,
        ordinal: usize,
        frame: &'a DecodedFrame,
        updates: &'a RecordedFrame,
    },
    BlockEnd {
        receipt: &'a BlockReceipt,
    },
    Tail,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ReaderCensus {
    pub raw_blocks: usize,
    pub peak_raw_blocks: usize,
    pub anchor_states: usize,
    pub anchor_bootstrap_reads: usize,
    pub anchor_bootstrap_stored_bytes: usize,
    pub anchor_bootstrap_raw_bytes: usize,
    pub anchor_bootstrap_cycles: Option<u64>,

    pub source_check_bytes: u64,
    pub point_validation_stored_bytes: u64,
    pub point_validation_raw_bytes: u64,
}

struct TableState {
    cursor: Option<TableCursor>,
    alias: Option<Resolution>,
    refusal: Option<Arc<TableUpdate>>,
}

struct RawResult {
    table: TableId,
    state: TableState,
    update: io::Result<Arc<TableUpdate>>,
}

struct Grouped {
    chunks: chunks::BlockChunks,
    states: Vec<Option<TableState>>,
    updates: Vec<Option<Arc<TableUpdate>>>,
    errors: Vec<Option<io::Error>>,
    at: usize,
    pending: Option<workers::Pending<RawResult>>,
}

enum Backend {
    Grouped(Grouped),
}

pub(super) struct Core {
    selection: ReaderSelection,
    decoded: [bool; TABLE_COUNT],
    frame: Option<DecodedFrame>,
    hidden: Vec<Option<SharedView>>,
    recorded: Option<RecordedFrame>,
    anchors: Vec<TableAnchors>,
    backend: Option<Backend>,
    index: Option<BlockIndex>,
    workers: workers::Workers,
    stop: Arc<AtomicBool>,
    credits: Arc<chunks::Credits>,
    parallel: bool,
}

impl Core {
    pub(super) fn new(selection: ReaderSelection, workers: usize) -> io::Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let count = workers.max(1).min(TABLE_COUNT);
        Ok(Self {
            selection,
            decoded: [false; TABLE_COUNT],
            frame: None,
            hidden: allocation::collect_vec((0..TABLE_COUNT).map(|_| None))?,
            recorded: None,
            anchors: allocation::collect_vec((0..TABLE_COUNT).map(|_| TableAnchors::default()))?,
            backend: None,
            index: None,
            workers: workers::Workers::new(count, Arc::clone(&stop))?,
            stop,
            credits: Arc::new(chunks::Credits::default()),
            parallel: count > 1,
        })
    }

    pub(super) fn local(selection: ReaderSelection, workers: usize) -> io::Result<Self> {
        let mut core = Self::new(selection, workers)?;
        core.anchors
            .iter_mut()
            .for_each(|state| *state = TableAnchors::local());
        Ok(core)
    }

    pub(super) fn open(
        &mut self,
        replay: &Replay,
        position: usize,
        later: bool,
        ahead: Option<chunks::BlockChunks>,
        check: &mut Check<'_>,
    ) -> io::Result<()> {
        self.recorded = None;
        self.backend = None;
        self.decoded = self.selection.decoded(replay.format.version());
        let index = &replay.index[position];
        self.index = Some(index.clone());
        let mut chunks = match ahead {
            Some(block) => block,
            None => chunks::BlockChunks::load(
                replay,
                position,
                &self.decoded,
                later,
                self.credits.acquire(2)?,
                check,
            )?,
        };
        let mut states = allocation::collect_vec((0..TABLE_COUNT).map(|_| None))?;
        let mut updates = allocation::collect_vec((0..TABLE_COUNT).map(|_| None))?;
        let mut errors = allocation::collect_vec((0..TABLE_COUNT).map(|_| None))?;
        for anchor in &mut self.anchors {
            anchor.admit_ordinal(replay.ordinals[position]);
        }
        for table in TableId::ALL {
            #[cfg(test)]
            let _step = super::check::Scope::enter(super::check::Step::Table(table));
            check.run()?;
            let slot = table.index();
            if !self.decoded[slot] {
                continue;
            }
            let result = (|| {
                if let Some(error) = chunks.failures[slot].take() {
                    return Err(error);
                }
                if let Some(update) = chunks.refusal(replay, table)? {
                    let update = Arc::new(update);
                    return Ok((
                        TableState {
                            cursor: None,
                            alias: None,
                            refusal: Some(Arc::clone(&update)),
                        },
                        update,
                    ));
                }
                let (cursor, update, marks) =
                    TableCursor::begin(replay, &mut chunks, table, &mut self.anchors[slot], check)?;
                let alias = marks
                    .map(|marks| Resolution::new(table, marks))
                    .transpose()?;
                Ok((
                    TableState {
                        cursor: Some(cursor),
                        alias,
                        refusal: None,
                    },
                    Arc::new(update),
                ))
            })();
            match result {
                Ok((state, update)) => {
                    #[cfg(test)]
                    ownership::update(index.first_hour, &update);
                    states[slot] = Some(state);
                    updates[slot] = Some(update);
                }
                Err(error) if check.failed() || !recording_damage(&error) => return Err(error),
                Err(error) => {
                    self.anchors[slot].release();
                    errors[slot] = Some(error);
                }
            }
        }
        self.backend = Some(Backend::Grouped(Grouped {
            chunks,
            states,
            updates,
            errors,
            at: 0,
            pending: None,
        }));
        self.required_errors(replay)
    }

    fn required_errors(&self, replay: &Replay) -> io::Result<()> {
        let Some(Backend::Grouped(group)) = &self.backend else {
            return Ok(());
        };
        let required = self.required(group, replay);
        for table in TableId::ALL {
            if required[table.index()]
                && let Some(error) = &group.errors[table.index()]
            {
                return Err(io::Error::new(error.kind(), error.to_string()));
            }
        }
        Ok(())
    }

    fn required(&self, group: &Grouped, replay: &Replay) -> [bool; TABLE_COUNT] {
        let mut required = self.selection.output;
        for table in TableId::ALL {
            if HISTORY[table.index()]
                .table_gap(table, replay.format.version())
                .is_some()
            {
                required[table.index()] = true;
            }
        }
        loop {
            let before = required;
            for table in TableId::ALL {
                if required[table.index()]
                    && let Some(alias) = group.states[table.index()]
                        .as_ref()
                        .and_then(|state| state.alias.as_ref())
                {
                    for source in alias.sources() {
                        required[source.index()] = true;
                    }
                }
            }
            if before == required {
                return required;
            }
        }
    }

    fn raw_job(
        (table, mut state, previous, hour, entered): (TableId, TableState, Hour, Hour, bool),
        check: &mut Check<'_>,
    ) -> RawResult {
        let update = match &mut state.cursor {
            Some(cursor) => cursor.next(previous, entered, check).map(Arc::new),
            None => check
                .run()
                .map(|_| Arc::clone(state.refusal.as_ref().unwrap())),
        };
        #[cfg(test)]
        if let Ok(update) = &update {
            ownership::update(hour, update);
        }
        #[cfg(not(test))]
        let _ = hour;
        RawResult {
            table,
            state,
            update,
        }
    }

    fn start_raw(&mut self, replay: &Replay, check: &mut Check<'_>) -> io::Result<()> {
        let Some(Backend::Grouped(group)) = &mut self.backend else {
            return Ok(());
        };
        if group.at == 0 || group.at == group.chunks.hours.len() || group.pending.is_some() {
            return Ok(());
        }
        let mut jobs = Vec::new();
        for table in TableId::ALL {
            if group.errors[table.index()].is_some() {
                continue;
            }
            if let Some(mut state) = group.states[table.index()].take() {
                let prepared = match &mut state.cursor {
                    Some(cursor) => cursor.prepare(
                        replay,
                        &mut group.chunks,
                        group.at,
                        state.alias.as_mut(),
                        check,
                    ),
                    None => Ok(()),
                };
                if let Err(error) = prepared {
                    if check.failed() || !recording_damage(&error) {
                        return Err(error);
                    }
                    group.errors[table.index()] = Some(error);
                    continue;
                }
                allocation::reserve(&mut jobs, 1)?;
                let entered = state.cursor.is_some() && group.chunks.entered(table, group.at);
                jobs.push((
                    table,
                    state,
                    group.chunks.hours[group.at - 1],
                    group.chunks.hours[group.at],
                    entered,
                ));
            }
        }
        if self.parallel {
            group.pending = Some(self.workers.start(jobs, Self::raw_job)?);
        } else {
            for job in jobs {
                let result = Self::raw_job(job, check);
                Self::install_raw(group, result, check)?;
            }
        }
        Ok(())
    }

    fn install_raw(group: &mut Grouped, result: RawResult, check: &Check<'_>) -> io::Result<()> {
        let slot = result.table.index();
        match result.update {
            Ok(update) => {
                group.states[slot] = Some(result.state);
                group.updates[slot] = Some(update);
            }
            Err(error) if check.failed() || !recording_damage(&error) => return Err(error),
            Err(error) => {
                group.errors[slot] = Some(error);
                group.states[slot] = None;
            }
        }
        Ok(())
    }

    pub(super) fn done(&self) -> bool {
        match &self.backend {
            Some(Backend::Grouped(group)) => group.at == group.chunks.hours.len(),
            None => true,
        }
    }

    pub(super) fn advance(&mut self, replay: &Replay, check: &mut Check<'_>) -> io::Result<()> {
        let _cpu = crate::cpu::scope("reconstruct", None);
        let measured = crate::cpu::enabled();
        let mut spent = Vec::new();
        check.run()?;
        self.recorded = None;
        self.start_raw(replay, check)?;
        if let Some(Backend::Grouped(group)) = &mut self.backend
            && let Some(pending) = group.pending.take()
        {
            for result in pending.finish(&self.stop, check)? {
                Self::install_raw(group, result, check)?;
            }
        }
        self.required_errors(replay)?;
        let Some(backend) = &mut self.backend else {
            return Err(invalid("reader has no active block"));
        };
        let mut previous = self.frame.take();
        if let Some(frame) = &mut previous {
            for (slot, view) in self.hidden.iter_mut().enumerate() {
                if let Some(view) = view.take() {
                    frame.tables[slot] = view;
                }
            }
        }
        let recorded;
        let mut reached;
        match backend {
            Backend::Grouped(group) => {
                let at = group.at;
                let hour = group.chunks.hours[at];
                let metrics = group.chunks.diagnostics.next(check)?;
                // The skeleton contains only this hour; failed hidden sources are consulted lazily.
                let skeleton = RecordedFrame::from_updates(
                    group.chunks.index.branch,
                    hour,
                    at == 0,
                    metrics.clone(),
                    Arc::new(std::array::from_fn(Some)),
                    allocation::collect_vec((0..TABLE_COUNT).map(|slot| {
                        group.updates[slot]
                            .as_ref()
                            .map_or_else(unselected_update, Arc::clone)
                    }))?,
                    unselected_update(),
                )?;
                reached = begin_frame(previous, &skeleton, &self.decoded)?;
                drop(skeleton);
                let mut facts =
                    allocation::collect_vec((0..TABLE_COUNT).map(|_| SourceFacts::default()))?;
                let mut resolved: Vec<Option<Arc<TableUpdate>>> =
                    allocation::collect_vec((0..TABLE_COUNT).map(|_| None))?;
                let mut remaining = self.decoded;
                while remaining.iter().any(|pending| *pending) {
                    check.run()?;
                    let wave = allocation::collect_vec(TableId::ALL.into_iter().filter(|table| {
                        remaining[table.index()]
                            && (!private::stored(*table, replay.format.version())
                                || private::dependencies(*table)
                                    .iter()
                                    .all(|source| !remaining[source.index()]))
                    }))?;
                    if wave.is_empty() {
                        return Err(invalid("private alias dependency cycle"));
                    }
                    let mut apply = Vec::new();
                    for table in wave {
                        #[cfg(test)]
                        let _step =
                            super::check::Scope::enter(super::check::Step::Group(table, at));
                        check.run()?;
                        let slot = table.index();
                        let update = if let Some(raw) = group.updates[slot].take() {
                            let state = group.states[slot].as_mut().unwrap();
                            match &mut state.alias {
                                Some(alias) => {
                                    #[cfg(test)]
                                    let _memory = ownership::category(ownership::Kind::Aliases);
                                    let raw = Arc::try_unwrap(raw).map_err(|_| {
                                        io::Error::other("raw update retained across hour barrier")
                                    })?;
                                    let sources = Sources {
                                        frame: &reached,
                                        updates: &resolved,
                                        facts: &facts,
                                        errors: &group.errors,
                                    };
                                    alias
                                        .hour(
                                            &group.chunks.hours,
                                            at,
                                            raw,
                                            state.cursor.as_ref().and_then(TableCursor::marks),
                                            reached.previous(table),
                                            &sources,
                                            check,
                                        )
                                        .map(Arc::new)
                                }
                                None => Ok(raw),
                            }
                        } else {
                            Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                "alias source failed decoding",
                            ))
                        };
                        match update {
                            Ok(update) => {
                                #[cfg(test)]
                                ownership::update(hour, &update);
                                facts[slot] = SourceFacts::capture(
                                    table,
                                    reached.previous(table).map(|(_, view)| view),
                                    &update,
                                )?;
                                resolved[slot] = Some(Arc::clone(&update));
                                allocation::reserve(&mut apply, 1)?;
                                apply.push((
                                    table,
                                    update,
                                    reached.take_previous(table),
                                    hour,
                                    metrics.table_ns[slot] != 0,
                                    !self.selection.output[slot],
                                ));
                            }
                            Err(error) if check.failed() || !recording_damage(&error) => {
                                return Err(error);
                            }
                            Err(error) => {
                                if group.errors[slot].is_none() {
                                    group.errors[slot] = Some(error);
                                }
                                group.states[slot] = None;
                                reached.take_previous(table);
                                reached.install(table, not_selected());
                            }
                        }
                        remaining[slot] = false;
                    }
                    let apply_one =
                        move |(table, update, previous, hour, read, hidden): (
                            TableId,
                            Arc<TableUpdate>,
                            Option<(Hour, SharedView)>,
                            Hour,
                            bool,
                            bool,
                        ),
                              check: &mut Check<'_>| {
                            #[cfg(test)]
                            let _memory = ownership::category(if hidden {
                                ownership::Kind::Hidden
                            } else {
                                ownership::Kind::Frame
                            });
                            #[cfg(not(test))]
                            let _ = hidden;
                            let started = if measured { crate::cpu::stamp() } else { 0 };
                            let view = check.run().and_then(|_| {
                                advance_public_table(table, &update, previous, hour, read)
                            });
                            #[cfg(test)]
                            if let Ok(view) = &view {
                                ownership::view(table, view, hidden);
                            }
                            (
                                table,
                                view,
                                if measured {
                                    crate::cpu::stamp().wrapping_sub(started)
                                } else {
                                    0
                                },
                            )
                        };
                    let applied = if self.parallel {
                        self.workers
                            .start(apply, apply_one)?
                            .finish(&self.stop, check)?
                    } else {
                        apply
                            .into_iter()
                            .map(|item| apply_one(item, check))
                            .collect()
                    };
                    for (table, view, cycles) in applied {
                        reached.install(table, view?);
                        if measured {
                            spent.push((table.index(), cycles));
                        }
                    }
                }
                let updates = allocation::collect_vec(self.selection.tables.iter().map(|table| {
                    resolved[table.index()]
                        .take()
                        .unwrap_or_else(unselected_update)
                }))?;
                recorded = RecordedFrame::from_updates(
                    group.chunks.index.branch,
                    hour,
                    at == 0,
                    metrics,
                    Arc::clone(&self.selection.slots),
                    updates,
                    unselected_update(),
                )?;
                group.at += 1;
            }
        }
        self.required_errors(replay)?;
        let mut frame = reached.finish()?;
        if measured {
            crate::cpu::reconstructed(frame.hour, &spent);
        }
        validate_campaign(&frame, &replay.campaign_id)?;
        for table in TableId::ALL {
            let slot = table.index();
            if self.decoded[slot] && !self.selection.output[slot] {
                self.hidden[slot] =
                    Some(std::mem::replace(&mut frame.tables[slot], not_selected()));
            }
        }
        check.run()?;
        self.frame = Some(frame);
        self.recorded = Some(recorded);
        // Raw decoding may overlap the caller's borrow; applying waits for its return.
        if self.parallel {
            self.start_raw(replay, check)?;
        }
        Ok(())
    }

    pub(super) fn finish(
        &mut self,
        replay: &Replay,
        check: &mut Check<'_>,
    ) -> io::Result<blake3::Hash> {
        if !self.done() {
            return Err(invalid("block has unread hours"));
        }
        let digest = match self
            .backend
            .take()
            .ok_or_else(|| invalid("reader has no active block"))?
        {
            Backend::Grouped(mut group) => {
                let required = self.required(&group, replay);
                for table in TableId::ALL {
                    check.run()?;
                    if let Some(state) = &group.states[table.index()] {
                        let result = (|| {
                            if let Some(cursor) = &state.cursor {
                                cursor.finish()?;
                            }
                            if let Some(alias) = &state.alias {
                                alias.finish(state.cursor.as_ref().and_then(TableCursor::marks))?;
                            }
                            Ok(())
                        })();
                        if let Err(error) = result {
                            // A dormant source has the legacy prefix semantics until somebody uses it.
                            if required[table.index()] {
                                return Err(error);
                            }
                        }
                    }
                }
                group.chunks.finish(replay, check)?
            }
        };
        check.run()?;
        replay.mark_verified(self.index.as_ref().unwrap());
        Ok(digest)
    }

    pub(super) fn validate_point_envelopes(
        &self,
        replay: &Replay,
        check: &mut Check<'_>,
    ) -> io::Result<()> {
        if let Some(Backend::Grouped(group)) = &self.backend {
            group
                .chunks
                .validate_point_envelopes(replay, &self.required(group, replay), check)?;
        }
        Ok(())
    }

    pub(super) fn current(&self) -> Option<&DecodedFrame> {
        self.frame.as_ref()
    }
    pub(super) fn can_continue_point(
        &self,
        branch: BranchId,
        hour: Hour,
        selection: &ReaderSelection,
    ) -> bool {
        self.selection == *selection
            && self
                .frame
                .as_ref()
                .is_some_and(|frame| frame.branch == branch && frame.hour <= hour)
            && self
                .index
                .as_ref()
                .is_some_and(|index| hour <= index.last_hour)
            && matches!(self.backend, Some(Backend::Grouped(_)))
    }
    pub(super) fn next_hour(
        &mut self,
        _replay: &Replay,
        _check: &mut Check<'_>,
    ) -> io::Result<Option<Hour>> {
        match &mut self.backend {
            Some(Backend::Grouped(group)) => Ok(group.chunks.hours.get(group.at).copied()),
            None => Ok(None),
        }
    }
    pub(super) fn finish_point(
        &mut self,
        replay: &Replay,
        check: &mut Check<'_>,
    ) -> io::Result<()> {
        if self.done() {
            self.finish(replay, check)?;
        }
        Ok(())
    }
    pub(super) fn take_frame(&mut self) -> Option<DecodedFrame> {
        self.frame.take()
    }
    pub(super) fn census(&self) -> ReaderCensus {
        let (raw_blocks, peak_raw_blocks) = self.credits.counts();
        ReaderCensus {
            raw_blocks,
            peak_raw_blocks,
            anchor_states: self.anchors.iter().map(TableAnchors::retained).sum(),
            anchor_bootstrap_reads: self
                .anchors
                .iter()
                .map(|state| state.bootstrap_reads.sections)
                .sum(),
            anchor_bootstrap_stored_bytes: self
                .anchors
                .iter()
                .map(|state| state.bootstrap_reads.stored_bytes)
                .sum(),
            anchor_bootstrap_raw_bytes: self
                .anchors
                .iter()
                .map(|state| state.bootstrap_reads.raw_bytes)
                .sum(),
            anchor_bootstrap_cycles: self
                .anchors
                .iter()
                .filter(|state| state.bootstrap_reads.sections != 0)
                .map(|state| state.bootstrap_reads.cycles)
                .sum(),
            source_check_bytes: self.credits.source_bytes.load(Ordering::Relaxed),
            point_validation_stored_bytes: self.credits.validation_stored.load(Ordering::Relaxed),
            point_validation_raw_bytes: self.credits.validation_raw.load(Ordering::Relaxed),
        }
    }
    fn abort(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.backend = None;
        self.recorded = None;
        self.frame = None;
        self.hidden.iter_mut().for_each(|view| *view = None);
        self.anchors.iter_mut().for_each(TableAnchors::release);
        self.workers.shutdown();
    }
}

fn unselected_update() -> Arc<TableUpdate> {
    static VALUE: std::sync::LazyLock<Arc<TableUpdate>> = std::sync::LazyLock::new(|| {
        Arc::new(TableUpdate::Unavailable {
            reason: "Table not selected for reconstruction".into(),
        })
    });
    Arc::clone(&VALUE)
}

pub struct ReaderCheckpoint {
    selection: ReaderSelection,
    cursor: ReplayCursor,
    anchors: Vec<TableAnchors>,
    source: Option<Arc<Replay>>,
    prefix: Option<source::Prefix>,
}

pub struct SequentialReader {
    core: Core,
    cursor: ReplayCursor,
    receipt: Option<BlockReceipt>,
    ahead: Option<workers::Ahead>,
    source: Option<Arc<Replay>>,
    poisoned: bool,
    prefix: Option<source::Prefix>,
    revalidate: bool,
    lease: Option<source::Lease>,
    block_ordinal: Option<usize>,
    block_done: bool,
}

impl SequentialReader {
    pub fn new(selection: ReaderSelection, execution: ReaderExecution) -> io::Result<Self> {
        let core = Core::new(selection, execution.workers)?;
        let ahead = execution
            .read_ahead
            .then(|| workers::Ahead::new(Arc::clone(&core.stop)))
            .transpose()?;
        Ok(Self {
            core,
            cursor: ReplayCursor::default(),
            receipt: None,
            ahead,
            source: None,
            poisoned: false,
            prefix: Some(source::Prefix::default()),
            revalidate: false,
            lease: None,
            block_ordinal: None,
            block_done: false,
        })
    }
    /// Stream only this original physical block; acceptance does not advance a file prefix.
    pub fn for_block(
        selection: ReaderSelection,
        ordinal: usize,
        execution: ReaderExecution,
    ) -> io::Result<Self> {
        let mut reader = Self::new(
            selection,
            ReaderExecution {
                read_ahead: false,
                ..execution
            },
        )?;
        reader
            .core
            .anchors
            .iter_mut()
            .for_each(|state| *state = TableAnchors::local());
        reader.block_ordinal = Some(ordinal);
        Ok(reader)
    }
    fn position(&self, replay: &Replay) -> io::Result<usize> {
        match self.block_ordinal {
            Some(ordinal) => replay
                .ordinals
                .binary_search(&ordinal)
                .map_err(|_| invalid("reader block ordinal is absent")),
            None => Ok(self.cursor.next_block),
        }
    }
    pub fn cursor(&self) -> &ReplayCursor {
        &self.cursor
    }
    pub fn current(&self) -> Option<&DecodedFrame> {
        self.core.current()
    }
    pub fn selection(&self) -> &ReaderSelection {
        &self.core.selection
    }
    pub fn census(&self) -> ReaderCensus {
        self.core.census()
    }

    pub fn into_checkpoint(mut self) -> io::Result<(ReaderCheckpoint, Option<DecodedFrame>)> {
        if self.block_ordinal.is_some()
            || self.poisoned
            || self.receipt.is_some()
            || self.core.backend.is_some()
        {
            return Err(invalid(
                "reader checkpoint requires an accepted block boundary",
            ));
        }
        self.ahead = None;
        let checkpoint = ReaderCheckpoint {
            selection: self.core.selection.clone(),
            cursor: self.cursor.clone(),
            anchors: std::mem::take(&mut self.core.anchors),
            source: self.source.take(),
            prefix: self.prefix.take(),
        };
        Ok((checkpoint, self.core.frame.take()))
    }

    pub fn resume(
        selection: ReaderSelection,
        checkpoint: ReaderCheckpoint,
        frame: Option<DecodedFrame>,
        execution: ReaderExecution,
    ) -> io::Result<Self> {
        if checkpoint.selection != selection {
            return Err(invalid("reader checkpoint selection mismatch"));
        }
        if checkpoint
            .cursor
            .previous
            .as_ref()
            .map(|index| (index.branch, index.last_hour))
            != frame.as_ref().map(|frame| (frame.branch, frame.hour))
        {
            return Err(invalid("reader checkpoint frame mismatch"));
        }
        let mut reader = Self::new(selection, execution)?;
        reader.cursor = checkpoint.cursor;
        reader.core.anchors = checkpoint.anchors;
        reader.core.frame = frame;
        reader.source = checkpoint.source;
        reader.prefix = checkpoint.prefix;
        reader.revalidate = true;
        Ok(reader)
    }

    pub fn advance(
        &mut self,
        replay: &Arc<Replay>,
        mut cancel: impl FnMut() -> io::Result<()>,
    ) -> io::Result<ReadStep<'_>> {
        let mut check = Check::new(&mut cancel);
        if let Err(error) = self.step(replay, &mut check).and_then(|_| check.run()) {
            self.poisoned = true;
            self.ahead = None;
            self.core.abort();
            self.prefix = None;
            self.source = None;
            self.lease = None;
            return Err(error);
        }
        if let Some(receipt) = &self.receipt {
            return Ok(ReadStep::BlockEnd { receipt });
        }
        if self.core.backend.is_none() {
            return Ok(ReadStep::Tail);
        }
        Ok(ReadStep::Hour {
            block: self.core.index.as_ref().unwrap(),
            ordinal: replay.ordinals[self.position(replay)?],
            frame: self.core.frame.as_ref().unwrap(),
            updates: self.core.recorded.as_ref().unwrap(),
        })
    }

    fn step(&mut self, replay: &Arc<Replay>, check: &mut Check<'_>) -> io::Result<()> {
        if self.poisoned {
            return Err(invalid(
                "reader is poisoned; restore the committed checkpoint",
            ));
        }
        check.run()?;
        if self.block_done {
            return Ok(());
        }
        self.check_source(replay, check)?;
        if self.receipt.is_some() {
            return Ok(());
        }
        if self.core.backend.is_some() && self.core.done() {
            let index = &replay.index[self.position(replay)?];
            self.lease = source::Lease::acquire(replay, index.offset + index.length)?;
            let source_digest = self.core.finish(replay, check)?;
            self.receipt = Some(BlockReceipt {
                position: self.position(replay)?,
                ordinal: replay.ordinals[self.position(replay)?],
                index: replay.index[self.position(replay)?].clone(),
                source_digest,
            });
            return Ok(());
        }
        if self.core.backend.is_none() {
            let position = self.position(replay)?;
            if position == replay.index.len() {
                return Ok(());
            }
            let ahead = match &mut self.ahead {
                Some(ahead) if ahead.pending == Some(position) => Some(ahead.take(check)?),
                _ => None,
            };
            self.lease = source::Lease::acquire(replay, replay.index[position].offset)?;
            self.core.open(replay, position, true, ahead, check)?;
            if let Some(ahead) = &mut self.ahead
                && matches!(self.core.backend, Some(Backend::Grouped(_)))
                && position + 1 < replay.index.len()
            {
                ahead.request(
                    Arc::clone(replay),
                    position + 1,
                    self.core.decoded,
                    self.core.credits.acquire(2)?,
                )?;
            }
        }
        self.core.advance(replay, check)
    }

    fn check_source(&mut self, replay: &Arc<Replay>, check: &mut Check<'_>) -> io::Result<()> {
        if self.block_ordinal.is_some() {
            if self
                .source
                .as_ref()
                .is_some_and(|old| !Arc::ptr_eq(old, replay))
            {
                return Err(invalid("block reader source revision changed"));
            }
            self.source = Some(Arc::clone(replay));
            return Ok(());
        }
        if self.cursor.next_block > replay.index.len()
            || self
                .cursor
                .next_block
                .checked_sub(1)
                .map(|at| &replay.index[at])
                != self.cursor.previous.as_ref()
        {
            return Err(invalid("preparation cursor differs from recording prefix"));
        }
        let replaced = self
            .source
            .as_ref()
            .is_some_and(|old| !Arc::ptr_eq(old, replay));
        if let Some(old) = &self.source
            && (replaced || self.revalidate)
        {
            if old.format != replay.format
                || old.capture_mode != replay.capture_mode
                || old.campaign_id != replay.campaign_id
                || old.identity != replay.identity
                || old.index.len() > replay.index.len()
                || old.index != replay.index[..old.index.len()]
                || old.ordinals != replay.ordinals[..old.ordinals.len()]
            {
                return Err(invalid("recording differs from reader source prefix"));
            }
        }
        if (replaced || self.revalidate)
            && !self
                .lease
                .as_ref()
                .map(|lease| lease.same_file(replay))
                .transpose()?
                .unwrap_or(false)
        {
            let blocks = self.prefix.as_ref().map_or(0, source::Prefix::blocks);
            let end = self.receipt.as_ref().map_or_else(
                || {
                    blocks
                        .checked_sub(1)
                        .map_or(0, |at| replay.index[at].offset + replay.index[at].length)
                },
                |receipt| receipt.index.offset + receipt.index.length,
            );
            let lease = source::Lease::acquire(replay, end)?;
            if let Some(prefix) = &self.prefix {
                prefix.verify(replay, &self.core.credits.source_bytes, check)?;
            }
            if let Some(receipt) = &self.receipt
                && source::block_digest(
                    replay,
                    &receipt.index,
                    &self.core.credits.source_bytes,
                    check,
                )? != receipt.source_digest
            {
                return Err(invalid("recording differs from reader source prefix"));
            }
            self.lease = lease;
        }
        self.revalidate = false;
        self.source = Some(Arc::clone(replay));
        Ok(())
    }

    pub fn accept_block(&mut self, receipt: &BlockReceipt) -> io::Result<()> {
        if self.poisoned || self.receipt.as_ref() != Some(receipt) {
            return Err(invalid("block receipt does not match pending boundary"));
        }
        if self.block_ordinal.is_some() {
            self.block_done = true;
            self.core.abort();
            self.receipt = None;
            self.source = None;
            self.lease = None;
            return Ok(());
        }
        self.prefix.as_mut().unwrap().accept(
            self.source.as_ref().unwrap(),
            receipt.position,
            receipt.source_digest,
        )?;
        self.cursor.next_block = receipt.position + 1;
        self.cursor.previous = Some(receipt.index.clone());
        self.receipt = None;
        Ok(())
    }
}

impl Drop for SequentialReader {
    fn drop(&mut self) {
        self.core.stop.store(true, Ordering::Release);
        self.ahead = None;
        self.core.abort();
    }
}
