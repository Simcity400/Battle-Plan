use super::*;
use content_reader_key_names::terrain::{TerrainAdjuster, TerrainCategory};

fn category(name: &str, color: Result<[f32; 4], &str>) -> TerrainCategory {
    TerrainCategory {
        name: name.as_bytes().to_vec(),
        movement_cost: 100_000,
        combat_width: 0,
        combat_support_width: 0,
        supply_flow_penalty_factor: 0,
        modifiers: Default::default(),
        classes: Vec::new(),
        minimum_seazone_dominance: 0,
        common_adjuster: TerrainAdjuster::default(),
        class_adjusters: Vec::new(),
        color: color
            .map(|color| color.map(f32::to_bits))
            .map_err(str::to_owned),
    }
}

const DEFINES: Defines = Defines {
    alpha: [0.5, 0.8],
    channel: [0.39, 0.973],
};

fn database(with_ocean: bool) -> TerrainDefinitions {
    let mut categories = vec![
        category(
            "forest",
            Ok([89.0 / 255.0, 199.0 / 255.0, 85.0 / 255.0, 1.0]),
        ),
        category("deep", Ok([2.0 / 255.0, 38.0 / 255.0, 150.0 / 255.0, 1.0])),
        category("broken", Err("terrain category broken: a packed colour")),
    ];
    if with_ocean {
        categories.push(category(
            "ocean",
            Ok([40.0 / 255.0, 83.0 / 255.0, 176.0 / 255.0, 1.0]),
        ));
    }
    TerrainDefinitions {
        categories,
        ..Default::default()
    }
}

fn sources_built(with_ocean: bool) -> Built {
    let database = database(with_ocean);
    let terrain = |province: u32| {
        let (flags, index) = match province {
            1 => (1 | 4, 1),
            2 | 3 => (16, 0),
            4 | 8 => (2, 0),
            5 => (1, 0),
            7 => (1 | 8, 3),
            _ => return None,
        };
        Some(ProvinceTerrain { flags, index })
    };
    let regions = BTreeMap::from([
        (
            10,
            StrategicRegion {
                naval_terrain: Some("Deep".to_owned()),
                ..Default::default()
            },
        ),
        (11, StrategicRegion::default()),
    ]);
    let region_of = BTreeMap::from([(1, 11), (2, 10), (3, 11), (8, 10)]);
    build(
        &Sources {
            categories: &database,
            provinces: &[1, 2, 3, 4, 5, 6, 7, 8],
            terrain: &terrain,
            region_of: &region_of,
            regions: &regions,
        },
        &DEFINES,
    )
    .unwrap()
}

#[test]
fn land_takes_its_category_and_water_its_regions_naval_terrain_or_ocean() {
    let built = sources_built(true);
    let layer = &built.layer;
    let color = |province: u32| layer.styles.get(&province).map(|style| style.color);
    let forest = [89, 199, 85, 127];
    let deep = [2, 38, 150, 204];
    let ocean = [40, 83, 176, 204];
    assert_eq!(color(1), Some(forest));
    assert_eq!(color(2), Some(deep));
    assert_eq!(color(3), Some(ocean));
    assert_eq!(color(4), Some(ocean));
    assert_eq!(color(8), Some(deep));
    for refused in [5, 6, 7] {
        assert_eq!(color(refused), None);
        assert_eq!(layer.region_of[refused as usize], 0);
    }
    assert_eq!(layer.region_of, [0, 1, 2, 3, 4, 0, 0, 0, 8]);
    assert!(
        layer
            .styles
            .values()
            .all(|style| !style.pulse && style.fill == 1.0)
    );
    assert_eq!(layer.thickness, Some(0.0));
    assert_eq!(layer.gradient, Some(TERRAIN_GRADIENT));
    assert_eq!(layer.channel, DEFINES.channel);
    assert_eq!(built.warnings.len(), 3, "{:?}", built.warnings);
    assert!(
        built
            .warnings
            .iter()
            .any(|line| line.contains("1 provinces unfilled: land provinces on the null terrain"))
    );
    assert!(
        built
            .warnings
            .iter()
            .any(|line| line.contains("without a terrain record"))
    );
    assert!(built.warnings.iter().any(|line| line.contains("broken")));
}

#[test]
fn water_without_an_ocean_category_is_refused_by_name() {
    let built = sources_built(false);
    let layer = &built.layer;
    assert!(layer.styles.contains_key(&2) && layer.styles.contains_key(&8));
    assert!(!layer.styles.contains_key(&3) && !layer.styles.contains_key(&4));
    assert!(
        built
            .warnings
            .iter()
            .any(|line| line.contains("2 provinces unfilled") && line.contains("no ocean"))
    );
}
