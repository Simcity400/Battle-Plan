//! What caused the event feed's events: the native operations running on a
//! thread when one happens ([`engine::event_feed_sites::provenance`]).
//!
//! Each operation sets a thread-local [`Context`] for as long as its original
//! runs and restores the enclosing one when it returns or unwinds, so the
//! native stack itself is the operation stack and no depth is chosen. An
//! event inside an operation takes its cause from the context ([`inside`]):
//! the declarations and joins of the war link CALL, the `on_annex` dispatch
//! and a conference's start. An operation that is itself an event (a war
//! relation ending, a release, a state's owner changing) names its caller as
//! its route and the running operation as the one it ran inside
//! ([`performed`]).
//!
//! The event feed family owns the eight entry detours ([`hooks`], enabled,
//! restored and quiesced with its CALL hooks). The politics family forwards
//! its diplomatic dispatch, civil war start, decision, scripted diplomacy and
//! on_action scopes ([`politics`]) and the territory family its
//! Set owner detour  ([`state_owner_before`]). A caller outside a
//! site's list, or a cause operand that does not read, leaves that fact
//! `None` and is counted in the status; an event whose own operands do not
//! read is refused by the store.

use crate::{
    event_feed_observer::{self, Fact},
    hook::Hook,
};
use engine::{
    Memory,
    event_feed_sites::{
        CountryIdentity,
        provenance::{self as native, Caller},
    },
    memory::LiveMemory,
};
use replay::event_feed_rows::{EventFeedCause, EventFeedCountry, EventFeedRoute, EventFeedSource};
use std::{
    cell::Cell,
    sync::{
        Mutex,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
};

const JOIN_WAR: usize = 0;
const END_WAR: usize = 1;
const CREATE_CONFERENCE: usize = 2;
const START_CONFERENCE: usize = 3;
const APPLY_CONFERENCE: usize = 4;
const ANNEX: usize = 5;
const RELEASE: usize = 6;
const RELEASE_COMMAND: usize = 7;
const ENTRIES: usize = native::ENTRIES.len();

static BASE: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL: [AtomicUsize; ENTRIES] = [const { AtomicUsize::new(0) }; ENTRIES];
/// Entry detours running now.
static ACTIVE: AtomicUsize = AtomicUsize::new(0);
/// Calls from outside their site's caller list.
static UNCLASSIFIED: AtomicU64 = AtomicU64::new(0);
/// Cause operands that did not read, and the first reason.
static UNREAD: AtomicU64 = AtomicU64::new(0);
static FIRST_UNREAD: Mutex<Option<&'static str>> = Mutex::new(None);

fn unread(reason: &'static str) {
    UNREAD.fetch_add(1, Ordering::Relaxed);
    if let Ok(mut first) = FIRST_UNREAD.try_lock() {
        first.get_or_insert(reason);
    }
}

/// The running operation on this thread.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Context {
    route: EventFeedRoute,
    within: EventFeedRoute,
    conference: Option<u64>,
    ally: Option<EventFeedCountry>,
    reason: Option<u32>,
    flags: u8,
}

thread_local! {
    static CONTEXT: Cell<Context> = const { Cell::new(Context {
        route: EventFeedRoute::None,
        within: EventFeedRoute::None,
        conference: None,
        ally: None,
        reason: None,
        flags: 0,
    }) };
}

/// A running operation. Dropping it, on return or unwind, restores the
/// enclosing context.
#[must_use]
pub(crate) struct Scope(Context);

impl Drop for Scope {
    fn drop(&mut self) {
        CONTEXT.set(self.0);
    }
}

/// Replace the running context until the scope drops.
fn set(context: Context) -> Scope {
    Scope(CONTEXT.replace(context))
}

/// Start an operation reached by `route` with its own operands; a
/// conference it names, or the enclosing one, stays the conference of
/// everything inside it.
fn enter(route: EventFeedRoute, conference: Option<u64>, flags: u8) -> Scope {
    let previous = CONTEXT.get();
    set(Context {
        route,
        within: previous.route,
        conference: conference.or(previous.conference),
        ally: None,
        reason: None,
        flags,
    })
}

/// The cause of an event inside the running operation.
pub(crate) fn inside() -> EventFeedCause {
    let context = CONTEXT.get();
    EventFeedCause {
        route: context.route,
        within: context.within,
        conference: context.conference,
        state: None,
        ally: context.ally,
        reason: context.reason,
        flags: context.flags,
    }
}

