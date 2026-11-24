use std::sync::{
    Mutex,
    atomic::{AtomicPtr, AtomicU64, AtomicUsize, Ordering},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Writer {
    RebuildStats,
    RefreshDerivedState,
    MoveAssign,
    Update,
    Publish,
    Destroy,
    SourceInvalidation,
    IdeaInvalidation,
    ProviderInvalidation,
    CountryInvalidation,
    ContentInvalidation,
    Edit,
    Roster,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Before,
    After,
    Unwound,
}
#[derive(Clone, Copy)]
pub struct Availability {
    word: u64,
    proof: &'static AtomicU64,
    dispatch: &'static AtomicU64,
    published: &'static AtomicU64,
}
impl Availability {
    pub fn stamp(self) -> u64 {
        self.word
    }
    pub fn is_current(self) -> bool {
        self.word & 1 != 0
            && self.proof.load(Ordering::SeqCst) == self.word
            && self.dispatch.load(Ordering::SeqCst) == self.word
            && self.published.load(Ordering::SeqCst) == self.word
            && self.proof.load(Ordering::SeqCst) == self.word
    }
}
#[doc = " Receivers are process-static. Neither callback may panic, re-enter"]
#[doc = " registration, block, or change hook availability. Reset delivers a withdrawn"]
#[doc = " capability on the native callback; active calls retain their terminal phase."]
#[doc = " Never convert availability into an unchecked cached boolean. A native pointer"]
#[doc = " is borrowed only for this invocation, not a reference-lifetime witness."]
pub struct Forwarder {
    pub call: unsafe fn(Writer, usize, Phase),
    pub enabled: fn(Availability),
}
struct State {
    receiver: Option<&'static Forwarder>,
}
struct Forwarding {
    state: Mutex<State>,
    admission: AtomicU64,
    dispatch: AtomicU64,
    published: AtomicU64,
    writers_active: fn() -> bool,
    registered: AtomicPtr<Forwarder>,
    receiver: AtomicPtr<Forwarder>,
    active: AtomicUsize,
}
impl Forwarding {
    const fn new(writers_active: fn() -> bool) -> Self {
        Self {
            state: Mutex::new(State { receiver: None }),
            admission: AtomicU64::new(0),
            dispatch: AtomicU64::new(0),
            published: AtomicU64::new(0),
            writers_active,
            registered: AtomicPtr::new(std::ptr::null_mut()),
            receiver: AtomicPtr::new(std::ptr::null_mut()),
            active: AtomicUsize::new(0),
        }
    }
    fn withdraw(&self) {
        self.published.store(0, Ordering::SeqCst);
        self.dispatch.store(0, Ordering::SeqCst);
        self.receiver.store(std::ptr::null_mut(), Ordering::SeqCst);
        self.drain_writers();
    }
    fn drain_writers(&self) {
        while (self.writers_active)() || self.active.load(Ordering::SeqCst) != 0 {
            std::thread::yield_now();
        }
    }
    fn register(&'static self, receiver: Option<&'static Forwarder>) {
        self.register_with(receiver, || {});
    }
    fn register_with(
        &'static self,
        receiver: Option<&'static Forwarder>,
        after_withdraw: impl FnOnce(),
    ) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let token = self
            .admission
            .fetch_add(2, Ordering::SeqCst)
            .wrapping_add(2);
        self.withdraw();
        after_withdraw();
        if let Some(previous) = state.receiver {
            (previous.enabled)(self.capability(0));
        }
        state.receiver = receiver;
        self.registered.store(
            receiver.map_or(std::ptr::null_mut(), |r| std::ptr::from_ref(r).cast_mut()),
            Ordering::SeqCst,
        );
        if let Some(receiver) = receiver {
            self.publish(receiver, token);
        }
    }
    #[cfg(test)]
    fn available(&'static self, available: bool) {
        self.available_at(self.admission.load(Ordering::SeqCst), available);
    }
    fn available_at(&'static self, token: u64, available: bool) {
        self.available_at_with(token, available, || {});
    }
    fn available_at_with(&'static self, token: u64, available: bool, after_cas: impl FnOnce()) {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if (token & 1 != 0) == available || self.admission.load(Ordering::SeqCst) != token {
            return;
        }
        let next = if available {
            token | 1
        } else {
            token.wrapping_add(2) & !1
        };
        self.withdraw();
        if self
            .admission
            .compare_exchange(token, next, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return;
        }
        after_cas();
        if let Some(receiver) = state.receiver {
            self.publish(receiver, next);
        }
    }
    fn capability(&'static self, word: u64) -> Availability {
        Availability {
            word,
            proof: &self.admission,
            dispatch: &self.dispatch,
            published: &self.published,
        }
    }
    fn publish(&'static self, receiver: &'static Forwarder, token: u64) {
        let capability = self.capability(token);
        if self.admission.load(Ordering::SeqCst) != token {
            return;
        }
        if token & 1 != 0 {
            self.receiver
                .store(std::ptr::from_ref(receiver).cast_mut(), Ordering::SeqCst);
            self.dispatch.store(token, Ordering::SeqCst);
            self.drain_writers();
            self.published.store(token, Ordering::SeqCst);
            if !capability.is_current() {
                self.dispatch.store(0, Ordering::SeqCst);
                self.receiver.store(std::ptr::null_mut(), Ordering::SeqCst);
                return;
            }
        }
        (receiver.enabled)(capability);
    }

    fn invalidate(&'static self) {
        let old = self
            .admission
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |old| {
                Some(old.wrapping_add(2) & !1)
            })
            .unwrap();
        let withdrawn = old.wrapping_add(2) & !1;
        self.published.store(0, Ordering::SeqCst);
        self.dispatch.store(0, Ordering::SeqCst);
        self.receiver.store(std::ptr::null_mut(), Ordering::SeqCst);
        let receiver = self.registered.load(Ordering::SeqCst);
        if !receiver.is_null() {
            unsafe { ((*receiver).enabled)(self.capability(withdrawn)) };
        }
    }
    /// # Safety
    /// `variant` is the object the notification concerns: a writer's receiver, either operand of a
    /// move, or zero when it names none. It must stay valid for whatever the registered handler
    /// reads in `Phase::Before`; after the native call it is an identity only, since a writer such
    /// as `Destroy` may have freed it.
    unsafe fn begin(&self, writer: Writer, variant: usize) -> Option<Invocation<'_>> {
        if self.receiver.load(Ordering::SeqCst).is_null() {
            return None;
        }
        self.active.fetch_add(1, Ordering::SeqCst);
        let token = self.admission.load(Ordering::SeqCst);
        let admitted = token & 1 != 0 && self.dispatch.load(Ordering::SeqCst) == token;
        let receiver = self.receiver.load(Ordering::SeqCst);
        if receiver.is_null() || !admitted {
            self.active.fetch_sub(1, Ordering::SeqCst);
            return None;
        }
        let invocation = Invocation {
            owner: self,
            receiver: unsafe { &*receiver },
            writer,
            variant,
            completed: false,
        };
        unsafe { (invocation.receiver.call)(writer, variant, Phase::Before) };
        Some(invocation)
    }
}
pub(super) struct Invocation<'a> {
    owner: &'a Forwarding,
    receiver: &'static Forwarder,
    writer: Writer,
    variant: usize,
    completed: bool,
}
impl Invocation<'_> {
    /// # Safety
    /// Delivers the successful terminal phase (`Phase::After`) to the registered handler, for which
    /// the object is an identity only, as [`Forwarding::begin`] says.
    pub(super) unsafe fn finish(mut self) {
        self.completed = true;
        unsafe { (self.receiver.call)(self.writer, self.variant, Phase::After) };
    }
}
impl Drop for Invocation<'_> {
    fn drop(&mut self) {
        if !self.completed {
            unsafe { (self.receiver.call)(self.writer, self.variant, Phase::Unwound) };
        }
        self.owner.active.fetch_sub(1, Ordering::SeqCst);
    }
}
fn writers_active() -> bool {
    super::DESIGN_GATE.any_active()
        || super::super::design_template_changes::active() != 0
        || collector_observers::world_input_changes::design_forwarder_active() != 0
}
static FORWARDING: Forwarding = Forwarding::new(writers_active);
pub fn forwarder(receiver: Option<&'static Forwarder>) {
    FORWARDING.register(receiver);
}
pub fn availability_is_current(stamp: u64) -> bool {
    FORWARDING.capability(stamp).is_current()
}
pub(super) fn token() -> u64 {
    FORWARDING.admission.load(Ordering::SeqCst)
}
pub(super) fn available_at(token: u64, enabled: bool) {
    FORWARDING.available_at(token, enabled);
}
pub(super) fn invalidate() {
    FORWARDING.invalidate();
}
fn writer(site: usize) -> Option<Writer> {
    use engine_native::cequipment_variant::mutations::{MUTATIONS, Receiver};
    match MUTATIONS[site].name {
        "RebuildStats" => Some(Writer::RebuildStats),
        "RefreshDerivedState" => Some(Writer::RefreshDerivedState),
        "MoveAssign" => Some(Writer::MoveAssign),
        "Update" => Some(Writer::Update),
        "Publish" => Some(Writer::Publish),
        "Destroy" => Some(Writer::Destroy),
        _ => match MUTATIONS[site].receiver {
            Receiver::Roster => Some(Writer::Roster),
            Receiver::CreateCommand => None,
            _ => Some(Writer::Edit),
        },
    }
}
/// # Safety
/// As [`Forwarding::begin`], with `site` a design writer site.
pub(super) unsafe fn begin(site: usize, variant: usize) -> Option<Invocation<'static>> {
    unsafe { FORWARDING.begin(writer(site)?, variant) }
}
/// # Safety
/// As [`Forwarding::begin`], with `receiver` the invalidating call's live receiver.
pub(super) unsafe fn invalidation(provider: bool, receiver: usize) -> Option<Invocation<'static>> {
    unsafe {
        FORWARDING.begin(
            if provider {
                Writer::ProviderInvalidation
            } else {
                Writer::SourceInvalidation
            },
            receiver,
        )
    }
}
/// # Safety
/// As [`Forwarding::begin`], with `receiver` the invalidating call's live country.
pub(super) unsafe fn country_invalidation(receiver: usize) -> Option<Invocation<'static>> {
    unsafe { FORWARDING.begin(Writer::CountryInvalidation, receiver) }
}
/// # Safety
/// As [`Forwarding::begin`], with `receiver` the invalidating call's live receiver.
pub(super) unsafe fn idea_invalidation(receiver: usize) -> Option<Invocation<'static>> {
    unsafe { FORWARDING.begin(Writer::IdeaInvalidation, receiver) }
}
/// # Safety
/// As [`Forwarding::begin`], for a content invalidation, which names no receiver.
pub(super) unsafe fn content_invalidation() -> Option<Invocation<'static>> {
    unsafe { FORWARDING.begin(Writer::ContentInvalidation, 0) }
}

#[cfg(test)]
mod tests;
