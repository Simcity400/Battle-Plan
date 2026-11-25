
use crate::{
    envelope::{Extent, Kind, RowPlace, SPAN_BLOCKS, SpanPages, UNIT_ORDINALS},
    invalid,
    leaf::{Reader, Writer},
    unit::{BlockPage, FrameEntry, TableEntry, get_frame, get_varint, put_frame, put_varint},
};
use std::io;

#[cfg(test)]
mod tests;

fn put_extent(out: &mut Vec<u8>, extent: &Extent) -> io::Result<()> {
    let mut writer = Writer::new();
    extent.write(&mut writer)?;
    out.extend_from_slice(writer.bytes());
    Ok(())
}

fn get_extent(bytes: &[u8], at: &mut usize) -> io::Result<Extent> {
    let rest = bytes.get(*at..).ok_or_else(|| invalid("A page ends inside an extent"))?;
    let mut reader = Reader::new(rest);
    let extent = Extent::read(&mut reader)?;
    *at += rest.len() - reader.left();
    Ok(extent)
}

fn get_count(bytes: &[u8], at: &mut usize, each: usize) -> io::Result<usize> {
    let count = get_varint(bytes, at)? as usize;
    if count > bytes.len().saturating_sub(*at) / each.max(1) + 1 {
        return Err(invalid("A page counts more entries than it holds"));
    }
    Ok(count)
}

