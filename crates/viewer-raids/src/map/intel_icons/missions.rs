use dashboard_effects::ScriptedEffects;
use super::{
    build::*,
    values::{self, ONE},
    *,
};
use content_reader_localisation::localisation::Argument as Arg;
use replay_data::{
    fixed_point::{fixed5_to_q15, q15_to_fixed5},
    rows::{IntelNetworkAuxiliaryRow, OperativeEffect, Recorded},
};
use viewer_native_gui::native_text::FLAG;
use viewer_foundation::tooltip_text::{Tooltip, ensure_newlines, localise};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Defines {
    network: [i64; 3],
    ideology: [i64; 4],
    trade: [i64; 3],
    diplomacy: [i64; 3],
    propaganda: [i64; 4],
    coverage: [i64; 3],
}

impl Defines {
    pub fn read(d: &content_reader_defines::defines::Defines) -> Result<Self, String> {
        let fixed = |name: &str| {
            d.fixed5("NOperatives", name)
                .map_err(|error| format!("NOperatives.{name}: {error}"))
        };
        let array = |names: [&str; 3]| -> Result<[i64; 3], String> {
            Ok([fixed(names[0])?, fixed(names[1])?, fixed(names[2])?])
        };
        Ok(Self {
            network: array([
                "OPERATIVE_BASE_INTEL_NETWORK_GAIN",
                "OPERATIVE_MAX_INTEL_NETWORK_GAIN",
                "INTEL_NETWORK_OPERATIVE_GAIN_STACKING_FACTOR",
            ])?,
            ideology: [
                fixed("OPERATIVE_BASE_BOOST_IDEOLOGY")?,
                fixed("BOOST_IDEOLOGY_NATIONAL_COVERAGE_FACTOR")?,
                fixed("BOOST_IDEOLOGY_DEFENSE_FACTOR")?,
                fixed("BOOST_IDEOLOGY_DRIFT_STACKING_FACTOR")?,
            ],
            trade: [
                fixed("OPERATIVE_BASE_CONTROL_TRADE_DRIFT")?,
                fixed("CONTROL_TRADE_STACKING_FACTOR")?,
                fixed("CONTROL_TRADE_MAX_INFLUENCE")?,
            ],
            diplomacy: array([
                "OPERATIVE_BASE_DIPLOMATIC_PRESSURE_AI_ACCEPTANCE_SCORE_DRIFT",
                "OPERATIVE_BASE_DIPLOMATIC_PRESSURE_TENSION_REQUIREMENTS_DRIFT",
                "DIPLOMATIC_PRESSURE_OPERATIVE_STACKING_FACTOR",
            ])?,
            propaganda: [
                fixed("OPERATIVE_BASE_PROPAGANDA_POWER")?,
                fixed("PROPAGANDA_SUB_NETWORK_STRENGTH_FACTOR")?,
                fixed("PROPAGANDA_DEFENSE_FACTOR")?,
                fixed("PROPAGANDA_OPERATIVE_STACKING_FACTOR")?,
            ],
            coverage: array([
                "INTEL_NETWORK_NATIONAL_COVERAGE_CORE_STATES_WEIGHT",
                "INTEL_NETWORK_NATIONAL_COVERAGE_CONTROLLED_STATES_WEIGHT",
                "INTEL_NETWORK_NATIONAL_COVERAGE_OWNED_WORTH_WEIGHT",
            ])?,
        })
    }
}

pub(super) struct Inputs<'a> {
    rows: HashMap<u64, &'a OperativeMissionRow>,
    groups: HashMap<(u32, u32, i32), Vec<&'a OperativeMissionRow>>,
    auxiliary: Result<HashMap<(u32, u32, u32, u8), &'a IntelNetworkAuxiliaryRow>, String>,
    defense: Result<HashMap<u32, i64>, String>,
}

