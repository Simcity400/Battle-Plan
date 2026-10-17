//! `--readiness`: time from a cold open until every registered widget's default
//! view and the map's first frame can draw, while background preparation runs.
//!
//! The recording opens through the data service exactly as the desktop opens it,
//! so preparation starts from the same entry point on the same workers. Every
//! widget's default requests are then in flight at once, as mounted widgets
//! issue them, and the service serves them on its own lanes. A prepared request
//! that is still pending is asked again when the coverage its last reply waits
//! for moves (the entries its `gate` names: a family prepared on its own, or
//! the hourly walk's), as the client's prepared query loop does
//! ([`progress_key`]); product loads are issued once.

use dashboard::{
    Config, Service, Source, prepared,
    products::ContentOptions,
    readiness::{self, Bindings, Instantiated, Outcome, Route},
    transport::{CAPACITY_REACHED, DataCommand, DataEvent, ProductRequest},
};
use dashboard_base::prepared_map;
use replay::container::SeekResult;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs::{self, File},
    io::{self, BufWriter, Write},
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex},
    thread::JoinHandle,
    time::{Duration, Instant},
};

pub struct Settings {
    pub recording: PathBuf,
    pub game: PathBuf,
    pub user: PathBuf,
    pub allow_unverified: bool,
    pub limit: Duration,
    pub table: Option<PathBuf>,
    /// Open once beforehand with preparation off, so the content hash index is
    /// as an installed app holds it; the prepared store stays empty.
    pub warm_content: bool,
}

/// Summary geometry width: the acceptance harness's chart viewport.
const CHART_WIDTH: u32 = 800;
const FIRST_POLL: Duration = Duration::from_millis(250);
const LONGEST_POLL: Duration = Duration::from_millis(1000);
/// Below the service's admission limit, leaving room for status requests.
const IN_FLIGHT: usize = 56;
/// Queries slower than this are reported as they finish.
const SLOW_QUERY_MS: f64 = 1000.0;

#[derive(Default)]
struct Inbox {
    partial: HashMap<String, String>,
    done: HashMap<String, Result<String, String>>,
}

#[derive(Default)]
struct Channel {
    inbox: Mutex<Inbox>,
    arrived: Condvar,
}

impl Channel {
    fn emit(&self, event: DataEvent) -> bool {
        let mut inbox = self.inbox.lock().unwrap_or_else(|error| error.into_inner());
        match event {
            DataEvent::Chunk {
                id, last, fragment, ..
            } => {
                inbox
                    .partial
                    .entry(id.clone())
                    .or_default()
                    .push_str(&fragment);
                if last {
                    let text = inbox.partial.remove(&id).unwrap_or_default();
                    inbox.done.insert(id, Ok(text));
                }
            }
            DataEvent::Error { id, message } => {
                inbox.partial.remove(&id);
                inbox.done.insert(id, Err(message));
            }
        }
        drop(inbox);
        self.arrived.notify_all();
        true
    }

    fn take(&self, id: &str) -> Option<Result<String, String>> {
        self.inbox
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .done
            .remove(id)
    }

    fn wait(&self, id: &str, deadline: Instant) -> io::Result<Result<String, String>> {
        let mut inbox = self.inbox.lock().unwrap_or_else(|error| error.into_inner());
        loop {
            if let Some(reply) = inbox.done.remove(id) {
                return Ok(reply);
            }
            let now = Instant::now();
            if now >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!("The data service did not answer {id} within the limit"),
                ));
            }
            inbox = self
                .arrived
                .wait_timeout(inbox, deadline - now)
                .unwrap_or_else(|error| error.into_inner())
                .0;
        }
    }

    /// Sleep until one of `ids` answers or the timeout passes.
    fn wait_any(&self, ids: &HashSet<String>, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        let mut inbox = self.inbox.lock().unwrap_or_else(|error| error.into_inner());
        while !ids.iter().any(|id| inbox.done.contains_key(id)) {
            let now = Instant::now();
            if now >= deadline {
                return;
            }
            inbox = self
                .arrived
                .wait_timeout(inbox, deadline - now)
                .unwrap_or_else(|error| error.into_inner())
                .0;
        }
    }
}

/// How often the evidence records every thread's CPU by label.
const CPU_EVERY: Duration = Duration::from_secs(30);

