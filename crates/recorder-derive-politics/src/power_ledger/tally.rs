
use replay_data::rows::{CountryPowerDailyRow, PowerSpendTotalRow as Total, Recorded};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct DayRuns(pub(super) Vec<(i32, i32)>);

impl DayRuns {
    pub(super) fn contains(&self, day: i32) -> bool {
        let at = self.0.partition_point(|&(first, _)| first <= day);
        at > 0 && day <= self.0[at - 1].1
    }

    pub(super) fn insert(&mut self, day: i32) -> bool {
        let at = self.0.partition_point(|&(first, _)| first <= day);
        if at > 0 && day <= self.0[at - 1].1 {
            return false;
        }
        let merge_left = at > 0 && self.0[at - 1].1.checked_add(24) == Some(day);
        let merge_right = at < self.0.len() && day.checked_add(24) == Some(self.0[at].0);
        match (merge_left, merge_right) {
            (true, true) => {
                self.0[at - 1].1 = self.0[at].1;
                self.0.remove(at);
            }
            (true, false) => self.0[at - 1].1 = day,
            (false, true) => self.0[at].0 = day,
            (false, false) => self.0.insert(at, (day, day)),
        }
        true
    }

    pub(super) fn remove(&mut self, day: i32) -> bool {
        let at = self.0.partition_point(|&(first, _)| first <= day);
        if at == 0 || day > self.0[at - 1].1 {
            return false;
        }
        let (first, last) = self.0[at - 1];
        match (first == day, last == day) {
            (true, true) => {
                self.0.remove(at - 1);
            }
            (true, false) => self.0[at - 1].0 = day + 24,
            (false, true) => self.0[at - 1].1 = day - 24,
            (false, false) => {
                self.0[at - 1].1 = day - 24;
                self.0.insert(at, (day + 24, last));
            }
        }
        true
    }

