
use engine_native_memory::{Memory, cpdx_array::Array, read, walk::Outcome};
use engine_native::ccountry;
use crate::{cincoming_diplomatic_action, diplomatic_action_body};

pub const ACTIONS: usize = 0x08;
pub const CAPACITY: usize = 0x10;
pub const COUNT: usize = 0x14;
pub const OWNER: usize = 0x20;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub country: u32,
    pub ordinal: u32,
    pub handle: u64,
    pub status: u8,
    pub action_type: i32,
    pub actor: u32,
    pub original_actor: u32,
    pub recipient: u32,
    pub original_recipient: u32,
    pub hour: i32,
    pub last_command_hour: i32,
}

pub fn count(memory: &impl Memory, base: usize, game_state: usize) -> Outcome {
    walk_buffer(memory, base, game_state, engine_native_memory::walk::Buffer::Count)
}

pub fn walk(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut Vec<Row>,
) -> Outcome {
    walk_buffer(
        memory,
        base,
        game_state,
        engine_native_memory::walk::Buffer::Growing(output),
    )
}

pub fn walk_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut output: engine_native_memory::walk::Buffer<'_, Row>,
) -> Outcome {
    let mut len = 0_usize;
    let mut unread_rows = 0_u32;
    let result = (|| {
        let countries = ccountry::Reader::new(memory, base, game_state)?;
        let reader = Reader { memory, base };
        for slot in 1..countries.count() {
            let Some(country) = countries.at(slot)? else {
                continue;
            };
            let queue = reader.optional(country.pointer, ccountry::INCOMING_ACTION_QUEUE)?;
            if queue == 0 {
                continue;
            }
            let owner = reader.field::<u32>(queue, OWNER)?;
            if owner != country.id {
                return Err("incoming diplomatic action queue owner differs from its country");
            }
            let actions = queue
                .checked_add(ACTIONS)
                .and_then(|address| Array::read(memory, address, 8))
                .ok_or("incoming diplomatic action queue header unreadable or invalid")?;
            output.ensure(
                len.checked_add(actions.count as usize)
                    .ok_or("incoming diplomatic action row demand overflow")?,
            )?;
            for ordinal in 0..actions.count {
                let wrapper = actions
                    .element(ordinal, 8)
                    .and_then(|address| read::<u64>(memory, address))
                    .and_then(|value| usize::try_from(value).ok())
                    .ok_or("incoming diplomatic action pointer unreadable")?;
                if wrapper == 0 {
                    continue;
                }
                let row = reader.row(wrapper, owner, ordinal)?;
                if let Some(destination) = output.get_mut(len) {
                    *destination = row;
                    len += 1;
                } else {
                    unread_rows = unread_rows
                        .checked_add(1)
                        .ok_or("incoming diplomatic action row count overflow")?;
                }
            }
        }
        Ok(())
    })();
    match result {
        Err(reason) => Outcome::Unavailable { reason },
        Ok(()) if unread_rows == 0 => Outcome::Rows { len },
        Ok(()) => Outcome::Truncated { len, unread_rows },
    }
}

struct Reader<'a, M> {
    memory: &'a M,
    base: usize,
}

