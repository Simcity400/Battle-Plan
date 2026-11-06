//! Equipment losses folded per segment on the walk's workers ([`Lens`]). A segment's lens folds
//! every country from the segment's first hour, each from what the restart before it held of the
//! amounts it keeps ([`Seen`]); its totals, read version and spans count from nothing, and what
//! it read before the segment is left for the emit. With the first hour it hands the emit what
//! it began each country from ([`Begun`]); at the segment's end, or where it stops short (a gap,
//! another campaign's hour, a refusal), what each country made of the hours it folded
//! ([`Given`]).
//!
//! The emit binds each country whose state is the one the lens began it from
//! ([`Prepared::bind`]) and folds the others itself. At the hand-over it adds each bound
//! country's carried totals, read version and spans once ([`Prepared::splice`]), numbers the
//! keys the segment made, and mends the first reads of amounts the country kept from before the
//! segment that the restart no longer held ([`Fresh`]). Ships are the emit's own throughout.

use super::*;
use dashboard_world::Roster;
use replay_data::format::FormatVersion;

/// What a lens's country holds of a date it read before the segment.
pub(super) const PRIOR: i32 = i32::MIN;

/// A kept amount's key, by its record.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, replay_spool::Spooled)]
pub(crate) enum Key {
    Log(LogKey),
    Convoy((u32, i32, u32)),
    Lend((u32, u8, u32)),
}

impl Key {
    /// What one of its record's amounts adds to its leaf's fixed5 total.
    fn scale(self) -> i128 {
        match self {
            Key::Convoy(_) => i128::from(AMOUNT_SCALE),
            Key::Log(_) | Key::Lend(_) => 1,
        }
    }
}

/// The amounts a country last read of each record it keeps them of, and its exchanges' pools.
#[derive(Clone, Default, replay_spool::Spooled)]
pub(crate) struct Seen {
    pub(super) log: Vec<(LogKey, i64)>,
    pub(super) convoys: Vec<((u32, i32, u32), i32)>,
    pub(super) lend: Vec<((u32, u8, u32), i64)>,
    pub(super) exchanges: Vec<(u32, Vec<(u8, u32)>)>,
}

/// The rows a restart held of the records whose amounts countries keep, by raw country.
#[derive(Default)]
pub(crate) struct Restored {
    /// Combat Log rows: raw owner, row key, entry and amount.
    pub(crate) log: Vec<(u32, u128, LogKey, i64)>,
    /// SunkConvoys rows: raw owner, entry and count.
    pub(crate) convoys: Vec<(u32, (u32, i32, u32), i32)>,
    /// Lend-lease rows: raw giver, row key, pool and amount.
    pub(crate) lend: Vec<(u32, u128, (u32, u8, u32), i64)>,
}

impl Restored {
    /// Each country's amounts by slot, as `roster` maps the raw countries: a key two aliases
    /// share holds the later row's, as a read takes a country's rows in key order.
    fn seen(self, roster: &Roster) -> HashMap<u32, Seen> {
        let mut log: HashMap<u32, Vec<(u128, LogKey, i64)>> = HashMap::new();
        for (raw, row, key, amount) in self.log {
            if let Some(slot) = roster.slot_of(raw) {
                log.entry(slot).or_default().push((row, key, amount));
            }
        }
        let mut convoys: HashMap<u32, BTreeMap<(u32, i32, u32), i32>> = HashMap::new();
        for (raw, key, count) in self.convoys {
            if let Some(slot) = roster.slot_of(raw) {
                convoys.entry(slot).or_default().insert(key, count);
            }
        }
        let mut lend: HashMap<u32, Vec<(u128, (u32, u8, u32), i64)>> = HashMap::new();
        for (raw, row, pool, amount) in self.lend {
            if let Some(slot) = roster.slot_of(raw) {
                lend.entry(slot).or_default().push((row, pool, amount));
            }
        }
        let mut seen: HashMap<u32, Seen> = HashMap::new();
        for (slot, mut rows) in log {
            rows.sort_unstable_by_key(|row| row.0);
            let kept: BTreeMap<LogKey, i64> = rows.into_iter().map(|row| (row.1, row.2)).collect();
            seen.entry(slot).or_default().log = kept.into_iter().collect();
        }
        for (slot, kept) in convoys {
            seen.entry(slot).or_default().convoys = kept.into_iter().collect();
        }
        for (slot, mut rows) in lend {
            rows.sort_unstable_by_key(|row| row.0);
            let pools: BTreeMap<(u32, u8, u32), i64> =
                rows.into_iter().map(|row| (row.1, row.2)).collect();
            let mut exchanges: BTreeMap<u32, Vec<(u8, u32)>> = BTreeMap::new();
            for &(exchange, kind, variant) in pools.keys() {
                exchanges.entry(exchange).or_default().push((kind, variant));
            }
            let held = seen.entry(slot).or_default();
            held.lend = pools.into_iter().collect();
            held.exchanges = exchanges.into_iter().collect();
        }
        seen
    }
}

/// What a lens's country holds after the segment's first hour of what its drains' coverage
/// goes on with.
#[derive(Clone, Default, PartialEq, Eq, replay_spool::Spooled)]
pub(crate) struct Status {
    pub(super) withheld: Option<String>,
    pub(super) counted: Option<i32>,
    /// Whether that hour dated its drain's coverage.
    pub(super) covered: bool,
}

