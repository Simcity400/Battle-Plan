mod budget;
mod directory;
mod index;
mod pages;
mod reader;
mod saved;
#[cfg(test)]
mod tests;

use crate::invalid;
pub use budget::{Credit, DecodeBudget, MemoryBudget, Resident};
use directory::Root;
pub use pages::ReadPages;
pub use reader::{Cursor, FactRef, KeyCursor, KeyRef, ReadPhase, Reader};
pub use saved::Checkpoint;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{collections::BTreeMap, io};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Status<T> {
    Value(T),
    Absent,
    Unavailable(String),
    Gap,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Interval<T> {
    pub first: i32,
    pub last: i32,
    pub status: Status<T>,
}
impl<T: Resident> Resident for Status<T> {
    fn heap_bytes(&self) -> usize {
        match self {
            Self::Value(value) => value.heap_bytes(),
            Self::Unavailable(reason) => reason.capacity(),
            _ => 0,
        }
    }
    fn decode_budget() -> DecodeBudget {
        let value = T::decode_budget();
        let reason = String::decode_budget();
        DecodeBudget {
            fixed_bytes: value
                .fixed_bytes
                .max(reason.fixed_bytes)
                .saturating_add(size_of::<Self>()),
            bytes_per_encoded_byte: value
                .bytes_per_encoded_byte
                .max(reason.bytes_per_encoded_byte),
        }
    }
}
impl<T: Resident> Resident for Interval<T> {
    fn heap_bytes(&self) -> usize {
        self.status.heap_bytes()
    }
    fn decode_budget() -> DecodeBudget {
        let status = Status::<T>::decode_budget();
        DecodeBudget {
            fixed_bytes: status.fixed_bytes.saturating_add(size_of::<Self>()),
            ..status
        }
    }
}

/// Every private fact section is named `{SECTIONS}{family}/{branch}/{column}`,
/// its part after another `/`.
pub const SECTIONS: &str = "preparation-facts-v2/";

/// The column the private fact section `section` belongs to.
pub fn column(section: &str) -> Option<&str> {
    let mut parts = section.strip_prefix(SECTIONS)?.splitn(4, '/');
    let (_family, branch, column) = (parts.next()?, parts.next()?, parts.next()?);
    (!branch.is_empty() && branch.bytes().all(|byte| byte.is_ascii_digit())).then_some(column)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Namespace {
    family: String,
    branch: u64,
    column: String,
}
impl Namespace {
    pub fn new(family: &str, branch: u64, column: &str) -> io::Result<Self> {
        for name in [family, column] {
            if name.is_empty()
                || !name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            {
                return Err(invalid("Invalid private fact family or column name"));
            }
        }
        Ok(Self {
            family: family.into(),
            branch,
            column: column.into(),
        })
    }
    pub fn section(&self) -> String {
        format!("{SECTIONS}{}/{}/{}", self.family, self.branch, self.column)
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub write_bytes: usize,
    pub cache_bytes: usize,
    pub page_bytes: usize,
}
impl Limits {
    fn validate(self) -> io::Result<Self> {
        if self.write_bytes == 0 || self.page_bytes == 0 {
            return Err(invalid(
                "Private fact write and page budgets must be positive",
            ));
        }
        Ok(self)
    }
}
struct Held<T> {
    interval: Interval<T>,
    credit: Credit,
}
impl<T: Serialize> Serialize for Held<T> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.interval.serialize(serializer)
    }
}
struct Column<T> {
    root: Root,
    indexed: bool,
    pending: Vec<Held<T>>,
    open: Option<Held<T>>,
    _credit: Credit,
}

pub struct Facts<K, V> {
    namespace: Namespace,
    budget: MemoryBudget,
    limits: Limits,
    columns: BTreeMap<K, Column<V>>,
    keys: index::IndexRoot,
    pending_bytes: usize,
    poisoned: bool,
    _credit: Credit,
}
impl<
    K: Ord + Clone + Serialize + DeserializeOwned + Resident + 'static,
    V: PartialEq + Serialize + DeserializeOwned + Resident + 'static,
> Facts<K, V>
{
    pub fn new(namespace: Namespace, budget: MemoryBudget, limits: Limits) -> io::Result<Self> {
        let credit = budget.admit(
            size_of::<Self>()
                .saturating_add(12 * (size_of::<K>() + size_of::<Column<V>>() + 128))
                .saturating_add(namespace.family.capacity())
                .saturating_add(namespace.column.capacity()),
        )?;
        Ok(Self {
            _credit: credit,
            namespace,
            budget,
            limits: limits.validate()?,
            columns: BTreeMap::new(),
            keys: index::IndexRoot::default(),
            pending_bytes: 0,
            poisoned: false,
        })
    }
    pub fn namespace(&self) -> &Namespace {
        &self.namespace
    }
    pub fn buffered_bytes(&self) -> usize {
        self.pending_bytes
    }
    pub fn active_keys(&self) -> usize {
        self.columns.len()
    }
    fn healthy(&self) -> io::Result<()> {
        if self.poisoned {
            Err(invalid(
                "Private fact transaction failed; restore its last committed checkpoint",
            ))
        } else {
            Ok(())
        }
    }
    fn held(&self, interval: Interval<V>) -> io::Result<Held<V>> {
        let bytes = interval
            .heap_bytes()
            .checked_add(size_of::<Held<V>>().saturating_mul(4))
            .ok_or_else(|| invalid("Fact allocation overflow"))?;
        Ok(Held {
            interval,
            credit: self.budget.admit(bytes)?,
        })
    }
    fn key_credit(&self, key: &K) -> io::Result<Credit> {
        self.budget.admit(
            key.heap_bytes()
                .saturating_add((size_of::<K>() + size_of::<Column<V>>() + 128).saturating_mul(12)),
        )
    }
    pub fn append(
        &mut self,
        writer: &mut store::Writer,
        key: K,
        first: i32,
        last: i32,
        status: Status<V>,
    ) -> io::Result<()> {
        self.healthy()?;
        if first > last {
            return Err(invalid("Private fact interval is reversed"));
        }
        if let Some(column) = self.columns.get_mut(&key) {
            if let Some(open) = &mut column.open {
                if first <= open.interval.last {
                    return Err(invalid("Private facts did not advance"));
                }
                if open.interval.last.checked_add(1) == Some(first)
                    && open.interval.status == status
                {
                    open.interval.last = last;
                    return Ok(());
                }
            }
        }
        let previous = self
            .columns
            .get(&key)
            .and_then(|c| c.open.as_ref().map(|v| v.interval.last).or(c.root.last));
        if previous.is_some_and(|previous| first <= previous) {
            return Err(invalid("Private facts did not advance"));
        }
        let held = self.held(Interval {
            first,
            last,
            status,
        })?;
        let vacant = if !self.columns.contains_key(&key) {
            let credit = self.key_credit(&key)?;
            let mut cache = pages::Cache::new(writer, self.budget.clone(), 0)?;
            let root: Option<Root> = index::get(
                &mut cache,
                &format!("{}/keys", self.namespace.section()),
                self.keys,
                &key,
            )?;
            let indexed = root.is_some();
            let root = root.unwrap_or_default();
            if root.last.is_some_and(|previous| first <= previous) {
                return Err(invalid("Private facts did not advance"));
            }
            Some(Column {
                root,
                indexed,
                pending: Vec::new(),
                open: None,
                _credit: credit,
            })
        } else {
            None
        };
        let previous = self
            .columns
            .get(&key)
            .and_then(|column| {
                column
                    .open
                    .as_ref()
                    .map(|v| v.interval.last)
                    .or(column.root.last)
            })
            .or_else(|| vacant.as_ref().and_then(|column| column.root.last));
        let gap = if let Some(start) = previous
            .and_then(|hour| hour.checked_add(1))
            .filter(|start| *start < first)
        {
            Some(Held {
                interval: Interval {
                    first: start,
                    last: first - 1,
                    status: Status::Gap,
                },
                credit: self.budget.admit(size_of::<Held<V>>().saturating_mul(4))?,
            })
        } else {
            None
        };
        let column = match self.columns.entry(key) {
            std::collections::btree_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::btree_map::Entry::Vacant(entry) => entry.insert(vacant.unwrap()),
        };
        if let Some(old) = column.open.take() {
            self.pending_bytes += old.credit.bytes();
            column.pending.push(old);
        }
        if let Some(gap) = gap {
            self.pending_bytes += gap.credit.bytes();
            column.pending.push(gap);
        }
        column.open = Some(held);
        if self.pending_bytes >= self.limits.write_bytes {
            self.spill(writer)?;
        }
        Ok(())
    }
    pub fn spill(&mut self, writer: &mut store::Writer) -> io::Result<()> {
        self.healthy()?;
        let result = self.write_pending(writer);
        if result.is_err() {
            self.poisoned = true;
        }
        result
    }
    fn write_pending(&mut self, writer: &mut store::Writer) -> io::Result<()> {
        for (key, column) in &mut self.columns {
            if column.pending.is_empty() {
                continue;
            }
            let section = self.namespace.section();
            let mut start = 0;
            while start < column.pending.len() {
                let mut end = start;
                let mut bytes = 0usize;
                while end < column.pending.len() && (end == start || bytes < self.limits.page_bytes)
                {
                    bytes = bytes.saturating_add(column.pending[end].credit.bytes());
                    end += 1;
                }
                let rows = &column.pending[start..end];
                let encoded = pages::encode(&self.budget, rows)?;
                let payload = pages::write(
                    writer,
                    &self.budget,
                    &format!("{section}/values"),
                    encoded.value.as_slice(),
                )?;
                drop(encoded);
                directory::append(
                    writer,
                    &self.budget,
                    &format!("{section}/directory"),
                    &mut column.root,
                    directory::Entry {
                        first: rows.first().unwrap().interval.first,
                        last: rows.last().unwrap().interval.last,
                        payload,
                    },
                    self.limits.page_bytes,
                )?;
                start = end;
            }
            self.pending_bytes -= column
                .pending
                .iter()
                .map(|v| v.credit.bytes())
                .sum::<usize>();
            column.pending = Vec::new();
            self.keys = index::put(
                writer,
                &self.budget,
                &format!("{section}/keys"),
                self.keys,
                key,
                column.root,
                self.limits.page_bytes,
            )?;
            column.indexed = true;
        }
        Ok(())
    }
    pub fn retire(&mut self, writer: &mut store::Writer, key: &K) -> io::Result<()> {
        self.healthy()?;
        if let Some(column) = self.columns.get_mut(key) {
            if let Some(open) = column.open.take() {
                self.pending_bytes += open.credit.bytes();
                column.pending.push(open);
            }
        } else {
            return Ok(());
        }
        self.spill(writer)?;
        self.columns.remove(key);
        Ok(())
    }
    pub fn reader<'a, R: ReadPages>(
        &'a self,
        source: &'a mut R,
    ) -> io::Result<Reader<'a, K, V, R>> {
        self.healthy()?;
        Reader::new(self, source)
    }
}
