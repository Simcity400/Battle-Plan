use super::*;
use replay_data::rows::*;
use replay_frame::frame::{Metrics, TableView};
use std::cell::Cell;

fn fixture(hour: i32) -> DecodedFrame {
    let mut frame = DecodedFrame {
        branch: 1,
        hour,
        tables: std::array::from_fn(|index| {
            TableView::Complete(TableRows::empty(TableId::ALL[index])).into()
        }),
        metrics: Metrics::default(),
    };
    let mut put = |rows: TableRows| {
        let index = rows.table().index();
        frame.tables[index] = TableView::Complete(rows).into();
    };
    put(TableRows::CountryLinks(vec![CountryLinkRow {
        id: 1,
        canonical_slot: 0,
    }]));
    put(TableRows::SimulationFeatures(vec![SimulationFeatureRow {
        dlc_mask: 0x100,
        ..Default::default()
    }]));
    put(TableRows::AirSortieModifierMetadata(vec![
        AirSortieModifierMetadataRow::default(),
    ]));
    put(TableRows::AirWings(
        (1..=6)
            .map(|id| AirWingRow {
                id,
                owner: 1,
                air_base: 10,
                planes: 1,
                pool_ordinal: id as u32 - 1,
                mission_mask: AIR_MISSION_TRAINING,
                ..Default::default()
            })
            .collect(),
    ));
    put(TableRows::AirReferenceWitnesses(vec![
        AirReferenceWitnessRow {
            handle: 10,
            base_kind: Some(0),
            state: Some(1),
            province: Some(1),
            ..Default::default()
        },
    ]));
    put(TableRows::VariantDefinitions(
        [10, 20, 30]
            .map(|id| VariantDefinitionRow {
                id,
                ..Default::default()
            })
            .into(),
    ));
    put(TableRows::WingEquipment(
        (1..=6)
            .map(|wing| WingEquipmentRow {
                wing,
                variant: 10 * (1 + (wing - 1) / 2),
                ordinal: 0,
                amount_raw: -100_000,
            })
            .collect(),
    ));
    frame
}

