
use super::{
    Admissions, Destination, FAILURE_SEGMENT, Finalized, Held, Naming, Refusal, Result,
    admitted_branch, apply_failure_ranges, block_settings, campaign_failure, failure,
    failure_segment_ranges, failure_session,
    session::{Live, end_campaign, is_restart, reconcile_session, written_campaigns},
    session_admissions, session_witness, with_packing, write_json,
};
use replay_recording::records::Records;
use replay::campaign::{Replay, writer};
use replay_settings::{publication, recorder_settings::RecorderSettings};
use replay_capture_spool::spool::{Recovered, Recovery};
use replay_data::{
    Hour,
    format::FormatVersion,
    witness::{TerminalWitness, UncommittedRange},
};
use replay_frame::branches::Gap;
use serde_json::Value;
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    time::Instant,
};

pub fn finalize_session(session_dir: &Path, replay_dir: &Path) -> Result<Vec<PathBuf>> {
    let finalized = finalize_session_outcome(session_dir, replay_dir)?;
    if finalized.errors.is_empty() {
        Ok(finalized.replays)
    } else {
        Err(finalized.errors.join("\n"))
    }
}

pub fn finalize_session_outcome(session_dir: &Path, replay_dir: &Path) -> Result<Finalized> {
    finalize_session_outcome_with_progress(session_dir, replay_dir, &|| Ok(()))
}

pub fn finalize_session_outcome_with_progress(
    session_dir: &Path,
    replay_dir: &Path,
    progress: &dyn Fn() -> std::io::Result<()>,
) -> Result<Finalized> {
    let settings = RecorderSettings::path();
    let naming = Naming {
        localisation: None,
        settings: &settings,
        report: None,
    };
    finalize_session_ahead(
        session_dir,
        replay_dir,
        progress,
        Live::default(),
        naming,
        &|_| Destination::Recording,
    )
}

pub fn finalize_session_ahead(
    session_dir: &Path,
    replay_dir: &Path,
    progress: &dyn Fn() -> std::io::Result<()>,
    mut live: Live,
    naming: Naming<'_>,
    destination: &dyn Fn(&str) -> Destination,
) -> Result<Finalized> {
    with_packing(|| {
        progress().map_err(|error| failure(session_dir, error))?;
        fs::create_dir_all(replay_dir).map_err(|error| failure(replay_dir, error))?;
        reconcile_session(session_dir, replay_dir)?;
        let admissions = session_admissions(session_dir)?;
        let witness = session_witness(session_dir)?;
        let campaigns = written_campaigns(session_dir)?;
        let ended = ["completed.json", "waiting.json"]
            .iter()
            .any(|name| session_dir.join(name).is_file());
        if campaigns.is_empty() && admissions.is_none() && !ended {
            return Err(failure(session_dir, "no campaign journal found"));
        }
        let mut finalized = Finalized::default();
        for campaign in campaigns {
            let save = match destination(&campaign) {
                Destination::Recording => true,
                Destination::Unsaved => false,
                Destination::Neither => continue,
            };
            let started = Instant::now();
            let after = |located: Option<&Path>| match &witness {
                Some(witness) => name_lost_hours(
                    session_dir,
                    replay_dir,
                    &campaign,
                    located,
                    witness,
                    admissions.as_ref(),
                ),
                None => Ok(None),
            };
            match end_campaign(
                &mut live,
                session_dir,
                replay_dir,
                &campaign,
                save,
                naming,
                progress,
                &after,
            ) {
                Ok(ended) => {
                    finalized.unrenamed.extend(ended.unrenamed);
                    if let Some(admissions) = &admissions {
                        finalized.omitted.extend(
                            ended
                                .omitted
                                .into_iter()
                                .map(|(load, hours)| admissions.omission(&campaign, load, hours)),
                        );
                    }
                    let Some(path) = ended.recording else {
                        continue;
                    };
                    if !ended.saved {
                        finalized.unsaved.push(path);
                        continue;
                    }
                    report(&path, ended.hours, started);
                    finalized.saved_campaigns.insert(campaign);
                    finalized.replays.push(path);
                }
                Err(error) => {
                    finalized.failed_campaigns.insert(campaign);
                    finalized.errors.push(error);
                }
            }
        }
        if admissions.is_some()
            && finalized.replays.is_empty()
            && finalized.unsaved.is_empty()
            && finalized.errors.is_empty()
        {
            println!("No admitted load recorded an hour; nothing published.");
        }
        for omission in &finalized.omitted {
            println!(
                "{}: {} hours of load {} left out ({})",
                omission.campaign,
                omission.hours,
                omission
                    .load
                    .map_or_else(|| "unidentified".to_owned(), |load| load.to_string()),
                omission.refusal.map_or("not admitted", Refusal::as_str)
            );
        }
        for unrenamed in &finalized.unrenamed {
            println!("{unrenamed}");
        }
        Ok(finalized)
    })
}

fn report(path: &Path, hours: u64, started: Instant) {
    let Ok(replay) = Replay::open_path(path) else {
        return;
    };
    let recorded: u64 = replay
        .index
        .iter()
        .map(|block| u64::from((block.last_hour - block.first_hour + 1) as u32))
        .sum();
    println!(
        "Finalization evidence: {hours} hours written at the end; {recorded} recorded hours in the recording."
    );
    println!(
        "finalize_s={:.3} replay_bytes={} hours_saved={hours}",
        started.elapsed().as_secs_f64(),
        replay.saved_length()
    );
    println!(
        "gaps={} damaged_segments={}",
        replay.gaps.len(),
        replay.recovery_notes.iter().filter(|note| !is_restart(note)).count()
    );
}

