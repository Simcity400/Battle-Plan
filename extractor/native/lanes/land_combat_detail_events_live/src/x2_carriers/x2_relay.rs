//! The relay representation, walled off so that nothing outside this module can
//! name its fields.
//!
//! # What the boundary buys, stated exactly
//!
//! It is a module rather than a plain type because Rust privacy is scoped to the
//! module, and `x2_carriers` is one file-sized module. A review of the first
//! version of this change demonstrated the consequence: a single added line in
//! `abort_width_install`,
//!
//! ```ignore
//! let _ = WIDTH_PRODUCER_RELAY.trampoline_cell.swap(0, Ordering::AcqRel);
//! ```
//!
//! re-created the whole late-entrant defect on an aborted install and passed all
//! 64 tests -- the behavioural guard never runs `abort_width_install`, and the
//! source law of the day was scanning for one spelling. Behind this boundary
//! that line does not compile.
//!
//! What the compiler enforces is ONLY that: no code outside this module can
//! write, read or alias these fields THROUGH FIELD SYNTAX, because it cannot
//! name them. It enforces nothing against a reach that never names them -- a
//! transmute, or pointer arithmetic from the struct base -- and it does NOT
//! enforce that there is one writer. A second writer added INSIDE this module
//! compiles perfectly well, and a review round produced one that even kept the
//! old counting law green by aliasing the field into a local first. Keeping the
//! writer single is `no_x2_path_takes_a_relay_address_away`'s job, not the
//! compiler's, and that test states its own limits.

use std::sync::atomic::{AtomicU64, AtomicU8, AtomicUsize, Ordering};

/// Which of three states one detour site is in.
///
/// # Why this type exists
///
/// These three states used to share one `AtomicUsize` whose ZERO was asked to
/// mean all of them at once:
///
/// * the detours read zero as "not installed, do not call the engine";
/// * teardown WROTE zero to say "this site is clean again";
/// * the install preflights read NON-zero as "custody left over from an install
///   that did not finish".
///
/// One null cannot answer three questions, and the collision was not cosmetic.
/// Because the preflights demanded a zero to accept a fresh install, teardown had
/// to write one, and that write is what made every detour's early return
/// reachable on a live game: a thread that had already branched into a detour and
/// not yet incremented its callback counter -- the exact entrant the retained
/// trampoline exists for, invisible to quiescence by construction -- would load
/// the freshly zeroed pointer and return WITHOUT calling the engine's own
/// function. The game then computed something the unhooked game would not have.
/// That is a mutated simulation, not a lost row.
///
/// Splitting the states apart is what lets teardown stop writing that zero. See
/// [`RelaySlot`] for the representation and [`retain_width_relays_at_teardown`]
/// for what teardown does instead.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum RelayCustody {
    /// No trampoline has ever been created for this site IN THIS DLL MAPPING.
    ///
    /// The recorded address is zero because there is nothing to record. A detour
    /// entered in this state cannot call the engine's own function -- a null is
    /// not callable and no fallback address exists, because the retained address
    /// and the live address are the same field. It is reached only through the
    /// unload-and-rebase case named on [`RelaySlot::original_for_detour`].
    NeverCreated = 0,
    /// A trampoline exists, this mapping holds its address, and no installation
    /// owns it. This is what a COMPLETED teardown leaves behind.
    ///
    /// A detour entered in this state still reaches the engine's own function.
    /// An install may adopt the entry rather than failing on
    /// `MH_ERROR_ALREADY_CREATED`.
    Retained = 1,
    /// A trampoline exists and an installation owns it. Outside a live install
    /// this is the leftover of an install that did not finish, which is the fact
    /// the preflights are actually looking for.
    Live = 2,
}

impl RelayCustody {
    pub(super) const fn as_u8(self) -> u8 {
        self as u8
    }

    pub(super) fn from_u8(raw: u8) -> Self {
        match raw {
            1 => Self::Retained,
            2 => Self::Live,
            _ => Self::NeverCreated,
        }
    }
}

