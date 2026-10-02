use super::*;

const fn at(produced: i64, remaining: i32) -> Progress {
    Progress {
        produced,
        cost: 1_000,
        remaining,
        repair_levels: 0,
    }
}

#[test]
fn a_stretch_adds_its_produced_and_the_levels_it_finished() {
    assert_eq!(
        building_stretch(at(100, 3), at(350, 3), None, false, Ok(0)),
        Ok(250)
    );
    // A level finished: 900 → 1150 is 1000 taken off and 150 left.
    assert_eq!(
        building_stretch(at(900, 3), at(150, 2), None, false, Ok(1)),
        Ok(250)
    );
    assert_eq!(
        building_stretch(at(100, 3), at(100, 3), None, false, Ok(0)),
        Ok(0)
    );
}

#[test]
fn a_stretch_the_reads_do_not_explain_is_unknown() {
    let gap = |was: Progress, now: Progress, unrecorded: bool, shared| {
        let fell = u32::try_from(was.remaining - now.remaining).unwrap_or(0);
        building_stretch(
            was,
            now,
            unrecorded.then_some(SpendingGap::NotRecorded),
            shared,
            Ok(fell),
        )
        .unwrap_err()
    };
    assert_eq!(
        gap(at(100, 3), at(350, 3), true, false),
        SpendingGap::NotRecorded
    );
    assert_eq!(
        gap(at(100, 3), at(350, 3), false, true),
        SpendingGap::SharedTarget
    );
    // Codex round 2: one level added and one finished before the next read
    // leave the count at 1 while produced goes 100 -> 400; the building's rise
    // shows the finish, so the stretch is 300 + 1000, not 300.
    assert_eq!(
        building_stretch(at(100, 1), at(400, 1), None, false, Ok(1)),
        Ok(1_300)
    );
    assert_eq!(
        gap(at(350, 3), at(100, 3), false, false),
        SpendingGap::Unexplained
    );
    // A lost level the record cannot settle leaves the stretch unknown; one it
    // shows dropped (no rise) adds nothing but the produced it gained.
    assert_eq!(
        building_stretch(
            at(900, 3),
            at(950, 2),
            None,
            false,
            Err(SpendingGap::LevelsUnaccounted)
        ),
        Err(SpendingGap::LevelsUnaccounted)
    );
    assert_eq!(
        building_stretch(at(900, 3), at(950, 2), None, false, Ok(0)),
        Ok(50)
    );
    let dearer = Progress {
        cost: 1_200,
        ..at(150, 2)
    };
    assert_eq!(
        gap(at(900, 3), dearer, false, false),
        SpendingGap::CostChanged
    );
    // Without a level finished, a changed cost does not matter.
    assert_eq!(
        building_stretch(
            at(100, 3),
            Progress {
                cost: 1_200,
                ..at(300, 3)
            },
            None,
            false,
            Ok(0)
        ),
        Ok(200)
    );
    let repairing = Progress {
        repair_levels: 1,
        ..at(0, 3)
    };
    assert_eq!(
        gap(at(100, 3), repairing, false, false),
        SpendingGap::Repairing
    );
}

#[test]
fn a_railway_counts_its_whole_route_progress() {
    let rail = |produced, cost| Progress {
        produced,
        cost,
        remaining: 2,
        repair_levels: 0,
    };
    assert_eq!(
        railway_stretch(rail(100, 500), rail(400, 500), None),
        Ok(300)
    );
    assert_eq!(
        railway_stretch(rail(100, 500), rail(400, 600), None),
        Err(SpendingGap::CostChanged)
    );
    assert_eq!(
        railway_stretch(
            rail(100, 500),
            rail(400, 500),
            Some(SpendingGap::NotRecorded)
        ),
        Err(SpendingGap::NotRecorded)
    );
}

