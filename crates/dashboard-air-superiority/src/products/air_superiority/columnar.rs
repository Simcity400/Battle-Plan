//! Air superiority's operator on a columnar walk ([`replay::segments`]). Each segment's operator
//! keeps its own read of the country groups, observing the group tables as the walk's world lends
//! them at the hours they may have changed, and hands its ordered emit ([`super::emit`]) only
//! what moved: the roster those groups assign at an hour that ends a day; the countries' tags and
//! score owners the segment had not named before; the score pairs that changed, by the
//! segment's owner index, with their operands and active regions; the wing counts that changed
//! and the theatres where they did. Its first hour, and the hour after a gap, give every part
//! whole. An hour is held back until the next shows whether it ends its day. The projection
//! follows the complete scores table row by row; any other status, or a new score identity, is
//! projected whole and handed on as what differs from the pairs handed on before.

use super::{
    DAY_HOURS,
    prepared::Assigned,
    scan::{self, Auxiliary, ScoreOperands},
};
use dashboard_base::{prepared::frame_changes::FrameChanges, products::country_groups::Scan};
use dashboard_core::invalid;
use dashboard_text::metadata::{recorded_rows, slot_reads};
use dashboard_world::{Hold, Needs, World};
use replay::{
    columns,
    segments::{self, Operator, Sink},
};
use replay_data::{
    format::FormatVersion,
    rows::{RaidAirScoreRow, TABLE_COUNT, TableId, tables},
};
use replay_frame::frame::{DecodedFrame, Metrics, not_selected};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    io,
    mem::size_of,
    sync::Arc,
};

/// The tables the operators read.
pub const TABLES: &[TableId] = &[
    TableId::Campaign,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::HumanControllers,
    TableId::Factions,
    TableId::LexicalTokens,
    TableId::FactionMembers,
    TableId::WarRelations,
    TableId::Capitulation,
    TableId::RaidAirScores,
    TableId::AirWings,
    TableId::AirTheatres,
    TableId::AirTheatreGroups,
    TableId::AirTheatreMembers,
];

/// The wing and theatre tables the auxiliary read takes.
const AUXILIARY: [TableId; 4] = [
    TableId::AirWings,
    TableId::AirTheatres,
    TableId::AirTheatreGroups,
    TableId::AirTheatreMembers,
];

/// How the world holds each table read, by the fields read of it.
struct Asks {
    /// The groups' own, as their native read holds them but for capitulations, held by the two
    /// fields the groups read of them; countries and links among them are the score identity's.
    groups: [(TableId, Hold); 8],
    /// The scores by their two weights: country and region are the row's key.
    scores: (TableId, Hold),
    /// Wings by mission region, theatres by name, theatre groups by theatre, rosters by key.
    auxiliary: [(TableId, Hold); 4],
    /// The scores' weight fields, as change-mask bits.
    weights: u64,
    /// The countries' slot and tag fields.
    keys: u64,
}

fn mask(table: TableId, names: &[&str]) -> io::Result<u64> {
    let mut mask = 0_u64;
    for name in names {
        mask |= 1_u64 << columns::field(table, name)?;
    }
    Ok(mask)
}

fn fields(hold: Hold) -> u64 {
    match hold {
        Hold::Whole => u64::MAX,
        Hold::Fields(fields) => fields,
    }
}

impl Asks {
    fn new() -> io::Result<Self> {
        let weights = mask(TableId::RaidAirScores, &["friendly", "enemy"])?;
        let mut groups = dashboard_base::products::country_groups::prepared::holds()?;
        let capitulated = mask(TableId::Capitulation, &["capitulated", "capitulated_hour"])?;
        for (table, hold) in &mut groups {
            if *table == TableId::Capitulation {
                *hold = Hold::Fields(capitulated);
            }
        }
        Ok(Self {
            groups,
            scores: (TableId::RaidAirScores, Hold::Fields(weights)),
            auxiliary: [
                (
                    TableId::AirWings,
                    Hold::Fields(mask(TableId::AirWings, &["mission_region"])?),
                ),
                (
                    TableId::AirTheatres,
                    Hold::Fields(mask(TableId::AirTheatres, &["name"])?),
                ),
                (
                    TableId::AirTheatreGroups,
                    Hold::Fields(mask(TableId::AirTheatreGroups, &["theatre"])?),
                ),
                (TableId::AirTheatreMembers, Hold::Fields(0)),
            ],
            weights,
            keys: mask(TableId::Countries, &["canonical_slot", "tag"])?,
        })
    }

