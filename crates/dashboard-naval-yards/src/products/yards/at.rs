//! A yard view at an hour (`naval-repairs`, `naval-refits`): one owner's ports or refit lines,
//! their ships and queues, its totals and the refusals behind them, read from the view's
//! publication ([`super::read`]) as its streams stood then. Every value is the observer's, as a
//! decimal string, and absent where it read none: never zero for missing.

use super::{
    YardChoice,
    layout::{self, line, port, refit_totals, refusal, repair_totals, ship},
    naming::{self, Operand},
    read::{self, Rows, Value},
};
use dashboard_base::catalogue::Catalogue;
use dashboard_series::{Knownness, PageRead, Snapshot, family::Directory};
use replay_data::rows::TableId;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::Arc,
};
use ts_rs::TS;

#[cfg(test)]
mod tests;

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(rename = "NavalYardsAtParams")]
pub struct YardsAtParams {
    pub branch_id: String,
    pub tag: String,
    pub hour: i32,
}

impl YardsAtParams {
    pub fn validate(&self) -> Result<(), &'static str> {
        self.branch_id
            .parse::<u64>()
            .map_err(|_| "Invalid naval yards branch")?;
        dashboard_core::protocol::validate_tag(&self.tag)
    }
}

/// A ship at the hour: its full handle and the measures the observer read of it.
#[derive(Clone, Debug, Serialize, TS)]
#[ts(rename = "NavalYardShip")]
pub struct YardShip {
    pub id: String,
    pub measures: BTreeMap<String, String>,
}

/// One entry of a port's repair queue: its place and its ship's full handle.
#[derive(Clone, Debug, Serialize, TS)]
#[ts(rename = "NavalYardQueued")]
pub struct YardQueued {
    pub ordinal: String,
    pub ship: String,
}

/// A repair port (by province) or refit line (by handle) at the hour, with its ships and queue.
#[derive(Clone, Debug, Serialize, TS)]
#[ts(rename = "NavalYardHolder")]
pub struct YardHolder {
    pub id: String,
    pub measures: BTreeMap<String, String>,
    pub ships: Vec<YardShip>,
    pub queue: Vec<YardQueued>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "NavalYardsAtReply")]
pub struct YardsAtReply {
    pub hour: i32,
    /// Whether the hour's countries and their links were recorded; nothing is known where not.
    pub known: bool,
    /// The dating table's last read by the hour.
    pub read_hour: Option<i32>,
    pub holders: Vec<YardHolder>,
    pub totals: BTreeMap<String, String>,
    /// Each source table's dated supplying read at the hour, by table, where it is dated.
    pub reads: BTreeMap<String, i32>,
    /// The tokens the hour's rows name, by number.
    pub tokens: BTreeMap<String, YardChoice>,
    pub warnings: Vec<String>,
}

