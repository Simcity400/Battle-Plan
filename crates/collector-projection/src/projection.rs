
use crate::table_conversion::{table, unavailable};
use engine_native_memory::walk::Outcome;
use replay_data::rows::{TABLE_COUNT, TableId};
use replay_data::{BranchId, Hour, rows::RowCodec, rows::TableRows, rows::TableSchema};
use replay_frame::{frame::FrameEncoder, frame::TableInput, frame::TableView};
use std::sync::{Mutex, MutexGuard, PoisonError};

pub const WITNESS_WORDS: usize = 9;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Witness([u64; WITNESS_WORDS]);

impl Witness {
    pub fn new<const K: usize>(words: [u64; K]) -> Self {
        const { assert!(K <= WITNESS_WORDS) };
        let mut all = [0; WITNESS_WORDS];
        all[..K].copy_from_slice(&words);
        Self(all)
    }

    pub fn generation(generation: u64) -> Self {
        Self::new([generation])
    }
}

pub fn generation() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

struct Held {
    branch: BranchId,
    hour: Hour,
    witness: Option<Witness>,
}

#[derive(Default)]
struct Ordering {
    indices: Vec<usize>,
    compared: u64,
    reused: u64,
    converted: u64,
    updated: u64,
    updated_rows: u64,
    sorted: u64,
    certified: Option<(BranchId, Hour, u64)>,
    held: Option<Held>,
    republished: u64,
    held_converted: u64,
    ledger: Option<Box<dyn std::any::Any + Send>>,
}

pub struct ProjectionScratch {
    tables: [Mutex<Ordering>; TABLE_COUNT],
}

impl Default for ProjectionScratch {
    fn default() -> Self {
        Self {
            tables: std::array::from_fn(|_| Mutex::default()),
        }
    }
}

impl ProjectionScratch {
    fn table(&self, table: replay_data::rows::TableId) -> MutexGuard<'_, Ordering> {
        self.tables[table.index()]
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    pub fn republish_counts(&self, table: TableId) -> [u64; 2] {
        let order = self.table(table);
        [order.republished, order.held_converted]
    }

    pub fn forget(&self, table: TableId) {
        self.table(table).held = None;
    }

}

#[derive(Clone, Copy)]
pub struct Projection<'a> {
    encoder: &'a FrameEncoder,
    branch: BranchId,
    hour: Hour,
    force: bool,
    scratch: &'a ProjectionScratch,
}

