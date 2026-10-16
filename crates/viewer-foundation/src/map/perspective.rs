//! Which country the map is viewed as, whether fog of war is on, and what that
//! country sees.
//!
//! Choice retains a viewing country and a separate fog switch. Disabling fog still draws from a
//! country viewpoint; no omniscient view is available.
//!
//! Fog off supplies full unit, province and country intel and a clear fog texture. Shared vision
//! remains country-specific, as do counter silhouettes, allegiance and battle plans.
//!
//! The unit byte, for a unit the viewer does not own:
//!
//! Shared vision supplies detection level three; otherwise use recorded UnitIntel. Naval levels map
//! to base values 0, 100, 150 and 250. Land levels map to 0, 150 and 250 and take the maximum with
//! province visibility. Apply the viewing-country modifier with fixed-point integer arithmetic and
//! clamp to byte range.
//!
//! Counters draw in full from 250 and as a question mark from 150. The province
//! bytes are rebuilt per viewer by [`super::visibility::derive`].

use std::sync::Arc;
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt, io,
};

use replay::{
    frame::DecodedFrame,
    rows::{RelationLinkRow, TableId, tables},
};

use super::{
    allegiance,
    tables::{Available, core, rows},
    visibility::{self, AirDefines, FogContent, NavalDefines, Relations},
};

/// Branch, hour and viewing choice, in addition to the memo's table identities.
pub type VisionKey = (replay::BranchId, replay::Hour, Choice);

/// Derive visibility only on a miss in the shared table-identity memo.
pub fn cached_vision(
    frame: &DecodedFrame,
    perspective: &Choice,
    fog: Result<&FogContent, &'static str>,
    memo: &super::memo::Keyed<Vision, VisionKey>,
) -> io::Result<Arc<Vision>> {
    let key = (frame.branch, frame.hour, perspective.clone());
    memo.get_or_build(frame, key, |frame| {
        let links: BTreeMap<u32, u32> = core::<tables::CountryLinks>(frame)?
            .iter()
            .map(|link| (link.id, link.canonical_slot))
            .collect();
        let tags: BTreeMap<u32, String> = core::<tables::Countries>(frame)?
            .iter()
            .map(|country| {
                (
                    country.canonical_slot,
                    String::from_utf8_lossy(&country.tag).into_owned(),
                )
            })
            .collect();
        Vision::read(frame, &links, &tags, perspective, fog).map(Arc::new)
    })
}

/// The detection level a land unit needs to draw in full.
pub const LAND_FULL_LEVEL: i8 = 2;
/// The detection level a land unit needs for a counter at all.
pub const LAND_SEEN_LEVEL: i8 = 1;
/// The detection level a naval unit needs to draw in full.
pub const NAVAL_FULL_LEVEL: i8 = 3;
/// The detection level a naval unit needs for a counter at all (level 1 reaches
/// only 100 on the naval ladder).
pub const NAVAL_SEEN_LEVEL: i8 = 2;
/// The unit byte at which a counter draws in full.
pub const FULL_VISIBILITY: u16 = 250;
/// The unit byte at which a stack has a counter at all.
pub const SHOWN_VISIBILITY: u16 = 150;
/// Province-byte threshold below which province-anchored map objects hide.
pub const MAP_OBJECT_VISIBILITY: u8 = 50;

/// How much of one stack this perspective is shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sight {
    /// Byte 250 or more: bars, glyphs and count.
    Full,
    /// Byte 150..250: the game's question mark, no bars.
    Partial,
    /// Below 150: the unit never enters the stack-icon bucket.
    None,
}

/// Which country the map is viewed as.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Country {
    /// The recording client's viewpoint, resolved by [`allegiance::perspective`].
    #[default]
    Recorded,
    /// A named country tag.
    Tag(String),
}

impl Country {
    /// Parses `--perspective <value>`: `recorded`, or a tag.
    /// `omniscient` is refused by name: the game has no such view.
    pub fn parse(value: &str) -> io::Result<Self> {
        match value.trim() {
            "" => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "--perspective takes recorded or a country tag",
            )),
            value if value.eq_ignore_ascii_case("omniscient") => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "--perspective omniscient is gone: the game has no omniscient view. Choose a \
                 viewing country (recorded or a tag) and --fog off to draw every stack in full",
            )),
            value if value.eq_ignore_ascii_case("recorded") => Ok(Country::Recorded),
            value => Ok(Country::Tag(value.to_ascii_uppercase())),
        }
    }
}

