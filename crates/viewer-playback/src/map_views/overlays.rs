use super::CounterView;
use viewer_foundation::map::mode_visibility::PriorityTable;

/// What the overlay's camera-bound inputs read for: each such set has a reader of its own, so a
/// map mode declares only what it draws and holds nothing once left.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OverlayInputs {
    pub priorities: PriorityTable,
    pub resources: bool,
    pub occupation: bool,
    pub air: bool,
    /// The whole overlay, its tables and layout, is made from these rows: the open map's.
    pub tables: bool,
}

impl OverlayInputs {
    /// None when no input that reads rows is drawn.
    pub fn of(view: &CounterView) -> Option<Self> {
        let priorities = view.visibility.priorities;
        Self { priorities, resources: view.visibility.resource_icons, occupation: view.occupation.is_some(), air: !view.air_selected.is_empty(), tables: false }.drawn()
    }

    /// The open map's overlay: its tables and layout as well as its camera-bound inputs, so it is
    /// always read.
    pub fn whole(view: &CounterView) -> Self {
        let priorities = view.visibility.priorities;
        Self { priorities, resources: view.visibility.resource_icons, occupation: view.occupation.is_some(), air: !view.air_selected.is_empty(), tables: true }
    }

    /// These inputs less the resource icons or the occupation icons, which the map's layers draw.
    pub fn without(self, resources: bool, occupation: bool) -> Option<Self> {
        Self { resources: self.resources && !resources, occupation: self.occupation && !occupation, ..self }.drawn()
    }

    fn drawn(self) -> Option<Self> {
        let mode = matches!(self.priorities, PriorityTable::StrategicAir | PriorityTable::Resistance | PriorityTable::StrategicNavy);
        (self.tables || mode || self.resources || self.occupation || self.air).then_some(self)
    }
}
