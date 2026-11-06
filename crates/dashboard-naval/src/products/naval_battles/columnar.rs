//! The naval family projected from the rows a columnar walk changed, for the ordered scan.
//! A walk starts only where the restart holds the sunk ships' last read, and carries it. The
//! countries, and the tables a sunk ship's place name reads, are the walk's
//! ([`dashboard_world`]): the naming tables are asked for from the first title on.

pub(super) mod emit;

pub use emit::Branch;
pub(super) use emit::Carried;

use super::{
    PREPARED_TABLES, Quantities, ResultInput, SideSample,
    details::{NavalBattleFleetNames, named_fleets, unknown_fleets},
    side_countries,
};
use crate::products::{
    ships_sunk::{self, battle_titles},
    sinkings::{self, ConvoyRead, ProjectedCountries, SavedBase, ShipBase, ShipRead},
};
use dashboard_base::{catalogue, source::Source};
use dashboard_core::config::LoadContext;
use dashboard_text::metadata::{LinkTags, slot_reads};
use dashboard_world::{Needs, World};
use replay::{
    columnar_frames::metrics,
    columns::{self, Status},
    segments::{self, Operator, Sink, Start, State},
};
use replay_data::{
    format::FormatVersion,
    rows::{
        FleetRow, LexicalTokenRow, NavalCombatDetailsRow, NavalCombatResultDetailsRow,
        NavalResultCountryRow, NavalResultMemberRow, NavalResultRow, NavalResultSourceRow,
        RowCodec, SunkConvoyRow, SunkShipEntry, SunkShipsRow, TABLE_COUNT, TableId,
    },
    scope::CaptureMode,
};
use replay_frame::frame::{ViewIdentity, view_reads};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet, hash_map::Entry},
    io,
    sync::{Arc, LazyLock, atomic::AtomicBool},
};

/// What a projection reads of its walk's world: the countries. Place names ask the frame for
/// their tables when a sunk ship first needs a title.
pub fn needs() -> Needs {
    Needs {
        countries: true,
        ..Needs::default()
    }
}

/// The family's tables and the trigger kit a scripted place name evaluates.
pub fn tables() -> &'static [TableId] {
    static TABLES: LazyLock<Vec<TableId>> = LazyLock::new(|| {
        let mut tables = PREPARED_TABLES.to_vec();
        tables.extend_from_slice(dashboard_text::triggers::RECORDED_TABLES);
        tables.sort_unstable();
        tables.dedup();
        tables
    });
    &TABLES
}

/// The tables a sunk ship's place name reads unless its text evaluates the trigger kit.
const NAMING: [TableId; 8] = [
    TableId::Countries,
    TableId::CountryLinks,
    TableId::LexicalTokens,
    TableId::Provinces,
    TableId::ProvinceNames,
    TableId::States,
    TableId::ReservedCountryNaming,
    TableId::CountryAppearance,
];

/// The tables whose changes the projection follows.
const FOLLOWED: [TableId; 15] = [
    TableId::Countries,
    TableId::CountryLinks,
    TableId::NavalCombats,
    TableId::NavalCombatParticipants,
    TableId::NavalCombatDetails,
    TableId::NavalResults,
    TableId::NavalResultMembers,
    TableId::NavalResultCountries,
    TableId::NavalCombatResultDetails,
    TableId::NavalResultSources,
    TableId::TaskForces,
    TableId::Fleets,
    TableId::SunkShips,
    TableId::SunkConvoys,
    TableId::LexicalTokens,
];
const COUNTRIES: usize = 0;
const LINKS: usize = 1;
const COMBATS: usize = 2;
const PARTICIPANTS: usize = 3;
const DETAILS: usize = 4;
const RESULTS: usize = 5;
const MEMBERS: usize = 6;
const RESULT_COUNTRIES: usize = 7;
const RESULT_DETAILS: usize = 8;
const SOURCES: usize = 9;
const TASK_FORCES: usize = 10;
const FLEETS: usize = 11;
const SUNK_SHIPS: usize = 12;
const SUNK_CONVOYS: usize = 13;
const TOKENS: usize = 14;

/// One projected hour.
#[derive(replay_spool::Spooled)]
pub struct Hour {
    pub branch: u64,
    pub hour: i32,
    pub(super) countries: Countries,
    pub(super) combats: Combats,
    pub(super) sources: Sources,
    pub(super) results: Results,
    /// The LexicalTokens rows the sinking scan reads, when they changed and are complete.
    pub(super) tokens: Option<Vec<(u32, Vec<u8>)>>,
    pub(super) ships: ShipRead,
    /// At the walk's first read: the read its comparison starts from, if any.
    pub(super) seeded: Option<Option<i32>>,
    pub(super) convoys: ConvoyRead,
    /// The labels of the combats whose rows they read changed (all when the rosters did).
    pub(super) fleets: Option<(Vec<FleetLabels>, catalogue::OwnRefusals)>,
    /// The title operands of the ships first found this hour.
    pub(super) sighting: Option<(battle_titles::Sighting, catalogue::OwnRefusals)>,
    /// At a segment's last hour, after a compare in it: what a later read compares with.
    pub(super) base: Option<Box<ShipBase>>,
}

/// The country tables link tags and country holders read.
#[derive(replay_spool::Spooled)]
pub(super) enum Countries {
    /// Countries or CountryLinks not complete.
    Incomplete,
    /// Neither changed what they read since the hour before.
    Same,
    Read(Box<ProjectedCountries>),
}

impl Countries {
    /// As the sinking scan takes them ([`sinkings::ShipHistory::prepare_projected`]).
    pub(super) fn tables(&self) -> Option<Option<&ProjectedCountries>> {
        match self {
            Self::Incomplete => None,
            Self::Same => Some(None),
            Self::Read(read) => Some(Some(&**read)),
        }
    }
}

/// The active combat census.
#[derive(replay_spool::Spooled)]
pub(super) enum Combats {
    /// NavalCombats not complete.
    Unavailable,
    /// No combat's sample read anything else than the hour before.
    Same,
    /// The samples that may differ from the hour before's (all when `all`), and the ended.
    Changed {
        all: bool,
        samples: Vec<CombatSample>,
        ended: Vec<u64>,
    },
}

