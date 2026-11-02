use crate::store::{EMPTY, Store, Tree, invalid};
use crate::{Blob, Bytes, Memory, StorageStats, TableId};
use std::cmp::Ordering;
use std::io::{self, Write};
use std::mem::size_of;
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Column {
    pub table: TableId,
    pub field: usize,
}

impl Column {
    pub fn named(table: TableId, name: &str) -> io::Result<Self> {
        let field = (0..table.schema().len())
            .find(|&field| table.field_name(field) == Some(name))
            .ok_or_else(|| invalid("relation field is not in the replay schema"))?;
        Ok(Self { table, field })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hours {
    pub first: u64,
    pub last: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Missing {
    Unknown,
    Refused,
    Lost,
    NotRecorded,
    ReadInPart,
    Truncated,
    Unavailable,
    MissingReference,
    Hidden,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cause<'a> {
    pub kind: Missing,
    pub hours: Hours,
    pub reason: &'a str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Evidence {
    pub kind: Missing,
    pub hours: Hours,
    pub reason: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cell<'a> {
    Value(&'a [u8]),
    Unknown(Cause<'a>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TableState {
    Complete,
    Truncated { missing: u64 },
    ReadInPart,
    Unavailable,
    NotRecorded,
    Refused,
    Lost,
}

#[derive(Clone, Copy, Debug)]
pub struct TableStatus<'a> {
    pub table: TableId,
    pub state: TableState,
    pub hours: Hours,
    pub reason: &'a str,
}

pub trait Key {
    fn len(&self) -> u64;
    fn read(&mut self, offset: u64, out: &mut [u8]) -> io::Result<()>;
    fn as_slice(&self) -> Option<&[u8]> {
        None
    }
    fn equals(&mut self, bytes: &[u8]) -> io::Result<bool> {
        if self.len() != bytes.len() as u64 {
            return Ok(false);
        }
        let mut buffer = [0; 256];
        for (i, chunk) in bytes.chunks(256).enumerate() {
            self.read((i * 256) as u64, &mut buffer[..chunk.len()])?;
            if &buffer[..chunk.len()] != chunk {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

struct StoredKey<'s, 'b> {
    store: &'s mut Store,
    bytes: Bytes<'b>,
}
impl Key for StoredKey<'_, '_> {
    fn len(&self) -> u64 {
        self.bytes.len()
    }
    fn read(&mut self, offset: u64, out: &mut [u8]) -> io::Result<()> {
        self.store.chunk(self.bytes, offset, out)
    }
    fn as_slice(&self) -> Option<&[u8]> {
        if let Bytes::Borrowed(bytes) = self.bytes {
            Some(bytes)
        } else {
            None
        }
    }
}

// Keys includes held hidden rows; out_of_view visits their keys at restore.
pub trait Columns {
    fn out_of_view(
        &self,
        table: TableId,
        visit: &mut dyn FnMut(&[u8]) -> io::Result<()>,
    ) -> io::Result<()>;
    fn cell<'r>(&'r self, column: Column, key: &mut dyn Key) -> io::Result<Option<Cell<'r>>>;
    fn keys(
        &self,
        table: TableId,
        prefix: &[u8],
        visit: &mut dyn FnMut(&[u8]) -> io::Result<()>,
    ) -> io::Result<()>;
}

#[derive(Clone, Copy, Debug)]
pub struct Link<'a> {
    pub column: Column,
    pub no_values: &'a [&'a [u8]],
}

impl From<Column> for Link<'_> {
    fn from(column: Column) -> Self {
        Self {
            column,
            no_values: &[],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lookup {
    Secondary,
    Prefix,
}

#[derive(Clone, Copy, Debug)]
pub struct RelationSpec<'a> {
    pub chain: &'a [Link<'a>],
    pub lookup: Lookup,
}

impl<'a> RelationSpec<'a> {
    pub fn secondary(chain: &'a [Link<'a>]) -> Self {
        Self {
            chain,
            lookup: Lookup::Secondary,
        }
    }
    pub fn prefix(chain: &'a [Link<'a>]) -> Self {
        Self {
            chain,
            lookup: Lookup::Prefix,
        }
    }
}

// Integer keys and values must use key::write_integer, whose byte order is numeric order; notice order relies on it.
#[derive(Clone, Copy, Debug)]
pub enum Input<'a> {
    Reread {
        table: TableId,
    },
    Appeared {
        table: TableId,
        key: &'a [u8],
        fields: &'a [(usize, Cell<'a>)],
    },
    Changed {
        column: Column,
        key: &'a [u8],
        previous: Cell<'a>,
        cell: Cell<'a>,
    },
    Gone {
        table: TableId,
        key: &'a [u8],
        previous: &'a [(usize, Cell<'a>)],
    },
    Visibility {
        table: TableId,
        key: &'a [u8],
        included: bool,
    },
    Status(TableStatus<'a>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Membership {
    Joined,
    Left,
}

#[derive(Clone, Copy, Debug)]
pub struct MembershipEvidence<'a> {
    pub kind: Missing,
    pub hours: Hours,
    pub reason: Bytes<'a>,
}

#[derive(Clone, Copy, Debug)]
// A cause belongs to the net membership change, rather than one input row event.
pub enum MembershipCause<'a> {
    Changed,
    Fresh,
    Recovered(MembershipEvidence<'a>),
    Missing(MembershipEvidence<'a>),
    Hidden(MembershipEvidence<'a>),
}

// Copy notification byte handles inside the callback; settlement can reclaim them.
#[derive(Clone, Copy, Debug)]
pub enum Notification<'a> {
    Membership {
        relation: usize,
        hour: u64,
        kind: Membership,
        cause: MembershipCause<'a>,
        value: Bytes<'a>,
        key: Bytes<'a>,
    },
    Status {
        relation: usize,
        table: TableId,
        state: TableState,
        evidence: Evidence,
    },
}

#[derive(Clone, Copy, Debug, Default)]
pub struct RelationStats {
    pub input_edges: u64,
    pub result_edges: u64,
    pub owned_entries: u64,
    pub payload_bytes: u64,
    pub live_bytes: u64,
}

struct Stage {
    column: Column,
    reverse: Option<Tree>,
}
struct Table {
    id: TableId,
    hidden: Tree,
    reason_tree: Tree,
    reason: Option<Blob>,
    state: TableState,
    evidence: Option<Evidence>,
    loss: Option<Evidence>,
    reread: Option<u64>,
}
struct Relation<'a> {
    chain: &'a [Link<'a>],
    lookup: Lookup,
    result: Option<Tree>,
    unresolved: Tree,
    dirty: Tree,
    settled: Tree,
}

impl Relation<'_> {
    fn reads_stage(&self, column: Column) -> bool {
        self.chain.iter().enumerate().any(|(position, link)| {
            link.column == column
                && (position + 1 < self.chain.len()
                    || (self.chain.len() == 1 && self.lookup == Lookup::Secondary))
        })
    }
}

