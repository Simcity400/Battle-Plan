//! Intel acquisition buffers. Allocation, growth and replay conversion are off-thread.

use crate::{
    sampling::{full_match, qpc},
    spooling::{table, unavailable},
    table_capture::{Arena, CaptureRows, grow_vec},
};
use engine::{
    Memory, ccountry, ccountry_intel, ccountry_intel_network, ccountry_operation_manager,
    ccountry_operation_token_manager, ccryptology, ccurrent_autonomy_status, cdiplomacy_status,
    cflag_manager, cgame_state, cintelligence_agency, coperation_instance, coperative_leader,
    country_intel_row,
    cpdx_array::Array,
    crelation_status, cscript_flag, cstate,
    csubject_relation::{self, Name},
    cvariables, incoming_diplomatic_action_queue, operation_assigned_operative, read, read_slice,
    walk::Outcome,
};
use replay::{
    frame::TableView,
    rows::{self, TableId, tables},
};
use std::mem::size_of;

pub const N: usize = 16;
pub const TABLES: [TableId; N] = [
    TableId::Capitulation,
    TableId::Subjects,
    TableId::Autonomy,
    TableId::IntelligenceAgencies,
    TableId::OperativeMemberships,
    TableId::Cryptology,
    TableId::IntelNetworks,
    TableId::ScriptFlags,
    TableId::ScriptVariables,
    TableId::Operations,
    TableId::OperationAssignments,
    TableId::OperationTokens,
    TableId::IncomingDiplomaticActions,
    TableId::RelationOpinions,
    TableId::RelationTuples,
    TableId::CountryIntel,
];
/// The operation walker supplies both tables in one census, and one relation
/// matrix pass supplies the cached opinions and the typed tuples. Each group's
/// entire cost, including whichever independent second reads are needed, is
/// charged once to its anchor.
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[
    ("operations and assigned slots", 9, &[9, 10]),
    ("relation opinions and typed tuples", 13, &[13, 14]),
];

pub const GLOBAL_SCOPE: u8 = 0;
pub const COUNTRY_SCOPE: u8 = 1;
pub const STATE_SCOPE: u8 = 2;

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    pub rows: [usize; N],
    /// Bytes of caller-owned scratch for the bulk intel-record copies. This is a
    /// fixed copy window, not a row capacity, so `grow` never raises it.
    pub intel_scratch: usize,
    pub subject_names: usize,
    pub autonomy_names: usize,
    pub variable_entries: usize,
    pub variable_names: usize,
    pub tokens: usize,
}