    pub(super) fn is_subset_of(&self, whole: &Self) -> bool {
        let mut at = 0;
        for &(first, last) in &self.0 {
            while at < whole.0.len() && whole.0[at].1 < first {
                at += 1;
            }
            if at == whole.0.len() || whole.0[at].0 > first || whole.0[at].1 < last {
                return false;
            }
        }
        true
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct ItemDays {
    pub(super) counted: DayRuns,
    pub(super) uncovered: DayRuns,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Payment {
    pub recurring: bool,
    pub upkeep: bool,
    pub hour: i32,
    pub read: i32,
    pub sequence: u64,
    pub ordinal: u32,
    pub charge: Option<i128>,
    pub covered: bool,
    pub realized: Option<i128>,
    pub count: i64,
}

pub(super) type GainDay = (Option<i64>, i64, bool);

pub(super) fn item(holder: u32, currency: u8, ordinal: u32) -> Total {
    Total {
        holder,
        currency,
        ordinal,
        gained: Recorded::NotRecorded,
        capped: Recorded::NotRecorded,
        gain_days: Recorded::NotRecorded,
        gain_unknown_days: Recorded::NotRecorded,
        unmatched: Recorded::NotRecorded,
        ..Total::default()
    }
}

pub(super) fn gains(holder: u32, currency: u8) -> Total {
    use Recorded::NotRecorded as No;
    Total {
        holder,
        currency,
        ordinal: Total::GAINS,
        known: No,
        payments: No,
        entries: No,
        unknown: No,
        uncovered: No,
        latest_hour: No,
        latest_sequence: No,
        latest_ordinal: No,
        latest_read: No,
        days: No,
        day_payments: No,
        day_entries: No,
        day_charge: No,
        day_unknown: No,
        days_uncovered: No,
        realized_gained: No,
        realized_lost: No,
        realized_unknown: No,
        first_day: No,
        last_day: No,
        last_lag: No,
        last_hour: No,
        last_sequence: No,
        last_ordinal: No,
        last_covered: No,
        balances: No,
        credits: No,
        ..Total::default()
    }
}

fn add(cell: &mut Recorded<i64>, amount: i128) {
    if let Recorded::Value(held) = *cell {
        *cell = i64::try_from(i128::from(held) + amount).map_or(Recorded::NotRecorded, Recorded::Value);
    }
}

fn get<T: Copy>(cell: Recorded<Option<T>>) -> Option<T> {
    match cell {
        Recorded::Value(value) => value,
        Recorded::NotRecorded => None,
    }
}

fn set<T>(cell: &mut Recorded<Option<T>>, value: T) {
    *cell = Recorded::Value(Some(value));
}

pub(super) fn midnight(hour: i32) -> i32 {
    hour.div_euclid(24) * 24
}

pub(super) fn pay(total: &mut Total, seen: &mut ItemDays, payment: &Payment) -> Option<(i32, bool, bool)> {
    if payment.recurring {
        day(total, seen, payment)
    } else {
        once(total, payment);
        None
    }
}

fn once(total: &mut Total, payment: &Payment) {
    let count = i128::from(payment.count);
    if count <= 0 {
        return;
    }
    add(&mut total.known, payment.charge.unwrap_or(0));
    add(&mut total.payments, count);
    add(&mut total.entries, count);
    add(&mut total.unknown, if payment.charge.is_none() { count } else { 0 });
    add(&mut total.uncovered, if !payment.covered { count } else { 0 });
    let latest = (payment.hour, payment.sequence, payment.ordinal);
    let held = get(total.latest_hour).zip(get(total.latest_sequence)).zip(get(total.latest_ordinal)).map(|((hour, sequence), ordinal)| (hour, sequence, ordinal));
    if held.is_none_or(|held| latest >= held) {
        set(&mut total.latest_hour, payment.hour);
        set(&mut total.latest_sequence, payment.sequence);
        set(&mut total.latest_ordinal, payment.ordinal);
        set(&mut total.latest_read, payment.read);
    }
}

fn day(total: &mut Total, seen: &mut ItemDays, payment: &Payment) -> Option<(i32, bool, bool)> {
    let count = i128::from(payment.count);
    if count <= 0 {
        return None;
    }
    let day = midnight(payment.hour);
    let lag = payment.read.saturating_sub(day);
    let new_day = seen.counted.insert(day);
    let newly_uncovered = !payment.covered && seen.uncovered.insert(day);
    add(&mut total.day_payments, count);
    add(&mut total.day_charge, payment.charge.unwrap_or(0));
    add(&mut total.day_unknown, if payment.charge.is_none() { count } else { 0 });
    match payment.realized {
        Some(moved) => {
            add(&mut total.realized_gained, moved.max(0));
            add(&mut total.realized_lost, moved.min(0));
        }
        None => add(&mut total.realized_unknown, count),
    }
    if new_day {
        add(&mut total.days, 1);
    }
    add(&mut total.day_entries, if payment.upkeep { i128::from(new_day) } else { count });
    if newly_uncovered {
        add(&mut total.days_uncovered, 1);
    }
    if get(total.first_day).is_none_or(|first| day < first) {
        set(&mut total.first_day, day);
    }
    let last = get(total.last_day);
    if last.is_none_or(|last| day > last) {
        set(&mut total.last_day, day);
        set(&mut total.last_lag, lag);
        set(&mut total.last_hour, payment.hour);
        set(&mut total.last_sequence, payment.sequence);
        set(&mut total.last_ordinal, payment.ordinal);
        total.last_covered = Recorded::Value(payment.covered);
    } else if last == Some(day) {
        if !payment.covered {
            total.last_covered = Recorded::Value(false);
        }
        let (held_lag, held_hour) = (get(total.last_lag), get(total.last_hour));
        set(&mut total.last_lag, held_lag.map_or(lag, |held| held.max(lag)));
        set(&mut total.last_hour, held_hour.map_or(payment.hour, |held| held.min(payment.hour)));
    }
    Some((day, new_day, newly_uncovered))
}

pub(super) fn count(total: &mut Total, credit: bool, times: i64) {
    if times > 0 {
        add(if credit { &mut total.credits } else { &mut total.balances }, i128::from(times));
    }
}

pub(super) fn unknown(total: &mut Total, times: i64) {
    if times > 0 {
        add(&mut total.unknown, i128::from(times));
    }
}

pub(super) fn unmatched(total: &mut Total, times: i64) {
    if times > 0 {
        add(&mut total.unmatched, i128::from(times));
    }
}

pub(super) fn gain(total: &mut Total, (gained, capped, read): GainDay, sign: i128) {
    match gained {
        Some(gained) => add(&mut total.gained, sign * i128::from(gained)),
        None => add(&mut total.gain_unknown_days, sign),
    }
    add(&mut total.capped, sign * i128::from(capped));
    if read {
        add(&mut total.gain_days, sign);
    }
}

pub(super) fn day_gains(row: &CountryPowerDailyRow) -> [GainDay; 2] {
    let political = row.pp_gross_gain_raw.is_some() || row.pp_cost_raw.is_some() || row.pp_net_gain_raw.is_some() || row.pp_realized_delta_raw.is_some() || row.pp_sequence.is_some();
    let command = row.cp_gain_raw.is_some() || row.cp_realized_delta_raw.is_some() || row.cp_sequence.is_some();
    [
        gain_day(row.pp_gross_gain_raw, row.pp_net_gain_raw, row.pp_realized_delta_raw, political, false),
        gain_day(row.cp_gain_raw, row.cp_gain_raw, row.cp_realized_delta_raw, command, true),
    ]
}

fn gain_day(gain: Option<i64>, net: Option<i64>, applied: Option<i64>, read: bool, command: bool) -> GainDay {
    let gained = gain.filter(|_| read).map(|gain| if command { gain.max(0) } else { gain });
    let capped = match net.zip(applied) {
        Some((net, applied)) if read && net > 0 && applied < net => net - applied.max(0),
        _ => 0,
    };
    (gained, capped, read)
}
