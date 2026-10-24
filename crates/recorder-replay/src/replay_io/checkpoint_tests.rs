use super::*;
use crate::test_support::{
    ALLOCATION_CAMPAIGN, allocation_records, allocation_test_directory, recorded_frames,
    write_load_segment,
};
use std::sync::{Arc, Mutex};

struct OutputCapture;
impl OutputCapture {
    fn take(&self) -> String {
        FINALIZATION_OUTPUT
            .with_borrow_mut(|output| std::mem::take(output.as_mut().unwrap()).concat())
    }

    fn new() -> Self {
        FINALIZATION_OUTPUT.with_borrow_mut(|output| {
            assert!(output.replace(Vec::new()).is_none());
        });
        Self
    }
}
impl Drop for OutputCapture {
    fn drop(&mut self) {
        FINALIZATION_OUTPUT.with_borrow_mut(|output| *output = None);
    }
}

struct Fixture {
    root: PathBuf,
    session: PathBuf,
    output: PathBuf,
    settings: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let root = allocation_test_directory(name);
        Self {
            session: root.join("session"),
            output: root.join("output"),
            settings: root.join("settings.json"),
            root,
        }
    }

    fn name() -> &'static str {
        std::str::from_utf8(ALLOCATION_CAMPAIGN).unwrap()
    }

    fn campaign(&self, session: &Path) -> PathBuf {
        session.join(Self::name())
    }

    fn naming(&self) -> Naming<'_> {
        Naming {
            localisation: None,
            settings: &self.settings,
            report: None,
        }
    }

    fn checkpoint(&self, through: u32, auto_save: bool, ahead: &mut Ahead) -> Checkpoint {
        checkpoint_campaign(
            &self.session,
            &self.output,
            Self::name(),
            through,
            auto_save,
            ahead,
            self.naming(),
            &|| Ok(()),
        )
        .unwrap()
        .unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn a_checkpoint_saves_closed_segments_deletes_them_and_the_last_save_adds_only_the_rest() {
    let fixture = Fixture::new("checkpoint-saves");
    let campaign = fixture.campaign(&fixture.session);
    let first = write_load_segment(&campaign, 0, 9, 2..=4, 19);
    let second = write_load_segment(&campaign, 1, 10, 5..=6, 21);
    let handed = Arc::new(Mutex::new(Vec::new()));
    let mut ahead = Ahead::default();
    let seen = Arc::clone(&handed);
    ahead.on_committed(Box::new(move |recording, replay, blocks| {
        let hours: Vec<_> = blocks
            .iter()
            .flat_map(|block| replay.read_block(block).unwrap())
            .map(|frame| (frame.branch, frame.hour))
            .collect();
        seen.lock().unwrap().push((recording.to_owned(), hours));
    }));
    let done = fixture.checkpoint(0, true, &mut ahead);
    assert_eq!(done.hours, 3);
    // The segment saved goes; the newest stays for the collector.
    assert_eq!(done.deleted, 1);
    assert!(!first.exists());
    assert!(second.exists());
    assert_eq!(recorded_frames(&done.recording), [(9, 2), (9, 3), (9, 4)]);
    assert_eq!(
        *handed.lock().unwrap(),
        [(done.recording.clone(), vec![(9, 2), (9, 3), (9, 4)])]
    );
    let checkpointed = fs::read(&done.recording).unwrap();
    // The save at the end adds what the checkpoint left, after every byte it wrote.
    let saved = finalize_session(&fixture.session, &fixture.output).unwrap();
    assert_eq!(saved.len(), 1);
    assert_eq!(
        recorded_frames(&saved[0]),
        [(9, 2), (9, 3), (9, 4), (10, 5), (10, 6)]
    );
    let finished = fs::read(&saved[0]).unwrap();
    let slot = Replay::open(checkpointed.clone()).unwrap();
    let header = slot.index[0].offset as usize - 32;
    assert_eq!(finished[..header], checkpointed[..header]);
    assert_eq!(
        finished[header + 32..checkpointed.len()],
        checkpointed[header + 32..]
    );
    // Saving again changes nothing.
    assert_eq!(
        finalize_session(&fixture.session, &fixture.output).unwrap(),
        saved
    );
    assert_eq!(fs::read(&saved[0]).unwrap(), finished);
}

#[test]
fn discard_keeps_published_checkpoint_hours_and_drops_only_the_remaining_spool() {
    let fixture = Fixture::new("checkpoint-discard");
    let earlier = fixture.root.join("earlier");
    write_load_segment(&fixture.campaign(&earlier), 0, 9, 2..=4, 19);
    let recording = finalize_session(&earlier, &fixture.output)
        .unwrap()
        .remove(0);

    let campaign = fixture.campaign(&fixture.session);
    write_load_segment(&campaign, 0, 10, 5..=6, 21);
    write_load_segment(&campaign, 1, 11, 7..=8, 23);
    let done = fixture.checkpoint(0, true, &mut Ahead::default());
    assert_eq!(done.recording, recording);
    assert_eq!(
        recorded_frames(&recording),
        [(9, 2), (9, 3), (9, 4), (10, 5), (10, 6)]
    );
    let published = fs::read(&recording).unwrap();
    assert!(read_checkpointed(&campaign).unwrap().is_none());
    discard_checkpoints(&fixture.session, Fixture::name()).unwrap();
    discard_unsaved(&fixture.session, Fixture::name()).unwrap();
    assert_eq!(fs::read(&recording).unwrap(), published);
    assert!(segment_paths(&campaign).unwrap().is_empty());
    assert!(!campaign.join(CHECKPOINT_FILE).exists());
}

#[test]
fn without_auto_save_checkpoints_wait_in_the_unsaved_recording_until_saved_or_discarded() {
    let fixture = Fixture::new("checkpoint-unsaved");
    let campaign = fixture.campaign(&fixture.session);
    write_load_segment(&campaign, 0, 9, 2..=4, 19);
    write_load_segment(&campaign, 1, 10, 5..=6, 21);
    let done = fixture.checkpoint(0, false, &mut Ahead::default());
    let staged = replay::publication::unsaved_path(&fixture.output, Fixture::name());
    assert_eq!(done.recording, staged);
    assert_eq!(
        recording_files::recording_of(&fixture.output, ALLOCATION_CAMPAIGN).unwrap(),
        None,
        "nothing is published"
    );
    // Its deleted segment's hours still wait, with the spool's.
    let waiting = unsaved(&fixture.session, Fixture::name()).unwrap().unwrap();
    assert_eq!(waiting.hours, 5);
    assert_eq!((waiting.first_hour, waiting.last_hour), (Some(2), Some(6)));
    discard_checkpoints(&fixture.session, Fixture::name()).unwrap();
    discard_unsaved(&fixture.session, Fixture::name()).unwrap();
    assert!(!staged.exists());
    assert!(segment_paths(&campaign).unwrap().is_empty());
}

#[test]
fn saving_only_checkpointed_hours_returns_the_publication_from_live_and_ended_sessions() {
    for ended in [false, true] {
        for context_only in [false, true] {
            let fixture = Fixture::new(&format!("checkpoint-only-save-{ended}-{context_only}"));
            begin_admissions(&fixture.session, &fixture.output).unwrap();
            Admissions::default()
                .admit(&fixture.session, &fixture.output, Fixture::name(), 19)
                .unwrap();
            let campaign = fixture.campaign(&fixture.session);
            let first = write_load_segment(&campaign, 0, 9, 2..=4, 19);
            if context_only {
                let context = replay::spool::recover_prefix(&fs::read(&first).unwrap());
                assert!(context.damage.is_none());
                fs::write(
                    campaign.join("spool.0001.h4s"),
                    context
                        .records
                        .iter()
                        .filter(|record| !matches!(record, Record::Frame(_)))
                        .flat_map(|record| replay::spool::encode_record(record).unwrap())
                        .collect::<Vec<_>>(),
                )
                .unwrap();
            } else {
                write_load_segment(&campaign, 1, 10, 5..=6, 21);
            }
            let checkpoint = fixture.checkpoint(0, false, &mut Ahead::default());
            assert_eq!(checkpoint.deleted, 1);
            if ended {
                write_session_end(
                    &fixture.session,
                    &fixture.output,
                    "waiting.json",
                    &json!({"waiting": []}),
                )
                .unwrap();
            }
            let (saved, omitted) = save_campaign(
                &fixture.session,
                &fixture.output,
                Fixture::name(),
                false,
                &|| Ok(()),
                &mut Ahead::default(),
                fixture.naming(),
            )
            .unwrap();
            let published = recording_files::recording_of(&fixture.output, ALLOCATION_CAMPAIGN)
                .unwrap()
                .unwrap();
            assert_eq!(saved, Some(published.clone()));
            assert_eq!(
                omitted
                    .iter()
                    .map(|omission| (omission.load, omission.hours))
                    .collect::<Vec<_>>(),
                if context_only {
                    vec![]
                } else {
                    vec![(Some(21), 2)]
                },
            );
            assert_eq!(recorded_frames(&published), [(9, 2), (9, 3), (9, 4)]);
            assert!(!checkpoint.recording.exists());
            assert!(read_checkpointed(&campaign).unwrap().is_none());
            reconcile_session_checkpoints(&fixture.session, &fixture.output).unwrap();
        }
    }
}

#[test]
fn frameless_checkpoint_tails_require_unchanged_metadata_and_no_recovery_damage() {
    use crate::test_support::allocation_frame;

    let lost = Gap {
        branch: 9,
        first_hour: 5,
        last_hour: 6,
        reason: "synthetic disk interruption".into(),
    };
    let clock = |hour: i32| {
        Record::Control(ControlRecord::ClockWitness {
            session: 7,
            qpc: 9_000 + hour as u64,
            hour,
            load_signal: Some(19),
        })
    };
    for auto_save in [false, true] {
        for metadata in [
            "unchanged",
            "gap",
            "gap-explained-clock",
            "gap-unexplained-clock",
            "gap-reordered-clocks",
            "gap-over-saved-hour",
            "event-loss",
            "clock",
            "reordered-clocks",
        ] {
            for torn in [false, true] {
                let fixture = Fixture::new(&format!(
                    "checkpoint-frameless-{auto_save}-{metadata}-{torn}"
                ));
                begin_admissions(&fixture.session, &fixture.output).unwrap();
                Admissions::default()
                    .admit(&fixture.session, &fixture.output, Fixture::name(), 19)
                    .unwrap();
                let campaign = fixture.campaign(&fixture.session);
                let first = write_load_segment(&campaign, 0, 9, 2..=4, 19);
                let recovered = replay::spool::recover_prefix(&fs::read(&first).unwrap());
                let mut context: Vec<_> = recovered
                    .records
                    .into_iter()
                    .filter(|record| !matches!(record, Record::Frame(_)))
                    .collect();
                let tail = campaign.join("spool.0001.h4s");
                fs::write(&tail, []).unwrap();
                let checkpoint = fixture.checkpoint(0, auto_save, &mut Ahead::default());
                assert_eq!(checkpoint.deleted, 1);
                assert!(!first.exists());
                match metadata {
                    "gap" => context.push(Record::Gap(lost.clone())),
                    "gap-explained-clock" => {
                        context.extend([Record::Gap(lost.clone()), clock(6)]);
                    }
                    "gap-unexplained-clock" => {
                        context.extend([Record::Gap(lost.clone()), clock(7)]);
                    }
                    "gap-reordered-clocks" => {
                        context[2..].reverse();
                        context.push(Record::Gap(lost.clone()));
                    }
                    "gap-over-saved-hour" => context.push(Record::Gap(Gap {
                        first_hour: 4,
                        ..lost.clone()
                    })),
                    "event-loss" => context.push(Record::Control(ControlRecord::EventLoss {
                        session: 7,
                        branch: 9,
                        after_hour: 4,
                        hour: 7,
                    })),
                    "clock" => context.push(clock(7)),
                    "reordered-clocks" => context[2..].reverse(),
                    "unchanged" => {}
                    _ => unreachable!(),
                }
                let mut bytes: Vec<_> = context
                    .iter()
                    .flat_map(|record| replay::spool::encode_record(record).unwrap())
                    .collect();
                if torn {
                    let frame =
                        replay::spool::encode_record(&Record::Frame(allocation_frame(9, 7, 19)))
                            .unwrap();
                    bytes.extend_from_slice(&frame[..frame.len() - 1]);
                }
                fs::write(&tail, &bytes).unwrap();
                let recovered = replay::spool::recover_prefix(&bytes);
                assert_eq!(recovered.damage.is_some(), torn);
                assert!(
                    !recovered
                        .records
                        .iter()
                        .any(|record| matches!(record, Record::Frame(_)))
                );
                let recording = fs::read(&checkpoint.recording).unwrap();
                let (revision, gaps, controls) = {
                    let before = held_recording(&checkpoint.recording).unwrap();
                    (
                        before.revision(),
                        before.gaps.clone(),
                        before.controls.clone(),
                    )
                };
                let save = || {
                    save_campaign(
                        &fixture.session,
                        &fixture.output,
                        Fixture::name(),
                        !auto_save,
                        &|| Ok(()),
                        &mut Ahead::default(),
                        fixture.naming(),
                    )
                };
                if matches!(metadata, "gap" | "gap-explained-clock") && !torn {
                    // Explicit loss adds its gap, and only its new clock, to the saved hours.
                    let saved = save().unwrap().0.unwrap();
                    if !auto_save {
                        assert_eq!(saved, checkpoint.recording);
                    }
                    let mut controls = controls;
                    if metadata == "gap-explained-clock" {
                        let Record::Control(control) = clock(6) else {
                            unreachable!()
                        };
                        controls.push(control);
                    }
                    {
                        let after = held_recording(&saved).unwrap();
                        assert!(after.contains_revision(revision).unwrap());
                        assert_eq!(after.gaps, [gaps, vec![lost.clone()]].concat());
                        assert_eq!(after.controls, controls);
                    }
                    assert_eq!(recorded_frames(&saved), [(9, 2), (9, 3), (9, 4)]);
                    assert_eq!(
                        read_saved(&campaign).unwrap().segments.get(&1),
                        Some(&(bytes.len() as u64))
                    );
                    let added = fs::read(&saved).unwrap();
                    assert_eq!(save().unwrap().0, Some(saved.clone()));
                    assert_eq!(fs::read(&saved).unwrap(), added);
                    assert_eq!(fs::read(&tail).unwrap(), bytes);
                    continue;
                }
                if metadata == "unchanged" && !torn {
                    assert_eq!(save().unwrap().0, Some(checkpoint.recording.clone()));
                    assert_eq!(
                        read_saved(&campaign).unwrap().segments.get(&1),
                        Some(&(bytes.len() as u64))
                    );
                } else {
                    for _ in 0..2 {
                        let error = save().unwrap_err();
                        assert!(
                            error.contains("frameless spool has damage or metadata"),
                            "{error}"
                        );
                        assert!(error.contains("spool retained"));
                        assert!(!read_saved(&campaign).unwrap().segments.contains_key(&1));
                    }
                }
                assert_eq!(fs::read(&tail).unwrap(), bytes);
                assert_eq!(fs::read(&checkpoint.recording).unwrap(), recording);
                assert_eq!(
                    recorded_frames(&checkpoint.recording),
                    [(9, 2), (9, 3), (9, 4)]
                );
            }
        }
    }
}

/// Checkpoints load 19's hours 2..=4 on branch 9 and returns the session start that tails repeat.
fn checkpointed_load(fixture: &Fixture, auto_save: bool, loads: &[i64]) -> (Checkpoint, Record) {
    begin_admissions(&fixture.session, &fixture.output).unwrap();
    let mut admissions = Admissions::default();
    for load in loads {
        admissions
            .admit(&fixture.session, &fixture.output, Fixture::name(), *load)
            .unwrap();
    }
    let campaign = fixture.campaign(&fixture.session);
    let first = write_load_segment(&campaign, 0, 9, 2..=4, 19);
    let session = replay::spool::recover_prefix(&fs::read(&first).unwrap())
        .records
        .remove(0);
    assert!(matches!(session, Record::SessionStart(_)));
    fs::write(campaign.join("spool.0001.h4s"), []).unwrap();
    let checkpoint = fixture.checkpoint(0, auto_save, &mut Ahead::default());
    assert_eq!(checkpoint.deleted, 1);
    assert!(!first.exists());
    (checkpoint, session)
}

fn write_tail(campaign: &Path, records: &[Record]) -> Vec<u8> {
    let bytes: Vec<_> = records
        .iter()
        .flat_map(|record| replay::spool::encode_record(record).unwrap())
        .collect();
    let recovered = replay::spool::recover_prefix(&bytes);
    assert!(recovered.damage.is_none());
    assert_eq!(recovered.records.len(), records.len());
    fs::write(campaign.join("spool.0001.h4s"), &bytes).unwrap();
    bytes
}

#[test]
fn a_context_only_loss_segment_after_a_checkpoint_appends_its_gaps_once_and_discards_cleanly() {
    for auto_save in [false, true] {
        let fixture = Fixture::new(&format!("checkpoint-explicit-loss-{auto_save}"));
        let (checkpoint, session) = checkpointed_load(&fixture, auto_save, &[19]);
        let campaign = fixture.campaign(&fixture.session);
        let (revision, gaps, controls, branch) = {
            let before = held_recording(&checkpoint.recording).unwrap();
            let branch = before
                .branches
                .iter()
                .find(|branch| branch.id == 9)
                .cloned();
            (
                before.revision(),
                before.gaps.clone(),
                before.controls.clone(),
                branch.unwrap(),
            )
        };
        // As the collector closes a failed campaign: context, its gaps and event loss, no clock.
        let lost = Gap {
            branch: 9,
            first_hour: 5,
            last_hour: 7,
            reason: "spool write failed".into(),
        };
        let event_loss = ControlRecord::EventLoss {
            session: 7,
            branch: 9,
            after_hour: 4,
            hour: 8,
        };
        let bytes = write_tail(
            &campaign,
            &[
                session,
                Record::BranchStart(branch),
                Record::Gap(lost.clone()),
                Record::Control(event_loss.clone()),
            ],
        );
        let save = || {
            save_campaign(
                &fixture.session,
                &fixture.output,
                Fixture::name(),
                !auto_save,
                &|| Ok(()),
                &mut Ahead::default(),
                fixture.naming(),
            )
            .unwrap()
        };
        let capture = OutputCapture::new();
        let (saved, omitted) = save();
        let spool_bytes: u64 = read_saved(&campaign).unwrap().segments.values().sum();
        let output = capture.take();
        let headers: Vec<_> = output
            .lines()
            .filter(|line| line.starts_with("finalize_s="))
            .collect();
        assert_eq!(headers.len(), 1, "{output}");
        assert!(
            headers[0].ends_with(&format!("spool_bytes={spool_bytes}")),
            "{output}"
        );
        let saved = saved.unwrap();
        assert!(omitted.is_empty());
        if !auto_save {
            assert_eq!(saved, checkpoint.recording);
        }
        {
            let after = held_recording(&saved).unwrap();
            assert!(after.contains_revision(revision).unwrap());
            assert_eq!(after.gaps, [gaps, vec![lost]].concat());
            assert_eq!(after.controls, [controls, vec![event_loss]].concat());
            assert!(after.index.iter().all(|block| block.last_hour <= 4));
        }
        assert_eq!(recorded_frames(&saved), [(9, 2), (9, 3), (9, 4)]);
        assert_eq!(
            read_saved(&campaign).unwrap().segments.get(&1),
            Some(&(bytes.len() as u64))
        );
        let added = fs::read(&saved).unwrap();
        assert_eq!(save().0, Some(saved.clone()));
        let output = capture.take();
        let headers: Vec<_> = output
            .lines()
            .filter(|line| line.starts_with("finalize_s="))
            .collect();
        assert_eq!(headers.len(), 1, "{output}");
        assert!(
            headers[0].ends_with(&format!("spool_bytes={spool_bytes}")),
            "{output}"
        );
        assert_eq!(fs::read(&saved).unwrap(), added);
        discard_checkpoints(&fixture.session, Fixture::name()).unwrap();
        discard_unsaved(&fixture.session, Fixture::name()).unwrap();
        // Off rolls back to before the first checkpoint; on keeps what it published.
        if auto_save {
            assert_eq!(fs::read(&saved).unwrap(), added);
        } else {
            assert!(!saved.exists());
        }
        assert!(segment_paths(&campaign).unwrap().is_empty());
        assert!(!campaign.join(CHECKPOINT_FILE).exists());
    }
}

#[test]
fn a_later_failure_witness_keeps_committed_clockless_losses_of_a_saved_load() {
    for auto_save in [false, true] {
        for failed_here in [false, true] {
            let fixture = Fixture::new(&format!(
                "checkpoint-committed-loss-before-failure-{auto_save}-{failed_here}"
            ));
            let (checkpoint, session) = checkpointed_load(&fixture, auto_save, &[19]);
            let campaign = fixture.campaign(&fixture.session);
            let branch = held_recording(&checkpoint.recording)
                .unwrap()
                .branches
                .iter()
                .find(|branch| branch.id == 9)
                .unwrap()
                .clone();
            let lost = Gap {
                branch: 9,
                first_hour: 5,
                last_hour: 7,
                reason: "spool write failed".into(),
            };
            let bytes = write_tail(
                &campaign,
                &[
                    session,
                    Record::BranchStart(branch),
                    Record::Gap(lost.clone()),
                ],
            );
            let mut witness = crate::test_support::allocation_witness(bytes.len() as u64);
            witness.committed_segment = Some(1);
            witness.branch = None;
            witness.load_signal = None;
            witness.failed_hour = None;
            if !failed_here {
                witness.campaign_id = Some(*b"31b7cc84-3028-4f65-8e0d-adf2f37ca1a6");
            }
            fs::write(fixture.session.join(WITNESS_FILE_NAME), witness.encode()).unwrap();
            let save = || {
                save_campaign(
                    &fixture.session,
                    &fixture.output,
                    Fixture::name(),
                    !auto_save,
                    &|| Ok(()),
                    &mut Ahead::default(),
                    fixture.naming(),
                )
                .unwrap()
            };
            let (saved, omitted) = save();
            assert!(omitted.is_empty());
            let saved = saved.unwrap();
            let recording = fs::read(&saved).unwrap();
            let replay = Replay::open(recording.clone()).unwrap();
            assert_eq!(replay.gaps, [lost]);
            assert!(replay.controls.iter().any(|control| {
                matches!(
                    control,
                    ControlRecord::Partial {
                        failed_hour: None,
                        ..
                    }
                )
            }));
            assert_eq!(recorded_frames(&saved), [(9, 2), (9, 3), (9, 4)]);
            assert_eq!(save().0, Some(saved.clone()));
            assert_eq!(fs::read(saved).unwrap(), recording);
        }
    }
}

#[test]
fn a_new_branch_lost_before_its_first_frame_is_added_only_with_admitted_clock_evidence() {
    for auto_save in [false, true] {
        for (clock, admitted) in [(true, true), (true, false), (false, true)] {
            let fixture = Fixture::new(&format!(
                "checkpoint-explicit-new-branch-{auto_save}-{clock}-{admitted}"
            ));
            let loads: &[i64] = if admitted { &[19, 21] } else { &[19] };
            let (checkpoint, session) = checkpointed_load(&fixture, auto_save, loads);
            let campaign = fixture.campaign(&fixture.session);
            let branch = BranchStart {
                id: 10,
                session: 7,
                loaded_hour: Some(4),
                first_hour: 5,
                parent: Parentage::Unknown,
                reason: replay_frame::branches::BranchReason::LoadSignal,
            };
            let lost = Gap {
                branch: 10,
                first_hour: 5,
                last_hour: 5,
                reason: "spool write failed".into(),
            };
            let witnessed = ControlRecord::ClockWitness {
                session: 7,
                qpc: 10_005,
                hour: 5,
                load_signal: Some(21),
            };
            let mut records = vec![
                session,
                Record::BranchStart(branch.clone()),
                Record::Gap(lost.clone()),
            ];
            if clock {
                records.push(Record::Control(witnessed.clone()));
            }
            let bytes = write_tail(&campaign, &records);
            let recording = fs::read(&checkpoint.recording).unwrap();
            let before = read_saved(&campaign).unwrap().segments;
            for _ in 0..2 {
                let (saved, omitted) = save_campaign(
                    &fixture.session,
                    &fixture.output,
                    Fixture::name(),
                    !auto_save,
                    &|| Ok(()),
                    &mut Ahead::default(),
                    fixture.naming(),
                )
                .unwrap();
                let saved = saved.unwrap();
                assert_eq!(recorded_frames(&saved), [(9, 2), (9, 3), (9, 4)]);
                let after = held_recording(&saved).unwrap();
                if clock && admitted {
                    assert!(omitted.is_empty());
                    assert_eq!(after.branches.last(), Some(&branch));
                    assert_eq!(after.gaps.last(), Some(&lost));
                    assert_eq!(after.controls.last(), Some(&witnessed));
                    assert_eq!(
                        read_saved(&campaign).unwrap().segments.get(&1),
                        Some(&(bytes.len() as u64))
                    );
                } else {
                    // No admitted load names the branch: nothing is guessed, and the spool waits.
                    assert_eq!(saved, checkpoint.recording);
                    assert_eq!(
                        omitted
                            .iter()
                            .map(|omission| (omission.load, omission.hours))
                            .collect::<Vec<_>>(),
                        [(clock.then_some(21), 0)]
                    );
                    assert!(!after.branches.iter().any(|known| known.id == 10));
                    drop(after);
                    assert_eq!(fs::read(&saved).unwrap(), recording);
                    assert_eq!(read_saved(&campaign).unwrap().segments, before);
                }
            }
            assert_eq!(fs::read(campaign.join("spool.0001.h4s")).unwrap(), bytes);
        }
    }
}

#[test]
fn rebinding_published_segments_to_an_unsaved_copy_does_not_make_their_hours_wait_again() {
    let fixture = Fixture::new("checkpoint-rebind-published-coverage");
    let campaign = fixture.campaign(&fixture.session);
    let first = write_load_segment(&campaign, 0, 9, 2..=4, 19);
    fs::write(campaign.join("spool.0001.h4s"), []).unwrap();
    let checkpoint = fixture.checkpoint(0, true, &mut Ahead::default());
    assert_eq!(checkpoint.deleted, 1);
    assert!(!first.exists());
    assert_eq!(read_saved(&campaign).unwrap().covered[&0].1, 3);
    let published = fs::read(&checkpoint.recording).unwrap();
    let _lock = NamesLock::wait(&fixture.output).unwrap();
    let replay = held_recording(&checkpoint.recording).unwrap();
    let staged = replay::publication::unsaved_path(&fixture.output, Fixture::name());
    stage_copy(&replay, &staged).unwrap();
    let saved = verified_saved(&campaign, &fixture.output, Some(&staged), Some(&replay))
        .unwrap()
        .unwrap();
    assert_eq!(saved.replay, staged);
    assert!(saved.segments.contains_key(&0));
    assert!(saved.covered.is_empty());
    assert_eq!(unsaved(&fixture.session, Fixture::name()).unwrap(), None);
    assert_eq!(fs::read(&checkpoint.recording).unwrap(), published);
}

#[test]
fn checkpoint_only_spools_are_retained_when_the_saved_revision_is_not_an_ancestor() {
    let fixture = Fixture::new("checkpoint-unproven-ancestor");
    begin_admissions(&fixture.session, &fixture.output).unwrap();
    Admissions::default()
        .admit(&fixture.session, &fixture.output, Fixture::name(), 19)
        .unwrap();
    let campaign = fixture.campaign(&fixture.session);
    let first = write_load_segment(&campaign, 0, 9, 2..=4, 19);
    let recovered = replay::spool::recover_prefix(&fs::read(&first).unwrap());
    let tail = campaign.join("spool.0001.h4s");
    let context: Vec<_> = recovered
        .records
        .iter()
        .filter(|record| !matches!(record, Record::Frame(_)))
        .flat_map(|record| replay::spool::encode_record(record).unwrap())
        .collect();
    fs::write(&tail, &context).unwrap();
    let checkpoint = fixture.checkpoint(0, false, &mut Ahead::default());
    assert_eq!(checkpoint.deleted, 1);
    write_session_end(
        &fixture.session,
        &fixture.output,
        "completed.json",
        &json!({"waiting": []}),
    )
    .unwrap();
    let bookkeeping = fs::read(campaign.join(SAVED_FILE)).unwrap();
    let replacement =
        Replay::from_records(ALLOCATION_CAMPAIGN.to_vec(), &[allocation_records()]).unwrap();
    assert!(
        !replacement
            .contains_revision(read_saved(&campaign).unwrap().revision)
            .unwrap()
    );
    fs::write(&checkpoint.recording, replacement.bytes()).unwrap();
    let error = save_campaign(
        &fixture.session,
        &fixture.output,
        Fixture::name(),
        true,
        &|| Ok(()),
        &mut Ahead::default(),
        fixture.naming(),
    )
    .unwrap_err();
    assert!(error.contains("no complete campaign identity"), "{error}");
    assert_eq!(fs::read(&tail).unwrap(), context);
    assert_eq!(fs::read(campaign.join(SAVED_FILE)).unwrap(), bookkeeping);
    assert_eq!(
        fs::read(&checkpoint.recording).unwrap(),
        replacement.bytes()
    );
}

#[test]
fn a_discard_from_the_waiting_list_drops_checkpointed_hours_with_the_unsaved_recording() {
    let fixture = Fixture::new("checkpoint-waiting-discard");
    let campaign = fixture.campaign(&fixture.session);
    write_load_segment(&campaign, 0, 9, 2..=4, 19);
    write_load_segment(&campaign, 1, 10, 5..=6, 21);
    fixture.checkpoint(0, false, &mut Ahead::default());
    write_session_end(
        &fixture.session,
        &fixture.output,
        "waiting.json",
        &json!({"waiting":[]}),
    )
    .unwrap();
    assert!(read_checkpointed(&campaign).unwrap().is_none());
    discard_checkpoints(&fixture.session, Fixture::name()).unwrap();
    discard_unsaved_recording(&fixture.output, Fixture::name()).unwrap();
    discard_unsaved(&fixture.session, Fixture::name()).unwrap();
    assert!(segment_paths(&campaign).unwrap().is_empty());
    assert!(!campaign.join(CHECKPOINT_FILE).exists());
    assert_eq!(unsaved(&fixture.session, Fixture::name()).unwrap(), None);
}

#[test]
fn without_auto_save_a_checkpoint_of_a_saved_recording_adds_to_a_copy_that_holds_it() {
    let fixture = Fixture::new("checkpoint-unsaved-copy");
    let earlier = fixture.root.join("earlier");
    write_load_segment(&fixture.campaign(&earlier), 0, 9, 2..=4, 19);
    let recording = finalize_session(&earlier, &fixture.output)
        .unwrap()
        .remove(0);
    let before = fs::read(&recording).unwrap();
    let campaign = fixture.campaign(&fixture.session);
    write_load_segment(&campaign, 0, 10, 5..=6, 21);
    write_load_segment(&campaign, 1, 11, 7..=8, 23);
    let done = fixture.checkpoint(0, false, &mut Ahead::default());
    let staged = replay::publication::unsaved_path(&fixture.output, Fixture::name());
    assert_eq!(done.recording, staged);
    assert_eq!(
        fs::read(&recording).unwrap(),
        before,
        "the saved recording waits"
    );
    assert!(held_recording(&staged).unwrap().holds(&recording).unwrap());
    // Save publishes what waits, the spool's hours too.
    let saved = finalize_session(&fixture.session, &fixture.output).unwrap();
    assert_eq!(saved.len(), 1);
    assert!(!staged.exists());
    assert_eq!(
        recorded_frames(&saved[0]),
        [(9, 2), (9, 3), (9, 4), (10, 5), (10, 6), (11, 7), (11, 8)]
    );
}

fn waiting_copy(fixture: &Fixture) -> Vec<u8> {
    let earlier = fixture.root.join("earlier");
    write_load_segment(&fixture.campaign(&earlier), 0, 8, 1..=1, 17);
    let path = save_campaign(
        &earlier,
        &fixture.output,
        Fixture::name(),
        true,
        &|| Ok(()),
        &mut Ahead::default(),
        fixture.naming(),
    )
    .unwrap()
    .0
    .unwrap();
    write_session_end(&earlier, &fixture.output, "completed.json", &json!({})).unwrap();
    fs::read(path).unwrap()
}

struct StepHook;

impl StepHook {
    fn new(hook: impl FnMut(CheckpointStep, &Path) -> std::io::Result<()> + 'static) -> Self {
        CHECKPOINT_STEP.with(|slot| *slot.borrow_mut() = Some(Box::new(hook)));
        Self
    }

    fn fail(step: CheckpointStep) -> Self {
        Self::new(move |found, _| {
            if found == step {
                Err(std::io::ErrorKind::Interrupted.into())
            } else {
                Ok(())
            }
        })
    }
}

impl Drop for StepHook {
    fn drop(&mut self) {
        CHECKPOINT_STEP.with(|slot| *slot.borrow_mut() = None);
    }
}

#[test]
fn interruption_at_every_unsaved_checkpoint_barrier_preserves_the_original_discard_point() {
    for existing in [false, true] {
        for step in [
            CheckpointStep::Preparing,
            CheckpointStep::Prepared,
            CheckpointStep::Written,
            CheckpointStep::Verified,
            CheckpointStep::Journaled,
            CheckpointStep::Handed,
        ] {
            let fixture = Fixture::new(&format!("checkpoint-matrix-{existing}-{step:?}"));
            let original = existing.then(|| waiting_copy(&fixture));
            let campaign = fixture.campaign(&fixture.session);
            let spool = write_load_segment(&campaign, 0, 9, 2..=4, 19);
            write_load_segment(&campaign, 1, 10, 5..=6, 21);
            let bytes = fs::read(&spool).unwrap();
            let fault = StepHook::fail(step);
            assert!(
                checkpoint_campaign(
                    &fixture.session,
                    &fixture.output,
                    Fixture::name(),
                    0,
                    false,
                    &mut Ahead::default(),
                    fixture.naming(),
                    &|| Ok(())
                )
                .is_err()
            );
            drop(fault);
            assert_eq!(fs::read(&spool).unwrap(), bytes);
            discard_unsaved(&fixture.session, Fixture::name()).unwrap();
            let staged = replay::publication::unsaved_path(&fixture.output, Fixture::name());
            match original {
                Some(bytes) => assert_eq!(fs::read(staged).unwrap(), bytes),
                None => assert!(!staged.exists()),
            }
        }
    }
}

#[test]
fn an_interrupted_later_checkpoint_is_reconciled_by_retry_end_or_next_capture_start() {
    for look in 0..3 {
        for step in [
            CheckpointStep::Prepared,
            CheckpointStep::Written,
            CheckpointStep::Verified,
            CheckpointStep::Journaled,
            CheckpointStep::Handed,
        ] {
            let fixture = Fixture::new(&format!("checkpoint-next-look-{look}-{step:?}"));
            let original_bytes = waiting_copy(&fixture);
            let original = replay::publication::unsaved_path(&fixture.output, Fixture::name());
            let original_revision = Replay::open(original_bytes).unwrap().revision();
            let campaign = fixture.campaign(&fixture.session);
            write_load_segment(&campaign, 0, 9, 2..=4, 19);
            write_load_segment(&campaign, 1, 10, 5..=6, 21);
            fixture.checkpoint(0, false, &mut Ahead::default());
            write_load_segment(&campaign, 2, 11, 7..=8, 23);
            let fault = StepHook::fail(step);
            assert!(
                checkpoint_campaign(
                    &fixture.session,
                    &fixture.output,
                    Fixture::name(),
                    1,
                    false,
                    &mut Ahead::default(),
                    fixture.naming(),
                    &|| Ok(())
                )
                .is_err()
            );
            drop(fault);
            match look {
                0 => {
                    fixture.checkpoint(1, false, &mut Ahead::default());
                }
                1 => {
                    finalize_session(&fixture.session, &fixture.output).unwrap();
                }
                _ => {
                    reconcile_session_checkpoints(&fixture.session, &fixture.output).unwrap();
                }
            }
            if look == 1 {
                assert!(read_checkpointed(&campaign).unwrap().is_none());
            } else {
                let journal = read_checkpointed(&campaign).unwrap().unwrap();
                assert_eq!(journal.before, Some(original_revision));
                assert!(!journal.pending);
                discard_checkpoints(&fixture.session, Fixture::name()).unwrap();
                assert_eq!(
                    held_recording(&original).unwrap().revision(),
                    original_revision
                );
            }
        }
    }
}

#[test]
fn switching_auto_save_off_rolls_back_only_the_new_unsaved_copy_and_discard_is_restartable() {
    for interrupt in [false, true] {
        let fixture = Fixture::new(&format!("checkpoint-toggle-{interrupt}"));
        let campaign = fixture.campaign(&fixture.session);
        write_load_segment(&campaign, 0, 9, 2..=4, 19);
        write_load_segment(&campaign, 1, 10, 5..=6, 21);
        let published = fixture.checkpoint(0, true, &mut Ahead::default()).recording;
        let before = fs::read(&published).unwrap();
        write_load_segment(&campaign, 2, 11, 7..=8, 23);
        let staged = fixture
            .checkpoint(1, false, &mut Ahead::default())
            .recording;
        let journal = read_checkpointed(&campaign).unwrap().unwrap();
        assert_eq!(journal.recording, staged);
        assert_eq!(journal.before, None);
        assert_eq!(
            unsaved(&fixture.session, Fixture::name())
                .unwrap()
                .unwrap()
                .hours,
            4
        );
        assert!(held_recording(&staged).unwrap().holds(&published).unwrap());
        if interrupt {
            let fault = StepHook::fail(CheckpointStep::Discarded);
            assert!(discard_checkpoints(&fixture.session, Fixture::name()).is_err());
            drop(fault);
        }
        discard_unsaved(&fixture.session, Fixture::name()).unwrap();
        assert_eq!(fs::read(&published).unwrap(), before);
        assert!(!staged.exists());
        assert!(!campaign.join(CHECKPOINT_FILE).exists());
    }
}

#[test]
fn a_left_out_load_keeps_only_its_own_segments_while_later_checkpoints_trim() {
    let fixture = Fixture::new("checkpoint-omitted-coverage");
    let campaign = fixture.campaign(&fixture.session);
    let omitted = write_load_segment(&campaign, 0, 9, 2..=4, 19);
    let first = write_load_segment(&campaign, 1, 10, 5..=6, 21);
    write_load_segment(&campaign, 2, 11, 7..=8, 23);
    begin_admissions(&fixture.session, &fixture.output).unwrap();
    let mut admissions = Admissions::default();
    admissions
        .admit(&fixture.session, &fixture.output, Fixture::name(), 21)
        .unwrap();
    admissions
        .admit(&fixture.session, &fixture.output, Fixture::name(), 23)
        .unwrap();
    let first_save = fixture.checkpoint(1, true, &mut Ahead::default());
    assert!(omitted.exists());
    assert!(!first.exists());
    assert_eq!(first_save.deleted, 1);
    write_load_segment(&campaign, 3, 12, 9..=10, 25);
    let next = fixture.checkpoint(2, true, &mut Ahead::default());
    assert!(omitted.exists());
    assert!(!campaign.join("spool.0002.h4s").exists());
    assert_eq!(next.deleted, 1);
    let saved = read_saved(&campaign).unwrap();
    assert!(!saved.segments.contains_key(&0));
    assert_eq!(
        saved.omitted,
        [(Some(19), 3)],
        "retained omissions are not accumulated twice"
    );
    assert_eq!(
        recorded_frames(&next.recording),
        [(10, 5), (10, 6), (11, 7), (11, 8)]
    );
}

#[test]
fn first_publication_reads_disk_blocks_before_callback_bookkeeping_or_trimming() {
    use std::io::{Seek, Write};
    let fixture = Fixture::new("checkpoint-first-readback");
    let campaign = fixture.campaign(&fixture.session);
    let first = write_load_segment(&campaign, 0, 9, 2..=4, 19);
    write_load_segment(&campaign, 1, 10, 5..=6, 21);
    let handed = Arc::new(Mutex::new(false));
    let seen = Arc::clone(&handed);
    let mut ahead = Ahead::default();
    ahead.on_committed(Box::new(move |_, _, _| *seen.lock().unwrap() = true));
    let fault = StepHook::new(|step, path| {
        if step == CheckpointStep::Written {
            let replay = Replay::open_path(path)?;
            let at = replay.index[0].offset;
            drop(replay);
            let mut file = fs::OpenOptions::new().write(true).open(path)?;
            file.seek(std::io::SeekFrom::Start(at))?;
            file.write_all(b"broken block")?;
            file.sync_all()?;
        }
        Ok(())
    });
    assert!(
        checkpoint_campaign(
            &fixture.session,
            &fixture.output,
            Fixture::name(),
            0,
            true,
            &mut ahead,
            fixture.naming(),
            &|| Ok(())
        )
        .is_err()
    );
    drop(fault);
    assert!(!*handed.lock().unwrap());
    assert!(first.exists());
    assert!(!campaign.join(SAVED_FILE).exists());
    assert!(read_checkpointed(&campaign).unwrap().is_none());
    discard_checkpoints(&fixture.session, Fixture::name()).unwrap();
}

#[test]
fn interrupted_spool_deletion_retries_only_segments_the_verified_revision_holds() {
    let fixture = Fixture::new("checkpoint-trim-restart");
    let campaign = fixture.campaign(&fixture.session);
    for number in 0..4 {
        write_load_segment(
            &campaign,
            number,
            9 + u64::from(number),
            2 + number as i32..=2 + number as i32,
            19 + i64::from(number),
        );
    }
    let fault = StepHook::fail(CheckpointStep::Trimmed);
    assert!(
        checkpoint_campaign(
            &fixture.session,
            &fixture.output,
            Fixture::name(),
            2,
            false,
            &mut Ahead::default(),
            fixture.naming(),
            &|| Ok(())
        )
        .is_err()
    );
    drop(fault);
    assert!(!campaign.join("spool.0000.h4s").exists());
    assert_eq!(drop_checkpointed(&campaign).unwrap(), 2);
    assert!(campaign.join("spool.0003.h4s").exists());
    assert_eq!(
        unsaved(&fixture.session, Fixture::name())
            .unwrap()
            .unwrap()
            .hours,
        4
    );
}

#[test]
fn a_stale_campaign_copy_is_replaced_under_the_naming_lock_not_left_per_process() {
    let fixture = Fixture::new("checkpoint-stale-copy");
    let source = fixture.root.join("earlier");
    write_load_segment(&fixture.campaign(&source), 0, 9, 2..=4, 19);
    let recording = finalize_session(&source, &fixture.output)
        .unwrap()
        .remove(0);
    let original = held_recording(&recording).unwrap();
    let staged = replay::publication::unsaved_path(&fixture.output, Fixture::name());
    replay::publication::create_unsaved_folder(&fixture.output).unwrap();
    let temporary = stage_copy_path(&staged);
    let _lock = NamesLock::wait(&fixture.output).unwrap();
    for length in [1, original.saved_length() / 2, original.saved_length() - 1] {
        fs::write(&temporary, vec![0; length as usize]).unwrap();
        stage_copy(&original, &staged).unwrap();
        assert!(!temporary.exists());
        assert!(held_recording(&staged).unwrap().holds(&recording).unwrap());
        replay::publication::posix_delete(&staged).unwrap();
    }
    fs::create_dir(&temporary).unwrap();
    assert!(
        stage_copy(&original, &staged)
            .unwrap_err()
            .contains("not an ordinary file")
    );
    assert!(temporary.is_dir());
    assert!(!staged.exists());
}

#[test]
fn a_segment_with_both_admitted_and_left_out_hours_stays_while_other_segments_are_trimmed() {
    let fixture = Fixture::new("checkpoint-mixed-coverage");
    let campaign = fixture.campaign(&fixture.session);
    let omitted = write_load_segment(&campaign, 0, 9, 2..=4, 19);
    let admitted = write_load_segment(&campaign, 1, 10, 5..=6, 21);
    let mut mixed = fs::read(&omitted).unwrap();
    mixed.extend(fs::read(&admitted).unwrap());
    fs::write(&omitted, &mixed).unwrap();
    write_load_segment(&campaign, 1, 11, 7..=8, 23);
    write_load_segment(&campaign, 2, 12, 9..=10, 25);
    begin_admissions(&fixture.session, &fixture.output).unwrap();
    let mut admissions = Admissions::default();
    for load in [21, 23] {
        admissions
            .admit(&fixture.session, &fixture.output, Fixture::name(), load)
            .unwrap();
    }
    fixture.checkpoint(0, true, &mut Ahead::default());
    let done = fixture.checkpoint(1, true, &mut Ahead::default());
    assert_eq!(done.deleted, 1);
    assert_eq!(fs::read(&omitted).unwrap(), mixed);
    let saved = read_saved(&campaign).unwrap();
    assert!(!saved.covered.contains_key(&0));
    assert!(saved.covered.contains_key(&1));
    assert!(read_checkpointed(&campaign).unwrap().is_none());
    assert_eq!(
        recorded_frames(&done.recording),
        [(10, 5), (10, 6), (11, 7), (11, 8)]
    );
}

#[test]
fn malformed_rollback_information_is_refused_never_reset_to_the_changed_revision() {
    for field in ["before", "after", "source", "intended"] {
        let fixture = Fixture::new(&format!("checkpoint-missing-{field}"));
        let earlier = fixture.root.join("earlier");
        write_load_segment(&fixture.campaign(&earlier), 0, 8, 1..=1, 17);
        finalize_session(&earlier, &fixture.output).unwrap();
        let campaign = fixture.campaign(&fixture.session);
        write_load_segment(&campaign, 0, 9, 2..=4, 19);
        write_load_segment(&campaign, 1, 10, 5..=6, 21);
        let recording = fixture
            .checkpoint(0, false, &mut Ahead::default())
            .recording;
        let bytes = fs::read(&recording).unwrap();
        let path = campaign.join(CHECKPOINT_FILE);
        let mut value: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        value.as_object_mut().unwrap().remove(field);
        fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(discard_checkpoints(&fixture.session, Fixture::name()).is_err());
        assert!(
            checkpoint_campaign(
                &fixture.session,
                &fixture.output,
                Fixture::name(),
                1,
                false,
                &mut Ahead::default(),
                fixture.naming(),
                &|| Ok(())
            )
            .is_err()
        );
        assert_eq!(fs::read(&recording).unwrap(), bytes);
        assert!(campaign.join("spool.0001.h4s").exists());
    }
}

#[test]
fn pending_first_staging_copy_is_cleaned_when_the_session_is_reconciled() {
    let fixture = Fixture::new("checkpoint-copy-recovery");
    let earlier = fixture.root.join("earlier");
    write_load_segment(&fixture.campaign(&earlier), 0, 9, 2..=4, 19);
    let recording = finalize_session(&earlier, &fixture.output)
        .unwrap()
        .remove(0);
    let original = fs::read(&recording).unwrap();
    let campaign = fixture.campaign(&fixture.session);
    write_load_segment(&campaign, 0, 10, 5..=6, 21);
    let staged = replay::publication::unsaved_path(&fixture.output, Fixture::name());
    replay::publication::create_unsaved_folder(&fixture.output).unwrap();
    {
        let _lock = NamesLock::wait(&fixture.output).unwrap();
        begin_checkpoint(
            &campaign,
            &staged,
            None,
            &held_recording(&recording).unwrap(),
        )
        .unwrap();
    }
    let temporary = stage_copy_path(&staged);
    fs::write(&temporary, &original[..original.len() / 2]).unwrap();
    reconcile_session_checkpoints(&fixture.session, &fixture.output).unwrap();
    assert!(!temporary.exists());
    assert!(!staged.exists());
    assert_eq!(fs::read(&recording).unwrap(), original);
    let journal = read_checkpointed(&campaign).unwrap().unwrap();
    assert!(!journal.pending);
    assert_eq!(journal.before, None);
    assert_eq!(journal.after, None);
    let done = fixture.checkpoint(0, false, &mut Ahead::default());
    assert_eq!(done.recording, staged);
    discard_checkpoints(&fixture.session, Fixture::name()).unwrap();
    assert_eq!(fs::read(&recording).unwrap(), original);
    assert!(!staged.exists());
}

#[test]
fn an_explicit_save_publishes_its_checkpoints_before_a_new_unsaved_interval() {
    let fixture = Fixture::new("checkpoint-explicit-acceptance");
    let campaign = fixture.campaign(&fixture.session);
    write_load_segment(&campaign, 0, 9, 2..=4, 19);
    write_load_segment(&campaign, 1, 10, 5..=6, 21);
    fixture.checkpoint(0, false, &mut Ahead::default());
    let saved = save_campaign(
        &fixture.session,
        &fixture.output,
        Fixture::name(),
        false,
        &|| Ok(()),
        &mut Ahead::default(),
        fixture.naming(),
    )
    .unwrap()
    .0
    .unwrap();
    let accepted = fs::read(&saved).unwrap();
    assert!(read_checkpointed(&campaign).unwrap().is_none());
    write_load_segment(&campaign, 2, 11, 7..=8, 23);
    write_load_segment(&campaign, 3, 12, 9..=10, 25);
    fixture.checkpoint(2, false, &mut Ahead::default());
    discard_checkpoints(&fixture.session, Fixture::name()).unwrap();
    assert_eq!(fs::read(saved).unwrap(), accepted);
}

#[test]
fn a_later_load_decision_saves_only_new_branches_of_a_retained_mixed_segment() {
    for interrupt in [false, true] {
        let fixture = Fixture::new(&format!("checkpoint-mixed-later-decision-{interrupt}"));
        let campaign = fixture.campaign(&fixture.session);
        let mixed = write_load_segment(&campaign, 0, 9, 2..=4, 19);
        let second = write_load_segment(&campaign, 1, 10, 5..=6, 21);
        let mut bytes = fs::read(&mixed).unwrap();
        bytes.extend(fs::read(&second).unwrap());
        fs::write(&mixed, &bytes).unwrap();
        write_load_segment(&campaign, 1, 11, 7..=8, 23);
        write_load_segment(&campaign, 2, 12, 9..=10, 25);
        begin_admissions(&fixture.session, &fixture.output).unwrap();
        let mut admissions = Admissions::default();
        for load in [21, 23] {
            admissions
                .admit(&fixture.session, &fixture.output, Fixture::name(), load)
                .unwrap();
        }
        fixture.checkpoint(1, true, &mut Ahead::default());
        assert!(mixed.exists());
        admissions
            .admit(&fixture.session, &fixture.output, Fixture::name(), 19)
            .unwrap();
        if interrupt {
            let fault = StepHook::fail(CheckpointStep::Verified);
            assert!(
                checkpoint_campaign(
                    &fixture.session,
                    &fixture.output,
                    Fixture::name(),
                    1,
                    true,
                    &mut Ahead::default(),
                    fixture.naming(),
                    &|| Ok(()),
                )
                .is_err()
            );
            drop(fault);
        }
        let done = fixture.checkpoint(1, true, &mut Ahead::default());
        assert_eq!(done.deleted, 1);
        assert!(!mixed.exists());
        let mut hours = recorded_frames(&done.recording);
        hours.sort_unstable();
        assert_eq!(
            hours,
            [(9, 2), (9, 3), (9, 4), (10, 5), (10, 6), (11, 7), (11, 8)]
        );
    }
}

#[test]
fn interrupted_discard_keeps_its_intent_until_no_spool_can_resurrect_its_hours() {
    let fixture = Fixture::new("checkpoint-discard-spool-crash-unsaved");
    let earlier = fixture.root.join("earlier");
    write_load_segment(&fixture.campaign(&earlier), 0, 8, 1..=1, 17);
    let recording = finalize_session(&earlier, &fixture.output)
        .unwrap()
        .remove(0);
    let before = fs::read(&recording).unwrap();
    let campaign = fixture.campaign(&fixture.session);
    write_load_segment(&campaign, 0, 9, 2..=4, 19);
    write_load_segment(&campaign, 1, 10, 5..=6, 21);
    write_load_segment(&campaign, 2, 11, 7..=8, 23);
    fixture.checkpoint(0, false, &mut Ahead::default());
    let fault = StepHook::fail(CheckpointStep::Trimmed);
    assert!(discard_checkpoints(&fixture.session, Fixture::name()).is_err());
    drop(fault);
    assert!(read_checkpointed(&campaign).unwrap().unwrap().discarding);
    assert!(!segment_paths(&campaign).unwrap().is_empty());
    reconcile_session_checkpoints(&fixture.session, &fixture.output).unwrap();
    assert_eq!(fs::read(&recording).unwrap(), before);
    assert!(segment_paths(&campaign).unwrap().is_empty());
    assert!(read_checkpointed(&campaign).unwrap().is_none());
}

fn other_recording(campaign: &[u8]) -> Replay {
    Replay::from_records(campaign.to_vec(), &[vec![allocation_records().remove(0)]]).unwrap()
}

#[test]
fn pending_publication_never_deletes_a_recording_renamed_into_its_destination() {
    for step in [
        CheckpointStep::Prepared,
        CheckpointStep::Written,
        CheckpointStep::Verified,
    ] {
        for same_campaign in [false, true] {
            let fixture = Fixture::new(&format!(
                "checkpoint-replaced-unsaved-{step:?}-{same_campaign}"
            ));
            let campaign = fixture.campaign(&fixture.session);
            let spool = write_load_segment(&campaign, 0, 9, 2..=4, 19);
            write_load_segment(&campaign, 1, 10, 5..=6, 21);
            let spool_bytes = fs::read(&spool).unwrap();
            let fault = StepHook::fail(step);
            assert!(
                checkpoint_campaign(
                    &fixture.session,
                    &fixture.output,
                    Fixture::name(),
                    0,
                    false,
                    &mut Ahead::default(),
                    fixture.naming(),
                    &|| Ok(()),
                )
                .is_err()
            );
            drop(fault);
            let journal = read_checkpointed(&campaign).unwrap().unwrap();
            assert_eq!(journal.campaign.as_bytes(), ALLOCATION_CAMPAIGN);
            let file = &journal;
            assert!(file.pending);
            assert_eq!(file.after, None);
            let destination = &file.recording;
            if destination.exists() {
                assert_eq!(
                    held_recording(destination).unwrap().revision(),
                    file.intended
                );
                replay::publication::rename_new(
                    destination,
                    &fixture.root.join("interrupted.battleplan"),
                )
                .unwrap();
            }
            let other = other_recording(if same_campaign {
                ALLOCATION_CAMPAIGN
            } else {
                b"00000000-0000-4000-8000-000000000000"
            });
            assert_ne!(other.revision(), file.intended);
            let source = fixture.root.join("unrelated.battleplan");
            fs::write(&source, other.bytes()).unwrap();
            replay::publication::rename_new(&source, destination).unwrap();
            let journal_bytes = fs::read(campaign.join(CHECKPOINT_FILE)).unwrap();
            for error in [
                reconcile_session_checkpoints(&fixture.session, &fixture.output).unwrap_err(),
                discard_checkpoints(&fixture.session, Fixture::name()).unwrap_err(),
                discard_unsaved(&fixture.session, Fixture::name()).unwrap_err(),
            ] {
                assert!(error.contains("checkpoint recording identity"), "{error}");
                assert!(error.contains(Fixture::name()), "{error}");
            }
            assert_eq!(drop_checkpointed(&campaign).unwrap(), 0);
            assert_eq!(fs::read(destination).unwrap(), other.bytes());
            assert_eq!(fs::read(&spool).unwrap(), spool_bytes);
            assert_eq!(
                fs::read(campaign.join(CHECKPOINT_FILE)).unwrap(),
                journal_bytes
            );
        }
    }
}

#[test]
fn a_pending_append_refuses_even_a_newer_recording_of_the_same_campaign() {
    let fixture = Fixture::new("checkpoint-newer-append");
    let earlier = fixture.root.join("earlier");
    write_load_segment(&fixture.campaign(&earlier), 0, 8, 1..=1, 17);
    finalize_session(&earlier, &fixture.output).unwrap();
    let campaign = fixture.campaign(&fixture.session);
    let spool = write_load_segment(&campaign, 0, 9, 2..=4, 19);
    let spool_bytes = fs::read(&spool).unwrap();
    let fault = StepHook::fail(CheckpointStep::Written);
    assert!(
        checkpoint_campaign(
            &fixture.session,
            &fixture.output,
            Fixture::name(),
            0,
            false,
            &mut Ahead::default(),
            fixture.naming(),
            &|| Ok(()),
        )
        .is_err()
    );
    drop(fault);
    let journal = read_checkpointed(&campaign).unwrap().unwrap();
    let journal_bytes = fs::read(campaign.join(CHECKPOINT_FILE)).unwrap();
    assert_ne!(Some(journal.intended), journal.after);
    let external = fixture.root.join("external");
    write_load_segment(&fixture.campaign(&external), 0, 12, 9..=10, 25);
    let changed = save_campaign(
        &external,
        &fixture.output,
        Fixture::name(),
        true,
        &|| Ok(()),
        &mut Ahead::default(),
        fixture.naming(),
    )
    .unwrap()
    .0
    .unwrap();
    let bytes = fs::read(&changed).unwrap();
    for error in [
        reconcile_session_checkpoints(&fixture.session, &fixture.output).unwrap_err(),
        discard_checkpoints(&fixture.session, Fixture::name()).unwrap_err(),
    ] {
        assert!(error.contains("checkpoint recording identity"), "{error}");
    }
    assert_eq!(fs::read(&changed).unwrap(), bytes);
    assert_eq!(fs::read(&spool).unwrap(), spool_bytes);
    assert_eq!(
        fs::read(campaign.join(CHECKPOINT_FILE)).unwrap(),
        journal_bytes
    );
}

#[test]
fn unresolved_unsaved_checkpoint_identity_keeps_the_copy_published_hours_and_spool() {
    for problem in ["missing", "ambiguous", "campaign", "revision"] {
        let fixture = Fixture::new(&format!("checkpoint-unresolved-{problem}"));
        let campaign = fixture.campaign(&fixture.session);
        write_load_segment(&campaign, 0, 9, 2..=4, 19);
        write_load_segment(&campaign, 1, 10, 5..=6, 21);
        let visible = fixture.checkpoint(0, true, &mut Ahead::default()).recording;
        let saved_bytes = fs::read(&visible).unwrap();
        write_load_segment(&campaign, 2, 11, 7..=8, 23);
        let staged = fixture
            .checkpoint(1, false, &mut Ahead::default())
            .recording;
        let staged_bytes = fs::read(&staged).unwrap();
        let copy = if problem == "ambiguous" {
            staged.parent().unwrap().join("copy.battleplan")
        } else {
            fixture.root.join("moved.battleplan")
        };
        fs::copy(&staged, &copy).unwrap();
        let replacement = match problem {
            "missing" => {
                replay::publication::posix_delete(&staged).unwrap();
                None
            }
            "campaign" | "revision" => {
                let other = other_recording(if problem == "campaign" {
                    b"00000000-0000-4000-8000-000000000000"
                } else {
                    ALLOCATION_CAMPAIGN
                });
                fs::write(&staged, other.bytes()).unwrap();
                Some(other.bytes().to_vec())
            }
            _ => Some(staged_bytes.clone()),
        };
        let journal_bytes = fs::read(campaign.join(CHECKPOINT_FILE)).unwrap();
        let spools: Vec<_> = segment_paths(&campaign)
            .unwrap()
            .into_iter()
            .map(|(_, path)| {
                let bytes = fs::read(&path).unwrap();
                (path, bytes)
            })
            .collect();
        for error in [
            reconcile_session_checkpoints(&fixture.session, &fixture.output).unwrap_err(),
            discard_checkpoints(&fixture.session, Fixture::name()).unwrap_err(),
            discard_unsaved(&fixture.session, Fixture::name()).unwrap_err(),
            drop_checkpointed(&campaign).unwrap_err(),
        ] {
            assert!(error.contains("checkpoint recording identity"), "{error}");
            assert!(error.contains(Fixture::name()), "{error}");
        }
        assert_eq!(fs::read(copy).unwrap(), staged_bytes);
        assert_eq!(fs::read(&visible).unwrap(), saved_bytes);
        if let Some(replacement) = replacement {
            assert_eq!(fs::read(&staged).unwrap(), replacement);
        }
        assert_eq!(
            fs::read(campaign.join(CHECKPOINT_FILE)).unwrap(),
            journal_bytes
        );
        for (path, bytes) in spools {
            assert_eq!(fs::read(path).unwrap(), bytes);
        }
    }
}

#[test]
fn pending_staging_accepts_the_source_revision_before_the_intended_append_is_written() {
    let fixture = Fixture::new("checkpoint-source-before-append");
    let earlier = fixture.root.join("earlier");
    write_load_segment(&fixture.campaign(&earlier), 0, 8, 1..=1, 17);
    let visible = finalize_session(&earlier, &fixture.output)
        .unwrap()
        .remove(0);
    let before = fs::read(&visible).unwrap();
    let previous = held_recording(&visible).unwrap();
    let campaign = fixture.campaign(&fixture.session);
    let spool = write_load_segment(&campaign, 0, 9, 2..=4, 19);
    let spool_bytes = fs::read(&spool).unwrap();
    let staged = replay::publication::unsaved_path(&fixture.output, Fixture::name());
    {
        let _lock = NamesLock::wait(&fixture.output).unwrap();
        begin_checkpoint(&campaign, &staged, None, &previous).unwrap();
        stage_copy(&previous, &staged).unwrap();
        intend_checkpoint(
            &campaign,
            &staged,
            other_recording(ALLOCATION_CAMPAIGN).revision(),
        )
        .unwrap();
    }
    let journal = read_checkpointed(&campaign).unwrap().unwrap();
    assert_eq!(journal.source, previous.revision());
    assert_ne!(journal.source, journal.intended);
    reconcile_session_checkpoints(&fixture.session, &fixture.output).unwrap();
    assert!(!staged.exists());
    assert_eq!(fs::read(&visible).unwrap(), before);
    assert_eq!(fs::read(&spool).unwrap(), spool_bytes);
    assert!(!read_checkpointed(&campaign).unwrap().unwrap().pending);
}

#[test]
fn discarding_multiple_unsaved_checkpoints_restores_one_point_without_invalidating_readers() {
    let fixture = Fixture::new("checkpoint-one-unsaved-point");
    let before = waiting_copy(&fixture);
    let expected = Replay::open(before.clone()).unwrap().revision();
    let campaign = fixture.campaign(&fixture.session);
    write_load_segment(&campaign, 0, 9, 2..=4, 19);
    write_load_segment(&campaign, 1, 10, 5..=6, 21);
    let first = fixture.checkpoint(0, false, &mut Ahead::default());
    write_load_segment(&campaign, 2, 11, 7..=8, 23);
    fixture.checkpoint(1, false, &mut Ahead::default());
    let held = held_recording(&first.recording).unwrap();
    let hours: Vec<_> = held
        .index
        .iter()
        .flat_map(|block| held.read_block(block).unwrap())
        .collect();
    let journal = read_checkpointed(&campaign).unwrap().unwrap();
    assert_eq!(journal.before, Some(expected));
    assert_ne!(journal.after, journal.before);
    discard_unsaved(&fixture.session, Fixture::name()).unwrap();
    assert_eq!(fs::read(&first.recording).unwrap(), before);
    assert_eq!(
        held.index
            .iter()
            .flat_map(|block| held.read_block(block).unwrap())
            .collect::<Vec<_>>(),
        hours
    );
    assert!(segment_paths(&campaign).unwrap().is_empty());
}

#[test]
fn switching_auto_save_on_publishes_waiting_hours_and_a_later_off_interval_cannot_take_them_back() {
    for earlier_waiting in [false, true] {
        let fixture = Fixture::new(&format!("checkpoint-toggle-publish-{earlier_waiting}"));
        if earlier_waiting {
            waiting_copy(&fixture);
        }
        let campaign = fixture.campaign(&fixture.session);
        write_load_segment(&campaign, 0, 9, 2..=4, 19);
        write_load_segment(&campaign, 1, 10, 5..=6, 21);
        let staged = fixture
            .checkpoint(0, false, &mut Ahead::default())
            .recording;
        assert!(read_checkpointed(&campaign).unwrap().is_some());
        write_load_segment(&campaign, 2, 11, 7..=8, 23);
        let published = fixture.checkpoint(1, true, &mut Ahead::default()).recording;
        assert!(!staged.exists());
        assert!(read_checkpointed(&campaign).unwrap().is_none());
        let mut expected = vec![(9, 2), (9, 3), (9, 4), (10, 5), (10, 6)];
        if earlier_waiting {
            expected.insert(0, (8, 1));
        }
        assert_eq!(recorded_frames(&published), expected);
        let before = fs::read(&published).unwrap();
        write_load_segment(&campaign, 3, 12, 9..=10, 25);
        fixture.checkpoint(2, false, &mut Ahead::default());
        assert_eq!(read_checkpointed(&campaign).unwrap().unwrap().before, None);
        discard_unsaved(&fixture.session, Fixture::name()).unwrap();
        assert!(!staged.exists());
        assert_eq!(fs::read(&published).unwrap(), before);
    }
}

#[test]
fn a_toggle_interrupted_around_publication_never_rolls_back_published_hours() {
    for step in [CheckpointStep::Publishing, CheckpointStep::Published] {
        let fixture = Fixture::new(&format!("checkpoint-publish-crash-{step:?}"));
        let campaign = fixture.campaign(&fixture.session);
        write_load_segment(&campaign, 0, 9, 2..=4, 19);
        write_load_segment(&campaign, 1, 10, 5..=6, 21);
        let staged = fixture
            .checkpoint(0, false, &mut Ahead::default())
            .recording;
        let fault = StepHook::fail(step);
        assert!(
            checkpoint_campaign(
                &fixture.session,
                &fixture.output,
                Fixture::name(),
                1,
                true,
                &mut Ahead::default(),
                fixture.naming(),
                &|| Ok(())
            )
            .is_err()
        );
        drop(fault);
        reconcile_session_checkpoints(&fixture.session, &fixture.output).unwrap();
        discard_unsaved(&fixture.session, Fixture::name()).unwrap();
        assert!(!staged.exists());
        let published =
            recording_files::recording_of(&fixture.output, ALLOCATION_CAMPAIGN).unwrap();
        if step == CheckpointStep::Published {
            assert_eq!(
                recorded_frames(&published.unwrap()),
                [(9, 2), (9, 3), (9, 4)]
            );
        } else {
            assert!(published.is_none());
        }
        assert!(read_checkpointed(&campaign).unwrap().is_none());
    }
}

#[test]
fn a_crash_after_the_session_end_record_retires_leftover_journals_without_validating_later_writers()
{
    for record in ["completed.json", "waiting.json"] {
        let fixture = Fixture::new(&format!("checkpoint-completion-crash-{record}"));
        let campaign = fixture.campaign(&fixture.session);
        write_load_segment(&campaign, 0, 9, 2..=4, 19);
        write_load_segment(&campaign, 1, 10, 5..=6, 21);
        let staged = fixture
            .checkpoint(0, false, &mut Ahead::default())
            .recording;
        let fault = StepHook::fail(CheckpointStep::Completed);
        assert!(
            write_session_end(
                &fixture.session,
                &fixture.output,
                record,
                &json!({"waiting":[]})
            )
            .is_err()
        );
        drop(fault);
        assert!(campaign.join(CHECKPOINT_FILE).exists());
        let later = fixture.root.join("later");
        write_load_segment(&fixture.campaign(&later), 0, 11, 7..=8, 23);
        save_campaign(
            &later,
            &fixture.output,
            Fixture::name(),
            true,
            &|| Ok(()),
            &mut Ahead::default(),
            fixture.naming(),
        )
        .unwrap();
        let bytes = fs::read(&staged).unwrap();
        fs::write(campaign.join(CHECKPOINT_FILE), b"a stale malformed journal").unwrap();
        reconcile_session_checkpoints(&fixture.session, &fixture.output).unwrap();
        assert!(!campaign.join(CHECKPOINT_FILE).exists());
        assert_eq!(fs::read(&staged).unwrap(), bytes);
        assert_eq!(
            unsaved(&fixture.session, Fixture::name())
                .unwrap()
                .unwrap()
                .hours,
            5
        );
    }
}

#[test]
fn a_final_save_after_unsaved_checkpoints_and_completion_never_leaves_a_live_journal() {
    let fixture = Fixture::new("checkpoint-final-save-completed");
    let campaign = fixture.campaign(&fixture.session);
    write_load_segment(&campaign, 0, 9, 2..=4, 19);
    write_load_segment(&campaign, 1, 10, 5..=6, 21);
    fixture.checkpoint(0, false, &mut Ahead::default());
    let saved = finalize_session(&fixture.session, &fixture.output)
        .unwrap()
        .remove(0);
    write_session_end(
        &fixture.session,
        &fixture.output,
        "completed.json",
        &json!({}),
    )
    .unwrap();
    let bytes = fs::read(&saved).unwrap();
    reconcile_session_checkpoints(&fixture.session, &fixture.output).unwrap();
    assert_eq!(fs::read(&saved).unwrap(), bytes);
    assert_eq!(
        recorded_frames(&saved),
        [(9, 2), (9, 3), (9, 4), (10, 5), (10, 6)]
    );
    assert!(read_checkpointed(&campaign).unwrap().is_none());
    assert!(segment_paths(&campaign).unwrap().is_empty());
}

#[test]
fn ended_saved_sessions_remove_the_newest_spool_but_waiting_sessions_keep_every_segment() {
    for checkpointed in [false, true] {
        for auto_save in [false, true] {
            let fixture = Fixture::new(&format!("ended-spool-{checkpointed}-{auto_save}"));
            let campaign = fixture.campaign(&fixture.session);
            write_load_segment(&campaign, 0, 9, 2..=4, 19);
            let newest = write_load_segment(&campaign, 1, 10, 5..=6, 21);
            if checkpointed {
                fixture.checkpoint(0, auto_save, &mut Ahead::default());
            }
            let recording = save_campaign(
                &fixture.session,
                &fixture.output,
                Fixture::name(),
                !auto_save,
                &|| Ok(()),
                &mut Ahead::default(),
                fixture.naming(),
            )
            .unwrap()
            .0
            .unwrap();
            assert!(newest.exists(), "live sessions keep the newest segment");
            let spools: Vec<_> = segment_paths(&campaign)
                .unwrap()
                .into_iter()
                .map(|(_, path)| {
                    let bytes = fs::read(&path).unwrap();
                    (path, bytes)
                })
                .collect();
            let waiting = if auto_save {
                vec![]
            } else {
                vec![json!({"campaign": Fixture::name()})]
            };
            write_session_end(
                &fixture.session,
                &fixture.output,
                "completed.json",
                &json!({"waiting": waiting}),
            )
            .unwrap();
            assert!(read_checkpointed(&campaign).unwrap().is_none());
            assert_eq!(drop_checkpointed(&campaign).unwrap(), 0);
            if auto_save {
                assert!(segment_paths(&campaign).unwrap().is_empty());
            } else {
                assert_eq!(segment_paths(&campaign).unwrap().len(), spools.len());
                for (path, bytes) in spools {
                    assert_eq!(fs::read(path).unwrap(), bytes);
                }
            }
            assert_eq!(
                recorded_frames(&recording),
                [(9, 2), (9, 3), (9, 4), (10, 5), (10, 6)]
            );
        }
    }
}

#[test]
fn finalizing_an_already_ended_session_cleans_only_campaigns_no_longer_waiting() {
    for waiting in [false, true] {
        let fixture = Fixture::new(&format!("ended-spool-explicit-finalize-{waiting}"));
        let campaign = fixture.campaign(&fixture.session);
        write_load_segment(&campaign, 0, 9, 2..=4, 19);
        write_load_segment(&campaign, 1, 10, 5..=6, 21);
        save_campaign(
            &fixture.session,
            &fixture.output,
            Fixture::name(),
            true,
            &|| Ok(()),
            &mut Ahead::default(),
            fixture.naming(),
        )
        .unwrap();
        let listed = if waiting {
            vec![json!({"campaign": Fixture::name()})]
        } else {
            vec![]
        };
        write_session_end(
            &fixture.session,
            &fixture.output,
            "completed.json",
            &json!({"waiting": listed}),
        )
        .unwrap();
        assert_eq!(segment_paths(&campaign).unwrap().len(), 2);
        let saved = finalize_session(&fixture.session, &fixture.output).unwrap();
        assert_eq!(saved.len(), 1);
        assert_eq!(
            recorded_frames(&saved[0]),
            [(9, 2), (9, 3), (9, 4), (10, 5), (10, 6)]
        );
        assert_eq!(
            segment_paths(&campaign).unwrap().len(),
            if waiting { 2 } else { 0 }
        );
    }
}

#[test]
fn ended_spool_cleanup_requires_durable_unlisting_retired_journals_and_exact_saved_coverage() {
    for problem in [
        "waiting",
        "journal",
        "revision",
        "campaign",
        "length",
        "uncovered",
        "none",
    ] {
        let fixture = Fixture::new(&format!("ended-spool-guard-{problem}"));
        let campaign = fixture.campaign(&fixture.session);
        let spool = write_load_segment(&campaign, 0, 9, 2..=4, 19);
        let saved = save_campaign(
            &fixture.session,
            &fixture.output,
            Fixture::name(),
            false,
            &|| Ok(()),
            &mut Ahead::default(),
            fixture.naming(),
        )
        .unwrap()
        .0
        .unwrap();
        let before = fs::read(&saved).unwrap();
        write_session_end(
            &fixture.session,
            &fixture.output,
            "waiting.json",
            &json!({"waiting": [{"campaign": Fixture::name()}]}),
        )
        .unwrap();
        assert!(spool.exists(), "publication alone cannot unlist a campaign");
        if problem != "waiting" {
            write_json(
                &fixture.session.join("waiting.json"),
                &json!({"waiting": []}),
            )
            .unwrap();
        }
        match problem {
            "journal" => {
                fs::write(campaign.join(CHECKPOINT_FILE), b"a journal still needed").unwrap()
            }
            "revision" => {
                let mut bookkeeping = read_saved(&campaign).unwrap();
                bookkeeping.revision.crc ^= 1;
                write_saved(&campaign, &bookkeeping).unwrap();
            }
            "campaign" => {
                let other = other_recording(b"00000000-0000-4000-8000-000000000000");
                fs::write(&saved, other.bytes()).unwrap();
            }
            "length" => {
                let mut bytes = fs::read(&spool).unwrap();
                bytes.push(0);
                fs::write(&spool, bytes).unwrap();
            }
            "uncovered" => {
                let mut bookkeeping = read_saved(&campaign).unwrap();
                bookkeeping.segments.clear();
                write_saved(&campaign, &bookkeeping).unwrap();
            }
            _ => {}
        }
        let spool_bytes = fs::read(&spool).unwrap();
        assert_eq!(
            drop_checkpointed(&campaign).unwrap(),
            usize::from(problem == "none")
        );
        if problem == "none" {
            assert!(segment_paths(&campaign).unwrap().is_empty());
            assert_eq!(fs::read(&saved).unwrap(), before);
        } else {
            assert_eq!(fs::read(&spool).unwrap(), spool_bytes);
        }
    }
}

#[test]
fn startup_retries_saved_spool_cleanup_after_interrupted_durable_completion() {
    let fixture = Fixture::new("ended-spool-completion-interrupted");
    let campaign = fixture.campaign(&fixture.session);
    let spool = write_load_segment(&campaign, 0, 9, 2..=4, 19);
    let checkpoint = fixture.checkpoint(0, false, &mut Ahead::default());
    let saved = save_campaign(
        &fixture.session,
        &fixture.output,
        Fixture::name(),
        false,
        &|| Ok(()),
        &mut Ahead::default(),
        fixture.naming(),
    )
    .unwrap()
    .0
    .unwrap();
    assert!(!checkpoint.recording.exists());
    let fault = StepHook::fail(CheckpointStep::Completed);
    assert!(
        write_session_end(
            &fixture.session,
            &fixture.output,
            "completed.json",
            &json!({"waiting": []}),
        )
        .is_err()
    );
    drop(fault);
    assert!(fixture.session.join("completed.json").exists());
    assert!(spool.exists());
    reconcile_session_checkpoints(&fixture.session, &fixture.output).unwrap();
    assert!(segment_paths(&campaign).unwrap().is_empty());
    assert_eq!(recorded_frames(&saved), [(9, 2), (9, 3), (9, 4)]);
}

#[test]
fn a_renamed_published_checkpoint_still_trims_its_spool_and_is_never_rolled_back() {
    let fixture = Fixture::new("checkpoint-published-rename");
    let campaign = fixture.campaign(&fixture.session);
    let first = write_load_segment(&campaign, 0, 9, 2..=4, 19);
    let published = fixture.checkpoint(0, true, &mut Ahead::default()).recording;
    assert!(first.exists());
    let before = fs::read(&published).unwrap();
    let renamed = {
        let _lock = NamesLock::wait(&fixture.output).unwrap();
        recording_files::rename(&published, "Renamed").unwrap()
    };
    let other = other_recording(b"00000000-0000-4000-8000-000000000000");
    fs::write(&published, other.bytes()).unwrap();
    write_load_segment(&campaign, 1, 10, 5..=6, 21);
    assert_eq!(drop_checkpointed(&campaign).unwrap(), 1);
    assert!(!first.exists());
    discard_unsaved(&fixture.session, Fixture::name()).unwrap();
    assert_eq!(fs::read(&renamed).unwrap(), before);
    assert_eq!(fs::read(&published).unwrap(), other.bytes());
    assert!(read_checkpointed(&campaign).unwrap().is_none());
    assert!(segment_paths(&campaign).unwrap().is_empty());
}

#[test]
fn startup_cleans_unlisted_saved_spools_after_a_later_session_appends() {
    let fixture = Fixture::new("ended-spool-ancestor-retry");
    let campaign = fixture.campaign(&fixture.session);
    let spool = write_load_segment(&campaign, 0, 9, 2..=4, 19);
    let saved = finalize_session(&fixture.session, &fixture.output)
        .unwrap()
        .remove(0);
    write_session_end(
        &fixture.session,
        &fixture.output,
        "waiting.json",
        &json!({"waiting": [{"campaign": Fixture::name()}]}),
    )
    .unwrap();
    write_json(
        &fixture.session.join("waiting.json"),
        &json!({"waiting": []}),
    )
    .unwrap();
    let revision = read_saved(&campaign).unwrap().revision;
    let later = fixture.root.join("later");
    write_load_segment(&fixture.campaign(&later), 0, 10, 5..=6, 21);
    finalize_session(&later, &fixture.output).unwrap();
    let replay = Replay::open_path(&saved).unwrap();
    assert_ne!(replay.revision(), revision);
    assert!(replay.contains_revision(revision).unwrap());
    assert!(spool.exists());
    reconcile_session_checkpoints(&fixture.session, &fixture.output).unwrap();
    assert!(segment_paths(&campaign).unwrap().is_empty());
    assert_eq!(
        recorded_frames(&saved),
        [(9, 2), (9, 3), (9, 4), (10, 5), (10, 6)]
    );
}

#[test]
fn one_campaigns_cleanup_error_does_not_stop_the_other_campaigns() {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::{FILE_SHARE_READ, FILE_SHARE_WRITE};

    let fixture = Fixture::new("ended-spool-independent-cleanup");
    let campaign = fixture.campaign(&fixture.session);
    let healthy = write_load_segment(&campaign, 0, 9, 2..=4, 19);
    let other_id = "00000000-0000-4000-8000-000000000000";
    let other = fixture.session.join(other_id);
    let blocked = write_load_segment(&other, 0, 9, 2..=4, 19);
    let mut records = replay::spool::recover_prefix(&fs::read(&blocked).unwrap()).records;
    for record in &mut records {
        if let Record::Frame(frame) = record {
            if let TableUpdate::Snapshot(TableRows::Campaign(rows)) =
                &mut frame.tables[TableId::Campaign.index()]
            {
                rows[0].id = other_id.as_bytes().to_vec();
            }
        }
    }
    fs::write(
        &blocked,
        records
            .iter()
            .flat_map(|record| replay::spool::encode_record(record).unwrap())
            .collect::<Vec<_>>(),
    )
    .unwrap();
    assert_eq!(
        finalize_session(&fixture.session, &fixture.output)
            .unwrap()
            .len(),
        2
    );
    let held = fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .open(&blocked)
        .unwrap();
    write_session_end(
        &fixture.session,
        &fixture.output,
        "completed.json",
        &json!({"waiting": []}),
    )
    .unwrap();
    assert!(blocked.exists());
    assert!(!healthy.exists());
    let errors = cleanup_ended_checkpoints(&fixture.session, &fixture.output).unwrap();
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(errors[0].contains(other_id), "{errors:?}");
    drop(held);
    reconcile_session_checkpoints(&fixture.session, &fixture.output).unwrap();
    assert!(!blocked.exists());
}

#[test]
fn record_once_prints_one_complete_timed_summary_after_multiple_checkpoints() {
    for context_only in [false, true] {
        let fixture = Fixture::new(&format!("checkpoint-once-summary-{context_only}"));
        begin_admissions(&fixture.session, &fixture.output).unwrap();
        let mut admissions = Admissions::default();
        for load in [19, 21, 23] {
            admissions
                .admit(&fixture.session, &fixture.output, Fixture::name(), load)
                .unwrap();
        }
        let campaign = fixture.campaign(&fixture.session);
        let first = write_load_segment(&campaign, 0, 9, 2..=4, 19);
        let second = write_load_segment(&campaign, 1, 10, 5..=6, 21);
        let third = write_load_segment(&campaign, 2, 11, 7..=8, 23);
        let mut expected = Box::<Summary>::default();
        let mut spool_bytes = 0;
        for path in [&first, &second, &third] {
            let mut records = replay::spool::recover_prefix(&fs::read(path).unwrap()).records;
            if path == &third && context_only {
                records = replay::spool::recover_prefix(&fs::read(&second).unwrap())
                    .records
                    .into_iter()
                    .filter(|record| !matches!(record, Record::Frame(_)))
                    .collect();
            }
            for record in &mut records {
                if let Record::Frame(frame) = record {
                    for (index, time) in frame.metrics.table_ns.iter_mut().enumerate() {
                        *time = (frame.hour as u64 + index as u64 + 1) * 1_000_000;
                    }
                    frame.metrics.total_ns = frame.metrics.table_ns.iter().sum::<u64>() + 1_000_000;
                    frame.metrics.pool_bytes = frame.hour as u64 * 1_024;
                    frame.metrics.dropped_frames = 0;
                    expected.observe(frame).unwrap();
                }
            }
            let bytes: Vec<_> = records
                .iter()
                .flat_map(|record| replay::spool::encode_record(record).unwrap())
                .collect();
            spool_bytes += bytes.len() as u64;
            fs::write(path, bytes).unwrap();
        }
        let _capture = OutputCapture::new();
        let mut ahead = Ahead::default();
        for (through, path) in [(0, &first), (1, &second)] {
            ahead.read(&fixture.session, &fixture.output).unwrap();
            let done = fixture.checkpoint(through, true, &mut ahead);
            assert_eq!(done.deleted, 1);
            assert!(!path.exists());
            ahead.forget(&campaign);
        }
        let done = finalize_session_ahead(
            &fixture.session,
            &fixture.output,
            &|| Ok(()),
            ahead,
            fixture.naming(),
            &|_| Destination::Recording,
        )
        .unwrap();
        assert!(done.errors.is_empty(), "{:?}", done.errors);
        assert_eq!(done.replays.len(), 1);
        let output = FINALIZATION_OUTPUT.with_borrow(|output| output.as_ref().unwrap().concat());
        let lines: Vec<_> = output.lines().collect();
        assert_eq!(
            lines
                .iter()
                .filter(|line| line.starts_with("checkpoint_s="))
                .count(),
            2
        );
        assert_eq!(
            lines
                .iter()
                .filter(|line| line.starts_with("finalize_s="))
                .count(),
            1
        );
        assert_eq!(
            lines
                .iter()
                .filter(|line| line.starts_with("table raw_encoded_bytes "))
                .count(),
            1
        );
        let finalization = lines
            .iter()
            .find(|line| line.starts_with("finalize_s="))
            .unwrap();
        assert!(
            finalization.ends_with(&format!("spool_bytes={spool_bytes}")),
            "{finalization}"
        );
        let replay_bytes = fs::metadata(&done.replays[0]).unwrap().len();
        assert!(
            finalization.contains(&format!("replay_bytes={replay_bytes} ")),
            "{finalization}"
        );
        let mut expected_text = Vec::new();
        expected.write(spool_bytes, &mut expected_text).unwrap();
        let expected_text = String::from_utf8(expected_text).unwrap();
        assert!(
            output.ends_with(&expected_text),
            "{output}\nexpected:\n{expected_text}"
        );
        assert!(expected_text.starts_with(if context_only { "hours=5 " } else { "hours=7 " }));
        assert!(!expected_text.contains("not_recorded"));
        assert_eq!(
            recorded_frames(&done.replays[0]).len(),
            if context_only { 5 } else { 7 }
        );
    }
}

#[test]
fn final_summary_includes_checkpointed_hours_when_live_statistics_are_unavailable() {
    let fixture = Fixture::new("checkpoint-recovered-summary");
    let campaign = fixture.campaign(&fixture.session);
    for path in [
        write_load_segment(&campaign, 0, 9, 2..=4, 19),
        write_load_segment(&campaign, 1, 10, 5..=6, 21),
    ] {
        let mut records = replay::spool::recover_prefix(&fs::read(&path).unwrap()).records;
        for record in &mut records {
            if let Record::Frame(frame) = record {
                frame.metrics.table_ns.fill(1_000_000);
            }
        }
        fs::write(
            &path,
            records
                .iter()
                .flat_map(|record| replay::spool::encode_record(record).unwrap())
                .collect::<Vec<_>>(),
        )
        .unwrap();
    }
    let checkpoint = fixture.checkpoint(0, true, &mut Ahead::default());
    assert_eq!(checkpoint.deleted, 1);
    let saved = finalize_session(&fixture.session, &fixture.output)
        .unwrap()
        .remove(0);
    let replay = Replay::open_path(&saved).unwrap();
    let mut report = FinalizationReport::default();
    report.complete(&replay, &|| Ok(())).unwrap();
    assert_eq!(report.summary.totals.len(), 5);
    assert_eq!(
        report.summary.read_times(TableId::Clock),
        "not_recorded not_recorded not_recorded"
    );
    assert!(
        report
            .text(replay.saved_length())
            .unwrap()
            .contains("hours=5 ")
    );
}

#[test]
fn a_discarded_checkpoint_contributes_no_hours_to_the_final_summary() {
    let fixture = Fixture::new("checkpoint-summary-discard");
    let campaign = fixture.campaign(&fixture.session);
    write_load_segment(&campaign, 0, 9, 2..=4, 19);
    write_load_segment(&campaign, 1, 10, 5..=6, 21);
    let mut ahead = Ahead::default();
    fixture.checkpoint(0, false, &mut ahead);
    let report = ahead.checkpoint_reports.remove(&campaign).unwrap();
    assert_eq!(report.summary.totals.len(), 3);
    discard_checkpoints(&fixture.session, Fixture::name()).unwrap();
    discard_unsaved(&fixture.session, Fixture::name()).unwrap();
    write_load_segment(&campaign, 2, 11, 7..=8, 23);
    let saved = finalize_session(&fixture.session, &fixture.output)
        .unwrap()
        .remove(0);
    let replay = Replay::open_path(&saved).unwrap();
    assert!(
        merge_reports(Some(report), None, &replay)
            .unwrap()
            .is_none()
    );
}

#[test]
fn restarted_finalization_counts_checkpointed_and_pending_spool_segments_once() {
    for live_checkpoint_after_restart in [false, true] {
        let fixture = Fixture::new(&format!(
            "checkpoint-restarted-bytes-{live_checkpoint_after_restart}"
        ));
        let campaign = fixture.campaign(&fixture.session);
        let first = write_load_segment(&campaign, 0, 9, 2..=4, 19);
        let second = write_load_segment(&campaign, 1, 10, 5..=6, 21);
        let mut lengths = BTreeMap::from([
            (0, fs::metadata(&first).unwrap().len()),
            (1, fs::metadata(&second).unwrap().len()),
        ]);
        let checkpoint = fixture.checkpoint(0, true, &mut Ahead::default());
        assert_eq!(checkpoint.deleted, 1);
        assert!(!first.exists());
        assert_eq!(
            read_saved(&campaign).unwrap().segments,
            BTreeMap::from([(0, lengths[&0])])
        );
        let mut restarted = Ahead::default();
        if live_checkpoint_after_restart {
            let third = write_load_segment(&campaign, 2, 11, 7..=8, 23);
            lengths.insert(2, fs::metadata(&third).unwrap().len());
            let checkpoint = fixture.checkpoint(1, true, &mut restarted);
            assert_eq!(checkpoint.deleted, 1);
            assert!(!second.exists());
        }
        let bytes: u64 = lengths.values().sum();
        let hours = if live_checkpoint_after_restart { 7 } else { 5 };
        let capture = OutputCapture::new();
        for repeat in [false, true] {
            let done = if live_checkpoint_after_restart && !repeat {
                finalize_session_ahead(
                    &fixture.session,
                    &fixture.output,
                    &|| Ok(()),
                    std::mem::take(&mut restarted),
                    fixture.naming(),
                    &|_| Destination::Recording,
                )
            } else {
                finalize_session_outcome(&fixture.session, &fixture.output)
            }
            .unwrap();
            assert!(done.errors.is_empty(), "{:?}", done.errors);
            assert_eq!(done.replays.len(), 1);
            assert_eq!(recorded_frames(&done.replays[0]).len(), hours);
            assert_eq!(read_saved(&campaign).unwrap().segments, lengths);
            let output = capture.take();
            let headers: Vec<_> = output
                .lines()
                .filter(|line| line.starts_with("finalize_s="))
                .collect();
            assert_eq!(headers.len(), 1, "{output}");
            assert!(
                headers[0].ends_with(&format!("spool_bytes={bytes}")),
                "{output}"
            );
            assert!(
                output
                    .lines()
                    .any(|line| line.starts_with(&format!("hours={hours} "))),
                "{output}"
            );
            assert!(
                output.lines().any(|line| line
                    == format!("spool_bytes_per_hour={:.3}", bytes as f64 / hours as f64)),
                "{output}"
            );
        }
    }
}

#[test]
fn an_ended_checkpoint_without_spools_still_finalizes_and_reports_its_saved_bytes() {
    let fixture = Fixture::new("ended-checkpoint-without-spools");
    let campaign = fixture.campaign(&fixture.session);
    let spool = write_load_segment(&campaign, 0, 9, 2..=4, 19);
    let spool_bytes = fs::metadata(&spool).unwrap().len();
    let checkpoint = fixture.checkpoint(0, true, &mut Ahead::default());
    write_session_end(
        &fixture.session,
        &fixture.output,
        "completed.json",
        &json!({"waiting": []}),
    )
    .unwrap();
    assert!(!spool.exists());
    assert!(spooled_campaigns(&fixture.session).unwrap().is_empty());
    let bytes = fs::read(&checkpoint.recording).unwrap();
    let capture = OutputCapture::new();
    for _ in 0..2 {
        let done = finalize_session_ahead(
            &fixture.session,
            &fixture.output,
            &|| Ok(()),
            Ahead::default(),
            fixture.naming(),
            &|_| Destination::Recording,
        )
        .unwrap();
        assert!(done.errors.is_empty(), "{:?}", done.errors);
        assert!(done.failed_campaigns.is_empty());
        assert_eq!(done.replays, [checkpoint.recording.clone()]);
        assert_eq!(recorded_frames(&done.replays[0]), [(9, 2), (9, 3), (9, 4)]);
        assert_eq!(fs::read(&done.replays[0]).unwrap(), bytes);
        let output = capture.take();
        let headers: Vec<_> = output
            .lines()
            .filter(|line| line.starts_with("finalize_s="))
            .collect();
        assert_eq!(headers.len(), 1, "{output}");
        assert!(
            headers[0].ends_with(&format!("spool_bytes={spool_bytes}")),
            "{output}"
        );
        assert!(
            output.lines().any(|line| line.starts_with("hours=3 ")),
            "{output}"
        );
    }
}

#[test]
fn an_unverifiable_saved_only_revision_is_refused_even_with_terminal_ranges() {
    for with_range in [false, true] {
        let fixture = Fixture::new(&format!("ended-unverified-checkpoint-{with_range}"));
        let campaign = fixture.campaign(&fixture.session);
        let spool = write_load_segment(&campaign, 0, 9, 2..=4, 19);
        let checkpoint = fixture.checkpoint(0, true, &mut Ahead::default());
        write_session_end(
            &fixture.session,
            &fixture.output,
            "completed.json",
            &json!({"waiting": []}),
        )
        .unwrap();
        assert!(!spool.exists());
        let mut saved = read_saved(&campaign).unwrap();
        saved.revision.crc ^= 1;
        write_saved(&campaign, &saved).unwrap();
        let bookkeeping = fs::read(campaign.join(SAVED_FILE)).unwrap();
        let bytes = fs::read(&checkpoint.recording).unwrap();
        if with_range {
            let witness = TerminalWitness {
                committed_campaign_id: None,
                committed_segment: None,
                committed_bytes: 0,
                failed_hour: None,
                uncommitted: Some(crate::test_support::allocation_range(9, Some(19), 5..=6)),
                ..crate::test_support::allocation_witness(0)
            };
            fs::write(fixture.session.join(WITNESS_FILE_NAME), witness.encode()).unwrap();
        }
        let done = finalize_session_outcome(&fixture.session, &fixture.output).unwrap();
        assert!(done.replays.is_empty());
        assert!(done.failed_campaigns.contains(Fixture::name()));
        assert_eq!(done.errors.len(), 1);
        assert!(done.errors[0].contains("saved revision cannot be verified; spool retained"));
        assert_eq!(fs::read(&checkpoint.recording).unwrap(), bytes);
        assert_eq!(fs::read(campaign.join(SAVED_FILE)).unwrap(), bookkeeping);
        assert!(segment_paths(&campaign).unwrap().is_empty());
    }
}
