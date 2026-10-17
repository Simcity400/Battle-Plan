//! Numeric daily inputs do not revise when later groups change historical
//! side classification. Retain those inputs by completed recording block;
//! classify them against the current groups on every reply.
use super::*;
use dashboard_base::prepared::{prepass::HeldBytes, query::observations};

type Input = (scan::CountryScores, scan::Auxiliary);
type Days = BTreeMap<i32, Arc<Input>>;

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
    let mut result = Days::new();
    let mut blocks = BTreeMap::new();
    for &hour in wanted {
        check_cancel(cancelled)?;
        let block = daily
            .then(|| {
                source.replay_ref().index.iter().find(|block| {
                    block.branch == branch
                        && block.first_hour <= hour
                        && hour <= block.last_hour
                        && accepted.is_some_and(|accepted| block.last_hour <= accepted)
                        && hour.rem_euclid(DAY_HOURS) == DAY_HOURS - 1
                })
            })
            .flatten();
        let Some(block) = block else {
            result.insert(hour, Arc::new(read(fields, &keys, values, events, hour)?));
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
                    let mut days = Days::new();
                    for run in hours.runs(0, block.first_hour, block.last_hour)? {
                        let first = i64::from(run.first_hour.div_euclid(DAY_HOURS))
                            * i64::from(DAY_HOURS)
                            + i64::from(DAY_HOURS - 1);
                        let mut hour = first;
                        while hour <= i64::from(run.last_hour) {
                            check_cancel(cancelled)?;
                            match read(fields, &keys, values, events, hour as i32) {
                                Ok(input) => {
                                    days.insert(hour as i32, Arc::new(input));
                                }
                                Err(error) if error.kind() == io::ErrorKind::Interrupted => {
                                    return Err(error);
                                }
                                // An unrequested day's failure cannot refuse this
                                // viewport. Requested misses take their ordinary read.
                                Err(_) => {}
                            }
                            hour += i64::from(DAY_HOURS);
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

fn read(
    fields: &[(Field, u64)],
    keys: &[u64],
    values: &mut Reader<'_>,
    events: &mut Reader<'_>,
    hour: i32,
) -> io::Result<Input> {
    let (_, operands) = events
        .latest_event(0, hour)?
        .ok_or_else(|| invalid("Prepared air score operands are missing"))?;
    let (_, auxiliary) = events
        .latest_event(1, hour)?
        .ok_or_else(|| invalid("Prepared air auxiliary operands are missing"))?;
    let mut projection = scan::CountryScores {
        operands: serde_json::from_value(operands)?,
        ..Default::default()
    };
    let mut auxiliary: scan::Auxiliary = serde_json::from_value(auxiliary)?;
    let (_, read) = events
        .latest_event(2, hour)?
        .ok_or_else(|| invalid("Prepared air theatre read is missing"))?;
    auxiliary.theatre_read = serde_json::from_value(read)?;
    // Consumers need amounts alone. Reconstructing every run's original
    // start and eventual end searches old and new batches for every field.
    for ((field, _), value) in fields.iter().zip(values.values_at(keys, hour)?) {
        let Some(value) = value else { continue };
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
    Ok((projection, auxiliary))
}
