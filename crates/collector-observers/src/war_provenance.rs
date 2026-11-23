
use crate::{
    event_feed_observer::{self, Fact},
    hook::Hook,
};
use engine_events::{
    event_feed_sites::CountryIdentity, event_feed_sites::provenance as native,
    event_feed_sites::provenance::Caller,
};
use engine_native_memory::{Memory, memory::LiveMemory};
use replay_data::{
    event_feed_rows::EventFeedCause, event_feed_rows::EventFeedCountry,
    event_feed_rows::EventFeedRoute, event_feed_rows::EventFeedSource,
};
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
static ACTIVE: AtomicUsize = AtomicUsize::new(0);
static UNCLASSIFIED: AtomicU64 = AtomicU64::new(0);
static UNREAD: AtomicU64 = AtomicU64::new(0);
static FIRST_UNREAD: Mutex<Option<&'static str>> = Mutex::new(None);

fn unread(reason: &'static str) {
    UNREAD.fetch_add(1, Ordering::Relaxed);
    if let Ok(mut first) = FIRST_UNREAD.try_lock() {
        first.get_or_insert(reason);
    }
}

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

#[must_use]
pub struct Scope(Context);

impl Drop for Scope {
    fn drop(&mut self) {
        CONTEXT.set(self.0);
    }
}

fn set(context: Context) -> Scope {
    Scope(CONTEXT.replace(context))
}

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

pub fn inside() -> EventFeedCause {
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

fn route(callers: &[Caller], caller: usize) -> EventFeedRoute {
    match native::route(BASE.load(Ordering::Relaxed), callers, caller)
        .and_then(EventFeedRoute::named)
    {
        Some(route) => route,
        None => {
            UNCLASSIFIED.fetch_add(1, Ordering::Relaxed);
            EventFeedRoute::None
        }
    }
}

fn flags(bytes: &[usize]) -> u8 {
    bytes.iter().enumerate().fold(0, |flags, (bit, byte)| {
        flags | (u8::from(*byte as u8 != 0) << bit)
    })
}

pub fn country(identity: CountryIdentity) -> EventFeedCountry {
    EventFeedCountry {
        reference: identity.reference,
        id: identity.id,
        instance: identity.instance,
    }
}

fn operand<T>(
    observe: impl FnOnce(&native::World<'_, LiveMemory>) -> Result<T, &'static str>,
) -> Option<T> {
    let memory = unsafe { LiveMemory::new() };
    native::World::new(&memory, BASE.load(Ordering::Relaxed))
        .and_then(|world| observe(&world))
        .map_err(unread)
        .ok()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PoliticsScope {
    Diplomacy,
    CivilWar,
    Decision,
    TargetedDecision,
    DecisionTimeout,
    ScriptedDiplomacy,
    OnAction,
}

pub fn politics(scope: PoliticsScope, caller: usize) -> Option<Scope> {
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

pub struct StateOwner {
    state: usize,
    id: u32,
    owner: u32,
    cause: EventFeedCause,
}

pub fn state_owner_before(
    state: usize,
    third: usize,
    fourth: usize,
    caller: usize,
) -> Option<StateOwner> {
    let base = BASE.load(Ordering::Relaxed);
    if base == 0 {
        return None;
    }
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

pub fn state_owner_after(before: Option<StateOwner>) {
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

struct Flight;

impl Flight {
    fn enter() -> Self {
        crate::event_feed_observer::writer_moved();
        ACTIVE.fetch_add(1, Ordering::SeqCst);
        Self
    }
}

impl Drop for Flight {
    fn drop(&mut self) {
        ACTIVE.fetch_sub(1, Ordering::SeqCst);
        crate::event_feed_observer::writer_moved();
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

#[doc = " Native arguments (four registers, four stack slots; entries with fewer"]
#[doc = " ignore the rest), the site and the caller's return address. RAX returns"]
#[doc = " unchanged from the original."]
#[allow(clippy::too_many_arguments)]
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
        CREATE_CONFERENCE => Some(enter(route(callers, caller), None, flags(&[g]))),
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
        APPLY_CONFERENCE => {
            let memory = unsafe { LiveMemory::new() };
            let conference = native::conference_reference(&memory, BASE.load(Ordering::Relaxed), a)
                .map_err(unread)
                .ok();
            Some(enter(EventFeedRoute::PeaceApplication, conference, 0))
        }
        ANNEX => Some(enter(route(callers, caller), None, flags(&[d, e]))),
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
///
/// # Safety
/// The memory source and base must identify the authenticated executable in
/// this process. The caller owns this family exclusively and must restore
/// its hooks and quiesce native calls before releasing code or unloading.
pub unsafe fn hooks(_memory: &impl Memory, base: usize) -> Result<Vec<Hook>, &'static str> {
    let mut hooks = Vec::with_capacity(ENTRIES);
    for (index, (entry, callback)) in native::ENTRIES.iter().zip(CALLBACKS).enumerate() {
        let code = engine_sites::hook(base, entry.rva)?;
        let hook = unsafe {
            Hook::new(
                code.address,
                callback as *const () as usize,
                code.sentinel::<16>()?,
            )
        }
        .map_err(|_| "Event feed entry detour construction failed")?;
        ORIGINAL[index].store(hook.trampoline(), Ordering::Release);
        hooks.push(hook);
    }
    BASE.store(base, Ordering::Relaxed);
    Ok(hooks)
}

#[cfg(any(test, feature = "test-support"))]
pub fn base_for_test(base: usize) {
    BASE.store(base, Ordering::Relaxed);
}

pub fn active() -> usize {
    ACTIVE.load(Ordering::SeqCst)
}


#[cfg(test)]
mod tests;