impl Default for Capacities {
    fn default() -> Self {
        Self {
            rows: [
                512, 256, 256, 512, 4096, 4096, 16_384, 8192, 2048, 2048, 8192, 4096, 512, 16_384,
                4096, 65_536,
            ],
            intel_scratch: 16_384,
            subject_names: 16_384,
            autonomy_names: 32_768,
            variable_entries: 16_384,
            variable_names: 512 * 1024,
            tokens: 32_768,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct ScopedFlag {
    scope_kind: u8,
    scope_id: u32,
    flag: cscript_flag::Row,
}

pub struct Captured {
    capitulation: CaptureRows<cdiplomacy_status::CapitulationRow>,
    subjects: CaptureRows<csubject_relation::Row>,
    autonomy: CaptureRows<ccurrent_autonomy_status::Row>,
    agencies: CaptureRows<cintelligence_agency::Row>,
    operatives: CaptureRows<coperative_leader::Row>,
    cryptology: CaptureRows<ccryptology::Row>,
    networks: CaptureRows<ccountry_intel_network::Row>,
    flags: CaptureRows<ScopedFlag>,
    variables: CaptureRows<cvariables::ScopeRow>,
    operations: CaptureRows<coperation_instance::Row>,
    assignments: CaptureRows<operation_assigned_operative::Row>,
    operation_tokens: CaptureRows<ccountry_operation_token_manager::Row>,
    incoming_actions: CaptureRows<incoming_diplomatic_action_queue::Row>,
    relation_opinions: CaptureRows<crelation_status::OpinionRow>,
    relation_tuples: CaptureRows<crelation_status::TupleRow>,
    country_intel: CaptureRows<country_intel_row::Row>,
    intel_scratch: Vec<u8>,
    subject_names: Arena<u8>,
    autonomy_names: Arena<u8>,
    variable_entries: Arena<cvariables::Row>,
    variable_names: Arena<u8>,
    tokens: Arena<u32>,
    flag_scratch: Vec<cscript_flag::Row>,
}

impl Captured {
    /// Every slice and returned array uses local TABLES order.
    pub fn with_capacities(capacity: &Capacities, successful: &[u16]) -> Self {
        let operations_successful = successful[9].min(successful[10]);
        let relations_successful = successful[13].min(successful[14]);
        Self {
            capitulation: CaptureRows::new(capacity.rows[0], successful[0]),
            subjects: CaptureRows::new(capacity.rows[1], successful[1]),
            autonomy: CaptureRows::new(capacity.rows[2], successful[2]),
            agencies: CaptureRows::new(capacity.rows[3], successful[3]),
            operatives: CaptureRows::new(capacity.rows[4], successful[4]),
            cryptology: CaptureRows::new(capacity.rows[5], successful[5]),
            networks: CaptureRows::new(capacity.rows[6], successful[6]),
            flags: CaptureRows::new(capacity.rows[7], successful[7]),
            variables: CaptureRows::new(capacity.rows[8], successful[8]),
            operations: CaptureRows::new(capacity.rows[9], operations_successful),
            assignments: CaptureRows::new(capacity.rows[10], operations_successful),
            operation_tokens: CaptureRows::new(capacity.rows[11], successful[11]),
            incoming_actions: CaptureRows::new(capacity.rows[12], successful[12]),
            relation_opinions: CaptureRows::new(capacity.rows[13], relations_successful),
            relation_tuples: CaptureRows::new(capacity.rows[14], relations_successful),
            country_intel: CaptureRows::new(capacity.rows[15], successful[15]),
            intel_scratch: vec![0; capacity.intel_scratch],
            subject_names: Arena::new(capacity.subject_names, successful[1]),
            autonomy_names: Arena::new(capacity.autonomy_names, successful[2]),
            variable_entries: Arena::new(capacity.variable_entries, successful[8]),
            variable_names: Arena::new(capacity.variable_names, successful[8]),
            tokens: Arena::new(capacity.tokens, successful[11]),
            flag_scratch: vec![cscript_flag::Row::default(); capacity.rows[7]],
        }
    }

    pub fn reset(&mut self) {
        self.capitulation.reset();
        self.subjects.reset();
        self.autonomy.reset();
        self.agencies.reset();
        self.operatives.reset();
        self.cryptology.reset();
        self.networks.reset();
        self.flags.reset();
        self.variables.reset();
        self.operations.reset();
        self.assignments.reset();
        self.operation_tokens.reset();
        self.incoming_actions.reset();
        self.relation_opinions.reset();
        self.relation_tuples.reset();
        self.country_intel.reset();
        self.subject_names.reset();
        self.autonomy_names.reset();
        self.variable_entries.reset();
        self.variable_names.reset();
        self.tokens.reset();
    }

    pub fn allocated_bytes(&self) -> usize {
        self.capitulation.allocated_bytes()
            + self.subjects.allocated_bytes()
            + self.autonomy.allocated_bytes()
            + self.agencies.allocated_bytes()
            + self.operatives.allocated_bytes()
            + self.cryptology.allocated_bytes()
            + self.networks.allocated_bytes()
            + self.flags.allocated_bytes()
            + self.variables.allocated_bytes()
            + self.operations.allocated_bytes()
            + self.assignments.allocated_bytes()
            + self.operation_tokens.allocated_bytes()
            + self.incoming_actions.allocated_bytes()
            + self.relation_opinions.allocated_bytes()
            + self.relation_tuples.allocated_bytes()
            + self.country_intel.allocated_bytes()
            + self.intel_scratch.capacity()
            + self.subject_names.allocated_bytes()
            + self.autonomy_names.allocated_bytes()
            + self.variable_entries.allocated_bytes()
            + self.variable_names.allocated_bytes()
            + self.tokens.allocated_bytes()
            + self.flag_scratch.capacity() * size_of::<cscript_flag::Row>()
    }

    pub fn grow(&mut self, capacity: &mut Capacities, successful: &[u16]) {
        let operations_successful = successful[9].min(successful[10]);
        let relations_successful = successful[13].min(successful[14]);
        self.capitulation.grow(&mut capacity.rows[0], successful[0]);
        self.subjects.grow(&mut capacity.rows[1], successful[1]);
        self.autonomy.grow(&mut capacity.rows[2], successful[2]);
        self.agencies.grow(&mut capacity.rows[3], successful[3]);
        self.operatives.grow(&mut capacity.rows[4], successful[4]);
        self.cryptology.grow(&mut capacity.rows[5], successful[5]);
        self.networks.grow(&mut capacity.rows[6], successful[6]);
        self.flags.grow(&mut capacity.rows[7], successful[7]);
        self.variables.grow(&mut capacity.rows[8], successful[8]);
        self.operations
            .grow(&mut capacity.rows[9], operations_successful);
        self.assignments
            .grow(&mut capacity.rows[10], operations_successful);
        self.operation_tokens
            .grow(&mut capacity.rows[11], successful[11]);
        self.incoming_actions
            .grow(&mut capacity.rows[12], successful[12]);
        self.relation_opinions
            .grow(&mut capacity.rows[13], relations_successful);
        self.relation_tuples
            .grow(&mut capacity.rows[14], relations_successful);
        self.country_intel.grow(&mut capacity.rows[15], successful[15]);
        // The intel scratch is a fixed copy window, not a row capacity: it bounds
        // how many 0x20-byte records one bulk copy moves, and a country longer
        // than the window is copied in several passes rather than truncated. It
        // therefore never grows with observed row counts.
        self.subject_names
            .grow(&mut capacity.subject_names, successful[1]);
        self.autonomy_names
            .grow(&mut capacity.autonomy_names, successful[2]);
        self.variable_entries
            .grow(&mut capacity.variable_entries, successful[8]);
        self.variable_names
            .grow(&mut capacity.variable_names, successful[8]);
        self.tokens.grow(&mut capacity.tokens, successful[11]);
        grow_vec(&mut self.flag_scratch, capacity.rows[7]);
    }

    /// No allocation, encoding, token-name resolution, engine calls or I/O.
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        needed: &[bool],
    ) -> ([bool; N], [u64; N]) {
        self.reset();
        let mut matched = [false; N];
        let mut ticks = [0; N];
        macro_rules! plain {
            ($index:literal, $field:ident, $walker:path) => {{
                let begin = qpc();
                matched[$index] = capture_rows(&mut self.$field, needed[$index], |rows| {
                    $walker(memory, base, state, rows)
                });
                ticks[$index] = elapsed(begin);
            }};
        }
        plain!(0, capitulation, cdiplomacy_status::walk_capitulation);

        let begin = qpc();
        matched[1] = capture_named(
            &mut self.subjects,
            &mut self.subject_names,
            needed[1],
            |rows, names, second| {
                let result = if second {
                    csubject_relation::walk_matrix(memory, base, state, rows, names)
                } else {
                    csubject_relation::walk(memory, base, state, rows, names)
                };
                ArenaRead::from(result)
            },
        );
        ticks[1] = elapsed(begin);
        let begin = qpc();
        matched[2] = capture_named(
            &mut self.autonomy,
            &mut self.autonomy_names,
            needed[2],
            |rows, names, _| {
                ArenaRead::from(ccurrent_autonomy_status::walk(
                    memory, base, state, rows, names,
                ))
            },
        );
        ticks[2] = elapsed(begin);
        plain!(3, agencies, cintelligence_agency::walk);
        plain!(4, operatives, coperative_leader::walk);
        plain!(5, cryptology, ccryptology::walk);
        plain!(6, networks, ccountry_intel_network::walk);

        let begin = qpc();
        matched[7] = capture_rows(&mut self.flags, needed[7], |rows| {
            walk_flags(memory, base, state, rows, &mut self.flag_scratch)
        });
        ticks[7] = elapsed(begin);

        let begin = qpc();
        matched[8] = capture_variables(
            &mut self.variables,
            &mut self.variable_entries,
            &mut self.variable_names,
            needed[8],
            |scopes, entries, names| {
                cvariables::walk_scopes(memory, base, state, scopes, entries, names)
            },
        );
        ticks[8] = elapsed(begin);

        let begin = qpc();
        let first = ccountry_operation_manager::walk(
            memory,
            base,
            state,
            &mut self.operations.rows,
            &mut self.assignments.rows,
        );
        self.operations.outcome = bounded(first.outcome, self.operations.rows.len());
        self.assignments.outcome = bounded(first.assignments, self.assignments.rows.len());
        if needed[9] || needed[10] {
            let second = ccountry_operation_manager::walk(
                memory,
                base,
                state,
                &mut self.operations.second,
                &mut self.assignments.second,
            );
            if needed[9] {
                self.operations.checked = true;
                matched[9] = self
                    .operations
                    .compare(bounded(second.outcome, self.operations.second.len()));
            }
            if needed[10] {
                self.assignments.checked = true;
                matched[10] = self
                    .assignments
                    .compare(bounded(second.assignments, self.assignments.second.len()));
            }
        }
        ticks[9] = elapsed(begin);
        // ticks[10] stays zero because the named group above owns its reads.
        let begin = qpc();
        matched[11] = capture_named(
            &mut self.operation_tokens,
            &mut self.tokens,
            needed[11],
            |rows, tokens, _| {
                let result =
                    ccountry_operation_token_manager::walk(memory, base, state, rows, tokens);
                ArenaRead {
                    outcome: result.outcome,
                    len: result.tokens_len,
                    required: result.required_tokens,
                }
            },
        );
        ticks[11] = elapsed(begin);
        plain!(
            12,
            incoming_actions,
            incoming_diplomatic_action_queue::walk
        );

        let begin = qpc();
        let first = crelation_status::walk(
            memory,
            base,
            state,
            &mut self.relation_opinions.rows,
            &mut self.relation_tuples.rows,
        );
        self.relation_opinions.outcome =
            bounded(first.opinions, self.relation_opinions.rows.len());
        self.relation_tuples.outcome = bounded(first.tuples, self.relation_tuples.rows.len());
        if needed[13] || needed[14] {
            let second = crelation_status::walk(
                memory,
                base,
                state,
                &mut self.relation_opinions.second,
                &mut self.relation_tuples.second,
            );
            if needed[13] {
                self.relation_opinions.checked = true;
                matched[13] = self
                    .relation_opinions
                    .compare(bounded(second.opinions, self.relation_opinions.second.len()));
            }
            if needed[14] {
                self.relation_tuples.checked = true;
                matched[14] = self
                    .relation_tuples
                    .compare(bounded(second.tuples, self.relation_tuples.second.len()));
            }
        }
        ticks[13] = elapsed(begin);
        // ticks[14] stays zero because the named group above owns its reads.

        let begin = qpc();
        matched[15] = capture_rows(&mut self.country_intel, needed[15], |rows| {
            ccountry_intel::walk(memory, base, state, rows, &mut self.intel_scratch)
        });
        ticks[15] = elapsed(begin);
        (matched, ticks)
    }

    pub fn outcomes(&self) -> [Outcome; N] {
        [
            effective(&self.capitulation),
            effective(&self.subjects),
            effective(&self.autonomy),
            effective(&self.agencies),
            effective(&self.operatives),
            effective(&self.cryptology),
            effective(&self.networks),
            effective(&self.flags),
            effective(&self.variables),
            effective(&self.operations),
            effective(&self.assignments),
            effective(&self.operation_tokens),
            effective(&self.incoming_actions),
            effective(&self.relation_opinions),
            effective(&self.relation_tuples),
            effective(&self.country_intel),
        ]
    }

    pub fn checked_mismatches(&self) -> [usize; N] {
        [
            self.capitulation.mismatches,
            self.subjects.mismatches,
            self.autonomy.mismatches,
            self.agencies.mismatches,
            self.operatives.mismatches,
            self.cryptology.mismatches,
            self.networks.mismatches,
            self.flags.mismatches,
            self.variables.mismatches,
            self.operations.mismatches,
            self.assignments.mismatches,
            self.operation_tokens.mismatches,
            self.incoming_actions.mismatches,
            self.relation_opinions.mismatches,
            self.relation_tuples.mismatches,
            self.country_intel.mismatches,
        ]
    }

    /// Called only on the spool thread; this owns all Vec and string conversion.
    pub fn tables(&self) -> [TableView; N] {
        let outcomes = self.outcomes();
        [
            table::<tables::Capitulation, _>(outcomes[0], &self.capitulation.rows, |r| {
                rows::CapitulationRow {
                    country: r.country,
                    capitulated: r.capitulated,
                    capitulated_hour: r.capitulated_hour,
                    last_surrender_hour: r.last_surrender_hour,
                    faction_join_hour: r.faction_join_hour,
                    legitimacy_raw: r.legitimacy_raw,
                    exile_army_leaders: r.exile_army_leaders,
                }
            }),
            self.subjects_table(outcomes[1]),
            self.autonomy_table(outcomes[2]),
            table::<tables::IntelligenceAgencies, _>(outcomes[3], &self.agencies.rows, |r| {
                rows::IntelligenceAgencyRow {
                    country: r.country,
                    upgrade_progress_raw: r.upgrade_progress_raw,
                    defense_raw: r.defense_raw,
                    active_count: r.active_count,
                    admission_ceiling: r.admission_ceiling,
                    unknown_f0: r.unknown_f0,
                }
            }),
            table::<tables::OperativeMemberships, _>(outcomes[4], &self.operatives.rows, |r| {
                rows::OperativeMembershipRow {
                    id: r.id,
                    owner_country: r.owner_country,
                    roster_country: r.roster_country,
                    roster_kind: r.roster_kind,
                    detected: r.detected,
                    assignment: r.assignment,
                    experience_raw: r.experience_raw,
                    skill: r.skill,
                    capturer_country: r.capturer_country,
                    captured_hour: r.captured_hour,
                    action_variant: r.action_variant,
                }
            }),
            table::<tables::Cryptology, _>(outcomes[5], &self.cryptology.rows, |r| {
                rows::CryptologyRow {
                    owner_country: r.owner_country,
                    target_country: r.target_country,
                    amount_raw: r.amount_raw,
                    bonus_days: r.bonus_days,
                    bonus_active: r.bonus_active,
                    decrypting: r.decrypting,
                }
            }),
            table::<tables::IntelNetworks, _>(outcomes[6], &self.networks.rows, |r| {
                rows::IntelNetworkRow {
                    owner_country: r.owner_country,
                    target_country: r.target_country,
                    state_id: r.state_id,
                    strength_raw: r.strength_raw,
                    subnetwork_selector: r.subnetwork_selector,
                }
            }),
            self.flags_table(outcomes[7]),
            self.variables_table(outcomes[8]),
            self.operations_table(outcomes[9]),
            table::<tables::OperationAssignments, _>(outcomes[10], &self.assignments.rows, |r| {
                rows::OperationAssignmentRow {
                    operation: r.operation,
                    ordinal: r.ordinal,
                    operative: r.operative,
                    selection: r.selection,
                }
            }),
            self.operation_tokens_table(outcomes[11]),
            table::<tables::IncomingDiplomaticActions, _>(
                outcomes[12],
                &self.incoming_actions.rows,
                |r| rows::IncomingDiplomaticActionRow {
                    country: r.country,
                    ordinal: r.ordinal,
                    handle: r.handle,
                    status: r.status,
                    action_type: r.action_type,
                    actor: r.actor,
                    original_actor: r.original_actor,
                    recipient: r.recipient,
                    original_recipient: r.original_recipient,
                    hour: r.hour,
                    last_command_hour: r.last_command_hour,
                },
            ),
            table::<tables::RelationOpinions, _>(outcomes[13], &self.relation_opinions.rows, |r| {
                rows::RelationOpinionRow {
                    owner_country: r.owner_country,
                    target_country: r.target_country,
                    opinion: r.opinion,
                }
            }),
            self.relation_tuples_table(outcomes[14]),
            table::<tables::CountryIntel, _>(outcomes[15], &self.country_intel.rows, |r| {
                rows::CountryIntelRow {
                    owner_country: r.owner_country,
                    target_index: r.target_index,
                    civilian: r.civilian,
                    army: r.army,
                    navy: r.navy,
                    air: r.air,
                }
            }),
        ]
    }
}

impl Captured {
    fn subjects_table(&self, outcome: Outcome) -> TableView {
        if !valid_arena(
            outcome,
            &self.subjects.rows,
            &self.subject_names.first,
            self.subject_names.len,
            self.subject_names.required,
        ) {
            return unavailable("subject name lies outside captured arena");
        }
        table::<tables::Subjects, _>(outcome, &self.subjects.rows, |r| rows::SubjectRow {
            overlord: r.overlord,
            subject: r.subject,
            start_hour: r.start_hour,
            end_hour: r.end_hour,
            cancel: r.cancel,
            autonomy_ratio: r.autonomy_ratio,
            state: literal(&self.subject_names.first, r.state).to_vec(),
        })
    }

    fn autonomy_table(&self, outcome: Outcome) -> TableView {
        if !valid_arena(
            outcome,
            &self.autonomy.rows,
            &self.autonomy_names.first,
            self.autonomy_names.len,
            self.autonomy_names.required,
        ) {
            return unavailable("autonomy name lies outside captured arena");
        }
        table::<tables::Autonomy, _>(outcome, &self.autonomy.rows, |r| rows::AutonomyRow {
            subject: r.subject,
            score: r.score,
            lower_score: r.lower_score,
            upper_score: r.upper_score,
            last_modified_bits: r.last_modified_bits,
            state: literal(&self.autonomy_names.first, r.state).to_vec(),
            previous: r
                .previous
                .map(|name| literal(&self.autonomy_names.first, name).to_vec()),
            next: r
                .next
                .map(|name| literal(&self.autonomy_names.first, name).to_vec()),
        })
    }

    fn flags_table(&self, outcome: Outcome) -> TableView {
        table::<tables::ScriptFlags, _>(outcome, &self.flags.rows, |r| rows::ScriptFlagRow {
            scope_kind: r.scope_kind,
            scope_id: r.scope_id,
            token: r.flag.token,
            set_hour: r.flag.set_hour,
            value: r.flag.value,
            remaining_days: r.flag.remaining_days,
        })
    }

    fn variables_table(&self, outcome: Outcome) -> TableView {
        if !valid_variables(
            outcome,
            &self.variables.rows,
            &self.variable_entries,
            &self.variable_names,
        ) {
            return unavailable("script variable entry or name lies outside captured arena");
        }
        let view = table::<tables::ScriptVariables, _>(outcome, &self.variables.rows, |r| {
            let (scope_kind, scope_id) = scope_identity(r.scope);
            let mut entries: Vec<_> = self.variable_entries.first
                [r.entries_start..r.entries_start + r.entries_len]
                .iter()
                .map(|entry| rows::ScriptVariableEntry {
                    name: self.variable_names.first
                        [entry.name_start..entry.name_start + entry.name_len]
                        .to_vec(),
                    value_raw: entry.value_raw,
                })
                .collect();
            entries.sort_unstable_by(|a, b| a.name.cmp(&b.name));
            rows::ScriptVariablesRow {
                scope_kind,
                scope_id,
                entries,
            }
        });
        let values = match &view {
            TableView::Complete(rows) | TableView::Truncated { rows, .. } => rows,
            _ => return view,
        };
        if values
            .rows::<tables::ScriptVariables>()
            .is_some_and(|rows| {
                rows.iter().any(|row| {
                    row.entries
                        .windows(2)
                        .any(|pair| pair[0].name == pair[1].name)
                })
            })
        {
            return unavailable("duplicate literal script variable key");
        }
        view
    }

    fn operations_table(&self, outcome: Outcome) -> TableView {
        table::<tables::Operations, _>(outcome, &self.operations.rows, |r| rows::OperationRow {
            id: r.id,
            owner: r.owner,
            target: r.target,
            target_province: r.target_province,
            definition_token: r.definition_token,
            preparation_current: r.preparation_current,
            preparation_required: r.preparation_required,
            flag_40: r.flag_40,
            auto_repeat: r.auto_repeat,
            terminal: r.terminal,
            start_hour: r.start_hour,
            duration_days: r.duration_days,
            phase_count: r.phase_count,
            assigned_slots: r.assigned_slots,
        })
    }

    /// The wire ordinal is the native serializable-vector index, so a native
    /// vector longer than the wire width cannot be represented and refuses the
    /// table rather than aliasing two relations onto one key.
    fn relation_tuples_table(&self, outcome: Outcome) -> TableView {
        if self.relation_tuples.rows[..outcome.len()]
            .iter()
            .any(|row| u16::try_from(row.ordinal).is_err())
        {
            return unavailable("relation tuple ordinal exceeds its wire width");
        }
        table::<tables::RelationTuples, _>(outcome, &self.relation_tuples.rows, |r| {
            rows::RelationTupleRow {
                owner_country: r.owner_country,
                target_country: r.target_country,
                type_id: r.type_id,
                ordinal: r.ordinal as u16,
                first_country: r.first_country,
                second_country: r.second_country,
                start_hour: r.start_hour,
                end_hour: r.end_hour,
                cancel: r.cancel,
                retired: r.retired,
            }
        })
    }

    fn operation_tokens_table(&self, outcome: Outcome) -> TableView {
        if !valid_arena(
            outcome,
            &self.operation_tokens.rows,
            &self.tokens.first,
            self.tokens.len,
            self.tokens.required,
        ) {
            return unavailable("operation token vector lies outside captured arena");
        }
        table::<tables::OperationTokens, _>(outcome, &self.operation_tokens.rows, |r| {
            rows::OperationTokensRow {
                owner: r.owner,
                target: r.target,
                tokens: self.tokens.first[r.tokens_start..r.tokens_start + r.tokens_len].to_vec(),
            }
        })
    }
}

fn elapsed(begin: i64) -> u64 {
    qpc().saturating_sub(begin).max(0) as u64
}

fn bounded(outcome: Outcome, capacity: usize) -> Outcome {
    if outcome.len() > capacity {
        Outcome::Unavailable {
            reason: "walker row count exceeds allocated buffer",
        }
    } else if matches!(outcome, Outcome::Truncated { unread_rows: 0, .. }) {
        Outcome::Unavailable {
            reason: "walker marked truncation without missing rows",
        }
    } else {
        outcome
    }
}

fn effective<R>(captured: &CaptureRows<R>) -> Outcome {
    if captured.checked && captured.mismatches != 0 {
        Outcome::Unavailable {
            reason: "table changed during double read",
        }
    } else {
        captured.outcome
    }
}

fn capture_rows<R: Default + Clone + PartialEq>(
    captured: &mut CaptureRows<R>,
    needed: bool,
    mut walk: impl FnMut(&mut [R]) -> Outcome,
) -> bool {
    captured.outcome = bounded(walk(&mut captured.rows), captured.rows.len());
    if !needed {
        return false;
    }
    captured.checked = true;
    let second = bounded(walk(&mut captured.second), captured.second.len());
    captured.compare(second)
}

#[derive(Clone, Copy)]
struct ArenaRead {
    outcome: Outcome,
    len: usize,
    required: usize,
}

impl From<csubject_relation::NameWalk> for ArenaRead {
    fn from(value: csubject_relation::NameWalk) -> Self {
        Self {
            outcome: value.outcome,
            len: value.bytes_len,
            required: value.required_bytes,
        }
    }
}

/// Equality compares the native fields and literal payload, never arena offsets.
trait ArenaRow<T>: Default + Clone + PartialEq {
    fn key(&self) -> u128;
    fn valid(&self, len: usize) -> bool;
    fn equivalent(&self, other: &Self, first: &[T], second: &[T]) -> bool;
}

fn fits(start: usize, count: usize, len: usize) -> bool {
    start.checked_add(count).is_some_and(|end| end <= len)
}

fn name_fits(name: Name, len: usize) -> bool {
    fits(name.start, name.len, len)
}

fn literal(bytes: &[u8], name: Name) -> &[u8] {
    &bytes[name.start..name.start + name.len]
}

fn optional_name_equal(a: Option<Name>, b: Option<Name>, first: &[u8], second: &[u8]) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => literal(first, a) == literal(second, b),
        _ => false,
    }
}

