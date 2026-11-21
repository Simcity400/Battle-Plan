use super::*;

#[cfg(test)]
mod tests;

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    static KEY_BY_KEY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(any(test, feature = "test-support"))]
pub fn key_by_key<T>(run: impl FnOnce() -> T) -> T {
    struct Restore;
    impl Drop for Restore {
        fn drop(&mut self) {
            KEY_BY_KEY.with(|flag| flag.set(false));
        }
    }
    KEY_BY_KEY.with(|flag| flag.set(true));
    let _restore = Restore;
    run()
}

#[derive(Clone, Debug, Default, PartialEq)]
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
    validated: [Option<Weak<TableView>>; TABLE_COUNT],
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
            validated: std::array::from_fn(|_| None),
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
                        None => state.reset = true,
                    }
                }
                (_, TableView::Complete(_)) => state.reset = true,
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
                        let full = full_mask(table.schema().len());
                        let walk = ascending(changed.iter().map(|row| row.key))
                            && ascending(deleted.iter().copied());
                        #[cfg(any(test, feature = "test-support"))]
                        let walk = walk && !KEY_BY_KEY.with(std::cell::Cell::get);
                        if walk {
                            state.apply_ascending(changed, deleted, full, &mut boundary)?;
                        } else {
                            state.apply_each(changed, deleted, full, &mut boundary)?;
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
        for ((validated, view), &selected) in self
            .validated
            .iter_mut()
            .zip(&frame.tables)
            .zip(&self.selected)
        {
            *validated = (selected && matches!(&**view, TableView::Complete(_)))
                .then(|| Arc::downgrade(&view.0));
        }
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

    pub fn same_root(tables: &[TableId], a: &DecodedFrame, b: &DecodedFrame) -> bool {
        Point::of(a) == Point::of(b)
            && a.metrics == b.metrics
            && tables.iter().all(|table| {
                let (a, b) = (&a.tables[table.index()], &b.tables[table.index()]);
                SharedView::ptr_eq(a, b) && a.sampled_at() == b.sampled_at()
            })
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
                    if table == TableId::Clock
                        || self.validated[table.index()]
                            .as_ref()
                            .is_none_or(|validated| validated.as_ptr() != Arc::as_ptr(&view.0))
                    {
                        validate_complete(table, rows, frame.hour)?;
                    }
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

    fn apply_each(
        &mut self,
        changed: &[DeltaRow],
        deleted: &[u128],
        full: u64,
        boundary: &mut MembershipBoundary,
    ) -> io::Result<()> {
        for &key in deleted {
            if let Ok(at) = self.members.binary_search(&key) {
                self.members.remove(at);
                push(&mut boundary.deleted, key)?;
            }
            if !self.reset {
                if let Ok(at) = self.edits.binary_search_by_key(&key, |edit| edit.0) {
                    self.edits.remove(at);
                }
                if let Ok(at) = self.introduced.binary_search(&key) {
                    self.introduced.remove(at);
                } else {
                    insert_key(&mut self.deleted, key)?;
                }
            }
        }
        for row in changed {
            let inserted = match self.members.binary_search(&row.key) {
                Ok(_) => false,
                Err(at) => {
                    allocation::reserve(&mut self.members, 1)?;
                    self.members.insert(at, row.key);
                    push(&mut boundary.inserted, row.key)?;
                    true
                }
            };
            if !self.reset {
                if let Ok(at) = self.deleted.binary_search(&row.key) {
                    self.deleted.remove(at);
                } else if inserted {
                    insert_key(&mut self.introduced, row.key)?;
                }
                let mask = if inserted { full } else { row.mask };
                match self.edits.binary_search_by_key(&row.key, |edit| edit.0) {
                    Ok(at) => {
                        self.edits[at].1 |= mask;
                        self.edits[at].2 |= inserted;
                    }
                    Err(at) => {
                        allocation::reserve(&mut self.edits, 1)?;
                        self.edits.insert(at, (row.key, mask, inserted));
                    }
                }
            }
        }
        Ok(())
    }

    fn apply_ascending(
        &mut self,
        changed: &[DeltaRow],
        deleted: &[u128],
        full: u64,
        boundary: &mut MembershipBoundary,
    ) -> io::Result<()> {
        let mut found = [0; 4];
        let (mut members, mut edits, mut introduced) = (Vec::new(), Vec::new(), Vec::new());
        let mut gone = Vec::new();
        for &key in deleted {
            if let Ok(at) = gallop(&self.members, &mut found[0], key, |&key| key) {
                push(&mut members, at)?;
                push(&mut boundary.deleted, key)?;
            }
            if self.reset {
                continue;
            }
            if let Ok(at) = gallop(&self.edits, &mut found[1], key, |edit| edit.0) {
                push(&mut edits, at)?;
            }
            match gallop(&self.introduced, &mut found[2], key, |&key| key) {
                Ok(at) => push(&mut introduced, at)?,
                Err(_) => {
                    if gallop(&self.deleted, &mut found[3], key, |&key| key).is_err() {
                        push(&mut gone, key)?;
                    }
                }
            }
        }
        remove_at(&mut self.members, &members);
        remove_at(&mut self.edits, &edits);
        remove_at(&mut self.introduced, &introduced);
        insert_ascending(&mut self.deleted, &gone, |&key| key)?;
        let mut found = [0; 4];
        let (mut returned, mut introduced, mut added) = (Vec::new(), Vec::new(), Vec::new());
        for row in changed {
            let inserted = gallop(&self.members, &mut found[0], row.key, |&key| key).is_err();
            if inserted {
                push(&mut boundary.inserted, row.key)?;
            }
            if self.reset {
                continue;
            }
            match gallop(&self.deleted, &mut found[1], row.key, |&key| key) {
                Ok(at) => push(&mut returned, at)?,
                Err(_) => {
                    if inserted
                        && gallop(&self.introduced, &mut found[2], row.key, |&key| key).is_err()
                    {
                        push(&mut introduced, row.key)?;
                    }
                }
            }
            let mask = if inserted { full } else { row.mask };
            match gallop(&self.edits, &mut found[3], row.key, |edit| edit.0) {
                Ok(at) => {
                    self.edits[at].1 |= mask;
                    self.edits[at].2 |= inserted;
                }
                Err(_) => push(&mut added, (row.key, mask, inserted))?,
            }
        }
        insert_ascending(&mut self.members, &boundary.inserted, |&key| key)?;
        remove_at(&mut self.deleted, &returned);
        insert_ascending(&mut self.introduced, &introduced, |&key| key)?;
        insert_ascending(&mut self.edits, &added, |edit| edit.0)
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
fn ascending(mut keys: impl Iterator<Item = u128>) -> bool {
    let Some(mut previous) = keys.next() else {
        return true;
    };
    keys.all(|key| std::mem::replace(&mut previous, key) < key)
}
fn gallop<T>(
    items: &[T],
    from: &mut usize,
    key: u128,
    key_of: impl Fn(&T) -> u128,
) -> Result<usize, usize> {
    let mut reach = 1;
    while *from + reach <= items.len() && key_of(&items[*from + reach - 1]) < key {
        reach *= 2;
    }
    let start = *from + reach / 2;
    let end = (*from + reach).min(items.len());
    let found = items[start..end]
        .binary_search_by_key(&key, key_of)
        .map(|at| start + at)
        .map_err(|at| start + at);
    *from = found.unwrap_or_else(|at| at);
    found
}
fn remove_at<T: Copy>(items: &mut Vec<T>, positions: &[usize]) {
    let Some(&first) = positions.first() else {
        return;
    };
    let mut to = first;
    for (index, &at) in positions.iter().enumerate() {
        let end = positions.get(index + 1).copied().unwrap_or(items.len());
        items.copy_within(at + 1..end, to);
        to += end - at - 1;
    }
    items.truncate(to);
}
fn insert_ascending<T: Copy>(
    items: &mut Vec<T>,
    added: &[T],
    key_of: impl Fn(&T) -> u128,
) -> io::Result<()> {
    let Some(&last) = added.last() else {
        return Ok(());
    };
    allocation::reserve(items, added.len())?;
    let mut end = items.len();
    items.resize(end + added.len(), last);
    let mut to = items.len();
    for item in added.iter().rev() {
        let at = items[..end].partition_point(|old| key_of(old) < key_of(item));
        items.copy_within(at..end, to - (end - at));
        to -= end - at + 1;
        items[to] = *item;
        end = at;
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
