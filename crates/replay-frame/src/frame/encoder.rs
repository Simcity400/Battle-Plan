use super::{
    BaselineToken, DecodedFrame, EncodedFrame, Frame, FrameInput, TABLE_COUNT, TableInput,
    TableView, put_count, validate_complete, validate_rows, validate_singleton, wire::table_stored,
};
use crate::{
    BranchId, Hour, KEYFRAME_INTERVAL,
    codec::{invalid_static as invalid, put_bytes, put_string},
    format::FormatVersion,
    rows::{ChangedRows, Field, RowCodec, RowsPairVisit, RowsVisit, TableId, TableRows, TableSchema},
};
use engine_allocation as allocation;
use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

#[derive(Debug)]
struct RowValidationError(io::Error);
impl std::fmt::Display for RowValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}
impl std::error::Error for RowValidationError {}
fn row_validation(error: io::Error) -> io::Error {
    if error.kind() == io::ErrorKind::InvalidData {
        io::Error::new(io::ErrorKind::InvalidData, RowValidationError(error))
    } else {
        error
    }
}

fn row_encoding(error: io::Error) -> io::Error {
    if error.kind() != io::ErrorKind::InvalidData {
        return error;
    }
    match crate::codec::try_format(format_args!("{error}")) {
        Ok(detail) if detail.starts_with("row ") && detail.contains(" column ") => {
            row_validation(error)
        }
        Ok(_) => error,
        Err(error) => error,
    }
}

fn validate_recorded(rows: &TableRows) -> io::Result<()> {
    struct Check;
    impl RowsVisit for Check {
        type Output = io::Result<()>;

        fn rows<R: RowCodec>(self, rows: &[R]) -> io::Result<()> {
            u32::try_from(rows.len()).map_err(|_| invalid("too many rows"))?;
            for row in rows {
                if row.not_recorded_fields() != 0 {
                    row.visit_wire(&mut |_, field| {
                        if matches!(field, Field::NotRecorded) {
                            return Err(invalid("missing or not recorded row field"));
                        }
                        Ok(())
                    })?;
                }
            }
            Ok(())
        }
    }
    rows.visit(Check)
}

fn validate_replacements(
    rows: &TableRows,
    previous: &TableRows,
    indices: &[usize],
) -> io::Result<()> {
    struct Check<'a>(&'a [usize]);
    impl RowsPairVisit for Check<'_> {
        type Output = io::Result<()>;

        fn rows<R: RowCodec>(self, rows: &[R], previous: &[R]) -> io::Result<()> {
            let indices = self.0;
            if rows.len() != indices.len()
                || indices.windows(2).any(|pair| pair[0] >= pair[1])
                || rows
                    .iter()
                    .zip(indices)
                    .any(|(row, &index)| previous.get(index).is_none_or(|old| row.key() != old.key()))
            {
                return Err(invalid(
                    "replacement rows must name unique ascending baseline positions with unchanged keys",
                ));
            }
            u32::try_from(rows.len()).map_err(|_| invalid("too many replacement rows"))?;
            Ok(())
        }
    }
    rows.visit_pair(previous, Check(indices))
        .map_err(|_| invalid("replacement rows have different types"))?
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BaselineRetention {
    Drop,
    Meta,
    Full,
}

pub(super) enum BaselineRows {
    Owned(TableRows),
    Shared(Arc<TableRows>),
    Meta { len: Option<usize>, shared: bool },
}

