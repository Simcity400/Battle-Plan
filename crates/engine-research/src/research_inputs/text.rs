use engine_native_memory::{Memory, msvc_string, read_slice};
use crate::allocation;

const INLINE: usize = 512;

#[derive(Debug, PartialEq, Eq)]
#[allow(
    clippy::large_enum_variant,
    reason = "Short text stays inline so the common row allocates nothing."
)]
pub enum Text {
    Inline(msvc_string::Text<INLINE>),
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

    fn inline(len: usize) -> msvc_string::Text<INLINE> {
        msvc_string::Text {
            len,
            ..Default::default()
        }
    }

    pub(crate) fn from_descriptor(
        memory: &impl Memory,
        descriptor: msvc_string::Descriptor,
    ) -> Result<Self, &'static str> {
        if descriptor.len > INLINE {
            let mut bytes = Vec::new();
            descriptor.copy_buffer(memory, &mut engine_native_memory::walk::Buffer::Growing(&mut bytes), 0)?;
            return Ok(Self::Owned(bytes));
        }
        let mut value = Self::inline(descriptor.len);
        descriptor.copy(memory, &mut value.bytes[..value.len])?;
        Ok(Self::Inline(value))
    }

    pub(crate) fn from_source(
        memory: &impl Memory,
        source: usize,
        len: usize,
        inline: Option<&[u8]>,
    ) -> Result<Self, &'static str> {
        if len > INLINE {
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
        let mut value = Self::inline(len);
        let output = &mut value.bytes[..len];
        if let Some(bytes) = inline {
            output.copy_from_slice(&bytes[..len]);
        } else if !read_slice(memory, source, output) {
            return Err("technology token unreadable");
        }
        Ok(Self::Inline(value))
    }
}
