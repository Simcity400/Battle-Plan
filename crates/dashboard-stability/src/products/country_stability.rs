//! Displayed stability, war support and maximum command power over one
//! branch, and the breakdown behind them at a requested hour.
//!
//! The game computes displayed stability, war support and maximum command
//! power from the operands CountryStabilityInputs records. [`formula`] keeps
//! that arithmetic exact: wrapping i64, truncating division, Q15 conversions
//! and clamps. Each country's inputs row is the
//! latest slice read (SliceReads); a run carries it over the following hours
//! while the country keeps its identity and its row stays recorded, and
//! `carried` lists the hours whose values come from an earlier read (a
//! truncated read can even move that back by an hour); every other shown hour
//! was read at that hour. The maximum command power also takes operands from
//! the country's CountryCommandPower row at the same frame, read every hour,
//! so `carried` alone names the read of its base operand. The war predicates and world
//! tension join a read but do not force one, so between reads they are the
//! read's values.
//!
//! The breakdown lists the modifier tree lines of each tracked key with the
//! identity the recorder proved, or the game's display text (in the
//! recording's language) for an unpaired line, and an explicit remainder when
//! the lines do not add up to the recorded total.

use dashboard_base::{Source, catalogue::View, products::country_measure::FixedRun};
use dashboard_base::{
    products::LoadContext,
    products::MAX_PRODUCT_REPLY_BYTES,
    products::Product,
    products::politics_content::{PoliticsContent, politics_content},
    products::recorded_rows,
    products::validate_tag,
};
use dashboard_core::transport::json_len;
use dashboard_text::{
    idea_names::{IdeaCountry, IdeaName},
    identity::{Look, built_template, character_script_token},
    metadata::{mapped_slot, validate_campaign},
    scripted_text::ScriptedText,
};
use installed_content::characters::BuiltTemplate;
use replay::{
    container::Replay,
    frame::FrameDecoder,
    rows::{CountryStabilityInputRow, CountryStabilityLineRow, TableId, tables},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use ts_rs::TS;

pub struct CountryStability;

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CountryStabilityParams {
    pub branch_id: String,
    pub tags: Vec<String>,
    /// The hour whose breakdown to include; none for runs only.
    #[serde(default)]
    pub hour: Option<i32>,
    /// Only the breakdown at `hour`: the histories are left out, so hovering a
    /// chart does not move them again.
    #[serde(default)]
    pub breakdown_only: bool,
}

/// One additive term of a displayed value; `None` when an input it needs was
/// not recorded (a tracked modifier key the game did not register).
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct FormulaTerm {
    pub term: String,
    pub value_raw: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct FormulaBreakdown {
    /// The displayed value, fixed5.
    pub value_raw: Option<String>,
    pub terms: Vec<FormulaTerm>,
    /// Displayed minus the unclamped sum: the clamp's contribution.
    pub clamp_raw: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct BreakdownLine {
    /// The child's position in the modifier tree (the tooltip's order).
    pub line: u32,
    /// `unpaired`, `idea`, `countryLeader`, `ideology`, `advisor`, `static`,
    /// `staticScaled`, `dynamicModifier` or `dynamicModifierValues`.
    pub kind: String,
    /// Script identity (idea, advisor idea, ideology, static or dynamic
    /// modifier token); empty for a leader or an unpaired line.
    pub identity: String,
    /// Character id or dynamic modifier scope country; zero otherwise.
    pub identity_id: u32,
    /// Dynamic modifier scope state; zero otherwise.
    pub secondary_raw: String,
    pub value_raw: String,
    /// The recorded display text of an unpaired line, in `language`, as plain
    /// text (see [`display_text`]).
    pub text: Option<String>,
    /// A paired line's name in the viewer's language, from the content stack
    /// (idea, advisor or leader name, ideology, static or dynamic modifier);
    /// None when content does not name it or is not configured.
    pub label: Option<String>,
    /// What the source is, from content: for an idea in a slot or an advisor,
    /// the slot's name in the viewer's language (`Economy Law`, `Political
    /// Advisor`); None otherwise.
    pub group: Option<String>,
    /// For an idea in a category rather than a slot, the category's content
    /// type (`national_spirit` for spirits, whatever a mod names the category).
    pub group_kind: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct ModifierBreakdown {
    /// The modifier's script name.
    pub key: String,
    /// The recorded key index; none when the game registers no such key.
    pub index: Option<u32>,
    /// ModifierDefinitions' `good` flag (bit 1): only for good keys does
    /// every line equal its contribution to the total.
    pub good: Option<bool>,
    pub total_raw: Option<String>,
    pub lines: Vec<BreakdownLine>,
    /// Total minus the lines, when it is not zero.
    pub remainder_raw: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct StabilityBreakdown {
    pub hour: i32,
    /// The hour the inputs were read at.
    pub read_hour: i32,
    /// The game language of unpaired lines' text.
    pub language: String,
    pub stability: FormulaBreakdown,
    pub war_support: FormulaBreakdown,
    pub max_command_power: FormulaBreakdown,
    /// False when the lines were not recorded by the same read.
    pub lines_available: bool,
    pub modifiers: Vec<ModifierBreakdown>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct CountryStabilitySeries {
    pub tag: String,
    pub stability: Vec<FixedRun>,
    pub war_support: Vec<FixedRun>,
    pub max_command_power: Vec<FixedRun>,
    /// Shown hours whose values come from an earlier read, as consecutive
    /// hours with the same read, in hour order. Every other shown hour was read
    /// at that hour, so hourly reads cost nothing here.
    pub carried: Vec<ReadRun>,
    /// Whether any hour was read at all; false when no inputs were recorded.
    pub recorded: bool,
    pub breakdown: Option<StabilityBreakdown>,
}

/// Consecutive shown hours whose values come from one earlier read of the
/// country's inputs (its SliceReads hour).
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct ReadRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub read_hour: i32,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CountryStabilityReply {
    pub branch_id: String,
    /// Decimal places of every raw value in this reply.
    pub decimals: u8,
    pub series: Vec<CountryStabilitySeries>,
    pub warnings: Vec<String>,
}

impl Product for CountryStability {
    const KEY: &'static str = "country-stability";
    const CONTENT_DEPENDENT: bool = true;
    type Params = CountryStabilityParams;
    type Reply = CountryStabilityReply;

    fn types() -> Vec<String> {
        vec![
            CountryStabilityParams::decl(),
            FixedRun::decl(),
            ReadRun::decl(),
            FormulaTerm::decl(),
            FormulaBreakdown::decl(),
            BreakdownLine::decl(),
            ModifierBreakdown::decl(),
            StabilityBreakdown::decl(),
            CountryStabilitySeries::decl(),
            CountryStabilityReply::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        if params.tags.is_empty() {
            return Err("Invalid stability selection");
        }
        for tag in &params.tags {
            validate_tag(tag)?;
        }
        if params.branch_id.parse::<u64>().is_err() {
            return Err("Invalid stability branch");
        }
        Ok(())
    }

    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let branch = params.branch_id.parse::<u64>().expect("validated branch");
        let found = if params.breakdown_only {
            BTreeMap::new()
        } else {
            histories(source, branch, &params.tags, cancelled)?
        };
        let mut warnings = Vec::new();
        // The hour's breakdown is separate from shared immutable histories: it
        // neither changes the cache nor copies a country's history just to name
        // its formula terms. Only the returned public series clone their runs.
        let mut found_breakdowns = BTreeMap::new();
        if let Some(hour) = params.hour {
            found_breakdowns.extend(
                params
                    .tags
                    .iter()
                    .map(|tag| (tag.clone(), CountryStabilitySeries::default())),
            );
            // Lines are named in the viewer's language when content is configured;
            // without it they keep their identities and recorded text.
            let content = match context.content.as_ref() {
                Some(options) => match politics_content(source, options) {
                    Ok(content) => {
                        // Names from content carry its standing notes, such as an
                        // unverified recording or files left out.
                        warnings.extend(content.warnings.iter().cloned());
                        Some(content)
                    }
                    Err(error) => {
                        warnings.push(format!("Breakdown lines are not named: {error}"));
                        None
                    }
                },
                None => None,
            };
            breakdowns(
                source,
                branch,
                hour,
                &mut found_breakdowns,
                &mut warnings,
                content.as_deref(),
                cancelled,
            )?;
            // Line names left out for a localisation refusal, by reason and key.
            if let Some(content) = &content {
                warnings.extend(content.catalogue.refusal_lines());
            }
        }
        let series = params
            .tags
            .iter()
            .map(|tag| {
                // A tag named twice gets the same series both times.
                let mut found = found
                    .get(tag)
                    .map(|series| (**series).clone())
                    .unwrap_or_default();
                found.breakdown = found_breakdowns
                    .get(tag)
                    .and_then(|series| series.breakdown.clone());
                if !found.recorded && !params.breakdown_only {
                    warnings.push(format!(
                        "{tag}: no recorded stability inputs in this branch"
                    ));
                }
                CountryStabilitySeries {
                    tag: tag.clone(),
                    ..found
                }
            })
            .collect();
        let reply = CountryStabilityReply {
            branch_id: params.branch_id.clone(),
            decimals: dashboard_base::products::country_measure::MEASURE_DECIMALS,
            series,
            warnings,
        };
        if json_len(&reply, MAX_PRODUCT_REPLY_BYTES).is_none() {
            return Err(io::Error::other(format!(
                "StabilityReplyTooLarge: the complete stability reply exceeds {} MiB; no countries or runs were omitted",
                MAX_PRODUCT_REPLY_BYTES / (1024 * 1024),
            )));
        }
        Ok(reply)
    }
}

/// The same cached histories consumed by Stability, Command power and Scoreboard.
pub fn histories(
    source: &Source,
    branch: u64,
    tags: &[String],
    cancelled: &AtomicBool,
) -> io::Result<BTreeMap<String, Arc<CountryStabilitySeries>>> {
    // Histories do not depend on the inspected hour: they are scanned once
    // per (branch, country) and kept; the breakdown reads only its hour.
    let state = source.state::<Cache>();
    // One lock, with hash lookups for each distinct requested tag; holding it
    // through a missing-tag scan also prevents concurrent requests from scanning
    // the same histories twice. A small-widget request cannot evict Scoreboard's
    // set, and returning an Arc never deep-copies the runs.
    let mut cache = state.lock().unwrap_or_else(|error| error.into_inner());
    let entries = cache.entries.entry(branch).or_default();
    let requested: BTreeSet<&str> = tags.iter().map(String::as_str).collect();
    let missing: Vec<&str> = requested
        .iter()
        .copied()
        .filter(|tag| !entries.contains_key(*tag))
        .collect();
    if !missing.is_empty() {
        let (scanned, _) = scan(&source.replay(), branch, &missing, None, cancelled)?;
        for (tag, series) in scanned {
            entries.insert(tag, Arc::new(series));
        }
    }
    let mut found = BTreeMap::new();
    for tag in requested {
        if let Some(entry) = entries.get(tag) {
            found.insert(tag.to_owned(), Arc::clone(entry));
        }
    }
    Ok(found)
}

/// A recorded modifier definition's good flag (0x2): flags 3 means percentage
/// and good, 2 means number and good. [`dashboard_base::products::idea_format`] reads the same field.
const GOOD_FLAG: u32 = 0x2;

pub(super) struct BreakdownInput<'a> {
    pub decoded: &'a replay::frame::DecodedFrame,
    pub hour: i32,
    pub row: &'a CountryStabilityInputRow,
    pub slot: u32,
    pub read_hour: i32,
    pub reads: &'a [replay::rows::SliceReadRow],
    pub tag: &'a str,
}

/// The breakdown of one country at the frame's hour: formula terms and lines.
pub(super) fn breakdown_of(
    input: BreakdownInput<'_>,
    warnings: &mut Vec<String>,
) -> StabilityBreakdown {
    let BreakdownInput {
        decoded,
        hour,
        row,
        slot,
        read_hour: read,
        reads,
        tag,
    } = input;
    let read_hour = |table, owner| replay::sliced::read_hour(reads, table, owner);
    // SliceReads names an owner only while it has rows: a
    // country read with no lines has none.
    let lines_read = read_hour(TableId::CountryStabilityLines, row.country);
    let lines = decoded
        .complete::<tables::CountryStabilityLines>()
        .ok()
        .filter(|_| lines_read.is_none_or(|lines_read| lines_read == read));
    if lines.is_none() {
        warnings.push(format!(
            "{tag}: the stability lines of hour {} were not recorded with its inputs",
            hour
        ));
    }
    let definitions = decoded.complete::<tables::ModifierDefinitions>().ok();
    let good = |index: u32| {
        definitions.and_then(|rows| {
            rows.binary_search_by_key(&index, |row| row.index)
                .ok()
                .map(|at| rows[at].flags & GOOD_FLAG != 0)
        })
    };
    StabilityBreakdown {
        hour,
        read_hour: read,
        language: String::from_utf8_lossy(&row.language).into_owned(),
        stability: breakdown(formula::stability(row)),
        war_support: breakdown(formula::war_support(row)),
        max_command_power: breakdown(formula::max_command_power(
            row,
            formula::command_power(decoded, slot),
        )),
        lines_available: lines.is_some(),
        modifiers: {
            let mut refused = dashboard_text::catalogue::Refusals::default();
            let modifiers = modifiers(
                row,
                formula::command_power(decoded, slot),
                lines,
                good,
                tag,
                &mut refused,
            );
            warnings.extend(refused.lines("stability tree labels are"));
            modifiers
        },
    }
}

/// The engine's arithmetic over one inputs row.
pub mod formula {
    use replay::rows::{CountryCommandPowerRow, CountryStabilityInputRow};

    pub use replay::fixed_point::{fixed5_to_q15, q15_to_fixed5};

    fn clamp(value: i64, minimum: i64, maximum: i64) -> i64 {
        if value < minimum {
            minimum
        } else if value > maximum {
            maximum
        } else {
            value
        }
    }

    /// Named additive terms, the unclamped sum and the displayed value.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Terms {
        pub terms: Vec<(&'static str, Option<i64>)>,
        pub unclamped: Option<i64>,
        pub value: Option<i64>,
    }

    fn sum(terms: &[(&'static str, Option<i64>)]) -> Option<i64> {
        terms
            .iter()
            .try_fold(0_i64, |total, (_, term)| Some(total.wrapping_add((*term)?)))
    }

    /// The ruling-party popularity term. The overflow sentinel (u32::MAX when
    /// the quotient is i64::MAX, MIN or MIN + 1) is kept although a quotient by
    /// 100000 cannot reach it.
    pub fn popularity_term(factor: i64, popularity: i64) -> i64 {
        let product = factor.wrapping_mul(popularity) / 100_000;
        if [i64::MAX, i64::MIN, i64::MIN + 1].contains(&product) {
            0xffff_ffff
        } else {
            product.wrapping_mul(100_000) / 10_000_000
        }
    }

    /// Displayed stability from the recorded operands.
    pub fn stability(row: &CountryStabilityInputRow) -> Terms {
        let at_war = row.at_war != 0;
        let offensive = at_war && row.offensive_war != 0;
        let defensive = at_war && row.defensive_war != 0;
        let war = if offensive || defensive {
            (|| {
                let mut war = row
                    .war_stability_factor?
                    .wrapping_add(row.base_stability_war_factor);
                if offensive {
                    war = war.wrapping_add(row.offensive_war_stability_factor?);
                }
                if defensive {
                    war = war.wrapping_add(row.defensive_war_stability_factor?);
                }
                Some(war)
            })()
        } else {
            Some(0)
        };
        let popularity = row.party_popularity_stability_factor.map(|factor| {
            popularity_term(
                row.base_stability_party_popularity_factor
                    .wrapping_add(factor),
                row.ruling_party_popularity_raw,
            )
        });
        let terms = vec![
            ("base", Some(row.stability_raw)),
            ("war", war),
            ("popularity", popularity),
            (
                "propaganda",
                Some(q15_to_fixed5(row.propaganda_stability_q15)),
            ),
            ("stabilityFactor", row.stability_factor),
            (
                "coastalProtection",
                Some(row.coastal_protection_stability_raw),
            ),
        ];
        let unclamped = sum(&terms);
        Terms {
            value: unclamped.map(|value| clamp(value, row.min_stability, row.max_stability)),
            terms,
            unclamped,
        }
    }

    /// The war-support interception term: sum, Q15 conversion, scaling, fixed5
    /// conversion, then clamp to the impact.
    pub fn interception_term(row: &CountryStabilityInputRow) -> i64 {
        let scaled = fixed5_to_q15(row.interception_sum_raw)
            .wrapping_mul(row.interception_war_support_scale_q15)
            >> 15;
        let value = q15_to_fixed5(scaled);
        let impact = row.interception_bombing_war_support_impact;
        let lowest = impact.wrapping_neg();
        if value < lowest {
            lowest
        } else if value > impact {
            impact
        } else {
            value
        }
    }

    /// Displayed war support from the recorded operands.
    pub fn war_support(row: &CountryStabilityInputRow) -> Terms {
        let at_war = row.at_war != 0;
        let terms = vec![
            ("base", Some(row.war_support_raw)),
            (
                "offensiveWar",
                Some(if at_war && row.offensive_war != 0 {
                    row.war_support_offensive_war
                } else {
                    0
                }),
            ),
            (
                "defensiveWar",
                Some(if at_war && row.defensive_war != 0 {
                    row.war_support_defensive_war
                } else {
                    0
                }),
            ),
            (
                "tension",
                Some(
                    row.world_tension_raw
                        .wrapping_mul(row.war_support_tension_impact)
                        / 100_000,
                ),
            ),
            ("bombing", Some(q15_to_fixed5(row.bombing_war_support_q15))),
            ("interception", Some(interception_term(row))),
            ("heroes", Some(q15_to_fixed5(row.heroes_war_support_q15))),
            ("convoys", Some(q15_to_fixed5(row.convoys_war_support_q15))),
            (
                "propaganda",
                Some(q15_to_fixed5(row.propaganda_war_support_q15)),
            ),
            ("warSupportFactor", row.war_support_factor),
        ];
        let unclamped = sum(&terms);
        Terms {
            value: unclamped.map(|value| clamp(value, row.min_war_support, row.max_war_support)),
            terms,
            unclamped,
        }
    }

    /// The maximum command power the game shows and clamps the balance to,
    /// minus allocated command power. Not additive; the terms are its operands
    /// and nothing is clamped. `launch` is the country's CountryCommandPower row
    /// at the same frame ([`command_power`]), the one record of max_command_power,
    /// max_command_power_mult and allocated power; without it those terms are
    /// unknown, never zero.
    pub fn max_command_power(
        row: &CountryStabilityInputRow,
        launch: Option<&CountryCommandPowerRow>,
    ) -> Terms {
        let max = launch.and_then(|launch| launch.max_command_power);
        let mult = launch.and_then(|launch| launch.max_command_power_factor);
        let allocated = launch.map(|launch| launch.allocated_command_power_raw);
        let value = (|| {
            let base = row.base_max_command_power.wrapping_add(max?);
            Some(
                (mult?.wrapping_add(100_000).wrapping_mul(base) / 100_000).wrapping_sub(allocated?),
            )
        })();
        Terms {
            terms: vec![
                ("baseMaxCommandPower", Some(row.base_max_command_power)),
                ("maxCommandPower", max),
                ("maxCommandPowerMult", mult),
                ("allocatedCommandPower", allocated),
            ],
            unclamped: value,
            value,
        }
    }

    /// The CountryCommandPower row of the country at canonical `slot` in the
    /// same frame (the stability row's native country ID mapped through
    /// CountryLinks); `None` when the table is not complete or has no row.
    pub fn command_power(
        decoded: &replay::frame::DecodedFrame,
        slot: u32,
    ) -> Option<&CountryCommandPowerRow> {
        let rows = decoded
            .complete::<replay::rows::tables::CountryCommandPower>()
            .ok()?;
        rows.binary_search_by_key(&slot, |row| row.canonical_slot)
            .ok()
            .map(|at| &rows[at])
    }
}

pub(crate) fn breakdown(terms: formula::Terms) -> FormulaBreakdown {
    FormulaBreakdown {
        value_raw: terms.value.map(|value| value.to_string()),
        clamp_raw: terms
            .value
            .zip(terms.unclamped)
            .map(|(value, unclamped)| value.wrapping_sub(unclamped).to_string()),
        terms: terms
            .terms
            .into_iter()
            .map(|(term, value)| FormulaTerm {
                term: term.into(),
                value_raw: value.map(|value| value.to_string()),
            })
            .collect(),
    }
}

fn kind(kind: u8) -> &'static str {
    match kind {
        CountryStabilityLineRow::IDEA => "idea",
        CountryStabilityLineRow::COUNTRY_LEADER => "countryLeader",
        CountryStabilityLineRow::IDEOLOGY => "ideology",
        CountryStabilityLineRow::ADVISOR => "advisor",
        CountryStabilityLineRow::STATIC => "static",
        CountryStabilityLineRow::STATIC_SCALED => "staticScaled",
        CountryStabilityLineRow::DYNAMIC_MODIFIER => "dynamicModifier",
        CountryStabilityLineRow::DYNAMIC_MODIFIER_VALUES => "dynamicModifierValues",
        _ => "unpaired",
    }
}

/// The per-key lines of one read. `lines` is None when the lines table does
/// not hold this read. A tree label [`display_text`] refuses is noted in
/// `refused` by `tag` and line identity.
pub fn modifiers(
    row: &CountryStabilityInputRow,
    launch: Option<&replay::rows::CountryCommandPowerRow>,
    lines: Option<&[CountryStabilityLineRow]>,
    good: impl Fn(u32) -> Option<bool>,
    tag: &str,
    refused: &mut dashboard_text::catalogue::Refusals,
) -> Vec<ModifierBreakdown> {
    // The last two keys' totals are CountryCommandPower's, their one record.
    let stability = row.totals();
    let totals: [Option<i64>; 10] = std::array::from_fn(|index| match index {
        8 => launch.and_then(|launch| launch.max_command_power),
        9 => launch.and_then(|launch| launch.max_command_power_factor),
        index => stability[index],
    });
    modifiers_with_totals(row, totals, lines, good, tag, refused)
}

/// [`modifiers`] with each [`CountryStabilityInputRow::KEY_NAMES`] key's
/// total given.
pub fn modifiers_with_totals(
    row: &CountryStabilityInputRow,
    totals: [Option<i64>; 10],
    lines: Option<&[CountryStabilityLineRow]>,
    good: impl Fn(u32) -> Option<bool>,
    tag: &str,
    refused: &mut dashboard_text::catalogue::Refusals,
) -> Vec<ModifierBreakdown> {
    CountryStabilityInputRow::KEY_NAMES
        .iter()
        .zip(&row.keys)
        .zip(totals)
        .map(|((name, &index), total)| {
            let index = (index != u32::MAX).then_some(index);
            let rows: Vec<&CountryStabilityLineRow> = match (lines, index) {
                (Some(lines), Some(index)) => lines
                    .iter()
                    .filter(|line| line.country == row.country && line.key == index)
                    .collect(),
                _ => Vec::new(),
            };
            let sum = rows
                .iter()
                .fold(0_i64, |sum, line| sum.wrapping_add(line.value));
            ModifierBreakdown {
                key: String::from_utf8_lossy(name).into_owned(),
                index,
                good: index.and_then(&good),
                total_raw: total.map(|total| total.to_string()),
                remainder_raw: total
                    .filter(|_| lines.is_some())
                    .map(|total| total.wrapping_sub(sum))
                    .filter(|remainder| *remainder != 0)
                    .map(|remainder| remainder.to_string()),
                lines: rows
                    .into_iter()
                    .map(|line| {
                        let identity = String::from_utf8_lossy(&line.identity).into_owned();
                        let text = (line.kind == CountryStabilityLineRow::UNPAIRED)
                            .then(|| display_text(&line.text))
                            .and_then(|text| {
                                text.inspect_err(|reason| {
                                    refused.note(reason.clone(), format!("{tag} {identity}"))
                                })
                                .ok()
                            });
                        BreakdownLine {
                            line: line.line,
                            kind: kind(line.kind).into(),
                            identity,
                            identity_id: line.identity_id,
                            secondary_raw: line.secondary.to_string(),
                            value_raw: line.value.to_string(),
                            text,
                            label: None,
                            group: None,
                            group_kind: None,
                        }
                    })
                    .collect(),
            }
        })
        .collect()
}

/// Every requested country history stays shared for this opened source. Branch
/// and tag hash lookups avoid a linear scan of a large Scoreboard cache.
#[derive(Default)]
struct Cache {
    entries: HashMap<u64, HashMap<String, Arc<CountryStabilitySeries>>>,
}

/// Adds each series' breakdown at `hour`, read from that hour's frame alone.
fn breakdowns(
    source: &Source,
    branch: u64,
    hour: i32,
    series: &mut BTreeMap<String, CountryStabilitySeries>,
    warnings: &mut Vec<String>,
    content: Option<&PoliticsContent>,
    cancelled: &AtomicBool,
) -> io::Result<()> {
    // A hover moves on quickly: an abandoned request gives its thread back
    // before decoding a block.
    if cancelled.load(Ordering::Acquire) {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "Stability request cancelled",
        ));
    }
    let replay = source.replay();
    let decoded = match replay.seek(branch, hour)? {
        replay::container::SeekResult::Frame(frame) => frame,
        replay::container::SeekResult::Gap { reason, .. } => {
            warnings.push(format!("No stability breakdown at hour {hour}: {reason}"));
            return Ok(());
        }
    };
    if let Err(error) = validate_campaign(&decoded, &replay) {
        warnings.push(format!("No stability breakdown at hour {hour}: {error}"));
        return Ok(());
    }
    // The same identity checks as the history scan.
    let (Ok(countries), Ok(links), Ok(inputs), Ok(reads)) = (
        decoded.complete::<tables::Countries>(),
        decoded.complete::<tables::CountryLinks>(),
        recorded_rows::<tables::CountryStabilityInputs>(&decoded),
        decoded.complete::<tables::SliceReads>(),
    ) else {
        warnings.push(format!(
            "No stability breakdown at hour {hour}: its tables are incomplete"
        ));
        return Ok(());
    };
    // Read discovery and formula evaluation share this recorded-tag index.
    // The first row for a tag supplies its identity and link validation.
    let mut countries_by_tag = HashMap::new();
    for country in countries {
        countries_by_tag
            .entry(country.tag.as_slice())
            .or_insert(country);
    }
    // Characters are named as recorded at the read behind the values, not at the
    // hour asked for, so a breakdown depends only on its read (the viewer caches it
    // by read). A read whose own hour cannot be decoded leaves them unnamed.
    let mut read_frames: BTreeMap<i32, Option<Box<replay::frame::DecodedFrame>>> = BTreeMap::new();
    // The names are expanded in each country's scope at its read hour; a
    // tag scope there needs the exact native _CountriesForOriginalTags
    // snapshot at that read (dashboard_text::scope::OriginalTagLists).
    let read_hours: BTreeSet<i32> = if content.is_some() {
        series
            .keys()
            .filter_map(|tag| {
                let country = countries_by_tag.get(tag.as_bytes()).copied()?;
                mapped_slot(country, links).ok()?;
                let index = inputs
                    .binary_search_by_key(&country.id, |row| row.country)
                    .ok()?;
                replay::sliced::read_hour(
                    reads,
                    TableId::CountryStabilityInputs,
                    inputs[index].country,
                )
            })
            .collect()
    } else {
        BTreeSet::new()
    };
    let original_tags =
        dashboard_text::scope::OriginalTagLists::derive(&replay, branch, &read_hours, cancelled)?;
    for (tag, entry) in series.iter_mut() {
        let Some(country) = countries_by_tag.get(tag.as_bytes()).copied() else {
            continue;
        };
        let Ok(slot) = mapped_slot(country, links) else {
            continue;
        };
        let Ok(index) = inputs.binary_search_by_key(&country.id, |row| row.country) else {
            continue;
        };
        let row = &inputs[index];
        let Some(read) =
            replay::sliced::read_hour(reads, TableId::CountryStabilityInputs, row.country)
        else {
            continue;
        };
        let mut breakdown = breakdown_of(
            BreakdownInput {
                decoded: &decoded,
                hour,
                row,
                slot,
                read_hour: read,
                reads,
                tag,
            },
            warnings,
        );
        if let Some(content) = content {
            if read != hour && !read_frames.contains_key(&read) {
                if cancelled.load(Ordering::Acquire) {
                    return Err(io::Error::new(
                        io::ErrorKind::Interrupted,
                        "Stability request cancelled",
                    ));
                }
                let frame = match replay.seek(branch, read)? {
                    replay::container::SeekResult::Frame(frame) => Some(frame),
                    replay::container::SeekResult::Gap { .. } => None,
                };
                read_frames.insert(read, frame);
            }
            let names = if read == hour {
                Some(&*decoded)
            } else {
                read_frames.get(&read).and_then(|frame| frame.as_deref())
            };
            let characters = names.and_then(|frame| frame.complete::<tables::Characters>().ok());
            // The breakdown's text is expanded in the country's event scope
            // at the read's hour.
            let scripted = match (names, content.catalogue.scripted_content()) {
                (Some(frame), Ok(scripted)) => mapped_slot(country, links).ok().map(|slot| {
                    let aliases = std::sync::Arc::clone(&scripted.tag_aliases);
                    let scopes = match original_tags.get(&read) {
                        Some(lists) => dashboard_text::scope::RecordedScopes::read_with(
                            frame,
                            lists.clone(),
                            aliases,
                        ),
                        None => dashboard_text::scope::RecordedScopes::read(frame, aliases),
                    };
                    ScriptedText::with_scopes(
                        frame,
                        &content.catalogue.localisation,
                        scripted,
                        slot,
                        scopes,
                    )
                }),
                (_, Err(reason)) => {
                    content
                        .catalogue
                        .refuse(&format!("scripted text is not evaluated: {reason}"), tag);
                    None
                }
                (None, _) => None,
            };
            let lexical = names.and_then(|frame| frame.complete::<tables::LexicalTokens>().ok());
            let look = names.and_then(|frame| Look::read(frame, country));
            for line in breakdown
                .modifiers
                .iter_mut()
                .flat_map(|modifier| modifier.lines.iter_mut())
            {
                let country = IdeaCountry {
                    tag: tag.as_str(),
                    slot: country.canonical_slot,
                    look: look.as_ref().map(Look::country_look),
                };
                let character = recorded_character(line, characters, lexical);
                (line.group, line.group_kind) = line_group(line, content, character.as_ref());
                line.label = line_name(line, content, character, scripted.as_ref(), country);
            }
        }
        entry.breakdown = Some(breakdown);
    }
    Ok(())
}

/// Which slot or kind of category a paired idea or advisor line belongs to.
/// An advisor's role is its recorded character's
/// ([`PoliticsContent::held_advisor`]).
pub(crate) fn line_group(
    line: &BreakdownLine,
    content: &PoliticsContent,
    character: Option<&RecordedCharacter>,
) -> (Option<String>, Option<String>) {
    line_group_for(line, content, character, true)
}

pub fn line_group_offline(
    line: &BreakdownLine,
    content: &PoliticsContent,
) -> (Option<String>, Option<String>) {
    line_group_for(line, content, None, false)
}

fn line_group_for(
    line: &BreakdownLine,
    content: &PoliticsContent,
    character: Option<&RecordedCharacter>,
    has_recording: bool,
) -> (Option<String>, Option<String>) {
    let tags = &content.ideas.tags;
    match line.kind.as_str() {
        "advisor" => (
            (if has_recording {
                content.held_advisor(&line.identity, built(character))
            } else {
                content.offline_advisor(&line.identity)
            })
            .ok()
            .flatten()
            .and_then(|role| content.localised(&role.slot, View::Plain)),
            None,
        ),
        "idea" => {
            let Ok(Some(idea)) = content.idea(&line.identity) else {
                return (None, None);
            };
            // The group names its category, or a slot in one.
            match tags.resolve(&idea.group) {
                Ok(Some(category)) if category.name == idea.group => (None, category.kind.clone()),
                Ok(_) => (content.localised(&idea.group, View::Plain), None),
                // A file that could declare the category stops short.
                Err(gap) => {
                    content.catalogue.refuse(&gap.to_string(), &idea.group);
                    (None, None)
                }
            }
        }
        _ => (None, None),
    }
}

/// A line's recorded character: its script token resolved through LexicalTokens,
/// saved name (`token`) and template ([`built_template`]). None when the frame
/// does not hold the character.
pub(crate) struct RecordedCharacter<'a> {
    pub script: Option<&'a str>,
    pub saved: &'a str,
    pub template: Result<BuiltTemplate, String>,
}

/// A line's character's template, as [`PoliticsContent::held_advisor`]
/// takes it.
fn built<'a>(character: Option<&'a RecordedCharacter>) -> Result<&'a BuiltTemplate, &'a str> {
    match character {
        Some(character) => character.template.as_ref().map_err(String::as_str),
        None => Err("the line's character is not recorded at its read"),
    }
}

pub(super) fn recorded_character<'a>(
    line: &BreakdownLine,
    characters: Option<&'a [replay::rows::CharacterRow]>,
    lexical: Option<&'a [replay::rows::LexicalTokenRow]>,
) -> Option<RecordedCharacter<'a>> {
    let rows = characters?;
    let index = rows
        .binary_search_by_key(&line.identity_id, |row| row.id)
        .ok()?;
    Some(recorded_character_row(&rows[index], lexical))
}

