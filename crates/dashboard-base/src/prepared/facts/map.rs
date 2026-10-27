use super::*;
use index::{IndexCursor, IndexRoot, Item};
use pages::Admitted;
use reader::PhaseOwner;
use std::{marker::PhantomData, ops::Deref, rc::Rc};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MapCheckpoint {
    namespace: Namespace,
    root: IndexRoot,
    pub cursor: u64,
}
impl Resident for MapCheckpoint {
    fn heap_bytes(&self) -> usize {
        self.namespace.family.capacity() + self.namespace.column.capacity()
    }
    fn decode_budget() -> DecodeBudget {
        DecodeBudget {
            fixed_bytes: size_of::<Self>(),
            bytes_per_encoded_byte: 4,
        }
    }
}
impl MapCheckpoint {
    pub fn namespace(&self) -> &Namespace {
        &self.namespace
    }
    pub fn write(
        &self,
        writer: &mut store::Writer,
        section: &str,
        budget: &MemoryBudget,
    ) -> io::Result<()> {
        let bytes = pages::encode(budget, self)?;
        writer.write_at(section, 0, bytes.as_slice())
    }
    pub fn read<R: ReadPages>(phase: &ReadPhase<'_, R>, section: &str) -> io::Result<MapRef<Self>> {
        Ok(MapRef {
            page: phase.cache.borrow_mut().read(section, 0)?,
        })
    }
}
pub struct PagedMap<K, V> {
    namespace: Namespace,
    root: IndexRoot,
    budget: MemoryBudget,
    limits: Limits,
    poisoned: bool,
    _types: PhantomData<(K, V)>,
    _credit: Credit,
}
pub struct MapRef<V> {
    page: Rc<Admitted<V>>,
}
impl<V> Deref for MapRef<V> {
    type Target = V;
    fn deref(&self) -> &V {
        &self.page.value
    }
}
pub struct MapCursor<K, V> {
    index: IndexCursor<K, u64>,
    pending: Option<Item<K, u64>>,
    values: String,
    owner: Rc<PhaseOwner>,
    _value: PhantomData<V>,
    _credit: Credit,
}
pub struct MapEntry<K, V> {
    key: Item<K, u64>,
    value: MapRef<V>,
}
impl<K, V> MapEntry<K, V> {
    pub fn key(&self) -> &K {
        &self.key.0
    }
    pub fn value(&self) -> &V {
        &self.value
    }
}
impl<K, V> PagedMap<K, V>
where
    K: Ord + Clone + Serialize + DeserializeOwned + Resident + 'static,
    V: Serialize + DeserializeOwned + Resident + 'static,
{
    pub fn new(namespace: Namespace, budget: MemoryBudget, limits: Limits) -> io::Result<Self> {
        let credit = budget
            .admit(size_of::<Self>() + namespace.family.capacity() + namespace.column.capacity())?;
        Ok(Self {
            namespace,
            root: IndexRoot::default(),
            budget,
            limits: limits.validate()?,
            poisoned: false,
            _types: PhantomData,
            _credit: credit,
        })
    }
    fn healthy(&self) -> io::Result<()> {
        if self.poisoned {
            Err(invalid(
                "Private map transaction failed; restore its committed checkpoint",
            ))
        } else {
            Ok(())
        }
    }
    pub fn len(&self) -> u64 {
        self.root.count
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn put(&mut self, writer: &mut store::Writer, key: K, value: V) -> io::Result<()> {
        self.healthy()?;
        let _input = self.budget.admit(
            size_of::<(K, V)>()
                .saturating_add(key.heap_bytes())
                .saturating_add(value.heap_bytes()),
        )?;
        let encoded = pages::encode(&self.budget, &value)?;
        let section = self.namespace.section();
        let result = (|| {
            let range = pages::write(
                writer,
                &self.budget,
                &format!("{section}/map-values"),
                encoded.as_slice(),
            )?;
            drop(encoded);
            self.root = index::put(
                writer,
                &self.budget,
                &format!("{section}/map-keys"),
                self.root,
                &key,
                range,
                self.limits.page_bytes,
            )?;
            Ok(())
        })();
        if result.is_err() {
            self.poisoned = true;
        }
        result
    }
    pub fn get<R: ReadPages>(
        &self,
        phase: &ReadPhase<'_, R>,
        key: &K,
    ) -> io::Result<Option<MapRef<V>>> {
        self.healthy()?;
        phase.check(&self.budget)?;
        let section = self.namespace.section();
        let mut cache = phase.cache.borrow_mut();
        let Some(range) = index::get(&mut cache, &format!("{section}/map-keys"), self.root, key)?
        else {
            return Ok(None);
        };
        Ok(Some(MapRef {
            page: cache.read(&format!("{section}/map-values"), range)?,
        }))
    }
    pub fn checkpoint(&self, cursor: u64) -> io::Result<MapCheckpoint> {
        self.healthy()?;
        Ok(MapCheckpoint {
            namespace: self.namespace.clone(),
            root: self.root,
            cursor,
        })
    }
    pub fn restore(
        checkpoint: &MapCheckpoint,
        budget: MemoryBudget,
        limits: Limits,
    ) -> io::Result<Self> {
        let mut map = Self::new(checkpoint.namespace.clone(), budget, limits)?;
        map.root = checkpoint.root;
        Ok(map)
    }
    pub fn cursor<R: ReadPages>(
        &self,
        phase: &ReadPhase<'_, R>,
        first: Option<&K>,
        last: Option<&K>,
    ) -> io::Result<MapCursor<K, V>> {
        self.healthy()?;
        phase.check(&self.budget)?;
        let section = self.namespace.section();
        let credit = self
            .budget
            .admit(size_of::<MapCursor<K, V>>() + section.capacity() + 32)?;
        Ok(MapCursor {
            index: IndexCursor::new(
                &mut phase.cache.borrow_mut(),
                format!("{section}/map-keys"),
                self.root,
                first,
                last,
            )?,
            pending: None,
            values: format!("{section}/map-values"),
            owner: phase.owner.clone(),
            _value: PhantomData,
            _credit: credit,
        })
    }
    pub fn next<R: ReadPages>(
        &self,
        phase: &ReadPhase<'_, R>,
        cursor: &mut MapCursor<K, V>,
    ) -> io::Result<Option<MapEntry<K, V>>> {
        self.healthy()?;
        phase.check(&self.budget)?;
        if !Rc::ptr_eq(&phase.owner, &cursor.owner)
            || cursor.values != format!("{}/map-values", self.namespace.section())
        {
            return Err(invalid("Private map cursor belongs to another read phase"));
        }
        let mut cache = phase.cache.borrow_mut();
        if cursor.pending.is_none() {
            cursor.pending = cursor.index.next(&mut cache)?;
        }
        let Some(key) = cursor.pending.as_ref() else {
            return Ok(None);
        };
        let value = MapRef {
            page: cache.read(&cursor.values, key.1)?,
        };
        Ok(Some(MapEntry {
            key: cursor.pending.take().unwrap(),
            value,
        }))
    }
    pub fn visit<R: ReadPages>(
        &self,
        phase: &ReadPhase<'_, R>,
        first: Option<&K>,
        last: Option<&K>,
        mut visit: impl FnMut(&K, &V) -> io::Result<()>,
    ) -> io::Result<()> {
        let mut cursor = self.cursor(phase, first, last)?;
        while let Some(row) = self.next(phase, &mut cursor)? {
            visit(row.key(), row.value())?;
        }
        Ok(())
    }
}
pub struct Records<V>(PagedMap<u64, V>);
impl<V: Serialize + DeserializeOwned + Resident + 'static> Records<V> {
    pub fn new(namespace: Namespace, budget: MemoryBudget, limits: Limits) -> io::Result<Self> {
        Ok(Self(PagedMap::new(namespace, budget, limits)?))
    }
    pub fn append(&mut self, writer: &mut store::Writer, value: V) -> io::Result<u64> {
        let ordinal = self.0.len();
        ordinal
            .checked_add(1)
            .ok_or_else(|| invalid("Private record ordinal overflow"))?;
        self.0.put(writer, ordinal, value)?;
        Ok(ordinal)
    }
    pub fn checkpoint(&self, cursor: u64) -> io::Result<MapCheckpoint> {
        self.0.checkpoint(cursor)
    }
    pub fn restore(
        checkpoint: &MapCheckpoint,
        budget: MemoryBudget,
        limits: Limits,
    ) -> io::Result<Self> {
        Ok(Self(PagedMap::restore(checkpoint, budget, limits)?))
    }
    pub fn len(&self) -> u64 {
        self.0.len()
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    pub fn get<R: ReadPages>(
        &self,
        phase: &ReadPhase<'_, R>,
        ordinal: u64,
    ) -> io::Result<Option<MapRef<V>>> {
        self.0.get(phase, &ordinal)
    }
    pub fn cursor<R: ReadPages>(
        &self,
        phase: &ReadPhase<'_, R>,
        first: u64,
        last: u64,
    ) -> io::Result<MapCursor<u64, V>> {
        self.0.cursor(phase, Some(&first), Some(&last))
    }
    pub fn next<R: ReadPages>(
        &self,
        phase: &ReadPhase<'_, R>,
        cursor: &mut MapCursor<u64, V>,
    ) -> io::Result<Option<MapEntry<u64, V>>> {
        self.0.next(phase, cursor)
    }
}
