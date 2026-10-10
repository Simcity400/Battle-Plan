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

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    time::{Duration, Instant},
};

use installed_content::{
    map::{MapData, ProvinceKind},
    regions::StrategicRegion,
};
use replay::{
    Hour,
    frame::DecodedFrame,
    rows::{CountryRow, ProvinceRow, SubjectRow, tables},
};

use super::{
    gradient::NO_REGION,
    tables::{Available, core, rows},
};
// The docs link into the gradient module; the code only needs NO_REGION.
#[cfg(doc)]
use super::gradient;

/// The viewer's marker fill for a country whose recorded colour is missing this hour. It is not a
/// game colour; the warning names the countries and the table state.
pub const UNAVAILABLE_COLOR: [u8; 3] = [128, 128, 128];

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

/// The region keys and colours of the country layer, indexed by province id.
#[derive(Clone, Debug, PartialEq)]
pub struct Regions {
    /// Layer 2 region key per province id: the controller's canonical slot plus one, or
    /// [`gradient::NO_REGION`].
    pub country_of: Vec<u32>,
    /// Border material key is the canonical state-owner slot plus one; CountryLinks resolves owner
    /// identity.
    pub owner_of: Vec<u32>,
    /// State id per province id, for the border classifier.
    pub state_of: Vec<u32>,
    /// Colour per layer 2 region key.
    pub country_colors: HashMap<u32, [u8; 3]>,
    /// Limitations at this hour, in the same style as `document::Palette`.
    pub warnings: Vec<String>,
    /// Political naval attributes indexed by strategic region id.
    pub strategic_attributes: BTreeMap<u32, super::gradient::Attributes>,
}

impl Regions {
    /// Colour of a region, black for a key no country claims.
    pub fn country_color(&self, key: u32) -> [u8; 3] {
        self.country_colors.get(&key).copied().unwrap_or([0, 0, 0])
    }
}

/// The identity of one political frame: everything [`build`] reads out of it.
///
/// Country identities, links, active subject relations and colours all change the fill without
/// changing any province's controller. The links also canonicalize the border owner keys.
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
}

impl FrameKey {
    pub fn with_naval(mut self, naval: &super::naval_dominance::Fill) -> Self {
        self.naval = Some(naval.clone());
        self
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
        })
    }
}

