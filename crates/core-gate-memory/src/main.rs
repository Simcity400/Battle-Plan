mod cpu;
mod detail;
mod encoding;
mod gate;
mod grouping;
mod manifest;
mod memory;
mod reads;
mod report;
mod reverse;
#[cfg(test)]
mod synthetic_tests;

use detail::Detail;
use gate::{Admission, Lost, SegmentReport};
use manifest::{Manifest, invalid};
use replay::{columnar_frames::Recording, container::Replay};
use replay_data::{
    codec::Decoder,
    identity::{Identity, IdentityCodec},
};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    fs::File,
    io::{self, BufWriter, Seek, SeekFrom, Write},
    path::PathBuf,
    sync::atomic::Ordering,
};

struct Args {
    recording: PathBuf,
    columnar: PathBuf,
    manifest: PathBuf,
    out: PathBuf,
    threads: usize,
}

impl Args {
    fn read() -> io::Result<Self> {
        let mut args = std::env::args_os().skip(1);
        let mut values = BTreeMap::new();
        while let Some(name) = args.next() {
            if name == "--help" {
                println!(
                    "core-gate-memory --recording <.battleplan> --columnar <.h4col> --manifest <toml> --out <new empty folder> --threads N"
                );
                std::process::exit(0);
            }
            let name = name
                .into_string()
                .map_err(|_| invalid("Non-UTF-8 option"))?;
            if !matches!(
                name.as_str(),
                "--recording" | "--columnar" | "--manifest" | "--out" | "--threads"
            ) {
                return Err(invalid(format!("Unknown option {name}")));
            }
            let value = args
                .next()
                .ok_or_else(|| invalid(format!("Missing value for {name}")))?;
            if values.insert(name.clone(), value).is_some() {
                return Err(invalid(format!("Repeated option {name}")));
            }
        }
        let mut take = |name: &str| {
            values
                .remove(name)
                .ok_or_else(|| invalid(format!("Required option {name}")))
        };
        let recording = take("--recording")?.into();
        let columnar = take("--columnar")?.into();
        let manifest = take("--manifest")?.into();
        let out = take("--out")?.into();
        let threads = take("--threads")?
            .to_str()
            .and_then(|s| s.parse().ok())
            .filter(|&n: &usize| n > 0)
            .ok_or_else(|| invalid("--threads must be a positive integer"))?;
        Ok(Self {
            recording,
            columnar,
            manifest,
            out,
            threads,
        })
    }
}

#[derive(Serialize)]
struct Tool {
    wall_seconds: f64,
    process_cpu_seconds: f64,
    peak_commit_bytes: usize,
    peak_working_set_bytes: usize,
    under_three_gb: bool,
    requested_threads: usize,
    maximum_worker_threads: usize,
    maximum_tool_threads_including_main: usize,
    peak_active_readers: usize,
    thread_cycle_hz: f64,
    cpu_clock: &'static str,
}

#[derive(Serialize)]
struct Lane {
    family: String,
    records: u64,
    bytes: u64,
    replay_cycles: u64,
    replay_us_per_hour: f64,
    fifty_year_seconds: f64,
    within_allowance: bool,
    segments: Detail<SegmentDelivery>,
}

#[derive(serde::Deserialize, Serialize)]
struct SegmentDelivery {
    segment: usize,
    hours: u64,
    delivery: encoding::Delivery,
    replay_us_per_hour: f64,
}

#[derive(Serialize)]
struct Report {
    version: u32,
    recording: PathBuf,
    columnar: PathBuf,
    metadata: serde_json::Value,
    manifest: manifest::Manifest,
    manifest_provenance: serde_json::Value,
    definitions: serde_json::Value,
    recorded_hours: u64,
    measured_hours: u64,
    segments: Detail<SegmentReport>,
    excluded_damage: Detail<Lost>,
    damage_count: usize,
    totals: serde_json::Value,
    groupings: Vec<grouping::Scenario>,
    group_walks: Vec<grouping::Walk>,
    shared_walk: Option<usize>,
    ordered_lanes: Vec<Lane>,
    verdict: String,
}

