//! Current resolved native joins; never part of the saved accounting history.
use super::*;
use dashboard_base::prepared::frame_changes::{FrameChanges, field_mask};
use replay::frame::{SharedView, TableView};
use replay::rows::{DeltaRow, TableSchema};
use std::sync::LazyLock;

/// Each country's row positions in table order.
type Index = BTreeMap<u32, Vec<usize>>;

#[derive(Clone, Default)]
pub(super) struct Inputs {
    pub countries: BTreeMap<u32, CountryRow>,
    pub census: Vec<CountryRow>,
    pub naming_ids: BTreeMap<u32, u32>,
    pub tag_slots: BTreeMap<String, BTreeSet<u32>>,
    pub refused: Vec<CountryRow>,
    pub refused_reasons: BTreeMap<u32, String>,
    pub links: BTreeMap<u32, u32>,
    pub witnesses: BTreeMap<u32, PoliticsCaptureCoverageRow>,
    pub deadlines: BTreeMap<i32, BTreeSet<u32>>,
    pub by_slot: BTreeMap<u32, PoliticsCaptureCoverageRow>,
    pub sums: BTreeMap<(u32, u32, u64, i32, u64), i64>,
    pub applications: BTreeSet<(u32, u32, u64, i32, u64)>,
    pub unreconciled: BTreeSet<u32>,
    pub unreconciled_counts: BTreeMap<u32, u64>,
    pub daily_mutations: BTreeMap<u32, BTreeSet<i32>>,
    pub unknown_additions: BTreeMap<u32, BTreeMap<(u32, u64), u8>>,
    /// The event tables as decoded, held rather than copied.
    power: Option<SharedView>,
    governments: Option<SharedView>,
    daily: Option<SharedView>,
    costs: Option<SharedView>,
    pub daily_keys: BTreeSet<(u32, u32, u64, i32)>,
    pub cost_keys: BTreeSet<(u32, u32, u64, i32, u32)>,
    pub countries_changed: bool,
    pub power_changed: bool,
    pub governments_changed: bool,
    pub daily_changed: bool,
    pub costs_changed: bool,
    pub identity_changed: bool,
    pub changed: bool,
    pub slots: BTreeSet<u32>,
    pub previous_tags: BTreeSet<String>,
    pub identity_slots: BTreeSet<u32>,
    identity_countries: BTreeSet<u32>,
    raw_slots: BTreeMap<u32, BTreeSet<u32>>,
    /// Row positions by country; none after a table changed by its rows,
    /// whose decoded rows ascend by key and so by country.
    power_index: Option<Index>,
    government_index: Option<Index>,
    daily_index: Option<Index>,
    cost_index: Option<Index>,
    character_slots: BTreeSet<u32>,
    pub command: BTreeMap<u32, i64>,
    complete: [bool; 4],
    lexical_complete: bool,
    keys: Option<dashboard_text::metadata::CountryKeys>,
    /// Whether this hour was joined from its changed rows alone.
    pub narrow: bool,
    /// Countries whose [power, government, daily, cost] rows are observed
    /// again this hour besides the identity countries.
    touched: [BTreeSet<u32>; 4],
    /// Each country's latest power row hour.
    latest_power: BTreeMap<u32, i32>,
    /// The countries with a daily political gain at each hour.
    mutation_hours: BTreeMap<i32, BTreeSet<u32>>,
    /// The countries whose [power, government] rows changed this hour, when
    /// it was joined from its changed rows; none when the whole tables were.
    pub changed_rows: Option<[BTreeSet<u32>; 2]>,
    /// How many [daily upkeep or gain power, government] rows date at each
    /// native day: the newest is the newest native day they publish.
    days: [BTreeMap<i32, usize>; 2],
    #[cfg(any(test, feature = "test-support"))]
    pub row_visits: usize,
}

/// The Countries fields a census reads; its id is the row key.
static CENSUS_FIELDS: LazyLock<u64> =
    LazyLock::new(|| field_mask(TableId::Countries, &["canonical_slot", "tag"]));

/// A held table's rows; none unless it is complete, as a failed read gives.
fn held<T: TableSchema>(view: &Option<SharedView>) -> &[T::Row] {
    match view.as_deref() {
        Some(TableView::Complete(rows)) => rows.rows::<T>().unwrap_or(&[]),
        _ => &[],
    }
}

/// The table's changed rows and deleted keys this hour. A table that stays
/// unreadable while its rows were already none changes nothing.
fn table_delta<'a>(
    changes: &'a FrameChanges,
    frame: &DecodedFrame,
    table: TableId,
    none_held: bool,
) -> Option<(&'a [DeltaRow], &'a [u128])> {
    changes.rows(table).or_else(|| {
        (none_held && !matches!(*frame.tables[table.index()], TableView::Complete(_)))
            .then_some((&[][..], &[][..]))
    })
}

/// The hours at which a capture's relation to the current hour changes: its
/// first hour, its last, and the first after it.
fn witness_hours(row: &PoliticsCaptureCoverageRow) -> [Option<i32>; 3] {
    [
        Some(row.since_hour),
        Some(row.through_hour),
        row.through_hour.checked_add(1),
    ]
}

fn schedule(deadlines: &mut BTreeMap<i32, BTreeSet<u32>>, raw: u32, hours: &[Option<i32>; 3]) {
    for &hour in hours.iter().flatten() {
        deadlines.entry(hour).or_default().insert(raw);
    }
}

fn unschedule(deadlines: &mut BTreeMap<i32, BTreeSet<u32>>, raw: u32, hours: &[Option<i32>; 3]) {
    for hour in hours.iter().flatten() {
        if let Some(raws) = deadlines.get_mut(hour) {
            raws.remove(&raw);
            if raws.is_empty() {
                deadlines.remove(hour);
            }
        }
    }
}