#[test]
fn an_ending_is_never_counted_and_says_why() {
    // The last read shows cost 1000 and produced 900, one level left; the
    // update recomputes the cost to 1200, adds 400 and finishes. Counting
    // 1000 - 900 = 100 at the last read's cost would miss 200.
    assert_eq!(
        ending(EndKind::Finished, None),
        SpendingGap::FinishedAfterRead
    );
    assert_eq!(
        ending(EndKind::Finished, Some(SpendingGap::NotRecorded)),
        SpendingGap::NotRecorded
    );
    assert_eq!(ending(EndKind::Removed, None), SpendingGap::LeftUnfinished);
    assert_eq!(ending(EndKind::Unknown, None), SpendingGap::EndUnknown);
    assert_eq!(
        ending(EndKind::Finished, Some(SpendingGap::ReadFailed)),
        SpendingGap::ReadFailed
    );
}

#[test]
fn a_first_read_counts_its_produced_only_while_its_building_held_its_level() {
    assert_eq!(before_first_read(Some(false), false, 700), Ok(700));
    assert_eq!(
        before_first_read(Some(true), false, 700),
        Err(SpendingGap::BuiltBeforeRead)
    );
    assert_eq!(
        before_first_read(Some(false), true, 0),
        Err(SpendingGap::BuiltBeforeRead)
    );
    assert_eq!(
        before_first_read(None, true, 700),
        Err(SpendingGap::NotRecorded)
    );
    assert_eq!(
        before_first_read(Some(false), false, -1),
        Err(SpendingGap::Unexplained)
    );
}

use crate::{
    products::construction_log::tests::{
        CIVILIAN, INFRASTRUCTURE, MOD_REACTOR, building_at, building_line, construction_frame,
        construction_frames, construction_frames_failing, railway_line, spending_line,
    },
    test_fixtures::{CAMPAIGN, records},
};
use replay::{container::Replay, frame::TableView, rows::TableId};
use std::{path::Path, sync::Arc};

