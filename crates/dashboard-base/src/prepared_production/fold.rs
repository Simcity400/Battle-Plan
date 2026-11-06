//! The line output folded into each producer's totals at the output's reads: rows kept in
//! buckets by their line and totals in buckets by their producer, the buckets' work shared
//! with the run's idle workers ([`super::shards`]). The table's changes go into the rows as
//! they come, each row they change listed once for the next read; a read visits only the rows
//! listed, those whose producer is untold, and those gone, in row order, as a read of every
//! row would change totals.

use super::shards;
use crate::products::equipment::production::{
    CostTotal, Entry, Identity, ProductionSpendingUnknown, Purpose, Total,
};
use replay_data::rows::Recorded;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    hash::{BuildHasher, Hasher},
    io,
};

/// Hashes the fold's integer keys: each word mixed in with one multiply, the high bits folded
/// down.
#[derive(Clone, Copy, Default)]
struct Words;

#[derive(Default)]
struct Mixed(u64);

impl BuildHasher for Words {
    type Hasher = Mixed;
    fn build_hasher(&self) -> Mixed {
        Mixed(0)
    }
}

impl Hasher for Mixed {
    fn finish(&self) -> u64 {
        self.0 ^ (self.0 >> 32)
    }

    fn write(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.write_u64(u64::from(byte));
        }
    }

    fn write_u32(&mut self, value: u32) {
        self.write_u64(u64::from(value));
    }

    fn write_u64(&mut self, value: u64) {
        self.0 = (self.0.rotate_left(26) ^ value).wrapping_mul(0xff51_afd7_ed55_8ccd);
    }
}

type Map<K, V> = HashMap<K, V, Words>;
type Set<T> = HashSet<T, Words>;

/// How many buckets rows and totals are kept in: fixed, so neither items nor saved state
/// depend on the machine.
pub(crate) const BUCKETS: usize = 64;

/// A row's line and ordinal.
pub(crate) type RowKey = (u64, u32);

/// The bucket a line's rows are kept in.
pub(crate) fn line_bucket(line: u64) -> usize {
    (line.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 58) as usize
}

/// The bucket a producer's totals are kept in.
fn identity_bucket(identity: Identity) -> usize {
    let key = (u64::from(identity.0) << 32 | u64::from(identity.1))
        ^ u64::from(identity.2).rotate_left(29);
    (key.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 58) as usize
}

/// A line output row as the fold reads it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, replay_spool::Spooled)]
pub(crate) struct OutputRow {
    pub line: u64,
    pub ordinal: u32,
    pub country: u32,
    pub variant: u64,
    pub units: i64,
    pub spent: Recorded<Option<i64>>,
    pub purpose: Purpose,
    pub exact: bool,
    pub ended: bool,
}

impl OutputRow {
    fn key(&self) -> RowKey {
        (self.line, self.ordinal)
    }
}

/// What the line output changed since the changes before: every live row where `whole`.
#[derive(replay_spool::Spooled)]
pub(crate) struct OutputChanges {
    pub whole: bool,
    /// The rows written or inserted, by bucket and key.
    rows: Vec<OutputRow>,
    /// The rows deleted, by bucket and key.
    deleted: Vec<RowKey>,
    /// The earliest observation start of the rows live after the changes.
    pub since: Option<i32>,
    /// Some row live after the changes records its spending.
    pub recorded: bool,
}

impl OutputChanges {
    pub(crate) fn new(
        whole: bool,
        mut rows: Vec<OutputRow>,
        mut deleted: Vec<RowKey>,
        since: Option<i32>,
        recorded: bool,
    ) -> Self {
        rows.sort_unstable_by_key(|row| (line_bucket(row.line), row.line, row.ordinal));
        deleted.sort_unstable_by_key(|&(line, ordinal)| (line_bucket(line), line, ordinal));
        Self {
            whole,
            rows,
            deleted,
            since,
            recorded,
        }
    }

    /// Whether they change nothing.
    pub(crate) fn is_empty(&self) -> bool {
        !self.whole && self.rows.is_empty() && self.deleted.is_empty()
    }

