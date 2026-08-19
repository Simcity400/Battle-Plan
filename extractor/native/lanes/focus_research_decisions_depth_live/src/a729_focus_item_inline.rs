//! original indirect call.  The joint relay tail-jumps to an ABI-compatible

#![cfg(windows)]

use core::ffi::c_void;
use core::ptr;
use std::sync::Mutex;

type Bool = i32;
type Dword = u32;
type Handle = *mut c_void;

const MEM_COMMIT: Dword = 0x1000;
const MEM_RESERVE: Dword = 0x2000;
const MEM_RELEASE: Dword = 0x8000;
const PAGE_READWRITE: Dword = 0x04;
const PAGE_EXECUTE_READ: Dword = 0x20;
const PAGE_EXECUTE_READWRITE: Dword = 0x40;
const ALLOCATION_GRANULARITY: usize = 0x1_0000;
const MAX_REL32_DISTANCE: usize = 0x7fff_0000;
const RELAY_CAPACITY: usize = 0x1000;

pub const DISPLAY_PERCENT_CALLSITE_RVA: usize = 0x01bb_636e;
pub const JOINT_COUNTRY_CALLSITE_RVA: usize = 0x0137_9e5f;
pub const JOINT_COUNTRY_RENDERER_RVA: usize = 0x00b3_5990;

const DISPLAY_ORIGINAL: [u8; 5] = [0x8b, 0xd6, 0x48, 0x8b, 0xc8];
const JOINT_ORIGINAL: [u8; 5] = [0xe8, 0x2c, 0xbb, 0x7b, 0xff];

#[link(name = "kernel32")]
extern "system" {
    fn VirtualAlloc(
        address: *mut c_void,
        size: usize,
        allocation_type: Dword,
        protect: Dword,
    ) -> *mut c_void;
    fn VirtualFree(address: *mut c_void, size: usize, free_type: Dword) -> Bool;
    fn VirtualProtect(
        address: *mut c_void,
        size: usize,
        new_protect: Dword,
        old_protect: *mut Dword,
    ) -> Bool;
    fn FlushInstructionCache(process: Handle, address: *const c_void, size: usize) -> Bool;
    fn GetCurrentProcess() -> Handle;
}

#[derive(Clone, Copy)]
struct Installed {
    display_site: usize,
    display_relay: usize,
    display_patch: [u8; 5],
    joint_site: usize,
    joint_relay: usize,
    joint_patch: [u8; 5],
}

static INSTALLED: Mutex<Option<Installed>> = Mutex::new(None);

pub fn install(
    image_base: usize,
    display_callback: *const c_void,
    joint_callback: *const c_void,
) -> Result<(), String> {
    if image_base == 0 || display_callback.is_null() || joint_callback.is_null() {
        return Err("A31 inline instrumentation has a null identity".into());
    }
    let mut slot = INSTALLED
        .lock()
        .map_err(|_| "A31 inline install mutex poisoned")?;
    if slot.is_some() {
        return Err("A31 inline instrumentation is already installed".into());
    }
    let display_site = image_base
        .checked_add(DISPLAY_PERCENT_CALLSITE_RVA)
        .ok_or("A31 display callsite address overflowed")?;
    let joint_site = image_base
        .checked_add(JOINT_COUNTRY_CALLSITE_RVA)
        .ok_or("A31 joint callsite address overflowed")?;
    verify_bytes(display_site, &DISPLAY_ORIGINAL, "display-percent")?;
    verify_bytes(joint_site, &JOINT_ORIGINAL, "joint-country")?;

    let display_relay = unsafe { allocate_near(display_site)? };
    let display_body = display_relay_body(display_callback as usize);
    if let Err(error) = unsafe { publish_relay(display_relay, &display_body) } {
        unsafe { VirtualFree(display_relay.cast(), 0, MEM_RELEASE) };
        return Err(error);
    }
    let display_patch = make_rel32_call(display_site, display_relay as usize)
        .ok_or("A31 display relay is outside rel32 range")?;

    let joint_relay = match unsafe { allocate_near(joint_site) } {
        Ok(value) => value,
        Err(error) => {
            unsafe { VirtualFree(display_relay.cast(), 0, MEM_RELEASE) };
            return Err(error);
        }
    };
    let joint_body = absolute_jump_body(joint_callback as usize);
    if let Err(error) = unsafe { publish_relay(joint_relay, &joint_body) } {
        unsafe {
            VirtualFree(joint_relay.cast(), 0, MEM_RELEASE);
            VirtualFree(display_relay.cast(), 0, MEM_RELEASE);
        }
        return Err(error);
    }
    let joint_patch = match make_rel32_call(joint_site, joint_relay as usize) {
        Some(value) => value,
        None => {
            unsafe {
                VirtualFree(joint_relay.cast(), 0, MEM_RELEASE);
                VirtualFree(display_relay.cast(), 0, MEM_RELEASE);
            }
            return Err("A31 joint relay is outside rel32 range".into());
        }
    };

    let installed = Installed {
        display_site,
        display_relay: display_relay as usize,
        display_patch,
        joint_site,
        joint_relay: joint_relay as usize,
        joint_patch,
    };
    if let Err(error) = unsafe { write_code(joint_site, &JOINT_ORIGINAL, &joint_patch) } {
        *slot = Some(installed);
        drop(slot);
        return Err(cleanup_failed_install(error));
    }
    if let Err(error) = unsafe { write_code(display_site, &DISPLAY_ORIGINAL, &display_patch) } {
        *slot = Some(installed);
        drop(slot);
        return Err(cleanup_failed_install(error));
    }
    *slot = Some(installed);
    Ok(())
}

