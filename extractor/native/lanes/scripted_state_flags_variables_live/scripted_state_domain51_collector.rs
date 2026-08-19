#[path = "scripted_state_compact.rs"]
pub mod compact;

pub const EXPECTED_SHA_870EBF: &str =
    "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad";

pub const CGAMESTATE_RVA: usize = 0x3316E90;
pub const COUNTRY_TABLE: usize = 0x310;
pub const COUNTRY_COUNT: usize = 0x31C;
pub const COUNTRY_VTABLE_RVA: usize = 0x27D0E90;
pub const COUNTRY_TAG_DB_ID: usize = 0x08;
pub const COUNTRY_VARIABLES: usize = 0x218;
pub const COUNTRY_FLAGS: usize = 0x230;
pub const GLOBAL_FLAGS: usize = 0x258;
pub const GLOBAL_VARIABLES: usize = 0x980;
pub const PROVINCE_ARRAY: usize = 0x2B0;
pub const PROVINCE_COUNT: usize = 0x2BC;
pub const CPROVINCE_VTABLE_RVA: usize = 0x295AF68;
pub const CPROVINCE_STATE: usize = 0xC0;
pub const STATE_VTABLE_RVA: usize = 0x29206C0;
pub const STATE_ID: usize = 0x58;
pub const STATE_FLAGS: usize = 0x540;
pub const STATE_VARIABLES: usize = 0x7F8;

pub const CFLAGMANAGER_ROWS: usize = 0x08;
pub const CFLAGMANAGER_PACKED_SLOT_ENTRY_COUNT: usize = 0x10;
pub const CFLAG_ROW_STRIDE: usize = 0x30;
pub const CFLAG_ROW_TOKEN_ID: usize = 0x08;
pub const CFLAG_ROW_DATE_HOUR: usize = 0x18;
pub const CFLAG_ROW_VALUE: usize = 0x28;

pub const DELAYED_EVENTS_BADGE_KEY: &str = compact::DELAYED_EVENTS_BADGE_KEY;
pub const SCRIPTED_GUI_BADGE_KEY: &str = compact::SCRIPTED_GUI_BADGE_KEY;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScopeReadStatus {
    Measured,
    MeasuredEmpty,
    Unmeasured,
}

