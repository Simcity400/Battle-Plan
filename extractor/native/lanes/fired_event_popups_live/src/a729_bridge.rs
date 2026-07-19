//! Address-authenticated collector bridge for the a729 event callback graph.
//!
//! The bridge deliberately depends on abstract memory and definition readers.
//! The collector owns the concrete in-process reader and the authenticated
//! game-file registry; deterministic fixtures exercise every join here without
//! manufacturing process pointers.

use crate::a729_runtime::{
    A729EventBridge, CoverageAttestation, PendingBaseline, RawFire, RawMaterialisation,
    RawResolution, ResolutionPath,
};
use crate::{
    FireInput, MaterialisationInput, PopupSubtype, ResolutionInput, ResolutionReason,
    PRODUCER_MANIFEST_SHA256,
};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

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
const MAX_PENDING_EVENTS: u32 = 1_000_000;
const MAX_FIRED_BUCKETS: u32 = 1_000_000;
const MAX_COUNTRY_TAGS: u32 = 100_000;

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
    pub combined_id: u32,
    pub event_id: String,
    pub class: A729EventClass,
    pub option_count: u32,
}

pub trait A729DefinitionRegistry: Send + Sync + 'static {
    fn get(&self, combined_id: u32) -> Option<A729EventDefinition>;
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct FireKey {
    event: usize,
    actor_tag_id: u32,
    recipient_tag_id: u32,
}

#[derive(Default)]
struct BridgeState {
    instance_to_sequence: BTreeMap<u32, u64>,
    unbound_by_key: BTreeMap<FireKey, Vec<u64>>,
    fired_shadow_expected: BTreeSet<usize>,
}