/// for the shared callback counter to quiesce before calling `release_relays`.
pub fn restore_sites() -> Result<(), String> {
    let slot = INSTALLED
        .lock()
        .map_err(|_| "A31 inline install mutex poisoned")?;
    let Some(installed) = *slot else {
        return Ok(());
    };
    let display = unsafe {
        restore_site(
            installed.display_site,
            &installed.display_patch,
            &DISPLAY_ORIGINAL,
            "display-percent",
        )
    };
    let joint = unsafe {
        restore_site(
            installed.joint_site,
            &installed.joint_patch,
            &JOINT_ORIGINAL,
            "joint-country",
        )
    };
    match (display, joint) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(display), Ok(())) => Err(display),
        (Ok(()), Err(joint)) => Err(joint),
        (Err(display), Err(joint)) => Err(format!("{display}; {joint}")),
    }
}

pub fn release_relays() -> Result<(), String> {
    let mut slot = INSTALLED
        .lock()
        .map_err(|_| "A31 inline install mutex poisoned")?;
    let Some(installed) = *slot else {
        return Ok(());
    };
    verify_bytes(
        installed.display_site,
        &DISPLAY_ORIGINAL,
        "restored display-percent",
    )?;
    verify_bytes(
        installed.joint_site,
        &JOINT_ORIGINAL,
        "restored joint-country",
    )?;
    *slot = None;
    let display_ok =
        unsafe { VirtualFree(installed.display_relay as *mut c_void, 0, MEM_RELEASE) } != 0;
    let joint_ok =
        unsafe { VirtualFree(installed.joint_relay as *mut c_void, 0, MEM_RELEASE) } != 0;
    if display_ok && joint_ok {
        Ok(())
    } else {
        Err("A31 inline relay release failed".into())
    }
}

fn cleanup_failed_install(error: String) -> String {
    match restore_sites().and_then(|_| release_relays()) {
        Ok(()) => error,
        Err(cleanup) => format!("{error}; A31 partial-install cleanup failed: {cleanup}"),
    }
}

unsafe fn restore_site(
    address: usize,
    patch: &[u8],
    original: &[u8],
    label: &str,
) -> Result<(), String> {
    let actual = unsafe { std::slice::from_raw_parts(address as *const u8, original.len()) };
    if actual == original {
        return Ok(());
    }
    if actual != patch {
        return Err(format!(
            "A31 exact-a729 {label} bytes are neither installed nor original at {address:#x}"
        ));
    }
    unsafe { write_code(address, patch, original) }
}

fn verify_bytes(address: usize, expected: &[u8], label: &str) -> Result<(), String> {
    let actual = unsafe { std::slice::from_raw_parts(address as *const u8, expected.len()) };
    if actual == expected {
        Ok(())
    } else {
        Err(format!(
            "A31 exact-a729 {label} bytes differ at {address:#x}"
        ))
    }
}

unsafe fn allocate_near(callsite: usize) -> Result<*mut u8, String> {
    let aligned = callsite & !(ALLOCATION_GRANULARITY - 1);
    let mut distance = 0usize;
    while distance <= MAX_REL32_DISTANCE {
        for candidate in [aligned.checked_add(distance), aligned.checked_sub(distance)]
            .into_iter()
            .flatten()
        {
            if make_rel32_call(callsite, candidate).is_none() {
                continue;
            }
            let relay = unsafe {
                VirtualAlloc(
                    candidate as *mut c_void,
                    RELAY_CAPACITY,
                    MEM_RESERVE | MEM_COMMIT,
                    PAGE_READWRITE,
                ) as *mut u8
            };
            if !relay.is_null() {
                return Ok(relay);
            }
        }
        distance = distance
            .checked_add(ALLOCATION_GRANULARITY)
            .ok_or("A31 near-relay search overflowed")?;
    }
    Err("A31 could not allocate a near relay".into())
}

