use super::*;
use replay_frame::{cpu, frame::TableView};
use serde_json::{Value, json};
use std::{
    fs::File,
    path::{Path, PathBuf},
};

pub struct Options {
    pub recording: PathBuf,
    pub content: ContentOptions,
    pub output: PathBuf,
    pub mode: String,
    pub scopes: bool,
    pub cores: usize,
    pub file_cache: String,
}

pub fn union_tables(source: &Source, content: &ContentOptions) -> io::Result<Vec<TableId>> {
    let replay = source.replay_ref();
    let (store, key) = crate::prepared::files(source, Some(content))?;
    let context = LoadContext {
        content: Some(content.clone()),
    };
    let batches = Batches::new(replay);
    let empty = BTreeMap::new();
    let last = BTreeMap::new();
    let pace = super::super::pace::WalkPace::terminal(replay.index.len());
    let cancelled = AtomicBool::new(false);
    let phase = independent::Phase {
        source,
        replay,
        store: &store,
        key: &key,
        context: &context,
        ranges: &empty,
        last_blocks: &last,
        batches: &batches,
        measured: false,
        cancelled: &cancelled,
        walk_pace: &pace,
    };
    let mut tables = independent::cost_tables(&phase);
    tables.extend_from_slice(selected());
    for family in prepass::families() {
        tables.extend_from_slice(family.tables);
    }
    tables.extend_from_slice(dashboard_base::prepared_map::selected_tables());
    tables.extend(crate::products::production_lines::prepared::Prepared::tables(true));
    tables.extend_from_slice(crate::triggers::RECORDED_TABLES);
    tables.extend_from_slice(&installed_content_country_names::country_names::TABLES);
    tables.extend([TableId::Clock, TableId::Campaign]);
    tables.sort_unstable_by_key(|table| table.index());
    tables.dedup();
    Ok(tables)
}

fn process_cpu() -> io::Result<u64> {
    cpu::process_cpu_ns().ok_or_else(|| io::Error::other("Process CPU accounting is unavailable"))
}

fn hash_file(path: &Path) -> io::Result<String> {
    let mut input = io::BufReader::new(File::open(path)?);
    let before = input.get_ref().metadata()?;
    let mut hash = blake3::Hasher::new();
    let mut bytes = [0u8; 65536];
    loop {
        let count = input.read(&mut bytes)?;
        if count == 0 {
            break;
        }
        hash.update(&bytes[..count]);
    }
    let after = input.get_ref().metadata()?;
    if before.len() != after.len() || before.modified()? != after.modified()? {
        return Err(invalid("An input changed while its audit hash was read"));
    }
    Ok(hash.finalize().to_hex().to_string())
}

fn delta(before: &Value, after: Value) -> Value {
    type Key = (Option<u64>, String, String, String);
    let key = |row: &Value| -> Key {
        (
            row["block"].as_u64(),
            row["category"].as_str().unwrap().into(),
            row["family"].as_str().unwrap().into(),
            row["phase"].as_str().unwrap().into(),
        )
    };
    let old: BTreeMap<_, _> = before
        .as_array()
        .into_iter()
        .flatten()
        .map(|row| (key(row), row))
        .collect();
    json!(
        after
            .as_array()
            .into_iter()
            .flatten()
            .map(|row| {
                let prior = old.get(&key(row));
                let mut row = row.clone();
                for field in ["ns", "calls"] {
                    row[field] = json!(
                        row[field]
                            .as_u64()
                            .unwrap()
                            .saturating_sub(prior.and_then(|v| v[field].as_u64()).unwrap_or(0))
                    );
                }
                row
            })
            .filter(|row| row["ns"] != 0 || row["calls"] != 0)
            .collect::<Vec<_>>()
    )
}

