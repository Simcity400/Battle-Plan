use super::*;
use super::{hook_failure, verify_hook};
use engine_decisions::decision_runtime as runtime;
use engine_events::script_event_sites::decisions as sites;
use replay_data::event_feed_rows::decisions::{self as rows, Operation};
use std::cell::Cell;
static ORIGINAL: [AtomicUsize; 27] = [const { AtomicUsize::new(0) }; 27];
static CALL: [AtomicUsize; 17] = [const { AtomicUsize::new(0) }; 17];
#[doc = " [`sites::ENTRIES`] from the first enclosing detour of a pre-store writer."]
const DETOURS: usize = 20;
#[doc = " [`sites::CALLS`] of the duration adds and the replaced-event retirement."]
const EXTEND_MISSION_TIMEOUT: usize = 14;
const EXTEND_DECISION_REMOVAL: usize = 15;
const REPLACE_EVENT_RETIREMENT: usize = 16;
std::thread_local! {static REENABLE:Cell<usize>=const {Cell::new(0)};static REACTIVATION:Cell<(usize,i32)>=const {Cell::new((0,0))};}
fn count(status: usize, container: u8) -> Result<u32, &'static str> {
    let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
    let offset = *runtime::HEADERS
        .get(container as usize)
        .ok_or("decision pass container differs")?;
    let count = field::<i32>(&memory, status, offset + engine_native_memory::cpdx_array::COUNT)?;
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
/// # Safety
/// The arguments are ones the selected site's published original accepts with its ABI, and
/// every object they reach stays valid for the reads its site makes; one the original destroys
/// is an identity only once it returns.
unsafe fn pass(status: usize, arg: usize, index: usize, container: u8) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::EventFeedProvenance);
    let mut flight = Flight::enter();
    let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
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
    let array = engine_native_memory::cpdx_array::Array::read(memory, header, 8)
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
    let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
    if let Err(reason) = observe_erase_range(&memory, header, first, last, feed::decisions::retire)
    {
        feed::refuse(reason);
    }
    type Original = unsafe extern "system-unwind" fn(usize, usize, usize) -> usize;
    let original: Original = unsafe { std::mem::transmute(ORIGINAL[5].load(Ordering::Acquire)) };
    let result = timing.original(|| unsafe { original(header, first, last) });
    flight.returned();
    result
}
/// # Safety
/// The arguments are ones the selected site's published original accepts with its ABI, and
/// every object they reach stays valid for the reads its site makes; one the original destroys
/// is an identity only once it returns.
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
            && engine_sites::at(base, engine_decisions::cdecision_status::VTABLE_RVA).is_some_and(
                |vtable| engine_native_memory::read::<u64>(memory, status) == Some(vtable as u64),
            )
        {
            return Ok((status, kind as u8));
        }
    }
    Err("decision candidate append owner differs")
}
fn appended_candidates(header: usize, before: Result<u32, &'static str>) {
    let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
    let result = (|| {
        let before = before?;
        let (status, kind) = candidate_owner(&memory, header)?;
        let array =
            engine_native_memory::cpdx_array::Array::read(&memory, header, runtime::CANDIDATE_STRIDE)
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
    let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
    engine_native_memory::cpdx_array::Array::read(&memory, header, runtime::CANDIDATE_STRIDE)
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
    let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
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
/// # Safety
/// The arguments are ones the selected site's published original accepts with its ABI, and
/// every object they reach stays valid for the reads its site makes; one the original destroys
/// is an identity only once it returns.
unsafe fn global_candidates(closure: usize, arg: usize, index: usize) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::EventFeedProvenance);
    let mut flight = Flight::enter();
    let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
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
/// # Safety
/// The arguments are ones the selected site's published original accepts with its ABI, and
/// every object they reach stays valid for the reads its site makes; one the original destroys
/// is an identity only once it returns.
unsafe fn ignore_token(container: usize, token: usize, index: usize) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::EventFeedProvenance);
    let mut flight = Flight::enter();
    let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
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
/// # Safety
/// The arguments are ones the selected site's published original accepts with its ABI, and
/// every object they reach stays valid for the reads its site makes; one the original destroys
/// is an identity only once it returns.
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
/// # Safety
/// The arguments are ones the selected site's published original accepts with its ABI, and
/// every object they reach stays valid for the reads its site makes; one the original destroys
/// is an identity only once it returns.
unsafe fn detour(a: usize, b: usize, c: usize, index: usize) -> usize {
    use super::staging::{Detour, Unit, Witness};
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::EventFeedProvenance);
    let unit = match index - DETOURS {
        0 => Unit::AutoActivation,
        1 => Unit::ScriptedActivation,
        2 => Unit::TimedSelection,
        3 => Unit::TargetedIgnore,
        4 => Unit::IgnoreAll,
        5 => Unit::RemovalRequest,
        _ => Unit::CandidateFinalization,
    };
    let detour = Detour::enter(unit);
    type Original = unsafe extern "system-unwind" fn(usize, usize, usize) -> usize;
    let original: Original =
        unsafe { std::mem::transmute(ORIGINAL[index].load(Ordering::Acquire)) };
    let result = timing.original(|| unsafe { original(a, b, c) });
    if unit == Unit::CandidateFinalization {
        #[cfg(test)]
        super::staging::published(super::staging::Published::Candidates { status: a });
        feed::decisions::marker(a, Operation::Rebuild, 3, 0, 0);
        clear_candidates(a);
    }
    detour.publish(|witness| match witness {
        Witness::Reactivated {
            status,
            definition,
            instance,
        } => super::reactivated(status, definition, instance),
        Witness::Selected { instance } => selected_result(instance, false),
        Witness::Ignored { instance, was } => committed_ignore(instance, was),
        Witness::RemovalRequested { instance, was } => committed_removal(instance, was),
        Witness::FiredMatch { .. } => feed::refuse(super::staging::OTHER_FRAME),
    });
    result
}
macro_rules! detour_entry {
    ($name:ident, $index:expr) => {
        unsafe extern "system-unwind" fn $name(a: usize, b: usize, c: usize) -> usize {
            unsafe { detour(a, b, c, $index) }
        }
    };
}
detour_entry!(auto_activation, DETOURS);
detour_entry!(scripted_activation, DETOURS + 1);
detour_entry!(timed_selection, DETOURS + 2);
detour_entry!(targeted_ignore_command, DETOURS + 3);
detour_entry!(ignore_all_command, DETOURS + 4);
detour_entry!(removal_request, DETOURS + 5);
detour_entry!(candidate_finalization, DETOURS + 6);
pub(super) fn selected_result(instance: usize, targeted: bool) {
    let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
    let at = if targeted {
        engine_decisions::ctargeted_decision::DAYS
    } else {
        engine_decisions::ctimed_decision::DAYS
    };
    let result = field::<i32>(&memory, instance, at).and_then(|days| {
        #[cfg(test)]
        super::staging::published(super::staging::Published::Selection { instance, days });
        emit_selected(
            instance,
            targeted,
            EventFeedSource::DecisionDurationChanged,
            Some(1),
            Some(days),
        )
    });
    if let Err(reason) = result {
        feed::refuse(reason);
    }
}
fn committed_ignore(instance: usize, was: bool) {
    let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
    match field::<u8>(
        &memory,
        instance,
        engine_decisions::ctargeted_decision::IGNORE,
    ) {
        Ok(now) => {
            #[cfg(test)]
            super::staging::published(super::staging::Published::Ignore {
                instance,
                was,
                now: now != 0,
            });
            feed::decisions::flags_committed(instance, Some((was, now != 0)), None)
        }
        Err(reason) => feed::refuse(reason),
    }
}
fn committed_removal(instance: usize, was: bool) {
    let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
    let stored = field::<u8>(&memory, instance, runtime::REMOVE_REQUESTED);
    #[cfg(test)]
    if let Ok(now) = stored {
        super::staging::published(super::staging::Published::Removal { instance, was, now });
    }
    match stored {
        Ok(1) => feed::decisions::flags_committed(instance, None, Some(was)),
        Ok(_) => {}
        Err(reason) => feed::refuse(reason),
    }
}
/// # Safety
/// Only the replaced-event retirement call's detour calls this, on the native thread running it,
/// with that call's arguments.
unsafe fn retirement(a: usize, b: usize, c: usize) -> usize {
    use super::staging::{Detour, Unit, Witness};
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::EventFeedProvenance);
    let detour = Detour::enter(Unit::FiredRetirement);
    type Original = unsafe extern "system-unwind" fn(usize, usize, usize) -> usize;
    let original: Original =
        unsafe { std::mem::transmute(CALL[REPLACE_EVENT_RETIREMENT].load(Ordering::Acquire)) };
    let result = timing.original(|| unsafe { original(a, b, c) });
    detour.publish(|witness| match witness {
        Witness::FiredMatch { state, definition } => super::retired_fired(state, definition),
        _ => feed::refuse(super::staging::OTHER_FRAME),
    });
    result
}
/// # Safety
/// The arguments are ones the selected site's published original accepts with its ABI, and
/// every object they reach stays valid for the reads its site makes; one the original destroys
/// is an identity only once it returns.
unsafe fn call(a: usize, b: usize, c: usize, index: usize) -> usize {
    if index == REPLACE_EVENT_RETIREMENT {
        return unsafe { retirement(a, b, c) };
    }
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::EventFeedProvenance);
    let mut flight = Flight::enter();
    let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
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
                    let was =
                        field::<u8>(&memory, b, engine_decisions::ctargeted_decision::IGNORE)?;
                    super::staging::stage(
                        super::staging::Unit::IgnoreAll,
                        super::staging::Witness::Ignored {
                            instance: b,
                            was: was != 0,
                        },
                    );
                }
            }
            12 | 13 => {
                if result != 0 {
                    let was = field::<u8>(&memory, result, runtime::REMOVE_REQUESTED)?;
                    super::staging::stage(
                        super::staging::Unit::RemovalRequest,
                        super::staging::Witness::RemovalRequested {
                            instance: result,
                            was: was == 1,
                        },
                    );
                }
            }
            EXTEND_MISSION_TIMEOUT | EXTEND_DECISION_REMOVAL => {
                super::lifecycle::adjustments::extended(a, c, index == EXTEND_DECISION_REMOVAL);
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
call_entry!(extend_mission_timeout, EXTEND_MISSION_TIMEOUT);
call_entry!(extend_decision_removal, EXTEND_DECISION_REMOVAL);
call_entry!(replace_event_retirement, REPLACE_EVENT_RETIREMENT);
core::arch::global_asm!(
    ".text",
    ".globl event_feed_extend_mission_timeout",
    "event_feed_extend_mission_timeout:",
    "mov r8, r15",
    "jmp {timeout}",
    ".globl event_feed_extend_decision_removal",
    "event_feed_extend_decision_removal:",
    "mov r8, rsi",
    "jmp {removal}",
    timeout = sym extend_mission_timeout,
    removal = sym extend_decision_removal,
);
unsafe extern "system-unwind" {
    fn event_feed_extend_mission_timeout();
    fn event_feed_extend_decision_removal();
}
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
            let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
            match field::<u64>(&memory, r.r14, 0) {
                Ok(pointer) => feed::decisions::cut(pointer as usize),
                Err(reason) => feed::refuse(reason),
            }
            if r.rax as i32 > 0 {
                feed::decisions::taken_erase(r.rdi, r.rbx, None, true);
            }
        }
        14 => feed::decisions::retire(r.r8, true, true),
        15 => super::staging::stage(
            super::staging::Unit::TimedSelection,
            super::staging::Witness::Selected { instance: r.rdi },
        ),
        16 => super::staging::stage(
            super::staging::Unit::TargetedSelection,
            super::staging::Witness::Selected { instance: r.rsi },
        ),
        17 => {
            let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
            match field::<u8>(&memory, r.rcx, engine_decisions::ctargeted_decision::IGNORE) {
                Ok(was) => super::staging::stage(
                    super::staging::Unit::TargetedIgnore,
                    super::staging::Witness::Ignored {
                        instance: r.rcx,
                        was: was != 0,
                    },
                ),
                Err(reason) => feed::refuse(reason),
            }
        }
        19 => {
            let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
            match field::<u64>(&memory, r.r8, 0) {
                Ok(pointer) if pointer != 0 => {
                    feed::decisions::retire(pointer as usize, true, false)
                }
                Ok(_) => {}
                Err(reason) => feed::refuse(reason),
            }
        }
        _ => feed::refuse("decision retained-state tap identity differs"),
    }
}
/// # Safety
/// `base` identifies the authenticated executable in this process. The caller owns these sites
/// exclusively and restores the hooks and quiesces native calls before releasing code or unloading.
pub(super) unsafe fn hooks(base: usize) -> Result<(Vec<Hook>, Vec<Tap>), String> {
    let verify = |site: native::Site| verify_hook::<16>(base, site);
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
        auto_activation as *const () as usize,
        scripted_activation as *const () as usize,
        timed_selection as *const () as usize,
        targeted_ignore_command as *const () as usize,
        ignore_all_command as *const () as usize,
        removal_request as *const () as usize,
        candidate_finalization as *const () as usize,
    ];
    let mut hooks = Vec::new();
    let mut taps = Vec::new();
    for (index, (site, callback)) in sites::ENTRIES.into_iter().zip(callbacks).enumerate() {
        let (at, original) = verify(site)?;
        let hook = unsafe { Hook::new(at, callback, original) }
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
        event_feed_extend_mission_timeout as *const () as usize,
        event_feed_extend_decision_removal as *const () as usize,
        replace_event_retirement as *const () as usize,
    ];
    for (index, (site, callback)) in sites::CALLS.into_iter().zip(callbacks).enumerate() {
        let (at, original, len) = verify_call(base, site)?;
        let offset = i32::from_le_bytes([original[1], original[2], original[3], original[4]]);
        let target = at
            .checked_add(5)
            .and_then(|p| p.checked_add_signed(offset as isize))
            .ok_or_else(|| hook_failure(site, "construction", "selected call target overflow"))?;
        let hook = unsafe { Hook::new_call_span(at, callback, original, len) }
            .map_err(|cause| hook_failure(site, "construction", cause))?
            .named(site.label());
        CALL[index].store(target, Ordering::Release);
        hooks.push(hook);
    }
    for (index, site) in sites::TAPS.into_iter().enumerate() {
        if site.rva == sites::TARGETED_SELECTION_RESULT.rva
            || site.rva == sites::CLEAR_CANDIDATES.rva
        {
            continue;
        }
        let (at, original) = verify(site)?;
        taps.push(
            unsafe { Tap::new(at, tap, index, original) }
                .map_err(|cause| hook_failure(site, "construction", cause))?
                .named(site.label()),
        );
    }
    Ok((hooks, taps))
}