/// One hour's compact political inputs and their derivation cost.
pub struct PoliticalLayers {
    /// Fill keys and the owner keys for the border pass.
    pub regions: std::sync::Arc<Regions>,
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
/// (`super::tables`), a recorded province the map does not define, a dangling country link, or
/// province ids that overflow the key tables.
pub fn regions(map: &MapData, frame: &DecodedFrame) -> io::Result<Regions> {
    let countries = core::<tables::Countries>(frame)?
        .iter()
        .map(|country| (country.canonical_slot, country))
        .collect::<BTreeMap<_, _>>();
    let links = core::<tables::CountryLinks>(frame)?
        .iter()
        .map(|link| (link.id, link.canonical_slot))
        .collect::<BTreeMap<_, _>>();
    let provinces = core::<tables::Provinces>(frame)?;
    let count = province_table_len(map, provinces)?;
    let mut result = Regions {
        country_of: vec![NO_REGION; count],
        owner_of: vec![NO_REGION; count],
        state_of: vec![NO_REGION; count],
        country_colors: HashMap::new(),
        warnings: Vec::new(),
        strategic_attributes: BTreeMap::new(),
    };
    let mut unresolved = Vec::new();
    let mut uncoloured = BTreeSet::new();
    let mut observed = BTreeSet::new();
    let inherited = inherited_colors(map, frame, &links, &mut result.warnings)?;
    // Stored primary colour is already cosmetic-resolved and HSV-modified; pass it directly to the
    // gradient builder.
    let appearance = rows::<tables::CountryAppearance>(frame)?;
    let gap = match &appearance {
        Available::Complete(_) => "CountryAppearance has no row for them".to_owned(),
        Available::Truncated { missing, .. } => {
            format!("CountryAppearance is truncated ({missing} missing rows)")
        }
        Available::Unavailable { reason } => format!("CountryAppearance is unavailable: {reason}"),
    };
    let recorded = appearance
        .rows()
        .iter()
        .map(|r| (r.country, r.map_color()))
        .collect::<BTreeMap<_, _>>();
    let mut colors = HashMap::new();
    for province in provinces {
        let definition = map.definitions.get(&province.id).ok_or_else(|| {
            invalid(format!(
                "recorded province {} is absent from map/definition.csv",
                province.id
            ))
        })?;
        observed.insert(province.id);
        // Owned/controller lakes belong to the country field. Province-region lookup has no terrain-
        // kind exclusion. Treating these lakes as sea adds false shore borders; unassigned lakes
        // remain outside the layer.
        if definition.kind == ProvinceKind::Sea
            || (definition.kind == ProvinceKind::Lake && province.controller == 0)
        {
            continue;
        }
        let index = province.id as usize;
        if province.owner != 0 {
            result.owner_of[index] = slot_of(&links, province.owner, "owner")?
                .checked_add(1)
                .ok_or_else(|| invalid("owner country slot exceeds the border key range"))?;
        }
        result.state_of[index] = province.state;
        if province.controller == 0 {
            unresolved.push(province.id);
            continue;
        }
        let slot = slot_of(&links, province.controller, "controller")?;
        let country = countries.get(&slot).ok_or_else(|| {
            invalid(format!(
                "political fill country link {} resolves to missing country slot {slot}",
                province.controller
            ))
        })?;
        let color = match colors.get(&slot) {
            Some(&color) => color,
            None => {
                let color = color_of(slot, &countries, &inherited, &recorded)?;
                colors.insert(slot, color);
                color
            }
        };
        let color = color.unwrap_or_else(|| {
            uncoloured.insert(String::from_utf8_lossy(&country.tag).into_owned());
            UNAVAILABLE_COLOR
        });
        // `NO_REGION` is zero, so a region key is the slot plus one and slot zero stays addressable.
        let key = slot
            .checked_add(1)
            .ok_or_else(|| invalid("country slot exceeds the gradient border region key range"))?;
        result.country_colors.insert(key, color);
        result.country_of[index] = key;
    }
    // A Provinces table that parses can still be short of the map; nothing downstream would notice,
    // so a province the hour never mentions is refused here rather than drawn unfilled.
    for &id in map.definitions.keys() {
        if id != 0 && !observed.contains(&id) {
            return Err(invalid(format!(
                "hour {} has no recorded row for map province {id}",
                frame.hour
            )));
        }
    }
    if !unresolved.is_empty() {
        result.warnings.push(format!(
            "{} land provinces have a zero controller reference and take no political fill; province ids {}",
            unresolved.len(),
            examples(unresolved.iter().map(u32::to_string))
        ));
    }
    if !uncoloured.is_empty() {
        result.warnings.push(format!(
            "{} countries have no recorded colour this hour ({gap}) and are filled grey: {}",
            uncoloured.len(),
            examples(uncoloured.into_iter())
        ));
    }
    Ok(result)
}

fn examples(items: impl Iterator<Item = String>) -> String {
    let items = items.collect::<Vec<_>>();
    let mut text = items
        .iter()
        .take(12)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    if items.len() > 12 {
        text.push_str(", …");
    }
    text
}

fn slot_of(links: &BTreeMap<u32, u32>, reference: u32, role: &str) -> io::Result<u32> {
    links.get(&reference).copied().ok_or_else(|| {
        invalid(format!(
            "political fill references missing country link {reference} for {role}"
        ))
    })
}

fn active_subject(row: &SubjectRow, hour: Hour) -> bool {
    row.cancel == 0 && (row.end_hour <= row.start_hour || hour < row.end_hour)
}

/// Resolve colour inheritance for each country this hour, retaining named unknown operands.
///
/// Follow the current autonomy state while its content definition enables `use_overlord_color`,
/// taking the active `Subjects` row's overlord at each step. Autonomy score and ratio do not enter
/// the colour selection.
#[derive(Clone, Debug, Default)]
pub(crate) struct Inheritance {
    /// Subject slot to the overlord slot whose colours it takes.
    pub(crate) inherited: BTreeMap<u32, u32>,
    /// Slots whose colour source the hour cannot decide, with the missing operand.
    pub(crate) undecided: BTreeMap<u32, String>,
    /// Slots with a current autonomy status (an `Autonomy` row).
    pub(crate) with_status: BTreeSet<u32>,
    /// Slots an active `Subjects` row names as a subject.
    pub(crate) subjects: BTreeSet<u32>,
    /// A slot without an `Autonomy` row is undecided while the table is incomplete, since it may hold
    /// a current status. `None` means an absent row is a known null status.
    pub(crate) absent: Option<String>,
}

impl Inheritance {
    /// Use the current autonomy colour source, following overlords to the end of the chain. Err names
    /// the missing operand for a reached country.
    pub(crate) fn source(&self, slot: u32) -> Result<u32, String> {
        let mut source = slot;
        let mut visited = BTreeSet::new();
        loop {
            if !visited.insert(source) {
                return Err("cycle in subject colour inheritance".to_owned());
            }
            if let Some(why) = self.undecided.get(&source) {
                return Err(format!(
                    "country slot {source}'s colour source is unknown: {why}"
                ));
            }
            if let Some(why) = &self.absent
                && !self.with_status.contains(&source)
            {
                return Err(format!(
                    "country slot {source}'s current autonomy status is unknown: {why}"
                ));
            }
            match self.inherited.get(&source) {
                Some(&overlord) => source = overlord,
                None => return Ok(source),
            }
        }
    }
}

/// This hour's [`Inheritance`].
pub(crate) fn inheritance(
    map: &MapData,
    frame: &DecodedFrame,
    links: &BTreeMap<u32, u32>,
) -> io::Result<Inheritance> {
    // Subjects.state stores the relation's initial definition. Colour uses the current autonomy
    // status, which changes independently during normal autonomy transitions.
    let autonomy = rows::<tables::Autonomy>(frame)?;
    let mut states = BTreeMap::new();
    for row in autonomy.rows() {
        let slot = slot_of(links, row.subject, "autonomy colour")?;
        if states
            .insert(slot, &row.state)
            .is_some_and(|old| old != &row.state)
        {
            return Err(invalid(format!(
                "country slot {slot} has conflicting current autonomy definitions"
            )));
        }
    }
    let subjects = rows::<tables::Subjects>(frame)?;
    let mut overlords: BTreeMap<u32, u32> = BTreeMap::new();
    let mut conflicting = BTreeSet::new();
    for row in subjects.rows() {
        if !active_subject(row, frame.hour) {
            continue;
        }
        let subject = slot_of(links, row.subject, "subject colour")?;
        let overlord = slot_of(links, row.overlord, "subject colour")?;
        if overlords
            .insert(subject, overlord)
            .is_some_and(|old| old != overlord)
        {
            conflicting.insert(subject);
        }
    }
    let mut result = Inheritance {
        with_status: states.keys().copied().collect(),
        subjects: overlords.keys().copied().collect(),
        absent: table_gap(&autonomy, "Autonomy"),
        ..Inheritance::default()
    };
    for (&subject, state) in &states {
        if state.is_empty() {
            continue;
        }
        let Some(&inherit) = map.country_colors.use_overlord_color.get(*state) else {
            result.undecided.insert(
                subject,
                format!(
                    "its current autonomy definition {} is not an autonomy state the content defines, so its use_overlord_color flag is unknown",
                    String::from_utf8_lossy(state)
                ),
            );
            continue;
        };
        if !inherit {
            continue;
        }
        if conflicting.contains(&subject) {
            return Err(invalid(format!(
                "country slot {subject} has conflicting colour overlords"
            )));
        }
        match overlords.get(&subject) {
            Some(&overlord) => {
                result.inherited.insert(subject, overlord);
            }
            None => {
                let why = match table_gap(&subjects, "Subjects") {
                    Some(gap) => format!("its current autonomy inherits, and {gap}"),
                    None => "its current autonomy inherits, but no active Subjects row names its overlord"
                        .to_owned(),
                };
                result.undecided.insert(subject, why);
            }
        }
    }
    Ok(result)
}

/// Why an incomplete table's missing row is no evidence; `None` when the table is complete.
fn table_gap<T: replay::rows::TableSchema>(table: &Available<'_, T>, name: &str) -> Option<String> {
    match table {
        Available::Complete(_) => None,
        Available::Truncated { missing, .. } => {
            Some(format!("{name} is truncated ({missing} missing rows)"))
        }
        Available::Unavailable { reason } => Some(format!("{name} is unavailable: {reason}")),
    }
}

/// The political fill's colour sources: [`inheritance`], each undecided country filled with its
/// own colour and named in `warnings`.
pub(crate) fn inherited_colors(
    map: &MapData,
    frame: &DecodedFrame,
    links: &BTreeMap<u32, u32>,
    warnings: &mut Vec<String>,
) -> io::Result<BTreeMap<u32, u32>> {
    let inheritance = inheritance(map, frame, links)?;
    if inheritance.absent.is_some() {
        for slot in inheritance.subjects.difference(&inheritance.with_status) {
            warnings.push(format!(
                "Subject slot {slot} current autonomy is unavailable; its own colour is used"
            ));
        }
    }
    for (slot, why) in &inheritance.undecided {
        warnings.push(format!(
            "Country slot {slot}'s colour source is unknown ({why}); its own colour is used"
        ));
    }
    Ok(inheritance.inherited)
}

/// Controller colour from its resolved colour source, or None when the hour has no colour for that
/// source. Follow subject colour inheritance to the end of the overlord chain.
pub(crate) fn color_source(slot: u32, inherited: &BTreeMap<u32, u32>) -> io::Result<u32> {
    let mut source = slot;
    let mut visited = BTreeSet::new();
    while let Some(&overlord) = inherited.get(&source) {
        if !visited.insert(source) {
            return Err(invalid("cycle in subject colour inheritance"));
        }
        source = overlord;
    }
    Ok(source)
}

fn color_of(
    slot: u32,
    countries: &BTreeMap<u32, &CountryRow>,
    inherited: &BTreeMap<u32, u32>,
    recorded: &BTreeMap<u32, [u8; 3]>,
) -> io::Result<Option<[u8; 3]>> {
    let source = color_source(slot, inherited)?;
    let country = countries.get(&source).ok_or_else(|| {
        invalid(format!(
            "political fill country slot {source} has no Countries row"
        ))
    })?;
    Ok(recorded.get(&country.id).copied())
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
        regions: std::sync::Arc::new(regions),
        build: start.elapsed(),
    })
}

/// Country and perspective-dependent naval fields of one recorded frame.
pub fn build_with_naval(
    map: &MapData,
    frame: &DecodedFrame,
    naval: super::naval_dominance::Fill,
) -> io::Result<PoliticalLayers> {
    let mut layers = build(map, frame)?;
    let regions = std::sync::Arc::make_mut(&mut layers.regions);
    regions.strategic_attributes = naval.attributes;
    regions.warnings.extend(naval.warnings);
    Ok(layers)
}

#[cfg(test)]
mod tests;
