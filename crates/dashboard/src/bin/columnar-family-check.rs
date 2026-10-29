//! Prepares one family as segment operators over a columnar conversion of a recording
//! ([`replay::segments`]) and compares the sections it writes, byte for byte, with those a
//! finished preparation of the same recording wrote, flushing at the hours that preparation
//! flushed (its series batches' last hours). Reports the CPU per recorded hour of the
//! projection (every worker) and of the ordered emission (the caller's thread), and of the
//! same walk with no operator, which the family's own cost is measured above.
//! `columnar-family-check --family stability|supply --recording <r.battleplan>
//!   --columnar <c.h4col> --prepared <prepared store folder> --output <new folder> [--threads N]`
//! A family reading game content takes it from BATTLE_PLAN_GAME_DIR and BATTLE_PLAN_USER_DIR. `--scopes`
//! also reports the CPU each accounting scope charged over both walks.

use dashboard::prepared_stability::StabilityAccumulator;
use dashboard_forces::products::supply_health;
use dashboard_stability::products::country_stability::columnar as stability;
use replay::{columnar_frames::Recording, container::Replay, segments};
use serde_json::json;
use std::{
    collections::BTreeMap,
    io::{self, Read},
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
    time::Instant,
};

#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, what.into())
}

/// CPU nanoseconds of the process and of the calling thread so far.
fn cpu() -> (u64, u64) {
    use windows_sys::Win32::{
        Foundation::FILETIME,
        System::Threading::{GetCurrentProcess, GetCurrentThread, GetProcessTimes, GetThreadTimes},
    };
    let zero = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let ticks =
        |time: FILETIME| u64::from(time.dwHighDateTime) << 32 | u64::from(time.dwLowDateTime);
    let (mut created, mut exited, mut kernel, mut user) = (zero, zero, zero, zero);
    // SAFETY: the pseudo-handles of the current process and thread and four owned FILETIMEs each.
    unsafe {
        GetProcessTimes(
            GetCurrentProcess(),
            &mut created,
            &mut exited,
            &mut kernel,
            &mut user,
        )
    };
    let process = (ticks(kernel) + ticks(user)) * 100;
    unsafe {
        GetThreadTimes(
            GetCurrentThread(),
            &mut created,
            &mut exited,
            &mut kernel,
            &mut user,
        )
    };
    (process, (ticks(kernel) + ticks(user)) * 100)
}

/// Every range of every section a prepared folder's manifest lists, by section name.
fn sections(folder: &Path) -> io::Result<BTreeMap<String, Vec<Vec<u8>>>> {
    let listing = store::Listing::open(&folder.join("manifest"))?
        .ok_or_else(|| invalid(format!("{} holds no manifest", folder.display())))?;
    let mut sections = BTreeMap::new();
    for (index, (name, count)) in listing.sections().enumerate() {
        let mut ranges = Vec::new();
        for range in 0..count {
            let mut bytes = Vec::new();
            if let Some(id) = listing.range(index, range)? {
                id.open(folder)?.read_to_end(&mut bytes)?;
            }
            ranges.push(bytes);
        }
        sections.insert(name.to_owned(), ranges);
    }
    Ok(sections)
}

