//! Independent off-thread live demand; hints never certify recorded rows.

use super::*;
use crate::country_scale::{DemandFailures, raise, rows};

static FAILURES: [DemandFailures; 13] = [const { DemandFailures::new() }; 13];

pub(crate) fn status() -> impl serde::Serialize {
    crate::status::map(
        [
            "relations",
            "subjects",
            "autonomy",
            "agencies",
            "operatives",
            "captured_operatives",
            "cryptology",
            "intel_networks",
            "script_flags",
            "script_variables",
            "operations",
            "operation_tokens",
            "incoming_actions",
        ]
        .into_iter()
        .zip(FAILURES.iter().map(DemandFailures::diagnostics)),
    )
}

impl Capacities {
    pub(crate) fn presize(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
    ) -> Result<(), &'static str> {
        let mut failure = FAILURES[0]
            .observe(self.presize_relations(memory, base, state))
            .err();
        macro_rules! count {
            ($family:literal, $index:literal, $outcome:expr) => {{
                match FAILURES[$family].observe(count($outcome)) {
                    Ok(count) => {
                        raise(&mut self.rows[$index], rows(count, 1));
                    }
                    Err(reason) => {
                        failure.get_or_insert(reason);
                    }
                }
            }};
        }
        let subjects = csubject_relation::walk(memory, base, state, &mut [], &mut []);
        count!(1, 1, subjects.outcome);
        raise(&mut self.subject_names, rows(subjects.required_bytes, 1));
        let autonomy = ccurrent_autonomy_status::walk(memory, base, state, &mut [], &mut []);
        count!(2, 2, autonomy.outcome);
        raise(&mut self.autonomy_names, rows(autonomy.required_bytes, 1));
        let agency = cintelligence_agency::walk(memory, base, state, &mut [], &mut []);
        count!(3, 3, agency.outcome);
        raise(&mut self.agency_names, rows(agency.required_bytes, 1));
        match FAILURES[3].observe(agency_upgrade_levels::native_demand(memory, base, state)) {
            Ok(demand) => {
                raise(&mut self.rows[17], rows(demand.rows, 1));
                raise(
                    &mut self.agency_upgrade_definitions,
                    rows(demand.catalog, 1),
                );
            }
            Err(reason) => {
                failure.get_or_insert(reason);
            }
        }
        let operatives = coperative_leader::walk(memory, base, state, &mut [], &mut []);
        count!(
            4,
            4,
            operative_storage_outcome(operatives.outcome, operatives.refused_rows)
        );
        raise(
            &mut self.operative_names,
            rows(operatives.required_bytes, 1),
        );
        count!(
            5,
            16,
            cintelligence_agency::walk_captured_operatives(memory, base, state, &mut [])
        );
        count!(6, 5, ccryptology::walk(memory, base, state, &mut []));
        count!(
            7,
            6,
            ccountry_intel_network::walk(memory, base, state, &mut [])
        );
        count!(8, 7, walk_flags(memory, base, state, &mut [], &mut []));
        let variables = cvariables::walk_scopes(memory, base, state, &mut [], &mut [], &mut []);
        count!(9, 8, variables.outcome);
        raise(
            &mut self.variable_entries,
            rows(variables.required_entries, 1),
        );
        raise(&mut self.variable_names, rows(variables.required_bytes, 1));
        let operations =
            ccountry_operation_manager::walk(memory, base, state, &mut [], &mut [], &mut []);
        count!(10, 9, operations.outcome);
        count!(10, 10, operations.assignments);
        raise(
            &mut self.operation_descriptions,
            rows(operations.required_bytes, 1),
        );
        let tokens = ccountry_operation_token_manager::walk(memory, base, state, &mut [], &mut []);
        count!(11, 11, tokens.outcome);
        raise(&mut self.tokens, rows(tokens.required_tokens, 1));
        // 141502590 retains every original nonnull native index.
        count!(
            12,
            12,
            incoming_diplomatic_action_queue::walk(memory, base, state, &mut [])
        );
        failure.map_or(Ok(()), Err)
    }
}

fn count(outcome: Outcome) -> Result<usize, &'static str> {
    match outcome {
        Outcome::Rows { len } => Ok(len),
        Outcome::Truncated { len, unread_rows } => len
            .checked_add(unread_rows as usize)
            .ok_or("intel sizing row count overflow"),
        Outcome::Unavailable { reason } => Err(reason),
    }
}
