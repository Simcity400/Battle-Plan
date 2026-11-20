use crate::{Options, document::Document};
use replay_frame::cpu;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    time::Instant,
};

pub(crate) mod flight;
pub(crate) mod paced;
pub(crate) mod proof;
pub(crate) mod toggle;
pub(crate) mod viewpoint;

const CAPTURES: &str = "BATTLE_PLAN_CAPTURES";
const ROWS: &[(&str, &str, f64)] = &[
    (
        "territory",
        "political/territory/occupation, stock and scripted styles",
        19.0,
    ),
    (
        "modes/styles",
        "political/territory/occupation, stock and scripted styles",
        19.0,
    ),
    ("fog", "perspective/visibility/intel decisions", 36.0),
    ("weather", "weather/snow/mud", 22.5),
    ("buildings", "buildings", 8.5),
    ("rails", "shared rail/train sources", 10.5),
    ("labels", "labels/world fit", 7.5),
    ("world names", "labels/world fit", 7.5),
    ("actors/units", "units/counters/selection operands", 33.0),
    ("air/naval source choices", "air/naval source choices", 14.5),
    ("engagement/strikes/text", "engagement/strikes/text", 9.0),
    ("raid responses", "raid responses", 18.0),
    (
        "music/battle/audio descriptors",
        "music/battle/audio descriptors",
        3.5,
    ),
    (
        "codec",
        "product encoding/compression/hash/root work and amortized asset baking",
        18.0,
    ),
];

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

/// Each harness job runs on a map thread of the host the desktop app runs its maps on.
pub(crate) fn dispatch(args: &[OsString]) -> Option<io::Result<()>> {
    let job: fn(&[OsString], Window) -> io::Result<()> = if args.iter().any(|arg| arg == "--playhead-proof") {
        |args, _| proof::run(args)
    } else if args.iter().any(|arg| arg == "--visual-toggle") {
        |args, _| toggle::run(args)
    } else if args.iter().any(|arg| arg == "--viewpoint-switch") {
        |args, _| viewpoint::run(args)
    } else if args.iter().any(|arg| arg == "--playback-cost") {
        run
    } else {
        return None;
    };
    // A windowed run's frames go out through a real swap chain: this thread owns and pumps the
    // window the map thread draws into, as the desktop's host owns its maps' windows.
    if args.iter().any(|arg| arg == "--windowed") {
        let size = match window_size(args) {
            Ok(size) => size,
            Err(error) => return Some(Err(error)),
        };
        let args = args.to_vec();
        return Some(crate::host::paced_window(size, move |window, inputs| {
            job(&args, Window::Own(window.clone(), inputs))
        }));
    }
    let args = args.to_vec();
    Some(
        crate::host::run("map (harness)", move |_| job(&args, Window::None).map_err(|error| error.to_string()))
            .and_then(|ended| ended.map_err(crate::host::Ending::into_error)),
    )
}

/// The window a harness job draws into, when it has one of its own.
pub(crate) enum Window {
    None,
    Own(crate::host::Window, crate::host::Inputs),
}

/// The window a `--windowed` run makes, which it names itself.
fn window_size(args: &[OsString]) -> io::Result<[u32; 2]> {
    let mut size = [0u32; 2];
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let slot = match arg.to_str() {
            Some("--width") => 0,
            Some("--height") => 1,
            _ => continue,
        };
        size[slot] = args
            .next()
            .and_then(|value| value.to_str())
            .and_then(|value| value.parse().ok())
            .filter(|pixels| *pixels > 0)
            .ok_or_else(|| invalid("--width and --height need a positive pixel count"))?;
    }
    if size.contains(&0) {
        return Err(invalid("--windowed needs --width and --height"));
    }
    Ok(size)
}

/// Names a finished run's `cache` folder to start this run's caches from.
const SEED_CACHE: &str = "BATTLE_PLAN_PLAYBACK_SEED_CACHE";

/// Copies every file under `from` to the same place under `to`; how many.
fn copy_tree(from: &Path, to: &Path) -> io::Result<usize> {
    let mut copied = 0;
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copied += copy_tree(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), &target)?;
            copied += 1;
        }
    }
    Ok(copied)
}