fn summarize(rows: &Value, hours: &[u64]) -> Value {
    let mut groups: BTreeMap<(String, String), (u64, u64, BTreeMap<usize, u64>)> = BTreeMap::new();
    for row in rows.as_array().into_iter().flatten() {
        let key = (
            row["family"].as_str().unwrap().to_owned(),
            row["phase"].as_str().unwrap().to_owned(),
        );
        let group = groups.entry(key).or_default();
        let ns = row["ns"].as_u64().unwrap();
        group.0 += ns;
        group.1 += row["calls"].as_u64().unwrap();
        if let Some(block) = row["block"].as_u64()
            && (block as usize) < hours.len()
        {
            *group.2.entry(block as usize).or_default() += ns;
        }
    }
    let total_hours: u64 = hours.iter().sum();
    json!(groups.into_iter().map(|((family, phase), (ns, calls, blocks))| {
        let mut costs: Vec<_> = blocks.iter().filter_map(|(block, ns)| hours.get(*block).map(|hours| *ns as f64 / 1000.0 / *hours as f64)).collect();
        costs.sort_by(f64::total_cmp);
        let mut block_us: Vec<_> = blocks.values().map(|ns| *ns as f64 / 1000.0).collect();
        block_us.sort_by(f64::total_cmp);
        let block_percentile = |percent: usize| block_us.get((block_us.len() * percent).div_ceil(100).saturating_sub(1)).copied();
        let percentile = |percent: usize| costs.get((costs.len() * percent).div_ceil(100).saturating_sub(1)).copied();
        json!({"family":family, "phase":phase, "cpu_ns":ns, "calls":calls,
            "cpu_us_per_recorded_hour": ns as f64 / 1000.0 / total_hours as f64,
            "block_cpu_us_per_recorded_hour":{"p50":percentile(50),"p95":percentile(95),"max":costs.last(),"measured_blocks":costs.len()},
            "block_cpu_us":{"p50":block_percentile(50),"p95":block_percentile(95),"max":block_us.last()},
            "unassigned_cpu_ns": ns.saturating_sub(blocks.values().sum()), "block_cpu_ns":blocks})
    }).collect::<Vec<_>>())
}

fn consume_decoded(frame: &DecodedFrame, tables: &[TableId], checksum: &mut u64) {
    let frame = std::hint::black_box(frame);
    let mut mix = |value: u64| {
        *checksum = checksum
            .rotate_left(7)
            .wrapping_add(value)
            .wrapping_mul(0x100000001b3);
    };
    mix(frame.branch);
    mix(frame.hour as u64);
    for table in tables {
        let view = &frame.tables[table.index()];
        mix(table.index() as u64);
        mix(view.sampled_at().map_or(u64::MAX, |hour| hour as u64));
        let rows = match &**view {
            TableView::Complete(rows) => {
                mix(1);
                rows
            }
            TableView::Truncated { rows, missing } => {
                mix(2);
                mix(u64::from(*missing));
                rows
            }
            TableView::Unavailable { reason } => {
                mix(3);
                for byte in reason.bytes() {
                    mix(u64::from(byte));
                }
                continue;
            }
            TableView::NotRecorded { .. } => {
                mix(4);
                continue;
            }
        };
        mix(rows.len() as u64);
        for row in [
            rows.first(),
            rows.len().checked_sub(1).and_then(|last| rows.get(last)),
        ]
        .into_iter()
        .flatten()
        {
            let key = row.key();
            mix(key as u64);
            mix((key >> 64) as u64);
        }
    }
    std::hint::black_box(*checksum);
}

