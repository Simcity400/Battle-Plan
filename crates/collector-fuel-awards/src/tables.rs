use crate::{
    fuel_awards::{self, CompactEntry, CompactPayload, NO_NAME, RootData},
    output_log::Drained,
    table_capture::{grow_capacity, grow_vec},
};
use engine_allocation::{copy_vec, default_vec, owned_vec, reserve_map};
use replay_data::{
    BranchId, rows::FuelAwardDay, rows::FuelAwardDayHour, rows::FuelAwardDayRecipient,
    rows::FuelAwardHour, rows::FuelAwardRecipient, rows::FuelAwardRecipientRow, rows::Recorded,
    rows::Retired, rows::RowCodec, rows::TableId, rows::TableRows,
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
    entries: Vec<CompactEntry>,
    roots: Vec<RootData>,
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
            roots: Vec::new(),
            names: owned_vec(engine_allocation::filled(capacity.names, 0)),
            drained: None,
        }
    }
    pub fn reset(&mut self) {
        self.roots.clear();
        self.drained = None;
    }
    pub fn allocated_bytes(&self) -> usize {
        self.entries.capacity() * size_of::<CompactEntry>()
            + self.roots.capacity() * size_of::<RootData>()
            + self.names.capacity()
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
        self.drained = Some(fuel_awards::drain_compact(
            &mut self.entries,
            &mut self.roots,
            &mut self.names,
        ));
    }
}

