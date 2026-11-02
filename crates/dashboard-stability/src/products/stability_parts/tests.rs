use dashboard_base::test_fixtures::{CAMPAIGN, records, stability_frame, stability_inputs};
use replay::container::Replay;
use replay_data::{rows::TableId, rows::TableRows};
use replay_frame::{frame::DecodedFrame, frame::TableView};

#[test]
fn appearance_projection_visits_only_changed_naming_rows() {
    use super::*;
    use dashboard_base::prepared::frame_changes::field_mask;
    use replay_data::{rows::CountryAppearanceRow, rows::DeltaRow};
    let mut scan = Scan {
        appearances_read: true,
        appearances_complete: true,
        appearances: (0..200)
            .map(|country| {
                (
                    country,
                    CountryAppearanceRow {
                        country,
                        cosmetic_tag: b"original".to_vec(),
                        ..Default::default()
                    },
                )
            })
            .collect(),
        ..Default::default()
    };
    let mut frame = appearance_frame(100, true, None);
    let mut rows: Vec<_> = scan.appearances.values().cloned().collect();
    rows[7].cosmetic_tag = b"changed".to_vec();
    *frame.tables[TableId::CountryAppearance.index()] =
        TableView::Complete(TableRows::CountryAppearance(rows));
    let row = DeltaRow {
        key: 7,
        mask: field_mask(TableId::CountryAppearance, &["cosmetic_tag"]),
        format: replay_data::format::FormatVersion::CURRENT.version(),
        recorded_fields: 0,
        values: Vec::new(),
    };
    let mut dirty = BTreeSet::new();
    assert!(scan.apply_appearances(&frame, Some((&[row], &[])), &mut dirty));
    assert_eq!(dirty, BTreeSet::from([7]));
    assert_eq!(scan.native.work.row_visits, 1);
    assert_eq!(scan.appearances[&7].cosmetic_tag, b"changed");
    dirty.clear();
    assert!(scan.apply_appearances(&frame, Some((&[], &[8])), &mut dirty));
    assert_eq!(dirty, BTreeSet::from([8]));
    assert!(!scan.appearances.contains_key(&8));
    scan.appearances_complete = false;
    assert!(!scan.apply_appearances(&frame, Some((&[], &[])), &mut dirty));
}

#[test]
fn refused_unpaired_labels_stay_distinct_parts_and_are_warned() {
    use crate::products::stability_parts::scan_parts;
    use replay_data::rows::CountryStabilityLineRow as Line;
    let line = |line, value, text: &[u8]| Line {
        country: 9001,
        line,
        key: 0x8d,
        kind: Line::UNPAIRED,
        value,
        text: text.to_vec(),
        ..Default::default()
    };
    let lines = vec![line(0, 10_000, b"caf\xc3"), line(1, 15_000, b"Front\x14")];
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            vec![stability_frame(100, stability_inputs(), 100, lines)],
        )],
    )
    .unwrap();
    let (stability, _, warnings) = scan_parts(&replay, 1, "GER").unwrap();
    let value = |id: &str| {
        stability
            .parts
            .iter()
            .find(|part| part.id == id)
            .map(|part| part.runs[0].value_raw.clone())
    };
    assert_eq!(value(r"unpaired-refused:caf\xc3"), Some("10000".to_owned()));
    assert_eq!(value(r"unpaired-refused:Front\x14"), Some("15000".to_owned()));
    assert!(
        warnings
            .iter()
            .any(|warning| warning.contains("stability tree labels are not shown")),
        "{warnings:?}"
    );
}