impl ArenaRow<u8> for csubject_relation::Row {
    fn key(&self) -> u128 {
        (u128::from(self.overlord) << 32) | u128::from(self.subject)
    }
    fn valid(&self, len: usize) -> bool {
        name_fits(self.state, len)
    }
    fn equivalent(&self, other: &Self, first: &[u8], second: &[u8]) -> bool {
        let mut adjusted = *other;
        adjusted.state = self.state;
        *self == adjusted && literal(first, self.state) == literal(second, other.state)
    }
}

impl ArenaRow<u8> for ccurrent_autonomy_status::Row {
    fn key(&self) -> u128 {
        u128::from(self.subject)
    }
    fn valid(&self, len: usize) -> bool {
        name_fits(self.state, len)
            && self.previous.is_none_or(|name| name_fits(name, len))
            && self.next.is_none_or(|name| name_fits(name, len))
    }
    fn equivalent(&self, other: &Self, first: &[u8], second: &[u8]) -> bool {
        let mut adjusted = *other;
        adjusted.state = self.state;
        adjusted.previous = self.previous;
        adjusted.next = self.next;
        *self == adjusted
            && literal(first, self.state) == literal(second, other.state)
            && optional_name_equal(self.previous, other.previous, first, second)
            && optional_name_equal(self.next, other.next, first, second)
    }
}

