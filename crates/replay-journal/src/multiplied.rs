
use replay_recording::schema::schema;
use crate::columnar_journal::{Journal, Record, Sections, Source};
use replay_columnar::{
    leaf::Reader,
    manifest::{INSERTS, FILLS, Manifest, SAMPLES, Section, Status},
    section::{self, Op, PatchItems, Patches, Run, TableHour},
    state::{Bits, Cell, Kind, Table, Values},
};
use replay_data::rows::TableId;
use std::{
    fs::File,
    io::{self, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::Arc,
};

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

pub const LAG: u64 = 168;

fn label() -> engine_allocation::labels::Scope {
    use engine_allocation::labels;
    labels::enter(labels::owner("multiplier"), labels::Kind::Operator)
}

#[derive(Clone, Debug)]
enum Col {
    U8(Vec<u8>),
    I8(Vec<i8>),
    U16(Vec<u16>),
    I16(Vec<i16>),
    U32(Vec<u32>),
    I32(Vec<i32>),
    U64(Vec<u64>),
    I64(Vec<i64>),
    Bool(Vec<u8>),
    Bytes(Vec<u32>),
    OI32(Vec<Option<i32>>),
    OU32(Vec<Option<u32>>),
    OI64(Vec<Option<i64>>),
    OU64(Vec<Option<u64>>),
    L32(Vec<u32>, Vec<u32>),
    L64(Vec<u32>, Vec<u64>),
    Opaque(Vec<u32>, Vec<u8>),
}

enum Wire<'a> {
    Borrowed(Values<'a>),
    OI32(Vec<u64>, Vec<i32>),
    OU32(Vec<u64>, Vec<u32>),
    OI64(Vec<u64>, Vec<i64>),
    OU64(Vec<u64>, Vec<u64>),
}

impl Wire<'_> {
    fn values(&self) -> Values<'_> {
        match self {
            Self::Borrowed(values) => *values,
            Self::OI32(present, values) => Values::OptionalI32 { present, values },
            Self::OU32(present, values) => Values::OptionalU32 { present, values },
            Self::OI64(present, values) => Values::OptionalI64 { present, values },
            Self::OU64(present, values) => Values::OptionalU64 { present, values },
        }
    }
}

fn split<T: Copy>(cells: &[Option<T>]) -> (Vec<u64>, Vec<T>) {
    let mut present = vec![0_u64; cells.len().div_ceil(64)];
    let mut values = Vec::new();
    for (at, cell) in cells.iter().enumerate() {
        if let Some(value) = cell {
            present[at / 64] |= 1 << (at % 64);
            values.push(*value);
        }
    }
    (present, values)
}

fn joined<T: Copy>(present: &[u64], values: &[T], count: usize) -> Vec<Option<T>> {
    let mut next = values.iter();
    (0..count).map(|at| (present[at / 64] >> (at % 64) & 1 == 1).then(|| *next.next().expect("a value per bit"))).collect()
}

macro_rules! gather_plain {
    ($parts:expr, $picks:expr, $variant:ident) => {
        Col::$variant($picks.iter().map(|&(part, at)| match $parts[part] { Col::$variant(values) => values[at], _ => unreachable!("one kind") }).collect())
    };
}

macro_rules! gather_list {
    ($parts:expr, $picks:expr, $variant:ident) => {{
        let (mut ends, mut items) = (Vec::with_capacity($picks.len()), Vec::new());
        for &(part, at) in $picks {
            let Col::$variant(their_ends, their_items) = $parts[part] else { unreachable!("one kind") };
            let start = if at == 0 { 0 } else { their_ends[at - 1] as usize };
            items.extend_from_slice(&their_items[start..their_ends[at] as usize]);
            ends.push(items.len() as u32);
        }
        Col::$variant(ends, items)
    }};
}

impl Col {
    fn read(reader: &mut Reader<'_>, kind: Kind, count: usize) -> io::Result<Self> {
        macro_rules! ints {
            ($variant:ident) => {{
                let mut values = Vec::new();
                reader.ints(count, &mut values)?;
                Col::$variant(values)
            }};
        }
        macro_rules! optional {
            ($variant:ident) => {{
                let mut present: Vec<u64> = Vec::new();
                reader.values(count.div_ceil(64), &mut present)?;
                let set: usize = present.iter().map(|word| word.count_ones() as usize).sum();
                let mut values = Vec::new();
                reader.ints(set, &mut values)?;
                Col::$variant(joined(&present, &values, count))
            }};
        }
        Ok(match kind {
            Kind::U8 | Kind::Bool => {
                let mut values = Vec::new();
                reader.values(count, &mut values)?;
                if kind == Kind::U8 { Col::U8(values) } else { Col::Bool(values) }
            }
            Kind::I8 => {
                let mut values = Vec::new();
                reader.values(count, &mut values)?;
                Col::I8(values)
            }
            Kind::U16 => ints!(U16),
            Kind::I16 => ints!(I16),
            Kind::U32 => ints!(U32),
            Kind::Bytes => ints!(Bytes),
            Kind::I32 => ints!(I32),
            Kind::U64 => ints!(U64),
            Kind::I64 => ints!(I64),
            Kind::OptionalI32 => optional!(OI32),
            Kind::OptionalU32 => optional!(OU32),
            Kind::OptionalI64 => optional!(OI64),
            Kind::OptionalU64 => optional!(OU64),
            Kind::U32List | Kind::U64List | Kind::Opaque => {
                let mut ends = Vec::new();
                reader.lengths(count, &mut ends)?;
                let items = ends.last().map_or(0, |&end| end as usize);
                match kind {
                    Kind::U32List => {
                        let mut values = Vec::new();
                        reader.ints(items, &mut values)?;
                        Col::L32(ends, values)
                    }
                    Kind::U64List => {
                        let mut values = Vec::new();
                        reader.ints(items, &mut values)?;
                        Col::L64(ends, values)
                    }
                    _ => {
                        let mut values = Vec::new();
                        reader.values(items, &mut values)?;
                        Col::Opaque(ends, values)
                    }
                }
            }
        })
    }

    fn of_table(table: &Table, kind: Kind, column: usize, slots: &[u32]) -> Self {
        let held = table.column(column);
        let cell = |slot: u32| held.and_then(|column| column.cell(slot));
        macro_rules! plain {
            ($variant:ident, $cell:ident) => {
                Col::$variant(slots.iter().map(|&slot| match cell(slot) { Some(Cell::$cell(value)) => value, _ => Default::default() }).collect())
            };
        }
        macro_rules! optional {
            ($variant:ident, $cell:ident) => {
                Col::$variant(slots.iter().map(|&slot| match cell(slot) { Some(Cell::$cell(value)) => value, _ => None }).collect())
            };
        }
        macro_rules! list {
            ($variant:ident, $cell:ident) => {{
                let (mut ends, mut items) = (Vec::with_capacity(slots.len()), Vec::new());
                for &slot in slots {
                    if let Some(Cell::$cell(values)) = cell(slot) {
                        items.extend_from_slice(values);
                    }
                    ends.push(items.len() as u32);
                }
                Col::$variant(ends, items)
            }};
        }
        match kind {
            Kind::U8 => plain!(U8, U8),
            Kind::I8 => plain!(I8, I8),
            Kind::U16 => plain!(U16, U16),
            Kind::I16 => plain!(I16, I16),
            Kind::U32 => plain!(U32, U32),
            Kind::I32 => plain!(I32, I32),
            Kind::U64 => plain!(U64, U64),
            Kind::I64 => plain!(I64, I64),
            Kind::Bool => Col::Bool(slots.iter().map(|&slot| u8::from(matches!(cell(slot), Some(Cell::Bool(true))))).collect()),
            Kind::Bytes => plain!(Bytes, Bytes),
            Kind::OptionalI32 => optional!(OI32, OptionalI32),
            Kind::OptionalU32 => optional!(OU32, OptionalU32),
            Kind::OptionalI64 => optional!(OI64, OptionalI64),
            Kind::OptionalU64 => optional!(OU64, OptionalU64),
            Kind::U32List => list!(L32, U32List),
            Kind::U64List => list!(L64, U64List),
            Kind::Opaque => list!(Opaque, Opaque),
        }
    }

    fn gather(parts: &[&Col], picks: &[(usize, usize)]) -> Self {
        match parts[0] {
            Col::U8(_) => gather_plain!(parts, picks, U8),
            Col::I8(_) => gather_plain!(parts, picks, I8),
            Col::U16(_) => gather_plain!(parts, picks, U16),
            Col::I16(_) => gather_plain!(parts, picks, I16),
            Col::U32(_) => gather_plain!(parts, picks, U32),
            Col::I32(_) => gather_plain!(parts, picks, I32),
            Col::U64(_) => gather_plain!(parts, picks, U64),
            Col::I64(_) => gather_plain!(parts, picks, I64),
            Col::Bool(_) => gather_plain!(parts, picks, Bool),
            Col::Bytes(_) => gather_plain!(parts, picks, Bytes),
            Col::OI32(_) => gather_plain!(parts, picks, OI32),
            Col::OU32(_) => gather_plain!(parts, picks, OU32),
            Col::OI64(_) => gather_plain!(parts, picks, OI64),
            Col::OU64(_) => gather_plain!(parts, picks, OU64),
            Col::L32(..) => gather_list!(parts, picks, L32),
            Col::L64(..) => gather_list!(parts, picks, L64),
            Col::Opaque(..) => gather_list!(parts, picks, Opaque),
        }
    }

    fn wire(&self) -> Wire<'_> {
        Wire::Borrowed(match self {
            Col::U8(values) => Values::U8(values),
            Col::I8(values) => Values::I8(values),
            Col::U16(values) => Values::U16(values),
            Col::I16(values) => Values::I16(values),
            Col::U32(values) => Values::U32(values),
            Col::I32(values) => Values::I32(values),
            Col::U64(values) => Values::U64(values),
            Col::I64(values) => Values::I64(values),
            Col::Bool(values) => Values::Bool(values),
            Col::Bytes(values) => Values::Bytes(values),
            Col::OI32(cells) => {
                let (present, values) = split(cells);
                return Wire::OI32(present, values);
            }
            Col::OU32(cells) => {
                let (present, values) = split(cells);
                return Wire::OU32(present, values);
            }
            Col::OI64(cells) => {
                let (present, values) = split(cells);
                return Wire::OI64(present, values);
            }
            Col::OU64(cells) => {
                let (present, values) = split(cells);
                return Wire::OU64(present, values);
            }
            Col::L32(ends, items) => Values::U32List { ends, items },
            Col::L64(ends, items) => Values::U64List { ends, items },
            Col::Opaque(ends, items) => Values::Opaque { ends, items },
        })
    }
}

fn base(kind: Kind) -> io::Result<Kind> {
    Ok(match kind {
        Kind::OptionalI32 => Kind::I32,
        Kind::OptionalU32 => Kind::U32,
        Kind::OptionalI64 => Kind::I64,
        Kind::OptionalU64 => Kind::U64,
        Kind::U8 | Kind::I8 | Kind::U16 | Kind::I16 | Kind::U32 | Kind::I32 | Kind::U64 | Kind::I64 => kind,
        other => return Err(invalid(format!("ADD to a {other:?} column"))),
    })
}

#[derive(Clone, Debug)]
enum Items {
    U32(Vec<u32>),
    U64(Vec<u64>),
    Bytes(Vec<u8>),
}

#[derive(Clone, Debug)]
struct OwnedRun {
    column: usize,
    op: Op,
    positions: Vec<u32>,
    values: Option<Col>,
    except: Vec<u32>,
    patches: Option<(Vec<u32>, Vec<(u32, u32, u32)>, Items)>,
}

#[derive(Clone, Debug, Default)]
struct Decoded {
    read: Option<Vec<u32>>,
    changed: Vec<u32>,
    deleted: Vec<u32>,
    inserted: Vec<u32>,
    keys: Vec<u128>,
    inserts: Vec<Col>,
    runs: Vec<OwnedRun>,
    unrecorded: Vec<(usize, Vec<u32>)>,
    touched: Vec<u32>,
}

impl Decoded {
    fn read(bytes: &[u8], kinds: &[Kind], truncated: bool) -> io::Result<Self> {
        let mut reader = Reader::new(bytes);
        let mut decoded = Decoded::default();
        if truncated {
            let mut read = Vec::new();
            reader.slots(&mut read)?;
            decoded.read = Some(read);
        }
        reader.slots(&mut decoded.changed)?;
        reader.slots(&mut decoded.deleted)?;
        reader.slots(&mut decoded.inserted)?;
        reader.keys(decoded.inserted.len(), &mut decoded.keys)?;
        if !decoded.inserted.is_empty() {
            for &kind in kinds {
                decoded.inserts.push(Col::read(&mut reader, kind, decoded.inserted.len())?);
            }
        }
        for _ in 0..reader.count()? {
            let column = reader.count()?;
            let kind = *kinds.get(column).ok_or_else(|| invalid("Run names a column past the schema"))?;
            let tag = reader.u8()?;
            let mut run = OwnedRun { column, op: Op::Set, positions: Vec::new(), values: None, except: Vec::new(), patches: None };
            match tag {
                3 => {
                    run.op = Op::Fill;
                    reader.slots(&mut run.except)?;
                    run.values = Some(Col::read(&mut reader, kind, 1)?);
                }
                2 => {
                    run.op = Op::Noop;
                    reader.positions(decoded.changed.len(), &mut run.positions)?;
                }
                4 => {
                    run.op = Op::Patch;
                    reader.positions(decoded.changed.len(), &mut run.positions)?;
                    let mut hunks = Vec::new();
                    reader.ints(run.positions.len(), &mut hunks)?;
                    let total: usize = hunks.iter().map(|&count: &u32| count as usize).sum();
                    let mut flat: Vec<u32> = Vec::new();
                    reader.ints(total * 3, &mut flat)?;
                    let edits: Vec<(u32, u32, u32)> = flat.chunks_exact(3).map(|edit| (edit[0], edit[1], edit[2])).collect();
                    let inserted: usize = edits.iter().map(|edit| edit.2 as usize).sum();
                    let items = match kind {
                        Kind::U32List => {
                            let mut items = Vec::new();
                            reader.ints(inserted, &mut items)?;
                            Items::U32(items)
                        }
                        Kind::U64List => {
                            let mut items = Vec::new();
                            reader.ints(inserted, &mut items)?;
                            Items::U64(items)
                        }
                        Kind::Opaque => {
                            let mut items = Vec::new();
                            reader.values(inserted, &mut items)?;
                            Items::Bytes(items)
                        }
                        other => return Err(invalid(format!("PATCH to a {other:?} column"))),
                    };
                    run.patches = Some((hunks, edits, items));
                }
                0 | 1 => {
                    run.op = if tag == 0 { Op::Set } else { Op::Add };
                    reader.positions(decoded.changed.len(), &mut run.positions)?;
                    let kind = if tag == 1 { base(kind)? } else { kind };
                    run.values = Some(Col::read(&mut reader, kind, run.positions.len())?);
                }
                other => return Err(invalid(format!("Column run op {other}"))),
            }
            decoded.runs.push(run);
        }
        for _ in 0..reader.count()? {
            let column = reader.count()?;
            let mut slots = Vec::new();
            reader.slots(&mut slots)?;
            decoded.unrecorded.push((column, slots));
        }
        reader.slots(&mut decoded.touched)?;
        reader.finish()?;
        Ok(decoded)
    }

