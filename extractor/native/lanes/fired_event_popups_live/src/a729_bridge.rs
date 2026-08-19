use crate::a729_runtime::{
    A729EventBridge, CaptureStartPendingWitness, CoverageAttestation, CoverageLifecycleWitnesses,
    CoverageMatrixObservation, CoverageReceipt, NonMaterialisedResolutionWitness,
    ObservedRuntimeTextOwner, PendingBaseline, RawFire, RawMaterialisation, RawResolution,
    RepeatedEventIdWitness, ResolutionPath, SameHourFiringsWitness,
};
use crate::{
    FireInput, MaterialisationInput, OptionRuntimeTextContext, PopupSubtype, ResolutionInput,
    ResolutionReason, PRODUCER_MANIFEST_SHA256,
};
use compact_contract::country_tag::ENGINE_COUNTRY_TAG_LEN;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

pub const GAME_STATE_SINGLETON_RVA: usize = 0x3316E90;
pub const COUNTRY_TAG_DB_RVA: usize = 0x33189C8;
pub const DATE_EPOCH_CONST: i32 = 43_799_999;
const GS_ACTIVE_OFF: usize = 0x0A39;
const GS_CURRENT_TICK_OFF: usize = 0x0468;
const GS_NEXT_EVENT_INSTANCE_OFF: usize = 0x0498;
const GS_FIRED_EVENT_SET_OFF: usize = 0x0538;
const GS_PENDING_EVENTS_OFF: usize = 0x0560;
const GS_PENDING_CAPACITY_OFF: usize = 0x0568;
const GS_PENDING_COUNT_OFF: usize = 0x056C;
const PENDING_STRIDE: usize = 0x38;
const EVENT_WINDOW_DUE_TICK_OFF: usize = 0x1268;
const MAX_PENDING_EVENTS: u32 = 1_000_000;
const MAX_FIRED_BUCKETS: u32 = 1_000_000;
const COUNTRY_TAG_VECTOR_CAPACITY_OFF: usize =
    compact_contract::country_tag_db::TAG_STRING_CAPACITY_OFFSET;
const COUNTRY_TAG_VECTOR_SIZE_OFF: usize = compact_contract::country_tag_db::TAG_STRING_SIZE_OFFSET;
const COUNTRY_TAG_VECTOR_STRIDE: usize = compact_contract::country_tag_db::TAG_STRING_STRIDE;

