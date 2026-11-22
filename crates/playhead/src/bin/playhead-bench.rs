
#![recursion_limit = "256"]

use playhead::{
    Consumer, Declared, Piece, Playhead, Rebuild, Report, Settings, State, Step, Timeline,
    changes::Changes,
    check::{self, differ, game, held_kinds, lane, samples, step_to},
    lane::{Held, Lane, log},
    pages::{Pages, Span, WorldPages},
};
use replay::campaign::Replay;
use replay_columnar::{
    manifest::Status,
    state::Kind,
    store::{TableState, Value},
    unit::{BLOCK, reader},
};
use replay_data::{BranchId, Hour, rows::TableId};
use serde_json::json;
use std::{
    collections::HashMap,
    io::{self, Write as _},
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

const USAGE: &str = "playhead-bench <open|check|time|seek|play|join> <recording> [--branch=N (open, play, join) | --run=N | --from=<game hour> --hours=N] [--tables=Name,...] [--select=<file>] [--step=24] [--columns=all|numeric] [--untimed] [--each] [--samples=40] [--threads=4] \
    [--speed=1440] [--hz=60] [--step-threads=1] [--fetch-threads=1] [--ahead=4] [--ring=3] [--check] [--seconds=S] [--flat] [--seek=<frames>] [--seeks=40] [--mirror=all|none] [--second=Table,...] [--second-every=30] [--churn=N] [--churn-new=Table,...] [--join=Table,...] [--trials=12]";

fn period(rate: f64) -> io::Result<Duration> {
    playhead::frame_period(rate).ok_or_else(|| {
        invalid(format!(
            "--hz needs a rate of {} to {} frames a second",
            playhead::FRAME_RATES.start(),
            playhead::FRAME_RATES.end()
        ))
    })
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

struct Options {
    tables: Option<Vec<String>>,
    select: Option<String>,
    step: u32,
    numeric: bool,
    samples: usize,
    threads: usize,
    timed: bool,
    each: bool,
    settings: Settings,
    check: bool,
    seconds: Option<f64>,
    flat: bool,
    seek_every: Option<u64>,
    seeks: usize,
    mirror: bool,
    passing: bool,
    second: Option<Vec<String>>,
    second_every: u64,
    branch: Option<BranchId>,
    run: Option<usize>,
    from: Option<Hour>,
    hours: u32,
    pieces: Vec<Piece>,
    window: (u32, u32),
    churn: Option<u64>,
    churn_new: Vec<String>,
    join: Option<Vec<String>>,
    trials: usize,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            tables: None,
            select: None,
            step: 24,
            numeric: false,
            samples: 40,
            threads: 4,
            timed: true,
            each: false,
            settings: Settings::default(),
            check: false,
            seconds: None,
            flat: false,
            seek_every: None,
            seeks: 40,
            mirror: true,
            passing: false,
            second: None,
            second_every: 30,
            branch: None,
            run: None,
            from: None,
            hours: u32::MAX,
            pieces: Vec::new(),
            window: (0, 0),
            churn: None,
            churn_new: Vec::new(),
            join: None,
            trials: 12,
        }
    }
}

fn main() -> io::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [command, recording, rest @ ..] = args.as_slice() else { return Err(invalid(USAGE)) };
    let mut options = Options::default();
    for option in rest {
        match option.as_str() {
            "--untimed" => options.timed = false,
            "--each" => options.each = true,
            "--check" => options.check = true,
            "--flat" => options.flat = true,
            "--passing" => options.passing = true,
            _ => {
                let (name, value) = option.split_once('=').ok_or_else(|| invalid(USAGE))?;
                let number = || value.parse::<f64>().map_err(|_| invalid(USAGE));
                match name {
                    "--tables" => options.tables = Some(value.split(',').map(str::to_owned).collect()),
                    "--select" => options.select = Some(value.to_owned()),
                    "--step" => options.step = number()?.max(1.0) as u32,
                    "--columns" if value == "all" || value == "numeric" => options.numeric = value == "numeric",
                    "--samples" => options.samples = number()? as usize,
                    "--threads" => options.threads = number()?.clamp(1.0, 64.0) as usize,
                    "--speed" => options.settings.speed = number()?,
                    "--hz" => {
                        options.settings.rate = number()?;
                        period(options.settings.rate)?;
                    }
                    "--step-threads" => options.settings.step_threads = number()?.max(1.0) as usize,
                    "--fetch-threads" => options.settings.fetch_threads = number()?.max(1.0) as usize,
                    "--ahead" => options.settings.ahead = number()? as u32,
                    "--ring" => options.settings.ring = number()?.max(1.0) as usize,
                    "--seconds" => options.seconds = Some(number()?),
                    "--seek" => options.seek_every = Some(number()?.max(1.0) as u64),
                    "--seeks" => options.seeks = number()? as usize,
                    "--mirror" if value == "none" || value == "all" => options.mirror = value == "all",
                    "--second" => options.second = Some(value.split(',').map(str::to_owned).collect()),
                    "--second-every" => options.second_every = number()?.max(1.0) as u64,
                    "--branch" => options.branch = Some(check::branch_id(value)?),
                    "--run" => options.run = Some(number()? as usize),
                    "--from" => options.from = Some(number()? as Hour),
                    "--hours" => options.hours = number()?.max(1.0) as u32,
                    "--churn" => options.churn = Some(number()?.max(1.0) as u64),
                    "--churn-new" => options.churn_new = value.split(',').map(str::to_owned).collect(),
                    "--join" => options.join = Some(value.split(',').map(str::to_owned).collect()),
                    "--trials" => options.trials = number()?.max(1.0) as usize,
                    _ => return Err(invalid(USAGE)),
                }
            }
        }
    }
    let span = match (options.run, options.from) {
        (Some(run), None) => Span::Run(run),
        (None, Some(from)) => Span::Hours { from, hours: options.hours },
        (None, None) => Span::Whole,
        (Some(_), Some(_)) => return Err(invalid("--run and --from each name what is read: give one")),
    };
    if options.branch.is_some() && !matches!(command.as_str(), "open" | "play" | "join") {
        return Err(invalid(format!("--branch picks the timeline open, play and join use; {command} reads the first run's own span: give --run or --from instead")));
    }
    let started = Instant::now();
    let replay = Replay::open_timelines(Path::new(recording))?;
    let pages = Arc::new(WorldPages::over(replay.recording().selecting(&TableId::ALL)?, span)?);
    let mut opened = pages.opened();
    opened.ns = started.elapsed().as_nanos() as u64;
    options.pieces = check::played(&replay, &pages, span, options.branch)?.pieces().to_vec();
    let piece = pages.pieces()[0];
    let first_hour = pages.runs()[0].first_hour;
    options.window = ((piece.first - first_hour) as u32, (piece.last - first_hour) as u32);
    let run = &pages.runs()[0];
    let selection = match &options.select {
        Some(path) => Some(read_selection(&pages, path, options.numeric)?),
        None => None,
    };
    for wanted in options.join.iter().flatten().chain(&options.churn_new) {
        available(&pages, wanted)?;
    }
    let mut tables = Vec::new();
    let mut absent = Vec::new();
    for (table, id) in TableId::ALL.iter().enumerate() {
        let name = format!("{id:?}");
        if options.tables.as_ref().is_some_and(|wanted| !wanted.contains(&name)) || selection.as_ref().is_some_and(|held| !held.iter().any(|declared| declared.table == table)) {
            continue;
        }
        if pages.runs().iter().any(|run| !run.pages.iter().any(|page| page.tables.get(table).is_some_and(|entry| entry.first.is_some() || entry.restore.is_some()))) {
            absent.push(name);
            continue;
        }
        tables.push(table);
    }
    let opened_json = json!({ "recording": recording, "span": format!("{span:?}"), "runs": pages.runs().len(), "pieces": options.pieces.iter().map(|piece| json!([piece.run, piece.first, piece.last])).collect::<Vec<_>>(),
        "blocks": opened.blocks, "parts": opened.parts, "frames": opened.frames, "packed": opened.packed, "open_ms": opened.ns as f64 / 1e6, "first_run_hours": run.hours(), "window": options.window, "tables": tables.len(), "without_a_restore": absent });
    eprintln!("{opened_json}");
    let pages: Arc<dyn Pages> = pages;
    match command.as_str() {
        "check" if options.passing => check_passing(&pages, &tables, &options),
        "check" => check(&pages, &tables, &options),
        "time" => time(&pages, &tables, &options, selection.as_deref()),
        "open" => {
            println!("{opened_json}");
            Ok(())
        }
        "seek" => seek(&pages, &tables, &options),
        "play" => {
            let selection = selection.ok_or_else(|| invalid("play needs --select"))?;
            play(&pages, selection.into_iter().filter(|declared| tables.contains(&declared.table)).collect(), &options)
        }
        "join" => {
            let selection = selection.ok_or_else(|| invalid("join needs --select"))?;
            join(&pages, selection.into_iter().filter(|declared| tables.contains(&declared.table)).collect(), &options)
        }
        _ => Err(invalid(USAGE)),
    }
}

fn name(table: usize) -> String {
    format!("{:?}", TableId::ALL[table])
}

fn list(kind: Kind) -> bool {
    matches!(kind, Kind::U32List | Kind::U64List | Kind::Opaque)
}

fn read_selection(pages: &WorldPages, path: &str, numeric: bool) -> io::Result<Vec<Declared>> {
    let text = std::fs::read_to_string(path)?;
    let mut declared = Vec::new();
    for line in text.lines().map(str::trim).filter(|line| !line.is_empty() && !line.starts_with('#')) {
        let (table_name, columns) = line.split_once(':').map_or((line, None), |(table, columns)| (table, Some(columns)));
        let (table, kinds) = available(pages, table_name)?;
        let mut chosen: Vec<usize> = match columns {
            None => (0..kinds.len()).collect(),
            Some("numeric") => (0..kinds.len()).filter(|&column| !list(kinds[column])).collect(),
            Some(names) => names
                .split(',')
                .map(|wanted| {
                    wanted.parse::<usize>().ok().filter(|&column| column < kinds.len()).or_else(|| (0..kinds.len()).find(|&column| TableId::ALL[table].field_name(column) == Some(wanted))).ok_or_else(|| invalid(format!("No column {wanted} in {table_name}")))
                })
                .collect::<io::Result<_>>()?,
        };
        if numeric {
            chosen.retain(|&column| !list(kinds[column]));
        }
        chosen.sort_unstable();
        chosen.dedup();
        declared.push(Declared { table, columns: chosen, keyed: false, passing: false, hourly: false });
    }
    Ok(declared)
}