/// The native day a power row closes: daily upkeep and gains only.
fn power_day(row: &CountryPowerChangeRow) -> Option<i32> {
    matches!(row.cause, PowerCause::AbilityUpkeep | PowerCause::DailyGain)
        .then(|| row.hour.div_euclid(24) * 24)
}

fn government_day(row: &GovernmentChangeRow) -> Option<i32> {
    Some(row.hour.div_euclid(24) * 24)
}

/// Count each row's native day in or out.
fn count_days<'a, T: 'a>(
    days: &mut BTreeMap<i32, usize>,
    rows: impl IntoIterator<Item = &'a T>,
    day: fn(&T) -> Option<i32>,
    add: bool,
) {
    for day in rows.into_iter().filter_map(day) {
        if add {
            *days.entry(day).or_default() += 1;
        } else if let Some(count) = days.get_mut(&day) {
            *count -= 1;
            if *count == 0 {
                days.remove(&day);
            }
        }
    }
}

/// Whether a changed capture only extends its interval from the previous
/// hour to this one, with the same identity, start and streams.
fn advances(old: &PoliticsCaptureCoverageRow, row: &PoliticsCaptureCoverageRow, hour: i32) -> bool {
    (old.instance, old.epoch, old.since_hour, old.streams)
        == (row.instance, row.epoch, row.since_hour, row.streams)
        && old.through_hour.checked_add(1) == Some(row.through_hour)
        && row.through_hour == hour
}

/// Move the deadlines of the captures that advanced to `hour` (each country
/// with its first hour) exactly as unscheduling each one's old hours and
/// scheduling its first hour and its next expiry does, one capture at a
/// time: each leaves the hour before (its old last hour), unless it began
/// then, and its expiry at `hour`, keeps its first hour, and expires at the
/// hour after. When every capture due at `hour` advanced, their expiries
/// move together.
fn advance_deadlines(
    deadlines: &mut BTreeMap<i32, BTreeSet<u32>>,
    advanced: &mut [(u32, i32)],
    hour: i32,
) {
    if advanced.is_empty() {
        return;
    }
    advanced.sort_unstable_by_key(|&(raw, _)| raw);
    let advanced = &*advanced;
    let first_hour = |raw: &u32| {
        advanced
            .binary_search_by_key(raw, |&(raw, _)| raw)
            .ok()
            .map(|at| advanced[at].1)
    };
    if let Some(last) = hour.checked_sub(1)
        && let Some(raws) = deadlines.get_mut(&last)
    {
        raws.retain(|raw| first_hour(raw).is_none_or(|since| since == last));
        if raws.is_empty() {
            deadlines.remove(&last);
        }
    }
    let mut moving = match deadlines.remove(&hour) {
        Some(due)
            if due.len() == advanced.len()
                && due.iter().zip(advanced).all(|(due, (raw, _))| due == raw) =>
        {
            due
        }
        due => {
            if let Some(mut due) = due {
                for (raw, _) in advanced {
                    due.remove(raw);
                }
                if !due.is_empty() {
                    deadlines.insert(hour, due);
                }
            }
            advanced.iter().map(|&(raw, _)| raw).collect()
        }
    };
    if let Some(next) = hour.checked_add(1) {
        deadlines.entry(next).or_default().append(&mut moving);
    }
}

impl Inputs {
    pub(super) fn power(&self) -> &[CountryPowerChangeRow] {
        held::<tables::CountryPowerChanges>(&self.power)
    }
    pub(super) fn governments(&self) -> &[GovernmentChangeRow] {
        held::<tables::GovernmentChanges>(&self.governments)
    }
    pub(super) fn daily(&self) -> &[CountryPowerDailyRow] {
        held::<tables::CountryPowerDaily>(&self.daily)
    }
    pub(super) fn costs(&self) -> &[CountryPowerDailyCostRow] {
        held::<tables::CountryPowerDailyCosts>(&self.costs)
    }
    /// The newest native day of the [daily upkeep or gain power, government]
    /// rows held.
    pub(super) fn newest_day(&self, table: usize) -> Option<i32> {
        self.days[table].last_key_value().map(|(&day, _)| day)
    }
    /// One country's power rows, in table order.
    pub(super) fn power_of(&self, country: u32) -> impl Iterator<Item = &CountryPowerChangeRow> {
        country_rows(self.power(), self.power_index.as_ref(), country, |row| {
            row.country
        })
    }
    /// One country's government rows, in table order.
    pub(super) fn governments_of(
        &self,
        country: u32,
    ) -> impl Iterator<Item = &GovernmentChangeRow> {
        country_rows(
            self.governments(),
            self.government_index.as_ref(),
            country,
            |row| row.country,
        )
    }
    #[cfg(test)]
    pub(super) fn native_days(&self) -> &[BTreeMap<i32, usize>; 2] {
        &self.days
    }
    /// The countries whose power or cost rows this hour observes.
    pub(super) fn observed(&self, table: usize) -> impl Iterator<Item = &u32> {
        self.identity_countries.union(&self.touched[table])
    }
    pub(super) fn power_rows(&self) -> impl Iterator<Item = &CountryPowerChangeRow> {
        selected_rows(
            self.power(),
            self.power_index.as_ref(),
            self.power_changed && !self.narrow,
            &self.identity_countries,
            &self.touched[0],
            |row| row.country,
        )
    }
    pub(super) fn government_rows(&self) -> impl Iterator<Item = &GovernmentChangeRow> {
        selected_rows(
            self.governments(),
            self.government_index.as_ref(),
            self.governments_changed && !self.narrow,
            &self.identity_countries,
            &self.touched[1],
            |row| row.country,
        )
    }
    pub(super) fn daily_rows(&self) -> impl Iterator<Item = &CountryPowerDailyRow> {
        selected_rows(
            self.daily(),
            self.daily_index.as_ref(),
            self.daily_changed && !self.narrow,
            &self.identity_countries,
            &self.touched[2],
            |row| row.country,
        )
    }
    pub(super) fn cost_rows(&self) -> impl Iterator<Item = &CountryPowerDailyCostRow> {
        selected_rows(
            self.costs(),
            self.cost_index.as_ref(),
            self.costs_changed && !self.narrow,
            &self.identity_countries,
            &self.touched[3],
            |row| row.country,
        )
    }
    pub(super) fn identity_rows(&self) -> impl Iterator<Item = &PoliticsCaptureCoverageRow> {
        self.identity_countries
            .iter()
            .filter_map(|raw| self.witnesses.get(raw))
    }

