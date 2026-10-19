//! Sparse retained-state writers and pass anchors. No quiet-member callback.
use super::*;
use super::{hook_failure, verify_hook};
use engine_decisions::decision_runtime as runtime;
use engine_events::script_event_sites::decisions as sites;
use replay_data::event_feed_rows::decisions::{self as rows, Operation};
use std::cell::Cell;
static ORIGINAL: [AtomicUsize; 20] = [const { AtomicUsize::new(0) }; 20];
static CALL: [AtomicUsize; 14] = [const { AtomicUsize::new(0) }; 14];
std::thread_local! {static REENABLE:Cell<usize>=const {Cell::new(0)};static REACTIVATION:Cell<(usize,i32)>=const {Cell::new((0,0))};}
fn count(status: usize, container: u8) -> Result<u32, &'static str> {
    let memory = unsafe { engine_native::memory::LiveMemory::new() };
    let offset = *runtime::HEADERS
        .get(container as usize)
        .ok_or("decision pass container differs")?;
    let count = field::<i32>(&memory, status, offset + engine_native::cpdx_array::COUNT)?;
    u32::try_from(count).map_err(|_| "decision pass count is negative")
}
fn begin(status: usize, container: u8) -> bool {
    match count(status, container) {
        Ok(0) => false,
        Ok(n) => {
            feed::decisions::marker(status, Operation::PassBegin, container, n, 0);
            true
        }
        Err(reason) => {
            feed::refuse(reason);
            false
        }
    }
}
fn end(status: usize, container: u8) {
    feed::decisions::marker(status, Operation::PassEnd, container, 0, 0);
}
fn close_reenable(status: usize) {
    if REENABLE.get() == status {
        end(status, rows::REENABLE);
        REENABLE.set(0);
    }
}
struct Anchor {
    status: usize,
    container: u8,
    opened: bool,
    completed: bool,
}
impl Drop for Anchor {
    fn drop(&mut self) {
        if self.opened && !self.completed {
            feed::refuse("decision pass did not return to close its anchor");
        }
    }
}
unsafe fn pass(status: usize, arg: usize, index: usize, container: u8) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::EventFeedProvenance);
    let mut flight = Flight::enter();
    let memory = unsafe { engine_native::memory::LiveMemory::new() };
    let admitted = if container != rows::REMOVAL {
        true
    } else {
        match field::<u8>(&memory, status, runtime::REMOVAL_PASS_ACTIVE) {
            Ok(value) => value == 0,
            Err(reason) => {
                feed::refuse(reason);
                false
            }
        }
    };
    let mut anchor = Anchor {
        status,
        container,
        opened: admitted && begin(status, container),
        completed: false,
    };
    type Original = unsafe extern "system-unwind" fn(usize, usize) -> usize;
    let original: Original =
        unsafe { std::mem::transmute(ORIGINAL[index].load(Ordering::Acquire)) };
    let result = timing.original(|| unsafe { original(status, arg) });
    if anchor.opened {
        end(anchor.status, anchor.container);
    }
    anchor.completed = true;
    flight.returned();
    result
}
macro_rules! pass_entry {
    ($name:ident,$index:expr,$container:expr) => {
        unsafe extern "system-unwind" fn $name(status: usize, arg: usize) -> usize {
            unsafe { pass(status, arg, $index, $container) }
        }
    };
}
pass_entry!(timed, 0, rows::TIMED);
pass_entry!(targeted, 1, rows::TARGETED);
pass_entry!(targeted_timed, 2, rows::TARGETED_TIMED);
pass_entry!(removal, 3, rows::REMOVAL);
unsafe extern "system-unwind" fn taken(status: usize, arg: usize) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::EventFeedProvenance);
    let mut flight = Flight::enter();
    close_reenable(status);
    type Original = unsafe extern "system-unwind" fn(usize, usize) -> usize;
    let original: Original = unsafe { std::mem::transmute(ORIGINAL[4].load(Ordering::Acquire)) };
    let result = timing.original(|| unsafe { original(status, arg) });
    flight.returned();
    result
}
fn observe_erase_range(
    memory: &impl Memory,
    header: usize,
    first: usize,
    last: usize,
    mut retire: impl FnMut(usize, bool, bool),
) -> Result<(), &'static str> {
    let array = engine_native::cpdx_array::Array::read(memory, header, 8)
        .ok_or("decision timer erase header unreadable")?;
    let first = u32::try_from(first).map_err(|_| "decision timer erase first differs")?;
    let last = u32::try_from(last).map_err(|_| "decision timer erase last differs")?;
    if first > last || last > array.count {
        return Err("decision timer erase range differs");
    }
    let status = REENABLE.get();
    let iteration = status != 0
        && status.checked_add(runtime::HEADERS[rows::REENABLE as usize]) == Some(header)
        && last == array.count
        && first.checked_add(1) == Some(last);
    // A last-member expiry retains its pointer in the removed tail and has
    // no lifecycle retirement. Other expiries already retired their null tail.
    // Only the native removed range, never the retained prefix/suffix.
    for ordinal in first..last {
        let at = array
            .element(ordinal, 8)
            .ok_or("decision timer erase ordinal differs")?;
        let pointer = field::<u64>(memory, at, 0)? as usize;
        if pointer != 0 {
            retire(pointer, iteration, iteration);
        }
    }
    Ok(())
}
unsafe extern "system-unwind" fn erase_range(header: usize, first: usize, last: usize) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::EventFeedProvenance);
    let mut flight = Flight::enter();
    let memory = unsafe { engine_native::memory::LiveMemory::new() };
    if let Err(reason) = observe_erase_range(&memory, header, first, last, feed::decisions::retire) {
        feed::refuse(reason);
    }
    type Original = unsafe extern "system-unwind" fn(usize, usize, usize) -> usize;
    let original: Original = unsafe { std::mem::transmute(ORIGINAL[5].load(Ordering::Acquire)) };
    let result = timing.original(|| unsafe { original(header, first, last) });
    flight.returned();
    result
}
unsafe fn reset_or_restore(status: usize, b: usize, c: usize, index: usize) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::EventFeedProvenance);
    let mut flight = Flight::enter();
    if index == 7 || b as u8 != 0 {
        feed::decisions::marker(status, Operation::Invalidate, 0, 0, 0);
    } else {
        clear_candidates(status);
    }
    type Original = unsafe extern "system-unwind" fn(usize, usize, usize) -> usize;
    let original: Original =
        unsafe { std::mem::transmute(ORIGINAL[index].load(Ordering::Acquire)) };
    let result = timing.original(|| unsafe { original(status, b, c) });
    flight.returned();
    result
}
unsafe extern "system-unwind" fn reset(a: usize, b: usize, c: usize) -> usize {
    unsafe { reset_or_restore(a, b, c, 6) }
}
unsafe extern "system-unwind" fn restore(a: usize, b: usize, c: usize) -> usize {
    unsafe { reset_or_restore(a, b, c, 7) }
}
unsafe extern "system-unwind" fn random(status: usize, definition: usize, target: usize) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::EventFeedProvenance);
    let mut flight = Flight::enter();
    type Original = unsafe extern "system-unwind" fn(usize, usize, usize) -> usize;
    let original: Original = unsafe { std::mem::transmute(ORIGINAL[8].load(Ordering::Acquire)) };
    let result = timing.original(|| unsafe { original(status, definition, target) });
    feed::decisions::random(status, definition, target as u32);
    flight.returned();
    result
}
fn candidate_owner(memory: &impl Memory, header: usize) -> Result<(usize, u8), &'static str> {
    let base = BASE.load(Ordering::Acquire);
    for (kind, offset) in runtime::CANDIDATE_HEADERS.into_iter().enumerate() {
        if let Some(status) = header.checked_sub(offset)
            && engine_native::read::<u64>(memory, status)
                == base
                    .checked_add(engine_decisions::cdecision_status::VTABLE_RVA)
                    .map(|v| v as u64)
        {
            return Ok((status, kind as u8));
        }
    }
    Err("decision candidate append owner differs")
}
fn appended_candidates(header: usize, before: Result<u32, &'static str>) {
    let memory = unsafe { engine_native::memory::LiveMemory::new() };
    let result = (|| {
        let before = before?;
        let (status, kind) = candidate_owner(&memory, header)?;
        let array =
            engine_native::cpdx_array::Array::read(&memory, header, runtime::CANDIDATE_STRIDE)
                .ok_or("decision candidate result header unreadable")?;
        if before > array.count {
            return Err("decision candidate append shrank its destination");
        }
        for ordinal in before..array.count {
            let at = array
                .element(ordinal, runtime::CANDIDATE_STRIDE)
                .ok_or("decision candidate result ordinal differs")?;
            let definition = field::<u64>(&memory, at, runtime::CANDIDATE_DEFINITION)? as usize;
            let target = field::<u32>(&memory, at, 0)?;
            let root = field::<u32>(&memory, at, runtime::CANDIDATE_ROOT)?;
            feed::decisions::candidate(status, kind, definition, target, root);
        }
        Ok::<_, &'static str>(())
    })();
    if let Err(reason) = result {
        feed::refuse(reason);
    }
}
fn candidate_count(header: usize) -> Result<u32, &'static str> {
    let memory = unsafe { engine_native::memory::LiveMemory::new() };
    engine_native::cpdx_array::Array::read(&memory, header, runtime::CANDIDATE_STRIDE)
        .map(|a| a.count)
        .ok_or("decision candidate append header unreadable")
}
unsafe extern "system-unwind" fn country_candidate(
    header: usize,
    root: usize,
    definition: usize,
    target: usize,
) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::EventFeedProvenance);
    let mut flight = Flight::enter();
    let before = candidate_count(header);
    type Original = unsafe extern "system-unwind" fn(usize, usize, usize, usize) -> usize;
    let original: Original = unsafe { std::mem::transmute(ORIGINAL[9].load(Ordering::Acquire)) };
    let result = timing.original(|| unsafe { original(header, root, definition, target) });
    appended_candidates(header, before);
    flight.returned();
    result
}
unsafe extern "system-unwind" fn state_candidates(
    status: usize,
    definition: usize,
    global: usize,
    explicit: usize,
) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::EventFeedProvenance);
    let mut flight = Flight::enter();
    let memory = unsafe { engine_native::memory::LiveMemory::new() };
    let selected =
        field::<u8>(&memory, definition, runtime::EXPLICIT_TARGETS).map(|flag| flag != 0);
    let before = if selected == Ok(true) {
        Some(candidate_count(explicit))
    } else {
        None
    };
    if let Err(reason) = selected {
        feed::refuse(reason);
    }
    type Original = unsafe extern "system-unwind" fn(usize, usize, usize, usize) -> usize;
    let original: Original = unsafe { std::mem::transmute(ORIGINAL[10].load(Ordering::Acquire)) };
    let result = timing.original(|| unsafe { original(status, definition, global, explicit) });
    if let Some(before) = before {
        appended_candidates(explicit, before);
    }
    flight.returned();
    result
}
unsafe fn global_candidates(closure: usize, arg: usize, index: usize) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::EventFeedProvenance);
    let mut flight = Flight::enter();
    let memory = unsafe { engine_native::memory::LiveMemory::new() };
    let destination = field::<u64>(&memory, closure, 0)
        .and_then(|inner| field::<u64>(&memory, inner as usize, 0))
        .map(|p| p as usize);
    let before = destination.and_then(candidate_count);
    type Original = unsafe extern "system-unwind" fn(usize, usize) -> usize;
    let original: Original =
        unsafe { std::mem::transmute(ORIGINAL[index].load(Ordering::Acquire)) };
    let result = timing.original(|| unsafe { original(closure, arg) });
    match destination {
        Ok(header) => appended_candidates(header, before),
        Err(reason) => feed::refuse(reason),
    }
    flight.returned();
    result
}
macro_rules! global_entry {
    ($name:ident,$index:expr) => {
        unsafe extern "system-unwind" fn $name(a: usize, b: usize) -> usize {
            unsafe { global_candidates(a, b, $index) }
        }
    };
}
global_entry!(global_states, 11);
global_entry!(owned_states, 12);
global_entry!(controlled_states, 13);
global_entry!(related_states, 14);
unsafe extern "system-unwind" fn filter(status: usize) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::EventFeedProvenance);
    let mut flight = Flight::enter();
    let headers = [
        status + runtime::CANDIDATE_HEADERS[2],
        status + runtime::CANDIDATE_HEADERS[3],
    ];
    let before = headers.map(candidate_count);
    type Original = unsafe extern "system-unwind" fn(usize) -> usize;
    let original: Original = unsafe { std::mem::transmute(ORIGINAL[15].load(Ordering::Acquire)) };
    let result = timing.original(|| unsafe { original(status) });
    for (header, before) in headers.into_iter().zip(before) {
        appended_candidates(header, before);
    }
    flight.returned();
    result
}
fn clear_candidates(status: usize) {
    feed::decisions::marker(
        status,
        Operation::Candidate,
        0,
        0,
        rows::ALL | rows::CLEAR_ALL_CANDIDATES,
    );
}
unsafe fn ignore_token(container: usize, token: usize, index: usize) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::EventFeedProvenance);
    let mut flight = Flight::enter();
    let memory = unsafe { engine_native::memory::LiveMemory::new() };
    let before = field::<u32>(&memory, container, 0x14);
    type Original = unsafe extern "system-unwind" fn(usize, usize) -> usize;
    let original: Original =
        unsafe { std::mem::transmute(ORIGINAL[index].load(Ordering::Acquire)) };
    let result = timing.original(|| unsafe { original(container, token) });
    let after = field::<u32>(&memory, container, 0x14);
    if before == after && before.is_ok() {
        flight.returned();
        return result;
    }
    match container.checked_sub(runtime::IGNORED_TOKENS - 8) {
        Some(country) => feed::decisions::ignored(country, token, index == 16),
        None => feed::refuse("decision ignored-token owner underflow"),
    };
    flight.returned();
    result
}
unsafe extern "system-unwind" fn ignore(a: usize, b: usize) -> usize {
    unsafe { ignore_token(a, b, 16) }
}
unsafe extern "system-unwind" fn unignore(a: usize, b: usize) -> usize {
    unsafe { ignore_token(a, b, 17) }
}
unsafe fn update(status: usize, b: usize, index: usize) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::EventFeedProvenance);
    let mut flight = Flight::enter();
    let previous = REENABLE.get();
    type Original = unsafe extern "system-unwind" fn(usize, usize) -> usize;
    let original: Original =
        unsafe { std::mem::transmute(ORIGINAL[index].load(Ordering::Acquire)) };
    let result = timing.original(|| unsafe { original(status, b) });
    close_reenable(status);
    REENABLE.set(previous);
    flight.returned();
    result
}
unsafe extern "system-unwind" fn hourly(a: usize, b: usize) -> usize {
    unsafe { update(a, b, 18) }
}
unsafe extern "system-unwind" fn refresh(a: usize, b: usize) -> usize {
    unsafe { update(a, b, 19) }
}
unsafe fn call(a: usize, b: usize, c: usize, index: usize) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::EventFeedProvenance);
    let mut flight = Flight::enter();
    let memory = unsafe { engine_native::memory::LiveMemory::new() };
    let selected = if index <= 6 {
        field::<u64>(&memory, b, 0).map(|p| p as usize)
    } else {
        Ok(a)
    };
    type Original = unsafe extern "system-unwind" fn(usize, usize, usize) -> usize;
    let original: Original = unsafe { std::mem::transmute(CALL[index].load(Ordering::Acquire)) };
    let result = timing.original(|| unsafe { original(a, b, c) });
    let observation = (|| {
        match index {
            0..=4 => {
                let container = if index == 0 {
                    rows::TAKEN
                } else if index == 1 {
                    rows::REMOVAL
                } else {
                    rows::REENABLE
                };
                let status = a
                    .checked_sub(runtime::HEADERS[container as usize])
                    .ok_or("decision append status underflow")?;
                feed::decisions::instance(
                    status,
                    selected?,
                    container,
                    Operation::Store,
                    if index == 4 { rows::ITERATION } else { 0 },
                );
            }
            5 | 6 => {
                if result != 0 {
                    feed::decisions::retire(selected?, false, false);
                }
            }
            7 => {
                REACTIVATION.set((a, b as i32));
            }
            8 => {
                feed::decisions::reactivated(a, b as i32, REACTIVATION.replace((0, 0)));
            }
            9 | 10 => {
                feed::decisions::write(a, None, Some(b as i32));
            }
            11 => {
                if result as u8 != 0 {
                    feed::decisions::flags(b, Some(true), false);
                }
            }
            12 | 13 => {
                if result != 0 {
                    feed::decisions::flags(result, None, true);
                }
            }
            _ => return Err("decision selected call identity differs"),
        }
        Ok::<_, &'static str>(())
    })();
    if let Err(reason) = observation {
        feed::refuse(reason);
    }
    flight.returned();
    result
}
macro_rules! call_entry {
    ($name:ident,$index:expr) => {
        unsafe extern "system-unwind" fn $name(a: usize, b: usize, c: usize) -> usize {
            unsafe { call(a, b, c, $index) }
        }
    };
}
call_entry!(append_taken, 0);
call_entry!(append_removal, 1);
call_entry!(transfer, 2);
call_entry!(append_reenable, 3);
call_entry!(expiry_transfer, 4);
call_entry!(clean_targeted, 5);
call_entry!(clean_targeted_timed, 6);
call_entry!(reactivate_days, 7);
call_entry!(reactivate_state, 8);
call_entry!(console_days, 9);
call_entry!(console_all_days, 10);
call_entry!(ignore_selected, 11);
call_entry!(request_timed, 12);
call_entry!(request_targeted, 13);
fn emit_selected(
    pointer: usize,
    _targeted: bool,
    source: EventFeedSource,
    state: Option<i32>,
    days: Option<i32>,
) -> Result<(), &'static str> {
    if !feed::decisions::write_result(pointer, state, days, source) {
        return Err("decision result has no owner baseline/pointer certificate");
    }
    Ok(())
}
unsafe extern "system-unwind" fn tap(r: &Registers, index: usize) {
    let _timing = crate::hook_timing::enter(crate::hook_timing::Family::EventFeedProvenance);
    match index {
        0 | 1 => {
            let status = if index == 0 { r.rbp } else { r.r14 };
            if REENABLE.get() != 0 {
                feed::refuse("decision re-enable anchor overlapped");
            }
            if begin(status, rows::REENABLE) {
                REENABLE.set(status);
            }
        }
        2..=10 => feed::decisions::cut(r.rbx),
        11 => feed::decisions::retire(r.rbx, true, true),
        12 => feed::decisions::retire(r.r14, true, false),
        13 => {
            let memory = unsafe { engine_native::memory::LiveMemory::new() };
            match field::<u64>(&memory, r.r14, 0) {
                Ok(pointer) => feed::decisions::cut(pointer as usize),
                Err(reason) => feed::refuse(reason),
            }
            if r.rax as i32 > 0 {
                feed::decisions::taken_erase(r.rdi, r.rbx, None, true);
            }
        }
        14 => feed::decisions::retire(r.r8, true, true),
        15 | 16 => {
            let targeted = index == 16;
            let pointer = if targeted { r.rsi } else { r.rdi };
            let default = if targeted { r.r15 as i32 } else { r.rbx as i32 };
            let days = if r.rax as i32 == 0 {
                default
            } else {
                r.rax as i32
            };

            if let Err(reason) = emit_selected(
                pointer,
                targeted,
                EventFeedSource::DecisionDurationChanged,
                Some(1),
                Some(days),
            ) {
                feed::refuse(reason);
            }
        }
        17 => {
            let memory = unsafe { engine_native::memory::LiveMemory::new() };
            match field::<u8>(&memory, r.rbp, 0x48) {
                Ok(value) => feed::decisions::flags(r.rcx, Some(value != 0), false),
                Err(reason) => feed::refuse(reason),
            }
        }
        19 => {
            let memory = unsafe { engine_native::memory::LiveMemory::new() };
            match field::<u64>(&memory, r.r8, 0) {
                Ok(pointer) if pointer != 0 => {
                    feed::decisions::retire(pointer as usize, true, false)
                }
                Ok(_) => {}
                Err(reason) => feed::refuse(reason),
            }
        }
        18 => {
            feed::decisions::marker(r.r14, Operation::Rebuild, 3, 0, 0);
            clear_candidates(r.r14);
        }
        _ => feed::refuse("decision retained-state tap identity differs"),
    }
}
pub(super) unsafe fn hooks(
    memory: &impl Memory,
    base: usize,
) -> Result<(Vec<Hook>, Vec<Tap>), &'static str> {
    let verify = |site: native::Site| verify_hook(memory, base, site, site.sentinel.len());
    let callbacks = [
        timed as *const () as usize,
        targeted as *const () as usize,
        targeted_timed as *const () as usize,
        removal as *const () as usize,
        taken as *const () as usize,
        erase_range as *const () as usize,
        reset as *const () as usize,
        restore as *const () as usize,
        random as *const () as usize,
        country_candidate as *const () as usize,
        state_candidates as *const () as usize,
        global_states as *const () as usize,
        owned_states as *const () as usize,
        controlled_states as *const () as usize,
        related_states as *const () as usize,
        filter as *const () as usize,
        ignore as *const () as usize,
        unignore as *const () as usize,
        hourly as *const () as usize,
        refresh as *const () as usize,
    ];
    let mut hooks = Vec::new();
    let mut taps = Vec::new();
    for (index, (site, callback)) in sites::ENTRIES.into_iter().zip(callbacks).enumerate() {
        let hook = unsafe { Hook::new(verify(site)?, callback, site.sentinel) }
            .map_err(|cause| hook_failure(site, "construction", cause))?
            .named(site.label());
        ORIGINAL[index].store(hook.trampoline(), Ordering::Release);
        hooks.push(hook);
    }
    let callbacks = [
        append_taken as *const () as usize,
        append_removal as *const () as usize,
        transfer as *const () as usize,
        append_reenable as *const () as usize,
        expiry_transfer as *const () as usize,
        clean_targeted as *const () as usize,
        clean_targeted_timed as *const () as usize,
        reactivate_days as *const () as usize,
        reactivate_state as *const () as usize,
        console_days as *const () as usize,
        console_all_days as *const () as usize,
        ignore_selected as *const () as usize,
        request_timed as *const () as usize,
        request_targeted as *const () as usize,
    ];
    for (index, (site, callback)) in sites::CALLS.into_iter().zip(callbacks).enumerate() {
        let len = sites::call_sentinel_len(site);
        let at = verify_hook(memory, base, site, len)?;
        let offset = i32::from_le_bytes(site.sentinel[1..5].try_into().unwrap());
        let target = at
            .checked_add(5)
            .and_then(|p| p.checked_add_signed(offset as isize))
            .ok_or_else(|| hook_failure(site, "construction", "selected call target overflow"))?;
        let hook = unsafe { Hook::new_call_span(at, callback, site.sentinel, len) }
            .map_err(|cause| hook_failure(site, "construction", cause))?
            .named(site.label());
        CALL[index].store(target, Ordering::Release);
        hooks.push(hook);
    }
    for (index, site) in sites::TAPS.into_iter().enumerate() {
        if site.rva == sites::TARGETED_SELECTION_RESULT.rva {
            continue;
        }
        taps.push(
            unsafe { Tap::new(verify(site)?, tap, index, site.sentinel) }
                .map_err(|cause| hook_failure(site, "construction", cause))?
                .named(site.label()),
        );
    }
    Ok((hooks, taps))
}

