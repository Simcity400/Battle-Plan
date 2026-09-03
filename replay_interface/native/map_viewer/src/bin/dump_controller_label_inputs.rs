//! Which controllers of one recorded hour carry the identity XMAP17 labels need.
//!
//! XMAP17 builds a country's on-map label from its ruling ideology and its
//! cosmetic tag, so a controller missing either refuses the frame. This says
//! which controllers those are, and how many of them there are, instead of
//! leaving the first one the refusal happens to name as the whole story.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::ExitCode;

use replay_core::{open_recording, open_recording_game, Leaf, WorldTimeline};

const USAGE: &str =
    "usage: dump_controller_label_inputs <recording-path> (--hour N | --scan) [--game ID]";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("dump_controller_label_inputs: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let mut path: Option<PathBuf> = None;
    let mut game: Option<String> = None;
    let mut hour: Option<i64> = None;
    let mut scan = false;
    let mut argv = std::env::args().skip(1);
    while let Some(arg) = argv.next() {
        match arg.as_str() {
            "--game" => game = Some(argv.next().ok_or("--game needs a value")?),
            "--hour" => {
                let raw = argv.next().ok_or("--hour needs a value")?;
                hour = Some(
                    raw.parse()
                        .map_err(|error| format!("--hour {raw:?} is not an integer: {error}"))?,
                );
            }
            "--scan" => scan = true,
            "--help" | "-h" => return Err(USAGE.to_string()),
            other if other.starts_with("--") => return Err(format!("unknown flag {other:?}")),
            other => path = Some(PathBuf::from(other)),
        }
    }
    let path = path.ok_or(USAGE)?;

    let source = match &game {
        Some(id) => open_recording_game(&path, id).map_err(|error| error.to_string())?,
        None => open_recording(&path).map_err(|error| error.to_string())?,
    };
    let timeline = WorldTimeline::open(source).map_err(|error| error.to_string())?;
    if scan {
        return scan_hours(&timeline);
    }
    let hour = hour.ok_or(USAGE)?;
    let frame = timeline
        .frame(hour)
        .map_err(|error| format!("hour {hour}: {error}"))?;

    let status = frame.control_by_province_status();
    if status != replay_core::ViewStatus::Present {
        return Err(format!("control.controller_by_province is {status:?}"));
    }
    let mut controllers: BTreeMap<String, usize> = BTreeMap::new();
    let mut unmeasured_rows = 0usize;
    for (_, leaf) in frame.control_by_province() {
        match leaf {
            Leaf::Measured(tag) if !tag.trim().is_empty() => {
                *controllers.entry(tag.tag().to_string()).or_default() += 1
            }
            _ => unmeasured_rows += 1,
        }
    }

    println!(
        "hour {hour}: {} controllers, {unmeasured_rows} unmeasured controller rows",
        controllers.len()
    );
    match &frame.diplomacy().subject_autonomy_current {
        Leaf::Measured(rows) => println!("subject_autonomy_current: {} rows", rows.len()),
        other => println!("subject_autonomy_current: {other:?}"),
    }
    let mut without_identity = Vec::new();
    let mut without_ideology = Vec::new();
    let mut without_cosmetic = Vec::new();
    for (tag, provinces) in &controllers {
        let Some(identity) = frame.country_identity(tag) else {
            without_identity.push(format!("{tag} ({provinces} provinces)"));
            continue;
        };
        if !matches!(&identity.ruling_ideology, Leaf::Measured(value) if !value.trim().is_empty()) {
            without_ideology.push(format!(
                "{tag} ({provinces} provinces, ruling_ideology {:?})",
                identity.ruling_ideology
            ));
        }
        if matches!(identity.cosmetic_tag, Leaf::Unobtained) {
            without_cosmetic.push(format!("{tag} ({provinces} provinces)"));
        }
    }
    for (label, rows) in [
        ("no country identity", &without_identity),
        ("no measured ruling ideology", &without_ideology),
        ("no cosmetic tag leaf", &without_cosmetic),
    ] {
        println!("{label}: {} of {}", rows.len(), controllers.len());
        for row in rows {
            println!("    {row}");
        }
    }
    Ok(())
}

/// One line per hour: how many controllers carry a measured ruling ideology.
fn scan_hours(timeline: &WorldTimeline) -> Result<(), String> {
    let mut best: Option<(i64, usize, usize)> = None;
    for hour in timeline.hours().to_vec() {
        let Ok(frame) = timeline.frame(hour) else {
            continue;
        };
        if frame.control_by_province_status() != replay_core::ViewStatus::Present {
            continue;
        }
        let mut controllers: BTreeMap<String, usize> = BTreeMap::new();
        for (_, leaf) in frame.control_by_province() {
            if let Leaf::Measured(tag) = leaf {
                if !tag.trim().is_empty() {
                    *controllers.entry(tag.tag().to_string()).or_default() += 1;
                }
            }
        }
        let measured = controllers
            .keys()
            .filter(|tag| {
                frame.country_identity(tag).is_some_and(|identity| {
                    matches!(&identity.ruling_ideology, Leaf::Measured(v) if !v.trim().is_empty())
                })
            })
            .count();
        let subjects = match &frame.diplomacy().subject_autonomy_current {
            Leaf::Measured(rows) => format!("{} subject rows", rows.len()),
            other => format!("subject_autonomy_current {other:?}"),
        };
        println!(
            "hour {hour}: {measured}/{} controllers carry a ruling ideology, {subjects}",
            controllers.len()
        );
        if best.is_none_or(|(_, m, t)| measured * t > m * controllers.len()) {
            best = Some((hour, measured, controllers.len()));
        }
    }
    match best {
        Some((hour, measured, total)) => {
            println!("best hour {hour}: {measured}/{total}");
            Ok(())
        }
        None => Err("no hour carries a Present controller map".to_string()),
    }
}
