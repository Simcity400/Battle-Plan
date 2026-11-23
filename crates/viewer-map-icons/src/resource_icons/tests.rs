use super::*;
use viewer_map_modes::map_mode::{MapMode, Stock, visibility};
use replay_data::{rows::ProvinceRow, rows::TableRows};
use viewer_gpu::camera::{Free, Lens, MapDefines, MapSize, Pose};
use viewer_foundation::{mode_visibility::PriorityTable, test_support::localisation::loaded};
use viewer_native_gui::test_support::gui;
use viewer_runtime::resource_origins::fixture::{
    self,
    AFG_SLOT,
    ALIASED,
    COSTED,
    GATED,
    GIVEN,
    GRANTED,
    MIXED,
};

fn content() -> ResourceIconContent {
    let mut priorities = Priorities::uniform([0; PriorityIcon::ALL.len()]);
    priorities.set(PriorityTable::Resources, PriorityIcon::Resource, 10);
    ResourceIconContent {
        origins: fixture::content(),
        defines: ResourceIconDefines {
            icons_cutoff: 900.,
            text_cutoff: 800.,
            priorities,
        },
    }
}

fn texts() -> Localisation {
    loaded(concat!(
        "l_english:\n",
        " MAP_ICON_NAME:0 \"$NAME$: $VAL$\"\n",
        " MAP_RESOURCE_ICON_VAL:0 \"$VAL$ of $MAX$\"\n",
        " PRODUCTION_MATERIALS_OIL:0 \"Fuel\"\n",
        " PRODUCTION_MATERIALS_STEEL:0 \"Metal\"\n",
        " PRODUCTION_MATERIALS_RUBBER:0 \"Gum\"\n",
    ))
}

fn camera(height: f32) -> Camera {
    Camera {
        center: [3000., 1500.],
        height,
        map: MapSize::TEST,
        defines: MapDefines::test(),
        pose: Pose::Map,
    }
}

fn frame() -> DecodedFrame {
    let mut frame = fixture::frame();
    let mut provinces: Vec<_> = (1..=8)
        .map(|state| ProvinceRow {
            id: state * 10,
            state,
            victory_points: 1,
            ..Default::default()
        })
        .collect();
    provinces.push(ProvinceRow {
        id: 11,
        state: MIXED,
        victory_points: 5,
        ..Default::default()
    });
    provinces.sort_by_key(|row| row.id);
    fixture::put(&mut frame, TableRows::Provinces(provinces));
    frame
}

fn anchors() -> BTreeMap<u32, [f32; 3]> {
    [10, 11, 20, 30, 40, 50, 60, 70, 80]
        .into_iter()
        .map(|province| (province, [3000., 10., 1500.]))
        .collect()
}

fn prepared(stock: Stock, height: f32) -> PreparedResourceIcons {
    let content = content();
    let localisation = texts();
    let anchors = anchors();
    prepare(
        &ResourceIconInputs {
            content: &content,
            localisation: &localisation,
            state_provinces: &(1..=8).map(|state| (state, vec![state * 10])).collect(),
            centroid: &|province| anchors.get(&province).copied(),
            camera: camera(height),
            viewport: [2048, 1152],
            ui_scale: 1.,
            visibility: visibility::of(&MapMode::Stock(stock)),
        },
        &frame(),
    )
}

#[test]
fn icons_draw_only_in_the_resources_mode_below_the_unit_icon_cutoff() {
    let defines = content().defines;
    let mode = |stock| visibility::of(&MapMode::Stock(stock));
    assert_eq!(shown(&mode(Stock::Resources), 100., &defines), Some(true));
    assert_eq!(shown(&mode(Stock::Resources), 800., &defines), Some(false));
    assert_eq!(shown(&mode(Stock::Resources), 899.9, &defines), Some(false));
    assert_eq!(shown(&mode(Stock::Resources), 900., &defines), None);
    for stock in Stock::ALL
        .into_iter()
        .filter(|stock| *stock != Stock::Resources)
    {
        assert_eq!(shown(&mode(stock), 100., &defines), None, "{stock:?}");
        assert!(prepared(stock, 100.).placed.is_empty(), "{stock:?}");
    }
    assert!(prepared(Stock::Resources, 900.).placed.is_empty());
    let far = prepared(Stock::Resources, 850.);
    assert_eq!(far.placed.len(), 6);
    assert!(
        far.placed
            .iter()
            .all(|icon| !icon.text && icon.rows.is_empty())
    );
}

