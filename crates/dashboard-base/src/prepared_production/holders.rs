//! What production keeps of each tag, and the steps an hour takes on each. Every step reads
//! one holder and what the hour shares, so the emit runs them here and on the run's idle
//! workers ([`super::shards`]) in contiguous runs of tags and applies what they give in tag
//! order.

use super::{
    Amount, CountryEndpoint, CoverageCorrection, DesignKeys, IC_SEEN, PurposeKey, RowEdit, SEEN,
    SERIES_PURPOSES, SpendingSource, SpentKey, UNITS_SEEN, VariantInventory, design_amounts,
    design_inventory, fold::Fold, known_sum, next_key, source_edit, spending_reason_from,
    take_totals,
};
use crate::prepared::series::Accumulator as Series;
use crate::products::equipment::production::{
    CostTotal, Edits, Entry, INVENTORY_READER, Identity, ProductionSpendingUnknown, Purpose,
    RocketTotals, SPENT_READER, Total,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
};

/// A track's last sample: the read it was taken at.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub(super) struct Sample {
    pub hour: i32,
    /// It starts a line: the read before it is not of the same identity, or not the hour before.
    pub starts: bool,
    pub identity: Identity,
    pub presence: Option<bool>,
}

/// One tag's reads of an output: its last sample, and the totals it took there.
#[derive(Default, Deserialize, Serialize)]
pub(super) struct Track {
    pub sample: Option<Sample>,
    /// The identity it holds through the accumulator's last hour; none once it stopped.
    pub live: Option<Identity>,
    #[serde(with = "crate::prepared_production::map_pairs")]
    pub current: BTreeMap<Entry, Total>,
    #[serde(with = "crate::prepared_production::map_pairs")]
    pub cost_current: BTreeMap<Entry, CostTotal>,
    pub taken: Option<(Identity, u64)>,
    pub cost_taken: Option<(Identity, u64)>,
    /// The last version the Rocket inventories took.
    #[serde(default)]
    pub version: u64,
    #[serde(skip)]
    pub edits: Edits,
}

/// A holder's spending projection of one source, as it last projected.
pub(super) struct SpentHolder {
    pub inputs: super::SpentInputs,
    pub active: BTreeSet<u64>,
}

/// Everything production keeps of one tag.
#[derive(Default, Deserialize, Serialize)]
pub(super) struct Holder {
    /// The countries name it.
    #[serde(skip)]
    pub listed: bool,
    /// Its canonical slot and id, where the countries resolve it.
    #[serde(skip)]
    pub country: Option<(u32, u32)>,
    /// No tag before it in tag order resolves to its slot and id.
    #[serde(skip)]
    pub first_of_country: bool,
    /// Its place in the facts' holder table, once it has one.
    #[serde(skip)]
    pub id: Option<u32>,
    /// The equipment track, then the Rocket track.
    pub tracks: [Option<Track>; 2],
    #[serde(skip)]
    pub spent: [Option<SpentHolder>; 2],
    pub inventory: [Option<VariantInventory>; 2],
    /// The slot and id each source was first projected with.
    pub first: [Option<(u32, u32)>; 2],
    /// Each source's continuity count.
    pub segments: [Option<u64>; 2],
    pub sources: [Option<SpendingSource>; 2],
    pub bound: Option<super::RocketBound>,
    #[serde(skip)]
    pub keys: HashMap<(bool, u64), DesignKeys>,
}

/// What steps counted, for tests' work bounds.
#[derive(Clone, Copy, Default)]
pub(super) struct Visits {
    pub history: usize,
    pub emitted: usize,
    pub spent: usize,
}

impl Visits {
    pub fn add(&mut self, other: Self) {
        self.history += other.history;
        self.emitted += other.emitted;
        self.spent += other.spent;
    }
}

/// An equipment read every holder samples.
pub(super) struct EquipmentRead<'a> {
    pub fold: &'a Fold,
    pub observation: u32,
    /// The fold's number for the read.
    pub number: u64,
    pub census_complete: bool,
    pub presence_unknown: bool,
}

/// A Rocket read every holder samples.
pub(super) struct RocketSample<'a> {
    pub totals: &'a RocketTotals,
    pub observation: u32,
}

/// What every holder's spending projection reads.
pub(super) struct SpendingRead<'a> {
    pub tainted_from: Option<(i32, ProductionSpendingUnknown)>,
    pub restarts: [Option<i32>; 2],
    /// The projections held through the hour before.
    pub consecutive: bool,
    pub spent: &'a Series,
    pub active: &'a BTreeSet<u64>,
    pub keys: Held<'a>,
    pub ids: &'a BTreeMap<String, u32>,
}

