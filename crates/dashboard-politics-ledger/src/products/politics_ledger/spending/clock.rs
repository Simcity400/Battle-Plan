//! Deferred endpoints and exact per-read diagnostic counts for the accounting.
use super::*;
use dashboard_base::prepared::frame_changes::FrameChanges;

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
}

impl Scanned {
    pub(super) fn advance(&mut self, hour: i32) -> io::Result<()> {
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
        // Only a join of the whole tables replaces the current cost keys.
        if !self.inputs.narrow && self.inputs.costs_changed {
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

    pub(super) fn begin_accounting(&mut self, frame: &DecodedFrame) -> io::Result<()> {
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
            let selected = self.clock.selected.clone();
            self.materialize_tags(previous, &selected)?;
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
            self.pending[0].retain(|key| !observed[0].contains(&((key >> 64) as u32)));
            self.pending[1].retain(|key| !observed[1].contains(&((key >> 32) as u32)));
        } else {
            self.pending = Default::default();
        }
        let power: Box<dyn Iterator<Item = &CountryPowerChangeRow> + '_> = if narrow {
            Box::new(inputs.power_rows())
        } else {
            Box::new(inputs.power().iter())
        };
        for row in power {
            if !self
                .events
                .contains(&(row.country, row.instance, row.epoch, row.sequence))
            {
                self.pending[0].insert(row.key());
            }
        }
        let costs: Box<dyn Iterator<Item = &CountryPowerDailyCostRow> + '_> = if narrow {
            Box::new(inputs.cost_rows())
        } else {
            Box::new(inputs.costs().iter())
        };
        for row in costs {
            if !self.costs.contains(&(
                row.country,
                row.instance,
                row.epoch,
                row.day_hour,
                row.ordinal,
            )) {
                self.pending[1].insert(row.key());
            }
        }
        self.pending_known = true;
    }
}
