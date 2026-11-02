use crate::codec::{Reader, Writer, record_len};
use crate::*;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::ops::ControlFlow;
use std::path::Path;
use std::sync::{Arc, Mutex};

fn insert_id(ids: &mut Buffer<usize>, id: usize, memory: &Memory) -> io::Result<()> {
    if let Err(index) = ids.binary_search(&id) {
        if ids.len() == ids.capacity() {
            ids.grow(
                memory,
                Owner::State,
                (ids.len() + 1).max(ids.capacity().saturating_mul(2)),
            )?;
        }
        ids.insert(index, id);
    }
    Ok(())
}

pub struct Row {
    pub address: Address,
    pub record: Record,
}

struct Dictionary {
    addresses: Buffer<Address>,
    _memory: Reservation,
}

pub struct State {
    dictionary: Arc<Dictionary>,
    selection: Arc<Selection>,
    committed: Buffer<u8>,
    pending: Buffer<u8>,
    touched: Buffer<bool>,
    rates: Buffer<usize>,
    rate_positions: Buffer<usize>,
    stride: usize,
    memory: Memory,
    _metadata: Reservation,
}

impl State {
    fn new(
        dictionary: Arc<Dictionary>,
        selection: Arc<Selection>,
        stride: usize,
        memory: &Memory,
    ) -> io::Result<Self> {
        let metadata = memory.reserve(
            Owner::State,
            std::mem::size_of::<Selection>() + 2 * std::mem::size_of::<usize>(),
        )?;
        let count = selection.len();
        let bytes = count
            .checked_mul(stride)
            .ok_or_else(|| io::Error::other("state size overflow"))?;
        let mut committed = Buffer::new(memory, Owner::State, bytes)?;
        let mut pending = Buffer::new(memory, Owner::State, bytes)?;
        for _ in 0..bytes {
            committed.push(0);
            pending.push(0);
        }
        let mut touched = Buffer::new(memory, Owner::State, count)?;
        let mut rate_positions = Buffer::new(memory, Owner::State, count)?;
        for _ in 0..count {
            touched.push(false);
            rate_positions.push(usize::MAX);
        }
        Ok(Self {
            dictionary,
            selection,
            committed,
            pending,
            touched,
            rates: Buffer::new(memory, Owner::State, count)?,
            rate_positions,
            stride,
            memory: memory.clone(),
            _metadata: metadata,
        })
    }
    fn initialize(&mut self, slot: usize, record: RecordRef<'_>) -> io::Result<()> {
        self.stage(slot, record)?;
        self.commit(std::slice::from_ref(&slot));
        self.touched[slot] = false;
        Ok(())
    }
}

impl ValueStore for State {
    fn selection(&self) -> Arc<Selection> {
        self.selection.clone()
    }
    fn address(&self, slot: usize) -> &Address {
        &self.dictionary.addresses[self.selection.id(slot)]
    }
    fn get(&self, address: &Address) -> Lookup<'_> {
        self.dictionary
            .addresses
            .binary_search(address)
            .ok()
            .and_then(|id| self.selection.slot(id))
            .map_or(Lookup::NotSelected, |slot| {
                Lookup::Record(self.record(slot, false))
            })
    }
    fn record(&self, slot: usize, staged: bool) -> RecordRef<'_> {
        let bytes = if staged && self.touched[slot] {
            &self.pending
        } else {
            &self.committed
        };
        Reader::new(&bytes[slot * self.stride..(slot + 1) * self.stride])
            .record(0)
            .unwrap()
    }
    fn load(&mut self, address: &Address) -> io::Result<Record> {
        let Lookup::Record(record) = self.get(address) else {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "unselected address",
            ));
        };
        crate::codec::owned(record, &self.memory)
    }
    fn begin(&mut self, previous: &[usize]) {
        for slot in previous {
            self.touched[*slot] = false;
        }
    }
    fn stage(&mut self, slot: usize, record: RecordRef<'_>) -> io::Result<bool> {
        if slot >= self.selection.len() || record_len(record, 0) > self.stride {
            return Err(io::Error::new(
                io::ErrorKind::OutOfMemory,
                "state columns need preparation",
            ));
        }
        Writer::new(Some(
            &mut self.pending[slot * self.stride..(slot + 1) * self.stride],
        ))
        .record(record, 0)?;
        let first = !self.touched[slot];
        self.touched[slot] = true;
        Ok(first)
    }
    fn include_active_rates(&self, changed: &mut Buffer<usize>) -> bool {
        for slot in self.rates.iter() {
            if !self.touched[*slot] && !changed.push(*slot) {
                return false;
            }
        }
        true
    }
    fn commit(&mut self, changed: &[usize]) {
        for slot in changed {
            if !self.touched[*slot] {
                continue;
            }
            let first = *slot * self.stride;
            let last = first + self.stride;
            self.committed[first..last].copy_from_slice(&self.pending[first..last]);
            let rate = matches!(self.record(*slot, false).value, ValueRef::Accumulating(_));
            let position = self.rate_positions[*slot];
            if rate && position == usize::MAX {
                self.rate_positions[*slot] = self.rates.len();
                self.rates.push(*slot);
            } else if !rate && position != usize::MAX {
                self.rates.swap_remove(position);
                self.rate_positions[*slot] = usize::MAX;
                if position < self.rates.len() {
                    self.rate_positions[self.rates[position]] = position;
                }
            }
        }
    }
    fn resident_bytes(&self) -> usize {
        self.committed.bytes()
            + self.pending.bytes()
            + self.touched.bytes()
            + self.rates.bytes()
            + self.rate_positions.bytes()
    }
    fn max_record_bytes(&self) -> usize {
        self.stride
    }
    fn active_rates(&self) -> usize {
        self.rates.len()
    }
}

pub struct MemoryPage {
    pub id: u64,
    pub hours: Hours,
    pub changes: Vec<Change>,
    pub coverage: Vec<Coverage>,
}
#[derive(Clone, Copy)]
struct EncodedRun {
    hour: Hour,
    start: usize,
    end: usize,
}
pub struct EncodedDecoder {
    runs: Buffer<EncodedRun>,
}
impl PageDecoder for EncodedDecoder {
    fn decode<'a>(
        &'a self,
        bytes: &'a [u8],
        hours: Hours,
        selection: &Selection,
        visit: &mut dyn FnMut(Hour, usize, RecordRef<'a>) -> io::Result<()>,
    ) -> io::Result<()> {
        let first = self.runs.partition_point(|run| run.hour < hours.first);
        let last = self.runs.partition_point(|run| run.hour <= hours.last);
        for run in &self.runs[first..last] {
            let mut reader = Reader::new(&bytes[run.start..run.end]);
            while reader.offset < run.end - run.start {
                let hour = Hour::try_from(reader.int()?).map_err(io::Error::other)?;
                let id = usize::try_from(reader.uint()?).map_err(io::Error::other)?;
                let len = usize::try_from(reader.uint()?).map_err(io::Error::other)?;
                let record = reader.bytes(len)?;
                if let Some(slot) = selection.slot(id) {
                    visit(hour, slot, Reader::new(record).record(hour)?)?;
                }
            }
        }
        Ok(())
    }
}

