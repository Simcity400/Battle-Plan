use super::test_support::{appearance, frame, map, occupied, province, strategic_regions};
use super::*;
use viewer_gpu::gradient_math::FieldDefines;
use viewer_foundation::tables::limitations as tables_limitations;
use viewer_scene_base::gradient::{self, Layer, LayerKind, LayerPlane};
use viewer_gpu::province_map::ProvinceMap;

struct OracleLayers {
    textures: gradient::GradientTextures,
    regions: Regions,
}
fn build_oracle(
    map: &MapData,
    frame: &DecodedFrame,
    strategic: &LayerPlane,
    field: &FieldDefines,
) -> io::Result<OracleLayers> {
    let regions = super::oracle::regions(map, frame)?;
    let color = |key| regions.country_color(key);
    let country = gradient::build_layer(
        &ProvinceMap {
            width: map.width,
            height: map.height,
            ids: &map.province_ids,
        },
        &Layer::new(LayerKind::Country, &regions.country_of, &color, field),
    )?;
    Ok(OracleLayers {
        textures: gradient::stack(strategic, &country)?,
        regions,
    })
}

use content_reader_map::map::ProvinceDefinition;
use replay_data::{
    rows::AutonomyRow, rows::CountryLinkRow, rows::TableId,
    rows::TableRows,
};
use replay_frame::frame::TableView;

const GER: [u8; 3] = [191, 127, 63];
const SOV: [u8; 3] = [127, 0, 63];

#[test]
fn naval_control_attributes_use_the_same_nonzero_keys_as_the_region_field() {
    let attributes = map_display_data::landscape::Attributes {
        control: 255,
        contested: 128,
        gradient: None,
    };
    let fill = super::super::naval_dominance::Fill {
        attributes: BTreeMap::from([(4, attributes)]),
        warnings: Vec::new(),
    };
    let built = build_with_naval(&map(), &occupied(), fill, Default::default()).unwrap();
    assert_eq!(
        built.regions.strategic_attributes,
        BTreeMap::from([(5, attributes)])
    );
    assert_eq!(
        strategic_regions_of(&map(), &strategic_regions()).unwrap()[5],
        5
    );
}

fn strategic() -> LayerPlane {
    {
        let map = map();
        let keys = strategic_regions_of(&map, &strategic_regions()).unwrap();
        let color = |_: u32| FieldDefines::TEST.navy_neutral;
        let mut layer = Layer::new(
            LayerKind::Fixed(FieldDefines::TEST.strategic_thickness),
            &keys,
            &color,
            &FieldDefines::TEST,
        );
        layer.region_opacity = 0;
        layer.no_region_color = FieldDefines::TEST.navy_neutral;
        gradient::build_layer(
            &ProvinceMap {
                width: map.width,
                height: map.height,
                ids: &map.province_ids,
            },
            &layer,
        )
        .unwrap()
    }
}

#[test]
fn owner_aliases_share_a_border_key_and_links_invalidate_the_cache() {
    let mut frame = occupied();
    let original = FrameKey::new(&frame).unwrap();
    let TableView::Complete(TableRows::CountryLinks(links)) =
        &mut *frame.tables[TableId::CountryLinks.index()]
    else {
        unreachable!()
    };
    links.push(CountryLinkRow {
        id: 101,
        canonical_slot: 0,
    });
    assert_ne!(original, FrameKey::new(&frame).unwrap());
    let TableView::Complete(TableRows::Provinces(provinces)) =
        &mut *frame.tables[TableId::Provinces.index()]
    else {
        unreachable!()
    };
    provinces[2].owner = 101;
    let regions = regions(&map(), &frame).unwrap();
    assert_eq!(regions.owner_of[1], regions.owner_of[3]);
    assert_ne!(regions.country_of[1], regions.country_of[3]);
}

#[test]
fn the_fill_takes_the_controllers_recorded_colour_as_stored() {
    let regions = regions(&map(), &occupied()).unwrap();
    assert_eq!(regions.state_of[1], 10);
    assert_eq!(regions.state_of[3], 11);
    assert_eq!(regions.country_colors[&1], GER);
    assert_eq!(regions.country_colors[&2], SOV);
}

