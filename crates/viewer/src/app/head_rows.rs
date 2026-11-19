//! The window's readers of the hour besides its tooltips and panels: the selection observers and a
//! selection's movement arrows, the clicks and menu of the overlay's map modes, the walk camera's
//! figure, the clock's speed and the picker's fourth button. Each reads rows at the head while the
//! map's playhead is open, and nothing else: while those rows do not hold what it reads it keeps
//! what it gave and is loading by name. Today's frame is read only with no open playhead map. Each
//! is open only while it is asked: a selection's while something is selected, the walk camera's
//! while walking; the clock's and the picker's every frame the open map draws. The overlay itself
//! is made on the map side's thread ([`crate::playhead_host::overlay`]).

use crate::{
    document::Document,
    playhead_host::views::{AtHead, Need, Pace, Views},
};
use replay_data::{Hour, rows::TableId};
use replay_frame::frame::DecodedFrame;


#[cfg(test)]
mod tests;

/// The open map's view store, whose mirrors every reader here reads the hour from; none while the
/// map's playhead is off.
pub(crate) type Head<'a> = Option<&'a Views>;

/// What a person sees change on the map: a selection, an icon, the clock's speed; at the step.
const PACE: Pace = Pace::Step;

pub(crate) fn head(side: Option<&crate::playhead_host::side::SideThread>) -> Head<'_> {
    side.map(crate::playhead_host::side::SideThread::views)
}

/// The branch the open map's rows are stamped with, which follows the clock's; the document's
/// with no open map.
pub(crate) fn branch(side: Option<&crate::playhead_host::side::SideThread>, document: &Document) -> replay_data::BranchId {
    side.and_then(crate::playhead_host::side::SideThread::branch).unwrap_or_else(|| document.playback.branch())
}

#[derive(Debug)]
pub(crate) struct HeadRows {
    /// The observers ask only whether a key lives: they hold keys, no column.
    units: AtHead,
    arrows: AtHead,
    air: AtHead,
    fleets: AtHead,
    /// A click or menu of the overlay's map modes: the overlay's own tables, read on the frame
    /// thread for the click alone.
    clicks: AtHead,
    walk: AtHead,
    map: AtHead,
    /// The fourth button's answer at the head, kept while the rows load.
    four: Option<bool>,
}

impl Default for HeadRows {
    fn default() -> Self {
        Self {
            units: AtHead::with("selection units", &[Need::keys(TableId::Divisions), Need::keys(TableId::RailwayGunCounterInputs), Need::keys(TableId::TaskForces)], PACE).keeps_its_answer(),
            arrows: AtHead::new("selection arrows", &[], PACE).keeps_its_answer(),
            air: AtHead::with("selection air", &[Need::keys(TableId::AirWings), Need::keys(TableId::AirBaseRegistrations)], PACE).keeps_its_answer(),
            fleets: AtHead::with("selection fleets", &[Need::keys(TableId::Fleets)], PACE).keeps_its_answer(),
            clicks: AtHead::new("overlay clicks", &crate::playhead_host::overlay::overlay_tables(), PACE),
            walk: AtHead::new("walk figure", &[], PACE).keeps_its_answer(),
            map: AtHead::new("clock and picker", &[TableId::Clock, TableId::SimulationFeatures], PACE).keeps_its_answer(),
            four: None,
        }
    }
}

/// `read` over the rows at the head of `head`'s playhead when it is open, learning the tables it
/// reads; none while those rows load. Over `today` only with no open map.
fn read_at<T>(at: &mut AtHead, head: Head<'_>, today: Option<&DecodedFrame>, mut read: impl FnMut(&DecodedFrame) -> T) -> Option<T> {
    match head {
        Some(views) => {
            let rows = at.frame(views)?;
            at.discover(|| read(&rows))
        }
        None => today.map(read),
    }
}

impl HeadRows {
    /// The unit selection's observer: it reads only the key fields of its rows.
    pub(crate) fn units<T>(&mut self, head: Head<'_>, today: Option<&DecodedFrame>, read: impl FnMut(&DecodedFrame) -> T) -> Option<T> {
        read_at(&mut self.units, head, today, read)
    }

    /// The unit selection's movement arrows, whose tables are learned whole.
    pub(crate) fn arrows<T>(&mut self, head: Head<'_>, today: Option<&DecodedFrame>, read: impl FnMut(&DecodedFrame) -> T) -> Option<T> {
        read_at(&mut self.arrows, head, today, read)
    }

    /// The air selection's observer: keys only.
    pub(crate) fn air<T>(&mut self, head: Head<'_>, today: Option<&DecodedFrame>, read: impl FnMut(&DecodedFrame) -> T) -> Option<T> {
        read_at(&mut self.air, head, today, read)
    }

    /// The fleet selection's observer: keys only.
    pub(crate) fn fleets<T>(&mut self, head: Head<'_>, today: Option<&DecodedFrame>, read: impl FnMut(&DecodedFrame) -> T) -> Option<T> {
        read_at(&mut self.fleets, head, today, read)
    }

    /// The walk camera's figure.
    pub(crate) fn walk<T>(&mut self, head: Head<'_>, today: Option<&DecodedFrame>, read: impl FnMut(&DecodedFrame) -> T) -> Option<T> {
        read_at(&mut self.walk, head, today, read)
    }

    /// The hour the walk camera's figure is read at while rows at the head give it.
    pub(crate) fn walk_hour(&mut self, head: Head<'_>) -> Option<Hour> {
        self.walk.frame(head?).map(|rows| rows.hour)
    }

    /// The clock's speed and the picker's fourth button.
    pub(crate) fn map<T>(&mut self, head: Head<'_>, today: Option<&DecodedFrame>, read: impl FnMut(&DecodedFrame) -> T) -> Option<T> {
        read_at(&mut self.map, head, today, read)
    }

    /// Whether the recording owns the fourth large button's feature: at the head with an open map,
    /// the last answer while its rows load and today's default before the first; today's without.
    pub(crate) fn four_buttons(&mut self, head: Head<'_>, document: &Document) -> bool {
        if head.is_none() {
            return super::picker::four_buttons(document);
        }
        if let Some(four) = self.map(head, None, super::picker::four_buttons_at) {
            self.four = Some(four);
        }
        self.four.unwrap_or(true)
    }


    /// A click or menu of the overlay's map mode, over its inputs' rows; today's frame with no open
    /// map. None while the rows load.
    pub(crate) fn clicks<T>(&mut self, head: Head<'_>, today: Option<&DecodedFrame>, read: impl FnMut(&DecodedFrame) -> T) -> Option<T> {
        read_at(&mut self.clicks, head, today, read)
    }

    /// Once a frame, after the last ask: a reader not asked this frame closes and retires its
    /// tables.
    pub(crate) fn end_frame(&mut self) {
        for at in [&mut self.units, &mut self.arrows, &mut self.air, &mut self.fleets, &mut self.walk, &mut self.map, &mut self.clicks] {
            at.end_frame();
        }
    }
}