#[doc = " Adjacent CALLs own only their bytes before the next one."]
pub(super) const REACTIVATE_DAYS_LEN: usize =
    sites::REACTIVATE_TARGETED_STATE.rva.0 - sites::REACTIVATE_TARGETED_DAYS.rva.0;
pub(super) const REQUEST_TIMED_LEN: usize =
    sites::REQUEST_TARGETED.rva.0 - sites::REQUEST_TARGETED_TIMED.rva.0;

#[doc = " A CALL site's address, its patch bytes and the span it owns."]
fn verify_call(base: usize, site: native::Site) -> Result<(usize, [u8; 16], usize), String> {
    Ok(if site.rva == sites::REACTIVATE_TARGETED_DAYS.rva {
        let (at, original) = verify_hook::<REACTIVATE_DAYS_LEN>(base, site)?;
        (at, original, REACTIVATE_DAYS_LEN)
    } else if site.rva == sites::REQUEST_TARGETED_TIMED.rva {
        let (at, original) = verify_hook::<REQUEST_TIMED_LEN>(base, site)?;
        (at, original, REQUEST_TIMED_LEN)
    } else {
        let (at, original) = verify_hook::<16>(base, site)?;
        (at, original, 16)
    })
}

/// # Safety
/// `base` identifies the authenticated executable in this process. The caller owns these sites
/// exclusively and restores the hooks and quiesces native calls before releasing code or unloading.
pub(super) unsafe fn selection_tap(base: usize) -> Result<Tap, String> {
    let site = sites::TARGETED_SELECTION_RESULT;
    let (at, original) = verify_hook::<16>(base, site)?;
    unsafe { Tap::new(at, tap, 16, original) }
        .map(|tap| tap.named(site.label()))
        .map_err(|cause| hook_failure(site, "construction", cause))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event_feed_history::{OwnerBoundary, Snapshot, Store};
    use engine_decisions::{cdecision, cdecision_cooldown as cooldown, cdecision_status};
    use engine_events::event_feed_sites::provenance::World;
    use engine_native::{ccountry, cgame_state};
    use engine_native_memory::cpdx_array;
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
            memory.q(cgame_state::SINGLETON_RVA.0, GAME);
            memory.d(GAME + cgame_state::CURRENT_HOUR, 7);
            memory.array(GAME + cgame_state::COUNTRIES, 0x1000, 2);
            memory.array(GAME + cgame_state::COUNTRY_LINKS, 0x2000, 2);
            memory.q(0x1000 + 8, COUNTRY);
            memory.d(0x2000 + 4, 1);
            memory.q(COUNTRY, ccountry::VTABLE_RVA.0);
            memory.d(COUNTRY + ccountry::ID, 1);
            memory.d(COUNTRY + ccountry::SURRENDER_TARGET, 44);
            memory.q(COUNTRY + ccountry::POLITICAL_STATUS, 0);
            memory.q(COUNTRY + ccountry::DECISION_STATUS, STATUS);
            memory.array(COUNTRY + runtime::IGNORED_TOKENS, 0, 0);
            memory.q(STATUS, cdecision_status::VTABLE_RVA.0);
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
                memory.q(pointer, cooldown::VTABLE_RVA.0);
                memory.q(pointer + cooldown::DEFINITION, definition);
                memory.d(pointer + cooldown::DAYS, days as u32);
                memory.q(definition, cdecision::VTABLE_RVA.0);
                memory.bytes(
                    definition + runtime::PERMANENT,
                    &[u8::from(ordinal % 2 == 1)],
                );
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
            .snapshot(
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
        let _globals = crate::event_feed_observer::test_globals();
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
            reduce(&mut store, &mut state);
            store
                .decision_baseline(
                    &memory,
                    0,
                    1,
                    GAME,
                    7,
                    engine_native_memory::country_identity::Country {
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
            assert_eq!(REENABLE.get(), STATUS);
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

    const COUNTRY2: usize = 0x28000;
    const STATUS2: usize = 0x38000;
    fn second_owner(memory: &mut Fixture) {
        memory.array(GAME + cgame_state::COUNTRIES, 0x1000, 3);
        memory.array(GAME + cgame_state::COUNTRY_LINKS, 0x2000, 3);
        memory.q(0x1000 + 16, COUNTRY2);
        memory.d(0x2000 + 8, 2);
        memory.q(COUNTRY2, ccountry::VTABLE_RVA.0);
        memory.d(COUNTRY2 + ccountry::ID, 2);
        memory.d(COUNTRY2 + ccountry::SURRENDER_TARGET, 45);
        memory.q(COUNTRY2 + ccountry::POLITICAL_STATUS, 0);
        memory.q(COUNTRY2 + ccountry::DECISION_STATUS, STATUS2);
        memory.array(COUNTRY2 + runtime::IGNORED_TOKENS, 0, 0);
        memory.q(STATUS2, cdecision_status::VTABLE_RVA.0);
        memory.q(STATUS2 + cdecision_status::COUNTRY, COUNTRY2);
        for offset in runtime::HEADERS
            .into_iter()
            .chain(runtime::CANDIDATE_HEADERS)
            .chain([cdecision_status::RANDOM_ITEMS])
        {
            memory.array(STATUS2 + offset, 0, 0);
        }
    }
    fn publish(
        store: &mut Store,
        state: &mut replay_data::event_feed_rows::ScriptEventState,
        hour: i32,
    ) -> crate::event_feed_history::Prepared {
        let mut snapshot = Snapshot::new(0);
        store
            .snapshot(
                &mut snapshot,
                OwnerBoundary {
                    generation: 1,
                    read_hour: hour,
                    coverage: EventFeedCoverage::Complete,
                    failure: None,
                },
            )
            .unwrap();
        let pack = snapshot.prepare_pack().unwrap();
        state.observe(pack.row()).unwrap();
        store.acknowledged(pack.publication());
        pack
    }

    #[test]
    fn a_result_for_an_uncertified_owner_keeps_its_occurrence_and_refuses_only_that_owner() {
        let _globals = crate::event_feed_observer::test_globals();
        const REASON: &str = "decision result reached an uncertified owner";
        let first = replay_data::event_feed_rows::EventFeedCountry {
            reference: 1,
            id: 1,
            instance: 44,
        };
        let second = replay_data::event_feed_rows::EventFeedCountry {
            reference: 2,
            id: 2,
            instance: 45,
        };
        let mut memory = Fixture::new([11, 0, 19, 0, 1]);
        second_owner(&mut memory);
        let mut store = Store::new(None).unwrap();
        store.prepare_capacity(64).unwrap();
        store.prepare_scripts(1024, 0, 0).unwrap();
        store.prepare_decisions(8).unwrap();
        let mut state = replay_data::event_feed_rows::ScriptEventState::default();
        publish(&mut store, &mut state, 7);
        for (pointer, id) in [(COUNTRY, 1), (COUNTRY2, 2)] {
            store
                .decision_baseline(
                    &memory,
                    0,
                    1,
                    GAME,
                    7,
                    engine_native_memory::country_identity::Country {
                        pointer,
                        canonical_slot: id,
                        id,
                    },
                )
                .unwrap();
        }
        publish(&mut store, &mut state, 8);
        assert!(state.decisions(first).is_ok());
        assert!(state.decisions(second).is_ok());

        let pointer = Fixture::pointer(2);
        marker(&mut store, &memory, Operation::Invalidate, 0);
        let world = World::new(&memory, 0).unwrap();
        assert_eq!(
            store.decision_result(
                &world,
                &memory,
                0,
                1,
                pointer,
                (Some(1), Some(30)),
                EventFeedSource::DecisionDurationChanged,
                REASON,
            ),
            Ok(false)
        );
        store
            .decision_selected(
                &world,
                &memory,
                0,
                1,
                COUNTRY,
                pointer,
                DecisionChange::new(Operation::Store, rows::REENABLE),
                false,
                (None, None),
            )
            .unwrap();
        let id = store.decisions.uncertified(pointer).unwrap().id;
        assert_ne!(id, 3);
        let result = crate::test_support::with_allocations_denied(|| {
            store.decision_result(
                &world,
                &memory,
                0,
                1,
                pointer,
                (Some(1), Some(30)),
                EventFeedSource::DecisionDurationChanged,
                REASON,
            )
        });
        assert_eq!(result, Ok(true));
        let refused = publish(&mut store, &mut state, 9);
        let row = refused.row();
        assert_eq!(row.coverage, EventFeedCoverage::Complete);
        assert!(row.failure.is_none());
        assert_eq!(row.counts.unwrap().rejected, 1);
        let changes: Vec<_> = row
            .events
            .iter()
            .map(|event| {
                let script = event.script.unwrap();
                let Some(EventFeedScriptChange::Decision(change)) = script.change else {
                    panic!("expected a retained decision operation");
                };
                (event.actor, change.operation, script.instance)
            })
            .collect();
        assert_eq!(
            changes,
            [
                (first, Operation::Invalidate, None),
                (first, Operation::Store, Some(id)),
                (first, Operation::Refused, None),
                (first, Operation::Update, Some(id)),
            ]
        );
        let marker = row.events[2].script.unwrap();
        let start = marker.token_start as usize;
        assert_eq!(
            &row.script_tokens[start..start + marker.token_len as usize],
            REASON.as_bytes()
        );
        let occurrence = row.events[3];
        assert_eq!(occurrence.source, EventFeedSource::DecisionDurationChanged);
        assert_eq!(occurrence.script.unwrap().days, Some(30));
        assert_eq!(occurrence.script.unwrap().lifecycle, Some(1));
        assert!(state.decisions(first).is_err());
        assert!(state.decisions(second).is_ok());

        assert_eq!(
            store.decision_result(
                &world,
                &memory,
                0,
                1,
                0x99000,
                (Some(1), Some(30)),
                EventFeedSource::DecisionDurationChanged,
                REASON,
            ),
            Ok(false)
        );
        store
            .decision_baseline(
                &memory,
                0,
                1,
                GAME,
                7,
                engine_native_memory::country_identity::Country {
                    pointer: COUNTRY,
                    canonical_slot: 1,
                    id: 1,
                },
            )
            .unwrap();
        assert!(store.decisions.find(pointer).is_some());
        assert_eq!(
            store.decision_result(
                &world,
                &memory,
                0,
                1,
                pointer,
                (Some(1), Some(30)),
                EventFeedSource::DecisionDurationChanged,
                REASON,
            ),
            Ok(true)
        );
        let certified = publish(&mut store, &mut state, 10);
        assert!(owner_refused(certified.row()).is_none());
        assert_eq!(certified.row().counts.unwrap().rejected, 1);
        assert!(state.decisions(first).is_ok());
        assert!(state.decisions(second).is_ok());
    }
    #[test]
    fn a_retired_member_of_an_uncertified_owner_leaves_a_reused_address_a_fresh_label() {
        let _globals = crate::event_feed_observer::test_globals();
        const REASON: &str = "decision result reached an uncertified owner";
        let memory = Fixture::new([11, 0, 19, 0, 1]);
        let mut store = Store::new(None).unwrap();
        store.prepare_capacity(64).unwrap();
        store.prepare_scripts(1024, 0, 0).unwrap();
        store.prepare_decisions(8).unwrap();
        let mut state = replay_data::event_feed_rows::ScriptEventState::default();
        publish(&mut store, &mut state, 7);
        store
            .decision_baseline(
                &memory,
                0,
                1,
                GAME,
                7,
                engine_native_memory::country_identity::Country {
                    pointer: COUNTRY,
                    canonical_slot: 1,
                    id: 1,
                },
            )
            .unwrap();
        publish(&mut store, &mut state, 8);
        let pointer = Fixture::pointer(2);
        let retired = store.decisions.find(pointer).unwrap().id;
        store.reject("synthetic writer refusal");
        assert_eq!(store.decisions.uncertified(pointer).unwrap().id, retired);
        let world = World::new(&memory, 0).unwrap();
        crate::test_support::with_allocations_denied(|| {
            store.decision_retire(&world, &memory, 0, 1, pointer, false, false)
        })
        .unwrap();
        assert!(store.decisions.uncertified(pointer).is_none());
        assert_eq!(
            store.decision_result(
                &world,
                &memory,
                0,
                1,
                pointer,
                (Some(1), Some(30)),
                EventFeedSource::DecisionDurationChanged,
                REASON,
            ),
            Ok(false)
        );
        store
            .decision_selected(
                &world,
                &memory,
                0,
                1,
                COUNTRY,
                pointer,
                DecisionChange::new(Operation::Store, rows::REENABLE),
                false,
                (None, None),
            )
            .unwrap();
        let reused = store.decisions.uncertified(pointer).unwrap().id;
        assert_ne!(reused, retired);
        assert_eq!(
            store.decision_result(
                &world,
                &memory,
                0,
                1,
                pointer,
                (Some(1), Some(30)),
                EventFeedSource::DecisionDurationChanged,
                REASON,
            ),
            Ok(true)
        );
        let row = publish(&mut store, &mut state, 9);
        let members: Vec<_> = row
            .row()
            .events
            .iter()
            .filter_map(|event| {
                let script = event.script.unwrap();
                let Some(EventFeedScriptChange::Decision(change)) = script.change else {
                    return None;
                };
                Some((change.operation, script.instance))
            })
            .collect();
        assert_eq!(
            members,
            [
                (Operation::Store, Some(reused)),
                (Operation::Refused, None),
                (Operation::Update, Some(reused)),
            ]
        );
    }

    #[test]
    fn an_uncertified_owners_member_follows_its_transfer_into_its_owner_local_result() {
        let _globals = crate::event_feed_observer::test_globals();
        let memory = Fixture::new([11, 0, 19, 0, 1]);
        let mut store = Store::new(None).unwrap();
        store.prepare_capacity(64).unwrap();
        store.prepare_scripts(1024, 0, 0).unwrap();
        store.prepare_decisions(8).unwrap();
        let mut state = replay_data::event_feed_rows::ScriptEventState::default();
        publish(&mut store, &mut state, 7);
        store.decisions.reset(1);
        let world = World::new(&memory, 0).unwrap();
        let pointer = Fixture::pointer(2);
        for container in [rows::REENABLE, rows::REMOVAL] {
            store
                .decision_selected(
                    &world,
                    &memory,
                    0,
                    1,
                    COUNTRY,
                    pointer,
                    DecisionChange::new(Operation::Store, container),
                    false,
                    (None, None),
                )
                .unwrap();
        }
        assert_eq!(
            store.decisions.uncertified(pointer).unwrap().container,
            rows::REMOVAL
        );
        assert_eq!(
            store.decision_result(
                &world,
                &memory,
                0,
                1,
                pointer,
                (None, Some(30)),
                EventFeedSource::DecisionDurationChanged,
                "decision result reached an uncertified owner",
            ),
            Ok(true)
        );
        let row = publish(&mut store, &mut state, 8);
        let occurrence = row.row().events.last().unwrap().script.unwrap();
        let Some(EventFeedScriptChange::Decision(change)) = occurrence.change else {
            panic!("expected a retained decision operation");
        };
        assert_eq!(change.operation, Operation::Update);
        assert_eq!(change.container, rows::REMOVAL);
    }
    fn owner_refused(
        row: &replay_data::event_feed_rows::EventFeedHistoryRow,
    ) -> Option<replay_data::event_feed_rows::EventFeedCountry> {
        row.events.iter().find_map(|event| {
            matches!(
                event.script?.change?,
                EventFeedScriptChange::Decision(change) if change.operation == Operation::Refused
            )
            .then_some(event.actor)
        })
    }

    #[test]
    fn reenable_iteration_erasure_requires_the_active_owners_single_tail_slot() {
        let _globals = crate::event_feed_observer::test_globals();
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

    mod detours {
        use super::super::*;
        use crate::event_feed_observer::{writer_cut, writer_cut_holds};
        use crate::script_event_observer::staging::{Published, open_frames, take_published};
        use engine_decisions::{
            cdecision_cooldown, ctargeted_decision, ctimed_decision, decision_runtime,
        };
        use std::cell::RefCell;

        struct Object(*mut [u8; 0x100]);
        impl Object {
            fn new() -> Self {
                Self(Box::into_raw(Box::new([0; 0x100])))
            }
            fn at(&self) -> usize {
                self.0 as usize
            }
            fn q(&mut self, offset: usize, value: usize) {
                unsafe { ((self.at() + offset) as *mut u64).write_unaligned(value as u64) }
            }
            fn d(&mut self, offset: usize, value: i32) {
                unsafe { store_i32(self.at() + offset, value) }
            }
            fn b(&mut self, offset: usize, value: u8) {
                unsafe { store_u8(self.at() + offset, value) }
            }
        }
        impl Drop for Object {
            fn drop(&mut self) {
                drop(unsafe { Box::from_raw(self.0) });
            }
        }
        unsafe fn store_i32(at: usize, value: i32) {
            unsafe { (at as *mut i32).write_unaligned(value) }
        }
        unsafe fn store_u8(at: usize, value: u8) {
            unsafe { (at as *mut u8).write(value) }
        }
        fn registers() -> crate::hook::Registers {
            unsafe { std::mem::zeroed() }
        }
        std::thread_local! {
            static PLAN: RefCell<Vec<(usize, i32)>> = const { RefCell::new(Vec::new()) };
        }
        fn plan(steps: &[(usize, i32)]) {
            PLAN.with_borrow_mut(|plan| *plan = steps.to_vec());
        }
        fn steps() -> Vec<(usize, i32)> {
            PLAN.with_borrow(Clone::clone)
        }
        fn assert_inside(before: u64) {
            assert!(writer_cut().is_none());
            assert!(!writer_cut_holds(before));
        }
        fn assert_after() {
            assert!(writer_cut().is_some());
            assert_eq!(open_frames(), 0);
        }
        static CUT: AtomicUsize = AtomicUsize::new(0);
        fn env() -> crate::event_feed_observer::StagingTest {
            crate::event_feed_observer::staging_test(64)
        }
        fn begin() {
            let _ = take_published();
            CUT.store(writer_cut().unwrap() as usize, Ordering::SeqCst);
        }
        fn cut() -> u64 {
            CUT.load(Ordering::SeqCst) as u64
        }

        fn timed(days: i32) -> Object {
            let mut object = Object::new();
            object.q(0, ctimed_decision::VTABLE_RVA.0);
            object.q(ctimed_decision::DEFINITION, 0x7000);
            object.d(ctimed_decision::DAYS, days);
            object.d(ctimed_decision::STATE, 2);
            object
        }

        #[test]
        fn a_duration_add_publishes_the_stored_days_after_it_returns() {
            let _env = env();
            unsafe extern "system-unwind" fn add(instance: usize, delta: usize, _: usize) -> usize {
                assert_inside(cut());
                let at = instance + ctimed_decision::DAYS;
                let days = unsafe { (at as *const i32).read_unaligned() };
                unsafe { store_i32(at, days.wrapping_add(delta as i32)) };
                0
            }
            unsafe extern "system-unwind" fn add_cooldown(
                instance: usize,
                delta: usize,
                _: usize,
            ) -> usize {
                assert_inside(cut());
                let at = instance + cdecision_cooldown::DAYS;
                let days = unsafe { (at as *const i32).read_unaligned() };
                unsafe { store_i32(at, days.wrapping_add(delta as i32)) };
                0
            }
            let mission = timed(10);
            CALL[EXTEND_MISSION_TIMEOUT].store(add as *const () as usize, Ordering::SeqCst);
            begin();
            unsafe { extend_mission_timeout(mission.at(), 5, 0x9000) };
            assert_after();
            assert_eq!(
                take_published(),
                [Published::Duration {
                    instance: mission.at(),
                    days: 15
                }]
            );
            let mut timer = Object::new();
            timer.q(0, cdecision_cooldown::VTABLE_RVA.0);
            timer.q(cdecision_cooldown::DEFINITION, 0x7000);
            timer.d(cdecision_cooldown::DAYS, 20);
            CALL[EXTEND_DECISION_REMOVAL]
                .store(add_cooldown as *const () as usize, Ordering::SeqCst);
            begin();
            unsafe { extend_decision_removal(timer.at(), (-3_i32) as u32 as usize, 0x9000) };
            assert_after();
            assert_eq!(
                take_published(),
                [Published::Duration {
                    instance: timer.at(),
                    days: 17
                }]
            );
        }

        #[test]
        fn reached_reactivations_publish_their_committed_stores_in_native_order() {
            let _env = env();
            unsafe extern "system-unwind" fn activate(status: usize, _: usize, _: usize) -> usize {
                assert_inside(cut());
                for (instance, days) in steps() {
                    let mut registers = registers();
                    registers.rbp = status;
                    registers.rsi = 0x7000;
                    registers.rdi = instance;
                    unsafe { super::super::super::tap(&registers, 5) };
                    unsafe {
                        store_i32(instance + ctimed_decision::DAYS, days);
                        store_i32(
                            instance + ctimed_decision::STATE,
                            ctimed_decision::STATE_ACTIVE,
                        );
                    }
                }
                0x1234
            }
            let (first, second) = (timed(1), timed(2));
            ORIGINAL[DETOURS].store(activate as *const () as usize, Ordering::SeqCst);
            plan(&[(second.at(), 40), (first.at(), 30)]);
            let status = Object::new();
            begin();
            assert_eq!(unsafe { auto_activation(status.at(), 0, 0) }, 0x1234);
            assert_after();
            assert_eq!(
                take_published(),
                [
                    Published::Activation {
                        instance: second.at(),
                        state: ctimed_decision::STATE_ACTIVE,
                        days: 40
                    },
                    Published::Activation {
                        instance: first.at(),
                        state: ctimed_decision::STATE_ACTIVE,
                        days: 30
                    },
                ]
            );
            plan(&[]);
            begin();
            unsafe { auto_activation(status.at(), 0, 0) };
            assert!(take_published().is_empty());
        }

        #[test]
        fn a_timed_selection_publishes_its_stored_days_after_returning() {
            let _env = env();
            unsafe extern "system-unwind" fn select(_: usize, _: usize, _: usize) -> usize {
                assert_inside(cut());
                for (instance, days) in steps() {
                    let mut registers = registers();
                    registers.rdi = instance;
                    unsafe { super::super::tap(&registers, 15) };
                    unsafe { store_i32(instance + ctimed_decision::DAYS, days) };
                }
                0
            }
            let mission = timed(0);
            ORIGINAL[DETOURS + 2].store(select as *const () as usize, Ordering::SeqCst);
            plan(&[(mission.at(), 9)]);
            let status = Object::new();
            begin();
            unsafe { timed_selection(status.at(), 0x7000, 0) };
            assert_after();
            assert_eq!(
                take_published(),
                [Published::Selection {
                    instance: mission.at(),
                    days: 9
                }]
            );
        }

        #[test]
        fn ignore_publication_is_decided_from_the_staged_pre_store_flag() {
            let _env = env();
            unsafe extern "system-unwind" fn command(_: usize, _: usize, _: usize) -> usize {
                assert_inside(cut());
                for (instance, _) in steps() {
                    let mut registers = registers();
                    registers.rcx = instance;
                    unsafe { super::super::tap(&registers, 17) };
                    unsafe { store_u8(instance + ctargeted_decision::IGNORE, 1) };
                }
                0
            }
            let (mut fresh, mut ignored) = (Object::new(), Object::new());
            fresh.b(ctargeted_decision::IGNORE, 0);
            ignored.b(ctargeted_decision::IGNORE, 1);
            ORIGINAL[DETOURS + 3].store(command as *const () as usize, Ordering::SeqCst);
            plan(&[(fresh.at(), 0), (ignored.at(), 0)]);
            let status = Object::new();
            begin();
            unsafe { targeted_ignore_command(status.at(), 0, 0) };
            assert_after();
            assert_eq!(
                take_published(),
                [
                    Published::Ignore {
                        instance: fresh.at(),
                        was: false,
                        now: true
                    },
                    Published::Ignore {
                        instance: ignored.at(),
                        was: true,
                        now: true
                    },
                ]
            );
            assert_eq!(
                crate::event_feed_observer::decisions::committed_flags(Some((false, true)), None),
                Some((Some(true), false))
            );
            assert_eq!(
                crate::event_feed_observer::decisions::committed_flags(Some((true, true)), None),
                None
            );
        }

        #[test]
        fn ignore_all_stages_at_its_admission_call_and_publishes_after_the_command() {
            let _env = env();
            unsafe extern "system-unwind" fn admitted(_: usize, _: usize, _: usize) -> usize {
                1
            }
            unsafe extern "system-unwind" fn command(status: usize, _: usize, _: usize) -> usize {
                assert_inside(cut());
                for (instance, _) in steps() {
                    if unsafe { ignore_selected(status, instance, 1) } as u8 != 0 {
                        unsafe { store_u8(instance + ctargeted_decision::IGNORE, 1) };
                    }
                }
                0
            }
            let mut target = Object::new();
            target.b(ctargeted_decision::IGNORE, 0);
            CALL[11].store(admitted as *const () as usize, Ordering::SeqCst);
            ORIGINAL[DETOURS + 4].store(command as *const () as usize, Ordering::SeqCst);
            plan(&[(target.at(), 0)]);
            let status = Object::new();
            begin();
            unsafe { ignore_all_command(status.at(), 0, 0) };
            assert_after();
            assert_eq!(
                take_published(),
                [Published::Ignore {
                    instance: target.at(),
                    was: false,
                    now: true
                }]
            );
        }

        #[test]
        fn removal_requests_publish_once_the_shared_store_completed() {
            let _env = env();
            unsafe extern "system-unwind" fn lookup(_: usize, _: usize, _: usize) -> usize {
                steps()[0].0
            }
            unsafe extern "system-unwind" fn effect(status: usize, _: usize, _: usize) -> usize {
                assert_inside(cut());
                let instance = unsafe { request_timed(status, 0x7000, 0) };
                if instance != 0 {
                    unsafe { store_u8(instance + decision_runtime::REMOVE_REQUESTED, 1) };
                }
                0x55
            }
            CALL[12].store(lookup as *const () as usize, Ordering::SeqCst);
            ORIGINAL[DETOURS + 5].store(effect as *const () as usize, Ordering::SeqCst);
            for was in [0, 1] {
                let mut target = Object::new();
                target.b(decision_runtime::REMOVE_REQUESTED, was);
                plan(&[(target.at(), 0)]);
                let effect = Object::new();
                begin();
                assert_eq!(unsafe { removal_request(effect.at(), 0, 0) }, 0x55);
                assert_after();
                assert_eq!(
                    take_published(),
                    [Published::Removal {
                        instance: target.at(),
                        was: was == 1,
                        now: 1
                    }]
                );
                assert_eq!(
                    crate::event_feed_observer::decisions::committed_flags(None, Some(was == 1))
                        .is_some(),
                    was == 0
                );
            }
        }

        #[test]
        fn a_fired_retirement_publishes_only_its_staged_match_branch() {
            let _env = env();
            unsafe extern "system-unwind" fn retire(
                state: usize,
                definition: usize,
                _: usize,
            ) -> usize {
                assert_inside(cut());
                if !steps().is_empty() {
                    let mut registers = registers();
                    registers.rsi = state;
                    registers.rdi = definition;
                    unsafe { super::super::super::tap(&registers, 8) };
                }
                0
            }
            CALL[REPLACE_EVENT_RETIREMENT].store(retire as *const () as usize, Ordering::SeqCst);
            let (world, definition) = (Object::new(), Object::new());
            plan(&[(0, 0)]);
            begin();
            unsafe { replace_event_retirement(world.at(), definition.at(), 0) };
            assert_after();
            assert_eq!(
                take_published(),
                [Published::Fired {
                    definition: definition.at()
                }]
            );
            plan(&[]);
            begin();
            unsafe { replace_event_retirement(world.at(), definition.at(), 0) };
            assert_after();
            assert!(take_published().is_empty());
        }

        #[test]
        fn candidate_finalization_publishes_its_marker_pair_after_every_clear() {
            let _env = env();
            unsafe extern "system-unwind" fn finalize(_: usize, _: usize, _: usize) -> usize {
                assert_inside(cut());
                assert!(take_published().is_empty());
                0
            }
            ORIGINAL[DETOURS + 6].store(finalize as *const () as usize, Ordering::SeqCst);
            let status = Object::new();
            begin();
            unsafe { candidate_finalization(status.at(), 0, 0) };
            assert_after();
            assert_eq!(
                take_published(),
                [Published::Candidates {
                    status: status.at()
                }]
            );
        }

        #[test]
        fn a_latch_outside_its_detour_stages_and_publishes_nothing() {
            let _env = env();
            let mission = timed(0);
            let mut registers = registers();
            registers.rdi = mission.at();
            begin();
            unsafe { super::super::tap(&registers, 15) };
            assert_eq!(open_frames(), 0);
            assert!(take_published().is_empty());
        }

        #[test]
        fn a_detour_whose_witnesses_a_boundary_reset_refuses_at_publication() {
            use crate::script_event_observer::staging::{
                Detour, Unit, WITNESS_ABANDONED, Witness, stage,
            };
            let env = env();
            env.prepare(1);
            let (_, rejected) = env.failure();
            let detour = Detour::enter(Unit::TimedSelection);
            stage(Unit::TimedSelection, Witness::Selected { instance: 1 });
            assert_eq!(env.staged_live(), 1);
            env.prepare(2);
            assert_eq!(env.failure(), (Some(WITNESS_ABANDONED), rejected + 1));
            let mut published = 0;
            detour.publish(|_| published += 1);
            assert_eq!(published, 0);
            assert_eq!(env.failure().1, rejected + 2);
            assert_eq!(open_frames(), 0);
        }

        #[test]
        fn a_latch_of_the_wrong_unit_inside_a_detour_refuses_and_stages_nothing() {
            unsafe extern "system-unwind" fn select(_: usize, _: usize, _: usize) -> usize {
                let mut registers = registers();
                registers.rcx = steps()[0].0;
                unsafe { super::super::tap(&registers, 17) };
                0
            }
            let _env = env();
            let target = Object::new();
            ORIGINAL[DETOURS + 2].store(select as *const () as usize, Ordering::SeqCst);
            plan(&[(target.at(), 0)]);
            let status = Object::new();
            begin();
            unsafe { timed_selection(status.at(), 0, 0) };
            assert_after();
            assert!(take_published().is_empty());
        }
    }
}
