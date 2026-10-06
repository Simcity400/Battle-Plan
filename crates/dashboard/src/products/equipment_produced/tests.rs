use super::*;
use crate::test_fixtures::{CAMPAIGN, records, source};
use replay::{
    container::Replay,
    frame::DecodedFrame,
    rows::{CountryLinkRow, CountryRow, TableRows},
};

fn hour(year: i64, month: usize, day: i64, hour: i64) -> i32 {
    use engine::cdate::{HOUR_EPOCH, MONTH_LENGTHS};
    let before: i64 = MONTH_LENGTHS[..month - 1]
        .iter()
        .map(|days| i64::from(*days))
        .sum();
    (i64::from(HOUR_EPOCH) + (year * 365 + before + day - 1) * 24 + hour) as i32
}

#[test]
fn produced_selections_follow_the_recorded_countries_and_types() {
    let types: HashMap<_, _> = (0..65)
        .map(|variant| {
            (
                variant,
                VariantType {
                    key: format!("type_{variant}"),
                    ordinal: Some(variant as usize),
                    domain: Some(Domain::Land),
                },
            )
        })
        .collect();
    let scanned = ScannedProduced {
        tags: (0..5_000)
            .map(|country| {
                (
                    format!("T{country}"),
                    TagHistory {
                        hours: vec![10],
                        last_hours: vec![10],
                        starts: vec![true],
                        identities: vec![(country, country, 0)],
                        changes: BTreeMap::from([(
                            (u64::from(country) % types.len() as u64, false),
                            vec![(
                                0,
                                Total {
                                    units: 1,
                                    inexact: 0,
                                },
                            )],
                        )]),
                        ..Default::default()
                    },
                )
            })
            .collect(),
        ..Default::default()
    };
    let mut params = ProducedParams {
        tags: scanned.tags.keys().cloned().collect(),
        included: types.values().map(|kind| kind.key.clone()).collect(),
        ..produced_params(&[], None, ProducedMeasure::Total, ProducedGrouping::Type)
    };
    assert!(EquipmentProduced::validate(&params).is_ok());
    let mut long_key = params.clone();
    long_key.included = vec!["equipment_type_".repeat(30)];
    assert!(EquipmentProduced::validate(&long_key).is_ok());
    let groups = grouped(&scanned, &types, Named::default(), &params);
    let requested = requested(&scanned, &groups, &params, &AtomicBool::new(false)).unwrap();
    assert_eq!(requested.series.len(), scanned.tags.len());
    assert_eq!(requested.keys.len(), types.len());
    params.tags.clear();
    params.top = Some(scanned.tags.len() as u32);
    assert!(EquipmentProduced::validate(&params).is_ok());
    assert_eq!(
        super::requested(&scanned, &groups, &params, &AtomicBool::new(false))
            .unwrap()
            .series
            .len(),
        scanned.tags.len()
    );
}

#[test]
fn a_month_before_is_the_same_moment_of_the_month_before_or_its_last_day() {
    assert_eq!(
        month_before(hour(1940, 3, 15, 7)),
        Some(hour(1940, 2, 15, 7))
    );
    // February is shorter: the 31st of March looks back to the 28th.
    assert_eq!(
        month_before(hour(1940, 3, 31, 7)),
        Some(hour(1940, 2, 28, 7))
    );
    // January looks back into the year before.
    assert_eq!(
        month_before(hour(1940, 1, 10, 0)),
        Some(hour(1939, 12, 10, 0))
    );
    assert_eq!(month_before(engine::cdate::HOUR_EPOCH - 1), None);
    assert_eq!(month_before(hour(0, 1, 5, 0)), None);
}

#[test]
fn values_code_as_deltas_from_the_first_nonzero_with_unknown_ranges() {
    let coded = encode(&[Some(0), Some(0), Some(5), Some(7), Some(7), Some(7)]).unwrap();
    assert_eq!((coded.from, coded.unknown.as_slice()), (2, &[][..]));
    assert_eq!(coded.deltas, [5, 2]);
    // Unknown samples are ranges and hold the last known sum, so a known value
    // after them is coded against the one before them.
    let unknown = encode(&[Some(0), Some(4), None, None, Some(9), None]).unwrap();
    assert_eq!(
        (unknown.from, unknown.unknown.as_slice()),
        (1, &[2, 4, 5, 6][..])
    );
    assert_eq!(unknown.deltas, [4, 0, 0, 5]);
    // Unknown before anything nonzero is still coded.
    let early = encode(&[None, Some(0)]).unwrap();
    assert_eq!((early.from, early.unknown.as_slice()), (2, &[0, 1][..]));
    // A per-week value can fall again.
    assert_eq!(
        encode(&[Some(3), Some(1), Some(0)]).unwrap().deltas,
        [3, -2, -1]
    );
    // A known zero throughout needs nothing.
    assert_eq!(encode(&[Some(0), Some(0)]), None);
    assert_eq!(encode(&[]), None);
}

#[test]
fn clipped_pieces_keep_a_break_only_where_they_start() {
    let piece = |first, last, starts, sample| Piece {
        first,
        last,
        starts,
        sample,
    };
    let clipped = clip(
        &[piece(0, 23, true, 0), piece(24, 47, false, 1)],
        &[
            HourSpan {
                first_hour: 10,
                last_hour: 30,
            },
            HourSpan {
                first_hour: 40,
                last_hour: 60,
            },
        ],
    );
    let shape: Vec<_> = clipped
        .iter()
        .map(|piece| (piece.first, piece.last, piece.starts, piece.sample))
        .collect();
    assert_eq!(
        shape,
        [(10, 23, false, 0), (24, 30, false, 1), (40, 47, false, 1)]
    );
}

fn produced_units(texts: &[&[u8]]) -> io::Result<installed_content::equipment::Database> {
    let paths: Vec<_> = (0..texts.len())
        .map(|i| format!("common/units/equipment/{i:03}.txt"))
        .collect();
    let files: Vec<_> = paths
        .iter()
        .zip(texts)
        .map(|(p, text)| (p.as_str(), std::str::from_utf8(text).unwrap()))
        .collect();
    let units = installed_content::equipment::fixture::load(&files).0;
    match &units.lost {
        Some(reason) => Err(crate::invalid(reason)),
        None => Ok(units),
    }
}

