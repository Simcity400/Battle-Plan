//! The only place in this crate where a custody record can come into existence.
//!
//! A custody record attests one atomic pointer-slot compare-exchange: the value
//! the hardware exchange returned, and the page protections around it. Those
//! values are copied verbatim into a promotion receipt, so a record holding a
//! *synthesized* `observed_before` is a forged attestation -- the exact defect
//! that was fixed on the restore path and then again, independently, on the
//! install path.
//!
//! Both of those fixes left the same hole. The record was an ordinary struct
//! with public fields, so
//!
//! ```text
//! RecordedInstallCas { observed_before: expected.expected_original_address, .. }
//! ```
//!
//! still compiled anywhere in the crate, still reached the receipt, and no test
//! could tell it from a real one. That is not a hole a test can close: the
//! forged value is indistinguishable from the honest one at the point a test
//! can look at it.
//!
//! This module closes it by construction instead. [`RecordedCas`]'s fields are
//! private and it has no public constructor, no `Default`, no `From`, and no
//! struct-update path out of this file, so **outside this module there is no
//! expression in safe Rust that produces a `RecordedCas`**. The only producers
//! are [`perform_recorded_cas`], which takes `observed_before` straight from the
//! return value of the `exchange` closure it has just run, and
//! [`reconcile_recorded_cas`], which can only refine a record that already
//! exists and never touches `observed_before`.
//!
//! Safe Rust, precisely: a `transmute` or a `MaybeUninit` write could still
//! manufacture one, as it could for any private-field type. This crate contains
//! no such construction and the claim is not that one is impossible -- it is
//! that a forged record is no longer something an ordinary edit can express by
//! accident or in passing.
//!
//! The review question for the rest of the crate therefore collapses to "does
//! this file fabricate a field?", and this file is short enough to answer it.
//!
//! # Directions
//!
//! Install (original -> shim) and restore (shim -> original) swap the roles of
//! the two addresses, so a record of one direction is not evidence for the
//! other. They stay distinct *types* -- `RecordedCas<Install>` will not coerce
//! to `RecordedCas<Restore>`, and neither will their custody cells -- while
//! sharing a single implementation, so the two directions cannot drift apart
//! the way two hand-copied modules can. Previous work on this file explicitly
//! declined a fix because "changing one side alone would desynchronise the two
//! directions"; here there is only one side to change.
//!
//! # What this module does not close
//!
//! [`perform_recorded_cas`] writes through the `&mut CasCustody` it is handed.
//! Safe Rust cannot express "this borrow points into the registry entry the
//! receipt will be minted from", so a caller can still hand over a leaked,
//! throwaway cell (`Box::leak(Box::new(CasCustody::unrecorded()))`) and lose the
//! evidence. That is a genuine residual, disclosed rather than closed.
//!
//! What bounds it is that it is fail-closed. The write goes nowhere the next
//! attempt reads, so the next attempt finds no record and refuses the transition
//! as unwitnessed rather than attesting it. A misdirected write costs a stranded
//! lease; it cannot produce a receipt for a transition nobody witnessed.
//!
//! [`perform_recorded_cas`] and [`reconcile_recorded_cas`] hand back an
//! [`UnconfirmedCas`] rather than a [`RecordedCas`], so the value they return is
//! unusable until it has been checked against an observation of the caller's own
//! storage. **That makes the check the shortest path, not the only one.** Two
//! ways around it compile today, both measured:
//!
//! * [`CasCustody::recorded`] hands back a plain [`RecordedCas`], so a caller can
//!   drop the `UnconfirmedCas` and read an equivalent record straight back out of
//!   whichever cell it passed in. This cannot be closed here: reading a record
//!   out of durable custody is exactly how a legitimate retry replays an earlier
//!   attempt, so that path has to exist.
//! * [`CasRetentionMismatch::performed`] is public, so
//!   `release_if_custody_matches(None)` returns the record inside an `Err` a
//!   caller can unwrap. Left public because it carries the diagnostic the refusal
//!   message needs, and closing it alone would buy nothing while the first route
//!   is open.
//!
//! Neither route forges anything -- the record still came from an `exchange`
//! closure -- so what is at stake is the retention check, not the attestation.
//! Nothing in this module ties a record to the particular cell it was written
//! into either: `reconcile_recorded_cas` will refine a record from one cell into
//! a different cell of the same direction. Production never does, because the
//! record comes from the binding the call has just matched.

use std::fmt;
use std::marker::PhantomData;

/// The original -> shim direction.
///
/// Uninhabited on purpose: it is a type-level tag, never a value.
pub enum Install {}

/// The shim -> original direction.
///
/// Uninhabited on purpose: it is a type-level tag, never a value.
pub enum Restore {}

/// Names a direction so `Debug` output says which one a record belongs to.
pub trait CasDirection {
    const DIRECTION: &'static str;
}

