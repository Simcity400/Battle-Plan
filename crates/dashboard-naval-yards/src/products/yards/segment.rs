//! The yard views read a segment at a time ([`dashboard_keyed::segment`]). A segment's reader
//! keeps the view's projection ([`super::project`]) and hands on each hour's sources and the rows
//! it changed, naming operands and refusals by spelling.
pub(crate) use super::project::Rows;
use super::{project::Projector, *};
use dashboard_base::prepared::{frame_changes::FrameChanges, prepass::HeldBytes};
use dashboard_keyed::segment::{Reader, Segmented, Touched};
use replay_data::cadence::{Cadence, cadence_in};
use std::marker::PhantomData;

/// A yard view: repairs or refits.
pub trait View: Segmented<Read = Read> {
    const REFITS: bool;
    const SOURCES: &'static [TableId];
}

/// The repair view: ports and their queues.
pub struct Repairs;
/// The refit view: lines and their ships.
pub struct Refits;

impl View for Repairs {
    const REFITS: bool = false;
    const SOURCES: &'static [TableId] = crate::products::naval_repairs::SOURCES;
}

impl View for Refits {
    const REFITS: bool = true;
    const SOURCES: &'static [TableId] = crate::products::naval_refits::SOURCES;
}

macro_rules! segmented {
    ($($view:ty => $read:expr),* $(,)?) => {$(
        impl Segmented for $view {
            const TABLES: &'static [TableId] = management::TABLES;
            const READ: TableId = $read;
            type Read = Read;

            fn fields() -> io::Result<Vec<TableFields>> {
                super::fields(management::TABLES)
            }

            fn changed(changes: &FrameChanges) -> bool {
                changed(changes)
            }

            fn reader(format: FormatVersion) -> Box<dyn Reader<Self>> {
                Box::new(Yards::<$view> {
                    format,
                    projector: None,
                    view: PhantomData,
                })
            }
        }
    )*};
}

segmented!(
    Repairs => TableId::NavalRepairQueue,
    Refits => TableId::NavalRefitProgress,
);

/// Whether an hour reaches a yard view: a watched row changed, or a table read off the hourly
/// cadence was read, which dates what the view holds.
fn changed(changes: &FrameChanges) -> bool {
    management::TABLES.iter().any(|&table| {
        changes.rows_changed[table.index()]
            || (!matches!(cadence_in(table, FormatVersion::CURRENT), Cadence::Hourly)
                && changes.slot_read[table.index()])
    })
}

/// An hour as a segment's reader hands it on.
pub struct Read {
    /// Each source table as the hour holds it.
    pub(crate) sources: Sources,
    /// The rows it changed; none where its countries or their links are not complete.
    pub(crate) rows: Option<Rows>,
}

struct Yards<V> {
    format: FormatVersion,
    /// None before its first observation or after one refused.
    projector: Option<Box<Projector>>,
    view: PhantomData<fn() -> V>,
}

impl<V: View> Reader<V> for Yards<V> {
    fn read(
        &mut self,
        frame: &DecodedFrame,
        _: &FrameChanges,
        touched: &Touched,
    ) -> io::Result<(Read, usize)> {
        let sources = Sources::read(self.format, frame, V::SOURCES);
        let rows = match &mut self.projector {
            Some(projector) => projector.hour(frame, touched),
            None => Projector::build(frame, V::REFITS)?.map(|(projector, rows)| {
                self.projector = Some(projector);
                rows
            }),
        };
        let mut bytes = sources.heap();
        let rows = match (rows, &mut self.projector) {
            (Some(rows), Some(projector)) => {
                let (owners, first) = projector.owners();
                if first {
                    let mut held = HeldBytes::default();
                    held.heap(&owners);
                    bytes += usize::try_from(held.total()).unwrap_or(usize::MAX);
                }
                bytes += rows.rows.capacity() * size_of::<project::Row>()
                    + rows.rows.iter().map(project::Row::carry).sum::<usize>();
                Some(rows)
            }
            _ => {
                self.projector = None;
                None
            }
        };
        Ok((Read { sources, rows }, bytes))
    }
}