/// What every holder reads at an hour.
pub(super) struct Shared<'a> {
    pub hour: i32,
    /// The hour follows the last the accumulator took.
    pub consecutive: bool,
    pub equipment: Option<EquipmentRead<'a>>,
    pub rocket: Option<RocketSample<'a>>,
    pub spending: Option<SpendingRead<'a>>,
}

/// The series key maps, read only.
#[derive(Clone, Copy)]
pub(super) struct Held<'a> {
    pub spent: &'a BTreeMap<SpentKey, u64>,
    pub unproven: &'a BTreeMap<PurposeKey, u64>,
    pub purposes: &'a BTreeMap<PurposeKey, u64>,
}

/// The series key maps, read only or making keys.
pub(super) enum Maps<'a> {
    /// A key not made yet is left to the ordered pass.
    Held(Held<'a>),
    Making {
        spent: &'a mut BTreeMap<SpentKey, u64>,
        unproven: &'a mut BTreeMap<PurposeKey, u64>,
        purposes: &'a mut BTreeMap<PurposeKey, u64>,
    },
}

impl Maps<'_> {
    fn spent(&self) -> &BTreeMap<SpentKey, u64> {
        match self {
            Self::Held(held) => held.spent,
            Self::Making { spent, .. } => spent,
        }
    }

    fn unproven(&self) -> &BTreeMap<PurposeKey, u64> {
        match self {
            Self::Held(held) => held.unproven,
            Self::Making { unproven, .. } => unproven,
        }
    }

    fn purposes(&self) -> &BTreeMap<PurposeKey, u64> {
        match self {
            Self::Held(held) => held.purposes,
            Self::Making { purposes, .. } => purposes,
        }
    }
}

/// One holder's series keys: spent, unproven and purpose keys share one numbering, and keys
/// are only ever added, so a design's keys stay what the maps hold.
pub(super) struct SeriesKeys<'a> {
    pub maps: Maps<'a>,
    pub holder: u32,
    pub designs: &'a mut HashMap<(bool, u64), DesignKeys>,
}

impl SeriesKeys<'_> {
    fn next(&self) -> u64 {
        next_key(
            self.maps.spent(),
            self.maps.unproven(),
            self.maps.purposes(),
        )
    }

    /// `variant`'s keys, read from the maps at its first need.
    fn design(&mut self, rocket: bool, variant: u64) -> &mut DesignKeys {
        let Self {
            maps,
            holder,
            designs,
        } = self;
        let holder = *holder;
        designs
            .entry((rocket, variant))
            .or_insert_with(|| DesignKeys {
                spent: [false, true].map(|refits| {
                    maps.spent()
                        .get(&(holder, rocket, refits, variant))
                        .copied()
                }),
                purposes: SERIES_PURPOSES.map(|purpose| {
                    (!rocket)
                        .then(|| maps.purposes().get(&(holder, variant, purpose)).copied())
                        .flatten()
                }),
                unproven: Purpose::ALL.map(|purpose| {
                    (!rocket)
                        .then(|| maps.unproven().get(&(holder, variant, purpose)).copied())
                        .flatten()
                }),
            })
    }

    /// Every key a projection of `variant` may hold.
    fn of(&mut self, rocket: bool, variant: u64) -> impl Iterator<Item = u64> + use<> {
        let design = *self.design(rocket, variant);
        design
            .spent
            .into_iter()
            .chain(design.purposes)
            .chain(design.unproven)
            .flatten()
    }

    /// The spent key of (`rocket`, `refits`, `variant`), the next one where it has none; none
    /// where it has none and these maps make none.
    fn spent_key(&mut self, rocket: bool, refits: bool, variant: u64) -> Option<u64> {
        if let Some(key) = self.design(rocket, variant).spent[usize::from(refits)] {
            return Some(key);
        }
        let next = self.next();
        let holder = self.holder;
        let Maps::Making { spent, .. } = &mut self.maps else {
            return None;
        };
        let key = *spent
            .entry((holder, rocket, refits, variant))
            .or_insert(next);
        self.design(rocket, variant).spent[usize::from(refits)] = Some(key);
        Some(key)
    }

    /// The key of equipment design `variant`'s input of `purpose`, one of [`SERIES_PURPOSES`],
    /// as [`Self::spent_key`] gives it.
    fn purpose_key(&mut self, variant: u64, purpose: Purpose) -> io::Result<Option<u64>> {
        let at = SERIES_PURPOSES
            .iter()
            .position(|&series| series == purpose)
            .ok_or_else(|| io::Error::other("A purpose without its own series"))?;
        if let Some(key) = self.design(false, variant).purposes[at] {
            return Ok(Some(key));
        }
        let next = self.next();
        let holder = self.holder;
        let Maps::Making { purposes, .. } = &mut self.maps else {
            return Ok(None);
        };
        let key = *purposes.entry((holder, variant, purpose)).or_insert(next);
        self.design(false, variant).purposes[at] = Some(key);
        Ok(Some(key))
    }

    /// Equipment design `variant`'s unproven key of `purpose`, if it has one.
    fn unproven(&mut self, variant: u64, purpose: Purpose) -> Option<u64> {
        self.design(false, variant).unproven[purpose.index()]
    }

    /// Equipment design `variant`'s unproven key of `purpose`, as [`Self::spent_key`] gives it.
    fn unproven_key(&mut self, variant: u64, purpose: Purpose) -> Option<u64> {
        if let Some(key) = self.unproven(variant, purpose) {
            return Some(key);
        }
        let next = self.next();
        let holder = self.holder;
        let Maps::Making { unproven, .. } = &mut self.maps else {
            return None;
        };
        let key = *unproven.entry((holder, variant, purpose)).or_insert(next);
        self.design(false, variant).unproven[purpose.index()] = Some(key);
        Some(key)
    }
}