impl<'a> Inputs<'a> {
    pub fn new(hour: &Hour<'_, 'a>, tables: &Tables<'a>) -> Self {
        let mut result = Self {
            rows: HashMap::new(),
            groups: HashMap::new(),
            auxiliary: complete::<tables::IntelNetworkAuxiliaries>(hour.frame, "IntelNetworkAuxiliaries").map(|rows| {
                rows.iter()
                    .filter_map(|row| Some(((hour.slot(row.owner)?, hour.slot(row.target)?, row.subnetwork, row.kind), row)))
                    .collect()
            }),
            defense: complete::<tables::IntelligenceAgencies>(hour.frame, "IntelligenceAgencies")
                .map(|rows| {
                    rows.iter()
                        .filter_map(|row| Some((hour.slot(row.country)?, row.defense_raw)))
                        .collect()
                }),
        };
        if let Ok(rows) = &tables.missions {
            for row in *rows {
                result.rows.insert(row.id, row);
                if row.assignment == 3
                    && let (Some(owner), Some(variant), Some(state)) =
                        (hour.slot(row.owner_country), row.variant, row.target_state)
                {
                    result
                        .groups
                        .entry((owner, variant, state))
                        .or_default()
                        .push(row);
                }
            }
        }
        result
    }
}

#[derive(Clone, Copy)]
struct Mission<'a> {
    row: &'a OperativeMissionRow,
    variant: u32,
    target: u32,
    state: i32,
    ideology: &'a [u8],
    effect: Option<&'a OperativeEffect>,
}

impl<'a> Mission<'a> {
    fn of(row: &'a OperativeMissionRow, pending: bool) -> Result<Self, String> {
        let (variant, target, state, ideology, effect) = if pending {
            (
                row.pending_variant,
                row.pending_country,
                row.pending_state,
                row.pending_ideology.as_slice(),
                &row.pending_effect,
            )
        } else {
            (
                row.variant,
                row.target_country,
                row.target_state,
                row.ideology.as_slice(),
                &row.active_effect,
            )
        };
        Ok(Self {
            row,
            variant: variant.ok_or("operative mission variant unavailable")?,
            target: target.ok_or("operative mission target unavailable")?,
            state: state.unwrap_or(0),
            ideology,
            effect: match effect {
                Recorded::Value(effect) => effect.as_deref(),
                Recorded::NotRecorded => None,
            },
        })
    }

    fn modifier(self, index: usize) -> Result<i64, String> {
        self.effect
            .and_then(|effect| effect.modifiers.get(index))
            .map(|modifier| modifier.value)
            .ok_or_else(|| "operative mission modifier operands were not captured".into())
    }
}

#[derive(Clone, Copy)]
struct Contribution {
    value: [i64; 2],
    defense: i64,
    coverage: i64,
    stacking: i64,
    q15: bool,
}

fn coverage(
    hour: &Hour<'_, '_>,
    shared: &Shared<'_, '_>,
    mission: Mission<'_>,
) -> Result<Option<i64>, String> {
    let owner = hour
        .slot(mission.row.owner_country)
        .ok_or("network owner link unavailable")?;
    let target = hour
        .slot(mission.target)
        .ok_or("network target link unavailable")?;
    let rows = shared.tables.networks.as_ref().map_err(Clone::clone)?;
    let Some(network) = rows.iter().find(|row| {
        row.state_id == mission.state
            && hour.slot(row.owner_country) == Some(owner)
            && hour.slot(row.target_country) == Some(target)
    }) else {
        return Ok(None);
    };
    let Ok(subnetwork) = u32::try_from(network.subnetwork_selector) else {
        return Ok(None);
    };
    let auxiliary = shared.missions.auxiliary.as_ref().map_err(Clone::clone)?;
    let get = |kind| {
        auxiliary
            .get(&(owner, target, subnetwork, kind))
            .copied()
            .ok_or_else(|| "subnetwork coverage operands unavailable".to_owned())
    };
    let covered = get(8)?;
    let total = get(9)?;
    let sums = get(10)?;
    let weights = hour
        .defines
        .missions
        .as_ref()
        .map_err(Clone::clone)?
        .coverage;
    let quotient = |a: i64, b: i64| {
        if b == 0 {
            0
        } else {
            values::divide(a, b).unwrap_or(i64::from(u32::MAX))
        }
    };
    let controlled = quotient(
        sums.first,
        i64::from(covered.count_b.wrapping_mul(100)).wrapping_mul(ONE),
    );
    let cores = quotient(
        sums.second,
        i64::from(covered.count_a.wrapping_mul(100)).wrapping_mul(ONE),
    );
    let counts = [covered.count_a, covered.count_b, covered.count_c];
    let totals = [total.count_a, total.count_b, total.count_c];
    let mut value = 0i64;
    for index in 0..3 {
        let weighted = values::times(i64::from(counts[index]).wrapping_mul(ONE), weights[index]);
        let share = quotient(weighted, i64::from(totals[index]).wrapping_mul(ONE));
        value = value.wrapping_add(values::times(
            share,
            if index == 0 { cores } else { controlled },
        ));
    }
    Ok(Some(value.clamp(0, ONE)))
}