#[test]
fn a_country_without_a_recorded_colour_keeps_its_region_grey_and_is_named() {
    let mut frame = occupied();
    *frame.tables[TableId::CountryAppearance.index()] = TableView::Truncated {
        rows: TableRows::CountryAppearance(vec![appearance(100, [0.75, 0.5, 0.25])]),
        missing: 1,
    };
    let regions = regions(&map(), &frame).unwrap();
    assert_eq!(regions.country_of[1], 1);
    assert_eq!(
        regions.country_of[3], 2,
        "the gap is scoped to the country, not blank land"
    );
    assert_eq!(regions.country_colors[&2], UNAVAILABLE_COLOR);
    assert_eq!(regions.owner_of[3], 1, "borders keep the owner");
    assert!(regions.warnings[0].contains("truncated (1 missing rows)"));
    assert!(regions.warnings[0].contains("SOV"));
}

#[test]
fn an_unresolved_reference_is_named_and_takes_no_fill() {
    let frame = frame(vec![
        province(1, 10, 0, 0),
        province(2, 10, 100, 100),
        province(3, 11, 100, 100),
        province(4, 11, 100, 100),
        province(5, 0, 0, 0),
    ]);
    let regions = regions(&map(), &frame).unwrap();
    assert_eq!(regions.country_of[1], NO_REGION);
    assert_eq!(regions.state_of[1], 10);
    assert!(regions.warnings[0].contains("zero controller"));
    assert!(regions.warnings[0].contains("province ids 1"));
}

#[test]
fn a_truncated_non_core_table_still_renders_the_political_layer_and_is_stated() {
    let mut frame = occupied();
    *frame.tables[TableId::ProvinceBuildings.index()] = TableView::Truncated {
        rows: TableRows::ProvinceBuildings(Vec::new()),
        missing: 1174,
    };
    let degraded = build_oracle(&map(), &frame, &strategic(), &FieldDefines::TEST).unwrap();
    let whole = build_oracle(&map(), &occupied(), &strategic(), &FieldDefines::TEST).unwrap();
    assert_eq!(degraded.textures.channel1, whole.textures.channel1);
    assert_eq!(degraded.textures.channel2, whole.textures.channel2);
    assert_eq!(degraded.regions.country_of, whole.regions.country_of);
    let limitations = tables_limitations(&frame);
    assert!(
        limitations.lines().iter().any(|line| {
            line.starts_with("Table ProvinceBuildings truncated (1174 missing rows): ")
                && line.contains("naval base")
        }),
        "{:?}",
        limitations.lines()
    );
}

#[test]
fn an_unavailable_core_table_is_a_named_error() {
    let mut frame = occupied();
    *frame.tables[TableId::Countries.index()] = TableView::Unavailable {
        reason: "the collector had not grown its buffer yet".into(),
    };
    let error = regions(&map(), &frame).unwrap_err().to_string();
    assert!(
        error.contains("hour 42 Countries is unavailable"),
        "{error}"
    );
    assert!(
        error.contains("the collector had not grown its buffer yet"),
        "{error}"
    );
    assert!(
        error.contains("Provinces, CountryLinks and Countries"),
        "{error}"
    );
    assert!(
        !tables_limitations(&frame)
            .lines()
            .iter()
            .any(|line| line.contains("Table Countries"))
    );
}

#[test]
fn dangling_links_are_named_errors() {
    let mut frame = occupied();
    *frame.tables[TableId::CountryLinks.index()] =
        TableView::Complete(TableRows::CountryLinks(Vec::new()));
    assert!(
        regions(&map(), &frame)
            .err()
            .unwrap()
            .to_string()
            .contains("references missing country link")
    );
    let mut frame = occupied();
    *frame.tables[TableId::Provinces.index()] = TableView::Truncated {
        rows: TableRows::Provinces(Vec::new()),
        missing: 3,
    };
    assert!(regions(&map(), &frame).is_err());
}

#[test]
fn an_undefined_recorded_province_is_refused() {
    let frame = frame(vec![province(99, 10, 100, 100)]);
    assert!(
        regions(&map(), &frame)
            .err()
            .unwrap()
            .to_string()
            .contains("province 99 is absent from map/definition.csv")
    );
}

