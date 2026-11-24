mod admission;
mod consumer;
mod convert;
mod gate;
mod payloads;
mod selection;
#[cfg(test)]
mod tests;
mod timing;

use core_spool::{Budget, Compression, Config, Layout, Next, PayloadReader, Reader};
use selection::{Candidate, Range, Samples};
use serde::Deserialize;
use std::{
    fs::{self, File, OpenOptions},
    hint::black_box,
    io::{self, BufReader, BufWriter, Read, Write},
    path::{Path, PathBuf},
};
use timing::{Clock, Time};

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}
struct Options {
    input: PathBuf,
    out: PathBuf,
    memory: u64,
    disk: u64,
    blocks: Vec<usize>,
    passes: u64,
    readers: u64,
    lanes: u64,
    dictionary_pool: u64,
    state_pool: u64,
    writer_budget: f64,
}
impl Options {
    fn parse() -> io::Result<Self> {
        let mut result = Self {
            input: PathBuf::new(),
            out: PathBuf::new(),
            memory: 3_000_000_000,
            disk: 1_000_000_000_000,
            blocks: vec![16384, 65536, 262144],
            passes: 3,
            readers: 8,
            lanes: 8,
            dictionary_pool: 900_000_000,
            state_pool: 500_000_000,
            writer_budget: 150.0,
        };
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            let value = args
                .next()
                .ok_or_else(|| invalid("expected a value after each option"))?;
            match arg.as_str() {
                "--input" => result.input = value.into(),
                "--out" => result.out = value.into(),
                "--memory-bytes" => result.memory = value.parse().map_err(io::Error::other)?,
                "--disk-bytes" => result.disk = value.parse().map_err(io::Error::other)?,
                "--block-bytes" => {
                    result.blocks = value
                        .split(',')
                        .map(|n| n.parse().map_err(io::Error::other))
                        .collect::<io::Result<_>>()?
                }
                "--readers-in-flight" => {
                    result.readers = value.parse().map_err(io::Error::other)?
                }
                "--ordered-lanes" => result.lanes = value.parse().map_err(io::Error::other)?,
                "--dictionary-pool-bytes" => {
                    result.dictionary_pool = value.parse().map_err(io::Error::other)?
                }
                "--state-pool-bytes" => {
                    result.state_pool = value.parse().map_err(io::Error::other)?
                }
                "--writer-budget-us-per-hour" => {
                    result.writer_budget = value.parse().map_err(io::Error::other)?
                }
                "--passes" => result.passes = value.parse().map_err(io::Error::other)?,
                _ => return Err(invalid(format!("unknown option {arg}"))),
            }
        }
        if result.input.as_os_str().is_empty()
            || result.out.as_os_str().is_empty()
            || result.passes < 3
            || result.readers < 8
            || result.lanes == 0
            || !result.writer_budget.is_finite()
            || result.writer_budget <= 0.0
            || result.writer_budget > 1860.0
            || result.blocks.is_empty()
        {
            return Err(invalid(
                "use --input <gate corpus> and --out <new directory>; at least three passes and eight readers, positive budgets and block sizes are required",
            ));
        }
        Ok(result)
    }
}
#[derive(Deserialize)]
struct TimingMetadata {
    hours: u64,
}
#[derive(Deserialize)]
struct Metadata {
    #[serde(default)]
    hours: Option<u64>,
    #[serde(default)]
    timing: Option<TimingMetadata>,
}
fn hours(corpus: &Path, segment: &str, family: &str, memory: &Budget) -> io::Result<u64> {
    let number = segment
        .strip_prefix("segment-")
        .ok_or_else(|| invalid("invalid Gate segment directory"))?
        .parse::<u64>()
        .map_err(io::Error::other)?;
    let delivery = corpus.join(format!("delivery-{family}/{number:08}.json"));
    let path = if delivery.is_file() {
        delivery
    } else {
        corpus.join(format!("segment-reports/{number:08}.json"))
    };
    let _held = memory
        .owner(core_spool::Owner::Metadata)
        .reserve(64 * 1024)?;
    let meta: Metadata =
        serde_json::from_reader(BufReader::new(File::open(&path)?)).map_err(io::Error::other)?;
    meta.hours
        .or(meta.timing.map(|t| t.hours))
        .filter(|h| *h != 0)
        .ok_or_else(|| invalid(format!("no recorded hour count in {}", path.display())))
}
fn sources(
    corpus: &Path,
    family: &str,
    mut visit: impl FnMut(&str, &Path) -> io::Result<()>,
) -> io::Result<usize> {
    let mut count = 0;
    let mut previous = None;
    loop {
        let mut next = None::<(u64, String)>;
        for entry in fs::read_dir(corpus)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(number) = name
                .strip_prefix("segment-")
                .and_then(|s| s.parse::<u64>().ok())
            else {
                continue;
            };
            if !entry.file_type()?.is_dir() || previous.is_some_and(|p| number <= p) {
                continue;
            }
            if next.as_ref().is_none_or(|(n, _)| number < *n) {
                next = Some((number, name));
            }
        }
        let Some((number, name)) = next else {
            break;
        };
        let path = corpus.join(&name).join(format!("{family}.spool"));
        if !path.is_file() {
            return Err(invalid(format!(
                "Gate source not recorded: {}",
                path.display()
            )));
        }
        visit(&name, &path)?;
        count += 1;
        previous = Some(number);
    }
    if count == 0 {
        return Err(invalid(format!("no Gate spools in {}", corpus.display())));
    }
    Ok(count)
}
fn index_families(input: &Path, index: &Path, disk: &Budget) -> io::Result<()> {
    fs::create_dir(index)?;
    for corpus in ["1941", "13y"] {
        for dir in fs::read_dir(input.join(corpus))? {
            let dir = dir?;
            let name = dir.file_name();
            if !name.to_string_lossy().starts_with("segment-") || !dir.file_type()?.is_dir() {
                continue;
            }
            for file in fs::read_dir(dir.path())? {
                let file = file?;
                let path = file.path();
                if !file.file_type()?.is_file() || path.extension().is_none_or(|e| e != "spool") {
                    continue;
                }
                let family = path
                    .file_stem()
                    .ok_or_else(|| invalid("invalid spool filename"))?;
                let marker = index.join(family);
                if !marker.exists() {
                    disk.grow_disk(4096)?;
                    OpenOptions::new()
                        .create_new(true)
                        .write(true)
                        .open(marker)?;
                }
            }
        }
    }
    Ok(())
}
#[derive(Default)]
struct Stats {
    records: u64,
    native: u64,
    hours: u64,
    bytes: u64,
    zstd: u64,
    sources: u64,
    writer_passes: u64,
    raw_decode: Time,
    encode: Time,
    live: Time,
    verify: Time,
    blocks: u64,
    block_bytes: u64,
    peak: u64,
    selection_bytes: u64,
    owners: [u64; core_spool::OWNER_COUNT],
    counts: [u64; 17],
    writer_paging: [(u64, u64, u64); core_spool::OWNER_COUNT],
    writer_samples: Samples,
}
impl Stats {
    fn peaks(&mut self, memory: &Budget) {
        self.peak = self.peak.max(memory.peak() + self.selection_bytes);
        for (i, peak) in memory.owner_peaks().into_iter().enumerate() {
            let peak = peak
                + if i == core_spool::Owner::Metadata as usize {
                    self.selection_bytes
                } else {
                    0
                };
            self.owners[i] = self.owners[i].max(peak);
        }
    }
}
fn verify(
    path: &Path,
    config: Config,
    memory: &Budget,
    expected: [u8; 32],
    source_kinds: &Path,
) -> io::Result<(u64, Time)> {
    let start = Clock::start()?;
    let mut reader = Reader::open(path, config.block_bytes, memory)?;
    let mut payload = PayloadReader::open(path, 65536, memory)?;
    let _side_held = memory.owner(core_spool::Owner::Metadata).reserve(8192)?;
    let mut source_kinds = BufReader::new(File::open(source_kinds)?);
    let mut hash = convert::Canonical::default();
    let mut count = 0;
    loop {
        match reader.next_block()? {
            Next::Block(block) => {
                for hour in block.hours() {
                    for (table, field, record) in hour.ordered() {
                        let source_kind = if record.kind == core_spool::Kind::Unknown {
                            let mut kind = [0];
                            source_kinds.read_exact(&mut kind)?;
                            Some(kind[0])
                        } else {
                            None
                        };
                        hash.spool_record(
                            hour.hour,
                            table,
                            field,
                            record,
                            &mut payload.for_block(block.location),
                            source_kind,
                        )
                        .map_err(|e| {
                            invalid(format!(
                                "hours {}..={}: {e}",
                                block.location.first, block.location.last
                            ))
                        })?;
                        count += 1;
                    }
                }
            }
            Next::Finished => break,
            Next::Pending => return Err(invalid("sealed spool ended without its finish commit")),
        }
    }
    if source_kinds.read(&mut [0])? != 0 {
        return Err(invalid("unused bench source kinds"));
    }
    if hash.finish() != expected {
        return Err(invalid("Gate/spool semantic SHA-256 differs"));
    }
    Ok((count, start.stop()?))
}
fn size(path: &Path) -> io::Result<u64> {
    let mut size = 0_u64;
    for file in fs::read_dir(path)? {
        size = size
            .checked_add(file?.metadata()?.len())
            .ok_or_else(|| invalid("spool size overflow"))?;
    }
    Ok(size)
}
fn drain(
    reader: &mut Reader,
    payload: &mut PayloadReader,
    mut state: Option<&mut consumer::Consumer>,
) -> io::Result<(u64, u64, u64)> {
    let (mut count, mut blocks, mut bytes) = (0, 0, 0);
    loop {
        match reader.next_block()? {
            Next::Block(block) => {
                count += block.records;
                blocks += 1;
                bytes += block.raw_bytes() as u64;
                for hour in block.hours() {
                    for (table, field, record) in hour.ordered() {
                        if let Some(state) = state.as_deref_mut() {
                            state.apply(
                                hour.hour,
                                table,
                                field,
                                record,
                                &mut payload.for_block(block.location),
                            )?;
                        } else {
                            black_box((
                                hour.hour,
                                table,
                                field,
                                record.order,
                                record.kind,
                                record.key.bytes(),
                                record.value,
                            ));
                            if let Some(blob) = record.blob() {
                                payload.for_block(block.location).blob(blob, |bytes| {
                                    black_box(bytes);
                                    Ok(())
                                })?;
                            }
                        }
                    }
                }
            }
            Next::Finished | Next::Pending => break,
        }
    }
    Ok((count, blocks, bytes))
}
fn convert_source(
    source: &Path,
    out: &Path,
    config: Config,
    allowances: core_spool::Allowances,
    hourly: bool,
    memory: &Budget,
    disk: &Budget,
    mut live_state: Option<&mut consumer::Consumer>,
    stats: &mut Stats,
) -> io::Result<()> {
    let held = memory.owner(core_spool::Owner::GateDecoder).reserve(8192)?;
    let mut canonical = convert::Canonical::default();
    let start = Clock::start()?;
    let (records, raw) = gate::replay(BufReader::new(File::open(source)?), &mut canonical, memory)?;
    stats.raw_decode.add(start.stop()?);
    let expected = canonical.finish();
    let side_path = out.with_extension("source-kinds");
    let _side_held = memory.owner(core_spool::Owner::Metadata).reserve(8192)?;
    let side_file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&side_path)?;
    let mut source_kinds = BufWriter::new(side_file);
    let mut charged_side = SideWriter {
        file: &mut source_kinds,
        disk,
    };
    let mut side = convert::SourceKinds::new(&mut charged_side);
    gate::replay(BufReader::new(File::open(source)?), &mut side, memory)?;
    if let Some(error) = side.error {
        return Err(error);
    }
    source_kinds.flush()?;
    let start = Clock::start()?;
    let before_paging = memory.paging();
    let mut writer = core_spool::Writer::create(out, config, allowances, memory, disk)?;
    let mut live = Time::default();
    let mut live_handles = if hourly {
        let start = Clock::start()?;
        let handles = (
            Reader::open(out, config.block_bytes, memory)?,
            PayloadReader::open(out, 65536, memory)?,
        );
        live.add(start.stop()?);
        Some(handles)
    } else {
        None
    };
    let mut live_count = 0;
    if let Some(state) = live_state.as_deref_mut() {
        let start = Clock::start()?;
        state.begin_segment()?;
        live.add(start.stop()?);
    }
    let mut publish = || -> io::Result<()> {
        let start = Clock::start()?;
        let (reader, payload) = live_handles.as_mut().unwrap();
        live_count += drain(reader, payload, live_state.as_deref_mut())?.0;
        live.add(start.stop()?);
        Ok(())
    };
    let counts = {
        let mut convert = convert::Convert::new(&mut writer);
        convert.publish_each_hour = hourly;
        if hourly {
            convert.on_publish = Some(&mut publish);
        }
        let decoded = gate::replay(BufReader::new(File::open(source)?), &mut convert, memory)?;
        if let Some(error) = convert.error {
            return Err(error);
        }
        if decoded != (records, raw) || convert.hash.finish() != expected {
            return Err(invalid("Gate conversion baseline differs"));
        }
        convert.counts
    };
    writer.finish()?;
    if hourly {
        publish()?;
    }
    stats.encode.add(start.stop()?);
    for (i, (misses, reloads, spilled)) in memory.paging().into_iter().enumerate() {
        stats.writer_paging[i].0 += misses - before_paging[i].0;
        stats.writer_paging[i].1 += reloads - before_paging[i].1;
        stats.writer_paging[i].2 += spilled - before_paging[i].2;
    }
    if hourly && live_count != counts.iter().sum::<u64>() {
        return Err(invalid("hourly concurrent delivery count differs"));
    }
    stats.live.add(live);
    let (native, verified) = verify(out, config, memory, expected, &side_path)?;
    if native != counts.iter().sum::<u64>() {
        return Err(invalid("native record count differs"));
    }
    stats.verify.add(verified);
    stats.native += native;
    stats.records += records;
    stats.bytes += size(out)?;
    stats.zstd += fs::metadata(source.with_extension("spool.zst"))?.len();
    stats.sources += 1;
    for (i, count) in counts.into_iter().enumerate() {
        stats.counts[i] += count;
    }
    drop(held);
    stats.peaks(memory);
    Ok(())
}
struct Measurement {
    time: Time,
    passes: u64,
    paging: [(u64, u64, u64); core_spool::OWNER_COUNT],
    samples: Samples,
}
fn deliver_corpus(
    corpus_in: &Path,
    corpus_out: &Path,
    family: &str,
    config: Config,
    options: &Options,
    disk: &Budget,
    apply: bool,
    stats: &mut Stats,
) -> io::Result<Measurement> {
    let mut result = Measurement {
        time: Time::default(),
        passes: 0,
        paging: [(0, 0, 0); core_spool::OWNER_COUNT],
        samples: Samples::default(),
    };
    while result.passes < options.passes || result.time.cpu_ns < 50_000_000 {
        let memory = Budget::new(options.memory - stats.selection_bytes);
        let before_cpu = result.time.cpu_ns;
        let mut state = if apply {
            Some(consumer::Consumer::create(
                &corpus_out.join(format!("apply-{}", result.passes)),
                admission::consumer_allowances(options.state_pool / options.lanes)?,
                &memory,
                disk,
            )?)
        } else {
            None
        };
        let mut count = 0;
        sources(corpus_in, family, |segment, _| {
            let start = Clock::start()?;
            if let Some(state) = state.as_mut() {
                state.begin_segment()?;
            }
            let path = corpus_out.join(segment);
            let mut reader = Reader::open(&path, config.block_bytes, &memory)?;
            let mut payload = PayloadReader::open(&path, 65536, &memory)?;
            let (n, blocks, bytes) = drain(&mut reader, &mut payload, state.as_mut())?;
            count += n;
            if !matches!(reader.next_block()?, Next::Finished) {
                return Err(invalid("unfinished spool during delivery"));
            }
            result.time.add(start.stop()?);
            if !apply && result.passes == 0 {
                stats.blocks += blocks;
                stats.block_bytes += bytes;
            }
            Ok(())
        })?;
        if count != stats.native {
            return Err(invalid("delivery count differs"));
        }
        if let Some(state) = state.as_mut() {
            if state.applied != count {
                return Err(invalid("current-value apply count differs"));
            }
            black_box(state);
        }
        for (i, (misses, reloads, spilled)) in memory.paging().into_iter().enumerate() {
            result.paging[i].0 += misses;
            result.paging[i].1 += reloads;
            result.paging[i].2 += spilled;
        }
        result
            .samples
            .push((result.time.cpu_ns - before_cpu) as f64);
        result.passes += 1;
        stats.peaks(&memory);
    }
    Ok(result)
}
struct Report {
    file: File,
    disk: Budget,
}
impl Report {
    fn write(&mut self, text: &str) -> io::Result<()> {
        self.disk.grow_disk(text.len() as u64)?;
        self.file.write_all(text.as_bytes())?;
        self.file.flush()
    }
}
fn report_paging(
    report: &mut Report,
    phase: &str,
    counters: &[(u64, u64, u64); core_spool::OWNER_COUNT],
    passes: u64,
) -> io::Result<()> {
    const OWNERS: [&str; core_spool::OWNER_COUNT] = [
        "Other",
        "Staging",
        "WriterBlocks",
        "Codec",
        "Dictionary",
        "ReaderBlocks",
        "PayloadPages",
        "GateDecoder",
        "Metadata",
        "CurrentValues",
        "Reasons",
    ];
    report.write(&format!("Paging: {phase}, {passes} passes.\n\n| Owner | Page misses/pass | Backing reads/pass | Spilled B/pass |\n|---|---:|---:|---:|\n"))?;
    for (i, &(misses, reads, bytes)) in counters.iter().enumerate() {
        if misses != 0 || reads != 0 || bytes != 0 {
            report.write(&format!(
                "| {} | {:.3} | {:.3} | {:.3} |\n",
                OWNERS[i],
                misses as f64 / passes as f64,
                reads as f64 / passes as f64,
                bytes as f64 / passes as f64
            ))?;
        }
    }
    report.write("\n")
}
struct SideWriter<'a> {
    file: &'a mut BufWriter<File>,
    disk: &'a Budget,
}
impl Write for SideWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.disk.grow_disk(bytes.len() as u64)?;
        self.file.write_all(bytes)?;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}