struct Locks(Vec<(PathBuf, File)>);
impl Drop for Locks {
    fn drop(&mut self) {
        for (path, file) in self.0.drain(..).rev() {
            drop(file);
            let _ = fs::remove_file(path);
        }
    }
}

// No work waits for anything (AGENTS.md): the lock only keeps two runs out of one output.
fn lock(output: &Path) -> io::Result<Locks> {
    let path = output.join("MEASURE_RUNNING");
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?;
    let mut locks = Locks(vec![(path, file)]);
    writeln!(
        locks.0[0].1,
        "viewer --playback-cost pid={}",
        std::process::id()
    )?;
    Ok(locks)
}

fn hash(path: &Path) -> io::Result<String> {
    let mut input = File::open(path)?;
    let before = input.metadata()?;
    let mut hash = Sha256::new();
    let mut bytes = [0u8; 65536];
    loop {
        let count = input.read(&mut bytes)?;
        if count == 0 {
            break;
        }
        hash.update(&bytes[..count]);
    }
    let after = input.metadata()?;
    if before.len() != after.len() || before.modified()? != after.modified()? {
        return Err(invalid(format!(
            "Input changed while hashing: {}",
            path.display()
        )));
    }
    Ok(format!("{:x}", hash.finalize()))
}

fn labels(snapshot: &Value) -> BTreeMap<(String, String, String), u64> {
    snapshot["labels"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|row| {
            (
                (
                    row["role"].as_str().unwrap().to_owned(),
                    row["category"].as_str().unwrap().to_owned(),
                    row["family"].as_str().unwrap().to_owned(),
                ),
                row["ns"].as_u64().unwrap(),
            )
        })
        .collect()
}

