use std::collections::BTreeSet;

#[cfg(windows)]
pub mod a729_hook;
pub mod production_carrier;
pub mod world_lifecycle;

pub const MAGIC: u32 = u32::from_le_bytes(*b"ABL1");
pub const SCHEMA_VERSION: u32 = 2;
pub const AUTHORITY_EXACT_A729_ABILITY_CONTROLLER: u32 = 0xA729_0036;
pub const EXACT_A729_AUTHORITY_BOUND: bool = true;
pub const LIVE_TRANSITION_PROMOTION_BOUND: bool = true;
pub const PORTABLE_TEXT_JOIN_BOUND: bool = false;
pub const COMPACT_DOMAIN_SCHEDULED: bool = true;
pub const HEADER_LEN: usize = 40;
pub const COMMAND_POWER_FIXED_SCALE: i64 = 100_000;
const MAX_ROWS: usize = 4096;
const MAX_STRING_BYTES: usize = 1 << 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectedContextKind {
    ArmyLeader,
    FieldMarshal,
    ArmyGroup,
    Army,
    Other(u32),
}

/// The escape's flag bit and the payload domain beside it, named once.
///
/// `validate_frame` refuses a payload that reaches into the flag, which is
/// what keeps `wire` injective over the frames this lane encodes (lane TV,
/// row R11). That guard held the flag as a second literal, so moving the
/// escape would have left the guard admitting a payload that then aliases
/// onto another value's code -- the lane's own defect class, in the guard that
/// closes it.
const SELECTED_CONTEXT_ESCAPE_FLAG: u32 = 0x8000_0000;
const SELECTED_CONTEXT_ESCAPE_PAYLOAD: u32 = !SELECTED_CONTEXT_ESCAPE_FLAG;

