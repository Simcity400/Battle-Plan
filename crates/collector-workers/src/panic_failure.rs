use std::{
    any::Any,
    cell::Cell,
    panic::PanicHookInfo,
    sync::{Arc, Mutex},
};

#[derive(Clone, Copy, Debug)]
pub struct Frame {
    pub index: usize,
    pub seq: u64,
    pub hour: i32,
    pub force_full: bool,
    pub load_signal: Option<i64>,
    pub world_generation: Option<u64>,
}

#[derive(Clone, Copy, Debug)]
struct Context {
    worker: &'static str,
    stage: &'static str,
    part: Option<usize>,
    frame: Option<Frame>,
}

thread_local! {
    static CONTEXT: Cell<Option<Context>> = const { Cell::new(None) };
    static PANIC: Cell<Option<&'static str>> = const { Cell::new(None) };
}

static FIRST: Mutex<Option<&'static str>> = Mutex::new(None);

pub fn first() -> Option<&'static str> {
    *FIRST
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn payload_text(payload: &(dyn Any + Send)) -> &str {
    payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("non-string panic payload")
}

fn describe(message: std::fmt::Arguments<'_>) -> &'static str {
    replay_data::codec::try_format(message)
        .map(|text| &*text.leak())
        .unwrap_or("decode panicked; failure context allocation failed")
}

fn confirm(payload: &(dyn Any + Send), fallback: &'static str) -> &'static str {
    let mut first = FIRST
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let observed = PANIC.take();
    first.get_or_insert_with(|| {
        observed.unwrap_or_else(|| {
            describe(format_args!("{fallback}; panic={}", payload_text(payload)))
        })
    })
}

pub fn joined(payload: Box<dyn Any + Send>, fallback: &'static str) -> &'static str {
    let message = confirm(&*payload, fallback);
    std::mem::forget(payload);
    message
}

type Hook = Box<dyn Fn(&PanicHookInfo<'_>) + Send + Sync + 'static>;

pub struct HookGuard(Option<Arc<Hook>>);

impl HookGuard {
    /// # Safety
    /// Install once during runtime attach, or while an isolated fixture owns the process panic
    /// hook. Retain the guard until every collector worker has stopped.
    pub unsafe fn install() -> Self {
        let previous = Arc::new(std::panic::take_hook());
        let chained = previous.clone();
        std::panic::set_hook(Box::new(move |info| {
            if let Some(context) = CONTEXT.get() {
                let Context {
                    worker,
                    stage,
                    part,
                    frame,
                } = context;
                if let Some(Frame {
                    index,
                    seq,
                    hour,
                    force_full,
                    load_signal,
                    world_generation,
                }) = frame
                {
                    PANIC.set(Some(describe(format_args!(
                        "decode panicked; worker={worker}; stage={stage}; part={part:?}; frame={index}; seq={seq}; hour={hour}; force_full={force_full}; load_signal={load_signal:?}; world_generation={world_generation:?}; branch=assigned by spool; panic={}; location={:?}",
                        payload_text(info.payload()), info.location()
                    ))));
                } else {
                    PANIC.set(Some(describe(format_args!(
                        "decode panicked; worker={worker}; stage={stage}; frame=none; panic={}; location={:?}",
                        payload_text(info.payload()), info.location()
                    ))));
                }
            }
            chained(info);
        }));
        Self(Some(previous))
    }
}

impl Drop for HookGuard {
    fn drop(&mut self) {
        drop(std::panic::take_hook());
        let previous = Arc::try_unwrap(self.0.take().unwrap())
            .unwrap_or_else(|_| unreachable!("decode panic hook still borrowed"));
        std::panic::set_hook(previous);
    }
}

pub struct Scope(Option<Context>);

impl Scope {
    pub fn worker(worker: &'static str) -> Self {
        PANIC.set(None);
        Self(CONTEXT.replace(Some(Context {
            worker,
            stage: "idle",
            part: None,
            frame: None,
        })))
    }

    pub fn frame(frame: Frame) -> Self {
        let old = CONTEXT.get();
        CONTEXT.set(Some(Context {
            worker: old.map_or("decode", |context| context.worker),
            stage: "take frame",
            part: None,
            frame: Some(frame),
        }));
        Self(old)
    }

    pub fn identity(load_signal: Option<i64>, world_generation: u64) {
        CONTEXT.set(CONTEXT.get().map(|mut context| {
            if let Some(frame) = &mut context.frame {
                frame.load_signal = load_signal;
                frame.world_generation = Some(world_generation);
            }
            context
        }));
    }
}

pub fn worker(run: impl FnOnce()) {
    let _scope = Scope::worker("decode worker");
    if let Err(payload) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(run)) {
        confirm(
            &*payload,
            "decode worker panicked; its in-flight frame may be incomplete",
        );
        std::panic::resume_unwind(payload);
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        CONTEXT.set(self.0);
    }
}

pub fn stage(stage: &'static str, part: Option<usize>) {
    CONTEXT.set(CONTEXT.get().map(|mut context| {
        context.stage = stage;
        context.part = part;
        context
    }));
}

#[cfg(any(test, feature = "test-support"))]
/// # Safety
/// Only an isolated test with exclusive use of the collector's process-wide panic state may change
/// this setting.
pub unsafe fn reset() {
    *FIRST.lock().unwrap() = None;
    PANIC.set(None);
}

pub fn owned_failed() -> bool {
    collector_memory::terminal::failed() || first().is_some()
}
