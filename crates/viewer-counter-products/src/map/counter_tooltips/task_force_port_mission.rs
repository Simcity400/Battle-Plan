
use super::*;
use replay_data::{
    rows::NavalPortMissionInputs, rows::ShipModelRow, rows::ShipRow,
    rows::TaskForceCounterInputRow, rows::TaskForceRow,
};
use viewer_unit_reads::port_mission_defines::Defines;
#[cfg(test)]
use viewer_unit_reads::port_mission_defines::ScoreDefines;
use viewer_foundation::tooltip_text::ensure_newlines;

const SCALE: i64 = 100_000;
const DIVISION_REFUSED: i64 = 0xffff_ffff;

fn divisible(numerator: i64) -> bool {
    (numerator as u64).wrapping_add(0x7fff_ffff_ffff_fffe) < 0xffff_ffff_ffff_fffd
}

fn fraction(numerator: i64, denominator: i64) -> Result<i64, String> {
    if denominator == 0 || !divisible(numerator) {
        return Ok(DIVISION_REFUSED);
    }
    numerator
        .wrapping_mul(SCALE)
        .checked_div(denominator)
        .ok_or_else(|| "native naval readiness fixed5 division traps".into())
}

fn multiply(a: i64, b: i64) -> i64 {
    a.wrapping_mul(b) / SCALE
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Power {
    ships: i32,
    capitals: i32,
    carriers: i32,
    light: i64,
    heavy: i64,
    light_piercing: i64,
    torpedo: i64,
    strength: i64,
    maximum_strength: i64,
    organisation: i64,
    maximum_organisation: i64,
}

fn score(own: Power, other: Power, defines: &Defines) -> Result<i64, String> {
    if own.ships == own.carriers && other.light_piercing == 0 {
        return Ok(100_000_000);
    }
    if other.ships == other.carriers && own.light_piercing == 0 {
        return Ok(0);
    }
    let d = defines.score.as_ref().map_err(Clone::clone)?;
    let screens = own
        .ships
        .wrapping_sub(own.capitals)
        .wrapping_sub(own.carriers);
    let screening = if screens > 0 {
        let ratio = fraction(i64::from(own.capitals) * SCALE, i64::from(screens) * SCALE)?;
        let screening_ratio = *d.screening_ratio.as_ref().map_err(Clone::clone)?;
        if screening_ratio != 0 && divisible(ratio) {
            fraction(ratio, screening_ratio)?.clamp(0, SCALE)
        } else {
            SCALE
        }
    } else {
        SCALE
    };
    let strength = if own.maximum_strength == 0 {
        0
    } else {
        fraction(own.strength, own.maximum_strength)?
    };
    let organisation = if own.maximum_organisation == 0 {
        0
    } else {
        fraction(own.organisation, own.maximum_organisation)?
    };
    let attacks = [
        own.heavy,
        own.heavy,
        own.light,
        own.light,
        own.torpedo,
        own.torpedo,
    ];
    let base = attacks
        .into_iter()
        .zip(d.efficiency)
        .fold(0_i64, |sum, (attack, efficiency)| {
            sum.wrapping_add(multiply(attack, efficiency))
        });
    let screening = multiply(screening, 25_000).wrapping_add(75_000);
    let health = multiply(
        multiply(strength, 40_000).wrapping_add(multiply(organisation, 60_000)),
        75_000,
    )
    .wrapping_add(25_000);
    Ok(multiply(multiply(base, screening), health))
}

fn exact_ready(own: Power, other: Power, aggression: i64, d: &Defines) -> Result<bool, String> {
    let enemy_score = score(other, own, d)?;
    let own_score = score(own, other, d)?.max(10_000);
    Ok(fraction(enemy_score, own_score)?.min(1_000_000) < aggression)
}

fn mix(mut value: u32) -> u32 {
    value = ((value >> 8) ^ value).wrapping_add(0x68e3_1da4);
    value = (value.wrapping_mul(0x100) ^ value).wrapping_mul(0x1b56_c4e9);
    value = ((value >> 8) ^ value).wrapping_mul(0x92d6_8ca2);
    ((value >> 8) ^ value) & 0x7fff_ffff
}

fn fuzzy_range(base: i64, width: i32, seed: i32) -> (i64, i64) {
    let seed = seed as u32;
    let mut a = seed.wrapping_mul(0x7eb8_da44);
    a = ((a >> 8) ^ a).wrapping_add(0x10d8_365e);
    a = (a.wrapping_mul(0x100) ^ a).wrapping_mul(0x6d22_f31b);
    a = (a >> 8) ^ a;
    let mut b = seed.wrapping_mul(0x205a_d647);
    b = ((b >> 8) ^ b).wrapping_add(0xb4f3_c315);
    b = (b.wrapping_mul(0x100) ^ b).wrapping_mul(0x387a_9f3a);
    b = (b >> 8) ^ b;
    let root = a.wrapping_add(b.wrapping_mul(0xb529_7a4d));
    let range = (base.wrapping_mul(i64::from(width)) / 100).max(100);
    let span = range * 2 + i64::from(range & 1 == 0);
    let first = u64::from(mix(root.wrapping_add(0xb529_7a4d))) | (u64::from(mix(root)) << 30);
    let second = u64::from(mix(root.wrapping_add(0x1f7c_6ee7)))
        | (u64::from(mix(root.wrapping_add(0x6a52_f49a))) << 30);
    let offset = ((first as i64 % span)
        .wrapping_add(second as i64 % span)
        .wrapping_sub(span)
        .wrapping_add(1))
        / 2;
    let center = base.wrapping_add(offset).max(0);
    (
        center.wrapping_sub(range).max(0),
        center.wrapping_add(range).max(200),
    )
}

fn likelihood_from_range(
    own_score: i64,
    range: (i64, i64),
    aggression: i64,
) -> Result<i64, String> {
    let (low, high) = if own_score == 0 {
        (1_000_000, 1_000_000)
    } else {
        (fraction(range.0, own_score)?, fraction(range.1, own_score)?)
    };
    if aggression <= low {
        return Ok(0);
    }
    if aggression <= high {
        let width = high.wrapping_sub(low);
        if width != 0 {
            let value = fraction(aggression.wrapping_sub(low), width)?;
            return Ok(if value > 99_000 && width > 0 {
                99_000
            } else {
                value
            });
        }
    }
    Ok(SCALE)
}

fn fuzzy_likelihood(
    own: Power,
    other: Power,
    aggression: i64,
    width: i32,
    seed: i32,
    d: &Defines,
) -> Result<i64, String> {
    likelihood_from_range(
        score(own, other, d)?,
        fuzzy_range(score(other, own, d)?, width, seed),
        aggression,
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Status {
    InCombat,
    NotReady,
    Engaging,
    EngagingAnother,
    Ready,
    NotEnoughIntel,
    VeryLikely,
    Likely,
    Unlikely,
    VeryUnlikely,
    TooStrong,
}

impl Status {
    fn key(self) -> &'static str {
        match self {
            Self::InCombat => "NAVAL_STRIKE_FORCE_STATUS_IN_COMBAT",
            Self::NotReady => "NAVAL_STRIKE_FORCE_STATUS_NOT_READY",
            Self::Engaging => "NAVAL_STRIKE_FORCE_STATUS_ENGAGING",
            Self::EngagingAnother => "NAVAL_STRIKE_FORCE_STATUS_ENGAGING_ANOTHER",
            Self::Ready => "NAVAL_STRIKE_FORCE_STATUS_READY",
            Self::NotEnoughIntel => "NAVAL_STRIKE_FORCE_STATUS_NOT_ENOUGH_INTEL",
            Self::VeryLikely => "NAVAL_STRIKE_FORCE_STATUS_VERY_LIKELY_TO_ENGAGE",
            Self::Likely => "NAVAL_STRIKE_FORCE_STATUS_LIKELY_TO_ENGAGE",
            Self::Unlikely => "NAVAL_STRIKE_FORCE_STATUS_UNLIKELY_TO_ENGAGE",
            Self::VeryUnlikely => "NAVAL_STRIKE_FORCE_STATUS_VERY_UNLIKELY_TO_ENGAGE",
            Self::TooStrong => "NAVAL_STRIKE_FORCE_STATUS_ENEMY_TOO_STRONG",
        }
    }
}

fn stable_status(combats: i32, detached: i32, mission: u32) -> Option<Status> {
    if combats > 0 {
        Some(Status::InCombat)
    } else if detached != 0 || mission != 2 {
        Some(Status::NotReady)
    } else {
        None
    }
}

fn engagement_status(reference: u64, resolves: bool, target: u64) -> Option<Status> {
    (reference != 0 && resolves).then_some(if reference == target {
        Status::Engaging
    } else {
        Status::EngagingAnother
    })
}

fn probability_status(probability: i64, d: &Defines) -> Result<(Status, bool), String> {
    if probability > 99_999 {
        return Ok((Status::Ready, false));
    }
    for (threshold, status) in
        d.likelihood
            .iter()
            .zip([Status::VeryLikely, Status::Likely, Status::Unlikely])
    {
        if probability >= *threshold.as_ref().map_err(Clone::clone)? {
            return Ok((status, true));
        }
    }
    Ok((
        if probability >= 1 {
            Status::VeryUnlikely
        } else {
            Status::TooStrong
        },
        true,
    ))
}

fn permits_types<'a>(
    types: impl IntoIterator<Item = Result<&'a [u8], String>>,
    avoided: &[Option<Vec<u8>>],
) -> Result<bool, String> {
    if avoided.is_empty() {
        return Ok(true);
    }
    for spelling in types {
        let spelling = spelling?;
        for name in avoided {
            let name = name
                .as_ref()
                .ok_or("avoided ship-type string is unreadable")?;
            if name == spelling {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

fn force<'a>(context: &Context<'a>, id: u64) -> Result<&'a TaskForceRow, String> {
    let rows = context
        .frame
        .complete::<tables::TaskForces>()
        .map_err(|e| e.to_string())?;
    rows.binary_search_by_key(&id, |row| row.id)
        .map(|i| &rows[i])
        .map_err(|_| format!("TaskForces task force {id} is absent"))
}

fn input<'a>(context: &Context<'a>, id: u64) -> Result<&'a TaskForceCounterInputRow, String> {
    let rows = context
        .frame
        .complete::<tables::TaskForceCounterInputs>()
        .map_err(|e| e.to_string())?;
    rows.binary_search_by_key(&id, |row| row.task_force)
        .map(|i| &rows[i])
        .map_err(|_| format!("TaskForceCounterInputs task force {id} is absent"))
}

fn mission<'a>(
    context: &Context<'a>,
    input: &'a TaskForceCounterInputRow,
) -> Result<&'a NavalPortMissionInputs, String> {
    let inputs = input
        .port_mission
        .require("TaskForceCounterInputs.port_mission")?
        .as_ref()
        .ok_or(
            "TaskForceCounterInputs.port_mission: task force port mission sample is unreadable",
        )?;
    if !inputs.sample_hour.fresh_at(context.frame.hour) {
        return Err(format!(
            "task force {} port mission sample is from hour {}, not {}",
            input.task_force, inputs.sample_hour, context.frame.hour
        ));
    }
    Ok(inputs)
}

