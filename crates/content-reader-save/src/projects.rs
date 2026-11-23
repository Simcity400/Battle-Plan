use super::{
    Decimal, GameDate, SavedDate, block, boolean, bytes, field, integer, invalid, required_block,
    required_bytes, scalar, typed_handle,
};
use content_reader_parser::parser::Item;
use std::{collections::BTreeMap, io};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pack {
    pub countries: BTreeMap<Vec<u8>, Country>,
    pub raids: Option<Vec<RaidCountry>>,
    pub raid_targets: Option<Vec<RaidTargetStatus>>,
    pub conferences: Option<BTreeMap<u64, Conference>>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Country {
    pub nukes: Option<Vec<Nuke>>,
    pub status: Option<ProgramStatus>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Nuke {
    pub amount: Decimal,
    pub ready: i32,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProgramStatus {
    pub programs: BTreeMap<u64, Program>,
    pub program_order: Vec<u64>,
    pub projects: BTreeMap<u64, Project>,
    pub breakthroughs: BTreeMap<Vec<u8>, i32>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Program {
    pub id: u64,
    pub country: Option<Vec<u8>>,
    pub province: u32,
    pub active_project: Option<Vec<u8>>,
    pub scientist: Option<u64>,
    pub building: Vec<u8>,
    pub unread_rewards: i32,
    pub assignments: Vec<ScientistAssignment>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScientistAssignment {
    pub scientist: u64,
    pub target: i32,
    pub current: i32,
    pub timed: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Project {
    pub id: u64,
    pub definition: Vec<u8>,
    pub program: Option<u64>,
    pub started: bool,
    pub completed: bool,
    pub current_state: i32,
    pub prototype_phase_progress: Decimal,
    pub prototype_progress: i32,
    pub iterations: i32,
    pub stopping_phase_progress: Decimal,
    pub history: Vec<History>,
    pub pending_reward: Option<Vec<u8>>,
    pub variables: super::intel::Variables,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct History {
    pub reward: Vec<u8>,
    pub option: Vec<u8>,
    pub date: GameDate,
    pub context: Context,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Context {
    pub province: i32,
    pub progress: i32,
    pub character: Option<u64>,
    pub country: Option<u32>,
    pub scientist: Option<u32>,
    pub state: Option<u32>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RaidCountry {
    pub ordinal: u32,
    pub country: Option<Vec<u8>>,
    pub instances: Vec<Raid>,
    pub used_types: Vec<Vec<u8>>,
    pub cooldowns: Vec<RaidCooldown>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RaidCooldown {
    pub raid_type: Vec<u8>,
    pub target: RaidTarget,
    pub days: i32,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RaidTargetStatus {
    pub valid: bool,
    pub target: RaidTarget,
    pub detected: Vec<u8>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Raid {
    pub id: u64,
    pub phase: i32,
    pub outcome: i32,
    pub show_for: Vec<Vec<u8>>,
    pub source: Option<RaidSource>,
    pub raid_type: Option<Vec<u8>>,
    pub detected: bool,
    pub target: Option<RaidTarget>,
    pub army: Option<u64>,
    pub air_wing: Option<u64>,
    pub travelled: Option<Decimal>,
    pub preparation_hours: i32,
    pub equipment: Option<super::production::Pool>,
    pub nukes: i32,
    pub end_date: Option<SavedDate>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RaidTarget {
    pub province: Option<u32>,
    pub state: Option<u32>,
    pub leader_province: Option<u32>,
    pub building_template: Option<Vec<u8>>,
    pub building_location: Option<i64>,
    pub leader: Option<u64>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RaidSource {
    pub kind: u32,
    pub province: Option<u32>,
    pub ship: Option<u64>,
    pub country: Option<Vec<u8>>,
    pub building_template: Option<Vec<u8>>,
    pub building_location: Option<i64>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conference {
    pub id: u64,
    pub ordinal: u32,
    pub finalized: bool,
    pub history_turns: BTreeMap<Vec<u8>, i32>,
    pub bids: BTreeMap<Vec<u8>, Vec<PeaceBid>>,
    pub bid_countries: Vec<Vec<u8>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PeaceBid {
    pub country: Option<Vec<u8>>,
    pub actions: Vec<PeaceAction>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PeaceAction {
    pub token: Vec<u8>,
    pub name: Option<Vec<u8>>,
    pub taker: Option<Vec<u8>>,
    pub giver: Option<Vec<u8>>,
    pub negotiator: Option<Vec<u8>>,
    pub status: i32,
    pub cost: i32,
    pub state: Option<i32>,
    pub flags: [bool; 4],
    pub puppet: Option<Vec<u8>>,
    pub liberate: Option<Vec<u8>>,
    pub ship: Option<u64>,
    pub ratio: Option<Decimal>,
}

fn peace_bid(items: &[Item]) -> io::Result<PeaceBid> {
    let mut actions = Vec::new();
    if let Some(value) = field(items, b"actions")? {
        for item in block(value)? {
            let token = item
                .key
                .clone()
                .ok_or_else(|| invalid("peace action lacks token"))?;
            let fields = block(&item.value)?;
            let mut flags = [false; 4];
            for (flag, key) in flags.iter_mut().zip([
                b"demilitarized_zone".as_slice(),
                b"war_reparation",
                b"resource_rights",
                b"dismantle_industry",
            ]) {
                *flag = field(fields, key)?
                    .map(boolean)
                    .transpose()?
                    .unwrap_or(false);
            }
            actions.push(PeaceAction {
                token,
                name: bytes(fields, b"name")?,
                taker: bytes(fields, b"taker")?,
                giver: bytes(fields, b"giver")?,
                negotiator: bytes(fields, b"negotiator")?,
                status: number(fields, b"status")?,
                cost: number(fields, b"cost")?,
                state: optional_number(fields, b"state")?,
                flags,
                puppet: bytes(fields, b"puppet")?,
                liberate: bytes(fields, b"liberate")?,
                ship: field(fields, b"ship")?.map(typed_handle).transpose()?,
                ratio: field(fields, b"ratio")?
                    .map(|v| Decimal::parse(scalar(v)?))
                    .transpose()?,
            });
        }
    }
    Ok(PeaceBid {
        country: bytes(items, b"country")?,
        actions,
    })
}

fn number<T: std::str::FromStr>(items: &[Item], key: &[u8]) -> io::Result<T> {
    integer(&required_bytes(items, key)?)
}
fn optional_number<T: std::str::FromStr>(items: &[Item], key: &[u8]) -> io::Result<Option<T>> {
    field(items, key)?
        .map(|value| integer(scalar(value)?))
        .transpose()
}
fn id(items: &[Item]) -> io::Result<u64> {
    typed_handle(field(items, b"id")?.ok_or_else(|| invalid("project object lacks typed id"))?)
}
fn anonymous(items: &[Item]) -> io::Result<Vec<&[Item]>> {
    items
        .iter()
        .map(|item| {
            if item.key.is_some() {
                return Err(invalid("expected anonymous saved array entry"));
            }
            block(&item.value)
        })
        .collect()
}
fn optional_handle(items: &[Item], key: &[u8]) -> io::Result<Option<u64>> {
    field(items, key)?.map(typed_handle).transpose()
}

fn parse_program(items: &[Item]) -> io::Result<Program> {
    let assignment = |fields: &[Item]| -> io::Result<ScientistAssignment> {
        let timer = field(fields, b"delayed_unassigned_scientist")?
            .map(block)
            .transpose()?
            .unwrap_or_default();
        Ok(ScientistAssignment {
            scientist: optional_handle(fields, b"scientist")?.unwrap_or(0),
            target: optional_number(timer, b"target")?.unwrap_or(0),
            current: optional_number(timer, b"current")?.unwrap_or(0),
            timed: field(timer, b"timed")?
                .map(boolean)
                .transpose()?
                .unwrap_or(false),
        })
    };
    let mut assignments = vec![assignment(required_block(items, b"scientist")?)?];
    if let Some(value) = field(items, b"supportive_scientist")? {
        for fields in anonymous(block(value)?)? {
            assignments.push(assignment(fields)?);
        }
    }
    Ok(Program {
        id: id(items)?,
        country: bytes(items, b"country")?,
        province: number(items, b"province")?,
        active_project: bytes(items, b"project")?,
        scientist: optional_handle(required_block(items, b"scientist")?, b"scientist")?,
        building: bytes(items, b"building")?.ok_or_else(|| invalid("program lacks building"))?,
        unread_rewards: number(items, b"unread_prototype_rewards")?,
        assignments,
    })
}
fn parse_context(items: &[Item]) -> io::Result<Context> {
    let bits = |key: &[u8]| -> io::Result<Option<u32>> {
        Ok(optional_number::<i32>(items, key)?.map(|value| value as u32))
    };
    Ok(Context {
        province: number(items, b"province")?,
        progress: number(items, b"progress")?,
        character: optional_handle(items, b"character")?,
        country: bits(b"country")?,
        scientist: bits(b"scientist")?,
        state: bits(b"state")?,
    })
}
fn parse_project(definition: &[u8], items: &[Item]) -> io::Result<Project> {
    let progress = required_block(items, b"progress")?;
    let prototype = required_block(progress, b"prototype_state")?;
    let stopping = required_block(progress, b"stopping_state")?;
    let mut history = Vec::new();
    if let Some(value) = field(items, b"history")? {
        for item in anonymous(block(value)?)? {
            history.push(History {
                reward: required_bytes(item, b"prototype_reward")?,
                option: required_bytes(item, b"option")?,
                date: GameDate::parse(&required_bytes(item, b"date")?)?,
                context: parse_context(required_block(item, b"context")?)?,
            });
        }
    }
    Ok(Project {
        pending_reward: bytes(items, b"multiple_choice")?,
        variables: super::intel::parse_variables(
            field(items, b"variables")?
                .map(block)
                .transpose()?
                .unwrap_or_default(),
        )?,
        id: id(items)?,
        definition: definition.to_vec(),
        program: optional_handle(items, b"program")?,
        started: boolean(field(items, b"start")?.ok_or_else(|| invalid("project lacks start"))?)?,
        completed: boolean(
            field(items, b"completed")?.ok_or_else(|| invalid("project lacks completed"))?,
        )?,
        current_state: number(progress, b"current_state")?,
        prototype_phase_progress: Decimal::parse(&required_bytes(prototype, b"phase_progress")?)?,
        prototype_progress: number(prototype, b"project_progress")?,
        iterations: number(prototype, b"iterations")?,
        stopping_phase_progress: Decimal::parse(&required_bytes(stopping, b"phase_progress")?)?,
        history,
    })
}
fn parse_status(items: &[Item]) -> io::Result<ProgramStatus> {
    let mut status = ProgramStatus::default();
    for item in items
        .iter()
        .filter(|item| item.key.as_deref() == Some(b"program"))
    {
        let program = parse_program(block(&item.value)?)?;
        status.program_order.push(program.id);
        if let Some(previous) = status.programs.insert(program.id, program.clone())
            && previous != program
        {
            return Err(invalid("conflicting duplicate project program"));
        }
    }
    for item in required_block(required_block(items, b"project_pool")?, b"project")? {
        let definition = item
            .key
            .as_deref()
            .ok_or_else(|| invalid("project lacks definition token"))?;
        let project = parse_project(definition, block(&item.value)?)?;
        if status.projects.insert(project.id, project).is_some() {
            return Err(invalid("duplicate project full handle within country pool"));
        }
    }
    for item in required_block(items, b"breakthrough")? {
        let specialization = item
            .key
            .clone()
            .ok_or_else(|| invalid("breakthrough lacks specialization token"))?;
        if status
            .breakthroughs
            .insert(specialization, integer(scalar(&item.value)?)?)
            .is_some()
        {
            return Err(invalid("duplicate breakthrough specialization"));
        }
    }
    Ok(status)
}
fn parse_raid_source(items: &[Item]) -> io::Result<RaidSource> {
    let kind = match required_bytes(items, b"type")?.as_slice() {
        b"air_base" => 0,
        b"rocket_site" => 1,
        b"naval_base" => 2,
        b"carrier" => 3,
        b"submarine" => 4,
        b"building" => 5,
        _ => return Err(invalid("unknown saved raid source kind")),
    };
    let building = field(items, b"building")?.map(block).transpose()?;
    Ok(RaidSource {
        kind,
        province: optional_number(items, b"province")?,
        ship: optional_handle(items, b"ship")?,
        country: bytes(items, b"tag")?,
        building_template: building
            .map(|items| required_bytes(items, b"template"))
            .transpose()?,
        building_location: building
            .map(|items| number(items, b"location"))
            .transpose()?,
    })
}
fn parse_target(items: &[Item]) -> io::Result<RaidTarget> {
    let building = field(items, b"building")?.map(block).transpose()?;
    Ok(RaidTarget {
        province: optional_number(items, b"province")?,
        state: optional_number(items, b"state")?,
        leader_province: optional_number(items, b"leader_province")?,
        building_template: building
            .map(|items| required_bytes(items, b"template"))
            .transpose()?,
        building_location: building
            .map(|items| number(items, b"location"))
            .transpose()?,
        leader: optional_handle(items, b"leader")?,
    })
}
fn scalars(items: &[Item]) -> io::Result<Vec<Vec<u8>>> {
    items
        .iter()
        .map(|item| {
            if item.key.is_some() {
                return Err(invalid("keyed entry in a saved scalar list"));
            }
            Ok(scalar(&item.value)?.to_vec())
        })
        .collect()
}
fn parse_target_statuses(items: &[Item]) -> io::Result<Vec<RaidTargetStatus>> {
    const COUNTERS: [&[u8]; 4] = [
        b"next_state",
        b"existing_target_num",
        b"detectable_target_num",
        b"next_target",
    ];
    const FIELDS: [&[u8]; 6] = [
        b"existing_target_num",
        b"detectable_target_num",
        b"valid",
        b"dynamic",
        b"target",
        b"detected",
    ];
    let mut statuses = Vec::new();
    for item in items {
        match item.key.as_deref() {
            Some(b"target") => {}
            Some(key) if COUNTERS.contains(&key) => continue,
            _ => return Err(invalid("unknown entry in saved raids.targets")),
        }
        let items = block(&item.value)?;
        if items
            .iter()
            .any(|item| !item.key.as_deref().is_some_and(|key| FIELDS.contains(&key)))
        {
            return Err(invalid("unknown entry in a saved raid target status"));
        }
        statuses.push(RaidTargetStatus {
            valid: field(items, b"valid")?
                .map(boolean)
                .transpose()?
                .unwrap_or(false),
            target: field(items, b"target")?
                .map(|value| parse_target(block(value)?))
                .transpose()?
                .unwrap_or_default(),
            detected: field(items, b"detected")?
                .map(|value| {
                    scalars(block(value)?)?
                        .iter()
                        .map(|byte| integer::<u8>(byte))
                        .collect::<io::Result<Vec<u8>>>()
                })
                .transpose()?
                .unwrap_or_default(),
        });
    }
    Ok(statuses)
}
fn parse_raid(items: &[Item]) -> io::Result<Raid> {
    let phase = match bytes(items, b"phase")?.as_deref() {
        None => 0,
        Some(b"ASSEMBLING") => 1,
        Some(b"PREPARING") => 2,
        Some(b"PREPARED") => 3,
        Some(b"IN_PROGRESS") => 4,
        Some(b"ENDED") => 5,
        _ => return Err(invalid("unknown saved raid phase")),
    };
    let outcome = match bytes(items, b"outcome")?.as_deref() {
        None => 0,
        Some(b"FAILURE") => 1,
        Some(b"LIMITED_SUCCESS") => 2,
        Some(b"SUCCESS") => 3,
        Some(b"CRITICAL_SUCCESS") => 4,
        Some(b"CANCELED") => 5,
        _ => return Err(invalid("unknown saved raid outcome")),
    };
    let target = field(items, b"target")?
        .map(|value| parse_target(block(value)?))
        .transpose()?;
    let unit = field(items, b"unit")?.map(block).transpose()?;
    let army = unit
        .map(|items| optional_handle(items, b"army"))
        .transpose()?
        .flatten();
    let air_wing = unit
        .map(|items| optional_handle(items, b"air_wing"))
        .transpose()?
        .flatten();
    Ok(Raid {
        army,
        air_wing,
        equipment: field(items, b"equipment")?
            .map(|value| super::production::parse_pool(block(value)?))
            .transpose()?,
        nukes: bytes(items, b"nukes")?
            .map(|text| integer(&text))
            .transpose()?
            .unwrap_or(0),
        end_date: bytes(items, b"end_date")?
            .map(|text| SavedDate::parse(&text))
            .transpose()?,
        preparation_hours: bytes(items, b"prep_time")?
            .map(|text| integer(&text))
            .transpose()?
            .unwrap_or(0),
        travelled: bytes(items, b"distance")?
            .map(|text| Decimal::parse(&text))
            .transpose()?,
        raid_type: bytes(items, b"type")?,
        detected: bytes(items, b"detected")?.as_deref() == Some(b"yes"),
        target,
        id: id(items)?,
        phase,
        outcome,
        show_for: field(items, b"show_for")?
            .map(|value| scalars(block(value)?))
            .transpose()?
            .unwrap_or_default(),
        source: field(items, b"raid_source")?
            .map(|value| parse_raid_source(block(value)?))
            .transpose()?,
    })
}
fn parse_raids(items: &[Item]) -> io::Result<Vec<RaidCountry>> {
    let mut countries = Vec::new();
    let mut ids = std::collections::BTreeSet::new();
    if let Some(value) = field(items, b"countries")? {
        for item in anonymous(block(value)?)? {
            let mut instances = Vec::new();
            if let Some(value) = field(item, b"raid_instance")? {
                for child in anonymous(block(value)?)? {
                    let raid = parse_raid(child)?;
                    if !ids.insert(raid.id) {
                        return Err(invalid("duplicate full raid handle"));
                    }
                    instances.push(raid);
                }
            }
            let cooldowns = field(item, b"target_cooldowns")?
                .map(|value| -> io::Result<Vec<RaidCooldown>> {
                    anonymous(block(value)?)?
                        .into_iter()
                        .map(|items| {
                            Ok(RaidCooldown {
                                raid_type: required_bytes(items, b"type")?,
                                days: number(items, b"cooldown")?,
                                target: field(items, b"target")?
                                    .map(|value| parse_target(block(value)?))
                                    .transpose()?
                                    .unwrap_or_default(),
                            })
                        })
                        .collect()
                })
                .transpose()?
                .unwrap_or_default();
            countries.push(RaidCountry {
                ordinal: u32::try_from(countries.len())
                    .map_err(|_| invalid("raid parent ordinal overflow"))?,
                country: bytes(item, b"tag")?,
                instances,
                used_types: field(item, b"used_types")?
                    .map(|value| scalars(block(value)?))
                    .transpose()?
                    .unwrap_or_default(),
                cooldowns,
            });
        }
    }
    Ok(countries)
}
fn parse_conferences(items: &[Item]) -> io::Result<BTreeMap<u64, Conference>> {
    let mut conferences = BTreeMap::new();
    for (ordinal, item) in items
        .iter()
        .filter(|item| item.key.as_deref() == Some(b"active_peace"))
        .enumerate()
    {
        let items = block(&item.value)?;
        let mut history_turns = BTreeMap::new();
        let mut bids = BTreeMap::new();
        let mut bid_countries = Vec::new();
        if let Some(value) = field(items, b"history")? {
            for country in block(value)? {
                let tag = country
                    .key
                    .clone()
                    .ok_or_else(|| invalid("peace history lacks country tag"))?;
                let mut count = 0_i32;
                let mut turns = Vec::new();
                for bid in block(&country.value)? {
                    if bid.key.as_deref() != Some(b"bidding") {
                        return Err(invalid("unknown peace history entry"));
                    }
                    turns.push(peace_bid(block(&bid.value)?)?);
                    count = count
                        .checked_add(1)
                        .ok_or_else(|| invalid("peace bidding count overflow"))?;
                }
                if history_turns.insert(tag.clone(), count).is_some() {
                    return Err(invalid("duplicate peace history country"));
                }
                bid_countries.push(tag.clone());
                bids.insert(tag, turns);
            }
        }
        let conference = Conference {
            id: id(items)?,
            ordinal: u32::try_from(ordinal).map_err(|_| invalid("peace ordinal overflow"))?,
            finalized: field(items, b"completed")?
                .map(boolean)
                .transpose()?
                .unwrap_or(false),
            history_turns,
            bids,
            bid_countries,
        };
        if conferences.insert(conference.id, conference).is_some() {
            return Err(invalid("duplicate full peace conference handle"));
        }
    }
    Ok(conferences)
}
pub fn parse(items: &[Item]) -> io::Result<Pack> {
    parse_with_countries(items, super::CountriesView::from_root(items)?)
}

pub(super) fn parse_with_countries(
    items: &[Item],
    countries: super::CountriesView<'_>,
) -> io::Result<Pack> {
    let mut pack = Pack::default();
    for country in countries.required()? {
        let tag = country
            .key
            .clone()
            .ok_or_else(|| invalid("project country lacks tag"))?;
        let items = block(&country.value)?;
        let nukes = field(items, b"nukes")?
            .map(|value| -> io::Result<Vec<Nuke>> {
                anonymous(block(value)?)?
                    .into_iter()
                    .map(|items| {
                        Ok(Nuke {
                            amount: Decimal::parse(&required_bytes(items, b"amount")?)?,
                            ready: number(items, b"nukes_ready")?,
                        })
                    })
                    .collect()
            })
            .transpose()?;
        let status = field(items, b"program_status")?
            .map(|value| parse_status(block(value)?))
            .transpose()?;
        if pack
            .countries
            .insert(tag, Country { nukes, status })
            .is_some()
        {
            return Err(invalid("duplicate project country"));
        }
    }
    pack.raids = field(items, b"raids")?
        .map(|value| parse_raids(block(value)?))
        .transpose()?;
    pack.raid_targets = match field(items, b"raids")? {
        Some(value) => field(block(value)?, b"targets")?
            .map(|value| parse_target_statuses(block(value)?))
            .transpose()?,
        None => None,
    };
    pack.conferences = field(items, b"peace_conference")?
        .map(|value| parse_conferences(block(value)?))
        .transpose()?;
    Ok(pack)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn parsed(text: &[u8]) -> Pack {
        parse(&content_reader_parser::parser::parse(text).unwrap().items).unwrap()
    }
    #[test]
    fn gap_research_grants_project_runtime_save_reads_supports_timers_pending_reward_and_variables()
    {
        let program = parse_program(&content_reader_parser::parser::parse(br#"id={id=3 type=83} province=9 building=mod_facility unread_prototype_rewards=0
            scientist={scientist={id=7 type=73} delayed_unassigned_scientist={target=5 current=-2 timed=yes}}
            supportive_scientist={{scientist={id=8 type=73} delayed_unassigned_scientist={target=8 current=3 timed=no}} {}}"#).unwrap().items).unwrap();
        assert_eq!(program.assignments.len(), 3);
        assert_eq!(program.assignments[0].scientist, (7u64 << 32) | 73);
        assert_eq!(
            (
                program.assignments[0].target,
                program.assignments[0].current,
                program.assignments[0].timed
            ),
            (5, -2, true)
        );
        assert_eq!(program.assignments[2].scientist, 0);
        let project = parse_project(b"mod_project", &content_reader_parser::parser::parse(br#"id={id=4 type=84} start=yes completed=no multiple_choice=mod_reward
            variables={mod_value=-1.23456 random={123 456}}
            progress={current_state=24 prototype_state={phase_progress=0 project_progress=2 iterations=3} stopping_state={phase_progress=0}}"#).unwrap().items).unwrap();
        assert_eq!(project.pending_reward, Some(b"mod_reward".to_vec()));
        assert_eq!(project.variables.seed, Some(456));
        assert_eq!(project.variables.entries[0].value.raw, -123456);
    }

    #[test]
    fn nuke_save_uses_fixed5_literals_and_full_signed_ready_count() {
        let pack = parsed(br"countries={ GER={ nukes={ { amount=1000.00001 nukes_ready=-2147483648 } { amount=90071992547.40993 nukes_ready=2147483647 } } } }");
        let nukes = pack.countries[b"GER".as_slice()].nukes.as_ref().unwrap();
        assert_eq!(nukes[0].amount.raw, 100_000_001);
        assert_eq!(nukes[0].ready, i32::MIN);
        assert_eq!(nukes[1].amount.raw, 9_007_199_254_740_993);
        assert_eq!(nukes[1].ready, i32::MAX);
        assert!(
            parse(
                &content_reader_parser::parser::parse(
                    br"countries={GER={nukes={{amount=0.000001 nukes_ready=0}}}}"
                )
                .unwrap()
                .items
            )
            .is_err()
        );
    }
    #[test]
    fn raid_target_statuses_refuse_unknown_and_keyed_entries() {
        let targets = |body: &str| {
            parse(
                &content_reader_parser::parser::parse(
                    format!("countries={{}} raids={{targets={{{body}}}}}").as_bytes(),
                )
                .unwrap()
                .items,
            )
            .map(|pack| pack.raid_targets.unwrap())
        };
        let exact = targets(
            "next_state=1 existing_target_num=2 detectable_target_num=1 next_target=2 target={existing_target_num=0 detectable_target_num=-1 valid=yes dynamic=no target={province=7} detected={ 4 0 }}",
        )
        .unwrap();
        assert_eq!(exact.len(), 1);
        assert_eq!(exact[0].detected, [4, 0]);
        assert_eq!(exact[0].target.province, Some(7));
        for body in [
            "next_status=1",
            "{valid=yes}",
            "target={valid=yes surprise=1}",
            "target={valid=yes detected={ first=4 }}",
        ] {
            assert!(targets(body).is_err(), "{body}");
        }
    }

    #[test]
    fn raid_order_source_omission_and_signed_fields_remain_distinct() {
        let pack = parsed(br#"countries={} raids={targets={next_state=1 target={valid=yes target={building={template=dam location=-1}} detected={ 4 0 }} target={valid=no target={leader={id=9 type=4713} leader_province=5}}} countries={{priority=-2 raid_instance={{id={id=4294967295 type=84} phase=PREPARED prep_time=-1}}} {tag=GER priority=1 raid_instance={{id={id=1 type=84} phase=ENDED outcome=SUCCESS end_date=1936.1.1.1 show_for={ "GER" "SOV" } raid_source={type=carrier ship={id=0 type=51} tag=GER}}} used_types={ oilfield_raid } target_cooldowns={{target={state=3} type="oilfield_raid" cooldown=2}}}}}"#);
        let parents = pack.raids.unwrap();
        assert_eq!(parents.len(), 2);
        assert_eq!(parents[0].ordinal, 0);
        assert_eq!(parents[0].country, None);
        assert_eq!(parents[0].instances[0].id, (u64::from(u32::MAX) << 32) | 84);
        assert_eq!(parents[0].instances[0].source, None);
        let raid = &parents[1].instances[0];
        assert_eq!(raid.source.as_ref().unwrap().ship, Some(51));
        assert_eq!(raid.show_for, [b"GER".to_vec(), b"SOV".to_vec()]);
        assert_eq!(
            raid.end_date,
            Some(SavedDate::Calendar(GameDate::parse(b"1936.1.1.1").unwrap()))
        );
        assert_eq!(parents[0].instances[0].end_date, None);
        assert_eq!(parents[1].used_types, [b"oilfield_raid".to_vec()]);
        assert_eq!(parents[1].cooldowns[0].target.state, Some(3));
        let targets = pack.raid_targets.unwrap();
        assert_eq!(targets.len(), 2);
        assert!(targets[0].valid && !targets[1].valid);
        assert_eq!(targets[0].target.building_location, Some(-1));
        assert_eq!(targets[0].detected, [4, 0]);
        assert_eq!(targets[1].target.leader, Some((9 << 32) | 4713));
    }
    #[test]
    fn context_keeps_optional_zero_and_signed_bit_patterns() {
        let items = content_reader_parser::parser::parse(br"province=-1 progress=-2147483648 country=-1 scientist=0 character={id=4294967295 type=73}").unwrap().items;
        let context = parse_context(&items).unwrap();
        assert_eq!(context.progress, i32::MIN);
        assert_eq!(context.country, Some(u32::MAX));
        assert_eq!(context.scientist, Some(0));
        assert_eq!(context.state, None);
        assert_eq!(context.character, Some((u64::from(u32::MAX) << 32) | 73));
    }
    #[test]
    fn project_pool_keeps_scoped_handles_history_order_and_saved_scales() {
        let pack = parsed(br#"countries={GER={program_status={
            project_pool={project={definition={id={id=4294967295 type=86}
                progress={current_state=-1 prototype_state={phase_progress=90071992547.40993 project_progress=-2147483648 iterations=-1} stopping_state={phase_progress=-0.00001}}
                start=no completed=yes program={id=0 type=83}
                history={{context={province=-1 progress=-2 country=-1 scientist=0} option=option_a prototype_reward=reward_a date=1936.1.1.1}
                         {context={province=1 progress=2 state=0} option=option_b prototype_reward=reward_b date=1936.1.1.2}}
            }}}
            program={id={id=0 type=83} country=GER province=1 scientist={scientist={id=0 type=73}} building="" unread_prototype_rewards=-1 project=definition}
            breakthrough={specialization=-2147483648}
        }} ENG={program_status={project_pool={project={}} breakthrough={}}}}"#);
        let status = pack.countries[b"GER".as_slice()].status.as_ref().unwrap();
        let project = &status.projects[&((u64::from(u32::MAX) << 32) | 86)];
        assert_eq!(project.prototype_phase_progress.raw, 9_007_199_254_740_993);
        assert_eq!(project.stopping_phase_progress.raw, -1);
        assert_eq!(project.prototype_progress, i32::MIN);
        assert_eq!(project.program, Some(83));
        assert_eq!(project.history[0].option, b"option_a");
        assert_eq!(project.history[1].context.state, Some(0));
        assert_eq!(status.programs[&83].scientist, Some(73));
        assert!(status.programs[&83].building.is_empty());
        assert_eq!(status.breakthroughs[b"specialization".as_slice()], i32::MIN);
        assert!(
            pack.countries[b"ENG".as_slice()]
                .status
                .as_ref()
                .unwrap()
                .projects
                .is_empty()
        );
    }
    #[test]
    fn conference_keeps_unequal_history_counts_and_saved_order() {
        let pack = parsed(br"countries={} peace_conference={active_peace={id={id=9 type=7} completed=yes history={GER={bidding={} bidding={}} ENG={bidding={}}}} active_peace={id={id=1 type=7}}}");
        let conferences = pack.conferences.unwrap();
        let first = &conferences[&((9_u64 << 32) | 7)];
        assert_eq!(first.ordinal, 0);
        assert!(first.finalized);
        assert_eq!(first.history_turns[b"GER".as_slice()], 2);
        assert_eq!(first.history_turns[b"ENG".as_slice()], 1);
        assert_eq!(conferences[&((1_u64 << 32) | 7)].ordinal, 1);
        let absent = parsed(b"countries={}");
        assert!(absent.raids.is_none() && absent.conferences.is_none());
        let empty = parsed(b"countries={} raids={} peace_conference={}");
        assert_eq!(empty.raids, Some(Vec::new()));
        assert_eq!(empty.conferences, Some(BTreeMap::new()));
    }

    #[test]
    fn peace_bids_preserve_mod_tokens_repeated_actions_and_optional_operands() {
        let pack = parsed(br#"countries={} peace_conference={active_peace={id={id=3 type=7} history={MOD_A={
            bidding={country=MOD_A actions={custom_action={name="Saved custom type" taker=MOD_A giver=MOD_B negotiator=MOD_C cost=-2 status=3 state=9876 puppet=MOD_P demilitarized_zone=yes resource_rights=yes}
            custom_action={taker=MOD_A giver=MOD_B negotiator=MOD_C cost=4 status=2 ship={id=9 type=11} ratio=0.00001}}}
            bidding={country=MOD_A actions={}}}}}} "#);
        let conference = &pack.conferences.as_ref().unwrap()[&((3_u64 << 32) | 7)];
        let turns = &conference.bids[b"MOD_A".as_slice()];
        assert_eq!(turns.len(), 2);
        assert!(turns[1].actions.is_empty());
        assert_eq!(turns[0].actions.len(), 2);
        assert_eq!(turns[0].actions[0].token, b"custom_action");
        assert_eq!(
            turns[0].actions[0].name.as_deref(),
            Some(b"Saved custom type".as_slice())
        );
        assert_eq!(turns[0].actions[0].flags, [true, false, true, false]);
        assert_eq!(
            turns[0].actions[0].puppet.as_deref(),
            Some(b"MOD_P".as_slice())
        );
        assert_eq!(turns[0].actions[1].ship, Some((9_u64 << 32) | 11));
        assert_eq!(turns[0].actions[1].ratio.as_ref().unwrap().raw, 1);
    }

    #[test]
    fn peace_history_country_groups_keep_writer_order_instead_of_tag_order() {
        let pack = parsed(br"countries={} peace_conference={active_peace={id={id=3 type=7} history={ZED={bidding={}} AAA={bidding={}}}}}");
        let conference = &pack.conferences.as_ref().unwrap()[&((3_u64 << 32) | 7)];
        assert_eq!(conference.bid_countries, [b"ZED".to_vec(), b"AAA".to_vec()]);
    }
}
