//! The political fill: the two gradient-border layers for one recorded hour.
//!
//! [`super::gradient`] is the field generator; this module is what feeds it from the replay. It
//! turns one [`DecodedFrame`] into the country layer the game puts in **layer 2**, and builds the
//! **layer 1** the game puts under it — the strategic regions over the sea — from static content
//! once. The result is cached so an hour that did not change the Provinces table does not pay for a
//! rebuild.
//!
//! What the two layers hold, read off the RenderDoc capture's own `GradientBorderChannel1` and
//! `GradientBorderChannel2` (the module header of [`super::gradient`] has the measurement):
//!
//! | | layer 1 | layer 2 |
//! |---|---|---|
//! | region key | the province's strategic region, sea provinces only | the controller's canonical country slot plus one |
//! | colour | [`gradient::NAVY_NEUTRAL_COLOR`], one flat colour for every region | [`gradient::country_map_color`] of the authored `colors.txt` colour |
//! | thickness | [`gradient::THICKNESS_STRATEGIC_REGIONS`] | [`gradient::country_thickness`] of the region's area |
//! | opacity | 0, so the layer draws nothing in this map mode | [`gradient::DEFAULT_REGION_OPACITY`] |
//!
//! Layer 1 is built because it is what the engine's texture holds, not because it shows: its
//! channel 2 green is zero over every texel of the game's own capture, and that green is the factor
//! `gradient_border_apply` mixes the whole channel in by. It is static — the strategic regions never
//! move — so [`PoliticalCache`] builds it once and keeps it across hours.
//!
//! Sea and lake provinces, and land provinces whose owner or controller reference is zero, are
//! [`gradient::NO_REGION`] in layer 2; a land province is [`gradient::NO_REGION`] in layer 1. A
//! pixel in no region of a layer carries [`gradient::NO_REGION_ALPHA`] there, which is the alpha the
//! shader draws nothing at.

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
    BranchId, Hour,
    frame::DecodedFrame,
    rows::{CountryRow, ProvinceRow, tables},
};

use super::{
    gradient::{self, GradientTextures, Layer, LayerKind, LayerPlane, NO_REGION, ProvinceMap},
    tables::core,
};

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

/// Which province field layer 1 groups by.
///
/// The political map fills by **controller** (section 1.7): the reference screenshot draws
/// Soviet-controlled Bohemia in Soviet purple, not Czechoslovak teal. [`Mode::Owner`] is the
/// viewer's own "Political" toggle, which answers the different question of who owns the ground.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// The game's political map mode.
    Controller,
    /// De jure ownership; the viewer's `Political` toggle and `--mode owner`.
    Owner,
}

impl Mode {
    /// The mode the viewer's `control` flag selects.
    pub fn from_control(control: bool) -> Self {
        if control {
            Mode::Controller
        } else {
            Mode::Owner
        }
    }

    /// The field name, for warnings.
    pub fn field(self) -> &'static str {
        match self {
            Mode::Controller => "controller",
            Mode::Owner => "owner",
        }
    }
}

/// The region key and colour tables for both layers, indexed by province id.
#[derive(Clone, Debug, PartialEq)]
pub struct Regions {
    /// Layer 1 region key per province id: the canonical country slot plus one, or
    /// [`gradient::NO_REGION`].
    pub country_of: Vec<u32>,
    /// Layer 2 region key per province id: the state id, or [`gradient::NO_REGION`].
    pub state_of: Vec<u32>,
    /// Layer 1 colour per region key, already through [`gradient::country_map_color`].
    pub country_colors: HashMap<u32, [u8; 3]>,
    /// Layer 2 colour per state id.
    pub state_colors: HashMap<u32, [u8; 3]>,
    /// Limitations at this hour, in the same style as `document::Palette`.
    pub warnings: Vec<String>,
}

impl Regions {
    /// Colour of a layer-1 region, black for a key no country claims.
    fn country_color(&self, key: u32) -> [u8; 3] {
        self.country_colors.get(&key).copied().unwrap_or([0, 0, 0])
    }
}

/// The identity of one political frame: everything [`build`] reads out of it.
///
/// Layer keys and colours depend only on the Provinces rows and the mode, so two hours with the
/// same rows produce the same textures byte for byte. The country and link tables reach the result
/// only through the colour of a slot, which is static content.
#[derive(Clone, Debug, PartialEq, Eq)]
struct FrameKey {
    mode: Mode,
    /// `(id, state, owner, controller)` per recorded province, in the table's own order.
    provinces: Vec<[u32; 4]>,
}