impl ArenaRow<u32> for ccountry_operation_token_manager::Row {
    fn key(&self) -> u128 {
        (u128::from(self.owner) << 32) | u128::from(self.target)
    }
    fn valid(&self, len: usize) -> bool {
        fits(self.tokens_start, self.tokens_len, len)
    }
    fn equivalent(&self, other: &Self, first: &[u32], second: &[u32]) -> bool {
        let mut adjusted = *other;
        adjusted.tokens_start = self.tokens_start;
        adjusted.tokens_len = self.tokens_len;
        *self == adjusted
            && first[self.tokens_start..self.tokens_start + self.tokens_len]
                == second[other.tokens_start..other.tokens_start + other.tokens_len]
    }
}

fn valid_arena<R: ArenaRow<T>, T>(
    outcome: Outcome,
    rows: &[R],
    values: &[T],
    len: usize,
    required: usize,
) -> bool {
    len <= values.len()
        && required >= len
        && rows
            .get(..outcome.len())
            .is_some_and(|rows| rows.iter().all(|row| row.valid(len)))
}

fn arena_outcome<R: ArenaRow<T>, T>(result: ArenaRead, rows: &[R], values: &[T]) -> Outcome {
    let outcome = bounded(result.outcome, rows.len());
    if valid_arena(outcome, rows, values, result.len, result.required) {
        outcome
    } else {
        Outcome::Unavailable {
            reason: "captured arena range invalid",
        }
    }
}

