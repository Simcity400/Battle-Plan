use super::heaviest::{GroupKey, drive, sync_live, thread_cpu_ns, thread_cycles, widen};
use dashboard_base::{Source, SourceKey, products::at_hour};
use dashboard_core::config::{ContentOptions, LoadContext};
use dashboard_follow::{
    Follow, Identity, Message, Query, SeriesMode, StepStats,
    live::CELLS,
    panel::{PanelData, ProductReader, ProductStep, Wake, dated, product::Delivery},
};
use playhead::{Declared, Hook, Lanes, Piece, Playhead, Step, pages::WorldPages};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::{self, BufRead, BufReader, BufWriter, Write},
    path::Path,
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    sync::{Arc, Mutex, atomic::AtomicBool, mpsc},
    time::{Duration, Instant},
};

#[derive(Deserialize)]
struct Plan {
    kind: String,
    view: String,
    page: String,
    viewport: Value,
    country: Value,
    requests: Vec<Request>,
    reasons: Vec<String>,
}

#[derive(Deserialize, Serialize, PartialEq)]
struct Ask {
    product: String,
    params: Value,
}

#[derive(Deserialize)]
#[serde(tag = "mode", rename_all = "kebab-case")]
enum Request {
    Follow {
        name: String,
        query: Value,
    },
    Read {
        name: String,
        hours: Vec<(i32, Ask)>,
        proof: Option<usize>,
    },
}

impl Request {
    fn describe(&self) -> Value {
        match self {
            Self::Follow { name, query } => json!({ "mode": "follow", "name": name, "query": query }),
            Self::Read { name, hours, .. } => json!({ "mode": "read", "name": name, "first": hours.first() }),
        }
    }
}

fn encode(out: &mut impl Write, value: &impl Serialize) -> io::Result<()> {
    serde_json::to_writer(&mut *out, value)?;
    writeln!(out)
}

struct UiRequests {
    child: Child,
    input: Option<BufWriter<ChildStdin>>,
    output: Option<BufReader<ChildStdout>>,
}

