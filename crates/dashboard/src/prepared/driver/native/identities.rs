//! The country identity history's native backend: each hour's operator hands its ordered emit
//! the identity tables the hour may have changed, copied from the walk's views in the fields the
//! history reads, and the emit
//! keeps the branch's frame of them and advances one [`HistoryScan`] an hour, a quiet step where
//! no country key, link, counter key or gain key moved. Its published page, the sealed history
//! a row preparation publishes, is also its continuation.

use super::backend::{Native, Projected};
use crate::{
    prepared::{driver::publication::Stage, frame_changes::FrameChanges},
    products::country_history::HistoryScan,
};
use dashboard_base::prepared::{gates, roots::Published};
use dashboard_world::{Hold, Needs, World};
use replay::{
    columnar_frames::Segment,
    columns,
    segments::{self, Operator, Sink},
};
use replay_data::{
    format::FormatVersion,
    rows::{TABLE_COUNT, TableId},
};
use replay_frame::frame::{DecodedFrame, Metrics, SharedView, TableView};
use std::{
    io::{self, BufReader},
    sync::atomic::AtomicBool,
};

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.to_owned())
}

/// The tables the history reads.
const TABLES: &[TableId] = &[
    TableId::Campaign,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CountryCounterInputs,
    TableId::CountryExperienceGains,
];

/// Of those, the tables a change to which makes the hour no quiet step.
const MOVING: [TableId; 4] = [
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CountryCounterInputs,
    TableId::CountryExperienceGains,
];

/// The tables whose reads witness the hour's native names.
const WITNESSES: [TableId; 3] = [
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CountryExperienceGains,
];

/// How the world holds each table the history reads, and the fields a change to which it
/// hands on: countries' slots and tags, every link, and the counters' and gains' keys alone.
fn holds() -> io::Result<[(TableId, Hold); 4]> {
    let field = |name| columns::field(TableId::Countries, name);
    let countries = 1_u64 << field("canonical_slot")? | 1_u64 << field("tag")?;
    Ok([
        (TableId::Countries, Hold::Fields(countries)),
        (TableId::CountryLinks, Hold::Whole),
        (TableId::CountryCounterInputs, Hold::Fields(0)),
        (TableId::CountryExperienceGains, Hold::Fields(0)),
    ])
}

/// One hour of a branch's identity tables.
#[derive(replay_spool::Spooled)]
pub(crate) struct Hour {
    hour: i32,
    /// The walk reads every table whole at this hour.
    opens: bool,
    metrics: Metrics,
    /// The tables the hour may have changed, in the fields the history reads.
    changes: Vec<(TableId, SharedView)>,
}

impl Projected for Hour {
    fn hour(&self) -> i32 {
        self.hour
    }
}

pub(crate) struct Projector {
    asks: [(TableId, Hold); 4],
}

impl Operator<World> for Projector {
    type Output = Hour;

    fn hour(
        &mut self,
        hour: &segments::Hour<'_, World>,
        out: &mut dyn Sink<Hour>,
    ) -> io::Result<()> {
        let world = hour.world;
        let opens = hour.starts || hour.opens;
        let mut changes = Vec::new();
        let campaign = (TableId::Campaign, Hold::Whole);
        for (table, hold) in std::iter::once(campaign).chain(self.asks) {
            let fields = match hold {
                Hold::Whole => u64::MAX,
                Hold::Fields(fields) => fields,
            };
            if opens || world.touched(table, fields) {
                engine_allocation::reserve(&mut changes, 1)?;
                changes.push((table, world.copy(hour.state, table, hold)?));
            }
        }
        out.push(Hour {
            hour: hour.hour,
            opens,
            metrics: world.metrics().clone(),
            changes,
        })
    }
}

/// A branch's identity history and its frame of the identity tables.
pub(crate) struct History {
    scan: HistoryScan,
    frame: Option<DecodedFrame>,
    format: FormatVersion,
}

impl History {
    fn observe(&mut self, branch: u64, hour: Hour) -> io::Result<()> {
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
            return Err(invalid("An identity history crossed branches"));
        }
        frame.hour = hour.hour;
        frame.metrics = hour.metrics;
        let mut changed = [hour.opens; TABLE_COUNT];
        for (table, view) in hour.changes {
            changed[table.index()] = true;
            frame.tables[table.index()] = view;
        }
        let quiet = MOVING.iter().all(|table| !changed[table.index()]);
        let witnessed = WITNESSES
            .iter()
            .all(|table| frame.metrics.table_ns[table.index()] != 0);
        if quiet && self.scan.advance_unchanged(hour.hour, witnessed) {
            return Ok(());
        }
        let mut facts = FrameChanges::alone(frame, self.format);
        facts.rows_changed = changed;
        self.scan
            .observe_owned_states(frame, Some(&facts), |_, _, _| {});
        Ok(())
    }
}

pub(crate) struct Identities {
    pub(crate) format: FormatVersion,
}

impl Native for Identities {
    type Operator = Projector;
    type Branch = Box<History>;

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
        Ok(Projector { asks: holds()? })
    }

    fn branch(&self, _: u64, _: i32) -> io::Result<Self::Branch> {
        Ok(Box::new(History {
            scan: HistoryScan::new(false, &[]),
            frame: None,
            format: self.format,
        }))
    }

    fn observe(
        &self,
        state: &mut Self::Branch,
        _: &mut Stage<'_, '_>,
        branch: u64,
        hour: Hour,
    ) -> io::Result<()> {
        state.observe(branch, hour)
    }

    fn flush(
        &self,
        state: &mut Self::Branch,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        _: Option<i32>,
        _: &AtomicBool,
    ) -> io::Result<()> {
        state.scan.seal();
        let page = serde_json::to_vec(&state.scan)?;
        stage
            .pages()?
            .write_at(&gates::identities_section(branch), 0, page.as_slice())
    }

    // Its published page goes on as it is.
    fn save(&self, _: &mut Self::Branch) -> io::Result<Option<Vec<u8>>> {
        Ok(Some(Vec::new()))
    }

    fn resume(
        &self,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        _: &Published,
        _: &[u8],
    ) -> io::Result<Self::Branch> {
        let page = stage
            .pages()?
            .read_at(&gates::identities_section(branch), 0)?
            .ok_or_else(|| invalid("Its published page is missing"))?;
        Ok(Box::new(History {
            scan: serde_json::from_reader(BufReader::new(page))?,
            frame: None,
            format: self.format,
        }))
    }
}