#[derive(Clone, Copy)]
enum Outcome<'a> {
    Member(Bytes<'a>),
    Absent,
    Hidden(Evidence),
    Missing(Evidence),
}

impl Outcome<'_> {
    fn evidence(self) -> Option<Evidence> {
        match self {
            Self::Hidden(e) | Self::Missing(e) => Some(e),
            _ => None,
        }
    }
}

pub struct Indexes<'a> {
    store: Store,
    stages: Vec<Stage>,
    tables: Vec<Table>,
    relations: Vec<Relation<'a>>,
    reasons: Tree,
    hour: Option<u64>,
    last_hour: Option<u64>,
    restart: Option<u64>,
    failed: bool,
    delivering: bool,
}

impl<'a> Indexes<'a> {
    pub fn new(path: &Path, memory: Memory, specs: &[RelationSpec<'a>]) -> io::Result<Self> {
        let count = specs.iter().try_fold(0usize, |sum, spec| {
            let chain = spec.chain;
            if spec.lookup == Lookup::Prefix && chain.len() != 1 {
                return Err(invalid("prefix needs one column"));
            }
            if chain.is_empty() {
                return Err(invalid("empty relation"));
            }
            for link in chain {
                if link.column.table.field_name(link.column.field).is_none() {
                    return Err(invalid("invalid relation field"));
                }
            }
            sum.checked_add(chain.len())
                .ok_or_else(|| invalid("relation metadata overflow"))
        })?;
        let metadata = count
            .checked_mul(size_of::<Stage>() + size_of::<Table>())
            .and_then(|bytes| bytes.checked_add(specs.len().checked_mul(size_of::<Relation>())?))
            .and_then(|bytes| bytes.checked_add(size_of::<Self>()))
            .ok_or_else(|| invalid("relation metadata overflow"))?;
        let trees = count
            .checked_mul(3)
            .and_then(|n| n.checked_add(specs.len().checked_mul(4)?))
            .and_then(|n| n.checked_add(1))
            .ok_or_else(|| invalid("relation metadata overflow"))?;
        let store = Store::new(path, memory, metadata, trees)?;
        let mut stages = Vec::new();
        let mut tables = Vec::new();
        let mut relations = Vec::new();
        stages.try_reserve_exact(count).map_err(io::Error::other)?;
        tables.try_reserve_exact(count).map_err(io::Error::other)?;
        relations
            .try_reserve_exact(specs.len())
            .map_err(io::Error::other)?;
        let mut next = 0;
        let mut tree = || {
            let tree = Tree(next);
            next += 1;
            tree
        };
        let reasons = tree();
        for spec in specs {
            let chain = spec.chain;
            for link in chain {
                if !stages.iter().any(|s: &Stage| s.column == link.column) {
                    stages.push(Stage {
                        column: link.column,
                        reverse: specs
                            .iter()
                            .any(|spec| {
                                spec.chain.iter().enumerate().any(|(p, l)| {
                                    l.column == link.column
                                        && (p + 1 < spec.chain.len()
                                            || (spec.chain.len() == 1
                                                && spec.lookup == Lookup::Secondary))
                                })
                            })
                            .then(&mut tree),
                    });
                }
                if !tables.iter().any(|t: &Table| t.id == link.column.table) {
                    tables.push(Table {
                        id: link.column.table,
                        hidden: tree(),
                        reason_tree: tree(),
                        reason: None,
                        state: TableState::NotRecorded,
                        evidence: None,
                        loss: None,
                        reread: None,
                    });
                }
            }
            let result = if spec.lookup == Lookup::Prefix {
                None
            } else if chain.len() == 1 {
                stages
                    .iter()
                    .find(|s| s.column == chain[0].column)
                    .unwrap()
                    .reverse
            } else {
                Some(tree())
            };
            relations.push(Relation {
                chain,
                lookup: spec.lookup,
                result,
                unresolved: tree(),
                dirty: tree(),
                settled: tree(),
            });
        }
        Ok(Self {
            store,
            stages,
            tables,
            relations,
            reasons,
            hour: None,
            last_hour: None,
            restart: None,
            failed: false,
            delivering: false,
        })
    }

    pub fn bootstrap(
        &mut self,
        hour: u64,
        reader: &impl Columns,
        statuses: &[TableStatus<'_>],
    ) -> io::Result<()> {
        self.healthy()?;
        if self.restart.is_some() {
            return Err(invalid("bootstrap must run once before hours"));
        }
        self.restart = Some(hour);
        let result = (|| {
            for table in 0..self.tables.len() {
                let status = statuses
                    .iter()
                    .find(|s| s.table == self.tables[table].id)
                    .ok_or_else(|| invalid("restored table needs an explicit status"))?;
                validate_hours(status.hours)?;
                self.tables[table].state = status.state;
                self.cache_status_reason(table, status.reason)?;
                reader.out_of_view(status.table, &mut |key| {
                    self.store.put(
                        self.tables[table].hidden,
                        Bytes::Borrowed(key),
                        EMPTY,
                        EMPTY,
                    )
                })?;
                self.tables[table].evidence = Some(self.intern(Cause {
                    kind: status_kind(status.state),
                    hours: status.hours,
                    reason: status.reason,
                })?);
                if unavailable(status.state) {
                    let evidence = self.tables[table].evidence.unwrap();
                    self.retain(evidence.reason)?;
                    self.tables[table].loss = Some(evidence);
                }
            }
            for stage in 0..self.stages.len() {
                let column = self.stages[stage].column;
                let Some(reverse) = self.stages[stage].reverse else {
                    continue;
                };
                if unavailable(self.table(column.table).state) {
                    continue;
                }
                reader.keys(column.table, &[], &mut |key| {
                    if self.contains(self.table(column.table).hidden, Bytes::Borrowed(key))? {
                        return Ok(());
                    }
                    let cell = self
                        .cell(reader, column, Bytes::Borrowed(key))?
                        .ok_or_else(|| invalid("restored row omits a declared field"))?;
                    if let Cell::Value(value) = cell {
                        self.store.put(
                            reverse,
                            Bytes::Borrowed(value),
                            Bytes::Borrowed(key),
                            EMPTY,
                        )?;
                    }
                    Ok(())
                })?;
            }
            for relation in 0..self.relations.len() {
                reader.keys(
                    self.relations[relation].chain[0].column.table,
                    &[],
                    &mut |key| {
                        let outcome = self.resolve(relation, reader, Bytes::Borrowed(key))?;
                        self.publish_root(
                            relation,
                            Bytes::Borrowed(key),
                            Outcome::Absent,
                            outcome,
                        )?;
                        Ok(())
                    },
                )?;
            }
            Ok(())
        })();
        if result.is_err() {
            self.failed = true;
        }
        result
    }

    pub fn begin_hour(&mut self, hour: u64) -> io::Result<()> {
        self.healthy()?;
        if self.restart.is_none()
            || self.hour.is_some()
            || self.last_hour.is_some_and(|h| hour <= h)
            || hour < self.restart.unwrap()
        {
            return Err(invalid("hours must follow bootstrap in increasing order"));
        }
        self.hour = Some(hour);
        Ok(())
    }

    // Apply the reader mutation first; keep Changed/Gone before-images, and reset rows before Reread.
    pub fn apply(
        &mut self,
        reader: &impl Columns,
        input: Input<'_>,
        mut sink: impl FnMut(&mut Self, Notification<'_>) -> io::Result<()>,
    ) -> io::Result<()> {
        self.healthy()?;
        if self.hour.is_none() || self.delivering {
            return Err(invalid(
                "open an hour before applying inputs outside delivery",
            ));
        }
        let result = self.apply_inner(reader, input, &mut sink);
        if result.is_err() {
            self.failed = true;
        }
        result
    }

    fn apply_inner(
        &mut self,
        reader: &impl Columns,
        input: Input<'_>,
        sink: &mut impl FnMut(&mut Self, Notification<'_>) -> io::Result<()>,
    ) -> io::Result<()> {
        if let Input::Status(status) = input {
            return self.status(reader, status, sink);
        }
        if let Input::Reread { table } = input {
            if !self.tables.iter().any(|t| t.id == table) {
                return Ok(());
            }
            if self.table(table).reread != self.hour {
                self.touch_status(reader, table)?;
            }
            for stage in &self.stages {
                if stage.column.table == table
                    && let Some(tree) = stage.reverse
                {
                    self.store.clear(tree)?;
                }
            }
            let index = self.tables.iter().position(|t| t.id == table).unwrap();
            self.store.clear(self.tables[index].hidden)?;
            self.tables[index].reread = self.hour;
            return Ok(());
        }
        let (table, key) = match input {
            Input::Appeared { table, key, .. }
            | Input::Gone { table, key, .. }
            | Input::Visibility { table, key, .. } => (table, key),
            Input::Changed { column, key, .. } => (column.table, key),
            Input::Status(_) | Input::Reread { .. } => unreachable!(),
        };
        if !self.tables.iter().any(|t| t.id == table) {
            return Ok(());
        }
        if let Input::Appeared { fields, .. }
        | Input::Gone {
            previous: fields, ..
        } = input
        {
            if self
                .stages
                .iter()
                .filter(|s| s.column.table == table)
                .any(|s| !fields.iter().any(|(field, _)| *field == s.column.field))
            {
                return Err(invalid("row input omits a declared relation field"));
            }
        }
        if let Input::Visibility { .. } = input {
            let column = self
                .stages
                .iter()
                .find(|s| s.column.table == table)
                .unwrap()
                .column;
            if self.cell(reader, column, Bytes::Borrowed(key))?.is_none() {
                return Err(invalid("visibility names an absent row"));
            }
        }
        for relation in 0..self.relations.len() {
            for position in 0..self.relations[relation].chain.len() {
                let column = self.relations[relation].chain[position].column;
                if column.table == table
                    && !matches!(input, Input::Changed { column: changed, .. } if changed != column)
                {
                    self.touch_at(
                        relation,
                        position,
                        Bytes::Borrowed(key),
                        reader,
                        Some(input),
                    )?;
                }
            }
        }
        let hidden = self.contains(self.table(table).hidden, Bytes::Borrowed(key))?;
        let included = match input {
            Input::Visibility { included, .. } => included,
            Input::Gone { .. } => false,
            Input::Appeared { .. } => true,
            _ => !hidden,
        };
        for stage in 0..self.stages.len() {
            let column = self.stages[stage].column;
            if column.table != table
                || matches!(input, Input::Changed { column: changed, .. } if changed != column)
            {
                continue;
            }
            let Some(tree) = self.stages[stage].reverse else {
                continue;
            };
            let previous = match input {
                Input::Changed { previous, .. } => Some(previous),
                Input::Gone { previous, .. } => previous
                    .iter()
                    .find(|(field, _)| *field == column.field)
                    .map(|(_, cell)| *cell),
                Input::Appeared { .. } => None,
                _ => self.cell(reader, column, Bytes::Borrowed(key))?,
            };
            let next = match input {
                Input::Appeared { fields, .. } => fields
                    .iter()
                    .find(|(field, _)| *field == column.field)
                    .map(|(_, cell)| *cell),
                Input::Changed { cell, .. } => Some(cell),
                Input::Visibility { .. } => previous,
                Input::Gone { .. } => None,
                Input::Status(_) | Input::Reread { .. } => unreachable!(),
            };
            if previous == next && included == !hidden {
                continue;
            }
            if !hidden && let Some(Cell::Value(value)) = previous {
                self.store
                    .remove(tree, Bytes::Borrowed(value), Bytes::Borrowed(key))?;
            }
            if included && let Some(Cell::Value(value)) = next {
                self.store
                    .put(tree, Bytes::Borrowed(value), Bytes::Borrowed(key), EMPTY)?;
            }
        }
        let meta = self.table(table);
        let hidden_tree = meta.hidden;
        if hidden && (included || matches!(input, Input::Gone { .. })) {
            self.store
                .remove(hidden_tree, Bytes::Borrowed(key), EMPTY)?;
        } else if !included && !hidden && matches!(input, Input::Visibility { .. }) {
            self.store
                .put(hidden_tree, Bytes::Borrowed(key), EMPTY, EMPTY)?;
        }
        Ok(())
    }

    fn touch_at(
        &mut self,
        relation: usize,
        position: usize,
        key: Bytes<'_>,
        reader: &impl Columns,
        before: Option<Input<'_>>,
    ) -> io::Result<()> {
        if position == 0 {
            if self.contains(self.relations[relation].dirty, key)? {
                return Ok(());
            }
            let old = if self.relations[relation].lookup == Lookup::Prefix {
                Outcome::Absent
            } else if let Some(entry) = self.get(self.relations[relation].unresolved, key)? {
                let old = self.decode(entry.value)?;
                if let Some(e) = old.evidence() {
                    self.retain(e.reason)?;
                }
                old
            } else {
                self.resolve_before(relation, reader, key, before)?
            };
            return self.put_outcome(self.relations[relation].dirty, key, old);
        }
        let column = self.relations[relation].chain[position - 1].column;
        let tree = self.stage(column);
        let mut cursor = self.store.cursor(tree, key, EMPTY, true)?;
        while let Some(entry) = self.store.advance(&mut cursor)? {
            if self.store.cmp(Bytes::Stored(entry.a), key)? != Ordering::Equal {
                break;
            }
            self.touch_at(
                relation,
                position - 1,
                Bytes::Stored(entry.b),
                reader,
                before,
            )?;
        }
        Ok(())
    }

    fn status(
        &mut self,
        reader: &impl Columns,
        status: TableStatus<'_>,
        sink: &mut impl FnMut(&mut Self, Notification<'_>) -> io::Result<()>,
    ) -> io::Result<()> {
        validate_hours(status.hours)?;
        let Some(index) = self.tables.iter().position(|t| t.id == status.table) else {
            return Ok(());
        };
        let same_reason = self.tables[index]
            .reason
            .is_some_and(|blob| blob.len == status.reason.len() as u64)
            && self.store.cmp(
                Bytes::Stored(self.tables[index].reason.unwrap()),
                Bytes::Borrowed(status.reason.as_bytes()),
            )? == Ordering::Equal;
        if self.tables[index].state == status.state
            && same_reason
            && (status.state != TableState::Complete
                || self.store.tree_stats(self.tables[index].hidden).entries == 0)
        {
            return Ok(());
        }
        let previous_state = self.tables[index].state;
        if unavailable(previous_state) || unavailable(status.state) {
            self.touch_status(reader, status.table)?;
        } else if status.state == TableState::Complete
            && self.store.tree_stats(self.tables[index].hidden).entries != 0
        {
            let mut cursor = self
                .store
                .cursor(self.tables[index].hidden, EMPTY, EMPTY, true)?;
            while let Some(entry) = self.store.advance(&mut cursor)? {
                for relation in 0..self.relations.len() {
                    for position in 0..self.relations[relation].chain.len() {
                        if self.relations[relation].chain[position].column.table == status.table {
                            self.touch_at(
                                relation,
                                position,
                                Bytes::Stored(entry.a),
                                reader,
                                None,
                            )?;
                        }
                    }
                }
            }
        }
        let evidence = if same_reason {
            Evidence {
                kind: status_kind(status.state),
                hours: status.hours,
                reason: self.tables[index].evidence.unwrap().reason,
            }
        } else {
            let evidence = self.intern(Cause {
                kind: status_kind(status.state),
                hours: status.hours,
                reason: status.reason,
            })?;
            if let Some(old) = self.tables[index].evidence {
                self.release(old.reason)?;
            }
            self.cache_status_reason(index, status.reason)?;
            evidence
        };
        self.tables[index].evidence = Some(evidence);
        self.tables[index].state = status.state;
        if unavailable(previous_state) && !unavailable(status.state) {
            self.tables[index].reread = self.hour;
        }
        if unavailable(status.state) {
            if let Some(old) = self.tables[index].loss {
                self.release(old.reason)?;
            }
            self.retain(evidence.reason)?;
            self.tables[index].loss = Some(evidence);
            self.store.clear(self.tables[index].hidden)?;
            for stage in 0..self.stages.len() {
                if self.stages[stage].column.table == status.table
                    && let Some(tree) = self.stages[stage].reverse
                {
                    self.store.clear(tree)?;
                }
            }
        } else if status.state == TableState::Complete
            && self.store.tree_stats(self.tables[index].hidden).entries != 0
        {
            let mut cursor = self
                .store
                .cursor(self.tables[index].hidden, EMPTY, EMPTY, true)?;
            while let Some(entry) = self.store.advance(&mut cursor)? {
                for stage in 0..self.stages.len() {
                    let column = self.stages[stage].column;
                    if column.table == status.table
                        && let Some(tree) = self.stages[stage].reverse
                        && let Some(Cell::Value(value)) =
                            self.cell(reader, column, Bytes::Stored(entry.a))?
                    {
                        self.store.put(
                            tree,
                            Bytes::Borrowed(value),
                            Bytes::Stored(entry.a),
                            EMPTY,
                        )?;
                    }
                }
            }
            self.store.clear(self.tables[index].hidden)?;
        }
        for relation in 0..self.relations.len() {
            if self.relations[relation]
                .chain
                .iter()
                .any(|l| l.column.table == status.table)
            {
                sink(
                    self,
                    Notification::Status {
                        relation,
                        table: status.table,
                        state: status.state,
                        evidence,
                    },
                )?;
            }
        }
        Ok(())
    }

    fn resolve<'r>(
        &mut self,
        relation: usize,
        reader: &'r impl Columns,
        root: Bytes<'r>,
    ) -> io::Result<Outcome<'r>> {
        self.resolve_before(relation, reader, root, None)
    }

    fn resolve_before<'r>(
        &mut self,
        relation: usize,
        reader: &'r impl Columns,
        root: Bytes<'r>,
        before: Option<Input<'r>>,
    ) -> io::Result<Outcome<'r>> {
        let mut key = root;
        for position in 0..self.relations[relation].chain.len() {
            let link = self.relations[relation].chain[position];
            let table = self.table(link.column.table);
            if unavailable(table.state) {
                let e = table.evidence.unwrap();
                self.retain(e.reason)?;
                return Ok(Outcome::Missing(e));
            }
            if self.contains(table.hidden, key)? {
                return Ok(Outcome::Hidden(self.intern(Cause {
                    kind: Missing::Hidden,
                    hours: self.hours(),
                    reason: "row is outside the recorded view",
                })?));
            }
            let previous = match before {
                Some(Input::Changed {
                    column,
                    key: changed,
                    previous,
                    ..
                }) if column == link.column
                    && self.store.cmp(key, Bytes::Borrowed(changed))? == Ordering::Equal =>
                {
                    Some(Some(previous))
                }
                Some(Input::Gone {
                    table,
                    key: gone,
                    previous,
                }) if table == link.column.table
                    && self.store.cmp(key, Bytes::Borrowed(gone))? == Ordering::Equal =>
                {
                    Some(
                        previous
                            .iter()
                            .find(|(f, _)| *f == link.column.field)
                            .map(|(_, c)| *c),
                    )
                }
                Some(Input::Appeared {
                    table,
                    key: appeared,
                    ..
                }) if table == link.column.table
                    && self.store.cmp(key, Bytes::Borrowed(appeared))? == Ordering::Equal =>
                {
                    Some(None)
                }
                _ => None,
            };
            let cell = match previous {
                Some(cell) => cell,
                None => self.cell(reader, link.column, key)?,
            };
            match cell {
                Some(Cell::Value(value)) => {
                    if link.no_values.contains(&value) {
                        return Ok(Outcome::Absent);
                    }
                    key = Bytes::Borrowed(value);
                }
                Some(Cell::Unknown(cause)) => {
                    validate_hours(cause.hours)?;
                    return Ok(Outcome::Missing(self.intern(cause)?));
                }
                None => {
                    let table = self.table(link.column.table);
                    if previous.is_some()
                        && let Some(e) = table.loss
                        && table.reread == self.hour
                    {
                        self.retain(e.reason)?;
                        return Ok(Outcome::Missing(e));
                    }
                    return if position == 0 {
                        Ok(Outcome::Absent)
                    } else {
                        Ok(Outcome::Missing(self.intern(Cause {
                            kind: Missing::MissingReference,
                            hours: self.hours(),
                            reason: "relation target is absent from the recorded view",
                        })?))
                    };
                }
            }
        }
        Ok(Outcome::Member(key))
    }

    pub fn finish_hour(
        &mut self,
        reader: &impl Columns,
        mut sink: impl FnMut(&mut Self, Notification<'_>) -> io::Result<()>,
    ) -> io::Result<()> {
        self.healthy()?;
        if self.delivering {
            return Err(invalid("settlement is already delivering"));
        }
        let hour = self.hour.ok_or_else(|| invalid("no open hour"))?;
        let result = (|| {
            for relation in 0..self.relations.len() {
                let tree = self.relations[relation].dirty;
                if self.store.tree_stats(tree).entries == 0 {
                    continue;
                }
                let mut cursor = self.store.cursor(tree, EMPTY, EMPTY, true)?;
                while let Some(entry) = self.store.advance(&mut cursor)? {
                    let key = Bytes::Stored(entry.a);
                    let previous = self.decode(entry.value)?;
                    let next = self.resolve(relation, reader, key)?;
                    let notify = self.relations[relation].lookup == Lookup::Secondary
                        && match (previous, next) {
                            (Outcome::Member(a), Outcome::Member(b)) => {
                                self.store.cmp(a, b)? != Ordering::Equal
                            }
                            (Outcome::Member(_), _) | (_, Outcome::Member(_)) => true,
                            _ => false,
                        };
                    if notify {
                        if let Outcome::Member(value) = previous {
                            self.put_settled(relation, value, key, Membership::Left, next)?;
                        }
                        if let Outcome::Member(value) = next {
                            self.put_settled(relation, value, key, Membership::Joined, previous)?;
                        }
                    }
                    self.publish_root(relation, key, previous, next)?;
                    if !notify && let Some(e) = previous.evidence() {
                        self.release(e.reason)?;
                    }
                }
                self.store.clear(tree)?;
            }
            for table in 0..self.tables.len() {
                if self.tables[table].reread == Some(hour) {
                    self.tables[table].reread = None;
                    if let Some(e) = self.tables[table].loss.take() {
                        self.release(e.reason)?;
                    }
                }
            }
            self.delivering = true;
            for relation in 0..self.relations.len() {
                let tree = self.relations[relation].settled;
                if self.store.tree_stats(tree).entries == 0 {
                    continue;
                }
                let mut cursor = self.store.cursor(tree, EMPTY, EMPTY, true)?;
                while let Some(entry) = self.store.advance(&mut cursor)? {
                    let mut joined = [0];
                    self.store.read(entry.value, 0, &mut joined)?;
                    let joined = joined[0] != 0;
                    let other = self.decode(entry.value.slice(1, entry.value.len - 1))?;
                    let cause = match (joined, other) {
                        (true, Outcome::Missing(e) | Outcome::Hidden(e)) => {
                            MembershipCause::Recovered(self.membership_evidence(e)?)
                        }
                        (false, Outcome::Missing(e)) => {
                            MembershipCause::Missing(self.membership_evidence(e)?)
                        }
                        (false, Outcome::Hidden(e)) => {
                            MembershipCause::Hidden(self.membership_evidence(e)?)
                        }
                        (true, _) => MembershipCause::Fresh,
                        (false, _) => MembershipCause::Changed,
                    };
                    sink(
                        self,
                        Notification::Membership {
                            relation,
                            hour,
                            kind: if joined {
                                Membership::Joined
                            } else {
                                Membership::Left
                            },
                            cause,
                            value: Bytes::Stored(entry.a),
                            key: Bytes::Stored(entry.b),
                        },
                    )?;
                    if joined && let Some(e) = other.evidence() {
                        self.release(e.reason)?;
                    }
                }
                self.store.clear(tree)?;
            }
            Ok(())
        })();
        if result.is_err() {
            self.failed = true;
            return result;
        }
        self.delivering = false;
        self.hour = None;
        self.last_hour = Some(hour);
        Ok(())
    }

    fn put_settled(
        &mut self,
        relation: usize,
        value: Bytes<'_>,
        key: Bytes<'_>,
        kind: Membership,
        other: Outcome<'_>,
    ) -> io::Result<()> {
        let other = match other {
            Outcome::Member(_) => Outcome::Absent,
            other => other,
        };
        let mut encoded = [0; 26];
        let (head, tail) = encode_outcome(other, &mut encoded);
        let joined = [u8::from(kind == Membership::Joined)];
        let parts = [Bytes::Borrowed(&joined), head, tail];
        // Net (value, member, joined) order has no ties before joined.
        self.store.put(
            self.relations[relation].settled,
            value,
            key,
            Bytes::Parts(&parts),
        )
    }

    fn membership_evidence(
        &mut self,
        evidence: Evidence,
    ) -> io::Result<MembershipEvidence<'static>> {
        Ok(MembershipEvidence {
            kind: evidence.kind,
            hours: evidence.hours,
            reason: self.reason_bytes(evidence.reason)?,
        })
    }

    fn publish_root(
        &mut self,
        relation: usize,
        key: Bytes<'_>,
        previous: Outcome<'_>,
        next: Outcome<'_>,
    ) -> io::Result<()> {
        if self.relations[relation].chain.len() > 1 {
            let equal = match (previous, next) {
                (Outcome::Member(a), Outcome::Member(b)) => {
                    self.store.cmp(a, b)? == Ordering::Equal
                }
                _ => false,
            };
            if !equal {
                if let Outcome::Member(value) = previous {
                    self.store
                        .remove(self.relations[relation].result.unwrap(), value, key)?;
                }
                if let Outcome::Member(value) = next {
                    self.store
                        .put(self.relations[relation].result.unwrap(), value, key, EMPTY)?;
                }
            }
        }
        let tree = self.relations[relation].unresolved;
        if let Some(entry) = self.get(tree, key)? {
            if let Some(e) = self.decode(entry.value)?.evidence() {
                self.release(e.reason)?;
            }
            self.store.remove(tree, key, EMPTY)?;
        }
        if next.evidence().is_some() {
            self.put_outcome(tree, key, next)?;
        }
        Ok(())
    }

    fn put_outcome(&mut self, tree: Tree, key: Bytes<'_>, outcome: Outcome<'_>) -> io::Result<()> {
        let mut bytes = [0; 26];
        let (head, tail) = encode_outcome(outcome, &mut bytes);
        self.store
            .put(tree, key, EMPTY, Bytes::Parts(&[head, tail]))
    }

    fn decode(&mut self, blob: Blob) -> io::Result<Outcome<'static>> {
        let mut code = [0];
        self.store.read(blob, 0, &mut code)?;
        match code[0] {
            1 => Ok(Outcome::Member(Bytes::Stored(blob.slice(1, blob.len - 1)))),
            2 => Ok(Outcome::Absent),
            3 | 4 => {
                let mut bytes = [0; 25];
                self.store.read(blob, 1, &mut bytes)?;
                let kind = match bytes[0] {
                    0 => Missing::Unknown,
                    1 => Missing::Refused,
                    2 => Missing::Lost,
                    3 => Missing::NotRecorded,
                    4 => Missing::ReadInPart,
                    5 => Missing::Truncated,
                    6 => Missing::Unavailable,
                    7 => Missing::MissingReference,
                    8 => Missing::Hidden,
                    _ => return Err(invalid("invalid missing evidence")),
                };
                let evidence = Evidence {
                    kind,
                    hours: Hours {
                        first: u64::from_le_bytes(bytes[1..9].try_into().unwrap()),
                        last: u64::from_le_bytes(bytes[9..17].try_into().unwrap()),
                    },
                    reason: u64::from_le_bytes(bytes[17..25].try_into().unwrap()),
                };
                Ok(if code[0] == 3 {
                    Outcome::Hidden(evidence)
                } else {
                    Outcome::Missing(evidence)
                })
            }
            _ => Err(invalid("invalid root state")),
        }
    }