/// The first series of `family`'s branch whose runs differ between the scratch store and
/// the reference, matched by catalogue name; None when every series agrees.
fn first_difference(
    (written, written_key): (&store::Store, &store::SourceKey),
    (reference, reference_key): (&store::Store, &store::SourceKey),
    (family, section): (&str, &str),
    branch: u64,
) -> io::Result<Option<String>> {
    use dashboard::prepared::series::{self, Catalogue};
    let cancelled = AtomicBool::new(false);
    let mine: Option<Catalogue<serde_json::Value>> =
        series::read_catalogue(written, written_key, family, branch)?;
    let theirs: Option<Catalogue<serde_json::Value>> =
        series::read_catalogue(reference, reference_key, family, branch)?;
    let (Some(mine), Some(theirs)) = (mine, theirs) else {
        return Ok(Some("a catalogue is missing".into()));
    };
    if mine.1 != theirs.1 {
        return Ok(Some("the catalogues' metadata differ".into()));
    }
    let mut ours = series::Reader::new(written, written_key, section.into(), &cancelled)?;
    let mut others = series::Reader::new(reference, reference_key, section.into(), &cancelled)?;
    for (name, &key) in &theirs.0 {
        let Some(&own) = mine.0.get(name) else {
            return Ok(Some(format!("{name} is missing")));
        };
        let (a, b) = (
            others.runs(key, i32::MIN, i32::MAX)?,
            ours.runs(own, i32::MIN, i32::MAX)?,
        );
        if a != b {
            let at = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
            return Ok(Some(format!(
                "{name}: run {at} of {} is {:?}, written {:?} (of {})",
                a.len(),
                a.get(at),
                b.get(at),
                b.len()
            )));
        }
    }
    Ok(mine
        .0
        .keys()
        .find(|name| !theirs.0.contains_key(*name))
        .map(|name| format!("{name} is not in the reference")))
}

/// A family as segment operators: one branch's ordered state, which keeps each projected
/// hour and flushes as the row path does.
enum Family {
    Stability(StabilityAccumulator),
    Supply(Box<supply_health::prepared::Prepared>),
}

impl Family {
    /// The family's catalogue name and its series sections of `branch`, whose batches date
    /// the reference preparation's flushes.
    fn sections(name: &str, branch: u64) -> (&'static str, Vec<String>) {
        match name {
            "supply" => (
                "supply",
                std::iter::once(format!("supply-runs-{branch}"))
                    .chain((0..3).map(|bucket| format!("supply-values-{branch}-{bucket}")))
                    .collect(),
            ),
            _ => ("stability", vec![format!("stability-{branch}")]),
        }
    }

    /// The branch whose section `name` is, when it is one of the family's.
    fn owns(name: &str, section: &str) -> Option<u64> {
        let digits = |text: &str| {
            (!text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit()))
                .then(|| text.parse().ok())
                .flatten()
        };
        let (catalogue, series) = match name {
            "supply" => ("supply-", ["supply-runs-", "supply-values-"].as_slice()),
            _ => ("stability-", ["stability-"].as_slice()),
        };
        if let Some(branch) = section
            .strip_prefix(catalogue)
            .and_then(|rest| rest.strip_suffix("-catalogue"))
        {
            return digits(branch);
        }
        series.iter().find_map(|prefix| {
            let rest = section.strip_prefix(prefix)?;
            let branch = match name {
                "supply" if *prefix == "supply-values-" => rest.rsplit_once('-')?.0,
                _ => rest,
            };
            digits(branch)
        })
    }

    fn observe(&mut self, hour: Box<dyn std::any::Any + Send>) -> io::Result<(u64, i32)> {
        match self {
            Self::Stability(accumulator) => {
                let hour = *hour
                    .downcast::<stability::Hour>()
                    .map_err(|_| invalid("A projection returned another hour"))?;
                let at = (hour.branch, hour.hour);
                accumulator.observe_projected(hour)?;
                Ok(at)
            }
            Self::Supply(prepared) => {
                let hour = *hour
                    .downcast::<supply_health::columnar::Hour>()
                    .map_err(|_| invalid("A projection returned another hour"))?;
                let at = (hour.branch, hour.hour);
                prepared.observe_projected(hour)?;
                Ok(at)
            }
        }
    }

    fn flush(
        &mut self,
        writer: &mut store::Writer,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        match self {
            Self::Stability(accumulator) => accumulator.flush_measures(writer, branch, cancelled),
            Self::Supply(prepared) => prepared.flush(writer, branch, cancelled),
        }
    }
}

