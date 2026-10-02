use super::{
    BaselineToken, DecodedFrame, EncodedFrame, Frame, FrameInput, TABLE_COUNT, TableInput,
    TableView, put_count, validate_complete, validate_rows, validate_singleton,
};
use crate::{
    BranchId, Hour, KEYFRAME_INTERVAL,
    codec::{invalid, put_string},
    rows::{TableId, TableRows, TableSchema},
};
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
        // Clock payloads depend on the enclosing hour and therefore cannot be
        // reused through a comparison with the preceding hour's complete rows.
        if T::ID == TableId::Clock
            || self.needs_keyframe(branch, hour, force)
            || self
                .state
                .last_hour
                .and_then(|previous| previous.checked_add(1))
                != Some(hour)
        {
            return None;
        }
        let (baseline, values) = self.state.tables[T::ID.index()].as_ref()?;
        Some((
            BaselineToken {
                encoder: self.identity,
                revision: self.revision,
                table: T::ID,
                branch,
                hour,
                baseline: *baseline,
                row_updates: matches!(values, BaselineRows::Owned(_))
                    && T::ID != TableId::Countries
                    && !crate::cadence::is_staggered(T::ID),
            },
            values.as_ref().rows::<T>()?,
        ))
    }

    fn encode_complete(
        &mut self,
        table: TableId,
        values: &TableRows,
        hour: Hour,
        keyframe: bool,
        output: &mut Vec<u8>,
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
            if values.encode_changes(previous.as_ref(), output, &mut self.deleted)? {
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

    /// Segment recovery forces a keyframe. Baselines commit only after all input
    /// tables have been validated and encoded successfully into the owned payload.
    pub fn encode_bytes(
        &mut self,
        source: FrameInput,
        force_keyframe: bool,
    ) -> io::Result<EncodedFrame> {
        let new_branch = self.state.branch != Some(source.branch);
        if !new_branch && self.state.last_hour.is_some_and(|hour| hour >= source.hour) {
            return Err(invalid("frame hour did not advance on its branch"));
        }
        let revision = self
            .revision
            .checked_add(1)
            .ok_or_else(|| invalid("encoder revision space exhausted"))?;
        let keyframe = self.needs_keyframe(source.branch, source.hour, force_keyframe);
        let mut output = vec![0; EncodedFrame::PREFIX_BYTES];
        output.extend_from_slice(&source.branch.to_le_bytes());
        output.extend_from_slice(&source.hour.to_le_bytes());
        output.push(u8::from(keyframe));
        output.extend_from_slice(&source.metrics.total_ns.to_le_bytes());
        for table in TableId::ALL {
            output.extend_from_slice(&source.metrics.table_ns[table.index()].to_le_bytes());
        }
        output.extend_from_slice(&source.metrics.pool_bytes.to_le_bytes());
        output.extend_from_slice(&source.metrics.dropped_frames.to_le_bytes());
        let mut unchanged = [false; TABLE_COUNT];
        for table in TableId::ALL {
            let view = &source.tables[table.index()];
            unchanged[table.index()] = match view {
                TableInput::SharedComplete(values) => {
                    if !keyframe
                        && !new_branch
                        && let Some((baseline, BaselineRows::Shared(previous))) =
                            self.state.tables[table.index()].as_ref()
                        && Arc::ptr_eq(values, previous)
                    {
                        validate_singleton(table, values, source.hour)?;
                        output.push(2);
                        output.extend_from_slice(&baseline.to_le_bytes());
                        true
                    } else {
                        self.encode_complete(table, values, source.hour, keyframe, &mut output)?
                    }
                }
                TableInput::ComparedEqual(token) => {
                    self.validate_comparison(*token, table, source.branch, source.hour, keyframe)?;
                    output.push(2);
                    output.extend_from_slice(&token.baseline.to_le_bytes());
                    true
                }
                TableInput::ComparedChanges {
                    token,
                    indices,
                    rows,
                } => {
                    self.validate_comparison(*token, table, source.branch, source.hour, keyframe)?;
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
                    if rows.encode_replacements(previous, indices, &mut output)? {
                        false
                    } else {
                        output.truncate(start);
                        output.push(2);
                        output.extend_from_slice(&token.baseline.to_le_bytes());
                        true
                    }
                }
                TableInput::Sampled(TableView::Complete(values)) => {
                    self.encode_complete(table, values, source.hour, keyframe, &mut output)?
                }
                TableInput::Sampled(TableView::Truncated { rows, missing }) => {
                    validate_rows(table, rows)?;
                    if *missing == 0 {
                        return Err(invalid("truncated table has no missing rows"));
                    }
                    if keyframe {
                        output.push(4);
                        put_string(
                            &mut output,
                            &format!("truncated at keyframe: {missing} rows not read"),
                        )?;
                    } else {
                        output.push(3);
                        output.extend_from_slice(&missing.to_le_bytes());
                        put_count(&mut output, rows.len())?;
                        rows.encode_rows(&mut output)?;
                    }
                    false
                }
                TableInput::Sampled(TableView::Unavailable { reason }) => {
                    if reason.is_empty() {
                        return Err(invalid("unavailable table needs a reason"));
                    }
                    if reason == crate::cadence::SKIPPED_REASON
                        && self.can_carry(table, source.branch, source.hour, force_keyframe)
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
                        put_string(&mut output, reason)?;
                        false
                    }
                }
            };
        }
        for (table, view) in TableId::ALL.into_iter().zip(source.tables) {
            let index = table.index();
            if let TableInput::ComparedChanges { indices, rows, .. } = view {
                let (hour, BaselineRows::Owned(previous)) = self.state.tables[index]
                    .as_mut()
                    .expect("validated replacement baseline")
                else {
                    unreachable!("validated owned replacement baseline")
                };
                previous.install_replacements(rows, &indices);
                *hour = source.hour;
            } else if unchanged[index] {
                let baseline = self.state.tables[index]
                    .as_mut()
                    .expect("unchanged table has a validated baseline");
                baseline.0 = source.hour;
                if let TableInput::SharedComplete(rows) = view {
                    baseline.1 = BaselineRows::Shared(rows);
                }
            } else {
                self.state.tables[index] = match view {
                    TableInput::Sampled(TableView::Complete(rows)) => {
                        Some((source.hour, BaselineRows::Owned(rows)))
                    }
                    TableInput::SharedComplete(rows) => {
                        Some((source.hour, BaselineRows::Shared(rows)))
                    }
                    _ => None,
                };
            }
        }
        self.state.branch = Some(source.branch);
        self.state.last_hour = Some(source.hour);
        self.revision = revision;
        if keyframe {
            self.keyframe_hour = Some(source.hour);
        }
        Ok(EncodedFrame {
            branch: source.branch,
            hour: source.hour,
            keyframe,
            bytes: output,
        })
    }
}
