use dashboard_core::test_fixtures::{CAMPAIGN, records, source};
use replay::container::Replay;
use replay_data::{rows::TableId, rows::TableRows};
use replay_frame::{frame::DecodedFrame, frame::TableView};
use std::{
    path::Path,
    sync::{Arc, atomic::AtomicBool},
};

/// The claim fixtures supply their own read times through StateGarrisons' timing
/// flag, including off-slot full reads; production cadence is checked separately.
pub(crate) fn timed_scan(
    replay: &Replay,
    branch: u64,
    cancelled: &AtomicBool,
) -> std::io::Result<super::ScannedLog> {
    super::scan_with_reads(replay, branch, cancelled, |frame, _| {
        frame.metrics.table_ns[TableId::StateGarrisons.index()] != 0
    })
}

/// Lexer ids of the building types the construction fixtures use; the reactor
/// is a type only a mod declares.
pub(crate) const CIVILIAN: u32 = 20013;
pub(crate) const INFRASTRUCTURE: u32 = 12203;
pub(crate) const MOD_REACTOR: u32 = 500;

/// A building line of GER (raw link 11) at `state`.
pub(crate) fn building_line(
    id: u64,
    state: u32,
    amount: i32,
    repair_levels: i32,
    factories: i32,
) -> replay_data::rows::BuildingConstructionRow {
    replay_data::rows::BuildingConstructionRow {
        id,
        country: 11,
        active_factories: factories,
        cost: 1_080_000_000,
        speed: 1_080_000_000,
        amount,
        created_hour: 90,
        building_type: if repair_levels > 0 {
            INFRASTRUCTURE
        } else {
            CIVILIAN
        },
        target_flags: engine_native::cbuilding_status::STATE_FLAG,
        target_location: state,
        target_state: state,
        repair_levels,
        ..Default::default()
    }
}

/// A building's (level, healthy level) at a location.
pub(crate) fn building_at(
    location: u32,
    building_type: u32,
    levels: (i16, i16),
) -> replay_data::rows::BuildingRow {
    replay_data::rows::BuildingRow {
        location,
        building_type,
        level: levels.0,
        healthy_level: levels.1,
        ..Default::default()
    }
}

/// A frame of the construction fixtures: GER's queues, state buildings (`None`:
/// the table is incomplete) and province buildings. `read` times a walk of
/// another table of the construction slot, as a shared walk times only its anchor.
pub(crate) fn construction_frame(
    hour: i32,
    read: bool,
    mut lines: Vec<replay_data::rows::BuildingConstructionRow>,
    mut rails: Vec<replay_data::rows::RailwayConstructionRow>,
    states: Option<Vec<replay_data::rows::BuildingRow>>,
    mut provinces: Vec<replay_data::rows::BuildingRow>,
) -> DecodedFrame {
    use replay_data::{rows::LexicalTokenRow, rows::ProvinceRow};
    let mut frame = source(1, hour, Some(1));
    lines.sort_by_key(|line| line.id);
    rails.sort_by_key(|line| line.id);
    provinces.sort_by_key(|row| (row.location, row.building_type));
    *frame.tables[TableId::BuildingConstruction.index()] =
        TableView::Complete(TableRows::BuildingConstruction(lines));
    *frame.tables[TableId::RailwayConstruction.index()] =
        TableView::Complete(TableRows::RailwayConstruction(rails));
    *frame.tables[TableId::StateBuildings.index()] = match states {
        Some(mut states) => {
            states.sort_by_key(|row| (row.location, row.building_type));
            TableView::Complete(TableRows::StateBuildings(states))
        }
        None => TableView::Unavailable {
            reason: "fixture".into(),
        },
    };
    *frame.tables[TableId::ProvinceBuildings.index()] =
        TableView::Complete(TableRows::ProvinceBuildings(provinces));
    *frame.tables[TableId::Provinces.index()] = TableView::Complete(TableRows::Provinces(
        [(1001, 40), (1002, 40), (1003, 41), (2001, 50), (2002, 51)]
            .map(|(id, state)| ProvinceRow {
                id,
                state,
                ..Default::default()
            })
            .to_vec(),
    ));
    *frame.tables[TableId::LexicalTokens.index()] = TableView::Complete(TableRows::LexicalTokens(
        [
            (MOD_REACTOR, "mod_reactor"),
            (INFRASTRUCTURE, "infrastructure"),
            (
                engine_native::cbuilding_database::RAILWAY_TEMPLATE_TOKEN,
                "rail_way",
            ),
            (CIVILIAN, "industrial_complex"),
        ]
        .map(|(token, literal)| LexicalTokenRow {
            token,
            literal: literal.as_bytes().to_vec(),
        })
        .to_vec(),
    ));
    if read {
        frame.metrics.table_ns[TableId::StateGarrisons.index()] = 1;
    }
    frame
}

#[test]
fn construction_projects_keep_runs_and_bound_departures_without_native_outcomes() {
    use crate::products::construction_log::{
        EndKind, LevelsBuilt, ProjectRun, UnknownEnd, tests::timed_scan as scan,
    };
    let reads = [100, 124, 148, 172];
    let frames = (100..=180)
        .filter(|hour| *hour != 152)
        .map(|hour| {
            let mut lines = Vec::new();
            // 1: one civilian factory, built at 144. Hour 101 is not a read, so
            // what it holds is not a read either.
            if hour < 148 {
                let factories = if hour == 101 { 99 } else { 10 };
                lines.push(building_line(1, 5, 1, 0, factories));
            }
            // 2: a repair, healthy again at 120.
            if hour < 124 {
                lines.push(building_line(2, 6, 0, 1, 0));
            }
            // 3: gone at 124 with nothing built.
            if hour < 124 {
                lines.push(building_line(3, 7, 1, 0, 5));
            }
            // 4: two levels, one at 120 and one at 144.
            if hour < 148 {
                lines.push(building_line(4, 8, if hour < 124 { 2 } else { 1 }, 0, 15));
            }
            // 5: first read at 148, gone at 172 after the missing hour 152 and
            // an hour whose building table is incomplete.
            if (148..172).contains(&hour) {
                lines.push(building_line(5, 9, 1, 0, 1));
            }
            // 6 and 7: two lines at one building, which rises once.
            if hour < 148 {
                lines.push(building_line(6, 10, 1, 0, 1));
                lines.push(building_line(7, 10, 1, 0, 1));
            }
            // 8: open across the missing hour 152 to the end.
            lines.push(building_line(8, 11, 1, 0, 3));
            // 9: a country the links do not resolve.
            if hour >= 124 {
                lines.push(replay_data::rows::BuildingConstructionRow {
                    country: 77,
                    ..building_line(9, 12, 1, 0, 1)
                });
            }
            let rose = |at: i32, before: (i16, i16), after: (i16, i16)| {
                if hour < at { before } else { after }
            };
            let states = (hour != 160).then(|| {
                vec![
                    building_at(5, CIVILIAN, rose(144, (2, 2), (3, 3))),
                    building_at(6, INFRASTRUCTURE, rose(120, (3, 2), (3, 3))),
                    building_at(7, CIVILIAN, (1, 1)),
                    building_at(8, CIVILIAN, rose(144, rose(120, (0, 0), (1, 1)), (2, 2))),
                    building_at(9, CIVILIAN, rose(168, (1, 1), (2, 2))),
                    building_at(10, CIVILIAN, rose(144, (1, 1), (2, 2))),
                ]
            });
            construction_frame(
                hour,
                reads.contains(&hour),
                lines,
                Vec::new(),
                states,
                Vec::new(),
            )
        })
        .collect();
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    let log = scan(&replay, 1, &AtomicBool::new(false)).unwrap();
    assert_eq!(log.reads, reads);
    assert_eq!(log.unresolved, 1);
    assert_eq!(
        log.types,
        [
            (CIVILIAN, Some("industrial_complex".to_owned())),
            (INFRASTRUCTURE, Some("infrastructure".to_owned())),
        ]
    );
    let project = |id: &str| {
        log.projects
            .iter()
            .find(|project| project.id == id)
            .unwrap_or_else(|| panic!("project {id}"))
    };
    let ending = |id: &str| {
        project(id)
            .end
            .as_ref()
            .map(|end| (end.kind, end.hour, end.known_at, end.unknown))
    };
    let built = |hour, known_at| LevelsBuilt {
        hour: Some(hour),
        known_at,
        count: 1,
        repair: false,
    };
    let run = |first_hour, last_hour, factories, remaining, repair_levels| ProjectRun {
        first_hour,
        last_hour,
        factories,
        remaining,
        repair_levels,
    };
    let one = project("1");
    assert_eq!(
        (
            one.tag.as_str(),
            one.state,
            one.created_hour,
            one.first_read
        ),
        ("GER", Some(5), 90, 100)
    );
    assert_eq!(one.runs, [run(100, 147, 10, 1, 0)]);
    assert!(one.built.is_empty());
    assert_eq!(
        ending("1"),
        Some((
            EndKind::Unknown,
            None,
            148,
            Some(UnknownEnd::CompletionNotRecorded)
        ))
    );
    assert!(project("2").repair);
    assert!(project("2").built.is_empty());
    assert_eq!(
        ending("2"),
        Some((
            EndKind::Unknown,
            None,
            124,
            Some(UnknownEnd::CompletionNotRecorded)
        ))
    );
    assert!(project("3").built.is_empty());
    assert_eq!(
        ending("3"),
        Some((
            EndKind::Unknown,
            None,
            124,
            Some(UnknownEnd::CompletionNotRecorded)
        ))
    );
    assert_eq!(
        project("4").runs,
        [run(100, 123, 15, 2, 0), run(124, 147, 15, 1, 0)]
    );
    assert_eq!(project("4").built, [built(120, 124)]);
    assert_eq!(
        ending("4"),
        Some((
            EndKind::Unknown,
            None,
            148,
            Some(UnknownEnd::CompletionNotRecorded)
        ))
    );
    assert_eq!(project("5").first_read, 148);
    assert_eq!(
        ending("5"),
        Some((EndKind::Unknown, None, 172, Some(UnknownEnd::NotRecorded)))
    );
    for shared in ["6", "7"] {
        assert!(project(shared).built.is_empty());
        assert_eq!(
            ending(shared),
            Some((EndKind::Unknown, None, 148, Some(UnknownEnd::SharedTarget)))
        );
    }
    // Carried to the gap, not across it, then from the next read; open at the end.
    assert_eq!(
        project("8").runs,
        [run(100, 151, 3, 1, 0), run(172, 180, 3, 1, 0)]
    );
    assert_eq!(ending("8"), None);
    assert!(log.projects.iter().all(|project| project.id != "9"));
}

