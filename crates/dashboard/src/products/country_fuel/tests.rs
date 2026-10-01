use crate::test_fixtures::{CAMPAIGN, records, source};
use replay::{
    container::Replay,
    frame::{DecodedFrame, TableView},
    rows::{CountryLinkRow, CountryRow, TableId, TableRows},
};
use std::sync::{Arc, atomic::AtomicBool};

/// A frame with Germany's fuel status (native id `germany`, linked through id 11
/// too) and Italy's, each owning a state. `read` times a walk of another table of
/// the fuel slot only, as a shared read times only its anchor. Fuel values are
/// (current, maximum, ring gain, ring consumed) raw.
fn fuel_frame(hour: i32, read: bool, germany: u32, fuel: (i64, i64, i64, i64)) -> DecodedFrame {
    use replay::rows::{CountryFuelRow, StateRow};
    let mut frame = source(1, hour, Some(1));
    let country = |id, canonical_slot, tag: &[u8]| CountryRow {
        id,
        canonical_slot,
        tag: tag.to_vec(),
        ..Default::default()
    };
    *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(vec![
        country(21, 8, b"ITA"),
        country(germany, 7, b"GER"),
    ]));
    let link = |id, canonical_slot| CountryLinkRow { id, canonical_slot };
    *frame.tables[TableId::CountryLinks.index()] =
        TableView::Complete(TableRows::CountryLinks(vec![
            link(11, 7),
            link(21, 8),
            link(germany, 7),
        ]));
    let (current_raw, maximum_raw, ring_gain_raw, ring_consumed_raw) = fuel;
    *frame.tables[TableId::CountryFuel.index()] =
        TableView::Complete(TableRows::CountryFuel(vec![
            // Germany's row names it by its other link.
            CountryFuelRow {
                country: 11,
                current_raw,
                maximum_raw,
                ring_gain_raw,
                ring_consumed_raw,
                ring_other_raw: 3,
                lend_lease_gain_raw: 7,
                ..Default::default()
            },
            CountryFuelRow {
                country: 21,
                current_raw: 1,
                maximum_raw: 2,
                ring_gain_raw: 3,
                ring_consumed_raw: 4,
                ..Default::default()
            },
        ]));
    let state = |id, owner| StateRow {
        id,
        owner,
        controller: owner,
        ..Default::default()
    };
    // Germany's state names it by its other link.
    *frame.tables[TableId::States.index()] =
        TableView::Complete(TableRows::States(vec![state(1, 21), state(2, 11)]));
    frame.metrics.table_ns[TableId::Countries.index()] = 1;
    if read {
        frame.metrics.table_ns[TableId::CountryFuelConsumers.index()] = 1;
    }
    frame
}

/// The frame with Germany's links gone: Countries lists it, but CountryLinks
/// (otherwise complete) cannot resolve it.
fn unlinked(mut frame: DecodedFrame) -> DecodedFrame {
    if let TableView::Complete(TableRows::CountryLinks(links)) =
        &mut *frame.tables[TableId::CountryLinks.index()]
    {
        links.retain(|link| link.canonical_slot != 7);
    }
    frame
}

/// The frame with Germany's state taken by Italy, and another state owned by a
/// link id the links cannot resolve.
fn stateless_with_unknown_owner(frame: DecodedFrame) -> DecodedFrame {
    use replay::rows::StateRow;
    let mut frame = stateless(frame);
    if let TableView::Complete(TableRows::States(states)) =
        &mut *frame.tables[TableId::States.index()]
    {
        states.push(StateRow {
            id: 3,
            owner: 99,
            ..Default::default()
        });
    }
    frame
}

/// The frame with Germany's state taken by Italy.
fn stateless(mut frame: DecodedFrame) -> DecodedFrame {
    use replay::rows::StateRow;
    *frame.tables[TableId::States.index()] = TableView::Complete(TableRows::States(vec![
        StateRow {
            id: 1,
            owner: 21,
            ..Default::default()
        },
        StateRow {
            id: 2,
            owner: 21,
            ..Default::default()
        },
    ]));
    frame
}

