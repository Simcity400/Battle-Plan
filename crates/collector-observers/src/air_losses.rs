//! Aircraft-loss writer context, outside native execution and Rust unwind frames.
use crate::{
    event_ledger::{Event, EventLedger},
    hook::Hook,
};
type Clock = engine_native::cgame_state::Clock<u64>;
use engine_air_history::air_loss_sites as native;
use engine_native::Memory;
use std::{
    ops::Range,
    sync::{
        OnceLock,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Entry {
    pub id: u64,
    pub game_hour: i32,
    pub country: u32,
    pub region: u32,
    pub native_cause: u32,
    pub family_base: u32,
    pub credited_enemy: bool,
    pub amount_raw: i64,
    pub opponent: Option<u32>,
    pub daily_slot: u32,
}
impl Event for Entry {
    fn set_id(&mut self, id: u64) {
        self.id = id;
    }
}
static LEDGER: EventLedger<Entry> = EventLedger::new();
static BASE: AtomicUsize = AtomicUsize::new(0);
static ENABLED: AtomicBool = AtomicBool::new(false);
static ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static INSTALLED: AtomicBool = AtomicBool::new(false);
static UNHOOKED: OnceLock<&'static str> = OnceLock::new();
#[cfg(any(test, feature = "test-support"))]
static TAPPED_TAGS: [AtomicUsize; 2] = [AtomicUsize::new(0), AtomicUsize::new(0)];
static WINGS: AtomicUsize = AtomicUsize::new(0);
/// Calls [`air_loss_dispatch`] passed straight to the native writer: those
/// the writer makes no write for, with nothing observed or counted in Rust.
static FAST_ZERO_WRITES: AtomicU64 = AtomicU64::new(0);
/// Calls whose gathered operands were withheld ([`native::gather`]), so the
/// serial reads ran once: a fault, a refusal, or a history the call creates.
static SERIAL_FALLBACKS: AtomicUsize = AtomicUsize::new(0);
#[cfg(any(test, feature = "test-support"))]
static FORCE_SERIAL: AtomicBool = AtomicBool::new(false);
fn gather_admitted() -> bool {
    #[cfg(any(test, feature = "test-support"))]
    if FORCE_SERIAL.load(Ordering::SeqCst) {
        return false;
    }
    true
}
pub fn active() -> usize {
    LEDGER.active.load(Ordering::SeqCst)
}
pub fn restore_sequence(previous: Option<u64>) -> Result<(), &'static str> {
    if ENABLED.load(Ordering::Acquire) {
        return Err("air loss sequence restoration requires disabled hooks");
    }
    LEDGER.restore_sequence(previous)
}
pub fn mark_stranded() {
    #[cfg(test)]
    if !tests::owns_observation() {
        return;
    }
    LEDGER.mark_stranded();
}
pub fn capacity() -> usize {
    LEDGER.demand()
}
pub fn presize(events: usize) {
    #[cfg(test)]
    if !tests::owns_observation() {
        return;
    }
    LEDGER.prepare(events);
}
pub fn lost_drain() {
    #[cfg(test)]
    if !tests::owns_observation() {
        return;
    }
    LEDGER.lost_drain();
}
pub fn lost_output(events: usize) {
    #[cfg(test)]
    if !tests::owns_observation() {
        return;
    }
    LEDGER.lost_output(events);
}
pub fn prepare(memory: &impl Memory, base: usize, countries: usize) {
    #[cfg(test)]
    if !tests::owns_observation() {
        return;
    }
    LEDGER.prepare(0);
    let game = match collector_extended_support::demand::demand_air_game_state(memory, base) {
        Ok(Some(game)) => game,
        Ok(None) => return,
        Err(reason) => {
            LEDGER.refuse_preparation(reason);
            return;
        }
    };
    let wings = match native::wing_count(memory, base, game) {
        Ok(wings) => wings,
        Err(reason) => {
            LEDGER.refuse_preparation(reason);
            return;
        }
    };
    let Some(population) = countries
        .checked_add(wings)
        .and_then(|n| n.checked_mul(native::SITES.len()))
    else {
        LEDGER.refuse_preparation("air loss preparation population size overflow");
        return;
    };
    WINGS.store(wings, Ordering::Release);
    LEDGER.prepare(population);
}
pub fn boundary(clock: Option<Clock>) {
    #[cfg(test)]
    if !tests::owns_observation() {
        return;
    }
    LEDGER.boundary(clock);
}
#[cfg(any(test, feature = "test-support"))]
pub fn drain(output: &mut Vec<Entry>) -> Result<usize, &'static str> {
    drain_admitted()?;
    LEDGER.drain(output)
}
pub fn drain_growing(output: &mut Vec<Entry>) -> Result<usize, &'static str> {
    drain_admitted()?;
    LEDGER.drain_growing(output)
}
fn drain_admitted() -> Result<(), &'static str> {
    #[cfg(test)]
    if !tests::owns_observation() {
        return Err("air loss hooks unavailable (see status unhooked)");
    }
    if UNHOOKED.get().is_some() || !ENABLED.load(Ordering::SeqCst) {
        return Err("air loss hooks unavailable (see status unhooked)");
    }
    Ok(())
}
struct Observation {
    generation: u64,
    snapshot: native::Snapshot,
    game: usize,
    hour: i32,
    token: usize,
    count: i32,
    opponent: Option<u32>,
}
#[repr(C)]
struct CallContext {
    tags: [usize; 2],
    observation: Option<Observation>,
}
// Original arguments/returns retain their reviewed offsets. Context starts A0;
// RSI/RDI are copied before Rust, never modified. .seh_stackalloc covers it all.
const CONTEXT_OFFSET: usize = 0xa0;
const FRAME_BYTES: usize =
    CONTEXT_OFFSET + std::mem::size_of::<CallContext>().div_ceil(16) * 16 + 8;
