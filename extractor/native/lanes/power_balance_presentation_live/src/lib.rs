//! A37/XUI18 exact power-balance presentation carrier (reserved d74).
//!
//! The carrier is complete and address-independent. It deliberately has no
//! save/game-file inference path: an authenticated a729 controller hook must
//! implement [`ExactPowerBalancePresentationSource`] before d74 is scheduled.

use std::collections::BTreeSet;

pub mod a729_hook;
pub mod a729_decision_grid;

pub const DOMAIN_ID: u16 = 74;
pub const DOMAIN_NAME: &str = "power_balance_presentation_live";
pub const SCHEMA_VERSION: u32 = 2;
pub const AUTHORITY_EXACT_A729_POWER_BALANCE_CONTROLLER: u32 = 1;
/// Static exact-20 authority receipt for the observer ABI. This does not
/// activate live collection by itself.
pub const A729_CONTROLLER_EXACT20_AUTHORITY_BOUND: bool = true;
pub const A729_CONTROLLER_EXACT20_JOB_ID: &str =
    "20260720T020812Z-w3-a37-power-balance-5c5d63f5";
pub const A729_CONTROLLER_EXACT20_SHARD: &str = "shard-11";
pub const A729_CONTROLLER_EXACT20_MANIFEST_SHA256: [u8; 32] = [
    0xa5, 0x40, 0xaf, 0x1c, 0x03, 0x0d, 0x9e, 0xf2, 0xfb, 0xb1, 0xcd, 0x29, 0x06, 0x27, 0x08, 0x76,
    0x27, 0x77, 0x1a, 0x19, 0x6e, 0x7a, 0x60, 0xe4, 0x24, 0x1c, 0xb5, 0x28, 0x7b, 0x66, 0xe4, 0xe2,
];
/// Remains false until an authenticated same-transition live fixture proves
/// current-value sign, change sign, independent trending-side identity,
/// one-based frame normalization, and replay parity together. The portable
/// codec may be exercised directly while live collection remains disabled.
pub const A729_CONTROLLER_OBSERVER_ACTIVATED: bool = false;
pub const MAGIC: u32 = u32::from_le_bytes(*b"PBP2");
pub const HEADER_LEN: usize = 76;
pub const MAX_BALANCES: usize = 4_096;
pub const MAX_DECISIONS_PER_BALANCE: usize = 131_072;
pub const MAX_STRING_BYTES: usize = 65_536;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BarState {
    StableLeft,
    StableRight,
    MovingLeft,
    MovingRight,
}

impl BarState {
    fn code(self) -> u32 {
        match self {
            Self::StableLeft => 0,
            Self::StableRight => 1,
            Self::MovingLeft => 2,
            Self::MovingRight => 3,
        }
    }

    fn from_code(code: u32) -> Result<Self, CarrierError> {
        match code {
            0 => Ok(Self::StableLeft),
            1 => Ok(Self::StableRight),
            2 => Ok(Self::MovingLeft),
            3 => Ok(Self::MovingRight),
            _ => invalid("bar state code invalid"),
        }
    }