static UNNAMED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

enum Column {
    Numbers { text: bool, values: Vec<u64>, state: Vec<u8> },
    Lists { cells: Vec<Vec<u8>>, state: Vec<u8> },
}

const HELD: u8 = 0;
const MISSING: u8 = 1;
const UNRECORDED: u8 = 2;

impl Column {
    fn new(kind: Kind) -> Self {
        if list(kind) { Self::Lists { cells: Vec::new(), state: Vec::new() } } else { Self::Numbers { text: kind == Kind::Bytes, values: Vec::new(), state: Vec::new() } }
    }

    fn grow(&mut self, slots: usize) {
        match self {
            Self::Numbers { values, state, .. } if values.len() < slots => {
                values.resize(slots, 0);
                state.resize(slots, UNRECORDED);
            }
            Self::Lists { cells, state } if cells.len() < slots => {
                cells.resize_with(slots, Vec::new);
                state.resize(slots, UNRECORDED);
            }
            _ => {}
        }
    }

    #[inline]
    fn set(&mut self, slot: usize, value: Value<'_>) {
        let (code, number, bytes) = match value {
            Value::Unrecorded => (UNRECORDED, 0, &[][..]),
            Value::Missing => (MISSING, 0, &[][..]),
            Value::Number(bits) => (HELD, bits, &[][..]),
            Value::Text(id) => (HELD, u64::from(id), &[][..]),
            Value::List(bytes) => (HELD, 0, bytes),
        };
        match self {
            Self::Numbers { values, state, .. } => {
                values[slot] = number;
                state[slot] = code;
            }
            Self::Lists { cells, state } => {
                cells[slot].clear();
                cells[slot].extend_from_slice(bytes);
                state[slot] = code;
            }
        }
    }

    fn holds(&self, slot: usize, value: Value<'_>) -> bool {
        match (self, value) {
            (Self::Numbers { state, .. } | Self::Lists { state, .. }, Value::Unrecorded) => state[slot] == UNRECORDED,
            (Self::Numbers { state, .. }, Value::Missing) => state[slot] == MISSING,
            (Self::Numbers { text: false, values, state }, Value::Number(bits)) => state[slot] == HELD && values[slot] == bits,
            (Self::Numbers { text: true, values, state }, Value::Text(id)) => state[slot] == HELD && values[slot] == u64::from(id),
            (Self::Lists { cells, state }, Value::List(bytes)) => state[slot] == HELD && cells[slot] == bytes,
            _ => false,
        }
    }

    fn renumber(&mut self, old: &[u32]) {
        match self {
            Self::Numbers { values, state, .. } => {
                for (new, &from) in old.iter().enumerate() {
                    values[new] = values[from as usize];
                    state[new] = state[from as usize];
                }
                values.truncate(old.len());
                state.truncate(old.len());
            }
            Self::Lists { cells, state } => {
                for (new, &from) in old.iter().enumerate() {
                    cells.swap(new, from as usize);
                    state[new] = state[from as usize];
                }
                cells.truncate(old.len());
                state.truncate(old.len());
            }
        }
    }

    fn bytes(&self) -> usize {
        match self {
            Self::Numbers { values, state, .. } => values.capacity() * 8 + state.capacity(),
            Self::Lists { cells, state } => cells.capacity() * 24 + cells.iter().map(Vec::capacity).sum::<usize>() + state.capacity(),
        }
    }
}

struct Mirror {
    columns: Vec<usize>,
    live: Vec<bool>,
    keys: Vec<u128>,
    cells: Vec<Column>,
    read: Vec<bool>,
    texts: HashMap<u32, Vec<u8>>,
    status: Option<(Status, Option<Hour>)>,
    head: Option<Hour>,
    refused: u64,
}

impl Mirror {
    fn new(kinds: &[Kind], columns: &[usize]) -> Self {
        Self { columns: columns.to_vec(), live: Vec::new(), keys: Vec::new(), cells: columns.iter().map(|&column| Column::new(kinds[column])).collect(), read: vec![true; columns.len()], texts: HashMap::new(), status: None, head: None, refused: 0 }
    }

    fn rebuild(lane: &Lane, kinds: &[Kind], columns: &[usize]) -> Self {
        let mut mirror = Self::new(kinds, columns);
        mirror.read = columns.iter().map(|column| lane.reads().contains(column)).collect();
        let cells = lane.cells();
        mirror.grow(cells.slots() as usize);
        let live: Vec<u32> = cells.rows().collect();
        for &slot in &live {
            mirror.live[slot as usize] = true;
            if lane.keyed() {
                mirror.keys[slot as usize] = cells.key(slot).expect("live");
            }
        }
        for (column, &index) in mirror.cells.iter_mut().zip(columns) {
            let Some(held) = cells.column(index) else { continue };
            match column {
                Column::Numbers { values, state, .. } => {
                    for &slot in &live {
                        (values[slot as usize], state[slot as usize]) = match held.value(slot) {
                            Value::Number(bits) => (bits, HELD),
                            Value::Text(id) => (u64::from(id), HELD),
                            Value::Missing => (0, MISSING),
                            Value::Unrecorded | Value::List(_) => (0, UNRECORDED),
                        };
                    }
                }
                Column::Lists { cells: lists, state } => {
                    for &slot in &live {
                        (lists[slot as usize], state[slot as usize]) = match held.value(slot) {
                            Value::List(bytes) => (bytes.to_vec(), HELD),
                            _ => (Vec::new(), UNRECORDED),
                        };
                    }
                }
            }
        }
        mirror.texts.extend(lane.texts().iter().map(|(id, text)| (id, text.to_vec())));
        mirror.status = Some(lane.status());
        mirror.head = lane.head();
        mirror
    }

    fn grow(&mut self, slots: usize) {
        if self.live.len() < slots {
            self.live.resize(slots, false);
            self.keys.resize(slots, 0);
        }
        self.cells.iter_mut().for_each(|column| column.grow(slots));
    }

    fn apply(&mut self, changes: &Changes) {
        let deleted = changes.deleted();
        for (at, &slot) in deleted.slots().iter().enumerate() {
            let slot = slot as usize;
            self.grow(slot + 1);
            if !self.live[slot] || deleted.keys().is_some_and(|keys| keys[at] != self.keys[slot]) {
                self.refused += 1;
            }
            self.live[slot] = false;
        }
        let inserted = changes.inserted();
        if let Some(&most) = inserted.slots().iter().max() {
            self.grow(most as usize + 1);
        }
        for (at, &slot) in inserted.slots().iter().enumerate() {
            let slot = slot as usize;
            if self.live[slot] {
                self.refused += 1;
            }
            self.live[slot] = true;
            if let Some(keys) = inserted.keys() {
                self.keys[slot] = keys[at];
            }
        }
        let written = changes.columns();
        for ((column, &index), &read) in self.cells.iter_mut().zip(&self.columns).zip(&self.read) {
            let Ok(at) = written.binary_search_by_key(&index, |held| held.column()) else {
                self.refused += u64::from(read);
                continue;
            };
            let written = &written[at];
            if let Some(&most) = written.slots().iter().max() {
                column.grow(most as usize + 1);
            }
            match column {
                Column::Numbers { values, state, .. } => written.each(|slot, value| {
                    let slot = slot as usize;
                    (values[slot], state[slot]) = match value {
                        Value::Number(bits) => (bits, HELD),
                        Value::Text(id) => (u64::from(id), HELD),
                        Value::Missing => (0, MISSING),
                        Value::Unrecorded | Value::List(_) => (0, UNRECORDED),
                    };
                }),
                Column::Lists { .. } => written.each(|slot, value| column.set(slot as usize, value)),
            }
        }
        if let Some(old) = changes.renumber() {
            let mut live = vec![false; old.len()];
            let mut keys = vec![0; old.len()];
            for (new, &from) in old.iter().enumerate() {
                live[new] = self.live[from as usize];
                keys[new] = self.keys[from as usize];
            }
            (self.live, self.keys) = (live, keys);
            self.cells.iter_mut().for_each(|column| column.renumber(old));
        }
        for (id, text) in changes.texts().iter() {
            self.texts.insert(id, text.to_vec());
        }
        if let Some(status) = changes.status() {
            self.status = Some(status);
        }
        self.head = changes.head();
    }

    fn differ(&self, state: &TableState, status: (Status, Option<Hour>), first: &mut Option<String>) -> u64 {
        let mut differing = 0;
        let slots = (state.slots() as usize).max(self.live.len());
        for slot in 0..slots {
            let live = self.live.get(slot) == Some(&true);
            let held = state.live().get(slot as u32);
            if live != held || (live && self.keys[slot] != state.key(slot as u32).expect("live")) {
                differing += 1;
                first.get_or_insert_with(|| format!("slot {slot}: live {live} against {held}"));
                continue;
            }
            if !live {
                continue;
            }
            for (column, &index) in self.cells.iter().zip(&self.columns) {
                let value = state.value(index, slot as u32);
                if !column.holds(slot, value) {
                    differing += 1;
                    first.get_or_insert_with(|| format!("slot {slot} column {index}: lane {value:?}"));
                }
                if let Value::Text(id) = value
                    && !self.texts.contains_key(&id)
                {
                    UNNAMED.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
        if self.status != Some(status) {
            differing += 1;
            first.get_or_insert_with(|| format!("status: mirror {:?}, lane {status:?}", self.status));
        }
        differing
    }

    fn bytes(&self) -> usize {
        self.live.capacity() + self.keys.capacity() * 16 + self.cells.iter().map(Column::bytes).sum::<usize>() + self.texts.values().map(|text| text.capacity() + 32).sum::<usize>()
    }
}

fn each_table(tables: &[usize], threads: usize, work: impl Fn(usize) -> io::Result<serde_json::Value> + Sync) -> io::Result<Vec<serde_json::Value>> {
    let next = AtomicUsize::new(0);
    let lines = Mutex::new(Vec::new());
    let failed = Mutex::new(None);
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| {
                loop {
                    let at = next.fetch_add(1, Ordering::Relaxed);
                    let Some(&table) = tables.get(at) else { break };
                    match work(table) {
                        Ok(line) => {
                            println!("{line}");
                            lines.lock().expect("lines").push(line);
                        }
                        Err(error) => {
                            failed.lock().expect("failed").get_or_insert_with(|| invalid(format!("{}: {error}", name(table))));
                            break;
                        }
                    }
                }
            });
        }
    });
    if let Some(error) = failed.into_inner().expect("failed") {
        return Err(error);
    }
    Ok(lines.into_inner().expect("lines"))
}

