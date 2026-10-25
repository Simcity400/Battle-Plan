//! Current resolved native joins; never part of the saved accounting history.
use super::*;
use dashboard_base::prepared::frame_changes::{FrameChanges, field_mask};
use replay_data::{rows::DeltaRow, rows::TableSchema};
use replay_frame::{frame::SharedView, frame::TableView};
use std::sync::LazyLock;

/// Each country's row positions in table order.
type Index = BTreeMap<u32, Vec<usize>>;

/// Captures extended to an hour, with the hour they reached.
type DeferredAdvance = (Arc<[(u32, i32)]>, i32);

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
    pub sums: BTreeMap<(u32, u64, i32, u64), i64>,
    pub applications: BTreeSet<(u32, u64, i32, u64)>,
    pub unreconciled: BTreeSet<u32>,
    pub unreconciled_counts: BTreeMap<u32, u64>,
    pub daily_mutations: BTreeMap<u32, BTreeSet<i32>>,
    pub unknown_additions: BTreeMap<u32, BTreeMap<(u32, u64), u8>>,
    /// The event tables as decoded, held rather than copied.
    power: Option<SharedView>,
    governments: Option<SharedView>,
    daily: Option<SharedView>,
    costs: Option<SharedView>,
    pub daily_keys: BTreeSet<(u32, u64, i32)>,
    pub cost_keys: BTreeSet<(u32, u64, i32, u32)>,
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
    /// Native countries with one census row, one slot and one tag, without aliases.
    unique_countries: BTreeSet<u32>,
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
    power_hours: BTreeMap<u32, BTreeMap<i32, usize>>,
    gain_counts: BTreeMap<u32, BTreeMap<i32, usize>>,
    addition_counts: BTreeMap<u32, BTreeMap<(u32, u64), [usize; 2]>>,
    sum_counts: BTreeMap<(u32, u64, i32, u64), usize>,
    #[cfg(test)]
    pub aggregate_visits: usize,
    #[cfg(test)]
    pub window_visits: usize,
    /// The countries with a daily political gain at each hour.
    mutation_hours: BTreeMap<i32, BTreeSet<u32>>,
    /// The countries whose [power, government] rows changed this hour, when
    /// it was joined from its changed rows; none when the whole tables were.
    pub changed_rows: Option<[BTreeSet<u32>; 2]>,
    /// Rows the accounting can change: recorded edits, unresolved events and
    /// the complete rows of identities observed again, in table order.
    accounting: Option<[Vec<usize>; 4]>,
    /// How many [daily upkeep or gain power, government, daily] rows date at each
    /// native day: the newest is the newest native day they publish.
    days: [BTreeMap<i32, usize>; 3],
    /// The captures this hour's join from its changed rows only extended to
    /// the hour, each with its first hour, by country: none after a join of
    /// the whole tables.
    pub advanced: Vec<(u32, i32)>,
    /// This read slid captures by one hour, in country order, each with its
    /// width relative to the hour; fixed-start certificates cannot describe it.
    pub windows: Vec<(u32, i64)>,
    window: Option<(Arc<WindowAdvance>, i32)>,
    /// Captures extended to an hour by [`Self::refresh_advanced`] whose
    /// witnesses still hold the last hour before: the captures and the hour
    /// they reached. [`Self::materialize_advance`] moves them there.
    deferred: Option<DeferredAdvance>,
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

