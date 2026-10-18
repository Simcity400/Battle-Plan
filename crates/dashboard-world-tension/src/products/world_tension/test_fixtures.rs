//! World tension frames for tests here and in the crates above.

use dashboard_base::test_fixtures::world_frame;
use replay_data::{rows::TableId, rows::TableRows};
use replay_frame::{frame::DecodedFrame, frame::TableView};

/// The first hour of day `day`. The tension tables are read every hour, so
/// every recorded hour holding them is a read; a quiet hour publishes the held
/// read, which the fixtures repeat.
pub fn tension_slot(day: i32) -> i32 {
    assert_eq!(
        replay_data::cadence::cadence(TableId::WorldTension),
        replay_data::cadence::Cadence::Hourly
    );
    day * replay_data::cadence::STAGGER_PERIOD
}

/// A threat source: (country link, date, reason, current, final).
pub type Threat = (u32, i32, &'static [u8], i64, i64);

/// A world frame holding the tension tables: `aggregates` by slot, `sources` as
/// listed, `total` the world tension; the frame times their walk.
pub fn tension_frame(
    hour: i32,
    austria: bool,
    total: i64,
    aggregates: &[(u32, i64)],
    sources: &[Threat],
) -> DecodedFrame {
    use replay_data::{
        rows::ThreatCountryAggregateRow, rows::ThreatSourceRow, rows::WorldTensionRow,
    };
    let mut frame = world_frame(hour, austria);
    *frame.tables[TableId::WorldTension.index()] =
        TableView::Complete(TableRows::WorldTension(vec![WorldTensionRow {
            total_raw: total,
            source_count: sources.len() as i32,
            country_aggregate_count: 10,
        }]));
    *frame.tables[TableId::ThreatSources.index()] = TableView::Complete(TableRows::ThreatSources(
        sources
            .iter()
            .enumerate()
            .map(
                |(ordinal, &(country, date, reason, current_raw, final_raw))| ThreatSourceRow {
                    ordinal: ordinal as u32,
                    country,
                    second_country: 0,
                    current_raw,
                    final_raw,
                    daily_raw: 0,
                    date,
                    reason: reason.to_vec(),
                },
            )
            .collect(),
    ));
    // Every slot of the census is listed, most at zero.
    *frame.tables[TableId::ThreatCountryAggregates.index()] =
        TableView::Complete(TableRows::ThreatCountryAggregates(
            (0..10)
                .map(|slot| ThreatCountryAggregateRow {
                    country_slot: slot,
                    total_raw: aggregates
                        .iter()
                        .find(|(at, _)| *at == slot)
                        .map_or(0, |(_, raw)| *raw),
                })
                .collect(),
        ));
    frame.metrics.table_ns[TableId::WorldTension.index()] = 1;
    frame
}
