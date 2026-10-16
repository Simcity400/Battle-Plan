//! Measurement-only, durable block evidence. No prepared store writes.
use super::*;
use dashboard::prepared::pipeline::{FamilyTimings, MeasuredBlock};
use std::io::{BufRead, BufReader, Read};

pub(super) fn family_rows(families: &BTreeMap<&str, FamilyTimings>, wall: Duration) -> Vec<Value> {
    let mut rows: Vec<_> = families.iter().collect();
    rows.sort_by(|(a, left), (b, right)| {
        (right.observe + right.flush)
            .cmp(&(left.observe + left.flush))
            .then_with(|| a.cmp(b))
    });
    rows.into_iter().map(|(name, timing)| {
        let seconds = (timing.observe + timing.flush).as_secs_f64();
        json!({"family": name, "observeSeconds": timing.observe.as_secs_f64(),
            "flushSeconds": timing.flush.as_secs_f64(), "calls": timing.calls,
            "meanSecondsPerCall": if timing.calls == 0 { 0.0 } else { seconds / timing.calls as f64 },
            "percentage": seconds / wall.as_secs_f64() * 100.0})
    }).collect()
}

pub(super) struct BlockEvidence {
    pub blocks: Vec<Value>,
    pub run_id: u64,
    pub preflight: Duration,
    path: PathBuf,
    journal: File,
    targets: Vec<u64>,
    resumed: bool,
    serial: bool,
    next_ordinal: usize,
    peak_private: u64,
}

impl BlockEvidence {
    pub fn open(
        path: &Path,
        source: &Source,
        content: Option<&ContentOptions>,
        serial: bool,
        resumed: bool,
        targets: &[u64],
    ) -> io::Result<Self> {
        let (_, key) = prepared::files(source, content)?;
        let input = json!({"sourceKey": key, "serial": serial,
            "observationMode": if serial { "serial" } else { "family-lockstep" }});
        let input_path = path.join("measurement-input.json");
        if resumed {
            let prior: Value = serde_json::from_reader(File::open(&input_path)?)?;
            if input != prior {
                return Err(invalid(
                    "Resume recording/content/mode differs from the measured prefix",
                ));
            }
        } else {
            if input_path.exists() || path.join("blocks.jsonl").exists() {
                return Err(invalid("Nonempty measurement evidence requires --resume"));
            }
            write_json(&input_path, &input)?;
        }
        let mut blocks = Vec::<Value>::new();
        let journal_path = path.join("blocks.jsonl");
        let mut journal_options = fs::OpenOptions::new();
        journal_options
            .create(true)
            .truncate(false)
            .read(true)
            .write(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            const READ_SHARING: u32 = 1;
            // The launcher can read commits; a second writer cannot truncate or
            // append to the same evidence while this measurement is active.
            journal_options.share_mode(READ_SHARING);
        }
        let mut journal = journal_options.open(&journal_path)?;
        let mut complete_bytes = 0;
        let mut reader = BufReader::new(&journal);
        loop {
            let mut line = String::new();
            let count = reader.read_line(&mut line)?;
            if count == 0 {
                break;
            }
            if !line.ends_with('\n') {
                break;
            } // Only a torn final append is discarded.
            let row: Value = serde_json::from_str(&line)?;
            if row["recordKind"] == "storage" {
                let ordinal = row["ordinal"]
                    .as_u64()
                    .ok_or_else(|| invalid("Missing storage ordinal"))?
                    as usize;
                let prior = blocks
                    .get_mut(ordinal)
                    .ok_or_else(|| invalid("Storage evidence precedes its commit"))?;
                *prior = row;
                complete_bytes += count as u64;
                continue;
            }
            if row["ordinal"].as_u64() != Some(blocks.len() as u64) {
                return Err(invalid(
                    "Block evidence is not a contiguous committed prefix",
                ));
            }
            blocks.push(row);
            complete_bytes += count as u64;
        }
        drop(reader);
        journal.set_len(complete_bytes)?;
        std::io::Seek::seek(&mut journal, std::io::SeekFrom::End(0))?;
        let next_ordinal = source.replay_ref().index.len()
            - prepared::unprepared_blocks(source, content)?
                .unwrap_or(source.replay_ref().index.len());
        if next_ordinal != blocks.len() {
            return Err(invalid(format!(
                "Resume evidence/cache cursor mismatch: {} journal blocks, {next_ordinal} accepted blocks; committed timing evidence is missing",
                blocks.len()
            )));
        }
        let run_id = blocks
            .last()
            .and_then(|row| row["runId"].as_u64())
            .map_or(0, |id| id + 1);
        let peak_private = blocks
            .iter()
            .filter_map(|row| row["peakPrivateBytes"].as_u64())
            .max()
            .unwrap_or(0);
        let mut targets = targets.to_vec();
        targets.sort_unstable();
        targets.dedup();
        let mut evidence = Self {
            blocks,
            run_id,
            preflight: Duration::ZERO,
            path: path.to_owned(),
            journal,
            targets,
            resumed,
            serial,
            next_ordinal,
            peak_private,
        };
        // A stop during read-only disk accounting must not lose the completed
        // block's timings. Only the last commit can await its storage enrichment.
        if evidence
            .blocks
            .last()
            .is_some_and(|row| row["recordKind"] != "storage")
        {
            evidence.complete_storage()?;
        }
        // Also support requesting a snapshot of an already measured prefix on resume.
        evidence.snapshots()?;
        Ok(evidence)
    }

