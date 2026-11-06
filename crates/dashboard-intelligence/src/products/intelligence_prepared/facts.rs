//! The intelligence family's operator on a columnar walk ([`replay::segments`]). It holds the
//! tables its observers read ([`super::TABLES`]), each by the fields they read, and reads them as
//! each observer does: the cipher scan's joined pairs, the placements' census, the agencies' rows,
//! rosters and acquired levels, the roster's operatives, operations and captors' names, and the
//! espionage log's operations and completions. It hands on only what an hour read otherwise than
//! the segment's hour before; of the tables, only the countries and the rare completions reach the
//! emit ([`super::emit`]). Every segment starts cold: its first hour reads every table again and
//! hands each read on whole, and an hour after one it did not walk reads every table again and
//! reports every table changed.

use super::{
    Reads, TABLES,
    delta::{self, Keyed},
};
use crate::products::{
    agency_comparison::{self, Acquired, AgencyRead, AgencyTouched, ScannedUpgrades},
    country_name_cache::{SlotNames, slot_names},
    decryption::{self, CipherRead, CipherTouched, PairRead},
    espionage_log::{self, CensusRead, CensusTouched, Completion, OperationRead},
    operative_placements::{self, PlacementDelta, PlacementRead, PlacementTouched},
    operative_rows::CountryInputs,
    operatives_roster::{self, Experience, Flags, Member, Members, MembersTouched},
};
use dashboard_base::prepared::frame_changes::FrameChanges;
use dashboard_text::metadata::link_slot;
use dashboard_world::{Hold, Needs, World};
use replay::{
    columns,
    segments::{self, Operator, Sink, State},
};
use replay_data::rows::{
    AutonomyRow, CountryAppearanceRow, CountryRow, IntelligenceAgencyRow, PoliticalPartyRow,
    PoliticsRow, RaidIntelInputRow, RowCodec, TABLE_COUNT, TableId, TableSchema, tables,
};
use replay_frame::frame::{
    ColumnView, DecodedFrame, Metrics, READ_TIME_NOT_RECORDED, SharedView, TableView,
    not_selected,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    mem::size_of,
};

// An hour's change bits are kept one per table.
const _: () = assert!(READ.len() <= 32);

/// Each of [`TABLES`], in order, with the fields of it its observers read, by name: None for
/// every field, and none for Campaign, which is handed on where a segment opens.
const READ: [(TableId, Option<&[&str]>); 20] = [
    (TableId::Campaign, Some(&[])),
    // The country join reads the id (the row key), canonical slot and tag.
    (TableId::Countries, Some(&["canonical_slot", "tag"])),
    (TableId::CountryLinks, None),
    // Every stored cipher operand: amount, countdown and both flags.
    (TableId::Cryptology, None),
    // Each target's cryptology strength.
    (TableId::RaidIntelInputs, Some(&["crypto_strength"])),
    // The agency feature bit.
    (TableId::SimulationFeatures, Some(&["dlc_mask"])),
    (TableId::OperativeMemberships, None),
    (TableId::CharacterOperatives, None),
    (TableId::CharacterPortraits, None),
    // The agency scalars the comparison shows; upgrade progress and the admission ceiling are
    // not read.
    (
        TableId::IntelligenceAgencies,
        Some(&["defense_raw", "active_count", "unknown_f0", "name", "created"]),
    ),
    (TableId::AgencyUpgradeLevels, None),
    (TableId::LexicalTokens, None),
    (TableId::Operations, None),
    (TableId::OperationAssignments, None),
    (TableId::OperationCompletions, None),
    (
        TableId::CountryAppearance,
        Some(&["cosmetic_tag", "name_tag", "leader_ideology", "overlord"]),
    ),
    (
        TableId::ReservedCountryNaming,
        Some(&[
            "tag",
            "name_tag",
            "cosmetic_tag",
            "leader_ideology",
            "group",
            "overlord",
            "autonomy",
            "built",
        ]),
    ),
    (TableId::Politics, Some(&["ruling_party"])),
    (TableId::PoliticalParties, Some(&["token"])),
    (TableId::Autonomy, Some(&["state"])),
];

/// The table whose change facts report a change of any field, read or not: the agency scan
/// prunes and saves the rosters it holds at each hour the agencies change.
const AGENCIES: TableId = TableId::IntelligenceAgencies;

/// The fields of Operations the roster reads: each operation's definition, by its id.
const ROSTER_OPERATIONS: &[&str] = &["definition_token"];

/// Each of [`TABLES`] with the hold the walk's frame keeps it in and the fields read.
fn holds() -> io::Result<Vec<(TableId, Hold, u64)>> {
    if READ.iter().map(|&(table, _)| table).ne(TABLES.iter().copied()) {
        return Err(io::Error::other("The intelligence reads name other tables"));
    }
    let mut holds = Vec::with_capacity(READ.len());
    for &(table, read) in &READ {
        let (hold, fields) = match read {
            None => (Hold::Whole, u64::MAX),
            Some([]) => (Hold::Whole, 0),
            Some(names) => {
                let mut fields = 0_u64;
                for name in names {
                    fields |= 1 << columns::field(table, name)?;
                }
                (Hold::Fields(fields).of(table), fields)
            }
        };
        holds.push((table, hold, fields));
    }
    Ok(holds)
}

/// What a projection reads of its walk's world: the frame of [`TABLES`], each by its read
/// fields.
pub fn needs() -> io::Result<Needs> {
    Ok(Needs {
        frame: holds()?
            .into_iter()
            .map(|(table, hold, _)| (table, hold))
            .collect(),
        ..Needs::default()
    })
}

/// One walked hour of a branch: its change facts, and what the observers read of it otherwise
/// than the segment's hour before.
#[derive(replay_spool::Spooled)]
pub struct Hour {
    pub branch: u64,
    pub hour: i32,
    /// The hour follows no hour its segment walked: every table is read again and changed.
    pub opens: bool,
    /// By [`TABLES`] position: the tables the observers are told changed, and those the hour
    /// changed in any field, read or not.
    pub changed: u32,
    pub rebuilt: u32,
    /// Whether OperativeMemberships and Operations were read (bits 0 and 1): the only reads
    /// the observers date.
    pub reads: u8,
    /// Whether the hour may have changed the fields of Operations the roster reads; the other
    /// observers read every field ([`ROSTER_OPERATIONS`]).
    pub roster_operations: bool,
    pub(crate) flags: Flags,
    /// Countries and CountryLinks as the observers read them, where either changed.
    pub(crate) countries: Option<[SharedView; 2]>,
    pub(crate) cipher: Option<Box<CipherDelta>>,
    pub(crate) placements: Option<Box<PlacementDelta>>,
    pub(crate) agencies: Option<Box<AgencyDelta>>,
    pub(crate) members: Option<Box<MembersDelta>>,
    pub(crate) operations: Option<Option<Vec<(u64, Option<String>)>>>,
    pub(crate) names: Option<Box<Keyed<u32, Result<String, String>>>>,
    pub(crate) census: Option<Box<CensusDelta>>,
    pub(crate) completions: Option<Option<Vec<Completion>>>,
    /// What the hour's reads hold.
    held: usize,
}