#[test]
fn stability_parts_add_up_to_the_value_and_are_known_only_where_recorded() {
    use crate::products::stability_parts::scan_parts;
    use dashboard_base::products::country_groups::HourSpan;
    use replay_data::rows::CountryStabilityLineRow as Line;
    let line = |line, kind, identity: &[u8], value, text: &[u8]| Line {
        country: 9001,
        line,
        key: 0x8d,
        kind,
        identity: identity.to_vec(),
        value,
        text: text.to_vec(),
        ..Default::default()
    };
    // stability_factor is 30 000: the two lines explain 25 000, the rest is a remainder.
    let lines = vec![
        line(0, Line::IDEA, b"idea_a", 10_000, b""),
        line(2, Line::UNPAIRED, b"", 15_000, b"Decision: X"),
    ];
    // A read whose lines table was not recorded: its parts are not known.
    let mut unlined = stability_frame(103, stability_inputs(), 103, vec![]);
    *unlined.tables[TableId::CountryStabilityLines.index()] = TableView::Unavailable {
        reason: "fixture".into(),
    };
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            vec![
                stability_frame(100, stability_inputs(), 100, lines.clone()),
                // Carried from the hour-100 read.
                stability_frame(101, stability_inputs(), 100, lines),
                // A read with no lines: the whole modifier term is unexplained.
                stability_frame(102, stability_inputs(), 102, vec![]),
                unlined,
            ],
        )],
    )
    .unwrap();
    let (stability, war_support, warnings) = scan_parts(&replay, 1, "GER").unwrap();
    let observed = [HourSpan {
        first_hour: 100,
        last_hour: 102,
    }];
    assert_eq!(stability.observed, observed);
    assert_eq!(war_support.observed, observed);
    let at = |parts: &crate::products::stability_parts::MeasureParts, hour: i32| -> i64 {
        parts
            .parts
            .iter()
            .flat_map(|part| &part.runs)
            .filter(|run| run.first_hour <= hour && hour <= run.last_hour)
            .map(|run| run.value_raw.parse::<i64>().unwrap())
            .sum()
    };
    // The parts add up to the displayed values (79.249% and the war support total).
    assert_eq!(at(&stability, 100), 79_249);
    assert_eq!(at(&stability, 101), 79_249);
    assert_eq!(at(&stability, 102), 79_249);
    let remainder = stability
        .parts
        .iter()
        .find(|part| part.id == "remainder")
        .unwrap();
    assert_eq!(
        remainder
            .runs
            .iter()
            .map(|run| (run.first_hour, run.last_hour, run.value_raw.as_str()))
            .collect::<Vec<_>>(),
        [(100, 101, "5000"), (102, 102, "30000")]
    );
    let displayed = crate::products::country_stability::formula::war_support(&stability_inputs());
    assert_eq!(at(&war_support, 100), displayed.value.unwrap());
    let ids: Vec<_> = stability
        .parts
        .iter()
        .map(|part| part.id.as_str())
        .collect();
    for id in [
        "term:base",
        "line:idea:idea_a:0:0",
        "unpaired:Decision: X",
        "remainder",
    ] {
        assert!(ids.contains(&id), "{id} in {ids:?}");
    }
    let idea = stability
        .parts
        .iter()
        .find(|part| part.id == "line:idea:idea_a:0:0")
        .unwrap();
    assert_eq!(
        idea.runs
            .iter()
            .map(|run| (run.first_hour, run.last_hour, run.value_raw.as_str()))
            .collect::<Vec<_>>(),
        [(100, 101, "10000")]
    );
    assert!(
        warnings
            .iter()
            .any(|warning| warning.contains("1 reads recorded no modifier lines"))
    );
    // Lines missing at one hour and recorded again at the next hour of the same
    // read: the parts are known again there, not lost until the next read.
    let mut missing = stability_frame(200, stability_inputs(), 200, vec![]);
    *missing.tables[TableId::CountryStabilityLines.index()] = TableView::Unavailable {
        reason: "fixture".into(),
    };
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            vec![
                missing,
                stability_frame(201, stability_inputs(), 200, vec![]),
            ],
        )],
    )
    .unwrap();
    let (stability, _, warnings) = scan_parts(&replay, 1, "GER").unwrap();
    assert_eq!(
        stability.observed,
        [HourSpan {
            first_hour: 201,
            last_hour: 201
        }]
    );
    assert!(warnings.is_empty(), "{warnings:?}");
    // Parts carried from a read are not reused for an hour whose lines table is missing.
    let mut dropped = stability_frame(301, stability_inputs(), 300, vec![]);
    *dropped.tables[TableId::CountryStabilityLines.index()] = TableView::Unavailable {
        reason: "fixture".into(),
    };
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            vec![
                stability_frame(300, stability_inputs(), 300, vec![]),
                dropped,
            ],
        )],
    )
    .unwrap();
    let (stability, _, warnings) = scan_parts(&replay, 1, "GER").unwrap();
    assert_eq!(
        stability.observed,
        [HourSpan {
            first_hour: 300,
            last_hour: 300
        }]
    );
    assert!(warnings.is_empty(), "{warnings:?}");
}