    /// Whether this hour was joined from its changed rows and they join no
    /// country: no census, link, capture identity, power, government,
    /// allocation, completeness, daily gain or naming change selects one,
    /// and no row is observed again. Its captures at most extend their
    /// intervals to the hour.
    pub(super) fn joins_nothing(&self) -> bool {
        self.narrow
            && !self.countries_changed
            && self.slots.is_empty()
            && self.previous_tags.is_empty()
            && self.identity_slots.is_empty()
            && self.identity_countries.is_empty()
            && self.touched.iter().all(BTreeSet::is_empty)
    }

    /// The hour's joins: from its changed rows when every table it reads
    /// changed by one recorded delta, otherwise from the whole tables.
    /// `pending` holds the power and cost rows not yet recorded as events.
    pub(super) fn refresh_changes(
        &mut self,
        frame: &DecodedFrame,
        changes: &FrameChanges,
        pending: Option<&[BTreeSet<u128>; 2]>,
    ) {
        if !pending.is_some_and(|pending| self.refresh_rows(frame, changes, pending)) {
            self.refresh(frame, Some(changes));
        }
    }

    pub(super) fn refresh(&mut self, frame: &DecodedFrame, changes: Option<&FrameChanges>) {
        let read = |table: TableId| {
            changes.is_none_or(|changes| {
                changes.rows_changed[table.index()] || changes.table_read[table.index()]
            })
        };
        self.narrow = false;
        self.changed_rows = None;
        self.touched = Default::default();
        self.slots.clear();
        self.previous_tags.clear();
        self.identity_slots.clear();
        self.identity_countries.clear();
        self.countries_changed = false;
        if read(TableId::Countries) {
            let rows = frame.complete::<tables::Countries>();
            #[cfg(any(test, feature = "test-support"))]
            {
                self.row_visits += rows.as_ref().map_or(0, |rows| rows.len());
            }
            if !rows
                .as_ref()
                .is_ok_and(|rows| self.keys.as_ref().is_some_and(|keys| keys.same(rows)))
            {
                self.countries_changed = true;
                self.previous_tags.extend(
                    self.countries
                        .values()
                        .filter_map(|row| valid_recorded_tag(row).ok().map(str::to_owned)),
                );
                self.keys = rows
                    .as_ref()
                    .ok()
                    .map(|rows| dashboard_text::metadata::CountryKeys::read(rows));
                self.census = rows
                    .unwrap_or_default()
                    .iter()
                    .map(|row| CountryRow {
                        id: row.id,
                        canonical_slot: row.canonical_slot,
                        tag: row.tag.clone(),
                        ..Default::default()
                    })
                    .collect();
                self.naming_ids = self
                    .census
                    .iter()
                    .map(|row| (row.canonical_slot, row.id))
                    .collect();
                self.refused = self
                    .census
                    .iter()
                    .filter(|row| valid_recorded_tag(row).is_err())
                    .cloned()
                    .collect();
                self.refused_reasons = self
                    .refused
                    .iter()
                    .filter_map(|row| {
                        valid_recorded_tag(row)
                            .err()
                            .map(|reason| (row.canonical_slot, format!("{reason} (count: 1)")))
                    })
                    .collect();
                self.countries = self
                    .census
                    .iter()
                    .filter(|row| valid_recorded_tag(row).is_ok())
                    .map(|row| (row.canonical_slot, row.clone()))
                    .collect();
                self.tag_slots.clear();
                for row in self.countries.values() {
                    self.tag_slots
                        .entry(valid_recorded_tag(row).unwrap().to_owned())
                        .or_default()
                        .insert(row.canonical_slot);
                }
            }
        }
        let links = read(TableId::CountryLinks);
        if links {
            let rows = frame.complete::<tables::CountryLinks>().unwrap_or_default();
            #[cfg(any(test, feature = "test-support"))]
            {
                self.row_visits += rows.len();
            }
            self.identity_slots.extend(self.links.values().copied());
            self.identity_countries.extend(self.links.keys().copied());
            self.links = rows
                .iter()
                .map(|row| (row.id, row.canonical_slot))
                .collect();
            self.identity_slots.extend(self.links.values().copied());
            self.identity_countries.extend(self.links.keys().copied());
            self.raw_slots.clear();
            for (&raw, &slot) in &self.links {
                self.raw_slots.entry(slot).or_default().insert(raw);
            }
        }
        let coverage = read(TableId::PoliticsCaptureCoverage);
        if coverage {
            let rows = frame
                .complete::<tables::PoliticsCaptureCoverage>()
                .unwrap_or_default();
            #[cfg(any(test, feature = "test-support"))]
            {
                self.row_visits += rows.len();
            }
            self.identity_countries
                .extend(self.witnesses.keys().copied());
            self.identity_countries
                .extend(rows.iter().map(|row| row.country));
            self.identity_slots.extend(
                self.identity_countries
                    .iter()
                    .filter_map(|raw| self.links.get(raw))
                    .copied(),
            );
            self.witnesses = rows.iter().map(|row| (row.country, row.clone())).collect();
            self.deadlines.clear();
            for row in rows {
                schedule(&mut self.deadlines, row.country, &witness_hours(row));
            }
        }
        self.identity_changed = self.countries_changed || links || coverage;
        if self.countries_changed {
            self.identity_slots.extend(self.countries.keys().copied());
            self.identity_countries.extend(
                self.identity_slots
                    .iter()
                    .filter_map(|slot| self.raw_slots.get(slot))
                    .flatten()
                    .copied(),
            );
        }
        self.slots.extend(self.identity_slots.iter().copied());
        if links || coverage {
            self.join_witnesses();
        }
        self.power_changed = read(TableId::CountryPowerChanges);
        self.governments_changed = read(TableId::GovernmentChanges);
        self.daily_changed = read(TableId::CountryPowerDaily);
        self.costs_changed = read(TableId::CountryPowerDailyCosts);
        let mut daily_owners = BTreeSet::new();
        if self.daily_changed {
            daily_owners.extend(owners(self.daily(), &self.daily_index, |row| row.country));
        }
        if self.costs_changed {
            daily_owners.extend(owners(self.costs(), &self.cost_index, |row| row.country));
        }
        let mut power_owners = BTreeSet::new();
        if self.power_changed {
            power_owners.extend(owners(self.power(), &self.power_index, |row| row.country));
        }
        if self.power_changed {
            self.slots.extend(
                held::<tables::CountryPowerChanges>(&self.power)
                    .iter()
                    .filter_map(|row| self.links.get(&row.country).copied()),
            );
            self.power = Some(frame.tables[TableId::CountryPowerChanges.index()].clone());
            let index = index_rows(self.power(), |row| row.country);
            power_owners.extend(index.keys().copied());
            self.power_index = Some(index);
            #[cfg(any(test, feature = "test-support"))]
            {
                self.row_visits += self.power().len();
            }
            self.slots.extend(
                held::<tables::CountryPowerChanges>(&self.power)
                    .iter()
                    .filter_map(|row| self.links.get(&row.country).copied()),
            );
            self.days[0].clear();
            count_days(
                &mut self.days[0],
                held::<tables::CountryPowerChanges>(&self.power),
                power_day,
                true,
            );
            self.daily_mutations.clear();
            self.mutation_hours.clear();
            self.latest_power.clear();
            let countries: Vec<_> = self
                .power_index
                .iter()
                .flat_map(|index| index.keys())
                .copied()
                .collect();
            self.index_power(countries);
        }
        if self.governments_changed {
            self.governments = Some(frame.tables[TableId::GovernmentChanges.index()].clone());
            self.government_index = Some(index_rows(self.governments(), |row| row.country));
            self.days[1].clear();
            count_days(
                &mut self.days[1],
                held::<tables::GovernmentChanges>(&self.governments),
                government_day,
                true,
            );
            #[cfg(any(test, feature = "test-support"))]
            {
                self.row_visits += self.governments().len();
            }
            self.slots.extend(
                held::<tables::GovernmentChanges>(&self.governments)
                    .iter()
                    .filter_map(|row| self.links.get(&row.country).copied()),
            );
        }
        if self.daily_changed {
            self.slots.extend(
                held::<tables::CountryPowerDaily>(&self.daily)
                    .iter()
                    .filter_map(|row| self.links.get(&row.country).copied()),
            );
            self.daily = Some(frame.tables[TableId::CountryPowerDaily.index()].clone());
            let index = index_rows(self.daily(), |row| row.country);
            daily_owners.extend(index.keys().copied());
            self.daily_index = Some(index);
            #[cfg(any(test, feature = "test-support"))]
            {
                self.row_visits += self.daily().len();
            }
            let daily = held::<tables::CountryPowerDaily>(&self.daily);
            self.daily_keys = daily
                .iter()
                .map(|row| (row.country, row.instance, row.epoch, row.day_hour))
                .collect();
            self.applications = daily
                .iter()
                .filter_map(|row| {
                    row.pp_sequence.map(|sequence| {
                        (row.country, row.instance, row.epoch, row.day_hour, sequence)
                    })
                })
                .collect();
            self.slots.extend(
                daily
                    .iter()
                    .filter_map(|row| self.links.get(&row.country).copied()),
            );
        }
        if self.costs_changed {
            self.slots.extend(
                held::<tables::CountryPowerDailyCosts>(&self.costs)
                    .iter()
                    .filter_map(|row| self.links.get(&row.country).copied()),
            );
            self.costs = Some(frame.tables[TableId::CountryPowerDailyCosts.index()].clone());
            let index = index_rows(self.costs(), |row| row.country);
            daily_owners.extend(index.keys().copied());
            self.cost_index = Some(index);
            #[cfg(any(test, feature = "test-support"))]
            {
                self.row_visits += self.costs().len();
            }
            let costs = held::<tables::CountryPowerDailyCosts>(&self.costs);
            self.cost_keys = costs
                .iter()
                .map(|row| {
                    (
                        row.country,
                        row.instance,
                        row.epoch,
                        row.day_hour,
                        row.ordinal,
                    )
                })
                .collect();
            self.sums.clear();
            for row in costs {
                let sum = self
                    .sums
                    .entry((
                        row.country,
                        row.instance,
                        row.epoch,
                        row.day_hour,
                        row.pp_sequence,
                    ))
                    .or_default();
                *sum = sum.wrapping_add(row.cost_raw);
            }
            self.slots.extend(
                costs
                    .iter()
                    .filter_map(|row| self.links.get(&row.country).copied()),
            );
        }
        self.observe_completeness(frame);
        if read(TableId::CountryCommandPower) {
            let rows = frame
                .complete::<tables::CountryCommandPower>()
                .unwrap_or_default();
            #[cfg(any(test, feature = "test-support"))]
            {
                self.row_visits += rows.len();
            }
            let next: BTreeMap<_, _> = rows
                .iter()
                .map(|row| (row.canonical_slot, row.allocated_command_power_raw))
                .collect();
            self.slots.extend(
                self.command
                    .iter()
                    .filter(|&(slot, amount)| next.get(slot) != Some(amount))
                    .map(|(slot, _)| *slot),
            );
            self.slots.extend(
                next.iter()
                    .filter(|&(slot, amount)| self.command.get(slot) != Some(amount))
                    .map(|(slot, _)| *slot),
            );
            self.command = next;
        }
        if read(TableId::Characters) {
            let rows = frame.complete::<tables::Characters>().unwrap_or_default();
            #[cfg(any(test, feature = "test-support"))]
            {
                self.row_visits += rows.len();
            }
            self.slots.extend(self.character_slots.iter().copied());
            self.character_slots = rows
                .iter()
                .filter_map(|row| self.links.get(&row.country).copied())
                .collect();
            self.slots.extend(self.character_slots.iter().copied());
        }
        if read(TableId::LexicalTokens) {
            self.slots.extend(self.countries.keys().copied());
        }
        if self.daily_changed || self.costs_changed {
            for country in daily_owners {
                let previous = self.unreconciled.remove(&country);
                for row in country_rows(
                    held::<tables::CountryPowerDaily>(&self.daily),
                    self.daily_index.as_ref(),
                    country,
                    |row| row.country,
                ) {
                    if let Some(sequence) = row.pp_sequence {
                        let key = (row.country, row.instance, row.epoch, row.day_hour, sequence);
                        if row
                            .pp_cost_raw
                            .is_none_or(|cost| self.sums.get(&key).copied().unwrap_or(0) != cost)
                        {
                            self.unreconciled.insert(row.country);
                        }
                    }
                }
                if !links
                    && previous != self.unreconciled.contains(&country)
                    && let Some(slot) = self.links.get(&country)
                {
                    let count = self.unreconciled_counts.entry(*slot).or_default();
                    if previous {
                        *count -= 1;
                    } else {
                        *count += 1;
                    }
                }
            }
        }
        if links {
            self.unreconciled_counts.clear();
            for (&raw, &slot) in &self.links {
                if self.unreconciled.contains(&raw) {
                    *self.unreconciled_counts.entry(slot).or_default() += 1;
                }
            }
        }
        if self.power_changed || coverage {
            power_owners.extend(self.identity_countries.iter().copied());
            self.reread_additions(power_owners);
        }
        self.changed = self.identity_changed
            || self.power_changed
            || self.governments_changed
            || self.daily_changed
            || self.costs_changed
            || [
                TableId::CountryCommandPower,
                TableId::Characters,
                TableId::CharacterPortraits,
                TableId::LexicalTokens,
            ]
            .into_iter()
            .any(read);
    }

