use crate::test_fixtures::{CAMPAIGN, records, source};
use replay::campaign::Replay;
use std::sync::{Arc, atomic::AtomicBool};

#[test]
fn measure_runs_compress_equal_hours_break_at_gaps_and_mark_net_drops() {
    use crate::products::{
        country_history::{NumericBalanceDrop, NumericRun, scan_histories},
        country_measure::Measure,
    };
    let run = |first_hour, last_hour, value, starts_segment| NumericRun {
        first_hour,
        last_hour,
        value,
        starts_segment,
    };
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            vec![
                source(1, 100, Some(1_000_000)),
                source(1, 101, Some(1_000_000)),
                source(1, 102, Some(900_000)),
                // A missing value is not zero and ends the run.
                source(1, 103, None),
                source(1, 104, Some(500_000)),
                // Hour 105 is missing entirely: no drop is counted across it.
                source(1, 106, Some(400_000)),
                source(1, 107, Some(450_000)),
            ],
        )],
    )
    .unwrap();
    let result = scan_histories(
        &replay,
        1,
        false,
        &[Measure::PoliticalPower],
        &AtomicBool::new(false),
    )
    .unwrap();
    let germany = &result.countries["GER"].political_power;
    assert_eq!(
        germany.runs,
        [
            run(100, 101, 1_000_000, true),
            // A new value in the next hour continues the line.
            run(102, 102, 900_000, false),
            // After a missing value or a missing hour the line starts again.
            run(104, 104, 500_000, true),
            run(106, 106, 400_000, true),
            run(107, 107, 450_000, false)
        ]
    );
    assert_eq!(
        germany.drops,
        [NumericBalanceDrop {
            hour: 102,
            before: 1_000_000,
            after: 900_000,
        }]
    );
    assert!(!result.countries.contains_key("ITA"));
}

#[test]
fn measures_reply_every_requested_country_and_warn_for_unrecorded_ones() {
    use crate::products::{
        LoadContext, Product,
        country_measure::{CountryMeasure, Measure, MeasureParams},
    };
    let replay = Arc::new(
        Replay::from_records(
            CAMPAIGN.into(),
            &[records(
                1,
                vec![source(1, 100, Some(1)), source(1, 101, Some(2))],
            )],
        )
        .unwrap(),
    );
    let source = crate::Source::from_replay(
        crate::SourceKey {
            source_id: "pp".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        replay,
        &AtomicBool::new(false),
    )
    .unwrap();
    let params = MeasureParams {
        branch_id: "1".into(),
        measure: Measure::PoliticalPower,
        tags: vec!["GER".into(), "XXX".into()],
    };
    let reply = CountryMeasure::load(
        &source,
        &params,
        &LoadContext { content: None },
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(reply.decimals, 5);
    assert_eq!(
        reply
            .series
            .iter()
            .map(|series| series.tag.as_str())
            .collect::<Vec<_>>(),
        ["GER", "XXX"]
    );
    assert_eq!(reply.series[0].runs.len(), 2);
    assert!(
        reply
            .warnings
            .iter()
            .any(|warning| warning.starts_with("XXX:"))
    );
    // A second request is answered from the cache.
    let again = CountryMeasure::load(
        &source,
        &params,
        &LoadContext { content: None },
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(again.series, reply.series);
    // Command power comes from its own field and is cached apart from political power.
    let command = CountryMeasure::load(
        &source,
        &MeasureParams {
            measure: Measure::CommandPower,
            ..params.clone()
        },
        &LoadContext { content: None },
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(command.measure, Measure::CommandPower);
    assert_eq!(command.series[0].runs[0].value_raw, i64::MAX.to_string());
    assert_ne!(command.series[0].runs, reply.series[0].runs);
}

#[test]
fn measure_oversized_complete_reply_is_refused_by_name() {
    use super::{FixedRun, Measure, MeasureReply, MeasureSeries, checked_reply};
    use crate::products::MAX_PRODUCT_REPLY_BYTES;

    let reply = |runs: usize| MeasureReply {
        branch_id: "1".into(),
        measure: Measure::PoliticalPower,
        decimals: 5,
        series: vec![MeasureSeries {
            tag: "opaque tag".into(),
            runs: (0..runs)
                .map(|hour| FixedRun {
                    first_hour: hour as i32,
                    last_hour: hour as i32,
                    value_raw: (hour % 2).to_string(),
                    starts_segment: hour == 0,
                })
                .collect(),
            drops: vec![],
        }],
        warnings: vec![],
    };
    assert_eq!(checked_reply(reply(2)).unwrap().series[0].runs.len(), 2);
    let error = checked_reply(reply(MAX_PRODUCT_REPLY_BYTES / 64)).unwrap_err();
    assert!(error.to_string().starts_with("MeasureReplyTooLarge:"));
}
