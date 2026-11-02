use super::invalid;
use core_spool::{Budget, paged::PagedFile};
use std::{io, path::Path};

pub struct Payloads {
    pub pages: PagedFile,
    free: [u64; 64],
}
impl Payloads {
    pub fn create(path: &Path, allowance: u64, memory: &Budget, disk: &Budget) -> io::Result<Self> {
        Ok(Self {
            pages: PagedFile::create(path, allowance, memory, disk)?,
            free: [u64::MAX; 64],
        })
    }
    fn class(len: u64) -> io::Result<(usize, u64)> {
        let size = len
            .max(1)
            .checked_next_power_of_two()
            .ok_or_else(|| invalid("payload capacity overflow"))?;
        Ok((size.trailing_zeros() as usize, size))
    }
    pub fn allocate(&mut self, len: u64) -> io::Result<u64> {
        let (class, size) = Self::class(len)?;
        let at = if self.free[class] == u64::MAX {
            let at = self.pages.len();
            self.pages.grow(
                at.checked_add(16)
                    .and_then(|n| n.checked_add(size))
                    .ok_or_else(|| invalid("payload store overflow"))?,
            )?;
            at
        } else {
            let at = self.free[class];
            let mut next = [0; 8];
            self.pages.read(at, &mut next)?;
            self.free[class] = u64::from_le_bytes(next);
            at
        };
        self.pages.write(at, &1_u64.to_le_bytes())?;
        self.pages.write(at + 8, &len.to_le_bytes())?;
        Ok(at + 16)
    }
    pub fn retain(&mut self, offset: u64) -> io::Result<()> {
        let at = offset
            .checked_sub(16)
            .ok_or_else(|| invalid("payload reference underflow"))?;
        let mut count = [0; 8];
        self.pages.read(at, &mut count)?;
        let count = u64::from_le_bytes(count)
            .checked_add(1)
            .ok_or_else(|| invalid("payload reference overflow"))?;
        self.pages.write(at, &count.to_le_bytes())
    }
    pub fn release(&mut self, offset: u64) -> io::Result<()> {
        let at = offset
            .checked_sub(16)
            .ok_or_else(|| invalid("payload reference underflow"))?;
        let mut header = [0; 16];
        self.pages.read(at, &mut header)?;
        let count = u64::from_le_bytes(header[..8].try_into().unwrap())
            .checked_sub(1)
            .ok_or_else(|| invalid("payload reference underflow"))?;
        if count == 0 {
            let len = u64::from_le_bytes(header[8..].try_into().unwrap());
            let (class, _) = Self::class(len)?;
            self.pages.write(at, &self.free[class].to_le_bytes())?;
            self.free[class] = at;
        } else {
            self.pages.write(at, &count.to_le_bytes())?;
        }
        Ok(())
    }
}
