use engine_native_parts::cpolitical_party_layout as political_party_layout;
use engine_native_parts::ccountry_leader_layout as country_leader_layout;
use engine_native_parts::cpolitical_status_layout as political_status_layout;
use engine_native_memory::country_identity::{MANPOWER};
use engine_native_memory::country_identity::{AuthenticatedRoster, Country, ReservedCountryObservation};

use engine_native_memory::{
    Memory,
    cpdx_array::{Array, copy_u32s},
    msvc_string::{STRIDE, Text, read_text_prefetched},
    read,
    walk::{Buffer, Outcome},
};
use crate::{carmy, ccharacter, cgame_state, cunit};
use engine_native_parts::{cconvoys, cradars_pool};
use engine_sites::Rva;

mod ahead;
pub use ahead::Ahead;
pub mod launch_sources;
mod native_reference;
pub mod rosters;
pub use native_reference::{NativeReferenceContext, NativeReferenceMemo};
pub use rosters::{
    CarriedRosters, CarriedSpans, CounterRosters, EffectRosterDemand, EffectRosters, NEIGHBORS,
    POLICY_STATUS_NULL, RosterDemand, RosterSpan, SENT_EXPEDITIONARIES,
    counter_effect_roster_demand, counter_roster_demand, counter_roster_demand_with_pride_observer,
};

pub const VTABLE_RVA: Rva = Rva(0x0286_4730);
pub const ID: usize = 8;
pub const NAME: usize = 0x10;
pub const ADJECTIVE: usize = 0x30;
pub const DEFINITE_NAME: usize = 0x50;
pub const OLD_NAME: usize = 0x90;
pub const OLD_ADJECTIVE: usize = 0xb0;
pub const OLD_DEFINITE_NAME: usize = 0xd0;
pub const COMMAND_POWER: usize = 0x1f0;
pub const ALLOCATED_COMMAND_POWER: usize = 0x1f8;
pub const POLITICAL_STATUS: usize = 0xf90;
pub const STABILITY: usize = 0x10d0;
pub const WAR_SUPPORT: usize = 0x10d8;

pub const SURRENDER_TARGET: usize = 0xc;
pub const THEATRES: usize = 0x168;
pub const AIR_THEATRES: usize = 0x180;
pub const FLAG_MANAGER: usize = 0x230;
pub const FLEETS: usize = 0x278;
pub const DIVISIONS: usize = 0x290;
pub const WAR_OVERVIEW_CORE_STATES: usize = 0x4a8;
pub const WAR_OVERVIEW_OFFSITE: usize = 0x1350;
pub const CURRENT_PRIMARY_COLOR: usize = 0x370;
pub const CURRENT_UI_COLOR: usize = 0x390;
pub const CONTROLLED_STATES: usize = 0x460;
pub const DYNAMIC_MODIFIERS: usize = 0xe58;
pub const TECHNOLOGY_STATUS: usize = 0xf60;
pub const PRODUCTION_STATUS: usize = 0xf68;
pub const DEPLOYMENT_MANAGER: usize = 0xf70;
pub const CAN_OCCUPY_NON_WAR: usize = 0x6b8;
pub const CIVIL_WAR_LINK: usize = 0x1308;
pub const CIVIL_WAR_TARGETS: usize = 0x12f0;
pub const DIPLOMACY_STATUS: usize = 0xf88;
pub const DECISION_STATUS: usize = 0xfa0;
pub const PROGRAM_STATUS: usize = 0xfa8;
pub const INTELLIGENCE_AGENCY: usize = 0xfc0;
pub const OCCUPATION_STATUS: usize = 0xfd0;
pub const INTEL_TABLE: usize = 0xfe8;
pub const SCIENTIST_ROSTER: usize = 0xff0;
pub const CHARACTERS: usize = SCIENTIST_ROSTER;
pub const INCOMING_ACTION_QUEUE: usize = 0xff8;
pub const CAPITAL: usize = 0x1018;
pub const ORIGINAL_CAPITAL: usize = 0x101c;
pub const CONVOY_STATE: usize = 0x1200;
pub const DELAYED_EVENTS: usize = 0x1290;
pub const ACES: usize = 0x12d8;
pub const PRIDE_OF_THE_FLEET: usize = 0x250;
pub const OWNED_STATES: usize = 0x478;
pub const CORE_STATES: usize = 0x4a8;
#[cfg(test)]
pub const OWNED_STATE_CAPACITY: usize = 0x480;
pub const OWNED_STATE_COUNT: usize = 0x484;
pub const ORIGINAL_TAG: usize = 0x130c;
#[cfg(test)]
pub const SHIP_COUNT: usize = 0x1330;
pub const FOCUS_TREE: usize = 0x1370;
pub const FOCUS_PROGRESS: usize = 0x1380;
pub const PEACE_CONFERENCE_PARTICIPANT: usize = 0x1458;
pub const DYNAMIC_MAJOR: usize = 0x145a;
pub const COSMETIC_TAG: usize = 0x1488;
pub const FUEL_STATUS: usize = 0x1580;
pub const EXPERIENCE_STATUS: usize = 0x1588;
pub const OPERATION_MANAGER: usize = 0x15a8;
pub const ACTIVE_COMMAND_ABILITIES: usize = 0x15b8;

pub const LOCK_ALL_TEMPLATES: usize = 0x1b4;
pub const DIVISION_TEMPLATES: usize = 0x1b8;
pub const VARIABLES: usize = 0x218;
pub const LOGISTICS_STATUS: usize = 0xf98;
pub const EQUIPMENT_MARKET_STATUS: usize = 0xfb8;
pub const COLLABORATION_STATUS: usize = 0xfd8;
pub const RADARS_POOL: usize = 0x10f8;
pub const RADAR_GROUPS: usize = RADARS_POOL + cradars_pool::GROUPS;
pub const RADARS_POOL_COUNTRY: usize = RADARS_POOL + cradars_pool::COUNTRY;
pub const RESOURCES: usize = 0x11f8;
pub const NUKES: usize = 0x1310;
pub const VOLUNTEER_HOSTS: usize = 0x1428;
#[cfg(any(test, feature = "test-support"))]
pub const VOLUNTEER_TALLY_COUNT: usize = 0x144c;
pub const OPERATION_TOKEN_MANAGER: usize = 0x15b0;
pub const DELAYED_EVENTS_COUNT: usize = 0x129c;

pub const GRAPHICAL_CULTURE: usize = 0x1028;
pub const RESOLVE_GRAPHICAL_CULTURE_RVA: Rva = Rva(0x0075_a3e0);
pub const GRAPHICAL_CULTURE_2D: usize = 0x1050;
#[cfg(any(test, feature = "test-support"))]
pub const THEATRES_COUNT: usize = 0x174;
#[cfg(any(test, feature = "test-support"))]
pub const FLEETS_CAPACITY: usize = 0x280;
#[cfg(any(test, feature = "test-support"))]
pub const FLEETS_COUNT: usize = 0x284;
#[cfg(any(test, feature = "test-support"))]
pub const DIVISIONS_CAPACITY: usize = 0x298;
pub const DIVISIONS_COUNT: usize = 0x29c;
pub const RAILWAY_UNITS: usize = 0x2a8;
pub const RAILWAY_UNITS_CAPACITY: usize = 0x2b0;
pub const RAILWAY_UNITS_COUNT: usize = 0x2b4;
#[cfg(test)]
pub const CONTROLLED_STATES_COUNT: usize = 0x46c;
#[cfg(any(test, feature = "test-support"))]
pub const RADAR_GROUPS_COUNT: usize = RADARS_POOL + cradars_pool::GROUPS_COUNT;
pub const VOLUNTEER_HOSTS_COUNT: usize = 0x1434;
pub const REBUILD_LOCALIZED_NAME_RVA: Rva = Rva(0x0075_e6d0);
pub const ACTIVE_ABILITIES: usize = 0x15b8;
pub const FOG_NETWORK_TABLE_COUNT: usize = 0x484;
pub const CONTROLLED_PROVINCES: usize = 0x428;
pub const CONTROLLED_PROVINCES_COUNT: usize = CONTROLLED_PROVINCES + 0xc;
pub const OWNED_PROVINCES: usize = 0x410;
pub const VOLUNTEER_TALLIES: usize = 0x1440;

pub const ADD_STABILITY_RVA: Rva = Rva(0x0071_9f40);
pub const ADD_WAR_SUPPORT_RVA: Rva = Rva(0x0071_b0b0);
pub const DAILY_UPDATE_RVA: Rva = Rva(0x0072_f220);
pub const WEEKLY_UPDATE_RVA: Rva = Rva(0x0076_0e30);

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub politics_witness: Option<(u32, Option<u32>)>,
    pub id: u32,
    pub canonical_slot: u32,
    pub tag: Text<8>,
    pub political_power_raw: Option<i64>,
    pub command_power_raw: i64,
    pub stability_raw: i64,
    pub war_support_raw: i64,
    pub convoys_raw: Option<i64>,
    pub peace_conference_participant: u8,
    pub manpower_residual: i32,
    pub border_land_doctrine_level: Option<i32>,
}

const GATHERED: [(usize, usize); 8] = [
    (POLITICAL_STATUS, 8),
    (MANPOWER + engine_native_parts::ccountry_manpower_layout::RESIDUAL, 4),
    (COMMAND_POWER, 8),
    (STABILITY, 8),
    (WAR_SUPPORT, 8),
    (PEACE_CONFERENCE_PARTICIPANT, 1),
    (CONVOY_STATE + cconvoys::CACHE_VALID, 1),
    (SURRENDER_TARGET, 4),
];
const GATHERED_STATUS: usize = 0;
const GATHERED_MANPOWER: usize = 1;
const GATHERED_COMMAND_POWER: usize = 2;
const GATHERED_STABILITY: usize = 3;
const GATHERED_WAR_SUPPORT: usize = 4;
const GATHERED_PEACE: usize = 5;
const GATHERED_CONVOY_VALID: usize = 6;
const GATHERED_SURRENDER_TARGET: usize = 7;

struct Fields {
    values: [[u8; 8]; GATHERED.len()],
    copied: usize,
}

impl Fields {
    fn gather(memory: &impl Memory, pointer: usize) -> Self {
        let mut values = [[0_u8; 8]; GATHERED.len()];
        let addresses = GATHERED.map(|(offset, _)| pointer.checked_add(offset).unwrap_or(0));
        let [a, b, c, d, e, f, g, h] = values.each_mut();
        let copied = memory.copy_scattered_prefetched(
            addresses,
            &mut [
                &mut a[..GATHERED[0].1],
                &mut b[..GATHERED[1].1],
                &mut c[..GATHERED[2].1],
                &mut d[..GATHERED[3].1],
                &mut e[..GATHERED[4].1],
                &mut f[..GATHERED[5].1],
                &mut g[..GATHERED[6].1],
                &mut h[..GATHERED[7].1],
            ],
        );
        Self { values, copied }
    }

    fn get<T: engine_native_memory::memory::Raw>(
        &self,
        memory: &impl Memory,
        pointer: usize,
        index: usize,
    ) -> Result<Option<T>, &'static str> {
        let (offset, len) = GATHERED[index];
        debug_assert_eq!(len, size_of::<T>());
        let address = pointer
            .checked_add(offset)
            .ok_or("country address overflow")?;
        Ok(match index.cmp(&self.copied) {
            std::cmp::Ordering::Less => {
                engine_native_memory::memory::scalar_from_bytes::<T>(&self.values[index][..len])
            }
            std::cmp::Ordering::Equal => None,
            std::cmp::Ordering::Greater => read::<T>(memory, address),
        })
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct LinkRow {
    pub id: u32,
    pub canonical_slot: u32,
}

pub fn copy_links_and_roster(memory: &impl Memory, game_state: usize, out: &mut Vec<u8>) -> bool {
    out.clear();
    let [Some(links), Some(roster)] =
        [(cgame_state::COUNTRY_LINKS, 4), (cgame_state::COUNTRIES, 8)].map(|(offset, stride)| {
            game_state
                .checked_add(offset)
                .and_then(|address| Array::read(memory, address, stride))
                .map(|array| (array, stride))
        })
    else {
        return false;
    };
    let len = [links, roster]
        .iter()
        .map(|(array, stride)| 12 + array.count as usize * stride)
        .sum();
    if engine_allocation::advisory(|| engine_allocation::reserve(out, len)).is_err() {
        return false;
    }
    for (array, stride) in [links, roster] {
        out.extend_from_slice(&(array.data as u64).to_le_bytes());
        out.extend_from_slice(&array.count.to_le_bytes());
        let at = out.len();
        out.resize(at + array.count as usize * stride, 0);
        if at != out.len() && !memory.copy(array.data, &mut out[at..]) {
            return false;
        }
    }
    true
}

const LEADER_IDEOLOGY: usize = 0x140;
const IDEOLOGY_TOKEN: usize = 0x10;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct AppearanceRow<T = Text<256>, G = Text<8>, I = Text<128>> {
    pub country: u32,
    pub subjects: Array,
    pub rgba: [u32; 4],
    pub ui_rgba: [u32; 4],
    pub cosmetic_tag: T,
    pub name_tag: G,
    pub leader_ideology: I,
    pub leader_character: Option<u32>,
    pub graphical_culture: T,
    pub graphical_culture_2d: T,
    pub overlord: i32,
    pub old_names: Option<[T; 3]>,
    pub original_tag_reference: i32,
    original_tag_is_name_tag: bool,
    pub original_tag: Option<crate::crelation::subject::Name>,
    pub original_tag_failure: Option<&'static str>,
}

#[derive(Clone, Copy, Debug)]
pub struct NamingContext {
    pub tags: Array,
    pub dynamic_boundary: u32,
}

pub fn dynamic_boundary(memory: &impl Memory, base: usize) -> Option<u32> {
    let database = read::<u64>(
        memory,
        engine_sites::at(base, engine_native_parts::ccountry_database::SINGLETON_RVA)?,
    )?;
    let database = usize::try_from(database).ok().filter(|&p| p != 0)?;
    read(
        memory,
        database.checked_add(engine_native_parts::ccountry_database::DYNAMIC_TAG_BOUNDARY)?,
    )
}

#[derive(Default)]
struct RulingLeader<I> {
    ideology: I,
    character: Option<u32>,
}

fn ruling_party(
    memory: &impl Memory,
    base: usize,
    country: usize,
) -> Result<Option<usize>, &'static str> {
    let pointer = |address: usize| -> Result<usize, &'static str> {
        read::<u64>(memory, address)
            .and_then(|value| usize::try_from(value).ok())
            .ok_or("country naming pointer unreadable")
    };
    let object = |address: usize, rva: Rva| -> Result<(), &'static str> {
        (read::<u64>(memory, address) == engine_sites::at(base, rva).map(|v| v as u64))
            .then_some(())
            .ok_or("country naming object identity mismatch")
    };
    let at = |object: usize, offset: usize| {
        object
            .checked_add(offset)
            .ok_or("country naming address overflow")
    };
    let politics = pointer(at(country, POLITICAL_STATUS)?)?;
    if politics == 0 {
        return Ok(None);
    }
    object(politics, political_status_layout::VTABLE_RVA)?;
    let party = pointer(at(politics, political_status_layout::RULING_PARTY)?)?;
    if party == 0 {
        return Ok(None);
    }
    object(party, political_party_layout::VTABLE_RVA)?;
    Ok(Some(party))
}

fn first_party_leader(
    memory: &impl Memory,
    base: usize,
    party: usize,
) -> Result<Option<usize>, &'static str> {
    let at = |offset| {
        party
            .checked_add(offset)
            .ok_or("country naming address overflow")
    };
    let count = read::<u32>(memory, at(political_party_layout::COUNTRY_LEADER_COUNT)?)
        .ok_or("country leader count unreadable")?;
    if count == 0 {
        return Ok(None);
    }
    let pointer = |address| {
        read::<u64>(memory, address)
            .and_then(|value| usize::try_from(value).ok())
            .ok_or("country naming pointer unreadable")
    };
    let leader = pointer(pointer(at(political_party_layout::COUNTRY_LEADERS)?)?)?;
    if read::<u64>(memory, leader)
        != engine_sites::at(base, country_leader_layout::VTABLE_RVA).map(|v| v as u64)
    {
        return Err("country naming object identity mismatch");
    }
    Ok(Some(leader))
}

fn ruling_leader<I: engine_native_memory::capture_text::Value>(
    memory: &impl Memory,
    base: usize,
    country: usize,
) -> Result<RulingLeader<I>, &'static str> {
    let Some(party) = ruling_party(memory, base, country)? else {
        return Ok(RulingLeader::default());
    };
    let Some(leader) = first_party_leader(memory, base, party)? else {
        return Ok(RulingLeader::default());
    };
    let at = |object: usize, offset: usize| {
        object
            .checked_add(offset)
            .ok_or("country naming address overflow")
    };
    let pointer = |address| {
        read::<u64>(memory, address)
            .and_then(|value| usize::try_from(value).ok())
            .ok_or("country naming pointer unreadable")
    };
    let ideology = pointer(at(leader, LEADER_IDEOLOGY)?)?;
    if ideology == 0 {
        return Err("country leader has no ideology");
    }
    let ideology = I::read(memory, at(ideology, IDEOLOGY_TOKEN)?, false).map_err(|_| {
        if I::GROWING {
            "country leader ideology unreadable or invalid"
        } else {
            "country leader ideology unreadable or too long"
        }
    })?;
    let character = at(leader, country_leader_layout::CHARACTER)
        .ok()
        .and_then(|address| pointer(address).ok())
        .filter(|&character| character != 0)
        .and_then(|character| ccharacter::identity_header(memory, base, character).ok());
    Ok(RulingLeader {
        ideology,
        character,
    })
}

fn name_tag_for_original(
    country: Country,
    naming: NamingContext,
    original: i32,
) -> Result<Option<usize>, &'static str> {
    if original > 0 && (naming.dynamic_boundary as i32) <= country.canonical_slot as i32 {
        Ok(Some(
            naming
                .tags
                .element(original as u32, STRIDE)
                .ok_or("original tag outside the tag array")?,
        ))
    } else {
        Ok(None)
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ReservedNamingRow {
    pub tag: crate::crelation::subject::Name,
    pub name_tag: crate::crelation::subject::Name,
    pub cosmetic_tag: crate::crelation::subject::Name,
    pub leader_ideology: crate::crelation::subject::Name,
    pub group: Option<crate::crelation::subject::Name>,
    pub overlord: i32,
    pub autonomy: Option<crate::crelation::subject::Name>,
    pub built: u8,
    pub original_tag: Option<crate::crelation::subject::Name>,
    pub original_tag_failure: Option<&'static str>,
    pub original_tag_reference: i32,
    pub dynamic_tag_boundary: i32,
    pub owned_state_count: Option<i32>,
    pub owned_state_count_failure: Option<&'static str>,
    pub faction: Option<u64>,
    pub faction_failure: Option<&'static str>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ReservedNamingWalk {
    pub row: ReservedNamingRow,
    pub outcome: Outcome,
    pub bytes_len: usize,
    pub required_bytes: usize,
    pub launch_states: Option<launch_sources::ReservedOwnedStates>,
    pub visibility: Option<ReservedVisibilityObservation>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReservedVisibilityObservation {
    root: Result<Country, &'static str>,
    diplomacy_pointer: Option<Result<usize, &'static str>>,
    diplomacy_identity: Option<Result<(), &'static str>>,
    diplomacy_backlink: Option<Result<(), &'static str>>,
    faction: Option<Result<u64, &'static str>>,
}

impl ReservedVisibilityObservation {
    fn from_root(memory: &impl Memory, base: usize, root: Result<Country, &'static str>) -> Self {
        let mut observed = Self {
            root,
            diplomacy_pointer: None,
            diplomacy_identity: None,
            diplomacy_backlink: None,
            faction: None,
        };
        let Ok(country) = root else { return observed };
        let pointer = country
            .pointer
            .checked_add(DIPLOMACY_STATUS)
            .ok_or("reserved naming address overflow")
            .and_then(|at| {
                read::<u64>(memory, at)
                    .and_then(|value| usize::try_from(value).ok())
                    .ok_or("reserved naming pointer unreadable")
            });
        observed.diplomacy_pointer = Some(pointer);
        let Ok(diplomacy) = pointer else {
            return observed;
        };
        if diplomacy == 0 {
            observed.faction = Some(Err("reserved faction diplomacy is null"));
            return observed;
        }
        let identity = engine_sites::at(base, engine_native_parts::cdiplomacy_status_layout::VTABLE_RVA)
            .ok_or("reserved naming module address overflow")
            .and_then(|expected| {
                (read::<u64>(memory, diplomacy) == Some(expected as u64))
                    .then_some(())
                    .ok_or("reserved naming object identity mismatch")
            });
        observed.diplomacy_identity = Some(identity);
        if identity.is_err() {
            return observed;
        }
        let backlink = diplomacy
            .checked_add(engine_native_parts::cdiplomacy_status_layout::COUNTRY)
            .and_then(|at| read::<u64>(memory, at))
            .ok_or("reserved faction diplomacy backlink unreadable")
            .and_then(|backlink| {
                (backlink == country.pointer as u64)
                    .then_some(())
                    .ok_or("reserved faction diplomacy country mismatch")
            });
        observed.diplomacy_backlink = Some(backlink);
        if backlink.is_err() {
            return observed;
        }
        observed.faction = Some((|| {
            let pointer = diplomacy
                .checked_add(engine_native_parts::cdiplomacy_status_layout::FACTION)
                .and_then(|at| read::<u64>(memory, at))
                .ok_or("reserved faction pointer unreadable")?;
            if pointer == 0 {
                return Ok(0);
            }
            crate::cfaction::identity(
                memory,
                base,
                usize::try_from(pointer).map_err(|_| "reserved faction pointer overflow")?,
            )
        })());
        observed
    }

    pub fn root(&self) -> Result<Country, &'static str> {
        self.root
    }

    fn naming_diplomacy(&self) -> Result<usize, &'static str> {
        self.root?;
        let pointer = self
            .diplomacy_pointer
            .ok_or("reserved visibility diplomacy unattempted")??;
        if pointer != 0 {
            self.diplomacy_identity
                .ok_or("reserved visibility identity unattempted")??;
        }
        Ok(pointer)
    }

    pub fn diplomacy(&self) -> Result<usize, &'static str> {
        let pointer = self.naming_diplomacy()?;
        if pointer == 0 {
            return Err("reserved faction diplomacy is null");
        }
        self.diplomacy_backlink
            .ok_or("reserved visibility backlink unattempted")??;
        Ok(pointer)
    }

    pub fn faction(&self) -> Result<u64, &'static str> {
        self.diplomacy()?;
        self.faction
            .ok_or("reserved visibility faction unattempted")?
    }
}

struct ReservedNamingSources {
    texts: [Option<engine_native_memory::msvc_string::Descriptor>; 6],
    original: i32,
    dynamic_tag_boundary: i32,
    overlord: i32,
    built: u8,
}

impl ReservedNamingSources {
    fn original_tag_source(
        &self,
        memory: &impl Memory,
        game_state: usize,
    ) -> Result<Option<engine_native_memory::msvc_string::Descriptor>, &'static str> {
        if self.original <= 0 || self.texts[1].is_some() {
            return Ok(None);
        }
        original_tag_descriptor(
            memory,
            original_tag_strings(memory, game_state)?,
            self.original as u32,
        )
        .map(Some)
    }

    fn original_tag_source_observed(
        &self,
        memory: &impl Memory,
        game_state: usize,
        journal: &mut engine_native_memory::raw_country_observations::Journal,
    ) -> Result<Option<engine_native_memory::msvc_string::Descriptor>, &'static str> {
        if self.original <= 0 || self.texts[1].is_some() {
            return Ok(None);
        }
        let tags = original_tag_strings(memory, game_state);
        journal.tag_control(tags);
        let selected = tags.and_then(|tags| {
            tags.element(self.original as u32, STRIDE)
                .map(|_| tags)
                .ok_or("country original tag outside live tag strings")
        });
        journal.tag_selection(
            self.original,
            engine_native_memory::raw_country_observations::Source::ReservedOriginal,
            selected,
        );
        let address = selected?
            .element(self.original as u32, STRIDE)
            .ok_or("country original tag outside live tag strings")?;
        engine_native_memory::msvc_string::Descriptor::read_observed(
            memory,
            address,
            self.original,
            engine_native_memory::raw_country_observations::Source::ReservedOriginal,
            journal,
        )
        .map(Some)
    }

    fn demand(&self) -> Result<usize, &'static str> {
        self.texts
            .iter()
            .flatten()
            .try_fold(0_usize, |total, text| {
                total
                    .checked_add(text.len)
                    .ok_or("reserved naming byte demand overflow")
            })
    }
}

fn original_tag_strings(memory: &impl Memory, game_state: usize) -> Result<Array, &'static str> {
    game_state
        .checked_add(cgame_state::COUNTRY_TAGS)
        .and_then(|address| Array::read(memory, address, STRIDE))
        .ok_or("country original tag strings unreadable or invalid")
}

fn original_tag_descriptor(
    memory: &impl Memory,
    tags: Array,
    reference: u32,
) -> Result<engine_native_memory::msvc_string::Descriptor, &'static str> {
    let address = tags
        .element(reference, STRIDE)
        .ok_or("country original tag outside live tag strings")?;
    engine_native_memory::msvc_string::Descriptor::read(memory, address)
}

pub fn read_reserved_root(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
) -> Result<Country, &'static str> {
    read_reserved_observation(memory, base, game_state).result()
}

pub fn cached_reserved_observation(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
) -> Option<ReservedCountryObservation> {
    memory
        .authenticated_reserved_country_observation(base, game_state)
        .or_else(|| {
            memory
                .authenticated_reserved_country(base, game_state)
                .map(ReservedCountryObservation::from_result)
        })
}

pub fn read_reserved_observation(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
) -> ReservedCountryObservation {
    if let Some(observation) = cached_reserved_observation(memory, base, game_state) {
        return observation;
    }
    let countries = game_state
        .checked_add(cgame_state::COUNTRIES)
        .and_then(|address| Array::read(memory, address, 8))
        .ok_or("reserved country table unreadable or invalid");
    match countries {
        Ok(countries) => observe_reserved_from_array(memory, base, countries),
        Err(reason) => ReservedCountryObservation::from_result(Err(reason)),
    }
}

pub fn reserved_from_array(
    memory: &impl Memory,
    base: usize,
    countries: Array,
) -> Result<Country, &'static str> {
    observe_reserved_from_array(memory, base, countries).result()
}

pub fn observe_reserved_from_array(
    memory: &impl Memory,
    base: usize,
    countries: Array,
) -> ReservedCountryObservation {
    let pointer = countries
        .element(0, 8)
        .and_then(|address| read::<u64>(memory, address))
        .and_then(|pointer| usize::try_from(pointer).ok())
        .ok_or("reserved country slot absent or unreadable");
    let result = (|| {
        let pointer = pointer?;
        if pointer == 0 {
            return Err("reserved country slot is null");
        }
        let expected =
            engine_sites::at(base, VTABLE_RVA).ok_or("reserved country module address overflow")?;
        if read::<u64>(memory, pointer) != Some(expected as u64) {
            return Err("reserved country object identity mismatch");
        }
        if pointer
            .checked_add(ID)
            .and_then(|address| read::<u32>(memory, address))
            != Some(0)
        {
            return Err("reserved country id is not zero");
        }
        Ok(Country {
            pointer,
            canonical_slot: 0,
            id: 0,
        })
    })();
    ReservedCountryObservation::new(pointer.ok(), result)
}

fn reserved_naming_sources(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
) -> Result<ReservedNamingSources, &'static str> {
    reserved_naming_sources_from_root(
        memory,
        base,
        game_state,
        read_reserved_root(memory, base, game_state)?,
        None,
        None,
    )
}

