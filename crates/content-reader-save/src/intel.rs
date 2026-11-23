
use std::{collections::BTreeMap, io};

use super::{
    Decimal, GameDate, block, boolean, bytes, decimal, field, integer, invalid, required_block,
    required_bytes, scalar,
};
use content_reader_parser::parser::{Item, Value};
#[path = "intel_network_auxiliary.rs"]
pub mod network_auxiliary;
#[path = "intel_sources.rs"]
pub mod sources;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pack {
    pub countries: BTreeMap<Vec<u8>, Country>,
    pub global_variables: Option<Variables>,
    pub state_variables: Option<BTreeMap<u32, Option<Variables>>>,
    pub strategic_operatives: Option<BTreeMap<u32, StrategicOperative>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Country {
    pub sources: Option<sources::Sources>,
    pub radar_source: Option<sources::Reference>,
    pub token_source: Option<sources::Reference>,
    pub agency: Option<Agency>,
    pub diplomacy: Option<Diplomacy>,
    pub flags: Option<Vec<Flag>>,
    pub variables: Option<Variables>,
    pub operation_tokens: Option<OperationTokenManager>,
    pub incoming_actions: Option<Vec<IncomingAction>>,
    pub operations: Option<Operations>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IncomingAction {
    pub ordinal: u32,
    pub id: u64,
    pub kind: Vec<u8>,
    pub action_type: i32,
    pub initiator_matters: Option<bool>,
    pub actor: Vec<u8>,
    pub original_actor: Vec<u8>,
    pub recipient: Vec<u8>,
    pub original_recipient: Vec<u8>,
    pub date: GameDate,
    pub last_command_date: GameDate,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Agency {
    pub name: Option<Vec<u8>>,
    pub created: bool,
    pub upgrade_progress: Decimal,
    pub upgrades: Vec<AgencyUpgradeLevel>,
    pub defense: Decimal,
    pub active_count: i32,
    pub unknown_f0: u32,
    pub admission_ceiling: i32,
    pub operatives: Vec<OperativeEntry>,
    pub captured: Vec<CapturedOperativeReference>,
    pub recruitment: Option<Recruitment>,
    pub cryptology: Option<Cryptology>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapturedOperativeReference {
    pub country: Option<Vec<u8>>,
    pub operative: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgencyUpgradeLevel {
    pub token: Vec<u8>,
    pub level: i32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Diplomacy {
    pub subjects: Option<Vec<Subject>>,
    pub autonomy: Option<Autonomy>,
    pub capitulated: Option<bool>,
    pub capitulated_date: Option<GameDate>,
    pub last_surrender_date: Option<GameDate>,
    pub faction_join_date: Option<GameDate>,
    pub legitimacy: Option<Decimal>,
    pub exile_host: Option<Vec<u8>>,
    pub exile_army_leaders: Option<i32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Autonomy {
    pub progress: Decimal,
    pub state: Vec<u8>,
    pub previous: Option<Vec<u8>>,
    pub next: Option<Vec<u8>>,
    pub last_modified: GameDate,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Subject {
    pub roster_owner: Vec<u8>,
    pub target: Vec<u8>,
    pub first: Vec<u8>,
    pub second: Vec<u8>,
    pub state: Option<Vec<u8>>,
    pub autonomy_ratio: Decimal,
    pub start_date: Option<GameDate>,
    pub end_date: Option<GameDate>,
    pub cancel: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Flag {
    pub name: Vec<u8>,
    pub value: i16,
    pub date: GameDate,
    pub remaining_days: Option<i16>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Variables {
    pub entries: Vec<Variable>,
    pub seed: Option<u32>,
    pub reserved_entries: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Variable {
    pub name: Vec<u8>,
    pub value: Decimal,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OperationTokenManager {
    pub operation_assets: Option<Vec<OperationTokens>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OperationTokens {
    pub target: Vec<u8>,
    pub names: Vec<Vec<u8>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Operations {
    pub priority: Option<i32>,
    pub finished: Option<Vec<Item>>,
    pub running: Option<Vec<OperationInstance>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OperationInstance {
    pub definition: Vec<u8>,
    pub id: u64,
    pub target: Vec<u8>,
    pub target_province: Option<u32>,
    pub date: Option<GameDate>,
    pub duration: Option<i32>,
    pub phases: Option<Vec<Vec<u8>>>,
    pub operative_slots: Option<Vec<OperationSlot>>,
    pub civilian_factories: Option<Decimal>,
    pub total: Option<Decimal>,
    pub resources: Option<Vec<OperationResource>>,
    pub equipment: Option<super::air::EquipmentPool>,
    pub terminal: bool,
    pub bonus: bool,
    pub detected: bool,
    pub effect_description: Option<Vec<u8>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OperationResource {
    Equipment { archetype: Vec<u8>, amount: Decimal },
    Factories { amount: i32, days: i32 },
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OperationSlot {
    pub operative: Option<u64>,
    pub mission: Option<Vec<Item>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OperativeEntry {
    pub id: u64,
    pub name: Option<Vec<u8>>,
    pub skill: Option<i32>,
    pub experience: Option<Decimal>,
    pub nationalities: Vec<Vec<u8>>,
    pub captured: Option<Vec<u8>>,
    pub capture_date: Option<GameDate>,
    pub cooldown_reason: Option<Vec<u8>>,
    pub cooldown_start_date: Option<GameDate>,
    pub cooldown_end_date: Option<GameDate>,
    pub state: Option<Vec<u8>>,
    pub operation: Option<u64>,
    pub mission_present: bool,
    pub mission_name: Option<Vec<u8>>,
    pub mission_target: Option<Vec<u8>>,
    pub mission_state: Option<i32>,
    pub mission_ideology: Option<Vec<u8>>,
    pub gfx: Option<Vec<u8>>,
    pub codename: Option<super::units::DivisionName>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Recruitment {
    pub recruitable: Vec<OperativeEntry>,
    pub not_to_spy_master: Vec<OperativeEntry>,
    pub other_rosters: Vec<(Vec<u8>, Vec<OperativeEntry>)>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Cryptology {
    pub targets: Option<Vec<CryptologyTarget>>,
    pub intel_source_country: Option<Vec<u8>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CryptologyTarget {
    pub target: Vec<u8>,
    pub amount: Option<Decimal>,
    pub date: Option<GameDate>,
    pub decryption: Option<bool>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IntelNetworkEntry {
    pub auxiliary: network_auxiliary::Network,
    pub target: Vec<u8>,
    pub source: Option<sources::Reference>,
    pub coverage: Option<NetworkCoverage>,
    pub strength_sum: Option<Decimal>,
    pub strength_sum_over_cores: Option<Decimal>,
    pub state_strength_max: Option<Decimal>,
    pub state_ids: Vec<i32>,
    pub strengths: Vec<Decimal>,
    pub gains: Vec<Decimal>,
    pub gains_from_operatives: Vec<Decimal>,
    pub gains_from_adjacency: Vec<Decimal>,
    pub local_quiet: Vec<bool>,
    pub subnetworks: Vec<SavedSubnetwork>,
    pub selectors: Vec<i32>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NetworkCoverage {
    pub core_states: i32,
    pub controlled_states: i32,
    pub owned_worth: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SavedSubnetwork {
    pub strength: Decimal,
    pub gain_factor: Decimal,
    pub strength_target: Decimal,
    pub target_from_operatives: Decimal,
    pub target_from_counterintelligence: Decimal,
    pub quiet: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StrategicOperative {
    pub index: u32,
    pub networks: Vec<IntelNetworkEntry>,
    pub mission_activity: Option<Decimal>,
    pub operation_activity: Option<Decimal>,
    pub trade_influence: Option<Vec<(Vec<u8>, Decimal)>>,
    pub diplomatic_pressure: Option<Vec<SavedPressure>>,
    pub recent_propaganda_effort: Vec<PropagandaEffort>,
    pub propaganda_weekly_drift: PropagandaEffort,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PropagandaEffort {
    pub stability: i64,
    pub war_support: i64,
}

fn parse_propaganda_effort(value: &Value) -> io::Result<PropagandaEffort> {
    let fields = block(value)?;
    Ok(PropagandaEffort {
        stability: super::economy::FuelQuantity::parse(&required_bytes(fields, b"stability")?)?
            .raw()?,
        war_support: super::economy::FuelQuantity::parse(&required_bytes(fields, b"war_support")?)?
            .raw()?,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SavedPressure {
    pub country: Vec<u8>,
    pub opinion: i64,
    pub tension: i64,
}

fn date(items: &[Item], key: &[u8]) -> io::Result<Option<GameDate>> {
    bytes(items, key)?
        .map(|value| GameDate::parse(&value))
        .transpose()
}

fn parse_agency(items: &[Item]) -> io::Result<Agency> {
    let mut operatives = Vec::new();
    for item in items
        .iter()
        .filter(|item| item.key.as_deref() == Some(b"operative"))
    {
        operatives.push(parse_operative_entry(block(&item.value)?)?);
    }
    let active_count = i32::try_from(operatives.len())
        .map_err(|_| invalid("saved agency active count exceeds i32"))?;
    let recruitment = field(items, b"recruitment")?
        .map(|value| parse_recruitment(block(value)?))
        .transpose()?;
    let cryptology = field(items, b"cryptology")?
        .map(|value| parse_cryptology(block(value)?))
        .transpose()?;
    let captured = field(items, b"captured")?
        .map(|value| {
            let entries = block(value)?;
            if entries.is_empty() {
                return Err(invalid(
                    "agency writer omits an empty captured-operative roster",
                ));
            }
            entries
                .iter()
                .map(|entry| {
                    if entry.key.is_some() {
                        return Err(invalid("captured-operative entry must be anonymous"));
                    }
                    let fields = block(&entry.value)?;
                    Ok(CapturedOperativeReference {
                        country: bytes(fields, b"country")?,
                        operative: super::null_permitted_handle(
                            field(fields, b"operative")?
                                .ok_or_else(|| invalid("captured entry lacks operative"))?,
                        )?,
                    })
                })
                .collect::<io::Result<Vec<_>>>()
        })
        .transpose()?
        .unwrap_or_default();
    Ok(Agency {
        name: bytes(items, b"name")?,
        created: field(items, b"is_created")?
            .map(boolean)
            .transpose()?
            .unwrap_or(false),
        upgrade_progress: decimal(items, b"upgrade_progress")?.unwrap_or(Decimal::parse(b"0")?),
        upgrades: field(items, b"upgrades")?
            .map(parse_agency_upgrades)
            .transpose()?
            .unwrap_or_default(),
        defense: Decimal::parse(&required_bytes(items, b"defense")?)?,
        active_count,
        unknown_f0: integer::<i32>(&required_bytes(items, b"max_operative_count")?)? as u32,
        admission_ceiling: integer(&required_bytes(items, b"usable_operative_slots")?)?,
        operatives,
        captured,
        recruitment,
        cryptology,
    })
}

fn parse_agency_upgrades(value: &Value) -> io::Result<Vec<AgencyUpgradeLevel>> {
    let mut entries = Vec::new();
    for entry in block(value)? {
        let token = entry
            .key
            .as_ref()
            .filter(|name| !name.is_empty())
            .ok_or_else(|| invalid("agency upgrades entry lacks its definition token"))?;
        let refuse = || {
            invalid(format!(
                "agency upgrade {} is not a signed i32 scalar assignment",
                String::from_utf8_lossy(token)
            ))
        };
        if entry.operator != content_reader_parser::parser::Operator::Equals {
            return Err(refuse());
        }
        let level = integer(scalar(&entry.value).map_err(|_| refuse())?).map_err(|_| refuse())?;
        entries.push(AgencyUpgradeLevel {
            token: token.clone(),
            level,
        });
    }
    if entries.is_empty() {
        return Err(invalid("native agency writer omits empty upgrades lists"));
    }
    Ok(entries)
}

fn parse_operative_entry(items: &[Item]) -> io::Result<OperativeEntry> {
    let id = super::object_id(items)?;
    let nationalities = field(items, b"nationalities")?
        .map(|value| {
            let mut tags = Vec::new();
            for entry in block(value)? {
                if entry.key.is_some() {
                    return Err(invalid("operative nationalities must be an anonymous list"));
                }
                let tag = scalar(&entry.value)?.to_vec();
                if tag.is_empty() {
                    return Err(invalid("operative nationality tag is empty"));
                }
                tags.push(tag);
            }
            Ok::<_, io::Error>(tags)
        })
        .transpose()?
        .unwrap_or_default();
    let captured = bytes(items, b"captured")?.filter(|tag| !tag.is_empty());
    let capture_date = date(items, b"capture_date")?;
    if captured.is_none() != capture_date.is_none() {
        return Err(invalid(
            "operative capturer tag and capture date must be emitted together",
        ));
    }
    let (mission_present, mission_name, mission_target, mission_state, mission_ideology) =
        match field(items, b"mission")? {
            None => (false, None, None, None, None),
            Some(value) => {
                let fields = block(value)?;
                if fields.len() != 1 {
                    return Err(invalid(
                        "operative mission must contain exactly one definition",
                    ));
                }
                let name = fields[0]
                    .key
                    .clone()
                    .filter(|name| !name.is_empty())
                    .ok_or_else(|| invalid("operative mission lacks its definition name"))?;
                let definition = block(&fields[0].value)?;
                let target = bytes(definition, b"target")?;
                if target.as_ref().is_some_and(|target| target.is_empty()) {
                    return Err(invalid("operative mission target is empty"));
                }
                let state = field(definition, b"state")?
                    .map(|value| integer(scalar(value)?))
                    .transpose()?;
                let ideology = bytes(definition, b"ideology")?;
                if ideology
                    .as_ref()
                    .is_some_and(|ideology| ideology.is_empty())
                {
                    return Err(invalid("operative mission ideology is empty"));
                }
                (true, Some(name), target, state, ideology)
            }
        };
    Ok(OperativeEntry {
        id,
        name: bytes(items, b"name")?,
        skill: field(items, b"skill")?
            .map(|value| integer(scalar(value)?))
            .transpose()?,
        experience: decimal(items, b"experience")?,
        nationalities,
        captured,
        capture_date,
        cooldown_reason: bytes(items, b"cooldown_reason")?,
        cooldown_start_date: date(items, b"leader_cooldown_start_date")?,
        cooldown_end_date: date(items, b"leader_modifier_enable_date")?,
        state: bytes(items, b"state")?,
        operation: field(items, b"operation")?
            .map(super::typed_handle)
            .transpose()?,
        mission_present,
        mission_name,
        mission_target,
        mission_state,
        mission_ideology,
        gfx: match bytes(items, b"gfx")? {
            Some(gfx) if gfx.is_empty() => {
                return Err(invalid("the leader writer omits an empty portrait gfx"));
            }
            gfx => gfx,
        },
        codename: field(items, b"codename")?
            .map(|value| super::units::division_name(block(value)?))
            .transpose()?,
    })
}

fn parse_recruitment_list(value: &Value) -> io::Result<Vec<OperativeEntry>> {
    let mut entries = Vec::new();
    for entry in block(value)? {
        if entry.key.is_some() {
            return Err(invalid(
                "recruitment list must hold anonymous operative blocks",
            ));
        }
        entries.push(parse_operative_entry(block(&entry.value)?)?);
    }
    Ok(entries)
}

fn parse_recruitment(items: &[Item]) -> io::Result<Recruitment> {
    let mut other_rosters = Vec::new();
    for item in items {
        let key = item
            .key
            .as_deref()
            .ok_or_else(|| invalid("recruitment roster lacks its saved name"))?;
        if key != b"recruitable_operatives" && key != b"recruitable_operatives_not_to_spy_master" {
            other_rosters.push((key.to_vec(), parse_recruitment_list(&item.value)?));
        }
    }
    Ok(Recruitment {
        recruitable: field(items, b"recruitable_operatives")?
            .map(parse_recruitment_list)
            .transpose()?
            .unwrap_or_default(),
        not_to_spy_master: field(items, b"recruitable_operatives_not_to_spy_master")?
            .map(parse_recruitment_list)
            .transpose()?
            .unwrap_or_default(),
        other_rosters,
    })
}

fn parse_cryptology(items: &[Item]) -> io::Result<Cryptology> {
    let targets = field(items, b"targets")?
        .map(|value| {
            let mut result = Vec::new();
            let mut seen = std::collections::BTreeSet::new();
            for entry in block(value)? {
                if entry.key.is_some() {
                    return Err(invalid("cryptology targets must be anonymous blocks"));
                }
                let fields = block(&entry.value)?;
                let target = required_bytes(fields, b"target")?;
                if !seen.insert(target.clone()) {
                    return Err(invalid("duplicate cryptology target"));
                }
                let decryption = field(fields, b"decryption")?.map(boolean).transpose()?;
                if decryption == Some(false) {
                    return Err(invalid(
                        "cryptology decryption explicitly saves no; native writer omits an unset flag",
                    ));
                }
                result.push(CryptologyTarget {
                    target,
                    amount: decimal(fields, b"amount")?,
                    date: date(fields, b"date")?,
                    decryption,
                });
            }
            if result.is_empty() {
                return Err(invalid("native cryptology writer omits empty target lists"));
            }
            Ok::<_, io::Error>(result)
        })
        .transpose()?;
    let intel_source_country = field(items, b"intel_source")?
        .map(|value| {
            let fields = block(value)?;
            bytes(fields, b"country")
        })
        .transpose()?
        .flatten();
    Ok(Cryptology {
        targets,
        intel_source_country,
    })
}

fn parse_intel_network_entry(items: &[Item]) -> io::Result<IntelNetworkEntry> {
    let target = required_bytes(items, b"target")?;
    let inner = required_block(items, b"intel_network")?;
    let has_states = field(inner, b"state_id")?.is_some();
    let list = |key: &[u8]| -> io::Result<&[Item]> {
        match field(inner, key)? {
            Some(value) => block(value),
            None if !has_states => Ok(&[]),
            None => Err(invalid(format!(
                "intel network lacks {}",
                String::from_utf8_lossy(key)
            ))),
        }
    };
    if !has_states
        && [
            b"strength".as_slice(),
            b"sub_network_id",
            b"gain",
            b"gain_breakdown",
        ]
        .iter()
        .any(|key| inner.iter().any(|item| item.key.as_deref() == Some(*key)))
    {
        return Err(invalid("intel network parallel arrays lack state_id"));
    }
    let mut state_ids = Vec::new();
    for entry in list(b"state_id")? {
        if entry.key.is_some() {
            return Err(invalid("intel state_id list must be anonymous"));
        }
        state_ids.push(integer(scalar(&entry.value)?)?);
    }
    let mut strengths = Vec::new();
    for entry in list(b"strength")? {
        if entry.key.is_some() {
            return Err(invalid("intel strength list must be anonymous"));
        }
        strengths.push(Decimal::parse(scalar(&entry.value)?)?);
    }
    let mut selectors = Vec::new();
    for entry in list(b"sub_network_id")? {
        if entry.key.is_some() {
            return Err(invalid("intel sub_network_id list must be anonymous"));
        }
        selectors.push(integer(scalar(&entry.value)?)?);
    }
    let mut gains = Vec::new();
    for entry in list(b"gain")? {
        if entry.key.is_some() {
            return Err(invalid("intel gain list must be anonymous"));
        }
        gains.push(Decimal::parse(scalar(&entry.value)?)?);
    }
    let mut gains_from_operatives = Vec::new();
    let mut gains_from_adjacency = Vec::new();
    for entry in list(b"gain_breakdown")? {
        if entry.key.is_some() {
            return Err(invalid("intel gain_breakdown list must be anonymous"));
        }
        let parts = block(&entry.value)?;
        gains_from_operatives.push(Decimal::parse(&required_bytes(parts, b"operatives")?)?);
        gains_from_adjacency.push(Decimal::parse(&required_bytes(parts, b"adjacencies")?)?);
    }
    let local_quiet = field(inner, b"sub_network_is_quiet")?
        .map(|value| {
            block(value)?
                .iter()
                .map(|entry| {
                    if entry.key.is_some() {
                        return Err(invalid("intel sub_network_is_quiet list must be anonymous"));
                    }
                    boolean(&entry.value)
                })
                .collect::<io::Result<Vec<_>>>()
        })
        .transpose()?
        .unwrap_or_default();
    if state_ids.len() != strengths.len()
        || state_ids.len() != selectors.len()
        || state_ids.len() != gains.len()
        || state_ids.len() != gains_from_operatives.len()
    {
        return Err(invalid("intel parallel arrays have different lengths"));
    }
    if has_states && state_ids.is_empty() {
        return Err(invalid(
            "native intel network writer omits empty state lists",
        ));
    }
    let mut seen = std::collections::BTreeSet::new();
    for id in &state_ids {
        if !seen.insert(*id) {
            return Err(invalid("duplicate intel state id in network"));
        }
    }
    let mut subnetworks = Vec::new();
    if let Some(value) = field(items, b"sub_intel_networks")? {
        for entry in block(value)? {
            if entry.key.is_some() {
                return Err(invalid("intel subnetworks list must be anonymous"));
            }
            let fields = block(&entry.value)?;
            let required = |key: &[u8]| -> io::Result<Decimal> {
                decimal(fields, key)?.ok_or_else(|| {
                    invalid(format!(
                        "intel subnetwork lacks saved {}",
                        String::from_utf8_lossy(key)
                    ))
                })
            };
            let modifiers = required_block(fields, b"modifiers")?;
            subnetworks.push(SavedSubnetwork {
                strength: required(b"strength")?,
                gain_factor: decimal(modifiers, b"intel_network_gain_factor")?.ok_or_else(
                    || invalid("intel subnetwork lacks saved intel_network_gain_factor"),
                )?,
                strength_target: required(b"sub_network_strength_target")?,
                target_from_operatives: required(b"sub_network_strength_target_from_operatives")?,
                target_from_counterintelligence: required(
                    b"sub_network_strength_target_from_counterintelligence",
                )?,
                quiet: field(fields, b"sub_network_is_quiet")?
                    .map(boolean)
                    .transpose()?
                    .unwrap_or(false),
            });
        }
    }
    for selector in &selectors {
        if *selector >= 0 && *selector as usize >= subnetworks.len() {
            return Err(invalid(
                "intel subnetwork selector outside saved subnetworks",
            ));
        }
    }
    Ok(IntelNetworkEntry {
        auxiliary: network_auxiliary::parse(items, inner)?,
        target,
        source: field(items, b"intel_source")?
            .map(sources::reference)
            .transpose()?,
        coverage: field(items, b"coverage")?
            .map(|value| {
                let fields = block(value)?;
                Ok::<_, io::Error>(NetworkCoverage {
                    core_states: integer(&required_bytes(fields, b"core_states")?)?,
                    controlled_states: integer(&required_bytes(fields, b"controlled_states")?)?,
                    owned_worth: integer(&required_bytes(fields, b"owned_worth")?)?,
                })
            })
            .transpose()?,
        strength_sum: decimal(items, b"strength_sum")?,
        strength_sum_over_cores: decimal(items, b"strength_sum_over_cores")?,
        state_strength_max: decimal(items, b"state_strength_max")?,
        state_ids,
        strengths,
        gains,
        gains_from_operatives,
        gains_from_adjacency,
        local_quiet,
        subnetworks,
        selectors,
    })
}

fn parse_strategic_operatives(items: &[Item]) -> io::Result<BTreeMap<u32, StrategicOperative>> {
    let mut result = BTreeMap::new();
    for item in required_block(items, b"strategic_operative")? {
        let index = integer::<u32>(
            item.key
                .as_deref()
                .ok_or_else(|| invalid("strategic operative lacks its numeric index key"))?,
        )?;
        let mut networks = Vec::new();
        let mut seen_targets = std::collections::BTreeSet::new();
        let fields = block(&item.value)?;
        for entry in fields
            .iter()
            .filter(|entry| entry.key.as_deref() == Some(b"intel_network"))
        {
            let network = parse_intel_network_entry(block(&entry.value)?)?;
            if !seen_targets.insert(network.target.clone()) {
                return Err(invalid(
                    "duplicate intel network target for operative index",
                ));
            }
            networks.push(network);
        }
        let level = |key: &[u8]| {
            field(fields, key)?
                .map(|value| {
                    decimal(block(value)?, b"value")?.ok_or_else(|| {
                        invalid(format!(
                            "strategic operative {} lacks its value",
                            String::from_utf8_lossy(key)
                        ))
                    })
                })
                .transpose()
        };
        let operative = StrategicOperative {
            index,
            networks,
            mission_activity: level(b"subversive_activity_level".as_slice())?,
            operation_activity: level(b"danger_level".as_slice())?,
            trade_influence: field(fields, b"trade_influence_modifier_of_buyers")?
                .map(parse_trade_influence)
                .transpose()?,
            diplomatic_pressure: field(fields, b"diplomatic_pressure_from_countries")?
                .map(parse_diplomatic_pressure)
                .transpose()?,
            recent_propaganda_effort: field(fields, b"recent_propaganda_effort")?
                .map(|value| {
                    block(value)?
                        .iter()
                        .map(|entry| {
                            if entry.key.is_some() {
                                return Err(invalid("propaganda effort history entry is keyed"));
                            }
                            parse_propaganda_effort(&entry.value)
                        })
                        .collect::<io::Result<Vec<_>>>()
                })
                .transpose()?
                .unwrap_or_default(),
            propaganda_weekly_drift: field(fields, b"propaganda_weekly_drift")?
                .map(parse_propaganda_effort)
                .transpose()?
                .unwrap_or_default(),
        };
        if result.insert(index, operative).is_some() {
            return Err(invalid("duplicate strategic operative index"));
        }
    }
    Ok(result)
}

fn influence_entry<'a>(entry: &'a Item, map: &str) -> io::Result<(Vec<u8>, &'a Value)> {
    let refuse = || invalid(format!("{map} entry must hold a country and its value"));
    if entry.key.is_some() {
        return Err(refuse());
    }
    let parts = block(&entry.value)?;
    let [country, payload] = parts else {
        return Err(refuse());
    };
    if country.key.is_some() || payload.key.is_some() {
        return Err(refuse());
    }
    let country = scalar(&country.value)?.to_vec();
    if country.is_empty() {
        return Err(refuse());
    }
    Ok((country, &payload.value))
}

fn parse_trade_influence(value: &Value) -> io::Result<Vec<(Vec<u8>, Decimal)>> {
    const MAP: &str = "trade_influence_modifier_of_buyers";
    let mut entries = Vec::new();
    for entry in block(value)? {
        let (country, amount) = influence_entry(entry, MAP)?;
        entries.push((country, Decimal::parse(scalar(amount)?)?));
    }
    if entries.is_empty() {
        return Err(invalid(format!("native writer omits an empty {MAP}")));
    }
    Ok(entries)
}

fn parse_diplomatic_pressure(value: &Value) -> io::Result<Vec<SavedPressure>> {
    const MAP: &str = "diplomatic_pressure_from_countries";
    let mut entries = Vec::new();
    for entry in block(value)? {
        let (country, payload) = influence_entry(entry, MAP)?;
        let fields = block(payload)?;
        entries.push(SavedPressure {
            country,
            opinion: integer(&required_bytes(fields, b"opinion")?)?,
            tension: integer(&required_bytes(fields, b"tension")?)?,
        });
    }
    if entries.is_empty() {
        return Err(invalid(format!("native writer omits an empty {MAP}")));
    }
    Ok(entries)
}

fn parse_subject(owner: &[u8], target: &[u8], items: &[Item]) -> io::Result<Subject> {
    let cancel = field(items, b"cancel")?.map(boolean).transpose()?;
    if cancel == Some(false) {
        return Err(invalid(
            "subject relation explicitly saves an unset cancel flag",
        ));
    }
    let cancel = cancel.unwrap_or(false);
    let start_date = date(items, b"start_date")?;
    let end_date = date(items, b"end_date")?;
    if cancel == start_date.is_some() {
        return Err(invalid(
            "subject relation must save start_date exactly when not cancelled",
        ));
    }
    if let (Some(start), Some(end)) = (start_date, end_date)
        && start.engine_hour()? >= end.engine_hour()?
    {
        return Err(invalid("subject relation end_date is not after start_date"));
    }
    Ok(Subject {
        roster_owner: owner.to_vec(),
        target: target.to_vec(),
        first: required_bytes(items, b"first")?,
        second: required_bytes(items, b"second")?,
        state: bytes(items, b"autonomy_state")?,
        autonomy_ratio: decimal(items, b"value")?.unwrap_or(Decimal::parse(b"0")?),
        start_date,
        end_date,
        cancel,
    })
}

fn parse_diplomacy(owner: &[u8], items: &[Item]) -> io::Result<Diplomacy> {
    let capitulated = field(items, b"capitulated")?.map(boolean).transpose()?;
    let capitulated_date = date(items, b"capitulated_date")?;
    if capitulated == Some(false) || capitulated.is_some() != capitulated_date.is_some() {
        return Err(invalid(
            "capitulation flag and date must be emitted together for a true flag",
        ));
    }
    let subjects = field(items, b"active_relations")?
        .map(|value| {
            let mut result = Vec::new();
            for relation in block(value)? {
                let target = relation
                    .key
                    .as_deref()
                    .filter(|tag| !tag.is_empty())
                    .ok_or_else(|| invalid("active relation lacks its country target"))?;
                if let Some(subject) = field(block(&relation.value)?, b"puppet")? {
                    result.push(parse_subject(owner, target, block(subject)?)?);
                }
            }
            Ok::<_, io::Error>(result)
        })
        .transpose()?;
    Ok(Diplomacy {
        subjects,
        autonomy: field(items, b"autonomy_state")?
            .map(|value| {
                let fields = block(value)?;
                Ok::<_, io::Error>(Autonomy {
                    progress: decimal(fields, b"progress")?
                        .ok_or_else(|| invalid("autonomy lacks saved progress"))?,
                    state: required_bytes(fields, b"current_state")?,
                    previous: bytes(fields, b"prev_state")?,
                    next: bytes(fields, b"next_state")?,
                    last_modified: GameDate::parse(&required_bytes(
                        fields,
                        b"last_modified_autonomy_date",
                    )?)?,
                })
            })
            .transpose()?,
        capitulated,
        capitulated_date,
        last_surrender_date: date(items, b"last_surrender_date")?,
        faction_join_date: date(items, b"faction_join_date")?,
        legitimacy: decimal(items, b"legitimacy")?,
        exile_host: bytes(items, b"hosting_our_government_in_exile")?,
        exile_army_leaders: bytes(items, b"exile_army_leaders")?
            .map(|value| integer(&value))
            .transpose()?,
    })
}

pub(super) fn parse_flags(items: &[Item]) -> io::Result<Vec<Flag>> {
    let mut result = Vec::new();
    for item in items {
        let name = item
            .key
            .clone()
            .ok_or_else(|| invalid("saved flag lacks its literal key"))?;
        let items = block(&item.value)?;
        let remaining_days = bytes(items, b"days")?
            .map(|value| integer::<i16>(&value))
            .transpose()?;
        if remaining_days.is_some_and(|days| days <= 0) {
            return Err(invalid("saved flag duration must be positive when present"));
        }
        result.push(Flag {
            name,
            value: integer(&required_bytes(items, b"value")?)?,
            date: GameDate::parse(&required_bytes(items, b"date")?)?,
            remaining_days,
        });
    }
    Ok(result)
}

fn parse_operation_tokens(items: &[Item]) -> io::Result<Vec<OperationTokens>> {
    let mut result = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for item in items {
        let target = item
            .key
            .clone()
            .filter(|tag| !tag.is_empty())
            .ok_or_else(|| invalid("operation token list lacks target country"))?;
        if !seen.insert(target.clone()) {
            return Err(invalid("duplicate operation-token target country"));
        }
        let mut names = Vec::new();
        for name in block(&item.value)? {
            if name.key.is_some() {
                return Err(invalid("operation token list contains a keyed entry"));
            }
            names.push(scalar(&name.value)?.to_vec());
        }
        if names.is_empty() {
            return Err(invalid(
                "native operation-token writer omits empty target lists",
            ));
        }
        result.push(OperationTokens { target, names });
    }
    Ok(result)
}

fn parse_operation_phases(value: &Value) -> io::Result<Vec<Vec<u8>>> {
    let mut phases = Vec::new();
    for entry in block(value)? {
        if entry.key.is_some() {
            return Err(invalid("operation phase list contains a keyed entry"));
        }
        phases.push(scalar(&entry.value)?.to_vec());
    }
    if phases.is_empty() {
        return Err(invalid("native operation writer omits empty phase lists"));
    }
    Ok(phases)
}

fn parse_operation_slots(value: &Value) -> io::Result<Vec<OperationSlot>> {
    let mut slots = Vec::new();
    for entry in block(value)? {
        if entry.key.is_some() {
            return Err(invalid("operation slot list contains a keyed entry"));
        }
        let fields = block(&entry.value)?;
        let operative = field(fields, b"operative")?
            .map(super::typed_handle)
            .transpose()?;
        let mission = field(fields, b"mission")?
            .map(|value| block(value).map(<[Item]>::to_vec))
            .transpose()?;
        slots.push(OperationSlot { operative, mission });
    }
    if slots.is_empty() {
        return Err(invalid(
            "native operation writer omits empty assigned-slot lists",
        ));
    }
    Ok(slots)
}

fn parse_operation_resources(value: &Value) -> io::Result<Vec<OperationResource>> {
    let mut resources = Vec::new();
    for entry in block(value)? {
        let name = entry
            .key
            .as_deref()
            .filter(|name| !name.is_empty())
            .ok_or_else(|| invalid("operation resource lacks its name"))?;
        resources.push(match &entry.value {
            Value::Block(fields) => {
                if name != b"civilian_factories" {
                    return Err(invalid(
                        "operation resource block is not the civilian-factory requirement",
                    ));
                }
                OperationResource::Factories {
                    amount: integer(&required_bytes(fields, b"amount")?)?,
                    days: integer(&required_bytes(fields, b"days")?)?,
                }
            }
            value => OperationResource::Equipment {
                archetype: name.to_vec(),
                amount: Decimal::parse(scalar(value)?)?,
            },
        });
    }
    Ok(resources)
}

fn parse_operation_equipment(value: &Value) -> io::Result<super::air::EquipmentPool> {
    let fields = block(value)?;
    let allow_zero = boolean(
        field(fields, b"allow_zero_entries")?
            .ok_or_else(|| invalid("operation equipment pool lacks allow_zero_entries"))?,
    )?;
    let mut entries = Vec::new();
    for entry in fields
        .iter()
        .filter(|item| item.key.as_deref() == Some(b"equipment"))
    {
        let entry = block(&entry.value)?;
        entries.push(super::air::Equipment {
            variant: super::typed_handle(
                field(entry, b"id")?
                    .ok_or_else(|| invalid("operation equipment entry lacks its variant"))?,
            )?,
            amount: decimal(entry, b"amount")?
                .ok_or_else(|| invalid("operation equipment entry lacks its amount"))?,
        });
    }
    if !allow_zero && entries.iter().any(|entry| entry.amount.raw == 0) {
        return Err(invalid(
            "zero operation equipment entry contradicts the pool writer's omission rule",
        ));
    }
    Ok(super::air::EquipmentPool {
        entries,
        allow_zero,
    })
}

fn parse_operation_instance(definition: &[u8], items: &[Item]) -> io::Result<OperationInstance> {
    Ok(OperationInstance {
        definition: definition.to_vec(),
        id: super::object_id(items)?,
        target: required_bytes(items, b"target")?,
        target_province: field(items, b"target_provinces")?
            .map(|value| integer(scalar(value)?))
            .transpose()?,
        date: bytes(items, b"date")?
            .map(|value| GameDate::parse(&value))
            .transpose()?,
        duration: field(items, b"duration")?
            .map(|value| integer(scalar(value)?))
            .transpose()?,
        phases: field(items, b"phases")?
            .map(parse_operation_phases)
            .transpose()?,
        operative_slots: field(items, b"operative_slots")?
            .map(parse_operation_slots)
            .transpose()?,
        civilian_factories: decimal(items, b"civilian_factories")?,
        total: decimal(items, b"total")?,
        resources: field(items, b"resources")?
            .map(parse_operation_resources)
            .transpose()?,
        equipment: field(items, b"equipment")?
            .map(parse_operation_equipment)
            .transpose()?,
        terminal: field(items, b"completed")?
            .map(boolean)
            .transpose()?
            .unwrap_or(false),
        bonus: field(items, b"outcome_extra")?
            .map(boolean)
            .transpose()?
            .unwrap_or(false),
        detected: field(items, b"risk_extra")?
            .map(boolean)
            .transpose()?
            .unwrap_or(false),
        effect_description: bytes(items, b"looted")?,
    })
}

fn parse_operations(items: &[Item]) -> io::Result<Operations> {
    let finished = field(items, b"finished")?
        .map(|value| block(value).map(<[Item]>::to_vec))
        .transpose()?;
    let running = field(items, b"running")?
        .map(|value| {
            let mut instances = Vec::new();
            let mut seen_ids = std::collections::BTreeSet::new();
            for item in block(value)? {
                let definition = item
                    .key
                    .clone()
                    .filter(|name| !name.is_empty())
                    .ok_or_else(|| invalid("running operation lacks its definition name"))?;
                let instance = parse_operation_instance(&definition, block(&item.value)?)?;
                if !seen_ids.insert(instance.id) {
                    return Err(invalid("duplicate running operation id"));
                }
                instances.push(instance);
            }
            if instances.is_empty() {
                return Err(invalid("native operation writer omits empty running lists"));
            }
            Ok::<_, io::Error>(instances)
        })
        .transpose()?;
    Ok(Operations {
        priority: field(items, b"priority")?
            .map(|value| integer(scalar(value)?))
            .transpose()?,
        finished,
        running,
    })
}

pub(super) fn parse_variables(items: &[Item]) -> io::Result<Variables> {
    let mut variables = Variables::default();
    for item in items {
        let name = item
            .key
            .as_deref()
            .ok_or_else(|| invalid("saved variable lacks a literal key"))?;
        if name == b"random" {
            if let Some(words) = item.value.items() {
                if words.len() != 2 || words.iter().any(|word| word.key.is_some()) {
                    return Err(invalid(
                        "variable random metadata requires two unsigned words",
                    ));
                }
                integer::<u32>(scalar(&words[0].value)?)?;
                let seed = integer::<u32>(scalar(&words[1].value)?)?;
                if variables.seed.replace(seed).is_some() {
                    return Err(invalid("duplicate variable random metadata"));
                }
            } else {
                variables.reserved_entries += 1;
            }
            continue;
        }
        variables.entries.push(Variable {
            name: name.to_vec(),
            value: Decimal::parse(scalar(&item.value)?)?,
        });
    }
    Ok(variables)
}

fn parse_incoming_actions(items: &[Item]) -> io::Result<Vec<IncomingAction>> {
    let mut actions = Vec::new();
    for entry in items {
        let ordinal =
            integer::<u32>(entry.key.as_deref().ok_or_else(|| {
                invalid("incoming diplomatic action lacks its native index key")
            })?)?;
        let fields = block(&entry.value)?;
        let id = super::object_id(fields)?;
        let mut body = fields
            .iter()
            .filter(|item| item.key.as_deref() != Some(b"id"));
        let action = body
            .next()
            .ok_or_else(|| invalid("incoming diplomatic action lacks its action body"))?;
        if body.next().is_some() {
            return Err(invalid(
                "incoming diplomatic action carries more than one action body",
            ));
        }
        let kind = action
            .key
            .clone()
            .ok_or_else(|| invalid("incoming diplomatic action body lacks its definition name"))?;
        let fields = block(&action.value)?;
        actions.push(IncomingAction {
            ordinal,
            id,
            kind,
            action_type: integer(&required_bytes(fields, b"type")?)?,
            initiator_matters: field(fields, b"initiator_matters")?
                .map(boolean)
                .transpose()?,
            actor: required_bytes(fields, b"actor")?,
            original_actor: required_bytes(fields, b"original_actor")?,
            recipient: required_bytes(fields, b"recipient")?,
            original_recipient: required_bytes(fields, b"original_recipient")?,
            date: GameDate::parse(&required_bytes(fields, b"date")?)?,
            last_command_date: GameDate::parse(&required_bytes(fields, b"last_command_date")?)?,
        });
    }
    if actions
        .windows(2)
        .any(|pair| pair[0].ordinal >= pair[1].ordinal)
    {
        return Err(invalid(
            "incoming diplomatic action indices are not strictly increasing",
        ));
    }
    Ok(actions)
}

pub fn parse(items: &[Item]) -> io::Result<Pack> {
    parse_with_countries(items, super::CountriesView::from_root(items)?)
}

pub(super) fn parse_with_countries(
    items: &[Item],
    countries: super::CountriesView<'_>,
) -> io::Result<Pack> {
    let global_variables = field(items, b"variables")?
        .map(|value| parse_variables(block(value)?))
        .transpose()?;
    let state_variables = field(items, b"states")?
        .map(|value| {
            let mut states = BTreeMap::new();
            for item in block(value)? {
                let id = integer::<u32>(
                    item.key
                        .as_deref()
                        .ok_or_else(|| invalid("intel state lacks its numeric key"))?,
                )?;
                let variables = field(block(&item.value)?, b"variables")?
                    .map(|value| parse_variables(block(value)?))
                    .transpose()?;
                if states.insert(id, variables).is_some() {
                    return Err(invalid("duplicate state id in intel save pack"));
                }
            }
            Ok::<_, io::Error>(states)
        })
        .transpose()?;
    let country_items = countries.required()?;
    let mut countries = BTreeMap::new();
    for item in country_items {
        let tag = item
            .key
            .as_deref()
            .filter(|tag| !tag.is_empty())
            .ok_or_else(|| invalid("intel country lacks tag"))?;
        let items = block(&item.value)?;
        let country = Country {
            sources: field(items, b"intel")?.map(sources::parse).transpose()?,
            radar_source: sources::owner_reference(items, b"radar")?,
            token_source: sources::owner_reference(items, b"tokens")?,
            agency: field(items, b"intelligence_agency")?
                .map(|value| parse_agency(block(value)?))
                .transpose()?,
            diplomacy: field(items, b"diplomacy")?
                .map(|value| parse_diplomacy(tag, block(value)?))
                .transpose()?,
            flags: field(items, b"flags")?
                .map(|value| parse_flags(block(value)?))
                .transpose()?,
            variables: field(items, b"variables")?
                .map(|value| parse_variables(block(value)?))
                .transpose()?,
            incoming_actions: field(items, b"incoming_diplomatic_action")?
                .map(|value| parse_incoming_actions(block(value)?))
                .transpose()?,
            operation_tokens: field(items, b"tokens")?
                .map(|value| {
                    Ok::<_, io::Error>(OperationTokenManager {
                        operation_assets: field(block(value)?, b"operation_assets")?
                            .map(|value| parse_operation_tokens(block(value)?))
                            .transpose()?,
                    })
                })
                .transpose()?,
            operations: field(items, b"operations")?
                .map(|value| parse_operations(block(value)?))
                .transpose()?,
        };
        if countries.insert(tag.to_vec(), country).is_some() {
            return Err(invalid("duplicate country tag in intel save pack"));
        }
    }
    Ok(Pack {
        countries,
        global_variables,
        state_variables,
        strategic_operatives: field(items, b"strategic_operatives")?
            .map(|value| parse_strategic_operatives(block(value)?))
            .transpose()?,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn propaganda_fixed15_channels_recover_fractional_negative_and_integral_native_words() {
        let save = parsed(
            br"countries={} strategic_operatives={strategic_operative={
            0={recent_propaganda_effort={
                {stability=0.00100 war_support=-0.00100}
                {stability=1 war_support=-2}
                {stability=-0.00003 war_support=0.00006}
            } propaganda_weekly_drift={stability=0.00704 war_support=-0.00704}}
        }}",
        )
        .unwrap();
        let objects = save.strategic_operatives.unwrap();
        assert_eq!(
            objects[&0].recent_propaganda_effort,
            [
                PropagandaEffort {
                    stability: 33,
                    war_support: -33
                },
                PropagandaEffort {
                    stability: 32_768,
                    war_support: -65_536
                },
                PropagandaEffort {
                    stability: -1,
                    war_support: 2
                },
            ]
        );
        assert_eq!(
            objects[&0].propaganda_weekly_drift,
            PropagandaEffort {
                stability: 231,
                war_support: -231
            }
        );
    }

    #[test]
    fn propaganda_fixed15_native_boundaries_have_exact_preimages_without_float_rounding() {
        for (literal, raw) in [
            ("0", 0),
            ("-0.00000", 0),
            ("+0.00003", 1),
            ("-0.00003", -1),
            ("0.00006", 2),
            ("0.00100", 33),
            ("-0.00704", -231),
            ("0.99996", 32_767),
            ("1", 32_768),
            ("-1", -32_768),
            ("1.000000", 32_768),
            ("1.00003", 32_769),
            ("-1.00003", -32_769),
            ("1000000000000.00003", 32_768_000_000_000_001),
            ("-281474976710655.99996", i64::MIN + 1),
            ("281474976710655.99996", i64::MAX),
        ] {
            let text = format!("effort={{stability={literal} war_support={literal}}}");
            let tree = content_reader_parser::parser::parse(text.as_bytes()).unwrap();
            assert_eq!(
                parse_propaganda_effort(&tree.items[0].value).unwrap(),
                PropagandaEffort {
                    stability: raw,
                    war_support: raw
                },
                "{literal}"
            );
        }
    }

    #[test]
    fn propaganda_fixed15_refuses_non_native_precision_overflow_and_missing_channels() {
        for literal in [
            "0.00001",
            "0.00004",
            "-0.00001",
            "0.000001",
            "1.000001",
            "281474976710656.00000",
            "-281474976710656.00000",
            "--281474976710656.00000",
            "NaN",
            "1e2",
            "1.",
        ] {
            for channel in ["stability", "war_support"] {
                let other = if channel == "stability" {
                    "war_support"
                } else {
                    "stability"
                };
                let text = format!("effort={{{channel}={literal} {other}=0}}");
                let tree = content_reader_parser::parser::parse(text.as_bytes()).unwrap();
                assert!(
                    parse_propaganda_effort(&tree.items[0].value).is_err(),
                    "{channel}={literal}"
                );
            }
        }
        for text in [
            b"effort={stability=0}".as_slice(),
            b"effort={war_support=0}",
        ] {
            let tree = content_reader_parser::parser::parse(text).unwrap();
            assert!(parse_propaganda_effort(&tree.items[0].value).is_err());
        }
    }

    #[test]
    fn propaganda_history_keeps_zero_occurrences_and_independent_loaded_drift() {
        let save = parsed(br#"countries={} strategic_operatives={strategic_operative={
            0={recent_propaganda_effort={{stability=0 war_support=0}{stability=-281474976710655.99996 war_support=281474976710655.99996}}
               propaganda_weekly_drift={stability=0.00051 war_support=-0.00057}}
            5000={}
        }}"#).unwrap();
        let objects = save.strategic_operatives.unwrap();
        assert_eq!(
            objects[&0].recent_propaganda_effort,
            [
                PropagandaEffort::default(),
                PropagandaEffort {
                    stability: i64::MIN + 1,
                    war_support: i64::MAX
                },
            ]
        );
        assert_eq!(
            objects[&0].propaganda_weekly_drift,
            PropagandaEffort {
                stability: 17,
                war_support: -19
            }
        );
        assert!(objects[&5000].recent_propaganda_effort.is_empty());
        assert_eq!(
            objects[&5000].propaganda_weekly_drift,
            PropagandaEffort::default()
        );
    }
    use super::*;

    fn parsed(input: &[u8]) -> io::Result<Pack> {
        parse(&content_reader_parser::parser::parse(input)?.items)
    }

    #[test]
    fn captured_references_preserve_order_nulls_duplicates_and_unsigned_words() {
        let pack = parsed(
            br#"countries={AAA={intelligence_agency={
            defense=0 max_operative_count=0 usable_operative_slots=0
            captured={
                {country=BBB operative={id=4294967295 type=4713} intel={}}
                {operative={id=0 type=0} intel={}}
                {country=CCC operative={id=4294967295 type=4713} intel={}}
            }
        }} BBB={intelligence_agency={defense=0 max_operative_count=0 usable_operative_slots=0}}}"#,
        )
        .unwrap();
        let captured = &pack.countries[b"AAA".as_slice()]
            .agency
            .as_ref()
            .unwrap()
            .captured;
        assert_eq!(
            captured
                .iter()
                .map(|entry| entry.operative)
                .collect::<Vec<_>>(),
            [
                (u64::from(u32::MAX) << 32) | 4713,
                0,
                (u64::from(u32::MAX) << 32) | 4713,
            ]
        );
        assert_eq!(captured[0].country.as_deref(), Some(b"BBB".as_slice()));
        assert!(captured[1].country.is_none());
        assert!(
            pack.countries[b"BBB".as_slice()]
                .agency
                .as_ref()
                .unwrap()
                .captured
                .is_empty()
        );
        for captured in [
            "{}",
            "{bad={operative={id=1 type=4713}}}",
            "{{}}",
            "{{operative={id=1}}}",
        ] {
            let input = format!(
                "countries={{AAA={{intelligence_agency={{defense=0 max_operative_count=0 usable_operative_slots=0 captured={captured}}}}}}}"
            );
            assert!(parsed(input.as_bytes()).is_err(), "{captured}");
        }
    }

    const INCOMING: &[u8] = br#"countries={GER={incoming_diplomatic_action={
        0={
            id={ id=63179 type=4713 }
            call_allies={
                type=0 actor="BUL" original_actor="BUL" recipient="GER"
                original_recipient="GER" date="1942.9.21.2" value=yes
                last_command_date="1942.9.21.3" envoy=-1 initiator_matters=no
                versus={ "ENG" "FRA" }
            }
        }
        2={
            id={ id=65511 type=4713 }
            join_faction={
                type=-1 actor="ITA" original_actor="ITA" recipient="GER"
                original_recipient="GER" date="1942.10.19.2" value=yes
                last_command_date="1942.10.19.2" envoy=-1 initiator_matters=no
            }
        }
    }} SOV={incoming_diplomatic_action={}} D01={}}"#;

    #[test]
    fn incoming_actions_keep_queue_indices_kind_names_and_signed_type() {
        let pack = parsed(INCOMING).unwrap();
        let actions = pack.countries[b"GER".as_slice()]
            .incoming_actions
            .as_ref()
            .unwrap();
        assert_eq!(actions.len(), 2);
        assert_eq!(actions[0].ordinal, 0);
        assert_eq!(actions[0].id, (63179 << 32) | 4713);
        assert_eq!(actions[0].kind, b"call_allies");
        assert_eq!(actions[0].action_type, 0);
        assert_eq!(actions[0].initiator_matters, Some(false));
        assert_eq!(actions[0].actor, b"BUL");
        assert_eq!(actions[0].recipient, b"GER");
        assert_eq!(
            actions[0].last_command_date.engine_hour().unwrap(),
            actions[0].date.engine_hour().unwrap() + 1
        );
        assert_eq!(actions[1].ordinal, 2);
        assert_eq!(actions[1].kind, b"join_faction");
        assert_eq!(actions[1].action_type, -1);
        assert_eq!(
            pack.countries[b"SOV".as_slice()].incoming_actions,
            Some(Vec::new())
        );
        assert_eq!(pack.countries[b"D01".as_slice()].incoming_actions, None);
    }

    #[test]
    fn incoming_initiator_flag_preserves_true_false_and_missing_independently_of_type() {
        for (replacement, expected) in [
            ("initiator_matters=yes", Some(true)),
            ("initiator_matters=no", Some(false)),
            ("", None),
        ] {
            let text = String::from_utf8(INCOMING.to_vec())
                .unwrap()
                .replace("initiator_matters=no", replacement);
            let pack = parsed(text.as_bytes()).unwrap();
            let action = &pack.countries[b"GER".as_slice()]
                .incoming_actions
                .as_ref()
                .unwrap()[0];
            assert_eq!(action.action_type, 0);
            assert_eq!(action.initiator_matters, expected);
        }
        let text = String::from_utf8(INCOMING.to_vec())
            .unwrap()
            .replace("initiator_matters=no", "initiator_matters=1");
        assert!(parsed(text.as_bytes()).is_err());
    }

    #[test]
    fn a_malformed_incoming_action_is_refused_rather_than_partly_read() {
        let text = String::from_utf8(INCOMING.to_vec()).unwrap();
        for broken in [
            text.replace("0={\n            id={ id=63179 type=4713 }", "0={"),
            text.replace("actor=\"BUL\"", ""),
            text.replace("type=0 ", ""),
            text.replace("date=\"1942.9.21.2\"", ""),
            text.replace(
                "versus={ \"ENG\" \"FRA\" }\n            }",
                "versus={ \"ENG\" }\n            } join_faction={ type=0 }",
            ),
            text.replace("2={", "0={"),
        ] {
            assert!(parsed(broken.as_bytes()).is_err());
        }
    }

    #[test]
    fn agency_keeps_qword_precision_signed_dwords_and_active_membership_count() {
        let pack = parsed(
            br#"countries={GER={intelligence_agency={
            defense=90071992547.40993 max_operative_count=-1 usable_operative_slots=-2147483648
            operative={id={id=4294967295 type=4713}} operative={id={id=4294967295 type=4713}}
        }} FRA={}}"#,
        )
        .unwrap();
        let agency = pack.countries[b"GER".as_slice()].agency.as_ref().unwrap();
        assert_eq!(agency.defense.raw, 9_007_199_254_740_993);
        assert_eq!(agency.upgrade_progress.raw, 0);
        assert_eq!((agency.active_count, agency.unknown_f0), (2, u32::MAX));
        assert_eq!(agency.admission_ceiling, i32::MIN);
        assert!(pack.countries[b"FRA".as_slice()].agency.is_none());
        assert!(parsed(b"countries={GER={intelligence_agency={max_operative_count=1 usable_operative_slots=2}}}").is_err());
    }

    #[test]
    fn agency_upgrade_save_entries_keep_order_duplicates_and_raw_signed_levels() {
        let pack = parsed(br#"countries={
            XAA={intelligence_agency={defense=0 max_operative_count=0 usable_operative_slots=0
                upgrades={mod_upgrade=3 other_upgrade=-2147483648 mod_upgrade=0}
            }}
            XAB={intelligence_agency={is_created=yes defense=0 max_operative_count=0 usable_operative_slots=0}}
            XAC={intelligence_agency={defense=0 max_operative_count=0 usable_operative_slots=0}}
            XAD={}
        }"#).unwrap();
        let entries = &pack.countries[b"XAA".as_slice()]
            .agency
            .as_ref()
            .unwrap()
            .upgrades;
        assert_eq!(
            entries,
            &vec![
                AgencyUpgradeLevel {
                    token: b"mod_upgrade".to_vec(),
                    level: 3
                },
                AgencyUpgradeLevel {
                    token: b"other_upgrade".to_vec(),
                    level: i32::MIN
                },
                AgencyUpgradeLevel {
                    token: b"mod_upgrade".to_vec(),
                    level: 0
                },
            ]
        );
        for tag in [b"XAB", b"XAC"] {
            assert!(
                pack.countries[tag.as_slice()]
                    .agency
                    .as_ref()
                    .unwrap()
                    .upgrades
                    .is_empty()
            );
        }
        assert!(pack.countries[b"XAD".as_slice()].agency.is_none());
    }

    #[test]
    fn agency_upgrade_save_malformed_entries_are_named_refusals() {
        for body in [
            "",
            "1",
            "bad_upgrade={x=1}",
            "bad_upgrade=2147483648",
            "bad_upgrade=-2147483649",
            "bad_upgrade>2",
            "\"\"=1",
        ] {
            let text = format!(
                "countries={{XAA={{intelligence_agency={{defense=0 max_operative_count=0 usable_operative_slots=0 upgrades={{{body}}}}}}}}}"
            );
            let failure = parsed(text.as_bytes()).unwrap_err().to_string();
            assert!(failure.contains("upgrade"), "{body}: {failure}");
            if body.starts_with("bad_upgrade") {
                assert!(failure.contains("bad_upgrade"), "{failure}");
            }
        }
    }

    #[test]
    fn operation_outcome_save_fields_keep_omission_and_description_bytes() {
        let pack = parsed(br#"countries={XAA={operations={running={
            mod_operation={id={id=1 type=8} target=XAB completed=yes outcome_extra=yes risk_extra=yes looted="line\\n\"quote\"\\literal"}
            mod_operation={id={id=2 type=8} target=XAB}
            mod_operation={id={id=3 type=8} target=XAB completed=yes looted=""}
        }}} XAB={}}"#).unwrap();
        let operations = pack.countries[b"XAA".as_slice()]
            .operations
            .as_ref()
            .unwrap()
            .running
            .as_ref()
            .unwrap();
        assert!(operations[0].terminal && operations[0].bonus && operations[0].detected);
        assert_eq!(
            operations[0].effect_description.as_deref(),
            Some(b"line\\n\"quote\"\\literal".as_slice())
        );
        assert!(!operations[1].terminal && !operations[1].bonus && !operations[1].detected);
        assert_eq!(operations[1].effect_description, None);
        assert_eq!(operations[2].effect_description, Some(vec![]));
        for malformed in [
            "outcome_extra=maybe",
            "risk_extra={}",
            "completed=yes completed=yes",
            "looted={}",
        ] {
            let text = format!(
                "countries={{XAA={{operations={{running={{mod_operation={{id={{id=1 type=8}} target=XAB {malformed}}}}}}}}}}}"
            );
            assert!(parsed(text.as_bytes()).is_err(), "{malformed}");
        }
    }

    #[test]
    fn scoped_subject_fields_and_omitted_envelopes_are_distinct() {
        let pack = parsed(
            br#"countries={GER={diplomacy={active_relations={FRA={puppet={
            first=GER second=FRA autonomy_state="puppet exact" value=-0.00001 start_date=1936.1.1.1
        }}}}} FRA={diplomacy={active_relations={}}}}"#,
        )
        .unwrap();
        let subject = &pack.countries[b"GER".as_slice()]
            .diplomacy
            .as_ref()
            .unwrap()
            .subjects
            .as_ref()
            .unwrap()[0];
        assert_eq!(subject.autonomy_ratio.raw, -1);
        assert_eq!(subject.state.as_deref(), Some(b"puppet exact".as_slice()));
        assert_eq!(subject.roster_owner, b"GER");
        assert_eq!(subject.target, b"FRA");
        assert!(subject.end_date.is_none());
        assert!(parsed(b"countries={GER={diplomacy={active_relations={FRA={puppet={first=GER second=FRA cancel=yes start_date=1936.1.1.1}}}}}}").is_err());
    }

    #[test]
    fn flags_keep_signed_words_literal_bytes_and_duration_presence() {
        let pack = parsed(b"countries={GER={flags={\"a\\x\"={value=-32768 date=1936.1.1.1} z={value=32767 date=1936.1.2.1 days=32767}}} FRA={flags={}} ENG={}}").unwrap();
        let flags = pack.countries[b"GER".as_slice()].flags.as_ref().unwrap();
        assert_eq!(flags[0].name, b"a\\x");
        assert_eq!(flags[0].value, i16::MIN);
        assert_eq!(flags[0].remaining_days, None);
        assert_eq!(flags[1].remaining_days, Some(i16::MAX));
        assert_eq!(pack.countries[b"FRA".as_slice()].flags, Some(vec![]));
        assert!(pack.countries[b"ENG".as_slice()].flags.is_none());
        assert!(parsed(b"countries={GER={flags={x={value=1 date=1936.1.1.1 days=0}}}}").is_err());
        assert!(parsed(b"countries={GER={flags={x={value=32768 date=1936.1.1.1}}}}").is_err());
        let repeated = parsed(b"countries={GER={flags={x={value=-1 date=1936.1.1.1} x={value=2 date=1936.1.2.1 days=3}}}}").unwrap();
        let flags = repeated.countries[b"GER".as_slice()]
            .flags
            .as_ref()
            .unwrap();
        assert_eq!(flags.len(), 2);
        assert_eq!(flags[0].name, flags[1].name);
        assert_eq!((flags[0].value, flags[1].value), (-1, 2));
    }

    #[test]
    fn operation_token_save_order_and_multiplicity_are_preserved() {
        let pack = parsed(b"countries={GER={tokens={operation_assets={FRA={z a a}}}}}").unwrap();
        let targets = pack.countries[b"GER".as_slice()]
            .operation_tokens
            .as_ref()
            .unwrap()
            .operation_assets
            .as_ref()
            .unwrap();
        assert_eq!(targets[0].target, b"FRA");
        assert_eq!(
            targets[0].names,
            [b"z".to_vec(), b"a".to_vec(), b"a".to_vec()]
        );
        assert!(parsed(b"countries={GER={tokens={operation_assets={FRA={}}}}}").is_err());
        assert!(parsed(b"countries={GER={tokens={operation_assets={FRA={x} FRA={y}}}}}").is_err());
    }

    #[test]
    fn conditional_payloads_keep_missing_empty_and_inactive_values_distinct() {
        let pack = parsed(b"countries={GER={tokens={}} FRA={tokens={operation_assets={}} diplomacy={exile_army_leaders=-2147483648}} ENG={diplomacy={capitulated=yes capitulated_date=1936.1.1.1 legitimacy=-0.00001 exile_army_leaders=0}}}").unwrap();
        assert!(
            pack.countries[b"GER".as_slice()]
                .operation_tokens
                .as_ref()
                .unwrap()
                .operation_assets
                .is_none()
        );
        assert_eq!(
            pack.countries[b"FRA".as_slice()]
                .operation_tokens
                .as_ref()
                .unwrap()
                .operation_assets,
            Some(vec![])
        );
        let inactive = pack.countries[b"FRA".as_slice()]
            .diplomacy
            .as_ref()
            .unwrap();
        assert_eq!(inactive.capitulated, None);
        assert_eq!(inactive.capitulated_date, None);
        assert_eq!(inactive.legitimacy, None);
        assert_eq!(inactive.exile_army_leaders, Some(i32::MIN));
        let active = pack.countries[b"ENG".as_slice()]
            .diplomacy
            .as_ref()
            .unwrap();
        assert_eq!(active.capitulated, Some(true));
        assert_eq!(active.legitimacy.as_ref().unwrap().raw, -1);
        assert!(parsed(b"countries={GER={diplomacy={capitulated=no}}}").is_err());
        assert!(parsed(b"countries={GER={diplomacy={capitulated=yes}}}").is_err());
        assert!(parsed(b"countries={GER={diplomacy={capitulated_date=1936.1.1.1}}}").is_err());
    }

    #[test]
    fn variables_preserve_repeated_literal_keys_and_exact_signed_values() {
        let pack = parsed(b"countries={GER={variables={random=1 a=90071992547.40993 a=-0.00001 \"\xff\"=-1.00001 \"\"=0 random=2}} FRA={variables={}} ENG={}}").unwrap();
        let variables = pack.countries[b"GER".as_slice()]
            .variables
            .as_ref()
            .unwrap();
        assert_eq!(variables.reserved_entries, 2);
        assert_eq!(variables.entries.len(), 4);
        assert_eq!(variables.entries[0].name, b"a");
        assert_eq!(variables.entries[1].name, b"a");
        assert_eq!(variables.entries[0].value.raw, 9_007_199_254_740_993);
        assert_eq!(variables.entries[1].value.raw, -1);
        assert_eq!(variables.entries[2].name, b"\xff");
        assert_eq!(variables.entries[2].value.raw, -100_001);
        assert!(variables.entries[3].name.is_empty());
        assert_eq!(
            pack.countries[b"FRA".as_slice()].variables,
            Some(Variables::default())
        );
        assert!(pack.countries[b"ENG".as_slice()].variables.is_none());
        assert!(parsed(b"countries={GER={variables={a={value=1}}}}").is_err());
    }

    #[test]
    fn variable_seed_metadata_preserves_the_second_unsigned_word_and_scalar_collisions() {
        let pack = parsed(
            b"variables={random={2147483648 4294967295}} countries={GER={variables={random={99 0} random=1.5}}} states={1={variables={random={1 2147483648}}}}",
        )
        .unwrap();
        assert_eq!(pack.global_variables.as_ref().unwrap().seed, Some(u32::MAX));
        let country = pack.countries[b"GER".as_slice()]
            .variables
            .as_ref()
            .unwrap();
        assert_eq!(country.seed, Some(0));
        assert_eq!(country.reserved_entries, 1);
        assert!(country.entries.is_empty());
        let state = pack.state_variables.as_ref().unwrap()[&1].as_ref().unwrap();
        assert_eq!(state.seed, Some(0x8000_0000));
        for metadata in [
            "random={}",
            "random={1}",
            "random={1 2 3}",
            "random={-1 2}",
            "random={1 4294967296}",
            "random={x=1 2}",
            "random={1 2} random={3 4}",
        ] {
            let save = format!("countries={{GER={{variables={{{metadata}}}}}}}");
            assert!(parsed(save.as_bytes()).is_err(), "{metadata}");
        }
    }

    #[test]
    fn variables_preserve_global_state_and_payload_scope_presence() {
        let pack = parsed(b"variables={same=-0.00001} countries={GER={variables={same=0.00002}}} states={0={variables={same=90071992547.40993}} 4294967295={variables={}} 7={}}").unwrap();
        assert_eq!(
            pack.global_variables.as_ref().unwrap().entries[0].value.raw,
            -1
        );
        assert_eq!(
            pack.countries[b"GER".as_slice()]
                .variables
                .as_ref()
                .unwrap()
                .entries[0]
                .value
                .raw,
            2
        );
        let states = pack.state_variables.as_ref().unwrap();
        assert_eq!(
            states[&0].as_ref().unwrap().entries[0].value.raw,
            9_007_199_254_740_993
        );
        assert_eq!(states[&u32::MAX], Some(Variables::default()));
        assert_eq!(states[&7], None);
        let absent = parsed(b"countries={}").unwrap();
        assert!(absent.global_variables.is_none());
        assert!(absent.state_variables.is_none());
        let empty = parsed(b"variables={} countries={} states={}").unwrap();
        assert_eq!(empty.global_variables, Some(Variables::default()));
        assert_eq!(empty.state_variables, Some(BTreeMap::new()));
        assert!(parsed(b"countries={} states={1={} 01={}}").is_err());
    }

    const OPERATIONS: &[u8] = br#"countries={
        GER={operations={ finished={} priority=1 }}
        ENG={operations={
            finished={ operation_infiltrate_armed_forces_airforce={ SOV 1 } }
            priority=2
            running={
                operation_rescue_operative={
                    id={ id=69023 type=4713 }
                    date="1942.12.6.10" duration=68 target="SOV"
                    operative_slots={
                        { operative={ id=9831 type=4713 } mission={ mission=build_intel_network target_country="SOV" target_state=219 } }
                    }
                    phases={ "infiltration_border" "free_operative_liberate_camp" }
                    resources={ infantry_equipment=180 }
                }
            }
        }}
        SOV={}
    }"#;

    #[test]
    fn operations_keep_container_shapes_running_census_and_slot_presence() {
        let pack = parsed(OPERATIONS).unwrap();
        let ger = pack.countries[b"GER".as_slice()]
            .operations
            .as_ref()
            .unwrap();
        assert_eq!(ger.priority, Some(1));
        assert_eq!(ger.finished, Some(Vec::new()));
        assert_eq!(ger.running, None);
        let eng = pack.countries[b"ENG".as_slice()]
            .operations
            .as_ref()
            .unwrap();
        assert_eq!(eng.priority, Some(2));
        let finished = eng.finished.as_ref().unwrap();
        assert_eq!(finished.len(), 1);
        assert_eq!(
            finished[0].key.as_deref(),
            Some(b"operation_infiltrate_armed_forces_airforce".as_slice())
        );
        assert_eq!(block(&finished[0].value).unwrap().len(), 2);
        let running = eng.running.as_ref().unwrap();
        assert_eq!(running.len(), 1);
        assert_eq!(running[0].definition, b"operation_rescue_operative");
        assert_eq!(running[0].id, (69023 << 32) | 4713);
        assert_eq!(running[0].target, b"SOV");
        assert_eq!(running[0].target_province, None);
        assert_eq!(
            running[0].date.unwrap().engine_hour().unwrap(),
            GameDate::parse(b"1942.12.6.10")
                .unwrap()
                .engine_hour()
                .unwrap()
        );
        assert_eq!(running[0].duration, Some(68));
        assert_eq!(running[0].phases.as_ref().unwrap().len(), 2);
        let slots = running[0].operative_slots.as_ref().unwrap();
        assert_eq!(slots.len(), 1);
        assert_eq!(slots[0].operative, Some((9831 << 32) | 4713));
        assert_eq!(slots[0].mission.as_ref().map(Vec::len), Some(3));
        assert!(pack.countries[b"SOV".as_slice()].operations.is_none());
    }

    #[test]
    fn operations_target_province_reads_the_saved_target_provinces_id() {
        let pack = parsed(
            br#"countries={ENG={operations={ running={ operation_boost_resistance={
                id={ id=87520 type=4713 } date="1942.12.27.15" duration=97
                target="GER" target_provinces=9434
            } } }}}"#,
        )
        .unwrap();
        let running = pack.countries[b"ENG".as_slice()]
            .operations
            .as_ref()
            .unwrap()
            .running
            .as_ref()
            .unwrap();
        assert_eq!(running[0].target_province, Some(9434));
    }

    #[test]
    fn operations_omitted_optionals_mean_zero_counts_and_null_refs() {
        let pack = parsed(
            br#"countries={GER={operations={
                priority=1 running={ operation_x={ id={ id=1 type=4713 } target="FRA" } }
            }}}"#,
        )
        .unwrap();
        let running = pack.countries[b"GER".as_slice()]
            .operations
            .as_ref()
            .unwrap()
            .running
            .as_ref()
            .unwrap();
        assert_eq!(running[0].date, None);
        assert_eq!(running[0].duration, None);
        assert_eq!(running[0].phases, None);
        assert_eq!(running[0].operative_slots, None);
        assert_eq!(running[0].target_province, None);
        let pack = parsed(
            br#"countries={GER={operations={
                running={ operation_x={
                    id={ id=1 type=4713 } target="FRA"
                    operative_slots={ { mission={ mission=build_intel_network } } { } }
                } }
            }}}"#,
        )
        .unwrap();
        let slots = pack.countries[b"GER".as_slice()]
            .operations
            .as_ref()
            .unwrap()
            .running
            .as_ref()
            .unwrap()[0]
            .operative_slots
            .as_ref()
            .unwrap();
        assert_eq!(slots.len(), 2);
        assert_eq!(slots[0].operative, None);
        assert_eq!(slots[0].mission.as_ref().map(Vec::len), Some(1));
        assert_eq!(slots[1].operative, None);
        assert_eq!(slots[1].mission, None);
    }

    #[test]
    fn operation_slot_missions_distinguish_different_valid_blocks_empty_and_absent() {
        let pack = parsed(
            br#"countries={GER={operations={
                running={ operation_x={
                    id={ id=1 type=4713 } target="FRA"
                    operative_slots={
                        { mission={ mission=build_intel_network target_country="SOV" target_state=219 } }
                        { mission={ mission=boost_ideology target_country="FRA" } }
                        { mission={} }
                        { }
                    }
                } }
            }}}"#,
        )
        .unwrap();
        let slots = pack.countries[b"GER".as_slice()]
            .operations
            .as_ref()
            .unwrap()
            .running
            .as_ref()
            .unwrap()[0]
            .operative_slots
            .as_ref()
            .unwrap();
        assert_eq!(slots.len(), 4);
        assert_ne!(slots[0], slots[1]);
        assert_ne!(slots[0], slots[2]);
        assert_ne!(slots[1], slots[2]);
        assert_ne!(slots[2], slots[3]);
        let first = slots[0].mission.as_deref().unwrap();
        assert_eq!(first.len(), 3);
        assert_eq!(
            bytes(first, b"mission").unwrap().as_deref(),
            Some(b"build_intel_network".as_slice())
        );
        assert_eq!(
            bytes(first, b"target_country").unwrap().as_deref(),
            Some(b"SOV".as_slice())
        );
        assert_eq!(
            bytes(first, b"target_state").unwrap().as_deref(),
            Some(b"219".as_slice())
        );
        let second = slots[1].mission.as_deref().unwrap();
        assert_eq!(second.len(), 2);
        assert_eq!(
            bytes(second, b"mission").unwrap().as_deref(),
            Some(b"boost_ideology".as_slice())
        );
        assert_eq!(
            bytes(second, b"target_country").unwrap().as_deref(),
            Some(b"FRA".as_slice())
        );
        assert_eq!(slots[2].mission, Some(Vec::new()));
        assert_eq!(slots[3].mission, None);
        assert!(
            parsed(
                br#"countries={GER={operations={running={operation_x={id={id=1 type=4713} target="FRA" operative_slots={{mission=not_a_block}}}}}}}"#
            )
            .is_err()
        );
    }

    #[test]
    fn repeated_operation_definitions_preserve_distinct_ids_targets_and_order() {
        let source = br#"countries={JAP={operations={ running={
            operation_rescue_operative={ id={ id=10 type=4713 } target="FRA" duration=68 }
            operation_rescue_operative={ id={ id=20 type=4713 } target="CHI" duration=35 }
        } }}}"#;
        let pack = parsed(source).unwrap();
        let running = pack.countries[b"JAP".as_slice()]
            .operations
            .as_ref()
            .unwrap()
            .running
            .as_ref()
            .unwrap();
        assert_eq!(running.len(), 2);
        assert_eq!(running[0].definition, b"operation_rescue_operative");
        assert_eq!(running[1].definition, running[0].definition);
        assert_eq!(running[0].id, (10 << 32) | 4713);
        assert_eq!(running[1].id, (20 << 32) | 4713);
        assert_eq!(running[0].target, b"FRA");
        assert_eq!(running[1].target, b"CHI");
        assert_eq!(running[0].duration, Some(68));
        assert_eq!(running[1].duration, Some(35));
        let duplicate = String::from_utf8(source.to_vec())
            .unwrap()
            .replace("id=20", "id=10");
        assert!(
            parsed(duplicate.as_bytes())
                .unwrap_err()
                .to_string()
                .contains("duplicate running operation id")
        );
    }

    #[test]
    fn malformed_operation_blocks_are_refused_rather_than_partly_read() {
        for broken in [
            "countries={GER={operations={ running={} }}}",
            "countries={GER={operations={ running={ operation_x={ id={ id=1 type=4713 } target=\"FRA\" phases={} } } }}}",
            "countries={GER={operations={ running={ operation_x={ id={ id=1 type=4713 } target=\"FRA\" operative_slots={} } } }}}",
            "countries={GER={operations={ running={ operation_x={ id={ id=1 type=4713 } target=\"FRA\" } operation_y={ id={ id=1 type=4713 } target=\"FRA\" } } }}}",
            "countries={GER={operations={ running={ operation_x={ target=\"FRA\" } } }}}",
            "countries={GER={operations={ running={ operation_x={ id={ id=1 type=4713 } } } }}}",
            "countries={GER={operations={ running={ operation_x={ id={ id=1 type=4713 } target=\"FRA\" phases={ k=\"v\" } } } }}}",
            "countries={GER={operations={ running={ operation_x={ id={ id=1 type=4713 } target=\"FRA\" operative_slots={ k={} } } } }}}",
            "countries={GER={operations={ running={ operation_x={ id={ id=0 type=0 } target=\"FRA\" } } }}}",
        ] {
            assert!(parsed(broken.as_bytes()).is_err(), "{broken}");
        }
    }

    const OPERATIVE_AGENCY: &[u8] = br#"countries={GER={intelligence_agency={
        defense=0 max_operative_count=1 usable_operative_slots=1
        operative={ id={ id=9831 type=4713 } skill=2 experience=453.456 nationalities={ ENG } state=on_operation operation={ id=69023 type=4713 } }
        operative={ id={ id=29260 type=4713 } skill=1 captured="SOV" capture_date="1942.11.28.12" state=on_capture operation={ id=55674 type=4713 } }
        recruitment={ recruitable_operatives={ { id={ id=1043 type=4713 } skill=1 nationalities={ ENG } state=on_mission } } recruitable_operatives_not_to_spy_master={ { id={ id=5801 type=4713 } skill=1 mission={ build_intel_network={ target="ITA" state=4 } } state=on_mission } } }
        cryptology={ targets={ { target="SOV" decryption=yes amount=27625 } { target="ITA" amount=12000 date="1940.1.15.1" } } intel_source={ country="GER" pool=2 id=1 discriminant=0 } }
    }} FRA={intelligence_agency={ defense=0 max_operative_count=0 usable_operative_slots=0 }}}"#;

    #[test]
    fn operative_codename_preserves_member_omission_rules_and_signed_order() {
        let pack = parsed(
            br#"countries={XXX={intelligence_agency={
            defense=0 max_operative_count=1 usable_operative_slots=1
            operative={ id={ id=1 type=4713 } codename={ type=2 } }
            recruitment={ recruitable_operatives={ {
                id={ id=2 type=4713 }
                codename={ type=2 name_order=-1 is_name_ordered=no override="renamed" override_set_programmatically=yes }
            } } }
        }}}"#,
        )
        .unwrap();
        let agency = pack.countries[b"XXX".as_slice()].agency.as_ref().unwrap();
        let first = agency.operatives[0].codename.as_ref().unwrap();
        assert_eq!(first.kind, 2);
        assert_eq!(
            (first.order, first.ordered, first.programmatic),
            (None, None, None)
        );
        assert!(first.custom.is_none());
        let second = agency.recruitment.as_ref().unwrap().recruitable[0]
            .codename
            .as_ref()
            .unwrap();
        assert_eq!(
            (second.order, second.ordered, second.programmatic),
            (Some(-1), Some(false), Some(true))
        );
        assert_eq!(second.custom.as_deref(), Some(b"renamed".as_slice()));
        for malformed in [
            "type=2 name_order=1 name_order=2",
            "name_order=1",
            "type=2 is_name_ordered=maybe",
        ] {
            let source = format!(
                "countries={{XXX={{intelligence_agency={{defense=0 max_operative_count=0 usable_operative_slots=0 operative={{id={{id=1 type=4713}} codename={{{malformed}}}}}}}}}}}"
            );
            assert!(parsed(source.as_bytes()).is_err());
        }
    }

    #[test]
    fn operatives_keep_roster_lists_capturer_dates_and_action_presence() {
        let pack = parsed(OPERATIVE_AGENCY).unwrap();
        let agency = pack.countries[b"GER".as_slice()].agency.as_ref().unwrap();
        assert_eq!(agency.operatives.len(), 2);
        assert_eq!(agency.operatives[0].id, (9831 << 32) | 4713);
        assert_eq!(agency.operatives[0].skill, Some(2));
        assert_eq!(agency.operatives[0].nationalities, [b"ENG".to_vec()]);
        assert_eq!(
            agency.operatives[0].state.as_deref(),
            Some(b"on_operation".as_slice())
        );
        assert_eq!(agency.operatives[0].operation, Some((69023 << 32) | 4713));
        assert!(!agency.operatives[0].mission_present);
        assert_eq!(
            agency.operatives[1].captured.as_deref(),
            Some(b"SOV".as_slice())
        );
        assert!(agency.operatives[1].capture_date.is_some());
        let recruitment = agency.recruitment.as_ref().unwrap();
        assert_eq!(recruitment.recruitable.len(), 1);
        assert_eq!(recruitment.not_to_spy_master.len(), 1);
        assert!(recruitment.not_to_spy_master[0].mission_present);
        assert_eq!(
            recruitment.not_to_spy_master[0].mission_name.as_deref(),
            Some(b"build_intel_network".as_slice())
        );
        assert_eq!(
            recruitment.not_to_spy_master[0].mission_target.as_deref(),
            Some(b"ITA".as_slice())
        );
        let fra = pack.countries[b"FRA".as_slice()].agency.as_ref().unwrap();
        assert!(fra.operatives.is_empty());
        assert!(fra.recruitment.is_none());
        assert!(fra.cryptology.is_none());
        let crypto = agency.cryptology.as_ref().unwrap();
        let targets = crypto.targets.as_ref().unwrap();
        assert_eq!(targets.len(), 2);
        assert_eq!(targets[0].decryption, Some(true));
        assert!(targets[1].date.is_some());
        assert_eq!(
            crypto.intel_source_country.as_deref(),
            Some(b"GER".as_slice())
        );
        for broken in [
            "countries={GER={intelligence_agency={ defense=0 max_operative_count=0 usable_operative_slots=0 operative={ id={ id=1 type=4713 } captured=\"SOV\" } }}}",
            "countries={GER={intelligence_agency={ defense=0 max_operative_count=0 usable_operative_slots=0 cryptology={ targets={ { target=\"SOV\" } { target=\"SOV\" } } } }}}",
            "countries={GER={intelligence_agency={ defense=0 max_operative_count=0 usable_operative_slots=0 cryptology={ targets={} } }}}",
            "countries={GER={intelligence_agency={ defense=0 max_operative_count=0 usable_operative_slots=0 cryptology={ targets={ { target=\"SOV\" decryption=no } } } }}}",
        ] {
            assert!(parsed(broken.as_bytes()).is_err(), "{broken}");
        }
    }

    #[test]
    fn recruitment_retains_recorded_pool_spelling_and_repeated_memberships() {
        let pack = parsed(
            br#"countries={ X={intelligence_agency={
            defense=0 max_operative_count=0 usable_operative_slots=0
            recruitment={ recorded_pool_name={
                { id={id=7 type=4713} name="first" }
                { id={id=7 type=4713} name="first" }
            } another_saved_list={ { id={id=8 type=4713} } } }
        }} }"#,
        )
        .unwrap();
        let recruitment = pack.countries[b"X".as_slice()]
            .agency
            .as_ref()
            .unwrap()
            .recruitment
            .as_ref()
            .unwrap();
        assert!(recruitment.recruitable.is_empty());
        assert_eq!(recruitment.other_rosters.len(), 2);
        let (name, entries) = &recruitment.other_rosters[0];
        assert_eq!(name, b"recorded_pool_name");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0], entries[1]);
        assert_eq!(recruitment.other_rosters[1].0, b"another_saved_list");
        assert_eq!(recruitment.other_rosters[1].1[0].id, (8 << 32) | 4713);
    }

    #[test]
    fn operative_name_and_nested_mission_target_preserve_the_saved_shape() {
        let prefix = "countries={X={intelligence_agency={ defense=0 max_operative_count=1 usable_operative_slots=1 operative={ id={id=7 type=4713} name=\"Renamed literal\" ";
        let suffix = " } } } }";
        let pack = parsed(
            format!("{prefix}mission={{custom_mission={{target=Y state=123}}}}{suffix}").as_bytes(),
        )
        .unwrap();
        let row = &pack.countries[b"X".as_slice()]
            .agency
            .as_ref()
            .unwrap()
            .operatives[0];
        assert_eq!(row.name.as_deref(), Some(b"Renamed literal".as_slice()));
        assert_eq!(
            row.mission_name.as_deref(),
            Some(b"custom_mission".as_slice())
        );
        assert_eq!(row.mission_target.as_deref(), Some(b"Y".as_slice()));
        assert_eq!(row.mission_state, Some(123));
        assert_eq!(row.mission_ideology, None);
        let pack = parsed(
            format!(
                "{prefix}mission={{boost_ideology={{target=Y state=-4 ideology=fascism}}}}{suffix}"
            )
            .as_bytes(),
        )
        .unwrap();
        let row = &pack.countries[b"X".as_slice()]
            .agency
            .as_ref()
            .unwrap()
            .operatives[0];
        assert_eq!(row.mission_state, Some(-4));
        assert_eq!(row.mission_ideology.as_deref(), Some(b"fascism".as_slice()));
        assert_eq!(row.gfx, None);
        let pack = parsed(format!("{prefix}gfx=\"GFX_spy\"{suffix}").as_bytes()).unwrap();
        let row = &pack.countries[b"X".as_slice()]
            .agency
            .as_ref()
            .unwrap()
            .operatives[0];
        assert_eq!(row.gfx.as_deref(), Some(b"GFX_spy".as_slice()));
        assert!(parsed(format!("{prefix}gfx=\"\"{suffix}").as_bytes()).is_err());
        let pack =
            parsed(format!("{prefix}mission={{custom_mission={{}}}}{suffix}").as_bytes()).unwrap();
        let row = &pack.countries[b"X".as_slice()]
            .agency
            .as_ref()
            .unwrap()
            .operatives[0];
        assert!(row.mission_present);
        assert_eq!(row.mission_target, None);
        assert_eq!(
            (row.mission_state, row.mission_ideology.as_deref()),
            (None, None)
        );
        for mission in [
            "mission={}",
            "mission={custom_mission={target=Y target=Z}}",
            "mission={custom_mission={target=\"\"}}",
            "mission={a={target=Y} b={target=Z}}",
            "mission={mission=custom_mission target_country=Y target_state=123}",
            "mission={custom_mission={target=Y state=1 state=2}}",
            "mission={custom_mission={target=Y state=north}}",
            "mission={custom_mission={target=Y state=4294967296}}",
            "mission={custom_mission={ideology=a ideology=b}}",
            "mission={custom_mission={ideology=\"\"}}",
        ] {
            assert!(
                parsed(format!("{prefix}{mission}{suffix}").as_bytes()).is_err(),
                "{mission}"
            );
        }
    }

    const NETWORKS: &[u8] = br#"countries={ENG={} SOV={} AUS={}}
        strategic_operatives={ strategic_operative={
            2={ intel_network={ target="SOV" intel_network={ state_id={ 219 253 } gain={ 0.85 -1.75 } gain_breakdown={ { operatives=0.85 adjacencies=0 } { operatives=0 adjacencies=0.2125 } } strength={ 71 0 } sub_network_id={ 0 -1 } sub_network_is_quiet={ yes } } sub_intel_networks={ { modifiers={ intel_network_gain_factor=1 } strength=72.5 sub_network_strength_target=57.5 sub_network_strength_target_from_operatives=15 sub_network_strength_target_from_counterintelligence=-47.5 sub_network_is_quiet=yes } } } intel_network={ target="AUS" intel_network={ state_id={ 4 } gain={ 0 } gain_breakdown={ { operatives=0 adjacencies=0 } } strength={ 100 } sub_network_id={ 0 } } sub_intel_networks={ { modifiers={ intel_network_gain_factor=0.5 } strength=100 sub_network_strength_target=105 sub_network_strength_target_from_operatives=15 sub_network_strength_target_from_counterintelligence=0 } } } }
        }}"#;

    #[test]
    fn intel_networks_keep_positional_parallel_arrays_and_index_presence() {
        let pack = parsed(NETWORKS).unwrap();
        let operatives = pack.strategic_operatives.as_ref().unwrap();
        assert_eq!(operatives.len(), 1);
        let networks = &operatives[&2].networks;
        assert_eq!(networks.len(), 2);
        assert_eq!(networks[0].target, b"SOV");
        assert_eq!(networks[0].state_ids, [219, 253]);
        assert_eq!(networks[0].selectors, [0, -1]);
        let decimal = |text: &str| Decimal::parse(text.as_bytes()).unwrap();
        assert_eq!(networks[0].gains, [decimal("0.85"), decimal("-1.75")]);
        assert_eq!(
            networks[0].gains_from_operatives,
            [decimal("0.85"), decimal("0")]
        );
        assert_eq!(
            networks[0].gains_from_adjacency,
            [decimal("0"), decimal("0.2125")]
        );
        assert_eq!(networks[0].local_quiet, [true]);
        assert_eq!(
            networks[0].subnetworks,
            [SavedSubnetwork {
                strength: decimal("72.5"),
                gain_factor: decimal("1"),
                strength_target: decimal("57.5"),
                target_from_operatives: decimal("15"),
                target_from_counterintelligence: decimal("-47.5"),
                quiet: true,
            }]
        );
        assert!(networks[1].local_quiet.is_empty());
        assert!(!networks[1].subnetworks[0].quiet);
        assert_eq!(networks[1].subnetworks[0].gain_factor, decimal("0.5"));
        for broken in [
            "countries={ENG={}} strategic_operatives={ strategic_operative={ 0={ intel_network={ target=\"SOV\" intel_network={ state_id={ 1 2 } gain={ 0 0 } gain_breakdown={ { operatives=0 adjacencies=0 } { operatives=0 adjacencies=0 } } strength={ 1 } sub_network_id={ 0 0 } } } } }}",
            "countries={ENG={}} strategic_operatives={ strategic_operative={ 0={ intel_network={ target=\"SOV\" intel_network={ state_id={ 1 1 } gain={ 0 0 } gain_breakdown={ { operatives=0 adjacencies=0 } { operatives=0 adjacencies=0 } } strength={ 1 1 } sub_network_id={ 0 0 } } } } }}",
            "countries={ENG={}} strategic_operatives={ strategic_operative={ 0={ intel_network={ target=\"SOV\" intel_network={ state_id={ 1 } gain={ 0 } gain_breakdown={ { operatives=0 adjacencies=0 } } strength={ 1 } sub_network_id={ 0 } } } intel_network={ target=\"SOV\" intel_network={ state_id={ 2 } gain={ 0 } gain_breakdown={ { operatives=0 adjacencies=0 } } strength={ 1 } sub_network_id={ 0 } } } } }}",
            "countries={ENG={}} strategic_operatives={ strategic_operative={ 0={ intel_network={ target=\"SOV\" } } }}",
            "countries={ENG={}} strategic_operatives={ strategic_operative={ 0={ intel_network={ target=\"SOV\" intel_network={ state_id={ 1 2 } gain={ 0 } gain_breakdown={ { operatives=0 adjacencies=0 } { operatives=0 adjacencies=0 } } strength={ 1 1 } sub_network_id={ -1 -1 } } } } }}",
            "countries={ENG={}} strategic_operatives={ strategic_operative={ 0={ intel_network={ target=\"SOV\" intel_network={ state_id={ 1 } gain={ 0 } strength={ 1 } sub_network_id={ -1 } } } } }}",
            "countries={ENG={}} strategic_operatives={ strategic_operative={ 0={ intel_network={ target=\"SOV\" intel_network={ state_id={ 1 } gain={ 0 } gain_breakdown={ { operatives=0 adjacencies=0 } } strength={ 1 } sub_network_id={ 0 } } sub_intel_networks={ { modifiers={ intel_network_gain_factor=1 } strength=1 sub_network_strength_target=1 sub_network_strength_target_from_operatives=1 } } } } }}",
        ] {
            assert!(parsed(broken.as_bytes()).is_err(), "{broken}");
        }
        let absent = parsed(b"countries={ENG={}}").unwrap();
        assert!(absent.strategic_operatives.is_none());
    }
    #[test]
    fn operative_inherited_cooldown_fields_keep_reason_and_both_dates() {
        let items = content_reader_parser::parser::parse(
            br#"id={ id=27 type=4713 } state=on_cooldown
            cooldown_reason=forced_into_hiding leader_cooldown_start_date=1938.4.3.7
            leader_modifier_enable_date=1938.4.8.19"#,
        )
        .unwrap()
        .items;
        let entry = parse_operative_entry(&items).unwrap();
        assert_eq!(
            entry.cooldown_reason.as_deref(),
            Some(b"forced_into_hiding".as_slice())
        );
        assert_eq!(
            entry.cooldown_end_date.unwrap().engine_hour().unwrap()
                - entry.cooldown_start_date.unwrap().engine_hour().unwrap(),
            5 * 24 + 12
        );
    }

    #[test]
    fn strategic_operatives_keep_activity_and_both_influence_maps_in_saved_order() {
        let pack = parsed(
            br#"countries={ENG={} SOV={}}
            strategic_operatives={ strategic_operative={
                3={
                    subversive_activity_level={ value=1.23456 }
                    danger_level={ value=-0.5 }
                    trade_influence_modifier_of_buyers={ { "SOV" 2.5 } { "ENG" 0 } { "SOV" 1 } }
                    diplomatic_pressure_from_countries={ { "SOV" { opinion=32768 tension=-16384 } } }
                }
                4={ subversive_activity_level={ value=0 } danger_level={ value=0 } }
            } }"#,
        )
        .unwrap();
        let operatives = pack.strategic_operatives.as_ref().unwrap();
        let first = &operatives[&3];
        assert_eq!(
            (
                first.mission_activity.as_ref().map(|value| value.raw),
                first.operation_activity.as_ref().map(|value| value.raw)
            ),
            (Some(123_456), Some(-50_000))
        );
        let trade = first.trade_influence.as_ref().unwrap();
        assert_eq!(
            trade
                .iter()
                .map(|(country, amount)| (country.as_slice(), amount.raw))
                .collect::<Vec<_>>(),
            [
                (b"SOV".as_slice(), 250_000),
                (b"ENG".as_slice(), 0),
                (b"SOV".as_slice(), 100_000)
            ]
        );
        assert_eq!(
            first.diplomatic_pressure.as_deref(),
            Some(
                [SavedPressure {
                    country: b"SOV".to_vec(),
                    opinion: 32_768,
                    tension: -16_384,
                }]
                .as_slice()
            )
        );
        let second = &operatives[&4];
        assert_eq!(
            (&second.trade_influence, &second.diplomatic_pressure),
            (&None, &None)
        );
        assert_eq!(
            second.mission_activity.as_ref().map(|value| value.raw),
            Some(0)
        );
        let bare =
            parsed(b"countries={} strategic_operatives={ strategic_operative={ 0={} } }").unwrap();
        let bare = &bare.strategic_operatives.as_ref().unwrap()[&0];
        assert_eq!(
            (&bare.mission_activity, &bare.operation_activity),
            (&None, &None)
        );
        for body in [
            "trade_influence_modifier_of_buyers={}",
            "diplomatic_pressure_from_countries={}",
            "trade_influence_modifier_of_buyers={ { \"SOV\" } }",
            "trade_influence_modifier_of_buyers={ { \"SOV\" 1 2 } }",
            "trade_influence_modifier_of_buyers={ { \"\" 1 } }",
            "trade_influence_modifier_of_buyers={ SOV=1 }",
            "trade_influence_modifier_of_buyers={ { \"SOV\" many } }",
            "diplomatic_pressure_from_countries={ { \"SOV\" { opinion=1 } } }",
            "diplomatic_pressure_from_countries={ { \"SOV\" { opinion=1 tension=0.5 } } }",
            "diplomatic_pressure_from_countries={ { \"SOV\" 1 } }",
            "subversive_activity_level={ level=1 }",
            "danger_level=1",
        ] {
            assert!(
                parsed(
                    format!(
                        "countries={{}} strategic_operatives={{ strategic_operative={{ 0={{ {body} }} }} }}"
                    )
                    .as_bytes()
                )
                .is_err(),
                "{body}"
            );
        }
    }

    #[test]
    fn operation_preparation_resources_and_equipment_pool_keep_saved_order() {
        let pack = parsed(
            br#"countries={ENG={operations={ priority=1 running={ operation_x={
                id={ id=1 type=4713 } target="SOV"
                civilian_factories=1.5 total=12
                resources={ infantry_equipment=180 civilian_factories={ amount=2 days=30 } infantry_equipment=0.5 }
                equipment={ allow_zero_entries=yes
                    equipment={ id={ id=12 type=70 } amount=100 }
                    equipment={ id={ id=13 type=70 } amount=0 } }
            } } }} SOV={}}"#,
        )
        .unwrap();
        let running = &pack.countries[b"ENG".as_slice()]
            .operations
            .as_ref()
            .unwrap()
            .running
            .as_ref()
            .unwrap()[0];
        assert_eq!(
            (
                running.civilian_factories.as_ref().map(|value| value.raw),
                running.total.as_ref().map(|value| value.raw)
            ),
            (Some(150_000), Some(1_200_000))
        );
        assert_eq!(
            running.resources.as_deref(),
            Some(
                [
                    OperationResource::Equipment {
                        archetype: b"infantry_equipment".to_vec(),
                        amount: Decimal::parse(b"180").unwrap(),
                    },
                    OperationResource::Factories {
                        amount: 2,
                        days: 30
                    },
                    OperationResource::Equipment {
                        archetype: b"infantry_equipment".to_vec(),
                        amount: Decimal::parse(b"0.5").unwrap(),
                    },
                ]
                .as_slice()
            )
        );
        let pool = running.equipment.as_ref().unwrap();
        assert!(pool.allow_zero);
        assert_eq!(
            pool.entries
                .iter()
                .map(|entry| (entry.variant, entry.amount.raw))
                .collect::<Vec<_>>(),
            [((12 << 32) | 70, 10_000_000), ((13 << 32) | 70, 0)]
        );
        let omitted = parsed(
            br#"countries={ENG={operations={ priority=1 running={ operation_x={
                id={ id=1 type=4713 } target="SOV" } } }} SOV={}}"#,
        )
        .unwrap();
        let running = &omitted.countries[b"ENG".as_slice()]
            .operations
            .as_ref()
            .unwrap()
            .running
            .as_ref()
            .unwrap()[0];
        assert_eq!(
            (
                &running.civilian_factories,
                &running.total,
                &running.resources,
                &running.equipment
            ),
            (&None, &None, &None, &None)
        );
        for body in [
            "resources={ infantry_equipment=many }",
            "resources={ civilian_factories={ amount=2 } }",
            "resources={ civilian_factories={ amount=1.5 days=3 } }",
            "resources={ other={ amount=1 days=1 } }",
            "resources={ 1 }",
            "equipment={ equipment={ id={ id=12 type=70 } amount=1 } }",
            "equipment={ allow_zero_entries=no equipment={ id={ id=12 type=70 } amount=0 } }",
            "equipment={ allow_zero_entries=yes equipment={ amount=1 } }",
            "equipment={ allow_zero_entries=yes equipment={ id={ id=12 type=70 } } }",
            "civilian_factories=lots",
            "total=1 total=2",
        ] {
            assert!(
                parsed(
                    format!(
                        "countries={{ENG={{operations={{ priority=1 running={{ operation_x={{ id={{ id=1 type=4713 }} target=\"SOV\" {body} }} }} }}}} SOV={{}}}}"
                    )
                    .as_bytes()
                )
                .is_err(),
                "{body}"
            );
        }
    }
}
