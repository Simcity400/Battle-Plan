//! The named refusal, and its bridge to the uniform domain contract.
//!
//! Every law in this crate returns `Result<T, Refusal>`. A `Refusal` carries a
//! stable machine reason (the name that goes on the wire and into the gap
//! ledger) and the producer's own text. Nothing in this crate returns a
//! defaulted value, an empty collection standing in for a failed read, or a
//! bare `None` that a caller could mistake for "measured nothing".
//!
//! [`Refusal::into_domain_outcome`] is the only bridge to
//! `compact_contract::domain_outcome::DomainOutcome`, so the uniform contract
//! is satisfied at exactly one place and a site cannot hand-roll a status bit.

use compact_contract::domain_outcome::{
    read_predicate, AnswerExtent, DomainOutcome, ReadCompletion,
    READ_PREDICATE_READER_RETURNED_ROWS, READ_PREDICATE_SOURCE_ANSWERED_EMPTY,
};

/// A read that did not produce an answer, and the name of why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal {
    /// Stable machine-readable reason. Dotted, lower-case, never localised.
    pub reason: &'static str,
    /// The producer's own text: values, addresses, which link failed.
    pub detail: String,
}

impl Refusal {
    pub fn new(reason: &'static str, detail: impl Into<String>) -> Self {
        Self {
            reason,
            detail: detail.into(),
        }
    }

    /// The uniform-contract outcome for this refusal.
    ///
    /// Always `DidNotComplete` + `Unprovable`: a chain that refused part-way
    /// neither finished nor can prove what it would have seen. It is never
    /// `measured_empty` — that constructor exists for a source that answered,
    /// and a refused chain has no answer to call empty.
    pub fn into_domain_outcome(self, domain_id: u16) -> DomainOutcome {
        let predicate = read_predicate(READ_PREDICATE_READER_RETURNED_ROWS)
            .expect("READ_PREDICATE_READER_RETURNED_ROWS is declared in READ_PREDICATES");
        DomainOutcome::failed(
            domain_id,
            predicate,
            ReadCompletion::DidNotComplete,
            AnswerExtent::Unprovable,
            format!("{}: {}", self.reason, self.detail),
        )
    }
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.reason, self.detail)
    }
}

/// The outcome for a completed, exhaustive read that carries rows.
pub fn succeeded_outcome(domain_id: u16) -> DomainOutcome {
    let predicate = read_predicate(READ_PREDICATE_READER_RETURNED_ROWS)
        .expect("READ_PREDICATE_READER_RETURNED_ROWS is declared in READ_PREDICATES");
    DomainOutcome::succeeded(domain_id, predicate)
}

/// The outcome for a source that answered with no rows.
///
/// Distinct from a refusal on purpose: an empty faction roster is a fact about
/// the save, and publishing it as `SOURCE_UNAVAILABLE` is the defect
/// `domain_outcome` exists to remove.
pub fn measured_empty_outcome(domain_id: u16, reason: impl Into<String>) -> DomainOutcome {
    let predicate = read_predicate(READ_PREDICATE_SOURCE_ANSWERED_EMPTY)
        .expect("READ_PREDICATE_SOURCE_ANSWERED_EMPTY is declared in READ_PREDICATES");
    DomainOutcome::measured_empty(domain_id, predicate, reason)
}

#[cfg(test)]
mod tests {
    use super::*;
    use compact_contract::domain_outcome::AnswerPresence;

    /// kills: mapping a refused chain onto `measured_empty`, which would make
    /// an unreadable anchor indistinguishable from a save with no factions.
    #[test]
    fn a_refusal_never_becomes_a_measured_empty_answer() {
        let outcome = Refusal::new("x.y", "z").into_domain_outcome(17);
        assert_eq!(outcome.completion, ReadCompletion::DidNotComplete);
        assert_eq!(outcome.extent, AnswerExtent::Unprovable);
        assert_eq!(outcome.presence, AnswerPresence::Present);
        assert!(outcome.reason.contains("x.y"));

        let empty = measured_empty_outcome(17, "roster answered with no factions");
        assert_eq!(empty.presence, AnswerPresence::Empty);
        assert_eq!(empty.completion, ReadCompletion::Completed);
    }
}
