//! A country's production lines running at an hour, as the game's production tab lists them
//! (`production-lines-at`): a rows-at-an-hour product, asked for an hour and followed as a panel
//! at the head. The reply is the lines in the asked order with their names and art; the numbers
//! that move as the lines run (factories, efficiency, speed, cost, units made) are live cells, so
//! the panel is pushed only when which lines run, their order or their names change.
//!
//! A request reads the owner's lines by key: the run's index of line lives ([`super::RunLines`])
//! gives the handles whose producing country links to the owner at the hour, so no other
//! country's lines or output rows are built.

use super::{
    DECIMALS, LineKind, OrganisationName, VariantName,
    names::{self, Named},
};
use crate::{
    Source,
    answer::ProductionIndex,
    products::{
        LoadContext, check_reply_budget,
        at_hour::{At, AtHour, Keys, Owner, Read, ReadsAt},
    },
};
use dashboard_follow::live::Source as Cell;
use dashboard_text::metadata::link_slot;
use replay_data::rows::{ProductionLineRow, TableId, tables};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, io};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub struct ProductionLinesAt;

/// The order lines are listed in: the game's queue, most factories assigned, or most IC a day.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum LineOrder {
    #[default]
    Game,
    Factories,
    Output,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProductionLinesAtParams {
    pub branch_id: String,
    pub tag: String,
    /// The game's queue order when absent.
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub order: Option<LineOrder>,
}

/// One output row of a line at the hour; its units made are in the line's `output` cell.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct OutputPartNow {
    pub variant: String,
    /// The organisation's reference; "0" for none.
    pub mio: String,
    /// False when a tick could not be counted: the line's total is then not known.
    pub exact: bool,
}

/// A line's output rows at the hour.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct OutputNow {
    pub parts: Vec<OutputPartNow>,
    pub since_hour: i32,
    pub whole_life: bool,
}

/// One line running at the hour. Its live cells, keyed by `handle`: `active` and `requested`
/// factories, `efficiency` (Q15, 100% is 3276800), `speed` and `cost` (fixed5 IC; units a week are
/// `speed × 700000 ÷ cost`, truncated) and `output` (units made since `output.sinceHour`, the sum
/// of its rows).
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LineNow {
    pub handle: String,
    pub kind: Option<LineKind>,
    pub variant: String,
    pub mio: String,
    pub max_factories: i32,
    pub priority: i32,
    /// Its output rows; null where they are not recorded at the hour.
    pub output: Option<OutputNow>,
}

#[derive(Clone, Debug, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ProductionLinesAtReply {
    pub branch_id: String,
    pub tag: String,
    pub decimals: u8,
    /// The country's lines in the asked order; empty with `missing` where they are not known.
    pub lines: Vec<LineNow>,
    /// Why the lines are not known at the hour, when they are not: never shown as none.
    pub missing: Option<String>,
    /// Why the lines' output is not known at the hour, when it is not.
    pub output_missing: Option<String>,
    pub variants: Vec<VariantName>,
    pub organisations: Vec<OrganisationName>,
    pub icons: Vec<super::EquipmentIcon>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(ProductionLinesAtReply);

/// What a reply reads beyond its frame: the recorded content, and the source's production index.
pub struct LinesContext {
    naming: names::Context,
    index: ProductionIndex,
}

/// The owner's lines in `held`, in the game's queue order.
fn owned<'a>(held: &'a DecodedFrame, owner: Owner) -> io::Result<Vec<&'a ProductionLineRow>> {
    let links = held.complete::<tables::CountryLinks>()?;
    let mut lines: Vec<_> = held.complete::<tables::ProductionLines>()?.iter().filter(|line| link_slot(links, line.country) == Some(owner.slot)).collect();
    lines.sort_by_key(|line| (line.priority, line.handle));
    Ok(lines)
}

/// The owner's lines in `order`; ties keep the queue's.
fn ordered<'a>(mut lines: Vec<&'a ProductionLineRow>, order: LineOrder) -> Vec<&'a ProductionLineRow> {
    match order {
        LineOrder::Game => {}
        LineOrder::Factories => lines.sort_by(|a, b| b.active_factories.cmp(&a.active_factories)),
        LineOrder::Output => lines.sort_by(|a, b| b.speed_raw.cmp(&a.speed_raw)),
    }
    lines
}

