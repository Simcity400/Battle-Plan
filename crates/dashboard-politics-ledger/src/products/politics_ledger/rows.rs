//! The native rows as the ledger's replies show them, and the rules every part of the family
//! reads them by: what a payment is, its category, a hold, a recorded tag.

use super::*;
use dashboard_base::products::validate_tag;

/// A power row as its item's presentation reads it.
pub(super) fn paid(row: &CountryPowerChangeRow) -> PaidRow {
    PaidRow {
        currency: format!("{:?}", row.currency),
        cause: format!("{:?}", row.cause),
        item: (&row.identity).into(),
        charged: row.requested_delta_raw.is_some_and(|raw| raw < 0),
    }
}

/// A daily row as read at `read`, its political and command parts at the reads that supplied
/// them.
pub(super) fn daily(
    row: &CountryPowerDailyRow,
    read: i32,
    reads: (Option<i32>, Option<i32>),
) -> LedgerDaily {
    LedgerDaily {
        country: row.country,
        instance: row.instance,
        epoch: row.epoch.to_string(),
        day_hour: row.day_hour,
        read_hour: read,
        pp_read_hour: reads.0,
        cp_read_hour: reads.1,
        pp_gross_raw: row.pp_gross_gain_raw.map(|raw| raw.to_string()),
        pp_cost_raw: row.pp_cost_raw.map(|raw| raw.to_string()),
        pp_income_raw: row.pp_net_gain_raw.map(|raw| raw.to_string()),
        pp_realized_raw: row.pp_realized_delta_raw.map(|raw| raw.to_string()),
        pp_sequence: row.pp_sequence.map(|raw| raw.to_string()),
        cp_income_raw: row.cp_gain_raw.map(|raw| raw.to_string()),
        cp_realized_raw: row.cp_realized_delta_raw.map(|raw| raw.to_string()),
        cp_sequence: row.cp_sequence.map(|raw| raw.to_string()),
    }
}

/// What one day's latest daily row adds to the gains of a currency (command when `command`).
pub(super) fn gain(row: &LedgerDaily, command: bool) -> io::Result<super::tally::GainDay> {
    let (gain, net, applied, read) = if command {
        (
            amount(row.cp_income_raw.as_deref())?,
            amount(row.cp_income_raw.as_deref())?,
            amount(row.cp_realized_raw.as_deref())?,
            row.cp_read_hour,
        )
    } else {
        (
            amount(row.pp_gross_raw.as_deref())?,
            amount(row.pp_income_raw.as_deref())?,
            amount(row.pp_realized_raw.as_deref())?,
            row.pp_read_hour,
        )
    };
    Ok(gain_day(gain, net, applied, read.is_some(), command))
}

/// [`gain`] of a native daily row whose political and command parts were read at `reads`.
pub(super) fn native_gain(
    row: &CountryPowerDailyRow,
    reads: (Option<i32>, Option<i32>),
    command: bool,
) -> super::tally::GainDay {
    let (gain, net, applied, read) = if command {
        (row.cp_gain_raw, row.cp_gain_raw, row.cp_realized_delta_raw, reads.1)
    } else {
        (
            row.pp_gross_gain_raw,
            row.pp_net_gain_raw,
            row.pp_realized_delta_raw,
            reads.0,
        )
    };
    gain_day(
        gain.map(i128::from),
        net.map(i128::from),
        applied.map(i128::from),
        read.is_some(),
        command,
    )
}

fn gain_day(
    gain: Option<i128>,
    net: Option<i128>,
    applied: Option<i128>,
    read: bool,
    command: bool,
) -> super::tally::GainDay {
    let gained = gain
        .filter(|_| read)
        .map(|gain| if command { gain.max(0) } else { gain });
    let lost = match net.zip(applied) {
        Some((net, applied)) if read && net > 0 && applied < net => (net - applied.max(0)) as u128,
        _ => 0,
    };
    (gained, lost, read)
}