fn check(pages: &Arc<dyn Pages>, tables: &[usize], options: &Options) -> io::Result<()> {
    let started = Instant::now();
    let lines = each_table(tables, options.threads, |table| check_table(pages, table, options))?;
    let sum = |key: &str| lines.iter().map(|line| line[key].as_u64().unwrap_or(0)).sum::<u64>();
    let keys = ["steps", "block_ends", "a_differing", "b_steps", "b_differing", "b_refused", "c_seeks", "c_differing", "d_differing", "renumbers"];
    let mut summary = serde_json::Map::new();
    summary.insert("summary".into(), json!(true));
    summary.insert("step".into(), json!(options.step));
    for key in keys {
        summary.insert(key.into(), json!(sum(key)));
    }
    let passed = ["a_differing", "b_differing", "b_refused", "c_differing", "d_differing"].iter().all(|key| sum(key) == 0);
    summary.insert("passed".into(), json!(passed));
    summary.insert("log_tables".into(), json!(lines.iter().filter(|line| line["log"] == true).count()));
    summary.insert("texts_unnamed".into(), json!(UNNAMED.load(Ordering::Relaxed)));
    summary.insert("seconds".into(), json!(started.elapsed().as_secs_f64()));
    println!("{}", serde_json::Value::Object(summary));
    Ok(())
}

fn check_table(pages: &Arc<dyn Pages>, table: usize, options: &Options) -> io::Result<serde_json::Value> {
    if log::is_log(&**pages, table)? {
        return check_log_table(pages, table, options);
    }
    let run = &pages.runs()[0];
    let kinds = pages.kinds(table).expect("a schema table");
    let all: Vec<usize> = (0..kinds.len()).collect();
    let thirds: Vec<usize> = all.iter().copied().step_by(3).collect();
    let mut full = lane(pages, table, &all)?;
    let mut third = lane(pages, table, &thirds)?;
    let mut none = lane(pages, table, &[])?;
    let (start, _) = options.window;
    for lane in [&mut full, &mut third, &mut none] {
        lane.seek(game(run, start))?;
    }
    let mut changes = Changes::default();
    let mut mirror = Mirror::rebuild(&full, kinds, &all);
    let sampled = samples(run, table, options.samples, options.window);
    let targets = check::targets(run, options.step, &sampled, options.window);
    let (mut steps, mut block_ends, mut a, mut b, mut seeks, mut c, mut d, mut renumbers) = (0_u64, 0_u64, 0_u64, 0_u64, 0_u64, 0_u64, 0_u64, 0_u64);
    let mut first: Option<String> = None;
    let mut scratch = Changes::default();
    for &target in &targets {
        step_to(&mut full, run, target, &mut changes, |lane, changes| {
            steps += 1;
            mirror.apply(changes);
            renumbers += u64::from(changes.renumber().is_some());
            b += mirror.differ(lane.cells(), lane.status(), &mut first);
        })?;
        step_to(&mut third, run, target, &mut scratch, |_, _| {})?;
        step_to(&mut none, run, target, &mut scratch, |_, _| {})?;
        for (lane, columns) in [(&third, &thirds), (&none, &Vec::new())] {
            d += differ(lane.cells(), full.cells(), columns, &mut first);
            if lane.status() != full.status() {
                d += 1;
            }
        }
        if let Some(block) = check::block_end(run, target) {
            block_ends += 1;
            let (rebuilt, _) = reader::rebuild(pages.extents(0), &run.pages, held_kinds(&**pages, table), table, block, run.first_hour)?;
            a += differ(full.cells(), &rebuilt, &all, &mut first);
            if full.status() != (rebuilt.status(), rebuilt.sampled_at()) {
                a += 1;
                first.get_or_insert_with(|| format!("block {block} status: lane {:?}, rebuilt {:?}", full.status(), (rebuilt.status(), rebuilt.sampled_at())));
            }
        }
        if sampled.binary_search(&target).is_ok() {
            seeks += 1;
            let mut sought = lane(pages, table, &all)?;
            sought.seek(game(run, target))?;
            c += differ(sought.cells(), full.cells(), &all, &mut first);
            if sought.status() != full.status() || sought.head() != full.head() {
                c += 1;
                first.get_or_insert_with(|| format!("seek to {target}: status {:?} {:?}", sought.status(), full.status()));
            }
            let copy = Mirror::rebuild(&sought, kinds, &all);
            c += copy.differ(full.cells(), full.status(), &mut first);
        }
    }
    Ok(json!({
        "table": name(table), "steps": steps, "block_ends": block_ends, "a_differing": a, "b_steps": steps, "b_differing": b,
        "b_refused": mirror.refused, "c_seeks": seeks, "c_differing": c, "d_differing": d, "renumbers": renumbers,
        "first_difference": first,
    }))
}

fn check_log_table(pages: &Arc<dyn Pages>, table: usize, options: &Options) -> io::Result<serde_json::Value> {
    let checked = check::log_table(pages, table, options.window, options.step, options.samples)?;
    Ok(json!({
        "table": name(table), "log": true, "steps": checked.steps, "block_ends": checked.block_ends, "a_differing": checked.a, "b_steps": checked.steps, "b_differing": checked.b,
        "b_refused": checked.refused, "c_seeks": checked.seeks, "c_differing": checked.c, "d_differing": checked.d, "renumbers": 0, "appended": checked.appended, "forgotten": checked.forgotten,
        "read_rows": checked.read_rows, "narrow_rows": checked.narrow_rows, "first_difference": checked.first,
    }))
}

fn row_cells(cells: &TableState, columns: &[usize], slot: u32) -> Vec<String> {
    columns.iter().map(|&column| format!("{:?}", cells.value(column, slot))).collect()
}

fn check_passing(pages: &Arc<dyn Pages>, tables: &[usize], options: &Options) -> io::Result<()> {
    let started = Instant::now();
    let mut held = Vec::new();
    for &table in tables {
        if log::is_log(&**pages, table)? {
            println!("{}", json!({ "table": name(table), "skipped": "a log: every row it appends passes once, in its steps' appended rows, which check without --passing compares against the reader" }));
        } else {
            held.push(table);
        }
    }
    let lines = each_table(&held, options.threads, |table| check_passing_table(pages, table, options))?;
    let sum = |key: &str| lines.iter().map(|line| line[key].as_u64().unwrap_or(0)).sum::<u64>();
    let passed = sum("differing") == 0 && sum("order") == 0;
    println!(
        "{}",
        json!({ "summary": true, "passing": true, "step": options.step, "tables": lines.len(), "skipped_logs": tables.len() - held.len(), "steps": sum("steps"), "passing_rows": sum("passing_rows"),
            "differing": sum("differing"), "order": sum("order"), "passed": passed, "seconds": started.elapsed().as_secs_f64() })
    );
    Ok(())
}

fn check_passing_table(pages: &Arc<dyn Pages>, table: usize, options: &Options) -> io::Result<serde_json::Value> {
    type Row = (Hour, Hour, u128, Vec<String>);
    let run = &pages.runs()[0];
    let kinds = pages.kinds(table).expect("a schema table");
    let all: Vec<usize> = (0..kinds.len()).collect();
    let mut passing = lane(pages, table, &all)?;
    passing.set_passing(true);
    let mut hourly = lane(pages, table, &all)?;
    let (start, end) = options.window;
    for lane in [&mut passing, &mut hourly] {
        lane.seek(game(run, start))?;
    }
    let mut targets: Vec<u32> = (1..).map(|at| start + at * options.step).take_while(|&hour| hour <= end).collect();
    targets.push(end);
    targets.retain(|&hour| hour > start);
    targets.dedup();
    let (mut changes, mut each) = (Changes::default(), Changes::default());
    let (mut steps, mut rows, mut differing, mut order) = (0_u64, 0_u64, 0_u64, 0_u64);
    let mut first: Option<String> = None;
    let mut alive: HashMap<u32, (Hour, u128, Vec<String>)> = HashMap::new();
    for &target in &targets {
        let mut got: Vec<Row> = Vec::new();
        step_to(&mut passing, run, target, &mut changes, |_, changes| {
            steps += 1;
            let held = changes.passing();
            if held.born().windows(2).any(|pair| pair[1] < pair[0]) {
                order += 1;
            }
            for row in 0..held.len() {
                let cell = |column: usize| held.columns().iter().find(|written| written.column() == column).map_or(Value::Unrecorded, |written| written.value(row));
                got.push((held.born()[row], held.last()[row], held.keys().expect("keyed")[row], all.iter().map(|&column| format!("{:?}", cell(column))).collect()));
            }
        })?;
        let head = passing.head().expect("sought");
        let mut wanted: Vec<Row> = Vec::new();
        alive.clear();
        while hourly.head().expect("sought") < head {
            let hour = hourly.head().expect("sought") + 1;
            hourly.fetch_through(hour)?;
            hourly.step(hour, &mut each)?;
            let cells = hourly.cells();
            for &slot in each.deleted().slots() {
                if let Some((born, key, last)) = alive.remove(&slot) {
                    wanted.push((born, hour - 1, key, last));
                }
            }
            for (at, &slot) in each.inserted().slots().iter().enumerate() {
                alive.insert(slot, (each.inserted().hours()[at], each.inserted().keys().expect("keyed")[at], row_cells(cells, &all, slot)));
            }
            for written in each.columns() {
                for &slot in written.slots() {
                    if let Some(held) = alive.get_mut(&slot) {
                        held.2 = row_cells(cells, &all, slot);
                    }
                }
            }
            if let Some(old) = each.renumber() {
                let moved: HashMap<u32, u32> = old.iter().enumerate().map(|(new, &from)| (from, new as u32)).collect();
                alive = alive.into_iter().filter_map(|(slot, held)| moved.get(&slot).map(|&new| (new, held))).collect();
            }
        }
        rows += got.len() as u64;
        got.sort();
        wanted.sort();
        if got != wanted {
            let only_got = got.iter().filter(|row| wanted.binary_search(row).is_err()).count();
            let only_wanted = wanted.iter().filter(|row| got.binary_search(row).is_err()).count();
            differing += (only_got + only_wanted) as u64;
            first.get_or_insert_with(|| format!("to {head}: passing {:?}, stepped hourly {:?}", got.iter().find(|row| wanted.binary_search(row).is_err()), wanted.iter().find(|row| got.binary_search(row).is_err())));
        }
    }
    Ok(json!({ "table": name(table), "steps": steps, "passing_rows": rows, "differing": differing, "order": order, "first_difference": first }))
}

