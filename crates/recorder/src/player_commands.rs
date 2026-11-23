use replay_capture_spool::commands;
use replay_recording::control::ControlRecord;
use serde_json::json;
use std::{ffi::OsString, path::Path};

pub(crate) const USAGE: &str =
    "recorder commands <recording.battleplan|recording.h4col> [--branch <id>] [--events]";

pub(crate) fn command(args: Vec<OsString>) -> Result<(), String> {
    let Some(path) = args.first() else {
        return Err(USAGE.into());
    };
    let mut branch = None;
    let mut events = false;
    let mut rest = args[1..].iter();
    while let Some(arg) = rest.next() {
        if arg == "--events" {
            events = true;
        } else if arg == "--branch" {
            branch = Some(
                rest.next()
                    .and_then(|arg| arg.to_str())
                    .and_then(|arg| arg.parse::<u64>().ok())
                    .ok_or(USAGE)?,
            );
        } else {
            return Err(USAGE.into());
        }
    }
    let replay =
        replay::campaign::commands::open_path(Path::new(path)).map_err(|error| error.to_string())?;
    let branches: Vec<_> = replay
        .branches
        .iter()
        .filter(|item| branch.is_none_or(|id| item.id == id))
        .map(|item| item.id)
        .collect();
    if branch.is_some() && branches.is_empty() {
        return Err("command branch is absent".into());
    }
    for branch in branches {
        let analysis = commands::analyze(&replay.sessions, &replay.controls, branch)
            .map_err(|error| error.to_string())?;
        println!(
            "{}",
            json!({ "branch": branch, "metric": "player-issued synchronized gameplay orders per observed unpaused real minute", "analysis": analysis, "origin_scope": "APM counts player-issued gameplay orders during unpaused play. Paused orders are separate counts and paused time is excluded. AI-country orders and automatic maintenance are excluded. Autonomous execution of a battle plan adds no player orders. Exact APM requires complete provenance; confirmed_orders_per_minute remains available with ambiguous kinds listed separately. Missing capture coverage withholds both rates. Local camera and keyboard activity are not replicated." })
        );
        if events {
            for control in &replay.controls {
                if let ControlRecord::PlayerCommands { session, packet } = control {
                    if packet.branch != branch {
                        continue;
                    }
                    let batch = packet.unpack().map_err(|error| error.to_string())?;
                    println!(
                        "{}",
                        json!({ "session": session, "branch": branch, "world": packet.world, "capture": packet.capture, "packet": packet.ordinal, "lost": packet.lost, "kinds": batch.kinds, "actors": batch.actors, "departures": batch.departures, "events": batch.events, "coverage": batch.spans })
                    );
                }
            }
        }
    }
    Ok(())
}
