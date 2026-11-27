use dashboard_follow::{Answer, Follow, Message, Query, Rows, StepStats, answer_at, decode};
use playhead::{
    Declared, Hook, Playhead, Report, Settings, Timeline,
    lane::Lane,
    pages::{Pages, Span, WorldPages},
};
use replay_columnar::state::Kind;
use replay_data::{Hour, rows::TableId};
use serde_json::json;
use std::{
    collections::{BTreeMap, HashMap},
    io,
    path::Path,
    sync::{Arc, mpsc},
    time::{Duration, Instant},
};

const USAGE: &str = "follow-bench <recording> [--run=N | --from=<game hour> --hours=N] --page=0|7|50|500 [--select=<file> | --occupied-states] [--hook=on|off] [--flat] [--speed=1440] [--hz=60] \
    [--step-threads=2] [--fetch-threads=1] [--seek=<frames>] [--seeks=20] [--seconds=S] [--check] [--service] [--log=<Table>] [--table]";

const OCCUPATION_TABLES: [TableId; 10] = [
    TableId::StateResistance,
    TableId::States,
    TableId::StateGarrisons,
    TableId::GarrisonStrengths,
    TableId::StateAssignedManpower,
    TableId::StateOccupationLaws,
    TableId::OccupationSettings,
    TableId::LexicalTokens,
    TableId::Countries,
    TableId::CountryLinks,
];

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
    run: Option<usize>,
    from: Option<Hour>,
    hours: u32,
    pieces: Vec<playhead::Piece>,
    page: usize,
    select: String,
    hook: bool,
    flat: bool,
    settings: Settings,
    seek_every: Option<u64>,
    seeks: usize,
    seconds: Option<f64>,
    check: bool,
    service: bool,
    log: Option<String>,
    table: bool,
    occupied_states: bool,
}

fn main() -> io::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [recording, rest @ ..] = args.as_slice() else { return Err(invalid(USAGE)) };
    let mut options = Options {
        run: None,
        from: None,
        hours: u32::MAX,
        pieces: Vec::new(),
        page: 7,
        select: String::new(),
        hook: true,
        flat: false,
        settings: Settings { step_threads: 2, fetch_threads: 1, ..Settings::default() },
        seek_every: None,
        seeks: 20,
        seconds: None,
        check: false,
        service: false,
        log: None,
        table: false,
        occupied_states: false,
    };
    for option in rest {
        match option.as_str() {
            "--flat" => options.flat = true,
            "--check" => options.check = true,
            "--service" => options.service = true,
            "--table" => options.table = true,
            "--occupied-states" => options.occupied_states = true,
            _ => {
                let (name, value) = option.split_once('=').ok_or_else(|| invalid(USAGE))?;
                let number = || value.parse::<f64>().map_err(|_| invalid(USAGE));
                match name {
                    "--run" => options.run = Some(number()? as usize),
                    "--from" => options.from = Some(number()? as Hour),
                    "--hours" => options.hours = number()?.max(1.0) as u32,
                    "--page" => options.page = number()? as usize,
                    "--select" => options.select = value.to_owned(),
                    "--hook" if value == "on" || value == "off" => options.hook = value == "on",
                    "--speed" => options.settings.speed = number()?,
                    "--hz" => {
                        options.settings.rate = number()?;
                        period(options.settings.rate)?;
                    }
                    "--step-threads" => options.settings.step_threads = number()?.max(1.0) as usize,
                    "--fetch-threads" => options.settings.fetch_threads = number()?.max(1.0) as usize,
                    "--seek" => options.seek_every = Some(number()?.max(1.0) as u64),
                    "--seeks" => options.seeks = number()? as usize,
                    "--seconds" => options.seconds = Some(number()?),
                    "--log" => options.log = Some(value.to_owned()),
                    _ => return Err(invalid(USAGE)),
                }
            }
        }
    }
    if options.select.is_empty() && !options.occupied_states {
        return Err(invalid(USAGE));
    }
    let span = match (options.run, options.from) {
        (Some(run), None) => Span::Run(run),
        (None, Some(from)) => Span::Hours { from, hours: options.hours },
        (None, None) => Span::Whole,
        (Some(_), Some(_)) => return Err(invalid("--run and --from each name what is read: give one")),
    };
    let opened = WorldPages::open(Path::new(recording), span)?;
    options.pieces = opened.pieces().to_vec();
    let selection = if options.occupied_states { occupied_selection(&opened)? } else { read_selection(&opened, &options.select)? };
    let pages: Arc<dyn Pages> = Arc::new(opened);
    let mut page = if options.occupied_states {
        occupied_page(&pages)?
    } else { match &options.log {
        Some(name) => {
            let table = table_of(name)?;
            let kinds = pages.kinds(table).ok_or_else(|| invalid(format!("{name} is not in the schema")))?;
            vec![Query::Cells { table, columns: (0..kinds.len()).filter(|&column| numeric(kinds[column])).take(3).collect(), rows: Rows::Appended }]
        }
        None => page(options.page, &pages, options.pieces[0].first)?,
    }};
    if options.table && !options.occupied_states {
        page.push(Query::Cells { table: TableId::Countries.index(), columns: vec![dashboard_declare::field(TableId::Countries, "political_power_raw")?], rows: Rows::All });
    }
    let summary = play(&pages, selection, &page, &options)?;
    println!("{summary}");
    Ok(())
}

