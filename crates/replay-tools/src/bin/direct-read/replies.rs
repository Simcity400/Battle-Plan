use super::{Options, chart_column, eight_keys, grid, invalid, sample_hours, views};
use replay::campaign::{Replay, SeekResult};
use replay_recording::{columnar_frames::Recording, summaries};
use replay_lanes::lanes;
use replay_columnar::summary::{self, Shape};
use replay_data::rows::TableId;
use serde_json::{Value, json};
use std::{fmt::Write as _, io, path::Path};

struct Reply {
    kind: &'static str,
    what: String,
    count: usize,
    text: String,
}

impl Reply {
    fn json(&self) -> Value {
        json!({ "kind": self.kind, "what": self.what, "count": self.count, "digest": blake3::hash(self.text.as_bytes()).to_hex()[..32].to_string() })
    }
}

fn numbers(snapshot: &lanes::Snapshot, columns: &[usize]) -> Vec<(u128, Vec<Option<i64>>)> {
    let view = snapshot.view();
    let mut out: Vec<(u128, Vec<Option<i64>>)> = view.rows().filter_map(|slot| Some((view.key(slot)?, columns.iter().map(|&column| view.number(column, slot)).collect()))).collect();
    out.sort_unstable_by_key(|(key, _)| *key);
    out
}

fn frame_reply(replay: &Replay, table: TableId, branch: u64, hour: i32) -> io::Result<Reply> {
    let text = match replay.seek_selected(branch, hour, Some(&[table]))? {
        SeekResult::Frame(frame) => format!("{:?}", frame.tables[table.index()]),
        SeekResult::Gap { reason, .. } => format!("gap {reason}"),
    };
    Ok(Reply { kind: "frame", what: format!("{table:?} {hour}"), count: text.len(), text })
}

fn outcome<T>(read: io::Result<T>, text: impl FnOnce(&T) -> String) -> (usize, String) {
    match read {
        Ok(held) => {
            let shown = text(&held);
            (shown.len(), shown)
        }
        Err(error) => (0, format!("refused: {error}")),
    }
}

fn chart_text(chart: &lanes::Chart) -> String {
    let mut text = format!("{} {} {} {}\n", chart.level, chart.bin_hours, chart.first_bin_hour, chart.bins);
    for (at, key) in chart.keys.iter().enumerate() {
        let _ = writeln!(text, "{key:x} {:?} {:?} {:?}", chart.values(at), chart.totals(at), chart.life(at));
    }
    text
}

const CHARTED: [TableId; 3] = [TableId::Countries, TableId::CountryStability, TableId::ArmyTotals];

