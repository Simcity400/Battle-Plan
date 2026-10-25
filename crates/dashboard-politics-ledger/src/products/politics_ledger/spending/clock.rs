//! Deferred endpoints and exact per-read diagnostic counts for the accounting.
use super::*;
use dashboard_base::prepared::frame_changes::FrameChanges;

static COST_IDENTITY_FIELDS: std::sync::LazyLock<u64> = std::sync::LazyLock::new(|| {
    dashboard_base::prepared::frame_changes::field_mask(
        TableId::CountryPowerDailyCosts,
        &["epoch", "day_hour"],
    )
});

/// Remove only the observed owners' unresolved rows, whose recorded keys
/// lead with their country. Returns the keys visited.
pub(super) fn remove_pending(
    keys: &mut BTreeSet<u128>,
    owners: &BTreeSet<u32>,
    shift: u32,
) -> usize {
    let mut visits = 0;
    for &owner in owners {
        let first = u128::from(owner) << shift;
        let last = first | ((1_u128 << shift) - 1);
        let removed: Vec<_> = keys.range(first..=last).copied().collect();
        visits += removed.len();
        for key in removed {
            keys.remove(&key);
        }
    }
    visits
}

#[derive(Clone)]
struct AdvancingDeadlines {
    captures: Arc<[(u32, i32)]>,
    through: i32,
    next: Option<i32>,
}

#[derive(Clone)]
struct WindowDeadlines {
    captures: Arc<inputs::WindowAdvance>,
    through: i32,
    next: Option<i32>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub(super) struct Clock {
    pub through: Option<i32>,
    pub previous: Option<i32>,
    pub next: Option<i32>,
    pub stable: bool,
    #[serde(skip)]
    bound: bool,
    #[serde(skip)]
    pub recording: bool,
    #[serde(skip)]
    pub warnings: BTreeMap<Option<String>, BTreeMap<String, u64>>,
    #[serde(skip)]
    warning_hours: BTreeMap<Option<String>, i32>,
    #[serde(skip)]
    refused: BTreeMap<u32, (i32, u64)>,
    #[serde(skip)]
    pub tag_refusals: BTreeMap<String, BTreeMap<String, u64>>,
    #[serde(skip)]
    pub presence_refreshed: bool,
    #[serde(skip)]
    pub recheck: BTreeSet<u32>,
    #[serde(skip)]
    pub selected: BTreeSet<String>,
    #[serde(skip)]
    pub active: BTreeSet<String>,
    #[serde(skip)]
    pub holders: BTreeSet<String>,
    #[serde(skip)]
    pub census_known: bool,
    #[serde(skip)]
    advancing_deadlines: Option<AdvancingDeadlines>,
    #[serde(skip)]
    window_deadlines: Option<WindowDeadlines>,
    #[cfg(test)]
    #[serde(skip)]
    deadline_visits: usize,
}

impl Scanned {
    pub(super) fn advance(&mut self, hour: i32) -> io::Result<()> {
        let _cpu = ::replay_frame::cpu::scope_phase("politics_ledger:clock_advance");
        if self
            .clock
            .through
            .is_some_and(|last| last.checked_add(1) != Some(hour))
        {
            self.materialize_clock()?;
            self.clock.stable = false;
        }
        self.clock.previous = self.clock.through;
        self.clock.through = Some(hour);
        self.last_hour = Some(hour);
        Ok(())
    }

    pub(super) fn materialize_clock(&mut self) -> io::Result<()> {
        let Some(through) = self.clock.through else {
            return Ok(());
        };
        let mut selected = self.clock.holders.clone();
        selected.extend(self.clock.active.iter().cloned());
        selected.extend(self.clock.warnings.keys().flatten().cloned());
        self.materialize_tags(through, &selected)?;
        self.materialize_refusals(through)
    }

