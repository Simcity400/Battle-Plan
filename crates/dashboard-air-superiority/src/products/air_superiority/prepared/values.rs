//! Numeric daily inputs do not revise when later groups change historical
//! side classification. Retain those inputs by completed recording block;
//! classify them against the current groups on every reply.
use super::*;
use dashboard_base::prepared::{prepass::HeldBytes, query::observations};

type Input = (scan::CountryScores, scan::Auxiliary);
type Days = BTreeMap<i32, Arc<Input>>;

/// Hours a [`Sweep`] reads together; the changes it holds grow with keys times this.
const SWEEP_HOURS: usize = 64;

#[allow(clippy::too_many_arguments)]
pub(super) fn samples(
    source: &Source,
    key: &store::SourceKey,
    branch: u64,
    accepted: Option<i32>,
    daily: bool,
    wanted: &[i32],
    fields: &[(Field, u64)],
    hours: &mut Reader<'_>,
    values: &mut Reader<'_>,
    events: &mut Reader<'_>,
    cancelled: &AtomicBool,
) -> io::Result<Days> {
    let cache = observations::scope(source, "air-superiority:daily-values", Some(key));
    let keys: Vec<_> = fields.iter().map(|(_, key)| *key).collect();
    let block_of = |hour: i32| {
        daily
            .then(|| {
                source.replay_ref().index.iter().find(|block| {
                    block.branch == branch
                        && block.first_hour <= hour
                        && hour <= block.last_hour
                        && accepted.is_some_and(|accepted| block.last_hour <= accepted)
                        && hour.rem_euclid(DAY_HOURS) == DAY_HOURS - 1
                })
            })
            .flatten()
    };
    let direct: Vec<i32> = wanted
        .iter()
        .copied()
        .filter(|&hour| block_of(hour).is_none())
        .collect();
    let mut sweep = Sweep::new(&direct);
    let mut result = Days::new();
    let mut blocks = BTreeMap::new();
    for &hour in wanted {
        check_cancel(cancelled)?;
        let Some(block) = block_of(hour) else {
            let input = read_at(fields, &keys, values, events, &mut sweep, hour)?;
            result.insert(hour, Arc::new(input));
            continue;
        };
        if let std::collections::btree_map::Entry::Vacant(entry) = blocks.entry(block.first_hour) {
            // Only complete days are retained. A prefix's partial last day,
            // and a day ending in a gap, keep their own reached-hour read.
            let days = cache.get_query(
                block,
                &(),
                cancelled,
                |days: &Days| {
                    let mut bytes = HeldBytes::default();
                    bytes.value(days);
                    bytes.total() as usize
                },
                || {
                    let mut ends = Vec::new();
                    for run in hours.runs(0, block.first_hour, block.last_hour)? {
                        let first = i64::from(run.first_hour.div_euclid(DAY_HOURS))
                            * i64::from(DAY_HOURS)
                            + i64::from(DAY_HOURS - 1);
                        let mut hour = first;
                        while hour <= i64::from(run.last_hour) {
                            ends.push(hour as i32);
                            hour += i64::from(DAY_HOURS);
                        }
                    }
                    let mut sweep = Sweep::new(&ends);
                    let mut days = Days::new();
                    for &hour in &ends {
                        check_cancel(cancelled)?;
                        match read_at(fields, &keys, values, events, &mut sweep, hour) {
                            Ok(input) => {
                                days.insert(hour, Arc::new(input));
                            }
                            Err(error) if error.kind() == io::ErrorKind::Interrupted => {
                                return Err(error);
                            }
                            // An unrequested day's failure cannot refuse this
                            // viewport. Requested misses take their ordinary read.
                            Err(_) => {}
                        }
                    }
                    Ok(days)
                },
            );
            let days = match days {
                Ok(days) => days,
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock
                    ) =>
                {
                    return Err(error);
                }
                Err(_) => {
                    result.insert(hour, Arc::new(read(fields, &keys, values, events, hour)?));
                    continue;
                }
            };
            entry.insert(days);
        }
        let input = match blocks[&block.first_hour].get(&hour) {
            Some(input) => Arc::clone(input),
            None => Arc::new(read(fields, &keys, values, events, hour)?),
        };
        result.insert(hour, input);
    }
    Ok(result)
}

/// `values_at` over ascending hours, read a chunk at a time; an hour it can't answer falls back to `values_at`.
struct Sweep<'h> {
    hours: &'h [i32],
    chunk: std::ops::Range<usize>,
    /// The chunk's changes as (hour index within the chunk, key index, value), in order.
    changes: Vec<(usize, usize, Option<i128>)>,
    next: usize,
    current: Vec<Option<i128>>,
    usable: bool,
}

