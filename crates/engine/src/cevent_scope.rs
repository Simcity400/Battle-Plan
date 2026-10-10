//! CEventScope fields copied into pending events, windows and commands.
//!
//! Scope country, actor and containing-country recipient are distinct. Zero
//! scope country is valid. Event readers are excluded from recording. The
//! ability argument shares this layout; no separate effect-scope layout is established.

/// CEventScope primary vtable.
pub const VTABLE_RVA: usize = 0x027b_df18;
/// CEventScope FROM scope pointer.
pub const FROM: usize = 0x20;
/// CEventScope country tag, u32.
pub const COUNTRY_TAG: usize = 0x08;
/// CEventScope first random word, u32.
pub const RANDOM_A: usize = 0x0c;
/// CEventScope second random word, u32.
pub const RANDOM_B: usize = 0x10;
/// CEventScope strategic-region pointer.
pub const STRATEGIC_REGION: usize = 0x48;
/// CEventScope character reference (u64 pair), copied from the unit leader character field; not a unit-leader handle.
pub const CHARACTER_REFERENCE: usize = 0x50;
/// CEventScope character-reference type word, u32.
pub const CHARACTER_REFERENCE_TYPE: usize = 0x50;
/// Character-reference id u32 word; nonzero gates registry resolution.
pub const CHARACTER_REFERENCE_ID: usize = 0x54;
/// CEventScope combat-context pointer.
pub const COMBAT_CONTEXT: usize = 0x60;
/// Reference pair for scope mask 0x20; registry target class remains unknown.
pub const SCOPE_MASK_20_REFERENCE: usize = 0x68;
/// First raw u32 of the scope-mask-0x20 reference; semantic word meaning remains unknown.
pub const SCOPE_MASK_20_REFERENCE_FIRST_WORD: usize = 0x68;
/// Second raw u32 of the scope-mask-0x20 reference; semantic word meaning remains unknown.
pub const SCOPE_MASK_20_REFERENCE_SECOND_WORD: usize = 0x6c;
/// Scope state id, signed i32; values at least one admit state scope.
pub const STATE_ID: usize = 0xa8;
/// CEventScope allocation size.
pub const SIZE: usize = 0xb0;
/// CEventScope country value meaning no country.
pub const NO_COUNTRY: u32 = 0;
/// Scope copy constructor.
pub const COPY_CONSTRUCT_RVA: usize = 0x0052_85f0;
/// Default scope constructor initializes the country word to zero.
pub const DEFAULT_CONSTRUCT_RVA: usize = 0x0052_8800;
/// Memberwise scope copy preserves the country word verbatim.
pub const COPY_MEMBERS_RVA: usize = 0x0052_96c0;
/// Scope country setter.
pub const SET_COUNTRY_RVA: usize = 0x0052_dd00;
/// Scope equality used in pending-event lookup.
pub const EQUALS_RVA: usize = 0x0052_bcc0;
/// Assign a unit leader's character reference to the scope.
pub const SET_UNIT_LEADER_RVA: usize = 0x0052_efd0;
/// Resolve a character from the scope reference registry.
pub const CHARACTER_RESOLVER_RVA: usize = 0x0052_9310;
/// Resolve the scope-mask-0x20 reference when nonzero; target class remains unknown.
pub const SCOPE_MASK_20_RESOLVER_RVA: usize = 0x0052_9e30;
/// Resolve the character, then its unit-leader role.
pub const ABILITY_CASTER_RESOLVER_RVA: usize = 0x0052_94f0;
/// Exact ability-caster resolver entry bytes.
pub const ABILITY_CASTER_RESOLVER_SENTINEL: [u8; 16] = [
    0x48, 0x83, 0xec, 0x28, 0x48, 0x83, 0xc1, 0x50, 0x83, 0x39, 0x00, 0x75, 0x06, 0x83, 0x79, 0x04,
];
