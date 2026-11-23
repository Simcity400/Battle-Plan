
use engine_allocation as allocation;
use replay::campaign::Replay;
use replay_recording::control::ControlRecord;
use replay_data::{BranchId, Hour, cadence::refresh_text, codec::invalid, rows::TableId};
use replay_frame::frame::TableView;
use serde_json::{Value, json};
use std::{collections::BTreeMap, ffi::OsString, io, path::Path};

pub const USAGE: &str = "recorder metrics <campaign.battleplan> <branch> <first-hour> <last-hour>\nrecorder <campaign.battleplan> <branch> <first-hour> <last-hour>";

fn number<T: std::str::FromStr>(value: &OsString) -> Result<T, String> {
    value
        .to_str()
        .ok_or_else(|| USAGE.to_owned())?
        .parse()
        .map_err(|_| USAGE.to_owned())
}

pub fn command(args: Vec<OsString>) -> Result<(), String> {
    let [path, branch, first, last] = args.as_slice() else {
        return Err(USAGE.into());
    };
    let branch = number::<BranchId>(branch)?;
    let first = number::<Hour>(first)?;
    let last = number::<Hour>(last)?;
    let replay = Replay::open_path(Path::new(path)).map_err(|error| error.to_string())?;
    let report = report(&replay, branch, first, last).map_err(|error| error.to_string())?;
    println!("{report}");
    if report["failures"]
        .as_object()
        .is_some_and(|failures| !failures.is_empty())
    {
        Err(format!(
            "requested interval has recorded failures: {}",
            report["failures"]
        ))
    } else {
        Ok(())
    }
}

pub fn percentiles_ns(values: &mut [u64]) -> Option<[u64; 3]> {
    if values.is_empty() {
        return None;
    }
    values.sort_unstable();
    Some([
        values[values.len().div_ceil(2) - 1],
        values[(values.len() * 95).div_ceil(100) - 1],
        *values.last()?,
    ])
}

fn capture_counted(table: TableId) -> bool {
    !table.is_derived()
        && replay_data::format::FormatVersion::CURRENT
            .table_refusal(table)
            .is_none()
}

