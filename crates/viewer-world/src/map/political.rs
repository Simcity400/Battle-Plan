//! The political fill: the two gradient-border layers for one recorded hour.
//!
//! [`super::gradient`] is the field generator; this module feeds it from the replay: the country
//! layer the game puts in **layer 2** per hour, and the strategic **layer 1** under it (the strategic
//! regions over the sea).
//!
//! | | layer 1 | layer 2 |
//! |---|---|---|
//! | region key | the province's strategic region, sea provinces only | the controller's canonical country slot plus one |
//! | colour | `STRATEGIC_NAVY_COLOR_NEUTRAL` | the recorded `CountryAppearance` colour, after the subject rule |
//! | thickness | `THICKNESS_STRATEGIC_REGIONS` | [`gradient::country_thickness`] of the region's area |
//! | opacity | 0; naval FX has independent control/contested channels | [`gradient::DEFAULT_REGION_OPACITY`] |
//!
//! Layer 1 geometry is static. Each political update refreshes its control and gaining-side bytes,
//! consumed through channel 2's red and alpha components by the resolved water effect.
//!
//! Sea provinces and provinces whose controller is zero are [`gradient::NO_REGION`] in layer 2; a
//! land province is [`gradient::NO_REGION`] in layer 1. A controller the hour recorded no colour for
//! keeps its region, filled [`UNAVAILABLE_COLOR`] and named in a warning, so a table gap stays
//! scoped to those countries and visible rather than blanking land.

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
use viewer_foundation::map::color_inheritance::{
    Inheritance, active_subject, inheritance, slot_of,
};

use installed_content_core::{map::MapData, map::ProvinceKind};
use installed_content_regions::regions::StrategicRegion;
#[cfg(test)]
use replay_data::rows::CountryRow;
use replay_data::{rows::ProvinceRow, rows::tables};
use replay_frame::frame::DecodedFrame;

use {
    super::gradient::NO_REGION,
    viewer_foundation::map::tables::{Available, core, rows},
};
// The docs link into the gradient module; the code only needs NO_REGION.
#[cfg(doc)]
use super::gradient;

/// The viewer's marker fill for a country whose recorded colour, or whose colour source, is
/// unknown this hour. It is not a game colour; the warnings name the countries and why.
pub const UNAVAILABLE_COLOR: [u8; 3] = [128, 128, 128];

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

pub use map_display_data::landscape::Regions;
mod fold;
pub(crate) mod relations;
pub use fold::TerritoryFold;

/// The identity of one political frame: everything [`build`] reads out of it.
///
/// Country identities, links, active subject relations and colours all change the fill without
/// changing any province's controller. The links also canonicalize the border owner keys.
#[cfg(test)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameKey {
    /// `(id, state, owner, controller)` per recorded province, in the table's own order.
    provinces: Vec<[u32; 4]>,
    countries: Vec<(u32, u32)>,
    links: Vec<(u32, u32)>,
    /// Active endpoints and current autonomy definitions affect colour, not the changing score/ratio.
    subjects: Vec<(u32, u32)>,
    autonomy: Vec<(u32, Vec<u8>)>,
    autonomy_complete: bool,
    subjects_complete: bool,
    colors: Vec<(u32, [u32; 3])>,
    colors_complete: bool,
    naval: Option<super::naval_dominance::Fill>,
    mode: Option<super::map_mode::ModeFill>,
}

#[cfg(test)]
impl FrameKey {
    pub fn with_naval(mut self, naval: &super::naval_dominance::Fill) -> Self {
        self.naval = Some(naval.clone());
        self
    }

    /// Hours share a fill only when the map mode drew the same country channel.
    pub fn with_mode(mut self, mode: &super::map_mode::ModeFill) -> Self {
        self.mode = Some(mode.clone());
        self
    }

    /// The map mode fill the key was built with.
    pub fn mode(&self) -> Option<&super::map_mode::ModeFill> {
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

/// One hour's compact political inputs and their derivation cost.
pub struct PoliticalLayers {
    /// Fill keys and the owner keys for the border pass.
    pub regions: Arc<Regions>,
    /// Wall time deriving the compact region tables.
    pub build: Duration,
}

impl PoliticalLayers {
    /// Limitations at this hour.
    pub fn warnings(&self) -> &[String] {
        &self.regions.warnings
    }

    /// Provinces this hour gives a political fill, and the distinct colours among them.
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

/// Derives the country layer's region keys and colours from one decoded frame.
///
/// Errors are named `InvalidData`: a core table that is truncated or unavailable
/// (`viewer_foundation::map::tables`), a recorded province the map does not define, a dangling country link, or
/// province ids that overflow the key tables.
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

/// The political fill's colour sources ([`inheritance`]), naming in `warnings` why any
/// country's source is undecided, as [`relations::Relations`] does: its colour is withheld.
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

/// A controller's colour source, following inheriting subjects to their overlords; `None` when
/// the hour cannot decide it.
#[cfg(test)]
pub(crate) fn color_source(slot: u32, inheritance: &Inheritance) -> io::Result<Option<u32>> {
    inheritance.known_source(slot)
}

/// A controller's recorded colour: `None` when its source is undecided, `Some(None)` when its
/// source has no recorded colour.
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

/// The loader resolves every bitmap ID through definitions, so this never scans pixels per hour.
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

/// Derives layer 1 region keys, the strategic regions over the sea, from static content.
///
/// Each sea province in the resolved strategic-region list belongs to that region. Land and lakes
/// use [`gradient::NO_REGION`] and [`gradient::NO_REGION_ALPHA`]. Field thickness and the flat navy
/// colour come from the content. Zero ordinary opacity disables the colour blend; naval FX reads
/// separate control/contested bytes.
///
/// A region naming a province the map does not define is ignored rather than refused, for the
/// reason [`super::border_pass::Topology`] gives: `map/strategicregions` is authored by hand.
pub fn strategic_regions_of(
    map: &MapData,
    strategic_regions: &BTreeMap<u32, StrategicRegion>,
) -> io::Result<Vec<u32>> {
    let count = province_table_len(map, &[])?;
    let mut region_of = vec![NO_REGION; count];
    for (&id, region) in strategic_regions {
        // `NO_REGION` is zero, so a region key is the region id plus one.
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

/// Derives compact political tables; the scene computes the field for the displayed hour.
pub fn build(map: &MapData, frame: &DecodedFrame) -> io::Result<PoliticalLayers> {
    let start = Instant::now();
    let regions = regions(map, frame)?;
    Ok(PoliticalLayers {
        regions: Arc::new(regions),
        build: start.elapsed(),
    })
}

/// Country and perspective-dependent naval fields of one recorded frame, and the map mode's
/// country channel.
#[cfg(test)]
pub fn build_with_naval(
    map: &MapData,
    frame: &DecodedFrame,
    naval: super::naval_dominance::Fill,
    mode: super::map_mode::ModeFill,
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
mod tests;

#[cfg(test)]
mod oracle;
