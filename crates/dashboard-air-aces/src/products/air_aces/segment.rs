//! Air aces on a native recording, a segment at a time ([`dashboard_keyed::segment`]): at each
//! hour a field the roster or the variant catalogue reads changed, or either was read, a
//! segment's reader keeps the tables they read. Each table is copied only where the hour changed
//! it, so a table's view stays the same view while its rows do (the roster compares its inputs
//! by view). The branch's emit ([`super::emit`]) gives those hours to its one observer.

use dashboard_base::prepared::{frame_changes::FrameChanges, kept::kept_changes};
use dashboard_keyed::segment::{Reader, Segmented, Touched};
use replay::{columnar_frames::view_heap_bytes, container::TableFields};
use replay_data::{format::FormatVersion, rows::TableId};
use replay_frame::frame::{DecodedFrame, Metrics, SharedView};
use std::{io, mem::size_of};

/// The tables the roster and the variant catalogue read.
pub const TABLES: &[TableId] = &[
    TableId::AirAces,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::AirWings,
    TableId::WingEquipment,
    TableId::Raids,
    TableId::RaidTooltipOperands,
    TableId::CountryAppearance,
    TableId::LexicalTokens,
    TableId::VariantDefinitions,
    TableId::VariantModules,
    TableId::VariantUpgrades,
];

/// Air aces, a segment at a time.
pub struct Segments;

impl Segmented for Segments {
    const TABLES: &'static [TableId] = TABLES;
    const READ: TableId = TableId::AirAces;
    type Read = Kept;

    fn fields() -> io::Result<Vec<TableFields>> {
        Ok(vec![
            // The roster: every field of every row (a run holds while its row is equal), and
            // every read, which dates the roster (daily).
            TableFields::all(TableId::AirAces).when_read(),
            // Country identity: the id (the row key), canonical slot and tag.
            TableFields::named(TableId::Countries, &["canonical_slot", "tag"])?,
            TableFields::all(TableId::CountryLinks),
            // A wing's id (the row key) and stored name.
            TableFields::named(TableId::AirWings, &["name"])?,
            // A wing's equipment variants by ordinal (the row key).
            TableFields::named(TableId::WingEquipment, &["variant"])?,
            // Legacy wing names come from the active raids' wing, phase and unit (keyed by id
            // and reference type) and their tooltip operands.
            TableFields::named(TableId::Raids, &["phase", "unit", "air_wing"])?,
            TableFields::all(TableId::RaidTooltipOperands),
            // Variant owners' original tags.
            TableFields::named(TableId::CountryAppearance, &["name_tag"])?,
            TableFields::all(TableId::LexicalTokens),
            // The variant catalogue: every field, and every read (daily), which dates the
            // variants' names.
            TableFields::all(TableId::VariantDefinitions).when_read(),
            TableFields::all(TableId::VariantModules),
            TableFields::all(TableId::VariantUpgrades),
        ])
    }

    /// A watched row changed, or the catalogue's slot was read, untimed or not: it names anew.
    fn changed(changes: &FrameChanges) -> bool {
        changes.slot_read[TableId::VariantDefinitions.index()]
            || TABLES
                .iter()
                .any(|table| changes.rows_changed[table.index()])
    }

    fn reader(_: FormatVersion) -> Box<dyn Reader<Self>> {
        Box::new(Keep {
            copies: vec![None; TABLES.len()],
        })
    }
}

/// An hour the roster or catalogue reads, as a segment's reader kept it: each table's view in
/// [`TABLES`] order, the hour's metrics and its change facts.
pub struct Kept {
    pub(super) views: Box<[SharedView]>,
    pub(super) metrics: Box<Metrics>,
    pub(super) changes: Box<FrameChanges>,
}

/// A segment's reader: its copy of each table, in [`TABLES`] order, as the hour that last
/// changed it held it.
struct Keep {
    copies: Vec<Option<SharedView>>,
}

impl Reader<Segments> for Keep {
    fn read(
        &mut self,
        frame: &DecodedFrame,
        changes: &FrameChanges,
        touched: &Touched,
    ) -> io::Result<(Kept, usize)> {
        let mut bytes = size_of::<Metrics>()
            + size_of::<FrameChanges>()
            + TABLES.len() * size_of::<SharedView>();
        let mut views = Vec::with_capacity(TABLES.len());
        for (held, &table) in self.copies.iter_mut().zip(TABLES) {
            let lent = &frame.tables[table.index()];
            // The world's views are never kept: an hour that changed one keeps a copy.
            let copy = match held {
                Some(copy) if !touched.of(table).any() => copy.clone(),
                _ => {
                    let copy = SharedView::from((**lent).clone());
                    bytes += view_heap_bytes(&copy);
                    *held = Some(copy.clone());
                    copy
                }
            };
            views.push(copy.with_sampled_at(lent.sampled_at()));
        }
        Ok((
            Kept {
                views: views.into_boxed_slice(),
                metrics: Box::new(frame.metrics.clone()),
                changes: Box::new(kept_changes(changes)),
            },
            bytes,
        ))
    }
}