pub struct InMemory {
    baseline_hour: Hour,
    dictionary: Arc<Dictionary>,
    baseline: Buffer<Record>,
    pages: Buffer<Arc<Page<EncodedDecoder>>>,
    stride: usize,
    pub versions: std::sync::atomic::AtomicUsize,
    pub reads: std::sync::atomic::AtomicUsize,
}

impl InMemory {
    pub fn new(
        memory: &Memory,
        baseline_hour: Hour,
        mut baseline: Vec<Row>,
        mut pages: Vec<MemoryPage>,
    ) -> io::Result<Self> {
        baseline.sort_unstable_by(|a, b| a.address.cmp(&b.address));
        if baseline
            .windows(2)
            .any(|pair| pair[0].address == pair[1].address)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "duplicate version address",
            ));
        }
        let total = baseline.len() + pages.iter().map(|page| page.changes.len()).sum::<usize>();
        let mut addresses = Buffer::new(memory, Owner::Metadata, total)?;
        for row in &baseline {
            addresses.push(row.address.clone());
        }
        for page in &pages {
            for row in &page.changes {
                addresses.push(row.address.clone());
            }
        }
        addresses.as_mut_slice().sort_unstable();
        let mut unique = 0;
        for index in 0..addresses.len() {
            if unique == 0 || addresses[index] != addresses[unique - 1] {
                addresses.as_mut_slice().swap(unique, index);
                unique += 1;
            }
        }
        addresses.truncate(unique);
        let dictionary = Arc::new(Dictionary {
            addresses,
            _memory: memory.reserve(
                Owner::Metadata,
                std::mem::size_of::<Dictionary>() + 2 * std::mem::size_of::<usize>(),
            )?,
        });
        let mut initial = Buffer::new(memory, Owner::State, unique)?;
        let mut stride = record_len(
            RecordRef {
                hour: baseline_hour,
                value: ValueRef::End,
                provenance: ProvenanceRef::default(),
                break_before: false,
            },
            0,
        );
        for address in dictionary.addresses.iter() {
            let record = baseline
                .iter()
                .find(|row| &row.address == address)
                .map(|row| row.record.clone())
                .unwrap_or(Record {
                    hour: baseline_hour,
                    value: Value::End,
                    provenance: Provenance::default(),
                    break_before: false,
                });
            stride = stride.max(record_len(record.borrowed(), 0));
            initial.push(record);
        }
        let mut stored = Buffer::new(memory, Owner::Forward, pages.len())?;
        pages.sort_unstable_by_key(|page| page.id);
        for mut page in pages {
            page.changes
                .sort_unstable_by(|a, b| (a.hour, &a.address).cmp(&(b.hour, &b.address)));
            let mut count = Writer::new(None);
            for change in &page.changes {
                if !page.hours.contains(change.hour) {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "row outside page",
                    ));
                }
                stride = stride.max(record_len(change.record.borrowed(), 0));
                count.int(i64::from(change.hour))?;
                count.uint(dictionary.addresses.binary_search(&change.address).unwrap() as u64)?;
                count.uint(record_len(change.record.borrowed(), change.hour) as u64)?;
                count.record(change.record.borrowed(), change.hour)?;
            }
            let mut bytes = Buffer::new(memory, Owner::Forward, count.len)?;
            for _ in 0..count.len {
                bytes.push(0);
            }
            let run_count = page
                .changes
                .iter()
                .enumerate()
                .filter(|(i, c)| *i == 0 || page.changes[*i - 1].hour != c.hour)
                .count();
            let mut runs = Buffer::new(memory, Owner::Forward, run_count)?;
            let mut writer = Writer::new(Some(&mut bytes));
            for change in &page.changes {
                if runs
                    .last()
                    .is_none_or(|run: &EncodedRun| run.hour != change.hour)
                {
                    runs.push(EncodedRun {
                        hour: change.hour,
                        start: writer.len,
                        end: writer.len,
                    });
                }
                writer.int(i64::from(change.hour))?;
                writer.uint(dictionary.addresses.binary_search(&change.address).unwrap() as u64)?;
                writer.uint(record_len(change.record.borrowed(), change.hour) as u64)?;
                writer.record(change.record.borrowed(), change.hour)?;
                runs.last_mut().unwrap().end = writer.len;
            }
            let mut coverage = Buffer::new(memory, Owner::Forward, page.coverage.len())?;
            for item in page.coverage {
                coverage.push(item);
            }
            let info = PageInfo {
                id: page.id,
                hours: page.hours,
                records: page.changes.len(),
                coverage: coverage.len(),
                resident_bytes: bytes.bytes()
                    + coverage.bytes()
                    + runs.bytes()
                    + Bytes::allocation_bytes(),
            };
            let page = Page {
                info,
                bytes: Bytes::from_buffer(memory, Owner::Forward, bytes)?,
                decoder: EncodedDecoder { runs },
                coverage,
                backing: None,
            };
            page.validate()?;
            stored.push(Arc::new(page));
        }
        Ok(Self {
            baseline_hour,
            dictionary,
            baseline: initial,
            pages: stored,
            stride,
            versions: Default::default(),
            reads: Default::default(),
        })
    }
    fn restore(
        &self,
        keys: &KeyFilter,
        hour: Hour,
        before: Option<u64>,
        scope: Hours,
        memory: &Memory,
    ) -> io::Result<State> {
        let mut ids = Buffer::new(memory, Owner::State, 0)?;
        match keys {
            KeyFilter::Keys(keys) => {
                ids.grow(memory, Owner::State, keys.len())?;
                for key in keys.iter() {
                    ids.push(self.dictionary.addresses.binary_search(key).map_err(|_| {
                        io::Error::new(io::ErrorKind::InvalidInput, "unknown filter key")
                    })?);
                }
            }
            KeyFilter::All => {
                for (id, record) in self.baseline.iter().enumerate() {
                    if !matches!(record.value, Value::End) {
                        insert_id(&mut ids, id, memory)?;
                    }
                }
                for page in self.pages.iter().take_while(|page| {
                    page.info.hours.first <= hour && before.is_none_or(|id| page.info.id < id)
                }) {
                    page.decoder.decode(
                        &page.bytes,
                        Hours {
                            first: self.baseline_hour.saturating_add(1),
                            last: hour,
                        },
                        &Selection::All { len: usize::MAX },
                        &mut |_, id, record| {
                            if matches!(record.value, ValueRef::End) {
                                if let Ok(i) = ids.binary_search(&id) {
                                    ids.remove(i);
                                }
                            } else {
                                insert_id(&mut ids, id, memory)?;
                            }
                            Ok(())
                        },
                    )?;
                }
                for page in self
                    .pages
                    .iter()
                    .filter(|page| page.info.hours.intersects(scope))
                {
                    page.decoder.decode(
                        &page.bytes,
                        scope,
                        &Selection::All { len: usize::MAX },
                        &mut |_, id, _| insert_id(&mut ids, id, memory),
                    )?;
                }
            }
        }
        let selection = Arc::new(Selection::Ids(ids));
        let mut state = State::new(self.dictionary.clone(), selection, self.stride, memory)?;
        for slot in 0..state.selection.len() {
            state.initialize(slot, self.baseline[state.selection.id(slot)].borrowed())?;
        }
        Ok(state)
    }
    fn replay(
        &self,
        state: &mut State,
        hour: Hour,
        before: Option<u64>,
        memory: &Memory,
    ) -> io::Result<()> {
        let selection = state.selection();
        let mut changed = Buffer::new(memory, Owner::Transient, selection.len())?;
        for page in self.pages.iter().take_while(|page| {
            page.info.hours.first <= hour && before.is_none_or(|id| page.info.id < id)
        }) {
            state.begin(&changed);
            changed.clear();
            page.decoder.decode(
                &page.bytes,
                Hours {
                    first: self.baseline_hour.saturating_add(1),
                    last: hour,
                },
                &selection,
                &mut |_, slot, record| {
                    if state.stage(slot, record)? {
                        changed.push(slot);
                    }
                    Ok(())
                },
            )?;
            state.commit(&changed);
        }
        state.begin(&changed);
        Ok(())
    }
}
impl LogSource for InMemory {
    type State = State;
    type Decoder = EncodedDecoder;
    fn version_at(
        &self,
        keys: &KeyFilter,
        hour: Hour,
        scope: Hours,
        memory: &Memory,
    ) -> io::Result<State> {
        self.versions
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let mut state = self.restore(keys, hour, None, scope, memory)?;
        self.replay(&mut state, hour, None, memory)?;
        Ok(state)
    }
    fn version_before_page(
        &self,
        keys: &KeyFilter,
        page: PageInfo,
        scope: Hours,
        memory: &Memory,
    ) -> io::Result<State> {
        self.versions
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let mut state = self.restore(keys, page.hours.last, Some(page.id), scope, memory)?;
        self.replay(&mut state, page.hours.last, Some(page.id), memory)?;
        Ok(state)
    }
    fn checkpoint_run(&self, _page: PageInfo) -> u64 {
        0
    }
    fn pages(
        &self,
        _keys: &KeyFilter,
        hours: Hours,
        visit: &mut dyn FnMut(PageInfo) -> ControlFlow<()>,
    ) -> io::Result<()> {
        for page in self
            .pages
            .iter()
            .filter(|page| page.info.hours.intersects(hours))
        {
            if visit(page.info).is_break() {
                break;
            }
        }
        Ok(())
    }
    fn read_page(&self, info: PageInfo, memory: &Memory) -> io::Result<Page<EncodedDecoder>> {
        self.reads
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let page = self
            .pages
            .iter()
            .find(|page| page.info == info)
            .ok_or_else(|| io::Error::other("unknown page"))?;
        let mut coverage = Buffer::new(memory, Owner::Forward, page.coverage.len())?;
        for item in page.coverage.iter() {
            coverage.push(item.clone());
        }
        let mut runs = Buffer::new(memory, Owner::Forward, page.decoder.runs.len())?;
        for run in page.decoder.runs.iter() {
            runs.push(*run);
        }
        Ok(Page {
            info,
            bytes: Bytes::copy_from(memory, Owner::Forward, &page.bytes)?,
            decoder: EncodedDecoder { runs },
            coverage,
            backing: None,
        })
    }
    fn coverage(
        &self,
        hours: Hours,
        visit: &mut dyn FnMut(Coverage) -> ControlFlow<()>,
    ) -> io::Result<()> {
        for page in self.pages.iter() {
            for item in page.coverage.iter() {
                if let Some(item) = clip(item, hours) {
                    if visit(item).is_break() {
                        return Ok(());
                    }
                }
            }
        }
        Ok(())
    }
}