#[test]
fn a_free_eye_shows_each_icon_by_its_own_distance() {
    let content = content();
    let localisation = texts();
    let anchors = anchors();
    let anchor = [3000., 10., 1500.];
    let prepared = |distance: f32| {
        let eye = [3000., 10., 1500. - distance];
        prepare(
            &ResourceIconInputs {
                content: &content,
                localisation: &localisation,
                state_provinces: &(1..=8).map(|state| (state, vec![state * 10])).collect(),
                centroid: &|province| anchors.get(&province).copied(),
                camera: Camera {
                    center: [eye[0], eye[2]],
                    height: eye[1],
                    map: MapSize::TEST,
                    defines: MapDefines::test(),
                    pose: Pose::Free(Free::looking_at(eye, anchor, Lens::MAP)),
                },
                viewport: [2048, 1152],
                ui_scale: 1.,
                visibility: visibility::of(&MapMode::Stock(Stock::Resources)),
            },
            &frame(),
        )
    };
    assert!(prepared(950.).placed.is_empty());
    let far = prepared(850.);
    assert_eq!(far.placed.len(), 6);
    assert!(
        far.placed
            .iter()
            .all(|icon| !icon.text && icon.rows.is_empty())
    );
    let near = prepared(100.);
    assert_eq!(near.placed.len(), 6);
    assert!(near.placed.iter().all(|icon| icon.text));
    let expected = IconAnchor {
        world: anchor,
        cutoff: 900.,
        family: IconFamily::Resources,
        moves: None,
    };
    assert!(near.placed.iter().all(|icon| icon.anchor == expected));
}

#[test]
fn every_state_with_resources_gets_an_icon_with_the_games_rows_and_flags() {
    let icons = prepared(Stock::Resources, 100.);
    let states: Vec<_> = icons.placed.iter().map(|icon| icon.state).collect();
    assert_eq!(states, [MIXED, GATED, COSTED, GRANTED, GIVEN, ALIASED]);
    assert_eq!(icons.report.refused.len(), 1, "{:?}", icons.report.refused);
    assert!(
        icons
            .report
            .refused
            .keys()
            .all(|reason| reason.contains("holds no resource origin"))
    );
    let icon = |state| {
        icons
            .placed
            .iter()
            .find(|icon| icon.state == state)
            .unwrap()
    };
    let mixed = icon(MIXED);
    assert!(mixed.text);
    assert_eq!(mixed.priority, 10);
    assert_eq!(mixed.flag, None);
    assert_eq!(
        mixed.rows,
        [
            Row {
                text: "\u{13}resources_strip|1 Fuel: \u{11}G5 of 5\u{11}!\n".into(),
                flag: Some(AFG_SLOT),
            },
            Row {
                text: "\u{13}resources_strip|2 Metal: \u{11}R5 of 7\u{11}!\n".into(),
                flag: None,
            },
        ]
    );
    let given = icon(GIVEN);
    assert_eq!(given.flag, Some(AFG_SLOT));
    assert_eq!(
        given.rows,
        [Row {
            text: "\u{13}resources_strip|2 Metal: 1 of 1\u{11}!\n".into(),
            flag: None,
        }]
    );
    assert!(icon(GATED).rows.is_empty() && icon(COSTED).rows.is_empty());
    assert_eq!(icon(GRANTED).rows.len(), 1);
    assert_eq!(icon(ALIASED).flag, None);
}

#[test]
fn resource_anchor_uses_centroid_mean_and_authored_ties_and_wrapping() {
    let members = BTreeMap::from([(1, vec![7, 4, 8]), (2, vec![9, 10])]);
    let centroid = |p| {
        Some(match p {
            7 => [1., 0., 0.],
            4 => [3., 0., 0.],
            8 => [9., 0., 0.],
            9 => [99., 0., 0.],
            10 => [1., 0., 0.],
            _ => unreachable!(),
        })
    };
    let anchors = anchor_provinces(&members, &centroid, 100.).unwrap();
    assert_eq!(anchors[&1], 4);
    assert_eq!(anchors[&2], 10);
    let reversed = BTreeMap::from([(2, vec![10, 9])]);
    assert_eq!(
        anchor_provinces(&reversed, &centroid, 100.).unwrap()[&2],
        10
    );
}

#[test]
fn graphical_state_anchor_uses_signed_integer_distance_and_keeps_normal_ties() {
    let members = BTreeMap::from([(1, vec![1, 2]), (2, vec![3, 4]), (3, vec![])]);
    let centroid = |province| {
        Some(match province {
            1 => [10., 0., 20.],
            2 => [12., 0., 20.],
            3 => [100_000., 0., 0.],
            4 => [0., 0., 0.],
            _ => unreachable!(),
        })
    };
    let anchors = anchor_provinces(&members, &centroid, 400_000.).unwrap();
    assert_eq!(anchors[&1], 1);
    assert_eq!(anchors[&2], 4);
    assert!(!anchors.contains_key(&3));
}

