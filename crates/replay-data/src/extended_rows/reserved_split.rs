//! Wire tombstones of the reserved singleton's transport packet tables.
//!
//! The selected transport preview's packet briefly moved from the singleton
//! into keyed hourly tables ([`RESERVED_SPLIT_TABLES`]) in provisional format
//! 102 and retired in the same format: no published build wrote them and every
//! recording refuses them by name. The singleton's field is a
//! [`Retired`] tombstone, so a format-101 inline packet is discarded through
//! its framing. The retired nearest-base caches' row types also remain here.
//! These row types keep the tables' ids and framing; nothing reads or writes
//! their rows.
use super::*;

/// The retired tables that held the split reserved packet, in [`TableId`] order.
pub const RESERVED_SPLIT_TABLES: [TableId; 9] = [
    TableId::TransportNavies,
    TableId::TransportLedgers,
    TableId::TransportReceivers,
    TableId::TransportSearchRegions,
    TableId::TransportSpottingOwners,
    TableId::TransportSpottingRegions,
    TableId::TransportAirRoots,
    TableId::TransportAirRegions,
    TableId::TransportRiskShips,
];

/// ReservedNearestNavies: key physical navy slot (32 bits), 2 payload mask bits.
///
/// Retired wire tombstone (`RETIRED_INPUTS_FORMAT`): no recording holds it.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ReservedNearestNavyRow {
    pub slot: u32,
    pub present: bool,
    pub provinces: Option<u32>,
}
typed_codec!(ReservedNearestNavyRow, 32, |row| row.slot as u128, |key| Self { slot: key as u32, ..Default::default() },
 { 0 => (present): Bool, 1 => (provinces): OptionalU32 }, |_row| Ok(()));

/// ReservedNearestBases: key navy slot:province (64 bits), 1 payload mask bit.
///
/// Retired wire tombstone (`RETIRED_INPUTS_FORMAT`): no recording holds it.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ReservedNearestBaseRow {
    pub slot: u32,
    pub province: u32,
    pub base: u32,
}
typed_codec!(ReservedNearestBaseRow, 64, |row| (row.slot as u128) << 32 | row.province as u128, |key| Self { slot: (key >> 32) as u32, province: key as u32, base: 0 },
 { 0 => (base): U32 }, |_row| Ok(()));

/// TransportNavies: key physical manager ordinal (32 bits), 5 payload mask bits.
///
/// [`TransportNavy`] without its ledgers. `ledgers` false refuses this navy's
/// ledger domain; true admits its TransportLedgers rows, each a nonempty
/// region (an omitted region is empty within the original outer domain).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
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

/// TransportLedgers: key navy ordinal:region (64 bits), 1 payload mask bit.
/// One [`TransportLedger`], in the navy's increasing native region order.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TransportLedgerRow {
    pub navy: u32,
    pub region: u32,
    pub occurrences: Option<Vec<u64>>,
}
typed_codec!(TransportLedgerRow, 64, |row| (row.navy as u128) << 32 | row.region as u128, |key| Self { navy: (key >> 32) as u32, region: key as u32, occurrences: None },
 { 0 => (occurrences): Opaque }, |_row| Ok(()));

/// TransportReceivers: key full task-force handle (64 bits), 8 payload mask bits.
/// One entry of the unique increasing receiver dictionary.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TransportReceiverRow {
    pub receiver: TransportMissionReceiver,
}
typed_codec!(TransportReceiverRow, 64, |row| row.receiver.task_force as u128, |key| Self { receiver: TransportMissionReceiver { task_force: key as u64, ..Default::default() } },
 { 0 => (receiver.mission_owner): OptionalU32, 1 => (receiver.logical_owner): OptionalU32, 2 => (receiver.in_regions): Opaque, 3 => (receiver.combat_references): OptionalI32, 4 => (receiver.combined_detection): OptionalI64, 5 => (receiver.detection_cache): Opaque, 6 => (receiver.received_supply_ratio): OptionalI64, 7 => (receiver.risk): Opaque }, |_row| Ok(()));

/// TransportSearchRegions: key physical region ordinal (32 bits), 1 payload mask bit.
/// None is an authenticated null region slot.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TransportSearchRegionRow {
    pub ordinal: u32,
    pub region: Option<TransportNavalRegion>,
}
typed_codec!(TransportSearchRegionRow, 32, |row| row.ordinal as u128, |key| Self { ordinal: key as u32, region: None },
 { 0 => (region): Opaque }, |_row| Ok(()));

