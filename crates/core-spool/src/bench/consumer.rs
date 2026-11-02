use super::{invalid, payloads::Payloads};
use core_spool::{
    Budget, Dictionary, EvidenceRef, Kind, Native, PayloadBlock, RecordView, Reservation, Type,
    paged::FlatFile,
};
use std::{io, path::Path};

const PAGE: u64 = 4096;
const CELL: usize = 80;
const PER_PAGE: u64 = PAGE / CELL as u64;

#[derive(Debug, PartialEq)]
enum TypedValue {
    Unit,
    U8(u8),
    I8(i8),
    U16(u16),
    I16(i16),
    U32(u32),
    I32(i32),
    U64(u64),
    I64(i64),
    Bool(bool),
    OptionalI32(Option<i32>),
    OptionalU32(Option<u32>),
    OptionalI64(Option<i64>),
    OptionalU64(Option<u64>),
    Bytes(core_spool::Blob),
    Text(core_spool::TextRef),
    U32List(core_spool::U32ListRef),
    U64List(core_spool::U64ListRef),
}
enum Current {
    Value(TypedValue),
    Unknown(EvidenceRef),
    Refused(EvidenceRef),
    Status(EvidenceRef),
    Lost(EvidenceRef),
    NotRecorded(EvidenceRef),
}
impl Current {
    fn decode(record: &RecordView<'_>) -> Self {
        if record.value_type == Type::Evidence {
            let evidence = EvidenceRef::decode(record.value);
            return match record.kind {
                Kind::Unknown => Self::Unknown(evidence),
                Kind::Refused => Self::Refused(evidence),
                Kind::Status => Self::Status(evidence),
                Kind::Lost => Self::Lost(evidence),
                Kind::NotRecorded => Self::NotRecorded(evidence),
                _ => unreachable!(),
            };
        }
        macro_rules! value {
            ($ty:ty, $name:ident) => {
                TypedValue::$name(<$ty as Native>::decode(record.value))
            };
        }
        Self::Value(match record.value_type {
            Type::Unit => TypedValue::Unit,
            Type::U8 => value!(u8, U8),
            Type::I8 => value!(i8, I8),
            Type::U16 => value!(u16, U16),
            Type::I16 => value!(i16, I16),
            Type::U32 => value!(u32, U32),
            Type::I32 => value!(i32, I32),
            Type::U64 => value!(u64, U64),
            Type::I64 => value!(i64, I64),
            Type::Bool => value!(bool, Bool),
            Type::OptionalI32 => value!(Option<i32>, OptionalI32),
            Type::OptionalU32 => value!(Option<u32>, OptionalU32),
            Type::OptionalI64 => value!(Option<i64>, OptionalI64),
            Type::OptionalU64 => value!(Option<u64>, OptionalU64),
            Type::Bytes => value!(core_spool::Blob, Bytes),
            Type::Text => value!(core_spool::TextRef, Text),
            Type::U32List => value!(core_spool::U32ListRef, U32List),
            Type::U64List => value!(core_spool::U64ListRef, U64List),
            Type::Evidence => unreachable!(),
        })
    }
    fn encode(&self, out: &mut [u8]) {
        macro_rules! number {
            ($n:expr) => {{
                let bytes = $n.to_le_bytes();
                out[..bytes.len()].copy_from_slice(&bytes);
            }};
        }
        macro_rules! optional {
            ($n:expr) => {{
                out[0] = u8::from($n.is_some());
                if let Some(n) = $n {
                    let bytes = n.to_le_bytes();
                    out[1..1 + bytes.len()].copy_from_slice(&bytes);
                }
            }};
        }
        fn blob(out: &mut [u8], blob: core_spool::Blob) {
            out[..8].copy_from_slice(&blob.offset.to_le_bytes());
            out[8..16].copy_from_slice(&blob.len.to_le_bytes());
        }
        match self {
            Self::Value(value) => match value {
                TypedValue::Unit => {}
                TypedValue::U8(n) => number!(n),
                TypedValue::I8(n) => number!(n),
                TypedValue::U16(n) => number!(n),
                TypedValue::I16(n) => number!(n),
                TypedValue::U32(n) => number!(n),
                TypedValue::I32(n) => number!(n),
                TypedValue::U64(n) => number!(n),
                TypedValue::I64(n) => number!(n),
                TypedValue::Bool(n) => out[0] = u8::from(*n),
                TypedValue::OptionalI32(n) => optional!(n),
                TypedValue::OptionalU32(n) => optional!(n),
                TypedValue::OptionalI64(n) => optional!(n),
                TypedValue::OptionalU64(n) => optional!(n),
                TypedValue::Bytes(b) => blob(out, *b),
                TypedValue::U32List(b) => blob(out, b.0),
                TypedValue::U64List(b) => blob(out, b.0),
                TypedValue::Text(t) => {
                    out[..4].copy_from_slice(&t.id.to_le_bytes());
                    out[4] = 1;
                    blob(&mut out[5..], t.text);
                }
            },
            Self::Unknown(e)
            | Self::Refused(e)
            | Self::Status(e)
            | Self::Lost(e)
            | Self::NotRecorded(e) => {
                out[0] = e.code;
                out[1..9].copy_from_slice(&e.reason.to_le_bytes());
                out[9..17].copy_from_slice(&e.first.to_le_bytes());
                out[17..25].copy_from_slice(&e.last.to_le_bytes());
                out[25] = u8::from(e.sample.is_some());
                out[26..34].copy_from_slice(&e.sample.unwrap_or_default().to_le_bytes());
                out[34] = u8::from(e.missing.is_some());
                out[35..43].copy_from_slice(&e.missing.unwrap_or_default().to_le_bytes());
            }
        }
    }
}

