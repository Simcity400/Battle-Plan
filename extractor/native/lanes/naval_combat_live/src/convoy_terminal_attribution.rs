use std::fmt;

pub const A729_EXE_SHA256: &str =
    "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad";

pub const A729_HOOK_RVA: usize = 0x0196_2378;
pub const A729_HOOK_RESUME_RVA: usize = 0x0196_237D;
pub const A729_TERMINAL_AGGREGATOR_RVA: usize = 0x001C_AE90;

/// silently admitting a drifted member or argument ABI.
pub const A729_CONTEXT_RVA: usize = 0x0196_2364;
pub const A729_CONTEXT_BYTES: [u8; 25] = [
    0x4D, 0x8D, 0x4C, 0x24, 0x20, 0x4D, 0x8B, 0xC7, 0xBA, 0x01, 0x00, 0x00, 0x00, 0x48, 0x8B, 0x0D,
    0x18, 0x4B, 0x9B, 0x01, 0xE8, 0x13, 0x8B, 0x86, 0xFE,
];
pub const A729_CALL_BYTES: [u8; 5] = [0xE8, 0x13, 0x8B, 0x86, 0xFE];

pub const CFEX_MEMBER_VTABLE_RVA: usize = 0x02A0_A228;
pub const CFEX_MEMBER_CONVOY_HANDLE_OFF: usize = 0x10;
pub const CFEX_MEMBER_OWNER_TAG_OFF: usize = 0x20;
pub const CFEX_MEMBER_CLIENT_HANDLE_OFF: usize = 0xD0;
pub const CFEX_MEMBER_CONVOY_INDEX_OFF: usize = 0xD8;
pub const CFEX_MEMBER_STATE_OFF: usize = 0xE0;
pub const CFEX_MEMBER_BATTLE_SLOT_OFF: usize = 0x120;
pub const TERMINAL_SUNK_STATE: u32 = 0x20;
pub const GAME_ENTITY_TYPE_TAG: u32 = 4713;
pub const MAX_COUNTRY_TAG_DB_ID: u32 = (i32::MAX as u32) - 1;

pub const CARRIER_MAGIC: u32 = u32::from_le_bytes(*b"CTA1");
pub const CARRIER_VERSION: u16 = 1;
pub const CARRIER_LEN: usize = 96;
pub const CARRIER_FLAG_LIVE_ROUTE_EXACT: u32 = 1 << 0;
pub const CARRIER_FLAG_LIVE_ROUTE_MANAGER_EXACT: u32 = CARRIER_FLAG_LIVE_ROUTE_EXACT;

pub const D90_STATIC_AUTHORITY_MANIFEST_SHA256: Option<[u8; 32]> = Some([
    0xAF, 0xEC, 0xCD, 0x1F, 0x65, 0xE7, 0x6F, 0xB2, 0xB1, 0x0C, 0x56, 0xBB, 0x28, 0xC5, 0x6C, 0x4E,
    0xE8, 0x4E, 0x4D, 0x10, 0xD3, 0x6B, 0x0F, 0x6E, 0x74, 0xC6, 0xE6, 0xC8, 0xD4, 0xC8, 0x0E, 0x99,
]);
pub const DISCOVERY_ONLY_CONVOY_CUSTODY_PROMOTABLE: bool = false;

pub const ACTIVATION_COMPILED: bool = true;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct EntityHandle {
    pub id: u32,
    pub type_tag: u32,
}

