use replay_data::{BranchId, codec::{Decoder, invalid, invalid_static, put_string}, format::FormatVersion, rows::compatibility::RecordKind};
use std::io;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ControlRecord {
    PlayerCommands {
        session: u64,
        packet: Box<crate::command_packet::Packet>,
    },
    Detach {
        session: u64,
        last_hour: Option<i32>,
    },
    ClockWitness {
        session: u64,
        qpc: u64,
        hour: i32,
        load_signal: Option<i64>,
    },
    GapSummary {
        session: u64,
        dropped_frames: u64,
        reason: String,
    },
    Partial {
        session: u64,
        reason: String,
        failed_hour: Option<i32>,
    },
    EventLoss {
        session: u64,
        branch: BranchId,
        after_hour: i32,
        hour: i32,
    },
}

impl ControlRecord {
    pub fn encoded_bytes(&self) -> io::Result<usize> {
        let tagged_session = size_of::<u8>() + size_of::<u64>();
        let (fields, reason) = match self {
            Self::PlayerCommands { packet, .. } => (packet.encoded_bytes()?, None),
            Self::Detach { last_hour, .. } => (
                size_of::<u8>() + usize::from(last_hour.is_some()) * size_of::<i32>(),
                None,
            ),
            Self::ClockWitness { load_signal, .. } => (
                size_of::<u64>()
                    + size_of::<i32>()
                    + size_of::<u8>()
                    + usize::from(load_signal.is_some()) * size_of::<i64>(),
                None,
            ),
            Self::GapSummary { reason, .. } => (size_of::<u64>(), Some(reason)),
            Self::Partial {
                reason,
                failed_hour,
                ..
            } => (
                size_of::<u8>() + usize::from(failed_hour.is_some()) * size_of::<i32>(),
                Some(reason),
            ),
            Self::EventLoss { .. } => (size_of::<BranchId>() + 2 * size_of::<i32>(), None),
        };
        engine_allocation::add(
            tagged_session + fields + usize::from(reason.is_some()) * size_of::<u32>(),
            reason.map_or(0, String::len),
        )
    }

    pub const fn kind(&self) -> RecordKind {
        match self {
            Self::PlayerCommands { .. } => RecordKind::PlayerCommands,
            Self::Detach { .. } => RecordKind::Detach,
            Self::ClockWitness { .. } => RecordKind::ClockWitness,
            Self::GapSummary { .. } => RecordKind::GapSummary,
            Self::Partial { .. } => RecordKind::Partial,
            Self::EventLoss { .. } => RecordKind::EventLoss,
        }
    }

    pub const fn session(&self) -> u64 {
        match self {
            Self::PlayerCommands { session, .. } => *session,
            Self::Detach { session, .. }
            | Self::ClockWitness { session, .. }
            | Self::GapSummary { session, .. }
            | Self::Partial { session, .. }
            | Self::EventLoss { session, .. } => *session,
        }
    }

