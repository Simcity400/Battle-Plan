use super::*;
use recorder_derive_framework::{
    testing::deployed::{Edits, Run},
    testing::noting::{self, Noted, Noting},
};
use replay_columnar::manifest::Status;
use replay_data::rows::{FuelDemandPart, FuelDemandTotalRow};

const INPUTS: &[TableId] = &[TableId::CountryFuel, TableId::FuelDemandParts];

fn writers() -> Vec<Box<dyn Writer>> {
    vec![Box::new(FuelDemand::default())]
}

fn fuel(country: u32, demand: Recorded<FuelDemandRead>) -> CountryFuelRow {
    CountryFuelRow { country, demand, ..Default::default() }
}

fn header() -> Recorded<FuelDemandRead> {
    Recorded::Value(FuelDemandRead::default())
}

fn division(country: u32, reference: u64, position: u32, rate: Option<i64>) -> FuelDemandPartRow {
    let unit = FuelDemandUnit { reference, rate_raw: rate, ..Default::default() };
    FuelDemandPartRow { country, kind: 0, ident: reference, occurrence: 0, position, part: FuelDemandPart::Division(unit) }
}

fn task_force(country: u32, reference: u64, position: u32, rate: Option<i64>) -> FuelDemandPartRow {
    let unit = FuelDemandUnit { reference, rate_raw: rate, ..Default::default() };
    FuelDemandPartRow { country, kind: 3, ident: reference, occurrence: 0, position, part: FuelDemandPart::TaskForce(unit) }
}

fn known(cell: &Recorded<i64>) -> Option<i64> {
    match cell {
        Recorded::Value(value) => Some(*value),
        Recorded::NotRecorded => None,
    }
}

fn totals(run: &Run) -> Vec<(u32, [Option<i64>; 9], [String; 3])> {
    run.rows::<FuelDemandTotalRow>(TableId::FuelDemandTotals)
        .iter()
        .map(|row| {
            let cells = [
                &row.native_army,
                &row.native_air,
                &row.native_navy,
                &row.known_army,
                &row.known_air,
                &row.known_navy,
                &row.positive_army,
                &row.positive_air,
                &row.positive_navy,
            ]
            .map(known);
            let reasons = [&row.army_reason, &row.air_reason, &row.navy_reason].map(|reason| String::from_utf8_lossy(reason).into_owned());
            (row.country, cells, reasons)
        })
        .collect()
}

fn demanded() -> Edits {
    let mut edits = Edits::default();
    edits
        .put(TableId::CountryFuel, fuel(10, header()))
        .put(TableId::CountryFuel, fuel(20, Recorded::NotRecorded))
        .put(TableId::FuelDemandParts, division(10, 1, 0, Some(1000)))
        .put(TableId::FuelDemandParts, division(10, 2, 1, Some(-100)))
        .put(TableId::FuelDemandParts, task_force(10, 3, 0, None));
    edits
}

const UNREAD: &str = "fuel current cached-unit rate unavailable";
const NO_PACK: &str = "Individual current fuel consumption was not recorded";

#[test]
fn fuel_demand_totals_sum_each_family_as_the_game_and_widened() {
    let mut run = Run::new(writers, INPUTS);
    run.hour(&demanded());
    assert_eq!(run.status(TableId::FuelDemandTotals), Status::Complete);
    let none = [None; 9];
    assert_eq!(
        totals(&run),
        [
            (10, [Some(21_600), Some(0), None, Some(21_600), Some(0), Some(0), Some(24_000), Some(0), Some(0)], [String::new(), String::new(), UNREAD.to_owned()]),
            (20, none, [NO_PACK.to_owned(), NO_PACK.to_owned(), NO_PACK.to_owned()]),
        ]
    );
    let mut edits = Edits::default();
    edits.put(TableId::FuelDemandParts, division(10, 1, 0, Some(2000))).delete(TableId::CountryFuel, 20);
    run.hour(&edits);
    assert_eq!(totals(&run), [(10, [Some(45_600), Some(0), None, Some(45_600), Some(0), Some(0), Some(48_000), Some(0), Some(0)], [String::new(), String::new(), UNREAD.to_owned()])]);
    let mut edits = Edits::default();
    edits.unavailable(TableId::FuelDemandParts, b"not read");
    run.hour(&edits);
    let lost = "FuelDemandParts was not complete".to_owned();
    assert_eq!(totals(&run), [(10, none, [lost.clone(), lost.clone(), lost])]);
    let mut edits = Edits::default();
    edits.status(TableId::FuelDemandParts, Status::Complete);
    run.hour(&edits);
    assert_eq!(totals(&run)[0].1[0], Some(45_600), "the parts are whole again");
}

