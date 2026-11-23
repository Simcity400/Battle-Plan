
use dashboard_base::products::Country;
use dashboard_base::products::{LoadContext, finite_fixed64, at_hour::{At, AtHour, Owner, Read}};
use dashboard_base::Source;
use content_reader_layers::content::diagnostics::Once as OnceLock;
use replay_data::rows::{CountryLinkRow, Recorded, TableId, tables};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PeopleParams {
    pub branch_id: String,
    pub country: Country,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct PeopleFact {
    pub label: String,
    pub value: Option<String>,
    pub unit: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct PeopleState {
    pub state: u32,
    pub available: i32,
    pub locked: Option<i32>,
    pub total: Option<i32>,
    pub effective: Option<String>,
    pub recruitable: Option<i32>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
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

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct PeopleReply {
    pub branch_id: String,
    pub tag: String,
    #[doc = " The country the request named, which a reply is matched to: a slot follows its country through any tag."]
    pub asked: dashboard_base::products::Country,
    pub hour: i32,
    pub read_hour: Option<i32>,
    pub manpower: Vec<PeopleFact>,
    pub politics: Vec<PeopleFact>,
    pub states: Vec<PeopleState>,
    pub exiles: Vec<PeopleExile>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(PeopleReply);

fn scalar<T: Copy>(value: &Recorded<Option<T>>) -> Option<T> {
    value.require("people operand").ok().copied().flatten()
}

fn recorded<T: Copy>(value: &Recorded<T>) -> Option<T> {
    value.require("people operand").ok().copied()
}

fn fact(label: &str, value: Option<impl ToString>, unit: &str) -> PeopleFact {
    PeopleFact { label: label.into(), value: value.map(|v| v.to_string()), unit: unit.into() }
}

pub fn recruitment_target(conscription: i64, factor: i64) -> i64 {
    factor.wrapping_add(100_000).wrapping_mul(conscription).wrapping_div(100_000).max(0)
}

pub fn mobilisation_change(applied: i64, target: i64, base: i64, factor: i64) -> i64 {
    let target = target.wrapping_mul(100);
    if applied == target {
        return 0;
    }
    let speed = factor.wrapping_add(100_000).wrapping_mul(base).wrapping_div(100_000);
    let distance = applied.wrapping_sub(target).wrapping_abs();
    let step = speed.min(distance).max(0);
    if target <= applied { step.wrapping_neg() } else { step }
}

#[derive(Clone, Copy)]
pub struct RecruitmentDefines {
    pub speed: i64,
    pub noncore_base: i64,
    pub noncore_max: i64,
}

pub fn content_defines(layers: &content_reader_layers::ResolvedLayers) -> Result<RecruitmentDefines, String> {
    let held = layers.derived::<OnceLock<Result<RecruitmentDefines, String>>>();
    held.get_or_init(|| {
        let load = || -> io::Result<RecruitmentDefines> {
            let defines = content_reader_defines::defines::read(layers)?;
            Ok(RecruitmentDefines {
                speed: defines.fixed5("NCountry", "BASE_MOBILIZATION_SPEED")?,
                noncore_base: defines.fixed5("NCountry", "LOCAL_MANPOWER_ACCESSIBLE_NON_CORE_FACTOR")?,
                noncore_max: defines.fixed5("NCountry", "MAX_NON_CORE_MANPOWER_FACTOR")?,
            })
        };
        load().map_err(|why| why.to_string())
    })
    .clone()
}

pub fn applied_fraction(raw: i64) -> i64 {
    if finite_fixed64(raw) { raw.wrapping_mul(100_000) / 10_000_000 } else { 0xffff_ffff }
}

pub fn state_recruitment(applied: i64, values: [i64; 4], core: bool, noncore: i64, defines: RecruitmentDefines) -> i64 {
    let mut value = applied_fraction(applied).wrapping_add(values[2]);
    value = value.wrapping_mul(values[3].wrapping_add(100_000)) / 100_000;
    value = value.wrapping_mul(values[0].wrapping_add(100_000)) / 100_000;
    if !core {
        value = value.wrapping_mul(defines.noncore_base.wrapping_add(values[1]).wrapping_add(noncore).min(defines.noncore_max)) / 100_000;
    }
    value.clamp(0, 100_000)
}

pub fn election_date(last: i32, interval: i32) -> i32 {
    use game_model::calendar::{HOUR_EPOCH, MONTH_LENGTHS};
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
        let shifted = if year < 0 { absolute.wrapping_neg() } else { absolute };
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
    let ordinal = (days % 365).wrapping_sub(old_prefix).wrapping_add(new_prefix);
    let absolute = year.wrapping_abs().wrapping_mul(365).wrapping_add(ordinal);
    let shifted = if year < 0 { absolute.wrapping_neg() } else { absolute };
    hour.wrapping_add(shifted.wrapping_sub(days).wrapping_mul(24))
}

pub fn next_election(last: i32, interval: Option<i32>, cache: Option<Option<i32>>) -> Option<i32> {
    if engine_native_politics::cpolitical_status::needs_election_cache(last) {
        cache.flatten()
    } else {
        interval.map(|interval| election_date(last, interval))
    }
}

pub fn next_election_date(row: &replay_data::rows::PoliticsRow) -> Option<i32> {
    next_election(row.last_election, recorded(&row.election_interval), recorded(&row.next_election_cache))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StateOperands {
    pub available: i32,
    pub locked: Option<i32>,
    pub total: Option<i32>,
    pub resources: Option<[i64; 4]>,
    pub core: Option<bool>,
}

pub const DEPLOYED: [&str; 5] = ["Deployed army", "Deployed navy", "Required navy crews", "Deployed air force", "Deployed exile air force"];

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Operands {
    pub residual: i32,
    pub states: Option<Vec<(u32, Option<StateOperands>)>>,
    pub applied: Option<i64>,
    pub read_hour: Option<i32>,
    pub effective: Option<[i64; 4]>,
    pub deployed: [Option<i32>; 5],
}

pub const RECRUITMENT_MODIFIERS: [&str; 4] = ["mobilization_speed", "conscription", "conscription_factor", "non_core_manpower"];

pub struct ManpowerView {
    pub facts: Vec<PeopleFact>,
    pub states: Vec<PeopleState>,
    pub read_hour: Option<i32>,
    pub warnings: Vec<String>,
}

pub fn manpower_view(operands: &Operands, defines: Option<RecruitmentDefines>) -> ManpowerView {
    let mut view = ManpowerView { facts: Vec::new(), states: Vec::new(), read_hour: operands.read_hour, warnings: Vec::new() };
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
                view.states.push(PeopleState { state: *id, available: row.available, locked: row.locked, total: row.total, effective: None, recruitable: None });
                resources.push((row.resources, row.core));
            }
        }
        None => {
            available = None;
            locked = None;
            total = None;
            view.warnings.push("1 refused manpower pool: controlled-state roster or States unavailable".into());
        }
    }
    view.facts.extend([fact("Available manpower", available, "people"), fact("Locked population", locked, "people"), fact("Total population", total, "people")]);
    let applied = operands.applied;
    view.facts.push(fact("Applied recruitment ratio", applied.map(applied_fraction), "ratio"));
    if applied.is_none() {
        view.warnings.push("1 refused applied recruitment ratio: country operand unavailable".into());
    }
    let effective = operands.effective;
    let target = effective.map(|values| recruitment_target(values[1], values[2]));
    let daily = applied
        .zip(target)
        .zip(defines.map(|value| value.speed))
        .zip(effective)
        .map(|(((applied, target), base), values)| mobilisation_change(applied, target, base, values[0]));
    view.facts.push(fact("Recruitment target", target, "ratio"));
    view.facts.push(fact("Daily mobilisation", daily.map(applied_fraction), "dailyRatio"));
    view.facts.push(fact("Mobilising reserves", target.zip(applied).map(|(target, applied)| target.wrapping_sub(applied_fraction(applied))), "ratio"));
    if effective.is_none() {
        view.warnings.push("1 refused effective recruitment: country modifier operands or definitions unavailable".into());
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
        recruitable = recruitable.zip(state.recruitable).map(|(total, value)| total.wrapping_add(value));
    }
    if operands.states.as_ref().is_none_or(|states| states.len() != view.states.len()) {
        recruitable = None;
    }
    view.facts.push(fact("Effective recruitable population", recruitable, "people"));
    for (label, value) in DEPLOYED.into_iter().zip(operands.deployed) {
        view.facts.push(fact(label, value, "people"));
        if value.is_none() {
            view.warnings.push(format!("1 refused {label}: required unit manpower or exact roster membership unavailable"));
        }
    }
    if recruitable.is_none() {
        view.warnings.push("1 refused effective recruitment: state recruitment operands, cores or recorded content unavailable".into());
    }
    view
}

pub fn politics_view(allowed: Option<bool>, interval: Option<i32>, next: Option<i32>) -> (Vec<PeopleFact>, Option<String>) {
    let facts = vec![
        fact("Elections", allowed.map(|value| if value { "Allowed" } else { "Not held" }), "text"),
        fact("Election interval", interval, "months"),
        if allowed == Some(false) { fact("Next election", Some("Not held"), "text") } else { fact("Next election", next, "date") },
    ];
    let refused = allowed.is_none() || interval.is_none() || (allowed == Some(true) && next.is_none());
    (facts, refused.then(|| "1 refused election details: Politics operands unavailable".into()))
}

pub fn exile_warnings(host_refused: bool, exiles: &[PeopleExile], refusals: [u32; 5]) -> Vec<String> {
    let mut warnings = Vec::new();
    if host_refused {
        warnings.push("1 refused exile status: country host unavailable".into());
    }
    for (count, field) in refusals.into_iter().zip(["host roster", "legitimacy", "manpower", "manpower cap", "leader resources"]) {
        if count != 0 {
            warnings.push(format!("{count} refused exile {field}: operands unavailable"));
        }
    }
    let missing_air = exiles.iter().filter(|exile| exile.deployed_air.is_none()).count();
    if missing_air != 0 {
        warnings.push(format!("{missing_air} refused exile deployed air crews: wing attribution or manpower unavailable"));
    }
    warnings
}

pub(crate) struct Links<'a> {
    links: &'a [CountryLinkRow],
    tags: BTreeMap<u32, &'a [u8]>,
}

impl<'a> Links<'a> {
    pub(crate) fn new(frame: &'a DecodedFrame) -> io::Result<Self> {
        let countries = frame.complete::<tables::Countries>()?;
        let links = frame.complete::<tables::CountryLinks>()?;
        Ok(Self { links, tags: countries.iter().map(|row| (row.canonical_slot, row.tag.as_slice())).collect() })
    }

    pub(crate) fn slot(&self, raw: u32) -> Option<u32> {
        self.links.binary_search_by_key(&raw, |link| link.id).ok().map(|at| self.links[at].canonical_slot)
    }

    pub(crate) fn equal(&self, first: u32, second: u32) -> bool {
        first == second || (first != 0 && second != 0 && self.slot(first).is_some_and(|slot| self.slot(second) == Some(slot)))
    }

    pub(crate) fn slot_tag(&self, slot: u32) -> Option<String> {
        self.tags.get(&slot).and_then(|tag| std::str::from_utf8(tag).ok()).filter(|tag| !tag.is_empty()).map(str::to_owned)
    }

    pub(crate) fn tag(&self, raw: u32) -> Option<String> {
        self.slot_tag(self.slot(raw)?)
    }
}

pub const EXILE_TABLES: [TableId; 5] = [TableId::Countries, TableId::CountryLinks, TableId::CountryStabilityInputs, TableId::SliceReads, TableId::CountryDeployedManpower];

pub fn exiles(frame: &DecodedFrame, include: impl Fn(u32, u32) -> bool) -> (Vec<PeopleExile>, [u32; 5]) {
    let (rows, refusals) = exile_rows(frame, include);
    (rows.into_iter().map(|row| row.exile).collect(), refusals)
}

pub struct ExileAt {
    pub country: u32,
    pub slot: Option<u32>,
    pub exile: PeopleExile,
}

pub fn exile_rows(frame: &DecodedFrame, include: impl Fn(u32, u32) -> bool) -> (Vec<ExileAt>, [u32; 5]) {
    let mut refusals = [0_u32; 5];
    let mut out = Vec::new();
    let (Ok(links), Ok(rows)) = (Links::new(frame), frame.complete::<tables::CountryStabilityInputs>()) else {
        return (out, refusals);
    };
    let deployed = frame.complete::<tables::CountryDeployedManpower>().ok();
    let reads = frame.complete::<tables::SliceReads>().ok();
    for row in rows {
        if scalar(&row.exile_host).is_none() {
            refusals[0] += 1;
        }
        let Some(host) = scalar(&row.exile_host).filter(|host| *host > 0) else { continue };
        if !include(row.country, host as u32) {
            continue;
        }
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
        let deployed_air = links.slot(row.country).and_then(|slot| {
            let rows = deployed?;
            let at = rows.binary_search_by_key(&slot, |row| row.slot).ok()?;
            recorded(&rows[at].exile_air)
        });
        out.push(ExileAt { country: row.country, slot: links.slot(row.country), exile: PeopleExile {
            tag: links.tag(row.country).unwrap_or_else(|| format!("Country {}", row.country)),
            host: links.tag(host as u32),
            legitimacy: scalar(&row.exile_legitimacy).map(|value| value.to_string()),
            manpower: scalar(&row.exile_manpower),
            cap: scalar(&row.exile_manpower_cap),
            leaders: scalar(&row.exile_leaders),
            deployed_air,
            read_hour: reads.and_then(|reads| replay_data::sliced::read_hour(reads, TableId::CountryStabilityInputs, row.country)),
        } });
    }
    (out, refusals)
}

fn operands(frame: &DecodedFrame, residual: i32, slot: u32, same: impl Fn(u32) -> bool) -> Operands {
    let counters = frame.complete::<tables::CountryCounterInputs>().ok().and_then(|rows| rows.iter().find(|row| same(row.country)));
    let state_rows = frame.complete::<tables::States>().ok();
    let roster = counters.and_then(|row| row.controlled_states.require("controlled states").ok()).and_then(Option::as_deref);
    let resource_inputs = frame.complete::<tables::StateResourceInputs>().ok();
    let cores = frame.complete::<tables::StateCores>().ok();
    let states = roster.zip(state_rows).map(|(roster, rows)| {
        let rows: BTreeMap<_, _> = rows.iter().map(|row| (row.id, row)).collect();
        let resource_inputs: Option<BTreeMap<_, _>> = resource_inputs.map(|rows| rows.iter().map(|row| (row.state, row)).collect());
        let cores: Option<BTreeMap<_, _>> = cores.map(|rows| rows.iter().map(|row| (row.state, row)).collect());
        roster
            .iter()
            .map(|id| {
                let row = rows.get(id).map(|row| StateOperands {
                    available: row.manpower_available,
                    locked: row.manpower_locked.require("locked").ok().copied(),
                    total: row.population.require("population").ok().copied(),
                    resources: (|| {
                        let values = resource_inputs.as_ref()?.get(id)?;
                        Some([scalar(&values.local_manpower)?, scalar(&values.local_noncore_manpower)?, scalar(&values.recruitment_add)?, scalar(&values.recruitment_factor)?])
                    })(),
                    core: cores.as_ref().and_then(|cores| cores.get(id)).map(|row| row.cores.iter().any(|&raw| same(raw))),
                });
                (*id, row)
            })
            .collect()
    });
    let input = frame.complete::<tables::CountryStabilityInputs>().ok().and_then(|rows| rows.iter().find(|row| same(row.country)));
    let reads = frame.complete::<tables::SliceReads>().ok();
    let read_hour = reads.and_then(|reads| input.and_then(|row| replay_data::sliced::read_hour(reads, TableId::CountryStabilityInputs, row.country)));
    let modifiers = counters.and_then(|row| row.recruitment_modifiers.require("recruitment modifiers").ok()).copied().flatten();
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
    let deployed = frame
        .complete::<tables::CountryDeployedManpower>()
        .ok()
        .and_then(|rows| rows.binary_search_by_key(&slot, |row| row.slot).ok().map(|at| &rows[at]));
    Operands {
        residual,
        states,
        applied: input.and_then(|row| scalar(&row.recruitment_ratio)),
        read_hour,
        effective,
        deployed: match deployed {
            Some(row) => [recorded(&row.army), recorded(&row.navy), recorded(&row.navy_required), recorded(&row.air), recorded(&row.exile_air)],
            None => [None; 5],
        },
    }
}

fn aliases(frame: &DecodedFrame, owner: Owner) -> BTreeSet<u32> {
    let mut out = BTreeSet::from([owner.id]);
    if let Ok(links) = frame.complete::<tables::CountryLinks>() {
        out.extend(links.iter().filter(|link| link.canonical_slot == owner.slot).map(|link| link.id));
    }
    out
}

pub const PEOPLE_TABLES: [TableId; 13] = [
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CountryCounterInputs,
    TableId::States,
    TableId::StateResourceInputs,
    TableId::StateCores,
    TableId::CountryStabilityInputs,
    TableId::SliceReads,
    TableId::ModifierDefinitions,
    TableId::LexicalTokens,
    TableId::SimulationFeatures,
    TableId::Politics,
    TableId::CountryDeployedManpower,
];

pub fn read(frame: &DecodedFrame, branch_id: &str, asked: &dashboard_base::products::Country, tag: &str, owner: Owner, defines: Option<RecruitmentDefines>) -> io::Result<PeopleReply> {
    let links = Links::new(frame)?;
    let countries = frame.complete::<tables::Countries>()?;
    let country = countries
        .iter()
        .find(|row| row.id == owner.id)
        .ok_or_else(|| dashboard_core::refused("Country unavailable at this hour"))?;
    let same = |raw: u32| links.equal(raw, owner.id);
    let manpower = manpower_view(&operands(frame, country.manpower_residual, owner.slot, same), defines);
    let politics = frame.complete::<tables::Politics>().ok().and_then(|rows| rows.iter().find(|row| same(row.country)));
    let exists = frame.complete::<tables::CountryCounterInputs>().ok().and_then(|rows| rows.iter().find(|row| same(row.country))).map(|row| row.owned_state_count > 0);
    let allowed = politics.and_then(|row| match (recorded(&row.has_elections)?, exists) {
        (false, _) | (true, Some(false)) => Some(false),
        (true, Some(true)) => Some(true),
        (true, None) => None,
    });
    let interval = politics.and_then(|row| recorded(&row.election_interval));
    let (politics_facts, politics_warning) = politics_view(allowed, interval, politics.and_then(next_election_date));
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
        branch_id: branch_id.to_owned(),
        tag: tag.to_owned(),
        asked: asked.clone(),
        hour: frame.hour,
        read_hour: manpower.read_hour,
        manpower: manpower.facts,
        politics: politics_facts,
        states: manpower.states,
        exiles,
        warnings,
    })
}

