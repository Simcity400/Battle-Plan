use crate::{
    event_feed_observer as feed,
    hook::{Registers, Tap},
};
use engine_decisions::cevent;
use engine_events::event_window_sites as native;
use engine_native_memory::{Memory, msvc_string::Descriptor};
use replay_data::event_feed_rows::{
    EventPresentation, EventWindows, OfferedOption, VerdictEntry, WindowKind, WindowVerdict,
};
use replay_data::rows::Recorded;
use std::cell::Cell;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

static BASE: AtomicUsize = AtomicUsize::new(0);
static VERDICT_WITHHELD: Mutex<Option<String>> = Mutex::new(None);
static PRESENTATION_WITHHELD: Mutex<Option<String>> = Mutex::new(None);
static VERDICTS_ARMED: AtomicUsize = AtomicUsize::new(0);
static PRESENTATIONS_ARMED: AtomicUsize = AtomicUsize::new(0);
static VERDICT_REFUSALS: AtomicU64 = AtomicU64::new(0);
static PRESENTATION_REFUSALS: AtomicU64 = AtomicU64::new(0);
static PRESENTATIONS_PUBLISHED: AtomicU64 = AtomicU64::new(0);
static PRESENTATIONS_OTHER_WORLD: AtomicU64 = AtomicU64::new(0);

const SECTION_INCOMPLETE: &str = "event window verdict section reached its end without its deciding gate";
const UNARMED: &str = "event window capture was withheld on this build";
const LOST: &str = "event window presentations of an earlier interval were discarded unpublished";
const ALLOCATION: &str = "event window presentation allocation failed";
const OTHER_WINDOW: &str = "event window build piece belongs to another window";
const WORLD_CHANGED: &str = "event window build crossed a world change or an unobserved world";

#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct Trace {
    frame: usize,
    past_ai: bool,
    past_viewer: bool,
    new_instance: bool,
    sender_excluded: bool,
    player_restricted: Option<WindowVerdict>,
    open_test: bool,
    requested: bool,
    refused: Option<&'static str>,
}

impl Trace {
    pub fn resolve(self, hidden: bool) -> Result<WindowVerdict, &'static str> {
        if let Some(reason) = self.refused {
            return Err(reason);
        }
        Ok(if !self.past_ai {
            if hidden {
                WindowVerdict::HiddenOrAiControlled
            } else {
                WindowVerdict::AiControlled
            }
        } else if !self.past_viewer {
            WindowVerdict::NotViewingCountry
        } else if self.sender_excluded {
            WindowVerdict::SenderExcluded
        } else if let Some(restricted) = self.player_restricted {
            restricted
        } else if !self.new_instance {
            WindowVerdict::ExistingInstance
        } else if !self.open_test {
            return Err(SECTION_INCOMPLETE);
        } else if !self.requested {
            WindowVerdict::AlreadyOpen
        } else {
            WindowVerdict::Requested
        })
    }
}

std::thread_local! {
    static TRACE: Cell<Trace> = const {
        Cell::new(Trace {
            frame: 0,
            past_ai: false,
            past_viewer: false,
            new_instance: false,
            sender_excluded: false,
            player_restricted: None,
            open_test: false,
            requested: false,
            refused: None,
        })
    };
    static RELOADING: Cell<usize> = const { Cell::new(0) };
}

fn field<T: engine_native_memory::memory::Raw>(
    memory: &impl Memory,
    object: usize,
    offset: usize,
) -> Result<T, &'static str> {
    object
        .checked_add(offset)
        .and_then(|at| engine_native_memory::read(memory, at))
        .ok_or("event window field unreadable or overflowed")
}

fn mark(frame: usize, apply: impl FnOnce(&mut Trace)) {
    TRACE.with(|cell| {
        let mut trace = cell.get();
        if trace.frame != frame {
            trace = Trace {
                frame,
                ..Trace::default()
            };
        }
        apply(&mut trace);
        cell.set(trace);
    });
}

