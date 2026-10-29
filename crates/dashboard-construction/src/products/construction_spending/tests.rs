use super::*;
use crate::products::construction_log::tests::{
    CIVILIAN, MOD_REACTOR, building_at, building_line, construction_frame, construction_frames,
    railway_line, spending_line, timed_scan,
};
use dashboard_core::test_fixtures::{CAMPAIGN, records};
use replay::container::Replay;
use replay_data::{rows::ConstructionLineOutputRow, rows::TableId, rows::TableRows};
use replay_frame::{frame::DecodedFrame, frame::TableView};
use std::{path::Path, sync::atomic::AtomicBool};

fn output(line: u64, construction_raw: i64, repair_raw: i64) -> ConstructionLineOutputRow {
    ConstructionLineOutputRow {
        line,
        construction_raw,
        repair_raw,
        since_hour: 100,
        observed_since_creation: true,
        exact: true,
        ..Default::default()
    }
}

fn with_output(
    mut frame: DecodedFrame,
    mut output: Vec<ConstructionLineOutputRow>,
) -> DecodedFrame {
    output.sort_by_key(|row| row.line);
    *frame.tables[TableId::ConstructionLineOutput.index()] =
        TableView::Complete(TableRows::ConstructionLineOutput(output));
    frame
}

fn scan(frames: Vec<DecodedFrame>) -> ScannedLog {
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    timed_scan(&replay, 1, &AtomicBool::new(false)).unwrap()
}

#[test]
fn construction_spending_keeps_signed_repair_and_deleted_final_output_without_repricing() {
    let frames = [100, 124, 148]
        .map(|hour| {
            let mut repair = building_line(1, 5, 0, 1, 2);
            repair.cost = if hour == 100 { 900 } else { 50_000_000 };
            repair.produced = 0;
            let building_type = repair.building_type;
            let mut row = output(1, 0, if hour == 100 { 200 } else { 175 });
            row.repair_ticks = 1;
            row.bank_raw = 8_000_000;
            row.carry_raw = 9_000_000;
            row.ended = hour == 124;
            with_output(
                construction_frame(
                    hour,
                    true,
                    if hour == 100 { vec![repair] } else { vec![] },
                    vec![],
                    Some(vec![building_at(5, building_type, (2, 1))]),
                    vec![],
                ),
                if hour == 148 { vec![] } else { vec![row] },
            )
        })
        .to_vec();
    let log = scan(frames);
    let (leaves, _) = leaves(&log, "GER");
    let repair = leaves.iter().find(|leaf| leaf.repair).unwrap();
    assert_eq!(
        repair.spent,
        [
            SpentStep {
                known_at: 100,
                total_raw: "200".into()
            },
            SpentStep {
                known_at: 124,
                total_raw: "175".into()
            }
        ]
    );
    assert!(repair.unknown.is_empty());
    assert_eq!(log.outputs[0].len(), 2);
    // The exact final spend cannot establish completion: no healthy rise exists.
    assert_ne!(
        log.projects[0].end.as_ref().unwrap().kind,
        crate::products::construction_log::EndKind::Finished
    );
}

#[test]
fn construction_spending_uses_railway_factory_output_not_progress_bank_or_carry() {
    let mut row = output(3, 9_007_199_254_741_499, 0);
    row.railway = true;
    row.bank_raw = 80;
    row.carry_raw = 100;
    let mut rail = railway_line(3, &[1001, 1002], 1);
    rail.produced = 20;
    let log = scan(vec![with_output(
        construction_frame(100, true, vec![], vec![rail], Some(vec![]), vec![]),
        vec![row],
    )]);
    let (leaves, _) = leaves(&log, "GER");
    assert_eq!(leaves[0].spent[0].total_raw, "9007199254741499");
}

#[test]
fn natural_repair_is_not_counted_again_after_an_unavailable_output_read() {
    let frame = |hour| {
        construction_frame(
            hour,
            true,
            vec![building_line(1, 5, 1, 0, 1)],
            vec![],
            Some(vec![]),
            vec![],
        )
    };
    let natural = ConstructionLineOutputRow {
        repair_ticks: 1,
        ..output(1, 0, 0)
    };
    let log = scan(vec![
        with_output(frame(100), vec![natural.clone()]),
        frame(124),
        with_output(frame(148), vec![natural]),
    ]);
    let (leaves, _) = leaves(&log, "GER");
    let repair = leaves.iter().find(|leaf| leaf.repair).unwrap();
    assert_eq!(repair.projects.len(), 1);
    assert_eq!(repair.projects[0].first_read, 100);
    assert_eq!(repair.projects[0].until, Some(101));
    assert_eq!(repair.unknown[0].until, Some(148));
}