pub fn all(path: &str, options: &Options) -> io::Result<Value> {
    let samples: usize = options.number("samples", 60)?;
    let seed = options.number("seed", 0x9e37_79b9_7f4a_7c15_u64)?;
    let replay = Replay::open_path(Path::new(path))?;
    let recording: &Recording = replay.recording();
    let held = recording.run(0)?;
    let (branch, first_hour, ordinals) = (held.branch, held.first_hour, held.ordinals);
    let hours = sample_hours(recording, samples, seed)?;
    let mut replies = Vec::new();
    let numeric: Vec<usize> = (0..TableId::Countries.field_count())
        .filter(|&field| !matches!(replay_recording::schema::schema().map(|schema| schema[TableId::Countries.index()][field]), Ok(replay_columnar::state::Kind::Bytes | replay_columnar::state::Kind::U32List | replay_columnar::state::Kind::U64List | replay_columnar::state::Kind::Opaque) | Err(_)))
        .collect();
    for &(run, ordinal) in &hours {
        let hour = recording.run(run)?.first_hour + ordinal as i32;
        replies.push(frame_reply(&replay, TableId::Countries, branch, hour)?);
        let snapshot = lanes::at(recording, run, TableId::Countries, hour, None)?;
        let cells = numbers(&snapshot, &numeric);
        replies.push(Reply { kind: "at", what: format!("Countries {hour}"), count: cells.len(), text: format!("{:?} {:?} {cells:?}", snapshot.status, snapshot.sampled_at) });
    }
    for &(run, ordinal) in hours.iter().take(20) {
        let hour = recording.run(run)?.first_hour + ordinal as i32;
        replies.push(frame_reply(&replay, TableId::DivisionStatus, branch, hour)?);
    }
    let mid = first_hour + (ordinals / 2) as i32;
    for table in CHARTED {
        let source = chart_column(table, options)?;
        let eight = eight_keys(recording, table)?;
        let lives = lanes::lifetimes(recording, 0, table)?;
        replies.push(Reply { kind: "lifetimes", what: format!("{table:?}"), count: lives.len(), text: format!("{lives:?}") });
        if let summary::Source::Cell(column) = summaries::registry()?.iter().find(|(held, _)| *held == table.index()).and_then(|(_, shape)| match shape {
            Shape::Values(columns) => columns.iter().find(|column| column.source.id() == source).map(|column| column.source),
            _ => None,
        }).ok_or_else(|| invalid("no such summarized column"))? {
            let to = (mid + 2000).min(first_hour + ordinals as i32 - 1);
            let (count, text) = outcome(lanes::runs(recording, 0, table, column, mid, to), |runs| format!("{runs:?}"));
            replies.push(Reply { kind: "runs", what: format!("{table:?} {column} {mid}..={to}"), count, text });
            let mut found = Vec::new();
            for at in 0..20u64 {
                let key = eight[(at as usize) % eight.len().max(1)];
                let hour = first_hour + ((at * 7919 + 13) % u64::from(ordinals - 1)) as i32;
                found.push((key, hour, outcome(lanes::next(recording, 0, table, column, key, hour), |found| format!("{found:?}")).1));
            }
            replies.push(Reply { kind: "next", what: format!("{table:?} {column}"), count: found.len(), text: format!("{found:?}") });
        }
        for (name, from, to) in views(i64::from(first_hour), ordinals) {
            let (level, bin_hours) = grid((i64::from(to) + 1 - i64::from(from)) / 1000);
            for (keys, shape) in [(Some(eight.as_slice()), "eight"), (None, "all")] {
                let (count, text) = outcome(lanes::chart_at(recording, 0, table, source, keys, from, to, level, bin_hours), chart_text);
                replies.push(Reply { kind: "chart", what: format!("{table:?} {source} {name} {shape}"), count, text });
            }
            let (count, text) = outcome(lanes::top(recording, 0, table, source, from, to, 1000, 10, lanes::By::Last), |top| format!("{top:?}"));
            replies.push(Reply { kind: "top", what: format!("{table:?} {source} {name}"), count, text });
        }
        let end = i64::from(first_hour) + i64::from(ordinals);
        let level = if end - i64::from(first_hour) >= summary::bucket_hours(1) { 1 } else { 0 };
        let size = summary::bucket_hours(level);
        let whole = (end - i64::from(first_hour) + 2 * size).div_euclid(size) * size;
        let (count, text) = outcome(lanes::chart_at(recording, 0, table, source, None, first_hour, (end - 1) as i32, level, whole), chart_text);
        replies.push(Reply { kind: "rank", what: format!("{table:?} {source}"), count, text });
    }
    for (index, shape) in summaries::registry()?.iter() {
        if !matches!(shape, Shape::Log(_)) {
            continue;
        }
        let table = TableId::ALL[*index];
        let (count, text) = outcome(lanes::log(recording, 0, table, first_hour, first_hour + ordinals as i32 - 1, 24), |counts| format!("{counts:?}"));
        replies.push(Reply { kind: "log", what: format!("{table:?}"), count, text });
    }
    let whole = blake3::hash(replies.iter().map(|reply| format!("{} {} {}\n", reply.kind, reply.what, reply.json()["digest"])).collect::<String>().as_bytes()).to_hex()[..32].to_string();
    Ok(json!({ "command": "replies", "samples": samples, "seed": seed, "count": replies.len(), "digest": whole, "replies": replies.iter().map(Reply::json).collect::<Vec<_>>() }))
}
