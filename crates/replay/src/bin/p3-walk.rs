//! Walks a `p3-convert` file (version 2) from its first hour through replay-columnar's
//! reader and reports, per recorded hour, the CPU of decompressing units, reading
//! manifests and applying sections, then the time to start a walker at each restart.
//! With `--rows`, also builds the recording's row values for every slot each hour
//! changed or inserted, as families read them. With `--table=<Name>`, also reports that
//! table's live rows and changes per hour. Measurement only:
//! `p3-walk <file> [repetitions] [--rows] [--table=<Name>]`.

use replay_columnar::{
    framing::{self, Unit},
    index::{Index, Record},
    leaf::Reader,
    manifest::Manifest,
    state::{Cell, Kind as Column, Table},
    walk::{Dictionary, State},
};
use replay_data::{
    codec::Decoder,
    rows::{Field, Kind, List, Opaque, TableId},
};
use serde_json::json;
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    time::Instant,
};

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

fn column(kind: Kind) -> Column {
    match kind {
        Kind::U8 => Column::U8,
        Kind::I8 => Column::I8,
        Kind::U16 => Column::U16,
        Kind::I16 => Column::I16,
        Kind::U32 => Column::U32,
        Kind::I32 => Column::I32,
        Kind::U64 => Column::U64,
        Kind::I64 => Column::I64,
        Kind::Bool => Column::Bool,
        Kind::OptionalI32 => Column::OptionalI32,
        Kind::OptionalU32 => Column::OptionalU32,
        Kind::OptionalI64 => Column::OptionalI64,
        Kind::OptionalU64 => Column::OptionalU64,
        Kind::Bytes => Column::Bytes,
        Kind::U32List => Column::U32List,
        Kind::U64List => Column::U64List,
        Kind::Opaque => Column::Opaque,
    }
}

/// This thread's CPU time in nanoseconds, at the scheduler's resolution.
#[cfg(windows)]
fn thread_ns() -> u64 {
    use windows_sys::Win32::{
        Foundation::FILETIME,
        System::Threading::{GetCurrentThread, GetThreadTimes},
    };
    let zero = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let (mut created, mut exited, mut kernel, mut user) = (zero, zero, zero, zero);
    // SAFETY: the pseudo-handle of the current thread and four owned FILETIMEs.
    unsafe {
        GetThreadTimes(
            GetCurrentThread(),
            &mut created,
            &mut exited,
            &mut kernel,
            &mut user,
        )
    };
    let ticks =
        |time: FILETIME| u64::from(time.dwHighDateTime) << 32 | u64::from(time.dwLowDateTime);
    (ticks(kernel) + ticks(user)) * 100
}

#[cfg(not(windows))]
fn thread_ns() -> u64 {
    0
}

fn record(bytes: &[u8], offset: u64, raw: u64, stored: u64) -> io::Result<Vec<u8>> {
    let start = usize::try_from(offset).map_err(|_| invalid("Record offset exceeds memory"))?;
    let length = usize::try_from(stored).map_err(|_| invalid("Record length exceeds memory"))?;
    let stored = start
        .checked_add(length)
        .and_then(|end| bytes.get(start..end))
        .ok_or_else(|| invalid("Record passes the file"))?;
    // The frame must say the length the index does, before that much is allocated.
    if zstd::zstd_safe::get_frame_content_size(stored).ok().flatten() != Some(raw) {
        return Err(invalid("Record's frame and index disagree on its length"));
    }
    let raw = usize::try_from(raw).map_err(|_| invalid("Record length exceeds memory"))?;
    let mut raw_bytes = engine_allocation::vec(raw)?;
    zstd::bulk::Decompressor::new()?.decompress_to_buffer(stored, &mut raw_bytes)?;
    if raw_bytes.len() != raw {
        return Err(invalid("Record decompresses to another length"));
    }
    Ok(raw_bytes)
}

fn unit(bytes: &[u8], unit: &Unit) -> io::Result<Vec<u8>> {
    record(bytes, unit.offset, unit.raw, unit.stored)
}

fn restart_record(bytes: &[u8], record_at: &Record) -> io::Result<Vec<u8>> {
    record(bytes, record_at.offset, record_at.raw, record_at.stored)
}

/// The format `p3-convert` encodes rows and opaque cells in: this build's.
const FORMAT: u16 = replay_data::FORMAT_VERSION;