/// The read bit of OperativeMemberships and of Operations in [`Hour::reads`].
pub const MEMBERSHIPS_READ: u8 = 1;
pub const OPERATIONS_READ: u8 = 2;

/// The cipher scan's read as read again: the pairs that changed or came, in row order, and the
/// raw keys of those gone, or the read whole.
#[derive(replay_spool::Spooled)]
pub(crate) enum CipherDelta {
    Whole(CipherRead),
    Pairs {
        incomplete: Option<String>,
        set: Vec<PairRead>,
        gone: Vec<(u32, u32)>,
    },
}

/// The agency scan's read as read again: its slots and acquired levels where built again, its
/// roster at a membership read.
#[derive(replay_spool::Spooled)]
pub(crate) struct AgencyDelta {
    complete: bool,
    refused: usize,
    slots: Option<Keyed<u32, Option<IntelligenceAgencyRow>>>,
    /// None where the memberships were not complete.
    roster: Option<Option<CountsDelta>>,
    acquired: Option<AcquiredDelta>,
}

/// A membership read's counts as counted again: each slot's count where it changed, with the
/// rows it could not count and those of unsupported rosters by kind.
#[derive(replay_spool::Spooled)]
pub(crate) struct CountsDelta {
    counts: Keyed<u32, u32>,
    unresolved: usize,
    unsupported: BTreeMap<u8, usize>,
}

#[derive(replay_spool::Spooled)]
pub(crate) enum AcquiredDelta {
    Whole(Result<Acquired, (io::ErrorKind, String)>),
    Slots {
        slots: Option<Keyed<u32, ScannedUpgrades>>,
        default: Option<ScannedUpgrades>,
    },
}

/// The roster's census as read again.
#[derive(replay_spool::Spooled)]
pub(crate) enum MembersDelta {
    Whole(Members),
    Slots {
        notes: Vec<(String, String)>,
        links_known: bool,
        slots: Vec<(u32, SlotPatch)>,
    },
}

/// A roster slot's members as read again.
#[derive(replay_spool::Spooled)]
pub(crate) enum SlotPatch {
    Whole(Vec<Member>),
    /// Only their experience moved: by position, that of each member it moved.
    Experience(Vec<(u32, Experience)>),
    Gone,
}

/// The espionage log's operations census as read again: the operations that changed or came,
/// in row order, and the handles of those gone, or the census whole.
#[derive(replay_spool::Spooled)]
pub(crate) enum CensusDelta {
    Whole(CensusRead),
    Rows {
        lexical: bool,
        set: Vec<OperationRead>,
        gone: Vec<u64>,
    },
}

/// What an hour's reads changed of those the emit holds, as each observer takes it.
#[derive(Default)]
pub(crate) struct Touched {
    pub(crate) cipher: CipherTouched,
    pub(crate) placements: PlacementTouched,
    pub(crate) agencies: AgencyTouched,
    pub(crate) members: MembersTouched,
    pub(crate) census: CensusTouched,
    /// The completions were handed on again.
    pub(crate) completions: bool,
}

impl Touched {
    /// Every observer goes through all it holds.
    fn everything(&mut self) {
        self.cipher.all = true;
        self.placements.all = true;
        self.agencies.all = true;
        self.members.all = true;
        self.census.all = true;
    }
}

impl Reads {
    /// The reads `hour` handed on, applied to those held: what they changed. An hour that opens
    /// changed everything.
    pub(crate) fn apply(&mut self, hour: &mut Hour) -> Touched {
        let mut touched = Touched::default();
        if hour.opens {
            touched.everything();
        }
        if let Some(cipher) = hour.cipher.take() {
            match *cipher {
                CipherDelta::Whole(read) => self.cipher.replace(read, &mut touched.cipher),
                CipherDelta::Pairs {
                    incomplete,
                    set,
                    gone,
                } => {
                    self.cipher.set_incomplete(incomplete);
                    for key in gone {
                        self.cipher.remove(key, &mut touched.cipher);
                    }
                    for pair in set {
                        self.cipher.set(pair, &mut touched.cipher);
                    }
                }
            }
        }
        if let Some(placements) = hour.placements.take() {
            PlacementRead::apply(&mut self.placements, *placements, &mut touched.placements);
        }
        if let Some(agencies) = hour.agencies.take() {
            let AgencyDelta {
                complete,
                refused,
                slots,
                roster,
                acquired,
            } = *agencies;
            let held = &mut self.agencies;
            let touch = &mut touched.agencies;
            held.complete = complete;
            held.refused = refused;
            if let Some(slots) = slots {
                touch_slots(touch, &slots);
                slots.apply(&mut held.slots);
            }
            match roster {
                Some(None) => {
                    held.roster = None;
                    touch.all = true;
                }
                Some(Some(CountsDelta {
                    counts,
                    unresolved,
                    unsupported,
                })) => {
                    touch_slots(touch, &counts);
                    let (by_slot, held_unresolved, held_unsupported) =
                        held.roster.get_or_insert_with(Default::default);
                    counts.apply(by_slot);
                    *held_unresolved = unresolved;
                    *held_unsupported = unsupported;
                }
                None => {}
            }
            match acquired {
                Some(AcquiredDelta::Whole(acquired)) => {
                    held.acquired = acquired;
                    touch.all = true;
                }
                Some(AcquiredDelta::Slots { slots, default }) => {
                    if let Ok((by_slot, held_default)) = &mut held.acquired {
                        if let Some(slots) = slots {
                            touch_slots(touch, &slots);
                            slots.apply(by_slot);
                        }
                        if let Some(default) = default {
                            *held_default = default;
                            touch.all = true;
                        }
                    }
                }
                None => {}
            }
        }
        if let Some(members) = hour.members.take() {
            match *members {
                MembersDelta::Whole(members) => {
                    self.roster.members = Some(members);
                    touched.members.all = true;
                }
                MembersDelta::Slots {
                    notes,
                    links_known,
                    slots,
                } => {
                    if let Some(held) = &mut self.roster.members {
                        touched.members.all |= held.links_known != links_known;
                        held.notes = notes;
                        held.links_known = links_known;
                        for (slot, patch) in slots {
                            touched.members.slots.push(slot);
                            match patch {
                                SlotPatch::Whole(members) => {
                                    held.by_slot.insert(slot, members);
                                }
                                SlotPatch::Experience(moved) => {
                                    if let Some(members) = held.by_slot.get_mut(&slot) {
                                        for (position, experience) in &moved {
                                            members[*position as usize].gain(experience);
                                        }
                                    }
                                }
                                SlotPatch::Gone => {
                                    held.by_slot.remove(&slot);
                                }
                            }
                        }
                    }
                }
            }
        }
        if let Some(operations) = hour.operations.take() {
            self.roster.operations = operations;
        }
        if let Some(names) = hour.names.take() {
            names.apply(&mut self.roster.names);
        }
        if let Some(census) = hour.census.take() {
            match *census {
                CensusDelta::Whole(census) => self.operations.replace(census, &mut touched.census),
                CensusDelta::Rows { lexical, set, gone } => {
                    self.operations
                        .patch(lexical, set, gone, &mut touched.census)
                }
            }
        }
        if let Some(completions) = hour.completions.take() {
            self.before = std::mem::replace(&mut self.completions, completions);
            touched.completions = true;
        }
        touched
    }
}