    fn write(&self, kinds: &[Kind]) -> io::Result<Vec<u8>> {
        let inserts: Vec<Wire<'_>> = self.inserts.iter().map(Col::wire).collect();
        let insert_values: Vec<Values<'_>> = inserts.iter().map(Wire::values).collect();
        let values: Vec<Option<Wire<'_>>> = self.runs.iter().map(|run| run.values.as_ref().map(Col::wire)).collect();
        let runs: Vec<Run<'_>> = self
            .runs
            .iter()
            .zip(&values)
            .map(|(run, values)| Run {
                column: run.column,
                op: run.op,
                positions: &run.positions,
                values: values.as_ref().map(Wire::values),
                except: &run.except,
                patches: run.patches.as_ref().map(|(hunks, edits, items)| Patches {
                    hunks,
                    edits,
                    items: match items {
                        Items::U32(items) => PatchItems::U32(items),
                        Items::U64(items) => PatchItems::U64(items),
                        Items::Bytes(items) => PatchItems::Bytes(items),
                    },
                }),
            })
            .collect();
        let unrecorded: Vec<(usize, &[u32])> = self.unrecorded.iter().map(|(column, slots)| (*column, slots.as_slice())).collect();
        section::write(
            kinds,
            &TableHour {
                read: self.read.as_deref(),
                changed: &self.changed,
                deleted: &self.deleted,
                inserted: &self.inserted,
                keys: &self.keys,
                inserts: &insert_values,
                runs: &runs,
                unrecorded: &unrecorded,
                touched: &self.touched,
            },
        )
    }

    fn whole(table: &Table, kinds: &[Kind]) -> Self {
        let inserted: Vec<u32> = table.live().ones().collect();
        let keys = inserted.iter().map(|&slot| table.key(slot).expect("a live row")).collect();
        let inserts = if inserted.is_empty() {
            Vec::new()
        } else {
            kinds.iter().enumerate().map(|(column, &kind)| Col::of_table(table, kind, column, &inserted)).collect()
        };
        let unrecorded = (0..kinds.len())
            .map(|column| (column, inserted.iter().copied().filter(|&slot| !table.recorded(column, slot)).collect::<Vec<u32>>()))
            .filter(|(_, slots)| !slots.is_empty())
            .collect();
        Self { inserted, keys, inserts, unrecorded, ..Self::default() }
    }
}

pub struct Factors {
    pub factor: Vec<u32>,
    pub in_phase: Vec<bool>,
}

impl Factors {
    pub fn read(path: &Path) -> io::Result<Self> {
        let mut factor = vec![1_u32; TableId::ALL.len()];
        for line in std::fs::read_to_string(path)?.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (name, given) = line.split_once(char::is_whitespace).ok_or_else(|| invalid(format!("Not `Table factor`: {line}")))?;
            let table = TableId::ALL
                .into_iter()
                .find(|table| format!("{table:?}") == name)
                .ok_or_else(|| invalid(format!("No table {name}")))?;
            factor[table.index()] = given.trim().parse().ok().filter(|&factor| factor >= 1).ok_or_else(|| invalid(format!("Not a factor: {line}")))?;
        }
        Ok(Self { in_phase: vec![false; factor.len()], factor })
    }

    pub fn largest(&self) -> u32 {
        self.factor.iter().copied().max().unwrap_or(1)
    }
}

pub struct Start {
    pub block: u32,
    pub file: PathBuf,
}

const START_MAGIC: &[u8; 8] = b"HOI4STA1";

struct Synthetic {
    witness: Option<crate::columnar_journal::Witness>,
    manifest: (u64, u64),
    file: PathBuf,
    sections: Vec<Option<(u64, u64)>>,
}

impl Synthetic {
    fn open(file: &Path, capture: u32, block: u32, tables: usize) -> io::Result<Self> {
        let mut held = File::open(file)?;
        let size = held.metadata()?.len();
        let read = |held: &mut File, at: u64, most: u64| -> io::Result<Vec<u8>> {
            held.seek(SeekFrom::Start(at))?;
            let mut bytes = Vec::new();
            Read::by_ref(held).take(most).read_to_end(&mut bytes)?;
            Ok(bytes)
        };
        let bytes = read(&mut held, 0, 64)?;
        let mut reader = Reader::new(&bytes);
        if reader.raw(START_MAGIC.len())? != START_MAGIC {
            return Err(invalid(format!("{} is not a multiplied run's start", file.display())));
        }
        if reader.varint_u32()? != capture || reader.varint_u32()? != block {
            return Err(invalid(format!("{} starts another capture or block", file.display())));
        }
        let witness = match reader.u8()? {
            0 => None,
            flags => {
                let qpc = u64::from_le_bytes(reader.raw(8)?.try_into().expect("eight bytes"));
                let signal = i64::from_le_bytes(reader.raw(8)?.try_into().expect("eight bytes"));
                Some(crate::columnar_journal::Witness { qpc, load_signal: (flags & 2 != 0).then_some(signal) })
            }
        };
        let length = reader.varint()?;
        let manifest = (reader.at() as u64, length);
        let after = manifest.0 + manifest.1;
        let bytes = read(&mut held, after, 10 * (tables as u64 + 1))?;
        let mut reader = Reader::new(&bytes);
        if reader.count()? != tables {
            return Err(invalid(format!("{} holds another schema's tables", file.display())));
        }
        let mut lengths = Vec::with_capacity(tables);
        for _ in 0..tables {
            lengths.push(reader.varint()?);
        }
        let mut at = after + reader.at() as u64;
        let sections = lengths
            .into_iter()
            .map(|length| {
                let held = (length > 0).then(|| (at, length - 1));
                at += length.saturating_sub(1);
                held
            })
            .collect();
        if at != size {
            return Err(invalid(format!("{} ends elsewhere than its sections", file.display())));
        }
        Ok(Self { witness, manifest, file: file.to_owned(), sections })
    }

    fn manifest(&self) -> io::Result<Vec<u8>> {
        let _label = label();
        let mut file = File::open(&self.file)?;
        file.seek(SeekFrom::Start(self.manifest.0))?;
        let mut bytes = vec![0; self.manifest.1 as usize];
        file.read_exact(&mut bytes)?;
        Ok(bytes)
    }

    fn section(&self, table: usize) -> io::Result<Option<Vec<u8>>> {
        let _label = label();
        let Some((at, length)) = self.sections.get(table).copied().flatten() else { return Ok(None) };
        let mut file = File::open(&self.file)?;
        file.seek(SeekFrom::Start(at))?;
        let mut bytes = vec![0; length as usize];
        file.read_exact(&mut bytes)?;
        Ok(Some(bytes))
    }

    fn decoded(&self, table: usize, kinds: &[Kind], truncated: bool) -> io::Result<Option<Decoded>> {
        let _label = label();
        self.section(table)?.map(|bytes| Decoded::read(&bytes, kinds, truncated)).transpose()
    }
}

pub fn preroll(directory: &Path, capture: u32, block: u32, out: &Path) -> io::Result<()> {
    let kinds = schema()?;
    let derived = derived_mask();
    let mut scan = scan_journal(directory, capture, &kinds, &derived, &REMOVED, true, true)?;
    let at = start_segment(&scan.scanned, block)?;
    let appends: Vec<Vec<u8>> = scan.scanned[..at].iter_mut().flat_map(|held| std::mem::take(&mut held.appends)).collect();
    let (witness, manifest, tables) = restored(directory, capture, scan.segments[at].0, &kinds, &derived, &scan.journal, appends)?;
    let mut sections = Vec::new();
    let mut lengths = Vec::with_capacity(tables.len());
    for (table, decoded) in tables.iter().enumerate() {
        match decoded {
            None => lengths.push(0),
            Some(decoded) => {
                let bytes = decoded.write(&kinds[table])?;
                lengths.push(bytes.len() as u64 + 1);
                sections.extend_from_slice(&bytes);
            }
        }
    }
    let mut head = START_MAGIC.to_vec();
    replay_columnar::unit::put_varint(&mut head, u64::from(capture));
    replay_columnar::unit::put_varint(&mut head, u64::from(block));
    match witness {
        None => head.push(0),
        Some(witness) => {
            head.push(1 | if witness.load_signal.is_some() { 2 } else { 0 });
            head.extend_from_slice(&witness.qpc.to_le_bytes());
            head.extend_from_slice(&witness.load_signal.unwrap_or(0).to_le_bytes());
        }
    }
    replay_columnar::unit::put_varint(&mut head, manifest.len() as u64);
    head.extend_from_slice(&manifest);
    replay_columnar::unit::put_varint(&mut head, lengths.len() as u64);
    for length in lengths {
        replay_columnar::unit::put_varint(&mut head, length);
    }
    let written = out.with_extension("writing");
    let mut file = File::create(&written)?;
    std::io::Write::write_all(&mut file, &head)?;
    std::io::Write::write_all(&mut file, &sections)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(written, out)
}

fn scan(path: &Path, mut visit: impl FnMut(u8, u64, &[u8]) -> io::Result<()>) -> io::Result<()> {
    let mut file = File::open(path)?;
    let size = file.metadata()?.len();
    let mut at = crate::columnar_journal::MAGIC.len() as u64;
    let mut head = [0_u8; 5];
    let mut bytes = Vec::new();
    while at + 9 <= size {
        file.seek(SeekFrom::Start(at))?;
        file.read_exact(&mut head)?;
        let length = u32::from_le_bytes(head[1..5].try_into().expect("four bytes")) as u64;
        if at + 9 + length > size {
            break;
        }
        let mut want = length;
        if head[0] == 3 {
            want = length.min(1 << 16);
        }
        loop {
            bytes.clear();
            Read::by_ref(&mut file).take(want).read_to_end(&mut bytes)?;
            if head[0] != 3 || want == length {
                break;
            }
            let mut reader = Reader::new(&bytes);
            let flags = reader.u8()?;
            reader.raw(if flags & 1 == 0 { 0 } else if flags & 2 == 0 { 8 } else { 16 })?;
            let manifest = reader.count()? as u64;
            let needed = reader.at() as u64 + manifest;
            if needed <= want {
                break;
            }
            want = needed.min(length);
            file.seek(SeekFrom::Start(at + 5))?;
        }
        visit(head[0], length, &bytes)?;
        at += 9 + length;
    }
    Ok(())
}

fn scanned_manifest(total: u64, bytes: &[u8]) -> io::Result<(&[u8], u64)> {
    let mut reader = Reader::new(bytes);
    let flags = reader.u8()?;
    reader.raw(if flags & 1 == 0 { 0 } else if flags & 2 == 0 { 8 } else { 16 })?;
    let length = reader.count()?;
    let manifest = reader.raw(length)?;
    let sections = total.checked_sub(reader.at() as u64).ok_or_else(|| invalid("An hour's manifest passes its record"))?;
    Ok((manifest, sections))
}

fn manifest_matches(bytes: &[u8], sections_bytes: u64, columns: &[usize]) -> bool {
    let Ok(manifest) = Manifest::read(bytes, columns) else {
        return false;
    };
    manifest.sections.iter().all(|entry| entry.bytes > 0)
        && manifest.sections.iter().try_fold(0_u64, |sum, entry| sum.checked_add(entry.bytes)) == Some(sections_bytes)
}

fn manifests_agree(candidates: &[(JournalTables, Vec<usize>)], bytes: &[u8], tables: usize, derived: &[bool]) -> bool {
    let first = candidates[0].0.read_with(bytes, tables, &candidates[0].1).map(|manifest| widen(manifest, derived));
    candidates[1..].iter().all(|(other, columns)| {
        let alt = other.read_with(bytes, tables, columns).map(|manifest| widen(manifest, derived));
        match (&first, &alt) {
            (Ok(a), Ok(b)) => a == b,
            (Err(a), Err(b)) => a.to_string() == b.to_string(),
            _ => false,
        }
    })
}

pub type Removed = (usize, &'static str, &'static [Kind]);

pub const REMOVED: [Removed; 1] = [(422, "LogForgets", &[Kind::I32])];

#[derive(Clone, Debug)]
struct JournalTables {
    columns: Vec<usize>,
    places: Vec<Option<usize>>,
    removed: Vec<(usize, &'static str)>,
}

impl JournalTables {
    fn written(kinds: &[Vec<Kind>], removed: &[Removed]) -> Self {
        let mut columns: Vec<usize> = kinds.iter().map(Vec::len).collect();
        let mut places: Vec<Option<usize>> = (0..kinds.len()).map(Some).collect();
        for &(at, _, held) in removed {
            columns.insert(at, held.len());
            places.insert(at, None);
        }
        Self { columns, places, removed: removed.iter().map(|&(at, name, _)| (at, name)).collect() }
    }

    fn first(&self, tables: usize) -> Self {
        Self { columns: self.columns[..tables].to_vec(), places: self.places[..tables].to_vec(), removed: self.removed.iter().copied().filter(|&(at, _)| at < tables).collect() }
    }

    fn structural_columns(&self) -> Vec<usize> {
        self.columns.iter().map(|&count| count.next_multiple_of(64).max(64)).collect()
    }