/// The cause of an event a call by `route` performs.
fn performed(route: EventFeedRoute, state: Option<u32>, flags: u8) -> EventFeedCause {
    let context = CONTEXT.get();
    EventFeedCause {
        route,
        within: context.route,
        conference: context.conference,
        state,
        ally: None,
        reason: None,
        flags,
    }
}

/// The route `callers` names for a call returning to the absolute address
/// `caller`; `None`, counted, outside the list.
fn route(callers: &[Caller], caller: usize) -> EventFeedRoute {
    let return_rva = caller.wrapping_sub(BASE.load(Ordering::Relaxed));
    match native::route(callers, return_rva).and_then(EventFeedRoute::named) {
        Some(route) => route,
        None => {
            UNCLASSIFIED.fetch_add(1, Ordering::Relaxed);
            EventFeedRoute::None
        }
    }
}

/// Bit n set when byte operand n is nonzero.
fn flags(bytes: &[usize]) -> u8 {
    bytes.iter().enumerate().fold(0, |flags, (bit, byte)| {
        flags | (u8::from(*byte as u8 != 0) << bit)
    })
}

pub(crate) fn country(identity: CountryIdentity) -> EventFeedCountry {
    EventFeedCountry {
        reference: identity.reference,
        id: identity.id,
        instance: identity.instance,
    }
}

/// A cause operand read from the current world, `None` (counted) when it
/// does not read.
fn operand<T>(
    observe: impl FnOnce(&native::World<'_, LiveMemory>) -> Result<T, &'static str>,
) -> Option<T> {
    // SAFETY: guarded reads of live memory on the calling thread.
    let memory = unsafe { LiveMemory::new() };
    native::World::new(&memory, BASE.load(Ordering::Relaxed))
        .and_then(|world| observe(&world))
        .map_err(unread)
        .ok()
}

/// The politics scopes that start an event feed operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PoliticsScope {
    Diplomacy,
    CivilWar,
    Decision,
    TargetedDecision,
    DecisionTimeout,
    ScriptedDiplomacy,
    OnAction,
}

/// Politics forward: the politics scope `scope` is about to run for
/// `caller`. The diplomatic dispatch and civil war start
/// civil war start are named by their callers; decisions,
/// scripted diplomacy and on_actions, whose callers the script decides, by
/// themselves. Without the event feed's hooks no operation starts.
pub(crate) fn politics(scope: PoliticsScope, caller: usize) -> Option<Scope> {
    if BASE.load(Ordering::Relaxed) == 0 {
        return None;
    }
    let reached = match scope {
        PoliticsScope::Diplomacy => route(&native::DIPLOMACY_CALLERS, caller),
        PoliticsScope::CivilWar => route(&native::CIVIL_WAR_CALLERS, caller),
        PoliticsScope::Decision => EventFeedRoute::Decision,
        PoliticsScope::TargetedDecision => EventFeedRoute::TargetedDecision,
        PoliticsScope::DecisionTimeout => EventFeedRoute::DecisionTimeout,
        PoliticsScope::ScriptedDiplomacy => EventFeedRoute::ScriptedDiplomacy,
        PoliticsScope::OnAction => EventFeedRoute::OnAction,
    };
    Some(enter(reached, None, 0))
}

/// A set owner call the territory family's detour is running.
pub(crate) struct StateOwner {
    state: usize,
    id: u32,
    owner: u32,
    cause: EventFeedCause,
}

/// Territory forward, before `state owner setter(state, owner, u8,
/// u8)` runs for `caller`: the state's id and current owner.
pub(crate) fn state_owner_before(
    state: usize,
    third: usize,
    fourth: usize,
    caller: usize,
) -> Option<StateOwner> {
    let base = BASE.load(Ordering::Relaxed);
    if base == 0 {
        return None;
    }
    // SAFETY: a guarded read of live memory on the calling thread.
    let memory = unsafe { LiveMemory::new() };
    match native::state_owner(&memory, base, state) {
        Ok((id, owner)) => Some(StateOwner {
            state,
            id,
            owner,
            cause: performed(
                route(&native::STATE_OWNER_CALLERS, caller),
                Some(id),
                flags(&[third, fourth]),
            ),
        }),
        Err(reason) => {
            event_feed_observer::refuse(reason);
            None
        }
    }
}