fn reserved_naming_sources_from_root(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    root: Country,
    visibility: Option<&ReservedVisibilityObservation>,
    mut journal: Option<&mut engine_native_memory::raw_country_observations::Journal>,
) -> Result<ReservedNamingSources, &'static str> {
    use engine_native_memory::msvc_string::Descriptor;
    let at = |object: usize, offset: usize| {
        object
            .checked_add(offset)
            .ok_or("reserved naming address overflow")
    };
    let pointer = |address| {
        read::<u64>(memory, address)
            .and_then(|value| usize::try_from(value).ok())
            .ok_or("reserved naming pointer unreadable")
    };
    let object = |address: usize, rva: Rva| {
        let expected =
            engine_sites::at(base, rva).ok_or("reserved naming module address overflow")?;
        (read::<u64>(memory, address) == Some(expected as u64))
            .then_some(())
            .ok_or("reserved naming object identity mismatch")
    };
    let country = root.pointer;
    let database = engine_sites::at(base, engine_native_parts::ccountry_database::SINGLETON_RVA)
        .ok_or("reserved naming address overflow")
        .and_then(pointer);
    if let Some(journal) = journal.as_deref_mut() {
        journal.reserved_database(database);
    }
    let tags = database
        .and_then(|database| {
            Array::read(
                memory,
                at(database, engine_native_parts::ccountry_database::TAGS)?,
                STRIDE,
            )
            .ok_or("reserved country database tags unreadable or invalid")
        })
        .and_then(|tags| {
            tags.element(0, STRIDE)
                .map(|_| tags)
                .ok_or("reserved country database tag absent")
        });
    if let Some(journal) = journal.as_deref_mut() {
        journal.tag_selection(
            0,
            engine_native_memory::raw_country_observations::Source::ReservedTag,
            tags,
        );
    }
    let database = database?;
    let tag_address = tags?
        .element(0, STRIDE)
        .ok_or("reserved country database tag absent")?;
    let tag = match journal.as_deref_mut() {
        Some(journal) => Descriptor::read_observed(
            memory,
            tag_address,
            0,
            engine_native_memory::raw_country_observations::Source::ReservedTag,
            journal,
        ),
        None => Descriptor::read(memory, tag_address),
    }?;
    let dynamic_boundary = read::<u32>(
        memory,
        at(database, engine_native_parts::ccountry_database::DYNAMIC_TAG_BOUNDARY)?,
    )
    .ok_or("reserved naming dynamic boundary unreadable")?;
    let naming = NamingContext {
        tags: if (dynamic_boundary as i32) <= 0 {
            Array::read(memory, at(game_state, cgame_state::COUNTRY_TAGS)?, STRIDE)
                .ok_or("reserved original tags unreadable or invalid")?
        } else {
            Array::default()
        },
        dynamic_boundary,
    };
    let original = at(country, ORIGINAL_TAG)
        .and_then(|address| read::<i32>(memory, address).ok_or("original tag unreadable"));
    if let Some(journal) = journal.as_deref_mut() {
        journal.original_reference(0, original);
    }
    let original = original?;
    if original > 0
        && naming.dynamic_boundary == 0
        && let Some(journal) = journal.as_deref_mut()
    {
        let selected = naming
            .tags
            .element(original as u32, STRIDE)
            .map(|_| naming.tags)
            .ok_or("original tag outside the tag array");
        journal.tag_selection(
            original,
            engine_native_memory::raw_country_observations::Source::ReservedOriginal,
            selected,
        );
    }
    let name_tag = name_tag_for_original(
        Country {
            pointer: country,
            canonical_slot: 0,
            id: 0,
        },
        naming,
        original,
    )?
    .map(|address| match journal {
        Some(journal) => Descriptor::read_observed(
            memory,
            address,
            original,
            engine_native_memory::raw_country_observations::Source::ReservedOriginal,
            journal,
        ),
        None => Descriptor::read(memory, address),
    })
    .transpose()?;
    let cosmetic = Descriptor::read(memory, at(country, COSMETIC_TAG)?)?;
    let party = ruling_party(memory, base, country)?;
    let (ideology, group) = if let Some(party) = party {
        let group = pointer(at(party, political_party_layout::IDEOLOGY_GROUP)?)?;
        object(group, engine_native_parts::cideology_group::VTABLE_RVA)?;
        let group = Descriptor::read(memory, at(group, engine_native_parts::cideology_group::TOKEN)?)?;
        let ideology = first_party_leader(memory, base, party)?
            .map(|leader| {
                let ideology = pointer(at(leader, LEADER_IDEOLOGY)?)?;
                if ideology == 0 {
                    return Err("country leader has no ideology");
                }
                Descriptor::read(memory, at(ideology, IDEOLOGY_TOKEN)?)
            })
            .transpose()?;
        (ideology, Some(group))
    } else {
        (None, None)
    };
    let diplomacy = match visibility {
        Some(observed) => observed.naming_diplomacy()?,
        None => pointer(at(country, DIPLOMACY_STATUS)?)?,
    };
    let (overlord, autonomy) = if diplomacy == 0 {
        (0, None)
    } else {
        if visibility.is_none() {
            object(diplomacy, engine_native_parts::cdiplomacy_status_layout::VTABLE_RVA)?;
        }
        let overlord = read::<i32>(memory, at(diplomacy, engine_native_parts::cdiplomacy_status_layout::OVERLORD)?)
            .ok_or("reserved naming overlord unreadable")?;
        let status = pointer(at(
            diplomacy,
            engine_native_parts::cdiplomacy_status_layout::CURRENT_AUTONOMY_STATUS,
        )?)?;
        let autonomy = if status == 0 {
            None
        } else {
            object(status, engine_native_parts::ccurrent_autonomy_status_layout::VTABLE_RVA)?;
            if pointer(at(
                status,
                engine_native_parts::ccurrent_autonomy_status_layout::SUBJECT_COUNTRY,
            )?)? != country
            {
                return Err("reserved autonomy subject country mismatch");
            }
            let definition = pointer(at(status, engine_native_parts::ccurrent_autonomy_status_layout::STATE)?)?;
            if definition == 0 {
                return Err("reserved autonomy definition is null");
            }
            Some(Descriptor::read(
                memory,
                at(definition, engine_native_parts::autonomy_state_definition::NAME)?,
            )?)
        };
        (overlord, autonomy)
    };
    let mut built = 0;
    for (index, offset) in [NAME, DEFINITE_NAME, ADJECTIVE].into_iter().enumerate() {
        let len = read::<u64>(memory, at(country, offset + engine_native_memory::msvc_string::LENGTH)?)
            .ok_or("reserved naming stored length unreadable")?;
        built |= u8::from(len != 0) << index;
    }
    Ok(ReservedNamingSources {
        texts: [
            Some(tag),
            name_tag,
            Some(cosmetic),
            ideology,
            group,
            autonomy,
        ],
        original,
        dynamic_tag_boundary: dynamic_boundary as i32,
        overlord,
        built,
    })
}

pub fn read_reserved_naming_demand(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
) -> Result<usize, &'static str> {
    let sources = reserved_naming_sources(memory, base, game_state)?;
    let prefix = sources.demand()?;
    Ok(sources
        .original_tag_source(memory, game_state)
        .ok()
        .flatten()
        .and_then(|source| prefix.checked_add(source.len))
        .unwrap_or(prefix))
}

pub fn read_country_naming_demand(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
) -> Result<usize, &'static str> {
    let mut required = 0_usize;
    let mut known = false;
    let mut first_failure = None;
    match read_reserved_naming_demand(memory, base, game_state) {
        Ok(bytes) => {
            required = bytes;
            known = true;
        }
        Err(reason) => first_failure = Some(reason),
    }
    let positive = (|| {
        let reader = Reader::new(memory, base, game_state)?;
        let tags = original_tag_strings(memory, game_state)?;
        for slot in 1..reader.count() {
            let Some(country) = reader.at(slot)? else {
                continue;
            };
            let Some(original) = country
                .pointer
                .checked_add(ORIGINAL_TAG)
                .and_then(|address| read::<i32>(memory, address))
            else {
                continue;
            };
            if original <= 0 {
                known = true;
                continue;
            }
            match original_tag_descriptor(memory, tags, original as u32) {
                Ok(source) => {
                    required = required
                        .checked_add(source.len)
                        .ok_or("country original tag byte demand overflow")?;
                    known = true;
                }
                Err(reason) => {
                    first_failure.get_or_insert(reason);
                }
            }
        }
        Ok::<_, &'static str>(())
    })();
    if let Err(reason) = positive {
        first_failure.get_or_insert(reason);
    }
    if known {
        Ok(required)
    } else {
        Err(first_failure.unwrap_or("country naming byte demand unavailable"))
    }
}

pub fn read_reserved_naming_observed(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    names: &mut Vec<u8>,
    journal: &mut engine_native_memory::raw_country_observations::Journal,
) -> ReservedNamingWalk {
    read_reserved_naming_observed_inner(
        memory,
        base,
        game_state,
        engine_native_memory::walk::Buffer::Growing(names),
        Some(journal),
    )
}

pub fn read_reserved_naming_observed_inner<M: Memory>(
    memory: &M,
    base: usize,
    game_state: usize,
    mut names: engine_native_memory::walk::Buffer<'_, u8>,
    mut journal: Option<&mut engine_native_memory::raw_country_observations::Journal>,
) -> ReservedNamingWalk {
    let launch_states = launch_sources::ReservedOwnedStates::new(memory, base, game_state);
    let visibility = ReservedVisibilityObservation::from_root(memory, base, launch_states.root());
    let mut required_bytes = 0;
    let result = (|| {
        let sources = reserved_naming_sources_from_root(
            memory,
            base,
            game_state,
            launch_states.root()?,
            Some(&visibility),
            journal.as_deref_mut(),
        )?;
        required_bytes = sources.demand()?;
        names.ensure(required_bytes)?;
        if required_bytes > names.len() {
            if let Ok(Some(source)) = match journal.as_deref_mut() {
                Some(journal) => sources.original_tag_source_observed(memory, game_state, journal),
                None => sources.original_tag_source(memory, game_state),
            } {
                required_bytes = required_bytes
                    .checked_add(source.len)
                    .unwrap_or(required_bytes);
            }
            return Ok((
                ReservedNamingRow::default(),
                0,
                Outcome::Truncated {
                    len: 0,
                    unread_rows: 1,
                },
            ));
        }
        let mut used = 0;
        let mut spans = [None; 6];
        for (index, source) in sources.texts.iter().enumerate() {
            if let Some(source) = source {
                let end = used + source.len;
                match journal.as_deref_mut() {
                    Some(journal) => source.copy_observed(memory, &mut names[used..end], journal),
                    None => source.copy(memory, &mut names[used..end]),
                }?;
                spans[index] = Some(crate::crelation::subject::Name {
                    start: used,
                    len: source.len,
                });
                used = end;
            }
        }
        let original_tag = (|| {
            let Some(source) = match journal.as_deref_mut() {
                Some(journal) => sources.original_tag_source_observed(memory, game_state, journal),
                None => sources.original_tag_source(memory, game_state),
            }?
            else {
                let span = if sources.original > 0 {
                    spans[1].expect("selected prefix tag was acquired")
                } else {
                    crate::crelation::subject::Name {
                        start: used,
                        len: 0,
                    }
                };
                return Ok(span);
            };
            required_bytes = required_bytes
                .checked_add(source.len)
                .ok_or("reserved original tag byte demand overflow")?;
            let end = used
                .checked_add(source.len)
                .ok_or("reserved original tag arena overflow")?;
            names.ensure(end)?;
            let output = names
                .get_mut(used..end)
                .ok_or("reserved original tag bytes do not fit arena")?;
            match journal.as_deref_mut() {
                Some(journal) => source.copy_observed(memory, output, journal),
                None => source.copy(memory, output),
            }?;
            let span = crate::crelation::subject::Name {
                start: used,
                len: source.len,
            };
            used = end;
            Ok::<_, &'static str>(span)
        })();
        let owned_state_count = launch_states.count();
        let faction = visibility.faction();
        Ok((
            ReservedNamingRow {
                tag: spans[0].unwrap_or_default(),
                name_tag: spans[1].unwrap_or_default(),
                cosmetic_tag: spans[2].unwrap_or_default(),
                leader_ideology: spans[3].unwrap_or_default(),
                group: spans[4],
                overlord: sources.overlord,
                autonomy: spans[5],
                built: sources.built,
                original_tag: original_tag.ok(),
                original_tag_failure: original_tag.err(),
                original_tag_reference: sources.original,
                dynamic_tag_boundary: sources.dynamic_tag_boundary,
                owned_state_count: owned_state_count.ok(),
                owned_state_count_failure: owned_state_count.err(),
                faction: faction.ok(),
                faction_failure: faction.err(),
            },
            used,
            Outcome::Rows { len: 1 },
        ))
    })();
    let (row, bytes_len, outcome) = match result {
        Ok(value) => value,
        Err(reason) => (
            ReservedNamingRow::default(),
            0,
            Outcome::Unavailable { reason },
        ),
    };
    ReservedNamingWalk {
        row,
        outcome,
        bytes_len,
        required_bytes,
        launch_states: Some(launch_states),
        visibility: Some(visibility),
    }
}

#[cfg(test)]
pub fn read_appearance(
    memory: &impl Memory,
    base: usize,
    country: Country,
    naming: NamingContext,
) -> Result<AppearanceRow, &'static str> {
    read_appearance_observed_inner(memory, base, country, naming, None)
}
fn read_appearance_observed_inner<
    T: engine_native_memory::capture_text::Value,
    G: engine_native_memory::capture_text::Value,
    I: engine_native_memory::capture_text::Value,
>(
    memory: &impl Memory,
    base: usize,
    country: Country,
    naming: NamingContext,
    mut journal: Option<&mut engine_native_memory::raw_country_observations::Journal>,
) -> Result<AppearanceRow<T, G, I>, &'static str> {
    let at = |offset| {
        country
            .pointer
            .checked_add(offset)
            .ok_or("country appearance address overflow")
    };
    let expected =
        engine_sites::at(base, VTABLE_RVA).ok_or("country appearance module address overflow")?;
    if country.pointer == 0
        || country.id == 0
        || read::<u64>(memory, country.pointer) != Some(expected as u64)
        || read::<u32>(memory, at(ID)?) != Some(country.id)
    {
        return Err("country appearance identity changed");
    }
    const _: () = assert!(CURRENT_UI_COLOR == CURRENT_PRIMARY_COLOR + 0x20);
    let bytes = read::<[u8; 48]>(memory, at(CURRENT_PRIMARY_COLOR)?)
        .ok_or("country appearance colour unreadable")?;
    let word = |offset: usize| u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
    let rgba = std::array::from_fn(|index| word(index * 4));
    let ui_rgba = std::array::from_fn(|index| word(0x20 + index * 4));
    let cosmetic_tag = T::read(memory, at(COSMETIC_TAG)?, false).map_err(|_| {
        if T::GROWING {
            "country cosmetic tag unreadable or invalid"
        } else {
            "country cosmetic tag unreadable, invalid or exceeds 256 bytes"
        }
    })?;
    let original = at(ORIGINAL_TAG)
        .and_then(|address| read::<i32>(memory, address).ok_or("original tag unreadable"));
    if let Some(journal) = journal.as_deref_mut() {
        journal.original_reference(country.id, original);
    }
    let original = original?;
    if original > 0
        && country.canonical_slot >= naming.dynamic_boundary
        && let Some(journal) = journal.as_deref_mut()
    {
        let selected = naming
            .tags
            .element(original as u32, STRIDE)
            .map(|_| naming.tags)
            .ok_or("original tag outside the tag array");
        journal.tag_selection(
            original,
            engine_native_memory::raw_country_observations::Source::AppearanceOriginal,
            selected,
        );
    }
    let name_tag_address = name_tag_for_original(country, naming, original)?;
    let name_tag = if let Some(address) = name_tag_address {
        match journal {
            Some(journal) => G::observed(
                memory,
                address,
                original,
                engine_native_memory::raw_country_observations::Source::AppearanceOriginal,
                journal,
            ),
            None => G::read(memory, address, false),
        }
        .map_err(|_| {
            if G::GROWING {
                "original tag unreadable or invalid"
            } else {
                "original tag unreadable or too long"
            }
        })?
    } else {
        G::default()
    };
    let diplomacy = read::<u64>(memory, at(DIPLOMACY_STATUS)?)
        .and_then(|value| usize::try_from(value).ok())
        .ok_or("country diplomacy pointer unreadable")?;
    if read::<u64>(memory, diplomacy)
        != engine_sites::at(base, engine_native_parts::cdiplomacy_status_layout::VTABLE_RVA).map(|v| v as u64)
    {
        return Err("country diplomacy vtable mismatch");
    }
    const SUBJECTS: usize = engine_native_parts::cdiplomacy_status_layout::SUBJECTS;
    const OVERLORD: usize = engine_native_parts::cdiplomacy_status_layout::OVERLORD;
    const _: () = assert!(OVERLORD == SUBJECTS + 0x18);
    let diplomacy = diplomacy
        .checked_add(engine_native_parts::cdiplomacy_status_layout::SUBJECTS)
        .and_then(|address| read::<[u8; 0x1c]>(memory, address))
        .ok_or("country subjects and overlord unreadable")?;
    let overlord = i32::from_le_bytes(diplomacy[0x18..].try_into().unwrap());
    let subjects = Array::from_header(diplomacy[..16].try_into().unwrap(), 4)
        .ok_or("country subject roster unreadable or invalid")?;
    let leader = ruling_leader(memory, base, country.pointer)?;
    Ok(AppearanceRow {
        country: country.id,
        subjects,
        rgba,
        ui_rgba,
        cosmetic_tag,
        name_tag,
        leader_ideology: leader.ideology,
        leader_character: leader.character,
        graphical_culture: T::read(memory, at(GRAPHICAL_CULTURE)?, false)
            .map_err(|_| "country graphical culture unreadable or too long")?,
        graphical_culture_2d: T::read(memory, at(GRAPHICAL_CULTURE_2D)?, false)
            .map_err(|_| "country 2D graphical culture unreadable or too long")?,
        overlord,
        old_names: (|| {
            let old = |offset| T::read(memory, at(offset).ok()?, false).ok();
            Some([old(OLD_NAME)?, old(OLD_ADJECTIVE)?, old(OLD_DEFINITE_NAME)?])
        })(),
        original_tag_reference: original,
        original_tag_is_name_tag: name_tag_address.is_some(),
        original_tag: None,
        original_tag_failure: None,
    })
}

pub fn capture_appearance_original_tags(
    memory: &impl Memory,
    game_state: usize,
    rows: &mut [AppearanceRow],
    names: &mut Vec<u8>,
    used: &mut usize,
    required: &mut usize,
) {
    capture_appearance_original_tags_observed_inner(
        memory,
        game_state,
        rows,
        engine_native_memory::walk::Buffer::Growing(names),
        used,
        required,
        None,
    )
}
pub fn capture_appearance_original_tags_observed(
    memory: &impl Memory,
    game_state: usize,
    rows: &mut [OwnedAppearanceRow],
    names: &mut Vec<u8>,
    used: &mut usize,
    required: &mut usize,
    journal: &mut engine_native_memory::raw_country_observations::Journal,
) {
    capture_appearance_original_tags_observed_inner(
        memory,
        game_state,
        rows,
        engine_native_memory::walk::Buffer::Growing(names),
        used,
        required,
        Some(journal),
    )
}
fn capture_appearance_original_tags_observed_inner<
    T: engine_native_memory::capture_text::Value,
    G: engine_native_memory::capture_text::Value,
    I: engine_native_memory::capture_text::Value,
>(
    memory: &impl Memory,
    game_state: usize,
    rows: &mut [AppearanceRow<T, G, I>],
    mut names: engine_native_memory::walk::Buffer<'_, u8>,
    used: &mut usize,
    required: &mut usize,
    mut journal: Option<&mut engine_native_memory::raw_country_observations::Journal>,
) {
    let tags = rows
        .iter()
        .any(|row| row.original_tag_reference > 0 && !row.original_tag_is_name_tag)
        .then(|| original_tag_strings(memory, game_state));
    if let Some(control) = tags
        && let Some(journal) = journal.as_deref_mut()
    {
        journal.tag_control(control);
    }
    for row in rows {
        let result = (|| {
            if row.original_tag_reference <= 0 {
                return Ok(crate::crelation::subject::Name {
                    start: *used,
                    len: 0,
                });
            }
            let source = if row.original_tag_is_name_tag {
                None
            } else {
                let selected = tags.expect("live tag source selected").and_then(|tags| {
                    tags.element(row.original_tag_reference as u32, STRIDE)
                        .map(|_| tags)
                        .ok_or("country original tag outside live tag strings")
                });
                if let Some(journal) = journal.as_deref_mut() {
                    journal.tag_selection(
                        row.original_tag_reference,
                        engine_native_memory::raw_country_observations::Source::AppearanceOriginal,
                        selected,
                    );
                }
                let tags = selected?;
                Some(match journal.as_deref_mut() {
                    Some(journal) => engine_native_memory::msvc_string::Descriptor::read_observed(
                        memory,
                        tags.element(row.original_tag_reference as u32, STRIDE)
                            .ok_or("country original tag outside live tag strings")?,
                        row.original_tag_reference,
                        engine_native_memory::raw_country_observations::Source::AppearanceOriginal,
                        journal,
                    ),
                    None => {
                        original_tag_descriptor(memory, tags, row.original_tag_reference as u32)
                    }
                }?)
            };
            let len = source
                .as_ref()
                .map_or(row.name_tag.as_bytes().len(), |source| source.len);
            *required = required
                .checked_add(len)
                .ok_or("country original tag byte demand overflow")?;
            let end = used
                .checked_add(len)
                .ok_or("country original tag arena overflow")?;
            names.ensure(end)?;
            let destination = names
                .get_mut(*used..end)
                .ok_or("country original tag bytes do not fit arena")?;
            if let Some(source) = source {
                match journal.as_deref_mut() {
                    Some(journal) => source.copy_observed(memory, destination, journal),
                    None => source.copy(memory, destination),
                }?;
            } else {
                destination.copy_from_slice(row.name_tag.as_bytes());
            }
            let span = crate::crelation::subject::Name { start: *used, len };
            *used = end;
            Ok::<_, &'static str>(span)
        })();
        row.original_tag = result.ok();
        row.original_tag_failure = result.err();
    }
}

pub fn walk_appearance_observed(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    dynamic_boundary: u32,
    output: &mut Vec<OwnedAppearanceRow>,
    journal: &mut engine_native_memory::raw_country_observations::Journal,
) -> Outcome {
    walk_appearance_observed_inner(
        memory,
        base,
        game_state,
        dynamic_boundary,
        engine_native_memory::walk::Buffer::Growing(output),
        Some(journal),
    )
}
#[derive(Clone, Copy, Debug)]
pub struct AppearanceGlobals {
    pub naming: NamingContext,
    pub links: Array,
}

pub type OwnedAppearanceRow = AppearanceRow<
    engine_native_memory::capture_text::Owned,
    engine_native_memory::capture_text::Owned,
    engine_native_memory::capture_text::Owned,
>;

impl OwnedAppearanceRow {
    pub fn copy_held(&mut self, held: &Self) -> Result<(), &'static str> {
        fn text(
            into: &mut engine_native_memory::capture_text::Owned,
            from: &engine_native_memory::capture_text::Owned,
        ) -> Result<(), &'static str> {
            into.prepare(from.len)
                .map_err(|_| "held appearance text allocation failed")?;
            into.copy_prepared(from.as_bytes())
        }
        self.country = held.country;
        self.subjects = held.subjects;
        self.rgba = held.rgba;
        self.ui_rgba = held.ui_rgba;
        text(&mut self.cosmetic_tag, &held.cosmetic_tag)?;
        text(&mut self.name_tag, &held.name_tag)?;
        text(&mut self.leader_ideology, &held.leader_ideology)?;
        self.leader_character = held.leader_character;
        text(&mut self.graphical_culture, &held.graphical_culture)?;
        text(&mut self.graphical_culture_2d, &held.graphical_culture_2d)?;
        self.overlord = held.overlord;
        match &held.old_names {
            Some(names) => {
                let into = self.old_names.get_or_insert_with(Default::default);
                for (into, from) in into.iter_mut().zip(names) {
                    text(into, from)?;
                }
            }
            None => self.old_names = None,
        }
        self.original_tag_reference = held.original_tag_reference;
        self.original_tag_is_name_tag = held.original_tag_is_name_tag;
        self.original_tag = held.original_tag;
        self.original_tag_failure = held.original_tag_failure;
        Ok(())
    }
}

#[allow(clippy::type_complexity)]
pub fn walk_appearance_carrying(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    dynamic_boundary: u32,
    output: &mut Vec<OwnedAppearanceRow>,
    journal: &mut engine_native_memory::raw_country_observations::Journal,
    carry: &mut dyn FnMut(AppearanceGlobals, Country, &mut OwnedAppearanceRow) -> bool,
) -> Outcome {
    walk_appearance_carrying_inner(
        memory,
        base,
        game_state,
        dynamic_boundary,
        engine_native_memory::walk::Buffer::Growing(output),
        Some(journal),
        carry,
    )
}

fn walk_appearance_observed_inner<
    T: engine_native_memory::capture_text::Value,
    G: engine_native_memory::capture_text::Value,
    I: engine_native_memory::capture_text::Value,
>(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    dynamic_boundary: u32,
    output: engine_native_memory::walk::Buffer<'_, AppearanceRow<T, G, I>>,
    journal: Option<&mut engine_native_memory::raw_country_observations::Journal>,
) -> Outcome {
    walk_appearance_carrying_inner(
        memory,
        base,
        game_state,
        dynamic_boundary,
        output,
        journal,
        &mut |_, _, _| false,
    )
}
#[allow(clippy::type_complexity)]
fn walk_appearance_carrying_inner<
    T: engine_native_memory::capture_text::Value,
    G: engine_native_memory::capture_text::Value,
    I: engine_native_memory::capture_text::Value,
>(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    dynamic_boundary: u32,
    mut output: engine_native_memory::walk::Buffer<'_, AppearanceRow<T, G, I>>,
    mut journal: Option<&mut engine_native_memory::raw_country_observations::Journal>,
    carry: &mut dyn FnMut(AppearanceGlobals, Country, &mut AppearanceRow<T, G, I>) -> bool,
) -> Outcome {
    let result = (|| {
        let reader = Reader::new(memory, base, game_state)?;
        output.ensure(reader.count().saturating_sub(1) as usize)?;
        let naming = NamingContext {
            tags: game_state
                .checked_add(cgame_state::COUNTRY_TAGS)
                .and_then(|p| Array::read(memory, p, STRIDE))
                .ok_or("country link tags unreadable or invalid")?,
            dynamic_boundary,
        };
        let globals = AppearanceGlobals {
            naming,
            links: reader.arrays()[1],
        };
        let mut len = 0;
        let mut unread_rows = 0_u32;
        let mut found_country = false;
        for slot in 1..reader.count() {
            let Some(country) = reader.at(slot)? else {
                continue;
            };
            found_country = true;
            let fits = if let Some(destination) = output.get_mut(len) {
                if !carry(globals, country, &mut *destination) {
                    *destination = read_appearance_observed_inner(
                        memory,
                        base,
                        country,
                        naming,
                        journal.as_deref_mut(),
                    )?;
                }
                len += 1;
                true
            } else {
                false
            };
            if !fits {
                unread_rows = unread_rows
                    .checked_add(1)
                    .ok_or("country appearance unread row count overflow")?;
            }
        }
        if !found_country {
            return Err("country table has no readable countries");
        }
        Ok(if unread_rows == 0 {
            Outcome::Rows { len }
        } else {
            Outcome::Truncated { len, unread_rows }
        })
    })();
    result.unwrap_or_else(|reason| Outcome::Unavailable { reason })
}

#[derive(Debug, Default)]
pub struct RosterSnapshot {
    slots: Vec<Option<Country>>,
    captured: Option<(usize, usize, [Array; 2])>,
    required: usize,
    reserved: Option<(usize, usize, ReservedCountryObservation)>,
}

impl RosterSnapshot {
    pub fn try_with_capacity(slots: usize) -> std::io::Result<Self> {
        Ok(Self {
            slots: engine_allocation::vec(slots)?,
            ..Self::default()
        })
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn with_capacity(slots: usize) -> Self {
        Self::try_with_capacity(slots).unwrap()
    }

    pub fn clear(&mut self) {
        self.slots.clear();
        self.captured = None;
        self.reserved = None;
    }

    pub fn capture(
        &mut self,
        roster: AuthenticatedRoster<'_>,
        base: usize,
        game_state: usize,
    ) -> bool {
        self.clear();
        self.required = roster.slots().len();
        if self.slots.capacity() < roster.slots().len() {
            return false;
        }
        self.slots.extend_from_slice(roster.slots());
        self.captured = Some((base, game_state, roster.arrays()));
        true
    }

    pub fn roster(&self, base: usize, game_state: usize) -> Option<AuthenticatedRoster<'_>> {
        self.captured
            .filter(|(module, state, _)| (*module, *state) == (base, game_state))
            .map(|(_, _, arrays)| AuthenticatedRoster::new(&self.slots, arrays))
    }

    #[cfg(test)]
    pub fn capture_reserved(
        &mut self,
        base: usize,
        game_state: usize,
        root: Result<Country, &'static str>,
    ) {
        self.capture_reserved_observation(
            base,
            game_state,
            ReservedCountryObservation::from_result(root),
        );
    }

    pub fn capture_reserved_observation(
        &mut self,
        base: usize,
        game_state: usize,
        observation: ReservedCountryObservation,
    ) {
        self.reserved = Some((base, game_state, observation));
        if let Some(slot) = self.slots.first_mut() {
            *slot = observation.result().ok();
        }
    }

    pub fn reserved(
        &self,
        base: usize,
        game_state: usize,
    ) -> Option<Result<Country, &'static str>> {
        self.reserved_observation(base, game_state)
            .map(ReservedCountryObservation::result)
    }

    pub fn reserved_observation(
        &self,
        base: usize,
        game_state: usize,
    ) -> Option<ReservedCountryObservation> {
        self.reserved
            .filter(|(module, state, _)| (*module, *state) == (base, game_state))
            .map(|(_, _, root)| root)
    }

    pub fn required(&self) -> usize {
        self.required
    }

    pub fn capacity(&self) -> usize {
        self.slots.capacity()
    }

    pub fn reserve(&mut self, slots: usize) -> std::io::Result<()> {
        let additional = slots.saturating_sub(self.slots.len());
        engine_allocation::reserve_exact(&mut self.slots, additional)
    }

    pub fn allocated_bytes(&self) -> usize {
        self.slots.capacity() * std::mem::size_of::<Option<Country>>()
    }
}

pub struct Reader<'a, M: Memory> {
    memory: &'a M,
    countries: Array,
    links: Array,
    vtable: usize,
    authenticated: Option<AuthenticatedRoster<'a>>,
    base: usize,
    game_state: usize,
    reserved: std::cell::Cell<Option<ReservedCountryObservation>>,
}

