//! Selected transport admission and output over current-hour facts.
//! Inputs are joined per viewer-local request from current-hour simulation
//! observations. The same result serves the separate ordered move-reason lane.

pub type Fact<T> = Result<T, &'static str>;
const ONE: i64 = 100_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    Army(u64),
    RailwayGun(u64),
    Other,
}

impl Unit {
    fn transport_candidate(self) -> bool {
        matches!(self, Self::Army(_) | Self::RailwayGun(_))
    }
}

/// Divide by fixed5 scale with signed truncation, add one only for a
/// positive non-integral input, multiply back with i64 wrap, divide again,
/// then consume the low signed 32 bits. The multiply matters at i64::MAX;
/// this is not plain mathematical ceiling followed by a cast.
pub fn unit_convoys(raw_weight: i64) -> i32 {
    let quotient = raw_weight / ONE;
    let rounded_raw = if raw_weight == quotient * ONE {
        raw_weight
    } else {
        quotient
            .wrapping_add(i64::from(raw_weight > 0))
            .wrapping_mul(ONE)
    };
    (rounded_raw / ONE) as i32
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Transfer {
    /// Raw signed transfer status. Every nonzero value takes the combat arm.
    pub status: Fact<i32>,
    /// The authenticated transfer source province. Its refusal is reached
    /// only when status is exactly zero.
    pub source: Fact<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    NoTargetBase,
    NoSourceBase,
    DuringCombat,
    AlreadyActive,
    NotEnough { available: i32, required: i32 },
    NoAccess,
    NoSeaRoute,
}

impl Reason {
    pub fn key(self) -> &'static str {
        match self {
            Self::NoTargetBase => "TRANSPORT_NO_NAVAL_BASE",
            Self::NoSourceBase => "TRANSPORT_NO_FROM_NAVAL_BASE",
            Self::DuringCombat => "TRANSPORT_NOT_DURING_COMBAT",
            Self::AlreadyActive => "TRANSPORT_ALREADY_ACTIVE",
            Self::NotEnough { .. } => "TRANSPORT_NOT_ENOUGH",
            Self::NoAccess => "TRANSPORT_NO_ACCESS",
            Self::NoSeaRoute => "TRANSPORT_NO_SEA_ROUTE",
        }
    }
}

/// Boolean admission is independent of localised output byte length.
/// A reason whose recorded-content localisation is empty remains false.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Admission {
    pub allowed: bool,
    pub reason: Option<Reason>,
}

fn rejected(reason: Reason) -> Admission {
    Admission {
        allowed: false,
        reason: Some(reason),
    }
}

/// Local-world inputs supply facts only when the reached branch needs
/// them. Target-specific calls require current operands, rather than carried
/// slices, dirty convoy caches, inferred access or a recorded actual route.
pub trait AdmissionInputs {
    fn target_base(&mut self) -> Fact<bool>;
    fn source_land(&mut self) -> Fact<bool>;
    fn source_base(&mut self) -> Fact<bool>;
    fn coastal_special_template(&mut self) -> Fact<bool>;
    fn transfer(&mut self) -> Fact<Option<Transfer>>;
    fn weight(&mut self) -> Fact<i64>;
    /// Own/aliased versus other convoy-pool selection, with dirty-pool
    /// fallback, priority, reservation filtering and signed arithmetic.
    fn available_convoys(&mut self) -> Fact<i32>;
    fn military_access(&mut self) -> Fact<bool>;
    fn sea_route(&mut self) -> Fact<bool>;
}

pub fn admit(unit: Unit, source: u32, inputs: &mut impl AdmissionInputs) -> Fact<Admission> {
    if !inputs.target_base()? {
        return Ok(rejected(Reason::NoTargetBase));
    }
    if inputs.source_land()? && !inputs.source_base()? {
        return if matches!(unit, Unit::Army(_)) && inputs.coastal_special_template()? {
            // Early success skips transfer, convoy, access and path branches.
            Ok(Admission {
                allowed: true,
                reason: None,
            })
        } else {
            Ok(rejected(Reason::NoSourceBase))
        };
    }
    if let Some(transfer) = inputs.transfer()? {
        if transfer.status? != 0 {
            return Ok(rejected(Reason::DuringCombat));
        }
        if transfer.source? == source {
            return Ok(rejected(Reason::AlreadyActive));
        }
    }
    let required = unit_convoys(inputs.weight()?);
    let available = inputs.available_convoys()?;
    if available < required {
        return Ok(rejected(Reason::NotEnough {
            available,
            required,
        }));
    }
    if !inputs.military_access()? {
        return Ok(rejected(Reason::NoAccess));
    }
    if !inputs.sea_route()? {
        return Ok(rejected(Reason::NoSeaRoute));
    }
    Ok(Admission {
        allowed: true,
        reason: None,
    })
}