    fn relax_derived(mut self, derived: &[bool]) -> Self {
        for (at, place) in self.places.iter().enumerate() {
            if place.is_some_and(|table| derived[table]) {
                self.columns[at] = self.columns[at].next_multiple_of(64).max(64);
            }
        }
        self
    }

    fn read<'a>(&self, bytes: &'a [u8], tables: usize) -> io::Result<Manifest<'a>> {
        self.read_with(bytes, tables, &self.columns)
    }

    fn read_with<'a>(&self, bytes: &'a [u8], tables: usize, columns: &[usize]) -> io::Result<Manifest<'a>> {
        let mut manifest = Manifest::read(bytes, columns)?;
        if self.removed.is_empty() {
            return Ok(manifest);
        }
        let place = |at: usize| self.places.get(at).copied().flatten();
        let mut statuses = vec![Status::NotRecorded; tables];
        let mut read = vec![0_u64; tables.div_ceil(64)];
        for (at, &status) in manifest.statuses.iter().enumerate() {
            let Some(table) = place(at) else { continue };
            statuses[table] = status;
            if manifest.read.get(at / 64).is_some_and(|word| word >> (at % 64) & 1 != 0) {
                read[table / 64] |= 1 << (table % 64);
            }
        }
        manifest.statuses = statuses;
        manifest.read = read;
        manifest.samples = manifest.samples.iter().filter_map(|&(at, hour)| place(at).map(|table| (table, hour))).collect();
        if manifest.samples.is_empty() {
            manifest.flags &= !SAMPLES;
        }
        for entry in &mut manifest.sections {
            entry.table = place(entry.table).ok_or_else(|| {
                let name = self.removed.iter().find(|&&(at, _)| at == entry.table).map_or("a removed table", |&(_, name)| name);
                invalid(format!("The journal holds a section of {name}, a table removed from this schema since it was written: it is not read"))
            })?;
        }
        Ok(manifest)
    }
}

fn widen<'a>(mut manifest: Manifest<'a>, derived: &[bool]) -> Manifest<'a> {
    let tables = derived.len();
    manifest.statuses.resize(tables, Status::NotRecorded);
    manifest.read.resize(tables.div_ceil(64), 0);
    for (table, &is_derived) in derived.iter().enumerate() {
        if is_derived {
            manifest.statuses[table] = Status::NotRecorded;
            manifest.read[table / 64] &= !(1 << (table % 64));
        }
    }
    manifest.samples.retain(|&(table, _)| !derived[table]);
    if manifest.samples.is_empty() {
        manifest.flags &= !SAMPLES;
    }
    manifest.sections.retain(|entry| !derived[entry.table]);
    manifest
}

fn journal_candidates(
    segments: &[(u32, PathBuf)],
    kinds: &[Vec<Kind>],
    derived: &[bool],
    removed: &[Removed],
) -> io::Result<(Vec<JournalTables>, bool)> {
    let mut identity = None;
    for (_, path) in segments {
        scan(path, |kind, _, bytes| {
            if kind == 1 && identity.is_none() {
                let mut reader = Reader::new(bytes);
                let length = reader.count()?;
                identity = Some(replay_columnar::envelope::Identity::read_capture(reader.raw(length)?)?);
            }
            Ok(())
        })?;
        if identity.is_some() {
            break;
        }
    }
    let identity = identity.ok_or_else(|| invalid("The journal holds no capture start"))?;
    let mut before = kinds.to_vec();
    for &(at, _, columns) in removed {
        before.insert(at, columns.to_vec());
    }
    for (written, removed) in [(kinds, &[][..]), (&before[..], removed)] {
        if let Some(tables) = (1..=written.len()).rev().find(|&tables| replay_columnar::envelope::schema_digest(&written[..tables]) == identity.schema) {
            return Ok((vec![JournalTables::written(kinds, removed).first(tables)], true));
        }
    }
    let mut candidates = Vec::new();
    for layout in [&[][..], removed] {
        if layout.is_empty() && !candidates.is_empty() {
            continue;
        }
        let written = JournalTables::written(kinds, layout);
        let first_derived = written
            .places
            .iter()
            .position(|place| place.is_some_and(|table| derived[table]))
            .unwrap_or(written.columns.len());
        for tables in first_derived.max(1)..=written.columns.len().min(kinds.len()) {
            let candidate = written.first(tables);
            if !layout.is_empty() && candidate.removed.is_empty() {
                continue;
            }
            candidates.push(candidate);
        }
    }
    Ok((candidates, false))
}

fn derived_mask() -> Vec<bool> {
    TableId::ALL.into_iter().map(TableId::is_derived).collect()
}

struct Scanned {
    hours: u64,
    appends: Vec<Vec<u8>>,
    controls: Vec<Vec<u8>>,
    begin: Option<(Vec<u8>, Vec<u8>)>,
    run: Option<(u64, u8, i32)>,
}

struct Scan {
    journal: JournalTables,
    exact: bool,
    scanned: Vec<Scanned>,
    segments: Vec<(u32, PathBuf)>,
    partial: Vec<bool>,
}

fn scan_journal(directory: &Path, capture: u32, kinds: &[Vec<Kind>], derived: &[bool], removed: &[Removed], appends: bool, one_run: bool) -> io::Result<Scan> {
    let segments: Vec<(u32, PathBuf)> = crate::columnar_journal::segments(directory)?
        .into_iter()
        .filter(|(held, _, _)| *held == capture)
        .map(|(_, segment, path)| (segment, path))
        .collect();
    let (candidates, exact) = journal_candidates(&segments, kinds, derived, removed)?;
    let mut candidates: Vec<(JournalTables, Vec<usize>)> = candidates
        .into_iter()
        .map(|candidate| {
            let structural = if exact { Vec::new() } else { candidate.structural_columns() };
            (candidate, structural)
        })
        .collect();
    let mut partial = vec![false; kinds.len()];
    let mut scanned: Vec<Scanned> = Vec::with_capacity(segments.len());
    for (_, path) in &segments {
        let mut held = Scanned { hours: 0, appends: Vec::new(), controls: Vec::new(), begin: None, run: None };
        scan(path, |kind, total, bytes| {
            match kind {
                3 => {
                    let (bytes, sections_bytes) = scanned_manifest(total, bytes)?;
                    if exact {
                        let manifest = widen(candidates[0].0.read(bytes, kinds.len())?, derived);
                        for (table, status) in manifest.statuses.iter().enumerate() {
                            if *status != Status::Complete {
                                partial[table] = true;
                            }
                        }
                        if appends {
                            held.appends.extend(manifest.appends.iter().map(|text| text.to_vec()));
                        }
                    } else {
                        candidates.retain(|(_, structural)| manifest_matches(bytes, sections_bytes, structural));
                        if candidates.is_empty() {
                            return Err(invalid("The journal's schema digest is unrecognized and no current table count reads every manifest of the journal"));
                        }
                    }
                    held.hours += 1;
                }
                1 => {
                    let mut reader = Reader::new(bytes);
                    let length = reader.count()?;
                    let identity = reader.raw(length)?.to_vec();
                    held.begin = Some((identity, reader.rest().to_vec()));
                }
                2 => {
                    let mut reader = Reader::new(bytes);
                    let branch = reader.varint()?;
                    let epoch = reader.u8()?;
                    let hour = reader.varint_u32()?;
                    held.run = Some((branch, epoch, ((hour >> 1) as i32) ^ -((hour & 1) as i32)));
                }
                4 => {
                    let mut reader = Reader::new(bytes);
                    for _ in 0..reader.count()? {
                        let length = reader.count()?;
                        held.controls.push(reader.raw(length)?.to_vec());
                    }
                }
                _ => {}
            }
            Ok(())
        })?;
        scanned.push(held);
    }
    let journal = if exact {
        candidates.swap_remove(0).0
    } else {
        if candidates.is_empty() {
            return Err(invalid("The journal's schema digest is unrecognized and no current table count reads every manifest of the journal"));
        }
        let journal = candidates
            .iter()
            .find(|(candidate, _)| candidate.removed.is_empty())
            .ok_or_else(|| invalid("The journal's schema digest is unrecognized and shifting tables across retired slots requires a recognized digest"))?
            .0
            .clone()
            .relax_derived(derived);
        for ((_, path), held) in segments.iter().zip(&mut scanned) {
            scan(path, |kind, total, bytes| {
                if kind != 3 {
                    return Ok(());
                }
                let (bytes, _) = scanned_manifest(total, bytes)?;
                if !manifests_agree(&candidates, bytes, kinds.len(), derived) {
                    return Err(invalid("The journal's schema digest is unrecognized and more than one table layout could describe its manifests"));
                }
                let manifest = widen(journal.read(bytes, kinds.len())?, derived);
                for (table, status) in manifest.statuses.iter().enumerate() {
                    if *status != Status::Complete {
                        partial[table] = true;
                    }
                }
                if appends {
                    held.appends.extend(manifest.appends.iter().map(|text| text.to_vec()));
                }
                Ok(())
            })?;
        }
        journal
    };
    if one_run && scanned.iter().filter(|held| held.run.is_some()).count() > 1 {
        return Err(invalid("A multiplied journal, or one given a start, holds one run"));
    }
    Ok(Scan { journal, exact, scanned, segments, partial })
}

fn start_segment(scanned: &[Scanned], block: u32) -> io::Result<usize> {
    let wanted = u64::from(block + 1) * 720;
    let mut before = 0;
    scanned
        .iter()
        .position(|held| {
            let found = before == wanted && held.hours > 0;
            before += held.hours;
            found
        })
        .ok_or_else(|| invalid("No journal segment starts the block after the restore"))
}

fn current_identity(identity: &[u8], kinds: &[Vec<Kind>]) -> io::Result<Vec<u8>> {
    let mut held = replay_columnar::envelope::Identity::read_capture(identity)?;
    held.schema = replay_columnar::envelope::schema_digest(kinds);
    held.chain_bytes()
}

enum Prefix {
    Begin(Vec<u8>, Vec<u8>),
    Controls(Vec<Vec<u8>>),
    Run { branch: u64, epoch: u8, first_hour: i32 },
}

struct Upstream {
    journal: Journal,
    prefix: Vec<Prefix>,
    at: usize,
    synthetic: Option<Arc<Synthetic>>,
    skip_hour: bool,
    runs: u32,
}

enum Item<'a> {
    Record(Record<'a>),
    Synthetic(Arc<Synthetic>),
}

impl Upstream {
    fn item(&mut self) -> io::Result<Option<Item<'_>>> {
        if let Some(prefix) = self.prefix.get(self.at) {
            return Ok(Some(Item::Record(match prefix {
                Prefix::Begin(identity, capture) => Record::Begin { identity, capture },
                Prefix::Controls(records) => Record::Controls(records.iter().map(Vec::as_slice).collect()),
                Prefix::Run { branch, epoch, first_hour } => Record::Run { branch: *branch, epoch: *epoch, first_hour: *first_hour },
            })));
        }
        if let Some(synthetic) = &self.synthetic {
            return Ok(Some(Item::Synthetic(synthetic.clone())));
        }
        if self.skip_hour {
            loop {
                match self.journal.record()? {
                    None => return Ok(None),
                    Some(Record::Hour { .. }) => {
                        self.journal.advance();
                        self.skip_hour = false;
                        break;
                    }
                    Some(_) => break,
                }
            }
        }
        Ok(self.journal.record()?.map(Item::Record))
    }

    fn advance(&mut self) {
        if self.at < self.prefix.len() {
            self.at += 1;
        } else if self.synthetic.is_some() {
            self.synthetic = None;
        } else {
            self.journal.advance();
        }
    }
}

#[derive(Clone)]
enum Piece {
    Decoded(Arc<Decoded>),
    Start(bool),
}

#[derive(Default)]
struct CopyHour {
    tables: Vec<Option<(Piece, Vec<u64>, u8)>>,
}

enum First {
    Raw(Vec<u8>),
    Decoded(Arc<Decoded>),
    Start(bool),
}

struct Built {
    witness: Option<crate::columnar_journal::Witness>,
    manifest: Vec<u8>,
    sections: Vec<u8>,
}

pub struct Multiplied {
    kinds: Vec<Vec<Kind>>,
    columns: Vec<usize>,
    journal: JournalTables,
    begin: Option<(Vec<u8>, Vec<u8>)>,
    start: Option<Arc<Synthetic>>,
    pub exact_schema: bool,
    derived: Vec<bool>,
    factors: Factors,
    first: Upstream,
    copies: Vec<Upstream>,
    hours: u64,
    live: Vec<Vec<Bits>>,
    built: Option<Built>,
    passing: Option<(Option<crate::columnar_journal::Witness>, Vec<u8>)>,
    runs: u32,
    one_run: bool,
    segment_hours: Vec<(u32, u64)>,
    pub spent: std::time::Duration,
}

fn set_bit(bits: &mut Vec<u64>, slot: u32) {
    let word = slot as usize / 64;
    if word >= bits.len() {
        bits.resize(word + 1, 0);
    }
    bits[word] |= 1 << (slot % 64);
}

fn clear_bit(bits: &mut [u64], slot: u32) {
    if let Some(word) = bits.get_mut(slot as usize / 64) {
        *word &= !(1 << (slot % 64));
    }
}

impl Multiplied {
    pub fn open(directory: &Path, capture: u32, factors: Factors, start: Option<Start>) -> io::Result<Self> {
        Self::open_with(directory, capture, factors, start, &REMOVED)
    }

