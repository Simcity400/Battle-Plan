use crate::{
    cpu,
    manifest::{Selection, invalid},
    memory::cell,
};
use replay::columns;
use replay_columnar::{
    manifest::{Manifest, Status},
    state::Cell,
    walk::{Packed, State},
};
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io::{self, BufReader, BufWriter, Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Clone, Default, Deserialize, Serialize)]
pub struct Encoded {
    pub raw_bytes: u64,
    pub zstd3_bytes: u64,
    pub records: u64,
    pub raw_file: PathBuf,
    pub zstd3_file: PathBuf,
}

pub struct Output {
    raw: BufWriter<File>,
    compressed: zstd::stream::write::Encoder<'static, BufWriter<File>>,
    size: Encoded,
}

impl Output {
    pub fn new(path: &Path) -> io::Result<Self> {
        let zstd = path.with_extension(format!(
            "{}.zst",
            path.extension().unwrap_or_default().to_string_lossy()
        ));
        Ok(Self {
            raw: BufWriter::new(File::create_new(path)?),
            compressed: zstd::stream::write::Encoder::new(
                BufWriter::new(File::create_new(&zstd)?),
                3,
            )?,
            size: Encoded {
                raw_file: path.into(),
                zstd3_file: zstd,
                ..Encoded::default()
            },
        })
    }

    pub fn record(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.write_all(&(bytes.len() as u64).to_le_bytes())?;
        self.write_all(bytes)?;
        self.size.records += 1;
        Ok(())
    }

    pub fn finish(mut self) -> io::Result<Encoded> {
        self.raw.flush()?;
        self.compressed.finish()?.flush()?;
        self.size.zstd3_bytes = std::fs::metadata(&self.size.zstd3_file)?.len();
        Ok(self.size)
    }
}

impl Write for Output {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.raw.write_all(bytes)?;
        self.compressed.write_all(bytes)?;
        self.size.raw_bytes += bytes.len() as u64;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        self.raw.flush()?;
        self.compressed.flush()
    }
}

fn blob(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
    out.extend_from_slice(bytes);
}

pub fn value(
    state: &State,
    value: Cell<'_>,
    recorded: bool,
    out: &mut Vec<u8>,
    dictionary: &mut Option<Arc<Packed>>,
) -> io::Result<()> {
    macro_rules! plain {
        ($tag:expr, $value:expr) => {{
            out.push($tag);
            out.extend_from_slice(&$value.to_le_bytes());
        }};
    }
    macro_rules! optional {
        ($tag:expr, $value:expr) => {{
            out.push($tag);
            out.push(u8::from($value.is_some()));
            if let Some(v) = $value {
                out.extend_from_slice(&v.to_le_bytes());
            }
        }};
    }
    match value {
        Cell::U8(v) => plain!(0, v),
        Cell::I8(v) => plain!(1, v),
        Cell::U16(v) => plain!(2, v),
        Cell::I16(v) => plain!(3, v),
        Cell::U32(v) => plain!(4, v),
        Cell::I32(v) => plain!(5, v),
        Cell::U64(v) => plain!(6, v),
        Cell::I64(v) => plain!(7, v),
        Cell::Bool(v) => {
            out.push(8);
            out.push(u8::from(v));
        }
        Cell::OptionalI32(v) => optional!(9, v),
        Cell::OptionalU32(v) => optional!(10, v),
        Cell::OptionalI64(v) => optional!(11, v),
        Cell::OptionalU64(v) => optional!(12, v),
        Cell::Bytes(id) => {
            out.push(13);
            out.extend_from_slice(&id.to_le_bytes());
            if let Some(text) = state.dictionary().resolve(id, dictionary)? {
                out.push(1);
                blob(out, text);
            } else if recorded {
                return Err(invalid(format!("Missing recorded dictionary string {id}")));
            } else {
                out.push(0);
            }
        }
        Cell::U32List(items) => {
            out.push(14);
            out.extend_from_slice(&(items.len() as u64).to_le_bytes());
            for v in items {
                out.extend_from_slice(&v.to_le_bytes());
            }
        }
        Cell::U64List(items) => {
            out.push(15);
            out.extend_from_slice(&(items.len() as u64).to_le_bytes());
            for v in items {
                out.extend_from_slice(&v.to_le_bytes());
            }
        }
        Cell::Opaque(bytes) => {
            out.push(16);
            blob(out, bytes);
        }
    }
    Ok(())
}