/// A country a lens began at the segment's first hour.
#[derive(replay_spool::Spooled)]
pub(crate) struct Opened {
    pub(super) identity: Identity,
    pub(super) seen: Seen,
    pub(super) status: Status,
}

/// What a lens hands the emit with the segment's first hour.
#[derive(replay_spool::Spooled)]
pub(crate) struct Begun {
    pub(super) first: i32,
    pub(super) countries: Vec<Opened>,
    pub(super) absent: Status,
}

/// A lens's first read of an amount its country held none of: the emit's country may have kept
/// one from before the segment, which that read's change counts from.
#[derive(Clone, replay_spool::Spooled)]
pub(crate) struct Fresh {
    pub(super) hour: i32,
    pub(super) leaf: LeafKey,
    pub(super) key: Key,
    /// In its record's units, as [`kept_amount`] reads them.
    pub(super) amount: i64,
    /// The change the lens took from it, in its leaf's fixed5 units.
    pub(super) delta: i128,
    /// The leaf's other changes that hour.
    pub(super) others: u32,
}

/// An exchange the lens saw end, whose last read and dates were all before the segment.
#[derive(replay_spool::Spooled)]
pub(crate) struct Prior {
    /// Its place in the lens's ended exchanges.
    pub(super) index: usize,
    pub(super) exchange: u32,
    /// Whether the lens's country had taken the lend-lease read hour before the segment.
    pub(super) read: bool,
}

/// A leaf a lens's country changed: its hour made, its total and its changes, from nothing.
#[derive(replay_spool::Spooled)]
pub(crate) struct Leaf {
    pub(super) leaf: LeafKey,
    pub(super) made: i32,
    pub(super) total: i128,
    pub(super) changes: Changes,
}

/// A leaf's totals at the hours that changed it, ascending, each kept as its hour's and total's
/// differences from the change before, zigzagged and in variable-length bytes.
#[derive(Clone, Default, replay_spool::Spooled)]
pub(crate) struct Changes {
    bytes: Vec<u8>,
    last: (i32, i128),
}

fn put_varint(bytes: &mut Vec<u8>, mut value: u128) {
    while value >= 0x80 {
        bytes.push(value as u8 | 0x80);
        value >>= 7;
    }
    bytes.push(value as u8);
}

fn take_varint(bytes: &[u8], at: &mut usize) -> u128 {
    let mut value = 0_u128;
    let mut shift = 0;
    while let Some(&byte) = bytes.get(*at) {
        *at += 1;
        if shift < 128 {
            value |= u128::from(byte & 0x7f) << shift;
        }
        if byte < 0x80 {
            break;
        }
        shift += 7;
    }
    value
}

impl Changes {
    /// The total `total` from `hour`, past every change it holds.
    pub(super) fn push(&mut self, hour: i32, total: i128) {
        let (last_hour, last_total) = self.last;
        let step = i64::from(hour) - i64::from(last_hour);
        put_varint(&mut self.bytes, u128::from((step << 1 ^ step >> 63) as u64));
        let delta = total.wrapping_sub(last_total);
        put_varint(&mut self.bytes, (delta << 1 ^ delta >> 127) as u128);
        self.last = (hour, total);
    }

    pub(super) fn iter(&self) -> impl Iterator<Item = (i32, i128)> + '_ {
        let (mut at, mut hour, mut total) = (0, 0_i64, 0_i128);
        std::iter::from_fn(move || {
            if at >= self.bytes.len() {
                return None;
            }
            let step = take_varint(&self.bytes, &mut at) as u64;
            hour += (step >> 1) as i64 ^ -((step & 1) as i64);
            let delta = take_varint(&self.bytes, &mut at);
            total = total.wrapping_add((delta >> 1) as i128 ^ -((delta & 1) as i128));
            Some((hour as i32, total))
        })
    }

    /// Lets go of every change after hour `last`.
    fn through(&mut self, last: i32) {
        if self.last.0 <= last {
            return;
        }
        let mut kept = Self::default();
        for (hour, total) in self.iter().take_while(|&(hour, _)| hour <= last) {
            kept.push(hour, total);
        }
        *self = kept;
    }
}

/// What a lens's country made of the hours it folded.
#[derive(replay_spool::Spooled)]
pub(crate) struct Given {
    pub(super) identity: Identity,
    pub(super) scanned: Scanned,
    /// Its kept amounts unlike those it began from.
    pub(super) kept: Vec<(Key, i64)>,
    /// Its exchanges, each with the hour it last read it, and the hour it last took the
    /// lend-lease read.
    pub(super) exchanges: Vec<(u32, i32, Vec<(u8, u32)>)>,
    pub(super) exchanges_read: Option<i32>,
    pub(super) withheld: Option<String>,
    pub(super) leaves: Vec<Leaf>,
    /// The hours its read version went up, and its first read with its hour.
    pub(super) versions: Vec<i32>,
    pub(super) first_read: Option<(i32, String)>,
    pub(super) last_read: Option<String>,
    pub(super) read_key: Option<(Option<Coverage>, Option<String>, u8)>,
    pub(super) fresh: Vec<Fresh>,
    pub(super) prior: Vec<Prior>,
    pub(super) variants: Vec<u32>,
}

/// What a lens hands back of every country it folded.
#[derive(replay_spool::Spooled)]
pub(crate) struct Part {
    pub(super) countries: Vec<Given>,
}