    /// Bucket `bucket`'s rows and deleted keys.
    fn bucket(&self, bucket: usize) -> (&[OutputRow], &[RowKey]) {
        let rows = |row: &OutputRow| line_bucket(row.line);
        let keys = |key: &RowKey| line_bucket(key.0);
        (
            &self.rows[self.rows.partition_point(|row| rows(row) < bucket)
                ..self.rows.partition_point(|row| rows(row) <= bucket)],
            &self.deleted[self.deleted.partition_point(|key| keys(key) < bucket)
                ..self.deleted.partition_point(|key| keys(key) <= bucket)],
        )
    }
}

/// A row as the fold last read it.
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub(crate) struct RowState {
    identity: Option<Identity>,
    entry: Entry,
    /// Its purpose was recorded.
    proved: bool,
    units: i64,
    exact: bool,
    ended: bool,
    spent_raw: i64,
}

/// One row: as the table holds it, and as the fold last read it.
#[derive(Default)]
struct Row {
    /// None while the table does not hold it.
    table: Option<OutputRow>,
    /// The table changed it since the fold last read it.
    dirty: bool,
    state: Option<RowState>,
    /// Expected at the next read: read and not ended.
    live: bool,
    /// In its bucket's list for the next read.
    listed: bool,
}

#[derive(Default)]
struct RowBucket {
    rows: Map<RowKey, Row>,
    /// Lines with output whose purpose was not recorded.
    unlisted: Set<u64>,
    /// The rows the next read may visit or find gone: those the table changed since the last
    /// read, and those whose producer it could not tell.
    listed: Vec<RowKey>,
}

/// Lists `row`, at `key`, for the next read.
fn list(listed: &mut Vec<RowKey>, key: RowKey, row: &mut Row) {
    if !row.listed {
        row.listed = true;
        listed.push(key);
    }
}

impl RowBucket {
    /// Takes `changes` of bucket `index` into the rows.
    fn apply(&mut self, changes: &OutputChanges, index: usize) {
        let Self {
            rows: held, listed, ..
        } = self;
        let (rows, deleted) = changes.bucket(index);
        // A row gone that no read took is forgotten; one read and live is found gone.
        let mut gone = |key: RowKey, row: &mut Row| {
            row.table = None;
            if row.live {
                list(listed, key, row);
            }
            row.state.is_some()
        };
        if changes.whole {
            held.retain(|key, row| {
                row.table.is_none()
                    || rows.binary_search_by_key(key, OutputRow::key).is_ok()
                    || gone(*key, row)
            });
        }
        for key in deleted {
            if let Some(row) = held.get_mut(key)
                && !gone(*key, row)
            {
                held.remove(key);
            }
        }
        for new in rows {
            let row = held.entry(new.key()).or_default();
            if row.table != Some(*new) {
                row.table = Some(*new);
                row.dirty = true;
                list(listed, new.key(), row);
            }
        }
    }
}

/// One producer's totals of a measure, versioned at each change, with the entries the latest
/// read changed.
#[derive(Default)]
pub(crate) struct Versioned<V> {
    pub version: u64,
    pub entries: BTreeMap<Entry, V>,
    /// The read that last changed them, their version before it, and the entries it changed.
    noted: (u64, u64, Vec<Entry>),
}

impl<V> Versioned<V> {
    fn note(&mut self, read: u64, entry: Entry) {
        if self.noted.0 != read {
            self.noted.0 = read;
            self.noted.1 = self.version;
            self.noted.2.clear();
        }
        self.noted.2.push(entry);
        self.version += 1;
    }

    /// The entries read `read` changed, where they held version `before` until it.
    pub(crate) fn since(&self, read: u64, before: u64) -> Option<&[Entry]> {
        (self.noted.0 == read && self.noted.1 == before).then_some(&self.noted.2[..])
    }
}

/// A producer's totals since its observation began.
#[derive(Default)]
pub(crate) struct Totals {
    pub units: Versioned<Total>,
    pub costs: Versioned<CostTotal>,
    /// A row of its produced, or a line of its was counted.
    pub seen: bool,
}