pub fn player_restriction(
    memory: &impl Memory,
    event: usize,
) -> Result<Option<WindowVerdict>, &'static str> {
    if field::<u8>(memory, event, cevent::MAJOR)? == 0 {
        return Ok(Some(WindowVerdict::PlayerRestrictedNotMajor));
    }
    let count = field::<i32>(memory, event, cevent::OPTION_COUNT)?;
    let options = field::<u64>(memory, event, cevent::OPTIONS)? as usize;
    for ordinal in 0..count.max(0) as usize {
        let option = ordinal
            .checked_mul(8)
            .and_then(|offset| options.checked_add(offset))
            .ok_or("event window option address overflow")?;
        let option = field::<u64>(memory, option, 0)? as usize;
        if field::<i32>(memory, option, cevent::OPTION_TRIGGER_COUNT)? != 0 {
            return Ok(Some(WindowVerdict::PlayerRestrictedOptionTrigger));
        }
    }
    Ok(None)
}

unsafe extern "system-unwind" fn verdict_tap(registers: &Registers, index: usize) {
    let _timing = crate::hook_timing::enter(crate::hook_timing::Family::EventFeedProvenance);
    let frame = registers.rbp;
    let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
    match index {
        0 => mark(frame, |trace| trace.past_ai = true),
        1 => mark(frame, |trace| {
            trace.past_viewer = true;
            trace.new_instance = registers.r14 as u8 != 0;
        }),
        2 => match field::<u8>(&memory, registers.rsi, cevent::FIRE_FOR_SENDER) {
            Ok(fire_for_sender) => mark(frame, |trace| trace.sender_excluded = fire_for_sender == 0),
            Err(reason) => refuse_verdict(frame, reason),
        },
        3 => match player_restriction(&memory, registers.rsi) {
            Ok(restricted) => mark(frame, |trace| trace.player_restricted = restricted),
            Err(reason) => refuse_verdict(frame, reason),
        },
        4 => mark(frame, |trace| trace.open_test = true),
        5 => mark(frame, |trace| trace.requested = true),
        6 => {
            let trace = TRACE.with(|cell| {
                let trace = cell.get();
                cell.set(Trace::default());
                trace
            });
            let trace = if trace.frame == frame {
                trace
            } else {
                Trace::default()
            };
            let verdict = field::<u8>(&memory, registers.rsi, cevent::HIDDEN)
                .and_then(|hidden| trace.resolve(hidden != 0));
            if verdict.is_err() {
                VERDICT_REFUSALS.fetch_add(1, Ordering::Relaxed);
            }
            feed::script_window(frame, verdict.ok());
        }
        _ => refuse_verdict(frame, "event window verdict tap identity differs"),
    }
}

fn refuse_verdict(frame: usize, reason: &'static str) {
    mark(frame, |trace| trace.refused = Some(reason));
}

#[derive(Default)]
struct Assembly {
    window: usize,
    world: Option<u64>,
    refused: Option<&'static str>,
    rebuild: bool,
    title: Option<Recorded<Vec<u8>>>,
    description: Option<Recorded<Vec<u8>>>,
    picture: Option<Recorded<Vec<u8>>>,
    options: Recorded<Vec<OfferedOption>>,
}

struct Queue {
    presentations: Vec<(u64, EventPresentation)>,
    failures: Vec<(u64, &'static str)>,
    overflow: Option<Overflow>,
    unattributed: Option<&'static str>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Overflow {
    first: u64,
    last: u64,
    reason: &'static str,
}

impl Queue {
    fn fail(&mut self, generation: u64, reason: &'static str) {
        if self.failures.iter().any(|(failed, _)| *failed == generation) {
            return;
        }
        if engine_allocation::reserve(&mut self.failures, 1).is_ok() {
            self.failures.push((generation, reason));
        } else {
            self.overflowed(generation, generation, reason);
        }
    }

