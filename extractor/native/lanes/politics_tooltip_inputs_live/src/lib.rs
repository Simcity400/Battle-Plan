//! A24 exact politics-tooltip computation carrier (reserved d73).
//!
//! This lane preserves engine-evaluated semantic inputs. It deliberately does
//! not accept static-script presence, OCR, RenderDoc text, or replay-side
//! trigger/modifier evaluation as a production source.

use std::collections::BTreeSet;

pub const DOMAIN_ID: u16 = 73;
pub const DOMAIN_NAME: &str = "politics_tooltip_inputs_live";
pub const SCHEMA_VERSION: u32 = 1;
pub const AUTHORITY_EXACT_A729_POLITICS_PRODUCER: u32 = 1;
pub const MAGIC: u32 = u32::from_le_bytes(*b"PTI1");
pub const HEADER_LEN: usize = 76;
pub const MAX_ROWS: usize = 131_072;
pub const MAX_TERMS_PER_ROW: usize = 65_536;
pub const MAX_STRING_BYTES: usize = 16_384;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum PoliticsLineKind {
    EvaluatedTriggerResultAndReasonTree,
    EffectiveHireOrRemovalCost,
    AggregatedConditionalModifier,
    AdvisorSlotCapacityVacancyEligibilityAndDisabledReason,
    TimedDynamicModifierValueSourceAndExpiry,
    DisplayedPartyPopularityChangeSource,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AvailabilityState {
    Measured,
    NotApplicable,
    Unobtainable { field: String, reason: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CostAction {
    Hire,
    Remove,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReasonNode {
    pub source_ordinal: u32,
    pub node_id: String,
    pub parent_node_id: Option<String>,
    pub source_identity: String,
    pub result: bool,
    pub resolved_reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NumericSourceTerm {
    pub source_ordinal: u32,
    pub source_identity: String,
    pub evaluation_context: String,
    pub raw_value: i64,
    pub scale: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PoliticsTypedValue {
    TriggerEvaluation {
        result: bool,
        reason_tree: Vec<ReasonNode>,
    },
    EffectiveCost {
        action: CostAction,
        resource_token: String,
        raw_value: i64,
        scale: u32,
        affordable: bool,
    },
    ConditionalModifierAggregate {
        modifier_token: String,
        raw_total: i64,
        scale: u32,
        terms: Vec<NumericSourceTerm>,
    },
    AdvisorSlotState {
        capacity: u32,
        occupied: u32,
        vacancy: bool,
        eligible: bool,
        disabled_reason_tree: Vec<ReasonNode>,
    },
    TimedDynamicModifier {
        modifier_token: String,
        raw_value: i64,
        scale: u32,
        source_token: String,
        applied_hour: Option<u64>,
        expiry_hour: Option<u64>,
    },
    PartyPopularityChange {
        party_token: String,
        raw_change: i64,
        scale: u32,
        sources: Vec<NumericSourceTerm>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PoliticsTooltipInput {
    pub frame_hour: u64,
    pub country_tag: String,
    pub target_kind: String,
    pub target_token_or_slot: String,
    pub line_kind: PoliticsLineKind,
    pub availability_state: AvailabilityState,
    pub typed_value: Option<PoliticsTypedValue>,
    pub source_identity: String,
    pub evaluation_context: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PoliticsTooltipFrame {
    pub elapsed_wall_ms: u64,
    pub status_flags: u32,
    pub frame_hour: u64,
    pub producer_manifest_sha256: [u8; 32],
    pub completeness_record_identity: String,
    pub expected_row_count: u32,
    pub rows: Vec<PoliticsTooltipInput>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PoliticsTooltipError {
    UnauthenticatedSource,
    Source(String),
    Invalid(String),
    Truncated,
    TrailingBytes,
}

pub trait ExactPoliticsTooltipSource {
    fn is_exact_a729_politics_source(&self) -> bool;
    fn producer_manifest_sha256(&self) -> [u8; 32];
    fn completeness_record_identity(&self, frame_hour: u64) -> Result<String, String>;
    fn expected_row_count(&self, frame_hour: u64) -> Result<u32, String>;
    fn collect_frame(&self, frame_hour: u64) -> Result<Vec<PoliticsTooltipInput>, String>;
}

pub fn collect_and_encode<S: ExactPoliticsTooltipSource>(
    source: &S,
    frame_hour: u64,
    elapsed_wall_ms: u64,
    status_flags: u32,
) -> Result<Vec<u8>, PoliticsTooltipError> {
    if !source.is_exact_a729_politics_source() {
        return Err(PoliticsTooltipError::UnauthenticatedSource);
    }
    encode_payload(&PoliticsTooltipFrame {
        elapsed_wall_ms,
        status_flags,
        frame_hour,
        producer_manifest_sha256: source.producer_manifest_sha256(),
        completeness_record_identity: source
            .completeness_record_identity(frame_hour)
            .map_err(PoliticsTooltipError::Source)?,
        expected_row_count: source
            .expected_row_count(frame_hour)
            .map_err(PoliticsTooltipError::Source)?,
        rows: source
            .collect_frame(frame_hour)
            .map_err(PoliticsTooltipError::Source)?,
    })
}

pub fn validate_frame(frame: &PoliticsTooltipFrame) -> Result<(), PoliticsTooltipError> {
    if frame.producer_manifest_sha256 == [0; 32] {
        return invalid("producer manifest hash is zero");
    }
    validate_nonempty(&frame.completeness_record_identity, "completeness record")?;
    if frame.rows.len() > MAX_ROWS || frame.expected_row_count as usize != frame.rows.len() {
        return invalid("row completeness/count mismatch");
    }
    let mut keys = BTreeSet::new();
    for row in &frame.rows {
        if row.frame_hour != frame.frame_hour {
            return invalid("row frame_hour differs from header");
        }
        validate_country_tag(&row.country_tag)?;
        validate_nonempty(&row.target_kind, "target kind")?;
        validate_nonempty(&row.target_token_or_slot, "target token or slot")?;
        validate_nonempty(&row.source_identity, "source identity")?;
        validate_nonempty(&row.evaluation_context, "evaluation context")?;
        if !keys.insert((
            row.country_tag.as_str(),
            row.target_kind.as_str(),
            row.target_token_or_slot.as_str(),
            row.line_kind,
        )) {
            return invalid("duplicate politics tooltip row key");
        }
        match (&row.availability_state, &row.typed_value) {
            (AvailabilityState::Measured, Some(value)) => {
                validate_typed_value(row.line_kind, value)?;
            }
            (AvailabilityState::NotApplicable, None) => {}
            (AvailabilityState::Unobtainable { field, reason }, None) => {
                validate_nonempty(field, "unobtainable field")?;
                validate_nonempty(reason, "unobtainable reason")?;
            }
            _ => return invalid("availability and typed_value disagree"),
        }
    }
    Ok(())
}

fn validate_typed_value(
    line_kind: PoliticsLineKind,
    value: &PoliticsTypedValue,
) -> Result<(), PoliticsTooltipError> {
    match (line_kind, value) {
        (
            PoliticsLineKind::EvaluatedTriggerResultAndReasonTree,
            PoliticsTypedValue::TriggerEvaluation { reason_tree, .. },
        ) => validate_reason_tree(reason_tree),
        (
            PoliticsLineKind::EffectiveHireOrRemovalCost,
            PoliticsTypedValue::EffectiveCost {
                resource_token,
                scale,
                ..
            },
        ) => {
            validate_nonempty(resource_token, "cost resource")?;
            validate_scale(*scale)
        }
        (
            PoliticsLineKind::AggregatedConditionalModifier,
            PoliticsTypedValue::ConditionalModifierAggregate {
                modifier_token,
                raw_total,
                scale,
                terms,
            },
        ) => {
            validate_nonempty(modifier_token, "modifier token")?;
            validate_scale(*scale)?;
            validate_numeric_terms(terms, *scale, Some(*raw_total))
        }
        (
            PoliticsLineKind::AdvisorSlotCapacityVacancyEligibilityAndDisabledReason,
            PoliticsTypedValue::AdvisorSlotState {
                capacity,
                occupied,
                vacancy,
                disabled_reason_tree,
                ..
            },
        ) => {
            if occupied > capacity || *vacancy != (*occupied < *capacity) {
                return invalid("advisor capacity/occupied/vacancy disagree");
            }
            validate_reason_tree(disabled_reason_tree)
        }
        (
            PoliticsLineKind::TimedDynamicModifierValueSourceAndExpiry,
            PoliticsTypedValue::TimedDynamicModifier {
                modifier_token,
                scale,
                source_token,
                applied_hour,
                expiry_hour,
                ..
            },
        ) => {
            validate_nonempty(modifier_token, "timed modifier token")?;
            validate_nonempty(source_token, "timed modifier source")?;
            validate_scale(*scale)?;
            if matches!((applied_hour, expiry_hour), (Some(start), Some(end)) if end < start) {
                return invalid("timed modifier expiry precedes application");
            }
            Ok(())
        }
        (
            PoliticsLineKind::DisplayedPartyPopularityChangeSource,
            PoliticsTypedValue::PartyPopularityChange {
                party_token,
                raw_change,
                scale,
                sources,
            },
        ) => {
            validate_nonempty(party_token, "party token")?;
            validate_scale(*scale)?;
            validate_numeric_terms(sources, *scale, Some(*raw_change))
        }
        _ => invalid("line_kind and typed_value variant disagree"),
    }
}

fn validate_reason_tree(nodes: &[ReasonNode]) -> Result<(), PoliticsTooltipError> {
    if nodes.len() > MAX_TERMS_PER_ROW {
        return invalid("reason tree exceeds bound");
    }
    let mut ids = BTreeSet::new();
    let mut prior = None;
    for node in nodes {
        if prior.is_some_and(|value| node.source_ordinal <= value) {
            return invalid("reason ordinals are not strictly increasing");
        }
        prior = Some(node.source_ordinal);
        validate_nonempty(&node.node_id, "reason node id")?;
        validate_nonempty(&node.source_identity, "reason source identity")?;
        validate_nonempty(&node.resolved_reason, "resolved reason")?;
        if !ids.insert(node.node_id.as_str()) {
            return invalid("duplicate reason node id");
        }
        if let Some(parent) = &node.parent_node_id {
            validate_nonempty(parent, "reason parent id")?;
            if !ids.contains(parent.as_str()) {
                return invalid("reason parent must precede child");
            }
        }
    }
    Ok(())
}

fn validate_numeric_terms(
    terms: &[NumericSourceTerm],
    scale: u32,
    expected_total: Option<i64>,
) -> Result<(), PoliticsTooltipError> {
    if terms.len() > MAX_TERMS_PER_ROW {
        return invalid("numeric term list exceeds bound");
    }
    let mut prior = None;
    let mut sum = 0i64;
    for term in terms {
        if prior.is_some_and(|value| term.source_ordinal <= value) {
            return invalid("numeric term ordinals are not strictly increasing");
        }
        prior = Some(term.source_ordinal);
        validate_nonempty(&term.source_identity, "numeric term source")?;
        validate_nonempty(&term.evaluation_context, "numeric term context")?;
        if term.scale != scale {
            return invalid("numeric term scale differs from aggregate");
        }
        sum = sum
            .checked_add(term.raw_value)
            .ok_or_else(|| PoliticsTooltipError::Invalid("numeric term sum overflow".into()))?;
    }
    if expected_total.is_some_and(|expected| sum != expected) {
        return invalid("numeric terms do not equal aggregate");
    }
    Ok(())
}

pub fn encode_payload(frame: &PoliticsTooltipFrame) -> Result<Vec<u8>, PoliticsTooltipError> {
    validate_frame(frame)?;
    let mut out = Vec::new();
    put_u32(&mut out, MAGIC);
    put_u32(&mut out, SCHEMA_VERSION);
    put_u32(&mut out, AUTHORITY_EXACT_A729_POLITICS_PRODUCER);
    put_u32(&mut out, frame.status_flags);
    put_u64(&mut out, frame.elapsed_wall_ms);
    put_u64(&mut out, frame.frame_hour);
    out.extend_from_slice(&frame.producer_manifest_sha256);
    put_u32(&mut out, frame.expected_row_count);
    put_u32(&mut out, frame.rows.len() as u32);
    put_u32(&mut out, HEADER_LEN as u32);
    put_string(&mut out, &frame.completeness_record_identity)?;
    for row in &frame.rows {
        put_u64(&mut out, row.frame_hour);
        put_string(&mut out, &row.country_tag)?;
        put_string(&mut out, &row.target_kind)?;
        put_string(&mut out, &row.target_token_or_slot)?;
        put_u32(&mut out, line_kind_code(row.line_kind));
        put_string(&mut out, &row.source_identity)?;
        put_string(&mut out, &row.evaluation_context)?;
        match &row.availability_state {
            AvailabilityState::Measured => {
                put_u32(&mut out, 0);
                put_optional_string(&mut out, None)?;
                put_optional_string(&mut out, None)?;
            }
            AvailabilityState::NotApplicable => {
                put_u32(&mut out, 1);
                put_optional_string(&mut out, None)?;
                put_optional_string(&mut out, None)?;
            }
            AvailabilityState::Unobtainable { field, reason } => {
                put_u32(&mut out, 2);
                put_optional_string(&mut out, Some(field))?;
                put_optional_string(&mut out, Some(reason))?;
            }
        }
        put_typed_value(&mut out, row.typed_value.as_ref())?;
    }
    Ok(out)
}

pub fn decode_payload(payload: &[u8]) -> Result<PoliticsTooltipFrame, PoliticsTooltipError> {
    let mut reader = Reader::new(payload);
    if reader.u32()? != MAGIC || reader.u32()? != SCHEMA_VERSION {
        return invalid("magic/schema mismatch");
    }
    if reader.u32()? != AUTHORITY_EXACT_A729_POLITICS_PRODUCER {
        return Err(PoliticsTooltipError::UnauthenticatedSource);
    }
    let status_flags = reader.u32()?;
    let elapsed_wall_ms = reader.u64()?;
    let frame_hour = reader.u64()?;
    let producer_manifest_sha256 = reader.array32()?;
    let expected_row_count = reader.u32()?;
    let row_count = bounded_count(reader.u32()?, MAX_ROWS)?;
    if reader.u32()? as usize != HEADER_LEN {
        return invalid("header length invalid");
    }
    let completeness_record_identity = reader.string()?;
    let mut rows = Vec::with_capacity(row_count);
    for _ in 0..row_count {
        let row_frame_hour = reader.u64()?;
        let country_tag = reader.string()?;
        let target_kind = reader.string()?;
        let target_token_or_slot = reader.string()?;
        let line_kind = decode_line_kind(reader.u32()?)?;
        let source_identity = reader.string()?;
        let evaluation_context = reader.string()?;
        let availability_code = reader.u32()?;
        let field = reader.optional_string()?;
        let reason = reader.optional_string()?;
        let availability_state = match (availability_code, field, reason) {
            (0, None, None) => AvailabilityState::Measured,
            (1, None, None) => AvailabilityState::NotApplicable,
            (2, Some(field), Some(reason)) => AvailabilityState::Unobtainable { field, reason },
            _ => return invalid("availability encoding invalid"),
        };
        rows.push(PoliticsTooltipInput {
            frame_hour: row_frame_hour,
            country_tag,
            target_kind,
            target_token_or_slot,
            line_kind,
            availability_state,
            typed_value: reader.typed_value()?,
            source_identity,
            evaluation_context,
        });
    }
    if !reader.done() {
        return Err(PoliticsTooltipError::TrailingBytes);
    }
    let frame = PoliticsTooltipFrame {
        elapsed_wall_ms,
        status_flags,
        frame_hour,
        producer_manifest_sha256,
        completeness_record_identity,
        expected_row_count,
        rows,
    };
    validate_frame(&frame)?;
    Ok(frame)
}

fn put_typed_value(
    out: &mut Vec<u8>,
    value: Option<&PoliticsTypedValue>,
) -> Result<(), PoliticsTooltipError> {
    match value {
        None => put_u32(out, 0),
        Some(PoliticsTypedValue::TriggerEvaluation {
            result,
            reason_tree,
        }) => {
            put_u32(out, 1);
            put_bool(out, *result);
            put_reason_tree(out, reason_tree)?;
        }
        Some(PoliticsTypedValue::EffectiveCost {
            action,
            resource_token,
            raw_value,
            scale,
            affordable,
        }) => {
            put_u32(out, 2);
            put_u32(
                out,
                match action {
                    CostAction::Hire => 0,
                    CostAction::Remove => 1,
                },
            );
            put_string(out, resource_token)?;
            put_i64(out, *raw_value);
            put_u32(out, *scale);
            put_bool(out, *affordable);
        }
        Some(PoliticsTypedValue::ConditionalModifierAggregate {
            modifier_token,
            raw_total,
            scale,
            terms,
        }) => {
            put_u32(out, 3);
            put_string(out, modifier_token)?;
            put_i64(out, *raw_total);
            put_u32(out, *scale);
            put_numeric_terms(out, terms)?;
        }
        Some(PoliticsTypedValue::AdvisorSlotState {
            capacity,
            occupied,
            vacancy,
            eligible,
            disabled_reason_tree,
        }) => {
            put_u32(out, 4);
            put_u32(out, *capacity);
            put_u32(out, *occupied);
            put_bool(out, *vacancy);
            put_bool(out, *eligible);
            put_reason_tree(out, disabled_reason_tree)?;
        }
        Some(PoliticsTypedValue::TimedDynamicModifier {
            modifier_token,
            raw_value,
            scale,
            source_token,
            applied_hour,
            expiry_hour,
        }) => {
            put_u32(out, 5);
            put_string(out, modifier_token)?;
            put_i64(out, *raw_value);
            put_u32(out, *scale);
            put_string(out, source_token)?;
            put_optional_u64(out, *applied_hour);
            put_optional_u64(out, *expiry_hour);
        }
        Some(PoliticsTypedValue::PartyPopularityChange {
            party_token,
            raw_change,
            scale,
            sources,
        }) => {
            put_u32(out, 6);
            put_string(out, party_token)?;
            put_i64(out, *raw_change);
            put_u32(out, *scale);
            put_numeric_terms(out, sources)?;
        }
    }
    Ok(())
}

fn put_reason_tree(out: &mut Vec<u8>, nodes: &[ReasonNode]) -> Result<(), PoliticsTooltipError> {
    put_u32(out, nodes.len() as u32);
    for node in nodes {
        put_u32(out, node.source_ordinal);
        put_string(out, &node.node_id)?;
        put_optional_string(out, node.parent_node_id.as_deref())?;
        put_string(out, &node.source_identity)?;
        put_bool(out, node.result);
        put_string(out, &node.resolved_reason)?;
    }
    Ok(())
}

fn put_numeric_terms(
    out: &mut Vec<u8>,
    terms: &[NumericSourceTerm],
) -> Result<(), PoliticsTooltipError> {
    put_u32(out, terms.len() as u32);
    for term in terms {
        put_u32(out, term.source_ordinal);
        put_string(out, &term.source_identity)?;
        put_string(out, &term.evaluation_context)?;
        put_i64(out, term.raw_value);
        put_u32(out, term.scale);
    }
    Ok(())
}

fn line_kind_code(kind: PoliticsLineKind) -> u32 {
    match kind {
        PoliticsLineKind::EvaluatedTriggerResultAndReasonTree => 0,
        PoliticsLineKind::EffectiveHireOrRemovalCost => 1,
        PoliticsLineKind::AggregatedConditionalModifier => 2,
        PoliticsLineKind::AdvisorSlotCapacityVacancyEligibilityAndDisabledReason => 3,
        PoliticsLineKind::TimedDynamicModifierValueSourceAndExpiry => 4,
        PoliticsLineKind::DisplayedPartyPopularityChangeSource => 5,
    }
}

fn decode_line_kind(value: u32) -> Result<PoliticsLineKind, PoliticsTooltipError> {
    match value {
        0 => Ok(PoliticsLineKind::EvaluatedTriggerResultAndReasonTree),
        1 => Ok(PoliticsLineKind::EffectiveHireOrRemovalCost),
        2 => Ok(PoliticsLineKind::AggregatedConditionalModifier),
        3 => Ok(PoliticsLineKind::AdvisorSlotCapacityVacancyEligibilityAndDisabledReason),
        4 => Ok(PoliticsLineKind::TimedDynamicModifierValueSourceAndExpiry),
        5 => Ok(PoliticsLineKind::DisplayedPartyPopularityChangeSource),
        _ => invalid("line kind invalid"),
    }
}

fn validate_country_tag(value: &str) -> Result<(), PoliticsTooltipError> {
    if value.len() != 3
        || !value
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
    {
        invalid("country tag is not canonical three-byte tag")
    } else {
        Ok(())
    }
}

fn validate_scale(scale: u32) -> Result<(), PoliticsTooltipError> {
    if scale == 0 {
        invalid("fixed-point scale is zero")
    } else {
        Ok(())
    }
}

fn validate_nonempty(value: &str, field: &str) -> Result<(), PoliticsTooltipError> {
    if value.trim().is_empty() || value.len() > MAX_STRING_BYTES {
        invalid(&format!("{field} empty or oversized"))
    } else {
        Ok(())
    }
}

fn invalid<T>(message: &str) -> Result<T, PoliticsTooltipError> {
    Err(PoliticsTooltipError::Invalid(message.into()))
}

fn bounded_count(value: u32, bound: usize) -> Result<usize, PoliticsTooltipError> {
    let value = value as usize;
    if value > bound {
        invalid("count exceeds bound")
    } else {
        Ok(value)
    }
}

fn put_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn put_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn put_i64(out: &mut Vec<u8>, value: i64) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn put_bool(out: &mut Vec<u8>, value: bool) {
    put_u32(out, u32::from(value));
}
fn put_optional_u64(out: &mut Vec<u8>, value: Option<u64>) {
    put_u32(out, u32::from(value.is_some()));
    if let Some(value) = value {
        put_u64(out, value);
    }
}

fn put_string(out: &mut Vec<u8>, value: &str) -> Result<(), PoliticsTooltipError> {
    validate_nonempty(value, "wire string")?;
    put_u32(out, value.len() as u32);
    out.extend_from_slice(value.as_bytes());
    Ok(())
}

fn put_optional_string(out: &mut Vec<u8>, value: Option<&str>) -> Result<(), PoliticsTooltipError> {
    match value {
        None => put_u32(out, u32::MAX),
        Some(value) => {
            validate_nonempty(value, "optional wire string")?;
            put_u32(out, value.len() as u32);
            out.extend_from_slice(value.as_bytes());
        }
    }
    Ok(())
}

struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }
    fn take(&mut self, count: usize) -> Result<&'a [u8], PoliticsTooltipError> {
        let end = self
            .offset
            .checked_add(count)
            .ok_or(PoliticsTooltipError::Truncated)?;
        let value = self
            .bytes
            .get(self.offset..end)
            .ok_or(PoliticsTooltipError::Truncated)?;
        self.offset = end;
        Ok(value)
    }
    fn u32(&mut self) -> Result<u32, PoliticsTooltipError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, PoliticsTooltipError> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn i64(&mut self) -> Result<i64, PoliticsTooltipError> {
        Ok(i64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn bool(&mut self) -> Result<bool, PoliticsTooltipError> {
        match self.u32()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => invalid("boolean encoding invalid"),
        }
    }
    fn array32(&mut self) -> Result<[u8; 32], PoliticsTooltipError> {
        Ok(self.take(32)?.try_into().unwrap())
    }
    fn string(&mut self) -> Result<String, PoliticsTooltipError> {
        let count = bounded_count(self.u32()?, MAX_STRING_BYTES)?;
        if count == 0 {
            return invalid("wire string is empty");
        }
        String::from_utf8(self.take(count)?.to_vec())
            .map_err(|_| PoliticsTooltipError::Invalid("wire string not UTF-8".into()))
    }
    fn optional_string(&mut self) -> Result<Option<String>, PoliticsTooltipError> {
        let count = self.u32()?;
        if count == u32::MAX {
            return Ok(None);
        }
        let count = bounded_count(count, MAX_STRING_BYTES)?;
        if count == 0 {
            return invalid("optional string is empty");
        }
        Ok(Some(
            String::from_utf8(self.take(count)?.to_vec())
                .map_err(|_| PoliticsTooltipError::Invalid("optional string not UTF-8".into()))?,
        ))
    }
    fn optional_u64(&mut self) -> Result<Option<u64>, PoliticsTooltipError> {
        match self.u32()? {
            0 => Ok(None),
            1 => Ok(Some(self.u64()?)),
            _ => invalid("optional u64 encoding invalid"),
        }
    }
    fn reason_tree(&mut self) -> Result<Vec<ReasonNode>, PoliticsTooltipError> {
        let count = bounded_count(self.u32()?, MAX_TERMS_PER_ROW)?;
        let mut nodes = Vec::with_capacity(count);
        for _ in 0..count {
            nodes.push(ReasonNode {
                source_ordinal: self.u32()?,
                node_id: self.string()?,
                parent_node_id: self.optional_string()?,
                source_identity: self.string()?,
                result: self.bool()?,
                resolved_reason: self.string()?,
            });
        }
        Ok(nodes)
    }
    fn numeric_terms(&mut self) -> Result<Vec<NumericSourceTerm>, PoliticsTooltipError> {
        let count = bounded_count(self.u32()?, MAX_TERMS_PER_ROW)?;
        let mut terms = Vec::with_capacity(count);
        for _ in 0..count {
            terms.push(NumericSourceTerm {
                source_ordinal: self.u32()?,
                source_identity: self.string()?,
                evaluation_context: self.string()?,
                raw_value: self.i64()?,
                scale: self.u32()?,
            });
        }
        Ok(terms)
    }
    fn typed_value(&mut self) -> Result<Option<PoliticsTypedValue>, PoliticsTooltipError> {
        Ok(match self.u32()? {
            0 => None,
            1 => Some(PoliticsTypedValue::TriggerEvaluation {
                result: self.bool()?,
                reason_tree: self.reason_tree()?,
            }),
            2 => Some(PoliticsTypedValue::EffectiveCost {
                action: match self.u32()? {
                    0 => CostAction::Hire,
                    1 => CostAction::Remove,
                    _ => return invalid("cost action invalid"),
                },
                resource_token: self.string()?,
                raw_value: self.i64()?,
                scale: self.u32()?,
                affordable: self.bool()?,
            }),
            3 => Some(PoliticsTypedValue::ConditionalModifierAggregate {
                modifier_token: self.string()?,
                raw_total: self.i64()?,
                scale: self.u32()?,
                terms: self.numeric_terms()?,
            }),
            4 => Some(PoliticsTypedValue::AdvisorSlotState {
                capacity: self.u32()?,
                occupied: self.u32()?,
                vacancy: self.bool()?,
                eligible: self.bool()?,
                disabled_reason_tree: self.reason_tree()?,
            }),
            5 => Some(PoliticsTypedValue::TimedDynamicModifier {
                modifier_token: self.string()?,
                raw_value: self.i64()?,
                scale: self.u32()?,
                source_token: self.string()?,
                applied_hour: self.optional_u64()?,
                expiry_hour: self.optional_u64()?,
            }),
            6 => Some(PoliticsTypedValue::PartyPopularityChange {
                party_token: self.string()?,
                raw_change: self.i64()?,
                scale: self.u32()?,
                sources: self.numeric_terms()?,
            }),
            _ => return invalid("typed value tag invalid"),
        })
    }
    fn done(&self) -> bool {
        self.offset == self.bytes.len()
    }
}

#[cfg(windows)]
pub mod a729_hook;

#[cfg(test)]
mod tests {
    use super::*;

    fn reason_tree() -> Vec<ReasonNode> {
        vec![
            ReasonNode {
                source_ordinal: 10,
                node_id: "root".into(),
                parent_node_id: None,
                source_identity: "CIdea.allowed_to_remove".into(),
                result: false,
                resolved_reason: "Cannot replace this idea".into(),
            },
            ReasonNode {
                source_ordinal: 20,
                node_id: "root/0".into(),
                parent_node_id: Some("root".into()),
                source_identity: "has_war=no".into(),
                result: false,
                resolved_reason: "At peace".into(),
            },
        ]
    }

    fn term(ordinal: u32, source: &str, value: i64) -> NumericSourceTerm {
        NumericSourceTerm {
            source_ordinal: ordinal,
            source_identity: source.into(),
            evaluation_context: "country=GER".into(),
            raw_value: value,
            scale: 1000,
        }
    }

    fn row(
        kind: PoliticsLineKind,
        value: PoliticsTypedValue,
        target: &str,
    ) -> PoliticsTooltipInput {
        PoliticsTooltipInput {
            frame_hour: 900,
            country_tag: "GER".into(),
            target_kind: "idea".into(),
            target_token_or_slot: target.into(),
            line_kind: kind,
            availability_state: AvailabilityState::Measured,
            typed_value: Some(value),
            source_identity: "politicalitems.cpp".into(),
            evaluation_context: "country=GER;pov=GER".into(),
        }
    }

    fn exact_frame() -> PoliticsTooltipFrame {
        let rows = vec![
            row(
                PoliticsLineKind::EvaluatedTriggerResultAndReasonTree,
                PoliticsTypedValue::TriggerEvaluation {
                    result: false,
                    reason_tree: reason_tree(),
                },
                "idea:a",
            ),
            row(
                PoliticsLineKind::EffectiveHireOrRemovalCost,
                PoliticsTypedValue::EffectiveCost {
                    action: CostAction::Remove,
                    resource_token: "political_power".into(),
                    raw_value: 150_000,
                    scale: 1000,
                    affordable: true,
                },
                "idea:b",
            ),
            row(
                PoliticsLineKind::AggregatedConditionalModifier,
                PoliticsTypedValue::ConditionalModifierAggregate {
                    modifier_token: "political_power_gain".into(),
                    raw_total: 25,
                    scale: 1000,
                    terms: vec![term(10, "base", 10), term(20, "conditional", 15)],
                },
                "idea:c",
            ),
            row(
                PoliticsLineKind::AdvisorSlotCapacityVacancyEligibilityAndDisabledReason,
                PoliticsTypedValue::AdvisorSlotState {
                    capacity: 3,
                    occupied: 2,
                    vacancy: true,
                    eligible: false,
                    disabled_reason_tree: reason_tree(),
                },
                "slot:political_advisor",
            ),
            row(
                PoliticsLineKind::TimedDynamicModifierValueSourceAndExpiry,
                PoliticsTypedValue::TimedDynamicModifier {
                    modifier_token: "war_support_modifier".into(),
                    raw_value: -50,
                    scale: 1000,
                    source_token: "event:12".into(),
                    applied_hour: Some(800),
                    expiry_hour: Some(1200),
                },
                "modifier:war_support_modifier",
            ),
            row(
                PoliticsLineKind::DisplayedPartyPopularityChangeSource,
                PoliticsTypedValue::PartyPopularityChange {
                    party_token: "fascism".into(),
                    raw_change: 30,
                    scale: 1000,
                    sources: vec![term(10, "drift", 10), term(20, "advisor", 20)],
                },
                "party:fascism",
            ),
        ];
        PoliticsTooltipFrame {
            elapsed_wall_ms: 7,
            status_flags: 0,
            frame_hour: 900,
            producer_manifest_sha256: [0xA5; 32],
            completeness_record_identity: "a729.politics.rows[900]".into(),
            expected_row_count: rows.len() as u32,
            rows,
        }
    }

    #[test]
    fn all_six_typed_families_round_trip() {
        let frame = exact_frame();
        assert_eq!(
            decode_payload(&encode_payload(&frame).unwrap()).unwrap(),
            frame
        );
    }

    #[test]
    fn authenticated_measured_empty_frame_round_trips() {
        let mut frame = exact_frame();
        frame.expected_row_count = 0;
        frame.rows.clear();
        assert_eq!(
            decode_payload(&encode_payload(&frame).unwrap()).unwrap(),
            frame
        );
    }

    #[test]
    fn completeness_key_and_variant_mismatch_fail_closed() {
        let mut frame = exact_frame();
        frame.expected_row_count += 1;
        assert!(validate_frame(&frame).is_err());
        let mut frame = exact_frame();
        frame.rows.push(frame.rows[0].clone());
        frame.expected_row_count += 1;
        assert!(validate_frame(&frame).is_err());
        let mut frame = exact_frame();
        frame.rows[0].line_kind = PoliticsLineKind::EffectiveHireOrRemovalCost;
        assert!(validate_frame(&frame).is_err());
    }

    #[test]
    fn availability_reason_tree_and_aggregate_invariants_fail_closed() {
        let mut frame = exact_frame();
        frame.rows[0].availability_state = AvailabilityState::NotApplicable;
        assert!(validate_frame(&frame).is_err());
        let mut frame = exact_frame();
        if let Some(PoliticsTypedValue::TriggerEvaluation { reason_tree, .. }) =
            &mut frame.rows[0].typed_value
        {
            reason_tree[1].parent_node_id = Some("missing".into());
        }
        assert!(validate_frame(&frame).is_err());
        let mut frame = exact_frame();
        if let Some(PoliticsTypedValue::ConditionalModifierAggregate { raw_total, .. }) =
            &mut frame.rows[2].typed_value
        {
            *raw_total += 1;
        }
        assert!(validate_frame(&frame).is_err());
    }

    struct Fake(bool);
    impl ExactPoliticsTooltipSource for Fake {
        fn is_exact_a729_politics_source(&self) -> bool {
            self.0
        }
        fn producer_manifest_sha256(&self) -> [u8; 32] {
            [0xA5; 32]
        }
        fn completeness_record_identity(&self, _: u64) -> Result<String, String> {
            Ok("frame".into())
        }
        fn expected_row_count(&self, _: u64) -> Result<u32, String> {
            Ok(6)
        }
        fn collect_frame(&self, _: u64) -> Result<Vec<PoliticsTooltipInput>, String> {
            Ok(exact_frame().rows)
        }
    }

    #[test]
    fn collector_seam_rejects_unauthenticated_source() {
        assert_eq!(
            collect_and_encode(&Fake(false), 900, 0, 0),
            Err(PoliticsTooltipError::UnauthenticatedSource)
        );
        assert!(collect_and_encode(&Fake(true), 900, 0, 0).is_ok());
    }
}