/// A railway line of GER along `route`, from level 1 to 2.
pub(crate) fn railway_line(
    id: u64,
    route: &[u32],
    amount: i32,
) -> replay_data::rows::RailwayConstructionRow {
    replay_data::rows::RailwayConstructionRow {
        id,
        country: 11,
        active_factories: 4,
        cost: 56_000_000,
        speed: 56_000_000,
        amount,
        created_hour: 95,
        base_level: 1,
        target_level: 2,
        route: route.to_vec(),
        ..Default::default()
    }
}

#[test]
fn railway_live_amount_falls_are_counted_but_route_rises_do_not_prove_completion() {
    use crate::products::construction_log::{
        EndKind, LevelsBuilt, ProjectKind, tests::timed_scan as scan,
    };
    let rail = engine_native::cbuilding_database::RAILWAY_TEMPLATE_TOKEN;
    let reads = [100, 124, 148];
    let frames = (100..=150)
        .map(|hour| {
            let mut rails = Vec::new();
            // 10: two segments, the first by 124 and the last at 144.
            if hour < 148 {
                rails.push(railway_line(
                    10,
                    &[1001, 1002, 1003],
                    if hour < 124 { 2 } else { 1 },
                ));
            }
            // 11: gone at 124 with its route still at level 1.
            if hour < 124 {
                rails.push(railway_line(11, &[2001, 2002], 1));
            }
            let level = |at: i32| if hour < at { (1, 1) } else { (2, 2) };
            let provinces = vec![
                building_at(1001, rail, level(120)),
                building_at(1002, rail, level(120)),
                building_at(1003, rail, level(144)),
                building_at(2001, rail, (1, 1)),
                building_at(2002, rail, (1, 1)),
            ];
            construction_frame(
                hour,
                reads.contains(&hour),
                Vec::new(),
                rails,
                Some(Vec::new()),
                provinces,
            )
        })
        .collect();
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    let log = scan(&replay, 1, &AtomicBool::new(false)).unwrap();
    assert_eq!(log.railway_reads, reads);
    let project = |id: &str| {
        log.projects
            .iter()
            .find(|project| project.id == id)
            .unwrap()
    };
    let railway = project("10");
    assert_eq!(railway.kind, ProjectKind::Railway);
    assert_eq!(
        (
            railway.state,
            railway.to_state,
            railway.route_provinces,
            railway.base_level,
            railway.target_level
        ),
        (Some(40), Some(41), Some(3), Some(1), Some(2))
    );
    assert_eq!(
        log.types[railway.building as usize],
        (rail, Some("rail_way".to_owned()))
    );
    let segment = |known_at| LevelsBuilt {
        hour: None,
        known_at,
        count: 1,
        repair: false,
    };
    assert_eq!(railway.built, [segment(124)]);
    assert_eq!(railway.unaccounted, [segment(148)]);
    let end = railway.end.as_ref().unwrap();
    assert_eq!(
        (end.kind, end.hour, end.known_at),
        (EndKind::Unknown, None, 148)
    );
    let removed = project("11").end.as_ref().unwrap();
    assert_eq!(
        (removed.kind, removed.hour, removed.known_at),
        (EndKind::Unknown, None, 124)
    );
}

