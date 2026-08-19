use compact_contract::domain_outcome::{
    read_predicate, AnswerExtent, DomainOutcome, ReadCompletion,
    READ_PREDICATE_READER_RETURNED_ROWS, READ_PREDICATE_SOURCE_ANSWERED_EMPTY,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal {
    pub reason: &'static str,
    pub detail: String,
}

impl Refusal {
    pub fn new(reason: &'static str, detail: impl Into<String>) -> Self {
        Self {
            reason,
            detail: detail.into(),
        }
    }

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

pub fn succeeded_outcome(domain_id: u16) -> DomainOutcome {
    let predicate = read_predicate(READ_PREDICATE_READER_RETURNED_ROWS)
        .expect("READ_PREDICATE_READER_RETURNED_ROWS is declared in READ_PREDICATES");
    DomainOutcome::succeeded(domain_id, predicate)
}

pub fn measured_empty_outcome(domain_id: u16, reason: impl Into<String>) -> DomainOutcome {
    let predicate = read_predicate(READ_PREDICATE_SOURCE_ANSWERED_EMPTY)
        .expect("READ_PREDICATE_SOURCE_ANSWERED_EMPTY is declared in READ_PREDICATES");
    DomainOutcome::measured_empty(domain_id, predicate, reason)
}

#[cfg(test)]
mod tests {
    use super::*;
    use compact_contract::domain_outcome::AnswerPresence;

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
