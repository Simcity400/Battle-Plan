//! Production-select: chosen or top countries at an hour, ranked over the
//! endpoint's holders and grouped by the classes content gives the variants.

use super::facts::{Endpoint, Holder, Leaf};
use super::read::{Class, Classes, Index, State};
use super::{IC_SEEN, UNITS_SEEN, check_cancel, predecessor, seen_in};
use crate::Source;
use crate::products::equipment::production::Purpose;
use crate::products::equipment::{self, Domain, StockVariantName};
use crate::products::equipment_produced::{
    ProducedCount, ProducedGrouping, ProducedKey, ProducedMeasure, ProducedParams, ProducedType,
};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::io;
use std::sync::{Arc, atomic::AtomicBool};

#[derive(Clone, Debug, Serialize, PartialEq, Eq, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "PreparedProductionValue")]
pub struct SelectedValue {
    pub key: String,
    pub amount: Option<String>,
    pub equipment_amount: Option<String>,
}
#[derive(Clone, Debug, Serialize, PartialEq, Eq, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "PreparedProductionCountry")]
pub struct SelectedCountry {
    pub tag: String,
    pub slot: u32,
    pub country_id: u32,
    pub segment: String,
    pub total: Option<String>,
    pub equipment_total: Option<String>,
    pub read_hour: i32,
    pub observation: Option<i32>,
    pub values: Vec<SelectedValue>,
}
#[derive(Clone, Debug, Serialize, PartialEq, Eq, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "PreparedProductionSelection")]
pub struct SelectionPage {
    pub schema: u32,
    pub branch_id: String,
    pub first_hour: i32,
    pub last_hour: i32,
    pub known: bool,
    pub keys: Vec<ProducedKey>,
    pub types: Vec<ProducedType>,
    pub total_countries: usize,
    /// Logical first rank in this page; quantities remain exact until geometry.
    pub offset: String,
    pub value_min: String,
    pub value_max: String,
    pub countries: Vec<SelectedCountry>,
    /// Totals by type before paging, for the automatic units choice. Only a
    /// total-by-type units selection supplies them; unknown amounts add nothing.
    pub type_totals: Option<Vec<(String, String)>>,
    /// The hour every running total at this page began counting, whichever countries are selected.
    pub observation: Option<i32>,
    /// Whether the branch prepared so far has read production output at all.
    pub recorded: bool,
    pub warnings: Vec<String>,
}

/// A source at an endpoint, its reads resolved there.
#[derive(Clone, Copy)]
struct SourceAt<'a> {
    slot: u32,
    country_id: u32,
    observation: Option<i32>,
    segment: u64,
    read_hour: i32,
    units_known: bool,
    spending_known: bool,
    leaves: &'a [Leaf],
}

fn source_at<'a>(
    endpoint: &Endpoint,
    holder: Option<&'a Holder>,
    rocket: bool,
) -> io::Result<Option<SourceAt<'a>>> {
    let Some(source) = holder.and_then(|holder| {
        if rocket {
            holder.rocket.as_ref()
        } else {
            holder.equipment.as_ref()
        }
    }) else {
        return Ok(None);
    };
    let head = &source.head;
    Ok(Some(SourceAt {
        slot: head.slot,
        country_id: head.country_id,
        observation: head.observation,
        segment: head.segment,
        read_hour: endpoint.read_of(head, rocket)?,
        units_known: head.units_known,
        spending_known: head.spending_known,
        leaves: &source.leaves,
    }))
}

fn compatible(now: &SourceAt<'_>, before: Option<&SourceAt<'_>>, measure: ProducedMeasure) -> bool {
    if measure == ProducedMeasure::Total {
        return true;
    }
    let Some(target) = predecessor(now.read_hour, measure) else {
        return false;
    };
    before.is_some_and(|before| {
        before.read_hour == target
            && (now.slot, now.country_id, now.observation, now.segment)
                == (
                    before.slot,
                    before.country_id,
                    before.observation,
                    before.segment,
                )
    })
}

