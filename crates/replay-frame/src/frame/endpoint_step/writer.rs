use super::*;

#[derive(Default)]
struct Pending {
    reset: bool,
    known: bool,
    rebuilt: bool,
    members: Vec<u128>,
    introduced: Vec<u128>,
    edits: Vec<(u128, u64, bool)>,
    deleted: Vec<u128>,
    boundaries: Vec<MembershipBoundary>,
}

pub struct EndpointWriter {
    selected: [bool; TABLE_COUNT],
    pending: [Pending; TABLE_COUNT],
    observed: Option<Point>,
    emitted: Option<Point>,
    poisoned: bool,
}
impl EndpointWriter {
    pub fn for_tables(tables: &[TableId]) -> Self {
        let mut selected = [false; TABLE_COUNT];
        for table in tables {
            selected[table.index()] = true;
        }
        Self {
            selected,
            pending: std::array::from_fn(|_| Pending {
                reset: true,
                ..Default::default()
            }),
            observed: None,
            emitted: None,
            poisoned: false,
        }
    }

    pub fn resume(tables: &[TableId], frame: &DecodedFrame) -> io::Result<Self> {
        let mut writer = Self::for_tables(tables);
        for table in TableId::ALL {
            if !writer.selected[table.index()] {
                continue;
            }
            let state = &mut writer.pending[table.index()];
            state.reset = false;
            if let TableView::Complete(rows) = &*frame.tables[table.index()] {
                state.members = allocation::collect_vec(rows.iter().map(|row| row.key()))?;
                state.known = true;
            }
        }
        writer.observed = Some(Point::of(frame));
        writer.emitted = writer.observed;
        Ok(writer)
    }

    pub fn diff(
        tables: &[TableId],
        previous: &DecodedFrame,
        frame: &DecodedFrame,
        out: &mut Vec<u8>,
    ) -> io::Result<EndpointStats> {
        let (from, to) = (Point::of(previous), Point::of(frame));
        if from.branch != to.branch || from.hour >= to.hour {
            return Err(invalid("endpoint observation does not advance its branch"));
        }
        let mut writer = Self::resume(tables, previous)?;
        for table in TableId::ALL {
            if !writer.selected[table.index()] {
                continue;
            }
            let (before, after) = (
                &previous.tables[table.index()],
                &frame.tables[table.index()],
            );
            let state = &mut writer.pending[table.index()];
            match (&**before, &**after) {
                (_, TableView::Complete(_)) if SharedView::ptr_eq(before, after) => {}
                (TableView::Complete(old), TableView::Complete(new)) => {
                    state.rebuilt = true;
                    match changes(table, old, new, from.hour)? {
                        Some((changed, deleted)) => {
                            for row in changed {
                                let inserted = state.members.binary_search(&row.key).is_err();
                                let mask = if inserted {
                                    full_mask(table.schema().len())
                                } else {
                                    row.mask
                                };
                                push(&mut state.edits, (row.key, mask, inserted))?;
                            }
                            state.deleted = deleted;
                        }
                        // Rows the plain delta cannot describe are written whole.
                        None => state.reset = true,
                    }
                }
                (_, TableView::Complete(_)) => state.reset = true,
                // `encode` writes a truncated or unavailable view as `frame` holds it.
                _ => {}
            }
        }
        writer.observed = Some(to);
        let start = out.len();
        let result = writer.encode(frame, out);
        if result.is_err() {
            out.truncate(start);
        }
        result
    }

