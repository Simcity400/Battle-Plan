//! Tally contributions from the native collector, changed occurrences only.
use super::*;

#[derive(Default, Serialize, Deserialize)]
struct Contribution {
    points: Projection,
    baselines: BTreeMap<String, i32>,
    unresolved: usize,
    crossings: usize,
    error: Option<String>,
}
#[derive(Default, Serialize, Deserialize)]
pub(super) struct Channel {
    totals: BTreeMap<String, BTreeMap<String, Delta>>,
    #[serde(with = "dashboard_core::map_pairs")]
    pending: BTreeMap<(bool, u64), Contribution>,
    #[serde(with = "dashboard_core::map_pairs")]
    errors: BTreeMap<(bool, u64), String>,
    baselines: BTreeMap<String, BTreeMap<i32, usize>>,
    unresolved: usize,
    crossings: usize,
}
fn add(out: &mut Projection, points: &Projection, positive: bool) -> io::Result<()> {
    for (key, values) in points {
        for (point, &value) in values {
            out.entry(key.clone())
                .or_default()
                .entry(point.clone())
                .or_default()
                .checked_add(value, positive)?;
        }
    }
    Ok(())
}
fn contribution(
    scan: &sinkings::Scanned,
    change: &sinkings::Change,
    params: &NavalLossesParams,
    flag: &AtomicBool,
) -> io::Result<Contribution> {
    let possible = change.uncertainties(params.attribution == NavalAttribution::Lost);
    let empty = BTreeMap::new();
    let uncertainty = if change.ship.is_some() {
        (&possible, &empty)
    } else {
        (&empty, &possible)
    };
    let collected = collect_parts(
        scan,
        change.ship.iter(),
        change.convoy.iter(),
        Some(uncertainty),
        params,
        flag,
        vec![],
        BTreeMap::new(),
    )?;
    let mut points = projection(&collected.events, params.period);
    let mut baselines = BTreeMap::new();
    if params.period == NavalPeriod::Running {
        for (&identity, events) in &collected.events {
            if let Some(read) = events.baseline_unknown {
                let key = actor(identity);
                baselines.insert(key, read);
                add_projection(
                    &mut points,
                    identity,
                    read,
                    None,
                    Delta {
                        convoy_unknown: -1,
                        ..Default::default()
                    },
                );
            }
        }
    }
    Ok(Contribution {
        points,
        baselines,
        unresolved: collected.unresolved,
        crossings: collected.baseline_crossings,
        error: collected.error,
    })
}
impl Channel {
    fn apply(
        &mut self,
        scan: &sinkings::Scanned,
        change: &sinkings::Change,
        params: &NavalLossesParams,
        new: Contribution,
    ) -> io::Result<Projection> {
        let old = self.pending.remove(&change.key).unwrap_or_default();
        let mut delta = Projection::new();
        add(&mut delta, &old.points, false)?;
        add(&mut delta, &new.points, true)?;
        self.unresolved -= old.unresolved;
        self.unresolved += new.unresolved;
        self.crossings -= old.crossings;
        self.crossings += new.crossings;
        for (key, points) in &delta {
            for (point, &value) in points {
                if value == Delta::default() {
                    continue;
                }
                let (_, bucket): (i32, Option<i32>) = serde_json::from_str(point)?;
                if params.period == NavalPeriod::Month && bucket < month(scan.last_hour) {
                    continue;
                }
                self.totals
                    .entry(serde_json::to_string(&bucket)?)
                    .or_default()
                    .entry(key.clone())
                    .or_default()
                    .checked_add(value, true)?;
            }
        }
        let baseline_keys: BTreeSet<_> = old
            .baselines
            .keys()
            .chain(new.baselines.keys())
            .cloned()
            .collect();
        for key in baseline_keys {
            let reads = self.baselines.entry(key.clone()).or_default();
            let before = reads.first_key_value().map(|(&read, _)| read);
            if let Some(read) = old.baselines.get(&key) {
                let count = reads.get_mut(read).expect("pending baseline marker");
                *count -= 1;
                if *count == 0 {
                    reads.remove(read);
                }
            }
            if let Some(&read) = new.baselines.get(&key) {
                *reads.entry(read).or_default() += 1;
            }
            let after = reads.first_key_value().map(|(&read, _)| read);
            if before != after {
                for (read, value) in [(before, -1), (after, 1)] {
                    if let Some(read) = read {
                        delta
                            .entry(key.clone())
                            .or_default()
                            .entry(serde_json::to_string(&(read, None::<i32>))?)
                            .or_default()
                            .checked_add(
                                Delta {
                                    convoy_unknown: value,
                                    ..Default::default()
                                },
                                true,
                            )?;
                    }
                }
            }
        }
        if change.pending {
            self.pending.insert(change.key, new);
        }
        Ok(delta)
    }
    fn value(&self, key: &str, bucket: Option<i32>, running: bool) -> Delta {
        let mut value = self
            .totals
            .get(&serde_json::to_string(&bucket).expect("month key"))
            .and_then(|values| values.get(key))
            .copied()
            .unwrap_or_default();
        if running
            && self
                .baselines
                .get(key)
                .is_some_and(|reads| !reads.is_empty())
        {
            value.convoy_unknown += 1;
        }
        value
    }
    fn prune(&mut self, bucket: Option<i32>) {
        self.totals.retain(|key, _| {
            serde_json::from_str::<Option<i32>>(key)
                .expect("own month key")
                .is_none_or(|month| Some(month) >= bucket)
        });
    }
}