pub(super) fn recorded_character_row<'a>(
    row: &'a replay::rows::CharacterRow,
    lexical: Option<&'a [replay::rows::LexicalTokenRow]>,
) -> RecordedCharacter<'a> {
    RecordedCharacter {
        script: lexical.and_then(|rows| character_script_token(row, rows)),
        saved: std::str::from_utf8(&row.token).ok().unwrap_or_default(),
        template: built_template(row, lexical),
    }
}

/// A line's name in the viewer's language, its character (if any) as recorded:
/// the saved name first, so a renamed character shows its current name; else the
/// content name of its script token, as runtime characters carry a literal name.
///
/// The stability breakdown expands all names in the country's event scope.
/// Each name is scripted text: evaluated with `scripted`, else a closed
/// `[...]` is refused.
pub(crate) fn line_name(
    line: &BreakdownLine,
    content: &PoliticsContent,
    character: Option<RecordedCharacter>,
    scripted: Option<&ScriptedText<'_>>,
    country: IdeaCountry<'_>,
) -> Option<String> {
    line_name_for(line, content, character, scripted, Some(country))
}

pub fn line_name_offline(line: &BreakdownLine, content: &PoliticsContent) -> Option<String> {
    line_name_for(line, content, None, None, None)
}

fn line_name_for(
    line: &BreakdownLine,
    content: &PoliticsContent,
    character: Option<RecordedCharacter>,
    scripted: Option<&ScriptedText<'_>>,
    country: Option<IdeaCountry<'_>>,
) -> Option<String> {
    let has_recording = country.is_some();
    let localise = |key: &str| match scripted {
        Some(scripted) => content.localise_in(key, scripted),
        None if !has_recording => content.catalogue.localised_offline_country(key),
        None => content.localised(key, View::Scripted),
    };
    // A character's name enters the breakdown text as it is shown, and the
    // whole text is then expanded.
    let expanded = |name: String| -> Option<String> {
        let Some(scripted) = scripted else {
            if !has_recording && name.contains('[') {
                let reason = "character name expansion needs recorded scripted country context; unavailable offline";
                content.catalogue.refuse_as(
                    dashboard_core::findings::Kind::Offline,
                    reason,
                    &line.identity,
                );
            }
            return (!name.contains('[')).then_some(name);
        };
        scripted
            .expand(&name)
            .and_then(|text| dashboard_text::catalogue::plain_text(&text))
            .inspect_err(|reason| content.catalogue.refuse(reason, &line.identity))
            .ok()
    };
    let character_name = |token: &str| content.authored_character_name(token);
    // A recorded name that cannot be resolved stays unknown (Some(None)): no
    // authored name stands in.
    let recorded = || {
        let character = character.as_ref()?;
        match content.recorded_name(character.saved) {
            dashboard_base::products::politics_content::RecordedName::Named(name) => {
                Some(Some(name))
            }
            dashboard_base::products::politics_content::RecordedName::Unresolved(_) => Some(None),
            dashboard_base::products::politics_content::RecordedName::None => {
                character.script.and_then(character_name).map(Some)
            }
        }
    };
    match line.kind.as_str() {
        // Only the selected key participates: a refusal is final.
        "idea" => match match country {
            Some(country) => content.idea_name(&line.identity, country),
            None => content.idea_name_offline(&line.identity),
        } {
            Ok(IdeaName::Key(key)) => localise(&key),
            // Generated text enters the breakdown as it is shown.
            Ok(IdeaName::Generated(text)) => expanded(text),
            Err(_) => None,
        },
        "advisor" => recorded()
            .unwrap_or_else(|| {
                let role = if has_recording {
                    content.held_advisor(&line.identity, built(character.as_ref()))
                } else {
                    content.offline_advisor(&line.identity)
                }
                .ok()??;
                character_name(&role.character)
            })
            .and_then(expanded),
        "countryLeader" => recorded().flatten().and_then(expanded),
        "ideology" | "static" | "staticScaled" | "dynamicModifier" | "dynamicModifierValues" => {
            localise(&line.identity)
        }
        _ => None,
    }
}

