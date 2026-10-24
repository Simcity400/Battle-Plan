//! Offline evidence for the integrated prepared slice, using production oracles.
//!
//! Run through tools/accept-rebuild-slice.ps1 to isolate all process caches.

use dashboard::{
    Source, SourceKey, prepared,
    prepared_focus::{FocusArt, FocusArtMode, FocusArtParams},
    products::{
        ContentOptions, LoadContext, Product,
        country_measure::{CountryMeasure, FixedRun, Measure, MeasureParams, MeasureSeries},
        equipment_produced::{
            EquipmentProduced, EquipmentProducedReply, ProducedCoded, ProducedCount,
            ProducedGrouping, ProducedMeasure, ProducedParams,
        },
        focus_artwork::{FocusArtwork, FocusArtworkParams},
        national_focus::FocusCache,
    },
    transport::ProductRequest,
};
use replay::container::SeekResult;
use replay_data::{rows::TableId, rows::tables};
use replay_frame::frame::TableView;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::{self, BufWriter, Write},
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[path = "dashboard-acceptance/readiness_probe.rs"]
mod readiness_probe;

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

/// An explicit request set makes comparisons repeatable across builds. Empty
/// hours use branch ends/midpoint and indexed block edges, never random samples.
#[derive(Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Requests {
    branch_id: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    hours: Vec<i32>,
    #[serde(default)]
    production: Vec<ProducedParams>,
    #[serde(default)]
    viewports: Vec<Viewport>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Viewport {
    from: f64,
    to: f64,
    width: u32,
}

struct Options {
    recording: PathBuf,
    prefix: Option<PathBuf>,
    evidence: PathBuf,
    game: Option<PathBuf>,
    user: Option<PathBuf>,
    requests: Option<PathBuf>,
    repetitions: usize,
    allow_unverified: bool,
    prepare_only: bool,
    progressive: Option<String>,
    prefix_blocks: usize,
    measurements: Option<PathBuf>,
    readiness: bool,
    readiness_limit: Duration,
    readiness_table: Option<PathBuf>,
    readiness_cold_content: bool,
}

fn options() -> io::Result<Options> {
    let mut args = std::env::args_os().skip(1);
    let mut recording = None;
    let mut prefix = None;
    let mut evidence = None;
    let mut game = std::env::var_os("BATTLE_PLAN_GAME_DIR").or_else(|| std::env::var_os("HOI4_GAME_DIR")).map(PathBuf::from);
    let mut user = std::env::var_os("BATTLE_PLAN_USER_DIR").or_else(|| std::env::var_os("HOI4_USER_DIR")).map(PathBuf::from);
    let mut requests = None;
    let mut repetitions = 20;
    let mut allow_unverified = false;
    let mut prepare_only = false;
    let mut progressive = None;
    let mut prefix_blocks = 13;
    let mut measurements = None;
    let mut readiness = false;
    let mut readiness_limit = Duration::from_secs(20 * 60);
    let mut readiness_table = None;
    let mut readiness_cold_content = false;
    while let Some(arg) = args.next() {
        if arg == "--readiness" {
            readiness = true;
            continue;
        }
        if arg == "--readiness-cold-content" {
            readiness_cold_content = true;
            continue;
        }
        if arg == "--allow-unverified" {
            allow_unverified = true;
            continue;
        }
        if arg == "--prepare-only" {
            prepare_only = true;
            continue;
        }
        let value = args.next().ok_or_else(|| invalid("Missing option value"))?;
        match arg.to_str() {
            Some("--progressive") => {
                let mode = value.to_string_lossy().into_owned();
                if !["setup", "rough", "zoom"].contains(&mode.as_str()) {
                    return Err(invalid("Progressive mode must be setup, rough or zoom"));
                }
                progressive = Some(mode);
            }
            Some("--measurements") => measurements = Some(PathBuf::from(value)),
            Some("--readiness-table") => readiness_table = Some(PathBuf::from(value)),
            Some("--readiness-limit") => {
                readiness_limit = value
                    .to_str()
                    .and_then(|value| value.parse().ok())
                    .filter(|value| *value > 0)
                    .map(Duration::from_secs)
                    .ok_or_else(|| invalid("Readiness limit must be positive seconds"))?;
            }
            Some("--prefix-blocks") => {
                prefix_blocks = value
                    .to_str()
                    .and_then(|value| value.parse().ok())
                    .filter(|value| *value > 0)
                    .ok_or_else(|| invalid("Prefix blocks must be positive"))?;
            }
            Some("--recording") => recording = Some(PathBuf::from(value)),
            Some("--prefix") => prefix = Some(PathBuf::from(value)),
            Some("--evidence") => evidence = Some(PathBuf::from(value)),
            Some("--game-dir") => game = Some(PathBuf::from(value)),
            Some("--user-dir") => user = Some(PathBuf::from(value)),
            Some("--requests") => requests = Some(PathBuf::from(value)),
            Some("--repetitions") => {
                repetitions = value
                    .to_str()
                    .and_then(|value| value.parse().ok())
                    .filter(|value| *value > 0)
                    .ok_or_else(|| invalid("Repetitions must be a positive integer"))?;
            }
            _ => {
                return Err(invalid(format!(
                    "Unknown option: {}",
                    arg.to_string_lossy()
                )));
            }
        }
    }
    Ok(Options {
        recording: recording.ok_or_else(|| invalid("Supply --recording"))?,
        prefix,
        evidence: evidence.ok_or_else(|| invalid("Supply --evidence outside Git"))?,
        game,
        user,
        requests,
        repetitions,
        allow_unverified,
        prepare_only,
        progressive,
        prefix_blocks,
        measurements,
        readiness,
        readiness_limit,
        readiness_table,
        readiness_cold_content,
    })
}

fn isolated(evidence: &Path) -> io::Result<PathBuf> {
    fs::create_dir_all(evidence)?;
    let evidence = fs::canonicalize(evidence)?;
    for start in [std::env::current_dir()?, std::env::current_exe()?] {
        for ancestor in start.ancestors() {
            if ancestor.join(".git").exists() && evidence.starts_with(ancestor) {
                return Err(invalid(
                    "Acceptance evidence must be outside the Git worktree",
                ));
            }
        }
    }
    for (name, leaf) in [("LOCALAPPDATA", "local"), ("APPDATA", "roaming")] {
        let expected = evidence.join("cache").join(leaf);
        fs::create_dir_all(&expected)?;
        let actual = std::env::var_os(name).ok_or_else(|| {
            invalid(format!(
                "{name} is unavailable; use the acceptance launcher"
            ))
        })?;
        if fs::canonicalize(actual)? != fs::canonicalize(&expected)? {
            return Err(invalid(format!(
                "{name} must point to {}; use the acceptance launcher",
                expected.display()
            )));
        }
    }
    Ok(evidence)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Timing {
    label: String,
    product: String,
    first_ms: f64,
    p50_ms: f64,
    p95_ms: f64,
    p99_ms: f64,
    samples: usize,
    reply_bytes: usize,
}

struct Evidence {
    path: PathBuf,
    comparisons: u64,
    differences: u64,
    errors: u64,
    skipped: Vec<String>,
    timings: Vec<Timing>,
    differences_file: BufWriter<File>,
    replies: BufWriter<File>,
    phases: BufWriter<File>,
}

impl Evidence {
    fn new(path: PathBuf) -> io::Result<Self> {
        Ok(Self {
            differences_file: BufWriter::new(File::create(path.join("differences.jsonl"))?),
            replies: BufWriter::new(File::create(path.join("replies.jsonl"))?),
            phases: BufWriter::new(File::create(path.join("phases.jsonl"))?),
            path,
            comparisons: 0,
            differences: 0,
            errors: 0,
            skipped: Vec::new(),
            timings: Vec::new(),
        })
    }
    fn row(writer: &mut impl Write, value: &Value) -> io::Result<()> {
        serde_json::to_writer(&mut *writer, value)?;
        writer.write_all(b"\n")
    }
    fn phase(&mut self, name: &str) -> io::Result<()> {
        let at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(io::Error::other)?;
        Self::row(
            &mut self.phases,
            &json!({"name": name, "unixMillis": at.as_millis().to_string()}),
        )?;
        self.phases.flush()
    }
    fn compare(&mut self, label: &str, expected: Value, actual: Value) -> io::Result<()> {
        self.comparisons += 1;
        Self::row(
            &mut self.replies,
            &json!({
                "label": label, "expected": expected, "actual": actual
            }),
        )?;
        self.diff(label, "", &expected, &actual)
    }
    fn diff(
        &mut self,
        label: &str,
        pointer: &str,
        expected: &Value,
        actual: &Value,
    ) -> io::Result<()> {
        if expected == actual {
            return Ok(());
        }
        match (expected, actual) {
            (Value::Object(left), Value::Object(right)) => {
                let keys: BTreeSet<_> = left.keys().chain(right.keys()).collect();
                for key in keys {
                    let next = format!("{pointer}/{}", key.replace('~', "~0").replace('/', "~1"));
                    match (left.get(key), right.get(key)) {
                        (Some(left), Some(right)) => self.diff(label, &next, left, right)?,
                        (left, right) => self.difference(label, &next, left, right)?,
                    }
                }
            }
            (Value::Array(left), Value::Array(right)) => {
                for index in 0..left.len().max(right.len()) {
                    let next = format!("{pointer}/{index}");
                    match (left.get(index), right.get(index)) {
                        (Some(left), Some(right)) => self.diff(label, &next, left, right)?,
                        (left, right) => self.difference(label, &next, left, right)?,
                    }
                }
            }
            _ => self.difference(label, pointer, Some(expected), Some(actual))?,
        }
        Ok(())
    }
    fn difference(
        &mut self,
        label: &str,
        pointer: &str,
        expected: Option<&Value>,
        actual: Option<&Value>,
    ) -> io::Result<()> {
        self.differences += 1;
        Self::row(
            &mut self.differences_file,
            &json!({
                "label": label, "pointer": pointer,
                "expectedPresent": expected.is_some(), "expected": expected,
                "actualPresent": actual.is_some(), "actual": actual
            }),
        )
    }
    fn error(&mut self, label: &str, error: &io::Error) -> io::Result<()> {
        self.errors += 1;
        Self::row(
            &mut self.differences_file,
            &json!({
                "label": label, "error": error.to_string(), "kind": format!("{:?}", error.kind())
            }),
        )
    }
    /// Includes coverage, stamp/manifest checks and JSON serialization in the
    /// actual host-direct dispatch. Parsing and oracle work are outside timing.
    fn query(
        &mut self,
        source: &Source,
        context: &LoadContext,
        product: &str,
        params: Value,
        label: &str,
        repetitions: usize,
    ) -> io::Result<Value> {
        let request = ProductRequest {
            source_id: source.key().source_id.clone(),
            revision: source.key().revision.clone(),
            product: product.into(),
            params,
        };
        let cancelled = AtomicBool::new(false);
        let started = Instant::now();
        let reply = prepared::query::query(source, &request, context.content.as_ref(), &cancelled)?;
        let first_ms = started.elapsed().as_secs_f64() * 1000.0;
        let value: Value = serde_json::from_str(&reply)?;
        let mut samples = Vec::with_capacity(repetitions);
        for _ in 0..repetitions {
            let started = Instant::now();
            let repeated =
                prepared::query::query(source, &request, context.content.as_ref(), &cancelled)?;
            samples.push(started.elapsed().as_secs_f64() * 1000.0);
            if repeated != reply {
                self.compare(
                    &format!("{label}/repeat"),
                    value.clone(),
                    serde_json::from_str(&repeated)?,
                )?;
            }
        }
        samples.sort_by(f64::total_cmp);
        let percentile =
            |percent: usize| samples[(samples.len() * percent).div_ceil(100).saturating_sub(1)];
        self.timings.push(Timing {
            label: label.into(),
            product: product.into(),
            first_ms,
            p50_ms: percentile(50),
            p95_ms: percentile(95),
            p99_ms: percentile(99),
            samples: repetitions,
            reply_bytes: reply.len(),
        });
        Self::row(
            &mut self.replies,
            &json!({"label": label, "request": request, "reply": value}),
        )?;
        self.compare(&format!("{label}/envelope"), json!({
            "sourceId": request.source_id, "revision": request.revision, "product": product
        }), json!({
            "sourceId": value["sourceId"], "revision": value["revision"], "product": value["product"]
        }))?;
        Ok(value["data"].clone())
    }
}

/// Preparation binds to the recording's current size and modification time.
fn open(path: &Path) -> io::Result<Source> {
    Source::open(
        SourceKey {
            source_id: "slice-acceptance".into(),
            revision: dashboard::protocol::replay_revision(path)?,
        },
        path.to_owned(),
        &AtomicBool::new(false),
    )
}

fn prepare(
    source: &Source,
    context: &LoadContext,
    evidence: &mut Evidence,
    label: &str,
) -> io::Result<Value> {
    evidence.phase(&format!("prepare/{label}"))?;
    let mut publications = Vec::new();
    let samples = std::env::var_os("BATTLE_PLAN_ACCEPTANCE_MEMORY_SAMPLES").or_else(|| std::env::var_os("HOI4_ACCEPTANCE_MEMORY_SAMPLES"))
        .map(|_| evidence.path.join("memory-samples.jsonl"));
    let replay = source.replay_ref();
    let sample = |full: bool| -> Value {
        if !full {
            return json!({"cacheBytes": replay.decoded_section_cache_bytes().to_string()});
        }
        let walk_millis = unix_millis();
        let walk = prepared::pipeline::walk_window();
        let census = replay.decoded_section_census(walk.and_then(|(_, window)| window));
        json!({
            "cacheBytes": census.bytes.to_string(),
            "walk": walk.map(|(state, window)| json!({
                "unixMillis": walk_millis, "state": state,
                "first": window.map(|(first, _)| first), "last": window.map(|(_, last)| last)
            })),
            "census": census_json(&census, unix_millis())
        })
    };
    let (accepted, elapsed) = with_memory_samples(samples.as_deref(), label, &sample, || {
        prepared::pipeline::run(
            source,
            context.content.as_ref(),
            &AtomicBool::new(false),
            |accepted| {
                publications.push(json!({"sections": accepted.sections, "bytes": accepted.bytes.to_string(),
                                         "nextBlock": accepted.next_block, "newBlocks": accepted.new_blocks,
                                         "unixMillis": unix_millis()}));
            },
        )
    })?;
    let result = json!({
        "elapsedMs": elapsed.as_secs_f64() * 1000.0,
        "blocks": source.replay_ref().index.len(), "publications": publications,
        "sections": accepted.sections, "preparedBytes": accepted.bytes.to_string(),
        "nextBlock": accepted.next_block, "newBlocks": accepted.new_blocks
    });
    Evidence::row(
        &mut evidence.replies,
        &json!({"label": label, "preparation": result}),
    )?;
    Ok(result)
}

/// Milliseconds since the Unix epoch, as phases are stamped.
fn unix_millis() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |at| at.as_millis())
        .to_string()
}

