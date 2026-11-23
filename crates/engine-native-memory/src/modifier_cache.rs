#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Value {
    pub raw: i64,
    pub present: bool,
}

#[derive(Default)]
pub struct PassDefinitions {
    slots: std::cell::RefCell<std::collections::HashMap<(usize, u32), Option<u32>>>,
}

impl PassDefinitions {
    pub fn clear(&mut self) {
        self.slots.get_mut().clear();
    }

    pub fn allocated_bytes(&self) -> usize {
        self.slots.borrow().capacity() * size_of::<((usize, u32), Option<u32>)>()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn cached_keys(&self) -> usize {
        self.slots.borrow().len()
    }

    pub fn flags(
        &self,
        base: usize,
        key: u32,
        read: impl FnOnce() -> Result<Option<u32>, &'static str>,
    ) -> Result<Option<u32>, &'static str> {
        if let Some(flags) = self.slots.borrow().get(&(base, key)) {
            return Ok(*flags);
        }
        let flags = read()?;
        let mut slots = self.slots.borrow_mut();
        engine_allocation::reserve_map(&mut slots, 1)
            .map_err(|_| "modifier pass memo allocation failed")?;
        slots.insert((base, key), flags);
        Ok(flags)
    }
}