fn base(
    hour: &Hour<'_, '_>,
    shared: &Shared<'_, '_>,
    mission: Mission<'_>,
) -> Result<Contribution, String> {
    let d = hour.defines.missions.as_ref().map_err(Clone::clone)?;
    let mut result = Contribution {
        value: [0; 2],
        defense: ONE,
        coverage: ONE,
        stacking: ONE,
        q15: false,
    };
    let modifier = mission.modifier(0)?;
    let multiplied = |base| values::times(base, ONE.wrapping_add(modifier));
    match mission.variant {
        BUILD_NETWORK => {
            let scale = mission
                .effect
                .ok_or("network gain multiplier unavailable")?
                .network_gain_scale;
            let value = values::times(
                d.network[0]
                    .wrapping_mul(i64::from(scale))
                    .wrapping_add(modifier),
                ONE.wrapping_add(mission.modifier(1)?),
            )
            .max(0);
            result.value[0] = if d.network[1] < 0 {
                value
            } else {
                value.min(d.network[1])
            };
        }
        COUNTER_INTELLIGENCE => {
            let d = hour.defines.counter.as_ref().map_err(Clone::clone)?;
            result.value[0] = values::times(
                d.operative_base.wrapping_add(modifier),
                ONE.wrapping_add(d.foreign_agent),
            );
        }
        ROOT_OUT => {
            result.value[0] =
                multiplied(hour.defines.root_out.as_ref().map_err(Clone::clone)?[0]).max(0)
        }
        CONTROL_TRADE => result.value[0] = multiplied(d.trade[0]).max(0),
        DIPLOMATIC_PRESSURE => {
            result.q15 = true;
            for (value, base) in result.value.iter_mut().zip(d.diplomacy) {
                *value = (fixed5_to_q15(ONE.wrapping_add(modifier))
                    .wrapping_mul(fixed5_to_q15(base))
                    >> 15)
                    .max(0);
            }
            result.defense = 32768;
            result.coverage = 32768;
            result.stacking = 32768;
        }
        BOOST_IDEOLOGY | PROPAGANDA => {
            let target = hour
                .slot(mission.target)
                .ok_or("mission target country link unavailable")?;
            let defense = *shared
                .missions
                .defense
                .as_ref()
                .map_err(Clone::clone)?
                .get(&target)
                .ok_or("mission target agency defense unavailable")?;
            let covered = coverage(hour, shared, mission)?;
            if mission.variant == BOOST_IDEOLOGY {
                let Some(covered) = covered else {
                    return Ok(result);
                };
                result.coverage = values::times(covered, d.ideology[1]);
                let denominator = ONE.wrapping_add(values::times(defense, d.ideology[2]));
                result.defense = if denominator == 0 {
                    ONE
                } else {
                    values::divide(ONE, denominator).ok_or("ideology defense factor overflow")?
                };
                result.value[0] = values::times(
                    values::times(multiplied(d.ideology[0]), result.coverage),
                    result.defense,
                )
                .max(0);
            } else {
                result.q15 = true;
                result.coverage = fixed5_to_q15(covered.unwrap_or(0))
                    .wrapping_mul(fixed5_to_q15(d.propaganda[1]))
                    >> 15;
                let denominator = 32768i64.wrapping_add(
                    fixed5_to_q15(defense).wrapping_mul(fixed5_to_q15(d.propaganda[2])) >> 15,
                );
                result.defense = if denominator == 0 {
                    32768
                } else {
                    (1i64 << 30)
                        .checked_div(denominator)
                        .ok_or("propaganda defense factor overflow")?
                };
                result.stacking = 32768;
                let base = fixed5_to_q15(ONE.wrapping_add(modifier))
                    .wrapping_mul(fixed5_to_q15(d.propaganda[0]))
                    >> 15;
                let factor = result.coverage.wrapping_mul(result.defense) >> 15;
                result.value = [(factor.wrapping_mul(base) >> 15).max(0); 2];
            }
        }
        _ => return Err("operative mission has no daily contribution".into()),
    }
    Ok(result)
}