#[test]
fn a_map_province_the_hour_never_mentions_is_refused() {
    let mut rows = match &*occupied().tables[TableId::Provinces.index()] {
        TableView::Complete(TableRows::Provinces(rows)) => rows.clone(),
        _ => unreachable!("the fixture builds a complete table"),
    };
    rows.pop();
    assert!(
        regions(&map(), &frame(rows))
            .err()
            .unwrap()
            .to_string()
            .contains("no recorded row for map province 5")
    );
}

#[test]
fn subject_colour_switch_preserves_regions_and_invalidates_the_cache() {
    let mut map = map();
    map.country_colors.use_overlord_color =
        [(b"puppet".to_vec(), false), (b"union".to_vec(), true)].into();
    let mut frame = occupied();
    let mut subject = SubjectRow {
        subject: 200,
        overlord: 100,
        state: b"puppet".to_vec(),
        ..Default::default()
    };
    let set = |frame: &mut DecodedFrame, subject: &SubjectRow| {
        *frame.tables[TableId::Subjects.index()] =
            TableView::Complete(TableRows::Subjects(vec![subject.clone()]));
        *frame.tables[TableId::Autonomy.index()] =
            TableView::Complete(TableRows::Autonomy(vec![AutonomyRow {
                subject: subject.subject,
                state: subject.state.clone(),
                ..Default::default()
            }]));
    };
    set(&mut frame, &subject);
    let puppet = FrameKey::new(&frame).unwrap();
    let own = super::regions(&map, &frame).unwrap().country_colors[&2];
    assert_eq!(own, SOV);
    subject.state = b"union".to_vec();
    frame.hour += 1;
    set(&mut frame, &subject);
    if let TableView::Complete(TableRows::Subjects(rows)) =
        &mut *frame.tables[TableId::Subjects.index()]
    {
        rows[0].state = b"puppet".to_vec();
    }

    let union = FrameKey::new(&frame).unwrap();
    assert_ne!(puppet, union);
    let regions = super::regions(&map, &frame).unwrap();
    assert_eq!(regions.country_colors[&2], GER);
    assert_ne!(
        regions.country_of[1], regions.country_of[3],
        "colour inheritance does not merge border regions"
    );
    subject.autonomy_ratio = 75_000;
    frame.hour += 1;
    set(&mut frame, &subject);
    assert_eq!(
        regions.country_colors[&2],
        super::regions(&map, &frame).unwrap().country_colors[&2]
    );
    assert_eq!(
        FrameKey::new(&frame).unwrap(),
        union,
        "autonomy progress does not change the colour source"
    );
    subject.cancel = 1;
    set(&mut frame, &subject);
    *frame.tables[TableId::Autonomy.index()] = TableView::Complete(TableRows::Autonomy(Vec::new()));
    assert_ne!(FrameKey::new(&frame).unwrap(), union);
    assert_eq!(
        super::regions(&map, &frame).unwrap().country_colors[&2],
        own
    );
}

#[test]
fn inheritance_chains_use_the_final_source_and_reject_cycles() {
    let mut map = map();
    map.country_colors
        .use_overlord_color
        .insert(b"union".to_vec(), true);
    let mut frame = occupied();
    let TableView::Complete(TableRows::Countries(countries)) =
        &mut *frame.tables[TableId::Countries.index()]
    else {
        unreachable!()
    };
    countries.push(CountryRow {
        id: 300,
        canonical_slot: 2,
        tag: b"NEW".to_vec(),
        ..Default::default()
    });
    let TableView::Complete(TableRows::CountryLinks(links)) =
        &mut *frame.tables[TableId::CountryLinks.index()]
    else {
        unreachable!()
    };
    links.push(CountryLinkRow {
        id: 300,
        canonical_slot: 2,
    });
    *frame.tables[TableId::Autonomy.index()] = TableView::Complete(TableRows::Autonomy(
        [100, 200, 300]
            .map(|subject| AutonomyRow {
                subject,
                state: b"union".to_vec(),
                ..Default::default()
            })
            .to_vec(),
    ));
    let relation = |subject, overlord| SubjectRow {
        subject,
        overlord,
        state: b"union".to_vec(),
        ..Default::default()
    };
    *frame.tables[TableId::Subjects.index()] = TableView::Complete(TableRows::Subjects(vec![
        relation(200, 300),
        relation(300, 100),
    ]));
    let regions = super::regions(&map, &frame).unwrap();
    assert_eq!(regions.country_colors[&1], regions.country_colors[&2]);
    *frame.tables[TableId::Subjects.index()] = TableView::Complete(TableRows::Subjects(vec![
        relation(200, 100),
        relation(100, 200),
    ]));
    assert!(
        super::regions(&map, &frame)
            .unwrap_err()
            .to_string()
            .contains("cycle")
    );
}