impl<'a, M: Memory> Reader<'a, M> {
    pub fn new(memory: &'a M, base: usize, game_state: usize) -> Result<Self, &'static str> {
        let authenticated = memory.authenticated_countries(base, game_state);
        let [countries, links] = if let Some(roster) = authenticated {
            roster.arrays()
        } else {
            let countries_addr = game_state
                .checked_add(cgame_state::COUNTRIES)
                .ok_or("country array unreadable or invalid")?;
            let links_addr = game_state
                .checked_add(cgame_state::COUNTRY_LINKS)
                .ok_or("country links unreadable or invalid")?;
            let mut countries_raw = [0u8; 16];
            let mut links_raw = [0u8; 16];
            let copied = memory.copy_scattered(
                [countries_addr, links_addr],
                &mut [&mut countries_raw[..], &mut links_raw[..]],
            );
            let countries = (copied >= 1)
                .then(|| Array::from_header(countries_raw, 8))
                .flatten()
                .ok_or("country array unreadable or invalid")?;
            let links = (copied >= 2)
                .then(|| Array::from_header(links_raw, 4))
                .flatten()
                .ok_or("country links unreadable or invalid")?;
            [countries, links]
        };
        Self::from_arrays(memory, base, game_state, [countries, links])
    }

    pub fn from_arrays(
        memory: &'a M,
        base: usize,
        game_state: usize,
        [countries, links]: [Array; 2],
    ) -> Result<Self, &'static str> {
        if countries.count < 2 {
            return Err("country table has no country slots");
        }
        if links.count == 0 {
            return Err("country links have no sentinel slot");
        }
        let vtable = engine_sites::at(base, VTABLE_RVA).ok_or("module address overflow")?;
        Ok(Self {
            memory,
            countries,
            links,
            vtable,
            authenticated: memory
                .authenticated_countries(base, game_state)
                .filter(|roster| roster.slots().len() == countries.count as usize),
            base,
            game_state,
            reserved: std::cell::Cell::new(cached_reserved_observation(memory, base, game_state)),
        })
    }

    pub fn reserved(&self) -> Result<Country, &'static str> {
        self.reserved_observation().result()
    }

    pub fn reserved_observation(&self) -> ReservedCountryObservation {
        if let Some(root) = self.reserved.get() {
            return root;
        }
        let root = cached_reserved_observation(self.memory, self.base, self.game_state)
            .unwrap_or_else(|| observe_reserved_from_array(self.memory, self.base, self.countries));
        self.reserved.set(Some(root));
        root
    }

    pub fn arrays(&self) -> [Array; 2] {
        [self.countries, self.links]
    }

    pub fn count(&self) -> u32 {
        self.countries.count
    }

    pub fn known(&self, slot: u32) -> Option<Country> {
        self.authenticated?
            .slots()
            .get(slot as usize)
            .copied()
            .flatten()
            .filter(|country| country.canonical_slot == slot)
    }

    pub fn at(&self, slot: u32) -> Result<Option<Country>, &'static str> {
        Ok(self
            .read_physical_at(slot)?
            .filter(|country| country.canonical_slot == slot))
    }

    pub fn resolve(&self, raw: u32) -> Result<Country, &'static str> {
        let canonical_slot = self.canonical(raw)?;
        let country = self
            .read_at(canonical_slot)?
            .ok_or("country link resolves to null country")?;
        if country.canonical_slot != canonical_slot {
            return Err("country link does not resolve to its canonical object");
        }
        Ok(country)
    }

    pub fn verify_canonical_pointer(&self, pointer: usize, id: u32) -> Option<Country> {
        if id == 0 {
            return (self.pointer_at(0).ok()? == pointer).then_some(Country {
                pointer,
                canonical_slot: 0,
                id: 0,
            });
        }
        let canonical_slot = self.canonical(id).ok()?;
        (self.pointer_at(canonical_slot).ok()? == pointer).then_some(Country {
            pointer,
            canonical_slot,
            id,
        })
    }

    pub fn of_pointer(&self, pointer: usize) -> Result<Country, &'static str> {
        if pointer == 0 {
            return Err("country pointer is null");
        }
        let id = pointer
            .checked_add(ID)
            .and_then(|p| read::<u32>(self.memory, p))
            .ok_or("country id unreadable")?;
        let country = self.resolve(id)?;
        if country.pointer != pointer {
            return Err("country pointer differs from its canonical object");
        }
        Ok(country)
    }

    fn pointer_at(&self, slot: u32) -> Result<usize, &'static str> {
        self.countries
            .element(slot, 8)
            .and_then(|p| read::<u64>(self.memory, p))
            .and_then(|p| usize::try_from(p).ok())
            .ok_or("country pointer unreadable or outside country table")
    }

    pub fn link(&self, raw: u32) -> Result<Option<u32>, &'static str> {
        if raw == 0 {
            return Err("country id is zero");
        }
        let address = self
            .links
            .element(raw, 4)
            .ok_or("country link unreadable or outside country table")?;
        let known = self
            .authenticated
            .and_then(|roster| roster.slots().get(raw as usize))
            .and_then(|country| *country)
            .map(|country| country.canonical_slot);
        let slot = match known {
            Some(slot) => slot,
            None => read::<u32>(self.memory, address)
                .ok_or("country link unreadable or outside country table")?,
        };
        if slot == 0 {
            return Ok(None);
        }
        if slot >= self.countries.count {
            return Err("country link unreadable or outside country table");
        }
        Ok(Some(slot))
    }

    fn canonical(&self, raw: u32) -> Result<u32, &'static str> {
        self.link(raw)?
            .ok_or("country link unreadable or outside country table")
    }

    pub fn read_at(&self, slot: u32) -> Result<Option<Country>, &'static str> {
        self.read_slot(slot, false)
    }

    pub fn read_physical_at(&self, slot: u32) -> Result<Option<Country>, &'static str> {
        self.read_slot(slot, true)
    }

    fn read_slot(&self, slot: u32, physical: bool) -> Result<Option<Country>, &'static str> {
        if slot == 0 {
            return Err("country id is zero");
        }
        if let Some(country) = self
            .authenticated
            .and_then(|roster| roster.slots().get(slot as usize))
        {
            return Ok(*country);
        }
        let pointer = self.pointer_at(slot)?;
        if pointer == 0 {
            return Ok(None);
        }
        if physical
            && let Some(alias) = self
                .reserved
                .get()
                .and_then(|root| root.physical_alias(pointer))
        {
            alias?;
            return Ok(None);
        }
        let id = identity(self.memory, pointer, self.vtable)?;
        if physical
            && id == 0
            && let Some(alias) = self.reserved_observation().physical_alias(pointer)
        {
            alias?;
            return Ok(None);
        }
        let canonical_slot = self.canonical(id)?;
        if self.canonical(slot)? != canonical_slot {
            return Err("country slot and object resolve to different countries");
        }
        if self.canonical(canonical_slot)? != canonical_slot {
            return Err("country link does not resolve to its canonical object");
        }
        if self.pointer_at(canonical_slot)? != pointer {
            return Err("canonical country slot points to a different object");
        }
        Ok(Some(Country {
            pointer,
            canonical_slot,
            id,
        }))
    }
}

pub fn identity(memory: &impl Memory, pointer: usize, vtable: usize) -> Result<u32, &'static str> {
    let mut actual = [0; 8];
    let mut id = [0; 4];
    let copied = memory.copy_scattered(
        [pointer, pointer.checked_add(ID).unwrap_or(0)],
        &mut [&mut actual, &mut id],
    );
    if copied == 0 || u64::from_le_bytes(actual) != vtable as u64 {
        return Err("country vtable mismatch");
    }
    if copied != 2 {
        return Err("country id unreadable");
    }
    Ok(u32::from_le_bytes(id))
}

fn movement_field<T: engine_native_memory::memory::Raw>(
    memory: &impl Memory,
    object: usize,
    offset: usize,
) -> Result<T, &'static str> {
    object
        .checked_add(offset)
        .and_then(|p| read(memory, p))
        .ok_or("movement country dependency unreadable or overflowed")
}

fn movement_pointer(
    memory: &impl Memory,
    object: usize,
    offset: usize,
) -> Result<usize, &'static str> {
    usize::try_from(movement_field::<u64>(memory, object, offset)?)
        .map_err(|_| "movement country pointer overflow")
}

fn movement_auth(
    memory: &impl Memory,
    base: usize,
    object: usize,
    rva: Rva,
) -> Result<(), &'static str> {
    if object == 0 || engine_sites::at(base, rva).map(|p| p as u64) != read::<u64>(memory, object) {
        return Err("movement country dependency identity mismatch");
    }
    Ok(())
}

fn movement_array(
    memory: &impl Memory,
    object: usize,
    offset: usize,
) -> Result<Array, &'static str> {
    object
        .checked_add(offset)
        .and_then(|p| Array::read(memory, p, 8))
        .ok_or("movement country dependency array invalid")
}

fn movement_element(memory: &impl Memory, array: Array, index: u32) -> Result<usize, &'static str> {
    let at = array
        .element(index, 8)
        .ok_or("movement country index outside array")?;
    movement_pointer(memory, at, 0)
}

fn movement_equal(
    countries: &Reader<'_, impl Memory>,
    a: u32,
    b: u32,
) -> Result<bool, &'static str> {
    Ok(a == b || (a != 0 && b != 0 && countries.canonical(a)? == countries.canonical(b)?))
}

fn movement_diplomacy(
    memory: &impl Memory,
    base: usize,
    country: usize,
) -> Result<usize, &'static str> {
    let diplomacy = movement_pointer(memory, country, DIPLOMACY_STATUS)?;
    movement_auth(
        memory,
        base,
        diplomacy,
        engine_native_parts::cdiplomacy_status_layout::VTABLE_RVA,
    )?;
    if movement_pointer(memory, diplomacy, engine_native_parts::cdiplomacy_status_layout::COUNTRY)? != country {
        return Err("movement diplomacy owner mismatch");
    }
    Ok(diplomacy)
}

fn movement_tuple(
    memory: &impl Memory,
    base: usize,
    countries: &Reader<'_, impl Memory>,
    diplomacy: usize,
    target: u32,
) -> Result<usize, &'static str> {
    let slot = if target == 0 {
        0
    } else {
        countries.canonical(target)?
    };
    let tuple = movement_element(
        memory,
        movement_array(memory, diplomacy, engine_native_parts::cdiplomacy_status_layout::RELATIONS)?,
        slot,
    )?;
    if tuple != 0 {
        movement_auth(memory, base, tuple, crate::crelation::status::VTABLE_RVA)?;
        if movement_pointer(memory, tuple, crate::crelation::status::DIPLOMACY_STATUS)? != diplomacy
            || !movement_equal(
                countries,
                movement_field(memory, tuple, crate::crelation::status::TARGET_COUNTRY)?,
                target,
            )?
        {
            return Err("movement relation matrix identity mismatch");
        }
    }
    Ok(tuple)
}

fn movement_active_war(
    memory: &impl Memory,
    base: usize,
    tuple: usize,
) -> Result<bool, &'static str> {
    let war = movement_pointer(memory, tuple, crate::crelation::status::WAR_RELATION)?;
    if war == 0 {
        return Ok(false);
    }
    movement_auth(memory, base, war, crate::cwar_relation::VTABLE_RVA)?;
    if movement_field::<u32>(memory, war, crate::crelation::TYPE_ID)? != crate::cwar_relation::TYPE
    {
        return Err("movement war relation type mismatch");
    }
    Ok(movement_field::<u8>(memory, war, crate::crelation::RETIRED)? == 0)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MovementRelation {
    Friendly,
    Enemy,
    Neither,
}

pub fn movement_relation_with(
    memory: &impl Memory,
    base: usize,
    countries: &Reader<'_, impl Memory>,
    owner_raw: u32,
    target_raw: u32,
) -> Result<MovementRelation, &'static str> {
    let owner = countries.resolve(owner_raw)?;
    if movement_equal(countries, owner.id, target_raw)? {
        return Ok(MovementRelation::Friendly);
    }
    let diplomacy = movement_diplomacy(memory, base, owner.pointer)?;
    let tuple = movement_tuple(memory, base, countries, diplomacy, target_raw)?;
    if tuple == 0 {
        return Err("movement friendly relation is null");
    }
    if movement_active_war(memory, base, tuple)? {
        return Ok(if (owner.id as i32) <= 0 {
            MovementRelation::Neither
        } else {
            MovementRelation::Enemy
        });
    }
    if movement_pointer(memory, tuple, 0x2a0)? != 0 {
        return Ok(MovementRelation::Friendly);
    }
    let faction = movement_pointer(memory, diplomacy, engine_native_parts::cdiplomacy_status_layout::FACTION)?;
    if faction != 0 {
        let target = if target_raw == 0 {
            let sentinel = countries.pointer_at(0)?;
            movement_auth(memory, base, sentinel, VTABLE_RVA)?;
            if movement_field::<u32>(memory, sentinel, ID)? != 0 {
                return Err("movement sentinel country identity mismatch");
            }
            sentinel
        } else {
            countries.resolve(target_raw)?.pointer
        };
        let target_diplomacy = movement_diplomacy(memory, base, target)?;
        if faction == movement_pointer(memory, target_diplomacy, engine_native_parts::cdiplomacy_status_layout::FACTION)?
        {
            return Ok(MovementRelation::Friendly);
        }
    }
    let subject = movement_pointer(memory, tuple, crate::crelation::status::SUBJECT_RELATION)?;
    if subject == 0 {
        return Ok(MovementRelation::Neither);
    }
    if movement_field::<u32>(memory, subject, crate::crelation::subject::TYPE_ID)?
        != crate::crelation::subject::TYPE_VALUE
    {
        return Ok(MovementRelation::Neither);
    }
    movement_auth(memory, base, subject, crate::crelation::subject::VTABLE_RVA)?;
    let target = movement_field(memory, tuple, crate::crelation::status::TARGET_COUNTRY)?;
    if movement_equal(
        countries,
        movement_field(memory, subject, crate::crelation::subject::OVERLORD)?,
        target,
    )? || movement_equal(
        countries,
        movement_field(memory, subject, crate::crelation::subject::SUBJECT)?,
        target,
    )? {
        Ok(MovementRelation::Friendly)
    } else {
        Ok(MovementRelation::Neither)
    }
}

pub fn movement_friendly(
    memory: &impl Memory,
    base: usize,
    state: usize,
    owner_raw: u32,
    target_raw: u32,
) -> Result<bool, &'static str> {
    let countries = Reader::new(memory, base, state)?;
    Ok(movement_relation_with(memory, base, &countries, owner_raw, target_raw)?
        == MovementRelation::Friendly)
}

pub fn river_crossing_against(
    memory: &impl Memory,
    base: usize,
    state: usize,
    owner_raw: u32,
    target_raw: u32,
) -> Result<Option<i64>, &'static str> {
    let countries = Reader::new(memory, base, state)?;
    let owner = countries.resolve(owner_raw)?;
    river_crossing_against_with(memory, base, &countries, owner, target_raw)
}

pub fn river_crossing_against_with(
    memory: &impl Memory,
    base: usize,
    countries: &Reader<'_, impl Memory>,
    owner: Country,
    target_raw: u32,
) -> Result<Option<i64>, &'static str> {
    let diplomacy = movement_pointer(memory, owner.pointer, DIPLOMACY_STATUS)?;
    if diplomacy == 0 {
        return Ok(None);
    }
    movement_auth(
        memory,
        base,
        diplomacy,
        engine_native_parts::cdiplomacy_status_layout::VTABLE_RVA,
    )?;
    let slot = if (target_raw as i32) <= 0 {
        0
    } else {
        countries.link(target_raw)?.unwrap_or(0)
    };
    let relation = movement_element(
        memory,
        movement_array(memory, diplomacy, engine_native_parts::cdiplomacy_status_layout::RELATIONS)?,
        slot,
    )?;
    if relation == 0 {
        return Ok(None);
    }
    movement_auth(memory, base, relation, crate::crelation::status::VTABLE_RVA)?;
    if movement_pointer(memory, relation, crate::crelation::status::DIPLOMACY_STATUS)? != diplomacy {
        return Err("movement relation matrix identity mismatch");
    }
    let accumulator = relation
        .checked_add(crate::crelation::status::MODIFIER_ACCUMULATOR)
        .ok_or("movement relation accumulator overflow")?;
    let value = engine_native_parts::modifier_accumulator::lookup_requested_at(
        memory,
        accumulator,
        engine_executable::modifier_keys::RIVER_CROSSING_FACTOR_AGAINST,
    )?;
    Ok(value.present.then_some(value.raw))
}

pub fn movement_enemy(
    memory: &impl Memory,
    base: usize,
    state: usize,
    owner_raw: u32,
    target_raw: u32,
) -> Result<bool, &'static str> {
    let countries = Reader::new(memory, base, state)?;
    movement_enemy_with(memory, base, &countries, owner_raw, target_raw)
}

pub fn movement_enemy_with(
    memory: &impl Memory,
    base: usize,
    countries: &Reader<'_, impl Memory>,
    owner_raw: u32,
    target_raw: u32,
) -> Result<bool, &'static str> {
    let owner = countries.resolve(owner_raw)?;
    if movement_equal(countries, owner.id, target_raw)? || (owner.id as i32) <= 0 {
        return Ok(false);
    }
    let diplomacy = movement_diplomacy(memory, base, owner.pointer)?;
    let tuple = movement_tuple(memory, base, countries, diplomacy, target_raw)?;
    if tuple == 0 {
        return Ok(false);
    }
    movement_active_war(memory, base, tuple)
}

fn movement_province(
    memory: &impl Memory,
    base: usize,
    provinces: Array,
    province: usize,
) -> Result<u32, &'static str> {
    movement_auth(memory, base, province, crate::cprovince::VTABLE_RVA)?;
    let id = movement_field(memory, province, crate::cprovince::ID)?;
    if id == 0 || movement_element(memory, provinces, id)? != province {
        return Err("movement province identity mismatch");
    }
    Ok(id)
}

fn movement_capital_area(
    memory: &impl Memory,
    base: usize,
    state: usize,
    countries: &Reader<'_, impl Memory>,
    owner: Country,
    provinces: Array,
) -> Result<usize, &'static str> {
    let capital = movement_field::<i32>(memory, owner.pointer, CAPITAL)?;
    if capital == 0 {
        return Ok(0);
    }
    if capital < 0 {
        return Err("movement capital state is negative");
    }
    let states = movement_array(memory, state, cgame_state::STATES)?;
    let capital_state = movement_element(memory, states, capital as u32)?;
    movement_auth(memory, base, capital_state, crate::cstate::VTABLE_RVA)?;
    if movement_field::<u32>(memory, capital_state, crate::cstate::ID)? != capital as u32 {
        return Err("movement capital state identity mismatch");
    }
    let members = movement_array(memory, capital_state, crate::cstate::PROVINCES)?;
    let mut best = 0;
    let mut best_count = 0_i32;
    for i in 0..members.count {
        let province = movement_element(memory, members, i)?;
        movement_province(memory, base, provinces, province)?;
        if !movement_equal(
            countries,
            owner.id,
            movement_field(memory, province, crate::cprovince::CONTROLLER)?,
        )? {
            continue;
        }
        let area = movement_pointer(memory, province, crate::cprovince::CONTROLLER_AREA)?;
        if area != 0 {
            movement_auth(memory, base, area, engine_native_parts::ccontroller_area::VTABLE_RVA)?;
            let count =
                movement_field::<i32>(memory, area, engine_native_parts::ccontroller_area::PROVINCE_COUNT)?;
            if count < 0 {
                return Err("movement controller area count negative");
            }
            if count > best_count {
                best = area;
                best_count = count;
            }
        }
    }
    Ok(best)
}

#[derive(Clone, Copy)]
struct MovementHash {
    data: usize,
    mask: u32,
    sentinel: usize,
    wide: bool,
}

impl MovementHash {
    fn read(memory: &impl Memory, object: usize, wide: bool) -> Result<Self, &'static str> {
        if object == 0 {
            return Err("movement supply hash is null");
        }
        let (data, mask, max_probe) = if let Some(at) = object.checked_add(0x28)
            && let Some(header) = read::<[u8; 17]>(memory, at)
        {
            (
                u64::from_le_bytes(header[0..8].try_into().unwrap()) as usize,
                i32::from_le_bytes(header[12..16].try_into().unwrap()),
                header[16] as usize,
            )
        } else {
            (
                movement_pointer(memory, object, 0x28)?,
                movement_field::<i32>(memory, object, 0x34)?,
                movement_field::<u8>(memory, object, 0x38)? as usize,
            )
        };
        if mask < 0 || !(mask as u32 + 1).is_power_of_two() || data == 0 {
            return Err("movement supply hash header invalid");
        }
        let sentinel = (mask as usize)
            .checked_add(max_probe)
            .and_then(|n| n.checked_add(1))
            .ok_or("movement supply hash bound overflow")?;
        Ok(Self {
            data,
            mask: mask as u32,
            sentinel,
            wide,
        })
    }

    fn lookup(&self, memory: &impl Memory, key: u32) -> Result<Option<i64>, &'static str> {
        let mut hash = (key ^ (key >> 16)).wrapping_mul(0x045d_9f3b);
        hash = (hash ^ (hash >> 16)).wrapping_mul(0x045d_9f3b);
        let index = ((hash ^ (hash >> 16)) & self.mask) as usize;
        let (stride, marker, key_offset, value_offset) = if self.wide {
            (0x18_usize, 4, 8, 0x10)
        } else {
            (0x0c, 0, 4, 8)
        };
        for distance in 1..=256_usize {
            let slot = index
                .checked_add(distance - 1)
                .filter(|&n| n <= self.sentinel)
                .ok_or("movement supply hash probe outside allocation")?;
            let bucket = slot
                .checked_mul(stride)
                .and_then(|n| self.data.checked_add(n))
                .ok_or("movement supply hash address overflow")?;
            if self.wide {
                if let Some(raw) = read::<[u8; 0x18]>(memory, bucket) {
                    let occupied = raw[4] as usize;
                    if distance > occupied {
                        return Ok(None);
                    }
                    if u32::from_le_bytes(raw[8..12].try_into().unwrap()) == key {
                        if slot == self.sentinel {
                            return Ok(None);
                        }
                        return Ok(Some(i64::from_le_bytes(raw[0x10..0x18].try_into().unwrap())));
                    }
                    if distance == 255 {
                        return Err("movement supply hash probe distance overflow");
                    }
                    continue;
                }
            } else if let Some(raw) = read::<[u8; 0x0c]>(memory, bucket) {
                let occupied = raw[0] as usize;
                if distance > occupied {
                    return Ok(None);
                }
                if u32::from_le_bytes(raw[4..8].try_into().unwrap()) == key {
                    if slot == self.sentinel {
                        return Ok(None);
                    }
                    return Ok(Some(i64::from(i32::from_le_bytes(
                        raw[8..12].try_into().unwrap(),
                    ))));
                }
                if distance == 255 {
                    return Err("movement supply hash probe distance overflow");
                }
                continue;
            }
            let occupied = movement_field::<u8>(memory, bucket, marker)? as usize;
            if distance > occupied {
                return Ok(None);
            }
            if movement_field::<u32>(memory, bucket, key_offset)? == key {
                if slot == self.sentinel {
                    return Ok(None);
                }
                return if self.wide {
                    movement_field(memory, bucket, value_offset).map(Some)
                } else {
                    movement_field::<i32>(memory, bucket, value_offset).map(|n| Some(i64::from(n)))
                };
            }
            if distance == 255 {
                return Err("movement supply hash probe distance overflow");
            }
        }
        Err("movement supply hash probe did not terminate")
    }
}

#[cfg(test)]
fn movement_hash_value(
    memory: &impl Memory,
    object: usize,
    key: u32,
    wide: bool,
) -> Result<Option<i64>, &'static str> {
    MovementHash::read(memory, object, wide)?.lookup(memory, key)
}

struct CachedOwnerSupply {
    raw: u32,
    owner: Result<Country, &'static str>,
    capital_area: Result<usize, &'static str>,
    companion_ptr: Option<Result<usize, &'static str>>,
    companion_hash: Option<Result<MovementHash, &'static str>>,
}

pub struct CoreSupplyAccessCache<'a, M: Memory> {
    memory: &'a M,
    base: usize,
    state: usize,
    countries: &'a Reader<'a, M>,
    provinces: Result<Array, &'static str>,
    systems: Option<Result<crate::ccountry_supply_system::world::Reader<'a, M>, &'static str>>,
    anchors_ptr: Option<Result<usize, &'static str>>,
    anchors_hash: Option<Result<MovementHash, &'static str>>,
    owner: Option<CachedOwnerSupply>,
}

impl<'a, M: Memory> CoreSupplyAccessCache<'a, M> {
    pub fn new(
        memory: &'a M,
        base: usize,
        state: usize,
        countries: &'a Reader<'a, M>,
    ) -> Self {
        let provinces = movement_array(memory, state, cgame_state::PROVINCES);
        Self {
            memory,
            base,
            state,
            countries,
            provinces,
            systems: None,
            anchors_ptr: None,
            anchors_hash: None,
            owner: None,
        }
    }

    pub fn has_access(&mut self, owner_raw: u32, area: usize) -> Result<bool, &'static str> {
        if let Some(result) =
            self.memory
                .core_supply_access(self.base, self.state, owner_raw, area)
        {
            return result;
        }
        let provinces = self.provinces;
        let cached = match &mut self.owner {
            Some(cached) if cached.raw == owner_raw => cached,
            slot => {
                let owner = self.countries.resolve(owner_raw);
                let capital_area = owner.and_then(|owner| {
                    let provinces = provinces?;
                    movement_capital_area(
                        self.memory,
                        self.base,
                        self.state,
                        self.countries,
                        owner,
                        provinces,
                    )
                });
                slot.insert(CachedOwnerSupply {
                    raw: owner_raw,
                    owner,
                    capital_area,
                    companion_ptr: None,
                    companion_hash: None,
                })
            }
        };
        let owner = cached.owner?;
        let provinces = provinces?;
        let capital_area = cached.capital_area?;
        if area == capital_area {
            return Ok(true);
        }
        if area == 0 {
            return Ok(false);
        }
        movement_auth(self.memory, self.base, area, engine_native_parts::ccontroller_area::VTABLE_RVA)?;
        let memory = self.memory;
        let base = self.base;
        let state = self.state;
        let systems = self
            .systems
            .get_or_insert_with(|| crate::ccountry_supply_system::world::Reader::new(memory, base, state));
        let companion_ptr = *cached.companion_ptr.get_or_insert_with(|| {
            let systems = systems.as_ref().map_err(|reason| *reason)?;
            let system = systems
                .at(owner.canonical_slot)?
                .ok_or("movement country supply is absent")?;
            movement_pointer(
                memory,
                system.pointer,
                crate::ccountry_supply_system::COMPANION,
            )
        });
        let companion_ptr = companion_ptr?;
        let members = movement_array(memory, area, 0xb0)?;
        let anchors_ptr = *self
            .anchors_ptr
            .get_or_insert_with(|| movement_pointer(memory, state, cgame_state::STRATEGIC_NAVY_MANAGER));
        let anchors_ptr = anchors_ptr?;
        if members.count == 0 {
            return Ok(false);
        }
        let anchors_hash = *self
            .anchors_hash
            .get_or_insert_with(|| MovementHash::read(memory, anchors_ptr, true));
        let mut i = 0u32;
        let mut raw = [0u8; 64 * 8];
        while i < members.count {
            let remaining = (members.count - i) as usize;
            let batch = remaining.min(64);
            let chunk_ok = batch >= 4
                && members
                    .element(i, 8)
                    .is_some_and(|at| memory.copy(at, &mut raw[..batch * 8]));
            let step = if chunk_ok { batch } else { 1 };
            for offset in 0..step {
                let province = if chunk_ok {
                    let at = offset * 8;
                    u64::from_le_bytes(raw[at..at + 8].try_into().unwrap()) as usize
                } else {
                    movement_element(memory, members, i + offset as u32)?
                };
                let id = movement_province(memory, base, provinces, province)?;
                if anchors_hash?.lookup(memory, id)?.is_some_and(|n| n != 0) {
                    let companion_hash = *cached
                        .companion_hash
                        .get_or_insert_with(|| MovementHash::read(memory, companion_ptr, false));
                    if companion_hash?.lookup(memory, id)?.is_some_and(|n| n >= 0) {
                        return Ok(true);
                    }
                }
            }
            i += step as u32;
        }
        Ok(false)
    }
}

pub fn has_core_supply_access(
    memory: &impl Memory,
    base: usize,
    state: usize,
    owner_raw: u32,
    area: usize,
) -> Result<bool, &'static str> {
    if let Some(result) = memory.core_supply_access(base, state, owner_raw, area) {
        return result;
    }
    has_core_supply_access_direct(memory, base, state, owner_raw, area)
}