#[test]
fn memo_primary_index_and_kept_placements_keep_fixed_multi_hour_aircraft_and_failure_order() {
    content_reader_script::test_game::enter();
    let map = MapData {
        width: 4,
        height: 2,
        province_ids: vec![1; 8],
        definitions: BTreeMap::new(),
        country_colors: Default::default(),
    };
    let geometry = ProvinceGeometry::build(&map);
    let heights = HeightField {
        width: 2,
        height: 2,
        values: &[0; 4],
    };
    let defines = Defines::new().unwrap();
    let mut assets = AircraftAssets::load(
        &ResolvedLayers::default(),
        Decode::default(),
        &map,
        (&defines, &defines),
        &BTreeMap::new(),
        &mut Vec::new(),
    )
    .unwrap();
    assets.roster = Arc::new(roster(
        r#"
        entity = { name = DesignerA state = { name = idle } }
        entity = { name = DesignerB state = { name = idle } }
        entity = { name = Graphical state = { name = idle } }
    "#,
    ));
    assets.flight_defines = Ok(FlightDefines::for_tests());
    assets
        .flight_defines
        .as_mut()
        .unwrap()
        .training
        .sizing
        .limit = 64;
    assets.sortie_defines = Ok(SortieDefines {
        fuel_penalty_raw: 0,
        missile_launcher_slots: 1,
        carrier_stance_ratios: Vec::new(),
        mine_maximum_raw: 100_000,
    });
    assets.region_draws = [(7, vec![1])].into();
    assets.launch_content =
        launches::Content::for_tests([((1, 0), Ok([2.0, 10.0, 2.0]))].into(), 0.0);
    assets.rigs.insert(
        "fighter_1_patrol".into(),
        Rig {
            tracks: vec![vec![
                Transform {
                    translation: [0.0, 10.0, 0.0],
                    ..Transform::IDENTITY
                },
                Transform {
                    translation: [0.0, 10.0, 1.0],
                    ..Transform::IDENTITY
                },
            ]],
            sides: vec![0],
            fps: 1.0,
            duration: 1.0,
            events: Vec::new(),
            speed: 1.0,
            unbound: vec![None],
            lands: vec![true],
            rockets: vec![false],
        },
    );
    let mut previous: Option<AircraftPlan> = None;
    let mut kept = Kept::default();
    for (hour, designer, placed) in [(100, "DesignerA", (4, 0)), (101, "DesignerB", (2, 2)), (102, "DesignerA", (2, 2))] {
        let frame = fixture(hour);
        let build = |kept: &mut Kept| {
            let memo = super::super::model_inputs::ModelMemo::default();
            let calls = Cell::new(0);
            let select = |variant, country| {
                memo.resolve(variant, country, || {
                    calls.set(calls.get() + 1);
                    let row = frame
                        .complete::<tables::VariantDefinitions>()
                        .unwrap()
                        .iter()
                        .find(|row| row.id == variant)
                        .unwrap();
                    aircraft_model::resolve_lazy(
                        row,
                        0x100,
                        &|| match variant {
                            10 => Ok(Some(designer.into())),
                            20 => Ok(None),
                            _ => Err("fixture variant unavailable".into()),
                        },
                        &|| {
                            assert_eq!(variant, 20);
                            Ok(Some("Graphical".into()))
                        },
                        &|| panic!("unreached country ladder"),
                        &|name| assets.roster.descriptors.entities.contains_key(name),
                    )
                })
            };
            let plan = AircraftPlan::make(
                &frame,
                &assets,
                &[(1, 7)].into(),
                &geometry,
                &heights,
                0,
                &select,
                &|_| panic!("unreached carrier"),
                &|_| None,
                kept,
            )
            .unwrap();
            assert_eq!(calls.get(), 3);
            plan
        };
        let mut plan = build(&mut Kept::default());
        let keeping = build(&mut kept);
        assert_eq!(
            (keeping.marks.scenarios, keeping.marks.kept),
            placed,
            "scenarios placed anew and kept at hour {hour}"
        );
        assert_eq!(
            keeping.differ(&plan).1,
            0,
            "kept at hour {hour}: {:?}",
            keeping.differ(&plan).2
        );
        assert_eq!(
            plan.flights
                .iter()
                .map(|flight| flight.wing)
                .collect::<Vec<_>>(),
            [4, 1, 2, 3]
        );
        let library = assets.library();
        assert_eq!(
            plan.flights
                .iter()
                .map(|flight| library.meshes[flight.mesh].name.as_str())
                .collect::<Vec<_>>(),
            ["Graphical", designer, designer, "Graphical"]
        );
        assert_eq!(
            plan.diagnostics,
            [
                "wing 5 equipment type: fixture variant unavailable",
                "wing 6 equipment type: fixture variant unavailable"
            ]
        );
        assert_eq!(plan.runs.len(), 4);
        if let Some(previous) = &mut previous {
            plan.carry_opening(previous);
            plan.carry_easing(previous);
            assert!(plan.opened);
            assert_eq!(plan.eased_at, Some(4.0));
            assert_eq!(plan.eased.len(), 4);
        }
        for time in [0.0, 0.25, 1.0, 2.25, 4.0] {
            let (field, hidden) = plan.field(&assets, time, &|_| true, &|_| None);
            assert_eq!(field.instances.len(), 4);
            assert_eq!(field.clocks.len(), 4);
            assert!(hidden.is_empty());
            assert!(plan.missile_explosions(time - 0.25, time).is_empty());
            assert_eq!(plan.ran_at, Some(time));
            assert_eq!(plan.eased_at, Some(time));
        }
        previous = Some(plan);
    }
}

#[test]
fn primary_index_keeps_missing_table_lazy_and_preserves_the_reached_refusal() {
    let mut frame = fixture(100);
    frame.tables[TableId::WingEquipment.index()] = TableView::Unavailable {
        reason: "fixture equipment".into(),
    }
    .into();
    let primary = super::super::model_inputs::Primary::new(&frame);
    for wing in [1, 2] {
        assert_eq!(
            primary.get(wing),
            Err("hour 100 WingEquipment is unavailable: fixture equipment".into())
        );
    }
}
