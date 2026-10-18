//! Cumulative values between changes of captured integer trade rates.
use super::*;

#[derive(Clone, Copy)]
pub(super) struct Rate {
    first: i64,
    amounts: [i128; 6],
    continuous: bool,
}

/// Hourly publication charges, independent of deferred rows and rate-cache memory.
/// Only the last endpoint, whether it is held, and whether this batch already
/// has its tail row affect the charge for an unchanged-rate span.
#[derive(Clone, Copy, Default)]
struct Channel {
    last: Option<i32>,
    active: bool,
    pending: bool,
}

impl Channel {
    fn extend(&mut self, through: i32) -> usize {
        if self.last.is_none_or(|last| last >= through) {
            return 0;
        }
        let rows = usize::from(!self.pending);
        self.last = Some(through);
        self.pending = true;
        rows
    }

    fn span(&mut self, rate: Rate, through: i32, amount: i128) -> usize {
        if rate.first > i64::from(through) {
            return 0;
        }
        if amount == 0 && rate.continuous && self.last.is_some() {
            return 0; // The hourly observer skips every point of this span.
        }
        let first = rate.first as i32;
        let mut rows = 0;
        // cumulative() pushes a constant prefix before observing the point.
        if self
            .last
            .is_some_and(|last| i64::from(last) + 1 < rate.first)
        {
            rows += self.extend(first - 1);
            self.active = false;
        }
        if self.active {
            rows += self.extend(first.saturating_sub(1));
        }
        let adjacent = self.last.and_then(|last| last.checked_add(1)) == Some(first);
        rows += usize::from(amount != 0 || !self.pending || !adjacent);
        self.last = Some(first);
        self.active = true;
        self.pending = true;
        if amount != 0 {
            // Each remaining checked addition changes the value, hence adds
            // exactly one pending Run. Zero rates skip those observations.
            rows += (i64::from(through) - rate.first) as usize;
            self.last = Some(through);
        }
        rows
    }
}

#[derive(Default)]
pub(super) struct Publication {
    channels: BTreeMap<u64, [Channel; 6]>,
    pub(super) bytes: usize,
    ready: bool,
    pub(super) continuous: bool,
}

impl Publication {
    pub(super) fn initialize(&mut self, keys: &BTreeMap<String, u64>, totals: &[Accumulator; 6]) {
        if self.ready {
            return;
        }
        for (index, values) in totals.iter().enumerate() {
            for &key in keys.values() {
                self.channels.entry(key).or_default()[index] = Channel {
                    last: values
                        .state(key)
                        .and_then(|state| state.last)
                        .map(|run| run.last_hour),
                    active: false,
                    pending: !values.pending_runs(key).is_empty(),
                };
            }
            for key in values.active() {
                self.channels.entry(key).or_default()[index].active = true;
            }
        }
        self.bytes = totals.iter().map(Accumulator::bytes).sum();
        self.ready = true;
    }

    pub(super) fn gap(&mut self, through: i32) {
        for channels in self.channels.values_mut() {
            for channel in channels {
                if channel.active {
                    self.bytes += channel.extend(through) * size_of::<series::Run>() * 2;
                }
                channel.active = false;
            }
        }
    }

    pub(super) fn pending(&self, key: u64, rate: Rate, through: i32) -> usize {
        let mut channels = self.channels.get(&key).copied().unwrap_or_default();
        charge(&mut channels, rate, through)
    }

    fn materialize(&mut self, key: u64, rate: Rate, through: i32) {
        self.bytes += charge(self.channels.entry(key).or_default(), rate, through);
    }
}

fn charge(channels: &mut [Channel; 6], rate: Rate, through: i32) -> usize {
    channels
        .iter_mut()
        .zip(rate.amounts)
        .map(|(channel, amount)| channel.span(rate, through, amount))
        .sum::<usize>()
        * size_of::<series::Run>()
        * 2
}

impl Prepared {
    pub(super) fn change_rate(
        &mut self,
        index: usize,
        hour: i32,
        amounts: Option<[i128; 6]>,
    ) -> io::Result<()> {
        #[cfg(test)]
        if self.hourly_integrals {
            return Ok(());
        }
        if self
            .rates
            .get(&index)
            .is_some_and(|rate| Some(rate.amounts) == amounts)
        {
            return Ok(());
        }
        if let Some(previous) = hour.checked_sub(1) {
            self.materialize_flow(index, previous, false)?;
        }
        match amounts {
            Some(amounts) => {
                self.rates.insert(
                    index,
                    Rate {
                        first: i64::from(hour),
                        amounts,
                        continuous: self.publication.continuous,
                    },
                );
            }
            None => {
                self.rates.remove(&index);
            }
        }
        Ok(())
    }

    pub(super) fn materialize_integrals(&mut self, hour: i32) -> io::Result<()> {
        let _cpu = ::replay_frame::cpu::scope_phase("trade:integrals");
        let indices: Vec<_> = self.rates.keys().copied().collect();
        for index in indices {
            self.materialize_flow(index, hour, true)?;
        }
        Ok(())
    }