#[test]
fn a_stamped_pack_a_read_left_is_not_known_and_a_read_moving_nothing_sets_nothing() {
    fn noted() -> Vec<Box<dyn Writer>> {
        vec![Box::new(Noting(FuelDemand::default()))]
    }
    let stamped = |at: i32| Recorded::Value(FuelDemandRead { hour: FuelDemandHour::Stamped(at), ..FuelDemandRead::default() });
    let mut run = Run::new(noted, INPUTS);
    let mut edits = Edits::default();
    edits
        .put(TableId::CountryFuel, fuel(10, stamped(0)))
        .put(TableId::CountryFuel, fuel(30, stamped(0)))
        .put(TableId::FuelDemandParts, division(10, 1, 0, Some(1000)));
    run.hour(&edits);
    let mut edits = Edits::default();
    edits.put(TableId::CountryFuel, fuel(30, stamped(1)));
    run.hour(&edits);
    let mut edits = Edits::default();
    edits.put(TableId::CountryFuel, fuel(30, stamped(1)));
    run.hour(&edits);
    run.hour(&edits);
    let fuel_totals = |keys: Vec<u128>| -> Noted { if keys.is_empty() { Vec::new() } else { vec![(TableId::FuelDemandTotals, keys, Vec::new())] } };
    assert_eq!(noting::take(), [fuel_totals(vec![10, 30]), fuel_totals(vec![10]), fuel_totals(vec![30]), fuel_totals(Vec::new())]);
    let mismatch = String::from_utf8_lossy(MISMATCH).into_owned();
    assert_eq!(totals(&run).iter().map(|row| row.2.clone()).collect::<Vec<_>>(), [[mismatch.clone(), mismatch.clone(), mismatch.clone()], [mismatch.clone(), mismatch.clone(), mismatch]]);
}

#[test]
fn a_read_that_changes_only_a_demand_pack_moves_it_and_leaves_an_older_stamped_pack_unknown() {
    let stamped = |at: i32| Recorded::Value(FuelDemandRead { hour: FuelDemandHour::Stamped(at), ..FuelDemandRead::default() });
    let mut run = Run::new(writers, INPUTS);
    let mut edits = demanded();
    edits.put(TableId::CountryFuel, fuel(30, stamped(0)));
    run.hour(&edits);
    assert_eq!(totals(&run)[0].1[0], Some(21_600));
    let mut edits = Edits::default();
    edits.put(TableId::CountryFuel, fuel(10, Recorded::NotRecorded));
    run.hour(&edits);
    let none = [None; 9];
    let no_pack = [NO_PACK.to_owned(), NO_PACK.to_owned(), NO_PACK.to_owned()];
    let mismatch = String::from_utf8_lossy(MISMATCH).into_owned();
    assert_eq!(
        totals(&run),
        [(10, none, no_pack.clone()), (20, none, no_pack), (30, none, [mismatch.clone(), mismatch.clone(), mismatch])],
        "the read moved only CountryFuel.demand, and left country 30's pack of hour 0"
    );
}

#[test]
fn a_resumed_fuel_demand_writer_writes_the_table_of_one_never_stopped() {
    let mut unbroken = Run::new(writers, INPUTS);
    unbroken.hour(&demanded());
    let mut stopped = Run::new(writers, INPUTS);
    stopped.hour(&demanded());
    let mut resumed = stopped.resumed();
    let mut edits = Edits::default();
    edits.delete(TableId::FuelDemandParts, division(10, 2, 1, None).key()).put(TableId::FuelDemandParts, task_force(10, 3, 0, Some(10)));
    unbroken.hour(&edits);
    resumed.hour(&edits);
    assert_eq!(totals(&unbroken), totals(&resumed));
    assert_eq!(totals(&resumed)[0].1, [Some(24_000), Some(0), Some(240), Some(24_000), Some(0), Some(240), Some(24_000), Some(0), Some(240)]);
}