    fn materialize_tags(&mut self, through: i32, selected: &BTreeSet<String>) -> io::Result<()> {
        for tag in selected.iter().cloned().map(Some).chain([None]) {
            let Some(applied) = self.clock.warning_hours.get_mut(&tag) else {
                continue;
            };
            let count = u64::try_from(i64::from(through) - i64::from(*applied)).unwrap_or(0);
            *applied = (*applied).max(through);
            let Some(reasons) = self.clock.warnings.get(&tag) else {
                continue;
            };
            let target = self.warnings.entry(tag.clone()).or_default();
            for (reason, rate) in reasons {
                count_reason(
                    target,
                    reason.clone(),
                    rate.checked_mul(count).ok_or_else(|| {
                        dashboard_core::invalid("Politics warning count overflow")
                    })?,
                )?;
            }
        }
        if self.clock.census_known
            && let Some(last) = self.presence.last_mut()
        {
            last.last = last.last.max(through);
        }
        for tag in selected {
            if !self.clock.active.contains(tag) {
                continue;
            }
            let Some(country) = self.countries.get_mut(tag) else {
                continue;
            };
            if let Some(last) = country.coverage.last_mut() {
                last.through_hour = last.through_hour.map(|last| last.max(through));
            }
            if let Some(last) = country.spending_coverage.last_mut() {
                last.through_hour = last.through_hour.map(|last| last.max(through));
            }
        }
        self.holds.advance(through, selected, &self.clock.active);
        Ok(())
    }

    fn materialize_refusals(&mut self, through: i32) -> io::Result<()> {
        for (slot, (applied, rate)) in &mut self.clock.refused {
            let count = u64::try_from(i64::from(through) - i64::from(*applied)).unwrap_or(0);
            let (_, total) = self
                .refused
                .get_mut(slot)
                .expect("observed refused country");
            *total = total
                .checked_add(
                    count
                        .checked_mul(*rate)
                        .ok_or_else(|| dashboard_core::invalid("Refused country count overflow"))?,
                )
                .ok_or_else(|| dashboard_core::invalid("Refused country count overflow"))?;
            *applied = (*applied).max(through);
        }
        Ok(())
    }

    /// Country balances are not identity operands. The shared projection avoids
    /// joins when only those unrelated fields supplied a new allocation.
    pub(super) fn inputs_changed(&mut self, frame: &DecodedFrame, changes: &FrameChanges) -> bool {
        let _cpu = ::replay_frame::cpu::scope_phase("politics_ledger:inputs_changed");
        self.clock.advancing_deadlines = None;
        self.clock.window_deadlines = None;
        let refused_tags: Vec<_> = if changes.rows_changed[TableId::Countries.index()]
            || changes.table_read[TableId::Countries.index()]
        {
            self.inputs
                .refused
                .iter()
                .filter_map(|row| self.known_tags.get(&row.canonical_slot).cloned())
                .collect()
        } else {
            Vec::new()
        };
        // Changed rows describe the hour after the last one this observed.
        let continuous =
            self.clock.previous.and_then(|last| last.checked_add(1)) == Some(frame.hour);
        let pending = (self.pending_known && continuous).then_some(&self.pending);
        #[cfg(test)]
        let pending = pending.filter(|_| !self.whole_tables);
        Arc::make_mut(&mut self.inputs).refresh_changes(frame, changes, pending);
        // Cost amounts and presentation operands do not change the retained
        // identities. Additions cannot remove one either; deletions and edits
        // to the occurrence's identity can, as can a whole-table read.
        if self.inputs.costs_changed
            && changes
                .rows(TableId::CountryPowerDailyCosts)
                .is_none_or(|(changed, deleted)| {
                    !deleted.is_empty()
                        || changed
                            .iter()
                            .any(|row| row.mask & *COST_IDENTITY_FIELDS != 0)
                })
        {
            self.costs_pruned = false;
        }
        #[cfg(test)]
        {
            self.row_hours += usize::from(self.inputs.narrow);
        }
        // A holder's name is read from the hour's characters, unit leaders
        // and dictionary; only holders named from a changed row can change.
        #[cfg(not(test))]
        let rows_known = true;
        #[cfg(test)]
        let rows_known = !self.whole_tables;
        let named = self.holds.naming_slots(changes, rows_known);
        if !self.clock.bound {
            // Resume rebuilt the old join for transition comparison, but the
            // first new read must still bind its per-read diagnostic rates.
            Arc::make_mut(&mut self.inputs).countries_changed = true;
        }
        let recheck = std::mem::take(&mut self.clock.recheck);
        let changed = self.inputs.changed || !recheck.is_empty() || !named.is_empty();
        let inputs = Arc::make_mut(&mut self.inputs);
        inputs.slots.extend(recheck);
        inputs.slots.extend(named);
        if self.clock.previous.and_then(|last| last.checked_add(1)) != Some(frame.hour)
            || self.clock.census_known != frame.complete::<tables::Countries>().is_ok()
        {
            // Reproject known tags without inventing a native holder change.
            // Holds uses previous_tags only for actual census transitions.
            self.dirty.extend(self.clock.holders.iter().cloned());
        }
        if self.inputs.countries_changed || !self.clock.bound {
            self.clock.bound = true;
            self.dirty.extend(refused_tags);
        }
        changed
    }

