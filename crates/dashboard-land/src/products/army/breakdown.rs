use super::{
    ArmyAttribution, ArmyMeasure, ArmyMixRun, ArmyRun, ArmyType, Groups, cached_army_mix, geometry, group_info, holding, invalid, line, mixes_over, token_name,
};
use dashboard_base::{
    Source,
    products::{Country, LoadContext, Product},
};
use dashboard_query::{identity::Holder, read, series::MissingSpan};
use replay_columnar::{manifest::Status, summary};
use replay_data::rows::TableId;
use replay_lanes::lanes;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::{Arc, atomic::AtomicBool},
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub struct ArmyBreakdown;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArmyBreakdownParams {
    pub branch_id: String,
    pub country: Country,
    pub count_for: ArmyAttribution,
    #[doc = " The hour answered: nothing recorded after it is read into the reply."]
    pub hour: i32,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ArmyBreakdownReply {
    #[doc = " The hour answered, never after the branch's last hour."]
    pub hour: i32,
    #[doc = " The first hour after `hour` at which totals, unit-group mix, holder identity, or table coverage changes; null when none does."]
    pub next_change: Option<i32>,
    pub available: bool,
    pub types: Vec<ArmyType>,
    #[cfg_attr(any(test, feature = "export-types"), ts(type = "Array<ArmyRun>"))]
    pub runs: Vec<ArmyRun>,
    pub mix: Vec<ArmyMixRun>,
    pub missing: Vec<MissingSpan>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(ArmyBreakdownReply);

struct SlotArmyHistory {
    totals: read::Runs,
    totals_statuses: Vec<lanes::StatusRun>,
    mix_statuses: Vec<lanes::StatusRun>,
    token_runs: Vec<(u32, read::Runs)>,
}

#[derive(Default)]
struct BreakdownCache {
    by_stretch_slot: HashMap<(u32, i32, i32, u32, u8), Arc<SlotArmyHistory>>,
}

fn cached_slot_history(
    source: &Source,
    totals_source: summary::Source,
    run: u32,
    first: i32,
    last: i32,
    slot: u32,
    attribution: ArmyAttribution,
) -> io::Result<Arc<SlotArmyHistory>> {
    let attr_code = u8::from(attribution == ArmyAttribution::Host);
    let key = (run, first, last, slot, attr_code);
    let cache = source.state::<BreakdownCache>();
    if let Some(held) = cache.lock().unwrap_or_else(std::sync::PoisonError::into_inner).by_stretch_slot.get(&key) {
        return Ok(Arc::clone(held));
    }
    let recording = source.replay_ref().recording();
    let slot_key = [u128::from(slot)];
    let totals = read::runs(recording, run, TableId::ArmyTotals, totals_source, Some(&slot_key), first, last)?
        .remove(&slot_key[0])
        .unwrap_or_default();
    let totals_statuses = lanes::statuses(recording, run, TableId::ArmyTotals, first, last)?;
    let mix_statuses = lanes::statuses(recording, run, TableId::ArmyMix, first, last)?;
    let lives = cached_army_mix(source, run)?;
    let low = u128::from(slot) << 40 | u128::from(attr_code) << 32;
    let start = lives.partition_point(|&(k, ..)| k < low);
    let mix_keys: Vec<u128> = lives[start..]
        .iter()
        .take_while(|&&(k, ..)| k <= low | u128::from(u32::MAX))
        .filter(|&&(_, born, gone)| born <= i64::from(last) && gone >= i64::from(first))
        .map(|&(k, ..)| k)
        .collect();
    let battalions_col = summary::Source::Cell(dashboard_declare::field(TableId::ArmyMix, "battalions")?);
    let mut token_runs: Vec<(u32, read::Runs)> = if mix_keys.is_empty() {
        Vec::new()
    } else {
        read::runs(recording, run, TableId::ArmyMix, battalions_col, Some(&mix_keys), first, last)?
            .into_iter()
            .map(|(k, runs)| (k as u32, runs))
            .collect()
    };
    token_runs.sort_unstable_by_key(|(token, _)| *token);
    let built = Arc::new(SlotArmyHistory { totals, totals_statuses, mix_statuses, token_runs });
    let mut guard = cache.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    Ok(Arc::clone(guard.by_stretch_slot.entry(key).or_insert(built)))
}

fn status_at(statuses: &[lanes::StatusRun], hour: i32) -> Status {
    statuses.iter().find(|run| run.first <= hour && hour <= run.last).map_or(Status::NotRecorded, |run| run.status)
}

fn totals_status_reason(status: Status) -> String {
    match status {
        Status::Complete => String::new(),
        Status::NotRecorded => "Army totals were not recorded at these hours".to_owned(),
        Status::Truncated { missing } => format!("Army totals were read in part at these hours ({missing} rows not read)"),
        Status::Unavailable { .. } => "Army totals were not available at these hours".to_owned(),
    }
}

fn mix_status_reason(status: Status) -> String {
    match status {
        Status::Complete => String::new(),
        Status::NotRecorded => "Battalion unit groups were not recorded at these hours".to_owned(),
        Status::Truncated { missing } => format!("Battalion unit groups were read in part at these hours ({missing} rows not read)"),
        Status::Unavailable { .. } => "Battalion unit groups were not available at these hours".to_owned(),
    }
}

fn push_missing(out: &mut Vec<MissingSpan>, first_hour: i32, last_hour: i32, reason: String) {
    if first_hour > last_hour {
        return;
    }
    match out.last_mut() {
        Some(prev) if prev.last_hour + 1 >= first_hour && prev.reason == reason => {
            prev.last_hour = prev.last_hour.max(last_hour);
        }
        _ => out.push(MissingSpan { first_hour, last_hour, reason }),
    }
}

fn push_run(out: &mut Vec<ArmyRun>, first_hour: i32, last_hour: i32, value: i64) {
    let total = Some(geometry(value, ArmyMeasure::Battalions));
    let raw = value.to_string();
    match out.last_mut() {
        Some(prev) if prev.last_hour + 1 == first_hour && prev.raw == raw => prev.last_hour = last_hour,
        _ => out.push(ArmyRun { first_hour, last_hour, total, raw }),
    }
}

fn next_slot_change(held: &SlotArmyHistory, hour: i32, limit: i32) -> Option<i32> {
    let mut next = i32::MAX;
    let mut after = |at: i32| {
        if at > hour && at <= limit && at < next {
            next = at;
        }
    };
    for &(first, last, _) in &held.totals {
        after(first);
        after(last.saturating_add(1));
    }
    for run in held.totals_statuses.iter().chain(held.mix_statuses.iter()) {
        after(run.first);
        after(run.last.saturating_add(1));
    }
    for (_, runs) in &held.token_runs {
        for &(first, last, _) in runs {
            after(first);
            after(last.saturating_add(1));
        }
    }
    (next != i32::MAX).then_some(next)
}

impl Product for ArmyBreakdown {
    const KEY: &'static str = "army-breakdown";
    const CONTENT_DEPENDENT: bool = true;
    const INTERACTIVE: bool = true;
    type Params = ArmyBreakdownParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, country, countFor, hour: Read];
    type Reply = ArmyBreakdownReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![ArmyBreakdownParams::decl(), ArmyBreakdownReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        if params.branch_id.parse::<u64>().is_err() {
            return Err("Invalid branch");
        }
        dashboard_base::products::validate_country(&params.country)
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let branch = params.branch_id.parse::<u64>().map_err(|_| invalid("Invalid branch"))?;
        let query = dashboard_base::queries::query(source, branch)?;
        let recording = query.replay.recording();
        let line = line(&query, ArmyMeasure::Battalions, params.count_for)?;
        let totals_source = line.source();
        let branch_first = query.timeline.first;
        let branch_last = query.timeline.last;
        let hour = params.hour.min(branch_last);

        let mut next = i32::MAX;
        let mut after = |at: i32| {
            if at > hour && at <= branch_last && at < next {
                next = at;
            }
        };
        for stretch in &query.timeline.stretches {
            after(stretch.first);
            after(stretch.last.saturating_add(1));
        }
        for run in query.holders.runs(&params.country) {
            after(run.first);
            after(run.last.saturating_add(1));
        }
        for &(first, last, _) in &query.holders.unknown {
            after(first);
            after(last.saturating_add(1));
        }

        let parts = if hour >= branch_first { line.parts(&params.country, branch_first, hour) } else { Vec::new() };
        let stretches: HashMap<u32, Vec<(i32, i32)>> = query.timeline.stretches.iter().fold(HashMap::new(), |mut acc, s| {
            acc.entry(s.run).or_default().push((s.first, s.last));
            acc
        });

        let mut runs: Vec<ArmyRun> = Vec::new();
        let mut raw_mix: Vec<(i32, i32, BTreeMap<u32, i64>)> = Vec::new();
        let mut missing: Vec<MissingSpan> = Vec::new();
        let mut named: BTreeMap<u32, (i32, BTreeSet<u32>)> = BTreeMap::new();
        let mut any_complete_totals = false;

        for run in query.holders.runs(&params.country) {
            if matches!(run.holder, Holder::Unresolved) && run.first <= hour {
                let lo = run.first.max(branch_first);
                let hi = run.last.min(hour);
                push_missing(&mut missing, lo, hi, "Country identity was not recorded at these hours".to_owned());
            }
        }
        for &(u_first, u_last, ref reason) in &query.holders.unknown {
            if u_first <= hour {
                let lo = u_first.max(branch_first);
                let hi = u_last.min(hour);
                push_missing(&mut missing, lo, hi, reason.clone());
            }
        }

        for part in &parts {
            dashboard_text::metadata::check_cancel(cancelled)?;
            let &(stretch_first, stretch_last) = stretches
                .get(&part.run)
                .and_then(|spans| spans.iter().find(|&&(a, b)| a <= part.first && part.last <= b))
                .unwrap_or(&(part.first, part.last));
            let held = cached_slot_history(source, totals_source, part.run, stretch_first, stretch_last, part.slot, params.count_for)?;
            if part.last == hour && stretch_last > hour
                && let Some(at) = next_slot_change(&held, hour, stretch_last)
            {
                after(at);
            }

            let mut cuts = vec![part.first];
            for &(first, last, _) in &held.totals {
                cuts.extend([first, last.saturating_add(1)]);
            }
            for s in held.totals_statuses.iter().chain(held.mix_statuses.iter()) {
                cuts.extend([s.first, s.last.saturating_add(1)]);
            }
            cuts.retain(|&c| part.first <= c && c <= part.last);
            cuts.sort_unstable();
            cuts.dedup();

            let mut totals_at = 0usize;
            let mut mix_known: Vec<(i32, i32, Option<i64>)> = Vec::new();
            for (idx, &a) in cuts.iter().enumerate() {
                let b = cuts.get(idx + 1).map_or(part.last, |n| n - 1);
                let t_status = status_at(&held.totals_statuses, a);
                if t_status != Status::Complete {
                    push_missing(&mut missing, a, b, totals_status_reason(t_status));
                    continue;
                }
                any_complete_totals = true;
                match holding(&held.totals, &mut totals_at, a) {
                    None => {
                        push_missing(&mut missing, a, b, "Battalions were not recorded at these hours".to_owned());
                    }
                    Some(total) => {
                        push_run(&mut runs, a, b, total);
                        let m_status = status_at(&held.mix_statuses, a);
                        if m_status != Status::Complete {
                            push_missing(&mut missing, a, b, mix_status_reason(m_status));
                        } else {
                            mix_known.push((a, b, Some(total)));
                        }
                    }
                }
            }

            if !mix_known.is_empty() {
                let token_slices: Vec<(u32, &read::Runs)> = held.token_runs.iter().map(|(token, r)| (*token, r)).collect();
                let part_mix = mixes_over(part.first, part.last, &mix_known, &token_slices);
                let entry = named.entry(part.run).or_insert((part.last, BTreeSet::new()));
                entry.0 = entry.0.max(part.last);
                for (first, last, counts) in part_mix {
                    entry.1.extend(counts.keys().copied());
                    raw_mix.push((first, last, counts));
                }
            }
        }

        let literal = dashboard_declare::field(TableId::LexicalTokens, "literal")?;
        let mut names: HashMap<u32, String> = HashMap::new();
        for (run, (lookup_hour, tokens)) in named {
            if tokens.is_empty() {
                continue;
            }
            let keys: Vec<u128> = tokens.iter().map(|&t| u128::from(t)).collect();
            let held = read::at(recording, run, &[read::Read { table: TableId::LexicalTokens, columns: Some(&[literal]), keys: read::Keys::These(&keys) }], lookup_hour)?;
            let (view, rows) = (held[0].view(), &held[0].rows);
            for token in tokens {
                let slot = rows.binary_search_by_key(&u128::from(token), |&(k, _)| k).ok().map(|at| rows[at].1);
                names.entry(token).or_insert_with(|| token_name(&view, slot, token));
            }
        }

        let mut warnings = Vec::new();
        let catalogue = match context.content.as_ref().map(|options| dashboard_base::catalogue::catalogue(source, options)).transpose() {
            Ok(catalogue) => catalogue,
            Err(error) => {
                warnings.push(format!("Unit-group content unavailable: {error}"));
                None
            }
        };
        let state = source.state::<Groups>();
        let mut groups = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(catalogue) = &catalogue
            && groups.content.as_ref().is_none_or(|(old, _)| !Arc::ptr_eq(old, catalogue))
        {
            let units = content_reader_key_names::unit_leader::sub_units(&catalogue.content.layers).map_err(|error| error.to_string());
            groups.content = Some((Arc::clone(catalogue), units));
        }
        let units = catalogue.as_ref().and_then(|_| groups.content.as_ref().map(|(_, units)| units));
        if let Some(Err(reason)) = units {
            warnings.push(format!("Unit-group reader refused: {reason}"));
        }
        let units = units.and_then(|u| u.as_ref().ok());
        if let Some(units) = units {
            warnings.extend(units.warnings().iter().cloned());
        }

        let tokens: BTreeSet<u32> = raw_mix.iter().flat_map(|(_, _, counts)| counts.keys().copied()).collect();
        let mut mapping: HashMap<u32, String> = HashMap::new();
        let mut types_map: BTreeMap<String, ArmyType> = BTreeMap::new();
        let mut unresolved = 0usize;
        let mut seen_names = BTreeSet::new();
        for token in &tokens {
            let name = names.get(token).cloned().unwrap_or_else(|| format!("Unresolved sub-unit token {token}"));
            let (key, info, warning) = group_info(units, catalogue.as_deref(), &name);
            mapping.insert(*token, key.clone());
            types_map.insert(key, info);
            if seen_names.insert(name)
                && let Some(warning) = warning
            {
                unresolved += 1;
                warnings.push(warning);
            }
        }
        if unresolved > 0 {
            warnings.push(format!("{unresolved} sub-unit group/name lookups are unresolved or refused; battalions remain counted."));
        }
        let types: Vec<ArmyType> = types_map.into_values().collect();
        let positions: HashMap<&String, usize> = types.iter().enumerate().map(|(at, info)| (&info.key, at)).collect();

        let mut mix: Vec<ArmyMixRun> = Vec::new();
        for (first, last, counts) in raw_mix {
            let mut values = vec![0i64; types.len()];
            for (token, count) in counts {
                values[positions[&mapping[&token]]] += count;
            }
            let raw: Vec<Option<String>> = values.iter().map(|v| Some(v.to_string())).collect();
            match mix.last_mut() {
                Some(prev) if prev.last_hour + 1 == first && prev.raw_values.as_ref() == Some(&raw) => prev.last_hour = last,
                _ => mix.push(ArmyMixRun {
                    first_hour: first,
                    last_hour: last,
                    values: values.iter().map(|v| Some(*v as f64)).collect(),
                    raw_values: Some(raw),
                }),
            }
        }

        missing.sort_unstable_by_key(|m| (m.first_hour, m.last_hour));
        let reply = ArmyBreakdownReply {
            hour,
            next_change: (next != i32::MAX).then_some(next),
            available: any_complete_totals || !runs.is_empty(),
            types,
            runs,
            mix,
            missing,
            warnings,
        };
        dashboard_base::products::check_reply_budget(&reply)?;
        Ok(reply)
    }
}
