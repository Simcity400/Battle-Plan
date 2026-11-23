use replay_data::codec::{Decoder, invalid_static as invalid, put_varint};
use serde::{Deserialize, Serialize};
use std::io;

mod codec;
use codec::{decode_batch, encode_batch, narrow};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[repr(u8)]
pub enum Category {
    #[default]
    Other,
    Army,
    BattlePlans,
    Navy,
    Air,
    Production,
    Construction,
    Research,
    Politics,
    Diplomacy,
    Intelligence,
    Logistics,
    Deployment,
    Equipment,
    Selection,
    Interface,
    Session,
    Automatic,
    System,
}

impl Category {
    pub fn counted(self) -> bool {
        !matches!(
            self,
            Self::Other
                | Self::System
                | Self::Session
                | Self::Automatic
                | Self::Interface
                | Self::Selection
        )
    }

    fn read(value: u8) -> io::Result<Self> {
        const ALL: [Category; 19] = [
            Category::Other,
            Category::Army,
            Category::BattlePlans,
            Category::Navy,
            Category::Air,
            Category::Production,
            Category::Construction,
            Category::Research,
            Category::Politics,
            Category::Diplomacy,
            Category::Intelligence,
            Category::Logistics,
            Category::Deployment,
            Category::Equipment,
            Category::Selection,
            Category::Interface,
            Category::Session,
            Category::Automatic,
            Category::System,
        ];
        ALL.get(value as usize)
            .copied()
            .ok_or_else(|| invalid("unknown command category"))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Kind {
    pub id: u32,
    pub token: u32,
    pub name: String,
    pub category: Category,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Actor {
    pub id: u64,
    pub player: i32,
    pub country: u32,
    pub since_qpc: u64,
    pub at_qpc: u64,
    pub user: Vec<u8>,
    pub name: Vec<u8>,
    pub tag: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum Origin {
    #[default]
    Unresolved,
    PlayerOrder,
    AutonomousCountry,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    pub qpc: u64,
    pub hour: i32,
    pub actor: u64,
    pub player: i32,
    pub kind: u32,
    pub command_country: Option<u32>,
    pub origin: Origin,
    pub sequence: u32,
    pub paused: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Span {
    pub from_qpc: u64,
    pub to_qpc: u64,
    pub complete: bool,
    pub hour: i32,
    pub paused: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct Batch {
    pub kinds: Vec<Kind>,
    pub actors: Vec<Actor>,
    pub departures: Vec<(u64, u64)>,
    pub events: Vec<Event>,
    #[serde(alias = "coverage")]
    pub spans: Vec<Span>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Phase {
    pub start: replay_frame::branches::BranchStart,
    pub campaign: [u8; 36],
    pub qpc: u64,
    pub hour: i32,
    pub load_signal: i64,
}

pub fn load_marker(raw: i64) -> Option<i64> {
    (raw != 0).then_some(raw)
}

impl Phase {
    pub fn load_marker(&self) -> Option<i64> {
        load_marker(self.load_signal)
    }

    pub fn validate(&self) -> io::Result<()> {
        self.start.validate()?;
        if self.qpc == 0
            || self.hour < self.start.first_hour
            || !replay_data::identity::is_campaign_uuid(&self.campaign)
        {
            return Err(invalid("invalid command phase identity"));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Packet {
    pub branch: u64,
    pub capture: u64,
    pub world: u64,
    pub ordinal: u64,
    pub from_qpc: u64,
    pub to_qpc: u64,
    pub lost: u64,
    pub phase: Option<Phase>,
    raw_length: u32,
    compressed: bool,
    data: Vec<u8>,
}

impl Packet {
    pub fn storage_bytes(&self) -> usize {
        self.data.capacity()
    }
    pub fn pack(
        branch: u64,
        capture: u64,
        world: u64,
        ordinal: u64,
        from_qpc: u64,
        to_qpc: u64,
        lost: u64,
        batch: &Batch,
    ) -> io::Result<Self> {
        if from_qpc > to_qpc || world == 0 || capture == 0 {
            return Err(invalid("invalid command packet interval"));
        }
        let raw = encode_batch(batch, from_qpc, to_qpc)?;
        let raw_length =
            u32::try_from(raw.len()).map_err(|_| invalid("command packet too large"))?;
        if raw.len() > replay_frame::envelope::MAX_RECORD_BYTES {
            return Err(invalid("command packet exceeds record bound"));
        }
        let compressed = if raw.len() > 1024 {
            Some(zstd::bulk::compress(&raw, 3)?)
        } else {
            None
        };
        let (data, compressed) = match compressed {
            Some(data) if data.len() < raw.len() => (data, true),
            _ => (engine_allocation::copy_vec(&raw)?, false),
        };
        Ok(Self {
            branch,
            capture,
            world,
            ordinal,
            from_qpc,
            to_qpc,
            lost,
            phase: None,
            raw_length,
            compressed,
            data,
        })
    }

    pub fn unpack(&self) -> io::Result<Batch> {
        if !self.compressed {
            if self.data.len() != self.raw_length as usize {
                return Err(invalid("command packet length mismatch"));
            }
            return decode_batch(&self.data, self.from_qpc, self.to_qpc);
        }
        let raw = zstd::bulk::decompress(&self.data, self.raw_length as usize)?;
        if raw.len() != self.raw_length as usize {
            return Err(invalid("command packet length mismatch"));
        }
        decode_batch(&raw, self.from_qpc, self.to_qpc)
    }

    pub(crate) fn encoded_bytes(&self) -> io::Result<usize> {
        engine_allocation::add(
            self.header_values()
                .into_iter()
                .map(varint_bytes)
                .sum::<usize>()
                + 2
                + self
                    .phase
                    .as_ref()
                    .map_or(0, |phase| phase.start.encoded_bytes() + 36 + 8 + 4 + 8),
            self.data.len(),
        )
    }

    fn header_values(&self) -> [u128; 9] {
        [
            self.branch.into(),
            self.capture.into(),
            self.world.into(),
            self.ordinal.into(),
            self.from_qpc.into(),
            (self.to_qpc - self.from_qpc).into(),
            self.lost.into(),
            self.raw_length.into(),
            self.data.len() as u128,
        ]
    }

    pub(crate) fn encode(&self, out: &mut Vec<u8>) -> io::Result<()> {
        engine_allocation::reserve(out, self.encoded_bytes()?)?;
        for value in &self.header_values()[..7] {
            put_varint(out, *value);
        }
        match &self.phase {
            None => out.push(0),
            Some(phase) => {
                phase.validate()?;
                if phase.start.id != self.branch {
                    return Err(invalid("command phase differs from packet"));
                }
                out.push(1);
                phase.start.encode(out)?;
                out.extend_from_slice(&phase.campaign);
                out.extend_from_slice(&phase.qpc.to_le_bytes());
                out.extend_from_slice(&phase.hour.to_le_bytes());
                out.extend_from_slice(&phase.load_signal.to_le_bytes());
            }
        }
        out.push(u8::from(self.compressed));
        put_varint(out, self.raw_length.into());
        put_varint(out, self.data.len() as u128);
        out.extend_from_slice(&self.data);
        Ok(())
    }

    pub(crate) fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        let branch = narrow(input.varint()?)?;
        let capture = narrow(input.varint()?)?;
        let world = narrow(input.varint()?)?;
        let ordinal = narrow(input.varint()?)?;
        let from_qpc: u64 = narrow(input.varint()?)?;
        let to_qpc = from_qpc
            .checked_add(narrow(input.varint()?)?)
            .ok_or_else(|| invalid("command packet interval overflow"))?;
        let lost = narrow(input.varint()?)?;
        let value = Self {
            branch,
            capture,
            world,
            ordinal,
            from_qpc,
            to_qpc,
            lost,
            phase: match input.u8()? {
                0 => None,
                1 => {
                    let start = replay_frame::branches::BranchStart::decode(input)?;
                    let mut campaign = [0; 36];
                    for byte in &mut campaign {
                        *byte = input.u8()?;
                    }
                    let phase = Phase {
                        start,
                        campaign,
                        qpc: input.u64()?,
                        hour: input.i32()?,
                        load_signal: input.i64()?,
                    };
                    phase.validate()?;
                    Some(phase)
                }
                _ => return Err(invalid("invalid command phase option")),
            },
            compressed: input.bool()?,
            raw_length: narrow(input.varint()?)?,
            data: {
                let length = narrow::<usize>(input.varint()?)?;
                if length > replay_frame::envelope::MAX_RECORD_BYTES {
                    return Err(invalid("command packet exceeds record bound"));
                }
                engine_allocation::copy_vec(input.take(length)?)?
            },
        };
        if value.world == 0
            || value.capture == 0
            || value.from_qpc > value.to_qpc
            || value.raw_length as usize > replay_frame::envelope::MAX_RECORD_BYTES
            || (!value.compressed && value.data.len() != value.raw_length as usize)
            || value
                .phase
                .as_ref()
                .is_some_and(|phase| phase.start.id != value.branch)
        {
            return Err(invalid("invalid command packet header"));
        }
        Ok(value)
    }
}

fn varint_bytes(value: u128) -> usize {
    ((128 - value.leading_zeros() as usize).max(1) + 6) / 7
}


#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
#[cfg(test)]
mod tests;
