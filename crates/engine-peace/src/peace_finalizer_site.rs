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

use engine_sites::Rva;

/// First instruction after the finalized-flag write.
pub const RVA: Rva = Rva(0x00eb_5089);
/// Bridge return address at the instruction following relocation. While
/// hooked, the site itself contains a tail-jump patch that would mislead unwinding.
/// The two positions share stack depth and exception-handler state.
pub const RETURN_RVA: Rva = Rva(0x00eb_5090);
