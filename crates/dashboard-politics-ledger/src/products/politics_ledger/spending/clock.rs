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
        Arc::make_mut(&mut self.inputs).refresh(frame, Some(changes));
        if !self.clock.bound {
            // Resume rebuilt the old join for transition comparison, but the
            // first new read must still bind its per-read diagnostic rates.
            Arc::make_mut(&mut self.inputs).countries_changed = true;
        }
        let recheck = std::mem::take(&mut self.clock.recheck);
        let changed = self.inputs.changed || !recheck.is_empty();
        Arc::make_mut(&mut self.inputs).slots.extend(recheck);
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
        self.clock.next = self
            .inputs
            .deadlines
            .range((
                std::ops::Bound::Excluded(frame.hour),
                std::ops::Bound::Unbounded,
            ))
            .next()
            .map(|(&hour, _)| hour);
        self.clock.recording = true;
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
    }
}