fn percentiles(values: &mut [u64]) -> serde_json::Value {
    values.sort_unstable();
    let at = |share: f64| values.get(((values.len() as f64 * share) as usize).min(values.len().saturating_sub(1))).copied().unwrap_or(0);
    json!({ "count": values.len(), "p50": at(0.5), "p95": at(0.95), "p99": at(0.99), "worst": values.last().copied().unwrap_or(0) })
}

fn columns_of(kinds: &[Kind], numeric: bool, selection: Option<&[Declared]>, table: usize) -> Vec<usize> {
    match selection.and_then(|held| held.iter().find(|declared| declared.table == table)) {
        Some(declared) => declared.columns.clone(),
        None => (0..kinds.len()).filter(|&column| !numeric || !list(kinds[column])).collect(),
    }
}

#[derive(Default)]
struct Timed {
    steps: Vec<u64>,
    boundary: Vec<u64>,
    plain: Vec<u64>,
    boundary_tracks: u64,
    reader_ns: u64,
    numeric_ns: u64,
    list_ns: u64,
    lifetimes_ns: u64,
    numeric_events: u64,
    list_events: u64,
    lives: u64,
    fetch_ns: u64,
    fetches: Vec<u64>,
    tracks: u64,
    peak: Held,
    renumbers: u64,
    bytes: u64,
    passing: u64,
}

fn feed(lane: &mut Lane, through: Hour, mut fetched: impl FnMut(u64, usize)) -> io::Result<()> {
    while let Some(fetch) = lane.want(through) {
        let read = fetch.run()?;
        fetched(read.ns, read.tracks);
        lane.give(read)?;
    }
    Ok(())
}

fn time(pages: &Arc<dyn Pages>, tables: &[usize], options: &Options, selection: Option<&[Declared]>) -> io::Result<()> {
    let run = &pages.runs()[0];
    let mut lanes = Vec::with_capacity(tables.len());
    for &table in tables {
        let kinds = pages.kinds(table).expect("a schema table");
        let mut lane = Lane::new(Arc::clone(pages), 0, table, &columns_of(kinds, options.numeric, selection, table), false)?;
        lane.set_passing(options.passing);
        lane.seek(game(run, options.window.0))?;
        lane.time(options.timed);
        lanes.push((lane, Changes::default(), Timed::default()));
    }
    let (start, end) = options.window;
    let mut targets: Vec<u32> = (1..).map(|at| start + at * options.step).take_while(|&hour| hour <= end).collect();
    if targets.last() != Some(&end) && end > start {
        targets.push(end);
    }
    let mut summed = vec![0_u64; targets.len()];
    let (mut together, mut together_peak) = (Held::default(), Held::default());
    let started = Instant::now();
    let mut from = start + 1;
    for (index, &target) in targets.iter().enumerate() {
        together = Held::default();
        for (lane, changes, timed) in &mut lanes {
            let table = lane.table();
            let mut opened = 0_u64;
            feed(lane, game(run, target), |ns, tracks| {
                timed.fetch_ns += ns;
                timed.fetches.push(ns);
                timed.tracks += tracks as u64;
                opened += tracks as u64;
            })?;
            let before = lane.spent();
            let began = Instant::now();
            let mut head = lane.step(game(run, target), changes)?;
            timed.renumbers += u64::from(changes.renumber().is_some());
            (timed.bytes, timed.passing) = (timed.bytes + changes.payload() as u64, timed.passing + changes.passing().len() as u64);
            while head < game(run, target) {
                let again = lane.step(game(run, target), changes)?;
                (timed.bytes, timed.passing) = (timed.bytes + changes.payload() as u64, timed.passing + changes.passing().len() as u64);
                if again == head {
                    return Err(invalid(format!("Table {} stalls at hour {head}", name(table))));
                }
                head = again;
            }
            let ns = began.elapsed().as_nanos() as u64;
            let spent = lane.spent();
            let numeric = (spent.numeric - before.numeric).as_nanos() as u64;
            let lists = (spent.list - before.list).as_nanos() as u64;
            let lifetimes = (spent.lifetimes - before.lifetimes).as_nanos() as u64;
            timed.numeric_ns += numeric;
            timed.list_ns += lists;
            timed.lifetimes_ns += lifetimes;
            timed.reader_ns += numeric + lists + lifetimes;
            timed.numeric_events += spent.numeric_events - before.numeric_events;
            timed.list_events += spent.list_events - before.list_events;
            timed.lives += spent.lives - before.lives;
            timed.steps.push(ns);
            summed[index] += ns;
            if options.each {
                eprintln!(
                    "{}",
                    json!({ "table": name(table), "hour": target, "ns": ns, "reader_ns": numeric + lists + lifetimes,
                        "numeric_events": spent.numeric_events - before.numeric_events, "lives": spent.lives - before.lives,
                        "slots_written": changes.cells(), "deleted": changes.deleted().len(), "inserted": changes.inserted().len(), "texts": changes.texts().len(), "fetched_tracks": opened })
                );
            }
            let crossed = run.pages.iter().enumerate().any(|(block, page)| {
                let start = block as u32 * BLOCK;
                page.tables[table].parts.iter().any(|part| (from..=target).contains(&(start + u32::from(part.first)))) || (from..=target).contains(&start)
            });
            if crossed {
                timed.boundary.push(ns);
                timed.boundary_tracks += opened;
            } else {
                timed.plain.push(ns);
            }
            let held = lane.held();
            for (peak, now) in [(&mut timed.peak.cells, held.cells), (&mut timed.peak.parts, held.parts), (&mut timed.peak.part_bytes, held.part_bytes), (&mut timed.peak.tracks, held.tracks), (&mut timed.peak.track_bytes, held.track_bytes)] {
                *peak = (*peak).max(now);
            }
            together.add(&held);
            let mut retired = Vec::new();
            lane.retire_into(&mut retired);
        }
        if together.total() > together_peak.total() {
            together_peak = together;
        }
        from = target + 1;
    }
    let seconds = started.elapsed().as_secs_f64();
    let per = |ns: u64, count: u64| ns as f64 / count.max(1) as f64;
    let mut out = io::BufWriter::new(io::stdout().lock());
    let mut total = Timed::default();
    let mut worst_over_p99 = 0_u64;
    for (lane, _, timed) in &mut lanes {
        let worst_at = timed.steps.iter().enumerate().max_by_key(|&(_, &ns)| ns).map(|(at, _)| targets[at]);
        let steps = percentiles(&mut timed.steps.clone());
        let p99 = steps["p99"].as_u64().unwrap_or(0);
        let worst = steps["worst"].as_u64().unwrap_or(0);
        let spike = worst > 3 * p99.max(1);
        worst_over_p99 += u64::from(spike);
        let step_ns: u64 = timed.steps.iter().sum();
        let mean = |values: &[u64]| values.iter().sum::<u64>() as f64 / values.len().max(1) as f64;
        writeln!(
            out,
            "{}",
            json!({
                "table": name(lane.table()), "columns": lane.requested().len(), "steps_ns": steps, "worst_over_3_p99": spike, "worst_ends_at_hour": worst_at,
                "boundary_steps": timed.boundary.len(), "boundary_mean_ns": mean(&timed.boundary), "plain_mean_ns": mean(&timed.plain),
                "boundary_tracks_opened": timed.boundary_tracks,
                "reader_ns": timed.reader_ns, "lane_ns": step_ns.saturating_sub(timed.reader_ns),
                "numeric": { "events": timed.numeric_events, "ns": timed.numeric_ns }, "list": { "events": timed.list_events, "ns": timed.list_ns },
                "lifetimes": { "lives": timed.lives, "ns": timed.lifetimes_ns },
                "fetch": { "parts": timed.fetches.len(), "ns_per_part": per(timed.fetch_ns, timed.fetches.len() as u64), "tracks": timed.tracks,
                    "ms_per_100k_tracks": timed.fetch_ns as f64 / 1e6 / (timed.tracks.max(1) as f64 / 1e5) },
                "held_peak": { "cells": timed.peak.cells, "parts": timed.peak.parts, "part_bytes": timed.peak.part_bytes, "tracks": timed.peak.tracks, "track_bytes": timed.peak.track_bytes },
                "renumbers": timed.renumbers, "bytes": timed.bytes, "passing_rows": timed.passing,
            })
        )?;
        total.reader_ns += timed.reader_ns;
        total.numeric_ns += timed.numeric_ns;
        total.list_ns += timed.list_ns;
        total.lifetimes_ns += timed.lifetimes_ns;
        total.numeric_events += timed.numeric_events;
        total.list_events += timed.list_events;
        total.lives += timed.lives;
        total.fetch_ns += timed.fetch_ns;
        total.tracks += timed.tracks;
        total.fetches.extend_from_slice(&timed.fetches);
        total.steps.extend(timed.steps.iter().copied());
        total.boundary.extend_from_slice(&timed.boundary);
        total.plain.extend_from_slice(&timed.plain);
        total.bytes += timed.bytes;
        total.passing += timed.passing;
    }
    let step_ns: u64 = total.steps.iter().sum();
    let lane_ns = step_ns.saturating_sub(total.reader_ns);
    let mean = |values: &[u64]| values.iter().sum::<u64>() as f64 / values.len().max(1) as f64;
    writeln!(
        out,
        "{}",
        json!({
            "summary": true, "step": options.step, "columns": if options.numeric { "numeric" } else { "all" }, "tables": lanes.len(), "steps": targets.len(), "seconds": seconds,
            "summed_per_step_ns": percentiles(&mut summed), "tables_worst_over_3_p99": worst_over_p99,
            "boundary_mean_ns": mean(&total.boundary), "plain_mean_ns": mean(&total.plain),
            "reader": { "numeric_ns_per_event": per(total.numeric_ns, total.numeric_events), "list_ns_per_event": per(total.list_ns, total.list_events),
                "ns_per_lifetime": per(total.lifetimes_ns, total.lives), "numeric_events": total.numeric_events, "list_events": total.list_events, "lives": total.lives,
                "ms": total.reader_ns as f64 / 1e6 },
            "lane_on_top": { "ms": lane_ns as f64 / 1e6, "share_of_reader": lane_ns as f64 / total.reader_ns.max(1) as f64 },
            "fetch": { "parts": total.fetches.len(), "per_part_ns": percentiles(&mut total.fetches), "ms": total.fetch_ns as f64 / 1e6,
                "ms_per_100k_tracks": total.fetch_ns as f64 / 1e6 / (total.tracks.max(1) as f64 / 1e5) },
            "held_together_peak": { "cells": together_peak.cells, "parts": together_peak.parts, "part_bytes": together_peak.part_bytes, "tracks": together_peak.tracks, "track_bytes": together_peak.track_bytes,
                "total": together_peak.total() },
            "held_at_end": { "cells": together.cells, "parts": together.parts, "part_bytes": together.part_bytes, "track_bytes": together.track_bytes },
            "passing": options.passing, "bytes": total.bytes, "bytes_per_step": total.bytes as f64 / targets.len().max(1) as f64, "passing_rows": total.passing,
        })
    )?;
    out.flush()
}