    fn open_with(directory: &Path, capture: u32, mut factors: Factors, start: Option<Start>, removed: &[Removed]) -> io::Result<Self> {
        let _label = label();
        let kinds = schema()?;
        let columns: Vec<usize> = kinds.iter().map(Vec::len).collect();
        let derived = derived_mask();
        let one_run = start.is_some() || factors.factor.iter().any(|&factor| factor > 1);
        let Scan { journal, exact, mut scanned, segments, partial } = scan_journal(directory, capture, &kinds, &derived, removed, false, one_run)?;
        for (table, partial) in partial.into_iter().enumerate() {
            factors.in_phase[table] |= partial;
        }
        let mut first_segment = 0;
        let mut prefix = Vec::new();
        let synthetic = match &start {
            None => None,
            Some(start) => {
                let at = start_segment(&scanned, start.block)?;
                first_segment = segments[at].0;
                let (branch, epoch, first_hour) =
                    scanned[..at].iter().find_map(|held| held.run).ok_or_else(|| invalid("The journal holds no run before the start"))?;
                let (identity, capture_bytes) =
                    scanned[..at].iter().find_map(|held| held.begin.clone()).ok_or_else(|| invalid("The journal holds no capture before the start"))?;
                prefix.push(Prefix::Begin(current_identity(&identity, &kinds)?, capture_bytes));
                let controls: Vec<Vec<u8>> = scanned[..at].iter_mut().flat_map(|held| std::mem::take(&mut held.controls)).collect();
                if !controls.is_empty() {
                    prefix.push(Prefix::Controls(controls));
                }
                let wanted = u64::from(start.block + 1) * 720;
                prefix.push(Prefix::Run { branch, epoch, first_hour: first_hour + wanted as i32 });
                Some(Arc::new(Synthetic::open(&start.file, capture, start.block, kinds.len())?))
            }
        };
        let segment_hours = segments.iter().zip(&scanned).map(|((segment, _), held)| (*segment, held.hours)).collect();
        drop(scanned);
        let upstream = |prefix: Vec<Prefix>| -> io::Result<Upstream> {
            let mut journal = Journal::open_read_only(directory, capture);
            journal.start(first_segment)?;
            Ok(Upstream { journal, prefix, at: 0, synthetic: synthetic.clone(), skip_hour: synthetic.is_some(), runs: 0 })
        };
        let first = upstream(prefix)?;
        let out_of_phase = (0..factors.factor.len()).filter(|&table| !factors.in_phase[table]).map(|table| factors.factor[table]).max().unwrap_or(1);
        let copies = (1..out_of_phase).map(|_| upstream(Vec::new())).collect::<io::Result<_>>()?;
        let live = factors.factor.iter().map(|&factor| if factor > 1 { vec![Bits::default(); factor as usize] } else { Vec::new() }).collect();
        Ok(Self { kinds, columns, journal, begin: None, start: synthetic.clone(), exact_schema: exact, derived, factors, first, copies, hours: 0, live, built: None, passing: None, runs: 0, one_run, segment_hours, spent: std::time::Duration::ZERO })
    }

    pub fn blocks_before(&self, segment: u32) -> u64 {
        self.segment_hours.iter().filter(|&&(held, hours)| held < segment && hours > 0).count() as u64
    }

    pub fn hours_before(&self, segment: u32) -> u64 {
        self.segment_hours.iter().filter(|&&(held, _)| held < segment).map(|&(_, hours)| hours).sum()
    }

    pub fn in_phase(&self) -> Vec<String> {
        (0..self.factors.factor.len())
            .filter(|&table| self.factors.factor[table] > 1 && self.factors.in_phase[table])
            .map(|table| format!("{:?}", TableId::ALL[table]))
            .collect()
    }

    fn copy_hour(&mut self, copy: usize) -> io::Result<Option<CopyHour>> {
        let factors = &self.factors;
        let derived = &self.derived;
        let wanted = |table: usize| !derived[table] && !factors.in_phase[table] && factors.factor[table] as usize > copy;
        let upstream = &mut self.copies[copy - 1];
        loop {
            let mut run = false;
            let hour = match upstream.item()? {
                None => return Ok(None),
                Some(Item::Synthetic(synthetic)) => {
                    let bytes = synthetic.manifest()?;
                    let manifest = Manifest::read(&bytes, &self.columns)?;
                    let mut hour = CopyHour { tables: vec![None; self.kinds.len()] };
                    for entry in &manifest.sections {
                        if wanted(entry.table) && synthetic.sections[entry.table].is_some() {
                            let truncated = matches!(manifest.statuses[entry.table], Status::Truncated { .. });
                            hour.tables[entry.table] = Some((Piece::Start(truncated), entry.columns.clone(), entry.ops));
                        }
                    }
                    Some(hour)
                }
                Some(Item::Record(Record::Hour { manifest, mut sections, .. })) => {
                    let manifest = self.journal.read(manifest, self.kinds.len())?;
                    let mut hour = CopyHour { tables: vec![None; self.kinds.len()] };
                    for entry in &manifest.sections {
                        let bytes = sections.next(entry.bytes)?;
                        if wanted(entry.table) {
                            let truncated = matches!(manifest.statuses[entry.table], Status::Truncated { .. });
                            let decoded = Decoded::read(bytes, &self.kinds[entry.table], truncated)?;
                            hour.tables[entry.table] = Some((Piece::Decoded(Arc::new(decoded)), entry.columns.clone(), entry.ops));
                        }
                    }
                    Some(hour)
                }
                Some(Item::Record(Record::Run { .. })) => {
                    run = true;
                    None
                }
                Some(Item::Record(_)) => None,
            };
            if run {
                if upstream.runs > 0 {
                    return Err(invalid("A multiplied journal holds one run"));
                }
                upstream.runs += 1;
            }
            upstream.advance();
            if hour.is_some() {
                return Ok(hour);
            }
        }
    }

    fn merge(&mut self, table: usize, parts: &[(usize, &Decoded)]) -> io::Result<Vec<u8>> {
        let factor = self.factors.factor[table];
        let kinds = &self.kinds[table];
        let at = |slot: u32, copy: usize| slot * factor + copy as u32;
        let mut changed_all: Vec<u32> = Vec::new();
        let mut targets: Vec<Vec<Vec<u32>>> = Vec::with_capacity(parts.len());
        for &(copy, part) in parts {
            if factor > 1 && part.keys.iter().any(|key| key >> 120 != 0) {
                return Err(invalid(format!("{:?} is multiplied, and its keys use the top byte the copies take", TableId::ALL[table])));
            }
            let live = &mut self.live[table][copy];
            let mut words = std::mem::take(live).words().to_vec();
            part.deleted.iter().for_each(|&slot| clear_bit(&mut words, slot));
            part.inserted.iter().for_each(|&slot| set_bit(&mut words, slot));
            let mut found = Vec::with_capacity(part.runs.len());
            let mut all = part.changed.clone();
            for run in &part.runs {
                if run.op != Op::Fill {
                    found.push(Vec::new());
                    continue;
                }
                let mut hits = Vec::new();
                for (index, &word) in words.iter().enumerate() {
                    let mut rest = word;
                    while rest != 0 {
                        let slot = index as u32 * 64 + rest.trailing_zeros();
                        rest &= rest - 1;
                        if run.except.binary_search(&slot).is_err() {
                            hits.push(slot);
                        }
                    }
                }
                all.extend_from_slice(&hits);
                found.push(hits);
            }
            all.sort_unstable();
            all.dedup();
            *live = Bits::from_words(words)?;
            changed_all.extend(all.iter().map(|&slot| at(slot, copy)));
            targets.push(found);
        }
        changed_all.sort_unstable();
        let place = |slot: u32| changed_all.binary_search(&slot).expect("a changed slot") as u32;
        let mut merged = Decoded::default();
        for (index, &(copy, part)) in parts.iter().enumerate() {
            for (run, hits) in part.runs.iter().zip(&targets[index]) {
                let remap = |positions: &[u32]| -> Vec<u32> {
                    positions.iter().map(|&position| place(at(part.changed[position as usize], copy))).collect()
                };
                let owned = match run.op {
                    Op::Fill => {
                        if hits.is_empty() {
                            continue;
                        }
                        let value = run.values.as_ref().ok_or_else(|| invalid("A FILL carries no value"))?;
                        OwnedRun {
                            column: run.column,
                            op: Op::Set,
                            positions: hits.iter().map(|&slot| place(at(slot, copy))).collect(),
                            values: Some(Col::gather(&[value], &vec![(0, 0); hits.len()])),
                            except: Vec::new(),
                            patches: None,
                        }
                    }
                    _ => OwnedRun { positions: remap(&run.positions), ..run.clone() },
                };
                merged.runs.push(owned);
            }
        }
        merged.changed = changed_all.clone();
        let union = |lists: &mut dyn Iterator<Item = (usize, &Vec<u32>)>| -> Vec<u32> {
            let mut all: Vec<u32> = Vec::new();
            for (copy, list) in lists {
                all.extend(list.iter().map(|&slot| at(slot, copy)));
            }
            all.sort_unstable();
            all
        };
        merged.deleted = union(&mut parts.iter().map(|&(copy, part)| (copy, &part.deleted)));
        merged.touched = union(&mut parts.iter().map(|&(copy, part)| (copy, &part.touched)));
        if parts.iter().any(|(_, part)| part.read.is_some()) {
            let empty = Vec::new();
            merged.read = Some(union(&mut parts.iter().map(|&(copy, part)| (copy, part.read.as_ref().unwrap_or(&empty)))));
        }
        let mut rows: Vec<(u32, usize, usize)> = Vec::new();
        for (index, &(copy, part)) in parts.iter().enumerate() {
            rows.extend(part.inserted.iter().enumerate().map(|(row, &slot)| (at(slot, copy), index, row)));
        }
        rows.sort_unstable();
        merged.inserted = rows.iter().map(|row| row.0).collect();
        merged.keys = rows.iter().map(|&(_, index, row)| parts[index].1.keys[row] | (parts[index].0 as u128) << 120).collect();
        if !rows.is_empty() {
            let picks: Vec<(usize, usize)> = rows.iter().map(|&(_, index, row)| (index, row)).collect();
            let holder = parts.iter().position(|(_, part)| !part.inserts.is_empty()).ok_or_else(|| invalid("Inserted rows carry no values"))?;
            for column in 0..kinds.len() {
                let columns: Vec<&Col> =
                    parts.iter().map(|(_, part)| part.inserts.get(column).unwrap_or(&parts[holder].1.inserts[column])).collect();
                merged.inserts.push(Col::gather(&columns, &picks));
            }
        }
        let mut unrecorded: Vec<(usize, Vec<u32>)> = Vec::new();
        for &(copy, part) in parts {
            for (column, slots) in &part.unrecorded {
                let moved = slots.iter().map(|&slot| at(slot, copy));
                match unrecorded.iter_mut().find(|(held, _)| held == column) {
                    Some((_, held)) => held.extend(moved),
                    None => unrecorded.push((*column, moved.collect())),
                }
            }
        }
        unrecorded.sort_unstable_by_key(|(column, _)| *column);
        for (_, slots) in &mut unrecorded {
            slots.sort_unstable();
        }
        merged.unrecorded = unrecorded;
        merged.write(kinds)
    }

    fn pass(&mut self) -> io::Result<bool> {
        if self.factors.factor.iter().any(|&factor| factor > 1) || self.first.synthetic.is_some() || self.first.at < self.first.prefix.len() {
            return Ok(false);
        }
        let Some(Item::Record(Record::Hour { witness, manifest, .. })) = self.first.item()? else {
            return Ok(false);
        };
        let original = self.journal.read(manifest, self.kinds.len())?;
        if original.sections.iter().any(|entry| self.derived[entry.table]) {
            return Ok(false);
        }
        let widened = widen(original, &self.derived).write(&self.columns)?;
        self.passing = Some((witness, widened));
        Ok(true)
    }

    fn build(&mut self) -> io::Result<()> {
        let _label = label();
        let started = std::time::Instant::now();
        let hour_index = self.hours;
        let tables = self.kinds.len();
        let mut copy_hours: Vec<Option<CopyHour>> = Vec::with_capacity(self.copies.len());
        for copy in 1..=self.copies.len() {
            copy_hours.push(if hour_index >= LAG * copy as u64 { self.copy_hour(copy)? } else { None });
        }
        let (witness, manifest_bytes, firsts) = match self.first.item()? {
            Some(Item::Synthetic(synthetic)) => {
                let bytes = synthetic.manifest()?;
                let manifest = Manifest::read(&bytes, &self.columns)?;
                let mut firsts: Vec<Option<(First, Vec<u64>, u8)>> = (0..tables).map(|_| None).collect();
                for entry in &manifest.sections {
                    if !self.derived[entry.table] && synthetic.sections[entry.table].is_some() {
                        let truncated = matches!(manifest.statuses[entry.table], Status::Truncated { .. });
                        firsts[entry.table] = Some((First::Start(truncated), entry.columns.clone(), entry.ops));
                    }
                }
                (synthetic.witness, bytes, firsts)
            }
            Some(Item::Record(Record::Hour { witness, manifest, mut sections })) => {
                let original = self.journal.read(manifest, self.kinds.len())?;
                let journal_sections = original.sections.clone();
                let manifest = widen(original, &self.derived);
                let manifest_bytes = manifest.write(&self.columns)?;
                let mut firsts: Vec<Option<(First, Vec<u64>, u8)>> = (0..tables).map(|_| None).collect();
                for entry in &journal_sections {
                    let bytes = sections.next(entry.bytes)?;
                    let table = entry.table;
                    if self.derived[table] {
                        continue;
                    }
                    let first = if self.factors.factor[table] > 1 {
                        let truncated = matches!(manifest.statuses[table], Status::Truncated { .. });
                        First::Decoded(Arc::new(Decoded::read(bytes, &self.kinds[table], truncated)?))
                    } else {
                        First::Raw(bytes.to_vec())
                    };
                    firsts[table] = Some((first, entry.columns.clone(), entry.ops));
                }
                (witness, manifest_bytes, firsts)
            }
            _ => return Err(invalid("An hour was asked of a record that is none")),
        };
        let manifest = Manifest::read(&manifest_bytes, &self.columns)?;
        let start = self.start.clone();
        let from_start = |table: usize, kinds: &[Kind], truncated: bool| -> io::Result<Arc<Decoded>> {
            let start = start.as_ref().ok_or_else(|| invalid("A start's section without a start"))?;
            Ok(Arc::new(start.decoded(table, kinds, truncated)?.ok_or_else(|| invalid("The start holds no section of the table"))?))
        };
        let mut sections = Vec::new();
        let mut entries = Vec::new();
        for (table, first) in firsts.into_iter().enumerate() {
            if self.derived[table] {
                continue;
            }
            let factor = self.factors.factor[table] as usize;
            let mut column_bits: Vec<u64> = first.as_ref().map(|held| held.1.clone()).unwrap_or_default();
            let mut ops = first.as_ref().map_or(0, |held| held.2);
            let bytes = if factor == 1 {
                match first {
                    None => continue,
                    Some((First::Raw(raw), _, _)) => raw,
                    Some((First::Decoded(decoded), _, _)) => decoded.write(&self.kinds[table])?,
                    Some((First::Start(_), _, _)) => match start.as_ref().map(|start| start.section(table)).transpose()?.flatten() {
                        Some(raw) => raw,
                        None => continue,
                    },
                }
            } else {
                let mut parts: Vec<(usize, Arc<Decoded>)> = Vec::new();
                let first = match first {
                    Some((First::Decoded(decoded), _, _)) => Some(decoded),
                    Some((First::Start(truncated), _, _)) => Some(from_start(table, &self.kinds[table], truncated)?),
                    _ => None,
                };
                if let Some(decoded) = first {
                    parts.push((0, decoded.clone()));
                    if self.factors.in_phase[table] {
                        for copy in 1..factor {
                            parts.push((copy, decoded.clone()));
                        }
                    }
                }
                if !self.factors.in_phase[table] {
                    for copy in 1..factor {
                        if let Some(Some(hour)) = copy_hours.get_mut(copy - 1)
                            && let Some((piece, columns, their_ops)) = hour.tables[table].take()
                        {
                            parts.push((copy, match piece {
                                Piece::Decoded(decoded) => decoded,
                                Piece::Start(truncated) => from_start(table, &self.kinds[table], truncated)?,
                            }));
                            for (at, word) in columns.iter().enumerate() {
                                if at >= column_bits.len() {
                                    column_bits.resize(at + 1, 0);
                                }
                                column_bits[at] |= word;
                            }
                            ops |= their_ops;
                        }
                    }
                }
                if parts.is_empty() {
                    continue;
                }
                ops &= !FILLS;
                let borrowed: Vec<(usize, &Decoded)> = parts.iter().map(|(copy, decoded)| (*copy, &**decoded)).collect();
                self.merge(table, &borrowed)?
            };
            entries.push(Section { table, bytes: bytes.len() as u64, columns: column_bits, ops });
            sections.extend_from_slice(&bytes);
        }
        let rebuilt = Manifest { sections: entries, ..manifest };
        self.built = Some(Built { witness, manifest: rebuilt.write(&self.columns)?, sections });
        self.spent += started.elapsed();
        Ok(())
    }
}

