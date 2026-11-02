use crate::{
    budget::{Budget, Buffer, Reservation},
    dictionary::{Arena, Dictionary},
    invalid,
    types::*,
};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::Path,
};

pub const HEADER: usize = 80;
pub const LANE_HEADER: usize = 32;
pub const COMMIT: usize = 40;
const WORST_RECORD: usize = LANE_HEADER + 1 + 8 + 8 + 43;

#[derive(Clone, Copy)]
pub struct Allowances {
    pub dictionary: u64,
    pub reasons: u64,
}

#[derive(Clone, Copy)]
pub struct Config {
    pub block_bytes: usize,
    pub layout: Layout,
    pub compression: Compression,
}
#[derive(Clone, Copy)]
struct Staged {
    hour: i64,
    table: u32,
    field: u32,
    tag: Type,
    kind: Kind,
    key: u64,
    order: u64,
    value: [u8; 43],
}
impl Staged {
    fn lane(&self) -> (i64, u32, u32, Type) {
        (self.hour, self.table, self.field, self.tag)
    }
}
pub struct Writer {
    blocks: File,
    commits: File,
    dictionary: Dictionary,
    arena: Arena,
    reasons: Dictionary,
    lane_slots: Vec<usize>,
    _lanes_held: Reservation,
    lane_count: usize,
    value_bytes: usize,
    max_id: u64,
    disk: Budget,
    config: Config,
    staged: Vec<Staged>,
    _staged_held: Reservation,
    raw: Buffer,
    compressed: Buffer,
    _codec_held: Reservation,
    order: u64,
    block_offset: u64,
    previous_hour: Option<i64>,
    closed_hour: Option<i64>,
    first_continues: bool,
    poisoned: bool,
}
impl Writer {
    pub fn create(
        path: &Path,
        config: Config,
        allowances: Allowances,
        memory: &Budget,
        disk: &Budget,
    ) -> io::Result<Self> {
        if config.block_bytes < WORST_RECORD {
            return Err(invalid("block size cannot hold a typed record"));
        }
        let capacity = config.block_bytes / 4;
        let slots = capacity
            .checked_mul(2)
            .and_then(usize::checked_next_power_of_two)
            .ok_or_else(|| invalid("lane index overflow"))?;
        let lanes_held = memory.owner(crate::Owner::Staging).reserve(
            slots
                .checked_mul(size_of::<usize>())
                .ok_or_else(|| invalid("lane reservation overflow"))? as u64,
        )?;
        let mut lane_slots = Vec::new();
        lane_slots
            .try_reserve_exact(slots)
            .map_err(io::Error::other)?;
        if lane_slots.capacity() != slots {
            return Err(io::Error::other("allocator exceeded lane reservation"));
        }
        lane_slots.resize(slots, 0);
        let held = memory.owner(crate::Owner::Staging).reserve(
            capacity
                .checked_mul(size_of::<Staged>())
                .ok_or_else(|| invalid("staging size overflow"))? as u64,
        )?;
        let mut staged = Vec::new();
        staged
            .try_reserve_exact(capacity)
            .map_err(io::Error::other)?;
        if staged.capacity() != capacity {
            return Err(io::Error::other("allocator exceeded staging reservation"));
        }
        let block_memory = memory.owner(crate::Owner::WriterBlocks);
        let raw = Buffer::new(
            config
                .block_bytes
                .checked_add(HEADER)
                .ok_or_else(|| invalid("block bound overflow"))?,
            &block_memory,
        )?;
        let compressed_size = compressed_bound(config.block_bytes)?;
        let compressed = Buffer::new(
            if config.compression == Compression::Lz4 {
                compressed_size
                    .checked_add(HEADER)
                    .ok_or_else(|| invalid("block bound overflow"))?
            } else {
                0
            },
            &block_memory,
        )?;
        let codec_held = memory.owner(crate::Owner::Codec).reserve(
            // An upper bound, not a measurement: LZ4 allocates an 8 KiB or 16 KiB hash table per compression call.
            if config.compression == Compression::Lz4 {
                16384
            } else {
                0
            },
        )?;
        fs::create_dir(path)?;
        let new = |name| {
            OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(path.join(name))
        };
        let mut format = [0; 24];
        format[..8].copy_from_slice(b"H4FORMAT");
        format[8..16].copy_from_slice(&(config.block_bytes as u64).to_le_bytes());
        let format_crc = crc32fast::hash(&format[..16]);
        format[16..20].copy_from_slice(&format_crc.to_le_bytes());
        disk.grow_disk(format.len() as u64)?;
        new("format")?.write_all(&format)?;
        Ok(Self {
            blocks: new("blocks")?,
            commits: new("commits")?,
            dictionary: Dictionary::create(
                path,
                allowances.dictionary,
                &memory.owner(crate::Owner::Dictionary),
                disk,
            )?,
            arena: Arena::create(path, config.block_bytes, memory, disk)?,
            reasons: Dictionary::create_named(
                path,
                "reasons",
                allowances.reasons,
                &memory.owner(crate::Owner::Reasons),
                disk,
            )?,
            lane_slots,
            _lanes_held: lanes_held,
            lane_count: 0,
            value_bytes: 0,
            max_id: 0,
            disk: disk.clone(),
            config,
            staged,
            _staged_held: held,
            raw,
            compressed,
            _codec_held: codec_held,
            order: 0,
            block_offset: 0,
            previous_hour: None,
            closed_hour: None,
            first_continues: false,
            poisoned: false,
        })
    }
    pub fn append(&mut self, record: Record<'_>) -> io::Result<()> {
        if self.poisoned {
            return Err(io::Error::other(
                "writer stopped after an earlier append failure",
            ));
        }
        let result = self.append_inner(record);
        if result.is_err() {
            self.poisoned = true;
        }
        result
    }
    fn append_inner(&mut self, record: Record<'_>) -> io::Result<()> {
        if self.previous_hour.is_some_and(|h| record.hour < h)
            || self.closed_hour.is_some_and(|h| record.hour <= h)
        {
            return Err(invalid("record hour precedes the published frontier"));
        }
        let key = self.dictionary.intern(record.key)?;
        let mut value = [0; 43];
        let tag = encode_value(&mut self.arena, &mut self.reasons, record.value, &mut value)?;
        if record.kind.evidence() != (tag == Type::Evidence) {
            return Err(invalid(
                "missing/status kinds require evidence, never a value",
            ));
        }
        let staged = Staged {
            hour: record.hour,
            table: record.table,
            field: record.field,
            tag,
            kind: record.kind,
            key,
            order: self.order,
            value,
        };
        let (mut slot, mut new_lane) = self.lane_slot(&staged);
        let bytes = (self.staged.len() + 1)
            * (1 + order_width(self.staged.len() as u64) + id_width(self.max_id.max(key)))
            + self.value_bytes
            + tag.width()
            + (self.lane_count + usize::from(new_lane)) * LANE_HEADER;
        if bytes > self.config.block_bytes {
            self.flush(self.previous_hour == Some(record.hour))?;
            (slot, new_lane) = self.lane_slot(&staged);
        }
        if new_lane {
            self.lane_count += 1;
            self.lane_slots[slot] = self.staged.len() + 1;
        }
        self.max_id = self.max_id.max(key);
        self.value_bytes += tag.width();
        self.staged.push(staged);
        self.order = self
            .order
            .checked_add(1)
            .ok_or_else(|| invalid("record order exceeds file address space"))?;
        self.previous_hour = Some(record.hour);
        Ok(())
    }
    fn lane_slot(&self, record: &Staged) -> (usize, bool) {
        let (hour, table, field, tag) = record.lane();
        let hash = (hour as u64).wrapping_mul(0x9e3779b97f4a7c15)
            ^ u64::from(table).wrapping_mul(0x100000001b3)
            ^ (u64::from(field) << 8)
            ^ tag as u64;
        let mut slot = hash as usize & (self.lane_slots.len() - 1);
        loop {
            let entry = self.lane_slots[slot];
            if entry == 0 {
                return (slot, true);
            }
            if self.staged[entry - 1].lane() == record.lane() {
                return (slot, false);
            }
            slot = (slot + 1) & (self.lane_slots.len() - 1);
        }
    }
    pub fn publish_hour(&mut self) -> io::Result<()> {
        if self.poisoned {
            return Err(io::Error::other("writer stopped after failure"));
        }
        let result = self.flush(false);
        if result.is_err() {
            self.poisoned = true;
        } else {
            self.closed_hour = self.previous_hour;
        }
        result
    }
    pub fn finish(mut self) -> io::Result<()> {
        self.publish_hour()?;
        self.dictionary.finish()?;
        self.reasons.finish()?;
        self.arena.flush()?;
        let offset = self.block_offset;
        self.commit(
            offset,
            0,
            self.previous_hour.unwrap_or(0),
            self.previous_hour.unwrap_or(0),
            1,
        )
    }
    fn commit(
        &mut self,
        offset: u64,
        bytes: u64,
        first: i64,
        last: i64,
        sealed: u32,
    ) -> io::Result<()> {
        let mut entry = [0; COMMIT];
        entry[..8].copy_from_slice(&offset.to_le_bytes());
        entry[8..16].copy_from_slice(&bytes.to_le_bytes());
        entry[16..24].copy_from_slice(&first.to_le_bytes());
        entry[24..32].copy_from_slice(&last.to_le_bytes());
        entry[32..36].copy_from_slice(&sealed.to_le_bytes());
        let crc = crc32fast::hash(&entry[..36]);
        entry[36..].copy_from_slice(&crc.to_le_bytes());
        self.disk.grow_disk(COMMIT as u64)?;
        self.commits.write_all(&entry)?;
        self.commits.flush()
    }
    fn flush(&mut self, continues: bool) -> io::Result<()> {
        if self.staged.is_empty() {
            return Ok(());
        }
        self.staged.sort_unstable_by_key(|r| (r.lane(), r.order));
        let first = self.staged.first().unwrap().hour;
        let last = self.staged.last().unwrap().hour;
        let ids = id_width(self.max_id);
        let orders = order_width(self.staged.len() as u64 - 1);
        self.raw.clear();
        self.raw.resize(HEADER)?;
        let base_order = self.staged.iter().map(|r| r.order).min().unwrap();
        let mut start = 0;
        while start < self.staged.len() {
            let lane = self.staged[start].lane();
            let mut end = start + 1;
            while end < self.staged.len() && self.staged[end].lane() == lane {
                end += 1;
            }
            let rows = &self.staged[start..end];
            let mut h = [0; LANE_HEADER];
            h[..8].copy_from_slice(&lane.0.to_le_bytes());
            h[8..12].copy_from_slice(&lane.1.to_le_bytes());
            h[12..16].copy_from_slice(&lane.2.to_le_bytes());
            h[16] = lane.3 as u8;
            h[24..32].copy_from_slice(&(rows.len() as u64).to_le_bytes());
            self.raw.append(&h)?;
            match self.config.layout {
                Layout::Rows => {
                    for r in rows {
                        self.raw
                            .append(&(r.order - base_order).to_le_bytes()[..orders])?;
                        self.raw.append(&[r.kind.0])?;
                        self.raw.append(&r.key.to_le_bytes()[..ids])?;
                        self.raw.append(&r.value[..r.tag.width()])?;
                    }
                }
                Layout::Columns => {
                    for r in rows {
                        self.raw
                            .append(&(r.order - base_order).to_le_bytes()[..orders])?;
                    }
                    for r in rows {
                        self.raw.append(&[r.kind.0])?;
                    }
                    for r in rows {
                        self.raw.append(&r.key.to_le_bytes()[..ids])?;
                    }
                    for r in rows {
                        self.raw.append(&r.value[..r.tag.width()])?;
                    }
                }
            }
            start = end;
        }
        let stored_size = match self.config.compression {
            Compression::None => self.raw.len() - HEADER,
            Compression::Lz4 => {
                self.compressed.resize(self.compressed.capacity())?;
                let size = lz4_flex::block::compress_into(
                    &self.raw[HEADER..],
                    &mut self.compressed[HEADER..],
                )
                .map_err(io::Error::other)?;
                self.compressed.resize(HEADER + size)?;
                size
            }
        };
        let mut header = [0; HEADER];
        header[..8].copy_from_slice(b"H4SPOOL3");
        header[8..16].copy_from_slice(&((self.raw.len() - HEADER) as u64).to_le_bytes());
        header[16..24].copy_from_slice(&(stored_size as u64).to_le_bytes());
        header[24..32].copy_from_slice(&(self.staged.len() as u64).to_le_bytes());
        header[32..40].copy_from_slice(&first.to_le_bytes());
        header[40..48].copy_from_slice(&last.to_le_bytes());
        header[48] = self.config.compression as u8;
        header[49] = self.config.layout as u8;
        header[50] = ids as u8;
        header[51] = u8::from(self.first_continues);
        header[52] = u8::from(continues);
        header[53] = orders as u8;
        header[56..60].copy_from_slice(&crc32fast::hash(&self.raw[HEADER..]).to_le_bytes());
        header[64..72].copy_from_slice(&base_order.to_le_bytes());
        let crc = crc32fast::hash(&header[..76]);
        header[76..80].copy_from_slice(&crc.to_le_bytes());
        let offset = self.block_offset;
        let size = (HEADER as u64)
            .checked_add(stored_size as u64)
            .ok_or_else(|| invalid("block file address overflow"))?;
        offset
            .checked_add(size)
            .ok_or_else(|| invalid("block file address overflow"))?;
        self.disk.grow_disk(size)?;
        self.dictionary.flush()?;
        self.reasons.flush()?;
        self.arena.flush()?;
        let stored = if self.config.compression == Compression::None {
            &mut self.raw
        } else {
            &mut self.compressed
        };
        stored[..HEADER].copy_from_slice(&header);
        self.blocks.write_all(stored)?;
        self.block_offset += size;
        self.commit(offset, size, first, last, 0)?;
        self.staged.clear();
        self.lane_slots.fill(0);
        self.lane_count = 0;
        self.value_bytes = 0;
        self.max_id = 0;
        self.first_continues = continues;
        Ok(())
    }
}
// lz4_flex::block::compress_into refuses output smaller than its get_maximum_output_size, 20 + 110% of the input.
pub(crate) fn compressed_bound(bytes: usize) -> io::Result<usize> {
    bytes
        .checked_mul(110)
        .and_then(|n| (n / 100).checked_add(20))
        .ok_or_else(|| invalid("compressed buffer size overflow"))
}
fn encode_value(
    arena: &mut Arena,
    reasons: &mut Dictionary,
    input: Value<'_>,
    out: &mut [u8; 43],
) -> io::Result<Type> {
    fn blob(out: &mut [u8], b: Blob) {
        out[..8].copy_from_slice(&b.offset.to_le_bytes());
        out[8..16].copy_from_slice(&b.len.to_le_bytes());
    }
    macro_rules! scalar {
        ($v:expr, $t:ident) => {{
            let b = $v.to_le_bytes();
            out[..b.len()].copy_from_slice(&b);
            Type::$t
        }};
    }
    macro_rules! optional {
        ($v:expr, $t:ident) => {{
            out[0] = u8::from($v.is_some());
            if let Some(v) = $v {
                let b = v.to_le_bytes();
                out[1..1 + b.len()].copy_from_slice(&b);
            }
            Type::$t
        }};
    }
    Ok(match input {
        Value::Unit => Type::Unit,
        Value::U8(v) => scalar!(v, U8),
        Value::I8(v) => scalar!(v, I8),
        Value::U16(v) => scalar!(v, U16),
        Value::I16(v) => scalar!(v, I16),
        Value::U32(v) => scalar!(v, U32),
        Value::I32(v) => scalar!(v, I32),
        Value::U64(v) => scalar!(v, U64),
        Value::I64(v) => scalar!(v, I64),
        Value::Bool(v) => {
            out[0] = u8::from(v);
            Type::Bool
        }
        Value::OptionalI32(v) => optional!(v, OptionalI32),
        Value::OptionalU32(v) => optional!(v, OptionalU32),
        Value::OptionalI64(v) => optional!(v, OptionalI64),
        Value::OptionalU64(v) => optional!(v, OptionalU64),
        Value::Bytes(v) => {
            blob(out, arena.bytes(v)?);
            Type::Bytes
        }
        Value::Text { id, text } => {
            if text.is_none() {
                return Err(invalid("unresolved text must be an unknown event"));
            }
            out[..4].copy_from_slice(&id.to_le_bytes());
            out[4] = u8::from(text.is_some());
            if let Some(v) = text {
                blob(&mut out[5..], arena.bytes(v)?);
            }
            Type::Text
        }
        Value::U32List(v) => {
            blob(out, arena.u32s(v)?);
            Type::U32List
        }
        Value::U64List(v) => {
            blob(out, arena.u64s(v)?);
            Type::U64List
        }
        Value::Evidence(v) => {
            if v.first > v.last {
                return Err(invalid("evidence hours are reversed"));
            }
            out[0] = v.code;
            out[1..9].copy_from_slice(&reasons.intern(v.reason)?.to_le_bytes());
            out[9..17].copy_from_slice(&v.first.to_le_bytes());
            out[17..25].copy_from_slice(&v.last.to_le_bytes());
            out[25] = u8::from(v.sample.is_some());
            out[26..34].copy_from_slice(&v.sample.unwrap_or_default().to_le_bytes());
            out[34] = u8::from(v.missing.is_some());
            out[35..43].copy_from_slice(&v.missing.unwrap_or_default().to_le_bytes());
            Type::Evidence
        }
    })
}

fn id_width(id: u64) -> usize {
    ((64 - id.leading_zeros()).div_ceil(8) as usize).max(1)
}

fn order_width(relative: u64) -> usize {
    id_width(relative).max(2)
}
