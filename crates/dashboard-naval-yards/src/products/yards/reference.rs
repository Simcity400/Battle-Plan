//! A verbatim copy of the yard views' whole observation before their changes were followed.
use super::*;
use crate::products::{naval_refits, naval_repairs};
use dashboard_base::{prepared::prepass::Family, products::finite_fixed64};
use dashboard_keyed::Observer;
use replay::container::TableFields;
use replay_data::rows::NavalRefitShipRow;

struct Projection {
    tokens: Vec<Operand>,
    owners: Vec<String>,
    repairs: Vec<(String, i128)>,
    refits: Vec<(String, i128)>,
}

fn repairs_project(
    state: &mut State,
    frame: &DecodedFrame,
    countries: &Countries,
    owners: &[String],
    values: &mut Values,
) {
    if let Ok(rows) = frame.complete::<tables::NavalRepairCountries>() {
        for row in rows {
            if let Some(owner) = countries.raw(row.country) {
                for (field, amount) in [
                    ("usedDockyards", row.num_used_dockyards),
                    ("repairLimit", row.max_allowed_dockyards_to_repair),
                    ("buildingRepairLimit", row.max_factories_for_repair),
                ] {
                    values.push(format!("{owner}|total|{field}"), i128::from(amount));
                }
            }
        }
    }
    if let Ok(rows) = frame.complete::<tables::CountryFactories>() {
        for row in rows {
            if let Some(owner) = countries.raw(row.country) {
                values.push(
                    format!("{owner}|total|dockyardsRaw"),
                    i128::from(row.naval_total_raw),
                );
                values.push(
                    format!("{owner}|total|dockyards"),
                    i128::from(row.naval_total_raw / 100_000),
                );
            }
        }
    }
    let Ok(bases) = frame.complete::<tables::NavalBases>() else {
        for owner in owners {
            state.refuse(values, owner, "Repair port ownership not recorded", 1);
        }
        return;
    };
    let ownership_complete = bases
        .iter()
        .all(|base| countries.raw(base.country).is_some());
    let repair_bases = frame.complete::<tables::NavalRepairBases>().ok();
    let queues = frame.complete::<tables::NavalRepairQueue>().ok();
    let mut totals = BTreeMap::<String, (usize, BTreeSet<u64>)>::new();
    for base in bases {
        let Some(owner) = countries.raw(base.country) else {
            continue;
        };
        let prefix = format!("{owner}|{}", base.province);
        let port = find(repair_bases, base.province, |row| row.province);
        let entries = queues.map(|rows| {
            let first = rows.partition_point(|row| row.province < base.province);
            let count = rows[first..].partition_point(|row| row.province == base.province);
            &rows[first..first + count]
        });
        if port.is_none() && entries.is_none_or(|rows| rows.is_empty()) {
            continue;
        }
        values.push(format!("{prefix}|present"), 1);
        if let Some(province) = find(
            frame.complete::<tables::Provinces>().ok(),
            base.province,
            |row| row.id,
        ) {
            values.push(format!("{prefix}|state"), i128::from(province.state));
        }
        state.label(
            values,
            format!("{prefix}|name"),
            Operand::Port(port_operand(frame, countries, base.province)),
        );
        if let Some(port) = port {
            values.push(format!("{prefix}|priority"), i128::from(port.priority));
            values.push(
                format!("{prefix}|dockyards"),
                i128::from(port.allocated_dockyards),
            );
            state.label(
                values,
                format!("{prefix}|policy"),
                Operand::Policy {
                    port: true,
                    value: port.priority,
                },
            );
        }
        if let Some(entries) = entries {
            values.push(format!("{prefix}|queueCount"), entries.len() as i128);
            let mut unique = BTreeSet::new();
            for entry in entries {
                values.push(
                    format!("{prefix}|queue|{}", entry.ordinal),
                    i128::from(entry.ship),
                );
                let total = totals.entry(owner.clone()).or_default();
                total.0 += 1;
                total.1.insert(entry.ship);
                if unique.insert(entry.ship) {
                    let ship_prefix = format!("{prefix}|ship|{}", entry.ship);
                    values.push(format!("{ship_prefix}|present"), 1);
                    state.ship(frame, values, &owner, &ship_prefix, entry.ship);
                }
            }
        }
    }
    for owner in owners {
        values.push(
            format!("{owner}|total|queueCensusComplete"),
            i128::from(ownership_complete && queues.is_some()),
        );
    }
    if queues.is_some() && ownership_complete {
        for owner in owners {
            let total = totals.get(owner);
            values.push(
                format!("{owner}|total|queueEntries"),
                total.map_or(0, |total| total.0) as i128,
            );
            values.push(
                format!("{owner}|total|distinctQueuedReferences"),
                total.map_or(0, |total| total.1.len()) as i128,
            );
            if let Ok(ships) = frame.complete::<tables::Ships>()
                && total.is_none_or(|total| {
                    total
                        .1
                        .iter()
                        .all(|ship| find(Some(ships), *ship, |row| row.id).is_some())
                })
            {
                values.push(
                    format!("{owner}|total|distinctQueuedShips"),
                    total.map_or(0, |total| total.1.len()) as i128,
                );
            }
            if let Some((count, _)) = total {
                state.refuse(
                    values,
                    owner,
                    "Repair rate and ETA unavailable: the exact repair rate is not recorded",
                    *count,
                );
            }
        }
    } else {
        for owner in owners {
            state.refuse(
                values,
                owner,
                "Repair queue or port ownership not recorded; empty queue cannot be established",
                1,
            );
        }
    }
}

