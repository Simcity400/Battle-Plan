//! A country's manpower operands at an hour, read from the manpower family's series
//! publication ([`super::emit`]): the values its keys hold then.

use super::emit::{
    APPLIED, AVAILABLE, CORE, DEPLOYED, EFFECTIVE, LOCKED, READ_HOUR, RESIDUAL, RESOURCES, ROSTER,
    TOTAL, kind,
};
use dashboard_base::{
    prepared::engine,
    products::country_people::{
        self as rules, Operands, PeopleExile, PeopleParams, PeopleReply, RecruitmentDefines,
        StateOperands,
    },
};
use dashboard_politics::products::politics_state::history::AtHour;
use dashboard_series::{
    Cell, PageRead, Selection, Snapshot,
    codec::{Binary, Decode},
    family::Directory,
    query::EventView,
};
use std::{collections::HashMap, io};

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

/// The value `actor`'s key of `measure` (and `dimension`) holds at `hour`.
fn value<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    (actor, measure, dimension): (u64, u64, Option<u64>),
    hour: i32,
) -> io::Result<Option<i64>> {
    let Some(key) = directory.key(actor, measure, dimension) else {
        return Ok(None);
    };
    engine::run_at(snapshot, key, hour)?
        .map(|(run, _)| i64::try_from(run.value).map_err(|_| invalid("A manpower value overflows")))
        .transpose()
}

fn narrow<T: TryFrom<i64>>(value: Option<i64>) -> io::Result<Option<T>> {
    value
        .map(|value| T::try_from(value).map_err(|_| invalid("A manpower value is out of range")))
        .transpose()
}

/// `actor`'s latest roster event by `hour`: its hour, and its states in their recorded order,
/// none where unknown.
fn roster<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    actor: u64,
    hour: i32,
) -> io::Result<Option<(i32, Option<Vec<u32>>)>> {
    let Some(key) = directory.key(actor, ROSTER, None) else {
        return Ok(None);
    };
    let selection = Selection {
        key,
        membership: None,
    };
    let Some(EventView::Family(event)) = snapshot.latest_event(selection, hour, None)? else {
        return Ok(None);
    };
    let mut bytes = vec![0; usize::try_from(event.fields.len).map_err(|_| invalid("A manpower roster is malformed"))?];
    snapshot.cell_bytes(event.fields, 0, &mut bytes)?;
    let states = match event.kind {
        kind::UNKNOWN => None,
        kind::ROSTER => Some(
            bytes
                .chunks_exact(Cell::SIZE)
                .map(|chunk| match Cell::decode(&mut Decode::new(chunk))? {
                    Cell::Unsigned(state) => {
                        u32::try_from(state).map_err(|_| invalid("A manpower state is out of range"))
                    }
                    _ => Err(invalid("A manpower roster is malformed")),
                })
                .collect::<io::Result<_>>()?,
        ),
        _ => return Err(invalid("A manpower roster is malformed")),
    };
    Ok(Some((event.native_hour, states)))
}

/// State `state`'s operands at `hour` as `actor` controls it; none without its row.
fn state_at<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    actor: u64,
    state: u64,
    hour: i32,
) -> io::Result<Option<StateOperands>> {
    let mut at = |measure: u64| value(snapshot, directory, (actor, measure, Some(state)), hour);
    let Some(available) = narrow::<i32>(at(AVAILABLE)?)? else {
        return Ok(None);
    };
    let mut resources = [0; 4];
    let mut known = true;
    for (slot, measure) in RESOURCES.into_iter().enumerate() {
        match at(measure)? {
            Some(value) => resources[slot] = value,
            None => known = false,
        }
    }
    Ok(Some(StateOperands {
        available,
        locked: narrow::<i32>(at(LOCKED)?)?,
        total: narrow::<i32>(at(TOTAL)?)?,
        resources: known.then_some(resources),
        core: at(CORE)?.map(|core| core != 0),
    }))
}

