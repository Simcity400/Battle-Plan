use super::*;
use replay_data::rows::{Recorded, TableId, TableRows};
use replay_frame::frame::{Metrics, TableView};

fn inputs() -> (DecodedFrame, Database, installed_content_core::defines::Defines) {
    engine_executable::executable_tables::initialize_for_tests().unwrap();
    let content = crate::test_fixtures::equipment_types(&[br#"
        equipment_modules = {
            old = { category = c add_stats = { build_cost_ic = 2 } multiply_stats = { build_cost_ic = 0.1 } }
            next = { category = c add_stats = { build_cost_ic = 3 } multiply_stats = { build_cost_ic = 0.2 }
                can_convert_from = { module = old convert_cost_ic = 4 convert_cost_resources = { x = 0.5 y = 2.7 } } }
        }
        upgrades = { upgrade = { add_stats = { build_cost_ic = 1.5 } resource_cost_thresholds = { 1 = { x = 1 } 2 = { x = 2.5 } } } }
        equipments = { type = { is_archetype = yes type = armor build_cost_ic = 10 module_slots = { main = { } } } }
    "#]).unwrap();
    let defines = installed_content_core::defines::Defines::new().unwrap();
    defines
        .load(
            "synthetic.lua",
            br#"NDefines={NProduction={
        CONVERSION_SPEED_BONUS=0.1,
        BASE_LAND_EQUIPMENT_CONVERSION_IC_COST_FACTOR=0.5,
        BASE_NAVAL_EQUIPMENT_CONVERSION_IC_COST_FACTOR=0.25,
        MIN_LAND_EQUIPMENT_CONVERSION_RESOURCE_COST_FACTOR=0.5,
        MIN_NAVAL_EQUIPMENT_CONVERSION_RESOURCE_COST_FACTOR=0,
        MAX_EQUIPMENT_RESOURCES_NEED=2}}
    "#,
        )
        .unwrap();
    let mut f = DecodedFrame {
        branch: 1,
        hour: 7,
        metrics: Metrics::default(),
        tables: std::array::from_fn(|_| {
            TableView::Unavailable {
                reason: "missing fixture operand".into(),
            }
            .into()
        }),
    };
    *f.tables[TableId::LexicalTokens.index()] = TableView::Complete(TableRows::LexicalTokens(
        [
            (1, "type"),
            (2, "MAIN"),
            (3, "old"),
            (4, "next"),
            (5, "upgrade"),
            (6, "x"),
            (7, "y"),
        ]
        .into_iter()
        .map(|(token, name)| rows::LexicalTokenRow {
            token,
            literal: name.as_bytes().to_vec(),
        })
        .collect(),
    ));
    *f.tables[TableId::VariantDefinitions.index()] =
        TableView::Complete(TableRows::VariantDefinitions(
            [10, 20]
                .map(|id| rows::VariantDefinitionRow {
                    id,
                    equipment_token: 1,
                    archetype_token: 1,
                    creator_link: 5_000,
                    design: Recorded::Value(rows::VariantDesignState {
                        build_cost: 1_485_000,
                        build_resources: vec![
                            rows::VariantStatEntry {
                                index: 7,
                                value: 100_000,
                            },
                            rows::VariantStatEntry {
                                index: 6,
                                value: 400_000,
                            },
                        ],
                        ..Default::default()
                    }),
                    ..Default::default()
                })
                .to_vec(),
        ));
    *f.tables[TableId::VariantModules.index()] = TableView::Complete(TableRows::VariantModules(
        [(10, 3), (20, 4)]
            .map(|(variant, definition_token)| rows::VariantModuleRow {
                variant,
                slot: 2,
                definition_token,
                definition_active: 1,
                ..Default::default()
            })
            .to_vec(),
    ));
    *f.tables[TableId::VariantUpgrades.index()] = TableView::Complete(TableRows::VariantUpgrades(
        [(10, 1), (20, 2)]
            .map(|(variant, level)| rows::VariantUpgradeRow {
                variant,
                definition_token: 5,
                level,
                ..Default::default()
            })
            .to_vec(),
    ));
    *f.tables[TableId::ResourceDefinitions.index()] =
        TableView::Complete(TableRows::ResourceDefinitions(
            [(1, 6), (2, 7)]
                .map(|(id, token)| rows::ResourceDefinitionRow {
                    id,
                    token,
                    ..Default::default()
                })
                .to_vec(),
        ));
    *f.tables[TableId::DesignCountryModifiers.index()] =
        TableView::Complete(TableRows::DesignCountryModifiers(
            [(0xdc, 10_000), (0x1d6, -20_000)]
                .map(|(modifier, value)| rows::DesignCountryModifierRow {
                    country: 5_000,
                    modifier,
                    value: Some(value),
                    failure: 0,
                })
                .to_vec(),
        ));
    (f, content, defines)
}
#[test]
fn selected_pair_conversion_keeps_hull_upgrade_modifiers_floors_and_target_resource_order() {
    let (mut f, content, defines) = inputs();
    let result = conversion_cost(&f, 20, 10, None, &content, &defines, |_| {
        panic!("comparison has no organisation")
    })
    .unwrap();
    assert_eq!(
        (
            result.conversion,
            result.construction,
            result.dismantling,
            result.multiplier_difference
        ),
        (840_000, 180_000, 0, 126_500)
    );
    assert_eq!(result.total.ic, 1_146_500);
    assert_eq!(result.total.resources, [(7, 200_000), (6, 200_000)]);
    let TableView::Complete(TableRows::VariantDefinitions(definitions)) =
        &mut *f.tables[TableId::VariantDefinitions.index()]
    else {
        panic!()
    };
    definitions[1].classifier_flags = engine_native::cequipment_type::SHIP_MASK;
    let naval = conversion_cost(&f, 20, 10, Some(777), &content, &defines, |_| {
        panic!("naval refit does not reach conversion-speed bonus")
    })
    .unwrap();
    assert_eq!(
        (
            naval.conversion,
            naval.construction,
            naval.multiplier_difference
        ),
        (650_000, 150_000, 126_500)
    );
    assert_eq!(naval.total.ic, 926_500);
    assert_eq!(naval.total.resources, [(7, 200_000), (6, 200_000)]);
}
#[test]
fn selected_pair_conversion_demands_only_reached_organisation_and_country_operands() {
    let (mut f, content, defines) = inputs();
    assert!(
        conversion_cost(&f, 20, 10, Some(777), &content, &defines, |reference| {
            assert_eq!(reference, 777);
            Err("selected organisation conversion bonus refused (count: 1)".into())
        })
        .unwrap_err()
        .contains("selected organisation")
    );
    let reduced =
        conversion_cost(&f, 20, 10, Some(777), &content, &defines, |_| Ok(100_000)).unwrap();
    assert_eq!(reduced.conversion, 480_000);
    *f.tables[TableId::DesignCountryModifiers.index()] =
        TableView::Complete(TableRows::DesignCountryModifiers(vec![]));
    assert!(
        conversion_cost(&f, 20, 10, None, &content, &defines, |_| unreachable!())
            .unwrap_err()
            .contains("modifier 220")
    );
}
#[test]
fn resource_limit_removes_first_smallest_before_integer_truncation_and_keeps_zeroes() {
    let mut pool = Pool {
        ic: 0,
        resources: vec![(9, 199_999), (7, 199_999), (8, 200_001), (6, -1)],
    };
    pool.prune(2, &[rows::VariantStatEntry { index: 8, value: 0 }]);
    assert_eq!(pool.resources, [(8, 200_000), (7, 100_000)]);
    pool.resources = vec![(1, 1), (2, -1), (3, i64::MAX)];
    pool.prune(3, &[]);
    assert_eq!(pool.resources[0], (1, 0));
    assert_eq!(pool.resources[1], (2, 0));
    assert_eq!(
        pool.resources[2].1,
        i64::from((i64::MAX / 100_000) as i32) * 100_000
    );
}
