#![cfg(windows)]

use core::{
    mem::{size_of, MaybeUninit},
    ptr,
};
use serde::{Deserialize, Serialize};
use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::GetCurrentProcess;

pub const EXECUTABLE_SHA256: &str =
    "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad";
pub const PREFERRED_IMAGE_BASE: usize = 0x140000000;
pub const MISSION_ASSIGNMENT_DISCRIMINATOR: u32 = 3;

pub const LEADER_ASSIGNMENT_OFFSET: usize = 0x1080;
pub const LEADER_MISSION_VALUE_OFFSET: usize = 0x1088;
pub const MISSION_IMPLEMENTATION_POINTER_OFFSET: usize = 0x10;
pub const MISSION_DISCRIMINATOR_OFFSET: usize = 0x18;
pub const IMPLEMENTATION_TARGET_COUNTRY_REFERENCE_OFFSET: usize = 0x10;
pub const IMPLEMENTATION_TARGET_STATE_POINTER_OFFSET: usize = 0x18;
pub const IMPLEMENTATION_BOOST_IDEOLOGY_POINTER_OFFSET: usize = 0x40;
pub const STATE_ID_OFFSET: usize = 0x58;
pub const STATE_VTABLE_RVA: usize = 0x29206c0;
pub const IDEOLOGY_AUTHORED_ID_STRING_OFFSET: usize = 0x18;

pub const ACTION_PRODUCER_JOB_ID: &str =
    "20260721T081524Z-wave6-a30-nonoperation-action-target-pro-e1e2a47d";
pub const ACTION_PRODUCER_MANIFEST_SHA256: &str =
    "b7d38da93555d033bae897991e545b1d1b0ed9bb28cace96a133da2bf41344e8";
pub const MISSION_IMPLEMENTATION_JOB_ID: &str =
    "20260721T082748Z-wave6-a30-operative-mission-producer-imp-e22ff499";
pub const MISSION_IMPLEMENTATION_MANIFEST_SHA256: &str =
    "0f44c560ba1d1761768ee2d95aa519d8e67ed788afdfea5357b597772c323d0d";
pub const ASSIGNMENT_CONSUMER_JOB_ID: &str =
    "20260721T083354Z-wave6-a30-nonoperation-assignment-consum-64bb6bca";
pub const ASSIGNMENT_CONSUMER_MANIFEST_SHA256: &str =
    "fb6a2e920abb641a5fcd0195a70bd6b9ad0806c0e081c5508aa2acc718bcc589";
pub const BOOST_IDEOLOGY_JOB_ID: &str =
    "20260721T083407Z-wave6-a30-boost-ideology-product-depth1-8aed0d4c";
