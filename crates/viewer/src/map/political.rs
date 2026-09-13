//! The political fill: the two gradient-border layers for one recorded hour.
//!
//! [`super::gradient`] is the field generator; this module is what feeds it from the replay. It
//! turns one [`DecodedFrame`] into the two region tables the generator wants — layer 1 keyed by the
//! province's controller (or owner, in the viewer's owner mode) and layer 2 keyed by the province's
//! state — and caches the result so an hour that did not change the Provinces table does not pay
//! for a rebuild.
//!
//! The specification is `C:\hoi4_scratch\replay_map\research\r2\R2-borders-names-icons.md`
//! sections 1.3, 1.7, 1.8 and 1.9 with its "Algorithms for implementers" A; section numbers below
//! refer to it. Readings the report could not close are marked UNVERIFIED.
//!
//! What the two layers hold (section 1.3):
//!
//! | | layer 1 | layer 2 |
//! |---|---|---|
//! | region key | the controller's canonical country slot plus one | the province's state id |
//! | colour | [`gradient::country_map_color`] of the authored `colors.txt` colour (section 1.8) | the same colour, for the state's controller (UNVERIFIED) |
//! | thickness | [`gradient::country_thickness`] of the region's area | [`gradient::THICKNESS_STATE`] |
//!
//! Sea and lake provinces, and land provinces whose owner or controller reference is zero, are
//! [`gradient::NO_REGION`] in both layers: nothing paints them and the water shader draws the sea
//! on its own (section 1.9).

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    time::{Duration, Instant},
};

use installed_content::map::{MapData, ProvinceKind};
use replay::{
    frame::DecodedFrame,
    rows::{CountryRow, ProvinceRow, tables},
};

use super::gradient::{self, GradientTextures, Layer, LayerKind, NO_REGION, ProvinceMap};

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

    /// Colour of a layer-2 region.
    fn state_color(&self, key: u32) -> [u8; 3] {
        self.state_colors.get(&key).copied().unwrap_or([0, 0, 0])
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
            provinces: frame
                .complete::<tables::Provinces>()?
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
#[derive(Default)]
pub struct PoliticalCache {
    key: Option<FrameKey>,
    layers: Option<PoliticalLayers>,
}

impl PoliticalCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Rebuilds the layers unless the frame and mode match the cached ones.
    ///
    /// Returns whether a rebuild happened, which is what the caller uploads on.
    pub fn ensure(&mut self, map: &MapData, frame: &DecodedFrame, mode: Mode) -> io::Result<bool> {
        let key = FrameKey::new(frame, mode)?;
        if self.key.as_ref() == Some(&key) && self.layers.is_some() {
            return Ok(false);
        }
        // A failed rebuild must not leave a stale frame looking current.
        self.key = None;
        self.layers = None;
        let layers = build(map, frame, mode)?;
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
/// Errors are named `InvalidData`: a required table that is truncated or unavailable, a recorded
/// province the map does not define, a dangling country link, a country whose colour the recorded
/// content does not carry, or province ids that overflow the key tables.
pub fn regions(map: &MapData, frame: &DecodedFrame, mode: Mode) -> io::Result<Regions> {
    let countries = frame
        .complete::<tables::Countries>()?
        .iter()
        .map(|country| (country.canonical_slot, country))
        .collect::<BTreeMap<_, _>>();
    let links = frame
        .complete::<tables::CountryLinks>()?
        .iter()
        .map(|link| (link.id, link.canonical_slot))
        .collect::<BTreeMap<_, _>>();
    let provinces = frame.complete::<tables::Provinces>()?;
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

/// Builds both gradient-border textures for one recorded hour.
///
/// The generator takes about a quarter of a second on the shipped 5632 x 2048 map, which is why
/// [`PoliticalCache`] exists; the headless dump blocks on it once per frame.
pub fn build(map: &MapData, frame: &DecodedFrame, mode: Mode) -> io::Result<PoliticalLayers> {
    let regions = regions(map, frame, mode)?;
    let province_map = ProvinceMap {
        width: map.width,
        height: map.height,
        ids: &map.province_ids,
    };
    let country_color = |key: u32| regions.country_color(key);
    let state_color = |key: u32| regions.state_color(key);
    let start = Instant::now();
    let textures = gradient::build(
        &province_map,
        &Layer::new(LayerKind::Country, &regions.country_of, &country_color),
        &Layer::new(LayerKind::State, &regions.state_of, &state_color),
    )?;
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
        let mut cache = PoliticalCache::new();
        assert!(cache.ensure(&map, &occupied(), Mode::Controller).unwrap());
        assert!(cache.layers().is_some());
        // The same rows at a different hour are the same political frame.
        let mut later = occupied();
        later.hour += 1;
        assert!(!cache.ensure(&map, &later, Mode::Controller).unwrap());
        // The mode is part of the key.
        assert!(cache.ensure(&map, &later, Mode::Owner).unwrap());
        assert!(!cache.ensure(&map, &occupied(), Mode::Owner).unwrap());
        // A changed controller is a rebuild even at the same hour.
        let liberated = frame(vec![
            province(1, 10, 100, 100),
            province(2, 10, 100, 100),
            province(3, 11, 100, 100),
            province(4, 11, 100, 100),
            province(5, 0, 0, 0),
        ]);
        assert!(cache.ensure(&map, &liberated, Mode::Owner).unwrap());
    }

    #[test]
    fn a_failed_rebuild_does_not_leave_a_stale_frame_looking_current() {
        let map = map();
        let mut cache = PoliticalCache::new();
        cache.ensure(&map, &occupied(), Mode::Controller).unwrap();
        let broken = frame(vec![province(99, 10, 100, 100)]);
        assert!(cache.ensure(&map, &broken, Mode::Controller).is_err());
        assert!(cache.layers().is_none());
        // The next good frame is built again rather than served from the cache.
        assert!(cache.ensure(&map, &occupied(), Mode::Controller).unwrap());
    }

    #[test]
    fn the_built_textures_carry_the_two_layers_at_half_resolution() {
        let layers = build(&map(), &occupied(), Mode::Controller).unwrap();
        let textures = &layers.textures;
        assert_eq!(textures.width, 3);
        assert_eq!(textures.layer_rows(), 1);
        assert_eq!(textures.height, 4);
        // Layer 1 paints the two controllers; layer 2 paints the two states.
        assert!(
            textures
                .channel1
                .iter()
                .any(|texel| texel[3] > 0 && texel[..3] != [0, 0, 0])
        );
        // Channel 2's green is the region opacity wherever the field reaches.
        for (index, texel) in textures.channel2.iter().enumerate() {
            assert_eq!(texel[1] > 0, textures.channel1[index][3] > 0);
            assert_eq!(texel[2], 0, "political mode never pulses a layer");
        }
    }
}