fn refits_project(
    state: &mut State,
    frame: &DecodedFrame,
    countries: &Countries,
    owners: &[String],
    values: &mut Values,
) {
    let progress = frame.complete::<tables::NavalRefitProgress>().ok();
    let production = frame.complete::<tables::ProductionLines>().ok();
    let ships = frame.complete::<tables::Ships>().ok();
    let tasks = frame.complete::<tables::TaskForces>().ok();
    let links = frame.complete::<tables::NavalRefitShips>().ok();
    let mut lines = BTreeMap::<(String, u64), BTreeSet<u64>>::new();
    let mut metadata_seen = BTreeSet::new();
    let mut census_complete = progress.is_some_and(|rows| rows.is_empty())
        || (progress.is_some() && links.is_some() && ships.is_some() && tasks.is_some());
    if let Some(links) = links {
        for link in links.iter().filter(|link| link.line != 0) {
            let ship = find(ships, link.ship, |row| row.id);
            let owner = ship
                .and_then(|ship| find(tasks, ship.task_force, |task| task.id))
                .and_then(|task| countries.raw(task.roster_owner));
            let typed = dashboard_forces::products::naval_management::projection::refit(
                link, progress, production,
            );
            // A stale carried link is not an active or queued ship row.
            if typed.live == Some(false) {
                continue;
            }
            let Some(owner) = owner else {
                census_complete = false;
                continue;
            };
            let prefix = format!("{owner}|{}", link.line);
            if let Some(amount) = typed.progress {
                lines
                    .entry((owner.clone(), link.line))
                    .or_default()
                    .insert(link.ship);
                values.push(format!("{prefix}|live"), 1);
                values.push(
                    format!("{prefix}|progress"),
                    i128::from(amount.progress_raw),
                );
                let ship_prefix = format!("{prefix}|ship|{}", link.ship);
                values.push(format!("{ship_prefix}|present"), 1);
                state.ship(frame, values, &owner, &ship_prefix, link.ship);
            }
            // Metadata is retained even when the hourly liveness gate is unknown.
            // Shared line metadata and its refusals occur once, not once per ship.
            if !metadata_seen.insert((owner.clone(), link.line)) {
                continue;
            }
            if let Some(line) = typed.metadata {
                values.push(format!("{prefix}|cost"), i128::from(line.cost_raw));
                values.push(
                    format!("{prefix}|dockyards"),
                    i128::from(line.active_factories),
                );
                state.design_value(frame, values, &owner, &prefix, "target", line.variant);
                if let Some(amount) = typed.progress {
                    if line.cost_raw > 0
                        && finite_fixed64(line.cost_raw)
                        && finite_fixed64(amount.progress_raw)
                    {
                        values.push(
                            format!("{prefix}|remainingCostRaw"),
                            i128::from(line.cost_raw) - i128::from(amount.progress_raw),
                        );
                    } else {
                        state.refuse(values,&owner,"Refit ratio and remaining cost unavailable: nonpositive or nonfinite last recorded cost or progress",1);
                    }
                }
            } else {
                state.refuse(
                    values,
                    &owner,
                    "Refit daily target, cost and allocation not recorded",
                    1,
                );
            }
        }
    }
    // Every line contributes once, irrespective of its ship multiplicity.
    let mut totals = BTreeMap::<String, (usize, usize, Option<i128>, Option<i128>)>::new();
    for ((owner, line), ships) in &lines {
        let total = totals
            .entry(owner.clone())
            .or_insert((0, 0, Some(0), Some(0)));
        total.0 += ships.len();
        total.1 += 1;
        let link = NavalRefitShipRow {
            ship: 0,
            line: *line,
        };
        let typed = dashboard_forces::products::naval_management::projection::refit(
            &link, progress, production,
        );
        total.2 = total
            .2
            .zip(typed.metadata)
            .map(|(sum, line)| sum + i128::from(line.active_factories));
        total.3 =
            total
                .3
                .zip(typed.metadata.zip(typed.progress))
                .and_then(|(sum, (line, amount))| {
                    (line.cost_raw > 0
                        && finite_fixed64(line.cost_raw)
                        && finite_fixed64(amount.progress_raw))
                    .then_some(sum + i128::from(line.cost_raw) - i128::from(amount.progress_raw))
                });
    }
    for owner in owners {
        values.push(
            format!("{owner}|total|censusComplete"),
            i128::from(census_complete),
        );
        if census_complete {
            let total = totals
                .get(owner)
                .copied()
                .unwrap_or((0, 0, Some(0), Some(0)));
            values.push(format!("{owner}|total|ships"), total.0 as i128);
            values.push(format!("{owner}|total|lines"), total.1 as i128);
            if let Some(dockyards) = total.2 {
                values.push(format!("{owner}|total|dockyards"), dockyards);
            }
            if let Some(remaining) = total.3 {
                values.push(format!("{owner}|total|remainingCostRaw"), remaining);
            }
        } else {
            state.refuse(values,owner,"Active refit count unavailable: incomplete hourly progress, links, ships or ownership census",1);
        }
    }
}

