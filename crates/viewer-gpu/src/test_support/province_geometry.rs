use content_reader_map::map::MapData;
use std::collections::BTreeMap;

pub fn fixture_map(width: u32, height: u32, province_ids: &[u32]) -> MapData {
    assert_eq!(province_ids.len(), (width * height) as usize);
    MapData {
        width,
        height,
        province_ids: province_ids.to_vec(),
        definitions: BTreeMap::new(),
        country_colors: Default::default(),
    }
}

pub fn centroids(entries: &[(u32, [i32; 2])]) -> Vec<Option<[i32; 2]>> {
    let mut out = vec![None; 16];
    for &(id, c) in entries {
        out[id as usize] = Some(c);
    }
    out
}