fn census_json(census: &replay::container::SectionCacheCensus, unix_millis: String) -> Value {
    let tables: Vec<Value> = TableId::ALL
        .into_iter()
        .filter_map(|table| {
            let at = table.index();
            let held = [
                census.largest[at],
                census.needed[at],
                census.obsolete[at],
                census.behind[at],
                census.window[at],
                census.beyond[at],
            ];
            held.iter().any(|&bytes| bytes != 0).then(|| {
                json!({
                    "table": format!("{table:?}"), "largest": census.largest[at],
                    "needed": census.needed[at], "obsolete": census.obsolete[at],
                    "behind": census.behind[at], "window": census.window[at],
                    "beyond": census.beyond[at], "earliestReader": census.retaining[at]
                })
            })
        })
        .collect();
    json!({
        "unixMillis": unix_millis, "bytes": census.bytes.to_string(),
        "budget": census.budget.to_string(), "readers": census.readers,
        "first": census.frontier.map(|(first, _)| first),
        "last": census.frontier.map(|(_, last)| last), "tables": tables,
        "earliestReaderMeans": "a live reader of the table at the least next block among its \
            readers: one representative of overlapping obligations, not the owner of every \
            byte listed"
    })
}

/// A sampled preparation's state, which its sampler follows.
const SAMPLING: u8 = 0;
const RAN: u8 = 1;
const FAILED: u8 = 2;
const UNWOUND: u8 = 3;

struct Sampling {
    state: std::sync::Mutex<u8>,
    changed: std::sync::Condvar,
}

impl Sampling {
    fn new() -> Self {
        Self {
            state: std::sync::Mutex::new(SAMPLING),
            changed: std::sync::Condvar::new(),
        }
    }

    fn finish(&self, state: u8) {
        *self.state.lock().unwrap_or_else(|error| error.into_inner()) = state;
        self.changed.notify_all();
    }
}

struct Ended<'a>(&'a Sampling);

impl Drop for Ended<'_> {
    fn drop(&mut self) {
        let mut state = self
            .0
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if *state == SAMPLING {
            *state = UNWOUND;
        }
        drop(state);
        self.0.changed.notify_all();
    }
}

fn with_memory_samples<T>(
    path: Option<&Path>,
    label: &str,
    sample: &(dyn Fn(bool) -> Value + Sync),
    run: impl FnOnce() -> io::Result<T>,
) -> io::Result<(T, Duration)> {
    with_memory_samples_using(path, label, sample, run, |outcome| {
        let state = outcome
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let _wait =
            outcome
                .changed
                .wait_timeout_while(state, Duration::from_millis(100), |state| {
                    *state == SAMPLING
                });
    })
}

fn with_memory_samples_using<T>(
    path: Option<&Path>,
    label: &str,
    sample: &(dyn Fn(bool) -> Value + Sync),
    run: impl FnOnce() -> io::Result<T>,
    tick: fn(&Sampling),
) -> io::Result<(T, Duration)> {
    let started = Instant::now();
    let Some(path) = path else {
        let ran = run()?;
        return Ok((ran, started.elapsed()));
    };
    let outcome = &Sampling::new();
    std::thread::scope(|scope| {
        let (ready, sampled_start) = std::sync::mpsc::channel();
        let sampler = std::thread::Builder::new()
            .name("acceptance memory sampler".into())
            .spawn_scoped(scope, move || {
                sample_memory_with_ticks(
                    path,
                    label,
                    sample,
                    outcome,
                    || tick(outcome),
                    || {
                        let _ = ready.send(());
                    },
                )
            })?;
        if sampled_start.recv().is_err() {
            return sampler
                .join()
                .unwrap_or_else(|_| Err(io::Error::other("Memory sampler panicked")))
                .and_then(|()| {
                    Err(io::Error::other(
                        "Memory sampler did not write its start row",
                    ))
                });
        }
        let ended = Ended(outcome);
        let ran = run();
        let elapsed = started.elapsed();
        outcome.finish(if ran.is_ok() { RAN } else { FAILED });
        drop(ended);
        let sampled = sampler
            .join()
            .unwrap_or_else(|_| Err(io::Error::other("Memory sampler panicked")));
        let ran = ran?;
        sampled.map(|()| (ran, elapsed))
    })
}

fn full_row(ordinal: u64, ended: bool) -> bool {
    ordinal.is_multiple_of(10) || ended
}

fn sample_memory_with_ticks(
    path: &Path,
    label: &str,
    sample: &(dyn Fn(bool) -> Value + Sync),
    outcome: &Sampling,
    mut tick: impl FnMut(),
    started: impl FnOnce(),
) -> io::Result<()> {
    let mut out = BufWriter::new(
        fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?,
    );
    let mut ordinal: u64 = 0;
    let mut write = |at: &str, ended: Option<&str>| -> io::Result<()> {
        let mut row =
            json!({"label": label, "unixMillis": unix_millis(), "at": at, "outcome": ended});
        let full = full_row(ordinal, ended.is_some());
        if let (Some(row), Value::Object(fields)) = (row.as_object_mut(), sample(full)) {
            row.extend(fields);
        }
        ordinal += 1;
        Evidence::row(&mut out, &row)?;
        out.flush()
    };
    write("start", None)?;
    started();
    loop {
        tick();
        // Read before sampling, so the end row follows the preparation's end.
        let ended = match *outcome
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
        {
            RAN => Some("ok"),
            FAILED => Some("error"),
            UNWOUND => Some("unwound"),
            _ => None,
        };
        match ended {
            Some(_) => return write("end", ended),
            None => write("running", None)?,
        }
    }
}

