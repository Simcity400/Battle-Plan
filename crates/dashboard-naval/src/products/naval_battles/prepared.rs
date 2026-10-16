//! Changed native battle records and drawn-row details in shared prepared batches.
use super::details::{
    self, NavalBattleDetailsParams, NavalBattleDetailsReply, NavalBattleFleetNames,
    NavalBattleNamedShip,
};
use super::*;
use crate::products::{ships_sunk, sinkings};
use dashboard_base::{
    catalogue,
    prepared::{frame_changes::FrameChanges, series},
};
use serde_json::Value;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
struct FleetRead {
    province: u32,
    fleets: [NavalBattleFleetNames; 2],
    refused: catalogue::Refusals,
}
#[derive(Serialize, Deserialize)]
struct PreparedShip {
    #[serde(flatten)]
    row: NavalBattleNamedShip,
    class_token: u32,
}
#[derive(Default, Serialize, Deserialize)]
pub(super) struct DetailPreparation {
    ships: sinkings::ShipHistory,
    sinking_log: ships_sunk::prepared::Prepared,
    sinking_titles: ships_sunk::battle_titles::Prepared,
    losses: super::super::naval_losses::prepared::Prepared,
    fleets: BTreeMap<u64, FleetRead>,
    fleet_reads: series::Accumulator,
    ship_reads: series::Accumulator,
    census_reads: series::Accumulator,
    ship_count: u32,
    #[serde(skip)]
    pending_named: BTreeMap<u64, Vec<PreparedShip>>,
    ship_pages: BTreeMap<String, u64>,
    /// The content refusals this family's own reads produced, which its
    /// sinking pages, titles and details report.
    #[serde(default)]
    refusals: catalogue::OwnRefusals,
    #[serde(skip)]
    cat: Option<Arc<catalogue::Catalogue>>,
    #[serde(skip)]
    catalogue_error: Option<String>,
    #[serde(skip)]
    naming: Option<Arc<ships_sunk::NameContent>>,
    #[serde(skip)]
    retention: Option<sinkings::RetentionProgress>,
    #[serde(skip)]
    configured: bool,
}
impl DetailPreparation {
    fn configure(
        &mut self,
        source: &Source,
        branch: u64,
        content: &ships_sunk::PreparedContent,
    ) -> io::Result<()> {
        if self.configured {
            return Ok(());
        }
        let replay = source.replay_ref();
        let branch = replay
            .branches
            .iter()
            .find(|entry| entry.id == branch)
            .ok_or_else(|| dashboard_core::invalid("Unknown sinking branch"))?;
        self.ships
            .begin(dashboard_text::metadata::branch_bounds(replay, branch).0);
        self.configured = true;
        match content {
            Ok((cat, naming)) => {
                self.retention = Some(sinkings::RetentionProgress::new(naming.retention.clone()));
                self.naming = Some(Arc::clone(naming));
                self.cat = Some(Arc::clone(cat));
            }
            Err(error) => self.catalogue_error = Some(error.clone()),
        }
        Ok(())
    }
}
impl Scan {
    pub fn configure(
        &mut self,
        source: &Source,
        branch: u64,
        content: &ships_sunk::PreparedContent,
    ) -> io::Result<()> {
        self.details.configure(source, branch, content)
    }

    pub fn resume(&mut self, frame: &DecodedFrame) {
        // Input caches are rebuilt by the same census step on the next read.
        self.result_inputs.clear();
        self.result_views.clear();
        self.details.ships.resume(frame);
    }

    /// Observes as one consumer of the shared content: what it publishes
    /// names the refusals of its own reads ([`catalogue::own_refusals`]).
    pub fn observe_details(
        &mut self,
        source: &Source,
        context: &LoadContext,
        frame: &DecodedFrame,
        cancelled: &AtomicBool,
        frame_changes: &FrameChanges,
    ) -> io::Result<()> {
        self.observe_details_read(source, context, frame, cancelled, frame_changes, true)
    }