/// What a lens hands the emit with an item.
#[derive(replay_spool::Spooled)]
pub(crate) enum Handed {
    Begun(Begun),
    Part(Part),
}

/// What a lens records of a country as it folds it.
#[derive(Default)]
pub(crate) struct Out {
    pub(super) seen: Seen,
    /// Whether the lens began it from the restart, and has taken it since.
    pub(super) begun: bool,
    pub(super) taken: bool,
    pub(super) versions: Vec<i32>,
    pub(super) first_read: Option<(i32, String)>,
    /// Each leaf's hour made and changes, by its key less one.
    pub(super) leaves: Vec<(i32, Changes)>,
    pub(super) fresh: Vec<Fresh>,
    /// The hour's first reads of amounts it held none of, until the hour publishes.
    touches: Vec<Fresh>,
    pub(super) prior: Vec<Prior>,
    pub(super) variants: BTreeSet<u32>,
}

/// Records the first reads among `rows` of keys `held` holds none of.
fn touch<K: Ord + Copy, V>(
    touches: &mut Vec<Fresh>,
    hour: i32,
    held: &BTreeMap<K, V>,
    rows: impl Iterator<Item = (K, i64)>,
    place: impl Fn(K) -> (LeafKey, Key),
) {
    let mut met = BTreeSet::new();
    for (key, amount) in rows {
        if held.contains_key(&key) || !met.insert(key) {
            continue;
        }
        let (leaf, key) = place(key);
        touches.push(Fresh {
            hour,
            leaf,
            key,
            amount,
            delta: i128::from(amount),
            others: 0,
        });
    }
}

impl Out {
    /// A Combat Log read's first of `key`, which its country held none of.
    pub(super) fn first_log(&mut self, hour: i32, key: LogKey, amount: i64) {
        let (_, kind, _, variant) = key;
        let source = if kind == engine_land_inputs::ncombat_log_cmanager::RECOVERED_KIND {
            LossSource::Recovered
        } else {
            LossSource::CombatLog(kind)
        };
        self.touches.push(Fresh {
            hour,
            leaf: (source, Some(variant)),
            key: Key::Log(key),
            amount,
            delta: i128::from(amount),
            others: 0,
        });
    }

    /// A SunkConvoys read's first of `key`, which its country held none of.
    pub(super) fn first_convoy(&mut self, hour: i32, key: (u32, i32, u32), count: i32) {
        let (amount, key) = (i64::from(count), Key::Convoy(key));
        self.touches.push(Fresh {
            hour,
            leaf: (LossSource::SunkConvoys, None),
            key,
            amount,
            delta: i128::from(amount) * key.scale(),
            others: 0,
        });
    }

    /// A lend-lease read of the country: its first reads, and the exchanges it saw end whose
    /// dates were all before the segment.
    pub(super) fn lend(
        &mut self,
        scanned: &mut Scanned,
        state: &mut State,
        hour: i32,
        pools: &[LendRead],
    ) {
        touch(&mut self.touches, hour, &state.lend_lease, pools.iter().copied(), |key| {
            let (_, kind, variant) = key;
            let source = if kind == engine_resources::clend_lease_exchange::SUNK_KIND {
                LossSource::LendLeaseSunk
            } else {
                LossSource::LendLeaseSent
            };
            ((source, Some(variant)), Key::Lend(key))
        });
        let present: BTreeSet<u32> = pools.iter().map(|((exchange, _, _), _)| *exchange).collect();
        let ending: Vec<u32> = state
            .exchanges
            .keys()
            .filter(|exchange| !present.contains(exchange))
            .copied()
            .collect();
        let (before, read) = (scanned.ended.len(), state.exchanges_read.is_some());
        lend_lease_rows(scanned, state, hour, pools);
        for (at, exchange) in ending.into_iter().enumerate() {
            let index = before + at;
            if scanned
                .ended
                .get(index)
                .is_some_and(|ended| ended.from == PRIOR.saturating_add(1))
            {
                self.prior.push(Prior {
                    index,
                    exchange,
                    read,
                });
            }
        }
    }

    /// A lens's [`publish`] of each leaf the hour changed, from nothing.
    pub(super) fn publish(
        &mut self,
        scanned: &mut Scanned,
        totals: &mut BTreeMap<LeafKey, i128>,
        leaves: &mut BTreeMap<LeafKey, u64>,
        hour: i32,
        drains: &[Drained],
    ) -> io::Result<()> {
        let events = std::mem::take(&mut scanned.events);
        let counts: BTreeMap<LeafKey, u32> = if self.touches.is_empty() {
            BTreeMap::new()
        } else {
            events
                .iter()
                .map(|(leaf, events)| (*leaf, events.len() as u32))
                .collect()
        };
        let mut amounts = Vec::with_capacity(events.len() + drains.len());
        for (leaf, events) in events {
            if let Some(variant) = leaf.1 {
                self.variants.insert(variant);
            }
            let mut delta = 0;
            for (at, amount) in events {
                if at != hour {
                    return Err(io::Error::other(format!(
                        "Equipment loss cause {:?} has an unsupported backdated hour {at}",
                        leaf.0
                    )));
                }
                delta += amount;
            }
            amounts.push((leaf, delta));
        }
        for &(cause, variant, delta) in drains {
            self.variants.insert(variant);
            amounts.push((
                (LossSource::Destruction(cause), Some(variant)),
                i128::from(delta),
            ));
        }
        amounts.sort_unstable_by_key(|item| item.0);
        let mut changed: Vec<LeafKey> = Vec::with_capacity(amounts.len());
        for (leaf, delta) in amounts {
            *totals.entry(leaf).or_default() += delta;
            if !leaves.contains_key(&leaf) {
                leaves.insert(leaf, leaves.len() as u64 + 1);
                self.leaves.push((hour, Changes::default()));
            }
            if changed.last() != Some(&leaf) {
                changed.push(leaf);
            }
        }
        for leaf in changed {
            let local = leaves[&leaf] as usize;
            self.leaves[local - 1].1.push(hour, totals[&leaf]);
        }
        if !self.touches.is_empty() {
            let touches = std::mem::take(&mut self.touches);
            let mut moved: BTreeMap<LeafKey, u32> = BTreeMap::new();
            for touch in touches.iter().filter(|touch| touch.delta != 0) {
                *moved.entry(touch.leaf).or_default() += 1;
            }
            for mut touch in touches {
                let all = counts.get(&touch.leaf).copied().unwrap_or(0);
                touch.others = all.saturating_sub(moved.get(&touch.leaf).copied().unwrap_or(0));
                self.fresh.push(touch);
            }
        }
        Ok(())
    }
}

