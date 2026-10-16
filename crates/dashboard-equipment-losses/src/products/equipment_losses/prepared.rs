//! Shared all-country loss observations, sparse exact values and native-dated ship events.
use super::*;
use dashboard_base::prepared::frame_changes::FrameChanges;
#[cfg(test)]
use dashboard_base::prepared::series::Run;
use dashboard_base::prepared::series::{self, Accumulator, Reader};
use serde_json::{Value, json};

type Identity = (u32, u32);

/// Current resolved native rows only. No replay rows or accumulated loss
/// operands live here; A3 refreshes these joins and resume rebuilds them.
#[derive(Default)]
struct CurrentJoins {
    census: Option<Vec<(u32, u32)>>,
    by_slot: BTreeMap<u32, Vec<Identity>>,
    rows: [BTreeMap<u32, Vec<usize>>; 4],
    unresolved: [BTreeSet<u32>; 4],
    pending: [bool; 3],
    lend_owners: BTreeSet<Identity>,
    reads: Option<[bool; 4]>,
    drain: DestructionInputs,
    drain_status: Option<(String, Option<String>, BTreeSet<u32>, bool)>,
    land: CoverageInputs,
    joined: bool,
    variants_pending: bool,
    /// Whether the last observed hour lacked complete Countries or CountryLinks.
    identities_incomplete: bool,
}

fn join_rows(
    rows: impl Iterator<Item = (usize, u32)>,
    links: Option<&[CountryLinkRow]>,
    by_slot: &mut BTreeMap<u32, Vec<usize>>,
    unresolved: &mut BTreeSet<u32>,
) -> usize {
    by_slot.clear();
    unresolved.clear();
    let mut visits = 0;
    for (index, raw) in rows {
        visits += 1;
        match links.and_then(|links| link_slot(links, raw)) {
            Some(slot) => by_slot.entry(slot).or_default().push(index),
            None => {
                unresolved.insert(raw);
            }
        }
    }
    visits
}

#[derive(Serialize, Deserialize)]
struct Country {
    actor: u64,
    scanned: Scanned,
    state: State,
    #[serde(with = "dashboard_core::map_pairs")]
    totals: BTreeMap<LeafKey, i128>,
    #[serde(with = "dashboard_core::map_pairs")]
    leaves: BTreeMap<LeafKey, u64>,
    last_read: Option<String>,
}

/// The existing naming and uncertainty inputs contain changes, not hourly values.
#[derive(Serialize, Deserialize)]
struct CountryMetadata {
    scanned: Scanned,
    #[serde(with = "dashboard_core::map_pairs")]
    leaves: BTreeMap<LeafKey, u64>,
}

#[derive(Serialize, Deserialize)]
struct ShipLoss {
    sink_hour: i32,
    variant: Option<u32>,
    delta_raw: String,
}

#[derive(Serialize, Deserialize)]
pub struct Prepared {
    next: i64,
    interrupted: bool,
    keys: BTreeMap<String, u64>,
    #[serde(with = "dashboard_core::map_pairs")]
    countries: BTreeMap<Identity, Country>,
    absent: Scanned,
    absent_state: State,
    variants: VariantCatalogue,
    loss_variants: BTreeSet<u32>,
    variant_counts: BTreeMap<u64, usize>,
    values: Accumulator,
    lend_read: Option<i32>,
    #[serde(skip)]
    joins: CurrentJoins,
    #[serde(skip)]
    current: BTreeMap<Identity, Option<usize>>,
    #[serde(skip)]
    memberships: Option<BTreeMap<u32, Option<census::Membership>>>,
    #[cfg(any(test, feature = "test-support"))]
    #[serde(skip)]
    test_changes: Option<dashboard_base::prepared::frame_changes::FrameChangeTracker>,
    #[cfg(test)]
    #[serde(skip)]
    work: dashboard_base::prepared::observer_tests::Work,
}

fn state(format: replay::format::FormatVersion) -> State {
    State {
        land_source: LandSource::CombatLog,
        include_all_destruction: true,
        destruction_coverage_not_recorded: format
            .field_gaps(TableId::EquipmentDestruction)
            .next()
            .is_some(),
        ..Default::default()
    }
}
fn key(keys: &mut BTreeMap<String, u64>, value: &impl Serialize) -> io::Result<u64> {
    let next = keys.len() as u64;
    Ok(*keys.entry(serde_json::to_string(value)?).or_insert(next))
}

impl Prepared {
    pub fn new(replay: &Replay, branch: u64, first: i32) -> Self {
        let first = replay
            .branches
            .iter()
            .find(|start| start.id == branch)
            .map_or(first, |start| branch_bounds(replay, start).0);
        Self {
            next: i64::from(first),
            interrupted: false,
            keys: BTreeMap::new(),
            countries: BTreeMap::new(),
            absent: Scanned {
                format: replay.format,
                ..Default::default()
            },
            absent_state: state(replay.format),
            variants: Default::default(),
            loss_variants: BTreeSet::new(),
            variant_counts: BTreeMap::new(),
            values: Default::default(),
            lend_read: None,
            joins: Default::default(),
            current: Default::default(),
            memberships: None,
            #[cfg(any(test, feature = "test-support"))]
            test_changes: None,
            #[cfg(test)]
            work: Default::default(),
        }
    }

    /// Existing parity fixtures use the shared fact producer across adjacent frames.
    #[cfg(any(test, feature = "test-support"))]
    pub fn observe(
        &mut self,
        frame: &DecodedFrame,
        replay: &Replay,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        let changes = self
            .test_changes
            .get_or_insert_with(dashboard_base::prepared::frame_changes::FrameChangeTracker::new)
            .observe(frame, replay.format);
        self.observe_changes(frame, replay, cancelled, &changes)
    }