/// What a holder's spending projection of one source observes: each key's value where it is
/// new or changed, and the keys it opens and closes.
#[derive(Default)]
pub(super) struct Projection {
    pub observed: Vec<(u64, Option<i128>, bool)>,
    pub opened: Vec<u64>,
    pub closed: Vec<u64>,
}

/// What a projection read of its holder and track before projecting.
pub(super) struct Gathered {
    inputs: super::SpentInputs,
    starts: bool,
    whole: bool,
    entries: Vec<Entry>,
    partial: bool,
}

/// A holder's spending projection of one source at an hour.
pub(super) enum Spending {
    /// Not projected: the keys it held close.
    Closed(Vec<u64>),
    /// Its inputs held: it goes on as it was.
    Held,
    Projected(Projection),
    /// It makes keys, which only the ordered pass does.
    Ordered(Gathered),
}

/// What one holder's step gave at an hour.
pub(super) struct Observed {
    pub spending: [Spending; 2],
}

/// Projects one design of a holder at an hour: its spent series with and without refits, and
/// for equipment its purpose and unproven series, noting in `known` each key it holds a known
/// value of. Returns how many observations it made; none where it needs a key `series` does
/// not make.
#[allow(clippy::too_many_arguments)]
fn project_design(
    series: &mut SeriesKeys<'_>,
    spent: &Series,
    active: &BTreeSet<u64>,
    known: &mut Vec<u64>,
    observed: &mut Vec<(u64, Option<i128>, bool)>,
    (rocket, variant): (bool, u64),
    amounts: &[Option<CostTotal>; Purpose::ALL.len()],
    (scope_reason, starts): (Option<ProductionSpendingUnknown>, bool),
) -> io::Result<Option<usize>> {
    let mut emitted = 0;
    for refits in if rocket {
        &[true][..]
    } else {
        &[false, true][..]
    } {
        // Without refits: designs with input of another purpose.
        if !*refits
            && Purpose::ALL
                .into_iter()
                .filter(|&purpose| purpose != Purpose::Refit)
                .all(|purpose| amounts[purpose.index()].is_none())
        {
            continue;
        }
        let Some(key) = series.spent_key(rocket, *refits, variant) else {
            return Ok(None);
        };
        let counted = Purpose::ALL
            .into_iter()
            .filter(|purpose| *refits || *purpose != Purpose::Refit)
            .filter_map(|purpose| amounts[purpose.index()].as_ref());
        let value = known_sum(scope_reason, counted)?;
        emitted += usize::from(observe_known(
            spent,
            active,
            known,
            observed,
            (key, value, starts),
        ));
    }
    if rocket {
        return Ok(Some(emitted));
    }
    // Converted and unrecorded-purpose input, for designs that ever had it: months read each
    // apart from new production in the series without refits.
    for purpose in SERIES_PURPOSES {
        let Some(total) = &amounts[purpose.index()] else {
            continue;
        };
        let Some(key) = series.purpose_key(variant, purpose)? else {
            return Ok(None);
        };
        let value = known_sum(scope_reason, [total])?;
        emitted += usize::from(observe_known(
            spent,
            active,
            known,
            observed,
            (key, value, starts),
        ));
    }
    // Input whose purpose is not recorded, per purpose, for entries that ever had it: ranges
    // and months read its change over their own interval.
    for (purpose, total) in Purpose::ALL
        .into_iter()
        .filter_map(|purpose| amounts[purpose.index()].map(|total| (purpose, total)))
    {
        if total.unproven_raw == 0 && series.unproven(variant, purpose).is_none() {
            continue;
        }
        let Some(key) = series.unproven_key(variant, purpose) else {
            return Ok(None);
        };
        let value = scope_reason
            .or(total.reason)
            .is_none()
            .then_some(total.unproven_raw);
        emitted += usize::from(observe_known(
            spent,
            active,
            known,
            observed,
            (key, value, starts),
        ));
    }
    Ok(Some(emitted))
}