impl fmt::Display for Country {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Country::Recorded => f.write_str("recorded"),
            Country::Tag(tag) => f.write_str(tag),
        }
    }
}

/// Tables used by [`Vision::read`], including links and tags used through `allegiance` and
/// `visibility`. The window retains one hour's vision while these tables share identity
/// (`crate::map::memo`), and builds a frame containing only these inputs. Missing tables are
/// unavailable rather than stale.
pub const VISION_TABLES: &[TableId] = &[
    TableId::CountryLinks,
    TableId::Countries,
    TableId::LocalPlayer,
    TableId::FactionMembers,
    TableId::WarRelations,
    TableId::RelationLinks,
    TableId::Subjects,
    TableId::UnitIntel,
    TableId::CountryVisibility,
    TableId::CountryIntel,
    TableId::CountryCounterInputs,
    TableId::ReservedCountryNaming,
    TableId::Provinces,
    TableId::Divisions,
    TableId::DivisionMovement,
    TableId::RadarCoverage,
    TableId::SimulationFeatures,
    TableId::StateVisibility,
    TableId::States,
    TableId::Fleets,
    TableId::TaskForces,
    TableId::StationedUnits,
    TableId::LandTheatres,
    TableId::TheatreAreas,
    TableId::VolunteerHosts,
];

/// The game's two map controls: a viewing country and the fog-of-war switch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Choice {
    pub country: Country,
    /// fog of war on, the game's default.
    pub fog: bool,
}

impl Default for Choice {
    fn default() -> Self {
        Self {
            country: Country::Recorded,
            fog: true,
        }
    }
}

impl Choice {
    /// Parses `--fog on|off`.
    pub fn parse_fog(value: &str) -> io::Result<bool> {
        match value.trim() {
            value if value.eq_ignore_ascii_case("on") => Ok(true),
            value if value.eq_ignore_ascii_case("off") => Ok(false),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "--fog takes on or off",
            )),
        }
    }
}

impl fmt::Display for Choice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}, fog of war {}",
            self.country,
            if self.fog { "on" } else { "off" }
        )
    }
}

/// Why a vision-sharing test cannot prove its negative.
/// A proven positive never carries one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SharingUnknown {
    /// A country reference `CountryLinks` does not place.
    Reference,
    /// The viewer's relations are not whole this hour: `Relations::read`
    /// refused on this input, so only the viewer is known to share.
    Relations(&'static str),
    /// A census the test reads is not whole this hour; the text names the
    /// table and why (a combat's country list, the combats in a province, or
    /// a side census).
    Census(&'static str),
}

impl SharingUnknown {
    /// The missing input, for a report line.
    pub fn describe(self) -> String {
        match self {
            Self::Reference => "a country reference is not in CountryLinks".into(),
            Self::Relations(input) => {
                format!("the viewer's relations are not whole this hour ({input})")
            }
            Self::Census(input) => input.into(),
        }
    }

    /// The whole clause for a counter line: a sharing input is reported as
    /// unknown, a census names itself.
    pub fn clause(self) -> String {
        match self {
            Self::Census(input) => input.into(),
            _ => format!("vision sharing unknown, {}", self.describe()),
        }
    }
}

/// How the viewing country was settled for this hour.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Resolution {
    /// The recording client's stored viewpoint.
    #[default]
    Recorded,
    /// The tag the caller named.
    Requested,
    /// No client viewpoint available: the viewer takes the lowest canonical slot.
    NoPlayer,
    /// The named tag has no country this hour; the default is used instead.
    Absent(String),
}

/// The viewing country's slot this hour, and how it was chosen. The only
/// error is an hour with no country at all.
pub fn viewer(
    frame: &DecodedFrame,
    links: &BTreeMap<u32, u32>,
    tags: &BTreeMap<u32, String>,
    country: &Country,
) -> io::Result<(u32, Resolution)> {
    let default = || -> io::Result<(u32, bool)> {
        match allegiance::perspective(frame, links)? {
            Some(slot) => Ok((slot, true)),
            None => tags
                .keys()
                .next()
                .map(|slot| (*slot, false))
                .ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("hour {} records no country to view the map as", frame.hour),
                    )
                }),
        }
    };
    Ok(match country {
        Country::Recorded => match default()? {
            (slot, true) => (slot, Resolution::Recorded),
            (slot, false) => (slot, Resolution::NoPlayer),
        },
        // Tags compare case-insensitively: a mod may define a lower-case tag.
        Country::Tag(tag) => match tags.iter().find(|(_, name)| name.eq_ignore_ascii_case(tag)) {
            Some((slot, _)) => (*slot, Resolution::Requested),
            None => (default()?.0, Resolution::Absent(tag.clone())),
        },
    })
}