    /// [`Self::observe_details`] of a frame that holds the tables sunk ships'
    /// scripted place names read only when `places`; a ship first read
    /// without them stops preparation
    /// ([`ships_sunk::battle_titles::Prepared::observe_read`]).
    pub fn observe_details_read(
        &mut self,
        source: &Source,
        context: &LoadContext,
        frame: &DecodedFrame,
        cancelled: &AtomicBool,
        frame_changes: &FrameChanges,
        places: bool,
    ) -> io::Result<()> {
        let mut own = std::mem::take(&mut self.details.refusals);
        let observed = catalogue::own_refusals(&mut own, || {
            self.observe_own_details(source, context, frame, cancelled, frame_changes, places)
        });
        self.details.refusals = own;
        observed
    }

    fn observe_own_details(
        &mut self,
        source: &Source,
        context: &LoadContext,
        frame: &DecodedFrame,
        cancelled: &AtomicBool,
        frame_changes: &FrameChanges,
        places: bool,
    ) -> io::Result<()> {
        let details = &mut self.details;
        details.fleet_reads.through(frame.hour);
        details.ship_reads.through(frame.hour);
        details.census_reads.through(frame.hour);
        if self
            .combat_hours
            .last()
            .is_some_and(|span| span.last_hour == frame.hour)
            && self
                .result_hours
                .last()
                .is_some_and(|span| span.last_hour == frame.hour)
        {
            details.census_reads.event(0, frame.hour, &frame.hour)?;
        }
        let reads = details.ships.scan.ship_reads.len();
        let changes = details
            .ships
            .prepare(source.replay_ref().format, frame, frame_changes);
        let changed = !changes.is_empty();
        let unavailable;
        let retention = if let Some(retention) = &mut details.retention {
            retention.update(&details.ships.scan)
        } else {
            unavailable = super::super::naval_losses::retention(
                &details.ships.scan,
                Err(details
                    .catalogue_error
                    .clone()
                    .expect("configured content outcome")),
            );
            &unavailable
        };
        details.losses.observe_changes(
            &details.ships.scan,
            &changes,
            details.ships.prepared_holders(),
            details.ships.prepared_scope_changed(),
            retention,
            cancelled,
        )?;
        details.sinking_titles.observe_read(
            source,
            context,
            frame,
            if changed {
                &details.ships.scan.ships
            } else {
                &[]
            },
            if changed {
                &details.ships.ship_indices
            } else {
                &[]
            },
            cancelled,
            places,
        )?;
        details.sinking_log.observe(
            &details.ships.scan,
            if changed {
                &details.ships.ship_indices
            } else {
                &[]
            },
            cancelled,
            details.cat.as_deref().ok_or_else(|| {
                details
                    .catalogue_error
                    .as_deref()
                    .expect("configured content outcome")
            }),
            details.naming.as_deref(),
            Some(retention),
        )?;
        let first_new = details
            .ships
            .ship_indices
            .partition_point(|index| *index < details.ship_count);
        let new_ships = if changed {
            &details.ships.scan.ships[first_new..]
        } else {
            &[]
        };
        let names = if new_ships.is_empty() {
            None
        } else {
            Some(ships_sunk::prepared_names(
                &details.ships.scan,
                new_ships,
                details.cat.as_deref().ok_or_else(|| {
                    details
                        .catalogue_error
                        .as_deref()
                        .expect("configured content outcome")
                }),
                details.naming.as_deref(),
                Some(retention),
                cancelled,
            )?)
        };
        for (&index, ship) in details.ships.ship_indices[first_new..]
            .iter()
            .zip(new_ships)
        {
            let (name, class_name) = names.as_ref().expect("new ship names").ship(ship);
            details
                .pending_named
                .entry(ship.entry.battle)
                .or_default()
                .push(PreparedShip {
                    row: NavalBattleNamedShip {
                        sink_hour: ship.entry.sink_hour,
                        known_at: ship.read_hour,
                        name,
                        class_name,
                    },
                    class_token: ship.entry.class_token,
                });
            details.ship_count = details.ship_count.max(index.saturating_add(1));
        }
        if details.ships.scan.ship_reads.len() != reads {
            details.ship_reads.event(0, frame.hour, &frame.hour)?;
        }
        details.ships.scan.ship_reads.clear();
        details.ships.scan.convoy_reads.clear();
        let fleet_tables = [
            TableId::NavalCombats,
            TableId::NavalCombatDetails,
            TableId::TaskForces,
            TableId::Fleets,
        ];
        // Equal acquisitions still supply census/read proof above, but cannot
        // change this fixed-content name projection. Rebuild only changed rows.
        if !fleet_tables
            .iter()
            .any(|table| frame_changes.rows_changed[table.index()])
        {
            return Ok(());
        }
        for (&combat, &index) in &self.combats {
            #[cfg(test)]
            sinkings::work::WORK.with(|work| work.borrow_mut().fleets += 1);
            let province = self.episodes[index].province;
            let mut refused = catalogue::Refusals::default();
            let fleets = details::fleet_names(
                Some(frame),
                combat,
                province,
                details.cat.as_deref(),
                &mut refused,
            );
            let value = FleetRead {
                province,
                fleets,
                refused,
            };
            if details.fleets.get(&combat) != Some(&value) {
                details.fleet_reads.event(combat, frame.hour, &value)?;
                details.fleets.insert(combat, value);
            }
        }
        Ok(())
    }
    pub(super) fn window_records(&self, window: &BattleWindow) -> Self {
        let episodes = self
            .episodes
            .iter()
            .filter_map(|episode| {
                if episode.kind == Kind::Result {
                    // Native end dates select ranking candidates; supplying read dates still gate their values.
                    let native = episode.runs.iter().any(|run| {
                        run.sample.end.is_some_and(|hour| {
                            hour >= window.first_hour && hour <= window.last_hour
                        })
                    });
                    let observed = episode
                        .runs
                        .iter()
                        .any(|run| run.first <= window.last_hour && run.last >= window.first_hour);
                    return (native || observed).then(|| episode.clone());
                }
                let mut row = episode.clone();
                row.runs = episode
                    .runs
                    .iter()
                    .filter(|run| run.first <= window.last_hour && run.last >= window.first_hour)
                    .cloned()
                    .map(|mut run| {
                        run.first = run.first.max(window.first_hour);
                        run.last = run.last.min(window.last_hour);
                        run
                    })
                    .collect();
                (!row.runs.is_empty()).then_some(row)
            })
            .collect();
        Self {
            episodes,
            recorded_hours: self.recorded_hours.clone(),
            combat_hours: self
                .combat_hours
                .iter()
                .filter(|span| {
                    span.first_hour <= window.last_hour && span.last_hour >= window.first_hour
                })
                .cloned()
                .collect(),
            result_hours: self
                .result_hours
                .iter()
                .filter(|span| {
                    span.first_hour <= window.last_hour && span.last_hour >= window.first_hour
                })
                .cloned()
                .collect(),
            refusals: self
                .refusals
                .iter()
                .filter(|((hour, _), _)| *hour >= window.first_hour && *hour <= window.last_hour)
                .map(|(key, count)| (key.clone(), *count))
                .collect(),
            ..Self::default()
        }
    }
    pub fn publish_block(
        &mut self,
        source: &Source,
        context: &LoadContext,
        writer: &mut store::Writer,
        branch: u64,
        ordinal: usize,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        let mut own = std::mem::take(&mut self.details.refusals);
        let published = catalogue::own_refusals(&mut own, || {
            if !self.details.configured {
                self.configure(
                    source,
                    branch,
                    &ships_sunk::prepared_content(source, context),
                )?;
            }
            self.publish_own_block(source.replay_ref(), writer, branch, ordinal, cancelled)
        });
        self.details.refusals = own;
        published
    }