impl BaselineRows {
    fn rows(&self) -> Option<&TableRows> {
        match self {
            Self::Owned(rows) => Some(rows),
            Self::Shared(rows) => Some(rows),
            Self::Meta { .. } => None,
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
pub struct PreparedFrame {
    frame: Option<EncodedFrame>,
    source: FrameInput,
    changed: Vec<(usize, ChangedRows)>,
    unchanged: [bool; TABLE_COUNT],
    sizes: [usize; TABLE_COUNT],
    revision: u64,
    encoder: u64,
    previous_revision: u64,
    keyframe: bool,
}

impl PreparedFrame {
    pub fn frame(&self) -> &EncodedFrame {
        self.frame.as_ref().expect("prepared record not taken")
    }
    pub fn take_frame(&mut self) -> EncodedFrame {
        self.frame.take().expect("prepared record already taken")
    }

    pub fn take_frame_keeping(
        &mut self,
        kept: impl Fn(TableId) -> bool,
    ) -> io::Result<EncodedFrame> {
        let frame = self.take_frame();
        let header = frame.bytes.len() - self.sizes.iter().sum::<usize>();
        let mut keep = [false; TABLE_COUNT];
        let mut length = header;
        for ((table, &size), keep) in TableId::ALL.into_iter().zip(&self.sizes).zip(&mut keep) {
            *keep = size != 0 && kept(table);
            length += if *keep { size } else { usize::from(size != 0) };
        }
        if length == frame.bytes.len() {
            return Ok(frame);
        }
        let mut bytes = allocation::vec(length)?;
        bytes.extend_from_slice(&frame.bytes[..header]);
        let mut at = header;
        for (&size, keep) in self.sizes.iter().zip(keep) {
            if size == 0 {
                continue;
            }
            if keep {
                bytes.extend_from_slice(&frame.bytes[at..at + size]);
            } else {
                bytes.push(5);
            }
            at += size;
        }
        Ok(EncodedFrame { bytes, ..frame })
    }

    pub fn input(&self) -> &FrameInput {
        &self.source
    }

    pub fn changed_rows(&self) -> &[(usize, ChangedRows)] {
        &self.changed
    }
}

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

pub struct TableSegment {
    context: FrameContext,
    table: TableId,
    bytes: Vec<u8>,
    unchanged: bool,
    changed: Option<ChangedRows>,
}

#[derive(Default)]
pub struct Retired(Vec<Retiring>);

#[expect(dead_code, reason = "the variants hold replaced rows only so they are dropped later, off the commit path")]
enum Retiring {
    Baseline(BaselineRows),
    Input(TableInput),
    Rows(TableRows),
}

impl Retired {
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
    pub fn identity(&self) -> u64 {
        self.identity
    }
    fn needs_keyframe(&self, branch: BranchId, hour: Hour, force: bool) -> bool {
        force
            || self.state.branch != Some(branch)
            || self.keyframe_hour.is_none_or(|base| {
                i64::from(hour) - i64::from(base) >= i64::from(KEYFRAME_INTERVAL)
            })
    }

    pub fn can_carry(&self, table: TableId, branch: BranchId, hour: Hour, force: bool) -> bool {
        crate::cadence::is_staggered(table)
            && !self.needs_keyframe(branch, hour, force)
            && self.state.last_hour.is_some_and(|previous| hour > previous)
            && self.state.tables[table.index()]
                .as_ref()
                .is_some_and(|(_, rows)| rows.rows().is_some())
    }

    pub fn keyframe_carry(
        &self,
        table: TableId,
        branch: BranchId,
        hour: Hour,
        force: bool,
    ) -> Option<&TableRows> {
        if !crate::cadence::is_staggered(table)
            || !self.needs_keyframe(branch, hour, force)
            || self.state.branch != Some(branch)
            || !self.state.last_hour.is_some_and(|previous| hour > previous)
        {
            return None;
        }
        self.committed_rows(table)
    }

    fn token(
        &self,
        table: TableId,
        branch: BranchId,
        hour: Hour,
        force: bool,
    ) -> Option<(BaselineToken, &BaselineRows)> {
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
                untracked: false,
            },
            values,
        ))
    }

    pub fn untracked_slice_token(
        &self,
        table: TableId,
        branch: BranchId,
        hour: Hour,
        force: bool,
    ) -> Option<BaselineToken> {
        if table == TableId::Clock
            || force
            || self.state.branch != Some(branch)
            || self
                .state
                .last_hour
                .and_then(|previous| previous.checked_add(1))
                != Some(hour)
        {
            return None;
        }
        Some(BaselineToken {
            encoder: self.identity,
            revision: self.revision,
            table,
            branch,
            hour,
            baseline: hour - 1,
            row_updates: false,
            untracked: true,
        })
    }

    pub fn committed_rows(&self, table: TableId) -> Option<&TableRows> {
        let (_, rows) = self.state.tables[table.index()].as_ref()?;
        rows.rows()
    }

    pub fn committed_hour(&self, table: TableId) -> Option<Hour> {
        self.state.tables[table.index()].as_ref().map(|(hour, _)| *hour)
    }

