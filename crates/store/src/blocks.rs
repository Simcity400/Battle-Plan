//! A range's bytes as independently compressed blocks.
//!
//! A packed range is its blocks, each [`BLOCK`] decoded bytes but the last, then one little-endian
//! u64 per block: where that block's stored bytes end. A block stored as long as it decodes is
//! raw; any other is one zstd frame.

use engine_allocation::zstd::{Compressor, Decompressor, Holder, bound};
use std::{cell::RefCell, io};

/// Decoded bytes per block: a read anywhere in a range decodes at most this much.
pub(crate) const BLOCK: usize = 64 << 10;
/// Table bytes a reader reads at a time.
pub(crate) const TABLE_STEP: usize = 4096;
/// Level 3 shrinks these blocks under 1% more at two thirds of the speed.
const LEVEL: i32 = 1;

static CONTEXTS: Holder = Holder::new("zstd: prepared blocks");

/// Whether a first block compressed to `stored` of `size` bytes pays for packing its range.
pub(crate) fn pays(stored: usize, size: usize) -> bool {
    stored * 20 <= size * 19
}

/// One thread's compression context and buffers.
pub(crate) struct Encoder {
    context: Compressor,
    pub(crate) block: Vec<u8>,
    frame: Vec<u8>,
    pub(crate) ends: Vec<u64>,
}

/// One thread's decompression context and the stored bytes of the block it decodes.
struct Decoder {
    context: Decompressor,
    frame: Vec<u8>,
}

thread_local! {
    static ENCODER: RefCell<Option<Encoder>> = const { RefCell::new(None) };
    static DECODER: RefCell<Option<Decoder>> = const { RefCell::new(None) };
}

impl Encoder {
    /// Runs `write` with this thread's encoder, made at its first use.
    pub(crate) fn with<T>(write: impl FnOnce(&mut Self) -> io::Result<T>) -> io::Result<T> {
        ENCODER.with_borrow_mut(|slot| {
            let encoder = match slot {
                Some(encoder) => encoder,
                None => slot.insert(Self {
                    context: Compressor::new(&CONTEXTS, LEVEL)?,
                    block: engine_allocation::vec(BLOCK)?,
                    frame: engine_allocation::vec(bound(BLOCK))?,
                    ends: Vec::new(),
                }),
            };
            write(encoder)
        })
    }

    /// The block compressed: one frame.
    pub(crate) fn compress(&mut self) -> io::Result<&[u8]> {
        self.frame.clear();
        self.context.compress_append(&self.block, &mut self.frame)?;
        Ok(&self.frame)
    }
}

/// Decodes one block into all of `into`: `read` fills its frame of `stored` bytes, below
/// [`BLOCK`], and the `trailer` bytes after it.
pub(crate) fn decode(
    stored: usize,
    trailer: usize,
    into: &mut [u8],
    read: impl FnOnce(&mut [u8]) -> io::Result<()>,
) -> io::Result<()> {
    DECODER.with_borrow_mut(|slot| {
        let decoder = match slot {
            Some(decoder) => decoder,
            None => slot.insert(Decoder {
                context: Decompressor::new(&CONTEXTS)?,
                frame: engine_allocation::filled(BLOCK + 8, 0)?,
            }),
        };
        read(&mut decoder.frame[..stored + trailer])?;
        let written = decoder
            .context
            .decompress(into, &decoder.frame[..stored])
            .map_err(|error| error.refused(|_| damaged()))?;
        if written != into.len() {
            return Err(damaged());
        }
        Ok(())
    })
}

pub(crate) fn damaged() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "damaged prepared block")
}
