use crate::{
    cpu,
    manifest::{Selection, invalid},
    memory::cell,
    readset::{Filter, Mode, ReadSet},
};
use replay::columns;
use replay_columnar::{
    manifest::{Manifest, Status},
    state::Cell,
    walk::{Packed, State},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::File,
    io::{self, BufReader, BufWriter, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Clone, Default, Deserialize, Serialize)]
pub struct Encoded {
    pub raw_bytes: u64,
    pub zstd3_bytes: u64,
    pub records: u64,
    pub active_hours: BTreeMap<String, u64>,
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

    pub(crate) fn position(&self) -> u64 {
        self.size.raw_bytes
    }

    pub(crate) fn read_record(
        &mut self,
        file: &mut File,
        offset: u64,
        bytes: &mut Vec<u8>,
    ) -> io::Result<()> {
        if offset + 8 > self.size.raw_bytes - self.raw.buffer().len() as u64 {
            self.raw.flush()?;
        }
        file.seek(SeekFrom::Start(offset))?;
        let mut length = [0; 8];
        file.read_exact(&mut length)?;
        let length = u64::from_le_bytes(length);
        let end = offset
            .checked_add(8)
            .and_then(|at| at.checked_add(length))
            .filter(|&end| end <= self.size.raw_bytes)
            .ok_or_else(|| invalid("Invalid delivered row offset"))?;
        if end > self.size.raw_bytes - self.raw.buffer().len() as u64 {
            self.raw.flush()?;
        }
        let length =
            usize::try_from(length).map_err(|_| invalid("Delivered row exceeds address space"))?;
        bytes.resize(length, 0);
        file.read_exact(bytes)
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

pub(crate) fn status(
    state: &State,
    selected: &Selection,
    hour: i32,
    out: &mut Vec<u8>,
    filter: Option<(&Filter, usize)>,
) -> io::Result<()> {
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
        let count_at = out.len();
        out.extend_from_slice(&0_u64.to_le_bytes());
        let mut count = 0_u64;
        let table = columns::table(state, selected.table)?;
        for &slot in view {
            let key = table
                .key(slot)
                .ok_or_else(|| invalid("A truncated view names a missing key"))?;
            if let Some((filter, at)) = filter {
                if !filter.contains(at, key)?
                    || !filter.readset.tables[at].accepts(state, slot, hour)?
                {
                    continue;
                }
            }
            out.extend_from_slice(&key.to_le_bytes());
            count += 1;
        }
        out[count_at..count_at + 8].copy_from_slice(&count.to_le_bytes());
    } else {
        out.extend_from_slice(&0_u64.to_le_bytes());
    }
    Ok(())
}

pub(crate) fn header(out: &mut Vec<u8>, hour: i32, kind: u8, table: u16, key: u128) {
    out.clear();
    out.extend_from_slice(&hour.to_le_bytes());
    out.push(kind);
    out.extend_from_slice(&table.to_le_bytes());
    out.extend_from_slice(&key.to_le_bytes());
}

pub(crate) fn row(
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
        status(state, selected, hour, &mut record, None)?;
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

fn presence(
    state: &State,
    selected: &Selection,
    hour: i32,
    previous: &mut Option<u8>,
    prior_status: &mut Option<Status>,
    filter: (&Filter, usize),
    record: &mut Vec<u8>,
    output: &mut Output,
) -> io::Result<()> {
    let now = columns::status(state, selected.table);
    let value = if now != Status::Complete {
        0
    } else if columns::table(state, selected.table)?
        .live()
        .ones()
        .next()
        .is_some()
    {
        2
    } else {
        1
    };
    if *prior_status != Some(now) {
        status(state, selected, hour, record, Some(filter))?;
        output.record(record)?;
        *prior_status = Some(now);
    }
    if *previous != Some(value) {
        header(record, hour, 7, selected.table as u16, 0);
        record.push(value);
        output.record(record)?;
        *previous = Some(value);
    }
    Ok(())
}

pub struct Spool {
    output: Output,
    tables: Vec<Selection>,
    statuses: Vec<Option<Status>>,
    presence: Vec<Option<u8>>,
    record: Vec<u8>,
    filter: Option<Filter>,
    history: Vec<Option<crate::history::History>>,
    gated: Vec<Option<crate::gated::Gated>>,
}

impl Spool {
    pub fn new(path: &Path, tables: &[Selection]) -> io::Result<Self> {
        let mut output = Output::new(path)?;
        output.size.active_hours.insert("every".into(), 0);
        Ok(Self {
            output,
            tables: tables.into(),
            statuses: vec![None; tables.len()],
            presence: vec![None; tables.len()],
            record: Vec::new(),
            filter: None,
            history: (0..tables.len()).map(|_| None).collect(),
            gated: (0..tables.len()).map(|_| None).collect(),
        })
    }

    pub fn filtered(
        path: &Path,
        readset: &ReadSet,
        format: replay_data::format::FormatVersion,
        cache: &crate::keyset::Cache,
    ) -> io::Result<Self> {
        let tables: Vec<_> = readset.tables.iter().map(|t| t.selected.clone()).collect();
        let mut spool = Self::new(path, &tables)?;
        spool.output.size.active_hours.clear();
        spool.filter = Some(Filter::new(path, readset, format, cache)?);
        let historical = readset.historical_tables();
        for (at, entry) in readset.tables.iter().enumerate() {
            if entry.mode != Mode::None {
                let rule = entry.hours.unwrap_or(crate::readset::Hours::Every);
                spool
                    .output
                    .size
                    .active_hours
                    .entry(rule.name().into())
                    .or_default();
            }
            if entry.hours.is_some() {
                spool.gated[at] = Some(crate::gated::Gated::new(
                    &path.with_file_name(format!(
                        "{}.{}.gated",
                        path.file_name().unwrap().to_string_lossy(),
                        at
                    )),
                    path,
                    cache,
                )?);
            }
            if historical[at] {
                let history = path.with_file_name(format!(
                    "{}.{}",
                    path.file_name().unwrap().to_string_lossy(),
                    at
                ));
                spool.history[at] = Some(crate::history::History::new(&history, cache)?);
            }
        }
        Ok(spool)
    }

    pub fn restored(&mut self, state: &State, hour: i32) -> io::Result<()> {
        for (selected, history) in self.tables.iter().zip(&mut self.history) {
            if let Some(history) = history {
                history.capture(state, selected, hour, true)?;
            }
        }
        if let Some(filter) = &mut self.filter {
            filter.refresh(state, true, hour)?;
        }
        for (at, selected) in self.tables.iter().enumerate() {
            if self
                .filter
                .as_ref()
                .is_some_and(|f| f.readset.tables[at].mode == Mode::None)
            {
                continue;
            }
            if let Some(filter) = &self.filter {
                let entry = &filter.readset.tables[at];
                if entry.mode == Mode::Snapshot {
                    continue;
                }
                if entry.mode == Mode::Presence {
                    presence(
                        state,
                        selected,
                        hour,
                        &mut self.presence[at],
                        &mut self.statuses[at],
                        (filter, at),
                        &mut self.record,
                        &mut self.output,
                    )?;
                    continue;
                }
            }
            self.statuses[at] = Some(columns::status(state, selected.table));
            status(
                state,
                selected,
                hour,
                &mut self.record,
                self.filter.as_ref().map(|f| (f, at)),
            )?;
            self.output.record(&self.record)?;
            for slot in columns::table(state, selected.table)?.live().ones() {
                if let Some(filter) = &self.filter {
                    let key = columns::table(state, selected.table)?
                        .key(slot)
                        .ok_or_else(|| invalid("Missing spool key"))?;
                    if !filter.contains(at, key)? {
                        continue;
                    }
                }
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
        for (selected, history) in self.tables.iter().zip(&mut self.history) {
            if let Some(history) = history {
                history.capture(state, selected, manifest.hour, first)?;
            }
        }
        if let Some(filter) = &mut self.filter {
            filter.refresh(state, first, manifest.hour)?;
        }
        if let Some(filter) = &self.filter {
            let mut counted = [false; 4];
            for (at, entry) in filter.readset.tables.iter().enumerate() {
                if entry.mode == Mode::None {
                    continue;
                }
                let rule = entry.hours.unwrap_or(crate::readset::Hours::Every);
                if filter.active(at) && !counted[rule as usize] {
                    *self.output.size.active_hours.get_mut(rule.name()).unwrap() += 1;
                    counted[rule as usize] = true;
                }
            }
        } else {
            *self.output.size.active_hours.get_mut("every").unwrap() += 1;
        }
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
            if self
                .filter
                .as_ref()
                .is_some_and(|f| f.readset.tables[at].mode == Mode::None)
            {
                continue;
            }
            if let Some(filter) = &self.filter {
                let entry = &filter.readset.tables[at];
                if entry.mode == Mode::Presence {
                    presence(
                        state,
                        selected,
                        manifest.hour,
                        &mut self.presence[at],
                        &mut self.statuses[at],
                        (filter, at),
                        &mut self.record,
                        &mut self.output,
                    )?;
                    continue;
                }
                if entry.hours.is_some() {
                    self.gated[at].as_mut().unwrap().hour(
                        state,
                        selected,
                        manifest.hour,
                        first,
                        (filter, at),
                        &mut self.record,
                        &mut self.output,
                    )?;
                    continue;
                }
            }
            if let (Some(filter), Some(history)) = (&self.filter, &mut self.history[at]) {
                history.hydrate(
                    selected.table as u16,
                    manifest.hour,
                    (filter, at),
                    &mut self.output,
                )?;
            }
            let id = selected.table;
            let changes = columns::changes(state, id)?;
            let now = columns::status(state, id);
            let restated = self.statuses[at] != Some(now);
            if restated || changes.truncated {
                status(
                    state,
                    selected,
                    manifest.hour,
                    &mut self.record,
                    self.filter.as_ref().map(|f| (f, at)),
                )?;
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
                if let Some(filter) = &self.filter {
                    if !filter.contained(at, key)? {
                        continue;
                    }
                }
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
                    if let Some(filter) = &self.filter {
                        let key = table
                            .key(slot)
                            .ok_or_else(|| invalid("Missing spool key"))?;
                        if !filter.contains(at, key)? {
                            continue;
                        }
                    }
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
                if let Some(filter) = &self.filter {
                    let key = table
                        .key(slot)
                        .ok_or_else(|| invalid("Missing spool key"))?;
                    if !filter.contains(at, key)? {
                        continue;
                    }
                }
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
            if let Some(filter) = &self.filter {
                if filter.readset.tables[at].mode == Mode::Referenced {
                    let mut next = Some(0);
                    while let Some(start) = next {
                        let Some(key) = filter.next_membership(at, start)? else {
                            break;
                        };
                        next = key.checked_add(1);
                        let now = filter.contains(at, key)?;
                        let before = filter.contained(at, key)?;
                        if now && !before {
                            if let Some(slot) = table
                                .slot(key)
                                .filter(|slot| !changes.inserted.contains(slot))
                            {
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
                        } else if before && !now {
                            header(&mut self.record, manifest.hour, 2, id as u16, key);
                            self.output.record(&self.record)?;
                        }
                    }
                }
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
                if let Some(filter) = &self.filter {
                    let key = table
                        .key(slot)
                        .ok_or_else(|| invalid("Missing changed key"))?;
                    if !filter.contains(at, key)? || !filter.contained(at, key)? {
                        continue;
                    }
                }
                let fields = selected
                    .fields
                    .iter()
                    .copied()
                    .filter(|field| mask & (1_u64 << field) != 0);
                if fields.clone().next().is_none() {
                    continue;
                }
                row(
                    state,
                    selected,
                    manifest.hour,
                    slot,
                    fields,
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
    fn presence(&mut self, value: u8) {
        std::hint::black_box(value);
    }
    fn dated_hour(&mut self, hour: i32) {
        std::hint::black_box(hour);
    }
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
    let schema = if event.kind <= 4 || matches!(event.kind, 7..=11) {
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
    if matches!(event.kind, 9..=11) {
        consumer.dated_hour(decoder.i32()?);
    }
    match event.kind {
        0 | 10 => {
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
        1 | 3 | 9 => {
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
        2 | 4 | 11 => {}
        5 | 6 => consumer.flags(decoder.u8()?),
        7 => {
            let presence = decoder.u8()?;
            if presence > 2 {
                return Err(invalid("Invalid presence state"));
            }
            consumer.presence(presence);
        }
        8 => {}
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
        presences: Vec<(i32, u8)>,
        dates: Vec<i32>,
    }
    impl Consumer for Collected {
        fn presence(&mut self, value: u8) {
            self.presences
                .push((self.events.last().unwrap().hour, value));
        }
        fn dated_hour(&mut self, hour: i32) {
            self.dates.push(hour);
        }
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
    fn presence_churn_is_coalesced_and_hourly_snapshots_expire_rows() {
        use replay_columnar::{
            manifest::Section,
            section::{self, TableHour},
            state::Values,
        };
        let text = "version=1\nfamilies_source='synthetic'\n[[families]]\nname='synthetic'\norigin='code'\nordered=true\n[[families.tables]]\nname='CountryLinks'\nfields=['canonical_slot']\nsubscription='stream'\nsource='synthetic'\n[[families.tables]]\nname='SliceReads'\nfields=['sample_hour']\nsubscription='stream'\nsource='synthetic'\n[[families.readset]]\nname='CountryLinks'\nfields=[]\nspool='presence'\nsource='synthetic'\n[[families.readset]]\nname='SliceReads'\nfields=['sample_hour']\nspool='snapshot'\nhours='every'\nfilter='relation-slices'\nsource='synthetic'";
        let resolved = toml::from_str::<crate::manifest::Manifest>(text)
            .unwrap()
            .resolve()
            .unwrap();
        let path = crate::synthetic_tests::folder().join("presence-snapshots.spool");
        let cache = crate::keyset::Cache::reserved("synthetic spool", 32 * 1024).unwrap();
        let mut spool = Spool::filtered(
            &path,
            resolved.readsets[0].as_ref().unwrap(),
            replay_data::format::FormatVersion::CURRENT,
            &cache,
        )
        .unwrap();
        let schema = replay::columnar_frames::schema().unwrap();
        let mut state = State::new(schema.clone()).unwrap();
        for hour in 10..=13 {
            let mut manifest = Manifest {
                branch: 1,
                hour,
                flags: 0,
                total_ns: 0,
                pool_bytes: 0,
                dropped_frames: 0,
                appends: vec![],
                statuses: vec![Status::Complete; TableId::ALL.len()],
                read: vec![0; TableId::ALL.len().div_ceil(64)],
                samples: vec![],
                sections: vec![],
            };
            let (inserted, keys, deleted): (&[u32], &[u128], &[u32]) = match hour {
                10 => (&[0, 1], &[10, 20], &[]),
                11 => (&[2], &[30], &[0]),
                12 => (&[], &[], &[1, 2]),
                _ => (&[], &[], &[]),
            };
            let values = if hour == 10 {
                vec![Values::U32(&[1, 2])]
            } else if hour == 11 {
                vec![Values::U32(&[3])]
            } else {
                vec![]
            };
            let links = section::write(
                &schema[TableId::CountryLinks.index()],
                &TableHour {
                    read: None,
                    changed: &[],
                    deleted,
                    inserted,
                    keys,
                    inserts: &values,
                    runs: &[],
                    touched: &[],
                    unrecorded: &[],
                },
            )
            .unwrap();
            let relation = (TableId::RelationTuples as u128) << 32 | 1;
            let other = (TableId::Divisions as u128) << 32 | 2;
            let slice_keys = [relation, other];
            let reads = section::write(
                &schema[TableId::SliceReads.index()],
                &TableHour {
                    read: None,
                    changed: &[],
                    deleted: &[],
                    inserted: if hour == 10 { &[0, 1] } else { &[] },
                    keys: if hour == 10 { &slice_keys } else { &[] },
                    inserts: if hour == 10 {
                        &[Values::I32(&[10, 10])]
                    } else {
                        &[]
                    },
                    runs: &[],
                    touched: &[],
                    unrecorded: &[],
                },
            )
            .unwrap();
            if hour == 13 {
                manifest.statuses[TableId::CountryLinks.index()] = Status::NotRecorded;
            }
            for (id, bytes) in [
                (TableId::CountryLinks, &links),
                (TableId::SliceReads, &reads),
            ] {
                manifest.sections.push(Section {
                    table: id.index(),
                    bytes: bytes.len() as u64,
                    columns: vec![1],
                    ops: 0,
                });
            }
            let sections = [&links, &reads];
            state.apply(&manifest, |at| Ok(sections[at])).unwrap();
            spool
                .hour(
                    &state,
                    &manifest,
                    hour == 10,
                    &vec![false; TableId::ALL.len()],
                )
                .unwrap();
        }
        let encoded = spool.finish().unwrap();
        let mut collected = Collected::default();
        replay(
            BufReader::new(File::open(encoded.raw_file).unwrap()),
            &mut collected,
        )
        .unwrap();
        assert_eq!(collected.presences, [(10, 2), (12, 1), (13, 0)]);
        assert!(
            !collected
                .events
                .iter()
                .any(|e| e.table == TableId::CountryLinks as u16 && matches!(e.kind, 1..=3))
        );
        let rows: Vec<_> = collected
            .events
            .iter()
            .filter(|e| e.table == TableId::SliceReads as u16 && e.kind == 1)
            .collect();
        assert_eq!(rows.len(), 1);
        assert_eq!(
            (rows[0].hour, rows[0].key),
            (10, (TableId::RelationTuples as u128) << 32 | 1)
        );
        assert_eq!(collected.events.iter().filter(|e| e.kind == 8).count(), 4);
    }

    #[test]
    fn gated_own_and_upper_bound_deliver_only_net_changes_at_active_hours() {
        use crate::readset::{Hours, ReadSet};
        use replay_columnar::{
            manifest::Section,
            section::{self, Op, Run, TableHour},
            state::Values,
        };
        let text = "version=1\nfamilies_source='synthetic'\n[[families]]\nname='synthetic'\norigin='code'\nordered=true\n[[families.tables]]\nname='CountryLinks'\nfields=['canonical_slot']\nsubscription='stream'\nsource='synthetic'\n[[families.readset]]\nname='CountryLinks'\nfields=['canonical_slot']\nspool='snapshot'\nhours='ledger-dated'\nsource='synthetic'";
        let resolved = toml::from_str::<crate::manifest::Manifest>(text)
            .unwrap()
            .resolve()
            .unwrap();
        let path = crate::synthetic_tests::folder();
        let cache = crate::keyset::Cache::reserved("synthetic spool", 32 * 1024).unwrap();
        let mut own = Spool::filtered(
            &path.join("own.spool"),
            resolved.readsets[0].as_ref().unwrap(),
            replay_data::format::FormatVersion::CURRENT,
            &cache,
        )
        .unwrap();
        let mut upper = Spool::filtered(
            &path.join("upper.spool"),
            &ReadSet::upper_bound(&resolved.families[0], Hours::LedgerDated),
            replay_data::format::FormatVersion::CURRENT,
            &cache,
        )
        .unwrap();
        let mut scoped = Spool::filtered(
            &path.join("scoped.spool"),
            &ReadSet::upper_bound(&resolved.families[0], Hours::ScopedProjects),
            replay_data::format::FormatVersion::CURRENT,
            &cache,
        )
        .unwrap();
        let schema = replay::columnar_frames::schema().unwrap();
        let mut state = State::new(schema.clone()).unwrap();
        for hour in 10..=21 {
            let mut manifest = Manifest {
                branch: 1,
                hour,
                flags: 0,
                total_ns: 0,
                pool_bytes: 0,
                dropped_frames: 0,
                appends: vec![],
                statuses: vec![Status::Complete; TableId::ALL.len()],
                read: vec![0; TableId::ALL.len().div_ceil(64)],
                samples: vec![],
                sections: vec![],
            };
            if matches!(hour, 13 | 14 | 16 | 17 | 19 | 21) {
                manifest.statuses[TableId::PoliticsCaptureCoverage.index()] = Status::NotRecorded;
            }
            let initial = hour == 10 || hour == 15;
            let value = if hour == 10 {
                1
            } else if hour == 11 {
                2
            } else if matches!(hour, 18 | 20) {
                9
            } else {
                3
            };
            let runs = [Run {
                column: 0,
                op: Op::Set,
                positions: &[0],
                values: Some(Values::U32(std::slice::from_ref(&value))),
                except: &[],
                patches: None,
            }];
            let unknown = [(0, [0].as_slice())];
            let inserts = [Values::U32(std::slice::from_ref(&value))];
            let bytes = section::write(
                &schema[TableId::CountryLinks.index()],
                &TableHour {
                    read: None,
                    changed: if matches!(hour, 11 | 12 | 14 | 16 | 18 | 19 | 20 | 21) {
                        &[0]
                    } else {
                        &[]
                    },
                    deleted: if hour == 15 { &[0] } else { &[] },
                    inserted: if initial { &[0] } else { &[] },
                    keys: if hour == 10 {
                        &[10]
                    } else if hour == 15 {
                        &[20]
                    } else {
                        &[]
                    },
                    inserts: if initial { &inserts } else { &[] },
                    runs: if matches!(hour, 11 | 12 | 14 | 18 | 19 | 20 | 21) {
                        &runs
                    } else {
                        &[]
                    },
                    touched: &[],
                    unrecorded: if hour == 16 { &unknown } else { &[] },
                },
            )
            .unwrap();
            manifest.sections.push(Section {
                table: TableId::CountryLinks.index(),
                bytes: bytes.len() as u64,
                columns: vec![1],
                ops: 0,
            });
            state.apply(&manifest, |_| Ok(&bytes)).unwrap();
            for spool in [&mut own, &mut upper, &mut scoped] {
                spool
                    .hour(
                        &state,
                        &manifest,
                        hour == 10,
                        &[false; replay_data::rows::TABLE_COUNT],
                    )
                    .unwrap();
            }
        }
        let own = own.finish().unwrap();
        let upper = upper.finish().unwrap();
        let scoped = scoped.finish().unwrap();
        assert_eq!(scoped.active_hours["scoped-projects"], 12);
        let mut scoped_rows = Collected::default();
        replay(
            BufReader::new(File::open(&scoped.raw_file).unwrap()),
            &mut scoped_rows,
        )
        .unwrap();
        assert!(
            !scoped_rows
                .events
                .iter()
                .any(|event| event.hour == 14 && matches!(event.kind, 1..=3))
        );
        assert_eq!(own.records, upper.records);
        assert_eq!(own.raw_bytes, upper.raw_bytes);
        assert_eq!(own.active_hours["ledger-dated"], 6);
        let mut rows = Collected::default();
        replay(
            BufReader::new(File::open(&own.raw_file).unwrap()),
            &mut rows,
        )
        .unwrap();
        let events: Vec<_> = rows
            .events
            .iter()
            .filter(|event| matches!(event.kind, 1..=3))
            .map(|event| (event.hour, event.kind, event.key))
            .collect();
        assert_eq!(events, [(13, 1, 10), (16, 2, 10), (16, 1, 20), (19, 1, 20)]);
        assert_eq!(
            rows.cells.iter().map(|cell| cell.1).collect::<Vec<_>>(),
            [true, false, true]
        );
        assert_eq!(
            rows.events.iter().filter(|event| event.kind == 8).count(),
            6
        );
    }

    #[test]
    fn filtered_spool_hydrates_unchanged_targets_releases_and_preserves_unknowns() {
        use replay_columnar::{
            manifest::Section,
            section::{self, Op, Run, TableHour},
            state::Values,
        };
        use replay_data::format::FormatVersion;
        let text = "version=1\nfamilies_source='synthetic'\n[[families]]\nname='synthetic'\norigin='code'\nordered=true\n[[families.tables]]\nname='CountryLinks'\nfields=['canonical_slot']\nsubscription='stream'\nsource='synthetic'\n[[families.tables]]\nname='LexicalTokens'\nfields=['literal']\nsubscription='lookup'\nsource='synthetic'\n[[families.readset]]\nname='CountryLinks'\nfields=['canonical_slot']\nspool='stream'\nsource='synthetic'\n[[families.readset]]\nname='LexicalTokens'\nfields=['literal']\nspool='referenced'\nprevious=true\nsource='synthetic'\n[[families.references]]\nfrom='CountryLinks.canonical_slot'\nto='LexicalTokens'\nsource='synthetic'";
        let manifest = toml::from_str::<crate::manifest::Manifest>(text)
            .unwrap()
            .resolve()
            .unwrap();
        let readset = manifest.readsets[0].as_ref().unwrap();
        let path = crate::synthetic_tests::folder().join("filtered.spool");
        let cache = crate::keyset::Cache::reserved("synthetic spool", 32 * 1024).unwrap();
        let mut spool = Spool::filtered(&path, readset, FormatVersion::CURRENT, &cache).unwrap();
        let schema = replay::columnar_frames::schema().unwrap();
        let mut state = State::new(schema.clone()).unwrap();
        let base = |hour| Manifest {
            branch: 1,
            hour,
            flags: 0,
            total_ns: 0,
            pool_bytes: 0,
            dropped_frames: 0,
            appends: vec![],
            statuses: vec![Status::Complete; TableId::ALL.len()],
            read: vec![0; TableId::ALL.len().div_ceil(64)],
            samples: vec![],
            sections: vec![],
        };
        let insert = |id: TableId, keys: &[u128], slots: &[u32], values: &[Values<'_>]| {
            section::write(
                &schema[id.index()],
                &TableHour {
                    read: None,
                    changed: &[],
                    deleted: &[],
                    inserted: slots,
                    keys,
                    inserts: values,
                    runs: &[],
                    touched: &[],
                    unrecorded: &[],
                },
            )
            .unwrap()
        };
        let source = insert(TableId::CountryLinks, &[10], &[0], &[Values::U32(&[1])]);
        let targets = insert(
            TableId::LexicalTokens,
            &[1, 2, 99],
            &[0, 1, 2],
            &[Values::Bytes(&[0, 1, 2])],
        );
        let section_entry = |id: TableId, bytes: &[u8]| Section {
            table: id.index(),
            bytes: bytes.len() as u64,
            columns: vec![1],
            ops: 0,
        };
        let mut first = base(10);
        first.appends = vec![b"one", b"two", b"unreachable", b"updated"];
        first.sections = vec![
            section_entry(TableId::CountryLinks, &source),
            section_entry(TableId::LexicalTokens, &targets),
        ];
        let sections = [&source, &targets];
        state.apply(&first, |at| Ok(sections[at])).unwrap();
        spool.restored(&state, 10).unwrap();
        for (hour, id, value, unknown) in [
            (11, TableId::CountryLinks, 2, false),
            (12, TableId::LexicalTokens, 3, false),
            (13, TableId::LexicalTokens, 1, true),
            (14, TableId::CountryLinks, 1, false),
        ] {
            let slot = if hour == 13 { 1 } else { 0 };
            let values = if id == TableId::CountryLinks {
                Values::U32(std::slice::from_ref(&value))
            } else {
                Values::Bytes(std::slice::from_ref(&value))
            };
            let runs = [Run {
                column: 0,
                op: Op::Set,
                positions: &[0],
                values: Some(values),
                except: &[],
                patches: None,
            }];
            let unknown_slots = [slot];
            let unrecorded = [(0, unknown_slots.as_slice())];
            let bytes = section::write(
                &schema[id.index()],
                &TableHour {
                    read: None,
                    changed: &[slot],
                    deleted: &[],
                    inserted: &[],
                    keys: &[],
                    inserts: &[],
                    runs: if unknown { &[] } else { &runs },
                    touched: &[],
                    unrecorded: if unknown { &unrecorded } else { &[] },
                },
            )
            .unwrap();
            let mut manifest = base(hour);
            manifest.sections = vec![section_entry(id, &bytes)];
            state.apply(&manifest, |_| Ok(&bytes)).unwrap();
            spool
                .hour(
                    &state,
                    &manifest,
                    false,
                    &[false; replay_data::rows::TABLE_COUNT],
                )
                .unwrap();
        }
        let partial = section::write(
            &schema[TableId::LexicalTokens.index()],
            &TableHour {
                read: Some(&[1]),
                changed: &[],
                deleted: &[],
                inserted: &[],
                keys: &[],
                inserts: &[],
                runs: &[],
                touched: &[],
                unrecorded: &[],
            },
        )
        .unwrap();
        let mut truncated = base(15);
        truncated.statuses[TableId::LexicalTokens.index()] = Status::Truncated { missing: 42 };
        truncated.sections = vec![section_entry(TableId::LexicalTokens, &partial)];
        state.apply(&truncated, |_| Ok(&partial)).unwrap();
        spool
            .hour(
                &state,
                &truncated,
                false,
                &[false; replay_data::rows::TABLE_COUNT],
            )
            .unwrap();
        let encoded = spool.finish().unwrap();
        let mut collected = Collected::default();
        replay(File::open(&encoded.raw_file).unwrap(), &mut collected).unwrap();
        let lexical: Vec<_> = collected
            .events
            .iter()
            .filter(|e| e.table == TableId::LexicalTokens as u16 && matches!(e.kind, 1..=3))
            .map(|e| (e.hour, e.kind, e.key))
            .collect();
        assert_eq!(
            lexical,
            [
                (10, 1, 1),
                (11, 2, 1),
                (11, 1, 2),
                (13, 3, 2),
                (14, 1, 1),
                (14, 2, 2)
            ]
        );
        assert!(
            collected
                .cells
                .iter()
                .any(|(_, recorded, value)| !recorded && value.contains("Text(1,"))
        );
        assert!(
            collected
                .cells
                .iter()
                .any(|(_, _, value)| value.contains("117, 112, 100, 97, 116, 101, 100"))
        );
        assert!(
            collected
                .statuses
                .iter()
                .any(|s| s.0 == 2 && s.1 == Some(42))
        );
        assert!(collected.visible.is_empty());
        assert!(collected.dates.contains(&10));
        assert!(collected.dates.contains(&13));
        assert!(
            collected
                .events
                .iter()
                .any(|event| event.kind == 9 && event.key == 2)
        );
    }

    #[test]
    fn delayed_held_payment_hydrates_deleted_leader_and_historical_reference_closure() {
        use replay_columnar::{
            manifest::Section,
            section::{self, Op, Run, TableHour},
            state::{Kind, Values},
        };
        use replay_data::{
            format::FormatVersion,
            rows::{PowerIdentity, PowerObjectKind},
            value::Value as _,
        };
        let text = r#"
version=1
families_source='synthetic'
[[families]]
name='synthetic'
origin='code'
ordered=true
[[families.tables]]
name='CountryPowerChanges'
fields=['identity', 'hour']
subscription='stream'
source='synthetic'
[[families.tables]]
name='UnitLeaders'
fields=['character_id', 'category']
subscription='lookup'
source='synthetic'
[[families.tables]]
name='Characters'
fields=['name_token_id', 'template_tag']
subscription='lookup'
source='synthetic'
[[families.tables]]
name='AdvisorRoles'
fields=[]
subscription='lookup'
source='synthetic'
[[families.tables]]
name='CountryLinks'
fields=['canonical_slot']
subscription='lookup'
source='synthetic'
[[families.tables]]
name='LexicalTokens'
fields=['literal']
subscription='lookup'
source='synthetic'
[[families.readset]]
name='CountryPowerChanges'
fields=['identity', 'hour']
spool='stream'
source='synthetic'
[[families.readset]]
name='UnitLeaders'
fields=['character_id', 'category']
spool='referenced'
previous=true
source='synthetic'
[[families.readset]]
name='Characters'
fields=['name_token_id', 'template_tag']
spool='referenced'
source='synthetic'
[[families.readset]]
name='AdvisorRoles'
fields=[]
spool='referenced'
source='synthetic'
[[families.readset]]
name='CountryLinks'
fields=['canonical_slot']
spool='referenced'
source='synthetic'
[[families.readset]]
name='LexicalTokens'
fields=['literal']
spool='referenced'
source='synthetic'
[[families.references]]
from='CountryPowerChanges.identity'
to='UnitLeaders'
values='power-leader'
source='synthetic'
[[families.references]]
from='CountryPowerChanges.identity'
to='UnitLeaders'
values='power-character'
lookup_field='character_id'
source='synthetic'
[[families.references]]
from='UnitLeaders.character_id'
to='Characters'
source='synthetic'
[[families.references]]
from='Characters.$key'
to='AdvisorRoles'
target_shift=32
source='synthetic'
[[families.references]]
from='Characters.name_token_id'
to='LexicalTokens'
source='synthetic'
[[families.references]]
from='Characters.template_tag'
to='CountryLinks'
lookup_field='canonical_slot'
source='synthetic'
[[families.references]]
from='CountryLinks.canonical_slot'
to='LexicalTokens'
source='synthetic'
"#;
        #[derive(Default)]
        struct HistoricalCollected {
            event: Option<Event>,
            dated: Option<i32>,
            rows: BTreeMap<(u16, u128), Vec<(u16, bool, String)>>,
        }
        impl Consumer for HistoricalCollected {
            fn event(&mut self, event: Event) {
                self.event = Some(event);
                self.dated = None;
                if event.kind == 9 {
                    assert_eq!(event.hour, 11);
                    assert!(
                        self.rows
                            .insert((event.table, event.key), Vec::new())
                            .is_none()
                    );
                }
            }
            fn dated_hour(&mut self, hour: i32) {
                self.dated = Some(hour);
            }
            fn status(
                &mut self,
                status: u8,
                missing: Option<u32>,
                reason: Option<&[u8]>,
                sample: Option<i32>,
            ) {
                assert_eq!((status, missing, reason, sample), (1, None, None, None));
                if self.event.unwrap().kind == 10 {
                    assert_eq!(self.dated, Some(10));
                }
            }
            fn visible_key(&mut self, _key: u128) {
                panic!("Complete synthetic tables have no partial view");
            }
            fn flags(&mut self, flags: u8) {
                assert_eq!(flags, replay_columnar::manifest::AFTER_GAP);
            }
            fn cell(&mut self, field: u16, recorded: bool, value: Value<'_>) {
                let event = self.event.unwrap();
                if event.kind == 9 {
                    assert_eq!(self.dated, Some(10));
                    self.rows.get_mut(&(event.table, event.key)).unwrap().push((
                        field,
                        recorded,
                        format!("{value:?}"),
                    ));
                }
            }
        }
        let resolved = toml::from_str::<crate::manifest::Manifest>(text)
            .unwrap()
            .resolve()
            .unwrap();
        let readset = resolved.readsets[0].as_ref().unwrap();
        let schema = replay::columnar_frames::schema().unwrap();
        let base = |hour| Manifest {
            branch: 1,
            hour,
            flags: 0,
            total_ns: 0,
            pool_bytes: 0,
            dropped_frames: 0,
            appends: vec![],
            statuses: vec![Status::Complete; TableId::ALL.len()],
            read: vec![0; TableId::ALL.len().div_ceil(64)],
            samples: vec![],
            sections: vec![],
        };
        let entry = |id: TableId, bytes: &[u8]| Section {
            table: id.index(),
            bytes: bytes.len() as u64,
            columns: vec![(1_u64 << schema[id.index()].len()) - 1],
            ops: 0,
        };
        let insert = |id: TableId,
                      keys: &[u128],
                      overrides: &[(&str, Values<'_>)],
                      unknown: &[(usize, &[u32])]| {
            let slots: Vec<_> = (0..keys.len() as u32).collect();
            let u8s = vec![0; keys.len()];
            let i8s = vec![0; keys.len()];
            let u16s = vec![0; keys.len()];
            let i16s = vec![0; keys.len()];
            let u32s = vec![0; keys.len()];
            let i32s = vec![0; keys.len()];
            let u64s = vec![0; keys.len()];
            let i64s = vec![0; keys.len()];
            let present = vec![0; keys.len().div_ceil(64)];
            let mut values: Vec<_> = schema[id.index()]
                .iter()
                .map(|kind| match kind {
                    Kind::U8 => Values::U8(&u8s),
                    Kind::I8 => Values::I8(&i8s),
                    Kind::U16 => Values::U16(&u16s),
                    Kind::I16 => Values::I16(&i16s),
                    Kind::U32 => Values::U32(&u32s),
                    Kind::I32 => Values::I32(&i32s),
                    Kind::U64 => Values::U64(&u64s),
                    Kind::I64 => Values::I64(&i64s),
                    Kind::Bool => Values::Bool(&u8s),
                    Kind::Bytes => Values::Bytes(&u32s),
                    Kind::OptionalI32 => Values::OptionalI32 {
                        present: &present,
                        values: &[],
                    },
                    Kind::OptionalU32 => Values::OptionalU32 {
                        present: &present,
                        values: &[],
                    },
                    Kind::OptionalI64 => Values::OptionalI64 {
                        present: &present,
                        values: &[],
                    },
                    Kind::OptionalU64 => Values::OptionalU64 {
                        present: &present,
                        values: &[],
                    },
                    Kind::U32List => Values::U32List {
                        ends: &u32s,
                        items: &[],
                    },
                    Kind::U64List => Values::U64List {
                        ends: &u32s,
                        items: &[],
                    },
                    Kind::Opaque => Values::Opaque {
                        ends: &u32s,
                        items: &[],
                    },
                })
                .collect();
            for &(name, value) in overrides {
                values[columns::field(id, name).unwrap()] = value;
            }
            section::write(
                &schema[id.index()],
                &TableHour {
                    read: None,
                    changed: &[],
                    deleted: &[],
                    inserted: &slots,
                    keys,
                    inserts: &values,
                    runs: &[],
                    touched: &[],
                    unrecorded: unknown,
                },
            )
            .unwrap()
        };
        let changes = |id: TableId, deleted: &[u32], runs: &[Run<'_>]| {
            section::write(
                &schema[id.index()],
                &TableHour {
                    read: None,
                    changed: if runs.is_empty() { &[] } else { &[0] },
                    deleted,
                    inserted: &[],
                    keys: &[],
                    inserts: &[],
                    runs,
                    touched: &[],
                    unrecorded: &[],
                },
            )
            .unwrap()
        };
        let leader = (23_u128 << 32) | 4;
        let role1 = (7_u128 << 32) | 1;
        let role2 = (7_u128 << 32) | 2;
        for reverse in [false, true] {
            let cache =
                crate::keyset::Cache::reserved("synthetic historical spool", 32 * 1024).unwrap();
            let path = crate::synthetic_tests::folder().join("historical.spool");
            let mut spool =
                Spool::filtered(&path, readset, FormatVersion::CURRENT, &cache).unwrap();
            let mut state = State::new(schema.clone()).unwrap();
            let lexical_unknown = columns::field(TableId::LexicalTokens, "literal").unwrap();
            let initial = [
                (
                    TableId::UnitLeaders,
                    insert(
                        TableId::UnitLeaders,
                        &[leader],
                        &[
                            ("character_id", Values::U32(&[7])),
                            ("category", Values::I32(&[3])),
                        ],
                        &[],
                    ),
                ),
                (
                    TableId::Characters,
                    insert(
                        TableId::Characters,
                        &[7],
                        &[
                            ("name_token_id", Values::I32(&[41])),
                            ("template_tag", Values::U32(&[50])),
                        ],
                        &[],
                    ),
                ),
                (
                    TableId::AdvisorRoles,
                    insert(
                        TableId::AdvisorRoles,
                        &[role1, role2, 8_u128 << 32],
                        &[],
                        &[],
                    ),
                ),
                (
                    TableId::CountryLinks,
                    insert(
                        TableId::CountryLinks,
                        &[88],
                        &[("canonical_slot", Values::U32(&[50]))],
                        &[],
                    ),
                ),
                (
                    TableId::LexicalTokens,
                    insert(
                        TableId::LexicalTokens,
                        &[41, 50, 99],
                        &[("literal", Values::Bytes(&[1, 2, 3]))],
                        &[(lexical_unknown, &[1])],
                    ),
                ),
            ];
            let mut restored = base(9);
            restored.appends = vec![
                b"",
                b"held name",
                b"unknown",
                b"unreachable",
                b"current name",
            ];
            restored.sections = initial
                .iter()
                .map(|(id, bytes)| entry(*id, bytes))
                .collect();
            state.apply(&restored, |at| Ok(&initial[at].1)).unwrap();
            spool.restored(&state, 9).unwrap();
            let mut held = base(10);
            held.flags = replay_columnar::manifest::AFTER_GAP;
            state.apply(&held, |_| unreachable!()).unwrap();
            spool
                .hour(
                    &state,
                    &held,
                    false,
                    &[false; replay_data::rows::TABLE_COUNT],
                )
                .unwrap();
            let identity = if reverse {
                PowerIdentity {
                    character: Some(7),
                    ..Default::default()
                }
            } else {
                PowerIdentity {
                    character: None,
                    object_kind: PowerObjectKind::Leader,
                    object_reference: Some(leader as u64),
                    ..Default::default()
                }
            };
            let mut identity_bytes = Vec::new();
            identity.encode(&mut identity_bytes).unwrap();
            let update = |column, values| Run {
                column,
                op: Op::Set,
                positions: &[0],
                values: Some(values),
                except: &[],
                patches: None,
            };
            let delayed = [
                (
                    TableId::CountryPowerChanges,
                    insert(
                        TableId::CountryPowerChanges,
                        &[1],
                        &[
                            ("hour", Values::I32(&[10])),
                            (
                                "identity",
                                Values::Opaque {
                                    ends: &[identity_bytes.len() as u32],
                                    items: &identity_bytes,
                                },
                            ),
                        ],
                        &[],
                    ),
                ),
                (
                    TableId::UnitLeaders,
                    changes(TableId::UnitLeaders, &[0], &[]),
                ),
                (
                    TableId::AdvisorRoles,
                    changes(TableId::AdvisorRoles, &[0, 1, 2], &[]),
                ),
                (
                    TableId::Characters,
                    changes(
                        TableId::Characters,
                        &[],
                        &[
                            update(
                                columns::field(TableId::Characters, "name_token_id").unwrap(),
                                Values::I32(&[42]),
                            ),
                            update(
                                columns::field(TableId::Characters, "template_tag").unwrap(),
                                Values::U32(&[51]),
                            ),
                        ],
                    ),
                ),
                (
                    TableId::CountryLinks,
                    changes(TableId::CountryLinks, &[], &[update(0, Values::U32(&[51]))]),
                ),
                (
                    TableId::LexicalTokens,
                    changes(
                        TableId::LexicalTokens,
                        &[],
                        &[update(0, Values::Bytes(&[4]))],
                    ),
                ),
            ];
            let mut payment = base(11);
            payment.sections = delayed
                .iter()
                .map(|(id, bytes)| entry(*id, bytes))
                .collect();
            state.apply(&payment, |at| Ok(&delayed[at].1)).unwrap();
            spool
                .hour(
                    &state,
                    &payment,
                    false,
                    &[false; replay_data::rows::TABLE_COUNT],
                )
                .unwrap();
            let quiet = base(12);
            state.apply(&quiet, |_| unreachable!()).unwrap();
            spool
                .hour(
                    &state,
                    &quiet,
                    false,
                    &[false; replay_data::rows::TABLE_COUNT],
                )
                .unwrap();
            let encoded = spool.finish().unwrap();
            let mut collected = HistoricalCollected::default();
            replay(File::open(&encoded.raw_file).unwrap(), &mut collected).unwrap();
            let expected = [
                (TableId::UnitLeaders as u16, leader),
                (TableId::Characters as u16, 7),
                (TableId::AdvisorRoles as u16, role1),
                (TableId::AdvisorRoles as u16, role2),
                (TableId::CountryLinks as u16, 88),
                (TableId::LexicalTokens as u16, 41),
                (TableId::LexicalTokens as u16, 50),
            ]
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>();
            assert_eq!(
                collected
                    .rows
                    .keys()
                    .copied()
                    .collect::<std::collections::BTreeSet<_>>(),
                expected
            );
            assert!(
                collected.rows[&(TableId::Characters as u16, 7)]
                    .iter()
                    .any(|(_, recorded, value)| *recorded && value == "I32(41)")
            );
            assert!(
                collected.rows[&(TableId::CountryLinks as u16, 88)]
                    .iter()
                    .any(|(_, recorded, value)| *recorded && value == "U32(50)")
            );
            assert!(!collected.rows[&(TableId::LexicalTokens as u16, 50)][0].1);
            assert!(
                collected.rows[&(TableId::LexicalTokens as u16, 41)][0]
                    .2
                    .contains("104, 101, 108, 100")
            );
        }
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
