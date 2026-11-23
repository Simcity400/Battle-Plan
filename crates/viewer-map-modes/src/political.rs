
#[cfg(test)]
use replay_data::rows::SubjectRow;
#[cfg(test)]
use std::collections::HashMap;
use std::sync::Arc;
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    time::{Duration, Instant},
};
#[cfg(test)]
use viewer_foundation::color_inheritance::{Inheritance, inheritance, slot_of};
#[cfg(any(test, feature = "test-support"))]
use viewer_foundation::color_inheritance::active_subject;

use content_reader_map::map::{MapData, ProvinceKind};
use content_reader_regions::regions::StrategicRegion;
#[cfg(test)]
use replay_data::rows::CountryRow;
use replay_data::{rows::ProvinceRow, rows::tables};
use replay_frame::frame::DecodedFrame;

use viewer_scene_base::gradient::NO_REGION;
use viewer_foundation::tables::{Available, core, rows};
#[cfg(doc)]
use viewer_scene_base::gradient;

pub const UNAVAILABLE_COLOR: [u8; 3] = [128, 128, 128];

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

use map_display_data::landscape::Regions;
mod fold;
pub(crate) mod relations;
pub use fold::TerritoryFold;

#[cfg(any(test, feature = "test-support"))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameKey {
    provinces: Vec<[u32; 4]>,
    countries: Vec<(u32, u32)>,
    links: Vec<(u32, u32)>,
    subjects: Vec<(u32, u32)>,
    autonomy: Vec<(u32, Vec<u8>)>,
    autonomy_complete: bool,
    subjects_complete: bool,
    colors: Vec<(u32, [u32; 3])>,
    colors_complete: bool,
    naval: Option<super::naval_dominance::Fill>,
    mode: Option<map_display_data::landscape::ModeFill>,
}

#[cfg(any(test, feature = "test-support"))]
impl FrameKey {
    pub fn with_naval(mut self, naval: &super::naval_dominance::Fill) -> Self {
        self.naval = Some(naval.clone());
        self
    }

    pub fn with_mode(mut self, mode: &map_display_data::landscape::ModeFill) -> Self {
        self.mode = Some(mode.clone());
        self
    }

    pub fn mode(&self) -> Option<&map_display_data::landscape::ModeFill> {
        self.mode.as_ref()
    }

    pub fn new(frame: &DecodedFrame) -> io::Result<Self> {
        let subjects = rows::<tables::Subjects>(frame)?;
        let autonomy = rows::<tables::Autonomy>(frame)?;
        let colors = rows::<tables::CountryAppearance>(frame)?;
        Ok(Self {
            provinces: core::<tables::Provinces>(frame)?
                .iter()
                .map(|row| [row.id, row.state, row.owner, row.controller])
                .collect(),
            countries: core::<tables::Countries>(frame)?
                .iter()
                .map(|row| (row.canonical_slot, row.id))
                .collect(),
            links: core::<tables::CountryLinks>(frame)?
                .iter()
                .map(|row| (row.id, row.canonical_slot))
                .collect(),
            subjects: subjects
                .rows()
                .iter()
                .filter(|row| active_subject(row, frame.hour))
                .map(|row| (row.subject, row.overlord))
                .collect(),
            subjects_complete: subjects.is_complete(),
            autonomy: autonomy
                .rows()
                .iter()
                .map(|row| (row.subject, row.state.clone()))
                .collect(),
            autonomy_complete: autonomy.is_complete(),
            colors: colors
                .rows()
                .iter()
                .map(|r| (r.country, [r.red, r.green, r.blue]))
                .collect(),
            colors_complete: colors.is_complete(),
            naval: None,
            mode: None,
        })
    }
}

pub struct PoliticalLayers {
    pub regions: Arc<Regions>,
    pub build: Duration,
}

impl PoliticalLayers {
    pub fn warnings(&self) -> &[String] {
        &self.regions.warnings
    }

    pub fn summary(&self) -> (usize, usize) {
        let colors = self
            .regions
            .country_of
            .iter()
            .filter(|&&key| key != NO_REGION)
            .map(|key| self.regions.country_color(*key))
            .collect::<Vec<_>>();
        let distinct = colors.iter().collect::<BTreeSet<_>>().len();
        (colors.len(), distinct)
    }
}

