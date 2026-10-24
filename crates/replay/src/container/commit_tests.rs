//! Saving to a recording in place, and reopening one a save was interrupted in.

use super::*;
use std::sync::atomic::AtomicU64;

struct Scratch(PathBuf);

impl Scratch {
    fn new(bytes: &[u8]) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "replay-in-place-{}-{}.battleplan",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(&path, bytes).unwrap();
        Self(path)
    }

    fn bytes(&self) -> Vec<u8> {
        std::fs::read(&self.0).unwrap()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn segment(number: u32, records: Vec<Record>) -> [(u32, Recovery); 1] {
    [(
        number,
        Recovery {
            format: Some(FormatVersion::CURRENT),
            records,
            valid_bytes: 0,
            damage: None,
        },
    )]
}

/// Add `segments` to the recording at `path` in place.
fn save_in_place(path: &Path, segments: &[(u32, Recovery)]) -> io::Result<Replay> {
    let mut replay = Replay::open_appendable(path)?;
    replay.append_in_place(
        &kept(segments),
        |number, frame| restore(segments, number, frame),
        || Ok(()),
        |_| Ok(()),
        None,
        |_| Ok(()),
    )?;
    Ok(replay)
}

#[test]
fn a_save_in_place_writes_what_an_append_in_memory_writes_and_reads_back_the_same() {
    let old = Replay::from_records(CAMPAIGN.into(), &[records(1, &[1, 2, 721])]).unwrap();
    let file = Scratch::new(old.bytes());
    let next = segment(4, records(2, &[800, 801]));
    let saved = save_in_place(&file.0, &next).unwrap();
    let expected = old.append(&[records(2, &[800, 801])]).unwrap();
    assert_eq!(file.bytes(), expected.bytes());
    assert_eq!(saved.revision(), expected.revision());
    assert_eq!(saved.index, expected.index);
    assert_eq!(saved.sessions, expected.sessions);
    assert_eq!(saved.controls, expected.controls);
    assert!(!saved.needed_recovery());
    for block in &saved.index {
        assert_eq!(
            saved.read_block(block).unwrap(),
            expected.read_block(block).unwrap()
        );
    }
    // The earlier save is kept whole, trailer and footer too, but the slot naming the newest.
    assert_eq!(
        &unslotted(&expected)[..old.bytes().len()],
        &unslotted(&old)[..]
    );
    assert_eq!(expected.regions.len(), 1);
    assert_eq!(
        expected.previous,
        Some((old.revision.trailer, old.revision.length))
    );
}

#[test]
fn revision_ancestry_requires_the_exact_footer_in_the_linked_chain() {
    let first = Replay::from_records(CAMPAIGN.into(), &[records(1, &[1, 2])]).unwrap();
    let second = first.append(&[records(2, &[3, 4])]).unwrap();
    let third = second.append(&[records(3, &[5, 6])]).unwrap();
    let file = Scratch::new(third.bytes());
    let held = Replay::open_path(&file.0).unwrap();
    for replay in [&third, &held] {
        for ancestor in [&first, &second, &third] {
            assert!(replay.contains_revision(ancestor.revision()).unwrap());
        }
        let mut different = first.revision();
        different.crc ^= 1;
        assert!(!replay.contains_revision(different).unwrap());
        different = first.revision();
        different.length += 1;
        assert!(!replay.contains_revision(different).unwrap());
    }
    assert!(!first.contains_revision(second.revision()).unwrap());
    let rewritten = Replay::from_records(
        CAMPAIGN.into(),
        &[
            records(1, &[1, 2]),
            records(2, &[3, 4]),
            records(3, &[5, 6]),
        ],
    )
    .unwrap();
    assert!(!rewritten.contains_revision(first.revision()).unwrap());
}

#[test]
fn a_save_cut_short_anywhere_reopens_as_the_save_before_it() {
    let old = Replay::from_records(CAMPAIGN.into(), &[records(1, &[1, 2])]).unwrap();
    let new = old.append(&[records(2, &[3, 4])]).unwrap();
    let slot = old.slot.unwrap();
    // A save writes its blocks, trailer and footer, then names itself in the slot.
    let mut written = new.bytes().to_vec();
    written[slot..slot + commit::SLOT_SIZE]
        .copy_from_slice(&old.bytes()[slot..slot + commit::SLOT_SIZE]);
    for length in old.bytes().len()..written.len() {
        let reopened = Replay::open(written[..length].to_vec()).unwrap();
        assert_eq!(reopened.revision(), old.revision(), "{length}");
        assert_eq!(reopened.index, old.index, "{length}");
        assert_eq!(reopened.sessions, old.sessions, "{length}");
        assert!(!reopened.needed_recovery(), "{length}");
        assert_eq!(reopened.bytes(), old.bytes(), "{length}");
        let file = Scratch::new(&written[..length]);
        let held = Replay::open_path(&file.0).unwrap();
        assert_eq!(held.revision(), old.revision(), "{length}");
        let mut source = std::io::Cursor::new(&written[..length]);
        assert_eq!(
            Replay::read_trailer_summary(&mut source, length as u64)
                .unwrap()
                .3,
            [1],
            "{length}"
        );
    }
    // Committed by its footer, it is the new save whatever the slot names.
    let reopened = Replay::open(written.clone()).unwrap();
    assert_eq!(reopened.revision(), new.revision());
    assert_eq!(reopened.index, new.index);
    // A torn footer leaves the save before it.
    let mut torn = written.clone();
    let end = torn.len();
    torn[end - 3] ^= 0xff;
    torn[end - 20] ^= 0xff;
    assert_eq!(Replay::open(torn).unwrap().revision(), old.revision());
    // Without a slot naming a complete save, a cut recording is refused as before.
    let mut unnamed = written[..written.len() - 1].to_vec();
    unnamed[slot..slot + commit::SLOT_SIZE].fill(0);
    assert!(Replay::open(unnamed).is_err());
}

#[test]
fn the_next_save_drops_an_interrupted_one_and_adds_to_the_save_before_it() {
    let old = Replay::from_records(CAMPAIGN.into(), &[records(1, &[1, 2])]).unwrap();
    let mut interrupted = old.bytes().to_vec();
    interrupted.extend_from_slice(b"the start of a save that never finished");
    let file = Scratch::new(&interrupted);
    assert_eq!(
        Replay::open_path(&file.0).unwrap().saved_length(),
        old.saved_length()
    );
    save_in_place(&file.0, &segment(0, records(2, &[3]))).unwrap();
    assert_eq!(
        file.bytes(),
        old.append(&[records(2, &[3])]).unwrap().bytes()
    );
}

#[test]
fn saves_in_place_chain_their_trailers_and_go_back_to_an_earlier_save() {
    let first = Replay::from_records(CAMPAIGN.into(), &[records(1, &[1, 2])]).unwrap();
    let file = Scratch::new(first.bytes());
    let second = save_in_place(&file.0, &segment(0, records(2, &[3, 4]))).unwrap();
    let after_second = file.bytes();
    let third = save_in_place(&file.0, &segment(0, records(3, &[5, 721, 722]))).unwrap();
    assert_eq!(third.regions.len(), 2);
    assert_eq!(
        third
            .sessions
            .iter()
            .map(|session| session.id)
            .collect::<Vec<_>>(),
        [1, 2, 3]
    );
    assert_eq!(
        third.earlier,
        [
            2,
            2,
            second.gaps.len(),
            second.controls.len(),
            second.recovery_notes.len()
        ]
    );
    assert!(third.index.starts_with(&second.index));
    let length = file.bytes().len() as u64;
    let summary =
        Replay::read_trailer_summary(&mut std::fs::File::open(&file.0).unwrap(), length).unwrap();
    assert_eq!(summary.1, Some((1, 722)));
    assert_eq!(summary.3, [1, 2, 3]);
    let resident = Replay::open(file.bytes()).unwrap();
    assert_eq!(resident.index, third.index);
    assert!(!resident.needed_recovery());
    let revision = second.revision();
    drop((second, third, resident));
    Replay::roll_back(&file.0, revision).unwrap();
    assert_eq!(file.bytes(), after_second);
    // Going back to a save the recording never made changes nothing.
    let unknown = Revision {
        end: revision.end - 1,
        ..revision
    };
    assert!(Replay::roll_back(&file.0, unknown).is_err());
    assert_eq!(file.bytes(), after_second);
}

#[test]
fn a_recording_saved_in_place_holds_the_one_it_continues_and_no_other() {
    let old = Replay::from_records(CAMPAIGN.into(), &[records(1, &[1, 2])]).unwrap();
    let earlier = Scratch::new(old.bytes());
    let copy = Scratch::new(old.bytes());
    let saved = save_in_place(&copy.0, &segment(0, records(2, &[3]))).unwrap();
    assert!(saved.holds(&earlier.0).unwrap());
    let resident = Replay::open(copy.bytes()).unwrap();
    assert!(resident.holds(&earlier.0).unwrap());
    let other = Scratch::new(
        Replay::from_records(CAMPAIGN.into(), &[records(1, &[1, 3])])
            .unwrap()
            .bytes(),
    );
    assert!(!saved.holds(&other.0).unwrap());
    assert!(!old.holds(&copy.0).unwrap());
}

#[test]
fn only_a_current_recording_opened_to_be_added_to_is_saved_in_place() {
    let current = Replay::from_records(CAMPAIGN.into(), &[records(1, &[1, 2])]).unwrap();
    let file = Scratch::new(current.bytes());
    let mut held = Replay::open_path(&file.0).unwrap();
    let next = segment(0, records(2, &[3]));
    let refused = held
        .append_in_place(
            &kept(&next),
            |number, frame| restore(&next, number, frame),
            || Ok(()),
            |_| Ok(()),
            None,
            |_| Ok(()),
        )
        .unwrap_err();
    assert_eq!(refused.kind(), io::ErrorKind::Unsupported);
    let older = Scratch::new(
        &relabelled(
            current.bytes(),
            crate::rows::compatibility::CHECKPOINT_FORMAT - 1,
        )
        .unwrap(),
    );
    let reopened = Replay::open_path(&older.0).unwrap();
    assert!(reopened.slot.is_none());
    assert_eq!(reopened.index.len(), current.index.len());
    assert_eq!(
        reopened.read_block(&reopened.index[0]).unwrap(),
        current.read_block(&current.index[0]).unwrap()
    );
    let before = older.bytes();
    assert!(save_in_place(&older.0, &next).is_err());
    assert_eq!(older.bytes(), before);
    assert_eq!(file.bytes(), current.bytes());
}

#[test]
fn a_full_length_torn_footer_crc_falls_back_to_the_intact_save_and_can_be_appended() {
    let old = Replay::from_records(CAMPAIGN.into(), &[records(1, &[1, 2])]).unwrap();
    let next = old.append(&[records(2, &[3, 4])]).unwrap();
    for damaged in 0..4 {
        let mut bytes = next.bytes().to_vec();
        let slot = old.slot.unwrap();
        bytes[slot..slot + commit::SLOT_SIZE]
            .copy_from_slice(&old.bytes()[slot..slot + commit::SLOT_SIZE]);
        let end = bytes.len();
        bytes[end - 4 + damaged] ^= 0xff;
        let reopened = Replay::open(bytes.clone()).unwrap();
        assert_eq!(reopened.revision(), old.revision());
        assert!(!reopened.needed_recovery());
        let file = Scratch::new(&bytes);
        let held = Replay::open_path(&file.0).unwrap();
        assert_eq!(held.revision(), old.revision());
        assert!(!held.needed_recovery());
        drop(held);
        let appended = save_in_place(&file.0, &segment(0, records(3, &[5]))).unwrap();
        assert_eq!(appended.sessions.len(), 2);
        assert_eq!(appended.sessions[1].id, 3);
    }
}

#[test]
fn a_checksum_valid_footer_with_an_invalid_trailer_chain_uses_the_intact_slot() {
    let old = Replay::from_records(CAMPAIGN.into(), &[records(1, &[1])]).unwrap();
    let next = old.append(&[records(2, &[2])]).unwrap();
    let mut bytes = next.bytes().to_vec();
    let slot = old.slot.unwrap();
    bytes[slot..slot + commit::SLOT_SIZE]
        .copy_from_slice(&old.bytes()[slot..slot + commit::SLOT_SIZE]);
    let start = next.revision().trailer as usize;
    let end = bytes.len() - FOOTER_SIZE;
    let mut trailer = Decoder::new(&bytes[start..end]);
    expect_magic(&mut trailer, TRAILER_MAGIC).unwrap();
    for _ in 0..5 {
        let count = trailer.count(4).unwrap();
        for _ in 0..count {
            trailer.bytes().unwrap();
        }
    }
    let link = end - trailer.remaining();
    bytes[link..link + 8].copy_from_slice(&(next.revision().trailer + 1).to_le_bytes());
    let crc = crc32fast::hash(&bytes[start..end]);
    bytes[end + 20..].copy_from_slice(&crc.to_le_bytes());
    let reopened = Replay::open(bytes).unwrap();
    assert_eq!(reopened.revision(), old.revision());
    assert!(!reopened.needed_recovery());
}

#[test]
fn opening_retries_when_the_slot_advanced_past_the_sampled_length() {
    struct Growing {
        bytes: Vec<u8>,
        calls: usize,
    }
    impl commit::Source for Growing {
        fn length(&mut self) -> io::Result<u64> {
            self.calls += 1;
            Ok(self.bytes.len() as u64 - u64::from(self.calls == 1))
        }
        fn read_at(&mut self, offset: u64, output: &mut [u8]) -> io::Result<()> {
            commit::Source::read_at(&mut commit::Resident(&self.bytes), offset, output)
        }
    }
    let old = Replay::from_records(CAMPAIGN.into(), &[records(1, &[1])]).unwrap();
    let next = old.append(&[records(2, &[2])]).unwrap();
    let mut source = Growing {
        bytes: next.bytes().to_vec(),
        calls: 0,
    };
    let located = commit::locate(&mut source).unwrap();
    assert_eq!(located.revision, next.revision());
    assert!(located.warnings.is_empty());
    assert!(source.calls >= 3);
}

#[test]
fn opening_retries_a_torn_slot_read_without_requiring_length_growth() {
    struct Repairing {
        bytes: Vec<u8>,
        slot: usize,
        sampled: bool,
    }
    impl commit::Source for Repairing {
        fn length(&mut self) -> io::Result<u64> {
            Ok(self.bytes.len() as u64)
        }
        fn read_at(&mut self, offset: u64, output: &mut [u8]) -> io::Result<()> {
            commit::Source::read_at(&mut commit::Resident(&self.bytes), offset, output)?;
            if offset == 0 && output.len() == self.slot + commit::SLOT_SIZE && !self.sampled {
                output[self.slot + FOOTER_SIZE] ^= 0xff;
                self.sampled = true;
            }
            Ok(())
        }
    }
    let old = Replay::from_records(CAMPAIGN.into(), &[records(1, &[1])]).unwrap();
    let mut bytes = old.bytes().to_vec();
    bytes.extend_from_slice(b"a still unfinished append");
    let mut source = Repairing {
        bytes,
        slot: old.slot.unwrap(),
        sampled: false,
    };
    let located = commit::locate(&mut source).unwrap();
    assert_eq!(located.revision, old.revision());
    assert!(located.warnings.is_empty());
    assert!(source.sampled);
}

#[test]
fn held_readers_keep_their_revision_after_rollback_and_new_appends_reuse_offsets() {
    let first = Replay::from_records(CAMPAIGN.into(), &[records(1, &[1, 2])]).unwrap();
    let file = Scratch::new(first.bytes());
    let second = save_in_place(&file.0, &segment(0, records(2, &[3, 4]))).unwrap();
    let expected: Vec<_> = second
        .index
        .iter()
        .map(|block| second.read_block(block).unwrap())
        .collect();
    drop(second);
    let held = Replay::open_path(&file.0).unwrap();
    Replay::roll_back(&file.0, first.revision()).unwrap();
    assert_eq!(file.bytes(), first.bytes());
    let replacement = save_in_place(&file.0, &segment(0, records(3, &[5, 6]))).unwrap();
    assert_eq!(replacement.sessions[1].id, 3);
    assert_eq!(held.sessions[1].id, 2);
    for (block, expected) in held.index.iter().zip(expected) {
        assert_eq!(held.read_block(block).unwrap(), expected);
    }
}

#[test]
fn an_unstable_commit_snapshot_stops_retrying_after_eight_attempts() {
    struct Unstable {
        bytes: Vec<u8>,
        calls: usize,
    }
    impl commit::Source for Unstable {
        fn length(&mut self) -> io::Result<u64> {
            self.calls += 1;
            Ok(self.bytes.len() as u64 - (self.calls % 2) as u64)
        }
        fn read_at(&mut self, offset: u64, output: &mut [u8]) -> io::Result<()> {
            commit::Source::read_at(&mut commit::Resident(&self.bytes), offset, output)
        }
    }
    let old = Replay::from_records(CAMPAIGN.into(), &[records(1, &[1])]).unwrap();
    let mut bytes = old.bytes().to_vec();
    let slot = old.slot.unwrap();
    bytes[slot..slot + commit::SLOT_SIZE].fill(0);
    bytes.extend_from_slice(b"a save with no usable commit slot or footer");
    let mut source = Unstable { bytes, calls: 0 };
    let error = commit::locate(&mut source).err().unwrap();
    assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
    assert_eq!(source.calls, 16);
}

#[test]
fn metadata_uses_a_fresh_snapshot_if_the_recording_grew_after_the_callers_length_sample() {
    let old = Replay::from_records(CAMPAIGN.into(), &[records(1, &[1])]).unwrap();
    let next = old.append(&[records(2, &[2])]).unwrap();
    let summary =
        Replay::read_trailer_summary(&mut std::io::Cursor::new(next.bytes()), old.saved_length())
            .unwrap();
    assert_eq!(summary.3, [1, 2]);
}

#[test]
fn a_held_revision_visits_its_original_bytes_after_the_live_slot_advances() {
    let first = Replay::from_records(CAMPAIGN.into(), &[records(1, &[1, 2])]).unwrap();
    let file = Scratch::new(first.bytes());
    let held = Replay::open_path(&file.0).unwrap();
    let second = save_in_place(&file.0, &segment(0, records(2, &[3]))).unwrap();
    assert_ne!(second.revision(), held.revision());
    let mut bytes = Vec::new();
    held.visit_held_bytes(|chunk| {
        bytes.extend_from_slice(chunk);
        Ok(())
    })
    .unwrap();
    assert_eq!(bytes, first.bytes());
}

#[test]
fn replacing_an_interrupted_tail_preserves_a_reader_that_saw_its_complete_footer() {
    let first = Replay::from_records(CAMPAIGN.into(), &[records(1, &[1, 2])]).unwrap();
    let file = Scratch::new(first.bytes());
    let second = save_in_place(&file.0, &segment(0, records(2, &[3, 4]))).unwrap();
    let expected: Vec<_> = second
        .index
        .iter()
        .map(|block| second.read_block(block).unwrap())
        .collect();
    let second_revision = second.revision();
    drop(second);
    let mut bytes = file.bytes();
    let slot = first.slot.unwrap();
    bytes[slot..slot + commit::SLOT_SIZE].copy_from_slice(&first.revision().slot());
    std::fs::write(&file.0, &bytes).unwrap();
    let held = Replay::open_path(&file.0).unwrap();
    assert_eq!(held.revision(), second_revision);
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    std::fs::write(&file.0, &bytes).unwrap();
    let replacement = save_in_place(&file.0, &segment(0, records(3, &[5, 6]))).unwrap();
    assert_eq!(replacement.sessions[1].id, 3);
    for (block, expected) in held.index.iter().zip(expected) {
        assert_eq!(held.read_block(block).unwrap(), expected);
    }
}

#[test]
fn opening_retries_a_slot_that_stays_torn_through_the_first_double_sample() {
    struct PausedWriter {
        bytes: Vec<u8>,
        slot: usize,
        samples: usize,
    }
    impl commit::Source for PausedWriter {
        fn length(&mut self) -> io::Result<u64> {
            Ok(self.bytes.len() as u64)
        }
        fn read_at(&mut self, offset: u64, output: &mut [u8]) -> io::Result<()> {
            commit::Source::read_at(&mut commit::Resident(&self.bytes), offset, output)?;
            let slot = if offset == 0 && output.len() == self.slot + commit::SLOT_SIZE {
                Some(self.slot)
            } else if offset == self.slot as u64 && output.len() == commit::SLOT_SIZE {
                Some(0)
            } else {
                None
            };
            if let Some(slot) = slot {
                self.samples += 1;
                if self.samples <= 2 {
                    output[slot + FOOTER_SIZE] ^= 0xff;
                }
            }
            Ok(())
        }
    }
    let old = Replay::from_records(CAMPAIGN.into(), &[records(1, &[1])]).unwrap();
    let mut bytes = old.bytes().to_vec();
    bytes.extend_from_slice(b"an incomplete append");
    let mut source = PausedWriter {
        bytes,
        slot: old.slot.unwrap(),
        samples: 0,
    };
    assert_eq!(
        commit::locate(&mut source).unwrap().revision,
        old.revision()
    );
    assert!(source.samples >= 3);
}

#[test]
fn rollback_reuses_one_recoverable_copy_and_refuses_nonordinary_stale_paths() {
    let first = Replay::from_records(CAMPAIGN.into(), &[records(1, &[1])]).unwrap();
    let file = Scratch::new(first.bytes());
    let second = save_in_place(&file.0, &segment(0, records(2, &[2]))).unwrap();
    drop(second);
    let second = Replay::open_path(&file.0).unwrap();
    let mut name = file.0.as_os_str().to_owned();
    name.push(".rollback.tmp");
    let temporary = PathBuf::from(name);
    std::fs::create_dir(&temporary).unwrap();
    assert!(Replay::roll_back(&file.0, first.revision()).is_err());
    assert_eq!(
        Replay::open_path(&file.0).unwrap().revision(),
        second.revision()
    );
    std::fs::remove_dir(&temporary).unwrap();
    std::fs::write(&temporary, b"an interrupted rollback copy").unwrap();
    Replay::roll_back(&file.0, first.revision()).unwrap();
    assert!(!temporary.exists());
    assert_eq!(file.bytes(), first.bytes());
    assert!(second.read_block(&second.index[1]).is_ok());
}

#[test]
fn an_append_names_its_revision_before_changing_the_recording() {
    let old = Replay::from_records(CAMPAIGN.into(), &[records(1, &[1, 2])]).unwrap();
    let file = Scratch::new(old.bytes());
    let next = segment(0, records(2, &[3]));
    let expected = old.append(&[records(2, &[3])]).unwrap();
    let mut replay = Replay::open_appendable(&file.0).unwrap();
    let mut prepared = None;
    replay
        .append_in_place(
            &kept(&next),
            |number, frame| restore(&next, number, frame),
            || Ok(()),
            |_| Ok(()),
            None,
            |revision| {
                assert_eq!(file.bytes(), old.bytes());
                assert_eq!(revision, expected.revision());
                prepared = Some(revision);
                Ok(())
            },
        )
        .unwrap();
    assert_eq!(prepared, Some(replay.revision()));
    assert_eq!(file.bytes(), expected.bytes());
}

#[test]
fn an_append_refused_by_its_journal_keeps_the_previous_save() {
    let old = Replay::from_records(CAMPAIGN.into(), &[records(1, &[1, 2])]).unwrap();
    let file = Scratch::new(old.bytes());
    let next = segment(0, records(2, &[3]));
    let mut replay = Replay::open_appendable(&file.0).unwrap();
    let error = replay
        .append_in_place(
            &kept(&next),
            |number, frame| restore(&next, number, frame),
            || Ok(()),
            |_| Ok(()),
            None,
            |_| Err(io::ErrorKind::Interrupted.into()),
        )
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    assert_eq!(file.bytes(), old.bytes());
    assert_eq!(replay.revision(), old.revision());
}

#[test]
fn rejected_preparation_never_replaces_the_recording_or_removes_its_torn_tail() {
    let old = Replay::from_records(CAMPAIGN.into(), &[records(1, &[1, 2])]).unwrap();
    let mut bytes = old.bytes().to_vec();
    bytes.extend_from_slice(b"an interrupted append");
    let file = Scratch::new(&bytes);
    let held = Replay::open_appendable(&file.0).unwrap();
    let next = segment(0, records(2, &[3]));
    let mut replay = Replay::open_appendable(&file.0).unwrap();
    let error = replay
        .append_in_place(
            &kept(&next),
            |number, frame| restore(&next, number, frame),
            || Ok(()),
            |_| Ok(()),
            None,
            |_| {
                Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "rejected preparation",
                ))
            },
        )
        .unwrap_err();
    assert_eq!(error.to_string(), "rejected preparation");
    assert_eq!(file.bytes(), bytes);
    assert_eq!(replay.revision(), old.revision());
    assert_eq!(
        held.read_block(&held.index[0]).unwrap(),
        old.read_block(&old.index[0]).unwrap()
    );
}

