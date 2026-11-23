use super::{Actor, Batch, Category, Event, Kind, Origin, Span, invalid};
use replay_data::codec::{Decoder, put_bytes, put_string, put_varint};
use std::io;

pub(super) fn encode_batch(batch: &Batch, from: u64, to: u64) -> io::Result<Vec<u8>> {
    let mut names = engine_allocation::mul(batch.departures.len(), 20)?;
    for kind in &batch.kinds {
        names = engine_allocation::add(names, engine_allocation::add(kind.name.len(), 32)?)?;
    }
    for actor in &batch.actors {
        names = engine_allocation::add(
            names,
            engine_allocation::add(
                engine_allocation::add(
                    engine_allocation::add(actor.user.len(), actor.name.len())?,
                    actor.tag.len(),
                )?,
                64,
            )?,
        )?;
    }
    let mut out = engine_allocation::vec(
        engine_allocation::add(
            names,
            engine_allocation::add(
                engine_allocation::mul(batch.events.len(), 90)?,
                engine_allocation::mul(batch.spans.len(), 32)?,
            )?,
        )?
        .checked_add(64)
        .ok_or_else(engine_allocation::oom)?,
    )?;
    out.extend_from_slice(
        &u32::try_from(batch.kinds.len())
            .map_err(|_| invalid("too many command kinds"))?
            .to_le_bytes(),
    );
    for kind in &batch.kinds {
        put_varint(&mut out, kind.id.into());
        put_varint(&mut out, kind.token.into());
        out.push(kind.category as u8);
        put_string(&mut out, &kind.name)?;
    }
    out.extend_from_slice(
        &u32::try_from(batch.actors.len())
            .map_err(|_| invalid("too many command actors"))?
            .to_le_bytes(),
    );
    for actor in &batch.actors {
        put_varint(&mut out, actor.id.into());
        put_varint(&mut out, zigzag(i128::from(actor.player)));
        put_varint(&mut out, actor.country.into());
        put_varint(&mut out, actor.since_qpc.into());
        put_varint(&mut out, actor.at_qpc.into());
        put_bytes(&mut out, &actor.user)?;
        put_bytes(&mut out, &actor.name)?;
        put_bytes(&mut out, &actor.tag)?;
    }
    out.extend_from_slice(
        &u32::try_from(batch.departures.len())
            .map_err(|_| invalid("too many player departures"))?
            .to_le_bytes(),
    );
    for &(actor, qpc) in &batch.departures {
        put_varint(&mut out, actor.into());
        put_varint(&mut out, qpc.into());
    }
    out.extend_from_slice(
        &u32::try_from(batch.events.len())
            .map_err(|_| invalid("too many commands"))?
            .to_le_bytes(),
    );
    let mut time = from;
    for event in &batch.events {
        if event.qpc < time || event.qpc > to {
            return Err(invalid("command timestamps out of order"));
        }
        put_varint(&mut out, (event.qpc - time).into());
        time = event.qpc;
    }
    let events = &batch.events;
    column(
        &mut out,
        events.iter().enumerate().map(|(i, e)| {
            zigzag(i128::from(e.hour) - i.checked_sub(1).map_or(0, |p| i128::from(events[p].hour)))
        }),
    );
    column(&mut out, events.iter().map(|e| e.actor.into()));
    column(
        &mut out,
        events
            .iter()
            .map(|e| zigzag(i128::from(e.player) - i128::from(e.actor))),
    );
    column(&mut out, events.iter().map(|e| e.kind.into()));
    column(
        &mut out,
        events.iter().map(|e| {
            zigzag(e.command_country.map_or(0, |c| i128::from(c) + 1) - (i128::from(e.actor) + 1))
        }),
    );
    column(
        &mut out,
        events
            .iter()
            .map(|e| u128::from(e.origin as u8 | (u8::from(e.paused) << 2))),
    );
    column(
        &mut out,
        events.iter().enumerate().map(|(i, e)| {
            zigzag(
                i128::from(e.sequence)
                    - i.checked_sub(1)
                        .map_or(0, |p| i128::from(events[p].sequence)),
            )
        }),
    );
    out.extend_from_slice(
        &u32::try_from(batch.spans.len())
            .map_err(|_| invalid("too many command intervals"))?
            .to_le_bytes(),
    );
    for span in &batch.spans {
        if span.from_qpc < from || span.to_qpc > to || span.from_qpc > span.to_qpc {
            return Err(invalid("invalid command coverage interval"));
        }
        put_varint(&mut out, (span.to_qpc - span.from_qpc).into());
    }
    let spans = &batch.spans;
    column(
        &mut out,
        spans.iter().enumerate().map(|(i, s)| {
            zigzag(
                i128::from(s.from_qpc)
                    - i128::from(i.checked_sub(1).map_or(from, |p| spans[p].to_qpc)),
            )
        }),
    );
    column(
        &mut out,
        spans
            .iter()
            .map(|s| u128::from(u8::from(s.complete) | (u8::from(s.paused) << 1))),
    );
    column(
        &mut out,
        spans.iter().enumerate().map(|(i, s)| {
            zigzag(
                i128::from(s.hour)
                    - i.checked_sub(1).map_or_else(
                        || events.last().map_or(0, |e| i128::from(e.hour)),
                        |p| i128::from(spans[p].hour),
                    ),
            )
        }),
    );
    Ok(out)
}

