use super::*;
use dashboard_base::products::{
    country_history::{Identity, observe_identity},
    equipment::VariantCatalogue,
    force_census::{Countries, changed_ids},
};
use dashboard_text::metadata::{IdentityTrack, slot_read, validate_campaign};
use replay::{
    container::Replay,
    frame::{DecodedFrame, Frame, FrameDecoder},
    rows::{AirWingRow, TableId, WingEquipmentRow, tables},
};
use std::{
    collections::{HashMap, HashSet},
    sync::atomic::Ordering,
};

#[derive(Default, Serialize, Deserialize)]
pub(super) struct Track {
    pub(super) runs: Vec<RawRun>,
    pub(super) segments: Vec<HourSpan>,
    last_identity: IdentityTrack,
    last_known: Option<(Identity, i32)>,
}

impl Track {
    // Only called within an authenticated span with unchanged identity and values.
    fn extend(&mut self, identity: Identity, hour: i32) {
        self.segments
            .last_mut()
            .expect("identity segment")
            .last_hour = hour;
        self.last_identity.advance(hour);
        self.runs.last_mut().expect("observed counts").last_hour = hour;
        if self.last_known.is_some() {
            self.last_known = Some((identity, hour));
        }
    }

    fn observe(
        &mut self,
        identity: Identity,
        hour: i32,
        roster: Option<Arc<Counts>>,
        effective: Option<Arc<Counts>>,
    ) {
        observe_identity(&mut self.segments, &mut self.last_identity, identity, hour);
        let known = roster.is_some() || effective.is_some();
        let continues = known
            && self
                .last_known
                .is_some_and(|(seen, at)| seen == identity && at.checked_add(1) == Some(hour));
        match self.runs.last_mut() {
            Some(run) if continues && run.roster == roster && run.effective == effective => {
                run.last_hour = hour;
            }
            _ => self.runs.push(RawRun {
                first_hour: hour,
                last_hour: hour,
                starts_segment: !continues,
                roster,
                effective,
            }),
        }
        self.last_known = known.then_some((identity, hour));
    }
}

#[derive(Default, Serialize, Deserialize)]
pub(super) struct Histories {
    #[serde(serialize_with = "dashboard_core::map_pairs::ordered_map")]
    pub(super) tags: HashMap<String, usize>,
    pub(super) tracks: Vec<Track>,
    #[serde(skip)]
    pub(super) active: Vec<(usize, Identity)>,
    #[serde(skip)]
    pub(super) active_tags: Vec<String>,
    #[serde(skip)]
    pub(super) observed: Vec<String>,
    #[serde(skip)]
    pub(super) removed: Vec<String>,
    #[serde(skip)]
    slots: HashMap<u32, Vec<usize>>,
    #[serde(skip)]
    undisplayable: Vec<Vec<u8>>,
    // Untouched tracks extend lazily to the last authenticated hour. Flush before
    // topology/coverage changes, so absent countries and gaps never get carried.
    pub(super) through: Option<i32>,
    #[cfg(test)]
    pub(super) projected: usize,
}

impl Histories {
    pub(super) fn flush(&mut self) {
        if let Some(hour) = self.through.take() {
            for &(track, identity) in &self.active {
                self.tracks[track].extend(identity, hour);
            }
        }
    }

    fn countries(&mut self, countries: &Countries) {
        let previous = std::mem::take(&mut self.active_tags);
        self.active.clear();
        self.slots.clear();
        self.undisplayable.clear();
        for (tag, identity) in &countries.tags {
            let Ok(tag) = std::str::from_utf8(tag) else {
                self.undisplayable.push(tag.clone());
                continue;
            };
            let track = if let Some(&index) = self.tags.get(tag) {
                index
            } else {
                let index = self.tracks.len();
                self.tracks.push(Track::default());
                self.tags.insert(tag.to_owned(), index);
                index
            };
            if let Some(identity) = *identity {
                self.slots
                    .entry(identity.0)
                    .or_default()
                    .push(self.active.len());
                self.active.push((track, identity));
                self.active_tags.push(tag.to_owned());
            }
        }
        let current: HashSet<_> = self.active_tags.iter().map(String::as_str).collect();
        self.removed.extend(
            previous
                .into_iter()
                .filter(|tag| !current.contains(tag.as_str())),
        );
    }