#[test]
fn the_construction_log_lists_projects_by_token_without_content() {
    use crate::products::construction_log::{
        ConstructionLog, ConstructionLogManifest, ConstructionLogParams,
    };
    use dashboard_base::products::{BranchParams, Product};
    use dashboard_core::config::LoadContext;
    let frames = (100..=101)
        .map(|hour| {
            construction_frame(
                hour,
                hour == 100,
                vec![building_line(1, 5, 1, 0, 3)],
                Vec::new(),
                Some(vec![building_at(5, CIVILIAN, (1, 1))]),
                Vec::new(),
            )
        })
        .collect();
    let replay = Arc::new(Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap());
    let source = dashboard_base::source::Source::from_replay(
        dashboard_base::source::SourceKey {
            source_id: "construction".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        replay,
        &AtomicBool::new(false),
    )
    .unwrap();
    let context = LoadContext { content: None };
    let manifest = |branch: &str| {
        ConstructionLogManifest::load(
            &source,
            &BranchParams {
                branch_id: branch.into(),
            },
            &context,
            &AtomicBool::new(false),
        )
        .unwrap()
    };
    let window = |branch: &str, window| {
        ConstructionLog::load(
            &source,
            &ConstructionLogParams {
                branch_id: branch.into(),
                window,
            },
            &context,
            &AtomicBool::new(false),
        )
    };
    let reply = manifest("1");
    assert_eq!(reply.cost_scale, 100_000);
    assert!(reply.recorded);
    assert_eq!(reply.windows.len(), 1);
    assert_eq!(reply.types[0].token.as_deref(), Some("industrial_complex"));
    assert_eq!(
        (reply.types[0].name.as_ref(), reply.types[0].icon.as_ref()),
        (None, None)
    );
    assert!(
        reply
            .warnings
            .iter()
            .any(|warning| warning.starts_with("Building and state names are unavailable")),
        "{:?}",
        reply.warnings
    );
    let projects = window("1", 0).unwrap().projects;
    assert_eq!(projects.len(), 1);
    assert_eq!(projects[0].cost_raw, "1080000000");
    assert!(window("1", 1).is_err());
    let other = manifest("2");
    assert!(!other.recorded && other.windows.is_empty());
    assert!(window("2", 0).is_err());
    assert!(
        ConstructionLog::validate(&ConstructionLogParams {
            branch_id: "x".into(),
            window: 0,
        })
        .is_err()
    );
}

/// Construction frames for hours 100..=`last` with reads at 100, 124 and 148.
pub(crate) fn construction_frames(last: i32, frame: impl Fn(i32, bool) -> DecodedFrame) -> Replay {
    let frames = (100..=last)
        .map(|hour| frame(hour, [100, 124, 148].contains(&hour)))
        .collect();
    Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap()
}

#[test]
fn construction_log_live_reply_warns_only_for_unproven_state_reading() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    use crate::products::construction_log::{ConstructionLogManifest, content_names, scanned};
    use dashboard_base::products::{BranchParams, Product};
    use dashboard_core::config::LoadContext;
    use replay::spool::Record;
    let (_, folder) = dashboard_text::catalogue::tests::folder_catalogue(&[
        ("hoi4.exe", "test"),
        (
            "user/settings.txt",
            "language=l_english force_pow2_textures=no",
        ),
        (
            "user/dlc_load.json",
            r#"{"enabled_mods":[],"disabled_dlcs":[]}"#,
        ),
        (
            "common/defines/defines.lua",
            "NDefines={NBuildings={MAX_BUILDING_LEVELS=15}}",
        ),
        (
            "common/buildings/buildings.txt",
            "buildings={industrial_complex={icon_frame=-1}}",
        ),
        (
            "map/default.map",
            "definitions=definition.csv provinces=provinces.bmp terrain=terrain.bmp rivers=rivers.bmp heightmap=heightmap.bmp tree_definition=trees.bmp",
        ),
        ("map/definition.csv", "0;0;0;0;land;false;unknown;0\n"),
        (
            "history/states/1-kept.txt",
            "@bad nope orphan=yes state={id=5 name=STATE_KEPT impassable=maybe provinces={1} resources={unknown=2} history={victory_points={1} buildings=4} state_startup_effect={free_operative={operative=authored}}}",
        ),
        (
            "history/states/2-duplicate.txt",
            "state={id=5 name=STATE_DUPLICATE}",
        ),
        (
            "localisation/construction_l_english.yml",
            "\u{feff}l_english:\n industrial_complex:0 \"Mod factory\"\n STATE_KEPT:0 \"Kept state\"\n STATE_DUPLICATE:0 \"Discarded state\"\n",
        ),
    ]);
    let (options, identity) = dashboard_base::test_fixtures::content_identity(&folder.0);
    let frames = (100..=101)
        .map(|hour| {
            construction_frame(
                hour,
                hour == 100,
                vec![building_line(1, 5, 1, 0, 3)],
                Vec::new(),
                Some(vec![building_at(5, CIVILIAN, (1, 1))]),
                Vec::new(),
            )
        })
        .collect();
    let mut input = records(1, frames);
    let Record::SessionStart(session) = &mut input[0] else {
        unreachable!()
    };
    session.identity = identity;
    let source = dashboard_base::source::Source::from_replay(
        dashboard_base::source::SourceKey {
            source_id: "construction-duplicate-state".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        Arc::new(Replay::from_records(CAMPAIGN.into(), &[input]).unwrap()),
        &AtomicBool::new(false),
    )
    .unwrap();
    let catalogue = dashboard_base::catalogue::catalogue(&source, &options).unwrap();
    let state_content =
        dashboard_text::scripted_text::read_state_content(&catalogue.content.layers).unwrap();
    assert_eq!(
        state_content.states.get(5).unwrap().name_key.as_deref(),
        Ok("STATE_KEPT")
    );
    assert!(state_content.warnings.iter().any(|note| {
        note.kind == installed_content_core::DiagnosticKind::Warning
            && note.message.contains("state 5 is defined by both")
    }));
    let proven: Vec<_> = state_content
        .warnings
        .iter()
        .filter(|note| note.cause.is_some())
        .collect();
    assert_eq!(proven.len(), 5, "{proven:?}");
    let context = LoadContext {
        content: Some(options),
    };
    let reply = ConstructionLogManifest::load(
        &source,
        &BranchParams {
            branch_id: "1".into(),
        },
        &context,
        &AtomicBool::new(false),
    )
    .unwrap();
    let log = scanned(&source, 1, &AtomicBool::new(false)).unwrap();
    let names = content_names(
        &source,
        &context,
        1,
        &log,
        &AtomicBool::new(false),
        &mut Vec::new(),
    )
    .unwrap();
    assert_eq!(log.projects.len(), 1);
    assert_eq!(
        names.states(&[5].into())[0].name.as_deref(),
        Some("Kept state")
    );
    assert_eq!(reply.types[0].name.as_deref(), Some("Mod factory"));
    assert!(reply.types[0].icon.is_none());
    assert!(
        reply
            .warnings
            .iter()
            .all(|note| !note.contains("state 5 is defined by both")),
        "{:?}",
        reply.warnings
    );
    for note in proven {
        assert!(
            reply
                .warnings
                .iter()
                .all(|warning| !warning.contains(&note.message)),
            "{:?}",
            reply.warnings
        );
    }
    for note in state_content
        .warnings
        .iter()
        .filter(|note| note.cause.is_none() && note.kind != installed_content_core::DiagnosticKind::Warning)
    {
        assert!(
            reply
                .warnings
                .contains(&format!("State names: {}", note.message)),
            "{:?}",
            reply.warnings
        );
    }
    assert!(
        reply
            .warnings
            .iter()
            .any(|note| note.contains("definition `@bad` has no `=`"))
    );
    assert!(
        reply
            .warnings
            .iter()
            .any(|note| note.contains("Deferred effect `free_operative`"))
    );
    assert!(
        reply
            .warnings
            .iter()
            .any(|note| note.contains("give a negative icon_frame; it is refused")),
        "{:?}",
        reply.warnings
    );
}

#[test]
fn a_railway_recorded_without_a_route_ends_unknown_and_is_counted() {
    use crate::products::construction_log::{ConstructionLogManifest, EndKind, UnknownEnd, scan};
    use dashboard_base::products::{BranchParams, Product};
    let replay = Arc::new(construction_frames(130, |hour, read| {
        let rails = if hour < 124 {
            vec![railway_line(12, &[], 1)]
        } else {
            Vec::new()
        };
        construction_frame(hour, read, Vec::new(), rails, Some(Vec::new()), Vec::new())
    }));
    let log = scan(&replay, 1, &AtomicBool::new(false)).unwrap();
    assert_eq!(log.empty_routes, 1);
    let end = log.projects[0].end.as_ref().unwrap();
    assert_eq!(
        (end.kind, end.hour, end.known_at, end.unknown),
        (
            EndKind::Unknown,
            None,
            124,
            Some(UnknownEnd::CompletionNotRecorded)
        )
    );
    let source = dashboard_base::source::Source::from_replay(
        dashboard_base::source::SourceKey {
            source_id: "routes".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        replay,
        &AtomicBool::new(false),
    )
    .unwrap();
    let reply = ConstructionLogManifest::load(
        &source,
        &BranchParams {
            branch_id: "1".into(),
        },
        &dashboard_core::config::LoadContext { content: None },
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(
        reply
            .warnings
            .iter()
            .any(|warning| warning.starts_with("1 railway lines were recorded without a route")),
        "{:?}",
        reply.warnings
    );
}

#[test]
fn levels_lost_while_the_building_was_not_recorded_are_unaccounted_not_erased() {
    use crate::products::construction_log::{EndKind, LevelsBuilt, tests::timed_scan as scan};
    // Two levels at 100; the first rises at 110, an hour whose building table is
    // incomplete; one is left at 124 and rises at 144.
    let replay = construction_frames(150, |hour, read| {
        let lines = if hour < 148 {
            vec![building_line(1, 5, if hour < 124 { 2 } else { 1 }, 0, 15)]
        } else {
            Vec::new()
        };
        let level = match hour {
            ..110 => (1, 1),
            110..144 => (2, 2),
            _ => (3, 3),
        };
        let states = (hour != 110).then(|| vec![building_at(5, CIVILIAN, level)]);
        construction_frame(hour, read, lines, Vec::new(), states, Vec::new())
    });
    let log = scan(&replay, 1, &AtomicBool::new(false)).unwrap();
    let project = &log.projects[0];
    let levels = |hour, known_at| LevelsBuilt {
        hour,
        known_at,
        count: 1,
        repair: false,
    };
    assert_eq!(project.unaccounted, [levels(None, 124), levels(None, 148)]);
    assert!(project.built.is_empty());
    let end = project.end.as_ref().unwrap();
    assert_eq!((end.kind, end.hour), (EndKind::Unknown, None));
}

#[test]
fn a_project_first_listed_at_the_same_place_makes_a_rise_nobody_s() {
    use crate::products::construction_log::{EndKind, UnknownEnd, tests::timed_scan as scan};
    let rail = engine_native::cbuilding_database::RAILWAY_TEMPLATE_TOKEN;
    // A (1 left at 100) leaves before 124; B, created at 110 at the same
    // building, builds a level at 120 and is first listed at 124 with one left.
    // Railway R leaves before 124 and S, first listed at 124 (created at 95),
    // starts from R's last province, which rose at 120. But C, whose building rose
    // at the daily update at 120, and railway T, whose last province rose then too,
    // still have no native completion proof: the lines first listed beside them (D, U) were created
    // only after the rises (121, 122), and a line never builds before it exists.
    let replay = construction_frames(130, |hour, read| {
        let mut lines = Vec::new();
        let mut rails = Vec::new();
        if hour < 124 {
            lines.push(building_line(1, 20, 1, 0, 15));
            lines.push(building_line(3, 21, 1, 0, 15));
            rails.push(railway_line(10, &[3001, 3002], 1));
            rails.push(railway_line(12, &[4001, 4002], 1));
        } else {
            lines.push(replay_data::rows::BuildingConstructionRow {
                created_hour: 110,
                ..building_line(2, 20, 1, 0, 15)
            });
            lines.push(replay_data::rows::BuildingConstructionRow {
                created_hour: 121,
                ..building_line(4, 21, 1, 0, 15)
            });
            rails.push(railway_line(11, &[3002, 3003], 1));
            rails.push(replay_data::rows::RailwayConstructionRow {
                created_hour: 122,
                ..railway_line(13, &[4002, 4003], 1)
            });
        }
        let level = if hour < 120 { (1, 1) } else { (2, 2) };
        let rose = |at: i32| if hour < at { (1, 1) } else { (2, 2) };
        let provinces = vec![
            building_at(3001, rail, (2, 2)),
            building_at(3002, rail, level),
            building_at(3003, rail, (1, 1)),
            building_at(4001, rail, (2, 2)),
            building_at(4002, rail, rose(120)),
            building_at(4003, rail, (1, 1)),
        ];
        let states = Some(vec![
            building_at(20, CIVILIAN, level),
            building_at(21, CIVILIAN, rose(120)),
        ]);
        construction_frame(hour, read, lines, rails, states, provinces)
    });
    let log = scan(&replay, 1, &AtomicBool::new(false)).unwrap();
    let ending = |id: &str| {
        let project = log
            .projects
            .iter()
            .find(|project| project.id == id)
            .unwrap();
        let end = project.end.as_ref().unwrap();
        (end.kind, end.hour, end.unknown)
    };
    for id in ["1", "10", "3", "12"] {
        assert_eq!(
            ending(id),
            (
                EndKind::Unknown,
                None,
                Some(UnknownEnd::CompletionNotRecorded)
            ),
            "project {id}"
        );
    }
}

/// A little TGA image of side-by-side 2x2 frames, frame k all of `colours[k]`.
fn strip_tga(colours: &[[u8; 4]]) -> Vec<u8> {
    let width = 2 * colours.len() as u16;
    let mut bytes = vec![0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    bytes.extend(width.to_le_bytes());
    bytes.extend(2_u16.to_le_bytes());
    bytes.extend([32, 0x28]);
    for _ in 0..2 {
        for [r, g, b, a] in colours.iter().flat_map(|colour| [*colour, *colour]) {
            bytes.extend([b, g, r, a]);
        }
    }
    bytes
}

/// The first pixel of a PNG data URL.
fn first_pixel(url: &str) -> [u8; 4] {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(url.strip_prefix("data:image/png;base64,").unwrap())
        .unwrap();
    let mut reader = png::Decoder::new(std::io::Cursor::new(bytes))
        .read_info()
        .unwrap();
    let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
    reader.next_frame(&mut pixels).unwrap();
    pixels[..4].try_into().unwrap()
}

/// The fixture Targa's BGRA pixels in a one-level, 32-bit DDS.
fn fixture_dds(tga: &[u8]) -> Vec<u8> {
    let width = u32::from(u16::from_le_bytes(tga[12..14].try_into().unwrap()));
    let height = u32::from(u16::from_le_bytes(tga[14..16].try_into().unwrap()));
    let mut bytes = vec![0; 128];
    bytes[..4].copy_from_slice(b"DDS ");
    for (offset, value) in [
        (4, 124_u32),
        (8, 0x100F),
        (12, height),
        (16, width),
        (20, width * 4),
        (28, 1),
        (76, 32),
        (80, 0x41),
        (88, 32),
        (92, 0xFF_0000),
        (96, 0xFF00),
        (100, 0xFF),
        (104, 0xFF00_0000),
        (108, 0x1000),
    ] {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    bytes.extend_from_slice(&tga[18..]);
    bytes
}

#[test]
fn construction_icons_follow_strip_direction_and_the_gui_repair_frame() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    use crate::products::construction_log::{ContentView, named};
    use dashboard_text::catalogue::tests::folder_catalogue;
    use std::collections::BTreeSet;

    let [red, green, blue] = [[200, 0, 0, 255], [0, 200, 0, 255], [0, 0, 200, 255]];
    for vertical in [false, true] {
        for frame in [-1, 0, 1, 2, 3] {
            let gui = format!(
                r#"guiTypes = {{ containerWindowType = {{ name = "production_building_line_entry"
                    iconType = {{ name = "picture" spriteType = "building_strip" frame = 99 }}
                    iconType = {{ name = "repair" spriteType = "repair_strip" frame = {frame} }} }} }}"#
            );
            let direction = if vertical { "yes" } else { "no" };
            let gfx = format!(
                r#"spriteTypes = {{
                    spriteType = {{ name = "building_strip" texturefile = "gfx/buildings.tga" noOfFrames = 3 frameDirectionY = {direction} }}
                    frameAnimatedSpriteType = {{ name = "repair_strip" texturefile = "gfx/repairs.tga" noOfFrames = 2 frameDirectionY = {direction} }} }}"#
            );
            let (catalogue, folder) = folder_catalogue(&[
                (
                    "common/buildings/strips.txt",
                    "buildings = { fixture_building = { icon_frame = 2 } }",
                ),
                ("interface/strips.gui", &gui),
                ("interface/strips.gfx", &gfx),
            ]);
            let texture = |colours: &[[u8; 4]]| {
                let mut bytes = strip_tga(colours);
                if vertical {
                    bytes[12..14].copy_from_slice(&2_u16.to_le_bytes());
                    bytes[14..16].copy_from_slice(&(2 * colours.len() as u16).to_le_bytes());
                    bytes.truncate(18);
                    for [r, g, b, a] in colours.iter().flat_map(|colour| [*colour; 4]) {
                        bytes.extend([b, g, r, a]);
                    }
                }
                bytes
            };
            std::fs::create_dir_all(folder.0.join("gfx")).unwrap();
            std::fs::write(folder.0.join("gfx/buildings.tga"), texture(&[blue; 3])).unwrap();
            std::fs::write(folder.0.join("gfx/repairs.tga"), texture(&[green; 2])).unwrap();
            // The native resolver chooses DDS ahead of the authored TGA name.
            std::fs::write(
                folder.0.join("gfx/buildings.dds"),
                fixture_dds(&texture(&[red, green, blue])),
            )
            .unwrap();
            std::fs::write(
                folder.0.join("gfx/repairs.dds"),
                fixture_dds(&texture(&[red, blue])),
            )
            .unwrap();
            let view = ContentView {
                layers: &catalogue.content.layers,
                sprites: &catalogue.sprites,
                guis: catalogue.guis().map_err(|error| error.to_string()),
                gui_decode: &catalogue.gui_decode,
                localise: &|_| None,
            };
            let named = named(
                &view,
                &[(42, Some("fixture_building".into()))],
                &BTreeSet::new(),
                &AtomicBool::new(false),
            )
            .unwrap();
            assert_eq!(named.types[&42].1.as_deref().map(first_pixel), Some(green));
            // The animated repair strip is cropped across only; one down its
            // texture is refused by name.
            let expected = match frame {
                _ if vertical => None,
                1 => Some(red),
                2 => Some(blue),
                _ => None,
            };
            assert_eq!(named.repair_icon.as_deref().map(first_pixel), expected);
            if vertical && (1..=2).contains(&frame) {
                assert!(named.warnings.iter().any(|(_, warning)| warning.contains(
                    "production_building_line_entry.repair (repair_strip, frameanimatedspritetype) runs down its texture, which is not supported"
                )));
            } else if expected.is_none() {
                assert!(named.warnings.iter().any(|(_, warning)| warning.contains(
                    &format!("production_building_line_entry.repair (repair_strip): frame {frame} is outside 1..=2")
                )));
            }
        }
    }
}

