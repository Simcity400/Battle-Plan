//! Value types of the retired TransportSpottingOwners and
//! TransportSpottingRegions tables ([`super::RESERVED_SPLIT_TABLES`]): wire
//! tombstones that keep the tables' framing. Nothing reads or writes them.
use super::*;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct TransportRadarCounts {
    /// The radar gate tests equality to zero, not signed nonpositive.
    pub signed_count: i32,
    /// None refuses the original body, including unsupported negative counts.
    pub counts: Option<Vec<i32>>,
}
value_struct!(TransportRadarCounts { signed_count: i32, counts: Option<Vec<i32>> });

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct TransportSpottingRegion {
    pub region_id: u32,
    pub runtime_slot: Option<i32>,
    /// Stored signed region, from the same original header as the sample.
    pub signed_province_count: Option<i32>,
    /// The province the air-spotting calculation samples in this region this hour.
    pub sample: SpottingSample,
    /// None is refusal; Some(None) is native null; Some(Some(v)) is effective2A.
    pub weather_detection: Option<Option<i64>>,
    /// Independent weather/night operands from that same original selector.
    pub air_environment: Option<TransportAirEnvironment>,
}
value_struct!(TransportSpottingRegion {
    region_id: u32, runtime_slot: Option<i32>, signed_province_count: Option<i32>,
    sample: SpottingSample, weather_detection: Option<Option<i64>>,
    air_environment: Option<TransportAirEnvironment>,
});

/// The province the air-spotting calculation sampled in a region, in the retired spotting
/// tables' layout.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub enum SpottingSample {
    /// Inline packets through format 101: the captured province, its id
    /// authenticated against the region; None is the capture's refusal.
    Recorded(Option<u32>),
    /// The retired split rows recorded none.
    #[default]
    Derived,
}

impl SpottingSample {
    /// The captured province, if this is a recorded sample that succeeded.
    pub fn captured(self) -> Option<u32> {
        match self {
            Self::Recorded(province) => province,
            Self::Derived => None,
        }
    }
}

/// Only inline packets store a sample, in their format-101 layout.
impl Value for SpottingSample {
    fn heap_bytes(&self) -> usize {
        0
    }
    const MINIMUM_ENCODED_BYTES: usize = <Option<u32> as Value>::MINIMUM_ENCODED_BYTES;
    fn encoded_bytes(&self) -> io::Result<usize> {
        match self {
            Self::Recorded(province) => Value::encoded_bytes(province),
            Self::Derived => Err(invalid("a derived spotting sample is never stored inline")),
        }
    }
    fn try_clone(&self) -> io::Result<Self> {
        Ok(*self)
    }
    fn encode_reserved(&self, output: &mut Vec<u8>) -> io::Result<()> {
        match self {
            Self::Recorded(province) => province.encode_reserved(output),
            Self::Derived => Err(invalid("a derived spotting sample is never stored inline")),
        }
    }
    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        Option::<u32>::decode(input).map(Self::Recorded)
    }
}