fn report_owners(report: &mut Report, stats: &Stats, options: &Options) -> io::Result<()> {
    let writer = admission::writer_allowances(options.dictionary_pool, options.readers)?;
    let state = admission::consumer_allowances(options.state_pool / options.lanes)?;
    let shared = options.memory - options.dictionary_pool - options.state_pool;
    report.write("| Owner | Allowance B (per instance unless shared) | Measured resident peak B |\n|---|---:|---:|\n")?;
    for (i, name) in [
        "Other",
        "Staging",
        "WriterBlocks",
        "Codec",
        "Dictionary",
        "ReaderBlocks",
        "PayloadPages",
        "GateDecoder",
        "Metadata",
        "CurrentValues",
        "Reasons",
    ]
    .iter()
    .enumerate()
    {
        let allowance = match i {
            4 => writer.dictionary.to_string(),
            9 => state.total().to_string(),
            10 => writer.reasons.to_string(),
            _ => format!("{shared} shared remainder"),
        };
        let bound = if *name == "Codec" {
            " (reservation bound, not measured)"
        } else {
            ""
        };
        report.write(&format!("| {name} | {allowance} | {}{bound} |\n", stats.owners[i]))?;
    }
    report.write("\nThe shared remainder is one envelope, not additive per owner; this isolated format bench does not measure the other engine owners. Cache directories and slot metadata are resident bytes; unused page capacity is not charged. Codec is a reservation bound for LZ4's per-call hash table, so totals that include it are bounds too.\n\n")
}
fn writer_sample(stats: &Stats) -> f64 {
    stats.encode.cpu_ns as f64 - stats.raw_decode.cpu_ns as f64 - stats.live.cpu_ns as f64
}
fn run(options: Options) -> io::Result<()> {
    if options
        .dictionary_pool
        .checked_add(options.state_pool)
        .is_none_or(|n| n >= options.memory)
    {
        return Err(invalid(
            "dictionary and state pools must leave a shared transient remainder",
        ));
    }
    let writer_allowances = admission::writer_allowances(options.dictionary_pool, options.readers)?;
    let consumer_allowances = admission::consumer_allowances(options.state_pool / options.lanes)?;
    let calibration = timing::calibrate()?;
    fs::create_dir(&options.out)?;
    let disk = Budget::new(options.disk);
    let index = options.out.join("families");
    index_families(&options.input, &index, &disk)?;
    let mut report = Report {
        file: OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(options.out.join("report.md"))?,
        disk: disk.clone(),
    };
    report.write(&format!("# Typed spool measurement\n\nReal Gate 1 inputs: {}. UNTRIMMED format cost at Gate volume; PR #1280 supplies the read-set trim. All 1941 and 13y segments run in order with consumer state carried. No handler, stitch, utilisation or whole-engine result is claimed.\n\nCPU uses QueryThreadCycleTime calibrated at {:.3} cycles/s; smallest observed delta {} cycles. At least {} independent corpus passes and 50 ms thread CPU per mode. Reported ranges are observed pass minima and maxima, not confidence intervals. Raw cycles, pipeline and live-delivery durations are retained. Writer CPU is the signed differential of decode+convert+writer minus independent raw decode/hash and timed live delivery; no concurrency divisor. Bench-only source kinds are streamed in a separate untimed extraction and used only for semantic verification. They occupy no production record bytes.\n\nCaller admission: {} B overall, {} B key/reason pool divided over {} readers in flight, {} B ordered-state pool divided over {} lanes; the remainder is shared transient admission. These are the caller's configurable allocations from design 2.8, not library fractions of a Budget. Writer key allowance {} B, separate reason allowance {} B; consumer {} B. Pages reserve on first residence and spill past their allowances, including metadata. Eight simultaneous writers sharing one Budget have synthetic coverage. Other design owners are outside this isolated bench. Disk admission {} B.\n\nConsumer uses a single-level leaf index per column, directly indexed resident page arrays for values, remaps and column indexes, and an associative LRU overflow. Typed cells carry row generations and evidence; payloads use reference-counted size classes released on overwrite and row lifecycle changes. Apply is reported separately as the signed differential of combined minus format-only delivery, with conservative range bounds. Fixed-width Changed records do not copy payloads or walk a radix tree.\n\nChoice: every corpus must pass serial delivery+apply <= 29.7 us/h, differential writer <= an assumed writer-only ceiling of {:.3} us/h, and writer+delivery+apply <= 1,860 us/h, using observed upper bounds. The ceiling's default is the whole reader's lower bound in design 2.9 (150-400 us/h for the whole reader); no writer share of that range has been derived, so passing it leaves the reader's other work unaccounted. Negative writer ranges are inconclusive. Among passing candidates, minimise total CPU; declare a winner only if its upper bound is below every rival's lower bound. Otherwise report a timing tie and select by bytes within the overlapping group. Aggregate range bounds sum corpus extrema weighted by hours; they are derived, not separately measured passes. Packed and hourly publication are measured separately.\n\n", options.input.display(), calibration.hz, calibration.observed_step, options.passes, options.memory, options.dictionary_pool, options.readers, options.state_pool, options.lanes, writer_allowances.dictionary, writer_allowances.reasons, consumer_allowances.total(), options.disk, options.writer_budget))?;
    for family in fs::read_dir(&index)? {
        let family = family?
            .file_name()
            .into_string()
            .map_err(|_| invalid("non-UTF8 family"))?;
        let family_out = options.out.join(&family);
        fs::create_dir(&family_out)?;
        report.write(&format!("## {family}\n\n"))?;
        for hourly in [false, true] {
            let mode = if hourly { "hourly" } else { "packed" };
            let candidate_count = options
                .blocks
                .len()
                .checked_mul(4)
                .ok_or_else(|| invalid("candidate count overflow"))?;
            let candidate_bytes = candidate_count
                .checked_mul(size_of::<Candidate>())
                .ok_or_else(|| invalid("candidate metadata overflow"))?
                as u64;
            let candidate_memory = Budget::new(options.memory);
            let _candidate_held = candidate_memory
                .owner(core_spool::Owner::Metadata)
                .reserve(candidate_bytes)?;
            let admitted = options
                .memory
                .checked_sub(candidate_bytes)
                .ok_or_else(|| invalid("candidate metadata exceeds admission"))?;
            let mut candidates = Vec::new();
            candidates
                .try_reserve_exact(candidate_count)
                .map_err(io::Error::other)?;
            if candidates.capacity() != candidate_count {
                return Err(invalid("candidate metadata exceeded reservation"));
            }
            for &block_bytes in &options.blocks {
                for layout in [Layout::Rows, Layout::Columns] {
                    for compression in [Compression::None, Compression::Lz4] {
                        let config = Config {
                            block_bytes,
                            layout,
                            compression,
                        };
                        let variant_out = family_out
                            .join(format!("{mode}-{layout:?}-{compression:?}-{block_bytes}"));
                        fs::create_dir(&variant_out)?;
                        let (mut total_serial, mut total_writer, mut total_hours, mut total_bytes) =
                            (Range::default(), Range::default(), 0, 0);
                        let mut passes_budgets = true;
                        for corpus in ["1941", "13y"] {
                            let corpus_in = options.input.join(corpus);
                            let corpus_out = variant_out.join(corpus);
                            fs::create_dir(&corpus_out)?;
                            let memory = Budget::new(admitted);
                            let mut live_state = if hourly {
                                Some(consumer::Consumer::create(
                                    &corpus_out.join("live-state"),
                                    consumer_allowances,
                                    &memory,
                                    &disk,
                                )?)
                            } else {
                                None
                            };
                            let mut stats = Stats {
                                selection_bytes: candidate_bytes,
                                ..Stats::default()
                            };
                            sources(&corpus_in, &family, |segment, source| {
                                stats.hours += hours(&corpus_in, segment, &family, &memory)?;
                                eprintln!(
                                    "{family} {corpus} {segment} {mode} {layout:?} {compression:?} {block_bytes}"
                                );
                                convert_source(
                                    source,
                                    &corpus_out.join(segment),
                                    config,
                                    writer_allowances,
                                    hourly,
                                    &memory,
                                    &disk,
                                    live_state.as_mut(),
                                    &mut stats,
                                )
                            })?;
                            drop(live_state);
                            stats.writer_samples.push(writer_sample(&stats));
                            stats.writer_passes = 1;
                            while stats.writer_passes < options.passes
                                || stats.encode.cpu_ns.saturating_sub(stats.live.cpu_ns)
                                    < 50_000_000
                            {
                                let repeat = corpus_out
                                    .join(format!("writer-repeat-{}", stats.writer_passes));
                                fs::create_dir(&repeat)?;
                                let mut state = if hourly {
                                    Some(consumer::Consumer::create(
                                        &repeat.join("state"),
                                        consumer_allowances,
                                        &memory,
                                        &disk,
                                    )?)
                                } else {
                                    None
                                };
                                let mut extra = Stats {
                                    selection_bytes: candidate_bytes,
                                    ..Stats::default()
                                };
                                sources(&corpus_in, &family, |segment, source| {
                                    convert_source(
                                        source,
                                        &repeat.join(segment),
                                        config,
                                        writer_allowances,
                                        hourly,
                                        &memory,
                                        &disk,
                                        state.as_mut(),
                                        &mut extra,
                                    )
                                })?;
                                for i in 0..core_spool::OWNER_COUNT {
                                    stats.writer_paging[i].0 += extra.writer_paging[i].0;
                                    stats.writer_paging[i].1 += extra.writer_paging[i].1;
                                    stats.writer_paging[i].2 += extra.writer_paging[i].2;
                                }
                                stats.writer_samples.push(writer_sample(&extra));
                                stats.encode.add(extra.encode);
                                stats.raw_decode.add(extra.raw_decode);
                                stats.live.add(extra.live);
                                stats.verify.add(extra.verify);
                                stats.peaks(&memory);
                                stats.writer_passes += 1;
                            }
                            let iteration = deliver_corpus(
                                &corpus_in,
                                &corpus_out,
                                &family,
                                config,
                                &options,
                                &disk,
                                false,
                                &mut stats,
                            )?;
                            let combined = deliver_corpus(
                                &corpus_in,
                                &corpus_out,
                                &family,
                                config,
                                &options,
                                &disk,
                                true,
                                &mut stats,
                            )?;
                            let delivery = iteration.samples.range();
                            let serial = combined.samples.range();
                            let writer = stats.writer_samples.range();
                            let apply = serial.difference(delivery);
                            let h = stats.hours as f64;
                            let n = stats.native as f64;
                            if h == 0.0 || n == 0.0 {
                                return Err(invalid("empty corpus has no cost measurement"));
                            }
                            let serial_us = serial.divided(1000.0 * h);
                            let writer_us = writer.divided(1000.0 * h);
                            let total_us = serial_us.plus(writer_us);
                            let pass =
                                selection::passes(serial_us, writer_us, options.writer_budget);
                            passes_budgets &= pass;
                            report.write("| Corpus | Mode | Layout | Codec | Block B | Typed records | Delivery ns/record | Apply increment ns/record | Serial us/h mean [min, max] / 29.7 | Writer us/h mean [min, max] / assumed writer ceiling | Total CPU us/h bounds / 1860 | Pass both | Bytes / zstd-3 | Peak resident B |\n|---|---|---|---|---:|---:|---:|---:|---|---|---|---|---|---:|\n")?;
                            report.write(&format!("| {corpus} | {mode} | {layout:?} | {compression:?} | {block_bytes} | {} | {:.3} | {:.3} | {:.3} [{:.3}, {:.3}] / 29.7 | {:.3} [{:.3}, {:.3}] / {:.3} | {:.3} [{:.3}, {:.3}] / 1860 | {pass} | {} / {} | {} |\n\n", stats.native, delivery.mean/n, apply.mean/n, serial_us.mean, serial_us.min, serial_us.max, writer_us.mean, writer_us.min, writer_us.max, options.writer_budget, total_us.mean, total_us.min, total_us.max, stats.bytes, stats.zstd, stats.peak))?;
                            report.write(&format!("Format-only delivery {:.3} [{:.3}, {:.3}] us/h; derived apply increment {:.3} [{:.3}, {:.3}] us/h. CPU ns and cycles: raw decode/hash {} / {}; writer pipeline {} / {}; live delivery/apply {} / {}; verification {} / {}; delivery {} / {} ({} passes); combined {} / {} ({} passes); writer {} passes. Pipeline {:.3} us/h; live {:.3} us/h; combined wall {:.3} us/h. {} Gate records, {} segments, {} hours, {} blocks, mean {:.1} raw B/block, {:.1} records/block. Emitted kinds {:?}.\n\n", delivery.mean/1000.0/h, delivery.min/1000.0/h, delivery.max/1000.0/h, apply.mean/1000.0/h, apply.min/1000.0/h, apply.max/1000.0/h, stats.raw_decode.cpu_ns, stats.raw_decode.cycles, stats.encode.cpu_ns, stats.encode.cycles, stats.live.cpu_ns, stats.live.cycles, stats.verify.cpu_ns, stats.verify.cycles, iteration.time.cpu_ns, iteration.time.cycles, iteration.passes, combined.time.cpu_ns, combined.time.cycles, combined.passes, stats.writer_passes, stats.encode.cpu_ns as f64/stats.writer_passes as f64/1000.0/h, stats.live.cpu_ns as f64/stats.writer_passes as f64/1000.0/h, combined.time.wall_ns as f64/combined.passes as f64/1000.0/h, stats.records, stats.sources, stats.hours, stats.blocks, stats.block_bytes as f64/stats.blocks as f64, n/stats.blocks as f64, stats.counts))?;
                            report_owners(&mut report, &stats, &options)?;
                            report_paging(
                                &mut report,
                                "writer pipeline (Dictionary excludes live CurrentValues)",
                                &stats.writer_paging,
                                stats.writer_passes,
                            )?;
                            report_paging(
                                &mut report,
                                "combined delivery/apply",
                                &combined.paging,
                                combined.passes,
                            )?;
                            total_serial = total_serial.plus(serial);
                            total_writer = total_writer.plus(writer);
                            total_hours += stats.hours;
                            total_bytes += stats.bytes;
                        }
                        let serial = total_serial.divided(1000.0 * total_hours as f64);
                        let writer = total_writer.divided(1000.0 * total_hours as f64);
                        let cpu = serial.plus(writer);
                        report.write(&format!("Candidate {mode} {layout:?} {compression:?} {block_bytes}: serial {:.3} [{:.3}, {:.3}] us/h / 29.7, writer {:.3} [{:.3}, {:.3}] us/h / {:.3}, summed CPU {:.3} [{:.3}, {:.3}] us/h / 1860, {} bytes; all corpora pass: {passes_budgets}.\n\n", serial.mean, serial.min, serial.max, writer.mean, writer.min, writer.max, options.writer_budget, cpu.mean, cpu.min, cpu.max, total_bytes))?;
                        candidates.push(Candidate {
                            config,
                            serial,
                            writer,
                            bytes: total_bytes,
                            passes_budgets,
                        });
                    }
                }
            }
            if let Some((chosen, tied)) = selection::choose(&candidates) {
                let candidate = &candidates[chosen];
                let config = candidate.config;
                let eligible = candidates.iter().filter(|c| c.passes_budgets).count();
                let result = if eligible == 1 {
                    "Only eligible candidate"
                } else if tied == 1 {
                    "Winner beyond observed spread"
                } else {
                    "Timing tie; bytes fallback"
                };
                report.write(&format!("{result}: {family}, {mode}, {:?}, {:?}, {} B block, {} bytes; {} overlapping candidate(s), serial {:.3} us/h, writer {:.3} us/h, full-weight CPU {:.3} us/h. Untrimmed format cost only.\n\n", config.layout, config.compression, config.block_bytes, candidate.bytes, tied, candidate.serial.mean, candidate.writer.mean, candidate.cpu().mean))?;
            } else {
                report.write(&format!("No eligible choice for {family}, {mode}: no candidate passed both budgets in every corpus over its observed range.\n\n"))?;
            }
        }
    }
    report.write(&format!("Peak reserved disk {} / {} bytes, including all retained candidates and paged consumer scratch.\n", disk.peak(), disk.limit()))?;
    Ok(())
}
fn main() {
    if let Err(error) = Options::parse().and_then(run) {
        eprintln!("core-spool-bench: {error}");
        std::process::exit(1);
    }
}