/// `tag`'s operands at `hour` and the holder (slot and country id) they are of; none where it
/// held no country then.
pub fn operands_at<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    tag: &str,
    hour: i32,
) -> io::Result<Option<(Operands, (u32, u32))>> {
    let Some(actor) = directory.actor(tag.as_bytes()) else {
        return Ok(None);
    };
    let Some(key) = directory.key(actor, RESIDUAL, None) else {
        return Ok(None);
    };
    let Some((run, evidence)) = engine::run_at(snapshot, key, hour)? else {
        return Ok(None);
    };
    let holder = evidence
        .holder
        .ok_or_else(|| invalid("A manpower value names no holder"))?;
    let holder = ((holder >> 32) as u32, holder as u32);
    let residual = narrow::<i32>(Some(
        i64::try_from(run.value).map_err(|_| invalid("A manpower value overflows"))?,
    ))?
    .ok_or_else(|| invalid("A manpower residual is missing"))?;
    let mut get = |measure: u64| value(snapshot, directory, (actor, measure, None), hour);
    let applied = get(APPLIED)?;
    let read_hour = narrow::<i32>(get(READ_HOUR)?)?;
    let mut effective = [None; 4];
    for (at, measure) in EFFECTIVE.into_iter().enumerate() {
        effective[at] = get(measure)?;
    }
    let mut deployed = [None; 5];
    for (at, measure) in DEPLOYED.into_iter().enumerate() {
        deployed[at] = narrow::<i32>(get(measure)?)?;
    }
    // The holder's segment writes its states where it starts.
    let first = evidence.identity_first.unwrap_or(run.span.first);
    let states = match roster(snapshot, directory, actor, hour)? {
        Some((at, _)) if at < first => {
            return Err(invalid("A manpower roster is older than its holder's segment"));
        }
        None => return Err(invalid("A manpower country holds no roster")),
        Some((_, None)) => None,
        Some((_, Some(roster))) => {
            let mut read = HashMap::new();
            let mut states = Vec::with_capacity(roster.len());
            for state in roster {
                if !read.contains_key(&state) {
                    let row = state_at(snapshot, directory, actor, u64::from(state), hour)?;
                    read.insert(state, row);
                }
                states.push((state, read[&state].clone()));
            }
            Some(states)
        }
    };
    let operands = Operands {
        residual,
        states,
        applied,
        read_hour,
        effective: effective
            .iter()
            .all(Option::is_some)
            .then(|| effective.map(|value| value.unwrap_or_default())),
        deployed,
    };
    Ok(Some((operands, holder)))
}

/// Why a view has no elections or exiles: `politics_state` has not published the branch
/// through the hour yet.
pub const UNPREPARED: &str =
    "Elections and governments in exile are not prepared for this hour yet";

fn small(value: Option<i128>) -> Option<i32> {
    value.and_then(|value| i32::try_from(value).ok())
}

/// The reply to `params` from the country's `operands` and its holder's `politics` (its
/// elections and the exiles it is or hosts, none before `politics_state` publishes the hour): the
/// manpower view by the game's rules, the election schedule and the exile cards, each card's
/// deployed air crews from `air` by its place.
pub fn assemble(
    params: &PeopleParams,
    operands: &Operands,
    defines: Result<RecruitmentDefines, String>,
    politics: Option<&AtHour>,
    air: &[Option<i32>],
) -> PeopleReply {
    let manpower = rules::manpower_view(operands, defines.as_ref().ok().copied());
    // The holder's own schedule: another country's unread row leaves it as read.
    let (allowed, interval, next) = match politics {
        Some(at) => (
            at.held.filter(|_| !at.refused).map(|held| held != 0),
            small(at.interval),
            small(at.next),
        ),
        None => (None, None, None),
    };
    let (politics_facts, politics_warning) = rules::politics_view(allowed, interval, next);
    let mut refusals = [0_u32; 5];
    let mut exiles = Vec::new();
    for (exile, air) in politics.into_iter().flat_map(|at| &at.exiles).zip(air) {
        for (field, value) in exile.values[..4].iter().enumerate() {
            if value.is_none() {
                refusals[field + 1] += 1;
            }
        }
        exiles.push(PeopleExile {
            tag: exile
                .tag
                .clone()
                .unwrap_or_else(|| format!("Country {}", exile.slot)),
            host: exile.host_tag.clone(),
            legitimacy: exile.values[0].map(|value| value.to_string()),
            manpower: small(exile.values[1]),
            cap: small(exile.values[2]),
            leaders: small(exile.values[3]),
            deployed_air: *air,
            read_hour: small(exile.values[4]),
        });
    }
    let exiles_known = politics.is_some_and(|at| at.exiles_known);
    let mut warnings = manpower.warnings;
    if politics.is_some() {
        warnings.extend(politics_warning);
        warnings.extend(rules::exile_warnings(!exiles_known, &exiles, refusals));
    } else {
        warnings.push(UNPREPARED.into());
    }
    if let Err(error) = defines {
        warnings.push(format!("1 refused mobilisation rate: {error}"));
    }
    PeopleReply {
        branch_id: params.branch_id.clone(),
        tag: params.tag.clone(),
        hour: params.hour,
        read_hour: manpower.read_hour,
        manpower: manpower.facts,
        politics: politics_facts,
        states: manpower.states,
        exiles,
        warnings,
    }
}