    pub fn last_committed(&self) -> Option<(BranchId, Hour)> {
        Some((self.state.branch?, self.state.last_hour?))
    }

    pub fn committed<T: TableSchema>(&self) -> Option<(Hour, &[T::Row])> {
        let (hour, values) = self.state.tables[T::ID.index()].as_ref()?;
        Some((*hour, values.rows()?.rows::<T>()?))
    }

    pub fn observed_baseline<T: TableSchema>(
        &self,
        branch: BranchId,
        hour: Hour,
    ) -> Option<&[T::Row]> {
        if self.state.branch != Some(branch)
            || self
                .state
                .last_hour
                .and_then(|previous| previous.checked_add(1))
                != Some(hour)
        {
            return None;
        }
        self.committed::<T>().map(|(_, rows)| rows)
    }

    pub fn observed_comparison_baseline<T: TableSchema>(
        &self,
        branch: BranchId,
        hour: Hour,
        force: bool,
    ) -> Option<(BaselineToken, &[T::Row])> {
        let (mut token, values) = self.token(T::ID, branch, hour, force)?;
        token.row_updates = matches!(values, BaselineRows::Owned(_)) && T::ID != TableId::Countries;
        Some((token, values.rows()?.rows::<T>()?))
    }

    pub fn comparison_baseline<T: TableSchema>(
        &self,
        branch: BranchId,
        hour: Hour,
        force: bool,
    ) -> Option<(BaselineToken, &[T::Row])> {
        let (token, values) = self.token(T::ID, branch, hour, force)?;
        Some((token, values.rows()?.rows::<T>()?))
    }

    pub fn republish_token(
        &self,
        table: TableId,
        branch: BranchId,
        hour: Hour,
        force: bool,
    ) -> Option<(BaselineToken, Hour)> {
        let (token, _) = self.token(table, branch, hour, force)?;
        Some((token, token.baseline))
    }

    pub fn slice_baseline(
        &self,
        table: TableId,
        branch: BranchId,
        hour: Hour,
        force: bool,
    ) -> Option<(BaselineToken, &Arc<TableRows>)> {
        match self.token(table, branch, hour, force)? {
            (token, BaselineRows::Shared(rows)) => Some((token, rows)),
            (_, BaselineRows::Owned(_) | BaselineRows::Meta { .. }) => None,
        }
    }

    pub fn slice_meta(
        &self,
        table: TableId,
        branch: BranchId,
        hour: Hour,
        force: bool,
    ) -> Option<(BaselineToken, Option<usize>)> {
        match self.token(table, branch, hour, force)? {
            (token, BaselineRows::Shared(rows)) => Some((token, Some(rows.len()))),
            (token, BaselineRows::Meta { len, shared: true }) => Some((token, *len)),
            (_, BaselineRows::Owned(_) | BaselineRows::Meta { shared: false, .. }) => None,
        }
    }

    pub fn shared_baseline(&self, table: TableId) -> Option<&Arc<TableRows>> {
        match self.state.tables[table.index()].as_ref()? {
            (_, BaselineRows::Shared(rows)) => Some(rows),
            (_, BaselineRows::Owned(_) | BaselineRows::Meta { .. }) => None,
        }
    }

    pub fn certified_keyframe_baseline(
        &self,
        table: TableId,
        branch: BranchId,
        hour: Hour,
        force: bool,
    ) -> Option<&Arc<TableRows>> {
        if !self.needs_keyframe(branch, hour, force)
            || self.state.branch != Some(branch)
            || self.state.last_hour?.checked_add(1) != Some(hour)
        {
            return None;
        }
        self.shared_baseline(table)
    }