/// A combat's sides as its rows read this hour, before the floors its episode keeps.
#[derive(Clone, PartialEq)]
#[derive(replay_spool::Spooled)]
pub(super) struct CombatSample {
    pub(super) id: u64,
    pub(super) province: u32,
    pub(super) sides: [SideSample; 2],
}

#[derive(replay_spool::Spooled)]
pub(super) enum Sources {
    /// NavalResultSources not complete.
    Unavailable,
    /// The rows of the hour before.
    Same,
    Rows(Vec<NavalResultSourceRow>),
}

/// The retained result census.
#[derive(replay_spool::Spooled)]
pub(super) enum Results {
    /// NavalResults not complete.
    Unavailable,
    /// None of the result tables changed since the hour before.
    Same,
    /// The rows refused by kind, the changed results' inputs (all when `all`), and the ended.
    Changed {
        all: bool,
        invalid: [usize; 4],
        inputs: Vec<ResultInput>,
        ended: Vec<u64>,
    },
}

/// A present combat's fleet labels at its province, and the names they refused.
pub(super) type FleetLabels = (u64, u32, [NavalBattleFleetNames; 2], catalogue::Refusals);

/// The columns a projection reads by name.
struct Fields {
    combat_province: usize,
    participant: [usize; 3],
    detail: [usize; 2],
    result: [usize; 3],
    member: [usize; 3],
    result_country: usize,
    result_detail: usize,
    task_fleet: usize,
    fleet_name: usize,
    convoy: [usize; 2],
    victory_points: usize,
}

/// The result rows a read refuses, kept as their rows change.
#[derive(Default)]
struct Validity {
    /// Each result's whether its selectors are out of range, and its hour.
    results: HashMap<u64, (bool, i32)>,
    selectors: usize,
    hours: BTreeMap<i32, usize>,
    /// Each member row's slot, and whether its side is out of range.
    members: BTreeMap<u128, (u32, bool)>,
    member_sides: usize,
    /// Each result country row's slot.
    countries: BTreeMap<u128, u32>,
    country_lists: usize,
    /// Results whose classification repeats a member or holds an invalid convoy flag.
    details: HashSet<u64>,
}

impl Validity {
    fn result(&mut self, id: u64, now: Option<(bool, i32)>) {
        if let Some((bad, hour)) = self.results.remove(&id) {
            self.selectors -= usize::from(bad);
            if let Some(count) = self.hours.get_mut(&hour) {
                *count -= 1;
                if *count == 0 {
                    self.hours.remove(&hour);
                }
            }
        }
        if let Some((bad, hour)) = now {
            self.selectors += usize::from(bad);
            *self.hours.entry(hour).or_default() += 1;
            self.results.insert(id, (bad, hour));
        }
    }

    fn member(&mut self, key: u128, now: Option<(u32, bool)>) {
        if let Some((_, bad)) = self.members.remove(&key) {
            self.member_sides -= usize::from(bad);
        }
        if let Some((slot, bad)) = now {
            self.member_sides += usize::from(bad);
            self.members.insert(key, (slot, bad));
        }
    }

    fn country(&mut self, key: u128, now: Option<u32>) {
        let bad = (key >> 32) as u8 > 2;
        if self.countries.remove(&key).is_some() {
            self.country_lists -= usize::from(bad);
        }
        if let Some(slot) = now {
            self.country_lists += usize::from(bad);
            self.countries.insert(key, slot);
        }
    }

    /// The results whose selectors or date a read at `hour` refuses.
    fn refused(&self, hour: i32) -> usize {
        self.selectors
            + hour.checked_add(1).map_or(0, |later| {
                self.hours.range(later..).map(|(_, count)| count).sum()
            })
    }
}

/// Each combat's details row at its province this hour, decoded once for its sample and labels.
type Details = HashMap<u64, Option<NavalCombatDetailsRow>>;

/// What a parked projection carries: the next hour reads every row anew.
#[derive(Serialize, Deserialize)]
struct Saved {
    base: SavedBase,
    /// Whether the walk's first read is checked against the scan's, and the read it seeds.
    seeded: bool,
    seed: Option<i32>,
    ships_read: bool,
    convoys_read: bool,
}

/// One segment's projection.
pub struct Projector<'a> {
    format: FormatVersion,
    mode: CaptureMode,
    source: &'a Source,
    context: &'a LoadContext,
    catalogue: Option<Arc<dashboard_text::catalogue::Catalogue>>,
    fields: Fields,
    shapes: [Option<Status>; FOLLOWED.len()],
    /// Which followed tables changed status this hour.
    reshaped: [bool; FOLLOWED.len()],
    previous: Option<i32>,
    /// Whether the next hour reads every row anew.
    whole: bool,
    tags: LinkTags,
    links_known: bool,
    participants: BTreeMap<u128, u32>,
    /// Each present combat's sample the hour before.
    combats: HashMap<u64, CombatSample>,
    validity: Validity,
    /// Which child tables the last result read could use; `reread` after a refused one.
    usable: Option<[bool; 3]>,
    reread: bool,
    base: ShipBase,
    seed: Option<Option<i32>>,
    ships_changed: bool,
    ships_read: bool,
    /// Whether a read compared the sunk ships since the walk last handed them on.
    compared: bool,
    /// The keys of the convoy rows changed since the last read; None for every row.
    convoys_changed: Option<BTreeSet<u128>>,
    convoys_read: bool,
    /// The hour projected last, held back for the segment's end to hand the sunk ships on.
    pending: Option<Hour>,
}

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

fn names(mask: u64, fields: &[usize]) -> bool {
    fields
        .iter()
        .any(|&column| column >= 64 || mask >> column & 1 != 0)
}

/// Whether the hour changed any of `fields` (every field when empty) of `table`.
fn touched(state: &State, table: TableId, fields: &[usize]) -> io::Result<bool> {
    let changes = columns::changes(state, table)?;
    let named = |mask: u64| fields.is_empty() || names(mask, fields);
    Ok(!changes.inserted.is_empty()
        || !changes.deleted.is_empty()
        || changes.truncated
        || changes.masks.iter().any(|&mask| named(mask))
        || changes.fill_masks.iter().any(|&mask| named(mask)))
}

