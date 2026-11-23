use super::*;
use recorder_derive_framework::testing::deployed::{Edits, Run};
use replay_columnar::manifest::Status;
use replay_data::rows::ConstructionCompletion;

const INPUTS: &[TableId] = &[
    TableId::BuildingConstruction,
    TableId::RailwayConstruction,
    TableId::StateBuildings,
    TableId::ProvinceBuildings,
    TableId::ConstructionLineOutput,
    TableId::Provinces,
];

fn writers() -> Vec<Box<dyn Writer>> {
    vec![Box::new(ConstructionProjects::default())]
}

fn line(id: u64, state: u32, building_type: u32, amount: i32) -> BuildingConstructionRow {
    BuildingConstructionRow {
        id,
        country: 10,
        active_factories: 3,
        cost: 500_000,
        amount,
        building_type,
        target_location: state,
        target_state: state,
        ..Default::default()
    }
}

fn building(state: u32, building_type: u32, level: i16, healthy_level: i16) -> BuildingRow {
    BuildingRow { location: state, building_type, level, healthy_level, active: true, ..Default::default() }
}

fn output(id: u64, construction: i64, ended: Option<Option<ConstructionCompletion>>) -> ConstructionLineOutputRow {
    ConstructionLineOutputRow {
        line: id,
        construction_raw: construction,
        exact: true,
        ended: ended.is_some(),
        completion: Recorded::Value(ended.flatten()),
        country: Recorded::Value(Some(20)),
        building_type: Recorded::Value(Some(5)),
        target_state: Recorded::Value(Some(100)),
        observed_since_creation: true,
        ..Default::default()
    }
}

fn idle(run: &mut Run, hour: &mut i32, next: i32) {
    while *hour < next {
        run.hour(&Edits::default());
        *hour += 1;
    }
}

fn step(run: &mut Run, hour: &mut i32, edits: &Edits) {
    run.hour(edits);
    *hour += 1;
}

type Shown = (
    (u64, u8, u8, Option<i32>, Option<i32>),
    (i64, Vec<(i32, u32)>, i64, Vec<(i32, u32)>, i64, i64, i64, i64, i64),
    (Option<i32>, u8, Option<i64>),
);

fn projects(run: &Run) -> Vec<Shown> {
    let known = |cell: Recorded<i64>| match cell {
        Recorded::Value(value) => Some(value),
        Recorded::NotRecorded => None,
    };
    run.rows::<P>(TableId::ConstructionProjects)
        .iter()
        .map(|row| {
            let remaining = match row.remaining {
                Recorded::Value(value) => Some(value),
                Recorded::NotRecorded => None,
            };
            (
                (row.line, row.end, row.end_reason, row.end_hour, row.end_known_at),
                (
                    row.built,
                    row.built_rises.iter().map(|rise| (rise.hour, rise.count)).collect(),
                    row.repaired,
                    row.repaired_rises.iter().map(|rise| (rise.hour, rise.count)).collect(),
                    row.added_new,
                    row.added_repair,
                    row.possible,
                    row.unaccounted_new,
                    row.unaccounted_repair,
                ),
                (remaining, row.output_reason, known(row.construction_raw)),
            )
        })
        .collect()
}

fn queued() -> Edits {
    let mut edits = Edits::default();
    edits
        .put(TableId::BuildingConstruction, line(1, 100, 5, 2))
        .put(TableId::BuildingConstruction, line(2, 200, 6, 1))
        .put(TableId::StateBuildings, building(100, 5, 1, 1))
        .put(TableId::StateBuildings, building(200, 6, 1, 1))
        .put(TableId::ConstructionLineOutput, output(2, 100, None));
    edits
}

fn timeline(run: &mut Run, hour: &mut i32, until: i32) {
    landed(run, hour, until, &[]);
}

fn landed(run: &mut Run, hour: &mut i32, until: i32, unstamped: &[TableId]) {
    let script: [(i32, fn(&mut Edits)); 4] = [
        (0, |edits| *edits = queued()),
        (24, |edits| {
            edits.put(TableId::StateBuildings, building(100, 5, 2, 2));
        }),
        (27, |edits| {
            edits.put(TableId::BuildingConstruction, line(1, 100, 5, 1)).put(TableId::BuildingConstruction, line(2, 200, 6, 1));
        }),
        (30, |edits| {
            edits.put(TableId::ConstructionLineOutput, output(2, 150, Some(Some(ConstructionCompletion { hour: 24, cancelled: false }))));
        }),
    ];
    for (at, edit) in script {
        if at < *hour || at >= until {
            continue;
        }
        idle(run, hour, at);
        let mut edits = Edits::default();
        edit(&mut edits);
        for &table in unstamped {
            edits.unstamped(table);
        }
        step(run, hour, &edits);
    }
    idle(run, hour, until);
}