/// Splits one segment's projected hours, so each reaches its branch's state alone.
fn hours(result: Box<dyn std::any::Any + Send>) -> io::Result<Vec<Box<dyn std::any::Any + Send>>> {
    let result = match result.downcast::<Vec<stability::Hour>>() {
        Ok(hours) => return Ok(hours.into_iter().map(|hour| Box::new(hour) as _).collect()),
        Err(result) => result,
    };
    match result.downcast::<Vec<supply_health::columnar::Hour>>() {
        Ok(hours) => Ok(hours.into_iter().map(|hour| Box::new(hour) as _).collect()),
        Err(_) => Err(invalid("A projection returned another result")),
    }
}

/// The branch an hour from [`hours`] belongs to.
fn branch(hour: &(dyn std::any::Any + Send)) -> io::Result<u64> {
    hour.downcast_ref::<stability::Hour>()
        .map(|hour| hour.branch)
        .or_else(|| {
            hour.downcast_ref::<supply_health::columnar::Hour>()
                .map(|hour| hour.branch)
        })
        .ok_or_else(|| invalid("A projection returned another hour"))
}

/// Visits every hour and keeps nothing: the walk's own cost.
struct Idle;

impl segments::Operator for Idle {
    fn hour(&mut self, _: &segments::Hour<'_>) -> io::Result<()> {
        Ok(())
    }

    fn finish(self: Box<Self>) -> io::Result<Box<dyn std::any::Any + Send>> {
        Ok(Box::new(()))
    }
}

/// The last hour of each batch a finished preparation flushed into `section`, in order.
fn flush_hours(folder: &Path, section: &str) -> io::Result<Vec<i32>> {
    let listing = store::Listing::open(&folder.join("manifest"))?
        .ok_or_else(|| invalid(format!("{} holds no manifest", folder.display())))?;
    let Some((index, (_, count))) = listing
        .sections()
        .enumerate()
        .find(|(_, (name, _))| *name == section)
    else {
        return Ok(Vec::new());
    };
    let mut hours = Vec::new();
    for range in 0..count {
        let id = listing
            .range(index, range)?
            .ok_or_else(|| invalid(format!("{section} lacks a batch")))?;
        let mut header = [0; 16];
        id.open(folder)?.read_exact(&mut header)?;
        hours.push(i32::from_le_bytes(
            header[4..8].try_into().expect("four bytes"),
        ));
    }
    Ok(hours)
}

