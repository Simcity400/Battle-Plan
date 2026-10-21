//! Cross-province map-icon grouping.
//!
//! Above the content's camera thresholds, icons join groups at the mean of their
//! members' world anchors. Their windows become rows in the group's `units_box` grid.
//! the host counter renderer owns drawing; the decisions here are pure.
//!
//! Grouping levels use the province, state and strategic-region distance defines, read against
//! the camera's height for the game's camera and against each icon's own distance from a free
//! eye, so a free eye groups distant icons and keeps near ones apart; an icon joins a group only
//! at the lowest of its own and every member's level, and only if every member still shares the
//! group's place there.
//! Level zero retains individual anchors. The source array retains activation order;
//! the cursor offers a content-defined budget of sources on each frame. Groups evict
//! ineligible members before merging sources and updating their mean positions.
//! The first member supplies a group's kind and comparison operands. State and
//! strategic-region size thresholds determine when geography can merge at each level.
//! Strategic-region groups use region centres. Visible icons sort by priority and depth.

use std::collections::{BTreeMap, HashMap};

use installed_content_core::defines::Defines;

/// The grouping defines, `NDefines.NGraphics` of the matching content.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GroupDefines {
    /// `MAP_ICONS_GROUP_CAM_DISTANCE`.
    pub province_distance: f32,
    /// `MAP_ICONS_STATE_GROUP_CAM_DISTANCE`.
    pub state_distance: f32,
    /// `MAP_ICONS_STRATEGIC_GROUP_CAM_DISTANCE`.
    pub strategic_distance: f32,
    /// `MAP_ICONS_STATE_HUGE`, an integer define : a state wider or taller than
    /// this never gathers.
    pub state_huge: f32,
    /// `MAP_ICONS_STRATEGIC_AREA_HUGE`, an integer define.
    pub strategic_area_huge: f32,
    /// `MAPICON_GROUP_PASSES`: the game offers this many sources a frame, times 500
    /// after a level change. At zero or below it offers none, so nothing ever groups.
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

    /// The shipped `00_graphics.lua` values, for tests only.
    #[cfg(any(test, feature = "test-support"))]
    pub const VANILLA: Self = Self {
        province_distance: 90.0,
        state_distance: 180.0,
        strategic_distance: 350.0,
        state_huge: 100.0,
        strategic_area_huge: 220.0,
        passes: 20,
    };

    /// Grouping level at the camera height, with a zero source budget retaining
    /// individual anchors as level zero does.
    pub fn grouping(&self, height: f32) -> Level {
        if self.passes <= 0 {
            Level::None
        } else {
            self.level(height)
        }
    }

    /// The level the game reports for a camera height.
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

/// How far apart the engine gathers icons at the current camera height.
///
/// The names are the game's branches: level 1 gathers a province's
/// icons, level 2 a state's, level 3 a strategic region's.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    None,
    Province,
    State,
    Strategic,
}

/// One icon offered to the grouping pass.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Icon {
    /// The icon's grouping layer and sub-type.
    pub kind: u32,
    /// The icon's owner country as a canonical slot.
    pub owner: u32,
    /// The icon's province.
    pub province: u32,
    /// The province's state, absent for sea and lake provinces.
    pub state: Option<u32>,
    /// The province's strategic region.
    pub region: Option<u32>,
    /// The icon's own world anchor before grouping.
    pub anchor: [f32; 3],
}

/// The bounding extents the two `HUGE` defines are compared against:
/// `max(width, height)` of each state's and region's provinces in map units.
#[derive(Clone, Debug, Default)]
pub struct Extents {
    states: BTreeMap<u32, f32>,
    regions: BTreeMap<u32, f32>,
    /// Strategic region centres, for [`Level::Strategic`].
    centres: BTreeMap<u32, [f32; 3]>,
    state_huge: f32,
    strategic_area_huge: f32,
    /// The map width group means wrap at.
    map_width: f32,
}

/// A province's bounding box in map units, `[min_x, min_z, max_x, max_z]`.
pub type Bounds = [f32; 4];