fn known(source: &SourceAt<'_>, count: ProducedCount) -> bool {
    match count {
        ProducedCount::Units => source.units_known,
        ProducedCount::Ic => source.spending_known,
    }
}

/// Per variant id: what a request counts of it.
const COUNTED: u8 = 1;
const OUTSIDE: u8 = 2;
const UNSURE: u8 = 4;
const ZERO: u8 = 8;

/// One predicate's sums over a source and its predecessor.
#[derive(Clone, Copy)]
struct Acc {
    now: i128,
    exact: bool,
    before: i128,
}

impl Default for Acc {
    fn default() -> Self {
        Self {
            now: 0,
            exact: true,
            before: 0,
        }
    }
}

#[derive(Default)]
struct Sums {
    counted: Acc,
    outside: Acc,
    unsure: Acc,
    zero: Acc,
    groups: Vec<Acc>,
}

/// What a request reads of every variant id: its flags and group.
struct Plan {
    flags: Vec<u8>,
    group: Vec<Option<u32>>,
    refits: bool,
    count: ProducedCount,
    measure: ProducedMeasure,
}

impl Plan {
    fn sums(&self, source: &SourceAt<'_>, before: Option<&SourceAt<'_>>, groups: usize) -> Sums {
        let mut sums = Sums {
            groups: vec![Acc::default(); groups],
            ..Default::default()
        };
        let mut add = |leaf: &Leaf, current: bool| {
            let variant = leaf.variant as usize;
            let flags = self.flags.get(variant).copied().unwrap_or(0);
            let pass = self.refits || leaf.purpose != Purpose::Refit;
            let (value, exact) = match self.count {
                ProducedCount::Units => (i128::from(leaf.units), leaf.units_exact),
                ProducedCount::Ic => (i128::from(leaf.spent), leaf.reason.is_none()),
            };
            let group = self
                .group
                .get(variant)
                .copied()
                .flatten()
                .filter(|_| pass)
                .and_then(|group| sums.groups.get_mut(group as usize));
            let accs = [
                (pass && flags & COUNTED != 0).then_some(&mut sums.counted),
                (pass && flags & OUTSIDE != 0).then_some(&mut sums.outside),
                (pass && flags & UNSURE != 0).then_some(&mut sums.unsure),
                (flags & ZERO != 0).then_some(&mut sums.zero),
                group,
            ];
            for acc in accs.into_iter().flatten() {
                if current {
                    acc.now += value;
                    acc.exact &= exact;
                } else {
                    acc.before += value;
                }
            }
        };
        for leaf in source.leaves {
            add(leaf, true);
        }
        if self.measure != ProducedMeasure::Total
            && let Some(before) = before
        {
            for leaf in before.leaves {
                add(leaf, false);
            }
        }
        sums
    }

    /// A predicate's amount: unknown where its source is not, or a counted
    /// leaf now is not exact.
    fn value(&self, acc: &Acc, ok: bool) -> Option<i128> {
        if !ok || !acc.exact {
            return None;
        }
        Some(if self.measure == ProducedMeasure::Total {
            acc.now
        } else {
            acc.now - acc.before
        })
    }
}

fn combined(left: Option<i128>, right: Option<i128>) -> Option<i128> {
    Some(left? + right?)
}

/// One country a request evaluates.
struct Row<'a> {
    tag: &'a str,
    equipment: SourceAt<'a>,
    equipment_before: Option<SourceAt<'a>>,
    /// The Rocket source joined to its supplying equipment read.
    rocket: Option<SourceAt<'a>>,
    rocket_before: Option<SourceAt<'a>>,
}

/// A group values are split by: its key, whether an unresolved variant
/// makes it unknown, and its first variant.
struct Group<'a> {
    key: String,
    ambiguous: bool,
    first: u32,
    class: &'a Class,
}

struct Evaluated {
    total: Option<i128>,
    country: SelectedCountry,
    covers: bool,
}

