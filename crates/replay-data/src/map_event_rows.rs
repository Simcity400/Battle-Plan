//! Map events: the simulation edges behind the game's floating map text and
//! the critical shore bombardment explosion, recorded as one hourly interval.
//!
//! [`MapEventsRow`] is a singleton (key 0) holding the events the collector
//! published since the previous interval, in their one global sequence, and
//! each family's coverage of the interval. A family that is complete with no
//! events certifies that none happened; an incomplete one is unknown, though
//! the events it does list happened. Each event is captured where the game
//! decides it, before the game's own local-viewpoint filter, so a replay can
//! show what any country would have seen.
use crate::{
    codec::{Decoder, invalid_static as invalid},
    value::{Value, value_struct},
};
use engine::allocation;
use std::io;

/// The event families, in the order of [`MapEventsRow::health`] and their payload tags.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum MapEventFamily {
    #[default]
    FakeDivisionRevealed = 0,
    RailwayGunOverrun = 1,
    CriticalShoreBombardment = 2,
    GeneralCaptured = 3,
    ParadropLanding = 4,
    ResistanceActivity = 5,
    BuildingDamage = 6,
}

impl MapEventFamily {
    pub const COUNT: usize = 7;
    pub const ALL: [Self; Self::COUNT] = [
        Self::FakeDivisionRevealed,
        Self::RailwayGunOverrun,
        Self::CriticalShoreBombardment,
        Self::GeneralCaptured,
        Self::ParadropLanding,
        Self::ResistanceActivity,
        Self::BuildingDamage,
    ];

    pub const fn index(self) -> usize {
        self as usize
    }

    pub fn from_index(index: u8) -> io::Result<Self> {
        Self::ALL
            .get(usize::from(index))
            .copied()
            .ok_or_else(|| invalid("unknown map event family"))
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::FakeDivisionRevealed => "fake_division_revealed",
            Self::RailwayGunOverrun => "railway_gun_overrun",
            Self::CriticalShoreBombardment => "critical_shore_bombardment",
            Self::GeneralCaptured => "general_captured",
            Self::ParadropLanding => "paradrop_landing",
            Self::ResistanceActivity => "resistance_activity",
            Self::BuildingDamage => "building_damage",
        }
    }
}

/// Why a family's interval is not complete. Each is a bit of [`MapEventFaults`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum MapEventFault {
    /// The family's hook is not installed.
    Unavailable = 0,
    /// The family was not enabled for the whole interval.
    Disabled = 1,
    /// Observation began inside the interval: attach, a world reset or a load.
    ObservationStart = 2,
    /// An event's native operands could not be read.
    Read = 3,
    /// An event did not fit its publication lane.
    Capacity = 4,
    /// A native exception unwound through the family's callback.
    Unwind = 5,
    /// An earlier interval's events were lost before they were written.
    OutputLost = 6,
    /// An event was still being published when the interval closed; it is
    /// listed by a later interval.
    InFlight = 7,
}

impl MapEventFault {
    pub const ALL: [Self; 8] = [
        Self::Unavailable,
        Self::Disabled,
        Self::ObservationStart,
        Self::Read,
        Self::Capacity,
        Self::Unwind,
        Self::OutputLost,
        Self::InFlight,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Unavailable => "unavailable",
            Self::Disabled => "disabled",
            Self::ObservationStart => "observation start",
            Self::Read => "read",
            Self::Capacity => "capacity",
            Self::Unwind => "unwind",
            Self::OutputLost => "output lost",
            Self::InFlight => "in flight",
        }
    }
}

/// A closed set of [`MapEventFault`] bits; unknown bits are malformed.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct MapEventFaults(u16);

impl MapEventFaults {
    const KNOWN: u16 = (1 << MapEventFault::ALL.len()) - 1;

    pub fn from_bits(bits: u16) -> io::Result<Self> {
        if bits & !Self::KNOWN != 0 {
            return Err(invalid("map event faults have unknown bits"));
        }
        Ok(Self(bits))
    }