/// The slots the hour wrote any of `fields` at (any field when empty), and inserted.
fn written(state: &State, table: TableId, fields: &[usize]) -> io::Result<Vec<u32>> {
    let changes = columns::changes(state, table)?;
    let named = |mask: u64| fields.is_empty() || names(mask, fields);
    Ok(changes
        .changed
        .iter()
        .zip(&changes.masks)
        .chain(changes.fill_slots.iter().zip(&changes.fill_masks))
        .filter(|&(_, &mask)| named(mask))
        .map(|(&slot, _)| slot)
        .chain(changes.inserted.iter().copied())
        .collect())
}

/// `table`'s live rows' keys and slots, ascending by key.
fn live(state: &State, table: TableId) -> io::Result<Vec<(u128, u32)>> {
    let held = columns::table(state, table)?;
    let mut rows: Vec<(u128, u32)> = held
        .live()
        .ones()
        .filter_map(|slot| held.key(slot).map(|key| (key, slot)))
        .collect();
    rows.sort_unstable();
    Ok(rows)
}

/// The keys of the rows the hour deleted from `table`.
fn deleted(state: &State, table: TableId) -> io::Result<&[u128]> {
    Ok(&columns::changes(state, table)?.deleted_keys)
}

fn value<T: columns::Plain + Copy>(
    state: &State,
    table: TableId,
    field: usize,
    slot: u32,
) -> io::Result<T> {
    columns::plain::<T>(columns::table(state, table)?, field)?
        .get(slot as usize)
        .copied()
        .ok_or_else(|| invalid(format!("{table:?} slot {slot} holds no value")))
}

/// Whether a result's classification repeats a member or holds a convoy flag above one.
fn bad_classification(row: &NavalCombatResultDetailsRow) -> bool {
    row.sides.iter().any(|side| {
        let mut members = HashSet::new();
        side.ships
            .iter()
            .any(|ship| ship.is_convoy > 1 || !members.insert(ship.member))
    })
}

impl<'a> Projector<'a> {
    /// A segment's projection, naming through `content`, the recording's prepared content.
    pub fn new(
        format: FormatVersion,
        mode: CaptureMode,
        source: &'a Source,
        context: &'a LoadContext,
        content: &ships_sunk::PreparedContent,
    ) -> io::Result<Self> {
        let field = columns::field;
        Ok(Self {
            format,
            mode,
            source,
            context,
            catalogue: content.as_ref().ok().map(|(cat, _)| Arc::clone(cat)),
            fields: Fields {
                combat_province: field(TableId::NavalCombats, "province")?,
                participant: [
                    field(TableId::NavalCombatParticipants, "ship")?,
                    field(TableId::NavalCombatParticipants, "owner")?,
                    field(TableId::NavalCombatParticipants, "state")?,
                ],
                detail: [
                    field(TableId::NavalCombatDetails, "province")?,
                    field(TableId::NavalCombatDetails, "sides")?,
                ],
                result: [
                    field(TableId::NavalResults, "result_hour")?,
                    field(TableId::NavalResults, "winning_side")?,
                    field(TableId::NavalResults, "losing_side")?,
                ],
                member: [
                    field(TableId::NavalResultMembers, "presence_value")?,
                    field(TableId::NavalResultMembers, "ordinal")?,
                    field(TableId::NavalResultMembers, "side")?,
                ],
                result_country: field(TableId::NavalResultCountries, "ordinal")?,
                result_detail: field(TableId::NavalCombatResultDetails, "sides")?,
                task_fleet: field(TableId::TaskForces, "fleet")?,
                fleet_name: field(TableId::Fleets, "name")?,
                convoy: [
                    field(TableId::SunkConvoys, "convoys")?,
                    field(TableId::SunkConvoys, "ordinal")?,
                ],
                victory_points: field(TableId::Provinces, "victory_points")?,
            },
            shapes: [None; FOLLOWED.len()],
            reshaped: [false; FOLLOWED.len()],
            previous: None,
            whole: true,
            tags: LinkTags::default(),
            links_known: false,
            participants: BTreeMap::new(),
            combats: HashMap::new(),
            validity: Validity::default(),
            usable: None,
            reread: true,
            base: ShipBase::default(),
            seed: None,
            ships_changed: true,
            ships_read: false,
            compared: false,
            convoys_changed: None,
            convoys_read: false,
            pending: None,
        })
    }

    fn row<R: RowCodec>(&self, state: &State, table: TableId, slot: u32) -> io::Result<R> {
        columns::row(state, table, slot, self.format.version())
    }

    /// Forgets what the next hour reads anew: a segment's first hour reads every row.
    fn forget(&mut self) {
        self.whole = true;
        self.tags = LinkTags::default();
        self.links_known = false;
        self.participants = BTreeMap::new();
        self.combats = HashMap::new();
        self.validity = Validity::default();
        self.convoys_changed = None;
    }

    /// The per-run indexes of the child tables, read whole.
    fn rebuild(&mut self, state: &State) -> io::Result<()> {
        self.participants = live(state, TableId::NavalCombatParticipants)?
            .into_iter()
            .collect();
        self.validity = Validity::default();
        for (key, slot) in live(state, TableId::NavalResults)? {
            let row = self.result_operands(state, slot)?;
            self.validity.result(key as u64, Some(row));
        }
        for (key, slot) in live(state, TableId::NavalResultMembers)? {
            let side: u8 = value(
                state,
                TableId::NavalResultMembers,
                self.fields.member[2],
                slot,
            )?;
            self.validity.member(key, Some((slot, side > 1)));
        }
        for (key, slot) in live(state, TableId::NavalResultCountries)? {
            self.validity.country(key, Some(slot));
        }
        for (key, slot) in live(state, TableId::NavalCombatResultDetails)? {
            let row: NavalCombatResultDetailsRow =
                self.row(state, TableId::NavalCombatResultDetails, slot)?;
            if bad_classification(&row) {
                self.validity.details.insert(key as u64);
            }
        }
        Ok(())
    }