unsafe fn publish_relay(relay: *mut u8, body: &[u8]) -> Result<(), String> {
    if body.len() > RELAY_CAPACITY {
        return Err("A31 relay body exceeds its allocation".into());
    }
    unsafe { ptr::copy_nonoverlapping(body.as_ptr(), relay, body.len()) };
    let mut old = 0;
    if unsafe { VirtualProtect(relay.cast(), RELAY_CAPACITY, PAGE_EXECUTE_READ, &mut old) } == 0 {
        return Err("A31 relay protection change failed".into());
    }
    if unsafe { FlushInstructionCache(GetCurrentProcess(), relay.cast(), body.len()) } == 0 {
        return Err("A31 relay instruction-cache flush failed".into());
    }
    Ok(())
}

unsafe fn write_code(address: usize, expected: &[u8], replacement: &[u8]) -> Result<(), String> {
    if expected.len() != replacement.len() {
        return Err("A31 inline replacement length differs".into());
    }
    verify_bytes(address, expected, "inline pre-write")?;

    let suspended = inline_detour::freeze_process()
        .map_err(|refusal| format!("A31 inline patch refused: {refusal}"))?;

    // release it -- a permanent hang of the player's game, not a slow path.
    let mut freeze_refusal = None;
    let mut refusal: Option<&'static str> = None;
    match suspended.any_instruction_pointer_in(&[(address, address + replacement.len())]) {
        Err(refused) => freeze_refusal = Some(refused),
        Ok(true) => refusal = Some("a thread is executing inside the site"),
        Ok(false) => {
            let mut old = 0;
            if unsafe {
                VirtualProtect(
                    address as *mut c_void,
                    replacement.len(),
                    PAGE_EXECUTE_READWRITE,
                    &mut old,
                )
            } == 0
            {
                refusal = Some("protection change failed");
            } else {
                unsafe {
                    ptr::copy_nonoverlapping(
                        replacement.as_ptr(),
                        address as *mut u8,
                        replacement.len(),
                    )
                };
                let mut ignored = 0;
                if unsafe {
                    VirtualProtect(address as *mut c_void, replacement.len(), old, &mut ignored)
                } == 0
                {
                    refusal = Some("protection restore failed");
                } else if unsafe {
                    FlushInstructionCache(
                        GetCurrentProcess(),
                        address as *const c_void,
                        replacement.len(),
                    )
                } == 0
                {
                    refusal = Some("instruction-cache flush failed");
                }
            }
        }
    }
    drop(suspended);

    if let Some(refused) = freeze_refusal {
        return Err(format!("A31 inline patch refused: {refused}"));
    }
    if let Some(reason) = refusal {
        return Err(format!(
            "A31 inline patch refused: {reason} at {address:#x}..{:#x}",
            address + replacement.len()
        ));
    }
    Ok(())
}

fn make_rel32_call(callsite: usize, destination: usize) -> Option<[u8; 5]> {
    let displacement = (destination as i128) - (callsite.checked_add(5)? as i128);
    let displacement = i32::try_from(displacement).ok()?;
    let mut bytes = [0u8; 5];
    bytes[0] = 0xe8;
    bytes[1..].copy_from_slice(&displacement.to_le_bytes());
    Some(bytes)
}

fn absolute_jump_body(destination: usize) -> Vec<u8> {
    let mut out = vec![0xff, 0x25, 0, 0, 0, 0];
    out.extend_from_slice(&(destination as u64).to_le_bytes());
    out
}

