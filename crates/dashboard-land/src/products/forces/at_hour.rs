use super::{
    AirForceMeasure, AirForceOwnership, Content, ForcePart, Named, NavyGrouping, NavyMeasure, NavyOwnership, UNRESOLVED, equipment, split, unidentified, validate_selection, variant_part,
};
use content_reader_script::sub_units::{SubUnit, SubUnits};
use dashboard_base::{
    Source,
    catalogue::{self, Catalogue},
    products::{
        Country, LoadContext,
        at_hour::{At, AtHour, Owner, Read},
    },
};
use dashboard_query::{
    coverage::{Witness, population, witnessed},
    identity::{Holder, Holders},
    series::ranked::GroupSelection,
};
use dashboard_world::products::group_members::GroupsAt;
use replay_data::rows::{AirForcePartRow, AirForceWingKindRow, NavyClassRow, Recorded, TableId, tables};
use replay_frame::frame::{DecodedFrame, Keys, TableView};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::Arc,
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub struct AirForceAt;
pub struct NavyAt;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AirForceAtParams {
    pub branch_id: String,
    #[serde(default)]
    pub countries: Vec<Country>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub top: Option<u32>,
    #[doc = " The group whose members at the hour are the rows; never with countries or a top."]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub group: Option<GroupSelection>,
    pub measure: AirForceMeasure,
    pub ownership: AirForceOwnership,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NavyAtParams {
    pub branch_id: String,
    #[serde(default)]
    pub countries: Vec<Country>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub top: Option<u32>,
    #[doc = " The group whose members at the hour are the rows; never with countries or a top."]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub group: Option<GroupSelection>,
    pub ownership: NavyOwnership,
    pub grouping: NavyGrouping,
    pub measure: NavyMeasure,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ForceAtRow {
    pub country: Country,
    #[doc = " The recorded slot holding the country at the hour; none where its holder is not resolved."]
    pub slot: Option<u32>,
    pub tag: String,
    #[cfg_attr(any(test, feature = "export-types"), ts(type = "number | null"))]
    pub total: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional, type = "number | null"))]
    pub reference_total: Option<Option<i64>>,
    pub missing: Option<String>,
    #[cfg_attr(any(test, feature = "export-types"), ts(type = "Array<number | null>"))]
    pub values: Vec<Option<i64>>,
    pub parts_missing: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ForceAtReply {
    pub parts: Vec<ForcePart>,
    pub rows: Vec<ForceAtRow>,
    pub missing: Option<String>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(ForceAtReply);

pub struct ForceContext {
    pub(crate) catalogue: Option<Arc<Catalogue>>,
    pub(crate) warnings: Vec<String>,
    pub(crate) groups: GroupsAt,
}

pub(crate) fn load_context(source: &Source, load: &LoadContext, what: &str) -> ForceContext {
    let mut warnings = Vec::new();
    let content = Content::read(source, load, what, &mut warnings);
    ForceContext { catalogue: content.catalogue, warnings, groups: GroupsAt::default() }
}

pub(crate) fn load_context_for(source: &Source, load: &LoadContext, what: &str, branch_id: &str, group: Option<&GroupSelection>) -> io::Result<ForceContext> {
    Ok(ForceContext { groups: GroupsAt::load(source, load, branch_id, group)?, ..load_context(source, load, what) })
}

pub(crate) enum Chosen {
    Population,
    These(Vec<Country>),
    Refused(String),
}

pub(crate) fn chosen(countries: &[Country], group: Option<&GroupSelection>, groups: &GroupsAt, hour: i32) -> io::Result<Chosen> {
    let Some(group) = group else {
        return Ok(if countries.is_empty() { Chosen::Population } else { Chosen::These(countries.to_vec()) });
    };
    let members = groups.members(group, hour)?;
    Ok(match members.refused {
        Some(reason) => Chosen::Refused(reason),
        None => Chosen::These(members.tags.into_iter().map(Country::from).collect()),
    })
}

pub(crate) fn validate_group(group: Option<&GroupSelection>, countries: &[Country], top: Option<u32>) -> Result<(), &'static str> {
    if group.is_some() && (top.is_some() || !countries.is_empty()) {
        return Err("A selection names a group, a top or countries, one of them");
    }
    Ok(())
}