/// Observes a known value of a holder's key where it is new or changed; true where it does.
/// A known value keeps the key active.
fn observe_known(
    spent: &Series,
    active: &BTreeSet<u64>,
    known: &mut Vec<u64>,
    observed: &mut Vec<(u64, Option<i128>, bool)>,
    (key, value, starts): (u64, Option<i128>, bool),
) -> bool {
    if value.is_none() {
        return false;
    }
    known.push(key);
    if starts
        || !active.contains(&key)
        || spent
            .state(key)
            .and_then(|state| state.last)
            .is_none_or(|run| Some(run.value) != value)
    {
        observed.push((key, value, starts));
        return true;
    }
    false
}

/// `gathered`'s projection of `track`, whose last projection was `held`: the keys it holds
/// after, and what it observes; none where it needs a key `series` does not make.
#[allow(clippy::too_many_arguments)]
fn project(
    track: &Track,
    held: Option<&SpentHolder>,
    gathered: &Gathered,
    series: &mut SeriesKeys<'_>,
    spent: &Series,
    active: &BTreeSet<u64>,
    rocket: bool,
    visits: &mut Visits,
) -> io::Result<Option<(BTreeSet<u64>, Projection)>> {
    let mut projection = Projection::default();
    let context = (gathered.inputs.reason, gathered.starts);
    let mut keys = Vec::new();
    if gathered.partial {
        let held = held.expect("a partial projection's last");
        let mut now = held.active.clone();
        let mut variants: Vec<u64> = gathered.entries.iter().map(|entry| entry.0).collect();
        variants.dedup();
        for variant in variants {
            let old: Vec<u64> = series
                .of(rocket, variant)
                .filter(|key| now.remove(key))
                .collect();
            keys.clear();
            if let Some(amounts) = design_amounts(track, variant, &mut visits.spent) {
                let Some(emitted) = project_design(
                    series,
                    spent,
                    active,
                    &mut keys,
                    &mut projection.observed,
                    (rocket, variant),
                    &amounts,
                    context,
                )?
                else {
                    return Ok(None);
                };
                visits.emitted += emitted;
            }
            for &key in &keys {
                now.insert(key);
                projection.opened.push(key);
            }
            projection
                .closed
                .extend(old.into_iter().filter(|key| !keys.contains(key)));
        }
        return Ok(Some((now, projection)));
    }
    let mut amounts = Vec::new();
    design_inventory(track, &mut amounts, &mut visits.spent);
    for (variant, amounts) in &amounts {
        let Some(emitted) = project_design(
            series,
            spent,
            active,
            &mut keys,
            &mut projection.observed,
            (rocket, *variant),
            amounts,
            context,
        )?
        else {
            return Ok(None);
        };
        visits.emitted += emitted;
    }
    let now: BTreeSet<u64> = keys.iter().copied().collect();
    projection.opened.extend(now.iter().copied());
    if let Some(held) = held {
        projection
            .closed
            .extend(held.active.difference(&now).copied());
    }
    Ok(Some((now, projection)))
}

impl Holder {
    /// Whether the countries name it, and how they resolve it.
    pub fn list(&mut self, country: Option<Option<(u32, u32)>>) {
        self.listed = country.is_some();
        self.country = country.flatten();
    }

    /// The identity a read of `observation` gives it: none where the countries do not name or
    /// resolve it.
    fn identity(&self, observation: u32) -> Option<Identity> {
        self.country
            .filter(|_| self.listed)
            .map(|(slot, id)| (slot, id, observation))
    }

    /// Ends source `source`'s read.
    pub fn end(&mut self, source: usize) {
        if let Some(track) = &mut self.tracks[source] {
            track.live = None;
        }
    }

