use super::{CAPTURES, SEED_CACHE, copy_tree, hash, invalid, lock};
use crate::Options;
use serde_json::json;
use std::{
    ffi::OsString,
    fs::{self, File},
    io::{self, Write},
    path::Path,
    time::Duration,
};

fn number<T: std::str::FromStr>(value: Option<&OsString>, flag: &str) -> io::Result<T> {
    value
        .and_then(|value| value.to_str())
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| invalid(format!("{flag} needs a number")))
}

pub(crate) fn run(args: &[OsString]) -> io::Result<()> {
    let mut translated = Vec::new();
    let mut countries = Vec::new();
    let mut run = crate::app::playback_cost::viewpoint::Switches {
        countries: Vec::new(),
        rounds: 2,
        play_switches: 2,
        play_speed: 24.0,
        hour_steps: 4,
        refresh_hz: 60.0,
        settle_frames: 30,
        timeout: Duration::from_secs(120),
    };
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--viewpoint-switch") => translated.push(OsString::from("--dump")),
            Some("--countries") => {
                let list = args.next().and_then(|value| value.to_str()).ok_or_else(|| invalid("--countries needs TAG,TAG,..."))?;
                countries.extend(list.split(',').map(str::trim).filter(|tag| !tag.is_empty()).map(str::to_owned));
            }
            Some("--rounds") => run.rounds = number(args.next(), "--rounds")?,
            Some("--play-switches") => run.play_switches = number(args.next(), "--play-switches")?,
            Some("--play-speed") => run.play_speed = number(args.next(), "--play-speed")?,
            Some("--hour-steps") => run.hour_steps = number(args.next(), "--hour-steps")?,
            Some("--refresh-hz") => run.refresh_hz = number(args.next(), "--refresh-hz")?,
            Some("--settle-frames") => run.settle_frames = number(args.next(), "--settle-frames")?,
            Some("--timeout") => run.timeout = Duration::from_secs(number(args.next(), "--timeout")?),
            _ => translated.push(arg.clone()),
        }
    }
    if countries.len() < 2 {
        return Err(invalid("--countries needs at least two tags to switch between"));
    }
    run.refresh_hz = super::refresh_rate(run.refresh_hz)?;
    if !(run.play_speed.is_finite() && run.play_speed > 0.0) {
        return Err(invalid("--play-speed needs a positive rate"));
    }
    run.countries = countries;
    let options = Options::parse(translated.into_iter())?;
    if options.hour.is_none() {
        return Err(invalid("--viewpoint-switch needs --hour H, the hour the switches are made at"));
    }
    let recording = fs::canonicalize(options.replay.as_ref().ok_or_else(|| invalid("--viewpoint-switch needs a recording"))?)?;
    let output = options.out.as_ref().ok_or_else(|| invalid("--viewpoint-switch needs --out <new directory>"))?;
    let parent = fs::canonicalize(output.parent().ok_or_else(|| invalid("Output needs a parent"))?)?;
    let captures = std::env::var_os(CAPTURES).ok_or_else(|| invalid("Set BATTLEPLAN_CAPTURES to the folder --out must be under"))?;
    if !parent.starts_with(fs::canonicalize(captures)?) {
        return Err(invalid("--out must be a new directory under BATTLEPLAN_CAPTURES"));
    }
    fs::create_dir(output)?;
    let output = fs::canonicalize(output)?;
    let _locks = lock(&output)?;
    let mut config = dashboard_core::config::Config::load()?;
    if let Some(path) = &options.game_dir {
        config.game_dir = path.clone();
    }
    if let Some(path) = &options.user_dir {
        config.user_dir = path.clone();
    }
    let local = output.join("cache/local");
    let roaming = output.join("cache/roaming");
    fs::create_dir_all(&local)?;
    fs::create_dir_all(&roaming)?;
    let seeded = match std::env::var_os(SEED_CACHE) {
        Some(seed) => Some(copy_tree(Path::new(&seed), &output.join("cache"))?),
        None => None,
    };
    unsafe {
        std::env::set_var("LOCALAPPDATA", local);
        std::env::set_var("APPDATA", roaming);
    }
    replay_frame::cpu::enable();
    let before = fs::metadata(&recording)?;
    let mut report = crate::app::playback_cost::viewpoint::measure(config, options, recording.clone(), &run)?;
    let after = fs::metadata(&recording)?;
    if after.len() != before.len() || after.modified()? != before.modified()? {
        return Err(invalid("Recording changed during the run"));
    }
    report["recording"] = json!({"path": recording, "sha256": hash(&recording)?});
    report["commit"] = json!(option_env!("BATTLEPLAN_PLAYBACK_COST_COMMIT"));
    let exe = std::env::current_exe()?;
    report["executable"] = json!({"path": exe, "sha256": hash(&exe)?});
    report["command"] = json!(std::env::args_os().map(|arg| arg.to_string_lossy().into_owned()).collect::<Vec<_>>());
    report["seeded_cache_files"] = json!(seeded);
    let mut file = File::create_new(output.join("report.json"))?;
    serde_json::to_writer_pretty(&mut file, &report)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    println!("report {}", output.join("report.json").display());
    Ok(())
}