pub const BOOST_IDEOLOGY_MANIFEST_SHA256: &str =
    "8abae8f7075aa4b59d68bb8e20d998c4e61b2b3b2bc05be5d0e80d7309911ab9";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MissionTargetKind {
    Country,
    StateWithinCountry,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperativeMissionProductIdentity {
    pub operative_id: u32,
    pub assignment_discriminator: u32,
    pub mission_discriminator: u32,
    pub authored_mission_localization_key: String,
    pub authored_action_localization_key: String,
    pub target_kind: MissionTargetKind,
    pub target_country_tag_db_id: u32,
    pub target_state_id: Option<u32>,
    pub authored_subject_id: Option<String>,
}

#[derive(Clone, Copy)]
struct MissionSpec {
    key: &'static str,
    target_kind: MissionTargetKind,
    state_required: bool,
    ideology_required: bool,
}

fn mission_spec(discriminator: u32) -> Option<MissionSpec> {
    let (key, target_kind, state_required, ideology_required) = match discriminator {
        1 => (
            "OPERATIVE_MISSION_BUILD_INTEL_NETWORK",
            MissionTargetKind::StateWithinCountry,
            true,
            false,
        ),
        2 => (
            "OPERATIVE_MISSION_QUIET_INTEL_NETWORK",
            MissionTargetKind::StateWithinCountry,
            true,
            false,
        ),
        3 => (
            "OPERATIVE_MISSION_COUNTER_INTELLIGENCE",
            MissionTargetKind::Country,
            false,
            false,
        ),
        4 => (
            "OPERATIVE_MISSION_ROOT_OUT_RESISTANCE",
            MissionTargetKind::StateWithinCountry,
            true,
            false,
        ),
        5 => (
            "OPERATIVE_MISSION_BOOST_IDEOLOGY",
            MissionTargetKind::StateWithinCountry,
            true,
            true,
        ),
        6 => (
            "OPERATIVE_MISSION_CONTROL_TRADE",
            MissionTargetKind::Country,
            false,
            false,
        ),
        7 => (
            "OPERATIVE_MISSION_DIPLOMATIC_PRESSURE",
            MissionTargetKind::Country,
            false,
            false,
        ),
        8 => (
            "OPERATIVE_MISSION_PROPAGANDA",
            MissionTargetKind::StateWithinCountry,
            true,
            false,
        ),
        _ => return None,
    };
    Some(MissionSpec {
        key,
        target_kind,
        state_required,
        ideology_required,
    })
}

pub fn read_mission_product_identity(
    operative_id: u32,
    leader: usize,
) -> Result<Option<OperativeMissionProductIdentity>, String> {
    if operative_id == 0 || leader == 0 {
        return Err(
            "A30 mission identity requires non-zero operative and leader identities".into(),
        );
    }

    let assignment_address = add(leader, LEADER_ASSIGNMENT_OFFSET)?;
    let assignment = read_stable::<u32>(assignment_address)
        .ok_or("A30 mission assignment discriminator was unreadable or torn")?;
    if assignment != MISSION_ASSIGNMENT_DISCRIMINATOR {
        return Ok(None);
    }

    let mission_value = add(leader, LEADER_MISSION_VALUE_OFFSET)?;
    let discriminator_address = add(mission_value, MISSION_DISCRIMINATOR_OFFSET)?;
    let implementation_address = add(mission_value, MISSION_IMPLEMENTATION_POINTER_OFFSET)?;
    let discriminator = read_stable::<u32>(discriminator_address)
        .ok_or("A30 mission discriminator was unreadable or torn")?;
    let spec = mission_spec(discriminator).ok_or_else(|| {
        format!("A30 mission discriminator {discriminator} is not authored by exact-a729")
    })?;
    let implementation = read_stable::<usize>(implementation_address)
        .filter(|pointer| *pointer != 0)
        .ok_or("A30 mission implementation pointer was null, unreadable, or torn")?;

    let target_country_address = add(
        implementation,
        IMPLEMENTATION_TARGET_COUNTRY_REFERENCE_OFFSET,
    )?;
    let target_country_tag_db_id = read_stable::<u32>(target_country_address)
        .filter(|id| *id != 0)
        .ok_or("A30 mission target country database id was zero, unreadable, or torn")?;

    let target_state_pointer_address =
        add(implementation, IMPLEMENTATION_TARGET_STATE_POINTER_OFFSET)?;
    let target_state = if spec.state_required {
        Some(
            read_stable::<usize>(target_state_pointer_address)
                .filter(|pointer| *pointer != 0)
                .ok_or("A30 state-target mission had no stable target-state pointer")?,
        )
    } else {
        None
    };
    let target_state_id = if let Some(state) = target_state {
        validate_state_pointer(state)?;
        let raw_state_id = read_stable::<u32>(add(state, STATE_ID_OFFSET)?)
            .ok_or("A30 mission target state id was unreadable or torn")?;
        if raw_state_id == 0 {
            return Err("A30 mission target state id was zero".into());
        }
        if compact_contract::bound_trips::bound_exceeded_u32(
            raw_state_id,
            compact_contract::build_profile::MAX_STATE_ID,
            compact_contract::bound_trips::BOUND_TRIP_MAP_STATE_ID,
        ) {
            return Err("A30 mission target state id was outside the state id space".into());
        }
        Some(raw_state_id)
    } else {
        None
    };

    let ideology_pointer_address =
        add(implementation, IMPLEMENTATION_BOOST_IDEOLOGY_POINTER_OFFSET)?;
    let ideology = if spec.ideology_required {
        Some(
            read_stable::<usize>(ideology_pointer_address)
                .filter(|pointer| *pointer != 0)
                .ok_or("A30 boost-ideology mission had no stable ideology pointer")?,
        )
    } else {
        None
    };
    let authored_subject_id = if let Some(ideology) = ideology {
        Some(read_msvc_string(add(
            ideology,
            IDEOLOGY_AUTHORED_ID_STRING_OFFSET,
        )?)?)
    } else {
        None
    };

    let state_changed = match (target_state, target_state_id) {
        (Some(state), Some(state_id)) => {
            read_stable::<usize>(target_state_pointer_address) != Some(state)
                || read_stable::<u32>(add(state, STATE_ID_OFFSET)?) != Some(state_id)
        }
        (None, None) => false,
        _ => true,
    };
    let ideology_changed = match (ideology, authored_subject_id.as_deref()) {
        (Some(ideology), Some(subject_id)) => {
            read_stable::<usize>(ideology_pointer_address) != Some(ideology)
                || read_msvc_string(add(ideology, IDEOLOGY_AUTHORED_ID_STRING_OFFSET)?)?
                    != subject_id
        }
        (None, None) => false,
        _ => true,
    };
    if read_stable::<u32>(assignment_address) != Some(assignment)
        || read_stable::<u32>(discriminator_address) != Some(discriminator)
        || read_stable::<usize>(implementation_address) != Some(implementation)
        || read_stable::<u32>(target_country_address) != Some(target_country_tag_db_id)
        || state_changed
        || ideology_changed
    {
        return Err("A30 mission identity changed while it was being observed".into());
    }

    Ok(Some(OperativeMissionProductIdentity {
        operative_id,
        assignment_discriminator: assignment,
        mission_discriminator: discriminator,
        authored_mission_localization_key: spec.key.to_owned(),
        authored_action_localization_key: format!("{}_ACTION", spec.key),
        target_kind: spec.target_kind,
        target_country_tag_db_id,
        target_state_id,
        authored_subject_id,
    }))
}

fn validate_state_pointer(state: usize) -> Result<(), String> {
    let module_base = unsafe { GetModuleHandleW(ptr::null()) } as usize;
    if module_base == 0 {
        return Err("A30 GetModuleHandleW(NULL) returned null".into());
    }
    let vtable = read_stable::<usize>(state)
        .ok_or("A30 mission target state vtable was unreadable or torn")?;
    let expected_vtable = add(module_base, STATE_VTABLE_RVA)?;
    if vtable != expected_vtable {
        return Err(format!(
            "A30 mission target state vtable 0x{vtable:x} did not match exact-a729"
        ));
    }
    Ok(())
}

fn add(address: usize, offset: usize) -> Result<usize, String> {
    address
        .checked_add(offset)
        .ok_or_else(|| "A30 process address overflowed usize".into())
}

fn read_msvc_string(address: usize) -> Result<String, String> {
    let header =
        read_bytes(address, 32).ok_or("A30 authored identity MSVC string header was unreadable")?;
    let length = usize::try_from(u64::from_le_bytes(header[16..24].try_into().unwrap()))
        .map_err(|_| "A30 authored identity string length overflowed usize")?;
    let capacity = usize::try_from(u64::from_le_bytes(header[24..32].try_into().unwrap()))
        .map_err(|_| "A30 authored identity string capacity overflowed usize")?;
    if length == 0 || length > 256 || capacity < length {
        return Err("A30 authored identity string length/capacity was invalid".into());
    }
    let bytes = if capacity < 16 {
        header[..length].to_vec()
    } else {
        let pointer = usize::from_le_bytes(header[..size_of::<usize>()].try_into().unwrap());
        if pointer == 0 {
            return Err("A30 authored identity string heap pointer was null".into());
        }
        read_bytes(pointer, length).ok_or("A30 authored identity string body was unreadable")?
    };
    let value =
        String::from_utf8(bytes).map_err(|_| "A30 authored identity string was not UTF-8")?;
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        return Err("A30 authored identity string contained non-identifier bytes".into());
    }
    Ok(value)
}

fn read_stable<T: Copy + PartialEq>(address: usize) -> Option<T> {
    let first = read_value::<T>(address)?;
    let second = read_value::<T>(address)?;
    (first == second).then_some(first)
}

fn read_value<T: Copy>(address: usize) -> Option<T> {
    let mut value = MaybeUninit::<T>::uninit();
    let mut read = 0usize;
    let ok = unsafe {
        ReadProcessMemory(
            GetCurrentProcess(),
            address as *const _,
            value.as_mut_ptr().cast(),
            size_of::<T>(),
            &mut read,
        )
    };
    (ok != 0 && read == size_of::<T>()).then(|| unsafe { value.assume_init() })
}

fn read_bytes(address: usize, length: usize) -> Option<Vec<u8>> {
    let mut bytes = vec![0u8; length];
    let mut read = 0usize;
    let ok = unsafe {
        ReadProcessMemory(
            GetCurrentProcess(),
            address as *const _,
            bytes.as_mut_ptr().cast(),
            length,
            &mut read,
        )
    };
    (ok != 0 && read == length).then_some(bytes)
}
