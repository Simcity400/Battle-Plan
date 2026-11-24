use core_indexes::{
    Cause, Cell, Column, Columns, Hours, Indexes, Input, Key, Link, Memory, Missing, RelationSpec,
    RelationStats, StorageStats, TableId, TableState, TableStatus, integer, write_integer,
};
use std::env;
use std::fs::{self, File};
use std::io::{self, Write};
use std::mem::size_of;
use std::path::PathBuf;
use std::time::Instant;

#[cfg(windows)]
mod clock {
    use std::ffi::c_void;
    use std::io;
    #[repr(C)]
    #[derive(Default)]
    struct FileTime {
        low: u32,
        high: u32,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentThread() -> *mut c_void;
        fn QueryThreadCycleTime(thread: *mut c_void, cycles: *mut u64) -> i32;
        fn GetThreadTimes(
            thread: *mut c_void,
            created: *mut FileTime,
            ended: *mut FileTime,
            kernel: *mut FileTime,
            user: *mut FileTime,
        ) -> i32;
    }
    pub fn cycles() -> io::Result<u64> {
        let mut cycles = 0;
        if unsafe { QueryThreadCycleTime(GetCurrentThread(), &raw mut cycles) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(cycles)
    }
    pub fn cpu_ns() -> io::Result<u64> {
        let mut created = FileTime::default();
        let mut ended = FileTime::default();
        let mut kernel = FileTime::default();
        let mut user = FileTime::default();
        if unsafe {
            GetThreadTimes(
                GetCurrentThread(),
                &raw mut created,
                &raw mut ended,
                &raw mut kernel,
                &raw mut user,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        let ticks = |t: FileTime| (u64::from(t.high) << 32) | u64::from(t.low);
        Ok((ticks(kernel) + ticks(user)) * 100)
    }
}
#[cfg(not(windows))]
mod clock {
    use std::io;
    pub fn cycles() -> io::Result<u64> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "requires Windows QueryThreadCycleTime",
        ))
    }
    pub fn cpu_ns() -> io::Result<u64> {
        cycles()
    }
}

struct Options {
    out: PathBuf,
    hours: u64,
    countries: u64,
    readers_budget: usize,
    caches: Option<Vec<usize>>,
    gate1: Option<PathBuf>,
}
fn options() -> io::Result<Options> {
    let mut o = Options {
        out: env::temp_dir().join("hoi4-core-indexes-bench"),
        hours: 113_869,
        countries: 1500,
        readers_budget: 900_000_000,
        caches: None,
        gate1: None,
    };
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        let value = args
            .next()
            .ok_or_else(|| io::Error::other("benchmark option needs a value"))?;
        match arg.as_str() {
            "--out" => o.out = value.into(),
            "--hours" => o.hours = value.parse().map_err(io::Error::other)?,
            "--countries" => o.countries = value.parse().map_err(io::Error::other)?,
            "--readers-budget" => o.readers_budget = value.parse().map_err(io::Error::other)?,
            "--cache-sizes" => {
                o.caches = Some(
                    value
                        .split(',')
                        .map(|n| n.parse().map_err(io::Error::other))
                        .collect::<io::Result<_>>()?,
                )
            }
            "--gate1" => o.gate1 = Some(value.into()),
            _ => return Err(io::Error::other("unknown benchmark option")),
        }
    }
    if o.hours == 0 || o.countries == 0 {
        return Err(io::Error::other("hours and countries must be positive"));
    }
    Ok(o)
}

