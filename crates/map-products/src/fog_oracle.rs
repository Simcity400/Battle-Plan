use replay_data::rows::{TableId, tables};
use replay_frame::frame::{DecodedFrame, TableView};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::Arc,
};
use viewer_foundation::{
    allegiance,
    perspective::{Choice, Country, Resolution, SharingUnknown, Sight},
    tables::{Available, rows},
    visibility::{self, FogContent, Relations},
};
const LAND_FULL_LEVEL: i8 = 2;
const LAND_SEEN_LEVEL: i8 = 1;
const FULL_VISIBILITY: u16 = 250;
const SHOWN_VISIBILITY: u16 = 150;

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
        Country::Tag(tag) => match tags.iter().find(|(_, name)| name.eq_ignore_ascii_case(tag)) {
            Some((slot, _)) => (*slot, Resolution::Requested),
            None => (default()?.0, Resolution::Absent(tag.clone())),
        },
        Country::Slot { slot, .. } if tags.contains_key(slot) => (*slot, Resolution::Requested),
        Country::Slot { .. } => (default()?.0, Resolution::Absent(country.to_string())),
    })
}

const FOG_TABLES: [TableId; 5] = [
    TableId::UnitIntel,
    TableId::CountryVisibility,
    TableId::CountryIntel,
    TableId::StateVisibility,
    TableId::RadarCoverage,
];

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Vision {
    ours: u32,
    resolution: Resolution,
    fog: bool,
    undecided: Option<&'static str>,
    shares: BTreeSet<u32>,
    relations_gap: Option<&'static str>,
    levels: BTreeMap<u64, i8>,
    unit_modifier: i64,
    visibility: Arc<BTreeMap<u32, u8>>,
    visibility_gap: Option<&'static str>,
    sight_provinces: BTreeMap<u64, u32>,
    truncated: Vec<&'static str>,
    province_controllers: BTreeMap<u32, u32>,
}

impl Vision {
    pub fn read(
        frame: &DecodedFrame,
        links: &BTreeMap<u32, u32>,
        tags: &BTreeMap<u32, String>,
        choice: &Choice,
        fog: Result<&FogContent, &'static str>,
    ) -> io::Result<Self> {
        let (ours, resolution) = viewer(frame, links, tags, &choice.country)?;
        let unrecorded = FOG_TABLES
            .iter()
            .any(|table| matches!(*frame.tables[table.index()], TableView::NotRecorded { .. }));
        let mut vision = Self {
            ours,
            resolution,
            fog: choice.fog && !unrecorded,
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
        if let Ok(provinces) = frame.complete::<tables::Provinces>() {
            vision.province_controllers = provinces
                .iter()
                .filter_map(|p| Some((p.id, *links.get(&p.controller)?)))
                .collect();
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
                    vision.visibility = Arc::new(bytes);
                    vision.sight_provinces = sight;
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
        self.shares.insert(ours);
        match relations {
            Ok(relations) => {
                self.shares
                    .extend(slots.filter(|&slot| relations.shared(slot, ours)));
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

    pub fn province_fog(&self) -> Option<&BTreeMap<u32, u8>> {
        (self.fog && self.visibility_gap.is_none()).then_some(self.visibility.as_ref())
    }

    pub fn province_byte(&self, province: u32) -> Option<u8> {
        if self.fog {
            self.visibility.get(&province).copied()
        } else {
            Some(u8::MAX)
        }
    }

    pub fn shares_vision_proven(&self, slot: u32) -> Result<bool, SharingUnknown> {
        if self.shares.contains(&slot) {
            return Ok(true);
        }
        self.relations_gap
            .map_or(Ok(false), |input| Err(SharingUnknown::Relations(input)))
    }

    pub fn sees_land(&self, unit: u64, owner: u32, province: u32) -> Sight {
        if self.in_full(owner)
            || self.visibility_gap.is_some()
            || (!self.shares.contains(&owner)
                && !self.levels.contains_key(&unit)
                && self.truncated.contains(&"UnitIntel"))
        {
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

    fn detection(&self, unit: u64, owner: u32) -> i8 {
        if self.shares.contains(&owner) {
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
            .province_controllers
            .get(&next)
            .copied()
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

pub fn intel(
    map: &content_reader_map::map::MapData,
    visibility: Option<&BTreeMap<u32, u8>>,
) -> map_display_data::Intel {
    let mut texels = Vec::new();
    for z in 0..map.height / 6 {
        for x in 0..map.width / 6 {
            let mut sum = 0u32;
            for dz in 0..6 {
                for dx in 0..6 {
                    let row = map.height - 1 - (z * 6 + dz);
                    let province = map.province_ids[(row * map.width + x * 6 + dx) as usize];
                    let land = map.definitions.get(&province).is_some_and(|definition| {
                        definition.kind == content_reader_map::map::ProvinceKind::Land
                    });
                    sum += u32::from(if land {
                        visibility
                            .and_then(|values| values.get(&province))
                            .copied()
                            .unwrap_or(255)
                    } else {
                        255
                    });
                }
            }
            texels.push(!((sum / 36) as u8));
        }
    }
    map_display_data::Intel::new(texels, visibility.is_some())
}
