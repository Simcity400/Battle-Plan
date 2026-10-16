//! Native-date window replies from the one shared retained sinking observer.
use super::*;
use dashboard_base::prepared::series;
use dashboard_text::metadata::check_cancel;
use serde_json::Value;

/// Window-local dictionaries and immutable global scan indices travel together.
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ShipsSunkWindowReply {
    pub window: usize,
    pub indices: Vec<u32>,
    pub reply: ShipsSunkReply,
}
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ShipsSunkManifestReply {
    pub windows: Vec<HourSpan>,
    /// One first occurrence per class supplies choices without reading log pages.
    pub classes: ShipsSunkReply,
    pub content_warnings: Vec<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WindowParams {
    branch_id: String,
    hour: i32,
}
#[derive(Serialize, Deserialize)]
struct IndexedShip {
    index: u32,
    ship: sinkings::ShipEvent,
}
#[derive(Serialize, Deserialize)]
struct AttributionPatch {
    index: u32,
    sink_hour: i32,
    victim: Option<String>,
    attacker: Option<String>,
    victim_as_of: Option<i32>,
    attacker_as_of: Option<i32>,
}
#[derive(Default, Serialize, Deserialize)]
struct NameFacts {
    classes: BTreeMap<u32, (Vec<u8>, i32, i32)>,
    #[serde(with = "dashboard_core::map_pairs")]
    recorded: BTreeMap<Vec<u8>, u32>,
    provinces: BTreeSet<u32>,
    stored_refusals: Refusals,
}
impl NameFacts {
    fn observe(&mut self, ship: &sinkings::ShipEvent) {
        self.classes
            .entry(ship.entry.class_token)
            .or_insert_with(|| {
                (
                    ship.entry.class_name.clone(),
                    ship.entry.sink_hour,
                    ship.read_hour,
                )
            });
        let next = self.recorded.len() as u32;
        self.recorded
            .entry(ship.entry.class_name.clone())
            .or_insert(next);
        if ship.entry.province_present != 0 {
            self.provinces.insert(ship.entry.province);
        }
    }
    fn scan(&self, metadata: &sinkings::Scanned) -> sinkings::Scanned {
        let mut scan = metadata.metadata();
        let Some((&token, _)) = self.classes.first_key_value() else {
            return scan;
        };
        let event =
            |token, class_name, sink_hour, read_hour, province: Option<u32>| sinkings::ShipEvent {
                entry: replay::rows::SunkShipEntry {
                    class_token: token,
                    class_name,
                    sink_hour,
                    province: province.unwrap_or(0),
                    province_present: u8::from(province.is_some()),
                    ..Default::default()
                },
                read_hour,
                victim: None,
                attacker: None,
                victim_as_of: None,
                attacker_as_of: None,
            };
        let mut recorded: Vec<_> = self.recorded.iter().collect();
        recorded.sort_by_key(|(_, ordinal)| **ordinal);
        for (name, _) in recorded {
            scan.ships.push(event(
                token,
                name.clone(),
                metadata.first_hour,
                metadata.first_ship_read.unwrap_or(metadata.first_hour),
                None,
            ));
        }
        for (&token, (name, sink, read)) in &self.classes {
            scan.ships
                .push(event(token, name.clone(), *sink, *read, None));
        }
        for &province in &self.provinces {
            scan.ships.push(event(
                token,
                Vec::new(),
                metadata.first_hour,
                metadata.first_ship_read.unwrap_or(metadata.first_hour),
                Some(province),
            ));
        }
        scan
    }
}
#[derive(Default, Serialize, Deserialize)]
#[allow(clippy::type_complexity)]
pub struct Prepared {
    published: u32,
    open: BTreeMap<u32, (Option<String>, Option<String>, Option<i32>, Option<i32>)>,
    facts: NameFacts,
    events: series::Accumulator,
    windows: series::Accumulator,
    #[serde(skip)]
    pending: BTreeMap<u32, sinkings::ShipEvent>,
    #[serde(skip)]
    patches: BTreeMap<u32, AttributionPatch>,
}
fn window_at(windows: &[HourSpan], hour: i32) -> usize {
    windows.partition_point(|window| window.last_hour < hour)
}
fn attribution(
    ship: &sinkings::ShipEvent,
) -> (Option<String>, Option<String>, Option<i32>, Option<i32>) {
    (
        ship.victim.clone(),
        ship.attacker.clone(),
        ship.victim_as_of,
        ship.attacker_as_of,
    )
}
/// Dictionary relocation only; every row still comes from the product's reply builder.
fn merge_reply(
    mut prior: ShipsSunkWindowReply,
    added: ShipsSunkWindowReply,
) -> ShipsSunkWindowReply {
    let mut classes: BTreeMap<_, _> = prior
        .reply
        .classes
        .iter()
        .chain(&added.reply.classes)
        .map(|class| (class.token, class.clone()))
        .collect();
    let classes: Vec<_> = std::mem::take(&mut classes).into_values().collect();
    let mut recorded = prior.reply.recorded_names.clone();
    for name in &added.reply.recorded_names {
        if !recorded.contains(name) {
            recorded.push(name.clone());
        }
    }
    let places: Vec<_> = prior
        .reply
        .places
        .iter()
        .chain(&added.reply.places)
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let mut rows = BTreeMap::new();
    for page in [&prior, &added] {
        for (&index, row) in page.indices.iter().zip(&page.reply.rows) {
            let mut row = row.clone();
            row.class_index = classes
                .binary_search_by_key(
                    &page.reply.classes[row.class_index as usize].token,
                    |class| class.token,
                )
                .unwrap() as u32;
            row.recorded_name_index = row.recorded_name_index.map(|at| {
                recorded
                    .iter()
                    .position(|name| name == &page.reply.recorded_names[at as usize])
                    .unwrap() as u32
            });
            row.place_index = row.place_index.map(|at| {
                places
                    .binary_search(&page.reply.places[at as usize])
                    .unwrap() as u32
            });
            rows.insert(index, row);
        }
    }
    prior.indices = rows.keys().copied().collect();
    prior.reply.rows = rows.into_values().collect();
    prior.reply.classes = classes;
    prior.reply.recorded_names = recorded;
    prior.reply.places = places;
    prior.reply.last_hour = added.reply.last_hour;
    prior.reply.unread = added.reply.unread;
    prior.reply.warnings = added.reply.warnings;
    prior
}
impl Prepared {
    pub fn observe(
        &mut self,
        scan: &sinkings::Scanned,
        indices: &[u32],
        flag: &AtomicBool,
        cat: Result<&Catalogue, &str>,
        content: Option<&NameContent>,
        retention: Option<&sinkings::Retention>,
    ) -> io::Result<()> {
        self.observe_with(scan, indices, flag, |page| {
            prepared_names(scan, &page.ships, cat, content, retention, flag)
        })
    }
    fn observe_with(
        &mut self,
        scan: &sinkings::Scanned,
        indices: &[u32],
        flag: &AtomicBool,
        names: impl Fn(&sinkings::Scanned) -> io::Result<Named>,
    ) -> io::Result<()> {
        check_cancel(flag)?;
        let mut changed = sinkings::Scanned::default();
        for (&index, ship) in indices.iter().zip(&scan.ships) {
            let value = attribution(ship);
            let update = self
                .open
                .get(&index)
                .is_some_and(|previous| previous != &value);
            if index >= self.published {
                self.pending.insert(index, ship.clone());
                changed.ships.push(ship.clone());
                self.events.event(
                    0,
                    scan.last_hour,
                    &IndexedShip {
                        index,
                        ship: ship.clone(),
                    },
                )?;
                self.facts.observe(ship);
            } else if update {
                let patch = AttributionPatch {
                    index,
                    sink_hour: ship.entry.sink_hour,
                    victim: ship.victim.clone(),
                    attacker: ship.attacker.clone(),
                    victim_as_of: ship.victim_as_of,
                    attacker_as_of: ship.attacker_as_of,
                };
                self.events.event(1, scan.last_hour, &patch)?;
                self.patches.insert(index, patch);
            }
            if (ship.entry.victim_country != 0 && ship.victim.is_none())
                || (ship.entry.attacker_country != 0 && ship.attacker.is_none())
            {
                self.open.insert(index, value);
            } else {
                self.open.remove(&index);
            }
            self.published = self.published.max(index.saturating_add(1));
        }
        if !changed.ships.is_empty() {
            let names = names(&changed)?;
            self.facts.stored_refusals.merge(&names.stored_refusals);
        }
        self.events.through(scan.last_hour);
        Ok(())
    }
    #[allow(clippy::too_many_arguments)]
    pub fn publish(
        &mut self,
        source: &Source,
        writer: &mut store::Writer,
        branch: u64,
        ordinal: usize,
        scan: &sinkings::Scanned,
        flag: &AtomicBool,
        cat: Result<&Catalogue, &str>,
        content: Option<&NameContent>,
        retention: Option<&sinkings::Retention>,
    ) -> io::Result<()> {
        let windows: Vec<_> = source
            .block_windows(branch)
            .iter()
            .map(|window| HourSpan {
                first_hour: window.first_hour,
                last_hour: window.last_hour,
            })
            .collect();
        self.publish_with(writer, branch, ordinal, scan, &windows, flag, |page| {
            prepared_names(page, &page.ships, cat, content, retention, flag)
        })
    }
    #[allow(clippy::too_many_arguments)]
    fn publish_with(
        &mut self,
        writer: &mut store::Writer,
        branch: u64,
        ordinal: usize,
        scan: &sinkings::Scanned,
        windows: &[HourSpan],
        flag: &AtomicBool,
        names: impl Fn(&sinkings::Scanned) -> io::Result<Named>,
    ) -> io::Result<()> {
        let boundary = if ordinal == 0 {
            i32::MIN
        } else {
            windows[ordinal - 1].last_hour.saturating_add(1)
        };
        self.windows.event(0, boundary, &(ordinal as u64))?;
        self.windows.through(windows[ordinal].last_hour);
        let mut pages = BTreeMap::<usize, BTreeMap<u32, sinkings::ShipEvent>>::from([(
            ordinal,
            BTreeMap::new(),
        )]);
        for (index, ship) in std::mem::take(&mut self.pending) {
            pages
                .entry(window_at(windows, ship.entry.sink_hour))
                .or_default()
                .insert(index, ship);
        }
        let mut patches = BTreeMap::<usize, Vec<AttributionPatch>>::new();
        for patch in std::mem::take(&mut self.patches).into_values() {
            let at = window_at(windows, patch.sink_hour);
            pages.entry(at).or_default();
            patches.entry(at).or_default().push(patch);
        }
        for (at, rows) in pages {
            check_cancel(flag)?;
            let mut page = scan.metadata();
            page.ships = rows.values().cloned().collect();
            let resolved = names(&page)?;
            let added = ShipsSunkWindowReply {
                window: at,
                indices: rows.keys().copied().collect(),
                reply: reply(&branch.to_string(), &page, &resolved, flag)?,
            };
            let section = format!("ships-sunk-{branch}");
            // The writer sees both committed input and an earlier staged repair in this transaction.
            let mut value = if let Some(file) = writer.read_at(&section, at as u64)? {
                let prior = serde_json::from_reader(zstd::stream::Decoder::new(file)?)?;
                merge_reply(prior, added)
            } else {
                added
            };
            for patch in patches.remove(&at).unwrap_or_default() {
                let row = value
                    .indices
                    .binary_search(&patch.index)
                    .ok()
                    .and_then(|index| value.reply.rows.get_mut(index))
                    .ok_or_else(|| {
                        dashboard_core::invalid("Prepared sinking attribution row is missing")
                    })?;
                row.victim = patch.victim;
                row.attacker = patch.attacker;
                row.victim_as_of = patch.victim_as_of;
                row.attacker_as_of = patch.attacker_as_of;
            }
            series::write_reply(writer, &section, at as u64, &value)?;
        }
        let mut classes = self.facts.scan(scan);
        let mut resolved = names(&classes)?;
        resolved.add_stored_refusals(&self.facts.stored_refusals);
        // The catalogue's class samples retain the first recorded fallback per class.
        classes.ships = self
            .facts
            .classes
            .iter()
            .map(|(&token, (name, sink, read))| sinkings::ShipEvent {
                entry: replay::rows::SunkShipEntry {
                    class_token: token,
                    class_name: name.clone(),
                    sink_hour: *sink,
                    ..Default::default()
                },
                read_hour: *read,
                victim: None,
                attacker: None,
                victim_as_of: None,
                attacker_as_of: None,
            })
            .collect();
        series::write_reply(
            writer,
            &format!("ships-sunk-manifest-{branch}"),
            0,
            &ShipsSunkManifestReply {
                windows: windows.to_vec(),
                classes: reply(&branch.to_string(), &classes, &resolved, flag)?,
                content_warnings: resolved.warnings.clone(),
            },
        )?;
        self.events
            .flush(writer, &format!("sinking-events-{branch}"), 0, flag)?;
        self.windows
            .flush(writer, &format!("sinking-windows-{branch}"), 0, flag)
    }
}
pub fn types() -> Vec<String> {
    vec![ShipsSunkWindowReply::decl(), ShipsSunkManifestReply::decl()]
}
pub fn query(
    source: &Source,
    product: &str,
    params: Value,
    store: &store::Store,
    key: &store::SourceKey,
    flag: &AtomicBool,
) -> io::Result<Value> {
    check_cancel(flag)?;
    let id = params
        .get("branchId")
        .and_then(Value::as_str)
        .ok_or_else(|| dashboard_core::invalid("Invalid ships sunk branch"))?;
    let branch: u64 = id
        .parse()
        .map_err(|_| dashboard_core::invalid("Invalid ships sunk branch"))?;
    if !source
        .replay_ref()
        .branches
        .iter()
        .any(|start| start.id == branch)
    {
        return Err(dashboard_core::invalid(
            "Ships sunk branch is not in this recording",
        ));
    }
    let value = match product {
        "ships-sunk-manifest" => {
            let params: BranchParams = serde_json::from_value(params)?;
            ShipsSunk::validate(&params).map_err(dashboard_core::invalid)?;
            let reply: ShipsSunkManifestReply =
                series::read_reply(store, key, &format!("ships-sunk-manifest-{branch}"), 0)?
                    .ok_or_else(|| {
                        dashboard_core::invalid("Prepared ships sunk manifest is missing")
                    })?;
            serde_json::to_value(reply)?
        }
        "ships-sunk-battle-titles" => {
            let params = serde_json::from_value(params)?;
            serde_json::to_value(battle_titles::prepared_query(&params, store, key, flag)?)?
        }
        "ships-sunk" => {
            let params: WindowParams = serde_json::from_value(params)?;
            let mut windows =
                series::Reader::new(store, key, format!("sinking-windows-{branch}"), flag)?;
            let ordinal = windows
                .latest_event(0, params.hour)?
                .map(|(_, value)| serde_json::from_value::<u64>(value))
                .transpose()?
                .ok_or_else(|| {
                    dashboard_core::invalid("Prepared sinking window lookup is missing")
                })?;
            let mut reply: ShipsSunkWindowReply =
                series::read_reply(store, key, &format!("ships-sunk-{branch}"), ordinal)?
                    .ok_or_else(|| {
                        dashboard_core::invalid("Prepared ships sunk window is missing")
                    })?;
            let manifest: ShipsSunkManifestReply =
                series::read_reply(store, key, &format!("ships-sunk-manifest-{branch}"), 0)?
                    .ok_or_else(|| {
                        dashboard_core::invalid("Prepared sinking catalogue is missing")
                    })?;
            let classes: BTreeMap<_, _> = manifest
                .classes
                .classes
                .into_iter()
                .map(|class| (class.token, class))
                .collect();
            for class in &mut reply.reply.classes {
                if let Some(current) = classes.get(&class.token) {
                    *class = current.clone();
                }
            }
            reply.reply.branch_id = params.branch_id;
            serde_json::to_value(reply)?
        }
        _ => unreachable!(),
    };
    dashboard_base::products::check_reply_budget(&value)?;
    Ok(value)
}
#[cfg(test)]
mod tests;