fn known_and_read(
    state: &mut State,
    format: FormatVersion,
    frame: &DecodedFrame,
    sources: &[TableId],
    owners: &[String],
    values: &mut Values,
) {
    state.refusal_counts.clear();
    // The persistent design cache is never read: its key is not rebuilt.
    if !state
        .management
        .continues(state.last_hour, frame.branch, frame.hour)
    {
        state.dates.clear();
    }
    state.last_hour = Some(frame.hour);
    for &table in sources {
        let known = matches!(&*frame.tables[table.index()], replay_frame::frame::TableView::Complete(rows) if rows.table() == table);
        if !known {
            state.dates.remove(&table.index());
        } else if dashboard_text::metadata::slot_read(format, &frame.metrics, table, frame.hour)
        {
            state.dates.insert(table.index(), frame.hour);
        } else if let Some(hour) = frame.tables[table.index()].sampled_at() {
            // A same-cadence sibling can witness an equal read later than
            // the table's own carried sample stamp.
            state.dates
                .entry(table.index())
                .and_modify(|read| *read = (*read).max(hour))
                .or_insert(hour);
        }
        for owner in owners {
            values.push(format!("{owner}|known|{table:?}"), i128::from(known));
            if !known {
                let reason = dashboard_base::products::table_missing_reason(frame, table)
                    .unwrap_or_else(|| "Table not recorded".into());
                state.refuse(values, owner, format!("{table:?}: {reason}"), 1);
            }
            if known
                && !matches!(
                    replay_data::cadence::cadence_in(table, format),
                    replay_data::cadence::Cadence::Hourly
                )
                && let Some(&read) = state.dates.get(&table.index())
            {
                values.push(format!("{owner}|read|{table:?}"), i128::from(read));
            }
        }
    }
}