pub const TRUNCATED_RECIPIENTS: &str =
    "fuel award recipients are not split from a truncated CountryFuel read, whose days stay whole";

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
        let (rows, complete_fuel) = match &mut tables[TableId::CountryFuel.index()] {
            TableInput::Sampled(TableView::Complete(TableRows::CountryFuel(rows))) => (rows, true),
            TableInput::Sampled(TableView::Truncated {
                rows: TableRows::CountryFuel(rows),
                ..
            }) => (rows, false),
            TableInput::Sampled(TableView::Unavailable { reason }) => {
                tables[TableId::FuelAwardRecipients.index()] =
                    TableInput::Sampled(TableView::Unavailable {
                        reason: reason.clone(),
                    });
                return Ok(());
            }
            _ => {
                tables[TableId::FuelAwardRecipients.index()] =
                    TableInput::Sampled(TableView::Unavailable {
                        reason: "fuel award recipients follow a CountryFuel read".into(),
                    });
                return Ok(());
            }
        };
        let mut recipients: Vec<FuelAwardRecipientRow> = Vec::new();
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
                match sum_day(hours, hour)? {
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
            if complete_fuel && reason.is_none() && day.splittable(hour) {
                let (header, mut owned) = day.split(row.country, hour)?;
                engine_allocation::reserve(&mut recipients, owned.len())?;
                recipients.append(&mut owned);
                day = header;
            }
            row.awards = Recorded::Value(Retired);
            row.award_day = Recorded::Value(day);
        }
        tables[TableId::FuelAwardRecipients.index()] = if complete_fuel {
            recipients.sort_unstable_by_key(RowCodec::key);
            TableInput::Sampled(TableView::Complete(TableRows::FuelAwardRecipients(
                recipients,
            )))
        } else {
            TableInput::Sampled(TableView::Unavailable {
                reason: TRUNCATED_RECIPIENTS.into(),
            })
        };
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
        let mut recipients: HashMap<u64, Vec<&CompactEntry>> = HashMap::new();
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
            .filter(|entry| matches!(entry.payload, CompactPayload::Root { .. }))
            .count();
        reserve_map(&mut recipients, roots)?;
        let mut completed = HashMap::new();
        reserve_map(&mut completed, ends)?;
        reserve_map(&mut self.countries, ends)?;
        reserve_map(&mut self.identities, ends)?;
        reserve_map(&mut self.status_owners, ends)?;
        for entry in entries {
            let (country, root_index) = match entry.payload {
                CompactPayload::Reset { status, .. } => {
                    if let Some(country) = self.status_owners.remove(&status) {
                        self.countries.remove(&country);
                        self.identities.remove(&country);
                    }
                    continue;
                }
                CompactPayload::Recipient { .. } => {
                    let root = recipients.entry(entry.root).or_default();
                    engine_allocation::reserve(root, 1)?;
                    root.push(entry);
                    continue;
                }
                CompactPayload::Root { country, index } => (country, index as usize),
            };
            let Some(root_data) = captured.roots.get(root_index) else {
                self.countries.remove(&country);
                recipients.remove(&entry.root);
                continue;
            };
            let repeated = completed.insert(country, entry.root).is_some();
            if repeated || root_data.refused {
                self.countries.remove(&country);
                recipients.remove(&entry.root);
                continue;
            }
            let mut rows = Vec::new();
            let part = recipients.remove(&entry.root).unwrap_or_default();
            engine_allocation::reserve_exact(&mut rows, part.len())?;
            let mut received = [0_i64; 3];
            let mut valid = true;
            for part in part {
                let CompactPayload::Recipient {
                    reference,
                    amount,
                    name_reason: part_name_reason,
                    country: part_country,
                    owner,
                    host,
                    name_start,
                    name_len,
                    kind,
                } = part.payload
                else {
                    continue;
                };
                if part_country != country || kind > 2 {
                    valid = false;
                    break;
                }
                received[kind as usize] = received[kind as usize].wrapping_add(amount);
                let name = (name_start != NO_NAME)
                    .then_some((name_start as usize, name_len as usize))
                    .and_then(|(start, len)| {
                        start
                            .checked_add(len)
                            .and_then(|end| captured.names.get(start..end))
                    })
                    .map(copy_vec)
                    .transpose()?;
                let name_reason = part_name_reason.or_else(|| {
                    (kind != 1 && name.is_none())
                        .then_some("fuel recipient stored name not recorded")
                });
                rows.push(FuelAwardRecipient {
                    kind,
                    reference,
                    owner,
                    host,
                    amount_raw: amount,
                    name,
                    name_reason: copy_vec(name_reason.unwrap_or("").as_bytes())?,
                });
            }
            if !valid || received != root_data.settled.received {
                self.countries.remove(&country);
                continue;
            }
            let identity = (root_data.started.country_instance, root_data.started.status);
            let old = self.identities.insert(country, identity);
            if old.map(|(_, status)| status) != Some(identity.1) {
                if let Some((_, status)) = old {
                    self.status_owners.remove(&status);
                }
                self.countries.remove(&country);
            }
            if let Some(old) = self
                .status_owners
                .insert(root_data.started.status, country)
                && old != country
            {
                self.countries.remove(&old);
                self.identities.remove(&old);
            }
            let (start, history) = self
                .countries
                .entry(country)
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
                gains_raw: root_data.settled.gains,
                own_gain_raw: root_data.settled.own_gain,
                transfers_raw: root_data.settled.transfers,
                consumed_raw: root_data.settled.consumed,
                received_raw: root_data.settled.received,
                recipients: rows,
                history_index: root_data.started.history_index,
                remaining_hours: root_data.started.remaining_hours,
                pending_raw: [
                    root_data
                        .started
                        .pending_gain
                        .wrapping_add(root_data.settled.own_gain),
                    root_data
                        .started
                        .pending_consumed
                        .wrapping_add(root_data.settled.consumed),
                    root_data
                        .started
                        .pending_other
                        .wrapping_add(root_data.settled.transfers[3]),
                ],
                current_raw: root_data.settled.current,
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

fn sum_day(hours: &VecDeque<FuelAwardHour>, hour: i32) -> io::Result<Option<FuelAwardDay>> {
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
    let last = hours.back().map_or(0, |last| last.hour);
    Ok(Some(FuelAwardDay {
        last_hour: if last == hour {
            FuelAwardDayHour::Host
        } else {
            FuelAwardDayHour::Stamped(last)
        },
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