/// What one country can see this hour.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Vision {
    ours: u32,
    resolution: Resolution,
    fog: bool,
    /// The first unit-sight input the hour does not hold.
    undecided: Option<&'static str>,
    /// Slots whose units this one sees in full by relation, including itself.
    shares: BTreeSet<u32>,
    /// Canonical slots in active wars from WarRelations; empty when relations_gap is set.
    wars: BTreeSet<u32>,
    /// The relation input `Relations::read` lacked this hour; `shares` then
    /// holds only the viewer, and no other slot's negative is proven.
    relations_gap: Option<&'static str>,
    /// The first `UnitIntel` entry keyed to this country, per unit.
    levels: BTreeMap<u64, i8>,
    /// The viewer's modifier 0x24B.
    unit_modifier: i64,
    /// Rebuilt province bytes for this country (fog on only).
    visibility: BTreeMap<u32, u8>,
    /// The input the province rebuild lacked, when it failed.
    visibility_gap: Option<&'static str>,
    /// A land unit invading from sea takes intel from its landing province.
    sight_provinces: BTreeMap<u64, u32>,
    /// Tables used in part, named for the limitation lines.
    truncated: Vec<&'static str>,
    /// This country's `CountryIntel` air lane per target slot; `None` unless complete.
    air_intel: Option<BTreeMap<u32, i64>>,
    air_defines: Option<AirDefines>,
    /// This country's `CountryIntel` navy lane per target slot; `None` unless complete.
    navy_intel: Option<BTreeMap<u32, i64>>,
    naval_defines: Option<NavalDefines>,
    /// Slots whose relation status grants this country air-base access; `None` unless `RelationLinks`
    /// is complete.
    air_access: Option<BTreeSet<u32>>,
    /// Province id to its controller's canonical slot.
    province_controllers: BTreeMap<u32, u32>,
}

impl Vision {
    /// Resolves `choice` against one decoded hour. `fog` is the content half
    /// of the fog rebuild, or the reason it failed to load. The only error is
    /// an hour with no country at all.
    pub fn read(
        frame: &DecodedFrame,
        links: &BTreeMap<u32, u32>,
        tags: &BTreeMap<u32, String>,
        choice: &Choice,
        fog: Result<&FogContent, &'static str>,
    ) -> io::Result<Self> {
        let (ours, resolution) = viewer(frame, links, tags, &choice.country)?;
        let mut vision = Self {
            ours,
            resolution,
            fog: choice.fog,
            ..Self::default()
        };

        let relations = Relations::read(frame, links);
        vision.relate(
            relations.as_ref().map_err(|reason| *reason),
            tags.keys().copied(),
        );
        if let Err(reason) = &relations {
            vision.undecided = Some(reason);
        }

        let intel = rows::<tables::UnitIntel>(frame)?;
        match &intel {
            Available::Unavailable { .. } => {
                vision.undecided.get_or_insert("UnitIntel");
            }
            Available::Truncated { .. } => vision.truncated.push("UnitIntel"),
            Available::Complete(_) => {}
        }
        // Use the first matching recorded viewer-level entry.
        let mut ordered: Vec<_> = intel.rows().iter().filter(|r| r.viewer == ours).collect();
        ordered.sort_by_key(|r| (r.unit, r.ordinal));
        for row in ordered {
            vision.levels.entry(row.unit).or_insert(row.level);
        }
        match frame.complete::<tables::CountryVisibility>() {
            Ok(rows) => {
                vision.unit_modifier = rows
                    .iter()
                    .find(|r| links.get(&r.country) == Some(&ours))
                    .map_or(0, |r| r.unit_modifier)
            }
            Err(_) => {
                vision.undecided.get_or_insert("CountryVisibility");
            }
        }

        let intel_rows = rows::<tables::CountryIntel>(frame)?;
        if matches!(intel_rows, Available::Truncated { .. }) {
            vision.truncated.push("CountryIntel");
        }
        if let Available::Complete(rows) = &intel_rows {
            // Only the viewer's own key-sorted slice, under each of its references.
            let (mut air, mut navy) = (BTreeMap::new(), BTreeMap::new());
            for (&raw, _) in links.iter().filter(|(_, slot)| **slot == ours) {
                for row in visibility::owner_rows(rows, raw) {
                    air.entry(row.target_index).or_insert(row.air);
                    navy.entry(row.target_index).or_insert(row.navy);
                }
            }
            vision.air_intel = Some(air);
            vision.navy_intel = Some(navy);
        }
        vision.air_defines = fog
            .ok()
            .and_then(|content| AirDefines::read(frame, &content.defines).ok());
        vision.naval_defines = fog
            .ok()
            .and_then(|content| NavalDefines::read(frame, &content.defines).ok());
        if let Ok(rows) = frame.complete::<tables::RelationLinks>() {
            vision.air_access = Some(
                rows.iter()
                    .filter(|r| {
                        r.links & RelationLinkRow::AIR_BASE_ACCESS != 0
                            && links.get(&r.country) == Some(&ours)
                    })
                    .map(|r| r.target_index)
                    .collect(),
            );
        }
        if let Ok(provinces) = frame.complete::<tables::Provinces>() {
            vision.province_controllers = provinces
                .iter()
                .filter_map(|p| Some((p.id, *links.get(&p.controller)?)))
                .collect();
        }

        if choice.fog {
            let rebuilt = fog.and_then(|content| {
                let relations = relations?.with_attaches(frame, links)?;
                let bytes = visibility::derive(frame, links, ours, content, &relations)?;
                let sight = invasion_sight(frame, links, &content.topology, &relations, &vision)?;
                Ok((bytes, sight))
            });
            match rebuilt {
                Ok((bytes, sight)) => {
                    vision.visibility = bytes;
                    vision.sight_provinces = sight;
                }
                Err(reason) => vision.visibility_gap = Some(reason),
            }
        }
        Ok(vision)
    }