    /// Whether an hour a pre-pass worker certified for the current join
    /// (every changed capture, `advanced`, only extends to the hour and
    /// nothing else joins a country) only advances captures here too, with
    /// no day boundary due (`day_due`): [`Self::inputs_changed`] would join
    /// it from its changed rows, observing again no row not yet recorded as
    /// an event, dirtying no country and checking none again; no linked
    /// capture is due once the captures moved; and [`Self::advances_only`]
    /// holds. No accounting state changes; deadline checks can retain a
    /// transient proof.
    pub(super) fn certified_advance(
        &mut self,
        frame: &DecodedFrame,
        changes: &FrameChanges,
        advanced: &Arc<[(u32, i32)]>,
        day_due: bool,
    ) -> bool {
        let _cpu = ::replay_frame::cpu::scope_phase("politics_ledger:certified_check");
        if advanced.is_empty() || !self.certifies_join(frame, changes, day_due) {
            return false;
        }
        // The captures due at the hour that the whole accounting observes.
        !self
            .clock
            .next
            .is_some_and(|next| next <= frame.hour && self.captures_due(next, frame.hour, advanced))
    }

    fn certifies_join(&self, frame: &DecodedFrame, changes: &FrameChanges, day_due: bool) -> bool {
        let read = |table: TableId| {
            changes.rows_changed[table.index()] || changes.table_read[table.index()]
        };
        #[cfg(not(test))]
        let rows_known = true;
        #[cfg(test)]
        let rows_known = !self.whole_tables;
        // The hour continues the last one joined, from its changed rows.
        let continuous =
            self.clock.through.and_then(|last| last.checked_add(1)) == Some(frame.hour);
        if day_due
            || !continuous
            || !self.pending_known
            || !rows_known
            || !self.clock.stable
            || !self.clock.bound
            || !self.clock.recheck.is_empty()
            || self.clock.presence_refreshed
            || !self.dirty.is_empty()
            || self.clock.census_known != frame.complete::<tables::Countries>().is_ok()
            || !self.holds.naming_slots(changes, rows_known).is_empty()
        {
            return false;
        }
        // The rows not yet recorded as events that the join would observe
        // again: every one of a table read whole, and those of a linked
        // country with links read or of a captured one.
        let links = read(TableId::CountryLinks);
        let coverage = read(TableId::PoliticsCaptureCoverage);
        let inputs = &self.inputs;
        for (keys, shift, whole) in [
            (&self.pending[0], 64, read(TableId::CountryPowerChanges)),
            (&self.pending[1], 32, read(TableId::CountryPowerDailyCosts)),
        ] {
            if keys.iter().any(|&key| {
                let raw = (key >> shift) as u32;
                whole
                    || (links && inputs.links.contains_key(&raw))
                    || (coverage && inputs.witnesses.contains_key(&raw))
            }) {
                return false;
            }
        }
        true
    }

    pub(super) fn certified_window(
        &mut self,
        frame: &DecodedFrame,
        changes: &FrameChanges,
        window: &Arc<inputs::WindowAdvance>,
        day_due: bool,
    ) -> bool {
        let _cpu = ::replay_frame::cpu::scope_phase("politics_ledger:window_certificate");
        if !self.certifies_join(frame, changes, day_due) {
            return false;
        }
        if self.clock.next.is_none_or(|next| next > frame.hour) {
            return true;
        }
        if let Some(held) = &mut self.clock.window_deadlines
            && Arc::ptr_eq(&held.captures, window)
            && held.through.checked_add(1) == Some(frame.hour)
        {
            held.through = frame.hour;
            return held.next.is_none_or(|next| next > frame.hour);
        }
        let _cpu = ::replay_frame::cpu::scope_phase("politics_ledger:window_capture_deadlines");
        let next = self
            .inputs
            .deadlines
            .range(self.clock.next.unwrap()..)
            .find_map(|(&hour, rows)| {
                rows.iter()
                    .any(|raw| {
                        #[cfg(test)]
                        {
                            self.clock.deadline_visits += 1;
                        }
                        self.inputs.links.contains_key(raw) && !window.contains(raw)
                    })
                    .then_some(hour)
            });
        self.clock.window_deadlines = Some(WindowDeadlines {
            captures: Arc::clone(window),
            through: frame.hour,
            next,
        });
        next.is_none_or(|next| next > frame.hour)
    }

