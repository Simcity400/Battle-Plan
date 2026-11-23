use content_reader_mesh::mesh::MeshProducts as _;
use super::*;
use viewer_models::entity_animation::{IDLE, TRAINING};
use replay_data::rows::{
    DivisionCamouflage, DivisionModelInputRow, DivisionStatusRow, Recorded, TableId, TableRows,
};
use replay_frame::frame::TableView;
use viewer_foundation::test_support::{assert_reads_recorded, map_scoped};

fn figure_assets(name: &str) -> (std::path::PathBuf, UnitModelAssets) {
    use viewer_models::test_support::{MESHES_GFX, triangle_mesh, visual_stack};
    let (root, layers) = visual_stack(
        name,
        &[
            ("base/gfx/models/meshes.gfx", MESHES_GFX),
            (
                "base/gfx/entities/land.asset",
                br#"entity = { name = GER_custom_entity pdxmesh = body }
                    entity = { name = GER_custom_entity_snow pdxmesh = body }"#,
            ),
            ("base/gfx/models/plane.mesh", &triangle_mesh(1.0)),
        ],
    );
    let mut assets = camouflaged_assets();
    assets.roster = layers.descriptor_roster().unwrap();
    assets.layers = layers;
    assets.bounds = Arc::new((1..=4).map(|id| (id, [0.0, 0.0, 1.0, 1.0])).collect());
    (root, assets)
}

fn figures(
    assets: &UnitModelAssets,
    frame: &DecodedFrame,
) -> (Vec<(u64, String, u32)>, BTreeMap<String, usize>) {
    let prepared = equipment_graphics::test_support::with_world(frame, &[], |limits| {
        assets
            .prepare(PrepareInputs {
                frame,
                limits,
                vision: &Vision::default(),
                etas: &Etas::default(),
                anchor: &|_, _| Some([0.0; 3]),
                world: None,
                terrain: &BTreeMap::new(), routes: None,
                ships: &ShipPlaces {
                    motions: &viewer_unit_reads::task_force_visual::TaskForceMotions::default(
                    ),
                    movement: None,
                    land: &|_| true,
                    missions: &Default::default(),
                    region: &|_| None,
                    scale: &|_, _| 1.0,
                },
                localisation: &content_reader_localisation::localisation::Localisation::default(),
            })
            .unwrap()
    });
    let library = assets.library();
    let placed = prepared
        .placed
        .iter()
        .map(|(target, _)| {
            let name = ["GER_custom_entity", "GER_custom_entity_snow"]
                .into_iter()
                .find(|name| library.slot(name) == Some(target.mesh))
                .expect("a baked figure");
            (target.identity, name.to_owned(), target.instance.animation)
        })
        .collect();
    (placed, prepared.report.selection_errors)
}

fn divisions() -> Vec<replay_data::rows::DivisionRow> {
    (1..=4).map(|id| division(id, id as u32, 50)).collect()
}

#[test]
fn map_figures_read_the_model_witness_as_full_reads_division_status() {
    let (root, assets) = figure_assets("model_witness_figures");
    let shown = Some(DivisionCamouflage { category: Some(1) });
    let plain = Some(DivisionCamouflage { category: None });

    let full_status = |division, camouflage, exercising| DivisionStatusRow {
        division,
        exercising,
        camouflage: Recorded::Value(camouflage),
        ..Default::default()
    };
    let full = hour_frame(
        divisions(),
        vec![
            full_status(1, shown, 1),
            full_status(2, plain, 0),
            full_status(3, None, 0),
        ],
        4,
    );
    let (full_figures, full_errors) = figures(&assets, &full);

    let witness = |division, camouflage, exercising| DivisionModelInputRow {
        division,
        model_number: Some(0),
        camouflage,
        exercising,
    };
    let mut map = map_scoped(hour_frame(divisions(), Vec::new(), 4));
    *map.tables[TableId::DivisionModelInputs.index()] =
        TableView::Complete(TableRows::DivisionModelInputs(vec![
            witness(1, shown, Some(1)),
            witness(2, plain, Some(0)),
            witness(3, None, Some(0)),
            witness(4, shown, None),
        ]));
    let (map_root, map_assets) = figure_assets("model_witness_figures_map");
    let (map_figures, map_errors) =
        assert_reads_recorded(&map, &[], || figures(&map_assets, &map));
    let mut projected = map.clone();
    for table in TableId::ALL {
        if table != TableId::DivisionStatus
            && matches!(
                *projected.tables[table.index()],
                TableView::NotRecorded { .. }
            )
        {
            *projected.tables[table.index()] = TableView::Unavailable {
                reason: "Table not selected for reconstruction".into(),
            };
        }
    }
    assert!(projected.capture_mode().is_err());
    assert_eq!(
        assert_reads_recorded(&projected, &[], || figures(&map_assets, &projected)),
        (map_figures.clone(), map_errors.clone())
    );

    let expected = [
        (1, "GER_custom_entity_snow".to_owned(), TRAINING),
        (2, "GER_custom_entity".to_owned(), IDLE),
    ];
    assert_eq!(full_figures, expected);
    assert_eq!(map_figures, expected);
    assert_eq!(
        full_errors,
        BTreeMap::from([
            ("division camouflage is unavailable this hour".to_owned(), 1),
            (
                "division acclimatization status is unavailable".to_owned(),
                1
            ),
        ])
    );
    assert_eq!(
        map_errors,
        BTreeMap::from([
            ("division camouflage is unavailable this hour".to_owned(), 1),
            ("division exercise is unavailable this hour".to_owned(), 1),
        ])
    );
    std::fs::remove_dir_all(root).unwrap();
    std::fs::remove_dir_all(map_root).unwrap();
}
