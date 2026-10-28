//! A block's kept hours, for a family prepared on its own whose fold observes
//! again, on its carried state, the hours its fields change (raids and air
//! aces, whose changes are rare). A pre-pass worker reads the block only at
//! those hours and keeps each one's frame, as the family reads it, with its
//! change facts; the quiet hours between keep only their read evidence, in
//! runs of equal evidence.
//!
//! The fold observes each kept hour as hourly preparation does, and each
//! quiet hour on the last kept frame at that hour, with its read evidence:
//! every field the family reads is the last kept hour's there. The first
//! quiet hour after a kept hour has the tables hourly preparation marks
//! changed there (those decoded again); the later ones too for a family
//! that reads its tables only at its daily reads ([`Rebuilt::Every`]), and
//! none for one that reads them every hour ([`Rebuilt::First`]), where
//! observing rows equal to the hour before's again only extends what that
//! hour holds.
//! Observers read an hour's read evidence from its change facts, never from
//! the frame's metrics, which a quiet hour's frame does not carry.

use dashboard_base::prepared::frame_changes::FrameChanges;
use dashboard_text::metadata::{CountryKeys, slot_read};
use replay::container::QuietHours;
use replay_data::rows::TABLE_COUNT;
use replay_data::{
    format::FormatVersion, rows::CountryRow, rows::TableId, rows::TableRows, rows::tables,
};
use replay_frame::{frame::DecodedFrame, frame::FrameLog, frame::SharedView, frame::TableView};
use std::{io, ops::Range};

/// The countries of kept frames reduced to the keys country identity reads
/// (`CountryKeys`: id, canonical slot, tag), whose balances change every
/// hour, and shared by kept hours with equal keys.
#[derive(Default)]
pub struct Countries(Option<(CountryKeys, SharedView)>);

impl Countries {
    pub fn reduce(&mut self, frame: &mut DecodedFrame) {
        let view = match frame.complete::<tables::Countries>() {
            Ok(rows) => {
                let held = self
                    .0
                    .as_ref()
                    .filter(|(keys, _)| keys.same(rows))
                    .map(|(_, view)| view.clone());
                match held {
                    Some(view) => view,
                    None => {
                        let keys = CountryKeys::read(rows);
                        let view: SharedView = TableView::Complete(TableRows::Countries(
                            keys.iter()
                                .map(|(id, canonical_slot, tag)| CountryRow {
                                    id,
                                    canonical_slot,
                                    tag: tag.to_vec(),
                                    ..Default::default()
                                })
                                .collect(),
                        ))
                        .into();
                        self.0 = Some((keys, view.clone()));
                        view
                    }
                }
            }
            Err(_) => return,
        };
        frame.tables[TableId::Countries.index()] = view;
    }
}

/// A complete table of kept frames reduced to the fields a family reads
/// (`reduce` keeps a row's read fields, defaulting the rest), shared by kept
/// hours whose reduced rows are equal. Truncated and unavailable views are
/// kept as they are.
pub struct Reduced<R> {
    table: TableId,
    held: Option<(Vec<R>, SharedView)>,
}

impl<R: PartialEq + Clone> Reduced<R> {
    pub fn new(table: TableId) -> Self {
        Self { table, held: None }
    }

    /// Replace the frame's table by `reduced`, its reduced complete rows,
    /// shared with the last kept hour's when equal; `wrap` makes the table's
    /// rows of them.
    pub fn keep(
        &mut self,
        frame: &mut DecodedFrame,
        reduced: Option<Vec<R>>,
        wrap: fn(Vec<R>) -> TableRows,
    ) {
        let Some(rows) = reduced else {
            return;
        };
        let shared = self
            .held
            .as_ref()
            .filter(|(held, _)| *held == rows)
            .map(|(_, view)| view.clone());
        let view = match shared {
            Some(view) => view,
            None => {
                let view: SharedView = TableView::Complete(wrap(rows.clone())).into();
                self.held = Some((rows, view.clone()));
                view
            }
        };
        frame.tables[self.table.index()] = view;
    }
}

