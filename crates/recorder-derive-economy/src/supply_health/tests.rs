use super::*;
use recorder_derive_framework::{
    testing::deployed::{Edits, Run, division, world},
    framework::Definitions,
    testing::noting::{self, Noted, Noting},
};
use replay_columnar::manifest::Status;
use replay_data::rows::SupplyHealthRow;

const INPUTS: &[TableId] = &[TableId::Countries, TableId::CountryLinks, TableId::Divisions, TableId::DivisionLandInputs, TableId::DivisionStatus];

fn writers() -> Vec<Box<dyn Writer>> {
    vec![Box::new(SupplyHealth::default())]
}

const THRESHOLDS: Thresholds = Thresholds { grace: 10, low: 60_000, very_low_icon: 15_000, low_icon: 40_000 };

fn definitions() -> Definitions {
    Definitions::read_none()
        .with_define(GRACE, Ok(THRESHOLDS.grace))
        .with_define(LOW, Ok(THRESHOLDS.low))
        .with_define(VERY_LOW_ICON, Ok(THRESHOLDS.very_low_icon))
        .with_define(LOW_ICON, Ok(THRESHOLDS.low_icon))
}

fn run() -> Run {
    let mut run = Run::new(writers, INPUTS);
    run.define(definitions());
    run
}

fn land(division: u64, naval_transfer: bool) -> DivisionLandInputsRow {
    DivisionLandInputsRow { division, naval_transfer, ..Default::default() }
}

fn status(division: u64, received: Option<i64>, remaining: i64) -> DivisionStatusRow {
    DivisionStatusRow {
        division,
        supply_ratio: received,
        supply_grace_maximum: Recorded::Value(1_000_000),
        supply_grace_remaining: Recorded::Value(remaining),
        supply_ratio_exact: Recorded::Value(None),
        ..Default::default()
    }
}

fn deployed(edits: &mut Edits, id: u64, roster: u32, expeditionary: u32, transfer: bool, received: i64, remaining: i64) {
    edits
        .put(TableId::Divisions, division(id, roster, roster, expeditionary, 7))
        .put(TableId::DivisionLandInputs, land(id, transfer))
        .put(TableId::DivisionStatus, status(id, Some(received), remaining));
}

fn forces() -> Edits {
    let mut edits = Edits::default();
    world(&mut edits);
    deployed(&mut edits, 1, 10, 0, false, 0, 1_000_000);
    deployed(&mut edits, 2, 11, 0, false, 14_000, 0);
    deployed(&mut edits, 3, 10, 0, false, 15_000, 0);
    deployed(&mut edits, 4, 20, 10, false, 40_000, 0);
    deployed(&mut edits, 5, 20, 0, true, 0, 0);
    edits
}

const U: Option<[i64; 3]> = None;

fn health(run: &Run) -> Vec<(u32, Option<[i64; 3]>)> {
    run.rows::<SupplyHealthRow>(TableId::SupplyHealth)
        .iter()
        .map(|row| {
            let counts = match (&row.no_supply, &row.strained, &row.in_supply) {
                (Recorded::Value(no), Recorded::Value(strained), Recorded::Value(supplied)) => Some([*no, *strained, *supplied]),
                (Recorded::NotRecorded, Recorded::NotRecorded, Recorded::NotRecorded) => None,
                cells => panic!("slot {}'s counts are known apart: {cells:?}", row.slot),
            };
            (row.slot, counts)
        })
        .collect()
}

#[test]
fn supply_states_follow_the_counter_icon() {
    let operands = |received, maximum_grace, remaining_grace, naval_transfer| Operands {
        received: [received, received],
        maximum_grace,
        remaining_grace: [remaining_grace, remaining_grace],
        naval_transfer,
    };
    let cases = [
        (operands(0, 1_000_000, 1_000_000, false), IN_SUPPLY, "full grace"),
        (operands(14_999, 1_000_000, 0, false), NO_SUPPLY, "under the very-low icon"),
        (operands(15_000, 1_000_000, 0, false), STRAINED, "at the very-low icon, under the low icon"),
        (operands(40_000, 1_000_000, 0, false), STRAINED, "at the low icon, low supply alone"),
        (operands(60_000, 1_000_000, 0, false), IN_SUPPLY, "at LOW_SUPPLY"),
        (operands(10_000, 1_000_000, 300_000, false), NO_SUPPLY, "grace lifts the effective supply, not the icon's ratio"),
        (operands(0, 1_000_000, 0, true), IN_SUPPLY, "in a naval transfer"),
        (operands(70_000, 0, 1_000_000, false), STRAINED, "no positive maximum grace: effective zero"),
        (operands(0, 1_000_000, i64::MIN, false), IN_SUPPLY, "a sentinel grace"),
    ];
    for (division, state, why) in cases {
        assert_eq!(THRESHOLDS.state(&division), state, "{why}");
    }
    let no_grace = Thresholds { grace: 0, ..THRESHOLDS };
    assert_eq!(no_grace.state(&operands(0, 1_000_000, 0, false)), IN_SUPPLY, "a zero SUPPLY_GRACE");
}

