
use std::collections::{BTreeMap, HashMap};

use content_reader_defines::defines::Defines;
use viewer_gpu::icon_tracks::{Place, Track};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GroupDefines {
    pub province_distance: f32,
    pub state_distance: f32,
    pub strategic_distance: f32,
    pub state_huge: f32,
    pub strategic_area_huge: f32,
    pub passes: i32,
}

impl GroupDefines {
    pub fn read(defines: &Defines) -> std::io::Result<Self> {
        let distance = |name| {
            defines
                .fixed5("NGraphics", name)
                .map(|value| (value as f64 / 100_000.0) as f32)
        };
        let int = |name| defines.int32("NGraphics", name).map(|value| value as f32);
        Ok(Self {
            province_distance: distance("MAP_ICONS_GROUP_CAM_DISTANCE")?,
            state_distance: distance("MAP_ICONS_STATE_GROUP_CAM_DISTANCE")?,
            strategic_distance: distance("MAP_ICONS_STRATEGIC_GROUP_CAM_DISTANCE")?,
            state_huge: int("MAP_ICONS_STATE_HUGE")?,
            strategic_area_huge: int("MAP_ICONS_STRATEGIC_AREA_HUGE")?,
            passes: defines.int32("NGraphics", "MAPICON_GROUP_PASSES")?,
        })
    }

    #[cfg(any(test, feature = "test-support"))]
    pub const FIXTURE: Self = Self {
        province_distance: 85.0,
        state_distance: 165.0,
        strategic_distance: 325.0,
        state_huge: 115.0,
        strategic_area_huge: 245.0,
        passes: 14,
    };

    pub fn grouping(&self, height: f32) -> Level {
        if self.passes <= 0 {
            Level::None
        } else {
            self.level(height)
        }
    }