fn requests(source: &Source, path: Option<&Path>) -> io::Result<Requests> {
    let mut requests: Requests = path
        .map(|path| -> io::Result<_> { Ok(serde_json::from_reader(File::open(path)?)?) })
        .transpose()?
        .unwrap_or_default();
    let metadata = source.metadata();
    let branch = requests
        .branch_id
        .clone()
        .or_else(|| metadata.branches.first().map(|b| b.id.clone()))
        .ok_or_else(|| invalid("The recording has no branch"))?;
    let branch_number: u64 = branch
        .parse()
        .map_err(|_| invalid("Invalid requested branch"))?;
    let bounds = metadata
        .branches
        .iter()
        .find(|b| b.id.parse::<u64>().ok() == Some(branch_number))
        .ok_or_else(|| invalid("Requested branch is not in recording"))?;
    requests.branch_id = Some(branch);
    if requests.tags.is_empty() {
        let player = metadata
            .countries
            .iter()
            .find(|c| Some(&c.id) == metadata.player_country_id.as_ref())
            .or_else(|| metadata.countries.first())
            .ok_or_else(|| invalid("Supply tags for this recording"))?;
        requests.tags.push(player.tag.clone());
    }
    if requests.hours.is_empty() {
        let middle = (i64::from(bounds.first_hour) + i64::from(bounds.last_hour)) / 2;
        let mut hours = BTreeSet::from([bounds.first_hour, middle as i32, bounds.last_hour]);
        for block in source
            .replay_ref()
            .index
            .iter()
            .filter(|b| b.branch == branch_number)
        {
            hours.insert(block.first_hour);
            hours.insert(block.last_hour);
        }
        requests.hours = hours.into_iter().collect();
    }
    if requests
        .hours
        .iter()
        .any(|h| *h < bounds.first_hour || *h > bounds.last_hour)
    {
        return Err(invalid("Requested hour lies outside branch bounds"));
    }
    if requests.viewports.is_empty() {
        requests.viewports.push(Viewport {
            from: bounds.first_hour.into(),
            to: bounds.last_hour.into(),
            width: 800,
        });
        let center = requests.hours[requests.hours.len() / 2];
        requests.viewports.push(Viewport {
            from: f64::from(center),
            to: f64::from(center),
            width: 800,
        });
    }
    if requests
        .viewports
        .iter()
        .any(|v| !v.from.is_finite() || !v.to.is_finite() || v.from > v.to || v.width == 0)
    {
        return Err(invalid(
            "Viewports require finite increasing bounds and positive width",
        ));
    }
    Ok(requests)
}

fn focus(
    source: &Source,
    context: &LoadContext,
    requests: &Requests,
    evidence: &mut Evidence,
    repetitions: usize,
) -> io::Result<()> {
    let branch = requests.branch_id.as_ref().unwrap();
    let mut oracle = FocusCache::default();
    for tag in &requests.tags {
        let history = match oracle.get(
            source.replay_ref(),
            branch.parse().unwrap(),
            tag,
            &AtomicBool::new(false),
        ) {
            Ok((history, stopped)) => {
                if let Some(hour) = stopped {
                    evidence.skipped.push(format!("focus progress oracle stopped at {hour} for {tag}; completeness beyond the old budget needs separate evidence"));
                }
                history
            }
            Err(error) => {
                evidence.error(&format!("focus/{tag}/oracle"), &error)?;
                continue;
            }
        };
        for &hour in &requests.hours {
            let label = format!("focus/{tag}/{hour}");
            let dated = history
                .segments
                .iter()
                .find(|segment| segment.first_hour <= hour && hour <= segment.last_hour);
            let actual = match evidence.query(
                source,
                context,
                "focus-hour",
                json!({
                    "branchId": branch, "tag": tag, "hour": hour
                }),
                &label,
                repetitions,
            ) {
                Ok(actual) => actual,
                Err(error) => {
                    evidence.error(&label, &error)?;
                    continue;
                }
            };
            evidence.compare(
                &format!("{label}/available"),
                json!(dated.is_some()),
                json!(!actual.is_null()),
            )?;
            let Some(dated) = dated.filter(|_| !actual.is_null()) else {
                continue;
            };
            let progress = history
                .progress
                .iter()
                .find(|run| run.first_hour <= hour && hour <= run.last_hour);
            let gain = progress.and_then(|run| {
                let index = history
                    .progress
                    .iter()
                    .position(|candidate| std::ptr::eq(candidate, run))?;
                let previous = index
                    .checked_sub(1)
                    .and_then(|index| history.progress.get(index))?;
                let previous_order = history.segments.iter().find(|segment| {
                    segment.first_hour <= previous.last_hour
                        && previous.last_hour <= segment.last_hour
                })?;
                if previous.last_hour.checked_add(1) != Some(run.first_hour)
                    || dated.order.current.is_none()
                    || previous_order.order.current != dated.order.current
                {
                    return None;
                }
                let gain = run.progress_raw.parse::<i128>().ok()?
                    - previous.progress_raw.parse::<i128>().ok()?;
                (gain > 0).then(|| gain.to_string())
            });
            evidence.compare(&format!("{label}/numeric"), json!({
                "current": dated.order.current, "continuous": dated.order.continuous,
                "paused": dated.order.paused, "progressRaw": progress.map(|run| &run.progress_raw), "dailyGainRaw": gain
            }), json!({
                "current": actual["current"], "continuous": actual["continuous"],
                "paused": actual["paused"], "progressRaw": actual["progressRaw"], "dailyGainRaw": actual["dailyGainRaw"]
            }))?;
            let native = source.replay_ref().seek_selected(
                branch.parse().unwrap(),
                hour,
                Some(&[TableId::Campaign, TableId::Countries, TableId::CountryLinks]),
            )?;
            if let SeekResult::Frame(frame) = native
                && let (Ok(countries), Ok(links)) = (
                    frame.complete::<tables::Countries>(),
                    frame.complete::<tables::CountryLinks>(),
                )
            {
                let native = dashboard::metadata::tagged_countries(countries, links)
                    .get(tag.as_bytes())
                    .and_then(|country| {
                        if let dashboard::metadata::TaggedCountry::Found(country, slot) = country {
                            Some(json!({"slot": slot, "countryId": country.id}))
                        } else {
                            None
                        }
                    });
                evidence.compare(
                    &format!("{label}/identity"),
                    native.unwrap_or(Value::Null),
                    json!({"slot": actual["slot"], "countryId": actual["countryId"]}),
                )?;
            }
            let entries = focus_pages(
                source,
                context,
                evidence,
                branch,
                tag,
                hour,
                "focus-order-page",
                &actual["completed"],
                repetitions,
            )?;
            evidence.compare(
                &format!("{label}/order"),
                serde_json::to_value(&dated.order.completed)?,
                json!(entries),
            )?;
            if context.content.is_none() {
                continue;
            }
            // Each date's art from `focus-art`, as the focus-artwork product
            // presents the same tokens at that hour.
            for (mode, descriptions) in [(FocusArtMode::Display, false), (FocusArtMode::Text, true)]
            {
                let mut dates: BTreeMap<i32, BTreeSet<String>> = BTreeMap::new();
                for entry in &dated.order.completed {
                    dates
                        .entry(entry.completed_by_hour.unwrap_or(dated.first_hour))
                        .or_default()
                        .insert(entry.token.clone());
                }
                if let Some(active) = dated
                    .order
                    .current
                    .as_ref()
                    .or(dated.order.continuous.as_ref())
                {
                    dates.entry(hour).or_default().insert(active.clone());
                }
                for (at, tokens) in dates {
                    let tokens: Vec<String> = tokens.into_iter().collect();
                    for (page, tokens) in tokens.chunks(128).enumerate() {
                        let art = FocusArt::load(
                            source,
                            &FocusArtParams {
                                branch_id: branch.clone(),
                                tag: tag.clone(),
                                hour: at,
                                tokens: tokens.to_vec(),
                                mode,
                            },
                            context,
                            &AtomicBool::new(false),
                        )?;
                        let expected = FocusArtwork::load(
                            source,
                            &FocusArtworkParams {
                                branch_id: branch.clone(),
                                hour: at,
                                tag: tag.clone(),
                                tokens: tokens.to_vec(),
                                text_only: descriptions.then_some(true),
                                include_descriptions: Some(descriptions),
                                icons_only: None,
                            },
                            context,
                            &AtomicBool::new(false),
                        )?;
                        evidence.compare(
                            &format!("{label}/focus-art/{mode:?}/{at}/{page}"),
                            serde_json::to_value(&expected.items)?,
                            serde_json::to_value(&art.art)?,
                        )?;
                        evidence.compare(
                            &format!("{label}/focus-art/{mode:?}/{at}/{page}/failures"),
                            json!([]),
                            serde_json::to_value(&art.failures)?,
                        )?;
                    }
                }
            }
        }
    }
    if context.content.is_none() {
        evidence.skipped.push("Focus titles, PNG bytes and details need --game-dir/BATTLE_PLAN_GAME_DIR and --user-dir/BATTLE_PLAN_USER_DIR".into());
    }
    focus_unavailable(source, context, requests, evidence, repetitions)
}

#[allow(clippy::too_many_arguments)]
fn focus_pages(
    source: &Source,
    context: &LoadContext,
    evidence: &mut Evidence,
    branch: &str,
    tag: &str,
    hour: i32,
    product: &str,
    pages: &Value,
    repetitions: usize,
) -> io::Result<Vec<Value>> {
    let count = decimal(&pages["count"])?;
    let rows = decimal(&pages["rows"])?;
    let mut result = Vec::new();
    for page in 0..count {
        let label = format!("{product}/{tag}/{hour}/{page}");
        let data = evidence.query(source, context, product, json!({
            "branchId": branch, "tag": tag, "hour": hour, "pages": pages, "page": page.to_string()
        }), &label, repetitions)?;
        let values = data
            .as_array()
            .ok_or_else(|| invalid("Prepared focus page is absent or not an array"))?;
        result.extend(values.iter().cloned());
    }
    evidence.compare(
        &format!("{product}/{tag}/{hour}/row-count"),
        json!(rows),
        json!(result.len()),
    )?;
    Ok(result)
}

fn decimal(value: &Value) -> io::Result<u64> {
    value
        .as_str()
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| invalid("Expected an exact decimal ordinal"))
}

