use content_reader_map::province_geometry::native_fixed_sqrt;
use content_reader_regions::regions::StrategicRegion;
use content_reader_states::states::StateDefinition;
use std::collections::{BTreeMap, BTreeSet};

pub fn battle_name_candidates(
    regions: &BTreeMap<u32, StrategicRegion>,
    states: &BTreeMap<u32, StateDefinition>,
    valid: &dyn Fn(u32) -> bool,
) -> BTreeMap<u32, Vec<u32>> {
    let lists: BTreeMap<u32, Vec<u32>> = states
        .iter()
        .map(|(id, state)| {
            let mut seen = BTreeSet::new();
            let list = state
                .provinces
                .iter()
                .copied()
                .filter(|province| valid(*province) && seen.insert(*province))
                .collect();
            (*id, list)
        })
        .collect();
    let mut state_of = BTreeMap::new();
    for (id, list) in &lists {
        for province in list {
            state_of.insert(*province, *id);
        }
    }
    regions
        .iter()
        .map(|(id, region)| {
            let mut seen = BTreeSet::new();
            let mut provinces = Vec::new();
            for province in region.provinces.iter().filter(|province| valid(**province)) {
                if let Some(state) = state_of.get(province)
                    && seen.insert(*state)
                {
                    provinces.extend(&lists[state]);
                }
            }
            (*id, provinces)
        })
        .collect()
}

fn wrapped_distance(width: i32, from: (u32, [i32; 2]), to: (u32, [i32; 2])) -> i64 {
    if from.0 == to.0 {
        return 0;
    }
    let dx = from.1[0].wrapping_sub(to.1[0]);
    let dy = from.1[1].wrapping_sub(to.1[1]);
    let across = dx.wrapping_abs();
    let dx = if across <= width / 2 {
        dx
    } else {
        width.wrapping_sub(across)
    };
    native_fixed_sqrt(dy.wrapping_mul(dy).wrapping_add(dx.wrapping_mul(dx)) as u32)
}

pub struct Picker<'a> {
    pub width: i32,
    pub factor: i64,
    pub cutoff: i64,
    pub position: &'a dyn Fn(u32) -> [i32; 2],
    pub victory_points: &'a dyn Fn(u32) -> Result<i32, String>,
}

impl Picker<'_> {
    pub fn pick(&self, battle: u32, candidates: &[u32]) -> Result<Option<u32>, String> {
        let at = (battle, (self.position)(battle));
        let mut best: Option<(u32, i64)> = None;
        for &candidate in candidates {
            let distance =
                wrapped_distance(self.width, at, (candidate, (self.position)(candidate)));
            let raw = i64::from((self.victory_points)(candidate)?)
                .wrapping_mul(100_000)
                .wrapping_mul(self.factor)
                / 100_000;
            let score =
                if distance == 0 || matches!(raw, i64::MAX | i64::MIN | -0x7fff_ffff_ffff_ffff) {
                    0xffff_ffff
                } else {
                    raw.wrapping_mul(100_000) / distance
                };
            if best.is_none_or(|(_, top)| top < score) {
                best = Some((candidate, score));
            }
        }
        Ok(best
            .filter(|(_, score)| self.cutoff <= *score)
            .map(|(province, _)| province))
    }
}

#[cfg(test)]
mod tests;