impl<'a> Projection<'a> {
    pub fn observed_baseline<S: TableSchema>(&self) -> Option<&'a [S::Row]> {
        self.encoder.observed_baseline::<S>(self.branch, self.hour)
    }

    pub fn observed_shared_baseline<S: TableSchema>(
        &self,
    ) -> Option<&'a std::sync::Arc<TableRows>> {
        self.observed_baseline::<S>()?;
        self.encoder.shared_baseline(S::ID)
    }

    pub fn observed_comparison_baseline<S: TableSchema>(
        &self,
    ) -> Option<(replay_frame::frame::BaselineToken, &'a [S::Row])> {
        self.encoder
            .observed_comparison_baseline::<S>(self.branch, self.hour, self.force)
    }

    pub fn slice_baseline_rows<S: TableSchema>(
        &self,
    ) -> Option<(replay_frame::frame::BaselineToken, &'a [S::Row])> {
        let (token, previous) =
            self.encoder
                .slice_baseline(S::ID, self.branch, self.hour, self.force)?;
        Some((token, previous.rows::<S>()?))
    }

    pub fn current_stamp(&self) -> (u64, BranchId, Hour) {
        (self.encoder.identity(), self.branch, self.hour)
    }

    pub fn take_custom<T: Send + 'static>(&self, table: TableId) -> Option<Box<T>> {
        self.scratch
            .table(table)
            .ledger
            .take()
            .and_then(|state| state.downcast().ok())
    }

    pub fn put_custom<T: Send + 'static>(&self, table: TableId, state: Box<T>) {
        self.scratch.table(table).ledger = Some(state);
    }

    pub fn note_compared(&self, table: TableId, updated_rows: Option<usize>) {
        let mut order = self.scratch.table(table);
        order.compared = order.compared.saturating_add(1);
        match updated_rows {
            None => order.reused = order.reused.saturating_add(1),
            Some(rows) => {
                order.updated = order.updated.saturating_add(1);
                order.updated_rows = order.updated_rows.saturating_add(rows as u64);
            }
        }
    }

    pub fn timed<R>(&self, table: TableId, work: impl FnOnce() -> R) -> R {
        crate::table_clock::timed(table, work)
    }

    pub fn new(
        encoder: &'a FrameEncoder,
        branch: BranchId,
        hour: Hour,
        force: bool,
        scratch: &'a ProjectionScratch,
    ) -> Self {
        Self {
            encoder,
            branch,
            hour,
            force,
            scratch,
        }
    }

    pub fn republish<S: TableSchema>(&self, witness: Option<Witness>) -> Option<TableInput> {
        let witness = witness?;
        let mut order = self.scratch.table(S::ID);
        let order = &mut *order;
        let held = order.held.as_mut()?;
        if held.branch != self.branch
            || held.hour.checked_add(1) != Some(self.hour)
            || held.witness != Some(witness)
        {
            return None;
        }
        let (token, committed_hour) =
            self.encoder
                .republish_token(S::ID, self.branch, self.hour, self.force)?;
        if committed_hour != held.hour {
            return None;
        }
        held.hour = self.hour;
        order.republished = order.republished.saturating_add(1);
        Some(TableInput::ComparedEqual(token))
    }

    pub fn republish_uniform_optional_i32<S: TableSchema>(
        &self,
        witness: Option<Witness>,
        column: usize,
        value: Option<i32>,
    ) -> Option<TableInput> {
        let witness = witness?;
        let mut order = self.scratch.table(S::ID);
        let order = &mut *order;
        let held = order.held.as_mut()?;
        if held.branch != self.branch
            || held.hour.checked_add(1) != Some(self.hour)
            || held.witness != Some(witness)
        {
            return None;
        }
        let (token, _) =
            self.encoder
                .slice_baseline(S::ID, self.branch, self.hour, self.force)?;
        if self.encoder.committed::<S>()?.0 != held.hour {
            return None;
        }
        held.hour = self.hour;
        order.republished = order.republished.saturating_add(1);
        Some(TableInput::UniformOptionalI32 {
            token,
            column,
            value,
            update: std::sync::Arc::new(replay_data::sliced::SliceUpdate::keyed(
                Vec::new(),
                S::wrap(Vec::new()),
            )),
        })
    }

    pub fn retain(&self, table: TableId, witness: Option<Witness>, input: &TableInput) {
        let complete = matches!(
            input,
            TableInput::Sampled(TableView::Complete(_))
                | TableInput::SharedComplete(_)
                | TableInput::ComparedEqual(_)
                | TableInput::ComparedChanges { .. }
                | TableInput::SliceUpdate { .. }
                | TableInput::UniformOptionalI32 { .. }
        );
        let mut order = self.scratch.table(table);
        order.held_converted = order.held_converted.saturating_add(1);
        order.held = complete.then_some(Held {
            branch: self.branch,
            hour: self.hour,
            witness,
        });
    }

    pub fn certified<S: TableSchema>(
        &self,
        snapshot: &crate::certified::Snapshot<S::Row>,
    ) -> TableInput {
        crate::table_clock::timed(S::ID, || self.certified_input::<S>(snapshot))
    }

    fn certified_input<S: TableSchema>(
        &self,
        snapshot: &crate::certified::Snapshot<S::Row>,
    ) -> TableInput {
        let mut scratch = self.scratch.table(S::ID);
        let baseline = self
            .encoder
            .slice_meta(S::ID, self.branch, self.hour, self.force);
        let previous = self.encoder.committed_hour(S::ID);
        let input = if let (Some((token, _)), Some((branch, hour, revision))) =
            (baseline, scratch.certified)
            && branch == self.branch
            && previous == Some(hour)
        {
            if revision == snapshot.revision {
                TableInput::ComparedEqual(token)
            } else if revision == snapshot.parent {
                TableInput::SliceUpdate {
                    token,
                    update: std::sync::Arc::new(replay_data::sliced::SliceUpdate {
                        keys: Vec::new(),
                        owners: Vec::new(),
                        ranges: snapshot.ranges.clone(),
                        rows: S::wrap(snapshot.changed.clone()),
                    }),
                }
            } else {
                match snapshot.complete::<S>() {
                    Ok(rows) => TableInput::SharedComplete(std::sync::Arc::new(rows)),
                    Err(reason) => unavailable(reason).into(),
                }
            }
        } else {
            match snapshot.complete::<S>() {
                Ok(rows) => TableInput::SharedComplete(std::sync::Arc::new(rows)),
                Err(reason) => unavailable(reason).into(),
            }
        };
        scratch.certified = Some((self.branch, self.hour, snapshot.revision));
        match &input {
            TableInput::ComparedEqual(_) => {
                scratch.compared = scratch.compared.saturating_add(1);
                scratch.reused = scratch.reused.saturating_add(1);
            }
            TableInput::SliceUpdate { update, .. } => {
                scratch.compared = scratch.compared.saturating_add(1);
                scratch.updated = scratch.updated.saturating_add(1);
                scratch.updated_rows = scratch
                    .updated_rows
                    .saturating_add(update.rows().len() as u64);
            }
            _ => scratch.converted = scratch.converted.saturating_add(1),
        }
        input
    }

    pub fn certified_owners(&self, rows: TableRows, owners: Vec<u32>, full: bool) -> TableInput {
        if full {
            return TableInput::SharedComplete(std::sync::Arc::new(rows));
        }
        let update = replay_data::sliced::SliceUpdate {
            ranges: Vec::new(),
            owners,
            keys: Vec::new(),
            rows,
        };
        self.certified_update(update)
    }

    pub fn certified_keys(&self, rows: TableRows, keys: Vec<u128>, full: bool) -> TableInput {
        if full {
            return TableInput::SharedComplete(std::sync::Arc::new(rows));
        }
        let update = replay_data::sliced::SliceUpdate::keyed(keys, rows);
        self.certified_update(update)
    }

    fn certified_update(&self, update: replay_data::sliced::SliceUpdate) -> TableInput {
        let table = update.rows.table();
        let sparse = self
            .encoder
            .slice_baseline(table, self.branch, self.hour, self.force);
        let Some(previous) = sparse.map(|(_, previous)| previous).or_else(|| {
            self.encoder
                .certified_keyframe_baseline(table, self.branch, self.hour, self.force)
        }) else {
            return unavailable("certified update has no continuous complete baseline").into();
        };
        if !update.fits(previous) {
            return unavailable("certified update has invalid rows, owners or keys").into();
        }
        if let Some((token, _)) = sparse {
            return TableInput::SliceUpdate {
                token,
                update: std::sync::Arc::new(update),
            };
        }
        if update.owners.is_empty() && update.keys.is_empty() {
            return TableInput::SharedComplete(std::sync::Arc::clone(previous));
        }
        let merged = previous.try_clone().and_then(|mut rows| {
            update.apply(&mut rows)?;
            Ok(rows)
        });
        match merged {
            Ok(rows) => TableInput::SharedComplete(std::sync::Arc::new(rows)),
            Err(_) => unavailable("certified keyframe materialization failed").into(),
        }
    }

    fn table<S: TableSchema, T>(
        &mut self,
        outcome: Outcome,
        source: &[T],
        key: impl Fn(&T) -> u128,
        equal: impl Fn(&T, &S::Row) -> bool,
        convert: impl Fn(&T) -> S::Row,
    ) -> TableInput {
        let Some(source) = source.get(..outcome.len()) else {
            return unavailable("walker row count exceeds allocated buffer").into();
        };
        match outcome {
            Outcome::Unavailable { reason } => return unavailable(reason).into(),
            Outcome::Truncated { unread_rows: 0, .. } => {
                return unavailable("walker marked truncation without a missing count").into();
            }
            _ => {}
        }
        let mut order = self.scratch.table(S::ID);
        let order = &mut *order;
        let natural = source.windows(2).all(|rows| key(&rows[0]) < key(&rows[1]));
        if !natural {
            if order.indices.len() != source.len() {
                order.indices.clear();
                if engine_allocation::reserve_exact(&mut order.indices, source.len()).is_err() {
                    return unavailable("collector allocation failed").into();
                }
                order.indices.extend(0..source.len());
            }
            if !order
                .indices
                .windows(2)
                .all(|pair| key(&source[pair[0]]) < key(&source[pair[1]]))
            {
                order
                    .indices
                    .sort_unstable_by_key(|index| key(&source[*index]));
                order.sorted = order.sorted.saturating_add(1);
            }
            if order
                .indices
                .windows(2)
                .any(|pair| key(&source[pair[0]]) == key(&source[pair[1]]))
            {
                return unavailable("walker produced duplicate engine ids").into();
            }
        }
        let at = |index| &source[if natural { index } else { order.indices[index] }];
        if matches!(outcome, Outcome::Rows { .. })
            && !replay_data::cadence::is_staggered(S::ID)
            && let Some((token, baseline)) =
                self.encoder
                    .comparison_baseline::<S>(self.branch, self.hour, self.force)
        {
            order.compared = order.compared.saturating_add(1);
            if token.allows_row_updates() && source.len() == baseline.len() {
                let mut indices = Vec::new();
                let mut allocation_failed = false;
                let same_keys = baseline.iter().enumerate().all(|(index, previous)| {
                    let current = at(index);
                    if key(current) != previous.key() {
                        return false;
                    }
                    if !equal(current, previous) {
                        if indices.len() == indices.capacity()
                            && engine_allocation::reserve(&mut indices, 1).is_err()
                        {
                            allocation_failed = true;
                            return false;
                        }
                        indices.push(index);
                    }
                    true
                });
                if allocation_failed {
                    return unavailable("collector allocation failed").into();
                }
                if same_keys {
                    if indices.is_empty() {
                        order.reused = order.reused.saturating_add(1);
                        return TableInput::ComparedEqual(token);
                    }
                    let mut rows = Vec::new();
                    if engine_allocation::reserve_exact(&mut rows, indices.len()).is_err() {
                        return unavailable("collector allocation failed").into();
                    }
                    rows.extend(indices.iter().map(|&index| convert(at(index))));
                    let rows = TableRows::from_rows::<S>(rows);
                    order.updated = order.updated.saturating_add(1);
                    order.updated_rows = order.updated_rows.saturating_add(indices.len() as u64);
                    return TableInput::ComparedChanges {
                        token,
                        indices,
                        rows,
                    };
                }
            } else if source.len() == baseline.len()
                && baseline.iter().enumerate().all(|(index, previous)| {
                    let current = at(index);
                    key(current) == previous.key() && equal(current, previous)
                })
            {
                order.reused = order.reused.saturating_add(1);
                return TableInput::ComparedEqual(token);
            }
        }
        let mut rows = Vec::new();
        if engine_allocation::reserve_exact(&mut rows, source.len()).is_err() {
            return unavailable("collector allocation failed").into();
        }
        rows.extend((0..source.len()).map(|index| convert(at(index))));
        let rows = TableRows::from_rows::<S>(rows);
        order.converted = order.converted.saturating_add(1);
        match outcome {
            Outcome::Truncated { unread_rows, .. } => TableView::Truncated {
                rows,
                missing: unread_rows,
            },
            _ => TableView::Complete(rows),
        }
        .into()
    }
}