fn stacked<'f>(
    hour: &Hour<'_, 'f>,
    shared: &Shared<'_, 'f>,
    mission: Mission<'f>,
) -> Result<Contribution, String> {
    if matches!(
        mission.variant,
        COUNTER_INTELLIGENCE | CONTROL_TRADE | DIPLOMATIC_PRESSURE
    ) && mission
        .effect
        .ok_or("operative mission target state count unavailable")?
        .owned_states
        < 1
    {
        return Ok(Contribution {
            value: [0; 2],
            defense: if mission.variant == DIPLOMATIC_PRESSURE {
                32768
            } else {
                ONE
            },
            coverage: if mission.variant == DIPLOMATIC_PRESSURE {
                32768
            } else {
                ONE
            },
            stacking: 0,
            q15: mission.variant == DIPLOMATIC_PRESSURE,
        });
    }
    if mission.variant == BOOST_IDEOLOGY && coverage(hour, shared, mission)?.is_none() {
        return Ok(Contribution {
            value: [0; 2],
            defense: 0,
            coverage: 0,
            stacking: 0,
            q15: false,
        });
    }
    let owner = hour
        .slot(mission.row.owner_country)
        .ok_or("operative owner country link unavailable")?;
    let target = hour
        .slot(mission.target)
        .ok_or("mission target country link unavailable")?;
    let mut members = Vec::new();
    members.push(mission);
    let mut add = |row: &'f OperativeMissionRow| -> Result<(), String> {
        let active = Mission::of(row, false)?;
        if row.id != mission.row.id
            && (matches!(mission.variant, COUNTER_INTELLIGENCE | BOOST_IDEOLOGY)
                || hour.slot(row.owner_country) == Some(owner))
            && (mission.variant != BOOST_IDEOLOGY
                || active.ideology.eq_ignore_ascii_case(mission.ideology))
        {
            members.push(active);
        }
        Ok(())
    };
    if matches!(mission.variant, BUILD_NETWORK | ROOT_OUT) {
        let controller = if mission.variant == BUILD_NETWORK {
            let states = hour
                .frame
                .complete::<tables::States>()
                .map_err(|error| error.to_string())?;
            let state = states
                .iter()
                .find(|state| state.id as i32 == mission.state)
                .ok_or("network state controller unavailable")?;
            Some(
                hour.slot(state.controller)
                    .ok_or("network state controller link unavailable")?,
            )
        } else {
            None
        };
        if let Some(rows) = shared
            .missions
            .groups
            .get(&(owner, mission.variant, mission.state))
        {
            for row in rows {
                if mission.variant != BUILD_NETWORK
                    || hour.slot(
                        row.target_country
                            .ok_or("network mission target unavailable")?,
                    ) == controller
                {
                    add(row)?;
                }
            }
        }
    } else {
        let row = shared
            .tables
            .strategic
            .as_ref()
            .map_err(Clone::clone)?
            .iter()
            .find(|row| hour.slot(row.country) == Some(target));
        if let Some(row) = row {
            let index = match mission.variant {
                COUNTER_INTELLIGENCE => 0,
                BOOST_IDEOLOGY => 1,
                PROPAGANDA => 2,
                CONTROL_TRADE => 3,
                DIPLOMATIC_PRESSURE => 4,
                _ => return Err("mission roster variant unsupported".into()),
            };
            let rosters = match &row.mission_rosters {
                Recorded::Value(Some(rosters)) => rosters,
                Recorded::Value(None) => {
                    return Err("target's retained mission rosters unavailable".into());
                }
                Recorded::NotRecorded => {
                    return Err("target's retained mission rosters were not recorded".into());
                }
            };
            for id in &rosters[index] {
                if *id == 0 {
                    continue;
                }
                let row = shared.missions.rows.get(id).copied().ok_or("retained mission roster references an operative absent from the acquired agency roster")?;
                if row.variant == Some(mission.variant)
                    && hour.slot(
                        row.target_country
                            .ok_or("retained mission target unavailable")?,
                    ) == Some(target)
                {
                    add(row)?;
                }
            }
        }
    }
    let d = hour.defines.missions.as_ref().map_err(Clone::clone)?;
    let factor = match mission.variant {
        BUILD_NETWORK => d.network[2],
        COUNTER_INTELLIGENCE => {
            hour.defines
                .counter
                .as_ref()
                .map_err(Clone::clone)?
                .stacking
        }
        ROOT_OUT => hour.defines.root_out.as_ref().map_err(Clone::clone)?[1],
        CONTROL_TRADE => d.trade[1],
        BOOST_IDEOLOGY => d.ideology[3],
        PROPAGANDA => d.propaganda[3],
        DIPLOMATIC_PRESSURE => d.diplomacy[2],
        _ => return Err("mission stacking factor unavailable".into()),
    };
    let mut entries = members
        .into_iter()
        .map(|member| Ok((member.row.id, base(hour, shared, member)?)))
        .collect::<Result<Vec<_>, String>>()?;
    entries.sort_unstable_by_key(|entry| entry.0);
    entries.dedup_by_key(|entry| entry.0);
    let mut answer = entries
        .iter()
        .find(|entry| entry.0 == mission.row.id)
        .ok_or("hovered operative contribution absent")?
        .1;
    let reference = |id: u64| (id as u32, (id >> 32) as u32);
    let channels = if answer.q15 { 2 } else { 1 };
    for channel in 0..channels {
        entries.sort_by(|a, b| {
            b.1.value[channel]
                .cmp(&a.1.value[channel])
                .then_with(|| reference(b.0).cmp(&reference(a.0)))
        });
        let mut weight = if answer.q15 { 32768 } else { ONE };
        for (id, entry) in &entries {
            if *id == mission.row.id {
                answer.value[channel] = if answer.q15 {
                    entry.value[channel].wrapping_mul(weight) >> 15
                } else {
                    values::times(entry.value[channel], weight)
                };
                answer.stacking = weight;
                break;
            }
            weight = if answer.q15 {
                weight.wrapping_mul(fixed5_to_q15(factor)) >> 15
            } else {
                values::times(weight, factor)
            };
        }
    }
    Ok(answer)
}