fn run() -> io::Result<()> {
    let usage = || {
        invalid(
            "columnar-family-check --family F --recording R --columnar C --prepared FOLDER --output NEW [--threads N]",
        )
    };
    let (mut recording, mut columnar, mut prepared, mut output) = (None, None, None, None);
    let mut family = "stability".to_owned();
    let mut scopes = false;
    let mut threads = std::thread::available_parallelism().map_or(1, usize::from);
    let mut args = std::env::args_os().skip(1);
    while let Some(flag) = args.next() {
        if flag == "--scopes" {
            scopes = true;
            continue;
        }
        let value = args.next().ok_or_else(usage)?;
        match flag.to_str() {
            Some("--recording") => recording = Some(PathBuf::from(value)),
            Some("--columnar") => columnar = Some(PathBuf::from(value)),
            Some("--prepared") => prepared = Some(PathBuf::from(value)),
            Some("--output") => output = Some(PathBuf::from(value)),
            Some("--family") => family = value.to_string_lossy().into_owned(),
            Some("--threads") => {
                threads = value
                    .to_str()
                    .and_then(|text| text.parse().ok())
                    .filter(|&count| count > 0)
                    .ok_or_else(usage)?
            }
            _ => return Err(usage()),
        }
    }
    let (Some(recording), Some(columnar), Some(prepared), Some(output)) =
        (recording, columnar, prepared, output)
    else {
        return Err(usage());
    };
    std::fs::create_dir(&output)?;
    if scopes {
        replay_frame::cpu::enable();
    }
    let local = output.join("local");
    std::fs::create_dir_all(&local)?;
    // Set before any other thread starts: the scratch store lives under it.
    unsafe { std::env::set_var("LOCALAPPDATA", &local) };

    if !["stability", "supply"].contains(&family.as_str()) {
        return Err(usage());
    }
    let replay = Replay::open_path(&recording)?;
    let format = replay.format.version();
    // A family's prototype, configured with the recording's content where it reads some.
    let prototype = match family.as_str() {
        "supply" => {
            let game = std::env::var_os("BATTLE_PLAN_GAME_DIR").or_else(|| std::env::var_os("HOI4_GAME_DIR"))
                .map(PathBuf::from)
                .ok_or_else(|| invalid("Set BATTLE_PLAN_GAME_DIR to the installed game"))?;
            let user = std::env::var_os("BATTLE_PLAN_USER_DIR").or_else(|| std::env::var_os("HOI4_USER_DIR"))
                .map(PathBuf::from)
                .ok_or_else(|| invalid("Set BATTLE_PLAN_USER_DIR to the game's user folder"))?;
            engine_executable::executable_tables::initialize(&game)?;
            let content = dashboard::config::ContentOptions {
                options: installed_content_core::ResolveOptions {
                    game_dir: game,
                    user_dir: user,
                    cache_dir: output.join("content"),
                },
                allow_unverified: false,
                presentation: Default::default(),
            };
            std::fs::create_dir_all(&content.options.cache_dir)?;
            let source = dashboard::Source::open(
                dashboard::SourceKey {
                    source_id: "columnar-family-check".into(),
                    revision: dashboard::protocol::replay_revision(&recording)?,
                },
                recording.clone(),
                &AtomicBool::new(false),
            )?;
            let mut prepared = supply_health::prepared::Prepared::default();
            prepared.configure(&source, Some(&content))?;
            Some(prepared)
        }
        _ => None,
    };
    let file = Recording::open(std::fs::read(&columnar)?)?;
    if let Some(damage) = file.damage().first() {
        return Err(invalid(format!("The columnar file is damaged: {damage}")));
    }

    let store = dashboard::prepared::recording_store(&recording)?;
    let written_key = store::SourceKey::capture(&recording, &[])?;
    let mut writer = store.replace(written_key.clone())?;
    let cancelled = AtomicBool::new(false);
    let mut flushes: BTreeMap<u64, Vec<i32>> = BTreeMap::new();
    let idle = |_: &replay::columnar_frames::Segment| -> io::Result<segments::Operators> {
        Ok(vec![Box::new(Idle)])
    };
    let walk_started = (Instant::now(), cpu().0);
    segments::run(&file, threads, 2 * threads, &idle, &mut |_, _| Ok(()))?;
    let walk_wall = walk_started.0.elapsed().as_secs_f64();
    let walk_cpu = cpu().0 - walk_started.1;

    let wall = Instant::now();
    let (process_before, thread_before) = cpu();
    let mut accumulators: BTreeMap<u64, Family> = BTreeMap::new();
    let mut emit_ns = 0_u64;
    let make = |_: &replay::columnar_frames::Segment| -> io::Result<segments::Operators> {
        Ok(match &prototype {
            Some(prepared) => vec![Box::new(prepared.projector(replay.format)?)],
            None => vec![Box::new(stability::Projector::new(format)?)],
        })
    };
    segments::run(&file, threads, 2 * threads, &make, &mut |_, results| {
        let started = cpu().1;
        for result in results {
            for hour in hours(result)? {
                let branch = branch(&*hour)?;
                if !flushes.contains_key(&branch) {
                    let mut hours = Vec::new();
                    for section in Family::sections(&family, branch).1 {
                        hours.extend(flush_hours(&prepared, &section)?);
                    }
                    hours.sort_unstable();
                    hours.dedup();
                    flushes.insert(branch, hours);
                }
                let state = accumulators
                    .entry(branch)
                    .or_insert_with(|| match &prototype {
                        Some(prepared) => Family::Supply(Box::new(prepared.fresh())),
                        None => Family::Stability(StabilityAccumulator::default()),
                    });
                let (_, at) = state.observe(hour)?;
                if flushes[&branch].binary_search(&at).is_ok() {
                    state.flush(&mut writer, branch, &cancelled)?;
                }
            }
        }
        emit_ns += cpu().1 - started;
        Ok(())
    })?;
    let (process_after, thread_after) = cpu();
    let walked = wall.elapsed().as_secs_f64();

    for (branch, accumulator) in &mut accumulators {
        if flushes.get(branch).is_none_or(Vec::is_empty) {
            accumulator.flush(&mut writer, *branch, &cancelled)?;
        }
    }
    writer.commit()?;
    let scratch = std::fs::read_dir(local.join("HOI4Replay/prepared"))?
        .next()
        .ok_or_else(|| invalid("The scratch store holds no folder"))??
        .path();

    let written = sections(&scratch)?;
    let reference = sections(&prepared)?;
    // Every section either store holds of the family, so a branch or section the walk
    // missed is compared too.
    let owns = |name: &str| Family::owns(&family, name).is_some();
    let names: std::collections::BTreeSet<&String> = written
        .keys()
        .chain(reference.keys().filter(|name| owns(name)))
        .collect();
    let mut compared = Vec::new();
    let mut differing = 0;
    for name in names {
        let same = reference.get(name) == written.get(name);
        differing += usize::from(!same);
        compared.push(json!({
            "section": name,
            "ranges": written.get(name).map(Vec::len),
            "bytes": written.get(name).map(|ranges| ranges.iter().map(Vec::len).sum::<usize>()),
            "reference_ranges": reference.get(name).map(Vec::len),
            "same": same,
        }));
    }
    // The reference store opens under its own local root; no other thread runs now.
    let root = prepared
        .ancestors()
        .nth(3)
        .ok_or_else(|| invalid("--prepared is not a prepared store folder"))?;
    unsafe { std::env::set_var("LOCALAPPDATA", root) };
    let reference_store = dashboard::prepared::recording_store(&recording)?;
    unsafe { std::env::set_var("LOCALAPPDATA", &local) };
    let reference_key = reference_store
        .saved_key()?
        .ok_or_else(|| invalid("The reference store holds no key"))?;
    let mut semantic = BTreeMap::new();
    let branches: std::collections::BTreeSet<u64> = accumulators
        .keys()
        .copied()
        .chain(
            reference
                .keys()
                .filter_map(|name| Family::owns(&family, name)),
        )
        .collect();
    for branch in &branches {
        let (catalogue, sections) = Family::sections(&family, *branch);
        for section in sections {
            semantic.insert(
                section.clone(),
                first_difference(
                    (&store, &written_key),
                    (&reference_store, &reference_key),
                    (catalogue, &section),
                    *branch,
                )?,
            );
        }
    }
    let hours = file.hours().max(1) as f64;
    let report = json!({
        "family": family,
        "recording": recording,
        "columnar": columnar,
        "threads": threads,
        "segments": file.segments().len(),
        "hours": file.hours(),
        "wall_seconds": walked,
        "process_cpu_us_per_hour": (process_after - process_before) as f64 / 1e3 / hours,
        "walk_only_cpu_us_per_hour": walk_cpu as f64 / 1e3 / hours,
        "walk_only_wall_seconds": walk_wall,
        "family_cpu_us_per_hour": (process_after - process_before).saturating_sub(walk_cpu) as f64 / 1e3 / hours,
        "emit_cpu_us_per_hour": emit_ns as f64 / 1e3 / hours,
        "caller_cpu_us_per_hour": (thread_after - thread_before) as f64 / 1e3 / hours,
        "sections_written": written.len(),
        "branches": branches,
        "sections_differing": differing,
        "first_series_difference": semantic,
        "sections": compared,
        "scopes": scopes.then(replay_frame::cpu::snapshot),
    });
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("columnar-family-check: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}