/// Territory forward, after state owner setter returned normally: a StateTransfer
/// when the state's owner changed (state owner setter returns at once for the same
/// country).
pub(crate) fn state_owner_after(before: Option<StateOwner>) {
    let Some(before) = before else { return };
    event_feed_observer::record(|world, memory, base| {
        let (id, owner) = native::state_owner(memory, base, before.state)?;
        if id != before.id {
            return Err("event feed state id changed during SetOwner");
        }
        if owner == before.owner {
            return Ok(None);
        }
        Ok(Some(Fact {
            source: EventFeedSource::StateTransfer,
            actor: world.country(owner)?,
            target: world.country(before.owner)?,
            cause: before.cause,
        }))
    });
}

/// Counts an entry detour in flight until it returns or unwinds.
struct Flight;

impl Flight {
    fn enter() -> Self {
        ACTIVE.fetch_add(1, Ordering::SeqCst);
        Self
    }
}

impl Drop for Flight {
    fn drop(&mut self) {
        ACTIVE.fetch_sub(1, Ordering::SeqCst);
    }
}

type NativeFunction = unsafe extern "system-unwind" fn(
    usize,
    usize,
    usize,
    usize,
    usize,
    usize,
    usize,
    usize,
) -> usize;

/// Native arguments (four registers, four stack slots; entries with fewer
/// ignore the rest), the site and the caller's return address. RAX returns
/// unchanged from the original.
#[allow(clippy::too_many_arguments)] // Native args plus shim-only provenance.
unsafe extern "system-unwind" fn body(
    a: usize,
    b: usize,
    c: usize,
    d: usize,
    e: usize,
    f: usize,
    g: usize,
    h: usize,
    index: usize,
    caller: usize,
) -> usize {
    let _flight = Flight::enter();
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::EventFeedProvenance);
    let original: NativeFunction =
        unsafe { std::mem::transmute(ORIGINAL[index].load(Ordering::Acquire)) };
    let callers = native::ENTRIES[index].callers;
    let _scope = match index {
        // (diplomacy*, ally*, enemy*, u32 reason, u8 expand)
        JOIN_WAR => {
            let previous = CONTEXT.get();
            Some(set(Context {
                route: route(callers, caller),
                within: previous.route,
                conference: previous.conference,
                ally: operand(|world| world.country_at(b)).map(country),
                reason: Some(d as u32),
                flags: flags(&[e]),
            }))
        }
        // (war*, u8 truce): the endpoints before the link clears.
        END_WAR => {
            let cause = performed(route(callers, caller), None, flags(&[b]));
            event_feed_observer::record(|world, _, _| {
                let [first, second] = world.war_endpoints(a)?;
                Ok(Some(Fact {
                    source: EventFeedSource::WarEnded,
                    actor: first,
                    target: second,
                    cause,
                }))
            });
            None
        }
        // (manager*, loser*, winner*, i64 factor, u32, u32, u8, message*)
        CREATE_CONFERENCE => Some(enter(route(callers, caller), None, flags(&[g]))),
        // (conference*): the creation's conference, before it can end wars.
        START_CONFERENCE => {
            let mut context = CONTEXT.get();
            match operand(|world| world.conference(a)) {
                Some((reference, [winner, loser])) => {
                    context.conference = Some(reference);
                    let mut cause = inside();
                    cause.conference = Some(reference);
                    event_feed_observer::record(|_, _, _| {
                        Ok(Some(Fact {
                            source: EventFeedSource::PeaceConference,
                            actor: winner,
                            target: loser,
                            cause,
                        }))
                    });
                }
                None => event_feed_observer::refuse("event feed conference unreadable"),
            }
            Some(set(context))
        }
        // (conference*, u8)
        APPLY_CONFERENCE => {
            // SAFETY: a guarded read of live memory on the calling thread.
            let memory = unsafe { LiveMemory::new() };
            let conference = native::conference_reference(&memory, BASE.load(Ordering::Relaxed), a)
                .map_err(unread)
                .ok();
            Some(enter(EventFeedRoute::PeaceApplication, conference, 0))
        }
        // (taker*, u32 target, i64* out, u8 transfer troops, u8)
        ANNEX => Some(enter(route(callers, caller), None, flags(&[d, e]))),
        // (releaser*, target*, u8, u8) and (releaser*, target*, u8)
        RELEASE | RELEASE_COMMAND => {
            let reached = route(callers, caller);
            let bytes = if index == RELEASE {
                flags(&[c, d])
            } else {
                flags(&[c])
            };
            let cause = performed(reached, None, bytes);
            event_feed_observer::record(|world, _, _| {
                Ok(Some(Fact {
                    source: EventFeedSource::Release,
                    actor: world.country_object(a)?,
                    target: world.country_at(b)?,
                    cause,
                }))
            });
            Some(enter(reached, None, bytes))
        }
        _ => unreachable!("event feed entry index"),
    };
    timing.original(|| unsafe { original(a, b, c, d, e, f, g, h) })
}