    /// [`Self::publish_block`] of a scan whose content is bound
    /// ([`Self::configure`]), which needs only the recording: a family
    /// prepared on its own publishes the block at the commit that follows it.
    pub fn publish_configured<W: dashboard_base::prepared::pages::Pages + ?Sized>(
        &mut self,
        replay: &Replay,
        writer: &mut W,
        branch: u64,
        ordinal: usize,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        if !self.details.configured {
            return Err(io::Error::other(
                "Naval battles published before their content was bound",
            ));
        }
        let mut own = std::mem::take(&mut self.details.refusals);
        let published = catalogue::own_refusals(&mut own, || {
            self.publish_own_block(replay, writer, branch, ordinal, cancelled)
        });
        self.details.refusals = own;
        published
    }

    fn publish_own_block<W: dashboard_base::prepared::pages::Pages + ?Sized>(
        &mut self,
        replay: &Replay,
        writer: &mut W,
        branch: u64,
        ordinal: usize,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.extend_results();
        let windows = windows(replay, branch);
        let window = &windows[ordinal];
        let boundary = if ordinal == 0 {
            i32::MIN
        } else {
            windows[ordinal - 1].last_hour.saturating_add(1)
        };
        self.windows.event(0, boundary, &(ordinal as u64))?;
        self.windows.through(window.last_hour);
        self.windows.flush(
            writer,
            &format!("naval-battle-windows-{branch}"),
            0,
            cancelled,
        )?;
        let mut changed = BTreeSet::from([ordinal]);
        for &index in &self.dirty {
            check_cancel(cancelled)?;
            let episode = &self.episodes[index];
            if episode.kind == Kind::Result {
                for hour in episode.runs.iter().filter_map(|run| run.sample.end) {
                    let at = windows.partition_point(|window| window.last_hour < hour);
                    if at <= ordinal && at < windows.len() {
                        changed.insert(at);
                    }
                }
            }
            let next = self.pages.len() as u64;
            let page = *self.pages.entry(episode.id.clone()).or_insert(next);
            series::write_reply(
                writer,
                &format!("naval-battle-records-{branch}"),
                page,
                episode,
            )?;
        }
        self.dirty.clear();
        series::write_catalogue(writer, "naval-battle-keys", branch, &self.pages, &())?;
        for at in changed {
            check_cancel(cancelled)?;
            let first = if at == 0 {
                i32::MIN
            } else {
                windows[at - 1].last_hour.saturating_add(1)
            };
            let interval = BattleWindow {
                first_hour: first,
                last_hour: windows[at].last_hour,
            };
            series::write_reply(
                writer,
                &format!("naval-battle-ranking-{branch}"),
                at as u64,
                &self.window_records(&interval),
            )?;
            let window = &windows[at];
            series::write_reply(
                writer,
                &format!("naval-battles-{branch}"),
                at as u64,
                &self.window_records(window).reply_window(at, window)?,
            )?;
        }
        let details = &mut self.details;
        details.ships.finish_prepared(window.last_hour);
        details
            .losses
            .flush(writer, branch, &details.ships.scan, cancelled)?;
        details.sinking_titles.flush(writer, branch, cancelled)?;
        details.fleet_reads.flush(
            writer,
            &format!("naval-battle-fleets-{branch}"),
            0,
            cancelled,
        )?;
        details.ship_reads.flush(
            writer,
            &format!("naval-battle-ship-reads-{branch}"),
            0,
            cancelled,
        )?;
        details.census_reads.flush(
            writer,
            &format!("naval-battle-reads-{branch}"),
            0,
            cancelled,
        )?;
        details.sinking_log.publish(
            replay,
            writer,
            branch,
            ordinal,
            &details.ships.scan,
            cancelled,
            details.cat.as_deref().ok_or_else(|| {
                details
                    .catalogue_error
                    .as_deref()
                    .expect("configured content outcome")
            }),
            details.naming.as_deref(),
            details
                .retention
                .as_mut()
                .map(|retention| retention.update(&details.ships.scan)),
        )?;
        for (combat, mut rows) in std::mem::take(&mut details.pending_named) {
            let next = details.ship_pages.len() as u64;
            let page = *details.ship_pages.entry(combat.to_string()).or_insert(next);
            let section = format!("naval-battle-ships-{branch}");
            if let Some(file) = writer.read_at(&section, page)? {
                let mut prior: Vec<PreparedShip> =
                    serde_json::from_reader(zstd::stream::Decoder::new(file.as_slice())?)?;
                prior.append(&mut rows);
                rows = prior;
            }
            series::write_reply(writer, &section, page, &rows)?;
        }
        series::write_catalogue(
            writer,
            "naval-battle-ship-keys",
            branch,
            &details.ship_pages,
            &(),
        )?;
        series::write_reply(
            writer,
            &format!("naval-battle-availability-{branch}"),
            0,
            &self.available,
        )?;
        let manifest: ships_sunk::prepared::ShipsSunkManifestReply =
            serde_json::from_reader(zstd::stream::Decoder::new(
                writer
                    .read_at(&format!("ships-sunk-manifest-{branch}"), 0)?
                    .ok_or_else(|| {
                        dashboard_core::invalid("Prepared sinking manifest is missing")
                    })?
                    .as_slice(),
            )?)?;
        series::write_reply(
            writer,
            &format!("naval-battle-name-warnings-{branch}"),
            0,
            &manifest.content_warnings,
        )
    }
}