#[test]
fn construction_icons_use_cached_guis_and_the_native_fallback_and_refuse_unknown_decode() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    use crate::products::construction_log::{ContentView, named};
    use dashboard_text::catalogue::tests::folder_catalogue;
    use std::collections::BTreeSet;
    let fallback =
        std::str::from_utf8(installed_content_core::gui::native::resources::FALLBACK_SPRITE).unwrap();
    let gfx = format!(
        r#"spriteTypes = {{ spriteType = {{ name = "{fallback}" texturefile = "gfx/fallback.tga" noOfFrames = 2 }} }}"#
    );
    let (mut catalogue, folder) = folder_catalogue(&[
        (
            "common/buildings/fixture.txt",
            "buildings = { fixture_building = { icon_frame = 2 } }",
        ),
        (
            "interface/fixture.gui",
            r#"guiTypes = { containerWindowType = { name = "production_building_line_entry"
            iconType = { name = "picture" spriteType = "absent_picture" }
            iconType = { name = "repair" spriteType = "absent_repair" frame = 1 } } }"#,
        ),
        ("interface/fixture.gfx", &gfx),
    ]);
    let [red, blue] = [[200, 0, 0, 255], [0, 0, 200, 255]];
    std::fs::create_dir_all(folder.0.join("gfx")).unwrap();
    std::fs::write(folder.0.join("gfx/fallback.tga"), strip_tga(&[red, blue])).unwrap();
    let types = [(42, Some("fixture_building".into()))];
    let read = |catalogue: &dashboard_text::catalogue::Catalogue| {
        named(
            &ContentView {
                layers: &catalogue.content.layers,
                sprites: &catalogue.sprites,
                guis: catalogue.guis().map_err(|error| error.to_string()),
                gui_decode: &catalogue.gui_decode,
                localise: &|_| Some("Fixture building".into()),
            },
            &types,
            &BTreeSet::new(),
            &AtomicBool::new(false),
        )
        .unwrap()
    };
    let known = read(&catalogue);
    assert_eq!(known.types[&42].1.as_deref().map(first_pixel), Some(blue));
    assert_eq!(known.repair_icon.as_deref().map(first_pixel), Some(red));
    // After the first dependent lookup, the cached registry supplies the controls.
    std::fs::write(folder.0.join("interface/fixture.gui"), "guiTypes = { }").unwrap();
    let cached = read(&catalogue);
    assert_eq!(cached.types[&42], known.types[&42]);
    assert_eq!(cached.repair_icon, known.repair_icon);
    catalogue.gui_decode = Err("settings.txt texture decode is unknown".into());
    let unknown = read(&catalogue);
    assert_eq!(unknown.types[&42], (Some("Fixture building".into()), None));
    assert!(unknown.repair_icon.is_none());
    assert!(
        unknown
            .warnings
            .iter()
            .any(|(_, why)| why.contains("fixture_building (frame 2: settings.txt"))
    );
    assert!(unknown.warnings.iter().any(
        |(_, why)| why.contains("repair marker is unavailable") && why.contains("settings.txt")
    ));
}

#[test]
fn refused_state_read_keeps_live_wording() {
    use crate::products::construction_log::{ContentView, named};
    use std::collections::BTreeSet;
    let (catalogue, _folder) = dashboard_text::catalogue::tests::folder_catalogue(&[(
        "common/buildings/test.txt",
        "buildings={mod_building={icon_frame=-1}}",
    )]);
    let error = dashboard_text::scripted_text::read_state_content(&catalogue.content.layers)
        .err()
        .unwrap();
    let expected = format!("State names are unavailable: {error}");
    let prepared = named(
        &ContentView {
            layers: &catalogue.content.layers,
            sprites: &catalogue.sprites,
            guis: catalogue.guis().map_err(|error| error.to_string()),
            gui_decode: &catalogue.gui_decode,
            localise: &|_| Some("Mod building".into()),
        },
        &[(1, Some("mod_building".into()))],
        &BTreeSet::from([1]),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(
        prepared
            .live_warnings()
            .filter(|message| message.starts_with("State names"))
            .collect::<Vec<_>>(),
        [expected.as_str()]
    );
}

#[test]
fn construction_titles_preserve_empty_and_key_equal_localisations() {
    use super::{ContentView, named};
    use std::collections::BTreeSet;

    let (catalogue, _folder) = dashboard_text::catalogue::tests::folder_catalogue(&[
        (
            "common/defines/defines.lua",
            "NDefines={NBuildings={MAX_BUILDING_LEVELS=15}}",
        ),
        (
            "common/buildings/buildings.txt",
            "buildings={mod_empty={} mod_same={} mod_missing={}}",
        ),
        (
            "map/default.map",
            "definitions=definition.csv provinces=provinces.bmp terrain=terrain.bmp rivers=rivers.bmp heightmap=heightmap.bmp tree_definition=trees.bmp",
        ),
        ("map/definition.csv", "0;0;0;0;land;false;unknown;0\n"),
        (
            "history/states/1-empty.txt",
            "state={id=1 name=STATE_EMPTY}",
        ),
        ("history/states/2-same.txt", "state={id=2 name=STATE_SAME}"),
        (
            "history/states/3-missing.txt",
            "state={id=3 name=STATE_MISSING}",
        ),
        (
            "localisation/english/construction_l_english.yml",
            "\u{feff}l_english:\n mod_empty:0 \"\"\n mod_same:0 \"mod_same\"\n STATE_EMPTY:0 \"\"\n STATE_SAME:0 \"STATE_SAME\"\n",
        ),
    ]);
    let localise = |key: &str| catalogue.localised(key, dashboard_text::catalogue::View::Plain);
    let view = ContentView {
        layers: &catalogue.content.layers,
        sprites: &catalogue.sprites,
        guis: catalogue.guis().map_err(|error| error.to_string()),
        gui_decode: &catalogue.gui_decode,
        localise: &localise,
    };
    let named = named(
        &view,
        &[
            (101, Some("mod_empty".into())),
            (102, Some("mod_same".into())),
            (103, Some("mod_missing".into())),
        ],
        &BTreeSet::from([1, 2, 3]),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(named.types[&101].0.as_deref(), Some(""));
    assert_eq!(named.types[&102].0.as_deref(), Some("mod_same"));
    assert_eq!(named.types[&103].0, None);
    assert_eq!(named.states[&1].as_deref(), Some(""));
    assert_eq!(named.states[&2].as_deref(), Some("STATE_SAME"));
    assert_eq!(named.states[&3], None);
    let absent: Vec<_> = named
        .warnings
        .iter()
        .map(|(_, message)| message)
        .filter(|message| message.contains("have no name"))
        .collect();
    assert_eq!(absent.len(), 2, "{absent:?}");
    assert!(
        absent
            .iter()
            .any(|message| message.starts_with("1 building types")
                && message.ends_with(": mod_missing."))
    );
    assert!(
        absent
            .iter()
            .any(|message| message.starts_with("1 states") && message.ends_with(": 3."))
    );
}

#[test]
fn a_modded_content_stack_names_and_pictures_its_own_buildings_and_refuses_what_it_cannot() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    use crate::products::construction_log::{ContentView, named};
    use std::collections::{BTreeMap, BTreeSet};
    let root = std::env::temp_dir().join(format!(
        "dashboard_construction_mod_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let (base, modded) = (root.join("base"), root.join("mod"));
    dashboard_text::catalogue::texture_tests::write_sprite_program(&base);
    let write = |dir: &Path, file: &str, bytes: &[u8]| {
        let path = dir.join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    };
    let [red, green, blue] = [[200, 0, 0, 255], [0, 200, 0, 255], [0, 0, 200, 255]];
    write(
        &base,
        "common/buildings/00_buildings.txt",
        b"buildings = { industrial_complex = { icon_frame = 1 } }",
    );
    write(
        &base,
        "interface/countryconstructionsview.gui",
        br#"guiTypes = { containerWindowType = { name = "production_building_line_entry"
            iconType = { name = "picture" spriteType = "GFX_buildings_strip" }
            iconType = { name = "repair" spriteType = "GFX_building_repair" } } }"#,
    );
    write(
        &base,
        "interface/construction.gfx",
        br#"spriteTypes = {
            spriteType = { name = "GFX_buildings_strip" texturefile = "gfx/interface/strip.tga" noOfFrames = 3 }
            frameAnimatedSpriteType = { name = "GFX_building_repair" texturefile = "gfx/interface/repair.tga" noOfFrames = 2 } }"#,
    );
    write(
        &base,
        "gfx/interface/strip.tga",
        &strip_tga(&[red, green, blue]),
    );
    write(&base, "gfx/interface/repair.tga", &strip_tga(&[green, red]));
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
    // The mod replaces the vanilla file (the civilian factory moves to frame 2),
    // adds its own types in another file, and adds a state.
    write(
        &modded,
        "common/buildings/00_buildings.txt",
        b"buildings = { industrial_complex = { icon_frame = 2 } }",
    );
    write(
        &modded,
        "common/buildings/zz_mod_buildings.txt",
        b"buildings = { mod_reactor = { icon_frame = 3 } mod_frameless = { } mod_scripted = { icon_frame = -1 } mod_far = { icon_frame = 9 } mod_twice = { icon_frame = 3 } mod_twice = { icon_frame = 4 } CUSTOM_FACTORY = { icon_frame = 1 } MOD_CASE = { icon_frame = 2 } mod_case = { icon_frame = 3 } }",
    );
    // A different file registers the later queue template and both mod sprites.
    write(
        &modded,
        "interface/zz_mod_construction.gui",
        br#"guiTypes = { containerWindowType = { name = "production_building_line_entry"
            iconType = { name = "picture" spriteType = "GFX_mod_buildings_strip" }
            iconType = { name = "repair" spriteType = "GFX_mod_repair" } } }"#,
    );
    write(
        &modded,
        "interface/zz_mod_construction.gfx",
        br#"spriteTypes = {
            spriteType = { name = "GFX_mod_buildings_strip" texturefile = "gfx/interface/mod_strip.tga" noOfFrames = 3 }
            spriteType = { name = "GFX_mod_repair" texturefile = "gfx/interface/mod_repair.tga" } }"#,
    );
    write(
        &modded,
        "gfx/interface/mod_strip.tga",
        &strip_tga(&[blue, red, green]),
    );
    write(&modded, "gfx/interface/mod_repair.tga", &strip_tga(&[blue]));
    write(
        &modded,
        "history/states/900-Atlantis.txt",
        br#"state = { id = 900 name = "STATE_900" }"#,
    );
    let layers =
        installed_content_core::ResolvedLayers::from_folders(&[(&base, &[][..]), (&modded, &[][..])])
            .unwrap();
    let catalogue = dashboard_text::catalogue::tests::layers_catalogue(layers);
    let texts = BTreeMap::from([
        ("industrial_complex", "Civilian Factory"),
        ("mod_reactor", "Fusion Reactor"),
        ("mod_frameless", "Frameless Works"),
        ("mod_twice", "Twice Works"),
        ("CUSTOM_FACTORY", "Custom Factory"),
        ("mod_case", "Ambiguous Works"),
        ("STATE_900", "Atlantis"),
    ]);
    let long_name = "Construction name ".repeat(30);
    let localise = |key: &str| {
        if matches!(key, "industrial_complex" | "STATE_900") {
            Some(long_name.clone())
        } else {
            texts.get(key).map(|text| text.to_string())
        }
    };
    let view = ContentView {
        layers: &catalogue.content.layers,
        sprites: &catalogue.sprites,
        guis: catalogue.guis().map_err(|error| error.to_string()),
        gui_decode: &catalogue.gui_decode,
        localise: &localise,
    };
    let types = [
        (CIVILIAN, Some("industrial_complex".to_owned())),
        (500, Some("mod_reactor".to_owned())),
        (501, Some("mod_frameless".to_owned())),
        (502, Some("mod_scripted".to_owned())),
        (503, Some("mod_far".to_owned())),
        (504, Some("not_a_building".to_owned())),
        (505, None),
        (506, Some("mod_twice".to_owned())),
        (507, Some("custom_factory".to_owned())),
        (508, Some("mod_case".to_owned())),
    ];
    let named = named(
        &view,
        &types,
        &BTreeSet::from([5, 900]),
        &AtomicBool::new(false),
    )
    .unwrap();
    let _ = std::fs::remove_dir_all(&root);
    let name = |id: u32| named.types[&id].0.as_deref();
    let icon = |id: u32| named.types[&id].1.as_deref().map(first_pixel);
    // The mod's own frame for the vanilla type, and its own type with its own frame.
    assert_eq!(
        (name(CIVILIAN), icon(CIVILIAN)),
        (Some(long_name.as_str()), Some(red))
    );
    assert_eq!(
        (name(500), icon(500)),
        (Some("Fusion Reactor"), Some(green))
    );
    assert_eq!((name(507), icon(507)), (Some("Custom Factory"), Some(blue)));
    // No frame, a negative frame, one past the strip or a type declared twice
    // with differing fields: no icon (the widget's placeholder), each refusal
    // counted by name.
    assert_eq!((name(501), icon(501)), (Some("Frameless Works"), None));
    assert_eq!((name(506), icon(506)), (Some("Twice Works"), None));
    assert_eq!((name(508), icon(508)), (Some("Ambiguous Works"), None));
    for id in [502, 503, 504, 505] {
        assert_eq!((name(id), icon(id)), (None, None), "{id}");
    }
    assert_eq!(named.repair_icon.as_deref().map(first_pixel), Some(blue));
    assert_eq!(
        (named.states[&5].as_deref(), named.states[&900].as_deref()),
        (None, Some(long_name.as_str()))
    );
    let warned = |start: &str| {
        named
            .warnings
            .iter()
            .map(|(_, message)| message)
            .find(|warning| warning.starts_with(start))
            .unwrap_or_else(|| panic!("{start}: {:?}", named.warnings))
            .clone()
    };
    assert!(warned("1 building types have icon_frame 0").ends_with(": mod_frameless."));
    assert!(warned("1 building types give a negative icon_frame").ends_with(": mod_scripted."));
    assert!(
        warned("2 building types are declared more than once with differing fields")
            .ends_with(": mod_twice, mod_case.")
    );
    assert!(warned("1 building icons could not be read").contains("mod_far (frame 9"));
    assert!(warned("1 building types are not declared").ends_with(": not_a_building."));
    assert!(
        warned("1 building types are not in the recorded lexical dictionary").ends_with(": 505.")
    );
    assert!(
        warned("3 building types have no name")
            .ends_with(": mod_scripted, mod_far, not_a_building.")
    );
    assert!(warned("1 states have no name").ends_with(": 5."));
    for (prefix, expected) in [
        (
            "1 building types give a negative icon_frame",
            installed_content_core::DiagnosticKind::Refused,
        ),
        (
            "2 building types are declared more than once",
            installed_content_core::DiagnosticKind::Ambiguous,
        ),
        (
            "1 building types have icon_frame 0",
            installed_content_core::DiagnosticKind::Fallback,
        ),
        (
            "3 building types have no name",
            installed_content_core::DiagnosticKind::Fallback,
        ),
    ] {
        assert_eq!(
            named
                .warnings
                .iter()
                .find(|(_, message)| message.starts_with(prefix))
                .unwrap()
                .0,
            expected
        );
    }
    assert!(named.declared_types.is_empty());
}

