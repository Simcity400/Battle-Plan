
pub const STRIDE: usize = 0x38;
pub const ROLE: usize = 0x00;
pub const SIZE: usize = 0x04;
pub const KIND: usize = 0x0c;
pub const IDENTITY: usize = 0x10;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row<T = engine_native_memory::msvc_string::Text<256>> {
    pub category: u32,
    pub size: u32,
    pub is_set: u8,
    pub kind: u32,
    pub identity: T,
}
pub fn read(
    memory: &impl engine_native_memory::Memory,
    pointer: usize,
    output: &mut Row,
) -> Result<(), &'static str> {
    read_buffer(memory, pointer, output)
}

pub(crate) fn read_buffer<T: engine_native_memory::capture_text::Value>(
    memory: &impl engine_native_memory::Memory,
    pointer: usize,
    output: &mut Row<T>,
) -> Result<(), &'static str> {
    use engine_native::cnaval_combatant::field;
    use engine_native_memory::msvc_string;
    let mut header = [0; 16];
    output.identity = if pointer != 0
        && pointer
            .checked_add(IDENTITY + msvc_string::STRIDE)
            .is_some()
    {
        let address = pointer + IDENTITY;
        let mut length = [0; 8];
        let mut capacity = [0; 8];
        match memory.copy_scattered_prefetched(
            [
                pointer,
                address + msvc_string::LENGTH,
                address + msvc_string::CAPACITY,
            ],
            &mut [&mut header, &mut length, &mut capacity],
        ) {
            0 => return Err("naval field unreadable"),
            3 => T::from_layout(
                memory,
                address,
                u64::from_le_bytes(length),
                u64::from_le_bytes(capacity),
            ),
            _ => Err("portrait identity layout unreadable"),
        }
    } else {
        header = field::<[u8; 16]>(memory, pointer, 0)?;
        T::read(
            memory,
            pointer
                .checked_add(IDENTITY)
                .ok_or("portrait identity address overflow")?,
            true,
        )
    }
    .map_err(|_| {
        if T::GROWING {
            "portrait identity invalid"
        } else {
            "portrait identity invalid or exceeds256 bytes"
        }
    })?;
    head(header, output);
    Ok(())
}

fn head<T>(header: [u8; 16], output: &mut Row<T>) {
    output.category = u32::from_le_bytes(header[..4].try_into().unwrap());
    output.size = u32::from_le_bytes(header[4..8].try_into().unwrap());
    output.is_set = header[8];
    output.kind = u32::from_le_bytes(header[12..16].try_into().unwrap());
}

pub(crate) fn from_image<T: engine_native_memory::capture_text::Value>(
    holder: &crate::portrait_cache::HolderImage<'_>,
    index: usize,
) -> Result<Row<T>, &'static str> {
    let mut row = Row {
        identity: T::from_image(holder.identity(index)?)?,
        ..Row::default()
    };
    head(holder.entries()[index].head, &mut row);
    Ok(row)
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native_memory::{Memory, memory::SliceMemory, msvc_string};
    use std::cell::RefCell;

    struct Traced<'a> {
        memory: SliceMemory<'a>,
        fields: RefCell<Vec<(usize, usize)>>,
        fault: Option<usize>,
    }
    impl Memory for Traced<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            let ordinal = self.fields.borrow().len();
            self.fields.borrow_mut().push((address, output.len()));
            if self.fault == Some(ordinal) {
                let prefix = output.len() / 2;
                self.memory.copy(address, &mut output[..prefix]);
                return false;
            }
            self.memory.copy(address, output)
        }
    }

    fn ordered(memory: &impl Memory, pointer: usize) -> Result<Row, &'static str> {
        let header = engine_native::cnaval_combatant::field::<[u8; 16]>(memory, pointer, 0)?;
        let identity = msvc_string::read_text_prefetched(
            memory,
            pointer
                .checked_add(IDENTITY)
                .ok_or("portrait identity address overflow")?,
        )
        .ok_or("portrait identity invalid or exceeds256 bytes")?;
        Ok(Row {
            category: u32::from_le_bytes(header[..4].try_into().unwrap()),
            size: u32::from_le_bytes(header[4..8].try_into().unwrap()),
            is_set: header[8],
            kind: u32::from_le_bytes(header[12..16].try_into().unwrap()),
            identity,
        })
    }

    fn observed(memory: &impl Memory, pointer: usize) -> Result<Row, &'static str> {
        let mut output = Row::default();
        read(memory, pointer, &mut output)?;
        Ok(output)
    }

    #[test]
    fn descriptor_batch_preserves_overflow_and_partial_fault_prefixes() {
        for pointer in [
            0,
            0x1000,
            usize::MAX - 64,
            usize::MAX - 48,
            usize::MAX - 47,
            usize::MAX - 40,
            usize::MAX - 32,
            usize::MAX - 24,
            usize::MAX - 16,
            usize::MAX - 15,
            usize::MAX,
        ] {
            for (len, capacity) in [
                (0_u64, 15_u64),
                (3, 15),
                (0, 31),
                (3, 31),
                (16, 15),
                (257, 511),
            ] {
                let mut bytes = [0; 512];
                bytes[..16].copy_from_slice(&[1, 0, 0, 0, 1, 0, 0, 0, 255, 0, 0, 0, 1, 0, 0, 0]);
                bytes[IDENTITY + msvc_string::LENGTH..IDENTITY + msvc_string::LENGTH + 8]
                    .copy_from_slice(&len.to_le_bytes());
                bytes[IDENTITY + msvc_string::CAPACITY..IDENTITY + msvc_string::CAPACITY + 8]
                    .copy_from_slice(&capacity.to_le_bytes());
                let source = if capacity == msvc_string::INLINE_CAPACITY {
                    IDENTITY
                } else {
                    bytes[IDENTITY..IDENTITY + 8].copy_from_slice(
                        &(pointer.checked_add(64).unwrap_or(0) as u64).to_le_bytes(),
                    );
                    64
                };
                if len == 3 {
                    bytes[source..source + 3].copy_from_slice(&[0xff, 0, b'a']);
                }
                let observe = |fault| Traced {
                    memory: SliceMemory {
                        base: pointer,
                        bytes: &bytes,
                    },
                    fields: RefCell::new(Vec::new()),
                    fault,
                };
                let before = observe(None);
                let after = observe(None);
                assert_eq!(observed(&after, pointer), ordered(&before, pointer));
                assert_eq!(*after.fields.borrow(), *before.fields.borrow());
                let fields = before.fields.borrow().clone();
                for ordinal in 0..fields.len() {
                    let before = observe(Some(ordinal));
                    let after = observe(Some(ordinal));
                    assert_eq!(observed(&after, pointer), ordered(&before, pointer));
                    assert_eq!(*after.fields.borrow(), fields[..=ordinal]);
                    assert_eq!(*after.fields.borrow(), *before.fields.borrow());
                }
            }
        }
    }
}