fn holders_unresolved(holders: &Holders, hour: i32) -> bool {
    holders.unknown.iter().any(|&(first, last, _)| first <= hour && hour <= last)
        || holders.tags.values().flatten().any(|run| run.first <= hour && hour <= run.last && run.holder == Holder::Unresolved)
}

fn slot_tags(holders: &Holders, hour: i32) -> HashMap<u32, &str> {
    let mut out = HashMap::new();
    for (tag, runs) in &holders.tags {
        if let Some(Holder::Found { slot, .. }) = runs.iter().find(|run| run.first <= hour && hour <= run.last).map(|run| run.holder) {
            out.entry(slot).or_insert(tag.as_str());
        }
    }
    out
}

pub(crate) fn recorded_value(recorded: Recorded<i64>) -> Option<i64> {
    match recorded {
        Recorded::Value(value) => Some(value),
        Recorded::NotRecorded => None,
    }
}

pub(crate) fn frame_rows<'a, T: replay_data::rows::TableSchema>(frame: &'a DecodedFrame) -> Option<&'a [T::Row]> {
    match &*frame.tables[T::ID.index()] {
        TableView::Complete(rows) | TableView::Truncated { rows, .. } => rows.rows::<T>(),
        TableView::Unavailable { .. } | TableView::NotRecorded { .. } => None,
    }
}

pub(crate) fn frame_literals(frame: &DecodedFrame) -> HashMap<u32, String> {
    frame_rows::<tables::LexicalTokens>(frame)
        .into_iter()
        .flatten()
        .filter_map(|row| Some((row.token, std::str::from_utf8(&row.literal).ok()?.to_owned())))
        .collect()
}

fn frame_variant_tokens(frame: &DecodedFrame) -> HashMap<u64, String> {
    let literals = frame_literals(frame);
    frame_rows::<tables::VariantDefinitions>(frame)
        .into_iter()
        .flatten()
        .filter_map(|row| Some((row.id, literals.get(&row.equipment_token)?.clone())))
        .collect()
}

pub(crate) struct SelectedRow {
    pub(crate) slot: Option<u32>,
    pub(crate) country: Country,
    pub(crate) tag: String,
    pub(crate) total: Option<i64>,
    pub(crate) reference_total: Option<Option<i64>>,
    pub(crate) missing: Option<String>,
}

pub(crate) fn select_population<R>(
    rows: &[R],
    slot_of: impl Fn(&R) -> u32,
    total_of: impl Fn(&R) -> Option<i64>,
    reference_of: impl Fn(&R) -> Option<Option<i64>>,
    holders: &Holders,
    hour: i32,
    top: Option<u32>,
    label: &str,
) -> Result<Vec<SelectedRow>, String> {
    let tags = slot_tags(holders, hour);
    let unresolved = holders_unresolved(holders, hour);
    let mut held: Vec<SelectedRow> = Vec::new();
    let mut unplaced: Vec<Option<i64>> = Vec::new();
    let mut unread = 0usize;
    for row in rows {
        let slot = slot_of(row);
        let total = total_of(row);
        let reference_total = reference_of(row);
        match tags.get(&slot) {
            Some(&tag) => {
                if total.is_none() {
                    unread += 1;
                }
                held.push(SelectedRow {
                    slot: Some(slot),
                    country: Country::from(tag),
                    tag: tag.to_owned(),
                    total,
                    reference_total,
                    missing: total.is_none().then(|| format!("{label} was not recorded at this hour")),
                });
            }
            None => unplaced.push(total),
        }
    }
    if let Some(limit) = top {
        if unread > 0 {
            return Err(format!("The top {limit} by {label} needs every country's reading, and {unread} of {} were not recorded at this hour", held.len()));
        }
        held.sort_unstable_by(|a, b| b.total.cmp(&a.total).then_with(|| a.tag.cmp(&b.tag)));
        let count = usize::try_from(limit).unwrap_or(usize::MAX);
        held.truncate(count);
        if unresolved && !unplaced.is_empty() {
            let full = held.len() >= count;
            let floor = held.last().and_then(|row| row.total);
            let contending = unplaced
                .iter()
                .filter(|total| match total {
                    Some(value) => !full || floor.is_none_or(|floor| *value >= floor),
                    None => true,
                })
                .count();
            if contending > 0 {
                return Err(format!("The top {limit} by {label} is not known: {contending} countries could not be matched to a tag at this hour and could be among them"));
            }
        }
    } else {
        held.sort_unstable_by(|a, b| match (a.total, b.total) {
            (Some(av), Some(bv)) => bv.cmp(&av).then_with(|| a.tag.cmp(&b.tag)),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => a.tag.cmp(&b.tag),
        });
    }
    Ok(held)
}