fn sources(
    hour: &Hour<'_, '_>,
    mission: Mission<'_>,
    base: i64,
    precision: i32,
    negative: bool,
) -> Result<String, String> {
    use dashboard_text::modifier_text::{self, Breakdown};
    let mut text = String::new();
    if base != 0 {
        text.push_str(&localise(hour.localisation, "STAT_BASE_VALUE", &[])?);
        let value = if negative { base.wrapping_neg() } else { base };
        let number = if matches!(mission.variant, ROOT_OUT | PROPAGANDA)
            || (mission.variant == DIPLOMATIC_PRESSURE && negative)
        {
            content_reader_parser::fixed5::fixed5_percent(
                value,
                if mission.variant == ROOT_OUT {
                    0
                } else {
                    precision
                },
                true,
            )
        } else {
            content_reader_parser::fixed5::fixed5_to_string(value, precision)
        };
        let colour = if value == 0 {
            'Y'
        } else if (value > 0) != negative {
            'G'
        } else {
            'R'
        };
        text.push_str(&values::coloured(colour, &number));
        text.push('\n');
    }
    let effect = mission
        .effect
        .ok_or("operative modifier sources were not captured")?;
    let definitions = hour
        .frame
        .complete::<tables::ModifierDefinitions>()
        .map_err(|error| error.to_string())?;
    for modifier in &effect.modifiers {
        let definition = definitions
            .iter()
            .find(|row| row.index == modifier.key)
            .ok_or("operative modifier definition unavailable")?;
        for tree in modifier
            .sources
            .as_ref()
            .ok_or("operative modifier source certificate unavailable")?
        {
            if tree.is_empty() {
                continue;
            }
            text.push_str(&modifier_text::breakdown(
                tree,
                Breakdown {
                    depth: -1,
                    prefix: "",
                    suffix: "",
                },
                |node| {
                    let mut name = dashboard_text::scripted_text::stored_text(
                        hour.localisation,
                        hour.language,
                        &node.name,
                        "operative modifier source",
                    )?;
                    if !name.is_empty() {
                        name.push_str(": ");
                    }
                    name.push_str(&modifier_text::value_text(
                        definition,
                        node.value,
                        2,
                        false,
                        false,
                        |key, _| {
                            localise(
                                hour.localisation,
                                std::str::from_utf8(key)
                                    .map_err(|_| "operative modifier key is not UTF-8")?,
                                &[],
                            )
                        },
                    )?);
                    Ok(name)
                },
            )?);
        }
    }
    Ok(text)
}

