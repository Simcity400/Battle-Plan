//! Numeric inputs are read in one pass over the requested hours. Each series batch's hours are
//! read together, so each key's blob is decoded once a batch, and each hour's projection is the
//! running one, changed only where that hour's amounts change.
use super::*;

type Input = (scan::CountryScores, scan::Auxiliary);

/// Hours one read of the amounts' changes covers at most: a whole batch of daily samples, whose
/// changes are held while they apply.
const SWEEP_HOURS: usize = 1024;

/// Each wanted hour's inputs in order, `visit`ed with the running projection and auxiliary
/// operands, which each hour changes only where its amounts and operands do.
pub(super) fn each_hour(
    wanted: &[i32],
    fields: &[(Field, u64)],
    values: &mut Reader<'_>,
    events: &mut Reader<'_>,
    cancelled: &AtomicBool,
    mut visit: impl FnMut(i32, &scan::CountryScores, &scan::Auxiliary) -> io::Result<()>,
) -> io::Result<()> {
    let keys: Vec<_> = fields.iter().map(|(_, key)| *key).collect();
    let mut running = Running::new(fields);
    let mut sweep = Sweep::new(wanted, keys.len());
    let mut held = Operands::default();
    for &hour in wanted {
        check_cancel(cancelled)?;
        // The hour's operands fail before any value is read.
        let (scores, auxiliary) = held.at(events, hour)?;
        running.operands(scores, auxiliary);
        sweep.at(values, &keys, hour)?;
        // Changed amounts apply in field order, so an hour's first invalid amount refuses it.
        for &key in &sweep.changed {
            running.set(key, sweep.current[key])?;
        }
        visit(hour, &running.projection, &running.auxiliary)?;
    }
    Ok(())
}

/// What each field's amount sets in an hour's inputs.
enum Role {
    /// A pair's friendly score, by its index in [`Running::pairs`].
    Friendly(usize),
    Enemy(usize),
    Wings(u32),
    Active(u32),
}

/// The inputs as of the hour last visited, kept from hour to hour.
struct Running {
    roles: Vec<Role>,
    /// Each country-region pair's key and its friendly and enemy scores, as amounts left them.
    pairs: Vec<((String, u32), Option<i64>, Option<i64>)>,
    projection: scan::CountryScores,
    auxiliary: scan::Auxiliary,
}

impl Running {
    fn new(fields: &[(Field, u64)]) -> Self {
        let mut pairs = Vec::new();
        let mut index = BTreeMap::new();
        let mut pair = |tag: &String, region: u32| {
            *index.entry((tag.clone(), region)).or_insert_with(|| {
                pairs.push(((tag.clone(), region), None, None));
                pairs.len() - 1
            })
        };
        let roles = fields
            .iter()
            .map(|(field, _)| match field {
                Field::Friendly(tag, region) => Role::Friendly(pair(tag, *region)),
                Field::Enemy(tag, region) => Role::Enemy(pair(tag, *region)),
                Field::Wings(region) => Role::Wings(*region),
                Field::Active(region) => Role::Active(*region),
            })
            .collect();
        Self {
            roles,
            pairs,
            projection: scan::CountryScores::default(),
            auxiliary: scan::Auxiliary::default(),
        }
    }

    /// The hour's operands, keeping the amounts' pairs, regions and wings.
    fn operands(&mut self, scores: scan::CountryScores, auxiliary: scan::Auxiliary) {
        self.projection.operands = scores.operands;
        let wings = std::mem::take(&mut self.auxiliary.wings);
        self.auxiliary = auxiliary;
        self.auxiliary.wings = wings;
    }

