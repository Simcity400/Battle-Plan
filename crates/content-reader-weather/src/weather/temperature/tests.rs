use super::*;
use crate::regions::StrategicRegion;

fn ordinal(name: &[u8]) -> i32 {
    match name {
        b"plains" => 1,
        b"mountain" => 4,
        b"urban" => 7,
        _ => 0,
    }
}

#[test]
fn settings_follow_the_native_loader() {
    let read = settings(
        br#"
        terrain_modifiers={ temperature={ terrain={ type=plains value=9.0 } } }
        terrain_modifiers={
            mud={ terrain={ type=plains value=0.5 } }
            temperature={
                terrain={ type="mountain" value=-7.0 }
                terrain={ type=swamp value=1.5 }
                terrain={ value=2 }
            }
            unknown_kind={ terrain={ type=urban value=3 } }
            temperature={ terrain={ type=urban value=not_a_number } terrain={ type=urban value=3.000001 } }
        }
        settings={
            temperature={
                temperature_variation=2.0
                max_temperature_change=6.0
                temperature_neighbor_smoothing=0.3
                temperature_change_neighbor_smoothing=0.5
                weather_extreme_cold={ -100 -12 }
            }
            performance={ provinces_per_update=584 regions_per_update=7 init_run_passes=14 texture_refresh_freq=2.0 }
        }
        settings={ temperature={ max_temperature_change=6.5 } }
        "#,
        ordinal,
    )
    .unwrap();
    assert_eq!(
        read,
        Settings {
            variation: 200_000,
            max_change: 650_000,
            smoothing: 30_000,
            change_smoothing: 50_000,
            provinces_per_update: 584,
            terrain: vec![
                TerrainModifier {
                    terrain: 4,
                    value: -700_000,
                },
                TerrainModifier {
                    terrain: 0,
                    value: 150_000,
                },
                TerrainModifier {
                    terrain: -1,
                    value: 200_000,
                },
                TerrainModifier {
                    terrain: 7,
                    value: 0,
                },
                TerrainModifier {
                    terrain: 7,
                    value: 300_000,
                },
            ],
        }
    );
    assert!(settings(b"settings={temperature={unknown=1}}", ordinal).is_err());
    assert_eq!(settings(b"", ordinal).unwrap(), Settings::default());
}

fn geometry(painted: &[u32], records: u32) -> ProvinceGeometry {
    ProvinceGeometry {
        width: 4,
        height: 4,
        centroids: (1..records).map(|id| (id, [0, 0])).collect(),
        bounds: painted.iter().map(|&id| (id, [0, 0, 1, 1])).collect(),
        ..Default::default()
    }
}

fn row(id: u16, terrain: &str) -> DefinitionRow {
    DefinitionRow {
        line: usize::from(id) + 1,
        id,
        rgb: [0; 3],
        kind: b"land".to_vec(),
        coastal: false,
        terrain: terrain.as_bytes().to_vec(),
        continent: 0,
    }
}

fn regions(lists: &[(u32, &[u32])]) -> StrategicRegions {
    StrategicRegions {
        regions: lists
            .iter()
            .map(|&(id, provinces)| {
                (
                    id,
                    StrategicRegion {
                        id,
                        provinces: provinces.to_vec(),
                        ..Default::default()
                    },
                )
            })
            .collect(),
        notes: Vec::new(),
    }
}

#[test]
fn province_setup_follows_initialize() {
    let rows = [
        row(0, "unknown"),
        row(1, "plains"),
        row(2, "swamp"),
        row(3, "mountain"),
        row(5, "plains"),
    ];
    let content = from_parts(
        |id| vec![id % 5 + 1, 2],
        &geometry(&[1, 3, 4, 5], 6),
        &rows,
        &regions(&[(9, &[1, 3, 1]), (2, &[3, 4, 7]), (12, &[4])]),
        Settings::default(),
        ordinal,
    )
    .unwrap();
    let provinces = &content.provinces;
    assert_eq!(provinces.len(), 6);
    assert_eq!(
        provinces.iter().map(|p| p.active).collect::<Vec<_>>(),
        [false, true, false, true, true, true]
    );
    assert_eq!(provinces[0].neighbours, Vec::<u32>::new());
    assert_eq!(provinces[3].neighbours, [4, 2]);
    assert_eq!(
        provinces.iter().map(|p| p.region).collect::<Vec<_>>(),
        [None, Some(9), None, Some(9), Some(12), None]
    );
    assert_eq!(provinces[1].terrain, Ok(Some(1)));
    assert_eq!(provinces[2].terrain, Ok(None));
    assert_eq!(provinces[3].terrain, Ok(Some(4)));
    assert!(
        provinces[4]
            .terrain
            .as_ref()
            .unwrap_err()
            .contains("declares province 5 at record 4")
    );
    assert!(
        provinces[5]
            .terrain
            .as_ref()
            .unwrap_err()
            .contains("invented")
    );
    assert_eq!(content.periods.len(), 3);

    let named = Settings {
        terrain: vec![TerrainModifier {
            terrain: 0,
            value: 1,
        }],
        ..Default::default()
    };
    let content = from_parts(
        |_| Vec::new(),
        &geometry(&[1, 2], 3),
        &rows[..3],
        &regions(&[]),
        named,
        ordinal,
    )
    .unwrap();
    assert!(
        content.provinces[2]
            .terrain
            .as_ref()
            .unwrap_err()
            .contains("null terrain")
    );
    let mut gapped = geometry(&[], 3);
    gapped.centroids.remove(&1);
    assert!(
        from_parts(
            |_| Vec::new(),
            &gapped,
            &rows,
            &regions(&[]),
            Settings::default(),
            ordinal
        )
        .is_err()
    );
}