fn decode(
    source: &Source,
    tables: Vec<TableId>,
    sequential: bool,
) -> io::Result<(Vec<u64>, Vec<Value>, Vec<TableId>, u64)> {
    let replay = Replay::open_path(source.path())?;
    replay.cache_decoded_sections(false);
    let mut stream = ReplayStream::new(&tables)?;
    let (mut hours, mut blocks) = (Vec::new(), Vec::new());
    let mut checksum = 0xcbf29ce484222325u64;
    for (ordinal, index) in replay.index.iter().enumerate() {
        let _block = cpu::block(Some(ordinal));
        let _cpu = cpu::scope("decode floor", Some("union"));
        let wall = Instant::now();
        let before = process_cpu()?;
        let mut count = 0u64;
        let mut visit = |frame: &DecodedFrame| -> io::Result<()> {
            consume_decoded(frame, &tables, &mut checksum);
            count += 1;
            Ok(())
        };
        if sequential {
            if !stream.next_block_recorded_with(&replay, |_, frame| visit(frame), || Ok(()))? {
                return Err(invalid("Reader stopped before the indexed EOF"));
            }
        } else {
            replay.read_block_recorded_alone(index, &tables, |_, frame| visit(frame), || Ok(()))?;
        }
        let ns = process_cpu()?.saturating_sub(before);
        if count == 0 {
            return Err(invalid("A block yielded no accepted recorded hours"));
        }
        hours.push(count);
        blocks.push(json!({"ordinal":ordinal,"branch":index.branch,"first_hour":index.first_hour,"last_hour":index.last_hour,
            "recorded_hours":count,"wall_seconds":wall.elapsed().as_secs_f64(),"process_cpu_ns":ns,
            "stored_bytes":index.length,"raw_bytes":index.raw_length,"decoded_output_checksum":format!("{checksum:016x}")}));
    }
    Ok((hours, blocks, tables, checksum))
}