#[derive(Default, Serialize, Deserialize)]
struct Track {
    identity: Option<(u32, u32)>,
    values: [Option<i64>; 3],
    read: Option<i32>,
}

#[derive(Default, Serialize, Deserialize)]
pub(crate) struct Scan {
    tracks: BTreeMap<String, Track>,
    hour: Option<i32>,
    active: BTreeSet<String>,
    #[serde(skip)]
    native: Native,
    #[serde(skip)]
    pub(crate) removed: Vec<String>,
}

/// Current native operands only. A3 refreshes each source independently; no
/// recorded frame or historical table is retained, and resume rebuilds it.
#[derive(Default)]
pub(super) struct Native {
    keys: Option<dashboard_text::metadata::CountryKeys>,
    countries: Option<Vec<replay::rows::CountryRow>>,
    links: Option<Vec<replay::rows::CountryLinkRow>>,
    pub(super) roster: BTreeMap<u32, (String, u32)>,
    pub(super) slots: BTreeMap<u32, Vec<u32>>,
    pub(super) inputs: Option<BTreeMap<u32, CountryStabilityInputRow>>,
    pub(super) reads: Option<BTreeMap<u32, i32>>,
    command: Option<BTreeMap<u32, replay::rows::CountryCommandPowerRow>>,
    pub(super) membership_changed: bool,
    #[cfg(test)]
    pub(super) work: dashboard_base::prepared::observer_tests::Work,
}
fn changed_keys<T: PartialEq>(
    old: &Option<BTreeMap<u32, T>>,
    new: &Option<BTreeMap<u32, T>>,
) -> BTreeSet<u32> {
    old.iter()
        .chain(new)
        .flat_map(|rows| rows.keys())
        .copied()
        .filter(|id| {
            old.as_ref().and_then(|rows| rows.get(id)) != new.as_ref().and_then(|rows| rows.get(id))
        })
        .collect()
}
fn same_keys<T>(old: &Option<BTreeMap<u32, T>>, new: &Option<BTreeMap<u32, T>>) -> bool {
    match (old, new) {
        (Some(old), Some(new)) => old.keys().eq(new.keys()),
        (None, None) => true,
        _ => false,
    }
}
impl Native {
    pub(super) fn country(&self, id: u32) -> Option<&replay::rows::CountryRow> {
        let rows = self.countries.as_ref()?;
        rows.binary_search_by_key(&id, |row| row.id)
            .ok()
            .map(|index| &rows[index])
    }
    pub(super) fn refresh(
        &mut self,
        frame: &replay::frame::DecodedFrame,
        changes: Option<&dashboard_base::prepared::frame_changes::FrameChanges>,
        command_power: bool,
    ) -> BTreeSet<u32> {
        self.membership_changed = false;
        // These operands use their rows/status. Supplying dates are explicit
        // SliceReads rows; an equal fresh acquisition changes no operand here.
        let reads =
            |table: TableId| changes.is_none_or(|changes| changes.rows_changed[table.index()]);
        let mut roster_changed = false;
        if reads(TableId::Countries) {
            let rows = frame.complete::<tables::Countries>().ok();
            #[cfg(test)]
            {
                self.work.row_visits += rows.map_or(0, |rows| rows.len());
            }
            if !rows.is_some_and(|rows| self.keys.as_ref().is_some_and(|keys| keys.same(rows))) {
                self.keys = rows.map(dashboard_text::metadata::CountryKeys::read);
                self.countries = rows.map(|rows| rows.to_vec());
                roster_changed = true;
            }
        }
        if reads(TableId::CountryLinks) {
            let rows = frame.complete::<tables::CountryLinks>().ok();
            #[cfg(test)]
            {
                self.work.row_visits += rows.map_or(0, |rows| rows.len());
            }
            let next = rows.map(|rows| rows.to_vec());
            roster_changed |= self.links != next;
            self.links = next;
        }
        let mut dirty = BTreeSet::new();
        if roster_changed {
            self.membership_changed = true;
            dirty.extend(self.roster.keys().copied());
            self.roster.clear();
            self.slots.clear();
            if let Some((countries, links)) = self.countries.as_ref().zip(self.links.as_ref()) {
                for country in countries {
                    let (Ok(tag), Ok(slot)) = (
                        std::str::from_utf8(&country.tag),
                        mapped_slot(country, links),
                    ) else {
                        continue;
                    };
                    self.roster.insert(country.id, (tag.to_owned(), slot));
                    self.slots.entry(slot).or_default().push(country.id);
                }
            }
            dirty.extend(self.roster.keys().copied());
        }
        if reads(TableId::CountryStabilityInputs) {
            let rows = recorded_rows::<tables::CountryStabilityInputs>(frame).ok();
            #[cfg(test)]
            {
                self.work.row_visits += rows.map_or(0, |rows| rows.len());
            }
            let next = rows.map(|rows| rows.iter().map(|row| (row.country, row.clone())).collect());
            self.membership_changed |= !same_keys(&self.inputs, &next);
            dirty.extend(changed_keys(&self.inputs, &next));
            self.inputs = next;
        }
        if reads(TableId::SliceReads) {
            let rows = frame.complete::<tables::SliceReads>().ok();
            #[cfg(test)]
            {
                self.work.row_visits += rows.map_or(0, |rows| rows.len());
            }
            let next = rows.map(|rows| {
                rows.iter()
                    .filter(|row| row.table == TableId::CountryStabilityInputs as u32)
                    .map(|row| (row.owner, row.sample_hour))
                    .collect()
            });
            self.membership_changed |= !same_keys(&self.reads, &next);
            dirty.extend(changed_keys(&self.reads, &next));
            self.reads = next;
        }
        if command_power && reads(TableId::CountryCommandPower) {
            let rows = frame.complete::<tables::CountryCommandPower>().ok();
            #[cfg(test)]
            {
                self.work.row_visits += rows.map_or(0, |rows| rows.len());
            }
            let next = rows.map(|rows| {
                rows.iter()
                    .map(|row| (row.canonical_slot, row.clone()))
                    .collect()
            });
            for slot in changed_keys(&self.command, &next) {
                dirty.extend(self.slots.get(&slot).into_iter().flatten().copied());
            }
            self.command = next;
        }
        if changes.is_none_or(|changes| changes.rows_changed.iter().all(|changed| *changed)) {
            dirty.extend(self.roster.keys().copied());
        }
        dirty
    }
}
impl Scan {
    pub(crate) fn step(
        &mut self,
        decoded: &replay::frame::DecodedFrame,
        tags: Option<&BTreeSet<String>>,
        changes: Option<&dashboard_base::prepared::frame_changes::FrameChanges>,
        mut sink: impl FnMut(
            &str,
            &CountryStabilityInputRow,
            u32,
            i32,
            [Option<i64>; 3],
            [bool; 3],
            [bool; 4],
        ) -> io::Result<()>,
    ) -> io::Result<()> {
        let adjacent = self.hour.and_then(|hour| hour.checked_add(1)) == Some(decoded.hour);
        self.hour = Some(decoded.hour);
        self.removed.clear();
        let mut dirty = self.native.refresh(decoded, changes, true);
        if !adjacent {
            dirty.extend(self.native.roster.keys().copied());
        }
        // Only current holder membership is considered, never historical tracks.
        // A roster/table refusal closes its active keys at the previous frontier.
        if self.native.membership_changed
            || changes.is_none_or(|changes| changes.rows_changed.iter().all(|changed| *changed))
        {
            let valid: BTreeSet<_> = self
                .native
                .roster
                .iter()
                .filter(|(id, _)| {
                    self.native
                        .inputs
                        .as_ref()
                        .is_some_and(|rows| rows.contains_key(id))
                        && self
                            .native
                            .reads
                            .as_ref()
                            .is_some_and(|rows| rows.contains_key(id))
                })
                .map(|(_, (tag, _))| tag.clone())
                .collect();
            for tag in self.active.difference(&valid) {
                self.removed.push(tag.clone());
                if let Some(track) = self.tracks.get_mut(tag) {
                    track.identity = None;
                }
            }
            self.active = valid;
        }
        for id in dirty {
            let Some((tag, slot)) = self.native.roster.get(&id) else {
                continue;
            };
            if tags.is_some_and(|tags| !tags.contains(tag)) {
                continue;
            }
            let Some((row, read)) = self
                .native
                .inputs
                .as_ref()
                .and_then(|rows| rows.get(&id))
                .zip(
                    self.native
                        .reads
                        .as_ref()
                        .and_then(|rows| rows.get(&id))
                        .copied(),
                )
            else {
                continue;
            };
            let identity = (*slot, id);
            let track = self.tracks.entry(tag.to_owned()).or_default();
            let continues = adjacent && track.identity == Some(identity);
            let values = [
                formula::stability(row).value,
                formula::war_support(row).value,
                formula::max_command_power(
                    row,
                    self.native.command.as_ref().and_then(|rows| rows.get(slot)),
                )
                .value,
            ];
            if changes.is_none() || !continues || track.values != values || track.read != Some(read)
            {
                #[cfg(test)]
                {
                    self.native.work.emitted += 1;
                }
                sink(
                    tag,
                    row,
                    *slot,
                    read,
                    values,
                    track.values.map(|value| continues && value.is_some()),
                    std::array::from_fn(|index| {
                        !continues
                            || track.read != Some(read)
                            || index > 0 && track.values[index - 1] != values[index - 1]
                    }),
                )?;
            }
            track.identity = Some(identity);
            track.values = values;
            track.read = Some(read);
        }
        Ok(())
    }
    #[cfg(all(test, feature = "slow-tests"))]
    pub(crate) fn work(&self) -> dashboard_base::prepared::observer_tests::Work {
        self.native.work
    }
}

