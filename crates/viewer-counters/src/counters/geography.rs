
use std::{collections::BTreeMap, convert::Infallible, io, sync::Arc};

use replay_data::rows::{ProvinceRow, tables};
use replay_frame::frame::DecodedFrame;
use viewer_runtime::memo::Recent;
use viewer_foundation::tables::core;

use viewer_counter_kit::counter_groups::{Bounds, Extents, GroupDefines};

pub(super) struct Geometry {
    pub state_of: BTreeMap<u32, u32>,
    pub extents: Extents,
}

impl Geometry {
    fn build(
        provinces: &[ProvinceRow],
        bounds: &BTreeMap<u32, Bounds>,
        region_of: &BTreeMap<u32, u32>,
        defines: &GroupDefines,
        map_width: f32,
    ) -> Self {
        let mut state_of = BTreeMap::new();
        for province in provinces {
            if province.state != 0 {
                state_of.insert(province.id, province.state);
            }
        }
        let extents = Extents::build(bounds, &state_of, region_of, defines, map_width);
        Self { state_of, extents }
    }
}

struct Key {
    states: Vec<(u32, u32)>,
    bounds: Arc<BTreeMap<u32, Bounds>>,
    region_of: Arc<BTreeMap<u32, u32>>,
    limits: [u32; 3],
}

impl PartialEq for Key {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.bounds, &other.bounds)
            && Arc::ptr_eq(&self.region_of, &other.region_of)
            && self.limits == other.limits
            && self.states == other.states
    }
}

pub(super) struct Memo {
    recent: Recent<Key, Geometry>,
}

impl Default for Memo {
    fn default() -> Self {
        Self {
            recent: Recent::new(),
        }
    }
}

impl Memo {
    fn geometry(
        &self,
        frame: &DecodedFrame,
        bounds: &Arc<BTreeMap<u32, Bounds>>,
        region_of: &Arc<BTreeMap<u32, u32>>,
        defines: &GroupDefines,
        map_width: f32,
    ) -> io::Result<Arc<Geometry>> {
        let provinces = core::<tables::Provinces>(frame)?;
        let key = Key {
            states: provinces.iter().map(|row| (row.id, row.state)).collect(),
            bounds: Arc::clone(bounds),
            region_of: Arc::clone(region_of),
            limits: [defines.state_huge, defines.strategic_area_huge, map_width].map(f32::to_bits),
        };
        Ok(self
            .recent
            .get_or_insert_without_waiting(key, || {
                Ok::<_, Infallible>(Arc::new(Geometry::build(
                    provinces, bounds, region_of, defines, map_width,
                )))
            })
            .unwrap_or_else(|never| match never {}))
    }
}

pub(super) fn read(
    frame: &DecodedFrame,
    content: &super::CounterContent,
) -> io::Result<Arc<Geometry>> {
    content.geography.geometry(
        frame,
        &content.bounds,
        &content.region_of,
        &content.defines.groups,
        content.map.x(),
    )
}

#[cfg(test)]
mod tests;
