//! The equipment stockpile at one hour (`equipment-stockpile-at`): one holder's stock,
//! production lines and logistics record as of its latest reads by the hour. Each stream is
//! its latest event by then over the keyframe a delta names, so an hour costs one
//! `latest_event` and at most a keyframe's chain per stream, plus the names and classes of
//! the variants it returns.

use super::{
    EquipmentStockpileReply, NEED_BOUND_RAW, StockVariant, TypeLogistics,
    content::{Reading, Types},
    emit as layout,
    read::{self, Cells, DailyStream, LogisticsStream},
};
use dashboard_base::{
    products::{country_groups::HourSpan, equipment::EQUIPMENT_DECIMALS, validate_tag},
    read_runs::AmountRun,
};
use dashboard_series::{PageRead, Snapshot, family::Directory};
use dashboard_text::catalogue::Refusals;
use serde::{Deserialize, Serialize};
use std::{io, sync::atomic::AtomicBool};
use ts_rs::TS;

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StockpileAtParams {
    pub branch_id: String,
    pub tag: String,
    pub hour: i32,
}

impl StockpileAtParams {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.branch_id.parse::<u64>().is_err() {
            return Err("Invalid branch");
        }
        validate_tag(&self.tag)
    }
}

/// The holder's stockpile at the hour; none past the publication's accepted hours, which are
/// pending.
pub fn query<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    params: &StockpileAtParams,
    reading: &Reading<'_>,
    cancelled: &AtomicBool,
) -> io::Result<Option<EquipmentStockpileReply>> {
    let hour = params.hour;
    if hour > snapshot.publication().accepted.last {
        return Ok(None);
    }
    let content = &reading.content;
    let mut reply = EquipmentStockpileReply {
        branch_id: params.branch_id.clone(),
        tag: params.tag.clone(),
        campaign_key: reading.campaign_key.clone(),
        decimals: EQUIPMENT_DECIMALS,
        types: Vec::new(),
        variants: Vec::new(),
        reads: Vec::new(),
        stock_hours: Vec::new(),
        line_hours: Vec::new(),
        line_reads: Vec::new(),
        logistics: Vec::new(),
        logistics_reads: Vec::new(),
        balance_hours: Vec::new(),
        ai_hours: Vec::new(),
        need_bound_raw: NEED_BOUND_RAW.to_string(),
        warnings: content.warnings.clone(),
    };
    let mut types = Types::default();
    if let Some(actor) = directory.actor(params.tag.as_bytes()) {
        let key = |measure| directory.key(actor, measure, None);
        let mut cells = Cells::default();
        let mut known = [false; 3];
        for (known, measure) in known.iter_mut().zip([
            layout::STOCK_HOURS,
            layout::LINES_HOURS,
            layout::LOGISTICS_HOURS,
        ]) {
            *known = read::known(read::evidence_at(snapshot, key(measure), hour)?.as_ref());
        }
        let [stock, lines, logistics] = known;
        let span = HourSpan {
            first_hour: hour,
            last_hour: hour,
        };
        let run = |amount: Option<i64>| {
            vec![AmountRun {
                first_hour: hour,
                last_hour: hour,
                amount_raw: amount.map(|amount| amount.to_string()),
            }]
        };
        let (mut unnamed, mut refused) = (0, Refusals::default());
        if stock || lines {
            // A known span starts at a read, which writes a keyframe.
            let (event, rows) =
                read::rows_at::<DailyStream, _>(snapshot, key(layout::DAILY), hour, &mut cells)?
                    .ok_or_else(read::malformed)?;
            let schedule = directory.key(layout::FAMILY, layout::DAILY_READS, None);
            let at = read::latest_read(snapshot, schedule, event, hour)?;
            if stock {
                reply.reads.push(at);
                reply.stock_hours.push(span);
            }
            if lines {
                reply.line_reads.push(at);
                reply.line_hours.push(span);
            }
            let forget = read::forget_at(snapshot, directory, hour)?;
            for (&id, row) in &rows {
                let (held, made) = (stock && row.stock_present, lines && row.lines_present);
                if !held && !made {
                    continue;
                }
                dashboard_text::metadata::check_cancel(cancelled)?;
                let token = read::token(snapshot, directory, id, &mut cells)?.unwrap_or_default();
                let naming = read::naming_at(snapshot, directory, id, hour, forget, &mut cells)?;
                if naming.is_none() {
                    unnamed += 1;
                }
                let names = naming
                    .map(|(first, raw)| {
                        for (_, why) in raw.iter().flat_map(|raw| &raw.refusals) {
                            refused.note(why.as_str(), id.to_string());
                        }
                        vec![read::name(first, raw.as_ref(), content.texts())]
                    })
                    .unwrap_or_default();
                let line = |amount| if made { run(amount) } else { Vec::new() };
                reply.variants.push(StockVariant {
                    id: id.to_string(),
                    archetype: types.variant(content, &token),
                    token,
                    names,
                    stock: if held { run(row.stock) } else { Vec::new() },
                    daily: line(row.units),
                    daily_ic: line(row.ic),
                    unit_cost_low: line(row.low),
                    unit_cost_high: line(row.high),
                });
            }
        }
        if logistics {
            let (event, rows) = read::rows_at::<LogisticsStream, _>(
                snapshot,
                key(layout::LOGISTICS),
                hour,
                &mut cells,
            )?
            .ok_or_else(read::malformed)?;
            if rows.is_empty() {
                // The game computed no balance: the holder was AI-controlled at the read.
                reply.ai_hours.push(span);
            } else {
                let schedule = directory.key(layout::FAMILY, layout::LOGISTICS_READS, None);
                reply
                    .logistics_reads
                    .push(read::latest_read(snapshot, schedule, event, hour)?);
                reply.balance_hours.push(span);
                let branch = params.branch_id.parse().expect("validated stockpile branch");
                let tokens = read::tokens(snapshot, directory, branch, reading, cancelled)?;
                for (token, row) in rows {
                    let spelling = tokens
                        .get(&token)
                        .cloned()
                        .unwrap_or_else(|| format!("token {token}"));
                    let key = types.logistics(content, &spelling);
                    // Two tokens of one archetype: the later token's row holds.
                    reply.logistics.retain(|item| item.key != key);
                    reply.logistics.push(TypeLogistics {
                        key,
                        balance: run(row.balance),
                        stock: run(row.stock),
                        available: run(row.available),
                    });
                }
            }
        }
        let counts = read::counts_at(snapshot, key(layout::COUNTS), hour, &mut cells)?;
        warnings(&mut reply.warnings, counts, known, unnamed, &refused);
    }
    reply.types = types.finish(content, &mut reply.warnings);
    reply.warnings.extend(content.refusals());
    Ok(Some(reply))
}

