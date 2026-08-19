use crate::diagnostic_channel::{
    decode_locator, decode_units, encode_locator, encode_units, Authority, DiagUnit, LocatorLevel,
    OffFrameOutcome, Predicate, TruncationAccount, TruncationKind, OFF_FRAME_LEN,
};

pub const FRAME_REFUSAL_MAGIC: &[u8; 4] = b"H4FR";
pub const FRAME_REFUSAL_VERSION: u16 = 2;
pub const FRAME_REFUSAL_HEADER_LEN: usize = 16;
pub const FRAME_REFUSAL_ENTRY_FIXED_LEN: usize = 52;
pub const FRAME_REFUSAL_MAX_LOCATOR_LEN: usize = 1024;
pub const FRAME_REFUSAL_MAX_UNITS_LEN: usize = 4096;
pub const FRAME_REFUSAL_MAX_PREDICATE_NAME_LEN: usize =
    crate::diagnostic_channel::MAX_PREDICATE_NAME_LEN;
const ENTRY_FLAG_OFF_FRAME: u16 = 1 << 0;
pub const FRAME_REFUSAL_MAX_REASON_LEN: usize = 512;

pub const FRAME_REFUSAL_REASON_SUFFIX_SEPARATOR: &str = " | reason=";
pub const FRAME_REFUSAL_MAX_OFFENDERS: usize = 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameRefusalOffenderV2 {
    pub domain_id: u16,
    pub status_flags: u32,
    pub non_tolerated_flags: u32,
    pub predicate: Predicate,
    pub authority: Authority,
    pub locator: Vec<LocatorLevel>,
    pub units: Vec<DiagUnit>,
    pub truncation: TruncationAccount,
    pub off_frame: Option<OffFrameOutcome>,
    pub reason: String,
    pub reason_units_named: u32,
}