    fn observe(
        &mut self,
        active: usize,
        hour: i32,
        complete: [bool; 2],
        census: &mut Census,
        pool: &mut HashSet<Arc<Counts>>,
    ) -> io::Result<()> {
        let (track, identity) = self.active[active];
        let track = &mut self.tracks[track];
        if let Some(previous) = self.through {
            track.extend(identity, previous);
        }
        let roster = if complete[0] {
            Some(census.counts(0, identity.0, pool)?)
        } else {
            None
        };
        let effective = if complete[1] {
            Some(census.counts(1, identity.0, pool)?)
        } else {
            None
        };
        track.observe(identity, hour, roster, effective);
        if track.runs.last().is_some_and(|run| run.first_hour == hour) {
            self.observed.push(self.active_tags[active].clone());
        }
        #[cfg(test)]
        {
            self.projected += 1;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct EquipmentNotes {
    missing: bool,
    refused: bool,
    remainders: bool,
}

fn add(total: &mut i64, amount: i64) -> io::Result<()> {
    *total = total
        .checked_add(amount)
        .filter(|value| value.abs() <= MAX_EXACT)
        .ok_or_else(|| io::Error::other("Air force count exceeds exact integer range"))?;
    Ok(())
}

fn add_part(
    parts: &mut BTreeMap<String, Option<i64>>,
    key: &str,
    amount: Option<i64>,
) -> io::Result<()> {
    if let Some(total) = parts.get_mut(key) {
        match (total.as_mut(), amount) {
            (Some(total), Some(amount)) => add(total, amount)?,
            (_, None) => *total = None,
            _ => {}
        }
    } else {
        parts.insert(key.to_owned(), amount);
    }
    Ok(())
}

/// The recorded plane count truncates the sum of the wing's fixed5 equipment
/// bank divided by 100000. The total uses that recorded word even if the
/// companion bank is missing or cannot reconcile; no stockpile is joined.
fn wing_counts(
    wing: &AirWingRow,
    equipment: Option<&[WingEquipmentRow]>,
    tokens: &HashMap<u64, String>,
    notes: &mut EquipmentNotes,
) -> io::Result<Counts> {
    let mut counts = Counts {
        planes: i64::from(wing.planes),
        wings: 1,
        ..Default::default()
    };
    let Some(entries) = equipment else {
        notes.missing = true;
        counts
            .planes_by_token
            .insert(UNIDENTIFIED.into(), Some(counts.planes));
        counts.wings_by_tokens.insert(vec![UNIDENTIFIED.into()], 1);
        return Ok(counts);
    };
    let token_of = |entry: &WingEquipmentRow| {
        tokens
            .get(&entry.variant)
            .cloned()
            .unwrap_or_else(|| format!("{}/", entry.variant))
    };
    let mut signature: Vec<_> = entries
        .iter()
        .filter(|entry| entry.amount_raw > 0)
        .map(&token_of)
        .collect();
    signature.sort_unstable();
    signature.dedup();
    if signature.is_empty() {
        signature.push(UNIDENTIFIED.into());
    }
    counts.wings_by_tokens.insert(signature, 1);
    let scale = engine::equipment_entry::FIXED_POINT_SCALE;
    let summed: i128 = entries
        .iter()
        .map(|entry| i128::from(entry.amount_raw))
        .sum();
    let exactly_splittable = entries
        .iter()
        .all(|entry| entry.amount_raw >= 0 && entry.amount_raw % scale == 0)
        && summed <= i128::from(wing.planes) * i128::from(scale);
    if !exactly_splittable {
        notes.refused = true;
        for entry in entries {
            counts.planes_by_token.insert(token_of(entry), None);
        }
        counts
            .planes_by_token
            .insert(UNIDENTIFIED.into(), Some(counts.planes));
        return Ok(counts);
    }
    let mut identified = 0;
    for entry in entries {
        let amount = entry.amount_raw / scale;
        add(&mut identified, amount)?;
        add_part(&mut counts.planes_by_token, &token_of(entry), Some(amount))?;
    }
    let remainder = counts.planes - identified;
    if remainder != 0 {
        notes.remainders = true;
        counts
            .planes_by_token
            .insert(UNIDENTIFIED.into(), Some(remainder));
    }
    Ok(counts)
}

// Reversible contributions keep unknown part evidence separate from known sums.
// Removing the last unknown contributor restores the exact remaining amount.
#[derive(Default)]
struct PartSum {
    sum: i128,
    entries: i64,
    unknown: i64,
}

#[derive(Default)]
struct CountryCounts {
    planes: i128,
    wings: i128,
    parts: BTreeMap<String, PartSum>,
    signatures: BTreeMap<Vec<String>, i64>,
    snapshot: Option<Arc<Counts>>,
}

impl CountryCounts {
    fn replace(&mut self, before: &Counts, after: &Counts) {
        // Attrition usually changes amounts under the same keys. Keep entries
        // and wing signatures in place instead of removing and re-adding them.
        if before
            .planes_by_token
            .keys()
            .eq(after.planes_by_token.keys())
        {
            self.planes += i128::from(after.planes) - i128::from(before.planes);
            for ((key, before), (_, after)) in
                before.planes_by_token.iter().zip(&after.planes_by_token)
            {
                let part = self.parts.get_mut(key).expect("counted wing part");
                part.sum += i128::from(after.unwrap_or(0)) - i128::from(before.unwrap_or(0));
                part.unknown += i64::from(after.is_none()) - i64::from(before.is_none());
            }
            if before.wings_by_tokens != after.wings_by_tokens {
                self.change_signatures(before, -1);
                self.change_signatures(after, 1);
            } else {
                self.wings += i128::from(after.wings) - i128::from(before.wings);
            }
            self.snapshot = None;
        } else {
            self.change(before, -1);
            self.change(after, 1);
        }
    }

    fn change(&mut self, counts: &Counts, sign: i64) {
        self.planes += i128::from(sign) * i128::from(counts.planes);
        for (key, amount) in &counts.planes_by_token {
            let part = if self.parts.contains_key(key) {
                self.parts.get_mut(key).expect("existing part")
            } else {
                self.parts.entry(key.clone()).or_default()
            };
            part.entries += sign;
            match amount {
                Some(amount) => part.sum += i128::from(sign) * i128::from(*amount),
                None => part.unknown += sign,
            }
            if part.entries == 0 {
                self.parts.remove(key);
            }
        }
        self.change_signatures(counts, sign);
        self.snapshot = None;
    }

    fn change_signatures(&mut self, counts: &Counts, sign: i64) {
        self.wings += i128::from(sign) * i128::from(counts.wings);
        for (keys, count) in &counts.wings_by_tokens {
            let total = if self.signatures.contains_key(keys) {
                self.signatures.get_mut(keys).expect("existing signature")
            } else {
                self.signatures.entry(keys.clone()).or_default()
            };
            *total += sign * count;
            if *total == 0 {
                self.signatures.remove(keys);
            }
        }
    }

    fn counts(&mut self, pool: &mut HashSet<Arc<Counts>>) -> io::Result<Arc<Counts>> {
        if let Some(counts) = &self.snapshot {
            return Ok(Arc::clone(counts));
        }
        let exact = |value: i128| {
            i64::try_from(value)
                .ok()
                .filter(|value| value.unsigned_abs() <= MAX_EXACT as u64)
                .ok_or_else(|| io::Error::other("Air force count exceeds exact integer range"))
        };
        let mut counts = Counts {
            planes: exact(self.planes)?,
            wings: exact(self.wings)?,
            ..Default::default()
        };
        // Same exactness guard as the full census; unknown parts do not invent a sum.
        for (key, part) in &self.parts {
            let amount = if part.unknown == 0 {
                Some(exact(part.sum)?)
            } else {
                None
            };
            counts.planes_by_token.insert(key.clone(), amount);
        }
        counts.wings_by_tokens = self.signatures.clone();
        let counts = shared_counts(counts, pool);
        self.snapshot = Some(Arc::clone(&counts));
        Ok(counts)
    }
}

struct WingCounts {
    counts: Arc<Counts>,
    owners: [Option<u32>; 2],
    notes: EquipmentNotes,
    invalid: bool,
}

#[derive(Default)]
struct Census {
    wings: HashMap<u64, WingCounts>,
    totals: [HashMap<u32, CountryCounts>; 2],
    unattributed: [i64; 2],
    notes: [i64; 3],
    ready: bool,
    equipment_complete: Option<bool>,
    dirty: HashSet<u32>,
    #[cfg(test)]
    reconciled: usize,
}

struct Inputs<'a> {
    decoded: &'a DecodedFrame,
    frame: &'a Frame,
    variants: &'a VariantCatalogue,
    countries: &'a Countries,
    rebuild: bool,
}

impl Census {
    fn replace(&mut self, before: &WingCounts, after: &WingCounts) {
        if before.owners != after.owners || before.invalid || after.invalid {
            self.change(before, -1);
            self.change(after, 1);
            return;
        }
        if before.counts != after.counts {
            for (owner, slot) in after.owners.iter().enumerate() {
                if let Some(slot) = slot {
                    self.totals[owner]
                        .get_mut(slot)
                        .expect("counted wing owner")
                        .replace(&before.counts, &after.counts);
                    self.dirty.insert(*slot);
                }
            }
        }
        for (index, (before, after)) in [
            (before.notes.missing, after.notes.missing),
            (before.notes.refused, after.notes.refused),
            (before.notes.remainders, after.notes.remainders),
        ]
        .into_iter()
        .enumerate()
        {
            self.notes[index] += i64::from(after) - i64::from(before);
        }
    }

    fn change(&mut self, wing: &WingCounts, sign: i64) {
        for (owner, slot) in wing.owners.iter().enumerate() {
            if let Some(slot) = slot.filter(|_| !wing.invalid) {
                self.totals[owner]
                    .entry(slot)
                    .or_default()
                    .change(&wing.counts, sign);
                if self.totals[owner]
                    .get(&slot)
                    .is_some_and(|total| total.wings == 0 && total.signatures.is_empty())
                {
                    self.totals[owner].remove(&slot);
                }
                self.dirty.insert(slot);
            } else {
                self.unattributed[owner] += sign;
            }
        }
        for (total, noted) in self.notes.iter_mut().zip([
            wing.notes.missing,
            wing.notes.refused,
            wing.notes.remainders,
        ]) {
            *total += sign * i64::from(noted);
        }
    }

    fn update(
        &mut self,
        inputs: Inputs<'_>,
        result: &mut ScannedAir,
        pool: &mut HashSet<Arc<Counts>>,
        cancelled: &AtomicBool,
    ) -> io::Result<[bool; 2]> {
        let Inputs {
            decoded,
            frame,
            variants,
            countries,
            rebuild,
        } = inputs;
        // HashSet iteration visits capacity: don't retain a dense hour's table
        // for sparse hours. Storage follows this hour's affected country slots.
        self.dirty = HashSet::new();
        let (Ok(wings), Ok(_)) = (
            decoded.complete::<tables::AirWings>(),
            decoded.complete::<tables::CountryLinks>(),
        ) else {
            self.ready = false;
            result.unreadable_owners[0] += 1;
            result.unreadable_owners[1] += 1;
            return Ok([false; 2]);
        };
        let entries = decoded.complete::<tables::WingEquipment>().ok();
        let evidence_changed = self.equipment_complete != Some(entries.is_some());
        self.equipment_complete = Some(entries.is_some());
        let mut ids = Vec::new();
        let sparse = self.ready
            && !rebuild
            && !evidence_changed
            && changed_ids(&frame.tables[TableId::AirWings.index()], 0, &mut ids)
            && (entries.is_none()
                || changed_ids(&frame.tables[TableId::WingEquipment.index()], 32, &mut ids));
        if !sparse {
            self.wings.clear();
            // A full read can remove a country's last wing without a delta key.
            self.dirty
                .extend(self.totals.iter().flat_map(|totals| totals.keys()).copied());
            self.totals = Default::default();
            self.unattributed = [0; 2];
            self.notes = [0; 3];
            ids.clear();
            ids.extend(wings.iter().map(|wing| wing.id));
        }
        // One token allocation per changed design, not per equipment row.
        let mut tokens = HashMap::new();
        let mut single_designs = HashMap::new();
        let dense = !sparse || ids.len() >= wings.len();
        let mut wing_index = 0;
        let mut equipment_index = 0;
        for (index, id) in ids.into_iter().enumerate() {
            if index.is_multiple_of(4096) {
                super::reply::cancel(cancelled)?;
            }
            let old = self.wings.remove(&id);
            let found = if dense {
                while wing_index < wings.len() && wings[wing_index].id < id {
                    wing_index += 1;
                }
                wings.get(wing_index).filter(|wing| wing.id == id)
            } else {
                wings
                    .binary_search_by_key(&id, |wing| wing.id)
                    .ok()
                    .map(|index| &wings[index])
            };
            let Some(wing) = found else {
                if let Some(old) = old {
                    self.change(&old, -1);
                }
                continue;
            };
            let equipment = entries.map(|rows| {
                let first = if !dense {
                    rows.partition_point(|entry| entry.wing < id)
                } else {
                    while equipment_index < rows.len() && rows[equipment_index].wing < id {
                        equipment_index += 1;
                    }
                    equipment_index
                };
                let last = if !dense {
                    first + rows[first..].partition_point(|entry| entry.wing == id)
                } else {
                    while equipment_index < rows.len() && rows[equipment_index].wing == id {
                        equipment_index += 1;
                    }
                    equipment_index
                };
                &rows[first..last]
            });
            for entry in equipment.into_iter().flatten() {
                tokens.entry(entry.variant).or_insert_with(|| {
                    let token = variants
                        .current(entry.variant)
                        .map_or("", |raw| raw.token.as_str());
                    format!("{}/{token}", entry.variant)
                });
            }
            let mut notes = EquipmentNotes::default();
            let invalid = wing.planes < 0;
            let counts = if invalid {
                notes.refused = true;
                shared_counts(Counts::default(), pool)
            } else if let Some([entry]) = equipment
                && wing.planes > 0
                && entry.amount_raw
                    == i64::from(wing.planes) * engine::equipment_entry::FIXED_POINT_SCALE
            {
                match single_designs.entry((wing.planes, entry.variant)) {
                    std::collections::hash_map::Entry::Occupied(entry) => Arc::clone(entry.get()),
                    std::collections::hash_map::Entry::Vacant(entry) => {
                        let counts =
                            shared_counts(wing_counts(wing, equipment, &tokens, &mut notes)?, pool);
                        entry.insert(Arc::clone(&counts));
                        counts
                    }
                }
            } else {
                shared_counts(wing_counts(wing, equipment, &tokens, &mut notes)?, pool)
            };
            // The effective country uses a signed-positive override.
            let effective = if wing.country_override > 0 {
                wing.country_override as u32
            } else {
                wing.owner
            };
            let counted = WingCounts {
                counts,
                owners: [countries.owner(wing.owner), countries.owner(effective)],
                notes,
                invalid,
            };
            let equal = old.as_ref().is_some_and(|old| {
                old.counts == counted.counts
                    && old.owners == counted.owners
                    && old.notes == counted.notes
                    && old.invalid == counted.invalid
            });
            if !equal {
                if let Some(old) = &old {
                    self.replace(old, &counted);
                } else {
                    self.change(&counted, 1);
                }
            }
            self.wings.insert(id, counted);
            #[cfg(test)]
            {
                self.reconciled += 1;
            }
        }
        self.ready = true;
        let complete = self.unattributed.map(|count| count == 0);
        for (count, complete) in result.unreadable_owners.iter_mut().zip(complete) {
            *count += usize::from(!complete);
        }
        result.equipment_missing += usize::from(self.notes[0] > 0);
        result.equipment_refused += usize::from(self.notes[1] > 0);
        result.equipment_remainders += usize::from(self.notes[2] > 0);
        Ok(complete)
    }

    fn counts(
        &mut self,
        owner: usize,
        slot: u32,
        pool: &mut HashSet<Arc<Counts>>,
    ) -> io::Result<Arc<Counts>> {
        match self.totals[owner].get_mut(&slot) {
            Some(totals) => totals.counts(pool),
            None => Ok(shared_counts(Counts::default(), pool)),
        }
    }
}

fn shared_counts(counts: Counts, pool: &mut HashSet<Arc<Counts>>) -> Arc<Counts> {
    if let Some(existing) = pool.get(&counts) {
        return Arc::clone(existing);
    }
    let counts = Arc::new(counts);
    pool.insert(Arc::clone(&counts));
    counts
}

#[derive(Default, Serialize, Deserialize)]
pub struct Scan {
    pub(super) histories: Histories,
    pub(super) result: ScannedAir,
    previous_complete: [bool; 2],
    variant_tokens: BTreeMap<u64, String>,
    definitions_complete: bool,
    last_hour: Option<i32>,
    #[serde(skip)]
    variants: Option<VariantCatalogue>,
    #[serde(skip)]
    counts_pool: HashSet<Arc<Counts>>,
    #[serde(skip)]
    pub(super) countries: Countries,
    #[serde(skip)]
    census: Census,
    #[serde(skip)]
    pub(super) applied: Option<i32>,
    #[serde(skip)]
    rates: [usize; 5],
    #[serde(skip)]
    joined: bool,
    /// The scan has read or forgotten variant names since it started: the
    /// names it holds are then those any scan holds after that hour, where
    /// before, a scan started afresh knows none a carried one knows.
    #[serde(skip)]
    pub(super) names_settled: bool,
    #[cfg(test)]
    #[serde(skip)]
    pub(super) work: dashboard_base::prepared::observer_tests::Work,
}
impl Scan {
    /// Rebuild transient joins from the committed decoder frame without repeating its history or counters.
    pub(super) fn rejoin(
        &mut self,
        frame: &Frame,
        decoded: &DecodedFrame,
        format: replay::format::FormatVersion,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        if self.last_hour != Some(decoded.hour) || self.applied.is_some() {
            return Ok(());
        }
        let histories = std::mem::take(&mut self.histories);
        let result = std::mem::take(&mut self.result);
        let previous_complete = self.previous_complete;
        self.last_hour = decoded.hour.checked_sub(1);
        self.step_changes(frame, decoded, format, cancelled, None)?;
        self.histories = histories;
        self.result = result;
        self.previous_complete = previous_complete;
        self.histories.countries(&self.countries);
        self.applied = Some(decoded.hour);
        Ok(())
    }
    pub(super) fn step_changes(
        &mut self,
        frame: &Frame,
        decoded: &DecodedFrame,
        format: replay::format::FormatVersion,
        cancelled: &AtomicBool,
        changes: Option<&dashboard_base::prepared::frame_changes::FrameChanges>,
    ) -> io::Result<()> {
        self.histories.observed.clear();
        self.histories.removed.clear();
        let joined = self.countries.update(decoded);
        self.joined = !matches!(joined, Ok(false));
        let unchanged = self
            .last_hour
            .is_some_and(|at| at.checked_add(1) == Some(decoded.hour))
            && matches!(joined, Ok(false))
            && changes.is_some_and(|changes| {
                ![
                    TableId::AirWings,
                    TableId::WingEquipment,
                    TableId::VariantDefinitions,
                    TableId::LexicalTokens,
                ]
                .iter()
                .any(|id| changes.rows_changed[id.index()])
                    && !changes.slot_read[TableId::VariantDefinitions.index()]
            });
        if unchanged {
            self.last_hour = Some(decoded.hour);
            if self.histories.through.is_some() {
                self.histories.through = Some(decoded.hour);
            }
            if let Some(variants) = &mut self.variants {
                variants.advance(decoded.hour);
            }
            self.result.unreadable_owners[0] += self.rates[0];
            self.result.unreadable_owners[1] += self.rates[1];
            self.result.equipment_missing += self.rates[2];
            self.result.equipment_refused += self.rates[3];
            self.result.equipment_remainders += self.rates[4];
            for tag in &self.histories.undisplayable {
                *self
                    .result
                    .undisplayable_tags
                    .entry(tag.clone())
                    .or_default() += 1;
            }
            return Ok(());
        }
        let before = [
            self.result.unreadable_owners[0],
            self.result.unreadable_owners[1],
            self.result.equipment_missing,
            self.result.equipment_refused,
            self.result.equipment_remainders,
        ];
        #[cfg(test)]
        {
            if let Some(changes) = changes {
                self.work.row_visits += <super::prepared::Air as crate::products::parts_race_prepared::Family>::SAMPLE_TABLES.iter()
                    .filter(|id| changes.rows_changed[id.index()])
                    .map(|id| match &*decoded.tables[id.index()] { replay::frame::TableView::Complete(rows)
                        | replay::frame::TableView::Truncated { rows, .. } => rows.len(), _ => 0 }).sum::<usize>();
            }
        }
        self.step(frame, decoded, format, cancelled)?;
        self.applied = Some(decoded.hour);
        self.rates = [
            self.result.unreadable_owners[0],
            self.result.unreadable_owners[1],
            self.result.equipment_missing,
            self.result.equipment_refused,
            self.result.equipment_remainders,
        ];
        for (rate, before) in self.rates.iter_mut().zip(before) {
            *rate -= before;
        }
        Ok(())
    }
    pub(super) fn step(
        &mut self,
        frame: &Frame,
        decoded: &DecodedFrame,
        format: replay::format::FormatVersion,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        let Self {
            histories,
            result,
            previous_complete,
            variant_tokens,
            definitions_complete,
            last_hour,
            variants,
            counts_pool,
            countries,
            census,
            joined,
            names_settled,
            ..
        } = self;
        // The continuation keeps each known variant's token: those variants stay
        // known, so a later forget ends them as it does in continuous observation.
        let variants = variants.get_or_insert_with(|| {
            VariantCatalogue::seeded(
                variant_tokens
                    .iter()
                    .map(|(&id, token)| {
                        (
                            id,
                            vec![dashboard_base::products::equipment::VariantVersion {
                                first_hour: last_hour.unwrap_or(decoded.hour),
                                raw: Some(dashboard_base::products::equipment::RawVariant {
                                    id,
                                    token: token.clone(),
                                    ..Default::default()
                                }),
                            }],
                        )
                    })
                    .collect(),
            )
        });
        let hour = decoded.hour;
        let continuous = last_hour.is_some_and(|last| last.checked_add(1) == Some(hour));
        let gap = last_hour.is_some() && !continuous;
        *last_hour = Some(hour);
        if !continuous {
            variants.forget(hour);
            variant_tokens.clear();
        }
        let complete = decoded.complete::<tables::VariantDefinitions>().is_ok();
        let read = slot_read(format, &decoded.metrics, TableId::VariantDefinitions, hour);
        // The catalogue reads every name at a due read, and forgets them all
        // after a gap, where the table is not complete or the read did not
        // land: after any of these, no name it holds comes from before.
        *names_settled |= gap || read || !complete;
        let check_names = !continuous || complete != *definitions_complete || read;
        *definitions_complete = complete;
        variants.observe(format, decoded);
        let names_changed = if check_names {
            let mut changed = false;
            if complete
                && decoded.complete::<tables::Countries>().is_ok()
                && decoded.complete::<tables::CountryLinks>().is_ok()
                && decoded.complete::<tables::LexicalTokens>().is_ok()
            {
                for definition in decoded
                    .complete::<tables::VariantDefinitions>()
                    .unwrap_or_default()
                {
                    if let Some(raw) = variants.current(definition.id)
                        && variant_tokens.get(&definition.id) != Some(&raw.token)
                    {
                        variant_tokens.insert(definition.id, raw.token.clone());
                        changed = true;
                    }
                }
            } else {
                changed = !variant_tokens.is_empty();
                variant_tokens.clear();
            }
            changed
        } else {
            false
        };
        let rebuild = !continuous || names_changed;
        let Ok(countries_changed) = countries.update(decoded) else {
            histories.flush();
            histories.removed.append(&mut histories.active_tags);
            histories.active.clear();
            census.ready = false;
            return Ok(());
        };
        let countries_changed = countries_changed || *joined;
        let complete = census.update(
            Inputs {
                decoded,
                frame,
                variants,
                countries,
                rebuild: rebuild || countries_changed,
            },
            result,
            counts_pool,
            cancelled,
        )?;
        let sparse_history = continuous
            && !countries_changed
            && histories.through.is_some()
            && *previous_complete == [true; 2]
            && complete == [true; 2];
        if !sparse_history {
            histories.flush();
            // Balance-only country reads retain this native slot/tag join.
            histories.countries(countries);
        }
        for tag in &histories.undisplayable {
            *result.undisplayable_tags.entry(tag.clone()).or_default() += 1;
        }
        if sparse_history {
            let affected: Vec<_> = census
                .dirty
                .iter()
                .filter_map(|slot| histories.slots.get(slot))
                .flatten()
                .copied()
                .collect();
            for active in affected {
                histories.observe(active, hour, complete, census, counts_pool)?;
            }
        } else {
            for active in 0..histories.active.len() {
                histories.observe(active, hour, complete, census, counts_pool)?;
            }
        }
        *previous_complete = complete;
        histories.through = Some(hour);
        Ok(())
    }
    pub(super) fn published(&mut self) {
        // Open tracks and census snapshots retain their Arcs; historical intern entries can go.
        self.counts_pool.clear();
        if let Some(variants) = &mut self.variants {
            for versions in variants.versions.values_mut() {
                if versions.len() > 1 {
                    versions.drain(..versions.len() - 1);
                }
            }
        }
    }
    pub(super) fn finish(mut self) -> ScannedAir {
        self.histories.flush();
        #[cfg(test)]
        {
            self.result.reconciled_wings = self.census.reconciled;
            self.result.projected_countries = self.histories.projected;
        }
        for (tag, index) in self.histories.tags {
            let track = std::mem::take(&mut self.histories.tracks[index]);
            self.result.segments.insert(tag.clone(), track.segments);
            self.result.series.insert(tag, track.runs);
        }
        self.result
    }
}

pub(super) fn scan(replay: &Replay, branch: u64, cancelled: &AtomicBool) -> io::Result<ScannedAir> {
    scan_with_decode(replay, branch, cancelled, |_| {})
}

// The ignored benches time these boundaries; production's empty hook compiles out.
pub(super) fn scan_with_decode(
    replay: &Replay,
    branch: u64,
    cancelled: &AtomicBool,
    mut decoding: impl FnMut(bool),
) -> io::Result<ScannedAir> {
    if !replay.branches.iter().any(|entry| entry.id == branch) {
        return Err(io::Error::other("Unknown branch"));
    }
    let mut selected = VariantCatalogue::TABLES.to_vec();
    selected.extend([TableId::Campaign, TableId::AirWings, TableId::WingEquipment]);
    let mut observer = Scan::default();
    dashboard_core::blocks::scan_blocks(
        replay,
        replay.index.iter().filter(|block| block.branch == branch),
        &selected,
        cancelled,
        |blocks| {
            // Waiting for the next block is its decoding.
            loop {
                decoding(true);
                let next = blocks.next();
                decoding(false);
                let Some((_, frames)) = next else {
                    break;
                };
                let mut decoder = FrameDecoder::for_tables(&selected);
                let Ok(frames) = frames else {
                    continue;
                };
                for frame in frames {
                    if cancelled.load(Ordering::Acquire) {
                        return Err(io::Error::new(
                            io::ErrorKind::Interrupted,
                            "Air force request cancelled",
                        ));
                    }
                    decoding(true);
                    let decoded = decoder.advance(&frame);
                    decoding(false);
                    let Ok(decoded) = decoded else {
                        break;
                    };
                    if validate_campaign(decoded, replay).is_err() {
                        break;
                    }
                    observer.step(&frame, decoded, replay.format, cancelled)?;
                }
            }
            Ok(())
        },
    )?;
    Ok(observer.finish())
}

/// One active track's state after an hour that decides its next: its last
/// run's counts and hour, its last known identity and hour, its identity
/// track and its last identity segment's hour.
#[derive(PartialEq)]
struct TrackTail {
    run: Option<(Option<Arc<Counts>>, Option<Arc<Counts>>, i32)>,
    known: Option<(Identity, i32)>,
    identity: String,
    segment: Option<i32>,
}

impl TrackTail {
    /// An active track's tail as extending it through `through` leaves it.
    /// A track the hour did not observe keeps its tail where it last was:
    /// the next observation of it, or the next flush, first extends it
    /// through `through`, so its state then is the extended one. A track the
    /// hour observed is already there.
    fn extended(track: &Track, identity: Identity, through: Option<i32>) -> Self {
        let mut last_identity = track.last_identity.clone();
        if let Some(hour) = through {
            last_identity.advance(hour);
        }
        Self {
            run: track.runs.last().map(|run| {
                (
                    run.roster.clone(),
                    run.effective.clone(),
                    through.unwrap_or(run.last_hour),
                )
            }),
            known: match through {
                Some(hour) => track.last_known.map(|_| (identity, hour)),
                None => track.last_known,
            },
            identity: serde_json::to_string(&last_identity).unwrap_or_default(),
            segment: track
                .segments
                .last()
                .map(|segment| through.unwrap_or(segment.last_hour)),
        }
    }
}

/// What decides a scan's next hours: its last hour and read evidence, the
/// variant names it knows and the variant catalogue's current names, the
/// country join, the census's readiness, the active tracks' tails (as the
/// next hour extends them) and the counter rates an unchanged hour adds. The
/// census itself holds the hour's wings either way, counted with the same
/// variant names and country join, so it is equal whenever these are.
/// Whether the hour's join changed is left out: every hour sets it before
/// reading it, and a census starting afresh joins anew where the carried
/// one finds its join held.
///
/// Of the catalogue, the census and the names check read only each known
/// variant's current token; its last hour is the scan's. When a version
/// began is left out: a read keeps a variant's version unless what it read
/// changed, so a carried catalogue dates a name from the read that first
/// gave it, a worker's from its own first read, and a restored one from its
/// restore, with the same name.
#[derive(PartialEq)]
struct Decision {
    last_hour: Option<i32>,
    previous_complete: [bool; 2],
    definitions_complete: bool,
    applied: Option<i32>,
    rates: [usize; 5],
    variant_tokens: BTreeMap<u64, String>,
    names: Option<BTreeMap<u64, String>>,
    countries: Vec<(Vec<u8>, Option<Identity>)>,
    census: (bool, Option<bool>),
    through: Option<i32>,
    active: Vec<(String, Identity)>,
    undisplayable: Vec<Vec<u8>>,
    tracks: Vec<TrackTail>,
}

/// The counters a scan adds to, which a fold carries by the block's
/// increase after its tail's hour.
#[derive(Clone)]
struct Counters {
    unreadable_owners: [usize; 2],
    equipment: [usize; 3],
    undisplayable_tags: BTreeMap<Vec<u8>, usize>,
}

/// A worker's scan right after the hour it takes its tail at.
pub struct Tail {
    hour: i32,
    decision: Decision,
    counters: Counters,
    /// The tags active then.
    active: HashSet<String>,
    /// The tracks projected by then, which tests count.
    #[cfg(test)]
    projected: usize,
}

impl Scan {
    fn decision(&self) -> Decision {
        let histories = &self.histories;
        Decision {
            last_hour: self.last_hour,
            previous_complete: self.previous_complete,
            definitions_complete: self.definitions_complete,
            applied: self.applied,
            rates: self.rates,
            variant_tokens: self.variant_tokens.clone(),
            names: self.variants.as_ref().map(current_names),
            countries: self.countries.tags.clone(),
            census: (self.census.ready, self.census.equipment_complete),
            through: histories.through,
            active: histories
                .active_tags
                .iter()
                .cloned()
                .zip(histories.active.iter().map(|&(_, identity)| identity))
                .collect(),
            undisplayable: histories.undisplayable.clone(),
            tracks: histories
                .active
                .iter()
                .map(|&(track, identity)| {
                    TrackTail::extended(&histories.tracks[track], identity, histories.through)
                })
                .collect(),
        }
    }

    fn counters(&self) -> Counters {
        Counters {
            unreadable_owners: self.result.unreadable_owners,
            equipment: [
                self.result.equipment_missing,
                self.result.equipment_refused,
                self.result.equipment_remainders,
            ],
            undisplayable_tags: self.result.undisplayable_tags.clone(),
        }
    }

    /// The scan right after the hour it takes its tail at, `hour`.
    pub(super) fn tail(&self, hour: i32) -> Tail {
        Tail {
            hour,
            decision: self.decision(),
            counters: self.counters(),
            active: self.histories.active_tags.iter().cloned().collect(),
            #[cfg(test)]
            projected: self.histories.projected,
        }
    }

    /// Whether this carried scan, having observed the block through the
    /// tail's hour, decides every later hour as the extract's did from
    /// `tail`: then each later step is the extract's, as every step reads
    /// only the hour's frame and what the decision holds. A track idle at
    /// the tail's hour continues only through adjacent hours, so its history
    /// decides nothing later. The extract's tracks active then each had a run
    /// and segment by that hour, which the carried ones continue.
    pub(super) fn continues(&self, tail: &Tail, block: &Scan) -> bool {
        self.decision() == tail.decision
            && tail.active.iter().all(|tag| {
                block.histories.tags.get(tag).is_some_and(|&track| {
                    let track = &block.histories.tracks[track];
                    track
                        .runs
                        .first()
                        .is_some_and(|run| run.first_hour <= tail.hour)
                        && track
                            .segments
                            .first()
                            .is_some_and(|segment| segment.first_hour <= tail.hour)
                })
            })
    }

    /// Carry this scan to the block's end: of each track, what the extract
    /// stepped through the tail's hour the carried scan observed itself; the
    /// run and segment a track active then held at that hour extend the
    /// carried ones they continued, later ones follow, new tags take the
    /// next tracks in the order the extract made them, the counters add
    /// their increase after the tail's hour, and everything else is the
    /// extract's.
    pub(super) fn carry(&mut self, tail: Tail, block: Scan) {
        let Scan {
            histories,
            result,
            previous_complete,
            variant_tokens,
            definitions_complete,
            last_hour,
            variants,
            counts_pool,
            countries,
            census,
            applied,
            rates,
            joined,
            ..
        } = block;
        let before = tail.counters;
        for (count, (now, then)) in self.result.unreadable_owners.iter_mut().zip(
            result
                .unreadable_owners
                .into_iter()
                .zip(before.unreadable_owners),
        ) {
            *count += now - then;
        }
        for (count, (now, then)) in [
            &mut self.result.equipment_missing,
            &mut self.result.equipment_refused,
            &mut self.result.equipment_remainders,
        ]
        .into_iter()
        .zip(
            [
                result.equipment_missing,
                result.equipment_refused,
                result.equipment_remainders,
            ]
            .into_iter()
            .zip(before.equipment),
        ) {
            *count += now - then;
        }
        for (tag, now) in result.undisplayable_tags {
            let then = before.undisplayable_tags.get(&tag).copied().unwrap_or(0);
            *self.result.undisplayable_tags.entry(tag).or_default() += now - then;
        }
        let Histories {
            tags,
            mut tracks,
            active,
            active_tags,
            observed,
            removed,
            slots,
            undisplayable,
            through,
            #[cfg(test)]
            projected,
        } = histories;
        let mut order: Vec<(usize, String)> = tags.into_iter().map(|(tag, at)| (at, tag)).collect();
        order.sort_unstable();
        let mut carried = vec![0; tracks.len()];
        for (at, tag) in order {
            let track = std::mem::take(&mut tracks[at]);
            let index = match self.histories.tags.get(&tag) {
                Some(&index) => index,
                None => {
                    let index = self.histories.tracks.len();
                    self.histories.tracks.push(Track::default());
                    self.histories.tags.insert(tag.clone(), index);
                    index
                }
            };
            carried[at] = index;
            carry_track(
                &mut self.histories.tracks[index],
                track,
                tail.active.contains(&tag),
                tail.hour,
            );
        }
        let held = &mut self.histories;
        held.active = active
            .into_iter()
            .map(|(track, identity)| (carried[track], identity))
            .collect();
        held.active_tags = active_tags;
        held.observed = observed;
        held.removed = removed;
        held.slots = slots;
        held.undisplayable = undisplayable;
        held.through = through;
        #[cfg(test)]
        {
            held.projected += projected - tail.projected;
        }
        self.previous_complete = previous_complete;
        self.variant_tokens = variant_tokens;
        self.definitions_complete = definitions_complete;
        self.last_hour = last_hour;
        self.variants = variants;
        self.counts_pool = counts_pool;
        self.countries = countries;
        self.census = census;
        self.applied = applied;
        self.rates = rates;
        self.joined = joined;
    }
}

/// Each variant the catalogue names now, with its current token: what its
/// next hours read of it.
fn current_names(variants: &VariantCatalogue) -> BTreeMap<u64, String> {
    variants
        .versions
        .keys()
        .filter_map(|&id| variants.current(id).map(|raw| (id, raw.token.clone())))
        .collect()
}

/// Carry one track: the runs and segments the extract began by `hour` cover
/// hours the carried track observed itself, so they go; with `opened`, the
/// track was active at `hour`, where the carried track's run and segment
/// continue, so the last of each extend those. Every later run and segment
/// follows, and a track the extract touched after `hour` takes its identity
/// and last known hour.
fn carry_track(carried: &mut Track, extracted: Track, opened: bool, hour: i32) {
    let Track {
        mut runs,
        mut segments,
        last_identity,
        last_known,
    } = extracted;
    let begun = runs.partition_point(|run| run.first_hour <= hour);
    if opened
        && begun > 0
        && let Some(open) = carried.runs.last_mut()
    {
        open.last_hour = runs[begun - 1].last_hour;
    }
    runs.drain(..begun);
    let begun = segments.partition_point(|segment| segment.first_hour <= hour);
    if opened
        && begun > 0
        && let Some(open) = carried.segments.last_mut()
    {
        open.last_hour = segments[begun - 1].last_hour;
    }
    segments.drain(..begun);
    let touched = opened || !runs.is_empty() || !segments.is_empty();
    carried.runs.extend(runs);
    carried.segments.extend(segments);
    if touched {
        carried.last_identity = last_identity;
        carried.last_known = last_known;
    }
}

/// What a block extract keeps of an air census ([`Held`]): its tracks, runs
/// and counts (each shared count once), its census, variant names and country
/// join, and its tail's decision.
mod held {
    use super::*;
    use dashboard_base::{
        prepared::prepass::{Held, HeldBytes},
        products::equipment::VariantVersion,
    };

    impl Held for Counts {
        fn held(&self, bytes: &mut HeldBytes) {
            bytes
                .heap(&self.planes_by_token)
                .heap(&self.wings_by_tokens);
        }
    }

    impl Held for RawRun {
        fn held(&self, bytes: &mut HeldBytes) {
            bytes.heap(&self.roster).heap(&self.effective);
        }
    }

    impl Held for ScannedAir {
        fn held(&self, bytes: &mut HeldBytes) {
            bytes
                .heap(&self.series)
                .heap(&self.segments)
                .heap(&self.undisplayable_tags);
        }
    }

    impl Held for Track {
        fn held(&self, bytes: &mut HeldBytes) {
            bytes.heap(&self.runs).heap(&self.segments);
        }
    }

    impl Held for Histories {
        fn held(&self, bytes: &mut HeldBytes) {
            bytes
                .heap(&self.tags)
                .heap(&self.tracks)
                .heap(&self.active)
                .heap(&self.active_tags)
                .heap(&self.observed)
                .heap(&self.removed)
                .heap(&self.slots)
                .heap(&self.undisplayable);
        }
    }

    impl Held for PartSum {
        fn held(&self, _: &mut HeldBytes) {}
    }

    impl Held for CountryCounts {
        fn held(&self, bytes: &mut HeldBytes) {
            bytes
                .heap(&self.parts)
                .heap(&self.signatures)
                .heap(&self.snapshot);
        }
    }

    impl Held for WingCounts {
        fn held(&self, bytes: &mut HeldBytes) {
            bytes.heap(&self.counts);
        }
    }

    impl Held for Census {
        fn held(&self, bytes: &mut HeldBytes) {
            bytes
                .heap(&self.wings)
                .heap(&self.totals)
                .heap(&self.dirty);
        }
    }

    /// A variant catalogue's versions and their names.
    fn variants(variants: &VariantCatalogue, bytes: &mut HeldBytes) {
        bytes.tree::<u64, Vec<VariantVersion>>(variants.versions.len());
        for versions in variants.versions.values() {
            bytes.add(versions.capacity() * size_of::<VariantVersion>());
            for raw in versions.iter().filter_map(|version| version.raw.as_ref()) {
                bytes
                    .heap(&raw.token)
                    .heap(&raw.display_name)
                    .heap(&raw.suffix)
                    .heap(&raw.name_tag)
                    .heap(&raw.type_tag)
                    .heap(&raw.design)
                    .heap(&raw.refusals);
            }
        }
    }

    impl Held for Scan {
        fn held(&self, bytes: &mut HeldBytes) {
            bytes
                .heap(&self.histories)
                .heap(&self.result)
                .heap(&self.variant_tokens)
                .heap(&self.counts_pool)
                .heap(&self.countries.tags)
                .heap(&self.census);
            if let Some(catalogue) = &self.variants {
                variants(catalogue, bytes);
            }
        }
    }

    impl Held for TrackTail {
        fn held(&self, bytes: &mut HeldBytes) {
            bytes.heap(&self.run).heap(&self.identity);
        }
    }

    impl Held for Tail {
        fn held(&self, bytes: &mut HeldBytes) {
            let decision = &self.decision;
            bytes
                .heap(&decision.variant_tokens)
                .heap(&decision.names)
                .heap(&decision.countries)
                .heap(&decision.active)
                .heap(&decision.undisplayable)
                .heap(&decision.tracks)
                .heap(&self.counters.undisplayable_tags)
                .heap(&self.active);
        }
    }
}
