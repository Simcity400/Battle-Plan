//! Current resolved native joins; never part of the saved accounting history.
use super::*;
use crate::prepared::frame_changes::FrameChanges;

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
    pub power: Vec<CountryPowerChangeRow>,
    pub governments: Vec<GovernmentChangeRow>,
    pub daily: Vec<CountryPowerDailyRow>,
    pub costs: Vec<CountryPowerDailyCostRow>,
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
    power_index: BTreeMap<u32, Vec<usize>>,
    government_index: BTreeMap<u32, Vec<usize>>,
    daily_index: BTreeMap<u32, Vec<usize>>,
    cost_index: BTreeMap<u32, Vec<usize>>,
    character_slots: BTreeSet<u32>,
    pub command: BTreeMap<u32, i64>,
    complete: [bool; 4],
    keys: Option<crate::metadata::CountryKeys>,
    #[cfg(any(test, feature = "test-support"))]
    pub row_visits: usize,
}

impl Inputs {
    pub(super) fn power_rows(&self) -> impl Iterator<Item = &CountryPowerChangeRow> {
        selected_rows(
            &self.power,
            &self.power_index,
            self.power_changed,
            &self.identity_countries,
        )
    }
    pub(super) fn government_rows(&self) -> impl Iterator<Item = &GovernmentChangeRow> {
        selected_rows(
            &self.governments,
            &self.government_index,
            self.governments_changed,
            &self.identity_countries,
        )
    }
    pub(super) fn daily_rows(&self) -> impl Iterator<Item = &CountryPowerDailyRow> {
        selected_rows(
            &self.daily,
            &self.daily_index,
            self.daily_changed,
            &self.identity_countries,
        )
    }
    pub(super) fn cost_rows(&self) -> impl Iterator<Item = &CountryPowerDailyCostRow> {
        selected_rows(
            &self.costs,
            &self.cost_index,
            self.costs_changed,
            &self.identity_countries,
        )
    }
    pub(super) fn identity_rows(&self) -> impl Iterator<Item = &PoliticsCaptureCoverageRow> {
        self.identity_countries
            .iter()
            .filter_map(|raw| self.witnesses.get(raw))
    }
    pub(super) fn refresh(&mut self, frame: &DecodedFrame, changes: Option<&FrameChanges>) {
        let read = |table: TableId| {
            changes.is_none_or(|changes| {
                changes.rows_changed[table.index()] || changes.table_read[table.index()]
            })
        };
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
                    .map(|rows| crate::metadata::CountryKeys::read(rows));
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
                for hour in [
                    Some(row.since_hour),
                    Some(row.through_hour),
                    row.through_hour.checked_add(1),
                ]
                .into_iter()
                .flatten()
                {
                    self.deadlines.entry(hour).or_default().insert(row.country);
                }
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
        self.power_changed = read(TableId::CountryPowerChanges);
        self.governments_changed = read(TableId::GovernmentChanges);
        self.daily_changed = read(TableId::CountryPowerDaily);
        self.costs_changed = read(TableId::CountryPowerDailyCosts);
        let mut daily_owners = BTreeSet::new();
        if self.daily_changed {
            daily_owners.extend(self.daily_index.keys().copied());
        }
        if self.costs_changed {
            daily_owners.extend(self.cost_index.keys().copied());
        }
        let mut power_owners = BTreeSet::new();
        if self.power_changed {
            power_owners.extend(self.power_index.keys().copied());
        }
        if self.power_changed {
            self.slots.extend(
                self.power
                    .iter()
                    .filter_map(|row| self.links.get(&row.country).copied()),
            );
            self.power = frame
                .complete::<tables::CountryPowerChanges>()
                .unwrap_or_default()
                .to_vec();
            self.power_index = index_rows(&self.power, |row| row.country);
            power_owners.extend(self.power_index.keys().copied());
            #[cfg(any(test, feature = "test-support"))]
            {
                self.row_visits += self.power.len();
            }
            self.slots.extend(
                self.power
                    .iter()
                    .filter_map(|row| self.links.get(&row.country).copied()),
            );
            self.daily_mutations.clear();
            for row in &self.power {
                if row.cause == PowerCause::DailyGain && row.currency == PowerCurrency::Political {
                    self.daily_mutations
                        .entry(row.country)
                        .or_default()
                        .insert(row.hour);
                }
            }
        }
        if self.governments_changed {
            self.governments = frame
                .complete::<tables::GovernmentChanges>()
                .unwrap_or_default()
                .to_vec();
            self.government_index = index_rows(&self.governments, |row| row.country);
            #[cfg(any(test, feature = "test-support"))]
            {
                self.row_visits += self.governments.len();
            }
            self.slots.extend(
                self.governments
                    .iter()
                    .filter_map(|row| self.links.get(&row.country).copied()),
            );
        }
        if self.daily_changed {
            self.slots.extend(
                self.daily
                    .iter()
                    .filter_map(|row| self.links.get(&row.country).copied()),
            );
            self.daily = frame
                .complete::<tables::CountryPowerDaily>()
                .unwrap_or_default()
                .to_vec();
            self.daily_index = index_rows(&self.daily, |row| row.country);
            daily_owners.extend(self.daily_index.keys().copied());
            #[cfg(any(test, feature = "test-support"))]
            {
                self.row_visits += self.daily.len();
            }
            self.daily_keys = self
                .daily
                .iter()
                .map(|row| (row.country, row.instance, row.epoch, row.day_hour))
                .collect();
            self.applications = self
                .daily
                .iter()
                .filter_map(|row| {
                    row.pp_sequence.map(|sequence| {
                        (row.country, row.instance, row.epoch, row.day_hour, sequence)
                    })
                })
                .collect();
            self.slots.extend(
                self.daily
                    .iter()
                    .filter_map(|row| self.links.get(&row.country).copied()),
            );
        }
        if self.costs_changed {
            self.slots.extend(
                self.costs
                    .iter()
                    .filter_map(|row| self.links.get(&row.country).copied()),
            );
            self.costs = frame
                .complete::<tables::CountryPowerDailyCosts>()
                .unwrap_or_default()
                .to_vec();
            self.cost_index = index_rows(&self.costs, |row| row.country);
            daily_owners.extend(self.cost_index.keys().copied());
            #[cfg(any(test, feature = "test-support"))]
            {
                self.row_visits += self.costs.len();
            }
            self.cost_keys = self
                .costs
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
            for row in &self.costs {
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
                self.costs
                    .iter()
                    .filter_map(|row| self.links.get(&row.country).copied()),
            );
        }
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
                for row in self
                    .daily_index
                    .get(&country)
                    .into_iter()
                    .flatten()
                    .map(|&index| &self.daily[index])
                {
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
            let mut next = BTreeMap::<u32, BTreeMap<(u32, u64), u8>>::new();
            for row in self.power_rows() {
                if row.operation == PowerOperation::Add
                    && row.requested_delta_raw.is_none()
                    && !(row.cause == PowerCause::DailyGain
                        && row.currency == PowerCurrency::Political)
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
            for country in power_owners {
                self.unknown_additions.remove(&country);
                if let Some(values) = next.remove(&country) {
                    self.unknown_additions.insert(country, values);
                }
            }
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
}

fn index_rows<T>(rows: &[T], country: impl Fn(&T) -> u32) -> BTreeMap<u32, Vec<usize>> {
    let mut index = BTreeMap::<_, Vec<_>>::new();
    for (position, row) in rows.iter().enumerate() {
        index.entry(country(row)).or_default().push(position);
    }
    index
}

fn selected_rows<'a, T>(
    rows: &'a [T],
    index: &'a BTreeMap<u32, Vec<usize>>,
    all: bool,
    countries: &'a BTreeSet<u32>,
) -> impl Iterator<Item = &'a T> {
    rows.iter().take(if all { rows.len() } else { 0 }).chain(
        countries
            .iter()
            .filter(move |_| !all)
            .filter_map(move |country| index.get(country))
            .flatten()
            .map(move |&position| &rows[position]),
    )
}