    /// Source `source`'s read goes on at an hour that reads nothing, while its identity in
    /// `observation` holds and the hour follows the one before.
    pub fn carry(&mut self, source: usize, observation: u32, consecutive: bool) {
        let identity = self.identity(observation);
        if let Some(track) = &mut self.tracks[source] {
            track.live = track
                .live
                .filter(|live| consecutive && Some(*live) == identity);
        }
    }

    /// The hour's samples and spending projections, as far as they need no new series key.
    pub fn observe(
        &mut self,
        tag: &str,
        shared: &Shared<'_>,
        visits: &mut Visits,
    ) -> io::Result<Option<Observed>> {
        if let Some(rocket) = &shared.rocket {
            self.sample_rocket(shared, rocket);
        }
        if let Some(equipment) = &shared.equipment {
            self.sample_equipment(shared, equipment);
        }
        let Some(read) = &shared.spending else {
            return Ok(None);
        };
        let mut spending = [Spending::Held, Spending::Held];
        for (source, spending) in spending.iter_mut().enumerate() {
            *spending = match self.gather(source, read, shared.hour, visits) {
                Ok(gathered) => self.project_held(tag, source, gathered, read, visits)?,
                Err(spending) => spending,
            };
        }
        Ok(Some(Observed { spending }))
    }

    fn sample_equipment(&mut self, shared: &Shared<'_>, read: &EquipmentRead<'_>) {
        let Some(identity) = self.identity(read.observation) else {
            self.end(0);
            return;
        };
        let track = self.tracks[0].get_or_insert_with(Track::default);
        let starts = !(shared.consecutive && track.live == Some(identity));
        let totals = read.fold.totals(identity);
        let presence = if totals.is_some_and(|totals| totals.seen) {
            Some(true)
        } else if read.census_complete && !read.presence_unknown {
            Some(false)
        } else {
            None
        };
        track.sample = Some(Sample {
            hour: shared.hour,
            starts,
            identity,
            presence,
        });
        track.live = Some(identity);
        let costs = totals.map(|totals| &totals.costs);
        let cost_version = costs.map_or(0, |costs| costs.version);
        if track.cost_taken != Some((identity, cost_version)) {
            take_totals(
                &mut track.cost_current,
                &mut track.edits,
                track.cost_taken,
                identity,
                costs,
                read.number,
            );
            track.cost_taken = Some((identity, cost_version));
        }
        let units = totals.map(|totals| &totals.units);
        let version = units.map_or(0, |units| units.version);
        if track.taken != Some((identity, version)) {
            take_totals(
                &mut track.current,
                &mut track.edits,
                track.taken,
                identity,
                units,
                read.number,
            );
            track.taken = Some((identity, version));
        }
    }

    fn sample_rocket(&mut self, shared: &Shared<'_>, read: &RocketSample<'_>) {
        let Some(identity) = self.identity(read.observation) else {
            self.end(1);
            return;
        };
        let first = self.first_of_country;
        let track = self.tracks[1].get_or_insert_with(Track::default);
        let live = track.live.filter(|_| !read.totals.restarted);
        let starts = !(shared.consecutive && live == Some(identity));
        let (units, costs) = read.totals.of(identity, first);
        track.sample = Some(Sample {
            hour: shared.hour,
            starts,
            identity,
            presence: read.totals.presence(identity),
        });
        track.live = Some(identity);
        let mut units_changed = track.taken.is_none_or(|(taken, _)| taken != identity);
        let mut costs_changed = track.cost_taken.is_none_or(|(taken, _)| taken != identity);
        if units_changed || costs_changed {
            track.edits.replaced();
        }
        let keys: BTreeSet<Entry> = track.current.keys().chain(units.keys()).copied().collect();
        for key in keys {
            if track.current.get(&key) != units.get(&key) {
                units_changed = true;
                track.edits.note(key);
            }
        }
        let keys: BTreeSet<Entry> = track
            .cost_current
            .keys()
            .chain(costs.keys())
            .copied()
            .collect();
        for key in keys {
            if track.cost_current.get(&key) != costs.get(&key) {
                costs_changed = true;
                track.edits.note(key);
            }
        }
        track.current = units;
        track.cost_current = costs;
        for (changed, taken) in [
            (units_changed, &mut track.taken),
            (costs_changed, &mut track.cost_taken),
        ] {
            if changed {
                track.version += 1;
                *taken = Some((identity, track.version));
            }
        }
    }