#[test]
fn construction_output_refuses_missing_and_linked_attribution_then_accepts_a_same_epoch_total() {
    let frame = |hour| {
        construction_frame(
            hour,
            true,
            vec![spending_line(1, 1, 900, 90)],
            vec![],
            Some(vec![]),
            vec![],
        )
    };
    let linked = ConstructionLineOutputRow {
        linked: true,
        ..output(1, 7, 0)
    };
    let log = scan(vec![
        frame(100),
        with_output(frame(124), vec![output(1, 50, 0)]),
        with_output(frame(148), vec![linked]),
    ]);
    let (leaves, _) = leaves(&log, "GER");
    let new = leaves.iter().find(|leaf| !leaf.repair).unwrap();
    assert_eq!(new.spent[0].total_raw, "50");
    assert_eq!(new.unknown[0].until, Some(124));
    assert_eq!(
        new.unknown.last().unwrap().reason,
        SpendingGap::LinkedOutput
    );
    assert!(log.outputs[0][0].construction_raw.is_none());
    assert_eq!(log.output_unknown[0].until, Some(124));
}

#[test]
fn construction_output_never_joins_a_reset_or_hides_ownerless_final_lines() {
    let frame = |hour| {
        construction_frame(
            hour,
            true,
            vec![spending_line(1, 1, 900, 90)],
            vec![],
            Some(vec![]),
            vec![],
        )
    };
    let reset = ConstructionLineOutputRow {
        since_hour: 120,
        ..output(1, 9, 0)
    };
    let unknown = ConstructionLineOutputRow {
        ended: true,
        ..output(99, 200, 0)
    };
    let log = scan(vec![
        with_output(frame(100), vec![output(1, 7, 0)]),
        with_output(frame(124), vec![reset.clone(), unknown]),
        with_output(frame(148), vec![reset]),
    ]);
    let (leaves, _) = leaves(&log, "GER");
    let new = leaves.iter().find(|leaf| !leaf.repair).unwrap();
    assert_eq!(new.spent.last().unwrap().total_raw, "7");
    assert_eq!(
        new.unknown.last().unwrap().reason,
        SpendingGap::ObservationRestart
    );
    assert_eq!(new.unknown.last().unwrap().until, None);
    let unowned = log
        .output_unknown
        .iter()
        .find(|gap| gap.line == Some(99))
        .unwrap();
    assert_eq!((unowned.known_at, unowned.until), (124, Some(125)));
}

#[test]
fn ownerless_output_stays_unattributed_only_until_a_census_identifies_its_line() {
    let frame = |hour, lines| construction_frame(hour, true, lines, vec![], Some(vec![]), vec![]);
    let mut census_failed = frame(100, vec![]);
    *census_failed.tables[TableId::BuildingConstruction.index()] = TableView::Unavailable {
        reason: "fixture: the read failed".into(),
    };
    let ended = ConstructionLineOutputRow {
        ended: true,
        ..output(1, 90, 0)
    };
    let log = scan(vec![
        with_output(census_failed, vec![output(1, 50, 0)]),
        with_output(
            frame(124, vec![spending_line(1, 1, 900, 90)]),
            vec![output(1, 80, 0), output(99, 7, 0)],
        ),
        with_output(frame(148, vec![]), vec![ended, output(99, 9, 0)]),
    ]);
    let unowned = |line| {
        let gap = log
            .output_unknown
            .iter()
            .find(|gap| gap.line == Some(line))
            .unwrap();
        (gap.reason, gap.known_at, gap.until)
    };
    // The census at 124 owns line 1 from then on; what was read at 100 stays unknown.
    assert_eq!(unowned(1), (SpendingGap::OwnerUnknown, 100, Some(124)));
    assert_eq!(unowned(99), (SpendingGap::OwnerUnknown, 124, None));
    let (leaves, _) = leaves(&log, "GER");
    let new = leaves.iter().find(|leaf| !leaf.repair).unwrap();
    assert_eq!(new.spent.last().unwrap().total_raw, "90");
}

