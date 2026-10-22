//! Anchor each native pass once. Only fire evaluation, retirement and queue
//! mutation boundaries are observed; quiet timers have no hook or callback.
use super::*;
use super::{hook_failure, verify_hook};
use std::cell::Cell;

#[derive(Clone, Copy, Default)]
struct Pass {
    owner: usize,
    generation: u64,
    cursor: u32,
    fire: usize,
    bad: bool,
}
impl Pass {
    fn advance(&mut self, end: u32) -> Result<u32, &'static str> {
        let count = end
            .checked_sub(self.cursor)
            .ok_or("delayed-event native cursor moved behind its anchor")?;
        self.cursor = end;
        Ok(count)
    }
    fn finish(&mut self, end: u32) -> u32 {
        // In-pass cancellation can shrink the vector behind the native
        // cursor. That closes the pass without visiting any additional entry.
        let count = end.saturating_sub(self.cursor);
        self.cursor = end;
        count
    }
    fn retired(&mut self, ordinal: u32, visited: bool) -> Result<(), &'static str> {
        if self.cursor
            != ordinal
                .checked_add(u32::from(visited))
                .ok_or("delayed-event retirement cursor overflow")?
        {
            return Err("delayed-event native retirement cursor differs");
        }
        self.cursor = ordinal;
        self.fire = 0;
        Ok(())
    }
}
std::thread_local! {static PASS:Cell<Pass>=const {Cell::new(Pass {owner:0,generation:0,cursor:0,fire:0,bad:false})};}
static ORIGINAL: [AtomicUsize; 2] = [const { AtomicUsize::new(0) }; 2];
// The pass entry's native Flight covers its anchor through the original's
// return, including unwind retention. No per-timer flight counter is needed.