    /// What source `source`'s spending projection reads at `hour`, or how it ends without one.
    fn gather(
        &mut self,
        source: usize,
        read: &SpendingRead<'_>,
        hour: i32,
        visits: &mut Visits,
    ) -> Result<Gathered, Spending> {
        let rocket = source == 1;
        let live = match &self.tracks[source] {
            Some(track) if self.listed => {
                visits.history += 1;
                track.live
            }
            _ => None,
        };
        let Some(identity) = live else {
            let closed = self.spent[source]
                .take()
                .map(|held| held.active.into_iter().collect())
                .unwrap_or_default();
            return Err(Spending::Closed(closed));
        };
        let first = *self.first[source].get_or_insert((identity.0, identity.1));
        let reason = spending_reason_from(
            read.tainted_from,
            Some(first),
            (identity.0, identity.1),
            read.restarts[source],
            rocket,
            hour,
        );
        let track = self.tracks[source].as_mut().expect("a live track");
        let starts = track
            .sample
            .is_some_and(|sample| sample.starts && sample.hour == hour);
        let inputs = super::SpentInputs {
            identity,
            units: track.taken,
            costs: track.cost_taken,
            reason,
        };
        let mut entries = Vec::new();
        let whole = track.edits.take(SPENT_READER, &mut entries);
        let mut partial = false;
        if !starts
            && inputs.units.is_some()
            && inputs.costs.is_some()
            && read.consecutive
            && let Some(held) = &self.spent[source]
        {
            if held.inputs == inputs {
                return Err(Spending::Held);
            }
            partial = !whole
                && !entries.is_empty()
                && held.inputs.identity == inputs.identity
                && held.inputs.reason == inputs.reason;
        }
        Ok(Gathered {
            inputs,
            starts,
            whole,
            entries,
            partial,
        })
    }

    /// `gathered`'s projection with the keys made already; left to the ordered pass where it
    /// needs a new one, or the holder has no place in the facts yet.
    fn project_held(
        &mut self,
        tag: &str,
        source: usize,
        gathered: Gathered,
        read: &SpendingRead<'_>,
        visits: &mut Visits,
    ) -> io::Result<Spending> {
        let Some(id) = self.id.or_else(|| read.ids.get(tag).copied()) else {
            return Ok(Spending::Ordered(gathered));
        };
        self.id = Some(id);
        let Self {
            tracks,
            spent,
            keys,
            ..
        } = self;
        let mut series = SeriesKeys {
            maps: Maps::Held(read.keys),
            holder: id,
            designs: keys,
        };
        let mut tried = Visits::default();
        let projected = project(
            tracks[source].as_ref().expect("a projected track"),
            spent[source].as_ref(),
            &gathered,
            &mut series,
            read.spent,
            read.active,
            source == 1,
            &mut tried,
        )?;
        Ok(match projected {
            Some((active, projection)) => {
                visits.add(tried);
                spent[source] = Some(SpentHolder {
                    inputs: gathered.inputs,
                    active,
                });
                Spending::Projected(projection)
            }
            None => Spending::Ordered(gathered),
        })
    }

    /// `gathered`'s projection in the ordered pass, which makes the keys it needs.
    #[allow(clippy::too_many_arguments)]
    pub fn project_ordered(
        &mut self,
        id: u32,
        source: usize,
        gathered: Gathered,
        maps: Maps<'_>,
        spent: &Series,
        active: &BTreeSet<u64>,
        visits: &mut Visits,
    ) -> io::Result<Projection> {
        self.id = Some(id);
        let Self {
            tracks,
            spent: held,
            keys,
            ..
        } = self;
        let mut series = SeriesKeys {
            maps,
            holder: id,
            designs: keys,
        };
        let (now, projection) = project(
            tracks[source].as_ref().expect("a projected track"),
            held[source].as_ref(),
            &gathered,
            &mut series,
            spent,
            active,
            source == 1,
            visits,
        )?
        .ok_or_else(|| io::Error::other("Production made no key a projection needs"))?;
        held[source] = Some(SpentHolder {
            inputs: gathered.inputs,
            active: now,
        });
        Ok(projection)
    }

