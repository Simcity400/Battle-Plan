//! Convoy health's native backend: each hour's operator hands its ordered emit the rows of the
//! convoy tables whose read fields the hour changed, or a table whole where the hour opens or its
//! status moved, and the tables it wrote anything of; the emit patches the branch's frame of them
//! and observes the hour with the convoys' census as the walk did, which reads again only the
//! rows handed on. It publishes the pages the walk publishes.

use super::backend::{Native, Projected};
use crate::{
    prepared::driver::publication::Stage,
    products::convoy_health::{Edit, Prepared as Census},
};
use dashboard_base::prepared::roots::{Published, Saved};
use dashboard_keyed::table_changes::{self, Change, apply_frame};
use dashboard_world::{Needs, World};
use replay::{
    columnar_frames::Segment,
    columns,
    campaign::Replay,
    segments::{self, Operator, Sink},
};
use replay_data::{
    format::FormatVersion,
    rows::{TABLE_COUNT, TableId},
    scope::CaptureMode,
};
use replay_frame::frame::{DecodedFrame, Metrics};
use std::{
    io,
    sync::{Arc, atomic::AtomicBool},
};

/// The convoy tables with the fields the census reads of each, by name: every field for those
/// named with none. The country join reads the id (the row key), canonical slot and tag.
const READS: [(TableId, &[&str]); 7] = [
    (TableId::Countries, &["canonical_slot", "tag"]),
    (TableId::CountryLinks, &[]),
    (TableId::LexicalTokens, &[]),
    (TableId::ResourceDefinitions, &[]),
    (TableId::RegionalConvoyEfficiency, &["efficiency"]),
    (
        TableId::ResourceExchanges,
        &[
            "exporter",
            "importer",
            "resource",
            "convoys_assigned",
            "convoys_required",
            "efficiency_a",
            "efficiency_b",
        ],
    ),
    (
        TableId::ConvoyClients,
        &["category", "owner", "other", "bad", "convoys_assigned", "convoys_required"],
    ),
];

/// The tables the operator reads.
pub(crate) const TABLES: &[TableId] = &[
    TableId::Countries,
    TableId::CountryLinks,
    TableId::LexicalTokens,
    TableId::ResourceDefinitions,
    TableId::RegionalConvoyEfficiency,
    TableId::ResourceExchanges,
    TableId::ConvoyClients,
];

/// One projected hour: the convoy tables it changed, as patches or whole views.
#[derive(replay_spool::Spooled)]
pub(crate) struct Hour {
    hour: i32,
    /// The hour is its segment's first: every table is handed on whole.
    opens: bool,
    changes: Vec<(TableId, Arc<Change>)>,
    /// The tables the hour wrote any field of, or that stayed incomplete (the row decoder read
    /// those anew each hour): each counts as changed, as the walk's change facts had it.
    touched: Vec<TableId>,
}

impl Projected for Hour {
    fn hour(&self) -> i32 {
        self.hour
    }
}

/// One segment's operator.
pub(crate) struct Projector {
    format: FormatVersion,
    mode: CaptureMode,
    masks: [(TableId, u64); 7],
    /// Each table's status the hour before.
    shapes: [Option<columns::Status>; TABLE_COUNT],
}

impl Projector {
    fn new(format: FormatVersion, mode: CaptureMode) -> io::Result<Self> {
        let mut masks = [(TableId::Countries, u64::MAX); 7];
        for (held, &(table, names)) in masks.iter_mut().zip(&READS) {
            let mut mask = if names.is_empty() { u64::MAX } else { 0 };
            for name in names {
                mask |= 1_u64 << columns::field(table, name)?;
            }
            *held = (table, mask);
        }
        Ok(Self {
            format,
            mode,
            masks,
            shapes: [None; TABLE_COUNT],
        })
    }
}

impl Operator<World> for Projector {
    type Output = Hour;

    fn start(&mut self, _: segments::Start<'_>, _: u64, _: i32) -> io::Result<()> {
        self.shapes = [None; TABLE_COUNT];
        Ok(())
    }