    /// The viewer and every slot of `slots` that shares vision with it, and
    /// the slots at war with it; without relations, only the viewer.
    fn relate(
        &mut self,
        relations: Result<&Relations, &'static str>,
        slots: impl Iterator<Item = u32>,
    ) {
        let ours = self.ours;
        self.shares.insert(ours);
        match relations {
            Ok(relations) => {
                self.shares
                    .extend(slots.filter(|&slot| relations.shared(slot, ours)));
                self.wars
                    .extend(relations.enemies(ours).filter(|&slot| slot != ours));
            }
            Err(reason) => self.relations_gap = Some(reason),
        }
    }

    /// The viewing country's canonical slot.
    pub fn ours(&self) -> u32 {
        self.ours
    }

    /// How the viewing country was settled.
    pub fn resolution(&self) -> &Resolution {
        &self.resolution
    }

    /// Whether fog of war is on.
    pub fn fog(&self) -> bool {
        self.fog
    }

    /// The unit-sight input missing with fog on. An unavailable table is not
    /// evidence that it is empty, so every stack is then drawn as fog off
    /// draws it and the limitation names the table.
    pub fn undecided(&self) -> Option<&'static str> {
        self.undecided.filter(|_| self.fog)
    }

    /// The input the province fog rebuild lacked, with fog on. The map then
    /// draws no fog, and land stacks are drawn in full, as an undecided hour
    /// draws every stack: the detection ladder alone would hide stacks the
    /// missing province term may reveal.
    pub fn visibility_gap(&self) -> Option<&'static str> {
        self.visibility_gap.filter(|_| self.fog)
    }

    /// Tables used in part, with fog on.
    pub fn truncated(&self) -> &[&'static str] {
        if self.fog { &self.truncated } else { &[] }
    }

    /// Provinces with a nonzero rebuilt byte.
    pub fn revealed_provinces(&self) -> usize {
        self.visibility.values().filter(|&&byte| byte > 0).count()
    }

    /// The province bytes for the intel texture, or `None` when no fog is drawn.
    pub fn province_fog(&self) -> Option<&BTreeMap<u32, u8>> {
        (self.fog && self.visibility_gap.is_none()).then_some(&self.visibility)
    }

    /// The province byte as the engine's direct readers see it: `0xff` with fog
    /// off, `None` when the rebuild failed or the province is unknown.
    pub fn province_byte(&self, province: u32) -> Option<u8> {
        if self.fog {
            self.visibility.get(&province).copied()
        } else {
            Some(u8::MAX)
        }
    }

    /// A viewer that shares vision with `shares` (itself included), fog on.
    #[cfg(any(test, feature = "test-support"))]
    pub fn sharing(ours: u32, shares: impl IntoIterator<Item = u32>) -> Self {
        let mut vision = Self {
            ours,
            fog: true,
            shares: shares.into_iter().collect(),
            ..Self::default()
        };
        vision.shares.insert(ours);
        vision
    }

    /// This viewer with fog of war off.
    #[cfg(any(test, feature = "test-support"))]
    pub fn without_fog(mut self) -> Self {
        self.fog = false;
        self
    }

    /// This viewer in an hour whose `UnitIntel` census was truncated after `levels`.
    #[cfg(any(test, feature = "test-support"))]
    pub fn with_truncated_unit_intel(
        mut self,
        levels: impl IntoIterator<Item = (u64, i8)>,
    ) -> Self {
        self.levels = levels.into_iter().collect();
        self.truncated.push("UnitIntel");
        self
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn set_visibility(&mut self, bytes: impl IntoIterator<Item = (u32, u8)>) {
        self.visibility = bytes.into_iter().collect();
        self.visibility_gap = None;
    }

    /// Map buildings hide below MAP_OBJECT_VISIBILITY unless always_shown.
    pub fn sees_province_object(&self, province: u32, always_shown: bool) -> Option<bool> {
        if always_shown {
            return Some(true);
        }
        self.province_byte(province)
            .map(|byte| byte >= MAP_OBJECT_VISIBILITY)
    }

    /// Air-port detail level from authored thresholds and the simulation feature. Also used for
    /// carrier-base summaries.
    pub fn air_port_level(&self, owner: u32) -> Option<u8> {
        let shared = self.shares.contains(&owner);
        if self.fog && self.undecided.is_some() && !shared {
            return None;
        }
        let raw = self
            .air_intel
            .as_ref()
            .map(|lanes| lanes.get(&owner).copied().unwrap_or(0));
        self.air_defines.as_ref()?.level(raw, shared, self.fog)
    }

    /// Air-base detail above level zero.
    pub fn sees_air_port_detail(&self, owner: u32) -> Option<bool> {
        self.air_port_level(owner).map(|level| level > 0)
    }

    /// Carrier-base count estimate from the viewer's air intel lane. Its wrapping clock/country/base
    /// seed is independent of the controller whose intel is queried. Missing relations or an unread
    /// intel lane stay unknown.
    pub fn air_count_range(
        &self,
        controller: u32,
        count: i32,
        seed: u32,
        hour: i32,
    ) -> Option<([i32; 2], bool)> {
        let shared = self.shares_vision_or_fog(controller).ok()?;
        let raw = self
            .air_intel
            .as_ref()
            .map(|lanes| lanes.get(&controller).copied().unwrap_or(0));
        self.air_defines
            .as_ref()?
            .count_range(count, raw, shared, self.fog, seed, hour)
    }

    /// Air-base icons are admitted by access to their controller, province visibility of at least 50,
    /// or the air-intel gate. A positive term decides; otherwise any unknown term leaves admission
    /// undecided.
    pub fn sees_air_base(&self, province: u32, controller: u32) -> Option<bool> {
        if !self.fog || controller == self.ours {
            return Some(true);
        }
        let access = self
            .air_access
            .as_ref()
            .map(|set| set.contains(&controller));
        let raw = self
            .air_intel
            .as_ref()
            .map(|lanes| lanes.get(&controller).copied().unwrap_or(0));
        let intel = self
            .air_defines
            .as_ref()
            .and_then(|d| d.show_ports(raw, self.shares.contains(&controller), self.fog));
        let byte = self
            .province_byte(province)
            .map(|byte| byte >= MAP_OBJECT_VISIBILITY);
        if access == Some(true) || intel == Some(true) || byte == Some(true) {
            return Some(true);
        }
        if self.undecided.is_some() {
            return None;
        }
        match (access, intel, byte) {
            (Some(false), Some(false), Some(false)) => Some(false),
            _ => None,
        }
    }

    /// The recorded controller's canonical slot for a province.
    pub fn province_controller(&self, province: u32) -> Option<u32> {
        self.province_controllers.get(&province).copied()
    }

    /// Shared vision admits our own country, faction partners, overlords and subjects, with a war veto
    /// independent of fog. A negative result requires complete relations; otherwise only the viewer
    /// is proven and `Err` names the missing input.
    pub fn shares_vision_proven(&self, slot: u32) -> Result<bool, SharingUnknown> {
        if self.shares.contains(&slot) {
            return Ok(true);
        }
        self.relations_gap
            .map_or(Ok(false), |input| Err(SharingUnknown::Relations(input)))
    }

    /// The country's shared-vision result, with fog off short-circuiting to true. Ordinary shared vision
    /// remains independent.
    pub fn shares_vision_or_fog(&self, slot: u32) -> Result<bool, SharingUnknown> {
        if self.fog {
            self.shares_vision_proven(slot)
        } else {
            Ok(true)
        }
    }

    /// How much of one land unit the viewer is shown; `province` is where it stands.
    pub fn sees_land(&self, unit: u64, owner: u32, province: u32) -> Sight {
        if self.in_full(owner) || self.visibility_gap.is_some() {
            return Sight::Full;
        }
        let ladder = match self.detection(unit, owner) {
            level if level >= LAND_FULL_LEVEL => FULL_VISIBILITY,
            level if level >= LAND_SEEN_LEVEL => SHOWN_VISIBILITY,
            _ => 0,
        };
        let province = self.sight_provinces.get(&unit).copied().unwrap_or(province);
        let base = match self.visibility.get(&province) {
            Some(&byte) => ladder.max(u16::from(byte)),
            None => ladder,
        };
        sight(visibility::unit_byte(base, self.unit_modifier))
    }

    /// Land unit intel without the counter fallback for unavailable fog or detection operands.
    pub fn land_byte_known(&self, unit: u64, owner: u32, province: u32) -> Option<u16> {
        if !self.fog || owner == self.ours {
            return Some(u16::from(u8::MAX));
        }
        if self.undecided.is_some()
            || self.visibility_gap.is_some()
            || (!self.shares.contains(&owner) && self.relations_gap.is_some())
            || (!self.shares.contains(&owner)
                && !self.levels.contains_key(&unit)
                && self.truncated.contains(&"UnitIntel"))
        {
            return None;
        }
        let ladder = match self.detection(unit, owner) {
            level if level >= LAND_FULL_LEVEL => FULL_VISIBILITY,
            level if level >= LAND_SEEN_LEVEL => SHOWN_VISIBILITY,
            _ => 0,
        };
        let province = self.sight_provinces.get(&unit).copied().unwrap_or(province);
        let byte = self.visibility.get(&province).copied().unwrap_or(0);
        Some(visibility::unit_byte(
            ladder.max(u16::from(byte)),
            self.unit_modifier,
        ))
    }

    /// How much of one task force the viewer is shown.
    pub fn sees_navy(&self, unit: u64, owner: u32) -> Sight {
        sight(self.navy_byte(unit, owner))
    }

    /// Task-force intel is 255 with fog off or for our own force; otherwise apply the viewer modifier
    /// to its detection-ladder base.
    pub fn navy_byte(&self, unit: u64, owner: u32) -> u16 {
        if self.in_full(owner) {
            return u16::from(u8::MAX);
        }
        let base = match self.detection(unit, owner) {
            level if level >= NAVAL_FULL_LEVEL => 250,
            level if level >= NAVAL_SEEN_LEVEL => 150,
            1 => 100,
            _ => 0,
        };
        visibility::unit_byte(base, self.unit_modifier)
    }

    /// [`Self::navy_byte`] where the hour can establish it: `None` for a foreign task force
    /// with no `UnitIntel` entry while that table is truncated, since the omitted entry could
    /// hold any level. A unit seen in full, shared vision and a recorded entry decide it.
    pub fn navy_byte_known(&self, unit: u64, owner: u32) -> Option<u16> {
        let decided = self.in_full(owner)
            || self.shares.contains(&owner)
            || self.levels.contains_key(&unit)
            || !self.truncated.contains(&"UnitIntel");
        decided.then(|| self.navy_byte(unit, owner))
    }

    /// Port detail level from zero to four; `None` when defines or the intel census are unavailable.
    pub fn naval_port_detail(&self, owner: u32) -> Option<u8> {
        let shared = self.shares.contains(&owner);
        let raw = self
            .navy_intel
            .as_ref()
            .map(|lanes| lanes.get(&owner).copied().unwrap_or(0));
        self.naval_defines.as_ref()?.level(raw, shared, self.fog)
    }

    /// Test shared vision for canonical country slots independently of fog. A positive stands;
    /// missing links or incomplete negative relation evidence leave it unknown. The recorded engaged
    /// flag can lag diplomacy and includes viewer roster ownership, so markers and windows read that
    /// separate flag from the engaged index.
    ///
    /// `missing` names the census when `countries` may lack an entry (a
    /// table unavailable or truncated, or no row for the combat): a
    /// positive among the entries held still stands, a negative does not.
    /// Every caller states it; a list read whole passes `None`.
    pub fn sees_combat(
        &self,
        countries: impl Iterator<Item = Option<u32>>,
        missing: Option<&'static str>,
    ) -> Result<bool, SharingUnknown> {
        let (mut unplaced, mut unrelated) = (false, None);
        for country in countries {
            match country.map(|slot| self.shares_vision_proven(slot)) {
                Some(Ok(true)) => return Ok(true),
                Some(Ok(false)) => {}
                Some(Err(cause)) => unrelated = Some(cause),
                None => unplaced = true,
            }
        }
        if let Some(missing) = missing {
            Err(SharingUnknown::Census(missing))
        } else if unplaced {
            Err(SharingUnknown::Reference)
        } else {
            unrelated.map_or(Ok(false), Err)
        }
    }

    /// Walk attacking primary countries in order to orient progress. The viewer or a vision-sharing
    /// country turns it; a hostile country stops it unturned. End-of-list remains unturned. An
    /// undecidable entry prevents reaching later entries and leaves orientation unknown.
    pub fn turns_naval_progress(
        &self,
        attackers: impl Iterator<Item = Option<u32>>,
    ) -> Result<bool, SharingUnknown> {
        for country in attackers {
            let slot = country.ok_or(SharingUnknown::Reference)?;
            if self.shares_vision_proven(slot)? {
                return Ok(true);
            }
            if self.at_war_proven(slot)? {
                return Ok(false);
            }
        }
        Ok(false)
    }

    /// Active war between the viewer and `slot`. A negative result requires complete relations.
    pub fn at_war_proven(&self, slot: u32) -> Result<bool, SharingUnknown> {
        if self.wars.contains(&slot) {
            return Ok(true);
        }
        if slot == self.ours {
            return Ok(false);
        }
        self.relations_gap
            .map_or(Ok(false), |input| Err(SharingUnknown::Relations(input)))
    }

    /// Shared vision is detection level 3 and still takes the modifier.
    fn detection(&self, unit: u64, owner: u32) -> i8 {
        if self.shares.contains(&owner) {
            3
        } else {
            self.levels.get(&unit).copied().unwrap_or(0)
        }
    }

    /// Fog off, our own unit, or an undecided hour.
    fn in_full(&self, owner: u32) -> bool {
        !self.fog || owner == self.ours || self.undecided.is_some()
    }
}