#[test]
fn construction_spending_counts_produced_only_where_the_reads_establish_it() {
    use crate::products::{
        construction_log::{ProjectKind, tests::timed_scan as scan},
        construction_spending::{
            ConstructionSpending, LeafProject, ProjectSpending, SpendingGap, SpentStep, leaves,
        },
    };
    let rail = engine::cbuilding_database::RAILWAY_TEMPLATE_TOKEN;
    let reads = [100, 124, 148, 172];
    let line = |id: u64, state: u32, amount: i32, created_hour: i32, produced: i64| {
        replay::rows::BuildingConstructionRow {
            created_hour,
            produced,
            ..building_line(id, state, amount, 0, 5)
        }
    };
    let frames: Vec<_> = (100..=180)
        .filter(|hour| *hour != 152)
        .map(|hour| {
            let by = |values: &[(i32, i64)]| {
                values
                    .iter()
                    .rev()
                    .find(|(from, _)| hour >= *from)
                    .map_or(0, |(_, value)| *value)
            };
            let mut lines = Vec::new();
            // 1: in the queue at the first read; finishes its last level at 144.
            if hour < 148 {
                lines.push(line(
                    1,
                    5,
                    1,
                    90,
                    by(&[(100, 1_000_000_000), (124, 1_070_000_000)]),
                ));
            }
            // 2: a repair: never measured.
            if hour < 124 {
                lines.push(building_line(2, 6, 0, 1, 3));
            }
            // 3: created after the first read with its building unchanged; open
            // across the missing hour 152 to the end.
            if hour >= 124 {
                lines.push(line(
                    3,
                    7,
                    2,
                    110,
                    by(&[(124, 30_000_000), (148, 60_000_000), (172, 90_000_000)]),
                ));
            }
            // 4: created after the first read, but its building rose at 120.
            if (124..172).contains(&hour) {
                lines.push(line(
                    4,
                    8,
                    2,
                    110,
                    by(&[(124, 30_000_000), (148, 40_000_000)]),
                ));
            }
            // 5: created before the first read yet first listed at the second.
            if (124..148).contains(&hour) {
                lines.push(line(5, 9, 1, 90, 5_000_000));
            }
            // 6: removed at 148 with its level unbuilt.
            if hour < 148 {
                lines.push(line(6, 10, 1, 90, by(&[(124, 20_000_000)])));
            }
            let rails = if hour < 148 {
                vec![replay::rows::RailwayConstructionRow {
                    produced: by(&[(100, 10_000_000), (124, 30_000_000)]),
                    ..railway_line(10, &[1001, 1002, 1003], if hour < 124 { 2 } else { 1 })
                }]
            } else {
                Vec::new()
            };
            let level = |at: i32, before: i16| {
                if hour < at {
                    (before, before)
                } else {
                    (before + 1, before + 1)
                }
            };
            let states = vec![
                building_at(5, CIVILIAN, level(144, 2)),
                building_at(6, INFRASTRUCTURE, if hour < 120 { (3, 2) } else { (3, 3) }),
                building_at(7, CIVILIAN, (1, 1)),
                building_at(8, CIVILIAN, level(120, 0)),
                building_at(9, CIVILIAN, (0, 0)),
                building_at(10, CIVILIAN, (4, 4)),
            ];
            let rail_level = |at: i32| if hour < at { (1, 1) } else { (2, 2) };
            let provinces = vec![
                building_at(1001, rail, rail_level(120)),
                building_at(1002, rail, rail_level(120)),
                building_at(1003, rail, rail_level(144)),
            ];
            construction_frame(
                hour,
                reads.contains(&hour),
                lines,
                rails,
                Some(states),
                provinces,
            )
        })
        .collect();
    let replay = Arc::new(Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap());
    let log = scan(&replay, 1, &AtomicBool::new(false)).unwrap();
    assert_eq!(log.spending.len(), log.projects.len());
    let spending = |id: &str| -> &ProjectSpending {
        let index = log
            .projects
            .iter()
            .position(|project| project.id == id)
            .unwrap();
        &log.spending[index]
    };
    let one = spending("1");
    assert!(one.before_first_read);
    // 70 between reads; it finished after its last read, at a cost no read shows.
    assert_eq!(one.spent, [(124, 70_000_000)]);
    assert_eq!(one.unknown, [(148, SpendingGap::FinishedAfterRead)]);
    assert_eq!(spending("2").spent, []);
    assert_eq!(spending("2").unknown, []);
    let three = spending("3");
    assert!(!three.before_first_read);
    assert_eq!(three.spent, [(124, 30_000_000), (148, 30_000_000)]);
    assert_eq!(three.unknown, [(172, SpendingGap::NotRecorded)]);
    assert_eq!(spending("4").spent, [(148, 10_000_000)]);
    assert_eq!(
        spending("4").unknown,
        [
            (124, SpendingGap::BuiltBeforeRead),
            (172, SpendingGap::NotRecorded)
        ]
    );
    assert_eq!(
        spending("5").unknown,
        [
            (124, SpendingGap::ListedLate),
            (148, SpendingGap::LeftUnfinished)
        ]
    );
    assert_eq!(spending("6").spent, [(124, 20_000_000)]);
    assert_eq!(spending("6").unknown, [(148, SpendingGap::LeftUnfinished)]);
    // The route's progress between reads; its finish is not counted.
    assert_eq!(spending("10").spent, [(124, 20_000_000)]);
    assert_eq!(
        spending("10").unknown,
        [(148, SpendingGap::FinishedAfterRead)]
    );
    assert!(spending("10").before_first_read);

    let (grouped, before) = leaves(&log, "GER");
    assert!(leaves(&log, "SOV").0.is_empty());
    let leaves = grouped;
    assert_eq!(before, 3);
    assert_eq!(leaves.len(), 7);
    let repair = leaves.iter().find(|leaf| leaf.repair).unwrap();
    assert_eq!((repair.state, repair.spent.len()), (Some(6), 0));
    // Its project was in the queue from its first read until the read that
    // showed it gone.
    assert_eq!(
        repair.projects,
        [LeafProject {
            project: "2".into(),
            known_at: None,
            first_read: 100,
            until: Some(124),
            unread: false,
            resumes: None
        }]
    );
    // Line 3 is open across the missing hour 152: its span breaks there and
    // resumes at the next read.
    let open = leaves.iter().find(|leaf| leaf.state == Some(7)).unwrap();
    assert_eq!(
        (
            open.projects[0].until,
            open.projects[0].unread,
            open.projects[0].resumes
        ),
        (Some(152), true, Some(172))
    );
    assert_eq!(open.projects[1].until, None);
    let railway = leaves
        .iter()
        .find(|leaf| leaf.kind == ProjectKind::Railway)
        .unwrap();
    assert_eq!((railway.state, railway.to_state), (Some(40), Some(41)));
    let step = |known_at, total: &str| SpentStep {
        known_at,
        total_raw: total.into(),
    };
    assert_eq!(railway.spent, [step(124, "20000000")]);
    let three = leaves.iter().find(|leaf| leaf.state == Some(7)).unwrap();
    assert_eq!(three.spent, [step(124, "30000000"), step(148, "60000000")]);
    assert_eq!(three.unknown[0].project, "3");

    let source = crate::Source::from_replay(
        crate::SourceKey {
            source_id: "spending".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        replay,
        &AtomicBool::new(false),
    )
    .unwrap();
    let load = |tag: &str| {
        ConstructionSpending::from_scanned(
            &source,
            &crate::products::CountryParams {
                branch_id: "1".into(),
                tag: tag.into(),
            },
            &crate::products::LoadContext { content: None },
            &log,
            &AtomicBool::new(false),
        )
        .unwrap()
    };
    let reply = load("GER");
    assert_eq!(
        (
            reply.cost_scale,
            reply.before_first_read,
            reply.leaves.len()
        ),
        (100_000, 3, 7)
    );
    assert_eq!(reply.reads, reads);
    assert!(
        reply
            .types
            .iter()
            .any(|kind| kind.token.as_deref() == Some("industrial_complex"))
    );
    assert_eq!(
        reply
            .states
            .iter()
            .map(|state| state.id)
            .collect::<Vec<_>>(),
        [5, 6, 7, 8, 9, 10, 40, 41]
    );
    let other = load("SOV");
    assert!(other.leaves.is_empty());
    assert!(
        other
            .warnings
            .iter()
            .any(|warning| warning.starts_with("SOV:"))
    );
}

#[test]
fn construction_spending_groups_a_mod_building_type_named_from_the_mod_layer() {
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
        installed_content::ResolvedLayers::from_folders(&[(&base, &[][..]), (&modded, &[][..])]).unwrap();
    let sprites = installed_content::gfx::read(&layers).unwrap();
    let guis = installed_content::gui::native::read_layers(&layers).unwrap();
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
        guis: &guis,
        gui_decode: &gui_decode,
        localise: &localise,
    };
    // Two lines created after the first read: a civilian factory in Ruhr and the
    // mod's reactor in the mod's state.
    let replay = construction_frames(150, |hour, read| {
        let line = |id: u64, state: u32, building_type: u32, produced: i64| {
            replay::rows::BuildingConstructionRow {
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
        construction_frame(hour, read, lines, Vec::new(), Some(states), Vec::new())
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
    assert_eq!(civilian.spent, [step(124, "30000000")]);
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
fn spending_follows_a_line_that_repairs_then_builds() {
    use crate::products::{
        construction_log::tests::timed_scan as scan,
        construction_spending::{LeafProject, SpendingGap, SpentStep, leaves},
    };
    // One line, first read repairing one level with two new levels queued behind
    // it: the repair finishes at 120, then it builds (produced 0 -> 400 by 148).
    let replay = construction_frames(150, |hour, read| {
        let (repair_levels, produced) = if hour < 124 {
            (1, 0)
        } else if hour < 148 {
            (0, 0)
        } else {
            (0, 40_000_000)
        };
        let line = replay::rows::BuildingConstructionRow {
            building_type: CIVILIAN,
            produced,
            ..building_line(9, 13, 2, repair_levels, 3)
        };
        let levels = if hour < 120 { (3, 2) } else { (3, 3) };
        construction_frame(
            hour,
            read,
            vec![line],
            Vec::new(),
            Some(vec![building_at(13, CIVILIAN, levels)]),
            Vec::new(),
        )
    });
    let log = scan(&replay, 1, &AtomicBool::new(false)).unwrap();
    let (grouped, _) = leaves(&log, "GER");
    let repair = grouped.iter().find(|leaf| leaf.repair).unwrap();
    assert_eq!(
        repair.projects,
        [LeafProject {
            project: "9".into(),
            known_at: None,
            first_read: 100,
            until: Some(124),
            unread: false,
            resumes: None
        }]
    );
    assert!(repair.spent.is_empty() && repair.unknown.is_empty());
    let new = grouped.iter().find(|leaf| !leaf.repair).unwrap();
    assert_eq!(
        new.projects,
        [LeafProject {
            project: "9".into(),
            known_at: None,
            first_read: 100,
            until: None,
            unread: false,
            resumes: None
        }]
    );
    // The stretch it repaired in is not counted; the one it built in is.
    assert_eq!(
        new.spent,
        [SpentStep {
            known_at: 148,
            total_raw: "40000000".into()
        }]
    );
    assert_eq!(
        new.unknown
            .iter()
            .map(|gap| (gap.known_at, gap.reason))
            .collect::<Vec<_>>(),
        [(124, SpendingGap::Repairing)]
    );
}

#[test]
fn a_failed_queue_read_makes_the_stretch_across_it_unknown() {
    use crate::products::{
        construction_log::tests::timed_scan as scan, construction_spending::SpendingGap,
    };
    // Reads at 100 and 148 show cost 1000 and produced 100 -> 200 while two
    // levels finish (at 120 and 144); the read at 124 failed, so the cost the
    // first level finished at is unseen.
    let replay = construction_frames(150, |hour, read| {
        let (amount, produced) = if hour < 148 {
            (3, 10_000_000)
        } else {
            (1, 20_000_000)
        };
        let level = if hour < 120 {
            1
        } else if hour < 144 {
            2
        } else {
            3
        };
        let mut frame = construction_frame(
            hour,
            read,
            vec![spending_line(1, amount, produced, 90)],
            Vec::new(),
            Some(vec![building_at(20, CIVILIAN, (level, level))]),
            Vec::new(),
        );
        if hour == 124 {
            *frame.tables[TableId::BuildingConstruction.index()] = TableView::Unavailable {
                reason: "fixture: the read failed".into(),
            };
        }
        frame
    });
    let log = scan(&replay, 1, &AtomicBool::new(false)).unwrap();
    let spending = &log.spending[0];
    assert!(spending.spent.is_empty(), "{:?}", spending.spent);
    assert_eq!(spending.unknown, [(148, SpendingGap::ReadFailed)]);
}

#[test]
fn a_building_that_changed_before_a_first_read_leaves_its_progress_unknown() {
    use crate::products::{
        construction_log::tests::timed_scan as scan, construction_spending::SpendingGap,
    };
    // Both lines are created at 110 and first read at 124 with produced 100.
    // Line 1's building rose at 115 (a level finished) and was demolished back at
    // 120, so its endpoints match but a level's cost may have been spent. Line 2's
    // building held its level throughout.
    let replay = construction_frames(130, |hour, read| {
        let lines = if hour >= 124 {
            vec![
                spending_line(1, 1, 10_000_000, 110),
                replay::rows::BuildingConstructionRow {
                    target_location: 21,
                    target_state: 21,
                    ..spending_line(2, 1, 10_000_000, 110)
                },
            ]
        } else {
            Vec::new()
        };
        let first = if (115..120).contains(&hour) { 3 } else { 2 };
        construction_frame(
            hour,
            read,
            lines,
            Vec::new(),
            Some(vec![
                building_at(20, CIVILIAN, (first, first)),
                building_at(21, CIVILIAN, (2, 2)),
            ]),
            Vec::new(),
        )
    });
    let log = scan(&replay, 1, &AtomicBool::new(false)).unwrap();
    let spending = |id: &str| {
        let index = log
            .projects
            .iter()
            .position(|project| project.id == id)
            .unwrap();
        &log.spending[index]
    };
    assert!(spending("1").spent.is_empty());
    assert_eq!(spending("1").unknown, [(124, SpendingGap::BuiltBeforeRead)]);
    assert_eq!(spending("2").spent, [(124, 10_000_000)]);
    assert!(spending("2").unknown.is_empty());
}

#[test]
fn a_repair_erasing_progress_before_a_first_read_leaves_spending_unknown() {
    use crate::products::{
        construction_log::tests::timed_scan as scan, construction_spending::SpendingGap,
    };
    // Empty at 100; created at 110, gained 100 IC at 120. The read at 124 fails;
    // damage at 130 is fully repaired at 144, clearing produced before its first
    // successful read at 148. The building's level never changes.
    let replay = construction_frames_failing(150, &[100, 124, 148], &[124], |hour, read| {
        let lines = if hour >= 110 {
            vec![replay::rows::BuildingConstructionRow {
                repair_levels: i32::from((130..144).contains(&hour)),
                ..spending_line(
                    1,
                    1,
                    if (120..144).contains(&hour) {
                        10_000_000
                    } else {
                        0
                    },
                    110,
                )
            }]
        } else {
            Vec::new()
        };
        let health = if (130..144).contains(&hour) { 1 } else { 2 };
        construction_frame(
            hour,
            read,
            lines,
            Vec::new(),
            Some(vec![building_at(20, CIVILIAN, (2, health))]),
            Vec::new(),
        )
    });
    let log = scan(&replay, 1, &AtomicBool::new(false)).unwrap();
    assert_eq!(log.reads, [100, 148]);
    assert_eq!(log.projects[0].first_read, 148);
    assert!(log.projects[0].earlier_unknown);
    assert!(log.spending[0].spent.is_empty());
    assert_eq!(
        log.spending[0].unknown,
        [(148, SpendingGap::BuiltBeforeRead)]
    );
}

#[test]
fn a_project_that_changes_kind_twice_keeps_an_interval_per_stretch() {
    use crate::products::{
        construction_log::tests::timed_scan as scan,
        construction_spending::{LeafProject, leaves},
    };
    // Repairing at 100, building at 124, repairing again from 148.
    let replay = construction_frames(150, |hour, read| {
        let repair_levels = if (124..148).contains(&hour) { 0 } else { 1 };
        let line = replay::rows::BuildingConstructionRow {
            building_type: CIVILIAN,
            ..building_line(9, 13, 1, repair_levels, 3)
        };
        construction_frame(
            hour,
            read,
            vec![line],
            Vec::new(),
            Some(vec![building_at(13, CIVILIAN, (3, 2))]),
            Vec::new(),
        )
    });
    let log = scan(&replay, 1, &AtomicBool::new(false)).unwrap();
    let (grouped, _) = leaves(&log, "GER");
    let repair = grouped.iter().find(|leaf| leaf.repair).unwrap();
    let span = |first_read, until| LeafProject {
        project: "9".into(),
        known_at: None,
        first_read,
        until,
        unread: false,
        resumes: None,
    };
    assert_eq!(repair.projects, [span(100, Some(124)), span(148, None)]);
}

#[test]
fn an_unclaimed_rise_leaves_the_stretch_unknown() {
    use crate::products::{
        construction_log::tests::timed_scan as scan, construction_spending::SpendingGap,
    };
    // A building costs 100. Between the reads at 124 and 148 the player adds a
    // level and the line completes one (the building rises at 144), so the count
    // stays at 1 while produced goes 10 -> 35: 25 alone would miss a level's cost.
    // Whether the log claims the rise or leaves it unclaimed, spending never
    // reports 25 as known.
    let replay = construction_frames(150, |hour, read| {
        let produced = if hour < 148 { 1_000_000 } else { 3_500_000 };
        let line = replay::rows::BuildingConstructionRow {
            cost: 10_000_000,
            ..spending_line(1, 1, produced, 90)
        };
        let level = if hour < 144 { 1 } else { 2 };
        construction_frame(
            hour,
            read,
            vec![line],
            Vec::new(),
            Some(vec![building_at(20, CIVILIAN, (level, level))]),
            Vec::new(),
        )
    });
    let log = scan(&replay, 1, &AtomicBool::new(false)).unwrap();
    let spending = &log.spending[0];
    let known: Vec<_> = spending
        .spent
        .iter()
        .filter(|(hour, _)| *hour == 148)
        .collect();
    let unknown: Vec<_> = spending
        .unknown
        .iter()
        .filter(|(hour, _)| *hour == 148)
        .collect();
    // Claimed: 25 + 100; unclaimed: unknown.
    assert!(
        known == [&(148, 12_500_000)] || unknown == [&(148, SpendingGap::UnclaimedRise)],
        "known {known:?}, unknown {unknown:?}"
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
    replay.gaps.push(replay::branches::Gap {
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