pub(super) unsafe fn selection_tap(memory: &impl Memory, base: usize) -> Result<Tap, &'static str> {
    let site = sites::TARGETED_SELECTION_RESULT;
    let at = verify_hook(memory, base, site, site.sentinel.len())?;
    unsafe { Tap::new(at, tap, 16, site.sentinel) }
        .map(|tap| tap.named(site.label()))
        .map_err(|cause| hook_failure(site, "construction", cause))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event_feed_history::{OwnerBoundary, Snapshot, Store};
    use engine_decisions::{cdecision, cdecision_cooldown as cooldown, cdecision_status};
    use engine_events::event_feed_sites::provenance::World;
    use engine_native::{ccountry, cgame_state, cpdx_array};
    use replay_data::event_feed_rows::{
        EventFeedCoverage, EventFeedScriptChange,
        decisions::{DecisionChange, DecisionState},
    };
    use std::collections::BTreeMap;

    const GAME: usize = 0x10000;
    const COUNTRY: usize = 0x20000;
    const STATUS: usize = 0x30000;
    const DATA: usize = 0x40000;

    #[derive(Default)]
    struct Fixture(BTreeMap<usize, u8>);
    impl Fixture {
        fn bytes(&mut self, address: usize, bytes: &[u8]) {
            for (offset, byte) in bytes.iter().enumerate() {
                self.0.insert(address + offset, *byte);
            }
        }
        fn q(&mut self, address: usize, value: usize) {
            self.bytes(address, &(value as u64).to_le_bytes());
        }
        fn d(&mut self, address: usize, value: u32) {
            self.bytes(address, &value.to_le_bytes());
        }
        fn array(&mut self, header: usize, data: usize, count: u32) {
            self.q(header, data);
            self.d(header + cpdx_array::CAPACITY, count);
            self.d(header + cpdx_array::COUNT, count);
        }
        fn pointer(ordinal: usize) -> usize {
            0x50000 + ordinal * 0x100
        }
        fn new(days: [i32; 5]) -> Self {
            let mut memory = Self::default();
            memory.q(cgame_state::SINGLETON_RVA, GAME);
            memory.d(GAME + cgame_state::CURRENT_HOUR, 7);
            memory.array(GAME + cgame_state::COUNTRIES, 0x1000, 2);
            memory.array(GAME + cgame_state::COUNTRY_LINKS, 0x2000, 2);
            memory.q(0x1000 + 8, COUNTRY);
            memory.d(0x2000 + 4, 1);
            memory.q(COUNTRY, ccountry::VTABLE_RVA);
            memory.d(COUNTRY + ccountry::ID, 1);
            memory.d(COUNTRY + ccountry::ID_INSTANCE, 44);
            memory.q(COUNTRY + ccountry::POLITICAL_STATUS, 0);
            memory.q(COUNTRY + ccountry::DECISION_STATUS, STATUS);
            memory.array(COUNTRY + runtime::IGNORED_TOKENS, 0, 0);
            memory.q(STATUS, cdecision_status::VTABLE_RVA);
            memory.q(STATUS + cdecision_status::COUNTRY, COUNTRY);
            for offset in runtime::HEADERS
                .into_iter()
                .chain(runtime::CANDIDATE_HEADERS)
                .chain([cdecision_status::RANDOM_ITEMS])
            {
                memory.array(STATUS + offset, 0, 0);
            }
            memory.array(STATUS + runtime::HEADERS[rows::REENABLE as usize], DATA, 5);
            for (ordinal, days) in days.into_iter().enumerate() {
                let pointer = Self::pointer(ordinal);
                let definition = 0x60000 + ordinal * 0x1000;
                memory.q(DATA + ordinal * 8, pointer);
                memory.q(pointer, cooldown::VTABLE_RVA);
                memory.q(pointer + cooldown::DEFINITION, definition);
                memory.d(pointer + cooldown::DAYS, days as u32);
                memory.q(definition, cdecision::VTABLE_RVA);
                memory.bytes(definition + runtime::PERMANENT, &[u8::from(ordinal % 2 == 1)]);
                let token = format!("cooldown_{ordinal}");
                let mut header = [0; 32];
                header[..token.len()].copy_from_slice(token.as_bytes());
                header[16..24].copy_from_slice(&(token.len() as u64).to_le_bytes());
                header[24..].copy_from_slice(&15_u64.to_le_bytes());
                memory.bytes(definition + cdecision::TOKEN, &header);
            }
            memory
        }
    }
    impl Memory for Fixture {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            for (offset, destination) in output.iter_mut().enumerate() {
                let Some(byte) = self.0.get(&(address + offset)) else {
                    return false;
                };
                *destination = *byte;
            }
            true
        }
    }
    fn reduce(store: &mut Store, state: &mut DecisionState) -> Vec<(Operation, u8, Option<u32>)> {
        let mut snapshot = Snapshot::new(0);
        store
            .snapshot_growing(
                &mut snapshot,
                OwnerBoundary {
                    generation: 1,
                    read_hour: 7,
                    coverage: EventFeedCoverage::Complete,
                    failure: None,
                },
            )
            .unwrap();
        let pack = snapshot.prepare_pack().unwrap();
        let operations = pack
            .row()
            .events
            .iter()
            .map(|event| {
                let script = event.script.unwrap();
                let Some(EventFeedScriptChange::Decision(change)) = script.change else {
                    panic!("expected a retained decision operation");
                };
                state.observe(pack.row(), event, script, change).unwrap();
                (change.operation, change.flags, script.instance)
            })
            .collect();
        store.acknowledged(pack.publication());
        operations
    }
    fn marker(store: &mut Store, memory: &Fixture, operation: Operation, ordinal: u32) {
        store
            .decision_marker(
                &World::new(memory, 0).unwrap(),
                memory,
                0,
                1,
                COUNTRY,
                DecisionChange {
                    ordinal,
                    ..DecisionChange::new(operation, rows::REENABLE)
                },
            )
            .unwrap();
    }

    #[test]
    fn reenable_erase_observation_retires_last_expiry_and_does_not_duplicate_nonlast_expiry() {
        for expired in [4, 0] {
            let mut memory = Fixture::new(if expired == 4 {
                [11, 0, 19, 0, 1]
            } else {
                [1, 0, 19, 0, 13]
            });
            let mut store = Store::new(None).unwrap();
            store.prepare_capacity(32).unwrap();
            store.prepare_scripts(256, 0, 0).unwrap();
            store.prepare_decisions(5).unwrap();
            let mut state = DecisionState::default();
            reduce(&mut store, &mut state); // Admit the observed generation.
            store
                .decision_baseline(
                    &memory,
                    0,
                    1,
                    GAME,
                    7,
                    ccountry::Country {
                        pointer: COUNTRY,
                        canonical_slot: 1,
                        id: 1,
                    },
                )
                .unwrap();
            reduce(&mut store, &mut state);
            assert!(state.complete());
            marker(&mut store, &memory, Operation::PassBegin, 5);
            REENABLE.set(STATUS);
            memory.d(Fixture::pointer(expired) + cooldown::DAYS, 0);
            if expired != 4 {
                // The existing lifecycle witness precedes the swap and null tail.
                store
                    .decision_retire(
                        &World::new(&memory, 0).unwrap(),
                        &memory,
                        0,
                        1,
                        Fixture::pointer(expired),
                        true,
                        true,
                    )
                    .unwrap();
                memory.q(DATA + expired * 8, Fixture::pointer(4));
                memory.q(DATA + 4 * 8, 0);
            }
            let mut retirements = 0;
            observe_erase_range(
                &memory,
                STATUS + runtime::HEADERS[rows::REENABLE as usize],
                4,
                5,
                |pointer, swap, visited| {
                    retirements += 1;
                    store
                        .decision_retire(
                            &World::new(&memory, 0).unwrap(),
                            &memory,
                            0,
                            1,
                            pointer,
                            swap,
                            visited,
                        )
                        .unwrap();
                },
            )
            .unwrap();
            assert_eq!(retirements, usize::from(expired == 4));
            assert_eq!(REENABLE.get(), STATUS); // Erasure must leave the pass open.
            assert_eq!(
                reduce(&mut store, &mut state),
                [
                    (Operation::PassBegin, 0, None),
                    (Operation::PassCut, 0, Some(expired as u32 + 1)),
                    (
                        Operation::Erase,
                        rows::SWAP | rows::ITERATION,
                        Some(expired as u32 + 1),
                    ),
                ]
            );
            assert!(!state.complete());
            memory.d(
                STATUS + runtime::HEADERS[rows::REENABLE as usize] + cpdx_array::COUNT,
                4,
            );
            marker(&mut store, &memory, Operation::PassEnd, 0);
            REENABLE.set(0);
            assert_eq!(
                reduce(&mut store, &mut state),
                [(Operation::PassEnd, 0, None)]
            );
            assert!(state.complete());
            assert_eq!(
                state
                    .members(rows::REENABLE)
                    .unwrap()
                    .iter()
                    .map(|row| (row.instance, row.days, row.flags))
                    .collect::<Vec<_>>(),
                [
                    if expired == 4 { (1, 10, 0) } else { (5, 12, 0) },
                    (2, 0, rows::PERMANENT),
                    (3, 18, 0),
                    (4, 0, rows::PERMANENT),
                ]
            );
        }
    }

    #[test]
    fn reenable_iteration_erasure_requires_the_active_owners_single_tail_slot() {
        let mut memory = Fixture::new([11, 0, 19, 0, 1]);
        let header = STATUS + runtime::HEADERS[rows::REENABLE as usize];
        let other_header = STATUS + runtime::HEADERS[rows::REMOVAL as usize];
        memory.array(other_header, DATA, 5);
        for (active, header, first, last) in [
            (0, header, 4, 5),
            (STATUS + 0x1000, header, 4, 5),
            (usize::MAX, header, 4, 5),
            (STATUS, other_header, 4, 5),
            (STATUS, header, 0, 1),
            (STATUS, header, 3, 5),
        ] {
            REENABLE.set(active);
            let mut retired = Vec::new();
            observe_erase_range(&memory, header, first, last, |pointer, swap, visited| {
                retired.push((pointer, swap, visited));
            })
            .unwrap();
            assert_eq!(
                retired,
                (first..last)
                    .map(|ordinal| (Fixture::pointer(ordinal), false, false))
                    .collect::<Vec<_>>()
            );
        }
        REENABLE.set(STATUS);
        memory.q(DATA + 4 * 8, 0);
        observe_erase_range(&memory, header, 4, 5, |_, _, _| {
            panic!("a null tail must not repeat its lifecycle retirement");
        })
        .unwrap();
        REENABLE.set(0);
    }
}
