use super::*;
use recorder_derive_framework::testing::deployed::{Edits, Run, world};
use replay_columnar::manifest::Status;

const INPUTS: &[TableId] = &[TableId::CountryFuel, TableId::Countries, TableId::CountryLinks, TableId::States];

fn writers() -> Vec<Box<dyn Writer>> {
    vec![Box::new(FuelDays::default())]
}

fn fuel(country: u32, award: Option<FuelAwardDay>) -> CountryFuelRow {
    CountryFuelRow {
        country,
        ring_gain_raw: 500,
        ring_consumed_raw: 65_536,
        ring_other_raw: 1_000,
        ring_received_0_raw: 10,
        ring_received_1_raw: 20,
        ring_received_2_raw: 30,
        award_day: award.map_or(Recorded::NotRecorded, Recorded::Value),
        ..Default::default()
    }
}

fn award(hour: i32) -> FuelAwardDay {
    FuelAwardDay {
        continuity_start_hour: Some(0),
        last_hour: replay_data::rows::FuelAwardDayHour::Stamped(hour),
        gains_raw: [100, 200, 300, 100],
        own_gain_raw: 700,
        own_gain_quantity_raw: 900,
        transfers_raw: [5, 6, 7, 1_000],
        consumed_raw: 65_536,
        received_raw: [10, 20, 30],
        ..Default::default()
    }
}

fn state(id: u32, owner: u32) -> StateRow {
    StateRow { id, owner, ..Default::default() }
}

fn owned() -> Edits {
    let mut edits = Edits::default();
    world(&mut edits).put(TableId::States, state(100, 10)).put(TableId::States, state(200, 20)).put(TableId::States, state(300, 30));
    edits
}

fn read<'a>(edits: &'a mut Edits, countries: &[u32]) -> &'a mut Edits {
    for &country in countries {
        edits.put(TableId::CountryFuel, fuel(country, None));
    }
    edits
}

fn landed<'a>(edits: &'a mut Edits, countries: &[u32], level: i64) -> &'a mut Edits {
    for &country in countries {
        edits.put(TableId::CountryFuel, CountryFuelRow { current_raw: level, ..fuel(country, None) });
    }
    edits.unstamped(TableId::CountryFuel)
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

fn days(run: &Run) -> Vec<(u32, i32, u8, Option<i32>, Option<i64>)> {
    run.rows::<FuelDayRow>(TableId::FuelDays)
        .iter()
        .map(|row| {
            let gain = match row.gained {
                Recorded::Value(value) => Some(value),
                Recorded::NotRecorded => None,
            };
            (row.country, row.read_hour, row.left_out, row.overlapped, gain)
        })
        .collect()
}

fn raw_of(run: &Run, country: u32) -> [Option<i64>; 14] {
    let rows = run.rows::<FuelDayRow>(TableId::FuelDays);
    let row = rows.iter().find(|row| row.country == country).expect("a row");
    [
        &row.gained,
        &row.burned,
        &row.received_army,
        &row.received_air,
        &row.received_navy,
        &row.other,
        &row.gain_base,
        &row.gain_states,
        &row.gain_oil,
        &row.gain_negative_cost,
        &row.own_gain_quantity,
        &row.incoming,
        &row.outgoing,
        &row.flat,
    ]
    .map(|cell| match cell {
        Recorded::Value(value) => Some(*value),
        Recorded::NotRecorded => None,
    })
}

const KEPT: u8 = FuelDayRow::KEPT;
const UNSEEN: u8 = FuelDayRow::OWNERSHIP_NOT_RECORDED;
const STATELESS: u8 = FuelDayRow::NOT_UPDATED;
const OVERLAPS: u8 = FuelDayRow::OVERLAPS;

#[test]
fn a_fuel_day_is_kept_only_when_every_governing_hour_shows_the_country_owning_a_state() {
    let mut run = Run::new(writers, INPUTS);
    let mut hour = 0;
    let mut edits = owned();
    step(&mut run, &mut hour, read(&mut edits, &[10, 20, 30]));
    assert_eq!(run.status(TableId::FuelDays), Status::Complete);
    assert_eq!(days(&run), [(10, 0, UNSEEN, None, None), (20, 0, UNSEEN, None, None), (30, 0, UNSEEN, None, None)]);
    idle(&mut run, &mut hour, 17);
    step(&mut run, &mut hour, read(&mut Edits::default(), &[10, 20, 30]));
    assert_eq!(days(&run)[0], (10, 17, UNSEEN, None, None), "the slot read's day reaches back before the run");
    idle(&mut run, &mut hour, 20);
    let mut edits = Edits::default();
    edits.delete(TableId::States, 300);
    step(&mut run, &mut hour, &edits);
    idle(&mut run, &mut hour, 41);
    step(&mut run, &mut hour, read(&mut Edits::default(), &[10, 20, 30]));
    assert_eq!(days(&run), [(10, 41, KEPT, None, Some(500)), (20, 41, KEPT, None, Some(500)), (30, 41, STATELESS, None, None)]);
    let raw = raw_of(&run, 10);
    assert_eq!(raw, [Some(500), Some(65_536), Some(10), Some(20), Some(30), Some(1_000), None, None, None, None, None, None, None, None], "the ring's sums, raw");
    let chart = [Some(500), Some(65_536), None, None, None, None, Some(500), None, None, None, Some(100_000_000), Some(6_451_633_920), Some(327_680), Some(655_360), Some(983_040)];
    assert_eq!(FuelDayRow::measures(raw), chart);
    idle(&mut run, &mut hour, 65);
    step(&mut run, &mut hour, read(Edits::default().delete(TableId::CountryFuel, 30), &[10, 20]));
    assert_eq!(days(&run), [(10, 65, KEPT, None, Some(500)), (20, 65, KEPT, None, Some(500))], "a country the read no longer lists loses its row");
}

