//! Measures a native preparation of a recording as production prepares it
//! ([`dashboard::prepared::driver::native`]): every family with a native backend, or those
//! `--family` names, prepared from the native recording into a scratch store under `--output`,
//! after the change-run index production prepares first.
//! Reports what it cost per game hour beside the goal it is judged on: each family's operators,
//! ordered emit and publication, the walks every lane shares, the slowest serial lane and the
//! peak resident bytes, with every status row that failed. `served-check` checks the replies.
//! `columnar-family-check --recording <r.battleplan> --columnar <c.h4col> --output <new folder>
//!   [--family F[,F...]|ported] [--threads N] [--content-cache DIR] [--scopes] [--walk-only]`
//! Game content comes from BATTLE_PLAN_GAME_DIR and BATTLE_PLAN_USER_DIR, or none when neither is set. Its
//! hashes are kept in `--content-cache`, by default the app's own cache, so a run resolves it as
//! the app would; the report says what the resolve read.
//! `--scopes` also reports the CPU each accounting scope charged. `--walk-only` walks the runs
//! with one lane doing no family's work, for the floor decode, apply and the world cost.
//! report.json also holds the timeline of every task and phase, from the process's start.
//! Built with the `memory-labels` feature, its `memory` section says, for the index run, the
//! union run and the phases between, what each family and each part of the run held at the
//! peak, as the allocator counted it, beside what the system and mimalloc held then.

use dashboard::{
    config::{Config, ContentOptions},
    prepared::driver::native,
};
use engine_allocation::labels::{self, Kind, Phase};
use replay::columnar_frames::{Limits, Recording};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io,
    path::PathBuf,
    sync::atomic::AtomicBool,
};

#[path = "support/shader_compiler.rs"]
mod shader_compiler;

#[cfg(not(feature = "memory-labels"))]
#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[cfg(feature = "memory-labels")]
#[global_allocator]
static ALLOCATOR: labels::Counting<mimalloc::MiMalloc> = labels::Counting(mimalloc::MiMalloc);

const USAGE: &str = "columnar-family-check --recording R --columnar C --output NEW \
                     [--family F[,F...]|ported] [--threads N] [--content-cache DIR] [--scopes] [--walk-only]";

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, what.into())
}

/// The content the families read, its hashes kept in `cache_dir`; none when neither folder is set.
fn content(cache_dir: io::Result<PathBuf>) -> io::Result<Option<ContentOptions>> {
    let (game_dir, user_dir) = match (
        std::env::var_os("BATTLE_PLAN_GAME_DIR").or_else(|| std::env::var_os("HOI4_GAME_DIR")),
        std::env::var_os("BATTLE_PLAN_USER_DIR").or_else(|| std::env::var_os("HOI4_USER_DIR")),
    ) {
        (Some(game), Some(user)) => (PathBuf::from(game), PathBuf::from(user)),
        (None, None) => return Ok(None),
        _ => {
            return Err(invalid(
                "Set both BATTLE_PLAN_GAME_DIR and BATTLE_PLAN_USER_DIR, or neither",
            ));
        }
    };
    engine_executable::executable_tables::initialize(&game_dir)?;
    let cache_dir = cache_dir?;
    std::fs::create_dir_all(&cache_dir)?;
    Ok(Some(ContentOptions {
        options: installed_content_core::ResolveOptions {
            game_dir,
            user_dir,
            cache_dir,
        },
        allow_unverified: false,
        presentation: Default::default(),
    }))
}