    fn hour(
        &mut self,
        hour: &segments::Hour<'_, World>,
        out: &mut dyn Sink<Hour>,
    ) -> io::Result<()> {
        let _cpu = replay_frame::cpu::scope("convoys.columnar", Some("convoys"));
        let mut changes = Vec::new();
        let mut touched = Vec::new();
        for (table, mask) in self.masks {
            let index = table.index();
            let change = table_changes::read(
                hour,
                table,
                mask,
                self.shapes[index],
                (self.format, self.mode),
                &|rows| Ok(rows),
            )?;
            let status = columns::status(hour.state, table);
            self.shapes[index] = Some(status);
            let written = status != columns::Status::Complete || {
                let edits = columns::changes(hour.state, table)?;
                edits.truncated
                    || !edits.changed.is_empty()
                    || !edits.fill_slots.is_empty()
                    || !edits.inserted.is_empty()
                    || !edits.deleted.is_empty()
            };
            if written {
                touched.push(table);
            }
            if let Some(change) = change {
                changes.push((table, Arc::new(change)));
            }
        }
        out.push(Hour {
            hour: hour.hour,
            opens: hour.starts || hour.opens,
            changes,
            touched,
        })
    }

    fn save(&self, out: &mut Vec<u8>) -> io::Result<bool> {
        out.extend_from_slice(SAVED);
        Ok(true)
    }

    fn load(&mut self, bytes: &[u8]) -> io::Result<()> {
        if bytes != SAVED {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Not a saved convoys projector",
            ));
        }
        self.shapes = [None; TABLE_COUNT];
        Ok(())
    }
}

/// What [`Projector::save`] writes: nothing but that it restarts, every table handed on whole.
const SAVED: &[u8] = b"convoys projector: cold";

/// A branch's convoy census and its frame of the convoy tables.
pub(crate) struct Branch {
    census: Box<Census>,
    frame: Option<DecodedFrame>,
    /// The census reads no read evidence.
    metrics: Metrics,
}

impl Branch {
    fn new(census: Census) -> Self {
        Self {
            census: Box::new(census),
            frame: None,
            metrics: Metrics::default(),
        }
    }

    fn observe(&mut self, format: FormatVersion, branch: u64, hour: Hour) -> io::Result<()> {
        let mut facts = apply_frame(
            &mut self.frame,
            (branch, hour.hour),
            hour.opens,
            &self.metrics,
            &hour.changes,
            format,
        )?;
        for table in &hour.touched {
            facts.rows_changed[table.index()] = true;
        }
        let frame = self.frame.as_ref().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "The convoy census has no frame")
        })?;
        // A segment's first hour hands every table on whole: the census reads them whole.
        let edits: Option<Vec<(TableId, Edit)>> = (!hour.opens).then(|| {
            hour.changes
                .iter()
                .map(|(table, change)| {
                    let edit = match &**change {
                        Change::Replace(_) => Edit::Whole,
                        Change::Patch { rows, deleted, .. } => Edit::Rows {
                            written: rows.iter().map(|row| row.key()).collect(),
                            deleted: deleted.clone(),
                        },
                    };
                    (*table, edit)
                })
                .collect()
        });
        self.census.observe_edits(frame, &facts, edits.as_deref())
    }
}

pub(crate) struct Convoys<'p> {
    pub(crate) replay: &'p Replay,
}

impl Native for Convoys<'_> {
    type Operator = Projector;
    type Branch = Branch;

    fn tables(&self) -> &'static [TableId] {
        TABLES
    }

    fn needs(&self) -> io::Result<Needs> {
        Ok(Needs::default())
    }

    fn operator(&self, _: &Segment) -> io::Result<Projector> {
        Projector::new(self.replay.format, self.replay.capture_mode)
    }

    fn branch(&self, _: u64, _: i32) -> io::Result<Branch> {
        Ok(Branch::new(Census::default()))
    }

    fn observe(
        &self,
        state: &mut Branch,
        _: &mut Stage<'_, '_>,
        branch: u64,
        hour: Hour,
    ) -> io::Result<()> {
        state.observe(self.replay.format, branch, hour)
    }

    fn flush(
        &self,
        state: &mut Branch,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        _: Option<i32>,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        state.census.flush(stage.pages()?, branch, cancelled)
    }

    // The census goes on from what it saved; its frame is read whole again.
    fn save_changes(&self, state: &mut Branch, whole: bool) -> io::Result<Option<Saved>> {
        let pieces = state.census.save_changes(whole)?;
        Ok(Some(Saved { whole, pieces }))
    }

    fn resume_changes(
        &self,
        _: &mut Stage<'_, '_>,
        _: u64,
        _: &Published,
        pieces: &[Vec<u8>],
    ) -> io::Result<Branch> {
        Ok(Branch::new(Census::resumed(pieces)?))
    }
}