    pub const fn bits(self) -> u16 {
        self.0
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub const fn contains(self, fault: MapEventFault) -> bool {
        self.0 & (1 << fault as u8) != 0
    }

    pub fn insert(&mut self, fault: MapEventFault) {
        self.0 |= 1 << fault as u8;
    }

    pub fn iter(self) -> impl Iterator<Item = MapEventFault> {
        MapEventFault::ALL
            .into_iter()
            .filter(move |fault| self.contains(*fault))
    }
}

impl Value for MapEventFaults {
    const MINIMUM_ENCODED_BYTES: usize = 2;
    fn encoded_bytes(&self) -> io::Result<usize> {
        self.0.encoded_bytes()
    }
    fn try_clone(&self) -> io::Result<Self> {
        Ok(*self)
    }
    fn encode_reserved(&self, output: &mut Vec<u8>) -> io::Result<()> {
        self.0.encode_reserved(output)
    }
    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        Self::from_bits(u16::decode(input)?)
    }
}

/// One family's coverage of the interval. `missed` counts events the family
/// refused (unreadable or without room) since its epoch began.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct MapEventFamilyHealth {
    pub installed: bool,
    pub enabled: bool,
    pub complete: bool,
    pub faults: MapEventFaults,
    pub missed: u64,
}

impl MapEventFamilyHealth {
    pub fn validate(&self) -> io::Result<()> {
        if (self.enabled && !self.installed)
            || (self.complete && (!self.enabled || !self.faults.is_empty()))
        {
            return Err(invalid("map event family has inconsistent coverage"));
        }
        Ok(())
    }
}

impl Value for MapEventFamilyHealth {
    const MINIMUM_ENCODED_BYTES: usize = 13;
    fn encoded_bytes(&self) -> io::Result<usize> {
        Ok(Self::MINIMUM_ENCODED_BYTES)
    }
    fn try_clone(&self) -> io::Result<Self> {
        self.validate()?;
        Ok(*self)
    }
    fn encode_reserved(&self, output: &mut Vec<u8>) -> io::Result<()> {
        self.validate()?;
        self.installed.encode_reserved(output)?;
        self.enabled.encode_reserved(output)?;
        self.complete.encode_reserved(output)?;
        self.faults.encode_reserved(output)?;
        self.missed.encode_reserved(output)
    }
    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        let health = Self {
            installed: bool::decode(input)?,
            enabled: bool::decode(input)?,
            complete: bool::decode(input)?,
            faults: MapEventFaults::decode(input)?,
            missed: u64::decode(input)?,
        };
        health.validate()?;
        Ok(health)
    }
}

/// One entry of a unit's intel array: a country's canonical ordinal and its
/// signed level, in native order with repetitions kept.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct MapEventIntelEntry {
    pub country: u32,
    pub level: i8,
}
value_struct!(MapEventIntelEntry {
    country: u32,
    level: i8
});

/// A revealed fake division or an overrun railway gun: the unit's raw owner
/// reference, the province the text stands on, the province the game reads
/// a viewer's visibility at, the unit's intel array and whether fog of war
/// was on.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct MapEventUnitIntel {
    pub owner: u32,
    pub anchor_province: u32,
    pub visibility_province: u32,
    pub fog: bool,
    pub intel: Vec<MapEventIntelEntry>,
}
value_struct!(MapEventUnitIntel {
    owner: u32,
    anchor_province: u32,
    visibility_province: u32,
    fog: bool,
    intel: Vec<MapEventIntelEntry>,
});

/// A critical shore bombardment hit: the bombarded province. The game shows
/// its text and its explosion entity to every viewer.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct MapEventCriticalBombardment {
    pub province: u32,
}
value_struct!(MapEventCriticalBombardment { province: u32 });

/// A captured general: the general's country and the captor, as raw
/// references, and the province the text stands on.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct MapEventGeneralCapture {
    pub owner: u32,
    pub captor: u32,
    pub province: u32,
}
value_struct!(MapEventGeneralCapture {
    owner: u32,
    captor: u32,
    province: u32
});

/// A completed paradrop: the landing province, the dropping country's raw
/// reference, whether the landing took losses and whether fog of war was on.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct MapEventParadrop {
    pub province: u32,
    pub country: u32,
    pub lost: bool,
    pub fog: bool,
}
value_struct!(MapEventParadrop {
    province: u32,
    country: u32,
    lost: bool,
    fog: bool
});