fn daily_day(row: &CountryPowerDailyRow) -> Option<i32> {
    Some(row.day_hour)
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

/// Fast carry requires every raw capture operand except the end hour unchanged.
fn advances(old: &PoliticsCaptureCoverageRow, row: &PoliticsCaptureCoverageRow, hour: i32) -> bool {
    (
        old.country,
        old.instance,
        old.epoch,
        old.since_hour,
        old.streams,
    ) == (
        row.country,
        row.instance,
        row.epoch,
        row.since_hour,
        row.streams,
    ) && old.through_hour.checked_add(1) == Some(row.through_hour)
        && row.through_hour == hour
}

/// Remove the moving windows' old boundaries before inserting any new one:
/// the starts have already taken effect, so only the next expiry is scheduled.
fn shift_deadlines(
    deadlines: &mut BTreeMap<i32, BTreeSet<u32>>,
    shifted: &[(u32, i32, i32)],
    hour: i32,
) {
    let _cpu = ::replay_frame::cpu::scope_phase("politics_ledger:capture_windows");
    let Some(&(.., first, _)) = shifted.first() else {
        return;
    };
    let last = hour
        .checked_sub(1)
        .expect("a window continued the prior hour");
    // A common recorded window moves all its countries together. Reuse its
    // expiry set instead of removing and reinserting each country at each end.
    if shifted.iter().all(|&(_, old, _)| old == first)
        && deadlines.get(&hour).is_some_and(|due| {
            due.len() == shifted.len()
                && due.iter().zip(shifted).all(|(raw, moved)| *raw == moved.0)
        })
    {
        let mut moving = deadlines.remove(&hour).unwrap();
        for at in [first, last] {
            if let Some(rows) = deadlines.get_mut(&at) {
                if *rows == moving {
                    deadlines.remove(&at);
                } else {
                    rows.retain(|raw| !moving.contains(raw));
                    if rows.is_empty() {
                        deadlines.remove(&at);
                    }
                }
            }
        }
        if let Some(next) = hour.checked_add(1) {
            deadlines.entry(next).or_default().append(&mut moving);
        }
    } else {
        for &(raw, old, _) in shifted {
            unschedule(deadlines, raw, &[Some(old), Some(last), Some(hour)]);
        }
        for &(raw, _, _) in shifted {
            schedule(deadlines, raw, &[None, hour.checked_add(1), None]);
        }
    }
}

/// Countries whose capture starts and ends move together. Widths are relative
/// to the accepted hour, so consecutive windows share one certificate.
pub(super) struct WindowAdvance {
    pub captures: Vec<(u32, i64)>,
    countries: BTreeSet<u32>,
    starts: BTreeMap<i64, BTreeSet<u32>>,
}

impl WindowAdvance {
    pub fn new(captures: Vec<(u32, i64)>) -> Self {
        let mut starts = BTreeMap::<_, BTreeSet<_>>::new();
        for &(raw, width) in &captures {
            starts.entry(width).or_default().insert(raw);
        }
        Self {
            countries: captures.iter().map(|&(raw, _)| raw).collect(),
            captures,
            starts,
        }
    }

    pub fn contains(&self, raw: &u32) -> bool {
        self.countries.contains(raw)
    }

    fn shift(&self, deadlines: &mut BTreeMap<i32, BTreeSet<u32>>, hour: i32, carried: bool) {
        let _cpu = ::replay_frame::cpu::scope_phase("politics_ledger:window_expiry_changes");
        let before = hour
            .checked_sub(1)
            .expect("a window continued the prior hour");
        // Starts at or before this hour have already taken effect. As with
        // a fixed-start extension's old last hour, do not schedule them again:
        // a zero-width window would otherwise reselect every country now.
        if !carried {
            for (&width, countries) in &self.starts {
                let since =
                    i32::try_from(i64::from(hour) - width).expect("certified capture start");
                let old = since
                    .checked_sub(1)
                    .expect("a capture start moved by one hour");
                take_deadline_members(deadlines, old, countries, false);
            }
            take_deadline_members(deadlines, before, &self.countries, false);
        }
        let mut moving = take_deadline_members(deadlines, hour, &self.countries, carried)
            .unwrap_or_else(|| self.countries.clone());
        if let Some(next) = hour.checked_add(1) {
            deadlines.entry(next).or_default().append(&mut moving);
        }
    }
}

/// A carried certificate already inserted these members. An otherwise empty
/// bucket can move whole; mixed buckets retain only the other boundaries.
fn take_deadline_members(
    deadlines: &mut BTreeMap<i32, BTreeSet<u32>>,
    hour: i32,
    members: &BTreeSet<u32>,
    carried: bool,
) -> Option<BTreeSet<u32>> {
    let rows = deadlines.get_mut(&hour)?;
    if rows.len() == members.len() && (carried || rows == members) {
        return deadlines.remove(&hour);
    }
    rows.retain(|raw| {
        #[cfg(test)]
        WINDOW_DEADLINE_VISITS.with(|visits| visits.set(visits.get() + 1));
        !members.contains(raw)
    });
    if rows.is_empty() {
        deadlines.remove(&hour);
    }
    None
}

impl dashboard_base::prepared::prepass::Held for WindowAdvance {
    fn held(&self, held: &mut dashboard_base::prepared::prepass::HeldBytes) {
        held.heap(&self.captures)
            .heap(&self.countries)
            .heap(&self.starts);
    }
}

#[cfg(test)]
thread_local! {
    static WINDOW_DEADLINE_VISITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Move the deadlines of the captures that advanced to `hour` (each country
/// with its first hour, in country order) exactly as unscheduling each one's
/// old hours and scheduling its first hour and its next expiry does, one
/// capture at a time: each leaves the hour before (its old last hour),
/// unless it began then, and its expiry at `hour`, keeps its first hour, and
/// expires at the hour after. When every capture due at `hour` advanced,
/// their expiries move together.
fn advance_deadlines(
    deadlines: &mut BTreeMap<i32, BTreeSet<u32>>,
    advanced: &[(u32, i32)],
    hour: i32,
    carried: bool,
) {
    let _cpu = ::replay_frame::cpu::scope_phase("politics_ledger:expiry_changes");
    if advanced.is_empty() {
        return;
    }
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
                && (carried
                    || due.iter().zip(advanced).all(|(due, (raw, _))| {
                        #[cfg(test)]
                        DEADLINE_MEMBERSHIP_VISITS.with(|visits| visits.set(visits.get() + 1));
                        due == raw
                    })) =>
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

#[cfg(test)]
thread_local! {
    static DEADLINE_MEMBERSHIP_VISITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// What a join holds: its event views once, and every country, link,
/// witness, capture, deadline, sum, key, index and set it keeps beside them.
impl dashboard_base::prepared::prepass::Held for Inputs {
    fn held(&self, held: &mut dashboard_base::prepared::prepass::HeldBytes) {
        held.heap(&self.countries)
            .heap(&self.census)
            .heap(&self.naming_ids)
            .heap(&self.tag_slots)
            .heap(&self.refused)
            .heap(&self.refused_reasons)
            .heap(&self.links)
            .heap(&self.witnesses)
            .heap(&self.deadlines)
            .heap(&self.by_slot)
            .heap(&self.sums)
            .heap(&self.applications)
            .heap(&self.unreconciled)
            .heap(&self.unreconciled_counts)
            .heap(&self.daily_mutations)
            .heap(&self.unknown_additions)
            .heap(&self.power)
            .heap(&self.governments)
            .heap(&self.daily)
            .heap(&self.costs)
            .heap(&self.daily_keys)
            .heap(&self.cost_keys)
            .heap(&self.slots)
            .heap(&self.previous_tags)
            .heap(&self.identity_slots)
            .heap(&self.identity_countries)
            .heap(&self.raw_slots)
            .heap(&self.unique_countries)
            .heap(&self.power_index)
            .heap(&self.government_index)
            .heap(&self.daily_index)
            .heap(&self.cost_index)
            .heap(&self.character_slots)
            .heap(&self.command)
            .heap(&self.keys)
            .heap(&self.touched)
            .heap(&self.latest_power)
            .heap(&self.power_hours)
            .heap(&self.gain_counts)
            .heap(&self.addition_counts)
            .heap(&self.sum_counts)
            .heap(&self.mutation_hours)
            .heap(&self.changed_rows)
            .heap(&self.accounting)
            .heap(&self.days)
            .heap(&self.advanced)
            .heap(&self.windows)
            .heap(&self.window)
            .heap(&self.deferred);
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
    /// The newest native day of the [daily upkeep or gain power, government, daily]
    /// rows held.
    pub(super) fn newest_day(&self, table: usize) -> Option<i32> {
        self.days[table].last_key_value().map(|(&day, _)| day)
    }

    /// Owners whose costs may differ from the last join. Identity rechecks
    /// can add owners; their equal keys are safe to prune again as well.
    pub(super) fn changed_cost_owners(&self) -> Option<&BTreeSet<u32>> {
        self.narrow.then_some(&self.touched[3])
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
    pub(super) fn native_days(&self) -> &[BTreeMap<i32, usize>; 3] {
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
    pub(super) fn accounting_power(&self) -> Box<dyn Iterator<Item = &CountryPowerChangeRow> + '_> {
        let rows = self.power();
        match &self.accounting {
            Some(positions) => Box::new(positions[0].iter().map(move |&at| &rows[at])),
            None => Box::new(self.power_rows()),
        }
    }
    pub(super) fn accounting_government(
        &self,
    ) -> Box<dyn Iterator<Item = &GovernmentChangeRow> + '_> {
        let rows = self.governments();
        match &self.accounting {
            Some(positions) => Box::new(positions[1].iter().map(move |&at| &rows[at])),
            None => Box::new(self.government_rows()),
        }
    }
    pub(super) fn accounting_daily(&self) -> Box<dyn Iterator<Item = &CountryPowerDailyRow> + '_> {
        let rows = self.daily();
        match &self.accounting {
            Some(positions) => Box::new(positions[2].iter().map(move |&at| &rows[at])),
            None => Box::new(self.daily_rows()),
        }
    }
    pub(super) fn accounting_cost(
        &self,
    ) -> Box<dyn Iterator<Item = &CountryPowerDailyCostRow> + '_> {
        let rows = self.costs();
        match &self.accounting {
            Some(positions) => Box::new(positions[3].iter().map(move |&at| &rows[at])),
            None => Box::new(self.cost_rows()),
        }
    }

    pub(super) fn identity_rows(&self) -> impl Iterator<Item = &PoliticsCaptureCoverageRow> {
        self.identity_countries
            .iter()
            .filter_map(|raw| self.witnesses.get(raw))
    }

    /// Whether this hour was joined from its changed rows and they join no
    /// country: no census, link, capture identity, power, government,
    /// allocation, completeness, daily gain or naming change selects one,
    /// and no row is observed again. Its captures extend or slide to the
    /// hour without changing any accounting operand.
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
        // Both joins read the captures' last hours.
        self.materialize_advance();
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
        self.materialize_advance();
        self.advanced.clear();
        self.windows.clear();
        self.narrow = false;
        self.changed_rows = None;
        self.accounting = None;
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
        if self.countries_changed || links {
            self.join_unique_countries();
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
            self.power_hours.clear();
            self.gain_counts.clear();
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
            self.days[2].clear();
            count_days(&mut self.days[2], daily, daily_day, true);
            self.daily_keys = daily
                .iter()
                .map(|row| (row.country, row.epoch, row.day_hour))
                .collect();
            self.applications = daily
                .iter()
                .filter_map(|row| {
                    row.pp_sequence
                        .map(|sequence| (row.country, row.epoch, row.day_hour, sequence))
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
                .map(|row| (row.country, row.epoch, row.day_hour, row.ordinal))
                .collect();
            self.sums.clear();
            self.sum_counts.clear();
            for row in costs {
                *self
                    .sum_counts
                    .entry((row.country, row.epoch, row.day_hour, row.pp_sequence))
                    .or_default() += 1;
                let sum = self
                    .sums
                    .entry((row.country, row.epoch, row.day_hour, row.pp_sequence))
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
            self.reconcile_daily(daily_owners, links);
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
    /// changed by one recorded delta, or a census key, link or dictionary
    /// changed: the caller then refreshes the whole tables.
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
        let (Some(daily), Some(costs)) = (
            table_delta(
                changes,
                frame,
                TableId::CountryPowerDaily,
                self.daily().is_empty(),
            ),
            table_delta(
                changes,
                frame,
                TableId::CountryPowerDailyCosts,
                self.costs().is_empty(),
            ),
        ) else {
            return false;
        };
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
            self.row_visits += captures.len()
                + allocations.len()
                + power.0.len()
                + governments.0.len()
                + daily.0.len()
                + costs.0.len();
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
        let capture_cpu = ::replay_frame::cpu::scope_phase("politics_ledger:capture_join");
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
        let mut shifted = Vec::new();
        for row in captures {
            let raw = row.country;
            let unique = self.unique_countries.contains(&raw);
            if let Some(old) = self.witnesses.get_mut(&raw) {
                // No power row dated after the old interval: the extension adds
                // no event, daily gain or unknown addition to it.
                if unique
                    && advances(old, row, frame.hour)
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
                // Moving a start by one hour drops only its old first hour.
                // Unknown additions require their full counted recheck; a
                // daily gain there can change spending completeness. All
                // other operands and every old coverage run keep their value.
                if unique
                    && (old.country, old.instance, old.epoch, old.streams)
                        == (row.country, row.instance, row.epoch, row.streams)
                    && old.since_hour.checked_add(1) == Some(row.since_hour)
                    && old.through_hour.checked_add(1) == Some(row.through_hour)
                    && row.through_hour == frame.hour
                    && row.since_hour <= frame.hour
                    && self
                        .latest_power
                        .get(&raw)
                        .is_none_or(|&at| at <= old.through_hour)
                    && self
                        .unknown_additions
                        .get(&raw)
                        .is_none_or(BTreeMap::is_empty)
                    && !self
                        .daily_mutations
                        .get(&raw)
                        .is_some_and(|hours| hours.contains(&old.since_hour))
                {
                    shifted.push((raw, old.since_hour, row.since_hour));
                    old.since_hour = row.since_hour;
                    old.through_hour = row.through_hour;
                    if let Some(slot) = self.links.get(&raw)
                        && let Some(witness) = self.by_slot.get_mut(slot)
                        && witness.country == raw
                    {
                        witness.since_hour = row.since_hour;
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
        advanced.sort_unstable_by_key(|&(raw, _)| raw);
        advance_deadlines(&mut self.deadlines, &advanced, frame.hour, false);
        shifted.sort_unstable_by_key(|&(raw, _, _)| raw);
        shift_deadlines(&mut self.deadlines, &shifted, frame.hour);
        self.windows = shifted
            .iter()
            .map(|&(raw, _, since)| (raw, i64::from(frame.hour) - i64::from(since)))
            .collect();
        self.advanced = advanced;
        drop(capture_cpu);
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
            let _cpu = ::replay_frame::cpu::scope_phase("politics_ledger:join_power");
            let old = self.power.clone();
            for row in replaced_rows(
                held::<tables::CountryPowerChanges>(&old),
                self.power_index.as_ref(),
                power,
                |key| (key >> 64) as u32,
            ) {
                count_days(&mut self.days[0], [row], power_day, false);
                self.index_power_row(row, false);
                if !self.identity_countries.contains(&row.country) {
                    self.index_addition(row, false);
                }
            }
            self.power = Some(frame.tables[TableId::CountryPowerChanges.index()].clone());
            if !touched.is_empty() {
                self.power_index = None;
                let view = self.power.clone();
                for row in replaced_rows(
                    held::<tables::CountryPowerChanges>(&view),
                    None,
                    (power.0, &[]),
                    |key| (key >> 64) as u32,
                ) {
                    count_days(&mut self.days[0], [row], power_day, true);
                    self.index_power_row(row, true);
                    if !self.identity_countries.contains(&row.country) {
                        self.index_addition(row, true);
                    }
                }
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
            let old = self.governments.clone();
            for row in replaced_rows(
                held::<tables::GovernmentChanges>(&old),
                self.government_index.as_ref(),
                governments,
                |key| (key >> 64) as u32,
            ) {
                count_days(&mut self.days[1], [row], government_day, false);
            }
            self.governments = Some(frame.tables[TableId::GovernmentChanges.index()].clone());
            if !touched.is_empty() {
                self.government_index = None;
                for row in replaced_rows(
                    held::<tables::GovernmentChanges>(&self.governments),
                    None,
                    (governments.0, &[]),
                    |key| (key >> 64) as u32,
                ) {
                    count_days(&mut self.days[1], [row], government_day, true);
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
        // Replace only changed owners' daily keys, applications and wrapping
        // cost sums. Key order groups each table's rows by country, including
        // after additions and deletions move their positions.
        let daily_owners: BTreeSet<u32> = daily
            .0
            .iter()
            .map(|row| row.key)
            .chain(daily.1.iter().copied())
            .map(|key| key as u32)
            .collect();
        let cost_owners: BTreeSet<u32> = costs
            .0
            .iter()
            .map(|row| row.key)
            .chain(costs.1.iter().copied())
            .map(|key| (key >> 32) as u32)
            .collect();
        let cpu = ::replay_frame::cpu::scope_phase("politics_ledger:join_daily");
        if self.daily_changed {
            let old = self.daily.clone();
            for row in replaced_rows(
                held::<tables::CountryPowerDaily>(&old),
                self.daily_index.as_ref(),
                daily,
                |key| key as u32,
            ) {
                count_days(&mut self.days[2], [row], daily_day, false);
                self.daily_keys
                    .remove(&(row.country, row.epoch, row.day_hour));
                if let Some(sequence) = row.pp_sequence {
                    self.applications
                        .remove(&(row.country, row.epoch, row.day_hour, sequence));
                }
            }
            self.daily = Some(frame.tables[TableId::CountryPowerDaily.index()].clone());
            if !daily_owners.is_empty() {
                self.daily_index = None;
                for row in replaced_rows(
                    held::<tables::CountryPowerDaily>(&self.daily),
                    None,
                    (daily.0, &[]),
                    |key| key as u32,
                ) {
                    count_days(&mut self.days[2], [row], daily_day, true);
                    self.daily_keys
                        .insert((row.country, row.epoch, row.day_hour));
                    if let Some(sequence) = row.pp_sequence {
                        self.applications
                            .insert((row.country, row.epoch, row.day_hour, sequence));
                    }
                }
            }
        }
        drop(cpu);
        let cpu = ::replay_frame::cpu::scope_phase("politics_ledger:join_costs");
        if self.costs_changed {
            let old = self.costs.clone();
            for row in replaced_rows(
                held::<tables::CountryPowerDailyCosts>(&old),
                self.cost_index.as_ref(),
                costs,
                |key| (key >> 32) as u32,
            ) {
                self.index_cost(row, false);
            }
            self.costs = Some(frame.tables[TableId::CountryPowerDailyCosts.index()].clone());
            if !cost_owners.is_empty() {
                self.cost_index = None;
                let view = self.costs.clone();
                for row in replaced_rows(
                    held::<tables::CountryPowerDailyCosts>(&view),
                    None,
                    (costs.0, &[]),
                    |key| (key >> 32) as u32,
                ) {
                    self.index_cost(row, true);
                }
            }
        }
        drop(cpu);
        let affected: BTreeSet<_> = daily_owners.union(&cost_owners).copied().collect();
        self.reconcile_daily(affected.iter().copied(), false);
        self.slots.extend(
            affected
                .iter()
                .filter_map(|raw| self.links.get(raw))
                .copied(),
        );
        self.touched[2] = daily_owners;
        self.touched[3] = cost_owners;
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
        if !self.identity_countries.is_empty() {
            self.reread_additions(self.identity_countries.clone());
        }
        let _cpu = ::replay_frame::cpu::scope_phase("politics_ledger:accounting_rows");
        self.accounting = Some([
            accounting_positions(
                self.power(),
                self.power_index.as_ref(),
                &self.identity_countries,
                power.0,
                pending[0]
                    .iter()
                    .copied()
                    .filter(|key| self.touched[0].contains(&((key >> 64) as u32))),
                |row| row.country,
                |key| (key >> 64) as u32,
            ),
            accounting_positions(
                self.governments(),
                self.government_index.as_ref(),
                &self.identity_countries,
                governments.0,
                std::iter::empty(),
                |row| row.country,
                |key| (key >> 64) as u32,
            ),
            accounting_positions(
                self.daily(),
                self.daily_index.as_ref(),
                &self.identity_countries,
                daily.0,
                std::iter::empty(),
                |row| row.country,
                |key| key as u32,
            ),
            accounting_positions(
                self.costs(),
                self.cost_index.as_ref(),
                &self.identity_countries,
                costs.0,
                pending[1]
                    .iter()
                    .copied()
                    .filter(|key| self.touched[3].contains(&((key >> 32) as u32))),
                |row| row.country,
                |key| (key >> 32) as u32,
            ),
        ]);
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

    /// [`Self::refresh_rows`] of an hour a pre-pass worker certified for this
    /// join: from its changed rows, every changed capture (`advanced`, each
    /// with its first hour, in country order) only extends to the hour, and
    /// nothing else joins a country (see `prepared::BlockFacts`); the caller
    /// has checked that no row not yet recorded as an event is observed
    /// again. It does what that join does there, in its order: no country is
    /// joined, each capture's expiry moves to the hour after, and the tables
    /// read are held as the hour decodes them. Only the captures' witnesses
    /// keep their last hour until [`Self::materialize_advance`], which every
    /// other join runs first: nothing an hour that only advances captures
    /// reads takes it from them.
    pub(super) fn refresh_advanced(
        &mut self,
        frame: &DecodedFrame,
        changes: &FrameChanges,
        advanced: &Arc<[(u32, i32)]>,
    ) {
        // The prior advance inserted every one of these captures into this
        // hour's expiry bucket. With the same membership and bucket size no
        // key comparison is needed; extra fixed deadlines still take the join.
        if self.window.is_some() {
            self.materialize_advance();
        }
        let carried = self.deferred.as_ref().is_some_and(|(deferred, through)| {
            Arc::ptr_eq(deferred, advanced) && through.checked_add(1) == Some(frame.hour)
        });
        // Captures other than those deferred reach their last hour first.
        if self
            .deferred
            .as_ref()
            .is_some_and(|(deferred, _)| !Arc::ptr_eq(deferred, advanced) && deferred != advanced)
        {
            self.materialize_advance();
        }
        advance_deadlines(&mut self.deadlines, advanced, frame.hour, carried);
        self.deferred = Some((Arc::clone(advanced), frame.hour));
        self.carry_views(frame, changes);
    }

    /// The empty selection and retained views shared by both capture proofs.
    fn carry_views(&mut self, frame: &DecodedFrame, changes: &FrameChanges) {
        let read = |table: TableId| {
            changes.rows_changed[table.index()] || changes.table_read[table.index()]
        };
        self.narrow = true;
        self.advanced.clear();
        self.windows.clear();
        self.touched = Default::default();
        self.slots.clear();
        self.previous_tags.clear();
        self.identity_slots.clear();
        self.identity_countries.clear();
        self.countries_changed = false;
        let links = read(TableId::CountryLinks);
        let coverage_read = read(TableId::PoliticsCaptureCoverage);
        self.identity_changed = links || coverage_read;
        self.power_changed = read(TableId::CountryPowerChanges);
        self.governments_changed = read(TableId::GovernmentChanges);
        self.daily_changed = read(TableId::CountryPowerDaily);
        self.costs_changed = read(TableId::CountryPowerDailyCosts);
        // No power or government row changed: the held rows, their native
        // days and indexes are the hour's.
        if self.power_changed {
            self.power = Some(frame.tables[TableId::CountryPowerChanges.index()].clone());
        }
        if self.governments_changed {
            self.governments = Some(frame.tables[TableId::GovernmentChanges.index()].clone());
        }
        self.changed_rows = Some(Default::default());
        self.accounting = Some(Default::default());
        if self.daily_changed {
            self.daily = Some(frame.tables[TableId::CountryPowerDaily.index()].clone());
        }
        if self.costs_changed {
            self.costs = Some(frame.tables[TableId::CountryPowerDailyCosts.index()].clone());
        }
        // The tables stay as readable as they were; the dictionary's
        // readability is the hour's.
        self.observe_completeness(frame);
        // Each changed allocation holds its amount; no country's power rows
        // or capture identity changed, so no unknown addition is read again.
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

    /// Apply a worker-certified sliding window, carrying deadline membership
    /// and deferring witness writes until the next join reads them.
    pub(super) fn refresh_window(
        &mut self,
        frame: &DecodedFrame,
        changes: &FrameChanges,
        window: &Arc<WindowAdvance>,
    ) {
        let carried = self.window.as_ref().is_some_and(|(held, through)| {
            Arc::ptr_eq(held, window) && through.checked_add(1) == Some(frame.hour)
        });
        if self.deferred.is_some()
            || self
                .window
                .as_ref()
                .is_some_and(|(held, _)| !Arc::ptr_eq(held, window))
        {
            self.materialize_advance();
        }
        window.shift(&mut self.deadlines, frame.hour, carried);
        self.window = Some((Arc::clone(window), frame.hour));
        self.carry_views(frame, changes);
    }

    /// Move the deferred captures' witnesses to the last hour they reached,
    /// as each hour's join from its changed rows moves them.
    pub(super) fn materialize_advance(&mut self) {
        if let Some((window, through)) = self.window.take() {
            let _cpu = ::replay_frame::cpu::scope_phase("politics_ledger:window_witnesses");
            for &(raw, width) in &window.captures {
                #[cfg(test)]
                {
                    self.window_visits += 1;
                }
                let since =
                    i32::try_from(i64::from(through) - width).expect("certified capture start");
                if let Some(witness) = self.witnesses.get_mut(&raw) {
                    witness.since_hour = since;
                    witness.through_hour = through;
                }
                if let Some(slot) = self.links.get(&raw)
                    && let Some(witness) = self.by_slot.get_mut(slot)
                    && witness.country == raw
                {
                    witness.since_hour = since;
                    witness.through_hour = through;
                }
            }
        }
        let Some((advanced, through)) = self.deferred.take() else {
            return;
        };
        for &(raw, _) in advanced.iter() {
            if let Some(witness) = self.witnesses.get_mut(&raw) {
                witness.through_hour = through;
            }
            if let Some(slot) = self.links.get(&raw)
                && let Some(witness) = self.by_slot.get_mut(slot)
                && witness.country == raw
            {
                witness.through_hour = through;
            }
        }
    }

    /// Whether captures are deferred ([`Self::refresh_advanced`]).
    pub(super) fn deferred(&self) -> bool {
        self.deferred.is_some() || self.window.is_some()
    }

    /// Whether, once `advanced` (in country order) extended to `hour` and
    /// their expiries moved, a linked capture is due from `next` through
    /// `hour`: the captures the whole accounting observes there.
    #[cfg(test)]
    pub(super) fn due_after_advance(&self, next: i32, hour: i32, advanced: &[(u32, i32)]) -> bool {
        self.deadlines.range(next..=hour).any(|(&at, raws)| {
            raws.iter().any(|raw| {
                let moved = advanced
                    .binary_search_by_key(raw, |&(raw, _)| raw)
                    .is_ok_and(|position| {
                        // Its expiry at the hour moves to the hour after, and
                        // it leaves its old last hour unless it began then.
                        at == hour
                            || (Some(at) == hour.checked_sub(1) && advanced[position].1 != at)
                    });
                !moved && self.links.contains_key(raw)
            })
        })
    }

    /// Whether this join and `other` hold the same state for every later
    /// hour's join: everything a join reads from the hour before, beside the
    /// hour's own selections (the countries, slots and rows it observes, the
    /// tables it read, and how it joined) and the row indexes, which find the
    /// same rows as the rows' order does.
    pub(super) fn same_join(&self, other: &Self) -> bool {
        self.deferred.is_none()
            && other.deferred.is_none()
            && self.window.is_none()
            && other.window.is_none()
            && self.countries == other.countries
            && self.census == other.census
            && self.naming_ids == other.naming_ids
            && self.tag_slots == other.tag_slots
            && self.refused == other.refused
            && self.refused_reasons == other.refused_reasons
            && self.links == other.links
            && self.witnesses == other.witnesses
            && self.deadlines == other.deadlines
            && self.by_slot == other.by_slot
            && self.sums == other.sums
            && self.applications == other.applications
            && self.unreconciled == other.unreconciled
            && self.unreconciled_counts == other.unreconciled_counts
            && self.daily_mutations == other.daily_mutations
            && self.unknown_additions == other.unknown_additions
            && self.power() == other.power()
            && self.governments() == other.governments()
            && self.daily() == other.daily()
            && self.costs() == other.costs()
            && self.daily_keys == other.daily_keys
            && self.cost_keys == other.cost_keys
            && self.raw_slots == other.raw_slots
            && self.unique_countries == other.unique_countries
            && self.character_slots == other.character_slots
            && self.command == other.command
            && self.complete == other.complete
            && self.lexical_complete == other.lexical_complete
            // The census keys are read with the census, which holds them.
            && self.keys.is_some() == other.keys.is_some()
            && self.latest_power == other.latest_power
            && self.mutation_hours == other.mutation_hours
            && self.days == other.days
    }

    /// Coverage and holds are stored by tag, while accounting visits slots.
    /// Shared tags or aliases can append rows even when each native witness
    /// only advances. Cache the one-to-one joins when census or links change.
    fn join_unique_countries(&mut self) {
        let mut slots = BTreeMap::<_, usize>::new();
        let mut tags = BTreeMap::<_, usize>::new();
        for row in &self.census {
            *slots.entry(row.canonical_slot).or_default() += 1;
            *tags.entry(row.tag.as_slice()).or_default() += 1;
        }
        self.unique_countries = self
            .countries
            .values()
            .filter(|row| {
                slots[&row.canonical_slot] == 1
                    && tags[row.tag.as_slice()] == 1
                    && self
                        .raw_slots
                        .get(&row.canonical_slot)
                        .is_some_and(|raws| raws.len() == 1 && raws.contains(&row.id))
            })
            .map(|row| row.id)
            .collect();
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

    /// Rebuild a snapshot's power aggregates. Deltas update the same counts
    /// one row at a time, including repeated gains at one hour.
    fn index_power(&mut self, countries: impl IntoIterator<Item = u32>) {
        let view = self.power.clone();
        // The helpers below update aggregates only; the index is restored after.
        let index = self.power_index.take();
        for country in countries {
            self.power_hours.remove(&country);
            self.gain_counts.remove(&country);
            self.latest_power.remove(&country);
            for hour in self.daily_mutations.remove(&country).into_iter().flatten() {
                remove_member(&mut self.mutation_hours, hour, country);
            }
            let positions: Vec<_> = country_rows(
                held::<tables::CountryPowerChanges>(&view),
                index.as_ref(),
                country,
                |row| row.country,
            )
            .collect();
            for row in positions {
                self.index_power_row(row, true);
            }
        }
        self.power_index = index;
    }

    fn index_power_row(&mut self, row: &CountryPowerChangeRow, add: bool) {
        #[cfg(test)]
        {
            self.aggregate_visits += 1;
        }
        let hours = self.power_hours.entry(row.country).or_default();
        change_count(hours, row.hour, add);
        if let Some((&last, _)) = hours.last_key_value() {
            self.latest_power.insert(row.country, last);
        } else {
            self.power_hours.remove(&row.country);
            self.latest_power.remove(&row.country);
        }
        if row.cause == PowerCause::DailyGain && row.currency == PowerCurrency::Political {
            let hours = self.gain_counts.entry(row.country).or_default();
            change_count(hours, row.hour, add);
            if hours.contains_key(&row.hour) {
                self.daily_mutations
                    .entry(row.country)
                    .or_default()
                    .insert(row.hour);
                self.mutation_hours
                    .entry(row.hour)
                    .or_default()
                    .insert(row.country);
            } else {
                remove_member(&mut self.daily_mutations, row.country, row.hour);
                remove_member(&mut self.mutation_hours, row.hour, row.country);
            }
            if hours.is_empty() {
                self.gain_counts.remove(&row.country);
            }
        }
    }

    fn index_cost(&mut self, row: &CountryPowerDailyCostRow, add: bool) {
        #[cfg(test)]
        {
            self.aggregate_visits += 1;
        }
        let key = (row.country, row.epoch, row.day_hour, row.ordinal);
        let sum_key = (row.country, row.epoch, row.day_hour, row.pp_sequence);
        if add {
            self.cost_keys.insert(key);
            *self.sum_counts.entry(sum_key).or_default() += 1;
            let sum = self.sums.entry(sum_key).or_default();
            *sum = sum.wrapping_add(row.cost_raw);
        } else {
            self.cost_keys.remove(&key);
            change_count(&mut self.sum_counts, sum_key, false);
            if self.sum_counts.contains_key(&sum_key) {
                let sum = self.sums.get_mut(&sum_key).expect("retained cost sum");
                *sum = sum.wrapping_sub(row.cost_raw);
            } else {
                self.sums.remove(&sum_key);
            }
        }
    }

    /// Reconcile only countries whose daily applications or cost rows changed.
    fn reconcile_daily(&mut self, owners: impl IntoIterator<Item = u32>, links_changed: bool) {
        for country in owners {
            let previous = self.unreconciled.remove(&country);
            for row in country_rows(
                held::<tables::CountryPowerDaily>(&self.daily),
                self.daily_index.as_ref(),
                country,
                |row| row.country,
            ) {
                if let Some(sequence) = row.pp_sequence {
                    let key = (row.country, row.epoch, row.day_hour, sequence);
                    if row
                        .pp_cost_raw
                        .is_none_or(|cost| self.sums.get(&key).copied().unwrap_or(0) != cost)
                    {
                        self.unreconciled.insert(row.country);
                    }
                }
            }
            if !links_changed
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

    /// Identity changes recheck the owner's additions against its new capture.
    fn reread_additions(&mut self, owners: BTreeSet<u32>) {
        let view = self.power.clone();
        let index = self.power_index.take();
        for country in owners {
            self.unknown_additions.remove(&country);
            self.addition_counts.remove(&country);
            let rows: Vec<_> = country_rows(
                held::<tables::CountryPowerChanges>(&view),
                index.as_ref(),
                country,
                |row| row.country,
            )
            .collect();
            for row in rows {
                self.index_addition(row, true);
            }
        }
        self.power_index = index;
    }

    fn index_addition(&mut self, row: &CountryPowerChangeRow, add: bool) {
        #[cfg(test)]
        {
            self.aggregate_visits += 1;
        }
        if row.operation != PowerOperation::Add
            || row.requested_delta_raw.is_some()
            || row.cause == PowerCause::DailyGain && row.currency == PowerCurrency::Political
            || !self.witnesses.get(&row.country).is_some_and(|witness| {
                witness.epoch == row.epoch
                    && row.hour >= witness.since_hour
                    && row.hour <= witness.through_hour
            })
        {
            return;
        }
        let counts = self.addition_counts.entry(row.country).or_default();
        let key = (row.country, row.epoch);
        let count = counts.entry(key).or_default();
        let index = usize::from(row.currency == PowerCurrency::Command);
        if add {
            count[index] += 1;
        } else {
            count[index] -= 1;
        }
        let bits = (u8::from(count[0] != 0) * PoliticsCaptureCoverageRow::PP)
            | (u8::from(count[1] != 0) * PoliticsCaptureCoverageRow::CP);
        if bits != 0 {
            self.unknown_additions
                .entry(row.country)
                .or_default()
                .insert(key, bits);
        } else {
            counts.remove(&key);
            if counts.is_empty() {
                self.addition_counts.remove(&row.country);
            }
            if let Some(values) = self.unknown_additions.get_mut(&row.country) {
                values.remove(&key);
                if values.is_empty() {
                    self.unknown_additions.remove(&row.country);
                }
            }
        }
    }
}

fn change_count<K: Ord>(counts: &mut BTreeMap<K, usize>, key: K, add: bool) {
    if add {
        *counts.entry(key).or_default() += 1;
    } else if let Some(count) = counts.get_mut(&key) {
        *count -= 1;
        if *count == 0 {
            counts.remove(&key);
        }
    }
}

fn remove_member<K: Ord, V: Ord>(sets: &mut BTreeMap<K, BTreeSet<V>>, key: K, value: V) {
    if let Some(set) = sets.get_mut(&key) {
        set.remove(&value);
        if set.is_empty() {
            sets.remove(&key);
        }
    }
}

/// Old rows replaced or deleted by a delta, or its new changed rows. Snapshot
/// indexes preserve arbitrary row order; delta tables are sorted by key.
fn replaced_rows<'a, T: RowCodec>(
    rows: &'a [T],
    index: Option<&Index>,
    delta: (&[DeltaRow], &[u128]),
    owner: fn(u128) -> u32,
) -> Vec<&'a T> {
    let mut keys: Vec<_> = delta
        .0
        .iter()
        .map(|row| row.key)
        .chain(delta.1.iter().copied())
        .collect();
    keys.sort_unstable();
    keys.dedup();
    if let Some(index) = index {
        let mut selected = BTreeMap::<u32, BTreeSet<u128>>::new();
        for key in keys {
            selected.entry(owner(key)).or_default().insert(key);
        }
        selected
            .into_iter()
            .flat_map(|(raw, keys)| {
                index
                    .get(&raw)
                    .into_iter()
                    .flatten()
                    .copied()
                    .filter(move |&at| keys.contains(&rows[at].key()))
                    .map(move |at| &rows[at])
            })
            .collect()
    } else {
        keys.into_iter()
            .filter_map(|key| {
                rows.binary_search_by_key(&key, RowCodec::key)
                    .ok()
                    .map(|at| &rows[at])
            })
            .collect()
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
                let rows: &'a [T] = rows;
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

#[cfg(test)]
thread_local! {
    static ACCOUNTING_KEY_VISITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// A delta's rows are key sorted; snapshots can retain an index instead.
/// As the owner join, visit owners in order and each owner's rows in table
/// order. A changed identity's row is observed once.
fn accounting_positions<T: RowCodec>(
    rows: &[T],
    index: Option<&Index>,
    identities: &BTreeSet<u32>,
    changed: &[DeltaRow],
    pending: impl Iterator<Item = u128>,
    country: fn(&T) -> u32,
    owner: fn(u128) -> u32,
) -> Vec<usize> {
    let mut positions = Vec::new();
    for &raw in identities {
        match index {
            Some(index) => positions.extend(index.get(&raw).into_iter().flatten().copied()),
            None => {
                let first = rows.partition_point(|row| country(row) < raw);
                let last = first + rows[first..].partition_point(|row| country(row) == raw);
                positions.extend(first..last);
            }
        }
    }
    let key_of = |row: &T| {
        #[cfg(test)]
        ACCOUNTING_KEY_VISITS.with(|visits| visits.set(visits.get() + 1));
        row.key()
    };
    let keys = changed.iter().map(|row| row.key).chain(pending);
    if let Some(index) = index {
        let mut selected = BTreeMap::<u32, BTreeSet<u128>>::new();
        for key in keys {
            let raw = owner(key);
            if !identities.contains(&raw) {
                selected.entry(raw).or_default().insert(key);
            }
        }
        // Snapshot rows need not be key sorted. Scan each selected owner's
        // positions once, including when every row remains unresolved.
        for (raw, keys) in selected {
            positions.extend(
                index
                    .get(&raw)
                    .into_iter()
                    .flatten()
                    .copied()
                    .filter(|&at| keys.contains(&key_of(&rows[at]))),
            );
        }
    } else {
        for key in keys {
            if !identities.contains(&owner(key))
                && let Ok(at) = rows.binary_search_by_key(&key, key_of)
            {
                positions.push(at);
            }
        }
    }
    positions.sort_unstable_by_key(|&at| (country(&rows[at]), at));
    positions.dedup();
    positions
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

    #[test]
    fn accounting_rows_keep_owner_order_for_indexed_snapshots_and_sorted_deltas() {
        let rows = vec![
            CountryPowerChangeRow {
                country: 3,
                sequence: 2,
                ..Default::default()
            },
            CountryPowerChangeRow {
                country: 1,
                sequence: 3,
                ..Default::default()
            },
            CountryPowerChangeRow {
                country: 3,
                sequence: 1,
                ..Default::default()
            },
            CountryPowerChangeRow {
                country: 2,
                sequence: 4,
                ..Default::default()
            },
        ];
        let index = index_rows(&rows, |row| row.country);
        let identities = BTreeSet::from([3]);
        let pending = [rows[1].key(), rows[2].key()];
        let positions = accounting_positions(
            &rows,
            Some(&index),
            &identities,
            &[],
            pending.into_iter(),
            |row| row.country,
            |key| (key >> 64) as u32,
        );
        assert_eq!(positions, [1, 0, 2]);
        let mut sorted = rows;
        sorted.sort_by_key(RowCodec::key);
        let positions = accounting_positions(
            &sorted,
            None,
            &identities,
            &[],
            pending.into_iter(),
            |row| row.country,
            |key| (key >> 64) as u32,
        );
        assert_eq!(positions, [0, 2, 3]);
    }

    fn window_one_at_a_time(
        deadlines: &mut BTreeMap<i32, BTreeSet<u32>>,
        shifted: &[(u32, i32, i32)],
        hour: i32,
    ) {
        for &(raw, old, _) in shifted {
            unschedule(
                deadlines,
                raw,
                &[Some(old), hour.checked_sub(1), Some(hour)],
            );
        }
        for &(raw, _, _) in shifted {
            schedule(deadlines, raw, &[None, None, hour.checked_add(1)]);
        }
    }

    #[test]
    fn sliding_windows_move_only_their_boundaries_including_zero_width_and_integer_edges() {
        for (hour, widths) in [
            (50, vec![0, 0, 0]),
            (50, vec![0, 1, 17]),
            (i32::MIN + 1, vec![0]),
            (i32::MAX, vec![0, 1, 17]),
        ] {
            let shifted: Vec<_> = widths
                .iter()
                .enumerate()
                .map(|(at, &width)| {
                    let since = hour - width;
                    (at as u32 + 1, since - 1, since)
                })
                .collect();
            let window = WindowAdvance::new(
                shifted
                    .iter()
                    .map(|&(raw, _, since)| (raw, i64::from(hour) - i64::from(since)))
                    .collect(),
            );
            let mut original = BTreeMap::new();
            for &(raw, old, _) in &shifted {
                schedule(&mut original, raw, &[Some(old), Some(hour - 1), Some(hour)]);
            }
            // Other countries can share every boundary being moved.
            for at in [hour - 1, hour, hour.saturating_add(1)] {
                original.entry(at).or_default().insert(99);
            }
            let mut expected = original.clone();
            window_one_at_a_time(&mut expected, &shifted, hour);
            let mut ordinary = original.clone();
            shift_deadlines(&mut ordinary, &shifted, hour);
            let mut certified = original;
            window.shift(&mut certified, hour, false);
            assert_eq!(ordinary, expected, "ordinary {hour}");
            assert_eq!(certified, expected, "certificate {hour}");
        }
    }

    fn assert_carried_windows(count: u32) {
        let window = WindowAdvance::new((1..=count).map(|raw| (raw, i64::from(raw % 3))).collect());
        let mut deadlines = BTreeMap::new();
        for &(raw, width) in &window.captures {
            let since = 20 - width as i32;
            schedule(&mut deadlines, raw, &[Some(since), Some(20), Some(21)]);
        }
        for hour in 21..=28 {
            let shifted: Vec<_> = window
                .captures
                .iter()
                .map(|&(raw, width)| {
                    let since = hour - width as i32;
                    (raw, since - 1, since)
                })
                .collect();
            let mut expected = deadlines.clone();
            window_one_at_a_time(&mut expected, &shifted, hour);
            let before = WINDOW_DEADLINE_VISITS.with(std::cell::Cell::get);
            window.shift(&mut deadlines, hour, hour > 21);
            assert_eq!(deadlines, expected, "hour {hour}");
            if hour > 21 {
                assert_eq!(
                    WINDOW_DEADLINE_VISITS.with(std::cell::Cell::get),
                    before,
                    "a carried window must not revisit its {count} countries"
                );
            }
            // A bucket at the former last hour containing only other countries
            // must not be mistaken for moving members just because sizes match.
            deadlines.insert(hour, (count + 1..=count * 2).collect());
        }
    }

    #[test]
    fn sliding_windows_carry_expiry_membership_without_revisiting_countries() {
        assert_carried_windows(8);
    }

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

    fn assert_carried_expiries(count: u32) {
        let advanced: Vec<_> = (1..=count).map(|id| (id, 0)).collect();
        let raw: BTreeSet<_> = (1..=count).collect();
        let mut deadlines = BTreeMap::from([(0, raw.clone()), (1, raw)]);
        let mut oracle = deadlines.clone();
        for hour in 1..=8 {
            let before = DEADLINE_MEMBERSHIP_VISITS.with(std::cell::Cell::get);
            advance_deadlines(&mut deadlines, &advanced, hour, hour != 1);
            one_at_a_time(&mut oracle, &advanced, hour);
            assert_eq!(deadlines, oracle);
            let visits = DEADLINE_MEMBERSHIP_VISITS.with(std::cell::Cell::get) - before;
            assert_eq!(visits, if hour == 1 { count as usize } else { 0 });
        }
        // An extra unlinked deadline stays behind when the captures advance.
        deadlines.get_mut(&9).unwrap().insert(count + 1);
        oracle.get_mut(&9).unwrap().insert(count + 1);
        advance_deadlines(&mut deadlines, &advanced, 9, true);
        one_at_a_time(&mut oracle, &advanced, 9);
        assert_eq!(deadlines, oracle);
    }

    #[test]
    fn certified_expiry_buckets_carry_their_membership() {
        assert_carried_expiries(8);
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
        type Case = (i32, Vec<Capture>, Vec<Capture>, Vec<u32>);
        let cases: [Case; 5] = [
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
            advanced.sort_unstable_by_key(|&(raw, _)| raw);
            advance_deadlines(&mut deadlines, &advanced, hour, false);
            assert_eq!(deadlines, expected, "case {case}");
            assert!(
                deadlines.values().all(|raws| !raws.is_empty()),
                "case {case}: no empty deadline is kept"
            );
        }
    }
}