    /// [`Self::refresh`] from the hour's changed rows, joining again only the
    /// countries whose rows changed. A capture that only extends its interval
    /// to this hour reads as before and joins nothing. Rows already recorded
    /// as events are skipped when observed, so only `pending` rows of the
    /// whole tables the full refresh would observe are observed again.
    /// False, changing nothing, when a table read was rebuilt rather than
    /// changed by one recorded delta, or a census key, link, dictionary or
    /// daily publication changed: the caller then refreshes the whole tables.
    fn refresh_rows(
        &mut self,
        frame: &DecodedFrame,
        changes: &FrameChanges,
        pending: &[BTreeSet<u128>; 2],
    ) -> bool {
        let read = |table: TableId| {
            changes.rows_changed[table.index()] || changes.table_read[table.index()]
        };
        let unchanged = |delta: Option<(&[DeltaRow], &[u128])>| {
            delta.is_some_and(|(changed, deleted)| changed.is_empty() && deleted.is_empty())
        };
        // A census read keeps its keys: no country is added, deleted or
        // given another slot or tag.
        if read(TableId::Countries) {
            let Some((changed, deleted)) = changes.rows(TableId::Countries) else {
                return false;
            };
            if self.keys.is_none()
                || frame.complete::<tables::Countries>().is_err()
                || !deleted.is_empty()
                || changed.iter().any(|row| {
                    row.mask & *CENSUS_FIELDS != 0
                        || self
                            .census
                            .binary_search_by_key(&row.key, |country| u128::from(country.id))
                            .is_err()
                })
            {
                return false;
            }
        }
        if read(TableId::CountryLinks)
            && !unchanged(table_delta(
                changes,
                frame,
                TableId::CountryLinks,
                self.links.is_empty(),
            ))
        {
            return false;
        }
        let lexical_kept = changes.rows(TableId::LexicalTokens).is_none()
            && !self.lexical_complete
            && !matches!(
                *frame.tables[TableId::LexicalTokens.index()],
                TableView::Complete(_)
            );
        if read(TableId::LexicalTokens)
            && !unchanged(changes.rows(TableId::LexicalTokens))
            && !lexical_kept
        {
            return false;
        }
        for (table, none_held) in [
            (TableId::CountryPowerDaily, self.daily().is_empty()),
            (TableId::CountryPowerDailyCosts, self.costs().is_empty()),
        ] {
            if read(table) && !unchanged(table_delta(changes, frame, table, none_held)) {
                return false;
            }
        }
        let (Some(coverage), Some(power), Some(governments), Some(command), Some(_characters)) = (
            table_delta(
                changes,
                frame,
                TableId::PoliticsCaptureCoverage,
                self.witnesses.is_empty(),
            ),
            table_delta(
                changes,
                frame,
                TableId::CountryPowerChanges,
                self.power().is_empty(),
            ),
            table_delta(
                changes,
                frame,
                TableId::GovernmentChanges,
                self.governments().is_empty(),
            ),
            table_delta(
                changes,
                frame,
                TableId::CountryCommandPower,
                self.command.is_empty(),
            ),
            table_delta(
                changes,
                frame,
                TableId::Characters,
                self.character_slots.is_empty(),
            ),
        ) else {
            return false;
        };
        // Read every changed row before changing anything.
        let mut captures = Vec::with_capacity(coverage.0.len());
        if !coverage.0.is_empty() {
            let Ok(rows) = frame.complete::<tables::PoliticsCaptureCoverage>() else {
                return false;
            };
            // Changed rows usually follow the table's key order: try the row
            // after the last one found before searching.
            let mut next = 0;
            for change in coverage.0 {
                let at = match rows.get(next) {
                    Some(row) if row.key() == change.key => next,
                    _ => match rows.binary_search_by_key(&change.key, |row| row.key()) {
                        Ok(at) => at,
                        Err(_) => return false,
                    },
                };
                captures.push(&rows[at]);
                next = at + 1;
            }
        }
        let mut allocations = Vec::with_capacity(command.0.len());
        if !command.0.is_empty() {
            let Ok(rows) = frame.complete::<tables::CountryCommandPower>() else {
                return false;
            };
            for change in command.0 {
                let Ok(at) = rows.binary_search_by_key(&change.key, |row| row.key()) else {
                    return false;
                };
                allocations.push((
                    rows[at].canonical_slot,
                    rows[at].allocated_command_power_raw,
                ));
            }
        }
        #[cfg(test)]
        {
            self.row_visits +=
                captures.len() + allocations.len() + power.0.len() + governments.0.len();
        }
        self.narrow = true;
        self.touched = Default::default();
        self.slots.clear();
        self.previous_tags.clear();
        self.identity_slots.clear();
        self.identity_countries.clear();
        self.countries_changed = false;
        let links = read(TableId::CountryLinks);
        let coverage_read = read(TableId::PoliticsCaptureCoverage);
        for &key in coverage.1 {
            let raw = key as u32;
            if let Some(old) = self.witnesses.remove(&raw) {
                unschedule(&mut self.deadlines, raw, &witness_hours(&old));
            }
            self.identity_countries.insert(raw);
        }
        // A capture that only extends its interval keeps its witness, whose
        // last hour moves in place, and only its expiry moves, to the hour
        // after this one: every such capture's together
        // ([`advance_deadlines`]).
        let mut advanced = Vec::new();
        for row in captures {
            let raw = row.country;
            if let Some(old) = self.witnesses.get_mut(&raw) {
                // No power row dated after the old interval: the extension adds
                // no event, daily gain or unknown addition to it.
                if advances(old, row, frame.hour)
                    && self
                        .latest_power
                        .get(&raw)
                        .is_none_or(|&latest| latest <= old.through_hour)
                {
                    advanced.push((raw, old.since_hour));
                    old.through_hour = row.through_hour;
                    if let Some(slot) = self.links.get(&raw)
                        && let Some(witness) = self.by_slot.get_mut(slot)
                        && witness.country == raw
                    {
                        witness.through_hour = row.through_hour;
                    }
                    continue;
                }
                unschedule(&mut self.deadlines, raw, &witness_hours(old));
            }
            schedule(&mut self.deadlines, raw, &witness_hours(row));
            self.identity_countries.insert(raw);
            self.witnesses.insert(raw, row.clone());
        }
        advance_deadlines(&mut self.deadlines, &mut advanced, frame.hour);
        self.identity_slots.extend(
            self.identity_countries
                .iter()
                .filter_map(|raw| self.links.get(raw))
                .copied(),
        );
        self.identity_changed = links || coverage_read;
        self.slots.extend(self.identity_slots.iter().copied());
        if !self.identity_slots.is_empty() {
            self.join_witnesses();
        }
        self.power_changed = read(TableId::CountryPowerChanges);
        self.governments_changed = read(TableId::GovernmentChanges);
        self.daily_changed = read(TableId::CountryPowerDaily);
        self.costs_changed = read(TableId::CountryPowerDailyCosts);
        if self.power_changed {
            let touched: BTreeSet<u32> = power
                .0
                .iter()
                .map(|row| row.key)
                .chain(power.1.iter().copied())
                .map(|key| (key >> 64) as u32)
                .collect();
            for &country in &touched {
                let rows = country_rows(
                    held::<tables::CountryPowerChanges>(&self.power),
                    self.power_index.as_ref(),
                    country,
                    |row| row.country,
                );
                count_days(&mut self.days[0], rows, power_day, false);
            }
            self.power = Some(frame.tables[TableId::CountryPowerChanges.index()].clone());
            if !touched.is_empty() {
                self.power_index = None;
                for &country in &touched {
                    let rows = country_rows(
                        held::<tables::CountryPowerChanges>(&self.power),
                        None,
                        country,
                        |row| row.country,
                    );
                    count_days(&mut self.days[0], rows, power_day, true);
                }
                self.index_power(touched.iter().copied());
                self.slots.extend(
                    touched
                        .iter()
                        .filter_map(|raw| self.links.get(raw))
                        .copied(),
                );
                self.touched[0] = touched;
            }
        }
        let mut changed_rows: [BTreeSet<u32>; 2] = Default::default();
        changed_rows[0].clone_from(&self.touched[0]);
        if self.governments_changed {
            let touched: BTreeSet<u32> = governments
                .0
                .iter()
                .map(|row| row.key)
                .chain(governments.1.iter().copied())
                .map(|key| (key >> 64) as u32)
                .collect();
            for &country in &touched {
                let rows = country_rows(
                    held::<tables::GovernmentChanges>(&self.governments),
                    self.government_index.as_ref(),
                    country,
                    |row| row.country,
                );
                count_days(&mut self.days[1], rows, government_day, false);
            }
            self.governments = Some(frame.tables[TableId::GovernmentChanges.index()].clone());
            if !touched.is_empty() {
                self.government_index = None;
                for &country in &touched {
                    let rows = country_rows(
                        held::<tables::GovernmentChanges>(&self.governments),
                        None,
                        country,
                        |row| row.country,
                    );
                    count_days(&mut self.days[1], rows, government_day, true);
                }
                self.slots.extend(
                    touched
                        .iter()
                        .filter_map(|raw| self.links.get(raw))
                        .copied(),
                );
                self.touched[1] = touched;
            }
        }
        changed_rows[1].clone_from(&self.touched[1]);
        self.changed_rows = Some(changed_rows);
        // Unchanged daily publications read as before.
        if self.daily_changed {
            self.daily = Some(frame.tables[TableId::CountryPowerDaily.index()].clone());
        }
        if self.costs_changed {
            self.costs = Some(frame.tables[TableId::CountryPowerDailyCosts.index()].clone());
        }
        // The whole tables observe every row, and identity reads every row
        // of a linked or captured country, again; only rows not yet recorded
        // as events can change anything.
        for (index, keys, shift, whole) in [
            (0, &pending[0], 64, self.power_changed),
            (3, &pending[1], 32, self.costs_changed),
        ] {
            for &key in keys {
                let raw = (key >> shift) as u32;
                if whole
                    || (links && self.links.contains_key(&raw))
                    || (coverage_read
                        && (self.witnesses.contains_key(&raw)
                            || coverage.1.contains(&u128::from(raw))))
                {
                    self.touched[index].insert(raw);
                    if let Some(&slot) = self.links.get(&raw) {
                        self.slots.insert(slot);
                    }
                }
            }
        }
        // A country without a capture of this hour counts the daily gains of
        // this hour only, and one with it counts those since its start: both
        // can change at a gain's hour and the hour after.
        for hour in [frame.hour.checked_sub(1), Some(frame.hour)]
            .into_iter()
            .flatten()
        {
            for &raw in self.mutation_hours.get(&hour).into_iter().flatten() {
                // A capture of this hour that began by the gain still counts it.
                if hour != frame.hour
                    && self
                        .witnesses
                        .get(&raw)
                        .is_some_and(|row| row.through_hour == frame.hour && row.since_hour <= hour)
                {
                    continue;
                }
                // Without a capture the gain is counted under the census id.
                let slot = self.links.get(&raw).copied().or_else(|| {
                    self.census
                        .binary_search_by_key(&raw, |row| row.id)
                        .ok()
                        .map(|at| self.census[at].canonical_slot)
                });
                if let Some(slot) = slot {
                    self.slots.insert(slot);
                }
            }
        }
        self.observe_completeness(frame);
        for (slot, amount) in allocations {
            if self.command.insert(slot, amount) != Some(amount) {
                self.slots.insert(slot);
            }
        }
        for &key in command.1 {
            if self.command.remove(&(key as u32)).is_some() {
                self.slots.insert(key as u32);
            }
        }
        if self.power_changed || coverage_read {
            let owners = self.observed(0).copied().collect();
            self.reread_additions(owners);
        }
        self.changed = self.identity_changed
            || self.power_changed
            || self.governments_changed
            || self.daily_changed
            || self.costs_changed
            || [
                TableId::CountryCommandPower,
                TableId::Characters,
                TableId::CharacterPortraits,
                TableId::LexicalTokens,
            ]
            .into_iter()
            .any(read);
        true
    }