    fn overflowed(&mut self, first: u64, last: u64, reason: &'static str) {
        let held = self.overflow.get_or_insert(Overflow { first, last, reason });
        held.first = held.first.min(first);
        held.last = held.last.max(last);
    }

    fn take_failure(&mut self, generation: u64) -> Option<&'static str> {
        let own = self
            .failures
            .iter()
            .find(|(failed, _)| *failed == generation)
            .map(|(_, reason)| *reason);
        self.failures.retain(|(failed, _)| *failed > generation);
        let overflow = self
            .overflow
            .filter(|overflow| (overflow.first..=overflow.last).contains(&generation))
            .map(|overflow| overflow.reason);
        if self.overflow.is_some_and(|overflow| overflow.last <= generation) {
            self.overflow = None;
        }
        own.or(overflow).or(self.unattributed.take())
    }
}

static ASSEMBLY: Mutex<Option<Assembly>> = Mutex::new(None);
static QUEUE: Mutex<Queue> = Mutex::new(Queue {
    presentations: Vec::new(),
    failures: Vec::new(),
    overflow: None,
    unattributed: None,
});

fn copy_string(memory: &impl Memory, address: usize) -> Recorded<Vec<u8>> {
    match Descriptor::read(memory, address) {
        Ok(descriptor) => copy_descriptor(memory, descriptor),
        Err(_) => {
            PRESENTATION_REFUSALS.fetch_add(1, Ordering::Relaxed);
            Recorded::NotRecorded
        }
    }
}

fn copy_descriptor(memory: &impl Memory, descriptor: Descriptor) -> Recorded<Vec<u8>> {
    let copied = (|| {
        let mut bytes = engine_allocation::vec(descriptor.len).map_err(|_| ALLOCATION)?;
        bytes.resize(descriptor.len, 0);
        descriptor.copy(memory, &mut bytes)?;
        Ok::<_, &'static str>(bytes)
    })();
    match copied {
        Ok(bytes) => Recorded::Value(bytes),
        Err(_) => {
            PRESENTATION_REFUSALS.fetch_add(1, Ordering::Relaxed);
            Recorded::NotRecorded
        }
    }
}

fn frame_string(registers: &Registers, offset: isize) -> Option<usize> {
    registers.rbp.checked_add_signed(offset)
}

fn assembling(window: usize, apply: impl FnOnce(&mut Assembly)) {
    let mut assembly = ASSEMBLY
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    match assembly.as_mut() {
        Some(current) if current.window == window => apply(current),
        _ => {
            PRESENTATION_REFUSALS.fetch_add(1, Ordering::Relaxed);
        }
    }
}

fn window_of(memory: &impl Memory, window: usize) -> Result<usize, &'static str> {
    let base = BASE.load(Ordering::Acquire);
    match engine_sites::at(base, native::WINDOW_VTABLE_RVA) {
        Some(vtable) if engine_native_memory::read::<u64>(memory, window) == Some(vtable as u64) => {
            Ok(window)
        }
        _ => Err(OTHER_WINDOW),
    }
}