fn run(args: &[OsString], window: Window) -> io::Result<()> {
    if viewer_foundation::profile_writer::profiling() {
        return Err(invalid("--playback-cost requires unprofiled playback"));
    }
    let mut translated = Vec::new();
    let mut hours = None;
    let mut pointer = None;
    let mut schedule = crate::app::playback_cost::Schedule {
        step: 1,
        pan: None,
        pngs: Vec::new(),
        png_frames: None,
        zoom: None,
        paused: false,
        refresh_hz: 0.0,
        complete_models: false,
        paced: None,
        seeks: 0,
        seek_back: None,
        layers: None,
        flight: None,
        windowed: false,
        campaign_tab: false,
        count_pictures: None,
    };
    let mut from = false;
    let mut speed = false;
    let mut step = false;
    // The app's frame rate limit for this run, which holds its frames below `--refresh-hz`.
    let mut frame_limit = None;
    // `--walk`, `--walk-keys` and `--walk-flying` in any order, joined after the loop: the
    // placed walk is the harness's, so the window dump's own refusals never see them.
    let (mut walk, mut walk_keys, mut walk_flying) = (None, None, false);
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--playback-cost") => translated.push(OsString::from("--dump")),
            Some("--from-hour") => {
                from = true;
                translated.push(OsString::from("--hour"));
            }
            Some("--hours") => {
                hours = Some(
                    args.next()
                        .and_then(|v| v.to_str())
                        .and_then(|v| v.parse::<u32>().ok())
                        .filter(|v| *v > 0)
                        .ok_or_else(|| invalid("--hours needs a positive u32"))?,
                );
            }
            Some("--hour-step") => {
                step = true;
                schedule.step = args
                    .next()
                    .and_then(|v| v.to_str())
                    .and_then(|v| v.parse::<u32>().ok())
                    .filter(|v| *v > 0)
                    .ok_or_else(|| invalid("--hour-step needs a positive u32"))?;
            }
            Some("--png-frame") => {
                schedule.pngs.push(
                    args.next()
                        .and_then(|v| v.to_str())
                        .and_then(|v| v.parse::<u32>().ok())
                        .ok_or_else(|| invalid("--png-frame needs a frame index"))?,
                );
            }
            Some("--pan") => {
                schedule.pan = Some(
                    args.next()
                        .and_then(|v| v.to_str())
                        .and_then(|v| v.parse::<f32>().ok())
                        .filter(|v| v.is_finite())
                        .ok_or_else(|| invalid("--pan needs map units per frame"))?,
                );
            }
            // The harness hovers this point, which a dump's options refuse.
            Some("--profile-pointer") => {
                let value = args.next().and_then(|v| v.to_str()).unwrap_or_default();
                pointer = Some(crate::parse_profile_pointer(value)?);
            }
            Some("--zoom") => {
                let value = args.next().and_then(|v| v.to_str()).unwrap_or_default();
                let (lines, every) = value
                    .split_once(':')
                    .and_then(|(lines, every)| {
                        Some((lines.parse::<f32>().ok()?, every.parse::<u32>().ok()?))
                    })
                    .filter(|(lines, every)| lines.is_finite() && *lines > 0.0 && *every > 0)
                    .ok_or_else(|| invalid("--zoom needs <wheel lines>:<every N frames>"))?;
                schedule.zoom = Some(crate::app::playback_cost::Zoom { lines, every });
            }
            Some("--paced") => {
                schedule.paced = Some(
                    args.next()
                        .and_then(|v| v.to_str())
                        .and_then(|v| v.parse::<f64>().ok())
                        .filter(|v| v.is_finite() && *v > 0.0)
                        .ok_or_else(|| invalid("--paced needs game hours per second"))?,
                );
            }
            Some("--flight") => {
                let value = args.next().and_then(|v| v.to_str()).unwrap_or_default();
                schedule.flight = Some(flight::Flight::parse(value)?);
            }
            Some("--walk-flying") => walk_flying = true,
            Some(flag @ ("--walk" | "--walk-keys")) => {
                let value = args
                    .next()
                    .and_then(|v| v.to_str())
                    .map(str::to_owned)
                    .ok_or_else(|| invalid(format!("{flag} needs a value")))?;
                if flag == "--walk" {
                    walk = Some(value);
                } else {
                    walk_keys = Some(value);
                }
            }
            Some("--png-frames") => {
                let value = args.next().and_then(|v| v.to_str()).unwrap_or_default();
                schedule.png_frames = Some(crate::app::playback_cost::Presented::parse(value)?);
            }
            Some("--count-pictures") => {
                let value = args.next().and_then(|v| v.to_str()).unwrap_or_default();
                schedule.count_pictures =
                    Some(crate::app::playback_cost::pictures::Counting::parse(value)?);
            }
            Some("--paused") => schedule.paused = true,
            // Consumed here: the window was made before this job started.
            Some("--windowed") => schedule.windowed = true,
            Some("--campaign-tab") => schedule.campaign_tab = true,
            Some("--register") => {
                schedule.layers = Some(
                    args.next()
                        .and_then(|v| v.to_str())
                        .map(|v| v.split(',').map(str::trim).filter(|name| !name.is_empty()).map(str::to_owned).collect())
                        .ok_or_else(|| invalid("--register needs the names of the layers the window registers, comma-separated"))?,
                );
            }
            Some("--seeks") => {
                schedule.seeks = args
                    .next()
                    .and_then(|v| v.to_str())
                    .and_then(|v| v.parse::<u32>().ok())
                    .ok_or_else(|| invalid("--seeks needs a count"))?;
            }
            Some("--seek-back") => {
                schedule.seek_back = Some(
                    args.next()
                        .and_then(|v| v.to_str())
                        .and_then(|v| v.parse::<i32>().ok())
                        .ok_or_else(|| invalid("--seek-back needs a game hour"))?,
                );
            }
            Some("--complete-models") => schedule.complete_models = true,
            // Before the device: the timestamps are a device feature.
            Some("--gpu-pass-times") => viewer_foundation::map::gpu_passes::record(),
            Some("--refresh-hz") => {
                schedule.refresh_hz = args
                    .next()
                    .and_then(|v| v.to_str())
                    .and_then(|v| v.parse::<f64>().ok())
                    .filter(|v| v.is_finite() && *v > 0.0)
                    .ok_or_else(|| invalid("--refresh-hz needs a positive rate"))?;
            }
            Some("--frame-limit") => {
                frame_limit = Some(
                    args.next()
                        .and_then(|v| v.to_str())
                        .and_then(|v| v.parse::<f64>().ok())
                        .filter(|v| dashboard::protocol::MAP_FRAME_LIMIT_RANGE.contains(v))
                        .ok_or_else(|| {
                            invalid(format!(
                                "--frame-limit needs a rate of {} to {} frames a second",
                                dashboard::protocol::MAP_FRAME_LIMIT_RANGE.start(),
                                dashboard::protocol::MAP_FRAME_LIMIT_RANGE.end()
                            ))
                        })?,
                );
            }
            Some("--speed") => {
                if args.next().map(|s| s.as_os_str()) != Some(std::ffi::OsStr::new("max")) {
                    return Err(invalid("--playback-cost only supports --speed max"));
                }
                speed = true;
            }
            Some(
                "--dump"
                | "--window-dump"
                | "--compare"
                | "--hour"
                | "--hour-fraction"
                | "--layers"
                | "--no-water"
                | "--no-trees"
                | "--no-shadows"
                | "--settle"
                | "--demilitarized"
                | "--building-camera"
                | "--debug-borders"
                | "--combat"
                | "--combat-at"
                | "--counters-json"
                | "--province-hover"
                | "--intel-hover"
                | "--presentation-time"
                | "--map-time",
            ) => return Err(invalid("Unsupported playback-cost option")),
            _ => translated.push(arg.clone()),
        }
    }
    if schedule.refresh_hz == 0.0 {
        return Err(invalid("--playback-cost requires --refresh-hz HZ"));
    }
    // The run's own frame clock stands for the display's refresh: the frame is budgeted for it,
    // and it caps the rate the simulation holds. `--frame-limit` is the app's setting, which holds
    // the maps below that rate, so the rate the run paces, budgets and reports at is what the two
    // together leave.
    viewer_foundation::map::frame_limit::limit_to(frame_limit);
    viewer_foundation::map::display_refresh::pace_at(schedule.refresh_hz);
    schedule.refresh_hz = viewer_foundation::map::display_refresh::rate();
    if !schedule.pngs.is_empty() && !schedule.complete_models {
        return Err(invalid(
            "--png-frame requires --complete-models; pixel proof runs are not timing runs",
        ));
    }
    if schedule.campaign_tab && schedule.paced.is_none() {
        return Err(invalid(
            "--campaign-tab measures the frames the app's campaign map tab runs while it plays, and needs --paced S",
        ));
    }
    if schedule.count_pictures.is_some() && schedule.paced.is_none() {
        return Err(invalid(
            "--count-pictures counts the pictures a paced run's display frames show and needs --paced S",
        ));
    }
    if schedule.png_frames.is_some() && schedule.paced.is_none() {
        return Err(invalid(
            "--png-frames saves the pixels of consecutive display frames a paced run presented and needs --paced S",
        ));
    }
    if schedule.windowed && schedule.paced.is_none() {
        return Err(invalid("--windowed presents a paced run through a real swap chain and needs --paced S"));
    }
    if schedule.flight.is_some()
        && (schedule.paced.is_none() || schedule.pan.is_some() || schedule.zoom.is_some())
    {
        return Err(invalid("--flight flies the camera of a paced run, by each frame's due time, and replaces --pan and --zoom"));
    }
    if schedule.seek_back.is_some() && (schedule.pan.is_some() || schedule.paused) {
        return Err(invalid("--seek-back seeks as the window does after the run's hours: not with --pan or --paused"));
    }
    let hours = hours.ok_or_else(|| invalid("--playback-cost requires --hours N"))?;
    if !from {
        return Err(invalid("--playback-cost requires --from-hour H"));
    }
    match schedule.paced {
        Some(_)
            if step
                || speed
                || (schedule.paused && schedule.seeks > 0)
                || schedule.zoom.is_some()
                || !schedule.pngs.is_empty() =>
        {
            return Err(invalid(
                "--paced replaces --hour-step and --speed, holds the hour for --hours frames with --paused, takes no --seeks with --paused, and runs without --zoom or --png-frame",
            ));
        }
        Some(_) => {}
        None if !speed => return Err(invalid("--playback-cost requires --speed max or --paced S")),
        None => {}
    }
    let walk = match walk {
        Some(place) => Some(
            crate::controls::walker::Start::parse(&place, walk_keys.as_deref(), walk_flying)
                .map_err(invalid)?,
        ),
        None if walk_keys.is_some() || walk_flying => {
            return Err(invalid(
                "--walk-keys and --walk-flying hold keys for a --walk",
            ));
        }
        None => None,
    };
    if walk.is_some() && (schedule.paced.is_none() || schedule.flight.is_some()) {
        return Err(invalid(
            "--walk walks the viewing country's infantry through a paced run, by each frame's due time, and replaces --flight",
        ));
    }
    let mut options = Options::parse(translated.into_iter())?;
    if walk.is_some() && options.camera.is_some() {
        return Err(invalid(
            "--walk places its own camera behind the figure; give it or a camera",
        ));
    }
    options.walk = walk;
    options.profile_pointer = pointer;
    if options.hour_fraction != 0.0 {
        return Err(invalid("--from-hour must be a whole game hour"));
    }
    let span = match (schedule.paced, schedule.pan) {
        (Some(_), _) => i64::from(hours),
        (None, Some(_)) => 0,
        (None, None) => (i64::from(hours) - 1) * i64::from(schedule.step),
    };
    let end = i64::from(options.hour.unwrap()) + span;
    options.playback_cost_end =
        Some(i32::try_from(end).map_err(|_| invalid("Hour range overflows"))?);
    if schedule.seek_back.is_some_and(|back| i64::from(back) >= end) {
        return Err(invalid("--seek-back takes an hour before the run's last"));
    }
    let recording = fs::canonicalize(options.replay.as_ref().unwrap())?;
    let output = options.out.as_ref().unwrap();
    let parent = fs::canonicalize(
        output
            .parent()
            .ok_or_else(|| invalid("Output needs a parent"))?,
    )?;
    let captures = std::env::var_os(CAPTURES)
        .ok_or_else(|| invalid("Set BATTLE_PLAN_CAPTURES to the folder --out must be under"))?;
    if !parent.starts_with(fs::canonicalize(captures)?) {
        return Err(invalid("--out must be a new directory under BATTLE_PLAN_CAPTURES"));
    }
    fs::create_dir(output)?;
    let output = fs::canonicalize(output)?;
    let _locks = lock(&output)?;
    let mut config = dashboard::Config::load()?;
    if let Some(path) = &options.game_dir {
        config.game_dir = path.clone();
    }
    if let Some(path) = &options.user_dir {
        config.user_dir = path.clone();
    }
    let mut resolve = config.resolve_options()?;
    resolve.cache_dir = output.join("cache/content");
    fs::create_dir_all(&resolve.cache_dir)?;
    let local = output.join("cache/local");
    let roaming = output.join("cache/roaming");
    fs::create_dir_all(&local)?;
    fs::create_dir_all(&roaming)?;
    // A finished run's caches (its compiled shaders), so a run does not compile them all again.
    // A harness setting for measurement runs only; the product never reads it.
    let seeded = match std::env::var_os(SEED_CACHE) {
        Some(seed) => Some(copy_tree(Path::new(&seed), &output.join("cache"))?),
        None => None,
    };
    // Isolation is set before any viewer worker exists.
    unsafe {
        std::env::set_var("LOCALAPPDATA", local);
        std::env::set_var("APPDATA", roaming);
    }
    cpu::enable();
    let monitor = crate::app::playback_cost::Monitor::start()?;
    let setup = Instant::now();
    crate::render::probe_dump_device()?;
    let document = Document::load(
        &recording,
        &resolve,
        options.allow_unverified,
        &crate::document::InitialHour {
            branch: options.branch,
            hour: options.hour,
            hour_fraction: 0.0,
        },
        (!options.visual_mods.is_empty()).then_some(options.visual_mods.as_slice()),
        0,
    )?;
    engine_executable::executable_tables::enter(document.content.tables);
    // What the stack's defines, visual mods' included, gave the camera, and what loading refused.
    let camera_law = format!("{:?}", document.camera_law);
    let camera_heights = document.map_defines.heights;
    let load_warnings = document.warnings.clone();
    let content_reads = document.uncovered_reads.clone();
    let branch = options.branch.unwrap_or(document.playback.branch());
    let (first, last) = document
        .playback
        .range(branch)
        .ok_or_else(|| invalid("Unknown branch"))?;
    if options.hour.unwrap() < first || options.playback_cost_end.unwrap() > last {
        return Err(invalid(format!(
            "Requested hours exceed branch coverage {first}..={last}"
        )));
    }
    let recording_meta = fs::metadata(&recording)?;
    let pixel_proof = !schedule.pngs.is_empty();
    let mut report = crate::app::playback_cost::measure(
        config,
        options.clone(),
        document,
        hours,
        schedule,
        monitor,
        &output,
        window,
    )?;
    report["setup_and_teardown_seconds"] =
        json!(setup.elapsed().as_secs_f64() - report["wall_seconds"].as_f64().unwrap());
    report["recording"] = json!({"path":recording,"sha256":hash(&recording)?});
    report["commit"] = json!(option_env!("BATTLE_PLAN_PLAYBACK_COST_COMMIT"));
    report["baseline_main_commit"] = json!(option_env!("BATTLE_PLAN_PLAYBACK_COST_MAIN"));
    report["build_dirty"] = json!(option_env!("BATTLE_PLAN_PLAYBACK_COST_DIRTY").map(|v| v == "true"));
    let exe = std::env::current_exe()?;
    report["executable"] = json!({"path":exe,"sha256":hash(&exe)?});
    report["command"] = json!(
        std::env::args_os()
            .map(|s| s.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
    );
    report["game_directory"] = json!(resolve.game_dir);
    report["user_directory"] = json!(resolve.user_dir);
    report["visual_mods"] = json!(
        options.visual_mods
            .iter()
            .map(|choice| &choice.id)
            .collect::<Vec<_>>()
    );
    report["camera_law"] = json!(camera_law);
    report["camera_heights"] = json!(camera_heights);
    // The map's limitations: the load's warnings and what the content's reads refused.
    let mut limitations = load_warnings;
    content_reads.append_to(&mut limitations);
    report["limitations"] = json!(limitations);
    report["seeded_cache_files"] = json!(seeded);
    report["file_cache"] =
        json!("uncontrolled; isolated application caches, no system cache purge");
    let after = fs::metadata(&recording)?;
    if after.len() != recording_meta.len() || after.modified()? != recording_meta.modified()? {
        return Err(invalid("Recording changed during playback"));
    }
    let mut file = File::create_new(output.join("report.json"))?;
    serde_json::to_writer_pretty(&mut file, &report)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    println!(
        "{} hours; {:.3} game hours/s; report {}",
        hours,
        report["game_hours_per_wall_second"].as_f64().unwrap(),
        output.join("report.json").display()
    );
    // A pixel proof run is not a timing run: it settles models and art within its frames, so its
    // threads' cycles need not account for the process's CPU. Its report keeps the figures.
    if !pixel_proof && report["cpu_reconciliation"]["passed"] != true {
        return Err(io::Error::other(
            "Exclusive CPU scopes do not reconcile within 5%; see report.json",
        ));
    }
    Ok(())
}

