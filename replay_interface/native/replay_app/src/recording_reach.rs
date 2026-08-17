//! `--report-recording-reach`: what one REAL recording reaches in this viewer.
//!
//! Lane MH exists because nothing in this repository had ever opened a
//! recorder-produced `.battleplan` in the viewer — every replay fixture is
//! synthetic — so "which map inputs would light up on a real capture" was an
//! opinion. This command turns it into a file.
//!
//! It opens the recording through the SAME path the shipped windowed and
//! `--dump-frame` routes use — `recording_open::open_product_recording` plus
//! `WorldTimeline::open` — and materializes only the hours it reports.
//!
//! Deliberately NOT `validate::open_timeline_segment`: that is the
//! certification open, and it additionally runs `validate_all_frame_payloads`,
//! which materializes EVERY hour. On a real 186-hour capture (t79) that did not
//! finish in 68 minutes and peaked at 9.2 GB, because a frame carrying
//! `_r2_direct_frd_transitions` schema 4 re-folds the whole history from hour
//! one (`replay_core::world::accumulate_frd_history`). A report that cannot run
//! on a real recording answers nothing, and "what a player's viewer shows" is a
//! question about the product open anyway.
//!
//! For each reported hour it writes:
//!
//!   * the frame's own `coverage_warnings` — the recorded reason a domain did
//!     not survive admission (a malformed carrier names itself here), and
//!   * the complete XMAP00 map-authority projection for that hour: every
//!     required map input with its disposition and, when refused, its blocker.
//!
//! It renders NOTHING. It needs no D3D11 device, no game install and no
//! content binding, which is exactly why it can answer "what is dark and why"
//! on a machine and in a CI run where a real render cannot happen. What the
//! frame composition then does with a refused input is a separate question,
//! answered by `--dump-frame`.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Instant;



/// Which hours to materialize. A recording is hundreds of hours long and every
/// hour costs a decode, so the default samples the three the eye checks first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HourSelection {
    /// First, middle and last recorded hour.
    Sample,
    /// Every recorded hour.
    All,
}

#[derive(serde::Serialize)]
struct HourReport {
    hour_index: i64,
    coverage_warnings: Vec<String>,
    /// `None` when the projection itself could not be built for this hour.
    map_authority_error: Option<String>,
    admitted_inputs: Vec<String>,
    gap_inputs: Vec<GapRow>,
    frame_decode_ms: f64,
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
    /// input_id -> how many of the reported hours refused it.
    gap_input_hour_counts: BTreeMap<String, usize>,
}

const REACH_REPORT_SCHEMA: &str = "hoi4.replay.recording_reach.v1";

/// The hours this report materializes, in recorded order and without repeats.
///
/// `Sample` is first, middle and last — the three an eye checks — and it must
/// stay exactly those: on a two-hour recording it is first and last with no
/// middle, and on a one-hour recording it is that hour once.
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

/// Write the report without ever truncating a file this process did not write.
///
/// `--report-out` is an operator-supplied path and the input it describes is a
/// recording — under the single-file law the recording is the only
/// extractor-to-replay connection and this command must never write into it or
/// into a sibling of a rollover package set. A plain `fs::write` truncates
/// whatever is there, so an out path that named the package-set descriptor (or
/// an unselected member) would destroy it. Creating the file exclusively means
/// the only file this can ever write is one that did not exist; re-running onto
/// its own byte-identical output is still allowed so a repeated report is not a
/// failure.
fn write_report_create_new_or_equal(out: &Path, bytes: &[u8]) -> crate::Result<()> {
    use std::io::Write;
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(out)
    {
        Ok(mut file) => {
            file.write_all(bytes)
                .map_err(|error| format!("write reach report {}: {error}", out.display()))?;
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
        Err(error) => Err(format!("create reach report {}: {error}", out.display()).into()),
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

    /// `run` calls this exact function, so changing which hour the sample picks
    /// changes this result.
    ///
    /// kills: returning every hour for `Sample`; dropping the last hour (the
    /// one an off-by-one loses first); emitting a repeated hour on a 1- or
    /// 2-hour recording, which would decode and report the same hour twice;
    /// and indexing `all_hours[len / 2]` on an empty slice, which panics.
    #[test]
    fn sample_selection_is_first_middle_last_and_never_repeats() {
        assert_eq!(select_hours(&[], HourSelection::Sample), Vec::<i64>::new());
        assert_eq!(select_hours(&[7], HourSelection::Sample), vec![7]);
        assert_eq!(select_hours(&[7, 8], HourSelection::Sample), vec![7, 8]);
        // Odd length: the middle is the true centre.
        assert_eq!(select_hours(&[1, 2, 3], HourSelection::Sample), vec![1, 2, 3]);
        // Even length: len/2 is the upper of the two central hours.
        let even: Vec<i64> = (100..112).collect();
        assert_eq!(select_hours(&even, HourSelection::Sample), vec![100, 106, 111]);
        let odd: Vec<i64> = (100..111).collect();
        assert_eq!(select_hours(&odd, HourSelection::Sample), vec![100, 105, 110]);
    }

    /// kills: replacing the exclusive create with `fs::write`, which would
    /// truncate an existing file — the defect this guards is `--report-out`
    /// naming a recording or a rollover package-set descriptor.
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

        // A fresh path is written, and re-writing the same bytes is not a failure.
        let fresh = dir.join("report.json");
        write_report_create_new_or_equal(&fresh, b"{}\n").expect("a fresh path is written");
        write_report_create_new_or_equal(&fresh, b"{}\n").expect("an identical rerun is allowed");
        assert_eq!(std::fs::read(&fresh).expect("report readable"), b"{}\n");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
