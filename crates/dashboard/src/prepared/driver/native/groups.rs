//! The country groups' native backend: each hour's operator hands its ordered emit the group
//! tables the hour may have changed, as the walk's world holds them, and whether it read the
//! dictionary; the emit keeps the branch's frame of them and observes it with the groups' scan
//! and change publisher, as the hourly walk does, a quiet step where nothing they read moved.
//! It publishes the pages the walk publishes: the scan, the changes and their catalogue.

use super::backend::{Native, Projected};
use crate::{
    Source,
    config::LoadContext,
    prepared::{driver::publication::Stage, frame_changes::FrameChanges},
    products::country_groups::{Scan, changes::Prepared as Changes, prepared::TABLES},
};
use dashboard_base::prepared::{
    roots::Published,
    series::{compress, decompress},
};
use dashboard_text::metadata::Extents;
use dashboard_world::{Hold, Needs, World};
use replay::{
    columnar_frames::{Segment, view_heap_bytes},
    columns,
    segments::{self, Item, Operator, Sink},
};
use replay_data::rows::{TABLE_COUNT, TableId};
use replay_frame::frame::{DecodedFrame, Metrics, SharedView, TableView};
use std::{
    io::{self, BufReader},
    mem::size_of,
    sync::atomic::AtomicBool,
};

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.to_owned())
}

/// How the world holds each table the groups read, and the fields a change to which they hand
/// on: countries' slots and tags, wars' dates and instigator, every field of the others, since
/// any change of those is an hour the walk observes.
fn holds() -> io::Result<[(TableId, Hold); 8]> {
    let fields = |table, names: &[&str]| -> io::Result<Hold> {
        let mut mask = 0_u64;
        for name in names {
            mask |= 1_u64 << columns::field(table, name)?;
        }
        Ok(Hold::Fields(mask))
    };
    Ok([
        (
            TableId::Countries,
            fields(TableId::Countries, &["canonical_slot", "tag"])?,
        ),
        (TableId::CountryLinks, Hold::Whole),
        (TableId::HumanControllers, Hold::Whole),
        (TableId::Factions, Hold::Whole),
        (TableId::FactionMembers, Hold::Whole),
        (
            TableId::WarRelations,
            fields(
                TableId::WarRelations,
                &["start_hour", "end_hour", "first_was_instigator"],
            )?,
        ),
        (TableId::Capitulation, Hold::Whole),
        (TableId::LexicalTokens, Hold::Whole),
    ])
}

/// One hour of a branch's group tables.
pub(crate) struct Hour {
    hour: i32,
    /// The hour is its segment's first: its tables are all handed on, and its scan begins a
    /// block there.
    starts: bool,
    /// The hour read the dictionary.
    lexical: bool,
    /// The tables the hour may have changed, as the walk's world holds them.
    changes: Vec<(TableId, SharedView)>,
}

impl Item for Hour {
    fn heap(&self) -> usize {
        // Each view is its own once the world's next build copies it.
        let views: usize = self
            .changes
            .iter()
            .map(|(_, view)| view_heap_bytes(view))
            .sum();
        self.changes.capacity() * size_of::<(TableId, SharedView)>() + views
    }
}

impl Projected for Hour {
    fn hour(&self) -> i32 {
        self.hour
    }
}

pub(crate) struct Projector {
    asks: [(TableId, Hold); 8],
    /// The asks of the hour's moved tables, kept from hour to hour.
    moved: Vec<(TableId, Hold)>,
}

impl Operator<World> for Projector {
    type Output = Hour;

    fn hour(
        &mut self,
        hour: &segments::Hour<'_, World>,
        out: &mut dyn Sink<Hour>,
    ) -> io::Result<()> {
        let _cpu = replay_frame::cpu::scope("groups.columnar", Some("groups"));
        let world = hour.world;
        let opens = hour.starts || hour.opens;
        self.moved.clear();
        self.moved.extend(self.asks.iter().copied().filter(|&(table, hold)| {
            let fields = match hold {
                Hold::Whole => u64::MAX,
                Hold::Fields(fields) => fields,
            };
            // The row decoder reads a table it cannot read whole anew every hour.
            opens
                || world.touched(table, fields)
                || columns::status(hour.state, table) != columns::Status::Complete
        }));
        let mut changes = Vec::new();
        if !self.moved.is_empty() {
            let frame = world.frame_held(hour.state, &self.moved)?;
            engine_allocation::reserve(&mut changes, self.moved.len())?;
            for &(table, _) in &self.moved {
                changes.push((table, frame.tables[table.index()].clone()));
            }
        }
        out.push(Hour {
            hour: hour.hour,
            starts: hour.starts,
            lexical: world.metrics().table_ns[TableId::LexicalTokens.index()] != 0,
            changes,
        })
    }
}

/// A branch's groups, their change publisher, its first and last hours and its frame of the
/// group tables.
pub(crate) struct Branch {
    scan: Box<Scan>,
    changes: Box<Changes>,
    bounds: (i32, i32),
    frame: Option<DecodedFrame>,
}