/// The read's own warnings, from the parts of its counts whose coverage holds at the hour.
fn warnings(
    warnings: &mut Vec<String>,
    counts: [u64; 11],
    [stock, lines, logistics]: [bool; 3],
    unnamed: usize,
    refused: &Refusals,
) {
    let part = |held: bool, at: usize| if held { counts[at] } else { 0 };
    if unnamed > 0 {
        warnings.push(format!(
            "{unnamed} variants in stock or production have no recorded definition by this hour; their type is unknown."
        ));
    }
    warnings.extend(refused.diagnostic_lines("equipment variants with unknown text/design operands"));
    let costless = part(lines, 1);
    if costless > 0 {
        warnings.push(format!(
            "{costless} production lines at this read have no positive cost, so their variant's daily output and cost per unit are unknown (the game shows only a placeholder rate for them)."
        ));
    }
    let other = part(lines, 2);
    if other > 0 {
        warnings.push(format!(
            "{other} naval, refit or railway-gun lines at this read make equipment whose daily output is not established, so it, its IC and its cost are unknown."
        ));
    }
    let missing = part(logistics, 7);
    if missing > 0 {
        warnings.push(format!(
            "{missing} logistics archetypes at this read have no matching recorded available-stock operand; convoy, truck and train need remains unknown."
        ));
    }
    let overflowed = part(stock, 0)
        + part(lines, 3)
        + part(lines, 4)
        + part(logistics, 8)
        + part(logistics, 9)
        + part(logistics, 10);
    if overflowed > 0 {
        warnings.push(format!(
            "{overflowed} recorded totals at this read do not fit a 64-bit amount; they are shown as unknown, never clamped."
        ));
    }
}

#[cfg(test)]
mod tests;
