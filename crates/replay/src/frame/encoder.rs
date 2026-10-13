use super::{
    BaselineToken, DecodedFrame, EncodedFrame, Frame, FrameInput, TABLE_COUNT, TableInput,
    TableView, put_count, validate_complete, validate_rows, validate_singleton,
};
use crate::{
    BranchId, Hour, KEYFRAME_INTERVAL,
    codec::{invalid_static as invalid, put_bytes, put_string},
    format::FormatVersion,
    rows::{TableId, TableRows, TableSchema},
};
use engine::allocation;
use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

pub(super) enum BaselineRows {
    Owned(TableRows),
    Shared(Arc<TableRows>),
}

impl AsRef<TableRows> for BaselineRows {
    fn as_ref(&self) -> &TableRows {
        match self {
            Self::Owned(rows) => rows,
            Self::Shared(rows) => rows,
        }
    }
}

pub(super) struct Baselines {
    branch: Option<BranchId>,
    pub(super) last_hour: Option<Hour>,
    pub(super) tables: [Option<(Hour, BaselineRows)>; TABLE_COUNT],
}

impl Default for Baselines {
    fn default() -> Self {
        Self {
            branch: None,
            last_hour: None,
            tables: std::array::from_fn(|_| None),
        }
    }
}
/// An encoded record whose baselines have not yet been committed.
pub struct PreparedFrame {
    frame: Option<EncodedFrame>,
    source: FrameInput,
    unchanged: [bool; TABLE_COUNT],
    revision: u64,
    encoder: u64,
    previous_revision: u64,
    keyframe: bool,
}

impl PreparedFrame {
    pub fn frame(&self) -> &EncodedFrame {
        self.frame.as_ref().expect("prepared record not taken")
    }
    /// Move the bytes to the spool without copying; retain the pending baselines.
    pub fn take_frame(&mut self) -> EncodedFrame {
        self.frame.take().expect("prepared record already taken")
    }
}

/// The encoder state and verdicts one frame's tables are encoded against.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameContext {
    encoder: u64,
    revision: u64,
    branch: BranchId,
    hour: Hour,
    force: bool,
    keyframe: bool,
    new_branch: bool,
}

/// One table's update bytes, encoded beside its frame (possibly on another
/// thread) by [`FrameEncoder::encode_segment`]; the frame takes them in table
/// order ([`FrameEncoder::prepare_bytes_with`]).
pub struct TableSegment {
    context: FrameContext,
    table: TableId,
    bytes: Vec<u8>,
    unchanged: bool,
}

/// What a commit no longer holds: replaced baselines and rows, and inputs it
/// did not keep. Dropping this frees them, on whichever thread drops it.
#[derive(Default)]
pub struct Retired(Vec<Retiring>);

#[allow(dead_code)] // Held only to be dropped.
enum Retiring {
    Baseline(BaselineRows),
    Input(TableInput),
    Rows(TableRows),
}

