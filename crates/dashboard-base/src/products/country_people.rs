//! The game's recruitment, manpower pool, election and exile rules, and the country-people
//! reply they make: one country's manpower operands at an hour ([`Operands`]) through
//! [`manpower_view`], its election schedule through [`politics_view`] and its exile cards.
//! The product (`dashboard-manpower`) reads the operands its family prepared. Nothing here
//! reads live memory; totals use the recorded controlled-state order.
#[cfg(any(test, feature = "test-support"))]
use crate::invalid;
use installed_content_core::content::diagnostics::Once as OnceLock;
use replay_data::rows::{Recorded, TableId, tables};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
#[cfg(any(test, feature = "test-support"))]
use std::collections::BTreeMap;
use std::io;
use ts_rs::TS;

#[derive(Clone, Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PeopleParams {
    pub branch_id: String,
    pub tag: String,
    pub hour: i32,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PeopleFact {
    pub label: String,
    pub value: Option<String>,
    pub unit: String,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PeopleState {
    pub state: u32,
    pub available: i32,
    pub locked: Option<i32>,
    pub total: Option<i32>,
    pub effective: Option<String>,
    pub recruitable: Option<i32>,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PeopleExile {
    pub tag: String,
    pub host: Option<String>,
    pub legitimacy: Option<String>,
    pub manpower: Option<i32>,
    pub cap: Option<i32>,
    pub leaders: Option<i32>,
    pub deployed_air: Option<i32>,
    pub read_hour: Option<i32>,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PeopleReply {
    pub branch_id: String,
    pub tag: String,
    pub hour: i32,
    pub read_hour: Option<i32>,
    pub manpower: Vec<PeopleFact>,
    pub politics: Vec<PeopleFact>,
    pub states: Vec<PeopleState>,
    pub exiles: Vec<PeopleExile>,
    pub warnings: Vec<String>,
}
fn scalar<T: Copy>(value: &Recorded<Option<T>>) -> Option<T> {
    value.require("people operand").ok().copied().flatten()
}
fn fact(label: &str, value: Option<impl ToString>, unit: &str) -> PeopleFact {
    PeopleFact {
        label: label.into(),
        value: value.map(|v| v.to_string()),
        unit: unit.into(),
    }
}
/// Country target is the effective conscription entry times its factor; the
/// applied ratio is stored at one hundred times that fixed5 fraction.
pub fn recruitment_target(conscription: i64, factor: i64) -> i64 {
    factor
        .wrapping_add(100_000)
        .wrapping_mul(conscription)
        .wrapping_div(100_000)
        .max(0)
}
pub fn mobilisation_change(applied: i64, target: i64, base: i64, factor: i64) -> i64 {
    let target = target.wrapping_mul(100);
    if applied == target {
        return 0;
    }
    let speed = factor
        .wrapping_add(100_000)
        .wrapping_mul(base)
        .wrapping_div(100_000);
    let distance = applied.wrapping_sub(target).wrapping_abs();
    let step = speed.min(distance).max(0);
    if target <= applied {
        step.wrapping_neg()
    } else {
        step
    }
}
#[derive(Clone, Copy)]
pub struct RecruitmentDefines {
    pub speed: i64,
    pub noncore_base: i64,
    pub noncore_max: i64,
}

/// Runtime and offline preparation share the same validated stack operands,
/// including named refusals; neither invents defaults for missing defines.
pub fn content_defines(
    layers: &installed_content_core::ResolvedLayers,
) -> Result<RecruitmentDefines, String> {
    let held = layers.derived::<OnceLock<Result<RecruitmentDefines, String>>>();
    held.get_or_init(|| {
        let load = || -> io::Result<RecruitmentDefines> {
            let defines = installed_content_core::defines::read(layers)?;
            Ok(RecruitmentDefines {
                speed: defines.fixed5("NCountry", "BASE_MOBILIZATION_SPEED")?,
                noncore_base: defines
                    .fixed5("NCountry", "LOCAL_MANPOWER_ACCESSIBLE_NON_CORE_FACTOR")?,
                noncore_max: defines.fixed5("NCountry", "MAX_NON_CORE_MANPOWER_FACTOR")?,
            })
        };
        load().map_err(|why| why.to_string())
    })
    .clone()
}

/// The fixed-point getter maps its three non-finite words to its invalid result.
pub fn applied_fraction(raw: i64) -> i64 {
    if super::finite_fixed64(raw) {
        raw.wrapping_mul(100_000) / 10_000_000
    } else {
        0xffff_ffff
    }
}
pub fn state_recruitment(
    applied: i64,
    values: [i64; 4],
    core: bool,
    noncore: i64,
    defines: RecruitmentDefines,
) -> i64 {
    let mut value = applied_fraction(applied).wrapping_add(values[2]);
    value = value.wrapping_mul(values[3].wrapping_add(100_000)) / 100_000;
    value = value.wrapping_mul(values[0].wrapping_add(100_000)) / 100_000;
    if !core {
        value = value.wrapping_mul(
            defines
                .noncore_base
                .wrapping_add(values[1])
                .wrapping_add(noncore)
                .min(defines.noncore_max),
        ) / 100_000;
    }
    value.clamp(0, 100_000)
}
/// The engine's month operation preserves the day offset, so a shorter
/// destination month can overflow into the following month. Dates and month
/// arithmetic retain signed native wrapping, including years before the epoch.
pub fn election_date(last: i32, interval: i32) -> i32 {
    use engine_native::cdate::{HOUR_EPOCH, MONTH_LENGTHS};
    fn parts(hour: i32) -> (i32, i32, i32) {
        let days = hour.wrapping_sub(HOUR_EPOCH) / 24;
        let ordinal = days % 365;
        let mut month = 0;
        let mut prefix = 0;
        while month < 11 && ordinal >= prefix + MONTH_LENGTHS[month] {
            prefix += MONTH_LENGTHS[month];
            month += 1;
        }
        (days, month as i32, prefix)
    }
    fn replace_year(hour: i32, year: i32) -> i32 {
        let (days, _, _) = parts(hour);
        let ordinal = days % 365;
        let absolute = year.wrapping_abs().wrapping_mul(365).wrapping_add(ordinal);
        let shifted = if year < 0 {
            absolute.wrapping_neg()
        } else {
            absolute
        };
        hour.wrapping_add(shifted.wrapping_sub(days).wrapping_mul(24))
    }
    let (days, month, _) = parts(last);
    let combined = month.wrapping_add(interval);
    let mut years = combined / 12;
    let mut destination = combined;
    let mut hour = last;
    if years == 0 {
        if combined < 0 {
            hour = replace_year(hour, (days / 365).wrapping_sub(1));
            destination = destination.wrapping_add(12);
        }
    } else {
        hour = replace_year(hour, (days / 365).wrapping_add(years));
        if combined < 0 {
            destination = destination.wrapping_add(12);
            years = destination / 12;
        }
        destination = destination.wrapping_sub(years.wrapping_mul(12));
    }
    let (days, _, old_prefix) = parts(hour);
    let year = days / 365;
    let new_prefix: i32 = MONTH_LENGTHS[..destination.max(0) as usize].iter().sum();
    let ordinal = (days % 365)
        .wrapping_sub(old_prefix)
        .wrapping_add(new_prefix);
    let absolute = year.wrapping_abs().wrapping_mul(365).wrapping_add(ordinal);
    let shifted = if year < 0 {
        absolute.wrapping_neg()
    } else {
        absolute
    };
    hour.wrapping_add(shifted.wrapping_sub(days).wrapping_mul(24))
}

/// The next scheduled election: the independent cache for exceptional
/// initializers, otherwise the native calendar step from the last election.
pub fn next_election_date(row: &replay_data::rows::PoliticsRow) -> Option<i32> {
    if engine_native::cpolitical_status::needs_election_cache(row.last_election) {
        scalar(&row.next_election_cache)
    } else {
        row.election_interval
            .require("election interval")
            .ok()
            .map(|&interval| election_date(row.last_election, interval))
    }
}

/// One controlled state's manpower operands, as the pool and its recruitment read them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StateOperands {
    pub available: i32,
    pub locked: Option<i32>,
    pub total: Option<i32>,
    /// Local manpower, local non-core manpower, recruitment add and recruitment factor; none
    /// where StateResourceInputs is unavailable, holds no row for the state or any is unread.
    pub resources: Option<[i64; 4]>,
    /// Whether the country cores the state; none where StateCores is unavailable or holds no
    /// row for it.
    pub core: Option<bool>,
}

/// The deployed manpower labels, in [`Operands::deployed`]'s order.
pub const DEPLOYED: [&str; 5] = [
    "Deployed army",
    "Deployed navy",
    "Required navy crews",
    "Deployed air force",
    "Deployed exile air force",
];

/// One country's manpower operands at an hour: everything its manpower view reads.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Operands {
    pub residual: i32,
    /// Each controlled state in recorded order, none where States holds no row for it; none
    /// where the controlled-state roster or States is unavailable.
    pub states: Option<Vec<(u32, Option<StateOperands>)>>,
    /// The applied recruitment ratio, raw.
    pub applied: Option<i64>,
    /// The hour the country's recruitment inputs were read.
    pub read_hour: Option<i32>,
    /// Effective mobilisation speed, conscription, conscription factor and non-core manpower.
    pub effective: Option<[i64; 4]>,
    /// Deployed army, navy, required navy crews, air force and exile air force ([`DEPLOYED`]).
    pub deployed: [Option<i32>; 5],
}

/// The modifiers [`Operands::effective`] holds, in its order.
pub const RECRUITMENT_MODIFIERS: [&str; 4] = [
    "mobilization_speed",
    "conscription",
    "conscription_factor",
    "non_core_manpower",
];

/// A country's manpower view: its facts, controlled states, recruitment read and warnings.
pub struct ManpowerView {
    pub facts: Vec<PeopleFact>,
    pub states: Vec<PeopleState>,
    pub read_hour: Option<i32>,
    pub warnings: Vec<String>,
}

/// The manpower view of `operands`, with the recorded content's recruitment defines.
pub fn manpower_view(operands: &Operands, defines: Option<RecruitmentDefines>) -> ManpowerView {
    let mut view = ManpowerView {
        facts: Vec::new(),
        states: Vec::new(),
        read_hour: operands.read_hour,
        warnings: Vec::new(),
    };
    let mut available = Some(operands.residual);
    let mut locked = Some(0_i32);
    let mut total = Some(0_i32);
    let mut resources = Vec::new();
    match &operands.states {
        Some(states) => {
            for (id, row) in states {
                let Some(row) = row else {
                    available = None;
                    locked = None;
                    total = None;
                    view.warnings.push(format!("1 missing controlled state {id}: manpower pool and effective recruitment unavailable"));
                    continue;
                };
                available = available.map(|sum| sum.wrapping_add(row.available));
                locked = locked.zip(row.locked).map(|(sum, value)| sum.wrapping_add(value));
                total = total.zip(row.total).map(|(sum, value)| sum.wrapping_add(value));
                view.states.push(PeopleState {
                    state: *id,
                    available: row.available,
                    locked: row.locked,
                    total: row.total,
                    effective: None,
                    recruitable: None,
                });
                resources.push((row.resources, row.core));
            }
        }
        None => {
            available = None;
            locked = None;
            total = None;
            view.warnings.push(
                "1 refused manpower pool: controlled-state roster or States unavailable".into(),
            );
        }
    }
    view.facts.extend([
        fact("Available manpower", available, "people"),
        fact("Locked population", locked, "people"),
        fact("Total population", total, "people"),
    ]);
    let applied = operands.applied;
    view.facts.push(fact(
        "Applied recruitment ratio",
        applied.map(applied_fraction),
        "ratio",
    ));
    if applied.is_none() {
        view.warnings
            .push("1 refused applied recruitment ratio: country operand unavailable".into());
    }
    let effective = operands.effective;
    let target = effective.map(|values| recruitment_target(values[1], values[2]));
    let daily = applied
        .zip(target)
        .zip(defines.map(|value| value.speed))
        .zip(effective)
        .map(|(((applied, target), base), values)| {
            mobilisation_change(applied, target, base, values[0])
        });
    view.facts.push(fact("Recruitment target", target, "ratio"));
    view.facts.push(fact(
        "Daily mobilisation",
        daily.map(applied_fraction),
        "dailyRatio",
    ));
    view.facts.push(fact(
        "Mobilising reserves",
        target
            .zip(applied)
            .map(|(target, applied)| target.wrapping_sub(applied_fraction(applied))),
        "ratio",
    ));
    if effective.is_none() {
        view.warnings.push(
            "1 refused effective recruitment: country modifier operands or definitions unavailable"
                .into(),
        );
    }
    let mut recruitable = Some(operands.residual);
    for (state, (resources, core)) in view.states.iter_mut().zip(resources) {
        let value = (|| {
            let rate = state_recruitment(applied?, resources?, core?, effective?[3], defines?);
            let amount = i64::from(state.total?).wrapping_mul(rate) / 100_000;
            Some((rate, amount as i32))
        })();
        state.effective = value.map(|value| value.0.to_string());
        state.recruitable = value.map(|value| value.1);
        recruitable = recruitable
            .zip(state.recruitable)
            .map(|(total, value)| total.wrapping_add(value));
    }
    if operands
        .states
        .as_ref()
        .is_none_or(|states| states.len() != view.states.len())
    {
        recruitable = None;
    }
    view.facts.push(fact(
        "Effective recruitable population",
        recruitable,
        "people",
    ));
    for (label, value) in DEPLOYED.into_iter().zip(operands.deployed) {
        view.facts.push(fact(label, value, "people"));
        if value.is_none() {
            view.warnings.push(format!(
                "1 refused {label}: required unit manpower or exact roster membership unavailable"
            ));
        }
    }
    if recruitable.is_none() {
        view.warnings.push("1 refused effective recruitment: state recruitment operands, cores or recorded content unavailable".into());
    }
    view
}

/// The election facts of a schedule, and the warning a refused part of it gives.
pub fn politics_view(
    allowed: Option<bool>,
    interval: Option<i32>,
    next: Option<i32>,
) -> (Vec<PeopleFact>, Option<String>) {
    let facts = vec![
        fact(
            "Elections",
            allowed.map(|value| if value { "Allowed" } else { "Not held" }),
            "text",
        ),
        fact("Election interval", interval, "months"),
        if allowed == Some(false) {
            fact("Next election", Some("Not held"), "text")
        } else {
            fact("Next election", next, "date")
        },
    ];
    let refused = allowed.is_none() || interval.is_none() || (allowed == Some(true) && next.is_none());
    (
        facts,
        refused.then(|| "1 refused election details: Politics operands unavailable".into()),
    )
}

/// The warnings of exile cards: a refused host for the country itself, each refused field's
/// count, and the cards whose deployed air crews are unknown.
pub fn exile_warnings(host_refused: bool, exiles: &[PeopleExile], refusals: [u32; 5]) -> Vec<String> {
    let mut warnings = Vec::new();
    if host_refused {
        warnings.push("1 refused exile status: country host unavailable".into());
    }
    for (count, field) in refusals.into_iter().zip([
        "host roster",
        "legitimacy",
        "manpower",
        "manpower cap",
        "leader resources",
    ]) {
        if count != 0 {
            warnings.push(format!("{count} refused exile {field}: operands unavailable"));
        }
    }
    let missing_air = exiles
        .iter()
        .filter(|exile| exile.deployed_air.is_none())
        .count();
    if missing_air != 0 {
        warnings.push(format!("{missing_air} refused exile deployed air crews: wing attribution or manpower unavailable"));
    }
    warnings
}

/// The country's manpower operands in `frame`, the effective modifiers read through
/// the hour's modifier definitions; `same` tells the country's raw references.
#[cfg(any(test, feature = "test-support"))]
pub fn frame_operands(
    frame: &DecodedFrame,
    country: &replay_data::rows::CountryRow,
    same: impl Fn(u32) -> bool,
    equal: impl Fn(u32, u32) -> bool,
) -> Operands {
    let counters = frame
        .complete::<tables::CountryCounterInputs>()
        .ok()
        .and_then(|rows| rows.iter().find(|row| same(row.country)));
    let state_rows = frame.complete::<tables::States>().ok();
    let roster = counters
        .and_then(|row| row.controlled_states.require("controlled states").ok())
        .and_then(Option::as_deref);
    let resource_inputs = frame.complete::<tables::StateResourceInputs>().ok();
    let cores = frame.complete::<tables::StateCores>().ok();
    let states = roster.zip(state_rows).map(|(roster, rows)| {
        let rows: BTreeMap<_, _> = rows.iter().map(|row| (row.id, row)).collect();
        roster
            .iter()
            .map(|id| {
                let row = rows.get(id).map(|row| StateOperands {
                    available: row.manpower_available,
                    locked: row.manpower_locked.require("locked").ok().copied(),
                    total: row.population.require("population").ok().copied(),
                    resources: (|| {
                        let values = resource_inputs?.iter().find(|row| row.state == *id)?;
                        Some([
                            scalar(&values.local_manpower)?,
                            scalar(&values.local_noncore_manpower)?,
                            scalar(&values.recruitment_add)?,
                            scalar(&values.recruitment_factor)?,
                        ])
                    })(),
                    core: cores
                        .and_then(|cores| cores.iter().find(|row| row.state == *id))
                        .map(|row| row.cores.iter().any(|&raw| same(raw))),
                });
                (*id, row)
            })
            .collect()
    });
    let input = frame
        .complete::<tables::CountryStabilityInputs>()
        .ok()
        .and_then(|rows| rows.iter().find(|row| same(row.country)));
    let reads = frame.complete::<tables::SliceReads>().ok();
    let read_hour = reads.and_then(|reads| {
        input.and_then(|row| {
            replay_data::sliced::read_hour(reads, TableId::CountryStabilityInputs, row.country)
        })
    });
    let modifiers = counters
        .and_then(|row| {
            row.recruitment_modifiers
                .require("recruitment modifiers")
                .ok()
        })
        .copied()
        .flatten();
    let effective = modifiers.and_then(|values| {
        let registry = replay_derive::modifier_registry::Registry::from_frame(frame).ok();
        let mut result = [0; 4];
        for (index, name) in RECRUITMENT_MODIFIERS.into_iter().enumerate() {
            result[index] = match values[index] {
                None => 0,
                Some(raw) => {
                    let registry = registry.as_ref()?;
                    let definition = registry.named(name).ok()?;
                    registry.effective(definition, raw).ok()?
                }
            };
        }
        Some(result)
    });
    let army = frame
        .complete::<tables::Divisions>()
        .ok()
        .zip(frame.complete::<tables::DivisionLandInputs>().ok())
        .zip(frame.complete::<tables::DivisionTemplateIcons>().ok())
        .and_then(|((divisions, land), templates)| {
            let land: BTreeMap<_, _> = land.iter().map(|row| (row.division, row)).collect();
            let templates: BTreeMap<_, _> =
                templates.iter().map(|row| (row.template, row)).collect();
            let ordinary_template = |division: &replay_data::rows::DivisionRow| {
                templates
                    .get(&division.template)?
                    .manpower_exile
                    .require("template manpower exile")
                    .ok()
                    .map(|exile| !*exile)
            };
            let amount = |id| {
                land.get(&id).map(|row| {
                    row.manpower
                        .iter()
                        .find(|entry| same(entry.country))
                        .map_or(0, |entry| entry.amount)
                })
            };
            let mut total = 0_i32;
            for division in divisions
                .iter()
                .filter(|division| same(division.roster_owner))
            {
                let owner = replay_data::military_aid::owner(division);
                if same(owner)
                    && equal(division.roster_owner, division.logical_owner)
                    && ordinary_template(division)?
                {
                    total = total.wrapping_add(amount(division.id)?);
                }
            }
            let expeditionaries = counters?
                .sent_expeditionaries
                .require("sent expeditionaries")
                .ok()?
                .as_ref()?;
            total = total.wrapping_add(replay_data::military_aid::listed_manpower(
                expeditionaries.iter().copied(),
                divisions,
                ordinary_template,
                amount,
            )?);
            total = total.wrapping_add(replay_data::military_aid::listed_manpower(
                frame
                    .complete::<tables::SentVolunteers>()
                    .ok()?
                    .iter()
                    .filter(|row| same(row.country))
                    .map(|row| row.division),
                divisions,
                ordinary_template,
                amount,
            )?);
            Some(total)
        });
    let navy = frame
        .complete::<tables::Ships>()
        .ok()
        .zip(frame.complete::<tables::TaskForces>().ok())
        .zip(frame.complete::<tables::Fleets>().ok())
        .and_then(|((ships, tasks), fleets)| {
            let owners: BTreeMap<_, _> =
                fleets.iter().map(|fleet| (fleet.id, fleet.owner)).collect();
            let tasks: BTreeMap<_, _> = tasks.iter().map(|task| (task.id, task.fleet)).collect();
            let mut total = Some(0_i32);
            let mut required = Some(0_i32);
            for ship in ships {
                let owner = owners.get(tasks.get(&ship.task_force)?)?;
                if same(*owner) {
                    total = total
                        .zip(scalar(&ship.acquired_manpower))
                        .map(|(sum, value)| sum.wrapping_add(value));
                    required = required
                        .zip(scalar(&ship.required_manpower))
                        .map(|(sum, value)| sum.wrapping_add(value));
                }
            }
            Some((total, required))
        });
    let air_wings = frame.complete::<tables::AirWings>().ok();
    Operands {
        residual: country.manpower_residual,
        states,
        applied: input.and_then(|row| scalar(&row.recruitment_ratio)),
        read_hour,
        effective,
        deployed: [
            army,
            navy.and_then(|values| values.0),
            navy.and_then(|values| values.1),
            air_wings.and_then(|wings| ordinary_air_manpower(wings, &same)),
            air_wings.and_then(|wings| exile_air_manpower(wings, |raw| equal(raw, country.id))),
        ],
    }
}

/// The reply `frame` gives: the reference the prepared reply is held to.
#[cfg(any(test, feature = "test-support"))]
pub fn read(
    frame: &DecodedFrame,
    p: &PeopleParams,
    defines: Option<RecruitmentDefines>,
) -> io::Result<PeopleReply> {
    let countries = frame.complete::<tables::Countries>()?;
    let links = frame.complete::<tables::CountryLinks>()?;
    let tagged = crate::metadata::tagged_countries(countries, links);
    let (country, _) = tagged
        .get(p.tag.as_bytes())
        .and_then(|lookup| lookup.found())
        .ok_or_else(|| dashboard_core::refused("Country unavailable at this hour"))?;
    let canonical = |raw: u32| {
        links
            .iter()
            .find(|link| link.id == raw)
            .map(|link| link.canonical_slot)
    };
    let equal = |first: u32, second: u32| {
        first == second
            || (first != 0
                && second != 0
                && canonical(first).is_some_and(|slot| canonical(second) == Some(slot)))
    };
    let same = |raw: u32| equal(raw, country.id);
    let manpower = manpower_view(&frame_operands(frame, country, same, equal), defines);
    let politics = frame
        .complete::<tables::Politics>()
        .ok()
        .and_then(|rows| rows.iter().find(|row| same(row.country)));
    let allowed = politics.and_then(|row| {
        row.has_elections
            .require("election eligibility")
            .ok()
            .copied()
    });
    let interval = politics.and_then(|row| {
        row.election_interval
            .require("election interval")
            .ok()
            .copied()
    });
    let (politics_facts, politics_warning) =
        politics_view(allowed, interval, politics.and_then(next_election_date));
    let host_refused = frame
        .complete::<tables::CountryStabilityInputs>()
        .ok()
        .and_then(|rows| rows.iter().find(|row| same(row.country)))
        .and_then(|row| scalar(&row.exile_host))
        .is_none();
    let (exiles, exile_refusals) = exiles(frame, |raw, host| same(raw) || same(host));
    let mut warnings = manpower.warnings;
    warnings.extend(politics_warning);
    warnings.extend(exile_warnings(host_refused, &exiles, exile_refusals));
    Ok(PeopleReply {
        branch_id: p.branch_id.clone(),
        tag: p.tag.clone(),
        hour: frame.hour,
        read_hour: manpower.read_hour,
        manpower: manpower.facts,
        politics: politics_facts,
        states: manpower.states,
        exiles,
        warnings,
    })
}

/// Every government in exile the hour's sliced reads show, as the exile cards
/// show them: a positive recorded host is an exile, a nonpositive one is not,
/// and an unrecorded host stays unknown (counted first in the refusals).
/// `include` chooses rows by the raw exile and raw host references. The
/// refusals count, in order: unrecorded hosts, then the included exiles'
/// missing legitimacy, manpower, manpower cap and leader resources.
pub fn exiles(
    frame: &DecodedFrame,
    include: impl Fn(u32, u32) -> bool,
) -> (Vec<PeopleExile>, [u32; 5]) {
    let mut refusals = [0_u32; 5];
    let mut rows_out = Vec::new();
    let (Ok(countries), Ok(links), Ok(rows)) = (
        frame.complete::<tables::Countries>(),
        frame.complete::<tables::CountryLinks>(),
        frame.complete::<tables::CountryStabilityInputs>(),
    ) else {
        return (rows_out, refusals);
    };
    let canonical = |raw: u32| {
        links
            .iter()
            .find(|link| link.id == raw)
            .map(|link| link.canonical_slot)
    };
    let equal = |first: u32, second: u32| {
        first == second
            || (first != 0
                && second != 0
                && canonical(first).is_some_and(|slot| canonical(second) == Some(slot)))
    };
    let tag_for = |raw: u32| {
        canonical(raw)
            .and_then(|slot| countries.iter().find(|row| row.canonical_slot == slot))
            .and_then(|row| String::from_utf8(row.tag.clone()).ok())
    };
    let air_wings = frame.complete::<tables::AirWings>().ok();
    let reads = frame.complete::<tables::SliceReads>().ok();
    for row in rows {
        if scalar(&row.exile_host).is_none() {
            refusals[0] += 1;
        }
        if let Some(host) = scalar(&row.exile_host).filter(|host| *host > 0)
            && include(row.country, host as u32)
        {
            for (index, available) in [
                scalar(&row.exile_legitimacy).is_some(),
                scalar(&row.exile_manpower).is_some(),
                scalar(&row.exile_manpower_cap).is_some(),
                scalar(&row.exile_leaders).is_some(),
            ]
            .into_iter()
            .enumerate()
            {
                if !available {
                    refusals[index + 1] += 1;
                }
            }
            rows_out.push(PeopleExile {
                tag: tag_for(row.country).unwrap_or_else(|| format!("Country {}", row.country)),
                host: tag_for(host as u32),
                legitimacy: scalar(&row.exile_legitimacy).map(|value| value.to_string()),
                manpower: scalar(&row.exile_manpower),
                cap: scalar(&row.exile_manpower_cap),
                leaders: scalar(&row.exile_leaders),
                deployed_air: air_wings.and_then(|wings| {
                    exile_air_manpower(wings, |raw| equal(raw, row.country))
                }),
                read_hour: reads.and_then(|reads| {
                    replay_data::sliced::read_hour(
                        reads,
                        TableId::CountryStabilityInputs,
                        row.country,
                    )
                }),
            });
        }
    }
    (rows_out, refusals)
}

#[cfg(any(test, feature = "test-support"))]
fn ordinary_air_manpower(
    wings: &[replay_data::rows::AirWingRow],
    same: impl Fn(u32) -> bool,
) -> Option<i32> {
    let mut total = 0_i32;
    for wing in wings.iter().filter(|wing| same(wing.owner)) {
        if scalar(&wing.exile_country)? <= 0 {
            total = total.wrapping_add(scalar(&wing.acquired_manpower)?);
        }
    }
    Some(total)
}
fn exile_air_manpower(
    wings: &[replay_data::rows::AirWingRow],
    same: impl Fn(u32) -> bool,
) -> Option<i32> {
    let mut total = 0_i32;
    for wing in wings {
        let exile = scalar(&wing.exile_country)?;
        if exile > 0 && same(exile as u32) {
            total = total.wrapping_add(scalar(&wing.acquired_manpower)?);
        }
    }
    Some(total)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn manpower_election_initializers_preserve_the_independent_cache_and_its_refusal() {
        for last in [
            engine_native::cgame_date::NULL_RAW_HOUR,
            engine_native::cpolitical_status::RESET_LAST_ELECTION,
        ] {
            let mut row = replay_data::rows::PoliticsRow {
                last_election: last,
                election_interval: 48.into(),
                next_election_cache: Some(12345).into(),
                ..Default::default()
            };
            assert_eq!(next_election_date(&row), Some(12345));
            row.next_election_cache = None.into();
            assert_eq!(next_election_date(&row), None);
        }
    }
    #[test]
    fn manpower_election_date_derives_native_month_overflow_and_preserves_hours() {
        use engine_native::cdate::HOUR_EPOCH;
        let january_31 = HOUR_EPOCH + 1936 * 365 * 24 + 30 * 24 + 7;
        assert_eq!(
            election_date(january_31, 1),
            HOUR_EPOCH + 1936 * 365 * 24 + 61 * 24 + 7
        );
        assert_eq!(election_date(january_31, 48), january_31 + 4 * 365 * 24);
        assert_eq!(election_date(HOUR_EPOCH - 24, 1), HOUR_EPOCH + 30 * 24);
        assert_eq!(election_date(january_31, 0), january_31);
    }
    #[test]
    fn manpower_mobilisation_preserves_native_order_scaling_and_signed_targets() {
        assert_eq!(recruitment_target(2500, 50_000), 3750);
        assert_eq!(recruitment_target(-2500, 0), 0);
        assert_eq!(mobilisation_change(200_000, 2500, 1000, 0), 1000);
        assert_eq!(mobilisation_change(250_001, 2500, 1000, 0), -1);
        assert_eq!(mobilisation_change(200_000, 2500, 1000, -200_000), 0);
    }
    #[test]
    fn manpower_state_recruitment_keeps_native_order_core_gate_and_signed_clamp() {
        let defines = RecruitmentDefines {
            speed: 1000,
            noncore_base: 2000,
            noncore_max: 100_000,
        };
        assert_eq!(applied_fraction(250_099), 2500);
        assert_eq!(applied_fraction(i64::MIN), 0xffff_ffff);
        assert_eq!(
            state_recruitment(250_000, [50_000, 7000, 1000, 50_000], true, 1000, defines),
            7875
        );
        assert_eq!(
            state_recruitment(250_000, [50_000, 7000, 1000, 50_000], false, 1000, defines),
            787
        );
        assert_eq!(
            state_recruitment(250_000, [-200_000, 0, 0, 0], true, 0, defines),
            0
        );
        assert_eq!(
            state_recruitment(250_000, [0, 0, 200_000, 0], true, 0, defines),
            100_000
        );
    }
    #[test]
    fn manpower_view_uses_native_rosters_first_alias_contributor_and_exile_air_gate() {
        use replay_data::rows::*;
        use replay_frame::frame::{Metrics, TableView};
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 100,
            tables: std::array::from_fn(|index| {
                TableView::Complete(TableRows::empty(TableId::ALL[index])).into()
            }),
            metrics: Metrics::default(),
        };
        let mut set = |id: TableId, rows| {
            *frame.tables[id.index()] = TableView::Complete(rows);
        };
        set(
            TableId::Countries,
            TableRows::Countries(vec![
                CountryRow {
                    id: 101,
                    canonical_slot: 1,
                    tag: b"MOD".to_vec(),
                    manpower_residual: 7,
                    ..Default::default()
                },
                CountryRow {
                    id: 201,
                    canonical_slot: 2,
                    tag: b"HST".to_vec(),
                    ..Default::default()
                },
            ]),
        );
        set(
            TableId::CountryLinks,
            TableRows::CountryLinks(vec![
                CountryLinkRow {
                    id: 101,
                    canonical_slot: 1,
                },
                CountryLinkRow {
                    id: 102,
                    canonical_slot: 1,
                },
                CountryLinkRow {
                    id: 201,
                    canonical_slot: 2,
                },
            ]),
        );
        set(
            TableId::States,
            TableRows::States(vec![StateRow {
                id: 1,
                manpower_available: -9,
                manpower_locked: (-3).into(),
                population: 100_000.into(),
                ..Default::default()
            }]),
        );
        set(
            TableId::CountryCounterInputs,
            TableRows::CountryCounterInputs(vec![CountryCounterInputRow {
                country: 101,
                controlled_states: Some(vec![1]).into(),
                recruitment_modifiers: Some([None; 4]).into(),
                sent_expeditionaries: Some(vec![20, 20, 0, u64::MAX].into()).into(),
                ..Default::default()
            }]),
        );
        set(
            TableId::CountryStabilityInputs,
            TableRows::CountryStabilityInputs(vec![CountryStabilityInputRow {
                country: 101,
                recruitment_ratio: Some(250_000).into(),
                exile_host: Some(201).into(),
                exile_legitimacy: Some(12345).into(),
                exile_manpower: Some(400).into(),
                exile_manpower_cap: Some(900).into(),
                exile_leaders: Some(2).into(),
                ..Default::default()
            }]),
        );
        set(
            TableId::SliceReads,
            TableRows::SliceReads(vec![SliceReadRow {
                table: TableId::CountryStabilityInputs.index() as u32,
                owner: 101,
                sample_hour: 85,
            }]),
        );
        set(
            TableId::StateResourceInputs,
            TableRows::StateResourceInputs(vec![StateResourceInputRow {
                state: 1,
                local_manpower: Some(0).into(),
                local_noncore_manpower: Some(0).into(),
                recruitment_add: Some(0).into(),
                recruitment_factor: Some(0).into(),
                ..Default::default()
            }]),
        );
        set(
            TableId::StateCores,
            TableRows::StateCores(vec![StateCoreRow {
                state: 1,
                cores: vec![102],
                ..Default::default()
            }]),
        );
        set(
            TableId::Divisions,
            TableRows::Divisions(vec![
                DivisionRow {
                    id: 10,
                    template: 100,
                    roster_owner: 101,
                    logical_owner: 102,
                    ..Default::default()
                },
                DivisionRow {
                    id: 20,
                    template: 100,
                    roster_owner: 201,
                    logical_owner: 201,
                    expeditionary_owner: 101,
                    ..Default::default()
                },
                DivisionRow {
                    id: 30,
                    template: 100,
                    roster_owner: 101,
                    logical_owner: 101,
                    ..Default::default()
                },
            ]),
        );
        set(
            TableId::DivisionTemplateIcons,
            TableRows::DivisionTemplateIcons(vec![DivisionTemplateIconRow {
                template: 100,
                manpower_exile: false.into(),
                ..Default::default()
            }]),
        );
        set(
            TableId::DivisionStatus,
            TableRows::DivisionStatus(vec![
                DivisionStatusRow {
                    division: 10,
                    ..Default::default()
                },
                DivisionStatusRow {
                    division: 30,
                    move_flags: DivisionStatusRow::MOVE_EXILED,
                    ..Default::default()
                },
            ]),
        );
        set(
            TableId::DivisionLandInputs,
            TableRows::DivisionLandInputs(vec![
                DivisionLandInputsRow {
                    division: 30,
                    manpower: vec![DivisionManpowerContributor {
                        country: 101,
                        amount: 5,
                    }],
                    ..Default::default()
                },
                DivisionLandInputsRow {
                    division: 10,
                    manpower: vec![
                        DivisionManpowerContributor {
                            country: 102,
                            amount: 11,
                        },
                        DivisionManpowerContributor {
                            country: 101,
                            amount: 17,
                        },
                    ],
                    ..Default::default()
                },
                DivisionLandInputsRow {
                    division: 20,
                    manpower: vec![DivisionManpowerContributor {
                        country: 101,
                        amount: 7,
                    }],
                    ..Default::default()
                },
            ]),
        );
        set(
            TableId::SentVolunteers,
            TableRows::SentVolunteers(vec![SentVolunteerRow {
                country: 102,
                ordinal: 0,
                division: 20,
            }]),
        );
        set(
            TableId::Fleets,
            TableRows::Fleets(vec![
                FleetRow {
                    id: 1,
                    owner: 102,
                    ..Default::default()
                },
                FleetRow {
                    id: 2,
                    owner: 201,
                    ..Default::default()
                },
            ]),
        );
        set(
            TableId::TaskForces,
            TableRows::TaskForces(vec![
                TaskForceRow {
                    id: 1,
                    fleet: 1,
                    ..Default::default()
                },
                TaskForceRow {
                    id: 2,
                    fleet: 2,
                    ..Default::default()
                },
            ]),
        );
        set(
            TableId::Ships,
            TableRows::Ships(vec![
                ShipRow {
                    id: 1,
                    task_force: 1,
                    acquired_manpower: Some(55).into(),
                    required_manpower: Some(77).into(),
                    ..Default::default()
                },
                ShipRow {
                    id: 2,
                    task_force: 2,
                    ..Default::default()
                },
            ]),
        );
        set(
            TableId::AirWings,
            TableRows::AirWings(vec![
                AirWingRow {
                    id: 1,
                    owner: 102,
                    acquired_manpower: Some(22).into(),
                    exile_country: Some(0).into(),
                    ..Default::default()
                },
                AirWingRow {
                    id: 2,
                    owner: 102,
                    acquired_manpower: Some(44).into(),
                    exile_country: Some(201).into(),
                    ..Default::default()
                },
                AirWingRow {
                    id: 3,
                    owner: 201,
                    acquired_manpower: Some(100).into(),
                    exile_country: Some(102).into(),
                    ..Default::default()
                },
            ]),
        );
        set(
            TableId::Politics,
            TableRows::Politics(vec![PoliticsRow {
                country: 102,
                has_elections: true.into(),
                election_interval: 47.into(),
                ..Default::default()
            }]),
        );
        let params = PeopleParams {
            branch_id: "1".into(),
            tag: "MOD".into(),
            hour: 100,
        };
        let reply = read(
            &frame,
            &params,
            Some(RecruitmentDefines {
                speed: 1000,
                noncore_base: 2000,
                noncore_max: 100_000,
            }),
        )
        .unwrap();
        let value = |label| {
            reply
                .manpower
                .iter()
                .find(|fact| fact.label == label)
                .unwrap()
                .value
                .as_deref()
        };
        assert_eq!(value("Available manpower"), Some("-2"));
        // Movement exile does not remove ordinary manpower.
        assert_eq!(value("Deployed army"), Some("37"));
        assert_eq!(value("Effective recruitable population"), Some("2507"));
        assert_eq!(value("Deployed exile air force"), Some("100"));
        assert_eq!(reply.exiles[0].deployed_air, Some(100));
        assert_eq!(value("Deployed navy"), Some("55"));
        assert_eq!(value("Required navy crews"), Some("77"));
        assert_eq!(value("Deployed air force"), Some("22"));
        assert_eq!(reply.states[0].recruitable, Some(2500));
        assert_eq!(reply.read_hour, Some(85));
        assert_eq!(reply.exiles[0].host.as_deref(), Some("HST"));
        assert_eq!(
            (
                reply.exiles[0].manpower,
                reply.exiles[0].cap,
                reply.exiles[0].leaders
            ),
            (Some(400), Some(900), Some(2))
        );
        assert_eq!(
            reply
                .politics
                .iter()
                .find(|fact| fact.label == "Election interval")
                .unwrap()
                .value
                .as_deref(),
            Some("47")
        );
        assert!(reply.warnings.is_empty(), "{:?}", reply.warnings);
        // A known subset must never masquerade as the controlled-state total.
        // Empty membership still has the country's residual, while an absent
        // member refuses the total regardless of its position in the roster.
        for (controlled, expected) in [
            (vec![1, 2], None),
            (vec![2, 1], None),
            (vec![2], None),
            (vec![], Some("7")),
        ] {
            let missing = controlled.contains(&2);
            let mut subset = frame.clone();
            if let TableView::Complete(TableRows::CountryCounterInputs(rows)) =
                &mut *subset.tables[TableId::CountryCounterInputs.index()]
            {
                rows[0].controlled_states = Some(controlled).into();
            }
            let result = read(
                &subset,
                &params,
                Some(RecruitmentDefines {
                    speed: 1000,
                    noncore_base: 2000,
                    noncore_max: 100_000,
                }),
            )
            .unwrap();
            assert_eq!(
                result
                    .manpower
                    .iter()
                    .find(|fact| fact.label == "Effective recruitable population")
                    .unwrap()
                    .value
                    .as_deref(),
                expected
            );
            if missing {
                assert!(
                    result
                        .warnings
                        .iter()
                        .any(|warning| warning.contains("controlled state 2")
                            && warning.contains("effective recruitment")),
                    "{:?}",
                    result.warnings
                );
            }
            // Retained state details remain independently useful and exact.
            for state in result.states {
                assert_eq!(state.recruitable, Some(2500));
            }
        }
        let host = read(
            &frame,
            &PeopleParams {
                tag: "HST".into(),
                ..params.clone()
            },
            None,
        )
        .unwrap();
        assert_eq!(host.exiles[0].deployed_air, Some(100));
        assert_eq!(
            host.manpower
                .iter()
                .find(|fact| fact.label == "Deployed air force")
                .unwrap()
                .value
                .as_deref(),
            Some("0")
        );

        // The same template gate excludes ordinary, sent-reference (including
        // duplicates), and volunteer contributions without a movement-bit proxy.
        let mut exiled = frame.clone();
        if let TableView::Complete(TableRows::DivisionTemplateIcons(rows)) =
            &mut *exiled.tables[TableId::DivisionTemplateIcons.index()]
        {
            rows[0].manpower_exile = true.into();
        }
        let result = read(&exiled, &params, None).unwrap();
        assert_eq!(
            result
                .manpower
                .iter()
                .find(|fact| fact.label == "Deployed army")
                .unwrap()
                .value
                .as_deref(),
            Some("0")
        );
        if let TableView::Complete(TableRows::DivisionTemplateIcons(rows)) =
            &mut *exiled.tables[TableId::DivisionTemplateIcons.index()]
        {
            rows[0].manpower_exile = Recorded::NotRecorded;
        }
        let result = read(&exiled, &params, None).unwrap();
        assert_eq!(
            result
                .manpower
                .iter()
                .find(|fact| fact.label == "Deployed army")
                .unwrap()
                .value,
            None
        );

        let mut wrong_owner = frame.clone();
        if let TableView::Complete(TableRows::Divisions(rows)) =
            &mut *wrong_owner.tables[TableId::Divisions.index()]
        {
            rows[0].logical_owner = 201;
        }
        let result = read(&wrong_owner, &params, None).unwrap();
        assert_eq!(
            result
                .manpower
                .iter()
                .find(|fact| fact.label == "Deployed army")
                .unwrap()
                .value
                .as_deref(),
            Some("26")
        );
        if let TableView::Complete(TableRows::Divisions(rows)) =
            &mut *wrong_owner.tables[TableId::Divisions.index()]
        {
            rows[0].logical_owner = 102;
            rows[0].expeditionary_owner = 201;
        }
        let result = read(&wrong_owner, &params, None).unwrap();
        assert_eq!(
            result
                .manpower
                .iter()
                .find(|fact| fact.label == "Deployed army")
                .unwrap()
                .value
                .as_deref(),
            Some("26")
        );

        let mut wrapping = frame.clone();
        if let TableView::Complete(TableRows::Countries(rows)) =
            &mut *wrapping.tables[TableId::Countries.index()]
        {
            rows[0].manpower_residual = i32::MAX;
        }
        let result = read(
            &wrapping,
            &params,
            Some(RecruitmentDefines {
                speed: 1000,
                noncore_base: 2000,
                noncore_max: 100_000,
            }),
        )
        .unwrap();
        assert_eq!(
            result
                .manpower
                .iter()
                .find(|fact| fact.label == "Effective recruitable population")
                .unwrap()
                .value,
            Some(i32::MAX.wrapping_add(2500).to_string())
        );
    }
    #[test]
    fn manpower_air_channels_preserve_aliases_signed_wrapping_and_refusals() {
        use replay_data::rows::AirWingRow;
        let mut wings = vec![
            AirWingRow {
                owner: 201,
                exile_country: Some(102).into(),
                acquired_manpower: Some(i32::MAX).into(),
                ..Default::default()
            },
            AirWingRow {
                owner: 201,
                exile_country: Some(101).into(),
                acquired_manpower: Some(1).into(),
                ..Default::default()
            },
        ];
        let alias = |raw| matches!(raw, 101 | 102);
        assert_eq!(ordinary_air_manpower(&wings, |raw| raw == 201), Some(0));
        assert_eq!(exile_air_manpower(&wings, alias), Some(i32::MIN));
        wings[0].acquired_manpower = None.into();
        assert_eq!(exile_air_manpower(&wings, alias), None);
        wings[0].exile_country = None.into();
        assert_eq!(exile_air_manpower(&wings, |_| false), None);
        assert_eq!(ordinary_air_manpower(&wings, |raw| raw == 201), None);
    }
}