#[test]
fn levels_left_when_a_project_leaves_unknowably_stay_unaccounted_on_every_path() {
    use crate::products::construction_log::{
        EndKind, LevelsBuilt, UnknownEnd, tests::timed_scan as scan,
    };
    // A building line (two left) and a railway (one segment left, empty route of
    // a second railway aside) are read at 100; hours 101..=123 are missing and
    // read 124 finds both gone: either could have finished in the gap.
    let frames = [100, 124]
        .into_iter()
        .map(|hour| {
            let (lines, rails) = if hour == 100 {
                (
                    vec![building_line(1, 5, 2, 0, 15)],
                    vec![railway_line(10, &[1001, 1002], 1), railway_line(11, &[], 1)],
                )
            } else {
                (Vec::new(), Vec::new())
            };
            let rail = engine_native::cbuilding_database::RAILWAY_TEMPLATE_TOKEN;
            let provinces = vec![
                building_at(1001, rail, (1, 1)),
                building_at(1002, rail, (1, 1)),
            ];
            construction_frame(
                hour,
                true,
                lines,
                rails,
                Some(vec![building_at(5, CIVILIAN, (1, 1))]),
                provinces,
            )
        })
        .collect();
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    let log = scan(&replay, 1, &AtomicBool::new(false)).unwrap();
    let project = |id: &str| {
        log.projects
            .iter()
            .find(|project| project.id == id)
            .unwrap()
    };
    let unaccounted = |count| LevelsBuilt {
        hour: None,
        known_at: 124,
        count,
        repair: false,
    };
    for (id, count, why) in [
        ("1", 2, UnknownEnd::NotRecorded),
        ("10", 1, UnknownEnd::CompletionNotRecorded),
        ("11", 1, UnknownEnd::CompletionNotRecorded),
    ] {
        let end = project(id).end.as_ref().unwrap();
        assert_eq!(
            (end.kind, end.unknown),
            (EndKind::Unknown, Some(why)),
            "{id}"
        );
        assert!(project(id).built.is_empty(), "{id}");
        assert_eq!(project(id).unaccounted, [unaccounted(count)], "{id}");
    }
}

#[test]
fn building_types_are_named_once_a_later_dictionary_read_succeeds_and_read_once_while_unchanged() {
    use crate::products::construction_log::tests::timed_scan as scan;
    // The dictionary is unavailable until 110, then complete and unchanged to 160.
    let replay = construction_frames(160, |hour, read| {
        let mut frame = construction_frame(
            hour,
            read,
            vec![building_line(1, 5, 1, 0, 3)],
            Vec::new(),
            Some(vec![building_at(5, CIVILIAN, (1, 1))]),
            Vec::new(),
        );
        if hour < 110 {
            *frame.tables[TableId::LexicalTokens.index()] = TableView::Unavailable {
                reason: "fixture".into(),
            };
        }
        frame
    });
    let log = scan(&replay, 1, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        log.types,
        [(CIVILIAN, Some("industrial_complex".to_owned()))]
    );
    // Fifty-one complete hours share one dictionary: taken once, not each hour.
    assert_eq!(log.dictionaries, 1);
}

#[test]
fn a_progress_fall_keeps_build_and_addition_only_possible() {
    use crate::products::construction_log::{EndKind, LevelsBuilt, tests::timed_scan as scan};
    // Line 1: one left at 100 and 124, a progress fall and a target rise at
    // 120. Completion plus addition is possible, but negative output plus a
    // focus rise fits the same reads. Another rise and departure prove no count.
    // Line 2: one left at 100, rises at 110 (not a daily update) and 120, then
    // gone at 124. Without a native outcome, departure also proves no count.
    let replay = construction_frames(150, |hour, read| {
        let mut lines = Vec::new();
        if hour < 148 {
            lines.push(replay_data::rows::BuildingConstructionRow {
                produced: if hour < 124 {
                    1_000_000_000
                } else {
                    100_000_000
                },
                ..building_line(1, 5, 1, 0, 15)
            });
        }
        if hour < 124 {
            lines.push(building_line(2, 6, 1, 0, 15));
        }
        let level = |rises: &[i32]| {
            let risen = rises.iter().filter(|at| hour >= **at).count() as i16;
            (1 + risen, 1 + risen)
        };
        let states = Some(vec![
            building_at(5, CIVILIAN, level(&[120, 144])),
            building_at(6, CIVILIAN, level(&[110, 120])),
        ]);
        construction_frame(hour, read, lines, Vec::new(), states, Vec::new())
    });
    let log = scan(&replay, 1, &AtomicBool::new(false)).unwrap();
    let project = |id: &str| {
        log.projects
            .iter()
            .find(|project| project.id == id)
            .unwrap()
    };
    let levels = |hour, known_at| LevelsBuilt {
        hour,
        known_at,
        count: 1,
        repair: false,
    };
    let one = project("1");
    assert!(one.built.is_empty() && one.added.is_empty());
    assert_eq!(one.possible, [levels(None, 124)]);
    assert_eq!(one.unaccounted, [levels(None, 148)]);
    let end = one.end.as_ref().unwrap();
    assert_eq!((end.kind, end.hour), (EndKind::Unknown, None));
    let two = project("2");
    assert!(two.built.is_empty());
    assert!(two.added.is_empty());
    assert_eq!(two.unaccounted, [levels(None, 124)]);
    let end = two.end.as_ref().unwrap();
    assert_eq!((end.kind, end.hour), (EndKind::Unknown, None));
}