fn evaluate(
    plan: &Plan,
    groups: &[Group<'_>],
    row: &Row<'_>,
    hydrate: bool,
    cancelled: &AtomicBool,
) -> io::Result<Evaluated> {
    check_cancel(cancelled)?;
    let width = if hydrate { groups.len() } else { 0 };
    let equipment = &row.equipment;
    let e_ok = compatible(equipment, row.equipment_before.as_ref(), plan.measure)
        && known(equipment, plan.count);
    let e = plan.sums(equipment, row.equipment_before.as_ref(), width);
    let rocket = row.rocket.as_ref().filter(|rocket| {
        (rocket.slot, rocket.country_id) == (equipment.slot, equipment.country_id)
            && match (equipment.observation, rocket.observation) {
                (Some(left), Some(right)) => left == right,
                _ => true,
            }
    });
    let r_ok = rocket.is_some_and(|rocket| {
        compatible(rocket, row.rocket_before.as_ref(), plan.measure) && known(rocket, plan.count)
    });
    let r = rocket.map_or_else(
        || Sums {
            groups: vec![Acc::default(); width],
            ..Default::default()
        },
        |rocket| plan.sums(rocket, row.rocket_before.as_ref(), width),
    );
    let scope_known = plan.value(&r.zero, r_ok).is_some();
    let rocket_value = |value: Option<i128>| if scope_known { value } else { None };
    let e_outside = plan.value(&e.outside, e_ok);
    let e_total = plan.value(&e.counted, e_ok).filter(|_| e_outside == Some(0));
    let r_outside = plan.value(&r.outside, r_ok);
    let r_total = rocket_value(plan.value(&r.counted, r_ok).filter(|_| r_outside == Some(0)));
    let total = combined(e_total, r_total);
    let covers = e_ok;
    let mut values = Vec::with_capacity(width);
    if hydrate {
        let e_unsure = plan.value(&e.unsure, e_ok);
        let r_unsure = plan.value(&r.unsure, r_ok);
        for (index, group) in groups.iter().enumerate() {
            let e_amount = plan
                .value(&e.groups[index], e_ok)
                .filter(|_| !group.ambiguous || e_unsure == Some(0));
            let r_amount = rocket_value(
                plan.value(&r.groups[index], r_ok)
                    .filter(|_| !group.ambiguous || r_unsure == Some(0)),
            );
            values.push(SelectedValue {
                key: group.key.clone(),
                amount: combined(e_amount, r_amount).map(|value| value.to_string()),
                equipment_amount: e_amount.map(|value| value.to_string()),
            });
        }
    }
    Ok(Evaluated {
        total,
        country: SelectedCountry {
            tag: row.tag.to_owned(),
            slot: equipment.slot,
            country_id: equipment.country_id,
            segment: equipment.segment.to_string(),
            total: total.map(|value| value.to_string()),
            equipment_total: e_total.map(|value| value.to_string()),
            read_hour: equipment.read_hour,
            observation: equipment.observation,
            values,
        },
        covers,
    })
}

/// The classified variants a request groups: (handle, id, class, seen), in
/// handle order.
type Kinds<'a> = Vec<(u64, u32, &'a Class, u8)>;

fn kinds<'a>(index: &Index, classes: &'a Classes) -> Kinds<'a> {
    classes
        .sorted
        .iter()
        .filter_map(|&variant| {
            let kind = index.finals.get(variant as usize)?.as_ref()?;
            let class = classes.of.get(variant as usize)?.as_ref()?;
            Some((index.variants[variant as usize], variant, class, kind.seen))
        })
        .collect()
}

/// A domain group's place and key.
fn domain_rank(domain: Option<Domain>) -> (usize, &'static str) {
    match domain {
        Some(Domain::Land) => (0, "land"),
        Some(Domain::Air) => (1, "air"),
        Some(Domain::Naval) => (2, "naval"),
        None => (3, equipment::UNRESOLVED_ARM),
    }
}

fn invalid_request(params: &ProducedParams) -> io::Result<()> {
    if params.top.is_some() && !params.tags.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Choose countries or a top count, never both",
        ));
    }
    <crate::products::equipment_produced::EquipmentProduced as crate::products::Product>::validate(
        params,
    )
    .map_err(|reason| io::Error::new(io::ErrorKind::InvalidInput, reason))
}

