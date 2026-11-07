//! Raids on a native recording, a segment at a time ([`dashboard_keyed::segment`]): at each hour
//! a field the scan reads changed, a segment's reader keeps the tables the scan reads, built from
//! the walk's views in the fields it reads. Each small table is copied only where the hour changed
//! a field it reads. Of the provinces and states, whose
//! owners and controllers move every hour at war, and of the lexical tokens, which grow with
//! anything the game names, only the rows the hour's raid targets reach are kept, which are all
//! the scan's target readers look up. The branch's emit ([`super::emit`]) gives those hours to its
//! one scan.

use dashboard_base::prepared::{frame_changes::FrameChanges, kept::kept_changes};
use dashboard_keyed::segment::{Reader, Segmented, Touched};
use replay::{columnar_frames::view_heap_bytes, campaign::TableFields};
use replay_data::{
    format::FormatVersion,
    rows::{ProvinceRow, TableId, TableSchema, column, tables},
};
use replay_frame::frame::{ColumnFrame, ColumnView, Metrics, SharedView, TableView};
use std::{collections::BTreeSet, io, mem::size_of};

/// The tables the raid scan reads, its target names' included: a target state owned by country
/// zero is named by the reserved country's original tag.
pub const TABLES: &[TableId] = &[
    TableId::Countries,
    TableId::CountryLinks,
    TableId::RaidCountries,
    TableId::Raids,
    TableId::RaidTargets,
    TableId::LexicalTokens,
    TableId::Provinces,
    TableId::States,
    TableId::CountryAppearance,
    TableId::ReservedCountryNaming,
];

/// Raids, a segment at a time.
pub struct Segments;

impl Segmented for Segments {
    const TABLES: &'static [TableId] = TABLES;
    const READ: TableId = TableId::Raids;
    type Read = Kept;

    fn fields() -> io::Result<Vec<TableFields>> {
        Ok(vec![
            // Country identity: the id (the row key), canonical slot and tag.
            TableFields::named(TableId::Countries, &["canonical_slot", "tag"])?,
            TableFields::all(TableId::CountryLinks),
            // Each country's slot (the row key), its country and eligible raid types.
            TableFields::named(TableId::RaidCountries, &["country", "eligible_types"])?,
            // Each raid: its parent, phase, outcome, flight distance and preparation hours.
            TableFields::named(
                TableId::Raids,
                &[
                    "parent_slot",
                    "phase",
                    "outcome",
                    "travelled",
                    "preparation_hours",
                ],
            )?,
            // Each target's scope and type.
            TableFields::all(TableId::RaidTargets),
            // Raid type and building tokens.
            TableFields::all(TableId::LexicalTokens),
            // A target province's state and controller.
            TableFields::named(TableId::Provinces, &["state", "controller"])?,
            // A target state's owner and controller, and its stored names.
            TableFields::named(
                TableId::States,
                &["owner", "controller", "custom_name", "owner_name"],
            )?,
            // A target state owner's original tag.
            TableFields::named(
                TableId::CountryAppearance,
                &["original_tag", "original_tag_reference"],
            )?,
            TableFields::all(TableId::ReservedCountryNaming),
        ])
    }

    fn reader(_: FormatVersion) -> Box<dyn Reader<Self>> {
        Box::new(Keep {
            copies: Vec::new(),
            reduced: [None, None, None],
            fields: None,
        })
    }
}

/// An hour the scan reads, as a segment's reader kept it: each table's view in [`TABLES`] order,
/// the hour's metrics and its change facts, the provinces and states flagged changed only where
/// the rows kept of them did.
#[derive(replay_spool::Spooled)]
pub struct Kept {
    pub(super) views: Box<[SharedView]>,
    pub(super) metrics: Box<Metrics>,
    pub(super) changes: Box<FrameChanges>,
}

/// A segment's reader: each small table's copy, and the provinces, states and tokens it kept last.
struct Keep {
    copies: Vec<SharedView>,
    reduced: [Option<SharedView>; 3],
    /// The fields the scan reads of each table, in [`TABLES`] order, once read.
    fields: Option<Vec<u64>>,
}

/// The fields the scan reads of each table, in [`TABLES`] order: every field of one it names
/// none of.
fn fields() -> io::Result<Vec<u64>> {
    let named = Segments::fields()?;
    Ok(TABLES
        .iter()
        .map(|&table| {
            named
                .iter()
                .find(|fields| fields.table == table)
                .map_or(table.full_mask(), |fields| fields.fields)
        })
        .collect())
}