fn roster<'a>(
    context: &Context<'a>,
    target: u64,
) -> Result<Vec<(&'a ShipRow, &'a ShipModelRow)>, String> {
    let ships = context
        .frame
        .complete::<tables::Ships>()
        .map_err(|e| e.to_string())?;
    let ships = ships
        .iter()
        .filter(|ship| ship.task_force == target)
        .collect::<Vec<_>>();
    if ships.is_empty() {
        return Ok(Vec::new());
    }
    let models = context
        .frame
        .complete::<tables::ShipModels>()
        .map_err(|e| e.to_string())?;
    let mut result = ships
        .into_iter()
        .map(|ship| {
            let model = models
                .binary_search_by_key(&ship.id, |row| row.ship)
                .map(|i| &models[i])
                .map_err(|_| format!("ShipModels ship {} is absent", ship.id))?;
            model
                .counter
                .as_ref()
                .ok_or_else(|| format!("ship {} counter operands are unreadable", ship.id))?;
            Ok((ship, model))
        })
        .collect::<Result<Vec<_>, String>>()?;
    result.sort_unstable_by_key(|(_, model)| model.counter.as_ref().unwrap().roster_ordinal);
    for (ordinal, (_, model)) in result.iter().enumerate() {
        if model.counter.as_ref().unwrap().roster_ordinal != ordinal as u32 {
            return Err(format!(
                "task force {target} ship roster has missing or duplicate ordinals"
            ));
        }
    }
    Ok(result)
}

