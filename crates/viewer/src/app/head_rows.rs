//! The window's readers of the hour besides its tooltips and panels: the selection observers and a
//! selection's movement arrows, the overlay (its tables, its layout and the clicks and menu of its
//! map modes), the walk camera's figure, the clock's speed and the picker's fourth button. Each
//! reads rows at the head while the map's playhead is open, and nothing else: while those rows do
//! not hold what it reads it keeps what it gave and is loading by name. Today's frame is read only
//! with no open playhead map. Each is open only while it is asked: a selection's while something
//! is selected, the walk camera's while walking; the overlay's, the clock's and the picker's every
//! frame the open map draws.

use crate::{
    document::Document,
    map_views::CounterView,
    playhead_host::{
        Entry, MapSide,
        rows::{AtHead, Cadence, Need, Recording},
    },
};
use replay_data::{Hour, rows::TableId};
use replay_frame::frame::DecodedFrame;
use std::sync::Arc;
use viewer_foundation::map::mode_visibility::PriorityTable;

pub(crate) mod tables;
#[cfg(test)]
mod tests;

/// The map's playhead and the recording facts its rows carry; none while it is off.
pub(crate) type Head<'a> = Option<(&'a Arc<Entry>, Recording)>;

/// What a person sees change on the map: a selection, an icon, the clock's speed; at the step.
const CADENCE: Cadence = Cadence::Ready;

/// The tables the open map's overlay is known to read, besides the vision's
/// ([`viewer_foundation::map::perspective::VISION_TABLES`]): declared when its reader opens, so its
/// first rows hold them and its first build is made at once. A table a script reads beyond them is
/// learned.
const OVERLAY: &[TableId] = &[
    // The land stacks, the railway guns and their combat markers.
    TableId::Divisions,
    TableId::DivisionMovement,
    TableId::DivisionStatus,
    TableId::DivisionPanelStats,
    TableId::DivisionModelInputs,
    TableId::DivisionTemplateIcons,
    TableId::DivisionTransportSpeeds,
    TableId::ArmyIconInputs,
    TableId::ActiveAbilities,
    TableId::UnitLeaders,
    TableId::LeaderPortraits,
    TableId::CharacterPortraits,
    TableId::OrdersGroups,
    TableId::OrdersGroupPlans,
    TableId::RailCooldowns,
    TableId::RailwayGunCounterInputs,
    TableId::ProvinceUnitOrder,
    TableId::LandCombats,
    TableId::LandCombatDivisions,
    TableId::LandCombatSides,
    TableId::LandCombatDetails,
    // The fleets, their missions, mines and naval combats and results.
    TableId::Ships,
    TableId::ShipModels,
    TableId::TaskForces,
    TableId::TaskForceCounterInputs,
    TableId::TaskForceMissions,
    TableId::Fleets,
    TableId::FleetCounterInputs,
    TableId::NavalBases,
    TableId::NavalMissionLedgers,
    TableId::NavalMissionTaskForces,
    TableId::NavalMissionCommanders,
    TableId::NavalMissionCountries,
    TableId::NavalDominanceTaskForceRanges,
    TableId::SliceReads,
    TableId::ModifierDefinitions,
    TableId::NavalCombats,
    TableId::NavalCombatDetails,
    TableId::NavalCombatShips,
    TableId::NavalCombatTransfers,
    TableId::NavalProvinceCombats,
    TableId::NavalResults,
    TableId::NavalResultMembers,
    TableId::NavalResultCountries,
    TableId::NavalTransferUnits,
    TableId::NavalAccidentReports,
    TableId::Capitulation,
    // The air bases and wings, their missions, ranges and orders, and the volunteers.
    TableId::AirBases,
    TableId::AirBaseRegistrations,
    TableId::AirWings,
    TableId::AirReferenceWitnesses,
    TableId::AirRegionalMissions,
    TableId::AirMissionInputs,
    TableId::AirGroundCrews,
    TableId::AirRegionCombat,
    TableId::AirCountryRelations,
    TableId::WingEquipment,
    TableId::VariantDefinitions,
    TableId::AceRangeInputs,
    TableId::RaidAirSites,
    TableId::RaidAirScores,
    TableId::RaidCountryInputs,
    TableId::VolunteerHosts,
    TableId::SentVolunteers,
    // The base windows, the faction pings, the weather the missions read and the radar.
    TableId::ProvinceBuildings,
    TableId::FacilityMapFlags,
    TableId::StateCores,
    TableId::StateFlags,
    TableId::FactionTheatres,
    TableId::WeatherProvinces,
    TableId::WeatherRegions,
    TableId::RadarCoverage,
    // The countries every counter names, tints and flags, and the occupation icons.
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CountryAppearance,
    TableId::CountryCounterInputs,
    TableId::FactionMembers,
    TableId::LexicalTokens,
    TableId::Politics,
    TableId::PoliticalParties,
    TableId::ReservedCountryNaming,
    TableId::Subjects,
    TableId::WarRelations,
    TableId::SimulationFeatures,
    TableId::Provinces,
    TableId::States,
    TableId::StateResistance,
    TableId::Autonomy,
    // The raid arrows.
    TableId::Raids,
    TableId::RaidTargets,
    TableId::RaidEquipment,
    TableId::RaidSpeedInputs,
    TableId::AvailableEquipmentVariants,
    TableId::CountryCommandPower,
    // The intel icons.
    TableId::IntelligenceAgencies,
    TableId::IntelNetworks,
    TableId::IntelNetworkAuxiliaries,
    TableId::Operations,
    TableId::OperativeMissions,
    TableId::StrategicOperatives,
    TableId::CharacterOperatives,
    TableId::AgencyOperationBuffers,
    TableId::RaidIntelInputs,
    TableId::CountryStabilityInputs,
    // What the default view's build was found to read beyond them in every paced run: the counters'
    // orders and portraits, the sorties' scripts, the movement times and the naval repairs.
    TableId::AirSortieBaseCountries,
    TableId::AirSortieModifierMetadata,
    TableId::AirSortieModifiers,
    TableId::AirSortieSpecialBases,
    TableId::ArmyOrderPlans,
    TableId::ArmyOrders,
    TableId::Characters,
    TableId::CoreSupplyAccess,
    TableId::CountryUnitSpeedEntries,
    TableId::GlobalSavedEventTargets,
    TableId::LeaderPortraitPaths,
    TableId::MovementStateTerms,
    TableId::NavalRepairBases,
    TableId::NavalRepairQueue,
    TableId::OrdersGroupCounterInputs,
    TableId::RailEdges,
    TableId::RiverRelations,
    TableId::ScriptVariables,
    TableId::TaskForceMapMotion,
];