impl Projection<'_> {
    fn keyed_table<S: TableSchema, T>(
        &mut self,
        outcome: Outcome,
        source: &[T],
        key: impl Fn(&T) -> u128,
        equal: impl Fn(&T, &S::Row) -> bool,
        convert: impl Fn(&T) -> S::Row,
    ) -> TableInput {
        let shared = self
            .encoder
            .slice_baseline(S::ID, self.branch, self.hour, self.force)
            .is_some();
        if !shared
            && self
                .encoder
                .comparison_baseline::<S>(self.branch, self.hour, self.force)
                .is_some()
        {
            return self.table::<S, _>(outcome, source, key, equal, convert);
        }
        let Some(source) = source.get(..outcome.len()) else {
            return unavailable("walker row count exceeds allocated buffer").into();
        };
        match outcome {
            Outcome::Unavailable { reason } => return unavailable(reason).into(),
            Outcome::Truncated { unread_rows: 0, .. } => {
                return unavailable("walker marked truncation without a missing count").into();
            }
            _ => {}
        }
        let mut order = self.scratch.table(S::ID);
        let order = &mut *order;
        let natural = source.windows(2).all(|rows| key(&rows[0]) < key(&rows[1]));
        if !natural {
            if order.indices.len() != source.len() {
                order.indices.clear();
                if engine_allocation::reserve_exact(&mut order.indices, source.len()).is_err() {
                    return unavailable("collector allocation failed").into();
                }
                order.indices.extend(0..source.len());
            }
            if !order
                .indices
                .windows(2)
                .all(|pair| key(&source[pair[0]]) < key(&source[pair[1]]))
            {
                order
                    .indices
                    .sort_unstable_by_key(|index| key(&source[*index]));
                order.sorted = order.sorted.saturating_add(1);
            }
            if order
                .indices
                .windows(2)
                .any(|pair| key(&source[pair[0]]) == key(&source[pair[1]]))
            {
                return unavailable("walker produced duplicate engine ids").into();
            }
        }
        let at = |index: usize| &source[if natural { index } else { order.indices[index] }];
        if matches!(outcome, Outcome::Rows { .. })
            && !replay_data::cadence::is_staggered(S::ID)
            && let Some((token, previous)) =
                self.encoder
                    .slice_baseline(S::ID, self.branch, self.hour, self.force)
            && let Some(previous) = previous.rows::<S>()
        {
            order.compared = order.compared.saturating_add(1);
            let mut keys = Vec::new();
            let mut rows = Vec::new();
            let mut edit = |key: u128, row: Option<S::Row>| {
                if keys.len() == keys.capacity() {
                    engine_allocation::reserve(&mut keys, 1)?;
                }
                keys.push(key);
                if let Some(row) = row {
                    if rows.len() == rows.capacity() {
                        engine_allocation::reserve(&mut rows, 1)?;
                    }
                    rows.push(row);
                }
                Ok::<_, std::io::Error>(())
            };
            let (mut current, mut held) = (0, 0);
            let merged = loop {
                let row = (current < source.len()).then(|| at(current));
                let result = match (row, previous.get(held)) {
                    (None, None) => break Ok(()),
                    (Some(row), Some(baseline)) if key(row) == baseline.key() => {
                        current += 1;
                        held += 1;
                        if equal(row, baseline) {
                            continue;
                        }
                        edit(key(row), Some(convert(row)))
                    }
                    (Some(row), baseline) if baseline.is_none_or(|b| key(row) < b.key()) => {
                        current += 1;
                        edit(key(row), Some(convert(row)))
                    }
                    (_, Some(baseline)) => {
                        held += 1;
                        edit(baseline.key(), None)
                    }
                    (Some(_), None) => unreachable!("an absent baseline row takes the insert arm"),
                };
                if result.is_err() {
                    break result;
                }
            };
            if merged.is_err() {
                return unavailable("collector allocation failed").into();
            }
            if keys.is_empty() {
                order.reused = order.reused.saturating_add(1);
                return TableInput::ComparedEqual(token);
            }
            order.updated = order.updated.saturating_add(1);
            order.updated_rows = order.updated_rows.saturating_add(rows.len() as u64);
            return TableInput::SliceUpdate {
                token,
                update: std::sync::Arc::new(replay_data::sliced::SliceUpdate::keyed(
                    keys,
                    TableRows::from_rows::<S>(rows),
                )),
            };
        }
        let mut rows = Vec::new();
        if engine_allocation::reserve_exact(&mut rows, source.len()).is_err() {
            return unavailable("collector allocation failed").into();
        }
        rows.extend((0..source.len()).map(|index| convert(at(index))));
        let rows = TableRows::from_rows::<S>(rows);
        order.converted = order.converted.saturating_add(1);
        match outcome {
            Outcome::Truncated { unread_rows, .. } => TableView::Truncated {
                rows,
                missing: unread_rows,
            }
            .into(),
            _ => TableInput::SharedComplete(std::sync::Arc::new(rows)),
        }
    }
}