/// Tells `touched` the slots `keyed` changed: every slot where it was taken whole.
fn touch_slots<V>(touched: &mut AgencyTouched, keyed: &Keyed<u32, V>) {
    touched.all |= keyed.whole;
    touched
        .slots
        .extend(keyed.set.iter().map(|(slot, _)| *slot));
    touched.slots.extend(keyed.gone.iter().copied());
}

/// What one view keeps alive: its allocation and its rows or reason.
fn view_bytes(view: &TableView) -> usize {
    let held = match view {
        TableView::Complete(rows) | TableView::Truncated { rows, .. } => rows.heap_bytes(),
        TableView::Unavailable { reason } => reason.capacity(),
        TableView::NotRecorded { .. } => 0,
    };
    size_of::<TableView>() + 2 * size_of::<usize>() + held
}

/// `view` with every row reduced by `reduce`, or the view `held` last reduced while the reduced
/// rows are equal. A view that is not complete is handed on as it is.
fn reduced<T: TableSchema>(
    view: &SharedView,
    held: &mut Option<SharedView>,
    reduce: impl Fn(&T::Row) -> T::Row,
) -> SharedView
where
    T::Row: PartialEq,
{
    let TableView::Complete(rows) = &**view else {
        return view.clone();
    };
    let Some(rows) = T::borrow(rows) else {
        return view.clone();
    };
    let rows: Vec<T::Row> = rows.iter().map(reduce).collect();
    if let Some(previous) = held.as_ref()
        && let TableView::Complete(previous) = &**previous
        && T::borrow(previous) == Some(rows.as_slice())
    {
        return held.clone().expect("a held reduced view");
    }
    let view: SharedView = TableView::Complete(T::wrap(rows)).into();
    *held = Some(view.clone());
    view
}

fn country(row: &CountryRow) -> CountryRow {
    CountryRow {
        id: row.id,
        canonical_slot: row.canonical_slot,
        tag: row.tag.clone(),
        ..Default::default()
    }
}

fn raid(row: &RaidIntelInputRow) -> RaidIntelInputRow {
    RaidIntelInputRow {
        country: row.country,
        crypto_strength: row.crypto_strength,
        ..Default::default()
    }
}

fn agency(row: &IntelligenceAgencyRow) -> IntelligenceAgencyRow {
    IntelligenceAgencyRow {
        country: row.country,
        defense_raw: row.defense_raw,
        active_count: row.active_count,
        unknown_f0: row.unknown_f0,
        name: row.name.clone(),
        created: row.created,
        ..Default::default()
    }
}

fn appearance(row: &CountryAppearanceRow) -> CountryAppearanceRow {
    CountryAppearanceRow {
        country: row.country,
        cosmetic_tag: row.cosmetic_tag.clone(),
        name_tag: row.name_tag.clone(),
        leader_ideology: row.leader_ideology.clone(),
        overlord: row.overlord,
        ..Default::default()
    }
}

fn politics(row: &PoliticsRow) -> PoliticsRow {
    PoliticsRow {
        country: row.country,
        ruling_party: row.ruling_party,
        ..Default::default()
    }
}

fn party(row: &PoliticalPartyRow) -> PoliticalPartyRow {
    PoliticalPartyRow {
        country: row.country,
        id: row.id,
        token: row.token.clone(),
        ..Default::default()
    }
}

fn autonomy(row: &AutonomyRow) -> AutonomyRow {
    AutonomyRow {
        subject: row.subject,
        state: row.state.clone(),
        ..Default::default()
    }
}

/// `view` of `table` as the observers read it: the walk's frame holds a table by some fields
/// with its others as last built, so those tables keep only the fields read, as default, and
/// equal reads share one view.
fn read_view(table: TableId, view: &SharedView, held: &mut Option<SharedView>) -> SharedView {
    match table {
        TableId::Countries => reduced::<tables::Countries>(view, held, country),
        TableId::RaidIntelInputs => reduced::<tables::RaidIntelInputs>(view, held, raid),
        TableId::IntelligenceAgencies => {
            reduced::<tables::IntelligenceAgencies>(view, held, agency)
        }
        TableId::CountryAppearance => reduced::<tables::CountryAppearance>(view, held, appearance),
        TableId::Politics => reduced::<tables::Politics>(view, held, politics),
        TableId::PoliticalParties => reduced::<tables::PoliticalParties>(view, held, party),
        TableId::Autonomy => reduced::<tables::Autonomy>(view, held, autonomy),
        _ => view.clone(),
    }
}

/// `rows`, ascending by key, with the rows `set` and the keys `gone` applied, both ascending.
fn merge<R: RowCodec>(rows: &[R], set: Vec<R>, gone: &[u128]) -> Vec<R> {
    let mut merged = Vec::with_capacity(rows.len() + set.len());
    let mut set = set.into_iter().peekable();
    let mut gone = gone.iter().copied().peekable();
    for row in rows {
        let key = row.key();
        while let Some(next) = set.next_if(|next| next.key() < key) {
            merged.push(next);
        }
        if let Some(next) = set.next_if(|next| next.key() == key) {
            merged.push(next);
            continue;
        }
        while gone.next_if(|&gone| gone < key).is_some() {}
        if gone.next_if(|&gone| gone == key).is_some() {
            continue;
        }
        merged.push(row.clone());
    }
    merged.extend(set);
    merged
}