/// The tables the open map's overlay waited for when the list last changed, and whether the paced
/// report has taken it since ([`note_waiting`], [`take_waiting`]).
static WAITING: std::sync::Mutex<(Vec<TableId>, bool)> = std::sync::Mutex::new((Vec::new(), true));

/// The tables the overlay waits for this frame, by name, while frames are measured; kept only
/// when the list changed.
pub(crate) fn note_waiting(mut tables: Vec<TableId>) {
    if !replay_frame::cpu::enabled() {
        return;
    }
    tables.sort_unstable_by_key(|table| table.index());
    tables.dedup();
    let mut held = WAITING.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if held.0 != tables {
        *held = (tables, false);
    }
}

/// The overlay's tables waited for, when the list changed since the last call.
pub(crate) fn take_waiting() -> Option<Vec<TableId>> {
    let mut held = WAITING.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    (!std::mem::replace(&mut held.1, true)).then(|| held.0.clone())
}

/// The tables the open map's overlay reader opens with: [`OVERLAY`] and the vision's, ascending.
fn overlay_tables() -> Vec<TableId> {
    let mut tables: Vec<TableId> = OVERLAY.iter().chain(viewer_foundation::map::perspective::VISION_TABLES).copied().collect();
    tables.sort_unstable_by_key(|table| table.index());
    tables.dedup();
    tables
}

/// The reader's rows stamped with its branch, which follows the clock's; the document's only for
/// a reader opened without one.
pub(crate) fn head<'a>(side: Option<&'a MapSide>, document: &Document) -> Head<'a> {
    side.map(|side| (side.entry(), Recording { branch: side.branch().unwrap_or_else(|| document.playback.branch()) }))
}

/// What the overlay's camera-bound inputs read for: each such set has a reader of its own, so a
/// map mode declares only what it draws and holds nothing once left.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct OverlayInputs {
    priorities: PriorityTable,
    resources: bool,
    occupation: bool,
    air: bool,
    /// The whole overlay, its tables and layout, is made from these rows: the open map's.
    tables: bool,
}

impl OverlayInputs {
    /// None when no input that reads rows is drawn.
    pub(crate) fn of(view: &CounterView) -> Option<Self> {
        let priorities = view.visibility.priorities;
        Self { priorities, resources: view.visibility.resource_icons, occupation: view.occupation.is_some(), air: !view.air_selected.is_empty(), tables: false }.drawn()
    }

    /// The open map's overlay: its tables and layout as well as its camera-bound inputs, so it is
    /// always read.
    pub(crate) fn whole(view: &CounterView) -> Self {
        let priorities = view.visibility.priorities;
        Self { priorities, resources: view.visibility.resource_icons, occupation: view.occupation.is_some(), air: !view.air_selected.is_empty(), tables: true }
    }

