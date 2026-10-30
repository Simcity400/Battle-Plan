//! Measures a native preparation of a recording as production prepares it
//! ([`dashboard::prepared::driver::native`]): every family with a native backend, or those
//! `--family` names, prepared from the native recording into a scratch store under `--output`.
//! Reports what it cost per game hour beside the goal it is judged on: each family's operators,
//! ordered emit and publication, the walks every lane shares, the slowest serial lane and the
//! peak resident bytes, with every status row that failed. `served-check` checks the replies.
//! `columnar-family-check --recording <r.battleplan> --columnar <c.h4col> --output <new folder>
//!   [--family F[,F...]|ported] [--threads N] [--scopes]`
//! Game content comes from BATTLE_PLAN_GAME_DIR and BATTLE_PLAN_USER_DIR, or none when neither is set.
//! `--scopes` also reports the CPU each accounting scope charged.

use dashboard::{config::ContentOptions, prepared::driver::native};
use replay::columnar_frames::{Limits, Recording};
use serde_json::json;
use std::{
    io,
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};

#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

const USAGE: &str = "columnar-family-check --recording R --columnar C --output NEW \
                     [--family F[,F...]|ported] [--threads N] [--scopes]";

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, what.into())
}

/// The content the families read, cached under `output`; none when neither folder is set.
fn content(output: &Path) -> io::Result<Option<ContentOptions>> {
    let (game_dir, user_dir) = match (
        std::env::var_os("BATTLE_PLAN_GAME_DIR").or_else(|| std::env::var_os("HOI4_GAME_DIR")),
        std::env::var_os("BATTLE_PLAN_USER_DIR").or_else(|| std::env::var_os("HOI4_USER_DIR")),
    ) {
        (Some(game), Some(user)) => (PathBuf::from(game), PathBuf::from(user)),
        (None, None) => return Ok(None),
        _ => {
            return Err(invalid(
                "Set both BATTLE_PLAN_GAME_DIR and BATTLE_PLAN_USER_DIR, or neither",
            ));
        }
    };
    engine_executable::executable_tables::initialize(&game_dir)?;
    let cache_dir = output.join("content");
    std::fs::create_dir_all(&cache_dir)?;
    Ok(Some(ContentOptions {
        options: installed_content_core::ResolveOptions {
            game_dir,
            user_dir,
            cache_dir,
        },
        allow_unverified: false,
        presentation: Default::default(),
    }))
}

fn run() -> io::Result<()> {
    let (mut recording, mut columnar, mut output) = (None, None, None);
    let mut families: Option<String> = None;
    let mut threads = std::thread::available_parallelism().map_or(1, usize::from);
    let mut scopes = false;
    let mut args = std::env::args_os().skip(1);
    while let Some(flag) = args.next() {
        if flag == "--scopes" {
            scopes = true;
            continue;
        }
        let value = args.next().ok_or_else(|| invalid(USAGE))?;
        match flag.to_str() {
            Some("--recording") => recording = Some(PathBuf::from(value)),
            Some("--columnar") => columnar = Some(PathBuf::from(value)),
            Some("--output") => output = Some(PathBuf::from(value)),
            Some("--family") => families = Some(value.to_string_lossy().into_owned()),
            Some("--threads") => {
                threads = value
                    .to_str()
                    .and_then(|text| text.parse().ok())
                    .filter(|&count| count > 0)
                    .ok_or_else(|| invalid(USAGE))?
            }
            _ => return Err(invalid(USAGE)),
        }
    }
    let (Some(recording), Some(columnar), Some(output)) = (recording, columnar, output) else {
        return Err(invalid(USAGE));
    };
    let named: Option<Vec<&str>> = families
        .as_deref()
        .filter(|list| *list != "ported")
        .map(|list| list.split(',').collect());
    std::fs::create_dir(&output)?;
    let local = output.join("local");
    std::fs::create_dir_all(&local)?;
    // Set before any other thread starts: the scratch store lives under it.
    unsafe { std::env::set_var("LOCALAPPDATA", &local) };
    if scopes {
        replay_frame::cpu::enable();
    }
    let content = content(&output)?;
    let source = dashboard::Source::open(
        dashboard::SourceKey {
            source_id: "columnar-family-check".into(),
            revision: dashboard::protocol::replay_revision(&recording)?,
        },
        recording.clone(),
        &AtomicBool::new(false),
    )?;
    let file = Recording::open_path(&columnar, Limits::default())?;
    let damage: Vec<String> = file.damage().iter().map(ToString::to_string).collect();
    let (accepted, measured) = native::measure(
        &source,
        content.as_ref(),
        file,
        named.as_deref(),
        threads,
        &AtomicBool::new(false),
    )?;
    let failed: Vec<_> = accepted
        .sections
        .iter()
        .filter(|section| section.error.is_some())
        .collect();
    let report = json!({
        "recording": recording,
        "columnar": columnar,
        "damage": damage,
        "measured": measured,
        "failed": failed,
        "scopes": scopes.then(replay_frame::cpu::snapshot),
    });
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("columnar-family-check: {error}");
        std::process::exit(1);
    }
}