pub fn regions(map: &MapData, frame: &DecodedFrame) -> io::Result<Regions> {
    TerritoryFold::default()
        .read(map, frame)
        .map(|value| value.paint.as_ref().clone())
}

fn examples(mut items: impl Iterator<Item = String>) -> String {
    let mut text = String::new();
    for (at, item) in items.by_ref().take(12).enumerate() {
        if at != 0 {
            text.push_str(", ");
        }
        text.push_str(&item);
    }
    if items.next().is_some() {
        text.push_str(", …");
    }
    text
}

#[cfg(test)]
pub(crate) fn inherited_colors(
    map: &MapData,
    frame: &DecodedFrame,
    links: &BTreeMap<u32, u32>,
    warnings: &mut Vec<String>,
) -> io::Result<Inheritance> {
    let inheritance = inheritance(map, frame, links)?;
    warnings.extend(
        inheritance
            .absent
            .as_deref()
            .map(relations::autonomy_gap_warning),
    );
    warnings.extend(
        inheritance
            .undecided
            .iter()
            .map(|(slot, why)| relations::undecided_warning(*slot, why)),
    );
    Ok(inheritance)
}

#[cfg(test)]
pub(crate) fn color_source(slot: u32, inheritance: &Inheritance) -> io::Result<Option<u32>> {
    inheritance.known_source(slot)
}

#[cfg(test)]
fn color_of(
    slot: u32,
    countries: &BTreeMap<u32, &CountryRow>,
    inheritance: &Inheritance,
    recorded: &BTreeMap<u32, [u8; 3]>,
) -> io::Result<Option<Option<[u8; 3]>>> {
    let Some(source) = color_source(slot, inheritance)? else {
        return Ok(None);
    };
    let country = countries.get(&source).ok_or_else(|| {
        invalid(format!(
            "political fill country slot {source} has no Countries row"
        ))
    })?;
    Ok(Some(recorded.get(&country.id).copied()))
}

fn province_table_len(map: &MapData, provinces: &[ProvinceRow]) -> io::Result<usize> {
    let largest = map
        .definitions
        .last_key_value()
        .map(|(&id, _)| id)
        .unwrap_or(0)
        .max(provinces.iter().map(|row| row.id).max().unwrap_or(0));
    usize::try_from(largest)
        .ok()
        .and_then(|id| id.checked_add(1))
        .ok_or_else(|| invalid("province ids exceed address space"))
}

pub fn strategic_regions_of(
    map: &MapData,
    strategic_regions: &BTreeMap<u32, StrategicRegion>,
) -> io::Result<Vec<u32>> {
    let count = province_table_len(map, &[])?;
    let mut region_of = vec![NO_REGION; count];
    for (&id, region) in strategic_regions {
        let key = id
            .checked_add(1)
            .ok_or_else(|| invalid("strategic region id exceeds the region key range"))?;
        for province in &region.provinces {
            let Some(definition) = map.definitions.get(province) else {
                continue;
            };
            if definition.kind == ProvinceKind::Sea
                && let Some(slot) = region_of.get_mut(*province as usize)
            {
                *slot = key;
            }
        }
    }
    Ok(region_of)
}

pub fn build(map: &MapData, frame: &DecodedFrame) -> io::Result<PoliticalLayers> {
    let start = Instant::now();
    let regions = regions(map, frame)?;
    Ok(PoliticalLayers {
        regions: Arc::new(regions),
        build: start.elapsed(),
    })
}

#[cfg(test)]
pub fn build_with_naval(
    map: &MapData,
    frame: &DecodedFrame,
    naval: super::naval_dominance::Fill,
    mode: map_display_data::landscape::ModeFill,
) -> io::Result<PoliticalLayers> {
    let mut layers = build(map, frame)?;
    let regions = Arc::make_mut(&mut layers.regions);
    regions.strategic_attributes = naval
        .attributes
        .into_iter()
        .map(|(id, attributes)| {
            id.checked_add(1)
                .map(|key| (key, attributes))
                .ok_or_else(|| invalid("strategic region id exceeds the region key range"))
        })
        .collect::<io::Result<_>>()?;
    regions.warnings.extend(naval.warnings);
    regions.warnings.extend(mode.warnings);
    regions.mode = mode.draw.map(Arc::new);
    Ok(layers)
}

#[cfg(test)]
pub(super) mod tests;

#[cfg(test)]
mod oracle;

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
