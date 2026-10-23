use super::*;
use viewer_foundation::map::gradient_math::FieldDefines;
use viewer_foundation::map::tables::limitations as tables_limitations;
use {
    super::super::gradient::{self, Layer, LayerKind, LayerPlane},
    viewer_foundation::map::province_map::ProvinceMap,
};

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
    let regions = regions(map, frame)?;
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

use installed_content_core::{map::CountryColors, map::ProvinceDefinition};
use replay_data::{
    rows::AutonomyRow, rows::CountryAppearanceRow, rows::CountryLinkRow, rows::TableId,
    rows::TableRows,
};
use replay_frame::{frame::Metrics, frame::TableView};

/// Recorded colours: `0.75, 0.5, 0.25` truncate to these bytes.
const GER: [u8; 3] = [191, 127, 63];
const SOV: [u8; 3] = [127, 0, 63];

fn appearance(country: u32, rgb: [f32; 3]) -> CountryAppearanceRow {
    CountryAppearanceRow {
        country,
        red: rgb[0].to_bits(),
        green: rgb[1].to_bits(),
        blue: rgb[2].to_bits(),
        alpha: 1.0f32.to_bits(),
        ..Default::default()
    }
}

/// The strategic regions of the fixture map: the one sea province is region 4.
fn strategic_regions() -> BTreeMap<u32, StrategicRegion> {
    BTreeMap::from([(
        4,
        StrategicRegion {
            id: 4,
            name_key: "STRATEGICREGION_4".into(),
            provinces: vec![5],
            naval_terrain: None,
            static_modifiers: false,
            static_modifier_names: Vec::new(),
            ..Default::default()
        },
    )])
}

#[test]
fn naval_control_attributes_use_the_same_nonzero_keys_as_the_region_field() {
    let attributes = gradient::Attributes {
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

/// Layer 1 for the fixture map, which no hour changes.
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

/// Four land provinces in two states plus one sea province, in a 6 x 2 image.
fn map() -> MapData {
    let kinds = [
        (0, ProvinceKind::Land),
        (1, ProvinceKind::Land),
        (2, ProvinceKind::Land),
        (3, ProvinceKind::Land),
        (4, ProvinceKind::Land),
        (5, ProvinceKind::Sea),
    ];
    MapData {
        width: 6,
        height: 2,
        province_ids: vec![1, 1, 2, 3, 4, 5, 1, 1, 2, 3, 4, 5],
        definitions: kinds
            .into_iter()
            .map(|(id, kind)| {
                (
                    id,
                    ProvinceDefinition {
                        id,
                        rgb: [id as u8, 0, 0],
                        kind,
                        continent: 0,
                    },
                )
            })
            .collect(),
        country_colors: CountryColors::default(),
    }
}

fn province(id: u32, state: u32, owner: u32, controller: u32) -> ProvinceRow {
    ProvinceRow {
        id,
        state,
        owner,
        controller,
        victory_points: 0,
        ..Default::default()
    }
}

fn frame(rows: Vec<ProvinceRow>) -> DecodedFrame {
    let mut tables = std::array::from_fn(|_| TableView::Unavailable {
        reason: "not needed for the political layers".into(),
    });
    tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(vec![
        CountryRow {
            id: 100,
            canonical_slot: 0,
            tag: b"GER".to_vec(),
            ..Default::default()
        },
        CountryRow {
            id: 200,
            canonical_slot: 1,
            tag: b"SOV".to_vec(),
            ..Default::default()
        },
    ]));
    tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(vec![
        CountryLinkRow {
            id: 100,
            canonical_slot: 0,
        },
        CountryLinkRow {
            id: 200,
            canonical_slot: 1,
        },
    ]));
    tables[TableId::Provinces.index()] = TableView::Complete(TableRows::Provinces(rows));
    tables[TableId::CountryAppearance.index()] =
        TableView::Complete(TableRows::CountryAppearance(vec![
            appearance(100, [0.75, 0.5, 0.25]),
            appearance(200, [0.5, 0.0, 0.25]),
        ]));
    DecodedFrame {
        branch: 1,
        hour: 42,
        tables: tables.map(Into::into),
        metrics: Metrics::default(),
    }
}

/// Germany owns everything; the Soviets control province 3 in state 11.
fn occupied() -> DecodedFrame {
    frame(vec![
        province(1, 10, 100, 100),
        province(2, 10, 100, 100),
        province(3, 11, 100, 200),
        province(4, 11, 100, 100),
        province(5, 0, 0, 0),
    ])
}

#[test]
fn political_fill_uses_controller_and_borders_use_owner() {
    let map = map();
    let frame = occupied();
    let control = regions(&map, &frame).unwrap();
    // Slot plus one: Germany is 1 and the Soviet Union 2.
    assert_eq!(control.country_of[1], 1);
    assert_eq!(control.country_of[3], 2);
    assert_ne!(control.country_of[1], control.country_of[3]);
    assert_eq!(control.owner_of[3], 1);
    assert_eq!(
        super::super::border_pass::land_class(&control, 1, 3),
        super::super::borders::BorderClass::State
    );
    // The fill leaves the sea alone.
    assert_eq!(control.country_of[5], NO_REGION);
    assert_eq!(control.state_of[5], NO_REGION);
    assert!(control.warnings.is_empty());

    // An owned lake keeps exactly the same region as a land province;
    // an unassigned lake remains outside, without a missing-owner warning.
    let mut lakes = map;
    lakes.definitions.get_mut(&3).unwrap().kind = ProvinceKind::Lake;
    lakes.definitions.get_mut(&5).unwrap().kind = ProvinceKind::Lake;
    let lake_control = regions(&lakes, &frame).unwrap();
    assert_eq!(lake_control.country_of, control.country_of);
    assert_eq!(lake_control.owner_of, control.owner_of);
    assert!(lake_control.warnings.is_empty());
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
    // The state key survives: layer 2 does not depend on ownership.
    assert_eq!(regions.state_of[1], 10);
    assert!(regions.warnings[0].contains("zero controller"));
    assert!(regions.warnings[0].contains("province ids 1"));
}