impl Retired {
    /// How many baselines, inputs and row sets this holds.
    pub fn len(&self) -> usize {
        self.0.len()
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

pub struct FrameEncoder {
    pub(super) state: Baselines,
    keyframe_hour: Option<Hour>,
    identity: u64,
    pub(super) revision: u64,
    deleted: Vec<u8>,
}

impl Default for FrameEncoder {
    fn default() -> Self {
        static NEXT_ID: AtomicU64 = AtomicU64::new(1);
        Self {
            state: Baselines::default(),
            keyframe_hour: None,
            revision: 0,
            deleted: Vec::new(),
            identity: NEXT_ID
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
                .expect("encoder identity space exhausted"),
        }
    }
}

impl FrameEncoder {
    pub fn new() -> Self {
        Self::default()
    }
    fn needs_keyframe(&self, branch: BranchId, hour: Hour, force: bool) -> bool {
        force
            || self.state.branch != Some(branch)
            || self.keyframe_hour.is_none_or(|base| {
                i64::from(hour) - i64::from(base) >= i64::from(KEYFRAME_INTERVAL)
            })
    }

    /// A producer may leave the reserved cadence marker in this slot instead
    /// of cloning already validated rows. Missing/partial baselines, branch
    /// starts and keyframes still require the ordinary complete input.
    pub fn can_carry(&self, table: TableId, branch: BranchId, hour: Hour, force: bool) -> bool {
        crate::cadence::is_staggered(table)
            && !self.needs_keyframe(branch, hour, force)
            && self.state.last_hour.is_some_and(|previous| hour > previous)
            && self.state.tables[table.index()].is_some()
    }

    /// The token a comparison at `hour` names and the baseline it compares
    /// with, when the hour follows the baseline's and needs no keyframe.
    fn token(
        &self,
        table: TableId,
        branch: BranchId,
        hour: Hour,
        force: bool,
    ) -> Option<(BaselineToken, &BaselineRows)> {
        // Clock payloads depend on the enclosing hour and therefore cannot be
        // reused through a comparison with the preceding hour's complete rows.
        if table == TableId::Clock
            || self.needs_keyframe(branch, hour, force)
            || self
                .state
                .last_hour
                .and_then(|previous| previous.checked_add(1))
                != Some(hour)
        {
            return None;
        }
        let (baseline, values) = self.state.tables[table.index()].as_ref()?;
        Some((
            BaselineToken {
                encoder: self.identity,
                revision: self.revision,
                table,
                branch,
                hour,
                baseline: *baseline,
                row_updates: matches!(values, BaselineRows::Owned(_))
                    && table != TableId::Countries
                    && !crate::cadence::is_staggered(table),
            },
            values,
        ))
    }

    /// The latest committed complete rows of `T` and the hour that supplied
    /// them. `None` when this encoder holds no complete baseline for `T`: never
    /// committed, or truncated or unavailable at its last sample.
    pub fn committed<T: TableSchema>(&self) -> Option<(Hour, &[T::Row])> {
        let (hour, values) = self.state.tables[T::ID.index()].as_ref()?;
        Some((*hour, values.as_ref().rows::<T>()?))
    }

    /// Borrow a complete baseline for comparison with a new native sample.
    /// Callers must check all current acquisition/arena/uniqueness verdicts and
    /// exact row payloads before submitting `ComparedEqual`. The borrow is used
    /// only during conversion; no native capture or replay-row lease is retained.
    pub fn comparison_baseline<T: TableSchema>(
        &self,
        branch: BranchId,
        hour: Hour,
        force: bool,
    ) -> Option<(BaselineToken, &[T::Row])> {
        let (token, values) = self.token(T::ID, branch, hour, force)?;
        Some((token, values.as_ref().rows::<T>()?))
    }

    /// The shared complete baseline an owner-sliced table's
    /// [`TableInput::SliceUpdate`] at `hour` would update, with its token.
    pub fn slice_baseline(
        &self,
        table: TableId,
        branch: BranchId,
        hour: Hour,
        force: bool,
    ) -> Option<(BaselineToken, &Arc<TableRows>)> {
        match self.token(table, branch, hour, force)? {
            (token, BaselineRows::Shared(rows)) => Some((token, rows)),
            (_, BaselineRows::Owned(_)) => None,
        }
    }

    /// A table's committed baseline when it is shared with its producer.
    pub fn shared_baseline(&self, table: TableId) -> Option<&Arc<TableRows>> {
        match self.state.tables[table.index()].as_ref()? {
            (_, BaselineRows::Shared(rows)) => Some(rows),
            (_, BaselineRows::Owned(_)) => None,
        }
    }

    fn encode_complete(
        &self,
        table: TableId,
        values: &TableRows,
        hour: Hour,
        keyframe: bool,
        output: &mut Vec<u8>,
        deleted: &mut Vec<u8>,
    ) -> io::Result<bool> {
        validate_complete(table, values, hour)?;
        let Some((baseline, previous)) = self.state.tables[table.index()]
            .as_ref()
            .filter(|_| !keyframe)
        else {
            output.push(0);
            put_count(output, values.len())?;
            values.encode_rows(output)?;
            return Ok(false);
        };
        let start = output.len();
        if values != previous.as_ref() {
            output.push(1);
            output.extend_from_slice(&baseline.to_le_bytes());
            if values.encode_changes(previous.as_ref(), output, deleted)? {
                return Ok(false);
            }
            output.truncate(start);
        }
        output.push(2);
        output.extend_from_slice(&baseline.to_le_bytes());
        Ok(true)
    }

    fn validate_comparison(
        &self,
        token: BaselineToken,
        table: TableId,
        branch: BranchId,
        hour: Hour,
        keyframe: bool,
    ) -> io::Result<()> {
        if keyframe
            || token.encoder != self.identity
            || token.revision != self.revision
            || table == TableId::Clock
            || token.table != table
            || token.branch != branch
            || token.hour != hour
            || self
                .state
                .last_hour
                .and_then(|previous| previous.checked_add(1))
                != Some(hour)
            || self.state.tables[table.index()]
                .as_ref()
                .map(|entry| entry.0)
                != Some(token.baseline)
        {
            return Err(invalid(
                "compared table does not name the current complete baseline",
            ));
        }
        Ok(())
    }

    /// Materialize an editable frame for callers that need its decoded updates.
    /// Recording uses encode_bytes directly and never performs this decoding.
    pub fn encode(&mut self, source: DecodedFrame, force_keyframe: bool) -> io::Result<Frame> {
        self.encode_input(source.into(), force_keyframe)
    }

    pub fn encode_input(&mut self, source: FrameInput, force_keyframe: bool) -> io::Result<Frame> {
        self.encode_bytes(source, force_keyframe)?.decode()
    }

    /// Convenience for callers without an external record sink.
    /// Collector recording uses `prepare_bytes`, writes the complete spool record,
    /// then calls `commit` so failed writes never advance a baseline.
    pub fn encode_bytes(
        &mut self,
        source: FrameInput,
        force_keyframe: bool,
    ) -> io::Result<EncodedFrame> {
        let mut prepared = self.prepare_bytes(source, force_keyframe)?;
        let frame = prepared.take_frame();
        self.commit(prepared)?;
        Ok(frame)
    }

    /// The context a frame for `branch` and `hour` is encoded in.
    pub fn frame_context(
        &self,
        branch: BranchId,
        hour: Hour,
        force_keyframe: bool,
    ) -> FrameContext {
        FrameContext {
            encoder: self.identity,
            revision: self.revision,
            branch,
            hour,
            force: force_keyframe,
            keyframe: self.needs_keyframe(branch, hour, force_keyframe),
            new_branch: self.state.branch != Some(branch),
        }
    }

    /// Encode one table's update as [`Self::prepare_bytes`] encodes it in
    /// `context`'s frame. Borrows the encoder shared, so several tables of a
    /// frame may be encoded at once on different threads.
    pub fn encode_segment(
        &self,
        context: &FrameContext,
        table: TableId,
        input: &TableInput,
    ) -> io::Result<TableSegment> {
        if context.encoder != self.identity || context.revision != self.revision {
            return Err(invalid("segment context does not name this encoder state"));
        }
        let (mut bytes, mut deleted) = (Vec::new(), Vec::new());
        let unchanged = self.encode_table(context, table, input, &mut bytes, &mut deleted)?;
        Ok(TableSegment {
            context: *context,
            table,
            bytes,
            unchanged,
        })
    }

    pub fn prepare_bytes(
        &mut self,
        source: FrameInput,
        force_keyframe: bool,
    ) -> io::Result<PreparedFrame> {
        self.prepare_bytes_with(source, force_keyframe, &mut std::array::from_fn(|_| None))
    }

    /// [`Self::prepare_bytes`], taking each table's update from `segments`
    /// where one was encoded for this frame's context and table, and encoding
    /// the others here. The bytes are those `prepare_bytes` writes.
    pub fn prepare_bytes_with(
        &mut self,
        mut source: FrameInput,
        force_keyframe: bool,
        segments: &mut [Option<TableSegment>; TABLE_COUNT],
    ) -> io::Result<PreparedFrame> {
        // Owned rows are stored at their display steps (crate::display), so
        // the committed baseline is what decoders rebuild and a row that later
        // drifts only below a step equals it. Encoding rounds every stepped
        // value again, so a segment encoded from unrounded rows writes the
        // same values.
        source.round_to_display();
        let new_branch = self.state.branch != Some(source.branch);
        if !new_branch && self.state.last_hour.is_some_and(|hour| hour >= source.hour) {
            return Err(invalid("frame hour did not advance on its branch"));
        }
        let revision = self
            .revision
            .checked_add(1)
            .ok_or_else(|| invalid("encoder revision space exhausted"))?;
        let context = self.frame_context(source.branch, source.hour, force_keyframe);
        let keyframe = context.keyframe;
        let mut output = allocation::vec(EncodedFrame::PREFIX_BYTES + 37 + 8 * TABLE_COUNT)?;
        output.resize(EncodedFrame::PREFIX_BYTES, 0);
        output.extend_from_slice(&source.branch.to_le_bytes());
        output.extend_from_slice(&source.hour.to_le_bytes());
        output.push(u8::from(keyframe));
        output.extend_from_slice(&source.metrics.total_ns.to_le_bytes());
        for table in TableId::ALL {
            if !table
                .last_change()
                .present(FormatVersion::CURRENT.version())
            {
                continue;
            }
            output.extend_from_slice(&source.metrics.table_ns[table.index()].to_le_bytes());
        }
        output.extend_from_slice(&source.metrics.pool_bytes.to_le_bytes());
        output.extend_from_slice(&source.metrics.dropped_frames.to_le_bytes());
        let mut unchanged = [false; TABLE_COUNT];
        let mut deleted = std::mem::take(&mut self.deleted);
        for table in TableId::ALL {
            if !table
                .last_change()
                .present(FormatVersion::CURRENT.version())
            {
                continue;
            }
            unchanged[table.index()] = match segments[table.index()].take() {
                Some(segment) => {
                    if segment.context != context || segment.table != table {
                        return Err(invalid("table segment was encoded for another frame"));
                    }
                    allocation::reserve(&mut output, segment.bytes.len())?;
                    output.extend_from_slice(&segment.bytes);
                    segment.unchanged
                }
                None => {
                    allocation::reserve(&mut output, 9)?;
                    self.encode_table(
                        &context,
                        table,
                        &source.tables[table.index()],
                        &mut output,
                        &mut deleted,
                    )?
                }
            };
        }
        self.deleted = deleted;
        Ok(PreparedFrame {
            frame: Some(EncodedFrame {
                branch: source.branch,
                hour: source.hour,
                keyframe,
                bytes: output,
            }),
            source,
            unchanged,
            revision,
            encoder: self.identity,
            previous_revision: self.revision,
            keyframe,
        })
    }

    /// One table's update; returns whether the table's baseline is unchanged.
    fn encode_table(
        &self,
        context: &FrameContext,
        table: TableId,
        view: &TableInput,
        output: &mut Vec<u8>,
        deleted: &mut Vec<u8>,
    ) -> io::Result<bool> {
        let FrameContext {
            branch,
            hour,
            force,
            keyframe,
            new_branch,
            ..
        } = *context;
        Ok(match view {
            TableInput::SharedComplete(values) => {
                if !keyframe
                    && !new_branch
                    && let Some((baseline, BaselineRows::Shared(previous))) =
                        self.state.tables[table.index()].as_ref()
                    && Arc::ptr_eq(values, previous)
                {
                    validate_singleton(table, values, hour)?;
                    output.push(2);
                    output.extend_from_slice(&baseline.to_le_bytes());
                    true
                } else {
                    self.encode_complete(table, values, hour, keyframe, output, deleted)?
                }
            }
            TableInput::ComparedEqual(token) => {
                self.validate_comparison(*token, table, branch, hour, keyframe)?;
                output.push(2);
                output.extend_from_slice(&token.baseline.to_le_bytes());
                true
            }
            TableInput::ComparedChanges {
                token,
                indices,
                rows,
            } => {
                self.validate_comparison(*token, table, branch, hour, keyframe)?;
                let Some((_, BaselineRows::Owned(previous))) =
                    self.state.tables[table.index()].as_ref()
                else {
                    return Err(invalid("row replacements require an owned baseline"));
                };
                if !token.allows_row_updates() {
                    return Err(invalid("table does not allow row replacements"));
                }
                validate_rows(table, rows)?;
                let start = output.len();
                output.push(1);
                output.extend_from_slice(&token.baseline.to_le_bytes());
                if rows.encode_replacements(previous, indices, output)? {
                    false
                } else {
                    output.truncate(start);
                    output.push(2);
                    output.extend_from_slice(&token.baseline.to_le_bytes());
                    true
                }
            }
            TableInput::SliceUpdate { token, update } => {
                self.validate_comparison(*token, table, branch, hour, keyframe)?;
                let Some((_, BaselineRows::Shared(previous))) =
                    self.state.tables[table.index()].as_ref()
                else {
                    return Err(invalid("a slice update requires a shared baseline"));
                };
                if !update.fits(previous) {
                    return Err(invalid("slice update does not fit its baseline"));
                }
                validate_rows(table, update.rows())?;
                let start = output.len();
                output.push(1);
                output.extend_from_slice(&token.baseline.to_le_bytes());
                if update.encode_changes(previous, output, deleted)? {
                    false
                } else {
                    output.truncate(start);
                    output.push(2);
                    output.extend_from_slice(&token.baseline.to_le_bytes());
                    true
                }
            }
            TableInput::Sampled(TableView::Complete(values)) => {
                self.encode_complete(table, values, hour, keyframe, output, deleted)?
            }
            TableInput::Sampled(TableView::Truncated { rows, missing }) => {
                validate_rows(table, rows)?;
                if *missing == 0 {
                    return Err(invalid("truncated table has no missing rows"));
                }
                if keyframe {
                    output.push(4);
                    let mut reason = [0_u8; 64];
                    let prefix = super::TRUNCATED_KEYFRAME_REASON.as_bytes();
                    reason[..prefix.len()].copy_from_slice(prefix);
                    let mut number = [0_u8; 10];
                    let mut value = *missing;
                    let mut start = number.len();
                    loop {
                        start -= 1;
                        number[start] = b'0' + (value % 10) as u8;
                        value /= 10;
                        if value == 0 {
                            break;
                        }
                    }
                    let end = prefix.len() + number.len() - start;
                    reason[prefix.len()..end].copy_from_slice(&number[start..]);
                    let suffix = b" rows not read";
                    reason[end..end + suffix.len()].copy_from_slice(suffix);
                    put_bytes(output, &reason[..end + suffix.len()])?;
                } else {
                    output.push(3);
                    output.extend_from_slice(&missing.to_le_bytes());
                    put_count(output, rows.len())?;
                    rows.encode_rows(output)?;
                }
                false
            }
            TableInput::Sampled(TableView::Unavailable { reason }) => {
                if reason.is_empty() {
                    return Err(invalid("unavailable table needs a reason"));
                }
                if reason == crate::cadence::SKIPPED_REASON
                    && self.can_carry(table, branch, hour, force)
                {
                    output.push(2);
                    output.extend_from_slice(
                        &self.state.tables[table.index()]
                            .as_ref()
                            .unwrap()
                            .0
                            .to_le_bytes(),
                    );
                    true
                } else {
                    output.push(4);
                    put_string(output, reason)?;
                    false
                }
            }
        })
    }

    /// Publish validated owning rows only after their record was fully written.
    pub fn commit(&mut self, prepared: PreparedFrame) -> io::Result<()> {
        self.commit_retiring(prepared).map(drop)
    }

    /// [`Self::commit`], handing back what the baselines no longer hold
    /// instead of freeing it here. Moving and replacing rows and handing over
    /// shared references allocate nothing beyond the hand-back list, which is
    /// reserved first. A slice update whose owners' row counts change rebuilds
    /// its baseline in storage reserved before that baseline moves; slice
    /// updates apply before every other table, so an allocation failure
    /// leaves the other baselines as they were.
    pub fn commit_retiring(&mut self, prepared: PreparedFrame) -> io::Result<Retired> {
        if prepared.encoder != self.identity || prepared.previous_revision != self.revision {
            return Err(invalid(
                "prepared frame no longer names the encoder baseline",
            ));
        }
        let PreparedFrame {
            source,
            unchanged,
            revision,
            keyframe,
            ..
        } = prepared;
        let mut retired = allocation::vec(2 * TABLE_COUNT)?;
        let mut tables = source.tables.map(Some);
        for (table, view) in TableId::ALL.into_iter().zip(tables.iter_mut()) {
            if !matches!(view, Some(TableInput::SliceUpdate { .. })) {
                continue;
            }
            let Some(TableInput::SliceUpdate { update, .. }) = view.take() else {
                unreachable!("matched above");
            };
            let Some((hour, BaselineRows::Shared(rows))) =
                self.state.tables[table.index()].as_mut()
            else {
                unreachable!("validated shared slice baseline");
            };
            if Arc::get_mut(rows).is_none() {
                // The producer still shares the baseline: replace a copy.
                *rows = Arc::new(rows.try_clone()?);
            }
            let update = Arc::try_unwrap(update).unwrap_or_else(|shared| (*shared).clone());
            let replaced = update.apply(Arc::get_mut(rows).expect("unique slice baseline"))?;
            *hour = source.hour;
            retired.push(Retiring::Rows(replaced));
        }
        for (table, view) in TableId::ALL.into_iter().zip(tables) {
            let index = table.index();
            let Some(view) = view else {
                continue;
            };
            if !table
                .last_change()
                .present(FormatVersion::CURRENT.version())
            {
                if let Some((_, rows)) = self.state.tables[index].take() {
                    retired.push(Retiring::Baseline(rows));
                }
                retired.push(Retiring::Input(view));
                continue;
            }
            if let TableInput::ComparedChanges { indices, rows, .. } = view {
                let (hour, BaselineRows::Owned(previous)) = self.state.tables[index]
                    .as_mut()
                    .expect("validated replacement baseline")
                else {
                    unreachable!("validated owned replacement baseline")
                };
                retired.push(Retiring::Rows(
                    previous.install_replacements(rows, &indices),
                ));
                *hour = source.hour;
            } else if unchanged[index] {
                let baseline = self.state.tables[index]
                    .as_mut()
                    .expect("unchanged table has a validated baseline");
                baseline.0 = source.hour;
                match view {
                    TableInput::SharedComplete(rows) => retired.push(Retiring::Baseline(
                        std::mem::replace(&mut baseline.1, BaselineRows::Shared(rows)),
                    )),
                    view => retired.push(Retiring::Input(view)),
                }
            } else {
                let next = match view {
                    TableInput::Sampled(TableView::Complete(rows)) => {
                        Some((source.hour, BaselineRows::Owned(rows)))
                    }
                    TableInput::SharedComplete(rows) => {
                        Some((source.hour, BaselineRows::Shared(rows)))
                    }
                    view => {
                        retired.push(Retiring::Input(view));
                        None
                    }
                };
                if let Some((_, rows)) = std::mem::replace(&mut self.state.tables[index], next) {
                    retired.push(Retiring::Baseline(rows));
                }
            }
        }
        self.state.branch = Some(source.branch);
        self.state.last_hour = Some(source.hour);
        self.revision = revision;
        if keyframe {
            self.keyframe_hour = Some(source.hour);
        }
        Ok(Retired(retired))
    }
}
