//! `viewer --visual-toggle <replay> --hour H --out <dir> [--mod-a NAME]... [--mod-b NAME]...
//! [--toggles N] [--refresh-hz HZ] [--settle-frames N] [--timeout S] [--no-fresh]`: opens the
//! recording with visual-mod set A, toggles to set B and back N times through the app's own path
//! ([`crate::app::playback_cost::toggle`]), and writes each toggle's stages, counts and pictures
//! to `<dir>/report.json` and `<dir>/*.png`. A mod is named by its descriptor name as the
//! settings list it, or by its id (`workshop/<item>`, `mod/<file>`); no mod is a set of none.
use super::{CAPTURES, SEED_CACHE, copy_tree, hash, invalid, lock};
use crate::Options;
use installed_content_core::content::visual::{Choice, InstalledMod};
use serde_json::json;
use std::{
    ffi::OsString,
    fs::{self, File},
    io::{self, Write},
    path::Path,
    sync::atomic::AtomicBool,
    time::Duration,
};

fn number<T: std::str::FromStr>(value: Option<&OsString>, flag: &str) -> io::Result<T> {
    value
        .and_then(|value| value.to_str())
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| invalid(format!("{flag} needs a number")))
}

/// The installed mod `wanted` names, by id or by its name without regard to case.
fn choose(installed: &[InstalledMod], wanted: &str) -> io::Result<Choice> {
    let matches: Vec<_> = installed
        .iter()
        .filter(|candidate| {
            candidate.id == wanted || candidate.name.trim().eq_ignore_ascii_case(wanted.trim())
        })
        .collect();
    match matches.as_slice() {
        [found] => Ok(Choice {
            id: found.id.clone(),
            name: found.name.clone(),
        }),
        [] => Err(invalid(format!(
            "No installed mod is named {wanted:?}; installed: {}",
            installed
                .iter()
                .map(|candidate| format!("{:?} ({})", candidate.name, candidate.id))
                .collect::<Vec<_>>()
                .join(", ")
        ))),
        several => Err(invalid(format!(
            "{} installed mods are named {wanted:?}: name one by its id ({})",
            several.len(),
            several
                .iter()
                .map(|candidate| candidate.id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ))),
    }
}

pub(crate) fn run(args: &[OsString]) -> io::Result<()> {
    let mut translated = Vec::new();
    let mut wanted: [Vec<String>; 2] = [Vec::new(), Vec::new()];
    let mut toggles = 4u32;
    let mut refresh_hz = 60.0f64;
    let mut settle_frames = 60u32;
    let mut timeout = 300u64;
    let mut fresh = true;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--visual-toggle") => translated.push(OsString::from("--dump")),
            Some(flag @ ("--mod-a" | "--mod-b")) => {
                let name = args
                    .next()
                    .and_then(|value| value.to_str())
                    .filter(|value| !value.trim().is_empty())
                    .ok_or_else(|| invalid(format!("{flag} needs a mod's name or id")))?;
                wanted[usize::from(flag == "--mod-b")].push(name.to_owned());
            }
            Some("--toggles") => toggles = number(args.next(), "--toggles")?,
            Some("--refresh-hz") => refresh_hz = number(args.next(), "--refresh-hz")?,
            Some("--settle-frames") => settle_frames = number(args.next(), "--settle-frames")?,
            Some("--timeout") => timeout = number(args.next(), "--timeout")?,
            Some("--no-fresh") => fresh = false,
            Some("--visual-mod") => {
                return Err(invalid("--visual-toggle takes its sets from --mod-a and --mod-b"));
            }
            _ => translated.push(arg.clone()),
        }
    }
    if !(refresh_hz.is_finite() && refresh_hz > 0.0) {
        return Err(invalid("--refresh-hz needs a positive rate"));
    }
    if wanted[0] == wanted[1] {
        return Err(invalid("--mod-a and --mod-b name the same set: a toggle changes nothing"));
    }
    let options = Options::parse(translated.into_iter())?;
    if options.hour.is_none() {
        return Err(invalid("--visual-toggle needs --hour H, the hour the map holds"));
    }
    let recording = fs::canonicalize(
        options
            .replay
            .as_ref()
            .ok_or_else(|| invalid("--visual-toggle needs a recording"))?,
    )?;
    let output = options
        .out
        .as_ref()
        .ok_or_else(|| invalid("--visual-toggle needs --out <new directory>"))?;
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
    let resolve = config.resolve_options()?;
    let installed = installed_content_core::content::visual::installed(&resolve, &AtomicBool::new(false))?;
    let sets = [
        wanted[0].iter().map(|name| choose(&installed, name)).collect::<io::Result<Vec<_>>>()?,
        wanted[1].iter().map(|name| choose(&installed, name)).collect::<io::Result<Vec<_>>>()?,
    ];
    let local = output.join("cache/local");
    let roaming = output.join("cache/roaming");
    fs::create_dir_all(&local)?;
    fs::create_dir_all(&roaming)?;
    // A finished run's caches (compiled shaders, content hashes), so a run starts from them.
    let seeded = match std::env::var_os(SEED_CACHE) {
        Some(seed) => Some(copy_tree(Path::new(&seed), &output.join("cache"))?),
        None => None,
    };
    // Isolation is set before any viewer worker exists: the map settings, the content caches and
    // the shader cache all live under `--out`.
    unsafe {
        std::env::set_var("LOCALAPPDATA", local);
        std::env::set_var("APPDATA", roaming);
    }
    replay_frame::cpu::enable();
    let run = crate::app::playback_cost::toggle::Toggle {
        sets,
        toggles,
        refresh_hz,
        settle_frames,
        timeout: Duration::from_secs(timeout),
        fresh,
    };
    let recording_meta = fs::metadata(&recording)?;
    // The run holds several maps' frames at once, on the map thread's stack.
    let mut report =
        crate::app::playback_cost::toggle::measure(config, options.clone(), recording.clone(), run, &output)?;
    let after = fs::metadata(&recording)?;
    if after.len() != recording_meta.len() || after.modified()? != recording_meta.modified()? {
        return Err(invalid("Recording changed during the run"));
    }
    report["recording"] = json!({"path": recording, "sha256": hash(&recording)?});
    report["commit"] = json!(option_env!("BATTLE_PLAN_PLAYBACK_COST_COMMIT"));
    let exe = std::env::current_exe()?;
    report["executable"] = json!({"path": exe, "sha256": hash(&exe)?});
    report["command"] = json!(
        std::env::args_os()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
    );
    report["game_directory"] = json!(resolve.game_dir);
    report["user_directory"] = json!(resolve.user_dir);
    report["seeded_cache_files"] = json!(seeded);
    let mut file = File::create_new(output.join("report.json"))?;
    serde_json::to_writer_pretty(&mut file, &report)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    println!("report {}", output.join("report.json").display());
    Ok(())
}