const STRIKE_UNRECORDED: &str = "the ships' strike operands (light, heavy, piercing and torpedo attack and the torpedo cooldown modifier) are not recorded, so the strike-force readiness is unknown";

fn power(context: &Context<'_>, target: u64) -> Result<Power, String> {
    if roster(context, target)?.is_empty() {
        return Ok(Power::default());
    }
    Err(STRIKE_UNRECORDED.into())
}

fn avoidance(
    context: &Context<'_>,
    target: u64,
    input: &NavalPortMissionInputs,
) -> Result<bool, String> {
    let avoided = input
        .avoided_ship_types
        .as_ref()
        .ok_or("avoided ship types are unreadable")?;
    if avoided.is_empty() {
        return Ok(true);
    }
    let roster = roster(context, target)?;
    if roster.is_empty() {
        return Ok(true);
    }
    let tokens = context
        .frame
        .complete::<tables::LexicalTokens>()
        .map_err(|e| e.to_string())?;
    permits_types(
        roster.into_iter().map(|(ship, _)| {
            let token = ship.class_id as u32;
            let spelling = tokens
                .binary_search_by_key(&token, |row| row.token)
                .map(|i| &tokens[i].literal)
                .map_err(|_| format!("ship {} sub-unit token {token} is absent", ship.id))?;
            Ok(spelling.as_slice())
        }),
        avoided,
    )
}

fn status(
    context: &Context<'_>,
    own: &TaskForceRow,
    target: u64,
    visibility: u16,
) -> Result<String, String> {
    if own.combat_references > 0 {
        return localise(context.loc, Status::InCombat.key(), &[]);
    }
    let input = input(context, own.id)?;
    if let Some(status) = stable_status(0, input.detached_activity, own.mission) {
        return localise(context.loc, status.key(), &[]);
    }
    let mission = mission(context, input)?;
    let engagement = mission
        .engagement_target
        .as_ref()
        .ok_or("strike-force target reference is unreadable")?;
    if engagement.reference != 0 {
        let resolves = engagement
            .resolves
            .ok_or("strike-force target registry lookup is unreadable")?;
        if let Some(status) = engagement_status(engagement.reference, resolves, target) {
            return localise(context.loc, status.key(), &[]);
        }
    }
    if input.engagement_rule == 4 {
        return localise(context.loc, Status::Ready.key(), &[]);
    }
    if visibility < 100 {
        return localise(context.loc, Status::NotEnoughIntel.key(), &[]);
    }
    let content = context
        .content
        .ok_or("port strike-force content is unavailable")?;
    let d = &content.port_mission_defines;
    let own_power = power(context, own.id)?;
    let target_power = power(context, target)?;
    let aggression = if input.running_away != 0 {
        0
    } else {
        usize::try_from(input.engagement_rule)
            .ok()
            .and_then(|i| content.aggression.get(i).copied())
            .ok_or_else(|| {
                format!(
                    "NNavy.AGGRESSION_SETTINGS_VALUES lacks native index {}",
                    input.engagement_rule
                )
            })?
    };
    let (status, explain) = if visibility < 250 {
        let width = *d.estimate_width[usize::from(visibility > 149)]
            .as_ref()
            .map_err(Clone::clone)?;
        probability_status(
            fuzzy_likelihood(
                own_power,
                target_power,
                aggression,
                width,
                (target >> 32) as i32,
                d,
            )?,
            d,
        )?
    } else if exact_ready(own_power, target_power, aggression, d)?
        && avoidance(context, target, mission)?
    {
        (Status::Ready, false)
    } else {
        (Status::TooStrong, true)
    };
    let mut text = localise(context.loc, status.key(), &[])?;
    if explain {
        let key = format!(
            "SHIP_ENGAGEMENT_BUTTON_AGGRESSIVENESS_{}",
            input.engagement_rule.wrapping_add(1)
        );
        let setting = localise(context.loc, &key, &[])?;
        let setting = localise(context.loc, &setting, &[])?;
        text.push('\n');
        text.push_str(&localise(
            context.loc,
            "NAVAL_STRIKE_FORCE_STATUS_ENGAGEMENT_SETTING_DESC",
            &[("SETTING", Arg::Text(&setting))],
        )?);
    }
    Ok(text)
}