/// The provinces, states and tokens the targets of `frame` reach: each scope's province, leader
/// province and building location, every state they name, their provinces' states, and each
/// target's raid and building type.
fn reached(frame: &ColumnFrame<'_>) -> io::Result<[BTreeSet<u32>; 3]> {
    const STATE: usize = column::<ProvinceRow>("state");
    let mut provinces = BTreeSet::new();
    let mut states = BTreeSet::new();
    let mut tokens = BTreeSet::new();
    if let Ok(targets) = frame.complete(TableId::RaidTargets) {
        for &slot in targets.slots() {
            let target: <tables::RaidTargets as TableSchema>::Row =
                targets.row(slot, TableId::RaidTargets.full_mask())?;
            let scope = &target.scope;
            for token in [target.raid_type, scope.building_type] {
                if token != 0 {
                    tokens.insert(token);
                }
            }
            for id in [scope.province, scope.leader_province, scope.building_location] {
                if id != 0 {
                    provinces.insert(id);
                }
            }
            for id in [scope.state, scope.building_location] {
                if id != 0 {
                    states.insert(id);
                }
            }
        }
    }
    if let Ok(rows) = frame.complete(TableId::Provinces) {
        for &id in &provinces {
            if let Some(slot) = rows.find(u128::from(id)) {
                let state = rows.value::<u32>(slot, STATE)?;
                if state != 0 {
                    states.insert(state);
                }
            }
        }
    }
    Ok([provinces, states, tokens])
}

/// The rows of `view` whose keys `keep` holds, in the fields `mask` names, when it is complete;
/// it whole otherwise.
fn reduce(view: ColumnView<'_>, keep: &BTreeSet<u32>, mask: u64) -> io::Result<TableView> {
    match view {
        ColumnView::Complete(rows) => {
            let mut slots = Vec::with_capacity(keep.len());
            for &id in keep {
                if let Some(slot) = rows.find(u128::from(id)) {
                    slots.push(slot);
                }
            }
            Ok(TableView::Complete(rows.rows_at(&slots, mask)?))
        }
        view => view.to_table_view(mask),
    }
}

impl Reader<Segments> for Keep {
    fn read(
        &mut self,
        frame: &ColumnFrame<'_>,
        changes: &FrameChanges,
        touched: &Touched,
    ) -> io::Result<(Kept, usize)> {
        let fields = match &mut self.fields {
            Some(fields) => fields,
            unread => unread.insert(fields()?),
        };
        let mut changes = kept_changes(changes);
        let mut bytes = size_of::<Metrics>()
            + size_of::<FrameChanges>()
            + TABLES.len() * size_of::<SharedView>();
        let [provinces, states, tokens] = reached(frame)?;
        let mut views = Vec::with_capacity(TABLES.len());
        let mut small = 0;
        for (&table, &mask) in TABLES.iter().zip(fields.iter()) {
            let lent = frame.view(table);
            let reduced = match table {
                TableId::Provinces => Some((0, reduce(lent, &provinces, mask)?)),
                TableId::States => Some((1, reduce(lent, &states, mask)?)),
                TableId::LexicalTokens => Some((2, reduce(lent, &tokens, mask)?)),
                _ => None,
            };
            let view = match reduced {
                Some((at, view)) => {
                    // Only the rows kept decide whether the hour changed what the scan reads.
                    let changed = !self.reduced[at].as_ref().is_some_and(|held| **held == view);
                    if changed {
                        let held = SharedView::from(view);
                        bytes += view_heap_bytes(&held);
                        self.reduced[at] = Some(held);
                    }
                    changes.rows_changed[table.index()] = changed;
                    self.reduced[at].clone().expect("a kept reduced table")
                }
                None => {
                    // An hour copies a table only where it changed a field read here.
                    let copy = match self.copies.get(small) {
                        Some(copy) if !touched.of(table).any() => copy.clone(),
                        _ => {
                            let copy = SharedView::from(lent.to_table_view(mask)?);
                            bytes += view_heap_bytes(&copy);
                            match self.copies.get_mut(small) {
                                Some(slot) => *slot = copy.clone(),
                                None => self.copies.push(copy.clone()),
                            }
                            copy
                        }
                    };
                    small += 1;
                    copy
                }
            };
            views.push(view.with_sampled_at(frame.sampled_at(table)));
        }
        Ok((
            Kept {
                views: views.into_boxed_slice(),
                metrics: Box::new(frame.metrics.clone()),
                changes: Box::new(changes),
            },
            bytes,
        ))
    }
}