/// One detour site's trampoline address and custody state, held together.
///
/// The address and the state are one object because they are one fact. Keeping
/// them in separate statics is what allowed teardown to move one without the
/// other.
pub(super) struct RelaySlot {
    /// Names this site in every error this slot raises.
    ///
    /// Held HERE rather than passed in at each call. The previous lane's audit
    /// found a mutation surviving because a roster's owner token could be
    /// replaced by a constant and no offline test noticed -- that wiring only
    /// runs in a live teardown. A label that travels with the slot cannot be
    /// paired with another site's slot at all, so that shape is closed by
    /// construction here rather than by a scan.
    label: &'static str,
    /// The trampoline address MinHook returned when THIS DLL MAPPING created the
    /// entry.
    ///
    /// Written once per mapping by [`Self::record_created`] and never cleared.
    ///
    /// Two different things hold that together, and they are not the same
    /// strength. The compiler guarantees that no code OUTSIDE `mod relay`
    /// can NAME this field, so no field-syntax access to it can be written
    /// there -- that is what the module boundary is for, and an earlier
    /// version of this type sat directly in `x2_carriers`, where "private"
    /// meant "private to the whole file" and a one-line `swap(0)` in
    /// `abort_width_install` re-created the defect with every test green.
    /// It guarantees nothing about a reach that never names the field, such
    /// as a transmute or pointer arithmetic from the struct base. Inside the
    /// module, nothing but `no_x2_path_takes_a_relay_address_away` stops a
    /// second writer being added, and that test says how far it reaches.
    ///
    /// The name is deliberately not an ordinary word: the law counts every
    /// lexical occurrence of it, which only works if the token cannot
    /// collide with prose or with `original_for_detour`.
    trampoline_cell: AtomicUsize,
    custody: AtomicU8,
    /// Detour invocations that returned without calling the engine's own
    /// function because this mapping holds no trampoline address. See
    /// [`Self::original_for_detour`].
    engine_original_skipped: AtomicU64,
}

impl RelaySlot {
    pub(super) const fn new(label: &'static str) -> Self {
        Self {
            label,
            trampoline_cell: AtomicUsize::new(0),
            custody: AtomicU8::new(RelayCustody::NeverCreated.as_u8()),
            engine_original_skipped: AtomicU64::new(0),
        }
    }

    pub(super) fn custody(&self) -> RelayCustody {
        RelayCustody::from_u8(self.custody.load(Ordering::Acquire))
    }

    /// Whether an installation still owns this site.
    ///
    /// This -- not "the pointer is non-zero" -- is what the install preflights
    /// and the aborted-receipt archives ask. A [`RelayCustody::Retained`] site
    /// is a finished teardown, not leftover custody, and must not be treated as
    /// one.
    pub(super) fn is_live(&self) -> bool {
        self.custody() == RelayCustody::Live
    }

    /// The address the detour calls, or `None` when this mapping never created a
    /// trampoline for this site.
    ///
    /// # The `None` case is a POISONED capture, not a skipped one
    ///
    /// Returning `None` makes the caller return a fabricated value to the ENGINE
    /// without running the engine's own function, so that call computes
    /// something the unhooked game would not have. There is no alternative once
    /// there is no address: a null is not callable, and there is no second
    /// address to fall back to -- the retained address and the live address are
    /// the same field precisely so that no path can have one without the other.
    /// So the answer is to make the state unreachable, and to count it loudly if
    /// it is ever reached. The count sets `poisoned` on the lane's
    /// `InstallAbortDiscard`; it is deliberately NOT folded into a discarded-row
    /// total, because a changed simulation is not a lost row. This is the same
    /// answer, for the same reason, as `fow_visibility_live::a729`'s
    /// `ENGINE_ORIGINAL_SKIPPED_COUNT`.
    ///
    /// # When it can be reached
    ///
    /// Unreachable WHILE THIS DLL MAPPING REMAINS LOADED: the address is stored
    /// before the target is ever patched, and no path clears it -- teardown
    /// retains. It is NOT unreachable full stop. The collector unloads this DLL
    /// with `FreeLibraryAndExitThread`, and that unload knows nothing about
    /// whether a thread is poised inside a detour body. If a later injection
    /// reuses the image base, that thread resumes in the NEW mapping's detour
    /// against a fresh [`RelayCustody::NeverCreated`] slot and lands here. That
    /// hazard is not created by retention and is not closed by it; it is shared
    /// with every inline-detour lane in this tree and is named the same way on
    /// `fow_visibility_live::a729::retain_and_disable`.
    ///
    /// # Fast path cost, measured
    ///
    /// Zero added instructions on the path that calls the engine. This was read
    /// out of the release assembly (`cargo rustc --release -- --emit asm`) for
    /// all seven detours, before and after -- not argued from the source.
    ///
    /// * The load is the same single instruction it was:
    ///   `movq ORIGINAL_PANEL_LEADER_RESOLVER(%rip), %rax` became
    ///   `movq PANEL_LEADER_RESOLVER_RELAY+16(%rip), %rax`. A RIP-relative
    ///   `movq` either way -- an acquire load is a plain `mov` on x86-64 and the
    ///   field offset is a link-time constant, so folding the address into a
    ///   struct costs a displacement, not an instruction. That substitution is
    ///   the whole of the change in all seven.
    /// * The `Option` inlines away; the test-and-branch after it is unchanged.
    /// * The only instructions ADDED anywhere are the
    ///   `lock incq <RELAY>+24(%rip)` for this counter, and every one of them
    ///   sits on the branch that returns WITHOUT calling the engine.
    /// * Two incidental differences, neither a cost: one `xorl` moved a slot
    ///   later in the schedule, and a spill on the unwind path narrowed from
    ///   `movq %r14, -8(%rbp)` to `movl %r14d, -4(%rbp)`.
    ///
    /// The counter shares a cache line with the address it guards. That is
    /// deliberate and harmless: it is written only on the branch that never runs
    /// while this mapping is loaded, so no detour dirties a line another detour
    /// reads from.
    #[inline]
    pub(super) fn original_for_detour(&self) -> Option<usize> {
        match self.trampoline_cell.load(Ordering::Acquire) {
            0 => {
                self.engine_original_skipped.fetch_add(1, Ordering::AcqRel);
                None
            }
            original => Some(original),
        }
    }