    fn materialize_flow(&mut self, index: usize, through: i32, endpoint: bool) -> io::Result<()> {
        let Some(rate) = self
            .rates
            .get(&index)
            .copied()
            .filter(|rate| rate.first <= i64::from(through))
        else {
            return Ok(());
        };
        let _cpu = ::replay_frame::cpu::scope_phase("trade:integrals");
        #[cfg(test)]
        {
            self.integral_visits += 1;
        }
        let key = self.flow_ids[index];
        self.publication.materialize(key, rate, through);
        for (values, amount) in self.totals.iter_mut().zip(rate.amounts) {
            integrate(values, key, rate.first as i32, through, amount, endpoint)?;
        }
        let rate = self.rates.get_mut(&index).expect("materialized rate");
        rate.first = i64::from(through) + 1;
        rate.continuous = true;
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn observe_integrals_hourly(
        &mut self,
        hour: i32,
        continuous: bool,
    ) -> io::Result<()> {
        for &index in &self.scan.active {
            self.integral_visits += 1;
            let id = flow_id(
                &mut self.keys,
                &mut self.flow_ids,
                &self.scan.result.flows,
                index,
            );
            let run = self.scan.result.flows[index]
                .runs
                .last()
                .expect("current native trade terms");
            for (values, amount) in self
                .totals
                .iter_mut()
                .zip(sample::amounts(run.terms).into_iter().chain([1]))
            {
                if amount == 0 && continuous && values.state(id).is_some() {
                    continue;
                }
                let before = values
                    .state(id)
                    .and_then(|state| state.last)
                    .map_or(0, |run| run.value);
                let total = before.checked_add(amount).ok_or_else(|| {
                    dashboard_core::invalid("Trade integral exceeds its integer range")
                })?;
                cumulative(values, id, hour, total);
            }
        }
        Ok(())
    }
}

fn integrate(
    values: &mut Accumulator,
    key: u64,
    first: i32,
    last: i32,
    rate: i128,
    endpoint: bool,
) -> io::Result<()> {
    let previous = values.state(key).and_then(|state| state.last);
    let base = previous.map_or(0, |run| run.value);
    let starts = previous.is_none_or(|run| run.last_hour >= first);
    if let Some(previous) = previous
        && let Some(start) = previous
            .last_hour
            .checked_add(1)
            .filter(|start| *start < first)
    {
        values.push(key, series::Run::new(start, first - 1, base, false));
    }
    let hours = i128::from(i64::from(last) - i64::from(first) + 1);
    let total = rate
        .checked_mul(hours)
        .and_then(|delta| base.checked_add(delta));
    let end = i64::from(last) - i64::from(u8::from(endpoint));
    if let Some(total) = total {
        if i64::from(first) <= end {
            if rate == 0 {
                values.push(key, series::Run::new(first, end as i32, base, starts));
            } else {
                // Captured rates are already integers. Each prefix is base +
                // rate * accepted hours, exactly the hourly checked sum. The
                // saved staircase still has one point per changing hour.
                values.push_runs(
                    key,
                    (i64::from(first)..=end).map(|hour| {
                        let value = base + rate * i128::from(hour - i64::from(first) + 1);
                        series::Run::new(
                            hour as i32,
                            hour as i32,
                            value,
                            starts && hour == i64::from(first),
                        )
                    }),
                );
            }
        }
        if endpoint {
            cumulative(values, key, last, total);
        }
    } else {
        // A continuation near the integer limit may overflow the product even
        // when cancellation keeps each hourly prefix representable.
        let mut total = base;
        for hour in i64::from(first)..=i64::from(last) {
            total = total.checked_add(rate).ok_or_else(|| {
                dashboard_core::invalid("Trade integral exceeds its integer range")
            })?;
            if endpoint && hour == i64::from(last) {
                cumulative(values, key, last, total);
            } else {
                values.push(
                    key,
                    series::Run::new(
                        hour as i32,
                        hour as i32,
                        total,
                        starts && hour == i64::from(first),
                    ),
                );
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use dashboard_base::prepared::series::tests::{Fixture, assert_payloads_equal};

    #[test]
    fn integer_rate_prefixes_match_hourly_addition_including_product_overflow_cancellation() {
        let flag = AtomicBool::new(false);
        for (base, rate, first, last) in [
            (0, -137, 1, 29),
            (0, 32769, 1, 37),
            (59, 0, 5, 41),
            (0, i128::from(i64::MAX), 1, 41),
            (i128::MIN, i128::MAX, 1, 2),
        ] {
            let fixtures = [Fixture::new(), Fixture::new()];
            let mut states: [Accumulator; 2] = Default::default();
            for (index, values) in states.iter_mut().enumerate() {
                values.advance(0).unwrap();
                values.observe(0, 0, Some(base), true);
                if index == 0 {
                    values.advance_span(first, last).unwrap();
                    integrate(values, 0, first, last, rate, true).unwrap();
                } else {
                    let mut total = base;
                    for hour in first..=last {
                        values.advance(hour).unwrap();
                        total = total.checked_add(rate).unwrap();
                        cumulative(values, 0, hour, total);
                    }
                }
                let mut writer = fixtures[index].writer();
                values
                    .flush(&mut writer, "integral-prefix", 5, &flag)
                    .unwrap();
                writer.commit().unwrap();
            }
            assert_payloads_equal(
                &fixtures[0].payloads(),
                &fixtures[1].payloads(),
                "integer integral prefixes",
            );
            assert_eq!(
                serde_json::to_vec(&states[0]).unwrap(),
                serde_json::to_vec(&states[1]).unwrap()
            );
        }
        let mut values = Accumulator::default();
        values.advance_span(1, 2).unwrap();
        assert_eq!(
            integrate(&mut values, 0, 1, 2, i128::MAX, true)
                .unwrap_err()
                .to_string(),
            "Trade integral exceeds its integer range"
        );
    }
}