#[test]
fn incomplete_inheritance_inputs_leave_the_colour_source_undecided() {
    let mut map = map();
    map.country_colors.use_overlord_color =
        [(b"puppet".to_vec(), false), (b"union".to_vec(), true)].into();
    let links: BTreeMap<u32, u32> = [(100, 1), (200, 2)].into();
    let status = |state: &[u8]| AutonomyRow {
        subject: 200,
        state: state.to_vec(),
        ..Default::default()
    };
    let relation = SubjectRow {
        subject: 200,
        overlord: 100,
        ..Default::default()
    };
    let with = |autonomy: TableView, subjects: TableView| {
        let mut frame = occupied();
        *frame.tables[TableId::Autonomy.index()] = autonomy;
        *frame.tables[TableId::Subjects.index()] = subjects;
        super::inheritance(&map, &frame, &links).unwrap()
    };
    let complete_subjects = || TableView::Complete(TableRows::Subjects(vec![relation.clone()]));
    let union = with(
        TableView::Complete(TableRows::Autonomy(vec![status(b"union")])),
        complete_subjects(),
    );
    assert_eq!(union.source(2), Ok(1));
    assert_eq!(union.source(1), Ok(1));
    let puppet = with(
        TableView::Complete(TableRows::Autonomy(vec![status(b"puppet")])),
        complete_subjects(),
    );
    assert_eq!(puppet.source(2), Ok(2));
    let modded = with(
        TableView::Complete(TableRows::Autonomy(vec![status(b"dominion")])),
        complete_subjects(),
    );
    assert!(modded.source(2).unwrap_err().contains("dominion"));
    let truncated = with(
        TableView::Truncated {
            rows: TableRows::Autonomy(Vec::new()),
            missing: 1,
        },
        complete_subjects(),
    );
    let why = truncated.source(2).unwrap_err();
    assert!(
        why.contains("current autonomy status") && why.contains("Autonomy is truncated"),
        "{why}"
    );
    let unavailable = with(
        TableView::Complete(TableRows::Autonomy(vec![status(b"union")])),
        TableView::Unavailable {
            reason: "walk failed".into(),
        },
    );
    assert!(
        unavailable
            .source(2)
            .unwrap_err()
            .contains("Subjects is unavailable: walk failed")
    );
    let orphan = with(
        TableView::Complete(TableRows::Autonomy(vec![status(b"union")])),
        TableView::Complete(TableRows::Subjects(Vec::new())),
    );
    assert!(orphan.source(2).unwrap_err().contains("overlord"));
    let mut frame = occupied();
    *frame.tables[TableId::Autonomy.index()] =
        TableView::Complete(TableRows::Autonomy(vec![status(b"dominion")]));
    let mut warnings = Vec::new();
    let sources = super::inherited_colors(&map, &frame, &links, &mut warnings).unwrap();
    assert!(sources.inherited.is_empty());
    assert!(
        warnings
            .iter()
            .any(|line| line.contains("dominion") && line.contains("withheld"))
    );
}

#[test]
fn the_frame_key_changes_with_control_and_colour_but_not_the_hour() {
    let key = FrameKey::new(&occupied()).unwrap();
    let mut later = occupied();
    later.hour += 1;
    assert_eq!(FrameKey::new(&later).unwrap(), key);
    let liberated = frame(vec![
        province(1, 10, 100, 100),
        province(2, 10, 100, 100),
        province(3, 11, 100, 100),
        province(4, 11, 100, 100),
        province(5, 0, 0, 0),
    ]);
    assert_ne!(FrameKey::new(&liberated).unwrap(), key);
    let mut recoloured = occupied();
    *recoloured.tables[TableId::CountryAppearance.index()] = TableView::Complete(
        TableRows::CountryAppearance(vec![appearance(100, [0.75, 0.5, 0.25])]),
    );
    assert_ne!(FrameKey::new(&recoloured).unwrap(), key);
}