fn run() -> io::Result<()> {
    let (mut recording, mut columnar, mut output) = (None, None, None);
    let mut families: Option<String> = None;
    let mut threads = std::thread::available_parallelism().map_or(1, usize::from);
    let (mut scopes, mut walk_only) = (false, false);
    let mut content_cache = None;
    let mut args = std::env::args_os().skip(1);
    while let Some(flag) = args.next() {
        if flag == "--scopes" {
            scopes = true;
            continue;
        }
        if flag == "--walk-only" {
            walk_only = true;
            continue;
        }
        let value = args.next().ok_or_else(|| invalid(USAGE))?;
        match flag.to_str() {
            Some("--recording") => recording = Some(PathBuf::from(value)),
            Some("--columnar") => columnar = Some(PathBuf::from(value)),
            Some("--output") => output = Some(PathBuf::from(value)),
            Some("--family") => families = Some(value.to_string_lossy().into_owned()),
            Some("--content-cache") => content_cache = Some(PathBuf::from(value)),
            Some("--threads") => {
                threads = value
                    .to_str()
                    .and_then(|text| text.parse().ok())
                    .filter(|&count| count > 0)
                    .ok_or_else(|| invalid(USAGE))?
            }
            _ => return Err(invalid(USAGE)),
        }
    }
    let (Some(recording), Some(columnar), Some(output)) = (recording, columnar, output) else {
        return Err(invalid(USAGE));
    };
    let named: Option<Vec<&str>> = families
        .as_deref()
        .filter(|list| *list != "ported")
        .map(|list| list.split(',').collect());
    std::fs::create_dir(&output)?;
    // Read before the scratch store moves LOCALAPPDATA.
    let content_cache = content_cache.map_or_else(Config::content_cache_dir, Ok);
    let local = output.join("local");
    std::fs::create_dir_all(&local)?;
    let shader_compiler = shader_compiler::configure();
    // Set before any other thread starts: the scratch store lives under it.
    unsafe { std::env::set_var("LOCALAPPDATA", &local) };
    if scopes {
        replay_frame::cpu::enable();
    }
    let span = replay_frame::timeline::span("game content");
    let content = content(content_cache)?;
    drop(span);
    let span = replay_frame::timeline::span("source open");
    let source = dashboard::Source::open(
        dashboard::SourceKey {
            source_id: "columnar-family-check".into(),
            revision: dashboard::protocol::replay_revision(&recording)?,
        },
        recording.clone(),
        &AtomicBool::new(false),
    )?;
    drop(span);
    let cancelled = AtomicBool::new(false);
    // The content resolves beside the index, as production prepares it.
    let options = content.as_ref().map(|content| &content.options);
    let (index, index_seconds, index_cpu, index_working, index_commit, damage, accepted, measured) =
        source.resolving_ahead(options, &cancelled, || -> io::Result<_> {
            // The change-run index first, as production prepares it: native readers serve from it.
            let span = replay_frame::timeline::span("index");
            labels::phase("index run");
            let started = std::time::Instant::now();
            let cpu_before = replay_frame::cpu::process_cpu_ns();
            let index = dashboard::prepared::change_runs::prepare(&source, &cancelled)?;
            let index_seconds = started.elapsed().as_secs_f64();
            let index_cpu = cpu_before
                .zip(replay_frame::cpu::process_cpu_ns())
                .map(|(before, after)| after.saturating_sub(before) as f64 / 1e9);
            // The process's peaks so far: the index is its first heavy work.
            let (index_working, index_commit) = native::peaks()?;
            labels::phase("families open");
            drop(span);
            let span = replay_frame::timeline::span("recording open");
            let file = Recording::open_path(&columnar, Limits::default())?;
            let damage: Vec<String> = file.damage().iter().map(ToString::to_string).collect();
            drop(span);
            let span = replay_frame::timeline::span("families");
            let (accepted, measured) = native::measure(
                &source,
                content.as_ref(),
                file,
                named.as_deref(),
                threads,
                walk_only,
                &cancelled,
            )?;
            drop(span);
            Ok((index, index_seconds, index_cpu, index_working, index_commit, damage, accepted, measured))
        })?;
    let failed: Vec<_> = accepted
        .sections
        .iter()
        .filter(|section| section.error.is_some())
        .collect();
    let runs: Vec<native::Run> = index.runs.into_iter().map(native::Run::from).collect();
    let resolved = content.as_ref().map(|content| {
        let stats = source
            .bound_content(&content.options)
            .map(|resolved| resolved.stats.clone());
        json!({
            "cache": content.options.cache_dir,
            // Nothing read: every file's digest came from the cache.
            "warm": stats.as_ref().map(|stats| stats.files_read == 0),
            "stats": stats.map(|stats| json!({
                "files": stats.files,
                "bytes": stats.bytes,
                "filesRead": stats.files_read,
                "bytesRead": stats.bytes_read,
                "cacheHits": stats.cache_hits,
                "milliseconds": u64::try_from(stats.elapsed_ms).ok(),
            })),
        })
    });
    let mut report = json!({
        "shaderCompiler": shader_compiler,
        "recording": recording,
        "columnar": columnar,
        "damage": damage,
        "measured": measured,
        "index": {
            "seconds": index_seconds,
            "refused": index.refused,
            "cpu_seconds": index_cpu,
            "peak_working_set_bytes": index_working,
            "peak_commit_bytes": index_commit,
            "runs": runs,
        },
        "content": resolved,
        "failed": failed,
        "scopes": scopes.then(replay_frame::cpu::snapshot),
        "memory": memory(),
    });
    let mut shown = String::new();
    render(&report, 0, &mut shown);
    println!("{shown}");
    // Printed without it: it holds a span for every task.
    report["timeline"] = replay_frame::timeline::snapshot();
    let mut written = String::new();
    render(&report, 0, &mut written);
    std::fs::write(output.join("report.json"), written)?;
    Ok(())
}