fn entries<'a>(context: &Context<'a>, target: u64) -> Result<Vec<&'a TaskForceRow>, String> {
    let countries = context
        .frame
        .complete::<tables::Countries>()
        .map_err(|e| e.to_string())?;
    let viewer = countries
        .iter()
        .find(|row| row.canonical_slot == context.vision.ours())
        .ok_or("current viewing country raw id is unavailable")?
        .id;
    if viewer as i32 <= 0 {
        return Ok(Vec::new());
    }
    let target = force(context, target)?;
    if target.province == 0 {
        return Ok(Vec::new());
    }
    let content = context
        .content
        .ok_or("port strike-force region content is unavailable")?;
    let Some(region) = content.region_of.get(&(target.province)).copied() else {
        return Ok(Vec::new());
    };
    let ledgers = context
        .frame
        .complete::<tables::NavalMissionLedgers>()
        .map_err(|e| e.to_string())?;
    let Some(ledger) = ledgers
        .iter()
        .find(|row| row.owner == viewer && row.region == region)
    else {
        return Ok(Vec::new());
    };
    let mut entries = Vec::new();
    for &id in &ledger.task_forces {
        let own = force(context, id)?;
        if own.mission == 2 {
            entries.push(own);
        }
    }
    Ok(entries)
}

pub(super) fn append(context: &Context<'_>, target: u64, visibility: u16, out: &mut Tooltip) {
    let entries = match entries(context, target) {
        Ok(entries) => entries,
        Err(reason) => {
            refuse(out, "port strike-force ledger", reason);
            return;
        }
    };
    if entries.is_empty() {
        return;
    }
    ensure_newlines(&mut out.title, 2);
    let header = localise(context.loc, "NAVAL_STRIKE_FORCE_STATUS_HEADER", &[])
        .map(|header| format!("{header}\n"));
    super::append(out, context, false, "port strike-force header", header);
    for own in entries {
        let entry = input(context, own.id)
            .and_then(|row| {
                dashboard_text::unit_names::task_force_name(
                    row,
                    context.loc,
                    context.recorded_language,
                )
            })
            .and_then(|name| {
                localise(
                    context.loc,
                    "NAVAL_STRIKE_FORCE_STATUS_ENTRY",
                    &[("NAME", Arg::Text(&name))],
                )
            });
        super::append(out, context, false, "port strike-force name", entry);
        super::append(
            out,
            context,
            false,
            "port strike-force status",
            status(context, own, target, visibility),
        );
        if !out.title.is_empty() && !out.title.ends_with('\n') {
            out.title.push('\n');
        }
    }
}

fn searcher(context: &Context<'_>, first: u64) -> Result<Option<u64>, String> {
    let packet = mission(context, input(context, first)?)?;
    let entries = packet
        .searching
        .as_ref()
        .ok_or("searching reference vector is unreadable")?;
    if entries.is_empty() {
        return Ok(None);
    }
    let countries = context
        .frame
        .complete::<tables::Countries>()
        .map_err(|e| e.to_string())?;
    let viewer = countries
        .iter()
        .find(|row| row.canonical_slot == context.vision.ours())
        .ok_or("current viewing country raw id is unavailable")?
        .id;
    for (ordinal, entry) in entries.iter().enumerate() {
        let reference = entry
            .reference
            .ok_or_else(|| format!("searching reference slot {ordinal} is unreadable"))?;
        if reference == 0 {
            return Err(format!(
                "native searching slot {ordinal} reference {reference} is null or dangling"
            ));
        }
        match entry.resolves {
            Some(true) => (),
            Some(false) => {
                return Err(format!(
                    "native searching slot {ordinal} reference {reference} is null or dangling"
                ));
            }
            None => {
                return Err(format!(
                    "searching slot {ordinal} registry lookup is unreadable"
                ));
            }
        }
        let owner = entry
            .roster_owner
            .ok_or_else(|| format!("searching slot {ordinal} roster owner is unreadable"))?;
        if owner == viewer
            || (owner != 0 && viewer != 0 && context.slot(owner)? == context.vision.ours())
        {
            return Ok(Some(reference));
        }
    }
    Ok(None)
}