    pub(super) fn inputs_window(
        &mut self,
        frame: &DecodedFrame,
        changes: &FrameChanges,
        window: &Arc<inputs::WindowAdvance>,
    ) {
        let _cpu = ::replay_frame::cpu::scope_phase("politics_ledger:inputs_window");
        Arc::make_mut(&mut self.inputs).refresh_window(frame, changes, window);
        #[cfg(test)]
        {
            self.row_hours += 1;
        }
    }

    /// During one certified span only these captures' last hours and expiries
    /// move. Every other linked deadline keeps its date until an ordinary join.
    fn captures_due(&mut self, next: i32, hour: i32, captures: &Arc<[(u32, i32)]>) -> bool {
        if let Some(held) = &mut self.clock.advancing_deadlines
            && Arc::ptr_eq(&held.captures, captures)
            && held.through.checked_add(1) == Some(hour)
        {
            held.through = hour;
            let due = held.next.is_some_and(|next| next <= hour);
            #[cfg(test)]
            assert_eq!(due, self.inputs.due_after_advance(next, hour, captures));
            return due;
        }
        let _cpu = ::replay_frame::cpu::scope_phase("politics_ledger:capture_deadlines");
        let due = self.inputs.deadlines.range(next..).find_map(|(&at, raws)| {
            raws.iter()
                .any(|raw| {
                    #[cfg(test)]
                    {
                        self.clock.deadline_visits += 1;
                    }
                    if !self.inputs.links.contains_key(raw) {
                        return false;
                    }
                    let moving = captures.binary_search_by_key(raw, |&(raw, _)| raw).ok();
                    match moving {
                        None => true,
                        Some(index) if at > hour => captures[index].1 == at,
                        Some(index) => {
                            at != hour
                                && !(Some(at) == hour.checked_sub(1) && captures[index].1 != at)
                        }
                    }
                })
                .then_some(at)
        });
        // A captured future start can coincide with an expiry removed by a
        // later advance. Keep checking those unusual intervals at each hour.
        self.clock.advancing_deadlines =
            captures
                .iter()
                .all(|(_, since)| *since < hour)
                .then(|| AdvancingDeadlines {
                    captures: Arc::clone(captures),
                    through: hour,
                    next: due,
                });
        due.is_some_and(|next| next <= hour)
    }

    /// [`Self::inputs_changed`] of an hour [`Self::certified_advance`]
    /// accepted: its join from its changed rows only extends the captures
    /// (`Inputs::refresh_advanced`); no country is dirtied, named again or
    /// checked again. Whether anything joined changed, as there.
    pub(super) fn inputs_advanced(
        &mut self,
        frame: &DecodedFrame,
        changes: &FrameChanges,
        advanced: &Arc<[(u32, i32)]>,
    ) -> bool {
        let _cpu = ::replay_frame::cpu::scope_phase("politics_ledger:inputs_advanced");
        Arc::make_mut(&mut self.inputs).refresh_advanced(frame, changes, advanced);
        #[cfg(test)]
        {
            self.row_hours += 1;
        }
        self.inputs.changed
    }

    /// Move captures extended without moving their witnesses to the last
    /// hour they reached.
    pub(super) fn materialize_advance(&mut self) {
        if self.inputs.deferred() {
            Arc::make_mut(&mut self.inputs).materialize_advance();
        }
    }