/// `held`, a complete table as the frame last read it, with the rows the hour's own changes name
/// read again by `mask` and `reduce`: `held` itself where none reads otherwise; None where the
/// changes cannot tell.
fn patched<T: TableSchema>(
    world: &World,
    state: &State,
    (table, mask): (TableId, u64),
    held: &SharedView,
    reduce: impl Fn(&T::Row) -> T::Row,
) -> io::Result<Option<SharedView>> {
    let (TableView::Complete(old), ColumnView::Complete(columns), Some(changes)) = (
        &**held,
        world.view(state, table),
        state.changes(table.index()),
    ) else {
        return Ok(None);
    };
    let Some(old) = T::borrow(old) else {
        return Ok(None);
    };
    if changes.truncated {
        return Ok(None);
    }
    let at = |key: u128| old.binary_search_by_key(&key, |row| row.key());
    let mut slots: Vec<u32> = changes
        .changed
        .iter()
        .chain(&changes.inserted)
        .chain(&changes.fill_slots)
        .copied()
        .collect();
    slots.sort_unstable();
    slots.dedup();
    let mut set = Vec::new();
    for slot in slots {
        // A slot deleted since holds no row.
        let Some(key) = columns.table().key(slot) else {
            continue;
        };
        let row = reduce(&columns.row::<T::Row>(slot, mask)?);
        if !at(key).is_ok_and(|at| old[at] == row) {
            set.push(row);
        }
    }
    let mut gone: Vec<u128> = changes
        .deleted_keys
        .iter()
        .copied()
        .filter(|&key| columns.find(key).is_none() && at(key).is_ok())
        .collect();
    if set.is_empty() && gone.is_empty() {
        return Ok(Some(held.clone()));
    }
    set.sort_unstable_by_key(|row| row.key());
    gone.sort_unstable();
    gone.dedup();
    let rows = merge(old, set, &gone);
    Ok(Some(TableView::Complete(T::wrap(rows)).into()))
}

/// [`patched`] of `ask`'s table, its rows read as [`read_view`] reads them.
fn patched_view(
    world: &World,
    state: &State,
    ask: (TableId, u64),
    held: &SharedView,
) -> io::Result<Option<SharedView>> {
    let (w, s, h) = (world, state, held);
    match ask.0 {
        TableId::Countries => patched::<tables::Countries>(w, s, ask, h, country),
        TableId::RaidIntelInputs => patched::<tables::RaidIntelInputs>(w, s, ask, h, raid),
        TableId::IntelligenceAgencies => {
            patched::<tables::IntelligenceAgencies>(w, s, ask, h, agency)
        }
        TableId::CountryAppearance => {
            patched::<tables::CountryAppearance>(w, s, ask, h, appearance)
        }
        TableId::Politics => patched::<tables::Politics>(w, s, ask, h, politics),
        TableId::PoliticalParties => patched::<tables::PoliticalParties>(w, s, ask, h, party),
        TableId::Autonomy => patched::<tables::Autonomy>(w, s, ask, h, autonomy),
        TableId::CountryLinks => patched::<tables::CountryLinks>(w, s, ask, h, Clone::clone),
        TableId::Cryptology => patched::<tables::Cryptology>(w, s, ask, h, Clone::clone),
        TableId::SimulationFeatures => {
            patched::<tables::SimulationFeatures>(w, s, ask, h, Clone::clone)
        }
        TableId::OperativeMemberships => {
            patched::<tables::OperativeMemberships>(w, s, ask, h, Clone::clone)
        }
        TableId::CharacterOperatives => {
            patched::<tables::CharacterOperatives>(w, s, ask, h, Clone::clone)
        }
        TableId::CharacterPortraits => {
            patched::<tables::CharacterPortraits>(w, s, ask, h, Clone::clone)
        }
        TableId::AgencyUpgradeLevels => {
            patched::<tables::AgencyUpgradeLevels>(w, s, ask, h, Clone::clone)
        }
        TableId::LexicalTokens => patched::<tables::LexicalTokens>(w, s, ask, h, Clone::clone),
        TableId::Operations => patched::<tables::Operations>(w, s, ask, h, Clone::clone),
        TableId::OperationAssignments => {
            patched::<tables::OperationAssignments>(w, s, ask, h, Clone::clone)
        }
        TableId::OperationCompletions => {
            patched::<tables::OperationCompletions>(w, s, ask, h, Clone::clone)
        }
        TableId::ReservedCountryNaming => {
            patched::<tables::ReservedCountryNaming>(w, s, ask, h, Clone::clone)
        }
        _ => Ok(None),
    }
}

/// Whether [`read_view`] reduces `table`'s rows, so its views carry no sample hour.
fn reduces(table: TableId) -> bool {
    matches!(
        table,
        TableId::Countries
            | TableId::RaidIntelInputs
            | TableId::IntelligenceAgencies
            | TableId::CountryAppearance
            | TableId::Politics
            | TableId::PoliticalParties
            | TableId::Autonomy
    )
}

/// `table` this hour as the observers read it ([`read_view`]), the frame holding it as `held`:
/// where the hour does not open, by the rows its own changes name ([`patched_view`]); else, or
/// where they cannot tell, copied whole from the walk's views.
fn read_table(
    world: &World,
    state: &State,
    (table, hold, fields): (TableId, Hold, u64),
    fresh: bool,
    held: &SharedView,
    reduced: &mut Option<SharedView>,
) -> io::Result<SharedView> {
    let mask = match hold.of(table) {
        Hold::Whole => table.full_mask(),
        Hold::Fields(fields) => fields,
    };
    // Fields held beyond those whose changes ask the table again go stale between asks.
    if !fresh
        && mask & !fields == 0
        && let Some(view) = patched_view(world, state, (table, mask), held)?
    {
        if !reduces(table) {
            return Ok(view.with_sampled_at(state.sampled_at(table.index())));
        }
        if !SharedView::ptr_eq(&view, held) {
            *reduced = Some(view.clone());
        }
        return Ok(view);
    }
    let copied = world.copy(state, table, hold)?;
    Ok(read_view(table, &copied, reduced))
}

/// The tables whose change makes the roster's census read again, beside the fields of Operations
/// naming its operations ([`ROSTER_OPERATIONS`]).
const MEMBERSHIP_INPUTS: [TableId; 5] = [
    TableId::OperativeMemberships,
    TableId::SimulationFeatures,
    TableId::CharacterOperatives,
    TableId::CharacterPortraits,
    TableId::LexicalTokens,
];