    /// Each identity slot's capture: that of its last linked country with one.
    fn join_witnesses(&mut self) {
        for slot in &self.identity_slots {
            self.by_slot.remove(slot);
            if let Some(row) = self
                .raw_slots
                .get(slot)
                .and_then(|raws| raws.iter().rev().find_map(|raw| self.witnesses.get(raw)))
            {
                self.by_slot.insert(*slot, row.clone());
            }
        }
    }

    /// A change in which event tables are readable observes every country.
    fn observe_completeness(&mut self, frame: &DecodedFrame) {
        let complete = [
            frame.complete::<tables::CountryPowerChanges>().is_ok(),
            frame.complete::<tables::CountryPowerDaily>().is_ok(),
            frame.complete::<tables::CountryPowerDailyCosts>().is_ok(),
            frame.complete::<tables::GovernmentChanges>().is_ok(),
        ];
        if self.complete != complete {
            self.slots.extend(self.countries.keys().copied());
            self.complete = complete;
        }
        self.lexical_complete = matches!(
            *frame.tables[TableId::LexicalTokens.index()],
            TableView::Complete(_)
        );
    }

    /// The countries' daily political gain hours and latest power row hour.
    fn index_power(&mut self, countries: impl IntoIterator<Item = u32>) {
        for country in countries {
            for hour in self.daily_mutations.remove(&country).into_iter().flatten() {
                if let Some(raws) = self.mutation_hours.get_mut(&hour) {
                    raws.remove(&country);
                    if raws.is_empty() {
                        self.mutation_hours.remove(&hour);
                    }
                }
            }
            self.latest_power.remove(&country);
            for row in country_rows(
                held::<tables::CountryPowerChanges>(&self.power),
                self.power_index.as_ref(),
                country,
                |row| row.country,
            ) {
                let latest = self.latest_power.entry(country).or_insert(row.hour);
                *latest = (*latest).max(row.hour);
                if row.cause == PowerCause::DailyGain && row.currency == PowerCurrency::Political {
                    self.daily_mutations
                        .entry(row.country)
                        .or_default()
                        .insert(row.hour);
                    self.mutation_hours
                        .entry(row.hour)
                        .or_default()
                        .insert(row.country);
                }
            }
        }
    }