fn field<'a>(cell: Cell<'a>, dictionary: &'a Dictionary) -> io::Result<Field<'a>> {
    Ok(match cell {
        Cell::U8(value) => Field::U8(value),
        Cell::I8(value) => Field::I8(value),
        Cell::U16(value) => Field::U16(value),
        Cell::I16(value) => Field::I16(value),
        Cell::U32(value) => Field::U32(value),
        Cell::I32(value) => Field::I32(value),
        Cell::U64(value) => Field::U64(value),
        Cell::I64(value) => Field::I64(value),
        Cell::Bool(value) => Field::Bool(value),
        Cell::OptionalI32(value) => Field::OptionalI32(value),
        Cell::OptionalU32(value) => Field::OptionalU32(value),
        Cell::OptionalI64(value) => Field::OptionalI64(value),
        Cell::OptionalU64(value) => Field::OptionalU64(value),
        Cell::Bytes(id) => Field::Bytes(
            dictionary
                .get(id)
                .ok_or_else(|| invalid("A text cell names a missing dictionary entry"))?,
        ),
        Cell::U32List(items) => Field::U32List(List::Native(items)),
        Cell::U64List(items) => Field::U64List(List::Native(items)),
        Cell::Opaque(bytes) => Field::Opaque(Opaque::Encoded(bytes, FORMAT)),
    })
}

/// The recording's rows at `slots`, in key order, through the rows' own field codec.
fn rows(
    id: TableId,
    table: &Table,
    dictionary: &Dictionary,
    slots: impl Iterator<Item = u32>,
    keyed: &mut Vec<(u128, u32)>,
    buffer: &mut Vec<u8>,
) -> io::Result<usize> {
    keyed.clear();
    for slot in slots {
        let key = table
            .key(slot)
            .ok_or_else(|| invalid("A changed slot holds no row"))?;
        keyed.push((key, slot));
    }
    keyed.sort_unstable();
    buffer.clear();
    let mut previous = 0;
    let columns = id.schema().len();
    for &(key, slot) in keyed.iter() {
        id.encode_key(key, previous, buffer)?;
        previous = key;
        for column in 0..columns {
            // A snapshot row has no way to say a cell was not recorded.
            if !table.recorded(column, slot) {
                return Err(invalid(format!("{id:?} slot {slot} column {column} is unrecorded")));
            }
            let cell = table
                .column(column)
                .and_then(|values| values.cell(slot))
                .ok_or_else(|| invalid("A row has no cell"))?;
            field(cell, dictionary)?.encode(buffer)?;
        }
    }
    let mut decoder = Decoder::new(buffer);
    decoder.format = FORMAT;
    let decoded = id.decode_rows(keyed.len(), &mut decoder)?;
    decoder.finish()?;
    std::hint::black_box(decoded);
    Ok(keyed.len())
}

#[derive(Debug, PartialEq)]
struct Options {
    path: String,
    repetitions: usize,
    materialize: bool,
    table: Option<String>,
}

/// The file, then the optional repetition count and options in any order.
fn options(mut args: impl Iterator<Item = String>) -> io::Result<Options> {
    let usage = || invalid("p3-walk <file.h4col> [repetitions] [--rows] [--table=<Name>]");
    let path = args.next().ok_or_else(usage)?;
    let (mut repetitions, mut materialize, mut table) = (None, false, None);
    for arg in args {
        if arg == "--rows" {
            materialize = true;
        } else if let Some(name) = arg.strip_prefix("--table=") {
            if table.replace(name.to_owned()).is_some() {
                return Err(usage());
            }
        } else if arg.starts_with("--") || repetitions.is_some() {
            return Err(usage());
        } else {
            repetitions = Some(
                arg.parse()
                    .map_err(|_| invalid("Repetitions is not a count"))?,
            );
        }
    }
    Ok(Options {
        path,
        repetitions: repetitions.unwrap_or(3),
        materialize,
        table,
    })
}

