//! What the retained corpus can and cannot settle about the exact ability panel.
//!
//! `extractor/native/runs/uz_ability_panel/READ_LAW.md` section 6 lists the
//! operands the derivation needs, and this gate counts which of them the typed
//! replay world actually carries, so section 9 rests on numbers rather than on
//! a reading of the recorder arguments.
//!
//! IT CANNOT BE COUNTED YET, and the reason is a bootstrap rather than a
//! defect. Lane UB changed both ends of the per-id country account: the
//! recorder emits `tag_db_id_index.country_composition_version` (world_fold.rs,
//! commit 19588fe7) and the viewer REQUIRES it, because a fold that predates
//! per-id composition may have fused two countries under one TAG. Both changes
//! are on this lane and neither is on master, so every capture in the retained
//! corpus was written by a recorder that does not emit the marker -- rc_vanilla8
//! included, sealed though it is. Measured, not assumed:
//!
//!   rc_vanilla8  hours=319  identities=0  countries_status=DomainUnavailable
//!                "countries: tag_db_id_index.country_composition_version is
//!                 None, not 2"
//!   rc_vanilla7  hours=238  identities=0  countries_status=DomainUnavailable
//!                "tag_db_id_index carries the retired v1 country account
//!                 member withheld_country_keys"
//!
//! So the operand count below is `#[ignore]`d rather than retargeted: asserting
//! zeros over a refused capture would read as "the domain is absent" when it
//! means "the recording was refused", which is the green tick that hides the
//! consequence. The first capture that can answer it is one taken with THIS
//! lane's recorder, after it lands. What CAN be asserted today, and is, is the
//! refusal itself.

use replay_core::{open_recording, Leaf, WorldTimeline};
use std::path::PathBuf;

/// Per-hour counts of the five D68 session constants that survive the country
/// refusal on rc_vanilla8. MEASURED off the capture, not copied from the
/// fixture's shape: a constant the recording never carried must not be
/// asserted into existence.
///
///   rc_vanilla8 session constants surviving the country refusal:
///   dlc=213 start_date_raw=213 save_version=213 session=213 version=213
///   of 319 hours
///
/// 213 and not 319: this capture does not carry a D68 header on every hour.
/// All five move TOGETHER, which is the thing worth watching -- they are one
/// grain, so a rule that withholds one of them on some subset of hours splits
/// the five counts apart even when the hour count does not move.
const EXPECTED_MEASURED_DLC: usize = 213;
const EXPECTED_MEASURED_START_DATE: usize = 213;
const EXPECTED_MEASURED_SAVE_VERSION: usize = 213;
const EXPECTED_MEASURED_SESSION: usize = 213;
const EXPECTED_MEASURED_VERSION: usize = 213;

/// The VALUES of those constants, measured off the same capture. A count says
/// a leaf is still there; only the value says it is still the one the recording
/// carried.
const EXPECTED_START_DATE_RAW: u32 = 60_759_371;
const EXPECTED_SAVE_VERSION: u32 = 33;
const EXPECTED_SESSION: u32 = 43_830;
const EXPECTED_VERSION: &str = "Operation Postern v1.19.2.0.a729";

/// A worktree carries no `captures/`, so the retained corpus is addressed the
/// way `map_viewer::grading_law` addresses its own: an env override over the
/// main checkout's path.
fn capture() -> PathBuf {
    std::env::var_os("BATTLE_PLAN_RECORDING_CAPTURES_ROOT").or_else(|| std::env::var_os("HOI4_RECORDING_CAPTURES_ROOT"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"captures"))
        .join("rc_vanilla8/native_session.battleplan")
}