fn seek(pages: &Arc<dyn Pages>, tables: &[usize], options: &Options) -> io::Result<()> {
    let run = &pages.runs()[0];
    let mut out = io::BufWriter::new(io::stdout().lock());
    let (mut restores, mut replays, mut over_rule, mut worst_events, mut worst_blocks) = (Vec::new(), Vec::new(), 0_u64, 0_u64, 0_u32);
    for &table in tables {
        let kinds = pages.kinds(table).expect("a schema table");
        let all: Vec<usize> = (0..kinds.len()).collect();
        let (mut restore_ns, mut replay_ns, mut events, mut blocks) = (Vec::new(), Vec::new(), 0_u64, 0_u32);
        let mut fetch_ns = 0_u64;
        for hour in samples(run, table, options.samples, options.window) {
            let mut lane = lane(pages, table, &all)?;
            lane.time(true);
            let sought = lane.seek(game(run, hour))?;
            let replayed = sought.spent.numeric_events + sought.spent.list_events + sought.spent.lives;
            if sought.blocks > 6 && replayed > 500_000 {
                over_rule += 1;
            }
            events = events.max(replayed);
            blocks = blocks.max(sought.blocks);
            restore_ns.push(sought.restore_ns);
            replay_ns.push(sought.replay_ns);
            fetch_ns += sought.fetch_ns;
        }
        worst_events = worst_events.max(events);
        worst_blocks = worst_blocks.max(blocks);
        restores.extend_from_slice(&restore_ns);
        replays.extend_from_slice(&replay_ns);
        writeln!(
            out,
            "{}",
            json!({ "table": name(table), "restore_ns": percentiles(&mut restore_ns), "replay_ns": percentiles(&mut replay_ns), "fetch_ms": fetch_ns as f64 / 1e6,
                "most_events_replayed": events, "most_blocks_replayed": blocks })
        )?;
    }
    writeln!(
        out,
        "{}",
        json!({ "summary": true, "restore_ns": percentiles(&mut restores), "replay_ns": percentiles(&mut replays), "most_events_replayed": worst_events,
            "most_blocks_replayed": worst_blocks, "seeks_over_the_rule": over_rule })
    )?;
    out.flush()
}

struct Frame {
    ns: u64,
    lag: Option<i32>,
    lag_tables: Option<i32>,
    reset: bool,
}

struct SeekRecord {
    hour: Hour,
    called: Instant,
    frame: u64,
    freed_at: u64,
    taken: Option<(u64, f64)>,
    built: Option<playhead::Seeking>,
    swapped: u32,
    swap_ns: u64,
    freed_ms: f64,
    first_step: Option<(i32, f64, bool)>,
    after: u32,
    worst_frame_ns: u64,
}

struct Side {
    consumer: Consumer,
    mirrors: HashMap<usize, Box<Mirror>>,
    compared: u64,
    behind: u64,
    differing: u64,
    unnamed: u64,
    arrived: Vec<(usize, Hour)>,
}

impl Side {
    fn new(consumer: Consumer) -> Self {
        Self { consumer, mirrors: HashMap::new(), compared: 0, behind: 0, differing: 0, unnamed: 0, arrived: Vec::new() }
    }

    fn take_step(&mut self, step: &Step, dropper: &mpsc::Sender<Box<Mirror>>) -> u32 {
        let mut swapped = 0;
        if step.reset() || step.tables().iter().any(Changes::reset) {
            for changes in step.tables().iter().filter(|changes| changes.reset()) {
                let Some(state) = self.consumer.state(step, changes.table()) else { continue };
                swapped += 1;
                self.arrived.push((changes.table(), step.last()));
                if let Ok(mirror) = state.downcast::<Mirror>()
                    && let Some(old) = self.mirrors.insert(changes.table(), mirror)
                {
                    let _ = dropper.send(old);
                }
            }
        }
        for changes in step.tables() {
            if let Some(mirror) = self.mirrors.get_mut(&changes.table()) {
                mirror.apply(changes);
            }
        }
        swapped
    }

    fn compare(&mut self, pages: &dyn Pages, run: usize, block: usize, hour: Hour, threads: usize, first: &mut Option<String>) -> io::Result<()> {
        let before = UNNAMED.load(Ordering::Relaxed);
        let (tables, short, cells, found) = compare_rebuilt(pages, &self.mirrors, run, block, hour, threads)?;
        self.unnamed += UNNAMED.load(Ordering::Relaxed) - before;
        self.compared += tables;
        self.behind += short;
        self.differing += cells;
        if first.is_none() {
            *first = found;
        }
        Ok(())
    }
}

fn block_end(pages: &dyn Pages, timeline: &Timeline, hour: Hour) -> Option<(usize, usize)> {
    let piece = timeline.pieces().iter().find(|piece| piece.first <= hour && hour <= piece.last)?;
    let run = &pages.runs()[piece.run];
    let at = (hour - run.first_hour) as u32;
    let block = (at / BLOCK) as usize;
    (at + 1 == block as u32 * BLOCK + u32::from(run.pages[block].hours)).then_some((piece.run, block))
}

fn compare_rebuilt(pages: &dyn Pages, mirrors: &HashMap<usize, Box<Mirror>>, run: usize, block: usize, hour: Hour, threads: usize) -> io::Result<(u64, u64, u64, Option<String>)> {
    let tables: Vec<usize> = mirrors.iter().filter(|(_, mirror)| mirror.head == Some(hour)).map(|(&table, _)| table).collect();
    let behind = (mirrors.len() - tables.len()) as u64;
    let next = AtomicUsize::new(0);
    let found = Mutex::new((0_u64, None::<String>, None::<io::Error>));
    let held = &pages.runs()[run];
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| {
                while let Some(&table) = tables.get(next.fetch_add(1, Ordering::Relaxed)) {
                    match reader::rebuild(pages.extents(run), &held.pages, held_kinds(pages, table), table, block, held.first_hour) {
                        Ok((rebuilt, texts)) => {
                            let mut first = None;
                            let mirror = &mirrors[&table];
                            let mut differing = mirror.differ(&rebuilt, (rebuilt.status(), rebuilt.sampled_at()), &mut first);
                            for (id, text) in &mirror.texts {
                                if texts.get(id).is_some_and(|held| held != text) {
                                    differing += 1;
                                    first.get_or_insert_with(|| format!("text {id} differs"));
                                }
                            }
                            let mut found = found.lock().expect("found");
                            found.0 += differing;
                            if let Some(first) = first {
                                found.1.get_or_insert_with(|| format!("{} at {hour}: {first}", name(table)));
                            }
                        }
                        Err(error) => {
                            found.lock().expect("found").2.get_or_insert(error);
                        }
                    }
                }
            });
        }
    });
    let (differing, first, error) = found.into_inner().expect("found");
    if let Some(error) = error {
        return Err(error);
    }
    Ok((tables.len() as u64, behind, differing, first))
}

fn mirror_rebuild(pages: &Arc<dyn Pages>, mirror: bool) -> Rebuild {
    if !mirror {
        return Arc::new(|_: &Lane, _: &[usize]| Ok(State::new((), 0)));
    }
    let pages = Arc::clone(pages);
    Arc::new(move |lane: &Lane, columns: &[usize]| {
        let mirror = Mirror::rebuild(lane, pages.kinds(lane.table()).expect("a schema table"), columns);
        let bytes = mirror.bytes();
        Ok(State::new(mirror, bytes))
    })
}

fn second_declared(selection: &[Declared], tables: &[usize], odd: bool, keyed: bool) -> Vec<Declared> {
    tables
        .iter()
        .filter_map(|&table| selection.iter().find(|declared| declared.table == table))
        .map(|declared| {
            let mut columns: Vec<usize> = declared.columns.iter().copied().skip(usize::from(odd)).step_by(2).collect();
            if columns.is_empty() {
                columns.clone_from(&declared.columns);
            }
            Declared { table: declared.table, columns, keyed, passing: false, hourly: false }
        })
        .collect()
}

fn table_named(wanted: &str) -> io::Result<usize> {
    TableId::ALL.iter().position(|id| format!("{id:?}") == wanted).ok_or_else(|| invalid(format!("No table {wanted}")))
}

fn available<'p>(pages: &'p WorldPages, wanted: &str) -> io::Result<(usize, &'p [Kind])> {
    let table = table_named(wanted)?;
    let kinds = pages.kinds(table).ok_or_else(|| invalid(format!("{wanted} is withheld: {}", pages.withheld(table).unwrap_or("the recording holds no such table"))))?;
    Ok((table, kinds))
}

fn every_column(pages: &dyn Pages, table: usize) -> Declared {
    Declared { table, columns: (0..pages.kinds(table).expect("a schema table").len()).collect(), keyed: true, passing: false, hourly: false }
}

fn churn_declared(pages: &dyn Pages, selection: &[Declared], second: &[usize], new: &[usize], change: u64) -> Vec<Declared> {
    let left = (change % 3) as usize;
    let kept: Vec<usize> = second.iter().enumerate().filter(|(at, _)| at % 3 != left).map(|(_, &table)| table).collect();
    let mut declared = second_declared(selection, &kept, change % 2 == 1, true);
    declared.extend(new.iter().enumerate().filter(|(at, _)| (*at as u64 + change).is_multiple_of(2)).map(|(_, &table)| every_column(pages, table)));
    declared.sort_by_key(|declared| declared.table);
    declared
}

fn against_sought(pages: &Arc<dyn Pages>, timeline: &Timeline, table: usize, mirror: &Mirror, first: &mut Option<String>) -> io::Result<u64> {
    let Some(hour) = mirror.head else { return Ok(0) };
    let piece = timeline.pieces().iter().find(|piece| piece.first <= hour && hour <= piece.last).ok_or_else(|| invalid(format!("Hour {hour} is in no piece")))?;
    let mut lane = Lane::new(Arc::clone(pages), piece.run, table, &mirror.columns, true)?;
    lane.seek(hour)?;
    let mut found = None;
    let differing = mirror.differ(lane.cells(), lane.status(), &mut found);
    if let Some(found) = found {
        first.get_or_insert_with(|| format!("{} arrived at {hour}: {found}", name(table)));
    }
    Ok(differing)
}