    pub fn observe(
        &mut self,
        source: &(impl FrameSource + ?Sized),
        frame: &DecodedFrame,
    ) -> io::Result<()> {
        if self.poisoned {
            return Err(invalid("endpoint writer requires a new root after failure"));
        }
        let point = Point::of(frame);
        if source.branch() != point.branch
            || source.hour() != point.hour
            || source.metrics() != &frame.metrics
        {
            return Err(invalid("endpoint source differs from reconstruction"));
        }
        if self.observed.is_some_and(|before| {
            before.branch != point.branch || before.hour.checked_add(1) != Some(point.hour)
        }) {
            return Err(invalid(
                "endpoint observation crosses a branch or native gap",
            ));
        }
        self.poisoned = true;
        for table in TableId::ALL {
            if !self.selected[table.index()] {
                continue;
            }
            let state = &mut self.pending[table.index()];
            let mut boundary = MembershipBoundary {
                table,
                hour: frame.hour,
                kind: BoundaryKind::Delta,
                inserted: Vec::new(),
                deleted: Vec::new(),
            };
            match &*frame.tables[table.index()] {
                TableView::Complete(rows) => {
                    if !state.known
                        || matches!(source.update(table.index()), TableUpdate::Snapshot(_))
                    {
                        boundary.kind = BoundaryKind::CompleteReset;
                        boundary.inserted =
                            allocation::collect_vec(rows.iter().map(|row| row.key()))?;
                        state.members = allocation::collect_vec(boundary.inserted.iter().copied())?;
                        state.reset = true;
                        state.known = true;
                        state.edits.clear();
                        state.deleted.clear();
                        state.introduced.clear();
                    } else if let TableUpdate::Delta {
                        changed, deleted, ..
                    } = source.update(table.index())
                    {
                        state.rebuilt = true;
                        for &key in deleted {
                            if let Ok(at) = state.members.binary_search(&key) {
                                state.members.remove(at);
                                push(&mut boundary.deleted, key)?;
                            }
                            if !state.reset {
                                if let Ok(at) =
                                    state.edits.binary_search_by_key(&key, |edit| edit.0)
                                {
                                    state.edits.remove(at);
                                }
                                if let Ok(at) = state.introduced.binary_search(&key) {
                                    state.introduced.remove(at);
                                } else {
                                    insert_key(&mut state.deleted, key)?;
                                }
                            }
                        }
                        for row in changed {
                            let inserted = match state.members.binary_search(&row.key) {
                                Ok(_) => false,
                                Err(at) => {
                                    allocation::reserve(&mut state.members, 1)?;
                                    state.members.insert(at, row.key);
                                    push(&mut boundary.inserted, row.key)?;
                                    true
                                }
                            };
                            if !state.reset {
                                if let Ok(at) = state.deleted.binary_search(&row.key) {
                                    state.deleted.remove(at);
                                } else if inserted {
                                    insert_key(&mut state.introduced, row.key)?;
                                }
                                let mask = if inserted {
                                    full_mask(table.schema().len())
                                } else {
                                    row.mask
                                };
                                match state.edits.binary_search_by_key(&row.key, |edit| edit.0) {
                                    Ok(at) => {
                                        state.edits[at].1 |= mask;
                                        state.edits[at].2 |= inserted;
                                    }
                                    Err(at) => {
                                        allocation::reserve(&mut state.edits, 1)?;
                                        state.edits.insert(at, (row.key, mask, inserted));
                                    }
                                }
                            }
                        }
                    }
                }
                TableView::Truncated { rows, .. } => {
                    boundary.kind = BoundaryKind::Partial;
                    boundary.inserted = allocation::collect_vec(rows.iter().map(|row| row.key()))?;
                    state.invalidate();
                }
                TableView::Unavailable { .. } | TableView::NotRecorded { .. } => {
                    boundary.kind = BoundaryKind::Unavailable;
                    state.invalidate();
                }
            }
            if boundary.kind != BoundaryKind::Delta
                || !boundary.inserted.is_empty()
                || !boundary.deleted.is_empty()
            {
                push(&mut state.boundaries, boundary)?;
            }
        }
        self.observed = Some(point);
        self.poisoned = false;
        Ok(())
    }

    pub fn finish(&mut self, frame: &DecodedFrame, out: &mut Vec<u8>) -> io::Result<EndpointStats> {
        if self.poisoned || self.observed != Some(Point::of(frame)) || self.emitted == self.observed
        {
            return Err(invalid("endpoint output is not the pending observed hour"));
        }
        let start = out.len();
        let result = self.encode(frame, out);
        if result.is_err() {
            out.truncate(start);
            return result;
        }
        self.emitted = self.observed;
        for state in &mut self.pending {
            state.reset = false;
            state.rebuilt = false;
            state.edits.clear();
            state.deleted.clear();
            state.introduced.clear();
            state.boundaries.clear();
        }
        result
    }

    pub fn root(
        frame: &DecodedFrame,
        tables: &[TableId],
        out: &mut Vec<u8>,
    ) -> io::Result<EndpointStats> {
        let writer = Self::for_tables(tables);
        let start = out.len();
        let result = writer.encode(frame, out);
        if result.is_err() {
            out.truncate(start);
        }
        result
    }