fn focus_unavailable(
    source: &Source,
    context: &LoadContext,
    requests: &Requests,
    evidence: &mut Evidence,
    repetitions: usize,
) -> io::Result<()> {
    let branch = requests.branch_id.as_ref().unwrap();
    let selected = [
        TableId::Countries,
        TableId::CountryLinks,
        TableId::Focus,
        TableId::CompletedFocuses,
    ];
    for &hour in &requests.hours {
        let label = format!("focus-unavailable/{hour}");
        let oracle =
            source
                .replay_ref()
                .seek_selected(branch.parse().unwrap(), hour, Some(&selected))?;
        let SeekResult::Frame(frame) = oracle else {
            continue;
        };
        let mut reasons = Vec::new();
        for (table, error) in [
            (
                TableId::Countries,
                frame.complete::<tables::Countries>().err(),
            ),
            (
                TableId::CountryLinks,
                frame.complete::<tables::CountryLinks>().err(),
            ),
            (TableId::Focus, frame.complete::<tables::Focus>().err()),
            (
                TableId::CompletedFocuses,
                frame.complete::<tables::CompletedFocuses>().err(),
            ),
        ] {
            if let Some(error) = error {
                let (reason, missing_rows) = match &*frame.tables[table.index()] {
                    TableView::Unavailable { reason } => (Some(reason.clone()), None),
                    TableView::NotRecorded { mode } => {
                        (Some(format!("not recorded in a {mode} recording")), None)
                    }
                    TableView::Truncated { missing, .. } => (None, Some(missing.to_string())),
                    TableView::Complete(_) => (Some(error.to_string()), None),
                };
                reasons.push(json!({"table": format!("{table:?}"), "reason": reason, "missingRows": missing_rows}));
            }
        }
        match evidence.query(
            source,
            context,
            "focus-unavailable",
            json!({"branchId": branch, "hour": hour}),
            &label,
            repetitions,
        ) {
            Ok(actual) => evidence.compare(
                &label,
                if reasons.is_empty() {
                    Value::Null
                } else {
                    json!(reasons)
                },
                if actual.is_null() {
                    Value::Null
                } else {
                    actual["reasons"].clone()
                },
            )?,
            Err(error) => evidence.error(&label, &error)?,
        }
    }
    Ok(())
}

fn coded(value: Option<&ProducedCoded>, sample: usize) -> Option<String> {
    let Some(value) = value else {
        return Some("0".into());
    };
    if value
        .unknown
        .chunks_exact(2)
        .any(|range| range[0] as usize <= sample && sample < range[1] as usize)
    {
        return None;
    }
    if sample < value.from as usize {
        return Some("0".into());
    }
    Some(
        value
            .deltas
            .iter()
            .take(sample - value.from as usize + 1)
            .map(|value| i128::from(*value))
            .sum::<i128>()
            .to_string(),
    )
}

fn default_production(
    source: &Source,
    context: &LoadContext,
    requests: &Requests,
) -> io::Result<Vec<ProducedParams>> {
    let mut params = ProducedParams {
        branch_id: requests.branch_id.clone().unwrap(),
        tags: Vec::new(),
        top: None,
        measure: ProducedMeasure::Total,
        count: ProducedCount::Units,
        grouping: ProducedGrouping::Type,
        included: Vec::new(),
        refits: false,
    };
    let catalogue = EquipmentProduced::load(source, &params, context, &AtomicBool::new(false))?;
    let mut subsets = vec![Vec::new()];
    for kind in &catalogue.types {
        subsets.push(vec![kind.key.clone()]);
    }
    // Duplicates and an unresolved selector are real request forms. Selectors
    // are opaque; a missing key stays missing rather than borrowing a type.
    if let Some(kind) = catalogue.types.first() {
        subsets.push(vec![kind.key.clone(), kind.key.clone()]);
    }
    let mut missing = "acceptance missing type / opaque".to_owned();
    while catalogue.types.iter().any(|kind| kind.key == missing) {
        missing.push('?');
    }
    subsets.push(vec![missing]);
    let mut result = Vec::new();
    params.tags = requests.tags.clone();
    for measure in [
        ProducedMeasure::Total,
        ProducedMeasure::Day,
        ProducedMeasure::Week,
        ProducedMeasure::Month,
    ] {
        for count in [ProducedCount::Units, ProducedCount::Ic] {
            for grouping in [
                ProducedGrouping::Type,
                ProducedGrouping::Variant,
                ProducedGrouping::Domain,
            ] {
                for refits in [false, true] {
                    for included in &subsets {
                        result.push(ProducedParams {
                            measure,
                            count,
                            grouping,
                            refits,
                            included: included.clone(),
                            ..params.clone()
                        });
                    }
                }
            }
        }
    }
    result.push(ProducedParams {
        tags: Vec::new(),
        top: Some(10),
        ..params
    });
    Ok(result)
}

fn production(
    source: &Source,
    context: &LoadContext,
    requests: &Requests,
    evidence: &mut Evidence,
    repetitions: usize,
) -> io::Result<()> {
    let cases = if requests.production.is_empty() {
        default_production(source, context, requests)?
    } else {
        requests.production.clone()
    };
    let mut written = File::create(evidence.path.join("production-requests.json"))?;
    serde_json::to_writer_pretty(&mut written, &cases)?;
    for (case, params) in cases.iter().enumerate() {
        EquipmentProduced::validate(params).map_err(invalid)?;
        if params.branch_id.parse::<u64>().ok()
            != requests
                .branch_id
                .as_ref()
                .and_then(|branch| branch.parse().ok())
        {
            return Err(invalid("Production requests must use the requested branch"));
        }
        let reference =
            match EquipmentProduced::load(source, params, context, &AtomicBool::new(false)) {
                Ok(reply) => reply,
                Err(error) => {
                    evidence.error(&format!("production/{case}/oracle"), &error)?;
                    continue;
                }
            };
        for &hour in &requests.hours {
            for ranked in [false, true] {
                let label = format!("production/{case}/{hour}/ranked-{ranked}");
                let expected = production_at(&reference, hour, ranked || params.top.is_some());
                let mut rows = Vec::new();
                let mut offset = 0_usize;
                loop {
                    let page_label = format!("{label}/{offset}");
                    let actual = match evidence.query(source, context, "production-select", json!({
                        "hour": hour, "selection": params, "offset": offset.to_string(), "limit": 128, "ranked": ranked
                    }), &page_label, repetitions) {
                        Ok(actual) => actual,
                        Err(error) => { evidence.error(&page_label, &error)?; break; }
                    };
                    if actual.is_null() {
                        evidence.compare(&label, json!(expected.is_empty()), json!(true))?;
                        break;
                    }
                    evidence.compare(
                        &format!("{page_label}/branch"),
                        json!(params.branch_id),
                        actual["branchId"].clone(),
                    )?;
                    evidence.compare(
                        &format!("{page_label}/keys"),
                        serde_json::to_value(&reference.keys)?,
                        actual["keys"].clone(),
                    )?;
                    evidence.compare(
                        &format!("{page_label}/types"),
                        serde_json::to_value(&reference.types)?,
                        actual["types"].clone(),
                    )?;
                    evidence.compare(
                        &format!("{page_label}/warnings"),
                        json!(reference.warnings),
                        actual["warnings"].clone(),
                    )?;
                    evidence.compare(
                        &format!("{page_label}/known"),
                        json!(
                            reference
                                .known
                                .iter()
                                .any(|span| span.first_hour <= hour && hour <= span.last_hour)
                        ),
                        actual["known"].clone(),
                    )?;
                    evidence.compare(
                        &format!("{page_label}/total-countries"),
                        json!(expected.len()),
                        actual["totalCountries"].clone(),
                    )?;
                    let countries = actual["countries"]
                        .as_array()
                        .ok_or_else(|| invalid("Prepared production countries are not an array"))?;
                    for country in countries {
                        rows.push(json!({"tag": country["tag"], "total": country["total"], "values": country["values"]}));
                    }
                    let total = actual["totalCountries"]
                        .as_u64()
                        .ok_or_else(|| invalid("Invalid production count"))?;
                    offset += countries.len();
                    if offset as u64 >= total {
                        break;
                    }
                    if countries.is_empty() {
                        return Err(invalid("Production pagination made no progress"));
                    }
                }
                evidence.compare(&label, json!(expected), json!(rows))?;
            }
        }
    }
    Ok(())
}

fn production_at(reply: &EquipmentProducedReply, hour: i32, ranked: bool) -> Vec<Value> {
    let mut rows = Vec::new();
    for series in &reply.series {
        let Some(sample) = series
            .first_hours
            .iter()
            .zip(&series.last_hours)
            .position(|(&first, &last)| first <= hour && hour <= last)
        else {
            continue;
        };
        let total = coded(series.total.as_ref(), sample);
        let values: Vec<_> = reply.keys.iter().enumerate().map(|(index, key)| {
            let values = series.values.iter().find(|values| values.key as usize == index);
            json!({
                "key": key.key,
                "amount": coded(values.map(|values| &values.values), sample),
                "equipmentAmount": coded(values.map(|values| values.equipment_values.as_ref().unwrap_or(&values.values)), sample)
            })
        }).collect();
        if !ranked || total.is_some() {
            rows.push(json!({"tag": series.tag, "total": total, "values": values}));
        }
    }
    if ranked {
        rows.sort_by(|left, right| {
            let exact = |value: &Value| {
                value["total"]
                    .as_str()
                    .and_then(|value| value.parse::<i128>().ok())
            };
            exact(right)
                .cmp(&exact(left))
                .then_with(|| left["tag"].as_str().cmp(&right["tag"].as_str()))
        });
    }
    rows
}