pub(crate) fn clip(item: &Coverage, hours: Hours) -> Option<Coverage> {
    let span = item.hours.intersection(hours)?;
    let mut next = item.clone();
    next.accepted_before = if span.first == item.hours.first {
        item.accepted_before
    } else {
        item.accepted_through(span.first - 1)
    };
    next.hours = span;
    Some(next)
}

#[derive(Clone, Copy)]
struct NumericCell {
    hour: Hour,
    number: f64,
    end: bool,
}
pub struct NumericState {
    dictionary: Arc<Dictionary>,
    selection: Arc<Selection>,
    extra: Arc<Buffer<Address>>,
    static_slots: usize,
    committed: Buffer<NumericCell>,
    pending: Buffer<NumericCell>,
    touched: Buffer<bool>,
    memory: Memory,
    max_record_bytes: usize,
    _metadata: Reservation,
}
impl NumericState {
    fn new(
        dictionary: Arc<Dictionary>,
        selection: Selection,
        memory: &Memory,
        max_record_bytes: usize,
    ) -> io::Result<Self> {
        let metadata = memory.reserve(
            Owner::State,
            std::mem::size_of::<Selection>() + 2 * std::mem::size_of::<usize>(),
        )?;
        let count = selection.len();
        let static_slots = (0..count)
            .take_while(|slot| selection.id(*slot) < dictionary.addresses.len())
            .count();
        let mut extra = Buffer::new(memory, Owner::State, count - static_slots)?;
        let packed = packed_keys(
            (static_slots..count).map(|slot| selection.id(slot)),
            memory,
            Owner::State,
        )?;
        let mut offset = 0;
        for slot in static_slots..count {
            let len = var_len(selection.id(slot) as u64);
            extra.push(Address {
                key: Key::shared_varint(packed.slice(offset, len)?),
                field: dictionary.addresses[0].field.clone(),
            });
            offset += len;
        }
        let extra_metadata = memory.reserve(
            Owner::State,
            std::mem::size_of::<Buffer<Address>>() + 2 * std::mem::size_of::<usize>(),
        )?;
        let extra = Arc::new(extra);
        let selection = selection.with_source(
            ShapeKeys {
                dictionary: dictionary.clone(),
                extra: extra.clone(),
                static_slots,
                _memory: extra_metadata,
            },
            memory,
        )?;
        let mut committed = Buffer::new(memory, Owner::State, count)?;
        let mut pending = Buffer::new(memory, Owner::State, count)?;
        let mut touched = Buffer::new(memory, Owner::State, count)?;
        for _ in 0..count {
            let cell = NumericCell {
                hour: -1,
                number: 0.0,
                end: true,
            };
            committed.push(cell);
            pending.push(cell);
            touched.push(false);
        }
        Ok(Self {
            dictionary,
            selection: Arc::new(selection),
            extra,
            static_slots,
            committed,
            pending,
            touched,
            memory: memory.clone(),
            max_record_bytes,
            _metadata: metadata,
        })
    }
    fn initialize(&mut self, slot: usize, record: RecordRef<'_>) -> io::Result<()> {
        self.stage(slot, record)?;
        self.committed[slot] = self.pending[slot];
        self.touched[slot] = false;
        Ok(())
    }
}
impl ValueStore for NumericState {
    fn selection(&self) -> Arc<Selection> {
        self.selection.clone()
    }
    fn address(&self, slot: usize) -> &Address {
        if slot < self.static_slots {
            &self.dictionary.addresses[self.selection.id(slot)]
        } else {
            &self.extra[slot - self.static_slots]
        }
    }
    fn get(&self, address: &Address) -> Lookup<'_> {
        let mut id = 0usize;
        for (i, byte) in address.key.varint().iter().enumerate() {
            if i * 7 >= usize::BITS as usize {
                return Lookup::NotSelected;
            }
            let Some(part) = usize::from(byte & 127).checked_shl((i * 7) as u32) else {
                return Lookup::NotSelected;
            };
            if usize::from(byte & 127) > (usize::MAX >> (i * 7)) {
                return Lookup::NotSelected;
            }
            id |= part;
        }
        if address.field != self.dictionary.addresses[0].field {
            return Lookup::NotSelected;
        }
        self.selection.slot(id).map_or(Lookup::NotSelected, |slot| {
            Lookup::Record(self.record(slot, false))
        })
    }
    fn record(&self, slot: usize, staged: bool) -> RecordRef<'_> {
        let cell = if staged && self.touched[slot] {
            self.pending[slot]
        } else {
            self.committed[slot]
        };
        numeric(cell)
    }
    fn load(&mut self, address: &Address) -> io::Result<Record> {
        let Lookup::Record(record) = self.get(address) else {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "unselected address",
            ));
        };
        crate::codec::owned(record, &self.memory)
    }
    fn begin(&mut self, previous: &[usize]) {
        for slot in previous {
            self.touched[*slot] = false;
        }
    }
    fn stage(&mut self, slot: usize, record: RecordRef<'_>) -> io::Result<bool> {
        let (number, end) = match record.value {
            ValueRef::Number(n) => (n, false),
            ValueRef::End => (0.0, true),
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "unexpected numeric schema",
                ));
            }
        };
        self.pending[slot] = NumericCell {
            hour: record.hour,
            number,
            end,
        };
        let first = !self.touched[slot];
        self.touched[slot] = true;
        Ok(first)
    }
    fn include_active_rates(&self, _changed: &mut Buffer<usize>) -> bool {
        true
    }
    fn commit(&mut self, changed: &[usize]) {
        for slot in changed {
            self.committed[*slot] = self.pending[*slot];
        }
    }
    fn resident_bytes(&self) -> usize {
        self.committed.bytes() + self.pending.bytes() + self.touched.bytes() + self.extra.bytes()
    }
    fn max_record_bytes(&self) -> usize {
        self.max_record_bytes
    }
    fn active_rates(&self) -> usize {
        0
    }
}
fn numeric(cell: NumericCell) -> RecordRef<'static> {
    RecordRef {
        hour: cell.hour,
        value: if cell.end {
            ValueRef::End
        } else {
            ValueRef::Number(cell.number)
        },
        provenance: ProvenanceRef::default(),
        break_before: false,
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Shape {
    pub hours: Hour,
    pub keys: usize,
    pub changes_per_tick: usize,
    pub tick_hours: Hour,
    pub lifetimes: bool,
    pub live_units: usize,
    pub churn_per_tick: usize,
}
#[derive(Clone, Copy)]
struct ShapeRun {
    hour: Hour,
    first: usize,
    last: usize,
}
struct ShapeKeys {
    dictionary: Arc<Dictionary>,
    extra: Arc<Buffer<Address>>,
    static_slots: usize,
    _memory: Reservation,
}
impl ShapeKeys {
    fn key(&self, selection: &Selection, slot: usize) -> &Key {
        if slot < self.static_slots {
            &self.dictionary.addresses[selection.id(slot)].key
        } else {
            &self.extra[slot - self.static_slots].key
        }
    }
}
fn packed_keys(
    ids: impl Iterator<Item = usize> + Clone,
    memory: &Memory,
    owner: Owner,
) -> io::Result<Bytes> {
    let capacity = ids.clone().try_fold(0usize, |bytes, id| {
        bytes
            .checked_add(var_len(id as u64))
            .ok_or_else(|| io::Error::other("key storage overflow"))
    })?;
    let mut packed = Buffer::new(memory, owner, capacity)?;
    for id in ids {
        let mut value = id as u64;
        loop {
            let mut byte = (value & 127) as u8;
            value >>= 7;
            if value != 0 {
                byte |= 128;
            }
            packed.push(byte);
            if value == 0 {
                break;
            }
        }
    }
    Bytes::from_buffer(memory, owner, packed)
}
pub struct ShapeDecoder {
    runs: Buffer<ShapeRun>,
    columns: [usize; 13],
    index: usize,
}
fn var_len(value: u64) -> usize {
    ((64 - value.leading_zeros()).max(1) as usize).div_ceil(7)
}
fn var_at(bytes: &[u8], offset: usize) -> &[u8] {
    let end = bytes[offset..]
        .iter()
        .position(|byte| byte & 128 == 0)
        .unwrap()
        + offset
        + 1;
    &bytes[offset..end]
}
fn id_cmp(a: &[u8], b: &[u8]) -> std::cmp::Ordering {
    a.len().cmp(&b.len()).then_with(|| {
        a.iter()
            .rev()
            .map(|b| b & 127)
            .cmp(b.iter().rev().map(|b| b & 127))
    })
}
fn gallop(mut first: usize, last: usize, mut before: impl FnMut(usize) -> bool) -> usize {
    if first >= last || !before(first) {
        return first;
    }
    let origin = first;
    let mut step = 1usize;
    while origin.saturating_add(step) < last && before(origin + step) {
        first = origin + step;
        step = step.saturating_mul(2);
    }
    let mut end = origin.saturating_add(step).min(last);
    first += 1;
    while first < end {
        let mid = first + (end - first) / 2;
        if before(mid) {
            first = mid + 1;
        } else {
            end = mid;
        }
    }
    first
}
impl ShapeDecoder {
    fn offset(&self, bytes: &[u8], row: usize, field: usize) -> usize {
        let at = self.index + row * 17 + 1 + field * 4;
        u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()) as usize
    }
    fn key<'a>(&self, bytes: &'a [u8], row: usize) -> &'a [u8] {
        var_at(bytes, self.columns[1] + self.offset(bytes, row, 0))
    }
}
impl PageDecoder for ShapeDecoder {
    fn decode<'a>(
        &'a self,
        bytes: &'a [u8],
        hours: Hours,
        selection: &Selection,
        visit: &mut dyn FnMut(Hour, usize, RecordRef<'a>) -> io::Result<()>,
    ) -> io::Result<()> {
        let keys = selection.source::<ShapeKeys>();
        let compare = |row: usize, slot: usize| {
            if let Some(keys) = keys {
                id_cmp(self.key(bytes, row), keys.key(selection, slot).varint())
            } else {
                (Reader::new(self.key(bytes, row)).uint().unwrap() as usize)
                    .cmp(&selection.id(slot))
            }
        };
        let first = self.runs.partition_point(|run| run.hour < hours.first);
        let last = self.runs.partition_point(|run| run.hour <= hours.last);
        for run in &self.runs[first..last] {
            let mut emit = |row: usize, slot: usize| {
                let end = bytes[self.columns[3] + row] == 6;
                let number = if end {
                    0.0
                } else {
                    let at = self.columns[7] + self.offset(bytes, row, 3);
                    let mut input = Reader::new(&bytes[at..self.columns[8]]);
                    let storage = input.uint()?;
                    let length = input.uint()?;
                    if storage != 0 || length == 0 {
                        return Err(io::Error::other("invalid inline numeric payload"));
                    }
                    let encoded = input.uint()?;
                    let value = ((encoded >> 1) as i64) ^ -((encoded & 1) as i64);
                    value as f64 / 8.0
                };
                visit(
                    run.hour,
                    slot,
                    numeric(NumericCell {
                        hour: run.hour,
                        number,
                        end,
                    }),
                )
            };
            if selection.len() < run.last - run.first {
                let mut row = run.first;
                for slot in 0..selection.len() {
                    row = gallop(row, run.last, |row| compare(row, slot).is_lt());
                    if row == run.last {
                        break;
                    }
                    if compare(row, slot).is_eq() {
                        emit(row, slot)?;
                        row += 1;
                    }
                }
            } else {
                let mut slot = 0;
                for row in run.first..run.last {
                    slot = gallop(slot, selection.len(), |slot| compare(row, slot).is_gt());
                    if slot == selection.len() {
                        break;
                    }
                    if compare(row, slot).is_eq() {
                        emit(row, slot)?;
                    }
                }
            }
        }
        Ok(())
    }
}
fn gcd(mut a: usize, mut b: usize) -> usize {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}
fn layout(shape: Shape) -> (usize, usize, usize) {
    let changes = shape.changes_per_tick - 2 * shape.churn_per_tick;
    let keys = if shape.lifetimes {
        (shape.keys - shape.live_units) / changes * changes
    } else {
        shape.keys
    };
    let groups = gcd(keys, changes);
    (keys / groups, changes / groups, changes)
}
fn static_key(shape: Shape, tick: u64, row: usize) -> usize {
    let (width, count, _) = layout(shape);
    let phase = tick as usize % width;
    let wrap = (phase + count).saturating_sub(width);
    let within = row % count;
    row / count * width
        + if within < wrap {
            within
        } else {
            phase + within - wrap
        }
}
fn rank(shape: Shape, tick: u64, key: usize) -> Option<usize> {
    let (width, count, changes) = layout(shape);
    if key >= changes / count * width {
        return None;
    }
    let phase = tick as usize % width;
    let wrap = (phase + count).saturating_sub(width);
    let within = key % width;
    let local = if within < wrap {
        within
    } else if within >= phase && within < phase + count {
        wrap + within - phase
    } else {
        return None;
    };
    Some(key / width * count + local)
}
fn last_tick(shape: Shape, key: usize, tick: i64) -> Option<i64> {
    let period = layout(shape).0 as i64;
    for distance in 0..period {
        let at = tick - distance;
        if at >= 0 && rank(shape, at as u64, key).is_some() {
            return Some(at);
        }
    }
    None
}
fn checkpoint_period(shape: Shape) -> usize {
    let period = layout(shape).0;
    if period.is_multiple_of(2) {
        period
    } else {
        period * 2
    }
}
fn value_at(shape: Shape, key: usize, tick: i64) -> f64 {
    key as f64 * 0.125 + tick.rem_euclid(checkpoint_period(shape) as i64) as f64 * 0.25
}
fn cell_bytes(cell: NumericCell) -> [u8; 13] {
    let mut bytes = [0; 13];
    bytes[..4].copy_from_slice(&cell.hour.to_le_bytes());
    bytes[4..12].copy_from_slice(&cell.number.to_le_bytes());
    bytes[12] = u8::from(cell.end);
    bytes
}
fn cell_from(bytes: &[u8]) -> NumericCell {
    NumericCell {
        hour: Hour::from_le_bytes(bytes[..4].try_into().unwrap()),
        number: f64::from_le_bytes(bytes[4..12].try_into().unwrap()),
        end: bytes[12] != 0,
    }
}
const ROOT_BYTES: u64 = 16;
const CHECKPOINT_CELL: usize = 34;
pub struct ShapedLog {
    shape: Shape,
    dictionary: Arc<Dictionary>,
    checkpoint: Mutex<File>,
    rows_per_page: u64,
    roots: u64,
    width: usize,
    pub checkpoint_reads: std::sync::atomic::AtomicUsize,
}
impl ShapedLog {
    pub fn create(shape: Shape, memory: &Memory, path: &Path) -> io::Result<Self> {
        if shape.hours <= 0
            || shape.keys == 0
            || shape.changes_per_tick == 0
            || shape.changes_per_tick > shape.keys
            || shape.tick_hours <= 0
            || (!shape.lifetimes && (shape.live_units != 0 || shape.churn_per_tick != 0))
            || (shape.lifetimes
                && (shape.churn_per_tick == 0
                    || shape.live_units < shape.churn_per_tick
                    || shape.live_units >= shape.keys
                    || shape.changes_per_tick <= 2 * shape.churn_per_tick
                    || 168 % shape.tick_hours != 0))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid synthetic shape",
            ));
        }
        let mut addresses = Buffer::new(memory, Owner::Metadata, shape.keys)?;
        let field = Key::from_u64_in(memory, 0)?;
        let packed = packed_keys(0..shape.keys, memory, Owner::Metadata)?;
        let mut offset = 0;
        for id in 0..shape.keys {
            let len = var_len(id as u64);
            addresses.push(Address {
                key: Key::shared_varint(packed.slice(offset, len)?),
                field: field.clone(),
            });
            offset += len;
        }
        let dictionary = Arc::new(Dictionary {
            addresses,
            _memory: memory.reserve(
                Owner::Metadata,
                std::mem::size_of::<Dictionary>() + 2 * std::mem::size_of::<usize>(),
            )?,
        });
        let max_id = shape
            .keys
            .checked_add(if shape.lifetimes {
                shape.hours as usize * shape.churn_per_tick
            } else {
                0
            })
            .ok_or_else(|| io::Error::other("shape size overflow"))?;
        let width = ((usize::BITS - max_id.leading_zeros()) as usize)
            .div_ceil(7)
            .max(1);
        let payload = var_len((max_id as u64 + checkpoint_period(shape) as u64 * 2) * 2) + 2;
        let bound = (256 * 1024 - 128) / (width + 4 + payload);
        let rows_per_page = if shape.lifetimes {
            let max_ticks = (bound / shape.changes_per_tick)
                .max(1)
                .min(168 / shape.tick_hours as usize);
            let ticks = (1..=max_ticks)
                .rev()
                .find(|ticks| (168 / shape.tick_hours as usize).is_multiple_of(*ticks))
                .unwrap();
            (ticks * shape.changes_per_tick) as u64
        } else {
            bound.min((168 / shape.tick_hours as usize).max(1) * shape.changes_per_tick) as u64
        };
        let total =
            ((shape.hours - 1) / shape.tick_hours + 1) as u64 * shape.changes_per_tick as u64;
        let roots = if shape.lifetimes {
            (shape.hours as u64).div_ceil(168)
        } else {
            total.div_ceil(rows_per_page)
        };
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(path)?;
        for version in 0..roots {
            let row = if shape.lifetimes {
                version * 168 / shape.tick_hours as u64 * shape.changes_per_tick as u64
            } else {
                version * rows_per_page
            };
            file.write_all(&row.to_le_bytes())?;
            let tick = row / shape.changes_per_tick as u64;
            let unit =
                shape.keys as u64 - shape.live_units as u64 + tick * shape.churn_per_tick as u64;
            file.write_all(&unit.to_le_bytes())?;
        }
        let static_keys = shape.keys - shape.live_units;
        let period = checkpoint_period(shape);
        let mut block = Buffer::new(memory, Owner::Transient, static_keys * CHECKPOINT_CELL)?;
        for _ in 0..static_keys * CHECKPOINT_CELL {
            block.push(0);
        }
        for phase in 0..period {
            let tick = (period * 2 + phase) as i64;
            for key in 0..static_keys {
                let at = key * CHECKPOINT_CELL;
                for (offset, cut) in [(0, tick - 1), (13, tick)] {
                    let last = last_tick(shape, key, cut);
                    let cell = NumericCell {
                        hour: last.map_or(Hour::MIN, |last| {
                            ((last - tick) * i64::from(shape.tick_hours)) as Hour
                        }),
                        number: last.map_or(key as f64 * 0.125, |last| value_at(shape, key, last)),
                        end: false,
                    };
                    block[at + offset..at + offset + 13].copy_from_slice(&cell_bytes(cell));
                }
                block[at + 26..at + 34].copy_from_slice(
                    &(rank(shape, tick as u64, key).map_or(u64::MAX, |row| row as u64))
                        .to_le_bytes(),
                );
            }
            file.write_all(&block)?;
        }
        if shape.lifetimes {
            let ticks = ((shape.hours - 1) / shape.tick_hours + 1) as usize;
            for offset in 0..shape.live_units + ticks * shape.churn_per_tick {
                let id = static_keys + offset;
                let born = if offset < shape.live_units {
                    -1
                } else {
                    ((offset - shape.live_units) / shape.churn_per_tick) as Hour * shape.tick_hours
                };
                file.write_all(&cell_bytes(NumericCell {
                    hour: born,
                    number: if born < 0 {
                        id as f64 * 0.125
                    } else {
                        value_at(shape, id, i64::from(born / shape.tick_hours))
                    },
                    end: false,
                }))?;
                file.write_all(&cell_bytes(NumericCell {
                    hour: (offset / shape.churn_per_tick) as Hour * shape.tick_hours,
                    number: 0.0,
                    end: true,
                }))?;
            }
        }
        file.flush()?;
        Ok(Self {
            shape,
            dictionary,
            checkpoint: Mutex::new(file),
            rows_per_page,
            roots,
            width,
            checkpoint_reads: Default::default(),
        })
    }
    pub fn shape(&self) -> Shape {
        self.shape
    }
    pub fn addresses(&self) -> &[Address] {
        &self.dictionary.addresses
    }
    pub fn filter(&self, count: usize) -> io::Result<KeyFilter> {
        KeyFilter::sorted(
            (0..count)
                .map(|slot| self.dictionary.addresses[slot * self.shape.keys / count].clone())
                .collect::<Vec<_>>()
                .into(),
        )
    }
    fn selection(
        &self,
        keys: &KeyFilter,
        hour: Hour,
        scope: Hours,
        memory: &Memory,
    ) -> io::Result<Selection> {
        match keys {
            KeyFilter::Keys(keys) => {
                let mut ids = Buffer::new(memory, Owner::State, keys.len())?;
                for key in keys.iter() {
                    ids.push(self.dictionary.addresses.binary_search(key).map_err(|_| {
                        io::Error::new(io::ErrorKind::InvalidInput, "unknown filter key")
                    })?);
                }
                Ok(Selection::Ids(ids))
            }
            KeyFilter::All if !self.shape.lifetimes => Ok(Selection::All {
                len: self.shape.keys,
            }),
            KeyFilter::All => {
                let first = scope.first.max(0).min(hour) / self.shape.tick_hours;
                let last = scope.last.min(self.shape.hours - 1).max(hour) / self.shape.tick_hours;
                let mut ids = Buffer::new(
                    memory,
                    Owner::State,
                    self.shape.keys + (last - first + 1) as usize * self.shape.churn_per_tick,
                )?;
                for key in 0..self.shape.keys - self.shape.live_units {
                    ids.push(key);
                }
                for key in self.shape.keys - self.shape.live_units
                    + first as usize * self.shape.churn_per_tick
                    ..self.shape.keys + (last as usize + 1) * self.shape.churn_per_tick
                {
                    ids.push(key);
                }
                Ok(Selection::Ids(ids))
            }
        }
    }
    fn info(&self, id: u64) -> PageInfo {
        let total = ((self.shape.hours - 1) / self.shape.tick_hours + 1) as u64
            * self.shape.changes_per_tick as u64;
        let first = id * self.rows_per_page;
        let records = (total - first).min(self.rows_per_page) as usize;
        let first_tick = first / self.shape.changes_per_tick as u64;
        let last_tick = (first + records as u64 - 1) / self.shape.changes_per_tick as u64;
        let runs = (last_tick - first_tick + 1) as usize;
        PageInfo {
            id,
            hours: Hours {
                first: first_tick as Hour * self.shape.tick_hours,
                last: ((last_tick as Hour + 1) * self.shape.tick_hours - 1)
                    .min(self.shape.hours - 1),
            },
            records,
            coverage: 1,
            resident_bytes: records
                * (self.width
                    + 4
                    + 2
                    + var_len(
                        ((self.shape.keys + self.shape.hours as usize * self.shape.churn_per_tick)
                            as u64
                            + checkpoint_period(self.shape) as u64 * 2)
                            * 2,
                    )
                    + 17)
                + 128
                + std::mem::size_of::<Coverage>()
                + runs * std::mem::size_of::<ShapeRun>()
                + Bytes::allocation_bytes(),
        }
    }
    fn restore_cut(
        &self,
        keys: &KeyFilter,
        hour: Hour,
        before: Option<u64>,
        scope: Hours,
        memory: &Memory,
    ) -> io::Result<NumericState> {
        if hour < 0 || hour >= self.shape.hours {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "hour outside synthetic log",
            ));
        }
        let version = if self.shape.lifetimes {
            hour as u64 / 168
        } else {
            before.unwrap_or(
                (((hour / self.shape.tick_hours) as u64 + 1) * self.shape.changes_per_tick as u64
                    - 1)
                    / self.rows_per_page,
            )
        };
        let selection = self.selection(keys, hour, scope, memory)?;
        let stride = record_len(
            numeric(NumericCell {
                hour: self.shape.hours,
                number: 0.0,
                end: false,
            }),
            0,
        );
        let mut state = NumericState::new(self.dictionary.clone(), selection, memory, stride)?;
        let selection = state.selection();
        let mut root = [0; ROOT_BYTES as usize];
        let mut file = self
            .checkpoint
            .lock()
            .map_err(|_| io::Error::other("checkpoint lock poisoned"))?;
        file.seek(SeekFrom::Start(version * ROOT_BYTES))?;
        file.read_exact(&mut root)?;
        let first_row = u64::from_le_bytes(root[..8].try_into().unwrap());
        let tick = first_row / self.shape.changes_per_tick as u64;
        let within = first_row % self.shape.changes_per_tick as u64;
        let phase = tick as usize % checkpoint_period(self.shape);
        let static_keys = self.shape.keys - self.shape.live_units;
        let mut block = Buffer::new(
            memory,
            Owner::Transient,
            selection.len().min(static_keys) * CHECKPOINT_CELL,
        )?;
        for _ in 0..selection.len().min(static_keys) * CHECKPOINT_CELL {
            block.push(0);
        }
        let mut slot = 0;
        while slot < selection.len() && selection.id(slot) < static_keys {
            let first = slot;
            slot += 1;
            while slot < selection.len()
                && selection.id(slot) < static_keys
                && selection.id(slot) == selection.id(slot - 1) + 1
            {
                slot += 1;
            }
            let offset = self.roots * ROOT_BYTES
                + ((phase * static_keys + selection.id(first)) * CHECKPOINT_CELL) as u64;
            file.seek(SeekFrom::Start(offset))?;
            file.read_exact(&mut block[first * CHECKPOINT_CELL..slot * CHECKPOINT_CELL])?;
        }
        let unit = u64::from_le_bytes(root[8..16].try_into().unwrap()) as usize;
        let past_first = (0..selection.len())
            .find(|slot| selection.id(*slot) >= static_keys)
            .unwrap_or(selection.len());
        let mut lifetimes = Buffer::new(
            memory,
            Owner::Transient,
            (selection.len() - past_first) * 26,
        )?;
        for _ in 0..lifetimes.capacity() {
            lifetimes.push(0);
        }
        if self.shape.lifetimes && past_first < selection.len() {
            let columns = self.roots * ROOT_BYTES
                + (checkpoint_period(self.shape) * static_keys * CHECKPOINT_CELL) as u64;
            let mut slot = past_first;
            while slot < selection.len() {
                let first = slot;
                slot += 1;
                while slot < selection.len() && selection.id(slot) == selection.id(slot - 1) + 1 {
                    slot += 1;
                }
                file.seek(SeekFrom::Start(
                    columns + ((selection.id(first) - static_keys) * 26) as u64,
                ))?;
                file.read_exact(
                    &mut lifetimes[(first - past_first) * 26..(slot - past_first) * 26],
                )?;
            }
        }
        drop(file);
        self.checkpoint_reads
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        for slot in 0..selection.len() {
            let id = selection.id(slot);
            let cell = if id < static_keys {
                let at = slot * CHECKPOINT_CELL;
                let ordinal = u64::from_le_bytes(block[at + 26..at + 34].try_into().unwrap());
                let offset = if ordinal < within { 13 } else { 0 };
                let mut cell = cell_from(&block[at + offset..at + offset + 13]);
                cell.hour = if cell.hour == Hour::MIN {
                    -1
                } else {
                    (tick as i64 * i64::from(self.shape.tick_hours) + i64::from(cell.hour)) as Hour
                };
                if cell.hour < 0 {
                    cell.hour = -1;
                    cell.number = id as f64 * 0.125;
                }
                cell
            } else if self.shape.lifetimes && id < unit + self.shape.live_units {
                let at = (slot - past_first) * 26 + if id < unit { 13 } else { 0 };
                cell_from(&lifetimes[at..at + 13])
            } else {
                NumericCell {
                    hour: -1,
                    number: 0.0,
                    end: true,
                }
            };
            state.initialize(slot, numeric(cell))?;
        }
        let last_row =
            ((hour / self.shape.tick_hours) as u64 + 1) * self.shape.changes_per_tick as u64 - 1;
        let mut changed = Buffer::new(memory, Owner::Transient, selection.len())?;
        for id in first_row / self.rows_per_page..=last_row / self.rows_per_page {
            if before.is_some_and(|limit| id >= limit) {
                break;
            }
            let page = self.read_page(self.info(id), memory)?;
            state.begin(&changed);
            changed.clear();
            page.decoder.decode(
                &page.bytes,
                Hours {
                    first: tick as Hour * self.shape.tick_hours,
                    last: hour,
                },
                &selection,
                &mut |_, slot, record| {
                    if state.stage(slot, record)? {
                        changed.push(slot);
                    }
                    Ok(())
                },
            )?;
            state.commit(&changed);
        }
        state.begin(&changed);
        Ok(state)
    }
}
impl LogSource for ShapedLog {
    type State = NumericState;
    type Decoder = ShapeDecoder;
    fn version_at(
        &self,
        keys: &KeyFilter,
        hour: Hour,
        scope: Hours,
        memory: &Memory,
    ) -> io::Result<NumericState> {
        self.restore_cut(keys, hour, None, scope, memory)
    }
    fn version_before_page(
        &self,
        keys: &KeyFilter,
        page: PageInfo,
        scope: Hours,
        memory: &Memory,
    ) -> io::Result<NumericState> {
        self.restore_cut(keys, page.hours.first, Some(page.id), scope, memory)
    }
    fn checkpoint_run(&self, page: PageInfo) -> u64 {
        if self.shape.lifetimes {
            page.hours.first as u64 / 168
        } else {
            page.id
        }
    }
    fn pages(
        &self,
        _keys: &KeyFilter,
        hours: Hours,
        visit: &mut dyn FnMut(PageInfo) -> ControlFlow<()>,
    ) -> io::Result<()> {
        let Some(span) = hours.intersection(Hours {
            first: 0,
            last: self.shape.hours - 1,
        }) else {
            return Ok(());
        };
        let first =
            (span.first / self.shape.tick_hours) as u64 * self.shape.changes_per_tick as u64;
        let last = ((span.last / self.shape.tick_hours) as u64 + 1)
            * self.shape.changes_per_tick as u64
            - 1;
        for id in first / self.rows_per_page..=last / self.rows_per_page {
            if visit(self.info(id)).is_break() {
                break;
            }
        }
        Ok(())
    }
    fn read_page(&self, info: PageInfo, memory: &Memory) -> io::Result<Page<ShapeDecoder>> {
        if self.info(info.id) != info {
            return Err(io::Error::other("unknown shaped page"));
        }
        let first = info.id * self.rows_per_page;
        let last = first + info.records as u64;
        let c = self.shape.changes_per_tick as u64;
        let row_data = |row: u64| {
            let tick = row / c;
            let ordinal = (row % c) as usize;
            let static_changes = self.shape.changes_per_tick - 2 * self.shape.churn_per_tick;
            let (key, end) = if self.shape.lifetimes && ordinal >= static_changes {
                let ordinal = ordinal - static_changes;
                (
                    self.shape.keys - self.shape.live_units
                        + tick as usize * self.shape.churn_per_tick
                        + if ordinal < self.shape.churn_per_tick {
                            ordinal
                        } else {
                            self.shape.live_units + ordinal - self.shape.churn_per_tick
                        },
                    ordinal < self.shape.churn_per_tick,
                )
            } else {
                (static_key(self.shape, tick, ordinal), false)
            };
            let number = (value_at(self.shape, key, tick as i64) * 8.0) as i64;
            let encoded = ((number as u64) << 1) ^ ((number >> 63) as u64);
            (tick as Hour * self.shape.tick_hours, key, end, encoded)
        };
        let mut lengths = [0usize; 12];
        lengths[0] = info.records;
        lengths[2] = info.records;
        lengths[3] = info.records;
        lengths[4] = info.records;
        for row in first..last {
            let (_, key, end, value) = row_data(row);
            lengths[1] += var_len(key as u64);
            if !end {
                lengths[7] += 2 + var_len(value);
            }
        }
        let header = var_len(info.records as u64)
            + lengths.iter().map(|n| var_len(*n as u64)).sum::<usize>();
        let mut columns = [0usize; 13];
        columns[0] = header;
        for i in 0..12 {
            columns[i + 1] = columns[i] + lengths[i];
        }
        let index = columns[12];
        let mut bytes = Buffer::new(memory, Owner::Forward, index + info.records * 17)?;
        for _ in 0..bytes.capacity() {
            bytes.push(0);
        }
        let mut writer = Writer::new(Some(&mut bytes[..header]));
        writer.uint(info.records as u64)?;
        for length in lengths {
            writer.uint(length as u64)?;
        }
        let mut offsets = columns;
        let mut previous = info.hours.first;
        let mut runs = Buffer::new(
            memory,
            Owner::Forward,
            ((last - 1) / c - first / c + 1) as usize,
        )?;
        for row in first..last {
            let (hour, key, end, value) = row_data(row);
            let local = (row - first) as usize;
            let at = index + local * 17;
            bytes[at] = (hour - info.hours.first) as u8;
            for (field, offset) in [offsets[1] - columns[1], local, 0, offsets[7] - columns[7]]
                .into_iter()
                .enumerate()
            {
                bytes[at + 1 + field * 4..at + 5 + field * 4]
                    .copy_from_slice(&(offset as u32).to_le_bytes());
            }
            bytes[offsets[0]] = (hour - previous) as u8;
            offsets[0] += 1;
            previous = hour;
            let mut writer = Writer::new(Some(&mut bytes[offsets[1]..columns[2]]));
            writer.uint(key as u64)?;
            offsets[1] += writer.len;
            bytes[columns[3] + local] = if end { 6 } else { 1 };
            if !end {
                let mut writer = Writer::new(Some(&mut bytes[offsets[7]..columns[8]]));
                writer.uint(0)?;
                writer.uint(var_len(value) as u64)?;
                writer.uint(value)?;
                offsets[7] += writer.len;
            }
            if runs.last().is_none_or(|run: &ShapeRun| run.hour != hour) {
                runs.push(ShapeRun {
                    hour,
                    first: local,
                    last: local,
                });
            }
            runs.last_mut().unwrap().last = local + 1;
        }
        let mut coverage = Buffer::new(memory, Owner::Forward, 1)?;
        coverage.push(Coverage {
            hours: info.hours,
            missing: None,
            accepted_before: info.hours.first as u64,
        });
        Ok(Page {
            info,
            bytes: Bytes::from_buffer(memory, Owner::Forward, bytes)?,
            decoder: ShapeDecoder {
                runs,
                columns,
                index,
            },
            coverage,
            backing: None,
        })
    }
    fn coverage(
        &self,
        hours: Hours,
        visit: &mut dyn FnMut(Coverage) -> ControlFlow<()>,
    ) -> io::Result<()> {
        if let Some(hours) = hours.intersection(Hours {
            first: 0,
            last: self.shape.hours - 1,
        }) {
            let _ = visit(Coverage {
                hours,
                missing: None,
                accepted_before: hours.first as u64,
            });
        }
        Ok(())
    }
}