    fn hold(&self, table: TableId) -> Option<(TableId, Hold)> {
        self.groups
            .iter()
            .chain(std::iter::once(&self.scores))
            .chain(&self.auxiliary)
            .find(|(held, _)| *held == table)
            .copied()
    }
}

/// What the operators read of the walk's world.
pub fn needs() -> io::Result<Needs> {
    let asks = Asks::new()?;
    let mut frame = asks.groups.to_vec();
    frame.push(asks.scores);
    frame.extend(asks.auxiliary);
    Ok(Needs {
        frame,
        ..Needs::default()
    })
}

/// A score pair: its owner's index among the segment's owners ([`Air::owners`]), its region, and
/// its friendly and enemy scores.
pub(super) type Scored = (u32, u32, i64, i64);

/// The pairs handed on, in pair order: by owner tag, then region.
#[derive(replay_spool::Spooled)]
pub(super) enum Pairs {
    /// Every pair.
    Whole(Vec<Scored>),
    /// Those new or with other scores than the pairs handed on before, and those gone.
    Changed {
        changed: Vec<Scored>,
        removed: Vec<(u32, u32)>,
    },
}

/// An hour's score projection, where it differs from the one before or is whole.
#[derive(replay_spool::Spooled)]
pub(super) struct Scores {
    /// None where they equal the operands handed on last.
    pub operands: Option<ScoreOperands>,
    pub pairs: Pairs,
    /// The regions any row is active in; None where they are the projection before's.
    pub active: Option<BTreeSet<u32>>,
}

/// What moved of the wings and theatres.
#[derive(replay_spool::Spooled)]
pub(super) struct AuxiliaryChange {
    /// The wing counts below are every region's: those held before are gone.
    pub whole: bool,
    pub wings_known: bool,
    /// Each region whose wing count changed, with its count now; zero where it has none.
    pub wings: Vec<(u32, u32)>,
    /// The theatres' regions and names, where either changed.
    #[allow(clippy::type_complexity)]
    pub theatres: Option<(
        Arc<BTreeMap<String, BTreeSet<u32>>>,
        Arc<BTreeMap<String, Vec<u8>>>,
    )>,
    pub theatres_known: bool,
}

/// What an hour hands the fold ([`super::prepared::Prepared::observe_columnar`]).
#[derive(replay_spool::Spooled)]
pub(super) struct Air {
    /// Every part below is given: the hour starts its segment or follows a gap.
    pub whole: bool,
    /// The hour is its segment's first: the owners its pairs index start anew.
    pub first: bool,
    /// The score owners the segment names, appended since the hour before.
    pub owners: Vec<String>,
    /// The countries' tags the segment had not named before.
    pub countries: Vec<String>,
    pub scores: Option<Box<Scores>>,
    pub auxiliary: Option<Box<AuxiliaryChange>>,
    /// The theatres were read at the hour.
    pub theatres_read: bool,
    /// The roster the groups assign at the hour, where the hour may end its day and the roster
    /// is not the one last handed on.
    pub roster: Option<Box<Assigned>>,
}

/// One hour as a segment's operator projected it.
#[derive(replay_spool::Spooled)]
pub struct Hour {
    pub hour: i32,
    pub(super) air: Air,
    /// The tag sets first handed on at this hour, which stay alive while it is queued.
    retained: usize,
}

/// The score identity the operator projects through: each recorded country's tag, the
/// identity operands, and each score owner's tag by its raw country.
#[derive(PartialEq)]
struct Identity {
    tags: Vec<String>,
    operands: ScoreOperands,
    owners: HashMap<u32, String>,
}

/// A projected pair by its owner's index among the projection's names, and region.
type Pair = (u32, u32);

/// A projection made whole: its operands, every pair in pair order and the active regions.
struct Rebuilt {
    operands: ScoreOperands,
    pairs: Vec<(Pair, (i64, i64))>,
    active: BTreeSet<u32>,
}

/// What a followed hour moved of the projection.
struct Followed {
    operands: ScoreOperands,
    changed: Vec<(Pair, (i64, i64))>,
    removed: Vec<Pair>,
    active: Option<BTreeSet<u32>>,
}