/// What a lens's fold begins its countries from.
#[derive(Default)]
pub(crate) struct Mode {
    baselines: HashMap<u32, Seen>,
}

impl Mode {
    /// What country `identity`, made at step `index`, begins from: at the segment's first step,
    /// what the restart held of it; later, nothing, as the emit makes it.
    pub(super) fn begin(&self, identity: Identity, index: u64, state: &mut State) -> Out {
        let seen = if index == 0 {
            self.baselines
                .get(&identity.0)
                .cloned()
                .unwrap_or_default()
        } else {
            Seen::default()
        };
        state.log = seen.log.iter().copied().collect();
        state.convoys = seen.convoys.iter().copied().collect();
        state.lend_lease = seen.lend.iter().copied().collect();
        state.exchanges = seen
            .exchanges
            .iter()
            .map(|(exchange, pools)| (*exchange, (PRIOR, pools.iter().copied().collect())))
            .collect();
        Out {
            seen,
            begun: index == 0,
            ..Out::default()
        }
    }
}

/// The segment a lens folds, as the emit took it up.
pub(crate) struct Span {
    pub(super) first: i32,
    /// The step of its first hour.
    pub(super) index: u64,
    /// The emit's last lend-lease read before it.
    pub(super) lend_read: Option<i32>,
    /// The emit's absent country before it, which the lens begins later countries as.
    pub(super) absent: Scanned,
    pub(super) withheld: Option<String>,
    /// Whether the emit's branch goes on into it unbroken, and its absent country with the lens's.
    live: bool,
    newborns: bool,
}

impl Span {
    pub(super) fn live(&self) -> bool {
        self.live
    }

    /// Whether a country made in it binds to the lens.
    pub(super) fn newborns(&self) -> bool {
        self.live && self.newborns
    }
}

/// A segment's fold of every country, on its walk's worker.
pub(crate) struct Lens {
    fold: Box<Prepared>,
    restored: Option<Restored>,
    first: i32,
    live: bool,
    started: bool,
}

/// A refusal a lens met, with what it hands back of the hours before it, at the last of them.
pub(crate) struct Failed {
    pub(crate) part: Option<(i32, Box<Handed>)>,
    pub(crate) error: io::Error,
}

impl Lens {
    /// The lens of a segment whose first hour is `first`, after the rows `restored` held.
    pub(crate) fn new(format: FormatVersion, first: i32, restored: Restored) -> Self {
        let mut fold = Box::new(Prepared::starting(format, first));
        fold.lens = Some(Box::default());
        fold.lend_read = Some(PRIOR);
        Self {
            fold,
            restored: Some(restored),
            first,
            live: true,
            started: false,
        }
    }

    /// Folds hour `at`'s facts, None at another campaign's hour: what the lens hands on with
    /// them, before them.
    pub(crate) fn hour(
        &mut self,
        at: i32,
        roster: &Roster,
        input: Option<&Input>,
    ) -> Result<Option<Box<Handed>>, Failed> {
        if !self.live {
            return Ok(None);
        }
        let Some(input) = input else {
            return Ok(self.stop());
        };
        if !self.started {
            if at != self.first {
                self.live = false;
                return Ok(None);
            }
            if let (Some(mode), Some(restored)) = (&mut self.fold.lens, self.restored.take()) {
                mode.baselines = restored.seen(roster);
            }
        } else if i64::from(at) > self.fold.next {
            return Ok(self.stop());
        }
        if let Err(error) = self
            .fold
            .observe_input(at, input)
            .and_then(|_| self.fold.drain())
        {
            self.live = false;
            let part = if self.started {
                self.give(at.saturating_sub(1))
            } else {
                None
            };
            return Err(Failed { part, error });
        }
        if self.started {
            return Ok(None);
        }
        self.started = true;
        Ok(Some(Box::new(Handed::Begun(self.begun(at)))))
    }

    /// What it hands back at the segment's end.
    pub(crate) fn end(&mut self) -> Option<(i32, Box<Handed>)> {
        if !std::mem::replace(&mut self.live, false) || !self.started {
            return None;
        }
        self.give(self.last())
    }