pub fn project_table<S: TableSchema, T>(
    projection: Option<&mut Projection<'_>>,
    outcome: Outcome,
    source: &[T],
    key: impl Fn(&T) -> u128,
    equal: impl Fn(&T, &S::Row) -> bool,
    convert: impl Fn(&T) -> S::Row,
) -> TableInput {
    let convert = |native: &T| displayed::<S>(convert(native));
    let Some(projection) = projection else {
        return table::<S, _>(outcome, source, convert).into();
    };
    crate::table_clock::timed(S::ID, || {
        if replay_data::sliced::takes_keyed_updates(S::ID) {
            projection.keyed_table::<S, _>(outcome, source, key, equal, convert)
        } else {
            projection.table::<S, _>(outcome, source, key, equal, convert)
        }
    })
}

pub struct Record<'a> {
    bytes: &'a mut Vec<u8>,
    failed: bool,
}

impl Record<'_> {
    #[inline]
    fn room(&mut self, len: usize) -> bool {
        if !self.failed && self.bytes.capacity() - self.bytes.len() < len {
            self.failed = self.bytes.try_reserve(len).is_err();
        }
        !self.failed
    }

    fn run(&mut self, len: Option<usize>, width: usize, each: impl FnOnce(&mut Vec<u8>)) {
        let Some(len) = len else {
            if self.room(1) {
                self.bytes.push(0);
            }
            return;
        };
        if self.room(9 + len * width) {
            self.bytes.push(1);
            self.bytes.extend_from_slice(&(len as u64).to_le_bytes());
            each(&mut *self.bytes);
        }
    }

    pub fn bytes(&mut self, data: Option<&[u8]>) {
        self.run(data.map(<[u8]>::len), 1, |out| out.extend_from_slice(data.unwrap_or_default()));
    }

    pub fn word(&mut self, value: i64) {
        if self.room(8) {
            self.bytes.extend_from_slice(&value.to_le_bytes());
        }
    }

    pub fn ids(&mut self, data: Option<&[u32]>) {
        self.run(data.map(<[u32]>::len), 4, |out| {
            let values = data.unwrap_or_default();
            #[cfg(target_endian = "little")]
            {
                // SAFETY: &[u32] is contiguous initialized memory with little-endian layout.
                let raw = unsafe {
                    std::slice::from_raw_parts(
                        values.as_ptr().cast::<u8>(),
                        std::mem::size_of_val(values),
                    )
                };
                out.extend_from_slice(raw);
            }
            #[cfg(not(target_endian = "little"))]
            for value in values {
                out.extend_from_slice(&value.to_le_bytes());
            }
        });
    }

    pub fn words(&mut self, data: Option<&[i64]>) {
        self.run(data.map(<[i64]>::len), 8, |out| {
            let values = data.unwrap_or_default();
            #[cfg(target_endian = "little")]
            {
                // SAFETY: &[i64] is contiguous initialized memory with little-endian layout.
                let raw = unsafe {
                    std::slice::from_raw_parts(
                        values.as_ptr().cast::<u8>(),
                        std::mem::size_of_val(values),
                    )
                };
                out.extend_from_slice(raw);
            }
            #[cfg(not(target_endian = "little"))]
            for value in values {
                out.extend_from_slice(&value.to_le_bytes());
            }
        });
    }
}

struct Ledger<C> {
    stamp: Option<(u64, BranchId, Hour)>,
    keys: Vec<u128>,
    fixed: Vec<Option<C>>,
    ends: Vec<usize>,
    bytes: Vec<u8>,
}

impl<C> Default for Ledger<C> {
    fn default() -> Self {
        Self {
            stamp: None,
            keys: Vec::new(),
            fixed: Vec::new(),
            ends: Vec::new(),
            bytes: Vec::new(),
        }
    }
}

impl<C> Ledger<C> {
    fn clear(&mut self) {
        self.stamp = None;
        self.keys.clear();
        self.fixed.clear();
        self.ends.clear();
        self.bytes.clear();
    }

    fn span(&self, at: usize) -> &[u8] {
        let start = at.checked_sub(1).map_or(0, |before| self.ends[before]);
        &self.bytes[start..self.ends[at]]
    }
}

struct Ledgers<C> {
    held: Ledger<C>,
    next: Ledger<C>,
    seen: Vec<bool>,
    edits: Vec<(u128, Option<usize>)>,
    index: std::collections::HashMap<u128, usize, foldhash::fast::FixedState>,
}

impl<C> Default for Ledgers<C> {
    fn default() -> Self {
        Self {
            held: Ledger::default(),
            next: Ledger::default(),
            seen: Vec::new(),
            edits: Vec::new(),
            index: std::collections::HashMap::with_hasher(foldhash::fast::FixedState::default()),
        }
    }
}