/// Whether a daily row holds its political part, and its command part.
pub(super) fn daily_parts(row: &CountryPowerDailyRow) -> (bool, bool) {
    (
        [
            row.pp_gross_gain_raw,
            row.pp_cost_raw,
            row.pp_net_gain_raw,
            row.pp_realized_delta_raw,
        ]
        .iter()
        .any(Option::is_some)
            || row.pp_sequence.is_some(),
        row.cp_gain_raw.is_some()
            || row.cp_realized_delta_raw.is_some()
            || row.cp_sequence.is_some(),
    )
}

/// The political part a daily application commits together, and the command part.
pub(super) fn daily_political(
    row: &CountryPowerDailyRow,
) -> (
    Option<i64>,
    Option<i64>,
    Option<i64>,
    Option<i64>,
    Option<u64>,
) {
    (
        row.pp_gross_gain_raw,
        row.pp_cost_raw,
        row.pp_net_gain_raw,
        row.pp_realized_delta_raw,
        row.pp_sequence,
    )
}

pub(super) fn daily_command(row: &CountryPowerDailyRow) -> (Option<i64>, Option<i64>, Option<u64>) {
    (row.cp_gain_raw, row.cp_realized_delta_raw, row.cp_sequence)
}

pub(super) fn government(
    row: &GovernmentChangeRow,
    read_hour: i32,
    tokens: (Option<String>, Option<String>),
    covered: bool,
) -> LedgerGovernment {
    LedgerGovernment {
        country: row.country,
        instance: row.instance,
        sequence: row.sequence.to_string(),
        epoch: row.epoch.to_string(),
        hour: row.hour,
        read_hour,
        cause: format!("{:?}", row.cause),
        before_token: tokens.0,
        after_token: tokens.1,
        covered,
    }
}

/// A recorded tag the dashboard can be asked for. Tags take three non-NUL bytes without a UTF-8
/// check; only the unrequestable identity is refused, keeping the other countries' facts.
pub(super) fn valid_tag(id: u32, slot: u32, tag: &[u8]) -> Result<&str, String> {
    let reason = match std::str::from_utf8(tag) {
        Err(_) => Some("not UTF-8"),
        Ok(text) => validate_tag(text).err().map(|_| {
            if text.is_empty() {
                "empty"
            } else if text.contains('\0') {
                "contains NUL"
            } else {
                "exceeds shared country-tag length bound"
            }
        }),
    };
    if let Some(reason) = reason {
        let escaped: String = tag
            .iter()
            .take(32)
            .flat_map(|byte| std::ascii::escape_default(*byte))
            .map(char::from)
            .collect();
        return Err(format!(
            "country {id} canonical slot {slot}: recorded tag b\"{escaped}\"{} ({} bytes), {reason}",
            if tag.len() > 32 { " (prefix)" } else { "" },
            tag.len()
        ));
    }
    Ok(std::str::from_utf8(tag).expect("validated UTF-8"))
}