/// What a row changes of its producer's totals at a read.
#[derive(Clone, Copy)]
enum Change {
    Seen,
    Cost {
        delta: i128,
        reason: Option<ProductionSpendingUnknown>,
    },
    Units {
        from: i64,
        to: i64,
        exact_from: bool,
        exact_to: bool,
    },
}

#[derive(Clone, Copy)]
struct Contribution {
    bucket: u8,
    /// Rows gone without their ended publication come after those read.
    lost: bool,
    key: RowKey,
    identity: Identity,
    entry: Entry,
    proved: bool,
    change: Change,
}

/// What a bucket's rows gave at a read.
#[derive(Default)]
struct BucketRead {
    contributions: Vec<Contribution>,
    unattributed: usize,
    inexact: usize,
    lost: usize,
    tainted: usize,
    /// The first row whose spending a read cannot tell, and why.
    spending: Option<(RowKey, ProductionSpendingUnknown)>,
    presence_unknown: bool,
    visits: usize,
}

/// One read of the output.
pub(crate) struct Read<'a> {
    pub observation: u32,
    /// The fold's number for the read, which totals note their changes by.
    pub number: u64,
    /// A new observation began: every row is read anew and every total starts again.
    pub restart: bool,
    /// Each raw country id's producer: its canonical slot and id.
    pub producers: &'a HashMap<u32, (u32, u32)>,
    /// The producers the lines census counted.
    pub census: &'a [Identity],
}

/// What a read gave beside the totals.
#[derive(Default)]
pub(crate) struct Folded {
    pub unattributed: usize,
    pub inexact: usize,
    pub lost: usize,
    /// Rows holding units whose producer cannot be told: each taints the totals.
    pub tainted: usize,
    /// Why spending is unknown, from the first row in row order whose spending cannot be told.
    pub spending: Option<ProductionSpendingUnknown>,
    pub presence_unknown: bool,
    pub visits: usize,
}

/// The rows, as the table holds them and as the last read read them, and the totals.
pub(crate) struct Fold {
    rows: Vec<RowBucket>,
    totals: Vec<Map<Identity, Totals>>,
    since: Option<i32>,
    recorded: bool,
}

impl Default for Fold {
    fn default() -> Self {
        Self {
            rows: (0..BUCKETS).map(|_| RowBucket::default()).collect(),
            totals: (0..BUCKETS).map(|_| Map::default()).collect(),
            since: None,
            recorded: false,
        }
    }
}

impl Fold {
    /// Takes the table's changes into the rows, for the next read.
    pub(crate) fn push(&mut self, changes: OutputChanges) -> io::Result<()> {
        self.since = changes.since;
        self.recorded = changes.recorded;
        if changes.is_empty() {
            return Ok(());
        }
        let changes = &changes;
        shards::chunks(&mut self.rows, |first, buckets| {
            for (at, bucket) in buckets.iter_mut().enumerate() {
                bucket.apply(changes, first + at);
            }
            Ok(())
        })?;
        Ok(())
    }

    /// The earliest observation start of the rows the table holds.
    pub(crate) fn since(&self) -> Option<i32> {
        self.since
    }

    /// Whether some row the table holds records its spending.
    pub(crate) fn recorded(&self) -> bool {
        self.recorded
    }

    /// Lines with output whose purpose was not recorded.
    pub(crate) fn unlisted(&self) -> usize {
        self.rows.iter().map(|bucket| bucket.unlisted.len()).sum()
    }

    pub(crate) fn totals(&self, identity: Identity) -> Option<&Totals> {
        self.totals[identity_bucket(identity)].get(&identity)
    }

