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

const CAPTURES: &str = r"captures";
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

pub(crate) fn dispatch(args: &[OsString]) -> Option<io::Result<()>> {
    args.iter()
        .any(|arg| arg == "--playback-cost")
        .then(|| run(args))
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

fn run(args: &[OsString]) -> io::Result<()> {
    if std::env::var_os("BATTLE_PLAN_VIEWER_PARENT").or_else(|| std::env::var_os("HOI4_VIEWER_PARENT")).is_some()
        || viewer_foundation::profile_writer::profiling()
        || crate::hours::page_grid()
    {
        return Err(invalid(
            "--playback-cost requires standalone, unprofiled, consecutive-hour playback",
        ));
    }
    let mut translated = Vec::new();
    let mut hours = None;
    let mut pointer = None;
    let mut schedule = crate::app::playback_cost::Schedule {
        step: 1,
        pan: None,
        pngs: Vec::new(),
    };
    let mut visual_mods = Vec::new();
    let mut from = false;
    let mut speed = false;
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
            Some("--visual-mod") => {
                let id = args
                    .next()
                    .and_then(|v| v.to_str())
                    .filter(|v| !v.is_empty())
                    .ok_or_else(|| invalid("--visual-mod needs a mod id, such as mod/name.mod"))?;
                visual_mods.push(installed_content_core::content::visual::Choice {
                    id: id.to_owned(),
                    name: id.to_owned(),
                });
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
    let hours = hours.ok_or_else(|| invalid("--playback-cost requires --hours N"))?;
    if !from || !speed {
        return Err(invalid(
            "--playback-cost requires --from-hour H and --speed max",
        ));
    }
    let mut options = Options::parse(translated.into_iter())?;
    options.profile_pointer = pointer;
    if options.hour_fraction != 0.0 {
        return Err(invalid("--from-hour must be a whole game hour"));
    }
    let span = match schedule.pan {
        Some(_) => 0,
        None => (i64::from(hours) - 1) * i64::from(schedule.step),
    };
    let end = i64::from(options.hour.unwrap()) + span;
    options.playback_cost_end =
        Some(i32::try_from(end).map_err(|_| invalid("Hour range overflows"))?);
    let recording = fs::canonicalize(options.replay.as_ref().unwrap())?;
    let output = options.out.as_ref().unwrap();
    let parent = fs::canonicalize(
        output
            .parent()
            .ok_or_else(|| invalid("Output needs a parent"))?,
    )?;
    if !parent.starts_with(fs::canonicalize(CAPTURES)?) {
        return Err(invalid(
            "--out must be a new directory under captures",
        ));
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
        (!visual_mods.is_empty()).then_some(visual_mods.as_slice()),
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
    let mut report = crate::app::playback_cost::measure(
        config,
        options.clone(),
        document,
        hours,
        schedule,
        monitor,
        &output,
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
        visual_mods
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
    if report["cpu_reconciliation"]["passed"] != true {
        return Err(io::Error::other(
            "Exclusive CPU scopes do not reconcile within 5%; see report.json",
        ));
    }
    Ok(())
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
    let passed = process_ns > 0 && residual.unsigned_abs() as f64 <= process_ns as f64 * 0.05;
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
            "process_minus_scopes_ns":residual,"relative_tolerance":0.05},
        "scope_notes":"Exclusive thread-cycle scopes; nested scopes replace their parent. Other is registered thread overhead plus the positive difference between whole-process cycles and registered thread cycles, including unregistered OS/library threads. All cycle costs use one calibrated TSC rate; they are not scaled to force agreement with the independent GetProcessTimes CPU counter, which must reconcile within 5%. Null means no instrumented call, not zero cost. Shared PLAN row budgets are not additive. Accounting overhead is included, not subtracted. Codec covers only the product restore path actually executed; no speculative display products are built."})
}