fn scaled(hour: &Hour<'_, '_>, contribution: Contribution) -> Result<String, String> {
    let unit = if contribution.q15 { 32768 } else { ONE };
    let factors = [
        ("MODIFIER_TARGET_INTELLIGENCE_DEFENSE", contribution.defense),
        ("MODIFIER_OPERATIVE_STACKING_FACTOR", contribution.stacking),
        ("MODIFIER_NATIONAL_COVERAGE_FACTOR", contribution.coverage),
    ];
    let changed = factors.iter().filter(|(_, value)| *value != unit).count();
    if changed == 0 {
        return Ok(String::new());
    }
    let mut combined = unit;
    for (_, value) in factors {
        combined = if contribution.q15 {
            combined.wrapping_mul(value) >> 15
        } else {
            values::times(combined, value)
        };
    }
    let percent = |value| {
        values::coloured(
            'Y',
            &content_reader_parser::fixed5::fixed5_percent(
                if contribution.q15 {
                    q15_to_fixed5(value)
                } else {
                    value
                },
                0,
                true,
            ),
        )
    };
    let header = if changed == 1 {
        localise(hour.localisation, "MODIFIER_SCALED_BY", &[])?
    } else {
        localise(
            hour.localisation,
            "MODIFIER_SCALED_BY",
            &[("VALUE", Arg::Text(&percent(combined)))],
        )?
    };
    let mut text = values::coloured('H', &header);
    for (key, value) in factors {
        if value == unit {
            continue;
        }
        ensure_newlines(&mut text, 1);
        text.push_str(&localise(
            hour.localisation,
            key,
            &[("VALUE", Arg::Text(&percent(value)))],
        )?);
    }
    ensure_newlines(&mut text, 1);
    Ok(text)
}