    /// Brings each inventory's members to its track, visiting only the designs whose entries
    /// changed where the inventory was kept: what they add and take of the variants' members,
    /// and every variant whose counts change.
    pub fn inventory(&mut self, out: &mut Presented) {
        let mut entries = Vec::new();
        for source in 0..2 {
            let kept = self.listed && self.tracks[source].is_some();
            let held = &mut self.inventory[source];
            if !kept {
                if let Some(old) = held.take() {
                    for (&variant, seen) in &old.members {
                        out.members.push((variant, *seen, source, false));
                    }
                    // Another holder may still select these: their selection changed.
                    out.changed.extend(old.members.keys().copied());
                }
                continue;
            }
            let track = self.tracks[source].as_mut().expect("a kept track");
            let versions = [track.taken, track.cost_taken];
            let whole = track.edits.take(INVENTORY_READER, &mut entries);
            if versions.iter().all(Option::is_some)
                && held.as_ref().is_some_and(|held| held.versions == versions)
            {
                continue;
            }
            if !whole
                && !entries.is_empty()
                && let Some(kept) = held.as_mut()
            {
                let mut at = 0;
                while at < entries.len() {
                    let variant = entries[at].0;
                    while entries.get(at).is_some_and(|entry| entry.0 == variant) {
                        at += 1;
                    }
                    let mut seen = [false; SEEN];
                    let design = (variant, Purpose::New)..=(variant, Purpose::Unknown);
                    let units = track.current.range(design.clone()).map(|(entry, _)| entry);
                    let costs = track.cost_current.range(design).map(|(entry, _)| entry);
                    for (measure, &(_, purpose)) in units
                        .map(|entry| (UNITS_SEEN, entry))
                        .chain(costs.map(|entry| (IC_SEEN, entry)))
                    {
                        out.visits += 1;
                        seen[measure + purpose.index()] = true;
                    }
                    let before = kept.members.get(&variant).copied();
                    if before != seen.contains(&true).then_some(seen) {
                        if let Some(before) = before {
                            out.members.push((variant, before, source, false));
                            kept.members.remove(&variant);
                        }
                        if seen.contains(&true) {
                            out.members.push((variant, seen, source, true));
                            kept.members.insert(variant, seen);
                        }
                        out.changed.push(variant);
                    }
                }
                kept.versions = versions;
                continue;
            }
            if let Some(old) = held.as_ref() {
                for (&variant, seen) in &old.members {
                    out.members.push((variant, *seen, source, false));
                }
            }
            let mut kept: BTreeMap<u64, [bool; SEEN]> = BTreeMap::new();
            for (measure, &(variant, purpose)) in track
                .current
                .keys()
                .map(|entry| (UNITS_SEEN, entry))
                .chain(track.cost_current.keys().map(|entry| (IC_SEEN, entry)))
            {
                out.visits += 1;
                kept.entry(variant).or_default()[measure + purpose.index()] = true;
            }
            for (&variant, seen) in &kept {
                out.members.push((variant, *seen, source, true));
            }
            let old = held.as_ref().map(|old| &old.members);
            out.changed
                .extend(kept.iter().filter_map(|(&variant, seen)| {
                    (old.and_then(|old| old.get(&variant)) != Some(seen)).then_some(variant)
                }));
            if let Some(old) = old {
                out.changed
                    .extend(old.keys().filter(|id| !kept.contains_key(id)).copied());
            }
            *held = Some(VariantInventory {
                versions,
                members: kept,
            });
        }
    }

    /// How its row of the open page, at `next` where it holds one, changes at the hour; `next`
    /// then moves past it. Binds its equipment read's Rocket read as a whole projection does,
    /// withdrawing the open page's join it ends.
    pub fn rows(
        &mut self,
        tag: &str,
        next: &mut usize,
        shared: &RowsShared<'_>,
        out: &mut RowsOut,
    ) -> io::Result<()> {
        let held = shared.held;
        let row = (*next < held.len() && held[*next].tag == tag).then_some(*next);
        let result = self.row(tag, row, *next, shared, out);
        if row.is_some() {
            *next += 1;
        }
        result
    }