    pub fn commit(&mut self, block: &MeasuredBlock) -> io::Result<()> {
        if block.ordinal != self.next_ordinal {
            return Err(invalid(
                "Measured commit did not extend the evidence prefix",
            ));
        }
        let private = private_bytes()?;
        let sampled_peak = fs::read(self.path.join("memory-current.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .and_then(|value| value["peakPrivateBytes"].as_u64())
            .unwrap_or(private);
        self.peak_private = self.peak_private.max(private).max(sampled_peak);
        let preflight = std::mem::take(&mut self.preflight);
        let wall = block.wall + preflight;
        let mut families = block.families.clone();
        if !preflight.is_zero() {
            families.insert(
                "measurement.preflight",
                FamilyTimings {
                    observe: preflight,
                    flush: Duration::ZERO,
                    calls: 1,
                },
            );
        }
        let row = json!({"recordKind": "commit", "ordinal": block.ordinal, "firstHour": block.first_hour, "lastHour": block.last_hour,
            "gameHours": block.hours, "cumulativeGameHours": block.cumulative_hours,
            "wallSeconds": wall.as_secs_f64(), "gameHoursPerSecond": block.hours as f64 / wall.as_secs_f64(),
            "decodeSeconds": if self.serial { Some(block.decode.as_secs_f64()) } else { None }, "familyTimings": family_rows(&families, wall),
            "observationCriticalPathSeconds": block.observe.as_secs_f64(),
            "workerObserveSeconds": block.worker_observe.as_secs_f64(),
            "residualSeconds": block.residual.as_secs_f64(), "storeBytes": block.store_bytes,
            "privateBytesAtCommit": private, "peakPrivateBytes": self.peak_private,
            "runId": self.run_id, "resumed": self.resumed});
        serde_json::to_writer(&mut self.journal, &row)?;
        self.journal.write_all(b"\n")?;
        self.journal.flush()?;
        self.journal.sync_data()?;
        self.blocks.push(row);
        self.next_ordinal += 1;
        println!(
            "COMMIT ordinal={} hours={} wall_seconds={:.6}",
            block.ordinal,
            block.cumulative_hours,
            wall.as_secs_f64()
        );
        self.complete_storage()?;
        self.snapshots()
    }

    fn complete_storage(&mut self) -> io::Result<()> {
        let (disk_bytes, sections) = store_sizes(&self.path)?;
        let row = self
            .blocks
            .last_mut()
            .expect("committed row before storage");
        row["recordKind"] = json!("storage");
        row["onDiskStoreBytes"] = json!(disk_bytes);
        row["storeBytesBySection"] = json!(sections);
        let ordinal = row["ordinal"].as_u64().expect("committed ordinal");
        let payload_file = write_payload_changes(&self.path, ordinal)?;
        row["payloadChangesFile"] = json!(payload_file);
        serde_json::to_writer(&mut self.journal, &*row)?;
        self.journal.write_all(b"\n")?;
        self.journal.flush()?;
        self.journal.sync_data()
    }

    fn snapshots(&self) -> io::Result<()> {
        for &target in &self.targets {
            let Some(end) = self
                .blocks
                .iter()
                .position(|row| row["cumulativeGameHours"].as_u64().unwrap_or(0) >= target)
            else {
                continue;
            };
            let output = self.path.join(format!("prefix-{target}.json"));
            if output.exists() {
                continue;
            }
            let rows = &self.blocks[..=end];
            let first = &rows[..rows.len().min(10)];
            let last = &rows[rows.len().saturating_sub(10)..];
            let first_rates = family_rates(first);
            let last_rates = family_rates(last);
            let names: BTreeSet<_> = first_rates.keys().chain(last_rates.keys()).collect();
            let growth: Vec<_> = names.into_iter().map(|name| {
                let first = first_rates.get(name).copied().unwrap_or(0.0);
                let last = last_rates.get(name).copied().unwrap_or(0.0);
                json!({"family": name, "first10SecondsPer1000Hours": first, "last10SecondsPer1000Hours": last,
                    "growthRatio": if first > 0.0 { Some(last / first) } else { None }})
            }).collect();
            let hours: f64 = rows
                .iter()
                .map(|row| row["gameHours"].as_f64().unwrap_or(0.0))
                .sum();
            write_json(
                &output,
                &json!({"throughHours": target, "committedThroughHours": rows[end]["cumulativeGameHours"],
                "blocks": rows.len(), "cumulativeGameHoursPerSecond": throughput(rows),
                "observationCriticalPathSeconds": rows.iter().map(|row| row["observationCriticalPathSeconds"].as_f64().unwrap_or(0.0)).sum::<f64>(),
                "workerObserveSeconds": rows.iter().map(|row| row["workerObserveSeconds"].as_f64().unwrap_or(0.0)).sum::<f64>(),
                "last10GameHoursPerSecond": throughput(last), "firstWindowBlocks": first.len(), "lastWindowBlocks": last.len(),
                "familyGrowth": growth, "peakPrivateBytes": rows.iter().filter_map(|row| row["peakPrivateBytes"].as_u64()).max(),
                "storeBytesPer1000Hours": rows[end]["storeBytes"].as_f64().unwrap_or(0.0) * 1000.0 / hours,
                "onDiskStoreBytesPer1000Hours": rows[end]["onDiskStoreBytes"].as_f64().unwrap_or(0.0) * 1000.0 / hours,
                "scope": "Committed prefix across all runs; first and last windows contain up to 10 blocks. Block wall excludes evidence writing and the final post-commit cleanup. Through-hours counts indexed block-hour spans; throughput counts observed frames. Store bytes are accepted live payload; per-section bytes and on-disk total include retained files and metadata at this commit."}),
            )?;
        }
        Ok(())
    }
}

// Compare immutable file identities with the prior committed manifest and hash
// only new/replaced payloads. Histories are streamed, not loaded or rehashed on
// every block; one small digest row per changed range goes straight to disk.
fn write_payload_changes(evidence: &Path, ordinal: u64) -> io::Result<String> {
    use std::io::{Seek, SeekFrom};
    let output_name = format!("payload-{ordinal}.jsonl");
    let output_path = evidence.join(&output_name);
    if output_path.is_file() {
        // The digest file is atomically published before advancing the mirror.
        // Repairing an interrupted enrichment must retain that completed delta.
        advance_payload_manifests(evidence)?;
        return Ok(output_name);
    }
    let temporary = output_path.with_extension("jsonl.tmp");
    let mut output = BufWriter::new(File::create(&temporary)?);
    let root = evidence.join("cache/local/HOI4Replay/prepared");
    for entry in fs::read_dir(root)? {
        let directory = entry?.path();
        if !directory.is_dir() {
            continue;
        }
        let manifest_path = directory.join("manifest");
        let mut manifest = BufReader::new(File::open(&manifest_path)?);
        let mut length = [0; 8];
        manifest.read_exact(&mut length)?;
        let length = u64::from_le_bytes(length);
        if length > manifest.get_ref().metadata()?.len().saturating_sub(8) {
            return Err(invalid("Truncated payload manifest header"));
        }
        let header: Value = serde_json::from_reader((&mut manifest).take(length))?;
        let snapshot = evidence.join(format!(
            "payload-manifest-{}",
            directory.file_name().unwrap().to_string_lossy()
        ));
        let mut prior = match File::open(&snapshot) {
            Ok(file) => Some(BufReader::new(file)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(error),
        };
        let mut prior_sections = BTreeMap::new();
        if let Some(prior) = &mut prior {
            let mut length = [0; 8];
            prior.read_exact(&mut length)?;
            let length = u64::from_le_bytes(length);
            let header: Value = serde_json::from_reader((&mut *prior).take(length))?;
            let mut offset = 8 + length;
            for section in header["sections"]
                .as_array()
                .ok_or_else(|| invalid("Missing prior section names"))?
            {
                let name = section["name"]
                    .as_str()
                    .ok_or_else(|| invalid("Missing prior section name"))?;
                let count = section["count"]
                    .as_u64()
                    .ok_or_else(|| invalid("Missing prior section count"))?;
                prior_sections.insert(name.to_owned(), (offset, count));
                offset += count * 16;
            }
        }
        for (section_index, section) in header["sections"]
            .as_array()
            .ok_or_else(|| invalid("Missing payload sections"))?
            .iter()
            .enumerate()
        {
            let name = section["name"]
                .as_str()
                .ok_or_else(|| invalid("Missing payload section name"))?;
            let count = section["count"]
                .as_u64()
                .ok_or_else(|| invalid("Missing payload section count"))?;
            let prior_count = if let (Some(prior), Some(&(offset, prior_count))) =
                (&mut prior, prior_sections.get(name))
            {
                prior.seek(SeekFrom::Start(offset))?;
                prior_count
            } else {
                0
            };
            for range in 0..count {
                let mut id = [0; 16];
                manifest.read_exact(&mut id)?;
                let same = if let Some(prior) = &mut prior {
                    if range < prior_count {
                        let mut previous = [0; 16];
                        prior.read_exact(&mut previous)?;
                        previous == id
                    } else {
                        false
                    }
                } else {
                    false
                };
                if same {
                    continue;
                }
                let payload_path = directory.join(format!(
                    "{section_index}_{range}_{}.bin",
                    uuid::Uuid::from_bytes(id).simple()
                ));
                let mut payload = File::open(&payload_path)?;
                let bytes = payload.metadata()?.len();
                let mut digest = blake3::Hasher::new();
                digest.update_reader(&mut payload)?;
                serde_json::to_writer(
                    &mut output,
                    &json!({"section": name, "range": range, "bytes": bytes, "blake3": digest.finalize().to_hex().to_string()}),
                )?;
                output.write_all(b"\n")?;
            }
        }
    }
    output.flush()?;
    output.get_ref().sync_all()?;
    drop(output);
    replay::publication::replace_file(&temporary, &output_path)?;
    // Storage enrichment is journaled by the caller. Repeating a snapshot after
    // interruption may rehash files, but never changes the prepared data.
    advance_payload_manifests(evidence)?;
    Ok(output_name)
}

fn advance_payload_manifests(evidence: &Path) -> io::Result<()> {
    for entry in fs::read_dir(evidence.join("cache/local/HOI4Replay/prepared"))? {
        let directory = entry?.path();
        if !directory.is_dir() {
            continue;
        }
        let manifest = directory.join("manifest");
        let snapshot = evidence.join(format!(
            "payload-manifest-{}",
            directory.file_name().unwrap().to_string_lossy()
        ));
        let temporary = snapshot.with_extension("tmp");
        fs::copy(manifest, &temporary)?;
        fs::OpenOptions::new()
            .write(true)
            .open(&temporary)?
            .sync_all()?;
        replay::publication::replace_file(&temporary, &snapshot)?;
    }
    Ok(())
}

fn throughput(rows: &[Value]) -> f64 {
    let hours: f64 = rows
        .iter()
        .map(|row| row["gameHours"].as_f64().unwrap_or(0.0))
        .sum();
    let seconds: f64 = rows
        .iter()
        .map(|row| row["wallSeconds"].as_f64().unwrap_or(0.0))
        .sum();
    hours / seconds
}

fn family_rates(rows: &[Value]) -> BTreeMap<String, f64> {
    let mut rates = BTreeMap::new();
    let hours: f64 = rows
        .iter()
        .map(|row| row["gameHours"].as_f64().unwrap_or(0.0))
        .sum();
    for row in rows {
        for family in row["familyTimings"].as_array().into_iter().flatten() {
            if let Some(name) = family["family"].as_str() {
                *rates.entry(name.to_owned()).or_insert(0.0) +=
                    (family["observeSeconds"].as_f64().unwrap_or(0.0)
                        + family["flushSeconds"].as_f64().unwrap_or(0.0))
                        * 1000.0
                        / hours;
            }
        }
    }
    rates
}

fn write_json(path: &Path, value: &Value) -> io::Result<()> {
    let temporary = path.with_extension("json.tmp");
    let mut file = File::create(&temporary)?;
    serde_json::to_writer_pretty(&mut file, value)?;
    file.sync_all()?;
    drop(file);
    replay::publication::replace_file(&temporary, path)
}

// Read-only disk accounting of the isolated prepared directory. The manifest's
// public documented header associates numeric file prefixes with section names.
fn store_sizes(evidence: &Path) -> io::Result<(u64, BTreeMap<String, u64>)> {
    let root = evidence.join("cache/local/HOI4Replay/prepared");
    let mut total = 0;
    let mut sections = BTreeMap::new();
    for entry in fs::read_dir(root)? {
        let directory = entry?.path();
        if !directory.is_dir() {
            continue;
        }
        let mut manifest = File::open(directory.join("manifest"))?;
        let mut length = [0; 8];
        manifest.read_exact(&mut length)?;
        let length = u64::from_le_bytes(length);
        if length > manifest.metadata()?.len().saturating_sub(8) {
            return Err(invalid("Truncated measurement store header"));
        }
        let header: Value = serde_json::from_reader(manifest.take(length))?;
        let names = header["sections"]
            .as_array()
            .ok_or_else(|| invalid("Missing store section names"))?;
        for file in fs::read_dir(&directory)? {
            let file = file?;
            let metadata = file.metadata()?;
            if !metadata.is_file() {
                continue;
            }
            let bytes = metadata.len();
            total += bytes;
            let file_name = file.file_name();
            let name = file_name.to_string_lossy();
            let section = name
                .split('_')
                .next()
                .and_then(|index| index.parse::<usize>().ok())
                .and_then(|index| names.get(index))
                .and_then(|row| row["name"].as_str())
                .unwrap_or("store-metadata");
            *sections.entry(section.to_owned()).or_insert(0) += bytes;
        }
    }
    Ok((total, sections))
}

#[cfg(windows)]
fn private_bytes() -> io::Result<u64> {
    #[repr(C)]
    struct Counters {
        cb: u32,
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
    #[link(name = "psapi")]
    unsafe extern "system" {
        fn GetProcessMemoryInfo(
            process: windows_sys::Win32::Foundation::HANDLE,
            counters: *mut Counters,
            size: u32,
        ) -> i32;
    }
    let mut counters: Counters = unsafe { std::mem::zeroed() };
    counters.cb = std::mem::size_of::<Counters>() as u32;
    if unsafe {
        GetProcessMemoryInfo(
            windows_sys::Win32::System::Threading::GetCurrentProcess(),
            &mut counters,
            std::mem::size_of::<Counters>() as u32,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(counters.private as u64)
}

#[cfg(not(windows))]
fn private_bytes() -> io::Result<u64> {
    Err(invalid(
        "Preparation private-memory measurement requires Windows",
    ))
}