#[test]
fn supply_health_counts_each_country_s_divisions_by_state() {
    let mut run = run();
    run.hour(&forces());
    assert_eq!(run.status(TableId::SupplyHealth), Status::Complete);
    assert_eq!(health(&run), [(1, Some([1, 2, 1])), (2, Some([0, 0, 1])), (3, Some([0, 0, 0]))]);
    let mut edits = Edits::default();
    edits
        .put(TableId::DivisionStatus, status(3, Some(100_000), 0))
        .delete(TableId::Divisions, 2)
        .delete(TableId::DivisionLandInputs, 2)
        .delete(TableId::DivisionStatus, 2)
        .put(TableId::Divisions, division(4, 20, 20, 0, 7));
    run.hour(&edits);
    assert_eq!(
        health(&run),
        [(1, Some([0, 0, 2])), (2, Some([0, 1, 1])), (3, Some([0, 0, 0]))],
        "division 3 resupplied, division 2 gone, division 4 back with its roster owner"
    );
    let mut edits = Edits::default();
    edits.delete(TableId::Countries, 30);
    run.hour(&edits);
    assert_eq!(health(&run).iter().map(|row| row.0).collect::<Vec<_>>(), [1, 2], "a slot no live country holds loses its row");
}

#[test]
fn what_supply_health_cannot_know_is_not_recorded() {
    let mut run = run();
    run.hour(&forces());
    let mut edits = Edits::default();
    edits
        .put(TableId::Divisions, division(6, 20, 20, 0, 7))
        .put(TableId::DivisionLandInputs, land(6, false))
        .put(TableId::Divisions, division(7, 30, 30, 0, 7))
        .put(TableId::DivisionLandInputs, land(7, false))
        .put(TableId::DivisionStatus, DivisionStatusRow { supply_ratio_exact: Recorded::NotRecorded, ..status(7, None, 0) });
    run.hour(&edits);
    assert_eq!(health(&run), [(1, Some([1, 2, 1])), (2, U), (3, U)], "ITA's division has no status row, ROM's no supply ratio recorded");
    let mut edits = Edits::default();
    edits
        .put(TableId::DivisionStatus, status(6, Some(100_000), 0))
        .put(TableId::DivisionStatus, DivisionStatusRow { supply_ratio_exact: Recorded::Value(Some(14_500)), ..status(7, None, 0) });
    run.hour(&edits);
    assert_eq!(health(&run), [(1, Some([1, 2, 1])), (2, Some([0, 0, 2])), (3, Some([1, 0, 0]))], "the exact ratio, at the shown step, stands in for the shown one");
    let mut edits = Edits::default();
    deployed(&mut edits, 8, 99, 0, false, 100_000, 0);
    run.hour(&edits);
    assert_eq!(health(&run), [(1, U), (2, U), (3, U)], "an owner no link resolves");
    let mut edits = Edits::default();
    edits.delete(TableId::Divisions, 8);
    run.hour(&edits);
    assert_eq!(health(&run), [(1, Some([1, 2, 1])), (2, Some([0, 0, 2])), (3, Some([1, 0, 0]))]);
    let mut edits = Edits::default();
    edits.unavailable(TableId::DivisionStatus, b"not read").put(TableId::DivisionStatus, status(1, Some(0), 0));
    run.hour(&edits);
    assert!(matches!(run.status(TableId::SupplyHealth), Status::Unavailable { .. }));
    assert_eq!(health(&run)[0], (1, Some([1, 2, 1])), "rows as they were");
    let mut edits = Edits::default();
    edits.status(TableId::DivisionStatus, Status::Complete);
    run.hour(&edits);
    assert_eq!(run.status(TableId::SupplyHealth), Status::Complete);
    assert_eq!(health(&run)[0], (1, Some([2, 2, 0])), "division 1 out of supply, read whole again");
}