fn first_named<R: ArenaRow<T>, T: Default + Clone + PartialEq>(
    captured: &mut CaptureRows<R>,
    arena: &mut Arena<T>,
    first: ArenaRead,
) {
    captured.outcome = arena_outcome(first, &captured.rows, &arena.first);
    arena.len = first.len;
    arena.required = first.required;
}

fn second_named<R: ArenaRow<T>, T: Default + Clone + PartialEq>(
    captured: &mut CaptureRows<R>,
    arena: &mut Arena<T>,
    second: ArenaRead,
) -> bool {
    captured.checked = true;
    let outcome = arena_outcome(second, &captured.second, &arena.second);
    if let (Outcome::Rows { len }, Outcome::Rows { len: second_len }) = (captured.outcome, outcome)
    {
        captured.rows[..len].sort_unstable_by_key(|row| row.key());
        captured.second[..second_len].sort_unstable_by_key(|row| row.key());
        captured.mismatches = usize::from(
            len != second_len || arena.len != second.len || arena.required != second.required,
        ) + captured.rows[..len]
            .iter()
            .zip(&captured.second[..second_len])
            .filter(|(first, other)| !first.equivalent(other, &arena.first, &arena.second))
            .count();
        if captured.rows[..len]
            .windows(2)
            .any(|r| r[0].key() == r[1].key())
            || captured.second[..second_len]
                .windows(2)
                .any(|r| r[0].key() == r[1].key())
        {
            captured.mismatches += 1;
        }
    } else {
        // Different partial censuses need not contain the same prefix.
        captured.mismatches = usize::from(
            matches!(captured.outcome, Outcome::Rows { .. })
                || matches!(outcome, Outcome::Rows { .. }),
        );
    }
    arena.required = arena.required.max(second.required);
    full_match(captured.outcome, outcome, captured.mismatches)
}

fn capture_named<R: ArenaRow<T>, T: Default + Clone + PartialEq>(
    captured: &mut CaptureRows<R>,
    arena: &mut Arena<T>,
    needed: bool,
    mut walk: impl FnMut(&mut [R], &mut [T], bool) -> ArenaRead,
) -> bool {
    let first = walk(&mut captured.rows, &mut arena.first, false);
    first_named(captured, arena, first);
    if !needed {
        return false;
    }
    let second = walk(&mut captured.second, &mut arena.second, true);
    second_named(captured, arena, second)
}

fn scope_identity(scope: cvariables::Scope) -> (u8, u32) {
    match scope {
        cvariables::Scope::Global => (GLOBAL_SCOPE, 0),
        cvariables::Scope::Country(id) => (COUNTRY_SCOPE, id),
        cvariables::Scope::State(id) => (STATE_SCOPE, id),
    }
}

fn capture_variables(
    captured: &mut CaptureRows<cvariables::ScopeRow>,
    entries: &mut Arena<cvariables::Row>,
    names: &mut Arena<u8>,
    needed: bool,
    mut walk: impl FnMut(
        &mut [cvariables::ScopeRow],
        &mut [cvariables::Row],
        &mut [u8],
    ) -> cvariables::ScopeWalk,
) -> bool {
    let first = walk(&mut captured.rows, &mut entries.first, &mut names.first);
    captured.outcome = variable_outcome(first, &captured.rows, &entries.first, &names.first);
    entries.len = first.entries_len;
    entries.required = first.required_entries;
    names.len = first.bytes_len;
    names.required = first.required_bytes;
    if !needed {
        return false;
    }
    let second = walk(&mut captured.second, &mut entries.second, &mut names.second);
    captured.checked = true;
    let outcome = variable_outcome(second, &captured.second, &entries.second, &names.second);
    captured.compare(outcome);
    // Both reads independently enumerate native slots into separate arenas.
    // Compare every literal byte and native value, not merely arena lengths.
    if matches!(captured.outcome, Outcome::Rows { .. }) && matches!(outcome, Outcome::Rows { .. }) {
        captured.mismatches +=
            usize::from(!entries.compare(second.entries_len, second.required_entries));
        captured.mismatches += usize::from(!names.compare(second.bytes_len, second.required_bytes));
    } else {
        entries.required = entries.required.max(second.required_entries);
        names.required = names.required.max(second.required_bytes);
    }
    full_match(captured.outcome, outcome, captured.mismatches)
}

fn variable_outcome(
    read: cvariables::ScopeWalk,
    rows: &[cvariables::ScopeRow],
    entries: &[cvariables::Row],
    names: &[u8],
) -> Outcome {
    if valid_variable_parts(read, rows, entries, names) {
        read.outcome
    } else {
        Outcome::Unavailable {
            reason: "script variable entry or name lies outside captured arena",
        }
    }
}

fn valid_variables(
    outcome: Outcome,
    rows: &[cvariables::ScopeRow],
    entries: &Arena<cvariables::Row>,
    names: &Arena<u8>,
) -> bool {
    valid_variable_parts(
        cvariables::ScopeWalk {
            outcome,
            entries_len: entries.len,
            required_entries: entries.required,
            bytes_len: names.len,
            required_bytes: names.required,
        },
        rows,
        &entries.first,
        &names.first,
    )
}

fn valid_variable_parts(
    read: cvariables::ScopeWalk,
    rows: &[cvariables::ScopeRow],
    entries: &[cvariables::Row],
    names: &[u8],
) -> bool {
    read.entries_len <= entries.len()
        && read.required_entries >= read.entries_len
        && read.bytes_len <= names.len()
        && read.required_bytes >= read.bytes_len
        && rows.get(..read.outcome.len()).is_some_and(|rows| {
            rows.iter().all(|row| {
                fits(row.entries_start, row.entries_len, read.entries_len)
                    && entries[row.entries_start..row.entries_start + row.entries_len]
                        .iter()
                        .all(|entry| fits(entry.name_start, entry.name_len, read.bytes_len))
            })
        })
}