#[test]
fn construction_projects_claim_levels_and_settle_endings_from_the_final_output() {
    let mut run = Run::new(writers, INPUTS);
    let mut hour = 0;
    timeline(&mut run, &mut hour, 1);
    assert_eq!(run.status(TableId::ConstructionProjects), Status::Complete);
    assert_eq!(
        projects(&run),
        [
            ((1, P::OPEN, 0, None, None), (0, vec![], 0, vec![], 0, 0, 0, 0, 0), (Some(2), P::OUTPUT_NOT_RECORDED, None)),
            ((2, P::OPEN, 0, None, None), (0, vec![], 0, vec![], 0, 0, 0, 0, 0), (Some(1), P::OUTPUT_KNOWN, Some(100)))
        ]
    );
    let first = &run.rows::<P>(TableId::ConstructionProjects)[0];
    assert!(first.earlier_unknown, "nothing was read before the run's first read");
    assert_eq!(first.census_read, Some(0));
    timeline(&mut run, &mut hour, 28);
    assert_eq!(projects(&run)[0], ((1, P::OPEN, 0, None, None), (1, vec![(24, 1)], 0, vec![], 0, 0, 0, 0, 0), (Some(1), P::OUTPUT_NOT_RECORDED, None)));
    timeline(&mut run, &mut hour, 51);
    step(&mut run, &mut hour, &Edits::default().status(TableId::BuildingConstruction, Status::Complete).delete(TableId::BuildingConstruction, 1).delete(TableId::BuildingConstruction, 2));
    assert_eq!(
        projects(&run),
        [
            ((1, P::UNKNOWN_END, P::COMPLETION_NOT_RECORDED, None, Some(51)), (1, vec![], 0, vec![], 0, 0, 0, 1, 0), (Some(1), P::OUTPUT_FINAL_MISSING, None)),
            ((2, P::FINISHED, 0, Some(24), Some(51)), (0, vec![], 0, vec![], 0, 0, 0, 1, 0), (Some(1), P::OUTPUT_KNOWN, Some(150))),
        ]
    );
    let ends = run.rows::<ConstructionLineEndRow>(TableId::ConstructionLineEnds);
    let last: Vec<(u64, i32)> = run.rows::<P>(TableId::ConstructionProjects).iter().map(|row| (row.line, row.end_known_at.unwrap_or(-1))).collect();
    assert_eq!(ends.iter().map(|end| (end.line, end.hour)).collect::<Vec<_>>(), last, "each settled line's end is logged the hour its row is written a last time");
    for (end, row) in ends.iter().zip(run.rows::<P>(TableId::ConstructionProjects).iter()) {
        assert_eq!(*end, ConstructionLineEndRow::of(end.hour, row));
        assert!(end.new_work && !end.repair_work, "levels built or lost unseen are new work, and none was repair");
        assert_eq!((end.kind, end.building_type, end.target_state), (P::BUILDING, row.building_type, row.target_state));
    }
    step(&mut run, &mut hour, Edits::default().put(TableId::StateBuildings, building(100, 5, 2, 1)));
    assert!(projects(&run).is_empty(), "settled rows are deleted the hour after");
    assert!(run.rows::<ConstructionLineEndRow>(TableId::ConstructionLineEnds).is_empty(), "an end is logged for its hour alone");
}

#[test]
fn queue_cells_written_at_an_hour_no_read_stamps_list_no_line_from_the_queue() {
    let mut run = Run::new(writers, INPUTS);
    let mut hour = 0;
    landed(&mut run, &mut hour, 1, &[TableId::BuildingConstruction, TableId::RailwayConstruction]);
    let rows = run.rows::<P>(TableId::ConstructionProjects);
    assert!(!rows.is_empty(), "the output still lists its lines");
    assert!(rows.iter().all(|row| row.census_read.is_none() && row.output_alone), "no queue was read: {rows:?}");
}