fn status(state: &State, selected: &Selection, hour: i32, out: &mut Vec<u8>) -> io::Result<()> {
    header(out, hour, 0, selected.table as u16, 0);
    match columns::status(state, selected.table) {
        Status::NotRecorded => out.push(0),
        Status::Complete => out.push(1),
        Status::Truncated { missing } => {
            out.push(2);
            out.extend_from_slice(&missing.to_le_bytes());
        }
        Status::Unavailable { reason } => {
            out.push(3);
            let mut held = None;
            blob(
                out,
                state
                    .dictionary()
                    .resolve(reason, &mut held)?
                    .ok_or_else(|| invalid("Missing refusal string"))?,
            );
        }
    }
    match state.sampled_at(selected.table.index()) {
        Some(hour) => {
            out.push(1);
            out.extend_from_slice(&hour.to_le_bytes());
        }
        None => out.push(0),
    }
    if let Some(view) = columns::truncated(state, selected.table) {
        out.extend_from_slice(&(view.len() as u64).to_le_bytes());
        let table = columns::table(state, selected.table)?;
        for &slot in view {
            out.extend_from_slice(
                &table
                    .key(slot)
                    .ok_or_else(|| invalid("A truncated view names a missing key"))?
                    .to_le_bytes(),
            );
        }
    } else {
        out.extend_from_slice(&0_u64.to_le_bytes());
    }
    Ok(())
}

fn header(out: &mut Vec<u8>, hour: i32, kind: u8, table: u16, key: u128) {
    out.clear();
    out.extend_from_slice(&hour.to_le_bytes());
    out.push(kind);
    out.extend_from_slice(&table.to_le_bytes());
    out.extend_from_slice(&key.to_le_bytes());
}

fn row(
    state: &State,
    selected: &Selection,
    hour: i32,
    slot: u32,
    fields: impl Iterator<Item = usize>,
    kind: u8,
    out: &mut Vec<u8>,
) -> io::Result<()> {
    let table = columns::table(state, selected.table)?;
    header(
        out,
        hour,
        kind,
        selected.table as u16,
        table
            .key(slot)
            .ok_or_else(|| invalid("Spool row has no live key"))?,
    );
    let mut dictionary = None;
    for field in fields {
        out.extend_from_slice(&(field as u16).to_le_bytes());
        out.push(u8::from(table.recorded(field, slot)));
        value(
            state,
            cell(table, field, slot)?,
            table.recorded(field, slot),
            out,
            &mut dictionary,
        )?;
    }
    Ok(())
}

pub fn checkpoint(
    state: &State,
    selected: &[Selection],
    hour: i32,
    path: &Path,
) -> io::Result<Encoded> {
    let mut output = Output::new(path)?;
    let mut record = Vec::new();
    for selected in selected {
        status(state, selected, hour, &mut record)?;
        output.record(&record)?;
        let table = columns::table(state, selected.table)?;
        for slot in table.live().ones() {
            row(
                state,
                selected,
                hour,
                slot,
                selected.fields.iter().copied(),
                1,
                &mut record,
            )?;
            output.record(&record)?;
        }
    }
    output.finish()
}

pub struct Spool {
    output: Output,
    tables: Vec<Selection>,
    statuses: Vec<Option<Status>>,
    record: Vec<u8>,
}

impl Spool {
    pub fn new(path: &Path, tables: &[Selection]) -> io::Result<Self> {
        Ok(Self {
            output: Output::new(path)?,
            tables: tables.into(),
            statuses: vec![None; tables.len()],
            record: Vec::new(),
        })
    }

