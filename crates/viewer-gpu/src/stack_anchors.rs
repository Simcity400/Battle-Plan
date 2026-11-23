use super::province_map::HeightField;
use viewer_foundation::unit_rules::STANDSTILL;
use content_reader_layers::ResolvedLayers;
use std::{collections::BTreeMap, io};

pub struct StackAnchors {
    pub positions: BTreeMap<(u32, u32), [f32; 3]>,
    pub scales: BTreeMap<(u32, u32), f32>,
}

impl StackAnchors {
    pub fn read(layers: &ResolvedLayers) -> io::Result<Self> {
        let mut positions = BTreeMap::new();
        let mut scales = BTreeMap::new();
        for stack in content_reader_map::positions::read_unit_stacks(layers)? {
            positions.insert((stack.province, stack.kind), [stack.x, stack.y, stack.z]);
            scales.insert((stack.province, stack.kind), stack.scale);
        }
        Ok(Self { positions, scales })
    }
}

pub fn seed_default_anchors(
    provinces: impl Iterator<Item = u32>,
    anchors: &BTreeMap<(u32, u32), [f32; 3]>,
    centroid: impl Fn(u32) -> Option<[f32; 2]>,
    heights: &HeightField<'_>,
) -> (BTreeMap<u32, [f32; 3]>, Vec<u32>) {
    let mut defaults = BTreeMap::new();
    let mut missing = Vec::new();
    for province in provinces.filter(|&province| province != 0) {
        if let Some([x, z]) = centroid(province) {
            defaults.insert(province, default_anchor([x, z], heights));
        } else if !anchors.contains_key(&(province, STANDSTILL)) {
            missing.push(province);
        }
    }
    (defaults, missing)
}

pub fn default_anchor([x, z]: [f32; 2], heights: &HeightField<'_>) -> [f32; 3] {
    let [x, z] = [(x as i32) as f32, (z as i32) as f32];
    [x, heights.native_height(x, z), z]
}
