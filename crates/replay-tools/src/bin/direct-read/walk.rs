use crate::{Options, open, table_named};
use crate::meter::{Meter, round};
use replay_recording::columnar_frames::{Frames, Recording, Spent};
use replay_columnar::unit::BLOCK;
use replay_data::rows::TableId;
use serde_json::{Value, json};
use std::io;

pub fn walk(path: &str, options: &Options) -> io::Result<Value> {
    let tables = match options.get("tables") {
        Some(names) => names.split(',').map(table_named).collect::<io::Result<Vec<_>>>()?,
        None => TableId::ALL.to_vec(),
    };
    let limit: u64 = options.number("hours", u64::MAX)?;
    let hold = options.get("hold").is_some();
    let recording = open(path)?;
    let rebuild = options.get("rebuild").is_some();
    let mut frames = Frames::new(&recording, &tables)?.timed();
    if rebuild {
        frames = frames.rebuilding();
    }
    let mut check = match options.get("check") {
        Some(_) => Some(Frames::new(&recording, &tables)?.rebuilding()),
        None => None,
    };
    let mut held = None;
    let (mut hours, mut walked_ms, mut differences) = (0_u64, 0.0, Vec::new());
    'runs: for run in 0..recording.run_count() {
        let at = recording.run(run)?;
        for ordinal in 0..at.ordinals {
            if hours >= limit {
                break 'runs;
            }
            let meter = Meter::start();
            frames.go_to(run, ordinal)?;
            if ordinal % BLOCK == 0 {
                frames.keyframe();
            }
            let made = frames.frame_recorded(at.branch)?;
            walked_ms += meter.stop().wall_ms;
            if let Some(check) = check.as_mut() {
                compare(&recording, check, &tables, (run, ordinal), &made, &mut differences)?;
            }
            if hold {
                held = Some(made);
            }
            hours += 1;
        }
    }
    drop(held);
    let mut spent: Vec<(TableId, Spent)> = frames.spent().collect();
    spent.sort_by_key(|(table, spent)| (std::cmp::Reverse(spent.move_ns + spent.view_ns + spent.update_ns), *table));
    let ms = |ns: u64| round(ns as f64 / 1e6);
    let total = |pick: fn(&Spent) -> u64| spent.iter().map(|(_, spent)| pick(spent)).sum::<u64>();
    let per_table: Vec<Value> = spent
        .iter()
        .map(|(table, spent)| {
            json!({ "table": format!("{table:?}"), "ms": ms(spent.move_ns + spent.view_ns + spent.update_ns),
                "move_ms": ms(spent.move_ns), "view_ms": ms(spent.view_ns), "update_ms": ms(spent.update_ns),
                "patched": spent.patched, "in_place": spent.in_place, "rebuilt": spent.rebuilt, "snapshots_patched": spent.snapshots_patched })
        })
        .collect();
    Ok(json!({
        "command": "walk",
        "tables": tables.len(),
        "hours": hours,
        "held": hold,
        "rebuilt_every_view": rebuild,
        "wall_ms": round(walked_ms),
        "wall_ms_per_hour": round(walked_ms / hours.max(1) as f64),
        "move_ms": ms(total(|spent| spent.move_ns)),
        "view_ms": ms(total(|spent| spent.view_ns)),
        "update_ms": ms(total(|spent| spent.update_ns)),
        "patched": total(|spent| spent.patched),
        "in_place": total(|spent| spent.in_place),
        "rebuilt": total(|spent| spent.rebuilt),
        "checked": check.is_some(),
        "differences": differences.len(),
        "first_differences": differences.iter().take(20).collect::<Vec<_>>(),
        "by_table_heaviest_first": per_table,
    }))
}

fn compare(
    recording: &Recording,
    check: &mut Frames<&Recording>,
    tables: &[TableId],
    (run, ordinal): (u32, u32),
    (frame, facts): &(replay_frame::frame::DecodedFrame, replay_frame::frame::RecordedFrame),
    differences: &mut Vec<Value>,
) -> io::Result<()> {
    check.go_to(run, ordinal)?;
    if ordinal % BLOCK == 0 {
        check.keyframe();
    }
    let (want, want_facts) = check.frame_recorded(recording.run(run)?.branch)?;
    if facts.keyframe != want_facts.keyframe {
        differences.push(json!({ "run": run, "ordinal": ordinal, "what": "keyframe" }));
    }
    for &table in tables {
        let index = table.index();
        let what = if frame.tables[index] != want.tables[index] {
            "view"
        } else if frame.tables[index].sampled_at() != want.tables[index].sampled_at() {
            "sample hour"
        } else if facts.tables[index] != want_facts.tables[index] {
            "update"
        } else {
            continue;
        };
        differences.push(json!({ "run": run, "ordinal": ordinal, "hour": frame.hour, "table": format!("{table:?}"), "what": what }));
    }
    Ok(())
}