/// The score projection, aggregated from the scores table's rows by the owner each row's
/// country resolves to ([`scan::score_countries`]): a row of a negative score marks its owner
/// invalid, a row no link resolves the projection unresolved, and each owner's pair in a region
/// takes the scores of its highest country's valid row there, as the row observer's projection
/// takes them.
#[derive(Default)]
struct Projection {
    identity_known: bool,
    present: Arc<BTreeSet<String>>,
    /// Each owner tag, ascending: a pair's tag index orders as its tag.
    names: Vec<String>,
    owners: HashMap<u32, u32>,
    /// The table's rows by key while the projection follows a complete table row by row.
    rows: Option<HashMap<u64, (i64, i64)>>,
    witnessed: Vec<u32>,
    invalid: Vec<u32>,
    unresolved: u64,
    /// Each region's rows with a positive score.
    active: HashMap<u32, u32>,
    /// Each pair's valid rows: country, friendly, enemy.
    pairs: HashMap<Pair, Vec<(u32, i64, i64)>>,
    /// A witnessed or invalid count, or an active one, crossed zero since last projected.
    sets_moved: bool,
    active_moved: bool,
    /// The pairs the hour touched, with their scores before it.
    touched: HashMap<Pair, Option<(i64, i64)>>,
    noting: bool,
    witnessed_set: Arc<BTreeSet<String>>,
    invalid_set: Arc<BTreeSet<String>>,
    /// The operands last projected.
    operands: Option<ScoreOperands>,
}

/// One count up or down, whether it crossed zero.
fn count(counter: &mut u32, up: bool) -> io::Result<bool> {
    if up {
        *counter = counter
            .checked_add(1)
            .ok_or_else(|| invalid("Air score count overflow"))?;
        Ok(*counter == 1)
    } else {
        *counter = counter
            .checked_sub(1)
            .ok_or_else(|| invalid("Air score count underflow"))?;
        Ok(*counter == 0)
    }
}

impl Projection {
    /// Projects through `identity` from now on, every count empty.
    fn identify(&mut self, identity: &Identity) {
        let mut names: Vec<String> = identity.owners.values().cloned().collect();
        names.sort_unstable();
        names.dedup();
        self.owners = identity
            .owners
            .iter()
            .map(|(&raw, tag)| {
                let index = names.binary_search(tag).expect("an owner's tag is named");
                (raw, index as u32)
            })
            .collect();
        self.identity_known = identity.operands.identity_known;
        self.present = Arc::clone(&identity.operands.present);
        self.names = names;
        self.clear();
    }

    fn clear(&mut self) {
        self.rows = None;
        self.witnessed.clear();
        self.witnessed.resize(self.names.len(), 0);
        self.invalid.clear();
        self.invalid.resize(self.names.len(), 0);
        self.unresolved = 0;
        self.active.clear();
        self.pairs.clear();
        self.touched.clear();
    }

    /// The pair's scores: those of its highest country's valid row.
    fn value(&self, pair: Pair) -> Option<(i64, i64)> {
        self.pairs.get(&pair).and_then(|rows| {
            rows.iter()
                .max_by_key(|(country, ..)| *country)
                .map(|&(_, friendly, enemy)| (friendly, enemy))
        })
    }

    /// Adds a row (`up`) or takes it away.
    fn apply(
        &mut self,
        country: u32,
        region: u32,
        (friendly, enemy): (i64, i64),
        up: bool,
    ) -> io::Result<()> {
        if friendly > 0 || enemy > 0 {
            let counter = self.active.entry(region).or_default();
            if count(counter, up)? {
                self.active_moved = true;
            }
            if *counter == 0 {
                self.active.remove(&region);
            }
        }
        if !self.identity_known {
            return Ok(());
        }
        let Some(&tag) = self.owners.get(&country) else {
            self.unresolved = if up {
                self.unresolved.checked_add(1)
            } else {
                self.unresolved.checked_sub(1)
            }
            .ok_or_else(|| invalid("Air score count overflow"))?;
            return Ok(());
        };
        let at = tag as usize;
        self.sets_moved |= count(&mut self.witnessed[at], up)?;
        if friendly < 0 || enemy < 0 {
            self.sets_moved |= count(&mut self.invalid[at], up)?;
            return Ok(());
        }
        let pair = (tag, region);
        if self.noting && !self.touched.contains_key(&pair) {
            let before = self.value(pair);
            self.touched.insert(pair, before);
        }
        if up {
            self.pairs.entry(pair).or_default().push((country, friendly, enemy));
        } else if let Some(rows) = self.pairs.get_mut(&pair) {
            let at = rows
                .iter()
                .position(|&row| row == (country, friendly, enemy))
                .ok_or_else(|| invalid("Air score row was not held"))?;
            rows.swap_remove(at);
            if rows.is_empty() {
                self.pairs.remove(&pair);
            }
        } else {
            return Err(invalid("Air score row was not held"));
        }
        Ok(())
    }

