use super::*;
use recorder_derive_framework::testing::deployed::{Edits, Run, counters, division, volunteer, world};
use replay_columnar::manifest::Status;
use replay_data::rows::{CountryDeployedManpowerRow, DivisionManpowerContributor};

const INPUTS: &[TableId] = &[
    TableId::Countries,
    TableId::CountryLinks,
    TableId::Divisions,
    TableId::DivisionLandInputs,
    TableId::DivisionTemplateIcons,
    TableId::CountryCounterInputs,
    TableId::SentVolunteers,
    TableId::Ships,
    TableId::TaskForces,
    TableId::Fleets,
    TableId::AirWings,
];

fn writers() -> Vec<Box<dyn Writer>> {
    vec![Box::new(Deployed::default())]
}

fn land(division: u64, entries: &[(u32, i32)]) -> DivisionLandInputsRow {
    DivisionLandInputsRow { division, manpower: entries.iter().map(|&(country, amount)| DivisionManpowerContributor { country, amount }).collect(), ..Default::default() }
}

fn template(template: u64, exile: Option<bool>) -> DivisionTemplateIconRow {
    DivisionTemplateIconRow { template, manpower_exile: exile.map_or(Recorded::NotRecorded, Recorded::Value), ..Default::default() }
}

fn ship(id: u64, task_force: u64, acquired: Option<i32>, required: Option<i32>) -> ShipRow {
    ShipRow { id, task_force, acquired_manpower: acquired.into(), required_manpower: required.into(), ..Default::default() }
}

fn task(id: u64, fleet: u64) -> TaskForceRow {
    TaskForceRow { id, fleet, ..Default::default() }
}

fn fleet(id: u64, owner: u32) -> FleetRow {
    FleetRow { id, owner, ..Default::default() }
}

fn wing(id: u64, owner: u32, exile: Option<i32>, acquired: Option<i32>) -> AirWingRow {
    AirWingRow { id, owner, exile_country: exile.into(), acquired_manpower: acquired.into(), ..Default::default() }
}

fn forces() -> Edits {
    let mut edits = Edits::default();
    world(&mut edits)
        .put(TableId::DivisionTemplateIcons, template(7, Some(false)))
        .put(TableId::DivisionTemplateIcons, template(8, Some(true)))
        .put(TableId::Divisions, division(1, 10, 10, 0, 7))
        .put(TableId::DivisionLandInputs, land(1, &[(20, 50), (11, 1000)]))
        .put(TableId::Divisions, division(2, 10, 20, 0, 7))
        .put(TableId::DivisionLandInputs, land(2, &[(10, 5)]))
        .put(TableId::Divisions, division(3, 10, 10, 0, 8))
        .put(TableId::DivisionLandInputs, land(3, &[(10, 5)]))
        .put(TableId::Divisions, division(4, 20, 20, 10, 7))
        .put(TableId::DivisionLandInputs, land(4, &[(10, 300), (20, 7)]))
        .put(TableId::CountryCounterInputs, counters(10, Some(&[4, 4, 0, 99])))
        .put(TableId::CountryCounterInputs, counters(20, Some(&[])))
        .put(TableId::CountryCounterInputs, counters(30, Some(&[])))
        .put(TableId::Divisions, division(5, 30, 30, 0, 7))
        .put(TableId::DivisionLandInputs, land(5, &[(20, 40)]))
        .put(TableId::SentVolunteers, volunteer(20, 0, 5))
        .put(TableId::TaskForces, task(100, 200))
        .put(TableId::Fleets, fleet(200, 11))
        .put(TableId::Ships, ship(1000, 100, Some(500), Some(600)))
        .put(TableId::TaskForces, task(101, 201))
        .put(TableId::Fleets, fleet(201, 20))
        .put(TableId::Ships, ship(1001, 101, Some(70), Some(80)))
        .put(TableId::AirWings, wing(1, 10, Some(0), Some(30)))
        .put(TableId::AirWings, wing(2, 20, Some(10), Some(9)))
        .put(TableId::AirWings, wing(3, 30, Some(-1), Some(4)));
    edits
}

const U: Option<i32> = None;

fn deployed(run: &Run) -> Vec<(u32, [Option<i32>; 5])> {
    let value = |cell: &Recorded<i32>| match cell {
        Recorded::Value(value) => Some(*value),
        Recorded::NotRecorded => None,
    };
    run.rows::<CountryDeployedManpowerRow>(TableId::CountryDeployedManpower)
        .iter()
        .map(|row| (row.slot, [value(&row.army), value(&row.navy), value(&row.navy_required), value(&row.air), value(&row.exile_air)]))
        .collect()
}

