use super::*;


#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ReservedNearestNavyRow {
    pub slot: u32,
    pub present: bool,
    pub provinces: Option<u32>,
}
typed_codec!(ReservedNearestNavyRow, 32, |row| row.slot as u128, |key| Self { slot: key as u32, ..Default::default() },
 { 0 => (present): Bool, 1 => (provinces): OptionalU32 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ReservedNearestBaseRow {
    pub slot: u32,
    pub province: u32,
    pub base: u32,
}
typed_codec!(ReservedNearestBaseRow, 64, |row| (row.slot as u128) << 32 | row.province as u128, |key| Self { slot: (key >> 32) as u32, province: key as u32, base: 0 },
 { 0 => (base): U32 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct TransportNavyRow {
    pub ordinal: u32,
    pub slot: u32,
    pub present: bool,
    pub region_slots: Option<u32>,
    pub ledgers: bool,
    pub access_modes: Option<Vec<u8>>,
}
typed_codec!(TransportNavyRow, 32, |row| row.ordinal as u128, |key| Self { ordinal: key as u32, ..Default::default() },
 { 0 => (slot): U32, 1 => (present): Bool, 2 => (region_slots): OptionalU32, 3 => (ledgers): Bool, 4 => (access_modes): Opaque }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct TransportLedgerRow {
    pub navy: u32,
    pub region: u32,
    pub occurrences: Option<Vec<u64>>,
}
typed_codec!(TransportLedgerRow, 64, |row| (row.navy as u128) << 32 | row.region as u128, |key| Self { navy: (key >> 32) as u32, region: key as u32, occurrences: None },
 { 0 => (occurrences): Opaque }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct TransportReceiverRow {
    pub receiver: TransportMissionReceiver,
}
typed_codec!(TransportReceiverRow, 64, |row| row.receiver.task_force as u128, |key| Self { receiver: TransportMissionReceiver { task_force: key as u64, ..Default::default() } },
 { 0 => (receiver.mission_owner): OptionalU32, 1 => (receiver.logical_owner): OptionalU32, 2 => (receiver.in_regions): Opaque, 3 => (receiver.combat_references): OptionalI32, 4 => (receiver.combined_detection): OptionalI64, 5 => (receiver.detection_cache): Opaque, 6 => (receiver.received_supply_ratio): OptionalI64, 7 => (receiver.risk): Opaque }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct TransportSearchRegionRow {
    pub ordinal: u32,
    pub region: Option<TransportNavalRegion>,
}
typed_codec!(TransportSearchRegionRow, 32, |row| row.ordinal as u128, |key| Self { ordinal: key as u32, region: None },
 { 0 => (region): Opaque }, |_row| Ok(()));

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub enum SpottingLanes {
    Copied(Vec<i64>),
    CountryIntel,
}

impl Default for SpottingLanes {
    fn default() -> Self {
        Self::Copied(Vec::new())
    }
}

impl Value for SpottingLanes {
    fn heap_bytes(&self) -> usize {
        match self {
            Self::Copied(lanes) => Value::heap_bytes(lanes),
            Self::CountryIntel => 0,
        }
    }
    const MINIMUM_ENCODED_BYTES: usize = 1;
    fn encoded_bytes(&self) -> io::Result<usize> {
        match self {
            Self::Copied(lanes) => allocation::add(1, Value::encoded_bytes(lanes)?),
            Self::CountryIntel => Ok(1),
        }
    }
    fn try_clone(&self) -> io::Result<Self> {
        Ok(match self {
            Self::Copied(lanes) => Self::Copied(Value::try_clone(lanes)?),
            Self::CountryIntel => Self::CountryIntel,
        })
    }
    fn encode_reserved(&self, output: &mut Vec<u8>) -> io::Result<()> {
        match self {
            Self::Copied(lanes) => {
                output.push(0);
                lanes.encode_reserved(output)
            }
            Self::CountryIntel => {
                output.push(1);
                Ok(())
            }
        }
    }
    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        match input.u8()? {
            0 => Vec::<i64>::decode(input).map(Self::Copied),
            1 => Ok(Self::CountryIntel),
            _ => Err(invalid("spotting lane source discriminator invalid")),
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct SpottingNavyIntel {
    pub owner_raw_id: Option<u32>,
    pub owner_slot: Option<u32>,
    pub signed_count: Option<i32>,
    pub lanes: Option<SpottingLanes>,
}
value_struct!(SpottingNavyIntel {
    owner_raw_id: Option<u32>,
    owner_slot: Option<u32>,
    signed_count: Option<i32>,
    lanes: Option<SpottingLanes>,
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct TransportSpottingOwnerRow {
    pub owner_slot: u32,
    pub occurrence: u32,
    pub owner_raw_id: u32,
    pub target_owned_states: Option<i32>,
    pub radar: Option<TransportRadarCounts>,
    pub navy_intel: Option<SpottingNavyIntel>,
}
typed_codec!(TransportSpottingOwnerRow, 64, |row| (row.owner_slot as u128) << 32 | row.occurrence as u128, |key| Self { owner_slot: (key >> 32) as u32, occurrence: key as u32, ..Default::default() },
 { 0 => (owner_raw_id): U32, 1 => (target_owned_states): OptionalI32, 2 => (radar): Opaque, 3 => (navy_intel): Opaque }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct TransportSpottingRegionRow {
    pub ordinal: u32,
    pub region: TransportSpottingRegion,
}
typed_codec!(TransportSpottingRegionRow, 32, |row| row.ordinal as u128, |key| Self { ordinal: key as u32, ..Default::default() },
 { 0 => (region.region_id): U32, 1 => (region.runtime_slot): OptionalI32, 2 => (region.signed_province_count): OptionalI32, 3 => (region.weather_detection): Opaque, 4 => (region.air_environment): Opaque },
 |row| if row.region.sample == SpottingSample::Derived { Ok(()) } else { Err(invalid("split spotting regions derive their sample")) });

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct TransportAirRootRow {
    pub ordinal: u32,
    pub country: Option<u32>,
    pub slot: u32,
    pub present: bool,
    pub regions: Option<u32>,
    pub factors: Option<Vec<Option<i64>>>,
}
typed_codec!(TransportAirRootRow, 32, |row| row.ordinal as u128, |key| Self { ordinal: key as u32, ..Default::default() },
 { 0 => (country): OptionalU32, 1 => (slot): U32, 2 => (present): Bool, 3 => (regions): OptionalU32, 4 => (factors): Opaque }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct TransportAirRegionRow {
    pub root: u32,
    pub ordinal: u32,
    pub region: TransportAirRegion,
}
typed_codec!(TransportAirRegionRow, 64, |row| (row.root as u128) << 32 | row.ordinal as u128, |key| Self { root: (key >> 32) as u32, ordinal: key as u32, ..Default::default() },
 { 0 => (region.region): U32, 1 => (region.count): OptionalI32, 2 => (region.missions): Opaque }, |_row| Ok(()));

impl TransportAirRegionRow {
    pub fn empty(ordinal: u32) -> TransportAirRegion {
        TransportAirRegion {
            region: ordinal,
            count: Some(0),
            missions: Some(Vec::new()),
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct TransportRiskShipsRow {
    pub receiver: TransportReceiverShips,
}
typed_codec!(TransportRiskShipsRow, 64, |row| row.receiver.task_force as u128, |key| Self { receiver: TransportReceiverShips { task_force: key as u64, ..Default::default() } },
 { 0 => (receiver.count): OptionalI32, 1 => (receiver.ships): Opaque }, |_row| Ok(()));

impl OpaqueValue for TransportReceiverRisk {}
impl OpaqueValue for TransportNavalRegion {}
impl OpaqueValue for TransportRadarCounts {}
impl OpaqueValue for SpottingNavyIntel {}
impl OpaqueValue for TransportAirEnvironment {}
impl OpaqueValue for TransportAirMission {}
impl OpaqueValue for TransportRiskShip {}

replay_rows::wire_value!(
    SpottingLanes,
);
