
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::Instant;



#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HourSelection {
    Sample,
    All,
}

#[derive(serde::Serialize)]
struct HourReport {
    hour_index: i64,
    coverage_warnings: Vec<String>,
    map_authority_error: Option<String>,
    admitted_inputs: Vec<String>,
    gap_inputs: Vec<GapRow>,
    recorder_domain_problems: Vec<RecorderDomainProblem>,
    wing_anchor_refusals: Vec<WingAnchorRefusalCensus>,
    frame_decode_ms: f64,
}

/// Whether the recorder delivered this hour's carrier at all, and under which
/// of its own typed causes it did not.
#[derive(serde::Serialize)]
struct RecorderDomainProblem {
    domain_id: i64,
    problem: String,
}

/// The wings one placement refusal accounts for, and the bases they name; the
/// ledger row itself refuses the whole hour for any one of them.
#[derive(serde::Serialize)]
struct WingAnchorRefusalCensus {
    cause: String,
    wings: usize,
    air_base_ids: Vec<String>,
}

#[derive(serde::Serialize)]
struct GapRow {
    input_id: String,
    disposition: String,
    blocker: Option<String>,
}

#[derive(serde::Serialize)]
struct ReachReport {
    schema: &'static str,
    recording: String,
    game_id: String,
    hour_count: usize,
    first_hour: i64,
    last_hour: i64,
    open_ms: f64,
    open_warnings: Vec<String>,
    required_input_count: usize,
    hours: Vec<HourReport>,
    gap_input_hour_counts: BTreeMap<String, usize>,
}

const REACH_REPORT_SCHEMA: &str = "hoi4.replay.recording_reach.v2";

fn select_hours(all_hours: &[i64], selection: HourSelection) -> Vec<i64> {
    match selection {
        HourSelection::All => all_hours.to_vec(),
        HourSelection::Sample => {
            let mut picks = Vec::new();
            if let Some(first) = all_hours.first() {
                picks.push(*first);
            }
            if all_hours.len() > 2 {
                picks.push(all_hours[all_hours.len() / 2]);
            }
            if let Some(last) = all_hours.last() {
                if !picks.contains(last) {
                    picks.push(*last);
                }
            }
            picks
        }
    }
}

