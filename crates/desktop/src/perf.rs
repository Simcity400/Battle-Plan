
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fs::File,
    io::{BufRead, BufReader, BufWriter, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Emitter, Manager};

const BUDGET_MS: f64 = 1000.0 / 95.0;
const PENDING_BYTES: usize = 32 << 20;
const PAGE_WAIT: Duration = Duration::from_secs(5);
const MAP_WAIT: Duration = Duration::from_secs(2);

enum Message {
    Lines(String, usize),
    Flush(Sender<()>),
}

#[derive(Default)]
struct Loss {
    count: u64,
    bytes: u64,
    from: Option<f64>,
    to: Option<f64>,
}

impl Loss {
    fn add(&mut self, count: u64, bytes: u64, from: Option<f64>, to: Option<f64>) {
        self.count += count;
        self.bytes += bytes;
        self.from = [self.from, from].into_iter().flatten().reduce(f64::min);
        self.to = [self.to, to].into_iter().flatten().reduce(f64::max);
    }

    fn merge(&mut self, other: Loss) {
        self.add(other.count, other.bytes, other.from, other.to);
    }

    fn json(&self) -> Value {
        json!({ "count": self.count, "bytes": self.bytes, "from": self.from, "to": self.to })
    }
}

#[derive(Default)]
struct Backlog {
    pending: AtomicUsize,
    lost: AtomicBool,
    loss: Mutex<(Loss, Loss)>,
}

impl Backlog {
    fn dropped(&self, lines: &str) {
        let (mut host, mut page) = (Loss::default(), Loss::default());
        for line in lines.lines() {
            if line.contains(r#""t":"lost""#)
                && let Ok(record) = serde_json::from_str::<Value>(line)
            {
                page.add(
                    record["count"].as_u64().unwrap_or(0),
                    record["bytes"].as_u64().unwrap_or(0),
                    number(&record, "from"),
                    number(&record, "to"),
                );
            } else {
                let at = at_of(line);
                host.add(1, line.len() as u64 + 1, at, at);
            }
        }
        let mut loss = self.loss.lock().unwrap_or_else(|error| error.into_inner());
        loss.0.merge(host);
        loss.1.merge(page);
        self.lost.store(true, Ordering::Release);
    }
}

pub struct PerfLog {
    path: PathBuf,
    map_prefix: PathBuf,
    queue: Sender<Message>,
    backlog: Arc<Backlog>,
    summarized: AtomicBool,
    page_final: Mutex<Option<Sender<bool>>>,
}

static LOG: OnceLock<Option<PerfLog>> = OnceLock::new();

pub fn log() -> Option<&'static PerfLog> {
    LOG.get_or_init(open).as_ref()
}

fn epoch_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |since| since.as_secs_f64() * 1000.0)
}

fn open() -> Option<PerfLog> {
    let path = PathBuf::from(std::env::var_os("BATTLEPLAN_PERF_LOG").or_else(|| std::env::var_os("HOI4_PERF_LOG")).filter(|path| !path.is_empty())?);
    let mut map_prefix = path.clone().into_os_string();
    map_prefix.push(".map");
    let map_prefix = PathBuf::from(map_prefix);
    let file = match File::create(&path) {
        Ok(file) => file,
        Err(error) => {
            eprintln!("BATTLEPLAN_PERF_LOG {}: {error}", path.display());
            return None;
        }
    };
    for stale in map_files(&map_prefix) {
        let _ = std::fs::remove_file(stale);
    }
    let (queue, messages) = mpsc::channel();
    let backlog = Arc::new(Backlog::default());
    let writer_backlog = Arc::clone(&backlog);
    if let Err(error) = std::thread::Builder::new()
        .name("perf log".into())
        .spawn(move || {
            write_records(
                BufWriter::with_capacity(1 << 20, file),
                messages,
                &writer_backlog,
            );
        })
    {
        eprintln!("BATTLEPLAN_PERF_LOG writer: {error}");
        return None;
    }
    let log = PerfLog {
        path,
        map_prefix,
        queue,
        backlog,
        summarized: AtomicBool::new(false),
        page_final: Mutex::new(None),
    };
    log.write(
        json!({ "t": "open", "at": epoch_ms(), "pid": std::process::id(), "budget_ms": BUDGET_MS })
            .to_string(),
    );
    Some(log)
}