    /// A result row's selector validity and hour.
    fn result_operands(&self, state: &State, slot: u32) -> io::Result<(bool, i32)> {
        let [hour, winning, losing] = self.fields.result;
        let winning: u8 = value(state, TableId::NavalResults, winning, slot)?;
        let losing: u8 = value(state, TableId::NavalResults, losing, slot)?;
        Ok((
            winning > 2 || losing > 2,
            value(state, TableId::NavalResults, hour, slot)?,
        ))
    }

    /// The indexes moved by the hour's changes. The combats whose rows changed, and the results.
    fn maintain(&mut self, state: &State) -> io::Result<(BTreeSet<u64>, BTreeSet<u64>)> {
        let mut combats = BTreeSet::new();
        let mut results = BTreeSet::new();
        let key_of = |table: TableId, slot: u32| -> io::Result<u128> {
            columns::table(state, table)?
                .key(slot)
                .ok_or_else(|| invalid(format!("{table:?} slot {slot} holds no row")))
        };
        for &key in deleted(state, TableId::NavalCombats)? {
            combats.insert(key as u64);
        }
        for slot in written(state, TableId::NavalCombats, &[self.fields.combat_province])? {
            combats.insert(key_of(TableId::NavalCombats, slot)? as u64);
        }
        for &key in deleted(state, TableId::NavalCombatParticipants)? {
            self.participants.remove(&key);
            combats.insert((key >> 33) as u64);
        }
        for slot in written(state, TableId::NavalCombatParticipants, &[])? {
            let key = key_of(TableId::NavalCombatParticipants, slot)?;
            self.participants.insert(key, slot);
            combats.insert((key >> 33) as u64);
        }
        for &key in deleted(state, TableId::NavalCombatDetails)? {
            combats.insert(key as u64);
        }
        for slot in written(state, TableId::NavalCombatDetails, &self.fields.detail)? {
            combats.insert(key_of(TableId::NavalCombatDetails, slot)? as u64);
        }
        for &key in deleted(state, TableId::NavalResults)? {
            self.validity.result(key as u64, None);
            results.insert(key as u64);
        }
        for slot in written(state, TableId::NavalResults, &[])? {
            let key = key_of(TableId::NavalResults, slot)?;
            let operands = self.result_operands(state, slot)?;
            self.validity.result(key as u64, Some(operands));
            results.insert(key as u64);
        }
        for &key in deleted(state, TableId::NavalResultMembers)? {
            self.validity.member(key, None);
            results.insert((key >> 64) as u64);
        }
        for slot in written(state, TableId::NavalResultMembers, &[])? {
            let key = key_of(TableId::NavalResultMembers, slot)?;
            let side: u8 = value(
                state,
                TableId::NavalResultMembers,
                self.fields.member[2],
                slot,
            )?;
            self.validity.member(key, Some((slot, side > 1)));
            results.insert((key >> 64) as u64);
        }
        for &key in deleted(state, TableId::NavalResultCountries)? {
            self.validity.country(key, None);
            results.insert((key >> 40) as u64);
        }
        for slot in written(state, TableId::NavalResultCountries, &[])? {
            let key = key_of(TableId::NavalResultCountries, slot)?;
            self.validity.country(key, Some(slot));
            results.insert((key >> 40) as u64);
        }
        for &key in deleted(state, TableId::NavalCombatResultDetails)? {
            self.validity.details.remove(&(key as u64));
            results.insert(key as u64);
        }
        for slot in written(
            state,
            TableId::NavalCombatResultDetails,
            &[self.fields.result_detail],
        )? {
            let key = key_of(TableId::NavalCombatResultDetails, slot)? as u64;
            let row: NavalCombatResultDetailsRow =
                self.row(state, TableId::NavalCombatResultDetails, slot)?;
            if bad_classification(&row) {
                self.validity.details.insert(key);
            } else {
                self.validity.details.remove(&key);
            }
            results.insert(key);
        }
        Ok((combats, results))
    }

    /// The country tables, and the link tags moved to them. Whether the tags changed.
    fn countries(&mut self, world: &World, whole: bool) -> io::Result<(Countries, bool)> {
        let roster = world.countries()?;
        let known = roster.complete();
        // Balances move every country every day; only slots and tags name them.
        let read = known
            && (whole || self.reshaped[COUNTRIES] || self.reshaped[LINKS] || roster.changed());
        let (countries, rebuilt) = if !known {
            let _ = self.tags.update_projected(None, None);
            (Countries::Incomplete, false)
        } else if read {
            let keys = roster
                .iter()
                .map(|country| (country.id, country.slot, roster.tag(country.tag).to_vec()))
                .collect();
            let links = roster
                .links()
                .iter()
                .map(|link| (link.id, link.canonical_slot))
                .collect();
            let read = ProjectedCountries::new(keys, links);
            let rebuilt = self
                .tags
                .update_projected(Some(&read.keys), Some(&read.links))
                .unwrap_or(true);
            (Countries::Read(Box::new(read)), rebuilt)
        } else {
            (Countries::Same, false)
        };
        let flipped = std::mem::replace(&mut self.links_known, known) != known;
        Ok((countries, rebuilt || flipped))
    }
}

