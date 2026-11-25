use super::{
    DayLeftOut, day_of, read_hour,
    awards::{award_day, award_runs, recipient_key},
    demand::{Key, ReadRow, Walk, demand_runs, stretch_reads},
};
use replay_data::rows::{CountryFuelRow, FuelAwardDay, FuelAwardDayHour, FuelAwardDayRecipient, FuelDayRow, FuelDemandHour, FuelDemandPartRow, FuelDemandRead, FuelDemandUnit, Recorded};
use std::collections::{BTreeMap, BTreeSet};

fn day(applied: bool) -> FuelDayRow {
    let mut row = FuelDayRow {
        country: 7,
        read_hour: 48,
        gained: Recorded::Value(500_000),
        burned: Recorded::Value(98_304),
        received_army: Recorded::Value(100_000),
        received_air: Recorded::Value(0),
        received_navy: Recorded::Value(200_000),
        other: Recorded::Value(32_768),
        left_out: FuelDayRow::KEPT,
        ..Default::default()
    };
    if applied {
        row.gain_base = Recorded::Value(300_000);
        row.gain_states = Recorded::Value(150_000);
        row.gain_oil = Recorded::Value(50_000);
        row.gain_negative_cost = Recorded::Value(0);
        row.own_gain_quantity = Recorded::Value(163_840);
        row.incoming = Recorded::Value(16_384);
        row.outgoing = Recorded::Value(8_192);
        row.flat = Recorded::Value(24_576);
    } else {
        for cell in [&mut row.gain_base, &mut row.gain_states, &mut row.gain_oil, &mut row.gain_negative_cost, &mut row.own_gain_quantity, &mut row.incoming, &mut row.outgoing, &mut row.flat] {
            *cell = Recorded::NotRecorded;
        }
    }
    row
}

#[test]
fn a_day_is_its_rows_raw_cells_with_sources_only_when_applied_and_otherwise_says_why_it_is_left_out() {
    let (ring, left) = day_of(&day(false));
    let ring = ring.expect("a kept ring day");
    assert_eq!((left, ring.gained_raw.as_str(), ring.burned_raw.as_str(), ring.other_raw.as_str()), (None, "500000", "98304", "32768"));
    assert_eq!(ring.received_raw, ["100000", "0", "200000"]);
    assert_eq!(ring.awards, None, "a ring day has no sources");

    let (applied, _) = day_of(&day(true));
    let awards = applied.expect("a kept applied day").awards.expect("its sources");
    assert_eq!(awards.gains_raw, ["300000", "150000", "50000", "0"]);
    assert_eq!((awards.own_gain_raw.as_str(), awards.own_gain_quantity_raw.as_str(), awards.consumed_raw.as_str()), ("500000", "163840", "98304"));
    assert_eq!(awards.transfers_raw, ["16384", "8192", "24576", "32768"], "incoming, outgoing, flat, then the total other");

    for (code, why) in [(FuelDayRow::OVERLAPS, DayLeftOut::Overlaps), (FuelDayRow::NOT_UPDATED, DayLeftOut::NotUpdated), (FuelDayRow::OWNERSHIP_NOT_RECORDED, DayLeftOut::OwnershipNotRecorded)] {
        assert_eq!(day_of(&FuelDayRow { left_out: code, ..day(false) }), (None, Some(why)));
    }
    let unrecorded = FuelDayRow { burned: Recorded::NotRecorded, ..day(false) };
    assert_eq!(day_of(&unrecorded), (None, Some(DayLeftOut::NotRecorded)), "never a zero for a cell not recorded");
}

fn recipient(reference: u64, amount: i64) -> FuelAwardDayRecipient {
    FuelAwardDayRecipient { kind: 0, reference, owner: 7, host: 0, name: Some(b"1st Division".to_vec()), name_reason: Vec::new(), first_hour: 25, last_hour: 48, amount_raw: amount }
}

fn fuel(hour: i32, award: Recorded<FuelAwardDay>) -> CountryFuelRow {
    CountryFuelRow { country: 7, award_day: award, demand: Recorded::NotRecorded, ..Default::default() }.with_hour(hour)
}

trait WithHour {
    fn with_hour(self, hour: i32) -> Self;
}

impl WithHour for CountryFuelRow {
    fn with_hour(mut self, hour: i32) -> Self {
        if let Recorded::Value(day) = &mut self.award_day {
            day.last_hour = FuelAwardDayHour::Stamped(hour);
        }
        self
    }
}

