//! CAdvisor role object; membership in active political ideas determines appointment, not the roster alone.

/// CAdvisor vtable; build_profile.rs:194 and the pinned executable's MSVC RTTI confirm the class.
pub const VTABLE_RVA: usize = 0x0299_0e60;
/// CAdvisor parent CCharacter*; extractor.rs:115 and read_advisor_raw_fields validate this link.
pub const CHARACTER: usize = 0x10;
/// CAdvisor idea-token MSVC string; extractor.rs:116 and attach_log.rs:87338 read its bytes.
pub const IDEA_TOKEN: usize = 0x30;
/// CAdvisor slot MSVC string; extractor.rs:117 and attach_log.rs:87386 read its bytes.
pub const SLOT: usize = 0x80;

// These excluded mutator sites affect advisor roles; their receiver types are not established by the gate.
/// ADVISOR_ERASE_EFFECT function, named and byte-checked by d47_character_change_gate/src/hooks_windows.rs:21-95; excluded gate hook.
pub const ADVISOR_ERASE_EFFECT_RVA: usize = 0x0035_C330;
/// ADVISOR_ERASE_EFFECT function prefix; d47_character_change_gate/src/hooks_windows.rs:32-73 checked these bytes before the excluded hook.
pub const ADVISOR_ERASE_EFFECT_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x48, 0x89, 0x7c, 0x24, 0x20, 0x55,
];

/// ADVISOR_REPLACE_EFFECT function, named and byte-checked by d47_character_change_gate/src/hooks_windows.rs:21-95; excluded gate hook.
pub const ADVISOR_REPLACE_EFFECT_RVA: usize = 0x0034_7DF0;
/// ADVISOR_REPLACE_EFFECT function prefix; d47_character_change_gate/src/hooks_windows.rs:32-73 checked these bytes before the excluded hook.
pub const ADVISOR_REPLACE_EFFECT_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x10, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56, 0x41, 0x57,
];

/// ADVISOR_BULK_COPY function, named and byte-checked by d47_character_change_gate/src/hooks_windows.rs:21-95; excluded gate hook.
pub const ADVISOR_BULK_COPY_RVA: usize = 0x00F8_B8A0;
/// ADVISOR_BULK_COPY function prefix; d47_character_change_gate/src/hooks_windows.rs:32-73 checked these bytes before the excluded hook.
pub const ADVISOR_BULK_COPY_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x18, 0x48, 0x89, 0x4c, 0x24, 0x08, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41,
];

/// ADVISOR_INSERT function, named and byte-checked by d47_character_change_gate/src/hooks_windows.rs:21-95; excluded gate hook.
pub const ADVISOR_INSERT_RVA: usize = 0x00F8_D380;
/// ADVISOR_INSERT function prefix; d47_character_change_gate/src/hooks_windows.rs:32-73 checked these bytes before the excluded hook.
pub const ADVISOR_INSERT_SENTINEL: [u8; 16] = [
    0x4c, 0x89, 0x4c, 0x24, 0x20, 0x44, 0x89, 0x44, 0x24, 0x18, 0x48, 0x89, 0x4c, 0x24, 0x08, 0x55,
];