    /// These inputs less the resource icons or the occupation icons, which the map's layers draw.
    pub(crate) fn without(self, resources: bool, occupation: bool) -> Option<Self> {
        Self { resources: self.resources && !resources, occupation: self.occupation && !occupation, ..self }.drawn()
    }

    fn drawn(self) -> Option<Self> {
        let mode = matches!(self.priorities, PriorityTable::StrategicAir | PriorityTable::Resistance | PriorityTable::StrategicNavy);
        (self.tables || mode || self.resources || self.occupation || self.air).then_some(self)
    }
}

#[derive(Debug)]
pub(crate) struct HeadRows {
    /// The observers ask only whether a key lives: they hold keys, no column.
    units: AtHead,
    arrows: AtHead,
    air: AtHead,
    fleets: AtHead,
    overlays: Vec<(OverlayInputs, AtHead)>,
    /// The overlay's inputs this frame, and the rows they read.
    overlay: Option<OverlayInputs>,
    /// The overlay's reader opened with the map ([`Self::open_overlay`]), kept open until the
    /// overlay first asks for its rows.
    opened: Option<OverlayInputs>,
    /// Its owner says when the overlay has finished with its rows ([`Self::overlay_ready`]); until
    /// it first does, the overlay's reader is ready each frame it looks.
    answers: bool,
    shown: Option<Arc<DecodedFrame>>,
    walk: AtHead,
    map: AtHead,
    /// The fourth button's answer at the head, kept while the rows load.
    four: Option<bool>,
}

impl Default for HeadRows {
    fn default() -> Self {
        Self {
            units: AtHead::with(&[Need::keys(TableId::Divisions), Need::keys(TableId::RailwayGunCounterInputs), Need::keys(TableId::TaskForces)], CADENCE),
            arrows: AtHead::new(&[], CADENCE),
            air: AtHead::with(&[Need::keys(TableId::AirWings), Need::keys(TableId::AirBaseRegistrations)], CADENCE),
            fleets: AtHead::with(&[Need::keys(TableId::Fleets)], CADENCE),
            overlays: Vec::new(),
            overlay: None,
            opened: None,
            answers: false,
            shown: None,
            walk: AtHead::new(&[], CADENCE),
            map: AtHead::new(&[TableId::Clock, TableId::SimulationFeatures], CADENCE),
            four: None,
        }
    }
}