    pub fn observe_changes(
        &mut self,
        frame: &DecodedFrame,
        replay: &Replay,
        cancelled: &AtomicBool,
        changes: &FrameChanges,
    ) -> io::Result<()> {
        dashboard_text::metadata::check_cancel(cancelled)?;
        if validate_campaign(frame, replay).is_err() || i64::from(frame.hour) < self.next {
            return Ok(());
        }
        let interrupted = self.interrupted || self.next < i64::from(frame.hour);
        if self.next < i64::from(frame.hour) {
            self.finish_block(frame.hour - 1);
        }
        self.values.advance(frame.hour)?;
        self.joins.variants_pending |= VariantCatalogue::TABLES
            .iter()
            .any(|table| changes.rows_changed[table.index()]);
        let variant_read = changes.slot_read[TableId::VariantDefinitions.index()];
        if interrupted
            || changes.rows_changed[TableId::VariantDefinitions.index()]
            || (variant_read && self.joins.variants_pending)
        {
            self.variants.observe(replay.format, frame);
            if variant_read {
                self.joins.variants_pending = false;
            }
        } else {
            self.variants.advance(frame.hour);
        }
        let links = frame.complete::<tables::CountryLinks>().ok();
        let census = frame.complete::<tables::Countries>().ok();
        let restoring = !self.joins.joined;
        if restoring {
            self.joins.joined = true;
        }
        let mut affected: BTreeMap<Identity, [bool; 4]> = BTreeMap::new();
        let projection_changed =
            self.joins.census.is_none() || changes.rows_changed[TableId::Countries.index()];
        let different = projection_changed
            && self.joins.census.as_ref().is_none_or(|before| {
                census.is_none_or(|rows| {
                    before.len() != rows.len()
                        || before
                            .iter()
                            .zip(rows)
                            .any(|(&(id, slot), row)| id != row.id || slot != row.canonical_slot)
                })
            });
        let mapping_changed =
            restoring || different || changes.rows_changed[TableId::CountryLinks.index()];
        self.absent.advance(frame.hour.saturating_sub(1));
        if mapping_changed {
            let before = std::mem::take(&mut self.current);
            self.joins.by_slot.clear();
            if let (Some(rows), Some(links)) = (census, links) {
                for (index, row) in rows.iter().enumerate() {
                    #[cfg(test)]
                    {
                        self.work.row_visits += 1;
                    }
                    if let Ok(slot) = mapped_slot(row, links) {
                        let identity = (slot, row.id);
                        self.current
                            .entry(identity)
                            .and_modify(|value| *value = None)
                            .or_insert(Some(index));
                        if !self.countries.contains_key(&identity) {
                            let actor = key(&mut self.keys, &("country", identity))?;
                            let mut continuation = state(replay.format);
                            continuation
                                .destruction_withheld
                                .clone_from(&self.absent_state.destruction_withheld);
                            self.countries.insert(
                                identity,
                                Country {
                                    actor,
                                    scanned: self.absent.clone(),
                                    state: continuation,
                                    totals: BTreeMap::new(),
                                    leaves: BTreeMap::new(),
                                    last_read: None,
                                },
                            );
                        }
                    } else {
                        let identity = (row.canonical_slot, row.id);
                        if self.countries.contains_key(&identity) {
                            self.current.insert(identity, None);
                        }
                    }
                }
            }
            for (&identity, index) in &self.current {
                if index.is_some() {
                    self.joins
                        .by_slot
                        .entry(identity.0)
                        .or_default()
                        .push(identity);
                }
                if before.get(&identity).map(Option::is_some) != Some(index.is_some()) {
                    affected.entry(identity).or_default();
                    // Only resolved current holders can consume a supplying
                    // read. Returning holders rebuild their deletion witness
                    // from retained exchanges, including after restore.
                    if index.is_some() && !self.countries[&identity].state.exchanges.is_empty() {
                        self.joins.lend_owners.insert(identity);
                    } else {
                        self.joins.lend_owners.remove(&identity);
                    }
                }
            }
            for identity in before
                .keys()
                .filter(|identity| !self.current.contains_key(identity))
            {
                affected.entry(*identity).or_default();
                self.joins.lend_owners.remove(identity);
            }
            self.joins.census = census.map(|rows| {
                rows.iter()
                    .map(|row| (row.id, row.canonical_slot))
                    .collect()
            });
        }
        let absent = if census.is_some() && links.is_some() {
            TaggedCountry::Absent
        } else {
            TaggedCountry::Unresolved
        };
        let old_memberships = if mapping_changed
            || [TableId::DivisionCensus, TableId::CountryLinks]
                .iter()
                .any(|table| changes.rows_changed[table.index()])
        {
            Some(std::mem::replace(
                &mut self.memberships,
                census::memberships(frame),
            ))
        } else {
            None
        };
        if let Some(old) = old_memberships {
            let slots: BTreeSet<_> = old
                .as_ref()
                .into_iter()
                .flat_map(|values| values.keys())
                .chain(
                    self.memberships
                        .as_ref()
                        .into_iter()
                        .flat_map(|values| values.keys()),
                )
                .copied()
                .collect();
            for slot in slots {
                let previous = old.as_ref().and_then(|values| values.get(&slot));
                let current = self
                    .memberships
                    .as_ref()
                    .and_then(|values| values.get(&slot));
                if previous != current
                    && let Some(identities) = self.joins.by_slot.get(&slot)
                {
                    for &identity in identities {
                        affected.entry(identity).or_default();
                    }
                }
            }
        }
        if mapping_changed
            || [
                TableId::Players,
                TableId::CombatLogState,
                TableId::SimulationFeatures,
                TableId::CountryLinks,
            ]
            .iter()
            .any(|table| changes.rows_changed[table.index()])
        {
            let land = links.map_or_else(CoverageInputs::default, |links| {
                coverage_inputs(frame, links)
            });
            if restoring || self.joins.land != land {
                for identities in self.joins.by_slot.values() {
                    for &identity in identities {
                        affected.entry(identity).or_default();
                    }
                }
            }
            self.joins.land = land;
        }
        let reads = [
            land_read(frame),
            matches!(frame.complete::<tables::SunkShips>(), Ok([_])),
            frame.complete::<tables::SunkConvoys>().is_ok(),
            frame.complete::<tables::LendLeaseEquipment>().is_ok(),
        ];
        if restoring || self.joins.reads != Some(reads) {
            for identities in self.joins.by_slot.values() {
                for &identity in identities {
                    affected.entry(identity).or_default();
                }
            }
        }
        self.joins.reads = Some(reads);
        let source_tables = [
            TableId::SunkShips,
            TableId::SunkConvoys,
            TableId::LendLeaseEquipment,
            TableId::CombatLogEquipment,
        ];
        for (record, table) in source_tables.into_iter().enumerate() {
            if !mapping_changed && !changes.rows_changed[table.index()] {
                continue;
            }
            if record < 3 {
                self.joins.pending[record] = true;
            }
            let (_visits, by_slot, unresolved) = {
                let mut by_slot = BTreeMap::new();
                let mut unresolved = BTreeSet::new();
                let visits = match record {
                    0 => join_rows(
                        frame
                            .complete::<tables::SunkShips>()
                            .ok()
                            .filter(|rows| rows.len() == 1)
                            .into_iter()
                            .flat_map(|rows| {
                                rows[0]
                                    .entries
                                    .iter()
                                    .enumerate()
                                    .map(|(index, row)| (index, row.victim_country))
                            }),
                        links,
                        &mut by_slot,
                        &mut unresolved,
                    ),
                    1 => join_rows(
                        frame
                            .complete::<tables::SunkConvoys>()
                            .ok()
                            .into_iter()
                            .flat_map(|rows| {
                                rows.iter()
                                    .enumerate()
                                    .map(|(index, row)| (index, row.owner_country))
                            }),
                        links,
                        &mut by_slot,
                        &mut unresolved,
                    ),
                    2 => join_rows(
                        frame
                            .complete::<tables::LendLeaseEquipment>()
                            .ok()
                            .into_iter()
                            .flat_map(|rows| {
                                rows.iter()
                                    .enumerate()
                                    .map(|(index, row)| (index, row.giver))
                            }),
                        links,
                        &mut by_slot,
                        &mut unresolved,
                    ),
                    _ => {
                        let rows = match &*frame.tables[TableId::CombatLogEquipment.index()] {
                            replay::frame::TableView::Complete(rows)
                            | replay::frame::TableView::Truncated { rows, .. } => {
                                rows.rows::<tables::CombatLogEquipment>()
                            }
                            replay::frame::TableView::Unavailable { .. } => None,
                        };
                        let mut visits = 0;
                        for (index, row) in rows.into_iter().flatten().enumerate() {
                            visits += 1;
                            by_slot
                                .entry(row.country)
                                .or_insert_with(Vec::new)
                                .push(index);
                            if links
                                .and_then(|links| link_slot(links, row.country))
                                .is_none()
                            {
                                unresolved.insert(row.country);
                            }
                        }
                        visits
                    }
                };
                (visits, by_slot, unresolved)
            };
            #[cfg(test)]
            {
                self.work.row_visits += _visits;
            }
            self.joins.rows[record] = by_slot;
            self.joins.unresolved[record] = unresolved;
        }
        if mapping_changed || changes.rows_changed[TableId::EquipmentDestruction.index()] {
            self.joins.drain = if self.absent_state.destruction_withheld.is_none()
                && !(absent == TaggedCountry::Absent && self.current.is_empty())
            {
                destruction_inputs(frame, links, replay.format)?
            } else {
                DestructionInputs::default()
            };
            #[cfg(test)]
            {
                self.work.row_visits += self.joins.drain.row_visits;
            }
            let status = match &*frame.tables[TableId::EquipmentDestruction.index()] {
                replay::frame::TableView::Complete(_) => "complete".to_owned(),
                replay::frame::TableView::Truncated { .. } => DESTRUCTION_TRUNCATED.to_owned(),
                replay::frame::TableView::Unavailable { reason } => reason.clone(),
            };
            let before = self.joins.drain_status.as_ref();
            if restoring || before.is_none_or(|old| old.0 != status) {
                for &identity in self.countries.keys() {
                    affected.entry(identity).or_default();
                }
            } else if before.is_some_and(|old| {
                old.1 != self.joins.drain.supplemental || old.2 != self.joins.drain.unresolved
            }) {
                for &identity in self.current.keys() {
                    affected.entry(identity).or_default();
                }
            } else if before.is_some_and(|old| old.3 != self.joins.drain.nonempty) {
                for (&identity, index) in &self.current {
                    if index.is_none() {
                        affected.entry(identity).or_default();
                    }
                }
                if census.is_none() || links.is_none() {
                    for &identity in self.countries.keys() {
                        affected.entry(identity).or_default();
                    }
                }
            }
            self.joins.drain_status = Some((
                status,
                self.joins.drain.supplemental.clone(),
                self.joins.drain.unresolved.clone(),
                self.joins.drain.nonempty,
            ));
        }
        // Every complete nonempty drain changes its owners' totals hourly.
        for slot in self.joins.drain.by_slot.keys() {
            if let Some(identities) = self.joins.by_slot.get(slot) {
                for &identity in identities {
                    affected.entry(identity).or_default();
                }
            }
        }
        let process = [
            self.joins.pending[0] && reads[1],
            self.joins.pending[1] && reads[2] && changes.slot_read[TableId::SunkConvoys.index()],
            self.joins.pending[2]
                && reads[3]
                && changes.slot_read[TableId::LendLeaseEquipment.index()],
        ];
        for (record, process) in process.into_iter().enumerate() {
            if !process {
                continue;
            }
            for slot in self.joins.rows[record].keys() {
                if let Some(identities) = self.joins.by_slot.get(slot) {
                    for &identity in identities {
                        affected.entry(identity).or_default()[record] = true;
                    }
                }
            }
            if !self.joins.unresolved[record].is_empty() {
                for identities in self.joins.by_slot.values() {
                    for &identity in identities {
                        affected.entry(identity).or_default()[record] = true;
                    }
                }
            }
            if record == 2 {
                for &identity in &self.joins.lend_owners {
                    affected.entry(identity).or_default()[2] = true;
                }
            }
            self.joins.pending[record] = false;
        }
        let mut combat_unresolved = BTreeSet::new();
        let mut combat_owners: BTreeMap<Identity, BTreeSet<u32>> = BTreeMap::new();
        if (mapping_changed
            || changes.rows_changed[TableId::SliceReads.index()]
            || changes.table_read[TableId::SliceReads.index()])
            && let Ok(slices) = frame.complete::<tables::SliceReads>()
        {
            let table = TableId::CombatLogEquipment.index() as u32;
            let first = slices.partition_point(|row| row.table < table);
            let last = slices.partition_point(|row| row.table <= table);
            for read in &slices[first..last] {
                #[cfg(test)]
                {
                    self.work.row_visits += 1;
                }
                if read.sample_hour != frame.hour {
                    continue;
                }
                if let Some(slot) = links.and_then(|links| link_slot(links, read.owner)) {
                    if let Some(identities) = self.joins.by_slot.get(&slot) {
                        for &identity in identities {
                            affected.entry(identity).or_default()[3] = true;
                            combat_owners
                                .entry(identity)
                                .or_default()
                                .insert(read.owner);
                        }
                    }
                } else if self.joins.unresolved[3].contains(&read.owner) {
                    combat_unresolved.insert(read.owner);
                }
            }
        }
        if !combat_unresolved.is_empty() {
            for identities in self.joins.by_slot.values() {
                for &identity in identities {
                    affected.entry(identity).or_default()[3] = true;
                }
            }
        }
        // No holder resolves while either identity table is incomplete: every
        // retained holder is unresolved then, and absent or found again at the
        // first hour both are complete.
        let identities_incomplete = census.is_none() || links.is_none();
        if restoring || interrupted || identities_incomplete || self.joins.identities_incomplete {
            for &identity in self.countries.keys() {
                affected.entry(identity).or_default();
            }
        }
        self.joins.identities_incomplete = identities_incomplete;
        country_metadata(
            &mut self.absent,
            &mut self.absent_state,
            frame,
            links,
            absent,
            true,
            None,
            &self.joins.drain,
        )?;
        let old_lend_read = self.lend_read;
        if reads[3] && changes.slot_read[TableId::LendLeaseEquipment.index()] {
            self.lend_read = Some(frame.hour);
        }
        for (identity, records) in affected {
            #[cfg(test)]
            {
                self.work.history_visits += 1;
            }
            dashboard_text::metadata::check_cancel(cancelled)?;
            let country = self
                .countries
                .get_mut(&identity)
                .expect("resolved loss owner");
            country.scanned.format = replay.format;
            if !country
                .scanned
                .unread_active
                .contains(&LossRecord::LendLease)
                && let Some(read) = old_lend_read
            {
                country.state.exchanges_read = Some(
                    country
                        .state
                        .exchanges_read
                        .map_or(read, |seen| seen.max(read)),
                );
            }
            country.scanned.advance(frame.hour.saturating_sub(1));
            let selected = match self.current.get(&identity) {
                Some(Some(index)) => TaggedCountry::Found(
                    &census.expect("complete census for current owner")[*index],
                    identity.0,
                ),
                Some(None) => TaggedCountry::Unresolved,
                None => absent,
            };
            let membership = self
                .memberships
                .as_ref()
                .and_then(|values| values.get(&identity.0).copied().flatten());
            #[cfg(test)]
            {
                if selected.found().is_some() && country.state.destruction_withheld.is_none() {
                    self.work.row_visits += self
                        .joins
                        .drain
                        .by_slot
                        .get(&identity.0)
                        .map_or(0, Vec::len);
                }
            }
            country_metadata(
                &mut country.scanned,
                &mut country.state,
                frame,
                links,
                selected,
                true,
                Some(self.joins.land.country(identity.0, membership)),
                &self.joins.drain,
            )?;
            if selected.found().is_some() {
                let own = |record: usize| {
                    self.joins.rows[record]
                        .get(&identity.0)
                        .map_or(&[][..], Vec::as_slice)
                };
                if records[0] {
                    if let Ok([ships]) = frame.complete::<tables::SunkShips>() {
                        #[cfg(test)]
                        {
                            self.work.row_visits += own(0).len();
                        }
                        ship_rows(
                            &mut country.scanned,
                            &mut country.state,
                            own(0).iter().map(|&index| &ships.entries[index]),
                        );
                    }
                    country.scanned.unresolved.extend(&self.joins.unresolved[0]);
                }
                if records[1] {
                    if let Ok(rows) = frame.complete::<tables::SunkConvoys>() {
                        #[cfg(test)]
                        {
                            self.work.row_visits += own(1).len();
                        }
                        convoy_rows(
                            &mut country.scanned,
                            &mut country.state,
                            frame.hour,
                            own(1).iter().map(|&index| &rows[index]),
                        );
                    }
                    country.scanned.unresolved.extend(&self.joins.unresolved[1]);
                }
                if records[2] {
                    if let Ok(rows) = frame.complete::<tables::LendLeaseEquipment>() {
                        #[cfg(test)]
                        {
                            self.work.row_visits += own(2).len() * 2;
                        }
                        lend_lease_rows(
                            &mut country.scanned,
                            &mut country.state,
                            frame.hour,
                            own(2).iter().map(|&index| &rows[index]),
                        );
                    }
                    country.scanned.unresolved.extend(&self.joins.unresolved[2]);
                    if country.state.exchanges.is_empty() {
                        self.joins.lend_owners.remove(&identity);
                    } else {
                        self.joins.lend_owners.insert(identity);
                    }
                }
                if records[3] {
                    let rows = match &*frame.tables[TableId::CombatLogEquipment.index()] {
                        replay::frame::TableView::Complete(rows)
                        | replay::frame::TableView::Truncated { rows, .. } => {
                            rows.rows::<tables::CombatLogEquipment>()
                        }
                        replay::frame::TableView::Unavailable { .. } => None,
                    };
                    if let Some(rows) = rows {
                        for raw in combat_owners.get(&identity).into_iter().flatten() {
                            let indices =
                                self.joins.rows[3].get(raw).map_or(&[][..], Vec::as_slice);
                            #[cfg(test)]
                            {
                                self.work.row_visits += indices.len();
                            }
                            combat_rows(
                                &mut country.scanned,
                                &mut country.state,
                                frame.hour,
                                indices.iter().map(|&index| &rows[index]),
                            );
                        }
                    }
                    country.scanned.combat_unresolved.extend(&combat_unresolved);
                }
            }
            let read = serde_json::to_string(&(
                country.scanned.coverage.last().map(|span| span.coverage),
                country
                    .scanned
                    .destruction_coverage
                    .last()
                    .and_then(|span| span.reason.as_deref()),
                country
                    .scanned
                    .unread
                    .iter()
                    .filter(|(_, spans)| {
                        spans
                            .last()
                            .is_some_and(|span| span.last_hour == frame.hour)
                    })
                    .map(|(record, _)| record)
                    .collect::<Vec<_>>(),
            ))?;
            let changed_read = country.last_read.as_ref() != Some(&read);
            let version = self
                .values
                .state(country.actor)
                .and_then(|state| state.last)
                .map_or(0, |run| run.value)
                + i128::from(changed_read);
            if changed_read || interrupted {
                #[cfg(test)]
                {
                    self.work.emitted += 1;
                }
                self.values
                    .observe(country.actor, frame.hour, Some(version), false);
            }
            country.last_read = Some(read);
            let mut changed = BTreeSet::new();
            for (leaf, events) in std::mem::take(&mut country.scanned.events) {
                if let Some(variant) = leaf.1 {
                    self.loss_variants.insert(variant);
                }
                if leaf.0 == LossSource::SunkShips {
                    for (sink_hour, delta) in events {
                        self.values.event(
                            country.actor,
                            frame.hour,
                            &ShipLoss {
                                sink_hour,
                                variant: leaf.1,
                                delta_raw: delta.to_string(),
                            },
                        )?;
                    }
                    continue;
                }
                for (hour, delta) in events {
                    if hour != frame.hour {
                        return Err(io::Error::other(format!(
                            "Equipment loss cause {:?} has an unsupported backdated hour {hour}",
                            leaf.0
                        )));
                    }
                    *country.totals.entry(leaf).or_default() += delta;
                    changed.insert(leaf);
                }
                let actor = key(&mut self.keys, &("loss", identity, leaf))?;
                country.leaves.insert(leaf, actor);
            }
            let observed = if interrupted {
                country.leaves.keys().copied().collect()
            } else {
                changed.clone()
            };
            for leaf in observed {
                #[cfg(test)]
                {
                    self.work.emitted += 1;
                }
                let actor = country.leaves[&leaf];
                let value = country.totals[&leaf];
                self.values.observe_boundary(
                    actor,
                    frame.hour,
                    Some(value),
                    false,
                    changed.contains(&leaf),
                );
                if changed.contains(&leaf) {
                    self.values.event(
                        actor,
                        frame.hour,
                        &LossPoint {
                            hour: frame.hour,
                            total_raw: value.to_string(),
                        },
                    )?;
                }
            }
        }
        self.values.through(frame.hour);
        self.next = i64::from(frame.hour) + 1;
        self.interrupted = false;
        Ok(())
    }