impl Projection<'_> {
    fn ledger_table<S: TableSchema, T, C: Copy + PartialEq + Send + 'static>(
        &mut self,
        outcome: Outcome,
        source: &[T],
        key: impl Fn(&T) -> u128,
        record: impl Fn(usize, &T, &mut Record<'_>) -> Option<C>,
        convert: impl Fn(&T) -> S::Row,
    ) -> TableInput {
        let held: Option<Box<Ledgers<C>>> = self
            .scratch
            .table(S::ID)
            .ledger
            .take()
            .and_then(|ledger| ledger.downcast().ok());
        let encoder = self.encoder;
        let shared = encoder.slice_meta(S::ID, self.branch, self.hour, self.force);
        let complete = matches!(outcome, Outcome::Rows { .. })
            && !replay_data::cadence::is_staggered(S::ID)
            && source.len() >= outcome.len();
        if !complete
            || (shared.is_none()
                && encoder
                    .comparison_baseline::<S>(self.branch, self.hour, self.force)
                    .is_some())
        {
            return self.keyed_table::<S, _>(
                outcome,
                source,
                &key,
                |row, previous| convert(row) == *previous,
                &convert,
            );
        }
        let source = &source[..outcome.len()];
        let mut ledgers = match held {
            Some(ledgers) => ledgers,
            None => match engine_allocation::boxed(Ledgers::default()) {
                Ok(ledgers) => ledgers,
                Err(_) => return unavailable("collector allocation failed").into(),
            },
        };
        let aligned_eligible = matches!(
            shared,
            Some((token, previous_len))
                if ledgers.held.stamp == Some(token.baseline())
                    && previous_len.is_none_or(|len| len == ledgers.held.keys.len())
        );
        let Ledgers {
            held,
            next,
            seen,
            edits,
            index,
        } = &mut *ledgers;
        next.clear();
        let bytes_need = held.bytes.len();
        let reserved = engine_allocation::reserve(&mut next.keys, source.len())
            .and_then(|_| engine_allocation::reserve(&mut next.fixed, source.len()))
            .and_then(|_| engine_allocation::reserve(&mut next.ends, source.len()))
            .and_then(|_| {
                if next.bytes.capacity() < bytes_need {
                    engine_allocation::reserve(&mut next.bytes, bytes_need)
                } else {
                    Ok(())
                }
            });
        if reserved.is_err() {
            return unavailable("collector allocation failed").into();
        }
        let mut aligned = aligned_eligible;
        let mut first_unaligned = 0;
        if aligned {
            edits.clear();
        }
        for (at, row) in source.iter().enumerate() {
            let start = next.bytes.len();
            let mut writer = Record {
                bytes: &mut next.bytes,
                failed: false,
            };
            let fixed = record(at, row, &mut writer);
            if writer.failed {
                return unavailable("collector allocation failed").into();
            }
            let end = next.bytes.len();
            let row_key = key(row);
            if aligned {
                if held.keys.get(at) == Some(&row_key) {
                    if !(fixed.is_some()
                        && fixed == held.fixed[at]
                        && &next.bytes[start..end] == held.span(at))
                    {
                        if edits.len() == edits.capacity()
                            && engine_allocation::reserve(edits, 1).is_err()
                        {
                            return unavailable("collector allocation failed").into();
                        }
                        edits.push((row_key, Some(at)));
                    }
                } else {
                    aligned = false;
                    first_unaligned = at;
                }
            }
            next.keys.push(row_key);
            next.fixed.push(fixed);
            next.ends.push(end);
        }
        let input = match shared {
            Some((token, previous_len))
                if held.stamp == Some(token.baseline())
                    && previous_len.is_none_or(|len| len == held.keys.len()) =>
            {
                let result = if aligned {
                    if source.len() < held.keys.len() {
                        let tail = held.keys.len() - source.len();
                        if engine_allocation::reserve(edits, tail).is_err() {
                            return unavailable("collector allocation failed").into();
                        }
                        for &removed in &held.keys[source.len()..] {
                            edits.push((removed, None));
                        }
                    }
                    finalize_ledger_edits::<S, T>(edits, source, &convert)
                } else {
                    ledger_update::<S, T, C>(
                        held,
                        next,
                        seen,
                        edits,
                        index,
                        first_unaligned,
                        source,
                        &convert,
                    )
                };
                match result {
                    Ok(None) => TableInput::ComparedEqual(token),
                    Ok(Some(update)) => TableInput::SliceUpdate {
                        token,
                        update: std::sync::Arc::new(update),
                    },
                    Err(reason) => return unavailable(reason).into(),
                }
            }
            _ => self.keyed_table::<S, _>(
                outcome,
                source,
                &key,
                |row, previous| convert(row) == *previous,
                &convert,
            ),
        };
        if matches!(
            input,
            TableInput::ComparedEqual(_)
                | TableInput::SliceUpdate { .. }
                | TableInput::SharedComplete(_)
        ) {
            let Ledgers {
                held,
                next,
                seen,
                edits,
                index,
            } = &mut *ledgers;
            std::mem::swap(held, next);
            held.stamp = Some((encoder.identity(), self.branch, self.hour));
            if next.keys.capacity() * size_of::<u128>()
                + next.fixed.capacity() * size_of::<Option<C>>()
                + next.ends.capacity() * size_of::<usize>()
                + next.bytes.capacity()
                > 65_536
            {
                *next = Ledger::default();
            }
            if seen.capacity() > 4096 {
                *seen = Vec::new();
            }
            if edits.capacity() > 1024 {
                *edits = Vec::new();
            }
            if index.capacity() > 1024 {
                *index = std::collections::HashMap::with_hasher(foldhash::fast::FixedState::default());
            }
            let mut order = self.scratch.table(S::ID);
            match &input {
                TableInput::ComparedEqual(_) => {
                    order.compared = order.compared.saturating_add(1);
                    order.reused = order.reused.saturating_add(1);
                }
                TableInput::SliceUpdate { update, .. } => {
                    order.compared = order.compared.saturating_add(1);
                    order.updated = order.updated.saturating_add(1);
                    order.updated_rows = order
                        .updated_rows
                        .saturating_add(update.rows().len() as u64);
                }
                _ => {}
            }
            order.ledger = Some(ledgers);
        }
        input
    }
}