/// An executed resistance activity: the state's best victory-point province,
/// its controller and its owner's overlord as raw references, and the
/// activity's authored map text key.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct MapEventResistance {
    pub province: u32,
    pub controller: u32,
    pub overlord: u32,
    pub key: Vec<u8>,
}
value_struct!(MapEventResistance {
    province: u32,
    controller: u32,
    overlord: u32,
    key: Vec<u8>,
});

/// The reason a building notification gives, which selects its text.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u32)]
pub enum MapEventBuildingReason {
    #[default]
    Bombed = 1,
    Sabotaged = 2,
    Collateral = 3,
    Repaired = 4,
    Scorched = 5,
}

impl MapEventBuildingReason {
    pub const ALL: [Self; 5] = [
        Self::Bombed,
        Self::Sabotaged,
        Self::Collateral,
        Self::Repaired,
        Self::Scorched,
    ];

    pub fn from_native(reason: u32) -> Option<Self> {
        Self::ALL.into_iter().find(|known| *known as u32 == reason)
    }
}

impl Value for MapEventBuildingReason {
    const MINIMUM_ENCODED_BYTES: usize = 1;
    fn encoded_bytes(&self) -> io::Result<usize> {
        Ok(1)
    }
    fn try_clone(&self) -> io::Result<Self> {
        Ok(*self)
    }
    fn encode_reserved(&self, output: &mut Vec<u8>) -> io::Result<()> {
        (*self as u8).encode_reserved(output)
    }
    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        Self::from_native(u32::from(u8::decode(input)?))
            .ok_or_else(|| invalid("unknown map event building reason"))
    }
}

/// One building level whose health a notification lowered: the building
/// type's lexical token, the province the text stands on, that province's
/// controller as a raw reference and the notification's reason. A
/// notification that lowers several levels is one event a level.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct MapEventBuildingDamage {
    pub building_type: u32,
    pub province: u32,
    pub controller: u32,
    pub reason: MapEventBuildingReason,
}
value_struct!(MapEventBuildingDamage {
    building_type: u32,
    province: u32,
    controller: u32,
    reason: MapEventBuildingReason
});

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MapEventPayload {
    FakeDivisionRevealed(MapEventUnitIntel),
    RailwayGunOverrun(MapEventUnitIntel),
    CriticalShoreBombardment(MapEventCriticalBombardment),
    GeneralCaptured(MapEventGeneralCapture),
    ParadropLanding(MapEventParadrop),
    ResistanceActivity(MapEventResistance),
    BuildingDamage(MapEventBuildingDamage),
}

impl Default for MapEventPayload {
    fn default() -> Self {
        Self::CriticalShoreBombardment(MapEventCriticalBombardment::default())
    }
}

impl MapEventPayload {
    pub const fn family(&self) -> MapEventFamily {
        match self {
            Self::FakeDivisionRevealed(_) => MapEventFamily::FakeDivisionRevealed,
            Self::RailwayGunOverrun(_) => MapEventFamily::RailwayGunOverrun,
            Self::CriticalShoreBombardment(_) => MapEventFamily::CriticalShoreBombardment,
            Self::GeneralCaptured(_) => MapEventFamily::GeneralCaptured,
            Self::ParadropLanding(_) => MapEventFamily::ParadropLanding,
            Self::ResistanceActivity(_) => MapEventFamily::ResistanceActivity,
            Self::BuildingDamage(_) => MapEventFamily::BuildingDamage,
        }
    }
}

