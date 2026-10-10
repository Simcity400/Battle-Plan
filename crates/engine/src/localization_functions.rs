//! Localization functions whose concrete C++ owning classes remain unresolved.
//! Function identities only; no presentation observer or engine calls are adopted.

/// Variable-substitution formatter; a missing key copies its raw bytes.
pub const MISSING_KEY_DISPLAY_RVA: usize = 0x0223_0940;
/// Localization formatter entry.
pub const FORMAT_RVA: usize = 0x0223_2ab0;
/// Country-name adjective suffix bytes.
pub const COUNTRY_ADJECTIVE_SUFFIX_RVA: usize = 0x027d_11cc;
/// Country-name definite-article suffix bytes.
pub const COUNTRY_DEFINITE_SUFFIX_RVA: usize = 0x027d_11d4;
/// Hashes localization key bytes with FNV-1a 64 before lookup.
pub const TABLE_PROBE_RVA: usize = 0x0238_9ec0;
/// Localization lookup after key hashing.
pub const HASHED_TABLE_LOOKUP_RVA: usize = 0x0238_9e30;
/// Key-presence predicate using the localization hash table.
pub const HAS_KEY_RVA: usize = 0x0238_8190;
/// Active-language getter with null guards and MSVC string storage selection.
pub const LANGUAGE_GETTER_RVA: usize = 0x0238_9260;
/// Localization-root availability probe.
pub const ROOT_AVAILABLE_RVA: usize = 0x0238_9500;
