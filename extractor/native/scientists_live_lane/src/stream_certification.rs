//! Canonical H4CR certification for the production domain-41 scientist slice.
//!
//! This module deliberately starts from the physical compact stream.  A loose
//! JSON manifest, a lane-only payload, or a detached row count is not proof.

use crate::{ScientistRow, MAX_CHARACTER_ID, MAX_SCIENTIST_DISPLAY_LEN, MAX_SCIENTIST_TOKEN_LEN};
use std::collections::{BTreeMap, BTreeSet};

pub const DOMAIN_PROJECTS_LIVE: u16 = 41;
pub const DOMAIN_CONTROL: u16 = 250;
pub const PROJECT_HEADER_EXT_PROGRESS_V2: u32 = 4;
pub const PROJECT_SOURCE_SCAN: u32 = 1;
pub const PROJECT_HEADER_PROGRESS_LEN: usize = 152;

const FACILITY_ROW_LEN: usize = 32;
const PROJECT_ROW_LEN: usize = 24;
const SCIENTIST_ROW_LEN: usize = 32;
const STATE_ROW_LEN: usize = 48;
const BREAKTHROUGH_ROW_LEN: usize = 16;
const FLAG_NAME_TOKEN: u32 = 1;
const FLAG_DISPLAY_NAME: u32 = 1 << 1;
const FLAG_TRAIT_COUNT: u32 = 1 << 2;
const FLAG_SKILLS: u32 = 1 << 3;
const FLAG_DELETED: u32 = 1 << 31;
const MAX_ROWS: usize = 1_000_000;
const MAX_HANDLE: u32 = 10_000_000;
const MAX_TRAITS: u32 = 256;
const MAX_SKILL: i32 = 1_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StreamCertificationPolicy {
    pub expected_perf_roster: BTreeSet<u16>,
    pub expected_data_roster: BTreeSet<u16>,
    pub require_nonempty_scientists: bool,
    /// Exact requested game-hour advancements. Production certification
    /// requires `N + 1` consecutive closed frames spanning exactly `N` hours.
    pub requested_game_hour_advancements: Option<u32>,
}

impl StreamCertificationPolicy {
    /// Exact roster derived from the shared production descriptor table.
    pub fn production(requested_game_hour_advancements: u32) -> Result<Self, String> {
        validate_requested_game_hours(requested_game_hour_advancements)?;
        let expected_perf_roster: BTreeSet<u16> = compact_contract::production_profile_domains()
            .into_iter()
            .map(|descriptor| descriptor.id)
            .collect();
        let expected_data_roster = expected_perf_roster
            .iter()
            .copied()
            .filter(|domain_id| compact_contract::requires_data_record(*domain_id))
            .collect();
        Ok(Self {
            expected_perf_roster,
            expected_data_roster,
            require_nonempty_scientists: true,
            requested_game_hour_advancements: Some(requested_game_hour_advancements),
        })
    }

    /// Small, production-wire-compatible fixture policy.  Only the roster is
    /// reduced; framing, D41 schema, source, fold, and close-boundary rules are
    /// identical to production.
    pub fn scientists_fixture() -> Self {
        Self {
            expected_perf_roster: BTreeSet::from([DOMAIN_PROJECTS_LIVE]),
            expected_data_roster: BTreeSet::from([DOMAIN_PROJECTS_LIVE]),
            require_nonempty_scientists: true,
            requested_game_hour_advancements: None,
        }
    }

