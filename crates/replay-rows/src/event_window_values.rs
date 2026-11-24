use crate::codec::{Decoder, invalid_static as invalid};
use crate::rows::Recorded;
use crate::value::Value;
use std::io;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub enum WindowVerdict {
    Requested,
    AiControlled,
    HiddenOrAiControlled,
    NotViewingCountry,
    SenderExcluded,
    PlayerRestrictedNotMajor,
    PlayerRestrictedOptionTrigger,
    ExistingInstance,
    AlreadyOpen,
}

impl WindowVerdict {
    pub const ALL: [Self; 9] = [
        Self::Requested,
        Self::AiControlled,
        Self::HiddenOrAiControlled,
        Self::NotViewingCountry,
        Self::SenderExcluded,
        Self::PlayerRestrictedNotMajor,
        Self::PlayerRestrictedOptionTrigger,
        Self::ExistingInstance,
        Self::AlreadyOpen,
    ];
    pub const fn code(self) -> u8 {
        self as u8 + 1
    }
    pub fn from_code(code: u32) -> Option<Self> {
        code.checked_sub(1)
            .and_then(|index| Self::ALL.get(index as usize).copied())
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct OfferedOption {
    pub ordinal: Option<u32>,
    pub label: Option<Vec<u8>>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub enum WindowKind {
    #[default]
    Normal,
    News,
    Leader,
    Operative,
}

impl WindowKind {
    const ALL: [Self; 4] = [Self::Normal, Self::News, Self::Leader, Self::Operative];
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct EventPresentation {
    pub native_hour: Option<i32>,
    pub instance: u32,
    pub event_kind: u32,
    pub event_id: u32,
    pub actor: u32,
    pub rebuild: bool,
    pub kind: WindowKind,
    pub shown: Option<bool>,
    pub option_count: u32,
    pub token: Recorded<Vec<u8>>,
    pub title: Recorded<Vec<u8>>,
    pub description: Recorded<Vec<u8>>,
    pub picture: Recorded<Option<Vec<u8>>>,
    pub options: Recorded<Vec<OfferedOption>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct VerdictEntry {
    pub sequence: u64,
    pub verdict: WindowVerdict,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct EventWindows {
    pub verdicts: Result<Vec<VerdictEntry>, Vec<u8>>,
    pub presentations: Vec<EventPresentation>,
    pub presentations_unrecorded: Option<Vec<u8>>,
}

impl Default for EventWindows {
    fn default() -> Self {
        Self {
            verdicts: Err(Self::NOT_ARMED.to_vec()),
            presentations: Vec::new(),
            presentations_unrecorded: Some(Self::NOT_ARMED.to_vec()),
        }
    }
}

impl EventWindows {
    pub const NOT_ARMED: &'static [u8] = b"event window capture was not armed";
}

fn text_bytes(text: &[u8]) -> io::Result<usize> {
    Value::encoded_bytes(&text.to_vec())
}

impl Value for OfferedOption {
    const MINIMUM_ENCODED_BYTES: usize = 2;
    fn heap_bytes(&self) -> usize {
        self.label.as_ref().map_or(0, Vec::capacity)
    }
    fn try_clone(&self) -> io::Result<Self> {
        Ok(Self {
            ordinal: self.ordinal,
            label: Value::try_clone(&self.label)?,
        })
    }
    fn encoded_bytes(&self) -> io::Result<usize> {
        Ok(Value::encoded_bytes(&self.ordinal)? + Value::encoded_bytes(&self.label)?)
    }
    fn encode_reserved(&self, output: &mut Vec<u8>) -> io::Result<()> {
        self.ordinal.encode_reserved(output)?;
        self.label.encode_reserved(output)
    }
    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        Ok(Self {
            ordinal: Value::decode(input)?,
            label: Value::decode(input)?,
        })
    }
}

impl Value for WindowKind {
    const MINIMUM_ENCODED_BYTES: usize = 1;
    fn heap_bytes(&self) -> usize {
        0
    }
    fn try_clone(&self) -> io::Result<Self> {
        Ok(*self)
    }
    fn encoded_bytes(&self) -> io::Result<usize> {
        Ok(1)
    }
    fn encode_reserved(&self, output: &mut Vec<u8>) -> io::Result<()> {
        output.push(*self as u8);
        Ok(())
    }
    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        Self::ALL
            .get(usize::from(input.u8()?))
            .copied()
            .ok_or_else(|| invalid("event window kind differs"))
    }
}

impl Value for WindowVerdict {
    const MINIMUM_ENCODED_BYTES: usize = 1;
    fn heap_bytes(&self) -> usize {
        0
    }
    fn try_clone(&self) -> io::Result<Self> {
        Ok(*self)
    }
    fn encoded_bytes(&self) -> io::Result<usize> {
        Ok(1)
    }
    fn encode_reserved(&self, output: &mut Vec<u8>) -> io::Result<()> {
        output.push(self.code());
        Ok(())
    }
    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        Self::from_code(u32::from(input.u8()?)).ok_or_else(|| invalid("event window verdict differs"))
    }
}

impl Value for VerdictEntry {
    const MINIMUM_ENCODED_BYTES: usize = 9;
    fn heap_bytes(&self) -> usize {
        0
    }
    fn try_clone(&self) -> io::Result<Self> {
        Ok(*self)
    }
    fn encoded_bytes(&self) -> io::Result<usize> {
        Ok(9)
    }
    fn encode_reserved(&self, output: &mut Vec<u8>) -> io::Result<()> {
        self.sequence.encode_reserved(output)?;
        self.verdict.encode_reserved(output)
    }
    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        Ok(Self {
            sequence: Value::decode(input)?,
            verdict: Value::decode(input)?,
        })
    }
}

impl Value for EventPresentation {
    const MINIMUM_ENCODED_BYTES: usize = 24;
    fn heap_bytes(&self) -> usize {
        Value::heap_bytes(&self.token)
            + Value::heap_bytes(&self.title)
            + Value::heap_bytes(&self.description)
            + Value::heap_bytes(&self.picture)
            + Value::heap_bytes(&self.options)
    }
    fn try_clone(&self) -> io::Result<Self> {
        Ok(Self {
            token: Value::try_clone(&self.token)?,
            title: Value::try_clone(&self.title)?,
            description: Value::try_clone(&self.description)?,
            picture: Value::try_clone(&self.picture)?,
            options: Value::try_clone(&self.options)?,
            ..*self
        })
    }
    fn encoded_bytes(&self) -> io::Result<usize> {
        Ok(Value::encoded_bytes(&self.native_hour)?
            + 18
            + Value::encoded_bytes(&self.shown)?
            + 4
            + recorded_bytes(&self.token)?
            + recorded_bytes(&self.title)?
            + recorded_bytes(&self.description)?
            + recorded_bytes(&self.picture)?
            + recorded_bytes(&self.options)?)
    }
    fn encode_reserved(&self, output: &mut Vec<u8>) -> io::Result<()> {
        self.native_hour.encode_reserved(output)?;
        for word in [self.instance, self.event_kind, self.event_id, self.actor] {
            word.encode_reserved(output)?;
        }
        self.rebuild.encode_reserved(output)?;
        self.kind.encode_reserved(output)?;
        self.shown.encode_reserved(output)?;
        self.option_count.encode_reserved(output)?;
        encode_recorded(&self.token, output)?;
        encode_recorded(&self.title, output)?;
        encode_recorded(&self.description, output)?;
        encode_recorded(&self.picture, output)?;
        encode_recorded(&self.options, output)
    }
    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        Ok(Self {
            native_hour: Value::decode(input)?,
            instance: Value::decode(input)?,
            event_kind: Value::decode(input)?,
            event_id: Value::decode(input)?,
            actor: Value::decode(input)?,
            rebuild: Value::decode(input)?,
            kind: Value::decode(input)?,
            shown: Value::decode(input)?,
            option_count: Value::decode(input)?,
            token: decode_recorded(input)?,
            title: decode_recorded(input)?,
            description: decode_recorded(input)?,
            picture: decode_recorded(input)?,
            options: decode_recorded(input)?,
        })
    }
}

fn recorded_bytes<T: Value>(value: &Recorded<T>) -> io::Result<usize> {
    Ok(1 + match value {
        Recorded::Value(value) => Value::encoded_bytes(value)?,
        Recorded::NotRecorded => 0,
    })
}
fn encode_recorded<T: Value>(value: &Recorded<T>, output: &mut Vec<u8>) -> io::Result<()> {
    match value {
        Recorded::Value(value) => {
            output.push(1);
            value.encode_reserved(output)
        }
        Recorded::NotRecorded => {
            output.push(0);
            Ok(())
        }
    }
}
fn decode_recorded<T: Value>(input: &mut Decoder<'_>) -> io::Result<Recorded<T>> {
    match input.u8()? {
        0 => Ok(Recorded::NotRecorded),
        1 => Ok(Recorded::Value(T::decode(input)?)),
        _ => Err(invalid("event window recorded tag differs")),
    }
}

fn encode_outcome<T: Value>(value: &Result<Vec<T>, Vec<u8>>, output: &mut Vec<u8>) -> io::Result<()> {
    match value {
        Ok(rows) => {
            output.push(1);
            rows.encode_reserved(output)
        }
        Err(reason) => {
            output.push(0);
            reason.encode_reserved(output)
        }
    }
}
fn outcome_bytes<T: Value>(value: &Result<Vec<T>, Vec<u8>>) -> io::Result<usize> {
    Ok(1 + match value {
        Ok(rows) => Value::encoded_bytes(rows)?,
        Err(reason) => text_bytes(reason)?,
    })
}
fn decode_outcome<T: Value>(input: &mut Decoder<'_>) -> io::Result<Result<Vec<T>, Vec<u8>>> {
    match input.u8()? {
        0 => Ok(Err(Value::decode(input)?)),
        1 => Ok(Ok(Value::decode(input)?)),
        _ => Err(invalid("event window outcome tag differs")),
    }
}
fn clone_outcome<T: Value>(value: &Result<Vec<T>, Vec<u8>>) -> io::Result<Result<Vec<T>, Vec<u8>>> {
    Ok(match value {
        Ok(rows) => Ok(Value::try_clone(rows)?),
        Err(reason) => Err(Value::try_clone(reason)?),
    })
}

impl Value for EventWindows {
    const MINIMUM_ENCODED_BYTES: usize = 10;
    fn heap_bytes(&self) -> usize {
        let side = |verdicts: usize, reason: Option<&Vec<u8>>| verdicts + reason.map_or(0, Vec::capacity);
        side(
            self.verdicts.as_ref().map_or(0, Value::heap_bytes),
            self.verdicts.as_ref().err(),
        ) + side(Value::heap_bytes(&self.presentations), self.presentations_unrecorded.as_ref())
    }
    fn try_clone(&self) -> io::Result<Self> {
        Ok(Self {
            verdicts: clone_outcome(&self.verdicts)?,
            presentations: Value::try_clone(&self.presentations)?,
            presentations_unrecorded: Value::try_clone(&self.presentations_unrecorded)?,
        })
    }
    fn encoded_bytes(&self) -> io::Result<usize> {
        Ok(outcome_bytes(&self.verdicts)?
            + Value::encoded_bytes(&self.presentations)?
            + Value::encoded_bytes(&self.presentations_unrecorded)?)
    }
    fn encode_reserved(&self, output: &mut Vec<u8>) -> io::Result<()> {
        encode_outcome(&self.verdicts, output)?;
        self.presentations.encode_reserved(output)?;
        self.presentations_unrecorded.encode_reserved(output)
    }
    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        Ok(Self {
            verdicts: decode_outcome(input)?,
            presentations: Value::decode(input)?,
            presentations_unrecorded: Value::decode(input)?,
        })
    }
}