    /// Field `key`'s amount, now `value`.
    fn set(&mut self, key: usize, value: Option<i128>) -> io::Result<()> {
        let (pair, friendly) = match self.roles[key] {
            Role::Friendly(pair) => (pair, true),
            Role::Enemy(pair) => (pair, false),
            Role::Wings(region) => {
                match value.filter(|&value| value > 0) {
                    Some(value) => {
                        let count = u32::try_from(value)
                            .map_err(|_| invalid("Prepared air wing count is invalid"))?;
                        self.auxiliary.wings.insert(region, count);
                    }
                    None => {
                        self.auxiliary.wings.remove(&region);
                    }
                }
                return Ok(());
            }
            Role::Active(region) => {
                if value.is_some_and(|value| value != 0) {
                    self.projection.active.insert(region);
                } else {
                    self.projection.active.remove(&region);
                }
                return Ok(());
            }
        };
        let score = value.map(i64::try_from).transpose().map_err(|_| {
            invalid(if friendly {
                "Prepared friendly score is invalid"
            } else {
                "Prepared enemy score is invalid"
            })
        })?;
        let (id, held_friendly, held_enemy) = &mut self.pairs[pair];
        if friendly {
            *held_friendly = score;
        } else {
            *held_enemy = score;
        }
        // A pair holds what either score holds, the other side as zero.
        let scores = match (*held_friendly, *held_enemy) {
            (None, None) => None,
            (friendly, enemy) => Some((friendly.unwrap_or(0), enemy.unwrap_or(0))),
        };
        let pairs = &mut self.projection.pairs;
        match (scores, pairs.get_mut(id)) {
            (Some(scores), Some(held)) => *held = scores,
            (Some(scores), None) => {
                pairs.insert(id.clone(), scores);
            }
            (None, _) => {
                pairs.remove(id);
            }
        }
        Ok(())
    }
}

/// Every key's amount at ascending hours, read a series batch of hours at a time, and the keys
/// whose amounts changed at the hour last asked. An hour it cannot answer from its read is read
/// alone with `values_at`.
struct Sweep<'h> {
    hours: &'h [i32],
    chunk: std::ops::Range<usize>,
    /// The chunk's changes as (hour index within the chunk, key index, value), in order.
    changes: Vec<(usize, usize, Option<i128>)>,
    next: usize,
    /// Every key's amount at the hour last asked.
    current: Vec<Option<i128>>,
    /// The keys whose amounts that hour changed, in key order.
    changed: Vec<usize>,
    usable: bool,
}

impl<'h> Sweep<'h> {
    fn new(hours: &'h [i32], keys: usize) -> Self {
        Self {
            hours,
            chunk: 0..0,
            changes: Vec::new(),
            next: 0,
            current: vec![None; keys],
            changed: Vec::new(),
            usable: hours.windows(2).all(|pair| pair[0] < pair[1]),
        }
    }

    /// The keys' amounts at `hour`, which must not precede the hour asked before.
    fn at(&mut self, values: &mut Reader<'_>, keys: &[u64], hour: i32) -> io::Result<()> {
        self.changed.clear();
        let index = self
            .hours
            .binary_search(&hour)
            .ok()
            .filter(|&index| self.usable && index >= self.chunk.start);
        let Some(index) = index else {
            let amounts = values.values_at(keys, hour)?;
            self.hold(amounts);
            return Ok(());
        };
        if index >= self.chunk.end {
            match self.read(values, keys, index) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::Interrupted => return Err(error),
                Err(_) => {
                    self.usable = false;
                    return self.at(values, keys, hour);
                }
            }
            // The chunk's first hour holds what its first changes give, every other key none.
            let mut amounts = vec![None; keys.len()];
            while let Some(&(0, key, value)) = self.changes.get(self.next) {
                amounts[key] = value;
                self.next += 1;
            }
            self.hold(amounts);
            return Ok(());
        }
        let at = index - self.chunk.start;
        while let Some(&(changed, key, value)) = self.changes.get(self.next) {
            if changed > at {
                break;
            }
            if self.current[key] != value {
                self.current[key] = value;
                self.changed.push(key);
            }
            self.next += 1;
        }
        Ok(())
    }

    /// Every key's amount, now `amounts`.
    fn hold(&mut self, amounts: Vec<Option<i128>>) {
        for (key, amount) in amounts.into_iter().enumerate() {
            if self.current[key] != amount {
                self.current[key] = amount;
                self.changed.push(key);
            }
        }
    }

    /// Reads the changes of the hours from `index` through the end of the series batch answering
    /// it, at most [`SWEEP_HOURS`] of them.
    fn read(&mut self, values: &mut Reader<'_>, keys: &[u64], index: usize) -> io::Result<()> {
        let mut end = (index + SWEEP_HOURS).min(self.hours.len());
        if let Some(through) = values.batch_through(self.hours[index])? {
            end = index + 1 + self.hours[index + 1..end].partition_point(|&hour| hour <= through);
        }
        let chunk = index..end;
        let read = values.changes_at(keys, &self.hours[chunk.clone()])?;
        let mut changes: Vec<_> = read
            .into_iter()
            .enumerate()
            .flat_map(|(key, changes)| changes.into_iter().map(move |(at, value)| (at, key, value)))
            .collect();
        changes.sort_unstable_by_key(|&(at, key, _)| (at, key));
        self.changes = changes;
        self.next = 0;
        self.chunk = chunk;
        Ok(())
    }
}

