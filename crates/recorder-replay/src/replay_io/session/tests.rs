use super::*;
use crate::{
    replay_io::{Destination, finalize_session_ahead},
    test_support::{
        ALLOCATION_CAMPAIGN, RELOAD, THIRD_LOAD, allocation_records, allocation_test_directory,
        load_segment_records, recorded_frames, write_campaign_journal, write_capture,
    },
};

const FIRST: [(u64, i32); 3] = [(9, 2), (9, 3), (9, 4)];
const BOTH: [(u64, i32); 5] = [(9, 2), (9, 3), (9, 4), (10, 5), (10, 6)];

fn campaign() -> &'static str {
    std::str::from_utf8(ALLOCATION_CAMPAIGN).unwrap()
}

struct Root(PathBuf);

impl Root {
    fn new(name: &str) -> Self {
        Self(allocation_test_directory(name))
    }

    fn target(&self) -> PathBuf {
        self.0.join("recordings")
    }

    fn settings(&self) -> PathBuf {
        self.0.join("settings.json")
    }

    fn first_session(&self) -> PathBuf {
        let session = self.0.join("sessions").join("1");
        write_campaign_journal(&session, &allocation_records());
        session
    }

    fn later_session(&self, name: &str) -> PathBuf {
        let session = self.0.join("sessions").join(name);
        write_capture(
            &session.join(campaign()),
            0,
            &load_segment_records(10, 5..=6, RELOAD),
            true,
        );
        session
    }

    fn end(&self, session: &Path, destination: Destination) -> crate::replay_io::Finalized {
        let settings = self.settings();
        let naming = Naming {
            localisation: None,
            settings: &settings,
            report: None,
        };
        let ended = finalize_session_ahead(
            session,
            &self.target(),
            &|| Ok(()),
            Live::default(),
            naming,
            &|_| destination,
        )
        .unwrap();
        assert!(ended.errors.is_empty(), "{:?}", ended.errors);
        ended
    }

    fn saved_first(&self) -> (PathBuf, Vec<u8>) {
        let session = self.first_session();
        let ended = self.end(&session, Destination::Recording);
        let recording = ended.replays[0].clone();
        assert_eq!(recorded_frames(&recording), FIRST);
        let bytes = fs::read(&recording).unwrap();
        (recording, bytes)
    }
}