    pub(super) fn begin_accounting(&mut self, frame: &DecodedFrame) -> io::Result<()> {
        let _cpu = ::replay_frame::cpu::scope_phase("politics_ledger:accounting_begin");
        self.clock.selected = self
            .inputs
            .slots
            .iter()
            .filter_map(|slot| self.inputs.countries.get(slot))
            .filter_map(|row| valid_recorded_tag(row).ok().map(str::to_owned))
            .chain(self.inputs.previous_tags.iter().cloned())
            .chain(self.dirty.iter().cloned())
            .collect();
        if let Some(previous) = self.clock.previous {
            let selected = std::mem::take(&mut self.clock.selected);
            let result = self.materialize_tags(previous, &selected);
            self.clock.selected = selected;
            result?;
            if self.inputs.countries_changed {
                self.materialize_refusals(previous)?;
            }
        }
        self.clock.stable = true;
        for tag in &self.clock.selected {
            self.clock.warnings.remove(&Some(tag.clone()));
            self.clock
                .warning_hours
                .insert(Some(tag.clone()), frame.hour);
        }
        if self.inputs.countries_changed
            || self.clock.previous.and_then(|hour| hour.checked_add(1)) != Some(frame.hour)
        {
            self.clock.refused.clear();
            for row in &self.inputs.refused {
                self.clock
                    .refused
                    .entry(row.canonical_slot)
                    .or_insert((frame.hour, 0))
                    .1 += 1;
            }
            self.clock.active = self
                .inputs
                .countries
                .values()
                .map(|row| String::from_utf8(row.tag.clone()).expect("validated tag"))
                .collect();
            self.clock.holders.extend(self.clock.active.iter().cloned());
            self.clock
                .holders
                .extend(self.inputs.previous_tags.iter().cloned());
            self.clock.holders.extend(
                self.inputs
                    .refused
                    .iter()
                    .filter_map(|row| self.known_tags.get(&row.canonical_slot).cloned()),
            );
        }
        self.clock.census_known = frame.complete::<tables::Countries>().is_ok();
        self.clock.next = self.next_deadline(frame.hour);
        self.clock.recording = true;
        Ok(())
    }

    /// The first hour after `hour` at which a capture's relation to the
    /// current hour changes.
    fn next_deadline(&self, hour: i32) -> Option<i32> {
        self.inputs
            .deadlines
            .range((std::ops::Bound::Excluded(hour), std::ops::Bound::Unbounded))
            .next()
            .map(|(&hour, _)| hour)
    }

    /// Whether the hour just joined ([`Self::inputs_changed`]) only advances
    /// captures: joined from its changed rows, which join no country
    /// ([`super::inputs::Inputs::joins_nothing`]), it continues the hour before
    /// with the census as readable as there, no country is dirty or to be
    /// checked again, and the per-read rates are bound. The whole accounting
    /// then selects no country: [`Self::advance_accounting`] is all it does.
    pub(super) fn advances_only(&self, frame: &DecodedFrame) -> bool {
        self.inputs.joins_nothing()
            && self.dirty.is_empty()
            && self.clock.recheck.is_empty()
            && self.clock.stable
            && self.clock.bound
            && !self.clock.presence_refreshed
            && self.clock.previous.and_then(|last| last.checked_add(1)) == Some(frame.hour)
            && self.clock.census_known == frame.complete::<tables::Countries>().is_ok()
    }

    /// The accounting of an hour that only advances captures
    /// ([`Self::advances_only`]): what [`Self::begin_accounting`],
    /// [`observe_accounting`] and [`Self::end_accounting`] do when no country
    /// is selected, joined or dirty. The deferred endpoints materialize
    /// through the hour before for no tag but the branch's own warnings; the
    /// census run extends; modifier formats follow the hour's dictionary;
    /// hold names are indexed once after a resume; the read is kept for the
    /// next. Every per-country step visits no country there, and no pending
    /// row changes, so none is taken.
    pub(super) fn advance_accounting(
        &mut self,
        frame: &DecodedFrame,
        content: Option<&PoliticsContent>,
    ) -> io::Result<()> {
        let _cpu = ::replay_frame::cpu::scope_phase("politics_ledger:advance_accounting");
        let hour = frame.hour;
        // As begin_accounting with nothing selected.
        self.clock.selected.clear();
        if let Some(previous) = self.clock.previous {
            self.materialize_tags(previous, &BTreeSet::new())?;
        }
        self.clock.stable = true;
        self.clock.census_known = frame.complete::<tables::Countries>().is_ok();
        self.clock.next = self.next_deadline(hour);
        self.clock.recording = true;
        // As observe_accounting: its event, identity, diagnostic and row
        // steps visit none of the joined countries, of which there are none.
        super::observe_presence(self, frame)?;
        self.refresh_modifier_formats(frame);
        super::holds::observe_holds(frame, self, content);
        self.first_hour.get_or_insert(hour);
        self.last_hour = Some(self.last_hour.map_or(hour, |last| last.max(hour)));
        self.retain_previous(frame);
        // As end_accounting: the presence refresh is not pending, the holds
        // of no selected tag advance, and with nothing observed the pending
        // rows stay those already known.
        self.clock.recording = false;
        Ok(())
    }

