//! Shared block pre-pass. Country reductions omit unrelated hourly balances;
//! owner-read evidence is observed at every hour, including rowless slices.
use super::prepared::Prepared;
use dashboard_base::prepared::{
    frame_changes::FrameChanges,
    kept::{self, BlockFacts, Hour, Reduced},
    prepass::{Extract, Family, Folded},
};
use replay::container::{BlockIndex, Replay, TableFields};
use replay_data::{
    format::FormatVersion,
    rows::{CountryRow, TableId},
};
use replay_frame::frame::DecodedFrame;
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
        countries: Reduced::new(TableId::Countries),
        next: None,
    })
}
struct Builder {
    hours: Vec<Hour>,
    countries: Reduced<CountryRow>,
    next: Option<i32>,
}
impl Extract for Builder {
    fn bytes(&self) -> u64 {
        kept::hours_bytes(&self.hours)
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
        self.hours.push(Hour::Observed {
            frame,
            changes: kept::kept_changes(changes),
        });
        Ok(())
    }
    fn deadline(&self) -> Option<i32> {
        self.next
    }
    fn finish(self: Box<Self>) -> io::Result<Box<dyn Any + Send>> {
        Ok(Box::new(BlockFacts {
            frames: self.hours.len(),
            hours: self.hours,
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
        .downcast::<BlockFacts>()
        .map_err(|_| io::Error::other("Diplomacy received another family's block facts"))?;
    let mut reached = None;
    for hour in facts.hours {
        check()?;
        let Hour::Observed { frame, changes } = hour else {
            return Err(io::Error::other(
                "Diplomacy owner read evidence omitted an hour",
            ));
        };
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
