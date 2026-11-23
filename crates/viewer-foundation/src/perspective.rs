
use std::sync::Arc;
pub mod incremental;
mod members;
pub mod values;
use std::{collections::BTreeMap, fmt, io};
use values::Values;

use replay_data::{rows::RelationLinkRow, rows::TableId, rows::tables, scope, scope::CaptureMode};
use replay_frame::frame::{DecodedFrame, TableView};

use super::{
    allegiance,
    tables::{Available, core, rows},
    visibility::{self, AirDefines, FogContent, NavalDefines, Relations},
};

pub type VisionKey = (replay_data::BranchId, replay_data::Hour, Choice);

pub fn cached_vision(
    frame: &DecodedFrame,
    perspective: &Choice,
    fog: Result<&FogContent, &'static str>,
    memo: &viewer_runtime::memo::Keyed<Vision, VisionKey>,
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

pub const LAND_FULL_LEVEL: i8 = 2;
pub const LAND_SEEN_LEVEL: i8 = 1;
pub const NAVAL_FULL_LEVEL: i8 = 3;
pub const NAVAL_SEEN_LEVEL: i8 = 2;
pub const FULL_VISIBILITY: u16 = 250;
pub const SHOWN_VISIBILITY: u16 = 150;
pub const MAP_OBJECT_VISIBILITY: u8 = 50;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sight {
    Full,
    Partial,
    None,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Country {
    #[default]
    Recorded,
    Tag(String),
    Slot { slot: u32, tag: String },
}

impl Country {
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
            Country::Slot { tag, .. } if !tag.is_empty() => f.write_str(tag),
            Country::Slot { slot, .. } => write!(f, "country slot {slot}"),
        }
    }
}

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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Choice {
    pub country: Country,
    pub fog: bool,
}

impl Default for Choice {
    fn default() -> Self {
        Self {
            country: Country::Recorded,
            fog: dashboard_core::map_display::option_defaults::FOG,
        }
    }
}