pub const EVENT_NAMESPACE_TABLE_RVA: usize = 0x3316F98;
pub const EVENT_NAMESPACE_COUNT_RVA: usize = 0x3316FA4;
const EVENT_NAMESPACE_STRIDE: usize = 0x28;
const EVENT_ID_NAMESPACE_SPAN: u32 = 100_000;
const EVENT_ID_FIRST_NAMESPACE_SLOT: u32 = 10;
const EVENT_ID_BARE_LIMIT: u32 = EVENT_ID_FIRST_NAMESPACE_SLOT * EVENT_ID_NAMESPACE_SPAN;
const MAX_EVENT_NAMESPACES: u32 = 42_940;
const MAX_EVENT_NAMESPACE_LEN: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum A729EventClass {
    Country,
    News,
    State,
    UnitLeader,
    OperativeLeader,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct A729EventDefinition {
    pub event_id: String,
    pub class: A729EventClass,
    pub option_count: u32,
}

fn definition_declares(definition: &A729EventDefinition, namespace: &str) -> bool {
    definition
        .event_id
        .rsplit_once('.')
        .is_some_and(|(declared, _)| declared.eq_ignore_ascii_case(namespace))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct A729AliasedEvent {
    pub declaring_namespace: String,
    pub reduced_id: u32,
    pub definition: A729EventDefinition,
}

pub trait A729DefinitionRegistry: Send + Sync + 'static {
    fn get(&self, namespace: &str, numeric_id: u32) -> Option<A729EventDefinition>;

    fn aliased(&self) -> Vec<A729AliasedEvent>;

    fn bare(&self, numeric_id: u32) -> Option<A729EventDefinition>;
}

pub trait A729Memory: Send + Sync + 'static {
    fn read_exact(&self, address: usize, out: &mut [u8]) -> bool;

    fn u8(&self, address: usize) -> Option<u8> {
        let mut bytes = [0u8; 1];
        self.read_exact(address, &mut bytes).then_some(bytes[0])
    }

    fn u32(&self, address: usize) -> Option<u32> {
        let mut bytes = [0u8; 4];
        self.read_exact(address, &mut bytes)
            .then_some(u32::from_le_bytes(bytes))
    }

    fn i32(&self, address: usize) -> Option<i32> {
        self.u32(address).map(|value| value as i32)
    }

    fn usize(&self, address: usize) -> Option<usize> {
        let mut bytes = [0u8; 8];
        self.read_exact(address, &mut bytes)
            .then_some(u64::from_le_bytes(bytes) as usize)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PopupTextOwnerBindings {
    pub title_owner: Option<usize>,
    pub description_owner: Option<usize>,
    pub option_owners: Vec<(u16, usize)>,
}

pub const TITLE_TEXT_SETTER_CALLER_RVA: u32 = 0x0122_6339;
pub const DESCRIPTION_TEXT_SETTER_CALLER_RVA: u32 = 0x0122_6D89;
pub const OPTION_TEXT_SETTER_CALLER_RVA: u32 = 0x0122_3DF0;

pub trait A729PopupTextOwnerJoin: Send + Sync + 'static {
    fn classify(
        &self,
        event_window: usize,
        event: usize,
        visible_option_ordinals: &[u16],
        observations: &[ObservedRuntimeTextOwner],
    ) -> Result<PopupTextOwnerBindings, String>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ExecutableA729PopupTextOwnerJoin;

impl A729PopupTextOwnerJoin for ExecutableA729PopupTextOwnerJoin {
    fn classify(
        &self,
        event_window: usize,
        event: usize,
        visible_option_ordinals: &[u16],
        observations: &[ObservedRuntimeTextOwner],
    ) -> Result<PopupTextOwnerBindings, String> {
        if event_window == 0 || event == 0 {
            return Err("XUI21 popup owner classifier roots are null".into());
        }
        if visible_option_ordinals
            .windows(2)
            .any(|pair| pair[0] >= pair[1])
        {
            return Err("XUI21 popup owner classifier ordinals are not strictly increasing".into());
        }

        let owners_at = |caller_rva| {
            observations
                .iter()
                .filter(|row| row.setter_caller_rva == caller_rva)
                .map(|row| row.textbox_owner)
                .collect::<Vec<_>>()
        };
        let title = owners_at(TITLE_TEXT_SETTER_CALLER_RVA);
        let description = owners_at(DESCRIPTION_TEXT_SETTER_CALLER_RVA);
        let options = owners_at(OPTION_TEXT_SETTER_CALLER_RVA);
        if title.len() != 1 {
            return Err(format!(
                "XUI21 title setter producer count is {}, expected one",
                title.len()
            ));
        }
        if description.len() != 1 {
            return Err(format!(
                "XUI21 description setter producer count is {}, expected one",
                description.len()
            ));
        }
        if !visible_option_ordinals.is_empty() && options.len() != visible_option_ordinals.len() {
            return Err(format!(
                "XUI21 option setter producer count {} does not match {} visible definitions",
                options.len(),
                visible_option_ordinals.len()
            ));
        }
        if visible_option_ordinals.is_empty() && options.len() != 1 {
            return Err(format!(
                "XUI21 no-choice popup produced {} synthetic option textboxes, expected one",
                options.len()
            ));
        }
        let distinct_owners: BTreeSet<usize> = title
            .iter()
            .chain(&description)
            .chain(&options)
            .copied()
            .collect();
        if distinct_owners.len() != title.len() + description.len() + options.len() {
            return Err("XUI21 popup roles reuse a CTextBox owner".into());
        }

        Ok(PopupTextOwnerBindings {
            title_owner: Some(title[0]),
            description_owner: Some(description[0]),
            option_owners: visible_option_ordinals
                .iter()
                .copied()
                .zip(options)
                .collect(),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct FireKey {
    event: usize,
    actor_tag_id: u32,
    recipient_tag_id: u32,
}

#[derive(Default)]
struct BridgeState {
    instance_to_sequence: BTreeMap<u32, InstanceBinding>,
    unbound_by_key: BTreeMap<FireKey, Vec<u64>>,
    pending_before_by_sequence: BTreeMap<u64, BTreeSet<u32>>,
    fired_shadow_expected: BTreeSet<usize>,
    coverage: CoverageState,
    sequence_coverage: BTreeMap<u64, SequenceCoverage>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct CoverageState {
    matrix: u32,
    observations: BTreeMap<u8, CoverageMatrixObservation>,
    lifecycle: CoverageLifecycleWitnesses,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SequenceCoverage {
    event: usize,
    event_id: String,
    recipient_tag: String,
    fire_date: String,
    popup_subtype: PopupSubtype,
    fire_hour: u64,
    fire_tick: u64,
    materialised: bool,
}

fn popup_subtype_index(subtype: PopupSubtype) -> u32 {
    match subtype {
        PopupSubtype::Country => 0,
        PopupSubtype::News => 1,
        PopupSubtype::State => 2,
        PopupSubtype::UnitLeader => 3,
        PopupSubtype::OperativeLeader => 4,
    }
}

static SESSION_ORDINAL: AtomicU64 = AtomicU64::new(0);

fn fresh_session_id() -> String {
    let ordinal = SESSION_ORDINAL.fetch_add(1, Ordering::AcqRel) + 1;
    let unix_nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    format!("p{}-n{unix_nanos}-s{ordinal}", std::process::id())
}

fn manifest_sha256_hex() -> String {
    PRODUCER_MANIFEST_SHA256
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct InstanceBinding {
    sequence: u64,
    event: usize,
    recipient_tag_id: u32,
}

pub struct ExactA729CollectorBridge<M: A729Memory, R: A729DefinitionRegistry> {
    memory: Arc<M>,
    registry: Arc<R>,
    module_base: usize,
    game_state: usize,
    next_sequence: AtomicU64,
    session_id: String,
    popup_text_owner_join: Option<Arc<dyn A729PopupTextOwnerJoin>>,
    namespace_slots: Mutex<Option<(u32, Arc<BTreeMap<String, u32>>)>>,
    state: Mutex<BridgeState>,
}

impl<M: A729Memory, R: A729DefinitionRegistry> ExactA729CollectorBridge<M, R> {
    pub fn new(
        memory: Arc<M>,
        registry: Arc<R>,
        module_base: usize,
        game_state: usize,
        coverage: CoverageAttestation,
    ) -> Result<Self, String> {
        if module_base == 0 || game_state == 0 {
            return Err("XUI21 module/game-state root is null".into());
        }
        if memory.usize(module_base + GAME_STATE_SINGLETON_RVA) != Some(game_state) {
            return Err("XUI21 game-state singleton does not match collector root".into());
        }
        if coverage != CoverageAttestation::default() {
            return Err(
                "XUI21 runtime matrix cannot be seeded; every session receipt starts empty".into(),
            );
        }
        let next = memory
            .u32(game_state + GS_NEXT_EVENT_INSTANCE_OFF)
            .ok_or_else(|| "XUI21 event sequence root is unreadable".to_string())?;
        let bridge = Self {
            memory,
            registry,
            module_base,
            game_state,
            next_sequence: AtomicU64::new(u64::from(next)),
            session_id: fresh_session_id(),
            popup_text_owner_join: None,
            namespace_slots: Mutex::new(None),
            state: Mutex::new(BridgeState::default()),
        };
        let fired = bridge.read_fired_event_set()?;
        bridge
            .state
            .lock()
            .map_err(|_| "XUI21 bridge mutex poisoned".to_string())?
            .fired_shadow_expected = fired;
        Ok(bridge)
    }

    pub fn with_popup_text_owner_join(mut self, join: Arc<dyn A729PopupTextOwnerJoin>) -> Self {
        self.popup_text_owner_join = Some(join);
        self
    }

    fn runtime_text_bindings(
        &self,
        raw: &RawMaterialisation,
        visible_option_ordinals: &[u16],
    ) -> Result<
        (
            Option<String>,
            Option<String>,
            Vec<OptionRuntimeTextContext>,
        ),
        String,
    > {
        let Some(join) = self.popup_text_owner_join.as_ref() else {
            return Ok((None, None, Vec::new()));
        };
        let bindings = join.classify(
            raw.event_window,
            raw.event,
            visible_option_ordinals,
            &raw.runtime_text_owners,
        )?;
        let observed = raw
            .runtime_text_owners
            .iter()
            .map(|row| (row.textbox_owner, row.context_id.as_str()))
            .collect::<BTreeMap<_, _>>();
        let mut used_owners = BTreeSet::new();
        let mut context_for = |owner: usize, label: &str| -> Result<String, String> {
            if owner == 0 || !used_owners.insert(owner) {
                return Err(format!("XUI21 {label} textbox owner is null or reused"));
            }
            observed
                .get(&owner)
                .filter(|context| !context.trim().is_empty())
                .map(|context| (*context).to_string())
                .ok_or_else(|| format!("XUI21 {label} textbox owner was not observed by A25"))
        };
        let title = bindings
            .title_owner
            .map(|owner| context_for(owner, "title"))
            .transpose()?;
        let description = bindings
            .description_owner
            .map(|owner| context_for(owner, "description"))
            .transpose()?;
        let mut option_contexts = Vec::with_capacity(bindings.option_owners.len());
        for (ordinal, owner) in bindings.option_owners {
            if !visible_option_ordinals.contains(&ordinal) {
                return Err(
                    "XUI21 runtime option textbox is not a visible definition ordinal".into(),
                );
            }
            option_contexts.push(OptionRuntimeTextContext {
                definition_ordinal: ordinal,
                context_id: context_for(owner, "option")?,
            });
        }
        option_contexts.sort_by_key(|row| row.definition_ordinal);
        if option_contexts
            .windows(2)
            .any(|pair| pair[0].definition_ordinal == pair[1].definition_ordinal)
        {
            return Err("XUI21 duplicate runtime option textbox ordinal".into());
        }
        Ok((title, description, option_contexts))
    }

    fn engine_namespace_count(&self) -> Result<u32, String> {
        self.memory
            .u32(self.module_base + EVENT_NAMESPACE_COUNT_RVA)
            .filter(|count| *count <= MAX_EVENT_NAMESPACES)
            .ok_or_else(|| "XUI21 event namespace count is invalid".to_string())
    }

    fn engine_event_namespace(&self, slot_index: u32) -> Result<Option<String>, String> {
        let count = self.engine_namespace_count()?;
        if slot_index >= count {
            return Ok(None);
        }
        let table = self
            .memory
            .usize(self.module_base + EVENT_NAMESPACE_TABLE_RVA)
            .filter(|value| *value != 0)
            .ok_or_else(|| "XUI21 event namespace table is unreadable".to_string())?;
        let address = (slot_index as usize)
            .checked_mul(EVENT_NAMESPACE_STRIDE)
            .and_then(|offset| table.checked_add(offset))
            .ok_or_else(|| "XUI21 event namespace row address overflow".to_string())?;
        let namespace = self.read_msvc_string(address, MAX_EVENT_NAMESPACE_LEN)?;
        if namespace.is_empty()
            || namespace
                .chars()
                .any(|value| value.is_whitespace() || value.is_control())
        {
            return Err(format!("XUI21 invalid event namespace {namespace:?}"));
        }
        Ok(Some(namespace))
    }

    fn engine_namespace_slots(&self) -> Result<Arc<BTreeMap<String, u32>>, String> {
        let count = self.engine_namespace_count()?;
        {
            let cache = self
                .namespace_slots
                .lock()
                .map_err(|_| "XUI21 namespace slot cache poisoned".to_string())?;
            if let Some((cached_count, slots)) = cache.as_ref() {
                if *cached_count == count {
                    return Ok(Arc::clone(slots));
                }
            }
        }
        let mut slots = BTreeMap::new();
        for slot_index in 0..count {
            let namespace = self
                .engine_event_namespace(slot_index)?
                .ok_or_else(|| "XUI21 event namespace table shrank while reading".to_string())?;
            if slots
                .insert(namespace.to_ascii_lowercase(), slot_index)
                .is_some()
            {
                return Err(format!("XUI21 duplicate engine namespace {namespace:?}"));
            }
        }
        let slots = Arc::new(slots);
        *self
            .namespace_slots
            .lock()
            .map_err(|_| "XUI21 namespace slot cache poisoned".to_string())? =
            Some((count, Arc::clone(&slots)));
        Ok(slots)
    }

    fn engine_event_id(slot_index: u32, reduced_id: u32) -> Option<u32> {
        let base = slot_index
            .wrapping_add(EVENT_ID_FIRST_NAMESPACE_SLOT)
            .wrapping_mul(EVENT_ID_NAMESPACE_SPAN);
        if (base as i32) < 0 {
            return None;
        }
        Some(base.wrapping_add(reduced_id))
    }

    fn definition(&self, event: usize) -> Result<A729EventDefinition, String> {
        let combined_id = self
            .memory
            .u32(
                event
                    .checked_add(0x0C)
                    .ok_or("XUI21 event address overflow")?,
            )
            .ok_or_else(|| "XUI21 CEvent combined id is unreadable".to_string())?;
        self.engine_namespace_count()?;
        let slot = combined_id / EVENT_ID_NAMESPACE_SPAN;
        let numeric_id = combined_id % EVENT_ID_NAMESPACE_SPAN;
        let mut decoded_namespace = None;
        let mut candidates: Vec<A729EventDefinition> = Vec::new();
        if combined_id < EVENT_ID_BARE_LIMIT {
            candidates.extend(self.registry.bare(combined_id));
        }
        if slot >= EVENT_ID_FIRST_NAMESPACE_SLOT {
            let slot_index = slot - EVENT_ID_FIRST_NAMESPACE_SLOT;
            if Self::engine_event_id(slot_index, numeric_id) == Some(combined_id) {
                if let Some(namespace) = self.engine_event_namespace(slot_index)? {
                    candidates.extend(
                        self.registry
                            .get(&namespace, numeric_id)
                            .filter(|definition| definition_declares(definition, &namespace)),
                    );
                    decoded_namespace = Some(namespace);
                }
            }
        }
        let aliased = self.registry.aliased();
        if !aliased.is_empty() {
            let slots = self.engine_namespace_slots()?;
            for alias in aliased {
                let Some(slot_index) = slots.get(&alias.declaring_namespace.to_ascii_lowercase())
                else {
                    continue;
                };
                if Self::engine_event_id(*slot_index, alias.reduced_id) == Some(combined_id)
                    && definition_declares(&alias.definition, &alias.declaring_namespace)
                {
                    candidates.push(alias.definition);
                }
            }
        }
        match candidates.len() {
            1 => Ok(candidates.remove(0)),
            0 => Err(match decoded_namespace {
                Some(namespace) => format!(
                    "XUI21 unknown event {namespace}.{numeric_id} (combined id {combined_id})"
                ),
                None => {
                    format!("XUI21 CEvent combined id {combined_id} names no registered namespace")
                }
            }),
            _ => {
                let ambiguous: Vec<&str> = candidates
                    .iter()
                    .map(|definition| definition.event_id.as_str())
                    .collect();
                Err(format!(
                    "XUI21 combined event id {combined_id} is ambiguous between {ambiguous:?}"
                ))
            }
        }
    }

    fn tag_string(&self, tag_id: u32) -> Result<String, String> {
        if tag_id == 0 {
            return Err(format!("XUI21 invalid recipient tag id {tag_id}"));
        }
        let db = self
            .memory
            .usize(self.module_base + COUNTRY_TAG_DB_RVA)
            .filter(|value| *value != 0)
            .ok_or_else(|| "XUI21 country-tag DB root is unreadable".to_string())?;
        let rows = self
            .memory
            .usize(db)
            .filter(|value| *value != 0)
            .ok_or_else(|| "XUI21 country-tag table is unreadable".to_string())?;
        let capacity = self
            .memory
            .u32(
                db.checked_add(COUNTRY_TAG_VECTOR_CAPACITY_OFF)
                    .ok_or("XUI21 country-tag capacity address overflow")?,
            )
            .ok_or_else(|| "XUI21 country-tag capacity is unreadable".to_string())?;
        let count = self
            .memory
            .u32(
                db.checked_add(COUNTRY_TAG_VECTOR_SIZE_OFF)
                    .ok_or("XUI21 country-tag count address overflow")?,
            )
            .ok_or_else(|| "XUI21 country-tag count is unreadable".to_string())?;
        if !compact_contract::country_tag_db::tag_db_size_structurally_valid(count, capacity) {
            return Err(format!(
                "XUI21 country-tag vector is not structurally valid (size {count}, capacity {capacity})"
            ));
        }
        if !compact_contract::country_tag_db::tag_db_id_in_table(tag_id, count) {
            return Err(format!("XUI21 recipient tag id {tag_id} exceeds {count}"));
        }
        let address = rows
            .checked_add(tag_id as usize * COUNTRY_TAG_VECTOR_STRIDE)
            .ok_or_else(|| "XUI21 country-tag row address overflow".to_string())?;
        let value = self.read_msvc_string(address, ENGINE_COUNTRY_TAG_LEN)?;
        if !compact_contract::country_tag::is_engine_country_tag_or_no_country_sentinel(&value) {
            return Err(format!("XUI21 invalid country tag {value:?}"));
        }
        Ok(value)
    }

    fn read_msvc_string(&self, address: usize, max_len: usize) -> Result<String, String> {
        // so the arithmetic is checked. Release builds enable overflow checks,
        let overflow = || "XUI21 MSVC string address overflow".to_string();
        let length = self
            .memory
            .usize(address.checked_add(0x10).ok_or_else(overflow)?)
            .filter(|length| *length <= max_len)
            .ok_or_else(|| "XUI21 MSVC string length is invalid".to_string())?;
        let capacity = self
            .memory
            .usize(address.checked_add(0x18).ok_or_else(overflow)?)
            .filter(|capacity| *capacity >= length)
            .ok_or_else(|| "XUI21 MSVC string capacity is invalid".to_string())?;
        let data = if capacity > 0x0F {
            self.memory
                .usize(address)
                .filter(|value| *value != 0)
                .ok_or_else(|| "XUI21 MSVC string heap pointer is invalid".to_string())?
        } else {
            address
        };
        data.checked_add(length).ok_or_else(overflow)?;
        let mut bytes = vec![0u8; length];
        if !self.memory.read_exact(data, &mut bytes) {
            return Err("XUI21 MSVC string bytes are unreadable".into());
        }
        String::from_utf8(bytes).map_err(|_| "XUI21 MSVC string is not UTF-8".into())
    }

    fn current_tick(&self) -> Result<u32, String> {
        self.memory
            .u32(self.game_state + GS_CURRENT_TICK_OFF)
            .ok_or_else(|| "XUI21 current game tick is unreadable".to_string())
    }

    fn fire_row(
        &self,
        event: usize,
        recipient_tag_id: u32,
        fire_tick: u32,
        event_sequence: u64,
    ) -> Result<FireInput, String> {
        let definition = self.definition(event)?;
        let timeout_days = self
            .memory
            .u32(event + 0x408)
            .ok_or_else(|| "XUI21 event timeout is unreadable".to_string())?;
        let effective_timeout_hours = timeout_days
            .checked_mul(24)
            .ok_or_else(|| "XUI21 event timeout overflows hours".to_string())?;
        let timeout_tick = fire_tick
            .checked_add(effective_timeout_hours)
            .ok_or_else(|| "XUI21 event timeout tick overflows".to_string())?;
        let fire_hour = i64::from(fire_tick) - i64::from(DATE_EPOCH_CONST);
        if fire_hour < 0 {
            return Err("XUI21 fire tick predates the engine epoch".into());
        }
        let popup_subtype = runtime_popup_subtype(self.memory.as_ref(), event, definition.class)?;
        Ok(FireInput {
            event_id: definition.event_id,
            recipient_tag: self.tag_string(recipient_tag_id)?,
            fire_date: decode_datehour(fire_tick as i32)
                .ok_or_else(|| "XUI21 fire date is outside the authenticated range".to_string())?,
            fire_hour: fire_hour as u64,
            fire_tick: u64::from(fire_tick),
            event_sequence,
            popup_subtype,
            effective_timeout_hours,
            timeout_tick: u64::from(timeout_tick),
        })
    }

    fn pending_rows(&self) -> Result<Vec<PendingRow>, String> {
        let base = self
            .memory
            .usize(self.game_state + GS_PENDING_EVENTS_OFF)
            .ok_or_else(|| "XUI21 pending-event vector root is unreadable".to_string())?;
        let capacity = self
            .memory
            .u32(self.game_state + GS_PENDING_CAPACITY_OFF)
            .ok_or_else(|| "XUI21 pending-event capacity is unreadable".to_string())?;
        let count = self
            .memory
            .u32(self.game_state + GS_PENDING_COUNT_OFF)
            .filter(|count| *count <= capacity && *count <= MAX_PENDING_EVENTS)
            .ok_or_else(|| "XUI21 pending-event count/capacity is invalid".to_string())?;
        if count != 0 && base == 0 {
            return Err("XUI21 nonempty pending-event vector has a null root".into());
        }
        let mut rows = Vec::with_capacity(count as usize);
        for index in 0..count as usize {
            rows.push(self.pending_row_at(base, index)?);
        }
        if self.memory.usize(self.game_state + GS_PENDING_EVENTS_OFF) != Some(base)
            || self.memory.u32(self.game_state + GS_PENDING_CAPACITY_OFF) != Some(capacity)
            || self.memory.u32(self.game_state + GS_PENDING_COUNT_OFF) != Some(count)
        {
            return Err("XUI21 pending-event vector changed during enumeration".into());
        }
        for (index, expected) in rows.iter().enumerate() {
            if self.pending_row_at(base, index)? != *expected {
                return Err(format!(
                    "XUI21 pending row {index} changed during enumeration"
                ));
            }
        }
        Ok(rows)
    }

    fn pending_row_at(&self, base: usize, index: usize) -> Result<PendingRow, String> {
        let row = base
            .checked_add(index * PENDING_STRIDE)
            .ok_or_else(|| "XUI21 pending-event row address overflow".to_string())?;
        let instance_id = self
            .memory
            .u32(row)
            .filter(|value| *value != 0)
            .ok_or_else(|| format!("XUI21 pending row {index} has invalid instance id"))?;
        let event = self
            .memory
            .usize(row + 0x08)
            .filter(|value| *value != 0)
            .ok_or_else(|| format!("XUI21 pending row {index} has invalid CEvent"))?;
        let scope = self
            .memory
            .usize(row + 0x10)
            .filter(|value| *value != 0)
            .ok_or_else(|| format!("XUI21 pending row {index} has invalid CEventScope"))?;
        let actor_tag_id = self
            .memory
            .u32(row + 0x18)
            .ok_or_else(|| format!("XUI21 pending row {index} actor is unreadable"))?;
        let recipient_tag_id = self
            .memory
            .u32(scope + 0x08)
            .filter(|value| *value != 0)
            .ok_or_else(|| format!("XUI21 pending row {index} recipient is invalid"))?;
        let due_tick = self
            .memory
            .u32(row + 0x28)
            .ok_or_else(|| format!("XUI21 pending row {index} due tick is unreadable"))?;
        Ok(PendingRow {
            instance_id,
            event,
            actor_tag_id,
            recipient_tag_id,
            due_tick,
        })
    }

    fn read_fired_event_set(&self) -> Result<BTreeSet<usize>, String> {
        let set = self.game_state + GS_FIRED_EVENT_SET_OFF;
        let count = self
            .memory
            .u32(set)
            .filter(|count| *count <= MAX_PENDING_EVENTS)
            .ok_or_else(|| "XUI21 fired-event set count is invalid".to_string())?;
        let bucket_count = self
            .memory
            .u32(set + 0x04)
            .filter(|buckets| *buckets > 0 && *buckets <= MAX_FIRED_BUCKETS)
            .ok_or_else(|| "XUI21 fired-event bucket count is invalid".to_string())?;
        let buckets = self
            .memory
            .usize(set + 0x08)
            .filter(|value| *value != 0)
            .ok_or_else(|| "XUI21 fired-event bucket root is invalid".to_string())?;
        let mut events = BTreeSet::new();
        let mut nodes = BTreeSet::new();
        for bucket in 0..bucket_count as usize {
            let mut node = self
                .memory
                .usize(buckets + bucket * 8)
                .ok_or_else(|| "XUI21 fired-event bucket is unreadable".to_string())?;
            while node != 0 {
                if nodes.len() >= count as usize || !nodes.insert(node) {
                    return Err("XUI21 fired-event hash chain is cyclic/oversized".into());
                }
                let event = self
                    .memory
                    .usize(node)
                    .filter(|value| *value != 0)
                    .ok_or_else(|| "XUI21 fired-event node has invalid CEvent".to_string())?;
                if !events.insert(event) {
                    return Err("XUI21 fired-event set contains a duplicate CEvent".into());
                }
                node = self
                    .memory
                    .usize(node + 0x08)
                    .ok_or_else(|| "XUI21 fired-event next pointer is unreadable".to_string())?;
            }
        }
        if nodes.len() != count as usize
            || self.memory.u32(set) != Some(count)
            || self.memory.u32(set + 0x04) != Some(bucket_count)
            || self.memory.usize(set + 0x08) != Some(buckets)
        {
            return Err("XUI21 fired-event set changed or count did not reconcile".into());
        }
        Ok(events)
    }

    fn bind_pending_instance(&self, key: FireKey, sequence: u64) -> Result<(), String> {
        let pending = match self.pending_rows() {
            Ok(rows) => rows,
            Err(error) => {
                self.discard_inflight_sequence(sequence)?;
                return Err(error);
            }
        };
        let matches: Vec<PendingRow> = pending
            .into_iter()
            .filter(|row| {
                row.event == key.event
                    && row.actor_tag_id == key.actor_tag_id
                    && row.recipient_tag_id == key.recipient_tag_id
            })
            .collect();
        if matches.is_empty() {
            let mut state = self
                .state
                .lock()
                .map_err(|_| "XUI21 bridge mutex poisoned".to_string())?;
            state.pending_before_by_sequence.remove(&sequence);
            remove_unbound(&mut state, key, sequence);
            return Ok(());
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| "XUI21 bridge mutex poisoned".to_string())?;
        let Some(before) = state.pending_before_by_sequence.remove(&sequence) else {
            remove_unbound(&mut state, key, sequence);
            return Err("XUI21 pre-fire pending baseline is missing".into());
        };
        let created: Vec<&PendingRow> = matches
            .iter()
            .filter(|row| {
                !before.contains(&row.instance_id)
                    && !state.instance_to_sequence.contains_key(&row.instance_id)
            })
            .collect();
        let Some(chosen) = created.first().copied().filter(|_| created.len() == 1) else {
            remove_unbound(&mut state, key, sequence);
            return Err("XUI21 pending instance set difference is missing/ambiguous".into());
        };
        state.instance_to_sequence.insert(
            chosen.instance_id,
            InstanceBinding {
                sequence,
                event: chosen.event,
                recipient_tag_id: chosen.recipient_tag_id,
            },
        );
        remove_unbound(&mut state, key, sequence);
        Ok(())
    }

    fn discard_inflight_sequence(&self, sequence: u64) -> Result<(), String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "XUI21 bridge mutex poisoned".to_string())?;
        state.pending_before_by_sequence.remove(&sequence);
        state.sequence_coverage.remove(&sequence);
        for values in state.unbound_by_key.values_mut() {
            values.retain(|value| *value != sequence);
        }
        state.unbound_by_key.retain(|_, values| !values.is_empty());
        Ok(())
    }

    fn sequence_for_instance_or_event(
        &self,
        instance_id: u32,
        event: usize,
        recipient_tag_id: u32,
    ) -> Result<u64, String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "XUI21 bridge mutex poisoned".to_string())?;
        if let Some(binding) = state.instance_to_sequence.get(&instance_id) {
            if binding.event != event || binding.recipient_tag_id != recipient_tag_id {
                return Err("XUI21 instance id is bound to a different event/recipient".to_string());
            }
            return Ok(binding.sequence);
        }
        let candidates: Vec<(FireKey, u64)> = state
            .unbound_by_key
            .iter()
            .filter(|(key, values)| {
                key.event == event && key.recipient_tag_id == recipient_tag_id && !values.is_empty()
            })
            .flat_map(|(key, values)| values.iter().copied().map(|value| (*key, value)))
            .collect();
        let (key, sequence) = candidates
            .first()
            .copied()
            .filter(|_| candidates.len() == 1)
            .ok_or_else(|| "XUI21 event sequence correlation is missing/ambiguous".to_string())?;
        if instance_id != 0 {
            state.instance_to_sequence.insert(
                instance_id,
                InstanceBinding {
                    sequence,
                    event,
                    recipient_tag_id,
                },
            );
        }
        remove_unbound(&mut state, key, sequence);
        Ok(sequence)
    }

    fn retire_resolved_instance(
        &self,
        instance_id: u32,
        event: usize,
        recipient_tag_id: u32,
        sequence: u64,
        path: ResolutionPath,
        chosen_option_ordinal: Option<u16>,
        resolved_tick: u64,
    ) -> Result<(), String> {
        if instance_id == 0 {
            return Err("XUI21 resolved lifecycle has no instance id".into());
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| "XUI21 bridge mutex poisoned".to_string())?;
        let Some(binding) = state.instance_to_sequence.get(&instance_id).copied() else {
            return Err("XUI21 resolved instance binding disappeared".into());
        };
        if binding.sequence != sequence
            || binding.event != event
            || binding.recipient_tag_id != recipient_tag_id
        {
            return Err("XUI21 resolved instance binding changed before retirement".into());
        }
        let coverage = state
            .sequence_coverage
            .get(&sequence)
            .cloned()
            .ok_or_else(|| "XUI21 resolved coverage sequence disappeared".to_string())?;
        let (route, route_name) = match (path, chosen_option_ordinal) {
            (ResolutionPath::Player, Some(_)) => (0, "player_selection"),
            (ResolutionPath::Ai, Some(_)) => (1, "ai_selection"),
            (ResolutionPath::Timeout, Some(_)) => (2, "timeout_default_option"),
            (ResolutionPath::Timeout, None) => (3, "timeout_no_choice"),
            _ => return Err("XUI21 resolution route is not a matrix cell".into()),
        };
        let cell_index = (popup_subtype_index(coverage.popup_subtype) * 4 + route) as u8;
        state.instance_to_sequence.remove(&instance_id);
        state.coverage.matrix |= 1 << cell_index;
        let observation = CoverageMatrixObservation {
            cell_index,
            popup_class: match coverage.popup_subtype {
                PopupSubtype::Country => "country",
                PopupSubtype::News => "news",
                PopupSubtype::State => "state",
                PopupSubtype::UnitLeader => "unit_leader",
                PopupSubtype::OperativeLeader => "operative_leader",
            }
            .to_string(),
            resolution_path: route_name.to_string(),
            event_id: coverage.event_id.clone(),
            recipient_tag: coverage.recipient_tag.clone(),
            fire_date: coverage.fire_date.clone(),
            fire_hour: coverage.fire_hour,
            fire_tick: coverage.fire_tick,
            event_sequence: sequence,
            instance_id,
            chosen_option_ordinal,
            resolved_tick,
            materialised_before_resolution: coverage.materialised,
        };
        match state.coverage.observations.entry(cell_index) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(observation);
            }
            std::collections::btree_map::Entry::Occupied(mut entry)
                if path == ResolutionPath::Ai
                    && !coverage.materialised
                    && entry.get().materialised_before_resolution =>
            {
                entry.insert(observation);
            }
            std::collections::btree_map::Entry::Occupied(_) => {}
        }
        if path == ResolutionPath::Ai && !coverage.materialised {
            state
                .coverage
                .lifecycle
                .non_materialised_resolution
                .get_or_insert_with(|| NonMaterialisedResolutionWitness {
                    event_id: coverage.event_id.clone(),
                    event_sequence: sequence,
                    resolution_path: "ai_selection".to_string(),
                });
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PendingRow {
    instance_id: u32,
    event: usize,
    actor_tag_id: u32,
    recipient_tag_id: u32,
    due_tick: u32,
}

impl<M: A729Memory, R: A729DefinitionRegistry> A729EventBridge for ExactA729CollectorBridge<M, R> {
    fn producer_manifest_sha256(&self) -> [u8; 32] {
        PRODUCER_MANIFEST_SHA256
    }

    fn pending_baseline(&self) -> Result<PendingBaseline, String> {
        let pending = self.pending_rows()?;
        let capture_start_pending = !pending.is_empty();
        let mut rows = Vec::with_capacity(pending.len());
        let mut state = self
            .state
            .lock()
            .map_err(|_| "XUI21 bridge mutex poisoned".to_string())?;
        let mut max_sequence = self.next_sequence.load(Ordering::Acquire);
        for pending in pending {
            let definition = self.definition(pending.event)?;
            let timeout_hours = self
                .memory
                .u32(pending.event + 0x408)
                .and_then(|days| days.checked_mul(24))
                .ok_or_else(|| "XUI21 baseline timeout is invalid".to_string())?;
            let fire_tick = pending
                .due_tick
                .checked_sub(timeout_hours)
                .ok_or_else(|| "XUI21 baseline due tick predates fire tick".to_string())?;
            let sequence = u64::from(pending.instance_id);
            if state
                .instance_to_sequence
                .insert(
                    pending.instance_id,
                    InstanceBinding {
                        sequence,
                        event: pending.event,
                        recipient_tag_id: pending.recipient_tag_id,
                    },
                )
                .is_some()
            {
                return Err("XUI21 baseline contains duplicate instance ids".into());
            }
            max_sequence = max_sequence.max(sequence);
            let mut row =
                self.fire_row(pending.event, pending.recipient_tag_id, fire_tick, sequence)?;
            row.event_id = definition.event_id;
            state.sequence_coverage.insert(
                sequence,
                SequenceCoverage {
                    event: pending.event,
                    event_id: row.event_id.clone(),
                    recipient_tag: row.recipient_tag.clone(),
                    fire_date: row.fire_date.clone(),
                    popup_subtype: row.popup_subtype,
                    fire_hour: row.fire_hour,
                    fire_tick: row.fire_tick,
                    materialised: false,
                },
            );
            state
                .coverage
                .lifecycle
                .capture_start_pending
                .get_or_insert_with(|| CaptureStartPendingWitness {
                    event_id: row.event_id.clone(),
                    event_sequence: sequence,
                });
            rows.push(row);
        }
        if !capture_start_pending {
            state.coverage.lifecycle.capture_start_pending = None;
        }
        rows.sort_by_key(|row| row.event_sequence);
        self.next_sequence.store(max_sequence, Ordering::Release);
        Ok(PendingBaseline {
            rows,
            complete: true,
        })
    }

    fn fire_input(&self, raw: RawFire) -> Result<Option<FireInput>, String> {
        let active = self
            .memory
            .u8(self.game_state + GS_ACTIVE_OFF)
            .ok_or_else(|| "XUI21 game-state active flag is unreadable".to_string())?;
        if active == 0 {
            return Ok(None);
        }
        if raw.event == 0 || raw.actor_tag.is_null() || raw.recipient_tag.is_null() {
            return Err("XUI21 fire producer arguments are invalid".into());
        }
        let fire_only_once = self
            .memory
            .u8(raw.event + 0x421)
            .ok_or_else(|| "XUI21 fire-only-once flag is unreadable".to_string())?
            != 0;
        if fire_only_once && self.read_fired_event_set()?.contains(&raw.event) {
            return Ok(None);
        }
        let actor_tag_id = self
            .memory
            .u32(raw.actor_tag as usize)
            .ok_or_else(|| "XUI21 actor tag is unreadable".to_string())?;
        let recipient_tag_id = self
            .memory
            .u32(raw.recipient_tag as usize)
            .filter(|value| *value != 0)
            .ok_or_else(|| "XUI21 recipient tag is invalid".to_string())?;
        let fire_tick = self.current_tick()?;
        let sequence = self.next_sequence.fetch_add(1, Ordering::AcqRel) + 1;
        let row = self.fire_row(raw.event, recipient_tag_id, fire_tick, sequence)?;
        let key = FireKey {
            event: raw.event,
            actor_tag_id,
            recipient_tag_id,
        };
        let pending_before = self
            .pending_rows()?
            .into_iter()
            .filter(|pending| {
                pending.event == key.event
                    && pending.actor_tag_id == key.actor_tag_id
                    && pending.recipient_tag_id == key.recipient_tag_id
            })
            .map(|pending| pending.instance_id)
            .collect();
        let mut state = self
            .state
            .lock()
            .map_err(|_| "XUI21 bridge mutex poisoned".to_string())?;
        let repeated_event_id_sequence = state
            .sequence_coverage
            .iter()
            .find(|(_, prior)| prior.event == raw.event)
            .map(|(sequence, prior)| (*sequence, prior.event_id.clone()));
        let same_hour_sequence = state
            .sequence_coverage
            .iter()
            .find(|(_, prior)| prior.fire_hour == row.fire_hour)
            .map(|(sequence, prior)| (*sequence, prior.fire_hour));
        if let Some((prior, event_id)) = repeated_event_id_sequence {
            state
                .coverage
                .lifecycle
                .repeated_event_id
                .get_or_insert(RepeatedEventIdWitness {
                    event_id,
                    first_sequence: prior,
                    second_sequence: sequence,
                });
        }
        if let Some((prior, fire_hour)) = same_hour_sequence {
            state
                .coverage
                .lifecycle
                .same_hour_multiple_firings
                .get_or_insert(SameHourFiringsWitness {
                    fire_hour,
                    first_sequence: prior,
                    second_sequence: sequence,
                });
        }
        state.sequence_coverage.insert(
            sequence,
            SequenceCoverage {
                event: raw.event,
                event_id: row.event_id.clone(),
                recipient_tag: row.recipient_tag.clone(),
                fire_date: row.fire_date.clone(),
                popup_subtype: row.popup_subtype,
                fire_hour: row.fire_hour,
                fire_tick: row.fire_tick,
                materialised: false,
            },
        );
        state
            .pending_before_by_sequence
            .insert(sequence, pending_before);
        state.unbound_by_key.entry(key).or_default().push(sequence);
        Ok(Some(row))
    }

    fn fire_committed(
        &self,
        raw: RawFire,
        event_sequence: u64,
        command_instance_id: Option<u32>,
    ) -> Result<(), String> {
        let Some(actor_tag_id) = self.memory.u32(raw.actor_tag as usize) else {
            self.discard_inflight_sequence(event_sequence)?;
            return Err("XUI21 committed actor tag is unreadable".into());
        };
        let Some(recipient_tag_id) = self.memory.u32(raw.recipient_tag as usize) else {
            self.discard_inflight_sequence(event_sequence)?;
            return Err("XUI21 committed recipient tag is unreadable".into());
        };
        let key = FireKey {
            event: raw.event,
            actor_tag_id,
            recipient_tag_id,
        };
        if let Some(instance_id) = command_instance_id.filter(|value| *value != 0) {
            let mut state = self
                .state
                .lock()
                .map_err(|_| "XUI21 bridge mutex poisoned".to_string())?;
            state.instance_to_sequence.insert(
                instance_id,
                InstanceBinding {
                    sequence: event_sequence,
                    event: raw.event,
                    recipient_tag_id,
                },
            );
            state.pending_before_by_sequence.remove(&event_sequence);
            remove_unbound(&mut state, key, event_sequence);
        } else {
            self.bind_pending_instance(key, event_sequence)?;
        }
        if self
            .memory
            .u8(raw.event + 0x421)
            .ok_or_else(|| "XUI21 committed fire-only-once flag is unreadable".to_string())?
            != 0
        {
            self.state
                .lock()
                .map_err(|_| "XUI21 bridge mutex poisoned".to_string())?
                .fired_shadow_expected
                .insert(raw.event);
        }
        Ok(())
    }

    fn materialisation_input(
        &self,
        raw: RawMaterialisation,
    ) -> Result<MaterialisationInput, String> {
        if raw.event == 0 || raw.event_window == 0 {
            return Err("XUI21 materialisation roots are invalid".into());
        }
        let definition = self.definition(raw.event)?;
        let runtime_count = self
            .memory
            .u32(raw.event + 0x374)
            .ok_or_else(|| "XUI21 runtime option count is unreadable".to_string())?;
        if runtime_count != definition.option_count || runtime_count > u16::MAX as u32 {
            return Err("XUI21 runtime/static option count mismatch".into());
        }
        let option_table = self
            .memory
            .usize(raw.event + 0x368)
            .ok_or_else(|| "XUI21 option table is unreadable".to_string())?;
        let mut visible_option_ordinals = Vec::with_capacity(raw.admitted_option_pointers.len());
        for pointer in raw.admitted_option_pointers.iter().copied() {
            let ordinal = (0..runtime_count)
                .find(|ordinal| {
                    self.memory.usize(option_table + *ordinal as usize * 8) == Some(pointer)
                })
                .ok_or_else(|| "XUI21 admitted option is not definition-owned".to_string())?;
            let ordinal = ordinal as u16;
            if visible_option_ordinals
                .last()
                .is_some_and(|last| *last >= ordinal)
            {
                return Err("XUI21 admitted option order is not strictly increasing".into());
            }
            visible_option_ordinals.push(ordinal);
        }
        let recipient_tag_id = self
            .memory
            .u32(raw.event_window + 0x80)
            .filter(|value| *value != 0)
            .ok_or_else(|| "XUI21 materialised recipient is invalid".to_string())?;
        let instance_id = self
            .memory
            .u32(raw.event_window + 0x60)
            .filter(|value| *value != 0)
            .ok_or_else(|| "XUI21 materialised instance id is invalid".to_string())?;
        let event_sequence =
            self.sequence_for_instance_or_event(instance_id, raw.event, recipient_tag_id)?;
        let popup_subtype =
            runtime_popup_subtype(self.memory.as_ref(), raw.event, definition.class)?;
        let timeout_tick = self
            .memory
            .u32(raw.event_window + EVENT_WINDOW_DUE_TICK_OFF)
            .filter(|value| *value != 0)
            .ok_or_else(|| "XUI21 materialised due tick is unreadable".to_string())?;
        let (
            runtime_title_text_context_id,
            runtime_description_text_context_id,
            runtime_option_text_contexts,
        ) = self.runtime_text_bindings(&raw, &visible_option_ordinals)?;
        {
            let mut state = self
                .state
                .lock()
                .map_err(|_| "XUI21 bridge mutex poisoned".to_string())?;
            let coverage = state
                .sequence_coverage
                .get_mut(&event_sequence)
                .ok_or_else(|| "XUI21 materialisation coverage sequence is missing".to_string())?;
            coverage.materialised = true;
        }
        Ok(MaterialisationInput {
            event_sequence,
            visible_option_ordinals,
            selected_desc_key: raw.selected_desc_key,
            shown_picture_gfx: raw.shown_picture_gfx,
            popup_subtype,
            timeout_tick: u64::from(timeout_tick),
            runtime_title_text_context_id,
            runtime_description_text_context_id,
            runtime_option_text_contexts,
        })
    }

    fn resolution_input(&self, raw: RawResolution) -> Result<Option<ResolutionInput>, String> {
        let resolved_tick = self.current_tick()?;
        if raw.path == ResolutionPath::Timeout {
            if raw.owner == 0 {
                return Err("XUI21 timeout owner is null".into());
            }
            let due_tick = self
                .memory
                .u32(raw.owner + EVENT_WINDOW_DUE_TICK_OFF)
                .filter(|value| *value != 0)
                .ok_or_else(|| "XUI21 timeout due tick is unreadable".to_string())?;
            if resolved_tick < due_tick {
                return Ok(None);
            }
        }
        let event_sequence = match raw.event_sequence_hint {
            Some(sequence) => {
                if raw.instance_id == 0 {
                    return Err("XUI21 hinted resolution has no command instance id".into());
                }
                let bound = self.sequence_for_instance_or_event(
                    raw.instance_id,
                    raw.event,
                    raw.recipient_tag_id,
                )?;
                if bound != sequence {
                    return Err("XUI21 hinted resolution disagrees with instance binding".into());
                }
                sequence
            }
            None => self.sequence_for_instance_or_event(
                raw.instance_id,
                raw.event,
                raw.recipient_tag_id,
            )?,
        };
        let chosen_option_ordinal = if raw.option_ordinal == u32::MAX {
            None
        } else {
            let definition = self.definition(raw.event)?;
            if raw.option_ordinal >= definition.option_count || raw.option_ordinal > u16::MAX as u32
            {
                return Err("XUI21 selected option ordinal is outside the definition".into());
            }
            Some(raw.option_ordinal as u16)
        };
        if raw.path != ResolutionPath::Timeout && chosen_option_ordinal.is_none() {
            return Err("XUI21 non-timeout resolution has no chosen option".into());
        }
        self.retire_resolved_instance(
            raw.instance_id,
            raw.event,
            raw.recipient_tag_id,
            event_sequence,
            raw.path,
            chosen_option_ordinal,
            u64::from(resolved_tick),
        )?;
        Ok(Some(ResolutionInput {
            event_sequence,
            chosen_option_ordinal,
            resolved_tick: u64::from(resolved_tick),
            reason: if raw.path == ResolutionPath::Timeout {
                ResolutionReason::Timeout
            } else {
                ResolutionReason::Selected
            },
        }))
    }

    fn coverage_receipt(&self) -> Result<Option<CoverageReceipt>, String> {
        let state = self
            .state
            .lock()
            .map_err(|_| "XUI21 bridge mutex poisoned".to_string())?;
        Ok(Some(CoverageReceipt {
            schema: "hoi4-xui21-runtime-coverage-receipt-v1".to_string(),
            session_id: self.session_id.clone(),
            program: "hoi4.exe 1.19.2.0.a729".to_string(),
            image_base: "0x140000000".to_string(),
            executable_sha256: "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
                .to_string(),
            producer_manifest_sha256: manifest_sha256_hex(),
            matrix_bits: state.coverage.matrix,
            observations: state.coverage.observations.values().cloned().collect(),
            lifecycle: state.coverage.lifecycle.clone(),
            hook_telemetry: None,
        }))
    }

    fn fired_ledger_shadow_matches(&self) -> Result<bool, String> {
        let state = self
            .state
            .lock()
            .map_err(|_| "XUI21 bridge mutex poisoned".to_string())?;
        if !state.unbound_by_key.is_empty() || !state.pending_before_by_sequence.is_empty() {
            return Err("XUI21 fired-event shadow requested during an in-flight fire".into());
        }
        let actual = self.read_fired_event_set()?;
        Ok(state.fired_shadow_expected == actual)
    }
}

fn remove_unbound(state: &mut BridgeState, key: FireKey, sequence: u64) {
    if let Some(values) = state.unbound_by_key.get_mut(&key) {
        values.retain(|value| *value != sequence);
        if values.is_empty() {
            state.unbound_by_key.remove(&key);
        }
    }
}

fn runtime_popup_subtype<M: A729Memory>(
    memory: &M,
    event: usize,
    class: A729EventClass,
) -> Result<PopupSubtype, String> {
    let operative = memory
        .u8(event + 0x41D)
        .ok_or_else(|| "XUI21 operative chrome flag is unreadable".to_string())?
        != 0;
    let unit = memory
        .u8(event + 0x41E)
        .ok_or_else(|| "XUI21 unit-leader chrome flag is unreadable".to_string())?
        != 0;
    let news = memory
        .u8(event + 0x424)
        .ok_or_else(|| "XUI21 news chrome flag is unreadable".to_string())?
        != 0;
    let runtime = if news {
        PopupSubtype::News
    } else if unit {
        PopupSubtype::UnitLeader
    } else if operative {
        PopupSubtype::OperativeLeader
    } else {
        match class {
            A729EventClass::Country => PopupSubtype::Country,
            A729EventClass::State => PopupSubtype::State,
            A729EventClass::News | A729EventClass::UnitLeader | A729EventClass::OperativeLeader => {
                return Err("XUI21 static event class is missing its runtime chrome flag".into())
            }
        }
    };
    let expected = match class {
        A729EventClass::Country => PopupSubtype::Country,
        A729EventClass::News => PopupSubtype::News,
        A729EventClass::State => PopupSubtype::State,
        A729EventClass::UnitLeader => PopupSubtype::UnitLeader,
        A729EventClass::OperativeLeader => PopupSubtype::OperativeLeader,
    };
    if runtime != expected {
        return Err("XUI21 runtime/static popup subtype mismatch".into());
    }
    Ok(runtime)
}

pub fn decode_datehour(stored: i32) -> Option<String> {
    let min = (1850 * 365 * 24) + DATE_EPOCH_CONST;
    let max = (2200 * 365 * 24) + DATE_EPOCH_CONST;
    if stored < min || stored > max {
        return None;
    }
    let hours = stored - DATE_EPOCH_CONST;
    let hour = hours % 24;
    let days = hours / 24;
    let year = days / 365;
    let mut day_of_year = days - year * 365;
    let mut month = 1;
    for length in [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31] {
        if day_of_year < length {
            break;
        }
        day_of_year -= length;
        month += 1;
    }
    Some(format!("{year}.{month}.{}.{hour}", day_of_year + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FixtureTextJoin;

    impl A729PopupTextOwnerJoin for FixtureTextJoin {
        fn classify(
            &self,
            _event_window: usize,
            _event: usize,
            visible_option_ordinals: &[u16],
            observations: &[ObservedRuntimeTextOwner],
        ) -> Result<PopupTextOwnerBindings, String> {
            Ok(PopupTextOwnerBindings {
                title_owner: Some(observations[0].textbox_owner),
                description_owner: Some(observations[1].textbox_owner),
                option_owners: visible_option_ordinals
                    .iter()
                    .zip(&observations[2..])
                    .map(|(ordinal, observation)| (*ordinal, observation.textbox_owner))
                    .collect(),
            })
        }
    }

    #[derive(Default)]
    struct FixtureMemory(Mutex<BTreeMap<usize, u8>>);

    impl FixtureMemory {
        fn put(&self, address: usize, bytes: &[u8]) {
            let mut memory = self.0.lock().unwrap();
            for (offset, byte) in bytes.iter().enumerate() {
                memory.insert(address + offset, *byte);
            }
        }
        fn u8(&self, address: usize, value: u8) {
            self.put(address, &[value]);
        }
        fn u32(&self, address: usize, value: u32) {
            self.put(address, &value.to_le_bytes());
        }
        fn u64(&self, address: usize, value: u64) {
            self.put(address, &value.to_le_bytes());
        }
        fn msvc_sso(&self, address: usize, value: &str) {
            self.put(address, value.as_bytes());
            self.u64(address + 0x10, value.len() as u64);
            self.u64(address + 0x18, 0x0F);
        }
        fn msvc_heap(&self, address: usize, heap: usize, value: &str) {
            self.put(heap, value.as_bytes());
            self.u64(address, heap as u64);
            self.u64(address + 0x10, value.len() as u64);
            self.u64(address + 0x18, value.len().max(0x10) as u64);
        }
    }

    impl A729Memory for FixtureMemory {
        fn read_exact(&self, address: usize, out: &mut [u8]) -> bool {
            let memory = self.0.lock().unwrap();
            for (offset, byte) in out.iter_mut().enumerate() {
                let Some(value) = memory.get(&(address + offset)) else {
                    return false;
                };
                *byte = *value;
            }
            true
        }
    }

    #[derive(Default)]
    struct FixtureRegistry {
        events: BTreeMap<(String, u32), A729EventDefinition>,
        aliased: Vec<A729AliasedEvent>,
        bare: BTreeMap<u32, A729EventDefinition>,
    }
    impl A729DefinitionRegistry for FixtureRegistry {
        fn get(&self, namespace: &str, numeric_id: u32) -> Option<A729EventDefinition> {
            self.events
                .get(&(namespace.to_ascii_lowercase(), numeric_id))
                .cloned()
        }
        fn aliased(&self) -> Vec<A729AliasedEvent> {
            self.aliased.clone()
        }
        fn bare(&self, numeric_id: u32) -> Option<A729EventDefinition> {
            self.bare.get(&numeric_id).cloned()
        }
    }

    const MODULE: usize = 0x1000_0000;
    const GAME: usize = 0x2000_0000;
    const TAG_DB: usize = 0x2100_0000;
    const TAG_ROWS: usize = 0x2200_0000;
    const BUCKETS: usize = 0x2300_0000;
    const EVENT: usize = 0x2400_0000;
    const OPTIONS: usize = 0x2500_0000;
    const SCOPE: usize = 0x2600_0000;
    const PENDING: usize = 0x2700_0000;
    const WINDOW: usize = 0x2800_0000;
    const ACTOR: usize = 0x2900_0000;
    const RECIPIENT: usize = 0x2900_0010;
    const FIRED_NODE: usize = 0x2A00_0000;
    const NAMESPACE_TABLE: usize = 0x2B00_0000;
    const NAMESPACE_HEAP: usize = 0x2C00_0000;

    const FIXTURE_NAMESPACE: &str = "ss_recruitment_event";
    const FIXTURE_NAMESPACE_SLOT: u32 = 428;
    const FIXTURE_COMBINED_ID: u32 =
        (FIXTURE_NAMESPACE_SLOT + EVENT_ID_FIRST_NAMESPACE_SLOT) * EVENT_ID_NAMESPACE_SPAN + 1;

    const FIXTURE_NEIGHBOUR_NAMESPACE: &str = "GER_inner_circle_himmler";
    const FIXTURE_NEIGHBOUR_SLOT: u32 = 419;
    const NAMESPACE_NEIGHBOUR_HEAP: usize = 0x2C00_1000;

    fn fixture(class: A729EventClass) -> (Arc<FixtureMemory>, Arc<FixtureRegistry>) {
        let (memory, registry) = fixture_parts(class);
        (memory, Arc::new(registry))
    }

    fn fixture_parts(class: A729EventClass) -> (Arc<FixtureMemory>, FixtureRegistry) {
        let memory = Arc::new(FixtureMemory::default());
        memory.u64(MODULE + GAME_STATE_SINGLETON_RVA, GAME as u64);
        memory.u64(MODULE + COUNTRY_TAG_DB_RVA, TAG_DB as u64);
        memory.u64(TAG_DB, TAG_ROWS as u64);
        memory.u32(TAG_DB + COUNTRY_TAG_VECTOR_CAPACITY_OFF, 4);
        memory.u32(TAG_DB + COUNTRY_TAG_VECTOR_SIZE_OFF, 3);
        memory.msvc_sso(TAG_ROWS, "---");
        memory.msvc_sso(TAG_ROWS + 0x20, "GER");
        memory.msvc_sso(TAG_ROWS + 0x40, "ENG");
        memory.u8(GAME + GS_ACTIVE_OFF, 1);
        memory.u32(
            GAME + GS_CURRENT_TICK_OFF,
            DATE_EPOCH_CONST as u32 + 1936 * 365 * 24 + 12,
        );
        memory.u32(GAME + GS_NEXT_EVENT_INSTANCE_OFF, 40);
        memory.u32(GAME + GS_FIRED_EVENT_SET_OFF, 0);
        memory.u32(GAME + GS_FIRED_EVENT_SET_OFF + 4, 4);
        memory.u64(GAME + GS_FIRED_EVENT_SET_OFF + 8, BUCKETS as u64);
        for index in 0..4 {
            memory.u64(BUCKETS + index * 8, 0);
        }
        memory.u64(GAME + GS_PENDING_EVENTS_OFF, PENDING as u64);
        memory.u32(GAME + GS_PENDING_CAPACITY_OFF, 8);
        memory.u32(GAME + GS_PENDING_COUNT_OFF, 0);
        memory.u64(MODULE + EVENT_NAMESPACE_TABLE_RVA, NAMESPACE_TABLE as u64);
        memory.u32(
            MODULE + EVENT_NAMESPACE_COUNT_RVA,
            FIXTURE_NAMESPACE_SLOT + 1,
        );
        for slot in 0..=FIXTURE_NAMESPACE_SLOT {
            memory.msvc_sso(
                NAMESPACE_TABLE + slot as usize * EVENT_NAMESPACE_STRIDE,
                &format!("ns_{slot}"),
            );
        }
        memory.msvc_heap(
            NAMESPACE_TABLE + FIXTURE_NAMESPACE_SLOT as usize * EVENT_NAMESPACE_STRIDE,
            NAMESPACE_HEAP,
            FIXTURE_NAMESPACE,
        );
        memory.msvc_heap(
            NAMESPACE_TABLE + FIXTURE_NEIGHBOUR_SLOT as usize * EVENT_NAMESPACE_STRIDE,
            NAMESPACE_NEIGHBOUR_HEAP,
            FIXTURE_NEIGHBOUR_NAMESPACE,
        );
        memory.u32(EVENT + 0x0C, FIXTURE_COMBINED_ID);
        memory.u64(EVENT + 0x368, OPTIONS as u64);
        memory.u32(EVENT + 0x374, 3);
        memory.u32(EVENT + 0x408, 13);
        memory.u8(EVENT + 0x421, 0);
        memory.u8(
            EVENT + 0x41D,
            u8::from(class == A729EventClass::OperativeLeader),
        );
        memory.u8(EVENT + 0x41E, u8::from(class == A729EventClass::UnitLeader));
        memory.u8(EVENT + 0x424, u8::from(class == A729EventClass::News));
        memory.u64(OPTIONS, 0x3000_0000);
        memory.u64(OPTIONS + 8, 0x3000_0100);
        memory.u64(OPTIONS + 16, 0x3000_0200);
        memory.u32(SCOPE + 8, 1);
        memory.u32(ACTOR, 2);
        memory.u32(RECIPIENT, 1);
        let registry = FixtureRegistry {
            events: BTreeMap::from([(
                (FIXTURE_NAMESPACE.to_ascii_lowercase(), 1),
                A729EventDefinition {
                    event_id: format!("{FIXTURE_NAMESPACE}.1"),
                    class,
                    option_count: 3,
                },
            )]),
            aliased: Vec::new(),
            bare: BTreeMap::new(),
        };
        (memory, registry)
    }

    fn bridge(class: A729EventClass) -> ExactA729CollectorBridge<FixtureMemory, FixtureRegistry> {
        let (memory, registry) = fixture(class);
        ExactA729CollectorBridge::new(
            memory,
            registry,
            MODULE,
            GAME,
            CoverageAttestation::default(),
        )
        .unwrap()
    }

    #[test]
    fn exact_fire_join_uses_dispatch_tick_recipient_and_monotonic_sequence() {
        let bridge = bridge(A729EventClass::Country);
        let row = bridge
            .fire_input(RawFire {
                dispatcher: 0,
                event: EVENT,
                actor_tag: ACTOR as *const u32,
                recipient_tag: RECIPIENT as *const u32,
                event_scope: SCOPE,
                immediate: 0,
            })
            .unwrap()
            .unwrap();
        assert_eq!(row.event_id, "ss_recruitment_event.1");
        assert_eq!(row.recipient_tag, "GER");
        assert_eq!(row.event_sequence, 41);
        assert_eq!(row.effective_timeout_hours, 312);
        assert_eq!(row.timeout_tick - row.fire_tick, 312);
        assert_eq!(row.fire_date, "1936.1.1.12");
    }

    fn bridge_with_extra_tag_rows(
        extra: &[&str],
    ) -> ExactA729CollectorBridge<FixtureMemory, FixtureRegistry> {
        let (memory, registry) = fixture_parts(A729EventClass::Country);
        let live_rows = 3 + extra.len() as u32;
        memory.u32(TAG_DB + COUNTRY_TAG_VECTOR_SIZE_OFF, live_rows);
        memory.u32(TAG_DB + COUNTRY_TAG_VECTOR_CAPACITY_OFF, live_rows + 1);
        for (index, tag) in extra.iter().enumerate() {
            memory.msvc_sso(TAG_ROWS + (3 + index) * 0x20, tag);
        }
        memory.u32(RECIPIENT, 3);
        ExactA729CollectorBridge::new(
            memory,
            Arc::new(registry),
            MODULE,
            GAME,
            CoverageAttestation::default(),
        )
        .unwrap()
    }

    fn fire_recipient_tag(
        bridge: &ExactA729CollectorBridge<FixtureMemory, FixtureRegistry>,
    ) -> Result<String, String> {
        bridge
            .fire_input(RawFire {
                dispatcher: 0,
                event: EVENT,
                actor_tag: ACTOR as *const u32,
                recipient_tag: RECIPIENT as *const u32,
                event_scope: SCOPE,
                immediate: 0,
            })
            .map(|row| row.unwrap().recipient_tag)
    }

    #[test]
    fn the_recipient_join_refuses_an_id_inside_the_capacity_slack() {
        let slack_bridge = |recipient: u32| {
            let (memory, registry) = fixture_parts(A729EventClass::Country);
            memory.msvc_sso(TAG_ROWS + 3 * 0x20, "D04");
            memory.msvc_sso(TAG_ROWS + 4 * 0x20, "SLK");
            memory.u32(TAG_DB + COUNTRY_TAG_VECTOR_SIZE_OFF, 4);
            memory.u32(TAG_DB + COUNTRY_TAG_VECTOR_CAPACITY_OFF, 5);
            memory.u32(RECIPIENT, recipient);
            ExactA729CollectorBridge::new(
                memory,
                Arc::new(registry),
                MODULE,
                GAME,
                CoverageAttestation::default(),
            )
            .unwrap()
        };

        let error = fire_recipient_tag(&slack_bridge(4))
            .expect_err("an id inside the capacity slack is not a published country");
        assert!(
            error.contains("exceeds"),
            "the slack id must be refused against the live size: {error}"
        );
        assert_eq!(
            fire_recipient_tag(&slack_bridge(3)).expect("id 3 is inside the live size"),
            "D04"
        );
    }

    #[test]
    fn the_recipient_join_admits_a_dynamic_country_tag() {
        let bridge = bridge_with_extra_tag_rows(&["D04", "D75"]);
        assert_eq!(
            fire_recipient_tag(&bridge).expect("a dynamic recipient tag must not fail the join"),
            "D04"
        );
    }

    #[test]
    fn the_recipient_join_still_refuses_a_misread_tag() {
        for garbage in ["ge4", "G R", "G_R", "G.R"] {
            let bridge = bridge_with_extra_tag_rows(&[garbage]);
            let error = fire_recipient_tag(&bridge).expect_err(&format!(
                "{garbage:?} is not a shape the engine's tag loader can produce"
            ));
            assert!(
                error.contains("invalid country tag"),
                "{garbage:?} failed for the wrong reason: {error}"
            );
        }
    }

    fn fire_with(edit: impl FnOnce(&FixtureMemory)) -> Result<String, String> {
        fire_with_parts(|memory, _| edit(memory))
    }

    fn fire_with_parts(
        edit: impl FnOnce(&FixtureMemory, &mut FixtureRegistry),
    ) -> Result<String, String> {
        let (memory, mut registry) = fixture_parts(A729EventClass::Country);
        edit(&memory, &mut registry);
        let bridge = ExactA729CollectorBridge::new(
            memory,
            Arc::new(registry),
            MODULE,
            GAME,
            CoverageAttestation::default(),
        )
        .unwrap();
        bridge
            .fire_input(RawFire {
                dispatcher: 0,
                event: EVENT,
                actor_tag: ACTOR as *const u32,
                recipient_tag: RECIPIENT as *const u32,
                event_scope: SCOPE,
                immediate: 0,
            })
            .map(|row| row.unwrap().event_id)
    }

    #[test]
    fn namespace_comes_from_the_engine_table_not_from_the_id() {
        assert_eq!(
            fire_with(|_| ()).unwrap(),
            "ss_recruitment_event.1",
            "unedited fixture must still resolve"
        );
        let error = fire_with(|memory| {
            memory.msvc_heap(
                NAMESPACE_TABLE + FIXTURE_NAMESPACE_SLOT as usize * EVENT_NAMESPACE_STRIDE,
                NAMESPACE_HEAP,
                "some_other_namespace",
            );
        })
        .unwrap_err();
        assert!(
            error.contains("some_other_namespace.1"),
            "expected the engine-read namespace in {error:?}"
        );
    }

    #[test]
    fn namespaced_ids_start_at_slot_ten() {
        assert_eq!(
            fire_with(|memory| memory.u32(EVENT + 0x0C, 43_800_001)).unwrap(),
            "ss_recruitment_event.1"
        );
        let error = fire_with(|memory| memory.u32(EVENT + 0x0C, 42_900_001)).unwrap_err();
        assert!(
            error.contains("GER_inner_circle_himmler.1"),
            "42900001 names slot 419, not ss_recruitment_event: {error:?}"
        );
    }

    /// panics under the release profile's overflow checks).
    #[test]
    fn an_unclaimed_namespaceless_id_is_terminal() {
        let error = fire_with(|memory| memory.u32(EVENT + 0x0C, 999_999)).unwrap_err();
        assert!(
            error.contains("999999") && error.contains("no registered namespace"),
            "unexpected error {error:?}"
        );
    }

    #[test]
    fn a_namespaceless_event_resolves_on_its_literal_id() {
        let joined = fire_with_parts(|memory, registry| {
            memory.u32(EVENT + 0x0C, 999_999);
            registry.bare.insert(
                999_999,
                A729EventDefinition {
                    event_id: "999999".into(),
                    class: A729EventClass::Country,
                    option_count: 1,
                },
            );
        })
        .unwrap();
        assert_eq!(joined, "999999");
    }

    #[test]
    fn a_namespace_count_that_makes_ids_non_invertible_is_terminal() {
        const FIRST_WRAPPING_SLOT: u32 = 42_940;
        const COUNT_ADMITTING_IT: u32 = FIRST_WRAPPING_SLOT + 1;
        let wrapped_id: u32 = FIRST_WRAPPING_SLOT
            .wrapping_add(EVENT_ID_FIRST_NAMESPACE_SLOT)
            .wrapping_mul(EVENT_ID_NAMESPACE_SPAN)
            .wrapping_add(1);
        assert_eq!(wrapped_id, 32_705, "the wrap this test is about");
        assert!(
            wrapped_id < EVENT_ID_BARE_LIMIT,
            "a wrapped base must land in the namespace-less range"
        );

        let seed = |memory: &FixtureMemory, registry: &mut FixtureRegistry| {
            memory.u32(EVENT + 0x0C, wrapped_id);
            registry.bare.insert(
                wrapped_id,
                A729EventDefinition {
                    event_id: "32705".into(),
                    class: A729EventClass::Country,
                    option_count: 1,
                },
            );
        };

        assert_eq!(
            fire_with_parts(|memory, registry| seed(memory, registry)).unwrap(),
            "32705"
        );

        const LAST_INVERTIBLE_COUNT: u32 = FIRST_WRAPPING_SLOT;
        assert_eq!(
            fire_with_parts(|memory, registry| {
                seed(memory, registry);
                memory.u32(MODULE + EVENT_NAMESPACE_COUNT_RVA, LAST_INVERTIBLE_COUNT);
            })
            .unwrap(),
            "32705"
        );

        let error = fire_with_parts(|memory, registry| {
            seed(memory, registry);
            memory.u32(MODULE + EVENT_NAMESPACE_COUNT_RVA, COUNT_ADMITTING_IT);
        })
        .unwrap_err();
        assert!(
            error.contains("namespace count is invalid"),
            "expected a terminal count refusal, got {error:?}"
        );
    }

    #[test]
    fn an_alias_wrapping_onto_a_namespaceless_id_is_terminal() {
        let bare_id: u32 = 999_999;
        let base =
            (FIXTURE_NEIGHBOUR_SLOT + EVENT_ID_FIRST_NAMESPACE_SLOT) * EVENT_ID_NAMESPACE_SPAN;
        let error = fire_with_parts(|memory, registry| {
            memory.u32(EVENT + 0x0C, bare_id);
            registry.bare.insert(
                bare_id,
                A729EventDefinition {
                    event_id: "999999".into(),
                    class: A729EventClass::Country,
                    option_count: 1,
                },
            );
            registry.aliased.push(A729AliasedEvent {
                declaring_namespace: FIXTURE_NEIGHBOUR_NAMESPACE.into(),
                reduced_id: bare_id.wrapping_sub(base),
                definition: A729EventDefinition {
                    event_id: "GER_inner_circle_himmler.wrapped_low".into(),
                    class: A729EventClass::Country,
                    option_count: 1,
                },
            });
        })
        .unwrap_err();
        assert!(
            error.contains("ambiguous")
                && error.contains("999999")
                && error.contains("GER_inner_circle_himmler.wrapped_low"),
            "unexpected error {error:?}"
        );
    }

    #[test]
    fn namespace_slot_beyond_the_engine_count_is_terminal() {
        let error = fire_with(|memory| {
            memory.u32(MODULE + EVENT_NAMESPACE_COUNT_RVA, FIXTURE_NAMESPACE_SLOT);
        })
        .unwrap_err();
        assert!(
            error.contains("names no registered namespace"),
            "unexpected error {error:?}"
        );
    }

    #[test]
    fn unknown_numeric_id_in_a_known_namespace_is_terminal() {
        let error = fire_with(|memory| {
            memory.u32(
                EVENT + 0x0C,
                (FIXTURE_NAMESPACE_SLOT + EVENT_ID_FIRST_NAMESPACE_SLOT) * EVENT_ID_NAMESPACE_SPAN
                    + 7,
            );
        })
        .unwrap_err();
        assert!(
            error.contains("ss_recruitment_event.7"),
            "unexpected error {error:?}"
        );
    }

    #[test]
    fn a_row_whose_script_id_contradicts_its_key_is_terminal() {
        assert_eq!(
            fire_with(|_| ()).unwrap(),
            "ss_recruitment_event.1",
            "unedited fixture must still resolve"
        );
        let error = fire_with_parts(|_memory, registry| {
            let key = (FIXTURE_NAMESPACE.to_ascii_lowercase(), 1);
            let definition = registry.events.get_mut(&key).expect("fixture row");
            definition.event_id = "some_other_namespace.1".to_string();
        })
        .unwrap_err();
        assert!(
            error.contains("ss_recruitment_event.1") && !error.contains("some_other_namespace"),
            "a contradicted row must not be reported as its own claim: {error:?}"
        );
    }

    fn neighbour_alias(event_id: &str) -> A729AliasedEvent {
        let base =
            (FIXTURE_NEIGHBOUR_SLOT + EVENT_ID_FIRST_NAMESPACE_SLOT) * EVENT_ID_NAMESPACE_SPAN;
        A729AliasedEvent {
            declaring_namespace: FIXTURE_NEIGHBOUR_NAMESPACE.into(),
            reduced_id: FIXTURE_COMBINED_ID.wrapping_sub(base),
            definition: A729EventDefinition {
                event_id: event_id.into(),
                class: A729EventClass::Country,
                option_count: 1,
            },
        }
    }

    #[test]
    fn an_id_two_script_events_share_is_terminal() {
        let error = fire_with_parts(|_, registry| {
            registry
                .aliased
                .push(neighbour_alias("GER_inner_circle_himmler.1_bis"));
        })
        .unwrap_err();
        assert!(
            error.contains("ambiguous")
                && error.contains("ss_recruitment_event.1")
                && error.contains("GER_inner_circle_himmler.1_bis"),
            "unexpected error {error:?}"
        );
    }

    #[test]
    fn an_aliased_event_alone_on_its_id_resolves() {
        let joined = fire_with_parts(|_, registry| {
            registry.events.clear();
            registry
                .aliased
                .push(neighbour_alias("GER_inner_circle_himmler.1_bis"));
        })
        .unwrap();
        assert_eq!(joined, "GER_inner_circle_himmler.1_bis");
    }

    #[test]
    fn an_alias_from_a_namespace_the_engine_does_not_have_is_not_a_match() {
        let mut alias = neighbour_alias("somewhere_else.1_bis");
        alias.declaring_namespace = "somewhere_else".into();
        let joined = fire_with_parts(|_, registry| registry.aliased.push(alias)).unwrap();
        assert_eq!(joined, "ss_recruitment_event.1");
    }

    #[test]
    fn a_wrapping_alias_id_is_still_matched() {
        let wrapped_id: u32 = 1_099_999;
        let base =
            (FIXTURE_NEIGHBOUR_SLOT + EVENT_ID_FIRST_NAMESPACE_SLOT) * EVENT_ID_NAMESPACE_SPAN;
        let reduced_id = wrapped_id.wrapping_sub(base);
        assert!(reduced_id > u32::MAX / 2, "the fixture must actually wrap");
        let joined = fire_with_parts(|memory, registry| {
            memory.u32(EVENT + 0x0C, wrapped_id);
            registry.aliased.push(A729AliasedEvent {
                declaring_namespace: FIXTURE_NEIGHBOUR_NAMESPACE.into(),
                reduced_id,
                definition: A729EventDefinition {
                    event_id: "GER_inner_circle_himmler.wrapped".into(),
                    class: A729EventClass::Country,
                    option_count: 1,
                },
            });
        })
        .unwrap();
        assert_eq!(joined, "GER_inner_circle_himmler.wrapped");
    }

    #[test]
    fn pending_baseline_reconstructs_fire_tick_and_instance_sequence() {
        let (memory, registry) = fixture(A729EventClass::Country);
        memory.u32(GAME + GS_PENDING_COUNT_OFF, 1);
        memory.u32(PENDING, 39);
        memory.u64(PENDING + 0x08, EVENT as u64);
        memory.u64(PENDING + 0x10, SCOPE as u64);
        memory.u32(PENDING + 0x18, 2);
        let fire_tick = DATE_EPOCH_CONST as u32 + 1936 * 365 * 24 + 12;
        memory.u32(PENDING + 0x28, fire_tick + 312);
        let bridge = ExactA729CollectorBridge::new(
            memory,
            registry,
            MODULE,
            GAME,
            CoverageAttestation::default(),
        )
        .unwrap();
        let baseline = bridge.pending_baseline().unwrap();
        assert!(baseline.complete);
        assert_eq!(baseline.rows.len(), 1);
        assert_eq!(baseline.rows[0].event_sequence, 39);
        assert_eq!(baseline.rows[0].fire_tick, u64::from(fire_tick));
    }

    #[test]
    fn all_five_popup_classes_require_exact_runtime_chrome() {
        for class in [
            A729EventClass::Country,
            A729EventClass::News,
            A729EventClass::State,
            A729EventClass::UnitLeader,
            A729EventClass::OperativeLeader,
        ] {
            let (memory, registry) = fixture(class);
            memory.u32(WINDOW + 0x60, 7);
            memory.u64(WINDOW + 0x68, EVENT as u64);
            memory.u32(WINDOW + 0x80, 1);
            memory.u32(
                WINDOW + EVENT_WINDOW_DUE_TICK_OFF,
                DATE_EPOCH_CONST as u32 + 1936 * 365 * 24 + 12 + 313,
            );
            let bridge = ExactA729CollectorBridge::new(
                memory,
                registry,
                MODULE,
                GAME,
                CoverageAttestation::default(),
            )
            .unwrap()
            .with_popup_text_owner_join(Arc::new(FixtureTextJoin));
            let popup_subtype = match class {
                A729EventClass::Country => PopupSubtype::Country,
                A729EventClass::News => PopupSubtype::News,
                A729EventClass::State => PopupSubtype::State,
                A729EventClass::UnitLeader => PopupSubtype::UnitLeader,
                A729EventClass::OperativeLeader => PopupSubtype::OperativeLeader,
            };
            let mut state = bridge.state.lock().unwrap();
            state.instance_to_sequence.insert(
                7,
                InstanceBinding {
                    sequence: 70,
                    event: EVENT,
                    recipient_tag_id: 1,
                },
            );
            state.sequence_coverage.insert(
                70,
                SequenceCoverage {
                    event: EVENT,
                    event_id: "ss_recruitment_event.1".into(),
                    recipient_tag: "GER".into(),
                    fire_date: "1936.1.1.12".into(),
                    popup_subtype,
                    fire_hour: 12,
                    fire_tick: u64::from(DATE_EPOCH_CONST as u32 + 1936 * 365 * 24 + 12),
                    materialised: false,
                },
            );
            drop(state);
            let row = bridge
                .materialisation_input(RawMaterialisation {
                    event_window: WINDOW,
                    event: EVENT,
                    admitted_option_pointers: vec![0x3000_0000, 0x3000_0200],
                    selected_desc_key: Some("event.desc".into()),
                    shown_picture_gfx: Some("GFX_event".into()),
                    runtime_text_owners: [
                        (0x4100, "ctx:title", "Title"),
                        (0x4200, "ctx:description", "Description"),
                        (0x4300, "ctx:option:0", "Name"),
                        (0x4400, "ctx:option:2", "Name"),
                    ]
                    .into_iter()
                    .map(
                        |(textbox_owner, context_id, element_id)| ObservedRuntimeTextOwner {
                            textbox_owner,
                            setter_caller_rva: match context_id {
                                "ctx:title" => TITLE_TEXT_SETTER_CALLER_RVA,
                                "ctx:description" => DESCRIPTION_TEXT_SETTER_CALLER_RVA,
                                _ => OPTION_TEXT_SETTER_CALLER_RVA,
                            },
                            context_id: context_id.into(),
                            view_id: "EventWindow".into(),
                            element_id: element_id.into(),
                        },
                    )
                    .collect(),
                })
                .unwrap();
            assert_eq!(row.visible_option_ordinals, vec![0, 2]);
            assert_eq!(row.event_sequence, 70);
            assert_eq!(
                row.runtime_title_text_context_id.as_deref(),
                Some("ctx:title")
            );
            assert_eq!(
                row.runtime_option_text_contexts,
                vec![
                    OptionRuntimeTextContext {
                        definition_ordinal: 0,
                        context_id: "ctx:option:0".into(),
                    },
                    OptionRuntimeTextContext {
                        definition_ordinal: 2,
                        context_id: "ctx:option:2".into(),
                    },
                ]
            );
            assert_eq!(
                row.timeout_tick,
                u64::from(DATE_EPOCH_CONST as u32 + 1936 * 365 * 24 + 12 + 313)
            );
            assert_eq!(row.popup_subtype, popup_subtype);
        }
    }

    #[test]
    fn player_ai_timeout_and_timeout_no_choice_are_correlated_without_guessing() {
        let (memory, registry) = fixture(A729EventClass::Country);
        let current_tick =
            <FixtureMemory as A729Memory>::u32(memory.as_ref(), GAME + GS_CURRENT_TICK_OFF)
                .unwrap();
        memory.u32(WINDOW + EVENT_WINDOW_DUE_TICK_OFF, current_tick);
        let bridge = ExactA729CollectorBridge::new(
            Arc::clone(&memory),
            registry,
            MODULE,
            GAME,
            CoverageAttestation::default(),
        )
        .unwrap();
        for (index, (path, ordinal, reason)) in [
            (ResolutionPath::Player, 2, ResolutionReason::Selected),
            (ResolutionPath::Ai, 1, ResolutionReason::Selected),
            (ResolutionPath::Timeout, 0, ResolutionReason::Timeout),
            (ResolutionPath::Timeout, u32::MAX, ResolutionReason::Timeout),
        ]
        .into_iter()
        .enumerate()
        {
            let instance_id = 9 + index as u32;
            let sequence = 90 + index as u64;
            let mut state = bridge.state.lock().unwrap();
            state.instance_to_sequence.insert(
                instance_id,
                InstanceBinding {
                    sequence,
                    event: EVENT,
                    recipient_tag_id: 1,
                },
            );
            state.sequence_coverage.insert(
                sequence,
                SequenceCoverage {
                    event: EVENT,
                    event_id: "ss_recruitment_event.1".into(),
                    recipient_tag: "GER".into(),
                    fire_date: "1936.1.1.12".into(),
                    popup_subtype: PopupSubtype::Country,
                    fire_hour: 12,
                    fire_tick: u64::from(current_tick),
                    materialised: false,
                },
            );
            drop(state);
            let row = bridge
                .resolution_input(RawResolution {
                    path,
                    owner: WINDOW,
                    event_sequence_hint: (path == ResolutionPath::Ai).then_some(sequence),
                    instance_id,
                    event: EVENT,
                    recipient_tag_id: 1,
                    event_scope: SCOPE,
                    option_ordinal: ordinal,
                })
                .unwrap()
                .unwrap();
            assert_eq!(row.event_sequence, sequence);
            assert_eq!(row.reason, reason);
            assert_eq!(
                row.chosen_option_ordinal,
                (ordinal != u32::MAX).then_some(ordinal as u16)
            );
            assert!(!bridge
                .state
                .lock()
                .unwrap()
                .instance_to_sequence
                .contains_key(&instance_id));
        }
        bridge.state.lock().unwrap().instance_to_sequence.insert(
            19,
            InstanceBinding {
                sequence: 99,
                event: EVENT,
                recipient_tag_id: 1,
            },
        );
        assert!(bridge
            .resolution_input(RawResolution {
                path: ResolutionPath::Player,
                owner: WINDOW,
                event_sequence_hint: None,
                instance_id: 19,
                event: EVENT,
                recipient_tag_id: 2,
                event_scope: SCOPE,
                option_ordinal: 0,
            })
            .is_err());

        memory.u32(WINDOW + EVENT_WINDOW_DUE_TICK_OFF, current_tick + 1);
        bridge.state.lock().unwrap().instance_to_sequence.insert(
            20,
            InstanceBinding {
                sequence: 100,
                event: EVENT,
                recipient_tag_id: 1,
            },
        );
        assert!(bridge
            .resolution_input(RawResolution {
                path: ResolutionPath::Timeout,
                owner: WINDOW,
                event_sequence_hint: None,
                instance_id: 20,
                event: EVENT,
                recipient_tag_id: 1,
                event_scope: SCOPE,
                option_ordinal: u32::MAX,
            })
            .unwrap()
            .is_none());
        assert!(bridge
            .state
            .lock()
            .unwrap()
            .instance_to_sequence
            .contains_key(&20));
    }

    #[test]
    fn incomplete_runtime_matrix_does_not_revoke_static_route_certification() {
        let (memory, registry) = fixture(A729EventClass::Country);
        let bridge = ExactA729CollectorBridge::new(
            memory,
            registry,
            MODULE,
            GAME,
            CoverageAttestation::default(),
        )
        .unwrap();
        assert_eq!(
            crate::a729_runtime::certified_route_attestation().unwrap(),
            CoverageAttestation {
                all_event_class_routes_certified: true,
                player_ai_timeout_no_choice_routes_certified: true,
            }
        );
        let receipt = bridge.coverage_receipt().unwrap().unwrap();
        assert_eq!(receipt.matrix_bits, 0);
        assert!(receipt.observations.is_empty());
        assert!(!receipt.complete());
    }

    #[test]
    fn inactive_and_already_fired_once_calls_are_rejected_without_rows() {
        let (memory, registry) = fixture(A729EventClass::Country);
        memory.u8(GAME + GS_ACTIVE_OFF, 0);
        let bridge = ExactA729CollectorBridge::new(
            memory,
            registry,
            MODULE,
            GAME,
            CoverageAttestation::default(),
        )
        .unwrap();
        assert!(bridge
            .fire_input(RawFire {
                dispatcher: 0,
                event: EVENT,
                actor_tag: ACTOR as *const u32,
                recipient_tag: RECIPIENT as *const u32,
                event_scope: SCOPE,
                immediate: 0,
            })
            .unwrap()
            .is_none());

        let (memory, registry) = fixture(A729EventClass::Country);
        memory.u8(EVENT + 0x421, 1);
        memory.u32(GAME + GS_FIRED_EVENT_SET_OFF, 1);
        memory.u64(BUCKETS, FIRED_NODE as u64);
        memory.u64(FIRED_NODE, EVENT as u64);
        memory.u64(FIRED_NODE + 8, 0);
        let bridge = ExactA729CollectorBridge::new(
            memory,
            registry,
            MODULE,
            GAME,
            CoverageAttestation::default(),
        )
        .unwrap();
        assert!(bridge
            .fire_input(RawFire {
                dispatcher: 0,
                event: EVENT,
                actor_tag: ACTOR as *const u32,
                recipient_tag: RECIPIENT as *const u32,
                event_scope: SCOPE,
                immediate: 0,
            })
            .unwrap()
            .is_none());
    }

    #[test]
    fn pending_instance_join_uses_exact_pre_post_set_difference() {
        let (memory, registry) = fixture(A729EventClass::Country);
        let write_pending = |index: usize, instance_id: u32| {
            let row = PENDING + index * PENDING_STRIDE;
            memory.u32(row, instance_id);
            memory.u64(row + 0x08, EVENT as u64);
            memory.u64(row + 0x10, SCOPE as u64);
            memory.u32(row + 0x18, 2);
            memory.u32(row + 0x28, DATE_EPOCH_CONST as u32 + 1936 * 365 * 24 + 324);
        };
        write_pending(0, 38);
        memory.u32(GAME + GS_PENDING_COUNT_OFF, 1);
        let bridge = ExactA729CollectorBridge::new(
            Arc::clone(&memory),
            registry,
            MODULE,
            GAME,
            CoverageAttestation::default(),
        )
        .unwrap();
        let raw = RawFire {
            dispatcher: 0,
            event: EVENT,
            actor_tag: ACTOR as *const u32,
            recipient_tag: RECIPIENT as *const u32,
            event_scope: SCOPE,
            immediate: 0,
        };
        let sequence = bridge.fire_input(raw).unwrap().unwrap().event_sequence;
        write_pending(1, 41);
        memory.u32(GAME + GS_PENDING_COUNT_OFF, 2);
        bridge.fire_committed(raw, sequence, None).unwrap();
        let state = bridge.state.lock().unwrap();
        assert_eq!(
            state.instance_to_sequence.get(&41),
            Some(&InstanceBinding {
                sequence,
                event: EVENT,
                recipient_tag_id: 1,
            })
        );
        assert_eq!(state.instance_to_sequence.get(&38), None);
    }

    #[test]
    fn fired_shadow_rejects_an_unobserved_engine_superset() {
        let (memory, registry) = fixture(A729EventClass::Country);
        let bridge = ExactA729CollectorBridge::new(
            Arc::clone(&memory),
            registry,
            MODULE,
            GAME,
            CoverageAttestation::default(),
        )
        .unwrap();
        memory.u32(GAME + GS_FIRED_EVENT_SET_OFF, 1);
        memory.u64(BUCKETS, FIRED_NODE as u64);
        memory.u64(FIRED_NODE, EVENT as u64);
        memory.u64(FIRED_NODE + 8, 0);
        assert!(!bridge.fired_ledger_shadow_matches().unwrap());
    }

    fn role_observation(
        owner: usize,
        caller_rva: u32,
        misleading_name: &str,
    ) -> ObservedRuntimeTextOwner {
        ObservedRuntimeTextOwner {
            textbox_owner: owner,
            setter_caller_rva: caller_rva,
            context_id: format!("ctx:{owner:x}"),
            view_id: "deliberately_not_a_role".into(),
            element_id: misleading_name.into(),
        }
    }

    #[test]
    fn executable_owner_classifier_uses_producers_not_gui_role_names() {
        let observations = vec![
            role_observation(0x11, TITLE_TEXT_SETTER_CALLER_RVA, "Option"),
            role_observation(0x21, OPTION_TEXT_SETTER_CALLER_RVA, "Title"),
            role_observation(0x22, OPTION_TEXT_SETTER_CALLER_RVA, "Description"),
            role_observation(0x12, DESCRIPTION_TEXT_SETTER_CALLER_RVA, "Name"),
        ];
        let bindings = ExecutableA729PopupTextOwnerJoin
            .classify(WINDOW, EVENT, &[0, 2], &observations)
            .unwrap();
        assert_eq!(bindings.title_owner, Some(0x11));
        assert_eq!(bindings.description_owner, Some(0x12));
        assert_eq!(bindings.option_owners, vec![(0, 0x21), (2, 0x22)]);
    }

    #[test]
    fn executable_owner_classifier_fails_closed_and_excludes_synthetic_ok() {
        let title = role_observation(0x11, TITLE_TEXT_SETTER_CALLER_RVA, "anything");
        let description = role_observation(0x12, DESCRIPTION_TEXT_SETTER_CALLER_RVA, "anything");
        let synthetic = role_observation(0x21, OPTION_TEXT_SETTER_CALLER_RVA, "anything");
        let bindings = ExecutableA729PopupTextOwnerJoin
            .classify(
                WINDOW,
                EVENT,
                &[],
                &[title.clone(), description.clone(), synthetic],
            )
            .unwrap();
        assert!(bindings.option_owners.is_empty());
        assert!(ExecutableA729PopupTextOwnerJoin
            .classify(WINDOW, EVENT, &[0], &[title, description])
            .is_err());
        let title = role_observation(0x11, TITLE_TEXT_SETTER_CALLER_RVA, "anything");
        let description = role_observation(0x12, DESCRIPTION_TEXT_SETTER_CALLER_RVA, "anything");
        assert!(ExecutableA729PopupTextOwnerJoin
            .classify(WINDOW, EVENT, &[], &[title, description])
            .is_err());
        let duplicate_owner = [
            role_observation(0x31, TITLE_TEXT_SETTER_CALLER_RVA, "anything"),
            role_observation(0x31, DESCRIPTION_TEXT_SETTER_CALLER_RVA, "anything"),
        ];
        assert!(ExecutableA729PopupTextOwnerJoin
            .classify(WINDOW, EVENT, &[], &duplicate_owner)
            .is_err());
    }
}
