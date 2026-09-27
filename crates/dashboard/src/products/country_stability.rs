//! Displayed stability, war support and maximum command power over one
//! branch, and the breakdown behind them at a requested hour.
//!
//! The game stores none of the displayed values: `CCountry::GetStability
//! @1406EBBE0`, `GetWarSupport@1406ED300` and `GetMaxCommandPower@1406E50D0`
//! compute them from the operands CountryStabilityInputs records. [`formula`]
//! repeats that arithmetic exactly (wrapping i64, truncating division, the
//! engine's Q15 conversions and clamps). Each country's inputs row is the
//! latest slice read (SliceReads); a run carries it over the following hours
//! while the country keeps its identity and its row stays recorded, and
//! `read_hours` lists the hours it was read. The war predicates and world
//! tension join a read but do not force one, so between reads they are the
//! read's values.
//!
//! The breakdown lists the modifier tree lines of each tracked key with the
//! identity the recorder proved, or the game's display text (in the
//! recording's language) for an unpaired line, and an explicit remainder when
//! the lines do not add up to the recorded total.

use super::{LoadContext, Product, recorded_rows, validate_tag};
use crate::{
    Source,
    metadata::{mapped_slot, validate_campaign},
    products::country_measure::FixedRun,
};
use replay::{
    container::Replay,
    frame::FrameDecoder,
    rows::{CountryStabilityInputRow, CountryStabilityLineRow, TableId, tables},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io,
    sync::atomic::{AtomicBool, Ordering},
};
use ts_rs::TS;

pub struct CountryStability;