/// Native categories are capture provenance, not a content-price catalogue.
pub(super) fn category(cause: PowerCause) -> (&'static str, &'static str) {
    match cause {
        PowerCause::Idea => ("ideas", "Ideas"),
        PowerCause::Advisor | PowerCause::AdvisorRemoval => ("advisors", "Advisors"),
        PowerCause::Decision | PowerCause::TargetedDecision | PowerCause::DecisionTimeout => {
            ("decisions", "Decisions")
        }
        PowerCause::Diplomacy | PowerCause::JoinWar => ("diplomacy", "Diplomacy"),
        PowerCause::WarJustification => ("warJustification", "War justification"),
        PowerCause::Focus | PowerCause::FocusRefund => ("focus", "Focus"),
        PowerCause::ScriptEffect | PowerCause::ScaledScriptEffect => {
            ("scriptEffects", "Script effects")
        }
        PowerCause::Autonomy => ("autonomy", "Autonomy"),
        PowerCause::SpyMaster => ("spyMaster", "Spy master"),
        PowerCause::ScientistRecruitment | PowerCause::ScientistFaction => {
            ("scientists", "Scientists")
        }
        PowerCause::ScorchedEarth => ("scorchedEarth", "Scorched earth"),
        PowerCause::PrideOfFleet => ("prideOfFleet", "Pride of the fleet"),
        PowerCause::Blockade => ("blockade", "Blockade"),
        PowerCause::MioPolicy => ("mioPolicy", "Organisation policies"),
        PowerCause::Medal => ("medals", "Medals"),
        PowerCause::Console => ("console", "Console"),
        PowerCause::ResetSetup => ("setup", "Setup"),
        PowerCause::AbilityActivation => ("abilities", "Abilities"),
        PowerCause::AbilityUpkeep => ("upkeep", "Upkeep"),
        PowerCause::Allocation | PowerCause::AllocationRefund => ("allocation", "Allocation"),
        PowerCause::LeaderTrait => ("traits", "Traits"),
        PowerCause::LeaderPromotion => ("promotions", "Promotions"),
        PowerCause::LeaderAssignment => ("assignments", "Assignments"),
        PowerCause::LeaderRecruitment => ("recruitment", "Recruitment"),
        PowerCause::Raid => ("raids", "Raids"),
        PowerCause::AirAction => ("airActions", "Air actions"),
        PowerCause::PreferredTactic => ("tactics", "Preferred tactics"),
        PowerCause::OfficerUpgrade => ("officerUpgrades", "Officer upgrades"),
        PowerCause::CaptainUpgrade => ("captainUpgrades", "Captain upgrades"),
        PowerCause::TimedActivity => ("timedActivities", "Timed activities"),
        PowerCause::DailyGain => ("dailyDeficit", "Daily deficit"),
        PowerCause::CapCorrection => ("capCorrection", "Cap correction"),
        PowerCause::NotIdentified => ("notIdentified", "Not identified"),
    }
}

/// The capture normalizes evaluated charges to signed deltas. Do not classify from the
/// realized delta: a no-deficit/cap floor may swallow it.
pub(super) fn charge(row: &CountryPowerChangeRow) -> Option<Option<i128>> {
    // PP's net daily application includes its separately recorded costs. CP instead applies
    // the signed daily operand without clamping it nonnegative.
    if is_hold(row)
        || row.cause == PowerCause::DailyGain && row.currency == PowerCurrency::Political
        || !matches!(
            row.operation,
            PowerOperation::Add | PowerOperation::Subtract
        )
    {
        return None;
    }
    match row.requested_delta_raw.map(i128::from) {
        Some(raw) if raw < 0 => Some(Some(-raw)),
        Some(_) => None,
        None if row.operation == PowerOperation::Subtract => Some(None),
        None => None,
    }
}

/// Charging callers label allocations as AbilityActivation, Raid, AirAction, Advisor or
/// LeaderAssignment; the writer identifies a hold. Ground crew releases, resizes and
/// reallocates its reservation and charges the adjustment directly.
pub(super) fn is_hold(row: &CountryPowerChangeRow) -> bool {
    row.currency == PowerCurrency::Command
        && (matches!(
            row.site,
            PowerSite::AllocateCommandPower
                | PowerSite::ReleaseCommandPower
                | PowerSite::GroundCrew
        ) || row.cause == PowerCause::AllocationRefund)
}

/// The holder an allocation names: its object, character, item or region.
pub(super) fn holder_key(item: &LedgerItem) -> Option<String> {
    if item.object_kind != "None" {
        return item.object_reference.as_ref().map(|reference| {
            format!(
                "{}:{reference}:{:?}:{:?}:{:?}:{:?}:{:?}:{:?}",
                item.object_kind,
                item.key,
                item.token,
                item.slot_key,
                item.target_country,
                item.target_state,
                item.target_region
            )
        });
    }
    if let Some(character) = item.character {
        return Some(format!(
            "character:{character}:{:?}:{:?}:{:?}",
            item.key, item.token, item.slot_key
        ));
    }
    if !item.key.is_empty() || item.token.is_some() {
        return Some(format!(
            "item:{:?}:{:?}:{:?}:{:?}:{:?}",
            item.key, item.token, item.target_country, item.target_state, item.target_region
        ));
    }
    item.target_region.map(|region| format!("region:{region}"))
}