/// The frame recorded without a States table, so it cannot tell whether Germany
/// owned a state.
fn unowned_unknown(mut frame: DecodedFrame) -> DecodedFrame {
    *frame.tables[TableId::States.index()] = TableView::Unavailable {
        reason: "fixture".into(),
    };
    frame
}

/// The fuel slot's hour on day `day`.
fn fuel_slot(day: i32) -> i32 {
    let replay::cadence::Cadence::Daily(slot) = replay::cadence::cadence(TableId::CountryFuel)
    else {
        panic!("CountryFuel is read in a daily slot");
    };
    day * replay::cadence::STAGGER_PERIOD + slot as i32
}

/// What a fuel test hour holds, relative to the fuel slot: a read, a sample
/// without a state or one recorded without States.
#[derive(Clone, Copy, PartialEq)]
enum FuelHour {
    Owning,
    Stateless,
    Unknown,
}

/// A frame for `hour` (relative to `slot`) as `kind` says.
fn fuel_hour(
    slot: i32,
    hour: i32,
    read: bool,
    germany: u32,
    fuel: (i64, i64, i64, i64),
    kind: FuelHour,
) -> DecodedFrame {
    let frame = fuel_frame(slot + hour, read, germany, fuel);
    match kind {
        FuelHour::Owning => frame,
        FuelHour::Stateless => stateless(frame),
        FuelHour::Unknown => unowned_unknown(frame),
    }
}

#[test]
fn fuel_reads_carry_until_the_next_keep_whole_days_once_and_never_cross_gaps() {
    use crate::products::country_fuel::{DayLeftOut, scan};
    let slot = fuel_slot(1000);
    let full = (655_360_000, 655_360_000, 1_000_000, 65_536);
    // Every hour from -25 to 336 is recorded but 25 and 311. Timed reads
    // (relative to the fuel slot): a branch start at -25, the slot at 0, a
    // refresh at 2, then daily slot reads and an off-slot refresh at 127. The
    // slot hours 120 and 144 time no table of the slot, as recordings whose
    // shared economy read is charged to the hourly ProvinceSupply show: the
    // cadence still reads the table there.
    let reads = [
        -25, 0, 2, 24, 48, 72, 96, 127, 168, 192, 216, 240, 264, 288, 312, 336,
    ];
    let missing = [25, 311];
    let kind = |hour| match hour {
        150 | 192 | 255 => FuelHour::Stateless,
        230 | 250 => FuelHour::Unknown,
        _ => FuelHour::Owning,
    };
    let fuel = |hour| match hour {
        ..0 => (32_768, 655_360_000, 100_000, 0),
        0 => (65_536, 655_360_000, 200_000, 32_768),
        // A value seen off a read is not a read: hour 1 carries the slot's read.
        1 => (1, 1, 1, 1),
        2..24 => (98_304, 655_360_000, 300_000, 0),
        _ => full,
    };
    // Another country takes the tag from hour 49.
    let country = |hour| if hour < 49 { 9001 } else { 9002 };
    let frames = (-25..=336)
        .filter(|hour| !missing.contains(hour))
        .map(|hour| {
            fuel_hour(
                slot,
                hour,
                reads.contains(&hour),
                country(hour),
                fuel(hour),
                kind(hour),
            )
        })
        .collect();
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    let scanned = scan(&replay, 1, "GER", &AtomicBool::new(false)).unwrap();
    let reads: Vec<_> = scanned
        .reads
        .iter()
        .map(|read| {
            (
                read.first_hour - slot,
                read.last_hour - slot,
                read.starts_segment,
                read.fuel_raw.as_str(),
                read.day.is_some(),
                read.left_out,
            )
        })
        .collect();
    let full = "655360000";
    let (unknown, stale, overlaps) = (
        Some(DayLeftOut::OwnershipNotRecorded),
        Some(DayLeftOut::NotUpdated),
        Some(DayLeftOut::Overlaps),
    );
    assert_eq!(
        reads,
        [
            // The branch's first read, then the slot read a day before: their
            // days lie before the recording.
            (-25, -25, true, "32768", false, unknown),
            (-24, -1, false, "32768", false, unknown),
            (0, 1, false, "65536", true, None),
            // A refresh two hours after the slot read: its day overlaps the slot read's.
            (2, 23, false, "98304", false, overlaps),
            // Hour 25 is missing: the read carries nothing past it.
            (24, 24, false, full, true, None),
            // The gap at 25 lies inside this read's day.
            (48, 48, true, full, false, unknown),
            // The first update of this day followed a sample of the old country.
            (72, 95, true, full, false, stale),
            (96, 119, false, full, true, None),
            // A slot hour reads the table by its cadence, timed or not.
            (120, 126, false, full, true, None),
            // A refresh within a day of the slot reads: its day overlaps theirs.
            (127, 143, false, full, false, overlaps),
            (144, 167, false, full, true, None),
            // Without a state at 150 the game did not update the ring.
            (168, 191, false, full, false, stale),
            // The read's own sample (without a state) governs the next update.
            (192, 215, false, full, true, None),
            // ...which is this day's first.
            (216, 239, false, full, false, stale),
            // An hour recorded without States cannot tell.
            (240, 263, false, full, false, unknown),
            // An hour without a state outweighs one that cannot tell.
            (264, 287, false, full, false, stale),
            (288, 310, false, full, true, None),
            // A gap at 311, the last sample of this day.
            (312, 335, true, full, false, unknown),
            // The same gap lies just outside this day.
            (336, 336, false, full, true, None),
        ]
    );
    assert_eq!(
        scanned.reads[4]
            .day
            .as_ref()
            .map(|day| (day.gained_raw.as_str(), day.burned_raw.as_str())),
        Some(("1000000", "65536"))
    );
    assert_eq!(scanned.reads[4].capacity_raw, "655360000");
    // The latest hour's lend-lease fuel, which the day's gain does not include.
    assert_eq!(scanned.reads[4].lend_lease_raw, "7");
}