/// A pre-composition capture is refused BY NAME.
///
/// This is the half of the question the corpus can answer today, and it is not
/// a placeholder: it holds the two halves of the lane's own cost rule against a
/// real 319-hour recording. The country half must be refused -- a fold with no
/// per-id account cannot say a TAG addresses one country -- and the SESSION
/// half must survive, because the account says nothing about the DLC word. The
/// second half is what Codex round 54 found broken: the refusal used to null
/// the whole D68 header and this capture read `dlc_masks={}`.
///
/// kills: the refusal going silent about its cause, and the refusal widening
/// back over the session constants.
#[test]
fn a_pre_composition_capture_is_refused_by_name() {
    let path = capture();
    assert!(
        path.is_file(),
        "this gate requires the retained capture at {}",
        path.display()
    );
    let timeline = WorldTimeline::open(open_recording(&path).expect("open rc_vanilla8"))
        .expect("world timeline");
    let hours = timeline.hours().to_vec();
    assert_eq!(hours.len(), 319, "rc_vanilla8 is a 319-hour capture");

    // EVERY HOUR, not hour 0. Checking the refusal on one frame passed a
    // mutation that made the refusal conditional on the hour carrying no DLC
    // word, so the hours that DID carry one published pre-composition country
    // data while hour 0 still looked refused (Codex round 56).
    let mut dlc_masks = std::collections::BTreeSet::new();
    let mut start_dates = std::collections::BTreeSet::new();
    let mut save_versions = std::collections::BTreeSet::new();
    let mut sessions = std::collections::BTreeSet::new();
    let mut versions = std::collections::BTreeSet::new();
    let mut measured_dlc = 0usize;
    let mut measured_start_date = 0usize;
    let mut measured_save_version = 0usize;
    let mut measured_session = 0usize;
    let mut measured_version = 0usize;
    for hour in &hours {
        let frame = timeline.frame(*hour).expect("frame");
        assert_eq!(
            frame.country_identities().len(),
            0,
            "a fold with no per-id account names no country @ hour {hour}"
        );
        assert_eq!(
            frame.country_identities_status(),
            replay_core::ViewStatus::DomainUnavailable,
            "and says so as unavailable rather than as an empty domain @ hour {hour}"
        );
        // The `countries` refusal ITSELF has to name the ACCOUNT. Filtering the
        // whole warning list passed a mutation that made the country refusal
        // cause-free while a sibling member still carried the cause (Codex
        // round 55) -- and pinning one CAUSE was wrong too: this capture is not
        // uniform. Some hours carry an index with no
        // `country_composition_version`; others carry a retired v1 account
        // member (`withheld_country_keys`). Both are refusals and both name
        // `tag_db_id_index`, which is the law -- the refusal says which account
        // it could not read.
        assert!(
            frame.coverage_warnings().iter().any(|warning| {
                warning.starts_with("countries: ") && warning.contains("tag_db_id_index")
            }),
            "the countries refusal must NAME the account it wanted @ hour {hour}: {:?}",
            frame.coverage_warnings()
        );
        // EVERY session constant, not just the DLC word. Watching one of the
        // five let a regression withhold another one conditionally -- on the
        // DLC word being absent, or on which of the three refusal causes the
        // hour took -- and this gate still passed (Codex round 57). Counted
        // rather than sampled so a leaf that goes Unobtained on ONE hour of the
        // 319 moves a number.
        let header = frame.session_header();
        if let Leaf::Measured(mask) = header.dlc_mask {
            dlc_masks.insert(mask);
        }
        if let Leaf::Measured(value) = header.start_date_raw {
            start_dates.insert(value);
        }
        if let Leaf::Measured(value) = header.save_version {
            save_versions.insert(value);
        }
        if let Leaf::Measured(value) = header.session {
            sessions.insert(value);
        }
        if let Leaf::Measured(value) = &header.version {
            versions.insert(value.clone());
        }
        // The five move as ONE. Counting them separately is what makes a rule
        // that withholds exactly one of them visible; the VALUE sets below are
        // what makes a rule that REWRITES one of them visible (Codex round 58 --
        // a clamp on save_version left every count at 213).
        measured_dlc += usize::from(matches!(header.dlc_mask, Leaf::Measured(_)));
        measured_start_date += usize::from(matches!(header.start_date_raw, Leaf::Measured(_)));
        measured_save_version += usize::from(matches!(header.save_version, Leaf::Measured(_)));
        measured_session += usize::from(matches!(header.session, Leaf::Measured(_)));
        measured_version += usize::from(matches!(header.version, Leaf::Measured(_)));
    }

    println!(
        "rc_vanilla8 session constants surviving the country refusal: \
         dlc={measured_dlc} start_date_raw={measured_start_date} \
         save_version={measured_save_version} session={measured_session} \
         version={measured_version} of {} hours",
        hours.len()
    );
    println!(
        "rc_vanilla8 session constant VALUES: dlc_masks={dlc_masks:?} \
         start_dates={start_dates:?} save_versions={save_versions:?} \
         sessions={sessions:?} versions={versions:?}"
    );
    // The VALUES, not just how many hours carried one. A count cannot see a
    // rule that REWRITES a constant it leaves present -- Codex round 58 named a
    // clamp on `save_version` that held all five counts at 213. These are
    // session constants, so each is one value across the whole capture, and
    // each is pinned to what the capture actually carries.
    assert_eq!(
        (
            start_dates.len(),
            save_versions.len(),
            sessions.len(),
            versions.len()
        ),
        (1, 1, 1, 1),
        "a session CONSTANT that takes more than one value over one session is \
         not a constant: {start_dates:?} {save_versions:?} {sessions:?} {versions:?}"
    );
    assert_eq!(
        (
            start_dates.iter().next().copied(),
            save_versions.iter().next().copied(),
            sessions.iter().next().copied(),
            versions.iter().next().map(String::as_str)
        ),
        (
            Some(EXPECTED_START_DATE_RAW),
            Some(EXPECTED_SAVE_VERSION),
            Some(EXPECTED_SESSION),
            Some(EXPECTED_VERSION)
        ),
        "a session constant was rewritten on its way through the country refusal"
    );
    // MEASURED on the capture, then pinned. Each is the count of hours whose
    // typed header still carries that constant after the country half was
    // refused; the account says nothing about any of them, so a drop here is a
    // refusal widening back over the session.
    assert_eq!(
        (
            measured_dlc,
            measured_start_date,
            measured_save_version,
            measured_session,
            measured_version
        ),
        (
            EXPECTED_MEASURED_DLC,
            EXPECTED_MEASURED_START_DATE,
            EXPECTED_MEASURED_SAVE_VERSION,
            EXPECTED_MEASURED_SESSION,
            EXPECTED_MEASURED_VERSION
        ),
        "a session constant went missing on some hour of this capture"
    );

    // ...and the SESSION constants survive that refusal, over the whole
    // capture. This is the half of the old gate that was real protection, and
    // it belongs HERE and not only in replay_core, because this capture takes a
    // DIFFERENT admission branch: it carries a `tag_db_id_index` whose VERSION
    // is missing, where the unit test carries no index at all. Before Codex
    // round 54 narrowed the D68 refusal this read `{}`.
    assert_eq!(
        dlc_masks,
        std::collections::BTreeSet::from([0x0003_EDFF]),
        "the DLC word is a session constant a country refusal does not take"
    );
}