fn write_records(mut out: BufWriter<File>, messages: Receiver<Message>, backlog: &Backlog) {
    while let Ok(first) = messages.recv() {
        for message in std::iter::once(first).chain(messages.try_iter()) {
            match message {
                Message::Lines(lines, counted) => {
                    let _ = out.write_all(lines.as_bytes());
                    let _ = out.write_all(b"\n");
                    backlog.pending.fetch_sub(counted, Ordering::AcqRel);
                }
                Message::Flush(done) => {
                    write_loss(&mut out, backlog);
                    let _ = out.flush();
                    let _ = done.send(());
                }
            }
        }
        write_loss(&mut out, backlog);
        let _ = out.flush();
    }
}

fn write_loss(out: &mut impl Write, backlog: &Backlog) {
    if !backlog.lost.swap(false, Ordering::AcqRel) {
        return;
    }
    let (host, page) = std::mem::take(&mut *backlog.loss.lock().unwrap_or_else(|error| error.into_inner()));
    for (source, loss) in [("host", host), ("ui", page)] {
        if loss.count > 0 {
            let mut record = loss.json();
            record["t"] = json!("lost");
            record["at"] = json!(epoch_ms());
            record["source"] = json!(source);
            let _ = writeln!(out, "{record}");
        }
    }
}

fn at_of(line: &str) -> Option<f64> {
    let rest = &line[line.find(r#""at":"#)? + 5..];
    let end = rest.find([',', '}']).unwrap_or(rest.len());
    rest[..end].trim().parse().ok()
}

pub fn init() {
    if log().is_none() {
        return;
    }
    dashboard::service::perf::install(Box::new(|timing: dashboard::service::perf::Timing<'_>| {
        if let Some(log) = log() {
            log.write(
                json!({
                    "t": "srv",
                    "at": epoch_ms(),
                    "id": timing.id,
                    "lane": timing.lane,
                    "outcome": timing.outcome,
                    "queue_ms": timing.queue_ms,
                    "compute_ms": timing.compute_ms,
                    "deliver_ms": timing.deliver_ms,
                    "total_ms": timing.total_ms,
                    "bytes": timing.bytes,
                })
                .to_string(),
            );
        }
    }));
}

pub fn map_prefix() -> Option<&'static Path> {
    log().map(|log| log.map_prefix.as_path())
}