/// The owner's lines' handles at `hour` of run `run` from its index, or none where the index or
/// the hour's links cannot be read (every line is then read).
fn indexed_handles(context: &LinesContext, run: u32, hour: i32, held: &DecodedFrame, owner: Owner) -> Option<Vec<u64>> {
    let links = held.complete::<tables::CountryLinks>().ok()?;
    let index = context.index.run(run).ok()?;
    let handles: BTreeSet<u64> = index.lines.iter().filter(|life| life.first <= hour && hour <= life.last && link_slot(links, life.country) == Some(owner.slot)).map(|life| life.handle).collect();
    Some(handles.into_iter().collect())
}

impl AtHour for ProductionLinesAt {
    const KEY: &'static str = "production-lines-at";
    const TITLE: &'static str = "Production lines";
    const CONTENT_DEPENDENT: bool = true;
    type Params = ProductionLinesAtParams;
    type Reply = ProductionLinesAtReply;
    type Context = LinesContext;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![ProductionLinesAtParams::decl(), ProductionLinesAtReply::decl(), LineOrder::decl(), OutputPartNow::decl(), OutputNow::decl(), LineNow::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        if params.branch_id.parse::<u64>().is_err() {
            return Err("Invalid branch");
        }
        crate::products::validate_tag(&params.tag)
    }