#[test]
fn a_fuel_day_is_governed_by_all_24_samples_before_its_read() {
    use crate::products::country_fuel::{DayLeftOut, scan};
    let slot = fuel_slot(1000);
    let full = (655_360_000, 655_360_000, 1_000_000, 65_536);
    // The day read at 24 (relative to the slot), recorded from `from` with the
    // hours in `missing` left out and those in `stateless` holding no state.
    let day_read_at_24 = |from: i32, missing: &[i32], stateless: &[i32]| {
        let frames = (from..=24)
            .filter(|hour| !missing.contains(hour))
            .map(|hour| {
                let kind = if stateless.contains(&hour) {
                    FuelHour::Stateless
                } else {
                    FuelHour::Owning
                };
                fuel_hour(slot, hour, hour == 0 || hour == 24, 9001, full, kind)
            })
            .collect();
        let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
        let scanned = scan(&replay, 1, "GER", &AtomicBool::new(false)).unwrap();
        scanned.reads.last().unwrap().left_out
    };
    assert_eq!(day_read_at_24(0, &[], &[]), None);
    // The sample 24 hours before a read (without a state) governs the first
    // update the read holds, although the country owns one again an hour later.
    assert_eq!(day_read_at_24(0, &[], &[0]), Some(DayLeftOut::NotUpdated));
    // The read's own sample (without a state) governs the next update.
    assert_eq!(day_read_at_24(0, &[], &[24]), None);
    // Nor does a sample 25 hours before it.
    assert_eq!(day_read_at_24(-1, &[], &[-1]), None);
    // A gap inside the day: the country may have lost its states then, unseen.
    assert_eq!(
        day_read_at_24(0, &[12], &[]),
        Some(DayLeftOut::OwnershipNotRecorded)
    );
    assert_eq!(
        day_read_at_24(0, &[0], &[]),
        Some(DayLeftOut::OwnershipNotRecorded)
    );
    // A gap just outside it (25 hours back) does not count.
    assert_eq!(day_read_at_24(-3, &[-1], &[]), None);
    // A sample without a state outweighs a gap.
    assert_eq!(day_read_at_24(0, &[12], &[5]), Some(DayLeftOut::NotUpdated));
    // With none of the 24 recorded (a branch's first read) there is no evidence.
    assert_eq!(
        day_read_at_24(24, &[], &[]),
        Some(DayLeftOut::OwnershipNotRecorded)
    );
}