fn produced_hour(day: i32) -> i32 {
    let replay::cadence::Cadence::Daily(slot) =
        replay::cadence::cadence(TableId::ProductionLineOutput)
    else {
        panic!("ProductionLineOutput is read in a daily slot");
    };
    day * replay::cadence::STAGGER_PERIOD + slot as i32
}

/// One output row: line, ordinal, producer link, variant, units, exact, ended.
type OutputFixture = (u64, u32, u32, u64, i64, bool, bool);

/// Germany (native id `germany`, slot 7, also linked as 11) and Italy (21, slot
/// 8) with the production lines a census lists (handle, producer link, kind,
/// variant) and the output rows, all counted since `since`. `read` times the
/// census's walk; `output` None records the output as unavailable. Variants 10
/// and 20 are infantry equipment (token 1), 30 a destroyer (token 2); variant 20
/// is named "Gewehr 43" from day 2 on.
fn produced_frame(
    hour: i32,
    read: bool,
    germany: u32,
    since: i32,
    lines: &[(u64, u32, u8, u64)],
    output: Option<&[OutputFixture]>,
) -> DecodedFrame {
    use replay::rows::{
        LexicalTokenRow, ProductionLineOutputRow, ProductionLineRow, VariantDefinitionRow,
    };
    let mut frame = source(1, hour, Some(1));
    let country = |id, canonical_slot, tag: &[u8]| CountryRow {
        id,
        canonical_slot,
        tag: tag.to_vec(),
        ..Default::default()
    };
    // Rows in ascending key order (native ids).
    *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(vec![
        country(21, 8, b"ITA"),
        country(germany, 7, b"GER"),
    ]));
    let mut links = vec![
        CountryLinkRow {
            id: 11,
            canonical_slot: 7,
        },
        CountryLinkRow {
            id: 21,
            canonical_slot: 8,
        },
        CountryLinkRow {
            id: germany,
            canonical_slot: 7,
        },
    ];
    links.sort_by_key(|link| link.id);
    *frame.tables[TableId::CountryLinks.index()] =
        TableView::Complete(TableRows::CountryLinks(links));
    *frame.tables[TableId::ProductionLines.index()] =
        TableView::Complete(TableRows::ProductionLines(
            lines
                .iter()
                .map(|&(handle, country, kind, variant)| ProductionLineRow {
                    handle,
                    country,
                    kind,
                    variant,
                    ..Default::default()
                })
                .collect(),
        ));
    *frame.tables[TableId::ProductionLineOutput.index()] = match output {
        Some(rows) => TableView::Complete(TableRows::ProductionLineOutput(
            rows.iter()
                .map(|&(line, ordinal, country, variant, units, exact, ended)| {
                    ProductionLineOutputRow {
                        line,
                        ordinal,
                        country,
                        variant,
                        units,
                        mio_credit: units,
                        since_hour: since,
                        observed_since_creation: true,
                        ended,
                        exact,
                        ..Default::default()
                    }
                })
                .collect(),
        )),
        None => TableView::Unavailable {
            reason: "unhooked".into(),
        },
    };
    *frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![
            LexicalTokenRow {
                token: 1,
                literal: b"infantry_equipment_1".to_vec(),
            },
            LexicalTokenRow {
                token: 2,
                literal: b"destroyer_1".to_vec(),
            },
        ]));
    let named = hour >= produced_hour(2);
    let variant = |id, equipment_token, name: &[u8]| VariantDefinitionRow {
        id,
        equipment_token,
        creator_link: 9001,
        display_name: name.to_vec(),
        ..Default::default()
    };
    *frame.tables[TableId::VariantDefinitions.index()] =
        TableView::Complete(TableRows::VariantDefinitions(vec![
            variant(10, 1, b""),
            variant(20, 1, if named { b"Gewehr 43" } else { b"" }),
            variant(30, 2, b""),
        ]));
    if read {
        frame.metrics.table_ns[TableId::ProductionLines.index()] = 1;
        frame.metrics.table_ns[TableId::VariantDefinitions.index()] = 1;
    }
    frame
}

/// One day of frames: the read at the output's slot hour, then the 23 hours it
/// is carried to, as a recording holds them.
fn produced_day(
    day: i32,
    germany: u32,
    since: i32,
    lines: &[(u64, u32, u8, u64)],
    output: Option<&[OutputFixture]>,
) -> Vec<DecodedFrame> {
    (0..replay::cadence::STAGGER_PERIOD)
        .map(|offset| {
            produced_frame(
                produced_hour(day) + offset,
                offset == 0,
                germany,
                since,
                lines,
                output,
            )
        })
        .collect()
}

