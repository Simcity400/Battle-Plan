//! The D18 country roster replay reads back out of the emitted `countries`
//! object, at COUNTRY grain.
//!
//! # Why this is its own module
//!
//! The view's fields are private so that a consumer has to say whether it needs
//! the WHOLE roster ([`D18CountryRosterView::complete`]) or can work per
//! country ([`D18CountryRosterView::partial`]). Private fields are visible to
//! the defining module and its descendants, and `world.rs` is a single ~50k-line
//! module that both builds the roster and consumes it in a dozen places — so
//! leaving the types there would have exempted exactly the call sites that most
//! needed the audit. Here, `world.rs` is a sibling and goes through the same
//! accessors as every other crate.

use crate::ViewStatus;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct D18CountryRosterEntry {
    pub tag: String,
    pub tag_db_id: u32,
}

/// Why a caller may not have the roster shape it asked for.
///
/// Carries the `ViewStatus` because most callers propagate one, and a `detail`
/// that NAMES the countries rather than saying "unavailable": a refusal that
/// cannot say which country it is about is a refusal nobody can act on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct D18RosterRefusal {
    pub status: ViewStatus,
    pub detail: String,
}

impl std::fmt::Display for D18RosterRefusal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.detail)
    }
}

/// The resolved half of a roster read, together with the countries it could not
/// resolve.
///
/// Both halves, always: a caller that takes this shape has said it can work per
/// country, and it is then obliged to carry `excluded` into whatever it
/// publishes — exclude AND account. Dropping it silently is the failure this
/// type exists to make impossible to do by accident.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct D18PartialRoster<'a> {
    pub resolved: &'a [D18CountryRosterEntry],
    /// The emitted `countries` keys whose D18 identity this hour did not
    /// resolve, ascending. The key is the country's ONLY name here — the fold
    /// keeps the numeric `tag_db_id` spelling rather than fabricating a TAG
    /// (`world_fold.rs::resolve_country_key`) — so it is what a refusal or a
    /// diagnostic must print.
    pub excluded: &'a [String],
}

/// The D18 country roster as replay reads it back out of the emitted
/// `countries` object.
///
/// # Per-entry, not all-or-nothing
///
/// The roster used to be one indivisible answer: a single country whose
/// `_r2_tag_db_id` the fold could not attach took the WHOLE roster to
/// `DomainUnavailable`, and with it every consumer of it. On `captures/oz_tip`
/// that cost 469 of 535 hours to one country (`367`) that owns 0 of 1,081
/// states and controls 0 of 10,272 provinces — measured in
/// `runs/pq_d18_roster/unresolved_key_census_base.json`.
///
/// A country the recorder could not name is a MISSING measurement, not a broken
/// one, so it is excluded and accounted rather than fatal. A malformed key — a
/// TAG outside `[A-Z0-9_]`, a duplicate numeric identity, a key with no typed
/// country — is a broken measurement and still refuses the whole view, because
/// excluding one would dress a malfunctioning read as absent data.
///
/// # Why the fields are private
///
/// Widening all-or-nothing without auditing each consumer is the "silently
/// stops checking" failure. A public `entries` would let every existing
/// `roster.entries` read keep compiling while quietly becoming a read of a
/// SHORTER list, so the two shapes are behind [`D18CountryRosterView::complete`]
/// and [`D18CountryRosterView::partial`] and a caller has to say which one it
/// means.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct D18CountryRosterView {
    status: ViewStatus,
    entries: Vec<D18CountryRosterEntry>,
    excluded: Vec<String>,
    diagnostic: Option<String>,
}

impl D18CountryRosterView {
    /// A roster read that produced at least one resolved country, plus the
    /// countries it could not resolve.
    pub fn present(entries: Vec<D18CountryRosterEntry>, excluded: Vec<String>) -> Self {
        D18CountryRosterView {
            status: ViewStatus::Present,
            entries,
            excluded,
            diagnostic: None,
        }
    }

    /// A roster read that produced nothing usable, and why.
    pub fn refused(status: ViewStatus, diagnostic: impl Into<String>) -> Self {
        D18CountryRosterView {
            status,
            entries: Vec::new(),
            excluded: Vec::new(),
            diagnostic: Some(diagnostic.into()),
        }
    }

    /// The whole roster, or a named refusal.
    ///
    /// For consumers whose claim is about EVERY country: an all-country producer
    /// census compared row for row, a whole-roster digest, an absence claim
    /// ("these are all of them"). A partial roster cannot back any of those, and
    /// the refusal names the countries that are missing so the reason is
    /// actionable rather than "the roster is unavailable".
    pub fn complete(&self) -> std::result::Result<&[D18CountryRosterEntry], D18RosterRefusal> {
        if self.status != ViewStatus::Present {
            return Err(D18RosterRefusal {
                status: self.status,
                detail: self
                    .diagnostic
                    .clone()
                    .unwrap_or_else(|| format!("the D18 country roster is {:?}", self.status)),
            });
        }
        if !self.excluded.is_empty() {
            return Err(D18RosterRefusal {
                status: ViewStatus::DomainUnavailable,
                detail: format!(
                    "the D18 country roster is not complete: {} of {} emitted country key(s) \
                     carry no D18 identity this hour ({})",
                    self.excluded.len(),
                    self.excluded.len() + self.entries.len(),
                    crate::world::name_excluded_keys(&self.excluded)
                ),
            });
        }
        if self.entries.is_empty() {
            return Err(D18RosterRefusal {
                status: ViewStatus::DomainUnavailable,
                detail: "the D18 country roster is empty".into(),
            });
        }
        Ok(&self.entries)
    }

    /// The countries that DID resolve, plus the ones that did not.
    ///
    /// For consumers that are honest per country: a per-country lookup, a
    /// per-country fill, a list the viewer shows. `Err` only when the read
    /// itself produced nothing — a non-`Present` status, or every emitted key
    /// unresolved, which is a roster rather than a partial.
    pub fn partial(&self) -> std::result::Result<D18PartialRoster<'_>, D18RosterRefusal> {
        if self.status != ViewStatus::Present {
            return Err(D18RosterRefusal {
                status: self.status,
                detail: self
                    .diagnostic
                    .clone()
                    .unwrap_or_else(|| format!("the D18 country roster is {:?}", self.status)),
            });
        }
        if self.entries.is_empty() {
            return Err(D18RosterRefusal {
                status: ViewStatus::DomainUnavailable,
                detail: "the D18 country roster resolved no country this hour".into(),
            });
        }
        Ok(D18PartialRoster {
            resolved: &self.entries,
            excluded: &self.excluded,
        })
    }

    /// `Present` only when the roster covers every country the frame carries.
    ///
    /// This is what the XMAP00 ledger row `country.d18_complete_roster` means by
    /// its own name, so it is derived from [`D18CountryRosterView::complete`]
    /// rather than from the read's status: a roster with an excluded country is
    /// a roster, but it is not a COMPLETE one, and a ledger row that stopped
    /// distinguishing the two would be the defect this split exists to avoid.
    pub fn complete_status(&self) -> ViewStatus {
        match self.complete() {
            Ok(_) => ViewStatus::Present,
            Err(refusal) => refusal.status,
        }
    }

    /// Whether the READ produced a usable per-country roster, regardless of
    /// completeness. Use [`D18CountryRosterView::complete_status`] for the
    /// completeness question.
    pub fn read_status(&self) -> ViewStatus {
        self.status
    }

    /// Why the read produced nothing, when it produced nothing.
    pub fn diagnostic(&self) -> Option<&str> {
        self.diagnostic.as_deref()
    }
}