fn identity(identity: &Identity) -> serde_json::Value {
    serde_json::json!({
        "game_version": identity.game_version, "language": identity.language,
        "vanilla_hash": identity.vanilla_hash,
        "dlc": identity.dlc.iter().map(|d| serde_json::json!({"id":d.id,"hash":d.hash,"declared_checksum":d.declared_checksum})).collect::<Vec<_>>(),
        "mods_in_recorded_order": identity.mods.iter().map(|m| serde_json::json!({"id":m.id,"display_name":m.display_name,"declared_version":m.declared_version,"hash":m.hash})).collect::<Vec<_>>(),
        "verification": format!("{:?}",identity.verification), "coverage_policy": identity.coverage_policy,
    })
}

fn metadata(args: &Args, columnar: &Recording) -> io::Result<serde_json::Value> {
    let replay = Replay::open_path(&args.recording)?;
    if replay.campaign_id != columnar.campaign() {
        return Err(invalid(
            "Row and columnar recordings name different campaigns",
        ));
    }
    let mut decoder = Decoder::new(&columnar.root_identity().content);
    let content = Identity::decode(&mut decoder)?;
    decoder.finish()?;
    if content != replay.identity {
        return Err(invalid(
            "Columnar content identity differs from the row recording",
        ));
    }
    Ok(serde_json::json!({
        "campaign_id_bytes": replay.campaign_id, "format_version": replay.format.version(),
        "content": identity(&replay.identity),
        "sessions": replay.sessions.iter().map(|s| serde_json::json!({"session":s.id,"content":identity(&s.identity)})).collect::<Vec<_>>(),
        "warnings": replay.warnings, "damaged_parts": replay.damaged_parts,
        "capture_mode": format!("{:?}",replay.capture_mode),
    }))
}