crate::wire_value!(EventWindows, EventPresentation);
crate::opaque_value!(EventWindows);
crate::opaque_value!(Option<EventPresentation>);

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip<T: Value + PartialEq + std::fmt::Debug>(value: &T) {
        let mut bytes = Vec::new();
        value.encode(&mut bytes).unwrap();
        assert_eq!(bytes.len(), value.encoded_bytes().unwrap());
        let mut input = Decoder::new(&bytes);
        assert_eq!(&T::decode(&mut input).unwrap(), value);
        input.finish().unwrap();
    }

    #[test]
    fn windows_keep_withheld_fields_beside_recorded_ones() {
        let presentation = EventPresentation {
            native_hour: Some(1200),
            instance: 7,
            event_kind: 3,
            event_id: 41,
            actor: 9,
            rebuild: true,
            kind: WindowKind::News,
            shown: Some(false),
            option_count: 3,
            token: Recorded::Value(b"news.7".to_vec()),
            title: Recorded::Value(b"A title".to_vec()),
            description: Recorded::NotRecorded,
            picture: Recorded::Value(None),
            options: Recorded::Value(vec![
                OfferedOption {
                    ordinal: Some(2),
                    label: Some(b"Third".to_vec()),
                },
                OfferedOption {
                    ordinal: Some(3),
                    label: None,
                },
                OfferedOption {
                    ordinal: None,
                    label: Some(b"OK".to_vec()),
                },
            ]),
        };
        round_trip(&presentation);
        round_trip(&EventWindows {
            verdicts: Ok(vec![VerdictEntry {
                sequence: 11,
                verdict: WindowVerdict::HiddenOrAiControlled,
            }]),
            presentations: Vec::new(),
            presentations_unrecorded: Some(b"lost".to_vec()),
        });
        round_trip(&EventWindows {
            verdicts: Err(EventWindows::NOT_ARMED.to_vec()),
            presentations: vec![presentation],
            presentations_unrecorded: Some(b"an earlier batch was discarded".to_vec()),
        });
        for verdict in WindowVerdict::ALL {
            assert_eq!(WindowVerdict::from_code(u32::from(verdict.code())), Some(verdict));
        }
        assert_eq!(WindowVerdict::from_code(0), None);
        assert_eq!(WindowVerdict::from_code(10), None);
    }
}