    fn encode_complete(
        &self,
        table: TableId,
        values: &TableRows,
        hour: Hour,
        keyframe: bool,
        output: &mut Vec<u8>,
        deleted: &mut Vec<u8>,
        changed: &mut Option<ChangedRows>,
    ) -> io::Result<bool> {
        validate_complete(table, values, hour).map_err(row_validation)?;
        let Some((baseline, previous)) = self.state.tables[table.index()]
            .as_ref()
            .filter(|_| !keyframe)
            .and_then(|(baseline, rows)| Some((baseline, rows.rows()?)))
        else {
            output.push(0);
            put_count(output, values.len())?;
            values.encode_rows(output).map_err(row_encoding)?;
            return Ok(false);
        };
        let start = output.len();
        output.push(1);
        output.extend_from_slice(&baseline.to_le_bytes());
        let mut rows = ChangedRows::default();
        let written = values
            .encode_changes_recorded(previous, output, deleted, &mut rows)
            .map_err(row_encoding)?;
        if !rows.lost {
            *changed = Some(rows);
        }
        if written {
            return Ok(false);
        }
        output.truncate(start);
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
            || token.untracked
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

    pub fn encode(&mut self, source: DecodedFrame, force_keyframe: bool) -> io::Result<Frame> {
        self.encode_input(source.into(), force_keyframe)
    }

    pub fn encode_input(&mut self, source: FrameInput, force_keyframe: bool) -> io::Result<Frame> {
        self.encode_bytes(source, force_keyframe)?.decode()
    }

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

    pub fn encode_segment(
        &self,
        context: &FrameContext,
        table: TableId,
        input: &TableInput,
    ) -> io::Result<TableSegment> {
        if context.encoder != self.identity || context.revision != self.revision {
            return Err(invalid("segment context does not name this encoder state"));
        }
        let (mut bytes, mut deleted, mut changed) = (Vec::new(), Vec::new(), None);
        let unchanged = crate::codec::with_error_details(|| {
            self.encode_table(context, table, input, &mut bytes, &mut deleted, &mut changed)
                .map_err(|error| crate::codec::error_context(error, format_args!("{table:?}")))
        })?;
        Ok(TableSegment {
            context: *context,
            table,
            bytes,
            unchanged,
            changed,
        })
    }

    pub fn prepare_bytes(
        &mut self,
        source: FrameInput,
        force_keyframe: bool,
    ) -> io::Result<PreparedFrame> {
        self.prepare_bytes_with(source, force_keyframe, &mut std::array::from_fn(|_| None))
    }

    pub fn prepare_bytes_with(
        &mut self,
        source: FrameInput,
        force_keyframe: bool,
        segments: &mut [Option<TableSegment>; TABLE_COUNT],
    ) -> io::Result<PreparedFrame> {
        self.prepare_contained(source, force_keyframe, segments, |_, error| Err(error))
    }

    pub fn prepare_contained(
        &mut self,
        source: FrameInput,
        force_keyframe: bool,
        segments: &mut [Option<TableSegment>; TABLE_COUNT],
        refuse: impl FnMut(TableId, io::Error) -> io::Result<String>,
    ) -> io::Result<PreparedFrame> {
        self.prepare_contained_keeping(source, force_keyframe, segments, |_| true, refuse)
    }

    pub fn prepare_contained_keeping(
        &mut self,
        mut source: FrameInput,
        force_keyframe: bool,
        segments: &mut [Option<TableSegment>; TABLE_COUNT],
        kept: impl Fn(TableId) -> bool,
        refuse: impl FnMut(TableId, io::Error) -> io::Result<String>,
    ) -> io::Result<PreparedFrame> {
        source.round_to_display();
        self.prepare_rounded_contained_keeping(source, force_keyframe, segments, kept, refuse)
    }

    pub fn prepare_rounded_contained_keeping(
        &mut self,
        mut source: FrameInput,
        force_keyframe: bool,
        segments: &mut [Option<TableSegment>; TABLE_COUNT],
        kept: impl Fn(TableId) -> bool,
        mut refuse: impl FnMut(TableId, io::Error) -> io::Result<String>,
    ) -> io::Result<PreparedFrame> {
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
        let mode = source.capture_mode()?;
        let mut output = allocation::vec(EncodedFrame::PREFIX_BYTES + 38 + 8 * TABLE_COUNT)?;
        output.resize(EncodedFrame::PREFIX_BYTES, 0);
        output.extend_from_slice(&source.branch.to_le_bytes());
        output.extend_from_slice(&source.hour.to_le_bytes());
        output.push(u8::from(keyframe));
        output.push(mode.byte());
        output.extend_from_slice(&source.metrics.total_ns.to_le_bytes());
        for table in TableId::ALL {
            if !table_stored(table, FormatVersion::CURRENT, mode) {
                continue;
            }
            output.extend_from_slice(&source.metrics.table_ns[table.index()].to_le_bytes());
        }
        output.extend_from_slice(&source.metrics.pool_bytes.to_le_bytes());
        output.extend_from_slice(&source.metrics.dropped_frames.to_le_bytes());
        let mut unchanged = [false; TABLE_COUNT];
        let mut sizes = [0; TABLE_COUNT];
        let mut changed_rows = Vec::new();
        let mut deleted = std::mem::take(&mut self.deleted);
        for table in TableId::ALL {
            if !table_stored(table, FormatVersion::CURRENT, mode) {
                continue;
            }
            let mut changed = None;
            let table_start = output.len();
            let keep = kept(table);
            unchanged[table.index()] = match segments[table.index()].take() {
                Some(segment) => {
                    if segment.context != context || segment.table != table {
                        return Err(invalid("table segment was encoded for another frame"));
                    }
                    if keep {
                        allocation::reserve(&mut output, segment.bytes.len())?;
                        output.extend_from_slice(&segment.bytes);
                    } else {
                        allocation::reserve(&mut output, 1)?;
                        output.push(5);
                    }
                    changed = segment.changed;
                    segment.unchanged
                }
                None if !keep => {
                    let result = self.validate_unencoded_table(
                        &context,
                        table,
                        &source.tables[table.index()],
                    );
                    let is_unchanged = match result {
                        Err(error)
                            if table != TableId::Clock
                                && error
                                    .get_ref()
                                    .is_some_and(|inner| inner.is::<RowValidationError>()) =>
                        {
                            let error =
                                crate::codec::error_context(error, format_args!("{table:?}"));
                            let reason = refuse(table, error)?;
                            source.tables[table.index()] = TableView::Unavailable { reason }.into();
                            self.validate_unencoded_table(
                                &context,
                                table,
                                &source.tables[table.index()],
                            )?
                        }
                        result => result.map_err(|error| {
                            crate::codec::error_context(error, format_args!("{table:?}"))
                        })?,
                    };
                    allocation::reserve(&mut output, 1)?;
                    output.push(5);
                    is_unchanged
                }
                None => {
                    allocation::reserve(&mut output, 9)?;
                    let start = output.len();
                    let result = self.encode_table(
                        &context,
                        table,
                        &source.tables[table.index()],
                        &mut output,
                        &mut deleted,
                        &mut changed,
                    );
                    match result {
                        Err(error)
                            if table != TableId::Clock
                                && error
                                    .get_ref()
                                    .is_some_and(|inner| inner.is::<RowValidationError>()) =>
                        {
                            let error =
                                crate::codec::error_context(error, format_args!("{table:?}"));
                            let reason = refuse(table, error)?;
                            output.truncate(start);
                            deleted.clear();
                            changed = None;
                            source.tables[table.index()] = TableView::Unavailable { reason }.into();
                            self.encode_table(
                                &context,
                                table,
                                &source.tables[table.index()],
                                &mut output,
                                &mut deleted,
                                &mut changed,
                            )?
                        }
                        result => result.map_err(|error| {
                            crate::codec::error_context(error, format_args!("{table:?}"))
                        })?,
                    }
                }
            };
            sizes[table.index()] = output.len() - table_start;
            if let Some(changed) = changed
                && allocation::advisory(|| allocation::reserve(&mut changed_rows, 1)).is_ok()
            {
                changed_rows.push((table.index(), changed));
            }
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
            changed: changed_rows,
            unchanged,
            sizes,
            revision,
            encoder: self.identity,
            previous_revision: self.revision,
            keyframe,
        })
    }