/// What a page is built from besides its rows.
struct Frame<'a> {
    first_hour: i32,
    last_hour: i32,
    kinds: Kinds<'a>,
    /// Every variant's handle, by id.
    handles: &'a [u64],
    /// A variant's names at the page's hour, by id.
    names: &'a dyn Fn(u32) -> io::Result<Vec<StockVariantName>>,
    warnings: Vec<String>,
}

#[allow(clippy::too_many_arguments)]
fn page(
    frame: Frame<'_>,
    rows: Vec<Row<'_>>,
    params: &ProducedParams,
    offset: usize,
    limit: usize,
    ranked: bool,
    cancelled: &AtomicBool,
) -> io::Result<SelectionPage> {
    let included: BTreeSet<&str> = params.included.iter().map(String::as_str).collect();
    let class_counted =
        |class: &Class| included.is_empty() || included.contains(class.key.as_str());
    let includes_resolved = frame
        .kinds
        .iter()
        .any(|(_, _, class, _)| class.ordinal.is_some() && included.contains(class.key.as_str()));
    let inventory = match params.count {
        ProducedCount::Units => UNITS_SEEN,
        ProducedCount::Ic => IC_SEEN,
    };
    // A variant content cannot type, or not classified at all, is unresolved.
    let classless = if includes_resolved { OUTSIDE } else { 0 }
        | if params.grouping == ProducedGrouping::Type {
            UNSURE
        } else {
            0
        };
    let zero = |handle: u64| if handle == 0 { ZERO } else { 0 };
    let mut flags: Vec<u8> = frame
        .handles
        .iter()
        .map(|&handle| classless | zero(handle))
        .collect();
    let mut ordered: BTreeMap<(usize, &str, u64), Group<'_>> = BTreeMap::new();
    let mut members: Vec<(u32, (usize, &str, u64))> = Vec::new();
    for &(handle, variant, class, seen) in &frame.kinds {
        let counted = class_counted(class);
        let unresolved = class.ordinal.is_none();
        let mut flag = u8::from(counted) * COUNTED;
        if includes_resolved && unresolved && !counted {
            flag |= OUTSIDE;
        }
        if unresolved
            && match params.grouping {
                ProducedGrouping::Type => true,
                ProducedGrouping::Domain => counted,
                ProducedGrouping::Variant => false,
            }
        {
            flag |= UNSURE;
        }
        flags[variant as usize] = flag | zero(handle);
        if !seen_in(seen, inventory, params.refits) || !counted {
            continue;
        }
        let ordinal = class.ordinal.unwrap_or(usize::MAX);
        let order = match params.grouping {
            ProducedGrouping::Type => (ordinal, class.key.as_str(), 0),
            ProducedGrouping::Variant => (ordinal, class.key.as_str(), handle),
            ProducedGrouping::Domain => (domain_rank(class.domain).0, "", 0),
        };
        ordered.entry(order).or_insert_with(|| Group {
            key: match params.grouping {
                ProducedGrouping::Type => class.key.clone(),
                ProducedGrouping::Variant => handle.to_string(),
                ProducedGrouping::Domain => domain_rank(class.domain).1.to_owned(),
            },
            ambiguous: match params.grouping {
                ProducedGrouping::Type => class.ordinal.is_some(),
                ProducedGrouping::Variant => false,
                ProducedGrouping::Domain => class.domain.is_some(),
            },
            first: variant,
            class,
        });
        members.push((variant, order));
    }
    let positions: BTreeMap<(usize, &str, u64), u32> = ordered
        .keys()
        .enumerate()
        .map(|(position, order)| (*order, position as u32))
        .collect();
    let mut group = vec![None; frame.handles.len()];
    for (variant, order) in members {
        group[variant as usize] = Some(positions[&order]);
    }
    let groups: Vec<Group<'_>> = ordered.into_values().collect();
    let plan = Plan {
        flags,
        group,
        refits: params.refits,
        count: params.count,
        measure: params.measure,
    };
    let mut evaluated = Vec::with_capacity(rows.len());
    let (mut has_known, mut has_unknown) = (false, false);
    for row in rows {
        let result = evaluate(&plan, &groups, &row, false, cancelled)?;
        if result.covers {
            has_known |= result.total.is_some();
            has_unknown |= result.total.is_none();
        }
        evaluated.push((result.total, row));
    }
    if ranked || params.top.is_some() {
        evaluated
            .retain(|(value, _)| value.is_some_and(|value| params.top.is_none() || value != 0));
        evaluated.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.tag.cmp(right.1.tag)));
        if let Some(top) = params.top {
            evaluated.truncate(top as usize);
        }
    }
    let total_countries = evaluated.len();
    let type_totals = if params.count == ProducedCount::Units
        && params.measure == ProducedMeasure::Total
        && params.grouping == ProducedGrouping::Type
        && params.included.is_empty()
    {
        let mut sums: BTreeMap<String, i128> = BTreeMap::new();
        for (_, row) in &evaluated {
            for value in evaluate(&plan, &groups, row, true, cancelled)?.country.values {
                if let Some(amount) = value.amount {
                    let amount: i128 = amount
                        .parse()
                        .expect("a value written from an integer");
                    *sums.entry(value.key).or_default() += amount;
                }
            }
        }
        Some(
            groups
                .iter()
                .filter_map(|group| {
                    sums.get(&group.key)
                        .map(|sum| (group.key.clone(), sum.to_string()))
                })
                .collect(),
        )
    } else {
        None
    };
    let value_min = evaluated
        .iter()
        .filter_map(|(value, _)| *value)
        .min()
        .unwrap_or(0)
        .min(0);
    let value_max = evaluated
        .iter()
        .filter_map(|(value, _)| *value)
        .max()
        .unwrap_or(0)
        .max(0);
    let countries = evaluated
        .iter()
        .skip(offset)
        .take(limit)
        .map(|(_, row)| evaluate(&plan, &groups, row, true, cancelled).map(|result| result.country))
        .collect::<io::Result<_>>()?;
    let mut keys = Vec::with_capacity(groups.len());
    for group in &groups {
        let class = group.class;
        keys.push(ProducedKey {
            key: group.key.clone(),
            name: if params.grouping == ProducedGrouping::Type {
                class.name.clone()
            } else {
                None
            },
            type_key: (params.grouping == ProducedGrouping::Variant).then(|| class.key.clone()),
            domain: if params.grouping == ProducedGrouping::Domain {
                match group.key.as_str() {
                    "land" => Some(Domain::Land),
                    "air" => Some(Domain::Air),
                    "naval" => Some(Domain::Naval),
                    _ => None,
                }
            } else {
                class.domain
            },
            names: if params.grouping == ProducedGrouping::Variant {
                (frame.names)(group.first)?
            } else {
                Vec::new()
            },
        });
    }
    let mut types = BTreeMap::new();
    for (_, _, class, seen) in &frame.kinds {
        if seen_in(*seen, inventory, true) {
            types
                .entry((class.ordinal.unwrap_or(usize::MAX), class.key.as_str()))
                .or_insert_with(|| ProducedType {
                    key: class.key.clone(),
                    name: class.name.clone(),
                    domain: class.domain,
                });
        }
    }
    Ok(SelectionPage {
        type_totals,
        observation: None,
        recorded: false,
        schema: 1,
        branch_id: params.branch_id.clone(),
        first_hour: frame.first_hour,
        last_hour: frame.last_hour,
        known: has_known && !has_unknown,
        keys,
        types: types.into_values().collect(),
        total_countries,
        offset: offset.to_string(),
        value_min: value_min.to_string(),
        value_max: value_max.to_string(),
        countries,
        warnings: frame.warnings,
    })
}