    /// Stops short before an hour it cannot fold: what it hands back with that hour.
    fn stop(&mut self) -> Option<Box<Handed>> {
        self.live = false;
        if !self.started {
            return None;
        }
        self.give(self.last()).map(|(_, part)| part)
    }

    fn last(&self) -> i32 {
        i32::try_from(self.fold.next - 1).unwrap_or(self.first)
    }

    fn begun(&self, first: i32) -> Begun {
        let status = |state: &State, scanned: &Scanned| Status {
            withheld: state.destruction_withheld.clone(),
            counted: scanned.destruction_counted_from,
            covered: !scanned.destruction_coverage.is_empty(),
        };
        Begun {
            first,
            countries: self
                .fold
                .countries
                .iter()
                .filter(|(_, country)| country.born == 0)
                .map(|(&identity, country)| Opened {
                    identity,
                    seen: country
                        .out
                        .as_ref()
                        .map(|out| out.seen.clone())
                        .unwrap_or_default(),
                    status: status(&country.state, &country.scanned),
                })
                .collect(),
            absent: status(&self.fold.absent_state, &self.fold.absent),
        }
    }

    /// Hands back every country through hour `last`.
    fn give(&mut self, last: i32) -> Option<(i32, Box<Handed>)> {
        let countries = self
            .fold
            .countries
            .iter_mut()
            .filter_map(|(&identity, country)| {
                let out = country.out.take()?;
                Some(given(identity, country, *out, last))
            })
            .collect();
        Some((last, Box::new(Handed::Part(Part { countries }))))
    }
}

/// What `country` made through hour `last`, as `out` recorded it.
fn given(identity: Identity, country: &mut Country, out: Out, last: i32) -> Given {
    let Out {
        seen,
        mut versions,
        first_read,
        leaves: made,
        mut fresh,
        prior,
        variants,
        ..
    } = out;
    let mut by_local: Vec<Option<LeafKey>> = vec![None; country.leaves.len()];
    for (&leaf, &local) in &country.leaves {
        if let Some(held) = by_local.get_mut((local as usize).wrapping_sub(1)) {
            *held = Some(leaf);
        }
    }
    let leaves = made
        .into_iter()
        .zip(by_local)
        .filter_map(|((made, mut changes), leaf)| {
            let leaf = leaf.filter(|_| made <= last)?;
            changes.through(last);
            Some(Leaf {
                leaf,
                made,
                total: country.totals.get(&leaf).copied().unwrap_or(0),
                changes,
            })
        })
        .collect();
    versions.retain(|hour| *hour <= last);
    fresh.retain(|touch| touch.hour <= last);
    Given {
        identity,
        kept: changed(&country.state, &seen),
        exchanges: country
            .state
            .exchanges
            .iter()
            .map(|(&exchange, (seen, pools))| (exchange, *seen, pools.iter().copied().collect()))
            .collect(),
        exchanges_read: country.state.exchanges_read,
        withheld: country.state.destruction_withheld.clone(),
        scanned: std::mem::take(&mut country.scanned),
        leaves,
        versions,
        first_read: first_read.filter(|(hour, _)| *hour <= last),
        last_read: country.last_read.clone(),
        read_key: country.read_key.clone(),
        fresh,
        prior,
        variants: variants.into_iter().collect(),
    }
}

/// The kept amounts of `state` unlike those of `seen`, which it began from.
fn changed(state: &State, seen: &Seen) -> Vec<(Key, i64)> {
    let mut kept = Vec::new();
    unlike(&state.log, &seen.log, |key, amount| {
        kept.push((Key::Log(key), amount));
    });
    unlike(&state.convoys, &seen.convoys, |key, count| {
        kept.push((Key::Convoy(key), i64::from(count)));
    });
    unlike(&state.lend_lease, &seen.lend, |key, amount| {
        kept.push((Key::Lend(key), amount));
    });
    kept
}

/// Each entry of `now` that `seen`, ascending, holds otherwise or not at all.
fn unlike<K: Ord + Copy, V: PartialEq + Copy>(
    now: &BTreeMap<K, V>,
    seen: &[(K, V)],
    mut out: impl FnMut(K, V),
) {
    let mut seen = seen.iter().peekable();
    for (&key, &value) in now {
        while seen.next_if(|(held, _)| *held < key).is_some() {}
        if seen
            .peek()
            .is_none_or(|(held, was)| *held != key || *was != value)
        {
            out(key, value);
        }
    }
}

/// Whether a country of the emit whose drains' coverage is as `withheld` and `scanned` hold
/// goes on as a lens's at `first` that `lens` describes.
fn binds(withheld: &Option<String>, scanned: &Scanned, lens: &Status, first: i32) -> bool {
    // The lens's read names its last drain coverage: the emit's goes on until the lens dates one.
    let covered = lens.covered || scanned.destruction_coverage.is_empty();
    covered
        && match withheld {
            Some(reason) => lens.withheld.as_deref() == Some(reason.as_str()),
            None => {
                scanned.destruction_counted_from.is_none()
                    || lens.counted == Some(first)
                    || lens.withheld.is_some()
            }
        }
}