fn measure(
    source: &Source,
    context: &LoadContext,
    requests: &Requests,
    evidence: &mut Evidence,
    repetitions: usize,
) -> io::Result<()> {
    let branch = requests.branch_id.as_ref().unwrap();
    let bounds = source
        .metadata()
        .branches
        .into_iter()
        .find(|b| b.id.parse::<u64>().ok() == branch.parse().ok())
        .unwrap();
    for measure in [Measure::PoliticalPower, Measure::CommandPower] {
        let reference = CountryMeasure::load(
            source,
            &MeasureParams {
                branch_id: branch.clone(),
                measure,
                tags: requests.tags.clone(),
            },
            context,
            &AtomicBool::new(false),
        )?;
        for &hour in &requests.hours {
            let label = format!("measure/{measure:?}/{hour}");
            let actual = evidence.query(source, context, "series-state", json!({
                "branchId": branch, "family": {"kind": "country-measure", "measure": measure},
                "reads": requests.tags.iter().map(|tag| json!({"tag": tag, "hour": hour})).collect::<Vec<_>>()
            }), &label, repetitions)?;
            evidence.compare(&format!("{label}/context"), json!({"branchId": branch, "measure": measure, "decimals": 5}),
                json!({"branchId": actual["branchId"], "measure": actual["measure"], "decimals": actual["decimals"]}))?;
            let expected: Vec<_> = reference
                .series
                .iter()
                .map(|series| {
                    let run = series
                        .runs
                        .iter()
                        .find(|run| run.first_hour <= hour && hour <= run.last_hour);
                    json!({"tag": series.tag, "hour": hour, "run": run,
                       "drop": series.drops.iter().find(|drop| drop.hour == hour)})
                })
                .collect();
            let states = actual["states"]
                .as_array()
                .ok_or_else(|| invalid("Measure states are not an array"))?;
            let reached: Vec<_> = states.iter().map(|state| json!({
                "tag": state["tag"], "hour": state["hour"], "drop": state["drop"],
                "run": if state["run"].is_null() { Value::Null } else { json!({
                    "firstHour": state["run"]["firstHour"], "lastHour": state["run"]["lastHour"],
                    "valueRaw": state["run"]["valueRaw"], "startsSegment": state["run"]["startsSegment"]
                }) }
            })).collect();
            evidence.compare(&label, json!(expected), json!(reached))?;
            if let SeekResult::Frame(frame) = source.replay_ref().seek_selected(
                branch.parse().unwrap(),
                hour,
                Some(&[TableId::Campaign, TableId::Countries, TableId::CountryLinks]),
            )? && let (Ok(countries), Ok(links)) = (
                frame.complete::<tables::Countries>(),
                frame.complete::<tables::CountryLinks>(),
            ) {
                for state in states.iter().filter(|state| !state["run"].is_null()) {
                    let tag = state["tag"]
                        .as_str()
                        .ok_or_else(|| invalid("Measure state tag is not text"))?;
                    let identity = dashboard::metadata::tagged_country(countries, links, tag)
                        .found()
                        .map(|(country, slot)| json!({"slot": slot, "countryId": country.id}))
                        .unwrap_or(Value::Null);
                    evidence.compare(&format!("{label}/{tag}/native-identity"), identity,
                            json!({"slot": state["run"]["slot"], "countryId": state["run"]["countryId"]}))?;
                    let segment = reference
                        .series
                        .iter()
                        .find(|series| series.tag == tag)
                        .and_then(|series| {
                            series
                                .runs
                                .iter()
                                .rev()
                                .find(|run| run.first_hour <= hour && run.starts_segment)
                        })
                        .map(|run| run.first_hour);
                    evidence.compare(
                        &format!("{label}/{tag}/segment-start"),
                        json!(segment),
                        state["run"]["segmentFirstHour"].clone(),
                    )?;
                }
            }
            evidence.compare(
                &format!("{label}/warnings"),
                json!(reference.warnings),
                actual["warnings"].clone(),
            )?;
        }
        for (viewport_id, viewport) in requests.viewports.iter().enumerate() {
            let label = format!("measure-summary/{measure:?}/{viewport_id}");
            let actual = evidence.query(source, context, "series-summary", json!({
                "branchId": branch, "family": {"kind": "country-measure", "measure": measure}, "tags": requests.tags,
                "from": viewport.from, "to": viewport.to, "width": viewport.width, "withMarks": true
            }), &label, repetitions)?;
            evidence.compare(&format!("{label}/context"), json!({"branchId": branch, "measure": measure, "decimals": 5}),
                json!({"branchId": actual["branchId"], "measure": actual["measure"], "decimals": actual["decimals"]}))?;
            let expected: Vec<_> = reference.series.iter().map(|series| json!({
                "tag": series.tag,
                "summary": chart_summary(&series.runs, bounds.first_hour, bounds.last_hour, viewport),
                "marks": mark_summary(series, viewport)
            })).collect();
            evidence.compare(&label, json!(expected), actual["series"].clone())?;
            for series in &reference.series {
                let label = format!("measure-marks/{measure:?}/{viewport_id}/{}", series.tag);
                let actual = evidence.query(
                    source,
                    context,
                    "events-clusters",
                    json!({
                        "branchId": branch, "family": {"kind": "country-measure", "measure": measure}, "tag": series.tag,
                        "from": viewport.from, "to": viewport.to, "width": viewport.width
                    }),
                    &label,
                    repetitions,
                )?;
                evidence.compare(
                    &label,
                    mark_summary(series, viewport),
                    actual["summary"].clone(),
                )?;
            }
            for key in ["ArrowLeft", "ArrowRight", "Home", "End"] {
                for origin in requests
                    .hours
                    .iter()
                    .copied()
                    .map(|h| Some(f64::from(h)))
                    .chain(std::iter::once(None))
                {
                    let label = format!("measure-key/{measure:?}/{viewport_id}/{key}/{origin:?}");
                    let actual = evidence.query(source, context, "series-next-change", json!({
                        "branchId": branch, "family": {"kind": "country-measure", "measure": measure}, "tags": requests.tags,
                        "key": key, "from": origin, "firstHour": viewport.from, "lastHour": viewport.to
                    }), &label, repetitions)?;
                    evidence.compare(
                        &label,
                        json!({"hour": key_hour(&reference.series, key, origin, viewport)}),
                        actual,
                    )?;
                }
            }
        }
        for series in &reference.series {
            for through in requests
                .hours
                .iter()
                .copied()
                .map(Some)
                .chain(std::iter::once(None))
            {
                let drops: Vec<_> = series
                    .drops
                    .iter()
                    .filter(|drop| through.is_none_or(|hour| drop.hour <= hour))
                    .collect();
                let mut reached = Vec::new();
                for first in (0..drops.len().max(1)).step_by(128) {
                    let label = format!(
                        "measure-drops/{measure:?}/{}/{through:?}/{first}",
                        series.tag
                    );
                    let actual = evidence.query(
                        source,
                        context,
                        "events-page",
                        json!({
                            "branchId": branch, "family": {"kind": "country-measure", "measure": measure}, "tags": [series.tag],
                            "cursor": first.to_string(), "pageSize": 128, "to": through
                        }),
                        &label,
                        repetitions,
                    )?;
                    evidence.compare(
                        &format!("{label}/total"),
                        json!(drops.len().to_string()),
                        actual["total"].clone(),
                    )?;
                    reached.extend(
                        actual["rows"]
                            .as_array()
                            .ok_or_else(|| invalid("Measure drops are not an array"))?
                            .iter()
                            .map(|event| event["row"].clone()),
                    );
                }
                evidence.compare(
                    &format!("measure-drops/{measure:?}/{}/{through:?}", series.tag),
                    json!(drops),
                    json!(reached),
                )?;
            }
        }
    }
    Ok(())
}

/// An independent sweep of the old product runs. It does not read prepared
/// geometry or call the prepared producer/query implementation as its oracle.
fn chart_summary(runs: &[FixedRun], first: i32, last: i32, viewport: &Viewport) -> Value {
    let from = (viewport.from.floor() as i64).max(i64::from(first));
    let to = (viewport.to.floor() as i64).min(i64::from(last));
    if from > to {
        return json!({"from": viewport.from, "to": viewport.to, "width": viewport.width, "buckets": [], "runs": []});
    }
    let count = (to - from + 1) as u64;
    let per_pixel = (count / u64::from(viewport.width)).max(1);
    let fine = per_pixel <= 16;
    let size = if fine {
        1_i64 << (63 - per_pixel.leading_zeros())
    } else {
        count.div_ceil(u64::from(viewport.width)) as i64
    };
    let mut start = from;
    let mut buckets = Vec::new();
    while start <= to {
        let end = if fine {
            i64::from(first) + ((start - i64::from(first)) / size + 1) * size - 1
        } else {
            start + size - 1
        }
        .min(to);
        buckets.push(chart_bucket(runs, start as i32, end as i32));
        start = end + 1;
    }
    let mut summary = json!({"from": viewport.from, "to": viewport.to, "width": viewport.width, "buckets": buckets});
    if size == 1 {
        summary["runs"] = json!(runs.iter().filter(|run| i64::from(run.last_hour) >= from && i64::from(run.first_hour) <= to)
            .map(|run| json!({
                "firstHour": run.first_hour, "lastHour": run.last_hour,
                "value": run.value_raw.parse::<i64>().expect("old measure supplies native fixed5") as f64 / 100_000.0,
                "breakBefore": run.starts_segment,
            })).collect::<Vec<_>>());
    }
    summary
}

fn chart_bucket(runs: &[FixedRun], first: i32, last: i32) -> Value {
    let mut found = Vec::new();
    for run in runs
        .iter()
        .filter(|run| run.first_hour <= last && first <= run.last_hour)
    {
        let begin = first.max(run.first_hour);
        let end = last.min(run.last_hour);
        let value = run
            .value_raw
            .parse::<i64>()
            .expect("old measure supplies native fixed5") as f64
            / 100_000.0;
        found.push((
            begin,
            end,
            value,
            run.starts_segment && begin == run.first_hour,
        ));
    }
    let Some(&(begin, _, first_value, first_break)) = found.first() else {
        return Value::Null;
    };
    let mut min = (begin, first_value);
    let mut max = min;
    let mut previous_end = i64::from(begin) - 1;
    let mut previous_value = first_value;
    let mut last_value_hour = begin;
    let mut has_break = begin != first;
    for &(begin, end, value, broken) in &found {
        if value < min.1 {
            min = (begin, value);
        }
        if value > max.1 {
            max = (begin, value);
        }
        if begin != found[0].0 && (broken || previous_end + 1 != i64::from(begin)) {
            has_break = true;
        }
        if broken || previous_end + 1 != i64::from(begin) || value != previous_value {
            last_value_hour = begin;
        }
        previous_end = i64::from(end);
        previous_value = value;
    }
    has_break |= previous_end != i64::from(last);
    json!({"firstHour": found[0].0, "lastHour": previous_end,
           "firstValue": first_value, "lastValue": previous_value, "lastValueHour": last_value_hour,
           "minHour": min.0, "minValue": min.1, "maxHour": max.0, "maxValue": max.1,
           "firstBreak": first_break, "hasBreak": has_break})
}