#[test]
fn a_fuel_day_is_not_known_where_the_links_cannot_tell_who_owns_what() {
    use crate::products::country_fuel::{DayLeftOut, scan};
    let slot = fuel_slot(1000);
    let full = (655_360_000, 655_360_000, 1_000_000, 65_536);
    let day_read_at_24 = |odd: fn(DecodedFrame) -> DecodedFrame| {
        let frames = (0..=24)
            .map(|hour| {
                let frame = fuel_frame(slot + hour, hour == 0 || hour == 24, 9001, full);
                if hour == 12 { odd(frame) } else { frame }
            })
            .collect();
        let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
        let scanned = scan(&replay, 1, "GER", &AtomicBool::new(false)).unwrap();
        scanned.reads.last().unwrap().left_out
    };
    // Countries lists the tag but its links are gone: whose it is, and whether it
    // owned a state, is unknown, not "no state".
    assert_eq!(
        day_read_at_24(unlinked),
        Some(DayLeftOut::OwnershipNotRecorded)
    );
    // No state resolves to the country, but one names an owner the links cannot
    // resolve: that one could be its own.
    assert_eq!(
        day_read_at_24(stateless_with_unknown_owner),
        Some(DayLeftOut::OwnershipNotRecorded)
    );
    // Every owner resolves and none is the country's: it held no state.
    assert_eq!(day_read_at_24(stateless), Some(DayLeftOut::NotUpdated));
}

#[test]
fn fuel_replies_with_the_engine_scales_and_answers_again_from_the_cache() {
    use crate::products::{CountryParams, LoadContext, Product, country_fuel::CountryFuel};
    let slot = fuel_slot(1000);
    let replay = Arc::new(
        Replay::from_records(
            CAMPAIGN.into(),
            &[records(
                1,
                // A whole recorded day before the slot read, which starts at the
                // previous day's slot read, then a read an hour later.
                (slot - 24..=slot + 1)
                    .map(|hour| {
                        fuel_frame(hour, hour >= slot, 9001, (65_536, 98_304, 200_000, 32_768))
                    })
                    .collect(),
            )],
        )
        .unwrap(),
    );
    let source = crate::Source::from_replay(
        crate::SourceKey {
            source_id: "fuel".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        replay,
        &AtomicBool::new(false),
    )
    .unwrap();
    let load = |tag: &str| {
        CountryFuel::load(
            &source,
            &CountryParams {
                branch_id: "1".into(),
                tag: tag.into(),
            },
            &LoadContext { content: None },
            &AtomicBool::new(false),
        )
        .unwrap()
    };
    let reply = load("GER");
    assert_eq!(
        (reply.quantity_scale, reply.gain_scale, reply.day_hours),
        (32_768, 100_000, 24)
    );
    assert_eq!(reply.reads.len(), 3);
    assert!(reply.reads[0].day.is_none());
    assert!(reply.reads[1].day.is_some());
    assert!(reply.reads[2].day.is_none());
    assert!(
        reply
            .warnings
            .iter()
            .any(|warning| warning.starts_with("Fuel reads less than a day from another: 1.")),
        "{:?}",
        reply.warnings
    );
    assert_eq!(load("GER").reads, reply.reads);
    let missing = load("XXX");
    assert!(missing.reads.is_empty());
    assert!(missing.warnings[0].starts_with("XXX:"));
}