fn millis(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

struct Output {
    file: BufWriter<File>,
}

impl Output {
    fn line(&mut self, value: Value) -> io::Result<()> {
        serde_json::to_writer(&mut self.file, &value)?;
        self.file.write_all(b"\n")?;
        self.file.flush()?;
        println!("{value}");
        Ok(())
    }
}

/// One query in flight: its request id and when it was asked.
struct Asked {
    id: String,
    at: Instant,
    at_ms: f64,
}

/// One distinct request; widgets asking the same thing share it, as the
/// client's shared product cache does.
struct Tracked {
    product: String,
    route: Route,
    params: Value,
    in_flight: Option<Asked>,
    /// Preparation epoch of the last ask.
    asked_epoch: Option<u64>,
    /// The status when it was last asked.
    asked_status: Value,
    /// The progress its last reply follows ([`progress_key`]); a pending
    /// prepared request is asked again once it moves, or when None.
    asked_key: Option<String>,
    /// The status entries its last reply waits for; None when it names none.
    gate: Option<Vec<String>>,
    outcome: Option<Outcome>,
    /// Its first view's class, as the table names it.
    first_view: Option<readiness::FirstView>,
    /// When its first view arrived: its first ready (or unavailable) reply.
    settled_ms: Option<f64>,
    /// When its reply became exact (or unavailable): no later preparation
    /// changes it. A ready reply is asked again until then.
    exact_ms: Option<f64>,
    /// Start and duration of the query whose reply settled it.
    settle_asked_ms: Option<f64>,
    settle_query_ms: Option<f64>,
    error: Option<String>,
    data: Option<Value>,
    keep: bool,
    queries: u32,
    query_ms: f64,
    longest_query_ms: f64,
    first_asked_ms: Option<f64>,
}

#[derive(Clone, Copy)]
enum Slot {
    Unresolved,
    Request(usize),
    /// Its window list names no window at that hour: nothing to read.
    Absent(f64),
}

struct WidgetState {
    entry: readiness::Widget,
    slots: Vec<Slot>,
    auxiliary: Vec<usize>,
    /// When every default request had its first view.
    ready_ms: Option<f64>,
    /// When every default request was exact (or unavailable).
    exact_ms: Option<f64>,
}

impl WidgetState {
    /// Resolve in dependency order from the latest replies, including slots
    /// already resolved from a sampled selection or an empty window list.
    fn refresh(&mut self, probe: &mut Probe, bindings: &Bindings) -> io::Result<()> {
        let kept: HashSet<usize> = self
            .entry
            .requests
            .iter()
            .flat_map(readiness::waits_for)
            .collect();
        let mut replies = BTreeMap::new();
        for index in 0..self.slots.len() {
            let template = &self.entry.requests[index];
            let waits = readiness::waits_for(template);
            if !waits.is_empty() || matches!(self.slots[index], Slot::Unresolved) {
                self.slots[index] =
                    match readiness::instantiate(&template.params, bindings, &replies)
                        .map_err(io::Error::other)?
                    {
                        Instantiated::Ready(params) => {
                            Slot::Request(probe.track(template, params, kept.contains(&index)))
                        }
                        Instantiated::Absent => {
                            let at = waits
                                .iter()
                                .filter_map(|&wait| match self.slots[wait] {
                                    Slot::Request(tracked) => probe.tracked[tracked].settled_ms,
                                    Slot::Absent(at) => Some(at),
                                    Slot::Unresolved => None,
                                })
                                .fold(0.0, f64::max);
                            Slot::Absent(at)
                        }
                        Instantiated::Waiting => Slot::Unresolved,
                    };
            }
            if let Slot::Request(tracked) = self.slots[index]
                && let Some(data) = &probe.tracked[tracked].data
            {
                replies.insert(index, data.clone());
            }
        }
        Ok(())
    }
}

struct Probe<'a> {
    service: &'a Service,
    channel: &'a Channel,
    lease: String,
    revision: String,
    started: Instant,
    next: u64,
    epoch: u64,
    tracked: Vec<Tracked>,
    by_key: HashMap<String, usize>,
    status_queries: u32,
    status_ms: f64,
    lines: Vec<Value>,
    /// The latest preparation status.
    status: Value,
}

/// What the client's prepared query loop (`preparedSeriesQuery`) asks a
/// pending or partial request again on: the coverage on the request's branch
/// (every branch without one) of each status entry its reply's `gate` names,
/// a family prepared on its own from `families` or the hourly walk's
/// `coverage`, or the walk's for a reply that names none; section progress
/// from a status that lists no coverage. Preparation finishing moves it too,
/// so a reply still null then is asked once more and settles as unavailable.
fn progress_key(status: &Value, params: &Value, gate: Option<&[String]>) -> String {
    let branch = params.get("branchId").and_then(Value::as_str);
    let extent = |coverage: &Value| {
        if coverage.is_object() {
            json!([
                coverage["firstHour"],
                coverage["lastHour"],
                coverage["complete"]
            ])
        } else {
            Value::Null
        }
    };
    let key = |map: &Value| match branch {
        Some(branch) => extent(&map[branch]),
        None => Value::Array(map.as_object().map_or_else(Vec::new, |branches| {
            branches
                .iter()
                .map(|(id, coverage)| json!([id, extent(coverage)]))
                .collect()
        })),
    };
    let walk = [prepared::coverage::WALK.to_owned()];
    let entries = gate.unwrap_or(&walk);
    let maps: Option<Vec<&Value>> = entries
        .iter()
        .map(|entry| {
            if entry == prepared::coverage::WALK {
                status.get("coverage")
            } else {
                status
                    .get("families")
                    .map(|families| &families[entry.as_str()])
            }
        })
        .collect();
    let progress = match (maps, gate) {
        (None, _) => json!([status["state"] == "ready", status["sections"]]),
        (Some(maps), None) => key(maps[0]),
        (Some(maps), Some(gate)) => Value::Array(
            gate.iter()
                .zip(maps)
                .map(|(entry, map)| json!([entry, key(map)]))
                .collect(),
        ),
    };
    let finished = matches!(
        status["state"].as_str(),
        Some("ready" | "failed" | "cancelled")
    );
    json!([finished, progress]).to_string()
}