fn display_relay_body(callback: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(128);
    out.extend_from_slice(&[0x48, 0x83, 0xec, 0x78]);
    out.extend_from_slice(&[0x48, 0x89, 0x44, 0x24, 0x20]);
    out.extend_from_slice(&[0x48, 0x89, 0x4c, 0x24, 0x28]);
    out.extend_from_slice(&[0x48, 0x89, 0x54, 0x24, 0x30]);
    out.extend_from_slice(&[0x4c, 0x89, 0x44, 0x24, 0x38]);
    out.extend_from_slice(&[0x4c, 0x89, 0x4c, 0x24, 0x40]);
    out.extend_from_slice(&[0x4c, 0x89, 0x54, 0x24, 0x48]);
    out.extend_from_slice(&[0x4c, 0x89, 0x5c, 0x24, 0x50]);
    out.extend_from_slice(&[0x49, 0x8b, 0x8e, 0x50, 0x0f, 0, 0]);
    out.extend_from_slice(&[0x8b, 0xd6]);
    out.extend_from_slice(&[0x48, 0xb8]);
    out.extend_from_slice(&(callback as u64).to_le_bytes());
    out.extend_from_slice(&[0xff, 0xd0]);
    out.extend_from_slice(&[0x4c, 0x8b, 0x5c, 0x24, 0x50]);
    out.extend_from_slice(&[0x4c, 0x8b, 0x54, 0x24, 0x48]);
    out.extend_from_slice(&[0x4c, 0x8b, 0x4c, 0x24, 0x40]);
    out.extend_from_slice(&[0x4c, 0x8b, 0x44, 0x24, 0x38]);
    out.extend_from_slice(&[0x48, 0x8b, 0x54, 0x24, 0x30]);
    out.extend_from_slice(&[0x48, 0x8b, 0x4c, 0x24, 0x28]);
    out.extend_from_slice(&[0x48, 0x8b, 0x44, 0x24, 0x20]);
    out.extend_from_slice(&[0x48, 0x83, 0xc4, 0x78]);
    out.extend_from_slice(&[0x8b, 0xd6, 0x48, 0x8b, 0xc8, 0xc3]);
    out
}

#[cfg(test)]
mod suspension_tests {
    use super::*;

    struct ScratchCode {
        base: *mut u8,
    }

    impl ScratchCode {
        fn seeded_with(bytes: &[u8]) -> Self {
            let base = unsafe {
                VirtualAlloc(
                    ptr::null_mut(),
                    RELAY_CAPACITY,
                    MEM_RESERVE | MEM_COMMIT,
                    PAGE_EXECUTE_READWRITE,
                ) as *mut u8
            };
            assert!(!base.is_null(), "the scratch code page must allocate");
            unsafe { ptr::copy_nonoverlapping(bytes.as_ptr(), base, bytes.len()) };
            Self { base }
        }

        fn address(&self) -> usize {
            self.base as usize
        }

        fn read(&self, len: usize) -> Vec<u8> {
            unsafe { std::slice::from_raw_parts(self.base, len) }.to_vec()
        }
    }

    impl Drop for ScratchCode {
        fn drop(&mut self) {
            unsafe { VirtualFree(self.base.cast(), 0, MEM_RELEASE) };
        }
    }

    #[test]
    fn rewriting_code_takes_the_shared_process_freeze() {
        let scratch = ScratchCode::seeded_with(&DISPLAY_ORIGINAL);
        let patch = make_rel32_call(scratch.address(), scratch.address() + 0x40)
            .expect("a relay 0x40 bytes along is inside rel32 range");

        let before = inline_detour::freeze_attempts_on_this_thread();
        unsafe { write_code(scratch.address(), &DISPLAY_ORIGINAL, &patch) }
            .expect("a scratch page with the expected bytes must be patchable");
        assert_eq!(
            inline_detour::freeze_attempts_on_this_thread(),
            before + 1,
            "write_code must freeze the process exactly once before rewriting bytes"
        );
        assert_eq!(scratch.read(patch.len()), patch.to_vec());

        let before = inline_detour::freeze_attempts_on_this_thread();
        unsafe { write_code(scratch.address(), &patch, &DISPLAY_ORIGINAL) }
            .expect("the restore must be patchable too");
        assert_eq!(
            inline_detour::freeze_attempts_on_this_thread(),
            before + 1,
            "restoring the original bytes must freeze the process too"
        );
        assert_eq!(
            scratch.read(DISPLAY_ORIGINAL.len()),
            DISPLAY_ORIGINAL.to_vec()
        );
    }

    #[test]
    fn a_byte_mismatch_refuses_before_the_process_is_frozen() {
        let scratch = ScratchCode::seeded_with(&JOINT_ORIGINAL);
        let before = inline_detour::freeze_attempts_on_this_thread();
        let refusal = unsafe { write_code(scratch.address(), &DISPLAY_ORIGINAL, &JOINT_ORIGINAL) }
            .expect_err("the seeded bytes are not the expected ones");
        assert!(refusal.contains("inline pre-write"), "{refusal}");
        assert_eq!(
            inline_detour::freeze_attempts_on_this_thread(),
            before,
            "a byte-gate refusal must not stop the world on the way out"
        );
    }
}