    /// Projects `rows` whole, following them row by row from now on when `complete`.
    fn rebuild(&mut self, rows: &[RaidAirScoreRow], complete: bool) -> io::Result<Rebuilt> {
        self.clear();
        self.noting = false;
        for row in rows {
            self.apply(row.country, row.region, (row.friendly, row.enemy), true)?;
        }
        if complete {
            self.rows = Some(
                rows.iter()
                    .map(|row| (key(row), (row.friendly, row.enemy)))
                    .collect(),
            );
        }
        let mut pairs: Vec<_> = self
            .pairs
            .keys()
            .filter_map(|&pair| self.value(pair).map(|scores| (pair, scores)))
            .collect();
        pairs.sort_unstable_by_key(|&(pair, _)| pair);
        self.sets_moved = true;
        self.active_moved = true;
        let operands = self.operands(complete, complete || !rows.is_empty());
        self.operands = Some(operands.clone());
        Ok(Rebuilt {
            operands,
            pairs,
            active: self.regions(),
        })
    }

    /// Follows the complete table's rows the hour changed (`state`'s changes, the weights by
    /// `weights`), read from `rows`, the hour's rows in key order; the projection where it moved.
    fn follow(
        &mut self,
        state: &segments::State,
        rows: &[RaidAirScoreRow],
        weights: u64,
    ) -> io::Result<Option<Followed>> {
        let changes = columns::changes(state, TableId::RaidAirScores)?;
        let table = columns::table(state, TableId::RaidAirScores)?;
        let slot_key = |slot: &u32| table.key(*slot).map(|key| key as u64);
        let mut keys: Vec<u64> = changes
            .changed
            .iter()
            .zip(&changes.masks)
            .filter(|(_, mask)| **mask & weights != 0)
            .filter_map(|(slot, _)| slot_key(slot))
            .chain(changes.inserted.iter().filter_map(slot_key))
            .chain(
                changes
                    .fill_slots
                    .iter()
                    .zip(&changes.fill_masks)
                    .filter(|(_, mask)| **mask & weights != 0)
                    .filter_map(|(slot, _)| slot_key(slot)),
            )
            .chain(changes.deleted_keys.iter().map(|&key| key as u64))
            .collect();
        keys.sort_unstable();
        keys.dedup();
        let mut held = self.rows.take().ok_or_else(|| invalid("Air scores are not followed"))?;
        self.noting = true;
        self.sets_moved = false;
        self.active_moved = false;
        let followed = (|| {
            for key in keys {
                let (country, region) = ((key >> 32) as u32, key as u32);
                let now = rows
                    .binary_search_by_key(&(country, region), |row| (row.country, row.region))
                    .ok()
                    .map(|at| (rows[at].friendly, rows[at].enemy));
                let before = held.get(&key).copied();
                if before == now {
                    continue;
                }
                if let Some(scores) = before {
                    self.apply(country, region, scores, false)?;
                }
                match now {
                    Some(scores) => {
                        self.apply(country, region, scores, true)?;
                        held.insert(key, scores);
                    }
                    None => {
                        held.remove(&key);
                    }
                }
            }
            io::Result::Ok(())
        })();
        self.noting = false;
        followed?;
        if held.len() != rows.len() {
            return Err(invalid("Air scores followed apart from the table"));
        }
        self.rows = Some(held);
        let mut changed = Vec::new();
        let mut removed = Vec::new();
        let mut touched: Vec<_> = self.touched.drain().collect();
        touched.sort_unstable_by_key(|&(pair, _)| pair);
        for (pair, before) in touched {
            let after = self.value(pair);
            if after == before {
                continue;
            }
            match after {
                Some(scores) => changed.push((pair, scores)),
                None => removed.push(pair),
            }
        }
        let operands = self.operands(true, true);
        let moved = self.operands.as_ref() != Some(&operands);
        let active = self.active_moved.then(|| self.regions());
        if !moved && changed.is_empty() && removed.is_empty() && active.is_none() {
            return Ok(None);
        }
        self.operands = Some(operands.clone());
        Ok(Some(Followed {
            operands,
            changed,
            removed,
            active,
        }))
    }