/// A hour the collector had not finished filling still has a political map:
/// the fill is a function of the three core tables alone, and every other
/// table's gap is stated rather than refused.
#[test]
fn a_truncated_non_core_table_still_renders_the_political_layer_and_is_stated() {
    let mut frame = occupied();
    *frame.tables[TableId::ProvinceBuildings.index()] = TableView::Truncated {
        rows: TableRows::ProvinceBuildings(Vec::new()),
        missing: 1174,
    };
    // The layer is built, and it is the same one the complete hour builds:
    // no non-core table reaches the fill at all.
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
    // A core gap is a frame error, never one of the render's limitations (whose
    // lines name their table first; other tables' names can contain "Countries").
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
    // The relation retains its seed while the current status changes.
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
    // The intermediate country has no recorded colour: the getter never reads it.
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

/// Current autonomy definitions select inherited overlord colours. Missing status, definition or
/// overlord operands leave the source undecided with a named reason. A missing row in a complete
/// table is a null status.
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
    // Complete inputs: the union subject takes its overlord's colours; no row is no status.
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
    // A definition the content does not define.
    let modded = with(
        TableView::Complete(TableRows::Autonomy(vec![status(b"dominion")])),
        complete_subjects(),
    );
    assert!(modded.source(2).unwrap_err().contains("dominion"));
    // A truncated Autonomy census: a country without a row may hold a status.
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
    // An inheriting status whose overlord no Subjects row names.
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
    // The political fill names them and keeps its own colour.
    let mut frame = occupied();
    *frame.tables[TableId::Autonomy.index()] =
        TableView::Complete(TableRows::Autonomy(vec![status(b"dominion")]));
    let mut warnings = Vec::new();
    assert!(
        super::inherited_colors(&map, &frame, &links, &mut warnings)
            .unwrap()
            .is_empty()
    );
    assert!(warnings.iter().any(|line| line.contains("dominion")));
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
    // Layer 2 paints the two controllers, which is where the game puts the country field.
    assert!(
        (0..textures.width)
            .filter_map(|x| textures.channel1_texel(1, x, 0))
            .any(|texel| texel[3] > 0 && texel[..3] != [0, 0, 0])
    );
    // Layer 1 is the strategic regions over the one sea province, at the navy colour, and its
    // opacity is zero, so it draws nothing however wide its field is.
    // The texel covers map columns 4 and 5, land and sea. Layer 1 is the navy colour over
    // both, because that is its colour where it has no region too, and the alpha is the box
    // average of the land's no-region 255 and the sea boundary's 247.
    let sea = textures.channel1_texel(0, 2, 0).unwrap();
    assert_eq!(sea, [51, 63, 89, 251]);
    for x in 0..textures.width {
        assert_eq!(textures.channel2_texel(0, x, 0).unwrap()[1], 0);
    }
    // Land is in no region of layer 1 and sea is in no region of layer 2; both hold the alpha
    // the shader draws nothing at, with no colour and no opacity.
    assert_eq!(
        textures.channel1_texel(0, 0, 0),
        Some([51, 63, 89, gradient::NO_REGION_ALPHA]),
        "land is in no region of layer 1 and still carries its colour"
    );
    // The same texel in layer 2 is the country field at map column 4 boxed with the sea's
    // no-region alpha at column 5: (247 + 247 + 255 + 255) / 4. Its one-pixel
    // coast extension keeps the neighbouring country's colour and opacity.
    assert_eq!(textures.channel1_texel(1, 2, 0), Some([191, 127, 63, 251]));
    assert_eq!(textures.channel2_texel(1, 2, 0), Some([0, 255, 0, 0]));
    // The state keys the border classifier reads survive; no layer is built from them.
    assert_eq!(layers.regions.state_of[1], 10);
    assert_eq!(layers.regions.state_of[3], 11);
    // Channel 2's green is the region opacity wherever layer 2 has a member.
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
fn a_map_recording_holds_the_political_map_labels_and_weather() {
    use viewer_foundation::map::test_support::{
        assert_reads_recorded, complete_frame, map_scoped, put_sample_campaign,
    };
    let map = map();
    let mut frame = complete_frame(11, |table| {
        replay_data::rows::sample_extended_table(table, 0)
    });
    put_sample_campaign(&mut frame);
    let frame = map_scoped(frame);
    assert_reads_recorded(&frame, &[], || {
        build(&map, &frame).unwrap();
        FrameKey::new(&frame).unwrap();
        crate::map::labels::LabelTables::read(&frame, 1, &Default::default()).unwrap();
        let weather = MapData {
            width: 8,
            height: 4,
            province_ids: [1, 2, 3, 4, 5, 5, 5, 5].repeat(4),
            ..map.clone()
        };
        let cells = crate::map::weather::SnowCells::new(&weather, &[false; 32]).unwrap();
        crate::map::weather::grid_from_cells(&cells, &frame).unwrap();
        crate::map::weather_effects::field(&frame).unwrap();
    });
}