impl Branch {
    fn new(scan: Scan, changes: Box<Changes>, bounds: (i32, i32)) -> Box<Self> {
        Box::new(Self {
            scan: Box::new(scan),
            changes,
            bounds,
            frame: None,
        })
    }

    fn observe(
        &mut self,
        source: &Source,
        context: &LoadContext,
        branch: u64,
        hour: Hour,
    ) -> io::Result<()> {
        let frame = self.frame.get_or_insert_with(|| DecodedFrame {
            branch,
            hour: hour.hour,
            metrics: Metrics::default(),
            tables: std::array::from_fn(|_| {
                TableView::Unavailable {
                    reason: String::new(),
                }
                .into()
            }),
        });
        if frame.branch != branch {
            return Err(invalid("Country groups crossed branches"));
        }
        if hour.starts {
            self.scan.begin_block();
        }
        frame.hour = hour.hour;
        let lexical = TableId::LexicalTokens.index();
        frame.metrics.table_ns[lexical] = u64::from(hour.lexical);
        let mut facts = FrameChanges {
            rows_changed: [false; TABLE_COUNT],
            table_read: [false; TABLE_COUNT],
            slot_read: [false; TABLE_COUNT],
            updates: None,
        };
        facts.table_read[lexical] = hour.lexical;
        for (table, view) in hour.changes {
            facts.rows_changed[table.index()] = true;
            frame.tables[table.index()] = view;
        }
        self.changes
            .observe_changed(&mut self.scan, frame, &facts, source, context)
    }

    /// Publishes the changes kept since the last publication, and the scan's page.
    fn publish(
        &mut self,
        writer: &mut store::Writer,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.scan.finish_within(Some(self.bounds));
        self.changes.flush(writer, branch, &self.scan, cancelled)?;
        writer.write_at(&scan_section(branch), 0, self.scan.saved_bytes()?.as_slice())
    }

    /// What it goes on with after a publication: the change publisher's state.
    fn saved(&self) -> io::Result<Vec<u8>> {
        compress(&serde_json::to_vec(&*self.changes)?, 3)
    }

    /// The branch going on from its published scan page and [`Self::saved`].
    fn resumed(page: impl io::Read, saved: &[u8], bounds: (i32, i32)) -> io::Result<Box<Self>> {
        let scan = Scan::read(page)?;
        let changes = serde_json::from_slice(&decompress(saved)?)?;
        Ok(Self::new(scan, changes, bounds))
    }
}

/// The scan's published page of one branch.
fn scan_section(branch: u64) -> String {
    format!("country-groups-{branch}")
}

pub(crate) struct Groups<'p> {
    pub(crate) source: &'p Source,
    pub(crate) context: &'p LoadContext,
    pub(crate) extents: &'p Extents,
}

impl Groups<'_> {
    fn bounds(&self, branch: u64) -> io::Result<(i32, i32)> {
        self.extents
            .branch(branch)
            .map(|extent| extent.bounds)
            .ok_or_else(|| invalid("The recording holds no hour of the branch"))
    }
}

impl Native for Groups<'_> {
    type Operator = Projector;
    type Branch = Box<Branch>;

    fn tables(&self) -> &'static [TableId] {
        TABLES
    }

    fn needs(&self) -> io::Result<Needs> {
        Ok(Needs {
            frame: holds()?.to_vec(),
            ..Needs::default()
        })
    }

    fn operator(&self, _: &Segment) -> io::Result<Projector> {
        Ok(Projector {
            asks: holds()?,
            moved: Vec::new(),
        })
    }

    fn branch(&self, branch: u64, _: i32) -> io::Result<Self::Branch> {
        let bounds = self.bounds(branch)?;
        Ok(Branch::new(
            Scan::within(Some(bounds)),
            Box::default(),
            bounds,
        ))
    }

    fn observe(
        &self,
        state: &mut Self::Branch,
        _: &mut Stage<'_, '_>,
        branch: u64,
        hour: Hour,
    ) -> io::Result<()> {
        state.observe(self.source, self.context, branch, hour)
    }

    fn flush(
        &self,
        state: &mut Self::Branch,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        _: Option<i32>,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        state.publish(stage.pages()?, branch, cancelled)
    }

    // The scan goes on from its published page.
    fn save(&self, state: &mut Self::Branch) -> io::Result<Option<Vec<u8>>> {
        state.saved().map(Some)
    }

    fn resume(
        &self,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        _: &Published,
        saved: &[u8],
    ) -> io::Result<Self::Branch> {
        let page = stage
            .pages()?
            .read_at(&scan_section(branch), 0)?
            .ok_or_else(|| invalid("Its published page is missing"))?;
        Branch::resumed(BufReader::new(page), saved, self.bounds(branch)?)
    }
}

#[cfg(test)]
mod tests;