    /// The owners' additions without an evaluated operand within their capture.
    fn reread_additions(&mut self, owners: BTreeSet<u32>) {
        let mut next = BTreeMap::<u32, BTreeMap<(u32, u64), u8>>::new();
        for row in self.power_rows() {
            if row.operation == PowerOperation::Add
                && row.requested_delta_raw.is_none()
                && !(row.cause == PowerCause::DailyGain && row.currency == PowerCurrency::Political)
                && self.witnesses.get(&row.country).is_some_and(|witness| {
                    witness.instance == row.instance
                        && witness.epoch == row.epoch
                        && row.hour >= witness.since_hour
                        && row.hour <= witness.through_hour
                })
            {
                *next
                    .entry(row.country)
                    .or_default()
                    .entry((row.instance, row.epoch))
                    .or_default() |= match row.currency {
                    PowerCurrency::Political => PoliticsCaptureCoverageRow::PP,
                    PowerCurrency::Command => PoliticsCaptureCoverageRow::CP,
                };
            }
        }
        for country in owners {
            self.unknown_additions.remove(&country);
            if let Some(values) = next.remove(&country) {
                self.unknown_additions.insert(country, values);
            }
        }
    }
}

fn index_rows<T>(rows: &[T], country: impl Fn(&T) -> u32) -> Index {
    let mut index = BTreeMap::<_, Vec<_>>::new();
    for (position, row) in rows.iter().enumerate() {
        index.entry(country(row)).or_default().push(position);
    }
    index
}