    /// The operands of the projection now.
    fn operands(&mut self, complete: bool, available: bool) -> ScoreOperands {
        if self.sets_moved {
            let named = |counts: &[u32]| -> Arc<BTreeSet<String>> {
                Arc::new(
                    counts
                        .iter()
                        .zip(&self.names)
                        .filter(|(count, _)| **count > 0)
                        .map(|(_, name)| name.clone())
                        .collect(),
                )
            };
            let witnessed = named(&self.witnessed);
            let invalids = named(&self.invalid);
            if *witnessed != *self.witnessed_set {
                self.witnessed_set = witnessed;
            }
            if *invalids != *self.invalid_set {
                self.invalid_set = invalids;
            }
            self.sets_moved = false;
        }
        ScoreOperands {
            identity_known: self.identity_known,
            present: Arc::clone(&self.present),
            witnessed: Arc::clone(&self.witnessed_set),
            invalid: Arc::clone(&self.invalid_set),
            complete,
            unresolved: self.unresolved > 0,
            available,
        }
    }

    /// The active regions now.
    fn regions(&mut self) -> BTreeSet<u32> {
        self.active_moved = false;
        self.active.keys().copied().collect()
    }
}

fn key(row: &RaidAirScoreRow) -> u64 {
    u64::from(row.country) << 32 | u64::from(row.region)
}

/// The score owners a segment's pairs name, by index in the order first named.
#[derive(Default)]
struct Owners {
    tags: Vec<String>,
    index: HashMap<String, u32>,
    /// How many of `tags` were handed on.
    handed: usize,
}

impl Owners {
    fn index(&mut self, tag: &str) -> io::Result<u32> {
        if let Some(&index) = self.index.get(tag) {
            return Ok(index);
        }
        let index =
            u32::try_from(self.tags.len()).map_err(|_| invalid("Air score owner count overflow"))?;
        self.tags.push(tag.to_owned());
        self.index.insert(tag.to_owned(), index);
        Ok(index)
    }

    /// The tags named since last asked.
    fn fresh(&mut self) -> Vec<String> {
        let fresh = self.tags[self.handed..].to_vec();
        self.handed = self.tags.len();
        fresh
    }
}

/// The pairs a segment's operator handed on, by owner index and region, as its emit holds them.
#[derive(Default)]
struct Handed {
    /// Each projection name's owner index.
    owners: Vec<u32>,
    pairs: HashMap<(u32, u32), (i64, i64)>,
}

impl Handed {
    fn scored(&self, ((tag, region), (friendly, enemy)): (Pair, (i64, i64))) -> Scored {
        (self.owners[tag as usize], region, friendly, enemy)
    }

    /// A whole projection, as every pair at a whole hour, or else as what differs from the pairs
    /// handed on before.
    fn rebuilt(&mut self, pairs: Vec<(Pair, (i64, i64))>, whole: bool, owners: &Owners) -> Pairs {
        let pairs: Vec<Scored> = pairs.into_iter().map(|pair| self.scored(pair)).collect();
        let now: HashMap<(u32, u32), (i64, i64)> = pairs
            .iter()
            .map(|&(owner, region, friendly, enemy)| ((owner, region), (friendly, enemy)))
            .collect();
        let before = std::mem::replace(&mut self.pairs, now);
        if whole {
            return Pairs::Whole(pairs);
        }
        let changed = pairs
            .into_iter()
            .filter(|&(owner, region, friendly, enemy)| {
                before.get(&(owner, region)) != Some(&(friendly, enemy))
            })
            .collect();
        let mut removed: Vec<(u32, u32)> = before
            .into_keys()
            .filter(|pair| !self.pairs.contains_key(pair))
            .collect();
        removed.sort_unstable_by(|left, right| {
            (&owners.tags[left.0 as usize], left.1).cmp(&(&owners.tags[right.0 as usize], right.1))
        });
        Pairs::Changed { changed, removed }
    }

    /// The pairs a followed hour moved.
    fn followed(&mut self, changed: Vec<(Pair, (i64, i64))>, removed: Vec<Pair>) -> Pairs {
        let changed: Vec<Scored> = changed.into_iter().map(|pair| self.scored(pair)).collect();
        let removed: Vec<(u32, u32)> = removed
            .into_iter()
            .map(|(tag, region)| (self.owners[tag as usize], region))
            .collect();
        for &(owner, region, friendly, enemy) in &changed {
            self.pairs.insert((owner, region), (friendly, enemy));
        }
        for pair in &removed {
            self.pairs.remove(pair);
        }
        Pairs::Changed { changed, removed }
    }
}

