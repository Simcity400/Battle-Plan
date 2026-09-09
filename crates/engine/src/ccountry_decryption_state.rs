//! CCountryDecryptionState inline rows in CCryptology.

/// CCountryDecryptionState vtable; old build_profile.rs:479, confirmed by pinned executable RTTI.
pub const VTABLE_RVA: usize = 0x029a_64f0;
/// CCountryDecryptionState array stride; old build_profile.rs:483 and read_cryptology_rows indexed walk.
pub const STRIDE: usize = 0x38;
/// CCountryDecryptionState target country reference, read u32; old build_profile.rs:484 and read_cryptology_rows.
pub const TARGET_COUNTRY: usize = 0x08;
/// CCountryDecryptionState decrypting byte, old reader treats nonzero as true; old build_profile.rs:485.
pub const DECRYPTING: usize = 0x12;
/// CCountryDecryptionState amount raw i64; old build_profile.rs:486 and read_cryptology_rows preserve all 64 bits, scale unproved here.
pub const AMOUNT: usize = 0x18;