core::arch::global_asm!(
    ".text", ".globl air_loss_entry", ".seh_proc air_loss_entry",
    "air_loss_entry:", "sub rsp, {frame}", ".seh_stackalloc {frame}", ".seh_endprologue",
    "mov [rsp + 0x40], rcx", "mov [rsp + 0x48], rdx",
    "mov [rsp + 0x50], r8", "mov [rsp + 0x58], r9",
    "mov rax, [rsp + {arg5}]", "mov [rsp + 0x60], rax",
    "mov rax, [rsp + {arg6}]", "mov [rsp + 0x68], rax",
    "mov rax, [rsp + {arg7}]", "mov [rsp + 0x70], rax",
    "mov [rsp + 0x20], r9", "mov rax, [rsp + 0x60]", "mov [rsp + 0x28], rax",
    "mov rax, [rsp + 0x68]", "mov [rsp + 0x30], rax",
    "mov rax, [rsp + 0x70]", "mov [rsp + 0x38], rax",
    "mov r9, r8", "mov r8, rdx", "mov rdx, rcx", "lea rcx, [rsp + {context}]",
    "mov [rsp + {context}], rsi", "mov [rsp + {tag2}], rdi",
    "call {pre}", "mov [rsp + 0x78], rax",
    "mov rcx, [rsp + 0x40]", "mov rdx, [rsp + 0x48]",
    "mov r8, [rsp + 0x50]", "mov r9, [rsp + 0x58]",
    "mov rax, [rsp + 0x60]", "mov [rsp + 0x20], rax",
    "mov rax, [rsp + 0x68]", "mov [rsp + 0x28], rax",
    "mov rax, [rsp + 0x70]", "mov [rsp + 0x30], rax",
    "call qword ptr [rsp + 0x78]",
    ".globl air_loss_original_return", "air_loss_original_return:",
    "mov [rsp + 0x80], rax", "movdqu [rsp + 0x88], xmm0",
    "lea rcx, [rsp + {context}]", "call {post}",
    "mov rax, [rsp + 0x80]", "movdqu xmm0, [rsp + 0x88]",
    "add rsp, {frame}", "ret", ".seh_endproc",
    frame = const FRAME_BYTES, context = const CONTEXT_OFFSET, tag2 = const CONTEXT_OFFSET + 8,
    arg5 = const FRAME_BYTES + 0x28, arg6 = const FRAME_BYTES + 0x30,
    arg7 = const FRAME_BYTES + 0x38, pre = sym pre, post = sym post,
);
// The hooked CALLs' target: a leaf that changes no register the writer reads
// and no stack slot, so it needs no unwind metadata. Air loss bookkeeping
// Writes nothing when its region  (fifth argument) is null, or for a credited
// Loss  (fourth argument's low byte 1) of cause 3 (seventh). Both gates skip
// the regional block and history write, through the epilogue. `pre` records
// nothing for exactly those calls
// Either  (a zero write), so they go straight to the writer: it returns to
// the native caller, and no collector frame is on the stack while it runs.
// Every other call enters `air_loss_entry` with the same registers and stack.
core::arch::global_asm!(
    ".text", ".globl air_loss_dispatch", "air_loss_dispatch:",
    "cmp qword ptr [rsp + 0x28], 0", "je 2f",
    "cmp r9b, 1", "jne 3f",
    "cmp dword ptr [rsp + 0x38], 3", "jne 3f",
    "2:", "lock inc qword ptr [rip + {fast}]", "jmp qword ptr [rip + {original}]",
    "3:", "jmp {entry}",
    fast = sym FAST_ZERO_WRITES, original = sym ORIGINAL, entry = sym air_loss_entry,
);
unsafe extern "system" {
    fn air_loss_dispatch();
    fn air_loss_entry();
    #[cfg(test)]
    fn air_loss_original_return();
}