fn flush_to(end: u32, finished: bool) {
    let mut pass = PASS.get();
    if pass.owner == 0 {
        return;
    }
    let result = if pass.bad || crate::world_lifetime::token() != Some(pass.generation) {
        Err("delayed-event anchor crossed an unobserved native world")
    } else if finished {
        Ok(pass.finish(end))
    } else {
        pass.advance(end)
    };
    match result {
        Ok(count) if count != 0 || finished => feed::record_script(|world, memory, _| {
            crate::event_feed_history::delayed::step(world, memory, pass.owner, count, finished)
                .map(Some)
        }),
        Ok(_) => {}
        Err(reason) => {
            pass.bad = true;
            feed::refuse(reason);
        }
    }
    PASS.set(pass);
}
unsafe extern "system-unwind" fn tap(registers: &Registers, index: usize) {
    let _timing = crate::hook_timing::enter(crate::hook_timing::Family::EventFeedProvenance);
    let pass = PASS.get();
    if pass.owner == 0 {
        return;
    }
    if pass.owner != registers.rdi {
        feed::refuse("delayed-event cursor boundary has a different owner");
        return;
    }
    if index == 0 {
        let Some(end) = u32::try_from(registers.rsi)
            .ok()
            .and_then(|n| n.checked_add(1))
        else {
            feed::refuse("delayed-event fire cursor exceeds native ordinal");
            return;
        };
        flush_to(end, false);
        let mut pass = PASS.get();
        pass.fire = registers.rbx;
        PASS.set(pass);
    } else {
        // R8 retains the selected ordinal; RSI has already been decremented
        // by native swap-removal. Invalid definitions bypass the decrement
        // and fire boundary, so they must not consume a countdown step.
        let visited = pass.fire == registers.rbx && registers.rbx != 0;
        let Some(ordinal) = u32::try_from(registers.r8).ok() else {
            feed::refuse("delayed-event retirement ordinal exceeds native word");
            return;
        };
        let Some(end) = ordinal.checked_add(u32::from(visited)) else {
            feed::refuse("delayed-event retirement ordinal overflow");
            return;
        };
        flush_to(end, false);
        // The pass this tap authenticated owns the retired timer.
        feed::retired_delayed(registers.rbx, Some((pass.owner, pass.generation)), visited);
        let mut pass = PASS.get();
        if let Err(reason) = pass.retired(ordinal, visited) {
            pass.bad = true;
            feed::refuse(reason);
        }
        PASS.set(pass);
    }
}
unsafe extern "system-unwind" fn schedule(
    country: usize,
    event: usize,
    originator: usize,
    scope: usize,
    hours: usize,
) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::EventFeedProvenance);
    let mut flight = Flight::enter();
    type Original = unsafe extern "system-unwind" fn(usize, usize, usize, usize, usize) -> usize;
    let original: Original = unsafe { std::mem::transmute(ORIGINAL[0].load(Ordering::Acquire)) };
    // Any in-pass script mutation follows the fire boundary, whose cursor
    // was already emitted before entering event evaluation/effects.
    let result = timing.original(|| unsafe { original(country, event, originator, scope, hours) });
    feed::scheduled_delayed(country);
    flight.returned();
    result
}
unsafe extern "system-unwind" fn destroy(pointer: usize, flags: usize) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::EventFeedProvenance);
    let mut flight = Flight::enter();
    feed::retired_delayed(pointer, None, false);
    type Original = unsafe extern "system-unwind" fn(usize, usize) -> usize;
    let original: Original = unsafe { std::mem::transmute(ORIGINAL[1].load(Ordering::Acquire)) };
    let result = timing.original(|| unsafe { original(pointer, flags) });
    flight.returned();
    result
}
struct Anchor {
    previous: Pass,
    opened: bool,
    completed: bool,
}
impl Drop for Anchor {
    fn drop(&mut self) {
        if self.opened {
            if !self.completed {
                feed::refuse("delayed-event pass did not return to close its anchor");
            }
            PASS.set(self.previous);
        }
    }
}
pub(super) fn forwarded_pass(country: usize, run: impl FnOnce() -> usize) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::EventFeedProvenance);
    let mut flight = Flight::enter();
    let memory = unsafe { engine_native::memory::LiveMemory::new() };
    let previous = PASS.get();
    let mut anchor = Anchor {
        previous,
        opened: false,
        completed: false,
    };
    // A single header word at the actual writer filters empty passes. No
    // timer is read and no queue is walked. Quiet members do no extra work.
    match field::<i32>(
        &memory,
        country,
        engine_native::ccountry::DELAYED_EVENTS_COUNT,
    ) {
        Ok(count) if count > 0 => {
            if previous.owner != 0 {
                feed::refuse("delayed-event native pass reentered before its anchor closed");
            } else if let Some(generation) = crate::world_lifetime::token() {
                PASS.set(Pass {
                    owner: country,
                    generation,
                    ..Pass::default()
                });
                anchor.opened = true;
            }
        }
        Ok(0) => {}
        _ => feed::refuse("delayed-event pass count unreadable or negative"),
    }
    let result = timing.original(run);
    if anchor.opened {
        match field::<i32>(
            &memory,
            country,
            engine_native::ccountry::DELAYED_EVENTS_COUNT,
        ) {
            Ok(count) if count >= 0 => flush_to(count as u32, true),
            _ => feed::refuse("delayed-event pass end count unreadable or negative"),
        }
        anchor.completed = true;
    }
    flight.returned();
    result
}
pub(super) unsafe fn hooks(
    memory: &impl Memory,
    base: usize,
) -> Result<(Vec<Hook>, Vec<Tap>), &'static str> {
    let mut hooks = Vec::new();

    let verify = |site: native::Site| verify_hook(memory, base, site, site.sentinel.len());
    for (index, (site, callback)) in native::DELAYED_ENTRIES
        .into_iter()
        .take(2)
        .zip([
            schedule as *const () as usize,
            destroy as *const () as usize,
        ])
        .enumerate()
    {
        let hook = unsafe { Hook::new(verify(site)?, callback, site.sentinel) }
            .map_err(|cause| hook_failure(site, "construction", cause))?
            .named(site.label());
        ORIGINAL[index].store(hook.trampoline(), Ordering::Release);
        hooks.push(hook);
    }
    Ok((hooks, Vec::new()))
}
pub(super) unsafe fn forwarded_taps(
    memory: &impl Memory,
    base: usize,
) -> Result<Vec<Tap>, &'static str> {
    let mut taps = Vec::new();
    for (index, site) in native::DELAYED_TAPS.into_iter().enumerate() {
        taps.push(
            unsafe {
                Tap::new(
                    verify_hook(memory, base, site, site.sentinel.len())?,
                    tap,
                    index,
                    site.sentinel,
                )
            }
            .map_err(|cause| hook_failure(site, "construction", cause))?
            .named(site.label()),
        );
    }
    Ok(taps)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancellation_can_close_the_pass_with_its_cursor_beyond_the_shortened_queue() {
        let mut pass = Pass::default();
        assert_eq!(pass.advance(5), Ok(5));
        assert_eq!(pass.finish(3), 0);
        assert_eq!(pass.cursor, 3);
    }
    #[test]
    fn sparse_cursor_anchors_derive_five_thousand_quiet_decrements_without_timer_callbacks() {
        let mut pass = Pass::default();
        assert_eq!(pass.advance(5000), Ok(5000)); // one pass end, no visits
        let mut pass = Pass::default();
        assert_eq!(pass.advance(3), Ok(3)); // third timer fires after decrement
        // Its effect appends a timer and cancels one beyond the cursor.
        assert_eq!(pass.retired(2, true), Ok(())); // swap is revisited
        assert_eq!(pass.advance(4), Ok(2));
        assert_eq!(pass.advance(5), Ok(1));
        assert_eq!(pass.retired(5, false), Ok(())); // invalid timer, no decrement
        assert_eq!(pass.advance(5000), Ok(4995));
        assert!(pass.advance(4999).is_err());
    }
    // regression: a306568425 Fix event registry writers, terminal results and delayed pass capture cost
    #[test]
    fn delayed_hooks_observe_only_passes_and_sparse_cursor_boundaries() {
        assert_eq!(
            native::DELAYED_TAPS.map(|site| site.name),
            ["DelayedEventFireBoundary", "RetireDelayedEvent"]
        );
        assert_eq!(native::DELAYED_ENTRIES[2].name, "DelayedEventPass");
    }
}
