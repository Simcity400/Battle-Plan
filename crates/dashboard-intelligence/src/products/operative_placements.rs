//! Operatives working in each country, dated by the membership read.
//! Targets follow the mission country link or the operation target,
//! as selected for status text.

use super::operative_rows;
use dashboard_base::products::Product;
use replay_data::{
    rows::CountryLinkRow, rows::CountryRow, rows::OperativeMembershipRow, rows::TableId,
    rows::tables,
};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    io,
    sync::atomic::AtomicBool,
};
use ts_rs::TS;

#[cfg(any(test, feature = "test-support"))]
use dashboard_base::source::Source;
#[cfg(any(test, feature = "test-support"))]
use dashboard_core::config::LoadContext;
#[cfg(any(test, feature = "test-support"))]
use dashboard_text::metadata::validate_campaign;
#[cfg(any(test, feature = "test-support"))]
use replay_frame::frame::FrameDecoder;
#[cfg(any(test, feature = "test-support"))]
use std::sync::atomic::Ordering;

pub mod prepared;

#[cfg(any(test, feature = "test-support"))]
mod oracle;
#[cfg(test)]
use oracle::identities;
#[cfg(any(test, feature = "test-support"))]
use oracle::scan;

pub struct OperativePlacements;

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OperativePlacementParams {
    pub branch_id: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum PlacementAvailability {
    Recorded,
    FeatureAbsent,
    TablesNotRecorded,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PlacementPeriod {
    pub first_hour: i32,
    pub last_hour: i32,
    pub availability: PlacementAvailability,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct OperativePlacementRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub count: u32,
    /// Roster/name changes split a run even when its count stays equal.
    pub operatives: Vec<PlacedOperative>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[derive(replay_spool::Spooled)]
pub struct PlacedOperative {
    pub id: String,
    pub name: Option<String>,
    /// A saved/localised empty name, distinct from an unreadable name.
    pub unnamed: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct OperativePlacementPair {
    /// Numeric country identities; independent of opaque tag spelling.
    pub id: String,
    pub owner: String,
    pub target: String,
    pub runs: Vec<OperativePlacementRun>,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct OperativePlacementReply {
    pub branch_id: String,
    pub pairs: Vec<OperativePlacementPair>,
    /// Coverage of complete reads held within an unbroken recorded stretch.
    pub periods: Vec<PlacementPeriod>,
    /// Actual membership read hours, kept separately from compressed equal values.
    pub read_hours: Vec<i32>,
    pub warnings: Vec<String>,
}

/// What a membership read gives the placements: each pair's operatives, or why none can be
/// placed, with the warnings their names add and whether the read holds any membership.
#[derive(Clone, Debug, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub(crate) struct PlacementRead {
    resolved: Result<BTreeMap<PairKey, Vec<PlacedOperative>>, String>,
    warnings: Vec<String>,
    empty: bool,
}

impl PlacementRead {
    pub(crate) fn heap(&self) -> usize {
        let resolved = match &self.resolved {
            Ok(pairs) => pairs
                .iter()
                .map(|(key, operatives)| {
                    key.owner.2.capacity()
                        + key.target.2.capacity()
                        + operatives
                            .iter()
                            .map(|operative| {
                                std::mem::size_of::<PlacedOperative>()
                                    + operative.id.capacity()
                                    + operative.name.as_ref().map_or(0, String::capacity)
                            })
                            .sum::<usize>()
                })
                .sum(),
            Err(reason) => reason.capacity(),
        };
        resolved + self.warnings.iter().map(String::capacity).sum::<usize>()
    }
}

/// What a membership read changed of the placements read before: the pairs whose operatives
/// changed or came and those gone, or the read whole.
#[derive(replay_spool::Spooled)]
pub(crate) enum PlacementDelta {
    Whole(PlacementRead),
    Pairs {
        warnings: Vec<String>,
        empty: bool,
        set: Vec<(PairKey, Vec<PlacedOperative>)>,
        gone: Vec<PairKey>,
    },
}

/// The pairs a read changed, or all.
#[derive(Default)]
pub(crate) struct PlacementTouched {
    pub(crate) all: bool,
    pub(crate) pairs: Vec<PairKey>,
}

impl PlacementRead {
    /// What `self` changed of `old`: none where equal.
    pub(crate) fn delta(&self, old: Option<&Self>) -> Option<PlacementDelta> {
        let Some(old) = old else {
            return Some(PlacementDelta::Whole(self.clone()));
        };
        match (&old.resolved, &self.resolved) {
            (Ok(before), Ok(pairs)) => {
                let (set, gone) = super::intelligence_prepared::delta::mapped(before, pairs);
                (!set.is_empty()
                    || !gone.is_empty()
                    || old.warnings != self.warnings
                    || old.empty != self.empty)
                    .then(|| PlacementDelta::Pairs {
                        warnings: self.warnings.clone(),
                        empty: self.empty,
                        set,
                        gone,
                    })
            }
            _ if old == self => None,
            _ => Some(PlacementDelta::Whole(self.clone())),
        }
    }

    /// Takes `delta` into the read `held`, telling `touched` which pairs it changed.
    pub(crate) fn apply(
        held: &mut Option<Self>,
        delta: PlacementDelta,
        touched: &mut PlacementTouched,
    ) {
        match delta {
            PlacementDelta::Whole(read) => {
                *held = Some(read);
                touched.all = true;
            }
            PlacementDelta::Pairs {
                warnings,
                empty,
                set,
                gone,
            } => {
                let Some(read) = held else {
                    return;
                };
                read.warnings = warnings;
                read.empty = empty;
                if let Ok(pairs) = &mut read.resolved {
                    for key in gone {
                        pairs.remove(&key);
                        touched.pairs.push(key);
                    }
                    for (key, operatives) in set {
                        touched.pairs.push(key.clone());
                        pairs.insert(key, operatives);
                    }
                }
            }
        }
    }
}

impl PlacementDelta {
    pub(crate) fn heap(&self) -> usize {
        let operatives = |operatives: &[PlacedOperative]| {
            operatives
                .iter()
                .map(|operative| {
                    std::mem::size_of::<PlacedOperative>()
                        + operative.id.capacity()
                        + operative.name.as_ref().map_or(0, String::capacity)
                })
                .sum::<usize>()
        };
        match self {
            Self::Whole(read) => read.heap(),
            Self::Pairs {
                warnings,
                set,
                gone,
                ..
            } => {
                warnings.iter().map(String::capacity).sum::<usize>()
                    + set
                        .iter()
                        .map(|(key, value)| key.heap() + operatives(value))
                        .sum::<usize>()
                    + gone.iter().map(PairKey::heap).sum::<usize>()
            }
        }
    }
}

impl PairKey {
    fn heap(&self) -> usize {
        std::mem::size_of::<Self>() + self.owner.2.capacity() + self.target.2.capacity()
    }
}

/// The placements `captured` gives, its countries `inputs` (complete), its names localised by
/// `localisation`.
pub(crate) fn read(
    captured: &[OperativeMembershipRow],
    inputs: &operative_rows::CountryInputs,
    localisation: Option<&installed_content_localisation::localisation::Localisation>,
) -> PlacementRead {
    #[cfg(test)]
    super::intelligence_prepared::work(|work| work.row_visits += captured.len());
    let (_, links) = inputs.complete().expect("complete placement countries");
    let mut warnings = std::collections::BTreeSet::new();
    let resolved = (inputs.unresolved.is_empty() && inputs.text)
        .then_some(&inputs.slots)
        .ok_or_else(|| "Unresolved operative country catalogue".to_owned())
        .and_then(|countries| resolve_rows(captured, countries, links, localisation, &mut warnings));
    PlacementRead {
        resolved,
        warnings: warnings.into_iter().collect(),
        empty: captured.is_empty(),
    }
}

/// What an hour tells the placements beside their read: whether the memberships were read, the
/// agency feature, and whether the memberships are complete.
#[derive(Clone, Copy)]
pub(crate) struct Census {
    pub(crate) read: bool,
    pub(crate) feature: Option<bool>,
    pub(crate) complete: bool,
}

impl Census {
    pub(crate) fn of(frame: &DecodedFrame) -> Self {
        Self {
            read: operative_rows::membership_read(frame),
            feature: operative_rows::agency_feature(frame),
            complete: recorded_operatives(frame).is_some(),
        }
    }

    /// Whether the hour's census is resolved, given its countries.
    pub(crate) fn resolves(self, inputs: &operative_rows::CountryInputs) -> bool {
        self.feature == Some(true) && self.read && self.complete && inputs.complete().is_some()
    }
}

fn recorded_operatives(frame: &DecodedFrame) -> Option<&[OperativeMembershipRow]> {
    // Snapshots keep target/name with the roster. Assignment 3 keeps its
    // mission target even with a reservation; only assignment 4 reads
    // the own operation target.
    frame.complete::<tables::OperativeMemberships>().ok()
}

#[derive(
    Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, replay_spool::Spooled,
)]
pub(crate) struct PairKey {
    owner: (u32, u32, String),
    target: (u32, u32, String),
}

fn resolve_rows<T: std::borrow::Borrow<CountryRow>>(
    rows: &[OperativeMembershipRow],
    countries: &HashMap<u32, T>,
    links: &[CountryLinkRow],
    localisation: Option<&installed_content_localisation::localisation::Localisation>,
    warnings: &mut std::collections::BTreeSet<String>,
) -> Result<BTreeMap<PairKey, Vec<PlacedOperative>>, String> {
    let mut result: BTreeMap<PairKey, Vec<PlacedOperative>> = BTreeMap::new();
    let mut seen = BTreeMap::new();
    let mut unnamed = dashboard_text::catalogue::Refusals::default();
    let mut refused = dashboard_text::catalogue::Refusals::default();
    for row in rows {
        if let Err(reason) =
            operative_rows::supported_kind_assignment(row.roster_kind, row.assignment)
        {
            refused.note(reason, row.id.to_string());
            continue;
        }
        if row.roster_kind != 0
            || operative_rows::captured(row.assignment)
            || !matches!(row.assignment, 3 | 4)
        {
            continue;
        }
        if row.assignment == 3 {
            match operative_rows::mission_variant(row.action_variant) {
                Ok(false) => continue,
                Ok(true) => {}
                Err(reason) => {
                    refused.note(reason, row.id.to_string());
                    continue;
                }
            }
        } else if row
            .operation
            .require("OperativeMemberships.operation")
            .ok()
            .copied()
            .flatten()
            .is_none_or(|operation| operation == 0)
        {
            refused.note("Operation reference not recorded", row.id.to_string());
            continue;
        }
        match operative_rows::admit(&mut seen, row, links, countries) {
            Ok(true) => {}
            Ok(false) => continue,
            Err(reason) => {
                refused.note(reason, row.id.to_string());
                continue;
            }
        }
        let key = (|| {
            let owner_id = operative_rows::identity(row.owner_country, links, countries)
                .ok_or_else(|| format!("Unresolved operative owner {}", row.owner_country))?;
            let target = row
                .target_country
                .require("OperativeMemberships.target_country")?
                .as_ref()
                .copied()
                .ok_or_else(|| "Target not recorded".to_owned())?;
            let target_id = operative_rows::identity(target, links, countries)
                .ok_or_else(|| format!("Unresolved operative target {target}"))?;
            let owner: &CountryRow = countries[&owner_id.slot].borrow();
            let target: &CountryRow = countries[&target_id.slot].borrow();
            Ok::<_, String>(PairKey {
                owner: (
                    owner_id.slot,
                    owner_id.id,
                    String::from_utf8(owner.tag.clone()).expect("validated country tag"),
                ),
                target: (
                    target_id.slot,
                    target_id.id,
                    String::from_utf8(target.tag.clone()).expect("validated country tag"),
                ),
            })
        })();
        let key = match key {
            Ok(key) => key,
            Err(reason) => {
                refused.note(reason, row.id.to_string());
                continue;
            }
        };
        let name = operative_rows::recorded_name(localisation, row);
        if name.literal_fallback {
            warnings.insert("Operative names show their saved literal/key until the recorded content stack is available.".into());
        }
        if let Some(reason) = name.missing {
            unnamed.note(
                reason,
                format!(
                    "{} ({:?})",
                    row.id,
                    String::from_utf8_lossy(
                        row.name
                            .require("OperativeMemberships.name")
                            .map(Vec::as_slice)
                            .unwrap_or_default()
                    )
                ),
            );
        }
        result.entry(key).or_default().push(PlacedOperative {
            id: row.id.to_string(),
            name: name.name,
            unnamed: name.unnamed,
        });
    }
    warnings.extend(unnamed.lines("operative names"));
    let refused = refused.lines("operative placements");
    if !refused.is_empty() {
        return Err(refused.join("\n"));
    }
    for operatives in result.values_mut() {
        operatives.sort_by(|a, b| a.id.cmp(&b.id));
    }
    Ok(result)
}

#[derive(Default, Serialize, Deserialize)]
struct History {
    #[serde(with = "dashboard_core::map_pairs")]
    pairs: BTreeMap<PairKey, OperativePlacementPair>,
    periods: Vec<PlacementPeriod>,
    reads: Vec<i32>,
    last_hour: Option<i32>,
    /// The pairs held, each run going on through `last_hour` however far its own end says.
    #[serde(with = "held_pairs")]
    held: Option<BTreeMap<PairKey, Vec<PlacedOperative>>>,
}

/// The held pairs persist as a list of key and operatives.
mod held_pairs {
    use super::{BTreeMap, PairKey, PlacedOperative};
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    type Held = Option<BTreeMap<PairKey, Vec<PlacedOperative>>>;

    pub fn serialize<S: Serializer>(held: &Held, serializer: S) -> Result<S::Ok, S::Error> {
        held.as_ref()
            .map(|held| held.iter().collect::<Vec<_>>())
            .serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Held, D::Error> {
        Ok(
            Option::<Vec<(PairKey, Vec<PlacedOperative>)>>::deserialize(deserializer)?
                .map(|held| held.into_iter().collect()),
        )
    }
}

enum Observation {
    Read(BTreeMap<PairKey, Vec<PlacedOperative>>),
    Hold,
    Unavailable(PlacementAvailability),
}

/// What an hour's observation told of the pairs held.
enum Told {
    /// A read going on from the hour before, changing only these pairs.
    Pairs(Vec<PairKey>),
    /// A read taken whole.
    Read,
    /// A hold going on unchanged.
    Held,
    /// Every pair, without a read.
    All,
}

impl History {
    /// Carries the runs of the pairs held through the last hour observed: each went on
    /// unchanged since it was last told.
    fn carry(&mut self) {
        let (Some(held), Some(last)) = (&self.held, self.last_hour) else {
            return;
        };
        for key in held.keys() {
            #[cfg(test)]
            super::intelligence_prepared::work(|work| work.history_visits += 1);
            if let Some(run) = self
                .pairs
                .get_mut(key)
                .and_then(|pair| pair.runs.last_mut())
            {
                run.last_hour = last;
            }
        }
    }

    /// Re-keys the pairs held by `countries`' identities; their runs are carried first.
    fn project<T: std::borrow::Borrow<CountryRow>>(
        &mut self,
        countries: &HashMap<u32, T>,
    ) -> Result<(), String> {
        let Some(held) = self.held.as_ref() else {
            return Ok(());
        };
        let current = |identity: &(u32, u32, String)| {
            let row = countries
                .get(&identity.0)
                .map(|row| std::borrow::Borrow::<CountryRow>::borrow(row))
                .filter(|row| row.id == identity.1)
                .ok_or_else(|| {
                    format!(
                        "Held operative country identity {}:{} is unavailable",
                        identity.0, identity.1
                    )
                })?;
            let tag = operative_rows::tag_of(
                Some(operative_rows::CountryIdentity {
                    slot: identity.0,
                    id: identity.1,
                }),
                countries,
            )
            .ok_or_else(|| "Held operative country tag is invalid".to_owned())?;
            Ok::<_, String>((identity.0, row.id, tag))
        };
        let projected = held
            .iter()
            .map(|(key, value)| {
                Ok((
                    PairKey {
                        owner: current(&key.owner)?,
                        target: current(&key.target)?,
                    },
                    value.clone(),
                ))
            })
            .collect::<Result<BTreeMap<_, _>, String>>()?;
        self.held = Some(projected);
        Ok(())
    }

    /// The pair of `key`, made where it is new.
    fn pair<'p>(
        pairs: &'p mut BTreeMap<PairKey, OperativePlacementPair>,
        key: &PairKey,
    ) -> &'p mut OperativePlacementPair {
        pairs
            .entry(key.clone())
            .or_insert_with(|| OperativePlacementPair {
                id: serde_json::to_string(&(&key.owner, &key.target))
                    .expect("serializable country identities"),
                owner: key.owner.2.clone(),
                target: key.target.2.clone(),
                runs: Vec::new(),
            })
    }

    /// Tells `pair`'s run of `value` at `hour`: the run going on to it, or a new one.
    fn run(pair: &mut OperativePlacementPair, hour: i32, value: &[PlacedOperative], goes_on: bool) {
        let count = u32::try_from(value.len()).expect("native operative census fits u32");
        match pair.runs.last_mut() {
            Some(run)
                if goes_on
                    && run.last_hour.checked_add(1) == Some(hour)
                    && run.operatives == value =>
            {
                run.last_hour = hour
            }
            _ => pair.runs.push(OperativePlacementRun {
                first_hour: hour,
                last_hour: hour,
                count,
                operatives: value.to_vec(),
            }),
        }
    }

    fn update(&mut self, hour: i32, observation: Observation) {
        self.carry();
        self.advance(hour, observation);
    }

    /// [`Self::update`] of pairs whose runs were carried through the last hour.
    fn advance(&mut self, hour: i32, observation: Observation) {
        let consecutive = self.last_hour.and_then(|last| last.checked_add(1)) == Some(hour);
        self.last_hour = Some(hour);
        if !consecutive {
            self.held = None;
        }
        let availability = match observation {
            Observation::Read(rows) => {
                self.reads.push(hour);
                self.held = Some(rows);
                PlacementAvailability::Recorded
            }
            Observation::Hold if self.held.is_some() => PlacementAvailability::Recorded,
            Observation::Hold => PlacementAvailability::TablesNotRecorded,
            Observation::Unavailable(availability) => {
                self.held = None;
                availability
            }
        };
        match self.periods.last_mut() {
            Some(period) if consecutive && period.availability == availability => {
                period.last_hour = hour
            }
            _ => self.periods.push(PlacementPeriod {
                first_hour: hour,
                last_hour: hour,
                availability,
            }),
        }
        if availability != PlacementAvailability::Recorded {
            return;
        }
        for (key, value) in self.held.as_ref().expect("recorded coverage has a read") {
            Self::run(Self::pair(&mut self.pairs, key), hour, value, consecutive);
        }
    }

    /// [`Self::update`] of a read of `resolved` at `hour` going on from the read held the hour
    /// before, of which only `keys` changed: only those pairs are told. False, changing
    /// nothing, where it does not go on.
    fn patch(
        &mut self,
        hour: i32,
        resolved: &BTreeMap<PairKey, Vec<PlacedOperative>>,
        keys: &[PairKey],
    ) -> bool {
        let consecutive = self.last_hour.and_then(|last| last.checked_add(1)) == Some(hour);
        let Some(last) = self
            .last_hour
            .filter(|_| consecutive && self.held.is_some())
        else {
            return false;
        };
        let held = self.held.as_mut().expect("a held read");
        for key in keys {
            if held.contains_key(key)
                && let Some(run) = self
                    .pairs
                    .get_mut(key)
                    .and_then(|pair| pair.runs.last_mut())
            {
                run.last_hour = last;
            }
            match resolved.get(key) {
                Some(value) => {
                    Self::run(Self::pair(&mut self.pairs, key), hour, value, true);
                    held.insert(key.clone(), value.clone());
                }
                None => {
                    held.remove(key);
                }
            }
        }
        self.last_hour = Some(hour);
        self.reads.push(hour);
        self.periods
            .last_mut()
            .expect("a held read's coverage")
            .last_hour = hour;
        true
    }

    /// [`Self::update`] of a hold going on from the read held the hour before, its countries
    /// unchanged: no pair changes. False, changing nothing, where it does not go on.
    fn hold(&mut self, hour: i32) -> bool {
        let consecutive = self.last_hour.and_then(|last| last.checked_add(1)) == Some(hour);
        if !consecutive || self.held.is_none() {
            return false;
        }
        self.last_hour = Some(hour);
        self.periods
            .last_mut()
            .expect("a held read's coverage")
            .last_hour = hour;
        true
    }
}

#[derive(Default, Serialize, Deserialize)]
struct Scan {
    history: History,
    warnings: std::collections::BTreeSet<String>,
    #[serde(skip)]
    catalogue: Option<std::sync::Arc<dashboard_text::catalogue::Catalogue>>,
    #[cfg(any(test, feature = "test-support"))]
    #[serde(skip)]
    native_countries: operative_rows::CountryInputs,
}
impl Scan {
    /// The catalogue a census holding memberships (not `empty`) names them by, as bound.
    fn load(
        &mut self,
        bound: &Result<
            Option<std::sync::Arc<dashboard_text::catalogue::Catalogue>>,
            dashboard_text::catalogue::ContentError,
        >,
        empty: bool,
    ) -> io::Result<()> {
        if self.catalogue.is_none() && !empty {
            self.catalogue = bound
                .as_ref()
                .map_err(dashboard_text::catalogue::ContentError::error)?
                .clone();
        }
        Ok(())
    }

    /// Observes `decoded` of the row path, its census read where [`Census::resolves`].
    #[cfg(any(test, feature = "test-support"))]
    fn observe_bound(
        &mut self,
        decoded: &DecodedFrame,
        inputs: &operative_rows::CountryInputs,
        bound: &Result<
            Option<std::sync::Arc<dashboard_text::catalogue::Catalogue>>,
            dashboard_text::catalogue::ContentError,
        >,
    ) -> io::Result<()> {
        let census = Census::of(decoded);
        let read = match recorded_operatives(decoded).filter(|_| census.resolves(inputs)) {
            Some(captured) => {
                self.load(bound, captured.is_empty())?;
                Some(read(
                    captured,
                    inputs,
                    self.catalogue
                        .as_ref()
                        .map(|catalogue| catalogue.localisation.as_ref()),
                ))
            }
            None => None,
        };
        self.observe_read(
            decoded.hour,
            census,
            inputs,
            bound,
            read.as_ref(),
            None,
            true,
        )
        .map(drop)
    }

    /// Observes `hour`, its census `read` given where [`Census::resolves`]: a read going on
    /// from the hour before tells only the pairs of `changed`, where given, and a hold of
    /// countries that did not change (`!rekey`) tells none.
    #[allow(clippy::too_many_arguments)]
    fn observe_read(
        &mut self,
        hour: i32,
        census: Census,
        inputs: &operative_rows::CountryInputs,
        bound: &Result<
            Option<std::sync::Arc<dashboard_text::catalogue::Catalogue>>,
            dashboard_text::catalogue::ContentError,
        >,
        read: Option<&PlacementRead>,
        changed: Option<&[PairKey]>,
        rekey: bool,
    ) -> io::Result<Told> {
        let mut observation = Observation::Unavailable(PlacementAvailability::TablesNotRecorded);
        let is_read = census.read;
        let mut carried = false;
        if let Some(feature) = census.feature {
            if !feature {
                observation = Observation::Unavailable(PlacementAvailability::FeatureAbsent);
            } else if census.resolves(inputs) {
                let read = read.ok_or_else(|| io::Error::other("The census was not read"))?;
                self.load(bound, read.empty)?;
                self.warnings.extend(read.warnings.iter().cloned());
                match &read.resolved {
                    Ok(resolved) => {
                        if let Some(keys) = changed
                            && self.history.patch(hour, resolved, keys)
                        {
                            return Ok(Told::Pairs(keys.to_vec()));
                        }
                        self.history
                            .update(hour, Observation::Read(resolved.clone()));
                        return Ok(Told::Read);
                    }
                    Err(reason) => {
                        self.warnings.insert(reason.clone());
                    }
                }
            } else if !is_read && census.complete {
                if !rekey && self.history.hold(hour) {
                    return Ok(Told::Held);
                }
                // The runs go on under the keys they had before any projection renames them.
                self.history.carry();
                carried = true;
                let Self {
                    history, warnings, ..
                } = self;
                // Complete held rows (including keyframes) retain the last
                // read's date. An unavailable/truncated table ends the hold,
                // including a scheduled failure with no measured walk.
                let projected = (|| {
                    inputs.countries.as_deref().ok_or_else(|| {
                        "Held operative country catalogue is unavailable".to_owned()
                    })?;
                    inputs
                        .links
                        .as_deref()
                        .ok_or_else(|| "Held operative country links are unavailable".to_owned())?;
                    if !inputs.unresolved.is_empty() || !inputs.text {
                        return Err("Held operative country identities are unresolved".to_owned());
                    }
                    history.project(&inputs.slots)
                })();
                match projected {
                    Ok(()) => observation = Observation::Hold,
                    Err(reason) => {
                        warnings.insert(reason);
                    }
                }
            }
        }
        if carried {
            self.history.advance(hour, observation);
        } else {
            self.history.update(hour, observation);
        }
        Ok(Told::All)
    }
    fn reply(&self, branch: u64) -> OperativePlacementReply {
        OperativePlacementReply {
            branch_id: branch.to_string(),
            pairs: self.history.pairs.values().cloned().collect(),
            periods: self.history.periods.clone(),
            read_hours: self.history.reads.clone(),
            warnings: self.warnings.iter().cloned().collect(),
        }
    }
}

impl Product for OperativePlacements {
    const KEY: &'static str = "operative-placements";
    const CONTENT_DEPENDENT: bool = true;
    const SERVED: Option<&'static dashboard_base::prepared::served::Plan> =
        Some(&super::intelligence_prepared::served::PLAN);
    type Params = OperativePlacementParams;
    type Reply = OperativePlacementReply;
    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params
            .branch_id
            .parse::<u64>()
            .map(|_| ())
            .map_err(|_| "Invalid operative placement branch")
    }
    fn types() -> Vec<String> {
        vec![
            OperativePlacementParams::decl(),
            PlacementAvailability::decl(),
            PlacementPeriod::decl(),
            PlacedOperative::decl(),
            OperativePlacementRun::decl(),
            OperativePlacementPair::decl(),
            OperativePlacementReply::decl(),
        ]
    }
    #[cfg(any(test, feature = "test-support"))]
    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let branch = params.branch_id.parse().expect("validated branch");
        scan(source, branch, context, cancelled)
    }
}

#[cfg(test)]
mod tests;
