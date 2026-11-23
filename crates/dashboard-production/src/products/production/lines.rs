use dashboard_world::products::equipment_names::{self as names, EquipmentIcon, Named, OrganisationName, VariantName};

use dashboard_base::products::{Country, validate_country};
use super::{
    Appearance, DECIMALS, DETAIL_HOURS, LineKind, OutputPart, OutputRun, RunLines, appearances, held_hours, joined,
    weekly_units_raw, window_hours,
};
use dashboard_base::{Source, products::{LoadContext, Product, check_reply_budget}, span::HourSpan};
use dashboard_query::{Query, read};
use replay_columnar::summary::Source as Column;
use replay_data::rows::TableId;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::{Arc, atomic::AtomicBool},
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub struct ProductionLines;
pub struct ProductionLineDetail;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LineStep {
    pub first_hour: i32,
    pub last_hour: i32,
    #[doc = " It follows hours the line was not recorded as the country's."]
    pub starts_segment: bool,
    pub variant: String,
    pub active_factories: i32,
    pub requested_factories: i32,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LineHistory {
    #[doc = " Unique in the branch: the handle and which appearance of it this is."]
    pub key: String,
    pub handle: String,
    #[doc = " Its native class; null where not recorded."]
    pub kind: Option<LineKind>,
    pub steps: Vec<LineStep>,
    #[doc = " The output published when it ended, counting every unit it made in the branch."]
    pub final_output: Option<OutputRun>,
    #[doc = " The first hour it was gone while its country was still recorded: it ended before."]
    pub ended_by: Option<i32>,
    #[doc = " It ran at the first recorded hour of its country: it began before recording."]
    pub before_recording: bool,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProductionLinesParams {
    pub branch_id: String,
    pub country: Country,
    #[doc = " The held identity's own runs: lines are read only while the country holds them."]
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub within: Option<Vec<dashboard_base::span::HourSpan>>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ProductionLinesReply {
    pub branch_id: String,
    pub tag: String,
    pub decimals: u8,
    #[doc = " Every line of the country in the branch, in the order first seen."]
    pub lines: Vec<LineHistory>,
    #[doc = " Each variant a line made, named at one hour it was made."]
    pub variants: Vec<VariantName>,
    #[doc = " Each organisation a line was assigned to, named at one hour it was."]
    pub organisations: Vec<OrganisationName>,
    pub icons: Vec<EquipmentIcon>,
    #[doc = " Hours a holder of the tag was recorded, ascending: outside them its lines are not known,"]
    #[doc = " which is not none."]
    pub line_hours: Vec<HourSpan>,
    #[doc = " The hours one `production-line-detail` window covers."]
    pub detail_hours: i32,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProductionDetailParams {
    pub branch_id: String,
    pub country: Country,
    pub window: i32,
    #[doc = " The held identity's own runs: lines are read only while the country holds them."]
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub within: Option<Vec<dashboard_base::span::HourSpan>>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LineRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub starts_segment: bool,
    pub variant: String,
    #[doc = r#" The organisation's reference; "0" for none."#]
    pub mio: String,
    pub active_factories: i32,
    pub requested_factories: i32,
    #[doc = " The most factories the line takes (the efficiency average's clamp)."]
    pub max_factories: i32,
    #[doc = " IC per unit, fixed5."]
    pub cost_raw: String,
    #[doc = " IC added at each production tick, fixed5."]
    pub speed_raw: String,
    #[doc = " Units per week, fixed5; null without a positive cost."]
    pub weekly_raw: Option<String>,
    #[doc = " Efficiency, Q15: 100% is 3276800."]
    pub efficiency_raw: String,
    pub priority: i32,
    #[doc = " Units still ordered; −1 for no limit."]
    pub amount: i32,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LineDetail {
    #[doc = " The line's key in `production-lines`."]
    pub key: String,
    pub runs: Vec<LineRun>,
    #[doc = " Its output rows over the window, where recorded."]
    pub output: Vec<OutputRun>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ProductionLineDetailReply {
    pub branch_id: String,
    pub tag: String,
    pub decimals: u8,
    pub window: i32,
    #[doc = " The window's hours, both included."]
    pub first_hour: i32,
    pub last_hour: i32,
    #[doc = " Every line with an hour in the window, in the order first seen."]
    pub lines: Vec<LineDetail>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(ProductionLinesReply, ProductionLineDetailReply);

fn validate_params(branch: &str, country: &Country) -> Result<(), &'static str> {
    if branch.parse::<u64>().is_err() {
        return Err("Invalid branch");
    }
    validate_country(country)
}

pub(super) fn index_of(source: &Source) -> impl Fn(u32) -> io::Result<Arc<RunLines>> + '_ {
    move |run| crate::products::production::cached_production_lines(source, run)
}

fn column_runs(recording: &replay_recording::columnar_frames::Recording, run: u32, table: TableId, columns: &[&str], keys: &[u128], from: i32, to: i32) -> io::Result<Vec<std::collections::HashMap<u128, read::Runs>>> {
    if keys.is_empty() {
        return Ok(columns.iter().map(|_| std::collections::HashMap::new()).collect());
    }
    columns
        .iter()
        .map(|name| {
            let column = dashboard_declare::field(table, name)?;
            read::runs(recording, run, table, Column::Cell(column), Some(keys), from, to)
        })
        .collect()
}

static NONE: read::Runs = Vec::new();

fn steps(appearance: &Appearance, by_run: &BTreeMap<u32, Vec<std::collections::HashMap<u128, read::Runs>>>) -> (Vec<LineStep>, Option<LineKind>, BTreeSet<u64>, BTreeSet<u64>) {
    let key = u128::from(appearance.handle);
    let (mut out, mut kind, mut variants, mut mios) = (Vec::new(), None, BTreeSet::new(), BTreeSet::new());
    for piece in &appearance.pieces {
        let Some(columns) = by_run.get(&piece.run) else { continue };
        let runs: Vec<&read::Runs> = columns.iter().map(|column| column.get(&key).unwrap_or(&NONE)).collect();
        let mut first = true;
        for (start, end, values) in joined(&runs, piece.first, piece.last) {
            let [variant, active, requested, line_kind, mio] = [values[0], values[1], values[2], values[3], values[4]];
            kind = kind.or(line_kind.and_then(LineKind::of));
            mios.extend(mio.filter(|&mio| mio != 0).map(|mio| mio as u64));
            let (Some(variant), Some(active), Some(requested)) = (variant, active, requested) else {
                first = true;
                continue;
            };
            variants.insert(variant as u64);
            let step = LineStep { first_hour: start, last_hour: end, starts_segment: first && (piece.starts_segment || start != piece.first), variant: (variant as u64).to_string(), active_factories: active as i32, requested_factories: requested as i32 };
            match out.last_mut() {
                Some(last) if !first && same_step(last, &step) => last.last_hour = end,
                _ => out.push(step),
            }
            first = false;
        }
    }
    (out, kind, variants, mios)
}

fn same_step(a: &LineStep, b: &LineStep) -> bool {
    a.last_hour.checked_add(1) == Some(b.first_hour) && a.variant == b.variant && a.active_factories == b.active_factories && a.requested_factories == b.requested_factories
}

const OUTPUT_COLUMNS: [&str; 6] = ["variant", "mio", "units", "since_hour", "observed_since_creation", "exact"];

fn part(values: &[Option<i64>]) -> Option<(OutputPart, i32, bool)> {
    let [variant, mio, units, since, observed, exact] = [values[0]?, values[1]?, values[2]?, values[3]?, values[4]?, values[5]?];
    Some((OutputPart { variant: (variant as u64).to_string(), mio: (mio as u64).to_string(), units: units.to_string(), exact: exact != 0 }, since as i32, observed != 0))
}

fn ended_rows(appearance: &Appearance, index: &RunLines) -> Vec<super::OutputLife> {
    let held_to = appearance.pieces.last().map_or(appearance.last(), |piece| piece.held_to);
    index
        .outputs_of(appearance.handle)
        .iter()
        .filter(|row| row.first <= appearance.last() && row.ended.is_some_and(|ended| ended >= appearance.last() && ended <= held_to))
        .map(|row| super::OutputLife { last: row.last.min(held_to), ..*row })
        .collect()
}

fn ended_parts(recording: &replay_recording::columnar_frames::Recording, run: u32, asks: &[(i32, u128)]) -> io::Result<BTreeMap<u128, (OutputPart, i32, bool)>> {
    let columns: Vec<usize> = OUTPUT_COLUMNS.iter().map(|name| dashboard_declare::field(TableId::ProductionLineOutput, name)).collect::<io::Result<_>>()?;
    let mut parts = BTreeMap::new();
    dashboard_query::cost::storage(|| {
        replay_lanes::lanes::at_each(recording, run, TableId::ProductionLineOutput, Some(&columns), asks, &mut |_, key, row| {
            let Some((view, slot)) = row else { return Ok(()) };
            let values: Vec<Option<i64>> = columns.iter().map(|&column| view.number(column, slot)).collect();
            if let Some(part) = part(&values) {
                parts.insert(key, part);
            }
            Ok(())
        })
    })?;
    Ok(parts)
}

fn final_output(rows: &[super::OutputLife], parts: &BTreeMap<u128, (OutputPart, i32, bool)>, ended_by: i32) -> Option<OutputRun> {
    let found: Vec<&(OutputPart, i32, bool)> = rows.iter().filter_map(|row| parts.get(&row.key)).collect();
    if found.is_empty() {
        return None;
    }
    let first = rows.iter().filter_map(|row| row.ended).min().unwrap_or(ended_by);
    let last = rows.iter().map(|row| row.last).max().unwrap_or(first);
    Some(OutputRun {
        first_hour: first,
        last_hour: last,
        since_hour: found.iter().map(|(_, since, _)| *since).min().unwrap_or(first),
        whole_life: found.iter().all(|(.., whole)| *whole),
        parts: found.into_iter().map(|(part, ..)| part.clone()).collect(),
    })
}

impl Product for ProductionLines {
    const KEY: &'static str = "production-lines";
    const CONTENT_DEPENDENT: bool = true;
    type Params = ProductionLinesParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, country, "within[].firstHour": Start, "within[].lastHour": End];
    type Reply = ProductionLinesReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![
            LineKind::decl(),
            OutputPart::decl(),
            OutputRun::decl(),
            VariantName::decl(),
            OrganisationName::decl(),
            EquipmentIcon::decl(),
            LineStep::decl(),
            LineHistory::decl(),
            ProductionLinesParams::decl(),
            ProductionLinesReply::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        validate_params(&params.branch_id, &params.country)
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let branch = params.branch_id.parse::<u64>().map_err(|_| dashboard_core::invalid("Invalid branch"))?;
        let query = dashboard_base::queries::query(source, branch)?;
        let content = names::context(source, context)?;
        let reply = lines_reply(source, &query, params, &content, cancelled)?;
        check_reply_budget(&reply)?;
        Ok(reply)
    }
}

pub(super) fn lines_reply(source: &Source, query: &Query<'_>, params: &ProductionLinesParams, content: &names::Context, cancelled: &AtomicBool) -> io::Result<ProductionLinesReply> {
    let recording = query.replay.recording();
    let index = index_of(source);
    let (from, to) = (query.timeline.first, query.timeline.last);
    let found = appearances(query, &params.country, params.within.as_deref(), from, to, &index)?;
    let mut warnings = Vec::new();
    if !query.holders.named(&params.country) {
        warnings.push(query.holders.say(&params.country, to, |name| format!("{name} is held by no country in this branch")));
    }
    let mut by_run: BTreeMap<u32, Vec<std::collections::HashMap<u128, read::Runs>>> = BTreeMap::new();
    let mut spans: BTreeMap<u32, (i32, i32, BTreeSet<u128>)> = BTreeMap::new();
    for appearance in &found {
        for piece in &appearance.pieces {
            let span = spans.entry(piece.run).or_insert((piece.first, piece.last, BTreeSet::new()));
            span.0 = span.0.min(piece.first);
            span.1 = span.1.max(piece.last);
            span.2.insert(u128::from(appearance.handle));
        }
    }
    for (&run, (first, last, keys)) in &spans {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let keys: Vec<u128> = keys.iter().copied().collect();
        by_run.insert(run, column_runs(recording, run, TableId::ProductionLines, &["variant", "active_factories", "requested_factories", "kind", "mio"], &keys, *first, *last)?);
    }
    let mut ended: Vec<Option<(i32, Vec<super::OutputLife>)>> = Vec::with_capacity(found.len());
    let mut asks: BTreeMap<u32, Vec<(i32, u128)>> = BTreeMap::new();
    for appearance in &found {
        let last_piece = appearance.pieces.last().copied().expect("an appearance holds a piece");
        let ended_by = (last_piece.last < last_piece.held_to).then(|| last_piece.last + 1);
        let rows = match ended_by {
            Some(_) => ended_rows(appearance, &*index(last_piece.run)?),
            None => Vec::new(),
        };
        asks.entry(last_piece.run).or_default().extend(rows.iter().map(|row| (row.last, row.key)));
        ended.push(ended_by.map(|ended_by| (ended_by, rows)));
    }
    let mut parts: BTreeMap<u32, BTreeMap<u128, (OutputPart, i32, bool)>> = BTreeMap::new();
    for (run, asked) in &asks {
        dashboard_text::metadata::check_cancel(cancelled)?;
        if !asked.is_empty() {
            parts.insert(*run, ended_parts(recording, *run, asked)?);
        }
    }
    let no_parts = BTreeMap::new();
    let mut lines = Vec::with_capacity(found.len());
    let mut used: BTreeMap<(u32, i32), (BTreeSet<u64>, BTreeSet<u64>)> = BTreeMap::new();
    let mut first_used: BTreeMap<u64, i32> = BTreeMap::new();
    for (appearance, ended) in found.iter().zip(&ended) {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let (steps, kind, variants, mios) = steps(appearance, &by_run);
        let last_piece = appearance.pieces.last().copied().expect("an appearance holds a piece");
        let ended_by = ended.as_ref().map(|(ended_by, _)| *ended_by);
        let final_output = ended.as_ref().and_then(|(ended_by, rows)| final_output(rows, parts.get(&last_piece.run).unwrap_or(&no_parts), *ended_by));
        let named_at = (last_piece.run, last_piece.last);
        let entry = used.entry(named_at).or_default();
        entry.0.extend(variants.iter().copied());
        entry.0.extend(final_output.iter().flat_map(|output| output.parts.iter().filter_map(|part| part.variant.parse::<u64>().ok())));
        entry.1.extend(mios.iter().copied());
        entry.1.extend(final_output.iter().flat_map(|output| output.parts.iter().filter_map(|part| part.mio.parse::<u64>().ok()).filter(|&mio| mio != 0)));
        for step in &steps {
            if let Ok(variant) = step.variant.parse::<u64>() {
                let held = first_used.entry(variant).or_insert(step.first_hour);
                *held = (*held).min(step.first_hour);
            }
        }
        let first_piece = appearance.pieces[0];
        lines.push(LineHistory {
            key: appearance.key.clone(),
            handle: appearance.handle.to_string(),
            kind,
            steps,
            final_output,
            ended_by,
            before_recording: first_piece.first == first_piece.held_from,
        });
    }
    let mut named = Named::default();
    name_used(query, &used, content, &mut named, cancelled)?;
    warnings.extend(std::mem::take(&mut named.warnings));
    let icons = named.icons();
    let line_hours = held_hours(query, &params.country, params.within.as_deref(), from, to);
    let named_at = if params.within.is_some() { line_hours.last().map_or(to, |span| span.last_hour) } else { to };
    Ok(ProductionLinesReply {
        branch_id: params.branch_id.clone(),
        tag: query.holders.name(&params.country, named_at),
        decimals: DECIMALS,
        lines,
        variants: named.variants.into_values().collect(),
        organisations: named.organisations.into_values().collect(),
        icons,
        line_hours,
        detail_hours: DETAIL_HOURS,
        warnings,
    })
}

fn name_used(query: &Query<'_>, used: &BTreeMap<(u32, i32), (BTreeSet<u64>, BTreeSet<u64>)>, content: &names::Context, named: &mut Named, cancelled: &AtomicBool) -> io::Result<()> {
    let recording = query.replay.recording();
    let mut latest: BTreeMap<u32, (i32, BTreeSet<u64>, BTreeSet<u64>)> = BTreeMap::new();
    for (&(run, hour), (variants, mios)) in used {
        let held = latest.entry(run).or_insert((hour, BTreeSet::new(), BTreeSet::new()));
        held.0 = held.0.max(hour);
        held.1.extend(variants);
        held.2.extend(mios);
    }
    for (&run, (hour, variants, mios)) in &latest {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let variants: Vec<u64> = variants.iter().copied().collect();
        let mios: Vec<u64> = mios.iter().copied().collect();
        names::name_at(recording, run, *hour, &variants, &mios, content, named)?;
    }
    for (&(run, hour), (variants, mios)) in used.iter().rev() {
        let variants: Vec<u64> = variants.iter().copied().filter(|id| !named.variants.get(id).is_some_and(|held| held.known)).collect();
        let mios: Vec<u64> = mios.iter().copied().filter(|id| named.organisations.get(id).is_none_or(|held| held.token.is_empty())).collect();
        if variants.is_empty() && mios.is_empty() {
            continue;
        }
        dashboard_text::metadata::check_cancel(cancelled)?;
        names::name_at(recording, run, hour, &variants, &mios, content, named)?;
    }
    Ok(())
}

fn line_runs(appearance: &Appearance, by_run: &BTreeMap<u32, Vec<std::collections::HashMap<u128, read::Runs>>>, first: i32, last: i32) -> Vec<LineRun> {
    let key = u128::from(appearance.handle);
    let mut out: Vec<LineRun> = Vec::new();
    for piece in &appearance.pieces {
        let (from, to) = (piece.first.max(first), piece.last.min(last));
        if from > to {
            continue;
        }
        let Some(columns) = by_run.get(&piece.run) else { continue };
        let runs: Vec<&read::Runs> = columns.iter().map(|column| column.get(&key).unwrap_or(&NONE)).collect();
        let mut gap = true;
        for (start, end, values) in joined(&runs, from, to) {
            let [variant, mio, active, requested, max, cost, speed, efficiency, priority, amount] =
                [values[0], values[1], values[2], values[3], values[4], values[5], values[6], values[7], values[8], values[9]];
            let (Some(variant), Some(mio), Some(active), Some(requested), Some(max), Some(cost), Some(speed), Some(efficiency), Some(priority), Some(amount)) =
                (variant, mio, active, requested, max, cost, speed, efficiency, priority, amount)
            else {
                gap = true;
                continue;
            };
            out.push(LineRun {
                first_hour: start,
                last_hour: end,
                starts_segment: gap && (start != piece.first || piece.starts_segment),
                variant: (variant as u64).to_string(),
                mio: (mio as u64).to_string(),
                active_factories: active as i32,
                requested_factories: requested as i32,
                max_factories: max as i32,
                cost_raw: cost.to_string(),
                speed_raw: speed.to_string(),
                weekly_raw: weekly_units_raw(speed, cost).map(|weekly| weekly.to_string()),
                efficiency_raw: efficiency.to_string(),
                priority: priority as i32,
                amount: amount as i32,
            });
            gap = false;
        }
    }
    out
}

fn output_runs(appearance: &Appearance, outputs: &BTreeMap<u32, (Vec<u128>, Vec<std::collections::HashMap<u128, read::Runs>>)>, first: i32, last: i32) -> Vec<OutputRun> {
    let low = u128::from(appearance.handle) << 32;
    let high = low | u128::from(u32::MAX);
    let mut out: Vec<OutputRun> = Vec::new();
    for piece in &appearance.pieces {
        let (from, to) = (piece.first.max(first), piece.last.min(last));
        if from > to {
            continue;
        }
        let Some((keys, columns)) = outputs.get(&piece.run) else { continue };
        let mine: Vec<u128> = keys.iter().copied().filter(|key| (low..=high).contains(key)).collect();
        if mine.is_empty() {
            continue;
        }
        let runs: Vec<&read::Runs> = mine.iter().flat_map(|key| columns.iter().map(move |column| column.get(key).unwrap_or(&NONE))).collect();
        for (start, end, values) in joined(&runs, from, to) {
            let parts: Vec<(OutputPart, i32, bool)> = values.chunks(OUTPUT_COLUMNS.len()).filter_map(part).collect();
            if parts.is_empty() {
                continue;
            }
            let run = OutputRun {
                first_hour: start,
                last_hour: end,
                since_hour: parts.iter().map(|(_, since, _)| *since).min().unwrap_or(start),
                whole_life: parts.iter().all(|(.., whole)| *whole),
                parts: parts.into_iter().map(|(part, ..)| part).collect(),
            };
            match out.last_mut() {
                Some(held) if held.last_hour.checked_add(1) == Some(start) && held.parts == run.parts && held.since_hour == run.since_hour && held.whole_life == run.whole_life => held.last_hour = end,
                _ => out.push(run),
            }
        }
    }
    out
}

const DETAIL_COLUMNS: [&str; 10] = ["variant", "mio", "active_factories", "requested_factories", "max_factories", "cost_raw", "speed_raw", "efficiency_raw", "priority", "amount"];

impl Product for ProductionLineDetail {
    const KEY: &'static str = "production-line-detail";
    const CONTENT_DEPENDENT: bool = false;
    type Params = ProductionDetailParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, country, window, "within[].firstHour": Start, "within[].lastHour": End];
    type Reply = ProductionLineDetailReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![LineRun::decl(), LineDetail::decl(), ProductionDetailParams::decl(), ProductionLineDetailReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        validate_params(&params.branch_id, &params.country)?;
        window_hours(params.window).map(|_| ()).ok_or("Invalid production window")
    }

    fn load(source: &Source, params: &Self::Params, _context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let branch = params.branch_id.parse::<u64>().map_err(|_| dashboard_core::invalid("Invalid branch"))?;
        let query = dashboard_base::queries::query(source, branch)?;
        let reply = detail_reply(source, &query, params, cancelled)?;
        check_reply_budget(&reply)?;
        Ok(reply)
    }
}

pub(super) fn detail_reply(source: &Source, query: &Query<'_>, params: &ProductionDetailParams, cancelled: &AtomicBool) -> io::Result<ProductionLineDetailReply> {
    let (first, last) = window_hours(params.window).ok_or_else(|| dashboard_core::invalid("Invalid production window"))?;
    let recording = query.replay.recording();
    let index = index_of(source);
    let found: Vec<Appearance> = appearances(query, &params.country, params.within.as_deref(), query.timeline.first, query.timeline.last, &index)?
        .into_iter()
        .filter(|appearance| appearance.pieces.iter().any(|piece| piece.first <= last && piece.last >= first))
        .collect();
    let mut spans: BTreeMap<u32, (i32, i32, BTreeSet<u64>)> = BTreeMap::new();
    for appearance in &found {
        for piece in appearance.pieces.iter().filter(|piece| piece.first <= last && piece.last >= first) {
            let span = spans.entry(piece.run).or_insert((i32::MAX, i32::MIN, BTreeSet::new()));
            span.0 = span.0.min(piece.first.max(first));
            span.1 = span.1.max(piece.last.min(last));
            span.2.insert(appearance.handle);
        }
    }
    let mut by_run = BTreeMap::new();
    let mut outputs = BTreeMap::new();
    for (&run, (from, to, handles)) in &spans {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let keys: Vec<u128> = handles.iter().map(|&handle| u128::from(handle)).collect();
        by_run.insert(run, column_runs(recording, run, TableId::ProductionLines, &DETAIL_COLUMNS, &keys, *from, *to)?);
        let lines = index(run)?;
        let output_keys: Vec<u128> = handles.iter().flat_map(|&handle| lines.outputs_of(handle).iter().filter(|row| row.first <= *to && row.last >= *from).map(|row| row.key)).collect();
        outputs.insert(run, (output_keys.clone(), column_runs(recording, run, TableId::ProductionLineOutput, &OUTPUT_COLUMNS, &output_keys, *from, *to)?));
    }
    let lines = found
        .iter()
        .map(|appearance| LineDetail { key: appearance.key.clone(), runs: line_runs(appearance, &by_run, first, last), output: output_runs(appearance, &outputs, first, last) })
        .collect();
    let named_at = super::held_hours(query, &params.country, params.within.as_deref(), first, last).last().map_or(last, |span| span.last_hour);
    Ok(ProductionLineDetailReply { branch_id: params.branch_id.clone(), tag: query.holders.name(&params.country, named_at), decimals: DECIMALS, window: params.window, first_hour: first, last_hour: last, lines, warnings: Vec::new() })
}