fn applied(recipients: Vec<FuelAwardDayRecipient>) -> Recorded<FuelAwardDay> {
    Recorded::Value(FuelAwardDay { country_instance: 0, continuity_start_hour: Some(0), recipients, ..Default::default() })
}

fn read(hour: i32, last: i32, row: CountryFuelRow) -> ReadRow {
    ReadRow { hour, last, starts_segment: hour == 24, row, parts: Ok(Vec::new()) }
}

#[test]
fn an_applied_day_is_refused_unless_it_is_this_reads_whole_observed_day() {
    assert!(award_day(&fuel(48, applied(Vec::new())), 48).is_ok());
    assert!(award_day(&fuel(48, Recorded::NotRecorded), 48).is_err(), "not recorded");
    let late = Recorded::Value(FuelAwardDay { country_instance: 0, continuity_start_hour: Some(30), ..Default::default() });
    assert!(award_day(&fuel(48, late), 48).is_err(), "observed for fewer than 24 updates");
    assert!(award_day(&fuel(47, applied(Vec::new())), 48).is_err(), "another read's day");
    let refused = Recorded::Value(FuelAwardDay { continuity_start_hour: Some(0), reason: b"sum past i64".to_vec(), ..Default::default() });
    assert_eq!(award_day(&fuel(48, refused), 48).err().as_deref(), Some("sum past i64"));
}

#[test]
fn a_recipients_history_is_its_receipt_in_each_known_day_a_recorded_zero_where_a_known_day_paid_it_nothing() {
    let first = recipient(11, 70_000);
    let key = recipient_key(&first);
    let reads = [
        read(24, 47, fuel(24, Recorded::NotRecorded)),
        read(48, 71, fuel(48, applied(vec![first.clone(), recipient(12, 5)]))),
        read(72, 95, fuel(72, applied(vec![recipient(12, 9)]))),
    ];
    let runs = award_runs(&reads, &key, 24, 95);
    assert_eq!(runs.iter().map(|run| (run.first_hour, run.last_hour, run.amount_raw.as_deref())).collect::<Vec<_>>(),
        [(24, 47, None), (48, 71, Some("70000")), (72, 95, Some("0"))]);
    assert!(runs[0].reason.is_some(), "an unknown day says why");
    let renamed = FuelAwardDayRecipient { name: Some(b"Renamed".to_vec()), ..first };
    assert_ne!(recipient_key(&renamed), key, "a changed stored name is another recipient");
}

fn demand(hour: i32, divisions: Vec<FuelDemandUnit>) -> CountryFuelRow {
    let pack = FuelDemandRead { hour: FuelDemandHour::Stamped(hour), country_instance: Some(0), divisions, ..Default::default() };
    CountryFuelRow { country: 7, demand: Recorded::Value(pack), ..Default::default() }
}

fn division(reference: u64, rate: Option<i64>) -> FuelDemandUnit {
    FuelDemandUnit { reference, parent: 3, ordinal: 1, rate_raw: rate, ..Default::default() }
}

#[test]
fn a_consumers_history_is_its_rate_where_the_same_occurrence_is_read_and_says_why_elsewhere() {
    let key = Key::Unit(0, 0, 41, 3, 1, None);
    let reads = [
        read(24, 47, demand(24, vec![division(41, Some(1_000))])),
        read(48, 71, demand(48, vec![division(41, None)])),
        read(72, 95, demand(72, vec![division(40, Some(5)), division(41, Some(2_000))])),
        read(96, 119, demand(95, vec![division(41, Some(3_000))])),
    ];
    let runs = demand_runs(&reads, std::slice::from_ref(&key), 24, 119).remove(0);
    assert_eq!(runs[0].amount_raw.as_deref(), Some("24000"), "a rate is projected over a day");
    assert_eq!((runs[1].amount_raw.as_deref(), runs[1].reason.is_some()), (None, true), "a rate not recorded is not a zero");
    assert_eq!((runs[2].amount_raw.as_deref(), runs[2].reason.is_some()), (None, true), "another division at its position is another consumer");
    assert_eq!((runs[3].amount_raw.as_deref(), runs[3].reason.is_some()), (None, true), "a pack read at another hour is not this read's");
}