#[test]
fn a_line_that_repairs_then_builds_is_classified_per_stretch() {
    use crate::products::construction_log::{EndKind, LevelsBuilt, tests::timed_scan as scan};
    // First read with one level to repair and one new level queued behind it: the
    // building is repaired at 120 (healthy 2 to 3), read 124 shows the repair done
    // and the new level under way, the new level rises at 144, gone at 148.
    let replay = construction_frames(150, |hour, read| {
        let lines = if hour < 148 {
            vec![replay_data::rows::BuildingConstructionRow {
                building_type: CIVILIAN,
                ..building_line(1, 5, 1, if hour < 124 { 1 } else { 0 }, 15)
            }]
        } else {
            Vec::new()
        };
        let levels = match hour {
            ..120 => (3, 2),
            120..144 => (3, 3),
            _ => (4, 4),
        };
        construction_frame(
            hour,
            read,
            lines,
            Vec::new(),
            Some(vec![building_at(5, CIVILIAN, levels)]),
            Vec::new(),
        )
    });
    let log = scan(&replay, 1, &AtomicBool::new(false)).unwrap();
    let project = &log.projects[0];
    assert!(project.repair);
    assert_eq!(
        project
            .runs
            .iter()
            .map(|run| (run.first_hour, run.remaining, run.repair_levels))
            .collect::<Vec<_>>(),
        [(100, 1, 1), (124, 1, 0)]
    );
    let levels = |hour, known_at, repair| LevelsBuilt {
        hour: Some(hour),
        known_at,
        count: 1,
        repair,
    };
    assert_eq!(project.built, [levels(120, 124, true)]);
    assert!(project.added.is_empty());
    assert_eq!(project.unaccounted[0].count, 1);
    let end = project.end.as_ref().unwrap();
    assert_eq!((end.kind, end.hour), (EndKind::Unknown, None));
}

#[test]
fn a_rise_the_line_does_not_show_it_made_is_not_claimed() {
    use crate::products::construction_log::{LevelsBuilt, tests::timed_scan as scan};
    let cost = 1_080_000_000;
    // Line 1: one left at every read and unchanged progress; a focus raises the
    // building at 120. It may have built and had a level added, or built nothing.
    // Line 2: one left at 100 and 124 with falling progress. A decision raises
    // the building at 110 and a daily rise follows at 120. Neither progress nor
    // target rises prove a completion that was hidden by a level added.
    // Line 3: two left at 100, one at 124; a decision raises the building at 110
    // and construction at 120. Its completion is dated 120, not 110.
    // Line 4: one left at 100 and 124, its progress falling, but the building was
    // damaged at 115 and the line has a level to repair at 124: the repair branch
    // keeps `produced` at zero, so the fall is no completion, and the rise at 110
    // is not the line's; the damage is added to its repair.
    let replay = construction_frames(150, |hour, read| {
        let line = |id, state, amount, repair_levels, produced| {
            replay_data::rows::BuildingConstructionRow {
                produced,
                building_type: CIVILIAN,
                ..building_line(id, state, amount, repair_levels, 0)
            }
        };
        let before = hour < 124;
        let lines = vec![
            replay_data::rows::BuildingConstructionRow {
                speed: 0,
                ..line(1, 5, 1, 0, 300_000_000)
            },
            line(
                2,
                6,
                1,
                0,
                if before {
                    cost - 80_000_000
                } else {
                    20_000_000
                },
            ),
            line(3, 7, if before { 2 } else { 1 }, 0, 0),
            line(
                4,
                8,
                1,
                if before { 0 } else { 1 },
                if before { cost - 80_000_000 } else { 0 },
            ),
        ];
        let rose = |rises: &[i32]| {
            let risen = rises.iter().filter(|at| hour >= **at).count() as i16;
            (1 + risen, 1 + risen)
        };
        let damaged = if hour < 110 {
            (1, 1)
        } else if hour < 115 {
            (2, 2)
        } else {
            (2, 1)
        };
        let states = Some(vec![
            building_at(5, CIVILIAN, rose(&[120])),
            building_at(6, CIVILIAN, rose(&[110, 120])),
            building_at(7, CIVILIAN, rose(&[110, 120])),
            building_at(8, CIVILIAN, damaged),
        ]);
        construction_frame(hour, read, lines, Vec::new(), states, Vec::new())
    });
    let log = scan(&replay, 1, &AtomicBool::new(false)).unwrap();
    let project = |id: &str| {
        log.projects
            .iter()
            .find(|project| project.id == id)
            .unwrap()
    };
    let levels = |hour, known_at, repair| LevelsBuilt {
        hour,
        known_at,
        count: 1,
        repair,
    };
    let one = project("1");
    assert!(one.built.is_empty() && one.added.is_empty() && one.unaccounted.is_empty());
    assert_eq!(one.possible, [levels(None, 124, false)]);
    assert!(one.end.is_none());
    let two = project("2");
    assert!(two.built.is_empty() && two.added.is_empty() && two.unaccounted.is_empty());
    assert_eq!(two.possible, [levels(None, 124, false)]);
    let three = project("3");
    assert_eq!(three.built, [levels(Some(120), 124, false)]);
    assert!(three.added.is_empty() && three.unaccounted.is_empty());
    let four = project("4");
    assert!(four.built.is_empty() && four.unaccounted.is_empty());
    assert_eq!(four.added, [levels(None, 124, true)]);
}

/// Construction frames for hours 100..=`last`, read at `reads`, whose building
/// queue read fails at `failed`.
pub(crate) fn construction_frames_failing(
    last: i32,
    reads: &[i32],
    failed: &[i32],
    frame: impl Fn(i32, bool) -> DecodedFrame,
) -> Replay {
    let frames = (100..=last)
        .map(|hour| {
            let mut frame = frame(hour, reads.contains(&hour));
            if failed.contains(&hour) {
                *frame.tables[TableId::BuildingConstruction.index()] = TableView::Unavailable {
                    reason: "fixture".into(),
                };
            }
            frame
        })
        .collect();
    Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap()
}

#[test]
fn a_line_s_opening_work_is_zero_only_when_its_opening_stretch_shows_no_daily_rise() {
    use crate::products::construction_log::{EndKind, tests::timed_scan as scan};
    let replay = construction_frames(150, |hour, read| {
        let mut lines = vec![building_line(3, 7, 1, 0, 15)];
        if hour >= 124 {
            for (id, state, created) in [
                (1, 5, 110),
                (2, 6, 110),
                (4, 8, 100),
                (5, 9, 110),
                (6, 10, 110),
                (7, 11, 110),
            ] {
                if id != 1 || hour < 145 {
                    lines.push(replay_data::rows::BuildingConstructionRow {
                        created_hour: created,
                        ..building_line(id, state, 1, 0, 15)
                    });
                }
            }
        }
        let level = match hour {
            ..120 => (0, 0),
            120..144 => (1, 1),
            _ => (2, 2),
        };
        let states = Some(vec![
            building_at(5, CIVILIAN, level),
            building_at(6, CIVILIAN, (1, 1)),
            building_at(7, CIVILIAN, (1, 1)),
            building_at(8, CIVILIAN, (1, 1)),
            // A healthy-only daily rise is opening repair work too.
            building_at(9, CIVILIAN, if hour < 120 { (2, 1) } else { (2, 2) }),
            // Neither an off-update rise nor a damaged level rise can be its work.
            building_at(10, CIVILIAN, if hour < 115 { (1, 1) } else { (2, 2) }),
            building_at(11, CIVILIAN, if hour < 120 { (2, 1) } else { (3, 1) }),
        ]);
        construction_frame(hour, read, lines, Vec::new(), states, Vec::new())
    });
    let log = scan(&replay, 1, &AtomicBool::new(false)).unwrap();
    let project = |id: &str| {
        log.projects
            .iter()
            .find(|project| project.id == id)
            .unwrap()
    };
    let first = project("1");
    assert!(first.earlier_unknown);
    assert_eq!((first.created_hour, first.first_read), (110, 124));
    // Neither a pre-read rise nor a final between-read rise proves its own work.
    assert!(first.built.is_empty());
    assert_eq!(first.end.as_ref().unwrap().kind, EndKind::Unknown);
    assert_eq!(first.unaccounted[0].count, 1);
    for id in ["3", "4", "5"] {
        assert!(project(id).earlier_unknown, "project {id}");
    }
    for id in ["2", "6", "7"] {
        assert!(!project(id).earlier_unknown, "project {id}");
    }
}

#[test]
fn a_lost_opening_hour_stays_unknown_until_a_successful_read_resets_it() {
    use crate::products::construction_log::tests::timed_scan as scan;
    for gap in [false, true] {
        let frames = (100..=150)
            .filter(|&hour| !gap || hour != 110)
            .map(|hour| {
                let mut lines = Vec::new();
                if hour >= 124 {
                    lines.push(replay_data::rows::BuildingConstructionRow {
                        created_hour: 110,
                        ..building_line(1, 5, 1, 0, 15)
                    });
                }
                if hour >= 148 {
                    lines.push(replay_data::rows::BuildingConstructionRow {
                        created_hour: 130,
                        ..building_line(2, 6, 1, 0, 15)
                    });
                }
                let states = (hour != 110).then(|| {
                    vec![
                        building_at(5, CIVILIAN, (1, 1)),
                        building_at(6, CIVILIAN, (1, 1)),
                    ]
                });
                construction_frame(
                    hour,
                    [100, 124, 148].contains(&hour),
                    lines,
                    Vec::new(),
                    states,
                    Vec::new(),
                )
            })
            .collect();
        let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
        let log = scan(&replay, 1, &AtomicBool::new(false)).unwrap();
        assert!(log.projects[0].earlier_unknown, "gap {gap}");
        assert!(!log.projects[1].earlier_unknown, "gap {gap}");
    }
}

#[test]
fn an_opening_stretch_keeps_changes_through_a_failed_queue_read() {
    use crate::products::construction_log::tests::timed_scan as scan;
    let replay = construction_frames_failing(150, &[100, 124, 148], &[124], |hour, read| {
        let lines = if hour >= 148 {
            [1, 2]
                .map(|id| replay_data::rows::BuildingConstructionRow {
                    created_hour: 110,
                    ..building_line(id, id as u32 + 4, 1, 0, 15)
                })
                .to_vec()
        } else {
            Vec::new()
        };
        let states = Some(vec![
            building_at(5, CIVILIAN, if hour < 120 { (1, 1) } else { (2, 2) }),
            building_at(6, CIVILIAN, (1, 1)),
        ]);
        construction_frame(hour, read, lines, Vec::new(), states, Vec::new())
    });
    let log = scan(&replay, 1, &AtomicBool::new(false)).unwrap();
    assert_eq!(log.reads, [100, 148]);
    assert!(log.projects[0].earlier_unknown);
    assert!(!log.projects[1].earlier_unknown);
}