pub fn has_core_supply_access_direct(
    memory: &impl Memory,
    base: usize,
    state: usize,
    owner_raw: u32,
    area: usize,
) -> Result<bool, &'static str> {
    let countries = Reader::new(memory, base, state)?;
    let owner = countries.resolve(owner_raw)?;
    let provinces = movement_array(memory, state, cgame_state::PROVINCES)?;
    let capital_area = movement_capital_area(memory, base, state, &countries, owner, provinces)?;
    if area == capital_area {
        return Ok(true);
    }
    if area == 0 {
        return Ok(false);
    }
    movement_auth(memory, base, area, engine_native_parts::ccontroller_area::VTABLE_RVA)?;
    let systems = crate::ccountry_supply_system::world::Reader::new(memory, base, state)?;
    let system = systems
        .at(owner.canonical_slot)?
        .ok_or("movement country supply is absent")?;
    let companion = movement_pointer(
        memory,
        system.pointer,
        crate::ccountry_supply_system::COMPANION,
    )?;
    let members = movement_array(memory, area, 0xb0)?;
    let anchors = movement_pointer(memory, state, cgame_state::STRATEGIC_NAVY_MANAGER)?;
    if members.count == 0 {
        return Ok(false);
    }
    let anchors_hash = MovementHash::read(memory, anchors, true);
    let mut companion_hash = None;
    for i in 0..members.count {
        let province = movement_element(memory, members, i)?;
        let id = movement_province(memory, base, provinces, province)?;
        if anchors_hash?.lookup(memory, id)?.is_some_and(|n| n != 0) {
            let companion_hash =
                *companion_hash.get_or_insert_with(|| MovementHash::read(memory, companion, false));
            if companion_hash?.lookup(memory, id)?.is_some_and(|n| n >= 0) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

pub fn walk_links(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut [LinkRow],
) -> Outcome {
    walk_links_observed_inner(memory, base, game_state, output, None)
}
pub fn walk_links_observed(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut [LinkRow],
    journal: &mut engine_native_memory::raw_country_observations::Journal,
) -> Outcome {
    walk_links_observed_inner(memory, base, game_state, output, Some(journal))
}
fn walk_links_observed_inner(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut [LinkRow],
    mut journal: Option<&mut engine_native_memory::raw_country_observations::Journal>,
) -> Outcome {
    let Some(countries) = game_state
        .checked_add(cgame_state::COUNTRIES)
        .and_then(|p| Array::read(memory, p, 8))
    else {
        if let Some(journal) = journal.as_deref_mut() {
            journal.link_controls(Err("country array unreadable or invalid"));
        }
        return Outcome::Unavailable {
            reason: "country array unreadable or invalid",
        };
    };
    let Some(links) = game_state
        .checked_add(cgame_state::COUNTRY_LINKS)
        .and_then(|p| Array::read(memory, p, 4))
    else {
        if let Some(journal) = journal.as_deref_mut() {
            journal.link_controls(Err("country links unreadable or invalid"));
        }
        return Outcome::Unavailable {
            reason: "country links unreadable or invalid",
        };
    };
    if let Some(journal) = journal.as_deref_mut() {
        journal.link_controls(Ok([countries, links]));
    }
    let result = (|| {
        if countries.count < 2 {
            return Err("country table has no country slots");
        }
        if links.count == 0 {
            return Err("country links have no sentinel slot");
        }
        let vtable = engine_sites::at(base, VTABLE_RVA).ok_or("module address overflow")?;
        let mapping = |id, journal: &mut Option<&mut engine_native_memory::raw_country_observations::Journal>| {
            let result = links
                .element(id, 4)
                .and_then(|p| read::<u32>(memory, p))
                .ok_or("country link unreadable or outside link table");
            if let Some(journal) = journal.as_deref_mut() {
                journal.link(id, result);
            }
            result
        };
        let mut len = 0;
        let mut unread_rows = 0;
        for id in 1..links.count {
            let canonical_slot = mapping(id, &mut journal)?;
            if canonical_slot == 0 {
                continue;
            }
            if canonical_slot >= countries.count {
                return Err("country link outside country table");
            }
            let pointer = countries
                .element(canonical_slot, 8)
                .and_then(|p| read::<u64>(memory, p))
                .and_then(|p| usize::try_from(p).ok())
                .ok_or("canonical country pointer unreadable")?;
            if pointer == 0 {
                return Err("country link resolves to null country");
            }
            if read::<u64>(memory, pointer) != Some(vtable as u64) {
                return Err("country vtable mismatch");
            }
            let object_id = pointer
                .checked_add(ID)
                .and_then(|p| read::<u32>(memory, p))
                .ok_or("country id unreadable")?;
            if object_id == 0 {
                return Err("country id is zero");
            }
            if mapping(canonical_slot, &mut journal)? != canonical_slot
                || mapping(object_id, &mut journal)? != canonical_slot
            {
                return Err("country link does not resolve to its canonical object");
            }
            if len == output.len() {
                unread_rows += 1;
            } else {
                output[len] = LinkRow { id, canonical_slot };
                len += 1;
            }
        }
        Ok(if unread_rows == 0 {
            Outcome::Rows { len }
        } else {
            Outcome::Truncated { len, unread_rows }
        })
    })();
    result.unwrap_or_else(|reason| Outcome::Unavailable { reason })
}

pub fn walk(memory: &impl Memory, base: usize, game_state: usize, output: &mut [Row]) -> Outcome {
    walk_observed_inner(memory, base, game_state, output, None)
}
pub fn walk_observed(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut [Row],
    journal: &mut engine_native_memory::raw_country_observations::Journal,
) -> Outcome {
    walk_observed_inner(memory, base, game_state, output, Some(journal))
}
fn walk_observed_inner<M: Memory>(
    memory: &M,
    base: usize,
    game_state: usize,
    output: &mut [Row],
    mut journal: Option<&mut engine_native_memory::raw_country_observations::Journal>,
) -> Outcome {
    let Some(countries) = game_state
        .checked_add(cgame_state::COUNTRIES)
        .and_then(|p| Array::read(memory, p, 8))
    else {
        if let Some(journal) = journal.as_deref_mut() {
            journal.link_controls(Err("country array unreadable or invalid"));
        }
        return Outcome::Unavailable {
            reason: "country array unreadable or invalid",
        };
    };
    let tag_control = game_state
        .checked_add(cgame_state::COUNTRY_TAGS)
        .and_then(|p| Array::read(memory, p, STRIDE))
        .ok_or("country link tags unreadable or invalid");
    if let Some(journal) = journal.as_deref_mut() {
        journal.tag_control(tag_control);
    }
    let Ok(tags) = tag_control else {
        return Outcome::Unavailable {
            reason: "country link tags unreadable or invalid",
        };
    };
    let Some(links) = game_state
        .checked_add(cgame_state::COUNTRY_LINKS)
        .and_then(|p| Array::read(memory, p, 4))
    else {
        if let Some(journal) = journal.as_deref_mut() {
            journal.link_controls(Err("country links unreadable or invalid"));
        }
        return Outcome::Unavailable {
            reason: "country links unreadable or invalid",
        };
    };
    if let Some(journal) = journal.as_deref_mut() {
        journal.link_controls(Ok([countries, links]));
    }
    if links.count != tags.count {
        return Outcome::Unavailable {
            reason: "country links and tags have different counts",
        };
    }
    let read_row = |id: u32,
                    canonical_slot: u32,
                    pointer: usize,
                    journal: &mut Option<&mut engine_native_memory::raw_country_observations::Journal>|
     -> Result<Row, &'static str> {
        let at = |offset| {
            pointer
                .checked_add(offset)
                .ok_or("country address overflow")
        };
        let scalar = |offset| read::<i64>(memory, at(offset)?).ok_or("country scalar unreadable");
        let tag_address = tags
            .element(id, STRIDE)
            .ok_or("country id outside link tags")?;
        let tag = match journal.as_deref_mut() {
            Some(journal) => engine_native_memory::msvc_string::read_text_observed::<8, true>(
                memory,
                tag_address,
                id as i32,
                engine_native_memory::raw_country_observations::Source::Countries,
                journal,
            ),
            None => read_text_prefetched::<8>(memory, tag_address),
        }
        .ok_or("country tag unreadable or too long")?;
        if tag.len == 0 {
            return Err("country tag empty");
        }
        let fields = Fields::gather(memory, pointer);
        let status = fields
            .get::<u64>(memory, pointer, GATHERED_STATUS)?
            .ok_or("political status pointer unreadable")?;
        let political_power_raw = if status == 0 {
            None
        } else {
            let status =
                usize::try_from(status).map_err(|_| "political status address overflow")?;
            let vtable = engine_sites::at(base, political_status_layout::VTABLE_RVA)
                .ok_or("module address overflow")?;
            if read::<u64>(memory, status) != Some(vtable as u64) {
                return Err("political status vtable mismatch");
            }
            if status
                .checked_add(political_status_layout::COUNTRY)
                .and_then(|p| read::<u64>(memory, p))
                != Some(pointer as u64)
            {
                return Err("political status country mismatch");
            }
            Some(
                status
                    .checked_add(political_status_layout::POLITICAL_POWER)
                    .and_then(|p| read::<i64>(memory, p))
                    .ok_or("political power unreadable")?,
            )
        };
        Ok(Row {
            id,
            canonical_slot,
            tag,
            political_power_raw,
            politics_witness: (|| {
                let instance = fields
                    .get::<u32>(memory, pointer, GATHERED_SURRENDER_TARGET)
                    .ok()??;
                let party = if status == 0 {
                    None
                } else {
                    let pointer = read::<u64>(
                        memory,
                        (status as usize).checked_add(political_status_layout::RULING_PARTY)?,
                    )? as usize;
                    if pointer == 0 {
                        None
                    } else {
                        Some(political_party_layout::identity(memory, base, pointer).ok()?.0)
                    }
                };
                Some((instance, party))
            })(),
            border_land_doctrine_level: None,
            manpower_residual: fields
                .get::<i32>(memory, pointer, GATHERED_MANPOWER)?
                .ok_or("country manpower residual unreadable")?,
            command_power_raw: fields
                .get::<i64>(memory, pointer, GATHERED_COMMAND_POWER)?
                .ok_or("country scalar unreadable")?,
            stability_raw: fields
                .get::<i64>(memory, pointer, GATHERED_STABILITY)?
                .ok_or("country scalar unreadable")?,
            war_support_raw: fields
                .get::<i64>(memory, pointer, GATHERED_WAR_SUPPORT)?
                .ok_or("country scalar unreadable")?,
            peace_conference_participant: fields
                .get::<u8>(memory, pointer, GATHERED_PEACE)?
                .ok_or("peace conference participation unreadable")?,
            convoys_raw: {
                let valid = fields
                    .get::<u8>(memory, pointer, GATHERED_CONVOY_VALID)?
                    .ok_or("convoy cache validity unreadable")?;
                if valid == 0 {
                    None
                } else {
                    Some(scalar(CONVOY_STATE + cconvoys::CACHE_RAW)?)
                }
            },
        })
    };
    let result = (|| {
        if countries.count < 2 {
            return Err("country table has no country slots");
        }
        let vtable = engine_sites::at(base, VTABLE_RVA).ok_or("module address overflow")?;
        let pointer_at = |index| {
            countries
                .element(index, 8)
                .and_then(|p| read::<u64>(memory, p))
                .and_then(|p| usize::try_from(p).ok())
                .ok_or("country pointer unreadable")
        };
        let canonical =
            |id, journal: &mut Option<&mut engine_native_memory::raw_country_observations::Journal>| {
                if id == 0 {
                    return Err("country id is zero");
                }
                let result = links
                    .element(id, 4)
                    .and_then(|p| read::<u32>(memory, p))
                    .ok_or("country link unreadable or outside country table");
                if let Some(journal) = journal.as_deref_mut() {
                    journal.link(id, result);
                }
                result.and_then(|id| {
                    if id > 0 && id < countries.count {
                        Ok(id)
                    } else {
                        Err("country link unreadable or outside country table")
                    }
                })
            };
        let mut len = 0;
        let mut unread_rows = 0;
        let mut reserved = cached_reserved_observation(memory, base, game_state);
        for slot in 1..countries.count {
            let original = (|| {
                let pointer = pointer_at(slot)?;
                if pointer == 0 {
                    return Ok(None);
                }
                if let Some(alias) = reserved.and_then(|root| root.physical_alias(pointer)) {
                    alias?;
                    return reserved
                        .expect("original reserved alias")
                        .result()
                        .map(Some);
                }
                if read::<u64>(memory, pointer) != Some(vtable as u64) {
                    return Err("country vtable mismatch");
                }
                let id = pointer
                    .checked_add(ID)
                    .and_then(|p| read::<u32>(memory, p))
                    .ok_or("country id unreadable")?;
                if id == 0 {
                    let root = *reserved.get_or_insert_with(|| {
                        observe_reserved_from_array(memory, base, countries)
                    });
                    if let Some(alias) = root.physical_alias(pointer) {
                        alias?;
                        return root.result().map(Some);
                    }
                }
                let canonical_id = canonical(id, &mut journal)?;
                if canonical(slot, &mut journal)? != canonical_id {
                    return Err("country slot and object resolve to different countries");
                }
                if pointer_at(canonical_id)? != pointer {
                    return Err("canonical country slot points to a different object");
                }
                Ok(Some(Country {
                    id,
                    canonical_slot: canonical_id,
                    pointer,
                }))
            })();
            let Some(country) = original? else {
                continue;
            };
            if slot != country.canonical_slot {
                continue;
            }
            let Country {
                id,
                canonical_slot: canonical_id,
                pointer,
            } = country;
            if len == output.len() {
                unread_rows += 1;
            } else {
                output[len] = read_row(id, canonical_id, pointer, &mut journal)?;
                len += 1;
            }
        }
        if unread_rows != 0 {
            Ok(Outcome::Truncated { len, unread_rows })
        } else if len == 0 {
            Err("country table has no readable countries")
        } else {
            Ok(Outcome::Rows { len })
        }
    })();
    result.unwrap_or_else(|reason| Outcome::Unavailable { reason })
}

pub const SENT_VOLUNTEERS: usize = 0x310;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SentVolunteerRow {
    pub country: u32,
    pub ordinal: u32,
    pub division: u64,
}

pub fn count_sent_volunteers(memory: &impl Memory, base: usize, game_state: usize) -> Outcome {
    walk_sent_volunteers_buffer(memory, base, game_state, Buffer::Count)
}


pub fn walk_sent_volunteers_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut output: Buffer<'_, SentVolunteerRow>,
) -> Outcome {
    let result = (|| {
        let countries = Reader::new(memory, base, game_state)?;
        let mut len = 0_usize;
        let mut unread_rows = 0_u32;
        for slot in 1..countries.count() {
            let Some(country) = countries.at(slot)? else {
                continue;
            };
            let array = country
                .pointer
                .checked_add(SENT_VOLUNTEERS)
                .and_then(|at| Array::read(memory, at, 8))
                .ok_or("sent volunteers vector unreadable or invalid")?;
            output.ensure(
                len.checked_add(array.count as usize)
                    .ok_or("sent volunteer demand overflow")?,
            )?;
            let mut position = 0;
            let mut bytes = [0_u8; 512];
            while position < array.count {
                let count = (array.count - position).min(64);
                let chunk = &mut bytes[..count as usize * 8];
                if !memory.copy(array.slice(position, count, 8)?.data, chunk) {
                    return Err("sent volunteer handles unreadable");
                }
                for (index, value) in chunk.chunks_exact(8).enumerate() {
                    if len == output.len() {
                        unread_rows = unread_rows
                            .checked_add(1)
                            .ok_or("sent volunteers omitted count overflow")?;
                    } else {
                        output[len] = SentVolunteerRow {
                            country: country.id,
                            ordinal: position + index as u32,
                            division: u64::from_le_bytes(value.try_into().unwrap()),
                        };
                        len += 1;
                    }
                }
                position += count;
            }
        }
        Ok(if unread_rows == 0 {
            Outcome::Rows { len }
        } else {
            Outcome::Truncated { len, unread_rows }
        })
    })();
    result.unwrap_or_else(|reason| Outcome::Unavailable { reason })
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct VolunteerHostRow {
    pub country: u32,
    pub position: u32,
    pub host: u32,
    pub deployed_divisions: i32,
    pub air_capacity_modifier: Option<i64>,
}

pub fn count_volunteer_hosts(memory: &impl Memory, base: usize, game_state: usize) -> Outcome {
    walk_volunteer_hosts_buffer(memory, base, game_state, Buffer::Count)
}

pub fn walk_volunteer_hosts_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: Buffer<'_, VolunteerHostRow>,
) -> Outcome {
    walk_volunteer_hosts_with_air_buffer(memory, base, game_state, output, false)
}

pub fn walk_volunteer_hosts_with_air_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut output: Buffer<'_, VolunteerHostRow>,
    air_modifiers: bool,
) -> Outcome {
    let result = (|| {
        let countries = Reader::new(memory, base, game_state)?;
        let mut len = 0_usize;
        let mut unread_rows = 0_u32;
        for slot in 1..countries.count() {
            let Some(country) = countries.at(slot)? else {
                continue;
            };
            let hosts = country
                .pointer
                .checked_add(VOLUNTEER_HOSTS)
                .and_then(|at| Array::read(memory, at, 4))
                .ok_or("volunteer host vector unreadable or invalid")?;
            let tallies = country
                .pointer
                .checked_add(VOLUNTEER_TALLIES)
                .and_then(|at| Array::read(memory, at, 4))
                .ok_or("volunteer tally vector unreadable or invalid")?;
            if hosts.count != tallies.count {
                return Err("volunteer host and tally vectors have different counts");
            }
            output.ensure(
                len.checked_add(hosts.count as usize)
                    .ok_or("volunteer host demand overflow")?,
            )?;
            const BATCH: usize = 64;
            let mut host_batch = [0_u32; BATCH];
            let mut tally_batch = [0_u32; BATCH];
            let mut position = 0;
            while position < hosts.count {
                let count = (hosts.count - position) as usize;
                let count = count.min(BATCH);
                copy_u32s(
                    memory,
                    hosts.slice(position, count as u32, 4)?,
                    &mut host_batch[..count],
                    "volunteer host entry unreadable",
                )?;
                copy_u32s(
                    memory,
                    tallies.slice(position, count as u32, 4)?,
                    &mut tally_batch[..count],
                    "volunteer tally entry unreadable",
                )?;
                for index in 0..count {
                    if len == output.len() {
                        unread_rows = unread_rows
                            .checked_add(1)
                            .ok_or("volunteer host omitted count overflow")?;
                        continue;
                    }
                    output[len] = VolunteerHostRow {
                        country: country.id,
                        position: position + index as u32,
                        host: host_batch[index],
                        deployed_divisions: tally_batch[index] as i32,
                        air_capacity_modifier: air_modifiers
                            .then(|| {
                                volunteer_air_modifier(
                                    memory,
                                    base,
                                    &countries,
                                    country,
                                    host_batch[index],
                                )
                                .ok()
                            })
                            .flatten(),
                    };
                    len += 1;
                }
                position += count as u32;
            }
        }
        Ok(if unread_rows == 0 {
            Outcome::Rows { len }
        } else {
            Outcome::Truncated { len, unread_rows }
        })
    })();
    result.unwrap_or_else(|reason| Outcome::Unavailable { reason })
}

fn volunteer_air_modifier(
    memory: &impl Memory,
    base: usize,
    countries: &Reader<'_, impl Memory>,
    sender: Country,
    host: u32,
) -> Result<i64, &'static str> {
    let slot = if host == 0 {
        0
    } else {
        countries.link(host)?.unwrap_or(0)
    };
    let diplomacy = read::<u64>(memory, sender.pointer + DIPLOMACY_STATUS)
        .and_then(|p| usize::try_from(p).ok())
        .ok_or("volunteer diplomacy unavailable")?;
    if read::<u64>(memory, diplomacy)
        != engine_sites::at(base, engine_native_parts::cdiplomacy_status_layout::VTABLE_RVA).map(|p| p as u64)
    {
        return Err("volunteer diplomacy identity mismatch");
    }
    if read::<u64>(memory, diplomacy + engine_native_parts::cdiplomacy_status_layout::COUNTRY)
        != Some(sender.pointer as u64)
    {
        return Err("volunteer diplomacy owner mismatch");
    }
    let relations = Array::read(memory, diplomacy + engine_native_parts::cdiplomacy_status_layout::RELATIONS, 8)
        .ok_or("volunteer relation matrix unavailable")?;
    let relation = relations
        .element(slot, 8)
        .and_then(|p| read::<u64>(memory, p))
        .and_then(|p| usize::try_from(p).ok())
        .ok_or("volunteer relation unavailable")?;
    if read::<u64>(memory, relation)
        != engine_sites::at(base, crate::crelation::status::VTABLE_RVA).map(|p| p as u64)
    {
        return Err("volunteer relation identity mismatch");
    }
    if read::<u64>(memory, relation + crate::crelation::status::DIPLOMACY_STATUS)
        != Some(diplomacy as u64)
    {
        return Err("volunteer relation owner mismatch");
    }
    let target = read::<u32>(memory, relation + crate::crelation::status::TARGET_COUNTRY)
        .ok_or("volunteer relation target unreadable")?;
    let target_slot = if target == 0 {
        0
    } else {
        countries.link(target)?.unwrap_or(0)
    };
    if target_slot != slot {
        return Err("volunteer relation target mismatch");
    }
    let accumulator = relation
        .checked_add(crate::crelation::status::MODIFIER_ACCUMULATOR)
        .ok_or("volunteer relation modifier address overflow")?;
    engine_native_parts::modifier_accumulator::requested_effective_at(
        memory,
        base,
        accumulator,
        AIR_MODIFIER_KEYS[4],
    )
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CounterInputsRow {
    pub country: u32,
    pub pride_of_the_fleet: u64,
    pub owned_state_count: i32,
    pub controlled_state_count: i32,
    pub first_division: u64,
    pub state_rosters: Option<[RosterSpan; 3]>,
    pub leader_rosters: Option<[RosterSpan; 2]>,
    pub sent_expeditionaries: Option<RosterSpan>,
    pub roster_reason: Option<&'static str>,
    pub neighbors: Option<RosterSpan>,
    pub mio_policies: Option<RosterSpan>,
    pub neighbor_reason: Option<&'static str>,
    pub policy_reason: Option<&'static str>,
    pub attack_modifiers: Option<[Option<i64>; 2]>,
    pub movement_modifiers: Option<[Option<i64>; MOVEMENT_MODIFIER_KEYS.len()]>,
    pub recruitment_modifiers: Option<[Option<i64>; 4]>,
    pub air_modifiers: Option<[Option<i64>; AIR_MODIFIER_KEYS.len()]>,
    pub air_combat_modifiers: Option<[Option<i64>; AIR_COMBAT_MODIFIER_KEYS.len()]>,
    pub mines_effect_reduction: Option<Option<i64>>,
    pub naval_detection: Option<Option<i64>>,
}

pub const AIR_MODIFIER_KEYS: [u32; 10] = [
    engine_executable::modifier_keys::AIR_INTERCEPT_DETECTION,
    engine_executable::modifier_keys::AIR_SUPERIORITY_DETECTION,
    engine_executable::modifier_keys::AIR_HOME_DEFENCE_FACTOR,
    engine_executable::modifier_keys::AIR_POWER_PROJECTION_FACTOR,
    engine_executable::modifier_keys::AIR_VOLUNTEER_CAPACITY,
    engine_executable::modifier_keys::AIR_STRATEGIC_BOMBER_NIGHT_PENALTY,
    engine_executable::modifier_keys::AIR_NIGHT_PENALTY,
    engine_executable::modifier_keys::AIR_WEATHER_PENALTY,
    engine_executable::modifier_keys::AIR_CARRIER_NIGHT_PENALTY_REDUCTION_FACTOR,
    engine_executable::modifier_keys::AIR_FUEL_CONSUMPTION_FACTOR,
];

pub const AIR_COMBAT_MODIFIER_KEYS: [u32; 7] = [
    engine_executable::modifier_keys::AIR_SUPERIORITY_EFFICIENCY,
    engine_executable::modifier_keys::AIR_INTERCEPT_EFFICIENCY,
    engine_executable::modifier_keys::AIR_CAS_EFFICIENCY,
    engine_executable::modifier_keys::AIR_NAV_EFFICIENCY,
    engine_executable::modifier_keys::AIR_ATTACK_FACTOR,
    engine_executable::modifier_keys::AIR_DEFENCE_FACTOR,
    engine_executable::modifier_keys::AIR_STRATEGIC_BOMBER_DEFENCE_FACTOR,
];

pub const MOVEMENT_MODIFIER_KEYS: [u32; 10] = [
    engine_executable::modifier_keys::ARMY_SPEED_FACTOR,
    engine_executable::modifier_keys::ARMY_ARMOR_SPEED_FACTOR,
    engine_executable::modifier_keys::ATTRITION,
    engine_executable::modifier_keys::WINTER_ATTRITION_FACTOR,
    engine_executable::modifier_keys::HEAT_ATTRITION_FACTOR,
    engine_executable::modifier_keys::OUT_OF_SUPPLY_FACTOR,
    engine_executable::modifier_keys::COMBAT_WIDTH_FACTOR,
    engine_executable::modifier_keys::SPECIAL_FORCES_OUT_OF_SUPPLY_FACTOR,
    engine_executable::modifier_keys::ARMY_ATTACK_SPEED_FACTOR,
    engine_executable::modifier_keys::ARMY_RETREAT_SPEED_FACTOR,
];

pub const RECRUITMENT_MODIFIER_KEYS: [u32; 4] = [
    engine_executable::modifier_keys::MOBILIZATION_SPEED,
    engine_executable::modifier_keys::CONSCRIPTION,
    engine_executable::modifier_keys::CONSCRIPTION_FACTOR,
    engine_executable::modifier_keys::NON_CORE_MANPOWER,
];

const COUNTER_MODIFIER_KEYS: [u32; MOVEMENT_MODIFIER_KEYS.len()
    + ATTACK_MODIFIER_KEYS.len()
    + AIR_MODIFIER_KEYS.len()
    + AIR_COMBAT_MODIFIER_KEYS.len()
    + 6] = [
    AIR_MODIFIER_KEYS[0],
    AIR_MODIFIER_KEYS[1],
    AIR_COMBAT_MODIFIER_KEYS[0],
    AIR_COMBAT_MODIFIER_KEYS[1],
    AIR_COMBAT_MODIFIER_KEYS[2],
    AIR_COMBAT_MODIFIER_KEYS[3],
    engine_executable::modifier_keys::NAVAL_DETECTION,
    engine_executable::modifier_keys::ARMY_SPEED_FACTOR,
    engine_executable::modifier_keys::ARMY_ARMOR_SPEED_FACTOR,
    RECRUITMENT_MODIFIER_KEYS[3],
    RECRUITMENT_MODIFIER_KEYS[0],
    RECRUITMENT_MODIFIER_KEYS[1],
    RECRUITMENT_MODIFIER_KEYS[2],
    engine_executable::modifier_keys::ATTRITION,
    engine_executable::modifier_keys::WINTER_ATTRITION_FACTOR,
    engine_executable::modifier_keys::HEAT_ATTRITION_FACTOR,
    engine_executable::modifier_keys::OUT_OF_SUPPLY_FACTOR,
    AIR_COMBAT_MODIFIER_KEYS[4],
    AIR_COMBAT_MODIFIER_KEYS[5],
    AIR_MODIFIER_KEYS[2],
    AIR_MODIFIER_KEYS[3],
    AIR_COMBAT_MODIFIER_KEYS[6],
    AIR_MODIFIER_KEYS[4],
    engine_executable::modifier_keys::LAND_NIGHT_ATTACK,
    AIR_MODIFIER_KEYS[5],
    AIR_MODIFIER_KEYS[6],
    AIR_MODIFIER_KEYS[7],
    AIR_MODIFIER_KEYS[8],
    engine_executable::modifier_keys::COMBAT_WIDTH_FACTOR,
    engine_executable::modifier_keys::SPECIAL_FORCES_OUT_OF_SUPPLY_FACTOR,
    AIR_MODIFIER_KEYS[9],
    engine_executable::modifier_keys::NAVAL_MINES_EFFECT_REDUCTION,
    engine_executable::modifier_keys::ARMY_ATTACK_SPEED_FACTOR,
    engine_executable::modifier_keys::LAND_BUNKER_EFFECTIVENESS_FACTOR,
    engine_executable::modifier_keys::ARMY_RETREAT_SPEED_FACTOR,
];

fn counter_entries<const N: usize>(
    values: &[Option<i64>; COUNTER_MODIFIER_KEYS.len()],
    keys: &[u32; N],
) -> [Option<i64>; N] {
    keys.map(|key| {
        COUNTER_MODIFIER_KEYS
            .iter()
            .position(|&merged| merged == key)
            .and_then(|at| values[at])
    })
}

pub const ATTACK_MODIFIER_KEYS: [u32; 2] = [
    engine_executable::modifier_keys::LAND_NIGHT_ATTACK,
    engine_executable::modifier_keys::LAND_BUNKER_EFFECTIVENESS_FACTOR,
];

#[allow(clippy::too_many_arguments)]
pub fn walk_counter_inputs_with_effect_rosters_observed<'a, M: Memory>(
    memory: &'a M,
    base: usize,
    game_state: usize,
    output: &mut Vec<CounterInputsRow>,
    rosters: &mut CounterRosters<'_>,
    effects: &mut EffectRosters<'_>,
    observed: &mut Option<Result<Reader<'a, M>, &'static str>>,
) -> Outcome {
    walk_counter_inputs_inner(
        memory,
        base,
        game_state,
        engine_native_memory::walk::Buffer::Growing(output),
        Some(rosters),
        Some(effects),
        Some(observed),
        None,
    )
}

pub type CounterCarry<'a> = dyn for<'r, 's, 'e, 'f> FnMut(
        Country,
        &'r mut CounterRosters<'s>,
        &'e mut EffectRosters<'f>,
    ) -> Option<CounterInputsRow>
    + 'a;

#[allow(clippy::too_many_arguments)]
pub fn walk_counter_inputs_carried<'a, M: Memory>(
    memory: &'a M,
    base: usize,
    game_state: usize,
    output: &mut Vec<CounterInputsRow>,
    rosters: &mut CounterRosters<'_>,
    effects: &mut EffectRosters<'_>,
    observed: &mut Option<Result<Reader<'a, M>, &'static str>>,
    carry: &mut CounterCarry<'_>,
) -> Outcome {
    walk_counter_inputs_inner(
        memory,
        base,
        game_state,
        engine_native_memory::walk::Buffer::Growing(output),
        Some(rosters),
        Some(effects),
        Some(observed),
        Some(carry),
    )
}