fn table_of(name: &str) -> io::Result<usize> {
    TableId::ALL.iter().position(|id| format!("{id:?}") == name).ok_or_else(|| invalid(format!("No table {name}")))
}

fn read_selection(pages: &WorldPages, path: &str) -> io::Result<Vec<Declared>> {
    let text = std::fs::read_to_string(path)?;
    let mut declared = Vec::new();
    for line in text.lines().map(str::trim).filter(|line| !line.is_empty() && !line.starts_with('#')) {
        let (name, columns) = line.split_once(':').map_or((line, None), |(table, columns)| (table, Some(columns)));
        let table = table_of(name)?;
        let kinds = pages.kinds(table).ok_or_else(|| invalid(format!("{name} is not in the schema")))?;
        let chosen: Vec<usize> = match columns {
            None => (0..kinds.len()).collect(),
            Some("numeric") => (0..kinds.len()).filter(|&column| numeric(kinds[column])).collect(),
            Some(names) => names
                .split(',')
                .map(|wanted| (0..kinds.len()).find(|&column| TableId::ALL[table].field_name(column) == Some(wanted)).ok_or_else(|| invalid(format!("No column {wanted} in {name}"))))
                .collect::<io::Result<_>>()?,
        };
        declared.push(Declared { table, columns: chosen, keyed: false, passing: false, hourly: false });
    }
    Ok(declared)
}

fn occupied_selection(pages: &WorldPages) -> io::Result<Vec<Declared>> {
    OCCUPATION_TABLES.iter().map(|&table| {
        let table = table.index();
        let kinds = pages.kinds(table).ok_or_else(|| invalid(format!("{:?} is not in the schema", TableId::ALL[table])))?;
        Ok(Declared { table, columns: (0..kinds.len()).collect(), keyed: false, passing: false, hourly: false })
    }).collect()
}

fn occupied_page(pages: &Arc<dyn Pages>) -> io::Result<Vec<Query>> {
    OCCUPATION_TABLES.iter().map(|&table| {
        let index = table.index();
        let kinds = pages.kinds(index).ok_or_else(|| invalid(format!("{table:?} is not in the schema")))?;
        let columns = (0..kinds.len()).collect();
        Ok(Query::Cells { table: index, columns, rows: Rows::All })
    }).collect()
}

fn numeric(kind: Kind) -> bool {
    !matches!(kind, Kind::Bytes | Kind::U32List | Kind::U64List | Kind::Opaque)
}