impl FrameKey {
    fn new(frame: &DecodedFrame, mode: Mode) -> io::Result<Self> {
        Ok(Self {
            mode,
            provinces: core::<tables::Provinces>(frame)?
                .iter()
                .map(|row| [row.id, row.state, row.owner, row.controller])
                .collect(),
        })
    }
}

/// One political frame's gradient-border textures and what they cost.
pub struct PoliticalLayers {
    pub textures: GradientTextures,
    /// The region keys the border pass re-buckets its land edges with.
    pub regions: Regions,
    /// Wall time of the [`gradient::build`] call that produced them.
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
        let distinct = colors
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len();
        (colors.len(), distinct)
    }
}

/// The political fill of the hour on screen, rebuilt only when it has to be.
///
/// [`PoliticalCache::ensure`] compares the Provinces rows and the mode against the frame it last
/// built (section "Refresh" of algorithm A: the replay never interpolates, so an hour boundary is
/// the only thing that can change the field). Nothing else invalidates it: the country colours come
/// from static content and the camera reaches the field only through the shader.
///
/// The window calls `ensure` on every repaint, so a frame that is already on screen must not even
/// pay for the row fingerprint; the branch, hour and mode it was built at answer that case first.
#[derive(Default)]
pub struct PoliticalCache {
    at: Option<(BranchId, Hour, Mode)>,
    key: Option<FrameKey>,
    layers: Option<PoliticalLayers>,
    /// Layer 1, which no hour changes; see [`strategic_layer`].
    strategic: Option<LayerPlane>,
}

impl PoliticalCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Rebuilds the layers unless the frame and mode match the cached ones.
    ///
    /// Returns whether a rebuild happened, which is what the caller uploads on.
    pub fn ensure(
        &mut self,
        map: &MapData,
        strategic_regions: &BTreeMap<u32, StrategicRegion>,
        frame: &DecodedFrame,
        mode: Mode,
    ) -> io::Result<bool> {
        let at = (frame.branch, frame.hour, mode);
        if self.at == Some(at) && self.layers.is_some() {
            return Ok(false);
        }
        let key = FrameKey::new(frame, mode)?;
        if self.key.as_ref() == Some(&key) && self.layers.is_some() {
            // A different hour that changed no province: keep the textures, note the new position.
            self.at = Some(at);
            return Ok(false);
        }
        // A failed rebuild must not leave a stale frame looking current.
        self.at = None;
        self.key = None;
        self.layers = None;
        if self.strategic.is_none() {
            self.strategic = Some(strategic_layer(map, strategic_regions)?);
        }
        let strategic = self.strategic.as_ref().expect("just built");
        let layers = build(map, frame, mode, strategic)?;
        self.at = Some(at);
        self.key = Some(key);
        self.layers = Some(layers);
        Ok(true)
    }

    /// The layers the last successful [`PoliticalCache::ensure`] built.
    pub fn layers(&self) -> Option<&PoliticalLayers> {
        self.layers.as_ref()
    }
}

