//! CCountryOperationTokenManager reached through CCountry +0x15b0.
//!
//! The old reader calls this COperationTokenManager; the pinned executable's RTTI
//! uses CCountryOperationTokenManager. No hourly writer or consistency proof is
//! inherited from the old repeated-read checks.

/// CCountryOperationTokenManager owning CCountry pointer, u64; old diplomacy_intel_operatives_live/src/extractor.rs:10776 compares it with the source country.
pub const COUNTRY: usize = 0x08;
/// CCountryOperationTokenManager inline target-country map; old diplomacy_intel_operatives_live/src/extractor.rs:94,10779-10786 reads its header and target entries.
pub const TARGETS: usize = 0x28;