fn produced_source(frames: Vec<DecodedFrame>) -> crate::Source {
    crate::Source::from_replay(
        crate::SourceKey {
            source_id: "produced".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        Arc::new(Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap()),
        &AtomicBool::new(false),
    )
    .unwrap()
}

fn produced_params(
    tags: &[&str],
    top: Option<u32>,
    measure: crate::products::equipment_produced::ProducedMeasure,
    grouping: crate::products::equipment_produced::ProducedGrouping,
) -> crate::products::equipment_produced::ProducedParams {
    crate::products::equipment_produced::ProducedParams {
        branch_id: "1".into(),
        tags: tags.iter().map(|tag| (*tag).to_owned()).collect(),
        top,
        measure,
        grouping,
        included: Vec::new(),
        refits: false,
    }
}

/// Each sample's value of a key (by its position in the reply's keys), as the
/// widget decodes it: 0 before `from`, the running sum of the deltas, unknown
/// from `unknown_from`.
fn produced_values(
    series: &crate::products::equipment_produced::ProducedSeries,
    key: u32,
) -> Vec<Option<i64>> {
    produced_decode(
        series
            .values
            .iter()
            .find(|values| values.key == key)
            .map(|values| &values.values),
        series.first_hours.len(),
    )
}

/// A coded value decoded at each of `length` samples, as the widget decodes it: 0
/// before `from`, the running sum of the deltas, unknown inside `unknown`'s ranges.
fn produced_decode(
    coded: Option<&crate::products::equipment_produced::ProducedCoded>,
    length: usize,
) -> Vec<Option<i64>> {
    (0..length)
        .map(|index| {
            let Some(coded) = coded else {
                return Some(0);
            };
            if coded
                .unknown
                .chunks(2)
                .any(|range| (range[0] as usize..range[1] as usize).contains(&index))
            {
                return None;
            }
            let from = coded.from as usize;
            if index < from {
                return Some(0);
            }
            let upto = (index - from + 1).min(coded.deltas.len());
            Some(coded.deltas[..upto].iter().sum())
        })
        .collect()
}

#[test]
fn produced_empty_resolved_groups_remain_unknown_for_an_unresolved_only_country() {
    use crate::products::equipment_produced::{
        Named, ProducedGrouping, ProducedMeasure, reply, scan,
    };
    let units = produced_units(&[br#"
        equipments = {
            infantry_equipment = { is_archetype = yes type = infantry }
            infantry_equipment_1 = { archetype = infantry_equipment }
        }
    "#])
    .unwrap();
    let since = produced_hour(0);
    let lines = [(1, 9001, 0, 10), (2, 21, 0, 30)];
    for exact in [true, false] {
        let frames = (0..3)
            .flat_map(|day| {
                let mut frames = produced_day(
                    day,
                    9001,
                    since,
                    &lines,
                    Some(&[
                        (1, 0, 11, 10, 3 * i64::from(day), true, false),
                        (2, 0, 21, 30, i64::from(day), day == 0 || exact, false),
                    ]),
                );
                for frame in &mut frames {
                    let TableView::Complete(TableRows::Countries(countries)) =
                        &mut *frame.tables[TableId::Countries.index()]
                    else {
                        unreachable!()
                    };
                    for country in countries {
                        country.tag = if country.canonical_slot == 7 {
                            b"AAA"
                        } else {
                            b"BBB"
                        }
                        .to_vec();
                    }
                }
                frames
            })
            .collect();
        let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
        let scanned = scan(&replay, 1, &AtomicBool::new(false)).unwrap();
        for (grouping, key) in [
            (ProducedGrouping::Type, "infantry_equipment"),
            (ProducedGrouping::Domain, "land"),
        ] {
            let params = produced_params(&["AAA", "BBB"], None, ProducedMeasure::Total, grouping);
            let result = reply(
                &scanned,
                &params,
                Named {
                    units: Some(&units),
                    texts: None,
                },
                Vec::new(),
                &AtomicBool::new(false),
            )
            .unwrap();
            let key = result.keys.iter().position(|item| item.key == key).unwrap() as u32;
            let country = |tag| {
                result
                    .series
                    .iter()
                    .find(|series| series.tag == tag)
                    .unwrap()
            };
            assert_eq!(
                produced_values(country("AAA"), key),
                [Some(0), Some(3), Some(6)]
            );
            // BBB has no resolved changes, but its unresolved output could be rifles.
            assert_eq!(produced_values(country("BBB"), key), [Some(0), None, None]);
            assert!(country("BBB").values.iter().any(|value| value.key == key));
            if exact {
                assert_eq!(
                    produced_decode(country("BBB").total.as_ref(), 3),
                    [Some(0), Some(1), Some(2)]
                );
            }
        }
    }
}

#[test]
fn produced_types_an_unresolved_variant_could_belong_to_are_unknown_while_it_holds_anything() {
    use crate::products::{
        equipment::Texts,
        equipment_produced::{Named, ProducedGrouping, ProducedMeasure, reply, scan},
    };
    // Infantry equipment resolves; the destroyer (variant 30) is not authored.
    let units = produced_units(&[br#"
        equipments = {
            infantry_equipment = { is_archetype = yes type = infantry }
            infantry_equipment_1 = { archetype = infantry_equipment }
        }
    "#])
    .unwrap();
    let texts: installed_content::localisation::Localisation = std::iter::empty().collect();
    let since = produced_hour(0);
    let lines = [(1, 9001, 0, 10), (2, 9001, 1, 30)];
    // Rifles every day; the destroyer is delivered from day 2 on.
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            (0..4)
                .flat_map(|day: i32| {
                    produced_day(
                        day,
                        9001,
                        since,
                        &lines,
                        Some(&[
                            (1, 0, 11, 10, 3 * i64::from(day), true, false),
                            (2, 0, 11, 30, i64::from(day >= 2), true, false),
                        ]),
                    )
                })
                .collect(),
        )],
    )
    .unwrap();
    let scanned = scan(&replay, 1, &AtomicBool::new(false)).unwrap();
    let named = Named {
        units: Some(&units),
        texts: Some(&texts as &dyn Texts),
    };
    let load = |grouping, included: &[&str]| {
        let mut params = produced_params(&["GER"], None, ProducedMeasure::Total, grouping);
        params.included = included.iter().map(|key| (*key).to_owned()).collect();
        reply(
            &scanned,
            &params,
            named,
            Vec::new(),
            &AtomicBool::new(false),
        )
        .unwrap()
    };
    let key_of = |reply: &crate::products::equipment_produced::EquipmentProducedReply,
                  key: &str| {
        reply.keys.iter().position(|item| item.key == key).unwrap() as u32
    };
    let types = load(ProducedGrouping::Type, &[]);
    let germany = &types.series[0];
    // The resolved type is known only while the unresolved variant holds nothing.
    assert_eq!(
        produced_values(germany, key_of(&types, "infantry_equipment")),
        [Some(0), Some(3), None, None]
    );
    // The unresolved group itself, and the total, stay exact.
    assert_eq!(
        produced_values(germany, key_of(&types, ":destroyer_1")),
        [Some(0), Some(0), Some(1), Some(1)]
    );
    assert_eq!(
        produced_decode(germany.total.as_ref(), 4),
        [Some(0), Some(3), Some(7), Some(10)]
    );
    // By arm: the land arm likewise; the unresolved arm is exact.
    let arms = load(ProducedGrouping::Domain, &[]);
    assert_eq!(
        produced_values(&arms.series[0], key_of(&arms, "land")),
        [Some(0), Some(3), None, None]
    );
    assert_eq!(
        produced_values(&arms.series[0], key_of(&arms, "unresolved")),
        [Some(0), Some(0), Some(1), Some(1)]
    );
    // By variant every value is its own, so all are exact.
    let variants = load(ProducedGrouping::Variant, &[]);
    assert_eq!(
        produced_values(&variants.series[0], key_of(&variants, "10")),
        [Some(0), Some(3), Some(6), Some(9)]
    );
    // Including only infantry: the left-out destroyer could be infantry, so the
    // included total is unknown once it holds anything; so is the type.
    let infantry = load(ProducedGrouping::Type, &["infantry_equipment"]);
    assert_eq!(
        produced_decode(infantry.series[0].total.as_ref(), 4),
        [Some(0), Some(3), None, None]
    );
    assert_eq!(
        produced_values(&infantry.series[0], 0),
        [Some(0), Some(3), None, None]
    );
}

#[test]
fn produced_totals_survive_deleted_lines_and_count_lines_made_between_reads() {
    use crate::products::{
        LoadContext, Product,
        equipment_produced::{EquipmentProduced, ProducedGrouping, ProducedMeasure},
    };
    let since = produced_hour(0);
    let frame = |day, lines: &[(u64, u32, u8, u64)], output: &[OutputFixture]| {
        produced_day(day, 9001, since, lines, Some(output))
    };
    let source = produced_source(
        [
            frame(
                0,
                &[(1, 9001, 0, 10), (2, 21, 0, 10)],
                &[
                    (1, 0, 11, 10, 0, true, false),
                    (2, 0, 21, 10, 0, true, false),
                ],
            ),
            // Line 3 was made and deleted since the last census: only its final rows name it.
            frame(
                1,
                &[(1, 9001, 0, 10), (2, 21, 0, 10)],
                &[
                    (1, 0, 11, 10, 5, true, false),
                    (2, 0, 21, 10, 3, true, false),
                    (3, 0, 9001, 20, 2, true, true),
                ],
            ),
            // Line 1 ended: its final count is published once, then dropped.
            frame(
                2,
                &[(2, 21, 0, 10), (4, 21, 2, 30)],
                &[
                    (1, 0, 11, 10, 7, true, true),
                    (2, 0, 21, 10, 4, true, false),
                    (4, 0, 21, 30, 0, true, false),
                ],
            ),
            // A refit line delivers a ship Italy already had.
            frame(
                3,
                &[(2, 21, 0, 10), (4, 21, 2, 30)],
                &[
                    (2, 0, 21, 10, 6, true, false),
                    (4, 0, 21, 30, 1, true, false),
                ],
            ),
        ]
        .concat(),
    );
    let context = LoadContext { content: None };
    let load = |params| {
        EquipmentProduced::load(&source, &params, &context, &AtomicBool::new(false)).unwrap()
    };
    let reply = load(produced_params(
        &["GER", "ITA", "XXX"],
        None,
        ProducedMeasure::Total,
        ProducedGrouping::Type,
    ));
    // Without content a type is kept apart under its token, never merged.
    let keys: Vec<&str> = reply.keys.iter().map(|key| key.key.as_str()).collect();
    assert_eq!(keys, [":infantry_equipment_1"]);
    let types: Vec<&str> = reply.types.iter().map(|kind| kind.key.as_str()).collect();
    assert_eq!(types, [":destroyer_1", ":infantry_equipment_1"]);
    assert_eq!(reply.reads.len(), 4);
    assert_eq!(reply.observations, [since]);
    let germany = &reply.series[0];
    assert_eq!(germany.tag, "GER");
    assert_eq!(
        germany.first_hours,
        [0, 1, 2, 3].map(produced_hour).to_vec()
    );
    assert_eq!(germany.last_hours[0], produced_hour(1) - 1);
    // Only the first sample starts a line: the hours between daily reads carry it.
    assert_eq!(germany.breaks, [0]);
    // 5 then 7 from line 1 (kept after it ended), and 2 from line 3.
    assert_eq!(
        produced_values(germany, 0),
        [Some(0), Some(7), Some(9), Some(9)]
    );
    let italy = &reply.series[1];
    assert_eq!(
        produced_values(italy, 0),
        [Some(0), Some(3), Some(4), Some(6)]
    );
    assert!(reply.warnings.iter().any(|warning| warning.contains("XXX")));
    assert!(
        reply
            .warnings
            .iter()
            .any(|warning| warning.contains("1 lines were made and deleted")),
        "{:?}",
        reply.warnings
    );
    // Refits count only when asked, as the type they deliver.
    let mut refits = produced_params(
        &["ITA"],
        None,
        ProducedMeasure::Total,
        ProducedGrouping::Type,
    );
    refits.refits = true;
    let with = load(refits);
    let keys: Vec<&str> = with.keys.iter().map(|key| key.key.as_str()).collect();
    assert_eq!(keys, [":destroyer_1", ":infantry_equipment_1"]);
    assert_eq!(
        produced_values(&with.series[0], 0),
        [Some(0), Some(0), Some(0), Some(1)]
    );
    // By variant: a variant is named only from the read that named it.
    let variants = load(produced_params(
        &["GER"],
        None,
        ProducedMeasure::Total,
        ProducedGrouping::Variant,
    ));
    let keys: Vec<&str> = variants.keys.iter().map(|key| key.key.as_str()).collect();
    assert_eq!(keys, ["10", "20"]);
    assert_eq!(variants.keys[1].names[0].name, None);
    assert_eq!(
        variants.keys[1].names.last().unwrap(),
        &crate::products::equipment::StockVariantName {
            first_hour: produced_hour(2),
            name: Some("Gewehr 43".into()),
            known: true,
            design: None,
        }
    );
    assert_eq!(
        produced_values(&variants.series[0], 1),
        [Some(0), Some(2), Some(2), Some(2)]
    );
    // By domain: without content, an unresolved arm.
    let domains = load(produced_params(
        &["GER"],
        None,
        ProducedMeasure::Total,
        ProducedGrouping::Domain,
    ));
    assert_eq!(domains.keys[0].key, "unresolved");
    // The types alone, for the settings.
    let listed = load(produced_params(
        &[],
        None,
        ProducedMeasure::Total,
        ProducedGrouping::Type,
    ));
    assert!(listed.series.is_empty() && listed.keys.is_empty());
    assert_eq!(listed.types.len(), 2);
}

#[test]
fn produced_totals_stay_unknown_where_a_count_was_lost_or_not_exact_or_unread() {
    use crate::products::{
        LoadContext, Product,
        equipment_produced::{EquipmentProduced, ProducedGrouping, ProducedMeasure},
    };
    let since = produced_hour(0);
    let lines = [(1, 9001, 0, 10), (2, 21, 0, 10)];
    let frame = |day: i32, germany_exact, italy: bool| {
        let mut rows = vec![(1, 0, 11, 10, i64::from(day), germany_exact, false)];
        if italy {
            rows.push((2, 0, 21, 10, 2 * i64::from(day), true, false));
        }
        produced_day(day, 9001, since, &lines, Some(&rows))
    };
    let source = produced_source(
        [
            frame(0, true, true),
            frame(1, true, true),
            // Germany's row left a tick out: unknown from here on.
            frame(2, false, true),
            frame(3, true, true),
            // Italy's row vanished without its ended publication: its final count is lost.
            frame(4, true, false),
            // The output became unavailable: nothing is known there.
            produced_day(5, 9001, since, &lines, None),
        ]
        .concat(),
    );
    let reply = EquipmentProduced::load(
        &source,
        &produced_params(
            &["GER", "ITA"],
            None,
            ProducedMeasure::Total,
            ProducedGrouping::Type,
        ),
        &LoadContext { content: None },
        &AtomicBool::new(false),
    )
    .unwrap();
    let germany = &reply.series[0];
    assert_eq!(germany.first_hours.len(), 5, "the unread day has no sample");
    assert_eq!(
        produced_values(germany, 0),
        [Some(0), Some(1), None, None, None]
    );
    assert_eq!(
        produced_values(&reply.series[1], 0),
        [Some(0), Some(2), Some(4), Some(6), None]
    );
    for expected in [
        "left out a tick",
        "disappeared without",
        "no readable output (unhooked)",
    ] {
        assert!(
            reply
                .warnings
                .iter()
                .any(|warning| warning.contains(expected)),
            "{expected}: {:?}",
            reply.warnings
        );
    }
    // A recording without the output says so rather than showing zeros.
    let missing = produced_source(vec![produced_frame(
        produced_hour(0),
        true,
        9001,
        since,
        &lines,
        None,
    )]);
    let reply = EquipmentProduced::load(
        &missing,
        &produced_params(
            &["GER"],
            None,
            ProducedMeasure::Total,
            ProducedGrouping::Type,
        ),
        &LoadContext { content: None },
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(reply.reads.is_empty() && reply.series.is_empty());
    assert!(
        reply
            .warnings
            .iter()
            .any(|warning| warning.contains("not in this recording"))
    );
}

#[test]
fn produced_per_week_needs_the_read_a_week_before_of_the_same_country() {
    use crate::products::{
        LoadContext, Product,
        equipment_produced::{EquipmentProduced, ProducedGrouping, ProducedMeasure},
    };
    let since = produced_hour(0);
    let lines = [(1, 9001, 0, 10)];
    // Germany delivers 2 a day; on day 9 another country takes the tag.
    let source = produced_source(
        (0..10)
            .flat_map(|day| {
                produced_day(
                    day,
                    if day < 9 { 9001 } else { 9002 },
                    since,
                    &lines,
                    Some(&[(1, 0, 11, 10, 2 * i64::from(day), true, false)]),
                )
            })
            .collect(),
    );
    let load = |measure| {
        EquipmentProduced::load(
            &source,
            &produced_params(&["GER"], None, measure, ProducedGrouping::Type),
            &LoadContext { content: None },
            &AtomicBool::new(false),
        )
        .unwrap()
    };
    let weekly = load(ProducedMeasure::Week);
    let germany = &weekly.series[0];
    // The first week has no read a week before, and nor has the new country.
    assert_eq!(germany.first_hours, [7, 8].map(produced_hour).to_vec());
    assert_eq!(germany.breaks, [0]);
    assert_eq!(produced_values(germany, 0), [Some(14), Some(14)]);
    // The running total jumps to the new country's (none), starting a new line.
    let total = load(ProducedMeasure::Total);
    let germany = &total.series[0];
    assert_eq!(germany.breaks, [0, 9]);
    assert_eq!(produced_values(germany, 0)[8..], [Some(16), Some(0)]);
    // Ten days hold no month.
    assert!(
        load(ProducedMeasure::Month).series[0]
            .first_hours
            .is_empty()
    );
}

#[test]
fn produced_zero_windows_remain_known_without_any_top_n_member() {
    use crate::products::{
        LoadContext, Product,
        equipment_produced::{EquipmentProduced, ProducedGrouping, ProducedMeasure},
    };
    let start = engine::cdate::HOUR_EPOCH / 24;
    let since = produced_hour(start);
    let lines = [(1, 9001, 0, 10)];
    let source = produced_source(
        (start..start + 40)
            .flat_map(|day| {
                produced_day(
                    day,
                    9001,
                    since,
                    &lines,
                    Some(&[(1, 0, 11, 10, 0, true, false)]),
                )
            })
            .collect(),
    );
    for (measure, first_day) in [(ProducedMeasure::Week, 7), (ProducedMeasure::Month, 31)] {
        let load = |tags: &[&str], top| {
            EquipmentProduced::load(
                &source,
                &produced_params(tags, top, measure, ProducedGrouping::Type),
                &LoadContext { content: None },
                &AtomicBool::new(false),
            )
            .unwrap()
        };
        let top = load(&[], Some(10));
        assert!(top.series.is_empty(), "zeros never rank");
        assert_eq!(
            top.known,
            vec![crate::products::country_groups::HourSpan {
                first_hour: produced_hour(start + first_day),
                last_hour: produced_hour(start + 40) - 1,
            }]
        );
        assert_eq!(load(&["GER"], None).known, top.known);
        assert!(load(&["absent"], None).known.is_empty());
    }
    // Reads alone do not establish an exact window for an inexact country.
    let mut scanned =
        crate::products::equipment_produced::scan(&source.replay(), 1, &AtomicBool::new(false))
            .unwrap();
    scanned.tags.retain(|tag, _| tag == "GER");
    scanned.tags.get_mut("GER").unwrap().changes.insert(
        (10, false),
        vec![(
            0,
            crate::products::equipment_produced::Total {
                units: 0,
                inexact: 1,
            },
        )],
    );
    let unknown = crate::products::equipment_produced::reply(
        &scanned,
        &produced_params(&[], Some(10), ProducedMeasure::Week, ProducedGrouping::Type),
        Default::default(),
        Vec::new(),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(unknown.series.is_empty());
    assert!(unknown.known.is_empty());
}

#[test]
fn known_windows_require_every_present_requested_country_to_be_exact() {
    let since = produced_hour(0);
    let frames = (0..10)
        .flat_map(|day| {
            let mut rows = vec![(1, 0, 11, 10, 0, true, false)];
            if day < 9 {
                rows.push((2, 0, 21, 30, 0, day != 8, false));
            }
            let mut frames = produced_day(
                day,
                9001,
                since,
                &[(1, 9001, 0, 10), (2, 21, 0, 30)],
                Some(&rows),
            );
            if day == 9 {
                for frame in &mut frames {
                    let TableView::Complete(TableRows::Countries(countries)) =
                        &mut *frame.tables[TableId::Countries.index()]
                    else {
                        unreachable!()
                    };
                    countries.retain(|country| country.canonical_slot != 8);
                }
            }
            frames
        })
        .collect();
    let source = produced_source(frames);
    let load = |tags: &[&str], top, measure| {
        EquipmentProduced::load(
            &source,
            &produced_params(tags, top, measure, ProducedGrouping::Type),
            &LoadContext { content: None },
            &AtomicBool::new(false),
        )
        .unwrap()
    };
    let spans = |reply: &EquipmentProducedReply| {
        reply
            .known
            .iter()
            .map(|span| (span.first_hour, span.last_hour))
            .collect::<Vec<_>>()
    };
    for (measure, first_day) in [(ProducedMeasure::Total, 0), (ProducedMeasure::Week, 7)] {
        let top = load(&[], Some(10), measure);
        assert!(top.series.is_empty(), "zeros and unknown totals never rank");
        // Both countries are zero through day 7. On day 8 one is unknown;
        // on day 9 it is absent, so it no longer blocks the other's known zero.
        assert_eq!(
            spans(&top),
            [
                (produced_hour(first_day), produced_hour(8) - 1),
                (produced_hour(9), produced_hour(10) - 1),
            ]
        );
        assert_eq!(
            load(&["GER", "ITA", "absent"], None, measure).known,
            top.known
        );
        // An unrequested country's unknown total cannot block explicit tags.
        assert_eq!(
            spans(&load(&["GER"], None, measure)),
            [(produced_hour(first_day), produced_hour(10) - 1),]
        );
    }
    // Before any complete window, even a recorded zero has no known week/month.
    assert!(load(&[], Some(10), ProducedMeasure::Month).known.is_empty());
}

#[test]
fn coverage_merges_overlaps_and_subtracts_unknown_spans_without_walking_hours() {
    let mut coverage = Coverage::default();
    for (first, last, total) in [
        (20, 30, Some(0)),
        (0, 9, Some(0)),
        (5, 12, Some(0)),
        (13, 15, Some(0)),
        (-10, -1, None),
        (3, 4, None),
        (4, 6, None),
        (10, 25, None),
        (i32::MAX - 2, i32::MAX, Some(0)),
        (i32::MAX, i32::MAX, None),
    ] {
        coverage.record(
            &Piece {
                first,
                last,
                starts: false,
                sample: 0,
            },
            total,
        );
    }
    let spans: Vec<_> = coverage
        .known()
        .iter()
        .map(|span| (span.first_hour, span.last_hour))
        .collect();
    assert_eq!(
        spans,
        [(0, 2), (7, 9), (26, 30), (i32::MAX - 2, i32::MAX - 1)]
    );
}

#[test]
fn produced_types_and_names_come_from_the_recorded_content_stack_mods_included() {
    use crate::products::{
        equipment::Texts,
        equipment_produced::{Named, ProducedGrouping, ProducedMeasure, reply, scan},
    };
    let root = std::env::temp_dir().join(format!(
        "dashboard_produced_mod_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let write = |layer: &str, path: &str, text: &str| {
        let path = root.join(layer).join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    };
    write(
        "vanilla",
        "common/units/equipment/infantry.txt",
        "equipments = { infantry_equipment = { is_archetype = yes type = infantry } infantry_equipment_1 = { archetype = infantry_equipment } }",
    );
    write(
        "vanilla",
        "common/units/equipment/ships.txt",
        "equipments = { destroyer = { is_archetype = yes type = convoy } destroyer_1 = { archetype = destroyer } }",
    );
    // The mod replaces the vanilla files of the same paths: its own archetype takes
    // the recorded variant, and the vanilla destroyer is gone.
    write(
        "mod",
        "common/units/equipment/infantry.txt",
        "equipments = { mod_small_arms = { is_archetype = yes type = infantry } infantry_equipment_1 = { archetype = mod_small_arms } }",
    );
    write(
        "mod",
        "common/units/equipment/ships.txt",
        "equipments = { }",
    );
    let layers = installed_content::content::ResolvedLayers::from_folders(&[
        (&root.join("vanilla"), &[][..]),
        (&root.join("mod"), &[][..]),
    ])
    .unwrap();
    let units = installed_content::equipment::Database::load_complete(&layers).unwrap();
    std::fs::remove_dir_all(&root).unwrap();
    // The mod's localisation names its type.
    let texts: installed_content::localisation::Localisation = [
        ("mod_small_arms".to_owned(), "Mod Rifles".to_owned()),
        (
            "infantry_equipment".to_owned(),
            "Infantry Equipment".to_owned(),
        ),
    ]
    .into_iter()
    .collect();
    let since = produced_hour(0);
    // Germany's rifles (variants 10 and 20) and a destroyer (variant 30) from a naval line.
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            [0, 1]
                .into_iter()
                .flat_map(|day| {
                    produced_day(
                        day,
                        9001,
                        since,
                        &[(1, 9001, 0, 10), (2, 9001, 1, 30)],
                        Some(&[
                            (1, 0, 11, 10, 5 * i64::from(day), true, false),
                            (2, 0, 11, 30, i64::from(day), true, false),
                        ]),
                    )
                })
                .collect(),
        )],
    )
    .unwrap();
    let scanned = scan(&replay, 1, &AtomicBool::new(false)).unwrap();
    let named = Named {
        units: Some(&units),
        texts: Some(&texts as &dyn Texts),
    };
    let load = |grouping| {
        reply(
            &scanned,
            &produced_params(&["GER"], None, ProducedMeasure::Total, grouping),
            named,
            Vec::new(),
            &AtomicBool::new(false),
        )
        .unwrap()
    };
    let types = load(ProducedGrouping::Type);
    let listed: Vec<(&str, Option<&str>)> = types
        .types
        .iter()
        .map(|kind| (kind.key.as_str(), kind.name.as_deref()))
        .collect();
    // The mod's type appears with its own name; the removed vanilla type is not
    // assumed: its variant is kept apart under its recorded token.
    assert_eq!(
        listed,
        [
            ("mod_small_arms", Some("Mod Rifles")),
            (":destroyer_1", None)
        ]
    );
    // The mod's archetype holds the rifles; once the unresolved destroyer holds
    // something it could be of that type too, so the type's value is unknown then,
    // while the total (5 rifles and 1 destroyer) stays exact.
    assert_eq!(produced_values(&types.series[0], 0), [Some(0), None]);
    assert_eq!(
        produced_decode(types.series[0].total.as_ref(), 2),
        [Some(0), Some(6)]
    );
    assert!(
        types
            .warnings
            .iter()
            .any(|warning| warning.starts_with("1 produced variants have an equipment type")),
        "{:?}",
        types.warnings
    );
    // By arm: the mod's type is land by the engine's classifiers; the unresolved one
    // is its own group, never merged into an arm.
    let arms = load(ProducedGrouping::Domain);
    let keys: Vec<&str> = arms.keys.iter().map(|key| key.key.as_str()).collect();
    assert_eq!(keys, ["land", "unresolved"]);
}

/// Loads GER and ITA by type as running totals, without content.
fn produced_load(
    source: &crate::Source,
    measure: crate::products::equipment_produced::ProducedMeasure,
) -> crate::products::equipment_produced::EquipmentProducedReply {
    use crate::products::{
        LoadContext, Product,
        equipment_produced::{EquipmentProduced, ProducedGrouping},
    };
    EquipmentProduced::load(
        source,
        &produced_params(&["GER", "ITA"], None, measure, ProducedGrouping::Type),
        &LoadContext { content: None },
        &AtomicBool::new(false),
    )
    .unwrap()
}

#[test]
fn a_publication_that_cannot_be_ingested_leaves_every_total_unknown_from_then_on() {
    use crate::products::equipment_produced::ProducedMeasure;
    let since = produced_hour(0);
    let lines = [(1, 9001, 0, 10), (2, 21, 0, 10)];
    let rows = |germany: i64, italy: i64| {
        vec![
            (1, 0, 11, 10, germany, true, false),
            (2, 0, 21, 10, italy, true, false),
        ]
    };
    // Day 2's read publishes Italy's line 3, made and deleted since day 1, with 5
    // units, but its country table is not complete, so the publication is not
    // ingested; day 3's no longer lists the line.
    let mut day2 = produced_day(
        2,
        9001,
        since,
        &lines,
        Some(&[rows(2, 2), vec![(3, 0, 21, 10, 5, true, true)]].concat()),
    );
    *day2[0].tables[TableId::Countries.index()] = TableView::Unavailable {
        reason: "fixture".into(),
    };
    let source = produced_source(
        [
            produced_day(0, 9001, since, &lines, Some(&rows(0, 0))),
            produced_day(1, 9001, since, &lines, Some(&rows(1, 1))),
            day2,
            produced_day(3, 9001, since, &lines, Some(&rows(3, 3))),
        ]
        .concat(),
    );
    let reply = produced_load(&source, ProducedMeasure::Total);
    // Italy's total would resume 5 too low: from the missed read on nothing is
    // known, for Italy nor for anyone else, since its producer could not be told.
    for series in &reply.series {
        assert_eq!(
            series.first_hours,
            [0, 1].map(produced_hour).to_vec(),
            "{}",
            series.tag
        );
    }
    assert_eq!(produced_values(&reply.series[1], 0), [Some(0), Some(1)]);
    assert!(
        reply
            .warnings
            .iter()
            .any(|warning| warning.contains("could not be ingested")),
        "{:?}",
        reply.warnings
    );
    // A producer the links cannot tell, once it holds units, does the same.
    let unlinked = produced_source(
        [
            produced_day(0, 9001, since, &lines, Some(&rows(0, 0))),
            produced_day(
                1,
                9001,
                since,
                &lines,
                Some(&[rows(1, 1), vec![(4, 0, 777, 10, 3, true, false)]].concat()),
            ),
        ]
        .concat(),
    );
    let reply = produced_load(&unlinked, ProducedMeasure::Total);
    assert_eq!(reply.series[0].first_hours, [produced_hour(0)].to_vec());
}

#[test]
fn known_values_after_an_unknown_one_survive_and_another_identity_starts_a_new_line() {
    use crate::products::equipment_produced::ProducedMeasure;
    let since = produced_hour(0);
    // Germany's line becomes inexact on day 1; on day 2 another country takes the
    // tag and makes 0, then 2, on its own line.
    let source = produced_source(
        [
            produced_day(
                0,
                9001,
                since,
                &[(1, 11, 0, 10)],
                Some(&[(1, 0, 11, 10, 5, true, false)]),
            ),
            produced_day(
                1,
                9001,
                since,
                &[(1, 11, 0, 10)],
                Some(&[(1, 0, 11, 10, 6, false, false)]),
            ),
            produced_day(
                2,
                9002,
                since,
                &[(1, 11, 0, 10), (2, 11, 0, 10)],
                Some(&[
                    (1, 0, 11, 10, 6, false, false),
                    (2, 0, 11, 10, 0, true, false),
                ]),
            ),
            produced_day(
                3,
                9002,
                since,
                &[(1, 11, 0, 10), (2, 11, 0, 10)],
                Some(&[
                    (1, 0, 11, 10, 6, false, false),
                    (2, 0, 11, 10, 2, true, false),
                ]),
            ),
        ]
        .concat(),
    );
    let reply = produced_load(&source, ProducedMeasure::Total);
    let germany = &reply.series[0];
    assert_eq!(
        produced_values(germany, 0),
        [Some(5), None, Some(0), Some(2)]
    );
    assert_eq!(
        produced_decode(germany.total.as_ref(), 4),
        [Some(5), None, Some(0), Some(2)]
    );
    assert_eq!(germany.breaks, [0, 2]);
}

#[test]
fn a_week_is_known_only_over_unbroken_coverage() {
    use crate::products::equipment_produced::ProducedMeasure;
    let since = produced_hour(0);
    let lines = [(1, 9001, 0, 10)];
    // Two a day, with day 3 missing from the recording.
    let source = produced_source(
        (0..12)
            .filter(|day| *day != 3)
            .flat_map(|day| {
                produced_day(
                    day,
                    9001,
                    since,
                    &lines,
                    Some(&[(1, 0, 11, 10, 2 * i64::from(day), true, false)]),
                )
            })
            .collect(),
    );
    let reply = produced_load(&source, ProducedMeasure::Week);
    let germany = &reply.series[0];
    // Days 7 and 8 have a read a week before, but the gap lies between; day 10's
    // week starts on the missing day; day 11's week is unbroken.
    assert_eq!(germany.first_hours, [produced_hour(11)].to_vec());
    assert_eq!(produced_values(germany, 0), [Some(14)]);
    assert_eq!(
        reply.known,
        vec![crate::products::country_groups::HourSpan {
            first_hour: produced_hour(11),
            last_hour: produced_hour(12) - 1,
        }]
    );
}

#[test]
fn produced_top_n_ranks_every_country_by_the_included_types() {
    use crate::products::{
        LoadContext, Product,
        equipment_produced::{
            EquipmentProduced, EquipmentProducedReply, ProducedGrouping, ProducedMeasure,
        },
    };
    let since = produced_hour(0);
    let lines = [(1, 9001, 0, 10), (2, 21, 0, 10), (3, 21, 0, 30)];
    // Germany leads its infantry equipment; Italy's destroyers are another type.
    let delivered: [(i64, i64, i64); 3] = [(0, 0, 0), (4, 1, 5), (5, 6, 9)];
    let source = produced_source(
        delivered
            .iter()
            .enumerate()
            .flat_map(|(day, &(germany, italy, ships))| {
                produced_day(
                    day as i32,
                    9001,
                    since,
                    &lines,
                    Some(&[
                        (1, 0, 11, 10, germany, true, false),
                        (2, 0, 21, 10, italy, true, false),
                        (3, 0, 21, 30, ships, true, false),
                    ]),
                )
            })
            .collect(),
    );
    let load = |included: &[&str]| {
        let mut params =
            produced_params(&[], Some(1), ProducedMeasure::Total, ProducedGrouping::Type);
        params.included = included.iter().map(|key| (*key).to_owned()).collect();
        EquipmentProduced::load(
            &source,
            &params,
            &LoadContext { content: None },
            &AtomicBool::new(false),
        )
        .unwrap()
    };
    let spans = |reply: &EquipmentProducedReply, tag: &str| {
        reply
            .series
            .iter()
            .find(|series| series.tag == tag)
            .map(|series| {
                series
                    .membership
                    .as_ref()
                    .unwrap()
                    .iter()
                    .map(|span| (span.first_hour, span.last_hour))
                    .collect::<Vec<_>>()
            })
    };
    // Every type: nobody has produced anything on day 0, so nobody ranks (a zero
    // is not ranked); Italy leads from day 1 (1 + 5 > 4), so Germany never ranks.
    let end = |day| produced_hour(day + 1) - 1;
    let all = load(&[]);
    assert_eq!(spans(&all, "GER"), None);
    assert_eq!(spans(&all, "ITA"), Some(vec![(produced_hour(1), end(2))]));
    // Its samples are cut to the hours it was among them.
    let italy = all
        .series
        .iter()
        .find(|series| series.tag == "ITA")
        .unwrap();
    assert_eq!(italy.first_hours, [1, 2].map(produced_hour).to_vec());
    // Infantry equipment only: Germany leads from its first delivery until day 2.
    let infantry = load(&[":infantry_equipment_1"]);
    assert_eq!(
        spans(&infantry, "GER"),
        Some(vec![(produced_hour(1), end(1))])
    );
    assert_eq!(
        spans(&infantry, "ITA"),
        Some(vec![(produced_hour(2), end(2))])
    );
    assert_eq!(infantry.keys.len(), 1);
}
