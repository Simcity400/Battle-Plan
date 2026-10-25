//! Current reservations, separate from payments. Allocation stores the full
//! amount in the country; release removes that amount and refunds it.
use super::*;
use dashboard_base::prepared::frame_changes::FrameChanges;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct LedgerCommandHolder {
    pub id: String,
    /// Recorded object domain, cause or writer site; never a guessed holder.
    pub kind: String,
    pub name: Option<String>,
    pub amount_raw: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct LedgerCommandHold {
    pub read_hour: i32,
    pub through_hour: i32,
    pub total_raw: Option<String>,
    pub holders: Vec<LedgerCommandHolder>,
    pub reason: Option<String>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub(super) struct Holds {
    countries: BTreeMap<String, CountryHolds>,
    /// The rows naming each country's allocators; none until the first
    /// snapshot after a resume.
    #[serde(skip)]
    names: Option<NameRows>,
}

/// The unit leader and character rows each country's allocator names were
/// read from at its last snapshot, so a changed row observes again only the
/// countries it can rename.
#[derive(Clone, Default)]
struct NameRows {
    leaders: BTreeMap<u128, BTreeSet<String>>,
    characters: BTreeMap<u32, BTreeSet<String>>,
    of: BTreeMap<String, (Vec<u128>, Vec<u32>)>,
    /// Countries holding allocators not yet snapshotted since the resume.
    unindexed: BTreeSet<String>,
}
impl NameRows {
    fn index(&mut self, tag: &str, rows: (Vec<u128>, Vec<u32>)) {
        if let Some((leaders, characters)) = self.of.remove(tag) {
            for key in leaders {
                unindex(&mut self.leaders, key, tag);
            }
            for id in characters {
                unindex(&mut self.characters, id, tag);
            }
        }
        self.unindexed.remove(tag);
        if rows.0.is_empty() && rows.1.is_empty() {
            return;
        }
        for &key in &rows.0 {
            self.leaders.entry(key).or_default().insert(tag.to_owned());
        }
        for &id in &rows.1 {
            self.characters
                .entry(id)
                .or_default()
                .insert(tag.to_owned());
        }
        self.of.insert(tag.to_owned(), rows);
    }
}
fn unindex<K: Ord>(index: &mut BTreeMap<K, BTreeSet<String>>, key: K, tag: &str) {
    if let Some(tags) = index.get_mut(&key) {
        tags.remove(tag);
        if tags.is_empty() {
            index.remove(&key);
        }
    }
}
impl Holds {
    pub(super) fn known_tags(&self) -> impl Iterator<Item = &String> {
        self.countries.keys()
    }
    pub(super) fn advance(
        &mut self,
        through: i32,
        selected: &BTreeSet<String>,
        active: &BTreeSet<String>,
    ) {
        // A removal or refusal supplies one snapshot. Only a current recorded
        // holder has another snapshot on each unchanged accepted read.
        for tag in selected.intersection(active) {
            let Some(country) = self.countries.get_mut(tag) else {
                continue;
            };
            if country.last_hour.is_some_and(|last| last < through) {
                country.last_hour = Some(through);
                if let Some(last) = country.history.last_mut() {
                    last.through_hour = through;
                }
            }
        }
    }
    /// The slots of countries holding an allocator whose name this hour's
    /// characters, unit leaders or lexical dictionary may change: those named
    /// from a changed row, or from a changed table when its rows are unknown.
    /// Without `rows_known`, every country named from a changed table is.
    pub(super) fn naming_slots(&self, changes: &FrameChanges, rows_known: bool) -> Vec<u32> {
        let changed = [
            TableId::Characters,
            TableId::UnitLeaders,
            TableId::LexicalTokens,
        ]
        .map(|table| changes.rows_changed[table.index()]);
        if !changed.contains(&true) {
            return Vec::new();
        }
        let named_from = |tag: &String| {
            self.countries.get(tag).is_some_and(|country| {
                country
                    .active
                    .values()
                    .any(|holder| holder.named_from(changed))
            })
        };
        let rows = (
            self.names.as_ref().filter(|_| rows_known),
            changes.rows(TableId::Characters),
            changes.rows(TableId::UnitLeaders),
        );
        let tags: BTreeSet<&String> = match rows {
            (Some(names), Some(characters), Some(leaders)) if !changed[2] => {
                let mut tags = BTreeSet::new();
                for key in characters
                    .0
                    .iter()
                    .map(|row| row.key)
                    .chain(characters.1.iter().copied())
                {
                    if let Ok(id) = u32::try_from(key) {
                        tags.extend(names.characters.get(&id).into_iter().flatten());
                    }
                }
                for key in leaders
                    .0
                    .iter()
                    .map(|row| row.key)
                    .chain(leaders.1.iter().copied())
                {
                    tags.extend(names.leaders.get(&key).into_iter().flatten());
                }
                tags.extend(names.unindexed.iter().filter(|tag| named_from(tag)));
                tags
            }
            _ => self
                .countries
                .keys()
                .filter(|tag| named_from(tag))
                .collect(),
        };
        #[cfg(test)]
        NAMED.with(|named| named.set(named.get() + tags.len()));
        tags.into_iter()
            .filter_map(|tag| self.countries.get(tag)?.slot)
            .collect()
    }
    /// How many countries hold an allocator.
    #[cfg(test)]
    pub(super) fn holding(&self) -> usize {
        self.countries
            .values()
            .filter(|country| !country.active.is_empty())
            .count()
    }
    pub(super) fn retain_open(&mut self) {
        for country in self.countries.values_mut() {
            let closed = country.history.len().saturating_sub(1);
            country.history.drain(..closed);
        }
    }
}

#[derive(Clone, Default, Serialize, Deserialize)]
struct CountryHolds {
    slot: Option<u32>,
    identity: Option<(u32, u64)>,
    sequence: Option<u64>,
    last_hour: Option<i32>,
    active: BTreeMap<String, Holder>,
    history: Vec<LedgerCommandHold>,
    /// An empty allocator set's snapshot depends only on its recorded total
    /// and, without that total, its refusal. Rebuilt after a resume.
    #[serde(skip)]
    empty_snapshot: Option<(Option<i64>, Option<String>)>,
}

#[derive(Clone, Serialize, Deserialize)]
struct Holder {
    kind: String,
    item: LedgerItem,
    amount: i128,
    #[serde(with = "facts::native_cause")]
    cause: PowerCause,
}

/// Charging callers label allocations as AbilityActivation, Raid, AirAction,
/// Advisor or LeaderAssignment. The writer identifies a hold.
/// Ground crew releases/resizes/reallocates its reservation and charges
/// the reservation adjustment directly.
pub(super) fn is_hold(row: &LedgerMutation) -> bool {
    row.currency == "Command"
        && (matches!(
            row.site.as_str(),
            "AllocateCommandPower" | "ReleaseCommandPower" | "GroundCrew"
        ) || row.cause == "AllocationRefund")
}

fn holder_key(item: &LedgerItem) -> Option<String> {
    if item.object_kind != "None" {
        return item.object_reference.as_ref().map(|reference| {
            format!(
                "{}:{reference}:{:?}:{:?}:{:?}:{:?}:{:?}:{:?}",
                item.object_kind,
                item.key,
                item.token,
                item.slot_key,
                item.target_country,
                item.target_state,
                item.target_region
            )
        });
    }
    if let Some(character) = item.character {
        return Some(format!(
            "character:{character}:{:?}:{:?}:{:?}",
            item.key, item.token, item.slot_key
        ));
    }
    if !item.key.is_empty() || item.token.is_some() {
        return Some(format!(
            "item:{:?}:{:?}:{:?}:{:?}:{:?}",
            item.key, item.token, item.target_country, item.target_state, item.target_region
        ));
    }
    item.target_region.map(|region| format!("region:{region}"))
}

fn holder_kind(row: &LedgerMutation) -> String {
    if row.item.object_kind != "None" {
        row.item.object_kind.clone()
    } else {
        row.cause.clone()
    }
}

impl CountryHolds {
    fn observe_snapshot(
        &mut self,
        frame: &DecodedFrame,
        total: Option<i64>,
        content: Option<&PoliticsContent>,
        missing: Option<&str>,
    ) -> bool {
        let missing = missing.filter(|_| total.is_none());
        if self.active.is_empty()
            && self
                .empty_snapshot
                .as_ref()
                .is_some_and(|(old, reason)| *old == total && reason.as_deref() == missing)
            && let Some(last) = self.history.last_mut()
            && last.through_hour.checked_add(1) == Some(frame.hour)
        {
            self.last_hour = Some(frame.hour);
            last.through_hour = frame.hour;
            return false;
        }
        #[cfg(test)]
        SNAPSHOTS.with(|count| count.set(count.get() + 1));
        let value = self.snapshot(frame, total, content, missing);
        self.empty_snapshot = self
            .active
            .is_empty()
            .then(|| (total, missing.map(str::to_owned)));
        self.append(value)
    }

    /// The unit leader keys and character ids naming the allocators, read
    /// with `leaders`.
    fn name_rows(&self, leaders: Option<&[UnitLeaderRow]>) -> (Vec<u128>, Vec<u32>) {
        let mut rows = (Vec::new(), Vec::new());
        for holder in self.active.values() {
            let (leader, character) = holder.name_rows(leaders);
            rows.0.extend(leader);
            rows.1.extend(character);
        }
        rows
    }
}

impl Holder {
    /// The unit leader key and character id [`held_name`] reads to name this
    /// allocator in a read holding `leaders`.
    fn name_rows(&self, leaders: Option<&[UnitLeaderRow]>) -> (Option<u128>, Option<u32>) {
        if self.cause == PowerCause::AbilityActivation || self.item.object_kind == "Ability" {
            return (None, None);
        }
        let reference = (self.item.object_kind == "Leader")
            .then(|| self.item.object_reference.as_deref()?.parse::<u64>().ok())
            .flatten();
        let leader = reference.zip(leaders).and_then(|(reference, leaders)| {
            let at = leaders
                .binary_search_by_key(&reference, |row| {
                    (u64::from(row.id) << 32) | u64::from(row.reference_type)
                })
                .ok()?;
            Some(&leaders[at])
        });
        let character = self
            .item
            .character
            .or_else(|| leader.map(|row| row.character_id));
        (reference.map(u128::from), character)
    }
    /// Whether [`held_name`] reads one of the changed tables: characters,
    /// unit leaders and the lexical dictionary.
    fn named_from(&self, [characters, leaders, lexical]: [bool; 3]) -> bool {
        if self.cause == PowerCause::AbilityActivation || self.item.object_kind == "Ability" {
            return lexical && self.item.key.is_empty();
        }
        let leader = self.item.object_kind == "Leader";
        (leader || self.item.character.is_some()) && (characters || lexical || (leader && leaders))
    }
}

fn held_name(
    frame: &DecodedFrame,
    holder: &Holder,
    content: Option<&PoliticsContent>,
) -> Result<String, String> {
    let content = content.ok_or("recorded content has not been resolved")?;
    if holder.cause == PowerCause::AbilityActivation || holder.item.object_kind == "Ability" {
        let token = facts::token_identity(frame, &holder.item)
            .ok_or("ability holder identity is not recorded")?;
        // Ability names start as the token and can be replaced by authored `name`.
        let power = content.power_content()?;
        let key = power.abilities.name_key(token.as_bytes()).ok_or_else(|| {
            format!("ability {token}: name key is not registered in recorded content")
        })?;
        return content
            .localised(key, View::Plain)
            .ok_or_else(|| format!("ability {token}: name is not localised"));
    }
    if let Some(name) = facts::recorded_name(frame, &holder.item, content) {
        return name.ok_or("recorded holder character name is not identified".into());
    }
    Err(format!(
        "{} holder naming form is not identified",
        holder.kind
    ))
}

impl CountryHolds {
    fn apply(&mut self, row: &LedgerMutation, cause: PowerCause) {
        let Some(sequence) = row.sequence.parse::<u64>().ok() else {
            self.active.clear();
            return;
        };
        if self.sequence.is_some_and(|old| sequence <= old) {
            return;
        }
        self.sequence = Some(sequence);
        if !is_hold(row) || row.operation == "AllocationClamp" {
            return;
        }
        let amount = row
            .amount_raw
            .as_deref()
            .and_then(|raw| raw.parse::<i128>().ok());
        let key = holder_key(&row.item);
        if row.cause == "AllocationRefund" {
            // Anonymous releases cannot tell which named allocator left.
            let Some((key, amount)) = key.zip(amount).filter(|(_, amount)| *amount > 0) else {
                self.active.clear();
                return;
            };
            if let Some(holder) = self.active.get_mut(&key) {
                if amount > holder.amount {
                    self.active.clear();
                } else if amount == holder.amount {
                    self.active.remove(&key);
                } else {
                    holder.amount -= amount;
                }
            } else {
                self.active.clear();
            }
            return;
        }
        if row.site == "ReleaseCommandPower" {
            self.active.clear();
            return;
        }
        if row.site != "AllocateCommandPower" {
            return;
        }
        // A successful charged allocation records the evaluated negative
        // operand. Uncharged allocations have no operand in old recordings;
        // the hourly total below preserves their unidentified remainder.
        let Some((key, amount)) = key.zip(amount).filter(|(_, amount)| *amount < 0) else {
            return;
        };
        let amount = -amount;
        if let Some(holder) = self.active.get_mut(&key) {
            holder.amount += amount;
        } else {
            self.active.insert(
                key,
                Holder {
                    kind: holder_kind(row),
                    item: row.item.clone(),
                    amount,
                    cause,
                },
            );
        }
    }

    fn snapshot(
        &mut self,
        frame: &DecodedFrame,
        total: Option<i64>,
        content: Option<&PoliticsContent>,
        missing: Option<&str>,
    ) -> LedgerCommandHold {
        let Some(total) = total else {
            self.active.clear();
            return LedgerCommandHold {
                read_hour: frame.hour,
                through_hour: frame.hour,
                total_raw: None,
                holders: Vec::new(),
                reason: Some(missing.unwrap_or("CountryCommandPower unavailable").into()),
            };
        };
        let total = i128::from(total);
        let mut attributed: i128 = self.active.values().map(|holder| holder.amount).sum();
        if total == 0 || attributed > total {
            // An observed total never permits stale identities or an invented
            // negative remainder. Native negative totals remain signed.
            self.active.clear();
            attributed = 0;
        }
        let mut reasons = BTreeMap::<String, usize>::new();
        let mut holders: Vec<_> = self
            .active
            .iter()
            .map(|(id, holder)| {
                let name = match held_name(frame, holder, content) {
                    Ok(name) => Some(name),
                    Err(reason) => {
                        *reasons.entry(reason).or_default() += 1;
                        None
                    }
                };
                LedgerCommandHolder {
                    id: id.clone(),
                    kind: holder.kind.clone(),
                    name,
                    amount_raw: Some(holder.amount.to_string()),
                }
            })
            .collect();
        let remainder = total - attributed;
        if remainder != 0 {
            holders.push(LedgerCommandHolder {
                id: "notIdentified".into(),
                kind: "Not identified".into(),
                name: None,
                amount_raw: Some(remainder.to_string()),
            });
            *reasons
                .entry(
                    "Command power holder identity is not recorded for part of the held amount"
                        .into(),
                )
                .or_default() += 1;
        }
        let reason = (!reasons.is_empty()).then(|| {
            reasons
                .into_iter()
                .map(|(reason, count)| format!("{reason} (count: {count})"))
                .collect::<Vec<_>>()
                .join("; ")
        });
        LedgerCommandHold {
            read_hour: frame.hour,
            through_hour: frame.hour,
            total_raw: Some(total.to_string()),
            holders,
            reason,
        }
    }

    fn append(&mut self, value: LedgerCommandHold) -> bool {
        self.last_hour = Some(value.through_hour);
        if let Some(last) = self.history.last_mut()
            && last.through_hour.checked_add(1) == Some(value.read_hour)
            && last.total_raw == value.total_raw
            && last.holders == value.holders
            && last.reason == value.reason
        {
            last.through_hour = value.through_hour;
            return false;
        }
        self.history.push(value);
        true
    }
}

pub(super) fn observe_holds(
    frame: &DecodedFrame,
    scan: &mut Scanned,
    content: Option<&PoliticsContent>,
) {
    let _cpu = ::replay_frame::cpu::scope_phase("politics_ledger:accounting_holds");
    let mut holds = std::mem::take(&mut scan.holds);
    let native = Arc::clone(&scan.inputs);
    let countries_known = frame.complete::<tables::Countries>().is_ok();
    let countries = &native.countries;
    let by_country = &native.by_slot;
    let by_slot = &native.command;
    let affected: BTreeSet<_> = native
        .slots
        .iter()
        .filter_map(|slot| countries.get(slot))
        .filter_map(|row| valid_recorded_tag(row).ok().map(str::to_owned))
        .chain(native.previous_tags.iter().cloned())
        .collect();
    let mutations_complete = frame.complete::<tables::CountryPowerChanges>().is_ok();
    let mut mutations = BTreeMap::<_, Vec<&Mutation>>::new();
    let mutation_tags: BTreeSet<_> = affected.iter().chain(scan.dirty.iter()).collect();
    for history in mutation_tags
        .iter()
        .filter_map(|tag| scan.countries.get(*tag))
    {
        let start = history
            .mutations
            .partition_point(|row| row.read_hour < frame.hour);
        for row in history.mutations[start..]
            .iter()
            .take_while(|row| row.read_hour == frame.hour)
        {
            if row.row.currency == PowerCurrency::Command {
                mutations
                    .entry((row.row.country, row.row.epoch))
                    .or_default()
                    .push(row);
            }
        }
    }
    for rows in mutations.values_mut() {
        rows.sort_by_key(|row| row.row.sequence);
    }
    let mut present = BTreeSet::new();
    let refused = &native.refused_reasons;
    // Carry allocators by published country link before updating reused tags.
    let mut previous = BTreeMap::new();
    for tag in &affected {
        let Some(state) = holds.countries.get_mut(tag) else {
            continue;
        };
        if state.last_hour.and_then(|hour| hour.checked_add(1)) == Some(frame.hour)
            && let Some(identity) = state.identity
        {
            previous.insert(
                identity,
                (std::mem::take(&mut state.active), state.sequence.take()),
            );
        }
    }
    for country in native.slots.iter().filter_map(|slot| countries.get(slot)) {
        let Ok(tag) = std::str::from_utf8(&country.tag) else {
            continue;
        };
        if validate_tag(tag).is_err() {
            continue;
        }
        present.insert(tag);
        let state = holds.countries.entry(tag.to_owned()).or_default();
        state.slot = Some(country.canonical_slot);
        if state.last_hour.is_some_and(|old| old == frame.hour) {
            continue;
        }
        let witness = by_country.get(&country.canonical_slot);
        let identity = witness.map(|row| (row.country, row.epoch));
        let carried = identity.and_then(|identity| previous.remove(&identity));
        let contiguous = carried.is_some();
        if state.identity != identity {
            state.identity = identity;
            state.sequence = None;
            state.active.clear();
        }
        if let Some((active, sequence)) = carried {
            state.active = active;
            state.sequence = sequence;
        }
        if !contiguous
            || !mutations_complete
            || !witness.is_some_and(|row| {
                row.streams & PoliticsCaptureCoverageRow::CP != 0
                    && row.since_hour <= frame.hour
                    && row.through_hour >= frame.hour
            })
        {
            state.active.clear();
        }
        if let Some(rows) = identity.and_then(|identity| mutations.get(&identity)) {
            for stored in rows {
                let row = &stored.ledger();
                if !row.covered {
                    if is_hold(row) {
                        state.active.clear();
                    }
                    continue;
                }
                state.apply(row, stored.facts.native_cause);
            }
        }
        let total = by_slot.get(&country.canonical_slot).copied();
        if state.observe_snapshot(
            frame,
            total,
            content,
            Some("CountryCommandPower row unavailable"),
        ) {
            scan.dirty.insert(tag.to_owned());
        }
    }
    for tag in &affected {
        let Some(state) = holds.countries.get_mut(tag) else {
            continue;
        };
        if present.contains(tag.as_str()) || state.last_hour == Some(frame.hour) {
            continue;
        }
        state.active.clear();
        state.identity = None;
        state.sequence = None;
        // Invalid current tag bytes refuse this known country's identity;
        // they do not prove that the country ceased to exist.
        let refusal = state.slot.and_then(|slot| refused.get(&slot));
        let total = (countries_known && refusal.is_none()).then_some(0);
        if state.observe_snapshot(
            frame,
            total,
            content,
            Some(refusal.map_or("Countries unavailable", String::as_str)),
        ) {
            scan.dirty.insert(tag.clone());
        }
    }
    // Index the rows naming the allocators of every country observed.
    let leaders = frame.complete::<tables::UnitLeaders>().ok();
    let names = holds.names.get_or_insert_with(|| NameRows {
        unindexed: holds
            .countries
            .iter()
            .filter(|(_, country)| !country.active.is_empty())
            .map(|(tag, _)| tag.clone())
            .collect(),
        ..Default::default()
    });
    for tag in &affected {
        let rows = holds
            .countries
            .get(tag)
            .map(|country| country.name_rows(leaders))
            .unwrap_or_default();
        names.index(tag, rows);
    }
    scan.holds = holds;
}

#[cfg(test)]
thread_local! {
    /// Countries observed again because their allocators may be renamed.
    static NAMED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static SNAPSHOTS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
#[cfg(test)]
pub(super) fn named_count() -> usize {
    NAMED.with(std::cell::Cell::get)
}

#[cfg(test)]
pub(super) fn snapshot_count() -> usize {
    SNAPSHOTS.with(std::cell::Cell::get)
}

pub(super) fn history<'a>(scan: &'a Scanned, tag: &str) -> &'a [LedgerCommandHold] {
    scan.holds
        .countries
        .get(tag)
        .map_or(&[], |country| country.history.as_slice())
}
#[cfg(test)]
pub(super) fn present_holds(scan: &Scanned, tag: &str) -> Vec<LedgerCommandHold> {
    history(scan, tag).to_vec()
}

#[cfg(test)]
mod tests;