    fn row(
        &mut self,
        tag: &str,
        row: Option<usize>,
        next: usize,
        shared: &RowsShared<'_>,
        out: &mut RowsOut,
    ) -> io::Result<()> {
        if !self.listed {
            if let Some(row) = row {
                out.edits.push(RowEdit::Remove(row));
            }
            self.sources = [None, None];
            self.bound = None;
            return Ok(());
        }
        let at = shared.at;
        let held = shared.held;
        let identity = self.tracks[0]
            .as_ref()
            .and_then(|track| track.sample)
            .map(|sample| sample.identity);
        let units_known = identity.is_some_and(|identity| {
            shared
                .tainted
                .get(&identity.2)
                .is_none_or(|from| at < *from)
        });
        let spending_known = identity.is_some_and(|identity| {
            shared
                .spending_tainted
                .get(&identity.2)
                .is_none_or(|(from, _)| at < *from)
        });
        let Self {
            tracks,
            first,
            segments,
            sources,
            bound,
            ..
        } = self;
        let [equipment_track, rocket_track] = tracks;
        let [equipment_segment, rocket_segment] = segments;
        let [equipment_source, rocket_spending] = sources;
        let equipment = source_edit(
            equipment_track.as_mut(),
            at,
            shared.origins[0],
            equipment_segment,
            (units_known, spending_known),
            (shared.tainted_from, first[0], shared.restarts[0], false),
            row.and_then(|row| held[row].equipment.as_ref()),
            equipment_source,
            shared.consecutive,
            &mut out.entries,
            &mut out.amounts,
        )?;
        let rocket = source_edit(
            rocket_track.as_mut(),
            at,
            shared.origins[1],
            rocket_segment,
            (true, true),
            (shared.tainted_from, first[1], shared.restarts[1], true),
            row.and_then(|row| held[row].rocket.as_ref()),
            rocket_spending,
            shared.consecutive,
            &mut out.entries,
            &mut out.amounts,
        )?;
        let mut held_joined = row.map(|row| held[row].rocket_joined);
        let joined = match equipment.0 {
            Some(head) => {
                let (joined, withdrawn) = super::bind_rocket(
                    bound,
                    (head.slot, head.country_id, head.observation, head.read_hour),
                    rocket.0.map(|rocket| super::RocketBinding {
                        slot: rocket.slot,
                        country_id: rocket.country_id,
                        observation: rocket.observation,
                        read_hour: rocket.read_hour,
                        segment: rocket.segment,
                    }),
                    tag,
                    at,
                );
                if let Some(correction) = withdrawn {
                    if let Some(row) = row
                        && correction.matches(&held[row])
                    {
                        held_joined = Some(false);
                        out.unjoined.push(row);
                    }
                    out.corrections.push(correction);
                }
                joined
            }
            None => false,
        };
        let absent_since = sources_since([equipment_source.as_ref(), rocket_spending.as_ref()]);
        // A row lives while a source does; its missing source keeps its absence start.
        let live = equipment.0.is_some() || rocket.0.is_some();
        match row {
            None if live => out.edits.push(RowEdit::Insert(
                next,
                CountryEndpoint {
                    tag: tag.to_owned(),
                    equipment: super::whole_source(equipment.1),
                    rocket: super::whole_source(rocket.1),
                    rocket_joined: joined,
                    absent_since,
                },
            )),
            None => {}
            Some(row) if !live => out.edits.push(RowEdit::Remove(row)),
            Some(row) => {
                if !matches!(equipment.1, super::SourceEdit::Same)
                    || !matches!(rocket.1, super::SourceEdit::Same)
                    || held_joined != Some(joined)
                    || held[row].absent_since != absent_since
                {
                    out.edits.push(RowEdit::Modify {
                        held: row,
                        sources: [equipment.1, rocket.1],
                        joined,
                        absent_since,
                    });
                }
            }
        }
        if !live {
            *equipment_source = None;
            *rocket_spending = None;
            *bound = None;
        }
        Ok(())
    }
}

/// Each source's absence start: where its spending record holds no identity.
fn sources_since(sources: [Option<&SpendingSource>; 2]) -> [Option<i32>; 2] {
    sources.map(|source| {
        source
            .filter(|source| source.identity.is_none())
            .map(|source| source.since)
    })
}

/// What holders' inventories changed of the variants' members: each (variant, membership,
/// source, added), every variant whose counts changed, and the entries visited.
#[derive(Default)]
pub(super) struct Presented {
    pub members: Vec<(u64, [bool; SEEN], usize, bool)>,
    pub changed: Vec<u64>,
    pub visits: usize,
}

/// What every holder's row step reads.
pub(super) struct RowsShared<'a> {
    pub at: i32,
    pub consecutive: bool,
    /// The equipment and Rocket observations' starts.
    pub origins: [Option<i32>; 2],
    pub tainted: &'a BTreeMap<u32, i32>,
    pub spending_tainted: &'a BTreeMap<u32, (i32, ProductionSpendingUnknown)>,
    pub tainted_from: Option<(i32, ProductionSpendingUnknown)>,
    pub restarts: [Option<i32>; 2],
    /// The open page's rows.
    pub held: &'a [CountryEndpoint],
}

/// How a run of holders' rows change: edits in page order, the amount edits they name, the
/// joins they withdraw, and the open page's rows those withdraw from.
#[derive(Default)]
pub(super) struct RowsOut {
    pub edits: Vec<RowEdit>,
    pub amounts: Vec<(Entry, Option<Amount>)>,
    pub corrections: Vec<CoverageCorrection>,
    pub unjoined: Vec<usize>,
    pub entries: Vec<Entry>,
}