/// Scans one branch once for every tag; the breakdown is taken at `hour`.
pub fn scan(
    replay: &Replay,
    branch: u64,
    tags: &[&str],
    hour: Option<i32>,
    cancelled: &AtomicBool,
) -> io::Result<(BTreeMap<String, CountryStabilitySeries>, Vec<String>)> {
    let mut selected = vec![
        TableId::Campaign,
        TableId::Countries,
        TableId::CountryLinks,
        TableId::CountryStabilityInputs,
        TableId::CountryCommandPower,
        TableId::SliceReads,
    ];
    if hour.is_some() {
        selected.extend([TableId::CountryStabilityLines, TableId::ModifierDefinitions]);
    }
    let cancel = || io::Error::new(io::ErrorKind::Interrupted, "Stability request cancelled");
    let mut result: BTreeMap<String, CountryStabilitySeries> = tags
        .iter()
        .map(|tag| ((*tag).to_owned(), CountryStabilitySeries::default()))
        .collect();
    let mut observer = Scan::default();
    let selected_tags = tags.iter().map(|tag| (*tag).to_owned()).collect();
    let mut warnings = Vec::new();
    dashboard_core::blocks::scan_blocks(
        replay,
        replay.index.iter().filter(|block| block.branch == branch),
        &selected,
        cancelled,
        |blocks| {
            for (_, frames) in blocks {
                if cancelled.load(Ordering::Acquire) {
                    return Err(cancel());
                }
                let mut decoder = FrameDecoder::for_tables(&selected);
                let Ok(frames) = frames else {
                    continue;
                };
                for frame in frames {
                    if cancelled.load(Ordering::Acquire) {
                        return Err(cancel());
                    }
                    let Ok(decoded) = decoder.advance(&frame) else {
                        break;
                    };
                    if validate_campaign(decoded, replay).is_err() {
                        break;
                    }
                    observer.step(
                        decoded,
                        Some(&selected_tags),
                        None,
                        |tag, row, slot, read, values, joined, _changed| {
                            let series = result.get_mut(tag).unwrap();
                            for ((runs, value), joined) in [
                                &mut series.stability,
                                &mut series.war_support,
                                &mut series.max_command_power,
                            ]
                            .into_iter()
                            .zip(values)
                            .zip(joined)
                            {
                                let Some(value) = value else { continue };
                                match runs.last_mut() {
                                    Some(run)
                                        if joined && run.value_raw.parse::<i64>() == Ok(value) =>
                                    {
                                        run.last_hour = frame.hour;
                                    }
                                    _ => {
                                        runs.push(FixedRun {
                                            first_hour: frame.hour,
                                            last_hour: frame.hour,
                                            value_raw: value.to_string(),
                                            starts_segment: !joined,
                                        });
                                    }
                                }
                            }
                            series.recorded = true;
                            note_carried(&mut series.carried, frame.hour, read);
                            if hour == Some(frame.hour) {
                                series.breakdown = Some(breakdown_of(
                                    BreakdownInput {
                                        decoded,
                                        hour: frame.hour,
                                        row,
                                        slot,
                                        read_hour: read,
                                        reads: decoded.complete::<tables::SliceReads>().unwrap(),
                                        tag,
                                    },
                                    &mut warnings,
                                ));
                            }
                            Ok(())
                        },
                    )?;
                }
            }
            Ok(())
        },
    )?;
    Ok((result, warnings))
}