/// Proven expiry callers dereference all three manager roots. Slot zero and
/// null state holes are not scopes; canonical country aliases are visited once.
fn walk_flags(
    memory: &impl Memory,
    base: usize,
    state: usize,
    output: &mut [ScopedFlag],
    scratch: &mut [cscript_flag::Row],
) -> Outcome {
    let result = (|| {
        let mut len = 0;
        let mut unread = 0_u32;
        let mut scope = |kind, id, root, offset| -> Result<(), &'static str> {
            let manager = pointer(memory, root, offset)?;
            if manager == 0 {
                return Err("script flag manager root is null");
            }
            let available = (output.len() - len).min(scratch.len());
            let outcome = bounded(
                cflag_manager::walk(memory, base, manager, &mut scratch[..available]),
                available,
            );
            let count = match outcome {
                Outcome::Rows { len } => len,
                Outcome::Truncated { len, unread_rows } => {
                    unread = unread
                        .checked_add(unread_rows)
                        .ok_or("flag omitted count overflow")?;
                    len
                }
                Outcome::Unavailable { reason } => return Err(reason),
            };
            for flag in &scratch[..count] {
                output[len] = ScopedFlag {
                    scope_kind: kind,
                    scope_id: id,
                    flag: *flag,
                };
                len += 1;
            }
            Ok(())
        };
        scope(GLOBAL_SCOPE, 0, state, cgame_state::GLOBAL_FLAGS)?;
        let countries = ccountry::Reader::new(memory, base, state)?;
        let mut found_country = false;
        for slot in 1..countries.count() {
            if let Some(country) = countries.at(slot)? {
                found_country = true;
                scope(
                    COUNTRY_SCOPE,
                    country.id,
                    country.pointer,
                    ccountry::FLAG_MANAGER,
                )?;
            }
        }
        if !found_country {
            return Err("country table has no readable countries");
        }
        let states = state
            .checked_add(cgame_state::STATES)
            .and_then(|at| Array::read(memory, at, 8))
            .ok_or("state scope array unreadable")?;
        if states.count < 2 {
            return Err("state table has no state slots");
        }
        let state_vtable = base
            .checked_add(cstate::VTABLE_RVA)
            .ok_or("state vtable overflow")?;
        let mut first = 1;
        let mut found_state = false;
        let mut pointers = [0_u8; 512];
        while first < states.count {
            let count = ((states.count - first) as usize).min(64);
            let address = states
                .element(first, 8)
                .ok_or("state scope pointer overflow")?;
            if !read_slice(memory, address, &mut pointers[..count * 8]) {
                return Err("state scope pointer array unreadable");
            }
            for (index, bytes) in pointers[..count * 8].chunks_exact(8).enumerate() {
                let at = usize::try_from(u64::from_le_bytes(bytes.try_into().unwrap()))
                    .map_err(|_| "state scope pointer overflow")?;
                if at == 0 {
                    continue;
                }
                found_state = true;
                if read::<u64>(memory, at) != Some(state_vtable as u64) {
                    return Err("state scope vtable mismatch");
                }
                let id = at
                    .checked_add(cstate::ID)
                    .and_then(|p| read::<u32>(memory, p))
                    .ok_or("state scope id unreadable")?;
                if id != first + index as u32 {
                    return Err("state scope id differs from slot");
                }
                scope(STATE_SCOPE, id, at, cstate::FLAGS)?;
            }
            first += count as u32;
        }
        if !found_state {
            return Err("state table has no readable states");
        }
        Ok(if unread == 0 {
            Outcome::Rows { len }
        } else {
            Outcome::Truncated {
                len,
                unread_rows: unread,
            }
        })
    })();
    result.unwrap_or_else(|reason| Outcome::Unavailable { reason })
}