pub(super) fn holder_kind(row: &CountryPowerChangeRow) -> String {
    if row.identity.object_kind != PowerObjectKind::None {
        format!("{:?}", row.identity.object_kind)
    } else {
        format!("{:?}", row.cause)
    }
}

/// A command power holder, by the operands its allocation recorded.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[derive(replay_spool::Spooled)]
pub(super) struct Holder {
    pub kind: String,
    pub amount: i128,
    pub ability: bool,
    pub token: Option<String>,
    pub character: Option<(Option<String>, String)>,
}

/// The holder a command hold names, by the operands its facts (`dated`, serialized) recorded;
/// None for any other row.
pub(super) fn holder(row: &CountryPowerChangeRow, dated: Option<&[u8]>) -> Option<Holder> {
    if row.currency != PowerCurrency::Command || !is_hold(row) {
        return None;
    }
    let facts = dated
        .and_then(|bytes| dashboard_base::prepared::codec::decode::<items::Dated>(bytes).ok())
        .map(|dated| dated.facts)
        .unwrap_or_else(|| items::event_facts::<replay_frame::frame::DecodedFrame>(None, &row.identity, row.cause, None));
    Some(Holder {
        kind: holder_kind(row),
        amount: 0,
        ability: row.cause == PowerCause::AbilityActivation
            || row.identity.object_kind == PowerObjectKind::Ability,
        token: facts.token,
        character: facts
            .character
            .map(|character| (character.script_token, character.saved_name)),
    })
}

/// What makes two payments one item: the digest of the facts their hour recorded, and what they
/// record. Every ability's upkeep is one item.
pub(super) fn power_item(row: &CountryPowerChangeRow, dated: Option<&[u8; 32]>) -> [u8; 32] {
    if row.currency == PowerCurrency::Command && row.cause == PowerCause::AbilityUpkeep {
        return *blake3::hash(&[ABILITY_UPKEEP]).as_bytes();
    }
    let mut hash = item_hash(POWER, dated, row.cause, row.currency as u8, &row.identity);
    hash.update(&[u8::from(row.requested_delta_raw.is_some_and(|raw| raw < 0))]);
    *hash.finalize().as_bytes()
}

pub(super) fn cost_item(row: &CountryPowerDailyCostRow, dated: Option<&[u8; 32]>) -> [u8; 32] {
    let mut hash = item_hash(COST, dated, row.cause, row.component as u8, &row.identity);
    hash.update(&[u8::from(row.cost_raw < 0)]);
    *hash.finalize().as_bytes()
}

const POWER: u8 = 0;
const COST: u8 = 1;
const ABILITY_UPKEEP: u8 = 2;

/// An item's hash over every field it is told apart by, each length-prefixed or fixed-width.
fn item_hash(
    kind: u8,
    dated: Option<&[u8; 32]>,
    cause: PowerCause,
    part: u8,
    identity: &PowerIdentity,
) -> blake3::Hasher {
    fn optional(hash: &mut blake3::Hasher, value: Option<u64>) {
        match value {
            Some(value) => hash.update(&[1]).update(&value.to_le_bytes()),
            None => hash.update(&[0]),
        };
    }
    fn bytes(hash: &mut blake3::Hasher, value: &[u8]) {
        hash.update(&(value.len() as u64).to_le_bytes()).update(value);
    }
    let mut hash = blake3::Hasher::new();
    hash.update(&[kind]);
    match dated {
        Some(digest) => hash.update(&[1]).update(digest),
        None => hash.update(&[0]),
    };
    hash.update(&(cause as u16).to_le_bytes())
        .update(&[part, identity.action as u8]);
    optional(&mut hash, identity.item_token.map(u64::from));
    bytes(&mut hash, &identity.item_key);
    bytes(&mut hash, &identity.slot_key);
    optional(&mut hash, identity.character.map(u64::from));
    hash.update(&[identity.object_kind as u8]);
    optional(&mut hash, identity.object_reference);
    optional(&mut hash, identity.target_country.map(u64::from));
    optional(&mut hash, identity.target_state.map(u64::from));
    optional(&mut hash, identity.target_region.map(u64::from));
    hash.update(&[identity.origin_kind as u8]);
    optional(&mut hash, identity.origin_token.map(u64::from));
    bytes(&mut hash, &identity.origin_key);
    hash
}

