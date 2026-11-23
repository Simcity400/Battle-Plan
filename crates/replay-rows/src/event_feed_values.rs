use crate::codec::{Decoder, invalid_static as invalid, put_varint};
use crate::rows::OpaqueValue;
use crate::value::Value;
use std::io;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct EventFeedScope {
    pub country: u32,
    pub random: [u32; 2],
    pub bindings: [u32; 3],
    pub state: i32,
    pub references: [u64; 9],
    pub strategic_region: Option<u32>,
    pub combat_context_unrecorded: bool,
    pub target_storage_unrecorded: bool,
    pub tooltip_storage_unrecorded: bool,
    pub country_attempted: bool,
    pub country_id: Option<u32>,
    pub country_names: Option<[EventFeedText; 3]>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct EventFeedText {
    len: u16,
    bytes: [u8; Self::CAPACITY],
}
impl EventFeedText {
    pub const CAPACITY: usize = 256;
    pub fn new(bytes: &[u8]) -> Option<Self> {
        Self::filled(bytes.len(), |output| {
            output.copy_from_slice(bytes);
            true
        })
    }
    pub fn filled(len: usize, fill: impl FnOnce(&mut [u8]) -> bool) -> Option<Self> {
        let mut text = Self::default();
        let output = text.bytes.get_mut(..len)?;
        fill(output).then_some(())?;
        text.len = len as u16;
        Some(text)
    }
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len as usize]
    }
}
impl Default for EventFeedText {
    fn default() -> Self {
        Self {
            len: 0,
            bytes: [0; Self::CAPACITY],
        }
    }
}
impl std::fmt::Debug for EventFeedText {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", String::from_utf8_lossy(self.as_bytes()))
    }
}
impl serde::Serialize for EventFeedText {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serde::Serialize::serialize(self.as_bytes(), serializer)
    }
}
impl<'de> serde::Deserialize<'de> for EventFeedText {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let bytes = <Vec<u8> as serde::Deserialize>::deserialize(deserializer)?;
        Self::new(&bytes)
            .ok_or_else(|| serde::de::Error::custom("Event feed text exceeds its capacity"))
    }
}

pub fn compact_len(value: u32) -> usize {
    ((32 - value.leading_zeros()).max(1) as usize).div_ceil(7)
}

pub fn compact_write(value: u32, output: &mut Vec<u8>) {
    put_varint(output, value as u128);
}

pub fn compact_read(input: &mut Decoder<'_>) -> io::Result<u32> {
    u32::try_from(input.varint()?).map_err(|_| invalid("Event feed integer exceeds u32"))
}

pub fn signed_word(value: i32) -> u32 {
    (value as u32).wrapping_shl(1) ^ (value >> 31) as u32
}

pub fn signed_value(value: u32) -> i32 {
    ((value >> 1) as i32) ^ -((value & 1) as i32)
}

impl Value for EventFeedScope {
    const MINIMUM_ENCODED_BYTES: usize = 16;
    fn heap_bytes(&self) -> usize {
        0
    }
    fn try_clone(&self) -> io::Result<Self> {
        Ok(*self)
    }
    fn encoded_bytes(&self) -> io::Result<usize> {
        Ok(11
            + compact_len(self.country)
            + self
                .bindings
                .iter()
                .map(|id| compact_len(*id))
                .sum::<usize>()
            + compact_len(signed_word(self.state))
            + self
                .references
                .iter()
                .filter(|reference| **reference != 0)
                .count()
                * 8
            + self.strategic_region.map(compact_len).unwrap_or(0)
            + self.country_id.map(compact_len).unwrap_or(0)
            + self
                .country_names
                .iter()
                .flatten()
                .map(|text| compact_len(text.len.into()) + text.as_bytes().len())
                .sum::<usize>())
    }
    fn encode_reserved(&self, output: &mut Vec<u8>) -> io::Result<()> {
        compact_write(self.country, output);
        self.random.encode_reserved(output)?;
        for binding in self.bindings {
            compact_write(binding, output);
        }
        compact_write(signed_word(self.state), output);
        let mask = self
            .references
            .iter()
            .enumerate()
            .fold(0_u16, |mask, (n, reference)| {
                mask | (u16::from(*reference != 0) << n)
            });
        mask.encode_reserved(output)?;
        for reference in self.references {
            if reference != 0 {
                reference.encode_reserved(output)?;
            }
        }
        let flags = u8::from(self.strategic_region.is_some())
            | (u8::from(self.combat_context_unrecorded) << 1)
            | (u8::from(self.target_storage_unrecorded) << 2)
            | (u8::from(self.tooltip_storage_unrecorded) << 3)
            | (u8::from(self.country_id.is_some()) << 4)
            | (u8::from(self.country_names.is_some()) << 5)
            | (u8::from(self.country_attempted) << 6);
        flags.encode_reserved(output)?;
        for value in [self.strategic_region, self.country_id].into_iter().flatten() {
            compact_write(value, output);
        }
        for text in self.country_names.iter().flatten() {
            compact_write(text.len.into(), output);
            output.extend_from_slice(text.as_bytes());
        }
        Ok(())
    }
    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        let country = compact_read(input)?;
        let random = <[u32; 2]>::decode(input)?;
        let bindings = [
            compact_read(input)?,
            compact_read(input)?,
            compact_read(input)?,
        ];
        let state = signed_value(compact_read(input)?);
        let mask = u16::decode(input)?;
        if mask >> 9 != 0 {
            return Err(invalid("Event feed scope reference mask differs"));
        }
        let mut references = [0; 9];
        for (n, reference) in references.iter_mut().enumerate() {
            if mask & (1 << n) != 0 {
                *reference = u64::decode(input)?;
            }
        }
        let flags = u8::decode(input)?;
        if flags >> 7 != 0 {
            return Err(invalid("Event feed scope flags differ"));
        }
        let strategic_region = if flags & 1 != 0 {
            Some(compact_read(input)?)
        } else {
            None
        };
        let country_id = if flags & 0x10 != 0 {
            Some(compact_read(input)?)
        } else {
            None
        };
        let country_names = if flags & 0x20 != 0 {
            let mut texts = [EventFeedText::default(); 3];
            for text in &mut texts {
                let len = compact_read(input)? as usize;
                *text = EventFeedText::new(input.take(len)?)
                    .ok_or_else(|| invalid("Event feed scope name exceeds its capacity"))?;
            }
            Some(texts)
        } else {
            None
        };
        Ok(Self {
            country_attempted: flags & 0x40 != 0,
            country_id,
            country_names,
            country,
            random,
            bindings,
            state,
            references,
            strategic_region,
            combat_context_unrecorded: flags & 2 != 0,
            target_storage_unrecorded: flags & 4 != 0,
            tooltip_storage_unrecorded: flags & 8 != 0,
        })
    }
}

crate::wire_value!(EventFeedScope);
impl OpaqueValue for EventFeedScope {}