impl Extents {
    /// Combines per-province bounding boxes into per-state and per-region ones.
    ///
    /// A province whose bounds are unknown contributes nothing; a region with
    /// no known province is not huge, like the engine's null template.
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

    /// Compare every stored float bit, including the wrap width and unused extents.
    #[cfg(test)]
    pub(crate) fn assert_same_bits(&self, other: &Self) {
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

    /// Whether the state extent exceeds the content's huge-state threshold.
    pub fn state_is_huge(&self, state: u32) -> bool {
        self.states
            .get(&state)
            .is_some_and(|e| *e > self.state_huge)
    }

    /// Whether the region extent exceeds the content's huge-region threshold.
    pub fn region_is_huge(&self, region: u32) -> bool {
        self.regions
            .get(&region)
            .is_some_and(|e| *e > self.strategic_area_huge)
    }

    /// The centre the game puts a level-3 icon on.
    pub fn region_centre(&self, region: u32) -> Option<[f32; 3]> {
        self.centres.get(&region).copied()
    }
}

/// The geography branches: do the two icons share the place
/// this level gathers by?
fn shares_place(first: &Icon, other: &Icon, at: Level, extents: &Extents) -> bool {
    // Two provinces that both have a state take the strategic branch only at
    // level 3; two that both have none (sea) take it at every level.
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

/// May `other` join the group `first` leads?
///
/// `friendly(a, b)` gives `b`'s relation toward `a`; it is consulted only when
/// the owners differ, below [`Level::Strategic`] and after geography accepted.
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

/// One icon group: the icons it holds and where it sits.
#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    /// Indices into the icon slice, in join order. An icon joins every group
    /// whose first member accepts it.
    pub members: Vec<usize>,
    /// The mean of the members' anchors, which is where the group draws.
    pub anchor: [f32; 3],
}

/// Rebuild from source cursor zero on a level change, then update group positions.
///
/// `levels` holds each icon's grouping level. Candidates retain creation order and must share a
/// kind and an eligible place at the icon's level: the same province; a non-huge state at state
/// level or in a huge region; or a non-huge region at region level or for an icon without a
/// state. [`may_group`] still decides admission against the group's first member, at the lowest
/// level of the icon and every member, so every member shares the first's place at the group's
/// lowest level. A newcomer below that level joins only if every member also passes against the
/// first at the newcomer's level. For the game's camera, every icon has one level and admission is
/// the game's. `friendly` comes from the recorded relation cache. Icons all ungrouped return one
/// group each.
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
        }));
        update_anchors(&mut groups, icons, levels, extents);
        return groups;
    }
    let mut by_province: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
    let mut by_state: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
    let mut by_region: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
    let mut candidates: Vec<usize> = Vec::new();
    // Each group's lowest member level.
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
        // A shared state or region is the icon's own, so its hugeness decides
        // whether that branch can accept.
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
                // Lowering the group's level: its members must share the first's place there too.
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

/// Advances the means of existing groups, retaining member and summation order. `levels` holds
/// each icon's grouping level, as [`group`] takes it.
pub fn update_anchors(groups: &mut [Group], icons: &[Icon], levels: &[Level], extents: &Extents) {
    // at level 3 an icon's own position is already its
    // strategic region's centre, so the mean below is that centre.
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
    for group in groups {
        let count = group.members.len() as f32;
        // Members astride the date line are unwrapped against the first
        // member before the sum, and the mean is folded back onto the map.
        let reference = group.members.first().map_or(0.0, |first| anchor(*first)[0]);
        let mut sum = [0.0f32; 3];
        for member in &group.members {
            let mut position = anchor(*member);
            position[0] = unwrap_x(extents.map_width, reference, position[0]);
            for axis in 0..3 {
                sum[axis] += position[axis];
            }
        }
        group.anchor = sum.map(|total| total / count);
        group.anchor[0] = group.anchor[0].rem_euclid(extents.map_width);
    }
}

/// Shifts `x` by whole map widths until it is within half a width of `reference`.
pub fn unwrap_x(width: f32, reference: f32, x: f32) -> f32 {
    if !reference.is_finite() || !x.is_finite() {
        return x;
    }
    x - width * ((x - reference) / width).round()
}

#[cfg(test)]
mod tests;