#[test]
fn the_built_textures_paint_the_country_layer_second_and_the_strategic_one_first() {
    let layers = build_oracle(&map(), &occupied(), &strategic(), &FieldDefines::TEST).unwrap();
    let textures = &layers.textures;
    assert_eq!(textures.width, 3);
    assert_eq!(textures.layer_rows(), 1);
    assert_eq!(textures.height, 4);
    assert!(
        (0..textures.width)
            .filter_map(|x| textures.channel1_texel(1, x, 0))
            .any(|texel| texel[3] > 0 && texel[..3] != [0, 0, 0])
    );
    let sea = textures.channel1_texel(0, 2, 0).unwrap();
    assert_eq!(sea, [51, 63, 89, 251]);
    for x in 0..textures.width {
        assert_eq!(textures.channel2_texel(0, x, 0).unwrap()[1], 0);
    }
    assert_eq!(
        textures.channel1_texel(0, 0, 0),
        Some([51, 63, 89, gradient::NO_REGION_ALPHA]),
        "land is in no region of layer 1 and still carries its colour"
    );
    assert_eq!(textures.channel1_texel(1, 2, 0), Some([191, 127, 63, 251]));
    assert_eq!(textures.channel2_texel(1, 2, 0), Some([0, 255, 0, 0]));
    assert_eq!(layers.regions.state_of[1], 10);
    assert_eq!(layers.regions.state_of[3], 11);
    for x in 0..textures.width {
        let member = textures.channel1_texel(1, x, 0).unwrap()[3] != gradient::NO_REGION_ALPHA;
        let green = textures.channel2_texel(1, x, 0).unwrap()[1];
        assert_eq!(member, green > 0, "column {x}");
        assert_eq!(
            textures.channel2_texel(1, x, 0).unwrap()[2],
            0,
            "political mode never pulses a layer"
        );
    }
}

#[test]
fn resolved_territory_matches_frozen_owner_controller_colour_and_refusal_reference() {
    let map = map();
    let mut hour = frame(vec![
        province(1, 1, 100, 100),
        province(2, 1, 100, 200),
        province(3, 2, 200, 200),
        province(4, 2, 200, 0),
        province(5, 0, 0, 0),
    ]);
    for table in [
        None,
        Some(TableId::CountryAppearance),
        Some(TableId::Subjects),
        Some(TableId::CountryLinks),
    ] {
        let saved = table.map(|table| (table, hour.tables[table.index()].clone()));
        if let Some(table) = table {
            hour.tables[table.index()] = TableView::Unavailable {
                reason: format!("independent territory poison {table:?}"),
            }
            .into();
        }
        let (actual, expected) = (regions(&map, &hour), super::oracle::regions(&map, &hour));
        match (actual, expected) {
            (Ok(actual), Ok(expected)) => assert_eq!(
                product_codec::encode(&actual).unwrap(),
                product_codec::encode(&expected).unwrap()
            ),
            (Err(actual), Err(expected)) => assert_eq!(
                (actual.kind(), actual.to_string()),
                (expected.kind(), expected.to_string())
            ),
            values => panic!("territory candidate/reference mismatch: {values:?}"),
        }
        if let Some((table, view)) = saved {
            hour.tables[table.index()] = view;
        }
    }
}