/// Where an owner's positional raw NAVY lanes are stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpottingLanes {
    /// Copied with this hour's read.
    Copied(Vec<i64>),
    /// This hour's read equalled, lane for lane and in length, the navy lanes
    /// of this owner's rows in the frame's complete CountryIntel sample of
    /// [`SpottingCoverage::navy_intel_sample`]: CountryIntel `owner_country`
    /// is the owner's raw root ID, an index below the count without a row is
    /// zero, and a nonpositive count has no rows.
    CountryIntel,
}

impl Default for SpottingLanes {
    fn default() -> Self {
        Self::Copied(Vec::new())
    }
}

impl Value for SpottingLanes {
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

/// [`TransportNavyIntel`] with its lanes stored or certified.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
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

/// TransportSpottingOwners: key canonical owner slot:occurrence (64 bits), 4 payload mask bits.
/// One [`TransportSpottingOwner`] in the context's slot order; `occurrence`
/// counts earlier owners with the same slot.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
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

/// TransportSpottingRegions: key original region ordinal (32 bits), 5 payload mask bits.
/// The sampled province, which changes every hour by native definition, is derived and
/// not stored ([`SpottingSample::Derived`]); only the changed fields of a region are
/// stored.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TransportSpottingRegionRow {
    pub ordinal: u32,
    pub region: TransportSpottingRegion,
}
typed_codec!(TransportSpottingRegionRow, 32, |row| row.ordinal as u128, |key| Self { ordinal: key as u32, ..Default::default() },
 { 0 => (region.region_id): U32, 1 => (region.runtime_slot): OptionalI32, 2 => (region.signed_province_count): OptionalI32, 3 => (region.weather_detection): Opaque, 4 => (region.air_environment): Opaque },
 |row| if row.region.sample == SpottingSample::Derived { Ok(()) } else { Err(invalid("split spotting regions derive their sample")) });

/// TransportAirRoots: key physical strategic air manager ordinal (32 bits), 5 payload mask bits.
/// [`TransportAirCountry`] without its headers: `regions` is the length of its Enemy-vector header
/// domain, None its refusal.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
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

/// TransportAirRegions: key root ordinal:header ordinal (64 bits), 3 payload mask bits.
///
/// A region header that differs from the empty header at its ordinal (its
/// own region ID, an authentic zero count and an empty mission list). Below a
/// root's `regions`, a header without a row is that empty header.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TransportAirRegionRow {
    pub root: u32,
    pub ordinal: u32,
    pub region: TransportAirRegion,
}
typed_codec!(TransportAirRegionRow, 64, |row| (row.root as u128) << 32 | row.ordinal as u128, |key| Self { root: (key >> 32) as u32, ordinal: key as u32, ..Default::default() },
 { 0 => (region.region): U32, 1 => (region.count): OptionalI32, 2 => (region.missions): Opaque }, |_row| Ok(()));

impl TransportAirRegionRow {
    /// The header an idle root holds at `ordinal`; it is never stored.
    pub fn empty(ordinal: u32) -> TransportAirRegion {
        TransportAirRegion {
            region: ordinal,
            count: Some(0),
            missions: Some(Vec::new()),
        }
    }
}

/// TransportRiskShips: key full task-force handle (64 bits), 2 payload mask bits.
/// One SHIPS receiver, in the packet's unique increasing handle order.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TransportRiskShipsRow {
    pub receiver: TransportReceiverShips,
}
typed_codec!(TransportRiskShipsRow, 64, |row| row.receiver.task_force as u128, |key| Self { receiver: TransportReceiverShips { task_force: key as u64, ..Default::default() } },
 { 0 => (receiver.count): OptionalI32, 1 => (receiver.ships): Opaque }, |_row| Ok(()));

opaque_value!(Option<[i64; 2]>);
opaque_value!(Option<TransportReceiverRisk>);
opaque_value!(Option<TransportNavalRegion>);
opaque_value!(Option<TransportRadarCounts>);
opaque_value!(Option<SpottingNavyIntel>);
opaque_value!(Option<Option<i64>>);
opaque_value!(Option<TransportAirEnvironment>);
opaque_value!(Option<Vec<Option<i64>>>);
opaque_value!(Option<Vec<Option<TransportAirMission>>>);
opaque_value!(Option<Vec<TransportRiskShip>>);