/// Derives both layers' region keys and colours from one decoded frame.
///
/// Errors are named `InvalidData`: a core table that is truncated or unavailable (`super::tables`), a recorded
/// province the map does not define, a dangling country link, a country whose colour the recorded
/// content does not carry, or province ids that overflow the key tables.
pub fn regions(map: &MapData, frame: &DecodedFrame, mode: Mode) -> io::Result<Regions> {
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
        state_of: vec![NO_REGION; count],
        country_colors: HashMap::new(),
        state_colors: HashMap::new(),
        warnings: Vec::new(),
    };
    // Layer 2's colour is the state's controller, which a state split between two controllers does
    // not have; the most common key among the state's provinces wins, lowest key breaking ties.
    let mut state_votes: HashMap<u32, BTreeMap<u32, u32>> = HashMap::new();
    let mut unresolved = Vec::new();
    let mut observed = BTreeSet::new();
    for province in provinces {
        let definition = map.definitions.get(&province.id).ok_or_else(|| {
            invalid(format!(
                "recorded province {} is absent from map/definition.csv",
                province.id
            ))
        })?;
        observed.insert(province.id);
        // Sea and lake provinces have no owner, so nothing paints them (section 1.9).
        if matches!(definition.kind, ProvinceKind::Sea | ProvinceKind::Lake) {
            continue;
        }
        let index = province.id as usize;
        let reference = match mode {
            Mode::Controller => province.controller,
            Mode::Owner => province.owner,
        };
        if reference == 0 {
            unresolved.push(province.id);
        } else {
            let key = resolve(map, &links, &countries, province, reference, &mut result)?;
            result.country_of[index] = key;
            if province.state != NO_REGION {
                *state_votes
                    .entry(province.state)
                    .or_default()
                    .entry(key)
                    .or_default() += 1;
            }
        }
        result.state_of[index] = province.state;
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
    for (state, votes) in state_votes {
        let winner = votes
            .into_iter()
            .max_by_key(|&(key, count)| (count, std::cmp::Reverse(key)))
            .map(|(key, _)| key)
            .unwrap_or(NO_REGION);
        result
            .state_colors
            .insert(state, result.country_color(winner));
    }
    if !unresolved.is_empty() {
        let examples = unresolved
            .iter()
            .take(12)
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        result.warnings.push(format!(
            "{} land provinces have a zero {} reference and take no political fill; province ids {examples}{}",
            unresolved.len(),
            mode.field(),
            if unresolved.len() > 12 { ", …" } else { "" }
        ));
    }
    Ok(result)
}

/// Resolves one province's country reference to a layer-1 region key, recording its colour.
fn resolve(
    map: &MapData,
    links: &BTreeMap<u32, u32>,
    countries: &BTreeMap<u32, &CountryRow>,
    province: &ProvinceRow,
    reference: u32,
    result: &mut Regions,
) -> io::Result<u32> {
    let slot = *links.get(&reference).ok_or_else(|| {
        invalid(format!(
            "province {} references missing country link {reference}",
            province.id
        ))
    })?;
    let country = countries.get(&slot).ok_or_else(|| {
        invalid(format!(
            "province {} country link {reference} resolves to missing country slot {slot}",
            province.id
        ))
    })?;
    // `NO_REGION` is zero, so a region key is the slot plus one and slot zero stays addressable.
    let key = slot
        .checked_add(1)
        .ok_or_else(|| invalid("country slot exceeds the gradient border region key range"))?;
    if let std::collections::hash_map::Entry::Vacant(slot) = result.country_colors.entry(key) {
        let tag = String::from_utf8_lossy(&country.tag);
        let rgb = map.country_colors.rgb.get(&country.tag).ok_or_else(|| {
            let reason = map
                .country_colors
                .unavailable
                .get(&country.tag)
                .map(String::as_str)
                .unwrap_or("no static color exists in the recorded game content");
            invalid(format!("country {tag} color is unavailable: {reason}"))
        })?;
        slot.insert(gradient::country_map_color(*rgb));
    }
    Ok(key)
}

/// Length the per-province key tables need: one past the largest id the map or the frame uses.
fn province_table_len(map: &MapData, provinces: &[ProvinceRow]) -> io::Result<usize> {
    let largest = map
        .definitions
        .last_key_value()
        .map(|(&id, _)| id)
        .unwrap_or(0)
        .max(provinces.iter().map(|row| row.id).max().unwrap_or(0))
        .max(map.province_ids.iter().copied().max().unwrap_or(0));
    usize::try_from(largest)
        .ok()
        .and_then(|id| id.checked_add(1))
        .ok_or_else(|| invalid("province ids exceed address space"))
}

/// Builds layer 1, the strategic regions over the sea, from static content.
///
/// Every sea province of `map/strategicregions/*.txt` is a member of its region; land and lake
/// provinces are [`gradient::NO_REGION`], which is what the capture's layer 1 holds — its alpha is
/// [`gradient::NO_REGION_ALPHA`] over every land texel and a 150-pixel field over the sea. The
/// colour is one flat [`gradient::NAVY_NEUTRAL_COLOR`] for every region and the opacity is zero, so
/// the layer contributes no pixel in this map mode.
///
/// A region naming a province the map does not define is ignored rather than refused, for the
/// reason [`super::border_pass::Topology`] gives: `map/strategicregions` is authored by hand.
pub fn strategic_layer(
    map: &MapData,
    strategic_regions: &BTreeMap<u32, StrategicRegion>,
) -> io::Result<LayerPlane> {
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
    let color = |_: u32| gradient::NAVY_NEUTRAL_COLOR;
    let mut layer = Layer::new(LayerKind::StrategicRegion, &region_of, &color);
    layer.region_opacity = 0;
    layer.no_region_color = gradient::NAVY_NEUTRAL_COLOR;
    gradient::build_layer(
        &ProvinceMap {
            width: map.width,
            height: map.height,
            ids: &map.province_ids,
        },
        &layer,
    )
}