pub const MAX_COUNTRIES: usize = 16;
/// Runs kept per country and measure before the scan refuses.
const MAX_RUNS: usize = 400_000;

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CountryStabilityParams {
    pub branch_id: String,
    pub tags: Vec<String>,
    /// The hour whose breakdown to include; none for runs only.
    #[serde(default)]
    pub hour: Option<i32>,
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
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct ModifierBreakdown {
    /// The modifier's script name.
    pub key: String,
    /// The native key index; none when the game registers no such key.
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
    /// Hours the country's inputs were read at, ascending.
    pub read_hours: Vec<i32>,
    pub breakdown: Option<StabilityBreakdown>,
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
    type Params = CountryStabilityParams;
    type Reply = CountryStabilityReply;

    fn types() -> Vec<String> {
        vec![
            CountryStabilityParams::decl(),
            FixedRun::decl(),
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
        if params.tags.is_empty() || params.tags.len() > MAX_COUNTRIES {
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
        _context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let branch = params.branch_id.parse::<u64>().expect("validated branch");
        // Histories do not depend on the inspected hour: they are scanned once
        // per (branch, country) and kept; the breakdown reads only its hour.
        let state = source.state::<Cache>();
        let cached = |tag: &str| {
            let cache = state.lock().unwrap_or_else(|error| error.into_inner());
            cache
                .entries
                .iter()
                .find(|(key, _)| key.0 == branch && key.1 == tag)
                .map(|(_, series)| series.clone())
        };
        let mut found: BTreeMap<String, CountryStabilitySeries> = BTreeMap::new();
        let mut missing: Vec<&str> = Vec::new();
        for tag in &params.tags {
            match cached(tag) {
                Some(series) => {
                    found.insert(tag.clone(), series);
                }
                None if !missing.contains(&tag.as_str()) => missing.push(tag),
                None => {}
            }
        }
        let mut warnings = Vec::new();
        if !missing.is_empty() {
            let (scanned, scan_warnings) =
                scan(&source.replay(), branch, &missing, None, cancelled)?;
            warnings.extend(scan_warnings);
            let mut cache = state.lock().unwrap_or_else(|error| error.into_inner());
            for (tag, series) in scanned {
                cache
                    .entries
                    .push_back(((branch, tag.clone()), series.clone()));
                found.insert(tag, series);
            }
            while cache.entries.len() > MAX_CACHED_COUNTRIES.max(params.tags.len()) {
                cache.entries.pop_front();
            }
        }
        if let Some(hour) = params.hour {
            breakdowns(source, branch, hour, &mut found, &mut warnings)?;
        }
        let series = params
            .tags
            .iter()
            .map(|tag| {
                // A tag named twice gets the same series both times.
                let found = found.get(tag).cloned().unwrap_or_default();
                if found.read_hours.is_empty() {
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
        Ok(CountryStabilityReply {
            branch_id: params.branch_id.clone(),
            decimals: 5,
            series,
            warnings,
        })
    }
}

/// The breakdown of one country at the frame's hour: formula terms and lines.
fn breakdown_of(
    decoded: &replay::frame::DecodedFrame,
    hour: i32,
    row: &CountryStabilityInputRow,
    read: i32,
    reads: &[replay::rows::SliceReadRow],
    tag: &str,
    warnings: &mut Vec<String>,
) -> StabilityBreakdown {
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
                .map(|at| rows[at].flags & 2 != 0)
        })
    };
    StabilityBreakdown {
        hour,
        read_hour: read,
        language: String::from_utf8_lossy(&row.language).into_owned(),
        stability: breakdown(formula::stability(row)),
        war_support: breakdown(formula::war_support(row)),
        max_command_power: breakdown(formula::max_command_power(row)),
        lines_available: lines.is_some(),
        modifiers: modifiers(row, lines, good),
    }
}

/// The engine's arithmetic over one inputs row.
pub mod formula {
    use replay::rows::CountryStabilityInputRow;

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

    /// The ruling-party popularity term (1406EBEAE..1406EBF41). The engine's
    /// overflow sentinel (0xffffffff when the quotient is i64::MAX, MIN or
    /// MIN + 1) is kept although a quotient by 100000 cannot reach it.
    pub fn popularity_term(factor: i64, popularity: i64) -> i64 {
        let product = factor.wrapping_mul(popularity) / 100_000;
        if [i64::MAX, i64::MIN, i64::MIN + 1].contains(&product) {
            0xffff_ffff
        } else {
            product.wrapping_mul(100_000) / 10_000_000
        }
    }

    /// `CCountry::GetStability@1406EBBE0`.
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

    /// scale -> fixed5 -> clamp to the impact).
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

    /// `CCountry::GetWarSupport@1406ED300`.
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

    /// `CCountry::GetMaxCommandPower@1406E50D0`: not additive; the terms are
    /// its operands and nothing is clamped.
    pub fn max_command_power(row: &CountryStabilityInputRow) -> Terms {
        let value = (|| {
            let base = row
                .base_max_command_power
                .wrapping_add(row.max_command_power?);
            Some(
                row.max_command_power_mult?
                    .wrapping_add(100_000)
                    .wrapping_mul(base)
                    / 100_000,
            )
        })();
        Terms {
            terms: vec![
                ("baseMaxCommandPower", Some(row.base_max_command_power)),
                ("maxCommandPower", row.max_command_power),
                ("maxCommandPowerMult", row.max_command_power_mult),
            ],
            unclamped: value,
            value,
        }
    }
}

fn breakdown(terms: formula::Terms) -> FormulaBreakdown {
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
/// not hold this read.
pub fn modifiers(
    row: &CountryStabilityInputRow,
    lines: Option<&[CountryStabilityLineRow]>,
    good: impl Fn(u32) -> Option<bool>,
) -> Vec<ModifierBreakdown> {
    CountryStabilityInputRow::KEY_NAMES
        .iter()
        .zip(&row.keys)
        .zip(row.totals())
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
                    .map(|line| BreakdownLine {
                        line: line.line,
                        kind: kind(line.kind).into(),
                        identity: String::from_utf8_lossy(&line.identity).into_owned(),
                        identity_id: line.identity_id,
                        secondary_raw: line.secondary.to_string(),
                        value_raw: line.value.to_string(),
                        text: (line.kind == CountryStabilityLineRow::UNPAIRED)
                            .then(|| display_text(&line.text)),
                    })
                    .collect(),
            }
        })
        .collect()
}

/// Recently scanned (branch, country) histories, oldest first.
#[derive(Default)]
struct Cache {
    entries: std::collections::VecDeque<((u64, String), CountryStabilitySeries)>,
}

const MAX_CACHED_COUNTRIES: usize = 32;

/// Adds each series' breakdown at `hour`, read from that hour's frame alone.
fn breakdowns(
    source: &Source,
    branch: u64,
    hour: i32,
    series: &mut BTreeMap<String, CountryStabilitySeries>,
    warnings: &mut Vec<String>,
) -> io::Result<()> {
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
    for (tag, entry) in series.iter_mut() {
        let Some(country) = countries
            .iter()
            .find(|country| country.tag == tag.as_bytes())
        else {
            continue;
        };
        if mapped_slot(country, links).is_err() {
            continue;
        }
        let Ok(index) = inputs.binary_search_by_key(&country.id, |row| row.country) else {
            continue;
        };
        let row = &inputs[index];
        let Some(read) =
            replay::sliced::read_hour(reads, TableId::CountryStabilityInputs, row.country)
        else {
            continue;
        };
        entry.breakdown = Some(breakdown_of(
            &decoded, hour, row, read, reads, tag, warnings,
        ));
    }
    Ok(())
}

#[derive(Default)]
struct Track {
    identity: Option<(u32, u32)>,
    hour: Option<i32>,
    values: [Option<i64>; 3],
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
    let mut tracks: BTreeMap<String, Track> = BTreeMap::new();
    let mut warnings = Vec::new();
    for block in replay.index.iter().filter(|block| block.branch == branch) {
        if cancelled.load(Ordering::Acquire) {
            return Err(cancel());
        }
        let mut decoder = FrameDecoder::for_tables(&selected);
        let Ok(frames) = replay.read_block_selected(block, &selected) else {
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
            let (Ok(countries), Ok(links), Ok(inputs), Ok(reads)) = (
                decoded.complete::<tables::Countries>(),
                decoded.complete::<tables::CountryLinks>(),
                recorded_rows::<tables::CountryStabilityInputs>(decoded),
                decoded.complete::<tables::SliceReads>(),
            ) else {
                continue;
            };
            let read_hour = |table, owner| replay::sliced::read_hour(reads, table, owner);
            for country in countries {
                let Some(tag) = std::str::from_utf8(&country.tag).ok() else {
                    continue;
                };
                let Some(series) = result.get_mut(tag) else {
                    continue;
                };
                let Ok(slot) = mapped_slot(country, links) else {
                    continue;
                };
                let Ok(index) = inputs.binary_search_by_key(&country.id, |row| row.country) else {
                    continue;
                };
                let row = &inputs[index];
                let Some(read) = read_hour(TableId::CountryStabilityInputs, row.country) else {
                    continue;
                };
                let identity = (slot, country.id);
                let track = tracks.entry(tag.to_owned()).or_default();
                let continues = track.identity == Some(identity)
                    && track.hour.and_then(|h| h.checked_add(1)) == Some(frame.hour);
                let values = [
                    formula::stability(row).value,
                    formula::war_support(row).value,
                    formula::max_command_power(row).value,
                ];
                for ((runs, value), previous) in [
                    &mut series.stability,
                    &mut series.war_support,
                    &mut series.max_command_power,
                ]
                .into_iter()
                .zip(values)
                .zip(track.values)
                {
                    let Some(value) = value else { continue };
                    let joined = continues && previous.is_some();
                    match runs.last_mut() {
                        Some(run) if joined && run.value_raw.parse::<i64>() == Ok(value) => {
                            run.last_hour = frame.hour;
                        }
                        _ => {
                            if runs.len() >= MAX_RUNS {
                                return Err(io::Error::other(
                                    "Stability history exceeds its memory budget",
                                ));
                            }
                            runs.push(FixedRun {
                                first_hour: frame.hour,
                                last_hour: frame.hour,
                                value_raw: value.to_string(),
                                starts_segment: !joined,
                            });
                        }
                    }
                }
                if series.read_hours.last() != Some(&read) {
                    series.read_hours.push(read);
                }
                track.identity = Some(identity);
                track.hour = Some(frame.hour);
                track.values = values;
                if hour == Some(frame.hour) {
                    series.breakdown = Some(breakdown_of(
                        decoded,
                        frame.hour,
                        row,
                        read,
                        reads,
                        tag,
                        &mut warnings,
                    ));
                }
            }
        }
    }
    Ok((result, warnings))
}

/// A tree child's recorded display string as plain text. The engine keeps
/// localisation's `§` colour codes as U+0011 and `£` icons as U+0013 in its
/// strings (a live capture: `\u{13}pol_idea \u{11}RSubdued\u{11}!`). Colour
/// codes are removed as [`crate::catalogue::plain_text`] removes them, and an
/// icon (`£name`, closed by an optional second `£`) is dropped, as the viewer's
/// rich text reads it, with the space it leaves at either end.
pub fn display_text(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let mut chars = text.chars().map(|c| match c {
        '\u{11}' => '§',
        '\u{13}' => '£',
        c => c,
    });
    let mut clean = String::new();
    let mut pending = None;
    while let Some(c) = pending.take().or_else(|| chars.next()) {
        if c != '£' {
            clean.push(c);
            continue;
        }
        for next in chars.by_ref() {
            if !(next.is_ascii_alphanumeric() || next == '_') {
                if next != '£' {
                    pending = Some(next);
                }
                break;
            }
        }
    }
    crate::catalogue::plain_text(&clean).trim().to_owned()
}

#[cfg(test)]
mod display_text_tests {
    use super::display_text;

    #[test]
    fn engine_colour_codes_and_icons_become_plain_text() {
        assert_eq!(
            display_text(b"\x13pol_idea \x11RSubdued\x11! \x11YCivic Faction\x11!"),
            "Subdued Civic Faction"
        );
        assert_eq!(
            display_text(b"\x13decision_icon_small Women in the Workforce"),
            "Women in the Workforce"
        );
        assert_eq!(
            display_text("Léon \u{13}flag\u{13}Blum".as_bytes()),
            "Léon Blum"
        );
        assert_eq!(
            display_text("§YThe Reichstag§! £icon".as_bytes()),
            "The Reichstag"
        );
    }
}