/// The measures a row's cells hold, by cell, and which of them are tokens.
pub(crate) fn measures(stream: u64, refits: bool) -> &'static [(usize, &'static str)] {
    match stream {
        layout::HOLDERS if refits => &[
            (line::LIVE, "live"),
            (line::PROGRESS, "progress"),
            (line::COST, "cost"),
            (line::DOCKYARDS, "dockyards"),
            (line::TARGET, "target"),
            (line::TARGET_NAME, "targetName"),
            (line::REMAINING, "remainingCostRaw"),
        ],
        layout::HOLDERS => &[
            (port::STATE, "state"),
            (port::NAME, "name"),
            (port::PRIORITY, "priority"),
            (port::DOCKYARDS, "dockyards"),
            (port::POLICY, "policy"),
            (port::QUEUE_COUNT, "queueCount"),
        ],
        layout::SHIPS => &[
            (ship::HULL, "hull"),
            (ship::MAXIMUM, "maximum"),
            (ship::DESIGN, "design"),
            (ship::DESIGN_NAME, "designName"),
            (ship::CLASS, "class"),
            (ship::TASK, "task"),
            (ship::TASK_NAME, "taskName"),
            (ship::REPAIR_MODE, "repairMode"),
            (ship::POLICY, "policy"),
            (ship::REPAIR_SPLIT, "repairSplit"),
            (ship::REPAIR_TARGET, "repairTarget"),
            (ship::REJOIN, "rejoin"),
            (ship::REJOIN_RESOLVES, "rejoinResolves"),
            (ship::REFIT_INTENT, "refitIntent"),
            (ship::REFIT_INTENT_NAME, "refitIntentName"),
            (ship::AFTER_REPAIR_INTENT, "afterRepairIntent"),
            (ship::AFTER_REPAIR_INTENT_NAME, "afterRepairIntentName"),
            (ship::NAME, "name"),
            (ship::NAME_REBUILT, "nameRebuilt"),
        ],
        layout::TOTALS if refits => &[
            (refit_totals::CENSUS_COMPLETE, "censusComplete"),
            (refit_totals::SHIPS, "ships"),
            (refit_totals::LINES, "lines"),
            (refit_totals::DOCKYARDS, "dockyards"),
            (refit_totals::REMAINING, "remainingCostRaw"),
        ],
        layout::TOTALS => &[
            (repair_totals::USED_DOCKYARDS, "usedDockyards"),
            (repair_totals::REPAIR_LIMIT, "repairLimit"),
            (repair_totals::BUILDING_REPAIR_LIMIT, "buildingRepairLimit"),
            (repair_totals::DOCKYARDS_RAW, "dockyardsRaw"),
            (repair_totals::DOCKYARDS, "dockyards"),
            (repair_totals::QUEUE_CENSUS_COMPLETE, "queueCensusComplete"),
            (repair_totals::QUEUE_ENTRIES, "queueEntries"),
            (repair_totals::DISTINCT_QUEUED_REFERENCES, "distinctQueuedReferences"),
            (repair_totals::DISTINCT_QUEUED_SHIPS, "distinctQueuedShips"),
        ],
        _ => &[],
    }
}

/// The cells of `stream`'s rows that name tokens.
pub fn labels(stream: u64, refits: bool) -> &'static [usize] {
    match stream {
        layout::HOLDERS if refits => &[line::TARGET_NAME],
        layout::HOLDERS => &[port::NAME, port::POLICY],
        layout::SHIPS => &[
            ship::DESIGN_NAME,
            ship::CLASS,
            ship::TASK_NAME,
            ship::POLICY,
            ship::REFIT_INTENT_NAME,
            ship::AFTER_REPAIR_INTENT_NAME,
            ship::NAME,
            ship::NAME_REBUILT,
        ],
        _ => &[],
    }
}

/// The width of `stream`'s rows, their key and whether they are held included.
pub fn width(stream: u64, refits: bool) -> usize {
    match stream {
        layout::HOLDERS if refits => line::CELLS,
        layout::HOLDERS => port::CELLS,
        layout::SHIPS => ship::CELLS,
        layout::TOTALS if refits => refit_totals::CELLS,
        layout::TOTALS => repair_totals::CELLS,
        _ => refusal::CELLS,
    }
}

/// A row's cell (its own index, as `layout` numbers it) among the cells after key and held.
fn cell(row: &[Value], at: usize) -> Option<&Value> {
    row.get(at.checked_sub(2)?)
}

fn named(row: &[Value], stream: u64, refits: bool) -> BTreeMap<String, String> {
    measures(stream, refits)
        .iter()
        .filter_map(|&(at, name)| {
            cell(row, at)
                .and_then(Value::number)
                .map(|value| (name.to_owned(), value.to_string()))
        })
        .collect()
}

