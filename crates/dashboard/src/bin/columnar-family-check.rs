//! Prepares one family as segment operators over a columnar conversion of a recording
//! ([`replay::segments`]) and compares the sections it writes, byte for byte, with those a
//! finished preparation of the same recording wrote, flushing at the hours that preparation
//! flushed (its series batches' last hours). Reports the CPU per recorded hour of the
//! projection (every worker) and of the ordered emission (the caller's thread), and of the
//! same walk with no operator, which the family's own cost is measured above. Without
//! `--prepared` it only measures, flushing once at the end; `--row` folds army, fuel and
//! diplomacy through their production block paths, both flushing at each row block's end.
//! `columnar-family-check --family stability|supply|division-log|army|fuel|diplomacy_state --recording <r.battleplan>
//!   --columnar <c.h4col> [--prepared <prepared store folder>] --output <new folder> [--threads N]`
//! Both walks read only the family's tables; `--all-tables` walks every one. A family reading
//! game content takes it from BATTLE_PLAN_GAME_DIR and BATTLE_PLAN_USER_DIR. `--scopes`
//! also reports the CPU each accounting scope charged over both walks.

use dashboard::prepared_stability::StabilityAccumulator;
use dashboard_country_fuel::products::country_fuel;
use dashboard_diplomacy::products::diplomacy_state;
use dashboard_forces::products::{army_size, supply_health};
use dashboard_land_battles::products::division_log;
use dashboard_stability::products::country_stability::columnar as stability;
use replay::{
    columnar_frames::{Limits, Recording},
    container::Replay,
    segments,
};
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
    DivisionLog(Box<division_log::Scan>),
    Army(Box<army_size::prepared::Prepared>),
    Fuel(
        Box<country_fuel::prepared::Prepared>,
        replay_data::format::FormatVersion,
    ),
    Diplomacy(Box<diplomacy_state::prepared::Prepared>),
}