/// The process's CPU by its counter and by its threads' cycles agree within 5%, or within the
/// counter's own granularity on a short run: it counts a thread's time in 15.6 ms ticks, and sixteen
/// of them (0.25 s) bound what a run of a few seconds over the process's threads may differ by.
fn reconciles(process_ns: u64, scoped_ns: u64) -> bool {
    let residual = i128::from(process_ns) - i128::from(scoped_ns);
    process_ns > 0 && residual.unsigned_abs() as f64 <= (process_ns as f64 * 0.05).max(0.25e9)
}

pub(crate) fn accounting(
    before: &Value,
    after: &Value,
    process_ns: u64,
    process_cycle_ns: Option<u64>,
    hours: u32,
) -> Value {
    let before = labels(before);
    let mut sums: BTreeMap<String, (u64, u64)> = BTreeMap::new();
    let mut raw = Vec::new();
    for ((role, category, family), ns) in labels(after) {
        let delta = ns.saturating_sub(
            *before
                .get(&(role.clone(), category.clone(), family.clone()))
                .unwrap_or(&0),
        );
        if delta == 0 {
            continue;
        }
        let name = if [
            "inflate",
            "decode",
            "decode_wire",
            "decode_keyframe",
            "decode_shadow",
            "reconstruct",
            "anchors",
        ]
        .contains(&category.as_str())
            || category.starts_with("reader_")
        {
            "decode/hour-build"
        } else if ROWS.iter().any(|(name, _, _)| *name == family)
            || ["decode/hour-build", "upload", "render-submit"].contains(&family.as_str())
        {
            family.as_str()
        } else if role.contains("decode") || role.contains("block reader") {
            "decode/hour-build"
        } else {
            "other"
        };
        let entry = sums.entry(name.to_owned()).or_default();
        entry.0 += delta;
        entry.1 += 1;
        raw.push(json!({"role":role,"category":category,"family":family,"exclusive_cpu_ns":delta}));
    }
    let registered: u64 = sums.values().map(|v| v.0).sum();
    let unscoped = process_cycle_ns.and_then(|all| all.checked_sub(registered));
    if let Some(ns) = unscoped {
        sums.entry("other".to_owned()).or_default().0 += ns;
        raw.push(
            json!({"role":"unregistered process threads","category":"process-cycle remainder",
            "family":"other","exclusive_cpu_ns":ns}),
        );
    }
    let scoped: u64 = sums.values().map(|v| v.0).sum();
    let residual = i128::from(process_ns) - i128::from(scoped);
    let passed = reconciles(process_ns, scoped);
    let mut rows = Vec::new();
    for (name, plan_row, budget) in ROWS {
        rows.push(json!({"name":name,"plan_row":plan_row,"plan_row_budget_us_per_game_hour":budget,
            "exclusive_cpu_us_per_game_hour":sums.get(*name).map(|v| v.0 as f64 / 1000.0 / f64::from(hours))}));
    }
    for name in ["decode/hour-build", "upload", "render-submit", "other"] {
        rows.push(json!({"name":name,"exclusive_cpu_us_per_game_hour":sums.get(name).map(|v| v.0 as f64 / 1000.0 / f64::from(hours))}));
    }
    json!({"exclusive_cpu":rows,"raw_exclusive_scopes":raw,
        "cpu_reconciliation":{"passed":passed,"process_cpu_ns":process_ns,"scope_cpu_ns":scoped,
            "registered_scope_cpu_ns":registered,"process_cycle_cpu_ns":process_cycle_ns,
            "unregistered_thread_cpu_ns":unscoped,
            "process_minus_scopes_ns":residual,"relative_tolerance":0.05,"absolute_tolerance_ns":250_000_000u64},
        "scope_notes":"Exclusive thread-cycle scopes; nested scopes replace their parent. Other is registered thread overhead plus the positive difference between whole-process cycles and registered thread cycles, including unregistered OS/library threads. All cycle costs use one calibrated TSC rate; they are not scaled to force agreement with the independent GetProcessTimes CPU counter, which must reconcile within 5% or 0.25 s (its 15.6 ms ticks over a short run), whichever is larger. Null means no instrumented call, not zero cost. Shared PLAN row budgets are not additive. Accounting overhead is included, not subtracted. Codec covers only the product restore path actually executed; no speculative display products are built."})
}

pub(crate) fn distribution(mut values: Vec<f64>) -> Value {
    if values.is_empty() {
        return json!({"p50":null,"p90":null,"p95":null,"p99":null,"max":null});
    }
    values.sort_by(f64::total_cmp);
    let rank = |p: f64| ((values.len() as f64 * p).ceil() as usize).saturating_sub(1);
    json!({"p50":values[rank(0.5)],"p90":values[rank(0.9)],"p95":values[rank(0.95)],"p99":values[rank(0.99)],"max":values.last()})
}

#[cfg(test)]
mod reconciliation_tests {
    /// A three-second run (P10f's `i3c-late-1`): the process's CPU by its counter, which counts in
    /// 15.6 ms ticks a thread, and by its threads' cycles differ by 187 ms, inside the counter's own
    /// granularity; a long run is held to 5%.
    #[test]
    fn a_short_run_reconciles_within_its_counter_ticks_and_a_long_one_within_five_percent() {
        assert!(super::reconciles(3_078_125_000, 3_265_013_595));
        assert!(!super::reconciles(100_000_000_000, 106_000_000_000));
        assert!(super::reconciles(100_000_000_000, 104_000_000_000));
        assert!(!super::reconciles(0, 0));
    }
}