    fn encode(&self, frame: &DecodedFrame, out: &mut Vec<u8>) -> io::Result<EndpointStats> {
        let start = out.len();
        allocation::reserve(out, 8)?;
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&VERSION.to_le_bytes());
        out.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
        put(&self.emitted.is_some(), out)?;
        if let Some(point) = self.emitted {
            point.write(out)?;
        }
        Point::of(frame).write(out)?;
        put(&frame.metrics.total_ns, out)?;
        put(&frame.metrics.table_ns, out)?;
        put(&frame.metrics.pool_bytes, out)?;
        put(&frame.metrics.dropped_frames, out)?;
        put_count(
            self.selected.iter().filter(|&&selected| selected).count(),
            out,
        )?;
        let mut stats = EndpointStats::default();
        let mut payload = Vec::new();
        for table in TableId::ALL {
            if !self.selected[table.index()] {
                continue;
            }
            payload.clear();
            let state = &self.pending[table.index()];
            let view = &frame.tables[table.index()];
            let sample = view.sampled_at();
            if sample
                .is_some_and(|hour| hour > frame.hour || !matches!(&**view, TableView::Complete(_)))
            {
                return Err(invalid("invalid endpoint sample hour"));
            }
            put(&sample, &mut payload)?;
            match &**view {
                TableView::Complete(rows) => {
                    validate_complete(table, rows, frame.hour)?;
                    if state.reset
                        || (state.rebuilt && state.edits.is_empty() && state.deleted.is_empty())
                    {
                        put(&1_u8, &mut payload)?;
                        rows.encode_checkpoint_rows(&mut payload)?;
                    } else if state.edits.is_empty() && state.deleted.is_empty() {
                        put(&0_u8, &mut payload)?;
                    } else {
                        put(&2_u8, &mut payload)?;
                        rows.encode_endpoint_rows(&state.edits, &mut payload)?;
                        write_keys(&state.deleted, &mut payload)?;
                        stats.edited_rows += state.edits.len();
                        stats.deleted_rows += state.deleted.len();
                    }
                }
                TableView::Truncated { rows, missing } => {
                    validate_rows(table, rows)?;
                    if *missing == 0 {
                        return Err(invalid("truncated endpoint has no missing rows"));
                    }
                    put(&3_u8, &mut payload)?;
                    put(missing, &mut payload)?;
                    rows.encode_checkpoint_rows(&mut payload)?;
                }
                TableView::Unavailable { reason } => {
                    if reason.is_empty() {
                        return Err(invalid("unavailable endpoint needs a reason"));
                    }
                    put(&4_u8, &mut payload)?;
                    put_string(&mut payload, reason)?;
                }
                TableView::NotRecorded { mode } => {
                    put(&5_u8, &mut payload)?;
                    put(&mode.byte(), &mut payload)?;
                }
            }
            let history_start = payload.len();
            write_boundaries(&state.boundaries, &mut payload)?;
            stats.boundary_bytes += payload.len() - history_start;
            let table_start = out.len();
            put(
                &u16::try_from(table.index())
                    .map_err(|_| invalid("endpoint table index exceeds u16"))?,
                out,
            )?;
            put_bytes(out, &payload)?;
            stats.table_bytes[table.index()] = out.len() - table_start;
        }
        stats.bytes = out.len() - start;
        Ok(stats)
    }
}
impl Pending {
    fn invalidate(&mut self) {
        self.known = false;
        self.reset = true;
        self.members.clear();
        self.edits.clear();
        self.deleted.clear();
        self.introduced.clear();
    }
}
fn push<T>(out: &mut Vec<T>, value: T) -> io::Result<()> {
    allocation::reserve(out, 1)?;
    out.push(value);
    Ok(())
}
fn insert_key(keys: &mut Vec<u128>, key: u128) -> io::Result<()> {
    if let Err(at) = keys.binary_search(&key) {
        allocation::reserve(keys, 1)?;
        keys.insert(at, key);
    }
    Ok(())
}

fn changes(
    table: TableId,
    old: &TableRows,
    new: &TableRows,
    baseline: Hour,
) -> io::Result<Option<(Vec<DeltaRow>, Vec<u128>)>> {
    let mut bytes = allocation::vec(5)?;
    bytes.push(1);
    bytes.extend_from_slice(&baseline.to_le_bytes());
    let mut scratch = Vec::new();
    match new.encode_changes(old, &mut bytes, &mut scratch) {
        Ok(true) => {}
        Ok(false) => return Ok(Some((Vec::new(), Vec::new()))),
        Err(error) if allocation::is_allocation_error(&error) => return Err(error),
        Err(_) => return Ok(None),
    }
    let mut input = Decoder::new(&bytes);
    let update = match decode_update(
        table,
        &crate::rows::compatibility::HISTORY[table.index()],
        crate::format::FormatVersion::CURRENT,
        false,
        &mut input,
    ) {
        Ok(update) => update,
        Err(error) if allocation::is_allocation_error(&error) => return Err(error),
        Err(_) => return Ok(None),
    };
    if input.finish().is_err() {
        return Ok(None);
    }
    Ok(match update {
        TableUpdate::Delta {
            changed, deleted, ..
        } => Some((changed, deleted)),
        _ => None,
    })
}
