
use engine_native_memory::{Memory, cpdx_array::Array, read};
use crate::cstrategic_resource;
use engine_sites::Rva;

pub const SINGLETON_RVA: Rva = Rva(0x33dd4d0);
pub const ITEMS: usize = 0x28;
pub const ITEM_STRIDE: usize = 0x08;
pub const COUNT: usize = 0x34;
pub const KEYS: usize = 0x40;
pub const KEY_STRIDE: usize = 0x04;
pub const OIL: usize = 0x60;
pub const FUEL: usize = 0x68;
pub const CONSTRUCTOR_RVA: Rva = Rva(0xb13aa0);

pub(crate) struct Reader<'a, M: Memory> {
    memory: &'a M,
    base: usize,
    items: Array,
    keys: Array,
}

impl<'a, M: Memory> Reader<'a, M> {
    pub(crate) fn new(memory: &'a M, base: usize) -> Result<Self, &'static str> {
        let pointer = engine_sites::at(base, SINGLETON_RVA)
            .and_then(|p| read::<u64>(memory, p))
            .and_then(|p| usize::try_from(p).ok())
            .filter(|&p| p != 0)
            .ok_or("resource database pointer unreadable or null")?;
        let items = pointer
            .checked_add(ITEMS)
            .and_then(|p| Array::read(memory, p, ITEM_STRIDE))
            .ok_or("resource database items unreadable or invalid")?;
        let keys = pointer
            .checked_add(KEYS)
            .and_then(|p| Array::read(memory, p, KEY_STRIDE))
            .ok_or("resource database keys unreadable or invalid")?;
        if items.count == 0 || keys.count != items.count - 1 {
            return Err("resource database item and key counts disagree");
        }
        let null = items
            .element(0, ITEM_STRIDE)
            .and_then(|p| read::<u64>(memory, p));
        if null.is_none_or(|p| p == 0) {
            return Err("resource database has no null object");
        }
        Ok(Self {
            memory,
            base,
            items,
            keys,
        })
    }

    pub(crate) fn count(&self) -> u32 {
        self.items.count
    }

    pub(crate) fn token(&self, index: u32) -> Result<u32, &'static str> {
        index
            .checked_sub(1)
            .and_then(|i| self.keys.element(i, KEY_STRIDE))
            .and_then(|p| read::<u32>(self.memory, p))
            .ok_or("resource database token unreadable")
    }

    pub(crate) fn at(&self, index: u32) -> Result<(usize, cstrategic_resource::Row), &'static str> {
        let pointer = self
            .items
            .element(index, ITEM_STRIDE)
            .and_then(|p| read::<u64>(self.memory, p))
            .and_then(|p| usize::try_from(p).ok())
            .filter(|&p| p != 0)
            .ok_or("resource descriptor pointer unreadable or null")?;
        let row = cstrategic_resource::row(self.memory, self.base, pointer)?;
        if row.id != index || row.token != self.token(index)? {
            return Err("resource descriptor disagrees with database identity");
        }
        Ok((pointer, row))
    }
}