/// Notes that the values shown at `hour` come from the read at `read`: an
/// earlier read extends or starts a carried run, a read at `hour` adds nothing.
fn note_carried(carried: &mut Vec<ReadRun>, hour: i32, read: i32) {
    if read == hour {
        return;
    }
    match carried.last_mut() {
        Some(run) if run.read_hour == read && run.last_hour.checked_add(1) == Some(hour) => {
            run.last_hour = hour;
        }
        _ => {
            carried.push(ReadRun {
                first_hour: hour,
                last_hour: hour,
                read_hour: read,
            });
        }
    }
}

/// A tree child's recorded display string as plain text. The engine's strings
/// hold localisation as stored (a live capture:
/// `\u{13}pol_idea \u{11}RSubdued\u{11}!`), so they read as
/// [`dashboard_text::catalogue::plain_text`] reads stored text, trimmed. The error
/// names why the label cannot be shown: bytes that are not UTF-8, or a
/// spacing control plain text cannot show.
pub fn display_text(bytes: &[u8]) -> Result<String, String> {
    dashboard_text::catalogue::display_text(bytes)
        .map_err(|reason| format!("the recorded tree label {reason}"))
}

#[cfg(test)]
mod display_text_tests {
    use super::{display_text, modifiers};
    use replay::rows::{CountryStabilityInputRow, CountryStabilityLineRow};