    pub fn restored(&mut self, state: &State, hour: i32) -> io::Result<()> {
        for (at, selected) in self.tables.iter().enumerate() {
            self.statuses[at] = Some(columns::status(state, selected.table));
            status(state, selected, hour, &mut self.record)?;
            self.output.record(&self.record)?;
            for slot in columns::table(state, selected.table)?.live().ones() {
                row(
                    state,
                    selected,
                    hour,
                    slot,
                    selected.fields.iter().copied(),
                    1,
                    &mut self.record,
                )?;
                self.output.record(&self.record)?;
            }
        }
        Ok(())
    }

    pub fn hour(
        &mut self,
        state: &State,
        manifest: &Manifest<'_>,
        first: bool,
        supplied: &[bool],
    ) -> io::Result<()> {
        if first
            || manifest.flags
                & (replay_columnar::manifest::AFTER_GAP | replay_columnar::manifest::AFTER_LOAD)
                != 0
        {
            header(
                &mut self.record,
                manifest.hour,
                if first { 5 } else { 6 },
                0,
                0,
            );
            self.record.push(manifest.flags);
            self.output.record(&self.record)?;
        }
        for (at, selected) in self.tables.iter().enumerate() {
            let id = selected.table;
            let changes = columns::changes(state, id)?;
            let now = columns::status(state, id);
            let restated = self.statuses[at] != Some(now);
            if restated || changes.truncated {
                status(state, selected, manifest.hour, &mut self.record)?;
                self.output.record(&self.record)?;
                self.statuses[at] = Some(now);
            }
            let read = if selected.read_rule.as_deref() == Some("acquired") {
                crate::reads::acquired(manifest, id)
            } else {
                supplied[id.index()]
            };
            if read && selected.subscription.contains("wake-on-read") {
                header(&mut self.record, manifest.hour, 4, id as u16, 0);
                self.output.record(&self.record)?;
            }
            let table = columns::table(state, id)?;
            for &key in &changes.deleted_keys {
                header(&mut self.record, manifest.hour, 2, id as u16, key);
                self.output.record(&self.record)?;
            }
            if restated || changes.truncated {
                let slots: Box<dyn Iterator<Item = u32> + '_> =
                    if let Some(view) = columns::truncated(state, id) {
                        Box::new(view.iter().copied())
                    } else if now == Status::Complete {
                        Box::new(table.live().ones())
                    } else {
                        Box::new(std::iter::empty())
                    };
                for slot in slots {
                    row(
                        state,
                        selected,
                        manifest.hour,
                        slot,
                        selected.fields.iter().copied(),
                        1,
                        &mut self.record,
                    )?;
                    self.output.record(&self.record)?;
                }
                continue;
            }
            for &slot in &changes.inserted {
                row(
                    state,
                    selected,
                    manifest.hour,
                    slot,
                    selected.fields.iter().copied(),
                    1,
                    &mut self.record,
                )?;
                self.output.record(&self.record)?;
            }
            for (&slot, &mask) in changes
                .changed
                .iter()
                .zip(&changes.written)
                .chain(changes.fill_slots.iter().zip(&changes.fill_masks))
            {
                if !table.live().get(slot) || changes.inserted.contains(&slot) {
                    continue;
                }
                let fields: Vec<_> = selected
                    .fields
                    .iter()
                    .copied()
                    .filter(|field| mask & (1_u64 << field) != 0)
                    .collect();
                if fields.is_empty() {
                    continue;
                }
                row(
                    state,
                    selected,
                    manifest.hour,
                    slot,
                    fields.into_iter(),
                    3,
                    &mut self.record,
                )?;
                self.output.record(&self.record)?;
            }
        }
        Ok(())
    }

    pub fn finish(self) -> io::Result<Encoded> {
        self.output.finish()
    }
}