#[allow(clippy::too_many_arguments)]
unsafe extern "system" fn pre(
    context: *mut CallContext,
    receiver: usize,
    wing: usize,
    count: usize,
    enemy: usize,
    region: usize,
    _selection: usize,
    cause: usize,
) -> usize {
    let _timing = crate::hook_timing::enter(crate::hook_timing::Family::AirLosses);
    LEDGER.enter();
    let tags = unsafe { (*context).tags };
    #[cfg(any(test, feature = "test-support"))]
    for (captured, value) in TAPPED_TAGS.iter().zip(tags) {
        captured.store(value, Ordering::Relaxed);
    }
    unsafe {
        context.write(CallContext {
            tags,
            observation: None,
        });
    }
    let original = ORIGINAL.load(Ordering::Acquire);
    if ENABLED.load(Ordering::SeqCst) && region != 0 && !(enemy as u8 == 1 && cause as i32 == 3) {
        LEDGER.attempt();
        if LEDGER.poisoned.load(Ordering::SeqCst) || crate::allocation::failed() {
            LEDGER.lost_events_unknown.fetch_add(1, Ordering::Relaxed);
        } else {
            let memory = unsafe { engine_native::memory::LiveMemory::new() };
            let base = BASE.load(Ordering::Relaxed);
            let mut token = None;
            let result = (|| {
                let generation = crate::world_lifetime::generation();
                if crate::world_lifetime::invalidating() {
                    return Err("air loss native world awaits hourly boundary");
                }
                // The same operands in six gathered copies and the country
                // resolution; withheld, the serial reads below run once and
                // refuse exactly where they always did.
                let gathered = base
                    .checked_add(engine_native::cgame_state::SINGLETON_RVA)
                    .filter(|_| gather_admitted())
                    .and_then(|singleton| {
                        native::gather(
                            &memory,
                            base,
                            singleton,
                            receiver,
                            wing,
                            region,
                            enemy as u8,
                            cause as i32,
                            tags,
                        )
                    });
                if let Some(gathered) = gathered {
                    let claimed = LEDGER.claim((receiver, gathered.region as usize))?;
                    token = Some(claimed);
                    return Ok(Observation {
                        generation,
                        snapshot: gathered.snapshot,
                        game: gathered.game,
                        hour: gathered.hour,
                        token: claimed,
                        count: count as i32,
                        opponent: gathered.opponent,
                    });
                }
                SERIAL_FALLBACKS.fetch_add(1, Ordering::Relaxed);
                let region_id = engine_native::cstrategic_region::id(&memory, base, region)?
                    .ok_or("air loss region unexpectedly null")?;
                let claimed = LEDGER.claim((receiver, region_id as usize))?;
                token = Some(claimed);
                let game = base
                    .checked_add(engine_native::cgame_state::SINGLETON_RVA)
                    .and_then(|a| engine_native::read::<u64>(&memory, a))
                    .and_then(|p| usize::try_from(p).ok())
                    .filter(|p| *p != 0)
                    .ok_or("air loss game root unreadable")?;
                let hour = game
                    .checked_add(engine_native::cgame_state::CURRENT_HOUR)
                    .and_then(|a| engine_native::read::<i32>(&memory, a))
                    .ok_or("air loss native hour unreadable")?;
                let family = native::selected_family(&memory, base, wing, enemy as u8)?;
                let snapshot = native::before(
                    &memory,
                    base,
                    game,
                    receiver,
                    region_id,
                    enemy as u8,
                    cause as i32,
                    family,
                )?;
                let opponent = native::opponent(&memory, tags, enemy as u8)?;
                Ok(Observation {
                    generation,
                    snapshot,
                    game,
                    hour,
                    token: claimed,
                    count: count as i32,
                    opponent,
                })
            })();
            match result {
                Ok(observation) => unsafe {
                    (*context).observation = Some(observation);
                },
                Err(reason) => {
                    if let Some(token) = token {
                        let _ = LEDGER.complete(token, Ok(None));
                    }
                    LEDGER.lost_events_unknown.fetch_add(1, Ordering::Relaxed);
                    LEDGER.refuse(reason);
                }
            }
        }
    } else {
        LEDGER.zero_writes.fetch_add(1, Ordering::Relaxed);
    }
    original
}
unsafe extern "system" fn post(context: *mut CallContext) {
    let _timing = crate::hook_timing::enter_return(crate::hook_timing::Family::AirLosses);
    let context = unsafe { context.read() };
    if let Some(observation) = context.observation {
        let result = (|| {
            if observation.generation != crate::world_lifetime::generation()
                || crate::world_lifetime::invalidating()
            {
                return Err("air loss native world changed during original");
            }
            let memory = unsafe { engine_native::memory::LiveMemory::new() };
            let base = BASE.load(Ordering::Relaxed);
            if base
                .checked_add(engine_native::cgame_state::SINGLETON_RVA)
                .and_then(|a| engine_native::read::<u64>(&memory, a))
                != Some(observation.game as u64)
            {
                return Err("air loss game root changed during native call");
            }
            let change = native::after(&memory, base, observation.game, &observation.snapshot)?;
            let amount =
                validate_delta(&change, observation.snapshot.family_base, observation.count)?;
            Ok((amount != 0).then_some(Entry {
                id: 0,
                game_hour: observation.hour,
                country: change.country,
                region: change.region,
                native_cause: change.cause as u32,
                family_base: observation.snapshot.family_base,
                credited_enemy: change.enemy != 0,
                amount_raw: amount,
                opponent: observation.opponent,
                daily_slot: change.daily_slot,
            }))
        })();
        let known = result.as_ref().ok().is_some_and(Option::is_some);
        if let Err(reason) = LEDGER.complete(observation.token, result) {
            if known {
                LEDGER.lost_events.fetch_add(1, Ordering::Relaxed);
            } else {
                LEDGER.lost_events_unknown.fetch_add(1, Ordering::Relaxed);
            }
            LEDGER.refuse(reason);
        }
    }
    LEDGER.leave();
}
fn validate_delta(change: &native::Change, family: u32, count: i32) -> Result<i64, &'static str> {
    native::selected_indices(family, change.cause)?;
    let amount = i64::from(count) * 100000;
    let expected = if change.cause == 0 {
        amount * 2
    } else {
        amount
    };
    if change.fields != [expected; 2] {
        return Err("air loss counters saturated or changed outside selected fields");
    }
    Ok(amount)
}
pub struct Hooks {
    hooks: Vec<Hook>,
}
impl Hooks {
    /// Only the two supported loss CALLs are patched. Their native
    /// continuation frames and unwind metadata remain untouched.
    ///
    /// # Safety
    /// The memory source and base must identify the authenticated executable in
    /// this process. The caller owns this family exclusively and must restore
    /// its hooks and quiesce native calls before releasing code or unloading.
    pub unsafe fn new(memory: &impl Memory, base: usize) -> Self {
        BASE.store(base, Ordering::Relaxed);
        ORIGINAL.store(base + native::WRITER_RVA, Ordering::Release);
        let install = || -> Result<Vec<Hook>, &'static str> {
            for site in native::SITES {
                if !crate::hook::installation::sentinel(memory, base + site.rva, site.sentinel) {
                    return Err("air loss call sentinel differs or unreadable");
                }
            }
            let mut hooks = Vec::new();
            for site in native::SITES {
                let hook = unsafe {
                    Hook::new_call(
                        base + site.rva,
                        air_loss_dispatch as *const () as usize,
                        site.sentinel,
                    )
                }
                .map_err(|_| "air loss hook construction failed")?;
                hooks.push(hook);
            }
            Ok(hooks)
        };
        match install() {
            Ok(hooks) => {
                INSTALLED.store(true, Ordering::Release);
                Self { hooks }
            }
            Err(reason) => {
                crate::hook::installation::refused(
                    crate::hook::installation::Kind::Construction,
                    None,
                    reason,
                );
                let _ = UNHOOKED.set(reason);
                Self { hooks: Vec::new() }
            }
        }
    }
    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, &'static str> {
        if !enabled {
            ENABLED.store(false, Ordering::SeqCst);
        }
        // One census and suspension for the whole family, shared with the
        // other families in a runtime round (`hook::shared`): a thread at any
        // of its sites, or in a tap, holds every site back until the next
        // attempt.
        if !Hook::set_family_enabled_shared(&mut self.hooks, enabled)
            .map_err(|_| "air loss hook state change failed")?
        {
            return Ok(false);
        }
        ENABLED.store(
            enabled && self.hooks.len() == native::SITES.len(),
            Ordering::SeqCst,
        );
        Ok(true)
    }
    pub fn executable_ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.hooks.iter().flat_map(Hook::executable_ranges)
    }
}
pub fn status() -> impl serde::Serialize {
    crate::status::status!({
        "installed": INSTALLED.load(Ordering::Acquire), "enabled": ENABLED.load(Ordering::SeqCst),
        "active": active(), "stranded": LEDGER.stranded.load(Ordering::SeqCst), "unhooked": UNHOOKED.get(),
        "branch_unavailable": LEDGER.poisoned.load(Ordering::SeqCst),
        // The counters wrap as their atomics do; so do their sums.
        "calls": LEDGER.calls.load(Ordering::Relaxed).wrapping_add(FAST_ZERO_WRITES.load(Ordering::Relaxed)),
        "events": LEDGER.events.load(Ordering::Relaxed),
        "zero_writes": LEDGER.zero_writes.load(Ordering::Relaxed).wrapping_add(FAST_ZERO_WRITES.load(Ordering::Relaxed)),
        "fast_zero_writes": FAST_ZERO_WRITES.load(Ordering::Relaxed),
        "overflow": LEDGER.overflow.load(Ordering::Relaxed), "refusals": LEDGER.refusals.load(Ordering::Relaxed),
        "lost_events": LEDGER.lost_events.load(Ordering::Relaxed), "lost_events_unknown": LEDGER.lost_events_unknown.load(Ordering::Relaxed),
        "lost_drains": LEDGER.lost_drains.load(Ordering::Relaxed), "overlaps": LEDGER.overlaps.load(Ordering::Relaxed),
        "drains": LEDGER.drains.load(Ordering::Relaxed), "handed_over": LEDGER.handed_over.load(Ordering::Relaxed),
        "event_capacity": LEDGER.capacity.load(Ordering::Acquire), "event_required": capacity(), "pending_claim_capacity": 1,
        "prepare_failures": LEDGER.prepare_failures.load(Ordering::Relaxed), "wing_count": WINGS.load(Ordering::Acquire),
        "serial_fallbacks": SERIAL_FALLBACKS.load(Ordering::Relaxed),
        "prepare_last_refusal": LEDGER.prepare_last_refusal(),
        "partial_intervals": LEDGER.partial_intervals.load(Ordering::Relaxed), "partial_events": LEDGER.partial_events.load(Ordering::Relaxed),
        "hourly_peak_events": LEDGER.hourly_peak_events.load(Ordering::Relaxed), "hourly_peak_capacity": LEDGER.hourly_peak_capacity.load(Ordering::Relaxed),
        "scope": "since attach; first/recovery partial intervals withheld; calls and zero_writes include fast_zero_writes, the calls the writer makes no write for, which go straight to it without entering the observer, so hook_timing.families.air_losses counts and times only the other calls"
    })
}
#[cfg(test)]
mod tests;