unsafe extern "system-unwind" fn presentation_tap(registers: &Registers, index: usize) {
    let _timing = crate::hook_timing::enter(crate::hook_timing::Family::EventFeedProvenance);
    let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
    match index {
        0 => {
            let window = registers.rbx;
            let world = crate::world_lifetime::observed_generation();
            let rebuild = RELOADING.with(|cell| cell.replace(0)) == window;
            let title = frame_string(registers, native::TITLE_STRING_FROM_FRAME)
                .map_or(Recorded::NotRecorded, |at| copy_string(&memory, at));
            let refused = window_of(&memory, window).err();
            if refused.is_some() {
                PRESENTATION_REFUSALS.fetch_add(1, Ordering::Relaxed);
            }
            *ASSEMBLY
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Assembly {
                window,
                world,
                refused,
                rebuild,
                title: Some(title),
                options: Recorded::Value(Vec::new()),
                ..Assembly::default()
            });
        }
        1 => {
            let processed = frame_string(registers, native::PICTURE_RESOLVED_FROM_FRAME)
                .map_or(Recorded::NotRecorded, |at| copy_string(&memory, at));
            let picture = match processed {
                Recorded::Value(key) if !key.is_empty() => Recorded::Value(key),
                Recorded::Value(_) => registers
                    .rsp()
                    .checked_add(native::PICTURE_KEY_FROM_STACK)
                    .map_or(Recorded::NotRecorded, |at| copy_string(&memory, at)),
                Recorded::NotRecorded => Recorded::NotRecorded,
            };
            assembling(registers.r14, |assembly| assembly.picture = Some(picture));
        }
        2 | 3 => {
            let window = registers.rbx;
            let (label, ordinal) = if index == 2 {
                (copy_string(&memory, registers.rax), Some(registers.rdi as u32))
            } else {
                (copy_string(&memory, registers.rcx), None)
            };
            let label = match label {
                Recorded::Value(label) => Some(label),
                Recorded::NotRecorded => None,
            };
            assembling(window, |assembly| {
                assembly.options = match std::mem::replace(&mut assembly.options, Recorded::NotRecorded) {
                    Recorded::Value(mut options) => {
                        if engine_allocation::reserve(&mut options, 1).is_ok() {
                            options.push(OfferedOption { ordinal, label });
                            Recorded::Value(options)
                        } else {
                            PRESENTATION_REFUSALS.fetch_add(1, Ordering::Relaxed);
                            Recorded::NotRecorded
                        }
                    }
                    Recorded::NotRecorded => Recorded::NotRecorded,
                };
            });
        }
        4 => {
            let description = frame_string(registers, native::DESCRIPTION_STRING_FROM_FRAME)
                .map_or(Recorded::NotRecorded, |at| copy_string(&memory, at));
            assembling(registers.rbx, |assembly| assembly.description = Some(description));
        }
        5 => commit(&memory, registers),
        6 => RELOADING.with(|cell| cell.set(registers.rbx)),
        _ => {
            PRESENTATION_REFUSALS.fetch_add(1, Ordering::Relaxed);
        }
    }
}

fn kind_of(memory: &impl Memory, event: usize) -> Result<WindowKind, &'static str> {
    Ok(if field::<u8>(memory, event, cevent::NEWS_CHROME)? != 0 {
        WindowKind::News
    } else if field::<u8>(memory, event, cevent::LEADER_CHROME)? != 0 {
        WindowKind::Leader
    } else if field::<u8>(memory, event, cevent::OPERATIVE_CHROME)? != 0 {
        WindowKind::Operative
    } else {
        WindowKind::Normal
    })
}

