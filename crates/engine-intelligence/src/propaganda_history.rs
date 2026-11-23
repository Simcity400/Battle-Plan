
use engine_sites::Rva;
use engine_native_memory::{Memory, memory::Raw, read};

pub const HISTORY: usize = 0xb8;
pub const WEEKLY_DRIFT: usize = 0x130;
pub const EFFORT_STRIDE: usize = 0x28;
pub const DAILY_UPDATE_RVA: Rva = Rva(0x0128_ece0);
pub const RESET_RVA: Rva = Rva(0x0128_cf40);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Effort {
    pub stability: i64,
    pub war_support: i64,
}

impl Effort {
    pub fn wrapping_add(self, other: Self) -> Self {
        Self {
            stability: self.stability.wrapping_add(other.stability),
            war_support: self.war_support.wrapping_add(other.war_support),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ring {
    pub data: usize,
    pub capacity: u32,
    pub begin: u32,
    pub end: u32,
}

impl Ring {
    pub fn read(memory: &impl Memory, object: usize) -> Result<Self, &'static str> {
        let bytes = field::<[u8; 20]>(memory, object, HISTORY)?;
        let data = usize::try_from(u64::from_le_bytes(bytes[..8].try_into().unwrap()))
            .map_err(|_| "propaganda history pointer overflow")?;
        let capacity = i32::from_le_bytes(bytes[8..12].try_into().unwrap());
        let begin = i32::from_le_bytes(bytes[12..16].try_into().unwrap());
        let end = i32::from_le_bytes(bytes[16..20].try_into().unwrap());
        if capacity < 1 || begin < 0 || end < 0 || begin >= capacity || end >= capacity {
            return Err("propaganda history ring indices invalid");
        }
        let span = (capacity as usize)
            .checked_mul(EFFORT_STRIDE)
            .and_then(|bytes| data.checked_add(bytes));
        if data == 0 || span.is_none() {
            return Err("propaganda history ring span invalid");
        }
        Ok(Self {
            data,
            capacity: capacity as u32,
            begin: begin as u32,
            end: end as u32,
        })
    }

    pub fn len(self) -> u32 {
        if self.end >= self.begin {
            self.end - self.begin
        } else {
            self.capacity - self.begin + self.end
        }
    }

    pub fn is_empty(self) -> bool {
        self.begin == self.end
    }

    pub fn effort(self, memory: &impl Memory, ordinal: u32) -> Result<Effort, &'static str> {
        if ordinal >= self.len() {
            return Err("propaganda history ordinal outside logical ring");
        }
        let index = (u64::from(self.begin) + u64::from(ordinal)) % u64::from(self.capacity);
        let address = usize::try_from(index)
            .ok()
            .and_then(|index| index.checked_mul(EFFORT_STRIDE))
            .and_then(|offset| self.data.checked_add(offset))
            .ok_or("propaganda history effort address overflow")?;
        read_effort(memory, address)
    }

    pub fn newest(self, memory: &impl Memory) -> Result<Effort, &'static str> {
        self.effort(
            memory,
            self.len()
                .checked_sub(1)
                .ok_or("propaganda daily append left an empty history")?,
        )
    }
}

pub fn weekly_drift(memory: &impl Memory, object: usize) -> Result<Effort, &'static str> {
    read_effort(
        memory,
        object
            .checked_add(WEEKLY_DRIFT)
            .ok_or("propaganda weekly drift address overflow")?,
    )
}

fn read_effort(memory: &impl Memory, address: usize) -> Result<Effort, &'static str> {
    let bytes = read::<[u8; 16]>(memory, address).ok_or("propaganda effort unreadable")?;
    Ok(Effort {
        stability: i64::from_le_bytes(bytes[..8].try_into().unwrap()),
        war_support: i64::from_le_bytes(bytes[8..].try_into().unwrap()),
    })
}

fn field<T: Raw>(memory: &impl Memory, object: usize, offset: usize) -> Result<T, &'static str> {
    object
        .checked_add(offset)
        .and_then(|address| read(memory, address))
        .ok_or("propaganda history header unreadable")
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native::ccountry::synthetic::World;

    #[test]
    fn propaganda_ring_reads_logical_order_and_only_saved_channels() {
        let mut world = World::new(0, 1, 0x200);
        let object = world.alloc(0x200);
        let data = world.alloc(4 * EFFORT_STRIDE);
        world.u64(object + HISTORY, data as u64);
        world.u32(object + HISTORY + 8, 4);
        world.u32(object + HISTORY + 12, 3);
        world.u32(object + HISTORY + 16, 2);
        for (slot, stability, war_support) in [(3, -7_i64, 13_i64), (0, 17, -19), (1, 23, 29)] {
            world.u64(data + slot * EFFORT_STRIDE, stability as u64);
            world.u64(data + slot * EFFORT_STRIDE + 8, war_support as u64);
        }
        struct SavedOnly<'a> {
            world: &'a World,
            data: usize,
        }
        impl Memory for SavedOnly<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                if address >= self.data && address < self.data + 4 * EFFORT_STRIDE {
                    let offset = address - self.data;
                    if offset / EFFORT_STRIDE == 2
                        || !offset.is_multiple_of(EFFORT_STRIDE)
                        || output.len() != 16
                    {
                        return false;
                    }
                }
                self.world.memory().copy(address, output)
            }
        }
        let memory = SavedOnly {
            world: &world,
            data,
        };
        let ring = Ring::read(&memory, object).unwrap();
        assert_eq!(ring.len(), 3);
        assert_eq!(
            ring.effort(&memory, 0).unwrap(),
            Effort {
                stability: -7,
                war_support: 13
            }
        );
        assert_eq!(
            ring.effort(&memory, 1).unwrap(),
            Effort {
                stability: 17,
                war_support: -19
            }
        );
        assert_eq!(
            ring.newest(&memory).unwrap(),
            Effort {
                stability: 23,
                war_support: 29
            }
        );
        assert!(ring.effort(&memory, 3).is_err());
    }

    #[test]
    fn propaganda_ring_empty_does_not_read_stale_payload() {
        let mut world = World::new(0, 1, 0x200);
        let object = world.alloc(0x200);
        world.u64(object + HISTORY, (object + 0x200) as u64);
        world.u32(object + HISTORY + 8, 4097);
        world.u32(object + HISTORY + 12, 4096);
        world.u32(object + HISTORY + 16, 4096);
        let ring = Ring::read(&world.memory(), object).unwrap();
        assert!(ring.is_empty());
        assert!(ring.newest(&world.memory()).is_err());
        world.u32(object + HISTORY + 16, 4097);
        assert!(Ring::read(&world.memory(), object).is_err());
    }
}