impl Prepared {
    pub fn observe_changes(
        &mut self,
        scan: &sinkings::Scanned,
        changes: &[sinkings::Change],
        holders: &[(String, Identity)],
        scope_changed: bool,
        retention: &sinkings::Retention,
        flag: &AtomicBool,
    ) -> io::Result<()> {
        let hour = scan.last_hour;
        let previous = self.last;
        let contiguous = previous.is_some_and(|last| last.checked_add(1) == Some(hour));
        let membership_changed = scope_changed || self.live.is_none();
        let mut removed = BTreeSet::new();
        if membership_changed {
            let mut live = BTreeMap::<Identity, Vec<String>>::new();
            for (tag, identity) in holders {
                live.entry(*identity).or_default().push(tag.clone());
            }
            removed.extend(
                self.live
                    .iter()
                    .flat_map(|old| old.keys())
                    .filter(|identity| !live.contains_key(*identity))
                    .copied(),
            );
            self.live = Some(live);
        }
        for &identity in &scan.new_candidates {
            let next = self.actors.len() as u64;
            self.actors.entry(actor(identity)).or_insert(next);
        }
        for tag in &scan.new_tags {
            let next = self.tags.len() as u64;
            self.tags.entry(tag.clone()).or_insert(next);
        }
        let live = self.live.as_ref().expect("configured holders");
        for index in 0..12 {
            check_cancel(flag)?;
            let params = selection(0, index);
            let state = &mut self.channels[index];
            state.values.advance(hour)?;
            for identity in &removed {
                let holder = self.actors[&actor(*identity)];
                for field in [0, 1, 2, 3, 6, 7, 8, 9] {
                    let key = field_key(holder, field);
                    if state.values.state(key).is_some() {
                        state.values.observe(key, hour, None, false);
                    }
                }
            }
            let month_changed = params.period == NavalPeriod::Month && state.month != month(hour);
            if month_changed {
                state.ledger.clear();
                state.month = month(hour);
                state.incremental.prune(month(hour));
            }
            let mut difference = Projection::new();
            // Collect all changed operands before arithmetic so a missing
            // country keeps the native collector's error precedence. This is
            // proportional to this frame's changes, never its closed history.
            let contributions: Vec<_> = changes
                .iter()
                .map(|change| contribution(scan, change, &params, flag))
                .collect::<io::Result<_>>()?;
            for (change, contribution) in changes.iter().zip(&contributions) {
                state.incremental.errors.remove(&change.key);
                if let Some(error) = &contribution.error {
                    state.incremental.errors.insert(change.key, error.clone());
                }
            }
            for (change, contribution) in changes.iter().zip(contributions) {
                let update = state
                    .incremental
                    .apply(scan, change, &params, contribution)
                    .and_then(|update| add(&mut difference, &update, true));
                update.map_err(|failure| {
                    state
                        .incremental
                        .errors
                        .first_key_value()
                        .map_or(failure, |(_, message)| io::Error::other(message.clone()))
                })?;
            }
            let mut affected = BTreeSet::new();
            for (key, points) in difference {
                let Some(&holder) = self.actors.get(&key) else {
                    continue;
                };
                for (point, delta) in points {
                    if delta == Delta::default() {
                        continue;
                    }
                    affected.insert(serde_json::from_str::<Identity>(&key)?);
                    let (read, bucket): (i32, Option<i32>) = serde_json::from_str(&point)?;
                    if read >= hour {
                        continue;
                    }
                    self.corrections.event(
                        holder * 12 + index as u64,
                        hour,
                        &Correction {
                            read_hour: read,
                            month: bucket,
                            delta,
                        },
                    )?;
                    self.corrected.insert(holder * 12 + index as u64);
                    if bucket.is_none() || bucket == month(hour) {
                        state
                            .ledger
                            .entry(key.clone())
                            .or_default()
                            .checked_add(delta, true)?;
                    }
                }
            }
            let first = baseline(scan, params.kind);
            let known_ship = retention.ships
                && known(
                    if params.period == NavalPeriod::Running {
                        first
                    } else {
                        scan.first_ship_read
                    },
                    &scan.ship_unread,
                    &retention.ship_loss,
                    hour,
                    params.period,
                );
            let known_convoy = retention.convoys
                && known(
                    if params.period == NavalPeriod::Running {
                        first
                    } else {
                        scan.first_convoy_read
                    },
                    &scan.convoy_unread,
                    &retention.convoy_loss,
                    hour,
                    params.period,
                );
            let changed_read = state
                .values
                .state(field_key(0, 4))
                .and_then(|state| state.last)
                .map(|run| run.value)
                != Some(i128::from(known_ship))
                || state
                    .values
                    .state(field_key(0, 5))
                    .and_then(|state| state.last)
                    .map(|run| run.value)
                    != Some(i128::from(known_convoy));
            let all = !contiguous || membership_changed || month_changed || changed_read;
            if all {
                if let Some(previous) = previous {
                    state.extend_ranks(previous);
                }
                state.rank_tail.clear();
                affected.extend(live.keys().copied());
            }
            if !contiguous || changed_read {
                state
                    .values
                    .observe(field_key(0, 4), hour, Some(i128::from(known_ship)), false);
                state
                    .values
                    .observe(field_key(0, 5), hour, Some(i128::from(known_convoy)), false);
            }
            for identity in affected {
                let Some(tags) = live.get(&identity) else {
                    continue;
                };
                let key = actor(identity);
                let holder = self.actors[&key];
                let value = state.incremental.value(
                    &key,
                    if params.period == NavalPeriod::Running {
                        None
                    } else {
                        month(hour)
                    },
                    params.period == NavalPeriod::Running,
                );
                state.write_holder(
                    &key,
                    holder,
                    hour,
                    params.kind,
                    (value.ships, value.convoys),
                    (value.ship_unknown, value.convoy_unknown),
                    (known_ship, known_convoy),
                )?;
                let read = |field: u64| {
                    state
                        .values
                        .state(field_key(holder, field))
                        .and_then(|state| state.last)
                        .map(|run| run.value)
                };
                let ships =
                    (known_ship && read(2).unwrap_or(0) == 0).then(|| read(0).unwrap_or(0) as i64);
                let convoys = (known_convoy && read(3).unwrap_or(0) == 0)
                    .then(|| read(1).unwrap_or(0) as i64);
                for tag in tags {
                    let starts = scan
                        .segments
                        .get(tag)
                        .and_then(|spans| spans.last())
                        .is_some_and(|span| span.first_hour == hour);
                    state.write_rank(
                        tag,
                        NavalLossRun {
                            first_hour: hour,
                            last_hour: hour,
                            starts_segment: starts,
                            ships,
                            convoys,
                        },
                        previous,
                    );
                }
            }
            state.error = state
                .incremental
                .errors
                .first_key_value()
                .map(|(_, error)| error.clone());
            state.warnings = tally_warnings(
                scan,
                retention.warnings.clone(),
                state.incremental.unresolved,
                state.incremental.crossings,
            );
        }
        if scan.last_ship_read == Some(hour) {
            self.reads.event(0, hour, &hour)?;
        }
        if scan.last_convoy_read == Some(hour) {
            self.reads.event(1, hour, &hour)?;
        }
        self.reads.through(hour);
        self.corrections.through(hour);
        self.last = Some(hour);
        Ok(())
    }
}

#[cfg(test)]
mod tests;