/// The operand events last parsed, each with the count of its key's events through the hour it
/// was read for: later hours with no newer event of a key share its parse.
#[derive(Default)]
struct Operands {
    scores: Option<(u64, scan::ScoreOperands)>,
    auxiliary: Option<(u64, scan::Auxiliary)>,
    theatre_read: Option<(u64, Option<i32>)>,
}

/// A key's latest event through an hour: the one already parsed, or its row to parse.
enum Latest {
    Parsed,
    Row(u64, serde_json::Value),
}

impl Operands {
    /// The hour's operands. Each event is taken, then parsed, in the same order as ever, so an
    /// hour fails with the same error.
    fn at(&mut self, events: &mut Reader<'_>, hour: i32) -> io::Result<Input> {
        let scores = latest(
            &self.scores,
            events,
            0,
            hour,
            "Prepared air score operands are missing",
        )?;
        let auxiliary = latest(
            &self.auxiliary,
            events,
            1,
            hour,
            "Prepared air auxiliary operands are missing",
        )?;
        if let Latest::Row(count, row) = scores {
            self.scores = Some((count, serde_json::from_value(row)?));
        }
        if let Latest::Row(count, row) = auxiliary {
            self.auxiliary = Some((count, serde_json::from_value(row)?));
        }
        if let Latest::Row(count, row) = latest(
            &self.theatre_read,
            events,
            2,
            hour,
            "Prepared air theatre read is missing",
        )? {
            self.theatre_read = Some((count, serde_json::from_value(row)?));
        }
        let (Some((_, scores)), Some((_, auxiliary)), Some((_, read))) =
            (&self.scores, &self.auxiliary, &self.theatre_read)
        else {
            unreachable!("every operand was parsed above");
        };
        let projection = scan::CountryScores {
            operands: scores.clone(),
            ..Default::default()
        };
        let mut auxiliary = auxiliary.clone();
        auxiliary.theatre_read = *read;
        Ok((projection, auxiliary))
    }
}

