use replay_data::rows::CountryPowerDailyRow;

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
    pub hour: i32,
    pub sequence: u64,
    pub ordinal: u32,
    pub charge: Option<i128>,
    pub covered: bool,
    pub realized: Option<i128>,
    pub count: i64,
}

pub(super) type GainDay = (Option<i64>, i64, bool);

pub(super) fn midnight(hour: i32) -> i32 {
    hour.div_euclid(24) * 24
}

pub(super) fn count_day(seen: &mut ItemDays, payment: &Payment) -> Option<(i32, bool, bool)> {
    if payment.count <= 0 {
        return None;
    }
    let day = midnight(payment.hour);
    let new_day = seen.counted.insert(day);
    let newly_uncovered = !payment.covered && seen.uncovered.insert(day);
    Some((day, new_day, newly_uncovered))
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