    pub fn unresolved(&mut self, relation: usize, key: &[u8]) -> io::Result<Option<Evidence>> {
        self.readable()?;
        let tree = self
            .relations
            .get(relation)
            .ok_or_else(|| invalid("unknown relation"))?
            .unresolved;
        match self.store.get(tree, Bytes::Borrowed(key), EMPTY)? {
            Some(entry) => Ok(self.decode(entry.value)?.evidence()),
            None => Ok(None),
        }
    }

    pub fn secondary(
        &mut self,
        relation: usize,
        value: &[u8],
        mut visit: impl FnMut(&mut Self, Blob) -> io::Result<()>,
    ) -> io::Result<Option<Evidence>> {
        self.readable()?;
        let chain = self
            .relations
            .get(relation)
            .ok_or_else(|| invalid("unknown relation"))?
            .chain;
        if self.relations[relation].lookup != Lookup::Secondary {
            return Err(invalid("prefix declaration has no secondary tree"));
        }
        if chain.len() == 1 && chain[0].no_values.contains(&value) {
            return Ok(None);
        }
        for link in chain {
            let table = self.table(link.column.table);
            if table.state != TableState::Complete {
                return Ok(table.evidence);
            }
        }
        if let Some(entry) =
            self.store
                .next(self.relations[relation].unresolved, EMPTY, EMPTY, true)?
        {
            return Ok(self.decode(entry.value)?.evidence());
        }
        let mut cursor = self.store.cursor(
            self.relations[relation].result.unwrap(),
            Bytes::Borrowed(value),
            EMPTY,
            true,
        )?;
        while let Some(entry) = self.store.advance(&mut cursor)? {
            if self
                .store
                .cmp(Bytes::Stored(entry.a), Bytes::Borrowed(value))?
                != Ordering::Equal
            {
                break;
            }
            visit(self, entry.b)?;
        }
        Ok(None)
    }