fn commit(memory: &impl Memory, registers: &Registers) {
    let window = registers.rbx;
    let assembly = ASSEMBLY
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take();
    let base = BASE.load(Ordering::Acquire);
    let world = assembly
        .as_ref()
        .and_then(|assembly| assembly.world)
        .or_else(crate::world_lifetime::observed_generation);
    let built = (|| {
        let assembly = assembly
            .filter(|assembly| assembly.window == window)
            .ok_or(OTHER_WINDOW)?;
        if let Some(reason) = assembly.refused {
            return Err(reason);
        }
        let start = assembly.world.ok_or(WORLD_CHANGED)?;
        let event = field::<u64>(memory, window, native::WINDOW_EVENT)? as usize;
        let (identity, token) = crate::script_event_observer::definition(memory, base, event)?;
        let kind = kind_of(memory, event)?;
        let shown = if kind == WindowKind::News {
            registers.rdi as u8
        } else {
            registers.rdx as u8
        };
        let picture = match (kind, assembly.picture) {
            (WindowKind::Normal | WindowKind::News, Some(Recorded::Value(key))) => Recorded::Value(Some(key)),
            (WindowKind::Normal | WindowKind::News, Some(Recorded::NotRecorded)) => Recorded::NotRecorded,
            _ => Recorded::Value(None),
        };
        Ok::<_, &'static str>(EventPresentation {
            native_hour: engine_events::event_feed_sites::provenance::World::new(memory, base)
                .and_then(|world| world.hour())
                .ok(),
            instance: field(memory, window, native::WINDOW_PENDING_INSTANCE)?,
            event_kind: identity.kind,
            event_id: identity.id,
            actor: field(memory, window, native::WINDOW_ACTOR_TAG)?,
            rebuild: assembly.rebuild,
            kind,
            shown: Some(shown != 0),
            option_count: field(memory, event, cevent::OPTION_COUNT)?,
            token: copy_descriptor(memory, token),
            title: assembly.title.unwrap_or(Recorded::NotRecorded),
            description: assembly.description.unwrap_or(Recorded::NotRecorded),
            picture,
            options: assembly.options,
        })
        .and_then(|presentation| {
            if crate::world_lifetime::observed_generation() == Some(start) {
                Ok(presentation)
            } else {
                Err(WORLD_CHANGED)
            }
        })
    })();
    let mut queue = QUEUE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    match (built, world) {
        (Ok(presentation), Some(world)) => {
            if engine_allocation::reserve(&mut queue.presentations, 1).is_ok() {
                queue.presentations.push((world, presentation));
                PRESENTATIONS_PUBLISHED.fetch_add(1, Ordering::Relaxed);
            } else {
                queue.fail(world, ALLOCATION);
            }
        }
        (Err(reason), Some(world)) => {
            PRESENTATION_REFUSALS.fetch_add(1, Ordering::Relaxed);
            queue.fail(world, reason);
        }
        (_, None) => {
            PRESENTATION_REFUSALS.fetch_add(1, Ordering::Relaxed);
            queue.unattributed.get_or_insert(WORLD_CHANGED);
        }
    }
}

#[derive(Default)]
pub struct Taken {
    verdicts_armed: bool,
    presentations_armed: bool,
    presentations: Vec<(u64, EventPresentation)>,
    failure: Option<&'static str>,
}

pub fn take(generation: u64) -> Taken {
    let mut taken = Taken {
        verdicts_armed: VERDICTS_ARMED.load(Ordering::Acquire) != 0,
        presentations_armed: PRESENTATIONS_ARMED.load(Ordering::Acquire) != 0,
        ..Taken::default()
    };
    if taken.presentations_armed {
        let mut queue = QUEUE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        taken.presentations = std::mem::take(&mut queue.presentations);
        if let Some(newer) = taken.presentations.iter().position(|(built, _)| *built > generation) {
            let later = taken.presentations.len() - newer;
            if engine_allocation::reserve(&mut queue.presentations, later).is_ok() {
                queue.presentations.extend(taken.presentations.drain(newer..));
            } else {
                for index in newer..taken.presentations.len() {
                    let world = taken.presentations[index].0;
                    queue.fail(world, ALLOCATION);
                }
                taken.presentations.truncate(newer);
            }
        }
        let queued = taken.presentations.len();
        taken.presentations.retain(|(built, _)| *built == generation);
        PRESENTATIONS_OTHER_WORLD.fetch_add((queued - taken.presentations.len()) as u64, Ordering::Relaxed);
        taken.failure = queue.take_failure(generation);
    }
    taken
}

impl Taken {
    pub fn allocated_bytes(&self) -> usize {
        self.presentations.capacity() * size_of::<(u64, EventPresentation)>()
            + self
                .presentations
                .iter()
                .map(|(_, presentation)| replay_data::value::Value::heap_bytes(presentation))
                .sum::<usize>()
    }