pub(super) fn append<'f>(
    hour: &Hour<'_, 'f>,
    row: &'f OperativeMissionRow,
    shared: &Shared<'_, 'f>,
    text: &mut Tooltip,
) {
    let result = (|| {
        let mission = Mission::of(row, row.assignment != 3)?;
        let country = hour
            .slot(mission.target)
            .ok_or("mission target country link unavailable")?;
        let flag = format!(
            "{FLAG}{}",
            String::from_utf8_lossy(
                hour.world
                    .tag(country)
                    .ok_or("mission target tag unavailable")?
            )
        );
        let name = shared.names.name_with(country, "_DEF")?;
        let adjective = || shared.names.name_with(country, "_ADJ");
        let state = || {
            hour.state_name(
                shared,
                u32::try_from(mission.state)
                    .ok()
                    .filter(|&id| id != 0)
                    .ok_or("mission target state unavailable")?,
            )
        };
        let loc = |key: &str, args: &[(&str, Arg<'_>)]| localise(hour.localisation, key, args);
        let contribution = if mission.variant == QUIET_NETWORK {
            None
        } else {
            Some(stacked(hour, shared, mission)?)
        };
        let raw = |index| {
            let contribution = contribution.unwrap();
            if contribution.q15 {
                q15_to_fixed5(contribution.value[index])
            } else {
                contribution.value[index]
            }
        };
        let mut out = match mission.variant {
            BUILD_NETWORK | QUIET_NETWORK => loc(
                if mission.variant == BUILD_NETWORK {
                    "OPERATIVE_MISSION_BUILD_INTEL_NETWORK_TT"
                } else {
                    "OPERATIVE_MISSION_QUIET_INTEL_NETWORK_TT"
                },
                &[
                    ("STATE", Arg::Text(&state()?)),
                    ("COUNTRY_DEF", Arg::Text(&name)),
                    ("FLAG", Arg::Text(&flag)),
                ],
            )?,
            COUNTER_INTELLIGENCE => loc("OPERATIVE_MISSION_COUNTER_INTELLIGENCE_TT", &[])?,
            ROOT_OUT => loc(
                "OPERATIVE_MISSION_ROOT_OUT_RESISTANCE_TT",
                &[
                    ("STATE", Arg::Text(&state()?)),
                    ("COUNTRY_ADJ", Arg::Text(&adjective()?)),
                    ("FLAG", Arg::Text(&flag)),
                ],
            )?,
            CONTROL_TRADE => loc(
                "OPERATIVE_MISSION_CONTROL_TRADE_TT",
                &[
                    ("COUNTRY_ADJ", Arg::Text(&adjective()?)),
                    ("FLAG", Arg::Text(&flag)),
                    ("VALUE", Arg::Fixed5(raw(0))),
                ],
            )?,
            BOOST_IDEOLOGY => {
                let ideology = std::str::from_utf8(mission.ideology)
                    .map_err(|_| "mission ideology token is not UTF-8")?;
                let marked = format!("{flag} {}", shared.names.name(country)?);
                loc(
                    "OPERATIVE_MISSION_BOOST_IDEOLOGY_TT",
                    &[
                        ("IDEOLOGY", Arg::Text(&loc(ideology, &[])?)),
                        ("COUNTRY", Arg::Text(&marked)),
                        ("VALUE", Arg::Fixed5(raw(0))),
                    ],
                )?
            }
            DIPLOMATIC_PRESSURE => loc(
                "OPERATIVE_MISSION_DIPLOMATIC_PRESSURE_TT",
                &[
                    ("COUNTRY_DEF", Arg::Text(&name)),
                    ("FLAG", Arg::Text(&flag)),
                ],
            )?,
            PROPAGANDA => loc(
                "OPERATIVE_MISSION_PROPAGANDA_TT",
                &[
                    ("COUNTRY_DEF", Arg::Text(&name)),
                    ("FLAG", Arg::Text(&flag)),
                    ("STABILITY", Arg::Fixed5(raw(0))),
                    ("WAR_SUPPORT", Arg::Fixed5(raw(1))),
                ],
            )?,
            _ => return Err("unsupported operative mission tooltip".into()),
        };
        let Some(contribution) = contribution else {
            return Ok(out);
        };
        let d = hour.defines.missions.as_ref().map_err(Clone::clone)?;
        let mut paragraph =
            |key: &str, arg: &str, value: i64, source: String, gap| -> Result<(), String> {
                ensure_newlines(&mut out, gap);
                out.push_str(&values::coloured(
                    'H',
                    &loc(key, &[(arg, Arg::Fixed5(value))])?,
                ));
                ensure_newlines(&mut out, 1);
                out.push_str(&source);
                Ok(())
            };
        match mission.variant {
            BUILD_NETWORK => paragraph(
                "OPERATIVE_INTEL_NETWORK_GAIN",
                "GAIN",
                raw(0),
                sources(
                    hour,
                    mission,
                    d.network[0].wrapping_mul(i64::from(
                        mission
                            .effect
                            .ok_or("network gain multiplier unavailable")?
                            .network_gain_scale,
                    )),
                    1,
                    false,
                )?,
                2,
            )?,
            COUNTER_INTELLIGENCE => paragraph(
                "OPERATIVE_INTELLIGENCE_AGENCY_DEFENSE",
                "DEFENSE",
                raw(0),
                {
                    let d = hour.defines.counter.as_ref().map_err(Clone::clone)?;
                    let mut source = sources(hour, mission, d.operative_base, 2, false)?;
                    if d.foreign_agent != 0 {
                        source.push_str(&loc("MODIFIER_OPERATING_IN_FOREIGN_COUNTRY", &[])?);
                        source.push_str(&values::coloured(
                            'Y',
                            &content_reader_parser::fixed5::fixed5_percent(
                                ONE.wrapping_add(d.foreign_agent),
                                2,
                                true,
                            ),
                        ));
                        source.push('\n');
                    }
                    source
                },
                2,
            )?,
            ROOT_OUT => paragraph(
                "OPERATIVE_MISSION_ROOT_OUT_RESISTANCE_EFFORT",
                "VALUE",
                raw(0),
                sources(
                    hour,
                    mission,
                    hour.defines.root_out.as_ref().map_err(Clone::clone)?[0],
                    2,
                    false,
                )?,
                2,
            )?,
            BOOST_IDEOLOGY => paragraph(
                "OPERATIVE_MISSION_PROPAGANDA_EFFORT",
                "VALUE",
                raw(0),
                sources(hour, mission, d.ideology[0], 2, false)?,
                2,
            )?,
            CONTROL_TRADE => {
                ensure_newlines(&mut out, 2);
                out.push_str(&sources(hour, mission, d.trade[0], 2, false)?);
            }
            DIPLOMATIC_PRESSURE => {
                paragraph(
                    "DIPLOMATIC_PRESSURE_OPINION_REQUIREMENTS_DRIFT",
                    "DRIFT",
                    raw(0),
                    sources(
                        hour,
                        mission,
                        q15_to_fixed5(fixed5_to_q15(d.diplomacy[0])),
                        2,
                        false,
                    )?,
                    2,
                )?;
                paragraph(
                    "DIPLOMATIC_PRESSURE_TENSION_REQUIREMENTS_DRIFT",
                    "DRIFT",
                    raw(1),
                    sources(
                        hour,
                        mission,
                        q15_to_fixed5(fixed5_to_q15(d.diplomacy[1])),
                        1,
                        true,
                    )?,
                    2,
                )?;
            }
            PROPAGANDA => {
                let source = sources(
                    hour,
                    mission,
                    q15_to_fixed5(fixed5_to_q15(d.propaganda[0])),
                    2,
                    true,
                )?;
                paragraph(
                    "OPERATIVE_MISSION_PROPAGANDA_STABILITY_DRIFT",
                    "DRIFT",
                    raw(0),
                    source.clone(),
                    2,
                )?;
                paragraph(
                    "OPERATIVE_MISSION_PROPAGANDA_WAR_SUPPORT_DRIFT",
                    "DRIFT",
                    raw(1),
                    source,
                    2,
                )?;
            }
            _ => {}
        }
        ensure_newlines(
            &mut out,
            if matches!(mission.variant, PROPAGANDA | DIPLOMATIC_PRESSURE) {
                2
            } else {
                1
            },
        );
        out.push_str(&scaled(hour, contribution)?);
        if mission.variant == CONTROL_TRADE {
            let strategic = shared
                .tables
                .strategic
                .as_ref()
                .map_err(Clone::clone)?
                .iter()
                .find(|row| hour.slot(row.country) == Some(country))
                .ok_or("control trade target strategic operative unavailable")?;
            let owner = hour
                .slot(mission.row.owner_country)
                .ok_or("control trade owner link unavailable")?;
            let current = strategic
                .trade_influence
                .iter()
                .find(|entry| hour.slot(entry.owner) == Some(owner))
                .map_or(0, |entry| entry.value);
            let effective = mission
                .effect
                .and_then(|effect| effect.trade_opinion)
                .ok_or("current trade opinion was not captured")?;
            ensure_newlines(&mut out, 2);
            out.push_str(&loc(
                "OPERATIVE_MISSION_CONTROL_TRADE_PROGRESS",
                &[
                    ("CURRENT", Arg::Fixed5(current)),
                    ("MAX", Arg::Fixed5(d.trade[2])),
                    ("EFFECTIVE", Arg::Fixed5(effective)),
                ],
            )?);
        }
        if mission.variant == ROOT_OUT {
            ensure_newlines(&mut out, 2);
            out.push_str(&loc(
                "OPERATIVE_MISSION_ROOT_OUT_RESISTANCE_STATE_MODIFIER",
                &[],
            )?);
            ensure_newlines(&mut out, 1);
            let scripted = hour
                .assets
                .scripted
                .as_ref()
                .ok_or("operative static modifier content unavailable")?
                .as_ref()
                .as_ref()
                .map_err(Clone::clone)?;
            let script = dashboard_text::scripted_text::ScriptedText::new(
                hour.frame,
                hour.localisation,
                scripted,
                hour.ours,
            )
            .viewing(hour.ours);
            let effects = scripted.effects()?;
            let context = dashboard_effects::effect_description::Context::new(
                &script,
                hour.localisation,
                hour.ours,
                "common/modifiers",
                effects.view(None),
            );
            out.push_str(
                &dashboard_effects::effect_description::scaled_static_modifiers(
                    &context,
                    "root_out_resistance_mission_modifier",
                    "  ",
                    raw(0),
                )?,
            );
        }
        Ok(out)
    })();
    text.line(false, 2, "operative mission effects", result);
}