#[derive(Deserialize, Serialize)]
pub struct Delivery {
    pub records: u64,
    pub bytes: u64,
    pub cycles: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Event {
    pub hour: i32,
    pub kind: u8,
    pub table: u16,
    pub key: u128,
}

#[derive(Debug, PartialEq)]
pub enum Value<'a> {
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
    Text(u32, Option<&'a [u8]>),
    U32List(Vec<u32>),
    U64List(Vec<u64>),
    Opaque(&'a [u8]),
}

pub trait Consumer {
    fn event(&mut self, event: Event);
    fn cell(&mut self, field: u16, recorded: bool, value: Value<'_>);
    fn status(
        &mut self,
        status: u8,
        missing: Option<u32>,
        reason: Option<&[u8]>,
        sample: Option<i32>,
    );
    fn visible_key(&mut self, key: u128);
    fn flags(&mut self, flags: u8);
}

struct Noop;
impl Consumer for Noop {
    fn event(&mut self, event: Event) {
        std::hint::black_box(event);
    }
    fn cell(&mut self, field: u16, recorded: bool, value: Value<'_>) {
        std::hint::black_box((field, recorded, value));
    }
    fn status(
        &mut self,
        status: u8,
        missing: Option<u32>,
        reason: Option<&[u8]>,
        sample: Option<i32>,
    ) {
        std::hint::black_box((status, missing, reason, sample));
    }
    fn visible_key(&mut self, key: u128) {
        std::hint::black_box(key);
    }
    fn flags(&mut self, flags: u8) {
        std::hint::black_box(flags);
    }
}

fn count(decoder: &mut replay_data::codec::Decoder<'_>, width: usize) -> io::Result<usize> {
    let count = usize::try_from(decoder.u64()?)
        .map_err(|_| invalid("Spool length exceeds address space"))?;
    if count > decoder.remaining() / width {
        return Err(invalid("Spool payload exceeds record"));
    }
    Ok(count)
}
fn bytes<'a>(decoder: &mut replay_data::codec::Decoder<'a>) -> io::Result<&'a [u8]> {
    let length = count(decoder, 1)?;
    decoder.take(length)
}
fn bit(decoder: &mut replay_data::codec::Decoder<'_>) -> io::Result<bool> {
    match decoder.u8()? {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(invalid("Invalid spool boolean")),
    }
}
fn decoded_value<'a>(decoder: &mut replay_data::codec::Decoder<'a>) -> io::Result<Value<'a>> {
    macro_rules! optional {
        ($variant:ident, $method:ident) => {
            Value::$variant(if bit(decoder)? {
                Some(decoder.$method()?)
            } else {
                None
            })
        };
    }
    Ok(match decoder.u8()? {
        0 => Value::U8(decoder.u8()?),
        1 => Value::I8(decoder.i8()?),
        2 => Value::U16(decoder.u16()?),
        3 => Value::I16(decoder.i16()?),
        4 => Value::U32(decoder.u32()?),
        5 => Value::I32(decoder.i32()?),
        6 => Value::U64(decoder.u64()?),
        7 => Value::I64(decoder.i64()?),
        8 => Value::Bool(bit(decoder)?),
        9 => optional!(OptionalI32, i32),
        10 => optional!(OptionalU32, u32),
        11 => optional!(OptionalI64, i64),
        12 => optional!(OptionalU64, u64),
        13 => {
            let id = decoder.u32()?;
            Value::Text(
                id,
                if bit(decoder)? {
                    Some(bytes(decoder)?)
                } else {
                    None
                },
            )
        }
        14 => {
            let n = count(decoder, 4)?;
            Value::U32List((0..n).map(|_| decoder.u32()).collect::<io::Result<_>>()?)
        }
        15 => {
            let n = count(decoder, 8)?;
            Value::U64List((0..n).map(|_| decoder.u64()).collect::<io::Result<_>>()?)
        }
        16 => Value::Opaque(bytes(decoder)?),
        _ => return Err(invalid("Invalid spool cell kind")),
    })
}