fn page(size: usize, pages: &Arc<dyn Pages>, hour: Hour) -> io::Result<Vec<Query>> {
    if size == 0 {
        return Ok(Vec::new());
    }
    let countries = TableId::Countries.index();
    let field = |name: &str| dashboard_declare::field(TableId::Countries, name);
    let mut lane = Lane::new(Arc::clone(pages), 0, countries, &[], false)?;
    lane.seek(hour)?;
    let mut keys: Vec<u128> = lane.cells().rows().filter_map(|slot| lane.cells().key(slot)).collect();
    keys.sort_unstable();
    let first = |n: usize| keys.iter().take(n).copied().collect::<Vec<u128>>();
    let cells = |names: &[&str], keys: Vec<u128>| -> io::Result<Query> { Ok(Query::Cells { table: countries, columns: names.iter().map(|name| field(name)).collect::<io::Result<_>>()?, rows: Rows::Keys(keys) }) };
    let top = |name, n| -> io::Result<Query> { Ok(Query::Cells { table: countries, columns: vec![field(name)?], rows: Rows::Top { column: field(name)?, n } }) };
    let measures = ["political_power_raw", "command_power_raw", "stability_raw", "war_support_raw"];
    let mut out = vec![
        cells(&["political_power_raw"], first(8))?,
        cells(&["command_power_raw"], first(8))?,
        cells(&["political_power_raw"], first(64))?,
        cells(&["stability_raw"], first(8))?,
        cells(&["war_support_raw"], first(8))?,
        top("political_power_raw", 10)?,
        cells(&["command_power_raw"], first(64))?,
    ];
    if size <= 7 {
        return Ok(out);
    }
    let combats = TableId::LandCombats.index();
    let kinds = pages.kinds(combats).ok_or_else(|| invalid("No LandCombats in the schema"))?;
    let log_columns: Vec<usize> = (0..kinds.len()).filter(|&column| numeric(kinds[column])).take(3).collect();
    out.push(Query::Cells { table: combats, columns: log_columns, rows: Rows::Appended });
    let sounds = TableId::BattleSounds.index();
    let kinds = pages.kinds(sounds).ok_or_else(|| invalid("No BattleSounds in the schema"))?;
    out.push(Query::Cells { table: sounds, columns: (0..kinds.len()).filter(|&column| numeric(kinds[column])).take(3).collect(), rows: Rows::Appended });
    out.push(Query::Cells { table: countries, columns: vec![field("political_power_raw")?, field("command_power_raw")?], rows: Rows::All });
    for name in ["command_power_raw", "stability_raw", "war_support_raw"] {
        out.push(top(name, 10)?);
    }
    if size >= 500 {
        out.push(top("political_power_raw", usize::MAX)?);
        out.push(top("stability_raw", usize::MAX)?);
    }
    'sets: for n in [8, 16, 32, 64] {
        for measure in measures {
            if out.len() >= size {
                break 'sets;
            }
            out.push(cells(&[measure], first(n))?);
        }
    }
    'single: loop {
        for &key in &keys {
            for measure in measures {
                if out.len() >= size {
                    break 'single;
                }
                out.push(cells(&[measure], vec![key])?);
            }
        }
        if keys.is_empty() {
            break;
        }
    }
    out.truncate(size);
    Ok(out)
}

fn percentiles(values: &mut [u64]) -> serde_json::Value {
    if values.is_empty() {
        return json!(null);
    }
    values.sort_unstable();
    let at = |q: f64| values[((values.len() - 1) as f64 * q).round() as usize];
    json!({ "n": values.len(), "p50": at(0.5), "p99": at(0.99), "worst": values[values.len() - 1] })
}

fn percentiles_ms(values: &mut [u64]) -> serde_json::Value {
    if values.is_empty() {
        return json!(null);
    }
    values.sort_unstable();
    let at = |q: f64| values[((values.len() - 1) as f64 * q).round() as usize] as f64 / 1_000_000.0;
    json!({ "n": values.len(), "p50": at(0.5), "p99": at(0.99), "worst": at(1.0) })
}

fn signed(values: &mut [i64]) -> serde_json::Value {
    if values.is_empty() {
        return json!(null);
    }
    values.sort_unstable();
    let at = |q: f64| values[((values.len() - 1) as f64 * q).round() as usize];
    json!({ "n": values.len(), "p50": at(0.5), "p99": at(0.99), "worst": values[values.len() - 1] })
}

struct StepRecord {
    last: Hour,
    built_ns: u64,
    stalled: u32,
    reset: bool,
    log_head: Option<Hour>,
}

struct Seek {
    hour: Hour,
    called: Instant,
    taken_ms: Option<f64>,
    settled: Option<(u32, u32, bool)>,
}

fn report(held: &Report) -> serde_json::Value {
    json!({ "total": held.total, "cells": held.held.cells, "parts": held.held.part_bytes, "tracks": held.held.track_bytes, "lanes": held.lanes, "ring": held.ring_bytes, "states": held.state_bytes })
}

fn service_page(page: &[Query], table: bool) -> (Vec<serde_json::Value>, Vec<Option<Query>>) {
    let majors: Vec<serde_json::Value> = ["GER", "SOV", "USA", "ENG", "FRA", "ITA", "JAP", "CHI"].iter().map(|tag| json!({ "country": tag, "hour": 0 })).collect();
    let mut requests = vec![
        json!({ "product": "series-state", "params": { "branchId": "1", "family": { "kind": "country-stability", "measure": "stability" }, "reads": majors } }),
        json!({ "product": "army-size", "params": { "branchId": "1", "tags": [], "top": 10, "measure": "divisions", "countFor": "owner", "byType": false } }),
    ];
    let mut parsed: Vec<Option<Query>> = vec![None; requests.len()];
    let direct = page.iter().filter(|query| matches!(query, Query::Cells { rows: Rows::Appended, .. })).chain(page.last().filter(|_| table));
    for query in direct {
        requests.push(serde_json::Value::Null);
        parsed.push(Some(query.clone()));
    }
    (requests, parsed)
}

struct BenchHost {
    follow: Follow,
    held: Vec<bool>,
}

impl dashboard_follow::registry::Hosted for BenchHost {
    fn follow(&self) -> &Follow {
        &self.follow
    }