    pub(super) fn end_accounting(&mut self, hour: i32) {
        let _cpu = ::replay_frame::cpu::scope_phase("politics_ledger:accounting_end");
        if self.clock.presence_refreshed {
            // A refused alias before its first valid alias has no known tag on
            // the first read. Subsequent equal censuses use the final known tag.
            // Keep that steady rate separately from the first read's exact count.
            for (tag, reasons) in &self.clock.tag_refusals {
                if let Some(rates) = self.clock.warnings.get_mut(&Some(tag.clone())) {
                    for reason in reasons.keys() {
                        rates.remove(reason);
                    }
                }
            }
            self.clock.tag_refusals.clear();
            let mut tags = BTreeMap::new();
            for row in &self.inputs.census {
                match valid_recorded_tag(row) {
                    Ok(tag) => {
                        tags.insert(row.canonical_slot, tag.to_owned());
                    }
                    Err(reason) => {
                        if let Some(tag) = tags
                            .get(&row.canonical_slot)
                            .or_else(|| self.known_tags.get(&row.canonical_slot))
                        {
                            *self
                                .clock
                                .tag_refusals
                                .entry(tag.clone())
                                .or_default()
                                .entry(reason)
                                .or_default() += 1;
                        }
                    }
                }
            }
            for (tag, reasons) in &self.clock.tag_refusals {
                self.clock
                    .warnings
                    .entry(Some(tag.clone()))
                    .or_default()
                    .extend(reasons.clone());
                self.clock.warning_hours.insert(Some(tag.clone()), hour);
            }
            self.clock.presence_refreshed = false;
        }
        if self.clock.previous.and_then(|last| last.checked_add(1)) == Some(hour) {
            for tag in &self.clock.selected {
                if !self.clock.active.contains(tag) {
                    continue;
                }
                let Some(country) = self.countries.get_mut(tag) else {
                    continue;
                };
                if let Some(last) = country.coverage.last_mut() {
                    last.through_hour = Some(hour);
                }
                if let Some(last) = country.spending_coverage.last_mut() {
                    last.through_hour = Some(hour);
                }
            }
            self.holds
                .advance(hour, &self.clock.selected, &self.clock.active);
        }
        self.clock.recording = false;
        self.refresh_pending();
    }

    /// The power and cost rows not recorded as events: their identity had no
    /// tag when observed. After an hour joined from its changed rows, only
    /// the observed countries' rows can have been recorded or removed.
    fn refresh_pending(&mut self) {
        let inputs = Arc::clone(&self.inputs);
        let narrow = inputs.narrow && self.pending_known;
        if narrow {
            let observed: [BTreeSet<u32>; 2] = [
                inputs.observed(0).copied().collect(),
                inputs.observed(3).copied().collect(),
            ];
            remove_pending(&mut self.pending[0], &observed[0], 64);
            remove_pending(&mut self.pending[1], &observed[1], 32);
        } else {
            self.pending = Default::default();
        }
        let power: Box<dyn Iterator<Item = &CountryPowerChangeRow> + '_> = if narrow {
            inputs.accounting_power()
        } else {
            Box::new(inputs.power().iter())
        };
        for row in power {
            if !self
                .events
                .contains(&(row.country, row.epoch, row.sequence))
            {
                self.pending[0].insert(row.key());
            }
        }
        let costs: Box<dyn Iterator<Item = &CountryPowerDailyCostRow> + '_> = if narrow {
            inputs.accounting_cost()
        } else {
            Box::new(inputs.costs().iter())
        };
        for row in costs {
            if !self
                .costs
                .contains(&(row.country, row.epoch, row.day_hour, row.ordinal))
            {
                self.pending[1].insert(row.key());
            }
        }
        self.pending_known = true;
    }
}

#[cfg(test)]
mod deadline_tests {
    use super::*;