/// What each phase held, as the counting allocator charged it to its labels, beside the
/// system's and mimalloc's counts; why not, when this build counts nothing.
fn memory() -> Value {
    if !labels::ENABLED {
        return json!("Built without the memory-labels feature: nothing was counted");
    }
    let (version, options) = dashboard::allocator::retention();
    let options: serde_json::Map<String, Value> = options
        .into_iter()
        .map(|(name, value)| (name.to_owned(), json!(value)))
        .collect();
    json!({
        "allocator": {"name": "mimalloc", "version": version, "options": options},
        "phases": labels::phases().iter().map(phase_memory).collect::<Vec<_>>(),
    })
}

/// What a family held at the peak, by kind, and as its phase ended.
#[derive(Default)]
struct Held {
    /// Its operators' own: what they allocated and held then, less their output in flight.
    operator: i64,
    in_flight: i64,
    emit: i64,
    publish: i64,
    /// Of what it publishes: its series writers' key state, and its seals.
    keys: i64,
    seal: i64,
    content: i64,
    emit_end: i64,
    end: i64,
}

impl Held {
    fn at_peak(&self) -> i64 {
        self.operator
            + self.in_flight
            + self.emit
            + self.publish
            + self.keys
            + self.seal
            + self.content
    }
}

fn phase_memory(phase: &Phase) -> Value {
    let at = |owner: &str, kind: Kind| {
        phase
            .labels
            .iter()
            .find(|charge| charge.owner == owner && charge.kind == kind)
            .map_or(0, |charge| charge.at_peak)
    };
    let tracked = phase.tracked_at_peak;
    let less_tracked = |held: usize| i64::try_from(held).ok().map(|held| held - tracked);
    let os = phase.os_at_peak;
    let mut kinds: BTreeMap<&str, (i64, i64)> = BTreeMap::new();
    let mut families: BTreeMap<&str, Held> = BTreeMap::new();
    for charge in &phase.labels {
        let kind = kinds.entry(charge.kind.name()).or_default();
        kind.0 += charge.at_peak;
        kind.1 += charge.end;
        if charge.owner == "run" {
            continue;
        }
        let held = families.entry(charge.owner).or_default();
        held.end += charge.end;
        let in_flight = charge.in_flight.unwrap_or(0);
        match charge.kind {
            Kind::Operator | Kind::Restore => {
                held.operator += charge.at_peak - in_flight;
                held.in_flight += in_flight;
            }
            Kind::Emit => {
                held.emit += charge.at_peak;
                held.emit_end += charge.end;
            }
            Kind::Publish => held.publish += charge.at_peak,
            Kind::Keys => held.keys += charge.at_peak,
            Kind::Seal => held.seal += charge.at_peak,
            Kind::Content => held.content += charge.at_peak,
            _ => {}
        }
    }
    let mut ordered: Vec<(&str, &Held)> =
        families.iter().map(|(name, held)| (*name, held)).collect();
    ordered.sort_by_key(|(_, held)| std::cmp::Reverse(held.at_peak()));
    let top = |bytes: fn(&Held) -> i64| {
        let mut top: Vec<(&str, i64)> = ordered
            .iter()
            .map(|(name, held)| (*name, bytes(held)))
            .collect();
        top.sort_by_key(|(_, bytes)| std::cmp::Reverse(*bytes));
        top.into_iter()
            .take(10)
            .map(|(family, bytes)| json!({"family": family, "bytes": bytes}))
            .collect::<Vec<_>>()
    };
    let mut charges: Vec<_> = phase.labels.iter().collect();
    charges.sort_by_key(|charge| std::cmp::Reverse(charge.at_peak));
    let (started, ended) = (phase.os_started, phase.os_ended);
    // A phase's own peak is known only where it raised the process's.
    let raised = |peak: fn(&labels::Os) -> usize| {
        ended.and_then(|end| {
            started
                .is_none_or(|start| peak(&end) > peak(&start))
                .then(|| peak(&end))
        })
    };
    let index = families.get("change runs").map(|held| {
        let lane = |kind: Kind| at("change runs", kind);
        json!({
            // The walks' own, the builders' scratch among them, past what is named below.
            "operatorBytes": held.operator,
            "cursorBytes": lane(Kind::IndexCursors),
            "spillBufferBytes": lane(Kind::IndexSpill),
            "pieceDirectoryBytes": lane(Kind::IndexPieces),
            "piecesInFlightBytes": held.in_flight,
            "directoryBytes": held.emit + held.publish + at("run", Kind::Index),
            "walkerBytes": at("run", Kind::Walker),
            "walkerTablesBytes": at("run", Kind::WalkerTables),
        })
    });
    json!({
        "phase": phase.name,
        "trackedPeakBytes": phase.tracked_peak,
        "atPeak": {
            "trackedBytes": tracked,
            "headerBytes": at("run", Kind::Headers),
            "commitBytes": os.map(|os| os.commit),
            "workingSetBytes": os.map(|os| os.working),
            "allocatorCommitBytes": os.and_then(|os| os.allocator),
            // Freed pages mimalloc keeps committed, and its fragmentation.
            "allocatorRetainedBytes": os.and_then(|os| os.allocator).and_then(less_tracked),
            // C libraries' own heaps, thread stacks, and the system's and drivers' memory.
            "outsideAllocatorBytes": os.and_then(|os| {
                os.allocator.map(|held| os.commit as i64 - held as i64)
            }),
            "untrackedCommitBytes": os.and_then(|os| less_tracked(os.commit)),
        },
        "process": {
            "peakCommitBytes": raised(|os| os.peak_commit),
            "peakWorkingSetBytes": raised(|os| os.peak_working),
            "processPeakCommitBytes": ended.map(|os| os.peak_commit),
            "processPeakWorkingSetBytes": ended.map(|os| os.peak_working),
            "endCommitBytes": ended.map(|os| os.commit),
            "endWorkingSetBytes": ended.map(|os| os.working),
        },
        "kinds": kinds.iter().map(|(kind, (at_peak, end))| {
            json!({"kind": kind, "atPeakBytes": at_peak, "endBytes": end})
        }).collect::<Vec<_>>(),
        "changeRuns": index,
        "top": {
            "operator": top(|held| held.operator),
            "outputInFlight": top(|held| held.in_flight),
            "emit": top(|held| held.emit),
            "publish": top(|held| held.publish),
            "keys": top(|held| held.keys),
            "seal": top(|held| held.seal),
        },
        "families": ordered.iter().map(|(family, held)| json!({
            "family": family,
            "atPeakBytes": held.at_peak(),
            "operatorBytes": held.operator,
            "outputInFlightBytes": held.in_flight,
            "emitBytes": held.emit,
            "publishBytes": held.publish,
            "keysBytes": held.keys,
            "sealBytes": held.seal,
            "contentBytes": held.content,
            "emitEndBytes": held.emit_end,
            "endBytes": held.end,
        })).collect::<Vec<_>>(),
        "labels": charges.iter().map(|charge| json!({
            "owner": charge.owner,
            "kind": charge.kind.name(),
            "peakBytes": charge.peak,
            "atPeakBytes": charge.at_peak,
            "endBytes": charge.end,
            "inFlightAtPeakBytes": charge.in_flight,
        })).collect::<Vec<_>>(),
        "overflowedFamilies": phase.overflowed,
    })
}