fn map_files(prefix: &Path) -> Vec<PathBuf> {
    let Some(name) = prefix.file_name().and_then(|name| name.to_str()) else {
        return Vec::new();
    };
    let folder = prefix
        .parent()
        .filter(|folder| !folder.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let Ok(entries) = std::fs::read_dir(folder) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .filter(|entry| {
            entry.file_name().to_str().is_some_and(|file| {
                file.strip_prefix(name)
                    .and_then(|rest| rest.strip_prefix('.'))
                    .and_then(|rest| rest.strip_suffix(".jsonl"))
                    .is_some_and(|map| !map.is_empty() && map.bytes().all(|byte| byte.is_ascii_digit()))
            })
        })
        .map(|entry| entry.path())
        .collect()
}

fn map_drained(path: &Path, end: f64) -> bool {
    let Ok(mut file) = File::open(path) else {
        return false;
    };
    let Ok(length) = file.seek(SeekFrom::End(0)) else {
        return false;
    };
    let mut tail = Vec::new();
    if file.seek(SeekFrom::Start(length.saturating_sub(64 * 1024))).is_err()
        || file.read_to_end(&mut tail).is_err()
    {
        return false;
    }
    String::from_utf8_lossy(&tail)
        .lines()
        .rev()
        .find_map(|line| serde_json::from_str::<Value>(line).ok())
        .is_some_and(|record| kind(&record) == "map_close" || at(&record).is_some_and(|at| at >= end))
}

fn wait_for_maps(files: &[PathBuf], end: f64) {
    let deadline = Instant::now() + MAP_WAIT;
    let mut waiting: Vec<&PathBuf> = files.iter().collect();
    loop {
        waiting.retain(|file| !map_drained(file, end));
        if waiting.is_empty() || Instant::now() >= deadline {
            return;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

impl PerfLog {
    fn write(&self, lines: String) {
        let counted = lines.len() + 1;
        if self.backlog.pending.fetch_add(counted, Ordering::AcqRel) + counted > PENDING_BYTES {
            self.backlog.pending.fetch_sub(counted, Ordering::AcqRel);
            self.backlog.dropped(&lines);
            return;
        }
        if self.queue.send(Message::Lines(lines, counted)).is_err() {
            self.backlog.pending.fetch_sub(counted, Ordering::AcqRel);
        }
    }

    fn flush(&self) {
        let (done, written) = mpsc::channel();
        if self.queue.send(Message::Flush(done)).is_ok() {
            let _ = written.recv();
        }
    }

    fn finish_page(&self, app: &AppHandle) -> Option<bool> {
        let window = app.get_webview_window("main")?;
        let (done, finished) = mpsc::channel();
        *self.page_final.lock().unwrap_or_else(|error| error.into_inner()) = Some(done);
        if window.emit("perf-finish", ()).is_err() {
            return Some(false);
        }
        Some(finished.recv_timeout(PAGE_WAIT).unwrap_or(false))
    }

    fn summarize(&self, end: Option<f64>, wait_maps: bool, page_flushed: Option<bool>) -> Result<Value, String> {
        self.flush();
        let records = read(&self.path)?;
        let end = end
            .or_else(|| {
                records
                    .iter()
                    .rev()
                    .find(|record| kind(record) == "stop")
                    .and_then(|record| at(record))
            })
            .unwrap_or_else(epoch_ms);
        let start = records
            .iter()
            .rev()
            .filter(|record| kind(record) == "start")
            .filter_map(at)
            .find(|&start| start <= end)
            .or_else(|| records.iter().filter_map(at).next())
            .unwrap_or(end);
        let files = map_files(&self.map_prefix);
        if wait_maps {
            wait_for_maps(&files, end);
        }
        let map: Vec<Value> = files
            .iter()
            .flat_map(|file| read(file).unwrap_or_default())
            .collect();
        let window = Window { start, end };
        let mut summary = summarize(&window, &records, &map, page_flushed);
        summary["t"] = json!("summary");
        summary["at"] = json!(epoch_ms());
        summary["window"] = json!({ "from": start, "to": end, "seconds": (end - start) / 1000.0 });
        summary["page_flushed"] = json!(page_flushed);
        let _ = self.queue.send(Message::Lines(summary.to_string(), 0));
        self.flush();
        self.summarized.store(true, Ordering::Release);
        Ok(summary)
    }
}

fn read(path: &Path) -> Result<Vec<Value>, String> {
    let file = File::open(path).map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(BufReader::new(file)
        .lines()
        .map_while(Result::ok)
        .filter_map(|line| serde_json::from_str(&line).ok())
        .collect())
}

fn kind(record: &Value) -> &str {
    record["t"].as_str().unwrap_or_default()
}

fn at(record: &Value) -> Option<f64> {
    record["at"].as_f64()
}

fn number(record: &Value, field: &str) -> Option<f64> {
    record[field].as_f64()
}

fn values(records: &[&Value], field: &str) -> Vec<f64> {
    records.iter().filter_map(|record| number(record, field)).collect()
}

fn stats(mut values: Vec<f64>) -> Value {
    if values.is_empty() {
        return json!({ "count": 0 });
    }
    values.sort_by(f64::total_cmp);
    let count = values.len();
    let rank = |p: f64| values[((p * count as f64).ceil() as usize).clamp(1, count) - 1];
    json!({
        "count": count,
        "p50": rank(0.5),
        "p95": rank(0.95),
        "p99": rank(0.99),
        "max": values[count - 1],
        "mean": values.iter().sum::<f64>() / count as f64,
    })
}

fn frame_stats(values: Vec<f64>) -> Value {
    let over = values.iter().filter(|&&value| value > BUDGET_MS).count();
    let count = values.len();
    let mut result = stats(values);
    if count > 0 {
        result["over_budget_share"] = json!(over as f64 / count as f64);
    }
    result
}

fn missed_frames(frames: &[&Value]) -> Value {
    let (mut main_thread, mut after_main, mut unmeasured) = (0u64, 0u64, 0u64);
    for record in frames {
        if !number(record, "dt").is_some_and(|dt| dt > 1.5 * BUDGET_MS) {
            continue;
        }
        match record.get("main") {
            None => unmeasured += 1,
            Some(main) => match main.as_f64() {
                Some(main) if main <= BUDGET_MS => after_main += 1,
                _ => main_thread += 1,
            },
        }
    }
    json!({
        "count": main_thread + after_main + unmeasured,
        "main_thread": main_thread,
        "after_main_thread": after_main,
        "unmeasured": unmeasured,
    })
}

fn widget_work(all: &[Value], end: f64, frames: &[&Value], playing_seconds: Option<f64>) -> Value {
    let mut names: HashMap<&str, &Value> = HashMap::new();
    for record in all.iter().filter(|record| kind(record) == "widget") {
        if at(record).is_some_and(|when| when <= end)
            && let Some(id) = record["id"].as_str()
        {
            names.insert(id, record);
        }
    }
    let mut counts: BTreeMap<&str, BTreeMap<&str, u64>> = BTreeMap::new();
    for record in frames {
        let Some(work) = record["work"].as_object() else {
            continue;
        };
        for (widget, done) in work {
            let Some(done) = done.as_object() else {
                continue;
            };
            let into = counts.entry(widget.as_str()).or_default();
            for (what, count) in done {
                *into.entry(what.as_str()).or_default() += count.as_u64().unwrap_or(0);
            }
        }
    }
    counts
        .into_iter()
        .map(|(widget, done)| {
            let named = names.get(widget);
            let per_second: serde_json::Map<String, Value> = done
                .iter()
                .map(|(what, count)| {
                    let rate = playing_seconds
                        .filter(|seconds| *seconds > 0.0)
                        .map(|seconds| *count as f64 / seconds);
                    ((*what).to_owned(), json!(rate))
                })
                .collect();
            (
                widget.to_owned(),
                json!({
                    "kind": named.map(|record| record["kind"].clone()),
                    "title": named.map(|record| record["title"].clone()),
                    "counts": done,
                    "per_second": per_second,
                }),
            )
        })
        .collect::<serde_json::Map<String, Value>>()
        .into()
}

fn commit_frames(frames: &[&Value]) -> Value {
    let (mut with, mut without) = ([0u64; 2], [0u64; 2]);
    let mut commits = 0u64;
    for record in frames {
        let Some(count) = record["commits"].as_u64() else {
            continue;
        };
        commits += count;
        let missed = number(record, "dt").is_some_and(|dt| dt > 1.5 * BUDGET_MS);
        let into = if count > 0 { &mut with } else { &mut without };
        into[0] += 1;
        into[1] += u64::from(missed);
    }
    json!({
        "commits": commits,
        "after_commit": { "frames": with[0], "missed": with[1] },
        "without_commit": { "frames": without[0], "missed": without[1] },
    })
}

fn zone_commits(frames: &[&Value], playing_seconds: Option<f64>) -> Value {
    let mut counts: BTreeMap<&str, u64> = BTreeMap::new();
    for record in frames {
        let Some(zones) = record["zones"].as_object() else {
            continue;
        };
        for (zone, count) in zones {
            *counts.entry(zone.as_str()).or_default() += count.as_u64().unwrap_or(0);
        }
    }
    counts
        .into_iter()
        .map(|(zone, count)| {
            let rate = playing_seconds
                .filter(|seconds| *seconds > 0.0)
                .map(|seconds| count as f64 / seconds);
            (zone.to_owned(), json!({ "count": count, "per_second": rate }))
        })
        .collect::<serde_json::Map<String, Value>>()
        .into()
}

struct Window {
    start: f64,
    end: f64,
}

impl Window {
    fn holds(&self, record: &Value) -> bool {
        at(record).is_some_and(|when| when >= self.start && when <= self.end)
    }
}

fn sorted<'a>(records: impl Iterator<Item = &'a Value>) -> Vec<&'a Value> {
    let mut records: Vec<&Value> = records.filter(|record| at(record).is_some()).collect();
    records.sort_by(|a, b| at(a).unwrap_or(0.0).total_cmp(&at(b).unwrap_or(0.0)));
    records
}

struct Interval {
    from: f64,
    to: f64,
    speed: f64,
    sequence: Option<f64>,
}

fn intervals(ticks: &[&Value], window: &Window) -> Vec<Interval> {
    let mut found = Vec::new();
    let mut open: Option<Interval> = None;
    let mut seek: Option<(Option<f64>, Option<f64>)> = None;
    for tick in ticks {
        let (Some(when), Some(speed)) = (at(tick), number(tick, "speed")) else {
            continue;
        };
        let playing = tick["playing"].as_bool() == Some(true);
        let sequence = number(tick, "seek");
        let seeked = seek.is_none_or(|(last, _)| last != sequence);
        if seeked {
            seek = Some((sequence, number(tick, "sequence")));
        }
        if let Some(interval) = open.take_if(|interval| !playing || seeked || interval.speed != speed) {
            found.push(Interval { to: when, ..interval });
        }
        if playing && open.is_none() {
            let sequence = seek.and_then(|(_, sequence)| sequence);
            open = Some(Interval { from: when, to: when, speed, sequence });
        }
    }
    if let Some(interval) = open {
        found.push(Interval { to: window.end, ..interval });
    }
    found
        .into_iter()
        .filter_map(|interval| {
            let (from, to) = (interval.from.max(window.start), interval.to.min(window.end));
            (to > from).then_some(Interval { from, to, ..interval })
        })
        .collect()
}

fn active_spans(lists: &[&Value], view: &str) -> Vec<(f64, f64)> {
    let mut spans = Vec::new();
    let mut since = None;
    for list in lists {
        let Some(when) = at(list) else { continue };
        let on = list["views"].as_array().is_some_and(|views| {
            views.iter().any(|each| {
                each["id"] == view && each["visible"] == true && each["follow"] == true
            })
        });
        match (since, on) {
            (None, true) => since = Some(when),
            (Some(from), false) => {
                spans.push((from, when));
                since = None;
            }
            _ => {}
        }
    }
    if let Some(from) = since {
        spans.push((from, f64::INFINITY));
    }
    spans
}

struct Ack {
    at: f64,
    shown: Option<f64>,
    sequence: Option<f64>,
}

fn shown(record: &Value) -> Option<f64> {
    if record["blank"] == true {
        return None;
    }
    Some(number(record, "hour")? + number(record, "fraction").unwrap_or(0.0))
}

#[derive(Default)]
struct Achieved {
    playing_ms: f64,
    covered_ms: f64,
    hours: f64,
    clock_hours: f64,
    longest_stall_ms: f64,
}

fn achieved(acks: &[Ack], intervals: &[Interval], spans: &[(f64, f64)]) -> Achieved {
    let mut result = Achieved::default();
    for interval in intervals {
        for &(active, inactive) in spans {
            let (from, to) = (interval.from.max(active), interval.to.min(inactive));
            if to <= from {
                continue;
            }
            result.playing_ms += to - from;
            let current = |ack: &Ack| {
                ack.at >= active
                    && interval
                        .sequence
                        .zip(ack.sequence)
                        .is_some_and(|(seeked, shows)| shows >= seeked)
            };
            let before = acks.partition_point(|ack| ack.at <= from);
            let mut segment: Option<(f64, f64, f64, f64)> = before
                .checked_sub(1)
                .map(|last| &acks[last])
                .filter(|ack| current(*ack))
                .and_then(|ack| ack.shown)
                .map(|shown| (from, shown, from, shown));
            let close = |segment: (f64, f64, f64, f64), until: f64, result: &mut Achieved| {
                let (start, first, last_at, last) = segment;
                result.covered_ms += until - start;
                result.hours += last - first;
                result.clock_hours += interval.speed * (until - start) / 1000.0;
                result.longest_stall_ms = result.longest_stall_ms.max(until - last_at);
            };
            for ack in acks[before..].iter().take_while(|ack| ack.at <= to) {
                match (ack.shown, segment) {
                    (Some(shown), Some((start, first, last_at, last))) if shown != last => {
                        result.longest_stall_ms = result.longest_stall_ms.max(ack.at - last_at);
                        segment = Some((start, first, ack.at, shown));
                    }
                    (Some(_), Some(_)) => {}
                    (Some(shown), None) if current(ack) => {
                        segment = Some((ack.at, shown, ack.at, shown));
                    }
                    (None, Some(open)) => {
                        close(open, ack.at, &mut result);
                        segment = None;
                    }
                    (_, None) => {}
                }
            }
            if let Some(open) = segment {
                close(open, to, &mut result);
            }
        }
    }
    result
}

fn summarize(window: &Window, all: &[Value], map: &[Value], page_flushed: Option<bool>) -> Value {
    let records: Vec<&Value> = all.iter().filter(|record| window.holds(record)).collect();
    let of = |name: &'static str| {
        records
            .iter()
            .copied()
            .filter(move |record| kind(record) == name)
    };

    let mut losses: BTreeMap<String, Loss> = BTreeMap::new();
    for record in all.iter().chain(map).filter(|record| kind(record) == "lost") {
        let (from, to) = (
            number(record, "from").or_else(|| at(record)),
            number(record, "to").or_else(|| at(record)),
        );
        if from.is_some_and(|from| from <= window.end) && to.is_some_and(|to| to >= window.start) {
            losses
                .entry(record["source"].as_str().unwrap_or("-").to_owned())
                .or_default()
                .add(
                    record["count"].as_u64().unwrap_or(0),
                    record["bytes"].as_u64().unwrap_or(0),
                    from,
                    to,
                );
        }
    }
    let page_complete =
        !losses.contains_key("ui") && !losses.contains_key("host") && page_flushed != Some(false);

    let frames: Vec<&Value> = of("frame").collect();
    let dt = |playing: bool| {
        frames
            .iter()
            .filter(|record| !playing || record["playing"].as_bool() == Some(true))
            .filter_map(|record| number(record, "dt"))
            .collect::<Vec<_>>()
    };
    let playing_frames: Vec<&Value> = frames
        .iter()
        .copied()
        .filter(|record| record["playing"].as_bool() == Some(true))
        .collect();
    let page = all
        .iter()
        .rev()
        .find(|record| kind(record) == "page" && at(record).is_some_and(|when| when <= window.end));
    let long_tasks = if page.is_some_and(|page| page["longtask"].as_bool() == Some(true)) {
        let durations: Vec<f64> = of("longtask")
            .filter_map(|record| number(record, "dur"))
            .collect();
        json!({ "total_ms": durations.iter().sum::<f64>(), "duration_ms": stats(durations) })
    } else {
        json!({ "supported": page.map(|_| false) })
    };
    let long_frames = if page.is_some_and(|page| page["loaf"].as_bool() == Some(true)) {
        let loaf: Vec<&Value> = of("loaf").collect();
        json!({
            "duration_ms": stats(values(&loaf, "dur")),
            "blocking_ms": stats(values(&loaf, "blocking")),
            "script_ms": stats(values(&loaf, "script")),
            "render_ms": stats(values(&loaf, "render")),
        })
    } else {
        json!({ "supported": page.map(|_| false) })
    };

    let ticks = sorted(all.iter().filter(|record| kind(record) == "tick"));
    let playing = intervals(&ticks, window);
    let playing_ms: f64 = playing.iter().map(|interval| interval.to - interval.from).sum();
    let clock_hours: f64 = playing
        .iter()
        .map(|interval| interval.speed * (interval.to - interval.from) / 1000.0)
        .sum();
    let speed = of("tick").next_back().and_then(|record| number(record, "speed"));
    let lists = sorted(all.iter().filter(|record| kind(record) == "map_views"));
    let mut acks: BTreeMap<String, Vec<&Value>> = BTreeMap::new();
    for record in sorted(all.iter().filter(|record| kind(record) == "map_state")) {
        if let Some(view) = record["view"].as_str() {
            acks.entry(view.to_owned()).or_default().push(record);
        }
    }
    let mut maps: serde_json::Map<String, Value> = acks
        .into_iter()
        .filter(|(_, states)| states.iter().any(|record| window.holds(record)))
        .map(|(view, states)| {
            let inside: Vec<&Value> = states.iter().copied().filter(|record| window.holds(record)).collect();
            let lag: Vec<f64> = inside
                .iter()
                .filter(|record| record["playing"] == true && record["follow"] == true)
                .filter_map(|&record| Some(number(record, "expected")? - shown(record)?))
                .collect();
            let follow = inside.last().map(|record| record["follow"] == true);
            let timeline: Vec<Ack> = states
                .iter()
                .map(|&record| Ack {
                    at: at(record).unwrap_or(0.0),
                    shown: shown(record),
                    sequence: number(record, "seq"),
                })
                .collect();
            let spans = active_spans(&lists, &view);
            let measured = achieved(&timeline, &playing, &spans);
            let covered = page_complete && measured.covered_ms > 0.0;
            let result = json!({
                "kind": "dashboard",
                "follow": follow,
                "acks": inside.len(),
                "playing_seconds": measured.playing_ms / 1000.0,
                "covered_seconds": page_complete.then_some(measured.covered_ms / 1000.0),
                "coverage": (page_complete && measured.playing_ms > 0.0)
                    .then(|| measured.covered_ms / measured.playing_ms),
                "hours_per_second": covered.then(|| measured.hours * 1000.0 / measured.playing_ms),
                "clock_hours_per_second": covered.then(|| measured.clock_hours * 1000.0 / measured.covered_ms),
                "longest_stall_ms": covered.then_some(measured.longest_stall_ms),
                "lag_hours": stats(lag),
            });
            (view, result)
        })
        .collect();

    let mut by_map: BTreeMap<String, Vec<&Value>> = BTreeMap::new();
    for record in map {
        by_map.entry(record["map"].to_string()).or_default().push(record);
    }
    let mut roles = BTreeSet::new();
    let map_frames: serde_json::Map<String, Value> = by_map
        .into_iter()
        .filter(|(_, records)| records.iter().any(|record| window.holds(record)))
        .map(|(map_id, records)| {
            let role = records
                .iter()
                .find(|record| kind(record) == "map_open")
                .and_then(|record| record["role"].as_str())
                .map(str::to_owned);
            roles.extend(role.clone());
            let inside: Vec<&Value> = records.iter().copied().filter(|record| window.holds(record)).collect();
            let frames = || inside.iter().filter(|record| kind(record) == "map");
            let work = frames().filter_map(|record| number(record, "work_ms"));
            let interval = frames().filter_map(|record| number(record, "dt"));
            let paint = frames().filter_map(|record| number(record, "paint_ms"));
            let acquire = frames().filter_map(|record| number(record, "acquire_ms"));
            let mut lost = Loss::default();
            for record in records.iter().filter(|record| kind(record) == "lost") {
                let (from, to) = (number(record, "from"), number(record, "to"));
                if from.is_some_and(|from| from <= window.end) && to.is_some_and(|to| to >= window.start) {
                    lost.add(
                        record["count"].as_u64().unwrap_or(0),
                        record["bytes"].as_u64().unwrap_or(0),
                        from,
                        to,
                    );
                }
            }
            let drained = records.iter().any(|record| {
                kind(record) == "map_close" || at(record).is_some_and(|when| when >= window.end)
            });
            (
                map_id,
                json!({
                    "role": role,
                    "work_ms": stats(work.collect()),
                    "paint_ms": stats(paint.collect()),
                    "acquire_ms": stats(acquire.collect()),
                    "interval_ms": frame_stats(interval.collect()),
                    "lost": lost.json(),
                    "drained": drained,
                }),
            )
        })
        .collect();
    for (role, view, view_kind, reason) in [
        ("main", "campaign-map", "campaign", "not measured for campaign maps"),
        ("popout", "popout", "popout", "not measured for pop-out maps"),
    ] {
        if roles.contains(role) {
            maps.insert(
                view.to_owned(),
                json!({
                    "kind": view_kind,
                    "measured": reason,
                    "hours_per_second": null,
                    "lag_hours": null,
                }),
            );
        }
    }

    let server: HashMap<&str, &Value> = all
        .iter()
        .filter(|record| kind(record) == "srv")
        .filter_map(|record| Some((record["id"].as_str()?, record)))
        .collect();
    #[derive(Default)]
    struct Product<'a> {
        sent: Vec<&'a Value>,
        hits: usize,
        joined: usize,
    }
    let mut products: BTreeMap<String, Product<'_>> = BTreeMap::new();
    for record in of("q") {
        let product = products
            .entry(record["product"].as_str().unwrap_or("-").to_owned())
            .or_default();
        match record["cache"].as_str() {
            Some("hit") => product.hits += 1,
            Some("joined") => product.joined += 1,
            _ => product.sent.push(record),
        }
    }
    let queries: serde_json::Map<String, Value> = products
        .into_iter()
        .map(|(name, product)| {
            let outcome = |which: &str| {
                product
                    .sent
                    .iter()
                    .filter(|record| record["outcome"] == which)
                    .count()
            };
            let ok: Vec<&Value> = product
                .sent
                .iter()
                .copied()
                .filter(|record| record["outcome"] == "ok")
                .collect();
            let replies: Vec<&Value> = ok
                .iter()
                .filter_map(|record| server.get(record["id"].as_str()?).copied())
                .collect();
            let unmatched = product
                .sent
                .iter()
                .filter(|record| record["id"].as_str().is_none_or(|id| !server.contains_key(id)))
                .count();
            let flagged = |name: &str| replies.iter().filter(|record| record[name] == true).count();
            (
                name,
                json!({
                    "sent": product.sent.len(),
                    "ok": ok.len(),
                    "cancelled": outcome("cancelled"),
                    "error": outcome("error"),
                    "ui_cache_hits": product.hits,
                    "ui_joined": product.joined,
                    "ms": stats(values(&ok, "ms")),
                    "parse_ms": stats(values(&ok, "parse_ms")),
                    "bytes": stats(values(&ok, "bytes")),
                    "server_matched": replies.len(),
                    "server_unmatched": unmatched,
                    "server_queue_ms": stats(values(&replies, "queue_ms")),
                    "server_compute_ms": stats(values(&replies, "compute_ms")),
                    "server_deliver_ms": stats(values(&replies, "deliver_ms")),
                    "server_total_ms": stats(values(&replies, "total_ms")),
                    "server_sampled": flagged("sampled"),
                    "server_disk_cached": flagged("disk_cached"),
                }),
            )
        })
        .collect();
    let mut lanes: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    for record in of("srv") {
        if let (Some(lane), Some(queue)) = (record["lane"].as_str(), number(record, "queue_ms")) {
            lanes.entry(lane.to_owned()).or_default().push(queue);
        }
    }
    let lanes: serde_json::Map<String, Value> = lanes
        .into_iter()
        .map(|(lane, queue)| (lane, json!({ "queue_ms": stats(queue) })))
        .collect();

    let losses: serde_json::Map<String, Value> = losses
        .into_iter()
        .map(|(source, loss)| (source, loss.json()))
        .collect();
    json!({
        "budget_ms": BUDGET_MS,
        "complete": losses.is_empty() && page_flushed != Some(false),
        "losses": losses,
        "ui_frames": {
            "all": frame_stats(dt(false)),
            "playing": frame_stats(dt(true)),
            "playing_late_ms": stats(values(&playing_frames, "late")),
            "playing_main_ms": stats(values(&playing_frames, "main")),
            "playing_missed": missed_frames(&playing_frames),
            "playing_commits": commit_frames(&playing_frames),
        },
        "widgets": widget_work(all, window.end, &playing_frames, page_complete.then_some(playing_ms / 1000.0)),
        "zones": zone_commits(&playing_frames, page_complete.then_some(playing_ms / 1000.0)),
        "long_tasks": long_tasks,
        "long_frames": long_frames,
        "playback": {
            "speed": speed,
            "playing_seconds": page_complete.then_some(playing_ms / 1000.0),
            "clock_hours_per_second": (page_complete && playing_ms > 0.0)
                .then(|| clock_hours * 1000.0 / playing_ms),
            "maps": maps,
        },
        "queries": queries,
        "lanes": lanes,
        "map_frames": map_frames,
    })
}

#[tauri::command]
pub fn perf_enabled() -> bool {
    log().is_some()
}

#[tauri::command]
pub fn perf_write(lines: String) -> Result<(), String> {
    let log = log().ok_or("Measurement is off")?;
    let lines = lines.trim_end();
    if !lines.is_empty() {
        log.write(lines.to_owned());
        log.summarized.store(false, Ordering::Release);
    }
    Ok(())
}

#[tauri::command]
pub fn perf_finished(complete: bool) {
    if let Some(log) = log()
        && let Some(done) = log
            .page_final
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take()
    {
        let _ = done.send(complete);
    }
}

#[tauri::command]
pub async fn perf_stop() -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(|| {
        log()
            .ok_or("Measurement is off")?
            .summarize(None, true, Some(true))
    })
    .await
    .map_err(|error| error.to_string())?
}

pub fn finish_page(app: &AppHandle) -> Option<bool> {
    log()?.finish_page(app)
}

pub fn finish(page: Option<bool>) {
    let Some(log) = log() else {
        return;
    };
    if !log.summarized.swap(true, Ordering::AcqRel) {
        let _ = log.summarize(Some(epoch_ms()), false, page);
    }
}