/// Builds both gradient-border textures for one recorded hour.
///
/// `strategic` is [`strategic_layer`]'s static plane; it becomes layer 1 unchanged, and layer 2 is
/// the country field this hour's Provinces table implies.
///
/// The country layer takes about a quarter of a second on the shipped 5632 x 2048 map, which is why
/// [`PoliticalCache`] exists; the headless dump blocks on it once per frame.
pub fn build(
    map: &MapData,
    frame: &DecodedFrame,
    mode: Mode,
    strategic: &LayerPlane,
) -> io::Result<PoliticalLayers> {
    let regions = regions(map, frame, mode)?;
    let province_map = ProvinceMap {
        width: map.width,
        height: map.height,
        ids: &map.province_ids,
    };
    let country_color = |key: u32| regions.country_color(key);
    let start = Instant::now();
    let country = gradient::build_layer(
        &province_map,
        &Layer::new(LayerKind::Country, &regions.country_of, &country_color),
    )?;
    let textures = gradient::stack(strategic, &country)?;
    let build = start.elapsed();
    Ok(PoliticalLayers {
        textures,
        regions,
        build,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use installed_content::map::{CountryColors, ProvinceDefinition};
    use replay::{
        frame::{Metrics, TableView},
        rows::{CountryLinkRow, TableId, TableRows},
    };

    /// The strategic regions of the fixture map: the one sea province is region 4.
    fn strategic_regions() -> BTreeMap<u32, StrategicRegion> {
        BTreeMap::from([(
            4,
            StrategicRegion {
                id: 4,
                name_key: "STRATEGICREGION_4".into(),
                provinces: vec![5],
                naval_terrain: None,
            },
        )])
    }

    /// Layer 1 for the fixture map, which no hour changes.
    fn strategic() -> LayerPlane {
        strategic_layer(&map(), &strategic_regions()).unwrap()
    }

    /// Four land provinces in two states plus one sea province, in a 6 x 2 image.
    fn map() -> MapData {
        let kinds = [
            (0, ProvinceKind::Land),
            (1, ProvinceKind::Land),
            (2, ProvinceKind::Land),
            (3, ProvinceKind::Land),
            (4, ProvinceKind::Land),
            (5, ProvinceKind::Sea),
        ];
        MapData {
            width: 6,
            height: 2,
            province_ids: vec![1, 1, 2, 3, 4, 5, 1, 1, 2, 3, 4, 5],
            definitions: kinds
                .into_iter()
                .map(|(id, kind)| {
                    (
                        id,
                        ProvinceDefinition {
                            id,
                            rgb: [id as u8, 0, 0],
                            kind,
                        },
                    )
                })
                .collect(),
            country_colors: CountryColors {
                rgb: [
                    (b"GER".to_vec(), [193, 171, 8]),
                    (b"SOV".to_vec(), [125, 13, 24]),
                ]
                .into(),
                unavailable: BTreeMap::new(),
            },
        }
    }

    fn province(id: u32, state: u32, owner: u32, controller: u32) -> ProvinceRow {
        ProvinceRow {
            id,
            state,
            owner,
            controller,
            victory_points: 0,
        }
    }

    fn frame(rows: Vec<ProvinceRow>) -> DecodedFrame {
        let mut tables = std::array::from_fn(|_| TableView::Unavailable {
            reason: "not needed for the political layers".into(),
        });
        tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(vec![
            CountryRow {
                id: 100,
                canonical_slot: 0,
                tag: b"GER".to_vec(),
                ..Default::default()
            },
            CountryRow {
                id: 200,
                canonical_slot: 1,
                tag: b"SOV".to_vec(),
                ..Default::default()
            },
        ]));
        tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(vec![
            CountryLinkRow {
                id: 100,
                canonical_slot: 0,
            },
            CountryLinkRow {
                id: 200,
                canonical_slot: 1,
            },
        ]));
        tables[TableId::Provinces.index()] = TableView::Complete(TableRows::Provinces(rows));
        DecodedFrame {
            branch: 1,
            hour: 42,
            tables,
            metrics: Metrics::default(),
        }
    }

    /// Germany owns everything; the Soviets control province 3 in state 11.
    fn occupied() -> DecodedFrame {
        frame(vec![
            province(1, 10, 100, 100),
            province(2, 10, 100, 100),
            province(3, 11, 100, 200),
            province(4, 11, 100, 100),
            province(5, 0, 0, 0),
        ])
    }

    #[test]
    fn controller_mode_paints_the_occupier_and_owner_mode_the_owner() {
        let map = map();
        let frame = occupied();
        let control = regions(&map, &frame, Mode::Controller).unwrap();
        // Slot plus one: Germany is 1 and the Soviet Union 2 (section 1.7).
        assert_eq!(control.country_of[1], 1);
        assert_eq!(control.country_of[3], 2);
        assert_ne!(control.country_of[1], control.country_of[3]);
        let owner = regions(&map, &frame, Mode::Owner).unwrap();
        assert_eq!(owner.country_of[3], 1);
        assert_eq!(owner.country_of[1], owner.country_of[3]);
        // Both modes leave the sea alone.
        assert_eq!(control.country_of[5], NO_REGION);
        assert_eq!(control.state_of[5], NO_REGION);
        assert!(control.warnings.is_empty());
    }

    #[test]
    fn layer_two_is_keyed_by_state_and_takes_its_controller_colour() {
        let regions = regions(&map(), &occupied(), Mode::Controller).unwrap();
        assert_eq!(regions.state_of[1], 10);
        assert_eq!(regions.state_of[2], 10);
        assert_eq!(regions.state_of[3], 11);
        assert_eq!(regions.state_of[4], 11);
        // State 10 is wholly German; state 11 is split 1-1 and the lower key wins the tie.
        assert_eq!(
            regions.state_colors[&10],
            gradient::country_map_color([193, 171, 8])
        );
        assert_eq!(regions.state_colors[&11], regions.state_colors[&10]);
        // The map colour is the authored colour through the HSV modifiers, not the raw one.
        assert_ne!(regions.country_colors[&1], [193, 171, 8]);
        assert_eq!(
            regions.country_colors[&2],
            gradient::country_map_color([125, 13, 24])
        );
    }

    #[test]
    fn a_majority_controller_wins_the_state_colour() {
        let frame = frame(vec![
            province(1, 10, 100, 200),
            province(2, 10, 100, 200),
            province(3, 10, 100, 100),
            province(4, 11, 100, 100),
            province(5, 0, 0, 0),
        ]);
        let regions = regions(&map(), &frame, Mode::Controller).unwrap();
        assert_eq!(
            regions.state_colors[&10],
            gradient::country_map_color([125, 13, 24])
        );
        assert_eq!(
            regions.state_colors[&11],
            gradient::country_map_color([193, 171, 8])
        );
    }

    #[test]
    fn an_unresolved_reference_is_named_and_takes_no_fill() {
        let frame = frame(vec![
            province(1, 10, 0, 0),
            province(2, 10, 100, 100),
            province(3, 11, 100, 100),
            province(4, 11, 100, 100),
            province(5, 0, 0, 0),
        ]);
        let regions = regions(&map(), &frame, Mode::Controller).unwrap();
        assert_eq!(regions.country_of[1], NO_REGION);
        // The state key survives: layer 2 does not depend on ownership.
        assert_eq!(regions.state_of[1], 10);
        assert!(regions.warnings[0].contains("zero controller"));
        assert!(regions.warnings[0].contains("province ids 1"));
    }

    /// A hour the collector had not finished filling still has a political map:
    /// the fill is a function of the three core tables alone, and every other
    /// table's gap is stated rather than refused (`README.md`, "Truth").
    #[test]
    fn a_truncated_non_core_table_still_renders_the_political_layer_and_is_stated() {
        let mut frame = occupied();
        frame.tables[TableId::ProvinceBuildings.index()] = TableView::Truncated {
            rows: TableRows::ProvinceBuildings(Vec::new()),
            missing: 1174,
        };
        // The layer is built, and it is the same one the complete hour builds:
        // no non-core table reaches the fill at all.
        let degraded = build(&map(), &frame, Mode::Controller, &strategic()).unwrap();
        let whole = build(&map(), &occupied(), Mode::Controller, &strategic()).unwrap();
        assert_eq!(degraded.textures.channel1, whole.textures.channel1);
        assert_eq!(degraded.textures.channel2, whole.textures.channel2);
        assert_eq!(degraded.regions.country_of, whole.regions.country_of);
        let limitations = super::super::tables::limitations(&frame);
        assert!(
            limitations.lines().iter().any(|line| {
                line.starts_with("Table ProvinceBuildings truncated (1174 missing rows): ")
                    && line.contains("naval base")
            }),
            "{:?}",
            limitations.lines()
        );
    }

    #[test]
    fn an_unavailable_core_table_is_a_named_error() {
        let mut frame = occupied();
        frame.tables[TableId::Countries.index()] = TableView::Unavailable {
            reason: "the collector had not grown its buffer yet".into(),
        };
        let error = regions(&map(), &frame, Mode::Controller)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("hour 42 Countries is unavailable"),
            "{error}"
        );
        assert!(
            error.contains("the collector had not grown its buffer yet"),
            "{error}"
        );
        assert!(
            error.contains("Provinces, CountryLinks and Countries"),
            "{error}"
        );
        // A core gap is a frame error, never one of the render's limitations.
        assert!(
            !super::super::tables::limitations(&frame)
                .lines()
                .iter()
                .any(|line| line.contains("Countries"))
        );
    }

    #[test]
    fn missing_content_and_dangling_links_are_named_errors() {
        let mut map = map();
        map.country_colors.rgb.clear();
        map.country_colors
            .unavailable
            .insert(b"GER".to_vec(), "runtime color only".into());
        assert!(
            regions(&map, &occupied(), Mode::Controller)
                .err()
                .unwrap()
                .to_string()
                .contains("country GER color is unavailable: runtime color only")
        );
        let mut frame = occupied();
        frame.tables[TableId::CountryLinks.index()] =
            TableView::Complete(TableRows::CountryLinks(Vec::new()));
        assert!(
            regions(&super::tests::map(), &frame, Mode::Controller)
                .err()
                .unwrap()
                .to_string()
                .contains("references missing country link")
        );
        let mut frame = occupied();
        frame.tables[TableId::Provinces.index()] = TableView::Truncated {
            rows: TableRows::Provinces(Vec::new()),
            missing: 3,
        };
        assert!(regions(&super::tests::map(), &frame, Mode::Controller).is_err());
    }

    #[test]
    fn an_undefined_recorded_province_is_refused() {
        let frame = frame(vec![province(99, 10, 100, 100)]);
        assert!(
            regions(&map(), &frame, Mode::Controller)
                .err()
                .unwrap()
                .to_string()
                .contains("province 99 is absent from map/definition.csv")
        );
    }

    #[test]
    fn a_map_province_the_hour_never_mentions_is_refused() {
        let mut rows = match &occupied().tables[TableId::Provinces.index()] {
            TableView::Complete(TableRows::Provinces(rows)) => rows.clone(),
            _ => unreachable!("the fixture builds a complete table"),
        };
        rows.pop();
        assert!(
            regions(&map(), &frame(rows), Mode::Controller)
                .err()
                .unwrap()
                .to_string()
                .contains("no recorded row for map province 5")
        );
    }

    #[test]
    fn the_cache_rebuilds_on_a_changed_table_or_mode_and_not_otherwise() {
        let map = map();
        let regions = strategic_regions();
        let mut cache = PoliticalCache::new();
        assert!(
            cache
                .ensure(&map, &regions, &occupied(), Mode::Controller)
                .unwrap()
        );
        assert!(cache.layers().is_some());
        // The very same frame costs nothing at all, not even the row fingerprint.
        assert!(
            !cache
                .ensure(&map, &regions, &occupied(), Mode::Controller)
                .unwrap()
        );
        // The same rows at a different hour are the same political frame.
        let mut later = occupied();
        later.hour += 1;
        assert!(
            !cache
                .ensure(&map, &regions, &later, Mode::Controller)
                .unwrap()
        );
        // The mode is part of the key.
        assert!(cache.ensure(&map, &regions, &later, Mode::Owner).unwrap());
        assert!(
            !cache
                .ensure(&map, &regions, &occupied(), Mode::Owner)
                .unwrap()
        );
        // A changed controller at the next hour is a rebuild.
        let mut liberated = frame(vec![
            province(1, 10, 100, 100),
            province(2, 10, 100, 100),
            province(3, 11, 100, 100),
            province(4, 11, 100, 100),
            province(5, 0, 0, 0),
        ]);
        liberated.hour = occupied().hour + 2;
        assert!(
            cache
                .ensure(&map, &regions, &liberated, Mode::Owner)
                .unwrap()
        );
        // A different branch is a different frame even at the same hour.
        let mut forked = occupied();
        forked.branch += 1;
        forked.hour = liberated.hour;
        assert!(cache.ensure(&map, &regions, &forked, Mode::Owner).unwrap());
    }

    #[test]
    fn a_failed_rebuild_does_not_leave_a_stale_frame_looking_current() {
        let map = map();
        let regions = strategic_regions();
        let mut cache = PoliticalCache::new();
        cache
            .ensure(&map, &regions, &occupied(), Mode::Controller)
            .unwrap();
        let mut broken = frame(vec![province(99, 10, 100, 100)]);
        broken.hour = occupied().hour + 1;
        assert!(
            cache
                .ensure(&map, &regions, &broken, Mode::Controller)
                .is_err()
        );
        assert!(cache.layers().is_none());
        // The next good frame is built again rather than served from the cache.
        assert!(
            cache
                .ensure(&map, &regions, &occupied(), Mode::Controller)
                .unwrap()
        );
    }

    #[test]
    fn the_built_textures_paint_the_country_layer_second_and_the_strategic_one_first() {
        let layers = build(&map(), &occupied(), Mode::Controller, &strategic()).unwrap();
        let textures = &layers.textures;
        assert_eq!(textures.width, 3);
        assert_eq!(textures.layer_rows(), 1);
        assert_eq!(textures.height, 4);
        // Layer 2 paints the two controllers, which is where the game puts the country field.
        assert!(
            (0..textures.width)
                .filter_map(|x| textures.channel1_texel(1, x, 0))
                .any(|texel| texel[3] > 0 && texel[..3] != [0, 0, 0])
        );
        // Layer 1 is the strategic regions over the one sea province, at the navy colour, and its
        // opacity is zero, so it draws nothing however wide its field is.
        // The texel covers map columns 4 and 5, land and sea. Layer 1 is the navy colour over
        // both, because that is its colour where it has no region too, and the alpha is the box
        // average of the land's no-region 255 and the sea boundary's 247.
        let sea = textures.channel1_texel(0, 2, 0).unwrap();
        assert_eq!(sea, [51, 63, 89, 251]);
        for x in 0..textures.width {
            assert_eq!(textures.channel2_texel(0, x, 0).unwrap()[1], 0);
        }
        // Land is in no region of layer 1 and sea is in no region of layer 2; both hold the alpha
        // the shader draws nothing at, with no colour and no opacity.
        assert_eq!(
            textures.channel1_texel(0, 0, 0),
            Some([51, 63, 89, gradient::NO_REGION_ALPHA]),
            "land is in no region of layer 1 and still carries its colour"
        );
        // The same texel in layer 2 is the country field at map column 4 boxed with the sea's
        // no-region alpha at column 5: (247 + 247 + 255 + 255) / 4 and half the opacity.
        assert_eq!(textures.channel1_texel(1, 2, 0), Some([77, 72, 33, 251]));
        assert_eq!(textures.channel2_texel(1, 2, 0), Some([0, 128, 0, 0]));
        // The state keys the border classifier reads survive; no layer is built from them.
        assert_eq!(layers.regions.state_of[1], 10);
        assert_eq!(layers.regions.state_of[3], 11);
        // Channel 2's green is the region opacity wherever layer 2 has a member.
        for x in 0..textures.width {
            let member = textures.channel1_texel(1, x, 0).unwrap()[3] != gradient::NO_REGION_ALPHA;
            let green = textures.channel2_texel(1, x, 0).unwrap()[1];
            assert_eq!(member, green > 0, "column {x}");
            assert_eq!(
                textures.channel2_texel(1, x, 0).unwrap()[2],
                0,
                "political mode never pulses a layer"
            );
        }
    }
}
