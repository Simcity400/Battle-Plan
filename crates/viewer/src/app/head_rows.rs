//! The window's readers of the hour besides its tooltips and panels: the selection observers and a
//! selection's movement arrows, the clicks and menu of the overlay's map modes, the walk camera's
//! figure, the clock's speed, the picker's fourth button and the province order cursor. Each reads rows at the head while the
//! map's playhead is open, and nothing else: while those rows do not hold what it reads it keeps
//! what it gave and is loading by name. Today's frame is read only with no open playhead map. Each
//! is open only while it is asked: a selection's while something is selected, the walk camera's
//! while walking, the order cursor's while a selection is over the ground; the clock's and the picker's every frame the open map draws. The clicks' is open
//! while the map is, holding only the tables of what the view can click ([`click_tables`]), and a
//! click reads the rows of the hour it was made at ([`super::held_clicks`]). The overlay itself is made on the map side's thread
//! ([`crate::playhead_host::overlay`]).

use crate::{
    document::Document,
    playhead_host::views::{AtHead, Need, Pace, Views},
};
use replay_data::{BranchId, Hour, rows::TableId};
use replay_frame::frame::DecodedFrame;
use viewer_counters::map::counters::Shown;


#[cfg(test)]
mod tests;

/// The open map's view store, whose mirrors every reader here reads the hour from; none while the
/// map's playhead is off.
pub(crate) type Head<'a> = Option<&'a Views>;

/// What a person sees change on the map: a selection, an icon, the clock's speed; at the step.
const PACE: Pace = Pace::Step;

/// What a click can read at any view: the ground's and Ctrl+click's provinces, states and owners,
/// the fleet roster's and a naval base's fleets, an air base's province, a raid offer's targets and
/// the selection sounds' divisions and countries. Each is a table the overlay reads at every view
/// too, so its rows are made once for both, but for the few no overlay family names.
const CLICKS: &[TableId] = &[
    TableId::Provinces,
    TableId::States,
    TableId::StateFlags,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CountryAppearance,
    TableId::ReservedCountryNaming,
    TableId::CountryGameScoreInputs,
    TableId::Divisions,
    TableId::LandCombats,
    TableId::TaskForces,
    TableId::Fleets,
    TableId::Ships,
    TableId::NavalRepairQueue,
    TableId::AirRegionalMissions,
    TableId::Raids,
    TableId::RaidTargets,
    TableId::RaidFacilities,
];

/// What only a naval mission bubble's click reads.
const NAVAL_MISSION_CLICKS: &[TableId] = &[TableId::NavalMissionLedgers];

/// What only the air clicks read: an air base's, an air grid's, an air mission's, a box's wings and
/// an air order's.
const AIR_CLICKS: &[TableId] = &[TableId::AirBases, TableId::AirBaseRegistrations, TableId::AirReferenceWitnesses, TableId::AirWings];

/// What only a selected division's sounds read.
const DIVISION_CLICKS: &[TableId] = &[TableId::ArmyIconInputs, TableId::DivisionTemplateIcons];

/// What only a raid offer's details read beyond [`CLICKS`]: a captive leader's name.
const RAID_CLICKS: &[TableId] = &[TableId::UnitLeaders];

/// What the map can be clicked on at a view: each family's click reads its tables only while it
/// can be clicked, so the click reader makes no hidden family's rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Clickable {
    /// The icon families the overlay on screen and the one asked for draw.
    pub shown: Shown,
    /// Air bases, air grids or air missions are drawn, or air wings are selected.
    pub air: bool,
    /// Divisions can be selected: their counters or their models are drawn.
    pub divisions: bool,
    /// The overlay draws a raid offer.
    pub raids: bool,
}

impl Clickable {
    /// Nothing but the ground.
    pub(crate) const GROUND: Self = Self { shown: Shown { units: false, bases: false, land_combats: false, naval_missions: false }, air: false, divisions: false, raids: false };
}

/// The tables the click reader declares at a view that can click `clickable`, with the tables
/// clicks were found to read beyond them (`learned`).
pub(crate) fn click_tables(clickable: Clickable, learned: &[TableId]) -> Vec<TableId> {
    let family = |held: bool, tables: &'static [TableId]| held.then_some(tables).unwrap_or_default();
    let mut tables: Vec<TableId> = CLICKS
        .iter()
        .chain(family(clickable.shown.naval_missions, NAVAL_MISSION_CLICKS))
        .chain(family(clickable.air, AIR_CLICKS))
        .chain(family(clickable.divisions, DIVISION_CLICKS))
        .chain(family(clickable.raids, RAID_CLICKS))
        .chain(learned)
        .copied()
        .collect();
    tables.sort_unstable_by_key(|table| table.index());
    tables.dedup();
    tables
}

