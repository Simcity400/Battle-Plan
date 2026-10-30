//! The native rows as the ledger's replies show them, and the rules every part of the family
//! reads them by: what a payment is, its category, a hold, a recorded tag.

use super::*;
use dashboard_base::products::validate_tag;

pub(super) fn mutation(
    row: &CountryPowerChangeRow,
    read_hour: i32,
    covered: bool,
) -> LedgerMutation {
    LedgerMutation {
        country: row.country,
        instance: row.instance,
        sequence: row.sequence.to_string(),
        epoch: row.epoch.to_string(),
        hour: row.hour,
        read_hour,
        currency: format!("{:?}", row.currency),
        cause: format!("{:?}", row.cause),
        operation: format!("{:?}", row.operation),
        site: format!("{:?}", row.site),
        amount_raw: row.requested_delta_raw.map(|raw| raw.to_string()),
        realized_delta_raw: row.realized_delta_raw().to_string(),
        before_raw: row.balance_before_raw.to_string(),
        after_raw: row.balance_after_raw.to_string(),
        item: (&row.identity).into(),
        covered,
    }
}

pub(super) fn cost(
    row: &CountryPowerDailyCostRow,
    read_hour: i32,
    covered: bool,
) -> LedgerDailyCost {
    LedgerDailyCost {
        country: row.country,
        instance: row.instance,
        epoch: row.epoch.to_string(),
        day_hour: row.day_hour,
        pp_sequence: row.pp_sequence.to_string(),
        ordinal: row.ordinal,
        read_hour,
        covered,
        component: format!("{:?}", row.component),
        amount_raw: row.cost_raw.to_string(),
        cause: format!("{:?}", row.cause),
        item: (&row.identity).into(),
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
    let gained = gain
        .filter(|_| read.is_some())
        .map(|gain| if command { gain.max(0) } else { gain });
    let lost = match net.zip(applied) {
        Some((net, applied)) if read.is_some() && net > 0 && applied < net => {
            (net - applied.max(0)) as u128
        }
        _ => 0,
    };
    Ok((gained, lost, read.is_some()))
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
        before_party: row.before_party,
        after_party: row.after_party,
        before_ideology: row.before_ideology,
        after_ideology: row.after_ideology,
        before_token: tokens.0,
        after_token: tokens.1,
        actor_country: row.actor_country,
        origin_kind: format!("{:?}", row.origin_kind),
        origin_token: row.origin_token,
        origin_key: row.origin_key.clone(),
        covered,
    }
}

/// A capture as the coverage of read `read_hour` names it.
pub(super) fn capture(row: &PoliticsCaptureCoverageRow, read_hour: i32) -> LedgerCoverage {
    LedgerCoverage {
        country: Some(row.country),
        instance: Some(row.instance),
        read_hour,
        since_hour: Some(row.since_hour),
        through_hour: Some(row.through_hour),
        epoch: Some(row.epoch.to_string()),
        streams: row.streams,
        reason: None,
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

pub(super) fn mutation_id(country: u32, epoch: u64, sequence: u64) -> String {
    format!("{country}:{epoch}:{sequence}")
}

pub(super) fn cost_id(country: u32, epoch: u64, day: i32, ordinal: u32) -> String {
    format!("daily:{country}:{epoch}:{day}:{ordinal}")
}

/// A daily cost as the payment the presentation resolves it as.
pub(super) fn daily_mutation(cost: &LedgerDailyCost) -> LedgerMutation {
    LedgerMutation {
        country: cost.country,
        instance: cost.instance,
        sequence: cost.pp_sequence.clone(),
        epoch: cost.epoch.clone(),
        hour: cost.day_hour,
        read_hour: cost.read_hour,
        currency: "Political".into(),
        cause: cost.cause.clone(),
        operation: "Subtract".into(),
        site: format!("daily component:{}", cost.ordinal),
        amount_raw: cost
            .amount_raw
            .parse::<i128>()
            .ok()
            .and_then(i128::checked_neg)
            .map(|raw| raw.to_string()),
        realized_delta_raw: String::new(),
        before_raw: String::new(),
        after_raw: String::new(),
        item: cost.item.clone(),
        covered: cost.covered,
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

/// Whether `value` continues `old`, the coverage a country's last read gave.
pub(super) fn continues_coverage(old: &LedgerCoverage, value: &LedgerCoverage) -> bool {
    old.country == value.country
        && old.instance == value.instance
        && old.epoch == value.epoch
        && old.streams == value.streams
        && old.reason == value.reason
        && old.through_hour.and_then(|hour| hour.checked_add(1)) == value.through_hour
        && old
            .through_hour
            .zip(value.since_hour)
            .is_some_and(|(last, first)| first <= last.saturating_add(1))
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