/// The tables a captor's name is read from.
const NAMING: [TableId; 7] = [
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CountryAppearance,
    TableId::ReservedCountryNaming,
    TableId::Politics,
    TableId::PoliticalParties,
    TableId::Autonomy,
];

/// What the segment handed on of each read, as the emit holds it: none before its first.
#[derive(Default)]
struct Sent {
    cipher: Option<CipherRead>,
    placements: Option<PlacementRead>,
    agencies: Option<AgencyRead>,
    members: Option<Members>,
    operations: Option<Option<Vec<(u64, Option<String>)>>>,
    names: Option<SlotNames>,
    census: Option<CensusRead>,
    completions: Option<Option<Vec<Completion>>>,
}

/// One segment's operator.
pub struct Projector<'p> {
    content: &'p super::Content,
    /// Each of [`TABLES`], by position: its hold and the fields read.
    holds: Vec<(TableId, Hold, u64)>,
    /// The fields of Operations the roster reads, a bit by column.
    roster_operations: u64,
    /// Nothing is held: the next hour reads every table, as a segment's first.
    cold: bool,
    last: Option<i32>,
    /// Every table as last read, each by its read fields; by position, the last reduced view of
    /// a reduced table.
    frame: DecodedFrame,
    reduced: Vec<Option<SharedView>>,
    /// The positions of the tables the hour reads again, kept from hour to hour.
    asked: Vec<usize>,
    countries: CountryInputs,
    cipher: decryption::Reader,
    agencies: agency_comparison::Reader,
    roster: operatives_roster::Reader,
    /// The placements' and the roster's census must be read again at the next membership read.
    placements_dirty: bool,
    members_dirty: bool,
    /// The slots a captor of the segment's last membership read can be in: none before it,
    /// when every slot's name is read.
    captors: Option<BTreeSet<u32>>,
    sent: Sent,
    /// The country views last handed on: a view handed on again is charged only once.
    handed_countries: Option<[SharedView; 2]>,
}

/// What [`Projector::save`] writes: nothing but that it restarts, since every table is read
/// again from the walker's tables at the next hour, as at a segment's first.
const SAVED: &[u8] = b"intelligence projector: cold";

impl<'p> Projector<'p> {
    pub fn new(content: &'p super::Content) -> io::Result<Self> {
        let mut roster_operations = 0_u64;
        for name in ROSTER_OPERATIONS {
            roster_operations |= 1 << columns::field(TableId::Operations, name)?;
        }
        Ok(Self {
            content,
            holds: holds()?,
            roster_operations,
            cold: true,
            last: None,
            frame: DecodedFrame {
                branch: 0,
                hour: 0,
                tables: std::array::from_fn(|_| not_selected()),
                metrics: Metrics::default(),
            },
            reduced: vec![None; TABLES.len()],
            asked: Vec::with_capacity(TABLES.len()),
            countries: CountryInputs::default(),
            cipher: decryption::Reader::default(),
            agencies: agency_comparison::Reader::default(),
            roster: operatives_roster::Reader::default(),
            placements_dirty: true,
            members_dirty: true,
            captors: None,
            sent: Sent::default(),
            handed_countries: None,
        })
    }

    fn restart(&mut self) {
        self.cold = true;
        self.last = None;
        self.frame.tables = std::array::from_fn(|_| not_selected());
        self.reduced.fill(None);
        self.countries = CountryInputs::default();
        self.cipher = decryption::Reader::default();
        self.agencies = agency_comparison::Reader::default();
        self.roster = operatives_roster::Reader::default();
        self.placements_dirty = true;
        self.members_dirty = true;
        self.captors = None;
        self.sent = Sent::default();
        self.handed_countries = None;
    }

    /// What the frame's views and the reads handed on hold.
    fn held_bytes(&self) -> usize {
        let views: usize = self
            .frame
            .tables
            .iter()
            .chain(self.reduced.iter().flatten())
            .map(|view| view_bytes(view))
            .sum();
        let sent = &self.sent;
        let cipher = match &sent.cipher {
            Some(CipherRead::Rows { pairs, .. }) => {
                pairs.capacity() * size_of::<PairRead>()
                    + pairs.iter().map(PairRead::heap).sum::<usize>()
            }
            _ => 0,
        };
        let agencies = sent.agencies.as_ref().map_or(0, |read| {
            read.slots
                .values()
                .map(|row| size_of::<(u32, Option<IntelligenceAgencyRow>)>() + agency_comparison::agency_heap(row))
                .sum::<usize>()
                + read.acquired.as_ref().map_or(0, |(slots, _)| {
                    slots
                        .values()
                        .map(|upgrades| {
                            size_of::<(u32, ScannedUpgrades)>()
                                + agency_comparison::upgrades_heap(upgrades)
                        })
                        .sum()
                })
        });
        let members = sent
            .members
            .as_ref()
            .map_or(0, |members| groups(members.by_slot.values()));
        let names = sent.names.as_ref().map_or(0, |names| {
            names
                .values()
                .map(|name| match name {
                    Ok(name) | Err(name) => size_of::<(u32, Result<String, String>)>() + name.capacity(),
                })
                .sum()
        });
        let census = sent
            .census
            .as_ref()
            .and_then(|census| census.rows.as_ref())
            .map_or(0, |rows| {
                rows.capacity() * size_of::<OperationRead>()
                    + rows.iter().map(OperationRead::heap).sum::<usize>()
            });
        let completions = sent.completions.iter().flatten().flatten().map(|completion| {
            size_of::<Completion>() + espionage_log::completion_heap(completion)
        });
        views
            + cipher
            + sent.placements.as_ref().map_or(0, PlacementRead::heap)
            + agencies
            + members
            + names
            + census
            + completions.sum::<usize>()
    }
}

/// `new` where it differs from `sent`, which it becomes.
fn whole<T: Clone + PartialEq>(sent: &mut Option<T>, new: T) -> Option<T> {
    if sent.as_ref() == Some(&new) {
        return None;
    }
    *sent = Some(new.clone());
    Some(new)
}

/// What `new` changed of the cipher read `sent`, which it becomes.
fn cipher_delta(sent: &mut Option<CipherRead>, new: CipherRead) -> Option<CipherDelta> {
    let delta = match (sent.as_ref(), &new) {
        (
            Some(CipherRead::Rows {
                incomplete: held,
                pairs: old,
            }),
            CipherRead::Rows { incomplete, pairs },
        ) => {
            let (set, gone) = delta::sorted(old, pairs, PairRead::key);
            if set.is_empty() && gone.is_empty() && held == incomplete {
                return None;
            }
            CipherDelta::Pairs {
                incomplete: incomplete.clone(),
                set,
                gone,
            }
        }
        (Some(held), _) if *held == new => return None,
        _ => CipherDelta::Whole(new.clone()),
    };
    *sent = Some(new);
    Some(delta)
}