// Eight native arguments (four copied stack slots), the site and the
// caller's return address. The shim keeps the native return address and
// allocates its own aligned argument area.
macro_rules! entry {
    ($name:ident, $index:expr) => {
        core::arch::global_asm!(
            concat!(".text
.globl event_provenance_", stringify!($name), "
.seh_proc event_provenance_", stringify!($name), "
event_provenance_", stringify!($name), ":"),
            "mov r10, rsp", "sub rsp, 0x58", ".seh_stackalloc 0x58", ".seh_endprologue",
            "mov rax, qword ptr [r10 + 0x28]", "mov qword ptr [rsp + 0x20], rax",
            "mov rax, qword ptr [r10 + 0x30]", "mov qword ptr [rsp + 0x28], rax",
            "mov rax, qword ptr [r10 + 0x38]", "mov qword ptr [rsp + 0x30], rax",
            "mov rax, qword ptr [r10 + 0x40]", "mov qword ptr [rsp + 0x38], rax",
            "mov qword ptr [rsp + 0x40], {index}",
            "mov rax, qword ptr [r10]", "mov qword ptr [rsp + 0x48], rax",
            "call {body}", "add rsp, 0x58", "ret", ".seh_endproc",
            index = const $index, body = sym body,
        );
        unsafe extern "system-unwind" {
            #[link_name = concat!("event_provenance_", stringify!($name))]
            fn $name();
        }
    };
}
entry!(join_war_entry, JOIN_WAR);
entry!(end_war_entry, END_WAR);
entry!(create_conference_entry, CREATE_CONFERENCE);
entry!(start_conference_entry, START_CONFERENCE);
entry!(apply_conference_entry, APPLY_CONFERENCE);
entry!(annex_entry, ANNEX);
entry!(release_entry, RELEASE);
entry!(release_command_entry, RELEASE_COMMAND);

const CALLBACKS: [unsafe extern "system-unwind" fn(); ENTRIES] = [
    join_war_entry,
    end_war_entry,
    create_conference_entry,
    start_conference_entry,
    apply_conference_entry,
    annex_entry,
    release_entry,
    release_command_entry,
];

/// The verified entry detours, unpublished; the event feed observer owns
/// them with its CALL hooks.
pub(crate) unsafe fn hooks(memory: &impl Memory, base: usize) -> Result<Vec<Hook>, &'static str> {
    let mut hooks = Vec::with_capacity(ENTRIES);
    for (index, (entry, callback)) in native::ENTRIES.iter().zip(CALLBACKS).enumerate() {
        let address = native::verify(memory, base, *entry)?;
        let hook = unsafe { Hook::new(address, callback as *const () as usize, entry.sentinel) }
            .map_err(|_| "Event feed entry detour construction failed")?;
        ORIGINAL[index].store(hook.trampoline(), Ordering::Release);
        hooks.push(hook);
    }
    BASE.store(base, Ordering::Relaxed);
    Ok(hooks)
}

/// Entry detours in flight.
pub(crate) fn active() -> usize {
    ACTIVE.load(Ordering::SeqCst)
}

pub(crate) fn status() -> impl serde::Serialize {
    crate::status::status!({
        "active": active(),
        "unclassified_callers": UNCLASSIFIED.load(Ordering::Relaxed),
        "unread_operands": UNREAD.load(Ordering::Relaxed),
        "first_unread": FIRST_UNREAD.try_lock().ok().and_then(|first| *first),
        "scope": "cumulative since attach; an unclassified caller or unread cause operand leaves that fact unrecorded (None), never guessed",
    })
}

#[cfg(test)]
mod tests;
