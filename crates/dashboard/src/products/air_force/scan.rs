use super::*;
use crate::{
    metadata::{IdentityTrack, slot_read, validate_campaign},
    products::{
        country_history::{Identity, observe_identity},
        equipment::VariantCatalogue,
        force_census::{Countries, changed_ids},
    },
};
use replay::{
    container::Replay,
    frame::{DecodedFrame, Frame, FrameDecoder, ViewIdentity},
    rows::{AirWingRow, TableId, WingEquipmentRow, tables},
};
use std::{
    collections::{HashMap, HashSet},
    sync::atomic::Ordering,
};

#[derive(Default)]
struct Track {
    runs: Vec<RawRun>,
    segments: Vec<HourSpan>,
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
        self.last_identity.step(Some(identity), hour);
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

#[derive(Default)]
struct Histories {
    tags: HashMap<String, usize>,
    tracks: Vec<Track>,
    active: Vec<(usize, Identity)>,
    slots: HashMap<u32, Vec<usize>>,
    undisplayable: Vec<Vec<u8>>,
    // Untouched tracks extend lazily to the last authenticated hour. Flush before
    // topology/coverage changes, so absent countries and gaps never get carried.
    through: Option<i32>,
    #[cfg(test)]
    projected: usize,
}

impl Histories {
    fn flush(&mut self) {
        if let Some(hour) = self.through.take() {
            for &(track, identity) in &self.active {
                self.tracks[track].extend(identity, hour);
            }
        }
    }

    fn countries(&mut self, countries: &Countries) {
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
            }
        }
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
        self.totals[owner].entry(slot).or_default().counts(pool)
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
    let mut variants = VariantCatalogue::default();
    let mut histories = Histories::default();
    let mut previous_complete = [false; 2];
    let mut result = ScannedAir::default();
    // Equal immutable censuses share storage, including both ownership modes.
    let mut counts_pool = HashSet::new();
    let mut last_projection: Option<(i32, Vec<ViewIdentity>)> = None;
    let mut countries = Countries::default();
    let mut census = Census::default();
    let mut variant_tokens: BTreeMap<u64, String> = BTreeMap::new();
    let mut definitions_complete = false;
    crate::blocks::scan_blocks(
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
                    let hour = decoded.hour;
                    let continuous = last_projection
                        .as_ref()
                        .is_some_and(|(last, _)| last.checked_add(1) == Some(hour));
                    let complete = decoded.complete::<tables::VariantDefinitions>().is_ok();
                    let check_names = !continuous
                        || complete != definitions_complete
                        || slot_read(&decoded.metrics, TableId::VariantDefinitions, hour);
                    definitions_complete = complete;
                    variants.observe(decoded);
                    let names_changed = if check_names {
                        let current: BTreeMap<_, _> = variants
                            .versions
                            .iter()
                            .filter_map(|(&id, _)| {
                                variants.current(id).map(|raw| (id, raw.token.clone()))
                            })
                            .collect();
                        let changed = variant_tokens != current;
                        variant_tokens = current;
                        changed
                    } else {
                        false
                    };
                    let projection: Vec<_> = [
                        TableId::Countries,
                        TableId::CountryLinks,
                        TableId::AirWings,
                        TableId::WingEquipment,
                        TableId::VariantDefinitions,
                        TableId::LexicalTokens,
                    ]
                    .iter()
                    .map(|table| decoded.tables[table.index()].identity())
                    .collect();
                    let unchanged =
                        last_projection
                            .as_ref()
                            .is_some_and(|(last_hour, previous)| {
                                last_hour.checked_add(1) == Some(hour)
                                    && !slot_read(
                                        &decoded.metrics,
                                        TableId::VariantDefinitions,
                                        hour,
                                    )
                                    && previous
                                        .iter()
                                        .zip(&projection)
                                        .all(|(old, current)| old == current)
                            });
                    let rebuild = !continuous || names_changed;
                    last_projection = Some((hour, projection));
                    if unchanged && histories.through.is_some() {
                        // Identical authenticated tables confirm another recorded hour.
                        histories.through = Some(hour);
                        continue;
                    }
                    let Ok(countries_changed) = countries.update(decoded) else {
                        histories.flush();
                        census.ready = false;
                        continue;
                    };
                    let complete = census.update(
                        Inputs {
                            decoded,
                            frame: &frame,
                            variants: &variants,
                            countries: &countries,
                            rebuild: rebuild || countries_changed,
                        },
                        &mut result,
                        &mut counts_pool,
                        cancelled,
                    )?;
                    let sparse_history = continuous
                        && !countries_changed
                        && histories.through.is_some()
                        && previous_complete == [true; 2]
                        && complete == [true; 2];
                    if !sparse_history {
                        histories.flush();
                        // Balance-only country reads retain this native slot/tag join.
                        histories.countries(&countries);
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
                            histories.observe(
                                active,
                                hour,
                                complete,
                                &mut census,
                                &mut counts_pool,
                            )?;
                        }
                    } else {
                        for active in 0..histories.active.len() {
                            histories.observe(
                                active,
                                hour,
                                complete,
                                &mut census,
                                &mut counts_pool,
                            )?;
                        }
                    }
                    previous_complete = complete;
                    histories.through = Some(hour);
                }
            }
            Ok(())
        },
    )?;
    histories.flush();
    #[cfg(test)]
    {
        result.reconciled_wings = census.reconciled;
        result.projected_countries = histories.projected;
    }
    for (tag, index) in histories.tags {
        let track = std::mem::take(&mut histories.tracks[index]);
        result.segments.insert(tag.clone(), track.segments);
        result.series.insert(tag, track.runs);
    }
    Ok(result)
}