impl Probe<'_> {
    fn id(&mut self) -> String {
        self.next += 1;
        format!("readiness-{}", self.next)
    }

    fn now_ms(&self) -> f64 {
        millis(self.started.elapsed())
    }

    fn request(&self, index: usize) -> ProductRequest {
        ProductRequest {
            source_id: self.lease.clone(),
            revision: self.revision.clone(),
            product: self.tracked[index].product.clone(),
            params: self.tracked[index].params.clone(),
        }
    }

    fn in_flight(&self) -> usize {
        self.tracked
            .iter()
            .filter(|tracked| tracked.in_flight.is_some())
            .count()
    }

    fn track(&mut self, template: &readiness::Template, params: Value, keep: bool) -> usize {
        let key = serde_json::to_string(&json!([
            template.product,
            template.route == Route::Product,
            params
        ]))
        .expect("request key");
        if let Some(&index) = self.by_key.get(&key) {
            self.tracked[index].keep |= keep;
            self.tracked[index].first_view = self.tracked[index].first_view.or(template.first_view);
            return index;
        }
        let index = self.tracked.len();
        self.tracked.push(Tracked {
            product: template.product.clone(),
            route: template.route,
            params,
            in_flight: None,
            asked_epoch: None,
            asked_status: Value::Null,
            asked_key: None,
            gate: None,
            outcome: None,
            first_view: template.first_view,
            settled_ms: None,
            exact_ms: None,
            settle_asked_ms: None,
            settle_query_ms: None,
            error: None,
            data: None,
            keep,
            queries: 0,
            query_ms: 0.0,
            longest_query_ms: 0.0,
            first_asked_ms: None,
        });
        self.by_key.insert(key, index);
        index
    }

    /// Issue one request without waiting: a product load once, a prepared
    /// query again at each preparation epoch while it is pending.
    fn ask(&mut self, index: usize) {
        let id = self.id();
        let request = self.request(index);
        let command = match self.tracked[index].route {
            Route::Product => DataCommand::Product {
                id: id.clone(),
                request,
            },
            Route::Prepared => DataCommand::PreparedQuery {
                id: id.clone(),
                request,
            },
        };
        let at_ms = self.now_ms();
        let epoch = self.epoch;
        let status = self.status.clone();
        let tracked = &mut self.tracked[index];
        tracked.in_flight = Some(Asked {
            id,
            at: Instant::now(),
            at_ms,
        });
        tracked.asked_epoch = Some(epoch);
        tracked.asked_key = Some(progress_key(
            &status,
            &tracked.params,
            tracked.gate.as_deref(),
        ));
        tracked.asked_status = status;
        tracked.queries += 1;
        if tracked.first_asked_ms.is_none() {
            tracked.first_asked_ms = Some(at_ms);
        }
        self.service.dispatch(command);
    }

    /// Take every reply that has arrived.
    fn collect(&mut self, finished: bool) {
        for index in 0..self.tracked.len() {
            let Some(id) = self.tracked[index]
                .in_flight
                .as_ref()
                .map(|asked| asked.id.clone())
            else {
                continue;
            };
            let Some(reply) = self.channel.take(&id) else {
                continue;
            };
            let asked = self.tracked[index].in_flight.take().expect("asked");
            let spent = millis(asked.at.elapsed());
            let tracked = &mut self.tracked[index];
            tracked.query_ms += spent;
            tracked.longest_query_ms = tracked.longest_query_ms.max(spent);
            self.record(index, reply, finished, asked.at_ms, spent);
            if spent >= SLOW_QUERY_MS {
                let line = json!({"event": "slow-query", "ms": self.now_ms(), "askedMs": asked.at_ms,
                    "queryMs": spent, "request": self.describe(index)});
                self.lines.push(line);
            }
        }
    }

    fn record(
        &mut self,
        index: usize,
        reply: Result<String, String>,
        finished: bool,
        asked_ms: f64,
        spent: f64,
    ) {
        let now = self.now_ms();
        let tracked = &mut self.tracked[index];
        let data = reply.and_then(|text| {
            serde_json::from_str::<Value>(&text)
                .map(|mut value| {
                    let gate = value.get("gate").and_then(Value::as_array).map(|gate| {
                        gate.iter()
                            .filter_map(Value::as_str)
                            .map(str::to_owned)
                            .collect::<Vec<_>>()
                    });
                    let data = value
                        .get_mut("data")
                        .map(Value::take)
                        .unwrap_or(Value::Null);
                    (data, gate.filter(|gate| !gate.is_empty()))
                })
                .map_err(|error| format!("Malformed reply: {error}"))
        });
        match data {
            Ok((data, gate)) => {
                // The client follows the status it asked with, under the gate
                // this reply names; after a first null reply it asks once more
                // at its next status.
                tracked.asked_key = (!(data.is_null() && tracked.queries == 1))
                    .then(|| progress_key(&tracked.asked_status, &tracked.params, gate.as_deref()));
                tracked.gate = gate;
                // A product load is the whole reply: null there means nothing to show.
                let finished = finished || tracked.route == Route::Product;
                let outcome =
                    readiness::classify(&tracked.product, &tracked.params, &data, finished);
                tracked.error = None;
                tracked.outcome = Some(outcome);
                if tracked.keep {
                    tracked.data = outcome.settled().then_some(data);
                }
                if outcome.settled() && tracked.settled_ms.is_none() {
                    tracked.settled_ms = Some(now);
                    tracked.settle_asked_ms = Some(asked_ms);
                    tracked.settle_query_ms = Some(spent);
                    let line = json!({"event": "request", "ms": now, "request": index,
                        "settled": self.describe(index), "queries": self.tracked[index].queries,
                        "queryMs": self.tracked[index].query_ms});
                    self.lines.push(line);
                }
                if outcome.complete() && self.tracked[index].exact_ms.is_none() {
                    self.tracked[index].exact_ms = Some(now);
                    let line = json!({"event": "request-exact", "ms": now, "request": index,
                        "exact": self.describe(index), "queries": self.tracked[index].queries});
                    self.lines.push(line);
                }
            }
            Err(error) => {
                // The client retries a full queue; other refusals wait for
                // the next preparation change, as its query loop does.
                if error.starts_with(CAPACITY_REACHED) {
                    tracked.asked_epoch = None;
                    tracked.asked_key = None;
                }
                if tracked.error.as_ref() != Some(&error) {
                    let line = json!({"event": "request-error", "ms": now, "request": index,
                        "product": tracked.product, "params": tracked.params, "error": error});
                    self.lines.push(line);
                }
                tracked.error = Some(error);
            }
        }
    }

    fn describe(&self, index: usize) -> Value {
        let tracked = &self.tracked[index];
        json!({
            "request": index, "product": tracked.product,
            "family": tracked.params.get("family").and_then(|family| family.get("kind")),
            "route": if tracked.route == Route::Product { "product" } else { "prepared" },
            "outcome": tracked.outcome.map(Outcome::name), "settledMs": tracked.settled_ms,
            "firstView": tracked.first_view.map(readiness::FirstView::name),
            "firstViewMs": tracked.settled_ms, "exactMs": tracked.exact_ms,
            "settleAskedMs": tracked.settle_asked_ms, "settleQueryMs": tracked.settle_query_ms,
            "firstAskedMs": tracked.first_asked_ms, "error": tracked.error,
            "inFlightSinceMs": tracked.in_flight.as_ref().map(|asked| asked.at_ms),
        })
    }
}