pub fn run(options: &Options) -> io::Result<Value> {
    if !options.scopes && cpu::enabled() {
        return Err(invalid(
            "Prerequisite failed: scopes-off requires a fresh process with CPU accounting disabled",
        ));
    }
    if cfg!(test) || cfg!(debug_assertions) || !store::durable_writes_enabled() {
        return Err(invalid(
            "Prerequisite failed: use an ordinary release preparation-cost binary without test-support features; copy it outside the cargo target before compiling the ignored test harness",
        ));
    }
    if crate::prepared::recording_store(&options.recording)?
        .saved_key()?
        .is_some()
    {
        return Err(invalid(
            "Prerequisite failed: benchmark needs an empty isolated prepared store",
        ));
    }
    if !["decode", "decode-sequential", "families"].contains(&options.mode.as_str()) {
        return Err(invalid(
            "Mode must be decode, decode-sequential, or families",
        ));
    }
    let _affinity = Affinity::pin(options.cores)?;
    if options.mode != "families" && options.cores != 1 {
        return Err(invalid("The decode floor requires exactly one logical CPU"));
    }
    if options.scopes {
        cpu::enable_blocks();
    }
    let _scope = cpu::scope("harness", Some("shared"));
    let mut before = if options.scopes {
        cpu::block_snapshot()
    } else {
        json!([])
    };
    let mut cpu_start = process_cpu()?;
    let mut wall = Instant::now();
    let source = {
        let _cpu = cpu::scope("source open", Some("shared"));
        Source::open(
            crate::SourceKey {
                source_id: "preparation-cost".into(),
                revision: crate::protocol::replay_revision(&options.recording)?,
            },
            options.recording.clone(),
            &AtomicBool::new(false),
        )?
    };
    if source.replay_ref().index.is_empty() {
        return Err(invalid(
            "Prerequisite failed: recording has no durable blocks",
        ));
    }
    engine_executable::executable_tables::initialize(&options.content.options.game_dir)?;
    let union = if options.mode != "families" {
        union_tables(&source, &options.content)?
    } else {
        Vec::new()
    };
    let setup_seconds = wall.elapsed().as_secs_f64();
    if options.mode != "families" {
        before = if options.scopes {
            cpu::block_snapshot()
        } else {
            json!([])
        };
        cpu_start = process_cpu()?;
        wall = Instant::now();
    }
    let (hours, blocks, tables, completion, checksum) = if options.mode == "families" {
        let accepted = super::run(
            &source,
            Some(&options.content),
            &AtomicBool::new(false),
            |_| {},
        )?;
        let total = source.replay_ref().index.len();
        let (store, key) = crate::prepared::files(&source, Some(&options.content))?;
        let (checkpoint, _) = checkpoint(&store, &key, None)?
            .ok_or_else(|| invalid("Preparation published no checkpoint"))?;
        if !checkpoint.forward_complete(total)
            || !side::complete(&source, Some(&options.content), &store, &key)?
        {
            return Err(invalid(
                "Preparation returned before every forward family and side job completed",
            ));
        }
        let completion = json!({"walk_next_block":accepted.next_block,"total_blocks":total,"sections":accepted.sections,"families":accepted.families});
        // Count actual input frames outside the family interval, without inferred gap lengths.
        (Vec::new(), Vec::new(), Vec::new(), completion, None)
    } else {
        let (hours, blocks, tables, checksum) =
            decode(&source, union, options.mode == "decode-sequential")?;
        (
            hours,
            blocks,
            tables,
            Value::Null,
            Some(format!("{checksum:016x}")),
        )
    };
    let process_ns = process_cpu()?.saturating_sub(cpu_start);
    let seconds = wall.elapsed().as_secs_f64();
    let memory = process_memory()?;
    let raw = if options.scopes {
        delta(&before, cpu::block_snapshot())
    } else {
        json!([])
    };
    let accounting = options.scopes.then(cpu::snapshot);
    let (hours, blocks) = if options.mode == "families" {
        let mut hours = Vec::new();
        let mut blocks = Vec::new();
        for (ordinal, index) in source.replay_ref().index.iter().enumerate() {
            let mut count = 0u64;
            source.replay_ref().read_block_recorded_alone(
                index,
                &[TableId::Clock, TableId::Campaign],
                |_, _| {
                    count += 1;
                    Ok(())
                },
                || Ok(()),
            )?;
            if count == 0 {
                return Err(invalid("Block has no accepted recorded hours"));
            }
            hours.push(count);
            blocks.push(json!({"ordinal":ordinal,"branch":index.branch,"recorded_hours":count,"first_hour":index.first_hour,"last_hour":index.last_hour}));
        }
        (hours, blocks)
    } else {
        (hours, blocks)
    };
    let total_hours: u64 = hours.iter().sum();
    let scoped: u64 = raw
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["ns"].as_u64().unwrap())
        .sum();
    let residual = process_ns as i128 - scoped as i128;
    let tolerance = 0.05;
    let reconciles = !options.scopes
        || residual.unsigned_abs() as f64 <= (process_ns as f64 * tolerance).max(20_000_000.0);
    let (_, key) = crate::prepared::files(&source, Some(&options.content))?;
    let source_value = serde_json::to_value(&key)?;
    let mut fingerprints = Vec::new();
    for stamp in source_value["content"]
        .as_array()
        .ok_or_else(|| invalid("Missing content binding"))?
    {
        let path = PathBuf::from(
            stamp["path"]
                .as_str()
                .ok_or_else(|| invalid("Invalid content path"))?,
        );
        fingerprints.push(json!({"path":path,"blake3":hash_file(&path)?}));
    }
    let recording_hash = hash_file(&options.recording)?;
    let executable_hash = hash_file(&std::env::current_exe()?)?;
    let game_hash = hash_file(&options.content.options.game_dir.join("hoi4.exe"))?;
    if key != key.recapture()? {
        return Err(invalid("Inputs changed during the benchmark"));
    }
    let report = json!({"schema":1,"mode":options.mode,"scopes_enabled":options.scopes,
        "allocator":"mimalloc", "profile": if cfg!(debug_assertions) {"debug"} else {"release"},
        "logical_cpus": options.cores,"affinity_mask":format!("{:x}",_affinity.1), "driver":"ordinary production preparation; existing private readers and player/requested production scans",
        "file_cache_state":options.file_cache, "cold_prepared_store":true,
        "file_cache_scope":"Operator-labelled state at process start; source/content binding can warm input pages before the decode interval; no cache eviction",
        "executable_blake3":executable_hash,"game_directory":options.content.options.game_dir,
        "game_executable_blake3":game_hash,
        "source":source_value,"recording_blake3":recording_hash,"content_fingerprints":fingerprints,
        "format":source.replay_ref().format.version(),"recorded_hours":total_hours,
        "wall_seconds":seconds,"source_setup_seconds":setup_seconds,"process_cpu_ns":process_ns,
        "process_cpu_us_per_recorded_hour":process_ns as f64 / 1000.0 / total_hours as f64,
        "exclusive_cpu_ns":options.scopes.then_some(scoped),"cpu_reconciliation":{"passed":options.scopes.then_some(reconciles),"process_minus_scopes_ns":options.scopes.then_some(residual),"relative_tolerance":tolerance,"absolute_tolerance_ns":20_000_000},
        "decoded_output_checksum":checksum,
        "checksum_scope":"Rolling selected-table state, sample hour, row count and boundary keys; not a payload parity hash. The full borrowed reconstructed frame crosses black_box each hour without cloning or retention.",
        "memory":memory,"legacy_cpu_accounting":accounting,
        "source_commit":std::process::Command::new("git").args(["rev-parse","HEAD"]).output().ok().filter(|out|out.status.success()).map(|out|String::from_utf8_lossy(&out.stdout).trim().to_owned()),
        "families_and_phases":summarize(&raw, &hours),"exclusive_scopes":raw,"blocks":blocks,"completion":completion,
        "selected_tables":tables.iter().map(|table|format!("{table:?}")).collect::<Vec<_>>(),
        "limitations":["Current-driver baseline, not the future W1B one-pass runtime or all-country production", "Group adoption/checkpoint/manifest CPU is shared commit, with no block attribution; family staging retains its family and block", "Decode-sequential reuses today's ReplayStream decoder; it is not a new anchor cache", "Source/content fingerprinting and family-mode hour counting are outside the timed interval"]});
    serde_json::to_writer_pretty(File::create(options.output.join("report.json"))?, &report)?;
    std::fs::write(
        options.output.join("summary.txt"),
        format!(
            "{}: {} recorded hours, {:.6} s wall, {:.3} CPU-us/hour; decoded checksum {}; exclusive CPU reconciliation {}\n",
            options.mode,
            total_hours,
            seconds,
            process_ns as f64 / 1000.0 / total_hours as f64,
            checksum.as_deref().unwrap_or("NOT MEASURED"),
            if !options.scopes {
                "NOT MEASURED"
            } else if reconciles {
                "PASS"
            } else {
                "FAIL"
            }
        ),
    )?;
    if !reconciles {
        return Err(invalid(
            "Exclusive scopes do not reconcile with process CPU; see report.json",
        ));
    }
    Ok(report)
}