#[test]
fn construction_output_reads_only_at_cadence_and_accounts_for_every_owner() {
    use replay_data::{rows::CountryLinkRow, rows::CountryRow};
    let replay_data::cadence::Cadence::Daily(slot) =
        replay_data::cadence::cadence(TableId::BuildingConstruction)
    else {
        panic!("daily queue");
    };
    let hour = 1000 * replay_data::cadence::STAGGER_PERIOD + slot as i32;
    let frame = |at| {
        let countries = (1..=8_u32)
            .map(|id| CountryRow {
                id,
                canonical_slot: id,
                tag: format!("synthetic-{id}").into_bytes(),
                ..Default::default()
            })
            .collect();
        let links = (1..=8)
            .map(|id| CountryLinkRow {
                id,
                canonical_slot: id,
            })
            .collect();
        let lines = (1..=8_u32)
            .map(|id| replay_data::rows::BuildingConstructionRow {
                id: u64::from(id),
                country: id,
                ..building_line(u64::from(id), id, 1, 0, 1)
            })
            .collect();
        let output = (1..=8_u64).map(|id| output(id, i64::from(at), 0)).collect();
        let mut frame = with_output(
            construction_frame(at, false, lines, vec![], Some(vec![]), vec![]),
            output,
        );
        *frame.tables[TableId::Countries.index()] =
            TableView::Complete(TableRows::Countries(countries));
        *frame.tables[TableId::CountryLinks.index()] =
            TableView::Complete(TableRows::CountryLinks(links));
        frame
    };
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(1, vec![frame(hour), frame(hour + 1)])],
    )
    .unwrap();
    let log = crate::products::construction_log::scan(&replay, 1, &AtomicBool::new(false)).unwrap();
    assert_eq!(log.projects.len(), 8);
    assert!(log.output_unknown.is_empty());
    // Group every owner's leaves once; `leaves` regroups the whole log per tag.
    let mut grouped = all_leaves(&log);
    assert_eq!(grouped.len(), 8);
    for id in 1..=8 {
        let (leaves, _) = grouped.remove(&format!("synthetic-{id}")).unwrap();
        assert_eq!(leaves[0].spent.len(), 1);
        assert_eq!(leaves[0].spent[0].total_raw, hour.to_string());
    }
}