fn accepted_hours(status: &Value) -> i64 {
    status["coverage"].as_object().map_or(0, |branches| {
        branches
            .values()
            .filter_map(|coverage| {
                Some(coverage["lastHour"].as_i64()? - coverage["firstHour"].as_i64()? + 1)
            })
            .sum()
    })
}

/// Settle time of a slot list and the slot that set it, or None while any waits.
fn settled(probe: &Probe, slots: &[Slot]) -> Option<(f64, Option<usize>)> {
    reached(probe, slots, |tracked| tracked.settled_ms)
}

/// When every slot's reply was exact, with the request that became so last.
fn exact(probe: &Probe, slots: &[Slot]) -> Option<(f64, Option<usize>)> {
    reached(probe, slots, |tracked| tracked.exact_ms)
}

fn reached(
    probe: &Probe,
    slots: &[Slot],
    at: impl Fn(&Tracked) -> Option<f64>,
) -> Option<(f64, Option<usize>)> {
    let mut latest = (0.0, None);
    for slot in slots {
        let (at, gate) = match *slot {
            Slot::Unresolved => return None,
            Slot::Absent(at) => (at, None),
            Slot::Request(index) => (at(&probe.tracked[index])?, Some(index)),
        };
        if at >= latest.0 {
            latest = (at, gate.or(latest.1));
        }
    }
    Some(latest)
}

fn fresh(path: &Path) -> io::Result<()> {
    let prepared = path.join("cache").join("local").join("HOI4Replay");
    if prepared.exists() && fs::read_dir(&prepared)?.next().is_some() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Readiness measures a cold open; use a new evidence directory",
        ));
    }
    Ok(())
}

/// The map widget's first frame, read as the viewer seeds it on its own thread:
/// through committed checkpoints when there are any, otherwise from the recording.
fn first_frame(
    source: Arc<Source>,
    content: ContentOptions,
    branch: u64,
    hour: i32,
    started: Instant,
) -> io::Result<Value> {
    let asked = Instant::now();
    let checkpoint = match prepared::pipeline::map_checkpoint_binding(&source, Some(&content))? {
        Some((store, key, progress)) => prepared_map::Reader::new(
            store,
            key,
            progress,
            source.replay(),
            prepared_map::selected_tables(),
        )?
        .seek(branch, hour)?
        .map(|current| matches!(current, prepared_map::Current::Frame(_))),
        None => None,
    };
    let (via, frame) = match checkpoint {
        Some(frame) => ("checkpoint", frame),
        None => (
            "recording",
            matches!(
                source.replay_ref().seek_selected(branch, hour, None)?,
                SeekResult::Frame(_)
            ),
        ),
    };
    Ok(json!({
        "event": "map", "readyMs": millis(started.elapsed()), "queryMs": millis(asked.elapsed()),
        "hour": hour, "via": via, "frame": frame,
        "scope": "first frame's recorded tables at the opening hour; map assets, hour builders and GPU presentation excluded"
    }))
}

