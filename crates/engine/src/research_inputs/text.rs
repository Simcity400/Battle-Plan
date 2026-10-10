//! Research text retains fixed fixture reads and exact worker-owned payloads.
use crate::{Memory, allocation, msvc_string, read_slice};

#[derive(Debug, PartialEq, Eq)]
// Fixed readers retain allocation-free inline text; workers own longer payloads.
#[allow(clippy::large_enum_variant)]
pub enum Text {
    Inline(msvc_string::Text<512>),
    Owned(Vec<u8>),
}

impl Default for Text {
    fn default() -> Self {
        Self::Inline(msvc_string::Text::default())
    }
}

impl Clone for Text {
    fn clone(&self) -> Self {
        match self {
            Self::Inline(value) => Self::Inline(*value),
            Self::Owned(bytes) => Self::Owned(allocation::owned_vec(allocation::copy_vec(bytes))),
        }
    }
}

impl Text {
    pub fn allocated_bytes(&self) -> usize {
        match self {
            Self::Inline(_) => 0,
            Self::Owned(bytes) => bytes.capacity(),
        }
    }

    pub fn as_bytes(&self) -> &[u8] {
        match self {
            Self::Inline(value) => value.as_bytes(),
            Self::Owned(bytes) => bytes,
        }
    }

    fn inline(len: usize) -> Result<Self, &'static str> {
        if len <= 512 {
            Ok(Self::Inline(msvc_string::Text {
                len,
                ..Default::default()
            }))
        } else {
            Err("string output too short")
        }
    }

    fn bytes_mut(&mut self) -> &mut [u8] {
        match self {
            Self::Inline(value) => &mut value.bytes[..value.len],
            Self::Owned(bytes) => bytes,
        }
    }

    pub(crate) fn from_descriptor(
        memory: &impl Memory,
        descriptor: msvc_string::Descriptor,
        growing: bool,
    ) -> Result<Self, &'static str> {
        if descriptor.len > 512 && growing {
            let mut bytes = Vec::new();
            // 140EC8640/1413BAA70 consume the original std::string length.
            // Acquire payload chunks before growth so an invalid source keeps
            // its original read failure ahead of any allocation failure.
            descriptor.copy_buffer(memory, &mut crate::walk::Buffer::Growing(&mut bytes), 0)?;
            return Ok(Self::Owned(bytes));
        }
        let mut value = Self::inline(descriptor.len)?;
        descriptor.copy(memory, value.bytes_mut())?;
        Ok(value)
    }

    pub(crate) fn from_source(
        memory: &impl Memory,
        source: usize,
        len: usize,
        inline: Option<&[u8]>,
        growing: bool,
    ) -> Result<Self, &'static str> {
        if len > 512 && growing {
            let mut bytes = Vec::new();
            msvc_string::copy_growing(
                memory,
                source,
                len,
                &mut bytes,
                0,
                "technology token unreadable",
                "research text allocation failed",
            )?;
            return Ok(Self::Owned(bytes));
        }
        let mut value = Self::inline(len).map_err(|_| "technology token too long")?;
        if let Some(bytes) = inline {
            value.bytes_mut().copy_from_slice(&bytes[..len]);
        } else if !read_slice(memory, source, value.bytes_mut()) {
            return Err("technology token unreadable");
        }
        Ok(value)
    }
}
