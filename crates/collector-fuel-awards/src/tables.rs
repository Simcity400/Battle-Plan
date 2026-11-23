use crate::{
    fuel_awards::{self, Entry},
    output_log::Drained,
    table_capture::{grow_capacity, grow_vec},
};
use engine_allocation::{copy_vec, default_vec, owned_vec, reserve_map};
use replay_data::{
    BranchId, rows::FuelAwardDay, rows::FuelAwardDayRecipient, rows::FuelAwardHour,
    rows::FuelAwardRecipient, rows::Recorded, rows::Retired, rows::TableId, rows::TableRows,
};
use replay_frame::{frame::TableInput, frame::TableView};
use std::{
    collections::{HashMap, VecDeque},
    io,
};

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Capacities {
    pub entries: usize,
    pub names: usize,
}

collector_primitives::capacity_max!(Capacities { entries, names });
pub struct Captured {
    entries: Vec<Entry>,
    names: Vec<u8>,
    drained: Option<Result<(Drained, usize, usize), &'static str>>,
}
impl Captured {
    pub fn vacant() -> Self {
        Self::with_capacities(&Capacities::default())
    }
    pub fn fail(&mut self, reason: &'static str) {
        fuel_awards::lost_drain();
        self.drained = Some(Err(reason));
    }
    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            entries: owned_vec(default_vec(capacity.entries)),
            names: owned_vec(engine_allocation::filled(capacity.names, 0)),
            drained: None,
        }
    }
    pub fn reset(&mut self) {
        self.drained = None;
    }
    pub fn allocated_bytes(&self) -> usize {
        self.entries.capacity() * size_of::<Entry>() + self.names.capacity()
    }
    pub fn grow(&mut self, capacity: &mut Capacities) {
        let (entries, names) = fuel_awards::drained_capacities();
        grow_capacity(&mut capacity.entries, entries);
        grow_capacity(&mut capacity.names, names);
        if let Some(Ok((drained, _, required))) = self.drained {
            grow_capacity(&mut capacity.entries, drained.pending);
            grow_capacity(&mut capacity.names, required);
        }
        grow_vec(&mut self.entries, capacity.entries);
        grow_vec(&mut self.names, capacity.names);
    }
    pub fn drain(&mut self) {
        self.drained = Some(fuel_awards::drain(
            &mut self.entries,
            &mut self.names,
        ));
    }
}