impl Choice {
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SharingUnknown {
    Reference,
    Relations(&'static str),
    Census(&'static str),
}

impl SharingUnknown {
    pub fn describe(self) -> String {
        match self {
            Self::Reference => "a country reference is not in CountryLinks".into(),
            Self::Relations(input) => {
                format!("the viewer's relations are not whole this hour ({input})")
            }
            Self::Census(input) => input.into(),
        }
    }

    pub fn clause(self) -> String {
        match self {
            Self::Census(input) => input.into(),
            _ => format!("vision sharing unknown, {}", self.describe()),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Resolution {
    #[default]
    Recorded,
    Requested,
    NoPlayer,
    Absent(String),
}

pub fn viewer(
    frame: &DecodedFrame,
    links: &BTreeMap<u32, u32>,
    tags: &BTreeMap<u32, String>,
    country: &Country,
) -> io::Result<(u32, Resolution)> {
    crate::fold_tables::resolve(allegiance::perspective(frame, links)?, tags, country).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("hour {} records no country to view the map as", frame.hour),
        )
    })
}

const FOG_TABLES: [TableId; 5] = [
    TableId::UnitIntel,
    TableId::CountryVisibility,
    TableId::CountryIntel,
    TableId::StateVisibility,
    TableId::RadarCoverage,
];

pub fn fog_recorded(mode: CaptureMode) -> bool {
    FOG_TABLES.iter().all(|table| scope::records(mode, *table))
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Vision {
    ours: u32,
    resolution: Resolution,
    fog: bool,
    undecided: Option<&'static str>,
    shares: Values<u32, ()>,
    wars: Values<u32, ()>,
    relations_gap: Option<&'static str>,
    levels: Values<u64, i8>,
    unit_modifier: i64,
    visibility: Arc<Values<u32, u8>>,
    visibility_gap: Option<&'static str>,
    sight_provinces: Values<u64, u32>,
    truncated: Vec<&'static str>,
    air_intel: Option<Values<u32, i64>>,
    air_defines: Option<AirDefines>,
    navy_intel: Option<Values<u32, i64>>,
    naval_defines: Option<NavalDefines>,
    air_access: Option<Values<u32, ()>>,
    province_controllers: Values<u32, u32>,
}

#[derive(Clone, Debug, Default)]
pub struct Parts {
    pub ours: u32,
    pub resolution: Resolution,
    pub fog: bool,
    pub undecided: Option<&'static str>,
    pub shares: BTreeMap<u32, ()>,
    pub wars: BTreeMap<u32, ()>,
    pub relations_gap: Option<&'static str>,
    pub levels: BTreeMap<u64, i8>,
    pub unit_modifier: i64,
    pub visibility: BTreeMap<u32, u8>,
    pub visibility_gap: Option<&'static str>,
    pub sight_provinces: BTreeMap<u64, u32>,
    pub truncated: Vec<&'static str>,
    pub air_intel: Option<BTreeMap<u32, i64>>,
    pub air_defines: Option<AirDefines>,
    pub navy_intel: Option<BTreeMap<u32, i64>>,
    pub naval_defines: Option<NavalDefines>,
    pub air_access: Option<BTreeMap<u32, ()>>,
    pub province_controllers: BTreeMap<u32, u32>,
}

pub fn map_cutoff(cutoffs: &[i64], fraction: i64) -> i32 {
    let mut result = 0;
    for pair in cutoffs.chunks_exact(2) {
        result = visibility::rounded(pair[1]);
        if pair[0] > fraction {
            break;
        }
    }
    result
}

pub fn network_term(cutoff: i32, modifier: i64) -> i32 {
    visibility::network_term(cutoff, modifier)
}

pub fn saturate(current: u8, term: i32) -> u8 {
    visibility::saturate(current, term)
}

pub use replay_derive::visibility::{active_war, intel_fraction, map_intel_subject};

impl Vision {
    pub fn with_visibility(mut self, visibility: Arc<Values<u32, u8>>) -> Self {
        self.visibility = visibility;
        self
    }

    pub fn from_parts(parts: Parts) -> Self {
        Self {
            ours: parts.ours,
            resolution: parts.resolution,
            fog: parts.fog,
            undecided: parts.undecided,
            shares: Values::from_iter(parts.shares),
            wars: Values::from_iter(parts.wars),
            relations_gap: parts.relations_gap,
            levels: Values::from_iter(parts.levels),
            unit_modifier: parts.unit_modifier,
            visibility: Arc::new(Values::from_iter(parts.visibility)),
            visibility_gap: parts.visibility_gap,
            sight_provinces: Values::from_iter(parts.sight_provinces),
            truncated: parts.truncated,
            air_intel: parts.air_intel.map(Values::from_iter),
            air_defines: parts.air_defines,
            navy_intel: parts.navy_intel.map(Values::from_iter),
            naval_defines: parts.naval_defines,
            air_access: parts.air_access.map(Values::from_iter),
            province_controllers: Values::from_iter(parts.province_controllers),
        }
    }

    pub fn differences(&self, other: &Self) -> (u64, Vec<String>) {
        fn maps<K: values::Key + fmt::Debug, V: Clone + Eq + fmt::Debug>(name: &str, a: &Values<K, V>, b: &Values<K, V>, count: &mut u64, out: &mut Vec<String>) {
            let (a, b) = (a.flat(), b.flat());
            for (key, value) in a {
                if b.get(key) != Some(value) {
                    *count += 1;
                    if out.len() < 8 {
                        out.push(format!("{name}[{key:?}]: {value:?} against {:?}", b.get(key)));
                    }
                }
            }
            for (key, value) in b {
                if !a.contains_key(key) {
                    *count += 1;
                    if out.len() < 8 {
                        out.push(format!("{name}[{key:?}]: absent against {value:?}"));
                    }
                }
            }
        }
        fn optional<K: values::Key + fmt::Debug, V: Clone + Eq + fmt::Debug>(name: &str, a: &Option<Values<K, V>>, b: &Option<Values<K, V>>, count: &mut u64, out: &mut Vec<String>) {
            match (a, b) {
                (Some(a), Some(b)) => maps(name, a, b, count, out),
                (None, None) => {}
                _ => {
                    *count += 1;
                    out.push(format!("{name}: held {} against {}", a.is_some(), b.is_some()));
                }
            }
        }
        let (mut count, mut out) = (0, Vec::new());
        let scalars = [
            ("ours", self.ours.to_string(), other.ours.to_string()),
            ("resolution", format!("{:?}", self.resolution), format!("{:?}", other.resolution)),
            ("fog", self.fog.to_string(), other.fog.to_string()),
            ("undecided", format!("{:?}", self.undecided), format!("{:?}", other.undecided)),
            ("relations_gap", format!("{:?}", self.relations_gap), format!("{:?}", other.relations_gap)),
            ("unit_modifier", self.unit_modifier.to_string(), other.unit_modifier.to_string()),
            ("visibility_gap", format!("{:?}", self.visibility_gap), format!("{:?}", other.visibility_gap)),
            ("truncated", format!("{:?}", self.truncated), format!("{:?}", other.truncated)),
            ("air_defines", format!("{:?}", self.air_defines), format!("{:?}", other.air_defines)),
            ("naval_defines", format!("{:?}", self.naval_defines), format!("{:?}", other.naval_defines)),
        ];
        for (name, a, b) in scalars {
            if a != b {
                count += 1;
                out.push(format!("{name}: {a} against {b}"));
            }
        }
        optional("air_intel", &self.air_intel, &other.air_intel, &mut count, &mut out);
        optional("navy_intel", &self.navy_intel, &other.navy_intel, &mut count, &mut out);
        optional("air_access", &self.air_access, &other.air_access, &mut count, &mut out);
        maps("shares", &self.shares, &other.shares, &mut count, &mut out);
        maps("wars", &self.wars, &other.wars, &mut count, &mut out);
        maps("levels", &self.levels, &other.levels, &mut count, &mut out);
        maps("visibility", &self.visibility, &other.visibility, &mut count, &mut out);
        maps("sight_provinces", &self.sight_provinces, &other.sight_provinces, &mut count, &mut out);
        maps("province_controllers", &self.province_controllers, &other.province_controllers, &mut count, &mut out);
        (count, out)
    }

    pub fn entries(&self) -> u64 {
        let maps = [self.shares.flat().len(), self.wars.flat().len(), self.levels.flat().len(), self.visibility.flat().len(), self.sight_provinces.flat().len(), self.province_controllers.flat().len()];
        let optional = [self.air_intel.as_ref(), self.navy_intel.as_ref()].into_iter().flatten().map(|values| values.flat().len()).sum::<usize>() + self.air_access.as_ref().map_or(0, |values| values.flat().len());
        10 + (maps.iter().sum::<usize>() + optional) as u64
    }

    pub fn read(
        frame: &DecodedFrame,
        links: &BTreeMap<u32, u32>,
        tags: &BTreeMap<u32, String>,
        choice: &Choice,
        fog: Result<&FogContent, &'static str>,
    ) -> io::Result<Self> {
        let resolved = viewer(frame, links, tags, &choice.country)?;
        Self::read_resolved(frame, links, tags, choice.fog, resolved, fog)
    }

    pub fn read_resolved(
        frame: &DecodedFrame,
        links: &BTreeMap<u32, u32>,
        tags: &BTreeMap<u32, String>,
        fog_enabled: bool,
        (ours, resolution): (u32, Resolution),
        fog: Result<&FogContent, &'static str>,
    ) -> io::Result<Self> {
        let unrecorded = FOG_TABLES
            .iter()
            .any(|table| matches!(*frame.tables[table.index()], TableView::NotRecorded { .. }));
        let mut vision = Self {
            ours,
            resolution,
            fog: fog_enabled && !unrecorded,
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
            Available::Unavailable { .. } | Available::NotRecorded { .. } => {
                vision.undecided.get_or_insert("UnitIntel");
            }
            Available::Truncated { .. } => vision.truncated.push("UnitIntel"),
            Available::Complete(_) => {}
        }
        let mut ordered: Vec<_> = intel.rows().iter().filter(|r| r.viewer == ours).collect();
        ordered.sort_by_key(|r| (r.unit, r.ordinal));
        for row in ordered {
            if !vision.levels.contains_key(&row.unit) {
                vision.levels.insert(row.unit, row.level);
            }
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
            let (mut air, mut navy) = (BTreeMap::new(), BTreeMap::new());
            for (&raw, _) in links.iter().filter(|(_, slot)| **slot == ours) {
                for row in visibility::owner_rows(rows, raw) {
                    air.entry(row.target_index).or_insert(row.air);
                    navy.entry(row.target_index).or_insert(row.navy);
                }
            }
            vision.air_intel = Some(Values::from_iter(air));
            vision.navy_intel = Some(Values::from_iter(navy));
        }
        vision.air_defines = fog
            .ok()
            .and_then(|content| AirDefines::read(frame, &content.defines).ok());
        vision.naval_defines = fog
            .ok()
            .and_then(|content| NavalDefines::read(frame, &content.defines).ok());
        if let Ok(rows) = frame.complete::<tables::RelationLinks>() {
            vision.air_access = Some(Values::from_iter(
                rows.iter()
                    .filter(|r| {
                        r.links & RelationLinkRow::AIR_BASE_ACCESS != 0
                            && links.get(&r.country) == Some(&ours)
                    })
                    .map(|r| (r.target_index, ())),
            ));
        }
        if let Ok(provinces) = frame.complete::<tables::Provinces>() {
            vision.province_controllers = Values::from_iter(
                provinces
                    .iter()
                    .filter_map(|p| Some((p.id, *links.get(&p.controller)?))),
            );
        }

        if vision.fog {
            let rebuilt = fog.and_then(|content| {
                let relations = relations?.with_attaches(frame, links)?;
                let bytes = visibility::derive(frame, links, ours, content, &relations)?;
                let sight = invasion_sight(frame, links, &content.topology, &relations, &vision)?;
                Ok((bytes, sight))
            });
            match rebuilt {
                Ok((bytes, sight)) => {
                    vision.visibility = Arc::new(Values::from_iter(bytes));
                    vision.sight_provinces = Values::from_iter(sight);
                }
                Err(reason) => vision.visibility_gap = Some(reason),
            }
        }
        Ok(vision)
    }

    fn relate(
        &mut self,
        relations: Result<&Relations, &'static str>,
        slots: impl Iterator<Item = u32>,
    ) {
        let ours = self.ours;
        self.shares.insert(ours, ());
        match relations {
            Ok(relations) => {
                for slot in slots.filter(|&slot| relations.shared(slot, ours)) {
                    self.shares.insert(slot, ());
                }
                for slot in relations.enemies(ours).filter(|&slot| slot != ours) {
                    self.wars.insert(slot, ());
                }
            }
            Err(reason) => self.relations_gap = Some(reason),
        }
    }

    pub fn ours(&self) -> u32 {
        self.ours
    }

    pub fn resolution(&self) -> &Resolution {
        &self.resolution
    }

    pub fn fog(&self) -> bool {
        self.fog
    }

    pub fn undecided(&self) -> Option<&'static str> {
        self.undecided.filter(|_| self.fog)
    }

    pub fn visibility_gap(&self) -> Option<&'static str> {
        self.visibility_gap.filter(|_| self.fog)
    }

    pub fn truncated(&self) -> &[&'static str] {
        if self.fog { &self.truncated } else { &[] }
    }

    pub fn revealed_provinces(&self) -> usize {
        self.visibility.values().filter(|&&byte| byte > 0).count()
    }

    pub fn province_fog(&self) -> Option<&BTreeMap<u32, u8>> {
        (self.fog && self.visibility_gap.is_none()).then(|| self.visibility.flat())
    }

    pub fn shared_province_fog(&self) -> Option<&Arc<Values<u32, u8>>> {
        (self.fog && self.visibility_gap.is_none()).then_some(&self.visibility)
    }

    pub fn province_byte(&self, province: u32) -> Option<u8> {
        if self.fog {
            self.visibility.get(&province).copied()
        } else {
            Some(u8::MAX)
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn sharing(ours: u32, shares: impl IntoIterator<Item = u32>) -> Self {
        let mut vision = Self {
            ours,
            fog: true,
            shares: Values::from_iter(shares.into_iter().map(|slot| (slot, ()))),
            ..Self::default()
        };
        vision.shares.insert(ours, ());
        vision
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn without_fog(mut self) -> Self {
        self.fog = false;
        self
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn with_truncated_unit_intel(
        mut self,
        levels: impl IntoIterator<Item = (u64, i8)>,
    ) -> Self {
        self.levels = Values::from_iter(levels);
        self.truncated.push("UnitIntel");
        self
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn set_visibility(&mut self, bytes: impl IntoIterator<Item = (u32, u8)>) {
        self.visibility = Arc::new(Values::from_iter(bytes));
        self.visibility_gap = None;
    }

    pub fn sees_province_object(&self, province: u32, always_shown: bool) -> Option<bool> {
        if always_shown {
            return Some(true);
        }
        self.province_byte(province)
            .map(|byte| byte >= MAP_OBJECT_VISIBILITY)
    }

    pub fn air_port_level(&self, owner: u32) -> Option<u8> {
        let shared = self.shares.contains_key(&owner);
        if self.fog && self.undecided.is_some() && !shared {
            return None;
        }
        let raw = self
            .air_intel
            .as_ref()
            .map(|lanes| lanes.get(&owner).copied().unwrap_or(0));
        self.air_defines.as_ref()?.level(raw, shared, self.fog)
    }

    pub fn sees_air_port_detail(&self, owner: u32) -> Option<bool> {
        self.air_port_level(owner).map(|level| level > 0)
    }

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

    pub fn sees_air_base(&self, province: u32, controller: u32) -> Option<bool> {
        if !self.fog || controller == self.ours {
            return Some(true);
        }
        let access = self
            .air_access
            .as_ref()
            .map(|set| set.contains_key(&controller));
        let raw = self
            .air_intel
            .as_ref()
            .map(|lanes| lanes.get(&controller).copied().unwrap_or(0));
        let intel = self
            .air_defines
            .as_ref()
            .and_then(|d| d.show_ports(raw, self.shares.contains_key(&controller), self.fog));
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

    pub fn province_controller(&self, province: u32) -> Option<u32> {
        self.province_controllers.get(&province).copied()
    }

    pub fn shares_vision_proven(&self, slot: u32) -> Result<bool, SharingUnknown> {
        if self.shares.contains_key(&slot) {
            return Ok(true);
        }
        self.relations_gap
            .map_or(Ok(false), |input| Err(SharingUnknown::Relations(input)))
    }

    pub fn shares_vision_or_fog(&self, slot: u32) -> Result<bool, SharingUnknown> {
        if self.fog {
            self.shares_vision_proven(slot)
        } else {
            Ok(true)
        }
    }

    pub fn sees_land(&self, unit: u64, owner: u32, province: u32) -> Sight {
        if self.in_full(owner) || self.visibility_gap.is_some() || !self.detection_known(unit, owner) {
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

    pub fn land_byte_known(&self, unit: u64, owner: u32, province: u32) -> Option<u16> {
        if !self.fog || owner == self.ours {
            return Some(u16::from(u8::MAX));
        }
        if self.undecided.is_some()
            || self.visibility_gap.is_some()
            || (!self.shares.contains_key(&owner) && self.relations_gap.is_some())
            || !self.detection_known(unit, owner)
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

    pub fn sees_navy(&self, unit: u64, owner: u32) -> Sight {
        sight(self.navy_byte(unit, owner))
    }

    pub fn navy_byte(&self, unit: u64, owner: u32) -> u16 {
        if self.in_full(owner) || !self.detection_known(unit, owner) {
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

    pub fn navy_byte_known(&self, unit: u64, owner: u32) -> Option<u16> {
        (self.in_full(owner) || self.detection_known(unit, owner)).then(|| self.navy_byte(unit, owner))
    }

    pub fn naval_port_detail(&self, owner: u32) -> Option<u8> {
        let shared = self.shares.contains_key(&owner);
        let raw = self
            .navy_intel
            .as_ref()
            .map(|lanes| lanes.get(&owner).copied().unwrap_or(0));
        self.naval_defines.as_ref()?.level(raw, shared, self.fog)
    }

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

    pub fn at_war_proven(&self, slot: u32) -> Result<bool, SharingUnknown> {
        if self.wars.contains_key(&slot) {
            return Ok(true);
        }
        if slot == self.ours {
            return Ok(false);
        }
        self.relations_gap
            .map_or(Ok(false), |input| Err(SharingUnknown::Relations(input)))
    }

    fn detection_known(&self, unit: u64, owner: u32) -> bool {
        self.shares.contains_key(&owner)
            || self.levels.contains_key(&unit)
            || !self.truncated.contains(&"UnitIntel")
    }

    fn detection(&self, unit: u64, owner: u32) -> i8 {
        if self.shares.contains_key(&owner) {
            3
        } else {
            self.levels.get(&unit).copied().unwrap_or(0)
        }
    }

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