/// Whether `state` holds every amount and exchange `seen` holds.
fn holds(state: &State, seen: &Seen) -> bool {
    seen.log
        .iter()
        .all(|(key, amount)| state.log.get(key).copied().unwrap_or(0) == *amount)
        && seen
            .convoys
            .iter()
            .all(|(key, count)| state.convoys.get(key).copied().unwrap_or(0) == *count)
        && seen
            .lend
            .iter()
            .all(|(key, amount)| state.lend_lease.get(key).copied().unwrap_or(0) == *amount)
        && state.exchanges.len() == seen.exchanges.len()
        && state
            .exchanges
            .iter()
            .zip(&seen.exchanges)
            .all(|((exchange, (_, pools)), (held, designs))| {
                exchange == held && pools.iter().eq(designs.iter())
            })
}

/// The amount `state` kept of `key`.
fn kept_amount(state: &State, key: Key) -> i64 {
    match key {
        Key::Log(key) => state.log.get(&key).copied().unwrap_or(0),
        Key::Convoy(key) => state.convoys.get(&key).map_or(0, |count| i64::from(*count)),
        Key::Lend(key) => state.lend_lease.get(&key).copied().unwrap_or(0),
    }
}

/// How a leaf's hour goes otherwise than its lens took it: its change, and whether either
/// changed it.
#[derive(Clone, Copy)]
struct Correction {
    hour: i32,
    delta: i128,
    serial: bool,
}

/// A lens's changes of a leaf, from nothing, mended by `fixes`: the changes the emit's own fold
/// would have made, and what they add to the lens's total.
fn mend(changes: &[(i32, i128)], fixes: &[Correction]) -> (Vec<(i32, i128)>, i128) {
    if fixes.is_empty() {
        return (changes.to_vec(), 0);
    }
    let mut mended = Vec::with_capacity(changes.len() + fixes.len());
    let (mut offset, mut value) = (0_i128, 0_i128);
    let (mut changes, mut fixes) = (changes.iter().peekable(), fixes.iter().peekable());
    loop {
        let hour = match (changes.peek(), fixes.peek()) {
            (None, None) => break,
            (Some(change), Some(fix)) => change.0.min(fix.hour),
            (Some(change), None) => change.0,
            (None, Some(fix)) => fix.hour,
        };
        if let Some(change) = changes.next_if(|change| change.0 == hour) {
            value = change.1;
        }
        match fixes.next_if(|fix| fix.hour == hour) {
            None => mended.push((hour, value + offset)),
            Some(fix) => {
                offset += fix.delta;
                if fix.serial {
                    mended.push((hour, value + offset));
                }
            }
        }
    }
    (mended, offset)
}

/// A span of hours a lens dates from the segment's first.
trait Spanned {
    fn first(&self) -> i32;
    fn last(&self) -> i32;
    fn set_last(&mut self, last: i32);
    /// Whether it goes on as `other` does.
    fn same(&self, other: &Self) -> bool;
}

impl Spanned for CoverageSpan {
    fn first(&self) -> i32 {
        self.first_hour
    }
    fn last(&self) -> i32 {
        self.last_hour
    }
    fn set_last(&mut self, last: i32) {
        self.last_hour = last;
    }
    fn same(&self, other: &Self) -> bool {
        self.coverage == other.coverage
    }
}

impl Spanned for DestructionCoverageSpan {
    fn first(&self) -> i32 {
        self.first_hour
    }
    fn last(&self) -> i32 {
        self.last_hour
    }
    fn set_last(&mut self, last: i32) {
        self.last_hour = last;
    }
    fn same(&self, other: &Self) -> bool {
        self.reason == other.reason
    }
}

impl Spanned for UnknownSpan {
    fn first(&self) -> i32 {
        self.first_hour
    }
    fn last(&self) -> i32 {
        self.last_hour
    }
    fn set_last(&mut self, last: i32) {
        self.last_hour = last;
    }
    fn same(&self, _: &Self) -> bool {
        true
    }
}

/// Joins a lens's spans onto the emit's. An `open` span goes on through every hour before the
/// lens's first of its kind, or through the lens's last hour, as the takes between extended it.
fn join<T: Spanned>(carried: &mut Vec<T>, lens: Vec<T>, through: Option<i32>, open: bool) {
    let mut lens = lens.into_iter().peekable();
    if open && let Some(last) = carried.last_mut() {
        let until = lens
            .peek()
            .map(|span| span.first().saturating_sub(1))
            .or(through);
        if let Some(until) = until
            && last.last() < until
        {
            last.set_last(until);
        }
    }
    let joins = match (carried.last(), lens.peek()) {
        (Some(last), Some(next)) => {
            last.same(next) && i64::from(last.last()) + 1 >= i64::from(next.first())
        }
        _ => false,
    };
    if joins && let (Some(last), Some(next)) = (carried.last_mut(), lens.next()) {
        last.set_last(last.last().max(next.last()));
    }
    carried.extend(lens);
}