/// What `new` changed of the agency read `sent`, which it becomes: its slots and acquired levels
/// where `rebuilt`, its roster at a membership `read`.
fn agency_delta(
    sent: &mut Option<AgencyRead>,
    new: &AgencyRead,
    rebuilt: (bool, bool),
    read: bool,
) -> Option<Box<AgencyDelta>> {
    let Some(held) = sent.as_mut() else {
        *sent = Some(new.clone());
        return Some(Box::new(AgencyDelta {
            complete: new.complete,
            refused: new.refused,
            slots: Keyed::of(None, &new.slots),
            roster: Some(counts_delta(None, new.roster.as_ref())),
            acquired: Some(AcquiredDelta::Whole(new.acquired.clone())),
        }));
    };
    let slots = rebuilt
        .0
        .then(|| Keyed::of(Some(&held.slots), &new.slots))
        .flatten();
    let roster = (read && held.roster != new.roster)
        .then(|| counts_delta(held.roster.as_ref(), new.roster.as_ref()));
    let acquired = if !rebuilt.1 || held.acquired == new.acquired {
        None
    } else {
        match (&held.acquired, &new.acquired) {
            (Ok((held_slots, held_default)), Ok((slots, default))) => Some(AcquiredDelta::Slots {
                slots: Keyed::of(Some(held_slots), slots),
                default: (held_default != default).then(|| default.clone()),
            }),
            _ => Some(AcquiredDelta::Whole(new.acquired.clone())),
        }
    };
    if slots.is_none()
        && roster.is_none()
        && acquired.is_none()
        && held.complete == new.complete
        && held.refused == new.refused
    {
        return None;
    }
    held.complete = new.complete;
    held.refused = new.refused;
    if rebuilt.0 {
        held.slots.clone_from(&new.slots);
    }
    if roster.is_some() {
        held.roster.clone_from(&new.roster);
    }
    if acquired.is_some() {
        held.acquired.clone_from(&new.acquired);
    }
    Some(Box::new(AgencyDelta {
        complete: new.complete,
        refused: new.refused,
        slots,
        roster,
        acquired,
    }))
}

/// What the membership counts `new` changed of `old`.
fn counts_delta(
    old: Option<&agency_comparison::RosterCounts>,
    new: Option<&agency_comparison::RosterCounts>,
) -> Option<CountsDelta> {
    let (counts, unresolved, unsupported) = new?;
    Some(CountsDelta {
        counts: Keyed::of(old.map(|(counts, _, _)| counts), counts).unwrap_or(Keyed {
            whole: false,
            set: Vec::new(),
            gone: Vec::new(),
        }),
        unresolved: *unresolved,
        unsupported: unsupported.clone(),
    })
}

/// What `new` changed of the roster census `sent`, which it becomes.
fn members_delta(sent: &mut Option<Members>, new: Members) -> Option<MembersDelta> {
    let delta = match sent.as_ref() {
        None => MembersDelta::Whole(new.clone()),
        Some(held) => {
            let mut slots: Vec<(u32, SlotPatch)> = new
                .by_slot
                .iter()
                .filter_map(|(&slot, members)| {
                    let patch = match held.by_slot.get(&slot) {
                        Some(old) if old == members => return None,
                        Some(old) if old.len() == members.len() => {
                            match experience_moved(old, members) {
                                Some(moved) => SlotPatch::Experience(moved),
                                None => SlotPatch::Whole(members.clone()),
                            }
                        }
                        _ => SlotPatch::Whole(members.clone()),
                    };
                    Some((slot, patch))
                })
                .collect();
            slots.extend(
                held.by_slot
                    .keys()
                    .filter(|slot| !new.by_slot.contains_key(*slot))
                    .map(|&slot| (slot, SlotPatch::Gone)),
            );
            if slots.is_empty() && held.notes == new.notes && held.links_known == new.links_known
            {
                return None;
            }
            MembersDelta::Slots {
                notes: new.notes.clone(),
                links_known: new.links_known,
                slots,
            }
        }
    };
    *sent = Some(new);
    Some(delta)
}

/// By position, the experience of each of `new` that moved from `old`, where that is all that
/// changed: none where any changed otherwise.
fn experience_moved(old: &[Member], new: &[Member]) -> Option<Vec<(u32, Experience)>> {
    let mut moved = Vec::new();
    for (position, (old, new)) in (0_u32..).zip(old.iter().zip(new)) {
        if old != new {
            moved.push((position, old.experience_moved(new)?));
        }
    }
    Some(moved)
}

/// What `new` changed of the operations census `sent`, which it becomes.
fn census_delta(sent: &mut Option<CensusRead>, new: CensusRead) -> Option<CensusDelta> {
    let delta = match (sent.as_ref(), &new.rows) {
        (
            Some(CensusRead {
                rows: Some(old),
                lexical,
            }),
            Some(rows),
        ) => {
            let (set, gone) = delta::sorted(old, rows, |row: &OperationRead| row.id);
            if set.is_empty() && gone.is_empty() && *lexical == new.lexical {
                return None;
            }
            CensusDelta::Rows {
                lexical: new.lexical,
                set,
                gone,
            }
        }
        (Some(held), _) if *held == new => return None,
        _ => CensusDelta::Whole(new.clone()),
    };
    *sent = Some(new);
    Some(delta)
}

/// The slots a captor of `frame`'s membership read can be in: the reserved slot, and each
/// captured operative's captor's.
fn captors(frame: &DecodedFrame, countries: &CountryInputs) -> BTreeSet<u32> {
    let mut slots = BTreeSet::from([0]);
    if let (Some((_, links)), Ok(rows)) = (
        countries.complete(),
        frame.complete::<tables::OperativeMemberships>(),
    ) {
        slots.extend(
            rows.iter()
                .filter(|row| row.assignment == 0 && row.capturer_country != 0)
                .filter_map(|row| link_slot(links, row.capturer_country)),
        );
    }
    slots
}