/// One hour, or run of quiet hours, of a block, in hour order.
pub enum Hour {
    /// An hour the family observes, with its change facts; its frame is the
    /// next its block's kept frames hold.
    Kept { changes: FrameChanges },
    /// Consecutive hours `first..=last` at which nothing the family reads
    /// changed, each with the read evidence `read`.
    Quiet {
        first: i32,
        last: i32,
        read: Evidence,
    },
}

/// An hour's read evidence of a family's tables (none of other tables), and
/// the tables marked changed there.
#[derive(Clone, PartialEq, Eq)]
pub struct Evidence {
    rows_changed: [bool; TABLE_COUNT],
    table_read: [bool; TABLE_COUNT],
    slot_read: [bool; TABLE_COUNT],
}

impl Evidence {
    /// The `position`th of `hours`; with `rebuilt`, its tables are marked
    /// changed as hourly preparation marks them (decoded again), otherwise
    /// none is.
    fn of(
        tables: &[TableId],
        format: FormatVersion,
        hours: &QuietHours,
        position: usize,
        rebuilt: bool,
    ) -> Self {
        let hour = hours.hours()[position];
        let metrics = &hours.metrics()[position];
        let mut evidence = Self {
            rows_changed: [false; TABLE_COUNT],
            table_read: [false; TABLE_COUNT],
            slot_read: [false; TABLE_COUNT],
        };
        for &table in tables {
            evidence.rows_changed[table.index()] = rebuilt && hours.rebuilt(position, table);
            evidence.table_read[table.index()] = hours.read(position, table);
            evidence.slot_read[table.index()] = slot_read(format, metrics, table, hour);
        }
        evidence
    }

    /// The change facts of a quiet hour.
    fn changes(&self) -> FrameChanges {
        let _cpu = ::replay_frame::cpu::scope("frame changes", None);
        FrameChanges {
            rows_changed: self.rows_changed,
            table_read: self.table_read,
            slot_read: self.slot_read,
            updates: None,
        }
    }
}

/// Which quiet hours have the tables hourly preparation marks changed there
/// (those decoded again).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Rebuilt {
    /// The first quiet hour after a kept hour: a family that reads its
    /// tables every hour, so every change it has not yet read is a kept hour.
    First,
    /// Every quiet hour: a family that reads a table only at its daily read
    /// and then decides by these marks whether to read it again, so a row
    /// changed at an hour it did not read is read at the next read only if
    /// a table was decoded again there, as hourly preparation reads it.
    Every,
}

/// A block's kept hours, as its extract finds them.
pub struct Kept {
    tables: &'static [TableId],
    format: FormatVersion,
    rebuilt: Rebuilt,
    hours: Vec<Hour>,
    frames: FrameLog,
}

/// A block's hours and the frames of those kept ([`Kept::finish`]).
pub struct Hours {
    hours: Vec<Hour>,
    frames: FrameLog,
}

impl Kept {
    /// What the kept hours hold ([`dashboard_base::prepared::prepass::Extract::bytes`]):
    /// the kept frames' views once and their change facts.
    pub fn bytes(&self) -> u64 {
        let mut held = dashboard_base::prepared::prepass::HeldBytes::default();
        held.add(self.hours.capacity() * size_of::<Hour>());
        for hour in &self.hours {
            if let Hour::Kept { changes } = hour {
                held.changes(changes);
            }
        }
        for view in self.frames.views() {
            held.view(view);
        }
        held.add(self.frames.bytes());
        held.total()
    }

    /// Hours of a family reading `tables`, in a recording of `format`, its
    /// quiet hours marked `rebuilt`.
    pub fn new(tables: &'static [TableId], format: FormatVersion, rebuilt: Rebuilt) -> Self {
        Self {
            tables,
            format,
            rebuilt,
            hours: Vec::new(),
            frames: FrameLog::new(),
        }
    }

    /// An observed hour: its frame as the family reads it, with its change
    /// facts.
    pub fn keep(&mut self, frame: &DecodedFrame, changes: &FrameChanges) -> io::Result<()> {
        self.frames.push(frame)?;
        self.hours.push(Hour::Kept {
            changes: FrameChanges {
                rows_changed: changes.rows_changed,
                table_read: changes.table_read,
                slot_read: changes.slot_read,
                updates: None,
            },
        });
        Ok(())
    }