#[test]
fn a_resource_without_an_icon_frame_is_refused_by_name() {
    let definition = Definition {
        index: 1,
        token: 1,
        name: "oil".into(),
        icon_frame: None,
    };
    let refused = row_text(&texts(), &definition, 0, 100_000).unwrap_err();
    assert!(refused.contains("icon_frame"), "{refused}");
    let negative = row_text(
        &texts(),
        &Definition {
            icon_frame: Some(4),
            ..definition
        },
        -250_000,
        -50_000,
    )
    .unwrap();
    assert_eq!(
        negative,
        "\u{13}resources_strip|4 Fuel: \u{11}R-3 of -1\u{11}!\n"
    );
}

const GUI: &[u8] = br#"guiTypes = {
    containerWindowType = { name = resource_mapicon_item size = { width = 100 height = 16 }
        orientation = upper_left
        instantTextBoxType = { name = desc font = Main_14 maxWidth = 100 maxHeight = 20 }
        iconType = { name = flag spriteType = GFX_even position = { x = -25 y = -5 } } }
    containerWindowType = { name = resource_mapicon size = { width = 100 height = 25 }
        orientation = upper_left
        listboxType = { name = resources_list position = { x = 0 y = 0 } size = { x = 485 y = 460 }
            horizontal = 0 spacing = 0 borderSize = { x = 0 y = 0 } }
        iconType = { name = flag spriteType = GFX_even position = { x = 20 y = -22 } } }
}"#;

fn placed(text: bool, rows: usize, flag: Option<u32>) -> PreparedResourceIcons {
    PreparedResourceIcons {
        report: Default::default(),
        placed: vec![Placed {
            state: MIXED,
            screen: [100., 100.],
            depth: 0.5,
            anchor: IconAnchor {
                world: [3000., 10., 1500.],
                cutoff: 900.,
                family: IconFamily::Resources,
                moves: None,
            },
            priority: 10,
            text,
            rows: (0..rows)
                .map(|row| Row {
                    text: format!("row {row}\n"),
                    flag: (row == 0).then_some(AFG_SLOT),
                })
                .collect(),
            flag,
        }],
    }
}

#[test]
fn the_window_lists_its_rows_and_keeps_row_windows_the_rows_leave() {
    content_reader_script::test_game::enter();
    let resources = gui::fixture();
    let windows = gui::windows(GUI);
    let context = Context {
        resources: &resources,
        windows: &windows,
        viewport: [400, 400],
        time: Some(0.),
    };
    let mut view = ResourceIconView::default();
    let mut batch = Batch::default();
    let report = placed(true, 2, None).draw(&context, &mut view, &mut batch);
    assert_eq!((report.icons, report.rows), (1, 2), "{:?}", report.refused);
    assert!(!batch.quads.is_empty());
    assert_eq!(view.items.values().map(Vec::len).collect::<Vec<_>>(), [2]);
    let report = placed(true, 1, None).draw(&context, &mut view, &mut batch);
    assert_eq!((report.icons, report.rows), (1, 1), "{:?}", report.refused);
    assert_eq!(view.items.values().map(Vec::len).collect::<Vec<_>>(), [2]);
    let items = view.items.values().next().expect("the window's rows");
    assert_eq!(
        items.iter().map(Instance::drawn).collect::<Vec<_>>(),
        [true, false]
    );
    let report = placed(false, 0, Some(AFG_SLOT)).draw(&context, &mut view, &mut batch);
    assert_eq!(report.rows, 0, "{:?}", report.refused);
    let report = placed(true, 0, Some(AFG_SLOT)).draw(&context, &mut view, &mut batch);
    assert_eq!(report.icons, 1, "{:?}", report.refused);
}

#[test]
fn an_icon_s_quads_stand_at_its_anchor() {
    content_reader_script::test_game::enter();
    let resources = gui::fixture();
    let windows = gui::windows(GUI);
    let context = Context {
        resources: &resources,
        windows: &windows,
        viewport: [400, 400],
        time: Some(0.),
    };
    let icons = placed(true, 2, None);
    let mut batch = Batch::default();
    let report = icons.draw(&context, &mut ResourceIconView::default(), &mut batch);
    assert_eq!((report.icons, report.rows), (1, 2), "{:?}", report.refused);
    assert!(!batch.quads.is_empty());
    let anchor = Some(icons.placed[0].anchor);
    assert!(batch.quads.iter().all(|quad| quad.anchor == anchor));
    batch.push(batch.quads[0].clone());
    assert_eq!(batch.quads.last().unwrap().anchor, None);
}