/// The tables only the overlay's icon families `shown` leaves out read.
pub(crate) fn hidden_families(shown: Shown) -> Vec<TableId> {
    use crate::playhead_host::overlay::{overlay_tables, tables_shown};
    let drawn = tables_shown(shown);
    overlay_tables().into_iter().filter(|table| !drawn.contains(table)).collect()
}

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
    /// A click or menu of the overlay's map modes: the tables what can be clicked at the view reads
    /// ([`click_tables`]), read on the frame thread at the hour the click was made.
    clicks: AtHead,
    /// What can be clicked at the view, which the click reader's tables follow.
    clickable: Clickable,
    /// The tables a click read beyond [`click_tables`], learned whole.
    learned: Vec<TableId>,
    /// The branch and hour a click reads: the overlay's on screen, or a held click's own.
    click_at: Option<(BranchId, Hour)>,
    walk: AtHead,
    map: AtHead,
    /// The find view's index, while the view is open.
    find: AtHead,
    /// The province order cursor over the first selected unit, its tables learned whole.
    cursor: AtHead,
    /// The recording client's unit list order the model picker walks, with the hour it was read at.
    order: AtHead,
    order_read: Option<(BranchId, Hour, super::selection_input::ClientOrder)>,
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
            clicks: AtHead::new("overlay clicks", &click_tables(Clickable::GROUND, &[]), PACE),
            clickable: Clickable::GROUND,
            learned: Vec::new(),
            click_at: None,
            walk: AtHead::new("walk figure", &[], PACE).keeps_its_answer(),
            map: AtHead::new("clock and picker", &[TableId::Clock, TableId::SimulationFeatures], PACE).keeps_its_answer(),
            find: AtHead::new("find view", &[], PACE).keeps_its_answer(),
            cursor: AtHead::new("order cursor", &[], PACE).keeps_its_answer(),
            order: AtHead::new("unit order", &[TableId::LandUnitOrder, TableId::LocalPlayer, TableId::CountryLinks], PACE).keeps_its_answer(),
            order_read: None,
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

    /// The find view's index over one frame of the rows at the head, which carries their hour;
    /// none with no open map.
    pub(crate) fn find<T>(&mut self, head: Head<'_>, read: impl FnMut(&DecodedFrame) -> T) -> Option<T> {
        read_at(&mut self.find, Some(head?), None, read)
    }

    /// The province order cursor.
    pub(crate) fn cursor<T>(&mut self, head: Head<'_>, today: Option<&DecodedFrame>, read: impl FnMut(&DecodedFrame) -> T) -> Option<T> {
        read_at(&mut self.cursor, head, today, read)
    }

    /// The recording client's unit list order and the country it views, at the head; read once an
    /// hour. None while those rows load.
    pub(crate) fn unit_order(&mut self, head: Head<'_>) -> Option<super::selection_input::ClientOrder> {
        let rows = self.order.frame(head?)?;
        if let Some((branch, hour, read)) = &self.order_read
            && (*branch, *hour) == (rows.branch, rows.hour)
        {
            return Some(read.clone());
        }
        let read = self.order.discover(|| super::selection_input::recorded_unit_order(&rows))?;
        self.order_read = Some((rows.branch, rows.hour, read.clone()));
        Some(read)
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


    /// The branch and hour clicks read from now on; none while the map shows no hour.
    pub(crate) fn click_at(&mut self, at: Option<(BranchId, Hour)>) {
        self.click_at = at;
    }

    pub(crate) fn clicked_at(&self) -> Option<(BranchId, Hour)> {
        self.click_at
    }

    /// What can be clicked at the view from now on: the click reader declares its tables, letting
    /// a family it can no longer click go, with the learned tables only a family this view cannot
    /// draw names. A click that reads one again learns it again and holds it until the next change.
    pub(crate) fn clickable(&mut self, clickable: Clickable) {
        if clickable != self.clickable {
            self.clickable = clickable;
            let hidden = hidden_families(clickable.shown);
            self.learned.retain(|table| !hidden.contains(table));
            self.declare_clicks();
        }
    }

    fn declare_clicks(&mut self) {
        let needs: Vec<Need> = click_tables(self.clickable, &self.learned).into_iter().map(Need::every).collect();
        self.clicks.declare(&needs);
    }

    /// A click or menu of the overlay's map mode, over its inputs' rows of the hour it was made
    /// at ([`Self::click_at`]); today's frame with no open map. None while that hour's rows are not
    /// read.
    pub(crate) fn clicks<T>(&mut self, head: Head<'_>, today: Option<&DecodedFrame>, mut read: impl FnMut(&DecodedFrame) -> T) -> Option<T> {
        let Some(views) = head else { return today.map(read) };
        let (branch, hour) = self.click_at?;
        let rows = self.clicks.frame_of(views, hour).filter(|rows| rows.branch == branch)?;
        let (value, missed) = replay_frame::frame::discovering_fields(|| read(&rows));
        if missed.is_empty() {
            return Some(value);
        }
        // Learned whole, the set is the one any other reader of the whole table reads.
        for (table, _) in missed {
            if !self.learned.contains(&table) {
                self.learned.push(table);
            }
        }
        self.declare_clicks();
        None
    }

    /// Once a frame, after the last ask: a reader not asked this frame closes and retires its
    /// tables. The clicks' stays open while `head`'s map is, so a click's hour is read as it comes.
    pub(crate) fn end_frame(&mut self, head: Head<'_>) {
        if let Some(views) = head {
            self.clicks.keep(views);
        }
        for at in [&mut self.units, &mut self.arrows, &mut self.air, &mut self.fleets, &mut self.walk, &mut self.map, &mut self.cursor, &mut self.order, &mut self.clicks, &mut self.find] {
            at.end_frame();
        }
    }
}