/// An unobserved endpoint supplies no country amounts, ranks or classifications.
pub fn empty_selection(
    hour: i32,
    params: &ProducedParams,
    offset: usize,
    limit: usize,
    ranked: bool,
    cancelled: &AtomicBool,
) -> io::Result<SelectionPage> {
    check_cancel(cancelled)?;
    invalid_request(params)?;
    page(
        Frame {
            first_hour: hour,
            last_hour: hour,
            kinds: Vec::new(),
            handles: &[],
            names: &|_| Ok(Vec::new()),
            warnings: Vec::new(),
        },
        Vec::new(),
        params,
        offset,
        limit,
        ranked,
        cancelled,
    )
}

/// Endpoint states one request reads, each decoded once.
struct States<'a> {
    index: &'a Index,
    store: &'a store::Store,
    key: &'a store::SourceKey,
    cancelled: &'a AtomicBool,
    held: HashMap<u64, Arc<State>>,
}

impl States<'_> {
    fn at(&mut self, ordinal: u64) -> io::Result<Arc<State>> {
        if let Some(state) = self.held.get(&ordinal) {
            return Ok(Arc::clone(state));
        }
        let state = self
            .index
            .state(self.store, self.key, ordinal, self.cancelled)?;
        self.held.insert(ordinal, Arc::clone(&state));
        Ok(state)
    }
}