/// The operand count itself. See the module note: it needs a capture taken with
/// this lane's recorder, which cannot exist until this lane lands.
#[ignore = "needs a capture folded with per-id composition; every capture in the \
            retained corpus predates this lane's recorder. Re-baseline the four \
            counts from the printed line and delete this attribute."]
#[test]
fn rc_vanilla8_carries_the_evaluators_operands_the_read_law_asks_for() {
    let path = capture();
    assert!(
        path.is_file(),
        "the operand coverage gate requires the retained capture at {}",
        path.display()
    );
    let timeline = WorldTimeline::open(open_recording(&path).expect("open rc_vanilla8"))
        .expect("world timeline");
    let hours = timeline.hours().to_vec();
    println!("MEASURED hours={}", hours.len());

    let mut dlc_masks = std::collections::BTreeSet::new();
    let mut hours_with_focus = 0usize;
    let mut hours_with_flags = 0usize;
    let mut hours_with_leaders = 0usize;
    let mut hours_with_assignments = 0usize;
    let mut max_leaders_in_an_hour = 0usize;

    for hour in &hours {
        let frame = timeline.frame(*hour).expect("frame");
        if let Leaf::Measured(mask) = frame.session_header().dlc_mask {
            dlc_masks.insert(mask);
        }
        if frame
            .focus_by_country()
            .values()
            .any(|view| matches!(view.completed, Leaf::Measured(_)))
        {
            hours_with_focus += 1;
        }
        if frame
            .scripted_gui()
            .country_flag_states
            .values()
            .any(|states| matches!(states, Leaf::Measured(_)))
        {
            hours_with_flags += 1;
        }
        let leaders: usize = frame
            .character_rosters_exact()
            .values()
            .map(|roster| roster.unit_leaders.len())
            .sum();
        if leaders > 0 {
            hours_with_leaders += 1;
            max_leaders_in_an_hour = max_leaders_in_an_hour.max(leaders);
        }
        if frame.character_rosters_exact().values().any(|roster| {
            roster
                .unit_leaders
                .iter()
                .any(|leader| leader.assignment.is_some())
        }) {
            hours_with_assignments += 1;
        }
    }

    println!(
        "UZ operand coverage over rc_vanilla8: hours={} dlc_masks={dlc_masks:?} \
         hours_with_focus={hours_with_focus} hours_with_flags={hours_with_flags} \
         hours_with_leaders={hours_with_leaders} \
         hours_with_assignments={hours_with_assignments} \
         max_leaders_in_an_hour={max_leaders_in_an_hour}",
        hours.len()
    );

    // MEASURED on rc_vanilla8 (319 hours) and left here as the re-baseline:
    // every one of these is 0 because the capture is REFUSED, not because the
    // domain is absent, which is exactly why this test is ignored. Re-measure
    // them against a capture folded with per-id composition.
    assert_eq!(dlc_masks, std::collections::BTreeSet::from([0x0003_EDFF]));
    assert!(
        hours_with_focus > 0,
        "d12 must carry a measured completed-focus set once the capture is readable"
    );
    let _ = (hours_with_flags, hours_with_leaders, hours_with_assignments, max_leaders_in_an_hour);
}