    fn lacks(&self, table: usize) -> Option<String> {
        dashboard_follow::registry::lacking(table, self.held.get(table).copied().unwrap_or(false))
    }

    fn anchor(&self, _anchor: dashboard_follow::registry::Anchor) {}
}

struct BenchPlayheads(Arc<BenchHost>);

impl dashboard_follow::registry::Playheads for BenchPlayheads {
    fn open(&self, _path: &Path, _branch: u64, _clock: i32, _route: Option<u32>) -> Result<Arc<dyn dashboard_follow::registry::Hosted>, String> {
        Ok(Arc::clone(&self.0) as Arc<dyn dashboard_follow::registry::Hosted>)
    }
}

type Piped = (u32, Instant, Vec<u8>);

fn play(pages: &Arc<dyn Pages>, selection: Vec<Declared>, page: &[Query], options: &Options) -> io::Result<serde_json::Value> {
    let timeline = Timeline::new(&**pages, options.pieces.clone())?;
    let follow = Follow::new(0);
    let (requests, parsed) = if options.service { service_page(page, options.table) } else { (Vec::new(), page.iter().cloned().map(Some).collect()) };
    let owned: Vec<Query> = parsed.iter().map(|query| query.clone().unwrap_or_else(|| page[0].clone())).collect();
    let page = owned.as_slice();
    let (send, received) = mpsc::channel::<(StepStats, Option<Message>)>();
    let (step_send, step_received) = mpsc::channel::<StepRecord>();
    let following = options.hook && !page.is_empty();
    let log_table = options.log.as_ref().map(|_| page[0].table());
    let push = Arc::new(dashboard_follow::push::Push::default());
    let stamps: Arc<std::sync::Mutex<HashMap<u32, Instant>>> = Arc::default();
    let (piped_send, piped) = mpsc::channel::<Piped>();
    let mut pipe_threads = Vec::new();
    if options.service {
        let (mut reader, mut writer) = io::pipe()?;
        let (drained, stamped) = (Arc::clone(&push), Arc::clone(&stamps));
        pipe_threads.push(std::thread::Builder::new().name("bench push writer".into()).spawn(move || {
            while let Some((bytes, sequence, offered)) = drained.take() {
                stamped.lock().unwrap_or_else(std::sync::PoisonError::into_inner).insert(sequence, offered);
                if dashboard_follow::push::write_frame(&mut writer, dashboard_follow::push::FOLLOW, sequence, &bytes).is_err() {
                    return;
                }
            }
        })?);
        pipe_threads.push(std::thread::Builder::new().name("bench push reader".into()).spawn(move || {
            let mut payload = Vec::new();
            while let Ok(Some((_, sequence))) = dashboard_follow::push::read_frame(&mut reader, &mut payload) {
                if piped_send.send((sequence, Instant::now(), std::mem::take(&mut payload))).is_err() {
                    return;
                }
            }
        })?);
    }
    let pushing = Arc::clone(&push);
    let service = options.service;
    let (offer_send, offers) = mpsc::channel::<u64>();
    let mut inner: Option<Hook> = following.then(|| follow.hook(Box::new(move |stats: &StepStats, message: Option<Message>| {
        if service {
            if let Some(message) = message {
                let offering = Instant::now();
                pushing.offer(message.bytes);
                let _ = offer_send.send(offering.elapsed().as_nanos() as u64);
            }
            drop(send.send((*stats, None)));
        } else {
            drop(send.send((*stats, message)));
        }
    })));
    let hook: Hook = Box::new(move |step, lanes| {
        if let Some(inner) = &mut inner {
            inner(step, lanes);
        }
        let built = step.built();
        let log_head = log_table.and_then(|table| lanes.get(table)).and_then(Lane::head);
        drop(step_send.send(StepRecord { last: step.last(), built_ns: built.ns, stalled: built.stalled, reset: step.reset(), log_head }));
    });
    let settings = options.settings;
    let mut unavailable: Vec<Option<(usize, String)>> = Vec::new();
    let mut latency: Vec<u64> = Vec::new();
    let playhead = Arc::new(Playhead::new(Arc::clone(pages), timeline.clone(), settings, Some(hook))?);
    let map = playhead.declare(selection, None)?;
    let mut consumer = playhead.consumer(map);
    if following {
        let reader = playhead.declare(Vec::new(), None)?;
        let weak = Arc::downgrade(&playhead);
        follow.bind(move |tables, _joining| weak.upgrade().map_or(Ok(()), |playhead| playhead.redeclare(reader, tables)), playhead.log_reads())?;
        if service {
            let held = dashboard_follow::host::held_tables(&**pages, TableId::ALL.len());
            let registry = dashboard_follow::registry::Registry::new(Some(Arc::new(BenchPlayheads(Arc::new(BenchHost { follow: follow.clone(), held })))));
            for (id, request) in requests.iter().enumerate() {
                if request.is_null() {
                    follow.follow(id as u32, parsed[id].clone().expect("a log"))?;
                    continue;
                }
                let reply = registry.answer(Path::new("bench"), "follow", &json!({ "id": id, "branchId": "1", "clock": 0, "query": request }), |_| Err(invalid("The query layer reads no format-12 recording yet")), Box::new(|asked| Err(format!("No panel {}", asked["panel"]))));
                unavailable.push(Some((id, match reply {
                    Ok(reply) => reply.get("reason").and_then(serde_json::Value::as_str).unwrap_or("followed").to_owned(),
                    Err(error) => error.to_string(),
                })));
            }
        } else {
            for (id, followed) in page.iter().enumerate() {
                follow.follow(id as u32, followed.clone())?;
            }
        }
    } else if !page.is_empty() {
        let plain = Follow::new(0);
        for (id, followed) in page.iter().enumerate() {
            plain.follow(id as u32, followed.clone())?;
        }
        playhead.declare(plain.declared()?, None)?;
    }
    let quantum = ((settings.speed / settings.rate).round() as Hour).max(1);
    let frame = period(settings.rate)?;
    let mut stats: Vec<StepStats> = Vec::new();
    let mut steps: Vec<StepRecord> = Vec::new();
    let mut ages: Vec<i64> = Vec::new();
    let mut known: Vec<Hour> = Vec::new();
    let mut answers: Vec<(u32, Hour, Vec<u8>)> = Vec::new();
    let mut logged: Vec<(u64, Vec<u8>)> = Vec::new();
    let mut stretch = 0_u64;
    let mut last_answer: HashMap<u32, (Hour, Vec<u8>)> = HashMap::new();
    let mut messages_reset = 0_u64;
    let mut follow_peak = dashboard_follow::Held::default();
    let mut seeks: Vec<Seek> = Vec::new();
    let mut taken = Vec::new();
    let mut applied: Option<Hour> = None;
    let mut epoch = None;
    let mut since_reset = 0_u64;
    let mut seed = 0x2545_f491_4f6c_dd1d_u64;
    let mut sampled_at = Instant::now();
    let started = Instant::now();
    let mut index = 0_u64;
    let mut started_playing = false;
    playhead.play(settings.speed);
    playhead.seek(timeline.first());
    loop {
        if let Some(error) = playhead.error() {
            return Err(invalid(format!("The playhead stopped: {error}")));
        }
        let mut at_end = false;
        let head = if options.flat {
            match playhead.newest() {
                Some(newest) if Some(newest) != applied => newest,
                Some(newest) if applied == Some(timeline.last()) && options.seek_every.is_some() && seeks.len() < options.seeks && seeks.last().is_none_or(|seek| seek.taken_ms.is_some()) => {
                    at_end = true;
                    newest
                }
                newest => {
                    if applied == Some(timeline.last()) && options.seek_every.is_none_or(|_| seeks.len() >= options.seeks) {
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
            playhead.head(Instant::now())
        };
        playhead.advance(head);
        consumer.take(head, &mut taken);
        let mut new_reset = false;
        for step in taken.drain(..) {
            if step.reset() && epoch != Some(step.epoch()) {
                epoch = Some(step.epoch());
                new_reset = true;
                started_playing = true;
                if let Some(seek) = seeks.last_mut().filter(|seek| seek.taken_ms.is_none()) {
                    seek.taken_ms = Some(seek.called.elapsed().as_secs_f64() * 1e3);
                }
            }
            applied = Some(step.last());
        }
        while let Ok(record) = step_received.try_recv() {
            steps.push(record);
        }
        while let Ok((sequence, read, payload)) = piped.try_recv() {
            let offered = stamps.lock().unwrap_or_else(std::sync::PoisonError::into_inner).remove(&sequence);
            if let Some(offered) = offered {
                latency.push(read.saturating_duration_since(offered).as_nanos() as u64);
            }
            let decoded = decode(&payload)?;
            messages_reset += u64::from(decoded.reset);
            stretch += u64::from(decoded.reset);
            for (id, hour, body) in decoded.answers {
                if log_table.is_some() {
                    logged.push((stretch, body.to_vec()));
                }
                if options.check {
                    answers.push((id, hour, body.to_vec()));
                }
                last_answer.insert(id, (hour, body.to_vec()));
            }
        }
        while let Ok((stat, message)) = received.try_recv() {
            if stat.reset {
                known.clear();
                if let Some(seek) = seeks.last_mut().filter(|seek| seek.settled.is_none() && seek.hour == stat.last) {
                    seek.settled = Some((stat.evaluated, stat.sent, message.as_ref().is_some_and(|message| message.reset)));
                }
            }
            known.push(stat.last);
            if let Some(message) = message {
                messages_reset += u64::from(message.reset);
                stretch += u64::from(message.reset);
                let decoded = decode(&message.bytes)?;
                for (id, hour, body) in decoded.answers {
                    if log_table.is_some() {
                        logged.push((stretch, body.to_vec()));
                    }
                    if options.check {
                        answers.push((id, hour, body.to_vec()));
                    }
                    last_answer.insert(id, (hour, body.to_vec()));
                }
            }
            stats.push(stat);
        }
        let seeking = seeks.last().is_some_and(|seek| seek.taken_ms.is_none());
        if following && started_playing && !seeking {
            let newest = known.iter().rev().find(|&&hour| hour <= head).copied();
            if let Some(newest) = newest {
                ages.push(i64::from(head - newest));
            }
        }
        if sampled_at.elapsed() >= Duration::from_millis(100) {
            sampled_at = Instant::now();
            let held = follow.held();
            if held.total > follow_peak.total {
                follow_peak = held;
            }
        }
        index += 1;
        since_reset = if new_reset { 0 } else { since_reset + 1 };
        if let Some(every) = options.seek_every
            && !seeking
            && (since_reset >= every || at_end)
            && seeks.len() < options.seeks
        {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            let span = (timeline.last() - timeline.first()) as u64 + 1;
            let hour = timeline.first() + ((seed >> 33) % span) as Hour;
            seeks.push(Seek { hour, called: Instant::now(), taken_ms: None, settled: None });
            playhead.seek(hour);
        }
        let done = applied == Some(timeline.last()) && options.seek_every.is_none_or(|_| seeks.len() >= options.seeks && !seeking);
        if done || options.seconds.is_some_and(|seconds| started.elapsed().as_secs_f64() > seconds) {
            break;
        }
    }
    let wall = started.elapsed().as_secs_f64();
    let (_, held_peak, counters) = playhead.held();
    let held = follow.held();
    if held.total > follow_peak.total {
        follow_peak = held;
    }
    let follow_counters = follow.counters();
    drop(consumer);
    if let Ok(playhead) = Arc::try_unwrap(playhead) {
        playhead.finish();
    }
    push.stop();
    for thread in pipe_threads {
        let _ = thread.join();
    }
    while let Ok((sequence, read, payload)) = piped.try_recv() {
        if let Some(offered) = stamps.lock().unwrap_or_else(std::sync::PoisonError::into_inner).remove(&sequence) {
            latency.push(read.saturating_duration_since(offered).as_nanos() as u64);
        }
        let decoded = decode(&payload)?;
        stretch += u64::from(decoded.reset);
        for (id, hour, body) in decoded.answers {
            if log_table.is_some() {
                logged.push((stretch, body.to_vec()));
            }
            if options.check {
                answers.push((id, hour, body.to_vec()));
            }
            last_answer.insert(id, (hour, body.to_vec()));
        }
    }
    let push_counters = push.counters();
    let mut offer_ns: Vec<u64> = offers.try_iter().collect();
    let unavailable: Vec<(usize, String)> = unavailable.into_iter().flatten().collect();
    while let Ok(record) = step_received.try_recv() {
        steps.push(record);
    }
    while let Ok((stat, message)) = received.try_recv() {
        if let Some(message) = message {
            for (id, hour, body) in decode(&message.bytes)?.answers {
                if log_table.is_some() {
                    logged.push((stretch, body.to_vec()));
                }
                if options.check {
                    answers.push((id, hour, body.to_vec()));
                }
                last_answer.insert(id, (hour, body.to_vec()));
            }
        }
        stats.push(stat);
    }
    let answers: Vec<(u32, Hour, Vec<u8>)> = answers.into_iter().filter(|(id, ..)| parsed.get(*id as usize).is_some_and(Option::is_some)).collect();
    let last_answer: HashMap<u32, (Hour, Vec<u8>)> = last_answer.into_iter().filter(|(id, _)| parsed.get(*id as usize).is_some_and(Option::is_some)).collect();
    let by_hour: HashMap<Hour, &StepStats> = stats.iter().map(|stat| (stat.last, stat)).collect();
    let mut hook_ns: Vec<u64> = stats.iter().filter(|stat| !stat.reset).map(|stat| stat.ns).collect();
    let mut reset_ns: Vec<u64> = stats.iter().filter(|stat| stat.reset).map(|stat| stat.ns).collect();
    let mut built: Vec<u64> = steps.iter().filter(|step| !step.reset).map(|step| step.built_ns).collect();
    let mut with_hook: Vec<u64> = steps.iter().filter(|step| !step.reset).map(|step| step.built_ns + by_hour.get(&step.last).map_or(0, |stat| stat.ns)).collect();
    let mut bytes: Vec<u64> = stats.iter().filter(|stat| stat.bytes > 0).map(|stat| u64::from(stat.bytes)).collect();
    let mut touched: Vec<u64> = stats.iter().map(|stat| u64::from(stat.touched)).collect();
    let mut evaluated: Vec<u64> = stats.iter().map(|stat| u64::from(stat.evaluated)).collect();
    let mut changed: Vec<u64> = stats.iter().map(|stat| u64::from(stat.changed)).collect();
    let stalled = steps.iter().filter(|step| step.stalled > 0).count();
    let mut ages_steps: Vec<i64> = ages.iter().map(|age| (age + i64::from(quantum) - 1).div_euclid(i64::from(quantum))).collect();
    let checked = match log_table {
        Some(table) if options.check => check_log(pages, table, options.pieces[0], &logged, &steps)?,
        Some(_) => json!(null),
        None if options.check => check(pages, page, &answers, &last_answer)?,
        None => json!(null),
    };
    let mut seek_taken: Vec<u64> = seeks.iter().filter_map(|seek| seek.taken_ms.map(|ms| (ms * 1000.0) as u64)).collect();
    let service = options.service.then(|| json!({
        "queries": parsed.len(), "not_available": unavailable,
        "push_latency_ns": percentiles(&mut latency),
        "push_offer_ns": percentiles(&mut offer_ns),
        "push": { "offered": push_counters.offered, "written": push_counters.written, "superseded": push_counters.superseded, "merges": push_counters.merges,
            "unmerged": push_counters.unmerged, "most_queued": push_counters.most_queued, "bytes": push_counters.bytes, "byte_bound": dashboard_follow::lead::BYTES },
    }));
    let occupied = options.occupied_states.then(|| {
        let measured: Vec<&StepStats> = stats.iter().filter(|stat| !stat.reset && !stat.gap).collect();
        let mut hook: Vec<u64> = measured.iter().map(|stat| stat.ns).collect();
        let bytes: u64 = measured.iter().map(|stat| u64::from(stat.bytes)).sum();
        let stands = measured.iter().filter(|stat| stat.sent == 0).count();
        let data = measured.len().saturating_sub(stands);
        json!({
            "step_hook_ms": percentiles_ms(&mut hook),
            "bytes_per_second": if wall > 0.0 { bytes as f64 / wall } else { 0.0 },
            "stand_steps": stands,
            "data_steps": data,
            "declared_tables": OCCUPATION_TABLES.len(),
            "full_rows": true,
        })
    });
    Ok(json!({
        "page": page.len(), "hook": following, "flat": options.flat, "occupied_states": options.occupied_states, "speed": settings.speed, "hz": settings.rate, "quantum": quantum,
        "step_threads": settings.step_threads, "fetch_threads": settings.fetch_threads, "wall_s": wall,
        "steps": steps.len(), "stalled_steps": stalled, "stalls": counters.stalls, "seeks": seeks.len(),
        "hook_ns": percentiles(&mut hook_ns), "hook_reset_ns": percentiles(&mut reset_ns),
        "stepper_ns": percentiles(&mut built), "stepper_with_hook_ns": percentiles(&mut with_hook),
        "touched": percentiles(&mut touched), "evaluated": percentiles(&mut evaluated), "changed": percentiles(&mut changed),
        "message_bytes": percentiles(&mut bytes),
        "follow": {
            "messages": follow_counters.messages, "sent": follow_counters.sent,
            "messages_reset": messages_reset, "most_ns": follow_counters.most_ns, "evaluated": follow_counters.evaluated, "changed": follow_counters.changed,
        },
        "age_hours": signed(&mut ages), "age_steps": signed(&mut ages_steps),
        "seek_taken_us": percentiles(&mut seek_taken),
        "seeks_settled": seeks.iter().map(|seek| seek.settled.map(|(evaluated, sent, flagged)| json!([evaluated, sent, flagged]))).collect::<Vec<_>>(),
        "held_playhead_peak": report(&held_peak),
        "held_follow_peak": { "queries": follow_peak.queries, "sent": follow_peak.sent, "pending": follow_peak.pending, "queries_bytes": follow_peak.queries_bytes, "index": follow_peak.index, "total": follow_peak.total },
        "check": checked,
        "service": service,
        "occupied": occupied,
    }))
}

fn check_log(pages: &Arc<dyn Pages>, table: usize, piece: playhead::Piece, logged: &[(u64, Vec<u8>)], steps: &[StepRecord]) -> io::Result<serde_json::Value> {
    let (mut pushed, mut out_of_order, mut previous) = (Vec::new(), 0_u64, None::<(u64, Hour)>);
    for (stretch, body) in logged {
        let answer = Answer::parse(body)?;
        let hours = answer.hours.clone().ok_or_else(|| invalid("A log's answer holds no hours"))?;
        for (row, &hour) in hours.iter().enumerate() {
            if previous.is_some_and(|(held, last)| held == *stretch && hour < last) {
                out_of_order += 1;
            }
            previous = Some((*stretch, hour));
            pushed.push((*stretch, hour, answer.keys[row]));
        }
    }
    let (mut played, mut stretch, mut before) = (Vec::new(), 0_u64, None::<Hour>);
    for step in steps {
        let Some(head) = step.log_head else { continue };
        if step.reset {
            stretch += 1;
            before = Some(head);
            continue;
        }
        if let Some(from) = before
            && head > from
        {
            played.push((stretch, from + 1, head));
            before = Some(head);
        }
    }
    let mut lane = Lane::new(Arc::clone(pages), piece.run, table, &[], true)?;
    lane.seek(piece.first)?;
    lane.fetch_through(Hour::MAX)?;
    let mut changes = playhead::changes::Changes::default();
    let mut inserted: Vec<(Hour, u128)> = Vec::new();
    for hour in piece.first + 1..=piece.last {
        lane.step(hour, &mut changes)?;
        let lives = changes.inserted();
        inserted.extend(lives.hours().iter().zip(lives.keys().unwrap_or(&[])).map(|(&hour, &key)| (hour, key)));
    }
    let mut wanted: Vec<(u64, Hour, u128)> = Vec::new();
    for &(stretch, from, to) in &played {
        let at = inserted.partition_point(|&(hour, _)| hour < from);
        wanted.extend(inserted[at..].iter().take_while(|&&(hour, _)| hour <= to).map(|&(hour, key)| (stretch, hour, key)));
    }
    let mut got = pushed.clone();
    wanted.sort_unstable();
    got.sort_unstable();
    let duplicates = got.windows(2).filter(|pair| pair[0] == pair[1]).count();
    let missing = wanted.iter().filter(|row| got.binary_search(row).is_err()).count();
    let extra = got.iter().filter(|row| wanted.binary_search(row).is_err()).count();
    Ok(json!({ "pushed": pushed.len(), "wanted": wanted.len(), "missing": missing, "extra": extra, "duplicates_in_a_stretch": duplicates,
        "out_of_order": out_of_order, "stretches": stretch, "played_spans": played.len(),
        "passed": missing == 0 && extra == 0 && duplicates == 0 && out_of_order == 0 }))
}

fn check(pages: &Arc<dyn Pages>, page: &[Query], answers: &[(u32, Hour, Vec<u8>)], last: &HashMap<u32, (Hour, Vec<u8>)>) -> io::Result<serde_json::Value> {
    let every = (answers.len() / 400).max(1);
    let mut wanted: Vec<(Hour, u32, &[u8])> = answers.iter().step_by(every).map(|(id, hour, body)| (*hour, *id, body.as_slice())).collect();
    wanted.extend(last.iter().map(|(id, (hour, body))| (*hour, *id, body.as_slice())));
    wanted.retain(|(_, id, _)| !matches!(page[*id as usize], Query::Cells { rows: Rows::Appended, .. }));
    wanted.sort_by_key(|(hour, id, _)| (*hour, *id));
    let mut columns: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    let plain = Follow::new(0);
    for (id, query) in page.iter().enumerate() {
        plain.follow(id as u32, query.clone())?;
    }
    for declared in plain.declared()? {
        columns.insert(declared.table, declared.columns);
    }
    let mut lanes: HashMap<(usize, Hour), Lane> = HashMap::new();
    let (mut compared, mut differing, mut first) = (0_u64, 0_u64, None::<String>);
    for (hour, id, body) in wanted {
        let query = &page[id as usize];
        let table = query.table();
        let lane = match lanes.entry((table, hour)) {
            std::collections::hash_map::Entry::Occupied(held) => held.into_mut(),
            std::collections::hash_map::Entry::Vacant(free) => {
                let mut lane = Lane::new(Arc::clone(pages), 0, table, &columns[&table], false)?;
                lane.seek(hour)?;
                free.insert(lane)
            }
        };
        let fresh = answer_at(query, lane)?;
        compared += 1;
        if fresh != body {
            differing += 1;
            if first.is_none() {
                first = Some(format!("query {id} at hour {hour}: pushed {:?}, fresh {:?}", Answer::parse(body)?, Answer::parse(&fresh)?));
            }
        }
        if lanes.len() > 64 {
            lanes.retain(|(_, held), _| *held == hour);
        }
    }
    Ok(json!({ "compared": compared, "differing": differing, "first": first }))
}