fn sight(byte: u16) -> Sight {
    match byte {
        byte if byte >= FULL_VISIBILITY => Sight::Full,
        byte if byte >= SHOWN_VISIBILITY => Sight::Partial,
        _ => Sight::None,
    }
}

/// A land unit at sea whose next path province is hostile land takes intel from that province.
/// The unit itself does not move.
fn invasion_sight(
    frame: &DecodedFrame,
    links: &BTreeMap<u32, u32>,
    topology: &visibility::Topology,
    relations: &Relations,
    vision: &Vision,
) -> Result<BTreeMap<u64, u32>, &'static str> {
    let divisions = frame
        .complete::<tables::Divisions>()
        .map_err(|_| "Divisions")?;
    let moves: BTreeMap<_, _> = frame
        .complete::<tables::DivisionMovement>()
        .map_err(|_| "DivisionMovement")?
        .iter()
        .map(|m| (m.division, m))
        .collect();
    let mut out = BTreeMap::new();
    for division in divisions
        .iter()
        .filter(|d| topology.sea.contains(&d.province))
    {
        let Some(&next) = moves
            .get(&division.id)
            .ok_or("DivisionMovement division")?
            .path
            .first()
        else {
            continue;
        };
        if !topology.land.contains(&next) {
            continue;
        }
        let controller = vision
            .province_controller(next)
            .ok_or("invasion target controller")?;
        let owner = *links
            .get(&division.logical_owner)
            .ok_or("invasion division owner")?;
        if relations.at_war(controller, owner) {
            out.insert(division.id, next);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests;