impl Operator<World> for Projector<'_> {
    type Output = Hour;

    fn start(&mut self, _: segments::Start<'_>, _: u64, _: i32) -> io::Result<()> {
        self.restart();
        Ok(())
    }

    fn hour(
        &mut self,
        hour: &segments::Hour<'_, World>,
        out: &mut dyn Sink<Hour>,
    ) -> io::Result<()> {
        let _cpu = replay_frame::cpu::scope("intelligence.columnar", Some("intelligence"));
        let at = hour.hour;
        let world = hour.world;
        let opens = self.cold
            || hour.starts
            || hour.opens
            || self.last.and_then(|last| last.checked_add(1)) != Some(at);
        self.cold = false;
        self.last = Some(at);
        if opens {
            // The emit binds its reads afresh where an hour opens: each is handed on whole.
            self.sent = Sent::default();
        }
        let (mut changed, mut rebuilt) = (0_u32, 0_u32);
        self.asked.clear();
        for (position, &(table, _, fields)) in self.holds.iter().enumerate() {
            if !opens && !world.touched(table, u64::MAX) {
                continue;
            }
            rebuilt |= 1 << position;
            if opens || world.touched(table, fields) {
                changed |= 1 << position;
                self.asked.push(position);
            } else if table == AGENCIES {
                changed |= 1 << position;
            }
        }
        // Each table the hour changed in a field read, read again from the walk's views in its
        // hold.
        let fresh = opens || world.first();
        for &position in &self.asked {
            let ask = self.holds[position];
            let held = &mut self.frame.tables[ask.0.index()];
            let reduced = &mut self.reduced[position];
            let view = read_table(world, hour.state, ask, fresh, held, reduced)?;
            if !SharedView::ptr_eq(held, &view) || held.sampled_at() != view.sampled_at() {
                *held = view;
            }
        }
        self.frame.branch = hour.branch;
        self.frame.hour = at;
        let read = |table: TableId| world.metrics().table_ns[table.index()] != 0;
        let reads = if read(TableId::OperativeMemberships) {
            MEMBERSHIPS_READ
        } else {
            0
        } | if read(TableId::Operations) {
            OPERATIONS_READ
        } else {
            0
        };
        let mut table_read = [false; TABLE_COUNT];
        for (table, bit) in [
            (TableId::OperativeMemberships, MEMBERSHIPS_READ),
            (TableId::Operations, OPERATIONS_READ),
        ] {
            let read = reads & bit != 0;
            table_read[table.index()] = read;
            self.frame.metrics.table_ns[table.index()] = if read {
                READ_TIME_NOT_RECORDED
            } else {
                0
            };
        }
        let roster_operations =
            opens || world.touched(TableId::Operations, self.roster_operations);
        let mut rows_changed = [opens; TABLE_COUNT];
        for (position, table) in TABLES.iter().enumerate() {
            rows_changed[table.index()] |= changed & (1 << position) != 0;
        }
        let changes = FrameChanges::observed(rows_changed, table_read, [false; TABLE_COUNT], None);
        let rows = |table: TableId| rows_changed[table.index()];
        let mut held = 0;
        self.countries.update(&self.frame, Some(&changes));
        let countries = (rows(TableId::Countries) || rows(TableId::CountryLinks)).then(|| {
            let views = [
                self.frame.tables[TableId::Countries.index()].clone(),
                self.frame.tables[TableId::CountryLinks.index()].clone(),
            ];
            held += 2 * size_of::<SharedView>();
            // Each newly handed-on allocation stays alive with the queued hour: charge its rows.
            for (index, view) in views.iter().enumerate() {
                let handed = self.handed_countries.as_ref().map(|last| &last[index]);
                if !handed.is_some_and(|last| SharedView::ptr_eq(last, view)) {
                    held += view_bytes(view);
                }
            }
            self.handed_countries = Some(views.clone());
            views
        });
        let flags = Flags::of(&self.frame);
        let content = self.content;
        let frame = &self.frame;
        let inputs = &self.countries;
        let complete = inputs.complete().is_some();
        // Ciphers.
        let cipher = (rows(TableId::Cryptology)
            || rows(TableId::RaidIntelInputs)
            || inputs.changed)
            .then(|| {
                let read = self.cipher.read(
                    frame,
                    content.defines.as_ref().ok().copied(),
                    inputs,
                    Some(&changes),
                );
                cipher_delta(&mut self.sent.cipher, read).map(Box::new)
            })
            .flatten();
        if let Some(cipher) = &cipher {
            held += size_of::<CipherDelta>();
            held += match &**cipher {
                CipherDelta::Whole(CipherRead::Rows { pairs, .. }) => {
                    pairs.capacity() * size_of::<PairRead>()
                        + pairs.iter().map(PairRead::heap).sum::<usize>()
                }
                CipherDelta::Whole(_) => 0,
                CipherDelta::Pairs { set, gone, .. } => {
                    set.capacity() * size_of::<PairRead>()
                        + set.iter().map(PairRead::heap).sum::<usize>()
                        + gone.capacity() * size_of::<(u32, u32)>()
                }
            };
        }
        // Placements.
        self.placements_dirty |= rows(TableId::OperativeMemberships) || inputs.changed;
        let census = operative_placements::Census {
            read: flags.membership_read,
            feature: flags.feature,
            complete: flags.memberships,
        };
        let placements = (self.placements_dirty && census.resolves(inputs))
            .then(|| {
                self.placements_dirty = false;
                let captured = frame
                    .complete::<tables::OperativeMemberships>()
                    .expect("a complete census");
                let localisation = content
                    .catalogue
                    .as_ref()
                    .ok()
                    .and_then(Option::as_deref)
                    .map(|catalogue| catalogue.localisation.as_ref());
                let read = operative_placements::read(captured, inputs, localisation);
                let delta = read.delta(self.sent.placements.as_ref());
                self.sent.placements = Some(read);
                delta.map(Box::new)
            })
            .flatten();
        if let Some(placements) = &placements {
            held += size_of::<PlacementDelta>() + placements.heap();
        }
        // Agencies.
        let agencies = if complete
            && (rows(TableId::IntelligenceAgencies)
                || rows(TableId::AgencyUpgradeLevels)
                || rows(TableId::LexicalTokens)
                || rows(TableId::OperativeMemberships)
                || flags.membership_read
                || inputs.changed)
        {
            self.agencies.read(frame, inputs, Some(&changes))?;
            agency_delta(
                &mut self.sent.agencies,
                self.agencies.current(),
                self.agencies.rebuilt(),
                flags.membership_read,
            )
        } else {
            None
        };
        if let Some(agencies) = &agencies {
            held += size_of::<AgencyDelta>()
                + agencies
                    .slots
                    .as_ref()
                    .map_or(0, |slots| slots.bytes(agency_comparison::agency_heap))
                + agencies
                    .roster
                    .as_ref()
                    .and_then(Option::as_ref)
                    .map_or(0, |roster| {
                        roster.counts.bytes(|_| 0)
                            + roster.unsupported.len() * size_of::<(u8, usize)>()
                    })
                + match &agencies.acquired {
                    Some(AcquiredDelta::Whole(Ok((slots, _)))) => slots
                        .values()
                        .map(|upgrades| {
                            size_of::<(u32, ScannedUpgrades)>()
                                + agency_comparison::upgrades_heap(upgrades)
                        })
                        .sum(),
                    Some(AcquiredDelta::Slots {
                        slots: Some(slots), ..
                    }) => slots.bytes(agency_comparison::upgrades_heap),
                    _ => 0,
                };
        }
        // The roster's census, operations and captors' names.
        let operative = content.operative.as_ref().ok().and_then(Option::as_deref);
        if rows(TableId::CharacterOperatives)
            || rebuilt & (1 << position(TableId::CharacterOperatives)) != 0
        {
            self.roster.forget_characters();
        }
        self.members_dirty |= MEMBERSHIP_INPUTS.iter().any(|&table| rows(table))
            || roster_operations
            || inputs.changed;
        let members = (self.members_dirty
            && flags.feature == Some(true)
            && flags.memberships
            && flags.membership_read
            && complete)
            .then(|| {
                self.members_dirty = false;
                let members = self.roster.members(frame, operative, inputs, Some(&changes));
                members_delta(&mut self.sent.members, members).map(Box::new)
            })
            .flatten();
        if let Some(members) = &members {
            held += size_of::<MembersDelta>();
            held += match &**members {
                MembersDelta::Whole(members) => groups(members.by_slot.values()),
                MembersDelta::Slots { slots, .. } => {
                    slots.capacity() * size_of::<(u32, SlotPatch)>()
                        + slots
                            .iter()
                            .map(|(_, patch)| match patch {
                                SlotPatch::Whole(members) => groups(std::iter::once(members)),
                                SlotPatch::Experience(moved) => {
                                    moved.capacity() * size_of::<(u32, Experience)>()
                                        + moved
                                            .iter()
                                            .map(|(_, experience)| experience.heap())
                                            .sum::<usize>()
                                }
                                SlotPatch::Gone => 0,
                            })
                            .sum::<usize>()
                }
            };
        }
        let operations = (rows(TableId::Operations) || rows(TableId::LexicalTokens))
            .then(|| whole(&mut self.sent.operations, operatives_roster::operations(frame)))
            .flatten();
        if let Some(Some(operations)) = &operations {
            held += operations
                .iter()
                .map(|(_, token)| size_of::<(u64, Option<String>)>() + delta::optional(token))
                .sum::<usize>();
        }
        let names = match operative.filter(|_| complete) {
            Some(operative) => {
                let mut asked = NAMING.iter().any(|&table| rows(table));
                if flags.membership_read && flags.memberships {
                    let captors = captors(frame, inputs);
                    asked |= self.captors.as_ref() != Some(&captors);
                    self.captors = Some(captors);
                }
                asked
                    .then(|| {
                        let named = match &self.captors {
                            Some(captors) => slot_names(
                                frame,
                                captors.iter().copied(),
                                &operative.catalogue.localisation,
                            ),
                            // Before the segment's first membership read, every captor held
                            // from before it can be in any slot.
                            None => {
                                let (_, links) = inputs.complete().expect("complete countries");
                                slot_names(
                                    frame,
                                    std::iter::once(0)
                                        .chain(links.iter().map(|link| link.canonical_slot)),
                                    &operative.catalogue.localisation,
                                )
                            }
                        };
                        let keyed = Keyed::of(self.sent.names.as_ref(), &named);
                        self.sent.names = Some(named);
                        keyed.map(Box::new)
                    })
                    .flatten()
            }
            None => None,
        };
        if let Some(names) = &names {
            held += size_of::<Keyed<u32, Result<String, String>>>();
            held += names.bytes(|name| match name {
                Ok(name) | Err(name) => name.capacity(),
            });
        }
        // The espionage log's operations census and completions.
        let census = (flags.operations && flags.operations_read)
            .then(|| {
                census_delta(
                    &mut self.sent.census,
                    espionage_log::census_read(frame, inputs),
                )
                .map(Box::new)
            })
            .flatten();
        if let Some(census) = &census {
            held += size_of::<CensusDelta>();
            held += match &**census {
                CensusDelta::Whole(census) => census.rows.as_ref().map_or(0, |rows| {
                    rows.capacity() * size_of::<OperationRead>()
                        + rows.iter().map(OperationRead::heap).sum::<usize>()
                }),
                CensusDelta::Rows { set, gone, .. } => {
                    set.capacity() * size_of::<OperationRead>()
                        + set.iter().map(OperationRead::heap).sum::<usize>()
                        + gone.capacity() * size_of::<u64>()
                }
            };
        }
        let completions = (rows(TableId::OperationCompletions) || rows(TableId::LexicalTokens))
            .then(|| whole(&mut self.sent.completions, espionage_log::completions(frame)))
            .flatten();
        if let Some(Some(completions)) = &completions {
            held += completions.capacity() * size_of::<Completion>()
                + completions.iter().map(espionage_log::completion_heap).sum::<usize>();
        }
        out.push(Hour {
            branch: hour.branch,
            hour: at,
            opens,
            changed,
            rebuilt,
            reads,
            roster_operations,
            flags,
            countries,
            cipher,
            placements,
            agencies,
            members,
            operations,
            names,
            census,
            completions,
            held,
        })
    }

    fn carried(&self) -> Option<usize> {
        Some(self.held_bytes())
    }

    fn save(&self, out: &mut Vec<u8>) -> io::Result<bool> {
        out.extend_from_slice(SAVED);
        Ok(true)
    }

    fn load(&mut self, bytes: &[u8]) -> io::Result<()> {
        if bytes != SAVED {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Not a saved intelligence projector",
            ));
        }
        self.restart();
        Ok(())
    }
}

/// What the census's slot groups hold.
fn groups<'a>(by_slot: impl Iterator<Item = &'a Vec<Member>>) -> usize {
    by_slot
        .map(|members| {
            members.capacity() * size_of::<Member>() + members.iter().map(Member::heap).sum::<usize>()
        })
        .sum()
}

/// `table`'s position in [`TABLES`].
fn position(table: TableId) -> usize {
    TABLES
        .iter()
        .position(|&read| read == table)
        .expect("an intelligence table")
}
