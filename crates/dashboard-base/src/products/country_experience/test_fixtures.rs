//! Country experience frames for tests here and in the crates above.

use crate::test_fixtures::source;
use replay::{
    frame::{DecodedFrame, TableView},
    rows::{CountryLinkRow, CountryRow, TableId, TableRows},
};

/// Experience fixture with an opaque tag and an alternate raw country link.
/// Test hours are relative to a day's CountryExperience slot hour, so hours 0,
/// 24, 48, … are the table's cadence reads; `read` adds a timed (full-hour) read.
pub fn slot_base() -> i32 {
    let replay::cadence::Cadence::Daily(slot) =
        replay::cadence::cadence(TableId::CountryExperience)
    else {
        panic!("CountryExperience is read in a daily slot");
    };
    1000 * replay::cadence::STAGGER_PERIOD + slot as i32
}

pub fn experience_frame(hour: i32, read: bool, id: u32) -> DecodedFrame {
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

pub fn renamed_frame(hour: i32) -> DecodedFrame {
    use replay::rows::CountryExperienceGainRow;
    let mut frame = experience_frame(hour, false, 9001);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *frame.tables[TableId::Countries.index()]
    {
        rows[0].tag = if hour < 12 {
            b"opaque!tag".to_vec()
        } else if hour < 49 {
            b"renamed".to_vec()
        } else {
            b"third".to_vec()
        };
        for (from, tag, id, slot) in [(13, "opaque!tag", 9002, 8), (50, "renamed", 9003, 9)] {
            if hour >= from {
                rows.push(CountryRow {
                    id,
                    canonical_slot: slot,
                    tag: tag.as_bytes().to_vec(),
                    ..Default::default()
                });
            }
        }
    }
    if let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *frame.tables[TableId::CountryLinks.index()]
    {
        rows.extend([
            CountryLinkRow {
                id: 9002,
                canonical_slot: 8,
            },
            CountryLinkRow {
                id: 9003,
                canonical_slot: 9,
            },
        ]);
    }
    if let TableView::Complete(TableRows::CountryExperience(rows)) =
        &mut *frame.tables[TableId::CountryExperience.index()]
    {
        for (from, id) in [(13, 9002), (50, 9003)] {
            if hour >= from {
                rows.push(replay::rows::CountryExperienceRow {
                    country: id,
                    army_raw: 999 * 32_768,
                    ..Default::default()
                });
            }
        }
    }
    *frame.tables[TableId::CountryExperienceGains.index()] =
        TableView::Complete(TableRows::CountryExperienceGains(vec![
            CountryExperienceGainRow {
                country: 11,
                instance: 77,
                complete: u8::from(hour > 0),
                army_raw: 1,
                ..Default::default()
            },
        ]));
    frame
}
