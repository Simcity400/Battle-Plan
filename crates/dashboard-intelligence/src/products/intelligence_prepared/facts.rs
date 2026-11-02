//! The intelligence family's hours, projected from a columnar walk ([`replay::segments`]): the
//! tables its observers read ([`super::TABLES`]), each held by the fields they read, handed on
//! where the hour may have changed those fields, with the change facts the observers are told.
//! The observers themselves run in the ordered emit ([`super::emit`]), which keeps every table
//! as last handed on. Every segment starts cold: its first hour, and any hour after one it did
//! not walk, reads every table again and reports every table changed.

use super::TABLES;
use dashboard_world::{Hold, Needs, World};
use replay::{
    columns,
    segments::{self, Item, Operator, Sink},
};
use replay_data::rows::{
    AutonomyRow, CountryAppearanceRow, CountryRow, IntelligenceAgencyRow, PoliticalPartyRow,
    PoliticsRow, RaidIntelInputRow, TableId, TableSchema, tables,
};
use replay_frame::frame::{SharedView, TableView};
use std::{io, mem::size_of};

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

/// One walked hour of a branch.
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
    /// The views that differ from those last handed on, each reduced to its read fields.
    pub views: Vec<(TableId, SharedView)>,
    /// What the views first handed on here hold.
    held: usize,
}

impl Item for Hour {
    fn heap(&self) -> usize {
        self.views.capacity() * size_of::<(TableId, SharedView)>() + self.held
    }
}

/// The read bit of OperativeMemberships and of Operations in [`Hour::reads`].
pub const MEMBERSHIPS_READ: u8 = 1;
pub const OPERATIONS_READ: u8 = 2;

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

/// `view` of `table` as the observers read it: the walk's frame holds a table by some fields
/// with its others as last built, so those tables keep only the fields read, as default, and
/// equal reads share one view.
fn read_view(table: TableId, view: &SharedView, held: &mut Option<SharedView>) -> SharedView {
    match table {
        TableId::Countries => reduced::<tables::Countries>(view, held, |row| CountryRow {
            id: row.id,
            canonical_slot: row.canonical_slot,
            tag: row.tag.clone(),
            ..Default::default()
        }),
        TableId::RaidIntelInputs => {
            reduced::<tables::RaidIntelInputs>(view, held, |row| RaidIntelInputRow {
                country: row.country,
                crypto_strength: row.crypto_strength,
                ..Default::default()
            })
        }
        TableId::IntelligenceAgencies => {
            reduced::<tables::IntelligenceAgencies>(view, held, |row| IntelligenceAgencyRow {
                country: row.country,
                defense_raw: row.defense_raw,
                active_count: row.active_count,
                unknown_f0: row.unknown_f0,
                name: row.name.clone(),
                created: row.created,
                ..Default::default()
            })
        }
        TableId::CountryAppearance => {
            reduced::<tables::CountryAppearance>(view, held, |row| CountryAppearanceRow {
                country: row.country,
                cosmetic_tag: row.cosmetic_tag.clone(),
                name_tag: row.name_tag.clone(),
                leader_ideology: row.leader_ideology.clone(),
                overlord: row.overlord,
                ..Default::default()
            })
        }
        TableId::Politics => reduced::<tables::Politics>(view, held, |row| PoliticsRow {
            country: row.country,
            ruling_party: row.ruling_party,
            ..Default::default()
        }),
        TableId::PoliticalParties => {
            reduced::<tables::PoliticalParties>(view, held, |row| PoliticalPartyRow {
                country: row.country,
                id: row.id,
                token: row.token.clone(),
                ..Default::default()
            })
        }
        TableId::Autonomy => reduced::<tables::Autonomy>(view, held, |row| AutonomyRow {
            subject: row.subject,
            state: row.state.clone(),
            ..Default::default()
        }),
        _ => view.clone(),
    }
}

/// One segment's projection.
pub struct Projector {
    /// Each of [`TABLES`], by position: its hold and the fields read.
    holds: Vec<(TableId, Hold, u64)>,
    /// The fields of Operations the roster reads, a bit by column.
    roster_operations: u64,
    /// Nothing is held: the next hour hands on every table, as a segment's first.
    cold: bool,
    last: Option<i32>,
    /// By position: the view last handed on, and the last reduced view of a reduced table.
    sent: Vec<Option<SharedView>>,
    reduced: Vec<Option<SharedView>>,
    /// The tables the hour asks of the walk's frame, kept from hour to hour.
    asks: Vec<(TableId, Hold)>,
    asked: Vec<usize>,
}

/// What [`Projector::save`] writes: nothing but that it restarts, since every table is read
/// again from the walker's tables at the next hour, as at a segment's first.
const SAVED: &[u8] = b"intelligence projector: cold";

impl Projector {
    pub fn new() -> io::Result<Self> {
        let mut roster_operations = 0_u64;
        for name in ROSTER_OPERATIONS {
            roster_operations |= 1 << columns::field(TableId::Operations, name)?;
        }
        Ok(Self {
            holds: holds()?,
            roster_operations,
            cold: true,
            last: None,
            sent: vec![None; TABLES.len()],
            reduced: vec![None; TABLES.len()],
            asks: Vec::with_capacity(TABLES.len()),
            asked: Vec::with_capacity(TABLES.len()),
        })
    }

    fn restart(&mut self) {
        self.cold = true;
        self.last = None;
        self.sent.fill(None);
        self.reduced.fill(None);
    }
}

impl Operator<World> for Projector {
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
        let (mut changed, mut rebuilt) = (0_u32, 0_u32);
        self.asks.clear();
        self.asked.clear();
        for (position, &(table, hold, fields)) in self.holds.iter().enumerate() {
            if !opens && !world.touched(table, u64::MAX) {
                continue;
            }
            rebuilt |= 1 << position;
            if opens || world.touched(table, fields) {
                changed |= 1 << position;
                self.asks.push((table, hold));
                self.asked.push(position);
            } else if table == AGENCIES {
                changed |= 1 << position;
            }
        }
        let mut views = Vec::new();
        let mut held = 0;
        if !self.asks.is_empty() {
            let frame = world.frame_held(hour.state, &self.asks)?;
            views.reserve_exact(self.asked.len());
            for &position in &self.asked {
                let table = self.holds[position].0;
                let view = read_view(
                    table,
                    &frame.tables[table.index()],
                    &mut self.reduced[position],
                );
                if self.sent[position]
                    .as_ref()
                    .is_some_and(|sent| SharedView::ptr_eq(sent, &view))
                {
                    continue;
                }
                held += view_bytes(&view);
                self.sent[position] = Some(view.clone());
                views.push((table, view));
            }
        }
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
        let roster_operations =
            opens || world.touched(TableId::Operations, self.roster_operations);
        out.push(Hour {
            branch: hour.branch,
            hour: at,
            opens,
            changed,
            rebuilt,
            reads,
            roster_operations,
            views,
            held,
        })
    }

    fn carried(&self) -> Option<usize> {
        Some(
            self.sent
                .iter()
                .chain(&self.reduced)
                .flatten()
                .map(|view| view_bytes(view))
                .sum(),
        )
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