#[allow(clippy::too_many_arguments, reason = "internal ledger diffing helper")]
fn ledger_update<S: TableSchema, T, C: Copy + PartialEq>(
    held: &Ledger<C>,
    next: &Ledger<C>,
    seen: &mut Vec<bool>,
    edits: &mut Vec<(u128, Option<usize>)>,
    index: &mut std::collections::HashMap<u128, usize, foldhash::fast::FixedState>,
    first_unaligned: usize,
    source: &[T],
    convert: impl Fn(&T) -> S::Row,
) -> Result<Option<replay_data::sliced::SliceUpdate>, &'static str> {
    const DUPLICATE: &str = "walker produced duplicate engine ids";
    const ALLOCATION: &str = "collector allocation failed";
    seen.clear();
    let need = held.keys.len().saturating_sub(seen.capacity());
    if need > 0 {
        engine_allocation::reserve(seen, need).map_err(|_| ALLOCATION)?;
    }
    seen.resize(held.keys.len(), false);
    seen[..first_unaligned].fill(true);
    let held_sorted = held.keys.windows(2).all(|pair| pair[0] < pair[1]);
    let mut shift: isize = 0;
    let mut index_ready = false;
    let mut added: std::collections::HashSet<u128, foldhash::fast::FixedState> =
        std::collections::HashSet::default();
    let mut matched = first_unaligned;
    for (offset, &key) in next.keys[first_unaligned..].iter().enumerate() {
        let at = first_unaligned + offset;
        let base = at as isize + shift;
        let probed = usize::try_from(base)
            .ok()
            .filter(|&cand| held.keys.get(cand) == Some(&key))
            .or_else(|| {
                for delta in [1_isize, -1, 2, -2, 3, -3, 4, -4] {
                    if let Ok(cand) = usize::try_from(base + delta)
                        && held.keys.get(cand) == Some(&key)
                    {
                        shift = cand as isize - at as isize;
                        return Some(cand);
                    }
                }
                None
            });
        let old = if probed.is_some() {
            probed
        } else if held_sorted {
            let found = held.keys.binary_search(&key).ok();
            if let Some(cand) = found {
                shift = cand as isize - at as isize;
            }
            found
        } else {
            if !index_ready {
                index.clear();
                let need = held.keys.len().saturating_sub(index.capacity());
                if need > 0 {
                    engine_allocation::reserve_map(index, need).map_err(|_| ALLOCATION)?;
                }
                index.extend(held.keys.iter().enumerate().map(|(at, &key)| (key, at)));
                index_ready = true;
            }
            let found = index.get(&key).copied();
            if let Some(cand) = found {
                shift = cand as isize - at as isize;
            }
            found
        };
        match old {
            Some(old) => {
                if std::mem::replace(&mut seen[old], true) {
                    return Err(DUPLICATE);
                }
                matched += 1;
                if next.fixed[at].is_some()
                    && next.fixed[at] == held.fixed[old]
                    && next.span(at) == held.span(old)
                {
                    continue;
                }
            }
            None => {
                engine_allocation::reserve_set(&mut added, 1).map_err(|_| ALLOCATION)?;
                if !added.insert(key) {
                    return Err(DUPLICATE);
                }
            }
        }
        if edits.len() == edits.capacity() {
            engine_allocation::reserve(edits, 1).map_err(|_| ALLOCATION)?;
        }
        edits.push((key, Some(at)));
    }
    if matched < held.keys.len() {
        for (old, _) in seen.iter().enumerate().filter(|(_, was_seen)| !**was_seen) {
            if edits.len() == edits.capacity() {
                engine_allocation::reserve(edits, 1).map_err(|_| ALLOCATION)?;
            }
            edits.push((held.keys[old], None));
        }
    }
    finalize_ledger_edits::<S, T>(edits, source, convert)
}

fn finalize_ledger_edits<S: TableSchema, T>(
    edits: &mut [(u128, Option<usize>)],
    source: &[T],
    convert: impl Fn(&T) -> S::Row,
) -> Result<Option<replay_data::sliced::SliceUpdate>, &'static str> {
    const ALLOCATION: &str = "collector allocation failed";
    if edits.is_empty() {
        return Ok(None);
    }
    if !edits.windows(2).all(|pair| pair[0].0 < pair[1].0) {
        edits.sort_unstable_by_key(|edit| edit.0);
    }
    let mut keys = Vec::new();
    let mut rows = Vec::new();
    engine_allocation::reserve(&mut keys, edits.len()).map_err(|_| ALLOCATION)?;
    engine_allocation::reserve(&mut rows, edits.len()).map_err(|_| ALLOCATION)?;
    for &(key, at) in edits.iter() {
        keys.push(key);
        if let Some(at) = at {
            rows.push(convert(&source[at]));
        }
    }
    Ok(Some(replay_data::sliced::SliceUpdate::keyed(
        keys,
        TableRows::from_rows::<S>(rows),
    )))
}

pub fn ledger_table<S: TableSchema, T, C: Copy + PartialEq + Send + 'static>(
    projection: Option<&mut Projection<'_>>,
    outcome: Outcome,
    source: &[T],
    key: impl Fn(&T) -> u128,
    record: impl Fn(usize, &T, &mut Record<'_>) -> Option<C>,
    convert: impl Fn(&T) -> S::Row,
) -> TableInput {
    debug_assert!(replay_data::sliced::takes_keyed_updates(S::ID));
    let convert = |native: &T| displayed::<S>(convert(native));
    let Some(projection) = projection else {
        return table::<S, _>(outcome, source, convert).into();
    };
    crate::table_clock::timed(S::ID, || {
        projection.ledger_table::<S, T, C>(outcome, source, key, record, convert)
    })
}

pub fn held_input<'p, S: TableSchema>(
    projection: Option<&mut Projection<'p>>,
    witness: Option<Witness>,
    convert: impl FnOnce(Option<&mut Projection<'p>>) -> TableInput,
) -> TableInput {
    crate::table_clock::timed(S::ID, || held::<S>(projection, witness, convert))
}

fn held<'p, S: TableSchema>(
    projection: Option<&mut Projection<'p>>,
    witness: Option<Witness>,
    convert: impl FnOnce(Option<&mut Projection<'p>>) -> TableInput,
) -> TableInput {
    match projection {
        Some(projection) => {
            if let Some(input) = projection.republish::<S>(witness) {
                return input;
            }
            let input = convert(Some(&mut *projection));
            projection.retain(S::ID, witness, &input);
            input
        }
        None => convert(None),
    }
}

pub fn scalar_table<S: TableSchema, T>(
    projection: Option<&mut Projection<'_>>,
    outcome: Outcome,
    source: &[T],
    key: impl Fn(&T) -> u128,
    convert: impl Fn(&T) -> S::Row,
) -> TableInput {
    project_table::<S, _>(
        projection,
        outcome,
        source,
        key,
        |native, previous| displayed::<S>(convert(native)) == *previous,
        &convert,
    )
}

fn displayed<S: TableSchema>(mut row: S::Row) -> S::Row {
    row.round_to_display();
    row
}

