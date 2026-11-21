use super::invalid;
use std::io::{self, Read};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Event {
    pub hour: i32,
    pub kind: u8,
    pub table: u16,
    pub key: u128,
}

#[derive(Debug, PartialEq)]
pub enum Value<'a> {
    U8(u8),
    I8(i8),
    U16(u16),
    I16(i16),
    U32(u32),
    I32(i32),
    U64(u64),
    I64(i64),
    Bool(bool),
    OptionalI32(Option<i32>),
    OptionalU32(Option<u32>),
    OptionalI64(Option<i64>),
    OptionalU64(Option<u64>),
    Text(u32, Option<&'a [u8]>),
    U32List(Vec<u32>),
    U64List(Vec<u64>),
    Opaque(&'a [u8]),
}

pub trait Consumer {
    fn event(&mut self, event: Event);
    fn cell(&mut self, field: u16, recorded: bool, value: Value<'_>);
    fn status(
        &mut self,
        status: u8,
        missing: Option<u32>,
        reason: Option<&[u8]>,
        sample: Option<i32>,
    );
    fn visible_key(&mut self, key: u128);
    fn flags(&mut self, flags: u8);
}

#[cfg(test)]
pub struct Noop;
#[cfg(test)]
impl Consumer for Noop {
    fn event(&mut self, event: Event) {
        std::hint::black_box(event);
    }
    fn cell(&mut self, field: u16, recorded: bool, value: Value<'_>) {
        std::hint::black_box((field, recorded, value));
    }
    fn status(
        &mut self,
        status: u8,
        missing: Option<u32>,
        reason: Option<&[u8]>,
        sample: Option<i32>,
    ) {
        std::hint::black_box((status, missing, reason, sample));
    }
    fn visible_key(&mut self, key: u128) {
        std::hint::black_box(key);
    }
    fn flags(&mut self, flags: u8) {
        std::hint::black_box(flags);
    }
}

fn count(decoder: &mut replay_data::codec::Decoder<'_>, width: usize) -> io::Result<usize> {
    let count = usize::try_from(decoder.u64()?)
        .map_err(|_| invalid("Spool length exceeds address space"))?;
    if count > decoder.remaining() / width {
        return Err(invalid("Spool payload exceeds record"));
    }
    Ok(count)
}
fn bytes<'a>(decoder: &mut replay_data::codec::Decoder<'a>) -> io::Result<&'a [u8]> {
    let length = count(decoder, 1)?;
    decoder.take(length)
}
fn bit(decoder: &mut replay_data::codec::Decoder<'_>) -> io::Result<bool> {
    match decoder.u8()? {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(invalid("Invalid spool boolean")),
    }
}
fn decoded_value<'a>(decoder: &mut replay_data::codec::Decoder<'a>) -> io::Result<Value<'a>> {
    macro_rules! optional {
        ($variant:ident, $method:ident) => {
            Value::$variant(if bit(decoder)? {
                Some(decoder.$method()?)
            } else {
                None
            })
        };
    }
    Ok(match decoder.u8()? {
        0 => Value::U8(decoder.u8()?),
        1 => Value::I8(decoder.i8()?),
        2 => Value::U16(decoder.u16()?),
        3 => Value::I16(decoder.i16()?),
        4 => Value::U32(decoder.u32()?),
        5 => Value::I32(decoder.i32()?),
        6 => Value::U64(decoder.u64()?),
        7 => Value::I64(decoder.i64()?),
        8 => Value::Bool(bit(decoder)?),
        9 => optional!(OptionalI32, i32),
        10 => optional!(OptionalU32, u32),
        11 => optional!(OptionalI64, i64),
        12 => optional!(OptionalU64, u64),
        13 => {
            let id = decoder.u32()?;
            Value::Text(
                id,
                if bit(decoder)? {
                    Some(bytes(decoder)?)
                } else {
                    None
                },
            )
        }
        14 => {
            let n = count(decoder, 4)?;
            Value::U32List((0..n).map(|_| decoder.u32()).collect::<io::Result<_>>()?)
        }
        15 => {
            let n = count(decoder, 8)?;
            Value::U64List((0..n).map(|_| decoder.u64()).collect::<io::Result<_>>()?)
        }
        16 => Value::Opaque(bytes(decoder)?),
        _ => return Err(invalid("Invalid spool cell kind")),
    })
}

pub fn decode(record: &[u8], consumer: &mut impl Consumer) -> io::Result<()> {
    use replay_data::{codec::Decoder, rows::TableId};
    let mut decoder = Decoder::new(record);
    let event = Event {
        hour: decoder.i32()?,
        kind: decoder.u8()?,
        table: decoder.u16()?,
        key: decoder.u128()?,
    };
    let schema = if event.kind <= 4 {
        Some(
            TableId::ALL
                .into_iter()
                .find(|id| *id as u16 == event.table)
                .ok_or_else(|| invalid("Invalid spool table"))?
                .schema(),
        )
    } else {
        None
    };
    consumer.event(event);
    match event.kind {
        0 => {
            let status = decoder.u8()?;
            let (missing, reason) = match status {
                0 | 1 => (None, None),
                2 => (Some(decoder.u32()?), None),
                3 => (None, Some(bytes(&mut decoder)?)),
                _ => return Err(invalid("Invalid spool status")),
            };
            let sample = if bit(&mut decoder)? {
                Some(decoder.i32()?)
            } else {
                None
            };
            consumer.status(status, missing, reason, sample);
            let keys = count(&mut decoder, 16)?;
            for _ in 0..keys {
                consumer.visible_key(decoder.u128()?);
            }
        }
        1 | 3 => {
            let mut previous = None;
            while decoder.remaining() != 0 {
                let field = decoder.u16()?;
                if field as usize >= schema.unwrap().len() || previous.is_some_and(|p| p >= field) {
                    return Err(invalid("Invalid or unordered spool field"));
                }
                previous = Some(field);
                let recorded = bit(&mut decoder)?;
                consumer.cell(field, recorded, decoded_value(&mut decoder)?);
            }
        }
        2 | 4 => {}
        5 | 6 => consumer.flags(decoder.u8()?),
        _ => return Err(invalid("Invalid spool event kind")),
    }
    decoder.finish()
}

pub fn replay(
    mut reader: impl Read,
    consumer: &mut impl Consumer,
    memory: &core_spool::Budget,
) -> io::Result<(u64, u64)> {
    let (mut records, mut total) = (0, 0);
    let memory = &memory.owner(core_spool::Owner::GateDecoder);
    let mut buffer = Vec::new();
    let mut held = None;
    loop {
        let mut length = [0; 8];
        if reader.read(&mut length[..1])? == 0 {
            break;
        }
        reader.read_exact(&mut length[1..])?;
        let count = usize::try_from(u64::from_le_bytes(length))
            .map_err(|_| invalid("Spool record exceeds address space"))?;
        if count < 23 {
            return Err(invalid("Short spool record"));
        }
        if count > buffer.capacity() {
            let reservation = memory.reserve(
                (count as u64)
                    .checked_mul(3)
                    .ok_or_else(|| invalid("Gate decoder memory overflow"))?,
            )?;
            let mut next = Vec::new();
            next.try_reserve_exact(count).map_err(io::Error::other)?;
            if next.capacity() != count {
                return Err(invalid("Gate buffer exceeded reservation"));
            }
            buffer = next;
            held = Some(reservation);
        }
        buffer.resize(count, 0);
        reader.read_exact(&mut buffer)?;
        decode(&buffer, consumer)?;
        records += 1;
        total += count as u64 + 8;
    }
    drop(held);
    Ok((records, total))
}