/// Both outputs are independently localised bytes. Secondary output
/// participates in the first-call length gate but is never appended there.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Outputs {
    pub primary: Vec<u8>,
    pub secondary: Vec<u8>,
}

/// Transport over a viewer-local insertion-ordered selection. Sources
/// come from the chosen actual/planned mode; sea admission separately
/// uses the unit's actual location. No interface source cache is recorded.
pub trait MessageInputs {
    fn target_base(&mut self, target: u32) -> Result<bool, String>;
    fn source(&mut self, unit: Unit) -> Result<u32, String>;
    fn same_area(&mut self, source: u32, target: u32) -> Result<bool, String>;
    fn source_base(&mut self, source: u32) -> Result<bool, String>;
    fn admission(&mut self, unit: Unit, source: u32, target: u32) -> Result<Admission, String>;
    fn weight(&mut self, unit: Unit) -> Result<i64, String>;
    fn risk(&mut self, unit: Unit, source: u32, target: u32, convoys: i32) -> Result<i64, String>;
}

pub fn message(
    selection: &[Unit],
    target: u32,
    icon: bool,
    inputs: &mut impl MessageInputs,
    content: &impl super::CommonMoveText,
) -> Result<Outputs, String> {
    let mut output = Outputs::default();
    if !inputs.target_base(target)? {
        return Ok(output);
    }
    for &unit in selection {
        if !unit.transport_candidate() {
            continue;
        }
        let source = inputs.source(unit)?;
        let mut rejection = String::new();
        if (!inputs.same_area(source, target)? || inputs.source_base(source)?) && source != target {
            let admission = inputs.admission(unit, source, target)?;
            if admission.allowed {
                let mut convoys = 0_i32;
                for &member in selection {
                    if member.transport_candidate() {
                        convoys = convoys.wrapping_add(unit_convoys(inputs.weight(member)?));
                    }
                }
                let risk = inputs.risk(unit, source, target, convoys)?;
                if risk > 0 {
                    output.primary.push(b'\n');
                    output
                        .primary
                        .extend_from_slice(content.transport_risk(risk)?.as_bytes());
                }
                output.secondary.clear();
                output.secondary.push(b'\n');
                output
                    .secondary
                    .extend_from_slice(content.transport_description(convoys, icon)?.as_bytes());
                return Ok(output);
            }
            if let Some(reason) = admission.reason {
                rejection = if let Reason::NotEnough {
                    available,
                    required,
                } = reason
                {
                    content.transport_stock(available, required)?
                } else {
                    content.plain(reason.key())?
                };
            }
        }
        output.secondary = rejection.into_bytes();
    }
    Ok(output)
}

fn indent_nonempty_lines(text: &[u8], output: &mut Vec<u8>) {
    for (index, line) in text.split(|&byte| byte == b'\n').enumerate() {
        if index != 0 {
            output.push(b'\n');
        }
        if !line.is_empty() {
            output.extend_from_slice(b"    ");
            output.extend_from_slice(line);
        }
    }
}

pub fn append_first(body: &mut Vec<u8>, outputs: &Outputs) {
    if outputs.primary.is_empty() && outputs.secondary.is_empty() {
        return;
    }
    // Transform primary output, then ensure LF on a nonempty body, even
    // when only secondary was nonempty and primary appended zero bytes.
    if !body.is_empty() && body.last() != Some(&b'\n') {
        body.push(b'\n');
    }
    indent_nonempty_lines(&outputs.primary, body);
}

/// The second invocation gates on primary length alone, then appends a
/// literal ASCII space and raw bytes. Empty primary never reads secondary.
pub fn append_railway_second(body: &mut Vec<u8>, outputs: &Outputs) {
    if !outputs.primary.is_empty() {
        body.push(b' ');
        body.extend_from_slice(&outputs.primary);
    }
}

#[cfg(test)]
#[path = "transport_tests.rs"]
mod tests;