    #[test]
    fn engine_colour_codes_and_icons_become_plain_text() {
        let shown = |bytes: &[u8]| display_text(bytes).unwrap();
        assert_eq!(
            shown(b"\x13pol_idea \x11RSubdued\x11! \x11YCivic Faction\x11!"),
            "Subdued Civic Faction"
        );
        assert_eq!(
            shown(b"\x13decision_icon_small Women in the Workforce"),
            "Women in the Workforce"
        );
        assert_eq!(shown("Léon \u{13}flag\u{13}Blum".as_bytes()), "Léon Blum");
        // A `§` or `£` the engine holds as the character is drawn as one.
        assert_eq!(
            shown("§YThe Reichstag§! £icon".as_bytes()),
            "§YThe Reichstag§! £icon"
        );
    }

    /// A refused label reaches the breakdown's warnings with its reason and
    /// the line it belongs to, counted once per line.
    #[test]
    fn a_refused_tree_label_is_named_in_the_warnings() {
        let row = CountryStabilityInputRow {
            country: 7,
            keys: vec![3; CountryStabilityInputRow::KEY_NAMES.len()],
            ..Default::default()
        };
        let line = |identity: &str, text: &[u8]| CountryStabilityLineRow {
            country: 7,
            key: 3,
            kind: CountryStabilityLineRow::UNPAIRED,
            identity: identity.as_bytes().to_vec(),
            text: text.to_vec(),
            ..Default::default()
        };
        let lines = [
            line("gap", "Front\u{14}é".as_bytes()),
            line("broken", b"caf\xc3"),
            line("fine", "\u{11}YÉtat\u{11}!".as_bytes()),
        ];
        let mut refused = dashboard_text::catalogue::Refusals::default();
        let breakdown = modifiers(&row, None, Some(&lines), |_| None, "FRA", &mut refused);
        let texts: Vec<_> = breakdown[0]
            .lines
            .iter()
            .map(|line| line.text.as_deref())
            .collect();
        assert_eq!(texts, [None, None, Some("État")]);
        assert_eq!(
            refused.lines("stability tree labels are"),
            [
                "1 stability tree labels are not shown: the recorded tree label holds the control byte 0x14 (an authored `¤`), which plain text cannot show; FRA gap",
                "1 stability tree labels are not shown: the recorded tree label is not UTF-8; FRA broken",
            ]
        );
    }
}

#[cfg(test)]
mod tests;

// Advisor templates as politics content resolves them for breakdown lines.
#[cfg(test)]
mod held_advisor_tests;