fn get_array<const N: usize>(bytes: &[u8], at: &mut usize) -> io::Result<[u8; N]> {
    let held = bytes.get(*at..*at + N).ok_or_else(|| invalid("A page ends inside a digest"))?;
    *at += N;
    Ok(held.try_into().expect("N bytes"))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockHead {
    pub hours: u16,
    pub clock: FrameEntry,
    pub strings: Vec<StringChunk>,
    pub first_string: u32,
    pub chain: [u8; 32],
}

pub const STRING_CHUNK: usize = 1 << 20;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StringChunk {
    pub first: u32,
    pub frame: FrameEntry,
}

impl BlockHead {
    fn write(&self, out: &mut Vec<u8>) {
        put_varint(out, u64::from(self.hours));
        let mut previous = 0_u64;
        put_frame(out, &self.clock, &mut previous);
        put_varint(out, u64::from(self.first_string));
        put_varint(out, self.strings.len() as u64);
        let mut id = self.first_string;
        for chunk in &self.strings {
            put_varint(out, u64::from(chunk.first - id));
            id = chunk.first;
            put_frame(out, &chunk.frame, &mut previous);
        }
        out.extend_from_slice(&self.chain);
    }

    fn read(bytes: &[u8], at: &mut usize) -> io::Result<Self> {
        let hours = get_varint(bytes, at)?;
        if hours == 0 || hours > u64::from(UNIT_ORDINALS) {
            return Err(invalid(format!("A block of {hours} hours")));
        }
        let mut previous = 0_u64;
        let clock = get_frame(bytes, at, &mut previous)?;
        let first_string = u32::try_from(get_varint(bytes, at)?).map_err(|_| invalid("A string id passes 2^32"))?;
        let count = get_varint(bytes, at)? as usize;
        if count > bytes.len() {
            return Err(invalid("A block counts more string chunks than its page holds"));
        }
        let mut strings = Vec::with_capacity(count);
        let mut id = first_string;
        for _ in 0..count {
            id = u32::try_from(u64::from(id) + get_varint(bytes, at)?).map_err(|_| invalid("A string id passes 2^32"))?;
            strings.push(StringChunk { first: id, frame: get_frame(bytes, at, &mut previous)? });
        }
        let chain = get_array(bytes, at)?;
        Ok(Self { hours: hours as u16, clock, strings, first_string, chain })
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Entry {
    pub unit: TableEntry,
    pub summary: Option<Extent>,
}

impl Entry {
    fn write(&self, out: &mut Vec<u8>) -> io::Result<()> {
        self.unit.write(out);
        match &self.summary {
            None => out.push(0),
            Some(extent) => {
                out.push(1);
                put_extent(out, extent)?;
            }
        }
        Ok(())
    }

    fn read(bytes: &[u8], at: &mut usize, run: u32, table: u32) -> io::Result<Self> {
        let unit = TableEntry::read(bytes, at)?;
        let flag = *bytes.get(*at).ok_or_else(|| invalid("A page ends inside a table's entry"))?;
        *at += 1;
        let summary = match flag {
            0 => None,
            1 => {
                let extent = get_extent(bytes, at)?;
                if extent.kind != Kind::Summary || extent.run != run || extent.stream != table {
                    return Err(extent.damaged(&format!("is not table {table}'s summary piece of run {run}")));
                }
                Some(extent)
            }
            other => return Err(invalid(format!("Summary marker {other}"))),
        };
        Ok(Self { unit, summary })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    pub run: u32,
    pub block: u32,
    pub head: BlockHead,
    pub tables: Vec<Entry>,
}

impl Block {
    pub fn chain_after(before: &[u8; 32], body: &[u8]) -> [u8; 32] {
        let mut hasher = blake3::Hasher::new();
        hasher.update(before);
        hasher.update(body);
        *hasher.finalize().as_bytes()
    }

    pub fn body(&self) -> io::Result<Vec<u8>> {
        let mut out = Vec::new();
        put_varint(&mut out, u64::from(self.run));
        put_varint(&mut out, u64::from(self.block));
        let head = BlockHead { chain: [0; 32], ..self.head.clone() };
        head.write(&mut out);
        put_varint(&mut out, self.tables.len() as u64);
        for entry in &self.tables {
            entry.write(&mut out)?;
        }
        Ok(out)
    }

    pub fn read(bytes: &[u8], page: &Extent, tables: usize) -> io::Result<Self> {
        let mut at = 0;
        let run = get_varint(bytes, &mut at)? as u32;
        let block = get_varint(bytes, &mut at)? as u32;
        if page.kind != Kind::BlockPage || page.run != run || u64::from(page.first) != u64::from(block) * u64::from(UNIT_ORDINALS) {
            return Err(page.damaged("holds another block"));
        }
        let head = BlockHead::read(bytes, &mut at)?;
        let count = get_varint(bytes, &mut at)? as usize;
        if count != tables {
            return Err(page.damaged(&format!("names {count} tables, the schema {tables}")));
        }
        let mut entries = Vec::with_capacity(count);
        for table in 0..count {
            entries.push(Entry::read(bytes, &mut at, run, table as u32)?);
        }
        if at != bytes.len() {
            return Err(page.damaged("holds bytes past its tables"));
        }
        Ok(Self { run, block, head, tables: entries })
    }

    pub fn chains_from(&self, before: &[u8; 32]) -> io::Result<bool> {
        Ok(Self::chain_after(before, &self.body()?) == self.head.chain)
    }

    pub fn write(&self) -> io::Result<Vec<u8>> {
        let mut out = Vec::new();
        put_varint(&mut out, u64::from(self.run));
        put_varint(&mut out, u64::from(self.block));
        self.head.write(&mut out);
        put_varint(&mut out, self.tables.len() as u64);
        for entry in &self.tables {
            entry.write(&mut out)?;
        }
        Ok(out)
    }

    pub fn unit_page(&self) -> BlockPage {
        BlockPage { block: self.block, hours: self.head.hours, tables: self.tables.iter().map(|entry| entry.unit.clone()).collect() }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SpanRow {
    pub blocks: Vec<Entry>,
    pub summaries: Vec<(u8, Extent)>,
}

impl SpanRow {
    pub fn write(&self, out: &mut Vec<u8>) -> io::Result<()> {
        put_varint(out, self.blocks.len() as u64);
        for entry in &self.blocks {
            entry.write(out)?;
        }
        put_varint(out, self.summaries.len() as u64);
        for (level, extent) in &self.summaries {
            out.push(*level);
            put_extent(out, extent)?;
        }
        Ok(())
    }

    pub fn read(bytes: &[u8], run: u32, table: u32, blocks: u32) -> io::Result<Self> {
        let mut at = 0;
        let count = get_varint(bytes, &mut at)?;
        if count != u64::from(blocks) {
            return Err(invalid(format!("A span row of {count} blocks in a span of {blocks}")));
        }
        let mut entries = Vec::with_capacity(blocks as usize);
        for _ in 0..blocks {
            entries.push(Entry::read(bytes, &mut at, run, table)?);
        }
        let count = get_count(bytes, &mut at, 2)?;
        let mut summaries = Vec::with_capacity(count);
        for _ in 0..count {
            let level = *bytes.get(at).ok_or_else(|| invalid("A span row ends inside a summary"))?;
            at += 1;
            let extent = get_extent(bytes, &mut at)?;
            if extent.kind != Kind::Summary || extent.run != run || extent.stream != table {
                return Err(extent.damaged(&format!("is not table {table}'s summary piece of run {run}")));
            }
            summaries.push((level, extent));
        }
        if at != bytes.len() {
            return Err(invalid("A span row holds bytes past its end"));
        }
        Ok(Self { blocks: entries, summaries })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpanDirectory {
    pub run: u32,
    pub first_block: u32,
    pub blocks: Vec<BlockHead>,
    pub chain: [u8; 32],
    pub rows: Vec<RowPlace>,
}

impl SpanDirectory {
    pub fn write(&self) -> Vec<u8> {
        let mut out = Vec::new();
        put_varint(&mut out, u64::from(self.run));
        put_varint(&mut out, u64::from(self.first_block));
        put_varint(&mut out, self.blocks.len() as u64);
        for head in &self.blocks {
            head.write(&mut out);
        }
        out.extend_from_slice(&self.chain);
        put_varint(&mut out, self.rows.len() as u64);
        let mut offset = 0_u64;
        for row in &self.rows {
            put_varint(&mut out, row.offset - offset);
            offset = row.offset;
            put_varint(&mut out, u64::from(row.length));
            out.extend_from_slice(&row.digest);
        }
        out
    }

    pub fn read(bytes: &[u8], extent: &Extent, tables: usize) -> io::Result<Self> {
        let mut at = 0;
        let run = get_varint(bytes, &mut at)? as u32;
        let first_block = get_varint(bytes, &mut at)? as u32;
        let count = get_varint(bytes, &mut at)?;
        if extent.kind != Kind::SpanDirectory || run != extent.run || count != u64::from(SPAN_BLOCKS) || first_block % SPAN_BLOCKS != 0 {
            return Err(extent.damaged("is not a span of its run"));
        }
        let mut blocks = Vec::with_capacity(SPAN_BLOCKS as usize);
        for _ in 0..SPAN_BLOCKS {
            blocks.push(BlockHead::read(bytes, &mut at)?);
        }
        let chain = get_array(bytes, &mut at)?;
        let count = get_varint(bytes, &mut at)? as usize;
        if count != tables {
            return Err(extent.damaged(&format!("names {count} tables, the schema {tables}")));
        }
        let mut rows = Vec::with_capacity(count);
        let mut offset = 0_u64;
        for table in 0..count {
            offset += get_varint(bytes, &mut at)?;
            let length = u32::try_from(get_varint(bytes, &mut at)?).map_err(|_| invalid("A span row passes 2^32 bytes"))?;
            let digest = get_array(bytes, &mut at)?;
            rows.push(RowPlace { table: table as u32, offset, length, digest });
        }
        if at != bytes.len() {
            return Err(extent.damaged("holds bytes past its rows"));
        }
        Ok(Self { run, first_block, blocks, chain, rows })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunDirectory {
    pub branch: u64,
    pub epoch: u8,
    pub first_hour: i32,
    pub hours: u32,
    pub chain: [u8; 32],
    pub spans: Vec<SpanPages>,
    pub open: Vec<Extent>,
    pub tail: Vec<(u32, Extent)>,
}

impl RunDirectory {
    pub fn blocks(&self) -> u32 {
        self.hours.div_ceil(UNIT_ORDINALS)
    }

    pub fn write(&self, run: u32) -> io::Result<Vec<u8>> {
        self.validate(run)?;
        self.encode()
    }

    fn encode(&self) -> io::Result<Vec<u8>> {
        let mut out = Vec::new();
        put_varint(&mut out, self.branch);
        out.push(self.epoch);
        put_varint(&mut out, u64::from(((self.first_hour << 1) ^ (self.first_hour >> 31)) as u32));
        put_varint(&mut out, u64::from(self.hours));
        out.extend_from_slice(&self.chain);
        put_varint(&mut out, self.spans.len() as u64);
        for span in &self.spans {
            put_extent(&mut out, &span.directory)?;
            put_extent(&mut out, &span.index)?;
        }
        put_varint(&mut out, self.open.len() as u64);
        for page in &self.open {
            put_extent(&mut out, page)?;
        }
        put_varint(&mut out, self.tail.len() as u64);
        for (table, piece) in &self.tail {
            put_varint(&mut out, u64::from(*table));
            put_extent(&mut out, piece)?;
        }
        Ok(out)
    }

    pub fn read(bytes: &[u8], directory: &Extent, run: u32) -> io::Result<Self> {
        if directory.kind != Kind::RunDirectory || directory.run != run {
            return Err(directory.damaged(&format!("is not run {run}'s directory")));
        }
        let mut at = 0;
        let branch = get_varint(bytes, &mut at)?;
        let epoch = *bytes.get(at).ok_or_else(|| invalid("A run directory ends inside its epoch"))?;
        at += 1;
        let first_hour = get_varint(bytes, &mut at)? as u32;
        let first_hour = ((first_hour >> 1) as i32) ^ -((first_hour & 1) as i32);
        let hours = u32::try_from(get_varint(bytes, &mut at)?).map_err(|_| invalid("A run passes 2^32 hours"))?;
        let chain = get_array(bytes, &mut at)?;
        let count = get_count(bytes, &mut at, 48)?;
        let mut spans = Vec::with_capacity(count);
        for _ in 0..count {
            spans.push(SpanPages { directory: get_extent(bytes, &mut at)?, index: get_extent(bytes, &mut at)? });
        }
        let count = get_count(bytes, &mut at, 24)?;
        let mut open = Vec::with_capacity(count);
        for _ in 0..count {
            open.push(get_extent(bytes, &mut at)?);
        }
        let count = get_count(bytes, &mut at, 25)?;
        let mut tail = Vec::with_capacity(count);
        for _ in 0..count {
            let table = u32::try_from(get_varint(bytes, &mut at)?).map_err(|_| invalid("A table passes 2^32"))?;
            tail.push((table, get_extent(bytes, &mut at)?));
        }
        if at != bytes.len() {
            return Err(directory.damaged("holds bytes past its end"));
        }
        let read = Self { branch, epoch, first_hour, hours, chain, spans, open, tail };
        read.validate(run)?;
        Ok(read)
    }

    fn validate(&self, run: u32) -> io::Result<()> {
        let blocks = self.blocks();
        let span = i64::from(blocks) * i64::from(UNIT_ORDINALS);
        if self.first_hour == i32::MIN || span > i64::from(i32::MAX) || i64::from(self.first_hour) + span.max(1) > i64::from(i32::MAX) {
            return Err(invalid(format!("Run {run}'s hours pass the hour range")));
        }
        if self.spans.len() as u64 != u64::from(blocks / SPAN_BLOCKS) {
            return Err(invalid(format!("Run {run}'s spans do not cover its blocks")));
        }
        for (at, span) in self.spans.iter().enumerate() {
            let first = at as u64 * u64::from(SPAN_BLOCKS * UNIT_ORDINALS);
            if span.directory.kind != Kind::SpanDirectory || span.index.kind != Kind::SpanIndex || span.directory.run != run || u64::from(span.directory.first) != first {
                return Err(span.directory.damaged("is not the run's next span"));
            }
        }
        let open_first = self.spans.len() as u32 * SPAN_BLOCKS;
        if self.open.len() as u32 != blocks - open_first {
            return Err(invalid(format!("Run {run}'s open pages do not cover its blocks")));
        }
        for (at, page) in self.open.iter().enumerate() {
            let block = open_first + at as u32;
            let first = block * UNIT_ORDINALS;
            let hours = (self.hours - first).min(UNIT_ORDINALS);
            if page.kind != Kind::BlockPage || page.run != run || page.first != first || page.hours != hours {
                return Err(page.damaged("is not the run's next block"));
            }
        }
        Ok(())
    }
}