impl<'h> Sweep<'h> {
    fn new(hours: &'h [i32]) -> Self {
        Self {
            hours,
            chunk: 0..0,
            changes: Vec::new(),
            next: 0,
            current: Vec::new(),
            usable: hours.len() > 1 && hours.windows(2).all(|pair| pair[0] < pair[1]),
        }
    }

    /// The keys' amounts at `hour`, which must not precede the hour asked before.
    fn at(
        &mut self,
        values: &mut Reader<'_>,
        keys: &[u64],
        hour: i32,
    ) -> io::Result<Option<&[Option<i128>]>> {
        if !self.usable {
            return Ok(None);
        }
        let Ok(index) = self.hours.binary_search(&hour) else {
            return Ok(None);
        };
        if index < self.chunk.start {
            return Ok(None);
        }
        if index >= self.chunk.end {
            let chunk = index..(index + SWEEP_HOURS).min(self.hours.len());
            match values.changes_at(keys, &self.hours[chunk.clone()]) {
                Ok(read) => {
                    let mut changes: Vec<_> = read
                        .into_iter()
                        .enumerate()
                        .flat_map(|(key, changes)| {
                            changes.into_iter().map(move |(at, value)| (at, key, value))
                        })
                        .collect();
                    changes.sort_unstable_by_key(|&(at, key, _)| (at, key));
                    self.changes = changes;
                    self.next = 0;
                    self.current = vec![None; keys.len()];
                    self.chunk = chunk;
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => return Err(error),
                Err(_) => {
                    self.usable = false;
                    return Ok(None);
                }
            }
        }
        let at = index - self.chunk.start;
        while let Some(&(changed, key, value)) = self.changes.get(self.next) {
            if changed > at {
                break;
            }
            self.current[key] = value;
            self.next += 1;
        }
        Ok(Some(&self.current))
    }
}

/// One hour's input, its amounts from `sweep` where it answers the hour.
fn read_at(
    fields: &[(Field, u64)],
    keys: &[u64],
    values: &mut Reader<'_>,
    events: &mut Reader<'_>,
    sweep: &mut Sweep<'_>,
    hour: i32,
) -> io::Result<Input> {
    // The hour's operands fail before any value is read, as in `read`.
    let (mut projection, mut auxiliary) = operands(events, hour)?;
    match sweep.at(values, keys, hour)? {
        Some(amounts) => apply(fields, amounts, &mut projection, &mut auxiliary)?,
        None => apply(
            fields,
            &values.values_at(keys, hour)?,
            &mut projection,
            &mut auxiliary,
        )?,
    }
    Ok((projection, auxiliary))
}

fn read(
    fields: &[(Field, u64)],
    keys: &[u64],
    values: &mut Reader<'_>,
    events: &mut Reader<'_>,
    hour: i32,
) -> io::Result<Input> {
    let (mut projection, mut auxiliary) = operands(events, hour)?;
    // Consumers need amounts alone. Reconstructing every run's original
    // start and eventual end searches old and new batches for every field.
    apply(
        fields,
        &values.values_at(keys, hour)?,
        &mut projection,
        &mut auxiliary,
    )?;
    Ok((projection, auxiliary))
}

fn operands(events: &mut Reader<'_>, hour: i32) -> io::Result<Input> {
    let (_, operands) = events
        .latest_event(0, hour)?
        .ok_or_else(|| invalid("Prepared air score operands are missing"))?;
    let (_, auxiliary) = events
        .latest_event(1, hour)?
        .ok_or_else(|| invalid("Prepared air auxiliary operands are missing"))?;
    let projection = scan::CountryScores {
        operands: serde_json::from_value(operands)?,
        ..Default::default()
    };
    let mut auxiliary: scan::Auxiliary = serde_json::from_value(auxiliary)?;
    let (_, read) = events
        .latest_event(2, hour)?
        .ok_or_else(|| invalid("Prepared air theatre read is missing"))?;
    auxiliary.theatre_read = serde_json::from_value(read)?;
    Ok((projection, auxiliary))
}

fn apply(
    fields: &[(Field, u64)],
    amounts: &[Option<i128>],
    projection: &mut scan::CountryScores,
    auxiliary: &mut scan::Auxiliary,
) -> io::Result<()> {
    for ((field, _), value) in fields.iter().zip(amounts) {
        let Some(value) = *value else { continue };
        match field {
            Field::Friendly(tag, region) => {
                projection
                    .pairs
                    .entry((tag.clone(), *region))
                    .or_default()
                    .0 = i64::try_from(value)
                    .map_err(|_| invalid("Prepared friendly score is invalid"))?;
            }
            Field::Enemy(tag, region) => {
                projection
                    .pairs
                    .entry((tag.clone(), *region))
                    .or_default()
                    .1 =
                    i64::try_from(value).map_err(|_| invalid("Prepared enemy score is invalid"))?;
            }
            Field::Wings(region) if value > 0 => {
                auxiliary.wings.insert(
                    *region,
                    u32::try_from(value)
                        .map_err(|_| invalid("Prepared air wing count is invalid"))?,
                );
            }
            Field::Active(region) if value != 0 => {
                projection.active.insert(*region);
            }
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use dashboard_base::prepared::series::{Accumulator, tests::Fixture};

    #[test]
    fn a_sweep_reads_what_values_at_reads_in_chunks_across_batches_and_gaps() {
        let mut fixture = Fixture::new();
        let flag = AtomicBool::new(false);
        let mut amounts = Accumulator::default();
        for (batch, (first, last)) in [(0, 89), (90, 149), (170, 239)].into_iter().enumerate() {
            for hour in first..=last {
                for key in 0..12_u64 {
                    let amount = (key != 11 && (hour + key as i32) % 7 != 0).then(|| {
                        if key % 3 == 0 {
                            i128::from(hour / 10) * if key % 2 == 0 { 1 } else { -1 }
                        } else {
                            i128::from(key) << 70
                        }
                    });
                    amounts.observe(key, hour, amount, false);
                }
            }
            let mut writer = if batch == 2 {
                fixture.append_writer()
            } else {
                fixture.writer()
            };
            amounts.flush(&mut writer, "values", 0, &flag).unwrap();
            writer.commit().unwrap();
            amounts = serde_json::from_slice(&serde_json::to_vec(&amounts).unwrap()).unwrap();
        }
        let keys: Vec<u64> = (0..12).rev().chain([3, 0]).collect();
        let hours: Vec<i32> = (-5..250).collect();
        assert!(hours.len() > 2 * SWEEP_HOURS);
        let mut swept = Reader::new(&fixture.store, &fixture.key, "values".into(), &flag).unwrap();
        let mut alone = Reader::new(&fixture.store, &fixture.key, "values".into(), &flag).unwrap();
        let mut sweep = Sweep::new(&hours);
        for &hour in &hours {
            let read = sweep
                .at(&mut swept, &keys, hour)
                .unwrap()
                .map(<[_]>::to_vec);
            assert_eq!(
                read,
                Some(alone.values_at(&keys, hour).unwrap()),
                "hour {hour}"
            );
        }
        // Earlier, absent or unordered hours are left to `values_at`.
        assert_eq!(sweep.at(&mut swept, &keys, hours[3]).unwrap(), None);
        assert_eq!(sweep.at(&mut swept, &keys, 0).unwrap(), None);
        assert_eq!(
            Sweep::new(&[5, 2, 9]).at(&mut swept, &keys, 5).unwrap(),
            None
        );
        assert_eq!(Sweep::new(&[5]).at(&mut swept, &keys, 5).unwrap(), None);
    }

    #[test]
    fn an_hours_missing_operands_fail_before_its_values_are_read() {
        let fixture = Fixture::new();
        let flag = AtomicBool::new(false);
        let (mut amounts, mut operands) = (Accumulator::default(), Accumulator::default());
        for hour in 10..12 {
            amounts.observe(0, hour, Some(1), false);
            // Only the theatre read: no score operands at any hour.
            operands.event(2, hour, &serde_json::json!(null)).unwrap();
        }
        let mut writer = fixture.writer();
        amounts.flush(&mut writer, "values", 0, &flag).unwrap();
        operands.flush(&mut writer, "events", 0, &flag).unwrap();
        writer.commit().unwrap();
        let reading = AtomicBool::new(false);
        let mut values =
            Reader::new(&fixture.store, &fixture.key, "values".into(), &reading).unwrap();
        let mut events = Reader::new(&fixture.store, &fixture.key, "events".into(), &flag).unwrap();
        // A values read now stops as cancelled; the operands are read first.
        reading.store(true, std::sync::atomic::Ordering::Relaxed);
        let hours = [10, 11];
        let mut sweep = Sweep::new(&hours);
        let error = read_at(&[], &[0], &mut values, &mut events, &mut sweep, 10)
            .err()
            .unwrap();
        assert_ne!(error.kind(), io::ErrorKind::Interrupted);
        assert_eq!(error.to_string(), "Prepared air score operands are missing");
    }
}