fn latest<T>(
    parsed: &Option<(u64, T)>,
    events: &mut Reader<'_>,
    key: u64,
    hour: i32,
    missing: &'static str,
) -> io::Result<Latest> {
    let count = events.event_count_through(key, hour)?;
    if parsed.as_ref().is_some_and(|(parsed, _)| *parsed == count) {
        return Ok(Latest::Parsed);
    }
    let (_, row) = events
        .latest_event(key, hour)?
        .ok_or_else(|| invalid(missing))?;
    Ok(Latest::Row(count, row))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dashboard_base::prepared::series::{Accumulator, tests::Fixture};

    /// Amounts across three batches, the last appended after a gap, for `keys` keys.
    fn batches(fixture: &mut Fixture, keys: u64) {
        let flag = AtomicBool::new(false);
        let mut amounts = Accumulator::default();
        for (batch, (first, last)) in [(0, 89), (90, 149), (170, 239)].into_iter().enumerate() {
            for hour in first..=last {
                for key in 0..keys {
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
    }

    /// Asks `sweep` for `hour`, checking every key's amount against `values_at` and the
    /// changed keys against the amounts asked before.
    fn ask(
        sweep: &mut Sweep<'_>,
        swept: &mut Reader<'_>,
        alone: &mut Reader<'_>,
        keys: &[u64],
        hour: i32,
    ) {
        let previous = sweep.current.clone();
        sweep.at(swept, keys, hour).unwrap();
        let expected = alone.values_at(keys, hour).unwrap();
        assert_eq!(sweep.current, expected, "hour {hour}");
        let changed: Vec<usize> = (0..keys.len())
            .filter(|&key| previous[key] != expected[key])
            .collect();
        assert_eq!(sweep.changed, changed, "hour {hour}");
    }

    #[test]
    fn a_sweep_holds_what_values_at_reads_and_names_each_change_across_batches_and_gaps() {
        let mut fixture = Fixture::new();
        batches(&mut fixture, 12);
        let flag = AtomicBool::new(false);
        let keys: Vec<u64> = (0..12).rev().chain([3, 0]).collect();
        let open = || Reader::new(&fixture.store, &fixture.key, "values".into(), &flag).unwrap();
        let (mut swept, mut alone) = (open(), open());
        // A window ends with the batch answering its first hour (0 to 89, 90 to 149); one
        // starting where no batch answers, before the data or in the gap before the appended
        // batch, runs on to the end.
        for (first, starts) in [(0, vec![0, 90, 150]), (-5, vec![-5])] {
            let hours: Vec<i32> = (first..250).collect();
            let mut sweep = Sweep::new(&hours, keys.len());
            let mut read = BTreeSet::new();
            for &hour in &hours {
                ask(&mut sweep, &mut swept, &mut alone, &keys, hour);
                read.insert(hours[sweep.chunk.start]);
            }
            assert_eq!(read.into_iter().collect::<Vec<_>>(), starts);
            // Earlier and absent hours are read with `values_at`.
            ask(&mut sweep, &mut swept, &mut alone, &keys, hours[3]);
            ask(&mut sweep, &mut swept, &mut alone, &keys, 1000);
        }
        // So are unordered hours.
        let mut unordered = Sweep::new(&[5, 2, 9], keys.len());
        unordered.at(&mut swept, &keys, 5).unwrap();
        assert_eq!(unordered.current, alone.values_at(&keys, 5).unwrap());
    }

    #[test]
    fn a_sweep_applies_backdated_runs_as_point_reads_do() {
        use dashboard_base::prepared::series::Run;

        let fixture = Fixture::new();
        let flag = AtomicBool::new(false);
        let mut amounts = Accumulator::default();
        for (key, runs) in [
            vec![(0, 2, 1), (5, 5, 2), (0, 10, 3)],
            vec![(0, 10, 8), (2, 3, 4)],
        ]
        .into_iter()
        .enumerate()
        {
            for (first, last, value) in runs {
                amounts.push(key as u64, Run::new(first, last, value, true));
            }
        }
        let mut writer = fixture.writer();
        amounts.flush(&mut writer, "values", 0, &flag).unwrap();
        writer.commit().unwrap();
        let open = || Reader::new(&fixture.store, &fixture.key, "values".into(), &flag).unwrap();
        let (mut swept, mut alone) = (open(), open());
        let keys = [0, 1, 0, 2];
        for hours in [(-1..=12).collect::<Vec<_>>(), vec![1, 3, 5, 10, 11]] {
            let mut sweep = Sweep::new(&hours, keys.len());
            for &hour in &hours {
                ask(&mut sweep, &mut swept, &mut alone, &keys, hour);
            }
        }
    }

    #[test]
    fn hours_between_operand_events_share_one_read_of_them() {
        let fixture = Fixture::new();
        let flag = AtomicBool::new(false);
        let mut operands = Accumulator::default();
        let scores = |tags: &[&str]| scan::ScoreOperands {
            identity_known: true,
            witnessed: Arc::new(tags.iter().map(|tag| (*tag).to_owned()).collect()),
            ..Default::default()
        };
        let named = scan::Auxiliary {
            theatre_names: Arc::new([("1".to_owned(), b"North".to_vec())].into()),
            ..Default::default()
        };
        let known = scan::Auxiliary {
            theatres_known: true,
            ..named.clone()
        };
        // Each key's events change at their own hours, as a walk writes them.
        operands.event(0, 10, &scores(&["AAA"])).unwrap();
        operands.event(1, 10, &named).unwrap();
        operands.event(2, 10, &None::<i32>).unwrap();
        operands.event(1, 25, &known).unwrap();
        operands.event(0, 30, &scores(&["AAA", "BBB"])).unwrap();
        operands.event(2, 40, &Some(40)).unwrap();
        let mut writer = fixture.writer();
        operands.flush(&mut writer, "events", 0, &flag).unwrap();
        writer.commit().unwrap();
        let mut events = Reader::new(&fixture.store, &fixture.key, "events".into(), &flag).unwrap();
        let mut held = Operands::default();
        let mut first = None;
        for hour in 5..=45 {
            let shared = held.at(&mut events, hour);
            let alone = Operands::default().at(&mut events, hour);
            let (shared, alone) = match (shared, alone) {
                (Ok(shared), Ok(alone)) => (shared, alone),
                (Err(shared), Err(alone)) => {
                    assert_eq!(shared.to_string(), alone.to_string(), "hour {hour}");
                    continue;
                }
                _ => panic!("hour {hour} read differently"),
            };
            assert!(shared.0.operands == alone.0.operands, "hour {hour}");
            assert!(shared.1 == alone.1, "hour {hour}");
            let witnessed = Arc::clone(&shared.0.operands.witnessed);
            let first = first.get_or_insert(witnessed.clone());
            // Until the score operands change at hour 30, every hour holds the first read.
            assert_eq!(Arc::ptr_eq(first, &witnessed), hour < 30, "hour {hour}");
        }
        assert!(first.is_some());
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
        let fields = [(Field::Wings(1), 0)];
        let error = each_hour(
            &[10, 11],
            &fields,
            &mut values,
            &mut events,
            &flag,
            |_, _, _| panic!("no hour has its operands"),
        )
        .unwrap_err();
        assert_ne!(error.kind(), io::ErrorKind::Interrupted);
        assert_eq!(error.to_string(), "Prepared air score operands are missing");
    }

    /// An hour's inputs as reading its amounts whole gives them: every field with an amount
    /// applied in field order.
    fn whole(
        fields: &[(Field, u64)],
        values: &mut Reader<'_>,
        events: &mut Reader<'_>,
        hour: i32,
    ) -> io::Result<Input> {
        let (mut projection, mut auxiliary) = Operands::default().at(events, hour)?;
        let keys: Vec<_> = fields.iter().map(|(_, key)| *key).collect();
        for ((field, _), value) in fields.iter().zip(values.values_at(&keys, hour)?) {
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
                        .1 = i64::try_from(value)
                        .map_err(|_| invalid("Prepared enemy score is invalid"))?;
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

    #[test]
    fn each_hour_visits_what_reading_each_hour_whole_gives() {
        let fields = vec![
            (Field::Friendly("AAA".into(), 1), 0),
            (Field::Enemy("AAA".into(), 1), 1),
            (Field::Friendly("BBB".into(), 2), 2),
            (Field::Wings(1), 3),
            (Field::Active(2), 4),
            (Field::Enemy("CCC".into(), 3), 5),
            (Field::Wings(2), 6),
        ];
        let fixture = Fixture::new();
        let flag = AtomicBool::new(false);
        let mut amounts = Accumulator::default();
        let mut operands = Accumulator::default();
        let scores = |tags: &[&str]| scan::ScoreOperands {
            identity_known: true,
            witnessed: Arc::new(tags.iter().map(|tag| (*tag).to_owned()).collect()),
            ..Default::default()
        };
        let mut writer = fixture.writer();
        operands.event(0, 0, &scores(&["AAA"])).unwrap();
        operands.event(1, 0, &scan::Auxiliary::default()).unwrap();
        operands.event(2, 0, &None::<i32>).unwrap();
        operands.event(0, 70, &scores(&["AAA", "BBB"])).unwrap();
        for hour in 0..120 {
            for key in 0..fields.len() as u64 {
                // Runs of differing lengths, gaps, zeros and negative wing counts.
                let phase = (hour / (3 + key as i32)) % 5;
                let amount = match phase {
                    0 => None,
                    1 => Some(0),
                    2 => Some(-i128::from(key as i32 + 1)),
                    _ => Some(i128::from(hour % 13 + key as i32)),
                };
                amounts.observe(key, hour, amount, false);
            }
        }
        amounts.flush(&mut writer, "values", 0, &flag).unwrap();
        operands.flush(&mut writer, "events", 0, &flag).unwrap();
        writer.commit().unwrap();
        let open = |name: &str| Reader::new(&fixture.store, &fixture.key, name.into(), &flag);
        let (mut values, mut events) = (open("values").unwrap(), open("events").unwrap());
        let (mut alone, mut alone_events) = (open("values").unwrap(), open("events").unwrap());
        for wanted in [(0..120).collect::<Vec<_>>(), (5..120).step_by(11).collect()] {
            let mut visited = Vec::new();
            each_hour(
                &wanted,
                &fields,
                &mut values,
                &mut events,
                &flag,
                |hour, projection, auxiliary| {
                    let (expected, expected_auxiliary) =
                        whole(&fields, &mut alone, &mut alone_events, hour)?;
                    assert_eq!(projection.pairs, expected.pairs, "hour {hour}");
                    assert_eq!(projection.active, expected.active, "hour {hour}");
                    assert!(projection.operands == expected.operands, "hour {hour}");
                    assert!(*auxiliary == expected_auxiliary, "hour {hour}");
                    visited.push(hour);
                    Ok(())
                },
            )
            .unwrap();
            assert_eq!(visited, wanted);
        }
    }

    #[test]
    fn each_hour_refuses_an_invalid_amount_at_the_first_hour_it_holds() {
        let fields = vec![
            (Field::Friendly("AAA".into(), 1), 0),
            (Field::Enemy("AAA".into(), 1), 1),
        ];
        let fixture = Fixture::new();
        let flag = AtomicBool::new(false);
        let (mut amounts, mut operands) = (Accumulator::default(), Accumulator::default());
        operands
            .event(0, 0, &scan::ScoreOperands::default())
            .unwrap();
        operands.event(1, 0, &scan::Auxiliary::default()).unwrap();
        operands.event(2, 0, &None::<i32>).unwrap();
        for hour in 0..20 {
            amounts.observe(0, hour, Some(1), false);
            let enemy = if hour >= 12 {
                i128::from(i64::MAX) + 1
            } else {
                2
            };
            amounts.observe(1, hour, Some(enemy), false);
        }
        let mut writer = fixture.writer();
        amounts.flush(&mut writer, "values", 0, &flag).unwrap();
        operands.flush(&mut writer, "events", 0, &flag).unwrap();
        writer.commit().unwrap();
        let open = |name: &str| Reader::new(&fixture.store, &fixture.key, name.into(), &flag);
        let (mut values, mut events) = (open("values").unwrap(), open("events").unwrap());
        let mut visited = Vec::new();
        let wanted: Vec<i32> = (0..20).collect();
        let error = each_hour(
            &wanted,
            &fields,
            &mut values,
            &mut events,
            &flag,
            |hour, _, _| {
                visited.push(hour);
                Ok(())
            },
        )
        .unwrap_err();
        assert_eq!(visited, (0..12).collect::<Vec<_>>());
        let (mut alone, mut alone_events) = (open("values").unwrap(), open("events").unwrap());
        let expected = whole(&fields, &mut alone, &mut alone_events, 12)
            .err()
            .unwrap();
        assert_eq!(error.to_string(), expected.to_string());
    }
}