/// The page at `hour` of the countries `params` chooses, ranked when asked
/// (`ranked`) or for the top N; None where no endpoint holds the hour.
#[allow(clippy::too_many_arguments)]
pub fn select_at(
    source: &Source,
    content: Option<&crate::products::ContentOptions>,
    store: &store::Store,
    key: &store::SourceKey,
    hour: i32,
    params: &ProducedParams,
    offset: usize,
    limit: usize,
    ranked: bool,
    cancelled: &AtomicBool,
) -> io::Result<Option<SelectionPage>> {
    let branch = params
        .branch_id
        .parse::<u64>()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "Invalid branch"))?;
    check_cancel(cancelled)?;
    let Some((index, classes)) =
        super::read::index(source, content, store, key, branch, cancelled)?
    else {
        return Ok(None);
    };
    select_with(&index, &classes, store, key, hour, params, offset, limit, ranked, cancelled)
}

/// [`select_at`] over an index and its classes.
#[allow(clippy::too_many_arguments)]
pub(crate) fn select_with(
    index: &Index,
    classes: &Classes,
    store: &store::Store,
    key: &store::SourceKey,
    hour: i32,
    params: &ProducedParams,
    offset: usize,
    limit: usize,
    ranked: bool,
    cancelled: &AtomicBool,
) -> io::Result<Option<SelectionPage>> {
    check_cancel(cancelled)?;
    if params.tags.is_empty() && params.top.is_none() {
        invalid_request(params)?;
        // Keys are not sent without countries; their names are not read.
        let mut selection = page(
            Frame {
                first_hour: hour,
                last_hour: hour,
                kinds: kinds(index, classes),
                handles: &index.variants,
                names: &|_| Ok(Vec::new()),
                warnings: index.warnings(),
            },
            Vec::new(),
            params,
            offset,
            limit,
            ranked,
            cancelled,
        )?;
        selection.keys.clear();
        selection.observation = index.observation_at(hour);
        selection.recorded = !index.diagnostics.spending.reads.is_empty();
        return Ok(Some(selection));
    }
    let Some(current) = index.endpoint_at(hour) else {
        return Ok(None);
    };
    invalid_request(params)?;
    let mut states = States {
        index,
        store,
        key,
        cancelled,
        held: HashMap::new(),
    };
    let endpoint = *index.endpoint(current);
    let state = states.at(current)?;
    // The rows chosen, in tag order for the top N or as requested.
    let chosen: Vec<u32> = if params.top.is_some() {
        index.tags.values().copied().collect()
    } else {
        let mut seen = HashSet::new();
        params
            .tags
            .iter()
            .filter(|tag| seen.insert(tag.as_str()))
            .filter_map(|tag| index.tags.get(tag).copied())
            .collect()
    };
    // Each predecessor page, and the pages their supplying reads join Rocket from.
    let before = |read: Option<i32>| {
        read.and_then(|read| predecessor(read, params.measure))
            .and_then(|hour| index.endpoint_at(hour))
    };
    let equipment_before = before(endpoint.equipment_read);
    let rocket_before = before(endpoint.rocket_read);
    let mut loaded: Vec<(u64, Arc<State>)> = vec![(current, state)];
    for ordinal in [equipment_before, rocket_before].into_iter().flatten() {
        loaded.push((ordinal, states.at(ordinal)?));
    }
    let mut supplying = BTreeSet::new();
    for &holder in &chosen {
        for (ordinal, held) in &loaded {
            if (*ordinal == current || Some(*ordinal) == rocket_before)
                && let Some(at) = joined(index, *ordinal, held, holder)?
            {
                supplying.insert(at);
            }
        }
    }
    for ordinal in supplying {
        if !loaded.iter().any(|(held, _)| *held == ordinal) {
            loaded.push((ordinal, states.at(ordinal)?));
        }
    }
    let mut rows = Vec::new();
    for holder in chosen {
        let Some(equipment) =
            source_at(&endpoint, state_of(&loaded, current).holder(holder), false)?
        else {
            continue;
        };
        rows.push(Row {
            tag: &index.holders[holder as usize],
            equipment,
            equipment_before: match equipment_before {
                Some(ordinal) => source_at(
                    index.endpoint(ordinal),
                    state_of(&loaded, ordinal).holder(holder),
                    false,
                )?,
                None => None,
            },
            rocket: rocket_of(index, &loaded, current, holder)?,
            rocket_before: match rocket_before {
                Some(ordinal) => rocket_of(index, &loaded, ordinal, holder)?,
                None => None,
            },
        });
    }
    let names = |variant: u32| -> io::Result<Vec<StockVariantName>> {
        // Type identity is the publication's; recorded naming stays at this endpoint's read.
        match index.names_at(classes, variant, current)? {
            Some(names) => Ok(names),
            None => index.final_names(classes, variant),
        }
    };
    let mut selection = page(
        Frame {
            first_hour: endpoint.first_hour,
            last_hour: endpoint.last_hour,
            kinds: kinds(index, classes),
            handles: &index.variants,
            names: &names,
            warnings: index.warnings(),
        },
        rows,
        params,
        offset,
        limit,
        ranked,
        cancelled,
    )?;
    selection.observation = index.observation_at(hour);
    selection.recorded = !index.diagnostics.spending.reads.is_empty();
    Ok(Some(selection))
}