pub(crate) fn select_explicit<T: replay_data::rows::TableSchema>(
    frame: &DecodedFrame,
    countries: &[Country],
    total_of: impl Fn(&T::Row) -> Option<i64>,
    reference_of: impl Fn(&T::Row) -> Option<Option<i64>>,
    absent_reference: Option<Option<i64>>,
    holders: &Holders,
    hour: i32,
    label: &str,
) -> Vec<SelectedRow> {
    let mut seen = BTreeSet::new();
    let mut held_slots = BTreeSet::new();
    let mut out = Vec::new();
    for country in countries {
        if !seen.insert(country.clone()) {
            continue;
        }
        let Some(run) = holders.at(country, hour) else { continue };
        if let Holder::Found { slot, .. } = run.holder
            && !held_slots.insert(slot)
        {
            continue;
        }
        let tag = holders.name(country, hour);
        match run.holder {
            Holder::Found { slot, .. } => match witnessed::<T>(frame, u128::from(slot)) {
                Witness::Row(row) => {
                    let total = total_of(row);
                    out.push(SelectedRow {
                        slot: Some(slot),
                        country: country.clone(),
                        tag,
                        total,
                        reference_total: reference_of(row),
                        missing: total.is_none().then(|| format!("{label} was not recorded at this hour")),
                    });
                }
                Witness::Absent => out.push(SelectedRow {
                    slot: Some(slot),
                    country: country.clone(),
                    tag,
                    total: Some(0),
                    reference_total: absent_reference,
                    missing: None,
                }),
                Witness::NotRecorded(reason) => out.push(SelectedRow {
                    slot: Some(slot),
                    country: country.clone(),
                    tag,
                    total: None,
                    reference_total: absent_reference.map(|_| None),
                    missing: Some(reason),
                }),
            },
            Holder::Unresolved => out.push(SelectedRow {
                slot: None,
                country: country.clone(),
                tag,
                total: None,
                reference_total: absent_reference.map(|_| None),
                missing: Some("Country holder could not be resolved at this hour".into()),
            }),
        }
    }
    out.sort_unstable_by(|a, b| match (a.total, b.total) {
        (Some(av), Some(bv)) => bv.cmp(&av).then_with(|| a.tag.cmp(&b.tag)),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.tag.cmp(&b.tag),
    });
    out
}

fn air_plane_part_id(variant: Option<u64>) -> u128 {
    match variant {
        Some(variant) => (1u128 << 64) | u128::from(variant),
        None => 0,
    }
}

fn name_air_planes(frame: &DecodedFrame, parts: &BTreeSet<u128>, content: &Content, warnings: &mut Vec<String>) -> Named {
    let tokens = frame_variant_tokens(frame);
    let units = equipment(content, warnings);
    let mut notes = BTreeSet::new();
    let resolved = parts
        .iter()
        .map(|&part| {
            let named = if (part >> 64) & 1 == 1 {
                let variant = part as u64;
                variant_part(variant, tokens.get(&variant).map(String::as_str), units.as_deref(), content, &mut notes)
            } else {
                unidentified()
            };
            (part, named)
        })
        .collect();
    warnings.extend(notes);
    Named::group(resolved)
}