    /// Record the trampoline MinHook just returned and take live custody.
    ///
    /// A site whose address is already recorded may only be re-recorded with the
    /// SAME address. Nothing in this lane frees a trampoline, so MinHook cannot
    /// hand back a different one for the same target; a disagreement means the
    /// address this mapping would install against is not the one its detour
    /// would call, and that is refused rather than overwritten.
    pub(super) fn record_created(&self, trampoline: usize) -> Result<usize, String> {
        if trampoline == 0 {
            return Err(format!(
                "{} hook creation returned a null trampoline",
                self.label
            ));
        }
        match self.trampoline_cell.compare_exchange(
            0,
            trampoline,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => {}
            Err(existing) if existing == trampoline => {}
            Err(existing) => {
                return Err(format!(
                    "{} hook creation returned trampoline 0x{trampoline:X} but this process \
                     already recorded 0x{existing:X}",
                    self.label
                ))
            }
        }
        self.custody
            .store(RelayCustody::Live.as_u8(), Ordering::Release);
        Ok(trampoline)
    }

    /// Take live custody of the entry an earlier capture in this mapping created.
    ///
    /// Fails closed when nothing was recorded: installing without a trampoline is
    /// exactly the simulation-mutating outcome above.
    pub(super) fn adopt_retained(&self) -> Result<usize, String> {
        match self.trampoline_cell.load(Ordering::Acquire) {
            0 => Err(format!(
                "{} hook is already created but this process retained no trampoline for it",
                self.label
            )),
            trampoline => {
                self.custody
                    .store(RelayCustody::Live.as_u8(), Ordering::Release);
                Ok(trampoline)
            }
        }
    }

    /// Give up live custody, keeping the address.
    ///
    /// This is the whole of what teardown does to a relay, and the whole of what
    /// changed: the pointer the detours call is left alone. A site that was never
    /// created stays [`RelayCustody::NeverCreated`], so a rollback of a create
    /// that failed does not invent a retention nobody performed.
    pub(super) fn retain_at_teardown(&self) {
        let _ = self.custody.compare_exchange(
            RelayCustody::Live.as_u8(),
            RelayCustody::Retained.as_u8(),
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }

    /// The site's own name, for the errors this slot raises. Reading it is
    /// safe from anywhere; the address is what the module boundary protects.
    pub(super) fn label(&self) -> &'static str {
        self.label
    }

    pub(super) fn engine_original_skipped(&self) -> u64 {
        self.engine_original_skipped.load(Ordering::Acquire)
    }

    pub(super) fn recorded_original(&self) -> usize {
        self.trampoline_cell.load(Ordering::Acquire)
    }
}