#[derive(Default)]
pub struct Windows {
    branch: Option<BranchId>,
    hour: Option<i32>,
    epoch: Option<u64>,
    cursor: Option<u64>,
    countries: HashMap<u32, (i32, VecDeque<FuelAwardHour>)>,
    identities: HashMap<u32, (u32, usize)>,
    status_owners: HashMap<usize, u32>,
    reason: Option<&'static str>,
}
impl Windows {
    pub fn assemble(
        &mut self,
        branch: BranchId,
        hour: i32,
        captured: &Captured,
        tables: &mut [TableInput],
    ) -> io::Result<()> {
        self.observe(branch, hour, captured)?;
        let rows = match &mut tables[TableId::CountryFuel.index()] {
            TableInput::Sampled(TableView::Complete(TableRows::CountryFuel(rows))) => rows,
            TableInput::Sampled(TableView::Truncated {
                rows: TableRows::CountryFuel(rows),
                ..
            }) => rows,
            _ => return Ok(()),
        };
        for row in rows {
            let history = self.countries.get(&row.country);
            let hours = history.map(|(_, hours)| hours);
            let complete = hours.is_some_and(|hours| {
                hours.len() == 24
                    && hours.iter().enumerate().all(|(index, entry)| {
                        Some(entry.hour) == hour.checked_sub(23 - index as i32)
                    })
            });
            let instance = self
                .identities
                .get(&row.country)
                .map(|&(instance, _)| instance);
            let mut reason = if complete {
                None
            } else {
                self.reason.or(Some(
                    "fuel applied window lacks 24 consecutive updates of this country identity",
                ))
            };
            let mut day = FuelAwardDay::default();
            if let (None, Some(hours)) = (reason, hours) {
                match sum_day(hours)? {
                    Some(summed) => day = summed,
                    None => {
                        DAY_SUM_REFUSALS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        reason = Some("fuel applied day sum exceeds its native i64");
                    }
                }
            }
            day.country_instance = instance.unwrap_or_default();
            day.continuity_start_hour = history.map(|(start, _)| *start);
            day.reason = copy_vec(reason.unwrap_or("").as_bytes())?;
            row.awards = Recorded::Value(Retired);
            row.award_day = Recorded::Value(day);
        }
        Ok(())
    }
    fn observe(&mut self, branch: BranchId, hour: i32, captured: &Captured) -> io::Result<()> {
        let discontinuous =
            self.branch != Some(branch) || self.hour.and_then(|h| h.checked_add(1)) != Some(hour);
        self.branch = Some(branch);
        self.hour = Some(hour);
        let Some(Ok((drained, name_len, _))) = captured.drained else {
            self.countries.clear();
            self.identities.clear();
            self.status_owners.clear();
            self.cursor = None;
            self.epoch = None;
            self.reason = Some("fuel award handover unavailable");
            return Ok(());
        };
        let complete = drained.first.checked_add(drained.copied as u64) == Some(drained.cursor)
            && drained.copied == drained.pending
            && name_len <= captured.names.len();
        let continuous = !discontinuous
            && self.epoch == Some(drained.epoch)
            && self.cursor == Some(drained.first);
        self.epoch = Some(drained.epoch);
        self.cursor = Some(drained.cursor);
        if !continuous {
            self.countries.clear();
            self.identities.clear();
            self.status_owners.clear();
            self.reason = Some("fuel award branch or writer boundary has no prior sample");
            return Ok(());
        }
        if !complete {
            self.countries.clear();
            self.identities.clear();
            self.status_owners.clear();
            self.reason = Some("fuel award handover incomplete or overflowed");
            return Ok(());
        }
        self.reason = None;
        self.countries.retain(|_, (_, history)| {
            history
                .back()
                .is_some_and(|last| hour.saturating_sub(last.hour) < 24)
        });
        self.identities
            .retain(|country, _| self.countries.contains_key(country));
        self.status_owners.retain(|status, country| {
            self.identities
                .get(country)
                .is_some_and(|(_, pointer)| pointer == status)
        });
        let mut recipients: HashMap<u64, Vec<&Entry>> = HashMap::new();
        let entries = captured
            .entries
            .get(..drained.copied)
            .ok_or_else(|| io::Error::other("fuel drain exceeds owned entries"))?;
        let roots = (!entries.is_empty() as usize)
            + entries
                .windows(2)
                .filter(|pair| pair[0].root != pair[1].root)
                .count();
        let ends = entries
            .iter()
            .filter(|entry| entry.recipient.is_none() && !entry.reset)
            .count();
        reserve_map(&mut recipients, roots)?;
        let mut completed = HashMap::new();
        reserve_map(&mut completed, ends)?;
        reserve_map(&mut self.countries, ends)?;
        reserve_map(&mut self.identities, ends)?;
        reserve_map(&mut self.status_owners, ends)?;
        for entry in entries {
            if entry.reset {
                if let Some(country) = self.status_owners.remove(&entry.started.status) {
                    self.countries.remove(&country);
                    self.identities.remove(&country);
                }
                continue;
            }
            if entry.recipient.is_some() {
                let root = recipients.entry(entry.root).or_default();
                engine_allocation::reserve(root, 1)?;
                root.push(entry);
                continue;
            }
            let repeated = completed.insert(entry.country, entry.root).is_some();
            if repeated || entry.refused {
                self.countries.remove(&entry.country);
                recipients.remove(&entry.root);
                continue;
            }
            let mut rows = Vec::new();
            let part = recipients.remove(&entry.root).unwrap_or_default();
            engine_allocation::reserve_exact(&mut rows, part.len())?;
            let mut received = [0_i64; 3];
            let mut valid = true;
            for part in part {
                let Some(recipient) = part.recipient else {
                    continue;
                };
                if part.country != entry.country || recipient.kind > 2 {
                    valid = false;
                    break;
                }
                received[recipient.kind as usize] =
                    received[recipient.kind as usize].wrapping_add(part.amount);
                let name = part
                    .name
                    .and_then(|(start, len)| {
                        start
                            .checked_add(len)
                            .and_then(|end| captured.names.get(start..end))
                    })
                    .map(copy_vec)
                    .transpose()?;
                let name_reason = part.name_reason.or_else(|| {
                    (recipient.kind != 1 && name.is_none())
                        .then_some("fuel recipient stored name not recorded")
                });
                rows.push(FuelAwardRecipient {
                    kind: recipient.kind,
                    reference: recipient.reference,
                    owner: recipient.owner,
                    host: recipient.host,
                    amount_raw: part.amount,
                    name,
                    name_reason: copy_vec(name_reason.unwrap_or("").as_bytes())?,
                });
            }
            if !valid || received != entry.settled.received {
                self.countries.remove(&entry.country);
                continue;
            }
            let identity = (entry.started.country_instance, entry.started.status);
            let old = self.identities.insert(entry.country, identity);
            if old.map(|(_, status)| status) != Some(identity.1) {
                if let Some((_, status)) = old {
                    self.status_owners.remove(&status);
                }
                self.countries.remove(&entry.country);
            }
            if let Some(old) = self
                .status_owners
                .insert(entry.started.status, entry.country)
                && old != entry.country
            {
                self.countries.remove(&old);
                self.identities.remove(&old);
            }
            let (start, history) = self
                .countries
                .entry(entry.country)
                .or_insert_with(|| (hour, VecDeque::new()));
            if history
                .back()
                .is_some_and(|old| old.hour.checked_add(1) != Some(hour))
            {
                history.clear();
                *start = hour;
            }
            reserve_history(history)?;
            history.push_back(FuelAwardHour {
                hour,
                gains_raw: entry.settled.gains,
                own_gain_raw: entry.settled.own_gain,
                transfers_raw: entry.settled.transfers,
                consumed_raw: entry.settled.consumed,
                received_raw: entry.settled.received,
                recipients: rows,
                history_index: entry.started.history_index,
                remaining_hours: entry.started.remaining_hours,
                pending_raw: [
                    entry
                        .started
                        .pending_gain
                        .wrapping_add(entry.settled.own_gain),
                    entry
                        .started
                        .pending_consumed
                        .wrapping_add(entry.settled.consumed),
                    entry
                        .started
                        .pending_other
                        .wrapping_add(entry.settled.transfers[3]),
                ],
                current_raw: entry.settled.current,
            });
            while history.len() > 24 {
                history.pop_front();
            }
        }
        Ok(())
    }
}
static DAY_SUM_REFUSALS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub fn day_sum_refusals() -> u64 {
    DAY_SUM_REFUSALS.load(std::sync::atomic::Ordering::Relaxed)
}