    /// Folds the rows as the table holds them into the totals.
    pub(crate) fn read(&mut self, read: &Read<'_>) -> io::Result<Folded> {
        let reads = shards::chunks(&mut self.rows, |_, buckets| {
            buckets
                .iter_mut()
                .map(|bucket| read_bucket(bucket, read))
                .collect::<io::Result<Vec<_>>>()
        })?;
        let mut folded = Folded::default();
        let mut spending: Option<(RowKey, ProductionSpendingUnknown)> = None;
        let mut changes: Vec<Vec<Contribution>> = Vec::with_capacity(BUCKETS);
        for bucket in reads.into_iter().flatten() {
            folded.unattributed += bucket.unattributed;
            folded.inexact += bucket.inexact;
            folded.lost += bucket.lost;
            folded.tainted += bucket.tainted;
            folded.presence_unknown |= bucket.presence_unknown;
            folded.visits += bucket.visits;
            if let Some(first) = bucket.spending
                && spending.is_none_or(|held| first.0 < held.0)
            {
                spending = Some(first);
            }
            changes.push(bucket.contributions);
        }
        folded.spending = spending.map(|(_, reason)| reason);
        let mut census: Vec<Identity> = read.census.to_vec();
        census.sort_unstable_by_key(|&identity| identity_bucket(identity));
        let (changes, census) = (&changes[..], &census[..]);
        let number = read.number;
        let restart = read.restart;
        shards::chunks(&mut self.totals, |first, buckets| {
            for (at, totals) in buckets.iter_mut().enumerate() {
                apply_bucket(totals, first + at, changes, census, number, restart)?;
            }
            Ok(())
        })?;
        Ok(folded)
    }
}

/// Reads `bucket`'s rows a read of every row would change anything at.
fn read_bucket(bucket: &mut RowBucket, read: &Read<'_>) -> io::Result<BucketRead> {
    let mut out = BucketRead::default();
    let RowBucket {
        rows: held,
        unlisted,
        listed,
    } = bucket;
    let mut candidates = std::mem::take(listed);
    if read.restart {
        // A new observation reads every row anew; rows the table lost are forgotten.
        held.retain(|_, row| row.table.is_some());
        for row in held.values_mut() {
            row.state = None;
            row.live = false;
            row.listed = false;
        }
        candidates.clear();
        candidates.extend(held.keys().copied());
    }
    // A row forgotten and inserted again since is listed twice.
    candidates.sort_unstable();
    candidates.dedup();
    // As a look at every row would find them, in row order.
    for key in candidates {
        let Some(row) = held.get_mut(&key) else {
            continue;
        };
        row.listed = false;
        match row.table {
            Some(table) if row.dirty || row.state.is_none_or(|state| state.identity.is_none()) => {
                visit(
                    &mut out,
                    (&mut *unlisted, &mut *listed),
                    read,
                    key,
                    row,
                    table,
                );
            }
            None if row.live => lose(&mut out, key, row),
            _ => {}
        }
    }
    // Each totals bucket takes its share in row order, those lost last.
    out.contributions
        .sort_by_key(|change| (change.bucket, change.lost, change.key));
    Ok(out)
}