    pub fn windows(&self, verdicts: &[VerdictEntry]) -> std::io::Result<EventWindows> {
        let copy = |bytes: &[u8]| engine_allocation::copy_vec(bytes);
        let reason = |withheld: &Mutex<Option<String>>| {
            let held = withheld.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            copy(held.as_deref().unwrap_or(UNARMED).as_bytes())
        };
        let mut presentations = engine_allocation::vec(self.presentations.len())?;
        for (_, presentation) in &self.presentations {
            presentations.push(replay_data::value::Value::try_clone(presentation)?);
        }
        Ok(EventWindows {
            verdicts: if self.verdicts_armed {
                Ok(engine_allocation::copy_vec(verdicts)?)
            } else {
                Err(reason(&VERDICT_WITHHELD)?)
            },
            presentations,
            presentations_unrecorded: if self.presentations_armed {
                self.failure.map(|failure| copy(failure.as_bytes())).transpose()?
            } else {
                Some(reason(&PRESENTATION_WITHHELD)?)
            },
        })
    }
}

pub fn discarded(generation: u64) {
    QUEUE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .fail(generation, LOST);
}

pub fn status() -> impl serde::Serialize {
    let withheld = |reason: &Mutex<Option<String>>| {
        reason
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    };
    crate::status::status!({
        "verdict_armed": VERDICTS_ARMED.load(Ordering::Relaxed) != 0,
        "verdict_withheld": withheld(&VERDICT_WITHHELD),
        "verdict_refusals": VERDICT_REFUSALS.load(Ordering::Relaxed),
        "presentation_armed": PRESENTATIONS_ARMED.load(Ordering::Relaxed) != 0,
        "presentation_withheld": withheld(&PRESENTATION_WITHHELD),
        "presentation_refusals": PRESENTATION_REFUSALS.load(Ordering::Relaxed),
        "presentations": PRESENTATIONS_PUBLISHED.load(Ordering::Relaxed),
        "presentations_of_an_ended_world": PRESENTATIONS_OTHER_WORLD.load(Ordering::Relaxed),
    })
}

fn group(
    base: usize,
    sites: &[engine_events::script_event_sites::Site],
    observer: unsafe extern "system-unwind" fn(&Registers, usize),
    failure: &str,
) -> Result<Vec<Tap>, String> {
    let mut taps = Vec::with_capacity(sites.len());
    for (index, site) in sites.iter().enumerate() {
        let code = engine_sites::code(base, site.rva)
            .map_err(|cause| format!("{failure}: {}: {cause}", site.name))?;
        let owned = code
            .original::<16>()
            .map_err(|cause| format!("{failure}: {}: {cause}", site.name))?;
        taps.push(
            unsafe { Tap::new(code.address, observer, index, owned) }
                .map_err(|cause| format!("{failure}: {}: {cause}", site.name))?
                .named(site.name),
        );
    }
    Ok(taps)
}

/// # Safety
/// The event-feed host publishes, restores and quiesces these taps with the
/// rest of its family. A group that fails to construct is withheld by name and
/// leaves the rest of the family installed.
pub unsafe fn taps(base: usize) -> Vec<Tap> {
    BASE.store(base, Ordering::Release);
    let mut installed = Vec::new();
    for (sites, observer, failure, armed, withheld) in [
        (
            &native::VERDICT_TAPS[..],
            verdict_tap as unsafe extern "system-unwind" fn(&Registers, usize),
            native::VERDICT_FAILURE,
            &VERDICTS_ARMED,
            &VERDICT_WITHHELD,
        ),
        (
            &native::PRESENTATION_TAPS[..],
            presentation_tap as unsafe extern "system-unwind" fn(&Registers, usize),
            native::PRESENTATION_FAILURE,
            &PRESENTATIONS_ARMED,
            &PRESENTATION_WITHHELD,
        ),
    ] {
        match group(base, sites, observer, failure) {
            Ok(taps) => {
                armed.store(1, Ordering::Release);
                installed.extend(taps);
            }
            Err(reason) => {
                armed.store(0, Ordering::Release);
                *withheld.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(reason);
            }
        }
    }
    installed
}

#[cfg(test)]
mod tests;