impl Drop for Root {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn newest_frames(path: &Path) -> Vec<(u64, i32)> {
    let replay = Replay::open_newest(path).unwrap();
    replay
        .index
        .iter()
        .flat_map(|block| replay.read_block(block).unwrap())
        .map(|frame| (frame.branch, frame.hour))
        .collect()
}

#[test]
fn a_new_campaign_s_hours_wait_unseen_in_one_file_until_a_save_names_it() {
    let root = Root::new("session-unseen-until-saved");
    let (target, settings) = (root.target(), root.settings());
    let naming = Naming {
        localisation: None,
        settings: &settings,
        report: None,
    };
    let session = root.first_session();
    let mut live = Live::default();
    assert!(
        live.advance(&session, &target, &|_| false, naming, &|| false)
            .unwrap()
    );
    let waiting = publication::unsaved_path(&target, campaign());
    assert_eq!(locate(&target, campaign()).unwrap(), Some(waiting.clone()));
    assert_eq!(
        recording_files::recording_of(&target, ALLOCATION_CAMPAIGN).unwrap(),
        None
    );
    assert!(is_not_saved(
        &Replay::open_path(&waiting)
            .err()
            .expect("no saved revision")
    ));
    assert_eq!(newest_frames(&waiting), FIRST);
    assert!(
        journal_campaigns(&session).unwrap().is_empty(),
        "the journal is retired once written"
    );
    let held = unsaved(&target, campaign()).unwrap().expect("hours wait");
    assert_eq!(
        (held.hours, held.first_hour, held.last_hour),
        (3, Some(2), Some(4))
    );
    assert_eq!(
        live.unsaved(campaign()).map(|unsaved| unsaved.hours),
        Some(3)
    );
    let length = fs::metadata(&waiting).unwrap().len();
    let saved = live
        .save(&session, &target, campaign(), naming)
        .unwrap()
        .expect("a recording");
    assert_eq!(saved.parent(), Some(target.as_path()));
    assert!(!waiting.exists());
    assert!(fs::metadata(&saved).unwrap().len() > length);
    assert_eq!(recorded_frames(&saved), FIRST);
    assert_eq!(unsaved(&target, campaign()).unwrap(), None);
    assert_eq!(live.unsaved(campaign()), None);
}

#[test]
fn discarding_a_session_that_continued_a_saved_recording_cuts_it_back_to_what_it_held() {
    let root = Root::new("session-discard-continued");
    let (target, settings) = (root.target(), root.settings());
    let naming = Naming {
        localisation: None,
        settings: &settings,
        report: None,
    };
    let (recording, saved) = root.saved_first();
    let session = root.later_session("2");
    let mut live = Live::default();
    live.advance(&session, &target, &|_| false, naming, &|| false)
        .unwrap();
    assert_eq!(
        locate(&target, campaign()).unwrap(),
        Some(recording.clone())
    );
    assert!(fs::metadata(&recording).unwrap().len() > saved.len() as u64);
    assert_eq!(recorded_frames(&recording), FIRST);
    assert_eq!(newest_frames(&recording), BOTH);
    let held = unsaved(&target, campaign()).unwrap().expect("hours wait");
    assert_eq!(
        (held.hours, held.first_hour, held.last_hour),
        (2, Some(5), Some(6))
    );
    live.discard(&session, &target, campaign(), Discarded::Session)
        .unwrap();
    assert_eq!(
        fs::read(&recording).unwrap(),
        saved,
        "the recording is as it was before the session"
    );
    assert!(journal_campaigns(&session).unwrap().is_empty());
    assert!(!session.join(campaign()).join(WRITING_FILE).exists());
    assert_eq!(unsaved(&target, campaign()).unwrap(), None);
}

const LONG: std::ops::RangeInclusive<i32> = 5..=1450;

fn recording_while_held(
    root: &Root,
    finished: bool,
    paused: Option<(&Path, usize)>,
) -> (PathBuf, Live) {
    let settings = root.settings();
    let naming = Naming {
        localisation: None,
        settings: &settings,
        report: None,
    };
    let session = root.0.join("sessions").join("2");
    write_capture(
        &session.join(campaign()),
        0,
        &load_segment_records(10, LONG, RELOAD),
        finished,
    );
    let campaign_dir = session.join(campaign());
    let in_newest = std::cell::Cell::new(false);
    let stop = || {
        let Some((recording, blocks)) = paused else {
            return false;
        };
        let held = in_newest.get();
        in_newest.set(
            replay_journal::columnar_journal::segments(&campaign_dir).is_ok_and(|held| held.len() == 1),
        );
        held && Replay::open_path(recording).is_ok_and(|saved| saved.index.len() > blocks)
    };
    let mut live = Live::default();
    live.advance(&session, &root.target(), &|_| true, naming, &stop)
        .unwrap();
    assert_eq!(
        journal_campaigns(&session).unwrap(),
        [campaign()],
        "the writer holds the journal"
    );
    (session, live)
}

fn assert_discarded(root: &Root, session: &Path, recording: &Path, before: &[u8]) {
    assert_eq!(
        fs::read(recording).unwrap(),
        before,
        "the recording is as it was before the session, to the byte"
    );
    assert!(journal_campaigns(session).unwrap().is_empty());
    assert!(!session.join(campaign()).join(WRITING_FILE).exists());
    assert!(!session.join(campaign()).join(STATE_FILE).exists());
    assert_eq!(unsaved(&root.target(), campaign()).unwrap(), None);
}

#[test]
fn stop_s_discard_after_the_collector_detached_leaves_the_recording_as_it_was() {
    let root = Root::new("session-discard-detached");
    let (recording, seed) = root.saved_first();
    let blocks = Replay::open_path(&recording).unwrap().index.len();
    let (session, mut live) = recording_while_held(&root, true, Some((&recording, blocks)));
    assert!(
        recorded_frames(&recording).contains(&(10, 5)),
        "Auto save saved the session's first block"
    );
    live.discard(&session, &root.target(), campaign(), Discarded::Session)
        .unwrap();
    assert_discarded(&root, &session, &recording, &seed);
}

#[test]
fn stop_s_discard_after_the_game_exited_leaves_the_recording_as_it_was() {
    let root = Root::new("session-discard-exited");
    let (recording, seed) = root.saved_first();
    let (session, mut live) = recording_while_held(&root, false, None);
    assert!(recorded_frames(&recording).contains(&(10, 5)));
    live.discard(&session, &root.target(), campaign(), Discarded::Session)
        .unwrap();
    assert_discarded(&root, &session, &recording, &seed);
}

#[test]
fn stop_s_discard_of_a_session_that_began_the_recording_leaves_no_file() {
    let root = Root::new("session-discard-began");
    let target = root.target();
    let (session, mut live) = recording_while_held(&root, false, None);
    let recording = locate(&target, campaign())
        .unwrap()
        .expect("the session's recording");
    assert!(
        !is_unsaved_recording(&recording),
        "Auto save saved and named it"
    );
    live.discard(&session, &target, campaign(), Discarded::Session)
        .unwrap();
    assert!(!recording.exists());
    assert!(!publication::unsaved_path(&target, campaign()).exists());
    assert_eq!(locate(&target, campaign()).unwrap(), None);
    assert!(journal_campaigns(&session).unwrap().is_empty());
    assert!(!session.join(campaign()).join(WRITING_FILE).exists());
}

#[test]
fn a_discard_whose_collector_never_detached_is_recorded_at_once_and_finished_when_taken_up() {
    let root = Root::new("session-discard-never-detached");
    let target = root.target();
    let (recording, seed) = root.saved_first();
    let (session, live) = recording_while_held(&root, false, None);
    assert!(recorded_frames(&recording).contains(&(10, 5)));
    drop(live);
    let campaign_dir = session.join(campaign());
    let (_, _, newest) = replay_journal::columnar_journal::segments(&campaign_dir)
        .unwrap()
        .pop()
        .unwrap();
    let collector = fs::File::open(&newest).unwrap();
    note_discard(&session, &target, campaign()).unwrap();
    reconcile_session(&session, &target).unwrap();
    assert_eq!(fs::read(&recording).unwrap(), seed, "cut back at once");
    assert_eq!(journal_campaigns(&session).unwrap(), [campaign()]);
    assert_eq!(noted_discards(&session).unwrap(), [campaign()]);
    reconcile_session(&session, &target).unwrap();
    assert!(newest.exists());
    drop(collector);
    reconcile_session(&session, &target).unwrap();
    assert_discarded(&root, &session, &recording, &seed);
    assert!(noted_discards(&session).unwrap().is_empty());
}

#[test]
fn stop_s_discard_keeps_what_the_user_s_save_made_of_the_recording_in_the_session() {
    let root = Root::new("session-discard-after-save");
    let (target, settings) = (root.target(), root.settings());
    let naming = Naming {
        localisation: None,
        settings: &settings,
        report: None,
    };
    root.saved_first();
    let session = root.later_session("2");
    let mut live = Live::default();
    live.advance(&session, &target, &|_| false, naming, &|| false)
        .unwrap();
    let saved = live
        .save(&session, &target, campaign(), naming)
        .unwrap()
        .expect("a recording");
    let kept = fs::read(&saved).unwrap();
    write_capture(
        &session.join(campaign()),
        1,
        &load_segment_records(11, 7..=8, THIRD_LOAD),
        true,
    );
    live.advance(&session, &target, &|_| false, naming, &|| false)
        .unwrap();
    assert_ne!(fs::read(&saved).unwrap(), kept);
    live.discard(&session, &target, campaign(), Discarded::Session)
        .unwrap();
    assert_eq!(fs::read(&saved).unwrap(), kept);
    assert_eq!(recorded_frames(&saved), BOTH);
}

#[test]
fn a_discard_a_crash_interrupted_is_finished_before_the_recording_is_written_again() {
    let root = Root::new("session-discard-interrupted");
    let (target, settings) = (root.target(), root.settings());
    let naming = Naming {
        localisation: None,
        settings: &settings,
        report: None,
    };
    let (recording, saved) = root.saved_first();
    let session = root.later_session("2");
    let mut live = Live::default();
    live.advance(&session, &target, &|_| false, naming, &|| false)
        .unwrap();
    drop(live);
    let campaign_dir = session.join(campaign());
    let noted = read_writing(&campaign_dir)
        .unwrap()
        .expect("the session notes what it writes into");
    assert_eq!(
        noted.before.map(|before| before.length),
        Some(saved.len() as u64)
    );
    write_writing(
        &campaign_dir,
        Writing {
            discarding: true,
            ..noted
        },
    )
    .unwrap();
    assert_eq!(newest_frames(&recording), BOTH);
    reconcile_session(&session, &target).unwrap();
    assert_eq!(fs::read(&recording).unwrap(), saved);
    assert!(!campaign_dir.join(WRITING_FILE).exists());
    let next = root.later_session("3");
    let ended = root.end(&next, Destination::Recording);
    assert_eq!(recorded_frames(&ended.replays[0]), BOTH);
}

#[test]
fn hours_a_session_s_end_leaves_waiting_are_discarded_or_saved_later_in_the_same_file() {
    let root = Root::new("session-waiting-later");
    let (target, settings) = (root.target(), root.settings());
    let naming = Naming {
        localisation: None,
        settings: &settings,
        report: None,
    };
    let (recording, saved) = root.saved_first();
    let ended = root.end(&root.later_session("2"), Destination::Unsaved);
    assert_eq!(
        (ended.replays.as_slice(), ended.unsaved.as_slice()),
        (&[][..], &[recording.clone()][..])
    );
    assert_eq!(recorded_frames(&recording), FIRST);
    assert_eq!(
        unsaved(&target, campaign())
            .unwrap()
            .map(|unsaved| unsaved.hours),
        Some(2)
    );
    discard_waiting(&target, campaign()).unwrap();
    assert_eq!(fs::read(&recording).unwrap(), saved);
    root.end(&root.later_session("3"), Destination::Unsaved);
    assert_eq!(
        save_waiting(&target, campaign(), naming).unwrap(),
        Some(recording.clone())
    );
    assert_eq!(recorded_frames(&recording), BOTH);
    assert_eq!(unsaved(&target, campaign()).unwrap(), None);
}

const SECOND: &str = "8bb2e3e7-f322-45b0-9514-010098493828";

fn write_second_campaign(session: &Path) {
    let mut records = load_segment_records(12, 5..=6, RELOAD);
    for record in &mut records {
        if let replay_capture_spool::spool::Record::Frame(frame) = record {
            frame.tables[TableId::Campaign.index()] = replay_frame::frame::TableUpdate::Snapshot(
                replay_data::rows::TableRows::Campaign(vec![replay_data::rows::CampaignRow {
                    id: SECOND.as_bytes().to_vec(),
                }]),
            );
        }
    }
    write_capture(&session.join(SECOND), 0, &records, false);
}

#[test]
fn two_campaigns_of_one_process_are_both_published_while_the_second_has_no_revision_yet() {
    let root = Root::new("session-two-campaigns");
    let (target, settings) = (root.target(), root.settings());
    let naming = Naming {
        localisation: None,
        settings: &settings,
        report: None,
    };
    let session = root.first_session();
    write_second_campaign(&session);
    let mut live = Live::default();
    live.advance(&session, &target, &|_| true, naming, &|| false)
        .unwrap();
    let second = publication::unsaved_path(&target, SECOND);
    assert!(no_revision(&second), "the second campaign's hours wait for its first block");
    let ended = finalize_session_ahead(
        &session,
        &target,
        &|| Ok(()),
        live,
        naming,
        &|_| Destination::Recording,
    )
    .unwrap();
    assert!(ended.errors.is_empty(), "{:?}", ended.errors);
    assert!(ended.failed_campaigns.is_empty());
    assert!(ended.unsaved.is_empty());
    assert_eq!(
        ended.saved_campaigns,
        BTreeSet::from([campaign().to_owned(), SECOND.to_owned()])
    );
    let [first, later] = ended.replays.as_slice() else {
        panic!("two recordings: {:?}", ended.replays);
    };
    assert_eq!(recorded_frames(first), FIRST);
    assert_eq!(recorded_frames(later), [(12, 5), (12, 6)]);
    for (path, id) in [(first, campaign()), (later, SECOND)] {
        assert_eq!(path.parent(), Some(target.as_path()));
        assert_eq!(unsaved(&target, id).unwrap(), None, "nothing of {id} waits");
        assert!(!session.join(id).join(WRITING_FILE).exists());
    }
    assert!(!second.exists());
}

#[test]
fn a_campaign_short_of_its_first_block_is_published_at_the_session_s_end() {
    let root = Root::new("session-short-campaign");
    let (target, settings) = (root.target(), root.settings());
    let naming = Naming {
        localisation: None,
        settings: &settings,
        report: None,
    };
    let session = root.0.join("sessions").join("1");
    write_second_campaign(&session);
    let mut live = Live::default();
    live.advance(&session, &target, &|_| true, naming, &|| false)
        .unwrap();
    assert!(no_revision(&publication::unsaved_path(&target, SECOND)));
    let ended = finalize_session_ahead(
        &session,
        &target,
        &|| Ok(()),
        live,
        naming,
        &|_| Destination::Recording,
    )
    .unwrap();
    assert!(ended.errors.is_empty(), "{:?}", ended.errors);
    let [published] = ended.replays.as_slice() else {
        panic!("one recording: {:?}", ended.replays);
    };
    assert_eq!(published.parent(), Some(target.as_path()));
    assert_eq!(recorded_frames(published), [(12, 5), (12, 6)]);
    assert_eq!(unsaved(&target, SECOND).unwrap(), None);
}

#[test]
fn a_campaign_whose_recorder_died_before_its_first_block_is_written_again_from_its_journal() {
    let root = Root::new("session-short-campaign-killed");
    let (target, settings) = (root.target(), root.settings());
    let naming = Naming {
        localisation: None,
        settings: &settings,
        report: None,
    };
    let session = root.0.join("sessions").join("1");
    write_second_campaign(&session);
    let mut live = Live::default();
    live.advance(&session, &target, &|_| true, naming, &|| false)
        .unwrap();
    drop(live);
    let left = publication::unsaved_path(&target, SECOND);
    assert!(no_revision(&left));
    assert_eq!(locate(&target, SECOND).unwrap(), Some(left.clone()));
    let ended = root.end(&session, Destination::Recording);
    let [published] = ended.replays.as_slice() else {
        panic!("one recording: {:?}", ended.replays);
    };
    assert_eq!(recorded_frames(published), [(12, 5), (12, 6)]);
    assert!(!left.exists());
}

#[test]
fn a_capture_its_collector_never_finished_prints_one_stop_line() {
    let root = Root::new("inspect-stop");
    let session = root.0.join("sessions").join("1");
    write_capture(&session.join(campaign()), 0, &allocation_records(), false);
    let ended = root.end(&session, Destination::Recording);
    let replay = Replay::open_newest(&ended.replays[0]).unwrap();
    assert_eq!(
        crate::replay_io::stop_lines(&replay),
        ["stop spool=1 capture=0 branch=9 last_hour=Some(4): the collector ended before finishing this file"]
    );
    let finished = Root::new("inspect-no-stop");
    let ended = finished.end(&finished.first_session(), Destination::Recording);
    let replay = Replay::open_newest(&ended.replays[0]).unwrap();
    assert!(crate::replay_io::stop_lines(&replay).is_empty());
}

#[test]
fn a_discard_accepted_after_a_deferred_save_keeps_the_campaign_from_being_published() {
    let mut live = Live::default();
    live.campaigns.insert("a".into(), Box::new(Campaign { saving: true, ..Campaign::default() }));
    live.forget_save("a");
    assert!(!live.campaigns["a"].saving, "end_campaign no longer saves it on the deferred Save's word");
    live.forget_save("unknown");
}