#[test]
fn shared_and_lost_stretches_claim_nothing() {
    let mut run = Run::new(writers, INPUTS);
    let mut hour = 0;
    let mut edits = Edits::default();
    edits
        .put(TableId::BuildingConstruction, line(3, 100, 7, 1))
        .put(TableId::BuildingConstruction, line(4, 100, 7, 1))
        .put(TableId::BuildingConstruction, line(5, 300, 8, 1))
        .put(TableId::StateBuildings, building(100, 7, 1, 1))
        .put(TableId::StateBuildings, building(300, 8, 1, 1));
    step(&mut run, &mut hour, &edits);
    idle(&mut run, &mut hour, 24);
    step(&mut run, &mut hour, Edits::default().put(TableId::StateBuildings, building(100, 7, 2, 2)).put(TableId::StateBuildings, building(300, 8, 2, 2)));
    idle(&mut run, &mut hour, 27);
    step(
        &mut run,
        &mut hour,
        Edits::default().put(TableId::BuildingConstruction, line(3, 100, 7, 0)).put(TableId::BuildingConstruction, line(4, 100, 7, 0)).put(TableId::BuildingConstruction, line(5, 300, 8, 1)),
    );
    let shown = projects(&run);
    assert_eq!((shown[0].1.0, shown[0].1.7, shown[1].1.0, shown[1].1.7), (0, 1, 0, 1), "shared: nothing built, the fall unaccounted as new levels");
    assert_eq!((shown[2].1.0, shown[2].1.6), (0, 1), "a rise its count did not fall for is possible");
    idle(&mut run, &mut hour, 30);
    step(&mut run, &mut hour, Edits::default().unavailable(TableId::ProvinceBuildings, b"not read"));
    step(&mut run, &mut hour, Edits::default().status(TableId::ProvinceBuildings, Status::Complete));
    idle(&mut run, &mut hour, 51);
    step(&mut run, &mut hour, Edits::default().put(TableId::BuildingConstruction, line(5, 300, 8, 0)).put(TableId::BuildingConstruction, line(3, 100, 7, 0)).put(TableId::BuildingConstruction, line(4, 100, 7, 0)));
    let shown = projects(&run);
    assert_eq!((shown[2].1.0, shown[2].1.6, shown[2].1.7, shown[2].1.8), (0, 1, 1, 0), "lost: the count's fall is unaccounted");
}

#[test]
fn an_output_alone_line_is_a_project_settled_by_its_final_output() {
    let mut run = Run::new(writers, INPUTS);
    let mut edits = Edits::default();
    edits.put(TableId::ConstructionLineOutput, output(9, 40, Some(Some(ConstructionCompletion { hour: 0, cancelled: false }))));
    run.hour(&edits);
    let rows = run.rows::<P>(TableId::ConstructionProjects);
    assert_eq!(rows.len(), 1, "{}", run.why(TableId::ConstructionProjects));
    let row = &rows[0];
    assert!(row.output_alone && row.observed_since_creation);
    assert_eq!(row.census_read, None, "no queue read listed it");
    assert_eq!((row.country, row.building_type, row.target_state, row.first_read), (20, Some(5), Some(100), 0));
    assert_eq!((row.end, row.end_hour, row.end_known_at, row.construction_raw), (P::FINISHED, Some(0), Some(0), Recorded::Value(40)));
}

#[test]
fn repairs_count_apart_from_new_levels_dated_by_their_rises() {
    let mut run = Run::new(writers, INPUTS);
    let mut hour = 0;
    let mut edits = Edits::default();
    edits.put(TableId::BuildingConstruction, BuildingConstructionRow { repair_levels: 1, ..line(6, 400, 9, 1) }).put(TableId::StateBuildings, building(400, 9, 2, 1));
    step(&mut run, &mut hour, &edits);
    idle(&mut run, &mut hour, 24);
    step(&mut run, &mut hour, Edits::default().put(TableId::StateBuildings, building(400, 9, 2, 2)));
    idle(&mut run, &mut hour, 27);
    step(&mut run, &mut hour, Edits::default().put(TableId::BuildingConstruction, line(6, 400, 9, 1)));
    assert_eq!(projects(&run)[0].1, (0, vec![], 1, vec![(24, 1)], 0, 0, 0, 0, 0));
    idle(&mut run, &mut hour, 30);
    step(&mut run, &mut hour, Edits::default().put(TableId::StateBuildings, building(400, 9, 2, 0)));
    idle(&mut run, &mut hour, 51);
    step(&mut run, &mut hour, Edits::default().put(TableId::BuildingConstruction, BuildingConstructionRow { repair_levels: 2, ..line(6, 400, 9, 1) }));
    assert_eq!(projects(&run)[0].1, (0, vec![], 1, vec![], 0, 2, 0, 0, 0), "the damage at 30 is repair work added");
}

#[test]
fn a_resumed_construction_projects_writer_writes_the_table_of_one_never_stopped() {
    let mut unbroken = Run::new(writers, INPUTS);
    let mut unbroken_hour = 0;
    timeline(&mut unbroken, &mut unbroken_hour, 51);
    let mut stopped = Run::new(writers, INPUTS);
    let mut stopped_hour = 0;
    timeline(&mut stopped, &mut stopped_hour, 26);
    let mut resumed = stopped.resumed();
    timeline(&mut resumed, &mut stopped_hour, 51);
    let mut edits = Edits::default();
    edits.delete(TableId::BuildingConstruction, 1).delete(TableId::BuildingConstruction, 2);
    unbroken.hour(&edits);
    resumed.hour(&edits);
    assert_eq!(projects(&unbroken), projects(&resumed));
    assert_eq!(projects(&resumed)[0].1.0, 1, "the level built at 24 is claimed after the resume");
}