/// The old widget's balance-only stops are run starts and drop dates. The shared
/// chart's keyStops/keyedHour rules retain a held value at the clipped left edge.
/// Additional prepared unknown-boundary stops remain visible parity differences.
fn key_hour(
    series: &[MeasureSeries],
    key: &str,
    origin: Option<f64>,
    viewport: &Viewport,
) -> Option<f64> {
    let mut dates = BTreeSet::new();
    for series in series {
        for run in &series.runs {
            dates.insert(run.first_hour);
        }
        dates.extend(series.drops.iter().map(|drop| drop.hour));
    }
    let carried = dates.iter().any(|hour| f64::from(*hour) < viewport.from);
    let mut shown: Vec<_> = dates
        .into_iter()
        .map(f64::from)
        .filter(|hour| viewport.from <= *hour && *hour <= viewport.to)
        .collect();
    if carried && shown.first().is_none_or(|hour| *hour > viewport.from) {
        shown.insert(0, viewport.from);
    }
    if shown.is_empty() {
        return None;
    }
    match (key, origin) {
        ("Home", _) => shown.first().copied(),
        ("End", _) | (_, None) => shown.last().copied(),
        ("ArrowRight", Some(from)) => shown
            .iter()
            .copied()
            .find(|hour| *hour > from)
            .or_else(|| shown.iter().copied().rev().find(|hour| *hour <= from)),
        ("ArrowLeft", Some(from)) => shown
            .iter()
            .copied()
            .rev()
            .find(|hour| *hour < from)
            .or_else(|| shown.iter().copied().find(|hour| *hour >= from)),
        _ => None,
    }
}

fn mark_summary(series: &MeasureSeries, viewport: &Viewport) -> Value {
    let cells = viewport.width.div_ceil(18);
    let mut clusters = Vec::new();
    for cell in 0..cells {
        let begin =
            viewport.from + (viewport.to - viewport.from) * f64::from(cell) / f64::from(cells);
        let end =
            viewport.from + (viewport.to - viewport.from) * f64::from(cell + 1) / f64::from(cells);
        let events: Vec<_> = series
            .drops
            .iter()
            .enumerate()
            .filter(|(_, drop)| {
                f64::from(drop.hour) >= begin
                    && if cell + 1 == cells {
                        f64::from(drop.hour) <= viewport.to
                    } else {
                        f64::from(drop.hour) < end
                    }
            })
            .collect();
        if let Some((ordinal, first)) = events.first() {
            clusters.push(json!({"firstHour": first.hour, "lastHour": events.last().unwrap().1.hour,
                                "firstOrdinal": ordinal.to_string(), "count": events.len().to_string()}));
        }
    }
    json!({"from": viewport.from, "to": viewport.to, "width": viewport.width, "clusters": clusters})
}

const MAP_TABLES: &[TableId] = &[TableId::Campaign, TableId::Clock, TableId::DivisionStatus];

fn map(
    source: &Source,
    context: &LoadContext,
    requests: &Requests,
    evidence: &mut Evidence,
    repetitions: usize,
) -> io::Result<()> {
    let Some((store, key, progress)) =
        prepared::pipeline::map_checkpoint_binding(source, context.content.as_ref())?
    else {
        return Err(invalid("Prepared map progress missing"));
    };
    let branch: u64 = requests.branch_id.as_ref().unwrap().parse().unwrap();
    let mut reader = dashboard_base::prepared_map::Reader::new(
        store,
        key,
        progress.clone(),
        source.replay(),
        dashboard_base::prepared_map::selected_tables(),
    )?;
    let mut rows = BufWriter::new(File::create(evidence.path.join("map-reads.jsonl"))?);
    for &hour in &requests.hours {
        for operation in ["seek", "step"] {
            let target = if operation == "step" {
                let Some(target) = hour.checked_add(1).filter(|target| {
                    reader
                        .range(branch)
                        .is_some_and(|(_, last)| *target <= last)
                }) else {
                    continue;
                };
                target
            } else {
                hour
            };
            let label = format!("map/{operation}/{target}");
            let mut times = Vec::new();
            let mut actual = None;
            for _ in 0..repetitions {
                if operation == "step" {
                    reader.seek(branch, hour)?;
                }
                let started = Instant::now();
                actual = if operation == "seek" {
                    reader.seek(branch, target)?
                } else {
                    reader.advance(branch, target, |_| {})?
                };
                times.push(started.elapsed().as_secs_f64() * 1000.0);
                Evidence::row(
                    &mut rows,
                    &json!({
                        "label": label, "appliedFrames": reader.applied_frames,
                        "checkpointHeaderReads": reader.checkpoint_reads, "sourceBlockReads": reader.source_reads,
                        "checkpointStride": progress.stride,
                        "checkpointPayloadBytes": reader.checkpoint_bytes.to_string(),
                        "retainedSourceBlockFrames": reader.retained_source_frames,
                        "sourceBlockBytesUpperBound": reader.source_block_bytes.to_string(),
                        "sourceDecodedFrames": reader.source_decoded_frames,
                        "sourceLoadUs": reader.source_load_us
                    }),
                )?;
                if operation == "seek" {
                    evidence.compare(
                        &format!("{label}/bounded-replay"),
                        json!(true),
                        json!(reader.applied_frames < u64::from(progress.stride)),
                    )?;
                    let count = progress.branches[&branch].checkpoints;
                    let header_bound = if count == 0 {
                        0
                    } else {
                        u64::from(64 - count.leading_zeros())
                    };
                    evidence.compare(
                        &format!("{label}/bounded-header-reads"),
                        json!(true),
                        json!(reader.checkpoint_reads <= header_bound),
                    )?;
                } else {
                    evidence.compare(
                        &format!("{label}/step-checkpoint-reads"),
                        json!(0),
                        json!(reader.checkpoint_reads),
                    )?;
                    evidence.compare(
                        &format!("{label}/step-applied-frames"),
                        json!(true),
                        json!(reader.applied_frames <= 1),
                    )?;
                }
            }
            let first_ms = times[0];
            times.sort_by(f64::total_cmp);
            let percentile =
                |percent: usize| times[(times.len() * percent).div_ceil(100).saturating_sub(1)];
            evidence.timings.push(Timing {
                label: label.clone(),
                product: format!("map-{operation}"),
                first_ms,
                p50_ms: percentile(50),
                p95_ms: percentile(95),
                p99_ms: percentile(99),
                samples: repetitions,
                reply_bytes: 0,
            });
            let expected = source
                .replay_ref()
                .seek_selected(branch, target, Some(MAP_TABLES))?;
            match (expected, actual) {
                (
                    SeekResult::Frame(expected),
                    Some(dashboard_base::prepared_map::Current::Frame(actual)),
                ) => {
                    evidence.compare(
                        &format!("{label}/hour"),
                        json!([expected.branch, expected.hour as i64]),
                        json!([actual.branch, actual.hour as i64]),
                    )?;
                    for &table in MAP_TABLES {
                        // Debug retains row order, exact wide fields and Recorded
                        // states. Provenance is compared separately: SharedView's
                        // PartialEq deliberately compares only row/availability.
                        evidence.compare(
                            &format!("{label}/{table:?}"),
                            json!(format!("{:?}", &*expected.tables[table.index()])),
                            json!(format!("{:?}", &*actual.tables[table.index()])),
                        )?;
                        evidence.compare(
                            &format!("{label}/{table:?}/sampled-at"),
                            json!(expected.tables[table.index()].sampled_at()),
                            json!(actual.tables[table.index()].sampled_at()),
                        )?;
                    }
                }
                (
                    SeekResult::Gap { reason, .. },
                    Some(dashboard_base::prepared_map::Current::Gap { reason: actual, .. }),
                ) => {
                    evidence.compare(
                        &format!("{label}/gap-reason"),
                        json!(reason),
                        json!(actual),
                    )?;
                }
                (expected, actual) => evidence.compare(
                    &format!("{label}/availability"),
                    json!(matches!(expected, SeekResult::Frame(_))),
                    json!(matches!(
                        actual,
                        Some(dashboard_base::prepared_map::Current::Frame(_))
                    )),
                )?,
            }
        }
    }
    rows.flush()
}

/// About one year: the owner's ~2 s exact-zoom target; mirrors the shared reader.
const PROGRESSIVE_ZOOM_BLOCKS: usize = 13;

/// Select thirteen complete unprepared blocks, so the exact query cannot touch
/// a fourteenth block through a fractional boundary or a prepared overlap.
fn progressive_zoom(blocks: &[(i32, i32)], through: i32) -> io::Result<(i32, i32)> {
    let tail: Vec<_> = blocks
        .iter()
        .filter(|(first, _)| *first > through)
        .take(PROGRESSIVE_ZOOM_BLOCKS)
        .collect();
    if tail.len() != PROGRESSIVE_ZOOM_BLOCKS {
        return Err(invalid(
            "Need thirteen full blocks beyond accepted coverage",
        ));
    }
    Ok((tail[0].0, tail[PROGRESSIVE_ZOOM_BLOCKS - 1].1))
}

/// Mirrors tail's index selection and exact left-edge baseline rule. These
/// counts describe selected source blocks, never measured filesystem reads.
fn progressive_blocks_selected(
    blocks: &[(i32, i32)],
    gaps: &[(i32, i32)],
    through: i32,
    window: (i32, i32),
) -> (usize, usize) {
    let first = i64::from(window.0).max(i64::from(through) + 1);
    let selected: Vec<_> = blocks
        .iter()
        .filter(|(start, end)| i64::from(*end) >= first && *start <= window.1)
        .collect();
    let preceding = selected.len() <= PROGRESSIVE_ZOOM_BLOCKS
        && selected.first().is_some_and(|block| {
            let start = i64::from(block.0).max(first) as i32;
            start == block.0
                && start.checked_sub(1).is_some_and(|before| {
                    blocks
                        .iter()
                        .any(|(first, last)| *first <= before && *last >= before)
                        && !gaps
                            .iter()
                            .any(|(first, last)| *first <= before && *last >= before)
                })
        });
    (selected.len(), usize::from(preceding))
}

fn progressive_row(output: &Path, value: Value) -> io::Result<()> {
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(output)?;
    Evidence::row(&mut file, &value)?;
    file.flush()?;
    println!("{value}");
    Ok(())
}