    #[cfg(test)]
    fn scientists_fixture_with_requested_game_hours(
        requested_game_hour_advancements: u32,
    ) -> Result<Self, String> {
        validate_requested_game_hours(requested_game_hour_advancements)?;
        let mut policy = Self::scientists_fixture();
        policy.requested_game_hour_advancements = Some(requested_game_hour_advancements);
        Ok(policy)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StreamCertificationReport {
    pub ok: bool,
    pub problems: Vec<String>,
    pub physical_records: usize,
    pub closed_frames: usize,
    pub first_hour: Option<i32>,
    pub last_hour: Option<i32>,
    pub projects_data_records: usize,
    pub scientist_upserts: usize,
    pub scientist_deletes: usize,
    pub final_scientist_rows: usize,
    pub max_observed_scientists: u32,
    pub requested_game_hour_advancements: Option<u32>,
    pub actual_game_hour_advancements: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScientistMutation {
    Upsert(ScientistRow),
    Delete(u32),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProductionProjectsScientistsPayload {
    pub row_count: u32,
    pub changed_rows: u32,
    pub status_flags: u32,
    pub observed_scientists: u32,
    pub perf_observed_rows: u32,
    pub perf_emitted_units: u32,
    pub delta: bool,
    pub mutations: Vec<ScientistMutation>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PerfAuthority {
    row_count: u32,
    changed_rows: u32,
    unit_count: u32,
    changed_unit_count: u32,
    status_flags: u32,
}

/// Decode the exact production D41 progress-v2 payload and return its scientist
/// mutations.  All surrounding row families and the optional PLCF trailer are
/// structurally validated so an offset cannot be certified from a malformed
/// mixed-domain fixture.
pub fn decode_production_projects_scientists(
    payload: &[u8],
) -> Result<ProductionProjectsScientistsPayload, String> {
    if payload.len() < PROJECT_HEADER_PROGRESS_LEN {
        return Err(format!(
            "projects payload is {} byte(s), expected at least {PROJECT_HEADER_PROGRESS_LEN}",
            payload.len()
        ));
    }
    let row_count = u32_at(payload, 8)?;
    let changed_rows = u32_at(payload, 12)?;
    let status_flags = u32_at(payload, 16)?;
    let facility_count = count_at(payload, 20)?;
    let project_count = count_at(payload, 24)?;
    let scientist_count = count_at(payload, 28)?;
    let observed_facilities = u32_at(payload, 32)? as usize;
    let observed_projects = u32_at(payload, 36)? as usize;
    let observed_scientists = u32_at(payload, 40)?;
    let bridge_rows = u32_at(payload, 44)?;
    let source_kind = u32_at(payload, 48)?;
    let extension = u32_at(payload, 52)?;
    let state_count = count_at(payload, 136)?;
    let breakthrough_count = count_at(payload, 140)?;
    let observed_states = u32_at(payload, 144)? as usize;
    let observed_breakthrough = u32_at(payload, 148)? as usize;
    let perf_observed_rows = u32::try_from(observed_facilities)
        .ok()
        .and_then(|value| value.checked_add(observed_projects as u32))
        .and_then(|value| value.checked_add(observed_scientists))
        .ok_or_else(|| "projects PERF observed-row authority overflow".to_string())?;
    let perf_emitted_units = u32::try_from(facility_count)
        .ok()
        .and_then(|value| value.checked_add(project_count as u32))
        .and_then(|value| value.checked_add(scientist_count as u32))
        .ok_or_else(|| "projects PERF emitted-unit authority overflow".to_string())?;

    if extension != PROJECT_HEADER_EXT_PROGRESS_V2 {
        return Err(format!(
            "projects header extension {extension} is not production progress-v2"
        ));
    }
    if source_kind != PROJECT_SOURCE_SCAN || bridge_rows != 0 {
        return Err(format!(
            "projects source is not exact DLL scan authority (source_kind {source_kind}, bridge_rows {bridge_rows})"
        ));
    }
    reject_status(status_flags, "projects DATA")?;
    let delta = status_flags & compact_contract::COMPACT_STATUS_DELTA_DATA != 0;
    let counts = [
        facility_count,
        project_count,
        scientist_count,
        state_count,
        breakthrough_count,
    ];
    let total = counts
        .iter()
        .try_fold(0usize, |sum, value| sum.checked_add(*value))
        .ok_or_else(|| "projects row-count overflow".to_string())?;
    if total != row_count as usize || total > MAX_ROWS {
        return Err(format!(
            "projects row_count {row_count} does not equal mixed-family total {total}"
        ));
    }
    if changed_rows > row_count || (delta && changed_rows != row_count) {
        return Err(format!(
            "projects changed_rows {changed_rows} is inconsistent with row_count {row_count} and delta={delta}"
        ));
    }
    if !delta {
        for (name, emitted, observed) in [
            ("facility", facility_count, observed_facilities),
            ("project", project_count, observed_projects),
            ("scientist", scientist_count, observed_scientists as usize),
            ("state", state_count, observed_states),
            ("breakthrough", breakthrough_count, observed_breakthrough),
        ] {
            if emitted != observed {
                return Err(format!(
                    "clean full projects {name} rows {emitted} != observed {observed}"
                ));
            }
        }
    }
    // A clean certification cannot hide root failures in counters while leaving
    // the status word clear.  Neutral/absent optional roots (76..84) are valid.
    for offset in [
        64usize, 68, 72, 88, 92, 96, 100, 104, 108, 112, 116, 120, 124, 128, 132,
    ] {
        let value = u32_at(payload, offset)?;
        if value != 0 {
            return Err(format!(
                "projects root-failure counter at header +{offset:#x} is nonzero ({value})"
            ));
        }
    }

    let facility_start = PROJECT_HEADER_PROGRESS_LEN;
    let project_start = checked_rows_end(facility_start, facility_count, FACILITY_ROW_LEN)?;
    let scientist_start = checked_rows_end(project_start, project_count, PROJECT_ROW_LEN)?;
    let state_start = checked_rows_end(scientist_start, scientist_count, SCIENTIST_ROW_LEN)?;
    let breakthrough_start = checked_rows_end(state_start, state_count, STATE_ROW_LEN)?;
    let strings_start =
        checked_rows_end(breakthrough_start, breakthrough_count, BREAKTHROUGH_ROW_LEN)?;
    if strings_start > payload.len() {
        return Err("projects fixed rows are truncated".to_string());
    }

    let mut facility_flags = Vec::with_capacity(facility_count);
    let mut seen_facilities = BTreeSet::new();
    for index in 0..facility_count {
        let cursor = facility_start + index * FACILITY_ROW_LEN;
        let id = u32_at(payload, cursor)?;
        let province = u32_at(payload, cursor + 4)?;
        let flags = u32_at(payload, cursor + 8)?;
        let active_project = u32_at(payload, cursor + 12)?;
        let scientist_id = u32_at(payload, cursor + 16)?;
        let unread_rewards = u32_at(payload, cursor + 20)?;
        if id == 0 || id > MAX_HANDLE || !seen_facilities.insert(id) {
            return Err(format!(
                "projects facility row {index} has invalid/duplicate id {id}"
            ));
        }
        if flags & !(1 | (1 << 1) | (1 << 2) | FLAG_DELETED) != 0 {
            return Err(format!(
                "projects facility row {index} has unknown flags {flags:#x}"
            ));
        }
        if u32_at(payload, cursor + 24)? != 0 || u32_at(payload, cursor + 28)? != 0 {
            return Err(format!(
                "projects facility row {index} has nonzero reserved fields"
            ));
        }
        if flags & FLAG_DELETED != 0 {
            if !delta
                || flags != FLAG_DELETED
                || province != 0
                || active_project != 0
                || scientist_id != 0
                || unread_rewards != 0
            {
                return Err(format!("projects facility tombstone {id} carries live authority or appears in a full frame"));
            }
        } else if province == 0
            || active_project > MAX_HANDLE
            || scientist_id > MAX_HANDLE
            || (scientist_id > 0) != (flags & (1 << 1) != 0)
        {
            return Err(format!(
                "projects facility row {index} has inconsistent live fields"
            ));
        }
        facility_flags.push(flags);
    }

    let mut seen_projects = BTreeSet::new();
    for index in 0..project_count {
        let cursor = project_start + index * PROJECT_ROW_LEN;
        let id = u32_at(payload, cursor)?;
        let static_handle = u32_at(payload, cursor + 4)?;
        let iterations = u32_at(payload, cursor + 8)?;
        let worked_at = u32_at(payload, cursor + 12)?;
        let flags = u32_at(payload, cursor + 16)?;
        if id == 0 || id > MAX_HANDLE || !seen_projects.insert(id) {
            return Err(format!(
                "projects row {index} has invalid/duplicate id {id}"
            ));
        }
        if flags & !(1 | FLAG_DELETED) != 0 || u32_at(payload, cursor + 20)? != 0 {
            return Err(format!(
                "projects row {index} has invalid flags/reserved fields"
            ));
        }
        if flags & FLAG_DELETED != 0 {
            if !delta
                || flags != FLAG_DELETED
                || static_handle != 0
                || iterations != 0
                || worked_at != 0
            {
                return Err(format!(
                    "projects tombstone {id} carries live authority or appears in a full frame"
                ));
            }
        } else if static_handle == 0
            || static_handle > MAX_HANDLE
            || worked_at > MAX_HANDLE
            || (worked_at > 0) != (flags & 1 != 0)
        {
            return Err(format!("projects row {index} has inconsistent live fields"));
        }
    }

    let mut scientist_shapes = Vec::with_capacity(scientist_count);
    let mut mutations = Vec::with_capacity(scientist_count);
    let mut seen_scientists = BTreeSet::new();
    for index in 0..scientist_count {
        let cursor = scientist_start + index * SCIENTIST_ROW_LEN;
        let char_id = u32_at(payload, cursor)?;
        let flags = u32_at(payload, cursor + 4)?;
        let trait_count = u32_at(payload, cursor + 8)?;
        let skills = [
            i32_at(payload, cursor + 12)?,
            i32_at(payload, cursor + 16)?,
            i32_at(payload, cursor + 20)?,
            i32_at(payload, cursor + 24)?,
        ];
        if char_id == 0 || char_id > MAX_CHARACTER_ID || !seen_scientists.insert(char_id) {
            return Err(format!(
                "projects scientist row {index} has invalid/duplicate char_id {char_id}"
            ));
        }
        if flags
            & !(FLAG_NAME_TOKEN | FLAG_DISPLAY_NAME | FLAG_TRAIT_COUNT | FLAG_SKILLS | FLAG_DELETED)
            != 0
            || u32_at(payload, cursor + 28)? != 0
        {
            return Err(format!(
                "projects scientist row {index} has invalid flags/reserved field"
            ));
        }
        let deleted = flags & FLAG_DELETED != 0;
        if deleted {
            if !delta || flags != FLAG_DELETED || trait_count != 0 || skills != [0; 4] {
                return Err(format!("projects scientist tombstone {char_id} carries live fields or appears in a full frame"));
            }
            mutations.push(ScientistMutation::Delete(char_id));
        } else {
            if flags & FLAG_SKILLS == 0 {
                return Err(format!(
                    "projects scientist row {index} omits the required skills flag"
                ));
            }
            if (trait_count == u32::MAX) != (flags & FLAG_TRAIT_COUNT == 0)
                || (trait_count != u32::MAX && trait_count > MAX_TRAITS)
            {
                return Err(format!(
                    "projects scientist row {index} has inconsistent trait authority"
                ));
            }
            if skills
                .into_iter()
                .any(|skill| !(0..=MAX_SKILL).contains(&skill))
            {
                return Err(format!(
                    "projects scientist row {index} has invalid skill level"
                ));
            }
            mutations.push(ScientistMutation::Upsert(ScientistRow {
                char_id,
                name_token: None,
                display_name: None,
                specialization_nuclear: skills[0],
                specialization_naval: skills[1],
                specialization_air: skills[2],
                specialization_land: skills[3],
                trait_count: (trait_count != u32::MAX).then_some(trait_count),
            }));
        }
        scientist_shapes.push((flags, deleted));
    }

    validate_state_rows(payload, state_start, state_count, delta)?;
    validate_breakthrough_rows(payload, breakthrough_start, breakthrough_count, delta)?;

    let mut cursor = strings_start;
    for (index, flags) in facility_flags.into_iter().enumerate() {
        let building = read_string(payload, &mut cursor, 160, "facility building")?;
        let present = building.is_some();
        if present != (flags & (1 << 2) != 0) || (flags & FLAG_DELETED != 0 && present) {
            return Err(format!(
                "projects facility row {index} has inconsistent building string authority"
            ));
        }
    }
    for (index, (flags, deleted)) in scientist_shapes.into_iter().enumerate() {
        let token = read_string(
            payload,
            &mut cursor,
            MAX_SCIENTIST_TOKEN_LEN,
            "scientist token",
        )?;
        let display = read_string(
            payload,
            &mut cursor,
            MAX_SCIENTIST_DISPLAY_LEN,
            "scientist display",
        )?;
        if token.is_some() != (flags & FLAG_NAME_TOKEN != 0)
            || display.is_some() != (flags & FLAG_DISPLAY_NAME != 0)
            || (deleted && (token.is_some() || display.is_some()))
        {
            return Err(format!(
                "projects scientist row {index} has inconsistent string authority"
            ));
        }
        if let Some(token) = token.as_deref() {
            if !super::is_script_token(token) {
                return Err(format!(
                    "projects scientist row {index} token is not a canonical script token"
                ));
            }
        }
        if let ScientistMutation::Upsert(row) = &mut mutations[index] {
            row.name_token = token;
            row.display_name = display;
        }
    }
    validate_optional_lifecycle_trailer(payload, cursor, delta)?;

    Ok(ProductionProjectsScientistsPayload {
        row_count,
        changed_rows,
        status_flags,
        observed_scientists,
        perf_observed_rows,
        perf_emitted_units,
        delta,
        mutations,
    })
}

pub fn certify_canonical_stream(
    bytes: &[u8],
    policy: &StreamCertificationPolicy,
) -> StreamCertificationReport {
    let mut report = StreamCertificationReport {
        ok: false,
        problems: Vec::new(),
        physical_records: 0,
        closed_frames: 0,
        first_hour: None,
        last_hour: None,
        projects_data_records: 0,
        scientist_upserts: 0,
        scientist_deletes: 0,
        final_scientist_rows: 0,
        max_observed_scientists: 0,
        requested_game_hour_advancements: policy.requested_game_hour_advancements,
        actual_game_hour_advancements: None,
    };
    if !policy.expected_perf_roster.contains(&DOMAIN_PROJECTS_LIVE)
        || !policy.expected_data_roster.contains(&DOMAIN_PROJECTS_LIVE)
    {
        report
            .problems
            .push("certification policy omits required domain 41".to_string());
        return report;
    }
    if policy
        .expected_data_roster
        .iter()
        .any(|id| !policy.expected_perf_roster.contains(id))
    {
        report
            .problems
            .push("data roster is not a subset of PERF roster".to_string());
        return report;
    }
    if let Some(requested) = policy.requested_game_hour_advancements {
        if let Err(error) = validate_requested_game_hours(requested) {
            report.problems.push(error);
            return report;
        }
    }

    let mut offset = 0usize;
    let mut prior_sequence = None;
    let mut open_hour = None;
    let mut last_closed_hour: Option<i32> = None;
    let mut perf = BTreeMap::<u16, PerfAuthority>::new();
    let mut data_roster = BTreeSet::new();
    let mut projects_data = None;
    let mut scientists = BTreeMap::<u32, ScientistRow>::new();
    let mut baseline_seen = false;
    while offset < bytes.len() {
        let header = match compact_contract::parse_compact_record_header(&bytes[offset..]) {
            Ok(header) => header,
            Err(error) => {
                report.problems.push(format!("offset {offset}: {error}"));
                break;
            }
        };
        let Some(end) = offset.checked_add(header.record_len()) else {
            report
                .problems
                .push(format!("offset {offset}: record length overflow"));
            break;
        };
        if end > bytes.len() {
            report.problems.push(format!(
                "offset {offset}: record is truncated at physical EOF"
            ));
            break;
        }
        let expected_sequence =
            prior_sequence.map_or(1, |sequence: u64| sequence.saturating_add(1));
        if header.sequence != expected_sequence || prior_sequence == Some(u64::MAX) {
            report.problems.push(format!(
                "offset {offset}: sequence {} != expected {expected_sequence}",
                header.sequence
            ));
            break;
        }
        prior_sequence = Some(header.sequence);
        let payload = &bytes[offset + compact_contract::COMPACT_RECORD_HEADER_LEN..end];
        if let Err(error) = compact_contract::validate_compact_record_semantics(
            header.domain_id,
            header.record_type,
            header.record_flags,
            payload,
        ) {
            report.problems.push(format!("offset {offset}: {error}"));
            break;
        }
        report.physical_records += 1;

        if header.record_type != compact_contract::COMPACT_RECORD_TYPE_FRAME_END {
            match open_hour {
                Some(hour) if hour != header.game_hour_raw => {
                    report.problems.push(format!(
                        "offset {offset}: hour {} arrives while hour {hour} is open",
                        header.game_hour_raw
                    ));
                    break;
                }
                None => {
                    if let Some(closed) = last_closed_hour {
                        if closed.checked_add(1) != Some(header.game_hour_raw) {
                            report.problems.push(format!(
                                "offset {offset}: nonconsecutive game hour {} after {closed}",
                                header.game_hour_raw
                            ));
                            break;
                        }
                    }
                    open_hour = Some(header.game_hour_raw);
                }
                Some(_) => {}
            }
        }

        match header.record_type {
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA => {
                if header.domain_id == DOMAIN_CONTROL {
                    // Control/latch diagnostics are outside the scheduled roster.
                } else if !policy.expected_data_roster.contains(&header.domain_id) {
                    report.problems.push(format!(
                        "offset {offset}: unexpected DATA domain {}",
                        header.domain_id
                    ));
                    break;
                } else if !data_roster.insert(header.domain_id) {
                    report.problems.push(format!(
                        "offset {offset}: duplicate DATA domain {}",
                        header.domain_id
                    ));
                    break;
                }
                if header.domain_id == DOMAIN_PROJECTS_LIVE {
                    match decode_production_projects_scientists(payload) {
                        Ok(decoded) => {
                            report.projects_data_records += 1;
                            projects_data = Some(decoded);
                        }
                        Err(error) => {
                            report.problems.push(format!("offset {offset}: {error}"));
                            break;
                        }
                    }
                }
            }
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF => {
                if !policy.expected_perf_roster.contains(&header.domain_id) {
                    report.problems.push(format!(
                        "offset {offset}: unexpected PERF domain {}",
                        header.domain_id
                    ));
                    break;
                }
                match parse_perf(payload) {
                    Ok(authority) => {
                        if let Err(error) = reject_status(authority.status_flags, "DOMAIN_PERF") {
                            report.problems.push(format!("offset {offset}: {error}"));
                            break;
                        }
                        if perf.insert(header.domain_id, authority).is_some() {
                            report.problems.push(format!(
                                "offset {offset}: duplicate PERF domain {}",
                                header.domain_id
                            ));
                            break;
                        }
                    }
                    Err(error) => {
                        report.problems.push(format!("offset {offset}: {error}"));
                        break;
                    }
                }
            }
            compact_contract::COMPACT_RECORD_TYPE_FRAME_END => {
                let Some(hour) = open_hour else {
                    report
                        .problems
                        .push(format!("offset {offset}: bare FRAME_END"));
                    break;
                };
                if hour != header.game_hour_raw {
                    report.problems.push(format!(
                        "offset {offset}: FRAME_END hour {} != open hour {hour}",
                        header.game_hour_raw
                    ));
                    break;
                }
                if perf.keys().copied().collect::<BTreeSet<_>>() != policy.expected_perf_roster {
                    report.problems.push(format!(
                        "offset {offset}: PERF roster does not exactly match certification policy"
                    ));
                    break;
                }
                if data_roster != policy.expected_data_roster {
                    report.problems.push(format!(
                        "offset {offset}: DATA roster does not exactly match certification policy"
                    ));
                    break;
                }
                let Some(data) = projects_data.take() else {
                    report
                        .problems
                        .push(format!("offset {offset}: frame has no D41 DATA authority"));
                    break;
                };
                let Some(project_perf) = perf.get(&DOMAIN_PROJECTS_LIVE).copied() else {
                    report
                        .problems
                        .push(format!("offset {offset}: frame has no D41 PERF authority"));
                    break;
                };
                if (
                    data.perf_observed_rows,
                    data.changed_rows,
                    data.perf_emitted_units,
                    data.changed_rows,
                    data.status_flags,
                ) != (
                    project_perf.row_count,
                    project_perf.changed_rows,
                    project_perf.unit_count,
                    project_perf.changed_unit_count,
                    project_perf.status_flags,
                ) {
                    report.problems.push(format!(
                        "offset {offset}: D41 DATA/PERF observed rows, emitted units, changed rows, or status mismatch"
                    ));
                    break;
                }
                if let Err(error) = validate_frame_end(payload, &perf) {
                    report.problems.push(format!("offset {offset}: {error}"));
                    break;
                }
                if data.delta && !baseline_seen {
                    report
                        .problems
                        .push(format!("offset {offset}: D41 delta precedes full baseline"));
                    break;
                }
                if !data.delta {
                    scientists.clear();
                    baseline_seen = true;
                }
                for mutation in data.mutations {
                    match mutation {
                        ScientistMutation::Upsert(row) => {
                            report.scientist_upserts += 1;
                            scientists.insert(row.char_id, row);
                        }
                        ScientistMutation::Delete(char_id) => {
                            report.scientist_deletes += 1;
                            if scientists.remove(&char_id).is_none() {
                                report.problems.push(format!("offset {offset}: scientist tombstone {char_id} has no logical baseline row"));
                                break;
                            }
                        }
                    }
                }
                if !report.problems.is_empty() {
                    break;
                }
                if scientists.len() != data.observed_scientists as usize {
                    report.problems.push(format!(
                        "offset {offset}: folded scientist roster {} != source-observed {}",
                        scientists.len(),
                        data.observed_scientists
                    ));
                    break;
                }
                report.max_observed_scientists =
                    report.max_observed_scientists.max(data.observed_scientists);
                report.first_hour.get_or_insert(hour);
                report.last_hour = Some(hour);
                report.closed_frames += 1;
                open_hour = None;
                last_closed_hour = Some(hour);
                perf.clear();
                data_roster.clear();
            }
            _ => unreachable!("shared semantics rejected unknown record type"),
        }
        offset = end;
    }
    if report.problems.is_empty() {
        if offset != bytes.len() {
            report
                .problems
                .push("stream parser stopped before physical EOF".to_string());
        } else if open_hour.is_some() {
            report
                .problems
                .push("stream ends with an open frame".to_string());
        } else if report.closed_frames == 0 {
            report.problems.push("stream closes no frames".to_string());
        } else if policy.require_nonempty_scientists && report.max_observed_scientists == 0 {
            report
                .problems
                .push("stream never proves a nonempty scientist roster".to_string());
        }
    }
    report.actual_game_hour_advancements = report
        .first_hour
        .zip(report.last_hour)
        .and_then(|(first, last)| last.checked_sub(first))
        .and_then(|span| u32::try_from(span).ok());
    if report.problems.is_empty() {
        if let Some(requested) = policy.requested_game_hour_advancements {
            let Some(expected_frames) = requested
                .checked_add(1)
                .and_then(|frames| usize::try_from(frames).ok())
            else {
                report
                    .problems
                    .push("requested duration frame-count overflow".to_string());
                report.final_scientist_rows = scientists.len();
                return report;
            };
            if report.closed_frames != expected_frames {
                report.problems.push(format!(
                    "closed frame count {} != requested advancements {requested} + 1 ({expected_frames})",
                    report.closed_frames
                ));
            }
            if report.actual_game_hour_advancements != Some(requested) {
                report.problems.push(format!(
                    "stream hour span {:?} != requested advancements {requested}",
                    report.actual_game_hour_advancements
                ));
            }
        }
    }
    report.final_scientist_rows = scientists.len();
    report.ok = report.problems.is_empty();
    report
}

fn validate_requested_game_hours(requested: u32) -> Result<(), String> {
    if requested == 0 || requested > i32::MAX as u32 {
        return Err(format!(
            "requested game-hour advancements must be in 1..={}, got {requested}",
            i32::MAX
        ));
    }
    Ok(())
}

fn parse_perf(payload: &[u8]) -> Result<PerfAuthority, String> {
    if payload.len() < compact_contract::COMPACT_DOMAIN_PERF_BASE_LEN {
        return Err("DOMAIN_PERF payload is short".to_string());
    }
    Ok(PerfAuthority {
        row_count: u32_at(payload, 16)?,
        changed_rows: u32_at(payload, 20)?,
        unit_count: u32_at(payload, 24)?,
        changed_unit_count: u32_at(payload, 28)?,
        status_flags: u32_at(payload, 32)?,
    })
}

fn validate_frame_end(payload: &[u8], perf: &BTreeMap<u16, PerfAuthority>) -> Result<(), String> {
    let emitted = u32_at(payload, 16)? as usize;
    let scheduled = u32_at(payload, 20)? as usize;
    let status = u32_at(payload, 24)?;
    let reserved = u32_at(payload, 28)?;
    reject_status(status, "FRAME_END")?;
    let expected_status = perf.iter().fold(0u32, |status_or, (domain_id, authority)| {
        status_or
            | compact_contract::compact_status_for_global_interpretation(
                *domain_id,
                authority.status_flags,
            )
    });
    if status != expected_status {
        return Err(format!(
            "FRAME_END status {status:#x} != normalized PERF status OR {expected_status:#x}"
        ));
    }
    if reserved != 0 || emitted != scheduled || emitted != perf.len() {
        return Err(format!("FRAME_END roster authority mismatch (emitted {emitted}, scheduled {scheduled}, PERF {}, reserved {reserved})", perf.len()));
    }
    Ok(())
}

fn reject_status(status: u32, label: &str) -> Result<(), String> {
    let unknown = compact_contract::compact_status_unknown_flags_v1(status);
    if unknown != 0 {
        return Err(format!(
            "{label} status {status:#x} has unknown bits {unknown:#x}"
        ));
    }
    let failure = status & compact_contract::COMPACT_STATUS_FAILURE_MASK;
    if failure != 0 {
        return Err(format!(
            "{label} status {status:#x} has failure bits {failure:#x}"
        ));
    }
    Ok(())
}

fn validate_state_rows(
    payload: &[u8],
    start: usize,
    count: usize,
    delta: bool,
) -> Result<(), String> {
    let mut seen = BTreeSet::new();
    for index in 0..count {
        let cursor = start + index * STATE_ROW_LEN;
        let id = u32_at(payload, cursor)?;
        let static_handle = u32_at(payload, cursor + 4)?;
        let current_offset = u32_at(payload, cursor + 8)?;
        let enums = [
            u32_at(payload, cursor + 12)?,
            u32_at(payload, cursor + 16)?,
            u32_at(payload, cursor + 20)?,
            u32_at(payload, cursor + 24)?,
            u32_at(payload, cursor + 28)?,
        ];
        let iterations = u32_at(payload, cursor + 32)?;
        let progress = u32_at(payload, cursor + 36)?;
        let target = u32_at(payload, cursor + 40)?;
        let flags = u32_at(payload, cursor + 44)?;
        if id == 0 || id > MAX_HANDLE || !seen.insert(id) || flags & !(1 | FLAG_DELETED) != 0 {
            return Err(format!(
                "projects state row {index} has invalid identity/flags"
            ));
        }
        if flags & FLAG_DELETED != 0 {
            if !delta
                || flags != FLAG_DELETED
                || static_handle != 0
                || current_offset != 0
                || enums != [0; 5]
                || iterations != 0
                || progress != 0
                || target != 0
            {
                return Err(format!("projects state tombstone {id} carries live authority or appears in a full frame"));
            }
        } else if static_handle == 0
            || static_handle > MAX_HANDLE
            || !matches!(current_offset, 0x18 | 0xE8 | 0x168 | 0x1E8)
            || enums.into_iter().any(|value| value > 64)
            || target == 0
        {
            return Err(format!(
                "projects state row {index} has inconsistent live fields"
            ));
        }
    }
    Ok(())
}

fn validate_breakthrough_rows(
    payload: &[u8],
    start: usize,
    count: usize,
    delta: bool,
) -> Result<(), String> {
    let mut seen = BTreeSet::new();
    for index in 0..count {
        let cursor = start + index * BREAKTHROUGH_ROW_LEN;
        let tag = u32_at(payload, cursor)?;
        let handle = u32_at(payload, cursor + 4)?;
        let points = u32_at(payload, cursor + 8)?;
        let flags = u32_at(payload, cursor + 12)?;
        if tag == 0
            || handle == 0
            || handle > MAX_HANDLE
            || !seen.insert((tag, handle))
            || flags & !FLAG_DELETED != 0
        {
            return Err(format!(
                "projects breakthrough row {index} has invalid identity/flags"
            ));
        }
        if flags & FLAG_DELETED != 0 && (!delta || points != 0) {
            return Err(
                "projects breakthrough tombstone carries live authority or appears in a full frame"
                    .to_string(),
            );
        }
    }
    Ok(())
}

fn validate_optional_lifecycle_trailer(
    payload: &[u8],
    cursor: usize,
    delta: bool,
) -> Result<(), String> {
    if cursor == payload.len() {
        return Ok(());
    }
    let trailer = payload
        .get(cursor..)
        .ok_or_else(|| "lifecycle trailer offset overflow".to_string())?;
    if trailer.len() < 20
        || &trailer[..4] != b"PLCF"
        || u16_at(trailer, 4)? != 1
        || u16_at(trailer, 6)? != 20
    {
        return Err("projects trailing bytes are not a canonical PLCF v1 trailer".to_string());
    }
    let count = u32_at(trailer, 8)? as usize;
    let lifecycle_countries = u32_at(trailer, 12)? as usize;
    let aborted_countries = u32_at(trailer, 16)? as usize;
    if aborted_countries != 0 {
        return Err(format!(
            "projects PLCF reports {aborted_countries} aborted lifecycle countries"
        ));
    }
    if count > 0 && lifecycle_countries == 0 {
        return Err("projects PLCF has rows but zero measured lifecycle countries".to_string());
    }
    let expected = checked_rows_end(20, count, 12)?;
    if expected != trailer.len() {
        return Err("projects PLCF trailer length mismatch".to_string());
    }
    let mut seen = BTreeSet::new();
    for index in 0..count {
        let row = 20 + index * 12;
        let key = (u32_at(trailer, row)?, u32_at(trailer, row + 4)?);
        let flags = u32_at(trailer, row + 8)?;
        if key.0 == 0 || key.1 == 0 || !seen.insert(key) || flags & !(1 | 2 | FLAG_DELETED) != 0 {
            return Err(format!(
                "projects PLCF row {index} has invalid identity/flags"
            ));
        }
        if flags & FLAG_DELETED != 0 && (!delta || flags != FLAG_DELETED) {
            return Err(format!("projects PLCF tombstone row {index} is invalid"));
        }
    }
    let represented_countries = seen
        .iter()
        .map(|(tag, _)| *tag)
        .collect::<BTreeSet<_>>()
        .len();
    if represented_countries > lifecycle_countries {
        return Err(format!(
            "projects PLCF represents {represented_countries} countries but reports only {lifecycle_countries} measured"
        ));
    }
    Ok(())
}

fn read_string(
    payload: &[u8],
    cursor: &mut usize,
    max: usize,
    label: &str,
) -> Result<Option<String>, String> {
    let len = u16_at(payload, *cursor)? as usize;
    *cursor = cursor
        .checked_add(2)
        .ok_or_else(|| format!("{label} cursor overflow"))?;
    if len == 0 {
        return Ok(None);
    }
    if len > max {
        return Err(format!("{label} length {len} exceeds {max}"));
    }
    let end = cursor
        .checked_add(len)
        .ok_or_else(|| format!("{label} length overflow"))?;
    let bytes = payload
        .get(*cursor..end)
        .ok_or_else(|| format!("{label} is truncated"))?;
    *cursor = end;
    String::from_utf8(bytes.to_vec())
        .map(Some)
        .map_err(|_| format!("{label} is not UTF-8"))
}

fn checked_rows_end(start: usize, count: usize, stride: usize) -> Result<usize, String> {
    count
        .checked_mul(stride)
        .and_then(|bytes| start.checked_add(bytes))
        .ok_or_else(|| "projects row extent overflow".to_string())
}

fn count_at(payload: &[u8], offset: usize) -> Result<usize, String> {
    let value = u32_at(payload, offset)? as usize;
    if value > MAX_ROWS {
        return Err(format!("row count at +{offset:#x} exceeds {MAX_ROWS}"));
    }
    Ok(value)
}

fn u16_at(bytes: &[u8], offset: usize) -> Result<u16, String> {
    let end = offset
        .checked_add(2)
        .ok_or_else(|| "u16 offset overflow".to_string())?;
    let slice = bytes
        .get(offset..end)
        .ok_or_else(|| format!("u16 at +{offset:#x} is truncated"))?;
    Ok(u16::from_le_bytes(
        slice.try_into().expect("two-byte checked slice"),
    ))
}

fn u32_at(bytes: &[u8], offset: usize) -> Result<u32, String> {
    let end = offset
        .checked_add(4)
        .ok_or_else(|| "u32 offset overflow".to_string())?;
    let slice = bytes
        .get(offset..end)
        .ok_or_else(|| format!("u32 at +{offset:#x} is truncated"))?;
    Ok(u32::from_le_bytes(
        slice.try_into().expect("four-byte checked slice"),
    ))
}

fn i32_at(bytes: &[u8], offset: usize) -> Result<i32, String> {
    Ok(i32::from_le_bytes(u32_at(bytes, offset)?.to_le_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone)]
    struct FixtureScientist {
        id: u32,
        deleted: bool,
        token: Option<&'static str>,
        display: Option<&'static str>,
    }

    fn projects_payload(delta: bool, scientists: &[FixtureScientist], observed: u32) -> Vec<u8> {
        let full = !delta;
        let facility_count = if full { 1 } else { 0 };
        let project_count = if full { 1 } else { 0 };
        let state_count = if full { 1 } else { 0 };
        let breakthrough_count = if full { 1 } else { 0 };
        let row_count =
            facility_count + project_count + scientists.len() + state_count + breakthrough_count;
        let status = if delta {
            compact_contract::COMPACT_STATUS_DELTA_DATA
        } else {
            0
        };
        let mut payload = vec![0u8; PROJECT_HEADER_PROGRESS_LEN];
        put_u32(&mut payload, 8, row_count as u32);
        put_u32(&mut payload, 12, row_count as u32);
        put_u32(&mut payload, 16, status);
        put_u32(&mut payload, 20, facility_count as u32);
        put_u32(&mut payload, 24, project_count as u32);
        put_u32(&mut payload, 28, scientists.len() as u32);
        put_u32(&mut payload, 32, facility_count as u32);
        put_u32(&mut payload, 36, project_count as u32);
        put_u32(&mut payload, 40, observed);
        put_u32(&mut payload, 48, PROJECT_SOURCE_SCAN);
        put_u32(&mut payload, 52, PROJECT_HEADER_EXT_PROGRESS_V2);
        put_u32(&mut payload, 76, 3); // neutral optional program roots are legitimate
        put_u32(&mut payload, 136, state_count as u32);
        put_u32(&mut payload, 140, breakthrough_count as u32);
        put_u32(&mut payload, 144, state_count as u32);
        put_u32(&mut payload, 148, breakthrough_count as u32);
        if full {
            let mut facility = [0u8; FACILITY_ROW_LEN];
            put_u32(&mut facility, 0, 1);
            put_u32(&mut facility, 4, 12);
            put_u32(&mut facility, 8, 1 << 2);
            payload.extend_from_slice(&facility);
            let mut project = [0u8; PROJECT_ROW_LEN];
            put_u32(&mut project, 0, 2);
            put_u32(&mut project, 4, 3);
            payload.extend_from_slice(&project);
        }
        for scientist in scientists {
            let mut row = [0u8; SCIENTIST_ROW_LEN];
            put_u32(&mut row, 0, scientist.id);
            if scientist.deleted {
                put_u32(&mut row, 4, FLAG_DELETED);
            } else {
                let mut flags = FLAG_SKILLS | FLAG_TRAIT_COUNT;
                if scientist.token.is_some() {
                    flags |= FLAG_NAME_TOKEN;
                }
                if scientist.display.is_some() {
                    flags |= FLAG_DISPLAY_NAME;
                }
                put_u32(&mut row, 4, flags);
                put_u32(&mut row, 8, 3);
                put_i32(&mut row, 12, 4);
                put_i32(&mut row, 16, 1);
                put_i32(&mut row, 20, 2);
                put_i32(&mut row, 24, 3);
            }
            payload.extend_from_slice(&row);
        }
        if full {
            let mut state = [0u8; STATE_ROW_LEN];
            put_u32(&mut state, 0, 2);
            put_u32(&mut state, 4, 3);
            put_u32(&mut state, 8, 0x18);
            put_u32(&mut state, 40, 100);
            put_u32(&mut state, 44, 1);
            payload.extend_from_slice(&state);
            let mut breakthrough = [0u8; BREAKTHROUGH_ROW_LEN];
            put_u32(&mut breakthrough, 0, 7);
            put_u32(&mut breakthrough, 4, 9);
            put_u32(&mut breakthrough, 8, 10);
            payload.extend_from_slice(&breakthrough);
            append_string(&mut payload, Some("special_project_facility"));
        }
        for scientist in scientists {
            append_string(&mut payload, scientist.token);
            append_string(&mut payload, scientist.display);
        }
        if full {
            payload.extend_from_slice(b"PLCF");
            payload.extend_from_slice(&1u16.to_le_bytes());
            payload.extend_from_slice(&20u16.to_le_bytes());
            payload.extend_from_slice(&1u32.to_le_bytes());
            payload.extend_from_slice(&1u32.to_le_bytes());
            payload.extend_from_slice(&0u32.to_le_bytes());
            payload.extend_from_slice(&7u32.to_le_bytes());
            payload.extend_from_slice(&2u32.to_le_bytes());
            payload.extend_from_slice(&3u32.to_le_bytes());
        }
        payload
    }

    fn perf_payload(data: &[u8]) -> Vec<u8> {
        let mut payload = vec![0u8; compact_contract::COMPACT_DOMAIN_PERF_BASE_LEN];
        let observed_rows = [32usize, 36, 40]
            .into_iter()
            .map(|offset| u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap()))
            .sum();
        let emitted_units = [20usize, 24, 28]
            .into_iter()
            .map(|offset| u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap()))
            .sum();
        put_u32(&mut payload, 16, observed_rows);
        put_u32(
            &mut payload,
            20,
            u32::from_le_bytes(data[12..16].try_into().unwrap()),
        );
        put_u32(&mut payload, 24, emitted_units);
        put_u32(
            &mut payload,
            28,
            u32::from_le_bytes(data[12..16].try_into().unwrap()),
        );
        put_u32(
            &mut payload,
            32,
            u32::from_le_bytes(data[16..20].try_into().unwrap()),
        );
        payload
    }

    fn record(sequence: u64, hour: i32, domain: u16, kind: u16, payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(32 + payload.len());
        out.extend_from_slice(b"H4CR");
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&32u16.to_le_bytes());
        out.extend_from_slice(&sequence.to_le_bytes());
        out.extend_from_slice(&hour.to_le_bytes());
        out.extend_from_slice(&domain.to_le_bytes());
        out.extend_from_slice(&kind.to_le_bytes());
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(payload);
        out
    }

    fn frame(sequence: &mut u64, hour: i32, data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend(record(
            *sequence,
            hour,
            41,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA,
            data,
        ));
        *sequence += 1;
        out.extend(record(
            *sequence,
            hour,
            41,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
            &perf_payload(data),
        ));
        *sequence += 1;
        let mut end = [0u8; 32];
        put_u32(&mut end, 16, 1);
        put_u32(&mut end, 20, 1);
        put_u32(
            &mut end,
            24,
            u32::from_le_bytes(data[16..20].try_into().unwrap()),
        );
        out.extend(record(
            *sequence,
            hour,
            compact_contract::COMPACT_DOMAIN_FRAME,
            compact_contract::COMPACT_RECORD_TYPE_FRAME_END,
            &end,
        ));
        *sequence += 1;
        out
    }

    fn stream_with_closed_frames(frame_count: usize) -> Vec<u8> {
        let full = projects_payload(
            false,
            &[FixtureScientist {
                id: 44,
                deleted: false,
                token: Some("POR_scientist"),
                display: Some("Ana"),
            }],
            1,
        );
        let unchanged_delta = projects_payload(true, &[], 1);
        let mut stream = Vec::new();
        let mut sequence = 1;
        for index in 0..frame_count {
            let data = if index == 0 { &full } else { &unchanged_delta };
            stream.extend(frame(&mut sequence, 100 + index as i32, data));
        }
        stream
    }

    #[test]
    fn decodes_production_v2_mixed_family_fixture() {
        let payload = projects_payload(
            false,
            &[FixtureScientist {
                id: 44,
                deleted: false,
                token: Some("POR_scientist"),
                display: Some("Ana Ferreira"),
            }],
            1,
        );
        let decoded = decode_production_projects_scientists(&payload).unwrap();
        assert!(!decoded.delta);
        assert_eq!(decoded.row_count, 5);
        assert_eq!(decoded.observed_scientists, 1);
        assert_eq!(decoded.perf_observed_rows, 3);
        assert_eq!(decoded.perf_emitted_units, 3);
        assert!(
            matches!(&decoded.mutations[0], ScientistMutation::Upsert(row) if row.char_id == 44 && row.name_token.as_deref() == Some("POR_scientist"))
        );
    }

    #[test]
    fn canonical_stream_folds_full_then_delta_tombstone() {
        let full = projects_payload(
            false,
            &[FixtureScientist {
                id: 44,
                deleted: false,
                token: Some("POR_scientist"),
                display: Some("Ana"),
            }],
            1,
        );
        let delta = projects_payload(
            true,
            &[
                FixtureScientist {
                    id: 44,
                    deleted: true,
                    token: None,
                    display: None,
                },
                FixtureScientist {
                    id: 45,
                    deleted: false,
                    token: Some("POR_scientist_2"),
                    display: Some("Bea"),
                },
            ],
            1,
        );
        let mut sequence = 1;
        let mut stream = frame(&mut sequence, 100, &full);
        stream.extend(frame(&mut sequence, 101, &delta));
        let report =
            certify_canonical_stream(&stream, &StreamCertificationPolicy::scientists_fixture());
        assert!(report.ok, "{:?}", report.problems);
        assert_eq!(report.closed_frames, 2);
        assert_eq!(report.scientist_deletes, 1);
        assert_eq!(report.final_scientist_rows, 1);
    }

    #[test]
    fn rejects_bridge_source_old_schema_and_hidden_root_failures() {
        let valid = projects_payload(
            false,
            &[FixtureScientist {
                id: 44,
                deleted: false,
                token: None,
                display: None,
            }],
            1,
        );
        let mut bridge = valid.clone();
        put_u32(&mut bridge, 44, 1);
        put_u32(&mut bridge, 48, 2);
        assert!(decode_production_projects_scientists(&bridge)
            .unwrap_err()
            .contains("DLL scan"));
        let mut old = valid.clone();
        put_u32(&mut old, 52, 3);
        assert!(decode_production_projects_scientists(&old)
            .unwrap_err()
            .contains("progress-v2"));
        let mut hidden = valid;
        put_u32(&mut hidden, 96, 1);
        assert!(decode_production_projects_scientists(&hidden)
            .unwrap_err()
            .contains("root-failure"));
    }

    #[test]
    fn rejects_delta_before_baseline_perf_splice_and_physical_tail() {
        let delta = projects_payload(
            true,
            &[FixtureScientist {
                id: 44,
                deleted: false,
                token: None,
                display: None,
            }],
            1,
        );
        let mut sequence = 1;
        let stream = frame(&mut sequence, 100, &delta);
        let report =
            certify_canonical_stream(&stream, &StreamCertificationPolicy::scientists_fixture());
        assert!(report
            .problems
            .iter()
            .any(|problem| problem.contains("delta precedes")));

        let full = projects_payload(
            false,
            &[FixtureScientist {
                id: 44,
                deleted: false,
                token: None,
                display: None,
            }],
            1,
        );
        let mut sequence = 1;
        let mut mismatch = frame(&mut sequence, 100, &full);
        let perf_row_count_offset = 32 + full.len() + 32 + 16;
        mismatch[perf_row_count_offset..perf_row_count_offset + 4]
            .copy_from_slice(&99u32.to_le_bytes());
        let report =
            certify_canonical_stream(&mismatch, &StreamCertificationPolicy::scientists_fixture());
        assert!(report
            .problems
            .iter()
            .any(|problem| problem.contains("DATA/PERF")));

        let mut sequence = 1;
        let mut status_splice = frame(&mut sequence, 100, &full);
        let frame_status_offset = status_splice.len() - 8;
        status_splice[frame_status_offset..frame_status_offset + 4]
            .copy_from_slice(&compact_contract::COMPACT_STATUS_DELTA_DATA.to_le_bytes());
        let report = certify_canonical_stream(
            &status_splice,
            &StreamCertificationPolicy::scientists_fixture(),
        );
        assert!(report
            .problems
            .iter()
            .any(|problem| problem.contains("normalized PERF status OR")));

        let mut sequence = 1;
        let mut tailed = frame(&mut sequence, 100, &full);
        tailed.push(0xAA);
        let report =
            certify_canonical_stream(&tailed, &StreamCertificationPolicy::scientists_fixture());
        assert!(!report.ok);
        assert!(report
            .problems
            .iter()
            .any(|problem| problem.contains("offset") || problem.contains("physical EOF")));
    }

    #[test]
    fn rejects_string_flag_forgery_full_tombstones_and_fold_count_lies() {
        let mut forged = projects_payload(
            false,
            &[FixtureScientist {
                id: 44,
                deleted: false,
                token: Some("POR_scientist"),
                display: None,
            }],
            1,
        );
        let scientist = PROJECT_HEADER_PROGRESS_LEN + FACILITY_ROW_LEN + PROJECT_ROW_LEN;
        let flags = u32::from_le_bytes(forged[scientist + 4..scientist + 8].try_into().unwrap());
        put_u32(&mut forged, scientist + 4, flags & !FLAG_NAME_TOKEN);
        assert!(decode_production_projects_scientists(&forged)
            .unwrap_err()
            .contains("string authority"));

        let tombstone = projects_payload(
            false,
            &[FixtureScientist {
                id: 44,
                deleted: true,
                token: None,
                display: None,
            }],
            1,
        );
        assert!(decode_production_projects_scientists(&tombstone)
            .unwrap_err()
            .contains("tombstone"));

        let mut aborted = projects_payload(
            false,
            &[FixtureScientist {
                id: 44,
                deleted: false,
                token: None,
                display: None,
            }],
            1,
        );
        let trailer_start = aborted.len() - 32;
        put_u32(&mut aborted, trailer_start + 16, 1);
        assert!(decode_production_projects_scientists(&aborted)
            .unwrap_err()
            .contains("aborted lifecycle countries"));

        let full = projects_payload(
            false,
            &[FixtureScientist {
                id: 44,
                deleted: false,
                token: None,
                display: None,
            }],
            1,
        );
        let delta = projects_payload(true, &[], 0);
        let mut sequence = 1;
        let mut stream = frame(&mut sequence, 100, &full);
        stream.extend(frame(&mut sequence, 101, &delta));
        let report =
            certify_canonical_stream(&stream, &StreamCertificationPolicy::scientists_fixture());
        assert!(report
            .problems
            .iter()
            .any(|problem| problem.contains("folded scientist roster")));
    }

    #[test]
    fn production_policy_is_descriptor_derived_and_d41_is_data_owned() {
        let policy = StreamCertificationPolicy::production(48).unwrap();
        assert!(policy.expected_perf_roster.contains(&41));
        assert!(policy.expected_data_roster.contains(&41));
        assert!(policy.expected_perf_roster.len() >= policy.expected_data_roster.len());
        assert!(policy
            .expected_data_roster
            .iter()
            .all(|id| compact_contract::requires_data_record(*id)));
        assert_eq!(policy.requested_game_hour_advancements, Some(48));
        assert!(StreamCertificationPolicy::production(0).is_err());
    }

    #[test]
    fn duration_exact_n_plus_one_frames_and_span_pass() {
        let stream = stream_with_closed_frames(3);
        let policy =
            StreamCertificationPolicy::scientists_fixture_with_requested_game_hours(2).unwrap();
        let report = certify_canonical_stream(&stream, &policy);
        assert!(report.ok, "{:?}", report.problems);
        assert_eq!(report.closed_frames, 3);
        assert_eq!(report.actual_game_hour_advancements, Some(2));
    }

    #[test]
    fn duration_one_frame_cannot_certify_positive_request() {
        let stream = stream_with_closed_frames(1);
        let policy =
            StreamCertificationPolicy::scientists_fixture_with_requested_game_hours(1).unwrap();
        let report = certify_canonical_stream(&stream, &policy);
        assert!(!report.ok);
        assert!(report
            .problems
            .iter()
            .any(|problem| problem.contains("closed frame count 1")));
    }

    #[test]
    fn duration_under_capture_is_rejected() {
        let stream = stream_with_closed_frames(2);
        let policy =
            StreamCertificationPolicy::scientists_fixture_with_requested_game_hours(2).unwrap();
        let report = certify_canonical_stream(&stream, &policy);
        assert!(!report.ok);
        assert!(report
            .problems
            .iter()
            .any(|problem| problem.contains("closed frame count 2")));
    }

    #[test]
    fn duration_over_capture_is_rejected() {
        let stream = stream_with_closed_frames(3);
        let policy =
            StreamCertificationPolicy::scientists_fixture_with_requested_game_hours(1).unwrap();
        let report = certify_canonical_stream(&stream, &policy);
        assert!(!report.ok);
        assert!(report
            .problems
            .iter()
            .any(|problem| problem.contains("closed frame count 3")));
    }

    #[test]
    fn arbitrary_bytes_never_panic() {
        let mut state = 0x9E37_79B9_7F4A_7C15u64;
        for _ in 0..4096 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let len = (state as usize) % 640;
            let mut bytes = vec![0u8; len];
            for byte in &mut bytes {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                *byte = state as u8;
            }
            assert!(std::panic::catch_unwind(|| certify_canonical_stream(
                &bytes,
                &StreamCertificationPolicy::scientists_fixture()
            ))
            .is_ok());
        }
    }

    fn append_string(out: &mut Vec<u8>, value: Option<&str>) {
        match value {
            Some(value) => {
                out.extend_from_slice(&(value.len() as u16).to_le_bytes());
                out.extend_from_slice(value.as_bytes());
            }
            None => out.extend_from_slice(&0u16.to_le_bytes()),
        }
    }

    fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn put_i32(bytes: &mut [u8], offset: usize, value: i32) {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
}