fn name_air_wings(frame: &DecodedFrame, parts: &BTreeSet<u128>, content: &Content, warnings: &mut Vec<String>) -> Named {
    let mut signatures: HashMap<u32, Vec<u64>> = HashMap::new();
    for row in frame_rows::<tables::AirForceWingSignatures>(frame).into_iter().flatten() {
        match row.variants() {
            Ok(variants) => {
                signatures.insert(row.number, variants);
            }
            Err(error) => warnings.push(format!("Wing composition {}: {error}", row.number)),
        }
    }
    let tokens = frame_variant_tokens(frame);
    let units = equipment(content, warnings);
    let mut notes = BTreeSet::new();
    let mut resolved = BTreeMap::new();
    for &part in parts {
        let key = part as u32;
        let named = match signatures.get(&key) {
            None => ForcePart { key: format!("{UNRESOLVED}composition/{key}"), name: Some(format!("Wing composition {key}")), unresolved: true },
            Some(variants) if variants.is_empty() => unidentified(),
            Some(variants) => {
                let mut kinds: BTreeMap<String, ForcePart> = BTreeMap::new();
                for &variant in variants {
                    let named = variant_part(variant, tokens.get(&variant).map(String::as_str), units.as_deref(), content, &mut notes);
                    kinds.insert(named.key.clone(), named);
                }
                if kinds.len() == 1 {
                    kinds.into_values().next().expect("one kind")
                } else {
                    let keys: Vec<&String> = kinds.keys().collect();
                    ForcePart {
                        key: format!("mixed:{}", serde_json::to_string(&keys).expect("string keys")),
                        name: Some(format!(
                            "Mixed types: {}",
                            kinds.values().map(|kind| kind.name.as_deref().unwrap_or("Equipment type name unavailable")).collect::<Vec<_>>().join(" / ")
                        )),
                        unresolved: kinds.values().any(|kind| kind.unresolved),
                    }
                }
            }
        };
        resolved.insert(part, named);
    }
    warnings.extend(notes);
    Named::group(resolved)
}

fn name_navy_classes(frame: &DecodedFrame, parts: &BTreeSet<u128>, content: &Content, grouping: NavyGrouping, warnings: &mut Vec<String>) -> Named {
    let spelled = frame_literals(frame);
    let units: Option<Arc<SubUnits>> = content.catalogue.as_ref().and_then(|catalogue| {
        match catalogue
            .content
            .layers
            .derived::<catalogue::CachedResult<SubUnits>>()
            .get_or_init(|| content_reader_key_names::unit_leader::sub_units(&catalogue.content.layers))
        {
            Ok(units) => {
                warnings.extend(units.warnings().iter().cloned());
                Some(units)
            }
            Err(error) => {
                warnings.push(format!("Ship class content is unavailable: {error}"));
                None
            }
        }
    });
    let mut resolved = BTreeMap::new();
    for &part in parts {
        let token = part as u32;
        let spelling = spelled.get(&token);
        let mut refusal = None;
        let found = spelling.zip(units.as_deref()).and_then(|(spelling, units)| {
            let SubUnit::Index(index) = units.get(spelling) else { return None };
            let registered = units.name(index)?.ok()?;
            match grouping {
                NavyGrouping::Class => Some((format!("class:{registered}"), content.localise(registered))),
                NavyGrouping::Type => {
                    let mask = units.category(registered.as_bytes()).ok()?;
                    let names = match content_reader_script::equipment_stats::category_names(mask) {
                        Ok(names) if !names.is_empty() => names,
                        Ok(_) => return None,
                        Err(bits) => {
                            refusal = Some(format!("unknown native category bits {bits:#x}"));
                            return None;
                        }
                    };
                    let name = names.iter().map(|key| content.localise(key)).collect::<Option<Vec<_>>>().map(|names| names.join(" / "));
                    Some((format!("type:{mask}"), name))
                }
            }
        });
        let named = match found {
            Some((key, name)) => ForcePart { key, name, unresolved: false },
            None => {
                let visible = spelling.filter(|name| !name.is_empty()).cloned().unwrap_or_else(|| format!("Other / not identified (Sub-unit {token})"));
                let reason = refusal.as_deref().unwrap_or("the recorded content does not resolve this ship class or broad role");
                warnings.push(format!("{visible}: {reason}; kept as its own part."));
                ForcePart {
                    key: format!("{UNRESOLVED}ship:{}", serde_json::to_string(&(token, spelling)).expect("class identity")),
                    name: Some("Ship class name unavailable".into()),
                    unresolved: true,
                }
            }
        };
        resolved.insert(part, named);
    }
    Named::group(resolved)
}