#[cfg(test)]
#[path = "projection/market_tests.rs"]
mod market_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::{rows::StateRow, rows::TableId, rows::tables};
    use replay_frame::{
        frame::DecodedFrame, frame::FrameDecoder, frame::FrameInput, frame::Metrics,
    };
    use std::cell::Cell;

    fn input(hour: Hour, table: TableInput) -> FrameInput {
        let mut frame = FrameInput {
            branch: 1,
            hour,
            tables: std::array::from_fn(|_| unavailable("fixture table").into()),
            metrics: Metrics::default(),
        };
        frame.tables[TableId::States.index()] = table;
        frame
    }

    fn project(
        encoder: &FrameEncoder,
        scratch: &mut ProjectionScratch,
        hour: Hour,
        outcome: Outcome,
        rows: &[StateRow],
        conversions: &Cell<usize>,
    ) -> TableInput {
        project_table::<tables::States, _>(
            Some(&mut Projection::new(encoder, 1, hour, false, scratch)),
            outcome,
            rows,
            |row| u128::from(row.id),
            |row, previous| row == previous,
            |row| {
                conversions.set(conversions.get() + 1);
                row.clone()
            },
        )
    }

    #[test]
    fn certified_market_owners_require_a_baseline_and_keep_unmarked_rows() {
        use replay_data::rows::{CountryMarketRow, LendLeaseAgreementRow, MarketContractTermsRow};
        let families = [
            TableRows::CountryMarket(vec![
                CountryMarketRow {
                    country: 1,
                    ..Default::default()
                },
                CountryMarketRow {
                    country: 5000,
                    ..Default::default()
                },
            ]),
            TableRows::MarketContractTerms(vec![
                MarketContractTermsRow {
                    country: 1,
                    id: 10,
                    ..Default::default()
                },
                MarketContractTermsRow {
                    country: 5000,
                    id: 20,
                    ..Default::default()
                },
            ]),
            TableRows::LendLeaseAgreements(vec![
                LendLeaseAgreementRow {
                    country: 1,
                    id: 10,
                    ..Default::default()
                },
                LendLeaseAgreementRow {
                    country: 5000,
                    id: 20,
                    ..Default::default()
                },
            ]),
        ];
        for previous in families {
            let table = previous.table();
            let mut encoder = FrameEncoder::new();
            let scratch = ProjectionScratch::default();
            let missing = Projection::new(&encoder, 1, 0, false, &scratch).certified_owners(
                TableRows::empty(table),
                vec![5000],
                false,
            );
            assert!(matches!(
                missing,
                TableInput::Sampled(TableView::Unavailable { .. })
            ));
            let full = Projection::new(&encoder, 1, 0, false, &scratch).certified_owners(
                previous.clone(),
                vec![1, 5000],
                true,
            );
            let mut frame = input(0, unavailable("fixture table").into());
            frame.tables[table.index()] = full;
            let mut decoder = FrameDecoder::new();
            decoder
                .apply(&encoder.encode_input(frame, false).unwrap())
                .unwrap();
            let removed = Projection::new(&encoder, 1, 1, false, &scratch).certified_owners(
                TableRows::empty(table),
                vec![5000],
                false,
            );
            assert!(matches!(removed, TableInput::SliceUpdate { .. }));
            let mut frame = input(1, unavailable("fixture table").into());
            frame.tables[table.index()] = removed;
            let decoded = decoder
                .apply(&encoder.encode_input(frame, false).unwrap())
                .unwrap();
            let expected = replay_data::sliced::merge_slice(
                &previous,
                TableRows::empty(table),
                |owner| owner == 5000,
                |_| true,
            )
            .unwrap();
            assert_eq!(
                &*decoded.tables[table.index()],
                &TableView::Complete(expected)
            );
        }
    }

    #[test]
    fn current_native_rows_reuse_without_materialization_and_keep_exact_delta_behavior() {
        let mut projected = FrameEncoder::new();
        let mut ordinary = FrameEncoder::new();
        let mut projected_decoder = FrameDecoder::new();
        let mut ordinary_decoder = FrameDecoder::new();
        let mut scratch = ProjectionScratch::default();
        let conversions = Cell::new(0);
        let state = |id, owner| StateRow {
            id,
            owner,
            controller: owner + 1,
            movement_infrastructure: 0,
            ..Default::default()
        };
        let mut rows = vec![state(3, 30), state(1, 10), state(2, 20)];
        for hour in 0..8 {
            match hour {
                1 | 2 => rows.rotate_left(1),
                3 => rows.sort_unstable_by_key(|row| row.id),
                4 => rows[1].owner += 100,
                5 => {
                    rows.remove(0);
                    rows.push(state(4, 40));
                }
                6 => rows.reverse(),
                _ => {}
            }
            let outcome = Outcome::Rows { len: rows.len() };
            let before = conversions.get();
            let current = project(&projected, &mut scratch, hour, outcome, &rows, &conversions);
            let reused = matches!(hour, 1 | 2 | 3 | 6 | 7);
            assert_eq!(matches!(current, TableInput::ComparedEqual(_)), reused);
            assert_eq!(
                conversions.get() - before,
                if reused {
                    0
                } else if hour == 4 {
                    1
                } else {
                    rows.len()
                }
            );
            let baseline = table::<tables::States, _>(outcome, &rows, Clone::clone);
            let actual = projected.encode_input(input(hour, current), false).unwrap();
            let expected = ordinary
                .encode_input(input(hour, baseline.into()), false)
                .unwrap();
            assert_eq!(actual, expected);
            let actual: DecodedFrame = projected_decoder.apply(&actual).unwrap();
            assert_eq!(actual, ordinary_decoder.apply(&expected).unwrap());
        }
    }

    #[test]
    fn duplicate_partial_and_failed_sources_never_claim_an_equal_complete_table() {
        let mut encoder = FrameEncoder::new();
        let mut scratch = ProjectionScratch::default();
        let conversions = Cell::new(0);
        let rows = [
            StateRow {
                id: 3,
                owner: 7,
                controller: 9,
                movement_infrastructure: 0,
                ..Default::default()
            },
            StateRow {
                id: 1,
                owner: 2,
                controller: 4,
                movement_infrastructure: 0,
                ..Default::default()
            },
        ];
        let complete = Outcome::Rows { len: 2 };
        let first = project(&encoder, &mut scratch, 0, complete, &rows, &conversions);
        encoder.encode_input(input(0, first), false).unwrap();
        for (outcome, source) in [
            (complete, vec![rows[0].clone(), rows[0].clone()]),
            (Outcome::Rows { len: 3 }, rows.to_vec()),
            (
                Outcome::Unavailable {
                    reason: "fresh fault",
                },
                rows.to_vec(),
            ),
            (
                Outcome::Truncated {
                    len: 2,
                    unread_rows: 0,
                },
                rows.to_vec(),
            ),
        ] {
            let rejected = project(&encoder, &mut scratch, 1, outcome, &source, &conversions);
            assert!(matches!(
                rejected,
                TableInput::Sampled(TableView::Unavailable { .. })
            ));
        }
        let partial = project(
            &encoder,
            &mut scratch,
            1,
            Outcome::Truncated {
                len: 1,
                unread_rows: 1,
            },
            &rows,
            &conversions,
        );
        assert!(matches!(
            partial,
            TableInput::Sampled(TableView::Truncated { missing: 1, .. })
        ));
        encoder.encode_input(input(1, partial), false).unwrap();
        assert!(matches!(
            project(&encoder, &mut scratch, 2, complete, &rows, &conversions),
            TableInput::Sampled(TableView::Complete(_))
        ));
    }

    #[test]
    fn nonconsecutive_branch_and_recovery_hours_require_current_owned_rows() {
        let mut encoder = FrameEncoder::new();
        let mut scratch = ProjectionScratch::default();
        let conversions = Cell::new(0);
        let rows = [StateRow {
            id: 1,
            owner: 2,
            controller: 4,
            movement_infrastructure: 0,
            ..Default::default()
        }];
        let outcome = Outcome::Rows { len: 1 };
        let first = project(&encoder, &mut scratch, 0, outcome, &rows, &conversions);
        encoder.encode_input(input(0, first), false).unwrap();
        for (branch, hour, force) in [(1, 2, false), (2, 1, false), (1, 1, true)] {
            let current = scalar_table::<tables::States, _>(
                Some(&mut Projection::new(
                    &encoder, branch, hour, force, &scratch,
                )),
                outcome,
                &rows,
                |row| u128::from(row.id),
                Clone::clone,
            );
            assert!(matches!(
                current,
                TableInput::Sampled(TableView::Complete(_))
            ));
        }
    }
    #[test]
    fn observed_variant_baseline_survives_keyframes_and_refuses_branch_or_hour_gaps() {
        let mut encoder = FrameEncoder::new();
        let scratch = ProjectionScratch::default();
        let mut first = input(0, unavailable("fixture table").into());
        first.tables[TableId::VariantDefinitions.index()] =
            TableView::Complete(TableRows::VariantDefinitions(vec![
                replay_data::rows::VariantDefinitionRow {
                    id: 70,
                    ..Default::default()
                },
            ]))
            .into();
        encoder.encode_input(first, false).unwrap();
        for hour in 1..=replay_frame::KEYFRAME_INTERVAL as i32 + 1 {
            let projection = Projection::new(&encoder, 1, hour, true, &scratch);
            let rows = projection
                .observed_baseline::<tables::VariantDefinitions>()
                .unwrap();
            assert_eq!(rows[0].id, 70);
            assert!(
                Projection::new(&encoder, 2, hour, false, &scratch)
                    .observed_baseline::<tables::VariantDefinitions>()
                    .is_none()
            );
            assert!(
                Projection::new(&encoder, 1, hour + 1, false, &scratch)
                    .observed_baseline::<tables::VariantDefinitions>()
                    .is_none()
            );
            let mut next = input(hour, unavailable("fixture table").into());
            next.tables[TableId::VariantDefinitions.index()] =
                TableView::Complete(TableRows::VariantDefinitions(rows.to_vec())).into();
            encoder.encode_input(next, false).unwrap();
        }
    }

    #[test]
    fn a_witnessed_hour_compares_equal_unprojected_and_any_other_hour_converts() {
        let mut held_encoder = FrameEncoder::new();
        let mut ordinary = FrameEncoder::new();
        let mut held_decoder = FrameDecoder::new();
        let mut ordinary_decoder = FrameDecoder::new();
        let scratch = ProjectionScratch::default();
        let projections = Cell::new(0);
        let rows = vec![StateRow {
            id: 1,
            owner: 2,
            controller: 3,
            movement_infrastructure: 0,
            ..Default::default()
        }];
        let frame = |branch, hour, table: TableInput| {
            let mut frame = input(hour, unavailable("fixture table").into());
            frame.branch = branch;
            frame.tables[TableId::States.index()] = table;
            frame
        };
        let capture = |encoder: &FrameEncoder, branch, hour, witness| {
            let before = projections.get();
            let input = held_input::<tables::States>(
                Some(&mut Projection::new(encoder, branch, hour, false, &scratch)),
                witness,
                |projection| {
                    projections.set(projections.get() + 1);
                    scalar_table::<tables::States, _>(
                        projection,
                        Outcome::Rows { len: rows.len() },
                        &rows,
                        |row| u128::from(row.id),
                        Clone::clone,
                    )
                },
            );
            (input, projections.get() != before)
        };
        let quiet = Some(Witness::new([7, 11]));
        let moved = Some(Witness::new([7, 12]));
        let hours = [
            (1, 0, quiet, Some(true), false),
            (1, 1, quiet, Some(true), true),
            (1, 2, quiet, Some(true), true),
            (1, 3, None, Some(true), false),
            (1, 4, quiet, Some(true), false),
            (1, 5, moved, Some(true), false),
            (1, 6, moved, Some(true), true),
            (1, 8, moved, Some(true), false),
            (2, 9, moved, Some(true), false),
            (2, 10, moved, Some(true), true),
            (2, 11, moved, None, true),
            (2, 12, moved, Some(true), false),
            (2, 13, moved, Some(false), true),
            (2, 14, moved, Some(true), false),
            (2, 15, moved, Some(true), true),
        ];
        for (branch, hour, witness, committed, republished) in hours {
            let (current, projected) = capture(&held_encoder, branch, hour, witness);
            assert_eq!(projected, !republished, "hour {hour}");
            if republished {
                assert!(
                    matches!(current, TableInput::ComparedEqual(_)),
                    "hour {hour}"
                );
            }
            let (held, reference): (TableInput, TableInput) = match committed {
                None => continue,
                Some(true) => (
                    current,
                    TableView::Complete(TableRows::States(rows.clone())).into(),
                ),
                Some(false) => (
                    unavailable("fixture refusal").into(),
                    unavailable("fixture refusal").into(),
                ),
            };
            let actual = held_encoder
                .encode_input(frame(branch, hour, held), false)
                .unwrap();
            let expected = ordinary
                .encode_input(frame(branch, hour, reference), false)
                .unwrap();
            assert_eq!(actual, expected, "hour {hour}");
            let actual: DecodedFrame = held_decoder.apply(&actual).unwrap();
            assert_eq!(actual, ordinary_decoder.apply(&expected).unwrap());
        }
        assert_eq!(scratch.republish_counts(TableId::States), [7, 8]);
        scratch.forget(TableId::States);
        let (current, projected) = capture(&held_encoder, 2, 16, moved);
        assert!(projected && matches!(current, TableInput::ComparedEqual(_)));
        assert_eq!(scratch.republish_counts(TableId::States), [7, 9]);
    }
}