pub struct ExactA729CollectorBridge<M: A729Memory, R: A729DefinitionRegistry> {
    memory: Arc<M>,
    registry: Arc<R>,
    module_base: usize,
    game_state: usize,
    next_sequence: AtomicU64,
    coverage: CoverageAttestation,
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
        let next = memory
            .u32(game_state + GS_NEXT_EVENT_INSTANCE_OFF)
            .ok_or_else(|| "XUI21 event sequence root is unreadable".to_string())?;
        let bridge = Self {
            memory,
            registry,
            module_base,
            game_state,
            next_sequence: AtomicU64::new(u64::from(next)),
            coverage,
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

    fn definition(&self, event: usize) -> Result<A729EventDefinition, String> {
        let combined_id = self
            .memory
            .u32(
                event
                    .checked_add(0x0C)
                    .ok_or("XUI21 event address overflow")?,
            )
            .ok_or_else(|| "XUI21 CEvent combined id is unreadable".to_string())?;
        self.registry
            .get(combined_id)
            .filter(|definition| definition.combined_id == combined_id)
            .ok_or_else(|| format!("XUI21 unknown combined event id {combined_id}"))
    }

    fn tag_string(&self, tag_id: u32) -> Result<String, String> {
        if tag_id == 0 || tag_id >= MAX_COUNTRY_TAGS {
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
        let count = self
            .memory
            .u32(db + 0x08)
            .filter(|count| *count <= MAX_COUNTRY_TAGS)
            .ok_or_else(|| "XUI21 country-tag count is invalid".to_string())?;
        if tag_id >= count {
            return Err(format!("XUI21 recipient tag id {tag_id} exceeds {count}"));
        }
        let address = rows
            .checked_add(tag_id as usize * 0x20)
            .ok_or_else(|| "XUI21 country-tag row address overflow".to_string())?;
        let value = self.read_msvc_string(address, 3)?;
        if value.len() != 3
            || !value
                .bytes()
                .all(|byte| byte == b'-' || byte.is_ascii_uppercase())
        {
            return Err(format!("XUI21 invalid country tag {value:?}"));
        }
        Ok(value)
    }

    fn read_msvc_string(&self, address: usize, max_len: usize) -> Result<String, String> {
        let length = self
            .memory
            .usize(address + 0x10)
            .filter(|length| *length <= max_len)
            .ok_or_else(|| "XUI21 MSVC string length is invalid".to_string())?;
        let capacity = self
            .memory
            .usize(address + 0x18)
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
        Ok(FireInput {
            event_id: definition.event_id,
            recipient_tag: self.tag_string(recipient_tag_id)?,
            fire_date: decode_datehour(fire_tick as i32)
                .ok_or_else(|| "XUI21 fire date is outside the authenticated range".to_string())?,
            fire_hour: fire_hour as u64,
            fire_tick: u64::from(fire_tick),
            event_sequence,
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
            rows.push(PendingRow {
                instance_id,
                event,
                actor_tag_id,
                recipient_tag_id,
                due_tick,
            });
        }
        if self.memory.usize(self.game_state + GS_PENDING_EVENTS_OFF) != Some(base)
            || self.memory.u32(self.game_state + GS_PENDING_COUNT_OFF) != Some(count)
        {
            return Err("XUI21 pending-event vector changed during enumeration".into());
        }
        Ok(rows)
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
        let matches: Vec<PendingRow> = self
            .pending_rows()?
            .into_iter()
            .filter(|row| {
                row.event == key.event
                    && row.actor_tag_id == key.actor_tag_id
                    && row.recipient_tag_id == key.recipient_tag_id
            })
            .collect();
        if matches.is_empty() {
            // Immediate/no-option events have no persistent pending row.
            let mut state = self
                .state
                .lock()
                .map_err(|_| "XUI21 bridge mutex poisoned".to_string())?;
            remove_unbound(&mut state, key, sequence);
            return Ok(());
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| "XUI21 bridge mutex poisoned".to_string())?;
        let chosen = matches
            .iter()
            .filter(|row| !state.instance_to_sequence.contains_key(&row.instance_id))
            .max_by_key(|row| row.instance_id)
            .or_else(|| (matches.len() == 1).then_some(&matches[0]))
            .ok_or_else(|| "XUI21 pending instance correlation is ambiguous".to_string())?;
        state
            .instance_to_sequence
            .insert(chosen.instance_id, sequence);
        remove_unbound(&mut state, key, sequence);
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
        if let Some(sequence) = state.instance_to_sequence.get(&instance_id) {
            return Ok(*sequence);
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
            state.instance_to_sequence.insert(instance_id, sequence);
        }
        remove_unbound(&mut state, key, sequence);
        Ok(sequence)
    }
}

#[derive(Clone, Copy)]
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
                .insert(pending.instance_id, sequence)
                .is_some()
            {
                return Err("XUI21 baseline contains duplicate instance ids".into());
            }
            max_sequence = max_sequence.max(sequence);
            let mut row =
                self.fire_row(pending.event, pending.recipient_tag_id, fire_tick, sequence)?;
            row.event_id = definition.event_id;
            rows.push(row);
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
        self.state
            .lock()
            .map_err(|_| "XUI21 bridge mutex poisoned".to_string())?
            .unbound_by_key
            .entry(key)
            .or_default()
            .push(sequence);
        Ok(Some(row))
    }

    fn fire_committed(
        &self,
        raw: RawFire,
        event_sequence: u64,
        command_instance_id: Option<u32>,
    ) -> Result<(), String> {
        let actor_tag_id = self
            .memory
            .u32(raw.actor_tag as usize)
            .ok_or_else(|| "XUI21 committed actor tag is unreadable".to_string())?;
        let recipient_tag_id = self
            .memory
            .u32(raw.recipient_tag as usize)
            .ok_or_else(|| "XUI21 committed recipient tag is unreadable".to_string())?;
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
            state
                .instance_to_sequence
                .insert(instance_id, event_sequence);
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
        for pointer in raw.admitted_option_pointers {
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
        Ok(MaterialisationInput {
            event_sequence,
            visible_option_ordinals,
            selected_desc_key: raw.selected_desc_key,
            shown_picture_gfx: raw.shown_picture_gfx,
            popup_subtype,
            runtime_text_context_id: None,
        })
    }

    fn resolution_input(&self, raw: RawResolution) -> Result<ResolutionInput, String> {
        let event_sequence = match raw.event_sequence_hint {
            Some(sequence) => sequence,
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
        Ok(ResolutionInput {
            event_sequence,
            chosen_option_ordinal,
            resolved_tick: u64::from(self.current_tick()?),
            reason: if raw.path == ResolutionPath::Timeout {
                ResolutionReason::Timeout
            } else {
                ResolutionReason::Selected
            },
        })
    }

    fn coverage_attestation(&self) -> Result<CoverageAttestation, String> {
        if !self.coverage.all_event_classes || !self.coverage.player_ai_timeout_no_choice {
            return Err("XUI21 compiled coverage attestation is incomplete".into());
        }
        Ok(self.coverage)
    }

    fn fired_ledger_shadow_matches(&self) -> Result<bool, String> {
        let actual = self.read_fired_event_set()?;
        let state = self
            .state
            .lock()
            .map_err(|_| "XUI21 bridge mutex poisoned".to_string())?;
        Ok(state.fired_shadow_expected.is_subset(&actual))
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

    struct FixtureRegistry(BTreeMap<u32, A729EventDefinition>);
    impl A729DefinitionRegistry for FixtureRegistry {
        fn get(&self, combined_id: u32) -> Option<A729EventDefinition> {
            self.0.get(&combined_id).cloned()
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

    fn fixture(class: A729EventClass) -> (Arc<FixtureMemory>, Arc<FixtureRegistry>) {
        let memory = Arc::new(FixtureMemory::default());
        memory.u64(MODULE + GAME_STATE_SINGLETON_RVA, GAME as u64);
        memory.u64(MODULE + COUNTRY_TAG_DB_RVA, TAG_DB as u64);
        memory.u64(TAG_DB, TAG_ROWS as u64);
        memory.u32(TAG_DB + 8, 3);
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
        memory.u32(EVENT + 0x0C, 42_900_001);
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
        let registry = Arc::new(FixtureRegistry(BTreeMap::from([(
            42_900_001,
            A729EventDefinition {
                combined_id: 42_900_001,
                event_id: "ss_recruitment_event.1".into(),
                class,
                option_count: 3,
            },
        )])));
        (memory, registry)
    }

    fn bridge(class: A729EventClass) -> ExactA729CollectorBridge<FixtureMemory, FixtureRegistry> {
        let (memory, registry) = fixture(class);
        ExactA729CollectorBridge::new(
            memory,
            registry,
            MODULE,
            GAME,
            CoverageAttestation {
                all_event_classes: true,
                player_ai_timeout_no_choice: true,
            },
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
            CoverageAttestation {
                all_event_classes: true,
                player_ai_timeout_no_choice: true,
            },
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
            let bridge = ExactA729CollectorBridge::new(
                memory,
                registry,
                MODULE,
                GAME,
                CoverageAttestation {
                    all_event_classes: true,
                    player_ai_timeout_no_choice: true,
                },
            )
            .unwrap();
            bridge
                .state
                .lock()
                .unwrap()
                .instance_to_sequence
                .insert(7, 70);
            let row = bridge
                .materialisation_input(RawMaterialisation {
                    event_window: WINDOW,
                    event: EVENT,
                    admitted_option_pointers: vec![0x3000_0000, 0x3000_0200],
                    selected_desc_key: Some("event.desc".into()),
                    shown_picture_gfx: Some("GFX_event".into()),
                })
                .unwrap();
            assert_eq!(row.visible_option_ordinals, vec![0, 2]);
            assert_eq!(row.event_sequence, 70);
        }
    }

    #[test]
    fn player_ai_timeout_and_timeout_no_choice_are_correlated_without_guessing() {
        let bridge = bridge(A729EventClass::Country);
        bridge
            .state
            .lock()
            .unwrap()
            .instance_to_sequence
            .insert(9, 90);
        for (path, ordinal, reason) in [
            (ResolutionPath::Player, 2, ResolutionReason::Selected),
            (ResolutionPath::Ai, 1, ResolutionReason::Selected),
            (ResolutionPath::Timeout, 0, ResolutionReason::Timeout),
            (ResolutionPath::Timeout, u32::MAX, ResolutionReason::Timeout),
        ] {
            let row = bridge
                .resolution_input(RawResolution {
                    path,
                    owner: WINDOW,
                    event_sequence_hint: (path == ResolutionPath::Ai).then_some(90),
                    instance_id: 9,
                    event: EVENT,
                    recipient_tag_id: 1,
                    event_scope: SCOPE,
                    option_ordinal: ordinal,
                })
                .unwrap();
            assert_eq!(row.event_sequence, 90);
            assert_eq!(row.reason, reason);
            assert_eq!(
                row.chosen_option_ordinal,
                (ordinal != u32::MAX).then_some(ordinal as u16)
            );
        }
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
            CoverageAttestation {
                all_event_classes: true,
                player_ai_timeout_no_choice: true,
            },
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
            CoverageAttestation {
                all_event_classes: true,
                player_ai_timeout_no_choice: true,
            },
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
}