impl Source for Multiplied {
    fn start(&mut self, segment: u32) -> io::Result<()> {
        if segment == 0 {
            return Ok(());
        }
        if self.factors.factor.iter().any(|&factor| factor > 1) || !self.first.prefix.is_empty() || self.first.synthetic.is_some() {
            return Err(invalid("A multiplied journal, or one given a start, is read from its start"));
        }
        self.first.journal.start(segment)
    }

    fn record(&mut self) -> io::Result<Option<Record<'_>>> {
        if self.built.is_none() && self.passing.is_none() {
            let hour = match self.first.item()? {
                None => return Ok(None),
                Some(Item::Synthetic(_)) | Some(Item::Record(Record::Hour { .. })) => true,
                Some(Item::Record(Record::Run { .. })) => {
                    if self.one_run && self.runs > 0 {
                        return Err(invalid("A multiplied journal, or one given a start, holds one run"));
                    }
                    false
                }
                Some(_) => false,
            };
            if !hour {
                let begin = match self.first.item()? {
                    Some(Item::Record(Record::Begin { identity, capture })) => Some((identity.to_vec(), capture.to_vec())),
                    _ => None,
                };
                if let Some((identity, capture)) = begin {
                    self.begin = Some((current_identity(&identity, &self.kinds)?, capture));
                    let (identity, capture) = self.begin.as_ref().expect("set");
                    return Ok(Some(Record::Begin { identity, capture }));
                }
                return match self.first.item()? {
                    Some(Item::Record(record)) => Ok(Some(record)),
                    _ => Ok(None),
                };
            }
            if !self.pass()? {
                self.build()?;
            }
        }
        if let Some((witness, manifest)) = &self.passing {
            let Some(Item::Record(Record::Hour { sections, .. })) = self.first.item()? else {
                return Err(invalid("The hour passed on is gone from the journal"));
            };
            return Ok(Some(Record::Hour { witness: *witness, manifest, sections }));
        }
        let built = self.built.as_ref().expect("built");
        Ok(Some(Record::Hour { witness: built.witness, manifest: &built.manifest, sections: Sections::held(&built.sections) }))
    }

    fn advance(&mut self) {
        if self.built.take().is_some() | self.passing.take().is_some() {
            self.hours += 1;
        } else if matches!(self.first.item(), Ok(Some(Item::Record(Record::Run { .. })))) {
            self.runs += 1;
        }
        self.first.advance();
    }

    fn retire(&mut self) -> io::Result<()> {
        Ok(())
    }

    fn finish(&mut self) -> io::Result<()> {
        Ok(())
    }

    fn producing(&mut self) -> io::Result<bool> {
        Ok(false)
    }
}