impl Value for MapEventPayload {
    const MINIMUM_ENCODED_BYTES: usize = 5;
    fn encoded_bytes(&self) -> io::Result<usize> {
        let bytes = match self {
            Self::FakeDivisionRevealed(value) | Self::RailwayGunOverrun(value) => {
                value.encoded_bytes()
            }
            Self::CriticalShoreBombardment(value) => value.encoded_bytes(),
            Self::GeneralCaptured(value) => value.encoded_bytes(),
            Self::ParadropLanding(value) => value.encoded_bytes(),
            Self::ResistanceActivity(value) => value.encoded_bytes(),
            Self::BuildingDamage(value) => value.encoded_bytes(),
        }?;
        allocation::add(1, bytes)
    }
    fn try_clone(&self) -> io::Result<Self> {
        Ok(match self {
            Self::FakeDivisionRevealed(value) => Self::FakeDivisionRevealed(value.try_clone()?),
            Self::RailwayGunOverrun(value) => Self::RailwayGunOverrun(value.try_clone()?),
            Self::CriticalShoreBombardment(value) => Self::CriticalShoreBombardment(*value),
            Self::GeneralCaptured(value) => Self::GeneralCaptured(*value),
            Self::ParadropLanding(value) => Self::ParadropLanding(*value),
            Self::ResistanceActivity(value) => Self::ResistanceActivity(value.try_clone()?),
            Self::BuildingDamage(value) => Self::BuildingDamage(*value),
        })
    }
    fn encode_reserved(&self, output: &mut Vec<u8>) -> io::Result<()> {
        (self.family() as u8).encode_reserved(output)?;
        match self {
            Self::FakeDivisionRevealed(value) | Self::RailwayGunOverrun(value) => {
                value.encode_reserved(output)
            }
            Self::CriticalShoreBombardment(value) => value.encode_reserved(output),
            Self::GeneralCaptured(value) => value.encode_reserved(output),
            Self::ParadropLanding(value) => value.encode_reserved(output),
            Self::ResistanceActivity(value) => value.encode_reserved(output),
            Self::BuildingDamage(value) => value.encode_reserved(output),
        }
    }
    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        Ok(match MapEventFamily::from_index(u8::decode(input)?)? {
            MapEventFamily::FakeDivisionRevealed => {
                Self::FakeDivisionRevealed(MapEventUnitIntel::decode(input)?)
            }
            MapEventFamily::RailwayGunOverrun => {
                Self::RailwayGunOverrun(MapEventUnitIntel::decode(input)?)
            }
            MapEventFamily::CriticalShoreBombardment => {
                Self::CriticalShoreBombardment(MapEventCriticalBombardment::decode(input)?)
            }
            MapEventFamily::GeneralCaptured => {
                Self::GeneralCaptured(MapEventGeneralCapture::decode(input)?)
            }
            MapEventFamily::ParadropLanding => {
                Self::ParadropLanding(MapEventParadrop::decode(input)?)
            }
            MapEventFamily::ResistanceActivity => {
                Self::ResistanceActivity(MapEventResistance::decode(input)?)
            }
            MapEventFamily::BuildingDamage => {
                Self::BuildingDamage(MapEventBuildingDamage::decode(input)?)
            }
        })
    }
}

/// One event: its place in the global sequence, the game hour it happened
/// at and what happened.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct MapEvent {
    pub sequence: u64,
    pub hour: i32,
    pub payload: MapEventPayload,
}
value_struct!(MapEvent {
    sequence: u64,
    hour: i32,
    payload: MapEventPayload
});

/// The interval since the previous row. `epoch` changes when observation
/// starts again (attach, world reset, load). The interval covers the game
/// hours after `from_hour` through `to_hour` and the sequences from
/// `first_sequence` up to `next_sequence`. A row whose `first_sequence` is
/// the previous row's `next_sequence` in the same epoch continues it; any
/// other row starts with unknown coverage. Events are in sequence order;
/// one an earlier interval left in flight can precede `first_sequence`.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct MapEventsRow {
    pub epoch: u64,
    pub from_hour: i32,
    pub to_hour: i32,
    pub first_sequence: u64,
    pub next_sequence: u64,
    pub health: [MapEventFamilyHealth; MapEventFamily::COUNT],
    pub events: Vec<MapEvent>,
}

impl MapEventsRow {
    pub fn validate(&self) -> io::Result<()> {
        if self.from_hour > self.to_hour || self.first_sequence > self.next_sequence {
            return Err(invalid("map event interval boundaries reversed"));
        }
        for health in &self.health {
            health.validate()?;
        }
        let mut previous = None;
        for event in &self.events {
            if event.sequence == 0
                || event.sequence >= self.next_sequence
                || previous.is_some_and(|sequence| event.sequence <= sequence)
            {
                return Err(invalid("map event outside its interval or out of order"));
            }
            previous = Some(event.sequence);
        }
        Ok(())
    }

    pub fn health(&self, family: MapEventFamily) -> MapEventFamilyHealth {
        self.health[family.index()]
    }

    /// Whether this row continues `previous` without a gap in sequence or epoch.
    pub fn continues(&self, previous: &Self) -> bool {
        self.epoch == previous.epoch
            && self.first_sequence == previous.next_sequence
            && self.from_hour == previous.to_hour
    }
}