pub fn write_session_end(
    session_dir: &Path,
    replay_dir: &Path,
    name: &str,
    value: &Value,
) -> Result<()> {
    if !["completed.json", "waiting.json"].contains(&name) {
        return Err(failure(session_dir, "not a session end record"));
    }
    reconcile_session(session_dir, replay_dir)?;
    write_json(&session_dir.join(name), value)
}

fn name_lost_hours(
    session_dir: &Path,
    replay_dir: &Path,
    campaign: &str,
    located: Option<&Path>,
    primary: &TerminalWitness,
    admissions: Option<&Admissions>,
) -> Result<Option<PathBuf>> {
    let projected = campaign_failure(session_dir, campaign.as_bytes(), primary)?;
    let witness = &projected.witness;
    let none = BTreeSet::new();
    let bootstrap = BTreeSet::new();
    let admitted = admissions.map(|admissions| admissions.loads.get(campaign).unwrap_or(&none));
    let failing = witness
        .campaign_id
        .as_ref()
        .is_some_and(|id| id.as_slice() == campaign.as_bytes())
        && admitted_branch(admitted, &bootstrap, witness.branch, witness.load_signal);
    let ranges: Vec<UncommittedRange> = projected
        .ranges
        .iter()
        .copied()
        .filter(|range| {
            admitted_branch(admitted, &bootstrap, Some(range.branch), range.load_signal)
        })
        .collect();
    if ranges.is_empty() && !(failing && witness.failed_hour.is_some()) {
        return Ok(None);
    }
    let level = block_settings().level;
    let Some(path) = located else {
        let session = failure_session(session_dir, witness)?;
        let mut segments = [(
            FAILURE_SEGMENT,
            Recovery {
                format: Some(FormatVersion::CURRENT),
                records: vec![Recovered::SessionStart(session)],
                valid_bytes: 0,
                damage: None,
            },
        )];
        apply_failure_ranges(
            &mut segments,
            witness,
            failing,
            &ranges,
            projected.bundled,
            None,
        )?;
        let [(_, recovery)] = segments;
        let records = lost_records(recovery.records, None);
        if records.gaps.is_empty() && records.controls.is_empty() {
            return Ok(None);
        }
        publication::create_unsaved_folder(replay_dir)
            .map_err(|error| failure(replay_dir, error))?;
        let path = publication::unsaved_path(replay_dir, campaign);
        writer::create(&path, campaign.as_bytes(), &records, level)
            .map_err(|error| failure(&path, error))?;
        return Ok(Some(path));
    };
    let newest = Replay::open_newest(path).map_err(|error| failure(path, error))?;
    let held = Held {
        recording: &newest,
        path,
    };
    let Some((_, recovery)) =
        failure_segment_ranges(held, witness, failing, &ranges, projected.bundled)?
    else {
        return Ok(None);
    };
    let records = lost_records(recovery.records, Some(&newest));
    if !records.is_empty() {
        let addition = writer::Addition {
            records: Some(&records),
            ..writer::Addition::default()
        };
        writer::revise(path, newest.recording(), &addition, false, level)
            .map_err(|error| failure(path, error))?;
    }
    Ok(None)
}

fn lost_records(found: Vec<Recovered>, held: Option<&Replay>) -> Records {
    let mut records = Records::default();
    for record in found {
        match record {
            Recovered::SessionStart(session) => {
                if !held
                    .is_some_and(|held| held.sessions.iter().any(|known| known.id == session.id))
                {
                    records.sessions.push(session);
                }
            }
            Recovered::BranchStart(start) => {
                if !held.is_some_and(|held| held.branches.iter().any(|known| known.id == start.id))
                    && !records.branches.iter().any(|known| known.id == start.id)
                {
                    records.branches.push(start);
                }
            }
            Recovered::Gap(gap) => {
                let captured = |hour: i64| {
                    held.is_some_and(|held| {
                        held.index.iter().any(|block| {
                            block.branch == gap.branch
                                && i64::from(block.first_hour) <= hour
                                && hour <= i64::from(block.last_hour)
                        })
                    })
                };
                let last = i64::from(gap.last_hour);
                let mut first = i64::from(gap.first_hour);
                while first <= last {
                    if captured(first) {
                        first += 1;
                        continue;
                    }
                    let mut end = first;
                    while end < last && !captured(end + 1) {
                        end += 1;
                    }
                    let lost = Gap {
                        branch: gap.branch,
                        first_hour: first as Hour,
                        last_hour: end as Hour,
                        reason: gap.reason.clone(),
                    };
                    if !held.is_some_and(|held| held.gaps.contains(&lost)) {
                        records.gaps.push(lost);
                    }
                    first = end + 1;
                }
            }
            Recovered::Control(control) => {
                if !held.is_some_and(|held| held.controls.contains(&control)) {
                    records.controls.push(control);
                }
            }
            Recovered::Frame(_) => {}
        }
    }
    records
}