fn main() -> io::Result<()> {
    let Options {
        path,
        repetitions,
        materialize,
        table,
    } = options(std::env::args().skip(1))?;
    let watched = table
        .map(|name| {
            TableId::ALL
                .iter()
                .position(|id| format!("{id:?}") == name)
                .ok_or_else(|| invalid(format!("No table {name}")))
        })
        .transpose()?;
    let bytes = std::fs::read(&path)?;
    if !bytes.starts_with(b"H4COLUMN") || bytes.get(8..12) != Some(&2u32.to_le_bytes()[..]) {
        return Err(invalid("Not a version 2 H4COLUMN file"));
    }
    let end = bytes
        .len()
        .checked_sub(8)
        .filter(|&end| end >= 12)
        .ok_or_else(|| invalid("File ends before its index offset"))?;
    let index_at = usize::try_from(u64::from_le_bytes(
        bytes[end..].try_into().expect("eight bytes"),
    ))
    .ok()
    .filter(|&at| (12..=end).contains(&at))
    .ok_or_else(|| invalid("Index offset passes the file"))?;
    let index = Index::read(&bytes[index_at..end])?;
    let schema: Vec<Vec<Column>> = TableId::ALL
        .iter()
        .map(|id| id.schema().iter().map(|&kind| column(kind)).collect())
        .collect();
    let mut positions: BTreeMap<u32, Vec<&Unit>> = BTreeMap::new();
    for unit in &index.units {
        positions.entry(unit.first).or_default().push(unit);
    }

    let mut runs = Vec::new();
    for _ in 0..repetitions {
        let mut state = State::new(schema.clone())?;
        let columns = state.columns().to_vec();
        let (mut decompress_ns, mut manifest_ns, mut apply_ns) = (0u128, 0u128, 0u128);
        let (mut hours, mut sections, mut changed, mut inserted, mut deleted, mut filled) =
            (0u64, 0u64, 0u64, 0u64, 0u64, 0u64);
        let mut peak_raw = 0u64;
        let (mut rows_ns, mut rows_built) = (0u128, 0u64);
        let (mut keyed, mut buffer) = (Vec::new(), Vec::new());
        // Watched table: hours by live-row count, and hours it changed, inserted or deleted.
        let mut live_hours: BTreeMap<usize, u64> = BTreeMap::new();
        let (mut watched_sections, mut watched_changed, mut watched_inserted, mut watched_deleted) =
            (0u64, 0u64, 0u64, 0u64);
        let mut watched_samples = Vec::new();
        let cpu = thread_ns();
        let wall = Instant::now();
        for units in positions.values() {
            let begin = Instant::now();
            let mut manifest_unit = None;
            let mut tables: Vec<Option<Vec<u8>>> = (0..schema.len()).map(|_| None).collect();
            for unit_at in units {
                let raw = unit(&bytes, unit_at)?;
                if unit_at.stream == framing::MANIFESTS {
                    manifest_unit = Some(raw);
                } else {
                    let table = (unit_at.stream - 1) as usize;
                    *tables
                        .get_mut(table)
                        .ok_or_else(|| invalid("Unit names a stream past the schema"))? = Some(raw);
                }
            }
            decompress_ns += begin.elapsed().as_nanos();
            peak_raw = peak_raw.max(units.iter().map(|unit| unit.raw).sum());
            let manifest_unit =
                manifest_unit.ok_or_else(|| invalid("A unit position has no manifests"))?;
            let mut readers: Vec<Option<Reader<'_>>> = tables
                .iter()
                .map(|bytes| bytes.as_deref().map(Reader::new))
                .collect();
            let mut manifests = Reader::new(&manifest_unit);
            while let Some(manifest_bytes) =
                framing::next_manifest(&mut manifests, manifest_unit.len())?
            {
                let read = Instant::now();
                let manifest = Manifest::read(manifest_bytes, &columns)?;
                let mut hour_sections = Vec::with_capacity(manifest.sections.len());
                for section in &manifest.sections {
                    let reader = readers
                        .get_mut(section.table)
                        .and_then(Option::as_mut)
                        .ok_or_else(|| invalid("A section's table has no unit"))?;
                    hour_sections.push(framing::next_section(reader, section.bytes as usize)?);
                }
                let applied = Instant::now();
                manifest_ns += (applied - read).as_nanos();
                state.apply(&manifest, |index| Ok(hour_sections[index]))?;
                apply_ns += applied.elapsed().as_nanos();
                hours += 1;
                sections += manifest.sections.len() as u64;
                if materialize {
                    let built = Instant::now();
                    for section in &manifest.sections {
                        let changes = state.changes(section.table).expect("schema table");
                        rows_built += rows(
                            TableId::ALL[section.table],
                            state.table(section.table).expect("schema table"),
                            state.dictionary(),
                            changes
                                .changed
                                .iter()
                                .chain(&changes.inserted)
                                .chain(&changes.fill_slots)
                                .copied()
                                .collect::<BTreeSet<u32>>()
                                .into_iter(),
                            &mut keyed,
                            &mut buffer,
                        )? as u64;
                    }
                    rows_ns += built.elapsed().as_nanos();
                }
                if let Some(table) = watched {
                    let live = state.table(table).expect("schema table").live().count();
                    *live_hours.entry(live).or_default() += 1;
                    if manifest
                        .sections
                        .iter()
                        .any(|section| section.table == table)
                    {
                        let changes = state.changes(table).expect("schema table");
                        watched_sections += 1;
                        watched_changed += u64::from(!changes.changed.is_empty());
                        watched_inserted += u64::from(!changes.inserted.is_empty());
                        watched_deleted += u64::from(!changes.deleted.is_empty());
                        if watched_samples.len() < 12 {
                            watched_samples.push(json!({
                                "hour_ordinal": hours,
                                "live": live,
                                "changed": changes.changed.len(),
                                "inserted": changes.inserted.len(),
                                "deleted": changes.deleted.len(),
                            }));
                        }
                    }
                }
                for section in &manifest.sections {
                    let changes = state.changes(section.table).expect("schema table");
                    changed += changes.changed.len() as u64;
                    inserted += changes.inserted.len() as u64;
                    deleted += changes.deleted.len() as u64;
                    filled += changes.filled.len() as u64;
                }
            }
        }
        let cpu_ns = thread_ns() - cpu;
        let wall_ns = wall.elapsed().as_nanos();
        if hours != u64::from(index.hours) {
            return Err(invalid(format!("Walked {hours} of {} hours", index.hours)));
        }
        let live: usize = (0..schema.len())
            .map(|table| state.table(table).expect("schema table").live().count())
            .sum();

        // A walker starting at restart k restores the dictionary chunks of every restart
        // up to k, then every table.
        let mut chunks = Vec::new();
        let mut restore_ns = Vec::new();
        for restart in &index.restarts {
            chunks.push(restart_record(&bytes, &restart.chunk)?);
            let begin = Instant::now();
            let raw = restart_record(&bytes, &restart.state)?;
            let mut fresh = State::new(schema.clone())?;
            fresh.restore_dictionary(chunks.iter().map(Vec::as_slice))?;
            let mut reader = Reader::new(&raw);
            let tables = reader.count()?;
            for table in 0..tables {
                let length = reader.count()?;
                fresh.restore(table, reader.raw(length)?)?;
            }
            reader.finish()?;
            restore_ns.push(begin.elapsed().as_nanos() as f64 / 1e6);
        }
        let per_hour = |ns: u128| ns as f64 / 1000.0 / hours as f64;
        runs.push(json!({
            "thread_cpu_us_per_hour": per_hour(cpu_ns as u128),
            "wall_us_per_hour": per_hour(wall_ns),
            "decompress_us_per_hour": per_hour(decompress_ns),
            "manifest_us_per_hour": per_hour(manifest_ns),
            "apply_us_per_hour": per_hour(apply_ns),
            "rows_us_per_hour": per_hour(rows_ns),
            "rows_per_hour": rows_built as f64 / hours as f64,
            "ns_per_row": rows_ns as f64 / rows_built.max(1) as f64,
            "sections_per_hour": sections as f64 / hours as f64,
            "changed_slots_per_hour": changed as f64 / hours as f64,
            "inserted_per_hour": inserted as f64 / hours as f64,
            "deleted_per_hour": deleted as f64 / hours as f64,
            "fills_per_hour": filled as f64 / hours as f64,
            "live_rows_at_end": live,
            "peak_unit_raw_bytes": peak_raw,
            "restart_start_ms": restore_ns,
            "watched": watched.map(|table| json!({
                "table": format!("{:?}", TableId::ALL[table]),
                "hours_by_live_rows": live_hours.iter().map(|(live, hours)| json!([live, hours])).collect::<Vec<_>>(),
                "hours_with_section": watched_sections,
                "hours_changed": watched_changed,
                "hours_inserted": watched_inserted,
                "hours_deleted": watched_deleted,
                "first_sections": watched_samples,
            })),
        }));
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "file": path,
            "file_bytes": bytes.len(),
            "hours": index.hours,
            "units": index.units.len(),
            "restarts": index.restarts.len(),
            "runs": runs,
        }))?
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(args: &[&str]) -> io::Result<Options> {
        options(args.iter().map(|arg| arg.to_string()))
    }

    fn walk(repetitions: usize, materialize: bool, table: Option<&str>) -> Options {
        Options {
            path: "walk.h4col".into(),
            repetitions,
            materialize,
            table: table.map(str::to_owned),
        }
    }

    #[test]
    fn the_repetition_count_is_optional_before_or_among_the_options() {
        assert_eq!(parsed(&["walk.h4col"]).unwrap(), walk(3, false, None));
        assert_eq!(parsed(&["walk.h4col", "--rows"]).unwrap(), walk(3, true, None));
        assert_eq!(
            parsed(&["walk.h4col", "--table=Countries"]).unwrap(),
            walk(3, false, Some("Countries"))
        );
        assert_eq!(
            parsed(&["walk.h4col", "5", "--rows", "--table=Countries"]).unwrap(),
            walk(5, true, Some("Countries"))
        );
        assert_eq!(
            parsed(&["walk.h4col", "--rows", "1"]).unwrap(),
            walk(1, true, None)
        );
        let refused: [&[&str]; 5] = [
            &[],
            &["walk.h4col", "many"],
            &["walk.h4col", "1", "2"],
            &["walk.h4col", "--row"],
            &["walk.h4col", "--table=Countries", "--table=Provinces"],
        ];
        for args in refused {
            assert!(parsed(args).is_err(), "{args:?}");
        }
    }
}