pub fn query(
    source: &Source,
    product: &str,
    params: Value,
    context: &LoadContext,
    store: &store::Store,
    key: &store::SourceKey,
    cancelled: &AtomicBool,
) -> io::Result<Value> {
    let id = params
        .get("branchId")
        .and_then(Value::as_str)
        .ok_or_else(|| dashboard_core::invalid("Invalid battle branch"))?;
    let branch = branch_id(source, id)?;
    match product {
        "naval-battles-manifest" => {
            let params: NavalBattlesManifestParams = serde_json::from_value(params)?;
            NavalBattlesManifest::validate(&params).map_err(dashboard_core::invalid)?;
            let windows = windows(source.replay_ref(), branch);
            let available: bool = series::read_reply(
                store,
                key,
                &format!("naval-battle-availability-{branch}"),
                0,
            )?
            .ok_or_else(|| {
                dashboard_core::invalid("Prepared naval battle availability is missing")
            })?;
            serde_json::to_value(bounded(NavalBattlesManifestReply {
                windows,
                available,
                unavailable: (!available).then(|| "Not in this recording".into()),
                content_warnings: Vec::new(),
            })?)
            .map_err(Into::into)
        }
        "naval-battles" => {
            let params: NavalBattlesParams = serde_json::from_value(params)?;
            NavalBattles::validate(&params).map_err(dashboard_core::invalid)?;
            let reply: NavalBattlesReply = series::read_reply(
                store,
                key,
                &format!("naval-battles-{branch}"),
                params.window as u64,
            )?
            .ok_or_else(|| dashboard_core::invalid("Battle window not in this recording"))?;
            serde_json::to_value(bounded(reply)?).map_err(Into::into)
        }
        "naval-battles-ranking" => {
            let params: NavalBattlesRankingParams = serde_json::from_value(params)?;
            NavalBattlesRanking::validate(&params).map_err(dashboard_core::invalid)?;
            let mut scan = Scan::default();
            let mut episodes = HashMap::new();
            let mut index = series::Reader::new(
                store,
                key,
                format!("naval-battle-windows-{branch}"),
                cancelled,
            )?;
            let first = index
                .latest_event(0, params.first_hour)?
                .map(|(_, value)| serde_json::from_value::<u64>(value))
                .transpose()?
                .unwrap_or(0);
            let last = index
                .latest_event(0, params.last_hour)?
                .map(|(_, value)| serde_json::from_value::<u64>(value))
                .transpose()?
                .unwrap_or(0);
            for index in first..=last {
                check_cancel(cancelled)?;
                let page: Scan = series::read_reply(
                    store,
                    key,
                    &format!("naval-battle-ranking-{branch}"),
                    index,
                )?
                .ok_or_else(|| {
                    dashboard_core::invalid("Prepared naval battle window is missing")
                })?;
                for episode in page.episodes {
                    episodes.insert(episode.id.clone(), episode);
                }
                scan.refusals.extend(page.refusals);
                scan.recorded_hours = page.recorded_hours;
            }
            scan.episodes = episodes.into_values().collect();
            let mut reads = series::Reader::new(
                store,
                key,
                format!("naval-battle-reads-{branch}"),
                cancelled,
            )?;
            let observed: Vec<_> = reads
                .latest_event(0, params.last_hour)?
                .map(|(hour, _)| HourSpan {
                    first_hour: hour,
                    last_hour: hour,
                })
                .into_iter()
                .collect();
            scan.combat_hours = observed.clone();
            scan.result_hours = observed;
            serde_json::to_value(scan.ranked(&params, cancelled)?).map_err(Into::into)
        }
        "naval-battle-details" => {
            details_query(source, params, context, store, key, branch, cancelled)
        }
        _ => unreachable!(),
    }
}
fn details_query(
    source: &Source,
    params: Value,
    context: &LoadContext,
    store: &store::Store,
    key: &store::SourceKey,
    branch: u64,
    cancelled: &AtomicBool,
) -> io::Result<Value> {
    let params: NavalBattleDetailsParams = serde_json::from_value(params)?;
    details::NavalBattleDetails::validate(&params).map_err(dashboard_core::invalid)?;
    let mut warnings: Vec<String> = series::read_reply(
        store,
        key,
        &format!("naval-battle-name-warnings-{branch}"),
        0,
    )?
    .ok_or_else(|| dashboard_core::invalid("Prepared naval battle metadata is missing"))?;
    if let Err(reason) = context
        .content()
        .and_then(|content| catalogue::catalogue(source, content))
    {
        warnings.push(format!("Fleet names unavailable: {reason}"));
    }
    let mut fleet_reader = series::Reader::new(
        store,
        key,
        format!("naval-battle-fleets-{branch}"),
        cancelled,
    )?;
    let mut read_reader = series::Reader::new(
        store,
        key,
        format!("naval-battle-ship-reads-{branch}"),
        cancelled,
    )?;
    let mut refused = catalogue::Refusals::default();
    let (keys, ()) = series::read_catalogue::<()>(store, key, "naval-battle-keys", branch)?
        .ok_or_else(|| dashboard_core::invalid("Prepared battle catalogue is missing"))?;
    let (ship_keys, ()) =
        series::read_catalogue::<()>(store, key, "naval-battle-ship-keys", branch)?
            .ok_or_else(|| dashboard_core::invalid("Prepared sinking catalogue is missing"))?;
    let mut rows = Vec::new();
    for request in &params.battles {
        check_cancel(cancelled)?;
        let page = keys
            .get(&request.id)
            .copied()
            .ok_or_else(|| dashboard_core::invalid("Unknown naval battle"))?;
        let episode: Episode =
            series::read_reply(store, key, &format!("naval-battle-records-{branch}"), page)?
                .ok_or_else(|| dashboard_core::invalid("Prepared naval battle is missing"))?;
        let (run, roster) = details::selection(&episode, request.hour)?;
        let fleets = if let Some((combat, hour)) = roster {
            match fleet_reader.latest_event(combat, hour)? {
                Some((_, value)) => {
                    let read: FleetRead = serde_json::from_value(value)?;
                    if read.province == episode.province {
                        refused.merge(&read.refused);
                        read.fleets
                    } else {
                        missing_fleets("Fleet identities not recorded")
                    }
                }
                None => missing_fleets("Fleet roster or content not recorded"),
            }
        } else {
            missing_fleets("Fleet identities not recorded")
        };
        let named_ships = if let Some((combat, _)) = roster {
            let page = ship_keys.get(&combat.to_string()).copied();
            let names: Vec<PreparedShip> = match page {
                Some(page) => {
                    series::read_reply(store, key, &format!("naval-battle-ships-{branch}"), page)?
                        .unwrap_or_default()
                }
                None => Vec::new(),
            };
            let manifest: ships_sunk::prepared::ShipsSunkManifestReply =
                series::read_reply(store, key, &format!("ships-sunk-manifest-{branch}"), 0)?
                    .ok_or_else(|| {
                        dashboard_core::invalid("Prepared sinking catalogue is missing")
                    })?;
            let classes: BTreeMap<_, _> = manifest
                .classes
                .classes
                .into_iter()
                .map(|class| (class.token, class.name))
                .collect();
            names
                .into_iter()
                .map(|mut ship| {
                    if let Some(Some(name)) = classes.get(&ship.class_token) {
                        ship.row.class_name = Some(name.clone());
                    }
                    ship.row
                })
                .filter(|ship| {
                    details::visible(
                        ship.sink_hour,
                        ship.known_at,
                        run.sample.end.unwrap_or(request.hour),
                        request.hour,
                    )
                })
                .collect()
        } else {
            Vec::new()
        };
        let read = read_reader
            .latest_event(0, request.hour)?
            .map(|(hour, _)| hour);
        rows.push(details::detail_row(
            request,
            roster,
            fleets,
            named_ships,
            read,
        ));
    }
    warnings.extend(refused.lines("Naval battle fleet names"));
    let reply = NavalBattleDetailsReply { rows, warnings };
    dashboard_base::products::check_reply_budget(&reply)
        .map_err(|_| io::Error::other("Naval battle details exceed their reply budget"))?;
    serde_json::to_value(reply).map_err(Into::into)
}
fn missing_fleets(reason: &str) -> [NavalBattleFleetNames; 2] {
    std::array::from_fn(|_| NavalBattleFleetNames {
        names: Vec::new(),
        unknown: Some(reason.into()),
    })
}