#[test]
fn territory_changed_rows_match_frozen_regions_and_keep_untouched_planes() {
    use replay_frame::frame::{FrameDecoder, FrameEncoder};
    let map = Arc::new(map());
    let mut source = occupied();
    source.tables[TableId::Campaign.index()] = viewer_scene_base::test_support::test_campaign();
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::new().with_change_facts();
    let mut fold = TerritoryFold::default();
    let raw = encoder.encode(source.clone(), true).unwrap();
    let frame = decoder.advance(&raw).unwrap();
    let mut previous = fold.read_shared(&map, frame).unwrap();
    assert_eq!(
        *previous.paint,
        super::oracle::regions(&map, frame).unwrap()
    );
    for step in 0..4 {
        source.hour += 1;
        match step {
            0 => {
                let TableView::Complete(TableRows::CountryAppearance(rows)) =
                    &mut *source.tables[TableId::CountryAppearance.index()]
                else {
                    unreachable!()
                };
                rows[0].red = 0.125f32.to_bits();
            }
            1 => {
                let TableView::Complete(TableRows::Provinces(rows)) =
                    &mut *source.tables[TableId::Provinces.index()]
                else {
                    unreachable!()
                };
                rows[0].controller = 200;
            }
            2 => {
                let TableView::Complete(TableRows::Countries(rows)) =
                    &mut *source.tables[TableId::Countries.index()]
                else {
                    unreachable!()
                };
                rows.push(CountryRow {
                    id: 300,
                    canonical_slot: 2,
                    tag: b"NEW".to_vec(),
                    ..Default::default()
                });
            }
            _ => {
                let TableView::Complete(TableRows::Countries(rows)) =
                    &mut *source.tables[TableId::Countries.index()]
                else {
                    unreachable!()
                };
                rows.pop();
            }
        }
        let raw = encoder.encode(source.clone(), false).unwrap();
        let frame = decoder.advance(&raw).unwrap();
        let next = fold.read_shared(&map, frame).unwrap();
        assert_eq!(*next.paint, super::oracle::regions(&map, frame).unwrap());
        if step == 0 {
            assert!(Arc::ptr_eq(
                &previous.paint.country_of.0,
                &next.paint.country_of.0
            ));
        }
        assert!(Arc::ptr_eq(
            &previous.paint.owner_of.0,
            &next.paint.owner_of.0
        ));
        assert!(Arc::ptr_eq(
            &previous.paint.state_of.0,
            &next.paint.state_of.0
        ));
        assert_eq!(next.countries.get(&0).unwrap().native, 100);
        assert_eq!(
            next.countries.get(&2).map(|country| country.native),
            (step == 2).then_some(300)
        );
        assert!(
            next.countries
                .iter()
                .all(|(_, country)| country.observed_hour == source.hour)
        );
        previous = next;
    }
    drop(decoder);
    let source = source
        .tables
        .into_iter()
        .all(|table| table.try_unwrap().is_ok());
    assert!(source);
}
#[test]
fn territory_refusal_uses_first_province_before_missing_map_rows() {
    let mut map = map();
    map.country_colors
        .use_overlord_color
        .insert(b"inherited".to_vec(), true);
    let mut frame = occupied();
    let TableView::Complete(TableRows::CountryLinks(links)) =
        &mut *frame.tables[TableId::CountryLinks.index()]
    else {
        unreachable!()
    };
    links.push(CountryLinkRow {
        id: 300,
        canonical_slot: 2,
    });
    frame.tables[TableId::Subjects.index()] =
        TableView::Complete(TableRows::Subjects(vec![SubjectRow {
            subject: 100,
            overlord: 300,
            ..Default::default()
        }]))
        .into();
    frame.tables[TableId::Autonomy.index()] =
        TableView::Complete(TableRows::Autonomy(vec![AutonomyRow {
            subject: 100,
            state: b"inherited".to_vec(),
            ..Default::default()
        }]))
        .into();
    let TableView::Complete(TableRows::Provinces(rows)) =
        &mut *frame.tables[TableId::Provinces.index()]
    else {
        unreachable!()
    };
    rows[2].owner = 999;
    rows.pop();
    let expected = super::oracle::regions(&map, &frame).unwrap_err();
    let actual = TerritoryFold::default().read(&map, &frame).unwrap_err();
    assert_eq!(actual.kind(), expected.kind());
    assert_eq!(actual.to_string(), expected.to_string());
}