fn column(out: &mut Vec<u8>, values: impl Iterator<Item = u128> + Clone) {
    let Some(first) = values.clone().next() else {
        return;
    };
    let constant = values.clone().all(|v| v == first);
    out.push(u8::from(!constant));
    if constant {
        put_varint(out, first);
    } else {
        for value in values {
            put_varint(out, value);
        }
    }
}

pub(super) fn decode_batch(bytes: &[u8], from: u64, to: u64) -> io::Result<Batch> {
    let mut input = Decoder::new(bytes);
    let mut batch = Batch::default();
    let count = input.count(7)?;
    batch.kinds = engine_allocation::vec(count)?;
    for _ in 0..count {
        batch.kinds.push(Kind {
            id: narrow(input.varint()?)?,
            token: narrow(input.varint()?)?,
            category: Category::read(input.u8()?)?,
            name: input.string()?,
        });
    }
    let count = input.count(17)?;
    batch.actors = engine_allocation::vec(count)?;
    for _ in 0..count {
        batch.actors.push(Actor {
            id: narrow(input.varint()?)?,
            player: narrow_signed(input.varint()?)?,
            country: narrow(input.varint()?)?,
            since_qpc: narrow(input.varint()?)?,
            at_qpc: narrow(input.varint()?)?,
            user: input.bytes()?,
            name: input.bytes()?,
            tag: input.bytes()?,
        });
    }
    let count = input.count(2)?;
    batch.departures = engine_allocation::vec(count)?;
    for _ in 0..count {
        batch
            .departures
            .push((narrow(input.varint()?)?, narrow(input.varint()?)?));
    }
    let count = input.count(1)?;
    batch.events = engine_allocation::vec(count)?;
    let mut time = from;
    for _ in 0..count {
        time = time
            .checked_add(narrow::<u64>(input.varint()?)?)
            .filter(|&t| t <= to)
            .ok_or_else(|| invalid("command timestamp overflow"))?;
        batch.events.push(Event {
            qpc: time,
            hour: 0,
            actor: 0,
            player: 0,
            kind: 0,
            command_country: None,
            origin: Origin::Unresolved,
            sequence: 0,
            paused: false,
        });
    }
    let mut hour = 0_i128;
    read_column(&mut input, &mut batch.events, |e, v| {
        hour = hour
            .checked_add(unzigzag(v))
            .ok_or_else(|| invalid("command hour overflow"))?;
        e.hour = signed(hour)?;
        Ok(())
    })?;
    read_column(&mut input, &mut batch.events, |e, v| {
        e.actor = narrow(v)?;
        Ok(())
    })?;
    read_column(&mut input, &mut batch.events, |e, v| {
        e.player = signed(
            i128::from(e.actor)
                .checked_add(unzigzag(v))
                .ok_or_else(|| invalid("command player overflow"))?,
        )?;
        Ok(())
    })?;
    read_column(&mut input, &mut batch.events, |e, v| {
        e.kind = narrow(v)?;
        Ok(())
    })?;
    read_column(&mut input, &mut batch.events, |e, v| {
        let target = (i128::from(e.actor) + 1)
            .checked_add(unzigzag(v))
            .ok_or_else(|| invalid("command country overflow"))?;
        e.command_country = if target == 0 {
            None
        } else {
            Some(signed(target - 1)?)
        };
        Ok(())
    })?;
    read_column(&mut input, &mut batch.events, |e, v| {
        let flags = narrow::<u8>(v)?;
        e.origin = match flags & !4 {
            0 => Origin::Unresolved,
            1 => Origin::PlayerOrder,
            2 => Origin::AutonomousCountry,
            _ => return Err(invalid("unknown command origin")),
        };
        e.paused = flags & 4 != 0;
        Ok(())
    })?;
    let mut sequence = 0_i128;
    read_column(&mut input, &mut batch.events, |e, v| {
        sequence = sequence
            .checked_add(unzigzag(v))
            .ok_or_else(|| invalid("command sequence overflow"))?;
        e.sequence = signed(sequence)?;
        Ok(())
    })?;
    let count = input.count(1)?;
    batch.spans = engine_allocation::vec(count)?;
    for _ in 0..count {
        batch.spans.push(Span {
            from_qpc: 0,
            to_qpc: narrow(input.varint()?)?,
            hour: 0,
            complete: false,
            paused: false,
        });
    }
    let mut previous_end = from;
    read_column(&mut input, &mut batch.spans, |s, v| {
        let start = i128::from(previous_end)
            .checked_add(unzigzag(v))
            .ok_or_else(|| invalid("command coverage overflow"))?;
        s.from_qpc = signed::<u64>(start)?;
        s.to_qpc = s
            .from_qpc
            .checked_add(s.to_qpc)
            .filter(|&t| t <= to)
            .ok_or_else(|| invalid("command coverage overflow"))?;
        if s.from_qpc < from {
            return Err(invalid("invalid command coverage interval"));
        }
        previous_end = s.to_qpc;
        Ok(())
    })?;
    read_column(&mut input, &mut batch.spans, |s, v| {
        let flags = narrow::<u8>(v)?;
        if flags & !3 != 0 {
            return Err(invalid("unknown command timing flags"));
        }
        s.complete = flags & 1 != 0;
        s.paused = flags & 2 != 0;
        Ok(())
    })?;
    read_column(&mut input, &mut batch.spans, |s, v| {
        hour = hour
            .checked_add(unzigzag(v))
            .ok_or_else(|| invalid("command hour overflow"))?;
        s.hour = signed(hour)?;
        Ok(())
    })?;
    input.finish()?;
    Ok(batch)
}

fn read_column<T>(
    input: &mut Decoder<'_>,
    items: &mut [T],
    mut set: impl FnMut(&mut T, u128) -> io::Result<()>,
) -> io::Result<()> {
    if items.is_empty() {
        return Ok(());
    }
    let variable = input.bool()?;
    let constant = if variable { 0 } else { input.varint()? };
    for item in items {
        set(item, if variable { input.varint()? } else { constant })?;
    }
    Ok(())
}

fn zigzag(value: i128) -> u128 {
    ((value << 1) ^ (value >> 127)) as u128
}
fn unzigzag(value: u128) -> i128 {
    (value >> 1) as i128 ^ -((value & 1) as i128)
}
pub(super) fn narrow<T: TryFrom<u128>>(value: u128) -> io::Result<T> {
    T::try_from(value).map_err(|_| invalid("command integer out of range"))
}
fn signed<T: TryFrom<i128>>(value: i128) -> io::Result<T> {
    T::try_from(value).map_err(|_| invalid("command integer out of range"))
}
fn narrow_signed(value: u128) -> io::Result<i32> {
    signed(unzigzag(value))
}