fn pointer(memory: &impl Memory, root: usize, offset: usize) -> Result<usize, &'static str> {
    root.checked_add(offset)
        .and_then(|p| read::<u64>(memory, p))
        .and_then(|p| usize::try_from(p).ok())
        .ok_or("script scope root unreadable")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sampling::CONSISTENCY_TARGET;
    use engine::memory::SliceMemory;
    use replay::rows::TableSchema;

    fn small() -> (Capacities, Captured) {
        let capacities = Capacities {
            rows: [3; N],
            intel_scratch: 2 * engine::country_intel_row::STRIDE,
            subject_names: 8,
            autonomy_names: 8,
            variable_entries: 4,
            variable_names: 8,
            tokens: 8,
        };
        let captured = Captured::with_capacities(&capacities, &[0; N]);
        (capacities, captured)
    }

    fn complete<S: TableSchema>(view: &TableView) -> &[S::Row] {
        let TableView::Complete(rows) = view else {
            panic!("expected complete table");
        };
        rows.rows::<S>().unwrap()
    }

    fn variable_fixture(
        scopes: &mut [cvariables::ScopeRow],
        entries: &mut [cvariables::Row],
        names: &mut [u8],
    ) -> cvariables::ScopeWalk {
        scopes[0] = cvariables::ScopeRow {
            scope: cvariables::Scope::Country(u32::MAX),
            entries_start: 0,
            entries_len: 2,
        };
        entries[0] = cvariables::Row {
            name_start: 0,
            name_len: 2,
            value_raw: i64::MIN,
        };
        entries[1] = cvariables::Row {
            name_start: 2,
            name_len: 1,
            value_raw: i64::MAX,
        };
        names[..3].copy_from_slice(&[0xff, 0, b'x']);
        cvariables::ScopeWalk {
            outcome: Outcome::Rows { len: 1 },
            entries_len: 2,
            required_entries: 2,
            bytes_len: 3,
            required_bytes: 3,
        }
    }

    #[test]
    fn independent_variable_reads_compare_scope_values_and_every_literal_byte() {
        for changed in 0..4 {
            let (_, mut capture) = small();
            let mut calls = 0;
            let matched = capture_variables(
                &mut capture.variables,
                &mut capture.variable_entries,
                &mut capture.variable_names,
                true,
                |scopes, entries, names| {
                    calls += 1;
                    let result = variable_fixture(scopes, entries, names);
                    if calls == 2 {
                        match changed {
                            1 => entries[0].value_raw += 1,
                            2 => names[1] = 1,
                            3 => scopes[0].scope = cvariables::Scope::State(u32::MAX),
                            _ => {}
                        }
                    }
                    result
                },
            );
            assert_eq!(calls, 2);
            assert_eq!(matched, changed == 0);
            assert_eq!(capture.variables.mismatches, usize::from(changed != 0));
            assert!(capture.variables.checked);
        }
    }

    #[test]
    fn variable_partial_second_read_grows_both_arenas_without_counting_a_check() {
        let (mut capacities, mut capture) = small();
        let mut calls = 0;
        assert!(!capture_variables(
            &mut capture.variables,
            &mut capture.variable_entries,
            &mut capture.variable_names,
            true,
            |scopes, entries, names| {
                calls += 1;
                let mut result = variable_fixture(scopes, entries, names);
                if calls == 2 {
                    result.outcome = Outcome::Truncated {
                        len: 1,
                        unread_rows: 1,
                    };
                    result.required_entries = 17;
                    result.required_bytes = 65;
                }
                result
            },
        ));
        assert!(matches!(capture.outcomes()[8], Outcome::Unavailable { .. }));
        capture.grow(&mut capacities, &[0; N]);
        assert!(capacities.variable_entries >= 17);
        assert!(capacities.variable_names >= 65);
        capture.grow(&mut capacities, &[CONSISTENCY_TARGET; N]);
        assert!(capture.variables.second.is_empty());
        assert!(capture.variable_entries.second.is_empty());
        assert!(capture.variable_names.second.is_empty());
        capture.reset();
        calls = 0;
        assert!(!capture_variables(
            &mut capture.variables,
            &mut capture.variable_entries,
            &mut capture.variable_names,
            false,
            |scopes, entries, names| {
                calls += 1;
                variable_fixture(scopes, entries, names)
            },
        ));
        assert_eq!(calls, 1);
        assert!(!capture.variables.checked);
        assert_eq!(capture.variables.outcome, Outcome::Rows { len: 1 });
    }

    #[test]
    fn independent_subject_order_and_arena_offsets_compare_by_exact_content() {
        let (_, mut capture) = small();
        let read = ArenaRead {
            outcome: Outcome::Rows { len: 2 },
            len: 3,
            required: 3,
        };
        capture.subjects.rows[0] = csubject_relation::Row {
            overlord: 3,
            subject: 7,
            autonomy_ratio: i64::MIN,
            state: Name { start: 0, len: 2 },
            ..Default::default()
        };
        capture.subjects.rows[1] = csubject_relation::Row {
            overlord: 2,
            subject: 9,
            autonomy_ratio: i64::MAX,
            state: Name { start: 2, len: 1 },
            ..Default::default()
        };
        capture.subject_names.first[..3].copy_from_slice(&[0xff, 0, b'x']);
        capture.subjects.second[0] = capture.subjects.rows[1];
        capture.subjects.second[0].state.start = 0;
        capture.subjects.second[1] = capture.subjects.rows[0];
        capture.subjects.second[1].state.start = 1;
        capture.subject_names.second[..3].copy_from_slice(&[b'x', 0xff, 0]);
        first_named(&mut capture.subjects, &mut capture.subject_names, read);
        assert!(second_named(
            &mut capture.subjects,
            &mut capture.subject_names,
            read
        ));
        capture.subject_names.second[2] = 1;
        assert!(!second_named(
            &mut capture.subjects,
            &mut capture.subject_names,
            read
        ));
        assert!(matches!(capture.outcomes()[1], Outcome::Unavailable { .. }));
        assert!(!second_named(
            &mut capture.subjects,
            &mut capture.subject_names,
            ArenaRead {
                outcome: Outcome::Truncated {
                    len: 1,
                    unread_rows: 1
                },
                ..read
            }
        ));
    }

    #[test]
    fn replay_conversion_retains_native_widths_nullability_literal_keys_and_vector_order() {
        let (_, mut capture) = small();
        capture.capitulation.outcome = Outcome::Rows { len: 1 };
        capture.capitulation.rows[0] = cdiplomacy_status::CapitulationRow {
            country: u32::MAX,
            capitulated: 0xfe,
            capitulated_hour: i32::MIN,
            last_surrender_hour: i32::MAX,
            faction_join_hour: -1,
            legitimacy_raw: i64::MIN,
            exile_army_leaders: i32::MIN,
        };
        capture.operatives.outcome = Outcome::Rows { len: 1 };
        capture.operatives.rows[0] = coperative_leader::Row {
            id: u64::MAX,
            owner_country: u32::MAX,
            roster_country: u32::MAX - 1,
            roster_kind: u8::MAX,
            detected: 0xfe,
            assignment: u32::MAX,
            experience_raw: i64::MIN,
            skill: Some(i32::MIN),
            capturer_country: 0,
            captured_hour: i32::MAX,
            action_variant: None,
        };
        capture.networks.outcome = Outcome::Rows { len: 2 };
        capture.networks.rows[0] = ccountry_intel_network::Row {
            owner_country: 1,
            target_country: 2,
            state_id: 3,
            strength_raw: None,
            subnetwork_selector: i32::MIN,
        };
        capture.networks.rows[1] = ccountry_intel_network::Row {
            owner_country: 1,
            target_country: 2,
            state_id: 4,
            strength_raw: Some(i64::MIN),
            subnetwork_selector: 0,
        };
        capture.operations.outcome = Outcome::Rows { len: 1 };
        capture.operations.rows[0] = coperation_instance::Row {
            id: u64::MAX,
            definition_token: u32::MAX,
            preparation_current: i64::MIN,
            ..Default::default()
        };
        capture.operation_tokens.outcome = Outcome::Rows { len: 2 };
        capture.operation_tokens.rows[0] = ccountry_operation_token_manager::Row {
            owner: u32::MAX,
            target: 2,
            tokens_start: 0,
            tokens_len: 4,
        };
        capture.operation_tokens.rows[1] = ccountry_operation_token_manager::Row {
            owner: u32::MAX,
            target: 0,
            tokens_start: 4,
            tokens_len: 0,
        };
        capture.tokens.first[..4].copy_from_slice(&[u32::MAX, 2, 2, 0]);
        capture.tokens.len = 4;
        capture.tokens.required = 4;
        capture.autonomy.outcome = Outcome::Rows { len: 1 };
        capture.autonomy.rows[0] = ccurrent_autonomy_status::Row {
            subject: 9,
            state: Name { start: 0, len: 1 },
            previous: None,
            next: Some(Name { start: 1, len: 0 }),
            ..Default::default()
        };
        capture.autonomy_names.first[0] = 0xff;
        capture.autonomy_names.len = 1;
        capture.autonomy_names.required = 1;
        capture.variables.outcome = Outcome::Rows { len: 1 };
        capture.variables.rows[0] = cvariables::ScopeRow {
            scope: cvariables::Scope::Country(u32::MAX),
            entries_start: 0,
            entries_len: 2,
        };
        capture.variable_entries.first[0] = cvariables::Row {
            name_start: 0,
            name_len: 2,
            value_raw: i64::MIN,
        };
        capture.variable_entries.first[1] = cvariables::Row {
            name_start: 2,
            name_len: 1,
            value_raw: i64::MAX,
        };
        capture.variable_entries.len = 2;
        capture.variable_entries.required = 2;
        capture.variable_names.first[..3].copy_from_slice(&[0xff, 0, b'x']);
        capture.variable_names.len = 3;
        capture.variable_names.required = 3;

        let views = capture.tables();
        let networks = complete::<tables::IntelNetworks>(&views[6]);
        assert_eq!(
            (networks[0].subnetwork_selector, networks[0].strength_raw),
            (i32::MIN, None)
        );
        assert_eq!(
            (networks[1].subnetwork_selector, networks[1].strength_raw),
            (0, Some(i64::MIN))
        );
        let operation = &complete::<tables::Operations>(&views[9])[0];
        assert_eq!(
            (
                operation.id,
                operation.definition_token,
                operation.preparation_current
            ),
            (u64::MAX, u32::MAX, i64::MIN)
        );
        let capitulation = &complete::<tables::Capitulation>(&views[0])[0];
        assert_eq!(
            (
                capitulation.country,
                capitulation.capitulated,
                capitulation.legitimacy_raw
            ),
            (u32::MAX, 0xfe, i64::MIN)
        );
        let operative = &complete::<tables::OperativeMemberships>(&views[4])[0];
        assert_eq!(
            (
                operative.id,
                operative.roster_country,
                operative.roster_kind
            ),
            (u64::MAX, u32::MAX - 1, u8::MAX)
        );
        assert_eq!(
            (operative.skill, operative.action_variant),
            (Some(i32::MIN), None)
        );
        let autonomy = &complete::<tables::Autonomy>(&views[2])[0];
        assert_eq!(
            (autonomy.previous.as_deref(), autonomy.next.as_deref()),
            (None, Some(&[][..]))
        );
        let tokens = complete::<tables::OperationTokens>(&views[11]);
        assert_eq!(tokens[0].target, 0);
        assert!(tokens[0].tokens.is_empty());
        assert_eq!(tokens[1].tokens, [u32::MAX, 2, 2, 0]);
        let variables = &complete::<tables::ScriptVariables>(&views[8])[0].entries;
        assert_eq!(variables[0].name, b"x");
        assert_eq!(variables[1].name, [0xff, 0]);
        assert_eq!(
            (variables[0].value_raw, variables[1].value_raw),
            (i64::MAX, i64::MIN)
        );
    }

    #[test]
    fn invalid_arenas_duplicate_literal_keys_and_unavailable_scopes_never_become_empty_tables() {
        let (_, mut capture) = small();
        capture.operation_tokens.outcome = Outcome::Rows { len: 1 };
        capture.operation_tokens.rows[0].tokens_start = usize::MAX;
        capture.operation_tokens.rows[0].tokens_len = 2;
        assert!(matches!(
            capture.tables()[11],
            TableView::Unavailable { .. }
        ));

        capture.variables.outcome = Outcome::Rows { len: 1 };
        capture.variables.rows[0].entries_len = 2;
        capture.variable_entries.len = 2;
        capture.variable_entries.required = 2;
        capture.variable_entries.first[0] = cvariables::Row {
            name_start: 0,
            name_len: 1,
            value_raw: 1,
        };
        capture.variable_entries.first[1] = cvariables::Row {
            name_start: 1,
            name_len: 1,
            value_raw: 2,
        };
        capture.variable_names.first[..2].copy_from_slice(b"aa");
        capture.variable_names.len = 2;
        capture.variable_names.required = 2;
        assert!(matches!(capture.tables()[8], TableView::Unavailable { .. }));
        let unavailable = Outcome::Unavailable {
            reason: "script variable scope root is null",
        };
        capture.variables.outcome = unavailable;
        assert!(matches!(capture.tables()[8], TableView::Unavailable { .. }));
        assert!(!full_match(unavailable, unavailable, 0));
        capture.reset();
        assert_eq!(capture.checked_mismatches(), [0; N]);
        assert_eq!(capture.tokens.len, 0);
    }

    #[test]
    fn growth_keeps_shared_operation_comparisons_until_both_tables_finish() {
        let (mut capacities, mut capture) = small();
        capture.subjects.outcome = Outcome::Truncated {
            len: 3,
            unread_rows: 12,
        };
        capture.subject_names.required = 33;
        let mut successes = [CONSISTENCY_TARGET; N];
        successes[10] -= 1;
        capture.grow(&mut capacities, &successes);
        assert!(capacities.rows[1] >= 15);
        assert!(capacities.subject_names >= 33);
        assert!(capture.subjects.second.is_empty());
        assert!(!capture.operations.second.is_empty());
        assert!(!capture.assignments.second.is_empty());
        let before = capture.allocated_bytes();
        successes[10] = CONSISTENCY_TARGET;
        capture.grow(&mut capacities, &successes);
        assert!(capture.operations.second.is_empty());
        assert!(capture.assignments.second.is_empty());
        assert!(capture.allocated_bytes() < before);
        assert_eq!(
            READ_GROUPS,
            &[
                ("operations and assigned slots", 9, &[9, 10][..]),
                ("relation opinions and typed tuples", 13, &[13, 14][..]),
            ]
        );
    }

    const BASE: usize = 0x0001_4000_0000;
    const GAME: usize = 0x1000;

    fn put(bytes: &mut [u8], at: usize, value: &[u8]) {
        bytes[at..at + value.len()].copy_from_slice(value);
    }

    fn flag_scopes() -> Vec<u8> {
        let mut bytes = vec![0; 0xc000];
        for (offset, data, count) in [
            (cgame_state::COUNTRIES, 0x2000_u64, 3_u32),
            (cgame_state::COUNTRY_LINKS, 0x2100, 3),
            (cgame_state::STATES, 0x2200, 3),
        ] {
            put(&mut bytes, GAME + offset, &data.to_le_bytes());
            put(&mut bytes, GAME + offset + 8, &count.to_le_bytes());
            put(&mut bytes, GAME + offset + 12, &count.to_le_bytes());
        }
        put(&mut bytes, 0x2008, &0x4000_u64.to_le_bytes());
        put(&mut bytes, 0x2010, &0x4000_u64.to_le_bytes());
        put(&mut bytes, 0x2104, &1_u32.to_le_bytes());
        put(&mut bytes, 0x2108, &1_u32.to_le_bytes());
        put(
            &mut bytes,
            0x4000,
            &((BASE + ccountry::VTABLE_RVA) as u64).to_le_bytes(),
        );
        put(&mut bytes, 0x4000 + ccountry::ID, &2_u32.to_le_bytes());
        put(&mut bytes, 0x2200, &u64::MAX.to_le_bytes());
        put(&mut bytes, 0x2208, &0x8000_u64.to_le_bytes());
        put(
            &mut bytes,
            0x8000,
            &((BASE + cstate::VTABLE_RVA) as u64).to_le_bytes(),
        );
        put(&mut bytes, 0x8000 + cstate::ID, &1_u32.to_le_bytes());
        for (index, (root, offset)) in [
            (GAME, cgame_state::GLOBAL_FLAGS),
            (0x4000, ccountry::FLAG_MANAGER),
            (0x8000, cstate::FLAGS),
        ]
        .into_iter()
        .enumerate()
        {
            let manager = 0xa000 + index * 0x100;
            let row = 0xb000 + index * cscript_flag::STRIDE;
            put(&mut bytes, root + offset, &(manager as u64).to_le_bytes());
            put(
                &mut bytes,
                manager,
                &((BASE + cflag_manager::VTABLE_RVA) as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                manager + cflag_manager::ROWS,
                &(row as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                manager + cflag_manager::SLOT_COUNT,
                &1_i32.to_le_bytes(),
            );
            put(
                &mut bytes,
                manager + cflag_manager::ENTRY_COUNT,
                &1_i32.to_le_bytes(),
            );
            for (field, vtable) in [
                (0, cscript_flag::VTABLE_RVA),
                (cscript_flag::DATE, cscript_flag::DATE_VTABLE_RVA),
                (
                    cscript_flag::DATE_SECONDARY,
                    cscript_flag::DATE_SECONDARY_VTABLE_RVA,
                ),
            ] {
                put(
                    &mut bytes,
                    row + field,
                    &((BASE + vtable) as u64).to_le_bytes(),
                );
            }
            put(
                &mut bytes,
                row + cscript_flag::TOKEN_ID,
                &u32::MAX.to_le_bytes(),
            );
            put(
                &mut bytes,
                row + cscript_flag::SET_HOUR,
                &i32::MIN.to_le_bytes(),
            );
            put(
                &mut bytes,
                row + cscript_flag::VALUE,
                &i16::MIN.to_le_bytes(),
            );
            put(
                &mut bytes,
                row + cscript_flag::REMAINING_DAYS,
                &(-3_i16).to_le_bytes(),
            );
        }
        bytes
    }

    #[test]
    fn flag_census_preserves_all_scope_identities_aliases_and_exact_truncation() {
        let mut bytes = flag_scopes();
        let mut rows = [ScopedFlag::default(); 3];
        let mut scratch = [cscript_flag::Row::default(); 3];
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        assert_eq!(
            walk_flags(&memory, BASE, GAME, &mut rows, &mut scratch),
            Outcome::Rows { len: 3 }
        );
        assert_eq!(
            rows.map(|row| (row.scope_kind, row.scope_id)),
            [(GLOBAL_SCOPE, 0), (COUNTRY_SCOPE, 2), (STATE_SCOPE, 1)]
        );
        assert!(
            rows.iter()
                .all(|row| row.flag.value == i16::MIN && row.flag.remaining_days == -3)
        );
        assert_eq!(
            walk_flags(&memory, BASE, GAME, &mut rows[..1], &mut scratch),
            Outcome::Truncated {
                len: 1,
                unread_rows: 2
            }
        );
        put(&mut bytes, 0x8000 + cstate::FLAGS, &0_u64.to_le_bytes());
        assert!(matches!(
            walk_flags(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes
                },
                BASE,
                GAME,
                &mut rows,
                &mut scratch
            ),
            Outcome::Unavailable { .. }
        ));
    }
}