#[test]
fn chunked_appends_and_torn_tail_recovery_keep_the_last_committed_save() {
    let hours: Vec<_> = (1..=200).collect();
    let next_hours: Vec<_> = (721..=1000).collect();
    let old = Replay::from_records(CAMPAIGN.into(), &[records(1, &hours)]).unwrap();
    let expected = old.append(&[records(2, &next_hours)]).unwrap();
    assert!(expected.index.iter().any(|index| {
        expected
            .section_chunks(index, Section::Later(TableId::Clock))
            .unwrap()
            .len()
            > 1
    }));
    let file = Scratch::new(old.bytes());
    let saved = save_in_place(&file.0, &segment(7, records(2, &next_hours))).unwrap();
    assert_eq!(file.bytes(), expected.bytes());
    for index in &saved.index {
        assert_eq!(
            saved.read_block(index).unwrap(),
            expected.read_block(index).unwrap()
        );
    }
    let mut bytes = expected.bytes().to_vec();
    let slot = old.slot.unwrap();
    bytes[slot..slot + commit::SLOT_SIZE]
        .copy_from_slice(&old.bytes()[slot..slot + commit::SLOT_SIZE]);
    for end in [
        old.bytes().len(),
        old.bytes().len() + 1,
        expected.revision().trailer as usize,
        bytes.len() - 1,
    ] {
        let reopened = Replay::open(bytes[..end].to_vec()).unwrap();
        assert_eq!(reopened.revision(), old.revision());
        assert_eq!(reopened.index, old.index);
        for index in &reopened.index {
            assert_eq!(
                reopened.read_block(index).unwrap(),
                old.read_block(index).unwrap()
            );
        }
    }
}
