use crate::test_fixtures::{CAMPAIGN, records, source};
use replay::container::Replay;
use std::sync::{Arc, atomic::AtomicBool};

#[test]
fn measure_runs_compress_equal_hours_break_at_gaps_and_mark_net_drops() {
    use crate::products::country_measure::{BalanceDrop, FixedRun, Measure, scan};
    let run = |first_hour, last_hour, value: &str, starts_segment| FixedRun {
        first_hour,
        last_hour,
        value_raw: value.into(),
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
    let result = scan(
        &replay,
        1,
        Measure::PoliticalPower,
        &["GER", "ITA"],
        &AtomicBool::new(false),
    )
    .unwrap();
    let germany = &result["GER"];
    assert_eq!(
        germany.runs,
        [
            run(100, 101, "1000000", true),
            // A new value in the next hour continues the line.
            run(102, 102, "900000", false),
            // After a missing value or a missing hour the line starts again.
            run(104, 104, "500000", true),
            run(106, 106, "400000", true),
            run(107, 107, "450000", false)
        ]
    );
    assert_eq!(
        germany.drops,
        [BalanceDrop {
            hour: 102,
            before_raw: "1000000".into(),
            after_raw: "900000".into()
        }]
    );
    assert!(result["ITA"].runs.is_empty());
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
    assert_ne!(command.series[0].runs, reply.series[0].runs);
    let too_many = MeasureParams {
        branch_id: "1".into(),
        measure: Measure::PoliticalPower,
        tags: (0..17).map(|index| format!("T{index}")).collect(),
    };
    assert!(CountryMeasure::validate(&too_many).is_err());
}
