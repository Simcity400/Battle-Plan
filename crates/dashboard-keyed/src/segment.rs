//! A keyed family read a segment at a time on a native recording. A segment's operator asks the
//! walk's world for the family's tables, held in the fields the family reads, at each hour where
//! one of those fields changed, and hands its reader that frame with the keys the hour changed;
//! every other hour is quiet. What a reader hands on names what only the branch numbers or dates
//! by spelling; the branch's emit takes the hours in order.

use dashboard_base::prepared::frame_changes::FrameChanges;
use dashboard_world::{Hold, Needs, World};
use replay::{
    columnar_frames::metrics,
    columns,
    container::TableFields,
    segments::{self, Item, Sink},
};
use replay_data::{
    format::FormatVersion,
    rows::{TABLE_COUNT, TableId},
};
use replay_frame::frame::{DecodedFrame, Metrics, not_selected};
use std::io;

#[cfg(test)]
mod tests;

/// A keyed family read a segment at a time.
pub trait Segmented: Sized + Send + Sync + 'static {
    /// The tables it reads, and the one whose reads date what it reads.
    const TABLES: &'static [TableId];
    const READ: TableId;
    /// What its reader hands its branch of one hour.
    type Read: Send;

    /// The fields of its tables it reads, whose changes make an hour read; a table it names none
    /// of, by all of them.
    fn fields() -> io::Result<Vec<TableFields>>;

    /// Whether an hour's change facts reach what it reads.
    fn changed(changes: &FrameChanges) -> bool {
        Self::TABLES
            .iter()
            .any(|table| changes.rows_changed[table.index()])
    }

    /// A reader of one segment's hours, from its first.
    fn reader(format: FormatVersion) -> Box<dyn Reader<Self>>;
}

/// One segment's reader of a keyed family, given its hours in order.
pub trait Reader<S: Segmented>: Send {
    /// An hour at which a field `S` reads changed, or whose change facts `S` reads
    /// ([`Segmented::changed`]): `frame`, `S`'s tables at the hour as the world holds them,
    /// those facts, and what the hour changed of each table. What it reads, and the bytes that
    /// holds.
    fn read(
        &mut self,
        frame: &DecodedFrame,
        changes: &FrameChanges,
        touched: &Touched,
    ) -> io::Result<(S::Read, usize)>;
}

/// What an hour changed of one table a family reads.
#[derive(Debug, Default)]
pub struct Touch {
    /// Its status moved, or the hour is the segment's first: read it whole.
    pub restated: bool,
    /// The rows it wrote in a field the family reads, or inserted, by key: the fields written,
    /// every field where inserted.
    pub written: Vec<(u128, u64)>,
    /// The keys it deleted, ascending.
    pub deleted: Vec<u128>,
}

impl Touch {
    /// Whether the hour changed anything of it.
    pub fn any(&self) -> bool {
        self.restated || !self.written.is_empty() || !self.deleted.is_empty()
    }

    fn clear(&mut self) {
        self.restated = false;
        self.written.clear();
        self.deleted.clear();
    }
}

/// What an hour changed of each table a family reads.
#[derive(Debug)]
pub struct Touched {
    touches: Vec<Touch>,
}

impl Default for Touched {
    fn default() -> Self {
        Self {
            touches: (0..TABLE_COUNT).map(|_| Touch::default()).collect(),
        }
    }
}

impl Touched {
    pub fn of(&self, table: TableId) -> &Touch {
        &self.touches[table.index()]
    }

    pub fn of_mut(&mut self, table: TableId) -> &mut Touch {
        &mut self.touches[table.index()]
    }
}

/// An hour as a segment's operator read it: whether the family's dating table was read there,
/// and what its reader read, or nothing where no input changed.
pub struct Hour<S: Segmented> {
    pub hour: i32,
    pub dated: bool,
    pub read: Option<S::Read>,
    bytes: usize,
}

impl<S: Segmented> Item for Hour<S> {
    fn heap(&self) -> usize {
        self.bytes
    }
}

/// One table a family reads: the fields it reads, and whether each hour it was read is read too.
#[derive(Clone, Copy)]
struct Watch {
    table: TableId,
    fields: u64,
    read: bool,
}

fn watches<S: Segmented>() -> io::Result<Vec<Watch>> {
    let named = S::fields()?;
    Ok(S::TABLES
        .iter()
        .map(
            |&table| match named.iter().find(|fields| fields.table == table) {
                Some(&TableFields { fields, read, .. }) => Watch {
                    table,
                    fields,
                    read,
                },
                None => Watch {
                    table,
                    fields: table.full_mask(),
                    read: false,
                },
            },
        )
        .collect())
}

/// How a family asks the world for its tables: each in the fields it reads.
fn asks(watches: &[Watch]) -> Vec<(TableId, Hold)> {
    watches
        .iter()
        .map(|watch| (watch.table, Hold::Fields(watch.fields).of(watch.table)))
        .collect()
}

/// What a keyed family's operators read of the walk's world: its tables, each in the fields it
/// reads.
pub fn needs<S: Segmented>() -> io::Result<Needs> {
    Ok(Needs {
        frame: asks(&watches::<S>()?),
        ..Needs::default()
    })
}

/// One segment's operator of the keyed family `S`.
pub struct Operator<S: Segmented> {
    format: FormatVersion,
    watches: Vec<Watch>,
    asks: Vec<(TableId, Hold)>,
    /// Each watched table's status the hour before.
    shapes: [Option<columns::Status>; TABLE_COUNT],
    /// The frame the world's views are lent into for an hour, given back before it ends.
    frame: DecodedFrame,
    touched: Touched,
    reader: Box<dyn Reader<S>>,
}