/// `value` as indented JSON, but each array of scalars on one line: a timeline's spans, one to
/// a line.
fn render(value: &Value, indent: usize, out: &mut String) {
    let scalar = |value: &Value| !matches!(value, Value::Array(_) | Value::Object(_));
    let open = |out: &mut String, at: usize| {
        out.push_str(if at == 0 { "\n" } else { ",\n" });
        out.extend(std::iter::repeat_n(' ', indent + 2));
    };
    let close = |out: &mut String, end: char| {
        out.push('\n');
        out.extend(std::iter::repeat_n(' ', indent));
        out.push(end);
    };
    match value {
        Value::Object(fields) if !fields.is_empty() => {
            out.push('{');
            for (at, (name, field)) in fields.iter().enumerate() {
                open(out, at);
                out.push_str(&Value::from(name.as_str()).to_string());
                out.push_str(": ");
                render(field, indent + 2, out);
            }
            close(out, '}');
        }
        Value::Array(items) if !items.iter().all(scalar) => {
            out.push('[');
            for (at, item) in items.iter().enumerate() {
                open(out, at);
                render(item, indent + 2, out);
            }
            close(out, ']');
        }
        _ => out.push_str(&value.to_string()),
    }
}

fn main() {
    replay_frame::timeline::enable();
    // Measured with the memory settings the data service runs with.
    dashboard::allocator::return_freed_memory_promptly();
    labels::measure_allocator(dashboard::allocator::committed);
    if let Err(error) = run() {
        eprintln!("columnar-family-check: {error}");
        std::process::exit(1);
    }
}