pub struct CountryPeople;

pub struct PeopleContext {
    defines: Result<RecruitmentDefines, String>,
}

impl AtHour for CountryPeople {
    const KEY: &'static str = "country-people";
    const TITLE: &'static str = "Country people";
    const CONTENT_DEPENDENT: bool = true;
    type Params = PeopleParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, country];
    type Reply = PeopleReply;
    type Context = PeopleContext;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![PeopleParams::decl(), PeopleReply::decl(), PeopleFact::decl(), PeopleState::decl(), PeopleExile::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid people branch")?;
        dashboard_base::products::validate_country(&params.country)
    }

    fn owner(params: &Self::Params) -> Option<&Country> {
        Some(&params.country)
    }

    fn tables(_: &Self::Params) -> Vec<TableId> {
        PEOPLE_TABLES.to_vec()
    }

    fn reads(_: &Self::Params, owner: Option<Owner>, step: usize, held: &DecodedFrame) -> Vec<Read> {
        let Some(owner) = owner else { return Vec::new() };
        match step {
            0 => [TableId::Countries, TableId::CountryLinks, TableId::CountryStabilityInputs, TableId::SliceReads, TableId::SimulationFeatures]
                .map(Read::all)
                .into(),
            1 => {
                let aliases = aliases(held, owner);
                let mut slots = BTreeSet::from([owner.slot]);
                if let Ok(links) = Links::new(held) {
                    let rows = held.complete::<tables::CountryStabilityInputs>().ok().into_iter().flatten();
                    for row in rows {
                        let Some(host) = scalar(&row.exile_host).filter(|host| *host > 0) else { continue };
                        if aliases.contains(&row.country) || aliases.contains(&(host as u32)) {
                            slots.extend(links.slot(row.country));
                        }
                    }
                }
                vec![
                    Read::these(TableId::CountryCounterInputs, aliases.iter().map(|&id| u128::from(id))),
                    Read::these(TableId::Politics, aliases.iter().map(|&id| u128::from(id))),
                    Read::these(TableId::CountryDeployedManpower, slots.into_iter().map(u128::from)),
                ]
            }
            2 => {
                let aliases = aliases(held, owner);
                let counters = held.complete::<tables::CountryCounterInputs>().ok().and_then(|rows| rows.iter().find(|row| aliases.contains(&row.country)));
                let roster: Vec<u32> = counters
                    .and_then(|row| row.controlled_states.require("controlled states").ok())
                    .and_then(Option::as_deref)
                    .map(<[u32]>::to_vec)
                    .unwrap_or_default();
                let keys = || roster.iter().map(|&id| u128::from(id));
                let mut reads = vec![
                    Read::these(TableId::States, keys()),
                    Read::these(TableId::StateResourceInputs, keys()),
                    Read::these(TableId::StateCores, keys()),
                ];
                if counters.and_then(|row| row.recruitment_modifiers.require("recruitment modifiers").ok().copied().flatten()).is_some() {
                    reads.extend([TableId::ModifierDefinitions, TableId::LexicalTokens].map(Read::all));
                }
                reads
            }
            _ => Vec::new(),
        }
    }

    fn context(source: &Source, load: &LoadContext) -> io::Result<Self::Context> {
        let defines = load
            .content()
            .and_then(|content| dashboard_base::catalogue::catalogue(source, content))
            .map_err(|error| error.to_string())
            .and_then(|catalogue| content_defines(&catalogue.content.layers));
        Ok(PeopleContext { defines })
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let owner = at.owner.ok_or_else(|| dashboard_core::refused("Country people names no country"))?;
        let mut reply = read(frame, &params.branch_id, &params.country, &at.holders.name(&params.country, at.hour), owner, at.context.defines.as_ref().ok().copied())?;
        if let Err(reason) = &at.context.defines {
            reply.warnings.push(format!("1 refused mobilisation rate: {reason}"));
        }
        Ok(reply)
    }
}

#[cfg(test)]
mod tests;