fn state_of(loaded: &[(u64, Arc<State>)], ordinal: u64) -> &State {
    &loaded
        .iter()
        .find(|(held, _)| *held == ordinal)
        .expect("loaded endpoint")
        .1
}

/// The endpoint supplying the Rocket source joined to a holder's equipment
/// read at endpoint `ordinal`; None where it is not joined, or was withdrawn.
fn joined(index: &Index, ordinal: u64, state: &State, holder: u32) -> io::Result<Option<u64>> {
    let row = index.endpoint(ordinal);
    let Some(source) = state.holder(holder) else {
        return Ok(None);
    };
    let Some(equipment) = &source.equipment else {
        return Ok(None);
    };
    let read = row.read_of(&equipment.head, false)?;
    if !source.rocket_joined || index.revoked(holder, &equipment.head, read, row.first_hour) {
        return Ok(None);
    }
    Ok(index.endpoint_at(read))
}

/// The Rocket source joined to a holder's equipment read at endpoint `ordinal`.
fn rocket_of<'a>(
    index: &Index,
    loaded: &'a [(u64, Arc<State>)],
    ordinal: u64,
    holder: u32,
) -> io::Result<Option<SourceAt<'a>>> {
    match joined(index, ordinal, state_of(loaded, ordinal), holder)? {
        Some(supplying) => source_at(
            index.endpoint(supplying),
            state_of(loaded, supplying).holder(holder),
            true,
        ),
        None => Ok(None),
    }
}