/// Every stream of the owners named `tag` at `hour`, by owner: an owner is `TAG@slot`, and a
/// tag's holders over a branch are its owners.
pub fn owners_at<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    tag: &str,
    refits: bool,
    hour: i32,
) -> io::Result<Vec<(String, [Rows; 4])>> {
    let prefix = format!("{tag}@");
    let mut owners = Vec::new();
    for (actor, entry) in directory.actors.iter().enumerate() {
        let Ok(name) = std::str::from_utf8(&entry.name) else {
            continue;
        };
        if !name.starts_with(&prefix) || entry.first_hour > hour {
            continue;
        }
        let actor = actor as u64;
        let mut streams: [Rows; 4] = Default::default();
        let mut any = false;
        for stream in [layout::HOLDERS, layout::SHIPS, layout::TOTALS, layout::REFUSALS] {
            if let Some((_, rows)) = read::rows_at(
                snapshot,
                directory.key(actor, stream, None),
                width(stream, refits),
                hour,
            )? {
                any |= !rows.is_empty();
                streams[stream as usize] = rows;
            }
        }
        if any {
            owners.push((name.to_owned(), streams));
        }
    }
    Ok(owners)
}

/// The reply to `params` from the view's publication; none past what it has accepted.
pub fn query<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    refits: bool,
    params: &YardsAtParams,
    catalogue: Result<Option<&Arc<Catalogue>>, &str>,
) -> io::Result<Option<YardsAtReply>> {
    let hour = params.hour;
    if hour > snapshot.publication().accepted.last {
        return Ok(None);
    }
    let mut reply = YardsAtReply {
        hour,
        known: false,
        read_hour: None,
        holders: Vec::new(),
        totals: BTreeMap::new(),
        reads: BTreeMap::new(),
        tokens: BTreeMap::new(),
        warnings: Vec::new(),
    };
    let observed = read::evidence_at(
        snapshot,
        directory.key(layout::FAMILY, layout::OBSERVED, None),
        hour,
    )?;
    reply.known = observed.is_some_and(|evidence| evidence.known == Knownness::Known);
    if !reply.known {
        content(&mut reply.warnings, catalogue);
        return Ok(Some(reply));
    }
    reply.read_hour = read::last_read(snapshot, directory, hour)?;
    let mut tokens = BTreeSet::new();
    let mut refusals: BTreeMap<u64, u128> = BTreeMap::new();
    let mut holders: BTreeMap<u64, YardHolder> = BTreeMap::new();
    for (_, [holder_rows, ship_rows, total_rows, refusal_rows]) in
        owners_at(snapshot, directory, &params.tag, refits, hour)?
    {
        // The ports each of the owner's ships is queued at: its row names the lowest alone.
        let mut queued: BTreeMap<u64, BTreeSet<u64>> = BTreeMap::new();
        for (&key, row) in &holder_rows {
            for &at in labels(layout::HOLDERS, refits) {
                tokens.extend(cell(row, at).and_then(Value::number).and_then(|n| u64::try_from(n).ok()));
            }
            let mut measures = named(row, layout::HOLDERS, refits);
            measures.insert("present".into(), "1".into());
            let queue = match cell(row, port::QUEUE).filter(|_| !refits) {
                Some(Value::List(entries)) if entries.len() % 2 == 0 => entries
                    .chunks_exact(2)
                    .map(|entry| {
                        Ok(YardQueued {
                            ordinal: entry[0].number().ok_or_else(read::malformed)?.to_string(),
                            ship: entry[1].number().ok_or_else(read::malformed)?.to_string(),
                        })
                    })
                    .collect::<io::Result<Vec<_>>>()?,
                Some(Value::List(_)) => return Err(read::malformed()),
                _ => Vec::new(),
            };
            for entry in &queue {
                let ship = entry.ship.parse().map_err(|_| read::malformed())?;
                queued.entry(ship).or_default().insert(key);
            }
            holders.insert(
                key,
                YardHolder {
                    id: key.to_string(),
                    measures,
                    ships: Vec::new(),
                    queue,
                },
            );
        }
        for (&key, row) in &ship_rows {
            for &at in labels(layout::SHIPS, refits) {
                tokens.extend(cell(row, at).and_then(Value::number).and_then(|n| u64::try_from(n).ok()));
            }
            let Some(holder) = cell(row, ship::HOLDER)
                .and_then(Value::number)
                .and_then(|holder| u64::try_from(holder).ok())
            else {
                continue;
            };
            let mut measures = named(row, layout::SHIPS, refits);
            measures.insert("present".into(), "1".into());
            let ship = YardShip {
                id: key.to_string(),
                measures,
            };
            let mut ports = queued.remove(&key).unwrap_or_default();
            ports.insert(holder);
            for port in ports {
                if let Some(entry) = holders.get_mut(&port) {
                    entry.ships.push(ship.clone());
                }
            }
        }
        if let Some(row) = total_rows.get(&0) {
            reply.totals.extend(named(row, layout::TOTALS, refits));
        }
        for (&token, row) in &refusal_rows {
            if let Some(count) = cell(row, refusal::COUNT)
                .and_then(Value::number)
                .and_then(|count| u128::try_from(count).ok())
            {
                *refusals.entry(token).or_default() += count;
            }
        }
    }
    reply.holders = holders.into_values().collect();
    // Each source table's dated read, or why it is unknown.
    let mut reasons = Vec::new();
    for &(table, key) in directory.dimensions(layout::FAMILY, layout::SOURCE) {
        let Some(evidence) = read::evidence_at(snapshot, Some(key), hour)? else {
            continue;
        };
        match evidence.known {
            Knownness::Known => {
                if let (Some(read), Some(table)) = (
                    evidence.supplying_read,
                    usize::try_from(table)
                        .ok()
                        .and_then(|table| TableId::ALL.get(table).copied()),
                ) {
                    reply.reads.insert(format!("{table:?}"), read);
                }
            }
            Knownness::Unknown(token) => reasons.push(token),
        }
    }
    let operand = |snapshot: &mut Snapshot<'_, P>, token: u64| -> io::Result<Option<Operand>> {
        read::token(snapshot, directory, token)?
            .map(|text| serde_json::from_str(&text).map_err(|_| read::malformed()))
            .transpose()
    };
    let mut unnamed: BTreeMap<String, u64> = BTreeMap::new();
    let mut notes = BTreeSet::new();
    let loaded = catalogue.ok().flatten();
    for token in tokens {
        let Some(found) = operand(snapshot, token)? else {
            continue;
        };
        if let Some(loaded) = loaded {
            notes.extend(naming::notes(&found, loaded));
        }
        match naming::resolve(&found, loaded) {
            Ok(choice) => {
                reply.tokens.insert(token.to_string(), choice);
            }
            Err(why) => *unnamed.entry(why).or_default() += 1,
        }
    }
    let what = if refits { "refit fields" } else { "naval repair fields" };
    for (token, count) in refusals {
        if let Some(Operand::Refusal(why)) = operand(snapshot, token)? {
            reply.warnings.push(format!("{count} {what} refused: {why}"));
        }
    }
    for token in reasons {
        if let Some(Operand::Refusal(why)) = operand(snapshot, token)? {
            reply.warnings.push(why);
        }
    }
    for (why, count) in unnamed {
        reply.warnings.push(format!("{count} naval yard names refused: {why}"));
    }
    reply.warnings.extend(notes);
    content(&mut reply.warnings, catalogue);
    Ok(Some(reply))
}

/// The catalogue's own diagnostics, at every hour: why it did not load, or its verification,
/// warnings and refusals.
fn content(warnings: &mut Vec<String>, catalogue: Result<Option<&Arc<Catalogue>>, &str>) {
    match catalogue {
        Err(why) => warnings.push(format!("Naval yard content names unavailable: {why}")),
        Ok(Some(catalogue)) => {
            warnings.extend(
                catalogue
                    .verification
                    .iter()
                    .chain(&catalogue.warnings)
                    .cloned(),
            );
            warnings.extend(catalogue.refusal_lines());
        }
        Ok(None) => {}
    }
}