impl Projector<'_> {
    /// Combat `id`'s details row at `province` this hour, read once (`decoded`).
    fn detail<'d>(
        &self,
        state: &State,
        id: u64,
        province: u32,
        decoded: &'d mut Details,
    ) -> io::Result<&'d Option<NavalCombatDetailsRow>> {
        Ok(match decoded.entry(id) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => {
                let row = if columns::complete(state, TableId::NavalCombatDetails) {
                    match columns::table(state, TableId::NavalCombatDetails)?.slot(u128::from(id)) {
                        Some(at) => {
                            let row: NavalCombatDetailsRow =
                                self.row(state, TableId::NavalCombatDetails, at)?;
                            (row.province == province).then_some(row)
                        }
                        None => None,
                    }
                } else {
                    None
                };
                entry.insert(row)
            }
        })
    }

    /// Combat `id`'s sides at its NavalCombats `slot`, as the row path's census reads them.
    fn combat_sample(
        &self,
        state: &State,
        id: u64,
        slot: u32,
        decoded: &mut Details,
    ) -> io::Result<CombatSample> {
        let province: u32 = value(
            state,
            TableId::NavalCombats,
            self.fields.combat_province,
            slot,
        )?;
        // The participant codec keeps the side in the key; only a complete read samples them.
        let participants = columns::complete(state, TableId::NavalCombatParticipants);
        let mut members: [Vec<(u64, u32, u32)>; 2] = Default::default();
        if participants {
            let [ship, owner, status] = self.fields.participant;
            let first = u128::from(id) << 33;
            for (&key, &at) in self.participants.range(first..=first | ((1 << 33) - 1)) {
                let table = TableId::NavalCombatParticipants;
                members[(key >> 32 & 1) as usize].push((
                    value(state, table, ship, at)?,
                    value(state, table, owner, at)?,
                    value(state, table, status, at)?,
                ));
            }
        }
        let detail = self.detail(state, id, province, decoded)?;
        let mut sides = [SideSample::default(), SideSample::default()];
        for (which, side) in sides.iter_mut().enumerate() {
            let mut values = Quantities::default();
            // Combatant sides are 0 attacker / 1 defender; state 0x20 marks a sinking.
            for &(ship, _, status) in &members[which] {
                values.add(ship == 0, status & 0x20 != 0);
            }
            side.quantities = participants.then_some(values);
            side.sunk = side.quantities.map(|q| q.sunk);
            side.convoys_sunk = side.quantities.map(|q| q.convoys_sunk);
            side_countries(
                &self.tags,
                self.links_known,
                members[which]
                    .iter()
                    .map(|&(_, owner, _)| owner)
                    .chain(detail.iter().flat_map(|detail| {
                        let summary = &detail.sides[which].summary;
                        summary
                            .countries
                            .iter()
                            .chain(&summary.secondary_countries)
                            .copied()
                    })),
                detail.is_some(),
                side,
            );
        }
        Ok(CombatSample {
            id,
            province,
            sides,
        })
    }

    /// The active combat census, moved by the combats whose rows changed (`touched`).
    fn combats(
        &mut self,
        state: &State,
        all: bool,
        touched: &BTreeSet<u64>,
        decoded: &mut Details,
    ) -> io::Result<Combats> {
        if !columns::complete(state, TableId::NavalCombats) {
            self.combats.clear();
            return Ok(Combats::Unavailable);
        }
        if all {
            let mut samples = Vec::new();
            for (key, slot) in live(state, TableId::NavalCombats)? {
                samples.push(self.combat_sample(state, key as u64, slot, decoded)?);
            }
            let present: HashSet<u64> = samples.iter().map(|sample| sample.id).collect();
            let mut ended: Vec<u64> = self
                .combats
                .keys()
                .copied()
                .filter(|id| !present.contains(id))
                .collect();
            ended.sort_unstable();
            self.combats = samples
                .iter()
                .map(|sample| (sample.id, sample.clone()))
                .collect();
            return Ok(Combats::Changed {
                all: true,
                samples,
                ended,
            });
        }
        let (mut samples, mut ended) = (Vec::new(), Vec::new());
        for &id in touched {
            match columns::table(state, TableId::NavalCombats)?.slot(u128::from(id)) {
                Some(slot) => {
                    let sample = self.combat_sample(state, id, slot, decoded)?;
                    if self.combats.get(&id) != Some(&sample) {
                        self.combats.insert(id, sample.clone());
                        samples.push(sample);
                    }
                }
                None => {
                    if self.combats.remove(&id).is_some() {
                        ended.push(id);
                    }
                }
            }
        }
        Ok(if samples.is_empty() && ended.is_empty() {
            Combats::Same
        } else {
            Combats::Changed {
                all: false,
                samples,
                ended,
            }
        })
    }

    /// Result `id`'s inputs through the child tables a read can use (`usable`).
    fn result_input(
        &self,
        state: &State,
        id: u64,
        slot: u32,
        usable: [bool; 3],
    ) -> io::Result<ResultInput> {
        let row: NavalResultRow = self.row(state, TableId::NavalResults, slot)?;
        let members = if usable[0] {
            let [presence, ordinal, side] = self.fields.member;
            let table = TableId::NavalResultMembers;
            let first = u128::from(id) << 64;
            let mut rows = Vec::new();
            for (&key, &(at, _)) in self
                .validity
                .members
                .range(first..=first | u128::from(u64::MAX))
            {
                rows.push(NavalResultMemberRow {
                    result: id,
                    member: key as u64,
                    presence_value: value(state, table, presence, at)?,
                    ordinal: value(state, table, ordinal, at)?,
                    side: value(state, table, side, at)?,
                });
            }
            Some(rows)
        } else {
            None
        };
        let countries = if usable[1] {
            let first = u128::from(id) << 40;
            let mut rows = Vec::new();
            for (&key, &at) in self
                .validity
                .countries
                .range(first..=first | ((1 << 40) - 1))
            {
                rows.push(NavalResultCountryRow {
                    result: id,
                    country: key as u32,
                    ordinal: value(
                        state,
                        TableId::NavalResultCountries,
                        self.fields.result_country,
                        at,
                    )?,
                    list: (key >> 32) as u8,
                });
            }
            Some(rows)
        } else {
            None
        };
        let classification = if usable[2] {
            match columns::table(state, TableId::NavalCombatResultDetails)?.slot(u128::from(id)) {
                Some(at) => {
                    let detail: NavalCombatResultDetailsRow =
                        self.row(state, TableId::NavalCombatResultDetails, at)?;
                    Some(std::array::from_fn(|which| {
                        detail.sides[which]
                            .ships
                            .iter()
                            .map(|ship| (ship.member, ship.is_convoy))
                            .collect()
                    }))
                }
                None => None,
            }
        } else {
            None
        };
        Ok(ResultInput {
            row,
            members,
            countries,
            classification,
        })
    }

    /// The retained result census, moved by the results whose rows changed (`touched`).
    fn results(
        &mut self,
        state: &State,
        at: i32,
        whole: bool,
        changed: &[bool; FOLLOWED.len()],
        touched: BTreeSet<u64>,
    ) -> io::Result<Results> {
        if !columns::complete(state, TableId::NavalResults) {
            self.reread = true;
            return Ok(Results::Unavailable);
        }
        let complete = [
            TableId::NavalResultMembers,
            TableId::NavalResultCountries,
            TableId::NavalCombatResultDetails,
        ]
        .map(|table| columns::complete(state, table));
        let invalid = [
            self.validity.refused(at),
            if complete[0] {
                self.validity.member_sides
            } else {
                0
            },
            if complete[1] {
                self.validity.country_lists
            } else {
                0
            },
            if complete[2] {
                self.validity.details.len()
            } else {
                0
            },
        ];
        let usable = std::array::from_fn(|index| complete[index] && invalid[index + 1] == 0);
        let all = std::mem::take(&mut self.reread)
            || whole
            || self.usable != Some(usable)
            || [RESULTS, MEMBERS, RESULT_COUNTRIES, RESULT_DETAILS]
                .iter()
                .any(|&index| self.reshaped[index]);
        self.usable = Some(usable);
        if invalid[0] != 0 {
            // The read is refused before any result is sampled; the next reads every one.
            self.reread = true;
            return Ok(Results::Changed {
                all: true,
                invalid,
                inputs: Vec::new(),
                ended: Vec::new(),
            });
        }
        if all {
            let mut inputs = Vec::new();
            for (key, slot) in live(state, TableId::NavalResults)? {
                inputs.push(self.result_input(state, key as u64, slot, usable)?);
            }
            return Ok(Results::Changed {
                all: true,
                invalid,
                inputs,
                ended: Vec::new(),
            });
        }
        if ![RESULTS, MEMBERS, RESULT_COUNTRIES, RESULT_DETAILS]
            .iter()
            .any(|&index| changed[index])
        {
            return Ok(Results::Same);
        }
        let table = columns::table(state, TableId::NavalResults)?;
        let (mut inputs, mut ended) = (Vec::new(), Vec::new());
        for id in touched {
            match table.slot(u128::from(id)) {
                Some(slot) => inputs.push(self.result_input(state, id, slot, usable)?),
                None => ended.push(id),
            }
        }
        Ok(Results::Changed {
            all: false,
            invalid,
            inputs,
            ended,
        })
    }

    /// The LexicalTokens rows to read: all when read whole, else those the hour wrote.
    fn tokens(&self, state: &State, whole: bool) -> io::Result<Vec<(u32, Vec<u8>)>> {
        let slots: Vec<u32> = if whole || self.reshaped[TOKENS] {
            live(state, TableId::LexicalTokens)?
                .into_iter()
                .map(|(_, slot)| slot)
                .collect()
        } else {
            written(state, TableId::LexicalTokens, &[])?
        };
        let mut rows = Vec::with_capacity(slots.len());
        for slot in slots {
            let row: LexicalTokenRow = self.row(state, TableId::LexicalTokens, slot)?;
            rows.push((row.token, row.literal));
        }
        Ok(rows)
    }

    /// A slot read of the sunk ships' one row: what it first finds and newly refuses.
    fn ships(&mut self, state: &State, at: i32) -> io::Result<ShipRead> {
        if !self.ships_changed && self.base.settled(at) {
            return Ok(ShipRead::Read {
                found: Vec::new(),
                refused: Vec::new(),
            });
        }
        let slot = columns::table(state, TableId::SunkShips)?
            .live()
            .ones()
            .next()
            .ok_or_else(|| invalid("The sunk ships hold no row"))?;
        let row: SunkShipsRow = self.row(state, TableId::SunkShips, slot)?;
        let (found, refused) = self.base.compare(row.entries, at);
        self.compared = true;
        Ok(ShipRead::Read { found, refused })
    }

    /// Notes the convoy rows the hour changed among those the next read takes.
    fn note_convoys(&mut self, state: &State, whole: bool) -> io::Result<()> {
        if whole || self.reshaped[SUNK_CONVOYS] {
            self.convoys_changed = None;
            return Ok(());
        }
        let Some(keys) = &mut self.convoys_changed else {
            return Ok(());
        };
        // A deleted row is no read's; one deleted and inserted again is written.
        let table = columns::table(state, TableId::SunkConvoys)?;
        for slot in written(state, TableId::SunkConvoys, &[])? {
            keys.insert(
                table
                    .key(slot)
                    .ok_or_else(|| invalid(format!("SunkConvoys slot {slot} holds no row")))?,
            );
        }
        Ok(())
    }

    /// The SunkConvoys rows of `keys` (every row when None), ascending by key.
    fn convoys(
        &self,
        state: &State,
        keys: Option<&BTreeSet<u128>>,
    ) -> io::Result<Vec<SunkConvoyRow>> {
        let [convoys, ordinal] = self.fields.convoy;
        let table = TableId::SunkConvoys;
        let held = columns::table(state, table)?;
        let slots: Vec<(u128, u32)> = match keys {
            None => live(state, table)?,
            Some(keys) => keys
                .iter()
                .filter_map(|&key| held.slot(key).map(|slot| (key, slot)))
                .collect(),
        };
        let mut rows = Vec::with_capacity(slots.len());
        for (key, slot) in slots {
            rows.push(SunkConvoyRow {
                month: (key >> 64) as u32 as i32,
                killer_country: (key >> 32) as u32,
                owner_country: key as u32,
                convoys: value(state, table, convoys, slot)?,
                ordinal: value(state, table, ordinal, slot)?,
            });
        }
        Ok(rows)
    }

    /// The title operands of the ships first found this hour.
    fn sight(
        &mut self,
        hour: &segments::Hour<'_, World>,
        found: &[SunkShipEntry],
    ) -> io::Result<Option<(battle_titles::Sighting, catalogue::OwnRefusals)>> {
        let ships: Vec<sinkings::ShipEvent> = found
            .iter()
            .map(|entry| sinkings::ShipEvent {
                entry: entry.clone(),
                read_hour: hour.hour,
                victim: None,
                attacker: None,
                victim_as_of: None,
                attacker_as_of: None,
            })
            .filter(battle_titles::wanted)
            .collect();
        if ships.is_empty() || self.catalogue.is_none() {
            return Ok(None);
        }
        let _cpu = ::replay_frame::cpu::scope("naval projection titles", None);
        let wanted: Vec<&sinkings::ShipEvent> = ships.iter().collect();
        let state = hour.state;
        let provinces = columns::status(state, TableId::Provinces);
        let truncated = columns::truncated(state, TableId::Provinces);
        let held = columns::table(state, TableId::Provinces)?;
        let field = self.fields.victory_points;
        let lookup = |id: u32| -> Option<i32> {
            let slot = held.slot(u128::from(id))?;
            match provinces {
                Status::Complete => {}
                Status::Truncated { .. } => {
                    truncated?.binary_search(&slot).ok()?;
                }
                _ => return None,
            }
            value::<i32>(state, TableId::Provinces, field, slot).ok()
        };
        let points: Result<&dyn Fn(u32) -> Option<i32>, String> = match provinces {
            Status::NotRecorded => Err(format!(
                "hour {} {:?} is not recorded in a {} recording",
                hour.hour,
                TableId::Provinces,
                self.mode
            )),
            _ => Ok(&lookup),
        };
        let flag = AtomicBool::new(false);
        let at = (hour.branch, hour.hour);
        let (sighted, read) = view_reads::record(|| {
            battle_titles::sighting(
                self.source,
                self.context,
                at,
                &wanted,
                points.clone(),
                || hour.world.frame(state, &NAMING),
                &flag,
            )
        });
        if !read.iter().any(ViewIdentity::not_selected) {
            return sighted;
        }
        // The names read a table the naming frame lacks: read them again with every table.
        battle_titles::sighting(
            self.source,
            self.context,
            at,
            &wanted,
            points,
            || hour.world.frame(state, tables()),
            &flag,
        )
    }

    /// The labels of the present combats in `combats` (all when None), and their notes.
    fn fleets(
        &self,
        state: &State,
        combats: Option<&BTreeSet<u64>>,
        decoded: &mut Details,
    ) -> io::Result<(Vec<FleetLabels>, catalogue::OwnRefusals)> {
        let present = match combats {
            None => live(state, TableId::NavalCombats)?,
            Some(combats) => {
                let table = columns::table(state, TableId::NavalCombats)?;
                combats
                    .iter()
                    .filter_map(|&id| {
                        table
                            .slot(u128::from(id))
                            .map(|slot| (u128::from(id), slot))
                    })
                    .collect()
            }
        };
        let (labels, noted) = catalogue::capture_isolated(|| -> io::Result<Vec<FleetLabels>> {
            let mut labels = Vec::with_capacity(present.len());
            for (key, slot) in present {
                let combat = key as u64;
                let province: u32 = value(
                    state,
                    TableId::NavalCombats,
                    self.fields.combat_province,
                    slot,
                )?;
                let mut refused = catalogue::Refusals::default();
                let fleets = self.fleet_names(state, combat, province, decoded, &mut refused)?;
                labels.push((combat, province, fleets, refused));
            }
            Ok(labels)
        });
        Ok((labels?, noted))
    }

    /// [`super::details::fleet_names`] from the walker's columns.
    fn fleet_names(
        &self,
        state: &State,
        combat: u64,
        province: u32,
        decoded: &mut Details,
        refused: &mut catalogue::Refusals,
    ) -> io::Result<[NavalBattleFleetNames; 2]> {
        let Some(cat) = &self.catalogue else {
            return Ok(unknown_fleets("Fleet roster or content not recorded"));
        };
        let rosters = columns::complete(state, TableId::TaskForces)
            && columns::complete(state, TableId::Fleets);
        let Some(detail) = self
            .detail(state, combat, province, decoded)?
            .as_ref()
            .filter(|_| rosters)
        else {
            return Ok(unknown_fleets("Fleet identities not recorded"));
        };
        let tasks = columns::table(state, TableId::TaskForces)?;
        let fleets = columns::table(state, TableId::Fleets)?;
        let fleet_of = |reference: u64| {
            let slot = tasks.slot(u128::from(reference))?;
            value::<u64>(state, TableId::TaskForces, self.fields.task_fleet, slot).ok()
        };
        let fleet = |id: u64| {
            let slot = fleets.slot(u128::from(id))?;
            let row: FleetRow = self.row(state, TableId::Fleets, slot).ok()?;
            Some((row.id, row.name))
        };
        Ok(named_fleets(detail, fleet_of, fleet, cat, refused))
    }
}