pub fn decode(record: &[u8], consumer: &mut impl Consumer) -> io::Result<()> {
    use replay_data::{codec::Decoder, rows::TableId};
    let mut decoder = Decoder::new(record);
    let event = Event {
        hour: decoder.i32()?,
        kind: decoder.u8()?,
        table: decoder.u16()?,
        key: decoder.u128()?,
    };
    let schema = if event.kind <= 4 {
        Some(
            TableId::ALL
                .into_iter()
                .find(|id| *id as u16 == event.table)
                .ok_or_else(|| invalid("Invalid spool table"))?
                .schema(),
        )
    } else {
        None
    };
    consumer.event(event);
    match event.kind {
        0 => {
            let status = decoder.u8()?;
            let (missing, reason) = match status {
                0 | 1 => (None, None),
                2 => (Some(decoder.u32()?), None),
                3 => (None, Some(bytes(&mut decoder)?)),
                _ => return Err(invalid("Invalid spool status")),
            };
            let sample = if bit(&mut decoder)? {
                Some(decoder.i32()?)
            } else {
                None
            };
            consumer.status(status, missing, reason, sample);
            let keys = count(&mut decoder, 16)?;
            for _ in 0..keys {
                consumer.visible_key(decoder.u128()?);
            }
        }
        1 | 3 => {
            let mut previous = None;
            while decoder.remaining() != 0 {
                let field = decoder.u16()?;
                if field as usize >= schema.unwrap().len() || previous.is_some_and(|p| p >= field) {
                    return Err(invalid("Invalid or unordered spool field"));
                }
                previous = Some(field);
                let recorded = bit(&mut decoder)?;
                consumer.cell(field, recorded, decoded_value(&mut decoder)?);
            }
        }
        2 | 4 => {}
        5 | 6 => consumer.flags(decoder.u8()?),
        _ => return Err(invalid("Invalid spool event kind")),
    }
    decoder.finish()
}

pub fn replay(mut reader: impl Read, consumer: &mut impl Consumer) -> io::Result<(u64, u64)> {
    let (mut records, mut total) = (0, 0);
    let mut buffer = Vec::new();
    loop {
        let mut length = [0; 8];
        if reader.read(&mut length[..1])? == 0 {
            break;
        }
        reader.read_exact(&mut length[1..])?;
        let count = usize::try_from(u64::from_le_bytes(length))
            .map_err(|_| invalid("Spool record exceeds address space"))?;
        if count < 23 {
            return Err(invalid("Short spool record"));
        }
        buffer.resize(count, 0);
        reader.read_exact(&mut buffer)?;
        decode(&buffer, consumer)?;
        records += 1;
        total += count as u64 + 8;
    }
    Ok((records, total))
}

