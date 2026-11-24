use super::*;
use viewer_map_modes::map_mode::{MapMode, Stock, visibility};
use viewer_gpu::camera::{Free, Lens, MapDefines, MapSize, Pose};
use viewer_foundation::mode_visibility::PriorityTable;
use viewer_native_gui::test_support::gui;

const RULES: &[u8] = b"adjacency_rule = { name = A icon = 7 offset = { 1 0 -2 } }
    adjacency_rule = { name = B }
    adjacency_rule = { name = C icon = 9 }
    adjacency_rule = { name = D icon = 7 offset = { 3 0 4 } }
    adjacency_rule = { name = E icon = 11 offset = { 1 2 } }";

fn fixture(cutoff: Result<f32, String>) -> AdjacencyIcons {
    AdjacencyIcons::of(
        &content_reader_map::adjacency_rules::parse(RULES).unwrap(),
        cutoff,
    )
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

fn mode(stock: Stock) -> Visibility {
    visibility::of(&MapMode::Stock(stock))
}

fn anchor(province: u32) -> Option<[f32; 3]> {
    match province {
        7 => Some([3000., 10., 1500.]),
        9 => Some([3010., 10., 1505.]),
        _ => None,
    }
}

fn prepare(icons: &AdjacencyIcons, stock: Stock, height: f32) -> PreparedAdjacencyIcons {
    icons.prepare(&Inputs {
        anchor: &anchor,
        camera: camera(height),
        viewport: [2048, 1152],
        ui_scale: 1.,
        visibility: mode(stock),
    })
}

#[test]
fn each_icon_province_links_one_icon_and_a_later_rule_replaces_it() {
    let icons = fixture(Ok(1700.));
    assert_eq!(
        icons.icons,
        [
            Icon {
                province: 7,
                offset: [3., 0., 4.]
            },
            Icon {
                province: 9,
                offset: [0.; 3]
            },
        ]
    );
    assert_eq!(icons.refused.values().sum::<usize>(), 1);
    assert!(
        icons
            .refused
            .keys()
            .any(|reason| reason.contains("offset holds 2 numbers"))
    );
}

#[test]
fn icons_show_in_the_navy_supply_and_resources_modes_below_the_cutoff() {
    let icons = fixture(Ok(1700.));
    for stock in Stock::ALL {
        let placed = prepare(&icons, stock, 300.).provinces().count();
        let expected = matches!(
            stock,
            Stock::StrategicNavy | Stock::Supply | Stock::Resources
        );
        assert_eq!(placed == 2, expected, "{stock:?}");
        if !expected {
            assert_eq!(placed, 0, "{stock:?}");
        }
    }
    assert_eq!(
        prepare(&icons, Stock::Supply, 1699.9).provinces().count(),
        2
    );
    assert_eq!(prepare(&icons, Stock::Supply, 1700.).provinces().count(), 0);
    let refused = prepare(&fixture(Err("missing".into())), Stock::Supply, 300.);
    assert_eq!(refused.provinces().count(), 0);
    assert_eq!(
        refused
            .report
            .get(&format!("{ADJACENCY_RULE_WINDOW} is not drawn: missing")),
        Some(&2)
    );
}

#[test]
fn the_icon_stands_at_the_province_position_plus_its_offset() {
    let icons = AdjacencyIcons::of(
        &content_reader_map::adjacency_rules::parse(
            b"adjacency_rule = { name = A icon = 7 offset = { 1 0 -2 } }",
        )
        .unwrap(),
        Ok(1700.),
    );
    let prepared = prepare(&icons, Stock::StrategicNavy, 300.);
    let expected = camera(300.)
        .project_map_icon(
            [3001., 10., 1498.],
            [2048, 1152],
            gui_viewport([2048, 1152], 1.),
        )
        .unwrap()
        .position;
    assert_eq!(prepared.screens().collect::<Vec<_>>(), [expected]);
    let unshifted = camera(300.)
        .project_map_icon(
            [3000., 10., 1500.],
            [2048, 1152],
            gui_viewport([2048, 1152], 1.),
        )
        .unwrap()
        .position;
    assert_ne!(expected, unshifted);
}

#[test]
fn a_free_eye_shows_each_icon_by_its_own_distance() {
    let icons = AdjacencyIcons::of(
        &content_reader_map::adjacency_rules::parse(
            b"adjacency_rule = { name = A icon = 7 offset = { 1 0 -2 } }",
        )
        .unwrap(),
        Ok(100.),
    );
    let world = [3001., 10., 1498.];
    let prepared = |distance: f32| {
        let eye = [world[0], world[1], world[2] - distance];
        icons.prepare(&Inputs {
            anchor: &anchor,
            camera: Camera {
                center: [eye[0], eye[2]],
                height: eye[1],
                map: MapSize::TEST,
                defines: MapDefines::test(),
                pose: Pose::Free(Free::looking_at(eye, world, Lens::MAP)),
            },
            viewport: [2048, 1152],
            ui_scale: 1.,
            visibility: mode(Stock::StrategicNavy),
        })
    };
    assert_eq!(prepared(100.).provinces().count(), 0);
    let near = prepared(99.);
    assert_eq!(near.provinces().collect::<Vec<_>>(), [7]);
    assert_eq!(
        near.placed[0].anchor,
        IconAnchor {
            world,
            cutoff: 100.,
            family: IconFamily::AdjacencyRules,
            moves: None,
        }
    );
}

#[test]
fn a_province_without_a_position_is_counted() {
    let icons = AdjacencyIcons::of(
        &content_reader_map::adjacency_rules::parse(b"adjacency_rule = { name = A icon = 99 }")
            .unwrap(),
        Ok(1700.),
    );
    let prepared = prepare(&icons, Stock::Resources, 300.);
    assert_eq!(prepared.provinces().count(), 0);
    assert_eq!(prepared.report.values().sum::<usize>(), 1);
}

#[test]
fn unknown_build_refuses_adjacency_icons_through_presentation() {
    content_reader_script::test_game::enter();
    let resources = gui::fixture();
    let windows = gui::windows(
        br#"guiTypes = {
        containerWindowType = { name = adjacency_rule_mapicon size = { width = 40 height = 20 }
            iconType = { name = icon spriteType = GFX_even } }
    }"#,
    );
    let context = Context {
        resources: &resources,
        windows: &windows,
        viewport: [2048, 1152],
        time: Some(0.),
    };
    for (icons, expected_fragment) in [
        (fixture(Ok(1700.)), "game build is unverified"),
        (
            fixture(Ok(1700.)).with_build("Countenance v1.19.5.0.dead"),
            "Countenance v1.19.5.0.dead",
        ),
    ] {
        let prepared = prepare(&icons, Stock::StrategicNavy, 300.);
        let mut batch = Batch::default();
        let mut report = BTreeMap::new();
        let drawn = prepared.draw(
            &context,
            0,
            &mut PoolView::default(),
            &mut batch,
            &mut report,
        );
        assert_eq!(drawn, 0);
        assert!(batch.quads.is_empty());
        assert!(
            report.keys().any(|reason| {
                reason.contains("adjacency rule map icon pool initial capacity is unknown")
                    && reason.contains(expected_fragment)
            }),
            "{report:?}"
        );
    }
}
