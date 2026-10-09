//! Current reservations, separate from payments. P: AllocateCommandPower
//! 140707C00 stores allocator+0x10 in country+0x1F8; ReleaseCommandPower
//! 1406DE290 removes that full amount and refunds it via 1406DE4CA.
use super::*;

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

#[derive(Default)]
pub(super) struct Holds {
    countries: BTreeMap<String, CountryHolds>,
}

#[derive(Default)]
struct CountryHolds {
    slot: Option<u32>,
    identity: Option<(u32, u32, u64)>,
    sequence: Option<u64>,
    last_hour: Option<i32>,
    active: BTreeMap<String, Holder>,
    history: Vec<LedgerCommandHold>,
}

struct Holder {
    kind: String,
    item: LedgerItem,
    amount: i128,
    cause: PowerCause,
}

/// P: charging callers also label allocations as AbilityActivation, Raid,
/// AirAction, Advisor or LeaderAssignment. The writer identifies a hold.
/// GroundCrew@1406C6D30 releases/resizes/reallocates at 1406C6D70/1406C7314;
/// its direct 1406C736B subtraction charges the reservation adjustment.
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

fn held_name(
    frame: &DecodedFrame,
    holder: &Holder,
    content: Option<&PoliticsContent>,
) -> Result<String, String> {
    let content = content.ok_or("recorded content has not been resolved")?;
    if holder.cause == PowerCause::AbilityActivation || holder.item.object_kind == "Ability" {
        let token = facts::token_identity(frame, &holder.item)
            .ok_or("ability holder identity is not recorded")?;
        // P: CAbility constructor 140FF44E0 seeds +248 with its token;
        // parse_key@140FF5EC0 replaces it with authored `name`.
        let power = content.power_content()?;
        let key = power.abilities.name_key(token.as_bytes()).ok_or_else(|| {
            format!("ability {token}: name key is not registered in recorded content")
        })?;
        return content
            .localised(key, View::Plain)
            .ok_or_else(|| format!("ability {token}: name is not localised"));
    }
    if let Some(character) = facts::character_fact(frame, &holder.item, holder.cause) {
        return facts::character_name(&character, content)
            .ok_or("recorded holder character name is not identified".into());
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

    fn append(&mut self, value: LedgerCommandHold) {
        self.last_hour = Some(value.through_hour);
        if let Some(last) = self.history.last_mut()
            && last.through_hour.checked_add(1) == Some(value.read_hour)
            && last.total_raw == value.total_raw
            && last.holders == value.holders
            && last.reason == value.reason
        {
            last.through_hour = value.through_hour;
            return;
        }
        self.history.push(value);
    }
}

pub(super) fn observe_holds(
    frame: &DecodedFrame,
    scan: &mut Scanned,
    content: Option<&PoliticsContent>,
) {
    let mut holds = std::mem::take(&mut scan.holds);
    let countries = frame.complete::<tables::Countries>().ok();
    let command = frame.complete::<tables::CountryCommandPower>().ok();
    let witnesses = frame.complete::<tables::PoliticsCaptureCoverage>().ok();
    let links = frame.complete::<tables::CountryLinks>().ok();
    let by_country: BTreeMap<_, _> = witnesses
        .unwrap_or_default()
        .iter()
        .filter_map(|row| crate::metadata::link_slot(links?, row.country).map(|slot| (slot, row)))
        .collect();
    let by_slot: BTreeMap<_, _> = command
        .unwrap_or_default()
        .iter()
        .map(|row| (row.canonical_slot, row))
        .collect();
    let mutations_complete = frame.complete::<tables::CountryPowerChanges>().is_ok();
    let mut mutations = BTreeMap::<_, Vec<&LedgerMutation>>::new();
    for reply in scan.countries.values() {
        let start = reply
            .mutations
            .partition_point(|row| row.read_hour < frame.hour);
        for row in reply.mutations[start..]
            .iter()
            .take_while(|row| row.read_hour == frame.hour)
        {
            if row.currency == "Command"
                && let Ok(epoch) = row.epoch.parse::<u64>()
            {
                mutations
                    .entry((row.country, row.instance, epoch))
                    .or_default()
                    .push(row);
            }
        }
    }
    for rows in mutations.values_mut() {
        rows.sort_by_key(|row| row.sequence.parse::<u64>().ok());
    }
    let mut present = BTreeSet::new();
    let refused: BTreeMap<_, _> = countries
        .unwrap_or_default()
        .iter()
        .filter_map(|country| {
            valid_recorded_tag(country)
                .err()
                .map(|reason| (country.canonical_slot, format!("{reason} (count: 1)")))
        })
        .collect();
    // Carry allocators by native instance before any reused tag is updated.
    let mut previous = BTreeMap::new();
    for state in holds.countries.values_mut() {
        if state.last_hour.and_then(|hour| hour.checked_add(1)) == Some(frame.hour)
            && let Some(identity) = state.identity
        {
            previous.insert(
                identity,
                (std::mem::take(&mut state.active), state.sequence.take()),
            );
        }
    }
    for country in countries.unwrap_or_default() {
        let Ok(tag) = std::str::from_utf8(&country.tag) else {
            continue;
        };
        if validate_tag(tag).is_err() {
            continue;
        }
        present.insert(tag.to_owned());
        let state = holds.countries.entry(tag.to_owned()).or_default();
        state.slot = Some(country.canonical_slot);
        if state.last_hour.is_some_and(|old| old == frame.hour) {
            continue;
        }
        let witness = by_country.get(&country.canonical_slot).copied();
        let identity = witness.map(|row| (row.country, row.instance, row.epoch));
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
            for row in rows {
                if !row.covered {
                    if is_hold(row) {
                        state.active.clear();
                    }
                    continue;
                }
                let cause = scan
                    .event_facts
                    .get(&item_text_id(row))
                    .map_or(PowerCause::NotIdentified, |facts| facts.native_cause);
                state.apply(row, cause);
            }
        }
        let total = by_slot
            .get(&country.canonical_slot)
            .map(|row| row.allocated_command_power_raw);
        let value = state.snapshot(
            frame,
            total,
            content,
            Some("CountryCommandPower row unavailable"),
        );
        state.append(value);
    }
    for (tag, state) in &mut holds.countries {
        if present.contains(tag) || state.last_hour == Some(frame.hour) {
            continue;
        }
        state.active.clear();
        state.identity = None;
        state.sequence = None;
        // Invalid current tag bytes refuse this known country's identity;
        // they do not prove that the country ceased to exist.
        let refusal = state.slot.and_then(|slot| refused.get(&slot));
        let total = countries.and_then(|_| refusal.is_none().then_some(0));
        let value = state.snapshot(
            frame,
            total,
            content,
            Some(refusal.map_or("Countries unavailable", String::as_str)),
        );
        state.append(value);
    }
    scan.holds = holds;
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