/// The emit's spans of a country, from before the segment, joined with what the lens dated
/// from its first hour; `withheld` is why the emit's country withheld its drains before.
fn merge(carried: &mut Scanned, lens: Scanned, first: i32, withheld: Option<&str>) {
    carried.advance(first.saturating_sub(1));
    let through = lens.last_hour;
    join(&mut carried.coverage, lens.coverage, through, true);
    join(
        &mut carried.destruction_coverage,
        lens.destruction_coverage,
        through,
        true,
    );
    for (record, spans) in lens.unread {
        join(carried.unread.entry(record).or_default(), spans, through, false);
    }
    carried.unread_active = lens.unread_active;
    let mut moved = Vec::with_capacity(lens.destruction_unknown.len());
    for span in lens.destruction_unknown {
        let detail = span.detail.clone().unwrap_or_default();
        // A lens withholding drains at the first hour repeats what withheld them before.
        if span.reason == UnknownReason::DestructionWithheld
            && span.first_hour == first
            && withheld == Some(detail.as_str())
        {
            moved.push(None);
            continue;
        }
        let held = carried
            .destruction_unknown_index
            .iter()
            .find(|((reason, text), _)| *reason == span.reason && *text == detail)
            .map(|(_, &index)| index);
        let index = match held {
            Some(index) => {
                let into = &mut carried.destruction_unknown[index];
                into.last_hour = into.last_hour.max(span.last_hour);
                index
            }
            None => {
                let index = carried.destruction_unknown.len();
                carried
                    .destruction_unknown_index
                    .insert((span.reason, detail), index);
                carried.destruction_unknown.push(span);
                index
            }
        };
        moved.push(Some(index));
    }
    carried.destruction_active = lens
        .destruction_active
        .iter()
        .filter_map(|&index| moved.get(index).copied().flatten())
        .collect();
    carried.ended.extend(lens.ended);
    carried.unresolved.extend(lens.unresolved);
    carried.combat_unresolved.extend(lens.combat_unresolved);
    carried.convoy_drops += lens.convoy_drops;
    carried.destruction_counted_from = carried
        .destruction_counted_from
        .or(lens.destruction_counted_from);
    carried.last_hour = lens.last_hour.or(carried.last_hour);
}

/// Adds what a lens made of bound `country` in `span` to what the emit carried of it.
fn splice_country(
    span: &Span,
    given: Given,
    country: &mut Country,
    names: &mut Vec<Name>,
    variants: &mut BTreeSet<u32>,
) -> io::Result<()> {
    let Given {
        identity,
        scanned,
        kept,
        exchanges,
        exchanges_read,
        withheld,
        leaves,
        versions,
        first_read,
        last_read,
        read_key,
        mut fresh,
        prior,
        variants: lost,
    } = given;
    let first = span.first;
    // Amounts the country kept from before the segment that the restart did not hold: the
    // lens counted their first reads from nothing.
    let mut drops = 0_usize;
    let mut corrections: BTreeMap<LeafKey, Vec<Correction>> = BTreeMap::new();
    fresh.sort_by_key(|touch| (touch.leaf, touch.hour));
    for touches in fresh.chunk_by(|a, b| (a.leaf, a.hour) == (b.leaf, b.hour)) {
        let mut serial = touches[0].others > 0;
        let (mut delta, mut kept_before) = (0_i128, false);
        for touch in touches {
            let held = kept_amount(&country.state, touch.key);
            if held == 0 {
                serial |= touch.delta != 0;
                continue;
            }
            kept_before = true;
            let own = touch.key.scale()
                * match touch.key {
                    Key::Convoy(_) if touch.amount < held => {
                        drops += 1;
                        i128::from(touch.amount)
                    }
                    _ => i128::from(touch.amount) - i128::from(held),
                };
            serial |= own != 0;
            delta += own - touch.delta;
        }
        if kept_before {
            corrections
                .entry(touches[0].leaf)
                .or_default()
                .push(Correction {
                    hour: touches[0].hour,
                    delta,
                    serial,
                });
        }
    }
    // Its read version goes up at the lens's first read only where that read differs from the
    // one it published last.
    let version = country
        .values
        .state(0)
        .and_then(|state| state.last)
        .map_or(0, |run| run.value);
    let same = first_read.as_ref().is_some_and(|(hour, read)| {
        versions.first() == Some(hour) && country.last_read.as_ref() == Some(read)
    });
    let ups = versions[usize::from(same)..]
        .iter()
        .zip(1_i128..)
        .map(|(&hour, up)| (hour, version + up));
    country.values.changes(0, ups, false, false)?;
    let packed = u64::from(identity.0) << 32 | u64::from(identity.1);
    let step = |hour: i32| {
        span.index + u64::try_from(i64::from(hour) - i64::from(first)).unwrap_or(0)
    };
    for leaf in leaves {
        let local = match country.leaves.get(&leaf.leaf) {
            Some(&local) => local,
            None => {
                let local = country.leaves.len() as u64 + 1;
                country.leaves.insert(leaf.leaf, local);
                names.push(((step(leaf.made), 1, packed, local), identity, Some(leaf.leaf)));
                local
            }
        };
        let base = country.totals.get(&leaf.leaf).copied().unwrap_or(0);
        let offset = match corrections.remove(&leaf.leaf) {
            None => {
                let changes = leaf.changes.iter().map(|(hour, total)| (hour, base + total));
                country.values.changes(local, changes, true, true)?;
                0
            }
            Some(fixes) => {
                let held: Vec<(i32, i128)> = leaf.changes.iter().collect();
                let (changes, offset) = mend(&held, &fixes);
                let changes = changes.into_iter().map(|(hour, total)| (hour, base + total));
                country.values.changes(local, changes, true, true)?;
                offset
            }
        };
        country.totals.insert(leaf.leaf, base + leaf.total + offset);
    }
    // A leaf the lens saw no change of, which an amount kept from before changed.
    for (leaf, fixes) in corrections {
        let local = *country.leaves.get(&leaf).ok_or_else(|| {
            io::Error::other("An equipment loss kept from before its segment has no leaf")
        })?;
        let base = country.totals.get(&leaf).copied().unwrap_or(0);
        let (changes, offset) = mend(&[], &fixes);
        let changes = changes.into_iter().map(|(hour, total)| (hour, base + total));
        country.values.changes(local, changes, true, true)?;
        country.totals.insert(leaf, base + offset);
    }
    // The lens's first take of a country it began left its lend-lease read date to the emit.
    let begun = country.born <= span.index;
    let before = country.state.exchanges_read;
    let lend_active = country
        .scanned
        .unread_active
        .contains(&LossRecord::LendLease);
    let carried_read = if begun && !lend_active {
        before.max(span.lend_read)
    } else {
        before
    };
    let resolve = |read: Option<i32>| match read {
        None => carried_read,
        Some(PRIOR) => carried_read.max(span.lend_read),
        read => read,
    };
    let base = country.scanned.ended.len();
    merge(
        &mut country.scanned,
        scanned,
        first,
        country.state.destruction_withheld.as_deref(),
    );
    country.scanned.convoy_drops += drops;
    for prior in prior {
        let seen = country
            .state
            .exchanges
            .get(&prior.exchange)
            .map(|held| held.0)
            .ok_or_else(|| io::Error::other("An ended exchange was never read before"))?;
        let read = if prior.read {
            carried_read.max(span.lend_read)
        } else {
            carried_read
        };
        if let Some(ended) = country.scanned.ended.get_mut(base + prior.index) {
            ended.from = read.map_or(seen, |read| seen.max(read)).saturating_add(1);
        }
    }
    for (key, amount) in kept {
        match key {
            Key::Log(key) => {
                country.state.log.insert(key, amount);
            }
            Key::Convoy(key) => {
                let count = i32::try_from(amount).map_err(io::Error::other)?;
                country.state.convoys.insert(key, count);
            }
            Key::Lend(key) => {
                country.state.lend_lease.insert(key, amount);
            }
        }
    }
    let mut held = BTreeMap::new();
    for (exchange, seen, pools) in exchanges {
        let seen = if seen == PRIOR {
            country
                .state
                .exchanges
                .get(&exchange)
                .map(|held| held.0)
                .ok_or_else(|| io::Error::other("A held exchange was never read before"))?
        } else {
            seen
        };
        held.insert(exchange, (seen, pools.into_iter().collect()));
    }
    country.state.exchanges = held;
    country.state.exchanges_read = resolve(exchanges_read);
    if country.state.destruction_withheld.is_none() {
        country.state.destruction_withheld = withheld;
    }
    country.last_read = last_read;
    country.read_key = read_key;
    country.taken_unresolved = Default::default();
    variants.extend(lost);
    Ok(())
}