    pub fn bytes(&self) -> usize {
        self.values.bytes()
    }

    pub fn finish_block(&mut self, last: i32) {
        if self.next > i64::from(last) {
            return;
        }
        let first = self.next as i32;
        self.absent.advance(first.saturating_sub(1));
        for country in self.countries.values_mut() {
            country.scanned.advance(first.saturating_sub(1));
            if !country
                .scanned
                .unread_active
                .contains(&LossRecord::LendLease)
                && let Some(read) = self.lend_read
            {
                country.state.exchanges_read = Some(
                    country
                        .state
                        .exchanges_read
                        .map_or(read, |seen| seen.max(read)),
                );
            }
        }
        self.variants.forget(first);
        self.absent.missing(first, last, LandSource::CombatLog);
        for country in self.countries.values_mut() {
            country.scanned.missing(first, last, LandSource::CombatLog);
        }
        self.values.through(last);
        self.next = i64::from(last) + 1;
        self.interrupted = true;
    }

    pub fn flush(
        &mut self,
        writer: &mut store::Writer,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        if let Ok(last) = i32::try_from(self.next - 1) {
            self.absent.advance(last);
            for country in self.countries.values_mut() {
                country.scanned.advance(last);
                if !country
                    .scanned
                    .unread_active
                    .contains(&LossRecord::LendLease)
                    && let Some(read) = self.lend_read
                {
                    country.state.exchanges_read = Some(
                        country
                            .state
                            .exchanges_read
                            .map_or(read, |seen| seen.max(read)),
                    );
                }
            }
        }
        self.values.flush(
            writer,
            &format!("equipment-loss-values-{branch}"),
            5,
            cancelled,
        )?;
        for country in self.countries.values() {
            dashboard_text::metadata::check_cancel(cancelled)?;
            series::write_reply(
                writer,
                &format!("equipment-loss-country-{branch}-{}", country.actor),
                0,
                &CountryMetadata {
                    scanned: country.scanned.clone(),
                    leaves: country.leaves.clone(),
                },
            )?;
        }
        for &variant in &self.loss_variants {
            let handle = u64::from(variant) << 32 | u64::from(VARIANT_TYPE);
            let Some(versions) = self.variants.versions.get(&handle) else {
                continue;
            };
            if self.variant_counts.get(&handle) == Some(&versions.len()) {
                continue;
            }
            let actor = key(&mut self.keys, &("variant", handle))?;
            series::write_reply(
                writer,
                &format!("equipment-loss-variant-{branch}-{actor}"),
                0,
                versions,
            )?;
            self.variant_counts.insert(handle, versions.len());
        }
        series::write_catalogue(writer, "equipment-loss", branch, &self.keys, &())
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Params {
    selection: LossesParams,
    from: Option<f64>,
    to: Option<f64>,
    width: Option<u32>,
    #[serde(default)]
    reads: Vec<i32>,
    head_hour: i32,
    detail_hour: i32,
}

/// Only viewport geometry is sampled. Every reached hour is an exact run read.
#[derive(Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PreparedLossesReply {
    #[serde(flatten)]
    pub reply: LossesReply,
    pub prepared_reads: Vec<i32>,
    pub prepared_head_hour: i32,
    pub prepared_detail_hour: i32,
    pub prepared_from: f64,
    pub prepared_to: f64,
    pub prepared_width: u32,
}

fn sampled_events(
    reader: &mut Reader<'_>,
    actor: u64,
    from: f64,
    to: f64,
    width: u32,
    reads: &[i32],
) -> io::Result<Vec<(i32, i128)>> {
    let summary = reader.summary(
        actor,
        (from.floor() as i32, to.ceil() as i32),
        (from, to, width),
        5,
    )?;
    let mut hours: BTreeSet<i32> = reads.iter().copied().collect();
    // Native cause uncertainty starts at its first loss, even when that
    // amount is neither viewport extremum nor a reached read.
    if let Some(first) = reader
        .events(&[actor], i32::MIN, i32::MAX, 0, 1)?
        .rows
        .first()
    {
        hours.insert(first.hour);
    }
    if let Some(runs) = summary.runs {
        hours.extend(runs.into_iter().map(|run| run.first_hour));
    } else {
        for bucket in summary.buckets.into_iter().flatten() {
            hours.extend([
                bucket.first_hour,
                bucket.min_hour,
                bucket.max_hour,
                bucket.last_value_hour,
            ]);
        }
    }
    // Palette weights use the complete requested counting interval's signed
    // peak, independently of the chart viewport.
    let first = reads.iter().min().copied().unwrap_or(from.floor() as i32);
    let last = reads.iter().max().copied().unwrap_or(to.ceil() as i32);
    for bucket in reader
        .summary(
            actor,
            (first, last),
            (f64::from(first), f64::from(last), 1),
            5,
        )?
        .buckets
        .into_iter()
        .flatten()
    {
        hours.extend([bucket.min_hour, bucket.max_hour]);
    }
    let mut samples = BTreeMap::new();
    for hour in hours {
        if let Some((_, row)) = reader.latest_event(actor, hour)? {
            let point: LossPoint = serde_json::from_value(row)?;
            let value = point
                .total_raw
                .parse::<i128>()
                .map_err(|_| dashboard_core::invalid("Invalid prepared loss amount"))?;
            samples.insert(point.hour, value);
        }
    }
    let mut previous = 0;
    Ok(samples
        .into_iter()
        .map(|(hour, value)| {
            let delta = value - previous;
            previous = value;
            (hour, delta)
        })
        .collect())
}

pub fn query(
    source: &Source,
    raw: &Value,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> io::Result<Value> {
    let params: Params = serde_json::from_value(raw.clone())?;
    EquipmentLosses::validate(&params.selection).map_err(dashboard_core::invalid)?;
    if !params.reads.contains(&params.head_hour)
        || !params.reads.contains(&params.detail_hour)
        || params.reads.windows(2).any(|hours| hours[0] >= hours[1])
    {
        return Err(dashboard_core::invalid(
            "Invalid prepared equipment loss reached hours",
        ));
    }
    let selected = params.selection.country.ok_or_else(|| {
        dashboard_core::invalid("Choose a recorded native country for equipment losses")
    })?;
    let branch = params.selection.branch();
    if !validate_origin(source, &params.selection, context, cancelled)? {
        return Ok(Value::Null);
    }
    let Some(coverage) = dashboard_base::prepared::coverage::coverage(
        source,
        context.content.as_ref(),
        branch,
        cancelled,
    )?
    else {
        return Ok(Value::Null);
    };
    let (from, to, width) = series::viewport(params.from, params.to, params.width)?;
    if !coverage.complete
        && (to.ceil() as i32 > coverage.last_hour
            || params.reads.iter().any(|hour| *hour > coverage.last_hour))
    {
        return Ok(Value::Null);
    }
    let (store, source_key) = dashboard_base::prepared::files(source, context.content.as_ref())?;
    let Some(scanned) = read_scanned(
        &store,
        &source_key,
        branch,
        (selected.slot, selected.country_id),
        (from, to, width),
        &params.reads,
        source.replay_ref().format,
        cancelled,
    )?
    else {
        return Ok(Value::Null);
    };
    dashboard_text::metadata::check_cancel(cancelled)?;
    let reply = reply_with_content(source, &params.selection, context, &scanned)?;
    let result = PreparedLossesReply {
        reply,
        prepared_reads: params.reads,
        prepared_head_hour: params.head_hour,
        prepared_detail_hour: params.detail_hour,
        prepared_from: from,
        prepared_to: to,
        prepared_width: width,
    };
    dashboard_base::products::check_reply_budget(&result)?;
    serde_json::to_value(result).map_err(io::Error::from)
}

fn validate_origin(
    source: &Source,
    selection: &LossesParams,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> io::Result<bool> {
    let selected = selection.country.ok_or_else(|| {
        dashboard_core::invalid("Choose a recorded native country for equipment losses")
    })?;
    let branch = selection.branch();
    let Some(history) = dashboard_base::products::country_history::prepared_branch_histories(
        source,
        branch,
        context.content.as_ref(),
        cancelled,
    )?
    else {
        return Ok(false);
    };
    let witnessed = history
        .countries
        .get(&selection.tag)
        .is_some_and(|history| {
            history.identities.iter().any(|identity| {
                identity.first_hour <= selected.origin_hour
                    && selected.origin_hour <= identity.last_hour
                    && identity.slot == selected.slot
                    && identity.country_id == selected.country_id
            })
        });
    if !witnessed {
        return Err(dashboard_core::invalid(
            "The selected native country identity is not recorded at its origin",
        ));
    }
    Ok(true)
}

#[allow(clippy::too_many_arguments)]
fn read_scanned(
    store: &store::Store,
    source_key: &store::SourceKey,
    branch: u64,
    identity: Identity,
    (from, to, width): (f64, f64, u32),
    reads: &[i32],
    format: replay::format::FormatVersion,
    cancelled: &AtomicBool,
) -> io::Result<Option<Scanned>> {
    let Some((keys, ())) =
        series::read_catalogue::<()>(store, source_key, "equipment-loss", branch)?
    else {
        return Ok(None);
    };

    let Some(&country) = keys.get(&serde_json::to_string(&("country", identity))?) else {
        return Ok(None);
    };
    let Some(mut metadata) = series::read_reply::<CountryMetadata>(
        store,
        source_key,
        &format!("equipment-loss-country-{branch}-{country}"),
        0,
    )?
    else {
        return Ok(None);
    };
    let (from, to, width) = series::viewport(Some(from), Some(to), Some(width))?;
    series::summary_budget(from, to, width, metadata.leaves.len())?;
    metadata.scanned.format = format;
    let mut reader = Reader::new(
        store,
        source_key,
        format!("equipment-loss-values-{branch}"),
        cancelled,
    )?;
    for (&leaf, &actor) in &metadata.leaves {
        metadata.scanned.events.insert(
            leaf,
            sampled_events(&mut reader, actor, from, to, width, reads)?,
        );
    }
    // The sparse ship key is ordered by observation. Native sinking dates
    // are applied by the existing points builder, even after prefix commits.
    let mut ships = Reader::new(
        store,
        source_key,
        format!("equipment-loss-values-{branch}"),
        cancelled,
    )?;
    let mut cursor = 0;
    loop {
        let page = ships.events(&[country], i32::MIN, i32::MAX, cursor, 1024)?;
        if page
            .cursor
            .is_some_and(|next| next <= cursor || page.rows.is_empty())
        {
            return Err(dashboard_core::invalid(
                "Prepared ship loss page made no progress",
            ));
        }
        for row in page.rows {
            let ship: ShipLoss = serde_json::from_value(row.row)?;
            let delta = ship
                .delta_raw
                .parse::<i128>()
                .map_err(|_| dashboard_core::invalid("Invalid prepared ship loss amount"))?;
            metadata
                .scanned
                .events
                .entry((LossSource::SunkShips, ship.variant))
                .or_default()
                .push((ship.sink_hour, delta));
        }
        let Some(next) = page.cursor else {
            break;
        };
        cursor = next;
    }
    for variant in metadata
        .scanned
        .events
        .keys()
        .filter_map(|(_, variant)| *variant)
        .collect::<BTreeSet<_>>()
    {
        let handle = u64::from(variant) << 32 | u64::from(VARIANT_TYPE);
        if let Some(&actor) = keys.get(&serde_json::to_string(&("variant", handle))?)
            && let Some(versions) = series::read_reply(
                store,
                source_key,
                &format!("equipment-loss-variant-{branch}-{actor}"),
                0,
            )?
        {
            metadata.scanned.variants.versions.insert(handle, versions);
        }
    }
    Ok(Some(metadata.scanned))
}

pub fn series_query(
    source: &Source,
    product: &str,
    raw: &Value,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> io::Result<Value> {
    let selection: LossesParams = serde_json::from_value(raw["family"]["selection"].clone())?;
    EquipmentLosses::validate(&selection).map_err(dashboard_core::invalid)?;
    if raw["branchId"].as_str() != Some(selection.branch_id.as_str()) {
        return Err(dashboard_core::invalid(
            "Equipment loss selection branch does not match its request",
        ));
    }
    if !validate_origin(source, &selection, context, cancelled)? {
        return Ok(Value::Null);
    }
    let identity = selection.country.ok_or_else(|| {
        dashboard_core::invalid("Choose a recorded native country for equipment losses")
    })?;
    let branch = selection.branch();
    let (store, source_key) = dashboard_base::prepared::files(source, context.content.as_ref())?;
    let Some((keys, ())) =
        series::read_catalogue::<()>(&store, &source_key, "equipment-loss", branch)?
    else {
        return Ok(Value::Null);
    };
    let Some(&country) = keys.get(&serde_json::to_string(&(
        "country",
        (identity.slot, identity.country_id),
    ))?) else {
        return Ok(Value::Null);
    };
    let Some(metadata) = series::read_reply::<CountryMetadata>(
        &store,
        &source_key,
        &format!("equipment-loss-country-{branch}-{country}"),
        0,
    )?
    else {
        return Ok(Value::Null);
    };
    let wanted: Option<Vec<LeafKey>> = raw
        .get("leaves")
        .filter(|value| !value.is_null())
        .map(|value| serde_json::from_value(value.clone()))
        .transpose()?;
    let actors: Vec<_> = wanted
        .as_ref()
        .map_or_else(
            || metadata.leaves.values().copied().collect::<Vec<_>>(),
            |leaves| {
                leaves
                    .iter()
                    .filter_map(|leaf| metadata.leaves.get(leaf))
                    .copied()
                    .collect()
            },
        )
        .into_iter()
        .chain(std::iter::once(country))
        .collect();
    let mut reader = Reader::new(
        &store,
        &source_key,
        format!("equipment-loss-values-{branch}"),
        cancelled,
    )?;
    match product {
        "series-next-change" => {
            let navigation: series::Navigation = serde_json::from_value(raw.clone())?;
            let origin = navigation.origin()?;
            let forward = matches!(
                navigation.key,
                series::MeasureKey::Home | series::MeasureKey::ArrowRight
            );
            let candidate = series::navigate(
                &mut [(
                    &mut reader,
                    actors.into_iter().map(|actor| (actor, None)).collect(),
                )],
                &navigation,
            )?;
            let mut reached = candidate.filter(|hour| {
                if forward {
                    *hour > origin
                } else {
                    *hour < origin
                }
            });
            let mut cursor = 0;
            loop {
                let page = reader.events(&[country], i32::MIN, i32::MAX, cursor, 1024)?;
                if page
                    .cursor
                    .is_some_and(|next| next <= cursor || page.rows.is_empty())
                {
                    return Err(dashboard_core::invalid(
                        "Prepared ship loss page made no progress",
                    ));
                }
                for row in page.rows {
                    let ship: ShipLoss = serde_json::from_value(row.row)?;
                    if wanted.as_ref().is_some_and(|leaves| {
                        !leaves.contains(&(LossSource::SunkShips, ship.variant))
                    }) {
                        continue;
                    }
                    let hour = f64::from(ship.sink_hour);
                    if hour < navigation.first_hour
                        || hour > navigation.last_hour
                        || if forward {
                            hour <= origin
                        } else {
                            hour >= origin
                        }
                    {
                        continue;
                    }
                    reached = Some(reached.map_or(hour, |before| {
                        if forward {
                            before.min(hour)
                        } else {
                            before.max(hour)
                        }
                    }));
                }
                let Some(next) = page.cursor else {
                    break;
                };
                cursor = next;
            }
            Ok(json!({"hour": reached.or(candidate)}))
        }
        "events-page" => {
            let (from, to, cursor, size) = series::event_window(
                raw["from"].as_f64(),
                raw["to"].as_f64(),
                raw["cursor"].as_str(),
                raw["pageSize"]
                    .as_u64()
                    .and_then(|size| u32::try_from(size).ok()),
            )?;
            let page = reader.events(&actors, from, to, cursor, size)?;
            Ok(
                json!({"firstOrdinal":page.first.to_string(), "total":page.total.to_string(), "cursor":page.cursor.map(|cursor| cursor.to_string()),
                "rows": page.rows.into_iter().map(|row| json!({"key":row.key.to_string(), "hour":row.hour, "ordinal":row.ordinal.to_string(), "row":row.row})).collect::<Vec<_>>()}),
            )
        }
        "events-clusters" => {
            let (from, to, width) = series::viewport(
                raw["from"].as_f64(),
                raw["to"].as_f64(),
                raw["width"]
                    .as_u64()
                    .and_then(|width| u32::try_from(width).ok()),
            )?;
            let mut lines = Vec::new();
            for actor in actors {
                lines.push(json!({"key":actor.to_string(), "summary":reader.event_clusters(actor, from, to, width)?}));
            }
            Ok(json!({"series":lines}))
        }
        "series-state" | "series-summary" => {
            let reads: Vec<i32> = raw["reads"].as_array().map_or_else(Vec::new, |reads| {
                reads
                    .iter()
                    .filter_map(|read| {
                        read["hour"]
                            .as_i64()
                            .and_then(|hour| i32::try_from(hour).ok())
                    })
                    .collect()
            });
            let mut reads = reads;
            let head = reads
                .first()
                .copied()
                .or_else(|| raw["to"].as_f64().map(|hour| hour.floor() as i32))
                .ok_or_else(|| dashboard_core::invalid("Missing equipment loss hour"))?;
            reads.push(head);
            reads.sort_unstable();
            reads.dedup();
            query(
                source,
                &json!({"selection":selection, "from":raw["from"].as_f64().unwrap_or(f64::from(head)), "to":raw["to"].as_f64().unwrap_or(f64::from(head)),
                "width":raw["width"].as_u64().unwrap_or(1), "reads":reads, "headHour":head, "detailHour":head}),
                context,
                cancelled,
            )
        }
        _ => Err(dashboard_core::invalid(
            "Unknown equipment-loss series query",
        )),
    }
}

#[cfg(test)]
mod tests;