pub(crate) fn attach_parts(selected: Vec<SelectedRow>, named: Named, by_slot: &HashMap<u32, Vec<(u128, Option<i64>)>>, parts_error: Option<String>) -> (Vec<ForcePart>, Vec<ForceAtRow>, Vec<String>) {
    let groups = named.parts.len();
    let mut warnings = Vec::new();
    let unresolved = named.parts.iter().filter(|part| part.unresolved).count();
    if unresolved > 0 {
        warnings.push(format!("{unresolved} parts could not be resolved through the recorded content; each remains visible."));
    }
    let rows = selected
        .into_iter()
        .map(|row| {
            if let Some(reason) = &parts_error {
                return ForceAtRow {
                    country: row.country,
                    slot: row.slot,
                    tag: row.tag,
                    total: row.total,
                    reference_total: row.reference_total,
                    missing: row.missing,
                    values: Vec::new(),
                    parts_missing: row.total.map(|_| reason.clone()),
                };
            }
            let held: Vec<(usize, Option<i64>)> = row
                .slot
                .and_then(|slot| by_slot.get(&slot))
                .into_iter()
                .flatten()
                .filter_map(|&(part, value)| Some((*named.of.get(&part)?, value)))
                .collect();
            let values = split(row.total, &held, groups);
            let parts_missing = if row.total.is_some() && values.iter().any(Option::is_none) {
                Some("Part breakdown does not reconcile with the recorded total at this hour".into())
            } else {
                None
            };
            ForceAtRow {
                country: row.country,
                slot: row.slot,
                tag: row.tag,
                total: row.total,
                reference_total: row.reference_total,
                missing: row.missing,
                values,
                parts_missing,
            }
        })
        .collect();
    (named.parts, rows, warnings)
}