fn run_frames(playhead: &Playhead, consumer: &mut Consumer, rate: f64, seconds: f64, idle: bool) -> io::Result<()> {
    let frame = period(rate)?;
    let until = Instant::now() + Duration::from_secs_f64(seconds);
    let (mut steps, mut newest, mut quiet) = (Vec::new(), None, 0);
    loop {
        if let Some(error) = playhead.error() {
            return Err(invalid(format!("The playhead stopped: {error}")));
        }
        let head = playhead.head(Instant::now());
        playhead.advance(head);
        consumer.take(head, &mut steps);
        steps.clear();
        quiet = if playhead.newest() == newest { quiet + 1 } else { 0 };
        newest = playhead.newest();
        let reached = newest.is_some_and(|newest| newest >= head || newest >= playhead.timeline().last());
        if Instant::now() >= until && playhead.holding().is_none() && (!idle || (quiet >= 5 && reached)) {
            return Ok(());
        }
        std::thread::sleep(frame);
    }
}

fn join(pages: &Arc<dyn Pages>, selection: Vec<Declared>, options: &Options) -> io::Result<()> {
    let names = options.join.as_ref().ok_or_else(|| invalid("join needs --join"))?;
    let tables: Vec<usize> = names.iter().map(|wanted| table_named(wanted)).collect::<io::Result<_>>()?;
    let declared: Vec<Declared> = tables.iter().map(|&table| every_column(&**pages, table)).collect();
    let timeline = Timeline::new(&**pages, options.pieces.clone())?;
    let settings = options.settings;
    let playhead = Playhead::new(Arc::clone(pages), timeline.clone(), settings, None)?;
    let map = playhead.declare(selection.clone(), Some(mirror_rebuild(pages, options.mirror)))?;
    let mut consumer = playhead.consumer(map);
    let (dropper, dropped) = mpsc::channel::<Box<Mirror>>();
    playhead.play(settings.speed);
    playhead.seek(timeline.first());
    run_frames(&playhead, &mut consumer, settings.rate, 0.5, false)?;
    let mut trials = Vec::new();
    let (mut all_ms, mut each_ms) = (Vec::new(), Vec::new());
    let (mut differing, mut missing, mut first_difference) = (0_u64, 0_usize, None::<String>);
    for trial in 0..options.trials {
        playhead.pause();
        run_frames(&playhead, &mut consumer, settings.rate, 0.0, true)?;
        let head = playhead.head(Instant::now());
        let mut steps = Vec::new();
        let started = Instant::now();
        let reader = playhead.declare(declared.clone(), Some(mirror_rebuild(pages, true)))?;
        let mut side = Side::new(playhead.consumer(reader));
        let mut arrived: Vec<(usize, f64)> = Vec::new();
        while arrived.len() < tables.len() && started.elapsed() < Duration::from_secs(10) {
            if let Some(error) = playhead.error() {
                return Err(invalid(format!("The playhead stopped: {error}")));
            }
            side.consumer.take(head, &mut steps);
            for step in steps.drain(..) {
                side.take_step(&step, &dropper);
            }
            let at = started.elapsed().as_secs_f64() * 1e3;
            for &(table, _) in &side.arrived {
                if !arrived.iter().any(|(held, _)| *held == table) {
                    arrived.push((table, at));
                }
            }
            consumer.take(head, &mut steps);
            steps.clear();
            std::thread::sleep(Duration::from_micros(250));
        }
        let ms = arrived.iter().map(|(_, ms)| *ms).fold(0.0, f64::max);
        missing += tables.len() - arrived.len();
        let mut trial_differing = 0;
        for (&table, mirror) in &side.mirrors {
            trial_differing += against_sought(pages, &timeline, table, mirror, &mut first_difference)?;
        }
        differing += trial_differing;
        if arrived.len() == tables.len() {
            all_ms.push((ms * 1000.0) as u64);
        }
        each_ms.extend(arrived.iter().map(|(_, ms)| (ms * 1000.0) as u64));
        let hours: Vec<Option<Hour>> = tables.iter().map(|table| side.arrived.iter().find(|(held, _)| held == table).map(|(_, hour)| *hour)).collect();
        trials.push(json!({ "trial": trial, "head": head, "hours": hours, "ms": ms, "each_ms": arrived.iter().map(|(table, ms)| json!([name(*table), ms])).collect::<Vec<_>>(), "arrived": arrived.len(), "differing": trial_differing }));
        eprintln!("{}", trials.last().expect("pushed"));
        playhead.retire(reader);
        drop(side);
        playhead.play(settings.speed);
        run_frames(&playhead, &mut consumer, settings.rate, 0.25, false)?;
        if playhead.head(Instant::now()) + (BLOCK as Hour) > timeline.last() {
            playhead.seek(timeline.first());
            run_frames(&playhead, &mut consumer, settings.rate, 0.25, false)?;
        }
    }
    let (_, peak, counters) = playhead.held();
    drop(consumer);
    drop(playhead);
    drop(dropper);
    drop(dropped);
    let ms = |values: &mut Vec<u64>| {
        let held = percentiles(values);
        json!({ "count": held["count"], "p50": held["p50"].as_u64().unwrap_or(0) as f64 / 1000.0, "p99": held["p99"].as_u64().unwrap_or(0) as f64 / 1000.0, "worst": held["worst"].as_u64().unwrap_or(0) as f64 / 1000.0 })
    };
    println!(
        "{}",
        json!({ "summary": true, "join": names, "tables": tables.len(), "first_reader_tables": selection.len(), "step_threads": settings.step_threads, "fetch_threads": settings.fetch_threads, "speed": settings.speed,
            "affinity": std::env::var("AFFINITY").ok(), "every_arrived_ms": ms(&mut all_ms), "each_arrived_ms": ms(&mut each_ms), "never_arrived": missing, "differing": differing, "first_difference": first_difference,
            "texts_unnamed": UNNAMED.load(Ordering::Relaxed), "joins_at_head": counters.joins_at_head, "landings": counters.landings, "peak_total": peak.total, "trials": trials,
            "passed": missing == 0 && differing == 0 })
    );
    Ok(())
}