fn walk_counter_inputs_inner<'a, M: Memory>(
    memory: &'a M,
    base: usize,
    game_state: usize,
    mut output: engine_native_memory::walk::Buffer<'_, CounterInputsRow>,
    mut rosters: Option<&mut CounterRosters<'_>>,
    mut effects: Option<&mut EffectRosters<'_>>,
    observed: Option<&mut Option<Result<Reader<'a, M>, &'static str>>>,
    mut carry: Option<&mut CounterCarry<'_>>,
) -> Outcome {
    let mut unobserved = None;
    let observed = observed.unwrap_or(&mut unobserved);
    *observed = Some(Reader::new(memory, base, game_state));
    let result = (|| {
        let reader = observed
            .as_ref()
            .expect("country reader attempted above")
            .as_ref()
            .map_err(|&reason| reason)?;
        output.ensure(reader.count().saturating_sub(1) as usize)?;
        let expected =
            engine_sites::at(base, VTABLE_RVA).ok_or("country counter module address overflow")?;
        let mut len = 0;
        let mut unread_rows = 0_u32;
        let mut found_country = false;
        for slot in 1..reader.count() {
            let Some(country) = reader.at(slot)? else {
                continue;
            };
            found_country = true;
            let fits = len < output.len();
            if !fits && rosters.is_none() && effects.is_none() {
                unread_rows = unread_rows
                    .checked_add(1)
                    .ok_or("country counter unread row count overflow")?;
                continue;
            }
            let at = |offset| {
                country
                    .pointer
                    .checked_add(offset)
                    .ok_or("country counter address overflow")
            };
            let identify = || -> Result<(), &'static str> {
                if read::<u64>(memory, country.pointer) != Some(expected as u64)
                    || read::<u32>(memory, at(ID)?) != Some(country.id)
                {
                    return Err("country counter identity changed");
                }
                Ok(())
            };
            let mut identified = false;
            if fits
                && let Some(carry) = carry.as_deref_mut()
                && let Some(rosters) = rosters.as_deref_mut()
                && let Some(effects) = effects.as_deref_mut()
            {
                identify()?;
                identified = true;
                if let Some(row) = carry(country, rosters, effects) {
                    output[len] = row;
                    len += 1;
                    continue;
                }
            }
            let hint = |offset: usize, len| {
                memory.prefetch(country.pointer.wrapping_add(offset), len);
            };
            for (offset, len) in [
                (PRIDE_OF_THE_FLEET, 8),
                (OWNED_STATES, 16),
                (CONTROLLED_STATES, 16),
                (DIVISIONS, 16),
            ] {
                hint(offset, len);
            }
            if rosters.is_some() {
                hint(CORE_STATES, 16);
                hint(CHARACTERS, 8);
            }
            if effects.is_some() {
                hint(rosters::NEIGHBORS, 16);
                hint(PRODUCTION_STATUS, 8);
            }
            if !identified {
                identify()?;
            }
            let pride = read::<u64>(memory, at(PRIDE_OF_THE_FLEET)?)
                .ok_or("country pride of the fleet unreadable")?;
            let states = Array::read(memory, at(OWNED_STATES)?, 8)
                .ok_or("country owned-state array unreadable or invalid")?;
            let controlled = Array::read(memory, at(CONTROLLED_STATES)?, 8)
                .ok_or("country controlled-state array unreadable or invalid")?;
            let roster = Array::read(memory, at(DIVISIONS)?, 8)
                .ok_or("country division roster unreadable or invalid")?;
            let first_division = match roster.element(0, 8) {
                None => 0,
                Some(slot) => match read::<u64>(memory, slot)
                    .ok_or("country first division pointer unreadable")?
                {
                    0 => 0,
                    army => {
                        let army = usize::try_from(army)
                            .map_err(|_| "country first division address overflow")?;
                        let fields = read::<[u8; cunit::HANDLE + 8]>(memory, army)
                            .ok_or("country first division unreadable")?;
                        if engine_sites::at(base, carmy::VTABLE_RVA)
                            != Some(u64::from_le_bytes(fields[..8].try_into().unwrap()) as usize)
                        {
                            return Err("country first division is not an army");
                        }
                        u64::from_le_bytes(fields[cunit::HANDLE..].try_into().unwrap())
                    }
                },
            };
            let mut roster_reason = None;
            let captured_rosters = if let Some(rosters) = rosters.as_deref_mut() {
                match rosters.read(memory, base, country, [states, controlled], fits) {
                    Ok(spans) => Some(spans),
                    Err(reason) => {
                        roster_reason = Some(reason);
                        None
                    }
                }
            } else {
                None
            };
            let mut neighbor_reason = None;
            let mut policy_reason = None;
            let (neighbors, mio_policies) = if let Some(effects) = effects.as_deref_mut() {
                let neighbors = effects
                    .read_neighbors(memory, country, fits)
                    .inspect_err(|reason| neighbor_reason = Some(*reason))
                    .ok();
                let policies = effects
                    .read_policies(memory, base, country, fits)
                    .inspect_err(|reason| policy_reason = Some(*reason))
                    .ok();
                (neighbors, policies)
            } else {
                (None, None)
            };
            if !fits {
                unread_rows = unread_rows
                    .checked_add(1)
                    .ok_or("country counter unread row count overflow")?;
                continue;
            }
            let mut merged =
                [engine_native_memory::modifier_cache::Value::default(); COUNTER_MODIFIER_KEYS.len()];
            let merged = engine_native_parts::modifier_accumulator::lookup_many(
                memory,
                country.pointer,
                &COUNTER_MODIFIER_KEYS,
                &mut merged,
            )
            .ok()
            .map(|()| merged.map(|value| value.present.then_some(value.raw)));
            let attack_modifiers = merged
                .as_ref()
                .map(|merged| counter_entries(merged, &ATTACK_MODIFIER_KEYS));
            let movement_modifiers = merged
                .as_ref()
                .map(|merged| counter_entries(merged, &MOVEMENT_MODIFIER_KEYS));
            let air_modifiers = merged
                .as_ref()
                .map(|merged| counter_entries(merged, &AIR_MODIFIER_KEYS));
            output[len] = CounterInputsRow {
                country: country.id,
                attack_modifiers,
                movement_modifiers,
                recruitment_modifiers: merged
                    .as_ref()
                    .map(|values| counter_entries(values, &RECRUITMENT_MODIFIER_KEYS)),
                air_modifiers,
                air_combat_modifiers: merged
                    .as_ref()
                    .map(|merged| counter_entries(merged, &AIR_COMBAT_MODIFIER_KEYS)),
                mines_effect_reduction: merged.as_ref().map(|merged| {
                    counter_entries(
                        merged,
                        &[engine_executable::modifier_keys::NAVAL_MINES_EFFECT_REDUCTION],
                    )[0]
                }),
                naval_detection: merged.as_ref().map(|merged| {
                    counter_entries(merged, &[engine_executable::modifier_keys::NAVAL_DETECTION])[0]
                }),
                pride_of_the_fleet: pride,
                owned_state_count: states.count as i32,
                controlled_state_count: controlled.count as i32,
                first_division,
                state_rosters: captured_rosters.map(|(states, _, _)| states),
                leader_rosters: captured_rosters.map(|(_, leaders, _)| leaders),
                sent_expeditionaries: captured_rosters.and_then(|(_, _, sent)| sent),
                roster_reason,
                neighbors,
                mio_policies,
                neighbor_reason,
                policy_reason,
            };
            len += 1;
        }
        if !found_country {
            return Err("country table has no readable countries");
        }
        Ok(if unread_rows == 0 {
            Outcome::Rows { len }
        } else {
            Outcome::Truncated { len, unread_rows }
        })
    })();
    result.unwrap_or_else(|reason| Outcome::Unavailable { reason })
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CivilWarRow {
    pub country: u32,
    pub ordinal: u32,
    pub target: u32,
}

pub fn count_civil_wars(memory: &impl Memory, base: usize, game_state: usize) -> Outcome {
    walk_civil_wars_buffer(memory, base, game_state, engine_native_memory::walk::Buffer::Count)
}

pub fn walk_civil_wars(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut Vec<CivilWarRow>,
) -> Outcome {
    walk_civil_wars_buffer(
        memory,
        base,
        game_state,
        engine_native_memory::walk::Buffer::Growing(output),
    )
}

fn walk_civil_wars_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut output: engine_native_memory::walk::Buffer<'_, CivilWarRow>,
) -> Outcome {
    let result = (|| {
        let reader = Reader::new(memory, base, game_state)?;
        let expected = engine_sites::at(base, VTABLE_RVA)
            .ok_or("country civil war module address overflow")?;
        let mut len = 0_usize;
        let mut unread_rows = 0_u32;
        let mut found_country = false;
        for slot in 1..reader.count() {
            let Some(country) = reader.at(slot)? else {
                continue;
            };
            found_country = true;
            let at = |offset| {
                country
                    .pointer
                    .checked_add(offset)
                    .ok_or("country civil war address overflow")
            };
            if read::<u64>(memory, country.pointer) != Some(expected as u64)
                || read::<u32>(memory, at(ID)?) != Some(country.id)
            {
                return Err("country civil war identity changed");
            }
            let targets = Array::read(memory, at(CIVIL_WAR_TARGETS)?, 4)
                .ok_or("country civil war array unreadable or invalid")?;
            let count = targets.count as usize;
            if count == 0 {
                continue;
            }
            output.ensure(
                len.checked_add(count)
                    .ok_or("country civil war row demand overflow")?,
            )?;
            if unread_rows != 0 || count > output.len() - len {
                unread_rows = unread_rows
                    .checked_add(targets.count)
                    .ok_or("country civil war unread row count overflow")?;
                continue;
            }
            let mut words = [0_u8; 64];
            let mut first = 0;
            while first < count {
                let batch = (count - first).min(words.len() / 4);
                let address = targets
                    .element(first as u32, 4)
                    .ok_or("country civil war address overflow")?;
                if !engine_native_memory::read_slice(memory, address, &mut words[..batch * 4]) {
                    return Err("country civil war targets unreadable");
                }
                for (offset, target) in words[..batch * 4].chunks_exact(4).enumerate() {
                    output[len] = CivilWarRow {
                        country: country.id,
                        ordinal: (first + offset) as u32,
                        target: u32::from_le_bytes(target.try_into().unwrap()),
                    };
                    len += 1;
                }
                first += batch;
            }
        }
        if !found_country {
            return Err("country table has no readable countries");
        }
        Ok(if unread_rows == 0 {
            Outcome::Rows { len }
        } else {
            Outcome::Truncated { len, unread_rows }
        })
    })();
    result.unwrap_or_else(|reason| Outcome::Unavailable { reason })
}

#[cfg(any(test, feature = "test-support"))]
#[doc(hidden)]
pub mod synthetic {
    use super::*;
    use engine_native_memory::memory::SliceMemory;

    pub struct World {
        pub bytes: Vec<u8>,
        pub module: usize,
        objects: usize,
        object_size: usize,
    }

    pub const START: usize = 0x10_0000;

    impl World {
        pub fn new(module: usize, countries: u32, object_size: usize) -> Self {
            let slots = countries as usize + 1;
            let object_size = object_size.max(ID + 4);
            let table = START + 0x1000;
            let links = table + slots * 8;
            let objects = (links + slots * 4 + 0xf) & !0xf;
            let mut world = Self {
                bytes: vec![0; objects + slots * object_size - START],
                module,
                objects,
                object_size,
            };
            world.array(START + cgame_state::COUNTRIES, table, slots as u32);
            world.array(START + cgame_state::COUNTRY_LINKS, links, slots as u32);
            for slot in 1..slots {
                let object = world.object(slot as u32);
                world.u64(table + slot * 8, object as u64);
                world.u32(links + slot * 4, slot as u32);
                world.u64(object, (module + VTABLE_RVA.0) as u64);
                world.u32(object + ID, slot as u32);
            }
            world
        }
        pub fn game_state(&self) -> usize {
            START
        }
        pub fn object(&self, slot: u32) -> usize {
            self.objects + slot as usize * self.object_size
        }
        pub fn alloc(&mut self, size: usize) -> usize {
            let address = (START + self.bytes.len() + 0xf) & !0xf;
            self.bytes.resize(address + size - START, 0);
            address
        }
        pub fn put(&mut self, address: usize, value: &[u8]) {
            self.bytes[address - START..address - START + value.len()].copy_from_slice(value);
        }
        pub fn u32(&mut self, address: usize, value: u32) {
            self.put(address, &value.to_le_bytes());
        }
        pub fn u64(&mut self, address: usize, value: u64) {
            self.put(address, &value.to_le_bytes());
        }
        pub fn array(&mut self, address: usize, data: usize, count: u32) {
            self.u64(address, data as u64);
            self.u32(address + 8, count);
            self.u32(address + 12, count);
        }
        pub fn memory(&self) -> SliceMemory<'_> {
            SliceMemory {
                base: START,
                bytes: &self.bytes,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use engine_native_memory::country_identity::MODIFIER_ACCUMULATOR;
    use super::*;
    use engine_native_memory::memory::SliceMemory;

    #[test]
    fn manpower_one_census_answers_attack_movement_and_recruitment_entries() {
        let mut union: Vec<u32> = MOVEMENT_MODIFIER_KEYS
            .iter()
            .chain(&ATTACK_MODIFIER_KEYS)
            .chain(&RECRUITMENT_MODIFIER_KEYS)
            .chain(&AIR_MODIFIER_KEYS)
            .chain(&AIR_COMBAT_MODIFIER_KEYS)
            .chain(&[
                engine_executable::modifier_keys::NAVAL_MINES_EFFECT_REDUCTION,
                engine_executable::modifier_keys::NAVAL_DETECTION,
            ])
            .copied()
            .collect();
        union.sort_unstable();
        union.dedup();
        assert_eq!(union, COUNTER_MODIFIER_KEYS);
        let values = COUNTER_MODIFIER_KEYS.map(|key| Some(i64::from(key)));
        assert_eq!(
            counter_entries(&values, &ATTACK_MODIFIER_KEYS),
            ATTACK_MODIFIER_KEYS.map(|key| Some(i64::from(key)))
        );
        assert_eq!(
            counter_entries(&values, &MOVEMENT_MODIFIER_KEYS),
            MOVEMENT_MODIFIER_KEYS.map(|key| Some(i64::from(key)))
        );
        assert_eq!(
            counter_entries(&values, &AIR_MODIFIER_KEYS),
            AIR_MODIFIER_KEYS.map(|key| Some(i64::from(key)))
        );
        assert_eq!(
            counter_entries(&values, &AIR_COMBAT_MODIFIER_KEYS),
            AIR_COMBAT_MODIFIER_KEYS.map(|key| Some(i64::from(key)))
        );
    }

    #[test]
    fn air_projection_reuses_the_complete_counter_accumulator_read() {
        use engine_native_parts::{modifier_accumulator, modifier_entry};
        use engine_native_memory::read_census::ReadCensus;
        let country = 0x100;
        let data = 0x1000;
        let mut bytes = vec![0_u8; 0x2000];
        let mut keys = COUNTER_MODIFIER_KEYS.to_vec();
        keys.push(0x400);
        put(
            &mut bytes,
            country + MODIFIER_ACCUMULATOR,
            &(data as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            country + MODIFIER_ACCUMULATOR + modifier_accumulator::CAPACITY,
            &(keys.len() as u32).to_le_bytes(),
        );
        put(
            &mut bytes,
            country + MODIFIER_ACCUMULATOR + modifier_accumulator::COUNT,
            &(keys.len() as u32).to_le_bytes(),
        );
        for (index, &key) in keys.iter().enumerate() {
            let at = data + index * modifier_entry::STRIDE;
            put(&mut bytes, at + modifier_entry::KEY, &key.to_le_bytes());
            put(
                &mut bytes,
                at + modifier_entry::VALUE,
                &(-i64::from(key)).to_le_bytes(),
            );
        }
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let census = ReadCensus::new(&memory);
        let mut old_keys: Vec<_> = MOVEMENT_MODIFIER_KEYS
            .into_iter()
            .chain(ATTACK_MODIFIER_KEYS)
            .collect();
        old_keys.sort_unstable();
        let mut old = vec![engine_native_memory::modifier_cache::Value::default(); old_keys.len()];
        modifier_accumulator::lookup_many(&census, country, &old_keys, &mut old).unwrap();
        let old_cost = (census.calls(), census.bytes());
        census.reset();
        let mut all = [engine_native_memory::modifier_cache::Value::default(); COUNTER_MODIFIER_KEYS.len()];
        modifier_accumulator::lookup_many(&census, country, &COUNTER_MODIFIER_KEYS, &mut all)
            .unwrap();
        assert_eq!((census.calls(), census.bytes()), old_cost);
        let raw = all.map(|entry| entry.present.then_some(entry.raw));
        assert_eq!(
            counter_entries(&raw, &AIR_COMBAT_MODIFIER_KEYS),
            AIR_COMBAT_MODIFIER_KEYS.map(|key| Some(-i64::from(key)))
        );
        assert_eq!(
            counter_entries(&raw, &AIR_MODIFIER_KEYS),
            AIR_MODIFIER_KEYS.map(|key| Some(-i64::from(key)))
        );
        assert_eq!(
            counter_entries(&raw, &[engine_executable::modifier_keys::NAVAL_MINES_EFFECT_REDUCTION]),
            [Some(-i64::from(
                engine_executable::modifier_keys::NAVAL_MINES_EFFECT_REDUCTION
            ))]
        );
        assert_eq!(
            counter_entries(&raw, &[engine_executable::modifier_keys::NAVAL_DETECTION]),
            [Some(-i64::from(engine_executable::modifier_keys::NAVAL_DETECTION))]
        );
        for (at, key) in old_keys.into_iter().enumerate() {
            assert_eq!(
                all[COUNTER_MODIFIER_KEYS.binary_search(&key).unwrap()],
                old[at]
            );
        }
    }

    struct ReservedMemory<'a> {
        bytes: &'a [u8],
    }

    impl Memory for ReservedMemory<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if address == engine_native_parts::ccountry_database::SINGLETON_RVA.0 && output.len() == 8 {
                output.copy_from_slice(&0xe000_u64.to_le_bytes());
                true
            } else {
                SliceMemory {
                    base: 0,
                    bytes: self.bytes,
                }
                .copy(address, output)
            }
        }
    }

    fn reserved_fixture() -> Vec<u8> {
        let mut bytes = appearance_fixture();
        put(&mut bytes, 0x2000, &0x11000_u64.to_le_bytes());
        put(&mut bytes, 0x11000, &(VTABLE_RVA.0 as u64).to_le_bytes());
        short_string(&mut bytes, 0x11000 + COSMETIC_TAG, b"MOD_ZERO");
        movement_test_array(&mut bytes, 0xe000, 0xc000, 4);
        put(
            &mut bytes,
            0xe000 + engine_native_parts::ccountry_database::DYNAMIC_TAG_BOUNDARY,
            &1_u32.to_le_bytes(),
        );
        short_string(&mut bytes, 0xc000, b"ZER");
        bytes
    }