impl CasDirection for Install {
    const DIRECTION: &'static str = "install";
}

impl CasDirection for Restore {
    const DIRECTION: &'static str = "restore";
}

/// What this crate recorded about a compare-exchange it performed.
///
/// Written the instant the exchange returns and *before* any fallible
/// page-protection cleanup, so a failure during that cleanup leaves evidence of
/// the transition behind instead of losing it.
///
/// Every field is private and every constructor lives in this module. A caller
/// that wants a record has to perform an exchange to get one.
pub struct RecordedCas<D> {
    /// The value the compare-exchange actually returned. Never derived from
    /// what the caller expected to see.
    observed_before: usize,
    /// Page protection observed before the exchange.
    protection_before: u32,
    /// Protection the exchange temporarily installed. Equals
    /// `protection_before` when the page was already writable.
    protection_temporary: u32,
    /// True only where the page protection has been *observed* back at
    /// `protection_before`. Set from a comparison against an observation, never
    /// asserted.
    protection_reconciled: bool,
    direction: PhantomData<D>,
}

impl<D> RecordedCas<D> {
    /// The value the compare-exchange returned.
    pub fn observed_before(&self) -> usize {
        self.observed_before
    }

    /// Page protection observed before the exchange.
    pub fn protection_before(&self) -> u32 {
        self.protection_before
    }

    /// Protection the exchange temporarily installed.
    pub fn protection_temporary(&self) -> u32 {
        self.protection_temporary
    }

    /// Whether the page protection has been observed back at
    /// [`Self::protection_before`].
    pub fn protection_reconciled(&self) -> bool {
        self.protection_reconciled
    }
}

// Hand-written rather than derived: `#[derive]` on a generic struct would
// demand `D: Clone + Copy + Eq + Debug`, and the direction tags are
// uninhabited. `D` is a `PhantomData` tag with no runtime content, so none of
// these impls can depend on it.
impl<D> Clone for RecordedCas<D> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<D> Copy for RecordedCas<D> {}

impl<D> PartialEq for RecordedCas<D> {
    fn eq(&self, other: &Self) -> bool {
        self.observed_before == other.observed_before
            && self.protection_before == other.protection_before
            && self.protection_temporary == other.protection_temporary
            && self.protection_reconciled == other.protection_reconciled
    }
}

impl<D> Eq for RecordedCas<D> {}

impl<D: CasDirection> fmt::Debug for RecordedCas<D> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RecordedCas")
            .field("direction", &D::DIRECTION)
            .field("observed_before", &format_args!("0x{:X}", self.observed_before))
            .field("protection_before", &format_args!("0x{:X}", self.protection_before))
            .field(
                "protection_temporary",
                &format_args!("0x{:X}", self.protection_temporary),
            )
            .field("protection_reconciled", &self.protection_reconciled)
            .finish()
    }
}

/// The durable slot one direction's record lives in.
///
/// Held by value inside the custody binding the registry owns. There is no
/// accessor trait and no `&mut Option<RecordedCas<_>>` in any public signature:
/// an implementor-supplied accessor was the previous shape, and its residual
/// hole was that an implementation could return a borrow of something other
/// than its own field. With no accessor there is no such implementation to
/// write.
pub struct CasCustody<D> {
    record: Option<RecordedCas<D>>,
}

impl<D> CasCustody<D> {
    /// A cell for a slot this crate has performed no exchange on yet.
    ///
    /// This is the *only* way to make a cell, and it can only make an empty
    /// one, so a binding cannot be created pre-loaded with custody it did not
    /// earn.
    pub fn unrecorded() -> Self {
        Self { record: None }
    }

    /// What this crate recorded for the slot, if anything.
    pub fn recorded(&self) -> Option<RecordedCas<D>> {
        self.record
    }
}

impl<D> Clone for CasCustody<D> {
    fn clone(&self) -> Self {
        Self {
            record: self.record,
        }
    }
}

impl<D> PartialEq for CasCustody<D> {
    fn eq(&self, other: &Self) -> bool {
        self.record == other.record
    }
}

impl<D> Eq for CasCustody<D> {}

impl<D: CasDirection> fmt::Debug for CasCustody<D> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CasCustody")
            .field("direction", &D::DIRECTION)
            .field("record", &self.record)
            .finish()
    }
}

/// A record an exchange in *this* call produced, whose landing in the caller's
/// durable custody has not been checked.
///
/// The ordering helpers return one of these rather than a bare [`RecordedCas`]
/// so that the value in hand is unusable until it has been put next to an
/// observation of the storage the helper was asked to write into. Using the
/// returned value at all therefore means checking it, and simply deleting the
/// check does not compile.
///
/// It is a nudge with teeth, not a proof. It does not make a misdirected write
/// impossible, and it does not make the check unavoidable -- a caller determined
/// to skip it can read an equivalent record back out of [`CasCustody::recorded`]
/// or out of [`CasRetentionMismatch::performed`]. Both routes are spelled out in
/// the module docs; neither can forge what the record says.
#[must_use = "an unconfirmed CAS record must be checked against the caller's durable custody"]
pub struct UnconfirmedCas<D>(RecordedCas<D>);