fn run(args: Args) -> io::Result<()> {
    let clock = cpu::Clock::start()?;
    let process_start = cpu::process_ns()?;
    let resolved = Manifest::read(&args.manifest)?;
    let shipped: Manifest = toml::from_str(include_str!("../manifest.toml"))
        .map_err(|e| invalid(format!("Invalid built-in family list: {e}")))?;
    let names: std::collections::BTreeSet<_> =
        resolved.manifest.families.iter().map(|f| &f.name).collect();
    let expected: std::collections::BTreeSet<_> =
        shipped.families.iter().map(|f| &f.name).collect();
    if names != expected {
        return Err(invalid(
            "Gate 1 requires exactly the 68 c13 measured families",
        ));
    }
    let shared_names: std::collections::BTreeSet<_> =
        resolved.manifest.shared.iter().map(|o| &o.name).collect();
    let expected_shared: std::collections::BTreeSet<_> =
        shipped.shared.iter().map(|o| &o.name).collect();
    if shared_names != expected_shared {
        return Err(invalid("Gate 1 requires the shared trigger-context owner"));
    }
    drop(expected_shared);
    drop(shared_names);
    drop(expected);
    drop(names);
    drop(shipped);
    if args.out.exists() {
        if !args.out.is_dir() || std::fs::read_dir(&args.out)?.next().is_some() {
            return Err(invalid("--out must be a new empty directory"));
        }
    } else {
        std::fs::create_dir_all(&args.out)?;
    }
    let mut recording = Recording::open_path(&args.columnar, gate::limits())?;
    recording.select(&resolved.tables);
    let metadata = metadata(&args, &recording)?;
    let mut layout = memory::Layout::new(&resolved.tables, resolved.relations.len());
    let previous = manifest::union(resolved.families.iter().flatten().filter(|s| s.previous));
    layout.hold_previous(&previous);
    let admission = Admission::new();
    let segments = Detail::<SegmentReport>::new(
        &args.out.join("segment-reports"),
        recording.segments().len(),
    )?;
    let excluded_damage = Detail::<Lost>::new(
        &args.out.join("damage-reports"),
        recording.segments().len() + recording.damage().len(),
    )?;
    let format = replay_data::format::FormatVersion::read(
        metadata["format_version"]
            .as_u64()
            .ok_or_else(|| invalid("Metadata has no format version"))? as u16,
    )?;
    gate::parallel(recording.segments().len(), args.threads, |index| {
        let _permit = admission.union()?;
        eprintln!("union segment {}/{}", index + 1, recording.segments().len());
        match gate::measure(index, &recording, &resolved, &layout, format, &args.out) {
            Ok(segment) => {
                segments.write(index, Some(&segment))?;
                excluded_damage.write(index, None)
            }
            Err(error) => {
                match gate::damage(index, &recording.segments()[index], &recording, &error) {
                    Some(damage) => {
                        segments.write(index, None)?;
                        excluded_damage.write(index, Some(&damage))
                    }
                    None => Err(error),
                }
            }
        }
    })?;
    for (at, damage) in recording.damage().iter().enumerate() {
        excluded_damage.write(
            recording.segments().len() + at,
            Some(&Lost {
                segment: None,
                run: None,
                first_ordinal: None,
                last_ordinal: None,
                first_hour: None,
                last_hour: None,
                scope: damage.scope.to_string(),
                reason: damage.to_string(),
            }),
        )?;
    }
    let (_, rate) = clock.elapsed()?;
    let mut union_timing = gate::Timing::default();
    let mut boundaries = gate::Boundaries::default();
    for at in 0..segments.slots {
        if let Some(mut segment) = segments.read(at)? {
            segment.timing.calibrate(rate);
            for index in &mut segment.reverse {
                index.bootstrap_us_per_hour =
                    Some(index.bootstrap_cycles as f64 / rate * 1e6 / segment.timing.hours as f64);
                index.upkeep_us_per_hour =
                    Some(index.upkeep_cycles as f64 / rate * 1e6 / segment.timing.hours as f64);
            }
            if boundaries.restore_is_duplicate(
                segment.run,
                segment.first_ordinal,
                segment.last_ordinal,
            ) {
                segment.restore_checkpoint_verification = Some(segment.checkpoints.remove(0));
            }
            union_timing.hours += segment.timing.hours;
            union_timing.restore_cycles += segment.timing.restore_cycles;
            union_timing.hourly_decode_apply_cycles += segment.timing.hourly_decode_apply_cycles;
            segments.replace(at, Some(&segment))?;
        }
    }
    let hours = union_timing.hours;
    let mut groupings = if hours == 0 {
        Vec::new()
    } else {
        grouping::plans(&resolved, &layout, &segments, &args.out)?
    };
    grouping::peaks(&mut groupings, &layout, &segments)?;
    let (group_walks, shared_walk) = if hours == 0 {
        (Vec::new(), None)
    } else {
        eprintln!("measuring unique family-group and shared-owner table selections");
        let shared_tables: Vec<_> = resolved.shared_union.iter().map(|s| s.table).collect();
        let (walks, shared) = grouping::walks(
            &mut groupings,
            &segments,
            &args.columnar,
            args.threads,
            rate,
            &admission,
            &resolved.tables,
            &shared_tables,
            &args.out,
            hours,
            &union_timing,
        )?;
        (walks, Some(shared))
    };
    let mut ordered_lanes = Vec::new();
    if hours != 0 {
        for family in resolved.manifest.families.iter().filter(|f| f.ordered) {
            let mut lane = Lane {
                family: family.name.clone(),
                records: 0,
                bytes: 0,
                replay_cycles: 0,
                replay_us_per_hour: 0.0,
                fifty_year_seconds: 0.0,
                within_allowance: false,
                segments: Detail::new(
                    &args.out.join(format!("delivery-{}", family.name)),
                    segments.slots,
                )?,
            };
            for at in 0..segments.slots {
                if let Some(segment) = segments.read(at)? {
                    let delivery = encoding::deliver(&segment.spools[&family.name])?;
                    lane.records += delivery.records;
                    lane.bytes += delivery.bytes;
                    lane.replay_cycles += delivery.cycles;
                    let replay_us_per_hour =
                        delivery.cycles as f64 / rate * 1e6 / segment.timing.hours as f64;
                    lane.segments.write(
                        at,
                        Some(&SegmentDelivery {
                            segment: segment.index,
                            hours: segment.timing.hours,
                            delivery,
                            replay_us_per_hour,
                        }),
                    )?;
                } else {
                    lane.segments.write(at, None)?;
                }
            }
            lane.replay_us_per_hour = lane.replay_cycles as f64 / rate * 1e6 / hours as f64;
            lane.fifty_year_seconds = lane.replay_us_per_hour * 438_000.0 / 1e6;
            lane.within_allowance = lane.replay_us_per_hour <= 29.7;
            ordered_lanes.push(lane);
        }
    }
    let verdict = if hours == 0 {
        "No verdict: no undamaged measured segments.".into()
    } else if let Some(plan) = groupings
        .iter()
        .find(|p| p.readers_fitting_largest_first >= 8)
    {
        format!(
            "{} groups admit {} readers within 0.9 GB; duplicated hourly decode-plus-apply {:.3} us/h.",
            plan.count,
            plan.readers_fitting_largest_first,
            plan.duplicated_decode_us_per_hour.unwrap()
        )
    } else {
        "No group count admits eight readers within 0.9 GB.".into()
    };
    let provenance = report::provenance(&resolved.manifest);
    let totals = report::totals(&segments, &ordered_lanes, rate)?;
    let mut damage_count = 0;
    excluded_damage.for_each(|_| {
        damage_count += 1;
        Ok(())
    })?;
    let report = Report {
        version: 1,
        recording: args.recording,
        columnar: args.columnar,
        metadata,
        manifest: resolved.manifest,
        manifest_provenance: provenance,
        definitions: serde_json::json!({
            "cpu":"QueryThreadCycleTime around Walker::begin and between hourly walk callbacks; decode and apply combined; every column of selected tables decoded. The final page checks and dictionary trim are included as tail cycles.",
            "cycle_calibration":"Invariant x86-64 TSC delta / measured wall seconds, as dashboard measure.rs; all raw cycle counts retained.",
            "cell_bytes":"Live cells: Kind::slot_bytes plus exact list/opaque payload; one recorded bit per cell, optional presence bit, rounded to 64-bit words per column. Text cells hold 32-bit dictionary IDs. Dictionary allocations reported separately.",
            "keys":"16-byte key and 4-byte slot per live row, plus 64-bit live words; compact sorted key/slot representation; no dead-slot slack counted in the floor. Actual complete-table allocation peaks also reported.",
            "previous":"Declared previous lookups retain the previous hour's named keys/cells. No previous hour is materialized at a segment's restore.",
            "reverse":"Concrete sorted vector of (typed value, u128 row key), allocated vector capacity and owned text payload. Rebuilt only when an input changes; this is measurement upkeep CPU, not a future reader estimate.",
            "checkpoint_encoding":"Version 1: u64 LE record length; i32 LE hour, u8 event, u16 LE table, u128 LE key; status/sample or repeated u16 field, u8 recorded, u8 cell-kind and absolute LE scalar/presence/list. Lengths are u64 LE, strings inline, opaque bytes lossless; unknowns retain their decode bases. Raw and single-thread zstd level 3 streams.",
            "spool":"The same absolute record framing, restored rows followed by changed keys/cells, deletions, statuses, read evidence and hour/break records; lookups carried as their source changes. Ordered delivery reads zstd-3 segments in recording order to a no-op consumer, with no family handler.",
            "shared_owner":"The trigger-context owner is a whole-table upper bound, outside family weights, group bytes and ordered spools. Admission charges it once per distinct admitted segment; decode includes its independent walk once per segment.",
            "tool_lifetime":"Counters include aggregation and the flushed JSON/Markdown report bodies. Only appending these fixed-size counter footers follows the final sample.",
            "grouping":"For each count 1..68, sort families by maximum measured named-cell/key peak descending, ties by name; assign to least sum of family peaks, ties by member count then group index. Union fields and identical reverse relations within each group, share nothing across groups; shared owners are separate. Exact group peaks replay census changes at each hour. Admit the descending prefix of all (segment,group) peaks; charge shared owners once per distinct admitted segment and stop at the first total that does not fit 900000000 bytes.",
            "duplicate_decode":"Measured sum of group-table-set walks plus one shared-owner walk per segment minus measured union walk; shared-table walks decoded independently. Negative differences are retained as timing noise, never clamped.",
            "tool_admission":"The complete-table union pass takes an exclusive 2.2 GB reader reservation. Later group walks reserve measured complete-table allocation peaks plus 320 MB for buffers/dictionary/transients, within the same 2.2 GB allowance. --threads is the worker maximum; actual active readers reported.",
            "damage":"Every damaged primary segment is wholly excluded from totals, spools and admission results. No zero or reconstructed substitute is inserted. The verdict describes only included coverage.",
        }),
        recorded_hours: recording.hours(),
        measured_hours: hours,
        segments,
        excluded_damage,
        damage_count,
        totals,
        groupings,
        group_walks,
        shared_walk,
        ordered_lanes,
        verdict,
    };
    let mut json = BufWriter::new(
        File::options()
            .read(true)
            .write(true)
            .create_new(true)
            .open(args.out.join("report.json"))?,
    );
    serde_json::to_writer_pretty(&mut json, &report)?;
    json.flush()?;
    let markdown_path = args.out.join("report.md");
    report::markdown(&report, &markdown_path)?;
    let (wall_seconds, _) = clock.elapsed()?;
    let process_cpu_seconds = cpu::process_ns()?.saturating_sub(process_start) as f64 / 1e9;
    let (peak_working_set_bytes, peak_commit_bytes) = cpu::peaks()?;
    let tool = Tool {
        wall_seconds,
        process_cpu_seconds,
        peak_commit_bytes,
        peak_working_set_bytes,
        under_three_gb: peak_commit_bytes < 3_000_000_000,
        requested_threads: args.threads,
        maximum_worker_threads: if hours == 0 {
            args.threads.min(recording.segments().len())
        } else {
            args.threads
        },
        maximum_tool_threads_including_main: if hours == 0 {
            args.threads.min(recording.segments().len()) + 1
        } else {
            args.threads + 1
        },
        peak_active_readers: admission.peak.load(Ordering::Relaxed),
        thread_cycle_hz: rate,
        cpu_clock: "QueryThreadCycleTime",
    };
    json.seek(SeekFrom::End(-1))?;
    write!(json, ",\n  \"tool\": ")?;
    serde_json::to_writer(&mut json, &tool)?;
    writeln!(json, "\n}}")?;
    json.flush()?;
    let mut markdown = File::options().append(true).open(markdown_path)?;
    writeln!(
        markdown,
        "\nTool: {:.3} s wall, {:.3} s process CPU, {} bytes peak commit, {} bytes peak working set; {} requested workers, {} peak active readers. Under 3 GB: {}.",
        tool.wall_seconds,
        tool.process_cpu_seconds,
        tool.peak_commit_bytes,
        tool.peak_working_set_bytes,
        tool.requested_threads,
        tool.peak_active_readers,
        tool.under_three_gb
    )?;
    markdown.flush()?;
    println!("{}", report.verdict);
    Ok(())
}

fn main() {
    if let Err(error) = Args::read().and_then(run) {
        eprintln!("core-gate-memory: {error}");
        std::process::exit(1);
    }
}
