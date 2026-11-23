use super::*;
use recorder_derive_framework::testing::deployed::{Edits, Run, counters, division, volunteer, world};
use replay_columnar::manifest::Status;

const INPUTS: &[TableId] = &[TableId::Countries, TableId::CountryLinks, TableId::Divisions, TableId::SentVolunteers, TableId::CountryCounterInputs, TableId::VolunteerHosts];
const VOL: u8 = MilitaryAidRow::VOLUNTEERS;
const EXP: u8 = MilitaryAidRow::EXPEDITIONARIES;
const TAL: u8 = MilitaryAidRow::TALLIES;

fn host(country: u32, position: u32, host: u32, divisions: i32) -> VolunteerHostRow {
    VolunteerHostRow { country, position, host, deployed_divisions: divisions, ..Default::default() }
}

fn writers() -> Vec<Box<dyn Writer>> {
    vec![Box::new(Aid::default())]
}

fn aid(run: &Run) -> Vec<(u32, u32, u8, Option<i64>)> {
    run.rows::<MilitaryAidRow>(TableId::MilitaryAid)
        .into_iter()
        .map(|row| {
            let count = match row.divisions {
                Recorded::Value(count) => Some(count),
                Recorded::NotRecorded => None,
            };
            (row.sender, row.host, row.channel, count)
        })
        .collect()
}

fn sent() -> Edits {
    let mut edits = Edits::default();
    world(&mut edits)
        .put(TableId::Divisions, division(1, 20, 20, 0, 7))
        .put(TableId::Divisions, division(2, 30, 30, 10, 7))
        .put(TableId::Divisions, division(3, 20, 20, 10, 7))
        .put(TableId::SentVolunteers, volunteer(10, 0, 1))
        .put(TableId::SentVolunteers, volunteer(10, 1, 1))
        .put(TableId::SentVolunteers, volunteer(10, 2, 0))
        .put(TableId::SentVolunteers, volunteer(10, 3, 99))
        .put(TableId::CountryCounterInputs, counters(10, Some(&[2, 3])))
        .put(TableId::CountryCounterInputs, counters(20, Some(&[])))
        .put(TableId::CountryCounterInputs, counters(30, Some(&[])));
    edits
}

#[test]
fn aid_counts_listed_live_divisions_by_host_and_channel() {
    let mut run = Run::new(writers, INPUTS);
    run.hour(&sent());
    assert_eq!(aid(&run), [(1, 2, VOL, Some(2)), (1, 2, EXP, Some(1)), (1, 3, EXP, Some(1))]);
    let mut edits = Edits::default();
    edits.delete(TableId::Divisions, 1).put(TableId::Divisions, division(2, 20, 30, 10, 7));
    run.hour(&edits);
    assert_eq!(aid(&run), [(1, 2, EXP, Some(2))], "the dead volunteer leaves; the moved expeditionary's roster owner hosts it");
    let mut edits = Edits::default();
    edits.unavailable(TableId::SentVolunteers, b"not read").delete(TableId::Divisions, 3);
    run.hour(&edits);
    assert!(matches!(run.status(TableId::MilitaryAid), Status::Unavailable { .. }));
    assert_eq!(aid(&run), [(1, 2, EXP, Some(2))]);
    let mut edits = Edits::default();
    edits.status(TableId::SentVolunteers, Status::Complete);
    run.hour(&edits);
    assert_eq!(run.status(TableId::MilitaryAid), Status::Complete);
    assert_eq!(aid(&run), [(1, 2, EXP, Some(1))]);
}