#[test]
fn a_railway_s_opening_work_is_checked_over_its_route_and_its_own_reads() {
    use crate::products::construction_log::{EndKind, tests::timed_scan as scan};
    let rail = engine_native::cbuilding_database::RAILWAY_TEMPLATE_TOKEN;
    let replay = construction_frames(150, |hour, read| {
        let mut rails = vec![railway_line(3, &[2001, 2002], 1)];
        if (124..145).contains(&hour) {
            rails.push(replay_data::rows::RailwayConstructionRow {
                created_hour: 110,
                ..railway_line(1, &[1001, 1002, 1003], 1)
            });
        }
        if hour >= 124 {
            rails.push(replay_data::rows::RailwayConstructionRow {
                created_hour: 110,
                ..railway_line(2, &[3001, 3002], 1)
            });
        }
        let rose = |at| if hour < at { (1, 1) } else { (2, 2) };
        let provinces = vec![
            building_at(1001, rail, (2, 2)),
            building_at(1002, rail, rose(120)),
            building_at(1003, rail, rose(144)),
            building_at(2001, rail, (2, 2)),
            building_at(2002, rail, (1, 1)),
            building_at(3001, rail, (2, 2)),
            building_at(3002, rail, (1, 1)),
        ];
        construction_frame(hour, read, Vec::new(), rails, Some(Vec::new()), provinces)
    });
    let log = scan(&replay, 1, &AtomicBool::new(false)).unwrap();
    let project = |id: &str| {
        log.projects
            .iter()
            .find(|project| project.id == id)
            .unwrap()
    };
    assert!(project("1").earlier_unknown);
    assert!(!project("2").earlier_unknown);
    assert!(project("3").earlier_unknown);
    assert_eq!(project("1").end.as_ref().unwrap().kind, EndKind::Unknown);

    // A successful building read must not clear a failed railway read's stretch.
    let replay = construction_frames(150, |hour, read| {
        let rails = if hour >= 148 {
            vec![replay_data::rows::RailwayConstructionRow {
                created_hour: 110,
                ..railway_line(4, &[1001, 1002], 1)
            }]
        } else {
            Vec::new()
        };
        let provinces = vec![
            building_at(1001, rail, (2, 2)),
            building_at(1002, rail, if hour < 120 { (1, 1) } else { (2, 2) }),
        ];
        let mut frame =
            construction_frame(hour, read, Vec::new(), rails, Some(Vec::new()), provinces);
        if hour == 124 {
            *frame.tables[TableId::RailwayConstruction.index()] = TableView::Unavailable {
                reason: "fixture".into(),
            };
        }
        frame
    });
    let log = scan(&replay, 1, &AtomicBool::new(false)).unwrap();
    assert_eq!(log.railway_reads, [100, 148]);
    assert!(log.projects[0].earlier_unknown);
}

#[test]
fn a_replacement_that_repaired_before_its_first_read_makes_the_repair_nobody_s() {
    use crate::products::construction_log::{EndKind, UnknownEnd, tests::timed_scan as scan};
    // A repairs one level from 100 and leaves at 110 (its builder lost the place).
    // B, created at 111 at the same building to repair it and then build, repairs
    // at 120 and is first listed at 124 with nothing left to repair and one new
    // level. The repair may be B's, so A is not finished by it.
    let replay = construction_frames(130, |hour, read| {
        let mut lines = Vec::new();
        if hour < 110 {
            lines.push(building_line(1, 5, 0, 1, 0));
        }
        if hour >= 124 {
            lines.push(replay_data::rows::BuildingConstructionRow {
                created_hour: 111,
                building_type: INFRASTRUCTURE,
                ..building_line(2, 5, 1, 0, 0)
            });
        }
        let levels = if hour < 120 { (3, 2) } else { (3, 3) };
        let states = Some(vec![building_at(5, INFRASTRUCTURE, levels)]);
        construction_frame(hour, read, lines, Vec::new(), states, Vec::new())
    });
    let log = scan(&replay, 1, &AtomicBool::new(false)).unwrap();
    let a = &log.projects[0];
    assert!(a.built.is_empty());
    let end = a.end.as_ref().unwrap();
    assert_eq!(
        (end.kind, end.hour, end.unknown),
        (
            EndKind::Unknown,
            None,
            Some(UnknownEnd::CompletionNotRecorded)
        )
    );
}

#[test]
fn sampled_speed_cannot_choose_between_possible_completion_dates() {
    use crate::products::construction_log::{LevelsBuilt, tests::timed_scan as scan};
    // Cost 100, 40 produced at 100 and two levels left; one left at 148 (the read
    // at 124 fails); the building rises at the daily updates at 120 and 144.
    // Line 1's speed is 40, then 60 at 148; line 2's is 40 at both reads.
    // More factories for a day can complete either at 120, followed by a focus
    // supplying 144. Both completions are dated by the read, never at 144.
    let replay = construction_frames_failing(150, &[100, 124, 148], &[124], |hour, read| {
        let line = |id, state, speed| replay_data::rows::BuildingConstructionRow {
            cost: 100,
            produced: 40,
            speed,
            ..building_line(id, state, if hour < 148 { 2 } else { 1 }, 0, 15)
        };
        let lines = vec![line(1, 5, if hour < 148 { 40 } else { 60 }), line(2, 6, 40)];
        let level = match hour {
            ..120 => (1, 1),
            120..144 => (2, 2),
            _ => (3, 3),
        };
        let states = Some(vec![
            building_at(5, CIVILIAN, level),
            building_at(6, CIVILIAN, level),
        ]);
        construction_frame(hour, read, lines, Vec::new(), states, Vec::new())
    });
    let log = scan(&replay, 1, &AtomicBool::new(false)).unwrap();
    let levels = |hour| LevelsBuilt {
        hour,
        known_at: 148,
        count: 1,
        repair: false,
    };
    assert_eq!(log.projects[0].built, [levels(None)]);
    assert_eq!(log.projects[1].built, [levels(None)]);
    for project in &log.projects {
        assert_eq!(project.possible, [levels(None)]);
        assert!(project.added.is_empty() && project.unaccounted.is_empty());
    }
}

#[test]
fn extra_possible_completions_of_a_listed_line_widen_built_and_added_counts() {
    use crate::products::construction_log::{LevelsBuilt, tests::timed_scan as scan};
    // Two left at 100, one at 148, unchanged progress. Both daily rises may be
    // built with a level added, or one may be supplied by a focus or event.
    let replay = construction_frames_failing(150, &[100, 148], &[], |hour, read| {
        let lines = vec![replay_data::rows::BuildingConstructionRow {
            produced: 40,
            cost: 100,
            ..building_line(1, 5, if hour < 148 { 2 } else { 1 }, 0, 15)
        }];
        let level = match hour {
            ..120 => (1, 1),
            120..144 => (2, 2),
            _ => (3, 3),
        };
        construction_frame(
            hour,
            read,
            lines,
            Vec::new(),
            Some(vec![building_at(5, CIVILIAN, level)]),
            Vec::new(),
        )
    });
    let log = scan(&replay, 1, &AtomicBool::new(false)).unwrap();
    let project = &log.projects[0];
    let levels = LevelsBuilt {
        hour: None,
        known_at: 148,
        count: 1,
        repair: false,
    };
    assert_eq!(project.built.as_slice(), std::slice::from_ref(&levels));
    assert_eq!(project.possible, [levels]);
    assert!(project.added.is_empty() && project.unaccounted.is_empty() && project.end.is_none());
}

#[test]
fn a_repair_started_and_finished_between_reads_is_counted() {
    use crate::products::construction_log::{LevelsBuilt, tests::timed_scan as scan};
    // A line building one new level, with nothing to repair at 100 and 124; its
    // building is damaged at 110 and repaired at the daily update at 120. The line
    // had the repair work and did it: one level repaired, one added, and the new
    // level still queued.
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
    let log = scan(&replay, 1, &AtomicBool::new(false)).unwrap();
    let project = &log.projects[0];
    let repair = |hour| LevelsBuilt {
        hour,
        known_at: 124,
        count: 1,
        repair: true,
    };
    assert_eq!(project.built, [repair(Some(120))]);
    assert_eq!(project.added, [repair(None)]);
    assert!(project.unaccounted.is_empty() && project.end.is_none());
}

#[test]
fn construction_queue_reads_follow_cadence_without_a_timed_slot_walk() {
    let replay_data::cadence::Cadence::Daily(slot) =
        replay_data::cadence::cadence(TableId::BuildingConstruction)
    else {
        panic!("BuildingConstruction is read in a daily slot");
    };
    let hour = 1000 * replay_data::cadence::STAGGER_PERIOD + slot as i32;
    let frame = |hour, factories| {
        construction_frame(
            hour,
            false,
            vec![building_line(1, 5, 1, 0, factories)],
            vec![],
            Some(vec![building_at(5, CIVILIAN, (1, 1))]),
            vec![],
        )
    };
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(1, vec![frame(hour, 10), frame(hour + 1, 99)])],
    )
    .unwrap();
    let log = super::scan(&replay, 1, &AtomicBool::new(false)).unwrap();
    assert_eq!(log.reads, [hour]);
    assert_eq!(log.railway_reads, [hour]);
    assert_eq!(log.projects.len(), 1);
    assert_eq!(log.projects[0].first_read, hour);
    assert_eq!(log.projects[0].runs.len(), 1);
    assert_eq!(log.projects[0].runs[0].factories, 10);
    assert_eq!(log.projects[0].runs[0].last_hour, hour + 1);
}

/// A building line at state 20 with `amount` left, `produced` and cost 1000.
pub(crate) fn spending_line(
    id: u64,
    amount: i32,
    produced: i64,
    created_hour: i32,
) -> replay_data::rows::BuildingConstructionRow {
    replay_data::rows::BuildingConstructionRow {
        cost: 100_000_000,
        produced,
        created_hour,
        building_type: CIVILIAN,
        ..building_line(id, 20, amount, 0, 5)
    }
}

#[test]
fn concurrent_requests_for_a_cold_branch_share_one_scan() {
    use crate::products::construction_log::{scanned, scans_run};
    let replay = Arc::new(construction_frames(130, |hour, read| {
        construction_frame(
            hour,
            read,
            vec![spending_line(1, 1, 0, 90)],
            Vec::new(),
            Some(vec![building_at(20, CIVILIAN, (1, 1))]),
            Vec::new(),
        )
    }));
    let source = dashboard_base::source::Source::from_replay(
        dashboard_base::source::SourceKey {
            source_id: "coalesce".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        replay,
        &AtomicBool::new(false),
    )
    .unwrap();
    let logs: Vec<_> = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..4)
            .map(|_| scope.spawn(|| scanned(&source, 1, &AtomicBool::new(false)).unwrap()))
            .collect();
        workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect()
    });
    assert_eq!(scans_run(&source), 1);
    assert!(logs.iter().all(|log| Arc::ptr_eq(log, &logs[0])));
}