    #[test]
    fn reserved_observation_retains_only_the_exact_selected_failure() {
        struct Observed<'a> {
            bytes: &'a [u8],
            fault: Option<usize>,
            calls: std::cell::RefCell<Vec<(usize, usize)>>,
        }
        impl Memory for Observed<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                self.calls.borrow_mut().push((address, output.len()));
                if self.fault == Some(address) {
                    return false;
                }
                ReservedMemory { bytes: self.bytes }.copy(address, output)
            }
        }
        for (fault, reason, selected) in [
            (0x2000, "reserved country slot absent or unreadable", false),
            (0x11000, "reserved country object identity mismatch", true),
            (0x11000 + ID, "reserved country id is not zero", true),
        ] {
            let bytes = reserved_fixture();
            let memory = Observed {
                bytes: &bytes,
                fault: Some(fault),
                calls: std::cell::RefCell::new(Vec::new()),
            };
            let root = read_reserved_observation(&memory, 0, 0x1000);
            assert_eq!(root.result(), Err(reason));
            assert!(!format!("{root:?}").contains("pointer"));
            let calls = memory.calls.borrow().clone();
            assert_eq!(
                calls
                    .iter()
                    .filter(|(address, _)| *address == fault)
                    .count(),
                1
            );
            assert_eq!(
                root.physical_alias(0x11000),
                selected.then_some(Err(reason))
            );
            assert_eq!(root.physical_alias(0x4000), None);
            assert_eq!(root.physical_alias(0), None);
            assert_eq!(
                *memory.calls.borrow(),
                calls,
                "alias decision read native memory"
            );
        }
        let mut bytes = reserved_fixture();
        put(&mut bytes, 0x11000 + ID, &9_u32.to_le_bytes());
        let root = read_reserved_observation(&ReservedMemory { bytes: &bytes }, 0, 0x1000);
        assert_eq!(
            root.physical_alias(0x11000),
            Some(Err("reserved country id is not zero"))
        );
        put(&mut bytes, 0x2000, &0_u64.to_le_bytes());
        let root = read_reserved_observation(&ReservedMemory { bytes: &bytes }, 0, 0x1000);
        assert_eq!(root.result(), Err("reserved country slot is null"));
        assert_eq!(root.physical_alias(0), None);
        assert_eq!(root.physical_alias(0x11000), None);

        let legacy = ReservedCountryObservation::from_result(Err("legacy root fault"));
        assert_eq!(legacy.physical_alias(0x11000), None);
        let legacy = ReservedCountryObservation::from_result(Ok(Country {
            pointer: 0x11000,
            canonical_slot: 0,
            id: 0,
        }));
        assert_eq!(legacy.physical_alias(0x11000), Some(Ok(())));
    }

    #[test]
    fn reserved_snapshot_context_clear_and_short_capture_retire_selection() {
        let mut bytes = reserved_fixture();
        put(&mut bytes, 0x11000, &1_u64.to_le_bytes());
        let root = read_reserved_observation(&ReservedMemory { bytes: &bytes }, 0, 0x1000);
        let mut snapshot = RosterSnapshot::with_capacity(0);
        snapshot.capture_reserved_observation(0, 0x1000, root);
        assert_eq!(snapshot.reserved(0, 0x1000), Some(root.result()));
        assert_eq!(
            snapshot
                .reserved_observation(0, 0x1000)
                .unwrap()
                .physical_alias(0x11000),
            Some(Err("reserved country object identity mismatch"))
        );
        assert_eq!(snapshot.reserved_observation(1, 0x1000), None);
        assert_eq!(snapshot.reserved_observation(0, 0x2000), None);
        let slots = [None; 2];
        let arrays = [
            Array {
                data: 0x2000,
                count: 2,
            },
            Array {
                data: 0x3000,
                count: 2,
            },
        ];
        assert!(!snapshot.capture(AuthenticatedRoster::new(&slots, arrays), 0, 0x1000));
        assert_eq!(snapshot.reserved_observation(0, 0x1000), None);
        snapshot.capture_reserved_observation(0, 0x1000, root);
        snapshot.clear();
        assert_eq!(snapshot.reserved_observation(0, 0x1000), None);
        assert!(snapshot.roster(0, 0x1000).is_none());
        snapshot.capture_reserved(0, 0x1000, Err("legacy snapshot fault"));
        assert_eq!(
            snapshot.reserved(0, 0x1000),
            Some(Err("legacy snapshot fault"))
        );
        assert_eq!(
            snapshot
                .reserved_observation(0, 0x1000)
                .unwrap()
                .physical_alias(0x11000),
            None
        );
    }

    #[test]
    fn reserved_naming_authenticates_only_slot_zero_and_preserves_unbuilt_strings() {
        let mut bytes = reserved_fixture();
        let mut names = vec![0; 64];
        let read = read_reserved_naming_observed_inner(&ReservedMemory { bytes: &bytes }, 0, 0x1000, engine_native_memory::walk::Buffer::Growing(&mut names), None);
        assert_eq!(read.outcome, Outcome::Rows { len: 1 });
        assert_eq!(read.row.built, 0);
        assert_eq!(read.row.group, None);
        assert_eq!(read.row.autonomy, None);
        assert_eq!(&names[..read.row.tag.len], b"ZER");
        assert_eq!(read.row.original_tag.unwrap().len, 0);
        assert_eq!(read.row.original_tag_failure, None);
        assert_eq!(
            read_reserved_naming_demand(&ReservedMemory { bytes: &bytes }, 0, 0x1000),
            Ok(11)
        );
        assert_eq!(
            read_appearance(
                &ReservedMemory { bytes: &bytes },
                0,
                Country {
                    pointer: 0x11000,
                    canonical_slot: 0,
                    id: 0,
                },
                naming(&bytes)
            ),
            Err("country appearance identity changed")
        );
        put(&mut bytes, 0x11000 + ID, &1_u32.to_le_bytes());
        assert_eq!(
            read_reserved_naming_observed_inner(&ReservedMemory { bytes: &bytes }, 0, 0x1000, engine_native_memory::walk::Buffer::Growing(&mut names), None).outcome,
            Outcome::Unavailable {
                reason: "reserved country id is not zero"
            }
        );
        put(&mut bytes, 0x11000 + ID, &0_u32.to_le_bytes());
        put(&mut bytes, 0x11000, &0_u64.to_le_bytes());
        assert_eq!(
            read_reserved_naming_observed_inner(&ReservedMemory { bytes: &bytes }, 0, 0x1000, engine_native_memory::walk::Buffer::Growing(&mut names), None).outcome,
            Outcome::Unavailable {
                reason: "reserved country object identity mismatch"
            }
        );
    }

    #[test]
    fn reserved_owned_state_count_is_signed_and_independent_of_naming_reads() {
        for count in [i32::MIN, -1, 0, 7, i32::MAX] {
            let mut bytes = reserved_fixture();
            put(
                &mut bytes,
                0x11000 + OWNED_STATE_COUNT,
                &count.to_le_bytes(),
            );
            let mut names = vec![0; 11];
            let captured =
                read_reserved_naming_observed_inner(&ReservedMemory { bytes: &bytes }, 0, 0x1000, engine_native_memory::walk::Buffer::Growing(&mut names), None);
            assert_eq!(captured.outcome, Outcome::Rows { len: 1 });
            assert_eq!(captured.row.owned_state_count, Some(count));
            assert_eq!(captured.row.owned_state_count_failure, None);
            assert_eq!(&names[..captured.row.tag.len], b"ZER");
            assert_eq!(captured.row.original_tag_reference, 0);
            assert_eq!(captured.row.original_tag.unwrap().len, 0);
        }
        struct CountFault<'a> {
            bytes: &'a [u8],
        }
        impl Memory for CountFault<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                if address == 0x11000 + OWNED_STATE_COUNT && output.len() == 4 {
                    false
                } else {
                    ReservedMemory { bytes: self.bytes }.copy(address, output)
                }
            }
        }
        let bytes = reserved_fixture();
        let mut names = vec![0; 11];
        let captured = read_reserved_naming_observed_inner(&CountFault { bytes: &bytes }, 0, 0x1000, engine_native_memory::walk::Buffer::Growing(&mut names), None);
        assert_eq!(captured.outcome, Outcome::Rows { len: 1 });
        assert_eq!(captured.row.owned_state_count, None);
        assert_eq!(
            captured.row.owned_state_count_failure,
            Some("reserved owned state count unreadable")
        );
        assert_eq!(captured.row.original_tag_failure, None);
        assert_eq!(captured.row.original_tag.unwrap().len, 0);
        assert_eq!(captured.row.original_tag_reference, 0);
        assert_eq!(&names[..captured.row.tag.len], b"ZER");
        assert_eq!((captured.bytes_len, captured.required_bytes), (11, 11));
    }

    #[test]
    fn reserved_visibility_controls_survive_prefix_shortage_without_retrying_scalars() {
        const DIPLOMACY: usize = 0x15000;
        let mut bytes = reserved_fixture();
        bytes.resize(0x16000, 0);
        put(
            &mut bytes,
            0x11000 + DIPLOMACY_STATUS,
            &(DIPLOMACY as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            DIPLOMACY,
            &(engine_native_parts::cdiplomacy_status_layout::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            DIPLOMACY + engine_native_parts::cdiplomacy_status_layout::COUNTRY,
            &0x11000_u64.to_le_bytes(),
        );
        struct Once<'a> {
            bytes: &'a [u8],
            at: usize,
            attempts: std::cell::Cell<usize>,
        }
        impl Memory for Once<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                if address <= self.at && self.at - address < output.len() {
                    let attempts = self.attempts.get();
                    self.attempts.set(attempts + 1);
                    if attempts == 0 {
                        return false;
                    }
                }
                ReservedMemory { bytes: self.bytes }.copy(address, output)
            }
        }
        for (at, reason, prefix_valid) in [
            (
                0x11000 + DIPLOMACY_STATUS,
                "reserved naming pointer unreadable",
                false,
            ),
            (DIPLOMACY, "reserved naming object identity mismatch", false),
            (
                DIPLOMACY + engine_native_parts::cdiplomacy_status_layout::COUNTRY,
                "reserved faction diplomacy backlink unreadable",
                true,
            ),
            (
                DIPLOMACY + engine_native_parts::cdiplomacy_status_layout::FACTION,
                "reserved faction pointer unreadable",
                true,
            ),
        ] {
            for capacity in [0, 11] {
                let memory = Once {
                    bytes: &bytes,
                    at,
                    attempts: std::cell::Cell::new(0),
                };
                let mut names = Vec::new();
                let captured = read_reserved_naming_observed_inner(
                    &memory,
                    0,
                    0x1000,
                    if capacity == 0 {
                        engine_native_memory::walk::Buffer::Count
                    } else {
                        engine_native_memory::walk::Buffer::Growing(&mut names)
                    },
                    None,
                );
                let observed = captured.visibility.unwrap();
                assert_eq!(observed.faction(), Err(reason));
                assert_eq!(observed.faction(), Err(reason));
                if prefix_valid && capacity == 11 {
                    assert_eq!(captured.outcome, Outcome::Rows { len: 1 });
                    assert_eq!(captured.row.faction, None);
                    assert_eq!(captured.row.faction_failure, Some(reason));
                } else if prefix_valid {
                    assert_eq!(
                        captured.outcome,
                        Outcome::Truncated {
                            len: 0,
                            unread_rows: 1
                        }
                    );
                } else {
                    assert_eq!(captured.outcome, Outcome::Unavailable { reason });
                }
                assert_eq!(memory.attempts.get(), 1);
                assert_eq!(captured.launch_states.as_ref().unwrap().count(), Ok(0));
            }
        }
    }

    #[test]
    fn reserved_visibility_success_remains_private_when_required_names_fail() {
        const DIPLOMACY: usize = 0x15000;
        let mut bytes = reserved_fixture();
        bytes.resize(0x16000, 0);
        put(
            &mut bytes,
            0x11000 + DIPLOMACY_STATUS,
            &(DIPLOMACY as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            DIPLOMACY,
            &(engine_native_parts::cdiplomacy_status_layout::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            DIPLOMACY + engine_native_parts::cdiplomacy_status_layout::COUNTRY,
            &0x11000_u64.to_le_bytes(),
        );
        let captured = read_reserved_naming_observed_inner(&ReservedMemory { bytes: &bytes }, 0, 0x1000, engine_native_memory::walk::Buffer::Count, None);
        assert_eq!(
            captured.outcome,
            Outcome::Truncated {
                len: 0,
                unread_rows: 1
            }
        );
        assert_eq!(captured.row, ReservedNamingRow::default());
        let observed = captured.visibility.unwrap();
        assert_eq!(observed.root().unwrap().id, 0);
        assert_eq!(observed.diplomacy(), Ok(DIPLOMACY));
        assert_eq!(observed.faction(), Ok(0));
        assert_eq!(captured.launch_states.as_ref().unwrap().count(), Ok(0));
    }

    #[test]
    fn reserved_faction_distinguishes_null_full_handle_and_independent_fault() {
        const DIPLOMACY: usize = 0x15000;
        const FACTION: usize = 0x18000;
        let mut bytes = reserved_fixture();
        bytes.resize(FACTION + 0x100, 0);
        put(
            &mut bytes,
            0x11000 + DIPLOMACY_STATUS,
            &(DIPLOMACY as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            DIPLOMACY,
            &(engine_native_parts::cdiplomacy_status_layout::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            DIPLOMACY + engine_native_parts::cdiplomacy_status_layout::COUNTRY,
            &0x11000_u64.to_le_bytes(),
        );
        put(
            &mut bytes,
            FACTION,
            &(crate::cfaction::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        let handle = (u64::from(u32::MAX) << 32) | u64::from(crate::cfaction::REFERENCE_TYPE);
        put(
            &mut bytes,
            FACTION + crate::cfaction::REFERENCE,
            &handle.to_le_bytes(),
        );
        for (pointer, expected) in [(0_u64, 0), (FACTION as u64, handle)] {
            put(
                &mut bytes,
                DIPLOMACY + engine_native_parts::cdiplomacy_status_layout::FACTION,
                &pointer.to_le_bytes(),
            );
            let mut names = vec![0; 11];
            let captured =
                read_reserved_naming_observed_inner(&ReservedMemory { bytes: &bytes }, 0, 0x1000, engine_native_memory::walk::Buffer::Growing(&mut names), None);
            assert_eq!(captured.outcome, Outcome::Rows { len: 1 });
            assert_eq!(captured.row.faction, Some(expected));
            assert_eq!(captured.row.faction_failure, None);
        }
        struct FactionFault<'a>(&'a [u8]);
        impl Memory for FactionFault<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                address != DIPLOMACY + engine_native_parts::cdiplomacy_status_layout::FACTION
                    && ReservedMemory { bytes: self.0 }.copy(address, output)
            }
        }
        let mut names = vec![0; 11];
        let captured = read_reserved_naming_observed_inner(&FactionFault(&bytes), 0, 0x1000, engine_native_memory::walk::Buffer::Growing(&mut names), None);
        assert_eq!(captured.outcome, Outcome::Rows { len: 1 });
        assert_eq!(captured.row.faction, None);
        assert_eq!(
            captured.row.faction_failure,
            Some("reserved faction pointer unreadable")
        );
        assert_eq!(captured.row.owned_state_count, Some(0));
        assert_eq!(captured.row.original_tag_failure, None);
        assert_eq!(&names[..captured.row.tag.len], b"ZER");
    }

    #[test]
    fn reserved_original_tag_ignores_generator_gate_and_reuses_prefixes() {
        for original in [i32::MIN, 0, 2] {
            for (boundary, dynamic) in [
                (0_u32, true),
                (1, false),
                (i32::MIN as u32, true),
                (i32::MAX as u32, false),
            ] {
                let mut bytes = reserved_fixture();
                put(&mut bytes, 0x11000 + ORIGINAL_TAG, &original.to_le_bytes());
                put(
                    &mut bytes,
                    0xe000 + engine_native_parts::ccountry_database::DYNAMIC_TAG_BOUNDARY,
                    &boundary.to_le_bytes(),
                );
                let mut names = vec![0; 64];
                let read =
                    read_reserved_naming_observed_inner(&ReservedMemory { bytes: &bytes }, 0, 0x1000, engine_native_memory::walk::Buffer::Growing(&mut names), None);
                assert_eq!(read.outcome, Outcome::Rows { len: 1 });
                let selected = read.row.original_tag.unwrap();
                assert_eq!(
                    &names[selected.start..selected.start + selected.len],
                    if original > 0 { &b"GER"[..] } else { &b""[..] }
                );
                assert_eq!(read.row.original_tag_failure, None);
                assert_eq!(read.row.original_tag_reference, original);
                assert_eq!(read.row.dynamic_tag_boundary, boundary as i32);
                if original <= 0 {
                    assert_eq!(selected.len, 0);
                    assert_eq!(read.required_bytes, 11);
                } else if dynamic {
                    assert_eq!(selected, read.row.name_tag);
                    assert_eq!(read.required_bytes, 14);
                } else {
                    assert_eq!(read.row.name_tag.len, 0);
                    assert_eq!(selected.start, 11);
                    assert_eq!(read.required_bytes, 14);
                }
                assert_eq!(
                    read_reserved_naming_demand(&ReservedMemory { bytes: &bytes }, 0, 0x1000),
                    Ok(read.required_bytes)
                );
            }
        }
        let mut bytes = reserved_fixture();
        put(&mut bytes, 0x11000 + ORIGINAL_TAG, &2_i32.to_le_bytes());
        bytes[0xc000 + 2 * STRIDE..0xc000 + 3 * STRIDE].fill(0);
        short_string(&mut bytes, 0xc000 + 2 * STRIDE, b"");
        let mut names = vec![0; 11];
        for boundary in [0_u32, 1, i32::MIN as u32, i32::MAX as u32] {
            put(
                &mut bytes,
                0xe000 + engine_native_parts::ccountry_database::DYNAMIC_TAG_BOUNDARY,
                &boundary.to_le_bytes(),
            );
            let read =
                read_reserved_naming_observed_inner(&ReservedMemory { bytes: &bytes }, 0, 0x1000, engine_native_memory::walk::Buffer::Growing(&mut names), None);
            assert_eq!(read.outcome, Outcome::Rows { len: 1 });
            assert_eq!(read.row.original_tag.unwrap().len, 0);
            assert_eq!(read.row.original_tag_reference, 2);
            assert_eq!(read.row.dynamic_tag_boundary, boundary as i32);
            assert_eq!(read.row.original_tag_failure, None);
            assert_eq!(read.row.name_tag.len, 0);
            assert_eq!(
                &names[read.row.tag.start..read.row.tag.start + read.row.tag.len],
                b"ZER"
            );
        }
    }

    #[test]
    fn reserved_original_tag_refusals_preserve_prefix_and_live_demand() {
        let mut bytes = reserved_fixture();
        bytes.resize(0x15000, 0);
        put(&mut bytes, 0x11000 + ORIGINAL_TAG, &2_i32.to_le_bytes());
        put(&mut bytes, 0xc000 + 2 * STRIDE, &0x14000_u64.to_le_bytes());
        put(
            &mut bytes,
            0xc000 + 2 * STRIDE + 16,
            &2048_u64.to_le_bytes(),
        );
        put(
            &mut bytes,
            0xc000 + 2 * STRIDE + 24,
            &2048_u64.to_le_bytes(),
        );
        bytes[0x14000..0x14800].fill(0xff);
        let memory = ReservedMemory { bytes: &bytes };
        assert_eq!(read_reserved_naming_demand(&memory, 0, 0x1000), Ok(2059));
        let mut names = Vec::new();
        let full = read_reserved_naming_observed_inner(&memory, 0, 0x1000, engine_native_memory::walk::Buffer::Growing(&mut names), None);
        assert_eq!(full.outcome, Outcome::Rows { len: 1 });
        assert_eq!(full.bytes_len, 2059);
        let tag = full.row.original_tag.unwrap();
        assert_eq!(&names[tag.start..tag.start + tag.len], &[0xff; 2048]);

        struct Fault<'a> {
            bytes: &'a [u8],
            address: usize,
        }
        impl Memory for Fault<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                address != self.address
                    && ReservedMemory { bytes: self.bytes }.copy(address, output)
            }
        }
        for (address, reason, demand) in [
            (0xc000 + 2 * STRIDE, "string header unreadable", 11),
            (0x14000, "string bytes unreadable", 2059),
        ] {
            let memory = Fault {
                bytes: &bytes,
                address,
            };
            assert_eq!(read_reserved_naming_demand(&memory, 0, 0x1000), Ok(demand));
            let failed = read_reserved_naming_observed_inner(&memory, 0, 0x1000, engine_native_memory::walk::Buffer::Growing(&mut names), None);
            assert_eq!(failed.outcome, Outcome::Rows { len: 1 });
            assert_eq!(failed.bytes_len, 11);
            assert_eq!(failed.required_bytes, demand);
            assert_eq!(failed.row.original_tag, None);
            assert_eq!(failed.row.original_tag_failure, Some(reason));
            assert_eq!(&names[..3], b"ZER");
            assert_eq!(&names[3..11], b"MOD_ZERO");
        }
        put(&mut bytes, 0x11000 + ORIGINAL_TAG, &i32::MAX.to_le_bytes());
        let read = read_reserved_naming_observed_inner(&ReservedMemory { bytes: &bytes }, 0, 0x1000, engine_native_memory::walk::Buffer::Growing(&mut names), None);
        assert_eq!(read.outcome, Outcome::Rows { len: 1 });
        assert_eq!(read.row.original_tag, None);
        assert_eq!(
            read.row.original_tag_failure,
            Some("country original tag outside live tag strings")
        );
        assert_eq!(
            read_reserved_naming_demand(&ReservedMemory { bytes: &bytes }, 0, 0x1000),
            Ok(11)
        );
    }

    #[test]
    fn appearance_original_tags_preserve_presence_outside_generator_gate() {
        for (boundary, empty) in [
            (1_u32, false),
            (2, true),
            (i32::MIN as u32, false),
            (i32::MAX as u32, true),
        ] {
            let bytes = appearance_fixture();
            let memory = SliceMemory {
                base: 0,
                bytes: &bytes,
            };
            let mut rows = vec![AppearanceRow::default(); 2];
            assert_eq!(
                walk_appearance_observed_inner(&memory, 0, 0x1000, boundary, engine_native_memory::walk::Buffer::Growing(&mut rows), None),
                Outcome::Rows { len: 2 }
            );
            let mut names = vec![0; 3];
            let (mut used, mut required) = (0, 0);
            capture_appearance_original_tags(
                &memory,
                0x1000,
                &mut rows,
                &mut names,
                &mut used,
                &mut required,
            );
            assert_eq!(rows[0].original_tag.unwrap().len, 3);
            assert_eq!(&names, b"GER");
            assert_eq!(rows[1].original_tag.unwrap().len, 0);
            assert_eq!((used, required), (3, 3));
            assert_eq!(rows[0].name_tag.as_bytes().is_empty(), empty);
        }
        let mut bytes = appearance_fixture();
        short_string(&mut bytes, 0xc000 + STRIDE, b"MOD_OWN");
        put(&mut bytes, 0x3000 + ORIGINAL_TAG, &1_i32.to_le_bytes());
        let mut rows = vec![AppearanceRow::default(); 2];
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        walk_appearance_observed_inner(&memory, 0, 0x1000, 3, engine_native_memory::walk::Buffer::Growing(&mut rows), None);
        let mut names = vec![0; 7];
        let (mut used, mut required) = (0, 0);
        capture_appearance_original_tags(
            &memory,
            0x1000,
            &mut rows,
            &mut names,
            &mut used,
            &mut required,
        );
        assert_eq!(&names, b"MOD_OWN");
        assert_eq!(rows[0].original_tag.unwrap().len, 7);
        for boundary in [1_u32, 3] {
            bytes[0xc000 + STRIDE..0xc000 + 2 * STRIDE].fill(0);
            short_string(&mut bytes, 0xc000 + STRIDE, b"");
            let memory = SliceMemory {
                base: 0,
                bytes: &bytes,
            };
            assert_eq!(
                walk_appearance_observed_inner(&memory, 0, 0x1000, boundary, engine_native_memory::walk::Buffer::Growing(&mut rows), None),
                Outcome::Rows { len: 2 }
            );
            capture_appearance_original_tags(
                &memory,
                0x1000,
                &mut rows,
                &mut names,
                &mut used,
                &mut required,
            );
            assert_eq!(rows[0].original_tag.unwrap().len, 0);
            assert_eq!(rows[0].original_tag_reference, 1);
            assert_eq!(rows[0].original_tag_failure, None);
            assert_eq!(rows[1].original_tag.unwrap().len, 0);
        }
    }

    #[test]
    fn appearance_original_tag_reference_preserves_signed_word_on_byte_refusal() {
        for original in [i32::MIN, -1, 0, 2, i32::MAX] {
            let mut bytes = appearance_fixture();
            put(&mut bytes, 0x3000 + ORIGINAL_TAG, &original.to_le_bytes());
            let memory = SliceMemory {
                base: 0,
                bytes: &bytes,
            };
            let mut rows = vec![AppearanceRow::default(); 2];
            assert_eq!(
                walk_appearance_observed_inner(&memory, 0, 0x1000, 3, engine_native_memory::walk::Buffer::Growing(&mut rows), None),
                Outcome::Rows { len: 2 }
            );
            let mut names = vec![0; 3];
            let (mut used, mut required) = (0, 0);
            capture_appearance_original_tags(
                &memory,
                0x1000,
                &mut rows,
                &mut names,
                &mut used,
                &mut required,
            );
            assert_eq!(rows[0].original_tag_reference, original);
            if original == i32::MAX {
                assert_eq!(rows[0].original_tag, None);
                assert_eq!(
                    rows[0].original_tag_failure,
                    Some("country original tag outside live tag strings")
                );
            } else {
                assert_eq!(rows[0].original_tag_failure, None, "original={original}");
                assert_eq!(
                    rows[0].original_tag.unwrap().len,
                    if original > 0 { 3 } else { 0 }
                );
            }
            assert_eq!(rows[1].original_tag.unwrap().len, 0);
            assert_eq!(rows[0].name_tag.as_bytes(), b"");
        }
    }

    #[test]
    fn appearance_original_tag_capacity_and_faults_preserve_siblings_and_demand_floor() {
        let mut bytes = reserved_fixture();
        bytes.resize(0x15000, 0);
        put(&mut bytes, 0x5000 + ORIGINAL_TAG, &3_i32.to_le_bytes());
        put(&mut bytes, 0xc000 + 3 * STRIDE, &0x14000_u64.to_le_bytes());
        put(
            &mut bytes,
            0xc000 + 3 * STRIDE + 16,
            &2048_u64.to_le_bytes(),
        );
        put(
            &mut bytes,
            0xc000 + 3 * STRIDE + 24,
            &2048_u64.to_le_bytes(),
        );
        bytes[0x14000..0x14800].fill(0xff);
        let mut rows = vec![AppearanceRow::default(); 2];
        let memory = ReservedMemory { bytes: &bytes };
        assert_eq!(
            walk_appearance_observed_inner(&memory, 0, 0x1000, 3, engine_native_memory::walk::Buffer::Growing(&mut rows), None),
            Outcome::Rows { len: 2 }
        );
        assert_eq!(read_country_naming_demand(&memory, 0, 0x1000), Ok(2062));
        let mut names = Vec::new();
        let (mut used, mut required) = (0, 0);
        capture_appearance_original_tags(
            &memory,
            0x1000,
            &mut rows,
            &mut names,
            &mut used,
            &mut required,
        );
        assert_eq!((used, required), (2051, 2051));
        assert_eq!(&names[3..], &[0xff; 2048]);
        struct Fault<'a>(&'a [u8]);
        impl Memory for Fault<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                address != 0xc000 + 2 * STRIDE
                    && ReservedMemory { bytes: self.0 }.copy(address, output)
            }
        }
        let (mut used, mut required) = (0, 0);
        capture_appearance_original_tags(
            &Fault(&bytes),
            0x1000,
            &mut rows,
            &mut names,
            &mut used,
            &mut required,
        );
        assert_eq!(
            rows[0].original_tag_failure,
            Some("string header unreadable")
        );
        assert_eq!(rows[1].original_tag.unwrap().start, 0);
        assert_eq!((used, required), (2048, 2048));
        put(&mut bytes, 0x11000 + ID, &1_u32.to_le_bytes());
        assert_eq!(
            read_country_naming_demand(&ReservedMemory { bytes: &bytes }, 0, 0x1000),
            Ok(2051)
        );
        put(&mut bytes, 0x5000, &0_u64.to_le_bytes());
        assert_eq!(
            read_country_naming_demand(&ReservedMemory { bytes: &bytes }, 0, 0x1000),
            Ok(3)
        );
    }

    #[test]
    fn reserved_naming_uses_live_byte_demand_and_counts_shortages_without_truncation() {
        let mut bytes = reserved_fixture();
        bytes.resize(0x15000, 0);
        bytes[0x14000..0x14800].fill(0xff);
        put(
            &mut bytes,
            0x11000 + COSMETIC_TAG,
            &0x14000_u64.to_le_bytes(),
        );
        put(
            &mut bytes,
            0x11000 + COSMETIC_TAG + 16,
            &2048_u64.to_le_bytes(),
        );
        put(
            &mut bytes,
            0x11000 + COSMETIC_TAG + 24,
            &2048_u64.to_le_bytes(),
        );
        for (offset, len) in [(NAME, 1_u64), (ADJECTIVE, 0), (DEFINITE_NAME, 9)] {
            put(&mut bytes, 0x11000 + offset + 16, &len.to_le_bytes());
        }
        let mut names = Vec::new();
        let read = read_reserved_naming_observed_inner(&ReservedMemory { bytes: &bytes }, 0, 0x1000, engine_native_memory::walk::Buffer::Growing(&mut names), None);
        assert_eq!(read.outcome, Outcome::Rows { len: 1 });
        assert_eq!(read.row.built, 3);
        assert_eq!(read.row.cosmetic_tag.len, 2048);
        assert_eq!(
            &names[read.row.cosmetic_tag.start..read.bytes_len],
            &[0xff; 2048]
        );
        struct PayloadFault<'a>(&'a [u8]);
        impl Memory for PayloadFault<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                if address == 0x14000 && output.len() == 2048 {
                    false
                } else {
                    ReservedMemory { bytes: self.0 }.copy(address, output)
                }
            }
        }
        let failed = read_reserved_naming_observed_inner(&PayloadFault(&bytes), 0, 0x1000, engine_native_memory::walk::Buffer::Growing(&mut names), None);
        assert_eq!(
            failed.outcome,
            Outcome::Unavailable {
                reason: "string bytes unreadable"
            }
        );
        assert_eq!(failed.required_bytes, 2051);
        assert_eq!(failed.bytes_len, 0);
        put(&mut bytes, 0x14800, &[1]);
        assert_eq!(
            read_reserved_naming_observed_inner(&ReservedMemory { bytes: &bytes }, 0, 0x1000, engine_native_memory::walk::Buffer::Growing(&mut names), None).outcome,
            Outcome::Unavailable {
                reason: "string terminator unreadable or invalid"
            }
        );
    }

    #[test]
    fn reserved_naming_records_ruling_group_and_current_autonomy_native_absence() {
        let mut bytes = reserved_fixture();
        put(
            &mut bytes,
            0x11000 + POLITICAL_STATUS,
            &0xa000_u64.to_le_bytes(),
        );
        put(
            &mut bytes,
            0xa400 + political_party_layout::IDEOLOGY_GROUP,
            &0xb000_u64.to_le_bytes(),
        );
        put(
            &mut bytes,
            0xb000,
            &(engine_native_parts::cideology_group::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        short_string(
            &mut bytes,
            0xb000 + engine_native_parts::cideology_group::TOKEN,
            b"mod_group",
        );
        put(
            &mut bytes,
            0x11000 + DIPLOMACY_STATUS,
            &0x8000_u64.to_le_bytes(),
        );
        put(
            &mut bytes,
            0x8000 + engine_native_parts::cdiplomacy_status_layout::CURRENT_AUTONOMY_STATUS,
            &0xb100_u64.to_le_bytes(),
        );
        put(
            &mut bytes,
            0xb100,
            &(engine_native_parts::ccurrent_autonomy_status_layout::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            0xb100 + engine_native_parts::ccurrent_autonomy_status_layout::SUBJECT_COUNTRY,
            &0x11000_u64.to_le_bytes(),
        );
        put(
            &mut bytes,
            0xb100 + engine_native_parts::ccurrent_autonomy_status_layout::STATE,
            &0xb200_u64.to_le_bytes(),
        );
        short_string(
            &mut bytes,
            0xb200 + engine_native_parts::autonomy_state_definition::NAME,
            b"mod_subject",
        );
        let mut names = vec![0; 128];
        let read = read_reserved_naming_observed_inner(&ReservedMemory { bytes: &bytes }, 0, 0x1000, engine_native_memory::walk::Buffer::Growing(&mut names), None);
        assert_eq!(read.outcome, Outcome::Rows { len: 1 });
        let text = |name: crate::crelation::subject::Name| &names[name.start..name.start + name.len];
        assert_eq!(text(read.row.leader_ideology), b"despotism");
        assert_eq!(text(read.row.group.unwrap()), b"mod_group");
        assert_eq!(text(read.row.autonomy.unwrap()), b"mod_subject");
        assert_eq!(read.row.overlord, 1);
        put(
            &mut bytes,
            0xb100 + engine_native_parts::ccurrent_autonomy_status_layout::STATE,
            &0_u64.to_le_bytes(),
        );
        assert_eq!(
            read_reserved_naming_observed_inner(&ReservedMemory { bytes: &bytes }, 0, 0x1000, engine_native_memory::walk::Buffer::Growing(&mut names), None).outcome,
            Outcome::Unavailable {
                reason: "reserved autonomy definition is null"
            }
        );
    }

    fn short_string(bytes: &mut [u8], at: usize, text: &[u8]) {
        put(bytes, at, text);
        put(bytes, at + 16, &(text.len() as u64).to_le_bytes());
        put(bytes, at + 24, &15_u64.to_le_bytes());
    }

    fn appearance_fixture() -> Vec<u8> {
        let mut bytes = movement_fixture();
        for country in [0x3000, 0x5000] {
            put(
                &mut bytes,
                country + COSMETIC_TAG + 24,
                &15_u64.to_le_bytes(),
            );
            put(
                &mut bytes,
                country + GRAPHICAL_CULTURE + 24,
                &15_u64.to_le_bytes(),
            );
            put(
                &mut bytes,
                country + GRAPHICAL_CULTURE_2D + 24,
                &15_u64.to_le_bytes(),
            );
        }
        short_string(&mut bytes, 0x3000 + GRAPHICAL_CULTURE, b"western_gfx");
        short_string(&mut bytes, 0x5000 + GRAPHICAL_CULTURE_2D, b"eastern_2d");
        short_string(&mut bytes, 0x3000 + OLD_NAME, b"Old Reich");
        short_string(&mut bytes, 0x3000 + OLD_ADJECTIVE, b"");
        short_string(&mut bytes, 0x3000 + OLD_DEFINITE_NAME, b"the Old Reich");
        movement_test_array(&mut bytes, 0x1000 + cgame_state::COUNTRY_TAGS, 0xc000, 4);
        put(
            &mut bytes,
            0x8000 + engine_native_parts::cdiplomacy_status_layout::OVERLORD,
            &1_i32.to_le_bytes(),
        );
        short_string(&mut bytes, 0xc000 + 2 * STRIDE, b"GER");
        put(&mut bytes, 0x3000 + ORIGINAL_TAG, &2_i32.to_le_bytes());
        put(
            &mut bytes,
            0x3000 + POLITICAL_STATUS,
            &0xa000_u64.to_le_bytes(),
        );
        for (object, vtable) in [
            (0xa000, political_status_layout::VTABLE_RVA.0),
            (0xa400, political_party_layout::VTABLE_RVA.0),
            (0xa900, country_leader_layout::VTABLE_RVA.0),
        ] {
            put(&mut bytes, object, &(vtable as u64).to_le_bytes());
        }
        put(
            &mut bytes,
            0xa000 + political_status_layout::RULING_PARTY,
            &0xa400_u64.to_le_bytes(),
        );
        movement_test_array(
            &mut bytes,
            0xa400 + political_party_layout::COUNTRY_LEADERS,
            0xa800,
            1,
        );
        put(&mut bytes, 0xa800, &0xa900_u64.to_le_bytes());
        put(
            &mut bytes,
            0xa900 + LEADER_IDEOLOGY,
            &0xaa00_u64.to_le_bytes(),
        );
        short_string(&mut bytes, 0xaa00 + IDEOLOGY_TOKEN, b"despotism");
        put(
            &mut bytes,
            0xa900 + country_leader_layout::CHARACTER,
            &0xab00_u64.to_le_bytes(),
        );
        put(
            &mut bytes,
            0xab00,
            &(ccharacter::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            0xab00 + ccharacter::TYPE,
            &ccharacter::REFERENCE_TYPE.to_le_bytes(),
        );
        put(&mut bytes, 0xab00 + ccharacter::ID, &777_u32.to_le_bytes());
        bytes
    }

    fn naming(bytes: &[u8]) -> NamingContext {
        NamingContext {
            tags: Array::read(
                &SliceMemory { base: 0, bytes },
                0x1000 + cgame_state::COUNTRY_TAGS,
                STRIDE,
            )
            .unwrap(),
            dynamic_boundary: 1,
        }
    }

    #[test]
    fn worker_appearance_keeps_mod_sized_cosmetic_tag_cultures_and_old_names() {
        let mut bytes = appearance_fixture();
        let country = Country {
            pointer: 0x3000,
            id: 1,
            canonical_slot: 1,
        };
        for len in [513, 4097] {
            for address in [
                0x3000 + COSMETIC_TAG,
                0x3000 + GRAPHICAL_CULTURE,
                0x3000 + GRAPHICAL_CULTURE_2D,
                0x3000 + OLD_NAME,
                0x3000 + OLD_ADJECTIVE,
                0x3000 + OLD_DEFINITE_NAME,
                0xc000 + 2 * STRIDE,
            ] {
                let payload = bytes.len();
                bytes.resize(payload + len + 1, 0);
                bytes[payload..payload + len].fill(0xff);
                put(&mut bytes, address, &(payload as u64).to_le_bytes());
                put(&mut bytes, address + 16, &(len as u64).to_le_bytes());
                put(&mut bytes, address + 24, &(len as u64).to_le_bytes());
            }
            let memory = SliceMemory {
                base: 0,
                bytes: &bytes,
            };
            assert!(read_appearance(&memory, 0, country, naming(&bytes)).is_err());
            let row = read_appearance_observed_inner::<
                engine_native_memory::capture_text::Owned,
                engine_native_memory::capture_text::Owned,
                engine_native_memory::capture_text::Owned,
            >(&memory, 0, country, naming(&bytes), None)
            .unwrap();
            assert_eq!(row.cosmetic_tag.as_bytes(), vec![0xff; len]);
            assert_eq!(row.name_tag.as_bytes(), vec![0xff; len]);
            assert_eq!(row.graphical_culture.as_bytes(), vec![0xff; len]);
            assert_eq!(row.graphical_culture_2d.as_bytes(), vec![0xff; len]);
            assert!(
                row.old_names
                    .as_ref()
                    .unwrap()
                    .iter()
                    .all(|name| name.as_bytes() == vec![0xff; len])
            );
            assert_eq!(row.leader_ideology.as_bytes(), b"despotism");
        }
    }

    #[test]
    fn appearance_preserves_float_bits_exact_cosmetic_bytes_and_canonical_census() {
        let mut bytes = appearance_fixture();
        let rgba = [0x7fc0_4321_u32, 0xff80_0000, 0x8000_0000, 0xffff_ffff];
        let ui_rgba = [0x3f80_0000_u32, 0x3f00_0000, 0x7f80_0001, 0x0000_0001];
        for (index, (bits, ui_bits)) in rgba.iter().zip(&ui_rgba).enumerate() {
            put(
                &mut bytes,
                0x3000 + CURRENT_PRIMARY_COLOR + index * 4,
                &bits.to_le_bytes(),
            );
            put(
                &mut bytes,
                0x3000 + CURRENT_UI_COLOR + index * 4,
                &ui_bits.to_le_bytes(),
            );
        }
        put(&mut bytes, 0x3000 + COSMETIC_TAG, b"BRM_british\0");
        put(
            &mut bytes,
            0x3000 + COSMETIC_TAG + 16,
            &11_u64.to_le_bytes(),
        );
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let mut output: Vec<AppearanceRow> = vec![AppearanceRow::default(); 3];
        assert_eq!(
            walk_appearance_observed_inner(&memory, 0, 0x1000, 1, engine_native_memory::walk::Buffer::Growing(&mut output), None),
            Outcome::Rows { len: 2 }
        );
        assert_eq!(output[0].country, 1);
        assert_eq!(output[0].rgba, rgba);
        assert_eq!(output[0].ui_rgba, ui_rgba);
        assert_eq!(output[0].cosmetic_tag.as_bytes(), b"BRM_british");
        assert_eq!(output[0].name_tag.as_bytes(), b"GER");
        assert_eq!(output[0].leader_ideology.as_bytes(), b"despotism");
        assert_eq!(output[0].leader_character, Some(777));
        assert_eq!(output[0].graphical_culture.as_bytes(), b"western_gfx");
        assert!(output[0].graphical_culture_2d.as_bytes().is_empty());
        assert_eq!(
            output[0]
                .old_names
                .map(|names| names.map(|name| name.as_bytes().to_vec())),
            Some([b"Old Reich".to_vec(), Vec::new(), b"the Old Reich".to_vec()])
        );
        assert_eq!(output[1].old_names, None);
        assert_eq!(output[1].country, 2);
        assert!(output[1].graphical_culture.as_bytes().is_empty());
        assert_eq!(output[1].graphical_culture_2d.as_bytes(), b"eastern_2d");
        assert_eq!(output[1].overlord, 1);
        assert!(output[1].cosmetic_tag.as_bytes().is_empty());
        assert!(output[1].name_tag.as_bytes().is_empty());
        assert!(output[1].leader_ideology.as_bytes().is_empty());
        assert_eq!(output[1].leader_character, None);
        let mut static_output: Vec<AppearanceRow> = vec![AppearanceRow::default(); 2];
        walk_appearance_observed_inner(&memory, 0, 0x1000, 2, engine_native_memory::walk::Buffer::Growing(&mut static_output), None);
        assert!(static_output[0].name_tag.as_bytes().is_empty());
        let census = engine_native_memory::read_census::ReadCensus::new(&memory);
        assert_eq!(
            read_appearance(
                &census,
                0,
                Country {
                    pointer: 0x3000,
                    id: 1,
                    canonical_slot: 1,
                },
                naming(&bytes)
            ),
            Ok(output[0])
        );
        assert_eq!(census.calls(), 34 + 11 + 3);
    }

    #[test]
    fn leader_character_is_missing_not_zero_and_never_withholds_the_row() {
        let mut bytes = appearance_fixture();
        let country = Country {
            pointer: 0x3000,
            id: 1,
            canonical_slot: 1,
        };
        let context = naming(&bytes);
        let run = |bytes: &[u8]| {
            read_appearance(&SliceMemory { base: 0, bytes }, 0, country, context)
                .map(|row| (row.leader_ideology, row.leader_character))
        };
        let despotism = run(&bytes).unwrap().0;
        assert_eq!(despotism.as_bytes(), b"despotism");
        assert_eq!(run(&bytes), Ok((despotism, Some(777))));
        put(&mut bytes, 0xab00 + ccharacter::ID, &0_u32.to_le_bytes());
        assert_eq!(run(&bytes), Ok((despotism, Some(0))));
        put(&mut bytes, 0xab00 + ccharacter::ID, &777_u32.to_le_bytes());
        let reference = 0xab00 + ccharacter::TYPE;
        put(
            &mut bytes,
            reference,
            &(ccharacter::REFERENCE_TYPE + 1).to_le_bytes(),
        );
        assert_eq!(run(&bytes), Ok((despotism, None)));
        put(
            &mut bytes,
            reference,
            &ccharacter::REFERENCE_TYPE.to_le_bytes(),
        );
        put(&mut bytes, 0xab00, &1_u64.to_le_bytes());
        assert_eq!(run(&bytes), Ok((despotism, None)));
        put(
            &mut bytes,
            0xab00,
            &(ccharacter::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        let parent = 0xa900 + country_leader_layout::CHARACTER;
        put(&mut bytes, parent, &0_u64.to_le_bytes());
        assert_eq!(run(&bytes), Ok((despotism, None)));
        put(&mut bytes, parent, &0xffff_0000_u64.to_le_bytes());
        assert_eq!(run(&bytes), Ok((despotism, None)));
        put(&mut bytes, parent, &0xab00_u64.to_le_bytes());
        assert_eq!(run(&bytes), Ok((despotism, Some(777))));
        let count = 0xa400 + political_party_layout::COUNTRY_LEADER_COUNT;
        put(&mut bytes, count, &0_u32.to_le_bytes());
        assert_eq!(run(&bytes), Ok((Text::default(), None)));
        put(&mut bytes, count, &1_u32.to_le_bytes());
        let ruling = 0xa000 + political_status_layout::RULING_PARTY;
        put(&mut bytes, ruling, &0_u64.to_le_bytes());
        assert_eq!(run(&bytes), Ok((Text::default(), None)));
        put(&mut bytes, ruling, &0xa400_u64.to_le_bytes());
        put(&mut bytes, 0x3000 + POLITICAL_STATUS, &0_u64.to_le_bytes());
        assert_eq!(run(&bytes), Ok((Text::default(), None)));
        put(
            &mut bytes,
            0x3000 + POLITICAL_STATUS,
            &0xa000_u64.to_le_bytes(),
        );
        put(&mut bytes, 0xa900, &0_u64.to_le_bytes());
        assert_eq!(run(&bytes), Err("country naming object identity mismatch"));
    }

    #[test]
    fn appearance_heap_bound_and_malformed_string_never_publish_shortened_tag() {
        let mut bytes = appearance_fixture();
        let country = Country {
            pointer: 0x3000,
            id: 1,
            canonical_slot: 1,
        };
        let string = country.pointer + COSMETIC_TAG;
        put(&mut bytes, string, &0xb000_u64.to_le_bytes());
        put(&mut bytes, string + 16, &256_u64.to_le_bytes());
        put(&mut bytes, string + 24, &300_u64.to_le_bytes());
        bytes[0xb000..0xb100].fill(0xff);
        let context = naming(&bytes);
        let run =
            |bytes: &[u8]| read_appearance(&SliceMemory { base: 0, bytes }, 0, country, context);
        assert_eq!(run(&bytes).unwrap().cosmetic_tag.as_bytes(), &[0xff; 256]);
        put(&mut bytes, string + 16, &257_u64.to_le_bytes());
        assert!(run(&bytes).is_err());
        put(&mut bytes, string + 16, &256_u64.to_le_bytes());
        bytes[0xb100] = 1;
        assert!(run(&bytes).is_err());
        bytes[0xb100] = 0;
        put(&mut bytes, string + 24, &255_u64.to_le_bytes());
        assert!(run(&bytes).is_err());
    }

    #[test]
    fn appearance_refuses_changed_country_identity_and_unreadable_colour() {
        let mut bytes = appearance_fixture();
        let country = Country {
            pointer: 0x3000,
            id: 1,
            canonical_slot: 1,
        };
        let context = naming(&bytes);
        let run =
            |bytes: &[u8]| read_appearance(&SliceMemory { base: 0, bytes }, 0, country, context);
        put(&mut bytes, 0x3000 + ID, &2_u32.to_le_bytes());
        assert_eq!(run(&bytes), Err("country appearance identity changed"));
        put(&mut bytes, 0x3000 + ID, &1_u32.to_le_bytes());
        assert_eq!(
            run(&bytes[..0x337f]),
            Err("country appearance colour unreadable")
        );
        assert_eq!(
            run(&bytes[..0x3000 + CURRENT_UI_COLOR + 15]),
            Err("country appearance colour unreadable")
        );
        put(&mut bytes, 0x3000, &0_u64.to_le_bytes());
        assert_eq!(run(&bytes), Err("country appearance identity changed"));
    }

    #[test]
    fn appearance_all_null_country_census_is_unavailable() {
        let mut bytes = appearance_fixture();
        for slot in 1..4 {
            put(&mut bytes, 0x2000 + slot * 8, &0_u64.to_le_bytes());
        }
        assert_eq!(
            walk_appearance_observed_inner(&SliceMemory {
                    base: 0,
                    bytes: &bytes
                }, 0, 0x1000, 1, engine_native_memory::walk::Buffer::<AppearanceRow>::Count, None),
            Outcome::Unavailable {
                reason: "country table has no readable countries"
            }
        );
    }

    fn movement_fixture() -> Vec<u8> {
        let mut bytes = vec![0_u8; 0x14000];
        movement_test_array(&mut bytes, 0x1000 + cgame_state::COUNTRIES, 0x2000, 4);
        movement_test_array(&mut bytes, 0x1000 + cgame_state::COUNTRY_LINKS, 0x2100, 4);
        for (slot, country) in [(1, 0x3000), (2, 0x5000), (3, 0x5000)] {
            put(
                &mut bytes,
                0x2000 + slot * 8,
                &(country as u64).to_le_bytes(),
            );
            put(&mut bytes, country, &(VTABLE_RVA.0 as u64).to_le_bytes());
        }
        for (slot, target) in [(1, 1_u32), (2, 2), (3, 2)] {
            put(&mut bytes, 0x2100 + slot * 4, &target.to_le_bytes());
        }
        put(&mut bytes, 0x3000 + ID, &1_u32.to_le_bytes());
        put(&mut bytes, 0x5000 + ID, &2_u32.to_le_bytes());
        for (country, diplomacy, matrix) in [(0x3000, 0x7000, 0x7800), (0x5000, 0x8000, 0x8800)] {
            put(
                &mut bytes,
                country + DIPLOMACY_STATUS,
                &(diplomacy as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                diplomacy,
                &(engine_native_parts::cdiplomacy_status_layout::VTABLE_RVA.0 as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                diplomacy + engine_native_parts::cdiplomacy_status_layout::COUNTRY,
                &(country as u64).to_le_bytes(),
            );
            movement_test_array(
                &mut bytes,
                diplomacy + engine_native_parts::cdiplomacy_status_layout::RELATIONS,
                matrix,
                4,
            );
        }
        put(&mut bytes, 0x7810, &0x9000_u64.to_le_bytes());
        put(
            &mut bytes,
            0x9000,
            &(crate::crelation::status::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(&mut bytes, 0x9050, &3_u32.to_le_bytes());
        put(&mut bytes, 0x91f0, &0x7000_u64.to_le_bytes());
        bytes
    }

    fn movement_test_array(bytes: &mut [u8], at: usize, data: usize, count: u32) {
        put(bytes, at, &(data as u64).to_le_bytes());
        put(bytes, at + 8, &count.to_le_bytes());
        put(bytes, at + 12, &count.to_le_bytes());
    }

    #[test]
    fn movement_river_relation_resolves_aliases_and_reserved_targets_without_requiring_a_relation()
    {
        use crate::crelation::status as relation;
        use engine_executable::modifier_keys;
        let mut bytes = movement_fixture();
        let run = |bytes: &[u8], target| {
            river_crossing_against(&SliceMemory { base: 0, bytes }, 0, 0x1000, 1, target)
        };
        let accumulator = 0x9000 + relation::MODIFIER_ACCUMULATOR;
        movement_test_array(&mut bytes, accumulator, 0xa000, 1);
        put(
            &mut bytes,
            0xa000,
            &modifier_keys::RIVER_CROSSING_FACTOR_AGAINST.to_le_bytes(),
        );
        put(&mut bytes, 0xa008, &(-27_000_i64).to_le_bytes());
        assert_eq!(run(&bytes, 2), Ok(Some(-27_000)));
        assert_eq!(run(&bytes, 3), Ok(Some(-27_000)));
        put(&mut bytes, 0x7800, &0x9000_u64.to_le_bytes());
        put(&mut bytes, 0x2100 + 3 * 4, &0_u32.to_le_bytes());
        for target in [0, u32::MAX, 3] {
            assert_eq!(run(&bytes, target), Ok(Some(-27_000)));
        }
        put(&mut bytes, 0x7800, &0_u64.to_le_bytes());
        assert_eq!(run(&bytes, 0), Ok(None));
        put(&mut bytes, 0x3000 + DIPLOMACY_STATUS, &0_u64.to_le_bytes());
        assert_eq!(run(&bytes, 2), Ok(None));
        put(
            &mut bytes,
            0x3000 + DIPLOMACY_STATUS,
            &0x7000_u64.to_le_bytes(),
        );
        put(
            &mut bytes,
            0x9000 + relation::DIPLOMACY_STATUS,
            &0_u64.to_le_bytes(),
        );
        assert!(run(&bytes, 2).is_err());
    }

    #[test]
    fn movement_relationship_aliases_war_precedence_and_subject_endpoints() {
        let mut bytes = movement_fixture();
        let run = |bytes: &[u8], owner, target| {
            let memory = SliceMemory { base: 0, bytes };
            (
                movement_friendly(&memory, 0, 0x1000, owner, target),
                movement_enemy(&memory, 0, 0x1000, owner, target),
            )
        };
        assert_eq!(run(&bytes, 3, 2), (Ok(true), Ok(false)));
        assert_eq!(run(&bytes, 1, 3), (Ok(false), Ok(false)));
        put(&mut bytes, 0x92a0, &1_u64.to_le_bytes());
        assert_eq!(run(&bytes, 1, 2), (Ok(true), Ok(false)));
        put(&mut bytes, 0x92e8, &0xa000_u64.to_le_bytes());
        put(
            &mut bytes,
            0xa000,
            &(crate::cwar_relation::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            0xa008,
            &crate::cwar_relation::TYPE.to_le_bytes(),
        );
        assert_eq!(run(&bytes, 1, 3), (Ok(false), Ok(true)));
        bytes[0xa049] = 0x80;
        assert_eq!(run(&bytes, 1, 3), (Ok(true), Ok(false)));
        put(&mut bytes, 0x92a0, &0_u64.to_le_bytes());
        put(&mut bytes, 0x92b0, &1_u64.to_le_bytes());
        assert_eq!(run(&bytes, 1, 3), (Ok(false), Ok(false)));
        put(&mut bytes, 0x9288, &0xb000_u64.to_le_bytes());
        put(
            &mut bytes,
            0xb000,
            &(crate::crelation::subject::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            0xb008,
            &crate::crelation::subject::TYPE_VALUE.to_le_bytes(),
        );
        put(&mut bytes, 0xb010, &1_u32.to_le_bytes());
        put(&mut bytes, 0xb014, &2_u32.to_le_bytes());
        assert_eq!(run(&bytes, 1, 3), (Ok(true), Ok(false)));
        put(&mut bytes, 0x91f0, &0_u64.to_le_bytes());
        assert!(run(&bytes, 1, 3).0.is_err());
        assert!(run(&bytes, 1, 3).1.is_err());
    }

    #[test]
    fn movement_hash_preserves_signed_values_probe_markers_and_faults() {
        let mut bytes = vec![0_u8; 0x1000];
        put(&mut bytes, 0x128, &0x200_u64.to_le_bytes());
        put(&mut bytes, 0x134, &0_i32.to_le_bytes());
        bytes[0x138] = 2;
        bytes[0x204] = 1;
        put(&mut bytes, 0x208, &77_u32.to_le_bytes());
        put(&mut bytes, 0x210, &(-3_i64).to_le_bytes());
        let lookup = |bytes: &[u8], key, wide| {
            movement_hash_value(&SliceMemory { base: 0, bytes }, 0x100, key, wide)
        };
        assert_eq!(lookup(&bytes, 77, true), Ok(Some(-3)));
        assert_eq!(lookup(&bytes, 78, true), Ok(None));
        bytes[0x21c] = 2;
        put(&mut bytes, 0x220, &78_u32.to_le_bytes());
        put(&mut bytes, 0x228, &i64::MAX.to_le_bytes());
        assert_eq!(lookup(&bytes, 78, true), Ok(Some(i64::MAX)));
        assert!(lookup(&bytes[..0x229], 78, true).is_err());
        bytes[0x200] = 1;
        put(&mut bytes, 0x204, &77_u32.to_le_bytes());
        put(&mut bytes, 0x208, &(-1_i32).to_le_bytes());
        assert_eq!(lookup(&bytes, 77, false), Ok(Some(-1)));
        put(&mut bytes, 0x134, &(-1_i32).to_le_bytes());
        assert!(lookup(&bytes, 77, false).is_err());
    }

    #[test]
    fn movement_core_supply_joins_authenticated_country_companion_and_both_hashes() {
        let mut bytes = movement_fixture();
        bytes.resize(0x20000, 0);
        movement_test_array(&mut bytes, 0x1000 + cgame_state::PROVINCES, 0xc000, 2);
        put(&mut bytes, 0xc008, &0xd000_u64.to_le_bytes());
        put(
            &mut bytes,
            0xd000,
            &(crate::cprovince::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(&mut bytes, 0xd0a4, &1_u32.to_le_bytes());
        put(
            &mut bytes,
            0x11000,
            &(engine_native_parts::ccontroller_area::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        movement_test_array(&mut bytes, 0x110b0, 0xc008, 1);
        put(
            &mut bytes,
            0x1000 + cgame_state::SUPPLY_SYSTEM,
            &0x16000_u64.to_le_bytes(),
        );
        put(
            &mut bytes,
            0x16000,
            &(crate::ccountry_supply_system::world::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            0x16008,
            &(crate::ccountry_supply_system::world::SECONDARY_VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        movement_test_array(&mut bytes, 0x16108, 0x17000, 4);
        movement_test_array(&mut bytes, 0x16120, 0x18000, 4);
        put(&mut bytes, 0x17008, &0x1b000_u64.to_le_bytes());
        put(
            &mut bytes,
            0x1b000,
            &(crate::ccountry_supply_system::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            0x1b008,
            &(crate::ccountry_supply_system::SECONDARY_VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(&mut bytes, 0x1b070, &0x16000_u64.to_le_bytes());
        put(&mut bytes, 0x1b078, &0x3000_u64.to_le_bytes());
        let companion = 0x18000 + engine_native_parts::supply_companion::STRIDE;
        put(&mut bytes, 0x1b080, &(companion as u64).to_le_bytes());
        put(
            &mut bytes,
            0x1000 + cgame_state::STRATEGIC_NAVY_MANAGER,
            &0x1c000_u64.to_le_bytes(),
        );
        put(&mut bytes, 0x1c028, &0x1d000_u64.to_le_bytes());
        bytes[0x1c038] = 1;
        bytes[0x1d004] = 1;
        put(&mut bytes, 0x1d008, &1_u32.to_le_bytes());
        put(&mut bytes, 0x1d010, &(-1_i64).to_le_bytes());
        put(&mut bytes, companion + 0x28, &0x1e000_u64.to_le_bytes());
        bytes[companion + 0x38] = 1;
        bytes[0x1e000] = 1;
        put(&mut bytes, 0x1e004, &1_u32.to_le_bytes());
        let access = |bytes: &[u8], area| {
            has_core_supply_access(&SliceMemory { base: 0, bytes }, 0, 0x1000, 1, area)
        };
        assert_eq!(access(&bytes, 0), Ok(true));
        assert_eq!(access(&bytes, 0x11000), Ok(true));
        put(&mut bytes, 0x1e008, &(-1_i32).to_le_bytes());
        assert_eq!(access(&bytes, 0x11000), Ok(false));
        put(&mut bytes, 0x1e008, &2_i32.to_le_bytes());
        bytes[0x1d004] = 0;
        assert_eq!(access(&bytes, 0x11000), Ok(false));
        bytes[0x1d004] = 1;
        assert!(access(&bytes[..0x1e009], 0x11000).is_err());
        put(&mut bytes, 0x1b078, &0x5000_u64.to_le_bytes());
        assert!(access(&bytes, 0x11000).is_err());
    }

    #[test]
    fn core_supply_optional_sharing_preserves_fallback_and_direct_verification() {
        use std::cell::Cell;
        struct Hook<'a> {
            bytes: &'a [u8],
            result: Option<Result<bool, &'static str>>,
            calls: Cell<usize>,
            reads: Cell<usize>,
        }
        impl Memory for Hook<'_> {
            fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
                self.reads.set(self.reads.get() + 1);
                SliceMemory {
                    base: 0,
                    bytes: self.bytes,
                }
                .copy(address, destination)
            }
            fn core_supply_access(
                &self,
                base: usize,
                state: usize,
                owner: u32,
                area: usize,
            ) -> Option<Result<bool, &'static str>> {
                assert_eq!((base, state, owner, area), (0, 0x1000, 1, 0));
                self.calls.set(self.calls.get() + 1);
                self.result
            }
        }
        let bytes = movement_fixture();
        assert_eq!(
            has_core_supply_access_direct(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes
                },
                0,
                0x1000,
                1,
                0
            ),
            Ok(true)
        );
        for result in [
            None,
            Some(Ok(true)),
            Some(Ok(false)),
            Some(Err("supply snapshot fault")),
        ] {
            let memory = Hook {
                bytes: &bytes,
                result,
                calls: Cell::new(0),
                reads: Cell::new(0),
            };
            assert_eq!(
                has_core_supply_access(&memory, 0, 0x1000, 1, 0),
                result.unwrap_or(Ok(true))
            );
            assert_eq!(memory.calls.get(), 1);
            assert_eq!(memory.reads.get() == 0, result.is_some());
            assert_eq!(
                has_core_supply_access_direct(&memory, 0, 0x1000, 1, 0),
                Ok(true)
            );
            assert_eq!(memory.calls.get(), 1);
            assert!(memory.reads.get() > 0);
        }
        assert!(
            has_core_supply_access_direct(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes
                },
                0,
                0x1000,
                0,
                0
            )
            .is_err()
        );
    }

    #[test]
    fn movement_capital_area_uses_largest_controlled_count_and_native_ties() {
        let mut bytes = movement_fixture();
        movement_test_array(&mut bytes, 0x1000 + cgame_state::PROVINCES, 0xc000, 4);
        movement_test_array(&mut bytes, 0x1000 + cgame_state::STATES, 0xc100, 2);
        put(&mut bytes, 0xc108, &0xd000_u64.to_le_bytes());
        put(
            &mut bytes,
            0xd000,
            &(crate::cstate::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(&mut bytes, 0xd058, &1_u32.to_le_bytes());
        movement_test_array(&mut bytes, 0xd018, 0xc200, 3);
        put(&mut bytes, 0x3000 + CAPITAL, &1_i32.to_le_bytes());
        for (id, province, area, count) in [
            (1, 0xe000, 0x11000, 8_i32),
            (2, 0xf000, 0x12000, 8),
            (3, 0x10000, 0x13000, 20),
        ] {
            put(
                &mut bytes,
                0xc000 + id * 8,
                &(province as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                0xc200 + (id - 1) * 8,
                &(province as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                province,
                &(crate::cprovince::VTABLE_RVA.0 as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                province + crate::cprovince::ID,
                &(id as u32).to_le_bytes(),
            );
            put(
                &mut bytes,
                province + crate::cprovince::CONTROLLER,
                &(if id == 3 { 2_u32 } else { 1 }).to_le_bytes(),
            );
            put(
                &mut bytes,
                province + crate::cprovince::CONTROLLER_AREA,
                &(area as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                area,
                &(engine_native_parts::ccontroller_area::VTABLE_RVA.0 as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                area + engine_native_parts::ccontroller_area::PROVINCE_COUNT,
                &count.to_le_bytes(),
            );
        }
        let select = |bytes: &[u8]| {
            let memory = SliceMemory { base: 0, bytes };
            let reader = Reader::new(&memory, 0, 0x1000).unwrap();
            movement_capital_area(
                &memory,
                0,
                0x1000,
                &reader,
                reader.resolve(1).unwrap(),
                movement_array(&memory, 0x1000, cgame_state::PROVINCES).unwrap(),
            )
        };
        assert_eq!(select(&bytes), Ok(0x11000));
        assert_eq!(
            has_core_supply_access(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes
                },
                0,
                0x1000,
                1,
                0x11000
            ),
            Ok(true)
        );
        put(&mut bytes, 0x1203c, &9_i32.to_le_bytes());
        assert_eq!(select(&bytes), Ok(0x12000));
        put(&mut bytes, 0x1203c, &(-1_i32).to_le_bytes());
        assert!(select(&bytes).is_err());
    }

    fn put(bytes: &mut [u8], offset: usize, data: &[u8]) {
        bytes[offset..offset + data.len()].copy_from_slice(data);
    }

    fn volunteer_world(hosts: u32, tallies: u32) -> Vec<u8> {
        const BASE: usize = 0x1000;
        const COUNTRY: usize = 0x3000;
        const HOSTS: usize = 0x5000;
        const TALLIES: usize = 0x6000;
        let mut bytes = vec![0_u8; 0x8000];
        let mut at = |address: usize, data: &[u8]| {
            put(&mut bytes, address - BASE, data);
        };
        at(BASE + cgame_state::COUNTRIES, &0x2000_u64.to_le_bytes());
        at(BASE + cgame_state::COUNTRIES_CAPACITY, &2_u32.to_le_bytes());
        at(BASE + cgame_state::COUNTRIES_COUNT, &2_u32.to_le_bytes());
        at(BASE + cgame_state::COUNTRY_LINKS, &0x2100_u64.to_le_bytes());
        at(
            BASE + cgame_state::COUNTRY_LINKS_CAPACITY,
            &2_u32.to_le_bytes(),
        );
        at(
            BASE + cgame_state::COUNTRY_LINKS_COUNT,
            &2_u32.to_le_bytes(),
        );
        at(0x2008, &(COUNTRY as u64).to_le_bytes());
        at(0x2104, &1_u32.to_le_bytes());
        at(COUNTRY, &(VTABLE_RVA.0 as u64).to_le_bytes());
        at(COUNTRY + ID, &1_u32.to_le_bytes());
        at(COUNTRY + VOLUNTEER_HOSTS, &(HOSTS as u64).to_le_bytes());
        at(COUNTRY + 0x1430, &hosts.to_le_bytes());
        at(COUNTRY + VOLUNTEER_HOSTS_COUNT, &hosts.to_le_bytes());
        at(COUNTRY + VOLUNTEER_TALLIES, &(TALLIES as u64).to_le_bytes());
        at(COUNTRY + 0x1448, &tallies.to_le_bytes());
        at(COUNTRY + VOLUNTEER_TALLY_COUNT, &tallies.to_le_bytes());
        for index in 0..hosts {
            at(HOSTS + index as usize * 4, &(index + 1).to_le_bytes());
        }
        for index in 0..tallies {
            at(
                TALLIES + index as usize * 4,
                &(-(index as i32)).to_le_bytes(),
            );
        }
        bytes
    }

    #[test]
    #[ignore = "unit-scale guarded sent-volunteer reader cost; run release"]
    fn sent_volunteers_unit_scale() {
        use engine_native_memory::memory::{CountedMemory, LiveMemory, ReadCounts};
        let stress = std::env::var("BATTLEPLAN_UNIT_SHAPE").or_else(|_| std::env::var("HOI4_UNIT_SHAPE")).as_deref() == Ok("stress");
        let (countries, divisions) = if stress {
            (5_000, 100_000)
        } else {
            (1_500, 50_000)
        };
        let mut world = synthetic::World::new(0, countries, SENT_VOLUNTEERS + 16);
        let mut pointers = vec![
            synthetic::START + cgame_state::COUNTRIES,
            synthetic::START + cgame_state::COUNTRY_LINKS,
        ];
        for slot in 1..=countries {
            let begin = (slot - 1) as usize * divisions / countries as usize;
            let end = slot as usize * divisions / countries as usize;
            let data = world.alloc((end - begin) * 8);
            world.array(
                world.object(slot) + SENT_VOLUNTEERS,
                data,
                (end - begin) as u32,
            );
            pointers.extend([
                synthetic::START + 0x1000 + slot as usize * 8,
                world.object(slot) + SENT_VOLUNTEERS,
            ]);
            for (ordinal, id) in (begin..end).enumerate() {
                world.u64(data + ordinal * 8, id as u64 + 1);
            }
        }
        let address = world.bytes.as_ptr() as usize;
        for pointer in pointers {
            let offset = pointer - synthetic::START;
            let old = u64::from_le_bytes(world.bytes[offset..offset + 8].try_into().unwrap());
            world.u64(pointer, (address + old as usize - synthetic::START) as u64);
        }
        // SAFETY: the fixture is fully allocated and its pointers relocated; it retains every pointee
        // and stays immutable through the walk.
        let memory = CountedMemory::new(unsafe { LiveMemory::new() });
        let mut rows = vec![SentVolunteerRow::default(); divisions];
        ReadCounts::take_current();
        assert_eq!(
            walk_sent_volunteers_buffer(&memory, 0, address, engine_native_memory::walk::Buffer::Growing(&mut rows)),
            Outcome::Rows { len: divisions }
        );
        let counts = ReadCounts::take_current();
        assert_eq!(rows.last().unwrap().division, divisions as u64);
        let mut times = Vec::new();
        for _ in 0..25 {
            let start = std::time::Instant::now();
            assert_eq!(
                walk_sent_volunteers_buffer(&memory, 0, address, engine_native_memory::walk::Buffer::Growing(&mut rows)),
                Outcome::Rows { len: divisions }
            );
            times.push(start.elapsed().as_secs_f64() * 1000.0);
        }
        times.sort_by(f64::total_cmp);
        println!(
            "SENT_VOLUNTEERS countries={countries} references={divisions} min_ms={:.6} median_ms={:.6} calls={} read_bytes={} output_bytes={} fixture_bytes={}",
            times[0],
            times[12],
            counts.copy_calls + counts.scattered_calls + counts.record_calls + counts.plan_calls,
            counts.bytes,
            rows.capacity() * size_of::<SentVolunteerRow>(),
            world.bytes.len()
        );
    }

    #[test]
    fn sent_volunteers_keep_raw_handles_order_and_demand() {
        let mut bytes = volunteer_world(0, 0);
        let offset = 0x3000 - 0x1000 + SENT_VOLUNTEERS;
        put(&mut bytes, offset, &0x5000_u64.to_le_bytes());
        put(&mut bytes, offset + 8, &65_u32.to_le_bytes());
        put(&mut bytes, offset + 12, &65_u32.to_le_bytes());
        for i in 0..65_u64 {
            put(&mut bytes, 0x4000 + i as usize * 8, &i.to_le_bytes());
        }
        let mut rows = vec![SentVolunteerRow::default(); 65];
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        assert_eq!(
            walk_sent_volunteers_buffer(&memory, 0, 0x1000, engine_native_memory::walk::Buffer::Growing(&mut rows)),
            Outcome::Rows { len: 65 }
        );
        for (i, r) in rows.iter().enumerate() {
            assert_eq!(
                *r,
                SentVolunteerRow {
                    country: 1,
                    ordinal: i as u32,
                    division: i as u64
                }
            );
        }
        let short = SliceMemory {
            base: 0x1000,
            bytes: &bytes[..0x4000 + 64 * 8],
        };
        assert!(matches!(
            walk_sent_volunteers_buffer(&short, 0, 0x1000, engine_native_memory::walk::Buffer::Growing(&mut rows)),
            Outcome::Unavailable { .. }
        ));
        put(&mut bytes, offset + 12, &66_u32.to_le_bytes());
        assert!(matches!(
            walk_sent_volunteers_buffer(&SliceMemory {
                    base: 0x1000,
                    bytes: &bytes
                }, 0, 0x1000, engine_native_memory::walk::Buffer::Growing(&mut rows)),
            Outcome::Unavailable { .. }
        ));
    }

    #[test]
    fn volunteer_air_relation_suffix_keeps_absent_key_separate_from_failed_pair() {
        let mut bytes = volunteer_world(2, 2);
        let mut at = |address: usize, data: &[u8]| put(&mut bytes, address - 0x1000, data);
        at(0x3000 + DIPLOMACY_STATUS, &0x7000_u64.to_le_bytes());
        at(
            0x7000,
            &(engine_native_parts::cdiplomacy_status_layout::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        at(
            0x7000 + engine_native_parts::cdiplomacy_status_layout::COUNTRY,
            &0x3000_u64.to_le_bytes(),
        );
        at(
            0x7000 + engine_native_parts::cdiplomacy_status_layout::RELATIONS,
            &0x8200_u64.to_le_bytes(),
        );
        at(
            0x7000 + engine_native_parts::cdiplomacy_status_layout::RELATION_CAPACITY,
            &2_u32.to_le_bytes(),
        );
        at(
            0x7000 + engine_native_parts::cdiplomacy_status_layout::RELATION_COUNT,
            &2_u32.to_le_bytes(),
        );
        at(0x8208, &0x7400_u64.to_le_bytes());
        at(
            0x7400,
            &(crate::crelation::status::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        at(
            0x7400 + crate::crelation::status::DIPLOMACY_STATUS,
            &0x7000_u64.to_le_bytes(),
        );
        at(
            0x7400 + crate::crelation::status::TARGET_COUNTRY,
            &1_u32.to_le_bytes(),
        );
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        let mut rows = vec![VolunteerHostRow::default(); 2];
        assert_eq!(
            walk_volunteer_hosts_with_air_buffer(
                &memory,
                0,
                0x1000,
                Buffer::Growing(&mut rows),
                true
            ),
            Outcome::Rows { len: 2 }
        );
        assert_eq!(
            (rows[0].air_capacity_modifier, rows[1].air_capacity_modifier),
            (Some(0), None)
        );
        assert_eq!((rows[0].host, rows[1].host), (1, 2));
        assert_eq!(
            (rows[0].deployed_divisions, rows[1].deployed_divisions),
            (0, -1)
        );
    }

    #[test]
    fn volunteer_positions_survive_the_batch_boundary_in_native_order() {
        let bytes = volunteer_world(65, 65);
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        let mut rows = vec![VolunteerHostRow::default(); 128];
        assert_eq!(
            walk_volunteer_hosts_buffer(&memory, 0, 0x1000, engine_native_memory::walk::Buffer::Growing(&mut rows)),
            Outcome::Rows { len: 65 }
        );
        for (position, row) in rows[..65].iter().enumerate() {
            assert_eq!(
                *row,
                VolunteerHostRow {
                    country: 1,
                    position: position as u32,
                    host: position as u32 + 1,
                    deployed_divisions: -(position as i32),
                    air_capacity_modifier: None,
                },
                "position {position}"
            );
        }
    }

    #[test]
    fn a_mid_edit_volunteer_pair_refuses_instead_of_pairing_the_wrong_tally() {
        let bytes = volunteer_world(65, 64);
        assert_eq!(
            walk_volunteer_hosts_buffer(&SliceMemory {
                    base: 0x1000,
                    bytes: &bytes
                }, 0, 0x1000, engine_native_memory::walk::Buffer::Growing(&mut vec![VolunteerHostRow::default(); 128])),
            Outcome::Unavailable {
                reason: "volunteer host and tally vectors have different counts"
            }
        );
    }

    fn countries_with_aliases() -> Vec<u8> {
        let mut bytes = vec![0_u8; 0x4000];
        put(
            &mut bytes,
            cgame_state::COUNTRIES,
            &0x1500_u64.to_le_bytes(),
        );
        put(&mut bytes, cgame_state::COUNTRIES + 8, &3_u32.to_le_bytes());
        put(
            &mut bytes,
            cgame_state::COUNTRIES + 12,
            &3_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            cgame_state::COUNTRY_TAGS,
            &0x1600_u64.to_le_bytes(),
        );
        put(
            &mut bytes,
            cgame_state::COUNTRY_TAGS + 8,
            &8_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            cgame_state::COUNTRY_TAGS + 12,
            &8_u32.to_le_bytes(),
        );
        put(&mut bytes, 0x508, &0x2000_u64.to_le_bytes());
        put(&mut bytes, 0x510, &0x2000_u64.to_le_bytes());
        put(
            &mut bytes,
            cgame_state::COUNTRY_LINKS,
            &0x1700_u64.to_le_bytes(),
        );
        put(
            &mut bytes,
            cgame_state::COUNTRY_LINKS + 8,
            &8_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            cgame_state::COUNTRY_LINKS + 12,
            &8_u32.to_le_bytes(),
        );
        for link in [1, 2, 7] {
            put(&mut bytes, 0x700 + link * 4, &1_u32.to_le_bytes());
        }
        let tag = 0x600 + 7 * STRIDE;
        put(&mut bytes, tag, b"D01");
        put(&mut bytes, tag + 16, &3_u64.to_le_bytes());
        put(&mut bytes, tag + 24, &15_u64.to_le_bytes());
        put(&mut bytes, 0x1000, &(VTABLE_RVA.0 as u64).to_le_bytes());
        put(&mut bytes, 0x1000 + ID, &7_u32.to_le_bytes());
        put(&mut bytes, 0x1000 + NAME + 24, &15_u64.to_le_bytes());
        put(
            &mut bytes,
            0x1000 + COMMAND_POWER,
            &4_294_967_297_i64.to_le_bytes(),
        );
        put(
            &mut bytes,
            0x1000 + STABILITY,
            &4_294_967_298_i64.to_le_bytes(),
        );
        put(
            &mut bytes,
            0x1000 + WAR_SUPPORT,
            &(-4_294_967_299_i64).to_le_bytes(),
        );
        put(
            &mut bytes,
            0x1000 + CONVOY_STATE + cconvoys::CACHE_RAW,
            &4_294_967_300_i64.to_le_bytes(),
        );
        bytes[0x1000 + CONVOY_STATE + cconvoys::CACHE_VALID] = 1;
        put(
            &mut bytes,
            0x1000 + POLITICAL_STATUS,
            &0x4000_u64.to_le_bytes(),
        );
        put(
            &mut bytes,
            0x3000,
            &(political_status_layout::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            0x3000 + political_status_layout::COUNTRY,
            &0x2000_u64.to_le_bytes(),
        );
        put(
            &mut bytes,
            0x3000 + political_status_layout::POLITICAL_POWER,
            &(-4_294_967_301_i64).to_le_bytes(),
        );
        bytes
    }

    #[test]
    fn reader_authenticates_aliases_and_preserves_current_native_id() {
        let mut bytes = countries_with_aliases();
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        let reader = Reader::new(&memory, 0, 0x1000).unwrap();
        let expected = Country {
            pointer: 0x2000,
            canonical_slot: 1,
            id: 7,
        };
        assert_eq!(reader.count(), 3);
        assert_eq!(reader.at(1), Ok(Some(expected)));
        assert_eq!(reader.at(2), Ok(None));
        for raw in [1, 2, 7] {
            assert_eq!(reader.resolve(raw), Ok(expected));
        }
        assert_eq!(reader.at(0), Err("country id is zero"));
        assert_eq!(reader.resolve(0), Err("country id is zero"));
        assert!(reader.at(3).is_err());
        assert!(reader.resolve(8).is_err());
        put(&mut bytes, 0x510, &0_u64.to_le_bytes());
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        let reader = Reader::new(&memory, 0, 0x1000).unwrap();
        assert_eq!(reader.at(2), Ok(None));
        assert_eq!(reader.resolve(2), Ok(expected));
        put(&mut bytes, 0x508, &0_u64.to_le_bytes());
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        let reader = Reader::new(&memory, 0, 0x1000).unwrap();
        assert_eq!(reader.at(1), Ok(None));
        assert_eq!(
            reader.resolve(7),
            Err("country link resolves to null country")
        );
    }

    #[test]
    fn reader_rejects_corrupt_aliases_before_skipping_them() {
        let cases: &[(usize, &[u8], &str)] = &[
            (0x1000, &0_u64.to_le_bytes(), "country vtable mismatch"),
            (0x1000 + ID, &0_u32.to_le_bytes(), "country id is zero"),
            (
                0x708,
                &2_u32.to_le_bytes(),
                "country slot and object resolve to different countries",
            ),
            (
                0x71c,
                &2_u32.to_le_bytes(),
                "country slot and object resolve to different countries",
            ),
            (
                0x508,
                &0_u64.to_le_bytes(),
                "canonical country slot points to a different object",
            ),
            (
                0x704,
                &2_u32.to_le_bytes(),
                "country link does not resolve to its canonical object",
            ),
        ];
        for &(offset, replacement, reason) in cases {
            let mut bytes = countries_with_aliases();
            put(&mut bytes, offset, replacement);
            let memory = SliceMemory {
                base: 0x1000,
                bytes: &bytes,
            };
            let reader = Reader::new(&memory, 0, 0x1000).unwrap();
            assert_eq!(reader.at(2), Err(reason));
        }
        for (offset, count) in [
            (cgame_state::COUNTRIES + 12, 1_u32),
            (cgame_state::COUNTRY_LINKS + 12, 0),
            (cgame_state::COUNTRY_LINKS + 12, 9),
        ] {
            let mut bytes = countries_with_aliases();
            put(&mut bytes, offset, &count.to_le_bytes());
            let memory = SliceMemory {
                base: 0x1000,
                bytes: &bytes,
            };
            assert!(Reader::new(&memory, 0, 0x1000).is_err());
        }
    }

    #[test]
    fn politics_witness_is_independent_of_existing_country_scalar_capture() {
        let mut bytes = countries_with_aliases();
        put(
            &mut bytes,
            0x1000 + SURRENDER_TARGET,
            &0xf001_0001_u32.to_le_bytes(),
        );
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        let mut rows = [Row::default(); 2];
        assert_eq!(
            walk(&memory, 0, 0x1000, &mut rows),
            Outcome::Rows { len: 1 }
        );
        assert_eq!(rows[0].politics_witness, Some((0xf001_0001, None)));
        let expected = rows[0];
        struct UnreadSurrenderTarget<'a>(SliceMemory<'a>);
        impl Memory for UnreadSurrenderTarget<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                if address == 0x2000 + SURRENDER_TARGET && output.len() == 4 {
                    false
                } else {
                    self.0.copy(address, output)
                }
            }
        }
        assert_eq!(
            walk(&UnreadSurrenderTarget(memory), 0, 0x1000, &mut rows),
            Outcome::Rows { len: 1 }
        );
        assert_eq!(rows[0].politics_witness, None);
        assert_eq!(
            rows[0],
            Row {
                politics_witness: None,
                ..expected
            }
        );
    }

    #[test]
    fn country_scalars_are_gathered_in_one_guarded_copy() {
        let bytes = countries_with_aliases();
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        let mut expected = [Row::default(); 2];
        assert_eq!(
            walk(&memory, 0, 0x1000, &mut expected),
            Outcome::Rows { len: 1 }
        );
        let census = engine_native_memory::read_census::ReadCensus::new(&memory);
        let mut rows = [Row::default(); 2];
        assert_eq!(
            walk(&census, 0, 0x1000, &mut rows),
            Outcome::Rows { len: 1 }
        );
        assert_eq!(rows, expected);
        struct Unbatched<'a, M>(&'a M);
        impl<M: Memory> Memory for Unbatched<'_, M> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                self.0.copy(address, output)
            }
            fn copy_scattered<const N: usize>(
                &self,
                addresses: [usize; N],
                destinations: &mut [&mut [u8]; N],
            ) -> usize {
                if N != 8 {
                    return self.0.copy_scattered(addresses, destinations);
                }
                for (index, (address, destination)) in addresses
                    .into_iter()
                    .zip(destinations.iter_mut())
                    .enumerate()
                {
                    if !self.0.copy(address, destination) {
                        return index;
                    }
                }
                N
            }
        }
        let alone = engine_native_memory::read_census::ReadCensus::new(&memory);
        assert_eq!(
            walk(&Unbatched(&alone), 0, 0x1000, &mut rows),
            Outcome::Rows { len: 1 }
        );
        assert_eq!(rows, expected);
        assert_eq!(alone.calls(), census.calls() + 7);
        struct Denied<'a>(SliceMemory<'a>);
        impl Memory for Denied<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                address != 0x2000 + STABILITY && self.0.copy(address, output)
            }
        }
        assert_eq!(
            walk(
                &Denied(SliceMemory {
                    base: 0x1000,
                    bytes: &bytes,
                }),
                0,
                0x1000,
                &mut rows
            ),
            Outcome::Unavailable {
                reason: "country scalar unreadable"
            }
        );
    }

    #[test]
    fn politics_witness_walk_acquires_every_native_country_row() {
        let mut world = synthetic::World::new(0x4000_0000, 65, PEACE_CONFERENCE_PARTICIPANT + 8);
        let tags = world.alloc(66 * STRIDE);
        world.array(world.game_state() + cgame_state::COUNTRY_TAGS, tags, 66);
        for id in 1..=65 {
            let object = world.object(id);
            world.u32(object + SURRENDER_TARGET, 0xf001_0000 + id);
            let tag = tags + id as usize * STRIDE;
            world.put(tag, b"SYN");
            world.u64(tag + 16, 3);
            world.u64(tag + 24, 15);
        }
        let mut rows = vec![Row::default(); 65];
        assert_eq!(
            walk(&world.memory(), world.module, world.game_state(), &mut rows),
            Outcome::Rows { len: 65 }
        );
        for (slot, row) in rows.iter().enumerate() {
            let id = slot as u32 + 1;
            assert_eq!((row.id, row.canonical_slot), (id, id));
            assert_eq!(row.politics_witness, Some((0xf001_0000 + id, None)));
        }
    }

    #[test]
    fn link_tags_and_native_scalars_survive_without_display_rounding() {
        let mut bytes = countries_with_aliases();
        let mut rows = [Row::default(); 2];
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        assert_eq!(
            walk(&memory, 0, 0x1000, &mut rows),
            Outcome::Rows { len: 1 }
        );
        assert_eq!(rows[0].tag.as_bytes(), b"D01");
        assert_eq!(rows[0].id, 7);
        assert_eq!(rows[0].canonical_slot, 1);
        assert_eq!(rows[0].command_power_raw, 4_294_967_297);
        assert_eq!(rows[0].political_power_raw, Some(-4_294_967_301));
        assert_eq!(rows[0].stability_raw, 4_294_967_298);
        assert_eq!(rows[0].war_support_raw, -4_294_967_299);
        assert_eq!(rows[0].convoys_raw, Some(4_294_967_300));
        assert_eq!(rows[0].peace_conference_participant, 0);
        bytes[0x1000 + CONVOY_STATE + cconvoys::CACHE_VALID] = 0;
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        assert_eq!(
            walk(&memory, 0, 0x1000, &mut rows),
            Outcome::Rows { len: 1 }
        );
        assert_eq!(rows[0].convoys_raw, None);
        put(&mut bytes, 0x708, &2_u32.to_le_bytes());
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        assert_eq!(
            walk(&memory, 0, 0x1000, &mut rows),
            Outcome::Unavailable {
                reason: "country slot and object resolve to different countries"
            }
        );
        put(&mut bytes, 0x708, &1_u32.to_le_bytes());
        bytes[0x3000 + political_status_layout::COUNTRY] = 1;
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        assert_eq!(
            walk(&memory, 0, 0x1000, &mut rows),
            Outcome::Unavailable {
                reason: "political status country mismatch"
            }
        );
    }

    #[test]
    fn links_preserve_raw_aliases_and_count_rows_beyond_capacity() {
        let mut bytes = countries_with_aliases();
        put(&mut bytes, 0x700, &1_u32.to_le_bytes());
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        let mut rows = [LinkRow::default(); 8];
        assert_eq!(
            walk_links(&memory, 0, 0x1000, &mut rows),
            Outcome::Rows { len: 3 }
        );
        assert_eq!(
            &rows[..3],
            &[
                LinkRow {
                    id: 1,
                    canonical_slot: 1
                },
                LinkRow {
                    id: 2,
                    canonical_slot: 1
                },
                LinkRow {
                    id: 7,
                    canonical_slot: 1
                },
            ]
        );
        bytes[0x704..0x720].fill(0);
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        assert_eq!(
            walk_links(&memory, 0, 0x1000, &mut []),
            Outcome::Rows { len: 0 }
        );
    }

    #[test]
    fn invalid_links_never_publish_a_successful_prefix() {
        let cases: &[(usize, &[u8], &str)] = &[
            (
                0x710,
                &3_u32.to_le_bytes(),
                "country link outside country table",
            ),
            (
                0x508,
                &0_u64.to_le_bytes(),
                "country link resolves to null country",
            ),
            (0x1000, &0_u64.to_le_bytes(), "country vtable mismatch"),
            (0x1000 + ID, &0_u32.to_le_bytes(), "country id is zero"),
            (
                0x71c,
                &0_u32.to_le_bytes(),
                "country link does not resolve to its canonical object",
            ),
            (
                0x704,
                &2_u32.to_le_bytes(),
                "country link does not resolve to its canonical object",
            ),
            (
                cgame_state::COUNTRIES,
                &0x9000_u64.to_le_bytes(),
                "canonical country pointer unreadable",
            ),
            (
                cgame_state::COUNTRY_LINKS,
                &0x9000_u64.to_le_bytes(),
                "country link unreadable or outside link table",
            ),
            (
                cgame_state::COUNTRY_LINKS + 12,
                &9_u32.to_le_bytes(),
                "country links unreadable or invalid",
            ),
        ];
        for &(offset, replacement, reason) in cases {
            let mut bytes = countries_with_aliases();
            put(&mut bytes, offset, replacement);
            let memory = SliceMemory {
                base: 0x1000,
                bytes: &bytes,
            };
            let mut rows = [LinkRow::default(); 8];
            let expected = Outcome::Unavailable { reason };
            assert_eq!(walk_links(&memory, 0, 0x1000, &mut rows), expected);
            assert_eq!(walk_links(&memory, 0, 0x1000, &mut []), expected);
        }
    }

    #[test]
    fn counter_inputs_keep_raw_pride_reference_and_owned_state_count() {
        let mut bytes = appearance_fixture();
        put(
            &mut bytes,
            0x3000 + PRIDE_OF_THE_FLEET,
            &((32_u64 << 32) | 51).to_le_bytes(),
        );
        movement_test_array(&mut bytes, 0x3000 + DIVISIONS, 0x12000, 2);
        put(&mut bytes, 0x12000, &0x12100_u64.to_le_bytes());
        put(&mut bytes, 0x12008, &0x12200_u64.to_le_bytes());
        put(
            &mut bytes,
            0x12100,
            &(carmy::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            0x12100 + cunit::HANDLE,
            &((7_u64 << 32) | 4).to_le_bytes(),
        );
        put(&mut bytes, 0x3000 + OWNED_STATES, &0x9800_u64.to_le_bytes());
        put(
            &mut bytes,
            0x3000 + OWNED_STATE_CAPACITY,
            &4_u32.to_le_bytes(),
        );
        put(&mut bytes, 0x3000 + OWNED_STATE_COUNT, &3_u32.to_le_bytes());
        put(
            &mut bytes,
            0x3000 + CONTROLLED_STATES,
            &0x9a00_u64.to_le_bytes(),
        );
        put(
            &mut bytes,
            0x3000 + CONTROLLED_STATES + 8,
            &6_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            0x3000 + CONTROLLED_STATES_COUNT,
            &5_u32.to_le_bytes(),
        );
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let mut output = vec![CounterInputsRow::default(); 2];
        let census = engine_native_memory::read_census::ReadCensus::new(&memory);
        assert_eq!(
            walk_counter_inputs_inner(&census, 0, 0x1000, engine_native_memory::walk::Buffer::Growing(&mut output), None, None, None, None),
            Outcome::Rows { len: 2 }
        );
        assert_eq!(
            output[..2],
            [
                CounterInputsRow {
                    country: 1,
                    pride_of_the_fleet: (32_u64 << 32) | 51,
                    owned_state_count: 3,
                    controlled_state_count: 5,
                    first_division: (7_u64 << 32) | 4,
                    attack_modifiers: Some([None, None]),
                    movement_modifiers: Some([None; MOVEMENT_MODIFIER_KEYS.len()]),
                    recruitment_modifiers: Some([None; 4]),
                    air_modifiers: Some([None; AIR_MODIFIER_KEYS.len()]),
                    air_combat_modifiers: Some([None; AIR_COMBAT_MODIFIER_KEYS.len()]),
                    mines_effect_reduction: Some(None),
                    naval_detection: Some(None),
                    ..Default::default()
                },
                CounterInputsRow {
                    country: 2,
                    attack_modifiers: Some([None, None]),
                    movement_modifiers: Some([None; MOVEMENT_MODIFIER_KEYS.len()]),
                    recruitment_modifiers: Some([None; 4]),
                    air_modifiers: Some([None; AIR_MODIFIER_KEYS.len()]),
                    air_combat_modifiers: Some([None; AIR_COMBAT_MODIFIER_KEYS.len()]),
                    mines_effect_reduction: Some(None),
                    naval_detection: Some(None),
                    ..Default::default()
                },
            ]
        );
        put(&mut bytes, 0x12000, &0_u64.to_le_bytes());
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        walk_counter_inputs_inner(&memory, 0, 0x1000, engine_native_memory::walk::Buffer::Growing(&mut output), None, None, None, None);
        assert_eq!(output[0].first_division, 0);
        put(&mut bytes, 0x12000, &0x12200_u64.to_le_bytes());
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        assert_eq!(
            walk_counter_inputs_inner(&memory, 0, 0x1000, engine_native_memory::walk::Buffer::Growing(&mut output), None, None, None, None),
            Outcome::Unavailable {
                reason: "country first division is not an army"
            }
        );
        put(&mut bytes, 0x12000, &0x12100_u64.to_le_bytes());
        put(&mut bytes, 0x3000 + OWNED_STATE_COUNT, &5_u32.to_le_bytes());
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        assert_eq!(
            walk_counter_inputs_inner(&memory, 0, 0x1000, engine_native_memory::walk::Buffer::Growing(&mut output), None, None, None, None),
            Outcome::Unavailable {
                reason: "country owned-state array unreadable or invalid"
            }
        );
        put(&mut bytes, 0x3000 + OWNED_STATE_COUNT, &3_u32.to_le_bytes());
        put(
            &mut bytes,
            0x3000 + CONTROLLED_STATES_COUNT,
            &7_u32.to_le_bytes(),
        );
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        assert_eq!(
            walk_counter_inputs_inner(&memory, 0, 0x1000, engine_native_memory::walk::Buffer::Growing(&mut output), None, None, None, None),
            Outcome::Unavailable {
                reason: "country controlled-state array unreadable or invalid"
            }
        );
    }

    fn civil_war_fixture() -> Vec<u8> {
        let mut bytes = appearance_fixture();
        movement_test_array(&mut bytes, 0x3000 + CIVIL_WAR_TARGETS, 0x9900, 3);
        for (index, target) in [9_u32, 3, 0x8000_0002].into_iter().enumerate() {
            put(&mut bytes, 0x9900 + index * 4, &target.to_le_bytes());
        }
        bytes
    }

    #[test]
    fn civil_wars_keep_native_order_and_raw_references() {
        let bytes = civil_war_fixture();
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let mut output = vec![CivilWarRow::default(); 4];
        assert_eq!(
            walk_civil_wars(&memory, 0, 0x1000, &mut output),
            Outcome::Rows { len: 3 }
        );
        assert_eq!(
            output[..3],
            [
                CivilWarRow {
                    country: 1,
                    ordinal: 0,
                    target: 9
                },
                CivilWarRow {
                    country: 1,
                    ordinal: 1,
                    target: 3
                },
                CivilWarRow {
                    country: 1,
                    ordinal: 2,
                    target: 0x8000_0002
                },
            ]
        );
        assert_eq!(
            walk_civil_wars_buffer(&memory, 0, 0x1000, engine_native_memory::walk::Buffer::Count),
            Outcome::Truncated {
                len: 0,
                unread_rows: 3
            }
        );
        assert_eq!(
            walk_civil_wars(
                &SliceMemory {
                    base: 0,
                    bytes: &appearance_fixture()
                },
                0,
                0x1000,
                &mut output
            ),
            Outcome::Rows { len: 0 }
        );
    }

    #[test]
    fn unreadable_civil_war_arrays_fail_the_table() {
        let mut bytes = civil_war_fixture();
        put(
            &mut bytes,
            0x3000 + CIVIL_WAR_TARGETS + 12,
            &4_u32.to_le_bytes(),
        );
        let run = |bytes: &[u8]| {
            walk_civil_wars(
                &SliceMemory { base: 0, bytes },
                0,
                0x1000,
                &mut vec![CivilWarRow::default(); 4],
            )
        };
        assert_eq!(
            run(&bytes),
            Outcome::Unavailable {
                reason: "country civil war array unreadable or invalid"
            }
        );
        let mut bytes = civil_war_fixture();
        put(
            &mut bytes,
            0x3000 + CIVIL_WAR_TARGETS,
            &0x9_0000_0000_u64.to_le_bytes(),
        );
        assert_eq!(
            run(&bytes),
            Outcome::Unavailable {
                reason: "country civil war targets unreadable"
            }
        );
        let mut bytes = civil_war_fixture();
        put(&mut bytes, 0x3000 + ID, &7_u32.to_le_bytes());
        assert!(matches!(run(&bytes), Outcome::Unavailable { .. }));
    }

    #[test]
    fn growing_appearance_preserves_long_ideology_and_rejects_its_original_fault() {
        let mut bytes = appearance_fixture();
        let len = 5_001;
        let payload = bytes.len();
        bytes.resize(payload + len + 1, 0);
        bytes[payload..payload + len].fill(b'i');
        put(
            &mut bytes,
            0xaa00 + IDEOLOGY_TOKEN,
            &(payload as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            0xaa00 + IDEOLOGY_TOKEN + 16,
            &(len as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            0xaa00 + IDEOLOGY_TOKEN + 24,
            &(len as u64).to_le_bytes(),
        );
        let capture = |bytes: &[u8]| {
            let mut rows = Vec::new();
            let outcome = walk_appearance_observed_inner::<
                engine_native_memory::capture_text::Owned,
                engine_native_memory::capture_text::Owned,
                engine_native_memory::capture_text::Owned,
            >(
                &SliceMemory { base: 0, bytes },
                0,
                0x1000,
                1,
                engine_native_memory::walk::Buffer::Growing(&mut rows),
                None,
            );
            (outcome, rows)
        };
        let (outcome, rows) = capture(&bytes);
        assert_eq!(outcome, Outcome::Rows { len: 2 });
        assert_eq!(rows[0].leader_ideology.as_bytes(), vec![b'i'; len]);
        assert_eq!(rows[0].leader_character, Some(777));
        bytes[payload + len] = 1;
        assert!(matches!(capture(&bytes).0, Outcome::Unavailable { .. }));
    }
}
