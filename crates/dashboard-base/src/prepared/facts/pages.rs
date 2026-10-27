use super::*;
use std::{
    any::{Any, TypeId},
    collections::BTreeMap,
    io::{Read, Write},
    rc::Rc,
};

pub trait ReadPages {
    fn open(&mut self, section: &str, range: u64) -> io::Result<Option<store::Range>>;
}
impl ReadPages for store::Writer {
    fn open(&mut self, section: &str, range: u64) -> io::Result<Option<store::Range>> {
        self.read_at_spooled(section, range)
    }
}
impl ReadPages for store::Session {
    fn open(&mut self, section: &str, range: u64) -> io::Result<Option<store::Range>> {
        self.read(section, range)
    }
}
impl ReadPages for store::Snapshot {
    fn open(&mut self, section: &str, range: u64) -> io::Result<Option<store::Range>> {
        self.read(section, range)
    }
}

pub(crate) struct Admitted<T> {
    pub value: T,
    pub _credit: Credit,
}
impl<T> std::ops::Deref for Admitted<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.value
    }
}
pub(crate) trait Page: Serialize + DeserializeOwned + Resident + 'static {}
impl<T: Serialize + DeserializeOwned + Resident + 'static> Page for T {}
type CacheKey = (String, u64, TypeId);
struct Entry {
    page: Rc<dyn Any>,
    bytes: usize,
    used: u64,
    _credit: Credit,
}
pub(crate) struct Cache<'a, R> {
    pub source: &'a mut R,
    budget: MemoryBudget,
    limit: usize,
    bytes: usize,
    clock: u64,
    pages: BTreeMap<CacheKey, Entry>,
    _credit: Credit,
}
impl<'a, R: ReadPages> Cache<'a, R> {
    pub fn new(source: &'a mut R, budget: MemoryBudget, limit: usize) -> io::Result<Self> {
        let credit = budget
            .admit(size_of::<Self>() + 12 * (size_of::<CacheKey>() + size_of::<Entry>() + 32))?;
        Ok(Self {
            _credit: credit,
            source,
            budget,
            limit,
            bytes: 0,
            clock: 0,
            pages: BTreeMap::new(),
        })
    }
    pub fn bytes(&self) -> usize {
        self.bytes
    }
    fn evict(&mut self) -> bool {
        let oldest = self
            .pages
            .iter()
            .min_by_key(|(_, entry)| entry.used)
            .map(|(key, _)| key.clone());
        let Some(key) = oldest else { return false };
        self.bytes -= self.pages.remove(&key).unwrap().bytes;
        true
    }
    pub(super) fn admit(&mut self, bytes: usize) -> io::Result<Credit> {
        loop {
            match self.budget.admit(bytes) {
                Ok(credit) => return Ok(credit),
                Err(error) if !self.evict() => return Err(error),
                Err(_) => {}
            }
        }
    }
    pub fn read<T: Page>(&mut self, section: &str, range: u64) -> io::Result<Rc<Admitted<T>>> {
        self.clock = self.clock.wrapping_add(1);
        let key = (section.to_owned(), range, TypeId::of::<T>());
        if let Some(entry) = self.pages.get_mut(&key) {
            entry.used = self.clock;
            return Rc::clone(&entry.page)
                .downcast()
                .map_err(|_| invalid("Private fact page type changed"));
        }
        let input = self
            .source
            .open(section, range)?
            .ok_or_else(|| invalid("Private fact page missing"))?;
        let encoded =
            usize::try_from(input.len()).map_err(|_| invalid("Private fact page too large"))?;
        let reserve = T::decode_budget()
            .for_encoded(encoded)?
            .checked_add(8192)
            .ok_or_else(|| invalid("Fact allocation overflow"))?;
        let mut credit = self.admit(reserve)?;
        let value: T = serde_json::from_reader(io::BufReader::new(input))?;
        let actual = size_of::<T>()
            .saturating_add(value.heap_bytes())
            .saturating_add(64);
        if actual > reserve {
            return Err(invalid("Private fact Resident decode bound was too small"));
        }
        credit.resize(actual)?;
        let page = Rc::new(Admitted {
            value,
            _credit: credit,
        });
        let metadata = key
            .0
            .capacity()
            .saturating_add(12 * (size_of::<CacheKey>() + size_of::<Entry>() + 32));
        let bytes = actual.saturating_add(metadata);
        if bytes <= self.limit {
            while self.bytes > self.limit - bytes {
                self.evict();
            }
            if let Ok(credit) = self.admit(metadata) {
                self.bytes += bytes;
                self.pages.insert(
                    key,
                    Entry {
                        page: page.clone(),
                        bytes,
                        used: self.clock,
                        _credit: credit,
                    },
                );
            }
        }
        Ok(page)
    }
}

pub(crate) fn encode<T: Serialize + ?Sized>(
    budget: &MemoryBudget,
    value: &T,
) -> io::Result<Admitted<Vec<u8>>> {
    struct Counter(usize);
    impl Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0 = self
                .0
                .checked_add(bytes.len())
                .ok_or_else(|| invalid("Fact encoding overflow"))?;
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut counter = Counter(0);
    serde_json::to_writer(&mut counter, value)?;
    let credit = budget.admit(counter.0.saturating_add(size_of::<Vec<u8>>()))?;
    struct Fixed<'a> {
        bytes: &'a mut Vec<u8>,
        limit: usize,
    }
    impl Write for Fixed<'_> {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if bytes.len() > self.limit - self.bytes.len() {
                return Err(invalid("Private fact serialization grew after admission"));
            }
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut bytes = Vec::with_capacity(counter.0);
    serde_json::to_writer(
        Fixed {
            bytes: &mut bytes,
            limit: counter.0,
        },
        value,
    )?;
    if bytes.len() != counter.0 {
        return Err(invalid("Private fact serialization is not stable"));
    }
    Ok(Admitted {
        value: bytes,
        _credit: credit,
    })
}
pub(crate) fn write(
    writer: &mut store::Writer,
    budget: &MemoryBudget,
    section: &str,
    data: impl Read,
) -> io::Result<u64> {
    let _credit = budget.admit(8192)?;
    writer.write_with_scratch(section, data, &mut [0; 8192])
}

pub(crate) fn read<T: Page>(
    source: &mut impl ReadPages,
    budget: &MemoryBudget,
    section: &str,
    range: u64,
) -> io::Result<Rc<Admitted<T>>> {
    Cache::new(source, budget.clone(), 0)?.read(section, range)
}