#[derive(Clone)]
struct Value {
    bytes: [u8; 21],
    len: usize,
    missing: Option<Hours>,
}
impl Value {
    fn known(number: u64) -> Self {
        let mut v = Self {
            bytes: [0; 21],
            len: 0,
            missing: None,
        };
        v.len = integer(number, &mut v.bytes).len();
        v
    }
    fn cell(&self) -> Cell<'_> {
        if let Some(hours) = self.missing {
            Cell::Unknown(Cause {
                kind: Missing::Unknown,
                hours,
                reason: "synthetic unreadable cell",
            })
        } else {
            Cell::Value(&self.bytes[..self.len])
        }
    }
}
struct Field {
    column: Column,
    values: Vec<Value>,
}
struct Table {
    id: TableId,
    present: Vec<bool>,
    bases: u64,
    first: u64,
}
struct Reader {
    fields: Vec<Field>,
    tables: Vec<Table>,
}
impl Reader {
    fn table(&self, id: TableId) -> &Table {
        self.tables.iter().find(|t| t.id == id).unwrap()
    }
    fn table_mut(&mut self, id: TableId) -> &mut Table {
        self.tables.iter_mut().find(|t| t.id == id).unwrap()
    }
    fn field(&self, column: Column) -> &Field {
        self.fields.iter().find(|f| f.column == column).unwrap()
    }
    fn field_mut(&mut self, column: Column) -> &mut Field {
        self.fields.iter_mut().find(|f| f.column == column).unwrap()
    }
    fn add(
        &mut self,
        column: Column,
        rows: u64,
        bases: u64,
        first: u64,
        mut value: impl FnMut(u64) -> u64,
    ) -> io::Result<()> {
        let count = usize::try_from(rows).map_err(io::Error::other)?;
        if !self.tables.iter().any(|t| t.id == column.table) {
            self.tables.push(Table {
                id: column.table,
                present: vec![true; count],
                bases,
                first,
            });
        }
        self.fields.push(Field {
            column,
            values: (0..rows).map(|row| Value::known(value(row))).collect(),
        });
        Ok(())
    }
    fn allocated_bytes(&self) -> usize {
        size_of::<Self>()
            + self.fields.capacity() * size_of::<Field>()
            + self.tables.capacity() * size_of::<Table>()
            + self
                .fields
                .iter()
                .map(|f| f.values.capacity() * size_of::<Value>())
                .sum::<usize>()
            + self
                .tables
                .iter()
                .map(|t| t.present.capacity() * size_of::<bool>())
                .sum::<usize>()
    }
    fn change(
        &mut self,
        indexes: &mut Indexes<'_>,
        column: Column,
        row: u64,
        value: Value,
    ) -> io::Result<u64> {
        if !self.table(column.table).present[row as usize] {
            return Ok(0);
        }
        let mut key = [0; 42];
        let key = key_bytes(self.table(column.table), row, &mut key)?;
        let previous = std::mem::replace(&mut self.field_mut(column).values[row as usize], value);
        indexes.apply(
            self,
            Input::Changed {
                column,
                key,
                previous: previous.cell(),
                cell: self.field(column).values[row as usize].cell(),
            },
            |_, _| Ok(()),
        )?;
        Ok(1)
    }
    fn visibility(
        &self,
        indexes: &mut Indexes<'_>,
        column: Column,
        row: u64,
        included: bool,
    ) -> io::Result<()> {
        let mut key = [0; 42];
        let key = key_bytes(self.table(column.table), row, &mut key)?;
        indexes.apply(
            self,
            Input::Visibility {
                table: column.table,
                key,
                included,
            },
            |_, _| Ok(()),
        )
    }
    fn status(
        &mut self,
        indexes: &mut Indexes<'_>,
        table: TableId,
        state: TableState,
        hour: u64,
    ) -> io::Result<()> {
        if state == TableState::Lost {
            self.table_mut(table).present.fill(false);
        }
        indexes.apply(
            self,
            Input::Status(TableStatus {
                table,
                state,
                hours: Hours {
                    first: hour,
                    last: hour,
                },
                reason: "synthetic table coverage transition",
            }),
            |_, _| Ok(()),
        )?;
        Ok(())
    }
    fn reread_links(&mut self, indexes: &mut Indexes<'_>, column: Column) -> io::Result<u64> {
        let count = self.table(column.table).present.len();
        self.table_mut(column.table).present.fill(false);
        indexes.apply(
            self,
            Input::Reread {
                table: column.table,
            },
            |_, _| Ok(()),
        )?;
        for row in 0..count {
            let mut key = [0; 42];
            let key = key_bytes(self.table(column.table), row as u64, &mut key)?;
            self.table_mut(column.table).present[row] = true;
            indexes.apply(
                self,
                Input::Appeared {
                    table: column.table,
                    key,
                    fields: &[(column.field, self.field(column).values[row].cell())],
                },
                |_, _| Ok(()),
            )?;
        }
        Ok(count as u64)
    }
}
fn key_bytes<'b>(table: &Table, row: u64, bytes: &'b mut [u8; 42]) -> io::Result<&'b [u8]> {
    let mut target = &mut bytes[..];
    if table.bases == 0 {
        write_integer(row + table.first, &mut target)?;
    } else {
        write_integer(row % table.bases, &mut target)?;
        write_integer(row / table.bases, &mut target)?;
    }
    let len = 42 - target.len();
    Ok(&bytes[..len])
}
fn number(bytes: &[u8], at: &mut usize) -> io::Result<u64> {
    let first = *bytes
        .get(*at)
        .ok_or_else(|| io::Error::other("synthetic key ended"))?;
    *at += 1;
    if first < 128 {
        return Ok(u64::from(first));
    }
    let count = usize::from(first - 128) + 2;
    let mut value = 0u64;
    for _ in 0..count {
        value = value
            .checked_mul(128)
            .and_then(|v| bytes.get(*at).and_then(|b| v.checked_add(u64::from(*b))))
            .ok_or_else(|| io::Error::other("synthetic integer overflow"))?;
        *at += 1;
    }
    Ok(value)
}
impl Columns for Reader {
    fn out_of_view(
        &self,
        _: TableId,
        _: &mut dyn FnMut(&[u8]) -> io::Result<()>,
    ) -> io::Result<()> {
        Ok(())
    }
    fn cell<'r>(&'r self, column: Column, key: &mut dyn Key) -> io::Result<Option<Cell<'r>>> {
        let mut bytes = [0; 42];
        let len = usize::try_from(key.len()).map_err(io::Error::other)?;
        if len > bytes.len() {
            return Err(io::Error::other(
                "this synthetic adapter expects u64 key components",
            ));
        }
        key.read(0, &mut bytes[..len])?;
        let mut at = 0;
        let first = number(&bytes[..len], &mut at)?;
        let table = self.table(column.table);
        let row = if table.bases == 0 {
            first.checked_sub(table.first)
        } else {
            number(&bytes[..len], &mut at)?
                .checked_mul(table.bases)
                .and_then(|n| n.checked_add(first))
        };
        let Some(row) = row.and_then(|n| usize::try_from(n).ok()) else {
            return Ok(None);
        };
        if !table.present.get(row).copied().unwrap_or(false) {
            return Ok(None);
        }
        Ok(Some(self.field(column).values[row].cell()))
    }
    fn keys(
        &self,
        table: TableId,
        prefix: &[u8],
        visit: &mut dyn FnMut(&[u8]) -> io::Result<()>,
    ) -> io::Result<()> {
        let table = self.table(table);
        let count = table.present.len() as u64;
        let mut one = |row: u64| -> io::Result<()> {
            if table.present[row as usize] {
                let mut key = [0; 42];
                let key = key_bytes(table, row, &mut key)?;
                if key.starts_with(prefix) {
                    visit(key)?;
                }
            }
            Ok(())
        };
        if table.bases == 0 {
            for row in 0..count {
                one(row)?;
            }
        } else {
            for base in 0..table.bases {
                for ordinal in 0..count.div_ceil(table.bases) {
                    let row = ordinal * table.bases + base;
                    if row < count {
                        one(row)?;
                    }
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Default)]
struct Work {
    cycles: u64,
    inputs: u64,
    operations: u64,
    decodes: u64,
    reads: u64,
    writes: u64,
    lookups: u64,
}
fn measure<'a>(
    indexes: &mut Indexes<'a>,
    work: &mut Work,
    run: impl FnOnce(&mut Indexes<'a>) -> io::Result<u64>,
) -> io::Result<()> {
    let before = indexes.storage_stats();
    let cycles = clock::cycles()?;
    let inputs = run(indexes)?;
    work.cycles += clock::cycles()? - cycles;
    let after = indexes.storage_stats();
    work.inputs += inputs;
    work.operations += after.tree_operations - before.tree_operations;
    work.decodes += after.page_decodes - before.page_decodes;
    work.reads += after.page_reads - before.page_reads;
    work.writes += after.page_writes - before.page_writes;
    work.lookups += after.reader_lookups - before.reader_lookups;
    Ok(())
}
fn skew(row: u64, countries: u64) -> u64 {
    let majors = countries.min(16);
    let regional = countries.div_ceil(10).max(1);
    let bucket = row.wrapping_mul(7919) % 100;
    1 + if bucket < 60 {
        row % majors
    } else if bucket < 90 {
        row % regional
    } else {
        row % countries
    }
}
const NAMES: [&str; 12] = [
    "repairs ship -> queue rows",
    "repairs/navy force -> ships",
    "repairs base country -> canonical holder",
    "convoys raw holder -> rows",
    "convoys raw partner -> rows",
    "land owners composed slots -> states",
    "navy class -> ships",
    "unknown/recovery cells",
    "visibility",
    "coverage status/count/cadence",
    "damage/restoration status",
    "reread rows",
];

fn main() -> io::Result<()> {
    let wall = Instant::now();
    let o = options()?;
    fs::create_dir_all(&o.out)?;
    let allowance = o.readers_budget / 8;
    let max_cache = allowance
        .checked_sub(1_200_000)
        .ok_or_else(|| io::Error::other("reader share cannot admit metadata and transient work"))?;
    let caches = o.caches.clone().unwrap_or_else(|| {
        vec![
            1_000_000.min(max_cache),
            8_000_000.min(max_cache),
            32_000_000.min(max_cache),
            max_cache,
        ]
    });
    let c = o.countries;
    let scaled = |n| {
        c.checked_mul(n)
            .ok_or_else(|| io::Error::other("row count overflow"))
    };
    let ships = scaled(64)?;
    let forces = scaled(8)?;
    let bases = scaled(4)?;
    let queues = scaled(16)?;
    let convoys = scaled(32)?;
    let states = scaled(8)?;
    let col = |t, f| Column::named(t, f);
    let queue = col(TableId::NavalRepairQueue, "ship")?;
    let force = col(TableId::Ships, "task_force")?;
    let base = col(TableId::NavalBases, "country")?;
    let importer = col(TableId::ResourceExchanges, "importer")?;
    let exporter = col(TableId::ResourceExchanges, "exporter")?;
    let owner = col(TableId::States, "owner")?;
    let slot = col(TableId::CountryLinks, "canonical_slot")?;
    let class = col(TableId::Ships, "class_id")?;
    let zero: &[u8] = &[0];
    let no_owner: &[&[u8]] = &[zero];
    let q = [queue.into()];
    let f = [force.into()];
    let b = [base.into(), slot.into()];
    let im = [importer.into()];
    let ex = [exporter.into()];
    let land = [
        Link {
            column: owner,
            no_values: no_owner,
        },
        slot.into(),
    ];
    let cl = [class.into()];
    let chains: [&[Link<'_>]; 7] = [&q, &f, &b, &im, &ex, &land, &cl];
    let mut report = File::create(o.out.join("report.md"))?;
    writeln!(
        report,
        "# core-indexes\n\nSynthetic, family-shaped workload: {} hours, {c} countries, {ships} ships, {forces} forces, {bases} bases, {queues} repair queue rows (base, ordinal), {convoys} convoy rows, {states} states. Restarts every 2,160 h. Ownership: 60% across up to 16 majors, 30% across a regional tenth, 10% across all countries, with unowned states. Peace-deal hours every 720 h move a third of states and a major link together. Links are shared by repairs and land owners. Shapes are synthetic; they are not recording measurements.",
        o.hours
    )?;
    writeln!(
        report,
        "\nFamily-note feed rates: 82 ship-force cells/h (repairs' 82.4 ship writes/h rounded), 12 class cells/h, 1 link and 1 convoy cell/h, queue/base/state cells daily. Semantic stress: unknown class each 97 h and recovery next hour; visibility each 211 h and recovery next hour; truncated base coverage each 503 h and Complete next hour; lost links at segment hour 1,080, Complete plus full reread next hour. These stress cadences are declared synthetic inputs; recording rates were not supplied. Actual input counts and rates are reported below."
    )?;
    writeln!(
        report,
        "\nBudget: {} readers bytes, one-eighth admission {} bytes. Live index bytes include occupied pages (including node fill), live overflow extents, and actual metadata allocations. In-segment peaks include the active hour working trees; post-compaction bytes exclude those recycled pages. Shared stages are charged to their first relation. The budget share is peak occupied bytes divided by the readers budget. Cache requests and admitted reservations are reported separately; neither is used as that share. Post-compaction occupied bytes are reported separately. The synthetic reader's column allocation is reported separately from indexes. Reader/decode and handler work are outside this engine benchmark; the 150–400 us/h reader projection includes them.",
        o.readers_budget, allowance
    )?;
    if let Some(path) = &o.gate1 {
        writeln!(
            report,
            "\nGate 1 source {}\n\n{}",
            path.display(),
            fs::read_to_string(path)?
        )?;
    } else {
        writeln!(
            report,
            "\nGate 1 baseline was not supplied; no reader-count or complete-reader memory claim is made."
        )?;
    }
    for (run, cache) in caches.iter().copied().enumerate() {
        let cpu_start = clock::cpu_ns()?;
        let cycle_start = clock::cycles()?;
        let memory = Memory {
            allowance,
            cache_bytes: cache,
            metadata_bytes: 65_536,
        };
        let mut work = [Work::default(); NAMES.len()];
        let mut finish = Work::default();
        let mut bootstrap = 0u64;
        let mut maintenance = Work::default();
        let mut compacted_live = 0u64;
        let mut peak = StorageStats::default();
        let mut index_peak = [RelationStats::default(); 7];
        let mut reader_bytes = 0;
        let mut segments = 0u64;
        let mut start = 0;
        let mut unknown_inputs = 0u64;
        let mut visibility_inputs = 0u64;
        let mut status_inputs = 0u64;
        let mut reread_inputs = 0u64;
        let mut dense_inputs = 0u64;
        while start < o.hours {
            let end = (start + 2160).min(o.hours);
            let path = o.out.join(format!("cache-{run}-segment-{segments}"));
            let mut reader = Reader {
                fields: Vec::new(),
                tables: Vec::new(),
            };
            reader.add(slot, c, 0, 1, |row| row)?;
            reader.add(force, ships, 0, 0, |row| (skew(row, c) - 1) * 8 + row % 8)?;
            reader.add(class, ships, 0, 0, |row| row % (c / 4).max(1))?;
            reader.add(queue, queues, bases, 0, |row| row % ships)?;
            reader.add(base, bases, 0, 0, |row| skew(row, c))?;
            reader.add(importer, convoys, 0, 0, |row| skew(row, c))?;
            reader.add(exporter, convoys, 0, 0, |row| {
                skew(row.wrapping_mul(13) + 1, c)
            })?;
            reader.add(owner, states, 0, 0, |row| {
                if row % 37 == 0 { 0 } else { skew(row, c) }
            })?;
            reader_bytes = reader_bytes.max(reader.allocated_bytes());
            let statuses: Vec<_> = reader
                .tables
                .iter()
                .map(|t| TableStatus {
                    table: t.id,
                    state: TableState::Complete,
                    hours: Hours {
                        first: start,
                        last: start,
                    },
                    reason: "synthetic complete restore",
                })
                .collect();
            let mut indexes = Indexes::new(&path, memory, &chains.map(RelationSpec::secondary))?;
            let began = clock::cycles()?;
            indexes.bootstrap(start, &reader, &statuses)?;
            bootstrap += clock::cycles()? - began;
            peak.live_bytes = peak.live_bytes.max(indexes.storage_stats().peak_live_bytes);
            for hour in start..end {
                indexes.begin_hour(hour)?;
                if hour - start == 1080 || hour - start == 1081 {
                    measure(&mut indexes, &mut work[10], |i| {
                        reader.status(
                            i,
                            slot.table,
                            if hour - start == 1080 {
                                TableState::Lost
                            } else {
                                TableState::Complete
                            },
                            hour,
                        )?;
                        status_inputs += 1;
                        Ok(1)
                    })?;
                }
                if hour % 503 <= 3 {
                    measure(&mut indexes, &mut work[9], |i| {
                        reader.status(
                            i,
                            base.table,
                            match hour % 503 {
                                0 => TableState::Truncated { missing: 1 },
                                1 => TableState::Truncated {
                                    missing: 2 + hour % bases,
                                },
                                2 => TableState::ReadInPart,
                                _ => TableState::Complete,
                            },
                            hour,
                        )?;
                        status_inputs += 1;
                        Ok(1)
                    })?;
                }
                if hour - start == 1081 {
                    measure(&mut indexes, &mut work[11], |i| {
                        let n = reader.reread_links(i, slot)?;
                        reread_inputs += n;
                        Ok(n)
                    })?;
                }
                if hour % 211 == 0 || hour % 211 == 1 {
                    measure(&mut indexes, &mut work[8], |i| {
                        reader.visibility(i, base, ((hour / 211) * 7) % bases, hour % 211 == 1)?;
                        visibility_inputs += 1;
                        Ok(1)
                    })?;
                }
                measure(&mut indexes, &mut work[1], |i| {
                    let mut n = 0;
                    for offset in 0..82 {
                        n += reader.change(
                            i,
                            force,
                            (hour * 82 + offset) % ships,
                            Value::known((hour + offset + 1) % forces),
                        )?;
                    }
                    Ok(n)
                })?;
                measure(&mut indexes, &mut work[6], |i| {
                    let mut n = 0;
                    for offset in 0..12 {
                        n += reader.change(
                            i,
                            class,
                            (hour * 12 + offset) % ships,
                            Value::known((hour + offset + 1) % (c / 4).max(1)),
                        )?;
                    }
                    Ok(n)
                })?;
                measure(&mut indexes, &mut work[5], |i| {
                    let mut n = 0;
                    if hour % 720 == 0 {
                        for row in 0..states / 3 {
                            n += reader.change(
                                i,
                                owner,
                                row,
                                Value::known(1 + (hour / 720 + row) % c),
                            )?;
                        }
                        dense_inputs += n;
                    }
                    let link = if hour % 720 == 0 {
                        (hour / 720) % c.min(16)
                    } else {
                        (hour / 24) % c
                    };
                    n += reader.change(i, slot, link, Value::known((hour + 1) % c))?;
                    if hour % 24 == 0 {
                        n += reader.change(
                            i,
                            owner,
                            hour % states,
                            Value::known(1 + (hour + 1) % c),
                        )?;
                    }
                    Ok(n)
                })?;
                let convoy_kind = if hour % 2 == 0 { 3 } else { 4 };
                measure(&mut indexes, &mut work[convoy_kind], |i| {
                    reader.change(
                        i,
                        if convoy_kind == 3 { importer } else { exporter },
                        hour % convoys,
                        Value::known(1 + (hour + 2) % c),
                    )
                })?;
                if hour % 24 == 0 {
                    measure(&mut indexes, &mut work[0], |i| {
                        reader.change(i, queue, hour % queues, Value::known((hour + 1) % ships))
                    })?;
                    measure(&mut indexes, &mut work[2], |i| {
                        reader.change(i, base, hour % bases, Value::known(1 + (hour + 1) % c))
                    })?;
                }
                if hour % 97 <= 1 {
                    measure(&mut indexes, &mut work[7], |i| {
                        let row = ((hour / 97) * 19) % ships;
                        let mut value = Value::known((hour + 3) % (c / 4).max(1));
                        if hour % 97 == 0 {
                            value.missing = Some(Hours {
                                first: hour,
                                last: hour,
                            });
                        }
                        unknown_inputs += 1;
                        reader.change(i, class, row, value)
                    })?;
                }
                measure(&mut indexes, &mut finish, |i| {
                    i.finish_hour(&reader, |_, _| Ok(()))?;
                    Ok(0)
                })?;
                let s = indexes.storage_stats();
                peak.live_bytes = peak.live_bytes.max(s.peak_live_bytes);
                for (index, stats) in index_peak.iter_mut().enumerate() {
                    let current = indexes.relation_stats(index)?;
                    stats.input_edges = stats.input_edges.max(current.input_edges);
                    stats.result_edges = stats.result_edges.max(current.result_edges);
                    stats.owned_entries = stats.owned_entries.max(current.owned_entries);
                    stats.payload_bytes = stats.payload_bytes.max(current.payload_bytes);
                    stats.live_bytes = stats.live_bytes.max(current.live_bytes);
                }
                measure(&mut indexes, &mut maintenance, |i| {
                    i.maintain()?;
                    Ok(0)
                })?;
                peak.resident_bytes = peak.resident_bytes.max(s.resident_bytes);
                peak.backing_bytes = peak.backing_bytes.max(s.peak_backing_bytes);
            }
            measure(&mut indexes, &mut maintenance, |i| {
                i.compact()?;
                Ok(0)
            })?;
            let s = indexes.storage_stats();
            peak.backing_bytes = peak.backing_bytes.max(s.peak_backing_bytes);
            compacted_live = compacted_live.max(s.live_bytes);
            peak.metadata_bytes = peak.metadata_bytes.max(s.metadata_bytes);
            peak.compactions += s.compactions;
            for (index, stats) in index_peak.iter_mut().enumerate() {
                let current = indexes.relation_stats(index)?;
                stats.input_edges = stats.input_edges.max(current.input_edges);
                stats.result_edges = stats.result_edges.max(current.result_edges);
                stats.owned_entries = stats.owned_entries.max(current.owned_entries);
                stats.payload_bytes = stats.payload_bytes.max(current.payload_bytes);
                stats.live_bytes = stats.live_bytes.max(current.live_bytes);
            }
            start = end;
            segments += 1;
        }
        while clock::cpu_ns()? - cpu_start < 5_000_000_000 {
            let mut n = 1u64;
            for _ in 0..100_000 {
                n = std::hint::black_box(n.wrapping_mul(6364136223846793005).wrapping_add(1));
            }
            std::hint::black_box(n);
        }
        let cpu_ns = clock::cpu_ns()? - cpu_start;
        let cycles = clock::cycles()? - cycle_start;
        let ns_per_cycle = cpu_ns as f64 / cycles as f64;
        let input_count: u64 = work.iter().map(|w| w.inputs).sum();
        let update_cycles: u64 = work.iter().map(|w| w.cycles).sum();
        let operations: u64 = work.iter().map(|w| w.operations).sum::<u64>() + finish.operations;
        let decodes: u64 = work.iter().map(|w| w.decodes).sum::<u64>() + finish.decodes;
        let reads: u64 = work.iter().map(|w| w.reads).sum::<u64>() + finish.reads;
        let lookups: u64 = work.iter().map(|w| w.lookups).sum::<u64>() + finish.lookups;
        let changed_cells = work[..7].iter().map(|w| w.inputs).sum::<u64>() + unknown_inputs;
        let live = peak.live_bytes + peak.metadata_bytes as u64;
        writeln!(
            report,
            "\n## Cache request {cache} bytes\n\nMeasured in-segment peak occupied index bytes {live}; {:.6}% of readers budget. Synthetic reader columns/keys allocate {reader_bytes} bytes; combined full-residency data share {:.6}%. Index/cache/metadata/transient admission reservation {} bytes of {allowance}. Post-compaction occupied bytes {}. Scratch high-water {} bytes; compactions {}. These are indexes plus the synthetic adapter, not a count of admissible complete readers.",
            live as f64 * 100.0 / o.readers_budget as f64,
            (live + reader_bytes as u64) as f64 * 100.0 / o.readers_budget as f64,
            peak.resident_bytes,
            compacted_live + peak.metadata_bytes as u64,
            peak.backing_bytes,
            peak.compactions
        )?;
        writeln!(
            report,
            "\nClock: QueryThreadCycleTime around each hour's input-kind batch and the hour's net settlement. ns estimates use a {:.3} s whole-run GetThreadTimes/cycle calibration ({ns_per_cycle:.6} ns/cycle, worst two-tick 31.25 ms quantisation {:.3}%). Calibration-only padding is excluded from every work counter. Counter-call overhead is included. No per-cell process clock is used.",
            cpu_ns as f64 / 1e9,
            31_250_000.0 * 100.0 / cpu_ns as f64
        )?;
        writeln!(
            report,
            "\n| index | input edges | result edges | owned live entries | payload bytes | occupied page + live overflow bytes |\n|---|---:|---:|---:|---:|---:|"
        )?;
        for (name, s) in NAMES.iter().zip(index_peak) {
            writeln!(
                report,
                "| {name} | {} | {} | {} | {} | {} |",
                s.input_edges, s.result_edges, s.owned_entries, s.payload_bytes, s.live_bytes
            )?;
        }
        writeln!(
            report,
            "\n| input kind | inputs | cycles/input | feed ns/input | tree operations/input | page decodes/input | page reads/input | reader cell lookups/input |\n|---|---:|---:|---:|---:|---:|---:|---:|"
        )?;
        for (name, w) in NAMES.iter().zip(work) {
            let denominator = w.inputs.max(1) as f64;
            writeln!(
                report,
                "| {name} | {} | {:.3} | {:.3} | {:.3} | {:.3} | {:.3} | {:.3} |",
                w.inputs,
                w.cycles as f64 / denominator,
                w.cycles as f64 * ns_per_cycle / denominator,
                w.operations as f64 / denominator,
                w.decodes as f64 / denominator,
                w.reads as f64 / denominator,
                w.lookups as f64 / denominator
            )?;
        }
        writeln!(
            report,
            "\nShared net settlement includes composed fan-out and notification delivery: {} cycles, {:.3} s, {} tree operations, {} page decodes. It is not attributed to an individual feed. Total feed + settlement {:.3} cycles/changed cell, {:.3} ns/changed cell; {:.3} tree operations, {:.3} page decodes, {:.3} page reads per changed cell. {} changed cells, {input_count} total inputs. Whole-hour update {:.3} us/h versus the reader's 150–400 us/h including decode. Bootstrap {:.3} us/segment over {segments} segments. Compaction/maintenance is separate: {} cycles, {:.3} s, {} tree operations, {} page decodes, {} reads, {} writes. Reader cell lookups {:.3}/changed cell. Total including compaction {:.3} us/h.",
            finish.cycles,
            finish.cycles as f64 * ns_per_cycle / 1e9,
            finish.operations,
            finish.decodes,
            (update_cycles + finish.cycles) as f64 / changed_cells.max(1) as f64,
            (update_cycles + finish.cycles) as f64 * ns_per_cycle / changed_cells.max(1) as f64,
            operations as f64 / changed_cells.max(1) as f64,
            decodes as f64 / changed_cells.max(1) as f64,
            reads as f64 / changed_cells.max(1) as f64,
            changed_cells,
            (update_cycles + finish.cycles) as f64 * ns_per_cycle / o.hours as f64 / 1000.0,
            bootstrap as f64 * ns_per_cycle / segments as f64 / 1000.0,
            maintenance.cycles,
            maintenance.cycles as f64 * ns_per_cycle / 1e9,
            maintenance.operations,
            maintenance.decodes,
            maintenance.reads,
            maintenance.writes,
            lookups as f64 / changed_cells.max(1) as f64,
            (update_cycles + finish.cycles + maintenance.cycles) as f64 * ns_per_cycle
                / o.hours as f64
                / 1000.0
        )?;
        writeln!(
            report,
            "\nMeasured synthetic traffic: unknown/recovery {unknown_inputs} ({:.6}/h), visibility {visibility_inputs} ({:.6}/h), status {status_inputs} ({:.6}/h), reread rows {reread_inputs} ({:.6}/h), dense owner cells {dense_inputs} ({:.6}/h).",
            unknown_inputs as f64 / o.hours as f64,
            visibility_inputs as f64 / o.hours as f64,
            status_inputs as f64 / o.hours as f64,
            reread_inputs as f64 / o.hours as f64,
            dense_inputs as f64 / o.hours as f64
        )?;
        report.flush()?;
    }
    writeln!(
        report,
        "\nWall {:.3} s. CPU by kind and cache curve above include input application, net fan-out, paging and automatic compaction. No decode, scheduler-utilisation or playback measurement is claimed.",
        wall.elapsed().as_secs_f64()
    )?;
    println!("{}", o.out.join("report.md").display());
    Ok(())
}
