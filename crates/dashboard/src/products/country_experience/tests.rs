use crate::test_fixtures::{CAMPAIGN, records, source};
use replay::{
    container::Replay,
    frame::{DecodedFrame, TableView},
    rows::{CountryLinkRow, CountryRow, TableId, TableRows},
};
use std::sync::atomic::AtomicBool;

/// Experience fixture with an opaque tag and an alternate raw country link.
/// Test hours are relative to a day's CountryExperience slot hour, so hours 0,
/// 24, 48, … are the table's cadence reads; `read` adds a timed (full-hour) read.
fn slot_base() -> i32 {
    let replay::cadence::Cadence::Daily(slot) =
        replay::cadence::cadence(TableId::CountryExperience)
    else {
        panic!("CountryExperience is read in a daily slot");
    };
    1000 * replay::cadence::STAGGER_PERIOD + slot as i32
}

fn experience_frame(hour: i32, read: bool, id: u32) -> DecodedFrame {
    use replay::rows::CountryExperienceRow;
    let mut frame = source(1, slot_base() + hour, Some(1));
    *frame.tables[TableId::Countries.index()] =
        TableView::Complete(TableRows::Countries(vec![CountryRow {
            id,
            canonical_slot: 7,
            tag: b"opaque!tag".to_vec(),
            ..Default::default()
        }]));
    *frame.tables[TableId::CountryLinks.index()] =
        TableView::Complete(TableRows::CountryLinks(vec![
            CountryLinkRow {
                id: 11,
                canonical_slot: 7,
            },
            CountryLinkRow {
                id,
                canonical_slot: 7,
            },
        ]));
    *frame.tables[TableId::CountryExperience.index()] =
        TableView::Complete(TableRows::CountryExperience(vec![CountryExperienceRow {
            country: 11,
            army_raw: i64::from(hour) * 32_768,
            navy_raw: -i64::from(hour),
            air_raw: 0,
            // These partial-day accumulators must never supply the change.
            army_daily_raw: i64::MAX,
            navy_daily_training_raw: i64::MIN,
            air_daily_raw: 999,
            ..Default::default()
        }]));
    if read {
        // A shared slot walk need not time CountryExperience itself.
        let anchor = TableId::ALL
            .into_iter()
            .find(|table| {
                *table != TableId::CountryExperience
                    && replay::cadence::cadence(*table)
                        == replay::cadence::cadence(TableId::CountryExperience)
            })
            .unwrap();
        frame.metrics.table_ns[anchor.index()] = 1;
    }
    frame
}

#[test]
fn experience_reads_hold_and_change_requires_a_read_exactly_one_day_earlier() {
    use crate::products::country_experience::scan;
    let read_hours = [-3, 0, 7, 21, 24, 31, 48, 52];
    let frames = (-3..=53)
        .map(|hour| experience_frame(hour, read_hours.contains(&hour), 9001))
        .collect();
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    let scanned = scan(
        &replay,
        1,
        &["opaque!tag", "missing"],
        &AtomicBool::new(false),
    )
    .unwrap();
    let reads = &scanned["opaque!tag"].reads;
    assert!(scanned["missing"].reads.is_empty());
    assert_eq!(
        reads
            .iter()
            .map(|read| (
                read.first_hour - slot_base(),
                read.last_hour - slot_base(),
                read.starts_segment,
                read.change.is_some()
            ))
            .collect::<Vec<_>>(),
        [
            (-3, -1, true, false),
            (0, 6, false, false),
            (7, 20, false, false),
            (21, 23, false, true),
            (24, 30, false, true),
            (31, 47, false, true),
            (48, 51, false, true),
            (52, 53, false, false)
        ]
    );
    assert_eq!(reads[2].army_raw, (7 * 32_768).to_string());
    for read in reads.iter().filter(|read| read.change.is_some()) {
        let change = read.change.as_ref().unwrap();
        assert_eq!(change.army_raw, (24 * 32_768).to_string());
        assert_eq!(change.navy_raw, "-24");
        assert_eq!(change.air_raw, "0");
        assert!(read.change_unknown.is_none());
    }
    assert!(reads[7].change_unknown.is_some());
}