#[allow(clippy::type_complexity, reason = "the hour's witness, manifest and sections, returned together")]
fn restored(
    directory: &Path,
    capture: u32,
    segment: u32,
    kinds: &[Vec<Kind>],
    derived: &[bool],
    journal_tables: &JournalTables,
    mut appends: Vec<Vec<u8>>,
) -> io::Result<(Option<crate::columnar_journal::Witness>, Vec<u8>, Vec<Option<Decoded>>)> {
    let columns: Vec<usize> = kinds.iter().map(Vec::len).collect();
    let mut state = replay_columnar::walk::State::new(kinds.to_vec())?;
    let mut journal = Journal::open_read_only(directory, capture);
    journal.start(0)?;
    let mut passed = 0_u32;
    let mut all = Vec::new();
    let (witness, manifest_bytes, sections) = loop {
        match journal.record()? {
            None => return Err(invalid("The journal ends before the started run's first hour")),
            Some(Record::Next) => passed += 1,
            Some(Record::Hour { witness, manifest, mut sections }) => {
                sections.read_all(&mut all)?;
                if passed >= segment {
                    break (witness, manifest.to_vec(), std::mem::take(&mut all));
                }
                let original = journal_tables.read(manifest, kinds.len())?;
                let mut spans = Vec::new();
                let mut at = 0_usize;
                for entry in &original.sections {
                    if !derived[entry.table] {
                        spans.push(at..at + entry.bytes as usize);
                    }
                    at += entry.bytes as usize;
                }
                let manifest = widen(original, derived);
                state.apply(&manifest, |index| Ok(&all[spans[index].clone()]))?;
            }
            Some(_) => {}
        }
        journal.advance();
    };
    let original = journal_tables.read(&manifest_bytes, kinds.len())?;
    let mut spans = Vec::new();
    let mut at = 0_usize;
    for entry in &original.sections {
        if !derived[entry.table] {
            spans.push(at..at + entry.bytes as usize);
        }
        at += entry.bytes as usize;
    }
    let manifest = widen(original, derived);
    state.apply(&manifest, |index| Ok(&sections[spans[index].clone()]))?;
    let mut tables = vec![None; kinds.len()];
    let mut entries = Vec::new();
    let mut samples = Vec::new();
    for (table, table_kinds) in kinds.iter().enumerate() {
        if derived[table] {
            continue;
        }
        let from_read = (manifest.read[table / 64] >> (table % 64) & 1 != 0).then_some(manifest.hour);
        let want = state.sampled_at(table);
        if want != from_read {
            samples.push((table, want));
        }
        let Some(held) = state.table(table) else { continue };
        let truncated = matches!(manifest.statuses[table], Status::Truncated { .. });
        if truncated {
            let read = state.truncated_view(table).unwrap_or(&[]);
            if held.live().ones().any(|slot| read.binary_search(&slot).is_err()) {
                return Err(invalid(format!(
                    "{:?} is partly read at the preroll hour with unread live rows, which a synthetic first hour cannot represent",
                    TableId::ALL[table]
                )));
            }
        } else if held.live().count() == 0 {
            continue;
        }
        let mut decoded = Decoded::whole(held, table_kinds);
        if truncated {
            decoded.read = Some(Vec::new());
        }
        let (bits, ops) = if decoded.inserted.is_empty() {
            (vec![0_u64; table_kinds.len().div_ceil(64)], 0)
        } else {
            let mut bits = vec![u64::MAX; table_kinds.len().div_ceil(64)];
            if table_kinds.len() % 64 != 0 {
                *bits.last_mut().expect("a word") = (1 << (table_kinds.len() % 64)) - 1;
            }
            (bits, INSERTS)
        };
        entries.push(Section { table, bytes: 0, columns: bits, ops });
        tables[table] = Some(decoded);
    }
    let mut flags = manifest.flags;
    if samples.is_empty() {
        flags &= !SAMPLES;
    } else {
        flags |= SAMPLES;
    }
    appends.extend(manifest.appends.iter().map(|text| text.to_vec()));
    let given: Vec<&[u8]> = appends.iter().map(Vec::as_slice).collect();
    let rebuilt = Manifest { flags, appends: given, samples, sections: entries, ..manifest.clone() };
    Ok((witness, rebuilt.write(&columns)?, tables))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::columnar_journal::{JournalWriter, Source};
    use replay_columnar::envelope::{Identity, schema_digest};

    const MIDDLE: [Removed; 1] = [(2, "Removed", &[Kind::I32])];

    struct Folder(PathBuf);

    impl Folder {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!("hoi4-multiplied-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for Folder {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn before() -> Vec<Vec<Kind>> {
        let mut kinds = schema().unwrap();
        kinds.insert(MIDDLE[0].0, MIDDLE[0].2.to_vec());
        kinds
    }

    fn journal(folder: &Path, digest: Option<[u8; 32]>, removed: bool) -> Vec<(usize, Vec<u8>)> {
        let before = before();
        let identity = Identity { schema: digest.unwrap_or_else(|| schema_digest(&before)), campaign: b"campaign".to_vec(), content: Vec::new(), capture_mode: 0 }.chain_bytes().unwrap();
        let capture = replay_recording::capture::Capture {
            campaign_id: b"campaign".to_vec(),
            content: Vec::new(),
            capture_mode: 0,
            sessions: Vec::new(),
            run_sessions: Vec::new(),
            stop: None,
            format: 0,
            journaled: 0,
            controls: Vec::new(),
        };
        let mut writer = JournalWriter::create(folder, 0, &identity, &capture).unwrap();
        let _ = writer.next_segment().unwrap();
        writer.run(1, 0, 0).unwrap();
        let section = |table: usize, values: Values<'_>| {
            let hour = TableHour { read: None, changed: &[], deleted: &[], inserted: &[0], keys: &[7], inserts: &[values], runs: &[], unrecorded: &[], touched: &[] };
            (table, section::write(&before[table], &hour).unwrap())
        };
        let links = TableId::CountryLinks.index() + 1;
        let mut sections = vec![section(TableId::Campaign.index(), Values::Bytes(&[0]))];
        if removed {
            sections.push(section(MIDDLE[0].0, Values::I32(&[-5])));
        }
        sections.push(section(links, Values::U32(&[42])));
        let mut statuses = vec![Status::NotRecorded; before.len()];
        let mut read = vec![0_u64; before.len().div_ceil(64)];
        for &(table, _) in &sections {
            statuses[table] = Status::Complete;
            read[table / 64] |= 1 << (table % 64);
        }
        let manifest = Manifest {
            branch: 1,
            hour: 0,
            flags: 0,
            total_ns: 0,
            pool_bytes: 0,
            dropped_frames: 0,
            appends: vec![&b"campaign"[..]],
            statuses,
            read,
            samples: Vec::new(),
            sections: sections.iter().map(|(table, bytes)| Section { table: *table, bytes: bytes.len() as u64, columns: vec![1], ops: INSERTS }).collect(),
        };
        let columns: Vec<usize> = before.iter().map(Vec::len).collect();
        let held: Vec<&[u8]> = sections.iter().map(|(_, bytes)| bytes.as_slice()).collect();
        writer.hour(None, &manifest.write(&columns).unwrap(), &held).unwrap();
        writer.finish(None).unwrap();
        sections
    }

    #[allow(clippy::type_complexity, reason = "a test's view of the hours read")]
    fn read(folder: &Path) -> io::Result<(bool, Vec<(Vec<(usize, String)>, Vec<Status>)>)> {
        let kinds = schema()?;
        let columns: Vec<usize> = kinds.iter().map(Vec::len).collect();
        let mut source = Multiplied::open_with(folder, 0, Factors { factor: vec![1; kinds.len()], in_phase: vec![false; kinds.len()] }, None, &MIDDLE)?;
        let exact = source.exact_schema;
        let mut hours = Vec::new();
        while let Some(record) = source.record()? {
            if let Record::Hour { manifest, mut sections, .. } = record {
                let manifest = Manifest::read(manifest, &columns)?;
                let mut rows = Vec::new();
                for entry in &manifest.sections {
                    let decoded = Decoded::read(sections.next(entry.bytes)?, &kinds[entry.table], false)?;
                    rows.push((entry.table, format!("{:?} {:?}", decoded.keys, decoded.inserts)));
                }
                hours.push((rows, manifest.statuses.clone()));
            }
            source.advance();
        }
        Ok((exact, hours))
    }

    #[test]
    fn a_journal_written_with_a_table_removed_since_reads_with_every_other_table_where_this_schema_holds_it() {
        let before = before();
        let named = Folder::new("named");
        let written = journal(&named.0, None, false);
        let expected: Vec<(usize, String)> = written
            .iter()
            .map(|(table, bytes)| {
                let decoded = Decoded::read(bytes, &before[*table], false).unwrap();
                (table - usize::from(*table > MIDDLE[0].0), format!("{:?} {:?}", decoded.keys, decoded.inserts))
            })
            .collect();
        let (exact, hours) = read(&named.0).unwrap();
        assert!(exact, "recognized retired digest names the tables");
        assert_eq!(hours.len(), 1);
        let (rows, statuses) = &hours[0];
        assert_eq!(rows, &expected, "Campaign and CountryLinks at their places, rows as written");
        assert_eq!(rows.iter().map(|(table, _)| *table).collect::<Vec<_>>(), [TableId::Campaign.index(), TableId::CountryLinks.index()]);
        assert_eq!(statuses[TableId::CountryLinks.index()], Status::Complete);
        assert_eq!(statuses[TableId::Countries.index()], Status::NotRecorded, "Countries takes no status of the table before it");

        let unnamed = Folder::new("unnamed");
        journal(&unnamed.0, Some([0xab; 32]), false);
        let refused = read(&unnamed.0).err().expect("unrecognized digest shifting across MIDDLE is refused by name");
        assert!(refused.to_string().contains("unrecognized"), "{refused}");
    }

    #[test]
    fn the_capture_s_committed_blocks_are_its_segments_that_hold_hours() {
        let folder = Folder::new("blocks");
        journal(&folder.0, None, false);
        let kinds = schema().unwrap();
        let source = Multiplied::open_with(&folder.0, 0, Factors { factor: vec![1; kinds.len()], in_phase: vec![false; kinds.len()] }, None, &MIDDLE).unwrap();
        assert_eq!([0, 1, 2, 3].map(|segment| source.blocks_before(segment)), [0, 0, 1, 1], "the opening segment holds no block; the run's segment holds block 0");
    }

    #[test]
    fn a_section_of_a_table_removed_since_is_refused_by_name() {
        let folder = Folder::new("removed");
        journal(&folder.0, None, true);
        let refused = read(&folder.0).unwrap_err().to_string();
        assert!(refused.contains("The journal holds a section of Removed, a table removed from this schema since it was written"), "{refused}");
    }

    fn test_capture() -> replay_recording::capture::Capture {
        replay_recording::capture::Capture {
            campaign_id: b"campaign".to_vec(),
            content: Vec::new(),
            capture_mode: 0,
            sessions: Vec::new(),
            run_sessions: Vec::new(),
            stop: None,
            format: 0,
            journaled: 0,
            controls: Vec::new(),
        }
    }

    fn insert_section(kinds: &[Vec<Kind>], table: usize, key: u128, inserts: &[Values<'_>]) -> (usize, Vec<u8>) {
        let hour = TableHour { read: None, changed: &[], deleted: &[], inserted: &[0], keys: &[key], inserts, runs: &[], unrecorded: &[], touched: &[] };
        (table, section::write(&kinds[table], &hour).unwrap())
    }

    fn write_hour(
        writer: &mut JournalWriter,
        kinds: &[Vec<Kind>],
        hour: i32,
        sections: &[(usize, Vec<u8>)],
        samples: Vec<(usize, Option<i32>)>,
        appends: Vec<&[u8]>,
    ) {
        let columns: Vec<usize> = kinds.iter().map(Vec::len).collect();
        let mut statuses = vec![Status::NotRecorded; kinds.len()];
        let mut read = vec![0_u64; kinds.len().div_ceil(64)];
        let mut entries = Vec::with_capacity(sections.len());
        for (table, bytes) in sections {
            statuses[*table] = Status::Complete;
            read[*table / 64] |= 1 << (*table % 64);
            let mut bits = vec![u64::MAX; kinds[*table].len().div_ceil(64)];
            if kinds[*table].len() % 64 != 0 {
                *bits.last_mut().expect("a word") = (1 << (kinds[*table].len() % 64)) - 1;
            }
            entries.push(Section { table: *table, bytes: bytes.len() as u64, columns: bits, ops: INSERTS });
        }
        let manifest = Manifest {
            branch: 1,
            hour,
            flags: if samples.is_empty() { 0 } else { SAMPLES },
            total_ns: 0,
            pool_bytes: 0,
            dropped_frames: 0,
            appends,
            statuses,
            read,
            samples,
            sections: entries,
        };
        let held: Vec<&[u8]> = sections.iter().map(|(_, bytes)| bytes.as_slice()).collect();
        writer.hour(None, &manifest.write(&columns).unwrap(), &held).unwrap();
    }

    #[test]
    fn recorded_tables_after_derived_ones_pass_multiply_and_preroll_while_derived_tables_stay_excluded() {
        let kinds = schema().unwrap();
        let columns: Vec<usize> = kinds.iter().map(Vec::len).collect();
        let derived = recorder_derive::writers().tables();
        let tag_holders = TableId::TagHolders.index();
        let land_order = TableId::LandUnitOrder.index();
        let region_weights = TableId::StrategicRegionWeights.index();
        let building_inputs = TableId::StateBuildingInputs.index();
        assert!(derived.contains(&tag_holders));
        for after in [land_order, region_weights, building_inputs] {
            assert!(after > tag_holders && !derived.contains(&after));
        }

        let empty_opt = Values::OptionalI64 { present: &[0], values: &[] };
        let building_values = [
            Values::U32(&[1]),
            Values::U32(&[2]),
            Values::I32(&[3]),
            empty_opt,
            empty_opt,
            empty_opt,
            empty_opt,
            empty_opt,
            empty_opt,
            empty_opt,
            empty_opt,
            empty_opt,
        ];
        let identity = Identity { schema: schema_digest(&kinds), campaign: b"campaign".to_vec(), content: Vec::new(), capture_mode: 0 }.chain_bytes().unwrap();
        let unit_factors = || Factors { factor: vec![1; kinds.len()], in_phase: vec![false; kinds.len()] };

        let folder = Folder::new("post-derived-pass");
        let mut writer = JournalWriter::create(&folder.0, 0, &identity, &test_capture()).unwrap();
        let _ = writer.next_segment().unwrap();
        writer.run(1, 0, 0).unwrap();
        let sections = vec![
            insert_section(&kinds, TableId::Campaign.index(), 1, &[Values::Bytes(&[0])]),
            insert_section(&kinds, land_order, 2, &[Values::Opaque { ends: &[2], items: &[9, 8] }]),
            insert_section(&kinds, region_weights, 3, &[Values::I32(&[77])]),
            insert_section(&kinds, building_inputs, 4, &building_values),
        ];
        write_hour(&mut writer, &kinds, 0, &sections, vec![(region_weights, Some(0))], vec![&b"campaign"[..]]);
        writer.finish(None).unwrap();
        let mut source = Multiplied::open(&folder.0, 0, unit_factors(), None).unwrap();
        assert!(source.exact_schema);
        let mut saw_hour = false;
        while let Some(record) = source.record().unwrap() {
            if let Record::Hour { manifest, sections: mut read_sections, .. } = record {
                let manifest = Manifest::read(manifest, &columns).unwrap();
                assert_eq!(manifest.samples, [(region_weights, Some(0))]);
                assert_ne!(manifest.flags & SAMPLES, 0);
                assert_eq!(manifest.statuses[tag_holders], Status::NotRecorded);
                assert_eq!(manifest.read[tag_holders / 64] >> (tag_holders % 64) & 1, 0);
                let mut got = Vec::new();
                for entry in &manifest.sections {
                    assert_eq!(manifest.statuses[entry.table], Status::Complete);
                    assert_eq!(manifest.read[entry.table / 64] >> (entry.table % 64) & 1, 1);
                    let decoded = Decoded::read(read_sections.next(entry.bytes).unwrap(), &kinds[entry.table], false).unwrap();
                    got.push((entry.table, decoded.keys));
                }
                assert_eq!(got, [(TableId::Campaign.index(), vec![1]), (land_order, vec![2]), (region_weights, vec![3]), (building_inputs, vec![4])]);
                saw_hour = true;
            }
            source.advance();
        }
        assert!(saw_hour);

        let folder = Folder::new("post-derived-filter");
        let mut writer = JournalWriter::create(&folder.0, 0, &identity, &test_capture()).unwrap();
        let _ = writer.next_segment().unwrap();
        writer.run(1, 0, 0).unwrap();
        let first = vec![
            insert_section(&kinds, tag_holders, 5, &[Values::Bytes(&[0])]),
            insert_section(&kinds, region_weights, 6, &[Values::I32(&[88])]),
        ];
        write_hour(&mut writer, &kinds, 0, &first, vec![(tag_holders, Some(0)), (region_weights, Some(0))], vec![&b"TAG"[..]]);
        let second = vec![
            insert_section(&kinds, tag_holders, 7, &[Values::Bytes(&[0])]),
            insert_section(&kinds, region_weights, 8, &[Values::I32(&[99])]),
        ];
        write_hour(&mut writer, &kinds, 1, &second, vec![(tag_holders, Some(1))], Vec::new());
        writer.finish(None).unwrap();
        let mut source = Multiplied::open(&folder.0, 0, unit_factors(), None).unwrap();
        let mut seen = Vec::new();
        while let Some(record) = source.record().unwrap() {
            if let Record::Hour { manifest, sections: mut read_sections, .. } = record {
                let manifest = Manifest::read(manifest, &columns).unwrap();
                assert_eq!(manifest.statuses[tag_holders], Status::NotRecorded);
                assert_eq!(manifest.read[tag_holders / 64] >> (tag_holders % 64) & 1, 0);
                assert_eq!(manifest.statuses[region_weights], Status::Complete);
                assert_eq!(manifest.read[region_weights / 64] >> (region_weights % 64) & 1, 1);
                let mut rows = Vec::new();
                for entry in &manifest.sections {
                    let decoded = Decoded::read(read_sections.next(entry.bytes).unwrap(), &kinds[entry.table], false).unwrap();
                    rows.push((entry.table, decoded.keys));
                }
                seen.push((manifest.flags & SAMPLES, manifest.samples, rows));
            }
            source.advance();
        }
        assert_eq!(
            seen,
            [
                (SAMPLES, vec![(region_weights, Some(0))], vec![(region_weights, vec![6])]),
                (0, Vec::new(), vec![(region_weights, vec![8])]),
            ]
        );

        let folder = Folder::new("post-derived-multiply");
        let mut writer = JournalWriter::create(&folder.0, 0, &identity, &test_capture()).unwrap();
        let _ = writer.next_segment().unwrap();
        writer.run(1, 0, 0).unwrap();
        for hour in 0..=(LAG as i32) {
            let mut statuses = vec![Status::NotRecorded; kinds.len()];
            statuses[region_weights] = Status::Complete;
            let mut read = vec![0_u64; kinds.len().div_ceil(64)];
            read[region_weights / 64] |= 1 << (region_weights % 64);
            let sections = if hour == 0 { vec![insert_section(&kinds, region_weights, 10, &[Values::I32(&[55])])] } else { Vec::new() };
            let entries: Vec<Section> = sections.iter().map(|(table, bytes)| Section { table: *table, bytes: bytes.len() as u64, columns: vec![1], ops: INSERTS }).collect();
            let manifest = Manifest { branch: 1, hour, flags: 0, total_ns: 0, pool_bytes: 0, dropped_frames: 0, appends: Vec::new(), statuses, read, samples: Vec::new(), sections: entries };
            let held: Vec<&[u8]> = sections.iter().map(|(_, bytes)| bytes.as_slice()).collect();
            writer.hour(None, &manifest.write(&columns).unwrap(), &held).unwrap();
        }
        writer.finish(None).unwrap();
        let mut factors = unit_factors();
        factors.factor[region_weights] = 2;
        let mut source = Multiplied::open(&folder.0, 0, factors, None).unwrap();
        assert!(!source.in_phase().contains(&"StrategicRegionWeights".to_owned()));
        let mut multiplied = Vec::new();
        while let Some(record) = source.record().unwrap() {
            if let Record::Hour { manifest, sections: mut read_sections, .. } = record {
                let manifest = Manifest::read(manifest, &columns).unwrap();
                for entry in &manifest.sections {
                    let decoded = Decoded::read(read_sections.next(entry.bytes).unwrap(), &kinds[entry.table], false).unwrap();
                    if entry.table == region_weights {
                        multiplied.push((manifest.hour, decoded.inserted, decoded.keys));
                    }
                }
            }
            source.advance();
        }
        assert_eq!(multiplied, [(0, vec![0], vec![10]), (LAG as i32, vec![1], vec![10 | (1_u128 << 120)])]);

        let folder = Folder::new("post-derived-preroll");
        let mut writer = JournalWriter::create(&folder.0, 0, &identity, &test_capture()).unwrap();
        let _ = writer.next_segment().unwrap();
        writer.run(1, 0, 0).unwrap();
        for hour in 0..720_i32 {
            let mut statuses = vec![Status::NotRecorded; kinds.len()];
            statuses[region_weights] = Status::Complete;
            statuses[tag_holders] = Status::Complete;
            let mut read = vec![0_u64; kinds.len().div_ceil(64)];
            read[region_weights / 64] |= 1 << (region_weights % 64);
            let sections = if hour == 0 {
                vec![
                    insert_section(&kinds, tag_holders, 5, &[Values::Bytes(&[0])]),
                    insert_section(&kinds, region_weights, 99, &[Values::I32(&[123])]),
                ]
            } else {
                Vec::new()
            };
            let appends = if hour == 0 { vec![&b"TAG"[..]] } else { Vec::new() };
            let entries: Vec<Section> = sections.iter().map(|(table, bytes)| Section { table: *table, bytes: bytes.len() as u64, columns: vec![1], ops: INSERTS }).collect();
            let manifest = Manifest { branch: 1, hour, flags: 0, total_ns: 0, pool_bytes: 0, dropped_frames: 0, appends, statuses, read, samples: Vec::new(), sections: entries };
            let held: Vec<&[u8]> = sections.iter().map(|(_, bytes)| bytes.as_slice()).collect();
            writer.hour(None, &manifest.write(&columns).unwrap(), &held).unwrap();
        }
        let _ = writer.next_segment().unwrap();
        let second_hour = TableHour { read: None, changed: &[], deleted: &[], inserted: &[1], keys: &[100], inserts: &[Values::I32(&[456])], runs: &[], unrecorded: &[], touched: &[] };
        let sections = vec![
            insert_section(&kinds, tag_holders, 6, &[Values::Bytes(&[0])]),
            (region_weights, section::write(&kinds[region_weights], &second_hour).unwrap()),
        ];
        write_hour(&mut writer, &kinds, 720, &sections, Vec::new(), Vec::new());
        writer.finish(None).unwrap();
        let start_file = folder.0.join("start.bin");
        preroll(&folder.0, 0, 0, &start_file).unwrap();
        let mut source = Multiplied::open(&folder.0, 0, unit_factors(), Some(Start { block: 0, file: start_file })).unwrap();
        let mut synthetic_rows = None;
        while let Some(record) = source.record().unwrap() {
            if let Record::Hour { manifest, sections: mut read_sections, .. } = record {
                let manifest = Manifest::read(manifest, &columns).unwrap();
                assert_eq!(manifest.statuses[tag_holders], Status::NotRecorded);
                assert_eq!(manifest.statuses[region_weights], Status::Complete);
                let mut tables = Vec::new();
                for entry in &manifest.sections {
                    let decoded = Decoded::read(read_sections.next(entry.bytes).unwrap(), &kinds[entry.table], false).unwrap();
                    tables.push((entry.table, decoded.keys, format!("{:?}", decoded.inserts)));
                }
                synthetic_rows = Some(tables);
                break;
            }
            source.advance();
        }
        assert_eq!(synthetic_rows, Some(vec![(region_weights, vec![99, 100], "[I32([123, 456])]".to_owned())]));

        let before = before();
        let named = Folder::new("post-derived-middle-named");
        let identity = Identity { schema: schema_digest(&before), campaign: b"campaign".to_vec(), content: Vec::new(), capture_mode: 0 }.chain_bytes().unwrap();
        let mut writer = JournalWriter::create(&named.0, 0, &identity, &test_capture()).unwrap();
        let _ = writer.next_segment().unwrap();
        writer.run(1, 0, 0).unwrap();
        let sections = vec![
            insert_section(&before, TableId::Campaign.index(), 1, &[Values::Bytes(&[0])]),
            insert_section(&before, region_weights + 1, 77, &[Values::I32(&[321])]),
            insert_section(&before, building_inputs + 1, 78, &building_values),
        ];
        write_hour(&mut writer, &before, 0, &sections, Vec::new(), vec![&b"campaign"[..]]);
        writer.finish(None).unwrap();
        let (exact, hours) = read(&named.0).unwrap();
        assert!(exact);
        assert_eq!(hours.len(), 1);
        assert_eq!(hours[0].0.iter().map(|(table, _)| *table).collect::<Vec<_>>(), [TableId::Campaign.index(), region_weights, building_inputs]);
        assert_eq!(hours[0].0[1].1, "[77] [I32([321])]");
        assert_eq!(hours[0].1[region_weights], Status::Complete);
        assert_eq!(hours[0].1[building_inputs], Status::Complete);
        assert_eq!(hours[0].1[tag_holders], Status::NotRecorded);

        let unnamed = Folder::new("post-derived-middle-unnamed");
        let unknown = Identity { schema: [0xab; 32], campaign: b"campaign".to_vec(), content: Vec::new(), capture_mode: 0 }.chain_bytes().unwrap();
        let mut writer = JournalWriter::create(&unnamed.0, 0, &unknown, &test_capture()).unwrap();
        let _ = writer.next_segment().unwrap();
        writer.run(1, 0, 0).unwrap();
        write_hour(&mut writer, &before, 0, &sections, Vec::new(), vec![&b"campaign"[..]]);
        writer.finish(None).unwrap();
        let refused = read(&unnamed.0).err().expect("unrecognized digest with 519 tables is refused by name");
        assert!(refused.to_string().contains("unrecognized"), "{refused}");
    }

    fn feed_store(source: &mut Multiplied, kinds: &[Vec<Kind>]) -> io::Result<replay_columnar::store::Store> {
        use replay_columnar::store::{Store, journal::Journal as Adapter};
        let columns: Vec<usize> = kinds.iter().map(Vec::len).collect();
        let derived = recorder_derive::writers().tables();
        let mut store = Store::new(kinds.to_vec(), replay_recording::schema::keeps_last().to_vec());
        store.declare_logs(&replay_data::rows::logs())?;
        let mut adapter = Adapter::leaving(&derived, replay_recording::schema::names());
        while let Some(record) = source.record()? {
            match record {
                Record::Run { first_hour, .. } => store.begin_run(first_hour),
                Record::Hour { manifest, mut sections, .. } => {
                    let manifest = Manifest::read(manifest, &columns)?;
                    adapter.begin_hour(&mut store, &manifest)?;
                    for entry in &manifest.sections {
                        let bytes = sections.next(entry.bytes)?;
                        adapter.section(&mut store, entry, bytes)?;
                    }
                    adapter.applied(&mut store);
                    store.end_hour(&mut replay_columnar::unit::Discard)?;
                    adapter.moved(&mut store);
                }
                _ => {}
            }
            source.advance();
        }
        Ok(store)
    }

    fn building_values<'a>(empty_opt: &'a Values<'a>) -> [Values<'a>; 12] {
        [
            Values::U32(&[1]),
            Values::U32(&[2]),
            Values::I32(&[3]),
            *empty_opt,
            *empty_opt,
            *empty_opt,
            *empty_opt,
            *empty_opt,
            *empty_opt,
            *empty_opt,
            *empty_opt,
            *empty_opt,
        ]
    }

    #[test]
    fn an_unknown_digest_on_the_post_removal_layout_never_silently_discards_tables_after_derived_ones() {
        let kinds = schema().unwrap();
        let columns: Vec<usize> = kinds.iter().map(Vec::len).collect();
        let land_order = TableId::LandUnitOrder.index();
        let building_inputs = TableId::StateBuildingInputs.index();
        let unit_factors = || Factors { factor: vec![1; kinds.len()], in_phase: vec![false; kinds.len()] };
        let unknown = Identity { schema: [0xab; 32], campaign: b"campaign".to_vec(), content: Vec::new(), capture_mode: 0 }.chain_bytes().unwrap();

        let folder = Folder::new("unknown-post-removal-ambiguous");
        let mut writer = JournalWriter::create(&folder.0, 0, &unknown, &test_capture()).unwrap();
        let _ = writer.next_segment().unwrap();
        writer.run(1, 0, 0).unwrap();
        let sections = vec![
            insert_section(&kinds, TableId::Campaign.index(), 1, &[Values::Bytes(&[0])]),
            insert_section(&kinds, land_order, 2, &[Values::Opaque { ends: &[2], items: &[9, 8] }]),
        ];
        write_hour(&mut writer, &kinds, 0, &sections, Vec::new(), vec![&b"campaign"[..]]);
        writer.finish(None).unwrap();
        let refused = Multiplied::open(&folder.0, 0, unit_factors(), None).err().expect("ambiguous unknown-digest post-removal journal is refused, never silently drops LandUnitOrder");
        assert!(refused.to_string().contains("more than one table layout"), "{refused}");

        let folder = Folder::new("unknown-post-removal-tail-ambiguous");
        let mut writer = JournalWriter::create(&folder.0, 0, &unknown, &test_capture()).unwrap();
        let _ = writer.next_segment().unwrap();
        writer.run(1, 0, 0).unwrap();
        let empty_opt = Values::OptionalI64 { present: &[0], values: &[] };
        let building = building_values(&empty_opt);
        let sections = vec![
            insert_section(&kinds, TableId::Campaign.index(), 1, &[Values::Bytes(&[0])]),
            insert_section(&kinds, land_order, 2, &[Values::Opaque { ends: &[2], items: &[9, 8] }]),
            insert_section(&kinds, building_inputs, 3, &building),
        ];
        write_hour(&mut writer, &kinds, 0, &sections, Vec::new(), vec![&b"campaign"[..]]);
        writer.finish(None).unwrap();
        let refused = Multiplied::open(&folder.0, 0, unit_factors(), None)
            .err()
            .expect("ambiguous unknown-digest journal at kinds.len() tables is refused rather than guessing by column counts");
        assert!(refused.to_string().contains("more than one table layout"), "{refused}");

        let folder = Folder::new("unknown-post-removal-derived-drift-ambiguous");
        let army_mix = TableId::ArmyMix.index();
        let mut drifted = kinds.clone();
        drifted[army_mix].push(Kind::I64);
        let drifted_identity = Identity { schema: schema_digest(&drifted), campaign: b"campaign".to_vec(), content: Vec::new(), capture_mode: 0 }.chain_bytes().unwrap();
        let mut writer = JournalWriter::create(&folder.0, 0, &drifted_identity, &test_capture()).unwrap();
        let _ = writer.next_segment().unwrap();
        writer.run(1, 0, 0).unwrap();
        let sections = vec![
            insert_section(&drifted, army_mix, 1, &[Values::I64(&[5]), Values::I64(&[6])]),
            insert_section(&drifted, land_order, 2, &[Values::Opaque { ends: &[2], items: &[9, 8] }]),
        ];
        write_hour(&mut writer, &drifted, 0, &sections, Vec::new(), vec![&b"campaign"[..]]);
        writer.finish(None).unwrap();
        let refused = Multiplied::open(&folder.0, 0, unit_factors(), None)
            .err()
            .expect("drifted derived column count must not drop current layout and silently discard LandUnitOrder");
        assert!(refused.to_string().contains("more than one table layout"), "{refused}");

        let folder = Folder::new("unknown-post-removal-non-derived-drift-ambiguous");
        let region_weights = TableId::StrategicRegionWeights.index();
        let mut drifted_land = kinds.clone();
        drifted_land[land_order].extend([Kind::I64, Kind::I64]);
        let drifted_land_identity = Identity { schema: schema_digest(&drifted_land), campaign: b"campaign".to_vec(), content: Vec::new(), capture_mode: 0 }.chain_bytes().unwrap();
        let mut writer = JournalWriter::create(&folder.0, 0, &drifted_land_identity, &test_capture()).unwrap();
        let _ = writer.next_segment().unwrap();
        writer.run(1, 0, 0).unwrap();
        let sections = vec![
            insert_section(&drifted_land, land_order, 2, &[Values::Opaque { ends: &[2], items: &[9, 8] }, Values::I64(&[11]), Values::I64(&[22])]),
            insert_section(&drifted_land, region_weights, 3, &[Values::I32(&[0])]),
        ];
        write_hour(&mut writer, &drifted_land, 0, &sections, Vec::new(), vec![&b"campaign"[..]]);
        writer.finish(None).unwrap();
        let refused = Multiplied::open(&folder.0, 0, unit_factors(), None)
            .err()
            .expect("drifted LandUnitOrder columns must not fall back to retired layout and emit StrategicRegionWeights as LandUnitOrder");
        assert!(refused.to_string().contains("more than one table layout"), "{refused}");

        let folder = Folder::new("unknown-appended-519-table");
        let mut appended = kinds.clone();
        appended.push(vec![Kind::I32]);
        let appended_columns: Vec<usize> = appended.iter().map(Vec::len).collect();
        let appended_identity = Identity { schema: schema_digest(&appended), campaign: b"campaign".to_vec(), content: Vec::new(), capture_mode: 0 }.chain_bytes().unwrap();
        let mut writer = JournalWriter::create(&folder.0, 0, &appended_identity, &test_capture()).unwrap();
        let _ = writer.next_segment().unwrap();
        writer.run(1, 0, 0).unwrap();
        let sections = vec![insert_section(&appended, land_order, 2, &[Values::Opaque { ends: &[2], items: &[9, 8] }])];
        let mut statuses = vec![Status::NotRecorded; appended.len()];
        let mut read_bits = vec![0_u64; appended.len().div_ceil(64)];
        statuses[land_order] = Status::Complete;
        read_bits[land_order / 64] |= 1 << (land_order % 64);
        let extra = appended.len() - 1;
        statuses[extra] = Status::Complete;
        read_bits[extra / 64] |= 1 << (extra % 64);
        let entries = vec![Section { table: land_order, bytes: sections[0].1.len() as u64, columns: vec![1], ops: INSERTS }];
        let manifest = Manifest { branch: 1, hour: 0, flags: 0, total_ns: 0, pool_bytes: 0, dropped_frames: 0, appends: vec![&b"campaign"[..]], statuses, read: read_bits, samples: Vec::new(), sections: entries };
        writer.hour(None, &manifest.write(&appended_columns).unwrap(), &[sections[0].1.as_slice()]).unwrap();
        writer.finish(None).unwrap();
        let refused = Multiplied::open(&folder.0, 0, unit_factors(), None)
            .err()
            .expect("an unrecognized 519-table appended schema must never be remapped across LogForgets");
        assert!(refused.to_string().contains("unrecognized"), "{refused}");

        let folder = Folder::new("unknown-pre-addition-equivalent");
        let earlier = &drifted[..land_order];
        let mut writer = JournalWriter::create(&folder.0, 0, &unknown, &test_capture()).unwrap();
        let _ = writer.next_segment().unwrap();
        writer.run(1, 0, 0).unwrap();
        let sections = vec![
            insert_section(earlier, TableId::Campaign.index(), 1, &[Values::Bytes(&[0])]),
            insert_section(earlier, TableId::CountryLinks.index(), 4, &[Values::U32(&[42])]),
            insert_section(earlier, army_mix, 5, &[Values::I64(&[7]), Values::I64(&[8])]),
        ];
        write_hour(&mut writer, earlier, 0, &sections, Vec::new(), vec![&b"campaign"[..]]);
        writer.finish(None).unwrap();
        let mut source = Multiplied::open(&folder.0, 0, unit_factors(), None).unwrap();
        assert!(!source.exact_schema);
        let mut tables = Vec::new();
        while let Some(record) = source.record().unwrap() {
            if let Record::Hour { manifest, sections: mut read_sections, .. } = record {
                let manifest = Manifest::read(manifest, &columns).unwrap();
                assert_eq!(manifest.statuses[army_mix], Status::NotRecorded);
                for entry in &manifest.sections {
                    let decoded = Decoded::read(read_sections.next(entry.bytes).unwrap(), &kinds[entry.table], false).unwrap();
                    tables.push((entry.table, decoded.keys));
                }
            }
            source.advance();
        }
        assert_eq!(tables, [(TableId::Campaign.index(), vec![1]), (TableId::CountryLinks.index(), vec![4])]);

        let folder = Folder::new("unknown-pre-addition-strict-non-derived");
        let mut drifted_earlier = earlier.to_vec();
        let links = TableId::CountryLinks.index();
        drifted_earlier[links].push(Kind::U32);
        let drifted_earlier_identity = Identity { schema: schema_digest(&drifted_earlier), campaign: b"campaign".to_vec(), content: Vec::new(), capture_mode: 0 }.chain_bytes().unwrap();
        let mut writer = JournalWriter::create(&folder.0, 0, &drifted_earlier_identity, &test_capture()).unwrap();
        let _ = writer.next_segment().unwrap();
        writer.run(1, 0, 0).unwrap();
        let sections = vec![
            insert_section(&drifted_earlier, TableId::Campaign.index(), 1, &[Values::Bytes(&[0])]),
            insert_section(&drifted_earlier, links, 4, &[Values::U32(&[42]), Values::U32(&[99])]),
        ];
        write_hour(&mut writer, &drifted_earlier, 0, &sections, Vec::new(), vec![&b"campaign"[..]]);
        writer.finish(None).unwrap();
        let refused = Multiplied::open(&folder.0, 0, unit_factors(), None)
            .err()
            .expect("agreed unrecognized-digest layout still enforces exact column counts on non-derived tables");
        assert!(refused.to_string().contains("column past its table"), "{refused}");
    }

    #[test]
    fn preroll_carries_last_sample_hour_of_unchanged_tables_to_match_uninterrupted_playback() {
        let kinds = schema().unwrap();
        let columns: Vec<usize> = kinds.iter().map(Vec::len).collect();
        let campaign = TableId::Campaign.index();
        let region_weights = TableId::StrategicRegionWeights.index();
        let identity = Identity { schema: schema_digest(&kinds), campaign: b"campaign".to_vec(), content: Vec::new(), capture_mode: 0 }.chain_bytes().unwrap();
        let unit_factors = || Factors { factor: vec![1; kinds.len()], in_phase: vec![false; kinds.len()] };

        let folder = Folder::new("preroll-sampled-at");
        let mut writer = JournalWriter::create(&folder.0, 0, &identity, &test_capture()).unwrap();
        let _ = writer.next_segment().unwrap();
        writer.run(1, 0, 0).unwrap();
        for hour in 0..720_i32 {
            let mut statuses = vec![Status::NotRecorded; kinds.len()];
            statuses[campaign] = Status::Complete;
            statuses[region_weights] = Status::Complete;
            let mut read = vec![0_u64; kinds.len().div_ceil(64)];
            let (sections, appends) = if hour == 0 {
                read[campaign / 64] |= 1 << (campaign % 64);
                read[region_weights / 64] |= 1 << (region_weights % 64);
                (
                    vec![
                        insert_section(&kinds, campaign, 1, &[Values::Bytes(&[0])]),
                        insert_section(&kinds, region_weights, 99, &[Values::I32(&[123])]),
                    ],
                    vec![&b"campaign"[..]],
                )
            } else {
                (Vec::new(), Vec::new())
            };
            let entries: Vec<Section> = sections.iter().map(|(table, bytes)| Section { table: *table, bytes: bytes.len() as u64, columns: vec![1], ops: INSERTS }).collect();
            let manifest = Manifest { branch: 1, hour, flags: 0, total_ns: 0, pool_bytes: 0, dropped_frames: 0, appends, statuses, read, samples: Vec::new(), sections: entries };
            let held: Vec<&[u8]> = sections.iter().map(|(_, bytes)| bytes.as_slice()).collect();
            writer.hour(None, &manifest.write(&columns).unwrap(), &held).unwrap();
        }
        let _ = writer.next_segment().unwrap();
        let mut statuses = vec![Status::NotRecorded; kinds.len()];
        statuses[campaign] = Status::Complete;
        statuses[region_weights] = Status::Complete;
        let mut read = vec![0_u64; kinds.len().div_ceil(64)];
        read[campaign / 64] |= 1 << (campaign % 64);
        let manifest = Manifest { branch: 1, hour: 720, flags: 0, total_ns: 0, pool_bytes: 0, dropped_frames: 0, appends: Vec::new(), statuses, read, samples: Vec::new(), sections: Vec::new() };
        writer.hour(None, &manifest.write(&columns).unwrap(), &[]).unwrap();
        writer.finish(None).unwrap();

        let mut uninterrupted = Multiplied::open(&folder.0, 0, unit_factors(), None).unwrap();
        let full = feed_store(&mut uninterrupted, &kinds).unwrap();
        let start_file = folder.0.join("start.bin");
        preroll(&folder.0, 0, 0, &start_file).unwrap();
        let mut prerolled = Multiplied::open(&folder.0, 0, unit_factors(), Some(Start { block: 0, file: start_file })).unwrap();
        let started = feed_store(&mut prerolled, &kinds).unwrap();
        for table in [campaign, region_weights] {
            let want = full.table_state(table).unwrap();
            let got = started.table_state(table).unwrap();
            assert_eq!(got.status(), want.status(), "table {table} status");
            assert_eq!(got.sampled_at(), want.sampled_at(), "table {table} sampled_at");
        }
        assert_eq!(started.table_state(region_weights).unwrap().sampled_at(), Some(0));
        assert_eq!(started.table_state(campaign).unwrap().sampled_at(), Some(720));
    }

    #[test]
    fn preroll_preserves_truncated_sections_and_refuses_unread_live_rows_at_the_boundary() {
        let kinds = schema().unwrap();
        let columns: Vec<usize> = kinds.iter().map(Vec::len).collect();
        let building_inputs = TableId::StateBuildingInputs.index();
        let building_bits = vec![(1_u64 << kinds[building_inputs].len()) - 1];
        let identity = Identity { schema: schema_digest(&kinds), campaign: b"campaign".to_vec(), content: Vec::new(), capture_mode: 0 }.chain_bytes().unwrap();
        let unit_factors = || Factors { factor: vec![1; kinds.len()], in_phase: vec![false; kinds.len()] };
        let empty_opt = Values::OptionalI64 { present: &[0], values: &[] };
        let building = building_values(&empty_opt);

        let folder = Folder::new("preroll-truncated-all-read");
        let mut writer = JournalWriter::create(&folder.0, 0, &identity, &test_capture()).unwrap();
        let _ = writer.next_segment().unwrap();
        writer.run(1, 0, 0).unwrap();
        for hour in 0..720_i32 {
            let mut statuses = vec![Status::NotRecorded; kinds.len()];
            statuses[building_inputs] = Status::Complete;
            let mut read = vec![0_u64; kinds.len().div_ceil(64)];
            read[building_inputs / 64] |= 1 << (building_inputs % 64);
            let sections = if hour == 0 { vec![insert_section(&kinds, building_inputs, 4, &building)] } else { Vec::new() };
            let entries: Vec<Section> = sections.iter().map(|(table, bytes)| Section { table: *table, bytes: bytes.len() as u64, columns: building_bits.clone(), ops: INSERTS }).collect();
            let manifest = Manifest { branch: 1, hour, flags: 0, total_ns: 0, pool_bytes: 0, dropped_frames: 0, appends: Vec::new(), statuses, read, samples: Vec::new(), sections: entries };
            let held: Vec<&[u8]> = sections.iter().map(|(_, bytes)| bytes.as_slice()).collect();
            writer.hour(None, &manifest.write(&columns).unwrap(), &held).unwrap();
        }
        let _ = writer.next_segment().unwrap();
        let truncated_hour = TableHour { read: Some(&[0]), changed: &[], deleted: &[], inserted: &[], keys: &[], inserts: &[], runs: &[], unrecorded: &[], touched: &[] };
        let truncated_bytes = section::write(&kinds[building_inputs], &truncated_hour).unwrap();
        let mut statuses = vec![Status::NotRecorded; kinds.len()];
        statuses[building_inputs] = Status::Truncated { missing: 2 };
        let mut read = vec![0_u64; kinds.len().div_ceil(64)];
        read[building_inputs / 64] |= 1 << (building_inputs % 64);
        let entries = vec![Section { table: building_inputs, bytes: truncated_bytes.len() as u64, columns: vec![0], ops: 0 }];
        let manifest = Manifest { branch: 1, hour: 720, flags: 0, total_ns: 0, pool_bytes: 0, dropped_frames: 0, appends: Vec::new(), statuses, read, samples: Vec::new(), sections: entries };
        writer.hour(None, &manifest.write(&columns).unwrap(), &[truncated_bytes.as_slice()]).unwrap();
        writer.finish(None).unwrap();
        let start_file = folder.0.join("start.bin");
        preroll(&folder.0, 0, 0, &start_file).unwrap();
        let mut source = Multiplied::open(&folder.0, 0, unit_factors(), Some(Start { block: 0, file: start_file.clone() })).unwrap();
        let store = feed_store(&mut source, &kinds).unwrap();
        let state = store.table_state(building_inputs).unwrap();
        assert_eq!(state.status(), Status::Truncated { missing: 2 });
        assert_eq!(state.view(), Some(&[0][..]));
        let mut factors = unit_factors();
        factors.factor[building_inputs] = 2;
        let mut scaled = Multiplied::open(&folder.0, 0, factors, Some(Start { block: 0, file: start_file })).unwrap();
        let scaled_store = feed_store(&mut scaled, &kinds).unwrap();
        let scaled_state = scaled_store.table_state(building_inputs).unwrap();
        assert_eq!(scaled_state.status(), Status::Truncated { missing: 2 });
        assert_eq!(scaled_state.view(), Some(&[0, 1][..]));

        let folder = Folder::new("preroll-truncated-empty");
        let mut writer = JournalWriter::create(&folder.0, 0, &identity, &test_capture()).unwrap();
        let _ = writer.next_segment().unwrap();
        writer.run(1, 0, 0).unwrap();
        for hour in 0..720_i32 {
            let manifest = Manifest { branch: 1, hour, flags: 0, total_ns: 0, pool_bytes: 0, dropped_frames: 0, appends: Vec::new(), statuses: vec![Status::NotRecorded; kinds.len()], read: vec![0; kinds.len().div_ceil(64)], samples: Vec::new(), sections: Vec::new() };
            writer.hour(None, &manifest.write(&columns).unwrap(), &[]).unwrap();
        }
        let _ = writer.next_segment().unwrap();
        let empty_truncated = TableHour { read: Some(&[]), changed: &[], deleted: &[], inserted: &[], keys: &[], inserts: &[], runs: &[], unrecorded: &[], touched: &[] };
        let empty_bytes = section::write(&kinds[building_inputs], &empty_truncated).unwrap();
        let mut statuses = vec![Status::NotRecorded; kinds.len()];
        statuses[building_inputs] = Status::Truncated { missing: 3 };
        let mut read = vec![0_u64; kinds.len().div_ceil(64)];
        read[building_inputs / 64] |= 1 << (building_inputs % 64);
        let entries = vec![Section { table: building_inputs, bytes: empty_bytes.len() as u64, columns: vec![0], ops: 0 }];
        let manifest = Manifest { branch: 1, hour: 720, flags: 0, total_ns: 0, pool_bytes: 0, dropped_frames: 0, appends: Vec::new(), statuses, read, samples: Vec::new(), sections: entries };
        writer.hour(None, &manifest.write(&columns).unwrap(), &[empty_bytes.as_slice()]).unwrap();
        writer.finish(None).unwrap();
        let start_file = folder.0.join("start.bin");
        preroll(&folder.0, 0, 0, &start_file).unwrap();
        let mut source = Multiplied::open(&folder.0, 0, unit_factors(), Some(Start { block: 0, file: start_file })).unwrap();
        let store = feed_store(&mut source, &kinds).unwrap();
        let state = store.table_state(building_inputs).unwrap();
        assert_eq!(state.status(), Status::Truncated { missing: 3 });
        assert_eq!(state.view(), Some(&[][..]));

        let folder = Folder::new("preroll-truncated-unread");
        let mut writer = JournalWriter::create(&folder.0, 0, &identity, &test_capture()).unwrap();
        let _ = writer.next_segment().unwrap();
        writer.run(1, 0, 0).unwrap();
        let two_empty_opt = Values::OptionalI64 { present: &[0], values: &[] };
        let two_building = [
            Values::U32(&[1, 1]),
            Values::U32(&[2, 2]),
            Values::I32(&[3, 4]),
            two_empty_opt,
            two_empty_opt,
            two_empty_opt,
            two_empty_opt,
            two_empty_opt,
            two_empty_opt,
            two_empty_opt,
            two_empty_opt,
            two_empty_opt,
        ];
        for hour in 0..720_i32 {
            let mut statuses = vec![Status::NotRecorded; kinds.len()];
            statuses[building_inputs] = Status::Complete;
            let mut read = vec![0_u64; kinds.len().div_ceil(64)];
            read[building_inputs / 64] |= 1 << (building_inputs % 64);
            let sections = if hour == 0 {
                let first = TableHour { read: None, changed: &[], deleted: &[], inserted: &[0, 1], keys: &[4, 5], inserts: &two_building, runs: &[], unrecorded: &[], touched: &[] };
                vec![(building_inputs, section::write(&kinds[building_inputs], &first).unwrap())]
            } else {
                Vec::new()
            };
            let entries: Vec<Section> = sections.iter().map(|(table, bytes)| Section { table: *table, bytes: bytes.len() as u64, columns: building_bits.clone(), ops: INSERTS }).collect();
            let manifest = Manifest { branch: 1, hour, flags: 0, total_ns: 0, pool_bytes: 0, dropped_frames: 0, appends: Vec::new(), statuses, read, samples: Vec::new(), sections: entries };
            let held: Vec<&[u8]> = sections.iter().map(|(_, bytes)| bytes.as_slice()).collect();
            writer.hour(None, &manifest.write(&columns).unwrap(), &held).unwrap();
        }
        let _ = writer.next_segment().unwrap();
        let partial_hour = TableHour { read: Some(&[0]), changed: &[], deleted: &[], inserted: &[], keys: &[], inserts: &[], runs: &[], unrecorded: &[], touched: &[] };
        let partial_bytes = section::write(&kinds[building_inputs], &partial_hour).unwrap();
        let mut statuses = vec![Status::NotRecorded; kinds.len()];
        statuses[building_inputs] = Status::Truncated { missing: 1 };
        let mut read = vec![0_u64; kinds.len().div_ceil(64)];
        read[building_inputs / 64] |= 1 << (building_inputs % 64);
        let entries = vec![Section { table: building_inputs, bytes: partial_bytes.len() as u64, columns: vec![0], ops: 0 }];
        let manifest = Manifest { branch: 1, hour: 720, flags: 0, total_ns: 0, pool_bytes: 0, dropped_frames: 0, appends: Vec::new(), statuses, read, samples: Vec::new(), sections: entries };
        writer.hour(None, &manifest.write(&columns).unwrap(), &[partial_bytes.as_slice()]).unwrap();
        writer.finish(None).unwrap();
        let start_file = folder.0.join("start.bin");
        let refused = preroll(&folder.0, 0, 0, &start_file).unwrap_err();
        assert!(refused.to_string().contains("StateBuildingInputs"), "{refused}");
    }
}