fn progressive_measure(options: Options, path: &Path) -> io::Result<bool> {
    let mode = options.progressive.as_deref().unwrap();
    let output = options
        .measurements
        .as_deref()
        .ok_or_else(|| invalid("Supply --measurements"))?;
    // The launcher creates the copy once. Reject accidentally measuring or
    // preparing the protected input directly, even if invoked without it.
    let recording = fs::canonicalize(&options.recording)?;
    if !recording.starts_with(path) {
        return Err(invalid(
            "Progressive recording must be a scratch copy inside evidence",
        ));
    }
    let output_parent = output
        .parent()
        .ok_or_else(|| invalid("Measurements need a parent directory"))?;
    if !fs::canonicalize(output_parent)?.starts_with(path) {
        return Err(invalid(
            "Measurements must be inside the scratch evidence directory",
        ));
    }
    let opened = Instant::now();
    let source = open(&recording)?;
    let cancelled = AtomicBool::new(false);
    if mode == "setup" {
        let mut cursor = 0;
        let result = prepared::pipeline::run(&source, None, &cancelled, |accepted| {
            cursor = accepted.next_block;
            println!(
                "Accepted {cursor} of {} blocks",
                source.replay_ref().index.len()
            );
            if cursor >= options.prefix_blocks {
                cancelled.store(true, std::sync::atomic::Ordering::Relaxed);
            }
        });
        match result {
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
            Ok(_) => {}
        }
        if cursor == 0 || cursor >= source.replay_ref().index.len() {
            return Err(invalid(
                "Preparation did not leave a nonempty accepted prefix and unprepared tail",
            ));
        }
        serde_json::to_writer_pretty(
            File::create(path.join("prefix.json"))?,
            &json!({
                "requestedBlocks": options.prefix_blocks, "acceptedCursor": cursor,
                "totalBlocks": source.replay_ref().index.len(), "wallMs": opened.elapsed().as_secs_f64()*1000.0
            }),
        )?;
        return Ok(true);
    }
    let requests = requests(&source, options.requests.as_deref())?;
    let branch_text = requests.branch_id.as_ref().unwrap();
    let branch: u64 = branch_text.parse().map_err(|_| invalid("Invalid branch"))?;
    let blocks: Vec<_> = source
        .replay_ref()
        .index
        .iter()
        .filter(|block| block.branch == branch)
        .map(|block| (block.first_hour, block.last_hour))
        .collect();
    let coverage_start = Instant::now();
    let coverage = prepared::pipeline::coverage(&source, None, branch, &cancelled)?
        .ok_or_else(|| invalid("Missing accepted prefix coverage"))?;
    let coverage_ms = coverage_start.elapsed().as_secs_f64() * 1000.0;
    if coverage.complete {
        return Err(invalid(
            "Progressive measurements require an unprepared tail",
        ));
    }
    let tail_blocks = blocks
        .iter()
        .filter(|(_, last)| *last > coverage.last_hour)
        .count();
    if tail_blocks <= PROGRESSIVE_ZOOM_BLOCKS {
        return Err(invalid(
            "Whole recording needs more than thirteen unprepared blocks for rough mode",
        ));
    }
    let bounds = (
        *blocks.first().ok_or_else(|| invalid("Empty branch"))?,
        *blocks.last().unwrap(),
    );
    let hours = if mode == "zoom" {
        progressive_zoom(&blocks, coverage.last_hour)?
    } else {
        (bounds.0.0, bounds.1.1)
    };
    let gaps: Vec<_> = source
        .replay_ref()
        .gaps
        .iter()
        .filter(|gap| gap.branch == branch)
        .map(|gap| (gap.first_hour, gap.last_hour))
        .collect();
    let (unprepared_blocks, preceding_blocks) =
        progressive_blocks_selected(&blocks, &gaps, coverage.last_hour, hours);
    let request = ProductRequest {
        source_id: source.key().source_id.clone(),
        revision: source.key().revision.clone(),
        product: "series-summary".into(),
        params: json!({"branchId": branch_text, "family": {"kind":"country-measure", "measure":"politicalPower"},
            "tags": requests.tags, "from": hours.0, "to": hours.1, "width": 800, "withMarks": false}),
    };
    let query_start = Instant::now();
    let reply = prepared::query::query(&source, &request, None, &cancelled)?;
    let wall_ms = query_start.elapsed().as_secs_f64() * 1000.0;
    let open_to_first_summary_ms = opened.elapsed().as_secs_f64() * 1000.0;
    let reply: Value = serde_json::from_str(&reply)?;
    let data = &reply["data"];
    let series = data["series"]
        .as_array()
        .ok_or_else(|| invalid("Summary has no series"))?;
    if series.is_empty() {
        return Err(invalid("Summary returned no requested countries"));
    }
    let approximate = series.iter().any(|series| {
        series["summary"]["approximate"]
            .as_array()
            .is_some_and(|spans| !spans.is_empty())
    });
    if approximate != (mode == "rough") {
        return Err(invalid(
            "Summary approximation does not match requested mode",
        ));
    }
    serde_json::to_writer(
        File::create(path.join(format!("{mode}-reply.json")))?,
        &reply,
    )?;
    if mode == "rough" {
        progressive_row(
            output,
            json!({"measurement":"coverage", "hours":{"first":coverage.first_hour,"last":coverage.last_hour},
            "blocks":0, "wallMs":coverage_ms, "coverage":coverage, "replyCoverage":data["coverage"],
            "peakPrivateBytes":null, "memoryScope":"launcher fills sampled process peak private bytes"}),
        )?;
    }
    progressive_row(
        output,
        json!({"measurement": mode, "hours": {"first":hours.0,"last":hours.1},
        "blocks": unprepared_blocks + preceding_blocks,
        "blocksSelected":unprepared_blocks + preceding_blocks,
        "unpreparedBlocksSelected":unprepared_blocks, "leftEdgeObservationBlocksSelected":preceding_blocks,
        "blockCountScope":"blocks selected from index and accepted coverage; not measured reads",
        "wallMs":wall_ms, "openToFirstSummaryMs":open_to_first_summary_ms,
        "coverageWallMs":coverage_ms, "coverage":coverage, "replyCoverage":data["coverage"],
        "peakPrivateBytes":null, "memoryScope":"launcher fills sampled process peak private bytes",
        "coldScope":"fresh process, same accepted prefix store, no OS cache eviction", "targetWallMs":2000}),
    )?;
    Ok(true)
}

fn run(options: Options) -> io::Result<bool> {
    let path = isolated(&options.evidence)?;
    if options.readiness {
        let (Some(game), Some(user)) = (options.game, options.user) else {
            return Err(invalid(
                "Readiness opens with the content stack; supply the game and user folders",
            ));
        };
        return readiness_probe::run(
            readiness_probe::Settings {
                recording: options.recording,
                game,
                user,
                allow_unverified: options.allow_unverified,
                limit: options.readiness_limit,
                table: options.readiness_table,
                warm_content: !options.readiness_cold_content,
            },
            &path,
        );
    }
    if options.progressive.is_some() {
        return progressive_measure(options, &path);
    }
    let mut evidence = Evidence::new(path.clone())?;
    evidence.phase("setup")?;
    let context = LoadContext {
        content: match (options.game, options.user) {
            (Some(game_dir), Some(user_dir)) => Some(ContentOptions {
                options: installed_content_core::ResolveOptions {
                    game_dir,
                    user_dir,
                    cache_dir: path.join("cache/content"),
                },
                allow_unverified: options.allow_unverified,
                presentation: Default::default(),
            }),
            (None, None) => None,
            _ => return Err(invalid("Supply both the game and user folders, or neither")),
        },
    };
    fs::create_dir_all(path.join("cache/content"))?;
    if let Some(content) = &context.content {
        engine_executable::executable_tables::initialize(&content.options.game_dir)?;
    }
    let canonical_recording = fs::canonicalize(options.recording)?;
    let mut growth = Value::Null;
    let recording = if let Some(prefix_path) = options.prefix {
        let staging = path.join("append-source.battleplan");
        if fs::canonicalize(&prefix_path)? == staging || canonical_recording == staging {
            return Err(invalid(
                "The append evidence staging file must differ from supplied recordings",
            ));
        }
        fs::copy(prefix_path, &staging)?;
        let prefix = open(&staging)?;
        let prefix_index = prefix.replay_ref().index.clone();
        let prefix_prepared = prepare(&prefix, &context, &mut evidence, "prefix")?;
        drop(prefix);
        fs::copy(&canonical_recording, &staging)?;
        let source = open(&staging)?;
        let same_index = source.replay_ref().index.starts_with(&prefix_index);
        evidence.compare("growth/index-prefix", json!(true), json!(same_index))?;
        let prepared = prepare(&source, &context, &mut evidence, "appended")?;
        evidence.compare(
            "growth/new-blocks",
            json!(
                source
                    .replay_ref()
                    .index
                    .len()
                    .saturating_sub(prefix_index.len())
            ),
            prepared["newBlocks"].clone(),
        )?;
        evidence.compare(
            "growth/accepted-cursor",
            json!(source.replay_ref().index.len()),
            prepared["nextBlock"].clone(),
        )?;
        growth = json!({"prefix": prefix_prepared, "appended": prepared, "indexPrefixMatches": same_index});
        staging
    } else {
        canonical_recording.clone()
    };
    let source = open(&recording)?;
    let preparation = prepare(&source, &context, &mut evidence, "complete")?;
    evidence.phase("prepared-reopen")?;
    let reopen_started = Instant::now();
    let reopened = open(&recording)?;
    let restored =
        prepared::pipeline::restore(&reopened, context.content.as_ref(), &AtomicBool::new(false))?;
    let reopen_ms = reopen_started.elapsed().as_secs_f64() * 1000.0;
    evidence.compare(
        "reopen/prepared-restored",
        json!(true),
        json!(restored.is_some()),
    )?;
    evidence.compare(
        "reopen/new-blocks",
        json!(0),
        json!(restored.as_ref().map(|value| value.new_blocks)),
    )?;
    let reused = prepare(&reopened, &context, &mut evidence, "unchanged-reuse")?;
    evidence.compare(
        "unchanged-reuse/new-blocks",
        json!(0),
        reused["newBlocks"].clone(),
    )?;
    drop(reopened);
    let requests = requests(&source, options.requests.as_deref())?;
    serde_json::to_writer_pretty(File::create(path.join("requests.json"))?, &requests)?;
    let (_, key) = prepared::files(&source, context.content.as_ref())?;
    let stamps = serde_json::to_value(&key)?;
    let content_files = stamps["content"].as_array().map_or(0, Vec::len);
    for label in ["focus", "production", "measure", "map"]
        .into_iter()
        .filter(|_| !options.prepare_only)
    {
        evidence.phase(&format!("parity-and-timing/{label}"))?;
        let outcome = match label {
            "focus" => focus(
                &source,
                &context,
                &requests,
                &mut evidence,
                options.repetitions,
            ),
            "production" => production(
                &source,
                &context,
                &requests,
                &mut evidence,
                options.repetitions,
            ),
            "measure" => measure(
                &source,
                &context,
                &requests,
                &mut evidence,
                options.repetitions,
            ),
            "map" => map(
                &source,
                &context,
                &requests,
                &mut evidence,
                options.repetitions,
            ),
            _ => unreachable!(),
        };
        if let Err(error) = outcome {
            evidence.error(label, &error)?;
        }
    }
    evidence.replies.flush()?;
    evidence.phase("complete")?;
    evidence.differences_file.flush()?;
    let prepared_ready = preparation["sections"]
        .as_array()
        .is_some_and(|sections| sections.iter().all(|section| section["state"] == "ready"));
    let passed = evidence.differences == 0
        && evidence.errors == 0
        && evidence.skipped.is_empty()
        && (!options.prepare_only || prepared_ready);
    serde_json::to_writer_pretty(
        File::create(path.join("report.json"))?,
        &json!({
            "recording": canonical_recording, "sourcePath": recording,
            "preparationOnly": options.prepare_only, "parityPerformed": !options.prepare_only,
            "sourceKey": key, "recordedContentFileStamps": content_files,
            "preparation": preparation, "growth": growth, "preparedReopenMs": reopen_ms,
            "comparisons": evidence.comparisons, "differences": evidence.differences, "errors": evidence.errors,
            "skipped": evidence.skipped, "passedRequestedChecks": passed, "timings": evidence.timings,
            "measurementScope": {
                "query": "host-direct dispatch through coverage, content stamps, manifest reads and JSON serialization",
                "cache": "first in-process request and repeated in-process requests; no cold OS-cache claim",
                "memory": "launcher process working-set/private-memory samples include oracle scans/caches; no prepared-only memory bound is claimed",
                "reads": "explicit checkpoint header/source block reads only; no kernel I/O counter for product queries",
                "map": "shared prepared map Reader, selected native/trigger inputs, compressed checkpoints and updates; Campaign/Clock/DivisionStatus oracle comparison; excludes hour builders and presentation",
                "targets": {"t1QueryP99Ms": 2, "productionRankFirstPageMs": 50, "productionOneCountryMs": 10,
                            "mapStepMs": 1, "mapSeekMs": 50, "preparationMsPerGameYear": 10000},
                "limitations": ["No frontend paint/frame-rate measurement", "No GPU or native clock/presentation acceptance", "No automatic target approval"]
            }
        }),
    )?;
    println!(
        "{} comparisons, {} differences, {} errors; evidence {}",
        evidence.comparisons,
        evidence.differences,
        evidence.errors,
        path.display()
    );
    Ok(passed)
}