impl Family {
    /// The family's catalogue name and its series sections of `branch`, whose batches date
    /// the reference preparation's flushes.
    fn sections(name: &str, branch: u64) -> (&'static str, Vec<String>) {
        match name {
            "fuel" => (
                "fuel",
                vec![format!("fuel-{branch}"), format!("fuel-days-{branch}")],
            ),
            "diplomacy_state" => (
                diplomacy_state::prepared::FAMILY_NAME,
                vec![diplomacy_state::prepared::section(branch)],
            ),
            "army" => (
                "army",
                (0..8)
                    .map(|index| format!("army-values-{branch}-{index}"))
                    .chain((0..8).map(|index| format!("army-ranks-{branch}-{index}")))
                    .chain([
                        format!("army-parts-{branch}"),
                        format!("army-coverage-{branch}"),
                        format!("army-aid-values-{branch}"),
                        format!("army-aid-proof-{branch}"),
                        format!("army-aid-occurrences-{branch}"),
                    ])
                    .collect(),
            ),
            "supply" => (
                "supply",
                std::iter::once(format!("supply-runs-{branch}"))
                    .chain((0..3).map(|bucket| format!("supply-values-{branch}-{bucket}")))
                    .collect(),
            ),
            "division-log" => (
                "division-log-count-keys",
                vec![format!("division-log-counts-{branch}")],
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
        if name == "diplomacy_state" {
            return section
                .strip_prefix("diplomacy-state-")
                .and_then(|rest| digits(rest.strip_suffix("-catalogue").unwrap_or(rest)))
                .or_else(|| section.strip_prefix("diplomacy-series-").and_then(digits));
        }
        if name == "army" {
            for prefix in ["army-", "army-aid-"] {
                if let Some(branch) = section
                    .strip_prefix(prefix)
                    .and_then(|rest| rest.strip_suffix("-catalogue"))
                    .and_then(digits)
                {
                    return Some(branch);
                }
            }
            return [
                "army-values-",
                "army-ranks-",
                "army-parts-",
                "army-coverage-",
                "army-aid-values-",
                "army-aid-proof-",
                "army-aid-occurrences-",
            ]
            .iter()
            .find_map(|prefix| {
                let rest = section.strip_prefix(prefix)?;
                let branch = if matches!(*prefix, "army-values-" | "army-ranks-") {
                    rest.rsplit_once('-')?.0
                } else {
                    rest
                };
                digits(branch)
            });
        }
        let (catalogue, series) = match name {
            "fuel" => ("fuel-", ["fuel-", "fuel-days-"].as_slice()),
            "supply" => ("supply-", ["supply-runs-", "supply-values-"].as_slice()),
            "division-log" => (
                "division-log-count-keys-",
                [
                    "division-log-manifest-",
                    "division-log-counts-",
                    "division-log-",
                ]
                .as_slice(),
            ),
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
            Self::Fuel(prepared, _) => {
                let hour = *hour
                    .downcast::<country_fuel::columnar::Hour>()
                    .map_err(|_| invalid("A projection returned another hour"))?;
                let at = (hour.branch, hour.hour);
                prepared.observe_projected(hour)?;
                Ok(at)
            }
            Self::Diplomacy(prepared) => {
                let hour = *hour
                    .downcast::<diplomacy_state::columnar::Hour>()
                    .map_err(|_| invalid("A projection returned another hour"))?;
                let at = (hour.branch, hour.hour);
                prepared.observe_projected(hour)?;
                Ok(at)
            }
            Self::Army(prepared) => {
                let hour = *hour
                    .downcast::<army_size::columnar::Hour>()
                    .map_err(|_| invalid("A projection returned another hour"))?;
                let at = (hour.branch, hour.hour);
                prepared.observe_projected(hour)?;
                Ok(at)
            }
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
            Self::DivisionLog(scan) => {
                let hour = *hour
                    .downcast::<division_log::columnar::Hour>()
                    .map_err(|_| invalid("A projection returned another hour"))?;
                let at = (hour.branch, hour.hour);
                scan.observe_projected(hour)?;
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
            Self::Army(prepared) => prepared.flush(writer, branch, cancelled),
            Self::Fuel(prepared, format) => prepared.flush(writer, branch, *format, cancelled),
            Self::Diplomacy(prepared) => {
                prepared.flush(writer, branch, cancelled)?;
                prepared.save(writer, branch)
            }
            // Its pages are published at each block's end.
            Self::DivisionLog(_) => Ok(()),
        }
    }
}

/// Splits one segment's projected hours, so each reaches its branch's state alone.
fn hours(result: Box<dyn std::any::Any + Send>) -> io::Result<Vec<Box<dyn std::any::Any + Send>>> {
    let result = match result.downcast::<Vec<country_fuel::columnar::Hour>>() {
        Ok(hours) => return Ok(hours.into_iter().map(|hour| Box::new(hour) as _).collect()),
        Err(result) => result,
    };
    let result = match result.downcast::<Vec<diplomacy_state::columnar::Hour>>() {
        Ok(hours) => return Ok(hours.into_iter().map(|hour| Box::new(hour) as _).collect()),
        Err(result) => result,
    };
    let result = match result.downcast::<Vec<army_size::columnar::Hour>>() {
        Ok(hours) => return Ok(hours.into_iter().map(|hour| Box::new(hour) as _).collect()),
        Err(result) => result,
    };
    let result = match result.downcast::<Vec<stability::Hour>>() {
        Ok(hours) => return Ok(hours.into_iter().map(|hour| Box::new(hour) as _).collect()),
        Err(result) => result,
    };
    let result = match result.downcast::<Vec<supply_health::columnar::Hour>>() {
        Ok(hours) => return Ok(hours.into_iter().map(|hour| Box::new(hour) as _).collect()),
        Err(result) => result,
    };
    match result.downcast::<Vec<division_log::columnar::Hour>>() {
        Ok(hours) => Ok(hours.into_iter().map(|hour| Box::new(hour) as _).collect()),
        Err(_) => Err(invalid("A projection returned another result")),
    }
}

/// The branch an hour from [`hours`] belongs to.
fn branch(hour: &(dyn std::any::Any + Send)) -> io::Result<u64> {
    hour.downcast_ref::<diplomacy_state::columnar::Hour>()
        .map(|hour| hour.branch)
        .or_else(|| {
            hour.downcast_ref::<country_fuel::columnar::Hour>()
                .map(|hour| hour.branch)
        })
        .or_else(|| {
            hour.downcast_ref::<stability::Hour>()
                .map(|hour| hour.branch)
        })
        .or_else(|| {
            hour.downcast_ref::<army_size::columnar::Hour>()
                .map(|hour| hour.branch)
        })
        .or_else(|| {
            hour.downcast_ref::<supply_health::columnar::Hour>()
                .map(|hour| hour.branch)
        })
        .or_else(|| {
            hour.downcast_ref::<division_log::columnar::Hour>()
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

    fn take(&mut self) -> io::Result<Box<dyn std::any::Any + Send>> {
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
            "columnar-family-check --family F --recording R --columnar C [--prepared FOLDER] --output NEW [--threads N]",
        )
    };
    let (mut recording, mut columnar, mut prepared, mut output) = (None, None, None, None);
    let mut family = "stability".to_owned();
    let mut scopes = false;
    let mut all_tables = false;
    let mut row = false;
    let mut threads = std::thread::available_parallelism().map_or(1, usize::from);
    let mut args = std::env::args_os().skip(1);
    while let Some(flag) = args.next() {
        if flag == "--scopes" {
            scopes = true;
            continue;
        }
        if flag == "--all-tables" {
            all_tables = true;
            continue;
        }
        if flag == "--row" {
            row = true;
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
    let (Some(recording), Some(columnar), Some(output)) = (recording, columnar, output) else {
        return Err(usage());
    };
    // These row paths read recording data alone.
    if row
        && (prepared.is_some() || !matches!(family.as_str(), "army" | "fuel" | "diplomacy_state"))
    {
        return Err(usage());
    }
    std::fs::create_dir(&output)?;
    if scopes {
        replay_frame::cpu::enable();
    }
    let local = output.join("local");
    std::fs::create_dir_all(&local)?;
    // Set before any other thread starts: the scratch store lives under it.
    unsafe { std::env::set_var("LOCALAPPDATA", &local) };

    if ![
        "stability",
        "supply",
        "division-log",
        "army",
        "fuel",
        "diplomacy_state",
    ]
    .contains(&family.as_str())
    {
        return Err(usage());
    }
    let replay = Replay::open_path(&recording)?;
    let format = replay.format.version();
    // The recording's source and content, for a family that reads content.
    let (source, context) = if matches!(
        family.as_str(),
        "stability" | "army" | "fuel" | "diplomacy_state"
    ) {
        (None, None)
    } else {
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
        (
            Some(source),
            Some(dashboard::config::LoadContext {
                content: Some(content),
            }),
        )
    };
    // Supply's prototype, configured with the content's defines.
    let prototype = match (family.as_str(), &source, &context) {
        ("supply", Some(source), Some(context)) => {
            let mut prepared = supply_health::prepared::Prepared::default();
            prepared.configure(source, context.content.as_ref())?;
            Some(prepared)
        }
        _ => None,
    };
    // Each branch's row blocks' last hours, by their ordinal in the branch.
    let mut block_ends: BTreeMap<(u64, i32), usize> = BTreeMap::new();
    let mut ordinals: BTreeMap<u64, usize> = BTreeMap::new();
    for block in &replay.index {
        let ordinal = ordinals.entry(block.branch).or_default();
        block_ends.insert((block.branch, block.last_hour), *ordinal);
        *ordinal += 1;
    }
    let mut file = Recording::open_path(&columnar, Limits::default())?;
    if let Some(damage) = file.damage().first() {
        return Err(invalid(format!("The columnar file is damaged: {damage}")));
    }
    // Both walks read only the family's tables, unless asked for every one.
    if !all_tables {
        file.select(match family.as_str() {
            "army" => army_size::prepared::TABLES,
            "fuel" => country_fuel::columnar::TABLES,
            "diplomacy_state" => diplomacy_state::block::TABLES,
            "supply" => supply_health::prepared::TABLES,
            "division-log" => division_log::block::FAMILY.tables,
            _ => stability::TABLES,
        });
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
    let mut publish_ns = 0_u64;
    let make = |_: &replay::columnar_frames::Segment| -> io::Result<segments::Operators> {
        Ok(match (family.as_str(), &prototype) {
            ("fuel", _) => vec![Box::new(country_fuel::columnar::Projector::new(&replay)?)],
            ("diplomacy_state", _) => vec![Box::new(diplomacy_state::columnar::Projector::new(
                replay.format,
                replay.capture_mode,
            )?)],
            ("army", _) => vec![Box::new(
                army_size::prepared::Prepared::default().projector(&replay)?,
            )],
            ("supply", Some(prepared)) => vec![Box::new(prepared.projector(replay.format)?)],
            ("division-log", _) => vec![Box::new(division_log::columnar::Projector::new(
                replay.format,
                replay.capture_mode,
            )?)],
            _ => vec![Box::new(stability::Projector::new(format)?)],
        })
    };
    segments::run(&file, threads, 2 * threads, &make, &mut |_, results| {
        let started = cpu().1;
        for result in results {
            for hour in hours(result)? {
                let branch = branch(&*hour)?;
                if !flushes.contains_key(&branch) {
                    let mut hours = Vec::new();
                    if let Some(prepared) = &prepared {
                        for section in Family::sections(&family, branch).1 {
                            hours.extend(flush_hours(prepared, &section)?);
                        }
                    } else if row {
                        hours.extend(
                            block_ends
                                .keys()
                                .filter(|(at, _)| *at == branch)
                                .map(|(_, hour)| *hour),
                        );
                    }
                    hours.sort_unstable();
                    hours.dedup();
                    flushes.insert(branch, hours);
                }
                let state = accumulators.entry(branch).or_insert_with(|| {
                    match (family.as_str(), &prototype) {
                        ("army", _) => Family::Army(Box::default()),
                        ("fuel", _) => Family::Fuel(Box::default(), replay.format),
                        ("diplomacy_state", _) => Family::Diplomacy(Box::default()),
                        ("supply", Some(prepared)) => Family::Supply(Box::new(prepared.fresh())),
                        ("division-log", _) => {
                            Family::DivisionLog(Box::new(division_log::Scan::new(replay.format)))
                        }
                        _ => Family::Stability(StabilityAccumulator::default()),
                    }
                });
                let (_, at) = state.observe(hour)?;
                let publishing = cpu().1;
                if let Family::DivisionLog(scan) = state {
                    // Its window, manifest and counts are published at each block's end.
                    if let (Some(&ordinal), Some(source), Some(context)) =
                        (block_ends.get(&(branch, at)), &source, &context)
                    {
                        scan.publish_block(
                            source,
                            context,
                            &mut writer,
                            branch,
                            ordinal,
                            &cancelled,
                        )?;
                    }
                } else if flushes[&branch].binary_search(&at).is_ok() {
                    state.flush(&mut writer, branch, &cancelled)?;
                }
                publish_ns += cpu().1 - publishing;
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
    let hours = file.hours().max(1) as f64;
    let mut report = json!({
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
        "emit_publish_cpu_us_per_hour": publish_ns as f64 / 1e3 / hours,
        "caller_cpu_us_per_hour": (thread_after - thread_before) as f64 / 1e3 / hours,
        "sections_written": written.len(),
        "scopes": scopes.then(replay_frame::cpu::snapshot),
    });
    let reference = match &prepared {
        Some(prepared) => Some(prepared.clone()),
        None if row => Some(row_reference(
            &family, &replay, &recording, &output, &local,
        )?),
        None => None,
    };
    if let Some(prepared) = &reference {
        compare(
            &mut report,
            &family,
            &recording,
            &store,
            prepared,
            &local,
            &written,
            &written_key,
            &accumulators,
        )?;
    }
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

/// The production block folds, with the checker's flush at each block's end.
fn row_reference(
    family: &str,
    replay: &Replay,
    recording: &Path,
    output: &Path,
    local: &Path,
) -> io::Result<PathBuf> {
    let root = output.join("row");
    std::fs::create_dir_all(&root)?;
    // No other thread runs now; the reference store lives under its own root.
    unsafe { std::env::set_var("LOCALAPPDATA", &root) };
    let store = dashboard::prepared::recording_store(recording);
    unsafe { std::env::set_var("LOCALAPPDATA", local) };
    let store = store?;
    let mut writer = store.replace(store::SourceKey::capture(recording, &[])?)?;
    let cancelled = AtomicBool::new(false);
    if family == "fuel" {
        let source = dashboard::Source::open(
            dashboard::SourceKey {
                source_id: "fuel-row-reference".into(),
                revision: dashboard::protocol::replay_revision(recording)?,
            },
            recording.to_owned(),
            &cancelled,
        )?;
        let mut states: BTreeMap<u64, dashboard::prepared::pipeline::FuelReference> =
            BTreeMap::new();
        for (ordinal, block) in replay.index.iter().enumerate() {
            let state = states.entry(block.branch).or_insert_with(|| {
                dashboard::prepared::pipeline::FuelReference::new(replay.format)
            });
            state.block(&source, replay, block, &cancelled)?;
            state.flush(&mut writer, replay, block.branch, &cancelled)?;
            if replay
                .index
                .get(ordinal + 1)
                .is_none_or(|next| next.branch != block.branch)
            {
                state.reload()?;
            }
        }
    } else {
        let mut states: BTreeMap<u64, Family> = BTreeMap::new();
        let contract = if family == "diplomacy_state" {
            &diplomacy_state::block::FAMILY
        } else {
            &army_size::prepared::FAMILY
        };
        let mut tables = contract.tables.to_vec();
        tables.sort_unstable_by_key(|table| table.index());
        tables.dedup();
        for block in &replay.index {
            let mut extract = (contract.start)(replay.format);
            let mut tracker = dashboard_base::prepared::frame_changes::FrameChangeTracker::new();
            let mut hours = 0_usize;
            replay.read_block_alone(
                block,
                &tables,
                |raw, frame| {
                    hours += 1;
                    let changes =
                        tracker.observe_updates(frame, std::sync::Arc::new(raw), replay.format);
                    extract.observe(replay, frame, &changes)
                },
                || Ok(()),
            )?;
            let state = states.entry(block.branch).or_insert_with(|| {
                if family == "diplomacy_state" {
                    Family::Diplomacy(Box::default())
                } else {
                    Family::Army(Box::default())
                }
            });
            if hours == 0 {
                if let Family::Diplomacy(state) = state {
                    state.break_segment();
                }
                continue;
            }
            match state {
                Family::Diplomacy(state) => {
                    diplomacy_state::block::fold(
                        state,
                        extract.finish()?,
                        replay,
                        &mut |_: i32, result: io::Result<()>| result.map(|_| true),
                        &mut || Ok(()),
                    )?;
                }
                Family::Army(state) => {
                    state
                        .fold(
                            extract.finish()?,
                            replay,
                            block,
                            &|_: i32, result: io::Result<()>| result,
                            &mut || -> io::Result<()> { Ok(()) },
                        )
                        .map_err(|failure| failure.error)?;
                }
                _ => unreachable!("row family"),
            }
            state.flush(&mut writer, block.branch, &cancelled)?;
        }
    }
    writer.commit()?;
    Ok(std::fs::read_dir(root.join("HOI4Replay/prepared"))?
        .next()
        .ok_or_else(|| invalid("The row reference store holds no folder"))??
        .path())
}

/// Adds to `report` how the sections `written` compare with the `prepared` reference's.
fn compare(
    report: &mut serde_json::Value,
    family: &str,
    recording: &Path,
    store: &store::Store,
    prepared: &Path,
    local: &Path,
    written: &BTreeMap<String, Vec<Vec<u8>>>,
    written_key: &store::SourceKey,
    accumulators: &BTreeMap<u64, Family>,
) -> io::Result<()> {
    let reference = sections(prepared)?;
    // Every section either store holds of the family, so a branch or section the walk
    // missed is compared too.
    let owns = |name: &str| Family::owns(family, name).is_some();
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
    let reference_store = dashboard::prepared::recording_store(recording)?;
    unsafe { std::env::set_var("LOCALAPPDATA", local) };
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
                .filter_map(|name| Family::owns(family, name)),
        )
        .collect();
    for branch in &branches {
        let (catalogue, sections) = Family::sections(family, *branch);
        for section in sections {
            if family == "army"
                && (section.starts_with("army-ranks-") || section.starts_with("army-aid-"))
            {
                continue;
            }
            semantic.insert(
                section.clone(),
                first_difference(
                    (store, written_key),
                    (&reference_store, &reference_key),
                    (catalogue, &section),
                    *branch,
                )?,
            );
        }
    }
    report["branches"] = json!(branches);
    report["sections_differing"] = json!(differing);
    report["first_series_difference"] = json!(semantic);
    report["sections"] = json!(compared);
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