#[test]
fn deployed_manpower_attributes_each_term_as_the_game_does() {
    let mut run = Run::new(writers, INPUTS);
    run.hour(&forces());
    assert_eq!(run.status(TableId::CountryDeployedManpower), Status::Complete);
    assert_eq!(
        deployed(&run),
        [(1, [Some(1600), Some(500), Some(600), Some(30), Some(9)]), (2, [Some(40), Some(70), Some(80), Some(0), Some(0)]), (3, [Some(0), Some(0), Some(0), Some(4), Some(0)])]
    );
    let mut edits = Edits::default();
    edits
        .put(TableId::DivisionLandInputs, land(1, &[(10, 1200)]))
        .delete(TableId::Divisions, 4)
        .delete(TableId::DivisionLandInputs, 4)
        .put(TableId::Fleets, fleet(201, 30))
        .put(TableId::AirWings, wing(3, 30, Some(-1), Some(i32::MAX)))
        .put(TableId::AirWings, wing(4, 30, Some(0), Some(i32::MAX)));
    run.hour(&edits);
    assert_eq!(
        deployed(&run),
        [(1, [Some(1200), Some(500), Some(600), Some(30), Some(9)]), (2, [Some(40), Some(0), Some(0), Some(0), Some(0)]), (3, [Some(0), Some(70), Some(80), Some(-2), Some(0)])],
        "the dead expeditionary leaves GER's list, the fleet's new owner takes its ship, crews wrap"
    );
    let mut edits = Edits::default();
    edits.delete(TableId::Countries, 30);
    run.hour(&edits);
    assert_eq!(deployed(&run).iter().map(|row| row.0).collect::<Vec<_>>(), [1, 2], "a slot no live country holds loses its row");
}

#[test]
fn what_deployed_manpower_cannot_know_is_not_recorded() {
    let mut run = Run::new(writers, INPUTS);
    run.hour(&forces());
    let mut edits = Edits::default();
    edits
        .put(TableId::Divisions, division(6, 20, 20, 0, 9))
        .put(TableId::DivisionLandInputs, land(6, &[(20, 1)]))
        .put(TableId::Ships, ship(1002, 999, Some(1), Some(1)))
        .put(TableId::AirWings, wing(2, 20, None, Some(9)))
        .put(TableId::CountryCounterInputs, counters(30, None));
    run.hour(&edits);
    assert_eq!(
        deployed(&run),
        [(1, [Some(1600), U, U, Some(30), U]), (2, [U, U, U, U, U]), (3, [U, U, U, Some(4), U])],
        "ITA's division of a template with no row, a ship of no task force, ITA's wing of no exile read, ROM's list not recorded"
    );
    let mut edits = Edits::default();
    edits
        .put(TableId::DivisionTemplateIcons, template(9, Some(false)))
        .delete(TableId::Ships, 1002)
        .put(TableId::AirWings, wing(2, 20, Some(10), Some(9)))
        .put(TableId::CountryCounterInputs, counters(30, Some(&[])))
        .put(TableId::DivisionTemplateIcons, template(7, None));
    run.hour(&edits);
    assert_eq!(
        deployed(&run),
        [(1, [U, Some(500), Some(600), Some(30), Some(9)]), (2, [U, Some(70), Some(80), Some(0), Some(0)]), (3, [U, Some(0), Some(0), Some(4), Some(0)])],
        "template 9 now ordinary, but template 7's exile flag is not recorded"
    );
    let mut edits = Edits::default();
    edits.put(TableId::DivisionTemplateIcons, template(7, Some(false))).unavailable(TableId::Ships, b"not read");
    run.hour(&edits);
    assert_eq!(deployed(&run), [(1, [Some(1600), U, U, Some(30), Some(9)]), (2, [Some(41), U, U, Some(0), Some(0)]), (3, [Some(0), U, U, Some(4), Some(0)])]);
    let mut edits = Edits::default();
    edits.status(TableId::Ships, Status::Complete);
    run.hour(&edits);
    assert_eq!(deployed(&run)[1], (2, [Some(41), Some(70), Some(80), Some(0), Some(0)]), "Ships complete again");
}

#[test]
fn a_resumed_deployed_writer_writes_the_tables_of_one_never_stopped() {
    let mut unbroken = Run::new(writers, INPUTS);
    unbroken.hour(&forces());
    let mut stopped = Run::new(writers, INPUTS);
    stopped.hour(&forces());
    let mut resumed = stopped.resumed();
    let mut edits = Edits::default();
    edits
        .put(TableId::DivisionLandInputs, land(1, &[(10, 7)]))
        .put(TableId::SentVolunteers, volunteer(10, 0, 5))
        .put(TableId::DivisionLandInputs, land(5, &[(20, 40), (10, 3)]))
        .put(TableId::Ships, ship(1000, 101, Some(500), Some(600)));
    unbroken.hour(&edits);
    resumed.hour(&edits);
    assert_eq!(deployed(&unbroken), deployed(&resumed));
    assert_eq!(deployed(&resumed)[0], (1, [Some(7 + 600 + 3), Some(0), Some(0), Some(30), Some(9)]));
}