    pub fn prefix(
        &mut self,
        relation: usize,
        reader: &impl Columns,
        prefix: &[u8],
        mut visit: impl FnMut(&mut Self, &[u8], &[u8]) -> io::Result<()>,
    ) -> io::Result<Option<Evidence>> {
        self.readable()?;
        let chain = self
            .relations
            .get(relation)
            .ok_or_else(|| invalid("unknown relation"))?
            .chain;
        if chain.len() != 1 {
            return Err(invalid("prefix requires a single-column relation"));
        }
        let column = chain[0].column;
        if self.table(column.table).state != TableState::Complete {
            return Ok(self.table(column.table).evidence);
        }
        if let Some(entry) = self.store.next(
            self.relations[relation].unresolved,
            Bytes::Borrowed(prefix),
            EMPTY,
            true,
        )? {
            if entry.a.len >= prefix.len() as u64
                && self.store.cmp(
                    Bytes::Stored(entry.a.slice(0, prefix.len() as u64)),
                    Bytes::Borrowed(prefix),
                )? == Ordering::Equal
            {
                return Ok(self.decode(entry.value)?.evidence());
            }
        }
        reader.keys(column.table, prefix, &mut |key| {
            match self.cell(reader, column, Bytes::Borrowed(key))? {
                Some(Cell::Value(value)) => {
                    if !chain[0].no_values.contains(&value) {
                        visit(self, key, value)?;
                    }
                }
                _ => return Err(invalid("reader omitted a declared prefix cell")),
            }
            Ok(())
        })?;
        Ok(None)
    }