    /// Quiet hours `span` of `hours`, joined into runs of equal evidence.
    /// The first quiet hour after a kept hour is always observed with the
    /// tables hourly preparation marks changed there: equal rows observed
    /// again can end a run that moved until the hour before (a raid's
    /// preparation stops), which hourly preparation publishes at that hour.
    /// After it, for a family reading every hour ([`Rebuilt::First`]),
    /// observing equal rows again extends what the hour before holds, as
    /// observing them unchanged does.
    pub fn quiet(&mut self, hours: &QuietHours, span: Range<usize>) -> io::Result<()> {
        if !matches!(self.hours.first(), Some(Hour::Kept { .. })) {
            return Err(io::Error::other(
                "Quiet hours came before the block's first",
            ));
        }
        let after_kept = matches!(self.hours.last(), Some(Hour::Kept { .. }));
        let first = span.start;
        for position in span {
            let hour = hours.hours()[position];
            let rebuilt = self.rebuilt == Rebuilt::Every || (after_kept && position == first);
            let read = Evidence::of(self.tables, self.format, hours, position, rebuilt);
            if let Some(Hour::Quiet {
                last, read: held, ..
            }) = self.hours.last_mut()
                && last.checked_add(1) == Some(hour)
                && *held == read
            {
                *last = hour;
                continue;
            }
            self.hours.push(Hour::Quiet {
                first: hour,
                last: hour,
                read,
            });
        }
        Ok(())
    }

    /// The block's hours, in hour order: the first kept.
    pub fn finish(self) -> io::Result<Hours> {
        if !matches!(self.hours.first(), Some(Hour::Kept { .. })) {
            return Err(io::Error::other("A block has no first hour"));
        }
        Ok(Hours {
            hours: self.hours,
            frames: self.frames,
        })
    }
}

/// A family's observation of one hour's outcome: `Ok(true)` while it goes
/// on, `Ok(false)` once it refused the branch.
pub type Observed<'a> = dyn FnMut(i32, io::Result<()>) -> io::Result<bool> + 'a;

/// Observe a block's `hours` on the carried state (`observe`), each kept
/// hour on its frame and each quiet hour on the last kept frame at its hour.
/// Returns the last hour observed and how many hours were kept.
pub fn fold(
    hours: Hours,
    observe: &mut dyn FnMut(&DecodedFrame, &FrameChanges) -> io::Result<()>,
    observed: &mut Observed<'_>,
    check: &mut dyn FnMut() -> io::Result<()>,
) -> io::Result<(Option<i32>, usize)> {
    let _cpu = ::replay_frame::cpu::scope("fold driver", None);
    let Hours { hours, frames } = hours;
    let mut frames = frames.into_frames();
    let mut reached = None;
    let mut kept = 0;
    let mut last: Option<DecodedFrame> = None;
    for hour in hours {
        let _cpu = ::replay_frame::cpu::scope("fold driver", None);
        check()?;
        match hour {
            Hour::Kept { changes } => {
                // Released first, so the next frame changes it in place.
                drop(last.take());
                let frame = frames.next().ok_or_else(|| {
                    io::Error::other("Kept hours lack an observed hour's frame")
                })??;
                kept += 1;
                let hour = frame.hour;
                reached = Some(hour);
                let result = {
                    let _cpu = ::replay_frame::cpu::scope("fold observe", None);
                    observe(&frame, &changes)
                };
                last = Some(frame);
                if !observed(hour, result)? {
                    return Ok((reached, kept));
                }
            }
            Hour::Quiet {
                first,
                last: end,
                read,
            } => {
                let frame = last
                    .as_mut()
                    .ok_or_else(|| io::Error::other("Quiet hours came before the block's first"))?;
                let changes = read.changes();
                for hour in first..=end {
                    let _cpu = ::replay_frame::cpu::scope("fold driver", None);
                    frame.hour = hour;
                    reached = Some(hour);
                    let result = {
                        let _cpu = ::replay_frame::cpu::scope("fold observe", None);
                        observe(frame, &changes)
                    };
                    if !observed(hour, result)? {
                        return Ok((reached, kept));
                    }
                }
            }
        }
    }
    Ok((reached, kept))
}