#[test]
fn fuel_cells_written_at_an_hour_no_read_stamps_write_no_day() {
    let mut run = Run::new(writers, INPUTS);
    let mut hour = 0;
    let mut edits = owned();
    step(&mut run, &mut hour, landed(&mut edits, &[10, 20, 30], 1));
    assert_eq!(run.store.view(TableId::CountryFuel.index()).sampled_at(), None);
    assert!(days(&run).is_empty(), "no read listed a country");
}

#[test]
fn an_unseen_governing_hour_leaves_the_day_out_unless_a_stateless_hour_does() {
    let mut run = Run::new(writers, INPUTS);
    let mut hour = 0;
    let mut edits = owned();
    step(&mut run, &mut hour, read(&mut edits, &[10, 30]));
    idle(&mut run, &mut hour, 20);
    let mut edits = Edits::default();
    edits.delete(TableId::States, 300);
    step(&mut run, &mut hour, &edits);
    idle(&mut run, &mut hour, 25);
    let mut edits = Edits::default();
    edits.unavailable(TableId::States, b"not read");
    step(&mut run, &mut hour, &edits);
    let mut edits = Edits::default();
    edits.status(TableId::States, Status::Complete);
    step(&mut run, &mut hour, &edits);
    idle(&mut run, &mut hour, 41);
    step(&mut run, &mut hour, read(&mut Edits::default(), &[10, 30]));
    assert_eq!(days(&run), [(10, 41, UNSEEN, None, None), (30, 41, STATELESS, None, None)]);
}

#[test]
fn applied_days_supersede_the_ring_and_no_two_kept_days_overlap() {
    let mut run = Run::new(writers, INPUTS);
    let mut hour = 0;
    let mut edits = owned();
    step(&mut run, &mut hour, read(&mut edits, &[10]));
    idle(&mut run, &mut hour, 30);
    let mut edits = Edits::default();
    edits.put(TableId::CountryFuel, fuel(10, Some(award(30))));
    step(&mut run, &mut hour, &edits);
    assert_eq!(days(&run), [(10, 30, KEPT, None, Some(700))]);
    let raw = raw_of(&run, 10);
    assert_eq!(raw, [Some(700), Some(65_536), Some(10), Some(20), Some(30), Some(1_000), Some(100), Some(200), Some(300), Some(100), Some(900), Some(5), Some(6), Some(7)], "the applied day, raw");
    assert_eq!(
        FuelDayRow::measures(raw),
        [
            Some(700),
            Some(65_536),
            Some(100),
            Some(200),
            Some(300),
            Some(100),
            None,
            Some(5),
            Some(600_000),
            Some(700_000),
            Some(98_700_000),
            Some(6_451_633_920),
            Some(327_680),
            Some(655_360),
            Some(983_040)
        ]
    );
    idle(&mut run, &mut hour, 41);
    step(&mut run, &mut hour, read(&mut Edits::default(), &[10]));
    assert_eq!(days(&run), [(10, 41, KEPT, Some(30), Some(500))], "the slot read's day leaves out the day read at 30");
    idle(&mut run, &mut hour, 50);
    let mut edits = Edits::default();
    edits.put(TableId::CountryFuel, fuel(10, Some(award(50))));
    step(&mut run, &mut hour, &edits);
    assert_eq!(days(&run), [(10, 50, OVERLAPS, None, None)]);
    assert_eq!(run.rows::<FuelDayRow>(TableId::FuelDays)[0].awards_reason, b"");
}

#[test]
fn a_resumed_fuel_days_writer_writes_the_table_of_one_never_stopped() {
    let timeline = |run: &mut Run, hour: &mut i32, until: i32| {
        if *hour == 0 {
            let mut edits = owned();
            step(run, hour, read(&mut edits, &[10, 20, 30]));
            idle(run, hour, 20);
            let mut edits = Edits::default();
            edits.delete(TableId::States, 300);
            step(run, hour, &edits);
        }
        idle(run, hour, until);
    };
    let mut unbroken = Run::new(writers, INPUTS);
    let mut unbroken_hour = 0;
    timeline(&mut unbroken, &mut unbroken_hour, 41);
    let mut stopped = Run::new(writers, INPUTS);
    let mut stopped_hour = 0;
    timeline(&mut stopped, &mut stopped_hour, 35);
    let mut resumed = stopped.resumed();
    timeline(&mut resumed, &mut stopped_hour, 41);
    step(&mut unbroken, &mut unbroken_hour, read(&mut Edits::default(), &[10, 20, 30]));
    step(&mut resumed, &mut stopped_hour, read(&mut Edits::default(), &[10, 20, 30]));
    assert_eq!(days(&unbroken), days(&resumed));
    assert_eq!(days(&resumed), [(10, 41, KEPT, None, Some(500)), (20, 41, KEPT, None, Some(500)), (30, 41, STATELESS, None, None)]);
}