impl EntityHandle {
    fn validate_game_entity(self, label: &'static str) -> Result<Self, CaptureError> {
        if self.id == 0 || self.type_tag != GAME_ENTITY_TYPE_TAG {
            return Err(CaptureError::InvalidHandle {
                label,
                handle: self,
            });
        }
        Ok(self)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum RouteEndpointKind {
    State = 1,
    Province = 2,
}

impl RouteEndpointKind {
    fn decode(value: u8) -> Result<Self, CarrierDecodeError> {
        match value {
            1 => Ok(Self::State),
            2 => Ok(Self::Province),
            _ => Err(CarrierDecodeError::InvalidEndpointKind(value)),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RouteEndpoint {
    pub kind: RouteEndpointKind,
    pub id: u32,
}

impl RouteEndpoint {
    fn validate(self, label: &'static str) -> Result<Self, CaptureError> {
        if self.id == 0 {
            return Err(CaptureError::InvalidEndpoint(label));
        }
        Ok(self)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum RouteProducerKind {
    NavalTransport = 1,
    LendLease = 2,
    Export = 3,
}

impl RouteProducerKind {
    fn decode(value: u8) -> Result<Self, CarrierDecodeError> {
        match value {
            1 => Ok(Self::NavalTransport),
            2 => Ok(Self::LendLease),
            3 => Ok(Self::Export),
            _ => Err(CarrierDecodeError::InvalidProducerKind(value)),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CombatEpoch {
    pub combat_id: u32,
    pub generation: u64,
}

impl CombatEpoch {
    fn validate(self) -> Result<Self, CaptureError> {
        if self.combat_id == 0 || self.generation == 0 {
            return Err(CaptureError::InvalidCombatEpoch);
        }
        Ok(self)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LiveRouteResolution {
    pub route_key: EntityHandle,
    pub producer_kind: RouteProducerKind,
    pub source: RouteEndpoint,
    pub destination: RouteEndpoint,
}

pub trait ExactLiveRouteResolver {
    fn resolve_client_route(&self, client: EntityHandle) -> Result<LiveRouteResolution, String>;
}

pub trait ExactCombatEpochResolver {
    /// Resolve a member address to the combat membership the same-hour d49/d50
    /// walk bound it to: the combat epoch AND the battle slot it occupied.
    /// The slot rides with the epoch so capture time can prove the member is
    /// still where the walk saw it, not merely that the address resolves.
    fn resolve_member_epoch(&self, member_address: usize) -> Result<CombatEpochMembership, String>;
}

/// One member's published combat membership: which combat (epoch) and where in
/// it (battle slot). Both must still match live memory at event time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CombatEpochMembership {
    pub epoch: CombatEpoch,
    pub battle_slot_index_raw: u32,
}

pub trait ExactMemory {
    fn read_exact(&self, address: usize, out: &mut [u8]) -> bool;

    fn u32(&self, address: usize) -> Option<u32> {
        let mut bytes = [0u8; 4];
        self.read_exact(address, &mut bytes)
            .then_some(u32::from_le_bytes(bytes))
    }

    fn u64(&self, address: usize) -> Option<u64> {
        let mut bytes = [0u8; 8];
        self.read_exact(address, &mut bytes)
            .then_some(u64::from_le_bytes(bytes))
    }

    fn handle(&self, address: usize) -> Option<EntityHandle> {
        Some(EntityHandle {
            type_tag: self.u32(address)?,
            id: self.u32(address.checked_add(4)?)?,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct A729PreCollapseRegisters {
    pub rcx_game_state: usize,
    pub edx_count: u32,
    pub r8_killer_country_ref: usize,
    pub r9_owner_country_ref: usize,
    pub r12_member: usize,
    pub r15_killer_country_ref: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConvoyTerminalAttributionCarrier {
    pub producer_sequence: u64,
    pub game_hour_raw: i32,
    pub combat: CombatEpoch,
    pub convoy: EntityHandle,
    pub owner_tag_db_id: u32,
    pub killer_tag_db_id: u32,
    pub client: EntityHandle,
    pub convoy_index: u32,
    pub member_state: u32,
    pub battle_slot: u32,
    pub route: LiveRouteResolution,
}

impl ConvoyTerminalAttributionCarrier {
    pub fn encode(self) -> [u8; CARRIER_LEN] {
        let mut out = [0u8; CARRIER_LEN];
        write_u32(&mut out, 0, CARRIER_MAGIC);
        write_u16(&mut out, 4, CARRIER_VERSION);
        write_u16(&mut out, 6, CARRIER_LEN as u16);
        write_u32(&mut out, 8, CARRIER_FLAG_LIVE_ROUTE_EXACT);
        write_u64(&mut out, 16, self.producer_sequence);
        write_i32(&mut out, 24, self.game_hour_raw);
        write_u32(&mut out, 28, self.combat.combat_id);
        write_u64(&mut out, 32, self.combat.generation);
        write_handle(&mut out, 40, self.convoy);
        write_u32(&mut out, 48, self.owner_tag_db_id);
        write_u32(&mut out, 52, self.killer_tag_db_id);
        write_handle(&mut out, 56, self.client);
        write_u32(&mut out, 64, self.convoy_index);
        write_u32(&mut out, 68, self.member_state);
        write_u32(&mut out, 72, self.battle_slot);
        write_handle(&mut out, 76, self.route.route_key);
        out[84] = self.route.producer_kind as u8;
        out[85] = self.route.source.kind as u8;
        out[86] = self.route.destination.kind as u8;
        write_u32(&mut out, 88, self.route.source.id);
        write_u32(&mut out, 92, self.route.destination.id);
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, CarrierDecodeError> {
        if bytes.len() != CARRIER_LEN {
            return Err(CarrierDecodeError::Length(bytes.len()));
        }
        if read_u32(bytes, 0)? != CARRIER_MAGIC {
            return Err(CarrierDecodeError::Magic);
        }
        if read_u16(bytes, 4)? != CARRIER_VERSION || read_u16(bytes, 6)? as usize != CARRIER_LEN {
            return Err(CarrierDecodeError::VersionOrLayout);
        }
        if read_u32(bytes, 8)? != CARRIER_FLAG_LIVE_ROUTE_EXACT {
            return Err(CarrierDecodeError::Flags);
        }
        if read_u32(bytes, 12)? != 0 || bytes[87] != 0 {
            return Err(CarrierDecodeError::Reserved);
        }
        let carrier = Self {
            producer_sequence: read_u64(bytes, 16)?,
            game_hour_raw: read_i32(bytes, 24)?,
            combat: CombatEpoch {
                combat_id: read_u32(bytes, 28)?,
                generation: read_u64(bytes, 32)?,
            },
            convoy: read_handle(bytes, 40)?,
            owner_tag_db_id: read_u32(bytes, 48)?,
            killer_tag_db_id: read_u32(bytes, 52)?,
            client: read_handle(bytes, 56)?,
            convoy_index: read_u32(bytes, 64)?,
            member_state: read_u32(bytes, 68)?,
            battle_slot: read_u32(bytes, 72)?,
            route: LiveRouteResolution {
                route_key: read_handle(bytes, 76)?,
                producer_kind: RouteProducerKind::decode(bytes[84])?,
                source: RouteEndpoint {
                    kind: RouteEndpointKind::decode(bytes[85])?,
                    id: read_u32(bytes, 88)?,
                },
                destination: RouteEndpoint {
                    kind: RouteEndpointKind::decode(bytes[86])?,
                    id: read_u32(bytes, 92)?,
                },
            },
        };
        validate_decoded_carrier(carrier)?;
        Ok(carrier)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActivationBlocker {
    StaticAuthorityManifestNotPinned,
    ActivationCompileTimeDisabled,
}

pub fn activation_blockers() -> Vec<ActivationBlocker> {
    let mut blockers = Vec::new();
    if D90_STATIC_AUTHORITY_MANIFEST_SHA256.is_none() {
        blockers.push(ActivationBlocker::StaticAuthorityManifestNotPinned);
    }
    if !ACTIVATION_COMPILED {
        blockers.push(ActivationBlocker::ActivationCompileTimeDisabled);
    }
    blockers
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedA729Hook {
    pub context_address: usize,
    pub hook_address: usize,
    pub resume_address: usize,
    pub original_target_address: usize,
    pub activation_blockers: Vec<ActivationBlocker>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrepareError {
    NullImageBase,
    BinaryHashMismatch,
    AddressOverflow,
    ContextUnreadable,
    ContextByteMismatch,
    CallTargetMismatch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct A729CallWindow {
    pub context_address: usize,
    pub hook_address: usize,
    pub resume_address: usize,
    pub original_target_address: usize,
}

pub fn a729_call_window_is_exact<M: ExactMemory>(
    memory: &M,
    image_base: usize,
    executable_sha256: &str,
) -> Result<A729CallWindow, PrepareError> {
    if image_base == 0 {
        return Err(PrepareError::NullImageBase);
    }
    if !executable_sha256.eq_ignore_ascii_case(A729_EXE_SHA256) {
        return Err(PrepareError::BinaryHashMismatch);
    }
    let context_address = image_base
        .checked_add(A729_CONTEXT_RVA)
        .ok_or(PrepareError::AddressOverflow)?;
    let hook_address = image_base
        .checked_add(A729_HOOK_RVA)
        .ok_or(PrepareError::AddressOverflow)?;
    let resume_address = image_base
        .checked_add(A729_HOOK_RESUME_RVA)
        .ok_or(PrepareError::AddressOverflow)?;
    let expected_target = image_base
        .checked_add(A729_TERMINAL_AGGREGATOR_RVA)
        .ok_or(PrepareError::AddressOverflow)?;
    let mut observed = [0u8; A729_CONTEXT_BYTES.len()];
    if !memory.read_exact(context_address, &mut observed) {
        return Err(PrepareError::ContextUnreadable);
    }
    if observed != A729_CONTEXT_BYTES {
        return Err(PrepareError::ContextByteMismatch);
    }
    let call_offset = A729_HOOK_RVA - A729_CONTEXT_RVA;
    if observed[call_offset..] != A729_CALL_BYTES {
        return Err(PrepareError::ContextByteMismatch);
    }
    let displacement = i32::from_le_bytes(A729_CALL_BYTES[1..5].try_into().unwrap());
    let actual_target = (resume_address as i128) + (displacement as i128);
    if actual_target < 0
        || actual_target > usize::MAX as i128
        || actual_target as usize != expected_target
    {
        return Err(PrepareError::CallTargetMismatch);
    }
    Ok(A729CallWindow {
        context_address,
        hook_address,
        resume_address,
        original_target_address: expected_target,
    })
}

pub fn prepare_a729_hook<M: ExactMemory>(
    memory: &M,
    image_base: usize,
    executable_sha256: &str,
) -> Result<PreparedA729Hook, PrepareError> {
    let window = a729_call_window_is_exact(memory, image_base, executable_sha256)?;
    Ok(PreparedA729Hook {
        context_address: window.context_address,
        hook_address: window.hook_address,
        resume_address: window.resume_address,
        original_target_address: window.original_target_address,
        activation_blockers: activation_blockers(),
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CaptureError {
    NullCallRegister(&'static str),
    CallAbiMismatch(&'static str),
    AddressOverflow,
    MemberUnreadable(&'static str),
    MemberVtableMismatch,
    InvalidHandle {
        label: &'static str,
        handle: EntityHandle,
    },
    InvalidOwnerTag(u32),
    InvalidKillerTag(u32),
    InvalidProducerSequence,
    InvalidMemberState(u32),
    InvalidCombatEpoch,
    InvalidEndpoint(&'static str),
    CombatJoin(String),
    /// The member's LIVE battle slot no longer matches the slot the same-hour
    /// d49/d50 walk bound it to. A member that changed combats (or a reused
    /// member address) fails here BY NAME instead of being attributed to the
    /// stale combat epoch.
    CombatSlotMismatch,
    RouteJoin(String),
    RouteKeyMismatch,
}

pub fn capture_candidate<M, C, R>(
    memory: &M,
    module_base: usize,
    registers: A729PreCollapseRegisters,
    producer_sequence: u64,
    game_hour_raw: i32,
    combat_resolver: &C,
    route_resolver: &R,
) -> Result<ConvoyTerminalAttributionCarrier, CaptureError>
where
    M: ExactMemory,
    C: ExactCombatEpochResolver,
    R: ExactLiveRouteResolver,
{
    if module_base == 0 {
        return Err(CaptureError::NullCallRegister("module_base"));
    }
    if producer_sequence == 0 {
        return Err(CaptureError::InvalidProducerSequence);
    }
    for (label, value) in [
        ("rcx_game_state", registers.rcx_game_state),
        ("r8_killer_country_ref", registers.r8_killer_country_ref),
        ("r9_owner_country_ref", registers.r9_owner_country_ref),
        ("r12_member", registers.r12_member),
        ("r15_killer_country_ref", registers.r15_killer_country_ref),
    ] {
        if value == 0 {
            return Err(CaptureError::NullCallRegister(label));
        }
    }
    if registers.edx_count != 1 {
        return Err(CaptureError::CallAbiMismatch("edx_count"));
    }
    if registers.r8_killer_country_ref != registers.r15_killer_country_ref {
        return Err(CaptureError::CallAbiMismatch("r8_r15_killer_ref"));
    }
    let owner_address = registers
        .r12_member
        .checked_add(CFEX_MEMBER_OWNER_TAG_OFF)
        .ok_or(CaptureError::AddressOverflow)?;
    if registers.r9_owner_country_ref != owner_address {
        return Err(CaptureError::CallAbiMismatch("r9_member_owner_ref"));
    }
    let expected_vtable = module_base
        .checked_add(CFEX_MEMBER_VTABLE_RVA)
        .ok_or(CaptureError::AddressOverflow)? as u64;
    let vtable = read_member_u64(memory, registers.r12_member, 0, "vtable")?;
    if vtable != expected_vtable {
        return Err(CaptureError::MemberVtableMismatch);
    }
    let convoy = read_member_handle(
        memory,
        registers.r12_member,
        CFEX_MEMBER_CONVOY_HANDLE_OFF,
        "convoy_handle",
    )?
    .validate_game_entity("convoy_handle")?;
    let client = read_member_handle(
        memory,
        registers.r12_member,
        CFEX_MEMBER_CLIENT_HANDLE_OFF,
        "client_handle",
    )?
    .validate_game_entity("client_handle")?;
    let owner_tag_db_id = read_member_u32(
        memory,
        registers.r12_member,
        CFEX_MEMBER_OWNER_TAG_OFF,
        "owner_tag_db_id",
    )?;
    if owner_tag_db_id == 0 || owner_tag_db_id > MAX_COUNTRY_TAG_DB_ID {
        return Err(CaptureError::InvalidOwnerTag(owner_tag_db_id));
    }
    let killer_tag_db_id = memory
        .u32(registers.r8_killer_country_ref)
        .ok_or(CaptureError::MemberUnreadable("killer_tag_db_id"))?;
    if killer_tag_db_id == 0 || killer_tag_db_id > MAX_COUNTRY_TAG_DB_ID {
        return Err(CaptureError::InvalidKillerTag(killer_tag_db_id));
    }
    let convoy_index = read_member_u32(
        memory,
        registers.r12_member,
        CFEX_MEMBER_CONVOY_INDEX_OFF,
        "convoy_index",
    )?;
    let member_state = read_member_u32(
        memory,
        registers.r12_member,
        CFEX_MEMBER_STATE_OFF,
        "member_state",
    )?;
    if member_state != TERMINAL_SUNK_STATE {
        return Err(CaptureError::InvalidMemberState(member_state));
    }
    let battle_slot = read_member_u32(
        memory,
        registers.r12_member,
        CFEX_MEMBER_BATTLE_SLOT_OFF,
        "battle_slot",
    )?;
    // The epoch join is not trusted blind: the member must still sit where the
    // same-hour walk saw it. A member that left its published battle slot (or
    // a reused member address) fails HERE, by name. What this check cannot
    // catch is a move to a DIFFERENT combat at the SAME slot index within one
    // cadence interval -- the sink ABI (game_state, count, killer_ref,
    // owner_ref) carries no combat identity for the recorder to compare, so
    // that residual is named in FIX.md rather than hidden.
    let membership = combat_resolver
        .resolve_member_epoch(registers.r12_member)
        .map_err(CaptureError::CombatJoin)?;
    let combat = membership.epoch.validate()?;
    if membership.battle_slot_index_raw != battle_slot {
        return Err(CaptureError::CombatSlotMismatch);
    }
    let route = route_resolver
        .resolve_client_route(client)
        .map_err(CaptureError::RouteJoin)?;
    if route.route_key != client {
        return Err(CaptureError::RouteKeyMismatch);
    }
    route.route_key.validate_game_entity("route_key")?;
    route.source.validate("source")?;
    route.destination.validate("destination")?;

    Ok(ConvoyTerminalAttributionCarrier {
        producer_sequence,
        game_hour_raw,
        combat,
        convoy,
        owner_tag_db_id,
        killer_tag_db_id,
        client,
        convoy_index,
        member_state,
        battle_slot,
        route,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CarrierDecodeError {
    Length(usize),
    Truncated,
    Magic,
    VersionOrLayout,
    Flags,
    Reserved,
    InvalidEndpointKind(u8),
    InvalidProducerKind(u8),
    InvalidField(&'static str),
}

impl fmt::Display for CarrierDecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

fn validate_decoded_carrier(
    carrier: ConvoyTerminalAttributionCarrier,
) -> Result<(), CarrierDecodeError> {
    if carrier.producer_sequence == 0 {
        return Err(CarrierDecodeError::InvalidField("producer_sequence"));
    }
    if carrier.combat.combat_id == 0 || carrier.combat.generation == 0 {
        return Err(CarrierDecodeError::InvalidField("combat_epoch"));
    }
    for (label, handle) in [
        ("convoy_handle", carrier.convoy),
        ("client_handle", carrier.client),
        ("route_key", carrier.route.route_key),
    ] {
        if handle.id == 0 || handle.type_tag != GAME_ENTITY_TYPE_TAG {
            return Err(CarrierDecodeError::InvalidField(label));
        }
    }
    if carrier.client != carrier.route.route_key {
        return Err(CarrierDecodeError::InvalidField("route_key_join"));
    }
    if carrier.owner_tag_db_id == 0
        || carrier.owner_tag_db_id > MAX_COUNTRY_TAG_DB_ID
        || carrier.killer_tag_db_id == 0
        || carrier.killer_tag_db_id > MAX_COUNTRY_TAG_DB_ID
    {
        return Err(CarrierDecodeError::InvalidField("country_tag"));
    }
    if carrier.member_state != TERMINAL_SUNK_STATE {
        return Err(CarrierDecodeError::InvalidField("member_state"));
    }
    if carrier.route.source.id == 0 || carrier.route.destination.id == 0 {
        return Err(CarrierDecodeError::InvalidField("route_endpoint"));
    }
    Ok(())
}

fn read_member_u32<M: ExactMemory>(
    memory: &M,
    member: usize,
    offset: usize,
    label: &'static str,
) -> Result<u32, CaptureError> {
    memory
        .u32(
            member
                .checked_add(offset)
                .ok_or(CaptureError::AddressOverflow)?,
        )
        .ok_or(CaptureError::MemberUnreadable(label))
}

fn read_member_u64<M: ExactMemory>(
    memory: &M,
    member: usize,
    offset: usize,
    label: &'static str,
) -> Result<u64, CaptureError> {
    memory
        .u64(
            member
                .checked_add(offset)
                .ok_or(CaptureError::AddressOverflow)?,
        )
        .ok_or(CaptureError::MemberUnreadable(label))
}

fn read_member_handle<M: ExactMemory>(
    memory: &M,
    member: usize,
    offset: usize,
    label: &'static str,
) -> Result<EntityHandle, CaptureError> {
    memory
        .handle(
            member
                .checked_add(offset)
                .ok_or(CaptureError::AddressOverflow)?,
        )
        .ok_or(CaptureError::MemberUnreadable(label))
}

fn write_u16(out: &mut [u8], offset: usize, value: u16) {
    out[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn write_u32(out: &mut [u8], offset: usize, value: u32) {
    out[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn write_i32(out: &mut [u8], offset: usize, value: i32) {
    out[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn write_u64(out: &mut [u8], offset: usize, value: u64) {
    out[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

fn write_handle(out: &mut [u8], offset: usize, handle: EntityHandle) {
    write_u32(out, offset, handle.id);
    write_u32(out, offset + 4, handle.type_tag);
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, CarrierDecodeError> {
    Ok(u16::from_le_bytes(
        bytes
            .get(offset..offset + 2)
            .ok_or(CarrierDecodeError::Truncated)?
            .try_into()
            .unwrap(),
    ))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, CarrierDecodeError> {
    Ok(u32::from_le_bytes(
        bytes
            .get(offset..offset + 4)
            .ok_or(CarrierDecodeError::Truncated)?
            .try_into()
            .unwrap(),
    ))
}

fn read_i32(bytes: &[u8], offset: usize) -> Result<i32, CarrierDecodeError> {
    Ok(i32::from_le_bytes(
        bytes
            .get(offset..offset + 4)
            .ok_or(CarrierDecodeError::Truncated)?
            .try_into()
            .unwrap(),
    ))
}

fn read_u64(bytes: &[u8], offset: usize) -> Result<u64, CarrierDecodeError> {
    Ok(u64::from_le_bytes(
        bytes
            .get(offset..offset + 8)
            .ok_or(CarrierDecodeError::Truncated)?
            .try_into()
            .unwrap(),
    ))
}

fn read_handle(bytes: &[u8], offset: usize) -> Result<EntityHandle, CarrierDecodeError> {
    Ok(EntityHandle {
        id: read_u32(bytes, offset)?,
        type_tag: read_u32(bytes, offset + 4)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[derive(Default)]
    struct Memory(BTreeMap<usize, u8>);

    impl Memory {
        fn put(&mut self, address: usize, bytes: &[u8]) {
            for (index, byte) in bytes.iter().copied().enumerate() {
                self.0.insert(address + index, byte);
            }
        }
        fn put_u32(&mut self, address: usize, value: u32) {
            self.put(address, &value.to_le_bytes());
        }
        fn put_u64(&mut self, address: usize, value: u64) {
            self.put(address, &value.to_le_bytes());
        }
        fn put_handle(&mut self, address: usize, handle: EntityHandle) {
            self.put_u32(address, handle.type_tag);
            self.put_u32(address + 4, handle.id);
        }
    }

    impl ExactMemory for Memory {
        fn read_exact(&self, address: usize, out: &mut [u8]) -> bool {
            for (index, slot) in out.iter_mut().enumerate() {
                let Some(byte) = self.0.get(&(address + index)) else {
                    return false;
                };
                *slot = *byte;
            }
            true
        }
    }

    struct Combat;
    impl ExactCombatEpochResolver for Combat {
        fn resolve_member_epoch(
            &self,
            member_address: usize,
        ) -> Result<CombatEpochMembership, String> {
            (member_address == 0x5000)
                .then_some(CombatEpochMembership {
                    epoch: CombatEpoch {
                        combat_id: 13,
                        generation: 9,
                    },
                    // Matches the fixture member's live battle slot at
                    // CFEX_MEMBER_BATTLE_SLOT_OFF; anything else now refuses.
                    battle_slot_index_raw: 10,
                })
                .ok_or_else(|| "foreign member".into())
        }
    }

    struct Route;
    impl ExactLiveRouteResolver for Route {
        fn resolve_client_route(
            &self,
            client: EntityHandle,
        ) -> Result<LiveRouteResolution, String> {
            Ok(LiveRouteResolution {
                route_key: client,
                producer_kind: RouteProducerKind::NavalTransport,
                source: RouteEndpoint {
                    kind: RouteEndpointKind::Province,
                    id: 12140,
                },
                destination: RouteEndpoint {
                    kind: RouteEndpointKind::Province,
                    id: 9950,
                },
            })
        }
    }

    #[test]
    fn engine_handle_layout_is_type_tag_low() {
        let mut memory = Memory::default();
        let raw: u64 = (106_381u64 << 32) | u64::from(GAME_ENTITY_TYPE_TAG);
        assert_eq!(raw, 0x0001_9F8D_0000_1269);
        memory.put_u64(0x4000, raw);

        let decoded = memory.handle(0x4000).unwrap();
        assert_eq!(
            decoded,
            EntityHandle {
                id: 106_381,
                type_tag: GAME_ENTITY_TYPE_TAG,
            },
            "the low u32 is the type tag and the high u32 is the instance id"
        );
        decoded
            .validate_game_entity("t25_shape")
            .expect("the t25 convoy client shape must be ADMITTED, not refused");

        let mut mirror = Memory::default();
        mirror.put_handle(0x4000, decoded);
        assert_eq!(mirror.u64(0x4000), Some(raw));

        let flipped = memory
            .u64(0x4000)
            .map(|value| EntityHandle {
                type_tag: (value >> 32) as u32,
                id: value as u32,
            })
            .unwrap();
        assert!(flipped.validate_game_entity("flipped").is_err());
    }

    #[test]
    fn exact_context_prepares_for_production() {
        let base = 0x1400_0000_0usize;
        let mut memory = Memory::default();
        memory.put(base + A729_CONTEXT_RVA, &A729_CONTEXT_BYTES);
        let prepared = prepare_a729_hook(&memory, base, A729_EXE_SHA256).unwrap();
        assert_eq!(prepared.hook_address, base + A729_HOOK_RVA);
        assert_eq!(
            prepared.original_target_address,
            base + A729_TERMINAL_AGGREGATOR_RVA
        );
        assert_eq!(prepared.activation_blockers, activation_blockers());
        assert!(prepared.activation_blockers.is_empty());
    }

    #[test]
    fn terminal_candidate_is_pointer_free_and_round_trips() {
        let base = 0x1400_0000_0usize;
        let member = 0x5000usize;
        let killer = 0x9000usize;
        let mut memory = Memory::default();
        memory.put_u64(member, (base + CFEX_MEMBER_VTABLE_RVA) as u64);
        memory.put_handle(
            member + CFEX_MEMBER_CONVOY_HANDLE_OFF,
            EntityHandle {
                id: 157947,
                type_tag: GAME_ENTITY_TYPE_TAG,
            },
        );
        memory.put_u32(member + CFEX_MEMBER_OWNER_TAG_OFF, 77);
        memory.put_handle(
            member + CFEX_MEMBER_CLIENT_HANDLE_OFF,
            EntityHandle {
                id: 156618,
                type_tag: GAME_ENTITY_TYPE_TAG,
            },
        );
        memory.put_u32(member + CFEX_MEMBER_CONVOY_INDEX_OFF, 0);
        memory.put_u32(member + CFEX_MEMBER_STATE_OFF, TERMINAL_SUNK_STATE);
        memory.put_u32(member + CFEX_MEMBER_BATTLE_SLOT_OFF, 10);
        memory.put_u32(killer, 51);
        let carrier = capture_candidate(
            &memory,
            base,
            A729PreCollapseRegisters {
                rcx_game_state: 0x7000,
                edx_count: 1,
                r8_killer_country_ref: killer,
                r9_owner_country_ref: member + CFEX_MEMBER_OWNER_TAG_OFF,
                r12_member: member,
                r15_killer_country_ref: killer,
            },
            1,
            72,
            &Combat,
            &Route,
        )
        .unwrap();
        let bytes = carrier.encode();
        assert_eq!(
            ConvoyTerminalAttributionCarrier::decode(&bytes).unwrap(),
            carrier
        );
        for pointer in [member as u64, killer as u64, 0x7000u64] {
            assert!(!bytes
                .windows(8)
                .any(|window| window == pointer.to_le_bytes()));
        }
    }

    /// The member left its published battle slot (changed combats, or a reused
    /// member address): the stale-epoch join must refuse BY NAME, never
    /// attribute the sink to combat it no longer belongs to.
    #[test]
    fn terminal_candidate_refuses_a_member_that_left_its_published_slot() {
        struct SlotDrift;
        impl ExactCombatEpochResolver for SlotDrift {
            fn resolve_member_epoch(
                &self,
                member_address: usize,
            ) -> Result<CombatEpochMembership, String> {
                let membership = Combat.resolve_member_epoch(member_address)?;
                Ok(CombatEpochMembership {
                    battle_slot_index_raw: membership.battle_slot_index_raw + 1,
                    ..membership
                })
            }
        }
        let base = 0x1400_0000_0usize;
        let member = 0x5000usize;
        let killer = 0x9000usize;
        let mut memory = Memory::default();
        memory.put_u64(member, (base + CFEX_MEMBER_VTABLE_RVA) as u64);
        memory.put_handle(
            member + CFEX_MEMBER_CONVOY_HANDLE_OFF,
            EntityHandle {
                id: 157947,
                type_tag: GAME_ENTITY_TYPE_TAG,
            },
        );
        memory.put_u32(member + CFEX_MEMBER_OWNER_TAG_OFF, 77);
        memory.put_handle(
            member + CFEX_MEMBER_CLIENT_HANDLE_OFF,
            EntityHandle {
                id: 156618,
                type_tag: GAME_ENTITY_TYPE_TAG,
            },
        );
        memory.put_u32(member + CFEX_MEMBER_CONVOY_INDEX_OFF, 0);
        memory.put_u32(member + CFEX_MEMBER_STATE_OFF, TERMINAL_SUNK_STATE);
        memory.put_u32(member + CFEX_MEMBER_BATTLE_SLOT_OFF, 10);
        memory.put_u32(killer, 51);
        assert_eq!(
            capture_candidate(
                &memory,
                base,
                A729PreCollapseRegisters {
                    rcx_game_state: 0x7000,
                    edx_count: 1,
                    r8_killer_country_ref: killer,
                    r9_owner_country_ref: member + CFEX_MEMBER_OWNER_TAG_OFF,
                    r12_member: member,
                    r15_killer_country_ref: killer,
                },
                1,
                72,
                &SlotDrift,
                &Route,
            )
            .unwrap_err(),
            CaptureError::CombatSlotMismatch
        );
    }

    #[test]
    fn five_byte_match_does_not_bypass_full_context_gate() {
        let base = 0x1400_0000_0usize;
        let mut memory = Memory::default();
        let mut drifted = A729_CONTEXT_BYTES;
        drifted[0] ^= 1;
        memory.put(base + A729_CONTEXT_RVA, &drifted);
        assert_eq!(
            prepare_a729_hook(&memory, base, A729_EXE_SHA256),
            Err(PrepareError::ContextByteMismatch)
        );
    }
}