#[test]
fn undecided_colour_sources_are_withheld_not_guessed() {
    use replay_frame::frame::{FrameDecoder, FrameEncoder};
    let mut map = map();
    map.country_colors.use_overlord_color =
        [(b"puppet".to_vec(), false), (b"union".to_vec(), true)].into();
    let map = Arc::new(map);
    let mut source = occupied();
    source.tables[TableId::Campaign.index()] = viewer_scene_base::test_support::test_campaign();
    source.tables[TableId::Subjects.index()] =
        TableView::Complete(TableRows::Subjects(vec![SubjectRow {
            subject: 200,
            overlord: 100,
            ..Default::default()
        }]))
        .into();
    let status = |state: &[u8]| {
        TableView::Complete(TableRows::Autonomy(vec![AutonomyRow {
            subject: 200,
            state: state.to_vec(),
            ..Default::default()
        }]))
    };
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::new().with_change_facts();
    let mut fold = TerritoryFold::default();
    let steps: [(TableView, [u8; 3], [u8; 3], Option<&str>); 5] = [
        (status(b"union"), GER, GER, None),
        (
            status(b"dominion"),
            GER,
            UNAVAILABLE_COLOR,
            Some("dominion"),
        ),
        (
            TableView::Unavailable {
                reason: "walk failed".into(),
            },
            UNAVAILABLE_COLOR,
            UNAVAILABLE_COLOR,
            Some("Autonomy is unavailable: walk failed"),
        ),
        (status(b"puppet"), GER, SOV, None),
        (status(b"union"), GER, GER, None),
    ];
    for (step, (autonomy, germany, soviet, why)) in steps.into_iter().enumerate() {
        source.hour = 42 + step as i32;
        source.tables[TableId::Autonomy.index()] = autonomy.into();
        let raw = encoder.encode(source.clone(), step == 0).unwrap();
        let frame = decoder.advance(&raw).unwrap();
        let next = fold.read_shared(&map, frame).unwrap();
        assert_eq!(
            *next.paint,
            super::oracle::regions(&map, frame).unwrap(),
            "step {step}"
        );
        assert_eq!(next.paint.country_colors[&1], germany, "step {step}");
        assert_eq!(next.paint.country_colors[&2], soviet, "step {step}");
        match why {
            Some(why) => assert!(
                next.paint
                    .warnings
                    .iter()
                    .any(|line| line.contains(why) && line.contains("withheld")),
                "step {step}: {:?}",
                next.paint.warnings
            ),
            None => assert!(next.paint.warnings.is_empty(), "step {step}"),
        }
    }
}

#[test]
fn territory_relation_delta_boundaries_and_link_aliases_match_independent_oracle() {
    use replay_frame::frame::{FrameDecoder, FrameEncoder};
    let mut map = map();
    map.country_colors
        .use_overlord_color
        .insert(b"inherited".to_vec(), true);
    map.country_colors
        .use_overlord_color
        .insert(b"own".to_vec(), false);
    let map = Arc::new(map);
    let mut source = occupied();
    source.hour = 10;
    source.tables[TableId::Campaign.index()] = viewer_scene_base::test_support::test_campaign();
    source.tables[TableId::Subjects.index()] =
        TableView::Complete(TableRows::Subjects(vec![SubjectRow {
            subject: 100,
            overlord: 200,
            start_hour: 0,
            end_hour: 12,
            ..Default::default()
        }]))
        .into();
    source.tables[TableId::Autonomy.index()] =
        TableView::Complete(TableRows::Autonomy(vec![AutonomyRow {
            subject: 100,
            state: b"inherited".to_vec(),
            ..Default::default()
        }]))
        .into();
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::new().with_change_facts();
    let mut fold = TerritoryFold::default();
    let mut previous: Option<Arc<Regions>> = None;
    for step in 0..9 {
        source.hour = 10 + step;
        match step {
            3 => {
                let TableView::Complete(TableRows::Subjects(rows)) =
                    &mut *source.tables[TableId::Subjects.index()]
                else {
                    unreachable!()
                };
                rows[0].end_hour = 20;
            }
            4 => {
                let TableView::Complete(TableRows::Autonomy(rows)) =
                    &mut *source.tables[TableId::Autonomy.index()]
                else {
                    unreachable!()
                };
                rows[0].state = b"undefined".to_vec();
            }
            5 => {
                let TableView::Complete(TableRows::Autonomy(rows)) =
                    &mut *source.tables[TableId::Autonomy.index()]
                else {
                    unreachable!()
                };
                rows[0].state = b"inherited".to_vec();
                rows.push(AutonomyRow {
                    subject: 300,
                    state: b"own".to_vec(),
                    ..Default::default()
                });
                let TableView::Complete(TableRows::CountryLinks(rows)) =
                    &mut *source.tables[TableId::CountryLinks.index()]
                else {
                    unreachable!()
                };
                rows.push(CountryLinkRow {
                    id: 300,
                    canonical_slot: 0,
                });
            }
            6 => {
                let TableView::Complete(TableRows::CountryLinks(rows)) =
                    &mut *source.tables[TableId::CountryLinks.index()]
                else {
                    unreachable!()
                };
                rows.pop();
            }
            7 => {
                source.tables[TableId::Autonomy.index()] = TableView::Truncated {
                    rows: TableRows::Autonomy(Vec::new()),
                    missing: 2,
                }
                .into();
            }
            8 => {
                source.tables[TableId::Autonomy.index()] =
                    TableView::Complete(TableRows::Autonomy(Vec::new())).into();
            }
            _ => {}
        }
        let raw = encoder.encode(source.clone(), step == 0).unwrap();
        let frame = decoder.advance(&raw).unwrap();
        match (
            fold.read_shared(&map, frame),
            super::oracle::regions(&map, frame),
        ) {
            (Ok(actual), Ok(expected)) => {
                assert_eq!(*actual.paint, expected, "step {step}");
                if let Some(previous) = &previous {
                    for (old, new) in [
                        (&previous.country_of, &actual.paint.country_of),
                        (&previous.owner_of, &actual.paint.owner_of),
                        (&previous.state_of, &actual.paint.state_of),
                    ] {
                        assert!(Arc::ptr_eq(&old.0, &new.0), "step {step}");
                    }
                }
                previous = Some(Arc::clone(&actual.paint));
            }
            (Err(actual), Err(expected)) => assert_eq!(
                (actual.kind(), actual.to_string()),
                (expected.kind(), expected.to_string()),
                "step {step}"
            ),
            (actual, expected) => panic!(
                "step {step}: {} / {expected:?}",
                if actual.is_ok() { "value" } else { "error" }
            ),
        }
    }
}