#[test]
fn supply_health_waits_for_its_defines_by_name() {
    let mut run = Run::new(writers, INPUTS);
    run.hour(&forces());
    let reason = |run: &Run| match run.status(TableId::SupplyHealth) {
        Status::Unavailable { reason } => String::from_utf8_lossy(run.store.view(TableId::SupplyHealth.index()).string(reason).unwrap()).into_owned(),
        status => panic!("{status:?}"),
    };
    let why = reason(&run);
    assert!(why.starts_with("the game's definition define NMilitary.SUPPLY_GRACE is not held: "), "{why}");
    assert!(health(&run).is_empty());
    run.define(Definitions::read_none().with_define(GRACE, Ok(10)).with_define(VERY_LOW_ICON, Ok(15_000)).with_define(LOW_ICON, Ok(40_000)));
    run.hour(&Edits::default());
    let why = reason(&run);
    assert!(why.contains("define NMilitary.LOW_SUPPLY is not held"), "{why}");
    run.define(definitions());
    run.hour(&Edits::default());
    assert_eq!(run.status(TableId::SupplyHealth), Status::Complete);
    assert_eq!(health(&run), [(1, Some([1, 2, 1])), (2, Some([0, 0, 1])), (3, Some([0, 0, 0]))]);
}

#[test]
fn one_division_s_supply_among_many_sets_its_country_s_row_alone() {
    fn noted() -> Vec<Box<dyn Writer>> {
        vec![Box::new(Noting(SupplyHealth::default()))]
    }
    let mut run = Run::new(noted, INPUTS);
    run.define(definitions());
    let mut edits = forces();
    for id in 100..300 {
        deployed(&mut edits, id, 20, 0, false, 100_000, 0);
    }
    run.hour(&edits);
    noting::take();
    let mut edits = Edits::default();
    edits.put(TableId::DivisionStatus, status(3, Some(100_000), 0));
    run.hour(&edits);
    let resupplied: Noted = vec![(TableId::SupplyHealth, vec![1], Vec::new())];
    assert_eq!(noting::take(), [resupplied]);
    assert_eq!(health(&run)[..2], [(1, Some([1, 1, 2])), (2, Some([0, 0, 201]))]);
    let mut edits = Edits::default();
    edits.put(TableId::DivisionStatus, status(150, Some(90_000), 0));
    run.hour(&edits);
    assert_eq!(noting::take(), [Noted::new()]);
}

#[test]
fn a_resumed_supply_writer_writes_the_table_of_one_never_stopped() {
    let mut unbroken = run();
    unbroken.hour(&forces());
    let mut stopped = run();
    stopped.hour(&forces());
    let mut resumed = stopped.resumed();
    let mut edits = Edits::default();
    edits.put(TableId::DivisionStatus, status(2, Some(100_000), 0)).put(TableId::DivisionLandInputs, land(3, true));
    deployed(&mut edits, 9, 30, 0, false, 15_000, 0);
    unbroken.hour(&edits);
    resumed.hour(&edits);
    assert_eq!(health(&unbroken), health(&resumed));
    assert_eq!(health(&resumed), [(1, Some([0, 1, 3])), (2, Some([0, 0, 1])), (3, Some([0, 1, 0]))]);
}

#[test]
fn supply_health_preserves_off_grid_exact_thresholds_and_withholds_ambiguous_historical_rows() {
    let mut run = Run::new(writers, INPUTS);
    run.define(
        Definitions::read_none()
            .with_define(GRACE, Ok(72))
            .with_define(LOW, Ok(50_500))
            .with_define(VERY_LOW_ICON, Ok(15_500))
            .with_define(LOW_ICON, Ok(35_500)),
    );
    let mut edits = Edits::default();
    world(&mut edits);
    let exact_status = |id: u64, ratio: i64, grace: i64| DivisionStatusRow {
        division: id,
        supply_ratio: None,
        supply_grace_maximum: Recorded::Value(7_200_000),
        supply_grace_remaining: Recorded::Value(DivisionStatusRow::displayed_grace(grace, Some(72))),
        supply_ratio_exact: Recorded::Value(Some(ratio)),
        supply_grace_exact: Recorded::Value(Some(grace)),
        ..Default::default()
    };
    for (id, owner, row) in [
        (1, 10, exact_status(1, 15_499, 0)),
        (2, 10, exact_status(2, 15_500, 0)),
        (3, 10, exact_status(3, 0, 3_636_000)),
        (4, 10, exact_status(4, 0, 3_635_999)),
        (5, 20, status(5, Some(14_000), 0)),
        (6, 20, status(6, Some(0), 3_672_000)),
        (7, 30, status(7, Some(15_000), 0)),
    ] {
        edits
            .put(TableId::Divisions, division(id, owner, owner, 0, 7))
            .put(TableId::DivisionLandInputs, land(id, false))
            .put(TableId::DivisionStatus, row);
    }
    run.hour(&edits);
    assert_eq!(
        health(&run),
        [(1, Some([2, 1, 1])), (2, Some([1, 0, 1])), (3, U)],
        "exact operands decide off-grid thresholds; historical 15% straddles 15.5%"
    );
    let mut edits = Edits::default();
    edits.put(TableId::DivisionStatus, status(7, Some(0), 3_600_000));
    run.hour(&edits);
    assert_eq!(health(&run)[2], (3, U), "historical 50% grace straddles 50.5% LOW_SUPPLY");
    let mut edits = Edits::default();
    edits.put(TableId::DivisionStatus, status(7, Some(16_000), 0));
    run.hour(&edits);
    assert_eq!(health(&run)[2], (3, Some([0, 1, 0])), "historical 16% is proven above 15.5% and below 50.5%");
}