impl Prepared {
    /// Takes up the segment's lens as it begins `begun`, before the facts of its first hour
    /// `at`: the countries it binds must stand as they did the hour before.
    pub(super) fn begin(&mut self, begun: &Begun, at: i32) -> io::Result<()> {
        self.drain()?;
        self.abandon()?;
        let live = begun.first == at && !self.interrupted && self.next == i64::from(at);
        let newborns = live
            && binds(
                &self.absent_state.destruction_withheld,
                &self.absent,
                &begun.absent,
                at,
            );
        self.span = Some(Box::new(Span {
            first: at,
            index: self.steps,
            lend_read: self.lend_read,
            absent: self.absent.clone(),
            withheld: self.absent_state.destruction_withheld.clone(),
            live,
            newborns,
        }));
        Ok(())
    }

    /// Binds each country of `begun` whose state is the one the lens began it from.
    pub(super) fn bind(&mut self, begun: Begun) {
        let Some((first, index)) = self
            .span
            .as_deref()
            .filter(|span| span.live)
            .map(|span| (span.first, span.index))
        else {
            return;
        };
        for opened in begun.countries {
            let Some(country) = self.countries.get_mut(&opened.identity) else {
                continue;
            };
            if country.born <= index
                && binds(
                    &country.state.destruction_withheld,
                    &country.scanned,
                    &opened.status,
                    first,
                )
                && holds(&country.state, &opened.seen)
            {
                country.lensed = true;
            }
        }
    }

    /// Adds what the lens made of each bound country, every country the emit's again.
    pub(super) fn splice(&mut self, part: Part) -> io::Result<()> {
        self.drain()?;
        let Some(span) = self.span.take() else {
            return Ok(());
        };
        if span.live {
            for given in part.countries {
                let Some(country) = self.countries.get_mut(&given.identity) else {
                    continue;
                };
                if !std::mem::take(&mut country.lensed) {
                    continue;
                }
                splice_country(&span, given, country, &mut self.names, &mut self.loss_variants)?;
            }
        }
        if self.countries.values().any(|country| country.lensed) {
            return Err(io::Error::other(
                "An equipment loss lens kept a country it bound",
            ));
        }
        self.name()
    }

    /// Ends a lens that never handed its countries back, as where its segment was lost to
    /// damage: the hours it folded of them are missing.
    pub(super) fn abandon(&mut self) -> io::Result<()> {
        let Some(span) = self.span.take() else {
            return Ok(());
        };
        if let Ok(last) = i32::try_from(self.next - 1)
            && span.live
        {
            for country in self.countries.values_mut().filter(|country| country.lensed) {
                country.scanned.advance(span.first.saturating_sub(1));
                country
                    .scanned
                    .missing(span.first, last, LandSource::CombatLog);
            }
        }
        for country in self.countries.values_mut() {
            country.lensed = false;
        }
        self.name()
    }
}
