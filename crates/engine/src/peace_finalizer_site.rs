//! Observation boundary after the peace finalized flag is stored and before
//! history turns and actions are destroyed. R13 holds the conference on either
//! incoming path, including immediate civil-war and zero-factor settlement.
//! The full reference is registered before startup. This boundary has no player
//! or country filter and requires no hourly roster observation.
//!
//! This interior site must resume its relocated seven-byte instruction through
//! a trampoline jump. The stack is aligned for a call; the bridge supplies
//! 32-byte shadow space and preserves nonvolatile registers. The tail overwrites
//! RCX, RDX and R8 before destruction and consumes no other volatile register or
//! condition flag. Pointers retained inside actions need independent lifetime
//! authentication after settlement has applied them.

use crate::{Memory, read};

/// First instruction after the finalized-flag write.
pub const RVA: usize = 0x00e3_1659;
/// Bridge return address at the instruction following relocation. While
/// hooked, the site itself contains a tail-jump patch that would mislead unwinding.
/// The two positions share stack depth and exception-handler state.
pub const RETURN_RVA: usize = 0x00e3_1660;
/// File-backed instruction sentinel for the interior site.
pub const SENTINEL: [u8; 16] = [
    0x49, 0x8b, 0x9d, 0xf8, 0x01, 0x00, 0x00, 0x4c, 0x8b, 0x43, 0x08, 0x49, 0x8d, 0x95, 0xf8, 0x01,
];

/// The enclosing loader must also verify `build::PINNED_BUILD`. A matching
/// sentinel alone never authorizes installation on another executable.
pub fn verify(memory: &impl Memory, base: usize) -> Result<usize, &'static str> {
    let site = base
        .checked_add(RVA)
        .ok_or("peace finalizer site address overflow")?;
    if read::<[u8; 16]>(memory, site) != Some(SENTINEL) {
        return Err("peace finalizer sentinel unreadable or differs");
    }
    Ok(site)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::SliceMemory;

    #[test]
    fn exact_bytes_required_including_the_unpatched_suffix() {
        let base = 0x140000000;
        let mut bytes = SENTINEL;
        assert_eq!(
            verify(
                &SliceMemory {
                    base: base + RVA,
                    bytes: &bytes
                },
                base
            ),
            Ok(base + RVA)
        );
        for index in 0..bytes.len() {
            bytes[index] ^= 1;
            assert_eq!(
                verify(
                    &SliceMemory {
                        base: base + RVA,
                        bytes: &bytes
                    },
                    base
                ),
                Err("peace finalizer sentinel unreadable or differs")
            );
            bytes[index] ^= 1;
        }
        for length in 0..bytes.len() {
            assert!(
                verify(
                    &SliceMemory {
                        base: base + RVA,
                        bytes: &bytes[..length]
                    },
                    base
                )
                .is_err()
            );
        }
        assert_eq!(
            verify(
                &SliceMemory {
                    base: 0,
                    bytes: &[]
                },
                usize::MAX
            ),
            Err("peace finalizer site address overflow")
        );
    }
}