impl UiRequests {
    fn open() -> io::Result<Self> {
        let mut command = Command::new("node");
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        let mut child = command
            .args([
                "--disable-warning=ExperimentalWarning",
                "tools/e2e/nativeBenchViews.mjs",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()?;
        let input = Some(BufWriter::new(child.stdin.take().expect("piped stdin")));
        let output = Some(BufReader::new(child.stdout.take().expect("piped stdout")));
        Ok(Self {
            child,
            input,
            output,
        })
    }

    fn send(&mut self, value: &Value) -> io::Result<()> {
        let input = self.input.as_mut().expect("piped stdin");
        encode(input, value)?;
        input.flush()
    }

    fn receive(&mut self) -> io::Result<Value> {
        let mut line = String::new();
        let output = self.output.as_mut().expect("piped stdout");
        if output.read_line(&mut line)? == 0 {
            return Err(io::Error::other(
                "The UI request builder closed its reply stream",
            ));
        }
        Ok(serde_json::from_str(&line)?)
    }

    fn plans(
        &mut self,
        source: &Arc<Source>,
        load: &LoadContext,
        context: &Value,
    ) -> io::Result<Vec<Plan>> {
        self.send(context)?;
        loop {
            let mut value = self.receive()?;
            if let Some(reason) = value["reason"].as_str() {
                return Err(io::Error::other(reason.to_owned()));
            }
            if let Some(views) = value
                .as_object_mut()
                .and_then(|fields| fields.remove("views"))
            {
                return Ok(serde_json::from_value(views)?);
            }
            let ask: Ask = serde_json::from_value(value)?;
            let reply = match dashboard::answer::answer(
                source,
                &ask.product,
                &ask.params,
                load,
                &false.into(),
            ) {
                Ok(data) => json!({ "data": data }),
                Err(error) => json!({ "reason": error.to_string() }),
            };
            self.send(&reply)?;
        }
    }

    fn through(&mut self, proof: usize, hour: i32, body: &[u8]) -> io::Result<i32> {
        let input = self.input.as_mut().expect("piped stdin");
        write!(input, r#"{{"proof":{proof},"hour":{hour},"data":"#)?;
        input.write_all(body)?;
        writeln!(input, "}}")?;
        input.flush()?;
        let reply = self.receive()?;
        if let Some(reason) = reply["reason"].as_str() {
            return Err(io::Error::other(reason.to_owned()));
        }
        Ok(serde_json::from_value(reply)?)
    }
}

impl Drop for UiRequests {
    fn drop(&mut self) {
        drop(self.input.take());
        drop(self.output.take());
        let _ = self.child.wait();
    }
}

struct Panel {
    read: ProductReader,
    delivery: Delivery,
    groups: Vec<(GroupKey, u32)>,
    input: Option<Arc<PanelData>>,
    delivered: Option<Arc<PanelData>>,
    confirmed: Option<(u64, i32)>,
}

struct Direct {
    hours: BTreeMap<i32, Ask>,
    at: Option<i32>,
    proof: Option<usize>,
    through: Option<i32>,
}

enum Reader {
    Panel(Box<Panel>),
    Direct(Direct),
}

struct View {
    follow: Follow,
    hook: Hook,
    message: Arc<Mutex<(StepStats, Option<Message>)>>,
    following: bool,
    uncounted_follow: bool,
    next_live: u32,
    readers: Vec<(u32, Reader)>,
    slots: BTreeMap<u32, String>,
    held: BTreeMap<u32, blake3::Hash>,
    reasons: BTreeMap<u32, String>,
}

impl View {
    fn open(
        source: &Arc<Source>,
        load: &LoadContext,
        identity: &Identity,
        plan: Plan,
        route: u32,
    ) -> (Self, Vec<String>) {
        let follow = Follow::with_route(0, route);
        let mut reasons = plan.reasons;
        let mut readers = Vec::new();
        let mut slots = BTreeMap::new();
        let mut following = false;
        let mut uncounted_follow = false;
        for (index, request) in plan.requests.into_iter().enumerate() {
            let id = index as u32 + 1;
            let (name, reader) = match request {
                Request::Read { name, hours, proof } => (
                    name,
                    Ok(Some(Reader::Direct(Direct {
                        hours: hours.into_iter().collect(),
                        at: None,
                        proof,
                        through: None,
                    }))),
                ),
                Request::Follow { name, query } if query.get("panel").is_some() => {
                    let panel = dashboard::panel::open(source, load, &query).and_then(|panel| {
                        let exact = panel.exact().ok_or("The panel has no stepped reader")?;
                        Ok(Some(Reader::Panel(Box::new(Panel {
                            read: exact.advance.clone(),
                            delivery: Delivery::default(),
                            groups: Vec::new(),
                            input: None,
                            delivered: None,
                            confirmed: None,
                        }))))
                    });
                    (name, panel)
                }
                Request::Follow { name, query } => {
                    let parsed = match dashboard_players::products::player_apm::head::head_of(
                        source, &query,
                    ) {
                        Some(head) => head.map(Query::Head),
                        None => dashboard_follow::parse(&query, identity)
                            .map_err(|error| error.to_string()),
                    };
                    (
                        name,
                        parsed.and_then(|query| {
                            let counted = matches!(&query, Query::Series { series, measure, mode: SeriesMode::Top { .. }, .. } if dashboard_declare::source(series.table, measure).is_ok());
                            follow
                                .follow(id, query)
                                .map(|()| {
                                    uncounted_follow |= !counted;
                                    None
                                })
                                .map_err(|error| error.to_string())
                        }),
                    )
                }
            };
            match reader {
                Ok(reader) => {
                    slots.insert(id, name);
                    if let Some(reader) = reader {
                        readers.push((id, reader));
                    } else {
                        following = true;
                    }
                }
                Err(reason) => reasons.push(format!("{name}: {reason}")),
            }
        }
        let message = Arc::new(Mutex::new((StepStats::default(), None)));
        let sink = Arc::clone(&message);
        let hook = follow.hook(Box::new(move |stats, message| {
            *sink.lock().expect("follow sink") = (*stats, message);
        }));
        (
            Self {
                follow,
                hook,
                message,
                following,
                uncounted_follow,
                next_live: 0,
                readers,
                slots,
                held: BTreeMap::new(),
                reasons: BTreeMap::new(),
            },
            reasons,
        )
    }

    fn step(
        &mut self,
        context: &StepContext<'_, '_>,
        declared: &mut Vec<Declared>,
        ui: &mut UiRequests,
    ) -> io::Result<Sample> {
        let mut counts = Counts::default();
        let mut received = Vec::new();
        let mut limited = Vec::new();
        let mut panels = None;
        let mut panel_through = None;
        let hour = context.step.last();
        let has_live = self
            .readers
            .iter()
            .any(|(_, reader)| matches!(reader, Reader::Panel(panel) if !panel.groups.is_empty()));
        if self.following || has_live {
            let ((), service) = measured(|| (self.hook)(context.step, context.lanes));
            counts.add(service);
            let (stats, packet) = std::mem::take(&mut *self.message.lock().expect("follow sink"));
            counts.rows += stats.rows_read;
            if let Some(packet) = packet {
                counts.bytes += packet.bytes.len() as u64;
                let decoded = dashboard_follow::decode(&packet.bytes)?;
                limited = decoded.limited;
                limited.sort_unstable();
                for (id, hour, body) in decoded.answers {
                    let reason = if body.first() == Some(&0) {
                        dashboard_follow::Answer::parse(body)?.not_recorded
                    } else {
                        None
                    };
                    received.push(Reply::new(id, hour, body, reason));
                }
            }
        }
        for (id, reader) in &mut self.readers {
            let (packet, service) = measured(|| match reader {
                Reader::Panel(panel) => {
                    let batch = panels.get_or_insert_with(|| {
                        dated::ProductBatch::new((
                            0,
                            self.follow.route(),
                            context.step.epoch() as u32,
                        ))
                    });
                    panel.step(
                        &self.follow,
                        *id,
                        &mut self.next_live,
                        context,
                        declared,
                        batch,
                    )
                }
                Reader::Direct(direct) => Ok(direct.step(context)),
            });
            counts.add(service);
            let packet = packet?;
            counts.rows += packet.rows;
            counts.bytes += packet.bytes;
            if let Some(payload) = packet.payload {
                if let Reader::Direct(direct) = reader {
                    counts.exact_requests += 1;
                    if let Payload::Direct(body) = &payload
                        && let (Some(proof), Some(at)) = (direct.proof, direct.at)
                    {
                        direct.through = Some(ui.through(proof, at, body)?);
                    }
                }
                received.push(payload.reply(*id, hour));
            }
        }

        if let Some(batch) = panels {
            let (bytes, service) = measured(|| batch.finish(Some(hour)));
            counts.add(service);
            counts.bytes += bytes.len() as u64;
            for record in dated::records(&bytes)? {
                if record.kind == dated::COMPLETED {
                    panel_through = Some(record.hour);
                    continue;
                }
                let reason = (record.kind == dated::REFUSED)
                    .then(|| String::from_utf8_lossy(record.body).into_owned());
                let mut reply = Reply::new(record.id, record.hour, record.body, reason);
                reply.kind = Some(record.kind);
                received.push(reply);
            }
        }

        let mut changed = false;
        for reply in &received {
            if reply
                .kind
                .is_some_and(|kind| matches!(kind, dated::AGAIN | dated::STANDS))
            {
                continue;
            }
            changed |= self.held.insert(reply.id, reply.hash) != Some(reply.hash);
            if let Some(reason) = &reply.reason {
                self.reasons.insert(reply.id, reason.clone());
            } else {
                self.reasons.remove(&reply.id);
            }
        }
        let refusals = self
            .reasons
            .iter()
            .map(|(id, reason)| {
                (
                    self.slots
                        .get(id)
                        .cloned()
                        .unwrap_or_else(|| format!("live cells {id}")),
                    reason.clone(),
                )
            })
            .collect();
        Ok(Sample {
            counts,
            changed: changed && !context.step.reset(),
            replies: received,
            limited,
            panel_through,
            rows_complete: !self.uncounted_follow && !has_live,
            refusals,
        })
    }
}

struct StepContext<'a, 'lane> {
    source: &'a Arc<Source>,
    load: &'a LoadContext,
    step: &'a Step,
    lanes: &'a Lanes<'lane>,
    sampled_hour: Option<i32>,
    cancelled: &'a AtomicBool,
}

#[derive(Default)]
struct Packet {
    rows: u64,
    bytes: u64,
    payload: Option<Payload>,
}

enum Payload {
    Direct(Vec<u8>),
    Refused(String),
}

impl Payload {
    fn reply(self, id: u32, hour: i32) -> Reply {
        match self {
            Self::Direct(bytes) => Reply::new(id, hour, &bytes, None),
            Self::Refused(reason) => {
                let hash = Reply::digest(reason.as_bytes(), true);
                Reply {
                    id,
                    hour,
                    hash,
                    kind: None,
                    reason: Some(reason),
                }
            }
        }
    }
}

impl Panel {
    fn step(
        &mut self,
        follow: &Follow,
        id: u32,
        next_live: &mut u32,
        context: &StepContext<'_, '_>,
        declared: &mut Vec<Declared>,
        batch: &mut dated::ProductBatch,
    ) -> io::Result<Packet> {
        let (ready, wait) = mpsc::channel();
        let wake: Wake = Arc::new(move || {
            let _ = ready.send(());
        });
        let reset = context.step.reset();
        let hour = context.step.last();
        let read = || {
            (self.read)(
                hour,
                reset,
                context.cancelled,
                Some(ProductStep {
                    step: context.step,
                    lanes: context.lanes,
                    held: None,
                    wake: Some(&wake),
                }),
            )
        };
        let mut reading = read();
        let mut rows = reading.rows;
        while reading.pending {
            if wait.recv_timeout(Duration::from_secs(30)).is_err() {
                return Err(io::Error::other(format!(
                    "Panel remained pending at hour {hour}"
                )));
            }
            reading = read();
            rows += reading.rows;
        }
        declared.append(&mut reading.lanes);
        let unchanged = !reset
            && self
                .input
                .as_ref()
                .is_some_and(|previous| Arc::ptr_eq(previous, &reading.value));
        let value = if unchanged && let Some(value) = &self.delivered {
            Arc::clone(value)
        } else {
            let live_id = || {
                let id = CELLS | *next_live;
                *next_live += 1;
                id
            };
            sync_live(
                follow,
                live_id,
                &mut self.groups,
                self.delivered.as_ref(),
                &reading.value,
                reading.live.as_deref(),
                hour,
                reset,
                reading.fresh,
                declared,
            )
        };
        let epoch = context.step.epoch();
        let unread_from = self
            .confirmed
            .filter(|(seen_epoch, _)| !reset && *seen_epoch == epoch)
            .map(|(_, through)| through.saturating_add(1));
        let from = reading.from.min(hour);
        let at = if self.groups.is_empty() {
            unread_from.map_or(from, |next| from.max(next)).min(hour)
        } else {
            hour
        };
        self.input = Some(reading.value);
        if let Some((kind, body)) = self.delivery.update(reset, &value) {
            batch.changed(id, at, kind, body);
        } else if unread_from.is_some_and(|next| from > next) {
            batch.changed(id, at, dated::AGAIN, &[]);
        }
        self.confirmed = Some((epoch, hour));
        self.delivery.commit();
        self.delivered = Some(value);
        Ok(Packet {
            rows,
            ..Packet::default()
        })
    }
}

impl Direct {
    fn step(&mut self, context: &StepContext<'_, '_>) -> Packet {
        let Some(hour) = context.sampled_hour else {
            return Packet::default();
        };
        if self.through.is_some_and(|through| hour <= through) {
            return Packet::default();
        }
        let Some(ask) = self.hours.get(&hour) else {
            return Packet::default();
        };
        if self.at.and_then(|at| self.hours.get(&at)) == Some(ask) {
            return Packet::default();
        }
        self.at = Some(hour);
        match dashboard::answer::answer(
            context.source,
            &ask.product,
            &ask.params,
            context.load,
            context.cancelled,
        ) {
            Ok(data) => {
                let bytes = serde_json::to_vec(&data).expect("JSON product reply");
                Packet {
                    bytes: bytes.len() as u64,
                    payload: Some(Payload::Direct(bytes)),
                    ..Packet::default()
                }
            }
            Err(error) => {
                let reason = error.to_string();
                Packet {
                    bytes: reason.len() as u64,
                    payload: Some(Payload::Refused(reason)),
                    ..Packet::default()
                }
            }
        }
    }
}

#[derive(Default, Serialize)]
struct Counts {
    cpu_ns: u64,
    cycles: u64,
    rows: u64,
    opens: u64,
    restores: u64,
    bytes: u64,
    exact_requests: u64,
}

impl Counts {
    fn add(&mut self, other: Self) {
        self.cpu_ns += other.cpu_ns;
        self.cycles += other.cycles;
        self.rows += other.rows;
        self.opens += other.opens;
        self.restores += other.restores;
    }
}

fn measured<T>(work: impl FnOnce() -> T) -> (T, Counts) {
    let ns = thread_cpu_ns();
    let cycles = thread_cycles();
    let ((value, rows), reads) = replay_recording::cells::count_reads(|| at_hour::counted(work));
    let mut counts = Counts {
        cpu_ns: thread_cpu_ns() - ns,
        cycles: thread_cycles() - cycles,
        rows,
        ..Counts::default()
    };
    for table in reads.values() {
        counts.opens += table.opened;
        counts.restores += table.restores;
    }
    (value, counts)
}

#[derive(Serialize)]
struct Reply {
    id: u32,
    hour: i32,
    #[serde(serialize_with = "write_hash")]
    hash: blake3::Hash,
    #[serde(skip_serializing_if = "Option::is_none")]
    kind: Option<u8>,
    reason: Option<String>,
}

fn write_hash<S: serde::Serializer>(hash: &blake3::Hash, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(hash.to_hex().as_str())
}

impl Reply {
    fn new(id: u32, hour: i32, body: &[u8], reason: Option<String>) -> Self {
        let hash = Self::digest(body, reason.is_some());
        Self {
            id,
            hour,
            hash,
            kind: None,
            reason,
        }
    }

    fn digest(body: &[u8], refused: bool) -> blake3::Hash {
        let mut hash = blake3::Hasher::new();
        hash.update(&[u8::from(refused)]);
        hash.update(body);
        hash.finalize()
    }
}

#[derive(Serialize)]
struct Sample {
    counts: Counts,
    changed: bool,
    replies: Vec<Reply>,
    limited: Vec<(u32, i32)>,
    #[serde(skip_serializing_if = "Option::is_none")]
    panel_through: Option<i32>,
    rows_complete: bool,
    refusals: BTreeMap<String, String>,
}

struct Options {
    quanta: Vec<i32>,
    steps: i32,
    branch: Option<String>,
    start: Option<i32>,
    views: Vec<String>,
}

impl Options {
    fn parse(flags: &[String]) -> io::Result<Self> {
        let mut out = Self {
            quanta: vec![24, 3, 1],
            steps: 48,
            branch: None,
            start: None,
            views: Vec::new(),
        };
        let mut args = flags.iter();
        while let Some(flag) = args.next() {
            let value = args
                .next()
                .ok_or_else(|| io::Error::other(format!("Missing value for {flag}")))?;
            let integer = || value.parse::<i32>().map_err(io::Error::other);
            match flag.as_str() {
                "--quantum" => {
                    let quantum = integer()?;
                    if ![24, 3, 1].contains(&quantum) {
                        return Err(io::Error::other("--quantum must be 24, 3, or 1"));
                    }
                    out.quanta = vec![quantum];
                }
                "--steps" => {
                    out.steps = integer()?;
                    if out.steps < 1 {
                        return Err(io::Error::other("--steps must be positive"));
                    }
                }
                "--branch" => out.branch = Some(value.clone()),
                "--start" => out.start = Some(integer()?),
                "--view" => out.views.push(value.clone()),
                _ => return Err(io::Error::other(format!("Unknown option {flag}"))),
            }
        }
        Ok(out)
    }
}

pub(super) fn run(
    recording: &str,
    game: &str,
    user: &str,
    cache: &str,
    tag: &str,
    flags: &[String],
) -> io::Result<()> {
    if !cfg!(windows) {
        return Err(io::Error::other(
            "The all-view benchmark uses Windows thread CPU counters",
        ));
    }
    let options = Options::parse(flags)?;
    let path = Path::new(recording);
    let source = Arc::new(Source::open(
        SourceKey {
            source_id: "native-bench".into(),
            revision: dashboard_core::protocol::replay_revision(path)?,
        },
        path.into(),
        &false.into(),
    )?);
    engine_executable::executable_tables::enter(source.tables());
    let load = LoadContext {
        content: Some(ContentOptions {
            options: content_reader_layers::ResolveOptions {
                game_dir: game.into(),
                user_dir: user.into(),
                cache_dir: cache.into(),
            },
            allow_unverified: false,
            presentation: Default::default(),
        }),
    };
    let metadata = source.metadata().map_err(io::Error::other)?;
    let branch = match options.branch {
        Some(id) => metadata
            .branches
            .iter()
            .find(|branch| branch.id == id)
            .ok_or_else(|| io::Error::other(format!("Unknown branch {id}")))?,
        None => metadata
            .branches
            .iter()
            .find(|branch| branch.main)
            .ok_or_else(|| io::Error::other("No main branch"))?,
    };
    let branch_id = branch.id.parse().map_err(io::Error::other)?;
    let q = dashboard_base::queries::query(&source, branch_id)?;
    let identity = Identity {
        timeline: q.timeline.clone(),
        holders: q.holders.clone(),
    };
    let start = options
        .start
        .unwrap_or(branch.first_hour + (branch.last_hour - branch.first_hour) * 3 / 10);
    let span = q
        .timeline
        .stretches
        .iter()
        .find(|span| span.first <= start && start <= span.last)
        .ok_or_else(|| {
            io::Error::other(format!(
                "Hour {start} is outside a recorded stretch of branch {branch_id}"
            ))
        })?;
    let pages = WorldPages::shared(source.replay_ref().recording())?;
    let run = pages
        .run_of(span.run)
        .ok_or_else(|| io::Error::other("The selected stretch is not readable by the playhead"))?;
    let started = Instant::now();
    let mut out = BufWriter::new(io::stdout().lock());
    for quantum in options.quanta {
        let origin = pages.pieces()[run].first;
        let first = start
            .checked_add(
                ((i64::from(origin) - 1 - i64::from(start)).rem_euclid(i64::from(quantum))) as i32,
            )
            .ok_or_else(|| io::Error::other("Aligned start exceeds the hour range"))?;
        let steps = options.steps.min((span.last - first) / quantum);
        if steps < 1 {
            return Err(io::Error::other(format!(
                "No complete {quantum}-hour step in the recorded stretch ending at {}",
                span.last
            )));
        }
        let last = first + quantum * steps;
        let piece = Piece { run, first, last };
        let mut ui = UiRequests::open()?;
        let prepared = source.in_scope(|| {
            ui.plans(
                &source,
                &load,
                &json!({
                    "metadata": metadata, "branch": branch.id, "country": tag,
                    "first": first, "last": last, "quantum": quantum, "views": options.views,
                }),
            )
        })?;
        encode(
            &mut out,
            &json!({
                "event": "run", "trace_version": 2, "recording": recording, "campaign": metadata.campaign_key,
                "recording_bytes": path.metadata()?.len(), "format": metadata.format, "branch": branch.id,
                "country": tag, "requested_start": start, "first": first, "last": last, "quantum": quantum,
                "step_threads": 2, "fetch_threads": 1, "service_threads": 1,
            }),
        )?;
        let rig = Rig {
            source: Arc::clone(&source),
            load: LoadContext {
                content: load.content.clone(),
            },
            pages: Arc::clone(&pages),
        };
        rig.measure(&identity, piece, quantum, prepared, ui, &mut out)?;
        out.flush()?;
    }
    eprintln!(
        "All-view benchmark finished in {:.1}s (one service stepper, one step helper, one fetch worker)",
        started.elapsed().as_secs_f64()
    );
    Ok(())
}

struct Rig {
    source: Arc<Source>,
    load: LoadContext,
    pages: Arc<WorldPages>,
}

impl Rig {
    fn measure(
        self,
        identity: &Identity,
        piece: Piece,
        quantum: i32,
        plans: Vec<Plan>,
        mut ui: UiRequests,
        out: &mut impl Write,
    ) -> io::Result<()> {
        let Piece { first, last, .. } = piece;
        let mut views = Vec::with_capacity(plans.len());
        let mut declared = BTreeMap::new();
        for (index, plan) in plans.into_iter().enumerate() {
            let key = json!({ "kind": plan.kind, "view": plan.view, "page": plan.page, "viewport": plan.viewport, "country": plan.country });
            let requests: Vec<_> = plan.requests.iter().map(Request::describe).collect();
            let (view, reasons) = self.source.in_scope(|| {
                View::open(&self.source, &self.load, identity, plan, index as u32 + 1)
            });
            widen(&mut declared, &view.follow.declared()?);
            encode(
                out,
                &json!({
                    "event": "view", "quantum": quantum, "index": index, "key": key,
                    "slots": view.slots, "reasons": reasons, "requests": requests,
                }),
            )?;
            views.push(view);
        }
        out.flush()?;
        eprintln!(
            "{} registered views, {quantum}-hour steps {first}..{last}",
            views.len()
        );
        let follows: Vec<_> = views.iter().map(|view| view.follow.clone()).collect();
        let (send, recv) = mpsc::channel();
        let pages = self.pages;
        let (source, load) = (self.source, self.load);
        let cancelled = AtomicBool::new(false);
        let mut farthest = i32::MIN;
        let mut next = first;
        let hook: Hook = Box::new(move |step, lanes| {
            let hour = step.last();
            if hour <= farthest || hour < first || hour > last {
                return;
            }
            farthest = hour;
            let sampled_hour = (hour >= next).then_some(next);
            let mut new_lanes = Vec::new();
            let context = StepContext {
                source: &source,
                load: &load,
                step,
                lanes,
                sampled_hour,
                cancelled: &cancelled,
            };
            let ns = thread_cpu_ns();
            let cycles = thread_cycles();
            let samples = source.in_scope(|| {
                views
                    .iter_mut()
                    .map(|view| {
                        if view.slots.is_empty() {
                            return None;
                        }
                        match view.step(&context, &mut new_lanes, &mut ui) {
                            Ok(sample) => Some(Ok(sample)),
                            Err(error) => {
                                view.slots.clear();
                                Some(Err(error.to_string()))
                            }
                        }
                    })
                    .collect::<Vec<_>>()
            });
            let cpu_span = (thread_cpu_ns() - ns, thread_cycles() - cycles);
            if sampled_hour.is_some() {
                next = next.saturating_add(quantum);
            }
            let _ = send.send((
                (hour, sampled_hour, step.reset(), samples, cpu_span),
                new_lanes,
            ));
        });
        let timer = Instant::now();
        let mut cpu_ns = 0;
        let mut cycles = 0;
        let mut ticks = 0;
        let setup = |ph: &Playhead| {
            for follow in follows {
                follow.bind(|_, _| Ok(()), ph.log_reads())?;
            }
            Ok(())
        };
        drive(
            pages,
            piece,
            quantum,
            declared,
            hook,
            recv,
            setup,
            |(hour, sample, reset, views, (ns, ticks_cpu))| {
                cpu_ns += ns;
                cycles += ticks_cpu;
                if sample.is_some() {
                    ticks += 1;
                }
                encode(
                    out,
                    &json!({
                        "event": "step", "quantum": quantum, "hour": hour, "sample": sample, "seed": reset, "views": views,
                    }),
                )?;
                Ok(Some(hour))
            },
        )?;
        if ticks != (last - first) / quantum + 1 {
            return Err(io::Error::other(format!(
                "Playhead reached {last} but did not sample every {quantum}-hour step"
            )));
        }
        let elapsed = timer.elapsed().as_secs_f64();
        encode(
            out,
            &json!({ "event": "end", "quantum": quantum, "elapsed_seconds": elapsed, "service_cpu_ns": cpu_ns, "service_cycles": cycles, "samples": ticks - 1 }),
        )?;
        eprintln!("{quantum}-hour run took {elapsed:.1}s");
        Ok(())
    }
}