    fn moving(self) -> bool {
        matches!(self, Self::MovingLeft | Self::MovingRight)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeCadence {
    Daily,
    Weekly,
}

impl ChangeCadence {
    fn code(self) -> u32 {
        match self {
            Self::Daily => 0,
            Self::Weekly => 1,
        }
    }

    fn from_code(code: u32) -> Result<Self, CarrierError> {
        match code {
            0 => Ok(Self::Daily),
            1 => Ok(Self::Weekly),
            _ => invalid("change cadence code invalid"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecisionAvailability {
    Available,
    Unavailable,
    Active,
    OnCooldown,
}

impl DecisionAvailability {
    fn code(self) -> u32 {
        match self {
            Self::Available => 0,
            Self::Unavailable => 1,
            Self::Active => 2,
            Self::OnCooldown => 3,
        }
    }

    fn from_code(code: u32) -> Result<Self, CarrierError> {
        match code {
            0 => Ok(Self::Available),
            1 => Ok(Self::Unavailable),
            2 => Ok(Self::Active),
            3 => Ok(Self::OnCooldown),
            _ => invalid("decision availability code invalid"),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TrendInput {
    pub cadence: ChangeCadence,
    pub magnitude: f64,
    pub resolved_text_ref: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedTextRefs {
    pub balance_name: String,
    pub left_side_name: String,
    pub right_side_name: String,
    pub active_range_name: String,
    pub balance_tooltip: String,
    pub active_range_tooltip: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecisionInput {
    pub decision_id: String,
    pub source_record_id: String,
    pub availability: DecisionAvailability,
    pub resolved_name_text_ref: String,
    pub cost_text_ref: Option<String>,
    pub timer_text_ref: Option<String>,
    pub target_tag_ref: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PowerBalanceInput {
    pub source_record_id: String,
    pub country_tag: String,
    pub balance_id: String,
    /// Always zero. The a729 controller exposes only the first matching
    /// engine-ordered record for a country.
    pub display_ordinal: u32,
    pub active_side_id: String,
    /// Independent engine identity from selected_record+0x28. It may differ
    /// from `active_side_id` while the balance crosses toward the other side.
    pub trending_side_id: String,
    pub active_range_id: String,
    pub effective_left_icon: String,
    pub effective_right_icon: String,
    /// Country-politics launcher state. The a729 controller selects the first
    /// engine-ordered balance for the country; therefore this is true exactly
    /// for display ordinal zero.
    pub launcher_visible: bool,
    pub launcher_percentage_text_ref: Option<String>,
    pub launcher_levels_text_ref: Option<String>,
    pub launcher_icon_sprite: Option<String>,
    pub launcher_glow_visible: bool,
    pub bar_state: BarState,
    pub range_indicator_frame: u32,
    pub range_indicator_top_left: [i32; 2],
    pub trend: Option<TrendInput>,
    /// A25 reference for the complete resolved `GetPowerBalanceModDesc`
    /// aggregate. Empty rendered text is represented by an existing reference,
    /// never by omitting this field or fabricating modifier rows.
    pub power_balance_modifier_desc_text_ref: String,
    pub decisions_complete: bool,
    pub decisions: Vec<DecisionInput>,
    pub resolved_text_refs: ResolvedTextRefs,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PowerBalancePresentationFrame {
    pub elapsed_wall_ms: u64,
    pub status_flags: u32,
    pub frame_hour: u64,
    pub producer_manifest_sha256: [u8; 32],
    pub completeness_record_identity: String,
    pub expected_balance_count: u32,
    pub balances: Vec<PowerBalanceInput>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CarrierError {
    UnauthenticatedSource,
    Source(String),
    Invalid(String),
    Truncated,
    TrailingBytes,
}

pub trait ExactPowerBalancePresentationSource {
    fn is_exact_a729_power_balance_source(&self) -> bool;
    fn producer_manifest_sha256(&self) -> [u8; 32];
    fn completeness_record_identity(&self, frame_hour: u64) -> Result<String, String>;
    fn expected_balance_count(&self, frame_hour: u64) -> Result<u32, String>;
    fn collect_frame(&self, frame_hour: u64) -> Result<Vec<PowerBalanceInput>, String>;
}

/// Normalize the one-based engine `GFX_bop_indicator` frame to the portable
/// zero-based frame consumed by the replay renderer.
pub fn normalize_engine_range_indicator_frame(engine_frame: u32) -> Result<u32, CarrierError> {
    match engine_frame {
        1 => Ok(0),
        2 => Ok(1),
        _ => invalid("engine range indicator frame is not 1 or 2"),
    }
}

/// Raw leaves sampled by the future receipted controller observer. Keeping the
/// three signs/identities separate prevents the hook from reproducing the old
/// replay-side conflation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct A729ControllerStateObservation {
    pub current_value_raw: i64,
    pub change_raw: i64,
    pub left_side_id: String,
    pub right_side_id: String,
    pub trending_side_id: String,
    pub engine_range_indicator_frame: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NormalizedControllerState {
    pub active_side_id: String,
    pub trending_side_id: String,
    pub bar_state: BarState,
    pub range_indicator_frame: u32,
}

pub fn normalize_a729_controller_state(
    observation: &A729ControllerStateObservation,
) -> Result<NormalizedControllerState, CarrierError> {
    validate_nonempty(&observation.left_side_id, "left side id")?;
    validate_nonempty(&observation.right_side_id, "right side id")?;
    validate_nonempty(&observation.trending_side_id, "trending side id")?;
    if observation.left_side_id == observation.right_side_id
        || (observation.trending_side_id != observation.left_side_id
            && observation.trending_side_id != observation.right_side_id)
    {
        return invalid("controller side identities are inconsistent");
    }
    let active_side_id = if observation.current_value_raw < 0 {
        observation.left_side_id.clone()
    } else {
        observation.right_side_id.clone()
    };
    let bar_state = if observation.change_raw < 0 {
        BarState::MovingLeft
    } else if observation.change_raw > 0 {
        BarState::MovingRight
    } else if observation.current_value_raw < 0 {
        BarState::StableLeft
    } else {
        BarState::StableRight
    };
    Ok(NormalizedControllerState {
        active_side_id,
        trending_side_id: observation.trending_side_id.clone(),
        bar_state,
        range_indicator_frame: normalize_engine_range_indicator_frame(
            observation.engine_range_indicator_frame,
        )?,
    })
}

pub fn collect_and_encode<S: ExactPowerBalancePresentationSource>(
    source: &S,
    frame_hour: u64,
    elapsed_wall_ms: u64,
    status_flags: u32,
) -> Result<Vec<u8>, CarrierError> {
    let producer_manifest_sha256 = source.producer_manifest_sha256();
    if !A729_CONTROLLER_EXACT20_AUTHORITY_BOUND
        || !A729_CONTROLLER_OBSERVER_ACTIVATED
        || !source.is_exact_a729_power_balance_source()
        || producer_manifest_sha256 != A729_CONTROLLER_EXACT20_MANIFEST_SHA256
    {
        return Err(CarrierError::UnauthenticatedSource);
    }
    encode_payload(&PowerBalancePresentationFrame {
        elapsed_wall_ms,
        status_flags,
        frame_hour,
        producer_manifest_sha256,
        completeness_record_identity: source
            .completeness_record_identity(frame_hour)
            .map_err(CarrierError::Source)?,
        expected_balance_count: source
            .expected_balance_count(frame_hour)
            .map_err(CarrierError::Source)?,
        balances: source
            .collect_frame(frame_hour)
            .map_err(CarrierError::Source)?,
    })
}

pub fn validate_frame(frame: &PowerBalancePresentationFrame) -> Result<(), CarrierError> {
    if frame.producer_manifest_sha256 != A729_CONTROLLER_EXACT20_MANIFEST_SHA256 {
        return invalid("producer manifest hash does not match bound exact20 receipt");
    }
    validate_nonempty(
        &frame.completeness_record_identity,
        "completeness record identity",
    )?;
    if frame.balances.len() > MAX_BALANCES
        || frame.expected_balance_count as usize != frame.balances.len()
    {
        return invalid("balance completeness/count mismatch");
    }

    let mut keys = BTreeSet::new();
    let mut source_records = BTreeSet::new();
    let mut countries = BTreeSet::new();
    for balance in &frame.balances {
        validate_nonempty(&balance.source_record_id, "balance source record")?;
        if !source_records.insert(balance.source_record_id.as_str()) {
            return invalid("duplicate balance source record");
        }
        validate_country_tag(&balance.country_tag)?;
        if !countries.insert(balance.country_tag.as_str()) {
            return invalid("more than one controller-selected balance for country");
        }
        for (value, label) in [
            (&balance.balance_id, "balance id"),
            (&balance.active_side_id, "active side id"),
            (&balance.trending_side_id, "trending side id"),
            (&balance.active_range_id, "active range id"),
            (&balance.effective_left_icon, "effective left icon"),
            (&balance.effective_right_icon, "effective right icon"),
            (
                &balance.power_balance_modifier_desc_text_ref,
                "power balance modifier description text ref",
            ),
        ] {
            validate_nonempty(value, label)?;
        }
        if !keys.insert((balance.country_tag.as_str(), balance.balance_id.as_str())) {
            return invalid("duplicate country/balance identity");
        }
        if balance.display_ordinal != 0 || !balance.launcher_visible {
            return invalid("row is not the visible first matching balance");
        }
        validate_optional_required(
            balance.launcher_percentage_text_ref.as_deref(),
            "launcher percentage text ref",
        )?;
        validate_optional_required(
            balance.launcher_levels_text_ref.as_deref(),
            "launcher levels text ref",
        )?;
        validate_optional_required(
            balance.launcher_icon_sprite.as_deref(),
            "launcher icon sprite",
        )?;
        if balance.range_indicator_frame > 1 {
            return invalid("range indicator frame exceeds two-frame sprite");
        }
        match (&balance.trend, balance.bar_state.moving()) {
            (None, false) => {}
            (Some(trend), true)
                if trend.magnitude.is_finite()
                    && trend.magnitude > 0.0
                    && !trend.resolved_text_ref.trim().is_empty() => {}
            _ => return invalid("trend and stable/moving bar state disagree"),
        }
        if !balance.decisions_complete {
            return invalid("decision list is not complete");
        }
        if balance.decisions.len() > MAX_DECISIONS_PER_BALANCE {
            return invalid("decision count exceeds bound");
        }
        let mut decision_ids = BTreeSet::new();
        let mut decision_records = BTreeSet::new();
        for decision in &balance.decisions {
            validate_nonempty(&decision.decision_id, "decision id")?;
            validate_nonempty(&decision.source_record_id, "decision source record")?;
            validate_nonempty(
                &decision.resolved_name_text_ref,
                "decision resolved-name text ref",
            )?;
            validate_optional_nonempty(decision.cost_text_ref.as_deref(), "cost text ref")?;
            validate_optional_nonempty(decision.timer_text_ref.as_deref(), "timer text ref")?;
            validate_optional_nonempty(decision.target_tag_ref.as_deref(), "target tag ref")?;
            if !decision_ids.insert(decision.decision_id.as_str())
                || !decision_records.insert(decision.source_record_id.as_str())
            {
                return invalid("duplicate decision identity or source record");
            }
        }
        validate_text_refs(&balance.resolved_text_refs)?;
    }
    Ok(())
}

pub fn encode_payload(frame: &PowerBalancePresentationFrame) -> Result<Vec<u8>, CarrierError> {
    validate_frame(frame)?;
    let mut out = Vec::new();
    put_u32(&mut out, MAGIC);
    put_u32(&mut out, SCHEMA_VERSION);
    put_u32(&mut out, AUTHORITY_EXACT_A729_POWER_BALANCE_CONTROLLER);
    put_u32(&mut out, frame.status_flags);
    put_u64(&mut out, frame.elapsed_wall_ms);
    put_u64(&mut out, frame.frame_hour);
    out.extend_from_slice(&frame.producer_manifest_sha256);
    put_u32(&mut out, frame.expected_balance_count);
    put_u32(&mut out, frame.balances.len() as u32);
    put_u32(&mut out, HEADER_LEN as u32);
    put_string(&mut out, &frame.completeness_record_identity)?;
    for balance in &frame.balances {
        put_string(&mut out, &balance.source_record_id)?;
        put_string(&mut out, &balance.country_tag)?;
        put_string(&mut out, &balance.balance_id)?;
        put_u32(&mut out, balance.display_ordinal);
        put_string(&mut out, &balance.active_side_id)?;
        put_string(&mut out, &balance.trending_side_id)?;
        put_string(&mut out, &balance.active_range_id)?;
        put_string(&mut out, &balance.effective_left_icon)?;
        put_string(&mut out, &balance.effective_right_icon)?;
        put_u32(&mut out, u32::from(balance.launcher_visible));
        put_optional_string(&mut out, balance.launcher_percentage_text_ref.as_deref())?;
        put_optional_string(&mut out, balance.launcher_levels_text_ref.as_deref())?;
        put_optional_string(&mut out, balance.launcher_icon_sprite.as_deref())?;
        put_u32(&mut out, u32::from(balance.launcher_glow_visible));
        put_u32(&mut out, balance.bar_state.code());
        put_u32(&mut out, balance.range_indicator_frame);
        put_i32(&mut out, balance.range_indicator_top_left[0]);
        put_i32(&mut out, balance.range_indicator_top_left[1]);
        match &balance.trend {
            Some(trend) => {
                put_u32(&mut out, 1);
                put_u32(&mut out, trend.cadence.code());
                put_f64(&mut out, trend.magnitude);
                put_string(&mut out, &trend.resolved_text_ref)?;
            }
            None => put_u32(&mut out, 0),
        }
        put_string(
            &mut out,
            &balance.power_balance_modifier_desc_text_ref,
        )?;
        put_u32(&mut out, u32::from(balance.decisions_complete));
        put_u32(&mut out, balance.decisions.len() as u32);
        put_text_refs(&mut out, &balance.resolved_text_refs)?;
        for decision in &balance.decisions {
            put_string(&mut out, &decision.decision_id)?;
            put_string(&mut out, &decision.source_record_id)?;
            put_u32(&mut out, decision.availability.code());
            put_string(&mut out, &decision.resolved_name_text_ref)?;
            put_optional_string(&mut out, decision.cost_text_ref.as_deref())?;
            put_optional_string(&mut out, decision.timer_text_ref.as_deref())?;
            put_optional_string(&mut out, decision.target_tag_ref.as_deref())?;
        }
    }
    Ok(out)
}

pub fn decode_payload(payload: &[u8]) -> Result<PowerBalancePresentationFrame, CarrierError> {
    let mut reader = Reader::new(payload);
    if reader.u32()? != MAGIC || reader.u32()? != SCHEMA_VERSION {
        return invalid("magic/schema mismatch");
    }
    if reader.u32()? != AUTHORITY_EXACT_A729_POWER_BALANCE_CONTROLLER {
        return Err(CarrierError::UnauthenticatedSource);
    }
    let status_flags = reader.u32()?;
    let elapsed_wall_ms = reader.u64()?;
    let frame_hour = reader.u64()?;
    let producer_manifest_sha256 = reader.array32()?;
    let expected_balance_count = reader.u32()?;
    let balance_count = reader.u32()? as usize;
    if reader.u32()? as usize != HEADER_LEN || balance_count > MAX_BALANCES {
        return invalid("header/count invalid");
    }
    let completeness_record_identity = reader.string()?;
    let mut balances = Vec::with_capacity(balance_count);
    for _ in 0..balance_count {
        let source_record_id = reader.string()?;
        let country_tag = reader.string()?;
        let balance_id = reader.string()?;
        let display_ordinal = reader.u32()?;
        let active_side_id = reader.string()?;
        let trending_side_id = reader.string()?;
        let active_range_id = reader.string()?;
        let effective_left_icon = reader.string()?;
        let effective_right_icon = reader.string()?;
        let launcher_visible = reader.bool()?;
        let launcher_percentage_text_ref = reader.optional_string()?;
        let launcher_levels_text_ref = reader.optional_string()?;
        let launcher_icon_sprite = reader.optional_string()?;
        let launcher_glow_visible = reader.bool()?;
        let bar_state = BarState::from_code(reader.u32()?)?;
        let range_indicator_frame = reader.u32()?;
        let range_indicator_top_left = [reader.i32()?, reader.i32()?];
        let trend = match reader.u32()? {
            0 => None,
            1 => Some(TrendInput {
                cadence: ChangeCadence::from_code(reader.u32()?)?,
                magnitude: reader.f64()?,
                resolved_text_ref: reader.string()?,
            }),
            _ => return invalid("trend presence code invalid"),
        };
        let power_balance_modifier_desc_text_ref = reader.string()?;
        let decisions_complete = reader.bool()?;
        let decision_count = reader.u32()? as usize;
        if decision_count > MAX_DECISIONS_PER_BALANCE {
            return invalid("decision count exceeds bound");
        }
        let resolved_text_refs = reader.text_refs()?;
        let mut decisions = Vec::with_capacity(decision_count);
        for _ in 0..decision_count {
            decisions.push(DecisionInput {
                decision_id: reader.string()?,
                source_record_id: reader.string()?,
                availability: DecisionAvailability::from_code(reader.u32()?)?,
                resolved_name_text_ref: reader.string()?,
                cost_text_ref: reader.optional_string()?,
                timer_text_ref: reader.optional_string()?,
                target_tag_ref: reader.optional_string()?,
            });
        }
        balances.push(PowerBalanceInput {
            source_record_id,
            country_tag,
            balance_id,
            display_ordinal,
            active_side_id,
            trending_side_id,
            active_range_id,
            effective_left_icon,
            effective_right_icon,
            launcher_visible,
            launcher_percentage_text_ref,
            launcher_levels_text_ref,
            launcher_icon_sprite,
            launcher_glow_visible,
            bar_state,
            range_indicator_frame,
            range_indicator_top_left,
            trend,
            power_balance_modifier_desc_text_ref,
            decisions_complete,
            decisions,
            resolved_text_refs,
        });
    }
    if !reader.done() {
        return Err(CarrierError::TrailingBytes);
    }
    let frame = PowerBalancePresentationFrame {
        elapsed_wall_ms,
        status_flags,
        frame_hour,
        producer_manifest_sha256,
        completeness_record_identity,
        expected_balance_count,
        balances,
    };
    validate_frame(&frame)?;
    Ok(frame)
}

fn validate_country_tag(tag: &str) -> Result<(), CarrierError> {
    if tag.len() == 3 && tag.bytes().all(|byte| byte.is_ascii_uppercase()) && tag != "---" {
        Ok(())
    } else {
        invalid("country tag is not a stable uppercase tag")
    }
}

fn validate_text_refs(refs: &ResolvedTextRefs) -> Result<(), CarrierError> {
    for (value, label) in [
        (&refs.balance_name, "balance name text ref"),
        (&refs.left_side_name, "left side name text ref"),
        (&refs.right_side_name, "right side name text ref"),
        (&refs.active_range_name, "active range name text ref"),
        (&refs.balance_tooltip, "balance tooltip text ref"),
        (&refs.active_range_tooltip, "active range tooltip text ref"),
    ] {
        validate_nonempty(value, label)?;
    }
    Ok(())
}

fn validate_nonempty(value: &str, label: &str) -> Result<(), CarrierError> {
    if value.trim().is_empty()
        || value.len() > MAX_STRING_BYTES
        || value.chars().any(char::is_control)
    {
        invalid(format!(
            "{label} is empty, exceeds bound, or contains a control character"
        ))
    } else {
        Ok(())
    }
}

fn validate_optional_nonempty(value: Option<&str>, label: &str) -> Result<(), CarrierError> {
    match value {
        Some(value) => validate_nonempty(value, label),
        None => Ok(()),
    }
}

fn validate_optional_required(value: Option<&str>, label: &str) -> Result<(), CarrierError> {
    value
        .ok_or_else(|| CarrierError::Invalid(format!("{label} is absent")))
        .and_then(|value| validate_nonempty(value, label))
}

fn invalid<T>(message: impl Into<String>) -> Result<T, CarrierError> {
    Err(CarrierError::Invalid(message.into()))
}

fn put_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn put_i32(out: &mut Vec<u8>, value: i32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn put_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn put_f64(out: &mut Vec<u8>, value: f64) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn put_string(out: &mut Vec<u8>, value: &str) -> Result<(), CarrierError> {
    if value.len() > MAX_STRING_BYTES {
        return invalid("string exceeds bound");
    }
    put_u32(out, value.len() as u32);
    out.extend_from_slice(value.as_bytes());
    Ok(())
}

fn put_optional_string(out: &mut Vec<u8>, value: Option<&str>) -> Result<(), CarrierError> {
    match value {
        Some(value) => {
            put_u32(out, 1);
            put_string(out, value)
        }
        None => {
            put_u32(out, 0);
            Ok(())
        }
    }
}

fn put_text_refs(out: &mut Vec<u8>, refs: &ResolvedTextRefs) -> Result<(), CarrierError> {
    put_string(out, &refs.balance_name)?;
    put_string(out, &refs.left_side_name)?;
    put_string(out, &refs.right_side_name)?;
    put_string(out, &refs.active_range_name)?;
    put_string(out, &refs.balance_tooltip)?;
    put_string(out, &refs.active_range_tooltip)
}

struct Reader<'a> {
    data: &'a [u8],
    cursor: usize,
}

impl<'a> Reader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, cursor: 0 }
    }

    fn take(&mut self, len: usize) -> Result<&'a [u8], CarrierError> {
        let end = self
            .cursor
            .checked_add(len)
            .ok_or(CarrierError::Truncated)?;
        let bytes = self
            .data
            .get(self.cursor..end)
            .ok_or(CarrierError::Truncated)?;
        self.cursor = end;
        Ok(bytes)
    }

    fn u32(&mut self) -> Result<u32, CarrierError> {
        Ok(u32::from_le_bytes(
            self.take(4)?.try_into().expect("four-byte slice"),
        ))
    }

    fn i32(&mut self) -> Result<i32, CarrierError> {
        Ok(i32::from_le_bytes(
            self.take(4)?.try_into().expect("four-byte slice"),
        ))
    }

    fn u64(&mut self) -> Result<u64, CarrierError> {
        Ok(u64::from_le_bytes(
            self.take(8)?.try_into().expect("eight-byte slice"),
        ))
    }

    fn f64(&mut self) -> Result<f64, CarrierError> {
        Ok(f64::from_le_bytes(
            self.take(8)?.try_into().expect("eight-byte slice"),
        ))
    }

    fn bool(&mut self) -> Result<bool, CarrierError> {
        match self.u32()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => invalid("boolean code invalid"),
        }
    }

    fn array32(&mut self) -> Result<[u8; 32], CarrierError> {
        Ok(self.take(32)?.try_into().expect("32-byte slice"))
    }

    fn string(&mut self) -> Result<String, CarrierError> {
        let len = self.u32()? as usize;
        if len > MAX_STRING_BYTES {
            return invalid("string length exceeds bound");
        }
        String::from_utf8(self.take(len)?.to_vec())
            .map_err(|_| CarrierError::Invalid("string is not utf-8".into()))
    }

    fn optional_string(&mut self) -> Result<Option<String>, CarrierError> {
        match self.u32()? {
            0 => Ok(None),
            1 => Ok(Some(self.string()?)),
            _ => invalid("optional-string presence code invalid"),
        }
    }

    fn text_refs(&mut self) -> Result<ResolvedTextRefs, CarrierError> {
        Ok(ResolvedTextRefs {
            balance_name: self.string()?,
            left_side_name: self.string()?,
            right_side_name: self.string()?,
            active_range_name: self.string()?,
            balance_tooltip: self.string()?,
            active_range_tooltip: self.string()?,
        })
    }

    fn done(&self) -> bool {
        self.cursor == self.data.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn balance() -> PowerBalanceInput {
        PowerBalanceInput {
            source_record_id: "d73:42:ITA:ITA_power_balance".into(),
            country_tag: "ITA".into(),
            balance_id: "ITA_power_balance".into(),
            display_ordinal: 0,
            active_side_id: "ITA_grand_council_side".into(),
            trending_side_id: "ITA_grand_council_side".into(),
            active_range_id: "ITA_grand_council_medium_control_range".into(),
            effective_left_icon: "GFX_bop_ITA_grand_council_side".into(),
            effective_right_icon: "GFX_bop_ITA_mussolini_side".into(),
            launcher_visible: true,
            launcher_percentage_text_ref: Some("a25:42:bop:launcher_percentage".into()),
            launcher_levels_text_ref: Some("a25:42:bop:launcher_levels".into()),
            launcher_icon_sprite: Some("GFX_bop_ITA_grand_council_side".into()),
            launcher_glow_visible: true,
            bar_state: BarState::MovingLeft,
            range_indicator_frame: 0,
            range_indicator_top_left: [198, 101],
            trend: Some(TrendInput {
                cadence: ChangeCadence::Daily,
                magnitude: 0.01,
                resolved_text_ref: "a25:42:trend".into(),
            }),
            power_balance_modifier_desc_text_ref: "a25:42:bop:modifier_desc".into(),
            decisions_complete: true,
            decisions: vec![DecisionInput {
                decision_id: "ITA_bop_decision".into(),
                source_record_id: "a34:42:ITA:0".into(),
                availability: DecisionAvailability::Available,
                resolved_name_text_ref: "a25:42:decision:0:name".into(),
                cost_text_ref: Some("a25:42:decision:0:cost".into()),
                timer_text_ref: None,
                target_tag_ref: None,
            }],
            resolved_text_refs: ResolvedTextRefs {
                balance_name: "a25:42:bop:name".into(),
                left_side_name: "a25:42:bop:left".into(),
                right_side_name: "a25:42:bop:right".into(),
                active_range_name: "a25:42:bop:range".into(),
                balance_tooltip: "a25:42:bop:tooltip".into(),
                active_range_tooltip: "a25:42:bop:range_tooltip".into(),
            },
        }
    }

    fn frame() -> PowerBalancePresentationFrame {
        PowerBalancePresentationFrame {
            elapsed_wall_ms: 7,
            status_flags: 0,
            frame_hour: 42,
            producer_manifest_sha256: A729_CONTROLLER_EXACT20_MANIFEST_SHA256,
            completeness_record_identity: "d74-complete:42".into(),
            expected_balance_count: 1,
            balances: vec![balance()],
        }
    }

    #[test]
    fn exact_frame_roundtrips_byte_stably() {
        let encoded = encode_payload(&frame()).unwrap();
        let decoded = decode_payload(&encoded).unwrap();
        assert_eq!(decoded, frame());
        assert_eq!(encode_payload(&decoded).unwrap(), encoded);
    }

    #[test]
    fn authenticated_measured_empty_frame_is_distinct_and_roundtrips() {
        let empty = PowerBalancePresentationFrame {
            elapsed_wall_ms: 8,
            status_flags: 0,
            frame_hour: 43,
            producer_manifest_sha256: A729_CONTROLLER_EXACT20_MANIFEST_SHA256,
            completeness_record_identity: "d74-complete:43:empty".into(),
            expected_balance_count: 0,
            balances: Vec::new(),
        };
        let encoded = encode_payload(&empty).expect("authenticated measured-empty frame");
        assert_eq!(decode_payload(&encoded).unwrap(), empty);
    }

    #[test]
    fn completeness_order_and_runtime_leaves_fail_closed() {
        let mut bad = frame();
        bad.expected_balance_count = 2;
        assert!(validate_frame(&bad).is_err());

        let mut bad = frame();
        bad.balances[0].display_ordinal = 1;
        assert!(validate_frame(&bad).is_err());

        let mut bad = frame();
        bad.balances[0]
            .power_balance_modifier_desc_text_ref
            .clear();
        assert!(validate_frame(&bad).is_err());

        let mut bad = frame();
        bad.balances[0].trend = None;
        assert!(validate_frame(&bad).is_err());

        let mut bad = frame();
        bad.balances[0].resolved_text_refs.balance_name.clear();
        assert!(validate_frame(&bad).is_err());

        let mut bad = frame();
        bad.balances[0].launcher_percentage_text_ref = None;
        assert!(validate_frame(&bad).is_err());

        let mut bad = frame();
        bad.balances[0].launcher_visible = false;
        assert!(validate_frame(&bad).is_err());

        let mut bad = frame();
        let mut second = balance();
        second.balance_id = "ITA_second_balance".into();
        second.source_record_id = "d73:42:ITA:second".into();
        bad.expected_balance_count = 2;
        bad.balances.push(second);
        assert!(validate_frame(&bad).is_err());

        let mut bad = frame();
        bad.balances[0].source_record_id = "controller\u{1f}record".into();
        assert!(validate_frame(&bad).is_err());
    }

    #[test]
    fn engine_indicator_frames_normalize_to_portable_zero_based_frames() {
        assert_eq!(normalize_engine_range_indicator_frame(1), Ok(0));
        assert_eq!(normalize_engine_range_indicator_frame(2), Ok(1));
        assert!(normalize_engine_range_indicator_frame(0).is_err());
        assert!(normalize_engine_range_indicator_frame(3).is_err());
    }

    #[test]
    fn controller_state_keeps_active_side_movement_and_trending_identity_independent() {
        let right_active_trending_left = normalize_a729_controller_state(
            &A729ControllerStateObservation {
                current_value_raw: 35_000,
                change_raw: -1_000,
                left_side_id: "left".into(),
                right_side_id: "right".into(),
                trending_side_id: "right".into(),
                engine_range_indicator_frame: 2,
            },
        )
        .unwrap();
        assert_eq!(right_active_trending_left.active_side_id, "right");
        assert_eq!(right_active_trending_left.trending_side_id, "right");
        assert_eq!(right_active_trending_left.bar_state, BarState::MovingLeft);
        assert_eq!(right_active_trending_left.range_indicator_frame, 1);

        let left_active_trending_right = normalize_a729_controller_state(
            &A729ControllerStateObservation {
                current_value_raw: -35_000,
                change_raw: 1_000,
                left_side_id: "left".into(),
                right_side_id: "right".into(),
                trending_side_id: "left".into(),
                engine_range_indicator_frame: 1,
            },
        )
        .unwrap();
        assert_eq!(left_active_trending_right.active_side_id, "left");
        assert_eq!(left_active_trending_right.trending_side_id, "left");
        assert_eq!(left_active_trending_right.bar_state, BarState::MovingRight);
        assert_eq!(left_active_trending_right.range_indicator_frame, 0);
    }

    #[test]
    fn exact20_authority_is_bound_but_live_observer_stays_inactive() {
        assert!(A729_CONTROLLER_EXACT20_AUTHORITY_BOUND);
        assert!(!A729_CONTROLLER_OBSERVER_ACTIVATED);
        assert_eq!(
            collect_and_encode(&Source(true), 42, 7, 0),
            Err(CarrierError::UnauthenticatedSource)
        );
    }

    struct Source(bool);

    impl ExactPowerBalancePresentationSource for Source {
        fn is_exact_a729_power_balance_source(&self) -> bool {
            self.0
        }
        fn producer_manifest_sha256(&self) -> [u8; 32] {
            A729_CONTROLLER_EXACT20_MANIFEST_SHA256
        }
        fn completeness_record_identity(&self, frame_hour: u64) -> Result<String, String> {
            Ok(format!("complete:{frame_hour}"))
        }
        fn expected_balance_count(&self, _: u64) -> Result<u32, String> {
            Ok(1)
        }
        fn collect_frame(&self, _: u64) -> Result<Vec<PowerBalanceInput>, String> {
            Ok(vec![balance()])
        }
    }

    #[test]
    fn source_authentication_and_trailing_bytes_are_enforced() {
        assert_eq!(
            collect_and_encode(&Source(false), 42, 7, 0),
            Err(CarrierError::UnauthenticatedSource)
        );
        assert_eq!(
            collect_and_encode(&Source(true), 42, 7, 0),
            Err(CarrierError::UnauthenticatedSource)
        );
        let mut encoded = encode_payload(&frame()).unwrap();
        encoded.push(0);
        assert_eq!(decode_payload(&encoded), Err(CarrierError::TrailingBytes));
    }
}