pub(super) fn mutation_id(country: u32, epoch: u64, sequence: u64) -> String {
    format!("{country}:{epoch}:{sequence}")
}

pub(super) fn cost_id(country: u32, epoch: u64, day: i32, ordinal: u32) -> String {
    format!("daily:{country}:{epoch}:{day}:{ordinal}")
}

/// A daily cost as the payment the presentation resolves it as.
pub(super) fn paid_cost(row: &CountryPowerDailyCostRow) -> PaidRow {
    PaidRow {
        currency: "Political".into(),
        cause: format!("{:?}", row.cause),
        item: (&row.identity).into(),
        // A daily cost is paid as its negation.
        charged: row.cost_raw > 0,
    }
}

pub(super) fn is_recurring(category: &str) -> bool {
    matches!(
        category,
        "dailyCosts" | "organisationUpkeep" | "upkeep" | "dailyDeficit"
    )
}

/// A reason without the hour or frame it names: equal reasons of different reads count as one.
pub(super) fn undated_reason(reason: &str) -> String {
    let mut words = reason.split_whitespace().peekable();
    let mut result = Vec::new();
    while let Some(word) = words.next() {
        result.push(word);
        if matches!(word, "hour" | "frame")
            && words.peek().is_some_and(|next| {
                next.trim_end_matches([':', ',', ';', '.'])
                    .parse::<i32>()
                    .is_ok()
            })
        {
            words.next();
        }
    }
    result.join(" ")
}

pub(super) fn combine_reason(target: &mut Option<String>, reason: Option<String>) {
    if let Some(reason) = reason {
        let reason = undated_reason(&reason);
        *target = Some(match target.take() {
            Some(previous) if previous != reason => format!("{previous}; {reason}"),
            Some(previous) => previous,
            None => reason,
        });
    }
}

pub(super) fn count_reason(
    reasons: &mut BTreeMap<String, u64>,
    reason: String,
    count: u64,
) -> io::Result<()> {
    let old = reasons.entry(reason).or_default();
    *old = old
        .checked_add(count)
        .ok_or_else(|| io::Error::other("Politics warning count overflow"))?;
    Ok(())
}

pub(super) fn sum_raw(values: impl Iterator<Item = Option<i128>>) -> io::Result<Option<i128>> {
    let mut total = 0i128;
    for value in values {
        let Some(value) = value else {
            return Ok(None);
        };
        total = total.checked_add(value).ok_or_else(|| {
            io::Error::other("Politics spending sum exceeds fixed-point integer range")
        })?;
    }
    Ok(Some(total))
}

pub(super) fn amount(value: Option<&str>) -> io::Result<Option<i128>> {
    value
        .map(str::parse)
        .transpose()
        .map_err(|error: std::num::ParseIntError| dashboard_core::invalid(error.to_string()))
}

pub(super) fn merge_spending_coverage(
    rows: &mut Vec<LedgerSpendCoverage>,
    value: LedgerSpendCoverage,
) {
    if let Some(old) = rows.last_mut()
        && old.political == value.political
        && old.command == value.command
        && old.reason == value.reason
        && old.through_hour.and_then(|hour| hour.checked_add(1)) == value.since_hour
    {
        old.through_hour = value.through_hour;
    } else {
        rows.push(value);
    }
}