/// The wing counts that differ between `before` and `now`, by region; zero for one gone.
fn wing_changes(before: &BTreeMap<u32, u32>, now: &BTreeMap<u32, u32>) -> Vec<(u32, u32)> {
    let mut changes: Vec<(u32, u32)> = now
        .iter()
        .filter(|&(region, count)| before.get(region) != Some(count))
        .map(|(&region, &count)| (region, count))
        .collect();
    changes.extend(
        before
            .keys()
            .filter(|&region| !now.contains_key(region))
            .map(|&region| (region, 0)),
    );
    changes
}

/// What moved of the wings and theatres `value` reads against `held`, the read before; every
/// part when `whole`. `value` is held from now on, sharing the theatres held where equal.
fn auxiliary_change(
    held: &mut Option<Auxiliary>,
    mut value: Auxiliary,
    whole: bool,
) -> Option<Box<AuxiliaryChange>> {
    let change = match held.as_ref() {
        Some(old) if !whole => {
            let wings = wing_changes(&old.wings, &value.wings);
            let theatres =
                if old.theatres == value.theatres && old.theatre_names == value.theatre_names {
                    value.theatres = Arc::clone(&old.theatres);
                    value.theatre_names = Arc::clone(&old.theatre_names);
                    None
                } else {
                    Some((
                        Arc::clone(&value.theatres),
                        Arc::clone(&value.theatre_names),
                    ))
                };
            let moved = !wings.is_empty()
                || theatres.is_some()
                || old.wings_known != value.wings_known
                || old.theatres_known != value.theatres_known;
            moved.then(|| AuxiliaryChange {
                whole: false,
                wings_known: value.wings_known,
                wings,
                theatres,
                theatres_known: value.theatres_known,
            })
        }
        _ => Some(AuxiliaryChange {
            whole: true,
            wings_known: value.wings_known,
            wings: value
                .wings
                .iter()
                .map(|(&region, &count)| (region, count))
                .collect(),
            theatres: Some((
                Arc::clone(&value.theatres),
                Arc::clone(&value.theatre_names),
            )),
            theatres_known: value.theatres_known,
        }),
    };
    *held = Some(value);
    change.map(Box::new)
}

/// One segment's operator.
pub struct Project {
    format: FormatVersion,
    asks: Asks,
    /// The hour's asks, kept from hour to hour.
    wanted: Vec<(TableId, Hold)>,
    /// The frame the world's views are lent into, given back before its hour ends.
    frame: DecodedFrame,
    last: Option<i32>,
    identity: Option<Identity>,
    projection: Projection,
    owners: Owners,
    handed: Handed,
    /// The countries' tags the segment named.
    countries: HashSet<String>,
    auxiliary: Option<Auxiliary>,
    /// The segment's own read of the country groups.
    groups: Box<Scan>,
    /// The recorded wars' later ends, latest first: the groups change there with no row changing.
    expiries: Vec<i32>,
    /// The groups or the present countries may have moved since the roster was last read.
    roster_moved: bool,
    /// The roster last handed on.
    roster: Option<Assigned>,
    /// The hour projected last, held until the next shows whether it ends its day.
    held: Option<Hour>,
    /// No hour was projected yet.
    first: bool,
    /// The score operands last handed on.
    handed_operands: Option<ScoreOperands>,
}

/// What a tag set keeps alive.
fn set_bytes(set: &BTreeSet<String>) -> usize {
    set.len() * (size_of::<String>() + 16) + set.iter().map(String::capacity).sum::<usize>()
}

impl Project {
    pub fn new(format: FormatVersion) -> io::Result<Self> {
        Ok(Self {
            format,
            asks: Asks::new()?,
            wanted: Vec::new(),
            frame: DecodedFrame {
                branch: 0,
                hour: 0,
                tables: std::array::from_fn(|_| not_selected()),
                metrics: Metrics::default(),
            },
            last: None,
            identity: None,
            projection: Projection::default(),
            owners: Owners::default(),
            handed: Handed::default(),
            countries: HashSet::new(),
            auxiliary: None,
            groups: Box::new(Scan::within(None)),
            expiries: Vec::new(),
            roster_moved: true,
            roster: None,
            held: None,
            first: true,
            handed_operands: None,
        })
    }

    /// Reads the roster at `held`'s hour, the groups' last observed, where it may have moved,
    /// and hands it on with that hour where it is not the one last handed on.
    fn close(&mut self, held: &mut Hour) {
        if !self.roster_moved {
            return;
        }
        self.roster_moved = false;
        self.groups.finish_within(None);
        let assigned = Assigned::read(
            &self.groups,
            held.hour,
            self.projection.identity_known,
            &self.projection.present,
        );
        if self.roster.as_ref() != Some(&assigned) {
            held.air.roster = Some(Box::new(assigned.clone()));
            self.roster = Some(assigned);
        }
    }
}