    fn owner(params: &Self::Params) -> Option<&str> {
        Some(&params.tag)
    }

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        let mut tables = vec![TableId::ProductionLines, TableId::ProductionLineOutput];
        tables.extend_from_slice(names::TABLES);
        tables
    }

    /// The countries whole; the owner's lines and their output rows (by key through the run's
    /// index where it is read, else whole); the definitions and organisations they name; the
    /// tokens and states naming asks of.
    fn reads_in(_params: &Self::Params, owner: Option<Owner>, step: usize, held: &DecodedFrame, at: &ReadsAt<'_, Self::Context>) -> Vec<Read> {
        let Some(owner) = owner else { return Vec::new() };
        match step {
            0 => names::WHOLE.iter().map(|&table| Read::all(table)).collect(),
            1 => match indexed_handles(at.context, at.run, at.hour, held, owner) {
                Some(handles) => vec![
                    Read::these(TableId::ProductionLines, handles.iter().map(|&handle| u128::from(handle))),
                    Read { table: TableId::ProductionLineOutput, keys: Keys::Ranges(handles.iter().map(|&handle| (u128::from(handle) << 32, u128::from(handle) << 32 | u128::from(u32::MAX))).collect()) },
                ],
                None => vec![Read::all(TableId::ProductionLines), Read::all(TableId::ProductionLineOutput)],
            },
            2 => {
                let lines = owned(held, owner).unwrap_or_default();
                let handles: BTreeSet<u64> = lines.iter().map(|line| line.handle).collect();
                let mut variants: BTreeSet<u64> = lines.iter().map(|line| line.variant).collect();
                let mut mios: BTreeSet<u64> = lines.iter().map(|line| line.mio).filter(|&mio| mio != 0).collect();
                for row in held.complete::<tables::ProductionLineOutput>().ok().into_iter().flatten().filter(|row| handles.contains(&row.line) && !row.ended) {
                    variants.insert(row.variant);
                    if row.mio != 0 {
                        mios.insert(row.mio);
                    }
                }
                vec![Read::these(TableId::VariantDefinitions, variants.into_iter().map(u128::from)), Read::these(TableId::IndustrialOrganisations, mios.into_iter().map(u128::from))]
            }
            3 => {
                let mut tokens: BTreeSet<u32> = names::variant_tokens(held).collect();
                tokens.extend(held.complete::<tables::IndustrialOrganisations>().ok().into_iter().flatten().map(|row| row.token));
                names::spelling_reads(held, tokens, true)
            }
            _ => Vec::new(),
        }
    }

    fn context(source: &Source, load: &LoadContext) -> io::Result<Self::Context> {
        Ok(LinesContext { naming: names::context(source, load)?, index: ProductionIndex::of(source) })
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let owner = at.owner.ok_or_else(|| dashboard_core::refused("Production lines country is unavailable at this hour"))?;
        let mut reply = ProductionLinesAtReply {
            branch_id: params.branch_id.clone(),
            tag: params.tag.clone(),
            decimals: DECIMALS,
            lines: Vec::new(),
            missing: None,
            output_missing: None,
            variants: Vec::new(),
            organisations: Vec::new(),
            icons: Vec::new(),
            warnings: Vec::new(),
        };
        let lines = match owned(frame, owner) {
            Ok(lines) => ordered(lines, params.order.unwrap_or_default()),
            Err(error) => {
                reply.missing = Some(format!("Production lines are not recorded at this hour: {error}"));
                return Ok(reply);
            }
        };
        let output = frame.complete::<tables::ProductionLineOutput>();
        if let Err(error) = &output {
            reply.output_missing = Some(format!("Units made are not recorded at this hour: {error}"));
        }
        let field = |table, name| dashboard_declare::field(table, name);
        let line_table = TableId::ProductionLines;
        let columns = (|| -> io::Result<_> {
            Ok((
                [field(line_table, "active_factories")?, field(line_table, "requested_factories")?, field(line_table, "efficiency_raw")?, field(line_table, "speed_raw")?, field(line_table, "cost_raw")?],
                field(TableId::ProductionLineOutput, "units")?,
            ))
        })();
        if let Err(error) = &columns {
            reply.warnings.push(format!("Line values cannot be read: {error}"));
        }
        let mut variants = BTreeSet::new();
        let mut mios = BTreeSet::new();
        for line in &lines {
            variants.insert(line.variant);
            if line.mio != 0 {
                mios.insert(line.mio);
            }
            let rows: Vec<_> = output.as_ref().map(|rows| rows.iter().filter(|row| row.line == line.handle && !row.ended).collect()).unwrap_or_default();
            for row in &rows {
                variants.insert(row.variant);
                if row.mio != 0 {
                    mios.insert(row.mio);
                }
            }
            let handle = line.handle.to_string();
            if let Ok((line_columns, units)) = &columns {
                for (name, &column) in ["active", "requested", "efficiency", "speed", "cost"].into_iter().zip(line_columns) {
                    at.live.value(&handle, name, Cell { table: line_table, column, key: u128::from(line.handle) }, 1.0);
                }
                if output.is_ok() && !rows.is_empty() {
                    let parts: Vec<Cell> = rows.iter().map(|row| Cell { table: TableId::ProductionLineOutput, column: *units, key: u128::from(row.line) << 32 | u128::from(row.ordinal) }).collect();
                    at.live.sum(&handle, "output", &parts, 1.0);
                }
            }
            reply.lines.push(LineNow {
                handle,
                kind: LineKind::of(i64::from(line.kind)),
                variant: line.variant.to_string(),
                mio: line.mio.to_string(),
                max_factories: line.max_factories,
                priority: line.priority,
                output: (output.is_ok() && !rows.is_empty()).then(|| OutputNow {
                    since_hour: rows.iter().map(|row| row.since_hour).min().unwrap_or(frame.hour),
                    whole_life: rows.iter().all(|row| row.observed_since_creation),
                    parts: rows.iter().map(|row| OutputPartNow { variant: row.variant.to_string(), mio: row.mio.to_string(), exact: row.exact }).collect(),
                }),
            });
        }
        let mut named = Named::default();
        let variants: Vec<u64> = variants.into_iter().collect();
        let mios: Vec<u64> = mios.into_iter().collect();
        names::name(frame, &variants, &mios, &at.context.naming, &mut named);
        reply.warnings.extend(std::mem::take(&mut named.warnings));
        reply.icons = named.icons();
        reply.variants = named.variants.into_values().map(|variant| VariantName { hour: None, ..variant }).collect();
        reply.organisations = named.organisations.into_values().map(|organisation| OrganisationName { hour: None, ..organisation }).collect();
        check_reply_budget(&reply)?;
        Ok(reply)
    }
}