impl<M: Memory> Reader<'_, M> {
    fn field<T: engine_native_memory::memory::Raw>(
        &self,
        pointer: usize,
        offset: usize,
    ) -> Result<T, &'static str> {
        pointer
            .checked_add(offset)
            .and_then(|address| read(self.memory, address))
            .ok_or("incoming diplomatic action field unreadable or address overflow")
    }

    fn optional(&self, pointer: usize, offset: usize) -> Result<usize, &'static str> {
        usize::try_from(self.field::<u64>(pointer, offset)?)
            .map_err(|_| "incoming diplomatic action pointer overflow")
    }

    fn row(&self, wrapper: usize, owner: u32, ordinal: u32) -> Result<Row, &'static str> {
        let header = read::<[u8; 32]>(self.memory, wrapper)
            .ok_or("incoming diplomatic action identity unreadable")?;
        let expected = engine_sites::at(self.base, cincoming_diplomatic_action::VTABLE_RVA)
            .ok_or("module address overflow")? as u64;
        if u64::from_le_bytes(header[..8].try_into().unwrap()) != expected {
            return Err("incoming diplomatic action vtable mismatch");
        }
        let handle = u64::from_le_bytes(
            header[cincoming_diplomatic_action::REFERENCE
                ..cincoming_diplomatic_action::REFERENCE + 8]
                .try_into()
                .unwrap(),
        );
        if handle as u32 != cincoming_diplomatic_action::REFERENCE_TYPE {
            return Err("incoming diplomatic action reference type mismatch");
        }
        let status = header[cincoming_diplomatic_action::STATUS_BYTE];
        let body = usize::try_from(u64::from_le_bytes(
            header[cincoming_diplomatic_action::BODY..cincoming_diplomatic_action::BODY + 8]
                .try_into()
                .unwrap(),
        ))
        .map_err(|_| "incoming diplomatic action body pointer overflow")?;
        if body == 0 {
            return Err("queued incoming diplomatic action has a null body");
        }
        Ok(Row {
            country: owner,
            ordinal,
            handle,
            status,
            action_type: self.field::<i32>(body, diplomatic_action_body::ACTION_TYPE)?,
            actor: self.field::<u32>(body, diplomatic_action_body::ACTOR)?,
            original_actor: self.field::<u32>(body, diplomatic_action_body::ORIGINAL_ACTOR)?,
            recipient: self.field::<u32>(body, diplomatic_action_body::RECIPIENT)?,
            original_recipient: self
                .field::<u32>(body, diplomatic_action_body::ORIGINAL_RECIPIENT)?,
            hour: self.field::<i32>(body, diplomatic_action_body::HOUR)?,
            last_command_hour: self
                .field::<i32>(body, diplomatic_action_body::LAST_COMMAND_HOUR)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native::cgame_state;
    use engine_native_memory::memory::SliceMemory;

    const GS: usize = 0x1000;
    const COUNTRIES: usize = 0x2000;
    const LINKS: usize = 0x2100;
    const C1: usize = 0x3000;
    const QUEUE: usize = 0x4000;
    const SLOTS: usize = 0x4100;
    const W1: usize = 0x5000;
    const W2: usize = 0x5100;
    const B1: usize = 0x6000;
    const B2: usize = 0x6100;

    fn p32(bytes: &mut [u8], at: usize, value: u32) {
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    fn p64(bytes: &mut [u8], at: usize, value: u64) {
        bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }
    fn vector(bytes: &mut [u8], at: usize, data: usize, count: u32) {
        p64(bytes, at, data as u64);
        p32(bytes, at + 8, count);
        p32(bytes, at + 12, count);
    }
    fn handle(id: u32, kind: u32) -> u64 {
        (u64::from(id) << 32) | u64::from(kind)
    }

    fn fixture() -> Vec<u8> {
        let mut bytes = vec![0; 0x7000];
        vector(&mut bytes, GS + cgame_state::COUNTRIES, COUNTRIES, 2);
        vector(&mut bytes, GS + cgame_state::COUNTRY_LINKS, LINKS, 2);
        p64(&mut bytes, COUNTRIES, u64::MAX);
        p64(&mut bytes, COUNTRIES + 8, C1 as u64);
        p32(&mut bytes, LINKS, 0);
        p32(&mut bytes, LINKS + 4, 1);
        p64(&mut bytes, C1, ccountry::VTABLE_RVA.0 as u64);
        p32(&mut bytes, C1 + ccountry::ID, 1);
        p64(
            &mut bytes,
            C1 + ccountry::INCOMING_ACTION_QUEUE,
            QUEUE as u64,
        );
        vector(&mut bytes, QUEUE + ACTIONS, SLOTS, 3);
        p32(&mut bytes, QUEUE + OWNER, 1);
        p64(&mut bytes, SLOTS, W1 as u64);
        p64(&mut bytes, SLOTS + 8, 0);
        p64(&mut bytes, SLOTS + 16, W2 as u64);
        for (wrapper, id, body, status) in [(W1, 63179_u32, B1, 0_u8), (W2, 65511, B2, 0xff)] {
            p64(
                &mut bytes,
                wrapper,
                cincoming_diplomatic_action::VTABLE_RVA.0 as u64,
            );
            p64(
                &mut bytes,
                wrapper + cincoming_diplomatic_action::REFERENCE,
                handle(id, cincoming_diplomatic_action::REFERENCE_TYPE),
            );
            bytes[wrapper + cincoming_diplomatic_action::STATUS_BYTE] = status;
            p64(
                &mut bytes,
                wrapper + cincoming_diplomatic_action::BODY,
                body as u64,
            );
        }
        for (body, actor) in [(B1, 11_u32), (B2, 13)] {
            p32(&mut bytes, body + diplomatic_action_body::ACTION_TYPE, 0);
            p32(&mut bytes, body + diplomatic_action_body::ACTOR, actor);
            p32(
                &mut bytes,
                body + diplomatic_action_body::ORIGINAL_ACTOR,
                actor,
            );
            p32(&mut bytes, body + diplomatic_action_body::RECIPIENT, 1);
            p32(
                &mut bytes,
                body + diplomatic_action_body::ORIGINAL_RECIPIENT,
                1,
            );
            p32(&mut bytes, body + diplomatic_action_body::HOUR, 60_818_944);
            p32(
                &mut bytes,
                body + diplomatic_action_body::LAST_COMMAND_HOUR,
                60_818_945,
            );
        }
        bytes
    }

    fn run(bytes: &[u8], capacity: usize) -> (Outcome, Vec<Row>) {
        let mut rows = vec![Row::default(); capacity];
        let outcome = walk(&SliceMemory { base: 0, bytes }, 0, GS, &mut rows);
        (outcome, rows)
    }

    #[test]
    fn growing_queue_preserves_occurrences_when_the_same_country_gains_slots() {
        let mut bytes = fixture();
        let mut rows = Vec::new();
        let expected = run(&bytes, 3);
        assert_eq!(
            walk(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes
                },
                0,
                GS,
                &mut rows
            ),
            expected.0
        );
        assert_eq!(&rows[..expected.0.len()], &expected.1[..expected.0.len()]);
        const COUNT: usize = 9;
        const DATA: usize = 0x10000;
        bytes.resize(DATA + COUNT * 8, 0);
        vector(&mut bytes, QUEUE + ACTIONS, DATA, COUNT as u32);
        for index in 0..COUNT {
            p64(
                &mut bytes,
                DATA + index * 8,
                if index % 3 == 1 { 0 } else { W1 as u64 },
            );
        }
        let expected = run(&bytes, COUNT);
        assert_eq!(expected.0, Outcome::Rows { len: 6 });
        assert_eq!(
            walk(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes
                },
                0,
                GS,
                &mut rows
            ),
            expected.0
        );
        assert_eq!(&rows[..expected.0.len()], &expected.1[..expected.0.len()]);
        assert_eq!(rows[expected.0.len() - 1].ordinal, 8);
    }

    #[test]
    fn native_queue_indices_survive_a_null_slot_and_every_raw_field_is_kept() {
        let bytes = fixture();
        let (outcome, rows) = run(&bytes, 4);
        assert_eq!(outcome, Outcome::Rows { len: 2 });
        assert_eq!(
            rows[0],
            Row {
                country: 1,
                ordinal: 0,
                handle: handle(63179, cincoming_diplomatic_action::REFERENCE_TYPE),
                status: 0,
                action_type: 0,
                actor: 11,
                original_actor: 11,
                recipient: 1,
                original_recipient: 1,
                hour: 60_818_944,
                last_command_hour: 60_818_945,
            }
        );
        assert_eq!(rows[1].ordinal, 2);
        assert_eq!(rows[1].status, 0xff);
    }

    #[test]
    fn a_missing_queue_is_absence_while_a_broken_wrapper_or_body_fails() {
        let mut bytes = fixture();
        p64(&mut bytes, C1 + ccountry::INCOMING_ACTION_QUEUE, 0);
        assert_eq!(run(&bytes, 4).0, Outcome::Rows { len: 0 });

        let mut bytes = fixture();
        p64(&mut bytes, W1, 1);
        assert_eq!(
            run(&bytes, 4).0,
            Outcome::Unavailable {
                reason: "incoming diplomatic action vtable mismatch"
            }
        );

        let mut bytes = fixture();
        p64(
            &mut bytes,
            W1 + cincoming_diplomatic_action::REFERENCE,
            handle(63179, 53),
        );
        assert_eq!(
            run(&bytes, 4).0,
            Outcome::Unavailable {
                reason: "incoming diplomatic action reference type mismatch"
            }
        );

        let mut bytes = fixture();
        p64(&mut bytes, W1 + cincoming_diplomatic_action::BODY, 0);
        assert_eq!(
            run(&bytes, 4).0,
            Outcome::Unavailable {
                reason: "queued incoming diplomatic action has a null body"
            }
        );

        let mut bytes = fixture();
        p32(&mut bytes, QUEUE + COUNT, 9);
        assert!(matches!(run(&bytes, 4).0, Outcome::Unavailable { .. }));

        let mut bytes = fixture();
        p32(&mut bytes, QUEUE + OWNER, 2);
        assert_eq!(
            run(&bytes, 4).0,
            Outcome::Unavailable {
                reason: "incoming diplomatic action queue owner differs from its country"
            }
        );
    }

    #[test]
    fn the_count_probe_counts_every_action() {
        let bytes = fixture();
        assert_eq!(
            count(&SliceMemory { base: 0, bytes: &bytes }, 0, GS),
            Outcome::Truncated {
                len: 0,
                unread_rows: 2
            }
        );
    }

    #[test]
    fn an_unreadable_world_yields_no_prefix() {
        struct Unreadable;
        impl Memory for Unreadable {
            fn copy(&self, _: usize, _: &mut [u8]) -> bool {
                false
            }
        }
        let mut rows = vec![Row::default(); 2];
        assert!(matches!(
            walk(&Unreadable, 0, GS, &mut rows),
            Outcome::Unavailable { .. }
        ));
    }
}