fn want(wanted: &mut Vec<(TableId, Hold)>, asks: &Asks, table: TableId) {
    if !wanted.iter().any(|(held, _)| *held == table)
        && let Some(ask) = asks.hold(table)
    {
        wanted.push(ask);
    }
}

impl Operator<World> for Project {
    type Output = Hour;

    fn cross(&mut self, _: i32) -> io::Result<()> {
        // A later run applies to empty tables: its first hour reads every part whole.
        self.last = None;
        Ok(())
    }

    fn hour(
        &mut self,
        hour: &segments::Hour<'_, World>,
        out: &mut dyn Sink<Hour>,
    ) -> io::Result<()> {
        let _cpu = ::replay_frame::cpu::scope("air_superiority.columnar", None);
        let (state, world, at) = (hour.state, hour.world, hour.hour);
        // The hour held back ends its day where this one starts another: its roster is read
        // before this hour moves the groups.
        if let Some(mut held) = self.held.take() {
            if held.hour.div_euclid(DAY_HOURS) != at.div_euclid(DAY_HOURS) {
                self.close(&mut held);
            }
            out.push(held)?;
        }
        let fresh = hour.starts || self.last.and_then(|last| last.checked_add(1)) != Some(at);
        let whole = fresh || hour.opens;
        self.last = Some(at);
        let opens = hour.starts || hour.opens;
        if hour.starts {
            self.groups.begin_block();
        }
        // The row decoder reads a table it cannot read whole anew every hour.
        let moved = |table: TableId, hold: Hold| {
            world.touched(table, fields(hold))
                || columns::status(state, table) != columns::Status::Complete
        };
        let lexical = world.metrics().table_ns[TableId::LexicalTokens.index()] != 0;
        let mut changes = FrameChanges {
            rows_changed: [false; TABLE_COUNT],
            table_read: [false; TABLE_COUNT],
            slot_read: [false; TABLE_COUNT],
            updates: None,
        };
        changes.table_read[TableId::LexicalTokens.index()] = lexical;
        let mut groups_moved = false;
        for &(table, hold) in &self.asks.groups {
            if opens || moved(table, hold) {
                changes.rows_changed[table.index()] = true;
                groups_moved = true;
            }
        }
        let expired = self.expiries.last().is_some_and(|&end| end <= at);
        // The groups read their tables only at the hours they do not pass over unchanged: a
        // group table changed or the dictionary read, a gap, or a war ending.
        let read_groups = groups_moved || fresh || lexical || expired;
        let identity = whole
            || world.touched(TableId::Countries, self.asks.keys)
            || world.touched(TableId::CountryLinks, u64::MAX);
        let scored = whole || identity || world.touched(TableId::RaidAirScores, self.asks.weights);
        let auxiliary = whole
            || self
                .asks
                .auxiliary
                .iter()
                .any(|&(table, hold)| world.touched(table, fields(hold)));
        self.wanted.clear();
        if read_groups {
            self.wanted.extend_from_slice(&self.asks.groups);
        }
        if identity {
            want(&mut self.wanted, &self.asks, TableId::Countries);
            want(&mut self.wanted, &self.asks, TableId::CountryLinks);
        }
        if scored {
            want(&mut self.wanted, &self.asks, TableId::RaidAirScores);
        }
        if auxiliary {
            for table in AUXILIARY {
                want(&mut self.wanted, &self.asks, table);
            }
        }
        self.frame.branch = hour.branch;
        self.frame.hour = at;
        let mut countries = Vec::new();
        let mut scores = None;
        let mut moved_auxiliary = None;
        // Where nothing the groups read moved, they pass over the hour without reading a table.
        let unlent = || {
            io::Error::other("Air superiority's groups changed at an hour their tables were not lent")
        };
        if self.wanted.is_empty() {
            if self.groups.advance_changed(&self.frame, &changes)?.is_some() {
                return Err(unlent());
            }
        } else {
            // The views go back before the hour ends, however it ends.
            let lent = Lent {
                frame: &mut self.frame,
                asks: self.wanted.as_slice(),
            };
            world.frame_held_into(state, lent.asks, lent.frame)?;
            let frame: &DecodedFrame = &*lent.frame;
            if self.groups.advance_changed(frame, &changes)?.is_some() {
                if !read_groups {
                    return Err(unlent());
                }
                self.roster_moved = true;
            }
            if changes.rows_changed[TableId::WarRelations.index()] {
                let mut ends: Vec<i32> = frame
                    .complete::<tables::WarRelations>()
                    .map(|rows| {
                        rows.iter()
                            .filter(|row| row.end_hour > row.start_hour && row.end_hour > at)
                            .map(|row| row.end_hour)
                            .collect()
                    })
                    .unwrap_or_default();
                ends.sort_unstable_by(|left, right| right.cmp(left));
                ends.dedup();
                self.expiries = ends;
            }
            let mut identified = false;
            if identity {
                let (operands, owners) = scan::score_countries(frame)?;
                let rows = frame.complete::<tables::Countries>().unwrap_or(&[]);
                let mut tags = Vec::with_capacity(rows.len());
                for country in rows {
                    let tag = std::str::from_utf8(&country.tag)
                        .map_err(|_| invalid("Air country tag is not UTF-8"))?;
                    dashboard_core::protocol::validate_tag(tag).map_err(invalid)?;
                    tags.push(tag.to_owned());
                }
                let read = Identity {
                    tags,
                    operands,
                    owners,
                };
                if whole || self.identity.as_ref() != Some(&read) {
                    for tag in &read.tags {
                        if self.countries.insert(tag.clone()) {
                            countries.push(tag.clone());
                        }
                    }
                    self.projection.identify(&read);
                    self.handed.owners = self
                        .projection
                        .names
                        .iter()
                        .map(|name| self.owners.index(name))
                        .collect::<io::Result<_>>()?;
                    self.identity = Some(read);
                    identified = true;
                    self.roster_moved = true;
                }
            }
            if whole || identified || world.touched(TableId::RaidAirScores, self.asks.weights) {
                let complete = frame.complete::<tables::RaidAirScores>().is_ok();
                if whole
                    || identified
                    || !complete
                    || self.projection.rows.is_none()
                    || world.restated(TableId::RaidAirScores)
                {
                    let rows = recorded_rows::<tables::RaidAirScores>(frame)?;
                    let rebuilt = self.projection.rebuild(rows, complete)?;
                    scores = Some(Box::new(Scores {
                        operands: Some(rebuilt.operands),
                        pairs: self.handed.rebuilt(rebuilt.pairs, whole, &self.owners),
                        active: Some(rebuilt.active),
                    }));
                } else {
                    let rows = frame.complete::<tables::RaidAirScores>()?;
                    let followed = self.projection.follow(state, rows, self.asks.weights)?;
                    if let Some(followed) = followed {
                        scores = Some(Box::new(Scores {
                            operands: Some(followed.operands),
                            pairs: self.handed.followed(followed.changed, followed.removed),
                            active: followed.active,
                        }));
                    }
                }
            }
            if auxiliary {
                moved_auxiliary =
                    auxiliary_change(&mut self.auxiliary, scan::auxiliary(frame), whole);
            }
        }
        while self.expiries.last().is_some_and(|&end| end <= at) {
            self.expiries.pop();
        }
        let theatres_read =
            slot_reads(self.format, world.metrics(), at)[TableId::AirTheatres.index()];
        // Operands go on only where they changed, so the one hour holding their sets is charged.
        let mut retained = 0;
        if let Some(scores) = &mut scores {
            if scores.operands.as_ref() == self.handed_operands.as_ref() {
                scores.operands = None;
            } else if let Some(operands) = &scores.operands {
                retained = set_bytes(&operands.present)
                    + set_bytes(&operands.witnessed)
                    + set_bytes(&operands.invalid);
                self.handed_operands = Some(operands.clone());
            }
        }
        self.held = Some(Hour {
            hour: at,
            retained,
            air: Air {
                whole,
                first: std::mem::take(&mut self.first),
                owners: self.owners.fresh(),
                countries,
                scores,
                auxiliary: moved_auxiliary,
                theatres_read,
                roster: None,
            },
        });
        Ok(())
    }

    fn end(&mut self, out: &mut dyn Sink<Hour>) -> io::Result<()> {
        // The segment's last hour may end its day: the segment after cannot read its roster.
        if let Some(mut held) = self.held.take() {
            self.close(&mut held);
            out.push(held)?;
        }
        Ok(())
    }
}

/// The world's views lent into `frame` for `asks`, given back when dropped.
struct Lent<'o> {
    frame: &'o mut DecodedFrame,
    asks: &'o [(TableId, Hold)],
}

impl Drop for Lent<'_> {
    fn drop(&mut self) {
        dashboard_world::release(self.frame, self.asks);
    }
}