impl FrameRefusalOffenderV2 {
    pub fn render(&self) -> String {
        let mut parts = vec![
            format!(
                "predicate={}#{}",
                self.predicate.name,
                self.predicate.code.get()
            ),
            self.authority.render(),
        ];
        if !self.locator.is_empty() {
            parts.push(format!(
                "at[{}]",
                self.locator
                    .iter()
                    .map(LocatorLevel::render)
                    .collect::<Vec<_>>()
                    .join(" / ")
            ));
        }
        if let Some(off_frame) = &self.off_frame {
            parts.push(off_frame.render());
        }
        if !self.units.is_empty() {
            parts.push(
                self.units
                    .iter()
                    .map(DiagUnit::render)
                    .collect::<Vec<_>>()
                    .join(","),
            );
        }
        parts.push(self.truncation.render());
        if !self.reason.is_empty() {
            parts.push(format!("reason={}", self.reason));
        }
        parts.push(format!(
            "reason_names={}/{}",
            self.reason_units_named,
            self.units.len()
        ));
        parts.join(" ")
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameRefusalFinding {
    pub predicate: Predicate,
    pub authority: Authority,
    pub locator: Vec<LocatorLevel>,
    pub reason: String,
}

pub const FRAME_REFUSAL_FINDING_FIXED_LEN: usize = 24;
pub const FRAME_REFUSAL_MAX_FINDINGS: usize = 256;

impl FrameRefusalFinding {
    pub fn render(&self) -> String {
        let mut parts = vec![
            format!(
                "predicate={}#{}",
                self.predicate.name,
                self.predicate.code.get()
            ),
            self.authority.render(),
        ];
        if !self.locator.is_empty() {
            parts.push(format!(
                "at[{}]",
                self.locator
                    .iter()
                    .map(LocatorLevel::render)
                    .collect::<Vec<_>>()
                    .join(" / ")
            ));
        }
        parts.push(format!("reason={}", self.reason));
        parts.join(" ")
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameRefusalRecordV2 {
    pub refused_hour: i32,
    pub offenders: Vec<FrameRefusalOffenderV2>,
    pub frame_findings: Vec<FrameRefusalFinding>,
}

impl FrameRefusalRecordV2 {
    pub fn encode(&self) -> Result<Vec<u8>, String> {
        if self.offenders.is_empty() && self.frame_findings.is_empty() {
            return Err(
                "frame refusal names neither an offending domain nor a frame-level finding"
                    .to_string(),
            );
        }
        if self.offenders.len() > FRAME_REFUSAL_MAX_OFFENDERS {
            return Err(format!(
                "frame refusal names {} offenders, above the {FRAME_REFUSAL_MAX_OFFENDERS} bound",
                self.offenders.len()
            ));
        }
        if self.frame_findings.len() > FRAME_REFUSAL_MAX_FINDINGS {
            return Err(format!(
                "frame refusal carries {} frame findings, above the \
                 {FRAME_REFUSAL_MAX_FINDINGS} bound",
                self.frame_findings.len()
            ));
        }
        let mut out = Vec::with_capacity(FRAME_REFUSAL_HEADER_LEN + self.offenders.len() * 192);
        out.extend_from_slice(FRAME_REFUSAL_MAGIC);
        out.extend_from_slice(&FRAME_REFUSAL_VERSION.to_le_bytes());
        out.extend_from_slice(&(FRAME_REFUSAL_HEADER_LEN as u16).to_le_bytes());
        out.extend_from_slice(&self.refused_hour.to_le_bytes());
        out.extend_from_slice(&(self.offenders.len() as u16).to_le_bytes());
        out.extend_from_slice(&(self.frame_findings.len() as u16).to_le_bytes());
        for offender in &self.offenders {
            if offender.non_tolerated_flags == 0 {
                return Err(format!(
                    "frame refusal lists domain {} with no non-tolerated bits",
                    offender.domain_id
                ));
            }
            if offender.authority == Authority::Undeclared
                || offender.truncation.kind == TruncationKind::Undeclared
            {
                return Err(format!(
                    "frame refusal for domain {} carries an undeclared authority or truncation \
                     account, which has no wire encoding",
                    offender.domain_id
                ));
            }
            if offender.predicate.name.is_empty() {
                return Err(format!(
                    "frame refusal gives domain {} a predicate with no name",
                    offender.domain_id
                ));
            }
            if offender.predicate.name.len() > FRAME_REFUSAL_MAX_PREDICATE_NAME_LEN {
                return Err(format!(
                    "frame refusal predicate name for domain {} is {} bytes, above the \
                     {FRAME_REFUSAL_MAX_PREDICATE_NAME_LEN} bound",
                    offender.domain_id,
                    offender.predicate.name.len()
                ));
            }
            let reason = offender.reason.as_bytes();
            if reason.is_empty() {
                return Err(format!(
                    "frame refusal gives domain {} no reason",
                    offender.domain_id
                ));
            }
            if reason.len() > FRAME_REFUSAL_MAX_REASON_LEN {
                return Err(format!(
                    "frame refusal reason for domain {} is {} bytes, above the \
                     {FRAME_REFUSAL_MAX_REASON_LEN} bound",
                    offender.domain_id,
                    reason.len()
                ));
            }
            if offender.truncation.units_emitted > offender.truncation.units_offered {
                return Err(format!(
                    "frame refusal for domain {} claims {} of {} units emitted",
                    offender.domain_id,
                    offender.truncation.units_emitted,
                    offender.truncation.units_offered
                ));
            }
            if offender.units.len() != offender.truncation.units_emitted as usize {
                return Err(format!(
                    "frame refusal for domain {} carries {} units but accounts {}",
                    offender.domain_id,
                    offender.units.len(),
                    offender.truncation.units_emitted
                ));
            }
            check_reason_names_its_units(
                &offender.reason,
                &offender.units,
                offender.reason_units_named,
                &format!("frame refusal for domain {}", offender.domain_id),
            )?;
            if offender.truncation.kind == TruncationKind::None
                && (offender.truncation.units_emitted != offender.truncation.units_offered
                    || offender.truncation.bytes_dropped != 0)
            {
                return Err(format!(
                    "frame refusal for domain {} claims no truncation while dropping {} unit(s) \
                     and {} byte(s)",
                    offender.domain_id,
                    offender.truncation.units_dropped(),
                    offender.truncation.bytes_dropped
                ));
            }
            let locator = encode_locator(&offender.locator)?;
            if locator.len() > FRAME_REFUSAL_MAX_LOCATOR_LEN {
                return Err(format!(
                    "frame refusal locator for domain {} is {} bytes, above the \
                     {FRAME_REFUSAL_MAX_LOCATOR_LEN} bound",
                    offender.domain_id,
                    locator.len()
                ));
            }
            let units = encode_units(&offender.units)?;
            if units.len() > FRAME_REFUSAL_MAX_UNITS_LEN {
                return Err(format!(
                    "frame refusal units for domain {} are {} bytes, above the \
                     {FRAME_REFUSAL_MAX_UNITS_LEN} bound",
                    offender.domain_id,
                    units.len()
                ));
            }
            let entry_flags = if offender.off_frame.is_some() {
                ENTRY_FLAG_OFF_FRAME
            } else {
                0
            };
            out.extend_from_slice(&offender.domain_id.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(&offender.status_flags.to_le_bytes());
            out.extend_from_slice(&offender.non_tolerated_flags.to_le_bytes());
            out.extend_from_slice(&offender.predicate.code.get().to_le_bytes());
            out.push(offender.truncation.kind.code());
            let reason_units_named = u8::try_from(offender.reason_units_named).map_err(|_| {
                format!(
                    "frame refusal for domain {} says its reason names {} units, above the \
                     {} the field holds",
                    offender.domain_id,
                    offender.reason_units_named,
                    u8::MAX
                )
            })?;
            out.push(reason_units_named);
            out.extend_from_slice(&(offender.predicate.name.len() as u16).to_le_bytes());
            out.extend_from_slice(&offender.truncation.units_offered.to_le_bytes());
            out.extend_from_slice(&offender.truncation.units_emitted.to_le_bytes());
            out.extend_from_slice(&offender.truncation.bytes_dropped.to_le_bytes());
            out.extend_from_slice(&offender.authority.encode());
            out.extend_from_slice(&entry_flags.to_le_bytes());
            out.extend_from_slice(&(reason.len() as u16).to_le_bytes());
            out.extend_from_slice(&(locator.len() as u16).to_le_bytes());
            out.extend_from_slice(&(units.len() as u16).to_le_bytes());
            out.extend_from_slice(offender.predicate.name.as_bytes());
            out.extend_from_slice(&locator);
            out.extend_from_slice(&units);
            if let Some(off_frame) = &offender.off_frame {
                out.extend_from_slice(&off_frame.encode());
            }
            out.extend_from_slice(reason);
        }
        for finding in &self.frame_findings {
            if finding.predicate.name.is_empty() {
                return Err("frame refusal finding has a predicate with no name".to_string());
            }
            if finding.predicate.name.len() > FRAME_REFUSAL_MAX_PREDICATE_NAME_LEN {
                return Err(format!(
                    "frame refusal finding predicate name is {} bytes, above the \
                     {FRAME_REFUSAL_MAX_PREDICATE_NAME_LEN} bound",
                    finding.predicate.name.len()
                ));
            }
            let reason = finding.reason.as_bytes();
            if reason.is_empty() {
                return Err("frame refusal finding gives no reason".to_string());
            }
            if reason.len() > FRAME_REFUSAL_MAX_REASON_LEN {
                return Err(format!(
                    "frame refusal finding reason is {} bytes, above the \
                     {FRAME_REFUSAL_MAX_REASON_LEN} bound",
                    reason.len()
                ));
            }
            let locator = encode_locator(&finding.locator)?;
            if locator.len() > FRAME_REFUSAL_MAX_LOCATOR_LEN {
                return Err(format!(
                    "frame refusal finding locator is {} bytes, above the \
                     {FRAME_REFUSAL_MAX_LOCATOR_LEN} bound",
                    locator.len()
                ));
            }
            out.extend_from_slice(&finding.predicate.code.get().to_le_bytes());
            out.extend_from_slice(&finding.authority.encode());
            out.extend_from_slice(&(finding.predicate.name.len() as u16).to_le_bytes());
            out.extend_from_slice(&(locator.len() as u16).to_le_bytes());
            out.extend_from_slice(&(reason.len() as u16).to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(finding.predicate.name.as_bytes());
            out.extend_from_slice(&locator);
            out.extend_from_slice(reason);
        }
        Ok(out)
    }

    pub fn decode(payload: &[u8]) -> Result<Self, String> {
        validate_frame_refusal_payload(payload)?;
        let refused_hour = i32::from_le_bytes([payload[8], payload[9], payload[10], payload[11]]);
        let offender_count = u16::from_le_bytes([payload[12], payload[13]]) as usize;
        let finding_count = u16::from_le_bytes([payload[14], payload[15]]) as usize;
        let mut offenders = Vec::with_capacity(offender_count);
        let mut offset = FRAME_REFUSAL_HEADER_LEN;
        for _ in 0..offender_count {
            let (offender, next) = decode_v2_entry(payload, offset)?;
            offenders.push(offender);
            offset = next;
        }
        let mut frame_findings = Vec::with_capacity(finding_count);
        for _ in 0..finding_count {
            let (finding, next) = decode_v2_finding(payload, offset)?;
            frame_findings.push(finding);
            offset = next;
        }
        Ok(Self {
            refused_hour,
            offenders,
            frame_findings,
        })
    }
}

fn decode_v2_finding(
    payload: &[u8],
    offset: usize,
) -> Result<(FrameRefusalFinding, usize), String> {
    let predicate_code = read_u32(payload, offset);
    let authority = Authority::decode(&payload[offset + 4..offset + 16])?;
    let predicate_name_len = read_u16(payload, offset + 16) as usize;
    let locator_len = read_u16(payload, offset + 18) as usize;
    let reason_len = read_u16(payload, offset + 20) as usize;
    let mut cursor = offset + FRAME_REFUSAL_FINDING_FIXED_LEN;
    let predicate_name = core::str::from_utf8(&payload[cursor..cursor + predicate_name_len])
        .map_err(|error| format!("frame refusal finding predicate name is not UTF-8: {error}"))?
        .to_string();
    cursor += predicate_name_len;
    let (locator, locator_used) = decode_locator(&payload[cursor..cursor + locator_len])?;
    if locator_used != locator_len {
        return Err(format!(
            "frame refusal finding locator declares {locator_len} bytes but decodes {locator_used}"
        ));
    }
    cursor += locator_len;
    let reason = core::str::from_utf8(&payload[cursor..cursor + reason_len])
        .map_err(|error| format!("frame refusal finding reason is not UTF-8: {error}"))?
        .to_string();
    cursor += reason_len;
    Ok((
        FrameRefusalFinding {
            predicate: Predicate::new(predicate_code, predicate_name)?,
            authority,
            locator,
            reason,
        },
        cursor,
    ))
}

fn read_u16(payload: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([payload[at], payload[at + 1]])
}

fn read_u32(payload: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([
        payload[at],
        payload[at + 1],
        payload[at + 2],
        payload[at + 3],
    ])
}

fn contains_as_a_field(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    let mut from = 0usize;
    while let Some(at) = haystack[from..].find(needle) {
        let start = from + at;
        let end = start + needle.len();
        let before_ok = start == 0
            || haystack[..start]
                .chars()
                .next_back()
                .is_some_and(|c| c == ',' || c == ' ');
        let after_ok = end == haystack.len()
            || haystack[end..].starts_with(',')
            || haystack[end..].starts_with(FRAME_REFUSAL_REASON_SUFFIX_SEPARATOR);
        if before_ok && after_ok {
            return true;
        }
        from = start + haystack[start..].chars().next().map_or(1, char::len_utf8);
        if from >= haystack.len() {
            break;
        }
    }
    false
}

fn check_reason_names_its_units(
    reason: &str,
    units: &[DiagUnit],
    named: u32,
    what: &str,
) -> Result<(), String> {
    if named as usize > units.len() {
        return Err(format!(
            "{what} says its reason names {named} units but carries {}",
            units.len()
        ));
    }
    if units.is_empty() {
        return Ok(());
    }
    if named == 0 {
        return Err(format!(
            "{what} carries {} unit(s) and names none of them in its reason",
            units.len()
        ));
    }
    for (position, unit) in units.iter().take(named as usize).enumerate() {
        let rendered = unit.render();
        if !contains_as_a_field(reason, &rendered) {
            return Err(format!(
                "{what} claims its reason names {named} unit(s) but the reason does not \
                 contain unit {position} {rendered:?}"
            ));
        }
    }
    Ok(())
}

fn decode_v2_entry(
    payload: &[u8],
    offset: usize,
) -> Result<(FrameRefusalOffenderV2, usize), String> {
    let domain_id = read_u16(payload, offset);
    let status_flags = read_u32(payload, offset + 4);
    let non_tolerated_flags = read_u32(payload, offset + 8);
    let predicate_code = read_u32(payload, offset + 12);
    let truncation_kind = TruncationKind::from_code(payload[offset + 16]).ok_or_else(|| {
        format!(
            "frame refusal truncation kind {} is not defined",
            payload[offset + 16]
        )
    })?;
    let predicate_name_len = read_u16(payload, offset + 18) as usize;
    let units_offered = read_u32(payload, offset + 20);
    let units_emitted = read_u32(payload, offset + 24);
    let bytes_dropped = read_u32(payload, offset + 28);
    let authority = Authority::decode(&payload[offset + 32..offset + 44])?;
    let entry_flags = read_u16(payload, offset + 44);
    let reason_len = read_u16(payload, offset + 46) as usize;
    let locator_len = read_u16(payload, offset + 48) as usize;
    let units_len = read_u16(payload, offset + 50) as usize;

    let mut cursor = offset + FRAME_REFUSAL_ENTRY_FIXED_LEN;
    let predicate_name = core::str::from_utf8(&payload[cursor..cursor + predicate_name_len])
        .map_err(|error| format!("frame refusal predicate name is not UTF-8: {error}"))?
        .to_string();
    cursor += predicate_name_len;
    let (locator, locator_used) = decode_locator(&payload[cursor..cursor + locator_len])?;
    if locator_used != locator_len {
        return Err(format!(
            "frame refusal locator for domain {domain_id} declares {locator_len} bytes but \
             decodes {locator_used}"
        ));
    }
    cursor += locator_len;
    let (units, units_used) = decode_units(&payload[cursor..cursor + units_len])?;
    if units_used != units_len {
        return Err(format!(
            "frame refusal units for domain {domain_id} declare {units_len} bytes but decode \
             {units_used}"
        ));
    }
    cursor += units_len;
    let off_frame = if entry_flags & ENTRY_FLAG_OFF_FRAME != 0 {
        let value = OffFrameOutcome::decode(&payload[cursor..cursor + OFF_FRAME_LEN])?;
        cursor += OFF_FRAME_LEN;
        Some(value)
    } else {
        None
    };
    let reason = core::str::from_utf8(&payload[cursor..cursor + reason_len])
        .map_err(|error| format!("frame refusal reason is not UTF-8: {error}"))?
        .to_string();
    cursor += reason_len;

    let predicate = Predicate::new(predicate_code, predicate_name)?;
    Ok((
        FrameRefusalOffenderV2 {
            domain_id,
            status_flags,
            non_tolerated_flags,
            predicate,
            authority,
            locator,
            units,
            truncation: TruncationAccount {
                kind: truncation_kind,
                units_offered,
                units_emitted,
                bytes_dropped,
            },
            off_frame,
            reason,
            reason_units_named: u32::from(payload[offset + 17]),
        },
        cursor,
    ))
}

pub fn validate_frame_refusal_payload(payload: &[u8]) -> Result<(), String> {
    if payload.len() < FRAME_REFUSAL_HEADER_LEN {
        return Err(format!(
            "frame refusal payload {} byte(s), below the {FRAME_REFUSAL_HEADER_LEN}-byte header",
            payload.len()
        ));
    }
    if &payload[0..4] != FRAME_REFUSAL_MAGIC {
        return Err("frame refusal magic is not H4FR".to_string());
    }
    let version = read_u16(payload, 4);
    if version != FRAME_REFUSAL_VERSION {
        return Err(unknown_frame_refusal_version(version));
    }
    let header_len = read_u16(payload, 6) as usize;
    if header_len != FRAME_REFUSAL_HEADER_LEN {
        return Err(format!(
            "frame refusal header length {header_len} is not {FRAME_REFUSAL_HEADER_LEN}"
        ));
    }
    let offender_count = read_u16(payload, 12) as usize;
    let finding_count = read_u16(payload, 14) as usize;
    if offender_count == 0 && finding_count == 0 {
        return Err(
            "frame refusal names neither an offending domain nor a frame-level finding".to_string(),
        );
    }
    if offender_count > FRAME_REFUSAL_MAX_OFFENDERS {
        return Err(format!(
            "frame refusal names {offender_count} offenders, above the \
             {FRAME_REFUSAL_MAX_OFFENDERS} bound"
        ));
    }
    if finding_count > FRAME_REFUSAL_MAX_FINDINGS {
        return Err(format!(
            "frame refusal carries {finding_count} frame findings, above the \
             {FRAME_REFUSAL_MAX_FINDINGS} bound"
        ));
    }
    let mut offset = FRAME_REFUSAL_HEADER_LEN;
    for index in 0..offender_count {
        if offset
            .checked_add(FRAME_REFUSAL_ENTRY_FIXED_LEN)
            .is_none_or(|end| end > payload.len())
        {
            return Err(format!(
                "frame refusal entry {index} is truncated at offset {offset}"
            ));
        }
        let domain_id = read_u16(payload, offset);
        if read_u16(payload, offset + 2) != 0 {
            return Err(format!(
                "frame refusal entry {index} (domain {domain_id}) has a nonzero reserved field"
            ));
        }
        if read_u32(payload, offset + 8) == 0 {
            return Err(format!(
                "frame refusal entry {index} (domain {domain_id}) has no non-tolerated bits"
            ));
        }
        if read_u32(payload, offset + 12) == 0 {
            return Err(format!(
                "frame refusal entry {index} (domain {domain_id}) declares no predicate"
            ));
        }
        if TruncationKind::from_code(payload[offset + 16]).is_none() {
            return Err(format!(
                "frame refusal entry {index} (domain {domain_id}) has truncation kind {}",
                payload[offset + 16]
            ));
        }
        let predicate_name_len = read_u16(payload, offset + 18) as usize;
        if predicate_name_len == 0 {
            return Err(format!(
                "frame refusal entry {index} (domain {domain_id}) has an unnamed predicate"
            ));
        }
        if predicate_name_len > FRAME_REFUSAL_MAX_PREDICATE_NAME_LEN {
            return Err(format!(
                "frame refusal entry {index} predicate name is {predicate_name_len} bytes, above \
                 the {FRAME_REFUSAL_MAX_PREDICATE_NAME_LEN} bound"
            ));
        }
        let units_offered = read_u32(payload, offset + 20);
        let units_emitted = read_u32(payload, offset + 24);
        if units_emitted > units_offered {
            return Err(format!(
                "frame refusal entry {index} (domain {domain_id}) emits {units_emitted} of \
                 {units_offered} units"
            ));
        }
        let bytes_dropped = read_u32(payload, offset + 28);
        if payload[offset + 16] == TruncationKind::None.code()
            && (units_emitted != units_offered || bytes_dropped != 0)
        {
            return Err(format!(
                "frame refusal entry {index} (domain {domain_id}) claims no truncation while \
                 dropping {} unit(s) and {bytes_dropped} byte(s)",
                units_offered.saturating_sub(units_emitted)
            ));
        }
        let reason_units_named = u32::from(payload[offset + 17]);
        Authority::decode(&payload[offset + 32..offset + 44])?;
        let entry_flags = read_u16(payload, offset + 44);
        if entry_flags & !ENTRY_FLAG_OFF_FRAME != 0 {
            return Err(format!(
                "frame refusal entry {index} has unknown entry flags {entry_flags:#06x}"
            ));
        }
        let reason_len = read_u16(payload, offset + 46) as usize;
        let locator_len = read_u16(payload, offset + 48) as usize;
        let units_len = read_u16(payload, offset + 50) as usize;
        if reason_len == 0 {
            return Err(format!("frame refusal entry {index} gives no reason"));
        }
        if reason_len > FRAME_REFUSAL_MAX_REASON_LEN {
            return Err(format!(
                "frame refusal entry {index} reason is {reason_len} bytes, above the \
                 {FRAME_REFUSAL_MAX_REASON_LEN} bound"
            ));
        }
        if locator_len > FRAME_REFUSAL_MAX_LOCATOR_LEN {
            return Err(format!(
                "frame refusal entry {index} locator is {locator_len} bytes, above the \
                 {FRAME_REFUSAL_MAX_LOCATOR_LEN} bound"
            ));
        }
        if units_len > FRAME_REFUSAL_MAX_UNITS_LEN {
            return Err(format!(
                "frame refusal entry {index} units are {units_len} bytes, above the \
                 {FRAME_REFUSAL_MAX_UNITS_LEN} bound"
            ));
        }
        let off_frame_len = if entry_flags & ENTRY_FLAG_OFF_FRAME != 0 {
            OFF_FRAME_LEN
        } else {
            0
        };
        let Some(end) = offset
            .checked_add(FRAME_REFUSAL_ENTRY_FIXED_LEN)
            .and_then(|value| value.checked_add(predicate_name_len))
            .and_then(|value| value.checked_add(locator_len))
            .and_then(|value| value.checked_add(units_len))
            .and_then(|value| value.checked_add(off_frame_len))
            .and_then(|value| value.checked_add(reason_len))
        else {
            return Err(format!("frame refusal entry {index} length overflows"));
        };
        if end > payload.len() {
            return Err(format!("frame refusal entry {index} runs past the payload"));
        }
        let mut cursor = offset + FRAME_REFUSAL_ENTRY_FIXED_LEN;
        if core::str::from_utf8(&payload[cursor..cursor + predicate_name_len]).is_err() {
            return Err(format!(
                "frame refusal entry {index} predicate name is not UTF-8"
            ));
        }
        cursor += predicate_name_len;
        let (_, locator_used) = decode_locator(&payload[cursor..cursor + locator_len])?;
        if locator_used != locator_len {
            return Err(format!(
                "frame refusal entry {index} locator declares {locator_len} bytes but decodes \
                 {locator_used}"
            ));
        }
        cursor += locator_len;
        let (units, units_used) = decode_units(&payload[cursor..cursor + units_len])?;
        if units_used != units_len {
            return Err(format!(
                "frame refusal entry {index} units declare {units_len} bytes but decode \
                 {units_used}"
            ));
        }
        if units.len() != units_emitted as usize {
            return Err(format!(
                "frame refusal entry {index} carries {} units but accounts {units_emitted}",
                units.len()
            ));
        }
        cursor += units_len;
        if off_frame_len != 0 {
            OffFrameOutcome::decode(&payload[cursor..cursor + OFF_FRAME_LEN])?;
            cursor += OFF_FRAME_LEN;
        }
        let Ok(reason) = core::str::from_utf8(&payload[cursor..cursor + reason_len]) else {
            return Err(format!("frame refusal entry {index} reason is not UTF-8"));
        };
        check_reason_names_its_units(
            reason,
            &units,
            reason_units_named,
            &format!("frame refusal entry {index} (domain {domain_id})"),
        )?;
        offset = end;
    }
    for index in 0..finding_count {
        if offset
            .checked_add(FRAME_REFUSAL_FINDING_FIXED_LEN)
            .is_none_or(|end| end > payload.len())
        {
            return Err(format!(
                "frame refusal finding {index} is truncated at offset {offset}"
            ));
        }
        if read_u32(payload, offset) == 0 {
            return Err(format!(
                "frame refusal finding {index} declares no predicate"
            ));
        }
        Authority::decode(&payload[offset + 4..offset + 16])?;
        let predicate_name_len = read_u16(payload, offset + 16) as usize;
        let locator_len = read_u16(payload, offset + 18) as usize;
        let reason_len = read_u16(payload, offset + 20) as usize;
        if read_u16(payload, offset + 22) != 0 {
            return Err(format!(
                "frame refusal finding {index} has a nonzero reserved field"
            ));
        }
        if predicate_name_len == 0 {
            return Err(format!(
                "frame refusal finding {index} has an unnamed predicate"
            ));
        }
        if predicate_name_len > FRAME_REFUSAL_MAX_PREDICATE_NAME_LEN {
            return Err(format!(
                "frame refusal finding {index} predicate name is {predicate_name_len} bytes, \
                 above the {FRAME_REFUSAL_MAX_PREDICATE_NAME_LEN} bound"
            ));
        }
        if reason_len == 0 {
            return Err(format!("frame refusal finding {index} gives no reason"));
        }
        if reason_len > FRAME_REFUSAL_MAX_REASON_LEN {
            return Err(format!(
                "frame refusal finding {index} reason is {reason_len} bytes, above the \
                 {FRAME_REFUSAL_MAX_REASON_LEN} bound"
            ));
        }
        if locator_len > FRAME_REFUSAL_MAX_LOCATOR_LEN {
            return Err(format!(
                "frame refusal finding {index} locator is {locator_len} bytes, above the \
                 {FRAME_REFUSAL_MAX_LOCATOR_LEN} bound"
            ));
        }
        let Some(end) = offset
            .checked_add(FRAME_REFUSAL_FINDING_FIXED_LEN)
            .and_then(|value| value.checked_add(predicate_name_len))
            .and_then(|value| value.checked_add(locator_len))
            .and_then(|value| value.checked_add(reason_len))
        else {
            return Err(format!("frame refusal finding {index} length overflows"));
        };
        if end > payload.len() {
            return Err(format!(
                "frame refusal finding {index} runs past the payload"
            ));
        }
        let mut cursor = offset + FRAME_REFUSAL_FINDING_FIXED_LEN;
        if core::str::from_utf8(&payload[cursor..cursor + predicate_name_len]).is_err() {
            return Err(format!(
                "frame refusal finding {index} predicate name is not UTF-8"
            ));
        }
        cursor += predicate_name_len;
        let (_, locator_used) = decode_locator(&payload[cursor..cursor + locator_len])?;
        if locator_used != locator_len {
            return Err(format!(
                "frame refusal finding {index} locator declares {locator_len} bytes but decodes \
                 {locator_used}"
            ));
        }
        cursor += locator_len;
        if core::str::from_utf8(&payload[cursor..cursor + reason_len]).is_err() {
            return Err(format!("frame refusal finding {index} reason is not UTF-8"));
        }
        offset = end;
    }
    if offset != payload.len() {
        return Err(format!(
            "frame refusal payload has {} trailing byte(s) after {offender_count} entries and \
             {finding_count} finding(s)",
            payload.len() - offset
        ));
    }
    Ok(())
}

pub fn declared_frame_refusal_version(payload: &[u8]) -> Option<u16> {
    if payload.len() >= 6 && &payload[0..4] == FRAME_REFUSAL_MAGIC {
        Some(read_u16(payload, 4))
    } else {
        None
    }
}

fn unknown_frame_refusal_version(version: u16) -> String {
    format!(
        "frame refusal version {version} is not a version this build decodes \
         ({FRAME_REFUSAL_VERSION}); the record was written by a \
         newer producer and cannot be read here"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::diagnostic_channel::{
        allocate_unit_budget, render_units_bounded, BudgetRequest, DiagUnit, OffFrameTerminal,
        UnitPresence, UnitRole, PREDICATE_UNDECLARED,
    };

    fn d42_offender() -> FrameRefusalOffenderV2 {
        FrameRefusalOffenderV2 {
            domain_id: 42,
            status_flags: 0x0000_0100,
            non_tolerated_flags: 0x0000_0100,
            predicate: Predicate::new(4216, "region_object_vtable_matches").unwrap(),
            authority: Authority::Engine { rva: 0x2A19_2A8 },
            locator: vec![
                LocatorLevel::new("country_container", 3, Some(12)),
                LocatorLevel::new("region", 41, Some(96)).with_address(0x2CE6_1D8A_0FF0),
            ],
            units: vec![
                DiagUnit::deciding("d42_region_walk_predicate", 4216),
                DiagUnit::locator("d42_region_walk_address", 0x2CE6_1D8A_0FF0),
            ],
            truncation: TruncationAccount::complete(2),
            off_frame: None,
            reason: "region object vtable echo did not match the module base: \
                     d42_region_walk_predicate=4216,d42_region_walk_address=49366849687536"
                .to_string(),
            reason_units_named: 2,
        }
    }

    fn d44_offender() -> FrameRefusalOffenderV2 {
        FrameRefusalOffenderV2 {
            domain_id: 44,
            status_flags: 0x0000_0200,
            non_tolerated_flags: 0x0000_0200,
            predicate: Predicate::new(4401, "country_supply_slot_is_readable").unwrap(),
            authority: Authority::SelfDomain,
            locator: vec![LocatorLevel::new("country_iteration", 3, Some(350))],
            units: vec![
                DiagUnit::new(
                    "d44_country_supply_failure_stage",
                    0,
                    UnitRole::Deciding,
                    UnitPresence::Observed,
                ),
                DiagUnit::new(
                    "d44_country_supply_failure_branch",
                    0,
                    UnitRole::Deciding,
                    UnitPresence::Observed,
                ),
                DiagUnit::new(
                    "d44_country_supply_slot_index",
                    0xFFFF_FFFF,
                    UnitRole::Locator,
                    UnitPresence::ResetSentinel,
                ),
            ],
            truncation: TruncationAccount::complete(3),
            off_frame: Some(OffFrameOutcome {
                spawn_generation: 7,
                elapsed_us: 51_000_000,
                deadline_us: 2_000_000,
                terminal: OffFrameTerminal::Running,
            }),
            reason: "country supply slot read refused: \
                     d44_country_supply_failure_stage=0,d44_country_supply_failure_branch=0"
                .to_string(),
            reason_units_named: 2,
        }
    }

    fn sample_v2() -> FrameRefusalRecordV2 {
        FrameRefusalRecordV2 {
            refused_hour: 60_831_436,
            offenders: vec![d42_offender(), d44_offender()],
            frame_findings: vec![FrameRefusalFinding {
                predicate: Predicate::new(0x0001_0002, "record_hour_label_matches_frame_hour")
                    .unwrap(),
                authority: Authority::Host,
                locator: vec![LocatorLevel::new("record", 12, Some(147))],
                reason: "domain 18 record_type 1: game hour 60830968 != frame hour 60831436"
                    .to_string(),
            }],
        }
    }

    #[test]
    fn v2_round_trips_with_predicate_authority_locator_units_and_truncation_preserved() {
        let value = sample_v2();
        let wire = value.encode().unwrap();
        validate_frame_refusal_payload(&wire).unwrap();
        assert_eq!(FrameRefusalRecordV2::decode(&wire).unwrap(), value);
    }

    #[test]
    fn a_zero_deciding_value_and_a_reset_sentinel_survive_and_stay_distinguishable() {
        let wire = sample_v2().encode().unwrap();
        let back = FrameRefusalRecordV2::decode(&wire).unwrap();
        let d44 = back.offenders.iter().find(|o| o.domain_id == 44).unwrap();
        let stage = d44
            .units
            .iter()
            .find(|unit| unit.name == "d44_country_supply_failure_stage")
            .expect("a zero-valued deciding unit must reach the wire");
        assert_eq!(stage.value, 0);
        assert_eq!(stage.presence, UnitPresence::Observed);
        assert!(stage.presence.value_is_a_measurement());
        let slot = d44
            .units
            .iter()
            .find(|unit| unit.name == "d44_country_supply_slot_index")
            .unwrap();
        assert_eq!(slot.value, 0xFFFF_FFFF);
        assert_eq!(slot.presence, UnitPresence::ResetSentinel);
        assert!(
            !slot.presence.value_is_a_measurement(),
            "0xFFFFFFFF must not read as an index"
        );
        assert_eq!(
            slot.render(),
            "d44_country_supply_slot_index=sentinel(0xffffffff)"
        );
    }

    #[test]
    fn v2_names_the_predicate_the_authority_and_an_ordinal_with_its_extent() {
        let d42 = d42_offender();
        let rendered = d42.render();
        assert!(
            rendered.contains("predicate=region_object_vtable_matches#4216"),
            "{rendered}"
        );
        assert!(
            rendered.contains("authority=engine@rva=0x2a192a8"),
            "{rendered}"
        );
        assert!(rendered.contains("country_container 3 of 12"), "{rendered}");
        assert!(rendered.contains("region 41 of 96"), "{rendered}");
        assert!(!rendered.contains("..."), "no literal ellipsis: {rendered}");
        let d91 = FrameRefusalOffenderV2 {
            authority: Authority::OtherDomain(69),
            ..d42
        };
        assert!(d91.render().contains("authority=domain69"));
    }

    #[test]
    fn an_off_frame_outcome_states_elapsed_against_its_deadline() {
        let d44 = d44_offender();
        let rendered = d44.render();
        assert!(rendered.contains("off_frame=running"), "{rendered}");
        assert!(rendered.contains("elapsed=51000000us"), "{rendered}");
        assert!(rendered.contains("deadline=2000000us"), "{rendered}");
        assert!(rendered.contains("OVERDUE"), "{rendered}");
        assert!(rendered.contains("gen=7"), "{rendered}");
    }

    #[test]
    fn an_entry_that_declares_no_predicate_is_refused_on_the_wire() {
        let wire = sample_v2().encode().unwrap();
        let mut bad = wire.clone();
        for byte in 0..4 {
            bad[FRAME_REFUSAL_HEADER_LEN + 12 + byte] = 0;
        }
        let error = validate_frame_refusal_payload(&bad).unwrap_err();
        assert!(error.contains("declares no predicate"), "{error}");
        assert!(FrameRefusalRecordV2::decode(&bad).is_err());
        let undeclared = FrameRefusalRecordV2 {
            refused_hour: 1,
            offenders: vec![FrameRefusalOffenderV2 {
                predicate: Predicate::undeclared(),
                ..d42_offender()
            }],
            frame_findings: Vec::new(),
        };
        let wire = undeclared.encode().unwrap();
        let back = FrameRefusalRecordV2::decode(&wire).unwrap();
        assert!(back.offenders[0].predicate.is_undeclared());
        assert_eq!(back.offenders[0].predicate.code.get(), PREDICATE_UNDECLARED);
    }

    #[test]
    fn a_truncation_account_that_contradicts_the_carried_units_is_refused() {
        let mut value = sample_v2();
        value.offenders[0].truncation.units_offered = 9;
        let error = value.encode().unwrap_err();
        assert!(error.contains("claims no truncation"), "{error}");

        let mut value = sample_v2();
        value.offenders[0].truncation.units_emitted = 1;
        let error = value.encode().unwrap_err();
        assert!(error.contains("carries 2 units but accounts 1"), "{error}");

        let mut value = sample_v2();
        value.offenders[0].truncation.units_emitted = 99;
        assert!(value.encode().is_err());

        let wire = sample_v2().encode().unwrap();
        let mut bad = wire.clone();
        bad[FRAME_REFUSAL_HEADER_LEN + 24] = 1;
        assert!(validate_frame_refusal_payload(&bad).is_err());
    }

    #[test]
    fn a_reason_that_names_none_of_its_carried_units_cannot_be_encoded() {
        let mut value = sample_v2();
        value.offenders[0].reason = "region walk refused".to_string();
        let error = value.encode().unwrap_err();
        assert!(error.contains("does not contain unit 0"), "{error}");

        let mut value = sample_v2();
        value.offenders[0].reason_units_named = 0;
        let error = value.encode().unwrap_err();
        assert!(error.contains("names none of them"), "{error}");

        let mut value = sample_v2();
        value.offenders[0].reason_units_named = 99;
        let error = value.encode().unwrap_err();
        assert!(error.contains("but carries 2"), "{error}");

        let mut value = sample_v2();
        value.offenders[0].units.clear();
        value.offenders[0].truncation = TruncationAccount::complete(0);
        value.offenders[0].reason_units_named = 1;
        let error = value.encode().unwrap_err();
        assert!(error.contains("names 1 units but carries 0"), "{error}");
    }

    #[test]
    fn a_forged_reason_unit_count_is_refused_on_the_way_in() {
        let wire = sample_v2().encode().unwrap();
        const AT: usize = FRAME_REFUSAL_HEADER_LEN + 17;
        assert_eq!(wire[AT], 2, "the fixture names both of its units");
        FrameRefusalRecordV2::decode(&wire).unwrap();

        let mut bad = wire.clone();
        bad[AT] = 3;
        let error = validate_frame_refusal_payload(&bad).unwrap_err();
        assert!(error.contains("but carries 2"), "{error}");
        assert!(FrameRefusalRecordV2::decode(&bad).is_err());

        let mut bad = wire.clone();
        bad[AT] = 0;
        let error = validate_frame_refusal_payload(&bad).unwrap_err();
        assert!(error.contains("names none of them"), "{error}");
        assert!(FrameRefusalRecordV2::decode(&bad).is_err());

        let back = FrameRefusalRecordV2::decode(&wire).unwrap();
        assert_eq!(back.offenders[0].reason_units_named, 2);
        assert!(back.offenders[0].render().contains("reason_names=2/2"));
    }

    #[test]
    fn a_payload_whose_reason_names_units_it_does_not_contain_is_refused_on_the_way_in() {
        let record = sample_v2();
        let wire = record.encode().unwrap();
        let original = &record.offenders[0].reason;
        let at = wire
            .windows(original.len())
            .position(|window| window == original.as_bytes())
            .expect("the reason is on the wire verbatim");
        let mut bad = wire.clone();
        let filler = "x".repeat(original.len());
        bad[at..at + original.len()].copy_from_slice(filler.as_bytes());

        let error = validate_frame_refusal_payload(&bad).unwrap_err();
        assert!(error.contains("does not contain unit 0"), "{error}");
        assert!(FrameRefusalRecordV2::decode(&bad).is_err());
    }

    #[test]
    fn a_unit_named_only_as_the_head_of_a_longer_one_does_not_count_as_named() {
        let short = DiagUnit::deciding("d44_stage", 1);
        let long = DiagUnit::deciding("d44_stage", 10);
        assert!(long.render().contains(&short.render()));

        assert!(check_reason_names_its_units(
            &format!("domain 44: {}", long.render()),
            core::slice::from_ref(&short),
            1,
            "test",
        )
        .is_err());
        for reason in [
            format!("domain 44: {}", short.render()),
            format!("domain 44: {},{}", short.render(), long.render()),
            format!("domain 44: {},{}", long.render(), short.render()),
        ] {
            check_reason_names_its_units(&reason, core::slice::from_ref(&short), 1, "test")
                .unwrap_or_else(|error| panic!("{reason:?}: {error}"));
        }
    }

    #[test]
    fn every_unit_the_count_claims_must_actually_appear() {
        let units = vec![
            DiagUnit::deciding("d44_stage", 0),
            DiagUnit::deciding("d44_branch", 0),
            DiagUnit::counter("d44_rows", 10_272),
        ];
        let honest = format!("domain 44: {},{}", units[0].render(), units[1].render());
        check_reason_names_its_units(&honest, &units, 2, "test").unwrap();
        let error = check_reason_names_its_units(&honest, &units, 3, "test").unwrap_err();
        assert!(error.contains("does not contain unit 2"), "{error}");
    }

    #[test]
    fn a_final_named_unit_before_the_producer_reason_suffix_counts_as_named() {
        let units = vec![
            DiagUnit::deciding("read_completion", 1),
            DiagUnit::deciding("answer_extent", 1),
        ];
        let reason = format!(
            "domain 88: {},{} | reason=D88 source refused",
            units[0].render(),
            units[1].render()
        );

        check_reason_names_its_units(&reason, &units, 2, "test").unwrap();

        let longer = format!(
            "domain 88: {},{}_suffix=2 | reason=D88 source refused",
            units[0].render(),
            units[1].render()
        );
        assert!(check_reason_names_its_units(&longer, &units, 2, "test").is_err());
    }

    #[test]
    fn a_nonzero_reserved_field_in_an_offender_entry_is_refused() {
        let wire = sample_v2().encode().unwrap();
        validate_frame_refusal_payload(&wire).unwrap();
        let mut bad = wire.clone();
        bad[FRAME_REFUSAL_HEADER_LEN + 2] = 1;
        let error = validate_frame_refusal_payload(&bad).unwrap_err();
        assert!(error.contains("nonzero reserved field"), "{error}");
        assert!(FrameRefusalRecordV2::decode(&bad).is_err());
    }

    #[test]
    fn a_verbose_domain_names_its_deciding_unit_and_states_what_the_reason_left_out() {
        let mut units: Vec<DiagUnit> = (0..600u64)
            .map(|index| DiagUnit::counter(format!("d44_counter_{index:04}"), index))
            .collect();
        units.push(DiagUnit::deciding("d44_country_supply_failure_stage", 0));
        let grants = allocate_unit_budget(
            &[BudgetRequest {
                domain_id: 44,
                units,
            }],
            4_096,
            FRAME_REFUSAL_MAX_UNITS_LEN,
        );
        let grant = &grants[0];
        let prefix = "domain 44: ";
        let (rendered, named) = render_units_bounded(
            &grant.units,
            FRAME_REFUSAL_MAX_REASON_LEN.saturating_sub(prefix.len()),
        );
        let reason = format!("{prefix}{rendered}");

        assert!(
            reason.len() <= FRAME_REFUSAL_MAX_REASON_LEN,
            "reason is {} bytes",
            reason.len()
        );
        assert!(
            reason.contains("d44_country_supply_failure_stage=0"),
            "the deciding unit must be named first: {reason}"
        );
        assert!(named >= 1 && (named as usize) < grant.units.len());
        assert!(!reason.contains("..."), "no literal ellipsis: {reason}");

        let record = FrameRefusalRecordV2 {
            refused_hour: 1,
            offenders: vec![FrameRefusalOffenderV2 {
                domain_id: 44,
                status_flags: 0x0000_0200,
                non_tolerated_flags: 0x0000_0200,
                predicate: Predicate::new(4401, "country_supply_slot_is_readable").unwrap(),
                authority: Authority::SelfDomain,
                locator: Vec::new(),
                units: grant.units.clone(),
                truncation: grant.account,
                off_frame: None,
                reason,
                reason_units_named: named,
            }],
            frame_findings: Vec::new(),
        };
        let wire = record.encode().unwrap();
        let back = FrameRefusalRecordV2::decode(&wire).unwrap();
        let offender = &back.offenders[0];
        assert_eq!(offender.reason_units_named, named);
        assert!(offender
            .reason
            .contains("d44_country_supply_failure_stage=0"));
        assert_eq!(offender.units[0].name, "d44_country_supply_failure_stage");
        assert!(
            (offender.reason_units_named as usize) < offender.units.len(),
            "this fixture is only meaningful while the reason cannot name everything"
        );
        assert!(offender.render().contains(&format!(
            "reason_names={}/{}",
            offender.reason_units_named,
            offender.units.len()
        )));
    }

    #[test]
    fn a_version_this_build_does_not_know_is_refused_and_names_the_version_it_does() {
        let known = sample_v2().encode().unwrap();
        for version in [0u16, 1, 3, 4, u16::MAX] {
            let mut future = known.clone();
            future[4..6].copy_from_slice(&version.to_le_bytes());

            let error = validate_frame_refusal_payload(&future).unwrap_err();
            assert!(
                error.contains(&format!("frame refusal version {version}")),
                "the refusal must name the version it could not read: {error}"
            );
            assert!(
                error.contains("not a version this build decodes"),
                "{error}"
            );
            assert!(
                error.contains(&format!("({FRAME_REFUSAL_VERSION})")),
                "the refusal must name every version this build does decode: {error}"
            );

            let decode_error = FrameRefusalRecordV2::decode(&future).unwrap_err();
            assert_eq!(
                decode_error, error,
                "the validator and the decoder must refuse an unknown version identically"
            );
        }
    }

    #[test]
    fn the_known_version_passes_the_entry_points_that_refuse_unknown_ones() {
        validate_frame_refusal_payload(&sample_v2().encode().unwrap()).unwrap();
        assert_eq!(
            FrameRefusalRecordV2::decode(&sample_v2().encode().unwrap()).unwrap(),
            sample_v2()
        );
    }

    #[test]
    fn bytes_that_are_not_a_refusal_are_diagnosed_by_magic_not_by_version() {
        let mut not_a_record = sample_v2().encode().unwrap();
        not_a_record[0] = b'X';
        not_a_record[4..6].copy_from_slice(&3u16.to_le_bytes());
        assert_eq!(
            declared_frame_refusal_version(&not_a_record),
            None,
            "a payload without the magic declares no version, whatever is in its version word"
        );
        let error = validate_frame_refusal_payload(&not_a_record).unwrap_err();
        assert!(error.contains("magic is not H4FR"), "{error}");
        let error = FrameRefusalRecordV2::decode(&not_a_record).unwrap_err();
        assert!(error.contains("magic is not H4FR"), "{error}");
        for len in 0..FRAME_REFUSAL_HEADER_LEN {
            let short = &not_a_record[..len.min(not_a_record.len())];
            let error = validate_frame_refusal_payload(short).unwrap_err();
            assert!(!error.contains("this build decodes"), "{len}: {error}");
            let error = FrameRefusalRecordV2::decode(short).unwrap_err();
            assert!(!error.contains("this build decodes"), "{len}: {error}");
        }
        let error = validate_frame_refusal_payload(&[]).unwrap_err();
        assert!(error.contains("below the"), "{error}");
        let error = FrameRefusalRecordV2::decode(&[]).unwrap_err();
        assert!(error.contains("below the"), "{error}");
    }

    #[test]
    fn the_record_is_wired_into_the_canonical_dispatch_and_misuse_fails_loudly() {
        use crate::record_semantics::{
            validate_compact_record_semantics, COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_REFUSED,
        };
        let wire = sample_v2().encode().unwrap();
        validate_compact_record_semantics(
            COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_REFUSED,
            0,
            &wire,
        )
        .expect("the record must be accepted through the canonical dispatch");

        let mut future = wire.clone();
        future[4] = 9;
        assert!(validate_compact_record_semantics(
            COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_REFUSED,
            0,
            &future
        )
        .is_err());

        assert!(
            validate_compact_record_semantics(47, COMPACT_RECORD_TYPE_FRAME_REFUSED, 0, &wire)
                .is_err(),
            "a frame-level record carried on a domain envelope must be refused"
        );

        let unassigned_record_type = u16::MAX;
        let unknown = validate_compact_record_semantics(
            COMPACT_DOMAIN_FRAME,
            unassigned_record_type,
            0,
            &wire,
        );
        assert!(
            unknown
                .as_ref()
                .err()
                .is_some_and(|error| error.contains("unknown compact record type")),
            "an undefined record type must fail loudly, got {unknown:?}"
        );
    }

    #[test]
    fn a_refusal_with_no_attributable_domain_is_still_a_complete_record() {
        let frame_only = FrameRefusalRecordV2 {
            refused_hour: 60_831_500,
            offenders: Vec::new(),
            frame_findings: vec![FrameRefusalFinding {
                predicate: Predicate::new(0x0001_0003, "frame_end_present_and_singular").unwrap(),
                authority: Authority::Host,
                locator: Vec::new(),
                reason: "framing: frame carries no FRAME_END record".to_string(),
            }],
        };
        let wire = frame_only
            .encode()
            .expect("a frame-level-only refusal must be encodable");
        validate_frame_refusal_payload(&wire).unwrap();
        let back = FrameRefusalRecordV2::decode(&wire).unwrap();
        assert_eq!(back, frame_only);
        assert!(back.offenders.is_empty());
        assert_eq!(back.frame_findings.len(), 1);
        assert!(back.frame_findings[0]
            .render()
            .contains("predicate=frame_end_present_and_singular#65539"));
        assert!(back.frame_findings[0].render().contains("authority=host"));
        use crate::record_semantics::{
            validate_compact_record_semantics, COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_REFUSED,
        };
        validate_compact_record_semantics(
            COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_REFUSED,
            0,
            &wire,
        )
        .expect("a frame-level-only refusal must pass the canonical dispatch");
    }

    #[test]
    fn an_offender_that_forced_nothing_or_says_nothing_does_not_encode() {
        let mut no_bits = sample_v2();
        no_bits.offenders[0].non_tolerated_flags = 0;
        let error = no_bits.encode().unwrap_err();
        assert!(
            error.contains("no non-tolerated bits"),
            "an offender that tolerated everything did not force the refusal: {error}"
        );

        let mut no_reason = sample_v2();
        no_reason.offenders[0].reason = String::new();
        no_reason.offenders[0].units = Vec::new();
        no_reason.offenders[0].truncation = TruncationAccount::complete(0);
        no_reason.offenders[0].reason_units_named = 0;
        let error = no_reason.encode().unwrap_err();
        assert!(error.contains("no reason"), "{error}");
    }

    #[test]
    fn a_refusal_that_substantiates_nothing_is_still_refused() {
        let empty = FrameRefusalRecordV2 {
            refused_hour: 1,
            offenders: Vec::new(),
            frame_findings: Vec::new(),
        };
        let error = empty.encode().unwrap_err();
        assert!(error.contains("neither an offending domain nor a frame-level finding"));

        let mut wire = sample_v2().encode().unwrap();
        wire[12] = 0;
        wire[13] = 0;
        wire[14] = 0;
        wire[15] = 0;
        assert!(validate_frame_refusal_payload(&wire).is_err());

        let mut wire = FrameRefusalRecordV2 {
            refused_hour: 1,
            offenders: Vec::new(),
            frame_findings: vec![FrameRefusalFinding {
                predicate: Predicate::new(7, "x").unwrap(),
                authority: Authority::Host,
                locator: Vec::new(),
                reason: "y".to_string(),
            }],
        }
        .encode()
        .unwrap();
        for byte in 0..4 {
            wire[FRAME_REFUSAL_HEADER_LEN + byte] = 0;
        }
        let error = validate_frame_refusal_payload(&wire).unwrap_err();
        assert!(error.contains("declares no predicate"), "{error}");
    }

    #[test]
    fn every_v2_truncation_and_corruption_fails_closed() {
        let wire = sample_v2().encode().unwrap();
        for len in 0..wire.len() {
            assert!(
                validate_frame_refusal_payload(&wire[..len]).is_err(),
                "prefix of {len} byte(s) must not validate"
            );
        }
        let mut trailing = wire.clone();
        trailing.push(0);
        assert!(validate_frame_refusal_payload(&trailing).is_err());
        for (index, label) in [(0usize, "magic"), (4, "version"), (6, "header_len")] {
            let mut bad = wire.clone();
            bad[index] ^= 0xFF;
            assert!(
                validate_frame_refusal_payload(&bad).is_err(),
                "corrupt {label} must fail"
            );
        }
        for index in FRAME_REFUSAL_HEADER_LEN..wire.len() {
            let mut bad = wire.clone();
            bad[index] ^= 0xFF;
            let _ = validate_frame_refusal_payload(&bad);
            let _ = FrameRefusalRecordV2::decode(&bad);
        }
    }

    #[test]
    fn under_budget_pressure_every_offenders_deciding_unit_still_reaches_the_wire() {
        let requests: Vec<BudgetRequest> = (40..=52u16)
            .map(|domain_id| {
                let mut units = vec![DiagUnit::new(
                    format!("d{domain_id}_deciding"),
                    0,
                    UnitRole::Deciding,
                    UnitPresence::Observed,
                )];
                for index in 0..120u64 {
                    units.push(DiagUnit::counter(
                        format!("d{domain_id}_counter_{index:03}"),
                        index,
                    ));
                }
                units.rotate_left(1);
                BudgetRequest { domain_id, units }
            })
            .collect();
        let grants = allocate_unit_budget(&requests, 512, FRAME_REFUSAL_MAX_UNITS_LEN);
        let offenders: Vec<FrameRefusalOffenderV2> = grants
            .iter()
            .map(|grant| FrameRefusalOffenderV2 {
                domain_id: grant.domain_id,
                status_flags: 0x0000_0100,
                non_tolerated_flags: 0x0000_0100,
                predicate: Predicate::new(u32::from(grant.domain_id) * 100 + 1, "walk_predicate")
                    .unwrap(),
                authority: Authority::SelfDomain,
                locator: Vec::new(),
                units: grant.units.clone(),
                truncation: grant.account,
                off_frame: None,
                reason: format!(
                    "refused: {}",
                    render_units_bounded(
                        &grant.units,
                        FRAME_REFUSAL_MAX_REASON_LEN.saturating_sub("refused: ".len())
                    )
                    .0
                ),
                reason_units_named: render_units_bounded(
                    &grant.units,
                    FRAME_REFUSAL_MAX_REASON_LEN.saturating_sub("refused: ".len()),
                )
                .1,
            })
            .collect();
        let record = FrameRefusalRecordV2 {
            refused_hour: 1,
            offenders,
            frame_findings: Vec::new(),
        };
        let wire = record.encode().unwrap();
        let back = FrameRefusalRecordV2::decode(&wire).unwrap();
        assert_eq!(back.offenders.len(), 13);
        for offender in &back.offenders {
            assert!(
                offender
                    .units
                    .iter()
                    .any(|unit| unit.role == UnitRole::Deciding),
                "domain {} lost its deciding unit under budget pressure",
                offender.domain_id
            );
            assert_eq!(offender.truncation.kind, TruncationKind::BudgetShare);
            assert!(offender.truncation.units_dropped() > 0);
            assert!(offender.truncation.bytes_dropped > 0);
            assert!(
                !offender.render().contains("..."),
                "truncation must be stated, not elided"
            );
        }
    }
}