/// `read` over the rows at the head of `head`'s playhead when it is open, learning the tables it
/// reads; none while those rows load. Over `today` only with no open map.
fn read_at<T>(at: &mut AtHead, head: Head<'_>, today: Option<&DecodedFrame>, mut read: impl FnMut(&DecodedFrame) -> T) -> Option<T> {
    match head {
        Some((entry, recording)) => {
            let rows = at.frame(entry, recording)?;
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
        let (entry, recording) = head?;
        self.walk.frame(entry, recording).map(|rows| rows.hour)
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

    /// The rows the overlay's inputs read this frame; asked once a frame. None while the playhead
    /// is off, while they load, or when `inputs` read none.
    pub(crate) fn overlay(&mut self, inputs: Option<OverlayInputs>, head: Head<'_>) -> Option<Arc<DecodedFrame>> {
        (self.overlay, self.opened) = (inputs, None);
        let answers = self.answers;
        self.shown = match (inputs, head) {
            (Some(inputs), Some((entry, recording))) => {
                let reader = self.overlay_reader(inputs);
                if !answers {
                    reader.ready();
                }
                reader.frame(entry, recording)
            }
            _ => None,
        };
        self.shown.clone()
    }

    /// The overlay has finished with the rows it took last (its tables job answered, or the head
    /// held): its reader is made its next frame at the next step that changes them, and none
    /// before. The first call hands this to its caller: from then on [`Self::overlay`] only takes
    /// the newest frame.
    pub(crate) fn overlay_ready(&mut self) {
        self.answers = true;
        if let Some((_, at)) = self.overlay.and_then(|inputs| self.overlays.iter().find(|(held, _)| *held == inputs)) {
            at.ready();
        }
    }

    /// Opens the overlay's reader of `inputs` on `head`'s playhead when the map is opened or sought,
    /// before its first frame is drawn: its tables are declared with the map's own, so the seek
    /// brings them and its first rows come with the map's first picture. It stays open until the
    /// overlay first asks for its rows ([`Self::overlay`]); nothing without a playhead.
    pub(crate) fn open_overlay(&mut self, inputs: OverlayInputs, head: Head<'_>) {
        let Some((entry, recording)) = head else { return };
        let reader = self.overlay_reader(inputs);
        reader.frame(entry, recording);
        // A map opened or sought wants its first rows whatever the overlay last took.
        reader.ready();
        (self.overlay, self.opened) = (Some(inputs), Some(inputs));
    }

    /// The overlay's rows at `hour` for a frame that is saved, one frame an hour (a stepped run, a
    /// window dump): `make` over them until it reads no table they lack, each table it read
    /// learned and the reader waited for in between, all within `limit`; refused by name.
    pub(crate) fn overlay_settled<T>(&mut self, inputs: OverlayInputs, head: Head<'_>, hour: Hour, limit: std::time::Duration, mut make: impl FnMut(&Arc<DecodedFrame>) -> (T, Vec<TableId>)) -> std::io::Result<(Arc<DecodedFrame>, T)> {
        let (entry, recording) = head.ok_or_else(|| std::io::Error::other("The overlay's rows at the head: the playhead's map is not open"))?;
        let until = std::time::Instant::now() + limit;
        self.overlay = Some(inputs);
        loop {
            let reader = self.overlay_reader(inputs);
            reader.frame(entry, recording);
            reader.ready();
            let left = until.saturating_duration_since(std::time::Instant::now());
            let rows = reader.wait(|frame| frame.hour == hour, left).map_err(|error| std::io::Error::other(format!("The overlay's rows at the head did not reach hour {hour}: {error}")))?;
            let (value, missed) = make(&rows);
            if missed.is_empty() {
                self.shown = Some(Arc::clone(&rows));
                return Ok((rows, value));
            }
            if std::time::Instant::now() >= until {
                return Err(std::io::Error::other(format!("The overlay at hour {hour} still reads tables its rows at the head do not hold: {missed:?}")));
            }
            self.learn(&missed);
        }
    }

    /// The rows the overlay's inputs read this frame.
    pub(crate) fn shown(&self) -> Option<Arc<DecodedFrame>> {
        self.shown.clone()
    }

    /// The hour the overlay's rows `frame` stand at while they are its reader's newest
    /// ([`crate::playhead_host::rows::RowReader::through`]).
    pub(crate) fn overlay_through(&self, frame: *const DecodedFrame) -> Option<Hour> {
        self.overlay.and_then(|inputs| self.overlays.iter().find(|(held, _)| *held == inputs)).and_then(|(_, at)| at.through(frame))
    }

    /// The tables the overlay's reader declared whose rows have not arrived.
    pub(crate) fn overlay_arriving(&self) -> Vec<TableId> {
        self.overlay.and_then(|inputs| self.overlays.iter().find(|(held, _)| *held == inputs)).map_or_else(Vec::new, |(_, at)| at.arriving())
    }

    /// Declares the tables the overlay's inputs read that their rows did not hold.
    pub(crate) fn learn(&mut self, missed: &[TableId]) {
        if let Some(inputs) = self.overlay
            && !missed.is_empty()
        {
            self.overlay_reader(inputs).learn(missed);
        }
    }

    /// `read` over `rows`, the overlay's this frame or the ones it was laid out from, on this
    /// thread; none when it read a table they do not hold, which the overlay's reader then learns.
    pub(crate) fn read_overlay<T>(&mut self, rows: &DecodedFrame, read: impl FnOnce(&DecodedFrame) -> T) -> Option<T> {
        let (value, missed) = replay_frame::frame::discovering(|| read(rows));
        if missed.is_empty() {
            return Some(value);
        }
        self.learn(&missed);
        None
    }

    /// A click or menu of the overlay's map mode, over its inputs' rows; today's frame with no open
    /// map. None while the rows load.
    pub(crate) fn clicks<T>(&mut self, head: Head<'_>, today: Option<&DecodedFrame>, read: impl FnMut(&DecodedFrame) -> T) -> Option<T> {
        match (self.overlay, head) {
            (Some(inputs), Some(_)) => read_at(self.overlay_reader(inputs), head, today, read),
            (None, Some(_)) => None,
            (_, None) => today.map(read),
        }
    }

    fn overlay_reader(&mut self, inputs: OverlayInputs) -> &mut AtHead {
        let at = match self.overlays.iter().position(|(held, _)| *held == inputs) {
            Some(at) => at,
            None => {
                let declared = if inputs.tables { overlay_tables() } else { Vec::new() };
                self.overlays.push((inputs, AtHead::new(&declared, CADENCE).ready_by_owner()));
                self.overlays.len() - 1
            }
        };
        &mut self.overlays[at].1
    }

    /// Once a frame, after the last ask: a reader not asked this frame closes and retires its
    /// tables.
    pub(crate) fn end_frame(&mut self) {
        for at in [&mut self.units, &mut self.arrows, &mut self.air, &mut self.fleets, &mut self.walk, &mut self.map] {
            at.end_frame();
        }
        for (inputs, at) in &mut self.overlays {
            if self.opened != Some(*inputs) {
                at.end_frame();
            }
        }
    }
}