impl AtHour for AirForceAt {
    const KEY: &'static str = "air-force-at";
    const TITLE: &'static str = "Air force size";
    const CONTENT_DEPENDENT: bool = true;
    const CLOCK: dashboard_base::products::at_hour::Clock = dashboard_base::products::at_hour::Clock::Boundaries;
    type Params = AirForceAtParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, countries, top, group, measure, ownership, "countries[]"];
    type Reply = ForceAtReply;
    type Context = ForceContext;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![AirForceAtParams::decl(), ForceAtRow::decl(), ForceAtReply::decl(), AirForceMeasure::decl(), AirForceOwnership::decl(), ForcePart::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        validate_selection(&params.branch_id, &params.countries, params.top.map(|n| n as usize))?;
        validate_group(params.group.as_ref(), &params.countries, params.top)
    }

    fn tables(params: &Self::Params) -> Vec<TableId> {
        match params.measure {
            AirForceMeasure::Planes => vec![TableId::AirForceTotals, TableId::AirForceParts, TableId::VariantDefinitions, TableId::LexicalTokens],
            AirForceMeasure::Wings => vec![TableId::AirForceTotals, TableId::AirForceWingKinds, TableId::AirForceWingSignatures, TableId::VariantDefinitions, TableId::LexicalTokens],
        }
    }

    fn reads(params: &Self::Params, _owner: Option<Owner>, step: usize, held: &DecodedFrame) -> Vec<Read> {
        match (params.measure, step) {
            (AirForceMeasure::Planes, 0) => vec![Read::all(TableId::AirForceTotals), Read::all(TableId::AirForceParts)],
            (AirForceMeasure::Wings, 0) => vec![Read::all(TableId::AirForceTotals), Read::all(TableId::AirForceWingKinds), Read::all(TableId::AirForceWingSignatures)],
            (AirForceMeasure::Planes, 1) => {
                let owner_code = if params.ownership == AirForceOwnership::Effective { 1 } else { 0 };
                let variants: Vec<u128> = frame_rows::<tables::AirForceParts>(held)
                    .into_iter()
                    .flatten()
                    .filter(|row| row.ownership == owner_code)
                    .filter_map(|row| row.variant.map(u128::from))
                    .collect();
                if variants.is_empty() { Vec::new() } else { vec![Read::these(TableId::VariantDefinitions, variants)] }
            }
            (AirForceMeasure::Wings, 1) => {
                let owner_code = if params.ownership == AirForceOwnership::Effective { 1 } else { 0 };
                let used: BTreeSet<u32> = frame_rows::<tables::AirForceWingKinds>(held)
                    .into_iter()
                    .flatten()
                    .filter(|row| row.ownership == owner_code)
                    .map(|row| row.signature)
                    .collect();
                let variants: Vec<u128> = frame_rows::<tables::AirForceWingSignatures>(held)
                    .into_iter()
                    .flatten()
                    .filter(|row| used.contains(&row.number))
                    .filter_map(|row| row.variants().ok())
                    .flatten()
                    .map(u128::from)
                    .collect();
                if variants.is_empty() { Vec::new() } else { vec![Read::these(TableId::VariantDefinitions, variants)] }
            }
            (_, 2) => {
                let tokens: Vec<u128> = frame_rows::<tables::VariantDefinitions>(held).into_iter().flatten().map(|row| u128::from(row.equipment_token)).collect();
                if tokens.is_empty() { Vec::new() } else { vec![Read::these(TableId::LexicalTokens, tokens)] }
            }
            _ => Vec::new(),
        }
    }

    fn context(source: &Source, load: &LoadContext) -> io::Result<Self::Context> {
        Ok(load_context(source, load, "Air equipment type names"))
    }

    fn context_for(source: &Source, load: &LoadContext, params: &Self::Params) -> io::Result<Self::Context> {
        load_context_for(source, load, "Air equipment type names", &params.branch_id, params.group.as_ref())
    }

    fn valid_through(_frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> i32 {
        at.context.groups.valid_through(params.group.as_ref(), at.hour)
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let mut warnings = at.context.warnings.clone();
        let content = Content { catalogue: at.context.catalogue.clone() };
        let total_of = |row: &replay_data::rows::AirForceTotalRow| match (params.measure, params.ownership) {
            (AirForceMeasure::Planes, AirForceOwnership::Roster) => recorded_value(row.roster_planes),
            (AirForceMeasure::Wings, AirForceOwnership::Roster) => recorded_value(row.roster_wings),
            (AirForceMeasure::Planes, AirForceOwnership::Effective) => recorded_value(row.effective_planes),
            (AirForceMeasure::Wings, AirForceOwnership::Effective) => recorded_value(row.effective_wings),
        };
        let label = match params.measure {
            AirForceMeasure::Planes => "air planes",
            AirForceMeasure::Wings => "air wings",
        };
        let selected = match chosen(&params.countries, params.group.as_ref(), &at.context.groups, frame.hour)? {
            Chosen::Refused(reason) => return Ok(ForceAtReply { parts: Vec::new(), rows: Vec::new(), missing: Some(reason), warnings }),
            Chosen::Population => {
                let totals = match population::<tables::AirForceTotals>(frame, &Keys::All) {
                    Ok(rows) => rows,
                    Err(reason) => return Ok(ForceAtReply { parts: Vec::new(), rows: Vec::new(), missing: Some(reason), warnings }),
                };
                match select_population(totals, |row| row.slot, total_of, |_| None, at.holders, frame.hour, params.top, label) {
                    Ok(rows) => rows,
                    Err(reason) => return Ok(ForceAtReply { parts: Vec::new(), rows: Vec::new(), missing: Some(reason), warnings }),
                }
            }
            Chosen::These(countries) => select_explicit::<tables::AirForceTotals>(frame, &countries, total_of, |_| None, None, at.holders, frame.hour, label),
        };
        let selected_slots: BTreeSet<u32> = selected.iter().filter_map(|row| row.slot).collect();
        let owner_code = if params.ownership == AirForceOwnership::Effective { 1 } else { 0 };
        let mut met = BTreeSet::new();
        let mut by_slot: HashMap<u32, Vec<(u128, Option<i64>)>> = HashMap::new();
        let parts_error = match params.measure {
            AirForceMeasure::Planes => match population::<tables::AirForceParts>(frame, &Keys::All) {
                Ok(rows) => {
                    for row in rows {
                        let row: &AirForcePartRow = row;
                        if row.ownership != owner_code || !selected_slots.contains(&row.slot) {
                            continue;
                        }
                        let part = air_plane_part_id(row.variant);
                        met.insert(part);
                        by_slot.entry(row.slot).or_default().push((part, recorded_value(row.planes)));
                    }
                    None
                }
                Err(reason) => Some(reason),
            },
            AirForceMeasure::Wings => match population::<tables::AirForceWingKinds>(frame, &Keys::All) {
                Ok(rows) => {
                    for row in rows {
                        let row: &AirForceWingKindRow = row;
                        if row.ownership != owner_code || !selected_slots.contains(&row.slot) {
                            continue;
                        }
                        let part = u128::from(row.signature);
                        met.insert(part);
                        by_slot.entry(row.slot).or_default().push((part, Some(row.wings)));
                    }
                    None
                }
                Err(reason) => Some(reason),
            },
        };
        let named = if parts_error.is_some() {
            Named::default()
        } else {
            match params.measure {
                AirForceMeasure::Planes => name_air_planes(frame, &met, &content, &mut warnings),
                AirForceMeasure::Wings => name_air_wings(frame, &met, &content, &mut warnings),
            }
        };
        let (parts, rows, extra_warnings) = attach_parts(selected, named, &by_slot, parts_error);
        warnings.extend(extra_warnings);
        Ok(ForceAtReply { parts, rows, missing: None, warnings })
    }
}