    pub fn level(&self, height: f32) -> Level {
        if height >= self.strategic_distance {
            Level::Strategic
        } else if height >= self.state_distance {
            Level::State
        } else if height >= self.province_distance {
            Level::Province
        } else {
            Level::None
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    None,
    Province,
    State,
    Strategic,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Icon {
    pub kind: u32,
    pub owner: u32,
    pub province: u32,
    pub state: Option<u32>,
    pub region: Option<u32>,
    pub anchor: [f32; 3],
    pub track: Option<Track>,
}

#[derive(Clone, Debug, Default)]
pub struct Extents {
    states: BTreeMap<u32, f32>,
    regions: BTreeMap<u32, f32>,
    centres: BTreeMap<u32, [f32; 3]>,
    state_huge: f32,
    strategic_area_huge: f32,
    map_width: f32,
}

pub type Bounds = [f32; 4];

impl Extents {
    pub fn build(
        bounds: &BTreeMap<u32, Bounds>,
        state_of: &BTreeMap<u32, u32>,
        region_of: &BTreeMap<u32, u32>,
        defines: &GroupDefines,
        map_width: f32,
    ) -> Self {
        let mut boxes: BTreeMap<(bool, u32), Bounds> = BTreeMap::new();
        for (province, b) in bounds {
            for (is_region, key) in [
                (false, state_of.get(province)),
                (true, region_of.get(province)),
            ] {
                let Some(key) = key else { continue };
                let cell = boxes.entry((is_region, *key)).or_insert([
                    f32::MAX,
                    f32::MAX,
                    f32::MIN,
                    f32::MIN,
                ]);
                cell[0] = cell[0].min(b[0]);
                cell[1] = cell[1].min(b[1]);
                cell[2] = cell[2].max(b[2]);
                cell[3] = cell[3].max(b[3]);
            }
        }
        let mut out = Self {
            state_huge: defines.state_huge,
            strategic_area_huge: defines.strategic_area_huge,
            map_width,
            ..Self::default()
        };
        for ((is_region, key), b) in boxes {
            let extent = (b[2] - b[0]).max(b[3] - b[1]);
            if is_region {
                out.regions.insert(key, extent);
                out.centres
                    .insert(key, [(b[0] + b[2]) / 2.0, 0.0, (b[1] + b[3]) / 2.0]);
            } else {
                out.states.insert(key, extent);
            }
        }
        out
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn assert_same_bits(&self, other: &Self) {
        let values = |map: &BTreeMap<u32, f32>| {
            map.iter()
                .map(|(&id, &value)| (id, value.to_bits()))
                .collect::<Vec<_>>()
        };
        let centres = |map: &BTreeMap<u32, [f32; 3]>| {
            map.iter()
                .map(|(&id, &value)| (id, value.map(f32::to_bits)))
                .collect::<Vec<_>>()
        };
        assert_eq!(values(&self.states), values(&other.states));
        assert_eq!(values(&self.regions), values(&other.regions));
        assert_eq!(centres(&self.centres), centres(&other.centres));
        assert_eq!(
            [self.state_huge, self.strategic_area_huge, self.map_width].map(f32::to_bits),
            [other.state_huge, other.strategic_area_huge, other.map_width].map(f32::to_bits),
        );
    }

    pub fn state_is_huge(&self, state: u32) -> bool {
        self.states
            .get(&state)
            .is_some_and(|e| *e > self.state_huge)
    }

    pub fn region_is_huge(&self, region: u32) -> bool {
        self.regions
            .get(&region)
            .is_some_and(|e| *e > self.strategic_area_huge)
    }

    pub fn region_centre(&self, region: u32) -> Option<[f32; 3]> {
        self.centres.get(&region).copied()
    }
}

fn shares_place(first: &Icon, other: &Icon, at: Level, extents: &Extents) -> bool {
    let strategic = match (first.state, other.state) {
        (Some(_), Some(_)) => at == Level::Strategic,
        (None, None) => true,
        _ => false,
    };
    let mut region_was_huge = false;
    if strategic
        && let (Some(a), Some(b)) = (first.region, other.region)
        && a == b
    {
        if !extents.region_is_huge(a) {
            return true;
        }
        region_was_huge = true;
    }
    if (at == Level::State || region_was_huge)
        && let (Some(a), Some(b)) = (first.state, other.state)
        && a == b
        && !extents.state_is_huge(a)
    {
        return true;
    }
    first.province == other.province
}

pub fn may_group(
    first: &Icon,
    other: &Icon,
    at: Level,
    extents: &Extents,
    friendly: &impl Fn(u32, u32) -> bool,
) -> bool {
    if at == Level::None || first.kind != other.kind || first.owner == 0 {
        return false;
    }
    shares_place(first, other, at, extents)
        && (at == Level::Strategic
            || first.owner == other.owner
            || friendly(first.owner, other.owner))
}

#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    pub members: Vec<usize>,
    pub anchor: [f32; 3],
    pub track: Option<Track>,
}

pub fn group(
    icons: &[Icon],
    levels: &[Level],
    extents: &Extents,
    friendly: &impl Fn(u32, u32) -> bool,
) -> Vec<Group> {
    let level = |index: usize| levels.get(index).copied().unwrap_or(Level::None);
    let mut groups: Vec<Group> = Vec::new();
    if (0..icons.len()).all(|index| level(index) == Level::None) {
        groups.extend((0..icons.len()).map(|index| Group {
            members: vec![index],
            anchor: [0.0; 3],
            track: None,
        }));
        update_anchors(&mut groups, icons, levels, extents);
        return groups;
    }
    let mut by_province: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
    let mut by_state: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
    let mut by_region: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
    let mut candidates: Vec<usize> = Vec::new();
    let mut lowest: Vec<Level> = Vec::new();
    for (index, icon) in icons.iter().enumerate() {
        let at = level(index);
        candidates.clear();
        candidates.extend(
            by_province
                .get(&(icon.kind, icon.province))
                .into_iter()
                .flatten(),
        );
        let huge_region = icon
            .region
            .is_some_and(|region| extents.region_is_huge(region));
        if (at == Level::State || (at == Level::Strategic && huge_region))
            && let Some(state) = icon.state
            && !extents.state_is_huge(state)
        {
            candidates.extend(by_state.get(&(icon.kind, state)).into_iter().flatten());
        }
        if (at == Level::Strategic || icon.state.is_none())
            && !huge_region
            && let Some(region) = icon.region
        {
            candidates.extend(by_region.get(&(icon.kind, region)).into_iter().flatten());
        }
        candidates.sort_unstable();
        candidates.dedup();
        let mut joined = false;
        for &candidate in &candidates {
            let group = &mut groups[candidate];
            let first = &icons[group.members[0]];
            let admit = at.min(lowest[candidate]);
            if may_group(first, icon, admit, extents, &friendly)
                && (admit == lowest[candidate]
                    || group.members[1..]
                        .iter()
                        .all(|&member| may_group(first, &icons[member], admit, extents, &friendly)))
            {
                group.members.push(index);
                lowest[candidate] = admit;
                joined = true;
            }
        }
        if !joined {
            let id = groups.len();
            groups.push(Group {
                members: vec![index],
                anchor: [0.0; 3],
                track: None,
            });
            lowest.push(at);
            by_province
                .entry((icon.kind, icon.province))
                .or_default()
                .push(id);
            if let Some(state) = icon.state {
                by_state.entry((icon.kind, state)).or_default().push(id);
            }
            if let Some(region) = icon.region {
                by_region.entry((icon.kind, region)).or_default().push(id);
            }
        }
    }
    update_anchors(&mut groups, icons, levels, extents);
    groups
}

pub fn update_anchors(groups: &mut [Group], icons: &[Icon], levels: &[Level], extents: &Extents) {
    let anchor = |index: usize| -> [f32; 3] {
        let icon = &icons[index];
        if levels.get(index) == Some(&Level::Strategic)
            && let Some(region) = icon.region
            && !extents.region_is_huge(region)
            && let Some(centre) = extents.region_centre(region)
        {
            return centre;
        }
        icon.anchor
    };
    let moving = |index: usize| -> (Option<Track>, [f32; 3]) {
        let standing = anchor(index);
        let track = icons
            .get(index)
            .and_then(|icon| icon.track.clone())
            .filter(|_| standing == icons[index].anchor);
        (track, standing)
    };
    let mut ordered: Vec<usize> = Vec::new();
    for group in groups {
        let count = group.members.len() as f64;
        ordered.clear();
        ordered.extend_from_slice(&group.members);
        ordered.sort_unstable();
        let reference = ordered.first().map_or(0.0, |first| anchor(*first)[0]);
        let mut sum = [0.0f64; 3];
        for member in &ordered {
            let mut position = anchor(*member);
            position[0] = unwrap_x(extents.map_width, reference, position[0]);
            for axis in 0..3 {
                sum[axis] += f64::from(position[axis]);
            }
        }
        group.anchor = sum.map(|total| (total / count) as f32);
        group.anchor[0] = group.anchor[0].rem_euclid(extents.map_width);
        group.track = mean_track(&ordered, &moving, extents.map_width, reference, count);
    }
}

fn mean_track(
    members: &[usize],
    moving: &dyn Fn(usize) -> (Option<Track>, [f32; 3]),
    width: f32,
    reference: f32,
    count: f64,
) -> Option<Track> {
    let hours = members.iter().find_map(|member| moving(*member).0)?;
    let hours: Vec<i32> = hours.places().iter().map(|place| place.hour).collect();
    let mut sums = vec![[0.0f64; 3]; hours.len()];
    for member in members {
        let (track, standing) = moving(*member);
        let mine = track.filter(|track| track.places().iter().map(|place| place.hour).eq(hours.iter().copied()));
        for (at, sum) in sums.iter_mut().enumerate() {
            let mut place = mine.as_ref().map_or(standing, |track| track.places()[at].at);
            place[0] = unwrap_x(width, reference, place[0]);
            for axis in 0..3 {
                sum[axis] += f64::from(place[axis]);
            }
        }
    }
    let places: Vec<Place> = hours
        .iter()
        .zip(&sums)
        .map(|(hour, sum)| Place { hour: *hour, at: sum.map(|total| (total / count) as f32) })
        .collect();
    let track = Track::new(&places, width);
    track.moves().then_some(track)
}

pub fn unwrap_x(width: f32, reference: f32, x: f32) -> f32 {
    if !reference.is_finite() || !x.is_finite() {
        return x;
    }
    x - width * ((x - reference) / width).round()
}

#[cfg(test)]
mod tests;