/// A synthetic branch of `periods` windows of 720 hours, read daily and unrecorded
/// for their last 12 hours, with the output table unavailable at one read in each.
/// Each window starts three projects: one created in the window before and finished
/// in the next, one whose final output was lost (read without a row to the branch's
/// end), and one created after the read that first lists it.
fn periodic_log(periods: i32) -> (super::ScannedLog, Vec<super::BattleWindow>) {
    use super::{
        BattleWindow, ConstructionOutputRead, ConstructionProject, EndKind, LevelsBuilt,
        ProjectEnd, ProjectKind, ProjectRun, ScannedLog, SpendingGap,
    };
    const BASE: i32 = 1_000_000;
    let last = BASE + 720 * periods - 1;
    let windows = (0..periods)
        .map(|period| BattleWindow {
            first_hour: BASE + 720 * period,
            last_hour: BASE + 720 * period + 707,
        })
        .collect();
    let mut log = ScannedLog {
        output_reads: (BASE + 10..=last)
            .step_by(24)
            .map(|hour| {
                let read = (hour - BASE) / 24;
                (hour, (read % 30 == 12).then_some(SpendingGap::ReadFailed))
            })
            .collect(),
        ..ScannedLog::default()
    };
    for period in 0..periods {
        let at = BASE + 720 * period;
        let end = |kind, hour, known_at| {
            Some(ProjectEnd {
                kind,
                hour,
                known_at,
                unknown: None,
            })
        };
        // Created, first read, last row published, last output read, the row's
        // output (none: growing at each read) and ending.
        let lines = [
            (
                at - 100,
                at + 10,
                at + 730,
                at + 730,
                None,
                end(EndKind::Finished, Some(at + 712), at + 730),
            ),
            (
                at + 8,
                at + 10,
                at + 250,
                last,
                Some("7"),
                end(EndKind::Unknown, None, at + 274),
            ),
            (
                at + 730,
                at + 682,
                at + 826,
                at + 826,
                Some("0"),
                end(EndKind::Finished, None, at + 826),
            ),
        ];
        for (index, (created_hour, first_read, published, until, raw, end)) in
            lines.into_iter().enumerate()
        {
            let output = log
                .output_reads
                .iter()
                .filter(|(hour, _)| (first_read..=until).contains(hour))
                .map(|&(hour, gap)| {
                    let row = gap.is_none() && hour <= published;
                    ConstructionOutputRead {
                        known_at: hour,
                        since_hour: row.then_some(at),
                        construction_raw: row
                            .then(|| raw.map_or_else(|| (hour - at).to_string(), str::to_owned)),
                        repair_raw: row.then(|| "0".to_owned()),
                        reason: (!row).then_some(gap.unwrap_or(SpendingGap::FinalOutputMissing)),
                        observed_since_creation: false,
                        repair_ticks: 0,
                    }
                })
                .collect();
            log.projects.push(ConstructionProject {
                id: (100 + 10 * period + index as i32).to_string(),
                kind: ProjectKind::Building,
                tag: "GER".into(),
                building: 0,
                repair: false,
                state: None,
                to_state: None,
                province: None,
                route_provinces: None,
                base_level: None,
                target_level: None,
                cost_raw: "100".into(),
                created_hour,
                first_read,
                earlier_unknown: false,
                runs: vec![ProjectRun {
                    first_hour: first_read,
                    last_hour: first_read,
                    factories: 1,
                    remaining: 1,
                    repair_levels: 0,
                }],
                built: (index == 0)
                    .then_some(LevelsBuilt {
                        hour: Some(at + 600),
                        known_at: at + 610,
                        count: 1,
                        repair: false,
                    })
                    .into_iter()
                    .collect(),
                unaccounted: Vec::new(),
                possible: Vec::new(),
                added: Vec::new(),
                end,
                first_repair_tick: None,
            });
            log.outputs.push(output);
        }
    }
    (log, windows)
}

#[test]
fn windows_hold_every_dated_project_and_the_output_its_tooltip_reads() {
    use super::{
        ConstructionLogReply, ConstructionOutputRead, ConstructionProject, Cut, SpendingGap,
        window_at,
    };
    use std::collections::BTreeMap;
    let (log, windows) = periodic_log(4);
    let (projects, outputs) = (log.projects.clone(), log.outputs.clone());
    let cut = Cut::new(Arc::new(log), windows.clone());
    let replies: Vec<_> = (0..windows.len()).map(|window| cut.reply(window)).collect();
    // The widget dates a row by the latest change known by then, else its start.
    let dated = |project: &ConstructionProject, hour: i32| match &project.end {
        Some(end) if end.known_at <= hour => end.hour.unwrap_or(end.known_at),
        _ => [&project.built, &project.unaccounted, &project.possible]
            .into_iter()
            .flatten()
            .filter(|levels| levels.known_at <= hour)
            .max_by_key(|levels| (levels.known_at, levels.hour.unwrap_or(levels.known_at)))
            .map_or(project.created_hour, |levels| {
                levels.hour.unwrap_or(levels.known_at)
            }),
    };
    // Its tooltip showed the latest read by then, from every read.
    let was = |reads: &[ConstructionOutputRead], hour| {
        reads
            .iter()
            .rev()
            .find(|read| read.known_at <= hour)
            .map(|read| {
                let raws = (read.construction_raw.clone(), read.repair_raw.clone());
                let before = read.since_hour.is_some() && !read.observed_since_creation;
                (read.known_at, raws, read.reason, before)
            })
    };
    // It now reads the hour's window and one other window listing the project.
    let now = |held: [&ConstructionLogReply; 2], id: &str, hour: i32| {
        let runs: BTreeMap<_, _> = held
            .iter()
            .flat_map(|reply| &reply.outputs)
            .filter(|output| output.id == id)
            .flat_map(|output| &output.runs)
            .map(|run| (run.from, run))
            .collect();
        let (_, run) = runs.range(..=hour).next_back()?;
        let reads = held[0];
        let known_at = if hour >= run.to {
            run.to
        } else {
            *reads
                .output_reads
                .iter()
                .rev()
                .find(|read| **read <= hour)?
        };
        let reason = if run.absent {
            let gap = reads
                .output_unavailable
                .iter()
                .find(|(read, _)| *read == known_at);
            Some(gap.map_or(SpendingGap::FinalOutputMissing, |(_, gap)| *gap))
        } else {
            run.reason
        };
        let raws = (run.construction_raw.clone(), run.repair_raw.clone());
        Some((known_at, raws, reason, run.before_observation))
    };
    let lists = |reply: &ConstructionLogReply, id: &str| {
        reply.projects.iter().any(|listed| listed.id == id)
    };
    for hour in windows[0].first_hour..windows[3].first_hour + 720 {
        let current = &replies[window_at(&windows, hour)];
        for (project, reads) in projects.iter().zip(&outputs) {
            if project.first_read > hour {
                continue;
            }
            // In the window of its row's date, or the hour's when dated later.
            let at = window_at(&windows, dated(project, hour).min(hour));
            assert!(lists(&replies[at], &project.id), "{} at {hour}", project.id);
            for member in replies.iter().filter(|reply| lists(reply, &project.id)) {
                assert_eq!(
                    now([current, member], &project.id, hour),
                    was(reads, hour),
                    "{} at {hour} with window {}",
                    project.id,
                    member.window
                );
            }
        }
    }
}

#[test]
fn a_window_does_not_grow_with_the_branch() {
    use super::Cut;
    let last = |periods| {
        let (log, windows) = periodic_log(periods);
        let window = windows.len() - 1;
        let reply = Cut::new(Arc::new(log), windows).reply(window);
        serde_json::to_string(&reply).unwrap().len()
    };
    assert_eq!(last(2), last(6));
}

#[test]
fn a_project_s_first_repair_advance_is_its_first_output_read_counting_one() {
    use replay_data::rows::ConstructionLineOutputRow;
    let replay = construction_frames(148, |hour, read| {
        let mut frame = construction_frame(
            hour,
            read,
            vec![building_line(1, 5, 1, 0, 0)],
            Vec::new(),
            Some(vec![building_at(5, CIVILIAN, (1, 1))]),
            Vec::new(),
        );
        *frame.tables[TableId::ConstructionLineOutput.index()] =
            TableView::Complete(TableRows::ConstructionLineOutput(vec![
                ConstructionLineOutputRow {
                    line: 1,
                    since_hour: 100,
                    exact: true,
                    repair_ticks: i32::from(hour >= 124),
                    ..Default::default()
                },
            ]));
        frame
    });
    let log = timed_scan(&replay, 1, &AtomicBool::new(false)).unwrap();
    assert_eq!(log.projects[0].first_repair_tick, Some(124));
    assert_eq!(log.outputs[0].len(), 3);
}

#[test]
fn construction_observer_restores_claims_from_the_shared_frame_after_two_commits() {
    let flag = AtomicBool::new(false);
    let mut frames: Vec<_> = (100..=151)
        .filter(|hour| *hour != 132)
        .map(|hour| {
            let lines = if hour < 148 {
                vec![building_line(900, 40, if hour < 124 { 2 } else { 1 }, 0, 3)]
            } else {
                vec![]
            };
            construction_frame(
                hour,
                [100, 124, 148].contains(&hour),
                lines,
                vec![],
                Some(vec![building_at(
                    40,
                    CIVILIAN,
                    (
                        if hour < 120 { 0 } else { 1 },
                        if hour < 120 { 0 } else { 1 },
                    ),
                )]),
                vec![],
            )
        })
        .collect();
    let dictionary = frames[0].tables[TableId::LexicalTokens.index()].clone();
    for frame in &mut frames {
        frame.tables[TableId::LexicalTokens.index()] = dictionary.clone();
    }
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames.clone())]).unwrap();
    let expected = timed_scan(&replay, 1, &flag).unwrap();
    let mut scan = super::Scan::default();
    for frame in &frames {
        let read = frame.metrics.table_ns[TableId::StateGarrisons.index()] != 0;
        scan.observe_reads(frame, read, read, read, [true; 2], true);
        if [117, 125].contains(&frame.hour) {
            let saved = serde_json::to_vec(&scan).unwrap();
            let value: serde_json::Value = serde_json::from_slice(&saved).unwrap();
            assert!(value["changes"].get("states").is_none());
            assert!(value.get("dictionary").is_none());
            scan = serde_json::from_slice(&saved).unwrap();
            scan.restore_frame(frame);
        }
    }
    scan.finish(&replay, 1);
    assert_eq!(
        serde_json::to_value(&scan.log).unwrap(),
        serde_json::to_value(expected).unwrap()
    );
}
