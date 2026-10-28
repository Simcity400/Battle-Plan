//! Shared block pre-pass. Country reductions omit unrelated hourly balances;
//! owner-read evidence is observed at every hour, including rowless slices.
use super::prepared::Prepared;
use dashboard_base::prepared::{
    frame_changes::FrameChanges,
    kept::{self, Reduced},
    prepass::{Extract, Family, Folded, HeldBytes},
};
use replay::container::{BlockIndex, Replay, TableFields};
use replay_data::{
    format::FormatVersion,
    rows::{CountryRow, TableId},
};
use replay_frame::frame::{DecodedFrame, FrameLog};
use std::{any::Any, io};

pub const TABLES: &[TableId] = &[
    TableId::Campaign,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::SliceReads,
    TableId::RelationOpinions,
    TableId::RelationTuples,
    TableId::IncomingDiplomaticActions,
    TableId::LexicalTokens,
];
pub(super) const OPINION_TABLES: &[TableId] = &[
    TableId::Campaign,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::SliceReads,
    TableId::RelationOpinions,
];
pub static FAMILY: Family = Family {
    name: "diplomacy_state",
    extract: "diplomacy_state.extract",
    sequential: "diplomacy_state.sequential",
    fallback: "diplomacy_state.fallback",
    tables: TABLES,
    fields: Some(fields),
    start,
};
fn fields() -> io::Result<Vec<TableFields>> {
    let mut fields = vec![TableFields::named(
        TableId::Countries,
        &["canonical_slot", "tag"],
    )?];
    fields.extend(
        TABLES
            .iter()
            .filter(|&&table| table != TableId::Countries && table != TableId::Campaign)
            .map(|&table| TableFields::all(table)),
    );
    Ok(fields)
}
fn start(_: FormatVersion) -> Box<dyn Extract> {
    Box::new(Builder {
        hours: Vec::new(),
        frames: FrameLog::new(),
        countries: Reduced::new(TableId::Countries),
        next: None,
    })
}
/// Every hour of a block: its change facts and its frame.
struct Builder {
    hours: Vec<FrameChanges>,
    frames: FrameLog,
    countries: Reduced<CountryRow>,
    next: Option<i32>,
}
struct Facts {
    hours: Vec<FrameChanges>,
    frames: FrameLog,
}
impl Extract for Builder {
    fn bytes(&self) -> u64 {
        let mut held = HeldBytes::default();
        held.add(self.hours.capacity() * size_of::<FrameChanges>());
        for changes in &self.hours {
            held.changes(changes);
        }
        for view in self.frames.views() {
            held.view(view);
        }
        held.add(self.frames.bytes());
        held.total()
    }
    fn observe(
        &mut self,
        _: &Replay,
        frame: &DecodedFrame,
        changes: &FrameChanges,
    ) -> io::Result<()> {
        let mut frame = frame.clone();
        self.countries.countries(&mut frame);
        self.next = frame.hour.checked_add(1);
        self.frames.push(&frame)?;
        self.hours.push(kept::kept_changes(changes));
        Ok(())
    }
    fn deadline(&self) -> Option<i32> {
        self.next
    }
    fn finish(self: Box<Self>) -> io::Result<Box<dyn Any + Send>> {
        Ok(Box::new(Facts {
            hours: self.hours,
            frames: self.frames,
        }))
    }
}
pub fn fold(
    scan: &mut Prepared,
    facts: Box<dyn Any + Send>,
    replay: &Replay,
    observed: &mut kept::Observed<'_>,
    check: &mut dyn FnMut() -> io::Result<()>,
) -> io::Result<(Folded, Option<i32>)> {
    let facts = facts
        .downcast::<Facts>()
        .map_err(|_| io::Error::other("Diplomacy received another family's block facts"))?;
    let Facts { hours, frames } = *facts;
    let mut frames = frames.into_frames();
    let mut reached = None;
    for changes in hours {
        check()?;
        let frame = frames
            .next()
            .ok_or_else(|| io::Error::other("Diplomacy facts lack an hour's frame"))??;
        reached = Some(frame.hour);
        if !observed(
            frame.hour,
            scan.observe_checked(&frame, replay.format, &changes, check),
        )? {
            break;
        }
    }
    Ok((Folded::Fast, reached))
}
pub fn observe_block(
    scan: &mut Prepared,
    replay: &Replay,
    index: &BlockIndex,
    observed: &mut kept::Observed<'_>,
    check: &mut dyn FnMut() -> io::Result<()>,
) -> io::Result<Option<i32>> {
    let check = std::cell::RefCell::new(check);
    kept::observe_hours(
        replay,
        index,
        TABLES,
        None,
        observed,
        &mut || check.borrow_mut()(),
        |frame, changes| {
            scan.observe_checked(frame, replay.format, changes, &mut *check.borrow_mut())
        },
    )
}
