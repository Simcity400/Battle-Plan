use crate::{Memory, msvc_string, raw_country_observations as raw, read};

pub trait Value: Default + Clone + std::fmt::Debug + PartialEq + Eq {
    const GROWING: bool;
    fn as_bytes(&self) -> &[u8];
    fn allocated_bytes(&self) -> usize {
        0
    }
    fn copy_owned(&self) -> Result<Self, &'static str> {
        Ok(self.clone())
    }
    fn read(memory: &impl Memory, address: usize, prefetched: bool) -> Result<Self, &'static str>;
    fn from_image(bytes: &[u8]) -> Result<Self, &'static str>;
    fn read_prepared(&mut self, memory: &impl Memory, address: usize) -> Result<(), &'static str> {
        *self = Self::read(memory, address, false)?;
        Ok(())
    }
    fn from_layout(
        memory: &impl Memory,
        address: usize,
        len: u64,
        capacity: u64,
    ) -> Result<Self, &'static str>;
    fn observed(
        memory: &impl Memory,
        address: usize,
        raw: i32,
        source: raw::Source,
        journal: &mut raw::Journal,
    ) -> Result<Self, &'static str>;
}

impl<const N: usize> Value for msvc_string::Text<N> {
    const GROWING: bool = false;
    fn as_bytes(&self) -> &[u8] {
        self.as_bytes()
    }
    fn read(memory: &impl Memory, address: usize, prefetched: bool) -> Result<Self, &'static str> {
        if prefetched {
            msvc_string::read_text_prefetched(memory, address)
        } else {
            msvc_string::read_text(memory, address)
        }
        .ok_or("string invalid or outside fixed buffer")
    }
    fn from_image(bytes: &[u8]) -> Result<Self, &'static str> {
        let mut value = Self::default();
        value
            .bytes
            .get_mut(..bytes.len())
            .ok_or("string invalid or outside fixed buffer")?
            .copy_from_slice(bytes);
        value.len = bytes.len();
        Ok(value)
    }
    fn from_layout(
        memory: &impl Memory,
        address: usize,
        len: u64,
        capacity: u64,
    ) -> Result<Self, &'static str> {
        let mut value = Self::default();
        msvc_string::read_text_with_layout_into(memory, address, len, capacity, &mut value)
            .ok_or("string invalid or outside fixed buffer")?;
        Ok(value)
    }
    fn observed(
        memory: &impl Memory,
        address: usize,
        raw: i32,
        source: raw::Source,
        journal: &mut raw::Journal,
    ) -> Result<Self, &'static str> {
        msvc_string::read_text_observed::<N, false>(memory, address, raw, source, journal)
            .ok_or("string invalid or outside fixed buffer")
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Owned {
    pub bytes: Vec<u8>,
    pub len: usize,
}
impl Owned {
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
    pub fn allocated_bytes(&self) -> usize {
        self.bytes.capacity()
    }
    pub fn prepare(&mut self, len: usize) -> std::io::Result<()> {
        if len > self.bytes.len() {
            let additional = len - self.bytes.len();
            engine_allocation::reserve(&mut self.bytes, additional)?;
            self.bytes.resize(len, 0);
        }
        Ok(())
    }
    pub fn copy_prepared(&mut self, bytes: &[u8]) -> Result<(), &'static str> {
        let target = self
            .bytes
            .get_mut(..bytes.len())
            .ok_or("string exceeds prepared native length")?;
        target.copy_from_slice(bytes);
        self.len = bytes.len();
        Ok(())
    }
    pub fn read_hint(
        memory: &impl Memory,
        address: usize,
        prefetched: bool,
    ) -> Result<Self, &'static str> {
        Self::read_with_policy::<false>(memory, address, prefetched)
    }
    fn read_with_policy<const TERMINAL: bool>(
        memory: &impl Memory,
        address: usize,
        prefetched: bool,
    ) -> Result<Self, &'static str> {
        let length_address = address
            .checked_add(msvc_string::LENGTH)
            .ok_or("string length address overflow")?;
        let capacity_address = address.checked_add(msvc_string::CAPACITY);
        let (len, capacity) = if prefetched
            && length_address.checked_add(8).is_some()
            && capacity_address.is_some_and(|at| at.checked_add(8).is_some())
        {
            let mut length = [0; 8];
            let mut capacity = [0; 8];
            if memory.copy_scattered_prefetched(
                [length_address, capacity_address.unwrap()],
                &mut [&mut length[..], &mut capacity[..]],
            ) != 2
            {
                return Err("string layout unreadable");
            }
            (u64::from_le_bytes(length), u64::from_le_bytes(capacity))
        } else {
            (
                read(memory, length_address).ok_or("string length unreadable")?,
                read(
                    memory,
                    capacity_address.ok_or("string capacity address overflow")?,
                )
                .ok_or("string capacity unreadable")?,
            )
        };
        Self::from_layout_with_policy::<TERMINAL>(memory, address, len, capacity)
    }
    fn from_layout_with_policy<const TERMINAL: bool>(
        memory: &impl Memory,
        address: usize,
        len: u64,
        capacity: u64,
    ) -> Result<Self, &'static str> {
        if len > capacity || capacity < msvc_string::INLINE_CAPACITY {
            return Err("string layout invalid");
        }
        let source = if capacity == msvc_string::INLINE_CAPACITY {
            address
        } else {
            usize::try_from(read::<u64>(memory, address).ok_or("string pointer unreadable")?)
                .map_err(|_| "string pointer overflow")?
        };
        let len = usize::try_from(len).map_err(|_| "string length overflow")?;
        if source
            .checked_add(len)
            .and_then(|at| read::<u8>(memory, at))
            != Some(0)
        {
            return Err("string terminator unreadable or invalid");
        }
        let mut value = Self::default();
        msvc_string::copy_with_policy::<TERMINAL>(
            memory,
            source,
            len,
            &mut value.bytes,
            0,
            "string bytes unreadable",
            "string allocation failed",
        )?;
        value.len = len;
        Ok(value)
    }
}
impl Value for Owned {
    const GROWING: bool = true;
    fn as_bytes(&self) -> &[u8] {
        self.as_bytes()
    }
    fn allocated_bytes(&self) -> usize {
        self.allocated_bytes()
    }
    fn copy_owned(&self) -> Result<Self, &'static str> {
        Ok(Self {
            bytes: engine_allocation::copy_vec(self.as_bytes())
                .map_err(|_| "string allocation failed")?,
            len: self.len,
        })
    }
    fn read(memory: &impl Memory, address: usize, prefetched: bool) -> Result<Self, &'static str> {
        Self::read_with_policy::<true>(memory, address, prefetched)
    }
    fn from_image(bytes: &[u8]) -> Result<Self, &'static str> {
        Ok(Self {
            bytes: engine_allocation::copy_vec(bytes).map_err(|_| "string allocation failed")?,
            len: bytes.len(),
        })
    }
    fn read_prepared(&mut self, memory: &impl Memory, address: usize) -> Result<(), &'static str> {
        let descriptor = msvc_string::Descriptor::read(memory, address)?;
        if descriptor.len > self.bytes.len() {
            return Err("string exceeds prepared native length");
        }
        descriptor.copy(memory, &mut self.bytes)?;
        self.len = descriptor.len;
        Ok(())
    }
    fn from_layout(
        memory: &impl Memory,
        address: usize,
        len: u64,
        capacity: u64,
    ) -> Result<Self, &'static str> {
        Self::from_layout_with_policy::<true>(memory, address, len, capacity)
    }
    fn observed(
        memory: &impl Memory,
        address: usize,
        reference: i32,
        source: raw::Source,
        journal: &mut raw::Journal,
    ) -> Result<Self, &'static str> {
        let index = journal.begin_text(reference, source, address);
        let mut attempt = raw::TextAttempt {
            address,
            ..Default::default()
        };
        let mut value = Self::default();
        let result = (|| {
            let length = address
                .checked_add(msvc_string::LENGTH)
                .and_then(|at| read::<u64>(memory, at))
                .ok_or("string length unreadable");
            attempt.length = raw::Attempt::Completed(length);
            let length = length?;
            let capacity = address
                .checked_add(msvc_string::CAPACITY)
                .and_then(|at| read::<u64>(memory, at))
                .ok_or("string capacity unreadable");
            attempt.capacity = raw::Attempt::Completed(capacity);
            let capacity = capacity?;
            if length > capacity || capacity < msvc_string::INLINE_CAPACITY {
                return Err("string layout invalid");
            }
            let pointer = if capacity == msvc_string::INLINE_CAPACITY {
                Ok(address)
            } else {
                read::<u64>(memory, address)
                    .and_then(|pointer| usize::try_from(pointer).ok())
                    .ok_or("string pointer unreadable")
            };
            attempt.source = raw::Attempt::Completed(pointer);
            let pointer = pointer?;
            let length = usize::try_from(length).map_err(|_| "string length overflow")?;
            let terminator = pointer
                .checked_add(length)
                .and_then(|at| read::<u8>(memory, at))
                .ok_or("string terminator unreadable");
            attempt.terminator = raw::Attempt::Completed(terminator);
            if terminator? != 0 {
                return Err("string terminator invalid");
            }
            let payload = msvc_string::copy_growing(
                memory,
                pointer,
                length,
                &mut value.bytes,
                0,
                "string bytes unreadable",
                "string allocation failed",
            );
            attempt.payload = raw::Attempt::Completed(payload.map(|()| raw::Span {
                start: 0,
                len: length,
            }));
            payload?;
            value.len = length;
            Ok(())
        })();
        attempt.failure = result.err();
        journal.finish_text(index, attempt, result.is_ok().then(|| value.as_bytes()));
        result.map(|()| value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::SliceMemory;
    use std::cell::RefCell;

    struct Traced<'a> {
        bytes: &'a [u8],
        reads: RefCell<Vec<(usize, usize)>>,
        fault: Option<usize>,
    }
    impl Memory for Traced<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            self.reads.borrow_mut().push((address, output.len()));
            !self.fault.is_some_and(|fault| {
                (address..address.saturating_add(output.len())).contains(&fault)
            }) && SliceMemory {
                base: 0x1000,
                bytes: self.bytes,
            }
            .copy(address, output)
        }
    }

    #[test]
    fn owned_strings_keep_fixed_protocol_and_grow_mod_sized_payloads_without_retry() {
        for len in [0, 1, 15, 256, 257, 4097, 5000] {
            let payload = if len <= 15 { 0 } else { 0x100 };
            let mut bytes = vec![0; 0x101 + len];
            bytes[payload..payload + len].fill(0xff);
            if len > 15 {
                bytes[..8].copy_from_slice(&0x1100_u64.to_le_bytes());
            }
            bytes[16..24].copy_from_slice(&(len as u64).to_le_bytes());
            bytes[24..32].copy_from_slice(&(if len <= 15 { 15 } else { len as u64 }).to_le_bytes());
            let growing = Traced {
                bytes: &bytes,
                reads: Default::default(),
                fault: None,
            };
            let owned = Owned::read(&growing, 0x1000, false).unwrap();
            assert_eq!(owned.as_bytes(), vec![0xff; len]);
            let hint = Traced {
                bytes: &bytes,
                reads: Default::default(),
                fault: None,
            };
            assert_eq!(Owned::read_hint(&hint, 0x1000, false).unwrap(), owned);
            assert_eq!(*hint.reads.borrow(), *growing.reads.borrow());
            for &(address, _) in &*growing.reads.borrow() {
                let capture_fault = Traced {
                    bytes: &bytes,
                    reads: Default::default(),
                    fault: Some(address),
                };
                let hint_fault = Traced {
                    bytes: &bytes,
                    reads: Default::default(),
                    fault: Some(address),
                };
                assert_eq!(
                    Owned::read_hint(&hint_fault, 0x1000, false),
                    Owned::read(&capture_fault, 0x1000, false),
                );
                assert_eq!(*hint_fault.reads.borrow(), *capture_fault.reads.borrow());
            }
            if len <= 256 {
                let fixed = Traced {
                    bytes: &bytes,
                    reads: Default::default(),
                    fault: None,
                };
                assert_eq!(
                    msvc_string::Text::<256>::read(&fixed, 0x1000, false)
                        .unwrap()
                        .as_bytes(),
                    owned.as_bytes()
                );
                assert_eq!(*growing.reads.borrow(), *fixed.reads.borrow());
            } else {
                assert!(msvc_string::Text::<256>::read(&growing, 0x1000, false).is_err());
            }
            let fault = Traced {
                bytes: &bytes,
                reads: Default::default(),
                fault: Some(0x1000 + payload + len),
            };
            assert!(Owned::read(&fault, 0x1000, false).is_err());
            assert_eq!(
                fault.reads.borrow().last(),
                Some(&(0x1000 + payload + len, 1))
            );
        }
    }
}