compact_contract::wire_coded_codes! {
    no_discriminant SelectedContextKind: u32,
    code = pub(crate) wire,
    Ok = from_wire(v) -> Result<Self, AbilityError> {
        invalid("selected context kind invalid")
    };

    ArmyLeader = 1,
    FieldMarshal = 2,
    ArmyGroup = 3,
    Army = 4,
    ; escape masked Other, flag = SELECTED_CONTEXT_ESCAPE_FLAG,
      payload = SELECTED_CONTEXT_ESCAPE_PAYLOAD
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimerValue {
    NotStarted,
    Indefinite,
    Hours(u32),
}

compact_contract::wire_coded_codes! {
    no_discriminant TimerValue: u32,
    code = tag,
    Ok = from_tag(tag) -> Result<Self, AbilityError> {
        invalid("timer sentinel invalid")
    };

    NotStarted = 0,
    Indefinite = 1,
    ; escape reserved 2 { Hours with 0 }
}

impl TimerValue {
    fn wire(self) -> (u32, u32) {
        (
            self.tag(),
            match self {
                Self::Hours(hours) => hours,
                _ => 0,
            },
        )
    }

    fn from_wire(tag: u32, value: u32) -> Result<Self, AbilityError> {
        match (Self::from_tag(tag)?, value) {
            (Self::Hours(_), hours) => Ok(Self::Hours(hours)),
            (sentinel, 0) => Ok(sentinel),
            _ => invalid("timer sentinel invalid"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AbilityRow {
    pub ability_id: String,
    pub available: bool,
    pub active: bool,
    pub activation_hour: Option<u64>,
    pub cooldown_total: TimerValue,
    pub cooldown_remaining: TimerValue,
    pub cost_command_power_fixed: Option<i64>,
    pub target_kind: Option<String>,
    pub target_id: Option<String>,
    pub duration_total: TimerValue,
    pub duration_remaining: TimerValue,
    pub display_order: u32,
    pub icon: String,
    pub tooltip: Option<String>,
    pub disabled_reason: Option<String>,
    pub affected_division_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AbilityFrame {
    pub elapsed_wall_ms: u64,
    pub status_flags: u32,
    pub frame_hour: u64,
    pub country_tag: String,
    pub selected_context_kind: SelectedContextKind,
    pub selected_context_id: String,
    pub source_record_id: String,
    pub complete: bool,
    pub rows: Vec<AbilityRow>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AbilityError {
    ActivationDisabled,
    UnauthenticatedSource,
    Source(String),
    Invalid(String),
    Truncated,
    TrailingBytes,
}

mod sealed {
    pub trait Sealed {}
}

pub trait ExactA729AbilityControllerSource: sealed::Sealed {
    fn is_exact_a729_ability_controller(&self) -> bool;
    fn collect_frame(&self, frame_hour: u64) -> Result<AbilityFrame, String>;
}

pub fn collect_and_encode<S: ExactA729AbilityControllerSource>(
    source: &S,
    frame_hour: u64,
) -> Result<Vec<u8>, AbilityError> {
    if !EXACT_A729_AUTHORITY_BOUND || !LIVE_TRANSITION_PROMOTION_BOUND || !COMPACT_DOMAIN_SCHEDULED
    {
        return Err(AbilityError::ActivationDisabled);
    }
    if !source.is_exact_a729_ability_controller() {
        return Err(AbilityError::UnauthenticatedSource);
    }
    let frame = source
        .collect_frame(frame_hour)
        .map_err(AbilityError::Source)?;
    if frame.frame_hour != frame_hour {
        return invalid("source returned the wrong frame hour");
    }
    encode_payload(&frame)
}

pub fn validate_frame(frame: &AbilityFrame) -> Result<(), AbilityError> {
    validate_nonempty(&frame.country_tag, "country_tag")?;
    validate_nonempty(&frame.selected_context_id, "selected_context_id")?;
    validate_nonempty(&frame.source_record_id, "source_record_id")?;
    if matches!(
        frame.selected_context_kind,
        SelectedContextKind::Other(v) if v & SELECTED_CONTEXT_ESCAPE_FLAG != 0
    )
    {
        return invalid("selected context kind exceeds exact wire range");
    }
    if !frame.complete {
        return invalid("A36 frame is not complete");
    }
    if frame.rows.len() > MAX_ROWS {
        return invalid("ability row count exceeds bound");
    }
    let mut ids = BTreeSet::new();
    for (ordinal, row) in frame.rows.iter().enumerate() {
        validate_nonempty(&row.ability_id, "ability_id")?;
        validate_nonempty(&row.icon, "icon")?;
        if let Some(tooltip) = &row.tooltip {
            validate_nonempty(tooltip, "tooltip")?;
        }
        if !ids.insert(row.ability_id.as_str()) {
            return invalid("duplicate ability_id");
        }
        if row.display_order as usize != ordinal {
            return invalid("display_order is not contiguous source order");
        }
        if row.available && row.disabled_reason.is_some() {
            return invalid("available row cannot carry a disabled_reason");
        }
        if row.active != row.activation_hour.is_some() {
            return invalid("activation_hour must exist exactly when active");
        }
        if row.active && matches!(row.duration_remaining, TimerValue::NotStarted) {
            return invalid("active ability has not-started remaining duration");
        }
        if !row.active && !matches!(row.duration_remaining, TimerValue::NotStarted) {
            return invalid("inactive ability has running remaining duration");
        }
        match (&row.target_kind, &row.target_id) {
            (None, None) | (Some(_), Some(_)) => {}
            _ => return invalid("target kind/id must be both present or both absent"),
        }
        if let Some(v) = &row.target_kind {
            validate_nonempty(v, "target_kind")?;
        }
        if let Some(v) = &row.target_id {
            validate_nonempty(v, "target_id")?;
        }
        if let Some(v) = &row.disabled_reason {
            validate_nonempty(v, "disabled_reason")?;
        }
        let mut divisions = BTreeSet::new();
        for id in &row.affected_division_ids {
            validate_nonempty(id, "affected_division_id")?;
            if !divisions.insert(id.as_str()) {
                return invalid("duplicate affected division for ability");
            }
        }
    }
    Ok(())
}

pub fn encode_payload(frame: &AbilityFrame) -> Result<Vec<u8>, AbilityError> {
    validate_frame(frame)?;
    let mut out = Vec::new();
    put_u32(&mut out, MAGIC);
    put_u32(&mut out, SCHEMA_VERSION);
    put_u32(&mut out, AUTHORITY_EXACT_A729_ABILITY_CONTROLLER);
    put_u32(&mut out, frame.status_flags);
    put_u64(&mut out, frame.elapsed_wall_ms);
    put_u64(&mut out, frame.frame_hour);
    put_u32(&mut out, frame.rows.len() as u32);
    put_u32(&mut out, HEADER_LEN as u32);
    put_string(&mut out, &frame.country_tag)?;
    put_u32(&mut out, frame.selected_context_kind.wire());
    put_string(&mut out, &frame.selected_context_id)?;
    put_string(&mut out, &frame.source_record_id)?;
    put_u32(&mut out, u32::from(frame.complete));
    for row in &frame.rows {
        put_string(&mut out, &row.ability_id)?;
        put_u32(&mut out, u32::from(row.available));
        put_u32(&mut out, u32::from(row.active));
        put_option_u64(&mut out, row.activation_hour);
        put_timer(&mut out, row.cooldown_total);
        put_timer(&mut out, row.cooldown_remaining);
        put_option_i64(&mut out, row.cost_command_power_fixed);
        put_optional_string(&mut out, row.target_kind.as_deref())?;
        put_optional_string(&mut out, row.target_id.as_deref())?;
        put_timer(&mut out, row.duration_total);
        put_timer(&mut out, row.duration_remaining);
        put_u32(&mut out, row.display_order);
        put_string(&mut out, &row.icon)?;
        put_optional_string(&mut out, row.tooltip.as_deref())?;
        put_optional_string(&mut out, row.disabled_reason.as_deref())?;
        put_u32(&mut out, row.affected_division_ids.len() as u32);
        for id in &row.affected_division_ids {
            put_string(&mut out, id)?;
        }
    }
    Ok(out)
}

pub fn decode_payload(payload: &[u8]) -> Result<AbilityFrame, AbilityError> {
    let mut r = Reader::new(payload);
    if r.u32()? != MAGIC || r.u32()? != SCHEMA_VERSION {
        return invalid("magic/schema mismatch");
    }
    if r.u32()? != AUTHORITY_EXACT_A729_ABILITY_CONTROLLER {
        return Err(AbilityError::UnauthenticatedSource);
    }
    let status_flags = r.u32()?;
    let elapsed_wall_ms = r.u64()?;
    let frame_hour = r.u64()?;
    let count = r.u32()? as usize;
    if r.u32()? as usize != HEADER_LEN || count > MAX_ROWS {
        return invalid("header/count invalid");
    }
    let country_tag = r.string()?;
    let selected_context_kind = SelectedContextKind::from_wire(r.u32()?)?;
    let selected_context_id = r.string()?;
    let source_record_id = r.string()?;
    let complete = r.boolean()?;
    let mut rows = Vec::with_capacity(count);
    for _ in 0..count {
        let ability_id = r.string()?;
        let available = r.boolean()?;
        let active = r.boolean()?;
        let activation_hour = r.option_u64()?;
        let cooldown_total = r.timer()?;
        let cooldown_remaining = r.timer()?;
        let cost_command_power_fixed = r.option_i64()?;
        let target_kind = r.optional_string()?;
        let target_id = r.optional_string()?;
        let duration_total = r.timer()?;
        let duration_remaining = r.timer()?;
        let display_order = r.u32()?;
        let icon = r.string()?;
        let tooltip = r.optional_string()?;
        let disabled_reason = r.optional_string()?;
        let division_count = r.u32()? as usize;
        if division_count > MAX_ROWS {
            return invalid("division count exceeds bound");
        }
        let mut affected_division_ids = Vec::with_capacity(division_count);
        for _ in 0..division_count {
            affected_division_ids.push(r.string()?);
        }
        rows.push(AbilityRow {
            ability_id,
            available,
            active,
            activation_hour,
            cooldown_total,
            cooldown_remaining,
            cost_command_power_fixed,
            target_kind,
            target_id,
            duration_total,
            duration_remaining,
            display_order,
            icon,
            tooltip,
            disabled_reason,
            affected_division_ids,
        });
    }
    if !r.done() {
        return Err(AbilityError::TrailingBytes);
    }
    let frame = AbilityFrame {
        elapsed_wall_ms,
        status_flags,
        frame_hour,
        country_tag,
        selected_context_kind,
        selected_context_id,
        source_record_id,
        complete,
        rows,
    };
    validate_frame(&frame)?;
    Ok(frame)
}

fn invalid<T>(message: &str) -> Result<T, AbilityError> {
    Err(AbilityError::Invalid(message.into()))
}
fn validate_nonempty(value: &str, field: &str) -> Result<(), AbilityError> {
    if value.trim().is_empty() || value.len() > MAX_STRING_BYTES {
        invalid(&format!("{field} empty or oversized"))
    } else {
        Ok(())
    }
}
pub(crate) fn put_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}
pub(crate) fn put_u64(out: &mut Vec<u8>, v: u64) {
    out.extend_from_slice(&v.to_le_bytes());
}
fn put_i64(out: &mut Vec<u8>, v: i64) {
    out.extend_from_slice(&v.to_le_bytes());
}
fn put_option_i64(out: &mut Vec<u8>, v: Option<i64>) {
    put_i64(out, v.unwrap_or(i64::MIN));
}
fn put_option_u64(out: &mut Vec<u8>, v: Option<u64>) {
    put_u64(out, v.unwrap_or(u64::MAX));
}
fn put_timer(out: &mut Vec<u8>, v: TimerValue) {
    let (tag, value) = v.wire();
    put_u32(out, tag);
    put_u32(out, value);
}
pub(crate) fn put_string(out: &mut Vec<u8>, v: &str) -> Result<(), AbilityError> {
    validate_nonempty(v, "wire string")?;
    put_u32(out, v.len() as u32);
    out.extend_from_slice(v.as_bytes());
    Ok(())
}
fn put_optional_string(out: &mut Vec<u8>, v: Option<&str>) -> Result<(), AbilityError> {
    match v {
        None => put_u32(out, u32::MAX),
        Some(v) => put_string(out, v)?,
    };
    Ok(())
}

pub(crate) struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
}
impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8], AbilityError> {
        let end = self.offset.checked_add(n).ok_or(AbilityError::Truncated)?;
        let v = self
            .bytes
            .get(self.offset..end)
            .ok_or(AbilityError::Truncated)?;
        self.offset = end;
        Ok(v)
    }
    fn u32(&mut self) -> Result<u32, AbilityError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, AbilityError> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn i64(&mut self) -> Result<i64, AbilityError> {
        Ok(i64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn boolean(&mut self) -> Result<bool, AbilityError> {
        match self.u32()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => invalid("boolean invalid"),
        }
    }
    fn option_u64(&mut self) -> Result<Option<u64>, AbilityError> {
        let v = self.u64()?;
        Ok((v != u64::MAX).then_some(v))
    }
    fn option_i64(&mut self) -> Result<Option<i64>, AbilityError> {
        let v = self.i64()?;
        Ok((v != i64::MIN).then_some(v))
    }
    fn timer(&mut self) -> Result<TimerValue, AbilityError> {
        let tag = self.u32()?;
        let value = self.u32()?;
        TimerValue::from_wire(tag, value)
    }
    fn string(&mut self) -> Result<String, AbilityError> {
        let n = self.u32()? as usize;
        if n == 0 || n > MAX_STRING_BYTES {
            return invalid("wire string length invalid");
        }
        String::from_utf8(self.take(n)?.to_vec())
            .map_err(|_| AbilityError::Invalid("wire string not UTF-8".into()))
    }
    fn optional_string(&mut self) -> Result<Option<String>, AbilityError> {
        let n = self.u32()?;
        if n == u32::MAX {
            return Ok(None);
        }
        let n = n as usize;
        if n == 0 || n > MAX_STRING_BYTES {
            return invalid("optional string length invalid");
        }
        Ok(Some(String::from_utf8(self.take(n)?.to_vec()).map_err(
            |_| AbilityError::Invalid("optional string not UTF-8".into()),
        )?))
    }
    fn done(&self) -> bool {
        self.offset == self.bytes.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn exact() -> AbilityFrame {
        AbilityFrame {
            elapsed_wall_ms: 7,
            status_flags: 0,
            frame_hour: 900,
            country_tag: "GER".into(),
            selected_context_kind: SelectedContextKind::ArmyLeader,
            selected_context_id: "leader:42".into(),
            source_record_id: "a36:900:42".into(),
            complete: true,
            rows: vec![AbilityRow {
                ability_id: "force_attack".into(),
                available: false,
                active: false,
                activation_hour: None,
                cooldown_total: TimerValue::NotStarted,
                cooldown_remaining: TimerValue::NotStarted,
                cost_command_power_fixed: Some(22_000),
                target_kind: Some("army_leader".into()),
                target_id: Some("leader:42".into()),
                duration_total: TimerValue::Hours(168),
                duration_remaining: TimerValue::NotStarted,
                display_order: 0,
                icon: "GFX_ability_force_attack".into(),
                tooltip: Some("Force Attack".into()),
                disabled_reason: Some("Not enough command power".into()),
                affected_division_ids: vec![],
            }],
        }
    }
    #[test]
    fn round_trip_preserves_exact_sentinels() {
        let f = exact();
        let bytes = encode_payload(&f).unwrap();
        assert_eq!(u32::from_le_bytes(bytes[36..40].try_into().unwrap()), 40);
        assert_eq!(decode_payload(&bytes).unwrap(), f);
    }
    #[test]
    fn other_context_kind_round_trips_without_aliasing() {
        let mut f = exact();
        f.selected_context_kind = SelectedContextKind::Other(73);
        let bytes = encode_payload(&f).unwrap();
        assert_eq!(decode_payload(&bytes).unwrap(), f);
        let mut invalid = bytes;
        let context_kind_offset = 40 + 4 + 3;
        invalid[context_kind_offset..context_kind_offset + 4].copy_from_slice(&73u32.to_le_bytes());
        assert!(decode_payload(&invalid).is_err());
    }
    #[test]
    fn rejects_inferred_or_malformed_rows() {
        let mut f = exact();
        f.complete = false;
        assert!(validate_frame(&f).is_err());
        let mut f = exact();
        f.rows[0].display_order = 1;
        assert!(validate_frame(&f).is_err());
        let mut f = exact();
        f.rows[0].available = true;
        assert!(validate_frame(&f).is_err());
    }
    struct Fake(bool);
    impl sealed::Sealed for Fake {}
    impl ExactA729AbilityControllerSource for Fake {
        fn is_exact_a729_ability_controller(&self) -> bool {
            self.0
        }
        fn collect_frame(&self, _: u64) -> Result<AbilityFrame, String> {
            Ok(exact())
        }
    }
    #[test]
    fn source_authentication_is_fail_closed() {
        assert_eq!(
            collect_and_encode(&Fake(false), 900),
            Err(AbilityError::UnauthenticatedSource)
        );
        assert!(collect_and_encode(&Fake(true), 900).is_ok());
    }
}