#[test]
fn construction_spending_groups_a_mod_building_type_named_from_the_mod_layer() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    use crate::products::{
        construction_log::{ContentView, names_in, tests::timed_scan as scan},
        construction_spending::{SpentStep, leaves},
    };
    use std::collections::{BTreeMap, BTreeSet};
    let root = std::env::temp_dir().join(format!(
        "dashboard_spending_mod_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let (base, modded) = (root.join("base"), root.join("mod"));
    let write = |dir: &Path, file: &str, bytes: &[u8]| {
        let path = dir.join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    };
    write(
        &base,
        "common/buildings/00_buildings.txt",
        b"buildings = { industrial_complex = { } }",
    );
    // What the state reader needs: the building level cap and the map's provinces.
    write(
        &base,
        "common/defines/00_defines.lua",
        b"NDefines = { NBuildings = { MAX_BUILDING_LEVELS = 15 } }",
    );
    write(
        &base,
        "map/default.map",
        br#"definitions = "definition.csv" provinces = "provinces.bmp" terrain = "terrain.bmp" rivers = "rivers.bmp" heightmap = "heightmap.bmp" tree_definition = "trees.bmp""#,
    );
    write(
        &base,
        "map/definition.csv",
        b"0;0;0;0;land;false;unknown;0
",
    );
    write(
        &base,
        "history/states/5-Ruhr.txt",
        br#"state = { id = 5 name = "STATE_5" }"#,
    );
    // The mod adds a building type and a state of its own.
    write(
        &modded,
        "common/buildings/zz_mod_buildings.txt",
        b"buildings = { mod_reactor = { } }",
    );
    write(
        &modded,
        "history/states/900-Atlantis.txt",
        br#"state = { id = 900 name = "STATE_900" }"#,
    );
    let layers =
        installed_content_core::ResolvedLayers::from_folders(&[(&base, &[][..]), (&modded, &[][..])])
            .unwrap();
    let sprites = installed_content_gfx::gfx::read(&layers).unwrap();
    let guis = installed_content_core::gui::native::read_layers(&layers).unwrap();
    let gui_decode = Ok(Default::default());
    let texts = BTreeMap::from([
        ("industrial_complex", "Civilian Factory"),
        ("mod_reactor", "Fusion Reactor"),
        ("STATE_5", "Ruhr"),
        ("STATE_900", "Atlantis"),
    ]);
    let localise = |key: &str| texts.get(key).map(|text| text.to_string());
    let view = ContentView {
        layers: &layers,
        sprites: &sprites,
        guis: Ok(&guis),
        gui_decode: &gui_decode,
        localise: &localise,
    };
    // Two lines created after the first read: a civilian factory in Ruhr and the
    // mod's reactor in the mod's state.
    let replay = construction_frames(150, |hour, read| {
        let line = |id: u64, state: u32, building_type: u32, produced: i64| {
            replay_data::rows::BuildingConstructionRow {
                created_hour: 110,
                produced,
                building_type,
                ..building_line(id, state, 1, 0, 5)
            }
        };
        let lines = if hour < 124 {
            Vec::new()
        } else {
            let reactor = if hour < 148 { 20_000_000 } else { 50_000_000 };
            vec![
                line(1, 5, CIVILIAN, 30_000_000),
                line(2, 900, MOD_REACTOR, reactor),
            ]
        };
        let states = vec![
            building_at(5, CIVILIAN, (1, 1)),
            building_at(900, MOD_REACTOR, (0, 0)),
        ];
        let outputs = lines
            .iter()
            .map(|line| output(line.id, line.produced, 0))
            .collect();
        with_output(
            construction_frame(hour, read, lines, Vec::new(), Some(states), Vec::new()),
            outputs,
        )
    });
    let log = scan(&replay, 1, &AtomicBool::new(false)).unwrap();
    let mut warnings = Vec::new();
    let names = names_in(&view, &log, &AtomicBool::new(false), &mut warnings).unwrap();
    let _ = std::fs::remove_dir_all(&root);
    let (grouped, before) = leaves(&log, "GER");
    assert_eq!((grouped.len(), before), (2, 0));
    let token = |building: u32| names.types[building as usize].token.as_deref();
    let reactor = grouped
        .iter()
        .find(|leaf| token(leaf.building) == Some("mod_reactor"))
        .unwrap();
    assert_eq!(
        names.types[reactor.building as usize].name.as_deref(),
        Some("Fusion Reactor")
    );
    let step = |known_at, total: &str| SpentStep {
        known_at,
        total_raw: total.into(),
    };
    // Its produced at its first read, then what it added by the next.
    assert_eq!(
        reactor.spent,
        [step(124, "20000000"), step(148, "50000000")]
    );
    assert_eq!(reactor.state, Some(900));
    let civilian = grouped
        .iter()
        .find(|leaf| token(leaf.building) == Some("industrial_complex"))
        .unwrap();
    assert_eq!(
        names.types[civilian.building as usize].name.as_deref(),
        Some("Civilian Factory")
    );
    assert_eq!(
        civilian.spent,
        [step(124, "30000000"), step(148, "30000000")]
    );
    let states = names.states(&BTreeSet::from([5, 900]));
    assert_eq!(
        states
            .iter()
            .map(|state| state.name.as_deref())
            .collect::<Vec<_>>(),
        [Some("Ruhr"), Some("Atlantis")]
    );
}

#[test]
fn a_break_in_a_projects_reads_splits_its_span() {
    use crate::products::construction_spending::{LeafProject, leaves};
    // Read at 100 and 124, then not recorded 125..=195, then read again at 220.
    let replay = {
        let frames = (100..=230)
            .filter(|hour| !(125..=195).contains(hour))
            .map(|hour| {
                construction_frame(
                    hour,
                    [100, 124, 220].contains(&hour),
                    vec![spending_line(1, 2, 0, 90)],
                    Vec::new(),
                    Some(vec![building_at(20, CIVILIAN, (1, 1))]),
                    Vec::new(),
                )
            })
            .collect();
        Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap()
    };
    let log =
        crate::products::construction_log::tests::timed_scan(&replay, 1, &AtomicBool::new(false))
            .unwrap();
    let (grouped, _) = leaves(&log, "GER");
    let span = |first_read, until, unread, resumes| LeafProject {
        project: "1".into(),
        known_at: None,
        first_read,
        until,
        unread,
        resumes,
    };
    assert_eq!(
        grouped[0].projects,
        [
            span(100, Some(125), true, Some(220)),
            span(220, None, false, None)
        ]
    );
}

#[test]
fn reads_that_stop_before_the_branch_ends_leave_the_rest_unread() {
    use crate::products::construction_spending::{LeafProject, leaves};
    // Read at 100 and carried to 123; the scheduled read at 124 fails and none
    // succeeds before the branch ends at 150.
    let replay = construction_frames(150, |hour, read| {
        let mut frame = construction_frame(
            hour,
            read,
            vec![spending_line(1, 2, 0, 90)],
            Vec::new(),
            Some(vec![building_at(20, CIVILIAN, (1, 1))]),
            Vec::new(),
        );
        if read && hour >= 124 {
            *frame.tables[TableId::BuildingConstruction.index()] = TableView::Unavailable {
                reason: "fixture: the read failed".into(),
            };
        }
        frame
    });
    let log =
        crate::products::construction_log::tests::timed_scan(&replay, 1, &AtomicBool::new(false))
            .unwrap();
    assert_eq!(log.last_hour, Some(150));
    let (grouped, _) = leaves(&log, "GER");
    assert_eq!(
        grouped[0].projects,
        [LeafProject {
            project: "1".into(),
            known_at: None,
            first_read: 100,
            until: Some(124),
            unread: true,
            resumes: None
        }]
    );
}

#[test]
fn a_project_gone_after_a_failed_read_keeps_the_hours_between_unread() {
    use crate::products::construction_spending::{LeafProject, leaves};
    // Read at 100 and carried to 123; the read at 124 fails and the one at 148
    // finds it gone, so 124..148 is not known, not nothing.
    let replay = construction_frames(150, |hour, read| {
        let lines = if hour < 148 {
            vec![spending_line(1, 2, 0, 90)]
        } else {
            Vec::new()
        };
        let mut frame = construction_frame(
            hour,
            read,
            lines,
            Vec::new(),
            Some(vec![building_at(20, CIVILIAN, (1, 1))]),
            Vec::new(),
        );
        if hour == 124 {
            *frame.tables[TableId::BuildingConstruction.index()] = TableView::Unavailable {
                reason: "fixture: the read failed".into(),
            };
        }
        frame
    });
    let log =
        crate::products::construction_log::tests::timed_scan(&replay, 1, &AtomicBool::new(false))
            .unwrap();
    let (grouped, _) = leaves(&log, "GER");
    assert_eq!(
        grouped[0].projects,
        [LeafProject {
            project: "1".into(),
            known_at: None,
            first_read: 100,
            until: Some(124),
            unread: true,
            resumes: Some(148)
        }]
    );
}

#[test]
fn a_branch_ending_in_a_recorded_gap_leaves_open_projects_unread_through_it() {
    use crate::products::construction_spending::{LeafProject, leaves};
    // Frames through 123, then a recorded gap to 150 with the project still open.
    let frames = (100..=123)
        .map(|hour| {
            construction_frame(
                hour,
                hour == 100,
                vec![spending_line(1, 2, 0, 90)],
                Vec::new(),
                Some(vec![building_at(20, CIVILIAN, (1, 1))]),
                Vec::new(),
            )
        })
        .collect();
    let mut replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    replay.gaps.push(replay_frame::branches::Gap {
        branch: 1,
        first_hour: 124,
        last_hour: 150,
        reason: "fixture".into(),
    });
    let log =
        crate::products::construction_log::tests::timed_scan(&replay, 1, &AtomicBool::new(false))
            .unwrap();
    assert_eq!(log.last_hour, Some(150));
    let (grouped, _) = leaves(&log, "GER");
    assert_eq!(
        grouped[0].projects,
        [LeafProject {
            project: "1".into(),
            known_at: None,
            first_read: 100,
            until: Some(124),
            unread: true,
            resumes: None
        }]
    );
}

#[test]
fn a_repair_done_between_reads_puts_its_project_in_the_repair_leaf_for_that_stretch() {
    use crate::products::construction_spending::{LeafProject, leaves};
    // As below: no read shows repair work, but the building was damaged at 110 and
    // repaired at 120, so the line repaired in the stretch the read at 124 closes.
    let replay = construction_frames(130, |hour, read| {
        let lines = vec![building_line(1, 5, 1, 0, 15)];
        let levels = match hour {
            ..110 => (2, 2),
            110..120 => (2, 1),
            _ => (2, 2),
        };
        let states = Some(vec![building_at(5, CIVILIAN, levels)]);
        construction_frame(hour, read, lines, Vec::new(), states, Vec::new())
    });
    let log =
        crate::products::construction_log::tests::timed_scan(&replay, 1, &AtomicBool::new(false))
            .unwrap();
    let (grouped, _) = leaves(&log, "GER");
    let repair = grouped.iter().find(|leaf| leaf.repair).unwrap();
    assert_eq!(
        repair.projects,
        [LeafProject {
            project: "1".into(),
            known_at: Some(124),
            first_read: 100,
            until: Some(124),
            unread: false,
            resumes: None
        }]
    );
}
