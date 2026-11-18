//! Read hour of a nested packet on an hourly host row.
//!
//! Every host of a [`PacketHour`] is hourly ([`crate::cadence`]): the
//! encoder never carries it (`FrameEncoder::can_carry` admits only staggered
//! tables), so an hour's complete or truncated host rows were acquired at that
//! hour, and so were the packets they hold. A packet that was not read is its
//! explicit refusal (`None`, or `None` operands), never an earlier packet.
//! Current writers therefore store no hour, equal acquisitions encode
//! `Unchanged`, and a keyframe or seek proves freshness from the frame alone.
//!
//! Recordings through [`STAMPED_PACKET_FORMAT`] stored the read hour in every
//! packet; it decodes as [`PacketHour::Stamped`] and still decides freshness.
use super::*;

/// Last format whose hourly-host packets store their read hour.
pub const STAMPED_PACKET_FORMAT: u16 = crate::rows::compatibility::POLITICAL_FORMAT;


#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub enum PacketHour {
    /// Read with its hourly host row; costs no bytes.
    #[default]
    Host,
    /// Historical per-packet read hour. Never written by current formats.
    Stamped(i32),
}

impl PacketHour {
    /// Whether the packet was read at `hour`, the hour of the frame whose
    /// decoded host rows hold it. A host packet always was; a historical
    /// stamp must equal that hour.
    pub fn fresh_at(self, hour: i32) -> bool {
        match self {
            Self::Host => true,
            Self::Stamped(stamp) => stamp == hour,
        }
    }

    /// The stored historical hour, for diagnostics; None for host packets.
    pub fn stamp(self) -> Option<i32> {
        match self {
            Self::Host => None,
            Self::Stamped(stamp) => Some(stamp),
        }
    }
}

/// Diagnostics name a historical stamp by its hour.
impl std::fmt::Display for PacketHour {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Host => f.write_str("its host row's hour"),
            Self::Stamped(stamp) => write!(f, "{stamp}"),
        }
    }
}

impl Value for PacketHour {
    fn heap_bytes(&self) -> usize {
        0
    }
    const MINIMUM_ENCODED_BYTES: usize = 0;

    fn encoded_bytes(&self) -> io::Result<usize> {
        match self {
            Self::Host => Ok(historical_stamp().map_or(0, |_| size_of::<i32>())),
            Self::Stamped(_) => Err(invalid("historical packet hour cannot be written")),
        }
    }
    fn recorded_bytes(&self) -> io::Result<usize> {
        Ok(match self {
            Self::Host => 0,
            Self::Stamped(_) => size_of::<i32>(),
        })
    }
    fn try_clone(&self) -> io::Result<Self> {
        Ok(*self)
    }
    fn encode_reserved(&self, output: &mut Vec<u8>) -> io::Result<()> {
        match self {
            Self::Host => {
                if let Some(hour) = historical_stamp() {
                    output.extend_from_slice(&hour.to_le_bytes());
                }
                Ok(())
            }
            Self::Stamped(_) => Err(invalid("historical packet hour cannot be written")),
        }
    }
    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        if input.format <= STAMPED_PACKET_FORMAT {
            input.i32().map(Self::Stamped)
        } else {
            Ok(Self::Host)
        }
    }
}

#[cfg(not(any(test, feature = "test-support")))]
pub fn historical_stamp() -> Option<i32> {
    None
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    static HISTORICAL_STAMP: std::cell::Cell<Option<i32>> = const { std::cell::Cell::new(None) };
}

#[cfg(any(test, feature = "test-support"))]
pub fn historical_stamp() -> Option<i32> {
    HISTORICAL_STAMP.with(std::cell::Cell::get)
}

/// Test-only stand-in for the format-101 writer: host packets encoded inside
/// `encode` carry `hour`, as every stamped packet of that hour's rows did.
#[cfg(any(test, feature = "test-support"))]
pub fn with_historical_stamp<R>(hour: i32, encode: impl FnOnce() -> R) -> R {
    let previous = HISTORICAL_STAMP.with(|stamp| stamp.replace(Some(hour)));
    let result = encode();
    HISTORICAL_STAMP.with(|stamp| stamp.set(previous));
    result
}

