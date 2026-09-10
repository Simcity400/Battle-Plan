//! Literal project, nuclear, raid and conference state in native text saves.
use super::{
    Decimal, GameDate, block, boolean, bytes, field, integer, invalid, required_block,
    required_bytes, scalar, typed_handle,
};
use crate::parser::Item;
use std::{collections::BTreeMap, io};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pack {
    pub countries: BTreeMap<Vec<u8>, Country>,
    /// Absence is retained separately from a present, empty manager.
    pub raids: Option<Vec<RaidCountry>>,
    pub conferences: Option<BTreeMap<u64, Conference>>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Country {
    pub nukes: Option<Vec<Nuke>>,
    pub status: Option<ProgramStatus>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Nuke {
    /// 10853B0 -> AE590 -> AF9A0 -> B89F0 saves raw/100000, although one weapon
    /// is 100000000 raw units. Never apply the weapon scale to this literal.
    pub amount: Decimal,
    /// 10853B0 always saves +3C as a signed, unscaled integer.
    pub ready: i32,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProgramStatus {
    pub programs: BTreeMap<u64, Program>,
    pub projects: BTreeMap<u64, Project>,
    /// 19F14F0 prints the native fixed4 storage as an unscaled signed integer.
    pub breakthroughs: BTreeMap<Vec<u8>, i32>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Program {
    pub id: u64,
    pub country: Option<Vec<u8>>,
    pub province: u32,
    /// A lexer token, not the registered CProject reference id.
    pub active_project: Option<Vec<u8>>,
    pub scientist: Option<u64>,
    pub building: Vec<u8>,
    pub unread_rewards: i32,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Project {
    pub id: u64,
    pub definition: Vec<u8>,
    pub program: Option<u64>,
    pub started: bool,
    pub completed: bool,
    /// 1A24510 emits the current state's +78 enum, not a pointer offset.
    pub current_state: i32,
    pub prototype_phase_progress: Decimal,
    pub prototype_progress: i32,
    pub iterations: i32,
    pub stopping_phase_progress: Decimal,
    pub history: Vec<History>,
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
    /// 1A25240 omits both null and unresolvable native references.
    pub character: Option<u64>,
    /// Signed integer spellings of optional u32 bits, distinct from tags.
    pub country: Option<u32>,
    pub scientist: Option<u32>,
    pub state: Option<u32>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RaidCountry {
    /// Native array order includes dummy/nonpositive country entries.
    pub ordinal: u32,
    pub country: Option<Vec<u8>>,
    pub priority: i32,
    pub instances: Vec<Raid>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Raid {
    pub id: u64,
    pub preparation_hours: i32,
    pub phase: i32,
    pub outcome: i32,
    pub end_date: Option<GameDate>,
    pub victim_country: Option<Vec<u8>>,
    /// FDA710 omits the entire source when kind presence is zero. Omission
    /// cannot certify its independently stored location/reference fields.
    pub source: Option<RaidSource>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RaidSource {
    pub kind: u32,
    pub province: Option<u32>,
    pub ship: Option<u64>,
    pub country: Option<Vec<u8>>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conference {
    pub id: u64,
    pub ordinal: u32,
    pub finalized: bool,
    /// E34E60 rebuilds turn from the first native country-keyed history node.
    /// Keep every count; file order or lexical tag order is not native order.
    pub history_turns: BTreeMap<Vec<u8>, i32>,
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
                return Err(invalid("expected anonymous native array entry"));
            }
            block(&item.value)
        })
        .collect()
}
fn optional_handle(items: &[Item], key: &[u8]) -> io::Result<Option<u64>> {
    field(items, key)?.map(typed_handle).transpose()
}

fn parse_program(items: &[Item]) -> io::Result<Program> {
    Ok(Program {
        id: id(items)?,
        country: bytes(items, b"country")?,
        province: number(items, b"province")?,
        active_project: bytes(items, b"project")?,
        scientist: optional_handle(required_block(items, b"scientist")?, b"scientist")?,
        building: bytes(items, b"building")?.ok_or_else(|| invalid("program lacks building"))?,
        unread_rewards: number(items, b"unread_prototype_rewards")?,
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
    // FD0950 omits history precisely when its native vector count is zero.
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
    // E63F30 receives the CPersistent secondary base (primary +8).
    for item in items
        .iter()
        .filter(|item| item.key.as_deref() == Some(b"program"))
    {
        let program = parse_program(block(&item.value)?)?;
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
    Ok(RaidSource {
        kind,
        province: optional_number(items, b"province")?,
        ship: optional_handle(items, b"ship")?,
        country: bytes(items, b"tag")?,
    })
}
fn parse_raid(items: &[Item]) -> io::Result<Raid> {
    // FDA710 omits precisely zero phase/outcome/preparation values.
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
    Ok(Raid {
        id: id(items)?,
        preparation_hours: optional_number(items, b"prep_time")?.unwrap_or(0),
        phase,
        outcome,
        end_date: field(items, b"end_date")?
            .map(|value| GameDate::parse(scalar(value)?))
            .transpose()?,
        victim_country: bytes(items, b"victim_country")?,
        source: field(items, b"raid_source")?
            .map(|value| parse_raid_source(block(value)?))
            .transpose()?,
    })
}
fn parse_raids(items: &[Item]) -> io::Result<Vec<RaidCountry>> {
    let mut countries = Vec::new();
    let mut ids = std::collections::BTreeSet::new();
    // E71B30 omits countries only when its native array count is zero.
    if let Some(value) = field(items, b"countries")? {
        for item in anonymous(block(value)?)? {
            let mut instances = Vec::new();
            // 14D89E0 omits its child vector precisely when empty.
            if let Some(value) = field(item, b"raid_instance")? {
                for child in anonymous(block(value)?)? {
                    let raid = parse_raid(child)?;
                    if !ids.insert(raid.id) {
                        return Err(invalid("duplicate full raid handle"));
                    }
                    instances.push(raid);
                }
            }
            countries.push(RaidCountry {
                ordinal: u32::try_from(countries.len())
                    .map_err(|_| invalid("raid parent ordinal overflow"))?,
                country: bytes(item, b"tag")?,
                priority: number(item, b"priority")?,
                instances,
            });
        }
    }
    Ok(countries)
}
fn parse_conferences(items: &[Item]) -> io::Result<BTreeMap<u64, Conference>> {
    let mut conferences = BTreeMap::new();
    // BAFF00 serializes each qword from manager+8 through count+14 in order,
    // emitting repeated active_peace objects without filtering resident rows.
    for (ordinal, item) in items
        .iter()
        .filter(|item| item.key.as_deref() == Some(b"active_peace"))
        .enumerate()
    {
        let items = block(&item.value)?;
        let mut history_turns = BTreeMap::new();
        // E3DED0 omits history precisely when its native map has no entries.
        if let Some(value) = field(items, b"history")? {
            for country in block(value)? {
                let tag = country
                    .key
                    .clone()
                    .ok_or_else(|| invalid("peace history lacks country tag"))?;
                let mut count = 0_i32;
                for bid in block(&country.value)? {
                    if bid.key.as_deref() != Some(b"bidding") {
                        return Err(invalid("unknown peace history entry"));
                    }
                    block(&bid.value)?;
                    count = count
                        .checked_add(1)
                        .ok_or_else(|| invalid("peace bidding count overflow"))?;
                }
                if history_turns.insert(tag, count).is_some() {
                    return Err(invalid("duplicate peace history country"));
                }
            }
        }
        let conference = Conference {
            id: id(items)?,
            ordinal: u32::try_from(ordinal).map_err(|_| invalid("peace ordinal overflow"))?,
            // E3DED0 emits completed=yes iff native +DD is nonzero.
            finalized: field(items, b"completed")?
                .map(boolean)
                .transpose()?
                .unwrap_or(false),
            history_turns,
        };
        if conferences.insert(conference.id, conference).is_some() {
            return Err(invalid("duplicate full peace conference handle"));
        }
    }
    Ok(conferences)
}
pub fn parse(items: &[Item]) -> io::Result<Pack> {
    let mut pack = Pack::default();
    for country in required_block(items, b"countries")? {
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
    pack.conferences = field(items, b"peace_conference")?
        .map(|value| parse_conferences(block(value)?))
        .transpose()?;
    Ok(pack)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn parsed(text: &[u8]) -> Pack {
        parse(&crate::parser::parse(text).unwrap().items).unwrap()
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
                &crate::parser::parse(
                    br"countries={GER={nukes={{amount=0.000001 nukes_ready=0}}}}"
                )
                .unwrap()
                .items
            )
            .is_err()
        );
    }
    #[test]
    fn raid_order_source_omission_and_signed_fields_remain_distinct() {
        let pack = parsed(br"countries={} raids={countries={{priority=-2 raid_instance={{id={id=4294967295 type=84} phase=PREPARED prep_time=-1}}} {tag=GER priority=1 raid_instance={{id={id=1 type=84} phase=ENDED outcome=SUCCESS end_date=1936.1.1.1 raid_source={type=carrier ship={id=0 type=51} tag=GER}}}}}}");
        let parents = pack.raids.unwrap();
        assert_eq!(parents.len(), 2);
        assert_eq!(parents[0].ordinal, 0);
        assert_eq!(parents[0].country, None);
        assert_eq!(parents[0].instances[0].id, (u64::from(u32::MAX) << 32) | 84);
        assert_eq!(parents[0].instances[0].source, None);
        let raid = &parents[1].instances[0];
        assert_eq!(raid.source.as_ref().unwrap().ship, Some(51));
        assert_eq!(raid.preparation_hours, 0);
        assert!(raid.end_date.is_some());
    }
    #[test]
    fn context_keeps_optional_zero_and_signed_bit_patterns() {
        let items = crate::parser::parse(br"province=-1 progress=-2147483648 country=-1 scientist=0 character={id=4294967295 type=73}").unwrap().items;
        let context = parse_context(&items).unwrap();
        assert_eq!(context.progress, i32::MIN);
        assert_eq!(context.country, Some(u32::MAX));
        assert_eq!(context.scientist, Some(0));
        assert_eq!(context.state, None);
        assert_eq!(context.character, Some((u64::from(u32::MAX) << 32) | 73));
    }
    #[test]
    fn project_pool_keeps_scoped_handles_history_order_and_native_save_scales() {
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
    fn conference_keeps_unequal_history_counts_and_native_order() {
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
}