pub(super) fn append_searching(
    context: &Context<'_>,
    first: u64,
    visibility: u16,
    out: &mut Tooltip,
) {
    let selected = match searcher(context, first) {
        Ok(Some(id)) => id,
        Ok(None) => return,
        Err(reason) => {
            refuse(out, "port task force searching references", reason);
            return;
        }
    };
    let key = if visibility < 100 {
        "NAVAL_SPOTTING_LEVEL_NONE_DESC"
    } else if visibility < 150 {
        "NAVAL_SPOTTING_LEVEL_LOW_DESC"
    } else if visibility < 250 {
        "NAVAL_SPOTTING_LEVEL_MEDIUM_DESC"
    } else {
        "NAVAL_SPOTTING_LEVEL_DETAILED_UNITS_DESC"
    };
    super::append(
        out,
        context,
        false,
        "port task force spotting level",
        localise(context.loc, key, &[]).map(|text| format!("{text}\n")),
    );
    let name = input(context, selected)
        .and_then(|row| {
            dashboard_text::unit_names::task_force_name(row, context.loc, context.recorded_language)
        })
        .and_then(|name| {
            localise(
                context.loc,
                "NAVAL_SPOTTING_SEARCHED_BY",
                &[("NAME", Arg::Text(&name))],
            )
        })
        .map(|text| format!("{text}\n"));
    super::append(out, context, false, "port task force searching name", name);
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::{
        rows::CountryLinkRow, rows::CountryRow, rows::NavalEngagementTarget, rows::NavalSearcher,
        rows::ShipCounterInputs, rows::TableRows,
    };
    use viewer_foundation::test_support::{test_frame, test_put};

    fn with_context(frame: &DecodedFrame, run: impl FnOnce(&Context<'_>)) {
        let mut loc: Localisation = [
            ("NAVAL_SPOTTING_LEVEL_NONE_DESC".into(), "none".into()),
            ("NAVAL_SPOTTING_LEVEL_LOW_DESC".into(), "low".into()),
            ("NAVAL_SPOTTING_LEVEL_MEDIUM_DESC".into(), "medium".into()),
            (
                "NAVAL_SPOTTING_LEVEL_DETAILED_UNITS_DESC".into(),
                "detailed".into(),
            ),
            (
                "NAVAL_SPOTTING_SEARCHED_BY".into(),
                "searched by $NAME$".into(),
            ),
            (Status::InCombat.key().into(), "combat".into()),
            (Status::NotReady.key().into(), "not ready".into()),
            (Status::Engaging.key().into(), "engaging".into()),
            (Status::EngagingAnother.key().into(), "other target".into()),
            (Status::Ready.key().into(), "ready".into()),
            (Status::NotEnoughIntel.key().into(), "no intel".into()),
        ]
        .into_iter()
        .collect();
        loc.language = "english".into();
        super::super::task_force::tests::with_test_context(
            frame,
            &loc,
            "unreached",
            None,
            |context| {
                run(&context);
            },
        );
    }

    fn packet(entries: Vec<NavalSearcher>) -> NavalPortMissionInputs {
        NavalPortMissionInputs {
            sample_hour: replay_data::rows::PacketHour::Stamped(7),
            searching: Some(entries),
            ..Default::default()
        }
    }

    fn task_input(packet: NavalPortMissionInputs) -> TaskForceCounterInputRow {
        TaskForceCounterInputRow {
            task_force: 50,
            port_mission: Some(packet).into(),
            ..Default::default()
        }
    }

    fn set_searchers(frame: &mut DecodedFrame, entries: Vec<NavalSearcher>) {
        test_put(
            frame,
            TableRows::TaskForceCounterInputs(vec![task_input(packet(entries))]),
        );
    }

    #[test]
    fn searching_preserves_first_raw_or_canonical_match_and_unknown_prefixes() {
        let mut frame = test_frame(7);
        test_put(
            &mut frame,
            TableRows::Countries(vec![CountryRow {
                id: 1,
                canonical_slot: 1,
                ..Default::default()
            }]),
        );
        test_put(
            &mut frame,
            TableRows::CountryLinks(vec![
                CountryLinkRow {
                    id: 2,
                    canonical_slot: 2,
                },
                CountryLinkRow {
                    id: 99,
                    canonical_slot: 1,
                },
            ]),
        );
        let nonmatch = NavalSearcher {
            reference: Some(100),
            resolves: Some(true),
            roster_owner: Some(2),
        };
        let alias = NavalSearcher {
            reference: Some(101),
            resolves: Some(true),
            roster_owner: Some(99),
        };
        set_searchers(&mut frame, vec![nonmatch, alias, NavalSearcher::default()]);
        with_context(&frame, |context| {
            assert_eq!(searcher(context, 50).unwrap(), Some(101))
        });
        for (entry, fault) in [
            (NavalSearcher::default(), "reference slot 0"),
            (
                NavalSearcher {
                    reference: Some(0),
                    ..Default::default()
                },
                "null or dangling",
            ),
            (
                NavalSearcher {
                    reference: Some(102),
                    resolves: Some(false),
                    ..Default::default()
                },
                "null or dangling",
            ),
            (
                NavalSearcher {
                    reference: Some(102),
                    ..Default::default()
                },
                "registry lookup",
            ),
            (
                NavalSearcher {
                    reference: Some(102),
                    resolves: Some(true),
                    ..Default::default()
                },
                "roster owner",
            ),
        ] {
            set_searchers(&mut frame, vec![entry, alias]);
            with_context(&frame, |context| {
                assert!(searcher(context, 50).unwrap_err().contains(fault))
            });
        }
        let mut raw_frame = test_frame(7);
        test_put(
            &mut raw_frame,
            TableRows::Countries(vec![CountryRow {
                id: 1,
                canonical_slot: 1,
                ..Default::default()
            }]),
        );
        set_searchers(
            &mut raw_frame,
            vec![
                NavalSearcher {
                    reference: Some(101),
                    resolves: Some(true),
                    roster_owner: Some(1),
                },
                NavalSearcher::default(),
            ],
        );
        with_context(&raw_frame, |context| {
            assert_eq!(searcher(context, 50).unwrap(), Some(101))
        });
    }

    #[test]
    fn searching_sample_is_current_and_empty_list_demands_no_roster() {
        let mut frame = test_frame(7);
        set_searchers(&mut frame, Vec::new());
        with_context(&frame, |context| {
            assert_eq!(searcher(context, 50).unwrap(), None)
        });
        let stale = task_input(NavalPortMissionInputs {
            sample_hour: replay_data::rows::PacketHour::Stamped(6),
            searching: Some(Vec::new()),
            ..Default::default()
        });
        test_put(&mut frame, TableRows::TaskForceCounterInputs(vec![stale]));
        with_context(&frame, |context| {
            assert!(
                searcher(context, 50)
                    .unwrap_err()
                    .contains("from hour 6, not 7")
            )
        });
        test_put(
            &mut frame,
            TableRows::TaskForceCounterInputs(vec![TaskForceCounterInputRow {
                task_force: 50,
                ..Default::default()
            }]),
        );
        with_context(&frame, |context| {
            let reason = searcher(context, 50).unwrap_err();
            assert!(reason.contains("TaskForceCounterInputs.port_mission"));
            assert!(reason.contains("sample is unreadable"));
        });
        test_put(
            &mut frame,
            TableRows::TaskForceCounterInputs(vec![TaskForceCounterInputRow {
                task_force: 50,
                port_mission: replay_data::rows::Recorded::NotRecorded,
                ..Default::default()
            }]),
        );
        with_context(&frame, |context| {
            assert_eq!(
                searcher(context, 50).unwrap_err(),
                "TaskForceCounterInputs.port_mission not recorded"
            );
        });
    }

    #[test]
    fn searching_text_keeps_visibility_thresholds_current_name_and_unconditional_newlines() {
        let mut frame = test_frame(7);
        test_put(
            &mut frame,
            TableRows::Countries(vec![CountryRow {
                id: 1,
                canonical_slot: 1,
                ..Default::default()
            }]),
        );
        test_put(
            &mut frame,
            TableRows::TaskForceCounterInputs(vec![
                task_input(packet(vec![NavalSearcher {
                    reference: Some(101),
                    resolves: Some(true),
                    roster_owner: Some(1),
                }])),
                TaskForceCounterInputRow {
                    task_force: 101,
                    name: Some(b"patrol".to_vec()).into(),
                    ..Default::default()
                },
            ]),
        );
        with_context(&frame, |context| {
            for (visibility, level) in [
                (99, "none"),
                (100, "low"),
                (149, "low"),
                (150, "medium"),
                (249, "medium"),
                (250, "detailed"),
            ] {
                let mut text = Tooltip {
                    title: "existing".into(),
                    ..Default::default()
                };
                append_searching(context, 50, visibility, &mut text);
                assert_eq!(text.title, format!("existing{level}\nsearched by patrol\n"));
                assert!(text.body.is_empty() && text.refused.is_empty());
            }
        });
    }

    #[test]
    fn native_status_early_arms_consume_only_reached_current_fields() {
        let mut frame = test_frame(7);
        let own = TaskForceRow {
            id: 50,
            mission: 2,
            ..Default::default()
        };
        with_context(&frame, |context| {
            assert_eq!(
                status(
                    context,
                    &TaskForceRow {
                        combat_references: 1,
                        ..own.clone()
                    },
                    101,
                    250
                )
                .unwrap(),
                "combat"
            )
        });
        for (detached, mission_kind) in [(1, 2), (0, 3)] {
            test_put(
                &mut frame,
                TableRows::TaskForceCounterInputs(vec![TaskForceCounterInputRow {
                    task_force: 50,
                    detached_activity: detached,
                    ..Default::default()
                }]),
            );
            with_context(&frame, |context| {
                assert_eq!(
                    status(
                        context,
                        &TaskForceRow {
                            mission: mission_kind,
                            ..own.clone()
                        },
                        101,
                        250
                    )
                    .unwrap(),
                    "not ready"
                )
            });
        }
        for (target, rule, intel, expected) in [
            (
                NavalEngagementTarget {
                    reference: 101,
                    resolves: Some(true),
                },
                0,
                0,
                "engaging",
            ),
            (
                NavalEngagementTarget {
                    reference: 102,
                    resolves: Some(true),
                },
                0,
                0,
                "other target",
            ),
            (
                NavalEngagementTarget {
                    reference: 0,
                    resolves: None,
                },
                4,
                0,
                "ready",
            ),
            (
                NavalEngagementTarget {
                    reference: 101,
                    resolves: Some(false),
                },
                4,
                0,
                "ready",
            ),
            (
                NavalEngagementTarget {
                    reference: 0,
                    resolves: None,
                },
                0,
                99,
                "no intel",
            ),
        ] {
            test_put(
                &mut frame,
                TableRows::TaskForceCounterInputs(vec![TaskForceCounterInputRow {
                    engagement_rule: rule,
                    ..task_input(NavalPortMissionInputs {
                        sample_hour: replay_data::rows::PacketHour::Stamped(7),
                        engagement_target: Some(target),
                        ..Default::default()
                    })
                }]),
            );
            with_context(&frame, |context| {
                assert_eq!(status(context, &own, 101, intel).unwrap(), expected)
            });
        }
        test_put(
            &mut frame,
            TableRows::TaskForceCounterInputs(vec![TaskForceCounterInputRow {
                engagement_rule: 4,
                ..task_input(NavalPortMissionInputs {
                    sample_hour: replay_data::rows::PacketHour::Stamped(7),
                    engagement_target: Some(NavalEngagementTarget {
                        reference: 101,
                        resolves: None,
                    }),
                    ..Default::default()
                })
            }]),
        );
        with_context(&frame, |context| {
            assert!(
                status(context, &own, 101, 250)
                    .unwrap_err()
                    .contains("registry lookup")
            )
        });
    }

    #[test]
    fn ship_power_refuses_without_strike_words_and_empty_rosters_stay_zero() {
        let mut frame = test_frame(7);
        test_put(
            &mut frame,
            TableRows::Ships(vec![ShipRow {
                id: 10,
                task_force: 50,
                ..Default::default()
            }]),
        );
        test_put(
            &mut frame,
            TableRows::ShipModels(vec![ShipModelRow {
                ship: 10,
                counter: Some(ShipCounterInputs {
                    roster_ordinal: 0,
                    ..Default::default()
                }),
                ..Default::default()
            }]),
        );
        with_context(&frame, |context| {
            assert_eq!(super::power(context, 50), Err(STRIKE_UNRECORDED.into()));
            assert_eq!(super::power(context, 51), Ok(Power::default()));
        });
        let mut empty = test_frame(7);
        test_put(&mut empty, TableRows::Ships(Vec::new()));
        with_context(&empty, |context| {
            assert_eq!(super::power(context, 50), Ok(Power::default()))
        });
    }

    fn score_defines() -> Defines {
        Defines {
            score: Ok(ScoreDefines {
                screening_ratio: Ok(400_000),
                efficiency: [SCALE, 0, SCALE, 0, SCALE, 0],
            }),
            ..thresholds([Ok(80_000), Ok(60_000), Ok(20_000)])
        }
    }

    fn power() -> Power {
        Power {
            ships: 1,
            light: SCALE,
            light_piercing: 1,
            strength: SCALE,
            maximum_strength: SCALE,
            organisation: SCALE,
            maximum_organisation: SCALE,
            ..Power::default()
        }
    }

    fn thresholds(values: [Result<i64, String>; 3]) -> Defines {
        Defines {
            score: Err("unreached score".into()),
            estimate_width: [Err("unreached low".into()), Err("unreached medium".into())],
            likelihood: values,
        }
    }

    #[test]
    fn fixed5_guard_uses_unsigned32_sentinel_and_wrapping_products() {
        for n in [i64::MIN, i64::MIN + 1, i64::MAX] {
            assert_eq!(fraction(n, SCALE).unwrap(), 4_294_967_295);
        }
        assert_eq!(fraction(1, 0).unwrap(), 4_294_967_295);
        assert_eq!(fraction(-5, 3).unwrap(), -166_666);
        assert_eq!(multiply(i64::MAX, 2), 0);
        assert!(fraction(1_i64 << 58, -1).is_err());
    }

    #[test]
    fn score_preserves_carrier_precedence_health_and_capitals_over_screens() {
        let d = score_defines();
        let carrier = Power {
            ships: 1,
            carriers: 1,
            ..Power::default()
        };
        assert_eq!(score(carrier, Power::default(), &d).unwrap(), 100_000_000);
        assert_eq!(
            score(
                Power {
                    light_piercing: 0,
                    ..power()
                },
                carrier,
                &d
            )
            .unwrap(),
            0
        );
        let missing = thresholds([Ok(0), Ok(0), Ok(0)]);
        assert_eq!(
            score(carrier, Power::default(), &missing).unwrap(),
            100_000_000
        );
        assert_eq!(
            score(
                Power {
                    light_piercing: 0,
                    ..power()
                },
                carrier,
                &missing
            )
            .unwrap(),
            0
        );
        let mut own = Power {
            ships: 4,
            capitals: 1,
            light: 200_000,
            heavy: 600_000,
            torpedo: 100_000,
            ..power()
        };
        assert_eq!(score(own, power(), &d).unwrap(), 693_747);
        own.maximum_strength = 0;
        own.maximum_organisation = 0;
        assert_eq!(score(own, power(), &d).unwrap(), 173_436);
        let mut missing_screen = d;
        missing_screen.score.as_mut().unwrap().screening_ratio = Err("unreached screening".into());
        assert_eq!(
            score(
                Power {
                    capitals: 1,
                    ..power()
                },
                power(),
                &missing_screen
            )
            .unwrap(),
            SCALE
        );
        assert_eq!(
            score(power(), power(), &missing_screen).unwrap_err(),
            "unreached screening"
        );
    }

    #[test]
    fn exact_readiness_is_strict_and_caps_the_enemy_ratio() {
        let d = score_defines();
        assert!(!exact_ready(power(), power(), SCALE, &d).unwrap());
        assert!(exact_ready(power(), power(), SCALE + 1, &d).unwrap());
        let powerless = Power {
            light: 0,
            ..power()
        };
        let enemy = Power {
            light: 2_000_000,
            ..power()
        };
        assert!(!exact_ready(powerless, enemy, 1_000_000, &d).unwrap());
        assert!(exact_ready(powerless, enemy, 1_000_001, &d).unwrap());
    }

    #[test]
    fn fuzzy_range_keeps_hash_packing_parity_and_signed_seed() {
        assert_eq!(fuzzy_range(1_000_000, 25, 0), (763_401, 1_263_401));
        assert_eq!(fuzzy_range(1_000_000, 25, 1), (619_796, 1_119_796));
        assert_eq!(
            fuzzy_range(1_000_010, 10, 0x1234_5678),
            (930_023, 1_130_025)
        );
        assert_eq!(fuzzy_range(0, 0, 0), (0, 200));
        assert_eq!(fuzzy_range(500, 33, -1), (277, 607));
    }

    #[test]
    fn fuzzy_probability_keeps_endpoint_and99percent_rules() {
        let range = (50_000, 150_000);
        assert_eq!(likelihood_from_range(SCALE, range, 50_000).unwrap(), 0);
        assert_eq!(likelihood_from_range(SCALE, range, SCALE).unwrap(), 50_000);
        assert_eq!(
            likelihood_from_range(SCALE, range, 150_000).unwrap(),
            99_000
        );
        assert_eq!(likelihood_from_range(SCALE, range, 150_001).unwrap(), SCALE);
        assert_eq!(likelihood_from_range(0, range, 1_000_000).unwrap(), 0);
        assert_eq!(likelihood_from_range(0, range, 1_000_001).unwrap(), SCALE);
    }

    #[test]
    fn status_precedence_distinguishes_dangling_target_and_exact_full_identity() {
        assert_eq!(stable_status(1, 1, 3), Some(Status::InCombat));
        assert_eq!(stable_status(0, 1, 2), Some(Status::NotReady));
        assert_eq!(stable_status(0, 0, 3), Some(Status::NotReady));
        assert_eq!(stable_status(0, 0, 2), None);
        let target = 0x0000_007b_0000_0002;
        assert_eq!(engagement_status(target, false, target), None);
        assert_eq!(engagement_status(0, true, target), None);
        assert_eq!(
            engagement_status(target, true, target),
            Some(Status::Engaging)
        );
        assert_eq!(
            engagement_status(target ^ 1, true, target),
            Some(Status::EngagingAnother)
        );
    }

    #[test]
    fn likelihood_thresholds_are_ordered_and_only_reached_values_are_demanded() {
        let d = thresholds([
            Ok(50_000),
            Err("unreached likely".into()),
            Err("unreached unlikely".into()),
        ]);
        assert_eq!(
            probability_status(50_000, &d).unwrap(),
            (Status::VeryLikely, true)
        );
        assert_eq!(
            probability_status(SCALE, &d).unwrap(),
            (Status::Ready, false)
        );
        assert_eq!(
            probability_status(49_999, &d).unwrap_err(),
            "unreached likely"
        );
        let d = thresholds([Ok(80_000), Ok(60_000), Ok(20_000)]);
        for (p, expected) in [
            (80_000, Status::VeryLikely),
            (60_000, Status::Likely),
            (20_000, Status::Unlikely),
            (1, Status::VeryUnlikely),
            (0, Status::TooStrong),
        ] {
            assert_eq!(probability_status(p, &d).unwrap(), (expected, true));
        }
    }

    #[test]
    fn avoidance_preserves_decisive_prefixes_and_empty_list_laziness() {
        let unconsumed = std::iter::once_with(|| -> Result<&[u8], String> {
            panic!("an empty exclusion list reads no target types")
        });
        assert!(permits_types(unconsumed, &[]).unwrap());
        let types = [Ok(b"screen".as_slice()), Err("unreached later ship".into())];
        assert!(!permits_types(types, &[Some(b"screen".to_vec()), None]).unwrap());
        assert_eq!(
            permits_types(
                [Ok(b"screen".as_slice())],
                &[None, Some(b"screen".to_vec())]
            )
            .unwrap_err(),
            "avoided ship-type string is unreadable"
        );
    }

    #[test]
    fn avoidance_uses_raw_bytes_without_utf8_or_case_folding() {
        assert!(permits_types([Ok(b"Screen".as_slice())], &[Some(b"screen".to_vec())]).unwrap());
        assert!(!permits_types([Ok([0xff, 0].as_slice())], &[Some(vec![0xff, 0])]).unwrap());
    }
}