/// The countries holding rows.
fn owners<T>(rows: &[T], index: &Option<Index>, country: fn(&T) -> u32) -> BTreeSet<u32> {
    match index {
        Some(index) => index.keys().copied().collect(),
        None => rows.iter().map(country).collect(),
    }
}

/// One country's rows, in table order.
enum CountryRows<'a, T> {
    Indexed(std::slice::Iter<'a, usize>, &'a [T]),
    Sorted(std::slice::Iter<'a, T>),
}

impl<'a, T> Iterator for CountryRows<'a, T> {
    type Item = &'a T;
    fn next(&mut self) -> Option<&'a T> {
        match self {
            Self::Indexed(positions, rows) => {
                let rows: &'a [T] = *rows;
                positions.next().map(|&position| &rows[position])
            }
            Self::Sorted(rows) => rows.next(),
        }
    }
}

/// Without an index the rows are decoded, ascending by a key that leads
/// with the country, so each country's rows are one run.
fn country_rows<'a, T>(
    rows: &'a [T],
    index: Option<&'a Index>,
    country: u32,
    of: fn(&T) -> u32,
) -> CountryRows<'a, T> {
    match index {
        Some(index) => CountryRows::Indexed(
            index
                .get(&country)
                .map(|positions| positions.iter())
                .unwrap_or_default(),
            rows,
        ),
        None => {
            let start = rows.partition_point(|row| of(row) < country);
            let end = start + rows[start..].partition_point(|row| of(row) == country);
            CountryRows::Sorted(rows[start..end].iter())
        }
    }
}

