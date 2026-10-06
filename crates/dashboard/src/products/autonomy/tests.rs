use crate::test_fixtures::{CAMPAIGN, records, world_frame};
use replay::{
    container::Replay,
    frame::{DecodedFrame, TableView},
    rows::{TableId, TableRows},
};
use std::sync::{Arc, atomic::AtomicBool};

/// A subject's autonomy: (subject link, overlord link or none, state, score,
/// lower, upper, previous and next exist).
type Status = (u32, Option<u32>, &'static [u8], i64, i64, i64, bool, bool);

/// A world frame (Austria included) with the autonomy and subject tables.
fn autonomy_frame(hour: i32, statuses: &[Status]) -> DecodedFrame {
    use replay::rows::{AutonomyRow, SubjectRow};
    let mut frame = world_frame(hour, true);
    let mut statuses = statuses.to_vec();
    // Rows go in key order: Autonomy by subject, Subjects by overlord then subject.
    statuses.sort_by_key(|status| status.0);
    let mut relations = statuses.clone();
    relations.sort_by_key(|status| (status.1, status.0));
    *frame.tables[TableId::Autonomy.index()] = TableView::Complete(TableRows::Autonomy(
        statuses
            .iter()
            .map(
                |&(subject, _, state, score, lower_score, upper_score, previous, next)| {
                    AutonomyRow {
                        subject,
                        score,
                        lower_score,
                        upper_score,
                        last_modified_bits: 0,
                        state: state.to_vec(),
                        previous: previous.then(|| b"lower".to_vec()),
                        next: next.then(|| b"higher".to_vec()),
                    }
                },
            )
            .collect(),
    ));
    *frame.tables[TableId::Subjects.index()] = TableView::Complete(TableRows::Subjects(
        relations
            .iter()
            .filter_map(|&(subject, overlord, state, ..)| {
                overlord.map(|overlord| SubjectRow {
                    overlord,
                    subject,
                    start_hour: 0,
                    end_hour: 0,
                    cancel: 0,
                    autonomy_ratio: 20_000,
                    state: state.to_vec(),
                })
            })
            .collect(),
    ));
    frame
}

#[test]
fn autonomy_runs_join_equal_hours_mark_level_changes_and_never_cross_gaps() {
    use crate::products::autonomy::scan;
    let puppet: Status = (
        31,
        Some(11),
        b"autonomy_puppet",
        250_000_000,
        200_000_000,
        300_000_000,
        true,
        true,
    );
    let colony: Status = (
        31,
        Some(11),
        b"autonomy_colony",
        300_000_000,
        300_000_000,
        400_000_000,
        true,
        true,
    );
    let risen: Status = (
        31,
        Some(11),
        b"autonomy_puppet",
        260_000_000,
        200_000_000,
        300_000_000,
        true,
        true,
    );
    let released: Status = (
        31,
        None,
        b"autonomy_colony",
        300_000_000,
        300_000_000,
        400_000_000,
        true,
        true,
    );
    // Italy is Germany's subject too, named by Germany's other link, at a modded level.
    let italy: Status = (21, Some(9001), b"modded_level", 10, 0, 100, false, true);
    let mut frames = vec![
        autonomy_frame(100, &[puppet, italy]),
        autonomy_frame(101, &[puppet, italy]),
        autonomy_frame(102, &[risen, italy]),
        autonomy_frame(103, &[colony, italy]),
        // 104 is a gap.
        autonomy_frame(105, &[colony, italy]),
        // Austria's relation ends: no overlord.
        autonomy_frame(106, &[released, italy]),
    ];
    let mut incomplete = autonomy_frame(107, &[released]);
    *incomplete.tables[TableId::Subjects.index()] = TableView::Unavailable {
        reason: "fixture".into(),
    };
    frames.push(incomplete);
    frames.push(autonomy_frame(108, &[released]));
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    let scanned = scan(&replay, 1, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        scanned.levels,
        ["modded_level", "autonomy_puppet", "autonomy_colony"]
    );
    assert_eq!(scanned.lowest_lower, [0, 200_000_000, 300_000_000]);
    // The gap at 104 and the incomplete hour 107 are not recorded.
    assert_eq!(
        scanned
            .recorded
            .iter()
            .map(|span| (span.first_hour, span.last_hour))
            .collect::<Vec<_>>(),
        [(100, 103), (105, 106), (108, 108)]
    );
    let (tag, runs) = &scanned.subjects[1];
    assert_eq!(tag, "D01 mod");
    let runs: Vec<_> = runs
        .iter()
        .map(|run| {
            (
                run.first_hour,
                run.last_hour,
                run.starts_segment,
                run.overlord.as_deref(),
                run.level,
                run.score_raw.as_str(),
                run.ratio_raw.as_deref(),
            )
        })
        .collect();
    assert_eq!(
        runs,
        [
            (100, 101, true, Some("GER"), 1, "250000000", Some("20000")),
            (102, 102, false, Some("GER"), 1, "260000000", Some("20000")),
            (103, 103, false, Some("GER"), 2, "300000000", Some("20000")),
            (105, 105, true, Some("GER"), 2, "300000000", Some("20000")),
            (106, 106, false, None, 2, "300000000", None),
            (108, 108, true, None, 2, "300000000", None),
        ]
    );
    let (tag, runs) = &scanned.subjects[0];
    assert_eq!(tag, "ITA");
    assert_eq!(
        runs.iter()
            .map(|run| (
                run.first_hour,
                run.last_hour,
                run.lowest,
                run.highest,
                run.lower_raw.as_str(),
                run.upper_raw.as_str()
            ))
            .collect::<Vec<_>>(),
        [
            (100, 103, true, false, "0", "100"),
            (105, 106, true, false, "0", "100")
        ]
    );
}

#[test]
fn autonomy_reply_filters_by_overlord_or_subject_and_orders_levels_by_recorded_bounds_without_content()
 {
    use crate::products::{
        LoadContext, Product,
        autonomy::{Autonomy, AutonomyParams, AutonomyReply},
    };
    let puppet: Status = (
        31,
        Some(11),
        b"autonomy_puppet",
        250_000_000,
        200_000_000,
        300_000_000,
        true,
        true,
    );
    let italy: Status = (21, Some(31), b"modded_level", 10, 0, 100, false, true);
    let frames = vec![
        autonomy_frame(100, &[puppet, italy]),
        autonomy_frame(101, &[puppet, italy]),
    ];
    let replay = Arc::new(Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap());
    let source = crate::Source::from_replay(
        crate::SourceKey {
            source_id: "au".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        replay,
        &AtomicBool::new(false),
    )
    .unwrap();
    let load = |overlords: &[&str], subjects: &[&str]| {
        Autonomy::load(
            &source,
            &AutonomyParams {
                branch_id: "1".into(),
                overlords: overlords.iter().map(|tag| tag.to_string()).collect(),
                subjects: subjects.iter().map(|tag| tag.to_string()).collect(),
            },
            &LoadContext { content: None },
            &AtomicBool::new(false),
        )
        .unwrap()
    };
    let tags = |reply: &AutonomyReply| {
        reply
            .subjects
            .iter()
            .map(|subject| subject.tag.clone())
            .collect::<Vec<_>>()
    };
    let germany = load(&["GER"], &[]);
    assert_eq!(tags(&germany), ["D01 mod"]);
    // Without content the modded level (lowest move-down score 0) comes first.
    assert_eq!(
        germany
            .levels
            .iter()
            .map(|level| (level.id.as_str(), level.name.is_none(), level.defined))
            .collect::<Vec<_>>(),
        [
            ("modded_level", true, false),
            ("autonomy_puppet", true, false)
        ]
    );
    assert_eq!(germany.subjects[0].runs[0].level, 1);
    assert_eq!(germany.score_scale, 100_000);
    assert!(germany.warnings[0].starts_with("Choose the game folders"));
    assert_eq!(tags(&load(&["D01 mod"], &[])), ["ITA"]);
    // In the order they were first recorded as subjects.
    assert_eq!(tags(&load(&[], &["D01 mod", "ITA"])), ["ITA", "D01 mod"]);
    // The branch reply lets the frontend resolve countries across recorded renames.
    assert_eq!(tags(&load(&[], &[])), ["ITA", "D01 mod"]);
    assert!(tags(&load(&["ITA"], &[])).is_empty());
    assert_eq!(germany.recorded.len(), 1);
    assert!(
        Autonomy::validate(&AutonomyParams {
            branch_id: "1".into(),
            overlords: vec![String::new()],
            subjects: vec![]
        })
        .is_err()
    );
}

#[test]
fn autonomy_hours_with_a_subject_or_overlord_the_recording_does_not_name_stay_partly_known() {
    use crate::products::{
        LoadContext, Product,
        autonomy::{Autonomy, AutonomyParams, scan},
    };
    let puppet: Status = (
        31,
        Some(11),
        b"autonomy_puppet",
        250_000_000,
        200_000_000,
        300_000_000,
        true,
        true,
    );
    let italy: Status = (21, Some(11), b"autonomy_puppet", 10, 0, 100, true, true);
    // Link 77 is in no CountryLinks row: a subject nobody can name.
    let stranger: Status = (77, Some(11), b"autonomy_puppet", 10, 0, 100, true, true);
    // Link 88 is in none either: Italy's overlord cannot be named.
    let orphan: Status = (21, Some(88), b"autonomy_puppet", 10, 0, 100, true, true);
    let frames = vec![
        autonomy_frame(100, &[puppet, stranger]),
        autonomy_frame(101, &[puppet, italy]),
        autonomy_frame(102, &[puppet, orphan]),
    ];
    let replay = Arc::new(Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap());
    let spans = |spans: &[crate::products::country_groups::HourSpan]| {
        spans
            .iter()
            .map(|span| (span.first_hour, span.last_hour))
            .collect::<Vec<_>>()
    };
    let scanned = scan(&replay, 1, &AtomicBool::new(false)).unwrap();
    // Every hour is recorded, and the two with an unnamed country are marked.
    assert_eq!(spans(&scanned.recorded), [(100, 102)]);
    assert_eq!(spans(&scanned.unidentified), [(100, 100), (102, 102)]);
    let source = crate::Source::from_replay(
        crate::SourceKey {
            source_id: "au-unidentified".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        replay,
        &AtomicBool::new(false),
    )
    .unwrap();
    let reply = Autonomy::load(
        &source,
        &AutonomyParams {
            branch_id: "1".into(),
            overlords: vec![],
            subjects: vec!["ITA".into()],
        },
        &LoadContext { content: None },
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(spans(&reply.unidentified), [(100, 100), (102, 102)]);
    assert!(
        reply.warnings.iter().any(|warning| warning
            .starts_with("Hours with a subject or overlord the recording does not name: 2.")),
        "{:?}",
        reply.warnings
    );
    // Italy is named at 102, without the overlord the recording cannot name.
    let runs: Vec<_> = reply.subjects[0]
        .runs
        .iter()
        .map(|run| (run.first_hour, run.last_hour, run.overlord.as_deref()))
        .collect();
    assert_eq!(runs, [(101, 101, Some("GER")), (102, 102, None)]);
}

/// A mod that replaces the autonomy states (its own levels, in another order and
/// number, one of them scripted) orders and names the recorded levels by its own
/// definitions: a level only vanilla defined, or one the mod cannot place, follows
/// the others by its recorded move-down score, and every such level is named.
#[test]
fn autonomy_levels_follow_a_mods_autonomy_states_in_their_order_and_count() {
    use crate::products::autonomy::{ScannedAutonomy, order_levels};
    use std::fs;
    let root = std::env::temp_dir().join(format!(
        "hoi4_dashboard_mod_autonomy_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = fs::remove_dir_all(&root);
    let (base, modded) = (root.join("base"), root.join("mod"));
    let states = |levels: &[(&str, &str)]| {
        levels
            .iter()
            .map(|(id, level)| {
                format!("autonomy_state = {{ id = {id} min_freedom_level = {level} }}\n")
            })
            .collect::<String>()
    };
    for (dir, file, text) in [
        (&base, "puppet.txt", states(&[("autonomy_puppet", "0.4")])),
        (
            &base,
            "dominion.txt",
            states(&[("autonomy_dominion", "0.8")]),
        ),
        (
            &modded,
            "serfdom.txt",
            states(&[
                ("autonomy_vassal", "0.5"),
                ("autonomy_serf", "0.1"),
                ("autonomy_scripted", "@LEVEL"),
            ]),
        ),
        (&modded, "allies.txt", states(&[("autonomy_ally", "0.95")])),
    ] {
        let path = dir.join("common/autonomous_states").join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    let layers = installed_content::ResolvedLayers::from_folders(&[
        (&base, &[][..]),
        (&modded, &["common/autonomous_states"][..]),
    ])
    .unwrap();
    let (defined, warnings) = installed_content::autonomy::read(&layers).unwrap();
    let mut ids: Vec<_> = defined
        .states
        .iter()
        .map(|state| state.id.as_str())
        .collect();
    ids.sort_unstable();
    // The mod replaced the folder: vanilla's levels are gone, the mod's four remain.
    assert_eq!(
        ids,
        [
            "autonomy_ally",
            "autonomy_scripted",
            "autonomy_serf",
            "autonomy_vassal"
        ]
    );
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(
        warnings[0].1.contains("autonomy_scripted") && warnings[0].1.contains("not a plain number"),
        "{warnings:?}"
    );
    // Recorded in another order, with a level only vanilla defined.
    let scanned = ScannedAutonomy {
        levels: [
            "autonomy_vassal",
            "autonomy_puppet",
            "autonomy_scripted",
            "autonomy_serf",
            "autonomy_ally",
        ]
        .map(String::from)
        .to_vec(),
        lowest_lower: vec![250, 200, 0, 50, 475],
        ..Default::default()
    };
    let (levels, places) = order_levels(&scanned, Some(&defined), |id| {
        (id == "autonomy_serf").then(|| "Serf".to_owned())
    });
    assert_eq!(
        levels
            .iter()
            .map(|level| (level.id.as_str(), level.name.as_deref(), level.defined))
            .collect::<Vec<_>>(),
        [
            ("autonomy_serf", Some("Serf"), true),
            ("autonomy_vassal", None, true),
            ("autonomy_ally", None, true),
            ("autonomy_scripted", None, false),
            ("autonomy_puppet", None, false),
        ]
    );
    assert_eq!(places, [1, 4, 3, 0, 2]);
    fs::remove_dir_all(&root).unwrap();
}

/// A branch recorded without the Autonomy table is not recorded, which the reply
/// tells apart from a branch where nobody was a subject.
#[test]
fn autonomy_never_recorded_is_not_no_subjects() {
    use crate::products::{
        LoadContext, Product,
        autonomy::{Autonomy, AutonomyParams},
    };
    let unrecorded = |hour| {
        let mut frame = autonomy_frame(hour, &[]);
        *frame.tables[TableId::Autonomy.index()] = TableView::Unavailable {
            reason: "fixture".into(),
        };
        frame
    };
    let load = |frames: Vec<DecodedFrame>| {
        let replay =
            Arc::new(Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap());
        let source = crate::Source::from_replay(
            crate::SourceKey {
                source_id: "au".into(),
                revision: "1".into(),
            },
            "memory.battleplan".into(),
            replay,
            &AtomicBool::new(false),
        )
        .unwrap();
        Autonomy::load(
            &source,
            &AutonomyParams {
                branch_id: "1".into(),
                overlords: vec!["GER".into()],
                subjects: vec![],
            },
            &LoadContext { content: None },
            &AtomicBool::new(false),
        )
        .unwrap()
    };
    let missing = load(vec![unrecorded(100), unrecorded(101)]);
    assert!(missing.subjects.is_empty() && missing.recorded.is_empty());
    assert!(
        missing
            .warnings
            .iter()
            .any(|warning| warning.contains("was not recorded in this branch"))
    );
    let none = load(vec![autonomy_frame(100, &[]), autonomy_frame(101, &[])]);
    assert!(none.subjects.is_empty());
    assert_eq!(
        none.recorded
            .iter()
            .map(|span| (span.first_hour, span.last_hour))
            .collect::<Vec<_>>(),
        [(100, 101)]
    );
}