/// Reads `row`, at `key`, as the table holds it, `table`.
fn visit(
    out: &mut BucketRead,
    (unlisted, listed): (&mut Set<u64>, &mut Vec<RowKey>),
    read: &Read<'_>,
    key: RowKey,
    row: &mut Row,
    table: OutputRow,
) {
    row.dirty = false;
    out.visits += 1;
    let purpose = table.purpose;
    let proved = purpose != Purpose::Unknown;
    if !proved {
        unlisted.insert(table.line);
    }
    let producer = || {
        read.producers
            .get(&table.country)
            .map(|&(slot, id)| (slot, id, read.observation))
    };
    // A producer told before has its totals seen already: they start again only with every
    // row's reading.
    let told = row.state.is_some_and(|state| state.identity.is_some());
    let state = row.state.get_or_insert_with(|| {
        let identity = producer();
        if identity.is_none() {
            out.unattributed += 1;
        }
        RowState {
            identity,
            entry: (table.variant, purpose),
            proved,
            units: 0,
            exact: true,
            ended: false,
            spent_raw: 0,
        }
    });
    // A producer the links could not tell before may be told now, while nothing was
    // delivered yet; units of a producer never told are lost to some country that cannot
    // be named, so no total is known after them.
    if state.identity.is_none() && state.units == 0 && state.exact {
        state.identity = producer();
    }
    if state.identity.is_none() {
        // Told perhaps at a later read: visited again there.
        row.listed = true;
        listed.push(key);
    }
    let contribute = |out: &mut BucketRead, identity: Identity, change: Change| {
        out.contributions.push(Contribution {
            bucket: identity_bucket(identity) as u8,
            lost: false,
            key,
            identity,
            entry: state.entry,
            proved: state.proved,
            change,
        });
    };
    match state.identity {
        Some(identity) if !told => contribute(out, identity, Change::Seen),
        Some(_) => {}
        None => out.presence_unknown = true,
    }
    if state.identity.is_none() && (table.units != 0 || !table.exact) {
        out.tainted += 1;
        out.spending
            .get_or_insert((key, ProductionSpendingUnknown::MissedPublication));
    }
    if state.identity.is_none() && !matches!(table.spent, Recorded::Value(Some(0))) {
        out.spending
            .get_or_insert((key, ProductionSpendingUnknown::SpendingUnavailable));
    }
    // Not exact once: a tick is left out of every later count.
    let exact = state.exact && table.exact;
    if state.exact && !exact {
        out.inexact += 1;
    }
    // The observer authenticates the applied input's tuple. Census cost, delivered units
    // and their exactness do not reprice it.
    let (delta, reason) = match table.spent {
        Recorded::Value(Some(value)) => (i128::from(value) - i128::from(state.spent_raw), None),
        Recorded::Value(None) => (0, Some(ProductionSpendingUnknown::SpendingUnavailable)),
        Recorded::NotRecorded => (0, Some(ProductionSpendingUnknown::NotRecorded)),
    };
    if let Some(identity) = state.identity {
        if delta != 0 || reason.is_some() {
            contribute(out, identity, Change::Cost { delta, reason });
        }
        if table.units != state.units || exact != state.exact {
            contribute(
                out,
                identity,
                Change::Units {
                    from: state.units,
                    to: table.units,
                    exact_from: state.exact,
                    exact_to: exact,
                },
            );
        }
    }
    if let Recorded::Value(Some(value)) = table.spent {
        state.spent_raw = value;
    }
    state.units = table.units;
    state.exact = exact;
    state.ended |= table.ended;
    // Rows still expected at the next read: those it saw that did not end.
    row.live = !state.ended;
}

/// `row`, at `key`, gone without its ended publication: it lost its final count.
fn lose(out: &mut BucketRead, key: RowKey, row: &mut Row) {
    row.live = false;
    let Some(state) = &mut row.state else {
        return;
    };
    out.lost += usize::from(state.exact);
    if let Some(identity) = state.identity {
        let gone = |change: Change| Contribution {
            bucket: identity_bucket(identity) as u8,
            lost: true,
            key,
            identity,
            entry: state.entry,
            proved: state.proved,
            change,
        };
        out.contributions.push(gone(Change::Cost {
            delta: 0,
            reason: Some(ProductionSpendingUnknown::LostOutput),
        }));
        if state.exact {
            out.contributions.push(gone(Change::Units {
                from: state.units,
                to: state.units,
                exact_from: true,
                exact_to: false,
            }));
        }
    }
    state.exact = false;
}

/// Applies the rows' and the census's changes to one bucket of totals, in row order.
fn apply_bucket(
    totals: &mut Map<Identity, Totals>,
    index: usize,
    changes: &[Vec<Contribution>],
    census: &[Identity],
    number: u64,
    restart: bool,
) -> io::Result<()> {
    if restart {
        totals.clear();
    }
    let bucket = index as u8;
    let mut mine: Vec<&Contribution> = Vec::new();
    for changes in changes {
        let start = changes.partition_point(|change| change.bucket < bucket);
        let end = changes.partition_point(|change| change.bucket <= bucket);
        mine.extend(&changes[start..end]);
    }
    mine.sort_by_key(|change| (change.lost, change.key));
    let start = census.partition_point(|&identity| identity_bucket(identity) < index);
    let end = census.partition_point(|&identity| identity_bucket(identity) <= index);
    for &identity in &census[start..end] {
        totals.entry(identity).or_default().seen = true;
    }
    // A row's changes, and often its neighbours', are of one producer: its totals found once.
    let mut at = 0;
    while let Some(first) = mine.get(at) {
        let identity = first.identity;
        let held = totals.entry(identity).or_default();
        for change in mine[at..]
            .iter()
            .take_while(|change| change.identity == identity)
        {
            at += 1;
            apply(held, change, number)?;
        }
    }
    Ok(())
}

