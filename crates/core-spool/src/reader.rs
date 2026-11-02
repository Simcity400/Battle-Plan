use crate::{
    budget::{Budget, Buffer, Reservation},
    invalid,
    types::*,
    writer::{COMMIT, HEADER, LANE_HEADER, compressed_bound},
};
use std::{
    fs::File,
    io::{self, BufReader, Read, Seek, SeekFrom},
    path::Path,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlockLocation {
    pub offset: u64,
    pub bytes: u64,
    pub first: i64,
    pub last: i64,
}
pub enum Next<'a> {
    Block(Block<'a>),
    Pending,
    Finished,
}
pub struct Reader {
    blocks: BufReader<File>,
    commits: BufReader<File>,
    block_position: u64,
    commit_position: u64,
    _io_held: Reservation,
    order: Vec<Entry>,
    _order_held: Reservation,
    cursor: u64,
    raw: Buffer,
    stored: Buffer,
    finished: bool,
    loaded: [u8; HEADER],
}
#[derive(Clone, Copy)]
pub struct Block<'a> {
    pub location: BlockLocation,
    pub records: u64,
    pub first_continues: bool,
    pub last_continues: bool,
    raw: &'a [u8],
    order: &'a [Entry],
    base_order: u64,
    ids: usize,
    orders: usize,
    layout: Layout,
}
impl Reader {
    pub fn open(path: &Path, maximum_block_bytes: usize, memory: &Budget) -> io::Result<Self> {
        let mut format = [0; 24];
        File::open(path.join("format"))?.read_exact(&mut format)?;
        if &format[..8] != b"H4FORMAT"
            || crc32fast::hash(&format[..16])
                != u32::from_le_bytes(format[16..20].try_into().unwrap())
        {
            return Err(invalid("spool format header checksum failed"));
        }
        let bound = u64_at(&format, 8);
        if (maximum_block_bytes as u64) < bound {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "maximum_block_bytes {maximum_block_bytes} is smaller than spool block bound {bound}"
                ),
            ));
        }
        let memory = memory.owner(crate::Owner::ReaderBlocks);
        let io_held = memory.reserve(2 * 65536)?;
        let capacity = maximum_block_bytes / 4;
        let order_held = memory.reserve(
            capacity
                .checked_mul(size_of::<Entry>())
                .ok_or_else(|| invalid("order reservation overflow"))? as u64,
        )?;
        let mut order = Vec::new();
        order
            .try_reserve_exact(capacity)
            .map_err(io::Error::other)?;
        if order.capacity() != capacity {
            return Err(invalid("allocator exceeded order reservation"));
        }
        Ok(Self {
            blocks: BufReader::with_capacity(65536, File::open(path.join("blocks"))?),
            commits: BufReader::with_capacity(65536, File::open(path.join("commits"))?),
            block_position: 0,
            commit_position: 0,
            _io_held: io_held,
            order,
            _order_held: order_held,
            cursor: 0,
            raw: Buffer::new(maximum_block_bytes, &memory)?,
            stored: Buffer::new(compressed_bound(maximum_block_bytes)?, &memory)?,
            finished: false,
            loaded: [0; HEADER],
        })
    }
    pub fn cursor(&self) -> u64 {
        self.cursor
    }
    pub fn restart(&mut self, cursor: u64) -> io::Result<()> {
        if !cursor.is_multiple_of(COMMIT as u64) {
            return Err(invalid("restart cursor is not a commit boundary"));
        }
        self.cursor = cursor;
        self.finished = false;
        Ok(())
    }
    fn committed(&mut self) -> io::Result<Option<BlockLocation>> {
        if self.finished {
            return Ok(None);
        }
        if self.commit_position != self.cursor {
            self.commits.seek(SeekFrom::Start(self.cursor))?;
            self.commit_position = self.cursor;
        }
        let mut c = [0; COMMIT];
        let mut n = 0;
        while n < COMMIT {
            let read = self.commits.read(&mut c[n..])?;
            self.commit_position += read as u64;
            if read == 0 {
                return Ok(None);
            }
            n += read;
        }
        let location = BlockLocation {
            offset: u64_at(&c, 0),
            bytes: u64_at(&c, 8),
            first: i64_at(&c, 16),
            last: i64_at(&c, 24),
        };
        if crc32fast::hash(&c[..36]) != u32::from_le_bytes(c[36..].try_into().unwrap()) {
            return Err(damage(location, "commit checksum failed"));
        }
        match u32::from_le_bytes(c[32..36].try_into().unwrap()) {
            0 if location.first <= location.last && location.bytes >= HEADER as u64 => {
                Ok(Some(location))
            }
            1 if location.bytes == 0 => {
                self.finished = true;
                Ok(None)
            }
            _ => Err(damage(location, "invalid block commit")),
        }
    }
    pub fn next_block(&mut self) -> io::Result<Next<'_>> {
        let Some(location) = self.committed()? else {
            return Ok(if self.finished {
                Next::Finished
            } else {
                Next::Pending
            });
        };
        self.load(location)?;
        self.cursor = self
            .cursor
            .checked_add(COMMIT as u64)
            .ok_or_else(|| invalid("commit address overflow"))?;
        Ok(Next::Block(self.view(location)))
    }
    pub fn skip_block(&mut self) -> io::Result<Option<BlockLocation>> {
        let Some(location) = self.committed()? else {
            return Ok(None);
        };
        self.header(location)?;
        self.cursor = self
            .cursor
            .checked_add(COMMIT as u64)
            .ok_or_else(|| invalid("commit address overflow"))?;
        Ok(Some(location))
    }
    pub fn read_at(&mut self, location: BlockLocation) -> io::Result<Block<'_>> {
        self.load(location)?;
        Ok(self.view(location))
    }
    fn header(&mut self, location: BlockLocation) -> io::Result<[u8; HEADER]> {
        let mut read = || -> io::Result<[u8; HEADER]> {
            if self.block_position != location.offset {
                self.blocks.seek(SeekFrom::Start(location.offset))?;
            }
            self.block_position = u64::MAX;
            let mut h = [0; HEADER];
            self.blocks.read_exact(&mut h)?;
            if &h[..8] != b"H4SPOOL3"
                || crc32fast::hash(&h[..76]) != u32::from_le_bytes(h[76..80].try_into().unwrap())
            {
                return Err(invalid("block header checksum or magic failed"));
            }
            if u64_at(&h, 16).checked_add(HEADER as u64) != Some(location.bytes)
                || i64_at(&h, 32) != location.first
                || i64_at(&h, 40) != location.last
            {
                return Err(invalid("block header disagrees with commit"));
            }
            if h[48] > 1
                || h[49] > 1
                || !(1..=8).contains(&h[50])
                || h[51] > 1
                || h[52] > 1
                || !(2..=8).contains(&h[53])
                || h[54..56] != [0; 2]
            {
                return Err(invalid("unsupported block encoding"));
            }
            location
                .offset
                .checked_add(location.bytes)
                .ok_or_else(|| invalid("block address overflow"))?;
            self.block_position = location.offset + HEADER as u64;
            Ok(h)
        };
        read().map_err(|e| damage(location, e))
    }
    fn load(&mut self, location: BlockLocation) -> io::Result<()> {
        let h = self.header(location)?;
        let mut load = || -> io::Result<()> {
            let raw_size = usize::try_from(u64_at(&h, 8))
                .map_err(|_| invalid("block exceeds address space"))?;
            let stored_size = usize::try_from(u64_at(&h, 16))
                .map_err(|_| invalid("block exceeds address space"))?;
            if raw_size > self.raw.capacity() || stored_size > self.stored.capacity() {
                return Err(invalid("block exceeds declared spool bound"));
            }
            self.raw.resize(raw_size)?;
            if h[48] == 0 {
                if raw_size != stored_size {
                    return Err(invalid("uncompressed block size mismatch"));
                }
                self.blocks.read_exact(&mut self.raw)?;
            } else {
                self.stored.resize(stored_size)?;
                self.blocks.read_exact(&mut self.stored)?;
                let decoded = lz4_flex::block::decompress_into(&self.stored, &mut self.raw)
                    .map_err(io::Error::other)?;
                if decoded != raw_size {
                    return Err(invalid("lz4 block size mismatch"));
                }
            }
            if crc32fast::hash(&self.raw) != u32::from_le_bytes(h[56..60].try_into().unwrap()) {
                return Err(invalid("block checksum failed"));
            }
            validate(
                &self.raw,
                u64_at(&h, 24),
                h[50] as usize,
                h[53] as usize,
                layout(h[49]),
                location,
                u64_at(&h, 64),
                &mut self.order,
            )?;
            self.block_position = location.offset + location.bytes;
            Ok(())
        };
        load().map_err(|e| damage(location, e))?;
        self.loaded = h;
        Ok(())
    }
    fn view(&self, location: BlockLocation) -> Block<'_> {
        let h = &self.loaded;
        Block {
            location,
            records: u64_at(h, 24),
            first_continues: h[51] != 0,
            last_continues: h[52] != 0,
            raw: &self.raw,
            order: &self.order,
            base_order: u64_at(h, 64),
            ids: h[50] as usize,
            orders: h[53] as usize,
            layout: layout(h[49]),
        }
    }
}
fn layout(tag: u8) -> Layout {
    if tag == 0 {
        Layout::Rows
    } else {
        Layout::Columns
    }
}
fn u64_at(b: &[u8], i: usize) -> u64 {
    u64::from_le_bytes(b[i..i + 8].try_into().unwrap())
}
fn i64_at(b: &[u8], i: usize) -> i64 {
    i64::from_le_bytes(b[i..i + 8].try_into().unwrap())
}
pub(crate) fn damage(location: BlockLocation, detail: impl std::fmt::Display) -> io::Error {
    invalid(format!(
        "spool block hours {}..={} at {}: {detail}",
        location.first, location.last, location.offset
    ))
}
fn lane_at(
    raw: &[u8],
    ids: usize,
    orders: usize,
    base_order: u64,
    layout: Layout,
) -> io::Result<(Lane<'_>, usize)> {
    if raw.len() < LANE_HEADER {
        return Err(invalid("short lane header"));
    }
    let tag = Type::parse(raw[16])?;
    if raw[17..24] != [0; 7] {
        return Err(invalid("unsupported lane header"));
    }
    let count = usize::try_from(u64_at(raw, 24))
        .map_err(|_| invalid("lane count exceeds address space"))?;
    let end = count
        .checked_mul(orders + 1 + ids + tag.width())
        .and_then(|n| n.checked_add(LANE_HEADER))
        .filter(|&end| end <= raw.len())
        .ok_or_else(|| invalid("lane exceeds block"))?;
    if count == 0 {
        return Err(invalid("empty typed lane"));
    }
    Ok((
        Lane {
            hour: i64_at(raw, 0),
            table: u32::from_le_bytes(raw[8..12].try_into().unwrap()),
            field: u32::from_le_bytes(raw[12..16].try_into().unwrap()),
            value_type: tag,
            data: &raw[LANE_HEADER..end],
            count,
            ids,
            orders,
            base_order,
            layout,
        },
        end,
    ))
}
#[derive(Clone, Copy, Default)]
struct Entry {
    table: u32,
    field: u32,
    key: usize,
    value: usize,
    tag: u8,
    kind: u8,
    occupied: bool,
}
fn validate(
    raw: &[u8],
    records: u64,
    ids: usize,
    orders: usize,
    layout: Layout,
    location: BlockLocation,
    base_order: u64,
    order: &mut Vec<Entry>,
) -> io::Result<()> {
    let n = usize::try_from(records).map_err(|_| invalid("record count exceeds address space"))?;
    if n == 0 || n > order.capacity() || base_order.checked_add(records).is_none() {
        return Err(invalid("invalid block record count"));
    }
    order.clear();
    order.resize(n, Entry::default());
    let mut at = 0;
    let mut count = 0;
    let mut previous = None;
    let mut first_hour = None;
    let mut last_hour = None;
    let mut prior_hour_max = None;
    let mut hour_max = None::<u64>;
    while at < raw.len() {
        let (lane, size) = lane_at(&raw[at..], ids, orders, base_order, layout)?;
        let key = (lane.hour, lane.table, lane.field, lane.value_type);
        if previous.is_some_and(|p| p >= key)
            || lane.hour < location.first
            || lane.hour > location.last
        {
            return Err(invalid("lanes are not in schema/hour order"));
        }
        first_hour.get_or_insert(lane.hour);
        if last_hour.is_some_and(|hour| hour != lane.hour) {
            prior_hour_max = hour_max;
        }
        last_hour = Some(lane.hour);
        previous = Some(key);
        for record in lane.records() {
            if prior_hour_max.is_some_and(|max| record.order <= max) {
                return Err(invalid("source order crosses hour boundaries"));
            }
            hour_max = Some(hour_max.map_or(record.order, |max| max.max(record.order)));
            if record.kind.0 > 16 || record.kind.evidence() != (lane.value_type == Type::Evidence) {
                return Err(invalid("invalid event kind or evidence"));
            }
            validate_value(lane.value_type, record.value)?;
            let index = record
                .order
                .checked_sub(base_order)
                .and_then(|n| usize::try_from(n).ok())
                .filter(|&i| i < n)
                .ok_or_else(|| invalid("invalid record order"))?;
            if order[index].occupied {
                return Err(invalid("duplicate record order"));
            }
            order[index] = Entry {
                table: lane.table,
                field: lane.field,
                key: record.key.bytes().as_ptr() as usize - raw.as_ptr() as usize,
                value: record.value.as_ptr() as usize - raw.as_ptr() as usize,
                tag: lane.value_type as u8,
                kind: record.kind.code(),
                occupied: true,
            };
            count += 1;
        }
        at += size;
    }
    if count != n || first_hour != Some(location.first) || last_hour != Some(location.last) {
        return Err(invalid("block record/hour count mismatch"));
    }
    Ok(())
}
fn validate_value(tag: Type, b: &[u8]) -> io::Result<()> {
    let valid = match tag {
        Type::Bool
        | Type::OptionalI32
        | Type::OptionalU32
        | Type::OptionalI64
        | Type::OptionalU64 => b[0] <= 1,
        Type::Text => b[4] == 1,
        Type::U32List => Blob::decode(b).len.is_multiple_of(4),
        Type::U64List => Blob::decode(b).len.is_multiple_of(8),
        Type::Evidence => b[25] <= 1 && b[34] <= 1 && i64_at(b, 9) <= i64_at(b, 17),
        _ => true,
    };
    if valid {
        Ok(())
    } else {
        Err(invalid("invalid native value"))
    }
}
impl<'a> Block<'a> {
    pub fn raw_bytes(self) -> usize {
        self.raw.len()
    }
    pub fn hours(self) -> Hours<'a> {
        Hours {
            block: self,
            rest: self.raw,
            directory: self.order,
            next_order: self.base_order,
            first: true,
        }
    }
}
pub struct Hours<'a> {
    block: Block<'a>,
    rest: &'a [u8],
    directory: &'a [Entry],
    next_order: u64,
    first: bool,
}
pub struct Hour<'a> {
    pub hour: i64,
    pub starts: bool,
    pub ends: bool,
    data: &'a [u8],
    directory: &'a [Entry],
    next_order: u64,
    raw: &'a [u8],
    ids: usize,
    orders: usize,
    base_order: u64,
    layout: Layout,
}
impl<'a> Iterator for Hours<'a> {
    type Item = Hour<'a>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.rest.is_empty() {
            return None;
        }
        let hour = i64_at(self.rest, 0);
        let mut bytes = 0;
        let mut count = 0;
        while bytes < self.rest.len() && i64_at(&self.rest[bytes..], 0) == hour {
            let (lane, size) = lane_at(
                &self.rest[bytes..],
                self.block.ids,
                self.block.orders,
                self.block.base_order,
                self.block.layout,
            )
            .unwrap();
            bytes += size;
            count += lane.len();
        }
        let (data, rest) = self.rest.split_at(bytes);
        self.rest = rest;
        let (directory, tail) = self.directory.split_at(count);
        self.directory = tail;
        let next_order = self.next_order;
        self.next_order += count as u64;
        let starts = !self.first || !self.block.first_continues;
        self.first = false;
        Some(Hour {
            hour,
            starts,
            ends: !self.rest.is_empty() || !self.block.last_continues,
            data,
            directory,
            next_order,
            raw: self.block.raw,
            ids: self.block.ids,
            orders: self.block.orders,
            base_order: self.block.base_order,
            layout: self.block.layout,
        })
    }
}
pub struct Lanes<'a> {
    rest: &'a [u8],
    ids: usize,
    orders: usize,
    base_order: u64,
    layout: Layout,
}
impl<'a> Iterator for Lanes<'a> {
    type Item = Lane<'a>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.rest.is_empty() {
            return None;
        }
        let (lane, size) = lane_at(
            self.rest,
            self.ids,
            self.orders,
            self.base_order,
            self.layout,
        )
        .unwrap();
        self.rest = &self.rest[size..];
        Some(lane)
    }
}
impl<'a> Hour<'a> {
    pub fn lanes(&self) -> Lanes<'a> {
        Lanes {
            rest: self.data,
            ids: self.ids,
            orders: self.orders,
            base_order: self.base_order,
            layout: self.layout,
        }
    }
    pub fn ordered(&self) -> impl ExactSizeIterator<Item = (u32, u32, RecordView<'a>)> + '_ {
        self.directory.iter().enumerate().map(|(index, entry)| {
            let tag = Type::parse(entry.tag).unwrap();
            (
                entry.table,
                entry.field,
                RecordView {
                    order: self.next_order + index as u64,
                    kind: Kind(entry.kind),
                    key: KeyId(&self.raw[entry.key..entry.key + self.ids]),
                    value_type: tag,
                    value: &self.raw[entry.value..entry.value + tag.width()],
                },
            )
        })
    }
}
