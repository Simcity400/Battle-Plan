use super::*;
pub(super) fn stock_fill(stock: Stock, inputs: &Inputs) -> io::Result<ModeFill> {
    use Source::{Country, Empty, Mode, Strategic};
    let political = inputs.content.defines.country_channel;
    let mut warnings = Vec::new();
    let variant = |variant, warnings: &mut Vec<String>| -> io::Result<Draw> {
        let country = country_variant(variant, inputs, warnings)?;
        Ok(Draw {
            channels: [built(&country, Country), Empty],
            strategic: None,
            country,
            layer: None,
            second: None,
            political,
        })
    };
    let over_countries = |layer: Option<ModeLayer>| Draw {
        channels: [Country, built(&layer, Mode)],
        strategic: None,
        country: None,
        layer,
        second: None,
        political,
    };
    let draw = match stock {
        Stock::Default => return Ok(ModeFill::default()),
        Stock::Resources => Draw::of([Country, Empty], political),
        Stock::StrategicNavy | Stock::StrategicAir => {
            let mut draw = Draw::of(
                if stock == Stock::StrategicAir {
                    [Strategic, Empty]
                } else {
                    POLITICAL
                },
                political,
            );
            draw.strategic = Some(inputs.content.strategic.oracle_layer(
                stock,
                inputs,
                &mut warnings,
            ));
            draw
        }
        Stock::Raids => Draw::of([Country, Empty], political),
        Stock::Factions => variant(Variant::Factions, &mut warnings)?,
        Stock::Players => variant(Variant::Players, &mut warnings)?,
        Stock::Ideology => variant(Variant::Ideology, &mut warnings)?,
        Stock::States => over_countries(states(inputs, &mut warnings)?),
        Stock::Resistance => over_countries(occupation(false, inputs, &mut warnings)?),
        Stock::Compliance => over_countries(occupation(true, inputs, &mut warnings)?),
        Stock::Operatives => over_countries(operatives(inputs, &mut warnings)?),
        Stock::Diplomacy => {
            let layer = diplomacy(inputs, &mut warnings)?;
            Draw {
                channels: [built(&layer, Mode), Empty],
                strategic: None,
                country: None,
                layer,
                second: None,
                political,
            }
        }
        Stock::Manpower => {
            let layer = manpower(inputs, &mut warnings)?;
            Draw {
                channels: [built(&layer, Mode), Country],
                strategic: None,
                country: None,
                layer,
                second: None,
                political,
            }
        }
        Stock::Terrain => {
            let layer = match &inputs.content.terrain {
                Ok(built) => {
                    warnings.extend(built.warnings.iter().cloned());
                    Some(built.layer.clone())
                }
                Err(why) => {
                    warnings.push(format!(
                        "Terrain map mode: its layer is not drawn and the countries show alone: {why}"
                    ));
                    None
                }
            };
            Draw {
                channels: [built(&layer, Mode), Country],
                strategic: None,
                country: None,
                layer,
                second: None,
                political,
            }
        }
        Stock::Infrastructure => {
            let layer = inputs.content.infrastructure.layer(
                &inputs.content.defines,
                inputs,
                &mut warnings,
            )?;
            Draw {
                channels: [built(&layer, Mode), Country],
                strategic: None,
                country: None,
                layer,
                second: None,
                political,
            }
        }
        Stock::Supply => {
            warnings.push(unbuilt(stock).to_owned());
            Draw::of([Empty, Empty], political)
        }
    };
    Ok(ModeFill {
        draw: Some(draw),
        warnings,
        colour: None,
    })
}