    fn advancing_deadlines_follow_their_span(count: u32) {
        let mut scan = Scanned::default();
        let captures: Arc<[(u32, i32)]> = (1..=count).map(|id| (id, 0)).collect();
        let inputs = Arc::make_mut(&mut scan.inputs);
        inputs.links = (1..=count + 1).map(|id| (id, id)).collect();
        // The captures at every accepted hour expire at the next, while a
        // different country's future deadline must still stop the carry.
        inputs.deadlines.insert(1, (1..=count).collect());
        inputs.deadlines.insert(6, BTreeSet::from([count + 1]));
        for hour in 1..=6 {
            let before = scan.clock.deadline_visits;
            let expected = scan.inputs.due_after_advance(hour, hour, &captures);
            assert_eq!(
                scan.captures_due(hour, hour, &captures),
                expected,
                "hour {hour}"
            );
            if hour > 1 {
                assert_eq!(
                    scan.clock.deadline_visits, before,
                    "unchanged advancing membership walked {count} captures"
                );
            }
            assert_eq!(expected, hour == 6);
            let inputs = Arc::make_mut(&mut scan.inputs);
            let mut moving = inputs.deadlines.remove(&hour).unwrap();
            moving.remove(&(count + 1));
            inputs.deadlines.entry(hour + 1).or_default().extend(moving);
        }
        // Another span, even with the same rows, cannot use the former proof.
        let replacement: Arc<[(u32, i32)]> = captures.to_vec().into();
        let before = scan.clock.deadline_visits;
        assert_eq!(
            scan.captures_due(7, 7, &replacement),
            scan.inputs.due_after_advance(7, 7, &replacement)
        );
        assert!(scan.clock.deadline_visits > before);
    }

    fn window_deadlines_follow_their_span(count: u32) {
        let mut scan = Scanned::default();
        scan.clock.stable = true;
        scan.clock.bound = true;
        scan.clock.census_known = true;
        scan.pending_known = true;
        let window = Arc::new(inputs::WindowAdvance::new(
            (1..=count).map(|raw| (raw, 0)).collect(),
        ));
        let inputs = Arc::make_mut(&mut scan.inputs);
        inputs.links = (1..=count + 1).map(|raw| (raw, raw)).collect();
        inputs.deadlines.insert(1, (1..=count).collect());
        inputs.deadlines.insert(6, BTreeSet::from([count + 1]));
        let changes = FrameChanges {
            rows_changed: [false; replay_data::rows::TABLE_COUNT],
            table_read: [false; replay_data::rows::TABLE_COUNT],
            slot_read: [false; replay_data::rows::TABLE_COUNT],
            updates: None,
        };
        let mut frame = super::super::tests::frame(1, 1);
        for hour in 1..=6 {
            frame.hour = hour;
            scan.clock.through = Some(hour - 1);
            scan.clock.next = Some(hour);
            let before = scan.clock.deadline_visits;
            assert_eq!(
                scan.certified_window(&frame, &changes, &window, false),
                hour < 6
            );
            if hour > 1 {
                assert_eq!(
                    scan.clock.deadline_visits, before,
                    "a carried window does not scan {count} captures again"
                );
            }
            let inputs = Arc::make_mut(&mut scan.inputs);
            let mut moving = inputs.deadlines.remove(&hour).unwrap();
            moving.remove(&(count + 1));
            inputs
                .deadlines
                .entry(hour + 1)
                .or_default()
                .append(&mut moving);
        }
        let replacement = Arc::new(inputs::WindowAdvance::new(window.captures.clone()));
        scan.clock.next = Some(7);
        scan.clock.through = Some(6);
        frame.hour = 7;
        let before = scan.clock.deadline_visits;
        assert!(scan.certified_window(&frame, &changes, &replacement, false));
        assert!(
            scan.clock.deadline_visits > before,
            "another certificate rechecks the deadlines"
        );
    }

    #[test]
    fn sliding_window_certificates_stop_at_other_countries_deadlines() {
        window_deadlines_follow_their_span(8);
    }

    #[test]
    fn future_capture_starts_keep_each_hours_exact_deadline_check() {
        let mut scan = Scanned::default();
        let captures: Arc<[(u32, i32)]> = vec![(1, 3)].into();
        Arc::make_mut(&mut scan.inputs).links.insert(1, 1);
        for hour in 1..=4 {
            let inputs = Arc::make_mut(&mut scan.inputs);
            inputs.deadlines.clear();
            inputs.deadlines.insert(hour, BTreeSet::from([1]));
            if hour < 3 {
                inputs.deadlines.insert(3, BTreeSet::from([1]));
            }
            let expected = scan.inputs.due_after_advance(hour, hour, &captures);
            assert_eq!(scan.captures_due(hour, hour, &captures), expected);
            if hour <= 3 {
                assert!(scan.clock.advancing_deadlines.is_none());
            }
        }
    }

    #[test]
    fn capture_deadlines_carry_membership_but_reach_other_countries_deadlines() {
        advancing_deadlines_follow_their_span(8);
    }
}