impl Operator<World> for Projector<'_> {
    type Output = Hour;

    fn starts_at(&self, start: Start<'_>) -> bool {
        match start {
            // A later run of the branch compares its sunk ships with the read before its gap.
            Start::Run { continues } => !continues,
            Start::Restart(state) => match columns::status(state, TableId::SunkShips) {
                Status::NotRecorded => true,
                Status::Complete => {
                    columns::table(state, TableId::SunkShips)
                        .is_ok_and(|table| table.live().count() == 1)
                        && state.sampled_at(TableId::SunkShips.index()).is_some()
                }
                _ => false,
            },
        }
    }

    fn start(&mut self, start: Start<'_>, _: u64, _: i32) -> io::Result<()> {
        self.whole = true;
        self.seed = Some(None);
        if let Start::Restart(state) = start
            && columns::complete(state, TableId::SunkShips)
        {
            // The read the scan compared last: the restart's row, refused as at its read.
            let read = state
                .sampled_at(TableId::SunkShips.index())
                .ok_or_else(|| invalid("A restart's sunk ships hold no read hour"))?;
            let slot = columns::table(state, TableId::SunkShips)?
                .live()
                .ones()
                .next()
                .ok_or_else(|| invalid("A restart's sunk ships hold no row"))?;
            let row: SunkShipsRow = self.row(state, TableId::SunkShips, slot)?;
            self.base.compare(row.entries, read);
            self.seed = Some(Some(read));
        }
        Ok(())
    }

    fn cross(&mut self, _: i32) -> io::Result<()> {
        // The walker's tables start empty: every row is read anew.
        self.whole = true;
        self.previous = None;
        Ok(())
    }

    fn hour(
        &mut self,
        hour: &segments::Hour<'_, World>,
        out: &mut dyn Sink<Hour>,
    ) -> io::Result<()> {
        let _cpu = ::replay_frame::cpu::scope("naval projection", None);
        let state = hour.state;
        let at = hour.hour;
        // Every followed table reads as changed at a segment's first hour, as at a row block's.
        let whole = std::mem::take(&mut self.whole)
            || hour.opens
            || hour.starts
            || self.previous.and_then(|previous| previous.checked_add(1)) != Some(at);
        let mut sectioned = [false; TABLE_COUNT];
        for section in &hour.manifest.sections {
            if let Some(held) = sectioned.get_mut(section.table) {
                *held = true;
            }
        }
        let mut changed = [false; FOLLOWED.len()];
        for (index, &table) in FOLLOWED.iter().enumerate() {
            let now = Some(columns::status(state, table));
            self.reshaped[index] = std::mem::replace(&mut self.shapes[index], now) != now;
            changed[index] = whole
                || self.reshaped[index]
                || sectioned[table.index()]
                || touched(state, table, &[])?;
        }
        let (touched_combats, touched_results) = if whole {
            self.rebuild(state)?;
            Default::default()
        } else {
            self.maintain(state)?
        };
        let (countries, tags) = self.countries(hour.world, whole)?;
        let all = whole
            || tags
            || [COMBATS, PARTICIPANTS, DETAILS]
                .iter()
                .any(|&index| self.reshaped[index]);
        let mut decoded = Details::new();
        let combats = self.combats(state, all, &touched_combats, &mut decoded)?;
        let sources = if !columns::complete(state, TableId::NavalResultSources) {
            Sources::Unavailable
        } else if !changed[SOURCES] {
            Sources::Same
        } else {
            let mut rows = Vec::new();
            for (_, slot) in live(state, TableId::NavalResultSources)? {
                rows.push(self.row(state, TableId::NavalResultSources, slot)?);
            }
            Sources::Rows(rows)
        };
        let results = self.results(state, at, whole, &changed, touched_results)?;

        let cpu = ::replay_frame::cpu::scope("naval projection sinkings", None);
        let tokens = (columns::complete(state, TableId::LexicalTokens) && changed[TOKENS])
            .then(|| self.tokens(state, whole))
            .transpose()?;
        let measured = metrics(hour.manifest);
        let reads = slot_reads(self.format, &measured, at);
        self.ships_changed |= changed[SUNK_SHIPS];
        let single = columns::complete(state, TableId::SunkShips)
            && columns::table(state, TableId::SunkShips)?.live().count() == 1;
        let (ships, seeded, sighting) = if single && reads[TableId::SunkShips.index()] {
            let seeded = (!std::mem::replace(&mut self.ships_read, true))
                .then_some(self.seed)
                .flatten();
            let ships = self.ships(state, at)?;
            self.ships_changed = false;
            let sighting = match &ships {
                ShipRead::Read { found, .. } if !found.is_empty() => self.sight(hour, found)?,
                _ => None,
            };
            (ships, seeded, sighting)
        } else {
            (ShipRead::Skipped { single }, None, None)
        };
        if changed[SUNK_CONVOYS] {
            self.note_convoys(state, whole)?;
        }
        let complete = columns::complete(state, TableId::SunkConvoys);
        let convoys = if complete
            && (reads[TableId::SunkConvoys.index()]
                || measured.table_ns[TableId::SunkConvoys.index()] != 0)
        {
            let first = !std::mem::replace(&mut self.convoys_read, true);
            let keys = std::mem::replace(&mut self.convoys_changed, Some(BTreeSet::new()));
            match keys.filter(|_| !first) {
                Some(keys) if keys.is_empty() => ConvoyRead::Confirmed,
                keys => ConvoyRead::Read(self.convoys(state, keys.as_ref())?),
            }
        } else {
            ConvoyRead::Skipped { complete }
        };
        drop(cpu);

        let cpu = ::replay_frame::cpu::scope("naval projection fleets", None);
        // A label reads its combat's province and details, and the rosters every label reads.
        let rosters = whole
            || [COMBATS, DETAILS, TASK_FORCES, FLEETS]
                .iter()
                .any(|&index| self.reshaped[index])
            || touched(state, TableId::TaskForces, &[self.fields.task_fleet])?
            || touched(state, TableId::Fleets, &[self.fields.fleet_name])?;
        let fleets = if !columns::complete(state, TableId::NavalCombats) {
            None
        } else if rosters {
            Some(self.fleets(state, None, &mut decoded)?)
        } else if touched_combats.is_empty() {
            None
        } else {
            Some(self.fleets(state, Some(&touched_combats), &mut decoded)?)
        };
        drop(cpu);

        self.previous = Some(at);
        let projected = Hour {
            branch: hour.branch,
            hour: at,
            countries,
            combats,
            sources,
            results,
            tokens,
            ships,
            seeded,
            convoys,
            fleets,
            sighting,
            base: None,
        };
        match self.pending.replace(projected) {
            Some(before) => out.push(before),
            None => Ok(()),
        }
    }

    fn end(&mut self, out: &mut dyn Sink<Hour>) -> io::Result<()> {
        // The entries the next walk's first read compares with, after this segment's last.
        if let Some(mut last) = self.pending.take() {
            if std::mem::take(&mut self.compared) {
                last.base = Some(Box::new(self.base.clone()));
            }
            out.push(last)?;
        }
        self.forget();
        Ok(())
    }

    fn carried(&self) -> Option<usize> {
        // The source, context and catalogue are shared with every projector of the run.
        Some(self.base.heap_bytes() + self.tags.heap_bytes())
    }

    fn save(&self, out: &mut Vec<u8>) -> io::Result<bool> {
        serde_json::to_writer(
            out,
            &Saved {
                base: self.base.saved(),
                seeded: self.seed.is_some(),
                seed: self.seed.flatten(),
                ships_read: self.ships_read,
                convoys_read: self.convoys_read,
            },
        )?;
        Ok(true)
    }

    fn load(&mut self, bytes: &[u8]) -> io::Result<()> {
        let saved: Saved = serde_json::from_slice(bytes)?;
        self.base = ShipBase::restored(saved.base);
        self.seed = saved.seeded.then_some(saved.seed);
        self.ships_read = saved.ships_read;
        self.convoys_read = saved.convoys_read;
        // Every row is read anew.
        self.pending = None;
        self.compared = false;
        self.ships_changed = true;
        self.forget();
        Ok(())
    }
}