pub fn run(settings: Settings, path: &Path) -> io::Result<bool> {
    fresh(path)?;
    // Measurement only: every thread's CPU by label, into the evidence.
    replay::cpu::enable();
    let table = match &settings.table {
        Some(file) => fs::read_to_string(file)?,
        None => readiness::WIDGETS.to_owned(),
    };
    let entries = readiness::widgets(&table).map_err(io::Error::other)?;
    let mut output = Output {
        file: BufWriter::new(File::create(path.join("readiness.jsonl"))?),
    };
    let recording = fs::canonicalize(&settings.recording)?;
    let config = Config {
        game_dir: settings.game.clone(),
        user_dir: settings.user.clone(),
        ..Default::default()
    };
    // The host resolves its folders before any recording opens.
    let content = ContentOptions {
        options: config.resolve_options()?,
        allow_unverified: settings.allow_unverified,
    };
    let revision = dashboard::protocol::replay_revision(&recording)?;
    let warm_open_ms = if settings.warm_content {
        let channel = Arc::new(Channel::default());
        let emit = Arc::clone(&channel);
        let warm = Service::new(Arc::new(move |event: DataEvent| emit.emit(event)))?;
        let quiet = Config {
            background_preparation: Some(false),
            ..config.clone()
        };
        warm.configure_content(&quiet, settings.allow_unverified)?;
        let asked = Instant::now();
        warm.dispatch(DataCommand::Open {
            id: "readiness-warm".into(),
            path: recording.to_string_lossy().into_owned(),
            revision: revision.clone(),
        });
        channel
            .wait("readiness-warm", Instant::now() + settings.limit)?
            .map_err(|error| io::Error::other(format!("Warm-up open: {error}")))?;
        let spent = millis(asked.elapsed());
        warm.shutdown();
        Some(spent)
    } else {
        None
    };
    let channel = Arc::new(Channel::default());
    let emit = Arc::clone(&channel);
    let service = Service::new(Arc::new(move |event: DataEvent| emit.emit(event)))?;
    service.configure_content(&config, settings.allow_unverified)?;

    let started = Instant::now();
    let started_unix_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(io::Error::other)?
        .as_millis()
        .to_string();
    let deadline = started + settings.limit;
    service.dispatch(DataCommand::Open {
        id: "readiness-open".into(),
        path: recording.to_string_lossy().into_owned(),
        revision: revision.clone(),
    });
    let opened = channel
        .wait("readiness-open", deadline)?
        .map_err(|error| io::Error::other(format!("Open: {error}")))?;
    let open_ms = millis(started.elapsed());
    let opened: Value = serde_json::from_str(&opened)?;
    let lease = opened["id"]
        .as_str()
        .ok_or_else(|| io::Error::other("Open reply has no source"))?
        .to_owned();
    let metadata = &opened["metadata"];
    let branch = &metadata["branches"][0];
    let (Some(branch_id), Some(first), Some(last)) = (
        branch["id"].as_str(),
        branch["firstHour"]
            .as_i64()
            .and_then(|hour| i32::try_from(hour).ok()),
        branch["lastHour"]
            .as_i64()
            .and_then(|hour| i32::try_from(hour).ok()),
    ) else {
        return Err(io::Error::other("The recording has no branch"));
    };
    let branch_number: u64 = branch_id
        .parse()
        .map_err(|_| io::Error::other("Invalid branch"))?;
    let countries = metadata["countries"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let tag = countries
        .iter()
        .find(|country| country["id"] == metadata["playerCountryId"])
        .or_else(|| countries.first())
        .and_then(|country| country["tag"].as_str())
        .ok_or_else(|| io::Error::other("The recording names no country"))?
        .to_owned();
    let bindings = Bindings {
        branch: branch_id.to_owned(),
        tag: tag.clone(),
        first,
        last,
        width: CHART_WIDTH,
    };
    let total_hours: i64 = metadata["branches"].as_array().map_or(0, |branches| {
        branches
            .iter()
            .filter_map(|branch| {
                Some(branch["lastHour"].as_i64()? - branch["firstHour"].as_i64()? + 1)
            })
            .sum()
    });
    replay::cpu::set_hours(first, last);
    output.line(json!({
        "event": "open", "recording": recording, "startedUnixMs": started_unix_ms, "openMs": open_ms,
        "branch": branch_id, "firstHour": first, "lastHour": last, "tag": tag, "totalHours": total_hours,
        "widgets": entries.len(), "limitMs": millis(settings.limit),
        "chartWidth": CHART_WIDTH, "warmContentOpenMs": warm_open_ms,
        "contentIndex": if settings.warm_content { "warmed by an earlier open with preparation off" } else { "cold" }
    }))?;

    // The map mounts with the widgets; the viewer seeds its first frame apart
    // from the dashboard's requests.
    let map_source = service
        .open_map(&recording, Some(revision.as_str()))
        .map_err(io::Error::other)?;
    let map_content = content.clone();
    let mut map_thread: Option<JoinHandle<io::Result<Value>>> = Some(
        std::thread::Builder::new()
            .name("readiness map first frame".into())
            .spawn(move || first_frame(map_source, map_content, branch_number, first, started))?,
    );
    let mut map_ms: Option<f64> = None;

    let mut probe = Probe {
        service: &service,
        channel: &channel,
        lease,
        revision,
        started,
        next: 0,
        epoch: 0,
        tracked: Vec::new(),
        by_key: HashMap::new(),
        status_queries: 0,
        status_ms: 0.0,
        lines: Vec::new(),
        status: Value::Null,
    };
    let mut widgets = Vec::with_capacity(entries.len());
    for entry in entries {
        let mut widget = WidgetState {
            slots: vec![Slot::Unresolved; entry.requests.len()],
            entry,
            auxiliary: Vec::new(),
            ready_ms: None,
            exact_ms: None,
        };
        widget.refresh(&mut probe, &bindings)?;
        let mut auxiliary = Vec::new();
        for template in &widget.entry.auxiliary {
            if let Instantiated::Ready(params) =
                readiness::instantiate(&template.params, &bindings, &BTreeMap::new())
                    .map_err(io::Error::other)?
            {
                auxiliary.push(probe.track(template, params, false));
            }
        }
        widget.auxiliary = auxiliary;
        widgets.push(widget);
    }

    let mut previous: Option<Value> = None;
    let mut poll = FIRST_POLL;
    let mut next_status = Instant::now();
    let mut status_asked: Option<(String, Instant)> = None;
    let mut status = Value::Null;
    let mut finished = false;
    let mut loops = 0_u64;
    let mut next_cpu = Instant::now() + CPU_EVERY;
    let mut cpu_at_ready = false;
    let until_prepared = std::env::var_os("BATTLE_PLAN_READINESS_CPU_UNTIL_PREPARED").or_else(|| std::env::var_os("HOI4_READINESS_CPU_UNTIL_PREPARED")).is_some();
    loop {
        loops += 1;
        if Instant::now() >= next_cpu {
            next_cpu = Instant::now() + CPU_EVERY;
            output.line(json!({
                "event": "cpu", "ms": probe.now_ms(), "acceptedHours": accepted_hours(&status),
                "sections": status["sections"], "cpu": replay::cpu::snapshot(),
            }))?;
        }
        // Preparation status has its own poll, apart from the queries.
        if status_asked.is_none() && Instant::now() >= next_status {
            let id = probe.id();
            service.dispatch(DataCommand::Preparation {
                id: id.clone(),
                source_id: probe.lease.clone(),
                revision: probe.revision.clone(),
                action: prepared::Action::Status,
            });
            status_asked = Some((id, Instant::now()));
        }
        let answered = status_asked
            .as_ref()
            .and_then(|(id, asked)| channel.take(id).map(|reply| (reply, asked.elapsed())));
        if let Some((reply, spent)) = answered {
            probe.status_queries += 1;
            probe.status_ms += millis(spent);
            status_asked = None;
            let text =
                reply.map_err(|error| io::Error::other(format!("Preparation status: {error}")))?;
            status = serde_json::from_str(&text)?;
            let state = status["state"].as_str().unwrap_or_default().to_owned();
            finished = matches!(state.as_str(), "ready" | "failed" | "cancelled");
            probe.status = status.clone();
            let key = json!([
                status["state"],
                status["coverage"],
                status["families"],
                status["sections"]
            ]);
            if previous.as_ref() != Some(&key) {
                probe.epoch += 1;
                output.line(json!({
                    "event": "preparation", "ms": probe.now_ms(), "state": state,
                    "acceptedHours": accepted_hours(&status), "totalHours": total_hours,
                    "coverage": status["coverage"], "families": status["families"], "error": status["error"],
                    "inFlight": probe.in_flight(),
                    "sections": status["sections"].as_array().map(|sections| sections.iter().map(|section| json!({
                        "id": section["id"], "state": section["state"], "done": section["done"], "total": section["total"],
                        "error": section["error"]
                    })).collect::<Vec<_>>())
                }))?;
                previous = Some(key);
                poll = FIRST_POLL;
            } else {
                poll = (poll * 2).min(LONGEST_POLL);
            }
            next_status = Instant::now() + poll;
        }
        probe.collect(finished);
        // Requests follow the latest dependency reply, as widget selections do.
        for widget in &mut widgets {
            widget.refresh(&mut probe, &bindings)?;
        }
        // Every request not yet answered is in flight, as mounted widgets keep
        // theirs; a pending prepared reply is asked again once the coverage it
        // waits for moves ([`progress_key`]).
        let active: HashSet<usize> = widgets
            .iter()
            .flat_map(|widget| {
                widget
                    .slots
                    .iter()
                    .filter_map(|slot| match slot {
                        Slot::Request(index) => Some(*index),
                        _ => None,
                    })
                    .chain(widget.auxiliary.iter().copied())
            })
            .collect();
        for index in 0..probe.tracked.len() {
            if !active.contains(&index) {
                continue;
            }
            let tracked = &probe.tracked[index];
            // A ready reply is asked again as its gate moves until it is exact.
            let due = tracked.in_flight.is_none()
                && tracked.exact_ms.is_none()
                && match tracked.route {
                    Route::Product => tracked.asked_epoch.is_none(),
                    Route::Prepared => tracked.asked_key.as_ref().is_none_or(|asked| {
                        *asked
                            != progress_key(&probe.status, &tracked.params, tracked.gate.as_deref())
                    }),
                };
            if due && probe.in_flight() < IN_FLIGHT {
                probe.ask(index);
            }
        }
        if let Some(thread) = map_thread.take_if(|thread| thread.is_finished()) {
            let line = thread
                .join()
                .map_err(|_| io::Error::other("Map first frame panicked"))??;
            map_ms = line["readyMs"].as_f64();
            output.line(line)?;
        }
        for line in std::mem::take(&mut probe.lines) {
            output.line(line)?;
        }
        for widget in &mut widgets {
            if widget.ready_ms.is_some() {
                if widget.exact_ms.is_none() {
                    let reached = if widget.entry.map {
                        widget.ready_ms.map(|at| (at, None))
                    } else {
                        exact(&probe, &widget.slots).map(|(at, gate)| (at.max(open_ms), gate))
                    };
                    if let Some((at, gate)) = reached {
                        widget.exact_ms = Some(at);
                        output.line(json!({
                            "event": "widget-exact", "kind": widget.entry.kind, "exactMs": at,
                            "gatedBy": gate.map(|gate| probe.describe(gate)),
                        }))?;
                    }
                }
                continue;
            }
            // Nothing can draw before the recording opens.
            let reached = if widget.entry.map {
                map_ms.map(|at| (at, None))
            } else {
                settled(&probe, &widget.slots).map(|(at, gate)| (at.max(open_ms), gate))
            };
            if let Some((at, gate)) = reached {
                widget.ready_ms = Some(at);
                output.line(json!({
                    "event": "widget", "kind": widget.entry.kind, "readyMs": at,
                    "gatedBy": gate.map(|gate| probe.describe(gate)),
                    "unavailable": widget.slots.iter().filter_map(|slot| match slot {
                        Slot::Request(index) if probe.tracked[*index].outcome == Some(Outcome::Unavailable) => Some(probe.describe(*index)),
                        _ => None,
                    }).collect::<Vec<_>>(),
                }))?;
            }
        }
        let ready_now = widgets.iter().all(|widget| widget.ready_ms.is_some());
        if ready_now && !cpu_at_ready {
            cpu_at_ready = true;
            output.line(json!({
                "event": "cpu", "ms": probe.now_ms(), "acceptedHours": accepted_hours(&status),
                "sections": status["sections"], "cpu": replay::cpu::snapshot(), "allReady": true,
            }))?;
        }
        // Measurement only: with BATTLE_PLAN_READINESS_CPU_UNTIL_PREPARED set, the
        // CPU record continues until preparation itself ends.
        let exact_now = widgets.iter().all(|widget| widget.exact_ms.is_some());
        if (exact_now && (!until_prepared || finished)) || Instant::now() >= deadline {
            break;
        }
        let mut waiting: HashSet<String> = probe
            .tracked
            .iter()
            .filter_map(|tracked| tracked.in_flight.as_ref().map(|asked| asked.id.clone()))
            .collect();
        if let Some((id, _)) = &status_asked {
            waiting.insert(id.clone());
        }
        let now = Instant::now();
        let wake = if status_asked.is_some() {
            LONGEST_POLL
        } else {
            next_status.saturating_duration_since(now)
        };
        channel.wait_any(
            &waiting,
            wake.min(deadline.saturating_duration_since(now))
                .max(Duration::from_millis(1)),
        );
    }
    let ended_ms = probe.now_ms();
    if map_ms.is_none()
        && let Some(thread) = map_thread.take()
    {
        let line = thread
            .join()
            .map_err(|_| io::Error::other("Map first frame panicked"))??;
        map_ms = line["readyMs"].as_f64();
        output.line(line)?;
    }
    let all_ready = widgets.iter().all(|widget| widget.ready_ms.is_some());
    let mut ordered: Vec<_> = widgets.iter().collect();
    ordered.sort_by(|a, b| {
        a.ready_ms
            .unwrap_or(f64::INFINITY)
            .total_cmp(&b.ready_ms.unwrap_or(f64::INFINITY))
    });
    let rows: Vec<_> = ordered
        .iter()
        .map(|widget| {
            let gate = settled(&probe, &widget.slots).and_then(|(_, gate)| gate);
            let waiting: Vec<_> = widget
                .slots
                .iter()
                .filter_map(|slot| match slot {
                    Slot::Request(index) if probe.tracked[*index].settled_ms.is_none() => {
                        Some(probe.describe(*index))
                    }
                    Slot::Unresolved => {
                        Some(json!({"waitingFor": "an earlier reply's window list"}))
                    }
                    _ => None,
                })
                .collect();
            let auxiliary: Vec<_> = widget
                .auxiliary
                .iter()
                .map(|&index| probe.describe(index))
                .collect();
            json!({
                "kind": widget.entry.kind, "readyMs": widget.ready_ms,
                "firstViewMs": widget.ready_ms, "exactMs": widget.exact_ms,
                "firstViews": widget.entry.requests.iter()
                    .map(|template| template.first_view.map(readiness::FirstView::name))
                    .collect::<Vec<_>>(),
                "gatedBy": if widget.entry.map { json!({"product": "map first frame"}) } else { gate.map(|gate| probe.describe(gate)).unwrap_or(Value::Null) },
                "pending": waiting, "auxiliary": auxiliary, "note": widget.entry.note,
            })
        })
        .collect();
    let requests: Vec<_> = probe
        .tracked
        .iter()
        .enumerate()
        .map(|(index, tracked)| {
            json!({
                "request": index, "product": tracked.product, "params": tracked.params,
                "route": if tracked.route == Route::Product { "product" } else { "prepared" },
                "outcome": tracked.outcome.map(Outcome::name), "settledMs": tracked.settled_ms,
                "firstView": tracked.first_view.map(readiness::FirstView::name),
                "firstViewMs": tracked.settled_ms, "exactMs": tracked.exact_ms,
                "settleAskedMs": tracked.settle_asked_ms, "settleQueryMs": tracked.settle_query_ms,
                "error": tracked.error, "queries": tracked.queries, "firstAskedMs": tracked.first_asked_ms,
                "inFlightSinceMs": tracked.in_flight.as_ref().map(|asked| asked.at_ms),
                "queryMs": tracked.query_ms, "longestQueryMs": tracked.longest_query_ms,
            })
        })
        .collect();
    let query_ms: f64 = probe.tracked.iter().map(|tracked| tracked.query_ms).sum();
    let queries: u32 = probe.tracked.iter().map(|tracked| tracked.queries).sum();
    let overall = all_ready.then(|| {
        widgets
            .iter()
            .filter_map(|widget| widget.ready_ms)
            .fold(map_ms.unwrap_or(0.0), f64::max)
    });
    let cpu = replay::cpu::snapshot();
    output.line(json!({
        "event": "cpu", "ms": probe.now_ms(), "acceptedHours": accepted_hours(&status),
        "sections": status["sections"], "cpu": cpu, "final": true,
    }))?;
    let summary = json!({
        "cpu": cpu,
        "event": "summary", "recording": recording, "allReady": all_ready, "allReadyMs": overall,
        "warmContentOpenMs": warm_open_ms,
        "endedMs": ended_ms, "limitMs": millis(settings.limit), "openMs": open_ms, "mapMs": map_ms,
        "preparation": {"state": status["state"], "acceptedHours": accepted_hours(&status),
                        "totalHours": total_hours, "coverage": status["coverage"], "error": status["error"],
                        "epochs": probe.epoch},
        "polling": {"loops": loops, "statusQueries": probe.status_queries, "statusMs": probe.status_ms,
                    "queries": queries, "queryMs": query_ms, "inFlightLimit": IN_FLIGHT,
                    "scope": "Every default request is in flight at once, as mounted widgets issue them, and the data service serves them on its own lanes; query times are dispatch to last reply fragment and overlap. A pending prepared request is asked again when the coverage its last reply waits for (its gate) moves, as the client's prepared query loop does. Status is polled every 250 ms, backing off to 1 s while unchanged."},
        "widgets": rows,
        "requests": requests,
        "rule": "A widget is ready when every default request has a useful first view: exact; answered beyond accepted coverage by the product's own sampling, with its approximate spans marked; or answered from its gate's prepared prefix, naming the hours and values still being prepared (pending, preparing), as an event list cut at accepted coverage does. Exact data replaces each of these as preparation covers its span. Null replies are pending; a reply still null after preparation finished is unavailable and settles the widget. A widget whose default view reads nothing is ready at open. Auxiliary requests are reported, not gated. Each widget's firstViewMs is when every default request had its first view, and its exactMs when every one was exact (or unavailable); a ready reply is asked again as its gate moves until it is exact, and the probe runs until every widget is exact or the limit. firstViews names each default request's class: a exact at the hour, b sampled approximate, c the proven newest window, d the prepared prefix.",
    });
    serde_json::to_writer_pretty(File::create(path.join("readiness-summary.json"))?, &summary)?;
    output.line(json!({
        "event": "done", "allReady": all_ready, "allReadyMs": overall, "endedMs": ended_ms,
        "pendingWidgets": widgets.iter().filter(|widget| widget.ready_ms.is_none()).map(|widget| widget.entry.kind.clone()).collect::<Vec<_>>(),
        "inexactWidgets": widgets.iter().filter(|widget| widget.exact_ms.is_none()).map(|widget| widget.entry.kind.clone()).collect::<Vec<_>>(),
        "summary": path.join("readiness-summary.json"),
    }))?;
    // Queries still running at the limit are cancelled before the service stops.
    for tracked in &probe.tracked {
        if let Some(asked) = &tracked.in_flight {
            service.dispatch(DataCommand::Cancel {
                id: asked.id.clone(),
            });
        }
    }
    drop(probe);
    service.shutdown();
    Ok(true)
}

#[cfg(test)]
#[path = "readiness_probe_tests.rs"]
mod tests;
