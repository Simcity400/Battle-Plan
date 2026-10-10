//! CCountryTag raw link identity; canonical database slots and live tag strings use separate arrays.

/// CCountryTag link index u32.
pub const LINK_ID: usize = 0;
/// Country-tag storage width: one u32 word in country and province fields.
pub const SIZE: usize = 4;
/// Country resolver: translates the raw link before indexing the country vector.
pub const RESOLVE_COUNTRY_RVA: usize = 0x00ba_5140;
/// Country-reference resolver accepts an i32 pointer, maps zero to slot zero and translates other links.
pub const RESOLVE_COUNTRY_REFERENCE_RVA: usize = 0x00ba_56a0;
/// Exact country-reference resolver entry bytes.
pub const RESOLVE_COUNTRY_REFERENCE_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x10, 0x57, 0x48, 0x83, 0xec, 0x30, 0x80, 0x3d, 0x41, 0x35, 0xa2, 0x02,
];
/// Country adjective accessor.
pub const ADJECTIVE_RVA: usize = 0x00ba_5560;
/// Country definite-name accessor writes the resolved string result.
pub const DEFINITE_NAME_RVA: usize = 0x00ba_5980;
/// Tag-string accessor uses the live state array, or the database when no state exists.
pub const TAG_STRING_RVA: usize = 0x00ba_5c20;
/// Alias comparison uses translated country-link indices.
pub const SAME_COUNTRY_RVA: usize = 0x00ba_60a0;
/// Canonical-slot translation through the state country-link array.
pub const CANONICAL_SLOT_RVA: usize = 0x00ba_6240;
/// Country-tag serializer emits a string only for a positive signed tag.
pub const SERIALIZE_RVA: usize = 0x00ba_6770;
/// List membership accepts raw equality, including zero; alias comparison requires two nonzero tags.
pub const LIST_CONTAINS_RVA: usize = 0x001a_fd40;
/// Name-substitution predicate; its comparison semantics remain unproven.
pub const NAME_SUBSTITUTION_PREDICATE_RVA: usize = 0x00ba_61f0;