impl<S: Segmented> Operator<S> {
    pub fn new(format: FormatVersion) -> io::Result<Self> {
        Self::with_reader(format, S::reader(format))
    }

    /// An operator handing its hours to `reader`, in place of [`Segmented::reader`]'s.
    pub fn with_reader(format: FormatVersion, reader: Box<dyn Reader<S>>) -> io::Result<Self> {
        let watches = watches::<S>()?;
        Ok(Self {
            format,
            asks: asks(&watches),
            watches,
            shapes: [None; TABLE_COUNT],
            frame: DecodedFrame {
                branch: 0,
                hour: 0,
                tables: std::array::from_fn(|_| not_selected()),
                metrics: Metrics::default(),
            },
            touched: Touched::default(),
            reader,
        })
    }
}

/// What `hour` changed of `watch`'s table, its status the hour before `shape`, into `touch`.
fn touch(
    hour: &segments::Hour<'_, World>,
    watch: &Watch,
    shape: Option<columns::Status>,
    touch: &mut Touch,
) -> io::Result<()> {
    touch.clear();
    let status = columns::status(hour.state, watch.table);
    // A segment's first hour gives every table whole, as a block's first does.
    if hour.starts || hour.opens || shape != Some(status) {
        touch.restated = true;
        return Ok(());
    }
    if status != columns::Status::Complete {
        return Ok(());
    }
    let held = columns::table(hour.state, watch.table)?;
    let edits = columns::changes(hour.state, watch.table)?;
    for (changed, masks) in [
        (&edits.changed, &edits.masks),
        (&edits.fill_slots, &edits.fill_masks),
    ] {
        if changed.len() != masks.len() {
            return Err(invalid("A table's slot masks miss their rows"));
        }
        for (&slot, &mask) in changed.iter().zip(masks) {
            if mask & watch.fields != 0
                && let Some(key) = held.key(slot)
            {
                touch.written.push((key, mask & watch.fields));
            }
        }
    }
    for &slot in &edits.inserted {
        if let Some(key) = held.key(slot) {
            touch.written.push((key, u64::MAX));
        }
    }
    touch.written.sort_unstable_by_key(|&(key, _)| key);
    touch.written.dedup_by(|(key, mask), (kept, fields)| {
        let same = key == kept;
        if same {
            *fields |= *mask;
        }
        same
    });
    touch.deleted.extend_from_slice(&edits.deleted_keys);
    touch.deleted.sort_unstable();
    touch.deleted.dedup();
    // A key deleted and inserted again within the hour is a row written.
    let written = &touch.written;
    touch
        .deleted
        .retain(|key| written.binary_search_by_key(key, |&(held, _)| held).is_err());
    Ok(())
}

impl<S: Segmented> segments::Operator<World> for Operator<S> {
    type Output = Hour<S>;

    fn hour(
        &mut self,
        hour: &segments::Hour<'_, World>,
        out: &mut dyn Sink<Hour<S>>,
    ) -> io::Result<()> {
        let measured = metrics(hour.manifest);
        let mut visited = false;
        let mut rows_changed = [false; TABLE_COUNT];
        for watch in &self.watches {
            let index = watch.table.index();
            touch(
                hour,
                watch,
                self.shapes[index],
                self.touched.of_mut(watch.table),
            )?;
            self.shapes[index] = Some(columns::status(hour.state, watch.table));
            visited |= watch.read && measured.table_ns[index] != 0;
            rows_changed[index] = self.touched.of(watch.table).any();
        }
        if !visited && !rows_changed.contains(&true) {
            // An hour no read field changed at is still read where its reads are.
            let facts = FrameChanges::observed(
                [false; TABLE_COUNT],
                std::array::from_fn(|index| measured.table_ns[index] != 0),
                dashboard_text::metadata::slot_reads(self.format, &measured, hour.hour),
                None,
            );
            if !S::changed(&facts) {
                return out.push(Hour {
                    hour: hour.hour,
                    dated: facts.slot_read[S::READ.index()],
                    read: None,
                    bytes: 0,
                });
            }
        }
        // The views go back before the hour ends, however it ends, so the next hour's builds
        // change them in place.
        let lent = Lent {
            frame: &mut self.frame,
            asks: &self.asks,
        };
        hour.world.frame_held_into(hour.state, lent.asks, lent.frame)?;
        for watch in &self.watches {
            let index = watch.table.index();
            lent.frame.tables[index] = lent.frame.tables[index]
                .clone()
                .with_sampled_at(hour.state.sampled_at(index));
        }
        let mut facts = FrameChanges::alone(lent.frame, self.format);
        facts.rows_changed = rows_changed;
        let (read, bytes) = self.reader.read(lent.frame, &facts, &self.touched)?;
        drop(lent);
        out.push(Hour {
            hour: hour.hour,
            dated: facts.slot_read[S::READ.index()],
            read: Some(read),
            bytes,
        })
    }
}

/// The world's views lent into `frame` for `asks`, given back when dropped.
struct Lent<'o> {
    frame: &'o mut DecodedFrame,
    asks: &'o [(TableId, Hold)],
}

impl Drop for Lent<'_> {
    fn drop(&mut self) {
        dashboard_world::release(self.frame, self.asks);
    }
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