impl AtHour for NavyAt {
    const KEY: &'static str = "navy-at";
    const TITLE: &'static str = "Navy size";
    const CONTENT_DEPENDENT: bool = true;
    const CLOCK: dashboard_base::products::at_hour::Clock = dashboard_base::products::at_hour::Clock::Boundaries;
    type Params = NavyAtParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, countries, top, group, ownership, grouping, measure, "countries[]"];
    type Reply = ForceAtReply;
    type Context = ForceContext;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![NavyAtParams::decl(), ForceAtRow::decl(), ForceAtReply::decl(), NavyMeasure::decl(), NavyOwnership::decl(), NavyGrouping::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        validate_selection(&params.branch_id, &params.countries, params.top.map(|n| n as usize))?;
        validate_group(params.group.as_ref(), &params.countries, params.top)
    }

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        vec![TableId::NavyTotals, TableId::NavyClasses, TableId::LexicalTokens]
    }

    fn reads(params: &Self::Params, _owner: Option<Owner>, step: usize, held: &DecodedFrame) -> Vec<Read> {
        match step {
            0 => vec![Read::all(TableId::NavyTotals), Read::all(TableId::NavyClasses)],
            1 => {
                let owner_code = if params.ownership == NavyOwnership::Logical { 1 } else { 0 };
                let tokens: Vec<u128> = frame_rows::<tables::NavyClasses>(held)
                    .into_iter()
                    .flatten()
                    .filter(|row| row.ownership == owner_code)
                    .map(|row| u128::from(row.class))
                    .collect();
                if tokens.is_empty() { Vec::new() } else { vec![Read::these(TableId::LexicalTokens, tokens)] }
            }
            _ => Vec::new(),
        }
    }

    fn context(source: &Source, load: &LoadContext) -> io::Result<Self::Context> {
        Ok(load_context(source, load, "Ship class names"))
    }

    fn context_for(source: &Source, load: &LoadContext, params: &Self::Params) -> io::Result<Self::Context> {
        load_context_for(source, load, "Ship class names", &params.branch_id, params.group.as_ref())
    }

    fn valid_through(_frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> i32 {
        at.context.groups.valid_through(params.group.as_ref(), at.hour)
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let mut warnings = at.context.warnings.clone();
        let content = Content { catalogue: at.context.catalogue.clone() };
        let total_of = |row: &replay_data::rows::NavyTotalRow| match (params.measure, params.ownership) {
            (NavyMeasure::Ships, NavyOwnership::Roster) => recorded_value(row.roster_ships),
            (NavyMeasure::Strength, NavyOwnership::Roster) => recorded_value(row.roster_strength_raw),
            (NavyMeasure::Ships, NavyOwnership::Logical) => recorded_value(row.logical_ships),
            (NavyMeasure::Strength, NavyOwnership::Logical) => recorded_value(row.logical_strength_raw),
        };
        let reference_of = |row: &replay_data::rows::NavyTotalRow| match (params.measure, params.ownership) {
            (NavyMeasure::Strength, NavyOwnership::Roster) => Some(recorded_value(row.roster_max_strength_raw)),
            (NavyMeasure::Strength, NavyOwnership::Logical) => Some(recorded_value(row.logical_max_strength_raw)),
            (NavyMeasure::Ships, _) => Some(None),
        };
        let absent_reference = if params.measure == NavyMeasure::Strength { Some(Some(0)) } else { Some(None) };
        let label = match params.measure {
            NavyMeasure::Ships => "ships",
            NavyMeasure::Strength => "hull strength",
        };
        let selected = match chosen(&params.countries, params.group.as_ref(), &at.context.groups, frame.hour)? {
            Chosen::Refused(reason) => return Ok(ForceAtReply { parts: Vec::new(), rows: Vec::new(), missing: Some(reason), warnings }),
            Chosen::Population => {
                let totals = match population::<tables::NavyTotals>(frame, &Keys::All) {
                    Ok(rows) => rows,
                    Err(reason) => return Ok(ForceAtReply { parts: Vec::new(), rows: Vec::new(), missing: Some(reason), warnings }),
                };
                match select_population(totals, |row| row.slot, total_of, reference_of, at.holders, frame.hour, params.top, label) {
                    Ok(rows) => rows,
                    Err(reason) => return Ok(ForceAtReply { parts: Vec::new(), rows: Vec::new(), missing: Some(reason), warnings }),
                }
            }
            Chosen::These(countries) => select_explicit::<tables::NavyTotals>(frame, &countries, total_of, reference_of, absent_reference, at.holders, frame.hour, label),
        };
        let selected_slots: BTreeSet<u32> = selected.iter().filter_map(|row| row.slot).collect();
        let owner_code = if params.ownership == NavyOwnership::Logical { 1 } else { 0 };
        let mut met = BTreeSet::new();
        let mut by_slot: HashMap<u32, Vec<(u128, Option<i64>)>> = HashMap::new();
        let parts_error = match population::<tables::NavyClasses>(frame, &Keys::All) {
            Ok(rows) => {
                for row in rows {
                    let row: &NavyClassRow = row;
                    if row.ownership != owner_code || !selected_slots.contains(&row.slot) {
                        continue;
                    }
                    let part = u128::from(row.class);
                    met.insert(part);
                    let value = match params.measure {
                        NavyMeasure::Ships => Some(row.ships),
                        NavyMeasure::Strength => recorded_value(row.strength_raw),
                    };
                    by_slot.entry(row.slot).or_default().push((part, value));
                }
                None
            }
            Err(reason) => Some(reason),
        };
        let named = if parts_error.is_some() {
            Named::default()
        } else {
            name_navy_classes(frame, &met, &content, params.grouping, &mut warnings)
        };
        let (parts, rows, extra_warnings) = attach_parts(selected, named, &by_slot, parts_error);
        warnings.extend(extra_warnings);
        Ok(ForceAtReply { parts, rows, missing: None, warnings })
    }
}