#[test]
fn a_channel_that_cannot_be_read_whole_holds_one_unknown_row() {
    let mut run = Run::new(writers, INPUTS);
    let mut edits = Edits::default();
    world(&mut edits)
        .put(TableId::Divisions, division(1, 20, 20, 0, 7))
        .put(TableId::Divisions, division(5, 30, 30, 0, 7))
        .put(TableId::SentVolunteers, volunteer(10, 0, 1))
        .put(TableId::SentVolunteers, volunteer(11, 0, 1))
        .put(TableId::SentVolunteers, volunteer(77, 0, 1))
        .put(TableId::CountryCounterInputs, counters(10, Some(&[])))
        .put(TableId::CountryCounterInputs, counters(20, Some(&[5])));
    run.hour(&edits);
    assert_eq!(aid(&run), [(1, NONE, VOL, None), (2, NONE, EXP, None), (3, NONE, EXP, None), (NONE, NONE, VOL, None)]);
    let mut edits = Edits::default();
    edits
        .delete(TableId::SentVolunteers, 11 << 32)
        .delete(TableId::SentVolunteers, 77 << 32)
        .put(TableId::CountryCounterInputs, counters(20, Some(&[])))
        .put(TableId::CountryCounterInputs, counters(30, Some(&[])));
    run.hour(&edits);
    assert_eq!(aid(&run), [(1, 2, VOL, Some(1))]);
}

#[test]
fn tallies_are_the_games_own_vector_and_unknown_by_sender_never_zero() {
    let mut run = Run::new(writers, INPUTS);
    let mut edits = Edits::default();
    world(&mut edits)
        .put(TableId::CountryCounterInputs, counters(10, Some(&[])))
        .put(TableId::CountryCounterInputs, counters(20, Some(&[])))
        .put(TableId::CountryCounterInputs, counters(30, Some(&[])))
        .put(TableId::VolunteerHosts, host(10, 0, 20, 3))
        .put(TableId::VolunteerHosts, host(10, 1, 30, 0))
        .put(TableId::VolunteerHosts, host(10, 2, 20, 9));
    run.hour(&edits);
    assert_eq!(aid(&run), [(1, 2, TAL, Some(3)), (1, 3, TAL, Some(0))], "a repeated host keeps its first count");
    let mut edits = Edits::default();
    edits.put(TableId::VolunteerHosts, host(11, 0, 30, 1));
    run.hour(&edits);
    assert_eq!(aid(&run), [(1, NONE, TAL, None)], "GER's two raw ids both hold vectors");
    let mut edits = Edits::default();
    edits.delete(TableId::VolunteerHosts, 11 << 32).put(TableId::VolunteerHosts, host(20, 0, 77, 1)).put(TableId::VolunteerHosts, host(77, 0, 20, 1));
    run.hour(&edits);
    assert_eq!(aid(&run), [(1, 2, TAL, Some(3)), (1, 3, TAL, Some(0)), (2, NONE, TAL, None), (NONE, NONE, TAL, None)]);
    let mut edits = Edits::default();
    edits.unavailable(TableId::VolunteerHosts, b"not read");
    run.hour(&edits);
    assert_eq!(run.status(TableId::MilitaryAid), Status::Complete, "the other channels stay known");
    assert_eq!(aid(&run), [(1, NONE, TAL, None), (2, NONE, TAL, None), (3, NONE, TAL, None)]);
    let mut edits = Edits::default();
    edits.status(TableId::VolunteerHosts, Status::Complete).delete(TableId::VolunteerHosts, 20 << 32).delete(TableId::VolunteerHosts, 77 << 32);
    run.hour(&edits);
    assert_eq!(aid(&run), [(1, 2, TAL, Some(3)), (1, 3, TAL, Some(0))]);
}

#[test]
fn a_resumed_aid_writer_writes_the_table_of_one_never_stopped() {
    let mut first = sent();
    first.put(TableId::VolunteerHosts, host(10, 0, 20, 2));
    let mut unbroken = Run::new(writers, INPUTS);
    unbroken.hour(&first);
    let mut stopped = Run::new(writers, INPUTS);
    stopped.hour(&first);
    let mut resumed = stopped.resumed();
    let mut edits = Edits::default();
    edits
        .put(TableId::Divisions, division(1, 30, 30, 0, 7))
        .put(TableId::CountryCounterInputs, counters(10, Some(&[3])))
        .put(TableId::VolunteerHosts, host(10, 1, 30, 4));
    unbroken.hour(&edits);
    resumed.hour(&edits);
    assert_eq!(aid(&unbroken), aid(&resumed));
    assert_eq!(aid(&resumed), [(1, 2, EXP, Some(1)), (1, 2, TAL, Some(2)), (1, 3, VOL, Some(2)), (1, 3, TAL, Some(4))]);
}
