use super::*;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct TransportRadarCounts {
    pub signed_count: i32,
    pub counts: Option<Vec<i32>>,
}
value_struct!(TransportRadarCounts { signed_count: i32, counts: Option<Vec<i32>> });

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct TransportSpottingRegion {
    pub region_id: u32,
    pub runtime_slot: Option<i32>,
    pub signed_province_count: Option<i32>,
    pub sample: SpottingSample,
    pub weather_detection: Option<Option<i64>>,
    pub air_environment: Option<TransportAirEnvironment>,
}
value_struct!(TransportSpottingRegion {
    region_id: u32, runtime_slot: Option<i32>, signed_province_count: Option<i32>,
    sample: SpottingSample, weather_detection: Option<Option<i64>>,
    air_environment: Option<TransportAirEnvironment>,
});

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub enum SpottingSample {
    Recorded(Option<u32>),
    #[default]
    Derived,
}

impl SpottingSample {
    pub fn captured(self) -> Option<u32> {
        match self {
            Self::Recorded(province) => province,
            Self::Derived => None,
        }
    }
}

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

use replay_rows::checkpoint::values::CheckpointValue;

impl CheckpointValue for SpottingSample {
    const MINIMUM_BYTES: usize = 1;
    fn encode(&self, output: &mut Vec<u8>) -> io::Result<()> {
        match self {
            Self::Recorded(province) => {
                Value::encode(&0_u8, output)?;
                CheckpointValue::encode(province, output)
            }
            Self::Derived => Value::encode(&1_u8, output),
        }
    }
    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        match input.u8()? {
            0 => CheckpointValue::decode(input).map(Self::Recorded),
            1 => Ok(Self::Derived),
            _ => Err(invalid("invalid checkpoint spotting sample")),
        }
    }
}
