//! The original sinking attribution and numerical collection feeding shared series.
use super::*;
use crate::{
    metadata::check_cancel,
    prepared::series::{self, Accumulator, Reader, Run},
};
use serde_json::{Value, json};
mod incremental;

const FIELDS: u64 = 8;
// Read completeness belongs to the shared retained table, independently of holders.
fn field_key(holder: u64, field: u64) -> u64 {
    match field {
        4 => u64::MAX - 1,
        5 => u64::MAX,
        6 => holder * FIELDS + 4,
        7 => holder * FIELDS + 5,
        8 => holder * FIELDS + 6,
        9 => holder * FIELDS + 7,
        _ => holder * FIELDS + field,
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Delta {
    ships: i64,
    convoys: i64,
    ship_unknown: i64,
    convoy_unknown: i64,
}
impl Delta {
    fn add(&mut self, other: Self) {
        self.ships += other.ships;
        self.convoys += other.convoys;
        self.ship_unknown += other.ship_unknown;
        self.convoy_unknown += other.convoy_unknown;
    }
    fn checked_add(&mut self, other: Self, positive: bool) -> io::Result<()> {
        let sum = |a: i64, b: i64| {
            (if positive {
                a.checked_add(b)
            } else {
                a.checked_sub(b)
            })
            .ok_or_else(|| io::Error::other("Naval losses count exceeds i64 arithmetic"))
        };
        *self = Self {
            ships: sum(self.ships, other.ships)?,
            convoys: sum(self.convoys, other.convoys)?,
            ship_unknown: sum(self.ship_unknown, other.ship_unknown)?,
            convoy_unknown: sum(self.convoy_unknown, other.convoy_unknown)?,
        };
        Ok(())
    }
    #[cfg(test)]
    fn negative(self) -> Self {
        Self {
            ships: -self.ships,
            convoys: -self.convoys,
            ship_unknown: -self.ship_unknown,
            convoy_unknown: -self.convoy_unknown,
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
struct Correction {
    read_hour: i32,
    month: Option<i32>,
    delta: Delta,
}
type Projection = BTreeMap<String, BTreeMap<String, Delta>>;
#[derive(Default, Serialize, Deserialize)]
struct Channel {
    #[cfg(test)]
    #[serde(with = "crate::prepared_production::map_pairs")]
    seed: BTreeMap<Identity, Events>,
    #[cfg(test)]
    open: Projection,
    ledger: BTreeMap<String, Delta>,
    month: Option<i32>,
    #[cfg(test)]
    permanent_unresolved: usize,
    #[cfg(test)]
    crossings: usize,
    error: Option<String>,
    warnings: Vec<String>,
    values: Accumulator,
    ranks: Accumulator,
    rank_tail: BTreeMap<String, NavalLossRun>,
    incremental: incremental::Channel,
    #[serde(skip)]
    rank_inputs: BTreeMap<String, Vec<NavalLossRun>>,
}
#[derive(Default, Serialize, Deserialize)]
pub(crate) struct Prepared {
    actors: BTreeMap<String, u64>,
    tags: BTreeMap<String, u64>,
    channels: [Channel; 12],
    corrections: Accumulator,
    corrected: BTreeSet<u64>,
    reads: Accumulator,
    last: Option<i32>,
    #[serde(skip)]
    live: Option<BTreeMap<Identity, Vec<String>>>,
}
impl Channel {
    fn extend_ranks(&mut self, hour: i32) {
        for tag in self.rank_tail.keys().cloned().collect::<Vec<_>>() {
            self.extend_rank(&tag, hour);
        }
    }
    fn extend_rank(&mut self, tag: &str, hour: i32) {
        let tail = self.rank_tail.get_mut(tag).expect("current rank holder");
        if tail.last_hour >= hour {
            return;
        }
        let first = tail.last_hour + 1;
        let runs = self.rank_inputs.entry(tag.to_owned()).or_default();
        if let Some(last) = runs.last_mut().filter(|last| last.last_hour == first - 1) {
            last.last_hour = hour;
        } else {
            runs.push(NavalLossRun {
                first_hour: first,
                last_hour: hour,
                starts_segment: false,
                ..tail.clone()
            });
        }
        tail.last_hour = hour;
    }
    fn write_rank(&mut self, tag: &str, run: NavalLossRun, previous: Option<i32>) {
        if let Some(tail) = self.rank_tail.get(tag) {
            let to = previous
                .unwrap_or(tail.last_hour)
                .min(run.first_hour.saturating_sub(1));
            self.extend_rank(tag, to);
        }
        self.rank_tail.insert(tag.to_owned(), run.clone());
        let runs = self.rank_inputs.entry(tag.to_owned()).or_default();
        if let Some(previous) = runs.last_mut()
            && !run.starts_segment
            && previous.last_hour.checked_add(1) == Some(run.first_hour)
            && previous.ships == run.ships
            && previous.convoys == run.convoys
        {
            previous.last_hour = run.last_hour;
        } else {
            runs.push(run);
        }
    }
    #[allow(clippy::too_many_arguments)]
    fn write_holder(
        &mut self,
        key: &str,
        holder: u64,
        hour: i32,
        kind: NavalKind,
        amounts: (i64, i64),
        missing: (i64, i64),
        complete: (bool, bool),
    ) -> io::Result<()> {
        #[cfg(test)]
        sinkings::work::WORK.with(|work| work.borrow_mut().values += 1);
        let (ships, convoys) = amounts;
        let (ship_unknown, convoy_unknown) = missing;
        let (known_ship, known_convoy) = complete;
        let adjustment = self.ledger.get(key).copied().unwrap_or_default();
        let base_ship = ships - adjustment.ships;
        let base_convoy = convoys - adjustment.convoys;
        let raw = [
            i128::from(base_ship),
            i128::from(base_convoy),
            i128::from(ship_unknown - adjustment.ship_unknown),
            i128::from(convoy_unknown - adjustment.convoy_unknown),
            i128::from(known_ship),
            i128::from(known_convoy),
            i128::from(match kind {
                NavalKind::Ships => base_ship,
                NavalKind::Convoys => base_convoy,
                NavalKind::Both => base_ship
                    .checked_add(base_convoy)
                    .ok_or_else(|| crate::invalid("Naval loss total overflow"))?,
            }),
        ];
        let base_ship_known = known_ship && ship_unknown - adjustment.ship_unknown == 0;
        let base_convoy_known = known_convoy && convoy_unknown - adjustment.convoy_unknown == 0;
        let display_known = match kind {
            NavalKind::Ships => base_ship_known,
            NavalKind::Convoys => base_convoy_known,
            NavalKind::Both => base_ship_known && base_convoy_known,
        };
        self.values.observe(
            field_key(holder, 7),
            hour,
            display_known.then_some(raw[6]),
            false,
        );
        if kind == NavalKind::Both {
            for (field, value) in [(8, raw[0]), (9, raw[1])] {
                let key = field_key(holder, field);
                if value != 0 || self.values.state(key).is_some() {
                    self.values
                        .observe(key, hour, display_known.then_some(value), false);
                }
            }
        }
        for (field, value) in raw.into_iter().enumerate() {
            if matches!(field, 4 | 5) {
                continue;
            }
            let key = field_key(holder, field as u64);
            if field == 6 || value != 0 || self.values.state(key).is_some() {
                self.values.observe(key, hour, Some(value), false);
            }
        }
        Ok(())
    }
}
#[derive(Serialize, Deserialize)]
struct Metadata {
    scan: sinkings::Scanned,
    tags: BTreeMap<String, u64>,
    corrected: BTreeSet<u64>,
    errors: Vec<Option<String>>,
    warnings: Vec<Vec<String>>,
}
fn selection(branch: u64, channel: usize) -> NavalLossesParams {
    NavalLossesParams {
        branch_id: branch.to_string(),
        tags: vec![],
        top: None,
        kind: [NavalKind::Ships, NavalKind::Convoys, NavalKind::Both][channel / 4],
        attribution: [NavalAttribution::Lost, NavalAttribution::SunkBy][channel / 2 % 2],
        period: [NavalPeriod::Running, NavalPeriod::Month][channel % 2],
    }
}
fn channel(params: &NavalLossesParams) -> usize {
    (match params.kind {
        NavalKind::Ships => 0,
        NavalKind::Convoys => 1,
        NavalKind::Both => 2,
    }) * 4
        + usize::from(params.attribution == NavalAttribution::SunkBy) * 2
        + usize::from(params.period == NavalPeriod::Month)
}
fn actor(identity: Identity) -> String {
    serde_json::to_string(&identity).expect("identity")
}
fn add_projection(
    projection: &mut Projection,
    identity: Identity,
    read: i32,
    month: Option<i32>,
    delta: Delta,
) {
    projection
        .entry(actor(identity))
        .or_default()
        .entry(serde_json::to_string(&(read, month)).expect("event date"))
        .or_default()
        .add(delta);
}
/// Convert the original collector's prefix points back to their signed changes.
fn projection(events: &BTreeMap<Identity, Events>, period: NavalPeriod) -> Projection {
    let mut out = Projection::new();
    for (&identity, events) in events {
        let mut points = |points: &[(i32, i64)], month: Option<i32>, ship: bool| {
            let mut previous = 0;
            for &(read, value) in points {
                let amount = value - previous;
                previous = value;
                add_projection(
                    &mut out,
                    identity,
                    read,
                    month,
                    if ship {
                        Delta {
                            ships: amount,
                            ..Default::default()
                        }
                    } else {
                        Delta {
                            convoys: amount,
                            ..Default::default()
                        }
                    },
                );
            }
        };
        if period == NavalPeriod::Running {
            points(&events.ships, None, true);
            points(&events.convoy_running, None, false);
            for &read in &events.unknown_ships {
                add_projection(
                    &mut out,
                    identity,
                    read,
                    None,
                    Delta {
                        ship_unknown: 1,
                        ..Default::default()
                    },
                );
            }
            for &read in &events.convoy_running_unknown {
                add_projection(
                    &mut out,
                    identity,
                    read,
                    None,
                    Delta {
                        convoy_unknown: 1,
                        ..Default::default()
                    },
                );
            }
            if let Some(read) = events.baseline_unknown {
                add_projection(
                    &mut out,
                    identity,
                    read,
                    None,
                    Delta {
                        convoy_unknown: 1,
                        ..Default::default()
                    },
                );
            }
        } else {
            for (&month, values) in &events.ship_months {
                points(values, Some(month), true);
            }
            for (&month, values) in &events.convoy_months {
                points(values, Some(month), false);
            }
            for &(month, read) in &events.ship_unknown_points {
                add_projection(
                    &mut out,
                    identity,
                    read,
                    Some(month),
                    Delta {
                        ship_unknown: 1,
                        ..Default::default()
                    },
                );
            }
            for &(month, read) in &events.convoy_unknown_points {
                add_projection(
                    &mut out,
                    identity,
                    read,
                    Some(month),
                    Delta {
                        convoy_unknown: 1,
                        ..Default::default()
                    },
                );
            }
        }
    }
    out
}
#[cfg(test)]
fn unknown(events: &Events, params: &NavalLossesParams, hour: i32) -> (i64, i64) {
    if params.period == NavalPeriod::Running {
        (
            events
                .unknown_ships
                .iter()
                .filter(|&&read| read <= hour)
                .count() as i64,
            events
                .convoy_running_unknown
                .iter()
                .filter(|&&read| read <= hour)
                .count() as i64
                + i64::from(events.baseline_unknown.is_some_and(|read| read <= hour)),
        )
    } else {
        let bucket = month(hour);
        (
            events
                .ship_unknown_points
                .iter()
                .filter(|&&(m, read)| Some(m) == bucket && read <= hour)
                .count() as i64,
            events
                .convoy_unknown_points
                .iter()
                .filter(|&&(m, read)| Some(m) == bucket && read <= hour)
                .count() as i64,
        )
    }
}
#[cfg(test)]
fn compact(events: &mut Events, hour: i32) {
    fn endpoint(points: &mut Vec<(i32, i64)>, hour: i32) {
        let value = at(points, hour);
        points.clear();
        if value != 0 {
            points.push((hour, value));
        }
    }
    endpoint(&mut events.ships, hour);
    endpoint(&mut events.convoy_running, hour);
    for months in [&mut events.ship_months, &mut events.convoy_months] {
        months.retain(|&m, _| month(hour).is_some_and(|current| m >= current));
        for points in months.values_mut() {
            endpoint(points, hour);
        }
    }
    events.convoys.clear();
    events.unknown_ships.clear();
    events.unknown_ship_months.clear();
    events.unknown_convoy_months.clear();
    events.ship_unknown_points.clear();
    events.convoy_unknown_points.clear();
    events.convoy_running_unknown.clear();
    events.unknown_convoys = events.baseline_unknown;
}
/// Closed occurrences affect only their own holders. Both paths use the
/// oracle's prefix arithmetic; inactive historical holders are never revisited.
#[cfg(test)]
fn merge_closed(previous: &mut Events, added: &Events, hour: i32) -> io::Result<()> {
    fn merge(points: &mut Vec<(i32, i64)>, added: &[(i32, i64)]) -> io::Result<()> {
        let mut last = 0;
        for &(read, value) in added {
            points.push((
                read,
                value
                    .checked_sub(last)
                    .ok_or_else(|| crate::invalid("Naval losses count exceeds i64 arithmetic"))?,
            ));
            last = value;
        }
        prefix(points)
    }
    merge(&mut previous.ships, &added.ships)?;
    merge(&mut previous.convoy_running, &added.convoy_running)?;
    for (&month, points) in &added.ship_months {
        merge(previous.ship_months.entry(month).or_default(), points)?;
    }
    for (&month, points) in &added.convoy_months {
        merge(previous.convoy_months.entry(month).or_default(), points)?;
    }
    previous.baseline_unknown = match (previous.baseline_unknown, added.baseline_unknown) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    };
    compact(previous, hour);
    Ok(())
}
#[cfg(test)]
fn ship_open(event: &sinkings::ShipEvent) -> bool {
    (event.entry.victim_country != 0 && event.victim.is_none())
        || (event.entry.attacker_country != 0 && event.attacker.is_none())
}
#[cfg(test)]
fn convoy_open(event: &sinkings::ConvoyEvent) -> bool {
    (event.owner_raw != 0 && event.owner.is_none())
        || (event.killer_raw != 0 && event.killer.is_none())
}
impl Prepared {
    fn extend_ranks(&mut self) {
        let Some(hour) = self.last else {
            return;
        };
        for state in &mut self.channels {
            state.extend_ranks(hour);
        }
    }
    #[cfg(test)]
    pub(crate) fn observe(
        &mut self,
        scan: &sinkings::Scanned,
        retention: &sinkings::Retention,
        flag: &AtomicBool,
    ) -> io::Result<()> {
        self.extend_ranks();
        let previous = self.last;
        let hour = scan.last_hour;
        let tail = scan;
        let candidates: BTreeSet<_> = if self.actors.is_empty() {
            scan.country_candidates
                .values()
                .flatten()
                .copied()
                .collect()
        } else {
            scan.new_candidates.clone()
        };
        for identity in candidates {
            let key = actor(identity);
            let next = self.actors.len() as u64;
            self.actors.entry(key).or_insert(next);
        }
        let tags: Vec<_> = if self.tags.is_empty() {
            scan.segments.keys().cloned().collect()
        } else {
            scan.new_tags.iter().cloned().collect()
        };
        for tag in tags {
            let next = self.tags.len() as u64;
            self.tags.entry(tag).or_insert(next);
        }
        let active: BTreeSet<_> = scan
            .current_holders
            .iter()
            .map(|(_, identity)| *identity)
            .collect();
        for index in 0..12 {
            check_cancel(flag)?;
            let params = selection(0, index);
            let state = &mut self.channels[index];
            state.values.advance(hour)?;
            state.rank_tail.clear();
            if params.period == NavalPeriod::Month && state.month != month(hour) {
                state.ledger.clear();
                state.month = month(hour);
            }
            let additions = collect(tail, &params, flag, vec![], BTreeMap::new())?;
            let now = projection(&additions.events, params.period);
            let mut difference = now.clone();
            for (key, points) in &state.open {
                for (point, &delta) in points {
                    difference
                        .entry(key.clone())
                        .or_default()
                        .entry(point.clone())
                        .or_default()
                        .add(delta.negative());
                }
            }
            for (key, points) in difference {
                let Some(&holder) = self.actors.get(&key) else {
                    continue;
                };
                for (point, delta) in points {
                    let (read, bucket): (i32, Option<i32>) = serde_json::from_str(&point)?;
                    if read >= hour || delta == Delta::default() {
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
                        state.ledger.entry(key.clone()).or_default().add(delta);
                    }
                }
            }
            let tallies = collect(
                tail,
                &params,
                flag,
                retention.warnings.clone(),
                active
                    .iter()
                    .filter_map(|identity| {
                        state
                            .seed
                            .get(identity)
                            .map(|events| (*identity, events.clone()))
                    })
                    .collect(),
            )?;
            state.error = tallies.error;
            state.warnings = tally_warnings(
                scan,
                retention.warnings.clone(),
                state.permanent_unresolved + tallies.unresolved,
                state.crossings + tallies.baseline_crossings,
            );
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
            state
                .values
                .observe(field_key(0, 4), hour, Some(i128::from(known_ship)), false);
            state
                .values
                .observe(field_key(0, 5), hour, Some(i128::from(known_convoy)), false);
            for &identity in &active {
                let key = actor(identity);
                let holder = self.actors[&key];
                let empty = Events::default();
                let events = tallies.events.get(&identity).unwrap_or(&empty);
                let (ships, convoys) = amounts(events, scan, &params, hour);
                let (ship_unknown, convoy_unknown) = unknown(events, &params, hour);
                state.write_holder(
                    &key,
                    holder,
                    hour,
                    params.kind,
                    (ships, convoys),
                    (ship_unknown, convoy_unknown),
                    (known_ship, known_convoy),
                )?;
            }
            for (tag, _) in &scan.current_holders {
                let identity = scan.identity(tag, hour).filter(|_| {
                    scan.segments[tag]
                        .last()
                        .is_some_and(|span| span.last_hour == hour)
                });
                let value = identity.map(|identity| {
                    let holder = self.actors[&actor(identity)];
                    let read = |field: u64| {
                        state
                            .values
                            .state(field_key(holder, field))
                            .and_then(|state| state.last)
                            .filter(|run| run.last_hour == hour)
                            .map(|run| run.value)
                    };
                    let ships = (read(4) == Some(1) && read(2).unwrap_or(0) == 0)
                        .then(|| read(0).unwrap_or(0) as i64);
                    let convoys = (read(5) == Some(1) && read(3).unwrap_or(0) == 0)
                        .then(|| read(1).unwrap_or(0) as i64);
                    NavalLossRun {
                        first_hour: hour,
                        last_hour: hour,
                        starts_segment: scan.segments[tag]
                            .last()
                            .is_some_and(|span| span.first_hour == hour),
                        ships,
                        convoys,
                    }
                });
                if let Some(run) = value {
                    state.write_rank(tag, run, previous);
                }
            }
            let closed_values = collect_parts(
                tail,
                tail.ships.iter().filter(|event| !ship_open(event)),
                tail.convoys.iter().filter(|event| !convoy_open(event)),
                None,
                &params,
                flag,
                vec![],
                BTreeMap::new(),
            )?;
            state.permanent_unresolved += closed_values.unresolved;
            state.crossings += closed_values.baseline_crossings;
            for (identity, events) in closed_values.events {
                merge_closed(state.seed.entry(identity).or_default(), &events, hour)?;
            }
            state.open = projection(
                &collect_parts(
                    tail,
                    tail.ships.iter().filter(|event| ship_open(event)),
                    tail.convoys.iter().filter(|event| convoy_open(event)),
                    Some(uncertainties(tail, &params)),
                    &params,
                    flag,
                    vec![],
                    BTreeMap::new(),
                )?
                .events,
                params.period,
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
    pub(crate) fn flush(
        &mut self,
        writer: &mut store::Writer,
        branch: u64,
        scan: &sinkings::Scanned,
        flag: &AtomicBool,
    ) -> io::Result<()> {
        self.extend_ranks();
        for (index, state) in self.channels.iter_mut().enumerate() {
            let params = selection(branch, index);
            crate::ranking::rank_positions(
                &state.rank_inputs,
                |run| (run.first_hour, run.last_hour),
                |run| {
                    (match params.kind {
                        NavalKind::Ships => run.ships,
                        NavalKind::Convoys => run.convoys,
                        NavalKind::Both => run
                            .ships
                            .zip(run.convoys)
                            .and_then(|(s, c)| s.checked_add(c)),
                    })
                    .map(i128::from)
                },
                flag,
                |rank, tag, first, last| {
                    state.ranks.push(
                        rank as u64 * 2,
                        Run::new(first, last, i128::from(self.tags[tag]), false),
                    );
                    state.ranks.push(
                        self.tags[tag] * 2 + 1,
                        Run::new(first, last, rank as i128 + 1, false),
                    );
                    Ok(())
                },
            )?;
            if let Some(last) = self.last {
                state.ranks.through(last);
            }
            state.values.flush(
                writer,
                &format!("naval-loss-values-{branch}-{index}"),
                0,
                flag,
            )?;
            state.ranks.flush(
                writer,
                &format!("naval-loss-ranks-{branch}-{index}"),
                0,
                flag,
            )?;
            state.rank_inputs.clear();
        }
        self.corrections
            .flush(writer, &format!("naval-loss-corrections-{branch}"), 0, flag)?;
        self.reads
            .flush(writer, &format!("naval-loss-reads-{branch}"), 0, flag)?;
        series::write_catalogue(
            writer,
            "naval-losses",
            branch,
            &self.actors,
            &Metadata {
                scan: scan.metadata(),
                tags: self.tags.clone(),
                corrected: self.corrected.clone(),
                errors: self
                    .channels
                    .iter()
                    .map(|channel| channel.error.clone())
                    .collect(),
                warnings: self
                    .channels
                    .iter()
                    .map(|channel| channel.warnings.clone())
                    .collect(),
            },
        )
    }
}

fn corrections(
    reader: &mut Reader<'_>,
    metadata: &Metadata,
    holder: u64,
    index: usize,
) -> io::Result<Vec<Correction>> {
    let key = holder * 12 + index as u64;
    if !metadata.corrected.contains(&key) {
        return Ok(Vec::new());
    }
    reader
        .events(&[key], i32::MIN, i32::MAX, 0, u32::MAX)?
        .rows
        .into_iter()
        .map(|row| serde_json::from_value(row.row).map_err(Into::into))
        .collect()
}
fn adjustment(rows: &[Correction], hour: i32, period: NavalPeriod) -> Delta {
    let mut value = Delta::default();
    for row in rows {
        if row.read_hour <= hour && (period == NavalPeriod::Running || row.month == month(hour)) {
            value.add(row.delta);
        }
    }
    value
}
fn exact(
    reader: &mut Reader<'_>,
    holder: u64,
    hour: i32,
    corrections: &[Correction],
    params: &NavalLossesParams,
) -> io::Result<(Option<i64>, Option<i64>)> {
    let mut raw = [0_i128; 6];
    for (field, value) in raw.iter_mut().enumerate() {
        *value = reader
            .value_at(field_key(holder, field as u64), hour)?
            .map_or(0, |run| run.value);
    }
    let correction = adjustment(corrections, hour, params.period);
    let ships = (raw[4] != 0 && raw[2] + i128::from(correction.ship_unknown) == 0)
        .then_some(raw[0] + i128::from(correction.ships));
    let convoys = (raw[5] != 0 && raw[3] + i128::from(correction.convoy_unknown) == 0)
        .then_some(raw[1] + i128::from(correction.convoys));
    let total = match params.kind {
        NavalKind::Ships => ships,
        NavalKind::Convoys => convoys,
        NavalKind::Both => ships.zip(convoys).map(|(s, c)| s + c),
    };
    if [ships, convoys, total].into_iter().flatten().any(|amount| {
        !(-i128::from(JS_MAX_SAFE_INTEGER)..=i128::from(JS_MAX_SAFE_INTEGER)).contains(&amount)
    }) {
        return Err(crate::invalid(
            "Naval losses count exceeds JavaScript MAX_SAFE_INTEGER",
        ));
    }
    Ok((ships.map(|n| n as i64), convoys.map(|n| n as i64)))
}
fn exact_runs(
    reader: &mut Reader<'_>,
    holder: u64,
    range: (i32, i32),
    corrections: &[Correction],
    params: &NavalLossesParams,
    starts: bool,
) -> io::Result<Vec<NavalLossRun>> {
    let (first, last) = range;
    let mut bounds = BTreeSet::from([i64::from(first), i64::from(last) + 1]);
    for field in 0..6 {
        for run in reader.runs(field_key(holder, field), first, last)? {
            bounds.insert(i64::from(run.first_hour).max(i64::from(first)));
            bounds.insert((i64::from(run.last_hour) + 1).min(i64::from(last) + 1));
        }
    }
    for correction in corrections {
        if correction.read_hour >= first && correction.read_hour <= last {
            bounds.insert(i64::from(correction.read_hour));
        }
    }
    if params.period == NavalPeriod::Month
        && let (Some(mut bucket), Some(end)) = (month(first), month(last))
    {
        while bucket <= end {
            if let Some(hour) = month_start(bucket)
                && hour >= first
                && hour <= last
            {
                bounds.insert(i64::from(hour));
            }
            bucket += 1;
        }
    }
    let bounds: Vec<_> = bounds.into_iter().collect();
    let mut runs: Vec<NavalLossRun> = Vec::new();
    for pair in bounds.windows(2) {
        let (ships, convoys) = exact(reader, holder, pair[0] as i32, corrections, params)?;
        let begins = starts && pair[0] == i64::from(first);
        if let Some(previous) = runs.last_mut()
            && !begins
            && previous.ships == ships
            && previous.convoys == convoys
        {
            previous.last_hour = (pair[1] - 1) as i32;
        } else {
            runs.push(NavalLossRun {
                first_hour: pair[0] as i32,
                last_hour: (pair[1] - 1) as i32,
                starts_segment: begins,
                ships,
                convoys,
            });
        }
    }
    Ok(runs)
}
fn candidates(
    ranks: &mut Reader<'_>,
    actors: &BTreeMap<String, u64>,
    metadata: &Metadata,
    params: &NavalLossesParams,
    range: (i32, i32),
) -> io::Result<BTreeSet<String>> {
    let Some(top) = params.top else {
        return Ok(params.tags.iter().cloned().collect());
    };
    let inverse: BTreeMap<_, _> = metadata
        .tags
        .iter()
        .map(|(tag, &key)| (key, tag.clone()))
        .collect();
    let index = channel(params);
    let corrected: BTreeSet<_> = metadata
        .corrected
        .iter()
        .filter(|&&key| key % 12 == index as u64)
        .map(|&key| key / 12)
        .collect();
    let mut tags = BTreeSet::new();
    let positions = (top as usize)
        .saturating_add(corrected.len())
        .min(metadata.tags.len());
    for position in 0..positions {
        for run in ranks.runs(position as u64 * 2, range.0, range.1)? {
            if let Some(tag) = inverse.get(&(run.value as u64)) {
                tags.insert(tag.clone());
            }
        }
    }
    for (identity, &holder) in actors {
        if !corrected.contains(&holder) {
            continue;
        }
        let identity: Identity = serde_json::from_str(identity)?;
        for (tag, spans) in &metadata.scan.country_identities {
            if spans.iter().any(|span| {
                span.identity == identity && span.first_hour <= range.1 && span.last_hour >= range.0
            }) {
                tags.insert(tag.clone());
            }
        }
    }
    Ok(tags)
}
/// Reply construction, identity selection and Top-N clipping stay in the original product.
#[allow(clippy::too_many_arguments)]
fn window_reply(
    store: &store::Store,
    key: &store::SourceKey,
    actors: &BTreeMap<String, u64>,
    metadata: &Metadata,
    histories: &RecordedHistories,
    params: &NavalLossesParams,
    range: Option<(i32, i32)>,
    flag: &AtomicBool,
) -> io::Result<NavalLossesReply> {
    let branch: u64 = params.branch_id.parse().expect("validated branch");
    let index = channel(params);
    if let Some(error) = &metadata.errors[index] {
        return Err(crate::invalid(error));
    }
    let mut values = Reader::new(
        store,
        key,
        format!("naval-loss-values-{branch}-{index}"),
        flag,
    )?;
    let mut ranks = Reader::new(
        store,
        key,
        format!("naval-loss-ranks-{branch}-{index}"),
        flag,
    )?;
    let mut corrections_reader =
        Reader::new(store, key, format!("naval-loss-corrections-{branch}"), flag)?;
    let mut scan = metadata.scan.clone();
    let mut series = BTreeMap::new();
    let tags = if let Some(range) = range {
        if params.top.is_some() {
            candidates(&mut ranks, actors, metadata, params, range)?
        } else {
            country_history::selected_aliases(histories, &params.tags)
                .into_iter()
                .collect()
        }
    } else if params.top.is_some() {
        metadata.tags.keys().cloned().collect()
    } else {
        country_history::selected_aliases(histories, &params.tags)
            .into_iter()
            .collect()
    };
    let selected = country_history::selected_identities(histories, &params.tags);
    for tag in tags {
        check_cancel(flag)?;
        let mut runs = Vec::new();
        if let Some((first, last)) = range {
            for identity in histories
                .countries
                .get(&tag)
                .into_iter()
                .flat_map(|country| country.identities.iter())
            {
                if params.top.is_none() && !selected.contains(&(identity.slot, identity.country_id))
                {
                    continue;
                }
                let range = (first.max(identity.first_hour), last.min(identity.last_hour));
                if range.0 > range.1 {
                    continue;
                }
                let Some(&holder) = actors.get(&actor((identity.slot, identity.country_id))) else {
                    continue;
                };
                let corrections = corrections(&mut corrections_reader, metadata, holder, index)?;
                runs.extend(exact_runs(
                    &mut values,
                    holder,
                    range,
                    &corrections,
                    params,
                    range.0 == identity.first_hour,
                )?);
            }
        }
        series.insert(tag, runs);
    }
    if let Some((first, last)) = range {
        let mut reads = Reader::new(store, key, format!("naval-loss-reads-{branch}"), flag)?;
        for (id, out) in [(0, &mut scan.ship_reads), (1, &mut scan.convoy_reads)] {
            if let Some((hour, _)) = first
                .checked_sub(1)
                .map(|before| reads.latest_event(id, before))
                .transpose()?
                .flatten()
            {
                out.push(hour);
            }
            out.extend(
                reads
                    .events(&[id], first, last, 0, u32::MAX)?
                    .rows
                    .into_iter()
                    .map(|row| row.hour),
            );
        }
    }
    let mut metadata_selection = params.clone();
    if range.is_none() && params.top.is_some() {
        metadata_selection.top = None;
        metadata_selection.tags = histories.countries.keys().cloned().collect();
    }
    if range.is_none() {
        scan.ship_reads = [scan.first_ship_read, scan.last_ship_read]
            .into_iter()
            .flatten()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        scan.convoy_reads = [scan.first_convoy_read, scan.last_convoy_read]
            .into_iter()
            .flatten()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
    }
    project_reply(
        &scan,
        &metadata_selection,
        flag,
        metadata.warnings[index].clone(),
        Some(histories),
        series,
    )
}
pub(crate) fn query(
    source: &Source,
    product: &str,
    params: &Value,
    context: &LoadContext,
    store: &store::Store,
    key: &store::SourceKey,
    flag: &AtomicBool,
) -> io::Result<Value> {
    let selection: NavalLossesParams = if product == "naval-losses" {
        let mut selection = params.clone();
        selection
            .as_object_mut()
            .ok_or_else(|| crate::invalid("Invalid naval loss selection"))?
            .remove("window");
        selection.as_object_mut().unwrap().remove("hour");
        serde_json::from_value(selection)?
    } else {
        serde_json::from_value(
            json!({ "branchId":params["branchId"],"tags":params["tags"],"top":params["family"]["top"],
        "kind":params["family"]["navalKind"],"attribution":params["family"]["attribution"],"period":params["family"]["period"] }),
        )?
    };
    NavalLosses::validate(&selection).map_err(crate::invalid)?;
    let branch = selection.branch_id.parse().expect("validated branch");
    let Some((actors, metadata)) =
        series::read_catalogue::<Metadata>(store, key, "naval-losses", branch)?
    else {
        return Ok(Value::Null);
    };
    let Some(histories) =
        country_history::prepared_branch_histories(source, branch, context.content.as_ref(), flag)?
    else {
        return Ok(Value::Null);
    };
    if product == "naval-losses" {
        let range = if let Some(window) = params["window"].as_u64() {
            let windows = source.block_windows(branch);
            let window = windows
                .get(window as usize)
                .ok_or_else(|| crate::invalid("Unknown naval loss window"))?;
            Some((window.first_hour, window.last_hour))
        } else if let Some(hour) = params["hour"].as_i64() {
            let hour =
                i32::try_from(hour).map_err(|_| crate::invalid("Invalid naval loss hour"))?;
            Some((hour, hour))
        } else {
            None
        };
        let reply = window_reply(
            store, key, &actors, &metadata, &histories, &selection, range, flag,
        )?;
        let mut reply = serde_json::to_value(reply)?;
        if range.is_none() {
            reply["windows"] = serde_json::to_value(
                source
                    .block_windows(branch)
                    .iter()
                    .map(|window| HourSpan {
                        first_hour: window.first_hour,
                        last_hour: window.last_hour,
                    })
                    .collect::<Vec<_>>(),
            )?;
        }
        Ok(reply)
    } else {
        series_query(
            product, params, &selection, &actors, &metadata, &histories, store, key, flag,
        )
    }
}

fn amount(run: &NavalLossRun, kind: NavalKind) -> Option<i64> {
    match kind {
        NavalKind::Ships => run.ships,
        NavalKind::Convoys => run.convoys,
        NavalKind::Both => run
            .ships
            .zip(run.convoys)
            .and_then(|(ships, convoys)| ships.checked_add(convoys)),
    }
}
/// Reached buckets use the shared numeric summary/merge, with exact original runs.
fn rebuilt_bucket(
    values: &mut Reader<'_>,
    holder: u64,
    range: (i32, i32),
    corrections: &[Correction],
    params: &NavalLossesParams,
    starts: bool,
    display: u64,
) -> io::Result<Option<crate::prepared_chart::ChartBucket>> {
    let runs = exact_runs(values, holder, range, corrections, params, starts)?;
    let mut bucket = None;
    let mut initialized = false;
    for run in runs {
        let count = amount(&run, params.kind).map(|total| match display {
            8 => run.ships.unwrap(),
            9 => run.convoys.unwrap(),
            _ => total,
        });
        let next = if let Some(amount) = count {
            let base = values.summary(
                field_key(holder, 6),
                (run.first_hour, run.last_hour),
                (f64::from(run.first_hour), f64::from(run.last_hour), 1),
                0,
            )?;
            let mut piece = None;
            for mut part in base.buckets.into_iter().flatten() {
                part.first_value = amount as f64;
                part.last_value = amount as f64;
                part.min_value = amount as f64;
                part.max_value = amount as f64;
                part.min_hour = part.first_hour;
                part.max_hour = part.first_hour;
                part.absolute_total = i128::from(amount).unsigned_abs()
                    * (i64::from(part.last_hour) - i64::from(part.first_hour) + 1) as u128;
                part.first_break |= run.starts_segment;
                piece = if piece.is_none() {
                    Some(part)
                } else {
                    crate::prepared_chart::merge(piece, Some(part))
                };
            }
            piece
        } else {
            None
        };
        bucket = if initialized {
            crate::prepared_chart::merge(bucket, next)
        } else {
            next
        };
        initialized = true;
    }
    Ok(bucket)
}
fn shifted(
    mut bucket: crate::prepared_chart::ChartBucket,
    delta: i64,
) -> io::Result<crate::prepared_chart::ChartBucket> {
    let old_positive = bucket.min_value >= 0.0;
    let shift = delta as f64;
    bucket.first_value += shift;
    bucket.last_value += shift;
    bucket.min_value += shift;
    bucket.max_value += shift;
    if bucket.min_value < -JS_MAX_SAFE_INTEGER as f64
        || bucket.max_value > JS_MAX_SAFE_INTEGER as f64
    {
        return Err(crate::invalid(
            "Naval losses count exceeds JavaScript MAX_SAFE_INTEGER",
        ));
    }
    let hours = i128::from(bucket.last_hour) - i128::from(bucket.first_hour) + 1;
    let adjustment = i128::from(delta) * hours;
    let signed = if old_positive {
        bucket.absolute_total as i128
    } else {
        -(bucket.absolute_total as i128)
    };
    let total = signed + adjustment;
    let absolute = if bucket.min_value >= 0.0 {
        total
    } else {
        -total
    };
    bucket.absolute_total = u128::try_from(absolute)
        .map_err(|_| crate::invalid("Naval loss absolute total overflow"))?;
    Ok(bucket)
}
#[allow(clippy::too_many_arguments)]
fn corrected_summary(
    values: &mut Reader<'_>,
    corrections_reader: &mut Reader<'_>,
    metadata: &Metadata,
    index: usize,
    spans: &[(u64, i32, i32, Option<bool>)],
    bounds: (i32, i32),
    view: (f64, f64, u32),
    selection: &NavalLossesParams,
    display: u64,
) -> io::Result<crate::prepared_chart::ChartSummary> {
    let displayed: Vec<_> = spans
        .iter()
        .map(|&(holder, first, last, starts)| (field_key(holder, display), first, last, starts))
        .collect();
    let mut summary = values.summary_spans(&displayed, bounds, view, 0)?;
    let source: Vec<_> = spans
        .iter()
        .map(|&(holder, first, last, starts)| (field_key(holder, 6), first, last, starts))
        .collect();
    let grid = values.summary_spans(&source, bounds, view, 0)?;
    let all: Vec<_> = spans
        .iter()
        .map(|&(holder, first, last, starts)| {
            corrections(corrections_reader, metadata, holder, index)
                .map(|rows| (holder, first, last, starts, rows))
        })
        .collect::<io::Result<_>>()?;
    if all.iter().all(|(_, _, _, _, rows)| rows.is_empty()) {
        return Ok(summary);
    }
    for (ordinal, prototype) in grid.buckets.iter().enumerate() {
        let Some(prototype) = prototype else {
            continue;
        };
        let mut combined = None;
        let mut initialized = false;
        for (holder, first, last, starts, rows) in &all {
            let lo = prototype.first_hour.max(*first);
            let hi = prototype.last_hour.min(*last);
            if lo > hi {
                continue;
            }
            let mut part = None;
            let current = adjustment(rows, lo, selection.period);
            let changes = rows
                .iter()
                .any(|row| row.read_hour > lo && row.read_hour <= hi)
                || selection.period == NavalPeriod::Month && month(lo) != month(hi);
            let relevant: &[u64] = match selection.kind {
                NavalKind::Ships => &[2, 4],
                NavalKind::Convoys => &[3, 5],
                NavalKind::Both => &[2, 3, 4, 5],
            };
            let mut known = true;
            let mut mixed = changes || prototype.has_break;
            for &field in relevant {
                let mask = values.summary(
                    field_key(*holder, field),
                    (lo, hi),
                    (f64::from(lo), f64::from(hi), 1),
                    0,
                )?;
                let delta = match field {
                    2 => current.ship_unknown,
                    3 => current.convoy_unknown,
                    _ => -1,
                } as f64;
                for bucket in &mask.buckets {
                    let (min, max) =
                        bucket.map_or((0.0, 0.0), |bucket| (bucket.min_value, bucket.max_value));
                    if min + delta != 0.0 || max + delta != 0.0 {
                        known = false;
                    }
                    if min != max {
                        mixed = true;
                    }
                }
            }
            if mixed {
                part = rebuilt_bucket(
                    values,
                    *holder,
                    (lo, hi),
                    rows,
                    selection,
                    lo == *first && *starts != Some(false),
                    display,
                )?;
            } else if known {
                let raw_field = match display {
                    8 => 0,
                    9 => 1,
                    _ => 6,
                };
                let raw = values.summary(
                    field_key(*holder, raw_field),
                    (lo, hi),
                    (f64::from(lo), f64::from(hi), 1),
                    0,
                )?;
                let delta = match display {
                    8 => current.ships,
                    9 => current.convoys,
                    _ => match selection.kind {
                        NavalKind::Ships => current.ships,
                        NavalKind::Convoys => current.convoys,
                        NavalKind::Both => current.ships + current.convoys,
                    },
                };
                for raw in raw.buckets {
                    if let Some(mut bucket) = raw {
                        if bucket.first_hour > lo
                            || bucket.last_hour < hi
                            || bucket.has_break
                            || bucket.min_value < 0.0 && bucket.max_value > 0.0
                            || bucket.min_value + (delta as f64) < 0.0
                                && bucket.max_value + (delta as f64) > 0.0
                        {
                            part = rebuilt_bucket(
                                values,
                                *holder,
                                (lo, hi),
                                rows,
                                selection,
                                lo == *first && *starts != Some(false),
                                display,
                            )?;
                            break;
                        }
                        bucket.first_break = lo == *first && *starts != Some(false);
                        let shifted = Some(shifted(bucket, delta)?);
                        part = if part.is_none() {
                            shifted
                        } else {
                            crate::prepared_chart::merge(part, shifted)
                        };
                    } else {
                        // An omitted sparse raw operand is zero; the shared total supplies its recorded extent.
                        let mut zero = *prototype;
                        zero.first_hour = lo;
                        zero.last_hour = hi;
                        zero.first_value = 0.0;
                        zero.last_value = 0.0;
                        zero.min_value = 0.0;
                        zero.max_value = 0.0;
                        zero.absolute_total = 0;
                        zero.first_break = lo == *first && *starts != Some(false);
                        zero.min_hour = lo;
                        zero.max_hour = lo;
                        zero.last_value_hour = lo;
                        part = Some(shifted(zero, delta)?);
                    }
                }
            }
            combined = if initialized {
                crate::prepared_chart::merge(combined, part)
            } else {
                part
            };
            initialized = true;
        }
        summary.buckets[ordinal] = combined;
    }
    if summary.runs.is_some() {
        let mut exact = Vec::new();
        for (holder, first, last, starts, rows) in all {
            let range = (
                first.max(view.0.floor() as i32),
                last.min(view.1.floor() as i32),
            );
            if range.0 > range.1 {
                continue;
            }
            for run in exact_runs(
                values,
                holder,
                range,
                &rows,
                selection,
                range.0 == first && starts != Some(false),
            )? {
                if let Some(total) = amount(&run, selection.kind) {
                    exact.push(crate::prepared_chart::ChartRun {
                        first_hour: run.first_hour,
                        last_hour: run.last_hour,
                        break_before: run.starts_segment,
                        value: match display {
                            8 => run.ships.unwrap() as f64,
                            9 => run.convoys.unwrap() as f64,
                            _ => total as f64,
                        },
                    });
                }
            }
        }
        summary.runs = Some(exact);
    }
    Ok(summary)
}

/// Bind the requested alias intervals to their authenticated native holders once.
#[allow(clippy::type_complexity)]
fn holder_spans(
    tag: &str,
    selection: &NavalLossesParams,
    selected: &BTreeSet<Identity>,
    actors: &BTreeMap<String, u64>,
    history: &RecordedHistories,
    within: Option<&serde_json::Map<String, Value>>,
) -> io::Result<Vec<(u64, i32, i32, Option<bool>)>> {
    let native: Vec<_> = history
        .countries
        .get(tag)
        .into_iter()
        .flat_map(|country| country.identities.iter())
        .filter(|identity| {
            selection.top.is_some() || selected.contains(&(identity.slot, identity.country_id))
        })
        .filter_map(|identity| {
            actors
                .get(&actor((identity.slot, identity.country_id)))
                .map(|&holder| (holder, identity.first_hour, identity.last_hour, Some(true)))
        })
        .collect();
    let Some(within) = within else {
        return Ok(native);
    };
    let rows = within
        .get(tag)
        .and_then(Value::as_array)
        .ok_or_else(|| crate::invalid("Invalid naval loss binding"))?;
    let mut bound = Vec::new();
    for row in rows {
        let row = row
            .as_array()
            .filter(|row| row.len() == 4)
            .ok_or_else(|| crate::invalid("Invalid naval loss binding row"))?;
        if row[0].as_str() != Some(tag) {
            return Err(crate::invalid("Naval loss binding names another alias"));
        }
        let hour = |value: &Value| {
            value
                .as_i64()
                .and_then(|hour| i32::try_from(hour).ok())
                .ok_or_else(|| crate::invalid("Invalid naval loss binding hour"))
        };
        let first = hour(&row[1])?;
        let last = hour(&row[2])?;
        let starts = row[3]
            .as_bool()
            .ok_or_else(|| crate::invalid("Invalid naval loss binding boundary"))?;
        if first > last {
            return Err(crate::invalid("Reversed naval loss binding"));
        }
        for &(holder, lo, hi, _) in &native {
            let clipped_first = lo.max(first);
            let clipped_last = hi.min(last);
            if clipped_first <= clipped_last {
                bound.push((
                    holder,
                    clipped_first,
                    clipped_last,
                    Some(clipped_first == lo || starts),
                ));
            }
        }
    }
    bound.sort_by_key(|span| span.1);
    let mut merged: Vec<(u64, i32, i32, Option<bool>)> = Vec::new();
    for span in bound {
        if let Some(previous) = merged.last_mut()
            && previous.0 == span.0
            && previous.2 >= span.1
        {
            previous.2 = previous.2.max(span.2);
            if previous.1 == span.1 {
                previous.3 = Some(previous.3 == Some(true) || span.3 == Some(true));
            }
        } else {
            merged.push(span);
        }
    }
    Ok(merged)
}

#[allow(clippy::too_many_arguments)]
fn series_query(
    product: &str,
    params: &Value,
    selection: &NavalLossesParams,
    actors: &BTreeMap<String, u64>,
    metadata: &Metadata,
    history: &RecordedHistories,
    store: &store::Store,
    key: &store::SourceKey,
    flag: &AtomicBool,
) -> io::Result<Value> {
    let branch: u64 = selection.branch_id.parse().expect("validated branch");
    let index = channel(selection);
    if let Some(error) = &metadata.errors[index] {
        return Err(crate::invalid(error));
    }
    let mut values = Reader::new(
        store,
        key,
        format!("naval-loss-values-{branch}-{index}"),
        flag,
    )?;
    let mut ranks = Reader::new(
        store,
        key,
        format!("naval-loss-ranks-{branch}-{index}"),
        flag,
    )?;
    let mut correction_reader =
        Reader::new(store, key, format!("naval-loss-corrections-{branch}"), flag)?;
    let selected = country_history::selected_identities(history, &selection.tags);
    let mut wanted = if selection.top.is_some() {
        metadata.tags.keys().cloned().collect::<Vec<_>>()
    } else {
        country_history::selected_aliases(history, &selection.tags)
    };
    let within = params
        .get("within")
        .filter(|value| !value.is_null())
        .map(|value| {
            value
                .as_object()
                .ok_or_else(|| crate::invalid("Invalid naval loss bindings"))
        })
        .transpose()?;
    if let Some(within) = within {
        wanted.retain(|tag| within.contains_key(tag));
    }
    if product == "series-next-change" {
        let navigation: series::Navigation = serde_json::from_value(params.clone())?;
        let origin = navigation.origin()?;
        let forward = matches!(
            navigation.key,
            series::MeasureKey::Home | series::MeasureKey::ArrowRight
        );
        let mut rank_keys = Vec::new();
        if let Some(top) = selection.top {
            let hour = origin
                .clamp(navigation.first_hour, navigation.last_hour)
                .floor() as i32;
            let reached = candidates(&mut ranks, actors, metadata, selection, (hour, hour))?;
            let corrected = metadata
                .corrected
                .iter()
                .filter(|&&key| key % 12 == index as u64)
                .count();
            for position in 0..(top as usize)
                .saturating_add(corrected)
                .min(metadata.tags.len())
            {
                rank_keys.push((position as u64 * 2, None));
            }
            // Like army, navigate current candidate values and ranked-holder
            // transitions. Sparse corrected holders can also enter independently
            // of the base rank, so retain their dated aliases.
            wanted.retain(|tag| {
                reached.contains(tag)
                    || history.countries.get(tag).is_some_and(|country| {
                        country.identities.iter().any(|identity| {
                            actors
                                .get(&actor((identity.slot, identity.country_id)))
                                .is_some_and(|&holder| {
                                    metadata.corrected.contains(&(holder * 12 + index as u64))
                                })
                        })
                    })
            });
        }
        let mut masks = Vec::new();
        let mut correction_starts = Vec::new();
        for tag in &wanted {
            for (holder, first, last, _) in
                holder_spans(tag, selection, &selected, actors, history, within)?
            {
                let rows = corrections(&mut correction_reader, metadata, holder, index)?;
                for row in &rows {
                    let end = (selection.period == NavalPeriod::Month)
                        .then(|| {
                            row.month
                                .and_then(|bucket| bucket.checked_add(1))
                                .and_then(month_start)
                        })
                        .flatten();
                    correction_starts.extend(
                        [Some(row.read_hour), end]
                            .into_iter()
                            .flatten()
                            .filter(|&hour| hour >= first && hour <= last)
                            .map(f64::from),
                    );
                }
                masks.push((
                    holder,
                    vec![HourSpan {
                        first_hour: first,
                        last_hour: last,
                    }],
                    !rows.is_empty(),
                ));
            }
        }
        let mut value_keys = Vec::new();
        for (holder, mask, corrected) in &masks {
            value_keys.push((field_key(*holder, 7), Some(mask.as_slice())));
            if *corrected {
                value_keys.push((field_key(*holder, 6), Some(mask.as_slice())));
                if selection.kind == NavalKind::Both {
                    value_keys.extend(
                        [0, 1].map(|field| (field_key(*holder, field), Some(mask.as_slice()))),
                    );
                }
            }
        }
        let prepared = series::navigate(
            &mut [(&mut values, value_keys), (&mut ranks, rank_keys)],
            &navigation,
        )?
        .filter(|hour| {
            if forward {
                *hour > origin
            } else {
                *hour < origin
            }
        });
        let correction = correction_starts
            .into_iter()
            .filter(|hour| {
                *hour >= navigation.first_hour
                    && *hour <= navigation.last_hour
                    && if forward {
                        *hour > origin
                    } else {
                        *hour < origin
                    }
            })
            .reduce(|a, b| if forward { a.min(b) } else { a.max(b) });
        let hour = match (prepared, correction) {
            (Some(a), Some(b)) => Some(if forward { a.min(b) } else { a.max(b) }),
            (a, b) => a.or(b),
        };
        return Ok(
            json!({"hour":hour.or_else(|| navigation.from.map(|hour| hour.clamp(navigation.first_hour,navigation.last_hour)))}),
        );
    }
    if product != "series-summary" {
        return Err(crate::invalid("Unsupported naval loss series query"));
    }
    let (from, to, width) = series::viewport(
        params["from"].as_f64(),
        params["to"].as_f64(),
        params["width"]
            .as_u64()
            .and_then(|width| u32::try_from(width).ok()),
    )?;
    let bounds = (metadata.scan.first_hour, metadata.scan.last_hour);
    let view = (from, to, width);
    let hourly = to.floor() - from.floor() + 1.0 <= f64::from(width);
    let reached = if hourly && selection.top.is_some() {
        Some(window_reply(
            store,
            key,
            actors,
            metadata,
            history,
            selection,
            Some((from.floor() as i32, to.floor() as i32)),
            flag,
        )?)
    } else {
        None
    };
    let mut sampled = BTreeMap::<String, Vec<NavalLossRun>>::new();
    let mut lines = Vec::new();
    for tag in wanted {
        check_cancel(flag)?;
        let Some(&tag_key) = metadata.tags.get(&tag) else {
            continue;
        };
        if let Some(top) = selection.top {
            let ranked = ranks.summary(tag_key * 2 + 1, bounds, (from, to, 1), 0)?;
            let corrected = history.countries.get(&tag).is_some_and(|country| {
                country.identities.iter().any(|identity| {
                    actors
                        .get(&actor((identity.slot, identity.country_id)))
                        .is_some_and(|&holder| {
                            metadata.corrected.contains(&(holder * 12 + index as u64))
                        })
                })
            });
            if !corrected
                && !ranked
                    .buckets
                    .iter()
                    .flatten()
                    .any(|bucket| bucket.min_value <= f64::from(top))
            {
                continue;
            }
        }
        let spans = holder_spans(&tag, selection, &selected, actors, history, within)?;
        let mut summary = corrected_summary(
            &mut values,
            &mut correction_reader,
            metadata,
            index,
            &spans,
            bounds,
            view,
            selection,
            7,
        )?;
        if let Some(top) = selection.top {
            if let Some(reached) = &reached {
                let exact = reached.series.iter().find(|series| series.tag == tag);
                for bucket in &mut summary.buckets {
                    if bucket.is_some_and(|bucket| {
                        exact.is_none_or(|series| {
                            !series.runs.iter().any(|run| {
                                run.first_hour <= bucket.first_hour
                                    && run.last_hour >= bucket.first_hour
                            })
                        })
                    }) {
                        *bucket = None;
                    }
                }
                summary.runs = exact.map(|series| {
                    series
                        .runs
                        .iter()
                        .filter_map(|run| {
                            amount(run, selection.kind).map(|value| {
                                crate::prepared_chart::ChartRun {
                                    first_hour: run.first_hour,
                                    last_hour: run.last_hour,
                                    value: value as f64,
                                    break_before: run.starts_segment,
                                }
                            })
                        })
                        .collect()
                });
            } else {
                let membership = ranks.summary(tag_key * 2 + 1, bounds, view, 0)?;
                let corrected = spans.iter().any(|&(holder, _, _, _)| {
                    metadata.corrected.contains(&(holder * 12 + index as u64))
                });
                // Approved D-12: the same min-rank bucket mask as army, retaining corrected candidates.
                for (bucket, rank) in summary.buckets.iter_mut().zip(membership.buckets) {
                    if !corrected && rank.is_none_or(|rank| rank.min_value > f64::from(top)) {
                        *bucket = None;
                    }
                }
                summary.runs = None;
            }
        }
        if params["parts"].as_bool() == Some(true) && selection.kind == NavalKind::Both {
            let ships = corrected_summary(
                &mut values,
                &mut correction_reader,
                metadata,
                index,
                &spans,
                bounds,
                view,
                selection,
                8,
            )?;
            let convoys = corrected_summary(
                &mut values,
                &mut correction_reader,
                metadata,
                index,
                &spans,
                bounds,
                view,
                selection,
                9,
            )?;
            let mut runs = Vec::new();
            for ((total, ships), convoys) in summary
                .buckets
                .iter()
                .zip(&ships.buckets)
                .zip(&convoys.buckets)
            {
                let Some(total) = total else {
                    continue;
                };
                let mut hours = BTreeSet::from([
                    total.first_hour,
                    total.last_hour,
                    total.min_hour,
                    total.max_hour,
                ]);
                for bucket in [ships, convoys].into_iter().flatten() {
                    hours.extend([
                        bucket.first_hour,
                        bucket.last_hour,
                        bucket.min_hour,
                        bucket.max_hour,
                    ]);
                }
                let hours: Vec<_> = hours.into_iter().collect();
                for (at, &hour) in hours.iter().enumerate() {
                    let Some((holder, first, _, _)) = spans
                        .iter()
                        .find(|(_, first, last, _)| hour >= *first && hour <= *last)
                    else {
                        continue;
                    };
                    let corrections =
                        corrections(&mut correction_reader, metadata, *holder, index)?;
                    let (ships, convoys) =
                        exact(&mut values, *holder, hour, &corrections, selection)?;
                    runs.push(NavalLossRun {
                        first_hour: hour,
                        last_hour: if total.has_break {
                            hour
                        } else {
                            hours.get(at + 1).map_or(total.last_hour, |next| next - 1)
                        },
                        starts_segment: total.has_break || hour == *first,
                        ships,
                        convoys,
                    });
                }
            }
            sampled.insert(tag.clone(), runs);
        }
        series::summary_budget(from, to, width, lines.len() + 1)?;
        lines.push(json!({"tag":tag,"summary":summary}));
    }
    let mut reply = json!({"series":lines});
    if params["parts"].as_bool() == Some(true) && selection.kind == NavalKind::Both {
        reply["sampled"] = serde_json::to_value(project_reply(
            &metadata.scan,
            selection,
            flag,
            metadata.warnings[index].clone(),
            Some(history),
            sampled,
        )?)?;
    }
    if json_len(&reply, MAX_PRODUCT_REPLY_BYTES).is_none() {
        return Err(crate::invalid(
            "Naval loss summary exceeds the transport reply budget",
        ));
    }
    Ok(reply)
}

#[cfg(test)]
mod tests;