#[test]
fn experience_gaps_and_tag_holder_changes_break_carry_and_the_next_change() {
    use crate::products::country_experience::scan;
    let frames = (0..=97)
        .filter(|hour| *hour != 12)
        .map(|hour| experience_frame(hour, hour % 24 == 0, if hour < 49 { 9001 } else { 9002 }))
        .collect();
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    let scanned = scan(&replay, 1, &["opaque!tag"], &AtomicBool::new(false)).unwrap();
    let reads = &scanned["opaque!tag"].reads;
    assert_eq!(
        reads
            .iter()
            .map(|read| (
                read.first_hour - slot_base(),
                read.last_hour - slot_base(),
                read.starts_segment,
                read.change.is_some()
            ))
            .collect::<Vec<_>>(),
        [
            (0, 11, true, false),
            (24, 47, true, false),
            (48, 48, false, true),
            (72, 95, true, false),
            (96, 97, false, true)
        ]
    );
}

#[test]
fn experience_incomplete_tables_unresolved_links_and_missing_rows_are_not_zero() {
    use crate::products::country_experience::scan;
    for failure in 0..4 {
        let frames = (0..=49)
            .map(|hour| {
                let mut frame = experience_frame(hour, hour % 24 == 0, 9001);
                if hour == 12 {
                    let table = match failure {
                        0 => TableId::CountryExperience,
                        1 => TableId::Countries,
                        _ => TableId::CountryLinks,
                    };
                    *frame.tables[table.index()] = if failure == 3 {
                        TableView::Complete(TableRows::CountryLinks(Vec::new()))
                    } else {
                        TableView::Unavailable {
                            reason: "fixture".into(),
                        }
                    };
                }
                if hour == 24 {
                    *frame.tables[TableId::CountryExperience.index()] =
                        TableView::Complete(TableRows::CountryExperience(Vec::new()));
                }
                frame
            })
            .collect();
        let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
        let scanned = scan(&replay, 1, &["opaque!tag"], &AtomicBool::new(false)).unwrap();
        let reads = &scanned["opaque!tag"].reads;
        assert_eq!(
            reads
                .iter()
                .map(|read| (
                    read.first_hour - slot_base(),
                    read.last_hour - slot_base(),
                    read.starts_segment
                ))
                .collect::<Vec<_>>(),
            [(0, 11, true), (48, 49, true)]
        );
        assert!(reads[1].change.is_none());
    }
}

#[test]
fn experience_change_widens_signed_raws_before_subtracting_and_requests_share_limits() {
    use crate::products::{
        Product,
        country_experience::{CountryExperience, ExperienceParams, scan},
    };
    let frames = (0..=24)
        .map(|hour| {
            let mut frame = experience_frame(hour, hour % 24 == 0, 9001);
            if let TableView::Complete(TableRows::CountryExperience(rows)) =
                &mut *frame.tables[TableId::CountryExperience.index()]
            {
                rows[0].army_raw = if hour == 0 { i64::MIN } else { i64::MAX };
            }
            frame
        })
        .collect();
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    let scanned = scan(&replay, 1, &["opaque!tag"], &AtomicBool::new(false)).unwrap();
    assert_eq!(
        scanned["opaque!tag"].reads[1]
            .change
            .as_ref()
            .unwrap()
            .army_raw,
        "18446744073709551615"
    );
    assert!(scan(&replay, 1, &["opaque!tag"], &AtomicBool::new(true)).is_err());
    let params = |branch: &str, tags| ExperienceParams {
        branch_id: branch.into(),
        tags,
    };
    assert!(CountryExperience::validate(&params("1", vec!["opaque!tag".into()])).is_ok());
    assert!(CountryExperience::validate(&params("1", vec![])).is_err());
    assert!(CountryExperience::validate(&params("bad", vec!["x".into()])).is_err());
    assert!(CountryExperience::validate(&params("1", vec!["x\0".into()])).is_err());
}