// Comparison and formatting only -- neither can produce a record, so neither
// widens what this module lets the rest of the crate express.
impl<D> PartialEq for UnconfirmedCas<D> {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl<D> Eq for UnconfirmedCas<D> {}

impl<D: CasDirection> fmt::Debug for UnconfirmedCas<D> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("UnconfirmedCas").field(&self.0).finish()
    }
}

/// What [`UnconfirmedCas::release_if_custody_matches`] was shown instead of the
/// record.
pub struct CasRetentionMismatch<D> {
    /// What the caller's durable custody actually reads.
    pub retained: Option<RecordedCas<D>>,
    /// The record the exchange in this call produced.
    pub performed: RecordedCas<D>,
}

impl<D: CasDirection> fmt::Debug for CasRetentionMismatch<D> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CasRetentionMismatch")
            .field("retained", &self.retained)
            .field("performed", &self.performed)
            .finish()
    }
}

impl<D> UnconfirmedCas<D> {
    /// Release the record if the custody the caller observed holds it.
    ///
    /// The name says only what this establishes: that the record matches the
    /// `observed_custody` value it was *given*. It cannot establish where that
    /// value came from. Passing back the same cell the helper wrote into makes
    /// the call vacuous, and nothing here can detect that -- the production call
    /// sites earn the check by re-reading the custody registry through
    /// `confirm_provisional_install_custody_retained` and its restore twin, and
    /// that re-read is where the real assurance lives.
    pub fn release_if_custody_matches(
        self,
        observed_custody: Option<RecordedCas<D>>,
    ) -> Result<RecordedCas<D>, CasRetentionMismatch<D>> {
        if observed_custody != Some(self.0) {
            return Err(CasRetentionMismatch {
                retained: observed_custody,
                performed: self.0,
            });
        }
        Ok(self.0)
    }
}

/// Sequence one compare-exchange so its result is persisted before anything
/// that can fail runs after it.
///
/// `prepare` holds everything fallible that must precede the exchange,
/// `exchange` is infallible by construction, the record is persisted, and only
/// then does `finish` run the fallible cleanup. Nothing fallible sits between
/// the exchange and the record, so a completed exchange can never be lost: a
/// `finish` failure leaves behind evidence that the transition happened and
/// that the page protection is still owed.
///
/// This is the sole producer of a `RecordedCas`, and `observed_before` is the
/// unmodified return value of `exchange`.
pub fn perform_recorded_cas<D, P, E>(
    prepare: impl FnOnce() -> Result<(P, u32, u32), E>,
    exchange: impl FnOnce(&P) -> usize,
    custody: &mut CasCustody<D>,
    finish: impl FnOnce(P) -> Result<u32, E>,
) -> Result<(UnconfirmedCas<D>, u32), E> {
    let (prepared, protection_before, protection_temporary) = prepare()?;
    let observed_before = exchange(&prepared);
    let recorded = RecordedCas {
        observed_before,
        protection_before,
        protection_temporary,
        protection_reconciled: false,
        direction: PhantomData,
    };
    // Between the exchange above and this line there is nothing fallible.
    custody.record = Some(recorded);
    let protection_after = finish(prepared)?;
    // `protection_reconciled` is set from an observation, not asserted: if the
    // cleanup returned some other protection, the record keeps saying the page
    // is still owed and the completion check will refuse it.
    let reconciled = RecordedCas {
        protection_reconciled: protection_after == protection_before,
        ..recorded
    };
    custody.record = Some(reconciled);
    Ok((UnconfirmedCas(reconciled), protection_after))
}

/// Finish the page-protection reconciliation a failed cleanup left owed, and
/// refine the record with what was observed.
///
/// `observed_before` is carried through untouched -- this cannot change what
/// transition the record attests, only whether the page is back where the
/// exchange found it. `reconcile` is handed the protection the exchange started
/// from and returns the protection observed afterwards.
pub fn reconcile_recorded_cas<D, E>(
    recorded: RecordedCas<D>,
    custody: &mut CasCustody<D>,
    reconcile: impl FnOnce(u32) -> Result<u32, E>,
) -> Result<(UnconfirmedCas<D>, u32), E> {
    let protection_after = reconcile(recorded.protection_before)?;
    let reconciled = RecordedCas {
        protection_reconciled: protection_after == recorded.protection_before,
        ..recorded
    };
    custody.record = Some(reconciled);
    Ok((UnconfirmedCas(reconciled), protection_after))
}
