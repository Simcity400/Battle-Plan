//!   `release_if_custody_matches(None)` returns the record inside an `Err` a

use std::fmt;
use std::marker::PhantomData;

pub enum Install {}

pub enum Restore {}

pub trait CasDirection {
    const DIRECTION: &'static str;
}

impl CasDirection for Install {
    const DIRECTION: &'static str = "install";
}

impl CasDirection for Restore {
    const DIRECTION: &'static str = "restore";
}

pub struct RecordedCas<D> {
    observed_before: usize,
    protection_before: u32,
    protection_temporary: u32,
    protection_reconciled: bool,
    direction: PhantomData<D>,
}

impl<D> RecordedCas<D> {
    pub fn observed_before(&self) -> usize {
        self.observed_before
    }

    pub fn protection_before(&self) -> u32 {
        self.protection_before
    }

    pub fn protection_temporary(&self) -> u32 {
        self.protection_temporary
    }

    pub fn protection_reconciled(&self) -> bool {
        self.protection_reconciled
    }
}

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
            .field(
                "observed_before",
                &format_args!("0x{:X}", self.observed_before),
            )
            .field(
                "protection_before",
                &format_args!("0x{:X}", self.protection_before),
            )
            .field(
                "protection_temporary",
                &format_args!("0x{:X}", self.protection_temporary),
            )
            .field("protection_reconciled", &self.protection_reconciled)
            .finish()
    }
}

pub struct CasCustody<D> {
    record: Option<RecordedCas<D>>,
}

impl<D> CasCustody<D> {
    pub fn unrecorded() -> Self {
        Self { record: None }
    }

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

#[must_use = "an unconfirmed CAS record must be checked against the caller's durable custody"]
pub struct UnconfirmedCas<D>(RecordedCas<D>);

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
pub struct CasRetentionMismatch<D> {
    pub retained: Option<RecordedCas<D>>,
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
    custody.record = Some(recorded);
    let protection_after = finish(prepared)?;
    let reconciled = RecordedCas {
        protection_reconciled: protection_after == protection_before,
        ..recorded
    };
    custody.record = Some(reconciled);
    Ok((UnconfirmedCas(reconciled), protection_after))
}

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