fn main() {
    let result = prepared::runner::worker_thread("dashboard acceptance")
        .spawn(|| options().and_then(run))
        .and_then(|worker| {
            worker
                .join()
                .unwrap_or_else(|_| Err(io::Error::other("Acceptance worker failed")))
        });
    match result {
        Ok(true) => {}
        Ok(false) => std::process::exit(1),
        Err(error) => {
            eprintln!("Acceptance harness: {error}");
            std::process::exit(2);
        }
    }
}

#[cfg(test)]
mod progressive_tests {
    use super::*;
    #[test]
    fn counts_match_tail_overlap_and_preceding_observation_rules() {
        let blocks: Vec<_> = (0..20).map(|i| (i * 10, i * 10 + 9)).collect();
        assert_eq!(
            progressive_blocks_selected(&blocks, &[], 29, (0, 199)),
            (17, 0)
        );
        assert_eq!(
            progressive_blocks_selected(&blocks, &[], 29, (30, 159)),
            (13, 1)
        );
        assert_eq!(
            progressive_blocks_selected(&blocks, &[(29, 29)], 29, (30, 159)),
            (13, 0)
        );
        assert_eq!(
            progressive_blocks_selected(&blocks, &[], 29, (35, 159)),
            (13, 0)
        );
        assert_eq!(
            progressive_blocks_selected(&blocks, &[], 34, (0, 159)),
            (13, 0)
        );
        assert_eq!(
            progressive_blocks_selected(&blocks, &[], 199, (0, 199)),
            (0, 0)
        );
        assert_eq!(
            progressive_blocks_selected(&blocks, &[], -1, (0, 129)),
            (13, 0)
        );
    }
    #[test]
    fn zoom_excludes_prepared_and_overlapping_blocks_and_requires_thirteen() {
        let blocks: Vec<_> = (0..20).map(|i| (i * 10, i * 10 + 9)).collect();
        assert_eq!(progressive_zoom(&blocks, 34).unwrap(), (40, 169));
        assert!(progressive_zoom(&blocks, 79).is_err());
    }
}

#[cfg(test)]
mod memory_sample_tests {
    use super::*;

    fn sampled<T>(
        path: Option<&Path>,
        label: &str,
        sample: &(dyn Fn(bool) -> Value + Sync),
        run: impl FnOnce() -> io::Result<T>,
    ) -> io::Result<(T, Duration)> {
        with_memory_samples_using(path, label, sample, run, |outcome| {
            let state = outcome
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let _wait = outcome
                .changed
                .wait_while(state, |state| *state == SAMPLING);
        })
    }

    /// This test's samples file in this process, removed first.
    fn samples(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "hoi4-acceptance-memory-samples-{}-{name}.jsonl",
            std::process::id()
        ));
        let _ = fs::remove_file(&path);
        path
    }

    /// The rows written to `path`, which is then removed.
    fn read_rows(path: &Path) -> Vec<Value> {
        let rows = fs::read_to_string(path)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        fs::remove_file(path).unwrap();
        rows
    }

    #[test]
    fn a_sampled_preparation_that_unwinds_ends_its_sampler_and_records_the_end() {
        let path = samples("unwinds");
        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            sampled(
                Some(path.as_path()),
                "test",
                &|_: bool| json!({"cacheBytes": "7"}),
                || -> io::Result<()> { panic!("injected preparation panic") },
            )
        }));
        assert!(unwound.is_err(), "the preparation's panic propagates");
        let rows = read_rows(&path);
        assert_eq!(rows[0]["at"], "start");
        let last = rows.last().unwrap();
        assert_eq!(
            (&last["at"], &last["outcome"]),
            (&json!("end"), &json!("unwound"))
        );
        assert!(rows.iter().all(|row| row["cacheBytes"] == "7"));
    }

    #[test]
    fn a_sampled_preparation_returns_its_result_or_error_and_records_it() {
        let path = samples("returns");
        let started = AtomicBool::new(false);
        let (value, _) = sampled(
            Some(path.as_path()),
            "test",
            &|_: bool| {
                started.store(true, std::sync::atomic::Ordering::Release);
                json!({"cacheBytes": "1"})
            },
            || {
                assert!(started.load(std::sync::atomic::Ordering::Acquire));
                Ok(5)
            },
        )
        .unwrap();
        assert_eq!(value, 5);
        let rows = read_rows(&path);
        assert_eq!(rows[0]["at"], "start");
        assert_eq!(rows.last().unwrap()["outcome"], "ok");
        let path = samples("fails");
        let error = sampled(
            Some(path.as_path()),
            "test",
            &|_: bool| json!({"cacheBytes": "1"}),
            || -> io::Result<()> { Err(io::Error::other("injected preparation error")) },
        )
        .unwrap_err();
        assert_eq!(error.to_string(), "injected preparation error");
        assert_eq!(read_rows(&path).last().unwrap()["outcome"], "error");
        // Without a path nothing is sampled.
        let (value, _) = sampled(None, "test", &|_: bool| -> Value { unreachable!() }, || {
            Ok(3)
        })
        .unwrap();
        assert_eq!(value, 3);
    }

    #[test]
    fn a_sampler_that_cannot_write_its_start_row_never_starts_preparation() {
        let path = samples("missing-parent").join("samples.jsonl");
        let error = sampled(Some(&path), "test", &|_| json!({}), || -> io::Result<()> {
            panic!("preparation started without memory evidence")
        })
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn a_census_row_names_the_earliest_reader_of_each_table_it_lists() {
        const TABLES: usize = replay_data::rows::TABLE_COUNT;
        let mut census = replay::container::SectionCacheCensus {
            bytes: 30,
            budget: 60,
            frontier: Some((1, 4)),
            readers: 3,
            largest: [0; TABLES],
            needed: [0; TABLES],
            obsolete: [0; TABLES],
            behind: [0; TABLES],
            window: [0; TABLES],
            beyond: [0; TABLES],
            retaining: [None; TABLES],
        };
        let (countries, clock, campaign) = (
            TableId::Countries.index(),
            TableId::Clock.index(),
            TableId::Campaign.index(),
        );
        census.needed[countries] = 10;
        census.retaining[countries] = Some("walk");
        census.obsolete[clock] = 20;
        census.retaining[clock] = Some("groups");
        census.retaining[campaign] = Some("pages");
        let row = census_json(&census, "7".into());
        let listed: BTreeMap<String, Value> = row["tables"]
            .as_array()
            .unwrap()
            .iter()
            .map(|table| (table["table"].as_str().unwrap().to_owned(), table.clone()))
            .collect();
        assert_eq!(listed["Countries"]["earliestReader"], "walk");
        assert_eq!(listed["Clock"]["earliestReader"], "groups");
        assert_eq!(listed.len(), 2, "a table holding nothing is not listed");
        assert!(
            row["earliestReaderMeans"]
                .as_str()
                .unwrap()
                .contains("not the owner")
        );
    }

    #[test]
    fn the_start_every_tenth_and_the_end_row_carry_the_full_sample() {
        assert!(full_row(0, false) && full_row(10, false) && full_row(13, true));
        assert!(!full_row(1, false) && !full_row(13, false));
        let path = samples("full");
        let outcome = Sampling::new();
        let mut ticks = 0;
        sample_memory_with_ticks(
            &path,
            "test",
            &|full| json!({"cacheBytes": "1", "full": full}),
            &outcome,
            || {
                ticks += 1;
                if ticks == 12 {
                    outcome.finish(RAN);
                }
            },
            || {},
        )
        .unwrap();
        let rows = read_rows(&path);
        let last = rows.len() - 1;
        assert_eq!(rows[last]["at"], "end");
        for (at, row) in rows.iter().enumerate() {
            let full = full_row(at as u64, at == last);
            assert_eq!(row["full"], full, "row {at} of {last}");
        }
    }
}