#[test]
fn stability_parts_name_a_read_s_characters_as_recorded_at_that_read() {
    use crate::products::stability_parts::scan_parts;
    use replay_data::rows::CountryStabilityLineRow as Line;
    let leader = vec![Line {
        country: 9001,
        line: 0,
        key: 0x8d,
        kind: Line::COUNTRY_LEADER,
        identity_id: 7,
        value: 30_000,
        ..Default::default()
    }];
    let named = |mut frame: DecodedFrame, saved: &str| {
        *frame.tables[TableId::Characters.index()] =
            TableView::Complete(TableRows::Characters(vec![
                replay_data::rows::CharacterRow {
                    id: 7,
                    token: saved.as_bytes().to_vec(),
                    name_token_id: -1,
                    ..Default::default()
                },
            ]));
        frame
    };
    // Read 100's lines are missing at hour 100 and arrive at 101, after a rename.
    let mut first = named(
        stability_frame(100, stability_inputs(), 100, leader.clone()),
        "Old Name",
    );
    *first.tables[TableId::CountryStabilityLines.index()] = TableView::Unavailable {
        reason: "fixture".into(),
    };
    let second = named(
        stability_frame(101, stability_inputs(), 100, leader),
        "New Name",
    );
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, vec![first, second])]).unwrap();
    let (stability, _, _) = scan_parts(&replay, 1, "GER").unwrap();
    let ids: Vec<_> = stability
        .parts
        .iter()
        .map(|part| part.id.as_str())
        .collect();
    assert!(ids.iter().any(|id| id.ends_with(":Old Name")), "{ids:?}");
    assert!(!ids.iter().any(|id| id.ends_with(":New Name")), "{ids:?}");
}

/// GER's one idea line, carried from the hour-100 read, at `hour`: its lines
/// recorded or not, and its CountryAppearance row with this cosmetic tag (None:
/// no row).
fn appearance_frame(hour: i32, lined: bool, cosmetic: Option<&str>) -> DecodedFrame {
    let idea = replay_data::rows::CountryStabilityLineRow {
        country: 9001,
        line: 0,
        key: 0x8d,
        kind: replay_data::rows::CountryStabilityLineRow::IDEA,
        identity: b"idea_a".to_vec(),
        value: 10_000,
        ..Default::default()
    };
    let mut frame = stability_frame(hour, stability_inputs(), 100, vec![idea]);
    if !lined {
        *frame.tables[TableId::CountryStabilityLines.index()] = TableView::Unavailable {
            reason: "fixture".into(),
        };
    }
    *frame.tables[TableId::CountryAppearance.index()] =
        TableView::Complete(TableRows::CountryAppearance(
            cosmetic
                .into_iter()
                .map(|cosmetic| replay_data::rows::CountryAppearanceRow {
                    country: 9001,
                    cosmetic_tag: cosmetic.as_bytes().to_vec(),
                    ..Default::default()
                })
                .collect(),
        ));
    frame
}
