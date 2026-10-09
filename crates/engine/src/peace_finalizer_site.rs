//! Observation boundary in the hash-pinned peace finalizer, before it frees
//! bidding turns and their actions. No game code is called by this module.
//!
//! Indexed a729 instructions (`hoi4re asm 140E30E30`) establish:
//! - 140E31651: `MOV byte ptr [R13+0xDD],1`.
//! - 140E31659: `MOV RBX,[R13+0x1F8]` (seven bytes).
//! - 140E31660: `MOV R8,[RBX+8]`.
//! - 140E31664/6B: load RDX/RCX with R13+0x1F8.
//! - 140E31672: call 140BAE7C0, which deletes turns and actions.
//!
//! R13 holds the conference on both incoming paths: the empty liberated list
//! jumps from 140E31417 to 140E31651 with R13 unchanged; the nonempty path
//! restores it from [RBP+0xF0] at 140E3164A. All normally applied settlements
//! inspected in round 1 reach this tail through 140E2E250 -> 140E30E30,
//! including the immediate pure-civil-war/zero-factor creation path. The
//! creation helper registers the full reference through slot 9 at 140BAFA6D,
//! before calling startup at 140BAFA73. There is no player or country filter
//! at this boundary and no dependence on an hourly roster observation.
//!
//! This is an interior boundary, unlike the line-output function entries.
//! A detour must resume the relocated seven-byte instruction by jumping to
//! its trampoline. Calling it as a function would corrupt the native stack.
//! The native stack is aligned for a call here; a bridge must supply its own
//! 32-byte shadow space. Nonvolatile registers must survive the bridge. The
//! tail overwrites RCX, RDX and R8 before the destructor call, and consumes no
//! other volatile registers or condition flags before restoring and returning.
//! This observation does not establish the lifetime of pointers held inside
//! actions after settlement has applied them; those need separate proof.

use crate::{Memory, read};

/// First instruction after the finalized write; VA 140E31659, image base 140000000.
pub const RVA: usize = 0x00e3_1659;
/// The return address a detour's bridge reports to stack walks: 140E31660, the
/// instruction after the relocated one. Not the site: while hooked, the site holds
/// the patch's `E9`, which the unwinder reads as a tail-jump epilogue, so a walk from
/// it would take the wrong return address and skip this frame's handler. Nothing
/// between the two moves RSP and both have the same C++ EH state.
pub const RETURN_RVA: usize = 0x00e3_1660;
/// Sixteen file-backed bytes at VA 140E31659 from the indexed a729 listing.
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