fn unbuilt(stock: Stock) -> &'static str {
    match stock {
        Stock::Supply => {
            "Supply map mode: its province supply and supply-border layers need each province's distributed supply, which is not recorded; both channels stay empty and the supply hub icons, flow arrows and supply tooltips are not drawn"
        }
        _ => "This map mode's layer is not drawn",
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Variant {
    Factions,
    Players,
    Ideology,
}

fn country_variant(
    variant: Variant,
    inputs: &Inputs,
    warnings: &mut Vec<String>,
) -> io::Result<Option<ModeLayer>> {
    let Inputs {
        content,
        map,
        frame,
        selected,
        ..
    } = inputs;
    let defines = &content.defines;
    let countries = Countries::read(frame)?;
    let (region_of, slots) = controller_regions(map, frame, &countries)?;
    let colors = subject_colors(map, frame, &countries, &slots, warnings)?;
    let mut styles = BTreeMap::new();
    let channel = if variant == Variant::Factions {
        defines.factions_channel
    } else {
        defines.country_channel
    };
    match variant {
        Variant::Factions => {
            let members = match complete::<tables::FactionMembers>(frame, "factions") {
                Ok(rows) => rows,
                Err(why) => {
                    warnings.push(why);
                    return Ok(None);
                }
            };
            let factions = match complete::<tables::Factions>(frame, "factions") {
                Ok(rows) => rows,
                Err(why) => {
                    warnings.push(why);
                    return Ok(None);
                }
            };
            let faction_colors = factions
                .iter()
                .map(|row| {
                    (
                        row.id,
                        [
                            row.color_red,
                            row.color_green,
                            row.color_blue,
                            row.color_alpha,
                        ]
                        .map(f32::from_bits),
                    )
                })
                .collect::<HashMap<_, _>>();
            let membership = members
                .iter()
                .filter_map(|row| Some((countries.slot(row.country)?, row.faction)))
                .collect::<HashMap<_, _>>();
            for &slot in &slots {
                let color = match membership
                    .get(&slot)
                    .and_then(|faction| faction_colors.get(faction))
                {
                    Some(&color) => with_alpha(color, defines.factions_member_alpha),
                    None => defines.factions_not_member,
                };
                styles.insert(slot + 1, style(color));
            }
        }
        Variant::Players => {
            let players = match complete::<tables::Players>(frame, "players") {
                Ok(rows) => rows,
                Err(why) => {
                    warnings.push(why);
                    return Ok(None);
                }
            };
            let human = players
                .iter()
                .filter_map(|row| countries.slot(row.country))
                .collect::<BTreeSet<_>>();
            for (&slot, &color) in &colors {
                let color = if human.contains(&slot) {
                    color
                } else {
                    with_alpha(color, defines.players_other_alpha)
                };
                let mut region = style(color);
                if Some(slot) == *selected {
                    region.pulse = true;
                    region.fill = defines.selected_fill;
                }
                styles.insert(slot + 1, region);
            }
        }
        Variant::Ideology => {
            let politics = match complete::<tables::Politics>(frame, "ideology") {
                Ok(rows) => rows,
                Err(why) => {
                    warnings.push(why);
                    return Ok(None);
                }
            };
            let parties = match complete::<tables::PoliticalParties>(frame, "ideology") {
                Ok(rows) => rows,
                Err(why) => {
                    warnings.push(why);
                    return Ok(None);
                }
            };
            let ruling = politics
                .iter()
                .filter_map(|row| Some((countries.slot(row.country)?, row.ruling_party?)))
                .collect::<HashMap<_, _>>();
            let mut unknown = BTreeSet::new();
            for (&slot, &color) in &colors {
                let party = ruling.get(&slot).and_then(|&party| {
                    parties
                        .iter()
                        .find(|row| row.id == party && countries.slot(row.country) == Some(slot))
                });
                let color = match party {
                    None => Some(color),
                    Some(party) => {
                        let token = String::from_utf8_lossy(&party.token);
                        let group = match content.ideologies.get(&token) {
                            Ok(Some(group)) => Ok(Some(group)),
                            Ok(None) => content.ideologies.group_of(&token),
                            Err(gap) => Err(gap),
                        };
                        match group {
                            Ok(Some(group)) => match &group.color {
                                Ok(group_color) => {
                                    Some(with_alpha(*group_color, defines.ideology_alpha))
                                }
                                Err(_) => {
                                    unknown.insert(token.into_owned());
                                    None
                                }
                            },
                            _ => {
                                unknown.insert(token.into_owned());
                                None
                            }
                        }
                    }
                };
                if let Some(color) = color {
                    styles.insert(slot + 1, style(color));
                }
            }
            if !unknown.is_empty() {
                warnings.push(format!(
                    "The ideology map mode leaves countries unfilled whose ruling ideology has no readable content colour: {}",
                    unknown.into_iter().collect::<Vec<_>>().join(", ")
                ));
            }
        }
    }
    Ok(Some(finish(region_of, styles, None, None, channel)))
}

fn finish(
    mut region_of: Vec<u32>,
    styles: BTreeMap<u32, Style>,
    thickness: Option<f32>,
    gradient: Option<u8>,
    channel: [f32; 2],
) -> ModeLayer {
    for key in &mut region_of {
        if *key != 0 && !styles.contains_key(key) {
            *key = 0;
        }
    }
    ModeLayer {
        region_of: region_of.into(),
        styles: map_display_data::value_index::ValueIndex::try_from_iter(styles).unwrap(),
        thickness,
        gradient,
        channel,
    }
}

fn states(inputs: &Inputs, warnings: &mut Vec<String>) -> io::Result<Option<ModeLayer>> {
    let defines = &inputs.content.defines;
    let states = match complete::<tables::States>(inputs.frame, "states") {
        Ok(states) => states,
        Err(why) => {
            warnings.push(why);
            return Ok(None);
        }
    };
    let names = dashboard_text::table_access::TokenNames::new(inputs.frame)?;
    let mut styles = BTreeMap::new();
    let (mut unread, mut unnamed, mut uncoloured) = (0, 0, 0);
    for state in states {
        let token = match state.category {
            replay_data::rows::Recorded::Value(Some(token)) => token,
            replay_data::rows::Recorded::Value(None) => {
                unread += 1;
                continue;
            }
            replay_data::rows::Recorded::NotRecorded => {
                warnings.push(
                    "The states map mode needs each state's live category, which this recording does not hold".to_owned(),
                );
                return Ok(None);
            }
        };
        let Ok(name) = names.name(token) else {
            unnamed += 1;
            continue;
        };
        match inputs
            .content
            .category_colors
            .get(name.to_ascii_lowercase().as_bytes())
        {
            Some(Ok(Some(color))) => {
                styles.insert(state.id + 1, style(*color));
            }
            Some(Ok(None)) => {
                styles.insert(state.id + 1, style(STATE_WITHOUT_CATEGORY));
            }
            Some(Err(_)) | None => uncoloured += 1,
        }
    }
    for (count, why) in [
        (unread, "whose category read was refused"),
        (
            unnamed,
            "whose category token the recorded dictionary does not name",
        ),
        (
            uncoloured,
            "whose category the content does not declare or colour",
        ),
    ] {
        if count != 0 {
            warnings.push(format!(
                "The states map mode leaves {count} states unfilled {why}"
            ));
        }
    }
    let provinces = core::<tables::Provinces>(inputs.frame)?;
    let mut region_of = vec![0; region_table_len(inputs.map, provinces.iter().map(|row| row.id))];
    for province in provinces {
        if province.state != 0 {
            region_of[province.id as usize] = province.state + 1;
        }
    }
    Ok(Some(finish(
        region_of,
        styles,
        Some(defines.state_thickness),
        None,
        defines.state_channel,
    )))
}

#[derive(Clone, Copy, Debug, Default)]
struct Relation {
    selected: bool,
    at_war: bool,
    same_faction: bool,
    war_goal: Option<bool>,
    opinion: i32,
}

impl Relation {
    fn color(self, defines: &ModeDefines) -> Result<Option<[f32; 4]>, ()> {
        Ok(Some(if self.selected {
            DIPLOMACY_SELF
        } else if self.at_war {
            DIPLOMACY_WAR
        } else if self.same_faction {
            DIPLOMACY_FACTION
        } else if self.war_goal.ok_or(())? {
            DIPLOMACY_WAR_GOAL
        } else if self.opinion > defines.very_good_opinion {
            DIPLOMACY_GOOD
        } else if self.opinion < defines.very_bad_opinion {
            DIPLOMACY_BAD
        } else {
            return Ok(None);
        }))
    }
}

const WAR_GOALS_UNRECORDED: &str = "the selected country's owned war goals are not recorded, and the war-goal test comes before the opinions";

fn war_goal_slots(
    frame: &DecodedFrame,
    countries: &Countries,
    selected_raw: u32,
) -> Result<BTreeSet<u32>, String> {
    let capitulation = complete::<tables::Capitulation>(frame, "diplomacy")?;
    let row = capitulation
        .iter()
        .find(|row| row.country == selected_raw)
        .ok_or_else(|| "the selected country has no Capitulation row".to_owned())?;
    let recipients = match &row.war_goal_recipients {
        replay_data::rows::Recorded::Value(Some(recipients)) => recipients,
        replay_data::rows::Recorded::Value(None) => {
            return Err("a war goal of the selected country did not read".to_owned());
        }
        replay_data::rows::Recorded::NotRecorded => return Err(WAR_GOALS_UNRECORDED.to_owned()),
    };
    let subjects = complete::<tables::Subjects>(frame, "diplomacy")?;
    let mut marked = BTreeSet::new();
    for &recipient in recipients {
        let slot = countries
            .slot(recipient)
            .ok_or_else(|| format!("war goal recipient {recipient} has no CountryLinks row"))?;
        marked.insert(slot);
        for row in subjects
            .iter()
            .filter(|row| row.subject == recipient && active_subject(row, frame.hour))
        {
            let overlord = countries
                .slot(row.overlord)
                .ok_or_else(|| format!("overlord {} has no CountryLinks row", row.overlord))?;
            marked.insert(overlord);
        }
    }
    Ok(marked)
}

fn diplomacy(inputs: &Inputs, warnings: &mut Vec<String>) -> io::Result<Option<ModeLayer>> {
    let Inputs {
        content,
        map,
        frame,
        selected,
        ..
    } = inputs;
    let defines = &content.defines;
    let Some(selected) = *selected else {
        warnings.push("The diplomacy map mode needs a viewing country".to_owned());
        return Ok(None);
    };
    let countries = Countries::read(frame)?;
    let Some(&selected_raw) = countries.raw.get(&selected) else {
        warnings.push("The diplomacy map mode's viewing country has no Countries row".to_owned());
        return Ok(None);
    };
    let tables = (|| {
        Ok::<_, String>((
            complete::<tables::FactionMembers>(frame, "diplomacy")?,
            complete::<tables::RelationOpinions>(frame, "diplomacy")?,
        ))
    })();
    let (members, opinions) = match tables {
        Ok(tables) => tables,
        Err(why) => {
            warnings.push(why);
            return Ok(None);
        }
    };
    let (region_of, slots) = controller_regions(map, frame, &countries)?;
    let faction_of = members
        .iter()
        .filter_map(|row| Some((countries.slot(row.country)?, row.faction)))
        .collect::<HashMap<_, _>>();
    let wars = directed_wars::Owner::new(frame, selected_raw);
    let war_goals = war_goal_slots(frame, &countries, selected_raw);
    let mut styles = BTreeMap::new();
    let mut refused = BTreeMap::<String, usize>::new();
    for &slot in &slots {
        let Some(&raw) = countries.raw.get(&slot) else {
            continue;
        };
        let opinion = if slot == selected {
            0
        } else {
            opinions
                .iter()
                .find(|row| {
                    countries.slot(row.owner_country) == Some(slot)
                        && countries.slot(row.target_country) == Some(selected)
                })
                .map_or(0, |row| row.opinion)
        };
        let at_war = match &wars {
            Ok(owner) => owner.at_war(raw),
            Err(why) => Err(why.clone()),
        };
        let at_war = match at_war {
            Ok(value) => value,
            Err(why) => {
                *refused.entry(why).or_default() += 1;
                continue;
            }
        };
        let same_faction = faction_of
            .get(&slot)
            .is_some_and(|faction| faction_of.get(&selected) == Some(faction));
        let relation = Relation {
            selected: slot == selected,
            at_war,
            same_faction,
            war_goal: war_goals.as_ref().ok().map(|marked| marked.contains(&slot)),
            opinion,
        };
        match relation.color(defines) {
            Ok(Some(color)) => {
                styles.insert(slot + 1, style(color));
            }
            Ok(None) => {}
            Err(()) => {
                let why = war_goals.as_ref().err().cloned().unwrap_or_default();
                *refused.entry(why).or_default() += 1;
            }
        }
    }
    for (why, count) in refused {
        warnings.push(format!(
            "The diplomacy map mode leaves {count} countries unfilled: {why}"
        ));
    }
    Ok(Some(finish(
        region_of,
        styles,
        Some(defines.diplomacy_thickness),
        None,
        defines.diplomacy_channel,
    )))
}

fn manpower_color(defines: &ModeDefines, population: i32) -> [f32; 4] {
    let t = (population as f32 / defines.manpower_max).clamp(0.0, 1.0);
    let [from, to] = defines.manpower_colors;
    std::array::from_fn(|i| (to[i] - from[i]) * t + from[i])
}

fn manpower(inputs: &Inputs, warnings: &mut Vec<String>) -> io::Result<Option<ModeLayer>> {
    let defines = &inputs.content.defines;
    let states = match complete::<tables::States>(inputs.frame, "manpower") {
        Ok(states) => states,
        Err(why) => {
            warnings.push(why);
            return Ok(None);
        }
    };
    let mut styles = BTreeMap::new();
    for state in states {
        match state.population {
            replay_data::rows::Recorded::Value(population) => {
                styles.insert(state.id + 1, style(manpower_color(defines, population)));
            }
            replay_data::rows::Recorded::NotRecorded => {
                warnings.push(
                    "The manpower map mode needs each state's population, which this recording does not hold".to_owned(),
                );
                return Ok(None);
            }
        }
    }
    let provinces = core::<tables::Provinces>(inputs.frame)?;
    let mut region_of = vec![0; region_table_len(inputs.map, provinces.iter().map(|row| row.id))];
    for province in provinces {
        if province.state != 0 {
            region_of[province.id as usize] = province.state + 1;
        }
    }
    Ok(Some(finish(
        region_of,
        styles,
        Some(defines.state_thickness),
        Some(LEVEL_GRADIENT),
        [0.0, defines.state_channel[1]],
    )))
}

const NETWORK_STRENGTH_SCALE: f32 = 100_000.0;
const NETWORK_STRENGTH_FULL: f32 = 100.0;

fn network_color(defines: &ModeDefines, strength: i64, quiet: bool) -> [f32; 4] {
    let share = (strength as f32 / NETWORK_STRENGTH_SCALE / NETWORK_STRENGTH_FULL).clamp(0.0, 1.0);
    let t = (share * share.max(0.5)).clamp(0.0, 1.0);
    let [from, to] = if quiet {
        defines.quiet_network_colors
    } else {
        defines.network_colors
    };
    std::array::from_fn(|i| (to[i] - from[i]) * t + from[i])
}

fn operatives(inputs: &Inputs, warnings: &mut Vec<String>) -> io::Result<Option<ModeLayer>> {
    let defines = &inputs.content.defines;
    let Some(selected) = inputs.selected else {
        warnings.push("The operatives map mode needs a viewing country".to_owned());
        return Ok(None);
    };
    let frame = inputs.frame;
    let networks = match complete::<tables::IntelNetworks>(frame, "operatives") {
        Ok(rows) => rows,
        Err(why) => {
            warnings.push(why);
            return Ok(None);
        }
    };
    let countries = Countries::read(frame)?;
    let controllers = core::<tables::States>(frame)?
        .iter()
        .map(|state| (state.id, state.controller))
        .collect::<HashMap<_, _>>();
    let mut styles = BTreeMap::new();
    let mut unknown_states = 0;
    for row in networks {
        if countries.slot(row.owner_country) != Some(selected) || row.local_strength_raw <= 0 {
            continue;
        }
        let Ok(state) = u32::try_from(row.state_id) else {
            unknown_states += 1;
            continue;
        };
        let Some(&controller) = controllers.get(&state) else {
            unknown_states += 1;
            continue;
        };
        if controller == 0
            || countries.slot(controller).is_none()
            || countries.slot(controller) != countries.slot(row.target_country)
        {
            continue;
        }
        styles.insert(
            state + 1,
            style(network_color(
                defines,
                row.local_strength_raw,
                row.local_quiet != 0,
            )),
        );
    }
    if unknown_states != 0 {
        warnings.push(format!(
            "The operatives map mode leaves {unknown_states} network states unfilled that are not recorded states"
        ));
    }
    let provinces = core::<tables::Provinces>(frame)?;
    let mut region_of = vec![0; region_table_len(inputs.map, provinces.iter().map(|row| row.id))];
    for province in provinces {
        if province.state != 0 {
            region_of[province.id as usize] = province.state + 1;
        }
    }
    Ok(Some(finish(
        region_of,
        styles,
        Some(defines.state_thickness),
        Some(LEVEL_GRADIENT),
        [0.0, defines.state_channel[1]],
    )))
}

fn occupation(
    compliance: bool,
    inputs: &Inputs,
    warnings: &mut Vec<String>,
) -> io::Result<Option<ModeLayer>> {
    let mode = if compliance {
        "compliance"
    } else {
        "resistance"
    };
    let defines = &inputs.content.defines;
    let Some(selected) = inputs.selected else {
        warnings.push(format!("The {mode} map mode needs a viewing country"));
        return Ok(None);
    };
    let frame = inputs.frame;
    let countries = Countries::read(frame)?;
    let resistance = match complete::<tables::StateResistance>(frame, mode) {
        Ok(rows) => rows,
        Err(why) => {
            warnings.push(why);
            return Ok(None);
        }
    };
    let by_state = resistance
        .iter()
        .map(|row| (row.state, row))
        .collect::<HashMap<_, _>>();
    let owned = core::<tables::States>(frame)?
        .iter()
        .filter(|state| countries.slot(state.controller) == Some(selected))
        .map(|state| state.id)
        .collect::<BTreeSet<_>>();
    let provinces = core::<tables::Provinces>(frame)?;
    let len = region_table_len(inputs.map, provinces.iter().map(|row| row.id));
    let mut region_of = vec![0; len];
    for province in provinces {
        if owned.contains(&province.state) {
            region_of[province.id as usize] = province.state + 1;
        }
    }
    let mut styles = BTreeMap::new();
    let mut missing = 0;
    for &state in &owned {
        let Some(row) = by_state.get(&state) else {
            missing += 1;
            continue;
        };
        let color = if row.country as i32 > 0 {
            let (list, level) = if compliance {
                (&defines.compliance_colors, row.compliance)
            } else {
                (&defines.resistance_colors, row.level)
            };
            list.color(level as f32 / RESISTANCE_SCALE)
        } else {
            defines.resistance_uninitialised
        };
        styles.insert(state + 1, style(color));
    }
    if missing != 0 {
        warnings.push(format!(
            "The {mode} map mode leaves {missing} controlled states unfilled without a StateResistance row"
        ));
    }
    warnings.push(format!(
        "The {mode} map mode shows the recording's sampled StateResistance values"
    ));
    Ok(Some(finish(
        region_of,
        styles,
        Some(defines.resistance_thickness),
        Some(LEVEL_GRADIENT),
        [0.0, defines.resistance_cutoff],
    )))
}