impl ScopeReadStatus {
    fn is_measured(self) -> bool {
        matches!(self, Self::Measured | Self::MeasuredEmpty)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VariableEntry {
    pub key: String,
    pub value_raw: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VariableScopeRead {
    pub scope_kind: u32,
    pub owner_id: u32,
    pub status: ScopeReadStatus,
    pub entries: Vec<VariableEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FlagEntry {
    pub key: Option<String>,
    pub token_id: Option<u32>,
    pub date_hour_raw: Option<i32>,
    pub value: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FlagScopeRead {
    pub scope_kind: u32,
    pub owner_id: u32,
    pub status: ScopeReadStatus,
    pub entries: Vec<FlagEntry>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpaqueRootState {
    NotRequested,
    MeasuredEmpty,
    UnresolvedFixtureBlocked,
}

impl OpaqueRootState {
    fn is_observed(self) -> bool {
        !matches!(self, Self::NotRequested)
    }

    fn is_measured(self) -> bool {
        matches!(self, Self::MeasuredEmpty)
    }

    fn is_rejected(self) -> bool {
        matches!(self, Self::UnresolvedFixtureBlocked)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DelayedEventRow {
    pub owner_tag_db_id: u32,
    pub combined_event_id: u32,
    pub scope_tag_db_id: u32,
    pub originator_tag_db_id: u32,
    pub scope_random: u64,
    pub due_remaining_hours: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Domain51FrameInput {
    pub elapsed_wall_ms: u64,
    pub variable_scopes: Vec<VariableScopeRead>,
    pub flag_scopes: Vec<FlagScopeRead>,
    pub delayed_events: OpaqueRootState,
    pub scripted_gui_open_state: OpaqueRootState,
    pub delayed_event_rows: Vec<DelayedEventRow>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Domain51CollectedFrame {
    pub domain_id: u16,
    pub elapsed_wall_ms: u64,
    pub payload: Vec<u8>,
    pub rows: Vec<compact::ScriptedStateRow>,
    pub row_count: u32,
    pub status_flags: u32,
    pub observed_scopes: u32,
    pub measured_scopes: u32,
    pub rejected_scopes: u32,
    pub validated_scope_partial_failure: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct A13FrameInput {
    pub elapsed_wall_ms: u64,
    pub executable_sha256: [u8; 32],
    pub game_hour_raw: i32,
    pub observed_game_hour_before: i32,
    pub observed_game_hour_after: i32,
    pub source_sequence: u64,
    pub variable_scopes: Vec<VariableScopeRead>,
    pub flag_scopes: Vec<FlagScopeRead>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum A13CarrierError {
    WrongExecutable,
    InvalidScope(String),
    IncompleteRoster,
    DuplicateIdentity,
    RowCountOverflow,
    CrossedGameHour,
}

pub const EXACT_A729_EXECUTABLE_SHA256: [u8; 32] = [
    0x87, 0x0e, 0xbf, 0x1f, 0x61, 0x52, 0x62, 0xfa, 0xcc, 0xa4, 0x8a, 0x3b, 0x94, 0x77, 0x75, 0xc4,
    0x1f, 0x0a, 0x53, 0x38, 0xf3, 0x06, 0x8c, 0x40, 0x95, 0x82, 0x09, 0xf5, 0xc4, 0x1c, 0x65, 0xad,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Domain51RegistrationDescriptor {
    pub id: u16,
    pub canonical_name: &'static str,
    pub wire_name: &'static str,
    pub status_before_integration: &'static str,
    pub status_when_registered: &'static str,
    pub schedulable_when_registered: bool,
    pub requires_data_record: bool,
}

pub fn registration_descriptor() -> Domain51RegistrationDescriptor {
    Domain51RegistrationDescriptor {
        id: compact::DOMAIN_ID,
        canonical_name: compact::DOMAIN_NAME,
        wire_name: compact::DOMAIN_NAME,
        status_before_integration: "reserved",
        status_when_registered: "active_integration",
        schedulable_when_registered: true,
        requires_data_record: true,
    }
}

pub fn collect_a13_domain51_frame(
    mut input: A13FrameInput,
) -> Result<Domain51CollectedFrame, A13CarrierError> {
    if input.executable_sha256 != EXACT_A729_EXECUTABLE_SHA256 {
        return Err(A13CarrierError::WrongExecutable);
    }
    if input.source_sequence == 0
        || input.observed_game_hour_before != input.game_hour_raw
        || input.observed_game_hour_after != input.game_hour_raw
    {
        return Err(A13CarrierError::CrossedGameHour);
    }
    canonicalise_variable_scopes(&mut input.variable_scopes)?;
    canonicalise_flag_scopes(&mut input.flag_scopes)?;
    let variable_roster = scope_roster(&input.variable_scopes)?;
    let flag_roster = scope_roster(&input.flag_scopes)?;
    if variable_roster != flag_roster
        || !variable_roster.contains(&(compact::SCOPE_GLOBAL, 0))
        || !variable_roster
            .iter()
            .any(|(scope_kind, _)| *scope_kind == compact::SCOPE_COUNTRY)
        || !variable_roster
            .iter()
            .any(|(scope_kind, _)| *scope_kind == compact::SCOPE_STATE)
        || input
            .variable_scopes
            .iter()
            .any(|scope| !scope.status.is_measured())
        || input
            .flag_scopes
            .iter()
            .any(|scope| !scope.status.is_measured())
    {
        return Err(A13CarrierError::IncompleteRoster);
    }
    let row_count = input
        .variable_scopes
        .iter()
        .map(|scope| scope.entries.len())
        .chain(input.flag_scopes.iter().map(|scope| scope.entries.len()))
        .try_fold(0usize, |total, rows| total.checked_add(rows))
        .ok_or(A13CarrierError::RowCountOverflow)?;
    if u32::try_from(row_count).is_err() {
        return Err(A13CarrierError::RowCountOverflow);
    }
    Ok(collect_domain51_frame(Domain51FrameInput {
        elapsed_wall_ms: input.elapsed_wall_ms,
        variable_scopes: input.variable_scopes,
        flag_scopes: input.flag_scopes,
        delayed_events: OpaqueRootState::NotRequested,
        scripted_gui_open_state: OpaqueRootState::NotRequested,
        delayed_event_rows: Vec::new(),
    }))
}

fn scope_roster<T>(scopes: &[T]) -> Result<std::collections::BTreeSet<(u32, u32)>, A13CarrierError>
where
    T: A13Scope,
{
    let mut roster = std::collections::BTreeSet::new();
    for scope in scopes {
        validate_scope_identity(scope.scope_kind(), scope.owner_id())?;
        if !roster.insert((scope.scope_kind(), scope.owner_id())) {
            return Err(A13CarrierError::DuplicateIdentity);
        }
    }
    Ok(roster)
}

trait A13Scope {
    fn scope_kind(&self) -> u32;
    fn owner_id(&self) -> u32;
}

impl A13Scope for VariableScopeRead {
    fn scope_kind(&self) -> u32 {
        self.scope_kind
    }
    fn owner_id(&self) -> u32 {
        self.owner_id
    }
}

impl A13Scope for FlagScopeRead {
    fn scope_kind(&self) -> u32 {
        self.scope_kind
    }
    fn owner_id(&self) -> u32 {
        self.owner_id
    }
}

fn validate_scope_identity(scope_kind: u32, owner_id: u32) -> Result<(), A13CarrierError> {
    let valid = match scope_kind {
        compact::SCOPE_GLOBAL => owner_id == 0,
        compact::SCOPE_COUNTRY => owner_id != 0 && owner_id <= compact::MAX_COUNTRY_TAG_DB_ID,
        compact::SCOPE_STATE => owner_id != 0 && owner_id <= compact::MAX_STATE_ID,
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err(A13CarrierError::InvalidScope(format!(
            "unsupported or invalid A13 scope {scope_kind}:{owner_id}"
        )))
    }
}

fn canonicalise_variable_scopes(scopes: &mut [VariableScopeRead]) -> Result<(), A13CarrierError> {
    for scope in scopes.iter_mut() {
        validate_scope_identity(scope.scope_kind, scope.owner_id)?;
        validate_status_shape(scope.status, scope.entries.is_empty())?;
        scope
            .entries
            .sort_by(|left, right| left.key.cmp(&right.key));
        for entry in &scope.entries {
            validate_key(&entry.key)?;
        }
        if scope
            .entries
            .windows(2)
            .any(|rows| rows[0].key == rows[1].key)
        {
            return Err(A13CarrierError::DuplicateIdentity);
        }
    }
    scopes.sort_by_key(|scope| (scope.scope_kind, scope.owner_id));
    Ok(())
}

fn canonicalise_flag_scopes(scopes: &mut [FlagScopeRead]) -> Result<(), A13CarrierError> {
    for scope in scopes.iter_mut() {
        validate_scope_identity(scope.scope_kind, scope.owner_id)?;
        validate_status_shape(scope.status, scope.entries.is_empty())?;
        for entry in &scope.entries {
            let token_id = entry.token_id.ok_or_else(|| {
                A13CarrierError::InvalidScope("flag row lacks engine token identity".into())
            })?;
            if token_id == 0 || entry.date_hour_raw.is_none() || entry.value.is_none() {
                return Err(A13CarrierError::InvalidScope(
                    "flag row lacks token/date/value producer fields".into(),
                ));
            }
            if let Some(key) = entry.key.as_deref() {
                validate_key(key)?;
            }
        }
        scope.entries.sort_by(|left, right| {
            left.token_id
                .cmp(&right.token_id)
                .then_with(|| left.key.cmp(&right.key))
        });
        if scope
            .entries
            .windows(2)
            .any(|rows| rows[0].token_id == rows[1].token_id)
        {
            return Err(A13CarrierError::DuplicateIdentity);
        }
    }
    scopes.sort_by_key(|scope| (scope.scope_kind, scope.owner_id));
    Ok(())
}

fn validate_status_shape(status: ScopeReadStatus, empty: bool) -> Result<(), A13CarrierError> {
    let valid = match status {
        ScopeReadStatus::Measured => !empty,
        ScopeReadStatus::MeasuredEmpty | ScopeReadStatus::Unmeasured => empty,
    };
    if valid {
        Ok(())
    } else {
        Err(A13CarrierError::InvalidScope(
            "scope status disagrees with row presence".into(),
        ))
    }
}

fn validate_key(key: &str) -> Result<(), A13CarrierError> {
    if key.is_empty()
        || key.len() > compact::MAX_KEY_BYTES
        || !key.bytes().all(|byte| (32..=126).contains(&byte))
    {
        return Err(A13CarrierError::InvalidScope(
            "scripted key is empty, oversized, or non-portable".into(),
        ));
    }
    Ok(())
}

pub fn collect_domain51_frame(input: Domain51FrameInput) -> Domain51CollectedFrame {
    let mut rows = Vec::new();
    let mut status_flags = 0u32;
    let mut observed_scopes = 0u32;
    let mut measured_scopes = 0u32;
    let mut rejected_scopes = 0u32;
    let mut validated_scope_partial_failure = false;

    for scope in input.variable_scopes {
        observed_scopes = observed_scopes.saturating_add(1);
        if scope.status.is_measured() {
            measured_scopes = measured_scopes.saturating_add(1);
        } else {
            rejected_scopes = rejected_scopes.saturating_add(1);
            validated_scope_partial_failure = true;
            continue;
        }
        if scope.status == ScopeReadStatus::MeasuredEmpty {
            continue;
        }
        for entry in scope.entries {
            rows.push(compact::ScriptedStateRow {
                scope_kind: scope.scope_kind,
                owner_id: scope.owner_id,
                value_kind: compact::VALUE_VARIABLE_FIXED_I64,
                key: Some(entry.key),
                token_id: None,
                value_raw: Some(entry.value_raw),
                date_hour_raw: None,
                deleted: false,
                delayed_due_hours: None,
            });
        }
    }

    for scope in input.flag_scopes {
        observed_scopes = observed_scopes.saturating_add(1);
        if scope.status.is_measured() {
            measured_scopes = measured_scopes.saturating_add(1);
        } else {
            rejected_scopes = rejected_scopes.saturating_add(1);
            validated_scope_partial_failure = true;
            continue;
        }
        if scope.status == ScopeReadStatus::MeasuredEmpty {
            continue;
        }
        for entry in scope.entries {
            let value_kind = if entry.key.is_some() {
                compact::VALUE_FLAG_NAMED
            } else {
                status_flags |= compact::STATUS_FLAG_NAMES_PARTIAL;
                compact::VALUE_FLAG_TOKEN_ONLY
            };
            rows.push(compact::ScriptedStateRow {
                scope_kind: scope.scope_kind,
                owner_id: scope.owner_id,
                value_kind,
                key: entry.key,
                token_id: if value_kind == compact::VALUE_FLAG_TOKEN_ONLY {
                    entry.token_id
                } else {
                    None
                },
                value_raw: entry.value.map(i64::from),
                date_hour_raw: entry.date_hour_raw,
                deleted: false,
                delayed_due_hours: None,
            });
        }
    }

    let delayed_events_state = if input.delayed_event_rows.is_empty() {
        input.delayed_events
    } else {
        observed_scopes = observed_scopes.saturating_add(1);
        measured_scopes = measured_scopes.saturating_add(1);
        for row in &input.delayed_event_rows {
            rows.push(compact::ScriptedStateRow {
                scope_kind: compact::SCOPE_COUNTRY,
                owner_id: row.owner_tag_db_id,
                value_kind: compact::VALUE_DELAYED_EVENT,
                key: None,
                token_id: Some(row.combined_event_id),
                value_raw: Some(row.scope_random as i64),
                date_hour_raw: Some(row.scope_tag_db_id as i32),
                deleted: false,
                delayed_due_hours: row.due_remaining_hours,
            });
        }
        OpaqueRootState::MeasuredEmpty
    };
    append_opaque_root(
        delayed_events_state,
        compact::STATUS_DELAYED_EVENTS_ROOT_UNRESOLVED,
        compact::VALUE_DELAYED_EVENT_OPAQUE,
        DELAYED_EVENTS_BADGE_KEY,
        &mut rows,
        &mut status_flags,
        &mut observed_scopes,
        &mut measured_scopes,
        &mut rejected_scopes,
    );
    append_opaque_root(
        input.scripted_gui_open_state,
        compact::STATUS_SCRIPTED_GUI_ROOT_UNRESOLVED,
        compact::VALUE_SCRIPTED_GUI_OPAQUE,
        SCRIPTED_GUI_BADGE_KEY,
        &mut rows,
        &mut status_flags,
        &mut observed_scopes,
        &mut measured_scopes,
        &mut rejected_scopes,
    );
    rows.sort_by(compact::canonical_row_cmp);

    if validated_scope_partial_failure {
        status_flags |= compact::STATUS_PARTIAL_DATA;
    }

    let row_count = rows.len().min(u32::MAX as usize) as u32;
    let header = compact::ScriptedStateHeader {
        elapsed_wall_ms: input.elapsed_wall_ms,
        status_flags,
        changed_rows: row_count,
        observed_scopes,
        measured_scopes,
        rejected_scopes,
    };
    let payload = compact::encode_payload(&header, &rows);

    Domain51CollectedFrame {
        domain_id: compact::DOMAIN_ID,
        elapsed_wall_ms: input.elapsed_wall_ms,
        payload,
        rows,
        row_count,
        status_flags,
        observed_scopes,
        measured_scopes,
        rejected_scopes,
        validated_scope_partial_failure,
    }
}

fn append_opaque_root(
    state: OpaqueRootState,
    unresolved_status: u32,
    value_kind: u32,
    badge_key: &str,
    rows: &mut Vec<compact::ScriptedStateRow>,
    status_flags: &mut u32,
    observed_scopes: &mut u32,
    measured_scopes: &mut u32,
    rejected_scopes: &mut u32,
) {
    if !state.is_observed() {
        return;
    }
    *observed_scopes = observed_scopes.saturating_add(1);
    if state.is_measured() {
        *measured_scopes = measured_scopes.saturating_add(1);
        return;
    }
    if state.is_rejected() {
        *rejected_scopes = rejected_scopes.saturating_add(1);
        *status_flags |= unresolved_status;
        rows.push(compact::ScriptedStateRow {
            scope_kind: compact::SCOPE_GLOBAL,
            owner_id: 0,
            value_kind,
            key: Some(badge_key.to_string()),
            token_id: None,
            value_raw: None,
            date_hour_raw: None,
            deleted: false,
            delayed_due_hours: None,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn country_vars(owner_id: u32) -> VariableScopeRead {
        VariableScopeRead {
            scope_kind: compact::SCOPE_COUNTRY,
            owner_id,
            status: ScopeReadStatus::Measured,
            entries: vec![VariableEntry {
                key: "usa_congress_senate_support".to_string(),
                value_raw: 5_250_000,
            }],
        }
    }

    fn named_flag(owner_id: u32) -> FlagScopeRead {
        FlagScopeRead {
            scope_kind: compact::SCOPE_COUNTRY,
            owner_id,
            status: ScopeReadStatus::Measured,
            entries: vec![FlagEntry {
                key: Some("has_election".to_string()),
                token_id: Some(0x1111),
                date_hour_raw: Some(13_440),
                value: Some(1),
            }],
        }
    }

    fn complete_a13_input() -> A13FrameInput {
        A13FrameInput {
            elapsed_wall_ms: 3,
            executable_sha256: EXACT_A729_EXECUTABLE_SHA256,
            game_hour_raw: 13_440,
            observed_game_hour_before: 13_440,
            observed_game_hour_after: 13_440,
            source_sequence: 1,
            variable_scopes: vec![
                VariableScopeRead {
                    scope_kind: compact::SCOPE_GLOBAL,
                    owner_id: 0,
                    status: ScopeReadStatus::MeasuredEmpty,
                    entries: vec![],
                },
                country_vars(1),
                VariableScopeRead {
                    scope_kind: compact::SCOPE_STATE,
                    owner_id: 42,
                    status: ScopeReadStatus::MeasuredEmpty,
                    entries: vec![],
                },
            ],
            flag_scopes: vec![
                FlagScopeRead {
                    scope_kind: compact::SCOPE_GLOBAL,
                    owner_id: 0,
                    status: ScopeReadStatus::MeasuredEmpty,
                    entries: vec![],
                },
                named_flag(1),
                FlagScopeRead {
                    scope_kind: compact::SCOPE_STATE,
                    owner_id: 42,
                    status: ScopeReadStatus::MeasuredEmpty,
                    entries: vec![],
                },
            ],
        }
    }

    #[test]
    fn a13_carrier_requires_one_complete_same_hour_scope_roster() {
        let frame = collect_a13_domain51_frame(complete_a13_input()).unwrap();
        assert_eq!(frame.observed_scopes, 6);
        assert_eq!(frame.measured_scopes, 6);
        assert_eq!(frame.rejected_scopes, 0);
        assert_eq!(frame.status_flags & compact::STATUS_PARTIAL_DATA, 0);
        assert!(compact::decode_payload(0, &frame.payload)
            .parse_errors
            .is_empty());
    }

    #[test]
    fn a13_carrier_rejects_crossed_hour_and_unmeasured_scope() {
        let mut crossed = complete_a13_input();
        crossed.observed_game_hour_after += 1;
        assert_eq!(
            collect_a13_domain51_frame(crossed),
            Err(A13CarrierError::CrossedGameHour)
        );

        let mut partial = complete_a13_input();
        partial.variable_scopes[0].status = ScopeReadStatus::Unmeasured;
        assert_eq!(
            collect_a13_domain51_frame(partial),
            Err(A13CarrierError::IncompleteRoster)
        );
    }

    #[test]
    fn a13_carrier_rejects_arbitrary_engine_scopes_and_roster_mismatch() {
        let mut arbitrary = complete_a13_input();
        arbitrary.variable_scopes[0].scope_kind = 4;
        assert!(matches!(
            collect_a13_domain51_frame(arbitrary),
            Err(A13CarrierError::InvalidScope(_))
        ));

        let mut missing_state_flags = complete_a13_input();
        missing_state_flags.flag_scopes.pop();
        assert_eq!(
            collect_a13_domain51_frame(missing_state_flags),
            Err(A13CarrierError::IncompleteRoster)
        );
    }

    #[test]
    fn encodes_validated_country_global_state_variables_and_flags() {
        let frame = collect_domain51_frame(Domain51FrameInput {
            elapsed_wall_ms: 17,
            variable_scopes: vec![
                country_vars(1),
                VariableScopeRead {
                    scope_kind: compact::SCOPE_GLOBAL,
                    owner_id: 0,
                    status: ScopeReadStatus::Measured,
                    entries: vec![VariableEntry {
                        key: "global.test".to_string(),
                        value_raw: -100_000,
                    }],
                },
                VariableScopeRead {
                    scope_kind: compact::SCOPE_STATE,
                    owner_id: 123,
                    status: ScopeReadStatus::MeasuredEmpty,
                    entries: vec![],
                },
            ],
            flag_scopes: vec![
                named_flag(1),
                FlagScopeRead {
                    scope_kind: compact::SCOPE_GLOBAL,
                    owner_id: 0,
                    status: ScopeReadStatus::Measured,
                    entries: vec![FlagEntry {
                        key: Some("global_flag".to_string()),
                        token_id: Some(0x2222),
                        date_hour_raw: Some(13_441),
                        value: Some(1),
                    }],
                },
                FlagScopeRead {
                    scope_kind: compact::SCOPE_STATE,
                    owner_id: 123,
                    status: ScopeReadStatus::Measured,
                    entries: vec![FlagEntry {
                        key: Some("state_flag".to_string()),
                        token_id: Some(0x3333),
                        date_hour_raw: Some(13_442),
                        value: Some(1),
                    }],
                },
            ],
            delayed_events: OpaqueRootState::MeasuredEmpty,
            scripted_gui_open_state: OpaqueRootState::MeasuredEmpty,
            delayed_event_rows: vec![],
        });

        assert_eq!(frame.domain_id, 51);
        assert_eq!(frame.row_count, 5);
        assert_eq!(frame.status_flags & compact::STATUS_PARTIAL_DATA, 0);
        assert!(!frame.validated_scope_partial_failure);
        let summary = compact::decode_payload(0, &frame.payload);
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        assert_eq!(
            summary
                .data_units_total
                .get("scripted_variable_rows")
                .copied(),
            Some(2)
        );
        assert_eq!(
            summary.data_units_total.get("scripted_flag_rows").copied(),
            Some(3)
        );
    }

    #[test]
    fn measured_delayed_event_rows_emit_without_unresolved_badge() {
        let frame = collect_domain51_frame(Domain51FrameInput {
            elapsed_wall_ms: 9,
            variable_scopes: vec![],
            flag_scopes: vec![],
            delayed_events: OpaqueRootState::UnresolvedFixtureBlocked,
            scripted_gui_open_state: OpaqueRootState::NotRequested,
            delayed_event_rows: vec![
                DelayedEventRow {
                    owner_tag_db_id: 2,
                    combined_event_id: 28_200_333,
                    scope_tag_db_id: 2,
                    originator_tag_db_id: 68,
                    scope_random: 0x3723_2577_1394_6267,
                    due_remaining_hours: Some(755),
                },
                DelayedEventRow {
                    owner_tag_db_id: 68,
                    combined_event_id: 36_000_046,
                    scope_tag_db_id: 68,
                    originator_tag_db_id: 2,
                    scope_random: 0x1,
                    due_remaining_hours: None,
                },
            ],
        });

        assert_eq!(frame.row_count, 2);
        assert_eq!(
            frame.status_flags & compact::STATUS_DELAYED_EVENTS_ROOT_UNRESOLVED,
            0
        );
        assert_eq!(frame.status_flags & compact::STATUS_PARTIAL_DATA, 0);
        let summary = compact::decode_payload(0, &frame.payload);
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        assert_eq!(
            summary
                .data_units_total
                .get("scripted_delayed_event_rows")
                .copied(),
            Some(2)
        );
        assert_eq!(
            summary
                .data_units_total
                .get("scripted_delayed_event_opaque_rows")
                .copied(),
            Some(0)
        );
        assert_eq!(
            summary
                .data_units_total
                .get("scripted_delayed_event_due_rows")
                .copied(),
            Some(1)
        );
        assert_eq!(frame.status_flags & compact::STATUS_FLAG_NAMES_PARTIAL, 0);
    }

    #[test]
    fn positive_empty_rooted_delayed_events_clear_the_unresolved_badge() {
        let frame = collect_domain51_frame(Domain51FrameInput {
            elapsed_wall_ms: 4,
            variable_scopes: vec![country_vars(1)],
            flag_scopes: vec![named_flag(1)],
            delayed_events: OpaqueRootState::MeasuredEmpty,
            scripted_gui_open_state: OpaqueRootState::NotRequested,
            delayed_event_rows: vec![],
        });

        assert_eq!(
            frame.status_flags & compact::STATUS_DELAYED_EVENTS_ROOT_UNRESOLVED,
            0,
            "a positive empty rooted witness must not set the unresolved badge"
        );
        assert_eq!(frame.status_flags & compact::STATUS_PARTIAL_DATA, 0);
        assert_eq!(frame.status_flags & compact::STATUS_SOURCE_UNAVAILABLE, 0);
        assert!(!frame.validated_scope_partial_failure);
        let summary = compact::decode_payload(0, &frame.payload);
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        assert_eq!(
            summary
                .data_units_total
                .get("scripted_delayed_event_rows")
                .copied()
                .unwrap_or(0),
            0
        );
        assert_eq!(
            summary
                .data_units_total
                .get("scripted_delayed_event_opaque_rows")
                .copied()
                .unwrap_or(0),
            0
        );
    }

    #[test]
    fn repeated_event_id_with_distinct_seed_is_not_a_duplicate() {
        let frame = collect_domain51_frame(Domain51FrameInput {
            elapsed_wall_ms: 1,
            variable_scopes: vec![],
            flag_scopes: vec![],
            delayed_events: OpaqueRootState::NotRequested,
            scripted_gui_open_state: OpaqueRootState::NotRequested,
            delayed_event_rows: vec![
                DelayedEventRow {
                    owner_tag_db_id: 20,
                    combined_event_id: 9_200_000,
                    scope_tag_db_id: 20,
                    originator_tag_db_id: 1,
                    scope_random: 0xAAAA_BBBB_CCCC_DDDD,
                    due_remaining_hours: Some(755),
                },
                DelayedEventRow {
                    owner_tag_db_id: 20,
                    combined_event_id: 9_200_000,
                    scope_tag_db_id: 20,
                    originator_tag_db_id: 1,
                    scope_random: 0x1111_2222_3333_4444,
                    due_remaining_hours: Some(755),
                },
            ],
        });
        assert_eq!(frame.row_count, 2);
        let summary = compact::decode_payload(0, &frame.payload);
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        assert_eq!(
            summary
                .data_units_total
                .get("scripted_delayed_event_rows")
                .copied(),
            Some(2)
        );
    }

    #[test]
    fn delayed_event_rows_reject_out_of_range_ids() {
        let frame = collect_domain51_frame(Domain51FrameInput {
            elapsed_wall_ms: 1,
            variable_scopes: vec![],
            flag_scopes: vec![],
            delayed_events: OpaqueRootState::NotRequested,
            scripted_gui_open_state: OpaqueRootState::NotRequested,
            delayed_event_rows: vec![DelayedEventRow {
                owner_tag_db_id: 2,
                combined_event_id: 42,
                scope_tag_db_id: 2,
                originator_tag_db_id: 2,
                scope_random: 0,
                due_remaining_hours: Some(755),
            }],
        });
        let summary = compact::decode_payload(0, &frame.payload);
        assert!(summary
            .parse_errors
            .iter()
            .any(|error| error.contains("out-of-range event id")));
    }

    #[test]
    fn unresolved_delayed_and_scripted_gui_roots_are_badges_not_partial_failure() {
        let frame = collect_domain51_frame(Domain51FrameInput {
            elapsed_wall_ms: 3,
            variable_scopes: vec![country_vars(1)],
            flag_scopes: vec![named_flag(1)],
            delayed_events: OpaqueRootState::UnresolvedFixtureBlocked,
            scripted_gui_open_state: OpaqueRootState::UnresolvedFixtureBlocked,
            delayed_event_rows: vec![],
        });

        assert_eq!(
            frame.status_flags & compact::STATUS_DELAYED_EVENTS_ROOT_UNRESOLVED,
            compact::STATUS_DELAYED_EVENTS_ROOT_UNRESOLVED
        );
        assert_eq!(
            frame.status_flags & compact::STATUS_SCRIPTED_GUI_ROOT_UNRESOLVED,
            compact::STATUS_SCRIPTED_GUI_ROOT_UNRESOLVED
        );
        assert_eq!(frame.status_flags & compact::STATUS_PARTIAL_DATA, 0);
        assert_eq!(frame.status_flags & compact::STATUS_SOURCE_UNAVAILABLE, 0);
        assert!(!frame.validated_scope_partial_failure);

        let summary = compact::decode_payload(64, &frame.payload);
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        assert_eq!(
            summary
                .data_units_total
                .get("scripted_delayed_event_opaque_rows")
                .copied(),
            Some(1)
        );
        assert_eq!(
            summary
                .data_units_total
                .get("scripted_gui_opaque_rows")
                .copied(),
            Some(1)
        );
    }

    #[test]
    fn unmeasured_validated_scope_sets_partial_data() {
        let frame = collect_domain51_frame(Domain51FrameInput {
            elapsed_wall_ms: 5,
            variable_scopes: vec![VariableScopeRead {
                scope_kind: compact::SCOPE_COUNTRY,
                owner_id: 1,
                status: ScopeReadStatus::Unmeasured,
                entries: vec![],
            }],
            flag_scopes: vec![named_flag(1)],
            delayed_events: OpaqueRootState::UnresolvedFixtureBlocked,
            scripted_gui_open_state: OpaqueRootState::NotRequested,
            delayed_event_rows: vec![],
        });

        assert_ne!(frame.status_flags & compact::STATUS_PARTIAL_DATA, 0);
        assert!(frame.validated_scope_partial_failure);
        assert_eq!(frame.rejected_scopes, 2);
        let summary = compact::decode_payload(128, &frame.payload);
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
    }

    #[test]
    fn token_only_flags_badge_names_without_source_failure() {
        let frame = collect_domain51_frame(Domain51FrameInput {
            elapsed_wall_ms: 5,
            variable_scopes: vec![],
            flag_scopes: vec![FlagScopeRead {
                scope_kind: compact::SCOPE_COUNTRY,
                owner_id: 1,
                status: ScopeReadStatus::Measured,
                entries: vec![FlagEntry {
                    key: None,
                    token_id: Some(0xB5E5),
                    date_hour_raw: Some(13_443),
                    value: Some(1),
                }],
            }],
            delayed_events: OpaqueRootState::NotRequested,
            scripted_gui_open_state: OpaqueRootState::NotRequested,
            delayed_event_rows: vec![],
        });

        assert_ne!(frame.status_flags & compact::STATUS_FLAG_NAMES_PARTIAL, 0);
        assert_eq!(frame.status_flags & compact::STATUS_PARTIAL_DATA, 0);
        assert_eq!(frame.status_flags & compact::STATUS_SOURCE_UNAVAILABLE, 0);
        let summary = compact::decode_payload(256, &frame.payload);
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        assert_eq!(
            summary
                .data_units_total
                .get("scripted_token_only_flag_rows")
                .copied(),
            Some(1)
        );
    }

    #[test]
    fn registration_descriptor_targets_domain_51_registry_slot() {
        let descriptor = registration_descriptor();
        assert_eq!(descriptor.id, 51);
        assert_eq!(
            descriptor.canonical_name,
            "scripted_state_flags_variables_live"
        );
        assert_eq!(descriptor.status_before_integration, "reserved");
        assert!(descriptor.schedulable_when_registered);
        assert!(descriptor.requires_data_record);
    }
}