/// Applies `change` to its producer's totals, `held`, at read `number`.
fn apply(held: &mut Totals, change: &Contribution, number: u64) -> io::Result<()> {
    match change.change {
        Change::Seen => held.seen = true,
        Change::Cost { delta, reason } => {
            held.costs.note(number, change.entry);
            let total = held.costs.entries.entry(change.entry).or_default();
            total.reason = total.reason.or(reason);
            if total.reason.is_none() {
                total.raw = total.raw.checked_add(delta).ok_or_else(|| {
                    io::Error::other("Applied IC exceeds its exact integer range")
                })?;
                // Magnitudes, so corrections between lines never cancel the evidence.
                if !change.proved {
                    total.unproven_raw = delta
                        .checked_abs()
                        .and_then(|delta| total.unproven_raw.checked_add(delta))
                        .ok_or_else(|| {
                            io::Error::other("Applied IC exceeds its exact integer range")
                        })?;
                }
            }
        }
        Change::Units {
            from,
            to,
            exact_from,
            exact_to,
        } => {
            held.units.note(number, change.entry);
            let total = held.units.entries.entry(change.entry).or_default();
            total.units += i128::from(to) - i128::from(from);
            total.inexact = total.inexact + u32::from(!exact_to) - u32::from(!exact_from);
        }
    }
    Ok(())
}

/// The rows and totals as a saved accumulator keeps them.
#[derive(Deserialize, Serialize)]
struct Saved {
    rows: Vec<(RowKey, RowState, bool)>,
    unlisted: Vec<u64>,
    totals: Vec<(Identity, SavedTotals)>,
}

#[derive(Deserialize, Serialize)]
struct SavedTotals {
    units: (u64, Vec<(Entry, Total)>),
    costs: (u64, Vec<(Entry, CostTotal)>),
    seen: bool,
}

impl Serialize for Fold {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut rows: Vec<_> = self
            .rows
            .iter()
            .flat_map(|bucket| bucket.rows.iter())
            .filter_map(|(key, row)| row.state.map(|state| (*key, state, row.live)))
            .collect();
        rows.sort_unstable_by_key(|row| row.0);
        let mut unlisted: Vec<u64> = self
            .rows
            .iter()
            .flat_map(|bucket| bucket.unlisted.iter().copied())
            .collect();
        unlisted.sort_unstable();
        let mut totals: Vec<_> = self
            .totals
            .iter()
            .flat_map(|bucket| bucket.iter())
            .map(|(identity, held)| {
                (
                    *identity,
                    SavedTotals {
                        units: (
                            held.units.version,
                            held.units.entries.iter().map(|(k, v)| (*k, *v)).collect(),
                        ),
                        costs: (
                            held.costs.version,
                            held.costs.entries.iter().map(|(k, v)| (*k, *v)).collect(),
                        ),
                        seen: held.seen,
                    },
                )
            })
            .collect();
        totals.sort_unstable_by_key(|held| held.0);
        Saved {
            rows,
            unlisted,
            totals,
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Fold {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let saved = Saved::deserialize(deserializer)?;
        let mut fold = Self::default();
        for (key, state, live) in saved.rows {
            let bucket = &mut fold.rows[line_bucket(key.0)];
            // A live row the table no longer holds is found gone at the next read.
            if live {
                bucket.listed.push(key);
            }
            bucket.rows.insert(
                key,
                Row {
                    table: None,
                    dirty: false,
                    state: Some(state),
                    live,
                    listed: live,
                },
            );
        }
        for line in saved.unlisted {
            fold.rows[line_bucket(line)].unlisted.insert(line);
        }
        for (identity, held) in saved.totals {
            fold.totals[identity_bucket(identity)].insert(
                identity,
                Totals {
                    units: Versioned {
                        version: held.units.0,
                        entries: held.units.1.into_iter().collect(),
                        noted: Default::default(),
                    },
                    costs: Versioned {
                        version: held.costs.0,
                        entries: held.costs.1.into_iter().collect(),
                        noted: Default::default(),
                    },
                    seen: held.seen,
                },
            );
        }
        Ok(fold)
    }
}
