//! CIncomingDiplomaticAction wrapper, distinct from the pointed-to action body.

use engine_sites::Rva;
/// CIncomingDiplomaticAction primary vtable.
pub const VTABLE_RVA: Rva = Rva(0x02aa_f838);
/// CIncomingDiplomaticAction constructor.
pub const CONSTRUCTOR_RVA: Rva = Rva(0x01b3_b1d0);
// Construction first copies the default invalid reference into its reference slot, then mints the
// real identity from `creference_registry::ID_COUNTER_RVA`. The invalid identity is
// `creference_registry::INVALID_REFERENCE_RVA`.
/// CIncomingDiplomaticAction packed handle (low u32 type/high u32 id).
pub const REFERENCE: usize = 0x08;
/// CIncomingDiplomaticAction reference type.
pub const REFERENCE_TYPE: u32 = 4_713;
/// Incoming-action byte initialized to zero; meaning remains unknown.
pub const STATUS_BYTE: usize = 0x10;
/// CIncomingDiplomaticAction action-body pointer.
pub const BODY: usize = 0x18;