pub fn deliver(path: &Encoded) -> io::Result<Delivery> {
    let start = cpu::cycles()?;
    let reader = zstd::stream::read::Decoder::new(BufReader::new(File::open(&path.zstd3_file)?))?;
    let (records, bytes) = replay(reader, &mut Noop)?;
    if records != path.records || bytes != path.raw_bytes {
        return Err(invalid("Spool record/byte count mismatch"));
    }
    Ok(Delivery {
        records,
        bytes,
        cycles: cpu::cycles()?.wrapping_sub(start),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::rows::TableId;

    #[derive(Default)]
    struct Collected {
        events: Vec<Event>,
        cells: Vec<(u16, bool, String)>,
        statuses: Vec<(u8, Option<u32>, Option<Vec<u8>>, Option<i32>)>,
        visible: Vec<u128>,
        flags: Vec<u8>,
    }
    impl Consumer for Collected {
        fn event(&mut self, event: Event) {
            self.events.push(event);
        }
        fn cell(&mut self, field: u16, recorded: bool, value: Value<'_>) {
            self.cells.push((field, recorded, format!("{value:?}")));
        }
        fn status(
            &mut self,
            status: u8,
            missing: Option<u32>,
            reason: Option<&[u8]>,
            sample: Option<i32>,
        ) {
            self.statuses
                .push((status, missing, reason.map(<[u8]>::to_vec), sample));
        }
        fn visible_key(&mut self, key: u128) {
            self.visible.push(key);
        }
        fn flags(&mut self, flags: u8) {
            self.flags.push(flags);
        }
    }
    fn frame(stream: &mut Vec<u8>, record: &[u8]) {
        stream.extend_from_slice(&(record.len() as u64).to_le_bytes());
        stream.extend_from_slice(record);
    }

    #[test]
    fn spool_replay_delivers_headers_every_cell_status_and_breaks() {
        let state = State::new(replay::columnar_frames::schema().unwrap()).unwrap();
        let values = [
            Cell::U8(7),
            Cell::I8(-8),
            Cell::U16(9),
            Cell::I16(-10),
            Cell::U32(11),
            Cell::I32(-12),
            Cell::U64(13),
            Cell::I64(-14),
            Cell::Bool(true),
            Cell::OptionalI32(Some(-15)),
            Cell::OptionalU32(None),
            Cell::OptionalI64(None),
            Cell::OptionalU64(Some(16)),
            Cell::U32List(&[17, 18]),
            Cell::U64List(&[19]),
            Cell::Opaque(b"opaque"),
        ];
        let mut stream = Vec::new();
        let mut record = Vec::new();
        let expected_values = [
            "U8(7)",
            "I8(-8)",
            "U16(9)",
            "I16(-10)",
            "U32(11)",
            "I32(-12)",
            "U64(13)",
            "I64(-14)",
            "Bool(true)",
            "OptionalI32(Some(-15))",
            "OptionalU32(None)",
            "OptionalI64(None)",
            "OptionalU64(Some(16))",
            "U32List([17, 18])",
            "U64List([19])",
            "Opaque([111, 112, 97, 113, 117, 101])",
        ];
        let mut expected = Vec::new();
        for (at, value_cell) in values.into_iter().enumerate() {
            header(
                &mut record,
                100 + at as i32,
                if at == 0 { 1 } else { 3 },
                TableId::Countries as u16,
                at as u128 + 90,
            );
            record.extend_from_slice(&0_u16.to_le_bytes());
            record.push(u8::from(at % 2 == 0));
            let mut encoded = Vec::new();
            value(&state, value_cell, true, &mut encoded, &mut None).unwrap();
            expected.push((0, at % 2 == 0, expected_values[at].to_owned()));
            record.extend_from_slice(&encoded);
            if at == 0 {
                record.extend_from_slice(&1_u16.to_le_bytes());
                record.extend_from_slice(&[1, 13]);
                record.extend_from_slice(&42_u32.to_le_bytes());
                record.push(1);
                blob(&mut record, b"inline");
                expected.push((
                    1,
                    true,
                    "Text(42, Some([105, 110, 108, 105, 110, 101]))".into(),
                ));
            }
            frame(&mut stream, &record);
        }
        for status in 0..4 {
            header(&mut record, 120, 0, TableId::Countries as u16, 0);
            record.push(status);
            if status == 2 {
                record.extend_from_slice(&123_u32.to_le_bytes());
            }
            if status == 3 {
                blob(&mut record, b"refused");
            }
            record.push(1);
            record.extend_from_slice(&119_i32.to_le_bytes());
            record.extend_from_slice(&1_u64.to_le_bytes());
            record.extend_from_slice(&999_u128.to_le_bytes());
            frame(&mut stream, &record);
        }
        for kind in [2, 4, 5, 6] {
            header(&mut record, 121, kind, TableId::Countries as u16, 444);
            if kind >= 5 {
                record.push(3);
            }
            frame(&mut stream, &record);
        }
        let mut consumer = Collected::default();
        let (records, bytes) = replay(&stream[..], &mut consumer).unwrap();
        assert_eq!(records, 24);
        assert_eq!(bytes, stream.len() as u64);
        assert_eq!(consumer.cells, expected);
        assert_eq!(
            consumer.events[0],
            Event {
                hour: 100,
                kind: 1,
                table: TableId::Countries as u16,
                key: 90
            }
        );
        assert_eq!(consumer.events[15].hour, 115);
        assert_eq!(consumer.events[23].kind, 6);
        assert_eq!(consumer.visible, [999; 4]);
        assert_eq!(consumer.statuses[2], (2, Some(123), None, Some(119)));
        assert_eq!(
            consumer.statuses[3],
            (3, None, Some(b"refused".to_vec()), Some(119))
        );
        assert_eq!(consumer.flags, [3, 3]);
        assert!(replay(&stream[..stream.len() - 1], &mut Noop).is_err());
        header(&mut record, 0, 2, TableId::Countries as u16, 0);
        record.push(0);
        assert!(decode(&record, &mut Noop).is_err());
        record[4] = 99;
        assert!(decode(&record, &mut Noop).is_err());
    }
}