struct Column {
    root: u64,
    leaves: u64,
    last: Option<(u64, u64)>,
}
#[derive(Clone, Copy)]
pub struct Allowances {
    pub values: u64,
    pub index: u64,
    pub identities: u64,
    pub reasons: u64,
    pub remap: u64,
    pub reason_remap: u64,
    pub payloads: u64,
    pub metadata: u64,
}
impl Allowances {
    pub fn total(self) -> u64 {
        self.values
            + self.index
            + self.identities
            + self.reasons
            + self.remap
            + self.reason_remap
            + self.payloads
            + self.metadata
    }
}
pub struct Consumer {
    identities: Dictionary,
    reasons: Dictionary,
    remap: FlatFile,
    reason_remap: FlatFile,
    pages: FlatFile,
    index: FlatFile,
    payloads: Payloads,
    columns: Vec<Column>,
    tables: Vec<(usize, usize)>,
    _held: Reservation,
    local_count: u64,
    reason_count: u64,
    pub applied: u64,
}
impl Consumer {
    pub fn metadata_bytes() -> io::Result<u64> {
        let tables = replay_data::rows::TableId::ALL
            .iter()
            .map(|t| *t as usize)
            .max()
            .unwrap_or(0)
            + 1;
        let columns = replay_data::rows::TableId::ALL
            .iter()
            .map(|t| t.schema().len() + 3)
            .sum();
        held_bytes(columns, tables)
    }
    pub fn create(
        path: &Path,
        allowances: Allowances,
        memory: &Budget,
        disk: &Budget,
    ) -> io::Result<Self> {
        std::fs::create_dir(path)?;
        let memory = memory.owner(core_spool::Owner::CurrentValues);
        let table_count = replay_data::rows::TableId::ALL
            .iter()
            .map(|t| *t as usize)
            .max()
            .unwrap_or(0)
            + 1;
        let columns = replay_data::rows::TableId::ALL
            .iter()
            .map(|t| t.schema().len() + 3)
            .sum::<usize>();
        let metadata = held_bytes(columns, table_count)?;
        if metadata > allowances.metadata {
            return Err(invalid("consumer metadata allowance exhausted"));
        }
        let held = memory.reserve(metadata)?;
        let mut tables = Vec::new();
        tables
            .try_reserve_exact(table_count)
            .map_err(io::Error::other)?;
        tables.resize(table_count, (usize::MAX, 0));
        let mut storage = Vec::new();
        storage
            .try_reserve_exact(columns)
            .map_err(io::Error::other)?;
        for table in replay_data::rows::TableId::ALL {
            tables[table as usize] = (storage.len(), storage.len() + table.schema().len() + 3);
            storage.extend((0..table.schema().len() + 3).map(|_| Column {
                root: 0,
                leaves: 0,
                last: None,
            }));
        }
        if storage.capacity() != columns || tables.capacity() != table_count {
            return Err(invalid("consumer metadata exceeded reservation"));
        }
        let mut pages = FlatFile::create(&path.join("values"), allowances.values, &memory, disk)?;
        pages.grow(PAGE)?;
        Ok(Self {
            identities: Dictionary::create_named(
                path,
                "identities",
                allowances.identities,
                &memory,
                disk,
            )?,
            reasons: Dictionary::create_named(path, "reasons", allowances.reasons, &memory, disk)?,
            remap: FlatFile::create(&path.join("remap"), allowances.remap, &memory, disk)?,
            reason_remap: FlatFile::create(
                &path.join("reason-remap"),
                allowances.reason_remap,
                &memory,
                disk,
            )?,
            pages,
            index: FlatFile::create(&path.join("column-index"), allowances.index, &memory, disk)?,
            payloads: Payloads::create(&path.join("payloads"), allowances.payloads, &memory, disk)?,
            columns: storage,
            tables,
            _held: held,
            local_count: 0,
            reason_count: 0,
            applied: 0,
        })
    }
    pub fn begin_segment(&mut self) -> io::Result<()> {
        for id in 0..self.reason_count {
            let mut entry = [0; 24];
            self.reason_remap.read(id * 24, &mut entry)?;
            if entry[16] != 0 {
                self.payloads
                    .release(u64::from_le_bytes(entry[..8].try_into().unwrap()))?;
            }
        }
        self.local_count = 0;
        self.reason_count = 0;
        Ok(())
    }
    fn resolve(&mut self, id: u64, payload: &mut PayloadBlock<'_>) -> io::Result<u64> {
        while self.local_count <= id {
            let mut bytes = [0; 16];
            let mut n = 0;
            let len = payload.key_number(self.local_count, |part| {
                if part.len() > bytes.len() - n {
                    return Err(invalid("Gate identity exceeds u128"));
                }
                bytes[n..n + part.len()].copy_from_slice(part);
                n += part.len();
                Ok(())
            })?;
            if len != 16 {
                return Err(invalid("Gate identity width changed"));
            }
            let slot = self.identities.intern(&bytes)?;
            self.remap
                .write(self.local_count * 8, &slot.to_le_bytes())?;
            self.local_count += 1;
        }
        let mut bytes = [0; 8];
        self.remap.read(id * 8, &mut bytes)?;
        Ok(u64::from_le_bytes(bytes))
    }
    fn reason(&mut self, id: u64, payload: &mut PayloadBlock<'_>) -> io::Result<u64> {
        while self.reason_count <= id {
            let mut bytes = [0; 4096];
            let mut n = 0;
            let len = payload.reason(self.reason_count, |part| {
                let count = part.len().min(bytes.len() - n);
                bytes[n..n + count].copy_from_slice(&part[..count]);
                n += count;
                Ok(())
            })?;
            let mapped = if len <= bytes.len() as u64 {
                self.reasons.intern(&bytes[..n])?
            } else {
                // Oversized reasons stream to owned spill pages.
                let offset = self.payloads.allocate(len)?;
                let mut at = offset;
                payload.reason(self.reason_count, |part| {
                    self.payloads.pages.write(at, part)?;
                    at += part.len() as u64;
                    Ok(())
                })?;
                offset
            };
            let mut entry = [0; 24];
            entry[..8].copy_from_slice(&mapped.to_le_bytes());
            entry[8..16].copy_from_slice(&len.to_le_bytes());
            entry[16] = u8::from(len > bytes.len() as u64);
            self.reason_remap.write(self.reason_count * 24, &entry)?;
            self.reason_count += 1;
        }
        Ok(id * 24)
    }
    fn allocate(&mut self) -> io::Result<u64> {
        let at = self.pages.len();
        self.pages.grow(
            at.checked_add(PAGE)
                .ok_or_else(|| invalid("consumer store overflow"))?,
        )?;
        Ok(at)
    }
    fn address(&mut self, column: usize, slot: u64) -> io::Result<u64> {
        self.find_address(column, slot, true)?
            .ok_or_else(|| invalid("consumer allocation missing"))
    }
    fn find_address(&mut self, column: usize, slot: u64, create: bool) -> io::Result<Option<u64>> {
        let page = slot / PER_PAGE;
        if let Some((last, at)) = self.columns[column].last {
            if last == page {
                return Ok(Some(at + slot % PER_PAGE * CELL as u64));
            }
        }
        if page >= self.columns[column].leaves {
            if !create {
                return Ok(None);
            }
            let leaves = page
                .checked_add(1)
                .and_then(u64::checked_next_power_of_two)
                .ok_or_else(|| invalid("column index overflow"))?;
            let root = self.index.len();
            self.index.grow(
                root.checked_add(
                    leaves
                        .checked_mul(8)
                        .ok_or_else(|| invalid("column index overflow"))?,
                )
                .ok_or_else(|| invalid("column index overflow"))?,
            )?;
            for i in 0..self.columns[column].leaves {
                let mut entry = [0; 8];
                self.index
                    .read(self.columns[column].root + i * 8, &mut entry)?;
                self.index.write(root + i * 8, &entry)?;
            }
            self.columns[column].root = root;
            self.columns[column].leaves = leaves;
        }
        let address = self.columns[column].root + page * 8;
        let mut entry = [0; 8];
        self.index.read(address, &mut entry)?;
        let mut at = u64::from_le_bytes(entry);
        if at == 0 {
            if !create {
                return Ok(None);
            }
            at = self.allocate()?;
            self.index.write(address, &at.to_le_bytes())?;
        }
        self.columns[column].last = Some((page, at));
        Ok(Some(at + slot % PER_PAGE * CELL as u64))
    }
    fn release_cell(&mut self, at: u64) -> io::Result<()> {
        let mut cell = [0; CELL];
        self.pages.read(at, &mut cell)?;
        if cell[0] != 0 {
            let start = match cell[2] {
                14 | 16 | 17 => Some(24),
                15 => Some(29),
                18 if cell[3] != 0 => Some(25),
                _ => None,
            };
            if let Some(start) = start {
                self.payloads.release(u64::from_le_bytes(
                    cell[start..start + 8].try_into().unwrap(),
                ))?;
            }
        }
        Ok(())
    }
    pub fn apply(
        &mut self,
        hour: i64,
        table: u32,
        field: u32,
        record: RecordView<'_>,
        payload: &mut PayloadBlock<'_>,
    ) -> io::Result<()> {
        let (base, table_end) = *self
            .tables
            .get(table as usize)
            .filter(|&&(v, _)| v != usize::MAX)
            .ok_or_else(|| invalid("consumer table has no schema"))?;
        let slot = self.resolve(record.key.index(), payload)?;
        let mut cell = [0; CELL];
        cell[0] = 1;
        cell[1] = record.kind.code();
        cell[2] = record.value_type as u8;
        cell[8..16].copy_from_slice(&hour.to_le_bytes());
        let row_address = self.address(base, slot)?;
        let mut row = [0; CELL];
        self.pages.read(row_address, &mut row)?;
        let mut generation = u64::from_le_bytes(row[16..24].try_into().unwrap());
        let column = match record.kind {
            Kind::Appeared if record.value_type == Type::Unit => {
                generation = generation
                    .checked_add(1)
                    .ok_or_else(|| invalid("row generation overflow"))?;
                base
            }
            Kind::Gone => base,
            Kind::RowStart => base + 2,
            Kind::Status | Kind::NotRecorded | Kind::Refused | Kind::Lost => base + 1,
            Kind::Unknown | Kind::Changed | Kind::Appeared => base + 3 + field as usize,
            _ => base + 2,
        };
        if column >= table_end {
            return Err(invalid("consumer field has no schema"));
        }
        let table_state = matches!(
            record.kind,
            Kind::Status | Kind::NotRecorded | Kind::Refused | Kind::Lost
        );
        let at = self.address(column, if table_state { 0 } else { slot })?;
        self.release_cell(at)?;
        if row[0] != 0
            && (record.kind == Kind::Gone
                || (record.kind == Kind::Appeared && record.value_type == Type::Unit))
        {
            for field_column in base + 2..table_end {
                if let Some(address) = self.find_address(field_column, slot, false)? {
                    self.release_cell(address)?;
                    self.pages.write(address, &[0; CELL])?;
                }
            }
        }
        cell[16..24].copy_from_slice(&generation.to_le_bytes());
        Current::decode(&record).encode(&mut cell[24..67]);
        if record.value_type == Type::Evidence {
            let evidence = EvidenceRef::decode(record.value);
            let mapped = self.reason(evidence.reason, payload)?;
            // A carried cell owns its reason mapping across segment-local dictionary reuse.
            let mut reason = [0; 24];
            self.reason_remap.read(mapped, &mut reason)?;
            cell[25..33].copy_from_slice(&reason[..8]);
            cell[3] = reason[16];
            if cell[3] != 0 {
                self.payloads
                    .retain(u64::from_le_bytes(reason[..8].try_into().unwrap()))?;
            }
            cell[72..80].copy_from_slice(&reason[8..16]);
        } else if let Some(blob) = record.blob() {
            let at = self.payloads.allocate(blob.len)?;
            let mut offset = at;
            payload.blob(blob, |part| {
                self.payloads.pages.write(offset, part)?;
                offset += part.len() as u64;
                Ok(())
            })?;
            let start = if record.value_type == Type::Text {
                29
            } else {
                24
            };
            cell[start..start + 8].copy_from_slice(&at.to_le_bytes());
        }
        self.pages.write(at, &cell)?;
        self.applied += 1;
        Ok(())
    }
    #[cfg(test)]
    pub fn long_reason(&mut self, table: usize, field: usize, slot: u64) -> io::Result<Vec<u8>> {
        let at = self.address(self.tables[table].0 + 3 + field, slot)?;
        let mut cell = [0; CELL];
        self.pages.read(at, &mut cell)?;
        assert_eq!((cell[2], cell[3]), (Type::Evidence as u8, 1));
        let offset = u64::from_le_bytes(cell[25..33].try_into().unwrap());
        let len = u64::from_le_bytes(cell[72..80].try_into().unwrap());
        let mut bytes = vec![0; len as usize];
        self.payloads.pages.read(offset, &mut bytes)?;
        Ok(bytes)
    }
    #[cfg(test)]
    pub fn storage_shape(&self) -> (u32, u64) {
        (
            u32::from(self.columns.iter().any(|c| c.leaves != 0)),
            self.payloads.pages.len(),
        )
    }
    #[cfg(test)]
    pub fn bytes(&mut self, table: usize, field: usize, slot: u64) -> io::Result<Vec<u8>> {
        let (_, tag, value) = self
            .cell(table, field, slot)?
            .ok_or_else(|| invalid("missing synthetic payload"))?;
        assert_eq!(tag, Type::Bytes);
        let blob = <core_spool::Blob as Native>::decode(&value);
        let mut bytes = vec![0; blob.len as usize];
        self.payloads.pages.read(blob.offset, &mut bytes)?;
        Ok(bytes)
    }
    #[cfg(test)]
    pub fn table_state(&mut self, table: usize) -> io::Result<(u8, Vec<u8>)> {
        let at = self.address(self.tables[table].0 + 1, 0)?;
        let mut cell = [0; CELL];
        self.pages.read(at, &mut cell)?;
        Ok((cell[1], cell[24..67].to_vec()))
    }
    #[cfg(test)]
    pub fn cell(
        &mut self,
        table: usize,
        field: usize,
        slot: u64,
    ) -> io::Result<Option<(Kind, Type, Vec<u8>)>> {
        let base = self.tables[table].0;
        let row_at = self.address(base, slot)?;
        let mut row = [0; CELL];
        self.pages.read(row_at, &mut row)?;
        let at = self.address(base + 3 + field, slot)?;
        let mut cell = [0; CELL];
        self.pages.read(at, &mut cell)?;
        if cell[0] == 0 || row[1] == Kind::Gone.code() || cell[16..24] != row[16..24] {
            return Ok(None);
        }
        let kind = [
            Kind::Appeared,
            Kind::Changed,
            Kind::Gone,
            Kind::Unknown,
            Kind::Refused,
            Kind::Status,
            Kind::Read,
            Kind::SegmentStart,
            Kind::Discontinuity,
            Kind::VisibleKey,
            Kind::RowStart,
            Kind::Acquired,
            Kind::Supplied,
            Kind::Observed,
            Kind::Reread,
            Kind::Lost,
            Kind::NotRecorded,
        ][cell[1] as usize];
        let tag = match cell[2] {
            5 => Type::U32,
            18 => Type::Evidence,
            14 => Type::Bytes,
            _ => return Err(invalid("synthetic cell type")),
        };
        Ok(Some((kind, tag, cell[24..24 + tag.width()].to_vec())))
    }
}

fn held_bytes(columns: usize, tables: usize) -> io::Result<u64> {
    columns
        .checked_mul(size_of::<Column>())
        .and_then(|n| {
            tables
                .checked_mul(size_of::<(usize, usize)>())
                .and_then(|m| n.checked_add(m))
        })
        .map(|n| n as u64)
        .ok_or_else(|| invalid("consumer metadata overflow"))
}