fn project(frame: &DecodedFrame) -> Projection {
    dashboard_base::prepared::prepass::note("naval-yards.project");
    let countries = Countries::read(frame).expect("yard identity tables checked");
    let owners = owners(frame, &countries);
    let mut state = State::default();
    let mut repairs = Values::default();
    let mut refits = Values::default();
    repairs_project(&mut state, frame, &countries, &owners, &mut repairs);
    state.refusal_counts.clear();
    refits_project(&mut state, frame, &countries, &owners, &mut refits);
    Projection {
        tokens: state.tokens,
        owners,
        repairs: repairs
            .iter()
            .map(|(name, value)| (name.to_owned(), value))
            .collect::<BTreeMap<_, _>>()
            .into_iter()
            .collect(),
        refits: refits
            .iter()
            .map(|(name, value)| (name.to_owned(), value))
            .collect::<BTreeMap<_, _>>()
            .into_iter()
            .collect(),
    }
}


fn observe(
    state: &mut State,
    format: FormatVersion,
    frame: &DecodedFrame,
    sources: &[TableId],
    refits: bool,
    values: &mut Values,
) -> io::Result<bool> {
    if frame.complete::<tables::Countries>().is_err()
        || frame.complete::<tables::CountryLinks>().is_err()
    {
        state.dates.clear();
        state.last_hour = None;
        return Ok(false);
    }
    let projection = project(frame);
    known_and_read(state, format, frame, sources, &projection.owners, values);
    let mut tokens = HashMap::new();
    for (name, amount) in if refits {
        &projection.refits
    } else {
        &projection.repairs
    } {
        if let Some((owner, id)) = name.split_once("|refusal|") {
            let id = id.parse::<usize>().map_err(io::Error::other)?;
            let mapped = *tokens
                .entry(id)
                .or_insert_with(|| state.intern(projection.tokens[id].clone()));
            state.refusal(
                values,
                owner,
                mapped,
                usize::try_from(*amount).map_err(io::Error::other)?,
            );
        } else if is_label(name.rsplit('|').next().unwrap_or_default()) {
            let id = usize::try_from(*amount).map_err(io::Error::other)?;
            let mapped = *tokens
                .entry(id)
                .or_insert_with(|| state.intern(projection.tokens[id].clone()));
            values.push(name.clone(), mapped as i128);
        } else {
            values.push(name.clone(), *amount);
        }
    }
    Ok(true)
}

macro_rules! reference {
    ($name:ident, $observer:ty, $family:ident, $refits:expr) => {
        /// The family observed whole every hour as before its changes were followed.
        #[derive(Default, Serialize, Deserialize)]
        pub(crate) struct $name {
            pub state: State,
        }
        impl Observer for $name {
            const NAME: &'static str = <$observer as Observer>::NAME;
            const TABLES: &'static [TableId] = management::TABLES;
            const READ: TableId = <$observer as Observer>::READ;
            fn family() -> &'static Family {
                &$family::FAMILY
            }
            fn fields() -> io::Result<Vec<TableFields>> {
                super::fields(management::TABLES)
            }
            fn changed(changes: &dashboard_base::prepared::frame_changes::FrameChanges) -> bool {
                management::changed(changes)
            }
            fn bind(
                &mut self,
                source: &dashboard_base::Source,
                _: Option<&dashboard_core::config::ContentOptions>,
                check: &mut dyn FnMut() -> io::Result<()>,
            ) -> io::Result<()> {
                check()?;
                self.state.bind(source);
                Ok(())
            }
            fn observe(
                &mut self,
                format: FormatVersion,
                frame: &DecodedFrame,
                values: &mut Values,
            ) -> io::Result<bool> {
                observe(&mut self.state, format, frame, $family::SOURCES, $refits, values)
            }
        }
    };
}
reference!(Repairs, naval_repairs::RepairsObserver, naval_repairs, false);
reference!(Refits, naval_refits::RefitsObserver, naval_refits, true);