    fn validate_unencoded_table(
        &self,
        context: &FrameContext,
        table: TableId,
        view: &TableInput,
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
                    && let Some((_, BaselineRows::Shared(previous))) =
                        self.state.tables[table.index()].as_ref()
                    && Arc::ptr_eq(values, previous)
                {
                    validate_singleton(table, values, hour).map_err(row_validation)?;
                    true
                } else {
                    validate_complete(table, values, hour).map_err(row_validation)?;
                    validate_recorded(values).map_err(row_encoding)?;
                    false
                }
            }
            TableInput::ComparedEqual(token) => {
                self.validate_comparison(*token, table, branch, hour, keyframe)?;
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
                validate_rows(table, rows).map_err(row_validation)?;
                validate_replacements(rows, previous, indices)?;
                validate_recorded(rows).map_err(row_encoding)?;
                indices.is_empty()
            }
            TableInput::SliceUpdate { token, update } if token.untracked => {
                if token.encoder != self.identity
                    || token.revision != self.revision
                    || table == TableId::Clock
                    || token.table != table
                    || token.branch != branch
                    || token.hour != hour
                    || force
                    || self.state.branch != Some(branch)
                    || self
                        .state
                        .last_hour
                        .and_then(|previous| previous.checked_add(1))
                        != Some(hour)
                {
                    return Err(invalid(
                        "untracked slice update does not name the current encoder state",
                    ));
                }
                if !update.ranges.is_empty() || !update.fits_meta(table) {
                    return Err(invalid("untracked slice update is invalid"));
                }
                validate_rows(table, update.rows()).map_err(row_validation)?;
                validate_recorded(update.rows()).map_err(row_encoding)?;
                update.owners.is_empty() && update.keys.is_empty() && update.rows.is_empty()
            }
            TableInput::SliceUpdate { token, update } => {
                self.validate_comparison(*token, table, branch, hour, keyframe)?;
                match self.state.tables[table.index()].as_ref() {
                    Some((_, BaselineRows::Shared(previous))) => {
                        if !update.fits(previous) {
                            return Err(invalid("slice update does not fit its baseline"));
                        }
                    }
                    Some((_, BaselineRows::Meta { shared: true, .. })) => {
                        if !update.fits_meta(table) {
                            return Err(invalid("slice update does not fit its baseline"));
                        }
                    }
                    _ => return Err(invalid("a slice update requires a shared baseline")),
                }
                validate_rows(table, update.rows()).map_err(row_validation)?;
                validate_recorded(update.rows()).map_err(row_encoding)?;
                update.owners.is_empty() && update.keys.is_empty() && update.ranges.is_empty()
            }
            TableInput::UniformOptionalI32 {
                token,
                column,
                update,
                ..
            } => {
                self.validate_comparison(*token, table, branch, hour, keyframe)?;
                let Some((_, BaselineRows::Shared(previous))) =
                    self.state.tables[table.index()].as_ref()
                else {
                    return Err(invalid("a uniform update requires a shared baseline"));
                };
                if !previous.supports_optional_i32_fill(*column) || !update.fits(previous) {
                    return Err(invalid("uniform update does not fit its baseline"));
                }
                validate_rows(table, update.rows()).map_err(row_validation)?;
                validate_recorded(update.rows()).map_err(row_encoding)?;
                false
            }
            TableInput::Sampled(TableView::Complete(values)) => {
                validate_complete(table, values, hour).map_err(row_validation)?;
                validate_recorded(values).map_err(row_encoding)?;
                false
            }
            TableInput::Sampled(TableView::Truncated { rows, missing }) => {
                validate_rows(table, rows).map_err(row_validation)?;
                if *missing == 0 {
                    return Err(invalid("truncated table has no missing rows"));
                }
                if !keyframe {
                    validate_recorded(rows).map_err(row_encoding)?;
                }
                false
            }
            TableInput::Sampled(TableView::NotRecorded { .. }) => false,
            TableInput::Sampled(TableView::Unavailable { reason }) => {
                if reason.is_empty() {
                    return Err(invalid("unavailable table needs a reason"));
                }
                reason == crate::cadence::SKIPPED_REASON
                    && self.can_carry(table, branch, hour, force)
            }
        })
    }

    fn encode_table(
        &self,
        context: &FrameContext,
        table: TableId,
        view: &TableInput,
        output: &mut Vec<u8>,
        deleted: &mut Vec<u8>,
        changed: &mut Option<ChangedRows>,
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
                    validate_singleton(table, values, hour).map_err(row_validation)?;
                    output.push(2);
                    output.extend_from_slice(&baseline.to_le_bytes());
                    true
                } else {
                    self.encode_complete(table, values, hour, keyframe, output, deleted, changed)?
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
                validate_rows(table, rows).map_err(row_validation)?;
                let start = output.len();
                output.push(1);
                output.extend_from_slice(&token.baseline.to_le_bytes());
                if rows
                    .encode_replacements(previous, indices, output)
                    .map_err(row_encoding)?
                {
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
                validate_rows(table, update.rows()).map_err(row_validation)?;
                let start = output.len();
                output.push(1);
                output.extend_from_slice(&token.baseline.to_le_bytes());
                let mut rows = ChangedRows::default();
                let written = update
                    .encode_changes_recorded(previous, output, deleted, &mut rows)
                    .map_err(row_encoding)?;
                if !rows.lost {
                    *changed = Some(rows);
                }
                if written {
                    false
                } else {
                    output.truncate(start);
                    output.push(2);
                    output.extend_from_slice(&token.baseline.to_le_bytes());
                    true
                }
            }
            TableInput::UniformOptionalI32 {
                token,
                column,
                value,
                update,
            } => {
                self.validate_comparison(*token, table, branch, hour, keyframe)?;
                let Some((_, BaselineRows::Shared(previous))) =
                    self.state.tables[table.index()].as_ref()
                else {
                    return Err(invalid("a uniform update requires a shared baseline"));
                };
                if !previous.supports_optional_i32_fill(*column) || !update.fits(previous) {
                    return Err(invalid("uniform update does not fit its baseline"));
                }
                validate_rows(table, update.rows()).map_err(row_validation)?;
                let mut merged = update.merged(previous)?;
                merged.fill_optional_i32(*column, *value)?;
                let mut ignored = None;
                self.encode_complete(table, &merged, hour, keyframe, output, deleted, &mut ignored)?
            }
            TableInput::Sampled(TableView::Complete(values)) => {
                self.encode_complete(table, values, hour, keyframe, output, deleted, changed)?
            }
            TableInput::Sampled(TableView::Truncated { rows, missing }) => {
                validate_rows(table, rows).map_err(row_validation)?;
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
                    rows.encode_rows(output).map_err(row_encoding)?;
                }
                false
            }
            TableInput::Sampled(TableView::NotRecorded { .. }) => {
                output.push(5);
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

    pub fn commit(&mut self, prepared: PreparedFrame) -> io::Result<()> {
        self.commit_retiring(prepared).map(drop)
    }

    pub fn commit_retiring(&mut self, prepared: PreparedFrame) -> io::Result<Retired> {
        self.commit_retiring_keeping(prepared, |_| true)
    }

    pub fn commit_retiring_keeping(
        &mut self,
        prepared: PreparedFrame,
        keep_baseline: impl Fn(TableId) -> bool,
    ) -> io::Result<Retired> {
        self.commit_retiring_mode(prepared, |table| {
            if keep_baseline(table) {
                BaselineRetention::Full
            } else {
                BaselineRetention::Drop
            }
        })
    }

    pub fn commit_retiring_mode(
        &mut self,
        prepared: PreparedFrame,
        keep_baseline: impl Fn(TableId) -> BaselineRetention,
    ) -> io::Result<Retired> {
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
        let mut need = 0_usize;
        for (table, view) in TableId::ALL.into_iter().zip(&source.tables) {
            let index = table.index();
            if !matches!(view, TableInput::ComparedEqual(_)) {
                need += 1;
            }
            if matches!(
                self.state.tables[index],
                Some((_, BaselineRows::Owned(_) | BaselineRows::Shared(_)))
            ) {
                need += 1;
            }
        }
        let mut retired = if need == 0 {
            Vec::new()
        } else {
            allocation::vec(need)?
        };
        let push_input = |retired: &mut Vec<Retiring>, input: TableInput| {
            if !matches!(input, TableInput::ComparedEqual(_)) {
                retired.push(Retiring::Input(input));
            }
        };
        let push_baseline = |retired: &mut Vec<Retiring>, baseline: BaselineRows| {
            if !matches!(baseline, BaselineRows::Meta { .. }) {
                retired.push(Retiring::Baseline(baseline));
            }
        };
        let mut tables = source.tables.map(Some);
        for (table, view) in TableId::ALL.into_iter().zip(tables.iter_mut()) {
            if !matches!(
                view,
                Some(TableInput::SliceUpdate { .. } | TableInput::UniformOptionalI32 { .. })
            ) {
                continue;
            }
            if keep_baseline(table) != BaselineRetention::Full || unchanged[table.index()] {
                continue;
            }
            let (update, fill) = match view.take() {
                Some(TableInput::SliceUpdate { update, .. }) => (update, None),
                Some(TableInput::UniformOptionalI32 {
                    column,
                    value,
                    update,
                    ..
                }) => (update, Some((column, value))),
                _ => unreachable!("matched above"),
            };
            let Some((hour, BaselineRows::Shared(rows))) =
                self.state.tables[table.index()].as_mut()
            else {
                unreachable!("validated shared slice baseline");
            };
            if Arc::get_mut(rows).is_none() {
                *rows = Arc::new(rows.try_clone()?);
            }
            let baseline = Arc::get_mut(rows).expect("unique slice baseline");
            let update = Arc::try_unwrap(update).unwrap_or_else(|shared| (*shared).clone());
            let replaced = update.apply(baseline)?;
            if let Some((column, value)) = fill {
                baseline.fill_optional_i32(column, value)?;
            }
            *hour = source.hour;
            retired.push(Retiring::Rows(replaced));
        }
        for (table, view) in TableId::ALL.into_iter().zip(tables) {
            let index = table.index();
            let Some(view) = view else {
                continue;
            };
            let retention = if table
                .last_change()
                .present(FormatVersion::CURRENT.version())
            {
                keep_baseline(table)
            } else {
                BaselineRetention::Drop
            };
            match retention {
                BaselineRetention::Drop => {
                    if let Some((_, rows)) = self.state.tables[index].take() {
                        push_baseline(&mut retired, rows);
                    }
                    push_input(&mut retired, view);
                }
                BaselineRetention::Meta => {
                    if unchanged[index] {
                        let baseline = self.state.tables[index]
                            .as_mut()
                            .expect("unchanged table has a validated baseline");
                        baseline.0 = source.hour;
                        let meta = match &baseline.1 {
                            BaselineRows::Owned(rows) => Some(BaselineRows::Meta {
                                len: Some(rows.len()),
                                shared: false,
                            }),
                            BaselineRows::Shared(rows) => Some(BaselineRows::Meta {
                                len: Some(rows.len()),
                                shared: true,
                            }),
                            BaselineRows::Meta { .. } => None,
                        };
                        if let Some(meta) = meta {
                            push_baseline(
                                &mut retired,
                                std::mem::replace(&mut baseline.1, meta),
                            );
                        }
                        push_input(&mut retired, view);
                    } else if matches!(view, TableInput::SliceUpdate { .. }) {
                        if let Some((_, old)) = std::mem::replace(
                            &mut self.state.tables[index],
                            Some((
                                source.hour,
                                BaselineRows::Meta {
                                    len: None,
                                    shared: true,
                                },
                            )),
                        ) {
                            push_baseline(&mut retired, old);
                        }
                        push_input(&mut retired, view);
                    } else {
                        let next = match &view {
                            TableInput::Sampled(TableView::Complete(rows)) => Some((
                                source.hour,
                                BaselineRows::Meta {
                                    len: Some(rows.len()),
                                    shared: false,
                                },
                            )),
                            TableInput::SharedComplete(rows) => Some((
                                source.hour,
                                BaselineRows::Meta {
                                    len: Some(rows.len()),
                                    shared: true,
                                },
                            )),
                            _ => None,
                        };
                        if let Some((_, rows)) =
                            std::mem::replace(&mut self.state.tables[index], next)
                        {
                            push_baseline(&mut retired, rows);
                        }
                        push_input(&mut retired, view);
                    }
                }
                BaselineRetention::Full => {
                    if unchanged[index] {
                        let baseline = self.state.tables[index]
                            .as_mut()
                            .expect("unchanged table has a validated baseline");
                        baseline.0 = source.hour;
                        match view {
                            TableInput::SharedComplete(rows) => push_baseline(
                                &mut retired,
                                std::mem::replace(&mut baseline.1, BaselineRows::Shared(rows)),
                            ),
                            view => push_input(&mut retired, view),
                        }
                    } else if let TableInput::ComparedChanges { indices, rows, .. } = view {
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
                    } else {
                        let next = match view {
                            TableInput::Sampled(TableView::Complete(rows)) => {
                                Some((source.hour, BaselineRows::Owned(rows)))
                            }
                            TableInput::SharedComplete(rows) => {
                                Some((source.hour, BaselineRows::Shared(rows)))
                            }
                            view => {
                                push_input(&mut retired, view);
                                None
                            }
                        };
                        if let Some((_, rows)) =
                            std::mem::replace(&mut self.state.tables[index], next)
                        {
                            push_baseline(&mut retired, rows);
                        }
                    }
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
