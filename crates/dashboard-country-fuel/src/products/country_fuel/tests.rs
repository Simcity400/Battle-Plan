use dashboard_base::test_fixtures::{CAMPAIGN, records, source};
use replay::container::Replay;
use replay_data::{rows::CountryLinkRow, rows::CountryRow, rows::TableId, rows::TableRows};
use replay_frame::{frame::DecodedFrame, frame::TableView};
use std::sync::{Arc, atomic::AtomicBool};

pub(super) fn replay_source(frames: Vec<DecodedFrame>, id: &str) -> dashboard_base::source::Source {
    dashboard_base::source::Source::from_replay(
        dashboard_base::source::SourceKey {
            source_id: id.into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        Arc::new(Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap()),
        &AtomicBool::new(false),
    )
    .unwrap()
}

/// A read as its reply shows it.
pub(super) fn reply_read(
    row: &replay_data::rows::CountryFuelRow,
    hour: i32,
    left_out: Option<super::DayLeftOut>,
) -> super::FuelRead {
    let read = super::read_of(row, hour, true, left_out, &mut Default::default());
    super::FuelRead::from(&read)
}

#[test]
fn day_keeps_receipts_and_other_in_their_recorded_native_units() {
    let row = replay_data::rows::CountryFuelRow {
        ring_gain_raw: 17,
        ring_consumed_raw: 32769,
        ring_received_0_raw: 50001,
        ring_received_1_raw: 25000,
        ring_received_2_raw: 24999,
        ring_other_raw: 1,
        ..Default::default()
    };
    let day = reply_read(&row, 1000, None).day.unwrap();
    assert_eq!(
        day.received_raw,
        ["50001", "25000", "24999"].map(str::to_owned)
    );
    assert_eq!(day.other_raw, "1");
    assert_eq!(day.burned_raw, "32769");
    assert!(
        reply_read(&row, 1000, Some(super::DayLeftOut::Overlaps))
            .day
            .is_none()
    );
}

/// A frame with Germany's fuel status (native id `germany`, linked through id 11
/// too) and Italy's, each owning a state. `read` times a walk of another table of
/// the fuel slot only, as a shared read times only its anchor. Fuel values are
/// (current, maximum, ring gain, ring consumed) raw.
pub(super) fn fuel_frame(
    hour: i32,
    read: bool,
    germany: u32,
    fuel: (i64, i64, i64, i64),
) -> DecodedFrame {
    use replay_data::{rows::CountryFuelRow, rows::StateRow};
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
    use replay_data::rows::StateRow;
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
    use replay_data::rows::StateRow;
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
pub(super) fn fuel_slot(day: i32) -> i32 {
    let replay_data::cadence::Cadence::Daily(slot) =
        replay_data::cadence::cadence(TableId::CountryFuel)
    else {
        panic!("CountryFuel is read in a daily slot");
    };
    day * replay_data::cadence::STAGGER_PERIOD + slot as i32
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
        scanned[4]
            .day
            .as_ref()
            .map(|day| (day.gained_raw.as_str(), day.burned_raw.as_str())),
        Some(("1000000", "65536"))
    );
    assert_eq!(scanned[4].capacity_raw, "655360000");
    // The latest hour's lend-lease fuel, which the day's gain does not include.
    assert_eq!(scanned[4].lend_lease_raw, "7");
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
        scanned.last().unwrap().left_out
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
        scanned.last().unwrap().left_out
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
    use crate::products::country_fuel::CountryFuel;
    use dashboard_base::products::{CountryParams, Product};
    use dashboard_core::config::LoadContext;
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
    let source = dashboard_base::source::Source::from_replay(
        dashboard_base::source::SourceKey {
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

#[test]
fn a_new_tag_has_the_same_absent_sample_evidence_as_a_single_country_scan() {
    let slot = fuel_slot(1000);
    let frames = (0..=24)
        .map(|hour| {
            let mut frame = fuel_frame(slot + hour, hour == 0 || hour == 24, 9001, (1, 2, 3, 4));
            if hour < 12
                && let TableView::Complete(TableRows::Countries(rows)) =
                    &mut *frame.tables[TableId::Countries.index()]
            {
                rows.iter_mut().find(|row| row.tag == b"GER").unwrap().tag = b"OLD".to_vec();
            }
            frame
        })
        .collect();
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    let scanned = super::scan(&replay, 1, "GER", &AtomicBool::new(false)).unwrap();
    assert_eq!(
        scanned.last().unwrap().left_out,
        Some(super::DayLeftOut::NotUpdated)
    );
}

/// The day of 24 applied updates through `hour`, each gaining [1, -2, 3, 0]
/// (own gain 2), transferring [1, -3, 4, 1], consuming 1 and paying one
/// recipient 1, whose retained name reads "before" at even hours and "after"
/// at odd ones: two recipients of 12 receipts each.
pub(super) fn award_row(hour: i32) -> replay_data::rows::CountryFuelRow {
    use replay_data::{
        rows::CountryFuelRow, rows::FuelAwardDay, rows::FuelAwardDayRecipient, rows::Recorded,
    };
    let recipient = |name: &[u8], first_hour: i32| FuelAwardDayRecipient {
        kind: 0,
        reference: 7 << 32 | 29,
        owner: 37,
        name: Some(name.to_vec()),
        first_hour,
        last_hour: first_hour + 22,
        amount_raw: -12,
        ..Default::default()
    };
    let (odd, even) = if hour % 2 == 0 {
        (hour - 23, hour - 22)
    } else {
        (hour - 22, hour - 23)
    };
    CountryFuelRow {
        country: 11,
        country_instance: Recorded::Value(Some(9)),
        ring_gain_raw: 999,
        award_day: Recorded::Value(FuelAwardDay {
            country_instance: 9,
            continuity_start_hour: Some(977),
            last_hour: hour,
            gains_raw: [24, -48, 72, 0],
            own_gain_raw: 48,
            own_gain_quantity_raw: 24 * engine_fuel::fuel_awards::quantity(2),
            transfers_raw: [24, -72, 96, 24],
            consumed_raw: -24,
            received_raw: [-24, 0, 0],
            recipients: vec![recipient(b"after", odd), recipient(b"before", even)],
            reason: Vec::new(),
        }),
        ..Default::default()
    }
}

#[test]
fn completed_award_totals_keep_i64_extremes_and_the_recorded_conversion_exact() {
    let mut row = award_row(1000);
    let replay_data::rows::Recorded::Value(recorded) = &mut row.award_day else {
        panic!()
    };
    recorded.own_gain_raw = i64::MAX;
    recorded.gains_raw = [i64::MAX, i64::MIN, 0, 0];
    recorded.consumed_raw = i64::MIN;
    recorded.own_gain_quantity_raw = i64::MIN + 1;
    let day = reply_read(&row, 1000, None).day.unwrap();
    let totals = day.awards.unwrap();
    assert_eq!(day.gained_raw, i64::MAX.to_string());
    assert_eq!(day.burned_raw, i64::MIN.to_string());
    assert_eq!(totals.gains_raw[0], day.gained_raw);
    assert_eq!(totals.gains_raw[1], day.burned_raw);
    // The sum of each update's conversion, which no day sum reproduces.
    assert_eq!(totals.own_gain_quantity_raw, (i64::MIN + 1).to_string());
}

#[test]
fn native_fuel_label_templates_expand_content_and_preserve_unrecorded_oil_operand() {
    let localisation = dashboard_text::catalogue::tests::loaded(
        "l_english:\n FUEL_DAILY_GAIN_BASE:0 \"$OIL$: $NUM$\"\n OIL:0 \"Content oil label\"\n MOD_BASE:0 \"Synthetic source\"\n FUEL_DAILY_GAIN_FROM_OIL:0 \"$MOD_BASE$: From $OIL$ oil: $NUM|H$\"\n",
    );
    let base = super::fuel_label(Some(&localisation), "FUEL_DAILY_GAIN_BASE", false);
    assert_eq!(base.template.as_deref(), Some("Content oil label: {NUM}"));
    assert_eq!(base.arguments, ["NUM"]);
    assert!(base.reason.is_none());
    let oil = super::fuel_label(Some(&localisation), "FUEL_DAILY_GAIN_FROM_OIL", true);
    assert_eq!(
        oil.template.as_deref(),
        Some("Synthetic source: From {OIL} oil: {NUM}")
    );
    assert_eq!(oil.arguments, ["NUM", "OIL"]);
    assert!(oil.reason.unwrap().contains("Historical OIL count"));
    let missing = super::fuel_label(Some(&localisation), "FUEL_DAILY_GAIN_FROM_STATES", false);
    assert!(missing.template.is_none());
    assert!(
        missing
            .reason
            .unwrap()
            .contains("FUEL_DAILY_GAIN_FROM_STATES")
    );
}

#[test]
fn applied_updates_override_prefilled_ring_and_stale_ownership_without_inventing_wings() {
    let row = award_row(1000);
    let read = reply_read(&row, 1000, Some(super::DayLeftOut::OwnershipNotRecorded));
    assert_eq!(read.left_out, None);
    let day = read.day.unwrap();
    assert_eq!(
        (day.gained_raw.as_str(), day.burned_raw.as_str()),
        ("48", "-24")
    );
    let totals = day.awards.unwrap();
    assert_eq!(
        totals.gains_raw,
        ["24", "-48", "72", "0"].map(str::to_owned)
    );
    assert_eq!(totals.own_gain_quantity_raw, "0");
    assert_eq!(
        totals.transfers_raw,
        ["24", "-72", "96", "24"].map(str::to_owned)
    );
    let recipients = super::recipients_of(
        &row,
        &AtomicBool::new(false),
        dashboard_text::catalogue::display_text,
    )
    .unwrap();
    assert_eq!(recipients.len(), 2);
    assert!(
        recipients
            .iter()
            .all(|part| part.amount_raw == "-12" && part.owner == "37")
    );
    assert_eq!(
        recipients
            .iter()
            .filter_map(|part| part.recorded_name.as_deref())
            .collect::<Vec<_>>(),
        ["after", "before"]
    );
    assert!(
        super::recipients_of(
            &row,
            &AtomicBool::new(true),
            dashboard_text::catalogue::display_text
        )
        .is_err()
    );
    let mut row = row;
    let replay_data::rows::Recorded::Value(recorded) = &mut row.award_day else {
        panic!()
    };
    recorded.recipients[0].kind = 1;
    recorded.recipients[0].name = None;
    let recipients = super::recipients_of(
        &row,
        &AtomicBool::new(false),
        dashboard_text::catalogue::display_text,
    )
    .unwrap();
    let air = recipients.iter().find(|part| part.kind == 1).unwrap();
    assert!(air.recorded_name.is_none());
    assert!(
        air.name_reason
            .as_ref()
            .unwrap()
            .contains("no wing receipt")
    );
}

#[test]
fn attribution_requires_the_supplying_hour_and_native_country_identity() {
    use replay_data::rows::Recorded;
    let mut row = award_row(1000);
    assert!(super::awards_of(&row, 1001).is_err());
    row.country_instance = Recorded::Value(Some(10));
    assert!(super::awards_of(&row, 1000).is_ok());
    row.country_instance = Recorded::Value(Some(9));
    assert!(super::awards_of(&row, 1000).is_ok());
    let Recorded::Value(day) = &mut row.award_day else {
        panic!()
    };
    // Observation began after the day's first update.
    day.continuity_start_hour = Some(978);
    assert!(super::awards_of(&row, 1000).is_err());
}

#[test]
fn cached_receipt_names_obey_viewer_language_without_losing_numeric_receipts() {
    let row = award_row(1000);
    let mut localisation = installed_content_localisation::localisation::Localisation::default();
    localisation.language = "french".into();
    let rows = super::recipients_of(&row, &AtomicBool::new(false), |bytes| {
        dashboard_text::scripted_text::stored_text(
            &localisation,
            "english",
            bytes,
            "fuel recipient cached name",
        )
    })
    .unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|row| {
        row.amount_raw == "-12"
            && row.recorded_name.is_none()
            && row
                .name_reason
                .as_ref()
                .unwrap()
                .contains("viewer's language is french")
    }));
    localisation.language = "english".into();
    let rows = super::recipients_of(&row, &AtomicBool::new(false), |bytes| {
        dashboard_text::scripted_text::stored_text(
            &localisation,
            "english",
            bytes,
            "fuel recipient cached name",
        )
    })
    .unwrap();
    assert!(rows.iter().all(|row| row.recorded_name.is_some()));
    const { assert!(<super::CountryFuelAwards as dashboard_base::products::Product>::CONTENT_DEPENDENT) };
}

#[test]
fn award_product_refuses_carried_dates_replacements_and_cancellation() {
    use super::{CountryFuelAwards, FuelAwardsParams};
    use dashboard_base::products::Product;
    use dashboard_core::config::LoadContext;
    let slot = fuel_slot(1000);
    let frames = (slot..=slot + 1)
        .map(|hour| {
            let mut frame = fuel_frame(hour, hour == slot, 9001, (1, 2, 3, 4));
            let TableView::Complete(TableRows::CountryFuel(rows)) =
                &mut *frame.tables[TableId::CountryFuel.index()]
            else {
                panic!()
            };
            rows[0] = award_row(slot);
            frame
        })
        .collect();
    let replay = Arc::new(Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap());
    let source = dashboard_base::source::Source::from_replay(
        dashboard_base::source::SourceKey {
            source_id: "fuel-awards".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        replay,
        &AtomicBool::new(false),
    )
    .unwrap();
    let mut params = FuelAwardsParams {
        branch_id: "1".into(),
        tag: "GER".into(),
        hour: slot,
        country: "11".into(),
        country_instance: 9,
    };
    let load = |params: &FuelAwardsParams, cancelled| {
        CountryFuelAwards::load(
            &source,
            params,
            &LoadContext { content: None },
            &AtomicBool::new(cancelled),
        )
    };
    let without_content = load(&params, false).unwrap();
    assert_eq!(without_content.awards.recipients.len(), 2);
    assert!(
        without_content
            .awards
            .recipients
            .iter()
            .all(|row| row.amount_raw == "-12" && row.recorded_name.is_none())
    );
    assert!(
        without_content
            .warnings
            .iter()
            .any(|warning| warning.starts_with("2 fuel recipient names refused:"))
    );
    assert!(load(&params, true).is_err());
    params.hour += 1;
    assert!(load(&params, false).is_err());
    params.hour = slot;
    params.country_instance += 1;
    assert!(load(&params, false).is_err());
}

#[test]
fn shared_observer_resumes_twice_and_append_revises_only_overlapping_days() {
    let slot = fuel_slot(1000);
    // The earlier daily slot lacks its preceding day, while the refresh one
    // hour before the next slot has a complete governing day.
    let frames: Vec<_> = (slot - 25..=slot)
        .map(|hour| fuel_frame(hour, hour == slot - 1, 9001, (11, 19, 23, 29)))
        .collect();
    let cancelled = AtomicBool::new(false);
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames.clone())]).unwrap();
    let mut scan = super::Scan::default();
    for (index, frame) in frames.iter().enumerate().take(frames.len() - 1) {
        scan.observe(frame, replay.format, &cancelled).unwrap();
        if index == 9 || index == 19 {
            scan = serde_json::from_slice(&serde_json::to_vec(&scan).unwrap()).unwrap();
        }
    }
    let checkpoint = serde_json::to_vec(&scan).unwrap();
    let prefix = scan.finish(replay.format);
    assert_eq!(
        prefix.countries["GER"].reads.last().unwrap().first_hour,
        slot - 1
    );
    assert!(prefix.countries["GER"].reads.last().unwrap().day.is_some());
    let mut resumed: super::Scan = serde_json::from_slice(&checkpoint).unwrap();
    resumed
        .observe(frames.last().unwrap(), replay.format, &cancelled)
        .unwrap();
    let appended = resumed.finish(replay.format);
    let expected = super::scan_branch(&replay, 1, &cancelled).unwrap();
    for tag in expected.countries.keys() {
        let actual = &appended.countries[tag];
        let expected = &expected.countries[tag];
        assert_eq!(
            actual
                .reads
                .iter()
                .map(super::FuelRead::from)
                .collect::<Vec<_>>(),
            expected
                .reads
                .iter()
                .map(super::FuelRead::from)
                .collect::<Vec<_>>()
        );
        assert_eq!(
            serde_json::to_value(
                actual
                    .consumption
                    .iter()
                    .map(super::demand::FuelConsumptionRead::from)
                    .collect::<Vec<_>>()
            )
            .unwrap(),
            serde_json::to_value(
                expected
                    .consumption
                    .iter()
                    .map(super::demand::FuelConsumptionRead::from)
                    .collect::<Vec<_>>()
            )
            .unwrap()
        );
    }
    let refresh = appended.countries["GER"]
        .reads
        .iter()
        .find(|read| read.first_hour == slot - 1)
        .unwrap();
    assert_eq!(refresh.left_out, Some(super::DayLeftOut::Overlaps));
    assert!(refresh.day.is_none());
    assert_eq!(appended.decoded_frames, frames.len());
    assert_eq!(appended.ownership_rows, frames.len() * 2);

    let frames: Vec<_> = (slot..=slot + 72)
        .filter(|hour| *hour != slot + 37)
        .map(|hour| {
            let mut frame = fuel_frame(
                hour,
                hour == slot + 27,
                if hour < slot + 49 { 9001 } else { 9002 },
                (31, 37, 41, 43),
            );
            if hour >= slot + 11
                && hour < slot + 49
                && let TableView::Complete(TableRows::Countries(rows)) =
                    &mut *frame.tables[TableId::Countries.index()]
            {
                rows.iter_mut().find(|row| row.tag == b"GER").unwrap().tag = b"RENAMED".to_vec();
            }
            frame
        })
        .collect();
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames.clone())]).unwrap();
    let mut scan = super::Scan::default();
    for (index, frame) in frames.iter().enumerate() {
        scan.observe(frame, replay.format, &cancelled).unwrap();
        if index == 24 || index == 48 {
            scan = serde_json::from_slice(&serde_json::to_vec(&scan).unwrap()).unwrap();
        }
    }
    let actual = scan.finish(replay.format);
    let expected = super::scan_branch(&replay, 1, &cancelled).unwrap();
    assert_eq!(actual.actor_aliases, expected.actor_aliases);
    for tag in expected.countries.keys() {
        assert_eq!(
            actual.countries[tag]
                .reads
                .iter()
                .map(super::FuelRead::from)
                .collect::<Vec<_>>(),
            expected.countries[tag]
                .reads
                .iter()
                .map(super::FuelRead::from)
                .collect::<Vec<_>>()
        );
    }
}