fn recorder_domain_problems(frame: &replay_core::WorldFrame) -> Vec<RecorderDomainProblem> {
    frame
        .provenance()
        .channels
        .get("collector")
        .map(|collector| {
            collector
                .problems
                .iter()
                .map(|problem| RecorderDomainProblem {
                    domain_id: problem.domain,
                    problem: problem.problem.clone(),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// From the same single placement law `units.air_base_location` and XMAP13 call.
fn wing_anchor_refusals(frame: &replay_core::WorldFrame) -> Vec<WingAnchorRefusalCensus> {
    let mut by_cause: BTreeMap<String, (usize, BTreeSet<String>)> = BTreeMap::new();
    for wing in frame.wings() {
        if let Err(refusal) = frame.wing_anchor_province(wing) {
            let entry = by_cause.entry(format!("{refusal:?}")).or_default();
            entry.0 += 1;
            if let Some(air_base) = wing.air_base.measured() {
                entry.1.insert(air_base.clone());
            }
        }
    }
    by_cause
        .into_iter()
        .map(|(cause, (wings, air_base_ids))| WingAnchorRefusalCensus {
            cause,
            wings,
            air_base_ids: air_base_ids.into_iter().collect(),
        })
        .collect()
}

pub(crate) fn write_report_create_new_or_equal(out: &Path, bytes: &[u8]) -> crate::Result<()> {
    use std::io::Write;
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(out)
    {
        Ok(mut file) => {
            file.write_all(bytes)
                .map_err(|error| format!("write report {}: {error}", out.display()))?;
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let existing = std::fs::read(out)
                .map_err(|error| format!("read existing {}: {error}", out.display()))?;
            if existing == bytes {
                Ok(())
            } else {
                Err(format!(
                    "--report-out {} already exists with different content; this command never \
                     truncates a file it did not write (the path could name a recording or a \
                     package-set descriptor) — choose a new path",
                    out.display()
                )
                .into())
            }
        }
        Err(error) => Err(format!("create report {}: {error}", out.display()).into()),
    }
}

pub(crate) fn run(
    recording: &Path,
    out: &Path,
    logical_stream_id: Option<u64>,
    hours: HourSelection,
) -> crate::Result<()> {
    let open_started = Instant::now();
    let source = match logical_stream_id {
        Some(id) => crate::recording_open::open_product_recording_segment(recording, Some(id)),
        None => crate::recording_open::open_product_recording(recording),
    }
    .map_err(|error| format!("open recording {}: {error}", recording.display()))?;
    let timeline = replay_core::WorldTimeline::open(source)
        .map_err(|error| format!("recording opened but its timeline is unusable: {error}"))?;
    let open_ms = open_started.elapsed().as_secs_f64() * 1000.0;
    let all_hours = timeline.hours().to_vec();
    let selected = select_hours(&all_hours, hours);

    let manifest = timeline.content_manifest();
    let mut hour_reports = Vec::with_capacity(selected.len());
    let mut gap_counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut required_input_count = 0usize;
    for hour in selected {
        let frame_started = Instant::now();
        let frame = timeline.frame(hour)?;
        let frame_decode_ms = frame_started.elapsed().as_secs_f64() * 1000.0;
        let mut admitted_inputs = Vec::new();
        let mut gap_inputs = Vec::new();
        let mut map_authority_error = None;
        match manifest {
            Some(manifest) => {
                match map_viewer::map_authority_projection_from_world_frame(&frame, manifest) {
                    Ok(projection) => {
                        required_input_count = projection.requirements.len();
                        for row in &projection.requirements {
                            match row.disposition {
                                map_viewer::MapAuthorityRequirementDisposition::Admitted => {
                                    admitted_inputs.push(row.input_id.clone())
                                }
                                _ => {
                                    *gap_counts.entry(row.input_id.clone()).or_default() += 1;
                                    gap_inputs.push(GapRow {
                                        input_id: row.input_id.clone(),
                                        disposition: format!("{:?}", row.disposition),
                                        blocker: row.blocker.clone(),
                                    });
                                }
                            }
                        }
                    }
                    Err(error) => map_authority_error = Some(error),
                }
            }
            None => {
                map_authority_error =
                    Some("recording carries no content manifest; the XMAP00 projection needs its digest".to_string())
            }
        }
        hour_reports.push(HourReport {
            hour_index: hour,
            coverage_warnings: frame.coverage_warnings().to_vec(),
            map_authority_error,
            admitted_inputs,
            gap_inputs,
            recorder_domain_problems: recorder_domain_problems(&frame),
            wing_anchor_refusals: wing_anchor_refusals(&frame),
            frame_decode_ms,
        });
    }

    let report = ReachReport {
        schema: REACH_REPORT_SCHEMA,
        recording: recording.display().to_string(),
        game_id: timeline.game_id().to_string(),
        hour_count: all_hours.len(),
        first_hour: timeline.first_hour(),
        last_hour: timeline.last_hour(),
        open_ms,
        open_warnings: timeline.warnings().to_vec(),
        required_input_count,
        hours: hour_reports,
        gap_input_hour_counts: gap_counts,
    };
    let mut bytes = serde_json::to_vec_pretty(&report)?;
    bytes.push(b'\n');
    write_report_create_new_or_equal(out, &bytes)?;
    println!(
        "{}: {} hour(s), {}..={}, opened in {:.1} ms, {} report hour(s) -> {}",
        report.game_id,
        report.hour_count,
        report.first_hour,
        report.last_hour,
        report.open_ms,
        report.hours.len(),
        out.display()
    );
    for hour in &report.hours {
        println!(
            "  hour {}: {} admitted, {} gap(s), {} coverage warning(s), decode {:.1} ms",
            hour.hour_index,
            hour.admitted_inputs.len(),
            hour.gap_inputs.len(),
            hour.coverage_warnings.len(),
            hour.frame_decode_ms
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_selection_is_first_middle_last_and_never_repeats() {
        assert_eq!(select_hours(&[], HourSelection::Sample), Vec::<i64>::new());
        assert_eq!(select_hours(&[7], HourSelection::Sample), vec![7]);
        assert_eq!(select_hours(&[7, 8], HourSelection::Sample), vec![7, 8]);
        assert_eq!(select_hours(&[1, 2, 3], HourSelection::Sample), vec![1, 2, 3]);
        let even: Vec<i64> = (100..112).collect();
        assert_eq!(select_hours(&even, HourSelection::Sample), vec![100, 106, 111]);
        let odd: Vec<i64> = (100..111).collect();
        assert_eq!(select_hours(&odd, HourSelection::Sample), vec![100, 105, 110]);
    }

    #[test]
    fn the_report_never_truncates_a_file_it_did_not_write() {
        let dir = std::env::temp_dir().join(format!(
            "battle-plan-reach-report-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock is after the epoch")
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("occupied.json");
        std::fs::write(&path, b"a recording, or something else that is not this report")
            .expect("seed the occupied path");

        let error = write_report_create_new_or_equal(&path, b"{}\n")
            .expect_err("an occupied path with different content is refused");
        assert!(
            error.to_string().contains("already exists with different content"),
            "the refusal names the cause: {error}"
        );
        assert_eq!(
            std::fs::read(&path).expect("the occupant survives"),
            b"a recording, or something else that is not this report",
            "the occupant's bytes are untouched"
        );

        let fresh = dir.join("report.json");
        write_report_create_new_or_equal(&fresh, b"{}\n").expect("a fresh path is written");
        write_report_create_new_or_equal(&fresh, b"{}\n").expect("an identical rerun is allowed");
        assert_eq!(std::fs::read(&fresh).expect("report readable"), b"{}\n");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