fn report(replay: &Replay, branch: BranchId, first: Hour, last: Hour) -> io::Result<Value> {
    if first > last {
        return Err(invalid("metrics range must be nonempty and increasing"));
    }
    let selected = replay
        .branches
        .iter()
        .find(|entry| entry.id == branch)
        .ok_or_else(|| invalid(format!("metrics branch {branch} is absent")))?;
    if let Some(partial) = replay.partial() {
        return Err(invalid(format!(
            "metrics refuse a partial recording: {partial:?}"
        )));
    }
    if !replay.recovery_notes.is_empty() {
        return Err(invalid(
            "metrics refuse recovered or damaged recording bytes",
        ));
    }
    if let Some(gap) = replay
        .gaps
        .iter()
        .find(|gap| gap.branch == branch && gap.first_hour <= last && gap.last_hour >= first)
    {
        return Err(invalid(format!(
            "metrics interval intersects a recorded gap: {}",
            gap.reason
        )));
    }
    if let Some(reason) = replay.controls.iter().find_map(|control| match control {
        ControlRecord::GapSummary {
            session, reason, ..
        } if *session == selected.session => Some(reason),
        _ => None,
    }) {
        return Err(invalid(format!(
            "metrics refuse undated frame loss: {reason}"
        )));
    }

    let mut next = i64::from(first);
    let end = i64::from(last) + 1;
    let mut samples = Vec::new();
    let mut total = 0u128;
    let mut pool_bytes_max = 0;
    let mut failures = BTreeMap::<String, u64>::new();
    let blocks: Vec<_> = replay
        .index
        .iter()
        .filter(|index| index.branch == branch && index.first_hour <= last && index.last_hour >= first)
        .collect();
    replay.visit_blocks(
        &blocks,
        &TableId::ALL,
        |_, _, frame| {
            if !(first..=last).contains(&frame.hour) {
                return Ok(true);
            }
            if i64::from(frame.hour) != next {
                return Err(invalid(format!(
                    "metrics expected hour {next}, found {}",
                    frame.hour
                )));
            }
            next += 1;
            allocation::reserve(&mut samples, 1)?;
            samples.push(frame.metrics.total_ns);
            total += u128::from(frame.metrics.total_ns);
            pool_bytes_max = pool_bytes_max.max(frame.metrics.pool_bytes);
            if frame.metrics.dropped_frames != 0 {
                *failures
                    .entry("recorded dropped_frames is nonzero (cumulative counter)".into())
                    .or_default() += 1;
            }
            for table in TableId::ALL {
                if !capture_counted(table) {
                    continue;
                }
                let reason = match &*frame.tables[table.index()] {
                    TableView::Complete(_) | TableView::NotRecorded { .. } => continue,
                    TableView::Truncated { missing, .. } => {
                        format!("truncated: {missing} rows missing")
                    }
                    TableView::Unavailable { reason } => format!("unavailable: {reason}"),
                };
                let key = format!("{table:?} ({}): {reason}", refresh_text(table));
                *failures.entry(key).or_default() += 1;
            }
            Ok(true)
        },
        || Ok(()),
    )?;
    if next != end {
        return Err(invalid(format!("metrics missing requested hour {next}")));
    }
    let [p50, p95, max] = percentiles_ns(&mut samples)
        .ok_or_else(|| invalid("metrics interval has no recorded timing samples"))?;
    Ok(json!({
        "reader_format": replay_data::FORMAT_VERSION,
        "recorded_format": replay.format.version(),
        "branch": branch,
        "hours": samples.len(),
        "observed_first": first,
        "observed_last": last,
        "callback": {
            "samples": samples.len(), "mean_ns": total as f64 / samples.len() as f64,
            "p50_ns": p50, "p95_ns": p95, "max_ns": max,
            "total_ns": total.to_string(),
        },
        "callback_scope": "recorded per-hour callback acquisition wall time; excludes the original native tick",
        "pool_bytes_max": pool_bytes_max,
        "failures": failures,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::replay;
    use replay_capture_spool::spool::Record;
    use replay_data::rows::TableRows;
    use replay_frame::{branches::Gap, frame::TableUpdate};

    fn records() -> Vec<Record> {
        crate::test_support::allocation_records()
    }

    #[test]
    fn exact_metrics_decode_only_the_requested_hours_and_keep_native_nanoseconds() {
        let mut records = records();
        for record in &mut records {
            if let Record::Frame(frame) = record {
                frame.metrics.total_ns = match frame.hour {
                    2 => 999,
                    3 => 10,
                    _ => 30,
                };
                frame.metrics.pool_bytes = if frame.hour == 2 { 999 } else { 70 };
                frame.metrics.table_ns[TableId::CountryPowerDaily.index()] = 999_999;
                if frame.hour == 2 {
                    frame.tables[TableId::CountryPowerDaily.index()] = TableUpdate::Unavailable {
                        reason: "outside requested interval".into(),
                    };
                }
            }
        }
        let output = report(&replay(records), 9, 3, 4).unwrap();
        assert_eq!(output["hours"], 2);
        assert_eq!(output["observed_first"], 3);
        assert_eq!(output["observed_last"], 4);
        assert_eq!(
            output["callback"],
            json!({
                "samples": 2, "mean_ns": 20.0, "p50_ns": 10, "p95_ns": 30,
                "max_ns": 30, "total_ns": "40",
            })
        );
        assert_eq!(output["pool_bytes_max"], 70);
        assert_eq!(output["failures"], json!({}));
    }

    #[test]
    fn a_recording_read_through_its_file_reports_as_one_held_whole() {
        let held = replay(records());
        let path = std::env::temp_dir().join(format!(
            "hoi4-metrics-file-{}-{}.battleplan",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        held.write_atomic(&path).unwrap();
        let read = Replay::open_path(&path).unwrap();
        assert_eq!(
            report(&read, 9, 2, 4).unwrap(),
            report(&held, 9, 2, 4).unwrap()
        );
        drop(read);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn exact_metrics_refuse_holes_partial_prefixes_corruption_and_unknown_formats() {
        let mut hole = records();
        hole.retain(|record| !matches!(record, Record::Frame(frame) if frame.hour == 3));
        assert!(
            report(&replay(hole.clone()), 9, 2, 4)
                .unwrap_err()
                .to_string()
                .contains("expected hour 3")
        );
        hole.insert(
            hole.len() - 1,
            Record::Gap(Gap {
                branch: 9,
                first_hour: 3,
                last_hour: 3,
                reason: "synthetic loss".into(),
            }),
        );
        assert!(
            report(&replay(hole), 9, 2, 4)
                .unwrap_err()
                .to_string()
                .contains("synthetic loss")
        );
        let mut partial = records();
        partial.insert(
            partial.len() - 1,
            Record::Control(ControlRecord::Partial {
                session: 7,
                reason: "synthetic failure".into(),
                failed_hour: None,
            }),
        );
        assert!(report(&replay(partial), 9, 2, 4).is_err());
        let mut undated = records();
        undated.insert(
            undated.len() - 1,
            Record::Control(ControlRecord::GapSummary {
                session: 7,
                dropped_frames: 1,
                reason: "undated synthetic loss".into(),
            }),
        );
        assert!(report(&replay(undated), 9, 2, 4).is_err());
        let good = replay(records());
        assert!(report(&good, 9, 4, 3).is_err());
        assert!(report(&good, 8, 2, 4).is_err());
        assert!(report(&good, 9, 2, 5).is_err());
        let mut duplicate = replay(records());
        duplicate.index.insert(1, duplicate.index[0].clone());
        assert!(report(&duplicate, 9, 2, 4).is_err());
        let mut bytes = good.bytes().to_vec();
        bytes[4..6].copy_from_slice(&(replay_data::FORMAT_VERSION + 1).to_le_bytes());
        assert!(Replay::open(bytes).is_err());
        let mut bytes = good.bytes().to_vec();
        let run = good.recording().run(good.index[0].run).unwrap();
        let page = run.directory().open[good.index[0].page as usize];
        bytes[(page.offset + page.stored / 2) as usize] ^= 1;
        assert!(report(&Replay::open(bytes).unwrap(), 9, 2, 4).is_err());
    }

    #[test]
    fn exact_metrics_preserve_named_failure_coverage_even_on_unscheduled_hours() {
        let mut records = records();
        for record in &mut records {
            if let Record::Frame(frame) = record
                && frame.hour == 3
            {
                frame.keyframe = false;
                frame.metrics.dropped_frames = 1;
                frame.tables[TableId::CountryPowerDaily.index()] = TableUpdate::Unavailable {
                    reason: "native operand refused".into(),
                };
                frame.tables[TableId::Countries.index()] = TableUpdate::Truncated {
                    rows: TableRows::empty(TableId::Countries),
                    missing: 2,
                };
            }
        }
        let output = report(&replay(records), 9, 3, 3).unwrap();
        let failures = output["failures"].as_object().unwrap();
        assert!(failures.keys().any(|key| {
            key.contains("CountryPowerDaily") && key.contains("native operand refused")
        }));
        assert!(
            failures
                .keys()
                .any(|key| key.contains("Countries") && key.contains("2 rows missing"))
        );
        assert!(failures.keys().any(|key| key.contains("dropped_frames")));
        assert_eq!(output["callback"]["samples"], 1);
    }
}