#[test]
fn supply_health_classifies_high_exact_receipt_when_low_supply_exceeds_one() {
    let mut run = Run::new(writers, INPUTS);
    run.define(
        Definitions::read_none()
            .with_define(GRACE, Ok(72))
            .with_define(LOW, Ok(100_500))
            .with_define(VERY_LOW_ICON, Ok(20_000))
            .with_define(LOW_ICON, Ok(35_000)),
    );
    let mut edits = Edits::default();
    world(&mut edits);
    edits
        .put(TableId::Divisions, division(1, 10, 10, 0, 7))
        .put(TableId::DivisionLandInputs, land(1, false))
        .put(
            TableId::DivisionStatus,
            DivisionStatusRow {
                division: 1,
                supply_ratio: None,
                supply_grace_maximum: Recorded::Value(7_200_000),
                supply_grace_remaining: Recorded::Value(0),
                supply_ratio_exact: Recorded::Value(Some(0xffff_ffff)),
                supply_grace_exact: Recorded::Value(Some(0)),
                ..Default::default()
            },
        );
    run.hour(&edits);
    assert_eq!(health(&run), [(1, Some([0, 1, 0])), (2, Some([0, 0, 0])), (3, Some([0, 0, 0]))]);
}

#[test]
fn supply_health_resumes_v1_state_and_rebuilds_v2_counts_on_next_hour() {
    use recorder_derive_framework::framework::Writers;
    use replay_columnar::state::Cell;
    let defs = Definitions::read_none()
        .with_define(GRACE, Ok(72))
        .with_define(LOW, Ok(50_500))
        .with_define(VERY_LOW_ICON, Ok(15_500))
        .with_define(LOW_ICON, Ok(35_500));
    let mut run = Run::new(writers, INPUTS);
    run.define(defs.clone());
    let mut edits = Edits::default();
    world(&mut edits);
    edits
        .put(TableId::Divisions, division(1, 10, 10, 0, 7))
        .put(TableId::DivisionLandInputs, land(1, false))
        .put(
            TableId::DivisionStatus,
            DivisionStatusRow {
                division: 1,
                supply_ratio: None,
                supply_grace_maximum: Recorded::Value(7_200_000),
                supply_grace_remaining: Recorded::Value(0),
                supply_ratio_exact: Recorded::Value(Some(15_499)),
                supply_grace_exact: Recorded::Value(Some(0)),
                ..Default::default()
            },
        )
        .put(TableId::Divisions, division(2, 30, 30, 0, 7))
        .put(TableId::DivisionLandInputs, land(2, false))
        .put(TableId::DivisionStatus, status(2, Some(15_000), 0));
    run.hour(&edits);
    assert_eq!(health(&run), [(1, Some([1, 0, 0])), (2, Some([0, 0, 0])), (3, U)]);
    let mut blob = Bytes::new();
    blob.varint(0x8000_0000).unwrap();
    blob.varint(0).unwrap();
    blob.varint(1).unwrap();
    blob.varint(b"SupplyHealth".len() as u64).unwrap();
    blob.raw(b"SupplyHealth").unwrap();
    blob.varint(1).unwrap();
    blob.u8(1).unwrap();
    blob.u8(0).unwrap();
    blob.varint(0).unwrap();
    blob.varint(0).unwrap();
    {
        let slot = run.store.view(TableId::SupplyHealth.index()).slot(3).unwrap();
        let mut table = run.store.table(TableId::SupplyHealth.index()).unwrap();
        table.write(slot, 0, Cell::I64(1)).unwrap();
        table.write(slot, 1, Cell::I64(0)).unwrap();
        table.write(slot, 2, Cell::I64(0)).unwrap();
    }
    let mut resumed = Writers::new(writers());
    resumed.define(defs);
    resumed.start_run();
    resumed.load(blob.bytes(), &mut run.store).unwrap();
    resumed.prime(&run.store, 1, 10);
    run.store.begin_hour(11, &[]).unwrap();
    resumed.hour(1, 11, false, &mut run.store).unwrap();
    run.store.end_hour(&mut replay_columnar::unit::Discard).unwrap();
    assert_eq!(run.status(TableId::SupplyHealth), Status::Complete);
    assert_eq!(health(&run), [(1, Some([1, 0, 0])), (2, Some([0, 0, 0])), (3, U)]);
}