    pub fn encode(&self, out: &mut Vec<u8>) -> io::Result<()> {
        engine_allocation::reserve(out, self.encoded_bytes()?)?;
        match self {
            Self::PlayerCommands { session, packet } => {
                out.push(9);
                out.extend_from_slice(&session.to_le_bytes());
                packet.encode(out)?;
            }
            Self::EventLoss {
                session,
                branch,
                after_hour,
                hour,
            } => {
                if after_hour >= hour {
                    return Err(invalid_static("event loss must end after it begins"));
                }
                out.push(8);
                out.extend_from_slice(&session.to_le_bytes());
                out.extend_from_slice(&branch.to_le_bytes());
                out.extend_from_slice(&after_hour.to_le_bytes());
                out.extend_from_slice(&hour.to_le_bytes());
            }
            Self::Partial {
                session,
                reason,
                failed_hour,
            } => {
                if reason.trim().is_empty() {
                    return Err(invalid_static("partial capture needs a reason"));
                }
                out.push(7);
                out.extend_from_slice(&session.to_le_bytes());
                put_string(out, reason)?;
                out.push(u8::from(failed_hour.is_some()));
                if let Some(hour) = failed_hour {
                    out.extend_from_slice(&hour.to_le_bytes());
                }
            }
            Self::GapSummary {
                session,
                dropped_frames,
                reason,
            } => {
                if *dropped_frames == 0 || reason.trim().is_empty() {
                    return Err(invalid_static("gap summary needs a count and reason"));
                }
                out.push(6);
                out.extend_from_slice(&session.to_le_bytes());
                out.extend_from_slice(&dropped_frames.to_le_bytes());
                put_string(out, reason)?;
            }
            Self::Detach { session, last_hour } => {
                out.push(4);
                out.extend_from_slice(&session.to_le_bytes());
                out.push(u8::from(last_hour.is_some()));
                if let Some(hour) = last_hour {
                    out.extend_from_slice(&hour.to_le_bytes());
                }
            }
            Self::ClockWitness {
                session,
                qpc,
                hour,
                load_signal,
            } => {
                out.push(5);
                out.extend_from_slice(&session.to_le_bytes());
                out.extend_from_slice(&qpc.to_le_bytes());
                out.extend_from_slice(&hour.to_le_bytes());
                out.push(u8::from(load_signal.is_some()));
                if let Some(signal) = load_signal {
                    out.extend_from_slice(&signal.to_le_bytes());
                }
            }
        }
        Ok(())
    }

    pub fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        Self::decode_version(input, FormatVersion::CURRENT)
    }

    pub fn decode_version(
        input: &mut Decoder<'_>,
        format: FormatVersion,
    ) -> io::Result<Self> {
        Self::decode_tag(input.u8()?, input, format)
    }

    pub fn decode_tag(tag: u8, input: &mut Decoder<'_>, format: FormatVersion) -> io::Result<Self> {
        if tag == 9 {
            format.require(RecordKind::PlayerCommands)?;
            return Ok(Self::PlayerCommands {
                session: input.u64()?,
                packet: engine_allocation::boxed(crate::command_packet::Packet::decode(input)?)?,
            });
        }
        format.require(match tag {
            4 => RecordKind::Detach,
            5 => RecordKind::ClockWitness,
            6 => RecordKind::GapSummary,
            7 => RecordKind::Partial,
            8 => RecordKind::EventLoss,
            _ => return Err(invalid("unknown control record tag")),
        })?;
        Ok(match tag {
            8 => {
                let (session, branch) = (input.u64()?, input.u64()?);
                let (after_hour, hour) = (input.i32()?, input.i32()?);
                if after_hour >= hour {
                    return Err(invalid("event loss must end after it begins"));
                }
                Self::EventLoss {
                    session,
                    branch,
                    after_hour,
                    hour,
                }
            }
            7 => {
                let session = input.u64()?;
                let reason = input.string()?;
                if reason.trim().is_empty() {
                    return Err(invalid("partial capture needs a reason"));
                }
                Self::Partial {
                    session,
                    reason,
                    failed_hour: if input.bool()? {
                        Some(input.i32()?)
                    } else {
                        None
                    },
                }
            }
            6 => {
                let session = input.u64()?;
                let dropped_frames = input.u64()?;
                let reason = input.string()?;
                if dropped_frames == 0 || reason.trim().is_empty() {
                    return Err(invalid("gap summary needs a count and reason"));
                }
                Self::GapSummary {
                    session,
                    dropped_frames,
                    reason,
                }
            }
            4 => Self::Detach {
                session: input.u64()?,
                last_hour: if input.bool()? {
                    Some(input.i32()?)
                } else {
                    None
                },
            },
            5 => Self::ClockWitness {
                session: input.u64()?,
                qpc: input.u64()?,
                hour: input.i32()?,
                load_signal: if input.bool()? {
                    Some(input.i64()?)
                } else {
                    None
                },
            },
            _ => return Err(invalid("unknown control record tag")),
        })
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_control_record_is_sized_for_its_common_variants() {
        assert_eq!(size_of::<super::ControlRecord>(), 48);
    }
}