fn process_memory() -> io::Result<Value> {
    #[repr(C)]
    #[derive(Default)]
    struct Counters {
        size: u32,
        faults: u32,
        peak_working: usize,
        working: usize,
        peak_paged: usize,
        paged: usize,
        peak_nonpaged: usize,
        nonpaged: usize,
        pagefile: usize,
        peak_pagefile: usize,
        private: usize,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn K32GetProcessMemoryInfo(
            process: windows_sys::Win32::Foundation::HANDLE,
            counters: *mut Counters,
            size: u32,
        ) -> i32;
    }
    let mut counters = Counters {
        size: std::mem::size_of::<Counters>() as u32,
        ..Default::default()
    };
    if unsafe {
        K32GetProcessMemoryInfo(
            windows_sys::Win32::System::Threading::GetCurrentProcess(),
            &mut counters,
            std::mem::size_of::<Counters>() as u32,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(
        json!({"private_bytes_at_completion":counters.private,"peak_process_commit_bytes":counters.peak_pagefile,"working_set_bytes_at_completion":counters.working,"peak_working_set_bytes":counters.peak_working,"scope":"OS process lifetime counters, including source/content binding; not per-walker heap attribution","walker_heap_bytes":null,"allocations":null}),
    )
}

struct Affinity(usize, usize);

impl Affinity {
    fn pin(cores: usize) -> io::Result<Self> {
        use windows_sys::Win32::System::Threading::{
            GetCurrentProcess, GetProcessAffinityMask, SetProcessAffinityMask,
        };
        let (mut process, mut system) = (0usize, 0usize);
        if unsafe { GetProcessAffinityMask(GetCurrentProcess(), &mut process, &mut system) } == 0 {
            return Err(io::Error::last_os_error());
        }
        if cores == 0 || cores > process.count_ones() as usize {
            return Err(invalid(
                "Requested logical CPUs exceed this process's affinity",
            ));
        }
        let mut chosen = 0usize;
        for bit in 0..usize::BITS {
            if process & (1usize << bit) != 0 && chosen.count_ones() < cores as u32 {
                chosen |= 1usize << bit;
            }
        }
        if unsafe { SetProcessAffinityMask(GetCurrentProcess(), chosen) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(process, chosen))
    }
}

impl Drop for Affinity {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::System::Threading::SetProcessAffinityMask(
                windows_sys::Win32::System::Threading::GetCurrentProcess(),
                self.0,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_frame::{
        frame::Metrics,
        rows::{CountryRow, TableRows},
    };

    #[test]
    fn shared_commit_summary_is_unassigned_and_excluded_from_family_block_costs() {
        for (leader, block) in [("family-a", 0), ("family-b", 1)] {
            let rows = json!([
                {"block":block,"category":"fold save","family":leader,"phase":"publication","ns":11,"calls":1},
                {"block":null,"category":"shared commit","family":"shared commit","phase":"shared commit","ns":29,"calls":1},
                {"block":null,"category":"commit","family":"shared commit","phase":"shared commit","ns":31,"calls":1},
            ]);
            let summary = summarize(&rows, &[2, 3]);
            let summary = summary.as_array().unwrap();
            let shared = summary
                .iter()
                .find(|row| row["family"] == "shared commit")
                .unwrap();
            assert_eq!(shared["phase"], "shared commit");
            assert_eq!(shared["cpu_ns"], 60);
            assert_eq!(shared["unassigned_cpu_ns"], 60);
            assert_eq!(shared["block_cpu_ns"], json!({}));
            assert_eq!(
                shared["block_cpu_us_per_recorded_hour"]["measured_blocks"],
                0
            );
            assert!(shared["block_cpu_us_per_recorded_hour"]["p95"].is_null());
            let family = summary.iter().find(|row| row["family"] == leader).unwrap();
            assert_eq!(family["cpu_ns"], 11);
            assert_eq!(family["unassigned_cpu_ns"], 0);
            assert_eq!(
                summary
                    .iter()
                    .map(|row| row["cpu_ns"].as_u64().unwrap())
                    .sum::<u64>(),
                71
            );
        }
    }

    #[test]
    fn decoded_checksum_consumes_selected_output_without_retaining_views() {
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 2,
            tables: std::array::from_fn(|_| {
                TableView::Unavailable {
                    reason: "synthetic".into(),
                }
                .into()
            }),
            metrics: Metrics::default(),
        };
        let checksum = |frame: &DecodedFrame| {
            let mut value = 0xcbf29ce484222325;
            consume_decoded(frame, &[TableId::Countries], &mut value);
            value
        };
        let unavailable = checksum(&frame);
        frame.tables[TableId::Countries.index()] =
            TableView::Complete(TableRows::Countries(vec![])).into();
        let empty = checksum(&frame);
        assert_ne!(empty, unavailable);
        frame.tables[TableId::Countries.index()] =
            TableView::Complete(TableRows::Countries(vec![CountryRow::default()])).into();
        let populated = checksum(&frame);
        assert_ne!(populated, empty);
        assert_eq!(checksum(&frame), populated);
        let view = std::mem::replace(
            &mut frame.tables[TableId::Countries.index()],
            TableView::Unavailable {
                reason: "released".into(),
            }
            .into(),
        );
        assert!(view.try_unwrap().is_ok());
    }
}
