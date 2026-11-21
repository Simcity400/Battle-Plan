
use replay_data::rows::{CountryPowerDailyRow, PowerSpendTotalRow as Total, Recorded};

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

pub(super) fn pay(total: &mut Total, payment: &Payment) {
    if payment.recurring { day(total, payment) } else { once(total, payment) }
}

fn once(total: &mut Total, payment: &Payment) {
    add(&mut total.known, payment.charge.unwrap_or(0));
    add(&mut total.payments, 1);
    add(&mut total.entries, 1);
    add(&mut total.unknown, i128::from(payment.charge.is_none()));
    add(&mut total.uncovered, i128::from(!payment.covered));
    let latest = (payment.hour, payment.sequence, payment.ordinal);
    let held = get(total.latest_hour).zip(get(total.latest_sequence)).zip(get(total.latest_ordinal)).map(|((hour, sequence), ordinal)| (hour, sequence, ordinal));
    if held.is_none_or(|held| latest >= held) {
        set(&mut total.latest_hour, payment.hour);
        set(&mut total.latest_sequence, payment.sequence);
        set(&mut total.latest_ordinal, payment.ordinal);
        set(&mut total.latest_read, payment.read);
    }
}

fn day(total: &mut Total, payment: &Payment) {
    let day = midnight(payment.hour);
    let lag = payment.read.saturating_sub(day);
    add(&mut total.day_payments, 1);
    add(&mut total.day_charge, payment.charge.unwrap_or(0));
    add(&mut total.day_unknown, i128::from(payment.charge.is_none()));
    match payment.realized {
        Some(moved) => {
            add(&mut total.realized_gained, moved.max(0));
            add(&mut total.realized_lost, moved.min(0));
        }
        None => add(&mut total.realized_unknown, 1),
    }
    let last = get(total.last_day);
    if last.is_none_or(|last| day > last) {
        add(&mut total.days, 1);
        add(&mut total.day_entries, 1);
        add(&mut total.days_uncovered, i128::from(!payment.covered));
        if get(total.first_day).is_none_or(|first| day < first) {
            set(&mut total.first_day, day);
        }
        set(&mut total.last_day, day);
        set(&mut total.last_lag, lag);
        set(&mut total.last_hour, payment.hour);
        set(&mut total.last_sequence, payment.sequence);
        set(&mut total.last_ordinal, payment.ordinal);
        total.last_covered = Recorded::Value(payment.covered);
        return;
    }
    add(&mut total.day_entries, i128::from(!payment.upkeep));
    if last == Some(day) {
        if total.last_covered == Recorded::Value(true) && !payment.covered {
            add(&mut total.days_uncovered, 1);
            total.last_covered = Recorded::Value(false);
        }
        let (held_lag, held_hour) = (get(total.last_lag), get(total.last_hour));
        set(&mut total.last_lag, held_lag.map_or(lag, |held| held.max(lag)));
        set(&mut total.last_hour, held_hour.map_or(payment.hour, |held| held.min(payment.hour)));
        return;
    }
    add(&mut total.days_uncovered, i128::from(!payment.covered));
    if get(total.first_day).is_none_or(|first| day < first) {
        add(&mut total.days, 1);
        set(&mut total.first_day, day);
    }
}

pub(super) fn count(total: &mut Total, credit: bool) {
    add(if credit { &mut total.credits } else { &mut total.balances }, 1);
}

pub(super) fn unmatched(total: &mut Total) {
    add(&mut total.unmatched, 1);
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