fn play(pages: &Arc<dyn Pages>, mut selection: Vec<Declared>, options: &Options) -> io::Result<()> {
    let timeline = Timeline::new(&**pages, options.pieces.clone())?;
    let settings = options.settings;
    let mirror = options.mirror || options.check;
    for declared in &mut selection {
        declared.keyed = options.check;
    }
    let columns: usize = selection.iter().map(|declared| declared.columns.len()).sum();
    let playhead = Playhead::new(Arc::clone(pages), timeline.clone(), settings, None)?;
    let first_reader = playhead.declare(selection.clone(), Some(mirror_rebuild(pages, mirror)))?;
    let mut sides = vec![Side::new(playhead.consumer(first_reader))];
    let second_tables: Vec<usize> = match &options.second {
        Some(names) => names
            .iter()
            .map(|wanted| selection.iter().map(|declared| declared.table).find(|&table| name(table) == *wanted).ok_or_else(|| invalid(format!("{wanted} is not in the first reader's selection"))))
            .collect::<io::Result<_>>()?,
        None => Vec::new(),
    };
    let churn_new: Vec<usize> = options.churn_new.iter().map(|wanted| table_named(wanted)).collect::<io::Result<_>>()?;
    if options.churn.is_some() && (second_tables.is_empty() || options.flat) {
        return Err(invalid("--churn needs --second and a clock (no --flat)"));
    }
    let mut churned = churn_declared(&**pages, &selection, &second_tables, &churn_new, 0);
    let mut second = None;
    let mut odd = false;
    if !second_tables.is_empty() {
        let declared = if options.churn.is_some() { churned.clone() } else { second_declared(&selection, &second_tables, odd, options.check) };
        let reader = playhead.declare(declared, Some(mirror_rebuild(pages, mirror)))?;
        sides.push(Side::new(playhead.consumer(reader)));
        second = Some(reader);
    }
    let (mut change, mut since_churn, mut waiting) = (0_u64, 0_u64, None::<(Instant, Vec<usize>)>);
    let (mut changes_by_mode, mut arrivals, mut never_arrived) = ([0_u64; 3], Vec::<u64>::new(), 0_usize);
    let (mut arrival_compared, mut arrival_differing, mut arrival_unnamed) = (0_u64, 0_u64, 0_u64);
    let (dropper, dropped) = mpsc::channel::<Box<Mirror>>();
    let dropping = std::thread::Builder::new().name("bench mirror drop".into()).spawn(move || dropped.into_iter().for_each(drop))?;
    let mut taken: Vec<Arc<Step>> = Vec::new();
    let mut frames: Vec<Frame> = Vec::new();
    let mut steps: Vec<(Hour, Hour, playhead::Built, bool, bool)> = Vec::new();
    let mut held: Vec<(f64, Report)> = Vec::new();
    let mut seeks: Vec<SeekRecord> = Vec::new();
    let mut first_difference = None::<String>;
    let mut table_heads: HashMap<usize, Hour> = HashMap::new();
    let mut applied: Option<Hour> = None;
    let mut epoch = None;
    let mut seed = 0x2545_f491_4f6c_dd1d_u64;
    let frame = period(settings.rate)?;
    let started = Instant::now();
    playhead.play(settings.speed);
    playhead.seek(timeline.first());
    let mut since_reset = 0_u64;
    let (mut since_step_reset, mut after_reset) = (0_u32, Vec::new());
    let mut index = 0_u64;
    let mut sampled_at = started;
    let mut startup: Option<f64> = None;
    let mut second_changes = 0_u64;
    let mut since_second = 0_u64;
    loop {
        if let Some(error) = playhead.error() {
            return Err(invalid(format!("The playhead stopped: {error}")));
        }
        let head = if options.flat {
            match playhead.newest() {
                Some(newest) if Some(newest) != applied => newest,
                newest => {
                    if applied == Some(timeline.last()) {
                        break;
                    }
                    playhead.advance(newest.unwrap_or(timeline.first()));
                    std::thread::sleep(Duration::from_micros(100));
                    if options.seconds.is_some_and(|seconds| started.elapsed().as_secs_f64() > seconds) {
                        break;
                    }
                    continue;
                }
            }
        } else {
            let deadline = started + frame * index as u32;
            let now = Instant::now();
            if deadline > now {
                std::thread::sleep(deadline - now);
            }
            playhead.head(deadline.max(started))
        };
        playhead.advance(head);
        let began = Instant::now();
        let watching = seeks.last().is_some_and(|record| record.after < 3);
        let mut new_reset = false;
        let mut reset_frame = false;
        for (side_at, side) in sides.iter_mut().enumerate() {
            side.consumer.take(head, &mut taken);
            for step in taken.drain(..) {
                let swapping = Instant::now();
                let swapped = side.take_step(&step, &dropper);
                let swap_ns = swapping.elapsed().as_nanos() as u64;
                if side_at == 0 {
                    if step.reset() {
                        reset_frame = true;
                    }
                    if step.reset() && epoch != Some(step.epoch()) {
                        epoch = Some(step.epoch());
                        new_reset = true;
                        if let Some(record) = seeks.last_mut().filter(|record| record.taken.is_none()) {
                            record.taken = Some((index - record.frame, record.called.elapsed().as_secs_f64() * 1e3));
                            record.built = step.built().sought;
                            (record.swapped, record.swap_ns) = (swapped, swap_ns);
                            record.freed_ms = (playhead.freed_ns() - record.freed_at) as f64 / 1e6;
                        }
                        startup.get_or_insert_with(|| started.elapsed().as_secs_f64());
                    } else if !step.reset()
                        && !step.gap()
                        && let Some(record) = seeks.last_mut().filter(|record| record.taken.is_some() && record.first_step.is_none())
                    {
                        record.first_step = Some((step.last() - step.first() + 1, step.built().ns as f64 / 1e6, step.built().stalled > 0));
                    }
                    if let Some(record) = seeks.last_mut().filter(|record| record.taken.is_some()) {
                        record.after += 1;
                    }
                    for changes in step.tables() {
                        if let Some(table_head) = changes.head() {
                            table_heads.insert(changes.table(), table_head);
                        }
                    }
                    applied = Some(step.last());
                    if options.each {
                        eprintln!("{}", json!({ "first": step.first(), "last": step.last(), "ns": step.built().ns, "threads": step.built().threads, "cells": step.built().cells, "rows": step.built().rows, "stalled": step.built().stalled, "reset": step.reset(), "gap": step.gap() }));
                    }
                    steps.push((step.first(), step.last(), *step.built(), step.reset(), step.gap()));
                    since_step_reset = if step.reset() { 0 } else { since_step_reset + 1 };
                    after_reset.push(since_step_reset <= 2);
                }
                if options.check && !step.gap() && let Some((run, block)) = block_end(&**pages, &timeline, step.last()) {
                    side.compare(&**pages, run, block, step.last(), options.threads, &mut first_difference)?;
                }
            }
        }
        let ns = began.elapsed().as_nanos() as u64;
        if let Some((since, expected)) = &waiting {
            let side = &sides[1];
            let done = expected.iter().all(|table| side.arrived.iter().any(|(held, _)| held == table));
            if done || since.elapsed() > Duration::from_secs(10) {
                if done {
                    arrivals.push(since.elapsed().as_micros() as u64);
                } else {
                    never_arrived += expected.iter().filter(|table| !side.arrived.iter().any(|(held, _)| held == *table)).count();
                }
                for &(table, hour) in &side.arrived {
                    if let Some(mirror) = side.mirrors.get(&table).filter(|mirror| mirror.head == Some(hour)) {
                        arrival_compared += 1;
                        let before = UNNAMED.load(Ordering::Relaxed);
                        arrival_differing += against_sought(pages, &timeline, table, mirror, &mut first_difference)?;
                        arrival_unnamed += UNNAMED.load(Ordering::Relaxed) - before;
                    }
                }
                waiting = None;
                playhead.play(settings.speed);
            }
        }
        let lag = applied.map(|applied| head - applied);
        let lag_tables = applied.and(table_heads.values().min().map(|least| head - least));
        let sought = seeks.last().is_some_and(|record| record.taken.is_none());
        if startup.is_some() {
            frames.push(Frame { ns, lag: lag.filter(|_| !sought), lag_tables: lag_tables.filter(|_| !sought), reset: reset_frame });
        }
        if let Some(record) = seeks.last_mut().filter(|_| watching) {
            record.worst_frame_ns = record.worst_frame_ns.max(ns);
        }
        if sampled_at.elapsed() >= Duration::from_millis(250) {
            sampled_at = Instant::now();
            held.push((started.elapsed().as_secs_f64(), playhead.held().0));
        }
        index += 1;
        since_reset += 1;
        since_second += 1;
        since_churn += 1;
        if new_reset {
            since_reset = 0;
        }
        let seeking = seeks.last().is_some_and(|record| record.taken.is_none());
        let mut churn_seek = false;
        if let (Some(every), Some(reader)) = (options.churn, second)
            && startup.is_some()
            && !seeking
            && waiting.is_none()
            && since_churn >= every
        {
            since_churn = 0;
            change += 1;
            let declared = churn_declared(&**pages, &selection, &second_tables, &churn_new, change);
            let mode = (change % 3) as usize;
            changes_by_mode[mode] += 1;
            if mode == 2 {
                seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
                let hour = timeline.first() + ((seed >> 33) % ((timeline.last() - timeline.first()) as u64 + 1)) as Hour;
                seeks.push(SeekRecord { hour, called: Instant::now(), frame: index, freed_at: playhead.freed_ns(), taken: None, built: None, swapped: 0, swap_ns: 0, freed_ms: 0.0, first_step: None, after: 0, worst_frame_ns: 0 });
                playhead.seek(hour);
                churn_seek = true;
            }
            if mode == 0 {
                playhead.pause();
            }
            let expected: Vec<usize> = if mode == 0 && (change / 3) % 2 == 1 {
                playhead.retire(reader);
                let fresh = playhead.declare(declared.clone(), Some(mirror_rebuild(pages, mirror)))?;
                let old = std::mem::replace(&mut sides[1], Side::new(playhead.consumer(fresh)));
                for held in old.mirrors.into_values() {
                    let _ = dropper.send(held);
                }
                second = Some(fresh);
                declared.iter().map(|held| held.table).collect()
            } else {
                playhead.redeclare(reader, declared.clone())?;
                let gone: Vec<usize> = sides[1].mirrors.keys().copied().filter(|table| !declared.iter().any(|held| held.table == *table)).collect();
                for table in gone {
                    if let Some(held) = sides[1].mirrors.remove(&table) {
                        let _ = dropper.send(held);
                    }
                }
                declared.iter().filter(|held| !churned.contains(held)).map(|held| held.table).collect()
            };
            churned = declared;
            sides[1].arrived.clear();
            if mode == 0 {
                waiting = Some((Instant::now(), expected));
            }
        }
        if let Some(reader) = second
            && options.churn.is_none()
            && startup.is_some()
            && !seeking
            && since_second >= options.second_every
        {
            odd = !odd;
            playhead.redeclare(reader, second_declared(&selection, &second_tables, odd, options.check))?;
            second_changes += 1;
            since_second = 0;
        }
        if let Some(every) = options.seek_every
            && !seeking
            && !churn_seek
            && waiting.is_none()
            && since_reset >= every
            && seeks.len() < options.seeks
        {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            let span = (timeline.last() - timeline.first()) as u64 + 1;
            let mut hour = timeline.first() + ((seed >> 33) % span) as Hour;
            if options.check && seeks.len().is_multiple_of(2) {
                let piece = timeline.pieces().iter().find(|piece| piece.first <= hour && hour <= piece.last).copied().unwrap_or(timeline.pieces()[0]);
                let run = &pages.runs()[piece.run];
                hour = (run.first_hour + (((hour - run.first_hour) as u32 / BLOCK + 1) * BLOCK) as Hour - 1).min(piece.last);
            }
            seeks.push(SeekRecord {
                hour,
                called: Instant::now(),
                frame: index,
                freed_at: playhead.freed_ns(),
                taken: None,
                built: None,
                swapped: 0,
                swap_ns: 0,
                freed_ms: 0.0,
                first_step: None,
                after: 0,
                worst_frame_ns: 0,
            });
            playhead.seek(hour);
        }
        let done = applied == Some(timeline.last()) && waiting.is_none() && options.seek_every.is_none_or(|_| seeks.len() >= options.seeks && !seeking);
        if done || options.seconds.is_some_and(|seconds| started.elapsed().as_secs_f64() > seconds) {
            break;
        }
    }
    let wall = started.elapsed().as_secs_f64();
    let (report, peak, counters) = playhead.held();
    let lanes = playhead.finish();
    let mut end_differing = 0;
    if options.check {
        for side in &sides {
            for lane in &lanes {
                if let Some(mirror) = side.mirrors.get(&lane.table()) {
                    let mut first = None;
                    end_differing += mirror.differ(lane.cells(), lane.status(), &mut first);
                    if first_difference.is_none() {
                        first_difference = first.map(|first| format!("{} at the end: {first}", name(lane.table())));
                    }
                }
            }
        }
    }
    let mirror_bytes: usize = sides.iter().flat_map(|side| side.mirrors.values()).map(|mirror| mirror.bytes()).sum();
    let refused: u64 = sides.iter().flat_map(|side| side.mirrors.values()).map(|mirror| mirror.refused).sum();
    let side_json: Vec<serde_json::Value> = sides
        .iter()
        .map(|side| json!({ "tables_compared": side.compared, "tables_behind": side.behind, "differing": side.differing, "texts_unnamed": side.unnamed, "refused": side.mirrors.values().map(|mirror| mirror.refused).sum::<u64>() }))
        .collect();
    let differing: u64 = sides.iter().map(|side| side.differing).sum();
    drop(sides);
    drop(dropper);
    let _ = dropping.join();
    let stepped: Vec<&(Hour, Hour, playhead::Built, bool, bool)> = steps.iter().filter(|(_, _, _, reset, gap)| !reset && !gap).collect();
    let steady: Vec<&(Hour, Hour, playhead::Built, bool, bool)> = steps.iter().zip(&after_reset).filter(|((_, _, _, reset, gap), after)| !reset && !gap && !**after).map(|(step, _)| step).collect();
    let after: Vec<&(Hour, Hour, playhead::Built, bool, bool)> = steps.iter().zip(&after_reset).filter(|((_, _, _, reset, gap), after)| !reset && !gap && **after).map(|(step, _)| step).collect();
    let mut steady_ns: Vec<u64> = steady.iter().map(|(_, _, built, _, _)| built.ns).collect();
    let steady_worst = steady.iter().max_by_key(|(_, _, built, _, _)| built.ns).map(|(first, last, built, _, _)| json!({ "first": first, "last": last, "ms": built.ns as f64 / 1e6, "threads": built.threads, "cells": built.cells, "rows": built.rows }));
    let mut top: Vec<serde_json::Value> = Vec::new();
    let mut ranked: Vec<&&(Hour, Hour, playhead::Built, bool, bool)> = steady.iter().collect();
    ranked.sort_by_key(|(_, _, built, _, _)| std::cmp::Reverse(built.ns));
    for (first, last, built, _, _) in ranked.iter().take(10).map(|step| ***step) {
        top.push(json!([first, last, built.ns as f64 / 1e6, built.threads, built.cells, built.rows]));
    }
    let mut step_ns: Vec<u64> = stepped.iter().map(|(_, _, built, _, _)| built.ns).collect();
    let worst = stepped.iter().max_by_key(|(_, _, built, _, _)| built.ns);
    let mut threads = std::collections::BTreeMap::new();
    for (_, _, built, _, _) in &stepped {
        *threads.entry(built.threads).or_insert(0_u64) += 1;
    }
    let mut fetched = playhead::Fetching::default();
    for (_, _, built, _, _) in &steps {
        fetched.add(&built.fetched);
    }
    let hours_played: i64 = stepped.iter().map(|(first, last, _, _, _)| i64::from(last - first + 1)).sum();
    let mut frame_ns: Vec<u64> = frames.iter().map(|frame| frame.ns).collect();
    let mut reset_frame_ns: Vec<u64> = frames.iter().filter(|frame| frame.reset).map(|frame| frame.ns).collect();
    let mut lags: Vec<u64> = frames.iter().filter_map(|frame| frame.lag).map(|lag| lag.max(0) as u64).collect();
    let mut table_lags: Vec<u64> = frames.iter().filter_map(|frame| frame.lag_tables).map(|lag| lag.max(0) as u64).collect();
    let mut cells: Vec<u64> = stepped.iter().map(|(_, _, built, _, _)| built.cells).collect();
    let mut rows: Vec<u64> = stepped.iter().map(|(_, _, built, _, _)| built.rows).collect();
    let mut bytes: Vec<u64> = stepped.iter().map(|(_, _, built, _, _)| built.bytes).collect();
    let mut seek_ms: Vec<u64> = seeks.iter().filter_map(|record| record.taken).map(|(_, ms)| (ms * 1000.0) as u64).collect();
    let mut first_hours: Vec<u64> = seeks.iter().filter_map(|record| record.first_step).map(|(hours, _, _)| hours.max(0) as u64).collect();
    let mut first_us: Vec<u64> = seeks.iter().filter_map(|record| record.first_step).map(|(_, ms, _)| (ms * 1000.0) as u64).collect();
    let seek_lines: Vec<serde_json::Value> = seeks
        .iter()
        .map(|record| {
            let built = record.built.unwrap_or_default();
            json!({ "hour": record.hour, "frames": record.taken.map(|(frames, _)| frames), "ms": record.taken.map(|(_, ms)| ms), "worst_frame_ms": record.worst_frame_ns as f64 / 1e6,
                "stepper_ms": built.ns as f64 / 1e6, "restore_ms": built.restore_ns as f64 / 1e6, "restore_most_ms": built.restore_most_ns as f64 / 1e6,
                "replay_ms": built.replay_ns as f64 / 1e6, "replay_most_ms": built.replay_most_ns as f64 / 1e6, "fetch_ms": built.fetch_ns as f64 / 1e6,
                "rebuild_ms": built.rebuild_ns as f64 / 1e6, "rebuild_most_ms": built.rebuild_most_ns as f64 / 1e6, "states": built.states, "state_mb": built.state_bytes as f64 / 1e6,
                "table_most_ms": built.table_most_ns as f64 / 1e6, "freed_ms": record.freed_ms, "swapped": record.swapped, "swap_ms": record.swap_ns as f64 / 1e6,
                "first_step": record.first_step.map(|(hours, ms, stalled)| json!({ "hours": hours, "ms": ms, "stalled": stalled })) })
        })
        .collect();
    let report_json = |report: &Report| {
        json!({ "total": report.total, "lanes": report.lanes, "cells": report.held.cells, "parts": report.held.parts, "part_bytes": report.held.part_bytes, "tracks": report.held.tracks,
            "track_bytes": report.held.track_bytes, "ring_steps": report.ring_steps, "ring_bytes": report.ring_bytes, "fetches_queued": report.fetches_queued, "jobs_running": report.jobs_running,
            "preparing": report.preparing, "preparing_held": report.preparing_held.total(), "states": report.states, "state_bytes": report.state_bytes })
    };
    let worst_step = worst.map(|(first, last, built, _, _)| {
        let around: i32 = frames.iter().filter_map(|frame| frame.lag).max().unwrap_or(0);
        json!({ "first": first, "last": last, "ms": built.ns as f64 / 1e6, "threads": built.threads, "cells": built.cells, "rows": built.rows, "bytes": built.bytes, "worst_lag_of_the_run": around })
    });
    let ms = |values: &mut Vec<u64>, scale: f64| {
        let held = percentiles(values);
        json!({ "count": held["count"], "p50": held["p50"].as_u64().unwrap_or(0) as f64 / scale, "p95": held["p95"].as_u64().unwrap_or(0) as f64 / scale,
            "p99": held["p99"].as_u64().unwrap_or(0) as f64 / scale, "worst": held["worst"].as_u64().unwrap_or(0) as f64 / scale })
    };
    let held_series: Vec<serde_json::Value> = held.iter().map(|(at, report)| json!([at, report.total, report.held.cells, report.held.part_bytes, report.held.track_bytes, report.ring_bytes, report.state_bytes, report.preparing_held.total()])).collect();
    let ring_peak = held.iter().map(|(_, report)| report.ring_bytes).max().unwrap_or(0).max(peak.ring_bytes);
    let churn = options.churn.map(|_| {
        json!({ "paused": changes_by_mode[0], "playing": changes_by_mode[1], "at_a_seek": changes_by_mode[2], "arrived_ms": ms(&mut arrivals, 1000.0), "never_arrived": never_arrived,
            "arrivals_compared": arrival_compared, "arrivals_differing": arrival_differing, "arrivals_unnamed": arrival_unnamed, "joins_at_head": counters.joins_at_head, "landings": counters.landings })
    });
    let passed = differing == 0 && end_differing == 0 && refused == 0 && arrival_differing == 0 && arrival_unnamed == 0 && never_arrived == 0;
    println!(
        "{}",
        json!({
            "summary": true, "flat": options.flat, "speed": settings.speed, "hz": settings.rate, "step_threads": settings.step_threads, "fetch_threads": settings.fetch_threads,
            "ahead": settings.ahead, "ring": settings.ring, "mirror": mirror, "freed_ms": peak.freed_ns.max(report.freed_ns) as f64 / 1e6, "tables": selection.len(), "columns": columns, "affinity": std::env::var("AFFINITY").ok(),
            "wall_s": wall, "startup_s": startup, "hours_played": hours_played, "applied_last": applied, "timeline_last": timeline.last(), "seeks_asked": options.seek_every.map(|_| options.seeks),
            "seconds_asked": options.seconds, "game_hours_a_second": hours_played as f64 / (wall - startup.unwrap_or(0.0)).max(1e-9),
            "frames": { "consumer_ms": ms(&mut frame_ns, 1e6), "over_1ms": frames.iter().filter(|frame| frame.ns > 1_000_000).count(), "over_2ms": frames.iter().filter(|frame| frame.ns > 2_000_000).count(),
                "reset_frames_ms": ms(&mut reset_frame_ns, 1e6) },
            "lag_hours": ms(&mut lags, 1.0), "lag_tables_hours": ms(&mut table_lags, 1.0),
            "steps": { "built": steps.len(), "stepped": stepped.len(), "ms": ms(&mut step_ns, 1e6), "threads": threads, "cells": percentiles(&mut cells), "rows": percentiles(&mut rows), "bytes": percentiles(&mut bytes),
                "worst": worst_step, "stalled_steps": stepped.iter().filter(|(_, _, built, _, _)| built.stalled > 0).count(), "stalls": stepped.iter().map(|(_, _, built, _, _)| u64::from(built.stalled)).sum::<u64>(),
                "most_short": stepped.iter().map(|(_, _, built, _, _)| built.most_short).max(), "stepper_busy_share": stepped.iter().map(|(_, _, built, _, _)| built.ns).sum::<u64>() as f64 / 1e9 / wall,
                "stepper_ms": stepped.iter().map(|(_, _, built, _, _)| built.ns).sum::<u64>() as f64 / 1e6,
                "steady": { "ms": ms(&mut steady_ns, 1e6), "worst": steady_worst, "slowest": top, "stalled_steps": steady.iter().filter(|(_, _, built, _, _)| built.stalled > 0).count() },
                "after_reset": { "steps": after.len(), "stalled_steps": after.iter().filter(|(_, _, built, _, _)| built.stalled > 0).count(), "worst_ms": after.iter().map(|(_, _, built, _, _)| built.ns).max().unwrap_or(0) as f64 / 1e6 } },
            "counters": { "steps": counters.steps, "stalls": counters.stalls, "stalled_steps": counters.stalled_steps, "piece_waits": counters.piece_waits, "seeks": counters.seeks, "joins": counters.joins,
                "join_holds": counters.join_holds, "split_steps": counters.split_steps, "second_changes": second_changes },
            "fetch": { "jobs": fetched.jobs, "parts": fetched.parts, "tracks": fetched.tracks, "stored": fetched.stored, "decompressed": fetched.decompressed, "ms": fetched.ns as f64 / 1e6,
                "ms_per_100k_tracks": fetched.ns as f64 / 1e6 / (fetched.tracks.max(1) as f64 / 1e5), "ms_a_second": fetched.ns as f64 / 1e6 / wall, "share_of_a_core": fetched.ns as f64 / 1e9 / wall },
            "held_now": report_json(&report), "held_peak": report_json(&peak), "ring_peak_sampled": ring_peak, "held_series": held_series, "mirror_bytes": mirror_bytes,
            "seeks": { "count": seeks.len(), "ms": ms(&mut seek_ms, 1000.0), "worst_frame_ms": seeks.iter().map(|record| record.worst_frame_ns).max().unwrap_or(0) as f64 / 1e6,
                "first_step_hours": ms(&mut first_hours, 1.0), "first_step_ms": ms(&mut first_us, 1000.0),
                "first_steps_stalled": seeks.iter().filter(|record| record.first_step.is_some_and(|(_, _, stalled)| stalled)).count(), "each": seek_lines },
            "churn": churn,
            "check": options.check.then(|| json!({ "texts_unnamed": UNNAMED.load(Ordering::Relaxed), "readers": side_json, "differing": differing, "end_differing": end_differing,
                "refused": refused, "first_difference": first_difference,
                "passed": passed })),
        })
    );
    Ok(())
}