#[test]
fn batched_territory_indexes_keep_repeated_links_counts_and_sorted_planes_exact() {
    use replay_frame::frame::{FrameDecoder, FrameEncoder};
    let mut map = map();
    let mut source = occupied();
    let TableView::Complete(TableRows::Provinces(provinces)) =
        &mut *source.tables[TableId::Provinces.index()]
    else { unreachable!() };
    for id in 8..=1508 {
        map.definitions.insert(id, ProvinceDefinition {
            id, rgb: [0; 3], kind: ProvinceKind::Land, continent: 0,
        });
        provinces.push(ProvinceRow {
            id, state: 1, owner: 100, controller: 100, ..Default::default()
        });
    }
    source.tables[TableId::Campaign.index()] = viewer_scene_base::test_support::test_campaign();
    let map = Arc::new(map);
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::new().with_change_facts();
    let mut fold = TerritoryFold::default();
    let mut previous: Option<Arc<Regions>> = None;
    for step in 0..4 {
        if step != 0 {
            source.hour += 1;
            let TableView::Complete(TableRows::Provinces(provinces)) =
                &mut *source.tables[TableId::Provinces.index()]
            else { unreachable!() };
            for row in provinces.iter_mut().filter(|row| row.id >= 8) {
                row.controller = if step == 1 { 200 } else { 100 };
                row.owner = if step == 2 { 200 } else { 100 };
            }
        }
        let update = encoder.encode(source.clone(), step == 0).unwrap();
        let frame = decoder.advance(&update).unwrap();
        let actual = fold.read_shared(&map, frame).unwrap();
        assert_eq!(*actual.paint, super::oracle::regions(&map, frame).unwrap());
        assert_eq!(actual.provinces.0.len(), core::<tables::Provinces>(frame).unwrap().len());
        if let Some(previous) = &previous {
            assert_eq!(
                [
                    Arc::ptr_eq(&previous.country_of.0, &actual.paint.country_of.0),
                    Arc::ptr_eq(&previous.owner_of.0, &actual.paint.owner_of.0),
                    Arc::ptr_eq(&previous.state_of.0, &actual.paint.state_of.0),
                ],
                [step == 3, step == 1, true],
                "step {step}"
            );
        }
        previous = Some(Arc::clone(&actual.paint));
    }
}