fn selected_rows<'a, T>(
    rows: &'a [T],
    index: Option<&'a Index>,
    all: bool,
    countries: &'a BTreeSet<u32>,
    touched: &'a BTreeSet<u32>,
    of: fn(&T) -> u32,
) -> impl Iterator<Item = &'a T> {
    rows.iter().take(if all { rows.len() } else { 0 }).chain(
        countries
            .union(touched)
            .filter(move |_| !all)
            .flat_map(move |&country| country_rows(rows, index, country, of)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each advancing capture's old hours unscheduled and its first hour and
    /// next expiry scheduled, one capture at a time.
    fn one_at_a_time(
        deadlines: &mut BTreeMap<i32, BTreeSet<u32>>,
        advanced: &[(u32, i32)],
        hour: i32,
    ) {
        for &(raw, since) in advanced {
            unschedule(
                deadlines,
                raw,
                &[Some(since), hour.checked_sub(1), Some(hour)],
            );
            schedule(deadlines, raw, &[Some(since), None, hour.checked_add(1)]);
        }
    }

    /// Captures advancing together move their deadlines exactly as one at a
    /// time: every capture due advancing, some not (a stale capture still
    /// due, one stopped long before), captures begun the hour before or at
    /// the hour, and an hour with no hour after it.
    #[test]
    fn advancing_captures_move_their_deadlines_as_one_at_a_time() {
        // Each case: the hour; the captures that extended to their last hour
        // before it (country, first hour, last hour), whose expiry alone is
        // scheduled; those that began at their last hour, all of whose hours
        // are; and the countries that advance to the hour.
        type Capture = (u32, i32, i32);
        let cases: [(i32, Vec<Capture>, Vec<Capture>, Vec<u32>); 5] = [
            (
                50,
                vec![(1, 10, 49), (3, 30, 49)],
                vec![(2, 49, 49)],
                vec![3, 1, 2],
            ),
            (
                50,
                vec![(1, 10, 49), (3, 30, 49), (4, 20, 40)],
                vec![(2, 49, 49), (6, 49, 49)],
                vec![3, 2],
            ),
            (
                50,
                vec![(1, 10, 49)],
                vec![(5, 50, 50), (7, 51, 51)],
                vec![1],
            ),
            (50, vec![(1, 10, 49)], vec![(2, 30, 49)], vec![2, 1]),
            (
                i32::MAX,
                vec![(1, 10, i32::MAX - 1)],
                vec![(2, i32::MAX - 1, i32::MAX - 1)],
                vec![1, 2],
            ),
        ];
        for (case, (hour, extended, begun, advancing)) in cases.into_iter().enumerate() {
            let mut deadlines = BTreeMap::new();
            let mut first = BTreeMap::new();
            for &(raw, since, through) in &extended {
                schedule(
                    &mut deadlines,
                    raw,
                    &[Some(since), None, through.checked_add(1)],
                );
                first.insert(raw, since);
            }
            for &(raw, since, through) in &begun {
                let row = PoliticsCaptureCoverageRow {
                    country: raw,
                    since_hour: since,
                    through_hour: through,
                    ..Default::default()
                };
                schedule(&mut deadlines, raw, &witness_hours(&row));
                first.insert(raw, since);
            }
            let mut advanced: Vec<_> = advancing.iter().map(|raw| (*raw, first[raw])).collect();
            let mut expected = deadlines.clone();
            one_at_a_time(&mut expected, &advanced, hour);
            advance_deadlines(&mut deadlines, &mut advanced, hour);
            assert_eq!(deadlines, expected, "case {case}");
            assert!(
                deadlines.values().all(|raws| !raws.is_empty()),
                "case {case}: no empty deadline is kept"
            );
        }
    }
}