#[test]
fn a_reads_hour_is_its_sample_hour_else_the_hour_its_applied_day_ends_and_never_a_guess() {
    assert_eq!(read_hour(&fuel(48, applied(Vec::new())), Some(50)), Some(50));
    assert_eq!(read_hour(&fuel(48, applied(Vec::new())), None), Some(48));
    assert_eq!(read_hour(&fuel(48, Recorded::NotRecorded), None), None);
    let refused = Recorded::Value(FuelAwardDay { continuity_start_hour: Some(0), reason: b"lacks 24 updates".to_vec(), ..Default::default() });
    let held = CountryFuelRow { demand: Recorded::Value(FuelDemandRead { hour: FuelDemandHour::Host, ..Default::default() }), ..fuel(41, refused) };
    assert_eq!(read_hour(&held, None), None);
    assert_eq!(read_hour(&held, Some(41)), Some(41), "the sample hour names it");
}

fn parts_at(hours: &[i32], part: &FuelDemandPartRow, with: i32) -> BTreeMap<i32, Vec<FuelDemandPartRow>> {
    hours.iter().map(|&hour| (hour, if hour == with { vec![part.clone()] } else { Vec::new() })).collect()
}

#[test]
fn the_read_in_force_where_a_window_cuts_a_stretch_is_dated_by_its_read_keeps_its_parts_and_continues_its_segment() {
    let part = FuelDemandPartRow { country: 7, position: 3, ..Default::default() };
    let hours = [100, 113, 137];
    let mut rows: BTreeMap<i32, CountryFuelRow> = hours.iter().map(|&hour| (hour, fuel(hour, Recorded::NotRecorded))).collect();
    let mut held = parts_at(&hours, &part, 100);
    let mut out = Vec::new();
    stretch_reads(&Walk { from: 100, to: 160, last: 150, cut: true, read: Some(89) }, &hours, &mut rows, &mut held, &BTreeSet::new(), &mut out);
    assert_eq!(out.iter().map(|read| (read.hour, read.last, read.starts_segment)).collect::<Vec<_>>(), [(89, 112, false), (113, 136, false), (137, 160, false)]);
    assert_eq!(out[0].parts, Ok(vec![part.clone()]), "the parts written with the read in force, not those of its earlier read hour");

    let mut rows: BTreeMap<i32, CountryFuelRow> = hours.iter().map(|&hour| (hour, fuel(hour, Recorded::NotRecorded))).collect();
    let mut held = parts_at(&hours, &part, 100);
    let mut out = Vec::new();
    stretch_reads(&Walk { from: 100, to: 160, last: 120, cut: false, read: None }, &hours, &mut rows, &mut held, &BTreeSet::from([113]), &mut out);
    assert_eq!(out.iter().map(|read| (read.hour, read.last, read.starts_segment)).collect::<Vec<_>>(), [(100, 112, true), (113, 136, false)],
        "a stretch's own first read starts its segment, dated by its write where nothing names an earlier read; reads past the window are left");
    assert!(out[1].parts.is_err(), "parts not readable at a read say so");
}

#[test]
fn a_history_covers_its_window_saying_where_no_read_holds_and_cuts_a_consumers_last_read_at_the_window() {
    let key = Key::Unit(0, 0, 41, 3, 1, None);
    let reads = [read(30, 53, demand(30, vec![division(41, Some(1_000))])), read(54, 100, demand(54, vec![division(41, Some(2_000))]))];
    let runs = demand_runs(&reads, std::slice::from_ref(&key), 0, 119).remove(0);
    assert_eq!(runs.iter().map(|run| (run.first_hour, run.last_hour, run.amount_raw.as_deref(), run.reason.is_some())).collect::<Vec<_>>(),
        [(0, 29, None, true), (30, 53, Some("24000"), false), (54, 100, Some("48000"), false), (101, 119, None, true)]);
    assert!(runs[0].starts_segment && runs[3].starts_segment, "an hour no read holds breaks the line");
    let cut = demand_runs(&reads, std::slice::from_ref(&key), 40, 70).remove(0);
    assert_eq!(cut.iter().map(|run| (run.first_hour, run.last_hour)).collect::<Vec<_>>(), [(30, 53), (54, 70)], "a consumer's last read ends with the window");

    let first = recipient(11, 70_000);
    let reads = [read(30, 53, fuel(30, applied(vec![first.clone()]))), read(54, 100, fuel(54, applied(Vec::new())))];
    let runs = award_runs(&reads, &recipient_key(&first), 0, 70);
    assert_eq!(runs.iter().map(|run| (run.first_hour, run.last_hour, run.amount_raw.as_deref(), run.starts_segment)).collect::<Vec<_>>(),
        [(0, 29, None, true), (30, 53, Some("70000"), true), (54, 100, Some("0"), false)], "a receipt's last read is held whole; a receipt after a gap starts a segment");
    assert!(runs[0].reason.is_some());
}