type Receipt<'a> = ((u32, u64, u32, u64, Option<&'a [u8]>, &'a [u8]), i32, i64);

fn sum_day(hours: &VecDeque<FuelAwardHour>) -> io::Result<Option<FuelAwardDay>> {
    let mut gains = [0_i128; 4];
    let mut transfers = [0_i128; 4];
    let mut received = [0_i128; 3];
    let (mut own, mut quantity, mut consumed) = (0_i128, 0_i128, 0_i128);
    let mut receipts: Vec<Receipt<'_>> = Vec::new();
    engine_allocation::reserve_exact(
        &mut receipts,
        hours.iter().map(|entry| entry.recipients.len()).sum(),
    )?;
    for entry in hours {
        for (sum, raw) in gains.iter_mut().zip(entry.gains_raw) {
            *sum += i128::from(raw);
        }
        for (sum, raw) in transfers.iter_mut().zip(entry.transfers_raw) {
            *sum += i128::from(raw);
        }
        for (sum, raw) in received.iter_mut().zip(entry.received_raw) {
            *sum += i128::from(raw);
        }
        own += i128::from(entry.own_gain_raw);
        quantity += i128::from(engine_fuel::fuel_awards::quantity(entry.own_gain_raw));
        consumed += i128::from(entry.consumed_raw);
        for part in &entry.recipients {
            receipts.push((
                (
                    part.kind,
                    part.reference,
                    part.owner,
                    part.host,
                    part.name.as_deref(),
                    part.name_reason.as_slice(),
                ),
                entry.hour,
                part.amount_raw,
            ));
        }
    }
    receipts.sort_unstable_by(|a, b| a.0.cmp(&b.0));
    let groups = receipts
        .iter()
        .enumerate()
        .filter(|(index, receipt)| *index == 0 || receipts[index - 1].0 != receipt.0)
        .count();
    let mut recipients = Vec::new();
    engine_allocation::reserve_exact(&mut recipients, groups)?;
    let mut at = 0;
    while at < receipts.len() {
        let identity = receipts[at].0;
        let end = at + receipts[at..].partition_point(|receipt| receipt.0 == identity);
        let group = &receipts[at..end];
        let amount: i128 = group.iter().map(|receipt| i128::from(receipt.2)).sum();
        let Ok(amount_raw) = i64::try_from(amount) else {
            return Ok(None);
        };
        let (kind, reference, owner, host, name, name_reason) = identity;
        recipients.push(FuelAwardDayRecipient {
            kind,
            reference,
            owner,
            host,
            name: name.map(copy_vec).transpose()?,
            name_reason: copy_vec(name_reason)?,
            first_hour: group
                .iter()
                .map(|receipt| receipt.1)
                .min()
                .unwrap_or_default(),
            last_hour: group
                .iter()
                .map(|receipt| receipt.1)
                .max()
                .unwrap_or_default(),
            amount_raw,
        });
        at = end;
    }
    let fit = |sum: i128| i64::try_from(sum).ok();
    let sums = (|| {
        Some((
            [
                fit(gains[0])?,
                fit(gains[1])?,
                fit(gains[2])?,
                fit(gains[3])?,
            ],
            fit(own)?,
            fit(quantity)?,
            [
                fit(transfers[0])?,
                fit(transfers[1])?,
                fit(transfers[2])?,
                fit(transfers[3])?,
            ],
            fit(consumed)?,
            [fit(received[0])?, fit(received[1])?, fit(received[2])?],
        ))
    })();
    let Some((
        gains_raw,
        own_gain_raw,
        own_gain_quantity_raw,
        transfers_raw,
        consumed_raw,
        received_raw,
    )) = sums
    else {
        return Ok(None);
    };
    Ok(Some(FuelAwardDay {
        last_hour: hours.back().map_or(0, |last| last.hour),
        gains_raw,
        own_gain_raw,
        own_gain_quantity_raw,
        transfers_raw,
        consumed_raw,
        received_raw,
        recipients,
        ..Default::default()
    }))
}

fn reserve_history(history: &mut VecDeque<FuelAwardHour>) -> io::Result<()> {
    if history.len() == history.capacity() {
        engine_allocation::allocation_attempt()?;
        history
            .try_reserve(1)
            .map_err(|_| engine_allocation::oom())?;
    }
    Ok(())
}

#[cfg(any(test, feature = "test-support"))]
pub fn allocation_failure_history_exercise() -> io::Result<()> {
    reserve_history(&mut VecDeque::new())
}
#[cfg(test)]
#[path = "tables/tests.rs"]
mod tests;
