use crate::movement_timing::TerrainSpeedContent;
use dashboard_text::table_access::TokenKeys;
use content_reader_regions::regions::StrategicRegion;
use std::{collections::BTreeMap, sync::Arc};

pub fn terrain(source: &str) -> TerrainSpeedContent {
    let terrain = Arc::new(content_reader_key_names::terrain::parse_definitions(source.as_bytes()).unwrap());
    let regions = BTreeMap::from([
        (
            9,
            StrategicRegion {
                id: 9,
                naval_terrain: Some("custom_water".into()),
                ..Default::default()
            },
        ),
        (
            10,
            StrategicRegion {
                id: 10,
                naval_terrain: Some("other_water".into()),
                ..Default::default()
            },
        ),
        (
            11,
            StrategicRegion {
                id: 11,
                naval_terrain: Some("unknown_water".into()),
                ..Default::default()
            },
        ),
    ]);
    let keys = ["custom_hull".to_owned(), "other_hull".to_owned()];
    TerrainSpeedContent::new(terrain, &regions, Ok(TokenKeys::new(&keys)))
}