    fn cell<'r>(
        &mut self,
        reader: &'r impl Columns,
        column: Column,
        bytes: Bytes<'_>,
    ) -> io::Result<Option<Cell<'r>>> {
        self.store.reader_lookup();
        reader.cell(
            column,
            &mut StoredKey {
                store: &mut self.store,
                bytes,
            },
        )
    }
    fn table(&self, id: TableId) -> &Table {
        self.tables.iter().find(|t| t.id == id).unwrap()
    }
    fn get(&mut self, tree: Tree, key: Bytes<'_>) -> io::Result<Option<crate::store::Entry>> {
        if self.store.tree_stats(tree).entries == 0 {
            return Ok(None);
        }
        self.store.get(tree, key, EMPTY)
    }
    fn contains(&mut self, tree: Tree, key: Bytes<'_>) -> io::Result<bool> {
        Ok(self.get(tree, key)?.is_some())
    }
    fn stage(&self, column: Column) -> Tree {
        self.stages
            .iter()
            .find(|s| s.column == column)
            .unwrap()
            .reverse
            .unwrap()
    }
    fn hours(&self) -> Hours {
        let hour = self.hour.or(self.restart).unwrap();
        Hours {
            first: hour,
            last: hour,
        }
    }
    fn healthy(&self) -> io::Result<()> {
        if self.failed {
            Err(invalid(
                "index update failed; restore this reader before reuse",
            ))
        } else {
            Ok(())
        }
    }
    fn readable(&self) -> io::Result<()> {
        self.healthy()?;
        if self.restart.is_none() {
            return Err(invalid("bootstrap before index lookups"));
        }
        if self.hour.is_some() && !self.delivering {
            Err(invalid("settle the hour before membership lookups"))
        } else {
            Ok(())
        }
    }

    fn cache_status_reason(&mut self, table: usize, reason: &str) -> io::Result<()> {
        let tree = self.tables[table].reason_tree;
        self.store
            .put(tree, EMPTY, EMPTY, Bytes::Borrowed(reason.as_bytes()))?;
        self.tables[table].reason = Some(self.store.get(tree, EMPTY, EMPTY)?.unwrap().value);
        Ok(())
    }

    fn refresh_status_reasons(&mut self) -> io::Result<()> {
        for table in &mut self.tables {
            table.reason = self
                .store
                .get(table.reason_tree, EMPTY, EMPTY)?
                .map(|e| e.value);
        }
        Ok(())
    }

    // Run maintenance after settlement so its cost is measured separately.
    pub fn maintain(&mut self) -> io::Result<bool> {
        self.readable()?;
        if self.hour.is_some() {
            return Err(invalid("maintenance must follow settlement"));
        }
        let result = (|| {
            let compacted = self.store.compact_if_needed()?;
            if compacted {
                self.refresh_status_reasons()?;
            }
            Ok(compacted)
        })();
        self.failed |= result.is_err();
        result
    }

    fn reaches_table<'r>(
        &mut self,
        relation: usize,
        mut key: Bytes<'r>,
        table: TableId,
        reader: &'r impl Columns,
    ) -> io::Result<bool> {
        for link in self.relations[relation].chain {
            if link.column.table == table {
                return Ok(true);
            }
            let meta = self.table(link.column.table);
            if unavailable(meta.state) || self.contains(meta.hidden, key)? {
                return Ok(false);
            }
            match self.cell(reader, link.column, key)? {
                Some(Cell::Value(value)) if !link.no_values.contains(&value) => {
                    key = Bytes::Borrowed(value)
                }
                _ => return Ok(false),
            }
        }
        Ok(false)
    }

    fn touch_status(&mut self, reader: &impl Columns, table: TableId) -> io::Result<()> {
        for relation in 0..self.relations.len() {
            if !self.relations[relation]
                .chain
                .iter()
                .any(|l| l.column.table == table)
            {
                continue;
            }
            if let Some(result) = self.relations[relation].result {
                let mut cursor = self.store.cursor(result, EMPTY, EMPTY, true)?;
                while let Some(entry) = self.store.advance(&mut cursor)? {
                    let key = Bytes::Stored(entry.b);
                    let mut absent = false;
                    if self.relations[relation].chain.len() == 1 {
                        for value in self.relations[relation].chain[0].no_values {
                            absent |= self
                                .store
                                .cmp(Bytes::Stored(entry.a), Bytes::Borrowed(value))?
                                == Ordering::Equal;
                        }
                    }
                    if !self.contains(self.relations[relation].dirty, key)? {
                        self.put_outcome(
                            self.relations[relation].dirty,
                            key,
                            if absent {
                                Outcome::Absent
                            } else {
                                Outcome::Member(Bytes::Stored(entry.a))
                            },
                        )?;
                    }
                }
            }
            for position in 0..self.relations[relation].chain.len() {
                if self.relations[relation].chain[position].column.table != table {
                    continue;
                }
                let predecessor = position.saturating_sub(1);
                let link = self.relations[relation].chain[predecessor];
                let tree = self
                    .stages
                    .iter()
                    .find(|s| s.column == link.column)
                    .unwrap()
                    .reverse;
                let Some(tree) = tree else { continue };
                let mut cursor = self.store.cursor(tree, EMPTY, EMPTY, true)?;
                while let Some(entry) = self.store.advance(&mut cursor)? {
                    if position != 0 && self.excludes(link, Bytes::Stored(entry.a))? {
                        continue;
                    }
                    self.touch_nonmember_at(relation, predecessor, Bytes::Stored(entry.b))?;
                }
            }
            let mut cursor =
                self.store
                    .cursor(self.relations[relation].unresolved, EMPTY, EMPTY, true)?;
            while let Some(entry) = self.store.advance(&mut cursor)? {
                let key = Bytes::Stored(entry.a);
                if self.reaches_table(relation, key, table, reader)? {
                    self.touch_at(relation, 0, key, reader, None)?;
                }
            }
        }
        Ok(())
    }

    fn excludes(&mut self, link: Link<'_>, value: Bytes<'_>) -> io::Result<bool> {
        for excluded in link.no_values {
            if self.store.cmp(value, Bytes::Borrowed(excluded))? == Ordering::Equal {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn touch_nonmember_at(
        &mut self,
        relation: usize,
        position: usize,
        key: Bytes<'_>,
    ) -> io::Result<()> {
        if position == 0 {
            let dirty = self.relations[relation].dirty;
            if self.contains(dirty, key)? {
                return Ok(());
            }
            // Members were touched first; the reader may already have discarded the old target.
            let previous = match self.get(self.relations[relation].unresolved, key)? {
                Some(entry) => self.decode(entry.value)?,
                None => Outcome::Absent,
            };
            if let Some(e) = previous.evidence() {
                self.retain(e.reason)?;
            }
            return self.put_outcome(dirty, key, previous);
        }
        let link = self.relations[relation].chain[position - 1];
        if self.excludes(link, key)? {
            return Ok(());
        }
        let mut cursor = self
            .store
            .cursor(self.stage(link.column), key, EMPTY, true)?;
        while let Some(entry) = self.store.advance(&mut cursor)? {
            if self.store.cmp(Bytes::Stored(entry.a), key)? != Ordering::Equal {
                break;
            }
            self.touch_nonmember_at(relation, position - 1, Bytes::Stored(entry.b))?;
        }
        Ok(())
    }

    fn reason_matches(&mut self, id: u64, reason: &str) -> io::Result<bool> {
        let key = id.to_be_bytes();
        let entry = self
            .store
            .get(self.reasons, Bytes::Borrowed(&key), EMPTY)?
            .ok_or_else(|| invalid("unknown reason"))?;
        Ok(self.store.cmp(
            Bytes::Stored(entry.value.slice(8, entry.value.len - 8)),
            Bytes::Borrowed(reason.as_bytes()),
        )? == Ordering::Equal)
    }

    fn intern(&mut self, cause: Cause<'_>) -> io::Result<Evidence> {
        validate_hours(cause.hours)?;
        let mut id = 0xcbf29ce484222325u64;
        for byte in cause.reason.bytes() {
            id = (id ^ u64::from(byte)).wrapping_mul(0x100000001b3);
        }
        loop {
            let key = id.to_be_bytes();
            match self.store.get(self.reasons, Bytes::Borrowed(&key), EMPTY)? {
                None => {
                    self.set_reason(id, 1, Bytes::Borrowed(cause.reason.as_bytes()))?;
                    break;
                }
                Some(_) if self.reason_matches(id, cause.reason)? => {
                    self.retain(id)?;
                    break;
                }
                Some(_) => id = id.wrapping_add(1),
            }
        }
        Ok(Evidence {
            kind: cause.kind,
            hours: cause.hours,
            reason: id,
        })
    }

    fn set_reason(&mut self, id: u64, count: u64, text: Bytes<'_>) -> io::Result<()> {
        let count = count.to_le_bytes();
        let parts = [Bytes::Borrowed(&count), text];
        self.store.put(
            self.reasons,
            Bytes::Borrowed(&id.to_be_bytes()),
            EMPTY,
            Bytes::Parts(&parts),
        )
    }

    fn retain(&mut self, id: u64) -> io::Result<()> {
        self.reason_ref(id, true)
    }
    fn release(&mut self, id: u64) -> io::Result<()> {
        self.reason_ref(id, false)
    }
    fn reason_ref(&mut self, id: u64, retain: bool) -> io::Result<()> {
        let key = id.to_be_bytes();
        let entry = self
            .store
            .get(self.reasons, Bytes::Borrowed(&key), EMPTY)?
            .ok_or_else(|| invalid("unknown reason"))?;
        let mut bytes = [0; 8];
        self.store.read(entry.value, 0, &mut bytes)?;
        let count = u64::from_le_bytes(bytes);
        if !retain && count == 1 {
            return self
                .store
                .remove(self.reasons, Bytes::Borrowed(&key), EMPTY);
        }
        let count = if retain {
            count
                .checked_add(1)
                .ok_or_else(|| invalid("reason reference overflow"))?
        } else {
            count - 1
        };
        self.set_reason(
            id,
            count,
            Bytes::Stored(entry.value.slice(8, entry.value.len - 8)),
        )
    }

    fn reason_bytes(&mut self, id: u64) -> io::Result<Bytes<'static>> {
        let key = id.to_be_bytes();
        let entry = self
            .store
            .get(self.reasons, Bytes::Borrowed(&key), EMPTY)?
            .ok_or_else(|| invalid("unknown reason"))?;
        Ok(Bytes::Stored(entry.value.slice(8, entry.value.len - 8)))
    }
    pub fn copy_reason(&mut self, id: u64, out: &mut impl Write) -> io::Result<()> {
        let bytes = self.reason_bytes(id)?;
        self.store.copy(bytes, out)
    }
    pub fn copy(&mut self, bytes: Bytes<'_>, out: &mut impl Write) -> io::Result<()> {
        self.store.copy(bytes, out)
    }
    pub fn storage_stats(&self) -> StorageStats {
        self.store.stats()
    }
    pub fn shared_stages(&self) -> usize {
        self.stages.iter().filter(|s| s.reverse.is_some()).count()
    }
    pub fn compact(&mut self) -> io::Result<()> {
        self.readable()?;
        if self.hour.is_some() {
            return Err(invalid("compaction must follow settlement"));
        }
        let result = (|| {
            self.store.compact()?;
            self.refresh_status_reasons()
        })();
        self.failed |= result.is_err();
        result
    }
    pub fn relation_stats(&mut self, relation: usize) -> io::Result<RelationStats> {
        let declared = self
            .relations
            .get(relation)
            .ok_or_else(|| invalid("unknown relation"))?;
        let chain = declared.chain;
        let result_tree = declared.result;
        let unresolved_tree = declared.unresolved;
        let result = result_tree
            .map(|t| self.store.tree_stats(t))
            .unwrap_or_default();
        let mut stats = RelationStats {
            result_edges: result.entries,
            ..RelationStats::default()
        };
        if chain.len() == 1 && result_tree.is_some() {
            for (at, &value) in chain[0].no_values.iter().enumerate() {
                if chain[0].no_values[..at].contains(&value) {
                    continue;
                }
                let mut cursor =
                    self.store
                        .cursor(result_tree.unwrap(), Bytes::Borrowed(value), EMPTY, true)?;
                while let Some(entry) = self.store.advance(&mut cursor)? {
                    if self
                        .store
                        .cmp(Bytes::Stored(entry.a), Bytes::Borrowed(value))?
                        != Ordering::Equal
                    {
                        break;
                    }
                    stats.result_edges -= 1;
                }
            }
        }
        for stage in &self.stages {
            if stage.reverse.is_none() {
                continue;
            }
            if !self.relations[relation].reads_stage(stage.column) {
                continue;
            }
            let tree = self.store.tree_stats(stage.reverse.unwrap());
            stats.input_edges += tree.entries;
            let owner = self
                .relations
                .iter()
                .position(|r| r.reads_stage(stage.column))
                .unwrap();
            if relation == owner {
                stats.owned_entries += tree.entries;
                stats.payload_bytes += tree.payload_bytes;
                stats.live_bytes += tree.page_bytes + tree.overflow_bytes;
            }
        }
        if chain.len() > 1 {
            stats.owned_entries += result.entries;
            stats.payload_bytes += result.payload_bytes;
            stats.live_bytes += result.page_bytes + result.overflow_bytes;
        }
        let unresolved = self.store.tree_stats(unresolved_tree);
        stats.owned_entries += unresolved.entries;
        stats.payload_bytes += unresolved.payload_bytes;
        stats.live_bytes += unresolved.page_bytes + unresolved.overflow_bytes;
        Ok(stats)
    }
}

fn encode_outcome<'a>(outcome: Outcome<'a>, bytes: &'a mut [u8; 26]) -> (Bytes<'a>, Bytes<'a>) {
    match outcome {
        Outcome::Member(value) => {
            bytes[0] = 1;
            (Bytes::Borrowed(&bytes[..1]), value)
        }
        Outcome::Absent => {
            bytes[0] = 2;
            (Bytes::Borrowed(&bytes[..1]), EMPTY)
        }
        Outcome::Hidden(e) | Outcome::Missing(e) => {
            bytes[0] = if matches!(outcome, Outcome::Hidden(_)) {
                3
            } else {
                4
            };
            bytes[1] = e.kind as u8;
            bytes[2..10].copy_from_slice(&e.hours.first.to_le_bytes());
            bytes[10..18].copy_from_slice(&e.hours.last.to_le_bytes());
            bytes[18..26].copy_from_slice(&e.reason.to_le_bytes());
            (Bytes::Borrowed(bytes), EMPTY)
        }
    }
}

fn validate_hours(hours: Hours) -> io::Result<()> {
    if hours.first > hours.last {
        Err(invalid("inverted evidence hours"))
    } else {
        Ok(())
    }
}
fn unavailable(state: TableState) -> bool {
    matches!(
        state,
        TableState::Lost | TableState::NotRecorded | TableState::Unavailable | TableState::Refused
    )
}
fn status_kind(state: TableState) -> Missing {
    match state {
        TableState::Complete => Missing::Unknown,
        TableState::Truncated { .. } => Missing::Truncated,
        TableState::ReadInPart => Missing::ReadInPart,
        TableState::Unavailable => Missing::Unavailable,
        TableState::NotRecorded => Missing::NotRecorded,
        TableState::Refused => Missing::Refused,
        TableState::Lost => Missing::Lost,
    }
}
